//! Settings → Widgets: which widgets the bar shows, where, in which order, and each widget's own
//! settings (display style, format, units, ...) behind its gear button.

use std::rc::Rc;

use gtk4 as gtk;
use gtk::{gio, glib, prelude::*};
use safora_common::{
    claude,
    config::{clock_pattern, widget_kind, widget_title, Config, WidgetOpts, ALL_WIDGETS, DEFAULT_TRAY, REPEATABLE},
    sys,
};

use crate::{dropdown, heading, page, row, spin, switch, Ctx};

fn section_of(cfg: &Config, name: &str) -> Option<usize> {
    [&cfg.bar.left, &cfg.bar.center, &cfg.bar.right].iter().position(|l| l.iter().any(|w| w == name))
}

fn section_mut(cfg: &mut Config, idx: usize) -> &mut Vec<String> {
    match idx {
        0 => &mut cfg.bar.left,
        1 => &mut cfg.bar.center,
        _ => &mut cfg.bar.right,
    }
}

fn note(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.add_css_class("dim-label");
    l
}

pub fn widgets_page(ctx: &Rc<Ctx>) -> gtk::Box {
    let outer = page();
    outer.append(&heading("Bar widgets"));
    outer.append(&note(
        "Switch widgets on or off, choose which part of the bar they appear in, and reorder them. \
         The gear button opens a widget's own settings: graphs, formats, units and more.",
    ));
    let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    outer.append(&list);
    fill_widget_rows(ctx, &list);

    let add = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for (kind, text) in [("spacer", "Add a spacer"), ("separator", "Add a separator")] {
        let b = gtk::Button::with_label(text);
        let (ctx, list) = (ctx.clone(), list.clone());
        b.connect_clicked(move |_| {
            ctx.update(|c| {
                let name = c.bar.new_instance(kind);
                c.bar.right.push(name);
            });
            refill(&ctx, &list);
        });
        add.append(&b);
    }
    outer.append(&add);
    outer.append(&note("Spacers and separators can be added as often as you like, to group widgets on the bar."));

    // SAFORA_SETTINGS_WIDGET=<name> opens that widget's settings straight away (used by the bar,
    // e.g. the weather widget's "Open settings" button).
    if let Ok(name) = std::env::var("SAFORA_SETTINGS_WIDGET") {
        if has_settings(widget_kind(&name)) {
            let ctx = ctx.clone();
            let opened = std::cell::Cell::new(false);
            outer.connect_map(move |w| {
                if !opened.replace(true) {
                    let (ctx, name, w) = (ctx.clone(), name.clone(), w.clone());
                    glib::idle_add_local_once(move || open_settings(&ctx, &name, &w));
                }
            });
        }
    }
    outer
}

fn refill(ctx: &Rc<Ctx>, list: &gtk::Box) {
    let (ctx, list) = (ctx.clone(), list.clone());
    glib::idle_add_local_once(move || fill_widget_rows(&ctx, &list));
}

/// Kinds that have a settings window.
fn has_settings(kind: &str) -> bool {
    matches!(
        kind,
        "apps" | "clock" | "cpu" | "ram" | "disk" | "temperature" | "netspeed" | "network" | "audio" | "bluetooth" | "battery" | "weather" | "claude" | "tray" | "spacer"
    )
}

