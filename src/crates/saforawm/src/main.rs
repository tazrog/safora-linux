//! saforawm — the Safora auto-tiling Wayland compositor.

mod handlers;
mod input;
mod ipc;
mod layout;
mod render;
mod state;
mod udev;
mod winit;

use std::time::Duration;

use smithay::{
    backend::session::Session,
    reexports::{calloop::EventLoop, wayland_server::Display},
};
use safora_common::Config;

use crate::state::SaforaState;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn,saforawm=info".into()),
        )
        .with_ansi(false)
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: saforawm [--winit] [--exec COMMAND]\n\n  --winit        run nested in a window (development)\n  --exec CMD     run CMD once the session is up");
        return;
    }
    let nested = args.iter().any(|a| a == "--winit")
        || (std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some())
            && std::env::var_os("XDG_SESSION_TYPE").is_none_or(|t| t != "tty");
    let exec: Vec<String> =
        args.windows(2).filter(|w| w[0] == "--exec").map(|w| w[1].clone()).collect();

    let mut config = Config::load();
    if nested && config.general.modifier == "super" && std::env::var_os("SAFORA_KEEP_SUPER").is_none() {
        // The host compositor owns Super; use Alt for the nested session.
        config.general.modifier = "alt".into();
    }

    let mut event_loop: EventLoop<'static, SaforaState> = EventLoop::try_new().expect("event loop");
    let display: Display<SaforaState> = Display::new().expect("wayland display");
    let mut state = SaforaState::new(event_loop.handle(), event_loop.get_signal(), display, "seat0", config);

    let backend = if nested {
        winit::init(&mut event_loop, &mut state)
    } else {
        udev::init(&mut event_loop, &mut state)
    };
    if let Err(e) = backend {
        tracing::error!("cannot start backend: {e}");
        std::process::exit(1);
    }

    if let Err(e) = ipc::init(&event_loop.handle()) {
        tracing::warn!("IPC disabled: {e}");
    }

    // Children (bar, apps) must talk to *us*, not to whatever compositor launched us.
    std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);
    std::env::set_var("XDG_SESSION_TYPE", "wayland");
    std::env::set_var("XDG_CURRENT_DESKTOP", "Safora");
    std::env::set_var("GDK_BACKEND", "wayland");
    std::env::set_var("MOZ_ENABLE_WAYLAND", "1");
    std::env::set_var("QT_QPA_PLATFORM", "wayland");
    std::env::remove_var("DISPLAY");
    tracing::info!("listening on {:?}", state.socket_name);

    let autostart = state.config.general.autostart.clone();
    for cmd in autostart.iter().chain(exec.iter()) {
        state.spawn(cmd);
    }

    let mut last_tick = std::time::Instant::now();
    let result = event_loop.run(Some(Duration::from_millis(16)), &mut state, |state| {
        state.after_dispatch();
        if last_tick.elapsed().as_secs() >= 2 {
            last_tick = std::time::Instant::now();
            tracing::debug!("loop alive; nested redraws so far: {}", winit::REDRAWS.load(std::sync::atomic::Ordering::Relaxed));
        }
        if let Some(vt) = state.vt_switch.take() {
            if let Some(session) = state.session.as_mut() {
                if let Err(e) = session.change_vt(vt) {
                    tracing::warn!("VT switch failed: {e}");
                }
            }
        }
    });
    if let Err(e) = result {
        tracing::error!("event loop: {e}");
    }
    let _ = std::fs::remove_file(safora_common::ipc::socket_path());
}
