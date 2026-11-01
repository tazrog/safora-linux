//! Bar widgets. Each builder returns a GTK widget; `build` maps a config name to a builder.

use std::{cell::{Cell, RefCell}, rc::Rc, time::Duration};

use gtk4 as gtk;
use gtk::{gio, glib, pango, prelude::*};
use sekuro_common::ipc::{self, Request, State};

use crate::{sys, Ctx};

pub fn build(name: &str, ctx: &Rc<Ctx>) -> Option<gtk::Widget> {
    Some(match name {
        "workspaces" => workspaces(ctx),
        "apps" => apps(ctx),
        "clock" => clock(ctx.config.bar.clock_24h),
        "cpu" => cpu(),
        "ram" => ram(),
        "disk" => disk(),
        "network" => network(),
        "bluetooth" => bluetooth(),
        "audio" => audio(),
        "power" => power(),
        other => {
            eprintln!("sekuro-bar: unknown widget `{other}`");
            return None;
        }
    })
}

// ---------------------------------------------------------------------- utils

/// Run `work` on a worker thread every `ms` milliseconds and hand the result to `apply` on the UI
/// thread. Stops automatically once `anchor` has been destroyed (e.g. after a bar rebuild).
fn poll<W, T>(anchor: &W, ms: u64, work: impl Fn() -> T + Clone + Send + 'static, apply: impl Fn(T) + 'static)
where
    W: IsA<gtk::Widget>,
    T: Send + 'static,
{
    let weak = anchor.downgrade();
    let apply = Rc::new(apply);
    let tick = move || -> bool {
        if weak.upgrade().is_none() {
            return false;
        }
        let (work, apply, weak) = (work.clone(), apply.clone(), weak.clone());
        glib::spawn_future_local(async move {
            if let Ok(value) = gio::spawn_blocking(move || work()).await {
                if weak.upgrade().is_some() {
                    apply(value);
                }
            }
        });
        true
    };
    tick();
    glib::timeout_add_local(Duration::from_millis(ms), move || {
        if tick() { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
    });
}

/// Run `work` once in the background.
fn once<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, apply: impl FnOnce(T) + 'static) {
    glib::spawn_future_local(async move {
        if let Ok(v) = gio::spawn_blocking(work).await {
            apply(v);
        }
    });
}

fn label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l
}

fn set_level(w: &impl IsA<gtk::Widget>, level: &str) {
    for c in ["warn", "crit"] {
        w.remove_css_class(c);
    }
    if !level.is_empty() {
        w.add_css_class(level);
    }
}

fn icon_label(icon: &str) -> (gtk::Box, gtk::Image, gtk::Label) {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    let i = gtk::Image::from_icon_name(icon);
    let l = gtk::Label::new(None);
    b.append(&i);
    b.append(&l);
    (b, i, l)
}

fn menu_button(child: &impl IsA<gtk::Widget>, popover: &gtk::Popover) -> gtk::MenuButton {
    let b = gtk::MenuButton::new();
    b.set_child(Some(child));
    b.set_popover(Some(popover));
    b.set_has_frame(false);
    b.set_direction(gtk::ArrowType::None);
    b
}

fn popover_box() -> (gtk::Popover, gtk::Box) {
    let pop = gtk::Popover::new();
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    b.set_margin_top(2);
    b.set_margin_bottom(2);
    b.set_margin_start(2);
    b.set_margin_end(2);
    pop.set_child(Some(&b));
    (pop, b)
}

fn title(text: &str) -> gtk::Label {
    let l = label(text);
    l.add_css_class("title");
    l
}

fn clear(b: &impl IsA<gtk::Widget>) {
    while let Some(c) = b.first_child() {
        c.unparent();
    }
}

// ----------------------------------------------------------------- workspaces

fn workspaces(ctx: &Rc<Ctx>) -> gtk::Widget {
    let count = ctx.config.general.workspaces;
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    let buttons: Vec<gtk::Button> = (0..count)
        .map(|i| {
            let b = gtk::Button::with_label(&(i + 1).to_string());
            b.add_css_class("workspace");
            b.connect_clicked(move |_| {
                let _ = ipc::send(&Request::Workspace { n: i + 1 });
            });
            row.append(&b);
            b
        })
        .collect();

    // Scrolling over the switcher walks through the workspaces.
    let active = Rc::new(Cell::new(0usize));
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    {
        let active = active.clone();
        scroll.connect_scroll(move |_, _, dy| {
            let cur = active.get() as i32;
            let next = (cur + if dy > 0.0 { 1 } else { -1 }).clamp(0, count as i32 - 1);
            if next != cur {
                let _ = ipc::send(&Request::Workspace { n: next as usize + 1 });
            }
            glib::Propagation::Stop
        });
    }
    row.add_controller(scroll);

    ctx.listeners.borrow_mut().push(Box::new(move |s: &State| {
        active.set(s.active);
        for (i, b) in buttons.iter().enumerate() {
            let occupied = s.workspaces.get(i).is_some_and(|w| w.windows > 0);
            b.set_css_classes(&["workspace"]);
            if i == s.active {
                b.add_css_class("active");
            } else if occupied {
                b.add_css_class("occupied");
            }
        }
    }));
    row.upcast()
}

