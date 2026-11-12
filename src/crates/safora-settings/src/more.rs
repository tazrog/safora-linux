//! Settings pages for power, accounts, date & time, language, accessibility and input devices.

use std::rc::Rc;

use gtk4 as gtk;
use gtk::{glib, prelude::*};
use safora_common::sys;

use crate::{dropdown, heading, page, row, spin, switch, Ctx};

/// power-profiles-daemon's D-Bus name, object and interface.
const PPD: (&str, &str, &str) =
    ("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles", "org.freedesktop.UPower.PowerProfiles");

/// A power-profiles-daemon property as JSON (`null` when the daemon is missing).
fn ppd_json(prop: &str) -> serde_json::Value {
    let out = cmd_out("busctl", &["--system", "--json=short", "get-property", PPD.0, PPD.1, PPD.2, prop]);
    serde_json::from_str::<serde_json::Value>(&out).map(|v| v["data"].clone()).unwrap_or_default()
}

fn ppd_get(prop: &str) -> String {
    ppd_json(prop).as_str().unwrap_or_default().to_string()
}

/// Power modes this computer offers ("power-saver", "balanced", "performance").
fn ppd_profiles() -> Vec<String> {
    ppd_json("Profiles")
        .as_array()
        .map(|a| a.iter().filter_map(|p| p["Profile"]["data"].as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn note(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.add_css_class("dim-label");
    l
}

fn cmd_out(cmd: &str, args: &[&str]) -> String {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// A dropdown over `(label, value)` pairs that can also be searched by typing.
fn searchable(items: &[String], selected: Option<usize>, on: impl Fn(usize) + 'static) -> gtk::DropDown {
    let strs: Vec<&str> = items.iter().map(String::as_str).collect();
    let d = gtk::DropDown::from_strings(&strs);
    d.set_expression(Some(gtk::PropertyExpression::new(gtk::StringObject::static_type(), None::<gtk::Expression>, "string")));
    d.set_enable_search(true);
    if let Some(i) = selected {
        d.set_selected(i as u32);
    }
    d.connect_selected_notify(move |d| on(d.selected() as usize));
    d
}

/// Show the result of an administrator action under `status`.
fn report(status: &gtk::Label, res: Result<String, String>, ok_text: &str) {
    match res {
        Ok(_) => status.set_text(ok_text),
        Err(e) => status.set_text(&format!("Not changed: {e}")),
    }
}

// ----------------------------------------------------------------------------------------------- power

const SCREEN_OFF: &[(&str, u32)] = &[("Never", 0), ("1 minute", 1), ("2 minutes", 2), ("5 minutes", 5), ("10 minutes", 10), ("15 minutes", 15), ("30 minutes", 30)];
const LOCK: &[(&str, u32)] = &[("Never", 0), ("1 minute", 1), ("2 minutes", 2), ("5 minutes", 5), ("10 minutes", 10), ("15 minutes", 15), ("30 minutes", 30)];
const SUSPEND: &[(&str, u32)] = &[("Never", 0), ("5 minutes", 5), ("10 minutes", 10), ("15 minutes", 15), ("30 minutes", 30), ("1 hour", 60), ("2 hours", 120)];
const PROFILES: &[(&str, &str)] = &[("Performance", "performance"), ("Balanced", "balanced"), ("Power saver", "power-saver")];

fn battery_line() -> Option<String> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for e in dir.flatten() {
        let p = e.path();
        if !sys::is_system_battery(&p) {
            continue;
        }
        let cap = std::fs::read_to_string(p.join("capacity")).ok()?;
        let st = std::fs::read_to_string(p.join("status")).unwrap_or_default();
        return Some(format!("Battery {}% — {}", cap.trim(), st.trim().to_lowercase()));
    }
    None
}

pub fn power_page(ctx: &Rc<Ctx>) -> gtk::Box {
    let p = page();
    let cfg = ctx.cfg.borrow().clone();
    p.append(&heading("Power"));
    p.append(&note(&battery_line().unwrap_or_else(|| "Running on mains power (no battery found).".into())));

    let pick = |list: &[(&str, u32)], cur: u32| list.iter().position(|(_, v)| *v == cur).unwrap_or(0);
    let c = ctx.clone();
    let labels: Vec<&str> = SCREEN_OFF.iter().map(|(l, _)| *l).collect();
    p.append(&row("Turn the screen off after", &dropdown(&labels, pick(SCREEN_OFF, cfg.power.screen_off_minutes), move |i| {
        c.update(|cfg| cfg.power.screen_off_minutes = SCREEN_OFF[i].1)
    })));
    let c = ctx.clone();
    let labels: Vec<&str> = SUSPEND.iter().map(|(l, _)| *l).collect();
    p.append(&row("Suspend after", &dropdown(&labels, pick(SUSPEND, cfg.power.suspend_minutes), move |i| {
        c.update(|cfg| cfg.power.suspend_minutes = SUSPEND[i].1)
    })));
    let c = ctx.clone();
    let labels: Vec<&str> = LOCK.iter().map(|(l, _)| *l).collect();
    p.append(&row("Lock the screen after", &dropdown(&labels, pick(LOCK, cfg.power.lock_minutes), move |i| {
        c.update(|cfg| cfg.power.lock_minutes = LOCK[i].1)
    })));
    let lock_now = gtk::Button::with_label("Lock the screen now");
    lock_now.set_halign(gtk::Align::Start);
    lock_now.connect_clicked(|_| {
        let _ = safora_common::ipc::send(&safora_common::ipc::Request::Lock);
    });
    p.append(&lock_now);
    p.append(&note("The timers count from the last keyboard or mouse action. A full-screen window (a video, say) keeps the screen on."));

    p.append(&heading("Power use"));
    // Talk to power-profiles-daemon over D-Bus with busctl (part of systemd). powerprofilesctl is a
    // Python script that needs python-gobject, so it is not relied on.
    let available = ppd_profiles();
    let offered: Vec<(&str, &str)> = PROFILES.iter().copied().filter(|(_, v)| available.iter().any(|a| a == v)).collect();
    if !offered.is_empty() {
        let cur = ppd_get("ActiveProfile");
        let idx = offered.iter().position(|(_, v)| *v == cur).or_else(|| offered.iter().position(|(_, v)| *v == "balanced")).unwrap_or(0);
        let labels: Vec<&str> = offered.iter().map(|(l, _)| *l).collect();
        let c = ctx.clone();
        p.append(&row("Power mode", &dropdown(&labels, idx, move |i| {
            let v = offered[i].1;
            c.update(|cfg| cfg.power.profile = v.into());
            sys::spawn("busctl", &["--system", "set-property", PPD.0, PPD.1, PPD.2, "ActiveProfile", "s", v]);
        })));
        p.append(&note("Performance runs the processor faster and uses more power; Power saver does the opposite."));
    } else {
        p.append(&note("This computer does not offer power modes."));
    }
    p
}

// -------------------------------------------------------------------------------------------- accounts

struct Account {
    name: String,
    admin: bool,
}

fn read_accounts() -> Vec<Account> {
    let wheel: Vec<String> = std::fs::read_to_string("/etc/group")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("wheel:"))
        .and_then(|l| l.rsplit(':').next().map(|m| m.split(',').map(str::to_string).collect()))
        .unwrap_or_default();
    std::fs::read_to_string("/etc/passwd")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            let uid: u32 = f.get(2)?.parse().ok()?;
            (1000..60000).contains(&uid).then(|| Account { name: f[0].to_string(), admin: wheel.iter().any(|w| w == f[0]) })
        })
        .collect()
}

