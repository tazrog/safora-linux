//! Compositor state: windows, workspaces, focus and tiling.

use std::{ffi::OsString, process::Command, sync::Arc, time::Instant};

use smithay::{
    backend::{
        renderer::{
            element::solid::SolidColorBuffer,
            gles::{element::PixelShaderElement, GlesRenderer},
        },
        session::libseat::LibSeatSession,
    },
    desktop::{layer_map_for_output, PopupManager, Space, Window, WindowSurfaceType},
    input::{pointer::CursorImageStatus, Seat, SeatState},
    output::Output,
    reexports::{
        calloop::{generic::Generic, Interest, LoopHandle, LoopSignal, Mode, PostAction},
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
            Display, DisplayHandle,
        },
    },
    utils::{Logical, Point, Rectangle, Serial, SERIAL_COUNTER},
    wayland::{
        compositor::{with_states, CompositorClientState, CompositorState},
        dmabuf::DmabufState,
        output::OutputManagerState,
        selection::{data_device::DataDeviceState, primary_selection::PrimarySelectionState},
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{decoration::XdgDecorationState, XdgShellState, XdgToplevelSurfaceData},
        },
        shm::ShmState,
        socket::ListeningSocketSource,
        viewporter::ViewporterState,
    },
};
use safora_common::{
    config::{parse_color, Config, TileMode},
    ipc::{State as IpcState, WindowInfo, WorkspaceInfo},
};

use crate::{
    input::{default_bindings, Action, Binding},
    layout::{self, Node, Rect},
    render::{Cursor, Shaders, Wallpaper},
};

/// A managed toplevel window.
pub struct Client {
    pub id: u32,
    pub window: Window,
    pub workspace: usize,
    pub floating: bool,
    pub fullscreen: bool,
    pub float_rect: Option<Rect>,
    /// The monitor area a fullscreen window covers (chosen when it went fullscreen).
    pub fs_rect: Option<Rect>,
    /// Where the tiling layout last put this window (used to spot windows too big for their tile).
    /// The size last requested from the client (configure), if tiled or floating.
    pub asked_size: Option<(i32, i32)>,
    /// top, bottom, left, right (used when corners are square)
    pub borders: [SolidColorBuffer; 4],
    /// Rounded border (area, color, radius, width) as last drawn.
    pub ring: Option<(PixelShaderElement, (Rectangle<i32, Logical>, [f32; 4], i32, i32))>,
}

impl Client {
    pub fn surface(&self) -> WlSurface {
        self.window.toplevel().expect("wayland window").wl_surface().clone()
    }
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

#[allow(dead_code)] // some protocol states only need to stay alive
pub struct SaforaState {
    pub start_time: Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, SaforaState>,
    pub loop_signal: LoopSignal,

    pub space: Space<Window>,
    pub popups: PopupManager,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: XdgDecorationState,
    pub layer_shell_state: WlrLayerShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<SaforaState>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub viewporter_state: ViewporterState,
    pub dmabuf_state: DmabufState,

    pub seat: Seat<Self>,
    /// The primary monitor: windows tile here and the bar's workspace switcher belongs to it.
    pub output: Option<Output>,
    /// Every enabled monitor, left to right; `outputs[0]` is the primary.
    pub outputs: Vec<Output>,

    pub config: Config,
    pub bindings: Vec<Binding>,
    pub clients: Vec<Client>,
    pub active_ws: usize,
    pub last_focus: Vec<Option<u32>>,
    pub focused: Option<u32>,
    /// A layer-shell surface (launcher, lock screen…) that currently owns the keyboard.
    pub layer_focus: Option<WlSurface>,
    pub next_id: u32,

    pub pointer_location: Point<f64, Logical>,
    pub cursor_status: CursorImageStatus,
    pub cursor: Cursor,

    pub ipc_subscribers: Vec<std::os::unix::net::UnixStream>,
    pub ipc_last: Option<IpcState>,

    /// Set by key bindings; the DRM backend performs the actual VT switch.
    pub vt_switch: Option<i32>,
    pub screenshot_request: Option<String>,
    pub suppressed_keys: Vec<smithay::input::keyboard::Keycode>,

