//! Reusable GTK4 control panels (Bluetooth, Sound, Network) shared by Sekuro Settings and the installer.

use std::{cell::{Cell, RefCell}, rc::Rc, time::Duration};

use gtk4 as gtk;
use gtk::{gio, glib, pango, prelude::*};
use sekuro_common::sys;

/// Make this application's GTK theme follow the system's dark/light choice.
pub fn apply_color_scheme(dark: bool) {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(dark);
    }
}

// ---------------------------------------------------------------------------------------- helpers

/// Run blocking `work` on a worker thread and hand the result to `apply` on the UI thread.
fn bg<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, apply: impl FnOnce(T) + 'static) {
    glib::spawn_future_local(async move {
        if let Ok(v) = gio::spawn_blocking(work).await {
            apply(v);
        }
    });
}

/// Call `f` every `ms` milliseconds while `anchor` is alive and mapped (visible).
fn every<W: IsA<gtk::Widget>>(anchor: &W, ms: u64, f: impl Fn() + 'static) {
    let weak = anchor.downgrade();
    glib::timeout_add_local(Duration::from_millis(ms), move || match weak.upgrade() {
        None => glib::ControlFlow::Break,
        Some(w) => {
            if w.is_mapped() {
                f();
            }
            glib::ControlFlow::Continue
        }
    });
}

fn label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l
}

fn heading(text: &str) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_markup(&format!("<b>{}</b>", glib::markup_escape_text(text)));
    l.set_xalign(0.0);
    l.set_margin_top(6);
    l
}

fn clear(b: &impl IsA<gtk::Widget>) {
    while let Some(c) = b.first_child() {
        c.unparent();
    }
}

fn panel() -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, 10)
}


fn list_frame(inner: &impl IsA<gtk::Widget>, min_h: i32) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .child(inner)
        .min_content_height(min_h)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build()
}

// ------------------------------------------------------------------------------------- bluetooth

/// Scan, pair, connect and forget Bluetooth devices.
pub fn bluetooth_panel() -> gtk::Box {
    let p = panel();
    p.append(&heading("Bluetooth"));

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let status = label("Checking adapter…");
    status.set_hexpand(true);
    let power = gtk::Switch::new();
    power.set_valign(gtk::Align::Center);
    let scan = gtk::Button::with_label("Search for devices");
    head.append(&status);
    head.append(&scan);
    head.append(&power);
    p.append(&head);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    p.append(&list_frame(&list, 130));
    let note = label("Put your device in pairing mode, press “Search for devices”, then choose Pair. Keyboards ask for a code: type it on the keyboard and press Enter.");
    note.set_wrap(true);
    note.add_css_class("dim-label");
    p.append(&note);

    let busy = Rc::new(Cell::new(false));
    let updating = Rc::new(Cell::new(false));

    let refresh = {
        let (list, status, power, busy, updating) = (list.clone(), status.clone(), power.clone(), busy.clone(), updating.clone());
        Rc::new(move || {
            if busy.get() {
                return; // don't rebuild rows while a pairing attempt is in flight
            }
            let (list, status, power, busy, updating) = (list.clone(), status.clone(), power.clone(), busy.clone(), updating.clone());
            bg(
                || {
                    let bt = sys::read_bt();
                    let devices = if bt.available { sys::bt_all_devices() } else { vec![] };
                    (bt.available, bt.powered, devices)
                },
                move |(available, powered, devices)| {
                    updating.set(true);
                    power.set_active(powered);
                    power.set_sensitive(available);
                    updating.set(false);
                    status.set_text(if !available {
                        "No Bluetooth adapter found"
                    } else if powered {
                        "Bluetooth is on"
                    } else {
                        "Bluetooth is off"
                    });
                    clear(&list);
                    if available && powered && devices.is_empty() {
                        list.append(&label("No devices yet — press “Search for devices”."));
                    }
                    for (dev, paired) in devices {
                        list.append(&device_row(&dev, paired, &busy));
                    }
                },
            );
        })
    };

    {
        let updating = updating.clone();
        power.connect_state_set(move |_, on| {
            if !updating.get() {
                sys::bt_power(on);
            }
            glib::Propagation::Proceed
        });
    }
    {
        let (refresh, status) = (refresh.clone(), status.clone());
        scan.connect_clicked(move |b| {
            sys::bt_power(true);
            sys::bt_scan(15);
            status.set_text("Searching for 15 seconds…");
            b.set_sensitive(false);
            let b = b.clone();
            glib::timeout_add_local_once(Duration::from_secs(15), move || b.set_sensitive(true));
            refresh();
        });
    }
    refresh();
    {
        let r = refresh.clone();
        every(&p, 3000, move || r());
    }
    p
}

