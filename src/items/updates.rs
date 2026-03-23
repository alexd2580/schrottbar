use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::types::{HoverFlag, PowerlineDirection, PowerlineStyle};
use log::{debug, warn};
use tokio::process::Command;
use tokio::{sync::Mutex, task::JoinHandle};

use crate::{
    error::Error,
    section_writer::{DARK_GREEN, RED, SectionWriter, mix_colors},
    state_item::{
        ItemAction, ItemActionReceiver, MainAction, MainActionSender, StateItem, wait_seconds,
    },
};

struct UpdatesData {
    repo_count: usize,
    aur_count: usize,
    last_upgrade: Option<chrono::DateTime<chrono::FixedOffset>>,
}

type SharedData = Arc<Mutex<Option<UpdatesData>>>;

pub struct Updates {
    data: SharedData,
    hover: HoverFlag,
}

impl Updates {
    pub fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(None)),
            hover: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Check for repo updates via `checkupdates` (from pacman-contrib).
async fn check_repo_updates() -> usize {
    match Command::new("checkupdates").output().await {
        Ok(out) => match out.status.code() {
            Some(0) => out.stdout.iter().filter(|&&b| b == b'\n').count(),
            Some(2) => 0, // no updates
            _ => {
                warn!("checkupdates exited with {:?}", out.status);
                0
            }
        },
        Err(e) => {
            warn!("checkupdates failed: {e}");
            0
        }
    }
}

/// Check if `checkupdates` is available (pacman-contrib installed).
async fn has_checkupdates() -> bool {
    Command::new("which")
        .arg("checkupdates")
        .output()
        .await
        .is_ok_and(|out| out.status.success())
}

/// Run `yay -Qua` (or `paru -Qua` as fallback) for AUR packages.
/// NOTE: paru fallback is untested.
async fn check_aur_updates() -> usize {
    // Try yay first
    if let Ok(out) = Command::new("yay").args(["-Qua"]).output().await {
        if out.status.success() {
            return out.stdout.iter().filter(|&&b| b == b'\n').count();
        }
        // yay exits 1 when there are no updates
        if out.status.code() == Some(1) {
            return 0;
        }
    }

    // Fallback to paru (untested)
    warn!("yay not available, falling back to paru (untested)");
    if let Ok(out) = Command::new("paru").args(["-Qua"]).output().await {
        if out.status.success() {
            return out.stdout.iter().filter(|&&b| b == b'\n').count();
        }
        if out.status.code() == Some(1) {
            return 0;
        }
    }

    warn!("neither yay nor paru available for AUR update check");
    0
}

/// Parse /var/log/pacman.log for the last transaction that upgraded packages.
/// Matches both `pacman -Syu` and `yay`/`paru` AUR upgrades (`pacman -U`).
fn last_upgrade_time() -> Option<chrono::DateTime<chrono::FixedOffset>> {
    use std::io::{BufRead, BufReader};

    let file = std::fs::File::open(PACMAN_LOG).ok()?;
    let reader = BufReader::new(file);

    let mut last_completed_ts = None;
    let mut candidate_ts: Option<String> = None;
    let mut saw_upgraded = false;

    for line in reader.lines() {
        let line = line.ok()?;
        if line.contains("[PACMAN]") {
            // Finalize previous candidate if it had upgrades.
            if candidate_ts.is_some() && saw_upgraded {
                last_completed_ts = candidate_ts.take();
            }
            // Any PACMAN action starts a new candidate.
            candidate_ts = line.strip_prefix('[').and_then(|s| {
                s.find(']').map(|end| s[..end].to_string())
            });
            saw_upgraded = false;
        } else if candidate_ts.is_some() && line.contains("[ALPM] upgraded ") {
            saw_upgraded = true;
        }
    }
    // Finalize last candidate.
    if candidate_ts.is_some() && saw_upgraded {
        last_completed_ts = candidate_ts;
    }

    let ts = last_completed_ts?;
    chrono::DateTime::parse_from_str(&ts, "%Y-%m-%dT%H:%M:%S%z").ok()
}

