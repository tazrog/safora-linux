//! safora-viewer — the Safora image viewer.
//!
//! `safora-viewer [FILE]` opens an image; without a file it shows the folder panel and how to pick or drop an image.
//! Images can also be opened with Ctrl+O or by dropping them on the window. Zoom with Ctrl+wheel, a touchpad
//! pinch, the header-bar buttons or the keyboard (+ / − / F fit / 1 actual size); drag to pan; ←/→ step
//! through the other images in the same folder.
//!
//! The folder panel on the left (F9 shows/hides it) browses folders and opens images inside the window.
//!
//! The ☰ menu picks the theme (Match System — the default, following Safora Settings' dark mode live —,
//! Light or Dark), the background behind the image and a checkerboard for transparent images. The choices
//! are saved in `~/.config/safora/viewer.toml`.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk4 as gtk;
use gtk::{gdk, gio, glib, graphene, gsk, prelude::*};
use safora_common::config::Config;
use serde::{Deserialize, Serialize};

mod files;

type Handler = Box<dyn Fn(&Rc<Viewer>)>;

const MIN_ZOOM: f64 = 0.02;
const MAX_ZOOM: f64 = 32.0;
const STEP: f64 = 1.25;
const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "jpe", "jfif", "gif", "bmp", "webp", "tif", "tiff", "ico", "svg", "svgz", "avif", "heic",
    "heif", "jxl", "tga", "ppm", "pgm", "pbm", "pnm", "xpm", "xbm", "qoi",
];

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("org.safora.Viewer")
        .flags(gio::ApplicationFlags::HANDLES_OPEN | gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_startup(|app| {
        install_css();
        set_accels(app);
        setup_prefs(app);
    });
    app.connect_activate(|app| {
        Viewer::new(app).window.present();
    });
    app.connect_open(|app, files, _| {
        let viewer = Viewer::new(app);
        if let Some(path) = files.first().and_then(|f| f.path()) {
            viewer.load(&path);
        }
        viewer.window.present();
    });
    app.run()
}

fn install_css() {
    let css = gtk::CssProvider::new();
    css.load_from_string(
        ".viewer-zoom { min-width: 5em; font-feature-settings: \"tnum\"; }
         .viewer-subtitle { font-size: smaller; opacity: 0.7; }
         .viewer-empty-icon { opacity: 0.4; }",
    );
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}

fn set_accels(app: &gtk::Application) {
    let accels: &[(&str, &[&str])] = &[
        ("win.open", &["<Ctrl>o"]),
        ("win.zoom-in", &["<Ctrl>plus", "<Ctrl>equal", "plus", "equal", "KP_Add", "<Ctrl>KP_Add"]),
        ("win.zoom-out", &["<Ctrl>minus", "minus", "KP_Subtract", "<Ctrl>KP_Subtract"]),
        ("win.zoom-fit", &["<Ctrl>0", "f", "KP_0"]),
        ("win.zoom-actual", &["<Ctrl>1", "1", "KP_1"]),
        ("win.next", &["Right", "Page_Down", "space"]),
        ("win.prev", &["Left", "Page_Up", "BackSpace"]),
        ("win.fullscreen", &["F11"]),
        ("win.sidebar", &["F9"]),
        ("win.leave-fullscreen", &["Escape"]),
        ("window.close", &["<Ctrl>w", "<Ctrl>q"]),
    ];
    for (action, keys) in accels {
        app.set_accels_for_action(action, keys);
    }
}

// ---------------------------------------------------------------------------------------------------------
// Preferences: theme, background, checkerboard
// ---------------------------------------------------------------------------------------------------------

pub const THEMES: &[(&str, &str)] = &[("system", "Match System"), ("light", "Light"), ("dark", "Dark")];
pub const BACKGROUNDS: &[(&str, &str)] = &[("theme", "Theme"), ("black", "Black"), ("gray", "Gray"), ("white", "White")];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// "system" (follow Safora's dark-mode setting), "light" or "dark".
    pub theme: String,
    /// Colour around and behind the image: "theme", "black", "gray" or "white".
    pub background: String,
    /// Draw a checkerboard behind the image so transparent areas are visible.
    pub checkerboard: bool,
    /// Show the folder panel on the left.
    pub sidebar: bool,
    /// The folder the panel showed last (it starts there when no image is opened).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub folder: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { theme: "system".into(), background: "theme".into(), checkerboard: true, sidebar: true, folder: String::new() }
    }
}

impl Prefs {
    fn path() -> PathBuf {
        Config::dir().join("viewer.toml")
    }

