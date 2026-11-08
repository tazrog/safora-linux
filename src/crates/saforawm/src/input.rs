//! Keybindings and input event handling.

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, InputTime, KeyState,
        KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    input::{
        keyboard::{keysyms, xkb, FilterResult, Keysym},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::shell::wlr_layer::{KeyboardInteractivity, Layer},
};
use safora_common::Config;

use crate::state::SaforaState;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    Spawn(String),
    Workspace(usize),
    MoveToWorkspace(usize),
    Close,
    ToggleFloating,
    ToggleFullscreen,
    CycleLayout,
    FocusNext,
    FocusPrev,
    SwapNext,
    SwapPrev,
    Reload,
    Quit,
    VtSwitch(i32),
    /// Send the focused window to the next monitor.
    NextOutput,
    /// Lock the screen.
    Lock,
    /// Make the focused window wider (+x) / narrower (-x) / taller (+y) / shorter (-y).
    Resize(i32, i32),
    /// Give the windows on this workspace equal shares again.
    ResizeReset,
}

impl Action {
    /// Parse the action syntax used in `[keybinds]`: `spawn:cmd`, `workspace:3`, `close`, …
    pub fn parse(s: &str) -> Option<Action> {
        let (head, arg) = s.split_once(':').map_or((s, ""), |(h, a)| (h, a));
        Some(match head.trim() {
            "spawn" => Action::Spawn(arg.trim().to_string()),
            "workspace" => Action::Workspace(arg.trim().parse::<usize>().ok()?.checked_sub(1)?),
            "move" => Action::MoveToWorkspace(arg.trim().parse::<usize>().ok()?.checked_sub(1)?),
            "close" => Action::Close,
            "float" => Action::ToggleFloating,
            "output" => Action::NextOutput,
            "lock" => Action::Lock,
            "fullscreen" => Action::ToggleFullscreen,
            "layout" => Action::CycleLayout,
            "next" => Action::FocusNext,
            "prev" => Action::FocusPrev,
            "swap_next" => Action::SwapNext,
            "swap_prev" => Action::SwapPrev,
            "resize" => match arg.trim() {
                "wider" => Action::Resize(1, 0),
                "narrower" => Action::Resize(-1, 0),
                "taller" => Action::Resize(0, 1),
                "shorter" => Action::Resize(0, -1),
                "reset" => Action::ResizeReset,
                _ => return None,
            },
            "reload" => Action::Reload,
            "quit" => Action::Quit,
            "none" => Action::None,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub logo: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub sym: Keysym,
    pub action: Action,
}

impl Binding {
    /// Parse `"Mod+Shift+Return"`. `Mod` resolves to the configured modifier.
    pub fn parse(spec: &str, modifier: &str, action: Action) -> Option<Binding> {
        let mut b = Binding { logo: false, alt: false, ctrl: false, shift: false, sym: Keysym::NoSymbol, action };
        let mut key = None;
        for part in spec.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "mod" => {
                    if modifier.eq_ignore_ascii_case("alt") {
                        b.alt = true
                    } else {
                        b.logo = true
                    }
                }
                "super" | "logo" | "win" => b.logo = true,
                "alt" => b.alt = true,
                "ctrl" | "control" => b.ctrl = true,
                "shift" => b.shift = true,
                _ => key = Some(part),
            }
        }
        let sym = xkb::keysym_from_name(key?, xkb::KEYSYM_CASE_INSENSITIVE);
        if sym.raw() == 0 {
            return None;
        }
        b.sym = sym;
        Some(b)
    }
}

pub fn default_bindings(cfg: &Config) -> Vec<Binding> {
    let m = cfg.general.modifier.as_str();
    let mut specs: Vec<(String, String)> = vec![
        ("Mod+Return".into(), format!("spawn:{}", cfg.general.terminal)),
        ("Mod+d".into(), format!("spawn:{}", cfg.general.launcher)),
        ("Mod+w".into(), "close".into()),
        ("Mod+f".into(), "fullscreen".into()),
        ("Mod+Shift+space".into(), "float".into()),
        ("Mod+o".into(), "output".into()),
        ("Mod+l".into(), "lock".into()),
        ("Mod+space".into(), "layout".into()),
        ("Mod+j".into(), "next".into()),
        ("Mod+k".into(), "prev".into()),
        ("Mod+Tab".into(), "next".into()),
        ("Mod+Shift+j".into(), "swap_next".into()),
        ("Mod+Shift+k".into(), "swap_prev".into()),
        ("Mod+Shift+r".into(), "reload".into()),
        ("Mod+Shift+e".into(), "quit".into()),
        ("Mod+s".into(), "spawn:safora-settings".into()),
        ("Mod+Ctrl+Right".into(), "resize:wider".into()),
        ("Mod+Ctrl+Left".into(), "resize:narrower".into()),
        ("Mod+Ctrl+Down".into(), "resize:taller".into()),
        ("Mod+Ctrl+Up".into(), "resize:shorter".into()),
        ("Mod+Ctrl+0".into(), "resize:reset".into()),
    ];
    for n in 1..=9 {
        specs.push((format!("Mod+{n}"), format!("workspace:{n}")));
        specs.push((format!("Mod+Shift+{n}"), format!("move:{n}")));
    }
    // User bindings override defaults with the same key spec.
    for (k, v) in &cfg.keybinds {
        specs.retain(|(dk, _)| !dk.eq_ignore_ascii_case(k));
        specs.push((k.clone(), v.clone()));
    }
    let mut out: Vec<Binding> = Vec::new();
    for (k, v) in specs {
        let Some(action) = Action::parse(&v) else {
            tracing::warn!("keybind {k}: unknown action `{v}`");
            continue;
        };
        let Some(b) = Binding::parse(&k, m, action) else {
            tracing::warn!("keybind `{k}` could not be parsed");
            continue;
        };
        // A later binding on the same keys wins, however it is spelled ("Super+Return" vs "Mod+Return").
        out.retain(|o| !o.same_keys(&b));
        out.push(b);
    }
    out
}

impl Binding {
    fn same_keys(&self, o: &Binding) -> bool {
        self.sym == o.sym && self.logo == o.logo && self.alt == o.alt && self.ctrl == o.ctrl && self.shift == o.shift
    }
}

impl SaforaState {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        self.needs_render = true;
        self.note_activity();
        match event {
            InputEvent::Keyboard { event } => self.on_key::<I>(event),
            InputEvent::PointerMotion { event } => {
                let before = self.pointer_location;
                self.pointer_location += (event.delta_x(), event.delta_y()).into();
                self.clamp_pointer(before);
                self.on_pointer_moved(event.time());
            }
            InputEvent::PointerMotionAbsolute { event } => {
                if let Some(geo) = self.desktop_bounds() {
                    let before = self.pointer_location;
                    self.pointer_location = event.position_transformed(geo.size) + geo.loc.to_f64();
                    self.clamp_pointer(before);
                    self.on_pointer_moved(event.time());
                }
            }
            InputEvent::PointerButton { event } => self.on_button::<I>(event),
            InputEvent::PointerAxis { event } => self.on_axis::<I>(event),
            _ => {}
        }
    }

