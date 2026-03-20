use std::sync::Arc;

use log::{debug, error};
use tokio::{sync::Mutex, task::JoinHandle};

use crate::{
    error::Error,
    section_writer::{ACCENT, DARK_GRAY, GRAY, LIGHT_GRAY, SectionWriter, WHITE},
    state_item::{ItemAction, ItemActionReceiver, MainAction, MainActionSender, StateItem},
    types::{PowerlineDirection, PowerlineStyle, RGBA},
};

use super::niri;

#[derive(Clone)]
struct State {
    windows: Vec<niri_ipc::Window>,
    workspaces: Vec<niri_ipc::Workspace>,
}

type SharedState = Arc<Mutex<State>>;

pub struct Windows(SharedState);

impl Windows {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(State {
            windows: Vec::new(),
            workspaces: Vec::new(),
        })))
    }
}

/// Returns true if the app_id identifies a browser.
fn is_browser(app_id: Option<&str>) -> bool {
    app_id.is_some_and(|id| {
        let id = id.to_lowercase();
        id.contains("firefox") || id.contains("chrom") || id.contains("brave")
    })
}

/// App-id-based icon. Codepoints from saftladen config.
fn app_icon(app_id: Option<&str>) -> Option<&'static str> {
    let id = app_id?.to_lowercase();
    let id = id.as_str();
    Some(match id {
        // Browsers
        _ if id.contains("firefox") => "\u{f269}",
        _ if id.contains("chrom") => "\u{f268}",
        _ if id.contains("brave") => "\u{f268}",
        // Communication
        _ if id.contains("discord") => "\u{f066f}",
        _ if id.contains("telegram") => "\u{e217}",
        _ if id.contains("slack") => "\u{f198}",
        _ if id.contains("mumble") => "\u{f130}",
        _ if id.contains("irssi") => "\u{f292}",
        // Media
        _ if id.contains("spotify") => "\u{f1bc}",
        _ if id.contains("steam") => "\u{ed29}",
        _ if id.contains("vlc") || id.contains("mpv") || id.contains("celluloid") => "\u{f057c}",
        // Tools
        _ if id.contains("gimp") => "\u{e67f}",
        _ if id.contains("nautilus")
            || id.contains("thunar")
            || id.contains("nemo")
            || id.contains("dolphin") =>
        {
            "\u{f07c}"
        }
        _ if id.contains("evince") || id.contains("zathura") || id.contains("okular") => "\u{f1c1}",
        _ if id.contains("obs") => "\u{f03d}",
        _ if id.contains("pavucontrol") => "\u{f028}",
        // Editors (GUI)
        _ if id.contains("code") && id.contains("visual") => "\u{e70c}",
        _ if id.contains("neovide") => "\u{e62b}",
        // Terminals — fall through to title-based
        _ if id.contains("kitty")
            || id.contains("alacritty")
            || id.contains("foot")
            || id.contains("wezterm")
            || id.contains("ghostty") =>
        {
            return None;
        }
        _ => return None,
    })
}