    fn load() -> Self {
        let mut p: Prefs =
            std::fs::read_to_string(Self::path()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default();
        p.sanitize();
        p
    }

    fn sanitize(&mut self) {
        let d = Prefs::default();
        if !THEMES.iter().any(|(k, _)| *k == self.theme) {
            self.theme = d.theme;
        }
        if !BACKGROUNDS.iter().any(|(k, _)| *k == self.background) {
            self.background = d.background;
        }
    }

    fn save(&self) {
        let path = Self::path();
        let res = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| toml::to_string_pretty(self).map_err(std::io::Error::other))
            .and_then(|text| std::fs::write(&path, text));
        if let Err(e) = res {
            eprintln!("safora-viewer: can't save {}: {e}", path.display());
        }
    }

    /// Whether the dark variant should be used, given the system's dark-mode setting.
    pub fn is_dark(&self, system_dark: bool) -> bool {
        match self.theme.as_str() {
            "light" => false,
            "dark" => true,
            _ => system_dark,
        }
    }

    /// CSS for the image area (empty for "theme": the theme's own window colour).
    pub fn background_css(&self) -> String {
        let color = match self.background.as_str() {
            "black" => "#000000",
            "gray" => "#808080",
            "white" => "#ffffff",
            _ => return String::new(),
        };
        format!(".viewer-view {{ background-color: {color}; }}")
    }
}

thread_local! {
    static PREFS: RefCell<Prefs> = RefCell::new(Prefs::default());
    static BG_CSS: gtk::CssProvider = gtk::CssProvider::new();
    /// Every live canvas, so a checkerboard change can redraw all windows.
    static CANVASES: RefCell<Vec<glib::WeakRef<Canvas>>> = const { RefCell::new(Vec::new()) };
}

fn prefs() -> Prefs {
    PREFS.with(|p| p.borrow().clone())
}

/// Apply the theme, background and checkerboard preferences to the whole application.
fn apply_prefs() {
    let p = prefs();
    if let Some(settings) = gtk::Settings::default() {
        set_dark(&settings, p.is_dark(Config::load().appearance.dark));
    }
    BG_CSS.with(|css| css.load_from_string(&p.background_css()));
    CANVASES.with(|list| {
        list.borrow_mut().retain(|w| match w.upgrade() {
            Some(c) => {
                c.queue_draw();
                true
            }
            None => false,
        })
    });
}

/// Switch the application between the light and dark variants of the theme. GTK 4.20 replaced
/// `gtk-application-prefer-dark-theme` with `gtk-interface-color-scheme` (and warns about the old one), so
/// use the new property when the running GTK has it.
fn set_dark(settings: &gtk::Settings, dark: bool) {
    const PROP: &str = "gtk-interface-color-scheme";
    if let Some(pspec) = settings.find_property(PROP) {
        // GtkInterfaceColorScheme: 2 = dark, 3 = light.
        let value = glib::EnumClass::with_type(pspec.value_type()).and_then(|c| c.to_value(if dark { 2 } else { 3 }));
        if let Some(value) = value {
            if settings.property_value(PROP).transform::<i32>().ok().and_then(|v| v.get::<i32>().ok())
                != value.transform::<i32>().ok().and_then(|v| v.get::<i32>().ok())
            {
                settings.set_property_from_value(PROP, &value);
            }
            return;
        }
    }
    if settings.is_gtk_application_prefer_dark_theme() != dark {
        settings.set_gtk_application_prefer_dark_theme(dark);
    }
}

fn update_prefs(f: impl FnOnce(&mut Prefs)) {
    let p = PREFS.with(|p| {
        let mut p = p.borrow_mut();
        f(&mut p);
        p.clone()
    });
    p.save();
    apply_prefs();
}