fn fill_widget_rows(ctx: &Rc<Ctx>, list: &gtk::Box) {
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    let cfg = ctx.cfg.borrow().clone();
    // Every widget once, plus the extra copies of spacers and separators that are on the bar.
    let mut names: Vec<String> = Vec::new();
    for (kind, _) in ALL_WIDGETS {
        names.push(kind.to_string());
        if REPEATABLE.contains(kind) {
            for n in cfg.bar.left.iter().chain(&cfg.bar.center).chain(&cfg.bar.right) {
                if n.starts_with(&format!("{kind}#")) {
                    names.push(n.clone());
                }
            }
        }
    }
    for name in names {
        let kind = widget_kind(&name).to_string();
        let place = section_of(&cfg, &name);
        let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let l = gtk::Label::new(Some(&widget_title(&name)));
        l.set_xalign(0.0);
        l.set_hexpand(true);
        r.append(&l);

        // Gear button (kept invisible but in place for widgets without settings, so the rows line up).
        let gear = gtk::Button::from_icon_name("preferences-system-symbolic");
        gear.set_tooltip_text(Some(&format!("{} settings", widget_title(&name))));
        if has_settings(&kind) {
            let (ctx, name) = (ctx.clone(), name.clone());
            gear.connect_clicked(move |b| open_settings(&ctx, &name, b));
        } else {
            gear.set_sensitive(false);
            gear.set_opacity(0.0);
        }

        if let Some(sec) = place {
            let side = dropdown(&["Left", "Center", "Right"], sec, {
                let (ctx, list, name) = (ctx.clone(), list.clone(), name.clone());
                move |i| {
                    ctx.update(|c| {
                        c.bar.remove(&name);
                        section_mut(c, i).push(name.clone());
                    });
                    refill(&ctx, &list);
                }
            });
            r.append(&side);
            for (icon, delta) in [("go-up-symbolic", -1i32), ("go-down-symbolic", 1)] {
                let b = gtk::Button::from_icon_name(icon);
                b.set_tooltip_text(Some(if delta < 0 { "Move left / up" } else { "Move right / down" }));
                let (ctx, list, name) = (ctx.clone(), list.clone(), name.clone());
                b.connect_clicked(move |_| {
                    ctx.update(|c| {
                        if let Some(sec) = section_of(c, &name) {
                            let v = section_mut(c, sec);
                            if let Some(pos) = v.iter().position(|w| *w == name) {
                                let to = pos as i32 + delta;
                                if to >= 0 && (to as usize) < v.len() {
                                    v.swap(pos, to as usize);
                                }
                            }
                        }
                    });
                    refill(&ctx, &list);
                });
                r.append(&b);
            }
        }

        // Just before the switch, so the gear buttons form one column.
        r.append(&gear);
        if name.contains('#') {
            // Extra copies are removed rather than switched off.
            let rm = gtk::Button::from_icon_name("user-trash-symbolic");
            rm.set_tooltip_text(Some("Remove"));
            let (ctx, list, name) = (ctx.clone(), list.clone(), name.clone());
            rm.connect_clicked(move |_| {
                ctx.update(|c| {
                    c.bar.remove(&name);
                    c.bar.widget.remove(&name);
                });
                refill(&ctx, &list);
            });
            r.append(&rm);
        } else {
            let always_on = kind == "workspaces";
            let is_claude = kind == "claude";
            let sw = switch(place.is_some(), {
                let (ctx, list, name) = (ctx.clone(), list.clone(), name.clone());
                move |on| {
                    if always_on {
                        return;
                    }
                    ctx.update(|c| {
                        if on {
                            if section_of(c, &name).is_none() {
                                c.bar.right.push(name.clone());
                            }
                        } else {
                            c.bar.remove(&name);
                        }
                    });
                    // The Claude widget needs Claude Code to report to it; set that up unless the
                    // user already has a status line of their own (the gear page offers to replace it).
                    if on && is_claude {
                        let _ = claude::connect(false);
                    }
                    refill(&ctx, &list);
                }
            });
            sw.set_sensitive(!always_on);
            if always_on {
                sw.set_tooltip_text(Some("The workspace switcher is always shown"));
            }
            r.append(&sw);
        }
        list.append(&r);
    }
}

// ------------------------------------------------------------------- settings window

/// Change one widget's settings; entries equal to the defaults are dropped to keep the file tidy.
fn set_opt(ctx: &Ctx, name: &str, f: impl FnOnce(&mut WidgetOpts)) {
    ctx.update(|c| {
        let e = c.bar.widget.entry(name.to_string()).or_default();
        f(e);
        if *e == WidgetOpts::default() {
            c.bar.widget.remove(name);
        }
    });
}