/// Website icon from browser title. Codepoints from saftladen websites config.
fn website_icon(title_lower: &str) -> Option<&'static str> {
    Some(match () {
        // Chat / communication
        _ if title_lower.contains("telegram") => "\u{e217}",
        _ if title_lower.contains("slack") => "\u{f198}",
        _ if title_lower.contains("discord") => "\u{f066f}",
        _ if title_lower.contains("whatsapp") => "\u{f232}",
        _ if title_lower.contains("signal") => "\u{f0553}",
        // Dev
        _ if title_lower.contains("github") => "\u{f408}",
        _ if title_lower.contains("gitlab") => "\u{f296}",
        _ if title_lower.contains("bitbucket") => "\u{f171}",
        _ if title_lower.contains("stack overflow") => "\u{f16c}",
        _ if title_lower.contains("jira") => "\u{f0303}",
        _ if title_lower.contains("trello") => "\u{f181}",
        _ if title_lower.contains("hacker news") => "\u{f1d4}",
        _ if title_lower.contains("crates.io") => "\u{e68b}",
        _ if title_lower.contains("docs.rs") => "\u{e68b}",
        _ if title_lower.contains("npmjs") => "\u{e71e}",
        _ if title_lower.contains("docker hub") => "\u{f308}",
        // Video / media
        _ if title_lower.contains("youtube") => "\u{f16a}",
        _ if title_lower.contains("twitch") => "\u{f1e8}",
        _ if title_lower.contains("spotify") => "\u{f1bc}",
        _ if title_lower.contains("netflix") => "\u{f008}",
        // Social
        _ if title_lower.contains("reddit") => "\u{f281}",
        _ if title_lower.contains("mastodon") => "\u{f0ad1}",
        _ if title_lower.contains("linkedin") => "\u{f0e1}",
        _ if title_lower.contains("facebook") || title_lower.contains("meta") => "\u{f09a}",
        _ if title_lower.contains("instagram") => "\u{f16d}",
        _ if title_lower.contains("twitter") || title_lower.contains("/ x") => "\u{f099}",
        _ if title_lower.contains("pinterest") => "\u{f0d2}",
        _ if title_lower.contains("medium") => "\u{f23a}",
        // Productivity
        _ if title_lower.contains("notion") => "\u{e6b1}",
        _ if title_lower.contains("figma") => "\u{e6b0}",
        _ if title_lower.contains("dropbox") => "\u{f16b}",
        _ if title_lower.contains("google docs") || title_lower.contains("google sheets") => {
            "\u{f0219}"
        }
        _ if title_lower.contains("google drive") => "\u{f0e43}",
        _ if title_lower.contains("google maps") => "\u{f05f5}",
        _ if title_lower.contains("google calendar") || title_lower.contains("calendar") => {
            "\u{f073}"
        }
        _ if title_lower.contains("google translate") => "\u{f0f14}",
        _ if title_lower.contains("gmail") => "\u{f02ab}",
        _ if title_lower.contains("proton") => "\u{f023}",
        // Shopping / services
        _ if title_lower.contains("paypal") => "\u{f1ed}",
        _ if title_lower.contains("amazon") => "\u{f270}",
        _ if title_lower.contains("wikipedia") => "\u{f266}",
        // Cloud
        _ if title_lower.contains("azure") => "\u{ebd8}",
        _ if title_lower.contains("aws") => "\u{f270}",
        // Generic — keep google last as many titles contain "google"
        _ if title_lower.contains("google") => "\u{f1a0}",
        _ => return None,
    })
}

/// Language icon from file extension in title. Codepoints from saftladen languages config.
fn lang_icon(title_lower: &str) -> Option<&'static str> {
    Some(match () {
        _ if title_lower.contains(".rs") => "\u{e68b}",
        _ if title_lower.contains(".py") => "\u{e235}",
        _ if title_lower.contains(".ts") || title_lower.contains(".tsx") => "\u{f06e6}",
        _ if title_lower.contains(".js") || title_lower.contains(".jsx") => "\u{e74e}",
        _ if title_lower.contains(".lua") => "\u{f08b1}",
        _ if title_lower.contains(".html") => "\u{f13b}",
        _ if title_lower.contains(".hpp")
            || title_lower.contains(".cpp")
            || title_lower.contains(".cc") =>
        {
            "\u{e646}"
        }
        _ if title_lower.contains(".h")
            || title_lower.contains(".c ")
            || title_lower.ends_with(".c") =>
        {
            "\u{f0671}"
        }
        _ if title_lower.contains(".go") => "\u{e627}",
        _ if title_lower.contains(".json") => "\u{e60b}",
        _ if title_lower.contains(".sh") || title_lower.contains(".zsh") => "\u{e691}",
        _ if title_lower.contains(".nix") => "\u{f1105}",
        _ if title_lower.contains("dockerfile") => "\u{f308}",
        _ => return None,
    })
}