fn device_row(dev: &sys::BtDevice, paired: bool, busy: &Rc<Cell<bool>>) -> gtk::Box {
    let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = label(if dev.name.is_empty() { &dev.mac } else { &dev.name });
    name.set_hexpand(true);
    name.set_ellipsize(pango::EllipsizeMode::End);
    r.append(&name);
    let state = label(if dev.connected { "Connected" } else if paired { "Paired" } else { "" });
    state.add_css_class("dim-label");
    r.append(&state);

    let mac = dev.mac.clone();
    if !paired {
        let b = gtk::Button::with_label("Pair");
        let busy = busy.clone();
        b.connect_clicked(move |b| {
            b.set_sensitive(false);
            b.set_label("Pairing…");
            busy.set(true);
            let (mac, busy, b) = (mac.clone(), busy.clone(), b.clone());
            bg(move || sys::bt_pair(&mac), move |ok| {
                busy.set(false);
                b.set_label(if ok { "Paired" } else { "Failed — retry" });
                b.set_sensitive(!ok);
            });
        });
        r.append(&b);
    } else {
        let b = gtk::Button::with_label(if dev.connected { "Disconnect" } else { "Connect" });
        let connected = dev.connected;
        let mac2 = mac.clone();
        b.connect_clicked(move |_| sys::bt_connect(&mac2, !connected));
        r.append(&b);
        let f = gtk::Button::with_label("Forget");
        f.connect_clicked(move |_| sys::bt_remove(&mac));
        r.append(&f);
    }
    r
}

// ----------------------------------------------------------------------------------------- sound

