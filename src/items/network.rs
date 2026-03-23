use std::sync::Arc;

use log::{debug, warn};
use tokio::{sync::Mutex, task::JoinHandle};

use crate::{
    error::Error,
    section_writer::{DARK_GREEN, GRAY, RED, SectionWriter, mix_colors},
    state_item::{ItemAction, ItemActionReceiver, MainAction, MainActionSender, StateItem},
    types::{HoverFlag, PowerlineDirection, PowerlineStyle},
};

#[derive(Debug, Clone)]
struct InterfaceInfo {
    name: String,
    kind: InterfaceKind,
    ip: Option<String>,
    up: bool,
}

#[derive(Debug, Clone)]
enum InterfaceKind {
    Wifi { ssid: Option<String>, signal_dbm: Option<i32> },
    Ethernet,
}

#[derive(Debug, Clone)]
struct NetworkData {
    interfaces: Vec<InterfaceInfo>,
    connectivity: bool,
}

/// Read the default route interface from /proc/net/route.
fn default_interface() -> Option<String> {
    let content = std::fs::read_to_string("/proc/net/route").ok()?;
    for line in content.lines().skip(1) {
        let mut cols = line.split_whitespace();
        let iface = cols.next()?;
        let dest = cols.next()?;
        if dest == "00000000" {
            return Some(iface.to_string());
        }
    }
    None
}

/// Check if an interface is wireless by looking for /sys/class/net/<iface>/wireless.
fn is_wireless(iface: &str) -> bool {
    std::path::Path::new(&format!("/sys/class/net/{iface}/wireless")).exists()
}

/// Check if an interface is up (operstate).
fn is_up(iface: &str) -> bool {
    std::fs::read_to_string(format!("/sys/class/net/{iface}/operstate"))
        .map(|s| s.trim() == "up")
        .unwrap_or(false)
}

/// Get the first IPv4 address for an interface by parsing `ip -4 addr show <iface>`.
fn get_ipv4(iface: &str) -> Option<String> {
    let output = std::process::Command::new("ip")
        .args(["-4", "-o", "addr", "show", iface])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Format: "2: enp5s0    inet 192.168.1.100/24 ..."
    for line in stdout.lines() {
        if let Some(inet_pos) = line.find("inet ") {
            let rest = &line[inet_pos + 5..];
            let addr = rest.split_whitespace().next()?;
            // Strip /prefix
            return Some(addr.split('/').next()?.to_string());
        }
    }
    None
}

/// Get WiFi SSID and signal via `iw dev <iface> link`.
fn get_wifi_info(iface: &str) -> (Option<String>, Option<i32>) {
    let output = match std::process::Command::new("iw")
        .args(["dev", iface, "link"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return (None, None),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);

    if stdout.contains("Not connected") {
        return (None, None);
    }

    let mut ssid = None;
    let mut signal = None;
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("SSID: ") {
            ssid = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("signal: ") {
            // "signal: -52 dBm"
            signal = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<i32>().ok());
        }
    }
    (ssid, signal)
}