// ----------------------------------------------------------------------- apps

fn app_icon(app_id: &str) -> gtk::Image {
    let wanted = app_id.to_lowercase();
    let icon = gio::AppInfo::all()
        .into_iter()
        .find(|a| a.id().is_some_and(|id| id.trim_end_matches(".desktop").to_lowercase() == wanted))
        .and_then(|info| info.icon());
    match icon {
        Some(g) => gtk::Image::from_gicon(&g),
        None => gtk::Image::from_icon_name(if app_id.is_empty() { "application-x-executable-symbolic" } else { &wanted }),
    }
}

fn apps(ctx: &Rc<Ctx>) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    let row2 = row.clone();
    ctx.listeners.borrow_mut().push(Box::new(move |s: &State| {
        clear(&row2);
        for w in &s.windows {
            let b = gtk::Button::new();
            let inner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            inner.append(&app_icon(&w.app_id));
            let text = if w.title.is_empty() { &w.app_id } else { &w.title };
            let l = gtk::Label::new(Some(text));
            l.set_ellipsize(pango::EllipsizeMode::End);
            l.set_max_width_chars(18);
            inner.append(&l);
            b.set_child(Some(&inner));
            b.add_css_class("app");
            if w.focused {
                b.add_css_class("focused");
            }
            if w.workspace != s.active {
                b.add_css_class("other-ws");
            }
            b.set_tooltip_text(Some(&format!("{} (workspace {})", w.title, w.workspace + 1)));
            let id = w.id;
            b.connect_clicked(move |_| {
                let _ = ipc::send(&Request::Focus { id });
            });
            // Middle click closes the window.
            let mid = gtk::GestureClick::new();
            mid.set_button(2);
            mid.connect_pressed(move |_, _, _, _| {
                let _ = ipc::send(&Request::Close { id });
            });
            b.add_controller(mid);
            row2.append(&b);
        }
    }));
    row.upcast()
}

// ---------------------------------------------------------------------- clock

fn clock(h24: bool) -> gtk::Widget {
    let l = gtk::Label::new(None);
    let update = {
        let l = l.clone();
        move || {
            if let Ok(now) = glib::DateTime::now_local() {
                if let Ok(s) = now.format(if h24 { "%a %d %b   %H:%M" } else { "%a %d %b   %I:%M %p" }) {
                    l.set_text(&s);
                }
            }
        }
    };
    update();
    let weak = l.downgrade();
    // The label only shows minutes, so wake up twice a minute instead of every second.
    glib::timeout_add_seconds_local(30, move || {
        if weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        update();
        glib::ControlFlow::Continue
    });

    let (pop, body) = popover_box();
    body.append(&gtk::Calendar::new());
    menu_button(&l, &pop).upcast()
}

// ------------------------------------------------------------ cpu / ram / disk

fn cpu() -> gtk::Widget {
    let l = gtk::Label::new(Some("CPU –"));
    l.add_css_class("metric");
    let prev = Rc::new(Cell::new(None::<(u64, u64)>));
    let l2 = l.clone();
    poll(&l, 2000, sys::read_cpu, move |now| {
        let Some(now) = now else { return };
        if let Some(p) = prev.get() {
            let pct = sys::cpu_percent(p, now);
            l2.set_text(&format!("CPU {pct:.0}%"));
            set_level(&l2, if pct > 95.0 { "crit" } else if pct > 80.0 { "warn" } else { "" });
        }
        prev.set(Some(now));
    });
    l.upcast()
}

fn ram() -> gtk::Widget {
    let l = gtk::Label::new(Some("RAM –"));
    l.add_css_class("metric");
    let l2 = l.clone();
    poll(&l, 3000, sys::read_mem, move |m| {
        let Some((total, avail)) = m else { return };
        let used = total.saturating_sub(avail);
        let pct = used as f64 * 100.0 / total.max(1) as f64;
        l2.set_text(&format!("RAM {}", sys::human_bytes(used * 1024)));
        l2.set_tooltip_text(Some(&format!(
            "{} used of {} ({pct:.0}%)",
            sys::human_bytes(used * 1024),
            sys::human_bytes(total * 1024)
        )));
        set_level(&l2, if pct > 95.0 { "crit" } else if pct > 85.0 { "warn" } else { "" });
    });
    l.upcast()
}