/// A dropdown over `(value, label)` pairs bound to a string field of the widget's settings.
fn choice(
    ctx: &Rc<Ctx>,
    name: &str,
    text: &str,
    items: &[(&'static str, &'static str)],
    current: &str,
    field: fn(&mut WidgetOpts) -> &mut String,
    on_change: impl Fn(&str) + 'static,
) -> gtk::Box {
    let labels: Vec<&str> = items.iter().map(|(_, l)| *l).collect();
    let values: Vec<&'static str> = items.iter().map(|(v, _)| *v).collect();
    let idx = values.iter().position(|v| *v == current).unwrap_or(0);
    let (ctx, name) = (ctx.clone(), name.to_string());
    row(
        text,
        &dropdown(&labels, idx, move |i| {
            let v = values[i.min(values.len() - 1)];
            // The first entry is the default: store it as "".
            let stored = if i == 0 { "" } else { v };
            set_opt(&ctx, &name, |o| *field(o) = stored.to_string());
            on_change(v);
        }),
    )
}

fn number(ctx: &Rc<Ctx>, name: &str, text: &str, min: f64, max: f64, current: u32, default: u32, field: fn(&mut WidgetOpts) -> &mut u32) -> gtk::Box {
    let (ctx, name) = (ctx.clone(), name.to_string());
    let shown = if current == 0 { default } else { current };
    row(
        text,
        &spin(min, max, 1.0, shown as f64, move |v| {
            let v = v as u32;
            set_opt(&ctx, &name, |o| *field(o) = if v == default { 0 } else { v });
        }),
    )
}

const DISPLAYS: &[(&str, &str)] = &[("text", "Text"), ("graph", "Graph"), ("meter", "Meter"), ("icon", "Icon and value")];
/// A meter needs a known maximum, which network traffic does not have.
const NET_DISPLAYS: &[(&str, &str)] = &[("text", "Text"), ("graph", "Graph"), ("icon", "Icon and value")];

/// Store the tray's app list. An empty tray is saved as `["none"]`, because an empty list means
/// "the default apps".
fn save_tray(ctx: &Ctx, name: &str, apps: &[String]) {
    let v = if apps.is_empty() { vec!["none".to_string()] } else { apps.to_vec() };
    set_opt(ctx, name, |o| o.apps = v);
}