/// Output/input device choice and volume.
pub fn sound_panel() -> gtk::Box {
    let p = panel();
    p.append(&heading("Output"));

    let out_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    out_scale.set_hexpand(true);
    out_scale.set_draw_value(true);
    out_scale.set_value_pos(gtk::PositionType::Right);
    out_scale.set_width_request(260);
    let mute = gtk::ToggleButton::with_label("Mute");
    let out_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    out_row.append(&label("Volume"));
    out_row.append(&out_scale);
    out_row.append(&mute);
    p.append(&out_row);
    let outs = gtk::Box::new(gtk::Orientation::Vertical, 4);
    p.append(&outs);

    p.append(&heading("Input"));
    let in_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    in_scale.set_hexpand(true);
    in_scale.set_draw_value(true);
    in_scale.set_value_pos(gtk::PositionType::Right);
    in_scale.set_width_request(260);
    let in_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    in_row.append(&label("Volume"));
    in_row.append(&in_scale);
    p.append(&in_row);
    let ins = gtk::Box::new(gtk::Orientation::Vertical, 4);
    p.append(&ins);

    let updating = Rc::new(Cell::new(false));
    {
        let u = updating.clone();
        out_scale.connect_value_changed(move |s| {
            if !u.get() {
                sys::set_volume(s.value() as u32);
            }
        });
        let u = updating.clone();
        in_scale.connect_value_changed(move |s| {
            if !u.get() {
                sys::set_source_volume(s.value() as u32);
            }
        });
        let u = updating.clone();
        mute.connect_toggled(move |m| {
            if !u.get() {
                sys::toggle_mute();
                let _ = m;
            }
        });
    }

    let refresh = {
        let (out_scale, in_scale, mute, outs, ins, updating) =
            (out_scale.clone(), in_scale.clone(), mute.clone(), outs.clone(), ins.clone(), updating.clone());
        Rc::new(move || {
            let (out_scale, in_scale, mute, outs, ins, updating) =
                (out_scale.clone(), in_scale.clone(), mute.clone(), outs.clone(), ins.clone(), updating.clone());
            bg(
                || (sys::read_audio(), sys::source_volume(), sys::read_sinks(), sys::default_sink(), sys::read_sources()),
                move |(audio, src_vol, sinks, def_sink, sources)| {
                    updating.set(true);
                    match audio {
                        Some(a) => {
                            out_scale.set_value(a.volume as f64);
                            mute.set_active(a.muted);
                        }
                        None => out_scale.set_sensitive(false),
                    }
                    if let Some(v) = src_vol {
                        in_scale.set_value(v as f64);
                    }
                    updating.set(false);

                    clear(&outs);
                    if sinks.is_empty() {
                        outs.append(&label("No output devices — is PulseAudio running?"));
                    }
                    for (name, desc) in sinks {
                        outs.append(&choice_row(&desc, name == def_sink, move || sys::set_default_sink(&name)));
                    }
                    clear(&ins);
                    if sources.is_empty() {
                        ins.append(&label("No input devices"));
                    }
                    let def_src = sys::run("pactl", &["get-default-source"]).map(|s| s.trim().to_string()).unwrap_or_default();
                    for (name, desc) in sources {
                        ins.append(&choice_row(&desc, name == def_src, move || sys::set_default_source(&name)));
                    }
                },
            );
        })
    };
    refresh();
    {
        let r = refresh.clone();
        every(&p, 4000, move || r());
    }
    // After choosing a device, refresh shortly so the check mark moves.
    let _ = refresh;
    p
}

fn choice_row(text: &str, selected: bool, on_pick: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::new();
    let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let l = label(text);
    l.set_hexpand(true);
    l.set_ellipsize(pango::EllipsizeMode::End);
    r.append(&l);
    if selected {
        r.append(&gtk::Image::from_icon_name("object-select-symbolic"));
    }
    b.set_child(Some(&r));
    b.connect_clicked(move |_| on_pick());
    b
}

// --------------------------------------------------------------------------------------- network