fn fill_accounts(list: &gtk::Box, status: &gtk::Label) {
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    let me = std::env::var("USER").unwrap_or_default();
    for acc in read_accounts() {
        let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = format!("{}{}{}", acc.name, if acc.name == me { "  (you)" } else { "" }, if acc.admin { "  ·  administrator" } else { "" });
        let l = gtk::Label::new(Some(&title));
        l.set_xalign(0.0);
        l.set_hexpand(true);
        r.append(&l);

        let pw = gtk::Button::with_label("Change password");
        {
            let (name, status, list) = (acc.name.clone(), status.clone(), list.clone());
            pw.connect_clicked(move |b| {
                new_password_dialog(b, &name, &status, &list);
            });
        }
        r.append(&pw);
        if acc.name != me {
            let adm = gtk::Button::with_label(if acc.admin { "Remove admin" } else { "Make admin" });
            {
                let (name, status, list, on) = (acc.name.clone(), status.clone(), list.clone(), !acc.admin);
                adm.connect_clicked(move |b| {
                    let (status, list) = (status.clone(), list.clone());
                    safora_panels::run_privileged(
                        b,
                        vec!["user-admin".into(), name.clone(), if on { "on" } else { "off" }.into()],
                        None,
                        move |res| {
                            report(&status, res, "Account updated.");
                            fill_accounts(&list, &status);
                        },
                    );
                });
            }
            r.append(&adm);
            let del = gtk::Button::with_label("Remove");
            del.add_css_class("destructive-action");
            {
                let (name, status, list) = (acc.name.clone(), status.clone(), list.clone());
                del.connect_clicked(move |b| {
                    let (status, list) = (status.clone(), list.clone());
                    safora_panels::run_privileged(b, vec!["user-del".into(), name.clone()], None, move |res| {
                        report(&status, res, "Account and its home folder removed.");
                        fill_accounts(&list, &status);
                    });
                });
            }
            r.append(&del);
        }
        list.append(&r);
    }
}

