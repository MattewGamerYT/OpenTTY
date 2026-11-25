pub mod input;

use std::ffi::c_void;

use wayland_client::protocol::{
    wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat,
    wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::decoration::zv1::client::{
    zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use self::input::{KeyAction, XkbInput};

pub struct WaylandApp {
    pub connection: Connection,
    pub event_queue: EventQueue<WaylandState>,
    pub qh: QueueHandle<WaylandState>,
    pub state: WaylandState,
}

pub struct WaylandState {
    pub compositor: Option<wl_compositor::WlCompositor>,
    pub wm_base: Option<xdg_wm_base::XdgWmBase>,
    pub seat: Option<wl_seat::WlSeat>,
    pub decoration_manager: Option<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1>,
    pub viewporter: Option<wp_viewporter::WpViewporter>,
    pub fractional_scale_manager:
        Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,

    pub surface: Option<wl_surface::WlSurface>,
    pub xdg_surface: Option<xdg_surface::XdgSurface>,
    pub xdg_toplevel: Option<xdg_toplevel::XdgToplevel>,
    pub decoration: Option<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1>,
    pub viewport: Option<wp_viewport::WpViewport>,
    pub fractional_scale: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    pub keyboard: Option<wl_keyboard::WlKeyboard>,
    pub pointer: Option<wl_pointer::WlPointer>,

    pub configured: bool,
    pub should_close: bool,
    pub frame_pending: bool,
    pub needs_redraw: bool,

    pub logical_width: u32,
    pub logical_height: u32,
    pending_configure_width: Option<u32>,
    pending_configure_height: Option<u32>,
    pub pending_configure_serial: Option<u32>,

    /// Scale factor stored in 120ths (e.g. 120 = 1.0x, 150 = 1.25x, 180 = 1.5x).
    pub scale_120: u32,
    output_integer_scale: i32,
    pub scale_changed: bool,
    pub size_changed: bool,

    pub xkb: XkbInput,
    pub app_cursor_keys: bool,
    pub pending_key_actions: Vec<KeyAction>,
    scroll_accum: f64,
}

impl WaylandState {
    fn new(initial_w: u32, initial_h: u32) -> Self {
        Self {
            compositor: None,
            wm_base: None,
            seat: None,
            decoration_manager: None,
            viewporter: None,
            fractional_scale_manager: None,
            surface: None,
            xdg_surface: None,
            xdg_toplevel: None,
            decoration: None,
            viewport: None,
            fractional_scale: None,
            keyboard: None,
            pointer: None,
            configured: false,
            should_close: false,
            frame_pending: false,
            needs_redraw: true,
            logical_width: initial_w.max(1),
            logical_height: initial_h.max(1),
            pending_configure_width: None,
            pending_configure_height: None,
            pending_configure_serial: None,
            scale_120: 120,
            output_integer_scale: 1,
            scale_changed: false,
            size_changed: false,
            xkb: XkbInput::new(),
            app_cursor_keys: false,
            pending_key_actions: Vec::new(),
            scroll_accum: 0.0,
        }
    }

    pub fn scale_factor(&self) -> f32 {
        (self.scale_120 as f32 / 120.0).max(0.5)
    }

    pub fn physical_size(&self) -> (u32, u32) {
        let scale = self.scale_120 as f64 / 120.0;
        let pw = ((self.logical_width as f64) * scale).round().max(1.0) as u32;
        let ph = ((self.logical_height as f64) * scale).round().max(1.0) as u32;
        (pw, ph)
    }
}

impl WaylandApp {
    pub fn connect(
        app_id: &str,
        title: &str,
        initial_logical_w: u32,
        initial_logical_h: u32,
    ) -> Result<Self, String> {
        let connection = Connection::connect_to_env()
            .map_err(|e| format!("failed to connect to Wayland display: {e}"))?;
        let display = connection.display();
        let mut event_queue = connection.new_event_queue();
        let qh = event_queue.handle();

        let mut state = WaylandState::new(initial_logical_w, initial_logical_h);
        display.get_registry(&qh, ());
        event_queue
            .roundtrip(&mut state)
            .map_err(|e| format!("Wayland registry roundtrip failed: {e}"))?;

        let compositor = state
            .compositor
            .clone()
            .ok_or_else(|| "Wayland compositor missing wl_compositor".to_string())?;
        let wm_base = state
            .wm_base
            .clone()
            .ok_or_else(|| "Wayland compositor missing xdg_wm_base".to_string())?;

        let surface = compositor.create_surface(&qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
        let xdg_toplevel = xdg_surface.get_toplevel(&qh, ());
        xdg_toplevel.set_app_id(app_id.to_string());
        xdg_toplevel.set_title(title.to_string());

        // Request KWin server-side decorations (Breeze titlebar & borders).
        if let Some(dec_mgr) = &state.decoration_manager {
            let decoration = dec_mgr.get_toplevel_decoration(&xdg_toplevel, &qh, ());
            decoration.set_mode(zxdg_toplevel_decoration_v1::Mode::ServerSide);
            state.decoration = Some(decoration);
        }

        // Enable viewporter for fractional physical-pixel swapchains over logical surfaces.
        if let Some(vp_mgr) = &state.viewporter {
            let viewport = vp_mgr.get_viewport(&surface, &qh, ());
            viewport.set_destination(initial_logical_w as i32, initial_logical_h as i32);
            state.viewport = Some(viewport);
        }

        // Subscribe to compositor fractional scaling events (e.g. 125% / 150% in KDE Plasma).
        if let Some(fs_mgr) = &state.fractional_scale_manager {
            let fs = fs_mgr.get_fractional_scale(&surface, &qh, ());
            state.fractional_scale = Some(fs);
        }

        surface.commit();

        state.surface = Some(surface);
        state.xdg_surface = Some(xdg_surface);
        state.xdg_toplevel = Some(xdg_toplevel);

        // Wait for initial xdg_surface::configure before creating/presenting swapchain images.
        while !state.configured && !state.should_close {
            event_queue
                .blocking_dispatch(&mut state)
                .map_err(|e| format!("Wayland dispatch failed during initial configure: {e}"))?;
        }

        Ok(Self {
            connection,
            event_queue,
            qh,
            state,
        })
    }

    pub fn raw_handles(&self) -> (*mut c_void, *mut c_void) {
        let display_ptr = self.connection.backend().display_ptr() as *mut c_void;
        let surface_ptr = self
            .state
            .surface
            .as_ref()
            .expect("wl_surface initialized")
            .id()
            .as_ptr() as *mut c_void;
        (display_ptr, surface_ptr)
    }

    pub fn set_title(&self, title: &str) {
        if let Some(toplevel) = &self.state.xdg_toplevel {
            toplevel.set_title(title.to_string());
        }
    }

    /// Arms the surface for atomic commit when Vulkan vkQueuePresentKHR runs.
    pub fn prepare_frame_commit(&mut self) {
        if let Some(serial) = self.state.pending_configure_serial.take()
            && let Some(xdg_surface) = &self.state.xdg_surface
        {
            xdg_surface.ack_configure(serial);
        }
        if let Some(viewport) = &self.state.viewport {
            viewport.set_destination(
                self.state.logical_width as i32,
                self.state.logical_height as i32,
            );
        }
        if let Some(surface) = &self.state.surface {
            surface.frame(&self.qh, ());
            self.state.frame_pending = true;
        }
        let _ = self.connection.flush();
    }
}

delegate_noop!(WaylandState: ignore wl_compositor::WlCompositor);
delegate_noop!(WaylandState: ignore wl_surface::WlSurface);
delegate_noop!(WaylandState: ignore zxdg_decoration_manager_v1::ZxdgDecorationManagerV1);
delegate_noop!(WaylandState: ignore wp_viewporter::WpViewporter);
delegate_noop!(WaylandState: ignore wp_viewport::WpViewport);
delegate_noop!(WaylandState: ignore wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);

impl Dispatch<wl_registry::WlRegistry, ()> for WaylandState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor =
                        Some(registry.bind::<wl_compositor::WlCompositor, _, _>(
                            name,
                            version.min(6),
                            qh,
                            (),
                        ));
                }
                "xdg_wm_base" => {
                    state.wm_base = Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(
                        name,
                        version.min(6),
                        qh,
                        (),
                    ));
                }
                "wl_seat" => {
                    state.seat = Some(registry.bind::<wl_seat::WlSeat, _, _>(
                        name,
                        version.min(8),
                        qh,
                        (),
                    ));
                }
                "wl_output" => {
                    registry.bind::<wl_output::WlOutput, _, _>(name, version.min(4), qh, ());
                }
                "zxdg_decoration_manager_v1" => {
                    state.decoration_manager = Some(
                        registry
                            .bind::<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1, _, _>(
                                name,
                                1,
                                qh,
                                (),
                            ),
                    );
                }
                "wp_viewporter" => {
                    state.viewporter =
                        Some(registry.bind::<wp_viewporter::WpViewporter, _, _>(name, 1, qh, ()));
                }
                "wp_fractional_scale_manager_v1" => {
                    state.fractional_scale_manager = Some(
                        registry
                            .bind::<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, _, _>(
                                name,
                                1,
                                qh,
                                (),
                            ),
                    );
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for WaylandState {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for WaylandState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            let new_w = state
                .pending_configure_width
                .take()
                .unwrap_or(state.logical_width);
            let new_h = state
                .pending_configure_height
                .take()
                .unwrap_or(state.logical_height);

            if !state.configured {
                xdg_surface.ack_configure(serial);
                state.configured = true;
            } else {
                // Defer ack_configure until the Vulkan frame is committed for atomic resize.
                state.pending_configure_serial = Some(serial);
            }

            if new_w != state.logical_width || new_h != state.logical_height {
                state.logical_width = new_w.max(1);
                state.logical_height = new_h.max(1);
                state.size_changed = true;
            }
            state.needs_redraw = true;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                if width > 0 {
                    state.pending_configure_width = Some(width as u32);
                }
                if height > 0 {
                    state.pending_configure_height = Some(height as u32);
                }
            }
            xdg_toplevel::Event::Close => {
                state.should_close = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1,
        _: zxdg_toplevel_decoration_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event
            && scale > 0
            && state.scale_120 != scale
        {
            state.scale_120 = scale;
            state.scale_changed = true;
            state.size_changed = true;
            state.needs_redraw = true;
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Scale { factor } = event
            && factor > 0
        {
            state.output_integer_scale = factor;
            if state.fractional_scale.is_none() {
                let scale_120 = (factor as u32) * 120;
                if state.scale_120 != scale_120 {
                    state.scale_120 = scale_120;
                    state.scale_changed = true;
                    state.size_changed = true;
                    state.needs_redraw = true;
                }
            }
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.frame_pending = false;
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for WaylandState {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            if caps.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if caps.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap {
                format: WEnum::Value(wl_keyboard::KeymapFormat::XkbV1),
                fd,
                size,
            } => {
                state.xkb.load_keymap_from_fd(fd, size);
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                state
                    .xkb
                    .update_modifiers(mods_depressed, mods_latched, mods_locked, group);
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => {
                state.xkb.set_repeat_info(rate, delay);
            }
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(key_state),
                ..
            } => match key_state {
                wl_keyboard::KeyState::Pressed => {
                    if let Some(action) = state.xkb.on_key_pressed(key, state.app_cursor_keys) {
                        state.pending_key_actions.push(action);
                    }
                }
                wl_keyboard::KeyState::Released => {
                    state.xkb.on_key_released(key);
                }
                _ => {}
            },
            wl_keyboard::Event::Leave { .. } => {
                state.xkb.clear_repeat();
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Axis {
            axis: WEnum::Value(wl_pointer::Axis::VerticalScroll),
            value,
            ..
        } = event
        {
            state.scroll_accum += value;
            let step = 12.0;
            while state.scroll_accum <= -step {
                state.scroll_accum += step;
                state.pending_key_actions.push(KeyAction::ScrollLines(3));
            }
            while state.scroll_accum >= step {
                state.scroll_accum -= step;
                state.pending_key_actions.push(KeyAction::ScrollLines(-3));
            }
        }
    }
}
