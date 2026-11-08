//! safora-settings — appearance and layout preferences for the Safora desktop.
//!
//! Every change is written to `~/.config/safora/config.toml` immediately; the bar and the
//! compositor pick it up live.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk4 as gtk;
use gtk::{gdk, glib, prelude::*};
use safora_common::{
    config::{BarPosition, Config, ThemeColors, TileMode, WallpaperMode},
    ipc::{self, Request},
};

struct Theme {
    name: &'static str,
    bar_bg: &'static str,
    bar_fg: &'static str,
    accent: &'static str,
    desktop: &'static str,
    border_inactive: &'static str,
}

const THEMES: &[Theme] = &[
    Theme { name: "Safora", bar_bg: "#1b1528", bar_fg: "#ece8f5", accent: "#3dd6d0", desktop: "#15101f", border_inactive: "#2e2542" },
    Theme { name: "Nord", bar_bg: "#2e3440", bar_fg: "#eceff4", accent: "#88c0d0", desktop: "#242933", border_inactive: "#3b4252" },
    Theme { name: "Forest", bar_bg: "#1d2021", bar_fg: "#ebdbb2", accent: "#b8bb26", desktop: "#161819", border_inactive: "#3c3836" },
    Theme { name: "Rose", bar_bg: "#1f1d2e", bar_fg: "#e0def4", accent: "#eb6f92", desktop: "#191724", border_inactive: "#26233a" },
    Theme { name: "Daylight", bar_bg: "#f2f3f7", bar_fg: "#2c3040", accent: "#3b6fd4", desktop: "#d9dce6", border_inactive: "#b8bdcc" },
    Theme { name: "High contrast", bar_bg: "#000000", bar_fg: "#ffffff", accent: "#ffd400", desktop: "#000000", border_inactive: "#555555" },
];

struct Ctx {
    cfg: RefCell<Config>,
}

impl Ctx {
    /// Mutate the config, persist it, and tell the compositor.
    fn update(&self, f: impl FnOnce(&mut Config)) {
        let mut cfg = self.cfg.borrow_mut();
        f(&mut cfg);
        cfg.bar.ensure_workspaces();
        if let Err(e) = cfg.save() {
            eprintln!("safora-settings: cannot save config: {e}");
        }
        let _ = safora_common::fonts::apply_system_fonts(&cfg);
        let _ = ipc::send(&Request::Reload);
        safora_panels::apply_color_scheme(cfg.appearance.dark);
    }
}

impl Theme {
    fn colors(&self) -> ThemeColors {
        ThemeColors {
            name: self.name.into(),
            // Daylight is the only light preset.
            dark: self.name != "Daylight",
            desktop: self.desktop.into(),
            bar_background: self.bar_bg.into(),
            bar_foreground: self.bar_fg.into(),
            accent: self.accent.into(),
            border_active: self.accent.into(),
            border_inactive: self.border_inactive.into(),
        }
    }
}

fn apply_theme(c: &mut Config, t: &Theme) {
    t.colors().apply(c);
}

mod keys;
mod more;
mod widgets;

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder().application_id("org.safora.Settings").build();
    app.connect_activate(|app| {
        if let Some(w) = app.active_window() {
            w.present();
            return;
        }
        let ctx = Rc::new(Ctx { cfg: RefCell::new(Config::load()) });
        safora_panels::apply_color_scheme(ctx.cfg.borrow().appearance.dark);
        let win = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Safora Settings")
            .default_width(760)
            .default_height(620)
            .build();

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        let sidebar = gtk::StackSidebar::new();
        sidebar.set_stack(&stack);
        sidebar.set_width_request(170);

        add_page(&stack, "appearance", "Appearance", appearance_page(&ctx));
        add_page(&stack, "wallpaper", "Wallpaper", wallpaper_page(&ctx));
        add_page(&stack, "display", "Display", display_page(&ctx));
        add_page(&stack, "bar", "Bar", bar_page(&ctx));
        add_page(&stack, "widgets", "Widgets", widgets::widgets_page(&ctx));
        add_page(&stack, "layout", "Windows", layout_page(&ctx));
        add_page(&stack, "fonts", "Fonts", fonts_page(&ctx));
        add_page(&stack, "sound", "Sound", safora_panels::sound_panel());
        add_page(&stack, "network", "Network", safora_panels::network_panel());
        add_page(&stack, "bluetooth", "Bluetooth", more::bluetooth_page());
        add_page(&stack, "apps", "Apps & startup", apps_page(&ctx));
        add_page(&stack, "power", "Power", more::power_page(&ctx));
        add_page(&stack, "input", "Input devices", more::input_page(&ctx));
        add_page(&stack, "time", "Date & time", more::datetime_page(&ctx));
        add_page(&stack, "language", "Language", more::language_page());
        add_page(&stack, "access", "Accessibility", more::access_page(&ctx));
        add_page(&stack, "keys", "Shortcuts", keys::keys_page(&ctx));
        add_page(&stack, "accounts", "Accounts", more::accounts_page());

        if let Ok(page) = std::env::var("SAFORA_SETTINGS_PAGE") {
            stack.set_visible_child_name(&page);
        }
        let split = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        split.append(&sidebar);
        split.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        stack.set_hexpand(true);
        split.append(&stack);
        win.set_child(Some(&split));
        win.present();
    });
    app.run()
}