    /// Renderer for the native backend (kept here so dmabuf imports can reach it).
    pub gpu: Option<GlesRenderer>,
    pub shaders: Option<Shaders>,
    /// Scaled wallpaper per monitor (by output name).
    pub wallpaper: std::collections::HashMap<String, Wallpaper>,
    /// Smart-split trees, one per workspace.
    pub trees: Vec<Option<Node>>,
    /// True when the framebuffer's Y axis is flipped relative to screen coordinates (nested backend).
    pub fb_flip: bool,
    /// Modes of the active monitor (filled by the DRM backend).
    pub output_modes: Vec<safora_common::ipc::ModeInfo>,
    /// Set when the display mode setting may have changed; the DRM backend re-evaluates it.
    pub mode_dirty: bool,
    /// Something visible changed since the last frame (the DRM backend repaints fully when set).
    pub needs_render: bool,
    /// Last keyboard/pointer activity, for the screen-off and suspend timers.
    pub last_input: Instant,
    /// The screen is dark because of the idle timer.
    pub blanked: bool,
    /// The screen is locked: only the lock screen gets input and is drawn.
    pub locked: bool,
    /// Secret handed to the lock screen process; only it can unlock.
    pub lock_token: String,
    pub lock_spawned: Option<Instant>,
    pub input_devices: Vec<smithay::reexports::input::Device>,
    pub session: Option<LibSeatSession>,
}

impl SaforaState {
    pub fn new(
        loop_handle: LoopHandle<'static, Self>,
        loop_signal: LoopSignal,
        display: Display<Self>,
        seat_name: &str,
        config: Config,
    ) -> Self {
        let dh = display.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let dmabuf_state = DmabufState::new();

        let cursor_size = config.accessibility.cursor_size();
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, seat_name);
        let xkb = smithay::input::keyboard::XkbConfig {
            layout: &config.general.keyboard_layout,
            variant: &config.general.keyboard_variant,
            ..Default::default()
        };
        if seat.add_keyboard(xkb, config.input.repeat_delay_ms, config.input.repeat_rate).is_err() {
            tracing::warn!("invalid keyboard layout, falling back to the default");
            seat.add_keyboard(Default::default(), 300, 30).expect("keyboard");
        }
        seat.add_pointer();

        let socket_name = Self::init_wayland_listener(display, &loop_handle);
        let workspaces = config.general.workspaces;
        let bindings = default_bindings(&config);

        Self {
            start_time: Instant::now(),
            socket_name,
            display_handle: dh,
            loop_handle,
            loop_signal,
            space: Space::default(),
            popups: PopupManager::default(),
            compositor_state,
            xdg_shell_state,
            xdg_decoration_state,
            layer_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            primary_selection_state,
            viewporter_state,
            dmabuf_state,
            seat,
            output: None,
            outputs: Vec::new(),
            config,
            bindings,
            clients: Vec::new(),
            active_ws: 0,
            last_focus: vec![None; workspaces],
            focused: None,
            layer_focus: None,
            next_id: 1,
            pointer_location: (0.0, 0.0).into(),
            cursor_status: CursorImageStatus::default_named(),
            cursor: Cursor::load(cursor_size, 1),
            ipc_subscribers: Vec::new(),
            ipc_last: None,
            vt_switch: None,
            screenshot_request: None,
            suppressed_keys: Vec::new(),
            gpu: None,
            shaders: None,
            wallpaper: std::collections::HashMap::new(),
            trees: vec![None; workspaces],
            fb_flip: false,
            output_modes: Vec::new(),
            mode_dirty: true,
            needs_render: true,
            last_input: Instant::now(),
            blanked: false,
            locked: false,
            lock_token: String::new(),
            lock_spawned: None,
            input_devices: Vec::new(),
            session: None,
        }
    }

    fn init_wayland_listener(display: Display<Self>, handle: &LoopHandle<'static, Self>) -> OsString {
        let socket = ListeningSocketSource::new_auto().expect("wayland socket");
        let name = socket.socket_name().to_os_string();

        handle
            .insert_source(socket, |stream, _, state| {
                if let Err(e) = state.display_handle.insert_client(stream, Arc::new(ClientState::default())) {
                    tracing::warn!("failed to accept wayland client: {e}");
                }
            })
            .expect("wayland listener");

        handle
            .insert_source(Generic::new(display, Interest::READ, Mode::Level), |_, display, state| {
                // Safety: the display is never dropped while the loop runs.
                unsafe { display.get_mut().dispatch_clients(state).expect("dispatch") };
                Ok(PostAction::Continue)
            })
            .expect("display source");
        name
    }

    // ---------------------------------------------------------------- outputs

    pub fn add_output(&mut self, output: Output) {
        let first = self.outputs.is_empty();
        self.outputs.push(output.clone());
        self.layout_outputs();
        if first {
            if let Some(geo) = self.space.output_geometry(&output) {
                self.pointer_location = (geo.loc.x as f64 + geo.size.w as f64 / 2.0, geo.loc.y as f64 + geo.size.h as f64 / 2.0).into();
            }
            self.output = Some(output);
        }
        self.relayout();
    }

    pub fn remove_output(&mut self, output: &Output) {
        self.space.unmap_output(output);
        self.outputs.retain(|o| o != output);
        self.output = self.outputs.first().cloned();
        self.layout_outputs();
        self.relayout();
    }

    /// Put the monitors side by side, left to right, with the primary at the origin.
    pub fn layout_outputs(&mut self) {
        let mut x = 0;
        for o in self.outputs.clone() {
            self.space.map_output(&o, (x, 0));
            let w = self.space.output_geometry(&o).map_or(0, |g| g.size.w);
            x += w;
        }
        for c in &mut self.clients {
            c.fs_rect = None;
        }
        self.needs_render = true;
    }