    /// Keep the pointer on a monitor: if it left them all, put it on the edge of the one it came from.
    fn clamp_pointer(&mut self, before: smithay::utils::Point<f64, smithay::utils::Logical>) {
        if self.output_at(self.pointer_location).is_some() {
            return;
        }
        let home = self.output_at(before).or_else(|| self.output.clone());
        if let Some(geo) = home.and_then(|o| self.space.output_geometry(&o)) {
            let max_x = (geo.loc.x + geo.size.w - 1) as f64;
            let max_y = (geo.loc.y + geo.size.h - 1) as f64;
            self.pointer_location.x = self.pointer_location.x.clamp(geo.loc.x as f64, max_x);
            self.pointer_location.y = self.pointer_location.y.clamp(geo.loc.y as f64, max_y);
        }
    }

    fn on_key<I: InputBackend>(&mut self, event: I::KeyboardKeyEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let time = event.time();
        let code = event.key_code();
        let key_state = event.state();
        let Some(keyboard) = self.seat.get_keyboard() else { return };

        let action = keyboard.input::<Action, _>(self, code, key_state, serial, time, |state, mods, handle| {
            if key_state == KeyState::Released {
                // Swallow the release of any key whose press we intercepted.
                if let Some(pos) = state.suppressed_keys.iter().position(|k| *k == code) {
                    state.suppressed_keys.swap_remove(pos);
                    return FilterResult::Intercept(Action::None);
                }
                return FilterResult::Forward;
            }

            let modified = handle.modified_sym();
            let raw = keysyms::KEY_XF86Switch_VT_1..=keysyms::KEY_XF86Switch_VT_12;
            if raw.contains(&modified.raw()) {
                state.suppressed_keys.push(code);
                let vt = (modified.raw() - keysyms::KEY_XF86Switch_VT_1 + 1) as i32;
                return FilterResult::Intercept(Action::VtSwitch(vt));
            }

            if state.locked {
                return FilterResult::Forward; // only the lock screen gets keys
            }

            let sym = handle.raw_latin_sym_or_raw_current_sym().unwrap_or(modified);
            let hit = state.bindings.iter().find(|b| {
                b.sym == sym && b.logo == mods.logo && b.alt == mods.alt && b.ctrl == mods.ctrl && b.shift == mods.shift
            });
            match hit {
                Some(b) => {
                    let action = b.action.clone();
                    state.suppressed_keys.push(code);
                    FilterResult::Intercept(action)
                }
                None => FilterResult::Forward,
            }
        });

        if let Some(action) = action {
            self.run_action(action);
        }
    }