// ------------------------------------------------------------------- helpers

fn add_page(stack: &gtk::Stack, name: &str, title: &str, content: gtk::Box) {
    content.set_margin_top(18);
    content.set_margin_bottom(18);
    content.set_margin_start(24);
    content.set_margin_end(24);
    let sw = gtk::ScrolledWindow::builder().child(&content).hscrollbar_policy(gtk::PolicyType::Never).build();
    stack.add_titled(&sw, Some(name), title);
}

fn page() -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, 12)
}

fn heading(text: &str) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_markup(&format!("<b>{}</b>", glib::markup_escape_text(text)));
    l.set_xalign(0.0);
    l.set_margin_top(6);
    l
}

fn row(text: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.set_hexpand(true);
    r.append(&l);
    r.append(widget);
    r
}

fn spin(min: f64, max: f64, step: f64, value: f64, on: impl Fn(f64) + 'static) -> gtk::SpinButton {
    let s = gtk::SpinButton::with_range(min, max, step);
    s.set_value(value);
    s.connect_value_changed(move |s| on(s.value()));
    s
}

fn dropdown(items: &[&str], selected: usize, on: impl Fn(usize) + 'static) -> gtk::DropDown {
    let d = gtk::DropDown::from_strings(items);
    d.set_selected(selected as u32);
    d.connect_selected_notify(move |d| on(d.selected() as usize));
    d
}

fn switch(active: bool, on: impl Fn(bool) + 'static) -> gtk::Switch {
    let s = gtk::Switch::new();
    s.set_active(active);
    s.set_valign(gtk::Align::Center);
    s.connect_active_notify(move |s| on(s.is_active()));
    s
}

fn hex_to_rgba(hex: &str) -> gdk::RGBA {
    let c = safora_common::config::parse_color(hex);
    gdk::RGBA::new(c[0], c[1], c[2], 1.0)
}

fn rgba_to_hex(c: &gdk::RGBA) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c.red()), b(c.green()), b(c.blue()))
}

fn color(hex: &str, on: impl Fn(String) + 'static) -> gtk::ColorDialogButton {
    let b = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
    b.set_rgba(&hex_to_rgba(hex));
    b.connect_rgba_notify(move |b| on(rgba_to_hex(&b.rgba())));
    b
}

fn entry(text: &str, on: impl Fn(String) + 'static) -> gtk::Entry {
    let e = gtk::Entry::new();
    e.set_text(text);
    e.set_width_chars(26);
    e.connect_changed(move |e| on(e.text().to_string()));
    e
}

// --------------------------------------------------------------------- pages

/// The colors of one theme, drawn as a strip of swatches.
fn swatches(t: &ThemeColors) -> gtk::Box {
    let sw = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    for hex in [&t.desktop, &t.bar_background, &t.accent] {
        let patch = gtk::DrawingArea::new();
        patch.set_content_width(34);
        patch.set_content_height(26);
        let c = hex_to_rgba(hex);
        patch.set_draw_func(move |_, cr, w, h| {
            cr.set_source_rgb(c.red() as f64, c.green() as f64, c.blue() as f64);
            let _ = cr.rectangle(0.0, 0.0, w as f64, h as f64);
            let _ = cr.fill();
        });
        sw.append(&patch);
    }
    sw
}

/// A clickable theme tile: swatches above the name.
fn theme_button(t: &ThemeColors) -> gtk::Button {
    let b = gtk::Button::new();
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 6);
    inner.append(&swatches(t));
    let name = gtk::Label::new(Some(&t.name));
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_max_width_chars(14);
    inner.append(&name);
    b.set_child(Some(&inner));
    b
}