    /// The monitor containing a point, if any.
    pub fn output_at(&self, pos: Point<f64, Logical>) -> Option<Output> {
        self.outputs.iter().find(|o| self.space.output_geometry(o).is_some_and(|g| g.to_f64().contains(pos))).cloned()
    }

    /// Bounding box of all monitors.
    pub fn desktop_bounds(&self) -> Option<Rectangle<i32, Logical>> {
        self.outputs.iter().filter_map(|o| self.space.output_geometry(o)).reduce(|a, b| a.merge(b))
    }

    /// Move the focused window to the next monitor: onto the primary it tiles again, elsewhere it floats.
    pub fn move_to_next_output(&mut self) {
        if self.outputs.len() < 2 {
            return;
        }
        let Some(i) = self.focused.and_then(|f| self.client_index(f)) else { return };
        let geo = self.space.element_geometry(&self.clients[i].window);
        let cur = geo
            .and_then(|g| self.output_at((g.loc.x as f64 + g.size.w as f64 / 2.0, g.loc.y as f64 + g.size.h as f64 / 2.0).into()))
            .and_then(|o| self.outputs.iter().position(|x| *x == o))
            .unwrap_or(0);
        let next = (cur + 1) % self.outputs.len();
        let Some(target) = self.space.output_geometry(&self.outputs[next]) else { return };
        if next == 0 {
            self.clients[i].floating = false;
            self.clients[i].float_rect = None;
        } else {
            let (w, h) = (target.size.w * 7 / 10, target.size.h * 7 / 10);
            self.clients[i].floating = true;
            self.clients[i].float_rect = Some(Rect::new(target.loc.x + (target.size.w - w) / 2, target.loc.y + (target.size.h - h) / 2, w, h));
        }
        if self.clients[i].fullscreen {
            self.clients[i].fs_rect = Some(Rect::new(target.loc.x, target.loc.y, target.size.w, target.size.h));
        }
        self.relayout();
    }

    // ---------------------------------------------------------------- lookup

    pub fn client_index_for_surface(&self, surface: &WlSurface) -> Option<usize> {
        self.clients.iter().position(|c| &c.surface() == surface)
    }

    pub fn client_by_id(&self, id: u32) -> Option<&Client> {
        self.clients.iter().find(|c| c.id == id)
    }

    fn client_index(&self, id: u32) -> Option<usize> {
        self.clients.iter().position(|c| c.id == id)
    }

    /// Ids of the clients on a workspace in tiling order.
    pub fn ws_ids(&self, ws: usize) -> Vec<u32> {
        self.clients.iter().filter(|c| c.workspace == ws).map(|c| c.id).collect()
    }

    // ---------------------------------------------------------------- windows

    pub fn add_toplevel(&mut self, window: Window, floating: bool) {
        let id = self.next_id;
        self.next_id += 1;
        let color = parse_color(&self.config.colors.border_inactive);
        self.clients.push(Client {
            id,
            window,
            workspace: self.active_ws,
            floating,
            fullscreen: false,
            float_rect: None,
            fs_rect: None,
            asked_size: None,
            borders: std::array::from_fn(|_| SolidColorBuffer::new((0, 0), color)),
            ring: None,
        });
        self.relayout();
        self.focus_client(Some(id));
    }

    pub fn remove_toplevel(&mut self, surface: &WlSurface) {
        let Some(idx) = self.client_index_for_surface(surface) else { return };
        let client = self.clients.remove(idx);
        self.space.unmap_elem(&client.window);
        for slot in self.last_focus.iter_mut() {
            if *slot == Some(client.id) {
                *slot = None;
            }
        }
        if self.focused == Some(client.id) {
            self.focused = None;
            let next = self.last_focus[self.active_ws]
                .filter(|id| self.client_by_id(*id).is_some())
                .or_else(|| self.ws_ids(self.active_ws).last().copied());
            self.relayout();
            self.focus_client(next);
        } else {
            self.relayout();
        }
    }

    /// Give keyboard focus to a client (or nobody).
    pub fn focus_client(&mut self, id: Option<u32>) {
        self.focused = id;
        if let Some(id) = id {
            self.last_focus[self.active_ws] = Some(id);
        }
        for c in &self.clients {
            if let Some(top) = c.window.toplevel() {
                let active = Some(c.id) == id;
                if c.window.set_activated(active) {
                    top.send_pending_configure();
                }
            }
        }
        if let Some(window) = id.and_then(|i| self.client_by_id(i)).map(|c| c.window.clone()) {
            if self.client_by_id(id.unwrap()).is_some_and(|c| c.floating || c.fullscreen) {
                self.space.raise_element(&window, false);
            }
        }
        if self.layer_focus.is_none() {
            self.set_keyboard_focus(id.and_then(|i| self.client_by_id(i)).map(|c| c.surface()));
        }
    }