fn format_age(dt: &chrono::DateTime<chrono::FixedOffset>) -> String {
    let elapsed = chrono::Local::now().signed_duration_since(dt);
    let secs = elapsed.num_seconds().max(0) as u64;
    if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

#[async_trait::async_trait]
impl StateItem for Updates {
    async fn print(&self, writer: &mut SectionWriter, _output: &str) -> Result<(), Error> {
        writer.set_style(PowerlineStyle::Powerline);
        writer.set_direction(PowerlineDirection::Right);
        writer.set_hover_flag(self.hover.clone());

        let state = self.data.lock().await;
        if let Some(ref data) = *state {
            let total = data.repo_count + data.aur_count;
            let age = data.last_upgrade.as_ref().map(format_age);

            if total > 0 {
                let bg = mix_colors(total as f32, 0f32, 50f32, DARK_GREEN, RED);
                writer.open_bg(bg);
                if writer.is_hovered() {
                    let age_str = age.as_deref().unwrap_or("?");
                    writer.write(format!(
                        "󰏔 {total} ({} repo, {} aur) {age_str} ago",
                        data.repo_count, data.aur_count
                    ));
                } else {
                    let suffix = age.map_or(String::new(), |a| format!(" {a}"));
                    writer.write(format!("󰏔 {total}{suffix}"));
                }
                writer.close();
            }
        }

        writer.clear_hover_flag();
        Ok(())
    }

    fn start_coroutine(
        &self,
        main_action_sender: MainActionSender,
        item_action_receiver: ItemActionReceiver,
    ) -> JoinHandle<()> {
        tokio::spawn(updates_coroutine(
            self.data.clone(),
            main_action_sender,
            item_action_receiver,
        ))
    }
}

const PACMAN_LOG: &str = "/var/log/pacman.log";

struct PacmanLogWatcher {
    _inotify: inotify::Inotify,
    async_fd: tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>,
}

/// Set up an inotify watch on the pacman log, integrated with tokio's event loop.
fn watch_pacman_log() -> Option<PacmanLogWatcher> {
    use inotify::{Inotify, WatchMask};
    use std::os::fd::{AsFd, OwnedFd};

    let inotify = Inotify::init().ok()?;
    inotify.watches().add(PACMAN_LOG, WatchMask::MODIFY).ok()?;

    let owned_fd: OwnedFd = inotify.as_fd().try_clone_to_owned().ok()?;
    let async_fd = tokio::io::unix::AsyncFd::new(owned_fd).ok()?;

    Some(PacmanLogWatcher {
        _inotify: inotify,
        async_fd,
    })
}

impl PacmanLogWatcher {
    async fn wait(&self) {
        let _ = self.async_fd.readable().await;
    }
}

async fn updates_coroutine(
    state: SharedData,
    main_action_sender: MainActionSender,
    mut item_action_receiver: ItemActionReceiver,
) {
    if !has_checkupdates().await {
        warn!("checkupdates not found — install pacman-contrib for repo update checks");
    }

    let mut last_remote_check = std::time::Instant::now() - std::time::Duration::from_secs(3600);
    let mut prev_last_upgrade: Option<chrono::DateTime<chrono::FixedOffset>> = None;

    loop {
        let last_upgrade = tokio::task::spawn_blocking(last_upgrade_time)
            .await
            .ok()
            .flatten();

        // Do a remote check if: an hour has passed, or the last upgrade time changed.
        let upgrade_changed = last_upgrade != prev_last_upgrade;
        let hour_elapsed = last_remote_check.elapsed() >= std::time::Duration::from_secs(3600);

        let (repo_count, aur_count) = if upgrade_changed || hour_elapsed {
            last_remote_check = std::time::Instant::now();
            if upgrade_changed {
                debug!("last upgrade time changed, rechecking remote updates");
            }
            tokio::join!(check_repo_updates(), check_aur_updates())
        } else {
            // Keep previous counts.
            let prev = state.lock().await;
            let (r, a) = prev.as_ref().map_or((0, 0), |d| (d.repo_count, d.aur_count));
            (r, a)
        };

        prev_last_upgrade = last_upgrade;

        {
            *state.lock().await = Some(UpdatesData {
                repo_count,
                aur_count,
                last_upgrade,
            });
        }

        if !main_action_sender.enqueue(MainAction::Redraw).await {
            break;
        }

        // Wait for: pacman log change, hourly timer, or termination signal.
        let watcher = watch_pacman_log();
        tokio::select! {
            message = item_action_receiver.next() => {
                match message {
                    None | Some(ItemAction::Update) => {},
                    Some(ItemAction::Terminate) => break,
                }
            }
            _ = async {
                if let Some(ref w) = watcher {
                    w.wait().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {
                debug!("pacman.log changed, rechecking");
                // Brief delay — pacman may still be writing.
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
            _ = wait_seconds(3600) => {}
        }
    }
    debug!("coroutine exiting");
}