/// The "Create your own" editor: its color buttons always show the colors in use, and changing
/// one recolors the desktop right away. "Save theme" stores the current colors under a name.
struct ThemeEditor {
    ctx: Rc<Ctx>,
    name: gtk::Entry,
    /// "Dark mode" at the top of the page: switching it picks the matching preset.
    mode: gtk::Switch,
    /// Dark mode for the theme being edited, keeping its colors.
    dark: gtk::Switch,
    colors: Vec<gtk::ColorDialogButton>,
    mine: gtk::FlowBox,
    status: gtk::Label,
    /// Set while the widgets are being filled from the config, so their change signals are ignored.
    syncing: Cell<bool>,
}

/// The editable colors of a theme, in the order the editor shows them.
const THEME_FIELDS: [&str; 6] =
    ["Desktop background", "Bar background", "Bar text", "Accent", "Focused window border", "Other window borders"];

fn theme_field(t: &mut ThemeColors, i: usize) -> &mut String {
    match i {
        0 => &mut t.desktop,
        1 => &mut t.bar_background,
        2 => &mut t.bar_foreground,
        3 => &mut t.accent,
        4 => &mut t.border_active,
        _ => &mut t.border_inactive,
    }
}

impl ThemeEditor {
    fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let mine = gtk::FlowBox::new();
        mine.set_selection_mode(gtk::SelectionMode::None);
        mine.set_max_children_per_line(3);
        mine.set_column_spacing(10);
        mine.set_row_spacing(10);
        let name = gtk::Entry::new();
        name.set_placeholder_text(Some("My theme"));
        name.set_width_chars(22);
        let status = gtk::Label::new(None);
        status.set_xalign(0.0);
        status.add_css_class("dim-label");
        let ed = Rc::new(Self {
            ctx: ctx.clone(),
            name,
            mode: gtk::Switch::new(),
            dark: gtk::Switch::new(),
            colors: (0..THEME_FIELDS.len()).map(|_| gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()))).collect(),
            mine,
            status,
            syncing: Cell::new(false),
        });
        ed.dark.set_valign(gtk::Align::Center);
        ed.mode.set_valign(gtk::Align::Center);
        let weak = Rc::downgrade(&ed);
        ed.mode.connect_active_notify(move |s| {
            let Some(ed) = weak.upgrade() else { return };
            if !ed.syncing.get() {
                ed.apply(&if s.is_active() { &THEMES[0] } else { &THEMES[4] }.colors());
            }
        });
        for (i, b) in ed.colors.iter().enumerate() {
            let weak = Rc::downgrade(&ed);
            b.connect_rgba_notify(move |b| {
                let Some(ed) = weak.upgrade() else { return };
                if ed.syncing.get() {
                    return;
                }
                let hex = rgba_to_hex(&b.rgba());
                ed.ctx.update(|cfg| {
                    let mut t = ThemeColors::from_config("", cfg);
                    *theme_field(&mut t, i) = hex;
                    t.apply(cfg);
                });
            });
        }
        let weak = Rc::downgrade(&ed);
        ed.dark.connect_active_notify(move |s| {
            let Some(ed) = weak.upgrade() else { return };
            if !ed.syncing.get() {
                ed.ctx.update(|cfg| cfg.appearance.dark = s.is_active());
                ed.sync();
            }
        });
        ed.sync();
        ed.rebuild_mine();
        ed
    }

    /// Show the colors currently in use.
    fn sync(&self) {
        self.syncing.set(true);
        let mut t = ThemeColors::from_config("", &self.ctx.cfg.borrow());
        for (i, b) in self.colors.iter().enumerate() {
            b.set_rgba(&hex_to_rgba(theme_field(&mut t, i)));
        }
        self.dark.set_active(t.dark);
        self.mode.set_active(t.dark);
        self.syncing.set(false);
    }

    /// Switch the desktop to `t` and show its colors in the editor.
    fn apply(&self, t: &ThemeColors) {
        self.ctx.update(|c| t.apply(c));
        self.sync();
    }

    fn save(self: &Rc<Self>) {
        let name = self.name.text().trim().to_string();
        if name.is_empty() {
            self.status.set_text("Type a name for the theme first.");
            self.name.grab_focus();
            return;
        }
        let mut replaced = false;
        self.ctx.update(|cfg| {
            let t = ThemeColors::from_config(&name, cfg);
            let themes = &mut cfg.appearance.themes;
            if let Some(old) = themes.iter_mut().find(|o| o.name.eq_ignore_ascii_case(&name)) {
                *old = t;
                replaced = true;
            } else {
                themes.push(t);
            }
        });
        self.status.set_text(&if replaced { format!("Updated \u{201c}{name}\u{201d}.") } else { format!("Saved \u{201c}{name}\u{201d}.") });
        self.rebuild_mine();
    }

    /// Refill the "My themes" tiles from the config.
    fn rebuild_mine(self: &Rc<Self>) {
        while let Some(child) = self.mine.first_child() {
            self.mine.remove(&child);
        }
        let themes = self.ctx.cfg.borrow().appearance.themes.clone();
        if themes.is_empty() {
            let l = gtk::Label::new(Some("No saved themes yet. Make one below."));
            l.add_css_class("dim-label");
            l.set_xalign(0.0);
            self.mine.append(&l);
            return;
        }
        for t in themes {
            let tile = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let b = theme_button(&t);
            let ed = self.clone();
            let theme = t.clone();
            b.connect_clicked(move |_| ed.apply(&theme));
            tile.append(&b);

            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            actions.set_halign(gtk::Align::Center);
            let edit = gtk::Button::from_icon_name("document-edit-symbolic");
            edit.set_tooltip_text(Some("Edit: use this theme and load it into the editor below"));
            edit.add_css_class("flat");
            let ed = self.clone();
            let theme = t.clone();
            edit.connect_clicked(move |_| {
                ed.apply(&theme);
                ed.name.set_text(&theme.name);
                ed.status.set_text("Change the colors, then press Save theme to update it.");
            });
            actions.append(&edit);
            let del = gtk::Button::from_icon_name("user-trash-symbolic");
            del.set_tooltip_text(Some("Delete this theme"));
            del.add_css_class("flat");
            let ed = self.clone();
            let name = t.name.clone();
            del.connect_clicked(move |_| {
                ed.ctx.update(|cfg| cfg.appearance.themes.retain(|o| o.name != name));
                ed.status.set_text(&format!("Deleted \u{201c}{name}\u{201d}."));
                // Defer: this button is inside the tile being removed.
                let ed = ed.clone();
                glib::idle_add_local_once(move || ed.rebuild_mine());
            });
            actions.append(&del);
            tile.append(&actions);
            self.mine.append(&tile);
        }
    }
}