const TERMINAL_IDS: &[&str] = &[
    "kitty",
    "alacritty",
    "foot",
    "wezterm",
    "ghostty",
    "terminator",
    "gnome-terminal",
    "konsole",
    "xterm",
    "urxvt",
    "st",
];

fn is_terminal(app_id: Option<&str>) -> bool {
    app_id.is_some_and(|id| {
        let id = id.to_lowercase();
        TERMINAL_IDS.iter().any(|t| id.contains(t))
    })
}

/// Title-based icon for terminal windows and other apps without app_id icons.
/// Codepoints from saftladen programs config.
fn title_icon(app_id: Option<&str>, title: &str) -> String {
    let lower = title.to_lowercase();

    // Editor detection (vim/nvim in terminal title)
    if lower.starts_with("nvim")
        || lower.contains(" - nvim")
        || lower.contains("[nvim]")
        || lower.starts_with("vim")
        || lower.contains(" - vim")
    {
        let vim = "\u{e62b}";
        return if let Some(lang) = lang_icon(&lower) {
            format!("{vim}{lang}")
        } else {
            vim.to_string()
        };
    }

    // Programs
    let icon = if lower.contains("volume control") {
        "\u{f028}"
    } else if lower.contains("htop") {
        "\u{f0e4}"
    } else if lower.contains("cargo") {
        "\u{e68b}"
    } else if lower.contains("make") {
        "\u{f423}"
    } else if lower.contains("docker") {
        "\u{f308}"
    } else if lower.contains("kube") {
        "\u{f10fe}"
    } else if lower.contains("npm") {
        "\u{e71e}"
    } else if lower.contains("node") {
        "\u{f0399}"
    } else if lower.contains("psql") {
        "\u{e76e}"
    } else if lower.contains("man ") || lower.starts_with("man") {
        "\u{f15c}"
    } else if lower.contains("gdb") {
        "\u{f423}"
    } else if is_terminal(app_id) {
        "\u{f120}" // terminal prompt
    } else {
        "\u{f2d0}" // generic window
    };
    icon.to_string()
}

fn window_label(app_id: Option<&str>, title: &str) -> String {
    let title_lower = title.to_lowercase();

    // App-id-based icon
    if let Some(icon) = app_icon(app_id) {
        if is_browser(app_id) {
            // Check for known website icon — icon only
            if let Some(site_icon) = website_icon(&title_lower) {
                return format!("{icon}{site_icon}");
            }
            // Unknown site — browser icon only
            return icon.to_string();
        }
        // Known app — icon only
        return icon.to_string();
    }

    // Title-based icon (terminals, unknown apps)
    title_icon(app_id, title)
}

/// A column in niri's scrolling layout.
struct Column<'a> {
    tiles: Vec<&'a niri_ipc::Window>,
    /// This column contains the workspace's focused window.
    is_workspace_focused: bool,
}

/// Group windows into columns based on layout position.
/// `active_window_id` is the workspace's active window (from niri_ipc::Workspace).
fn build_columns<'a>(
    windows: &[&'a niri_ipc::Window],
    active_window_id: Option<u64>,
) -> Vec<Column<'a>> {
    use std::collections::BTreeMap;
    let mut columns: BTreeMap<usize, Column<'a>> = BTreeMap::new();
    let mut floating: Vec<&'a niri_ipc::Window> = Vec::new();

    for &win in windows {
        if let Some((col_idx, _tile_idx)) = win.layout.pos_in_scrolling_layout {
            let col = columns.entry(col_idx).or_insert_with(|| Column {
                tiles: Vec::new(),
                is_workspace_focused: false,
            });
            col.tiles.push(win);
            if active_window_id == Some(win.id) {
                col.is_workspace_focused = true;
            }
        } else {
            floating.push(win);
        }
    }

    for col in columns.values_mut() {
        col.tiles.sort_by_key(|w| {
            w.layout
                .pos_in_scrolling_layout
                .map(|(_, t)| t)
                .unwrap_or(0)
        });
    }

    let mut result: Vec<Column<'a>> = columns.into_values().collect();

    for win in floating {
        result.push(Column {
            tiles: vec![win],
            is_workspace_focused: active_window_id == Some(win.id),
        });
    }

    result
}