fn open_settings(ctx: &Rc<Ctx>, name: &str, from: &impl IsA<gtk::Widget>) {
    let kind = widget_kind(name).to_string();
    let o = ctx.cfg.borrow().bar.opts(name);
    let win = gtk::Window::builder()
        .title(format!("{} settings", widget_title(name)))
        .modal(true)
        .default_width(460)
        .resizable(false)
        .build();
    if let Some(root) = from.root().and_downcast::<gtk::Window>() {
        win.set_transient_for(Some(&root));
    }
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_top(18);
    body.set_margin_bottom(18);
    body.set_margin_start(20);
    body.set_margin_end(20);

    match kind.as_str() {
        "cpu" | "ram" | "disk" | "temperature" | "netspeed" => metric_settings(ctx, name, &kind, &o, &body),
        "clock" => clock_settings(ctx, name, &o, &body),
        "apps" => {
            body.append(&choice(
                ctx,
                name,
                "Show",
                &[("", "Programs on all workspaces"), ("workspace", "Only programs on the current workspace")],
                &o.source,
                |o| &mut o.source,
                |_| {},
            ));
            body.append(&note("With all workspaces, programs on other workspaces are shown dimmed; clicking one switches to its workspace."));
        }
        "network" => {
            body.append(&choice(ctx, name, "Show", &[("", "Icon and connection name"), ("icon", "Icon only")], &o.format, |o| &mut o.format, |_| {}));
        }
        "audio" => {
            body.append(&choice(ctx, name, "Show", &[("", "Icon and volume"), ("icon", "Icon only")], &o.format, |o| &mut o.format, |_| {}));
            body.append(&note("Scroll over the widget to change the volume."));
        }
        "bluetooth" => {
            body.append(&choice(ctx, name, "Show", &[("", "Icon and number of connected devices"), ("icon", "Icon only")], &o.format, |o| &mut o.format, |_| {}));
        }
        "battery" => {
            body.append(&choice(
                ctx,
                name,
                "Show",
                &[("", "Icon and percentage"), ("percent-time", "Percentage and time left"), ("icon", "Icon only")],
                &o.format,
                |o| &mut o.format,
                |_| {},
            ));
            body.append(&number(ctx, name, "Update every (seconds)", 5.0, 300.0, o.interval, 20, |o| &mut o.interval));
            body.append(&note("The battery widget hides itself on computers without a battery."));
        }
        "weather" => weather_settings(ctx, name, &o, &body),
        "claude" => claude_settings(ctx, name, &o, &body),
        "tray" => tray_settings(ctx, name, &o, &body),
        "spacer" => {
            body.append(&number(ctx, name, "Width (pixels)", 0.0, 400.0, o.width, 16, |o| &mut o.width));
        }
        _ => {}
    }

    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::End);
    close.add_css_class("suggested-action");
    let w = win.clone();
    close.connect_clicked(move |_| w.close());
    body.append(&close);
    win.set_child(Some(&body));
    win.present();
}

fn metric_settings(ctx: &Rc<Ctx>, name: &str, kind: &str, o: &WidgetOpts, body: &gtk::Box) {
    let displays = if kind == "netspeed" { NET_DISPLAYS } else { DISPLAYS };
    let current = if o.display.is_empty() { "text" } else { o.display.as_str() };

    // Width only matters for graphs and meters; the name only when it is not replaced by an icon.
    let width = number(ctx, name, "Graph / meter width (pixels)", 20.0, 240.0, o.width, if current == "meter" { 40 } else { 48 }, |o| &mut o.width);
    let hide = row(
        "Show the name (CPU, RAM, ...)",
        &switch(!o.hide_label, {
            let (ctx, name) = (ctx.clone(), name.to_string());
            move |on| set_opt(&ctx, &name, |o| o.hide_label = !on)
        }),
    );
    let sync = {
        let (width, hide) = (width.clone(), hide.clone());
        move |d: &str| {
            width.set_visible(d == "graph" || d == "meter");
            hide.set_visible(d != "icon");
        }
    };
    sync(current);
    body.append(&choice(ctx, name, "Display", displays, current, |o| &mut o.display, sync));
    body.append(&width);
    body.append(&hide);

    match kind {
        "ram" => body.append(&choice(
            ctx,
            name,
            "Format",
            &[("", "Used (2.1 GB)"), ("percent", "Percentage used"), ("used-total", "Used of total (2.1 GB / 16 GB)"), ("free", "Free")],
            &o.format,
            |o| &mut o.format,
            |_| {},
        )),
        "disk" => {
            body.append(&choice(
                ctx,
                name,
                "Format",
                &[("", "Percentage used"), ("used", "Used"), ("free", "Free"), ("used-total", "Used of total")],
                &o.format,
                |o| &mut o.format,
                |_| {},
            ));
            let mounts = mount_points();
            let labels: Vec<&str> = mounts.iter().map(String::as_str).collect();
            let cur = if o.source.is_empty() { "/" } else { o.source.as_str() };
            let idx = mounts.iter().position(|m| m == cur).unwrap_or(0);
            let (c, n, m2) = (ctx.clone(), name.to_string(), mounts.clone());
            body.append(&row(
                "Drive (mount point)",
                &dropdown(&labels, idx, move |i| {
                    let v = m2.get(i).cloned().unwrap_or_default();
                    set_opt(&c, &n, |o| o.source = if v == "/" { String::new() } else { v });
                }),
            ));
        }
        "temperature" => {
            body.append(&choice(ctx, name, "Units", &[("", "Celsius (°C)"), ("imperial", "Fahrenheit (°F)")], &o.units, |o| &mut o.units, |_| {}));
        }
        "netspeed" => {
            body.append(&choice(
                ctx,
                name,
                "Show",
                &[("", "Download and upload"), ("down", "Download only"), ("up", "Upload only")],
                &o.format,
                |o| &mut o.format,
                |_| {},
            ));
            body.append(&choice(ctx, name, "Units", &[("", "Bytes per second (MB/s)"), ("bits", "Bits per second (Mb/s)")], &o.units, |o| &mut o.units, |_| {}));
            let mut ifaces = vec!["All connections".to_string()];
            ifaces.extend(sys::net_interfaces());
            let labels: Vec<&str> = ifaces.iter().map(String::as_str).collect();
            let idx = ifaces.iter().position(|i| *i == o.source).unwrap_or(0);
            let (c, n, i2) = (ctx.clone(), name.to_string(), ifaces.clone());
            body.append(&row(
                "Connection",
                &dropdown(&labels, idx, move |i| {
                    let v = if i == 0 { String::new() } else { i2.get(i).cloned().unwrap_or_default() };
                    set_opt(&c, &n, |o| o.source = v);
                }),
            ));
        }
        _ => {}
    }
    let default_secs = match kind {
        "cpu" | "netspeed" => 2,
        "ram" | "temperature" => 3,
        _ => 30,
    };
    body.append(&number(ctx, name, "Update every (seconds)", 1.0, 300.0, o.interval, default_secs, |o| &mut o.interval));
}