fn new_password_dialog(anchor: &gtk::Button, name: &str, status: &gtk::Label, _list: &gtk::Box) {
    let win = gtk::Window::builder().title(format!("New password for {name}")).modal(true).resizable(false).build();
    if let Some(parent) = anchor.root().and_downcast::<gtk::Window>() {
        win.set_transient_for(Some(&parent));
    }
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.set_margin_top(16);
    body.set_margin_bottom(16);
    body.set_margin_start(16);
    body.set_margin_end(16);
    let a = gtk::PasswordEntry::new();
    a.set_placeholder_text(Some("New password"));
    a.set_show_peek_icon(true);
    let b = gtk::PasswordEntry::new();
    b.set_placeholder_text(Some("Repeat the password"));
    b.set_show_peek_icon(true);
    let msg = gtk::Label::new(None);
    msg.set_xalign(0.0);
    let go = gtk::Button::with_label("Set password");
    go.add_css_class("suggested-action");
    for w in [a.clone().upcast::<gtk::Widget>(), b.clone().upcast(), msg.clone().upcast(), go.clone().upcast()] {
        body.append(&w);
    }
    win.set_child(Some(&body));
    let (win2, name, status) = (win.clone(), name.to_string(), status.clone());
    go.connect_clicked(move |btn| {
        let (p1, p2) = (a.text().to_string(), b.text().to_string());
        if p1.is_empty() || p1 != p2 {
            msg.set_text(if p1.is_empty() { "Type a password." } else { "The two passwords differ." });
            return;
        }
        let (status, win3) = (status.clone(), win2.clone());
        safora_panels::run_privileged(btn, vec!["user-passwd".into(), name.clone()], Some(p1), move |res| {
            report(&status, res, "Password changed.");
            win3.close();
        });
    });
    win.present();
}

