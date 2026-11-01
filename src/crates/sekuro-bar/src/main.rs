//! sekuro-bar — the Sekuro status bar: a GTK4 layer-shell window with pluggable widgets.

mod sys;
mod widgets;

use std::{cell::RefCell, rc::Rc, time::SystemTime};

use gtk4 as gtk;
use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use sekuro_common::{
    config::{BarPosition, Config},
    ipc::State,
};

/// Callbacks interested in compositor state changes (workspaces, windows).
pub type StateListener = Box<dyn Fn(&State)>;

pub struct Ctx {
    pub config: Config,
    pub listeners: RefCell<Vec<StateListener>>,
    pub last_state: RefCell<State>,
}

struct App {
    window: gtk::ApplicationWindow,
    css: gtk::CssProvider,
    ctx: RefCell<Option<Rc<Ctx>>>,
    last_state: RefCell<State>,
}

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder().application_id("org.sekuro.Bar").build();
    app.connect_activate(activate);
    app.run()
}

fn activate(app: &gtk::Application) {
    let window = gtk::ApplicationWindow::new(app);
    window.set_decorated(false);
    window.add_css_class("sekuro-bar-window");
    window.init_layer_shell();
    window.set_layer(Layer::Top);
    window.set_namespace(Some("sekuro-bar"));
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    // A bar started for a secondary monitor names its connector (e.g. HDMI-A-2).
    if let (Ok(want), Some(display)) = (std::env::var("SEKURO_BAR_OUTPUT"), gdk::Display::default()) {
        let monitors = display.monitors();
        for i in 0..monitors.n_items() {
            if let Some(m) = monitors.item(i).and_downcast::<gdk::Monitor>() {
                if m.connector().is_some_and(|c| c == want) {
                    window.set_monitor(Some(&m));
                }
            }
        }
    }

    let css = gtk::CssProvider::new();
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }

    let me = Rc::new(App { window, css, ctx: RefCell::new(None), last_state: RefCell::new(State::default()) });
    rebuild(&me, Config::load());
    me.window.present();

    // Compositor state arrives on a background thread and is forwarded to the UI thread.
    let (tx, rx) = async_channel::unbounded::<State>();
    std::thread::spawn(move || sys::ipc_thread(tx));
    {
        let me = me.clone();
        glib::spawn_future_local(async move {
            while let Ok(state) = rx.recv().await {
                *me.last_state.borrow_mut() = state.clone();
                if let Some(ctx) = me.ctx.borrow().as_ref() {
                    *ctx.last_state.borrow_mut() = state.clone();
                    for l in ctx.listeners.borrow().iter() {
                        l(&state);
                    }
                }
            }
        });
    }

    // Live-reload: the settings app rewrites the config file.
    let mtime = Rc::new(RefCell::new(config_mtime()));
    glib::timeout_add_seconds_local(1, move || {
        let now = config_mtime();
        if now != *mtime.borrow() {
            *mtime.borrow_mut() = now;
            rebuild(&me, Config::load());
        }
        glib::ControlFlow::Continue
    });
}

fn config_mtime() -> Option<SystemTime> {
    std::fs::metadata(Config::path()).and_then(|m| m.modified()).ok()
}