fn disk() -> gtk::Widget {
    let l = gtk::Label::new(Some("DISK –"));
    l.add_css_class("metric");
    let l2 = l.clone();
    poll(&l, 30_000, || sys::read_disk("/"), move |d| {
        let Some((used, total)) = d else { return };
        let pct = used as f64 * 100.0 / total.max(1) as f64;
        l2.set_text(&format!("DISK {pct:.0}%"));
        l2.set_tooltip_text(Some(&format!("{} used of {}", sys::human_bytes(used), sys::human_bytes(total))));
        set_level(&l2, if pct > 95.0 { "crit" } else if pct > 85.0 { "warn" } else { "" });
    });
    l.upcast()
}

// -------------------------------------------------------------------- network

fn network() -> gtk::Widget {
    let (content, icon, text) = icon_label("network-offline-symbolic");
    let (pop, body) = popover_box();
    body.set_width_request(260);

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let t = title("Wi-Fi");
    t.set_hexpand(true);
    let switch = gtk::Switch::new();
    head.append(&t);
    head.append(&switch);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    let scroller = gtk::ScrolledWindow::builder().child(&list).min_content_height(120).max_content_height(260).propagate_natural_height(true).build();
    let pw_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let pw = gtk::PasswordEntry::new();
    pw.set_hexpand(true);
    pw.set_show_peek_icon(true);
    pw.set_placeholder_text(Some("Password"));
    let join = gtk::Button::with_label("Join");
    pw_row.append(&pw);
    pw_row.append(&join);
    pw_row.set_visible(false);
    let advanced = gtk::Button::with_label("Network settings…");
    body.append(&head);
    body.append(&scroller);
    body.append(&pw_row);
    body.append(&advanced);
    advanced.connect_clicked({
        let pop = pop.clone();
        move |_| {
            pop.popdown();
            sys::spawn("nm-connection-editor", &[]);
        }
    });

    let selected = Rc::new(RefCell::new(String::new()));
    let updating = Rc::new(Cell::new(false));
    switch.connect_state_set({
        let updating = updating.clone();
        move |_, on| {
            if !updating.get() {
                sys::spawn("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]);
            }
            glib::Propagation::Proceed
        }
    });
    join.connect_clicked({
        let (selected, pw, pw_row, pop) = (selected.clone(), pw.clone(), pw_row.clone(), pop.clone());
        move |_| {
            let ssid = selected.borrow().clone();
            let pass = pw.text().to_string();
            if !ssid.is_empty() && !pass.is_empty() {
                once(move || sys::run("nmcli", &["device", "wifi", "connect", &ssid, "password", &pass]), |_| {});
            }
            pw.set_text("");
            pw_row.set_visible(false);
            pop.popdown();
        }
    });

    let refresh = {
        let (list, switch, updating, selected, pw_row, pop) =
            (list.clone(), switch.clone(), updating.clone(), selected.clone(), pw_row.clone(), pop.clone());
        move || {
            let (list, switch, updating, selected, pw_row, pop) =
                (list.clone(), switch.clone(), updating.clone(), selected.clone(), pw_row.clone(), pop.clone());
            once(
                || (sys::wifi_enabled(), sys::read_wifi(true), sys::saved_connections()),
                move |(enabled, aps, saved)| {
                    updating.set(true);
                    switch.set_active(enabled);
                    updating.set(false);
                    clear(&list);
                    if !enabled {
                        list.append(&label("Wi-Fi is turned off"));
                    } else if aps.is_empty() {
                        list.append(&label("No networks found"));
                    }
                    for ap in aps {
                        let b = gtk::Button::new();
                        let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                        let name = label(&ap.ssid);
                        name.set_hexpand(true);
                        if ap.in_use {
                            name.add_css_class("title");
                        }
                        r.append(&name);
                        if ap.secured {
                            r.append(&gtk::Image::from_icon_name("system-lock-screen-symbolic"));
                        }
                        r.append(&label(&format!("{}%", ap.signal)));
                        b.set_child(Some(&r));
                        let known = saved.contains(&ap.ssid);
                        let (selected, pw_row, pop) = (selected.clone(), pw_row.clone(), pop.clone());
                        b.connect_clicked(move |_| {
                            let ssid = ap.ssid.clone();
                            if ap.in_use {
                                return;
                            }
                            if ap.secured && !known {
                                *selected.borrow_mut() = ssid;
                                pw_row.set_visible(true);
                            } else {
                                pop.popdown();
                                once(move || sys::run("nmcli", &["device", "wifi", "connect", &ssid]), |_| {});
                            }
                        });
                        list.append(&b);
                    }
                },
            );
        }
    };
    pop.connect_show({
        let refresh = refresh.clone();
        move |_| refresh()
    });

    poll(&content, 5000, sys::read_net, move |net| {
        let (name, text_s) = match net.kind {
            sys::NetKind::Wifi => ("network-wireless-symbolic", net.name.clone()),
            sys::NetKind::Ethernet => ("network-wired-symbolic", "Wired".to_string()),
            sys::NetKind::None => ("network-offline-symbolic", "Offline".to_string()),
        };
        icon.set_icon_name(Some(name));
        text.set_text(&text_s);
        text.set_ellipsize(pango::EllipsizeMode::End);
        text.set_max_width_chars(14);
        text.set_tooltip_text(Some(&net.name));
        set_level(&text, "");
        if net.kind == sys::NetKind::None {
            text.add_css_class("muted");
        } else {
            text.remove_css_class("muted");
        }
    });
    menu_button(&content, &pop).upcast()
}

// ------------------------------------------------------------------ bluetooth

fn bluetooth() -> gtk::Widget {
    let (content, icon, text) = icon_label("bluetooth-disabled-symbolic");
    let (pop, body) = popover_box();
    body.set_width_request(240);

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let t = title("Bluetooth");
    t.set_hexpand(true);
    let switch = gtk::Switch::new();
    head.append(&t);
    head.append(&switch);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let manage = gtk::Button::with_label("Pair new device…");
    body.append(&head);
    body.append(&list);
    body.append(&manage);
    manage.connect_clicked({
        let pop = pop.clone();
        move |_| {
            pop.popdown();
            // Make the adapter discoverable and pairable for pairing from the other device.
            sys::spawn("bluetoothctl", &["--timeout", "60", "scan", "on"]);
            sys::spawn("blueman-manager", &[]);
        }
    });
    let updating = Rc::new(Cell::new(false));
    switch.connect_state_set({
        let updating = updating.clone();
        move |_, on| {
            if !updating.get() {
                sys::spawn("bluetoothctl", &["power", if on { "on" } else { "off" }]);
            }
            glib::Propagation::Proceed
        }
    });

    let apply = {
        let (list, switch, updating, icon, text) = (list.clone(), switch.clone(), updating.clone(), icon.clone(), text.clone());
        Rc::new(move |bt: sys::Bt| {
            updating.set(true);
            switch.set_active(bt.powered);
            switch.set_sensitive(bt.available);
            updating.set(false);
            let connected = bt.devices.iter().filter(|d| d.connected).count();
            icon.set_icon_name(Some(if bt.powered { "bluetooth-active-symbolic" } else { "bluetooth-disabled-symbolic" }));
            text.set_text(&if connected > 0 { connected.to_string() } else { String::new() });
            text.set_visible(connected > 0);
            clear(&list);
            if !bt.available {
                list.append(&label("No Bluetooth adapter"));
            } else if bt.devices.is_empty() {
                list.append(&label("No paired devices"));
            }
            for d in bt.devices {
                let b = gtk::Button::new();
                let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let n = label(&d.name);
                n.set_hexpand(true);
                r.append(&n);
                r.append(&label(if d.connected { "Connected" } else { "" }));
                b.set_child(Some(&r));
                let mac = d.mac.clone();
                let connected = d.connected;
                b.connect_clicked(move |_| {
                    let mac = mac.clone();
                    once(move || sys::run("bluetoothctl", &[if connected { "disconnect" } else { "connect" }, &mac]), |_| {});
                });
                list.append(&b);
            }
        })
    };
    {
        let apply = apply.clone();
        poll(&content, 6000, sys::read_bt, move |bt| apply(bt));
    }
    pop.connect_show(move |_| {
        let apply = apply.clone();
        once(sys::read_bt, move |bt| apply(bt));
    });
    menu_button(&content, &pop).upcast()
}

// ---------------------------------------------------------------------- audio

fn volume_icon(volume: u32, muted: bool) -> &'static str {
    if muted || volume == 0 {
        "audio-volume-muted-symbolic"
    } else if volume < 34 {
        "audio-volume-low-symbolic"
    } else if volume < 67 {
        "audio-volume-medium-symbolic"
    } else {
        "audio-volume-high-symbolic"
    }
}

