pub mod clipboard;
pub mod input;

use std::collections::HashMap;
use std::ffi::c_void;
use std::io::Write;
use std::os::fd::AsFd;

use wayland_client::backend::ObjectId;
use wayland_client::protocol::{
    wl_callback, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
    wl_data_source, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_surface,
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

#[derive(Debug, Clone, Copy)]
pub enum PointerAction {
    Button {
        button: u32,
        pressed: bool,
        x: f64,
        y: f64,
    },
    Motion {
        x: f64,
        y: f64,
    },
    Scroll {
        steps: isize,
        x: f64,
        y: f64,
    },
}

#[derive(Debug, Clone)]
pub enum DropPayload {
    Paths(String),
    Text(Vec<u8>),
}

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
    pub data_device_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    pub data_device: Option<wl_data_device::WlDataDevice>,
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

    pub data_source: Option<wl_data_source::WlDataSource>,
    pub owned_clipboard_text: String,
    pub clipboard_offer: Option<wl_data_offer::WlDataOffer>,
    pub dnd_offer: Option<wl_data_offer::WlDataOffer>,
    pub dnd_serial: u32,
    pub offer_mimes: HashMap<ObjectId, Vec<String>>,
    pub pending_drops: Vec<(wl_data_offer::WlDataOffer, Vec<String>)>,

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
    pub last_input_serial: u32,
    pub pointer_x: f64,
    pub pointer_y: f64,
    pub pending_key_actions: Vec<KeyAction>,
    pub pending_pointer_actions: Vec<PointerAction>,
    pub pending_focus_event: Option<bool>,
    scroll_accum: f64,
}