fn appearance_page(ctx: &Rc<Ctx>) -> gtk::Box {
    let p = page();
    let editor = ThemeEditor::new(ctx);

    p.append(&heading("Mode"));
    p.append(&row("Dark mode", &editor.mode));
    p.append(&heading("Theme"));
    let note = gtk::Label::new(Some("Pick a preset or one of your own themes, or make a new one below."));
    note.set_xalign(0.0);
    note.set_wrap(true);
    p.append(&note);

    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_max_children_per_line(3);
    flow.set_column_spacing(10);
    flow.set_row_spacing(10);
    for t in THEMES {
        let colors = t.colors();
        let b = theme_button(&colors);
        let ed = editor.clone();
        b.connect_clicked(move |_| ed.apply(&colors));
        flow.append(&b);
    }
    p.append(&flow);

    p.append(&heading("My themes"));
    p.append(&editor.mine);

    p.append(&heading("Create your own"));
    let hint = gtk::Label::new(Some(
        "These are the colors in use right now; changes show up immediately. Start from any theme above, adjust the colors, then name it and save it. Saving under an existing name updates that theme.",
    ));
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    hint.add_css_class("dim-label");
    p.append(&hint);
    for (label, b) in THEME_FIELDS.iter().zip(&editor.colors) {
        p.append(&row(label, b));
    }
    p.append(&row("Dark apps and windows", &editor.dark));
    let save = gtk::Button::with_label("Save theme");
    save.add_css_class("suggested-action");
    let ed = editor.clone();
    save.connect_clicked(move |_| ed.save());
    let ed = editor.clone();
    editor.name.connect_activate(move |_| ed.save());
    let save_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    save_row.append(&editor.name);
    save_row.append(&save);
    p.append(&row("Theme name", &save_row));
    p.append(&editor.status);

    p.append(&heading("Reset"));
    let reset = gtk::Button::with_label("Restore all defaults");
    reset.set_halign(gtk::Align::Start);
    reset.add_css_class("destructive-action");
    reset.set_tooltip_text(Some("Your saved themes are kept"));
    let ctx = ctx.clone();
    reset.connect_clicked(move |b| {
        ctx.update(|c| {
            let themes = std::mem::take(&mut c.appearance.themes);
            *c = Config::default();
            c.appearance.themes = themes;
        });
        if let Some(w) = b.root().and_downcast::<gtk::Window>() {
            w.close(); // reopen to show the restored values
        }
    });
    p.append(&reset);
    p
}

