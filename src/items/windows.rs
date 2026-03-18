use std::sync::Arc;

use log::{debug, error};
use tokio::{sync::Mutex, task::JoinHandle};

use crate::{
    error::Error,
    section_writer::{SectionWriter, ACCENT, DARK_GRAY, LIGHT_GRAY, WHITE},
    state_item::{
        ItemAction, ItemActionReceiver, MainAction, MainActionSender, StateItem,
    },
    types::{PowerlineDirection, PowerlineStyle},
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
        _ if id.contains("nautilus") || id.contains("thunar") || id.contains("nemo") || id.contains("dolphin") => "\u{f07c}",
        _ if id.contains("evince") || id.contains("zathura") || id.contains("okular") => "\u{f1c1}",
        _ if id.contains("obs") => "\u{f03d}",
        // Editors (GUI)
        _ if id.contains("code") && id.contains("visual") => "\u{e70c}",
        _ if id.contains("neovide") => "\u{e62b}",
        // Terminals — fall through to title-based
        _ if id.contains("kitty") || id.contains("alacritty") || id.contains("foot")
            || id.contains("wezterm") || id.contains("ghostty") => return None,
        _ => return None,
    })
}

/// Website icon from browser title. Codepoints from saftladen websites config.
fn website_icon(title_lower: &str) -> Option<&'static str> {
    Some(match () {
        _ if title_lower.contains("telegram") => "\u{e217}",
        _ if title_lower.contains("slack") => "\u{f198}",
        _ if title_lower.contains("github") => "\u{f408}",
        _ if title_lower.contains("gitlab") => "\u{f296}",
        _ if title_lower.contains("stack overflow") => "\u{f16c}",
        _ if title_lower.contains("youtube") => "\u{f16a}",
        _ if title_lower.contains("jira") => "\u{f0303}",
        _ if title_lower.contains("paypal") => "\u{f1ed}",
        _ if title_lower.contains("gmail") => "\u{f02ab}",
        _ if title_lower.contains("amazon") => "\u{f270}",
        _ if title_lower.contains("google") => "\u{f1a0}",
        _ if title_lower.contains("azure") => "\u{ebd8}",
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
        _ if title_lower.contains(".hpp") || title_lower.contains(".cpp") || title_lower.contains(".cc") => "\u{e646}",
        _ if title_lower.contains(".h") || title_lower.contains(".c ") || title_lower.ends_with(".c") => "\u{f0671}",
        _ if title_lower.contains(".go") => "\u{e627}",
        _ if title_lower.contains(".json") => "\u{e60b}",
        _ if title_lower.contains(".sh") || title_lower.contains(".zsh") => "\u{e691}",
        _ if title_lower.contains(".nix") => "\u{f1105}",
        _ if title_lower.contains("dockerfile") => "\u{f308}",
        _ => return None,
    })
}

/// Title-based icon for terminal windows and other apps without app_id icons.
/// Codepoints from saftladen programs config.
fn title_icon(title: &str) -> &'static str {
    let lower = title.to_lowercase();

    // Editor detection (vim/nvim in terminal title)
    if lower.starts_with("nvim") || lower.contains(" - nvim") || lower.contains("[nvim]")
        || lower.starts_with("vim") || lower.contains(" - vim")
    {
        if let Some(icon) = lang_icon(&lower) {
            return icon;
        }
        return "\u{e62b}";  // vim
    }

    // Programs
    if lower.contains("htop") { return "\u{f0e4}"; }
    if lower.contains("cargo") { return "\u{e68b}"; }
    if lower.contains("make") { return "\u{f423}"; }
    if lower.contains("docker") { return "\u{f308}"; }
    if lower.contains("kube") { return "\u{f10fe}"; }
    if lower.contains("npm") { return "\u{e71e}"; }
    if lower.contains("node") { return "\u{f0399}"; }
    if lower.contains("psql") { return "\u{e76e}"; }
    if lower.contains("man ") || lower.starts_with("man") { return "\u{f15c}"; }
    if lower.contains("gdb") { return "\u{f423}"; }

    // Generic terminal / zsh prompt
    "\u{f120}"
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
            // Unknown site — show browser icon + page title
            let page = title
                .rsplit_once(" — ")
                .or_else(|| title.rsplit_once(" - "))
                .map(|(page, _)| page)
                .unwrap_or(title);
            return format!("{icon} {}", shorten_chars(page, 25));
        }
        // Known app — icon only
        return icon.to_string();
    }

    // Title-based icon (terminals, unknown apps)
    let icon = title_icon(title);
    icon.to_string()
}