impl WaylandState {
    fn new(initial_w: u32, initial_h: u32) -> Self {
        Self {
            compositor: None,
            wm_base: None,
            seat: None,
            data_device_manager: None,
            data_device: None,
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
            data_source: None,
            owned_clipboard_text: String::new(),
            clipboard_offer: None,
            dnd_offer: None,
            dnd_serial: 0,
            offer_mimes: HashMap::new(),
            pending_drops: Vec::new(),
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
            last_input_serial: 0,
            pointer_x: 0.0,
            pointer_y: 0.0,
            pending_key_actions: Vec::new(),
            pending_pointer_actions: Vec::new(),
            pending_focus_event: None,
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

    fn ensure_data_device(&mut self, qh: &QueueHandle<Self>) {
        if self.data_device.is_none()
            && let (Some(mgr), Some(seat)) = (&self.data_device_manager, &self.seat)
        {
            self.data_device = Some(mgr.get_data_device(seat, qh, ()));
        }
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

    pub fn set_clipboard(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let (Some(mgr), Some(device)) = (
            self.state.data_device_manager.as_ref(),
            self.state.data_device.as_ref(),
        ) else {
            return;
        };

        if let Some(old_source) = self.state.data_source.take() {
            old_source.destroy();
        }

        let source = mgr.create_data_source(&self.qh, ());
        for mime in [
            "text/plain;charset=utf-8",
            "text/plain",
            "UTF8_STRING",
            "STRING",
            "TEXT",
        ] {
            source.offer(mime.to_string());
        }
        device.set_selection(Some(&source), self.state.last_input_serial);
        self.state.owned_clipboard_text = text.to_string();
        self.state.data_source = Some(source);
        let _ = self.connection.flush();
    }

    pub fn read_clipboard_text(&mut self) -> Option<String> {
        if self.state.data_source.is_some() && !self.state.owned_clipboard_text.is_empty() {
            return Some(self.state.owned_clipboard_text.clone());
        }

        let offer = self.state.clipboard_offer.clone()?;
        let mimes = self.state.offer_mimes.get(&offer.id())?.clone();
        let mime = clipboard::pick_text_mime(&mimes)?;

        let Ok((read_fd, write_fd)) = nix::unistd::pipe() else {
            return None;
        };
        offer.receive(mime, write_fd.as_fd());
        drop(write_fd);
        let _ = self.connection.flush();

        let bytes = clipboard::read_fd_with_timeout(read_fd);
        if bytes.is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(&bytes).into_owned())
        }
    }

    pub fn take_pending_drops(&mut self, shell_quote_paths: bool) -> Vec<DropPayload> {
        let drops: Vec<_> = self.state.pending_drops.drain(..).collect();
        let mut payloads = Vec::new();

        for (offer, mimes) in drops {
            if let Some(mime) = clipboard::pick_dnd_mime(&mimes)
                && let Ok((read_fd, write_fd)) = nix::unistd::pipe()
            {
                offer.receive(mime.clone(), write_fd.as_fd());
                drop(write_fd);
                let _ = self.connection.flush();

                let bytes = clipboard::read_fd_with_timeout(read_fd);
                if !bytes.is_empty() {
                    if mime.eq_ignore_ascii_case("text/uri-list")
                        || mime.eq_ignore_ascii_case("application/x-kde-urilist")
                    {
                        let paths =
                            clipboard::parse_uri_list_for_terminal(&bytes, shell_quote_paths);
                        if !paths.is_empty() {
                            payloads.push(DropPayload::Paths(paths));
                        }
                    } else {
                        payloads.push(DropPayload::Text(bytes));
                    }
                }
            }

            if offer.version() >= 3 {
                offer.finish();
            }
            offer.destroy();
        }

        if !payloads.is_empty() {
            let _ = self.connection.flush();
        }
        payloads
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
delegate_noop!(WaylandState: ignore wl_data_device_manager::WlDataDeviceManager);
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
                    state.ensure_data_device(qh);
                }
                "wl_data_device_manager" => {
                    state.data_device_manager = Some(
                        registry.bind::<wl_data_device_manager::WlDataDeviceManager, _, _>(
                            name,
                            version.min(3),
                            qh,
                            (),
                        ),
                    );
                    state.ensure_data_device(qh);
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

impl Dispatch<wl_data_device::WlDataDevice, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                state.offer_mimes.entry(id.id()).or_default();
            }
            wl_data_device::Event::Enter { serial, id, .. } => {
                if let Some(prev) = state.dnd_offer.take() {
                    state.offer_mimes.remove(&prev.id());
                    prev.destroy();
                }
                state.dnd_serial = serial;
                if let Some(offer) = id {
                    let picked = state
                        .offer_mimes
                        .get(&offer.id())
                        .and_then(|mimes| clipboard::pick_dnd_mime(mimes));
                    offer.accept(serial, picked);
                    if offer.version() >= 3 {
                        let supported = wl_data_device_manager::DndAction::Copy
                            | wl_data_device_manager::DndAction::Move
                            | wl_data_device_manager::DndAction::Ask;
                        offer.set_actions(supported, wl_data_device_manager::DndAction::Copy);
                    }
                    state.dnd_offer = Some(offer);
                }
            }
            wl_data_device::Event::Motion { .. } => {
                if let Some(offer) = &state.dnd_offer {
                    let picked = state
                        .offer_mimes
                        .get(&offer.id())
                        .and_then(|mimes| clipboard::pick_dnd_mime(mimes));
                    offer.accept(state.dnd_serial, picked);
                }
            }
            wl_data_device::Event::Leave => {
                if let Some(offer) = state.dnd_offer.take() {
                    state.offer_mimes.remove(&offer.id());
                    offer.destroy();
                }
            }
            wl_data_device::Event::Drop => {
                if let Some(offer) = state.dnd_offer.take() {
                    let mimes = state.offer_mimes.remove(&offer.id()).unwrap_or_default();
                    state.pending_drops.push((offer, mimes));
                }
            }
            wl_data_device::Event::Selection { id } => {
                let new_id = id.as_ref().map(|o| o.id());
                if let Some(prev) = state.clipboard_offer.take()
                    && Some(prev.id()) != new_id
                {
                    state.offer_mimes.remove(&prev.id());
                    prev.destroy();
                }
                if id.is_some() && state.data_source.is_some() {
                    // Selection may now belong to another client unless it matches our source.
                    // wl_data_source::Cancelled will clear our source when overwritten externally.
                }
                state.clipboard_offer = id;
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(WaylandState, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for WaylandState {
    fn event(
        state: &mut Self,
        offer: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_offer::Event::Offer { mime_type } => {
                state
                    .offer_mimes
                    .entry(offer.id())
                    .or_default()
                    .push(mime_type);
            }
            wl_data_offer::Event::SourceActions {
                source_actions: WEnum::Value(source_actions),
            } => {
                if offer.version() >= 3
                    && state
                        .dnd_offer
                        .as_ref()
                        .is_some_and(|cur| cur.id() == offer.id())
                {
                    let supported = wl_data_device_manager::DndAction::Copy
                        | wl_data_device_manager::DndAction::Move
                        | wl_data_device_manager::DndAction::Ask;
                    let acceptable = source_actions & supported;
                    let chosen = if acceptable.is_empty() {
                        wl_data_device_manager::DndAction::Copy
                    } else {
                        acceptable
                    };
                    let preferred = if chosen.contains(wl_data_device_manager::DndAction::Copy) {
                        wl_data_device_manager::DndAction::Copy
                    } else if chosen.contains(wl_data_device_manager::DndAction::Move) {
                        wl_data_device_manager::DndAction::Move
                    } else {
                        wl_data_device_manager::DndAction::Ask
                    };
                    offer.set_actions(chosen, preferred);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for WaylandState {
    fn event(
        state: &mut Self,
        source: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { fd, .. } => {
                let mut file = std::fs::File::from(fd);
                let _ = file.write_all(state.owned_clipboard_text.as_bytes());
            }
            wl_data_source::Event::Cancelled => {
                if state
                    .data_source
                    .as_ref()
                    .is_some_and(|cur| cur.id() == source.id())
                {
                    state.data_source = None;
                }
                source.destroy();
            }
            _ => {}
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
            wl_keyboard::Event::Enter { serial, .. } => {
                state.last_input_serial = serial;
                state.pending_focus_event = Some(true);
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
                serial,
                key,
                state: WEnum::Value(key_state),
                ..
            } => {
                state.last_input_serial = serial;
                match key_state {
                    wl_keyboard::KeyState::Pressed => {
                        if let Some(action) = state.xkb.on_key_pressed(key, state.app_cursor_keys) {
                            state.pending_key_actions.push(action);
                        }
                    }
                    wl_keyboard::KeyState::Released => {
                        state.xkb.on_key_released(key);
                    }
                    _ => {}
                }
            }
            wl_keyboard::Event::Leave { serial, .. } => {
                state.last_input_serial = serial;
                state.xkb.clear_repeat();
                state.pending_focus_event = Some(false);
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
        match event {
            wl_pointer::Event::Enter {
                serial,
                surface_x,
                surface_y,
                ..
            } => {
                state.last_input_serial = serial;
                state.pointer_x = surface_x;
                state.pointer_y = surface_y;
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_x = surface_x;
                state.pointer_y = surface_y;
                state.pending_pointer_actions.push(PointerAction::Motion {
                    x: surface_x,
                    y: surface_y,
                });
            }
            wl_pointer::Event::Button {
                serial,
                button,
                state: WEnum::Value(btn_state),
                ..
            } => {
                state.last_input_serial = serial;
                state.pending_pointer_actions.push(PointerAction::Button {
                    button,
                    pressed: btn_state == wl_pointer::ButtonState::Pressed,
                    x: state.pointer_x,
                    y: state.pointer_y,
                });
            }
            wl_pointer::Event::Leave { .. } => {
                state.scroll_accum = 0.0;
            }
            wl_pointer::Event::Axis {
                axis: WEnum::Value(wl_pointer::Axis::VerticalScroll),
                value,
                ..
            } => {
                state.scroll_accum += value;
                let step = 12.0;
                while state.scroll_accum <= -step {
                    state.scroll_accum += step;
                    state.pending_pointer_actions.push(PointerAction::Scroll {
                        steps: 1,
                        x: state.pointer_x,
                        y: state.pointer_y,
                    });
                }
                while state.scroll_accum >= step {
                    state.scroll_accum -= step;
                    state.pending_pointer_actions.push(PointerAction::Scroll {
                        steps: -1,
                        x: state.pointer_x,
                        y: state.pointer_y,
                    });
                }
            }
            _ => {}
        }
    }
}