    pub fn set_keyboard_focus(&mut self, surface: Option<WlSurface>) {
        let serial: Serial = SERIAL_COUNTER.next_serial();
        if let Some(kb) = self.seat.get_keyboard() {
            kb.set_focus(self, surface, serial);
        }
    }

    /// Return keyboard focus to the focused window after a layer surface lets go of it.
    pub fn release_layer_focus(&mut self) {
        self.layer_focus = None;
        let id = self.focused;
        self.set_keyboard_focus(id.and_then(|i| self.client_by_id(i)).map(|c| c.surface()));
    }

    pub fn focus_cycle(&mut self, forward: bool) {
        let ids = self.ws_ids(self.active_ws);
        if ids.is_empty() {
            return;
        }
        let cur = self.focused.and_then(|f| ids.iter().position(|i| *i == f)).unwrap_or(0);
        let next = if forward { (cur + 1) % ids.len() } else { (cur + ids.len() - 1) % ids.len() };
        self.focus_client(Some(ids[next]));
    }

    /// Swap the focused tile with its neighbour in tiling order.
    pub fn swap_focused(&mut self, forward: bool) {
        if self.config.layout.mode == TileMode::Smart {
            let ws = self.active_ws;
            if let (Some(tree), Some(f)) = (self.trees[ws].as_mut(), self.focused) {
                let mut order = Vec::new();
                tree.ids(&mut order);
                if let Some(pos) = order.iter().position(|i| *i == f) {
                    if order.len() > 1 {
                        let other = if forward { order[(pos + 1) % order.len()] } else { order[(pos + order.len() - 1) % order.len()] };
                        tree.swap_leaves(f, other);
                        self.relayout();
                    }
                }
            }
            return;
        }
        let Some(f) = self.focused.and_then(|f| self.client_index(f)) else { return };
        let ws = self.clients[f].workspace;
        let order: Vec<usize> = (0..self.clients.len()).filter(|i| self.clients[*i].workspace == ws).collect();
        let Some(pos) = order.iter().position(|i| *i == f) else { return };
        if order.len() < 2 {
            return;
        }
        let other = if forward { order[(pos + 1) % order.len()] } else { order[(pos + order.len() - 1) % order.len()] };
        self.clients.swap(f, other);
        self.relayout();
    }

    pub fn close_focused(&mut self) {
        if let Some(c) = self.focused.and_then(|f| self.client_by_id(f)) {
            if let Some(t) = c.window.toplevel() {
                t.send_close();
            }
        }
    }

    pub fn close_client(&mut self, id: u32) {
        if let Some(t) = self.client_by_id(id).and_then(|c| c.window.toplevel()) {
            t.send_close();
        }
    }

    pub fn toggle_floating(&mut self) {
        if let Some(i) = self.focused.and_then(|f| self.client_index(f)) {
            self.clients[i].floating = !self.clients[i].floating;
            self.relayout();
            let id = self.clients[i].id;
            self.focus_client(Some(id));
        }
    }

    pub fn set_fullscreen(&mut self, surface: &WlSurface, on: bool) {
        if let Some(i) = self.client_index_for_surface(surface) {
            self.clients[i].fullscreen = on;
            // Fullscreen covers the monitor the pointer is on.
            self.clients[i].fs_rect = on
                .then(|| self.output_at(self.pointer_location).or_else(|| self.output.clone()))
                .flatten()
                .and_then(|o| self.space.output_geometry(&o))
                .map(|g| Rect::new(g.loc.x, g.loc.y, g.size.w, g.size.h));
            if let Some(t) = self.clients[i].window.toplevel() {
                t.with_pending_state(|s| {
                    if on {
                        s.states.set(xdg_toplevel::State::Fullscreen);
                    } else {
                        s.states.unset(xdg_toplevel::State::Fullscreen);
                    }
                });
            }
            self.relayout();
        }
    }

    pub fn toggle_fullscreen(&mut self) {
        if let Some(i) = self.focused.and_then(|f| self.client_index(f)) {
            let on = !self.clients[i].fullscreen;
            let s = self.clients[i].surface();
            self.set_fullscreen(&s, on);
        }
    }

    pub fn cycle_layout(&mut self) {
        self.config.layout.mode = self.config.layout.mode.next();
        self.relayout();
    }

    // ------------------------------------------------------------- workspaces

    pub fn switch_workspace(&mut self, n: usize) {
        if n >= self.config.general.workspaces || n == self.active_ws {
            return;
        }
        self.active_ws = n;
        self.relayout();
        let next = self.last_focus[n]
            .filter(|id| self.client_by_id(*id).is_some())
            .or_else(|| self.ws_ids(n).first().copied());
        self.focus_client(next);
    }

