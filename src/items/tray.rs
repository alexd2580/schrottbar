use std::path::Path;
use std::sync::Arc;

use log::{debug, error, info, warn};
use tokio::{sync::Mutex, task::JoinHandle};
use zbus::{Connection, interface};

use smithay_client_toolkit::seat::pointer::BTN_RIGHT;

use crate::{
    error::Error,
    section_writer::{DARK_GRAY, SectionWriter},
    state_item::{ItemAction, ItemActionReceiver, MainAction, MainActionSender, StateItem},
    types::{ClickHandler, IconData, PowerlineDirection, PowerlineStyle},
};

// -- D-Bus interface: we implement the StatusNotifierWatcher ourselves --

struct WatcherState {
    items: Vec<String>,
    hosts: Vec<String>,
    /// Sender to notify the tray coroutine of changes.
    notify: tokio::sync::mpsc::UnboundedSender<WatcherEvent>,
}

#[derive(Debug)]
enum WatcherEvent {
    Registered(String),
    Unregistered(String),
    Changed(String),
}

struct StatusNotifierWatcherImpl {
    state: Arc<Mutex<WatcherState>>,
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl StatusNotifierWatcherImpl {
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) {
        // Some apps pass just a path; the bus name comes from the message sender.
        let full_service = if service.starts_with('/') {
            // service is an object path — prepend the sender's bus name
            if let Some(sender) = header.sender() {
                format!("{}{}", sender, service)
            } else {
                service.to_string()
            }
        } else {
            service.to_string()
        };

        let mut state = self.state.lock().await;
        if !state.items.contains(&full_service) {
            info!("SNI item registered: {full_service}");
            state.items.push(full_service.clone());
            let _ = state.notify.send(WatcherEvent::Registered(full_service));
        }
    }

    async fn register_status_notifier_host(&self, service: &str) {
        let mut state = self.state.lock().await;
        if !state.hosts.contains(&service.to_string()) {
            debug!("SNI host registered: {service}");
            state.hosts.push(service.to_string());
        }
    }

    #[zbus(property)]
    async fn registered_status_notifier_items(&self) -> Vec<String> {
        self.state.lock().await.items.clone()
    }

    #[zbus(property)]
    async fn is_status_notifier_host_registered(&self) -> bool {
        !self.state.lock().await.hosts.is_empty()
    }

    #[zbus(property)]
    async fn protocol_version(&self) -> i32 {
        0
    }
}

// -- D-Bus proxy for StatusNotifierItem (on each tray app) --

#[zbus::proxy(interface = "org.kde.StatusNotifierItem")]
trait StatusNotifierItem {
    #[zbus(property)]
    fn icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;