pub fn accounts_page() -> gtk::Box {
    let p = page();
    p.append(&heading("Accounts on this computer"));
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
    p.append(&list);
    p.append(&status);
    fill_accounts(&list, &status);

    p.append(&heading("Add an account"));
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Account name (lower-case letters and digits)"));
    name.set_hexpand(true);
    let pw = gtk::PasswordEntry::new();
    pw.set_placeholder_text(Some("Password"));
    pw.set_show_peek_icon(true);
    pw.set_hexpand(true);
    let admin = gtk::Switch::new();
    admin.set_valign(gtk::Align::Center);
    let create = gtk::Button::with_label("Create account");
    create.add_css_class("suggested-action");
    p.append(&name);
    p.append(&pw);
    p.append(&row("Administrator (can use sudo)", &admin));
    p.append(&create);
    {
        let (name, pw, admin, status, list) = (name.clone(), pw.clone(), admin.clone(), status.clone(), list.clone());
        create.connect_clicked(move |b| {
            let n = name.text().trim().to_string();
            let secret = pw.text().to_string();
            if n.is_empty() || secret.is_empty() {
                status.set_text("Type an account name and a password.");
                return;
            }
            let (status, list, name, pw) = (status.clone(), list.clone(), name.clone(), pw.clone());
            safora_panels::run_privileged(
                b,
                vec!["user-add".into(), n, if admin.is_active() { "admin" } else { "user" }.into()],
                Some(secret),
                move |res| {
                    if res.is_ok() {
                        name.set_text("");
                        pw.set_text("");
                    }
                    report(&status, res, "Account created.");
                    fill_accounts(&list, &status);
                },
            );
        });
    }
    p.append(&note("Changes ask for your password. Removing an account also deletes its home folder."));
    p
}

// ------------------------------------------------------------------------------------------ date & time

pub fn datetime_page(ctx: &Rc<Ctx>) -> gtk::Box {
    let p = page();
    p.append(&heading("Date & time"));
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);

    let now = gtk::Label::new(None);
    now.set_xalign(0.0);
    let tick = {
        let now = now.clone();
        move || {
            if let Ok(t) = glib::DateTime::now_local() {
                if let Ok(s) = t.format("%A %e %B %Y, %H:%M") {
                    now.set_text(&s);
                }
            }
        }
    };
    tick();
    glib::timeout_add_seconds_local(20, {
        let now = now.downgrade();
        move || match now.upgrade() {
            Some(_) => {
                tick();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        }
    });
    p.append(&now);

    let c = ctx.clone();
    p.append(&row("24-hour clock", &switch(ctx.cfg.borrow().bar.clock_24h, move |on| c.update(|cfg| cfg.bar.clock_24h = on))));

    let ntp = cmd_out("timedatectl", &["show", "-p", "NTP", "--value"]) == "yes";
    let ntp_sw = gtk::Switch::new();
    ntp_sw.set_active(ntp);
    ntp_sw.set_valign(gtk::Align::Center);
    {
        let status = status.clone();
        ntp_sw.connect_state_set(move |sw, on| {
            let status = status.clone();
            if on != (cmd_out("timedatectl", &["show", "-p", "NTP", "--value"]) == "yes") {
                safora_panels::run_privileged(sw, vec!["set-ntp".into(), if on { "on" } else { "off" }.into()], None, move |res| {
                    report(&status, res, "Clock setting saved.");
                });
            }
            glib::Propagation::Proceed
        });
    }
    p.append(&row("Set the time automatically (network time)", &ntp_sw));

    let zones: Vec<String> = cmd_out("timedatectl", &["list-timezones"]).lines().map(str::to_string).collect();
    let cur = cmd_out("timedatectl", &["show", "-p", "Timezone", "--value"]);
    let sel = zones.iter().position(|z| *z == cur);
    // The time zone picker applies through a button so a stray scroll can't change the clock.
    let dd2 = searchable(&zones, sel, |_| {});
    p.append(&row("Time zone", &dd2));
    let apply = gtk::Button::with_label("Apply time zone");
    apply.set_halign(gtk::Align::Start);
    {
        let (dd2, status, zones) = (dd2.clone(), status.clone(), zones.clone());
        apply.connect_clicked(move |b| {
            if let Some(z) = zones.get(dd2.selected() as usize) {
                let status = status.clone();
                safora_panels::run_privileged(b, vec!["set-timezone".into(), z.clone()], None, move |res| {
                    report(&status, res, "Time zone changed.");
                });
            }
        });
    }
    p.append(&apply);
    p.append(&status);
    p
}

// ---------------------------------------------------------------------------------------------- language

