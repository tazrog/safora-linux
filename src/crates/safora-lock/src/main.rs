//! safora-lock — the screen lock. Started by the compositor when the screen locks; it covers the screen as an
//! overlay, takes the keyboard, checks the user's password and then tells the compositor to unlock.

use std::{
    cell::{Cell, RefCell},
    io::Write,
    process::{Command, Stdio},
    rc::Rc,
    time::Duration,
};

use gtk4 as gtk;
use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use safora_common::{
    config::Config,
    ipc::{self, Request},
};

fn main() -> glib::ExitCode {
    // Without the compositor's secret this program can't unlock anything, so refuse to run by hand.
    let Ok(token) = std::env::var("SAFORA_LOCK_TOKEN") else {
        eprintln!("safora-lock is started by the compositor; use `saforactl lock` (or Mod+L) to lock the screen.");
        return glib::ExitCode::FAILURE;
    };
    let app = gtk::Application::builder().application_id("org.safora.Lock").build();
    app.connect_activate(move |app| build(app, token.clone()));
    app.run()
}

/// Ask `unix_chkpwd` (the setuid helper PAM itself uses) whether `password` belongs to the current user.
fn password_ok(user: &str, password: &str) -> bool {
    let Ok(mut child) = Command::new("/usr/bin/unix_chkpwd")
        .args([user, "nullok"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(password.as_bytes());
        let _ = stdin.write_all(&[0]);
    }
    child.wait().is_ok_and(|s| s.success())
}

fn build(app: &gtk::Application, token: String) {
    let cfg = Config::load();
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());

    let window = gtk::ApplicationWindow::new(app);
    window.set_decorated(false);
    window.add_css_class("lock");
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("safora-lock"));
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_exclusive_zone(-1);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }

    let css = gtk::CssProvider::new();
    css.load_from_string(&format!(
        r#"
window.lock {{ background: {bg}; color: {fg}; font-family: "{font}"; }}
.clock {{ font-size: {clock}pt; font-weight: 300; }}
.date {{ font-size: {date}pt; opacity: 0.75; }}
.who {{ font-size: {date}pt; margin-top: 28px; opacity: 0.9; }}
.hint {{ min-height: 1.4em; color: {accent}; }}
entry, passwordentry {{ min-width: 280px; background: rgba(255,255,255,0.07); color: {fg}; border-radius: 10px; border: 1px solid {accent}; padding: 6px 10px; }}
"#,
        bg = cfg.bar.background,
        fg = cfg.bar.foreground,
        accent = cfg.bar.accent,
        font = cfg.bar.font,
        clock = (64.0 * cfg.accessibility.text_scale) as i32,
        date = (16.0 * cfg.accessibility.text_scale) as i32,
    ));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }

    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    column.set_halign(gtk::Align::Center);
    column.set_valign(gtk::Align::Center);
    let clock = gtk::Label::new(None);
    clock.add_css_class("clock");
    let date = gtk::Label::new(None);
    date.add_css_class("date");
    let who = gtk::Label::new(Some(&format!("{user} — locked")));
    who.add_css_class("who");
    let entry = gtk::PasswordEntry::new();
    entry.set_show_peek_icon(false);
    entry.set_placeholder_text(Some("Password"));
    entry.set_halign(gtk::Align::Center);
    let hint = gtk::Label::new(None);
    hint.add_css_class("hint");
    for w in [clock.clone().upcast::<gtk::Widget>(), date.clone().upcast(), who.upcast(), entry.clone().upcast(), hint.clone().upcast()] {
        column.append(&w);
    }
    window.set_child(Some(&column));

    let h24 = cfg.bar.clock_24h;
    let tick = {
        let (clock, date) = (clock.clone(), date.clone());
        move || {
            if let Ok(now) = glib::DateTime::now_local() {
                if let Ok(s) = now.format(if h24 { "%H:%M" } else { "%I:%M %p" }) {
                    clock.set_text(&s);
                }
                if let Ok(s) = now.format("%A %e %B") {
                    date.set_text(&s);
                }
            }
        }
    };
    tick();
    glib::timeout_add_seconds_local(10, move || {
        tick();
        glib::ControlFlow::Continue
    });

    let failures = Rc::new(Cell::new(0u32));
    let busy = Rc::new(Cell::new(false));
    let token = Rc::new(RefCell::new(token));
    {
        let (hint, failures, busy, token, user, app) = (hint.clone(), failures.clone(), busy.clone(), token.clone(), user.clone(), app.clone());
        entry.connect_activate(move |entry| {
            if busy.get() {
                return;
            }
            let password = entry.text().to_string();
            entry.set_text("");
            if password.is_empty() {
                return;
            }
            busy.set(true);
            entry.set_sensitive(false);
            hint.set_text("Checking…");
            let (hint, failures, busy, token, user, app, entry) =
                (hint.clone(), failures.clone(), busy.clone(), token.clone(), user.clone(), app.clone(), entry.clone());
            glib::spawn_future_local(async move {
                let u = user.clone();
                let ok = gtk::gio::spawn_blocking(move || password_ok(&u, &password)).await.unwrap_or(false);
                if ok {
                    let _ = ipc::send(&Request::Unlock { token: token.borrow().clone() });
                    app.quit();
                    return;
                }
                failures.set(failures.get() + 1);
                // Slow guessing down: 2 s, growing to 10 s after repeated failures.
                let wait = (2 * failures.get().min(5)) as u64;
                hint.set_text(&format!("Wrong password. Try again in {wait} s."));
                glib::timeout_add_local_once(Duration::from_secs(wait), move || {
                    busy.set(false);
                    entry.set_sensitive(true);
                    entry.grab_focus();
                    hint.set_text("");
                });
            });
        });
    }

    window.present();
    entry.grab_focus();
}