fn bar_page(ctx: &Rc<Ctx>) -> gtk::Box {
    let p = page();
    let b = ctx.cfg.borrow().bar.clone();

    p.append(&heading("Position & size"));
    let c = ctx.clone();
    p.append(&row("Position", &dropdown(&["Top", "Bottom"], (b.position == BarPosition::Bottom) as usize, move |i| {
        c.update(|cfg| cfg.bar.position = if i == 0 { BarPosition::Top } else { BarPosition::Bottom })
    })));
    let c = ctx.clone();
    p.append(&row("Height (px)", &spin(20.0, 80.0, 1.0, b.height as f64, move |v| c.update(|cfg| cfg.bar.height = v as i32))));
    let c = ctx.clone();
    p.append(&row("Margin from screen edge (px)", &spin(0.0, 40.0, 1.0, b.margin as f64, move |v| c.update(|cfg| cfg.bar.margin = v as i32))));
    let c = ctx.clone();
    p.append(&row("Corner radius (px)", &spin(0.0, 30.0, 1.0, b.radius as f64, move |v| c.update(|cfg| cfg.bar.radius = v as i32))));
    let c = ctx.clone();
    let opacity = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.2, 1.0, 0.02);
    opacity.set_value(b.opacity);
    opacity.set_width_request(180);
    opacity.connect_value_changed(move |s| c.update(|cfg| cfg.bar.opacity = s.value()));
    p.append(&row("Opacity", &opacity));

    p.append(&heading("Text"));
    let c = ctx.clone();
    p.append(&row("Font", &entry(&b.font, move |t| c.update(|cfg| cfg.bar.font = t))));
    let c = ctx.clone();
    p.append(&row("Font size (pt)", &spin(7.0, 22.0, 1.0, b.font_size as f64, move |v| c.update(|cfg| cfg.bar.font_size = v as i32))));

    p.append(&heading("Colors"));
    let c = ctx.clone();
    p.append(&row("Background", &color(&b.background, move |h| c.update(|cfg| cfg.bar.background = h))));
    let c = ctx.clone();
    p.append(&row("Text", &color(&b.foreground, move |h| c.update(|cfg| cfg.bar.foreground = h))));
    let c = ctx.clone();
    p.append(&row("Accent", &color(&b.accent, move |h| c.update(|cfg| cfg.bar.accent = h))));
    p
}