    pub fn on_pointer_moved(&mut self, time: InputTime) {
        let pos = self.pointer_location;
        let under = self.surface_under(pos);

        // Focus follows mouse (only between windows, and never while a client holds a grab).
        if self.config.general.focus_follows_mouse && self.layer_focus.is_none() {
            let pointer = self.seat.get_pointer().unwrap();
            if !pointer.is_grabbed() {
                if let Some((window, _)) = self.window_under(pos) {
                    if let Some(idx) = self.client_index_for_surface(window.toplevel().unwrap().wl_surface()) {
                        let id = self.clients[idx].id;
                        if self.focused != Some(id) && under.as_ref().is_some_and(|(s, _)| !self.is_layer_surface(s)) {
                            self.focus_client(Some(id));
                        }
                    }
                }
            }
        }

        let pointer = self.seat.get_pointer().unwrap();
        pointer.motion(self, under, &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time });
        pointer.frame(self);
    }

    pub fn is_layer_surface(&self, surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface) -> bool {
        use smithay::desktop::{layer_map_for_output, WindowSurfaceType};
        self.outputs.iter().any(|o| layer_map_for_output(o).layer_for_surface(surface, WindowSurfaceType::ALL).is_some())
    }

    fn on_button<I: InputBackend>(&mut self, event: I::PointerButtonEvent) {
        let pointer = self.seat.get_pointer().unwrap();
        let serial = SERIAL_COUNTER.next_serial();
        let state = event.state();

        if state == ButtonState::Pressed && !pointer.is_grabbed() {
            let pos = self.pointer_location;
            let under = self.surface_under(pos);
            let layer_hit = under.as_ref().and_then(|(s, _)| self.layer_for(s));
            match layer_hit {
                Some((surface, interactivity)) => {
                    if interactivity != KeyboardInteractivity::None {
                        self.layer_focus = Some(surface.clone());
                        self.set_keyboard_focus(Some(surface));
                    }
                }
                None if self.locked => {}
                None => {
                    // Clicking a window releases any layer surface's keyboard grab on demand.
                    if self.layer_focus.is_some() && self.layer_on_demand() {
                        self.release_layer_focus();
                    }
                    if let Some((window, _)) = self.window_under(pos) {
                        if let Some(idx) = self.client_index_for_surface(window.toplevel().unwrap().wl_surface()) {
                            let id = self.clients[idx].id;
                            self.focus_client(Some(id));
                        }
                    }
                }
            }
        }

        pointer.button(self, &ButtonEvent { button: event.button_code(), state, serial, time: event.time() });
        pointer.frame(self);
    }

    /// The layer surface (root) for a surface plus its keyboard interactivity.
    fn layer_for(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> Option<(smithay::reexports::wayland_server::protocol::wl_surface::WlSurface, KeyboardInteractivity)> {
        use smithay::desktop::{layer_map_for_output, WindowSurfaceType};
        for output in &self.outputs {
            let map = layer_map_for_output(output);
            if let Some(layer) = map.layer_for_surface(surface, WindowSurfaceType::ALL) {
                let data = layer.cached_state().keyboard_interactivity;
                return Some((layer.wl_surface().clone(), data));
            }
        }
        None
    }

    pub(crate) fn layer_on_demand(&self) -> bool {
        self.layer_focus.as_ref().is_some_and(|s| {
            self.layer_for(s).is_some_and(|(_, i)| i == KeyboardInteractivity::OnDemand)
        })
    }

    fn on_axis<I: InputBackend>(&mut self, event: I::PointerAxisEvent) {
        let source = event.source();
        let amount = |axis| {
            event
                .amount(axis)
                .unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0)
        };
        let (h, v) = (amount(Axis::Horizontal), amount(Axis::Vertical));
        let mut frame = AxisFrame::new(event.time()).source(source);
        if h != 0.0 {
            frame = frame.value(Axis::Horizontal, h);
            if let Some(d) = event.amount_v120(Axis::Horizontal) {
                frame = frame.v120(Axis::Horizontal, d as i32);
            }
        }
        if v != 0.0 {
            frame = frame.value(Axis::Vertical, v);
            if let Some(d) = event.amount_v120(Axis::Vertical) {
                frame = frame.v120(Axis::Vertical, d as i32);
            }
        }
        if source == AxisSource::Finger {
            if event.amount(Axis::Horizontal) == Some(0.0) {
                frame = frame.stop(Axis::Horizontal);
            }
            if event.amount(Axis::Vertical) == Some(0.0) {
                frame = frame.stop(Axis::Vertical);
            }
        }
        let pointer = self.seat.get_pointer().unwrap();
        pointer.axis(self, frame);
        pointer.frame(self);
    }
}