/// Quick connectivity check: try to reach a known IP.
fn check_connectivity() -> bool {
    std::process::Command::new("ping")
        .args(["-c", "1", "-W", "2", "1.1.1.1"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// List physical network interfaces (skip lo, virtual, etc.).
fn list_interfaces() -> Vec<String> {
    let mut ifaces = Vec::new();
    let entries = match std::fs::read_dir("/sys/class/net") {
        Ok(e) => e,
        Err(_) => return ifaces,
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip loopback and virtual interfaces
        if name == "lo" || name.starts_with("veth") || name.starts_with("br-") || name.starts_with("docker") {
            continue;
        }
        // Only include interfaces that have a device backing (not purely virtual)
        if entry.path().join("device").exists() || is_wireless(&name) {
            ifaces.push(name);
        }
    }
    ifaces.sort();
    ifaces
}

fn gather_network_data() -> NetworkData {
    let default_iface = default_interface();
    let iface_names = list_interfaces();

    let mut interfaces = Vec::new();
    for name in iface_names {
        let up = is_up(&name);
        let ip = if up { get_ipv4(&name) } else { None };
        let kind = if is_wireless(&name) {
            let (ssid, signal) = if up { get_wifi_info(&name) } else { (None, None) };
            InterfaceKind::Wifi { ssid, signal_dbm: signal }
        } else {
            InterfaceKind::Ethernet
        };
        interfaces.push(InterfaceInfo { name, kind, ip, up });
    }

    // Sort: default interface first, then up before down
    interfaces.sort_by(|a, b| {
        let a_default = default_iface.as_deref() == Some(&a.name);
        let b_default = default_iface.as_deref() == Some(&b.name);
        b_default.cmp(&a_default).then(b.up.cmp(&a.up))
    });

    let connectivity = check_connectivity();

    NetworkData {
        interfaces,
        connectivity,
    }
}

/// WiFi signal quality icon based on dBm.
fn wifi_icon(signal_dbm: Option<i32>) -> &'static str {
    match signal_dbm {
        Some(s) if s >= -50 => "󰤨",  // excellent
        Some(s) if s >= -60 => "󰤥",  // good
        Some(s) if s >= -70 => "󰤢",  // fair
        Some(s) if s >= -80 => "󰤟",  // weak
        Some(_) => "󰤯",              // very weak
        None => "󰤮",                 // disconnected
    }
}

/// Map WiFi dBm to 0.0–100.0 for color mixing.
fn signal_quality(dbm: i32) -> f32 {
    // -30 dBm = best, -90 dBm = worst
    ((dbm + 90) as f32 / 60.0 * 100.0).clamp(0.0, 100.0)
}

type SharedData = Arc<Mutex<Option<NetworkData>>>;
pub struct Network {
    data: SharedData,
    hover: HoverFlag,
}

impl Network {
    pub fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(None)),
            hover: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

#[async_trait::async_trait]
impl StateItem for Network {
    async fn print(&self, writer: &mut SectionWriter, _output: &str) -> Result<(), Error> {
        writer.set_style(PowerlineStyle::Powerline);
        writer.set_direction(PowerlineDirection::Right);
        writer.set_hover_flag(self.hover.clone());

        let hovered = writer.is_hovered();
        let state = self.data.lock().await;
        let Some(data) = state.as_ref() else {
            let spinner_angle = crate::utils::spinner::angle();
            writer.with_bg(GRAY, &|writer| {
                writer.write("󰛳".to_string());
                writer.write_spinner(spinner_angle);
            });
            writer.clear_hover_flag();
            return Ok(());
        };

        if data.interfaces.is_empty() {
            writer.with_bg(RED, &|writer| {
                writer.write(format!("󰲛 no interfaces"));
            });
            writer.clear_hover_flag();
            return Ok(());
        }

        let mut first = true;
        for iface in &data.interfaces {
            if !iface.up {
                continue;
            }
            match &iface.kind {
                InterfaceKind::Wifi { ssid, signal_dbm } => {
                    let quality = signal_dbm.map(signal_quality).unwrap_or(0.0);
                    let bg = mix_colors(quality, 20.0, 80.0, RED, DARK_GREEN);
                    let icon = wifi_icon(*signal_dbm);
                    writer.open_bg(bg);
                    first = false;
                    if hovered {
                        if let Some(ssid) = ssid {
                            writer.write(format!("{icon} {ssid}"));
                        } else {
                            writer.write(format!("{icon}"));
                        }
                        if let Some(ip) = &iface.ip {
                            writer.split();
                            writer.write(format!("{ip}"));
                        }
                    } else {
                        writer.write(format!("{icon}"));
                    }
                    writer.close();
                }
                InterfaceKind::Ethernet => {
                    let bg = DARK_GREEN;
                    writer.open_bg(bg);
                    first = false;
                    if hovered {
                        writer.write(format!("󰈀 {}", iface.name));
                        if let Some(ip) = &iface.ip {
                            writer.split();
                            writer.write(format!("{ip}"));
                        }
                    } else {
                        writer.write(format!("󰈀"));
                    }
                    writer.close();
                }
            }
        }

        // If no up interfaces at all
        if first {
            writer.with_bg(RED, &|writer| {
                writer.write(format!("󰲛 disconnected"));
            });
            writer.clear_hover_flag();
            return Ok(());
        }

        // Connectivity indicator if no internet despite link up
        if !data.connectivity {
            writer.with_bg(RED, &|writer| {
                writer.write(format!("󰅛"));
            });
        }

        writer.clear_hover_flag();
        Ok(())
    }

    fn start_coroutine(
        &self,
        main_action_sender: MainActionSender,
        item_action_receiver: ItemActionReceiver,
    ) -> JoinHandle<()> {
        tokio::spawn(network_coroutine(
            self.data.clone(),
            main_action_sender,
            item_action_receiver,
        ))
    }
}

async fn network_coroutine(
    state: SharedData,
    main_action_sender: MainActionSender,
    mut item_action_receiver: ItemActionReceiver,
) {
    loop {
        let data = match tokio::task::spawn_blocking(gather_network_data).await {
            Ok(d) => d,
            Err(e) => {
                warn!("network data gather failed: {e}");
                break;
            }
        };
        {
            *state.lock().await = Some(data);
        }
        if !main_action_sender.enqueue(MainAction::Redraw).await {
            break;
        }

        tokio::select! {
            message = item_action_receiver.next() => {
                match message {
                    None | Some(ItemAction::Update) => {},
                    Some(ItemAction::Terminate) => break,
                }
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(10)) => {}
        }
    }
    debug!("network coroutine exiting");
}