fn layout_page(ctx: &Rc<Ctx>) -> gtk::Box {
    let p = page();
    let cfg = ctx.cfg.borrow().clone();

    p.append(&heading("Tiling"));
    let modes = [TileMode::Smart, TileMode::Dwindle, TileMode::Master, TileMode::Columns];
    let mode_idx = modes.iter().position(|m| *m == cfg.layout.mode).unwrap_or(0);
    let c = ctx.clone();
    p.append(&row(
        "Layout",
        &dropdown(&["Smart split (by window shape)", "Spiral", "Master + stack", "Columns"], mode_idx, move |i| {
            c.update(|cfg| cfg.layout.mode = modes[i.min(3)])
        }),
    ));
    let hint = gtk::Label::new(Some(
        "Smart split divides the focused window along its longer side: wide windows split side by side, tall windows split top and bottom.",
    ));
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    hint.add_css_class("dim-label");
    p.append(&hint);
    let c = ctx.clone();
    p.append(&row("Gap between windows (px)", &spin(0.0, 60.0, 1.0, cfg.layout.gap_inner as f64, move |v| c.update(|cfg| cfg.layout.gap_inner = v as i32))));
    let c = ctx.clone();
    p.append(&row("Gap around screen edge (px)", &spin(0.0, 60.0, 1.0, cfg.layout.gap_outer as f64, move |v| c.update(|cfg| cfg.layout.gap_outer = v as i32))));
    let c = ctx.clone();
    p.append(&row("Border width (px)", &spin(0.0, 12.0, 1.0, cfg.layout.border_width as f64, move |v| c.update(|cfg| cfg.layout.border_width = v as i32))));
    let c = ctx.clone();
    p.append(&row("Master window width (%)", &spin(20.0, 80.0, 1.0, cfg.layout.master_ratio * 100.0, move |v| c.update(|cfg| cfg.layout.master_ratio = v / 100.0))));

    p.append(&heading("Window effects"));
    let c = ctx.clone();
    p.append(&row("Rounded corners (px)", &spin(0.0, 40.0, 1.0, cfg.windows.corner_radius as f64, move |v| c.update(|cfg| cfg.windows.corner_radius = v as i32))));
    let c = ctx.clone();
    let oa = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.3, 1.0, 0.01);
    oa.set_value(cfg.windows.opacity_active);
    oa.set_width_request(180);
    oa.connect_value_changed(move |s| c.update(|cfg| cfg.windows.opacity_active = s.value()));
    p.append(&row("Transparency — focused window (opacity)", &oa));
    let c = ctx.clone();
    let oi = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.3, 1.0, 0.01);
    oi.set_value(cfg.windows.opacity_inactive);
    oi.set_width_request(180);
    oi.connect_value_changed(move |s| c.update(|cfg| cfg.windows.opacity_inactive = s.value()));
    p.append(&row("Transparency — other windows (opacity)", &oi));

    p.append(&heading("Colors"));
    let c = ctx.clone();
    p.append(&row("Desktop background", &color(&cfg.colors.background, move |h| c.update(|cfg| cfg.colors.background = h))));
    let c = ctx.clone();
    p.append(&row("Focused window border", &color(&cfg.colors.border_active, move |h| c.update(|cfg| cfg.colors.border_active = h))));
    let c = ctx.clone();
    p.append(&row("Other window borders", &color(&cfg.colors.border_inactive, move |h| c.update(|cfg| cfg.colors.border_inactive = h))));

    p.append(&heading("Behaviour"));
    let c = ctx.clone();
    p.append(&row("Focus follows mouse", &switch(cfg.general.focus_follows_mouse, move |on| c.update(|cfg| cfg.general.focus_follows_mouse = on))));
    let c = ctx.clone();
    p.append(&row("Workspaces", &spin(1.0, 12.0, 1.0, cfg.general.workspaces as f64, move |v| c.update(|cfg| cfg.general.workspaces = v as usize))));
    let c = ctx.clone();
    p.append(&row("Shortcut modifier (applies next login)", &dropdown(&["Super", "Alt"], cfg.general.modifier.eq_ignore_ascii_case("alt") as usize, move |i| {
        c.update(|cfg| cfg.general.modifier = if i == 0 { "super".into() } else { "alt".into() })
    })));
    p
}

fn apps_page(ctx: &Rc<Ctx>) -> gtk::Box {
    let p = page();
    let cfg = ctx.cfg.borrow().clone();
    p.append(&heading("Default programs"));
    let c = ctx.clone();
    p.append(&row("Terminal", &entry(&cfg.general.terminal, move |t| c.update(|cfg| cfg.general.terminal = t))));
    let c = ctx.clone();
    p.append(&row("Launcher", &entry(&cfg.general.launcher, move |t| c.update(|cfg| cfg.general.launcher = t))));

    p.append(&heading("Start at login"));
    let note = gtk::Label::new(Some("One command per line. These run when your session starts."));
    note.set_xalign(0.0);
    p.append(&note);
    let view = gtk::TextView::new();
    view.set_monospace(true);
    view.set_top_margin(6);
    view.set_left_margin(6);
    view.buffer().set_text(&cfg.general.autostart.join("\n"));
    let c = ctx.clone();
    view.buffer().connect_changed(move |b| {
        let text = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
        c.update(|cfg| cfg.general.autostart = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect());
    });
    let frame = gtk::Frame::new(None);
    frame.set_child(Some(&view));
    view.set_height_request(140);
    p.append(&frame);
    p
}

// ----------------------------------------------------------------------------------- wallpaper

fn wallpaper_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = vec![std::path::PathBuf::from("/usr/share/backgrounds/safora"), user_wallpaper_dir()];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(std::path::PathBuf::from(home).join("Pictures"));
    }
    dirs
}

fn user_wallpaper_dir() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("safora/wallpapers")
}

fn is_image(p: &std::path::Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("png" | "jpg" | "jpeg" | "webp")
    )
}