/// Wi-Fi switch, network list with password prompt, and connection status.
pub fn network_panel() -> gtk::Box {
    let p = panel();
    p.append(&heading("Network"));

    let status = label("Checking connection…");
    p.append(&status);

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let wifi_label = label("Wi-Fi");
    wifi_label.set_hexpand(true);
    let wifi = gtk::Switch::new();
    wifi.set_valign(gtk::Align::Center);
    head.append(&wifi_label);
    head.append(&wifi);
    p.append(&head);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    p.append(&list_frame(&list, 160));

    let pw_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let pw_for = label("");
    let pw = gtk::PasswordEntry::new();
    pw.set_hexpand(true);
    pw.set_show_peek_icon(true);
    pw.set_placeholder_text(Some("Wi-Fi password"));
    let join = gtk::Button::with_label("Join");
    pw_row.append(&pw_for);
    pw_row.append(&pw);
    pw_row.append(&join);
    pw_row.set_visible(false);
    p.append(&pw_row);

    let advanced = gtk::Button::with_label("Advanced network settings…");
    advanced.set_halign(gtk::Align::Start);
    advanced.connect_clicked(|_| sys::spawn("nm-connection-editor", &[]));
    p.append(&advanced);

    let selected = Rc::new(RefCell::new(String::new()));
    let updating = Rc::new(Cell::new(false));
    let busy = Rc::new(Cell::new(false));

    {
        let u = updating.clone();
        wifi.connect_state_set(move |_, on| {
            if !u.get() {
                sys::set_wifi(on);
            }
            glib::Propagation::Proceed
        });
    }

    let refresh = {
        let (list, status, wifi, updating, selected, pw_row, pw_for, busy) = (
            list.clone(), status.clone(), wifi.clone(), updating.clone(), selected.clone(), pw_row.clone(), pw_for.clone(), busy.clone(),
        );
        Rc::new(move |rescan: bool| {
            if busy.get() {
                return;
            }
            let (list, status, wifi, updating, selected, pw_row, pw_for, busy) = (
                list.clone(), status.clone(), wifi.clone(), updating.clone(), selected.clone(), pw_row.clone(), pw_for.clone(), busy.clone(),
            );
            bg(
                move || (sys::read_net(), sys::ip_address(), sys::wifi_enabled(), sys::read_wifi(rescan), sys::saved_connections()),
                move |(net, ip, enabled, aps, saved)| {
                    status.set_text(&match net.kind {
                        sys::NetKind::Wifi => format!("Connected to Wi-Fi “{}”{}", net.name, ip.map(|i| format!(" — {i}")).unwrap_or_default()),
                        sys::NetKind::Ethernet => format!("Connected by cable{}", ip.map(|i| format!(" — {i}")).unwrap_or_default()),
                        sys::NetKind::None => "Not connected".to_string(),
                    });
                    updating.set(true);
                    wifi.set_active(enabled);
                    updating.set(false);
                    clear(&list);
                    if !enabled {
                        list.append(&label("Wi-Fi is turned off"));
                    } else if aps.is_empty() {
                        list.append(&label("No Wi-Fi networks found (or no Wi-Fi adapter)"));
                    }
                    for ap in aps {
                        let known = saved.contains(&ap.ssid);
                        let b = gtk::Button::new();
                        let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                        let name = label(&ap.ssid);
                        name.set_hexpand(true);
                        r.append(&name);
                        if ap.in_use {
                            r.append(&label("Connected"));
                        }
                        if ap.secured {
                            r.append(&gtk::Image::from_icon_name("system-lock-screen-symbolic"));
                        }
                        r.append(&label(&format!("{}%", ap.signal)));
                        b.set_child(Some(&r));
                        let (selected, pw_row, pw_for, busy) = (selected.clone(), pw_row.clone(), pw_for.clone(), busy.clone());
                        b.connect_clicked(move |_| {
                            if ap.in_use {
                                return;
                            }
                            if ap.secured && !known {
                                *selected.borrow_mut() = ap.ssid.clone();
                                pw_for.set_text(&ap.ssid);
                                pw_row.set_visible(true);
                            } else {
                                busy.set(true);
                                let (ssid, busy) = (ap.ssid.clone(), busy.clone());
                                bg(move || sys::wifi_connect(&ssid, None), move |_| busy.set(false));
                            }
                        });
                        list.append(&b);
                    }
                },
            );
        })
    };

    {
        let (selected, pw, pw_row, busy) = (selected.clone(), pw.clone(), pw_row.clone(), busy.clone());
        join.connect_clicked(move |_| {
            let ssid = selected.borrow().clone();
            let pass = pw.text().to_string();
            if ssid.is_empty() || pass.is_empty() {
                return;
            }
            pw.set_text("");
            pw_row.set_visible(false);
            busy.set(true);
            let busy = busy.clone();
            bg(move || sys::wifi_connect(&ssid, Some(&pass)), move |_| busy.set(false));
        });
    }
    // Hidden networks: type the network name (SSID) and, if needed, the password.
    let hidden = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let h_ssid = gtk::Entry::new();
    h_ssid.set_placeholder_text(Some("Hidden network name (SSID)"));
    h_ssid.set_hexpand(true);
    let h_pw = gtk::PasswordEntry::new();
    h_pw.set_placeholder_text(Some("Password (if any)"));
    h_pw.set_show_peek_icon(true);
    let h_join = gtk::Button::with_label("Join");
    hidden.append(&h_ssid);
    hidden.append(&h_pw);
    hidden.append(&h_join);
    p.append(&hidden);
    {
        let (h_ssid, h_pw, busy) = (h_ssid.clone(), h_pw.clone(), busy.clone());
        h_join.connect_clicked(move |_| {
            let ssid = h_ssid.text().trim().to_string();
            if ssid.is_empty() {
                return;
            }
            let pass = h_pw.text().to_string();
            h_pw.set_text("");
            busy.set(true);
            let busy = busy.clone();
            bg(move || sys::wifi_connect_hidden(&ssid, Some(&pass)), move |_| busy.set(false));
        });
    }
    let scan = gtk::Button::with_label("Scan now");
    scan.set_halign(gtk::Align::Start);
    {
        let r = refresh.clone();
        scan.connect_clicked(move |_| r(true));
    }
    p.append(&scan);

    refresh(true);
    {
        let r = refresh.clone();
        let tick = Cell::new(0u32);
        // Re-read the list every 5 s and ask the adapter to scan again every 20 s.
        every(&p, 5000, move || {
            tick.set(tick.get() + 1);
            r(tick.get() % 4 == 0)
        });
    }
    p
}