/// App-wide stateful actions for the menu, and a watch on Safora's config so "Match System" follows
/// dark-mode changes made in Safora Settings while the viewer is open.
fn setup_prefs(app: &gtk::Application) {
    let p = Prefs::load();
    PREFS.with(|cell| cell.replace(p.clone()));
    if let Some(display) = gdk::Display::default() {
        BG_CSS.with(|css| {
            gtk::style_context_add_provider_for_display(&display, css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1)
        });
    }

    let theme = gio::SimpleAction::new_stateful("theme", Some(glib::VariantTy::STRING), &p.theme.to_variant());
    theme.connect_activate(|a, param| {
        if let Some(v) = param.and_then(|v| v.get::<String>()) {
            a.set_state(&v.to_variant());
            update_prefs(|p| p.theme = v);
        }
    });
    let bg = gio::SimpleAction::new_stateful("background", Some(glib::VariantTy::STRING), &p.background.to_variant());
    bg.connect_activate(|a, param| {
        if let Some(v) = param.and_then(|v| v.get::<String>()) {
            a.set_state(&v.to_variant());
            update_prefs(|p| p.background = v);
        }
    });
    let checker = gio::SimpleAction::new_stateful("checkerboard", None, &p.checkerboard.to_variant());
    checker.connect_activate(|a, _| {
        let on = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(true);
        a.set_state(&on.to_variant());
        update_prefs(|p| p.checkerboard = on);
    });
    app.add_action(&theme);
    app.add_action(&bg);
    app.add_action(&checker);

    let file = gio::File::for_path(Config::path());
    if let Ok(monitor) = file.monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) {
        monitor.connect_changed(|_, _, _, event| {
            if matches!(
                event,
                gio::FileMonitorEvent::ChangesDoneHint
                    | gio::FileMonitorEvent::Created
                    | gio::FileMonitorEvent::MovedIn
                    | gio::FileMonitorEvent::Renamed
            ) {
                apply_prefs();
            }
        });
        // Keep the monitor alive for the life of the application.
        app.connect_shutdown(move |_| {
            monitor.cancel();
        });
    }
    apply_prefs();
}

fn prefs_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let theme = gio::Menu::new();
    for (key, label) in THEMES {
        theme.append(Some(label), Some(&format!("app.theme::{key}")));
    }
    menu.append_section(Some("Theme"), &theme);
    let bg = gio::Menu::new();
    for (key, label) in BACKGROUNDS {
        bg.append(Some(label), Some(&format!("app.background::{key}")));
    }
    bg.append(Some("Checkerboard Behind Transparency"), Some("app.checkerboard"));
    menu.append_section(Some("Background"), &bg);
    let misc = gio::Menu::new();
    misc.append(Some("Open Image…"), Some("win.open"));
    misc.append(Some("Folder Panel"), Some("win.sidebar"));
    misc.append(Some("Fullscreen"), Some("win.fullscreen"));
    misc.append(Some("Close"), Some("window.close"));
    menu.append_section(None, &misc);
    menu
}

// ---------------------------------------------------------------------------------------------------------
// Canvas: draws the texture at exactly `zoom` × its pixel size, centred when smaller than its allocation.
// ---------------------------------------------------------------------------------------------------------

