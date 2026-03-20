mod bar;
#[allow(dead_code)]
mod compositor;
mod error;
mod items;
mod renderer;
mod section_writer;
mod state_item;
mod types;
mod utils;
mod wayland;

use std::collections::HashMap;
use std::fs::File;

use bar::Bar;
use log::{debug, error, info};
use section_writer::SectionWriter;
use state_item::{
    ItemAction, ItemActionSender, MainAction, MainActionReceiver, StateItem,
    new_item_action_channel, new_main_action_channel,
};
use tokio::signal;
use types::ContentItem;

struct StateItems {
    left: Vec<Box<dyn StateItem>>,
    center: Vec<Box<dyn StateItem>>,
    right: Vec<Box<dyn StateItem>>,
}

fn init_state_items() -> StateItems {
    use items::system::System;

    let workspaces = items::workspaces::Workspaces::new();
    let windows = items::windows::Windows::new();
    let paymo = items::paymo::Paymo::default();
    let tray = items::tray::Tray::new(25);
    let pulseaudio = items::pulseaudio::Pulseaudio::new();
    let weather = items::weather::Weather::new();
    let system = System::new();
    let time = items::time::Time::new();

    StateItems {
        left: vec![
            Box::new(workspaces),
            Box::new(items::hspace::HSpace::new(20)),
            Box::new(windows),
        ],
        center: vec![Box::new(weather), Box::new(paymo)],
        right: vec![
            Box::new(pulseaudio),
            Box::new(system),
            Box::new(time),
            Box::new(tray),
        ],
    }
}

type FrameContent = (Vec<ContentItem>, Vec<ContentItem>, Vec<ContentItem>);

async fn redraw(
    state_items: &StateItems,
    bar: &mut Bar,
    last_redraw: &mut std::time::Instant,
    prev_frames: &mut HashMap<String, FrameContent>,
) -> Result<(), error::Error> {
    let now = std::time::Instant::now();
    let dt = now.duration_since(*last_redraw);
    *last_redraw = now;
    debug!("Redraw (dt={:.1}ms)", dt.as_secs_f64() * 1000.0);
    let mut any_drawn = false;
    for (index, output) in bar.outputs().to_vec().iter().enumerate() {
        let mut left_writer = SectionWriter::default();
        for item in &state_items.left {
            item.print(&mut left_writer, &output.name).await?;
        }

        let mut center_writer = SectionWriter::default();
        for item in &state_items.center {
            item.print(&mut center_writer, &output.name).await?;
        }

        let mut right_writer = SectionWriter::default();
        for item in &state_items.right {
            item.print(&mut right_writer, &output.name).await?;
        }

        let left = left_writer.unwrap();
        let center = center_writer.unwrap();
        let right = right_writer.unwrap();

        let new_frame = (left, center, right);
        if prev_frames
            .get(&output.name)
            .is_some_and(|prev| prev == &new_frame)
        {
            debug!("Skipping draw for output {} (unchanged)", output.name);
            continue;
        }

        bar.draw(index, &new_frame.0, &new_frame.1, &new_frame.2);
        prev_frames.insert(output.name.clone(), new_frame);
        any_drawn = true;
    }
    if any_drawn {
        bar.flush();
    }
    debug!("Redraw done");
    Ok(())
}

/// Minimum time between redraws. Requests arriving sooner are deferred.
const MIN_REDRAW_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