    pub fn move_focused_to_workspace(&mut self, n: usize) {
        if n >= self.config.general.workspaces {
            return;
        }
        let Some(i) = self.focused.and_then(|f| self.client_index(f)) else { return };
        if self.clients[i].workspace == n {
            return;
        }
        let id = self.clients[i].id;
        self.clients[i].workspace = n;
        for slot in self.last_focus.iter_mut() {
            if *slot == Some(id) {
                *slot = None;
            }
        }
        self.focused = None;
        self.relayout();
        let next = self.ws_ids(self.active_ws).last().copied();
        self.focus_client(next);
    }

    // ----------------------------------------------------------------- layout

    /// The output rectangle minus layer-shell exclusive zones, in global coordinates.
    pub fn usable_area(&self) -> Option<Rectangle<i32, Logical>> {
        let output = self.output.as_ref()?;
        let geo = self.space.output_geometry(output)?;
        let zone = layer_map_for_output(output).non_exclusive_zone();
        Some(Rectangle::new(geo.loc + zone.loc, zone.size))
    }

    /// Recompute all window positions and sizes, and (un)map windows per workspace.
    pub fn relayout(&mut self) {
        self.needs_render = true;
        let Some(output) = self.output.clone() else { return };
        let Some(out_geo) = self.space.output_geometry(&output) else { return };
        let Some(usable) = self.usable_area() else { return };

        let lay = self.config.layout.clone();
        let bw = lay.border_width.max(0);
        let area = Rect::new(usable.loc.x, usable.loc.y, usable.size.w, usable.size.h).inset(lay.gap_outer);

        let tiled: Vec<usize> = (0..self.clients.len())
            .filter(|i| {
                let c = &self.clients[*i];
                c.workspace == self.active_ws && !c.floating && !c.fullscreen
            })
            .collect();
        let tiles: Vec<Rect> = if lay.mode == TileMode::Smart {
            let ids: Vec<u32> = tiled.iter().map(|i| self.clients[*i].id).collect();
            self.smart_tiles(&ids, area, lay.gap_inner)
        } else {
            layout::compute(lay.mode, tiled.len(), area, lay.gap_inner, lay.master_ratio)
        };

        if self.float_oversized() {
            self.relayout_again();
            return;
        }

        // Apply tiled geometry.
        let mut placements: Vec<(usize, Rect, bool)> = Vec::new();
        for (slot, idx) in tiled.iter().enumerate() {
            placements.push((*idx, tiles[slot].inset(bw), true));
        }
        for (idx, c) in self.clients.iter().enumerate() {
            if c.workspace != self.active_ws {
                continue;
            }
            if c.fullscreen {
                let r = c.fs_rect.unwrap_or_else(|| Rect::new(out_geo.loc.x, out_geo.loc.y, out_geo.size.w, out_geo.size.h));
                placements.push((idx, r, false));
            } else if c.floating {
                let (min_w, min_h) = (0, 0);
                let r = self.clients[idx].float_rect.unwrap_or_else(|| {
                    let w = ((area.w as f64 * 0.6) as i32).max(min_w).min(area.w);
                    let h = ((area.h as f64 * 0.65) as i32).max(min_h).min(area.h);
                    Rect::new(area.x + (area.w - w) / 2, area.y + (area.h - h) / 2, w, h)
                });
                placements.push((idx, r, false));
            }
        }

        let mut visible = Vec::new();
        for (idx, rect, tiled_state) in placements {
            let c = &mut self.clients[idx];
            if c.floating && c.float_rect.is_none() {
                c.float_rect = Some(rect);
            }
            c.asked_size = Some((rect.w, rect.h));
            if let Some(top) = c.window.toplevel() {
                top.with_pending_state(|s| {
                    s.size = Some((rect.w, rect.h).into());
                    for st in [
                        xdg_toplevel::State::TiledLeft,
                        xdg_toplevel::State::TiledRight,
                        xdg_toplevel::State::TiledTop,
                        xdg_toplevel::State::TiledBottom,
                    ] {
                        if tiled_state {
                            s.states.set(st);
                        } else {
                            s.states.unset(st);
                        }
                    }
                });
                if top.is_initial_configure_sent() {
                    top.send_pending_configure();
                }
            }
            visible.push((c.window.clone(), Point::from((rect.x, rect.y))));
        }

        // Unmap windows that aren't on the active workspace.
        let hidden: Vec<Window> = self
            .clients
            .iter()
            .filter(|c| c.workspace != self.active_ws)
            .map(|c| c.window.clone())
            .collect();
        for w in hidden {
            self.space.unmap_elem(&w);
        }
        for (window, loc) in visible {
            self.space.map_element(window, loc, false);
        }
    }

    /// True if the window acknowledged the size the layout asked for but still draws larger than it,
    /// i.e. it can't shrink to its tile (GTK4 windows with a large natural minimum, for example).
    pub fn outgrew_tile(&self, idx: usize) -> bool {
        let c = &self.clients[idx];
        let (Some(want), Some(top)) = (c.asked_size, c.window.toplevel()) else { return false };
        let acked = top.with_committed_state(|st| st.and_then(|s| s.size));
        if acked != Some(want.into()) {
            tracing::debug!("tile check: asked {want:?} but acked {acked:?}");
            return false; // the client hasn't caught up with our latest configure yet
        }
        let g = c.window.geometry().size;
        tracing::debug!("tile check: asked {want:?} acked {acked:?} drawn {g:?}");
        g.w > want.0 + 2 || g.h > want.1 + 2
    }