fn shorten_chars(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        text.to_string()
    } else {
        let mut s: String = text.chars().take(max_len - 1).collect();
        s.push('…');
        s
    }
}

#[async_trait::async_trait]
impl StateItem for Windows {
    async fn print(&self, writer: &mut SectionWriter, output: &str) -> Result<(), Error> {
        let state = self.0.lock().await;

        // Find the active workspace for this output
        let active_ws = state.workspaces.iter().find(|ws| {
            ws.output.as_deref() == Some(output) && ws.is_active
        });
        let Some(active_ws) = active_ws else {
            return Ok(());
        };

        writer.set_style(PowerlineStyle::Octagon);
        writer.set_direction(PowerlineDirection::Right);

        let mut windows: Vec<&niri_ipc::Window> = state
            .windows
            .iter()
            .filter(|w| w.workspace_id == Some(active_ws.id))
            .collect();
        windows.sort_by_key(|w| w.id);

        for win in &windows {
            let title = win.title.as_deref().unwrap_or("?");
            let label = window_label(win.app_id.as_deref(), title);

            if win.is_focused {
                writer.open(ACCENT, WHITE);
            } else {
                writer.open(DARK_GRAY, LIGHT_GRAY);
            }
            writer.write(format!(" {label} "));
        }
        if !windows.is_empty() {
            writer.close();
        }
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
            state.lock().await.windows = windows;
            true
        }
        niri_ipc::Event::WindowOpenedOrChanged { window } => {
            let mut s = state.lock().await;
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
            let output = s.workspaces.iter().find(|w| w.id == id).and_then(|w| w.output.clone());
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
        _ => false,
    }
}

// -- Demo item showing all window icons --

const ALL_ICONS: &[(&str, &str)] = &[
    // App icons
    ("firefox", "\u{f269}"),
    ("chrome", "\u{f268}"),
    ("discord", "\u{f066f}"),
    ("telegram", "\u{e217}"),
    ("slack", "\u{f198}"),
    ("mumble", "\u{f130}"),
    ("irssi", "\u{f292}"),
    ("spotify", "\u{f1bc}"),
    ("steam", "\u{ed29}"),
    ("vlc", "\u{f057c}"),
    ("gimp", "\u{e67f}"),
    ("vscode", "\u{e70c}"),
    ("vim", "\u{e62b}"),
    // Website icons
    ("github", "\u{f408}"),
    ("gitlab", "\u{f296}"),
    ("SO", "\u{f16c}"),
    ("youtube", "\u{f16a}"),
    ("jira", "\u{f0303}"),
    ("paypal", "\u{f1ed}"),
    ("gmail", "\u{f02ab}"),
    ("amazon", "\u{f270}"),
    ("google", "\u{f1a0}"),
    ("azure", "\u{ebd8}"),
    // Language icons
    ("rust", "\u{e68b}"),
    ("python", "\u{e235}"),
    ("ts", "\u{f06e6}"),
    ("js", "\u{e74e}"),
    ("lua", "\u{f08b1}"),
    ("html", "\u{f13b}"),
    ("c++", "\u{e646}"),
    ("c", "\u{f0671}"),
    ("go", "\u{e627}"),
    ("json", "\u{e60b}"),
    ("shell", "\u{e691}"),
    ("nix", "\u{f1105}"),
    ("docker", "\u{f308}"),
    // Program icons
    ("htop", "\u{f0e4}"),
    ("make", "\u{f423}"),
    ("kube", "\u{f10fe}"),
    ("npm", "\u{e71e}"),
    ("node", "\u{f0399}"),
    ("psql", "\u{e76e}"),
    ("man", "\u{f15c}"),
    ("zsh", "\u{f120}"),
];

pub struct WindowIconsDemo;

impl WindowIconsDemo {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl StateItem for WindowIconsDemo {
    async fn print(&self, writer: &mut SectionWriter, _output: &str) -> Result<(), Error> {
        writer.set_style(PowerlineStyle::Octagon);
        writer.set_direction(PowerlineDirection::Right);

        writer.open(DARK_GRAY, LIGHT_GRAY);
        for (_label, icon) in ALL_ICONS {
            writer.write(format!(" {icon}"));
        }
        writer.write(" ".to_string());
        writer.close();
        Ok(())
    }

    fn start_coroutine(
        &self,
        _main_action_sender: MainActionSender,
        mut item_action_receiver: ItemActionReceiver,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    message = item_action_receiver.next() => {
                        match message {
                            None | Some(ItemAction::Update) => {}
                            Some(ItemAction::Terminate) => break,
                        }
                    }
                }
            }
        })
    }
}
