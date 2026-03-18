use log::debug;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_surface},
    Connection, EventQueue, QueueHandle,
};

#[derive(Debug, Clone)]
pub struct OutputInfo {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub wl_output: wl_output::WlOutput,
}

pub struct SurfaceState {
    pub layer: LayerSurface,
    pub pool: SlotPool,
    pub width: u32,
    pub height: u32,
    pub configured: bool,
}

#[derive(Debug)]
pub enum BarEvent {
    Configure { surface_index: usize },
    Closed { surface_index: usize },
    OutputAdded,
    OutputRemoved,
}

pub struct WaylandState {
    pub registry_state: RegistryState,
    pub output_state: OutputState,
    pub shm: Shm,
    pub compositor: CompositorState,
    pub layer_shell: LayerShell,

    pub outputs: Vec<OutputInfo>,
    pub surfaces: Vec<SurfaceState>,
    pub pending_events: Vec<BarEvent>,
}

impl WaylandState {
    pub fn new(conn: &Connection) -> (Self, EventQueue<Self>) {
        let (globals, mut event_queue) = registry_queue_init(conn).unwrap();
        let qh = event_queue.handle();

        let compositor =
            CompositorState::bind(&globals, &qh).expect("wl_compositor not available");
        let layer_shell = LayerShell::bind(&globals, &qh).expect("wlr-layer-shell not available");
        let shm = Shm::bind(&globals, &qh).expect("wl_shm not available");
        let registry_state = RegistryState::new(&globals);
        let output_state = OutputState::new(&globals, &qh);

        let mut state = Self {
            registry_state,
            output_state,
            shm,
            compositor,
            layer_shell,
            outputs: Vec::new(),
            surfaces: Vec::new(),
            pending_events: Vec::new(),
        };

        // Do a roundtrip to receive initial outputs.
        event_queue.roundtrip(&mut state).unwrap();

        (state, event_queue)
    }

    pub fn create_surfaces(
        &mut self,
        qh: &QueueHandle<Self>,
        bar_height: u32,
    ) {
        // Sort outputs left-to-right, top-to-bottom.
        self.outputs.sort_by(|a, b| {
            if a.x == b.x {
                a.y.cmp(&b.y)
            } else {
                a.x.cmp(&b.x)
            }
        });

        for output_info in &self.outputs {
            let surface = self.compositor.create_surface(qh);
            let layer = self.layer_shell.create_layer_surface(
                qh,
                surface,
                Layer::Top,
                Some("schrottbar"),
                Some(&output_info.wl_output),
            );

            layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
            layer.set_size(0, bar_height);
            layer.set_exclusive_zone(bar_height as i32);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer.commit();

            // Initial pool size: output_width * bar_height * 4 bytes per pixel.
            // Use a reasonable default if we don't know the width yet.
            let initial_width = if output_info.width > 0 {
                output_info.width
            } else {
                1920
            };
            let pool_size = (initial_width * bar_height * 4) as usize;
            let pool = SlotPool::new(pool_size, &self.shm).expect("Failed to create slot pool");

            self.surfaces.push(SurfaceState {
                layer,
                pool,
                width: initial_width,
                height: bar_height,
                configured: false,
            });
        }

        debug!("Created {} layer surfaces", self.surfaces.len());
    }

    fn find_surface_index(&self, wl_surface: &wl_surface::WlSurface) -> Option<usize> {
        self.surfaces
            .iter()
            .position(|s| s.layer.wl_surface() == wl_surface)
    }
}

// --- Delegate implementations ---

impl CompositorHandler for WaylandState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for WaylandState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(info) = self.output_state.info(&output) {
            debug!("New output: {:?}", info.name);
            self.outputs.push(OutputInfo {
                name: info.name.clone().unwrap_or_default(),
                x: info.location.0,
                y: info.location.1,
                width: info
                    .logical_size
                    .map(|(w, _)| w as u32)
                    .unwrap_or(0),
                height: info
                    .logical_size
                    .map(|(_, h)| h as u32)
                    .unwrap_or(0),
                wl_output: output,
            });
            self.pending_events.push(BarEvent::OutputAdded);
        }
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(info) = self.output_state.info(&output) {
            if let Some(existing) = self.outputs.iter_mut().find(|o| o.wl_output == output) {
                existing.name = info.name.clone().unwrap_or_default();
                existing.x = info.location.0;
                existing.y = info.location.1;
                if let Some((w, h)) = info.logical_size {
                    existing.width = w as u32;
                    existing.height = h as u32;
                }
            }
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.outputs.retain(|o| o.wl_output != output);
        self.pending_events.push(BarEvent::OutputRemoved);
    }
}

impl LayerShellHandler for WaylandState {
    fn closed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
    ) {
        if let Some(idx) = self
            .surfaces
            .iter()
            .position(|s| &s.layer == layer)
        {
            self.pending_events.push(BarEvent::Closed {
                surface_index: idx,
            });
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if let Some(idx) = self
            .surfaces
            .iter()
            .position(|s| &s.layer == layer)
        {
            let surface = &mut self.surfaces[idx];
            let old_height = surface.height;
            if configure.new_size.0 > 0 {
                surface.width = configure.new_size.0;
            }
            if configure.new_size.1 > 0 {
                surface.height = configure.new_size.1;
            }
            if surface.height != old_height {
                log::warn!("Compositor changed surface height from {old_height} to {} (configure: {:?})",
                    surface.height, configure.new_size);
            }
            surface.configured = true;
            self.pending_events.push(BarEvent::Configure {
                surface_index: idx,
            });
        }
    }
}

impl ShmHandler for WaylandState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(WaylandState);
delegate_output!(WaylandState);
delegate_shm!(WaylandState);
delegate_layer!(WaylandState);
delegate_registry!(WaylandState);

impl ProvidesRegistryState for WaylandState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}