pub fn language_page() -> gtk::Box {
    let p = page();
    p.append(&heading("Language & region"));
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);

    let mut locales: Vec<String> = std::fs::read_to_string("/etc/locale.gen")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let l = l.trim_start_matches('#').trim();
            let (loc, set) = l.split_once(' ')?;
            (set.trim() == "UTF-8" && loc.ends_with(".UTF-8") && loc.chars().next().is_some_and(|c| c.is_ascii_lowercase())).then(|| loc.to_string())
        })
        .collect();
    locales.sort();
    locales.dedup();
    let current = std::fs::read_to_string("/etc/locale.conf")
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("LANG=").map(|v| v.trim().trim_matches('"').to_string())))
        .unwrap_or_else(|| "en_US.UTF-8".into());
    p.append(&note(&format!("Current language and region: {current}")));
    let sel = locales.iter().position(|l| *l == current);
    let dd = searchable(&locales, sel, |_| {});
    p.append(&row("Language and region", &dd));
    let apply = gtk::Button::with_label("Apply");
    apply.set_halign(gtk::Align::Start);
    {
        let (dd, status, locales) = (dd.clone(), status.clone(), locales.clone());
        apply.connect_clicked(move |b| {
            if let Some(l) = locales.get(dd.selected() as usize) {
                let status = status.clone();
                safora_panels::run_privileged(b, vec!["set-locale".into(), l.clone()], None, move |res| {
                    report(&status, res, "Saved. Log out and back in to use the new language.");
                });
            }
        });
    }
    p.append(&apply);
    p.append(&status);
    p.append(&note("The keyboard layout is on the Input devices page. Language packs for programs are installed with pacman."));
    p
}

// ----------------------------------------------------------------------------------------- accessibility

pub fn access_page(ctx: &Rc<Ctx>) -> gtk::Box {
    let p = page();
    let cfg = ctx.cfg.borrow().clone();
    p.append(&heading("Seeing"));
    let c = ctx.clone();
    p.append(&row("Text size (applies to the bar and new windows)", &spin(1.0, 2.0, 0.1, cfg.accessibility.text_scale, move |v| {
        c.update(|cfg| cfg.accessibility.text_scale = (v * 10.0).round() / 10.0)
    })));
    let c = ctx.clone();
    p.append(&row("Large mouse pointer", &switch(cfg.accessibility.large_cursor, move |on| c.update(|cfg| cfg.accessibility.large_cursor = on))));
    let c = ctx.clone();
    p.append(&row("High contrast colors", &switch(cfg.colors.border_active == "#ffd400", move |on| {
        c.update(|cfg| {
            let t = if on { &crate::THEMES[5] } else if cfg.appearance.dark { &crate::THEMES[0] } else { &crate::THEMES[4] };
            crate::apply_theme(cfg, t);
        })
    })));
    p.append(&heading("Motion"));
    let c = ctx.clone();
    p.append(&row("Reduce animations", &switch(cfg.accessibility.reduce_motion, move |on| c.update(|cfg| cfg.accessibility.reduce_motion = on))));
    p.append(&note("Programs that are already open pick up text size, pointer size and animation changes the next time they start."));
    p
}

// ------------------------------------------------------------------------------------------ input devices