mod imp {
    use super::*;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub struct Canvas {
        pub texture: RefCell<Option<gdk::Texture>>,
        pub zoom: Cell<f64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "SaforaViewerCanvas";
        type Type = super::Canvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Canvas {}

    impl WidgetImpl for Canvas {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let size = match &*self.texture.borrow() {
                Some(t) => {
                    let px = if orientation == gtk::Orientation::Horizontal { t.width() } else { t.height() };
                    (px as f64 * self.zoom.get()).round().max(1.0) as i32
                }
                None => 0,
            };
            (size, size, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(t) = &*self.texture.borrow() else { return };
            let widget = self.obj();
            let zoom = self.zoom.get();
            let (w, h) = (t.width() as f64 * zoom, t.height() as f64 * zoom);
            let x = ((widget.width() as f64 - w) / 2.0).max(0.0).floor();
            let y = ((widget.height() as f64 - h) / 2.0).max(0.0).floor();
            // Big pixels when zoomed far in (so they can be inspected), mipmapped when shrinking.
            let filter = if zoom >= 3.0 {
                gsk::ScalingFilter::Nearest
            } else if zoom < 1.0 {
                gsk::ScalingFilter::Trilinear
            } else {
                gsk::ScalingFilter::Linear
            };
            let rect = graphene::Rect::new(x as f32, y as f32, w as f32, h as f32);
            if PREFS.with(|p| p.borrow().checkerboard) {
                // A 16 px checkerboard in screen pixels, shown through transparent parts of the image.
                const S: f32 = 8.0;
                let light = gdk::RGBA::new(0.8, 0.8, 0.8, 1.0);
                let dark = gdk::RGBA::new(0.6, 0.6, 0.6, 1.0);
                let tile = graphene::Rect::new(x as f32, y as f32, 2.0 * S, 2.0 * S);
                snapshot.push_repeat(&rect, Some(&tile));
                snapshot.append_color(&light, &tile);
                snapshot.append_color(&dark, &graphene::Rect::new(x as f32, y as f32, S, S));
                snapshot.append_color(&dark, &graphene::Rect::new(x as f32 + S, y as f32 + S, S, S));
                snapshot.pop();
            }
            snapshot.append_scaled_texture(t, filter, &rect);
        }
    }
}

glib::wrapper! {
    pub struct Canvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Canvas {
    fn new() -> Self {
        let c: Self = glib::Object::new();
        c.imp_ref().zoom.set(1.0);
        CANVASES.with(|list| list.borrow_mut().push(c.downgrade()));
        c
    }
    fn imp_ref(&self) -> &imp::Canvas {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        self.imp()
    }
    fn texture(&self) -> Option<gdk::Texture> {
        self.imp_ref().texture.borrow().clone()
    }
    fn set_texture(&self, t: Option<gdk::Texture>) {
        self.imp_ref().texture.replace(t);
        self.queue_resize();
        self.queue_draw();
    }
    fn zoom(&self) -> f64 {
        self.imp_ref().zoom.get()
    }
    fn set_zoom(&self, z: f64) {
        self.imp_ref().zoom.set(z);
        self.queue_resize();
        // The allocation may not change (image still smaller than the window), so redraw explicitly.
        self.queue_draw();
    }
}

// ---------------------------------------------------------------------------------------------------------
// Viewer window
// ---------------------------------------------------------------------------------------------------------

struct Viewer {
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    scroller: gtk::ScrolledWindow,
    canvas: Canvas,
    title: gtk::Label,
    subtitle: gtk::Label,
    zoom_label: gtk::Button,
    empty_text: gtk::Label,
    image_actions: Vec<gio::SimpleAction>,
    nav_actions: Vec<gio::SimpleAction>,
    files: Rc<files::FilePanel>,
    path: RefCell<Option<PathBuf>>,
    siblings: RefCell<Vec<PathBuf>>,
    /// Fit-to-window mode: the zoom follows the window size until the user zooms by hand.
    fit: Cell<bool>,
    /// Scroll positions to apply once the viewport has grown/shrunk to the new zoom (see `zoom_at`).
    pending_h: Cell<Option<f64>>,
    pending_v: Cell<Option<f64>>,
    pointer: Cell<Option<(f64, f64)>>,
}

impl Viewer {
    fn new(app: &gtk::Application) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Image Viewer")
            .icon_name("safora-viewer")
            .default_width(1000)
            .default_height(720)
            .build();

        // Header bar
        let header = gtk::HeaderBar::new();
        let open_btn = icon_button("document-open-symbolic", "Open an image (Ctrl+O)", "win.open");
        let prev_btn = icon_button("go-previous-symbolic", "Previous image in folder (←)", "win.prev");
        let next_btn = icon_button("go-next-symbolic", "Next image in folder (→)", "win.next");
        let nav = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        nav.add_css_class("linked");
        nav.append(&prev_btn);
        nav.append(&next_btn);
        let side_btn = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Folder panel (F9)")
            .action_name("win.sidebar")
            .build();
        header.pack_start(&side_btn);
        header.pack_start(&open_btn);
        header.pack_start(&nav);

        let title = gtk::Label::new(Some("Image Viewer"));
        title.add_css_class("title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        let subtitle = gtk::Label::new(None);
        subtitle.add_css_class("viewer-subtitle");
        subtitle.set_visible(false);
        let title_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        title_box.set_valign(gtk::Align::Center);
        title_box.append(&title);
        title_box.append(&subtitle);
        header.set_title_widget(Some(&title_box));

        let full_btn = icon_button("view-fullscreen-symbolic", "Fullscreen (F11)", "win.fullscreen");
        let fit_btn = icon_button("zoom-fit-best-symbolic", "Fit to window (F)", "win.zoom-fit");
        let actual_btn = icon_button("zoom-original-symbolic", "Actual size (1)", "win.zoom-actual");
        let zoom_out = icon_button("zoom-out-symbolic", "Zoom out (−)", "win.zoom-out");
        let zoom_in = icon_button("zoom-in-symbolic", "Zoom in (+)", "win.zoom-in");
        let zoom_label = gtk::Button::with_label("100%");
        zoom_label.add_css_class("viewer-zoom");
        zoom_label.set_tooltip_text(Some("Switch between fit to window and actual size"));
        zoom_label.set_action_name(Some("win.zoom-toggle"));
        let zoom_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        zoom_box.add_css_class("linked");
        zoom_box.append(&zoom_out);
        zoom_box.append(&zoom_label);
        zoom_box.append(&zoom_in);
        let menu_btn = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .tooltip_text("Theme and background")
            .menu_model(&prefs_menu())
            .build();
        header.pack_end(&menu_btn);
        header.pack_end(&full_btn);
        header.pack_end(&actual_btn);
        header.pack_end(&fit_btn);
        header.pack_end(&zoom_box);
        window.set_titlebar(Some(&header));

        // Image page
        let canvas = Canvas::new();
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_child(Some(&canvas));
        scroller.add_css_class("viewer-view");
        scroller.set_hexpand(true);
        scroller.set_vexpand(true);

        // Empty page
        let empty = gtk::Box::new(gtk::Orientation::Vertical, 12);
        empty.set_valign(gtk::Align::Center);
        empty.set_halign(gtk::Align::Center);
        let icon = gtk::Image::from_icon_name("image-x-generic-symbolic");
        icon.set_pixel_size(96);
        icon.add_css_class("viewer-empty-icon");
        let empty_text = gtk::Label::new(Some("No image open"));
        empty_text.add_css_class("title-2");
        empty_text.set_wrap(true);
        empty_text.set_justify(gtk::Justification::Center);
        let hint = gtk::Label::new(Some(
            "Pick an image in the folder panel on the left,\nor drag one from your file manager onto this window.\n\nNo panel? Press F9 to show it.",
        ));
        hint.add_css_class("dim-label");
        hint.set_justify(gtk::Justification::Center);
        empty.append(&icon);
        empty.append(&empty_text);
        empty.append(&hint);

        let stack = gtk::Stack::new();
        stack.add_css_class("viewer-view");
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&scroller, Some("image"));
        stack.set_visible_child_name("empty");

        // Folder panel | image, with a draggable divider.
        let p = prefs();
        let start = Some(PathBuf::from(&p.folder))
            .filter(|d| !p.folder.is_empty() && d.is_dir())
            .or_else(|| glib::user_special_dir(glib::UserDirectory::Pictures).filter(|d| d.is_dir()))
            .unwrap_or_else(glib::home_dir);
        let files = files::FilePanel::new(&start);
        files.root.set_visible(p.sidebar);
        files.connect_dir_changed(|dir| {
            let dir = dir.to_string_lossy().into_owned();
            if prefs().folder != dir {
                update_prefs(|p| p.folder = dir);
            }
        });
        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_start_child(Some(&files.root));
        paned.set_end_child(Some(&stack));
        paned.set_resize_start_child(false);
        paned.set_shrink_start_child(false);
        paned.set_shrink_end_child(false);
        paned.set_position(250);
        window.set_child(Some(&paned));

        let mk = |name: &str| gio::SimpleAction::new(name, None);
        let image_actions: Vec<_> =
            ["zoom-in", "zoom-out", "zoom-fit", "zoom-actual", "zoom-toggle"].into_iter().map(mk).collect();
        let nav_actions: Vec<_> = ["next", "prev"].into_iter().map(mk).collect();

        let v = Rc::new(Viewer {
            window,
            stack,
            scroller,
            canvas,
            title,
            subtitle,
            zoom_label,
            empty_text,
            image_actions,
            nav_actions,
            files,
            path: RefCell::new(None),
            siblings: RefCell::new(Vec::new()),
            fit: Cell::new(true),
            pending_h: Cell::new(None),
            pending_v: Cell::new(None),
            pointer: Cell::new(None),
        });
        let weak = Rc::downgrade(&v);
        v.files.connect_open(move |path| {
            if let Some(v) = weak.upgrade() {
                v.load(path);
            }
        });
        v.setup_actions();
        v.setup_input();
        v.update_ui();
        // The signal handlers only hold weak references; the window owns the viewer state and frees it
        // when it is destroyed (GTK drops this handler then).
        let keep = Rc::clone(&v);
        v.window.connect_destroy(move |_| {
            let _ = &keep;
        });
        v
    }

    fn setup_actions(self: &Rc<Self>) {
        let add = |action: &gio::SimpleAction, f: Handler| {
            let v = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(v) = v.upgrade() {
                    f(&v)
                }
            });
            self.window.add_action(action);
        };
        let simple = |name: &str| gio::SimpleAction::new(name, None);

