//! safora-bar — the Safora status bar: a GTK4 layer-shell window with pluggable widgets.

mod extra;
mod sys;
mod widgets;

use std::{cell::RefCell, rc::Rc, time::SystemTime};

use gtk4 as gtk;
use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use safora_common::{
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
    let app = gtk::Application::builder().application_id("org.safora.Bar").build();
    app.connect_activate(activate);
    app.run()
}

fn activate(app: &gtk::Application) {
    let window = gtk::ApplicationWindow::new(app);
    window.set_decorated(false);
    window.add_css_class("safora-bar-window");
    window.init_layer_shell();
    window.set_layer(Layer::Top);
    window.set_namespace(Some("safora-bar"));
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    // A bar started for a secondary monitor names its connector (e.g. HDMI-A-2).
    if let (Ok(want), Some(display)) = (std::env::var("SAFORA_BAR_OUTPUT"), gdk::Display::default()) {
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
    root.add_css_class("safora-bar");
    root.set_height_request(ctx.config.bar.height);
    root.set_start_widget(Some(&section(&ctx, &ctx.config.bar.left)));
    root.set_center_widget(Some(&section(&ctx, &ctx.config.bar.center)));
    root.set_end_widget(Some(&section(&ctx, &ctx.config.bar.right)));
    // When space runs out, shrink the side sections (the window list scrolls) before the clock.
    root.set_shrink_center_last(true);
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
            widget.add_css_class(&format!("module-{}", safora_common::config::widget_kind(name)));
            b.append(&widget);
        }
    }
    b
}

fn rgba(hex: &str, alpha: f64) -> String {
    let c = safora_common::config::parse_color(hex);
    format!("rgba({}, {}, {}, {:.3})", (c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, alpha)
}

fn css_for(cfg: &Config) -> String {
    let b = &cfg.bar;
    format!(
        r#"
window.safora-bar-window {{ background: transparent; }}
.safora-bar {{
    background: {bg};
    color: {fg};
    border-radius: {radius}px;
    padding: 0 8px;
    font-family: "{font}", sans-serif;
    font-size: {size}pt;
}}
.safora-bar .section {{ padding: 0 2px; }}
.safora-bar .module {{ padding: 0 8px; border-radius: {inner}px; margin: 4px 0; }}
.safora-bar button {{
    background: transparent; color: {fg}; border: none; box-shadow: none;
    padding: 0 8px; min-height: 0; min-width: 0; border-radius: {inner}px;
}}
.safora-bar button:hover {{ background: {hover}; }}
.safora-bar .workspace {{ padding: 0 9px; opacity: 0.55; }}
.safora-bar .workspace.occupied {{ opacity: 0.9; }}
.safora-bar .workspace.active {{ background: {accent}; color: {accent_fg}; opacity: 1; font-weight: bold; }}
.safora-bar .app {{ padding: 0 8px; opacity: 0.7; }}
.safora-bar .app.other-ws {{ opacity: 0.4; }}
.safora-bar .app.focused {{ opacity: 1; background: {hover}; box-shadow: inset 0 -2px {accent}; }}
.safora-bar .metric {{ opacity: 0.95; }}
.safora-bar .muted {{ opacity: 0.7; }}
.safora-bar .warn {{ color: #e0af68; }}
.safora-bar .crit {{ color: #f7768e; }}
popover > contents {{
    background: {popbg}; color: {fg}; border-radius: {inner}px; padding: 8px;
    border: 1px solid {hover};
}}
/* Everything inside the bar's popovers uses the bar's own colors, never the GTK theme's, so text
   always contrasts with the background (light or dark theme alike). */
popover > contents label, popover > contents image {{ color: {fg}; }}
popover > contents list, popover > contents row, popover > contents scrolledwindow,
popover > contents viewport {{ background: transparent; color: {fg}; }}
popover button {{
    border-radius: {inner}px; background: {btn}; color: {fg}; border: none; box-shadow: none;
}}
popover button:hover {{ background: {btn_hover}; }}
popover button:active, popover button:checked {{ background: {btn_active}; }}
popover entry, popover passwordentry {{
    background: {btn}; color: {fg}; caret-color: {fg}; border: 1px solid {hover}; box-shadow: none;
}}
popover entry:focus-within, popover passwordentry:focus-within {{ border-color: {accent}; }}
popover entry placeholder, popover passwordentry placeholder {{ color: {fg}; opacity: 0.7; }}
popover row {{ border-radius: {inner}px; padding: 2px 4px; }}
popover .muted {{ opacity: 0.7; }}
/* Optional widgets. */
.safora-bar .module-spacer {{ padding: 0; margin: 0; }}
.safora-bar .module-separator {{ padding: 0; margin: 7px 4px; min-width: 1px; background: {sep}; }}
.safora-bar .module-tray {{ padding: 0 2px; }}
.safora-bar .module-tray button {{ padding: 0 5px; }}
.safora-bar levelbar.meter trough {{ min-height: 6px; padding: 0; border: none; border-radius: 3px; background: {hover}; }}
.safora-bar levelbar.meter block.filled {{ background: {accent}; border-radius: 3px; border: none; min-height: 6px; }}
.safora-bar levelbar.meter block.empty {{ background: transparent; border: none; }}
popover .title {{ font-weight: bold; }}
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
        btn = rgba(&b.foreground, 0.08),
        btn_hover = rgba(&b.foreground, 0.18),
        sep = rgba(&b.foreground, 0.30),
        btn_active = rgba(&b.foreground, 0.26),
    )
}