    /// Float tiled windows that can't fit their tile. Returns true if any changed.
    fn float_oversized(&mut self) -> bool {
        let mut changed = false;
        for i in 0..self.clients.len() {
            if self.clients[i].floating || self.clients[i].fullscreen || self.clients[i].workspace != self.active_ws {
                continue;
            }
            if self.outgrew_tile(i) {
                self.clients[i].floating = true;
                changed = true;
            }
        }
        changed
    }

    /// Keep the active workspace's split tree in sync with its tiled windows and return one tile
    /// per id, in the order of `ids`. New windows split the focused tile along its longer side.
    fn smart_tiles(&mut self, ids: &[u32], area: Rect, gap: i32) -> Vec<Rect> {
        let ws = self.active_ws;
        let mut tree = self.trees[ws].take();

        let mut present = Vec::new();
        if let Some(t) = &tree {
            t.ids(&mut present);
        }
        for id in present {
            if !ids.contains(&id) {
                tree = tree.and_then(|t| t.remove(id));
            }
        }
        for &id in ids {
            if tree.as_ref().is_some_and(|t| t.contains(id)) {
                continue;
            }
            match tree.as_mut() {
                None => tree = Some(Node::Leaf(id)),
                Some(t) => {
                    let mut rects = Vec::new();
                    t.rects(area, gap, &mut rects);
                    let target = self
                        .focused
                        .filter(|f| t.contains(*f))
                        .or_else(|| rects.last().map(|(i, _)| *i))
                        .expect("non-empty tree");
                    let target_rect = rects.iter().find(|(i, _)| *i == target).map(|(_, r)| *r).unwrap_or(area);
                    t.insert(target, id, target_rect, gap);
                }
            }
        }

        let mut rects = Vec::new();
        if let Some(t) = &tree {
            t.rects(area, gap, &mut rects);
        }
        self.trees[ws] = tree;
        ids.iter()
            .map(|id| rects.iter().find(|(i, _)| i == id).map(|(_, r)| *r).unwrap_or(area))
            .collect()
    }

    /// `relayout`, called again after a window was floated (bounded: floating windows aren't re-checked).
    fn relayout_again(&mut self) {
        self.relayout();
    }

    /// Re-read the config file and apply it.
    pub fn reload_config(&mut self) {
        self.needs_render = true;
        let cfg = Config::load();
        let n = cfg.general.workspaces;
        self.last_focus.resize(n, None);
        self.trees.resize(n, None);
        for c in &mut self.clients {
            c.workspace = c.workspace.min(n - 1);
        }
        self.active_ws = self.active_ws.min(n - 1);
        self.bindings = default_bindings(&cfg);
        self.config = cfg;
        self.apply_input_config();
        self.reload_cursor();
        self.mode_dirty = true;
        self.relayout();
        tracing::info!("configuration reloaded");
    }

    // ---------------------------------------------------------------- input and power

    /// Reload the pointer image for the current pointer size and the primary monitor's scale.
    pub fn reload_cursor(&mut self) {
        let scale = self.output.as_ref().map_or(1, |o| o.current_scale().integer_scale());
        self.cursor = Cursor::load(self.config.accessibility.cursor_size(), scale);
        self.needs_render = true;
    }

