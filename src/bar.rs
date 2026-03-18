use log::{debug, info};
use tiny_skia::PixmapMut;
use tokio::io::unix::AsyncFd;
use wayland_client::{
    protocol::wl_shm,
    Connection, EventQueue,
};

use crate::renderer::{self, Renderer};
use crate::types::{Alignment, ContentItem, ContentShape};
use crate::wayland::{BarEvent, OutputInfo, WaylandState};

use smithay_client_toolkit::shell::WaylandSurface;

pub struct Bar {
    height: u32,
    renderer: Renderer,
    conn: Connection,
    event_queue: EventQueue<WaylandState>,
    wayland: WaylandState,
}

impl Bar {
    pub fn new() -> Self {
        let conn = Connection::connect_to_env().expect("Failed to connect to Wayland");
        let (mut wayland, mut event_queue) = WaylandState::new(&conn);

        let font_family = "UbuntuMono Nerd Font Propo";
        let font_size = 19.0;
        let renderer = Renderer::new(font_family, font_size);
        let height = renderer.height();
        info!("Bar height: {height} (ascent={} descent={} padding={})",
            renderer.ascent(), renderer.descent(), Renderer::PADDING);

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
        }
    }

    pub fn outputs(&self) -> &[OutputInfo] {
        &self.wayland.outputs
    }

    pub fn height(&self) -> u32 {
        self.height
    }

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
        assert_eq!(surface.height, height, "surface height({}) != bar height({height})", surface.height);

        let expected_canvas_size = (width * height * 4) as usize;
        let (buffer, canvas) = surface
            .pool
            .create_buffer(width as i32, height as i32, stride, wl_shm::Format::Argb8888)
            .expect("Failed to create buffer");

        assert_eq!(canvas.len(), expected_canvas_size,
            "canvas size({}) != expected({})", canvas.len(), expected_canvas_size);

        // Clear to transparent (BGRA format for wl_shm Argb8888).
        canvas.fill(0x00);

        // Wrap canvas as a tiny-skia PixmapMut (RGBA byte order).
        // tiny-skia interprets our BGRA as "RGBA" — so we pass swapped colors.
        let mut pixmap =
            PixmapMut::from_bytes(canvas, width, height).expect("Failed to create pixmap");

        assert_eq!(pixmap.width(), width, "pixmap width mismatch");
        assert_eq!(pixmap.height(), height, "pixmap height mismatch");

        // Render each alignment section.
        for (alignment, items, item_widths) in &sections {
            let total_width: u32 = item_widths.iter().sum();

            let mut cursor = match alignment {
                Alignment::Left => 0u32,
                Alignment::Center => width.saturating_sub(total_width) / 2,
                Alignment::Right => width.saturating_sub(total_width),
            };

            for (item, &w) in items.iter().zip(item_widths.iter()) {
                Renderer::fill_rect(&mut pixmap, cursor, 0, w, height, item.bg);

                match &item.shape {
                    ContentShape::Text(text) => {
                        self.renderer
                            .draw_text(&mut pixmap, text, item.fg, cursor, height);
                    }
                    ContentShape::Powerline(style, fill, direction) => {
                        let polys =
                            renderer::shape_polys(height, cursor, *style, *direction, *fill);
                        Renderer::fill_polys(&mut pixmap, &polys, item.fg);
                    }
                    ContentShape::Spinner(angle) => {
                        let polys = renderer::shape_spinner(height, cursor, *angle);
                        Renderer::fill_polys(&mut pixmap, &polys, item.fg);
                    }
                }

                cursor += w;
            }
        }

        // Debug: save one frame.
        static SAVED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !SAVED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let _ = std::fs::write("/tmp/schrottbar_frame.bin", pixmap.data_mut() as &[u8]);
            log::debug!("Saved BGRA frame {width}x{height}");
        }
        drop(pixmap);

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

    pub fn present(&self) {
        // In the Wayland model, present is done per-surface in draw() via commit().
        // This method exists for API compatibility — just flush.
        self.flush();
    }

    pub fn flush(&self) {
        self.conn.flush().expect("Failed to flush Wayland connection");
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

    fn item_width(&mut self, item: &ContentItem) -> u32 {
        match &item.shape {
            ContentShape::Text(text) => self.renderer.text_width(text),
            ContentShape::Powerline(style, _, _) => renderer::powerline_width(self.height, *style),
            ContentShape::Spinner(_) => renderer::spinner_size(self.height),
        }
    }
}