pub fn input_page(ctx: &Rc<Ctx>) -> gtk::Box {
    let p = page();
    let cfg = ctx.cfg.borrow().clone();

    p.append(&heading("Keyboard"));
    let layouts = [("English (US)", "us", ""), ("English (UK)", "gb", ""), ("German", "de", ""), ("French", "fr", ""), ("Spanish", "es", ""), ("Italian", "it", ""),
        ("Portuguese", "pt", ""), ("Dutch", "nl", ""), ("Swedish", "se", ""), ("Norwegian", "no", ""), ("Danish", "dk", ""), ("Finnish", "fi", ""),
        ("Polish", "pl", ""), ("Czech", "cz", ""), ("Russian", "ru", ""), ("Ukrainian", "ua", ""), ("Turkish", "tr", ""), ("Greek", "gr", ""),
        ("Japanese", "jp", ""), ("Korean", "kr", ""), ("Dvorak (US)", "us", "dvorak"), ("Colemak (US)", "us", "colemak"), ("Canadian French", "ca", "fr")];
    let idx = layouts.iter().position(|(_, l, v)| *l == cfg.general.keyboard_layout && *v == cfg.general.keyboard_variant);
    let names: Vec<&str> = layouts.iter().map(|(n, _, _)| *n).collect();
    let c = ctx.clone();
    let dd = dropdown(&names, idx.unwrap_or(0), move |i| {
        let (_, l, v) = layouts[i];
        c.update(|cfg| {
            cfg.general.keyboard_layout = l.into();
            cfg.general.keyboard_variant = v.into();
        })
    });
    p.append(&row("Layout (applies at the next login)", &dd));
    let c = ctx.clone();
    p.append(&row("Delay before keys repeat (ms)", &spin(100.0, 1500.0, 50.0, cfg.input.repeat_delay_ms as f64, move |v| c.update(|cfg| cfg.input.repeat_delay_ms = v as i32))));
    let c = ctx.clone();
    p.append(&row("Repeats per second", &spin(2.0, 80.0, 1.0, cfg.input.repeat_rate as f64, move |v| c.update(|cfg| cfg.input.repeat_rate = v as i32))));

    p.append(&heading("Mouse and touchpad"));
    let c = ctx.clone();
    p.append(&row("Pointer speed (−1 slow … 1 fast)", &spin(-1.0, 1.0, 0.1, cfg.input.pointer_speed, move |v| c.update(|cfg| cfg.input.pointer_speed = (v * 10.0).round() / 10.0))));
    let c = ctx.clone();
    p.append(&row("Tap to click (touchpad)", &switch(cfg.input.tap_to_click, move |on| c.update(|cfg| cfg.input.tap_to_click = on))));
    let c = ctx.clone();
    p.append(&row("Natural scrolling", &switch(cfg.input.natural_scroll, move |on| c.update(|cfg| cfg.input.natural_scroll = on))));
    let c = ctx.clone();
    p.append(&row("Left-handed buttons", &switch(cfg.input.left_handed, move |on| c.update(|cfg| cfg.input.left_handed = on))));
    let c = ctx.clone();
    p.append(&row("Ignore the touchpad while typing", &switch(cfg.input.disable_while_typing, move |on| c.update(|cfg| cfg.input.disable_while_typing = on))));
    p.append(&note("Mouse and touchpad changes apply immediately."));

    p.append(&heading("Battery"));
    p.append(&device_batteries());
    p
}