fn audio() -> gtk::Widget {
    let (content, icon, text) = icon_label("audio-volume-muted-symbolic");
    let (pop, body) = popover_box();
    body.set_width_request(250);
    body.append(&title("Sound"));

    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    scale.set_hexpand(true);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Right);
    let mute = gtk::Button::with_label("Mute / unmute");
    let sinks = gtk::Box::new(gtk::Orientation::Vertical, 2);
    body.append(&scale);
    body.append(&mute);
    body.append(&title("Output device"));
    body.append(&sinks);

    let updating = Rc::new(Cell::new(false));
    scale.connect_value_changed({
        let updating = updating.clone();
        move |s| {
            if !updating.get() {
                sys::set_volume(s.value() as u32);
            }
        }
    });
    mute.connect_clicked(|_| sys::toggle_mute());

    let apply = {
        let (icon, text, scale, updating) = (icon.clone(), text.clone(), scale.clone(), updating.clone());
        Rc::new(move |a: Option<sys::Audio>| {
            match a {
                Some(a) => {
                    icon.set_icon_name(Some(volume_icon(a.volume, a.muted)));
                    text.set_text(&if a.muted { "muted".to_string() } else { format!("{}%", a.volume) });
                    if a.muted { text.add_css_class("muted") } else { text.remove_css_class("muted") }
                    updating.set(true);
                    scale.set_value(a.volume as f64);
                    updating.set(false);
                }
                None => {
                    icon.set_icon_name(Some("audio-volume-muted-symbolic"));
                    text.set_text("n/a");
                    text.add_css_class("muted");
                }
            }
        })
    };
    {
        let apply = apply.clone();
        poll(&content, 2000, sys::read_audio, move |a| apply(a));
    }

    pop.connect_show({
        let sinks = sinks.clone();
        let pop2 = pop.clone();
        move |_| {
            let (sinks, pop2) = (sinks.clone(), pop2.clone());
            once(sys::read_sinks, move |list| {
                clear(&sinks);
                if list.is_empty() {
                    sinks.append(&label("No audio devices"));
                }
                for (name, desc) in list {
                    let b = gtk::Button::new();
                    let l = label(&desc);
                    l.set_ellipsize(pango::EllipsizeMode::End);
                    l.set_max_width_chars(30);
                    b.set_child(Some(&l));
                    let pop2 = pop2.clone();
                    b.connect_clicked(move |_| {
                        sys::spawn("pactl", &["set-default-sink", &name]);
                        pop2.popdown();
                    });
                    sinks.append(&b);
                }
            });
        }
    });

    // Scroll over the widget to change the volume; the popover holds the full controls.
    let btn = menu_button(&content, &pop);
    let sc = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    sc.connect_scroll({
        let scale = scale.clone();
        move |_, _, dy| {
            let v = (scale.value() + if dy < 0.0 { 5.0 } else { -5.0 }).clamp(0.0, 100.0);
            scale.set_value(v);
            glib::Propagation::Stop
        }
    });
    btn.add_controller(sc);
    btn.upcast()
}