#[async_trait::async_trait]
impl StateItem for Windows {
    async fn print(&self, writer: &mut SectionWriter, output: &str) -> Result<(), Error> {
        let state = self.0.lock().await;

        // Find workspaces for this output, sorted by index
        let mut workspaces: Vec<&niri_ipc::Workspace> = state
            .workspaces
            .iter()
            .filter(|ws| ws.output.as_deref() == Some(output))
            .collect();
        workspaces.sort_by_key(|ws| ws.idx);

        // Collect all columns across visible workspaces
        let mut all_columns: Vec<(String, Option<RGBA>)> = Vec::new();
        for ws in workspaces.iter().filter(|ws| ws.is_active) {
            let mut windows: Vec<&niri_ipc::Window> = state
                .windows
                .iter()
                .filter(|w| w.workspace_id == Some(ws.id))
                .collect();
            if windows.is_empty() {
                continue;
            }
            windows.sort_by_key(|w| w.id);

            let columns = build_columns(&windows, ws.active_window_id);

            for col in &columns {
                let frame_color = if col.is_workspace_focused && ws.is_focused {
                    Some(ACCENT)
                } else if col.is_workspace_focused {
                    Some(GRAY)
                } else {
                    None
                };

                let content = if col.tiles.len() == 1 {
                    let title = col.tiles[0].title.as_deref().unwrap_or("?");
                    window_label(col.tiles[0].app_id.as_deref(), title)
                } else {
                    let active = col
                        .tiles
                        .iter()
                        .find(|w| ws.active_window_id == Some(w.id))
                        .unwrap_or(&col.tiles[0]);
                    let title = active.title.as_deref().unwrap_or("?");
                    let label = window_label(active.app_id.as_deref(), title);
                    format!("\u{f0328}{}{label}", col.tiles.len())
                };

                all_columns.push((content, frame_color));
            }
        }

        if all_columns.is_empty() {
            return Ok(());
        }

        writer.set_style(PowerlineStyle::Circle);
        writer.set_direction(PowerlineDirection::Left);
        writer.open(DARK_GRAY, LIGHT_GRAY);

        for (i, (content, frame_color)) in all_columns.iter().enumerate() {
            if i > 0 {
                writer.write_hspace(4);
            }
            let (circle_color, fg) = if let Some(color) = frame_color {
                (*color, WHITE)
            } else {
                (DARK_GRAY, LIGHT_GRAY)
            };
            writer.set_fg(fg);
            writer.write_circled(content.clone(), circle_color);
        }
        writer.set_direction(PowerlineDirection::Right);
        writer.close();
        Ok(())
    }

    fn start_coroutine(
        &self,
        main_action_sender: MainActionSender,
        item_action_receiver: ItemActionReceiver,
    ) -> JoinHandle<()> {
        tokio::spawn(windows_coroutine(
            self.0.clone(),
            main_action_sender,
            item_action_receiver,
        ))
    }
}