fn wallpaper_page(ctx: &Rc<Ctx>) -> gtk::Box {
    let p = page();
    let cfg = ctx.cfg.borrow().clone();

    p.append(&heading("Desktop wallpaper"));
    let preview = gtk::Picture::new();
    preview.set_content_fit(gtk::ContentFit::Cover);
    preview.set_size_request(480, 270);
    preview.set_halign(gtk::Align::Start);
    preview.add_css_class("card");
    if !cfg.desktop.wallpaper.is_empty() {
        preview.set_filename(Some(&cfg.desktop.wallpaper));
    }
    p.append(&preview);

    let set_wallpaper = {
        let (ctx, preview) = (ctx.clone(), preview.clone());
        Rc::new(move |path: String| {
            if path.is_empty() {
                preview.set_filename(None::<&str>);
            } else {
                preview.set_filename(Some(&path));
            }
            ctx.update(|c| c.desktop.wallpaper = path);
        })
    };

    let modes = [WallpaperMode::Fill, WallpaperMode::Fit, WallpaperMode::Stretch, WallpaperMode::Center];
    let mode_idx = modes.iter().position(|m| *m == cfg.desktop.wallpaper_mode).unwrap_or(0);
    let c = ctx.clone();
    p.append(&row(
        "Picture position",
        &dropdown(&["Fill screen", "Fit inside", "Stretch", "Centre"], mode_idx, move |i| {
            c.update(|cfg| cfg.desktop.wallpaper_mode = modes[i.min(3)])
        }),
    ));

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let choose = gtk::Button::with_label("Choose an image…");
    let none = gtk::Button::with_label("Use a plain color");
    buttons.append(&choose);
    buttons.append(&none);
    p.append(&buttons);
    {
        let sw = set_wallpaper.clone();
        none.connect_clicked(move |_| sw(String::new()));
    }
    {
        let sw = set_wallpaper.clone();
        choose.connect_clicked(move |b| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Images"));
            for pat in ["*.png", "*.jpg", "*.jpeg", "*.webp"] {
                filter.add_pattern(pat);
            }
            let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder().title("Choose a wallpaper").filters(&filters).build();
            let sw = sw.clone();
            let win = b.root().and_downcast::<gtk::Window>();
            dialog.open(win.as_ref(), gtk::gio::Cancellable::NONE, move |res| {
                let Ok(file) = res else { return };
                let Some(src) = file.path() else { return };
                // Keep a private copy so the wallpaper survives if the original is moved or deleted.
                let dir = user_wallpaper_dir();
                let dest = dir.join(src.file_name().unwrap_or_default());
                let chosen = if std::fs::create_dir_all(&dir).is_ok() && std::fs::copy(&src, &dest).is_ok() { dest } else { src };
                sw(chosen.to_string_lossy().into_owned());
            });
        });
    }

    p.append(&heading("Included and saved wallpapers"));
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_max_children_per_line(3);
    flow.set_column_spacing(10);
    flow.set_row_spacing(10);
    let mut seen = std::collections::BTreeSet::new();
    for dir in wallpaper_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut files: Vec<_> = rd.filter_map(Result::ok).map(|e| e.path()).filter(|p| is_image(p)).collect();
        files.sort();
        for f in files.into_iter().take(40) {
            if !seen.insert(f.clone()) {
                continue;
            }
            let pic = gtk::Picture::for_filename(&f);
            pic.set_content_fit(gtk::ContentFit::Cover);
            pic.set_size_request(150, 85);
            let b = gtk::Button::new();
            b.set_child(Some(&pic));
            b.set_tooltip_text(f.file_name().and_then(|n| n.to_str()));
            let sw = set_wallpaper.clone();
            let path = f.to_string_lossy().into_owned();
            b.connect_clicked(move |_| sw(path.clone()));
            flow.append(&b);
        }
    }
    p.append(&flow);
    p
}

// ---------------------------------------------------------------------------------------- fonts

fn font_button(family: &str, size: i32, on: impl Fn(String, i32) + 'static) -> gtk::FontDialogButton {
    let b = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
    b.set_level(gtk::FontLevel::Font);
    b.set_font_desc(&gtk::pango::FontDescription::from_string(&format!("{family} {size}")));
    b.connect_font_desc_notify(move |b| {
        if let Some(d) = b.font_desc() {
            let fam = d.family().map(|f| f.to_string()).unwrap_or_default();
            let sz = (d.size() as f64 / gtk::pango::SCALE as f64).round() as i32;
            if !fam.is_empty() && sz > 0 {
                on(fam, sz);
            }
        }
    });
    b
}

