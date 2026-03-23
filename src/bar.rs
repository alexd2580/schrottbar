use std::collections::HashMap;

use log::{debug, info};
use tiny_skia::PixmapMut;
use tokio::io::unix::AsyncFd;
use wayland_client::{Connection, EventQueue, protocol::wl_shm};

use crate::renderer::{self, Renderer};
use crate::types::{Alignment, ClickHandler, ContentItem, ContentShape, HoverFlag, PowerlineDirection, PowerlineFill, PowerlineStyle};
use crate::wayland::{BarEvent, OutputInfo, WaylandState};

use smithay_client_toolkit::shell::WaylandSurface;

struct HitZone {
    x_start: u32,
    x_end: u32,
    handler: ClickHandler,
}

struct HoverZone {
    x_start: u32,
    x_end: u32,
    flag: HoverFlag,
}

pub struct Bar {
    height: u32,
    renderer: Renderer,
    conn: Connection,
    event_queue: EventQueue<WaylandState>,
    wayland: WaylandState,
    hit_zones: HashMap<usize, Vec<HitZone>>,
    hover_zones: HashMap<usize, Vec<HoverZone>>,
    /// Track which hover zone is currently active per surface, so we can send leave.
    active_hover: HashMap<usize, usize>,
    /// Last known pointer X per surface, for re-evaluating hover after redraw.
    pointer_pos: HashMap<usize, f64>,
}

impl Bar {
    pub fn new() -> Self {
        let conn = Connection::connect_to_env().expect("Failed to connect to Wayland");
        let (mut wayland, mut event_queue) = WaylandState::new(&conn);

        let font_family = "UbuntuMono Nerd Font Propo";
        let font_size = 19.0;
        let renderer = Renderer::new(font_family, font_size);
        let height = renderer.height();
        info!(
            "Bar height: {height} (ascent={} descent={} padding={})",
            renderer.ascent(),
            renderer.descent(),
            Renderer::PADDING
        );

        let qh = event_queue.handle();
        wayland.create_surfaces(&qh, height);

        // Do another roundtrip to get initial configure events.
        event_queue.roundtrip(&mut wayland).unwrap();

        Self {
            height,
            renderer,
            conn,
            event_queue,
            wayland,
            hit_zones: HashMap::new(),
            hover_zones: HashMap::new(),
            active_hover: HashMap::new(),
            pointer_pos: HashMap::new(),
        }
    }

    pub fn outputs(&self) -> &[OutputInfo] {
        &self.wayland.outputs
    }

    #[allow(dead_code)]
    pub fn height(&self) -> u32 {
        self.height
    }

    #[allow(dead_code)]
    pub fn clear_monitors(&mut self) {
        for surface in &mut self.wayland.surfaces {
            if !surface.configured {
                continue;
            }
            // We'll clear when we draw — no need for a separate clear pass
            // since we create a fresh buffer each frame.
        }
    }