#[allow(dead_code)]
fn _unused(_: Point<f64, Logical>, _: Layer) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_actions() {
        assert_eq!(Action::parse("workspace:3"), Some(Action::Workspace(2)));
        assert_eq!(Action::parse("spawn: foot -e htop"), Some(Action::Spawn("foot -e htop".into())));
        assert_eq!(Action::parse("workspace:0"), None);
        assert_eq!(Action::parse("bogus"), None);
        assert_eq!(Action::parse("resize:wider"), Some(Action::Resize(1, 0)));
        assert_eq!(Action::parse("resize:reset"), Some(Action::ResizeReset));
        assert_eq!(Action::parse("resize:sideways"), None);
    }

    #[test]
    fn parses_bindings() {
        let b = Binding::parse("Mod+Shift+Return", "super", Action::Close).unwrap();
        assert!(b.logo && b.shift && !b.alt && !b.ctrl);
        let b = Binding::parse("Mod+q", "alt", Action::Close).unwrap();
        assert!(b.alt && !b.logo);
        assert!(Binding::parse("Mod+NotAKey", "super", Action::Close).is_none());
    }

    #[test]
    fn defaults_cover_workspaces() {
        let binds = default_bindings(&Config::default());
        assert!(binds.iter().any(|b| b.action == Action::Workspace(8)));
        assert!(binds.iter().any(|b| b.action == Action::MoveToWorkspace(0)));
        assert!(binds.iter().any(|b| b.action == Action::Resize(0, -1)));
    }

    #[test]
    fn user_binding_overrides_default_spelled_differently() {
        let mut cfg = Config::default();
        cfg.keybinds.insert("Super+RETURN".into(), "spawn:foot".into());
        let binds = default_bindings(&cfg);
        let ret: Vec<_> = binds.iter().filter(|b| b.logo && !b.shift && b.sym == Keysym::Return).collect();
        assert_eq!(ret.len(), 1);
        assert_eq!(ret[0].action, Action::Spawn("foot".into()));
    }
}