/// Mounted drives worth measuring (no pseudo or temporary file systems).
fn mount_points() -> Vec<String> {
    let out = std::process::Command::new("df")
        .args(["--output=target", "-x", "tmpfs", "-x", "devtmpfs", "-x", "efivarfs", "-x", "overlay", "-x", "squashfs"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let mut v: Vec<String> = out.lines().skip(1).map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    v.sort();
    v.dedup();
    if let Some(i) = v.iter().position(|m| m == "/") {
        v.remove(i);
    }
    v.insert(0, "/".into());
    v
}

const CLOCK_FORMATS: &[(&str, &str)] = &[
    ("", "Day, date and time"),
    ("time", "Time only"),
    ("seconds", "Time with seconds"),
    ("date", "Date only"),
    ("iso", "2026-11-02 14:30"),
    ("custom", "Custom…"),
];

fn clock_settings(ctx: &Rc<Ctx>, name: &str, o: &WidgetOpts, body: &gtk::Box) {
    let is_custom = o.format.contains('%');
    let current = if is_custom { "custom" } else { o.format.as_str() };
    let preview = gtk::Label::new(None);
    preview.set_xalign(0.0);
    preview.add_css_class("title-4");
    let update_preview = {
        let (ctx, name, preview) = (ctx.clone(), name.to_string(), preview.clone());
        Rc::new(move || {
            let cfg = ctx.cfg.borrow();
            let pattern = clock_pattern(&cfg.bar.opts(&name).format, cfg.bar.clock_24h);
            let text = glib::DateTime::now_local().ok().and_then(|n| n.format(&pattern).ok()).map(|s| s.to_string()).unwrap_or_default();
            preview.set_text(&format!("Preview:  {text}"));
        })
    };

    let custom = gtk::Entry::new();
    custom.set_placeholder_text(Some("%a %d %b  %H:%M"));
    custom.set_hexpand(true);
    if is_custom {
        custom.set_text(&o.format);
    }
    let custom_row = row("Pattern", &custom);
    let custom_help = note("Pattern codes: %H hour (24 h), %I hour (12 h), %M minutes, %S seconds, %p AM/PM, %a weekday, %d day, %b month, %Y year.");
    custom_row.set_visible(is_custom);
    custom_help.set_visible(is_custom);
    {
        let (ctx, name, up) = (ctx.clone(), name.to_string(), update_preview.clone());
        custom.connect_changed(move |e| {
            let t = e.text().to_string();
            // Only store real patterns, so a half-typed entry never blanks the clock.
            if t.contains('%') {
                set_opt(&ctx, &name, |o| o.format = t);
                up();
            }
        });
    }
    let fmt = {
        let labels: Vec<&str> = CLOCK_FORMATS.iter().map(|(_, l)| *l).collect();
        let idx = CLOCK_FORMATS.iter().position(|(v, _)| *v == current).unwrap_or(0);
        let (ctx, name, up, custom, custom_row, custom_help) =
            (ctx.clone(), name.to_string(), update_preview.clone(), custom.clone(), custom_row.clone(), custom_help.clone());
        row(
            "Format",
            &dropdown(&labels, idx, move |i| {
                let v = CLOCK_FORMATS[i.min(CLOCK_FORMATS.len() - 1)].0;
                let custom_on = v == "custom";
                custom_row.set_visible(custom_on);
                custom_help.set_visible(custom_on);
                if custom_on {
                    if custom.text().is_empty() {
                        custom.set_text("%a %d %b  %H:%M");
                    } else {
                        let t = custom.text().to_string();
                        set_opt(&ctx, &name, |o| o.format = t);
                    }
                } else {
                    set_opt(&ctx, &name, |o| o.format = v.to_string());
                }
                up();
            }),
        )
    };
    let h24 = {
        let on24 = ctx.cfg.borrow().bar.clock_24h;
        let (ctx, up) = (ctx.clone(), update_preview.clone());
        row(
            "24-hour clock",
            &switch(on24, move |on| {
                ctx.update(|c| c.bar.clock_24h = on);
                up();
            }),
        )
    };
    body.append(&preview);
    body.append(&fmt);
    body.append(&custom_row);
    body.append(&custom_help);
    body.append(&h24);
    update_preview();
}

fn weather_settings(ctx: &Rc<Ctx>, name: &str, o: &WidgetOpts, body: &gtk::Box) {
    let place = gtk::Entry::new();
    place.set_text(&o.location);
    place.set_placeholder_text(Some("e.g. Berlin, or Paris, France"));
    place.set_hexpand(true);
    let apply = gtk::Button::with_label("Use");
    let status = note(if o.location.is_empty() { "No place set yet." } else { "" });
    let save = {
        let (ctx, name, place, status) = (ctx.clone(), name.to_string(), place.clone(), status.clone());
        move || {
            let t = place.text().trim().to_string();
            set_opt(&ctx, &name, |o| o.location = t.clone());
            status.set_text(&if t.is_empty() { "No place set yet.".to_string() } else { format!("The bar now shows the weather for “{t}”.") });
        }
    };
    let s2 = save.clone();
    apply.connect_clicked(move |_| s2());
    place.connect_activate(move |_| save());
    let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let l = gtk::Label::new(Some("Place"));
    r.append(&l);
    r.append(&place);
    r.append(&apply);
    body.append(&r);
    body.append(&status);
    body.append(&choice(ctx, name, "Units", &[("", "Celsius, km/h"), ("imperial", "Fahrenheit, mph")], &o.units, |o| &mut o.units, |_| {}));
    body.append(&choice(
        ctx,
        name,
        "Show",
        &[("", "Icon and temperature"), ("temp-condition", "Temperature and conditions"), ("icon", "Icon only")],
        &o.format,
        |o| &mut o.format,
        |_| {},
    ));
    body.append(&number(ctx, name, "Update every (minutes)", 10.0, 240.0, o.interval, 30, |o| &mut o.interval));
    body.append(&note(
        "Privacy: the forecast comes from open-meteo.com (no account or key). Only the place you type is sent, \
         and the service sees your IP address like any website. Nothing is contacted while this widget is off.",
    ));
}

fn claude_settings(ctx: &Rc<Ctx>, name: &str, o: &WidgetOpts, body: &gtk::Box) {
    body.append(&note(
        "Shows how much of your Claude plan's 5-hour session and weekly limits you have used. \
         Click the widget for both limits, their reset times and a Refresh button.",
    ));
    body.append(&choice(
        ctx,
        name,
        "Show",
        &[("", "5-hour and weekly usage"), ("five-hour", "5-hour usage only"), ("week", "Weekly usage only")],
        &o.format,
        |o| &mut o.format,
        |_| {},
    ));
    let current = if o.display.is_empty() { "text" } else { o.display.as_str() };
    let width = number(ctx, name, "Meter width (pixels)", 20.0, 240.0, o.width, 36, |o| &mut o.width);
    width.set_visible(current == "meter");
    let w2 = width.clone();
    body.append(&choice(ctx, name, "Display", &[("text", "Percentages"), ("meter", "Meters and percentages")], current, |o| &mut o.display, move |d| {
        w2.set_visible(d == "meter")
    }));
    body.append(&width);
    body.append(&row(
        "Show the ✻ symbol",
        &switch(!o.hide_label, {
            let (ctx, name) = (ctx.clone(), name.to_string());
            move |on| set_opt(&ctx, &name, |o| o.hide_label = !on)
        }),
    ));
    body.append(&number(ctx, name, "Check every (seconds)", 2.0, 300.0, o.interval, 10, |o| &mut o.interval));

    // Connection to Claude Code (its status-line setting).
    body.append(&heading("Claude Code"));
    let status = note("");
    let button = gtk::Button::new();
    button.set_halign(gtk::Align::Start);
    let show_state = {
        let (status, button) = (status.clone(), button.clone());
        Rc::new(move |err: Option<String>| {
            let path = claude::settings_path().display().to_string();
            let (text, btn) = match claude::hook() {
                claude::Hook::Connected => (format!("Connected: Claude Code reports its usage to the bar ({path})."), Some("Disconnect")),
                claude::Hook::Missing => ("Not connected: Claude Code doesn't report its usage to the bar yet.".to_string(), Some("Connect Claude Code")),
                claude::Hook::Other(cmd) => (
                    format!("Claude Code already runs its own status line ({cmd}). Connecting replaces it with Safora's."),
                    Some("Replace and connect"),
                ),
                claude::Hook::Unreadable => (format!("{path} is not valid JSON, so Safora leaves it alone. Fix it, then come back here."), None),
            };
            status.set_text(&match err {
                Some(e) => format!("{text}\n{e}"),
                None => text,
            });
            button.set_visible(btn.is_some());
            button.set_label(btn.unwrap_or_default());
        })
    };
    show_state(None);
    {
        let show_state = show_state.clone();
        button.connect_clicked(move |_| {
            let r = match claude::hook() {
                claude::Hook::Connected => claude::disconnect(),
                claude::Hook::Other(_) => claude::connect(true),
                _ => claude::connect(false),
            };
            show_state(r.err());
        });
    }
    body.append(&status);
    body.append(&button);
    body.append(&note(
        "Privacy: no API and no network access. Claude Code passes its own usage figures to its status-line \
         command (saforactl claude-status), which keeps them in memory under /run/user until you log out. \
         The widget only updates while claude is running in a terminal; otherwise it shows the last values, dimmed.",
    ));
}

fn tray_settings(ctx: &Rc<Ctx>, name: &str, o: &WidgetOpts, body: &gtk::Box) {
    body.append(&note("Icons for starting your favourite apps with one click."));
    let all: Vec<gio::AppInfo> = {
        let mut v: Vec<gio::AppInfo> = gio::AppInfo::all().into_iter().filter(|a| a.should_show() && a.id().is_some()).collect();
        v.sort_by_key(|a| a.display_name().to_lowercase());
        v
    };
    let current: Vec<String> = if o.apps.is_empty() {
        DEFAULT_TRAY.iter().map(|s| s.to_string()).collect()
    } else {
        o.apps.iter().filter(|a| *a != "none").cloned().collect()
    };
    let apps = Rc::new(std::cell::RefCell::new(current));
    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    body.append(&list);

    fn fill(ctx: &Rc<Ctx>, name: &str, apps: &Rc<std::cell::RefCell<Vec<String>>>, all: &Rc<Vec<gio::AppInfo>>, list: &gtk::Box) {
        while let Some(c) = list.first_child() {
            list.remove(&c);
        }
        let ids = apps.borrow().clone();
        if ids.is_empty() {
            list.append(&note("No apps: the tray is empty."));
        }
        for (i, id) in ids.iter().enumerate() {
            let info = all.iter().find(|a| a.id().is_some_and(|x| x == id.as_str()));
            let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let img = match info.and_then(|a| a.icon()) {
                Some(icon) => gtk::Image::from_gicon(&icon),
                None => gtk::Image::from_icon_name("application-x-executable-symbolic"),
            };
            r.append(&img);
            let l = gtk::Label::new(Some(&info.map(|a| a.display_name().to_string()).unwrap_or_else(|| format!("{id} (not installed)"))));
            l.set_xalign(0.0);
            l.set_hexpand(true);
            r.append(&l);
            for (icon, delta) in [("go-up-symbolic", -1i32), ("go-down-symbolic", 1)] {
                let b = gtk::Button::from_icon_name(icon);
                let (ctx, name, apps, all, list) = (ctx.clone(), name.to_string(), apps.clone(), all.clone(), list.clone());
                b.connect_clicked(move |_| {
                    let to = i as i32 + delta;
                    if to >= 0 && (to as usize) < apps.borrow().len() {
                        apps.borrow_mut().swap(i, to as usize);
                        let v = apps.borrow().clone();
                        save_tray(&ctx, &name, &v);
                        fill(&ctx, &name, &apps, &all, &list);
                    }
                });
                r.append(&b);
            }
            let rm = gtk::Button::from_icon_name("user-trash-symbolic");
            rm.set_tooltip_text(Some("Remove from the tray"));
            {
                let (ctx, name, apps, all, list) = (ctx.clone(), name.to_string(), apps.clone(), all.clone(), list.clone());
                rm.connect_clicked(move |_| {
                    apps.borrow_mut().remove(i);
                    let v = apps.borrow().clone();
                    save_tray(&ctx, &name, &v);
                    fill(&ctx, &name, &apps, &all, &list);
                });
            }
            r.append(&rm);
            list.append(&r);
        }
    }
    let all = Rc::new(all);
    fill(ctx, name, &apps, &all, &list);

    let names: Vec<String> = all.iter().map(|a| a.display_name().to_string()).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let pick = gtk::DropDown::from_strings(&refs);
    pick.set_hexpand(true);
    pick.set_enable_search(true);
    pick.set_expression(Some(gtk::PropertyExpression::new(gtk::StringObject::static_type(), gtk::Expression::NONE, "string")));
    pick.set_search_match_mode(gtk::StringFilterMatchMode::Substring);
    let add = gtk::Button::with_label("Add");
    {
        let (ctx, name, apps, all, list, pick) = (ctx.clone(), name.to_string(), apps.clone(), all.clone(), list.clone(), pick.clone());
        add.connect_clicked(move |_| {
            let Some(id) = all.get(pick.selected() as usize).and_then(|a| a.id()).map(|s| s.to_string()) else { return };
            if !apps.borrow().contains(&id) {
                apps.borrow_mut().push(id);
                let v = apps.borrow().clone();
                save_tray(&ctx, &name, &v);
                fill(&ctx, &name, &apps, &all, &list);
            }
        });
    }
    let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    r.append(&pick);
    r.append(&add);
    body.append(&r);
}