    #[zbus(property)]
    fn icon_name(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn title(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn icon_theme_path(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn status(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn menu(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;

    fn activate(&self, x: i32, y: i32) -> zbus::Result<()>;
    fn secondary_activate(&self, x: i32, y: i32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn new_icon(&self);

    #[zbus(signal)]
    fn new_status(&self, status: String);
}

// -- D-Bus proxy for com.canonical.dbusmenu (used by libappindicator/ayatana apps) --

#[zbus::proxy(interface = "com.canonical.dbusmenu")]
trait DbusMenu {
    /// Send an event to a menu item. event_id is typically "clicked".
    fn event(
        &self,
        id: i32,
        event_id: &str,
        data: &zbus::zvariant::Value<'_>,
        timestamp: u32,
    ) -> zbus::Result<()>;

    /// Get the layout tree. Returns (revision, layout).
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: &[&str],
    ) -> zbus::Result<(
        u32,
        (
            i32,
            std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
            Vec<zbus::zvariant::OwnedValue>,
        ),
    )>;
}

// -- Icon conversion --

/// Convert SNI IconPixmap (big-endian ARGB) to RGBA and scale to target height.
fn convert_icon(pixmaps: &[(i32, i32, Vec<u8>)], target_height: u32) -> Option<IconData> {
    if pixmaps.is_empty() {
        return None;
    }

    // Pick the size closest to target_height, preferring >= target.
    let best = pixmaps
        .iter()
        .min_by_key(|(_, h, _)| {
            let h = *h as u32;
            if h >= target_height {
                h - target_height
            } else {
                (target_height - h) + 1000 // penalize smaller
            }
        })
        .unwrap();

    let (sw, sh, data) = best;
    let sw = *sw as u32;
    let sh = *sh as u32;
    if sw == 0 || sh == 0 || data.len() < (sw * sh * 4) as usize {
        return None;
    }

    // Convert big-endian ARGB to native RGBA.
    let mut rgba = vec![0u8; (sw * sh * 4) as usize];
    for i in 0..(sw * sh) as usize {
        let src = i * 4;
        let a = data[src];
        let r = data[src + 1];
        let g = data[src + 2];
        let b = data[src + 3];
        rgba[src] = r;
        rgba[src + 1] = g;
        rgba[src + 2] = b;
        rgba[src + 3] = a;
    }

    scale_rgba(&rgba, sw, sh, target_height)
}

/// Load a PNG file and return RGBA pixel data, scaled to target_height.
fn load_png_icon(path: &Path, target_height: u32) -> Option<IconData> {
    let file = std::fs::File::open(path).ok()?;
    let decoder = png::Decoder::new(file);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());

    let sw = info.width;
    let sh = info.height;

    // Convert to RGBA if needed.
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity((sw * sh * 4) as usize);
            for chunk in buf.chunks_exact(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        png::ColorType::GrayscaleAlpha => {
            let mut rgba = Vec::with_capacity((sw * sh * 4) as usize);
            for chunk in buf.chunks_exact(2) {
                rgba.push(chunk[0]);
                rgba.push(chunk[0]);
                rgba.push(chunk[0]);
                rgba.push(chunk[1]);
            }
            rgba
        }
        png::ColorType::Grayscale => {
            let mut rgba = Vec::with_capacity((sw * sh * 4) as usize);
            for &g in &buf {
                rgba.push(g);
                rgba.push(g);
                rgba.push(g);
                rgba.push(255);
            }
            rgba
        }
        _ => return None,
    };

    if rgba.len() < (sw * sh * 4) as usize {
        return None;
    }

    scale_rgba(&rgba, sw, sh, target_height)
}

/// Scale RGBA pixel data to fit target_height, preserving aspect ratio.
/// Uses bilinear interpolation for smooth scaling.
#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
fn scale_rgba(rgba: &[u8], sw: u32, sh: u32, target_height: u32) -> Option<IconData> {
    let th = target_height;
    let tw = if sh == 0 {
        target_height
    } else {
        (sw as f64 * th as f64 / sh as f64).round() as u32
    };

    if sw == tw && sh == th {
        return Some(IconData {
            width: tw,
            height: th,
            pixels: rgba.to_vec(),
        });
    }

    let mut scaled = vec![0u8; (tw * th * 4) as usize];
    for dy in 0..th {
        for dx in 0..tw {
            // Map destination pixel center to source coordinates.
            let sx = (dx as f64 + 0.5) * sw as f64 / tw as f64 - 0.5;
            let sy = (dy as f64 + 0.5) * sh as f64 / th as f64 - 0.5;

            let x0 = sx.floor() as i64;
            let y0 = sy.floor() as i64;
            let fx = (sx - x0 as f64) as f32;
            let fy = (sy - y0 as f64) as f32;

            // Sample 2x2 neighborhood with clamping.
            let sample = |x: i64, y: i64| -> [f32; 4] {
                let x = x.clamp(0, sw as i64 - 1) as u32;
                let y = y.clamp(0, sh as i64 - 1) as u32;
                let i = ((y * sw + x) * 4) as usize;
                [
                    rgba[i] as f32,
                    rgba[i + 1] as f32,
                    rgba[i + 2] as f32,
                    rgba[i + 3] as f32,
                ]
            };

            let s00 = sample(x0, y0);
            let s10 = sample(x0 + 1, y0);
            let s01 = sample(x0, y0 + 1);
            let s11 = sample(x0 + 1, y0 + 1);

            let dst = ((dy * tw + dx) * 4) as usize;
            for c in 0..4 {
                let top = s00[c] * (1.0 - fx) + s10[c] * fx;
                let bot = s01[c] * (1.0 - fx) + s11[c] * fx;
                scaled[dst + c] = (top * (1.0 - fy) + bot * fy).round() as u8;
            }
        }
    }

    Some(IconData {
        width: tw,
        height: th,
        pixels: scaled,
    })
}

/// Try to find an icon file by name in a theme path directory.
fn find_icon_in_theme_path(theme_path: &str, icon_name: &str) -> Option<std::path::PathBuf> {
    let dir = Path::new(theme_path);
    // Try common extensions.
    for ext in &["png", "svg", "xpm"] {
        let path = dir.join(format!("{icon_name}.{ext}"));
        if path.exists() {
            return Some(path);
        }
    }
    None
}

// -- Shared state --

#[derive(Clone)]
struct TrayItemData {
    service: String,
    icon: Option<IconData>,
    status: String,
}

struct TrayData {
    items: Vec<TrayItemData>,
    icon_height: u32,
    conn: Option<Connection>,
}

type SharedData = Arc<Mutex<TrayData>>;

pub struct Tray {
    data: SharedData,
}

impl Tray {
    pub fn new(bar_height: u32) -> Self {
        let icon_height = bar_height;
        Self {
            data: Arc::new(Mutex::new(TrayData {
                items: Vec::new(),
                icon_height,
                conn: None,
            })),
        }
    }
}

#[async_trait::async_trait]
impl StateItem for Tray {
    async fn print(&self, writer: &mut SectionWriter, _output: &str) -> Result<(), Error> {
        let data = self.data.lock().await;
        let conn = data.conn.clone();

        let has_visible = data.items.iter().any(|i| i.status != "Passive" && i.icon.is_some());
        if !has_visible {
            return Ok(());
        }

        writer.set_style(PowerlineStyle::Circle);
        writer.set_direction(PowerlineDirection::Left);
        writer.open_bg(DARK_GRAY);

        for item in &data.items {
            if item.status == "Passive" {
                continue;
            }
            if let Some(ref conn) = conn {
                let service = item.service.clone();
                let conn = conn.clone();
                writer.set_on_click(Arc::new(move |button| {
                    let service = service.clone();
                    let conn = conn.clone();
                    tokio::spawn(async move {
                        tray_click(&conn, &service, button).await;
                    });
                }) as ClickHandler);
            }
            if let Some(ref icon) = item.icon {
                writer.write_icon(icon.clone());
            }
            writer.clear_on_click();
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
        tokio::spawn(tray_coroutine(
            self.data.clone(),
            main_action_sender,
            item_action_receiver,
        ))
    }
}

// -- Parse service string into (bus_name, object_path) --

fn parse_sni_service(service: &str) -> (String, String) {
    // Format can be "bus_name", "bus_name/path", or ":1.23/path/to/item"
    if let Some((name, path)) = service.split_once('/') {
        (name.to_string(), format!("/{path}"))
    } else {
        (service.to_string(), "/StatusNotifierItem".to_string())
    }
}

// -- Fetch icon for a single item --

async fn fetch_item(conn: &Connection, service: &str, icon_height: u32) -> Option<TrayItemData> {
    let (bus_name, path) = parse_sni_service(service);
    let proxy = StatusNotifierItemProxy::builder(conn)
        .destination(bus_name.as_str())
        .ok()?
        .path(path.as_str())
        .ok()?
        .build()
        .await
        .ok()?;

    let status = proxy.status().await.unwrap_or_else(|_| "Active".into());

    // Try IconPixmap first, then fall back to IconThemePath + IconName.
    let pixmaps = proxy.icon_pixmap().await.unwrap_or_default();
    let mut icon = convert_icon(&pixmaps, icon_height);

    if icon.is_none() {
        let name = proxy.icon_name().await.unwrap_or_default();
        let theme_path = proxy.icon_theme_path().await.unwrap_or_default();

        if !name.is_empty() && !theme_path.is_empty()
            && let Some(file_path) = find_icon_in_theme_path(&theme_path, &name) {
                debug!("Tray item {service}: loading icon from {}", file_path.display());
                icon = load_png_icon(&file_path, icon_height);
            }

        if icon.is_none() && !name.is_empty() {
            debug!("Tray item {service}: icon_name={name}, theme_path={theme_path:?} — could not load icon");
        } else if icon.is_none() {
            debug!("Tray item {service}: no pixmap and no icon_name");
        }
    }

    Some(TrayItemData {
        service: service.to_string(),
        icon,
        status,
    })
}

// -- Watch for NameOwnerChanged to detect items disappearing --

async fn watch_name_lost(
    conn: Connection,
    watcher_state: Arc<Mutex<WatcherState>>,
    tray_data: SharedData,
    main_action_sender: MainActionSender,
) {
    use futures::StreamExt;

    let dbus_proxy = match zbus::fdo::DBusProxy::new(&conn).await {
        Ok(p) => p,
        Err(e) => {
            warn!("Failed to create DBus proxy for NameOwnerChanged: {e}");
            return;
        }
    };

    let mut stream = match dbus_proxy.receive_name_owner_changed().await {
        Ok(s) => s,
        Err(e) => {
            warn!("Failed to subscribe to NameOwnerChanged: {e}");
            return;
        }
    };

    while let Some(signal) = stream.next().await {
        if let Ok(args) = signal.args() {
            let name = args.name.as_str();
            let new_owner = args.new_owner.as_ref().map(|s| s.as_str()).unwrap_or("");

            // If new_owner is empty, the name was lost.
            if new_owner.is_empty() {
                let mut ws = watcher_state.lock().await;
                let before = ws.items.len();
                ws.items.retain(|svc| {
                    let (bus, _) = parse_sni_service(svc);
                    bus != name
                });
                if ws.items.len() < before {
                    info!("SNI item vanished (bus name lost): {name}");
                    let _ = ws.notify.send(WatcherEvent::Unregistered(name.to_string()));
                    // Also remove from tray data.
                    tray_data.lock().await.items.retain(|i| {
                        let (bus, _) = parse_sni_service(&i.service);
                        bus != name
                    });
                    let _ = main_action_sender.enqueue(MainAction::Redraw).await;
                }
            }
        }
    }
}

// -- Watch individual item signals (NewIcon, NewStatus) --

async fn watch_item_signals(
    conn: Connection,
    service: String,
    notify: tokio::sync::mpsc::UnboundedSender<WatcherEvent>,
) {
    use futures::StreamExt;

    let (bus_name, path) = parse_sni_service(&service);
    let proxy = match StatusNotifierItemProxy::builder(&conn)
        .destination(bus_name.as_str())
        .ok()
        .and_then(|b| b.path(path.as_str()).ok())
    {
        Some(b) => match b.build().await {
            Ok(p) => p,
            Err(_) => return,
        },
        None => return,
    };

    let mut icon_stream = match proxy.receive_new_icon().await {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut status_stream = match proxy.receive_new_status().await {
        Ok(s) => s,
        Err(_) => return,
    };

    loop {
        tokio::select! {
            Some(_) = icon_stream.next() => {
                debug!("NewIcon signal from {service}");
                let _ = notify.send(WatcherEvent::Changed(service.clone()));
            }
            Some(_) = status_stream.next() => {
                debug!("NewStatus signal from {service}");
                let _ = notify.send(WatcherEvent::Changed(service.clone()));
            }
            else => break,
        }
    }
}

// -- Click handling --

async fn tray_click(conn: &Connection, service: &str, button: u32) {
    let (bus_name, path) = parse_sni_service(service);
    let proxy = match StatusNotifierItemProxy::builder(conn)
        .destination(bus_name.as_str())
        .ok()
        .and_then(|b| b.path(path.as_str()).ok())
    {
        Some(b) => match b.build().await {
            Ok(p) => p,
            Err(e) => {
                debug!("Tray click proxy build: {e}");
                return;
            }
        },
        None => return,
    };
    if button == BTN_RIGHT {
        if let Err(e) = proxy.secondary_activate(0, 0).await {
            debug!("Tray right-click {service}: {e}");
        }
    } else {
        // Try Activate first; some apps (e.g. Steam/ayatana) don't implement it.
        match proxy.activate(0, 0).await {
            Ok(()) => return,
            Err(e) => debug!("Tray Activate unavailable for {service}: {e}"),
        }
        // Fall back to DBusMenu: activate the first non-separator menu item.
        if let Ok(menu_path) = proxy.menu().await {
            dbusmenu_activate_first(conn, &bus_name, menu_path.as_str()).await;
        }
    }
}

/// Activate the first item after the first separator in a com.canonical.dbusmenu menu.
/// This skips "recent items" sections (e.g. Steam's recently played games) and hits
/// the main action (e.g. "Store", "Library") which typically opens the app window.
/// If there are no separators, activates the first labelled item.
async fn dbusmenu_activate_first(conn: &Connection, bus_name: &str, menu_path: &str) {
    let menu_proxy = match DbusMenuProxy::builder(conn)
        .destination(bus_name)
        .ok()
        .and_then(|b| b.path(menu_path).ok())
    {
        Some(b) => match b.build().await {
            Ok(p) => p,
            Err(e) => {
                debug!("DBusMenu proxy build: {e}");
                return;
            }
        },
        None => return,
    };

    // Get top-level children (depth=1) with their labels.
    let layout = match menu_proxy.get_layout(0, 1, &["label", "type"]).await {
        Ok((_, layout)) => layout,
        Err(e) => {
            debug!("DBusMenu GetLayout: {e}");
            return;
        }
    };

    use zbus::zvariant::Value;

    // Parse children into (id, is_separator, has_label) tuples.
    struct MenuItem {
        id: i32,
        is_separator: bool,
        has_label: bool,
    }

    let items: Vec<MenuItem> = layout
        .2
        .iter()
        .filter_map(|child_val| {
            let Value::Structure(fields) = &**child_val else {
                return None;
            };
            let fields = fields.fields();
            let Value::I32(id) = fields.first()? else {
                return None;
            };
            let (is_separator, has_label) = if let Some(Value::Dict(props)) = fields.get(1) {
                let is_sep = props
                    .get::<&str, Value>(&"type")
                    .ok()
                    .flatten()
                    .is_some_and(|v| matches!(v, Value::Str(s) if s.as_str() == "separator"));
                let has_lbl = props
                    .get::<&str, Value>(&"label")
                    .ok()
                    .flatten()
                    .is_some_and(|v| matches!(v, Value::Str(s) if !s.is_empty()));
                (is_sep, has_lbl)
            } else {
                (false, false)
            };
            Some(MenuItem { id: *id, is_separator, has_label })
        })
        .collect();

    // Find the first separator, then the first labelled item after it.
    let first_sep = items.iter().position(|i| i.is_separator);
    let target = if let Some(sep_idx) = first_sep {
        items[sep_idx + 1..]
            .iter()
            .find(|i| !i.is_separator && i.has_label)
    } else {
        // No separators — just pick the first labelled item.
        items.iter().find(|i| !i.is_separator && i.has_label)
    };

    if let Some(item) = target {
        let empty = Value::from("");
        if let Err(e) = menu_proxy.event(item.id, "clicked", &empty, 0).await {
            debug!("DBusMenu Event({}): {e}", item.id);
        }
    } else {
        debug!("DBusMenu: no activatable items found");
    }
}

// -- Main coroutine --

async fn tray_coroutine(
    data: SharedData,
    main_action_sender: MainActionSender,
    mut item_action_receiver: ItemActionReceiver,
) {
    let conn = match Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to connect to D-Bus session bus: {e}");
            return;
        }
    };

    // Share the connection so click handlers can reuse it.
    data.lock().await.conn = Some(conn.clone());

    // Set up our own StatusNotifierWatcher.
    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::unbounded_channel();
    let watcher_state = Arc::new(Mutex::new(WatcherState {
        items: Vec::new(),
        hosts: Vec::new(),
        notify: notify_tx,
    }));

    let watcher_impl = StatusNotifierWatcherImpl {
        state: watcher_state.clone(),
    };

    // Serve the watcher interface on /StatusNotifierWatcher.
    if let Err(e) = conn.object_server().at("/StatusNotifierWatcher", watcher_impl).await {
        error!("Failed to serve StatusNotifierWatcher: {e}");
        return;
    }

    // Request the well-known name so apps can find us.
    if let Err(e) = conn
        .request_name("org.kde.StatusNotifierWatcher")
        .await
    {
        warn!("Failed to claim org.kde.StatusNotifierWatcher: {e}");
        // Another watcher might be running — fall back to using it as a client.
        // But since we checked earlier and it wasn't there, this is likely a race. Continue anyway.
    }

    // Also register ourselves as a host.
    let host_name = format!("org.kde.StatusNotifierHost-{}", std::process::id());
    if let Err(e) = conn.request_name(host_name.as_str()).await {
        warn!("Failed to request host name {host_name}: {e}");
    }
    {
        let mut ws = watcher_state.lock().await;
        ws.hosts.push(host_name);
    }

    info!("StatusNotifierWatcher active — waiting for tray items");

    let icon_height = data.lock().await.icon_height;

    // Watch for bus names disappearing.
    tokio::spawn(watch_name_lost(
        conn.clone(),
        watcher_state.clone(),
        data.clone(),
        main_action_sender.clone(),
    ));

    loop {
        tokio::select! {
            Some(event) = notify_rx.recv() => {
                match event {
                    WatcherEvent::Registered(svc) => {
                        debug!("Fetching icon for new tray item: {svc}");
                        if let Some(item) = fetch_item(&conn, &svc, icon_height).await {
                            data.lock().await.items.push(item);
                            let _ = main_action_sender.enqueue(MainAction::Redraw).await;
                        }
                        // Watch for icon/status changes on this item.
                        tokio::spawn(watch_item_signals(
                            conn.clone(),
                            svc,
                            watcher_state.lock().await.notify.clone(),
                        ));
                    }
                    WatcherEvent::Unregistered(svc) => {
                        debug!("Tray item unregistered: {svc}");
                        data.lock().await.items.retain(|i| i.service != svc);
                        let _ = main_action_sender.enqueue(MainAction::Redraw).await;
                    }
                    WatcherEvent::Changed(svc) => {
                        debug!("Tray item changed: {svc}");
                        if let Some(updated) = fetch_item(&conn, &svc, icon_height).await {
                            let mut d = data.lock().await;
                            if let Some(existing) = d.items.iter_mut().find(|i| i.service == svc) {
                                *existing = updated;
                            }
                            drop(d);
                            let _ = main_action_sender.enqueue(MainAction::Redraw).await;
                        }
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
    debug!("tray coroutine exiting");
}