/// Battery levels of connected keyboards, mice and other wireless devices, refreshed every 30 s.
fn device_batteries() -> gtk::Box {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let fill = {
        let weak = list.downgrade();
        move || {
            let weak = weak.clone();
            glib::spawn_future_local(async move {
                let Ok(bats) = gtk::gio::spawn_blocking(sys::read_device_batteries).await else { return };
                let Some(list) = weak.upgrade() else { return };
                while let Some(c) = list.first_child() {
                    list.remove(&c);
                }
                if bats.is_empty() {
                    list.append(&note(
                        "No connected keyboard or mouse reports its battery. Wired devices have none, and some \
                         wireless ones do not share their level with the computer.",
                    ));
                }
                for b in bats {
                    let mut text = match (b.percent, &b.level) {
                        (Some(pct), _) => format!("{pct}%"),
                        (None, Some(l)) => l.clone(),
                        (None, None) => "Unknown".into(),
                    };
                    if b.charging {
                        text.push_str(" (charging)");
                    }
                    let value = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    if let Some(pct) = b.percent {
                        let bar = gtk::LevelBar::for_interval(0.0, 100.0);
                        bar.set_value(pct as f64);
                        bar.set_width_request(120);
                        bar.set_valign(gtk::Align::Center);
                        bar.add_offset_value(gtk::LEVEL_BAR_OFFSET_LOW, 20.0);
                        bar.add_offset_value(gtk::LEVEL_BAR_OFFSET_HIGH, 50.0);
                        value.append(&bar);
                    }
                    let l = gtk::Label::new(Some(&text));
                    l.set_width_chars(5);
                    l.set_xalign(1.0);
                    value.append(&l);
                    list.append(&row(&b.name, &value));
                }
            });
        }
    };
    fill();
    let weak = list.downgrade();
    glib::timeout_add_seconds_local(30, move || match weak.upgrade() {
        Some(l) => {
            if l.is_mapped() {
                fill();
            }
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
    list
}

// ------------------------------------------------------------------------- bluetooth keyboard unlock

/// Bluetooth page: the shared device panel plus "unlock the disk with a Bluetooth keyboard".
pub fn bluetooth_page() -> gtk::Box {
    let p = page();
    p.append(&safora_panels::bluetooth_panel());
    p.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    p.append(&bt_unlock_section());
    p
}

fn bt_unlock_enabled() -> bool {
    std::path::Path::new("/etc/saforabt/enabled").exists()
}

fn disk_encrypted() -> bool {
    std::fs::read_to_string("/proc/cmdline").map(|c| c.contains("cryptdevice=")).unwrap_or(false)
}

fn bt_unlock_section() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.append(&heading("Unlock the disk with a Bluetooth keyboard"));

    let r = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let state = gtk::Label::new(None);
    state.set_xalign(0.0);
    state.set_hexpand(true);
    state.set_wrap(true);
    let toggle = gtk::Button::new();
    toggle.set_valign(gtk::Align::Center);
    let update = gtk::Button::with_label("Update keyboards");
    update.set_valign(gtk::Align::Center);
    update.set_tooltip_text(Some("Run this again after pairing a new keyboard or pairing the same one again"));
    r.append(&state);
    r.append(&update);
    r.append(&toggle);
    b.append(&r);

    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    b.append(&status);

    b.append(&note(
        "Pair and connect your keyboard above first, then turn this on. At startup, press a key on the keyboard so it \
         reconnects, then type the disk password. Rebuilding the startup image takes up to a minute.\n\
         Trade-off: the keyboard's pairing key is copied to the unencrypted /boot partition, so someone with physical access \
         to the disk could read it and impersonate the keyboard. A USB keyboard always works without this.",
    ));

    let show = {
        let (state, toggle, update) = (state.clone(), toggle.clone(), update.clone());
        Rc::new(move || {
            if !disk_encrypted() {
                state.set_text("Not available: this computer's disk is not encrypted.");
                toggle.set_label("Turn on");
                toggle.set_sensitive(false);
                update.set_visible(false);
                return;
            }
            let on = bt_unlock_enabled();
            state.set_text(if on { "On: a paired Bluetooth keyboard can type the disk password at startup." } else { "Off" });
            toggle.set_label(if on { "Turn off" } else { "Turn on" });
            toggle.set_sensitive(true);
            update.set_visible(on);
        })
    };
    show();

    let act = {
        let (status, show, toggle, update) = (status.clone(), show.clone(), toggle.clone(), update.clone());
        Rc::new(move |anchor: &gtk::Button, on: bool| {
            status.set_text("Waiting for your password…");
            toggle.set_sensitive(false);
            update.set_sensitive(false);
            let (status2, show2, update2) = (status.clone(), show.clone(), update.clone());
            let (status, show, update) = (status.clone(), show.clone(), update.clone());
            safora_panels::run_privileged(anchor, vec!["bt-unlock".into(), if on { "on" } else { "off" }.into()], None, move |res| {
                match res {
                    Ok(msg) => status.set_text(&msg),
                    Err(e) => status.set_text(&format!("Not changed: {e}")),
                }
                update.set_sensitive(true);
                show();
            });
            // A cancelled password dialog never calls back: re-enable the buttons once it's gone.
            glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
                let dialog_open = gtk::Window::list_toplevels()
                    .iter()
                    .filter_map(|w| w.downcast_ref::<gtk::Window>().cloned())
                    .any(|w| w.is_visible() && w.title().as_deref() == Some("Administrator password"));
                if dialog_open {
                    return glib::ControlFlow::Continue;
                }
                if status2.text() == "Waiting for your password…" {
                    status2.set_text("");
                }
                update2.set_sensitive(true);
                show2();
                glib::ControlFlow::Break
            });
        })
    };
    {
        let act = act.clone();
        toggle.connect_clicked(move |t| act(t, !bt_unlock_enabled()));
    }
    update.connect_clicked(move |u| act(u, true));
    b
}