/// (Re)create the bar contents and apply layout/appearance from `config`.
fn rebuild(app: &Rc<App>, config: Config) {
    let bar = &config.bar;
    let w = &app.window;

    // Anchoring and size.
    let (near, far) = match bar.position {
        BarPosition::Top => (Edge::Top, Edge::Bottom),
        BarPosition::Bottom => (Edge::Bottom, Edge::Top),
    };
    w.set_anchor(near, true);
    w.set_anchor(far, false);
    w.set_anchor(Edge::Left, true);
    w.set_anchor(Edge::Right, true);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        w.set_margin(edge, 0);
    }
    w.set_margin(near, bar.margin);
    w.set_margin(Edge::Left, bar.margin);
    w.set_margin(Edge::Right, bar.margin);
    w.set_default_size(-1, bar.height);
    w.auto_exclusive_zone_enable();

    app.css.load_from_string(&css_for(&config));

    let ctx = Rc::new(Ctx {
        config,
        listeners: RefCell::new(Vec::new()),
        last_state: RefCell::new(app.last_state.borrow().clone()),
    });

    let root = gtk::CenterBox::new();
    root.add_css_class("sekuro-bar");
    root.set_height_request(ctx.config.bar.height);
    root.set_start_widget(Some(&section(&ctx, &ctx.config.bar.left)));
    root.set_center_widget(Some(&section(&ctx, &ctx.config.bar.center)));
    root.set_end_widget(Some(&section(&ctx, &ctx.config.bar.right)));
    w.set_child(Some(&root));

    // Give new widgets the latest known state immediately.
    let state = ctx.last_state.borrow().clone();
    for l in ctx.listeners.borrow().iter() {
        l(&state);
    }
    *app.ctx.borrow_mut() = Some(ctx);
}

fn section(ctx: &Rc<Ctx>, names: &[String]) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    b.add_css_class("section");
    for name in names {
        if let Some(widget) = widgets::build(name, ctx) {
            widget.add_css_class("module");
            widget.add_css_class(&format!("module-{name}"));
            b.append(&widget);
        }
    }
    b
}

fn rgba(hex: &str, alpha: f64) -> String {
    let c = sekuro_common::config::parse_color(hex);
    format!("rgba({}, {}, {}, {:.3})", (c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, alpha)
}

fn css_for(cfg: &Config) -> String {
    let b = &cfg.bar;
    format!(
        r#"
window.sekuro-bar-window {{ background: transparent; }}
.sekuro-bar {{
    background: {bg};
    color: {fg};
    border-radius: {radius}px;
    padding: 0 8px;
    font-family: "{font}", sans-serif;
    font-size: {size}pt;
}}
.sekuro-bar .section {{ padding: 0 2px; }}
.sekuro-bar .module {{ padding: 0 8px; border-radius: {inner}px; margin: 4px 0; }}
.sekuro-bar button {{
    background: transparent; color: {fg}; border: none; box-shadow: none;
    padding: 0 8px; min-height: 0; min-width: 0; border-radius: {inner}px;
}}
.sekuro-bar button:hover {{ background: {hover}; }}
.sekuro-bar .workspace {{ padding: 0 9px; opacity: 0.55; }}
.sekuro-bar .workspace.occupied {{ opacity: 0.9; }}
.sekuro-bar .workspace.active {{ background: {accent}; color: {accent_fg}; opacity: 1; font-weight: bold; }}
.sekuro-bar .app {{ padding: 0 8px; opacity: 0.7; }}
.sekuro-bar .app.other-ws {{ opacity: 0.4; }}
.sekuro-bar .app.focused {{ opacity: 1; background: {hover}; box-shadow: inset 0 -2px {accent}; }}
.sekuro-bar .metric {{ opacity: 0.95; }}
.sekuro-bar .muted {{ opacity: 0.5; }}
.sekuro-bar .warn {{ color: #e0af68; }}
.sekuro-bar .crit {{ color: #f7768e; }}
popover > contents {{
    background: {popbg}; color: {fg}; border-radius: {inner}px; padding: 8px;
    border: 1px solid {hover};
}}
popover button {{ border-radius: {inner}px; }}
popover row {{ border-radius: {inner}px; padding: 2px 4px; }}
popover .title {{ font-weight: bold; opacity: 0.8; }}
"#,
        bg = rgba(&b.background, b.opacity),
        fg = b.foreground,
        radius = b.radius,
        inner = (b.radius / 2).max(3),
        font = b.font,
        size = ((b.font_size as f64) * cfg.accessibility.text_scale).round() as i32,
        hover = rgba(&b.foreground, 0.12),
        accent = b.accent,
        accent_fg = b.background,
        popbg = rgba(&b.background, 0.98),
    )
}