async fn main_loop(
    main_action_receiver: &mut MainActionReceiver,
    _item_action_sender: &mut ItemActionSender,
    state_items: &StateItems,
) -> Result<(), error::Error> {
    let mut bar = Bar::new();
    let mut last_redraw = std::time::Instant::now();
    let mut prev_frames: HashMap<String, FrameContent> = HashMap::new();
    let mut redraw_pending = false;

    loop {
        // If a redraw is pending, sleep until the minimum interval has elapsed,
        // then drain any further redraws and render.
        let redraw_deadline = if redraw_pending {
            let elapsed = last_redraw.elapsed();
            if elapsed >= MIN_REDRAW_INTERVAL {
                tokio::time::Instant::now()
            } else {
                tokio::time::Instant::now() + (MIN_REDRAW_INTERVAL - elapsed)
            }
        } else {
            // Far future — effectively disabled.
            tokio::time::Instant::now() + std::time::Duration::from_secs(86400)
        };

        tokio::select! {
            events = bar.next_event() => {
                for event in events {
                    if let crate::wayland::BarEvent::Click { surface_index, x, button } = event {
                        bar.handle_click(surface_index, x, button);
                    }
                }
            }
            _ = signal::ctrl_c() => {
                debug!("Received CTRL+C, terminating");
                break;
            }
            _ = tokio::time::sleep_until(redraw_deadline), if redraw_pending => {
                // Drain any redraws that arrived while we waited.
                if drain_redraws(main_action_receiver, &mut bar, &mut prev_frames) {
                    break;
                }
                redraw_pending = false;
                redraw(state_items, &mut bar, &mut last_redraw, &mut prev_frames).await?;
            }
            message = main_action_receiver.next() => {
                match message {
                    None => {}
                    Some(MainAction::Reinit) => {
                        bar = Bar::new();
                        prev_frames.clear();
                    },
                    Some(MainAction::Terminate) => break,
                    Some(MainAction::Redraw) => {
                        // Drain any burst of redraws already in the channel.
                        if drain_redraws(main_action_receiver, &mut bar, &mut prev_frames) {
                            break;
                        }

                        let since_last = last_redraw.elapsed();
                        if since_last >= MIN_REDRAW_INTERVAL {
                            redraw(state_items, &mut bar, &mut last_redraw, &mut prev_frames).await?;
                        } else {
                            // Defer: will fire on the next select iteration via the deadline branch.
                            redraw_pending = true;
                        }
                    }
                }
            },
        }
    }
    Ok(())
}

/// Drain all pending Redraw messages, handling Reinit inline.
/// Returns true if Terminate was encountered.
fn drain_redraws(
    receiver: &mut MainActionReceiver,
    bar: &mut Bar,
    prev_frames: &mut HashMap<String, FrameContent>,
) -> bool {
    let mut coalesced = 0u32;
    loop {
        match receiver.try_next() {
            Some(MainAction::Redraw) => {
                coalesced += 1;
            }
            Some(MainAction::Reinit) => {
                *bar = Bar::new();
                prev_frames.clear();
            }
            Some(MainAction::Terminate) => {
                debug!("Received Terminate while draining redraws");
                return true;
            }
            None => break,
        }
    }
    if coalesced > 0 {
        debug!("Coalesced {coalesced} additional redraw(s)");
    }
    false
}

fn init_logger() {
    let config = simplelog::ConfigBuilder::new()
        .add_filter_ignore_str("cosmic_text")
        .add_filter_ignore_str("sctk")
        .add_filter_ignore_str("calloop")
        .build();

    let level = if cfg!(debug_assertions) {
        simplelog::LevelFilter::Debug
    } else {
        simplelog::LevelFilter::Info
    };

    let mut loggers: Vec<Box<dyn simplelog::SharedLogger>> = Vec::new();
    loggers.push(simplelog::TermLogger::new(
        level,
        config.clone(),
        simplelog::TerminalMode::Mixed,
        simplelog::ColorChoice::Auto,
    ));

    let time = chrono::Local::now();
    let log_file_name = time.format("/var/log/schrottbar/%FT%T.log").to_string();
    let log_file = File::create(&log_file_name);
    let (log_file, log_file_err) = match log_file {
        Ok(log_file) => (Some(log_file), None),
        Err(log_file_err) => (None, Some(log_file_err)),
    };
    if let Some(log_file) = log_file {
        loggers.push(simplelog::WriteLogger::new(
            level,
            config,
            log_file,
        ));
    }

    simplelog::CombinedLogger::init(loggers).unwrap();

    if log_file_err.is_some() {
        info!("File logging disabled ({log_file_name} not writable)");
    }
}

#[tokio::main]
async fn main() {
    init_logger();
    info!("Launching schrottbar");

    let (main_action_sender, mut main_action_receiver) = new_main_action_channel();
    let (mut item_action_sender, _item_action_receiver) = new_item_action_channel();

    let mut state_items = init_state_items();
    let threads = state_items
        .left
        .iter_mut()
        .chain(state_items.center.iter_mut())
        .chain(state_items.right.iter_mut())
        .map(|item| item.start_coroutine(main_action_sender.clone(), item_action_sender.listen()))
        .collect::<Vec<_>>();

    if let Err(err) = main_loop(
        &mut main_action_receiver,
        &mut item_action_sender,
        &state_items,
    )
    .await
    {
        error!("Main loop terminated with: {err}");
    }

    let _ = item_action_sender.enqueue(ItemAction::Terminate);
    futures::future::join_all(threads).await;

    info!("Goodbye");
}