    pub fn draw(
        &mut self,
        monitor_index: usize,
        left: &[ContentItem],
        center: &[ContentItem],
        right: &[ContentItem],
    ) {
        if !self.wayland.surfaces[monitor_index].configured {
            return;
        }

        // Measure all item widths before borrowing the surface mutably.
        let height = self.height;
        let sections: Vec<(Alignment, &[ContentItem], Vec<u32>)> = [
            (Alignment::Left, left),
            (Alignment::Center, center),
            (Alignment::Right, right),
        ]
        .into_iter()
        .map(|(align, items)| {
            let widths: Vec<u32> = items.iter().map(|item| self.item_width(item)).collect();
            (align, items, widths)
        })
        .collect();

        let surface = &mut self.wayland.surfaces[monitor_index];
        let width = surface.width;
        let stride = width as i32 * 4;

        assert_eq!(height, self.height, "bar height changed between frames");
        assert_eq!(
            surface.height, height,
            "surface height({}) != bar height({height})",
            surface.height
        );

        let expected_canvas_size = (width * height * 4) as usize;
        let (buffer, canvas) = surface
            .pool
            .create_buffer(
                width as i32,
                height as i32,
                stride,
                wl_shm::Format::Argb8888,
            )
            .expect("Failed to create buffer");

        assert_eq!(
            canvas.len(),
            expected_canvas_size,
            "canvas size({}) != expected({})",
            canvas.len(),
            expected_canvas_size
        );

        // Clear to transparent (BGRA format for wl_shm Argb8888).
        canvas.fill(0x00);

        // Wrap canvas as a tiny-skia PixmapMut (RGBA byte order).
        // tiny-skia interprets our BGRA as "RGBA" — so we pass swapped colors.
        let mut pixmap =
            PixmapMut::from_bytes(canvas, width, height).expect("Failed to create pixmap");

        assert_eq!(pixmap.width(), width, "pixmap width mismatch");
        assert_eq!(pixmap.height(), height, "pixmap height mismatch");

        // Collect hit/hover zones for this monitor.
        let mut zones = Vec::new();
        let mut hover_zones_vec: Vec<HoverZone> = Vec::new();

        // Render each alignment section in three passes:
        // 1. Backgrounds  2. Frames  3. Content (text, shapes)
        // This layering lets frame outlines sit on top of backgrounds
        // but underneath text from neighboring items.
        for (alignment, items, item_widths) in &sections {
            let total_width: u32 = item_widths.iter().sum();

            let start = match alignment {
                Alignment::Left => 0u32,
                Alignment::Center => width.saturating_sub(total_width) / 2,
                Alignment::Right => width.saturating_sub(total_width),
            };

            // Pre-compute positions.
            let positions: Vec<u32> = {
                let mut pos = Vec::with_capacity(item_widths.len());
                let mut cursor = start;
                for &w in item_widths.iter() {
                    pos.push(cursor);
                    cursor += w;
                }
                pos
            };

            // Collect hit/hover zones from items with handlers.
            for (i, (item, &w)) in items.iter().zip(item_widths.iter()).enumerate() {
                if let Some(ref handler) = item.on_click {
                    zones.push(HitZone {
                        x_start: positions[i],
                        x_end: positions[i] + w,
                        handler: handler.clone(),
                    });
                }
                if let Some(ref flag) = item.hover_flag {
                    hover_zones_vec.push(HoverZone {
                        x_start: positions[i],
                        x_end: positions[i] + w,
                        flag: flag.clone(),
                    });
                }
            }

            // Pass 1: backgrounds (fades are drawn with content below)
            for (i, (item, &w)) in items.iter().zip(item_widths.iter()).enumerate() {
                if matches!(item.shape, ContentShape::Powerline(PowerlineStyle::Fade, PowerlineFill::Full, ..)) {
                    continue;
                }
                Renderer::fill_rect(&mut pixmap, positions[i], 0, w, height, item.bg);
            }

            // Pass 2: decorations (circles/pills) drawn over backgrounds, under text
            for (i, (item, &w)) in items.iter().zip(item_widths.iter()).enumerate() {
                match &item.shape {
                    ContentShape::CircledText(_, circle_color) => {
                        if w <= height {
                            let r = height as f32 / 2.0;
                            let cx = positions[i] as f32 + r;
                            renderer::draw_filled_circle(&mut pixmap, cx, r, r, *circle_color);
                        } else {
                            renderer::draw_pill(
                                &mut pixmap,
                                positions[i] as f32, 0.0,
                                w as f32, height as f32,
                                *circle_color,
                            );
                        }
                    }
                    ContentShape::RingedText(_, ring_color) => {
                        let thickness = 3.0;
                        if w <= height {
                            let r = height as f32 / 2.0;
                            let cx = positions[i] as f32 + r;
                            renderer::draw_ring(&mut pixmap, cx, r, r, thickness, *ring_color);
                        } else {
                            renderer::draw_pill_ring(
                                &mut pixmap,
                                positions[i] as f32, 0.0,
                                w as f32, height as f32,
                                thickness,
                                *ring_color,
                            );
                        }
                    }
                    _ => {}
                }
            }

            // Pass 3: content (text, powerlines, spinners)
            const SHADOW: crate::types::RGBA = (0, 0, 0, 100);
            for (i, item) in items.iter().enumerate() {
                match &item.shape {
                    ContentShape::Text(text) => {
                        self.renderer.draw_text_outlined(
                            &mut pixmap,
                            text,
                            item.fg,
                            SHADOW,
                            positions[i],
                            height,
                        );
                    }
                    ContentShape::CircledText(text, _)
                    | ContentShape::RingedText(text, _) => {
                        let text_w = self.renderer.text_width(text);
                        let x_offset = height.saturating_sub(text_w) / 2;
                        self.renderer.draw_text_outlined(
                            &mut pixmap,
                            text,
                            item.fg,
                            SHADOW,
                            positions[i] + x_offset,
                            height,
                        );
                    }
                    ContentShape::Powerline(style, fill, direction) => {
                        if *style == PowerlineStyle::Fade && *fill == PowerlineFill::Full {
                            let (left_c, right_c) = match direction {
                                PowerlineDirection::Left => (item.bg, item.fg),
                                PowerlineDirection::Right => (item.fg, item.bg),
                            };
                            let solid = if left_c.3 >= right_c.3 { left_c } else { right_c };
                            let left = if left_c.3 >= right_c.3 {
                                solid
                            } else {
                                (solid.0, solid.1, solid.2, 0)
                            };
                            let right = if right_c.3 >= left_c.3 {
                                solid
                            } else {
                                (solid.0, solid.1, solid.2, 0)
                            };
                            renderer::fill_gradient_rect(
                                &mut pixmap,
                                positions[i], 0,
                                item_widths[i], height,
                                left, right,
                            );
                        } else if *style == PowerlineStyle::Fade {
                            // Sparse fade: use powerline outline shape
                            let polys = renderer::shape_polys(
                                height, positions[i],
                                PowerlineStyle::Powerline, *direction, *fill,
                            );
                            Renderer::fill_polys(&mut pixmap, &polys, item.fg);
                        } else {
                            let polys =
                                renderer::shape_polys(height, positions[i], *style, *direction, *fill);
                            Renderer::fill_polys(&mut pixmap, &polys, item.fg);
                        }
                    }
                    ContentShape::Spinner(angle) => {
                        let polys = renderer::shape_spinner(height, positions[i], *angle);
                        Renderer::fill_polys(&mut pixmap, &polys, item.fg);
                    }
                    ContentShape::Icon(icon) => {
                        Renderer::draw_icon(&mut pixmap, icon, positions[i], height);
                    }
                    ContentShape::HSpace(_) => {}
                }
            }
        }

        self.hit_zones.insert(monitor_index, zones);
        self.hover_zones.insert(monitor_index, hover_zones_vec);

        // Debug: save one frame.
        static SAVED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !SAVED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let _ = std::fs::write("/tmp/schrottbar_frame.bin", pixmap.data_mut() as &[u8]);
            log::debug!("Saved BGRA frame {width}x{height}");
        }