async fn windows_coroutine(
    state: SharedState,
    main_action_sender: MainActionSender,
    mut item_action_receiver: ItemActionReceiver,
) {
    // Initial fetch of windows and workspaces
    let initial_windows = match niri::niri_request(niri_ipc::Request::Windows).await {
        Ok(niri_ipc::Response::Windows(w)) => w,
        Ok(other) => {
            error!("Unexpected niri response for windows: {other:?}");
            return;
        }
        Err(err) => {
            error!("Failed to get initial windows: {err}");
            return;
        }
    };
    let initial_workspaces = match niri::niri_request(niri_ipc::Request::Workspaces).await {
        Ok(niri_ipc::Response::Workspaces(w)) => w,
        Ok(other) => {
            error!("Unexpected niri response for workspaces: {other:?}");
            return;
        }
        Err(err) => {
            error!("Failed to get initial workspaces: {err}");
            return;
        }
    };

    {
        let mut s = state.lock().await;
        s.windows = initial_windows;
        s.workspaces = initial_workspaces;
    }
    let _ = main_action_sender.enqueue(MainAction::Redraw).await;

    let mut lines = match niri::open_event_stream().await {
        Ok(l) => l,
        Err(err) => {
            error!("{err}");
            return;
        }
    };

    loop {
        tokio::select! {
            event = niri::next_event(&mut lines) => {
                match event {
                    Some(event) => {
                        let changed = handle_event(&state, event).await;
                        if changed && !main_action_sender.enqueue(MainAction::Redraw).await {
                            break;
                        }
                    }
                    None => {
                        debug!("niri event stream ended");
                        break;
                    }
                }
            }
            message = item_action_receiver.next() => {
                match message {
                    None | Some(ItemAction::Update) => {}
                    Some(ItemAction::Terminate) => break,
                }
            }
        }
    }
    debug!("windows coroutine exiting");
}

async fn handle_event(state: &SharedState, event: niri_ipc::Event) -> bool {
    match event {
        niri_ipc::Event::WindowsChanged { windows } => {
            let mut s = state.lock().await;
            s.windows = windows;

            true
        }
        niri_ipc::Event::WindowOpenedOrChanged { window } => {
            let mut s = state.lock().await;
            debug!(
                "WindowOpenedOrChanged: id={} app_id={:?} pos={:?}",
                window.id, window.app_id, window.layout.pos_in_scrolling_layout
            );
            if let Some(existing) = s.windows.iter_mut().find(|w| w.id == window.id) {
                *existing = window;
            } else {
                s.windows.push(window);
            }
            true
        }
        niri_ipc::Event::WindowClosed { id } => {
            state.lock().await.windows.retain(|w| w.id != id);
            true
        }
        niri_ipc::Event::WindowFocusChanged { id } => {
            let mut s = state.lock().await;
            // Update active_window_id on the workspace that contains the newly focused window
            if let Some(id) = id {
                let ws_id = s
                    .windows
                    .iter()
                    .find(|w| w.id == id)
                    .and_then(|w| w.workspace_id);
                if let Some(ws_id) = ws_id
                    && let Some(ws) = s.workspaces.iter_mut().find(|ws| ws.id == ws_id)
                {
                    ws.active_window_id = Some(id);
                }
            }
            for w in s.windows.iter_mut() {
                w.is_focused = Some(w.id) == id;
            }
            true
        }
        niri_ipc::Event::WorkspacesChanged { workspaces } => {
            state.lock().await.workspaces = workspaces;
            true
        }
        niri_ipc::Event::WorkspaceActivated { id, focused } => {
            let mut s = state.lock().await;
            let output = s
                .workspaces
                .iter()
                .find(|w| w.id == id)
                .and_then(|w| w.output.clone());
            for ws in s.workspaces.iter_mut() {
                if focused {
                    ws.is_focused = ws.id == id;
                }
                if ws.output == output {
                    ws.is_active = ws.id == id;
                }
            }
            true
        }
        niri_ipc::Event::WindowLayoutsChanged { changes } => {
            let mut s = state.lock().await;
            for (id, layout) in changes {
                if let Some(win) = s.windows.iter_mut().find(|w| w.id == id) {
                    win.layout = layout;
                }
            }
            true
        }
        niri_ipc::Event::WorkspaceActiveWindowChanged {
            workspace_id,
            active_window_id,
        } => {
            let mut s = state.lock().await;
            if let Some(ws) = s.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                ws.active_window_id = active_window_id;
            }
            true
        }
        other => {
            debug!("Unhandled niri event: {other:?}");
            false
        }
    }
}