// ------------------------------------------------------------------------------- administrator actions

/// Run `/usr/lib/sekuro/sekuro-sysconf <args>` as root. Asks for the user's password in a small dialog and feeds it to
/// `sudo -S` over stdin (never on a command line). `secret` (e.g. a new account's password) travels the same way.
/// `done` gets the helper's output, or the reason it failed.
pub fn run_privileged(
    anchor: &impl IsA<gtk::Widget>,
    args: Vec<String>,
    secret: Option<String>,
    done: impl Fn(Result<String, String>) + 'static,
) {
    let win = gtk::Window::builder().title("Administrator password").modal(true).resizable(false).build();
    if let Some(parent) = anchor.root().and_downcast::<gtk::Window>() {
        win.set_transient_for(Some(&parent));
    }
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.set_margin_top(16);
    body.set_margin_bottom(16);
    body.set_margin_start(16);
    body.set_margin_end(16);
    body.append(&label("Enter your password to make this change."));
    let pw = gtk::PasswordEntry::new();
    pw.set_show_peek_icon(true);
    pw.set_activates_default(true);
    pw.set_width_request(280);
    body.append(&pw);
    let msg = label("");
    body.append(&msg);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let ok = gtk::Button::with_label("Authorize");
    ok.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&ok);
    body.append(&buttons);
    win.set_child(Some(&body));
    win.set_default_widget(Some(&ok));
    {
        let win = win.clone();
        cancel.connect_clicked(move |_| win.close());
    }
    let done = Rc::new(done);
    {
        let (win, pw, msg, ok) = (win.clone(), pw.clone(), msg.clone(), ok.clone());
        ok.clone().connect_clicked(move |_| {
            let password = pw.text().to_string();
            pw.set_text("");
            ok.set_sensitive(false);
            msg.set_text("Working…");
            let (args, secret) = (args.clone(), secret.clone());
            let (win, msg, ok, done) = (win.clone(), msg.clone(), ok.clone(), done.clone());
            bg(
                move || sudo_helper(&password, &args, secret.as_deref()),
                move |res| {
                    ok.set_sensitive(true);
                    match &res {
                        Err(e) if e.contains("try again") || e.contains("incorrect password") => msg.set_text("Wrong password, try again."),
                        _ => {
                            win.close();
                            done(res);
                        }
                    }
                },
            );
        });
    }
    win.present();
    pw.grab_focus();
}

fn sudo_helper(password: &str, args: &[String], secret: Option<&str>) -> Result<String, String> {
    use std::{io::Write, process::{Command, Stdio}};
    let mut child = Command::new("sudo")
        .args(["-S", "-p", "", "-k", "/usr/lib/sekuro/sekuro-sysconf"])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run sudo: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{password}");
        if let Some(s) = secret {
            let _ = writeln!(stdin, "SEKURO:{s}");
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().last().unwrap_or("the change was refused").trim().to_string())
    }
}
