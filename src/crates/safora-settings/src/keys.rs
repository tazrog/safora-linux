//! Shortcuts page: the built-in key bindings plus an editor for custom ones.
//!
//! Custom shortcuts live in `[keybinds]` of `config.toml` as `"Mod+Shift+t" = "spawn:foot"`;
//! the compositor merges them over its defaults on every reload.

use std::rc::Rc;

use gtk4 as gtk;
use gtk::{gdk, glib, prelude::*};

use crate::{heading, page, row, Ctx};

/// The compositor's default bindings (keep in sync with `saforawm::input::default_bindings`).
const DEFAULT_SPECS: &[(&str, &str)] = &[
    ("Mod+Return", "Open terminal"),
    ("Mod+d", "Application launcher"),
    ("Mod+q", "Close window"),
    ("Mod+f", "Fullscreen"),
    ("Mod+Shift+space", "Float / tile window"),
    ("Mod+o", "Send window to next monitor"),
    ("Mod+l", "Lock screen"),
    ("Mod+space", "Change tiling layout"),
    ("Mod+j", "Focus next window"),
    ("Mod+k", "Focus previous window"),
    ("Mod+Tab", "Focus next window"),
    ("Mod+Shift+j", "Move window forward"),
    ("Mod+Shift+k", "Move window back"),
    ("Mod+Shift+r", "Reload configuration"),
    ("Mod+Shift+e", "Log out"),
    ("Mod+s", "Open these settings"),
];

/// Actions offered in the editor: (label, config action, argument kind).
#[derive(Clone, Copy, PartialEq)]
enum Arg {
    None,
    Command,
    Workspace,
}

const ACTIONS: &[(&str, &str, Arg)] = &[
    ("Run a command", "spawn", Arg::Command),
    ("Go to workspace", "workspace", Arg::Workspace),
    ("Send window to workspace", "move", Arg::Workspace),
    ("Close window", "close", Arg::None),
    ("Fullscreen", "fullscreen", Arg::None),
    ("Float / tile window", "float", Arg::None),
    ("Change tiling layout", "layout", Arg::None),
    ("Focus next window", "next", Arg::None),
    ("Focus previous window", "prev", Arg::None),
    ("Move window forward", "swap_next", Arg::None),
    ("Move window back", "swap_prev", Arg::None),
    ("Send window to next monitor", "output", Arg::None),
    ("Lock screen", "lock", Arg::None),
    ("Reload configuration", "reload", Arg::None),
    ("Log out", "quit", Arg::None),
    ("Disable this shortcut", "none", Arg::None),
];

/// Human description of a config action string.
fn describe(action: &str) -> String {
    let (head, arg) = action.split_once(':').map_or((action, ""), |(h, a)| (h.trim(), a.trim()));
    match ACTIONS.iter().find(|(_, a, _)| *a == head) {
        Some((_, "spawn", _)) => format!("Run: {arg}"),
        Some((label, _, Arg::Workspace)) => format!("{label} {arg}"),
        Some((label, _, _)) => label.to_string(),
        None => format!("Unknown action “{action}”"),
    }
}

/// A key spec reduced to comparable form: (super, alt, ctrl, shift, lower-case key name).
type Combo = (bool, bool, bool, bool, String);

fn combo(spec: &str, modifier: &str) -> Option<Combo> {
    let (mut logo, mut alt, mut ctrl, mut shift) = (false, false, false, false);
    let mut key = None;
    for part in spec.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "mod" if modifier.eq_ignore_ascii_case("alt") => alt = true,
            "mod" => logo = true,
            "super" | "logo" | "win" => logo = true,
            "alt" => alt = true,
            "ctrl" | "control" => ctrl = true,
            "shift" => shift = true,
            "" => return None,
            _ if key.is_some() => return None,
            _ => key = Some(part),
        }
    }
    let key = key?;
    // The compositor resolves names case-insensitively, as xkb does.
    if gdk::Key::from_name(key).is_none() && gdk::Key::from_name(key.to_ascii_lowercase()).is_none() {
        return None;
    }
    Some((logo, alt, ctrl, shift, key.to_ascii_lowercase()))
}