    /// Open a status bar on a secondary monitor.
    pub fn spawn_bar_on(&self, output_name: &str) {
        let mut cmd = Command::new("safora-bar");
        cmd.env("WAYLAND_DISPLAY", &self.socket_name).env("SAFORA_BAR_OUTPUT", output_name);
        match cmd.spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(e) => tracing::warn!("cannot start the bar on {output_name}: {e}"),
        }
    }

    /// Apply tap-to-click, scrolling, pointer speed and friends to one libinput device.
    pub fn configure_device(&self, device: &mut smithay::reexports::input::Device) {
        let i = &self.config.input;
        if device.config_tap_finger_count() > 0 {
            let _ = device.config_tap_set_enabled(i.tap_to_click);
        }
        if device.config_scroll_has_natural_scroll() {
            let _ = device.config_scroll_set_natural_scroll_enabled(i.natural_scroll);
        }
        if device.config_accel_is_available() {
            let _ = device.config_accel_set_speed(i.pointer_speed);
        }
        if device.config_left_handed_is_available() {
            let _ = device.config_left_handed_set(i.left_handed);
        }
        if device.config_dwt_is_available() {
            let _ = device.config_dwt_set_enabled(i.disable_while_typing);
        }
    }

    /// Re-apply the input settings to every known device and to key repeat.
    pub fn apply_input_config(&mut self) {
        let mut devices = std::mem::take(&mut self.input_devices);
        for d in &mut devices {
            self.configure_device(d);
        }
        self.input_devices = devices;
        if let Some(kb) = self.seat.get_keyboard() {
            kb.change_repeat_info(self.config.input.repeat_rate, self.config.input.repeat_delay_ms);
        }
    }

    /// Dark screen and suspend after the configured idle time. A fullscreen window (video, game) keeps the screen on.
    pub fn check_idle(&mut self) {
        let idle = self.last_input.elapsed().as_secs();
        let p = self.config.power.clone();
        let fullscreen = self.clients.iter().any(|c| c.fullscreen && c.workspace == self.active_ws);
        if p.screen_off_minutes > 0 && !fullscreen && !self.blanked && idle >= p.screen_off_minutes as u64 * 60 {
            self.blanked = true;
            self.needs_render = true;
        }
        if p.lock_minutes > 0 && !fullscreen && !self.locked && idle >= p.lock_minutes as u64 * 60 {
            self.lock();
        }
        if p.suspend_minutes > 0 && idle >= p.suspend_minutes as u64 * 60 {
            self.last_input = Instant::now();
            self.lock();
            self.spawn("systemctl suspend");
        }
        self.keep_lock_screen_alive();
    }

    // ---------------------------------------------------------------- screen lock

    /// Lock the screen: hide everything, send all input to the lock screen program.
    pub fn lock(&mut self) {
        // The live installer session has no password, so it could never be unlocked.
        if self.locked || std::path::Path::new("/etc/safora/live").exists() {
            return;
        }
        let mut bytes = [0u8; 16];
        if std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes)).is_err() {
            tracing::error!("cannot read /dev/urandom, not locking");
            return;
        }
        self.lock_token = bytes.iter().map(|b| format!("{b:02x}")).collect();
        self.locked = true;
        self.layer_focus = None;
        self.set_keyboard_focus(None);
        // Drop pointer focus from the window under the pointer so a click can't reach it.
        self.on_pointer_moved(smithay::backend::input::InputTime::now());
        self.needs_render = true;
        tracing::info!("screen locked");
        self.spawn_lock_screen();
    }

    fn spawn_lock_screen(&mut self) {
        self.lock_spawned = Some(Instant::now());
        let mut cmd = Command::new("safora-lock");
        cmd.env("WAYLAND_DISPLAY", &self.socket_name).env("SAFORA_LOCK_TOKEN", &self.lock_token);
        match cmd.spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(e) => tracing::error!("cannot start the lock screen: {e}"),
        }
    }

    /// Unlock if `token` is the one given to the lock screen process.
    pub fn unlock(&mut self, token: &str) {
        if !self.locked || token.len() != self.lock_token.len() {
            return;
        }
        // Compare without early exit.
        let diff = token.bytes().zip(self.lock_token.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y));
        if diff != 0 {
            tracing::warn!("unlock refused: wrong token");
            return;
        }
        self.locked = false;
        self.lock_token.clear();
        self.lock_spawned = None;
        self.release_layer_focus();
        self.last_input = Instant::now();
        self.needs_render = true;
        tracing::info!("screen unlocked");
    }

    /// The lock screen's overlay surface, if it is on screen.
    fn lock_screen_surface(&self) -> Option<WlSurface> {
        use smithay::desktop::layer_map_for_output;
        for o in &self.outputs {
            let map = layer_map_for_output(o);
            let found = map.layers().find(|l| l.namespace() == "safora-lock").map(|l| l.wl_surface().clone());
            if found.is_some() {
                return found;
            }
        }
        None
    }

    /// While locked: give the lock screen the keyboard, and restart it if it died.
    pub fn keep_lock_screen_alive(&mut self) {
        if !self.locked {
            return;
        }
        match self.lock_screen_surface() {
            Some(s) => {
                if self.layer_focus.as_ref() != Some(&s) {
                    self.layer_focus = Some(s.clone());
                    self.set_keyboard_focus(Some(s));
                }
            }
            None => {
                if self.lock_spawned.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(4)) {
                    self.spawn_lock_screen();
                }
            }
        }
    }

    /// Any input wakes the screen and restarts the idle timers.
    pub fn note_activity(&mut self) {
        self.last_input = Instant::now();
        if self.blanked {
            self.blanked = false;
            self.needs_render = true;
        }
    }

    // ---------------------------------------------------------------- actions

    pub fn run_action(&mut self, action: Action) {
        match action {
            Action::None => {}
            Action::Spawn(cmd) => self.spawn(&cmd),
            Action::Workspace(n) => self.switch_workspace(n),
            Action::MoveToWorkspace(n) => self.move_focused_to_workspace(n),
            Action::Close => self.close_focused(),
            Action::ToggleFloating => self.toggle_floating(),
            Action::NextOutput => self.move_to_next_output(),
            Action::Lock => self.lock(),
            Action::ToggleFullscreen => self.toggle_fullscreen(),
            Action::CycleLayout => self.cycle_layout(),
            Action::FocusNext => self.focus_cycle(true),
            Action::FocusPrev => self.focus_cycle(false),
            Action::SwapNext => self.swap_focused(true),
            Action::SwapPrev => self.swap_focused(false),
            Action::Reload => self.reload_config(),
            Action::Quit => self.loop_signal.stop(),
            Action::VtSwitch(vt) => self.vt_switch = Some(vt),
        }
    }

    pub fn spawn(&self, command: &str) {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd.env("WAYLAND_DISPLAY", &self.socket_name);
        cmd.env("XDG_SESSION_TYPE", "wayland");
        cmd.env("XDG_CURRENT_DESKTOP", "Safora");
        match cmd.spawn() {
            Ok(mut child) => {
                // Reap the child when it exits so we don't leave zombies behind.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(e) => tracing::warn!("failed to spawn `{command}`: {e}"),
        }
    }

    // -------------------------------------------------------------------- IPC

    pub fn outputs_info(&self) -> Vec<safora_common::ipc::OutputInfo> {
        use safora_common::ipc::{ModeInfo, OutputInfo};
        self.outputs
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let current = o.current_mode().map(|m| ModeInfo {
                    width: m.size.w as u32,
                    height: m.size.h as u32,
                    refresh_mhz: m.refresh as u32,
                    preferred: false,
                });
                // The settings app lists the modes of the primary monitor.
                OutputInfo { name: o.name(), current, modes: if i == 0 { self.output_modes.clone() } else { Vec::new() } }
            })
            .collect()
    }

    pub fn ipc_state(&self) -> IpcState {
        let windows = self
            .clients
            .iter()
            .map(|c| {
                let (title, app_id) = with_states(&c.surface(), |s| {
                    let d = s.data_map.get::<XdgToplevelSurfaceData>().map(|d| d.lock().unwrap());
                    d.map(|d| (d.title.clone().unwrap_or_default(), d.app_id.clone().unwrap_or_default()))
                        .unwrap_or_default()
                });
                WindowInfo { id: c.id, title, app_id, workspace: c.workspace, focused: Some(c.id) == self.focused }
            })
            .collect();
        let workspaces = (0..self.config.general.workspaces)
            .map(|id| WorkspaceInfo { id, windows: self.clients.iter().filter(|c| c.workspace == id).count() })
            .collect();
        IpcState {
            workspaces,
            active: self.active_ws,
            windows,
            layout: format!("{:?}", self.config.layout.mode).to_lowercase(),
        }
    }

    /// Push the state to every subscriber if it changed since the last push.
    pub fn broadcast_state(&mut self) {
        if self.ipc_subscribers.is_empty() {
            return;
        }
        let state = self.ipc_state();
        if self.ipc_last.as_ref() == Some(&state) {
            return;
        }
        let mut line = serde_json::to_string(&state).unwrap_or_default();
        line.push('\n');
        use std::io::Write;
        self.ipc_subscribers.retain_mut(|s| s.write_all(line.as_bytes()).is_ok());
        self.ipc_last = Some(state);
    }

    /// Topmost window under a point (global coordinates).
    pub fn window_under(&self, pos: Point<f64, Logical>) -> Option<(Window, Point<i32, Logical>)> {
        self.space.element_under(pos).map(|(w, l)| (w.clone(), l))
    }

    /// The surface under a point, searching layers above windows, windows, then layers below.
    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        use smithay::wayland::shell::wlr_layer::Layer;
        let output = self.output_at(pos).or_else(|| self.output.clone())?;
        let output = &output;
        let geo = self.space.output_geometry(output)?;
        let rel = pos - geo.loc.to_f64();
        let map = layer_map_for_output(output);

        let from_layer = |layer: Layer| {
            let l = map.layer_under(layer, rel)?;
            let lgeo = map.layer_geometry(l)?;
            let (s, p) = l.surface_under(rel - lgeo.loc.to_f64(), WindowSurfaceType::ALL)?;
            Some((s, (p + lgeo.loc + geo.loc).to_f64()))
        };

        if let Some(hit) = from_layer(Layer::Overlay) {
            return Some(hit);
        }
        if self.locked {
            return None;
        }
        let fullscreen_active = self.clients.iter().any(|c| c.fullscreen && c.workspace == self.active_ws);
        if !fullscreen_active {
            if let Some(hit) = from_layer(Layer::Top) {
                return Some(hit);
            }
        }
        if let Some((window, loc)) = self.window_under(pos) {
            if let Some((s, p)) = window.surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL) {
                return Some((s, (p + loc).to_f64()));
            }
        }
        from_layer(Layer::Bottom).or_else(|| from_layer(Layer::Background))
    }

    pub fn border_colors(&self) -> ([f32; 4], [f32; 4]) {
        (parse_color(&self.config.colors.border_active), parse_color(&self.config.colors.border_inactive))
    }
}
