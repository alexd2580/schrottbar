use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::types::{ClickHandler, HoverFlag, PowerlineDirection, PowerlineStyle};
use log::debug;
use tokio::process::Command;
use tokio::{sync::Mutex, task::JoinHandle};

use crate::{
    error::Error,
    section_writer::{DARK_GREEN, RED, SectionWriter, mix_colors},
    state_item::{
        ItemAction, ItemActionReceiver, MainAction, MainActionSender, StateItem, wait_seconds,
    },
};

struct PulseaudioData {
    #[allow(dead_code)]
    port: String,
    mute: bool,
    volume: f32,
}
type SharedData = Arc<Mutex<Option<PulseaudioData>>>;
pub struct Pulseaudio {
    data: SharedData,
    hover: HoverFlag,
}

impl Pulseaudio {
    pub fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(None)),
            hover: Arc::new(AtomicBool::new(false)),
        }
    }
}

async fn try_get_volume() -> Result<PulseaudioData, Error> {
    let volume_out = Command::new("pactl")
        .args(["get-sink-volume", "@DEFAULT_SINK@"])
        .output()
        .await
        .map_err(|e| Error::Local(format!("pactl volume: {e}")))?;

    let volume_str = String::from_utf8_lossy(&volume_out.stdout);
    // Parse "Volume: front-left: 42256 /  64% / ..."
    let volume = volume_str
        .split('/')
        .nth(1)
        .and_then(|s| s.trim().strip_suffix('%'))
        .and_then(|s| s.parse::<f32>().ok())
        .ok_or_else(|| Error::Local(format!("failed to parse volume: {volume_str}")))?;

    let mute_out = Command::new("pactl")
        .args(["get-sink-mute", "@DEFAULT_SINK@"])
        .output()
        .await
        .map_err(|e| Error::Local(format!("pactl mute: {e}")))?;

    let mute_str = String::from_utf8_lossy(&mute_out.stdout);
    let mute = mute_str.contains("yes");

    let port_out = Command::new("pactl")
        .args(["list", "sinks"])
        .output()
        .await
        .map_err(|e| Error::Local(format!("pactl list: {e}")))?;

    let default_sink_out = Command::new("pactl")
        .args(["get-default-sink"])
        .output()
        .await
        .map_err(|e| Error::Local(format!("pactl default-sink: {e}")))?;

    let default_sink = String::from_utf8_lossy(&default_sink_out.stdout)
        .trim()
        .to_string();
    let sinks_str = String::from_utf8_lossy(&port_out.stdout);

    let port =
        parse_active_port(&sinks_str, &default_sink).unwrap_or_else(|| "unknown".to_string());

    Ok(PulseaudioData { port, mute, volume })
}

fn parse_active_port(sinks_output: &str, default_sink: &str) -> Option<String> {
    let mut in_default_sink = false;
    for line in sinks_output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Name:") {
            in_default_sink = trimmed.ends_with(default_sink);
        }
        if in_default_sink && trimmed.starts_with("Active Port:") {
            return Some(trimmed.strip_prefix("Active Port:")?.trim().to_string());
        }
    }
    None
}

#[async_trait::async_trait]
impl StateItem for Pulseaudio {
    async fn print(&self, writer: &mut SectionWriter, _output: &str) -> Result<(), Error> {
        writer.set_style(PowerlineStyle::Powerline);
        writer.set_direction(PowerlineDirection::Right);
        writer.set_hover_flag(self.hover.clone());

        writer.set_on_click(Arc::new(|_button| {
            tokio::spawn(async {
                let _ = Command::new("pavucontrol").spawn();
            });
        }) as ClickHandler);

        let state = self.data.lock().await;
        if let Some(ref data) = *state {
            let vol_color = mix_colors(data.volume, 100f32, 125f32, DARK_GREEN, RED);
            writer.open_bg(vol_color);

            let icon = if data.mute {
                "󰖁"
            } else if data.volume < 33f32 {
                "󰕿"
            } else if data.volume < 66f32 {
                "󰖀"
            } else {
                "󰕾"
            };

            let volume = data.volume;

            if writer.is_hovered() {
                writer.write(format!("{icon} {volume:.0}% {}", data.port));
            } else {
                writer.write(format!("{icon} {volume:.0}%"));
            }
            writer.close();
        }
        writer.clear_on_click();
        writer.clear_hover_flag();
        Ok(())
    }

    fn start_coroutine(
        &self,
        main_action_sender: MainActionSender,
        item_action_receiver: ItemActionReceiver,
    ) -> JoinHandle<()> {
        tokio::spawn(pulseaudio_coroutine(
            self.data.clone(),
            main_action_sender,
            item_action_receiver,
        ))
    }
}

async fn pulseaudio_coroutine(
    state: SharedData,
    main_action_sender: MainActionSender,
    mut item_action_receiver: ItemActionReceiver,
) {
    loop {
        {
            *(state.lock().await) = try_get_volume().await.ok();

            if !main_action_sender.enqueue(MainAction::Redraw).await {
                break;
            }
        }

        tokio::select! {
            message = item_action_receiver.next() => {
                match message {
                    None | Some(ItemAction::Update)  => {},
                    Some(ItemAction::Terminate) => break,
                }
            }
            _ = wait_seconds(10) => {}
        }
    }
    debug!("coroutine exiting");
}