        // Attach buffer and commit.
        let surface = &self.wayland.surfaces[monitor_index];
        surface
            .layer
            .wl_surface()
            .damage_buffer(0, 0, width as i32, height as i32);
        buffer
            .attach_to(surface.layer.wl_surface())
            .expect("Failed to attach buffer");
        surface.layer.commit();
    }

    pub fn flush(&self) {
        self.conn
            .flush()
            .expect("Failed to flush Wayland connection");
    }

    pub fn dispatch_pending(&mut self) -> Vec<BarEvent> {
        // Dispatch any pending events without blocking.
        self.event_queue
            .dispatch_pending(&mut self.wayland)
            .expect("Failed to dispatch Wayland events");

        std::mem::take(&mut self.wayland.pending_events)
    }

    pub async fn next_event(&mut self) -> Vec<BarEvent> {
        loop {
            // First try to dispatch pending events.
            let events = self.dispatch_pending();
            if !events.is_empty() {
                return events;
            }

            // Prepare to read and wait for the fd to be readable.
            let read_guard = self.conn.prepare_read().expect("Failed to prepare read");
            {
                let fd = read_guard.connection_fd();
                let async_fd = AsyncFd::new(fd).expect("Failed to create AsyncFd");
                let _ = async_fd
                    .readable()
                    .await
                    .expect("Failed to wait for Wayland events");
                // async_fd is dropped here, releasing the borrow on read_guard.
            }
            // Read events from the socket.
            let _ = read_guard.read();
        }
    }

    pub fn handle_click(&self, surface_index: usize, x: f64, button: u32) {
        if let Some(zones) = self.hit_zones.get(&surface_index) {
            let x = x as u32;
            for zone in zones {
                if x >= zone.x_start && x < zone.x_end {
                    debug!("Click hit zone [{}, {}) button={button}", zone.x_start, zone.x_end);
                    (zone.handler)(button);
                    return;
                }
            }
        }
    }

    pub fn handle_hover(&mut self, surface_index: usize, x: f64) -> bool {
        self.pointer_pos.insert(surface_index, x);
        self.update_hover_flags(surface_index)
    }

    pub fn handle_hover_leave(&mut self, surface_index: usize) -> bool {
        self.pointer_pos.remove(&surface_index);
        self.update_hover_flags(surface_index)
    }

    /// Re-evaluate hover zones after a redraw (zones may have moved/changed).
    /// Returns true if the hovered item changed (caller should re-render).
    pub fn recheck_hover(&mut self) -> bool {
        let surface_indices: Vec<usize> = self.pointer_pos.keys().copied().collect();
        let mut changed = false;
        for surface_index in surface_indices {
            if self.update_hover_flags(surface_index) {
                changed = true;
            }
        }
        changed
    }

    /// Update hover flags for a surface based on current pointer position.
    /// Returns true if the active hover zone changed.
    fn update_hover_flags(&mut self, surface_index: usize) -> bool {
        use std::sync::atomic::Ordering::Relaxed;

        let x = self.pointer_pos.get(&surface_index).copied();
        let new_zone = x.and_then(|x| {
            let x = x as u32;
            self.hover_zones.get(&surface_index).and_then(|zones| {
                zones.iter().enumerate().find_map(|(i, zone)| {
                    (x >= zone.x_start && x < zone.x_end).then_some(i)
                })
            })
        });

        let prev = self.active_hover.get(&surface_index).copied();
        if new_zone == prev {
            return false;
        }

        // Clear previous zone's flag.
        if let Some(prev_idx) = prev
            && let Some(zones) = self.hover_zones.get(&surface_index)
            && let Some(zone) = zones.get(prev_idx)
        {
            zone.flag.store(false, Relaxed);
        }

        // Set new zone's flag.
        if let Some(new_idx) = new_zone {
            if let Some(zones) = self.hover_zones.get(&surface_index)
                && let Some(zone) = zones.get(new_idx)
            {
                zone.flag.store(true, Relaxed);
            }
            self.active_hover.insert(surface_index, new_idx);
        } else {
            self.active_hover.remove(&surface_index);
        }

        true
    }

    fn item_width(&mut self, item: &ContentItem) -> u32 {
        match &item.shape {
            ContentShape::Text(text) => self.renderer.text_width(text),
            ContentShape::CircledText(text, _) | ContentShape::RingedText(text, _) => {
                let text_w = self.renderer.text_width(text);
                let padding = self.height / 4;
                self.height.max(text_w + padding)
            }
            ContentShape::Powerline(style, _, _) => renderer::powerline_width(self.height, *style),
            ContentShape::Spinner(_) => renderer::spinner_size(self.height),
            ContentShape::Icon(icon) => icon.width,
            ContentShape::HSpace(w) => *w,
        }
    }
}