fn fonts_page(ctx: &Rc<Ctx>) -> gtk::Box {
    let p = page();
    let cfg = ctx.cfg.borrow().clone();
    p.append(&heading("Fonts"));
    let note = gtk::Label::new(Some(
        "The interface font is used by the bar and by GTK applications (restart an application to see the change). \
         The monospace font and size are used by the Alacritty terminal.",
    ));
    note.set_wrap(true);
    note.set_xalign(0.0);
    p.append(&note);

    let c = ctx.clone();
    p.append(&row(
        "Interface font",
        &font_button(&cfg.fonts.interface, cfg.fonts.interface_size, move |fam, sz| {
            c.update(|cfg| {
                cfg.fonts.interface = fam.clone();
                cfg.fonts.interface_size = sz;
                cfg.bar.font = fam;
                cfg.bar.font_size = sz;
            });
            let _ = safora_common::fonts::apply_system_fonts(&c.cfg.borrow());
        }),
    ));
    let c = ctx.clone();
    p.append(&row(
        "Monospace font (terminal)",
        &font_button(&cfg.fonts.monospace, cfg.fonts.monospace_size, move |fam, sz| {
            c.update(|cfg| {
                cfg.fonts.monospace = fam;
                cfg.fonts.monospace_size = sz;
            });
            let _ = safora_common::fonts::apply_system_fonts(&c.cfg.borrow());
        }),
    ));
    p
}

// -------------------------------------------------------------------------------------- display

fn display_page(ctx: &Rc<Ctx>) -> gtk::Box {
    let p = page();
    p.append(&heading("Screen size"));
    let outputs = ipc::get_outputs();
    let Some(out) = outputs.first() else {
        let l = gtk::Label::new(Some("The Safora session isn't reachable, so screen sizes can't be listed right now."));
        l.set_wrap(true);
        l.set_xalign(0.0);
        p.append(&l);
        return p;
    };

    let hz = |mhz: u32| mhz as f64 / 1000.0;
    if let Some(cur) = &out.current {
        let l = gtk::Label::new(Some(&format!("Current: {} × {} at {:.0} Hz on {}", cur.width, cur.height, hz(cur.refresh_mhz), out.name)));
        l.set_xalign(0.0);
        p.append(&l);
    }

    // Biggest first; one entry per size/refresh pair.
    let mut modes = out.modes.clone();
    modes.sort_by(|a, b| (b.width * b.height, b.refresh_mhz).cmp(&(a.width * a.height, a.refresh_mhz)));
    let mut labels = vec!["Automatic".to_string()];
    let mut values = vec!["auto".to_string()];
    for m in &modes {
        labels.push(format!("{} × {}  ·  {:.0} Hz{}", m.width, m.height, hz(m.refresh_mhz), if m.preferred { "  ·  recommended" } else { "" }));
        values.push(format!("{}x{}@{:.2}", m.width, m.height, hz(m.refresh_mhz)));
    }
    let current_setting = ctx.cfg.borrow().display.mode.clone();
    let selected = values
        .iter()
        .position(|v| {
            *v == current_setting
                || (safora_common::config::parse_mode(v).zip(safora_common::config::parse_mode(&current_setting))
                    .is_some_and(|((w1, h1, r1), (w2, h2, r2))| w1 == w2 && h1 == h2 && r2.is_none_or(|r| r1.is_some_and(|x| (x - r).abs() < 0.05))))
        })
        .unwrap_or(0);
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let c = ctx.clone();
    let vals = values.clone();
    p.append(&row("Resolution and refresh rate", &dropdown(&refs, selected, move |i| {
        if let Some(v) = vals.get(i) {
            c.update(|cfg| cfg.display.mode = v.clone());
        }
    })));

    let scales = ["Automatic (200% on 4K screens)", "100%", "200%"];
    let svals = ["auto", "1", "2"];
    let cur = ctx.cfg.borrow().display.scale.clone();
    let c = ctx.clone();
    p.append(&row("Interface scale", &dropdown(&scales, svals.iter().position(|v| *v == cur).unwrap_or(0), move |i| {
        c.update(|cfg| cfg.display.scale = svals[i].to_string());
    })));

    let c = ctx.clone();
    p.append(&row("Use additional monitors (applies at the next login)", &switch(ctx.cfg.borrow().display.extra_monitors, move |on| c.update(|cfg| cfg.display.extra_monitors = on))));
    if outputs.len() > 1 {
        let names: Vec<String> = outputs.iter().map(|o| o.name.clone()).collect();
        let l = gtk::Label::new(Some(&format!(
            "Monitors, left to right: {}. Windows tile on the first one; press Mod+O to send the focused window to the next monitor.",
            names.join(", ")
        )));
        l.set_wrap(true);
        l.set_xalign(0.0);
        p.append(&l);
    }

    let note = gtk::Label::new(Some(
        "“Automatic” uses the monitor's recommended size. In a virtual machine it follows the size of the VM window, so full-screen and window resizing work.",
    ));
    note.set_wrap(true);
    note.set_xalign(0.0);
    note.add_css_class("dim-label");
    p.append(&note);
    p
}