/// Pretty form for display: "Mod + Shift + T".
fn pretty(spec: &str) -> String {
    spec.split('+')
        .map(|p| {
            let p = p.trim();
            if p.chars().count() == 1 { p.to_uppercase() } else { p.to_string() }
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Turn a key press into a spec like "Mod+Shift+t". `None` for bare modifier presses.
fn spec_from_event(display: &gdk::Display, keyval: gdk::Key, keycode: u32, state: gdk::ModifierType, modifier: &str) -> Option<String> {
    let name = keyval.name()?.to_string();
    let is_mod = ["Shift", "Control", "Alt", "Super", "Meta", "Hyper", "ISO_Level", "Caps_Lock", "Num_Lock"]
        .iter()
        .any(|m| name.starts_with(m));
    if is_mod {
        return None;
    }
    // The compositor matches the unshifted key ("Mod+Shift+1", not "Mod+Shift+exclam").
    let base = display
        .map_keycode(keycode)
        .and_then(|keys| keys.into_iter().find(|(k, _)| k.group() == 0 && k.level() == 0).map(|(_, v)| v))
        .unwrap_or_else(|| keyval.to_lower());
    let key = base.name().map(|n| n.to_string()).unwrap_or(name);

    let alt_is_mod = modifier.eq_ignore_ascii_case("alt");
    let mut parts = Vec::new();
    if state.contains(gdk::ModifierType::SUPER_MASK) {
        parts.push(if alt_is_mod { "Super" } else { "Mod" });
    }
    if state.contains(gdk::ModifierType::ALT_MASK) {
        parts.push(if alt_is_mod { "Mod" } else { "Alt" });
    }
    if state.contains(gdk::ModifierType::CONTROL_MASK) {
        parts.push("Ctrl");
    }
    if state.contains(gdk::ModifierType::SHIFT_MASK) {
        parts.push("Shift");
    }
    parts.sort_by_key(|p| ["Mod", "Super", "Alt", "Ctrl", "Shift"].iter().position(|x| x == p));
    let mut spec = parts.join("+");
    if !spec.is_empty() {
        spec.push('+');
    }
    spec.push_str(&key);
    Some(spec)
}

fn note(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.add_css_class("dim-label");
    l
}

pub fn keys_page(ctx: &Rc<Ctx>) -> gtk::Box {
    let p = page();

    // ---------------------------------------------------------------- custom
    p.append(&heading("Custom shortcuts"));
    let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    p.append(&list);
    fill_list(&list, ctx);

    p.append(&heading("Add a shortcut"));
    let keys = gtk::Entry::new();
    keys.set_placeholder_text(Some("Mod+Shift+T"));
    keys.set_width_chars(20);
    let record = gtk::Button::with_label("Record…");
    record.set_tooltip_text(Some("Click, then press the key combination"));
    let keys_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    keys_box.append(&keys);
    keys_box.append(&record);
    p.append(&row("Keys", &keys_box));

    let labels: Vec<&str> = ACTIONS.iter().map(|(l, _, _)| *l).collect();
    let action = gtk::DropDown::from_strings(&labels);
    p.append(&row("Action", &action));

    let command = gtk::Entry::new();
    command.set_placeholder_text(Some("e.g. firefox"));
    command.set_width_chars(26);
    let command_row = row("Command", &command);
    p.append(&command_row);

    let ws = gtk::SpinButton::with_range(1.0, 9.0, 1.0);
    let ws_row = row("Workspace", &ws);
    ws_row.set_visible(false);
    p.append(&ws_row);

    {
        let (command_row, ws_row) = (command_row.clone(), ws_row.clone());
        action.connect_selected_notify(move |d| {
            let arg = ACTIONS.get(d.selected() as usize).map_or(Arg::None, |a| a.2);
            command_row.set_visible(arg == Arg::Command);
            ws_row.set_visible(arg == Arg::Workspace);
        });
    }

    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);

    // Warn as soon as the typed combination clashes with something.
    {
        let (ctx, status) = (ctx.clone(), status.clone());
        keys.connect_changed(move |e| status.set_text(&clash_text(&ctx, &e.text())));
    }

    // Recording: while armed, the button eats the next non-modifier key press.
    let recording = Rc::new(std::cell::Cell::new(false));
    {
        let recording = recording.clone();
        record.connect_clicked(move |b| {
            recording.set(true);
            b.set_label("Press keys…");
            b.grab_focus();
        });
    }
    {
        let keyc = gtk::EventControllerKey::new();
        keyc.set_propagation_phase(gtk::PropagationPhase::Capture);
        let (recording, keys, btn, ctx) = (recording.clone(), keys.clone(), record.clone(), ctx.clone());
        keyc.connect_key_pressed(move |_, keyval, keycode, state| {
            if !recording.get() {
                return glib::Propagation::Proceed;
            }
            let plain = !state.intersects(
                gdk::ModifierType::SUPER_MASK | gdk::ModifierType::ALT_MASK | gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
            );
            if keyval == gdk::Key::Escape && plain {
                recording.set(false);
                btn.set_label("Record…");
                return glib::Propagation::Stop;
            }
            let modifier = ctx.cfg.borrow().general.modifier.clone();
            if let Some(spec) = spec_from_event(&btn.display(), keyval, keycode, state, &modifier) {
                recording.set(false);
                btn.set_label("Record…");
                keys.set_text(&spec);
            }
            glib::Propagation::Stop
        });
        record.add_controller(keyc);
    }
    {
        let (recording, btn) = (recording.clone(), record.clone());
        let focus = gtk::EventControllerFocus::new();
        focus.connect_leave(move |_| {
            recording.set(false);
            btn.set_label("Record…");
        });
        record.add_controller(focus);
    }

    let save = gtk::Button::with_label("Save shortcut");
    save.add_css_class("suggested-action");
    save.set_halign(gtk::Align::Start);
    {
        let (ctx, list, status) = (ctx.clone(), list.clone(), status.clone());
        let (keys, action, command, ws) = (keys.clone(), action.clone(), command.clone(), ws.clone());
        save.connect_clicked(move |_| {
            let spec = keys.text().trim().to_string();
            let modifier = ctx.cfg.borrow().general.modifier.clone();
            let Some(new) = combo(&spec, &modifier) else {
                status.set_text("Those keys aren't valid. Use names like Mod+Shift+T, Ctrl+Alt+Delete or Print.");
                return;
            };
            let Some(&(_, name, arg)) = ACTIONS.get(action.selected() as usize) else { return };
            let value = match arg {
                Arg::None => name.to_string(),
                Arg::Workspace => format!("{name}:{}", ws.value_as_int()),
                Arg::Command => {
                    let cmd = command.text().trim().to_string();
                    if cmd.is_empty() {
                        status.set_text("Enter the command to run.");
                        return;
                    }
                    format!("spawn:{cmd}")
                }
            };
            // One entry per key combination, however it was spelled.
            ctx.update(|cfg| {
                cfg.keybinds.retain(|k, _| combo(k, &modifier).as_ref() != Some(&new));
                cfg.keybinds.insert(spec.clone(), value.clone());
            });
            fill_list(&list, &ctx);
            keys.set_text("");
            command.set_text("");
            status.set_text(&format!("Saved {} → {}", pretty(&spec), describe(&value)));
        });
    }
    p.append(&save);
    p.append(&status);
    p.append(&note(
        "Combinations the desktop already uses (such as Mod+Q) act immediately and can't be recorded; type them in the Keys box instead to change what they do. Key names follow xkb: Return, space, Print, F5, XF86AudioRaiseVolume…",
    ));

    // ---------------------------------------------------------------- defaults
    p.append(&heading("Default shortcuts"));
    let m = "Mod";
    let rows = [
        (format!("{m} + Enter"), "Open terminal"),
        (format!("{m} + D"), "Application launcher"),
        (format!("{m} + Q"), "Close window"),
        (format!("{m} + F"), "Fullscreen"),
        (format!("{m} + Shift + Space"), "Float / tile window"),
        (format!("{m} + Space"), "Change tiling layout"),
        (format!("{m} + J / K  ·  {m} + Tab"), "Focus next / previous window"),
        (format!("{m} + Shift + J / K"), "Move window within the layout"),
        (format!("{m} + O"), "Send window to next monitor"),
        (format!("{m} + L"), "Lock screen"),
        (format!("{m} + 1 … 9"), "Go to workspace"),
        (format!("{m} + Shift + 1 … 9"), "Send window to workspace"),
        (format!("{m} + S"), "Open these settings"),
        (format!("{m} + Shift + R"), "Reload configuration"),
        (format!("{m} + Shift + E"), "Log out"),
        ("Ctrl + Alt + F1…F6".to_string(), "Switch virtual terminal"),
    ];
    let grid = gtk::Grid::new();
    grid.set_column_spacing(24);
    grid.set_row_spacing(8);
    for (i, (k, d)) in rows.iter().enumerate() {
        let kl = gtk::Label::new(Some(k));
        kl.set_xalign(0.0);
        kl.add_css_class("monospace");
        let dl = gtk::Label::new(Some(d));
        dl.set_xalign(0.0);
        grid.attach(&kl, 0, i as i32, 1, 1);
        grid.attach(&dl, 1, i as i32, 1, 1);
    }
    p.append(&grid);
    p.append(&note(
        "“Mod” is Super (the Windows key) unless changed in Windows → Shortcut modifier. A custom shortcut on the same keys replaces the default.",
    ));
    p
}

/// What the typed combination would replace, if anything.
fn clash_text(ctx: &Ctx, spec: &str) -> String {
    let spec = spec.trim();
    if spec.is_empty() {
        return String::new();
    }
    let cfg = ctx.cfg.borrow();
    let modifier = &cfg.general.modifier;
    let Some(c) = combo(spec, modifier) else { return String::new() };
    if let Some((k, v)) = cfg.keybinds.iter().find(|(k, _)| combo(k, modifier).as_ref() == Some(&c)) {
        return format!("Replaces your shortcut {} ({}).", pretty(k), describe(v));
    }
    let mut defaults: Vec<(String, String)> = DEFAULT_SPECS.iter().map(|(k, d)| (k.to_string(), d.to_string())).collect();
    for n in 1..=9 {
        defaults.push((format!("Mod+{n}"), format!("Go to workspace {n}")));
        defaults.push((format!("Mod+Shift+{n}"), format!("Send window to workspace {n}")));
    }
    match defaults.iter().find(|(k, _)| combo(k, modifier).as_ref() == Some(&c)) {
        Some((k, d)) => format!("Replaces the default {} ({d}).", pretty(k)),
        None => String::new(),
    }
}

/// Rebuild the list of custom shortcuts.
fn fill_list(list: &gtk::Box, ctx: &Rc<Ctx>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let binds = ctx.cfg.borrow().keybinds.clone();
    if binds.is_empty() {
        list.append(&note("No custom shortcuts yet. Add one below."));
        return;
    }
    let grid = gtk::Grid::new();
    grid.set_column_spacing(24);
    grid.set_row_spacing(6);
    for (i, (k, v)) in binds.iter().enumerate() {
        let kl = gtk::Label::new(Some(&pretty(k)));
        kl.set_xalign(0.0);
        kl.add_css_class("monospace");
        let dl = gtk::Label::new(Some(&describe(v)));
        dl.set_xalign(0.0);
        dl.set_hexpand(true);
        dl.set_ellipsize(gtk::pango::EllipsizeMode::End);
        dl.set_tooltip_text(Some(v));
        let rm = gtk::Button::from_icon_name("user-trash-symbolic");
        rm.set_tooltip_text(Some("Remove this shortcut"));
        rm.add_css_class("flat");
        let (ctx2, list2, key) = (ctx.clone(), list.clone(), k.clone());
        rm.connect_clicked(move |_| {
            ctx2.update(|cfg| {
                cfg.keybinds.remove(&key);
            });
            // Rebuild after this handler returns; the button is about to be destroyed.
            let (ctx3, list3) = (ctx2.clone(), list2.clone());
            glib::idle_add_local_once(move || fill_list(&list3, &ctx3));
        });
        grid.attach(&kl, 0, i as i32, 1, 1);
        grid.attach(&dl, 1, i as i32, 1, 1);
        grid.attach(&rm, 2, i as i32, 1, 1);
    }
    list.append(&grid);
}
