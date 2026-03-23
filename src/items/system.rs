use std::{sync::Arc, time};

use crate::types::{HoverFlag, PowerlineDirection, PowerlineStyle};
use log::debug;
use tokio::{sync::Mutex, task::JoinHandle};

use crate::{
    error::Error,
    section_writer::{DARK_GREEN, RED, SectionWriter, mix_colors},
    state_item::{ItemAction, ItemActionReceiver, MainAction, MainActionSender, StateItem},
};

fn format_bytes(a: u64) -> String {
    if a < 1_000 {
        format!("{a}B")
    } else if a < 1_000_000 {
        let a = a / 1_000;
        format!("{a}kB")
    } else if a < 1_000_000_000 {
        let a = a / 1_000_000;
        format!("{a}MB")
    } else if a < 1_000_000_000_000 {
        let a = a / 1_000_000_000;
        format!("{a}GB")
    } else {
        let a = a / 1_000_000_000_000;
        format!("{a}TB")
    }
}

struct SystemData {
    /// Refresh period of cpu info.
    cpu_refresh_period: time::Duration,
    /// Timestamp of last refresh of cpu info.
    cpu_refresh_time: time::Instant,
    /// The index of the sysinfo component that holds the "k10temp Tctl" temperature.
    /// We don't support any other CPU/driver at the moment.
    cpu_temp_component_index: Option<usize>,
    /// CPU usage in %. None until first real reading.
    cpu_usage: Option<f32>,
    /// CPU Temperature. Refreshed every `cpu_refresh_period`.
    cpu_temp: Option<f32>,

    /// Refresh period of memory usage info.
    mem_refresh_period: time::Duration,
    /// Timestamp of last refresh of memory usage info.
    mem_refresh_time: time::Instant,
    /// Ram usage in %. None until first real reading.
    ram_usage: Option<f32>,
    /// Formatted used/total sizes.
    ram_usage_formatted: Option<(String, String)>,

    sysinfo: sysinfo::System,
    components: sysinfo::Components,
}