        add(&simple("open"), Box::new(|v| v.open_dialog()));
        add(&simple("fullscreen"), Box::new(|v| {
            if v.window.is_fullscreen() { v.window.unfullscreen() } else { v.window.fullscreen() }
        }));
        add(&simple("leave-fullscreen"), Box::new(|v| v.window.unfullscreen()));
        let sidebar = gio::SimpleAction::new_stateful("sidebar", None, &prefs().sidebar.to_variant());
        let w = Rc::downgrade(self);
        sidebar.connect_activate(move |a, _| {
            let on = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(true);
            a.set_state(&on.to_variant());
            if let Some(v) = w.upgrade() {
                v.files.root.set_visible(on);
            }
            update_prefs(|p| p.sidebar = on);
        });
        self.window.add_action(&sidebar);

        let handlers: [Handler; 5] = [
            Box::new(|v| v.zoom_step(STEP)),
            Box::new(|v| v.zoom_step(1.0 / STEP)),
            Box::new(|v| v.set_fit()),
            Box::new(|v| v.zoom_centered(1.0)),
            Box::new(|v| {
                if v.fit.get() && (v.canvas.zoom() - 1.0).abs() > 1e-6 { v.zoom_centered(1.0) } else { v.set_fit() }
            }),
        ];
        for (action, f) in self.image_actions.iter().zip(handlers) {
            add(action, f);
        }
        add(&self.nav_actions[0], Box::new(|v| v.step(1)));
        add(&self.nav_actions[1], Box::new(|v| v.step(-1)));
    }

    fn setup_input(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);

        // Keep fit mode fitted when the window changes size; apply deferred scroll positions after a zoom.
        let (hadj, vadj) = (self.scroller.hadjustment(), self.scroller.vadjustment());
        for adj in [&hadj, &vadj] {
            let w = weak.clone();
            adj.connect_page_size_notify(move |_| {
                if let Some(v) = w.upgrade() {
                    if v.fit.get() {
                        v.apply_fit();
                    }
                }
            });
        }
        let w = weak.clone();
        hadj.connect_upper_notify(move |a| {
            if let Some(val) = w.upgrade().and_then(|v| v.pending_h.take()) {
                a.set_value(val);
            }
        });
        let w = weak.clone();
        vadj.connect_upper_notify(move |a| {
            if let Some(val) = w.upgrade().and_then(|v| v.pending_v.take()) {
                a.set_value(val);
            }
        });

        // Pointer position, so wheel zoom keeps the point under the cursor in place.
        let motion = gtk::EventControllerMotion::new();
        let w = weak.clone();
        motion.connect_motion(move |_, x, y| {
            if let Some(v) = w.upgrade() {
                v.pointer.set(Some((x, y)));
            }
        });
        let w = weak.clone();
        motion.connect_leave(move |_| {
            if let Some(v) = w.upgrade() {
                v.pointer.set(None);
            }
        });
        self.scroller.add_controller(motion);

        // Ctrl + wheel zooms (capture phase, so the scrolled window doesn't scroll as well).
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = weak.clone();
        scroll.connect_scroll(move |ctl, _dx, dy| {
            let Some(v) = w.upgrade() else { return glib::Propagation::Proceed };
            if !ctl.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) || v.canvas.texture().is_none() {
                return glib::Propagation::Proceed;
            }
            let factor = if ctl.unit() == gdk::ScrollUnit::Surface { (-dy / 200.0).exp() } else { STEP.powf(-dy) };
            let (x, y) = v.pointer.get().unwrap_or_else(|| v.view_center());
            v.zoom_at(v.canvas.zoom() * factor, x, y);
            glib::Propagation::Stop
        });
        self.scroller.add_controller(scroll);

        // Touchpad / touchscreen pinch.
        let pinch = gtk::GestureZoom::new();
        let start = Rc::new(Cell::new(1.0));
        let (w, s) = (weak.clone(), start.clone());
        pinch.connect_begin(move |_, _| {
            if let Some(v) = w.upgrade() {
                s.set(v.canvas.zoom());
            }
        });
        let w = weak.clone();
        pinch.connect_scale_changed(move |g, scale| {
            if let Some(v) = w.upgrade() {
                let (x, y) = g.bounding_box_center().unwrap_or_else(|| v.view_center());
                v.zoom_at(start.get() * scale, x, y);
            }
        });
        self.scroller.add_controller(pinch);

        // Drag with the left mouse button to pan.
        let drag = gtk::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        let origin = Rc::new(Cell::new((0.0, 0.0)));
        let (w, o) = (weak.clone(), origin.clone());
        drag.connect_drag_begin(move |_, _, _| {
            if let Some(v) = w.upgrade() {
                o.set((v.scroller.hadjustment().value(), v.scroller.vadjustment().value()));
                v.scroller.set_cursor_from_name(Some("grabbing"));
            }
        });
        let w = weak.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            if let Some(v) = w.upgrade() {
                let (h0, v0) = origin.get();
                v.scroller.hadjustment().set_value(h0 - dx);
                v.scroller.vadjustment().set_value(v0 - dy);
            }
        });
        let w = weak.clone();
        drag.connect_drag_end(move |_, _, _| {
            if let Some(v) = w.upgrade() {
                v.scroller.set_cursor_from_name(None);
            }
        });
        self.scroller.add_controller(drag);

        // Double-click toggles fit / actual size.
        let click = gtk::GestureClick::new();
        let w = weak.clone();
        click.connect_pressed(move |_, n, x, y| {
            if n == 2 {
                if let Some(v) = w.upgrade() {
                    if v.fit.get() && (v.canvas.zoom() - 1.0).abs() > 1e-6 { v.zoom_at(1.0, x, y) } else { v.set_fit() }
                }
            }
        });
        self.scroller.add_controller(click);

        // Drop an image file onto the window.
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let w = weak.clone();
        drop.connect_drop(move |_, value, _, _| {
            let Some(v) = w.upgrade() else { return false };
            let Ok(list) = value.get::<gdk::FileList>() else { return false };
            match list.files().first().and_then(|f| f.path()) {
                Some(path) => {
                    v.load(&path);
                    true
                }
                None => false,
            }
        });
        self.window.add_controller(drop);
    }

    // --- loading ---------------------------------------------------------------------------------------

    fn open_dialog(self: &Rc<Self>) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Images"));
        filter.add_mime_type("image/*");
        filter.add_pixbuf_formats();
        let all = gtk::FileFilter::new();
        all.set_name(Some("All files"));
        all.add_pattern("*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        filters.append(&all);

        let dialog = gtk::FileDialog::builder().title("Open Image").modal(true).filters(&filters).build();
        dialog.set_default_filter(Some(&filter));
        if let Some(dir) = self.path.borrow().as_ref().and_then(|p| p.parent()) {
            dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        let weak = Rc::downgrade(self);
        dialog.open(Some(&self.window), gio::Cancellable::NONE, move |res| {
            if let (Some(v), Ok(file)) = (weak.upgrade(), res) {
                if let Some(path) = file.path() {
                    v.load(&path);
                }
            }
        });
    }

    fn load(self: &Rc<Self>, path: &Path) {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        match gdk::Texture::from_filename(&path) {
            Ok(texture) => {
                self.canvas.set_texture(Some(texture));
                self.path.replace(Some(path.clone()));
                self.siblings.replace(list_siblings(&path));
                self.pending_h.set(None);
                self.pending_v.set(None);
                self.stack.set_visible_child_name("image");
                self.files.reveal(&path);
                self.fit.set(true);
                self.apply_fit();
                self.scroller.hadjustment().set_value(0.0);
                self.scroller.vadjustment().set_value(0.0);
            }
            Err(err) => {
                self.canvas.set_texture(None);
                self.path.replace(None);
                self.siblings.replace(Vec::new());
                self.empty_text.set_text(&format!(
                    "Can't open “{}”\n{}",
                    path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
                    err.message()
                ));
                self.stack.set_visible_child_name("empty");
            }
        }
        self.update_ui();
    }

    fn step(self: &Rc<Self>, delta: isize) {
        let next = {
            let sibs = self.siblings.borrow();
            let cur = self.path.borrow();
            let Some(cur) = cur.as_ref() else { return };
            let Some(i) = sibs.iter().position(|p| p == cur) else { return };
            if sibs.len() < 2 {
                return;
            }
            sibs[(i as isize + delta).rem_euclid(sibs.len() as isize) as usize].clone()
        };
        self.load(&next);
    }

    // --- zoom ------------------------------------------------------------------------------------------

    fn view_size(&self) -> (f64, f64) {
        (self.scroller.hadjustment().page_size(), self.scroller.vadjustment().page_size())
    }

    fn view_center(&self) -> (f64, f64) {
        let (w, h) = self.view_size();
        (w / 2.0, h / 2.0)
    }

    fn fit_zoom(&self) -> Option<f64> {
        let t = self.canvas.texture()?;
        let (vw, vh) = self.view_size();
        if vw < 1.0 || vh < 1.0 {
            return None;
        }
        // Only shrink to fit: small images are shown at their real size.
        Some((vw / t.width() as f64).min(vh / t.height() as f64).clamp(MIN_ZOOM, 1.0))
    }

    fn apply_fit(&self) {
        if let Some(z) = self.fit_zoom() {
            if (z - self.canvas.zoom()).abs() > 1e-9 {
                self.canvas.set_zoom(z);
            }
        }
        self.update_zoom_label();
    }

    fn set_fit(&self) {
        self.fit.set(true);
        self.apply_fit();
        self.update_ui();
    }

    fn zoom_step(&self, factor: f64) {
        let (x, y) = self.view_center();
        self.zoom_at(self.canvas.zoom() * factor, x, y);
    }

    fn zoom_centered(&self, zoom: f64) {
        let (x, y) = self.view_center();
        self.zoom_at(zoom, x, y);
    }

    /// Zoom to `zoom`, keeping the image point under viewport position (`vx`, `vy`) where it is.
    fn zoom_at(&self, zoom: f64, vx: f64, vy: f64) {
        let Some(t) = self.canvas.texture() else { return };
        let old = self.canvas.zoom();
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        self.fit.set(false);
        if (zoom - old).abs() < 1e-9 {
            self.update_ui();
            return;
        }
        let axis = |adj: gtk::Adjustment, px: i32, vpos: f64, pending: &Cell<Option<f64>>| {
            let view = adj.page_size();
            let (old_len, new_len) = (px as f64 * old, px as f64 * zoom);
            let old_off = ((view - old_len) / 2.0).max(0.0);
            let new_off = ((view - new_len) / 2.0).max(0.0);
            let img = ((adj.value() + vpos - old_off) / old).clamp(0.0, px as f64);
            let target = (new_off + img * zoom - vpos).max(0.0);
            // The adjustment can only take the new value once the canvas has been re-measured; if its
            // range won't change, setting it now is enough.
            if (new_len.max(view) - adj.upper()).abs() > 0.5 {
                pending.set(Some(target));
            } else {
                pending.set(None);
            }
            adj.set_value(target);
        };
        self.canvas.set_zoom(zoom);
        axis(self.scroller.hadjustment(), t.width(), vx, &self.pending_h);
        axis(self.scroller.vadjustment(), t.height(), vy, &self.pending_v);
        self.update_ui();
    }

    // --- chrome ----------------------------------------------------------------------------------------

    fn update_zoom_label(&self) {
        let z = self.canvas.zoom() * 100.0;
        let text = if self.canvas.texture().is_none() {
            "—".to_string()
        } else if z < 10.0 {
            format!("{z:.1}%")
        } else {
            format!("{z:.0}%")
        };
        self.zoom_label.set_label(&text);
    }

    fn update_ui(&self) {
        let has_image = self.canvas.texture().is_some();
        for a in &self.image_actions {
            a.set_enabled(has_image);
        }
        let many = self.siblings.borrow().len() > 1;
        for a in &self.nav_actions {
            a.set_enabled(has_image && many);
        }
        self.update_zoom_label();

        let path = self.path.borrow();
        match (path.as_ref(), self.canvas.texture()) {
            (Some(p), Some(t)) => {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.title.set_text(&name);
                self.window.set_title(Some(&format!("{name} — Image Viewer")));
                let mut sub = format!("{} × {}", t.width(), t.height());
                let sibs = self.siblings.borrow();
                if let Some(i) = sibs.iter().position(|s| s == p) {
                    if sibs.len() > 1 {
                        sub.push_str(&format!("  ·  {} of {}", i + 1, sibs.len()));
                    }
                }
                self.subtitle.set_text(&sub);
                self.subtitle.set_visible(true);
            }
            _ => {
                self.title.set_text("Image Viewer");
                self.window.set_title(Some("Image Viewer"));
                self.subtitle.set_visible(false);
            }
        }
    }
}