// ---------------------------------------------------------------------- power

fn power() -> gtk::Widget {
    let icon = gtk::Image::from_icon_name("system-shutdown-symbolic");
    let (pop, body) = popover_box();
    body.append(&title("Power"));
    let entries: [(&str, &str, Box<dyn Fn()>); 5] = [
        ("Lock screen", "system-lock-screen-symbolic", Box::new(|| {
            let _ = ipc::send(&Request::Lock);
        })),
        ("Log out", "system-log-out-symbolic", Box::new(|| {
            let _ = ipc::send(&Request::Quit);
        })),
        ("Suspend", "media-playback-pause-symbolic", Box::new(|| {
            // Lock first so the screen is already locked when the machine wakes up.
            let _ = ipc::send(&Request::Lock);
            sys::spawn("systemctl", &["suspend"])
        })),
        ("Restart", "system-reboot-symbolic", Box::new(|| sys::spawn("systemctl", &["reboot"]))),
        ("Shut down", "system-shutdown-symbolic", Box::new(|| sys::spawn("systemctl", &["poweroff"]))),
    ];
    for (name, icon_name, action) in entries {
        let b = gtk::Button::new();
        let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        r.append(&gtk::Image::from_icon_name(icon_name));
        r.append(&label(name));
        b.set_child(Some(&r));
        let pop = pop.clone();
        b.connect_clicked(move |_| {
            pop.popdown();
            action();
        });
        body.append(&b);
    }
    menu_button(&icon, &pop).upcast()
}