impl SystemData {
    fn new() -> Self {
        let mut sysinfo = sysinfo::System::new();

        sysinfo.refresh_cpu_all();
        let cpu_refresh_time = time::Instant::now();
        sysinfo.refresh_memory();
        let mem_refresh_time = time::Instant::now();

        let components = sysinfo::Components::new_with_refreshed_list();

        let k10_tctl_label = "k10temp Tctl";
        let cpu_temp_component_index = components
            .iter()
            .enumerate()
            .find_map(|(index, value)| (value.label() == k10_tctl_label).then_some(index));

        let total_ram = sysinfo.total_memory();
        let available_ram = sysinfo.available_memory();
        let used_ram = total_ram - available_ram;
        #[allow(clippy::cast_precision_loss)]
        let ram_pct = 100f32 * used_ram as f32 / total_ram as f32;

        Self {
            cpu_refresh_period: time::Duration::from_secs_f32(4.5),
            cpu_refresh_time,
            cpu_temp_component_index,
            cpu_usage: None,
            cpu_temp: None,
            mem_refresh_period: time::Duration::from_secs_f32(14.5),
            mem_refresh_time,
            ram_usage: Some(ram_pct),
            ram_usage_formatted: Some((format_bytes(used_ram), format_bytes(total_ram))),
            sysinfo,
            components,
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn ram_usage(&self) -> (f32, String, String) {
        let sysinfo = &self.sysinfo;
        let total_ram = sysinfo.total_memory();
        let available_ram = sysinfo.available_memory();
        let used_ram = total_ram - available_ram;
        let usage = 100f32 * used_ram as f32 / total_ram as f32;

        (usage, format_bytes(used_ram), format_bytes(total_ram))
    }

    fn update(&mut self) -> bool {
        let mut updated = false;
        if self.cpu_refresh_time.elapsed() > self.cpu_refresh_period {
            self.cpu_refresh_time = time::Instant::now();
            self.sysinfo.refresh_cpu_all();

            self.cpu_usage = Some(self.sysinfo.global_cpu_usage());

            self.cpu_temp = self.cpu_temp_component_index.and_then(|index| {
                self.components.refresh(false);
                self.components[index].temperature()
            });

            updated = true;
        }

        if self.mem_refresh_time.elapsed() > self.mem_refresh_period {
            self.mem_refresh_time = time::Instant::now();
            self.sysinfo.refresh_memory();

            let (usage, used_formatted, total_formatted) = self.ram_usage();
            self.ram_usage = Some(usage);
            self.ram_usage_formatted = Some((used_formatted, total_formatted));

            updated = true;
        }

        updated
    }

    fn is_loading(&self) -> bool {
        self.cpu_usage.is_none() || self.ram_usage.is_none()
    }
}

struct Overrides {
    cpu_usage: f32,
    cpu_temp: f32,
    ram_usage: f32,
}

type SharedData = Arc<Mutex<Option<SystemData>>>;
pub struct System {
    data: SharedData,
    overrides: Option<Overrides>,
    hover: HoverFlag,
}

impl System {
    pub fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(None)),
            overrides: None,
            hover: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    #[allow(dead_code)]
    pub fn with_overrides(cpu_usage: f32, cpu_temp: f32, ram_usage: f32) -> Self {
        Self {
            data: Arc::new(Mutex::new(None)),
            overrides: Some(Overrides {
                cpu_usage,
                cpu_temp,
                ram_usage,
            }),
            hover: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

#[async_trait::async_trait]
impl StateItem for System {
    async fn print(&self, writer: &mut SectionWriter, _output: &str) -> Result<(), Error> {
        writer.set_style(PowerlineStyle::Powerline);
        writer.set_direction(PowerlineDirection::Right);
        writer.set_hover_flag(self.hover.clone());

        let hovered = writer.is_hovered();
        let state = self.data.lock().await;
        let spinner_angle = crate::utils::spinner::angle();

        let (cpu_usage, cpu_temp, ram_usage, ram_formatted);
        if let Some(o) = &self.overrides {
            cpu_usage = Some(o.cpu_usage);
            cpu_temp = Some(o.cpu_temp);
            ram_usage = Some(o.ram_usage);
            ram_formatted = None;
        } else {
            cpu_usage = state.as_ref().and_then(|d| d.cpu_usage);
            cpu_temp = state.as_ref().and_then(|d| d.cpu_temp);
            ram_usage = state.as_ref().and_then(|d| d.ram_usage);
            ram_formatted = state.as_ref().and_then(|d| d.ram_usage_formatted.as_ref());
        }

        // CPU + RAM in one merged section
        let cpu_bg = if let Some(cpu_usage) = cpu_usage {
            if let Some(cpu_temp) = cpu_temp {
                mix_colors(cpu_temp, 50f32, 70f32, DARK_GREEN, RED)
            } else {
                mix_colors(cpu_usage, 80f32, 100f32, DARK_GREEN, RED)
            }
        } else {
            DARK_GREEN
        };

        writer.open_bg(cpu_bg);
        if let Some(cpu_usage) = cpu_usage {
            writer.write(format!("󰍛 {cpu_usage:.0}%"));
            if let Some(cpu_temp) = cpu_temp {
                writer.split();
                writer.write(format!("{cpu_temp:.0}°C"));
            }
        } else {
            writer.write("󰍛".to_string());
            writer.write_spinner(spinner_angle);
        }

        // Inner separator between CPU and RAM
        writer.split();

        if let Some(ram_usage) = ram_usage {
            if hovered {
                if let Some((used_ram, total_ram)) = ram_formatted {
                    writer.write(format!(
                        "󰘚 {ram_usage:.0}% ({used_ram}/{total_ram})"
                    ));
                } else {
                    writer.write(format!("󰘚 {ram_usage:.0}%"));
                }
            } else {
                writer.write(format!("󰘚 {ram_usage:.0}%"));
            }
        } else {
            writer.write("󰘚".to_string());
            writer.write_spinner(spinner_angle);
        }

        writer.close();
        writer.clear_hover_flag();
        Ok(())
    }

    fn start_coroutine(
        &self,
        main_action_sender: MainActionSender,
        item_action_receiver: ItemActionReceiver,
    ) -> JoinHandle<()> {
        tokio::spawn(system_coroutine(
            self.data.clone(),
            main_action_sender,
            item_action_receiver,
        ))
    }
}

async fn system_coroutine(
    state: SharedData,
    main_action_sender: MainActionSender,
    mut item_action_receiver: ItemActionReceiver,
) {
    // Initialize sysinfo off the async runtime — it does blocking I/O.
    let init_data = tokio::task::spawn_blocking(SystemData::new)
        .await
        .expect("sysinfo init panicked");
    {
        *state.lock().await = Some(init_data);
    }

    loop {
        let (loading, updated) = {
            let mut state_lock = state.lock().await;
            let data = state_lock.as_mut().unwrap();
            let updated = data.update();
            (data.is_loading(), updated)
        };

        // Only request redraw when data changed or spinner is animating.
        if (loading || updated) && !main_action_sender.enqueue(MainAction::Redraw).await {
            break;
        }

        // While loading, tick at spinner rate for animation.
        // Once loaded, use normal 5s interval.
        let sleep_ms = if loading {
            crate::utils::spinner::TICK_MS
        } else {
            5000
        };

        tokio::select! {
            message = item_action_receiver.next() => {
                match message {
                    None | Some(ItemAction::Update)  => {},
                    Some(ItemAction::Terminate) => break,
                }
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_millis(sleep_ms)) => {}
        }
    }
    debug!("coroutine exiting");
}