fn icon_button(icon: &str, tooltip: &str, action: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tooltip));
    b.set_action_name(Some(action));
    b
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The images in `path`'s folder, in natural (human) name order, including `path` itself.
fn list_siblings(path: &Path) -> Vec<PathBuf> {
    let Some(dir) = path.parent() else { return vec![path.to_path_buf()] };
    let mut list: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_file() && is_image(p))
                .collect()
        })
        .unwrap_or_default();
    if !list.iter().any(|p| p == path) {
        list.push(path.to_path_buf());
    }
    list.sort_by(|a, b| natural_cmp(&a.to_string_lossy().to_lowercase(), &b.to_string_lossy().to_lowercase()));
    list
}

/// Compare strings so that "img2" sorts before "img10".
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = a.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    a.next();
                }
                let mut nb = String::new();
                while let Some(c) = b.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    b.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_theme_and_parse() {
        let p: Prefs = toml::from_str("theme = \"dark\"").unwrap();
        assert_eq!(p.background, "theme");
        assert!(p.checkerboard);
        assert!(p.is_dark(false));
        assert!(!Prefs { theme: "light".into(), ..Prefs::default() }.is_dark(true));
        assert!(Prefs::default().is_dark(true) && !Prefs::default().is_dark(false));
        let mut bad = Prefs { theme: "neon".into(), background: "plaid".into(), checkerboard: false, ..Prefs::default() };
        bad.sanitize();
        assert_eq!((bad.theme.as_str(), bad.background.as_str()), ("system", "theme"));
        assert_eq!(Prefs::default().background_css(), "");
        assert!(Prefs { background: "black".into(), ..Prefs::default() }.background_css().contains("#000000"));
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["img10.png", "img2.png", "img1.png", "a.png"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["a.png", "img1.png", "img2.png", "img10.png"]);
    }

    #[test]
    fn image_extensions() {
        assert!(is_image(Path::new("/x/Photo.JPG")));
        assert!(is_image(Path::new("a.webp")));
        assert!(!is_image(Path::new("notes.txt")));
        assert!(!is_image(Path::new("noext")));
    }
}
