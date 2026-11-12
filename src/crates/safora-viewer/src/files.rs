//! The folder panel on the left of the viewer window: browse folders and open images without a file
//! dialog (which, drawn by another program, could end up behind a fullscreen viewer).
//!
//! Click a folder to go into it and an image to show it; ↑/↓ preview the images in the list. The Up, Home
//! and Places buttons above the list jump around; Places also lists mounted drives. The list follows the
//! open image (opened any other way, or stepped to with ←/→) and refreshes when the folder changes.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    time::Duration,
};

use gtk4 as gtk;
use gtk::{gio, glib, prelude::*};

use crate::{is_image, natural_cmp};

/// One row of the list.
#[derive(Clone)]
struct Entry {
    path: PathBuf,
    is_dir: bool,
}

pub struct FilePanel {
    pub root: gtk::Box,
    list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    folder: gtk::Label,
    up: gtk::Button,
    places: gtk::Box,
    entries: RefCell<Vec<Entry>>,
    dir: RefCell<PathBuf>,
    /// The image shown in the viewer (highlighted in the list).
    current: RefCell<Option<PathBuf>>,
    /// Set while the list is being filled or selected from code, so selection changes don't open files.
    quiet: Cell<bool>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    refresh_pending: Cell<bool>,
    on_open: RefCell<Option<Box<dyn Fn(&Path)>>>,
    on_dir: RefCell<Option<Box<dyn Fn(&Path)>>>,
}

impl FilePanel {
    pub fn new(start: &Path) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("viewer-files");
        root.set_width_request(180);

        // Tool row: up, home, places, then the folder name.
        let tools = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        tools.set_margin_top(6);
        tools.set_margin_bottom(6);
        tools.set_margin_start(6);
        tools.set_margin_end(6);
        let up = gtk::Button::from_icon_name("go-up-symbolic");
        up.set_tooltip_text(Some("Up to the parent folder (Alt+↑)"));
        up.add_css_class("flat");
        let home = gtk::Button::from_icon_name("user-home-symbolic");
        home.set_tooltip_text(Some("Home folder"));
        home.add_css_class("flat");
        let places = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let places_pop = gtk::Popover::new();
        places_pop.set_child(Some(&places));
        let places_btn = gtk::MenuButton::builder()
            .icon_name("drive-harddisk-symbolic")
            .tooltip_text("Places and drives")
            .popover(&places_pop)
            .has_frame(false)
            .build();
        let folder = gtk::Label::new(None);
        folder.set_xalign(0.0);
        folder.set_hexpand(true);
        folder.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        folder.add_css_class("heading");
        folder.set_margin_start(4);
        tools.append(&up);
        tools.append(&home);
        tools.append(&places_btn);
        tools.append(&folder);
        root.append(&tools);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.set_activate_on_single_click(true);
        list.add_css_class("navigation-sidebar");
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        root.append(&scroller);

        let panel = Rc::new(Self {
            root,
            list,
            scroller,
            folder,
            up,
            places,
            entries: RefCell::new(Vec::new()),
            dir: RefCell::new(PathBuf::new()),
            current: RefCell::new(None),
            quiet: Cell::new(false),
            monitor: RefCell::new(None),
            refresh_pending: Cell::new(false),
            on_open: RefCell::new(None),
            on_dir: RefCell::new(None),
        });

        let w = Rc::downgrade(&panel);
        panel.up.connect_clicked(move |_| with(&w, |p| p.go_up()));
        let w = Rc::downgrade(&panel);
        home.connect_clicked(move |_| with(&w, |p| p.set_dir(&glib::home_dir())));
        let w = Rc::downgrade(&panel);
        places_pop.connect_show(move |pop| with(&w, |p| p.fill_places(pop)));

        // Click / Enter: folders are entered, images opened.
        let w = Rc::downgrade(&panel);
        panel.list.connect_row_activated(move |_, row| {
            with(&w, |p| {
                let Some(e) = p.entries.borrow().get(row.index() as usize).cloned() else { return };
                if e.is_dir {
                    p.set_dir(&e.path);
                } else {
                    p.open(&e.path);
                }
            })
        });
        // Moving through the list with the keyboard previews each image.
        let w = Rc::downgrade(&panel);
        panel.list.connect_row_selected(move |_, row| {
            with(&w, |p| {
                if p.quiet.get() {
                    return;
                }
                let Some(row) = row else { return };
                let Some(e) = p.entries.borrow().get(row.index() as usize).cloned() else { return };
                if !e.is_dir && p.current.borrow().as_deref() != Some(e.path.as_path()) {
                    p.open(&e.path);
                }
            })
        });
        // Alt+↑ goes to the parent folder.
        let keys = gtk::EventControllerKey::new();
        let w = Rc::downgrade(&panel);
        keys.connect_key_pressed(move |_, key, _, state| {
            if key == gtk::gdk::Key::Up && state.contains(gtk::gdk::ModifierType::ALT_MASK) {
                with(&w, |p| p.go_up());
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        panel.root.add_controller(keys);

        panel.set_dir(start);
        panel
    }

    /// Called with the image to show when one is picked in the list.
    pub fn connect_open(&self, f: impl Fn(&Path) + 'static) {
        self.on_open.replace(Some(Box::new(f)));
    }

    /// Called with the new folder whenever the panel changes folder.
    pub fn connect_dir_changed(&self, f: impl Fn(&Path) + 'static) {
        self.on_dir.replace(Some(Box::new(f)));
    }

    fn open(&self, path: &Path) {
        if let Some(f) = self.on_open.borrow().as_ref() {
            f(path);
        }
    }

    fn go_up(self: &Rc<Self>) {
        let parent = self.dir.borrow().parent().map(Path::to_path_buf);
        if let Some(parent) = parent {
            let child = self.dir.borrow().clone();
            self.set_dir(&parent);
            // Keep the folder we came from selected, so going back down is one click away.
            self.select_path(&child, true);
        }
    }

    /// Show the folder `dir` (falls back to the nearest existing parent, then the home folder).
    pub fn set_dir(self: &Rc<Self>, dir: &Path) {
        let mut dir = dir.to_path_buf();
        while !dir.is_dir() {
            match dir.parent() {
                Some(p) => dir = p.to_path_buf(),
                None => {
                    dir = glib::home_dir();
                    break;
                }
            }
        }
        let changed = *self.dir.borrow() != dir;
        self.dir.replace(dir.clone());
        self.refresh();
        self.scroller.vadjustment().set_value(0.0);
        if changed {
            self.watch(&dir);
            if let Some(f) = self.on_dir.borrow().as_ref() {
                f(&dir);
            }
        }
    }

    /// Follow the image the viewer now shows: go to its folder and highlight it.
    pub fn reveal(self: &Rc<Self>, image: &Path) {
        self.current.replace(Some(image.to_path_buf()));
        if let Some(parent) = image.parent() {
            if *self.dir.borrow() != parent {
                self.set_dir(parent);
            }
        }
        self.select_path(image, true);
    }

    /// Re-read the folder, keeping the highlighted image.
    fn refresh(self: &Rc<Self>) {
        let dir = self.dir.borrow().clone();
        self.folder.set_text(&display_name(&dir));
        self.folder.set_tooltip_text(Some(&dir.to_string_lossy()));
        self.up.set_sensitive(dir.parent().is_some());

        let entries = read_folder(&dir);
        self.quiet.set(true);
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        if entries.is_empty() {
            let l = gtk::Label::new(Some("No images or folders here"));
            l.add_css_class("dim-label");
            l.set_margin_top(12);
            l.set_margin_bottom(12);
            let row = gtk::ListBoxRow::builder().child(&l).activatable(false).selectable(false).build();
            self.list.append(&row);
        }
        for e in &entries {
            self.list.append(&entry_row(e));
        }
        self.entries.replace(entries);
        self.quiet.set(false);
        if let Some(cur) = self.current.borrow().clone() {
            self.select_path(&cur, false);
        }
    }

    /// Select the row for `path` (without opening it), optionally scrolling it into view.
    fn select_path(self: &Rc<Self>, path: &Path, scroll: bool) {
        let idx = self.entries.borrow().iter().position(|e| e.path == path);
        self.quiet.set(true);
        match idx.and_then(|i| self.list.row_at_index(i as i32)) {
            Some(row) => {
                self.list.select_row(Some(&row));
                if scroll {
                    // After layout, so the row has its final position.
                    let (list, sc) = (self.list.clone(), self.scroller.clone());
                    glib::idle_add_local_once(move || {
                        if let Some(b) = row.compute_bounds(&list) {
                            let adj = sc.vadjustment();
                            let (top, bottom) = (b.y() as f64, (b.y() + b.height()) as f64);
                            if top < adj.value() {
                                adj.set_value(top);
                            } else if bottom > adj.value() + adj.page_size() {
                                adj.set_value(bottom - adj.page_size());
                            }
                        }
                    });
                }
            }
            None => self.list.unselect_all(),
        }
        self.quiet.set(false);
    }

    /// Refresh (a moment later, once a burst of changes is over) when files come and go in the folder.
    fn watch(self: &Rc<Self>, dir: &Path) {
        if let Some(old) = self.monitor.take() {
            old.cancel();
        }
        let Ok(monitor) = gio::File::for_path(dir).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        else {
            return;
        };
        let w = Rc::downgrade(self);
        monitor.connect_changed(move |_, _, _, event| {
            if matches!(event, gio::FileMonitorEvent::ChangesDoneHint | gio::FileMonitorEvent::AttributeChanged) {
                return;
            }
            with(&w, |p| {
                if p.refresh_pending.replace(true) {
                    return;
                }
                let w = Rc::downgrade(p);
                glib::timeout_add_local_once(Duration::from_millis(400), move || {
                    with(&w, |p| {
                        p.refresh_pending.set(false);
                        p.refresh();
                    })
                });
            })
        });
        self.monitor.replace(Some(monitor));
    }

    /// The Places popover: the usual folders, the file system root and mounted drives.
    fn fill_places(self: &Rc<Self>, pop: &gtk::Popover) {
        while let Some(child) = self.places.first_child() {
            self.places.remove(&child);
        }
        let mut items: Vec<(String, &'static str, PathBuf)> = vec![("Home".into(), "user-home-symbolic", glib::home_dir())];
        for (dir, icon) in [
            (glib::UserDirectory::Pictures, "folder-pictures-symbolic"),
            (glib::UserDirectory::Desktop, "user-desktop-symbolic"),
            (glib::UserDirectory::Downloads, "folder-download-symbolic"),
            (glib::UserDirectory::Documents, "folder-documents-symbolic"),
        ] {
            if let Some(p) = glib::user_special_dir(dir).filter(|p| p.is_dir() && *p != glib::home_dir()) {
                items.push((display_name(&p), icon, p));
            }
        }
        items.push(("File System".into(), "drive-harddisk-symbolic", PathBuf::from("/")));
        for mount in gio::VolumeMonitor::get().mounts() {
            if let Some(p) = mount.root().path() {
                items.push((mount.name().to_string(), "drive-removable-media-symbolic", p));
            }
        }
        for (name, icon, path) in items {
            let b = gtk::Button::new();
            b.add_css_class("flat");
            let inner = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            inner.append(&gtk::Image::from_icon_name(icon));
            let l = gtk::Label::new(Some(&name));
            l.set_xalign(0.0);
            inner.append(&l);
            b.set_child(Some(&inner));
            b.set_tooltip_text(Some(&path.to_string_lossy()));
            let (w, pop) = (Rc::downgrade(self), pop.clone());
            b.connect_clicked(move |_| {
                pop.popdown();
                with(&w, |p| p.set_dir(&path));
            });
            self.places.append(&b);
        }
    }
}

fn with(w: &Weak<FilePanel>, f: impl FnOnce(&Rc<FilePanel>)) {
    if let Some(p) = w.upgrade() {
        f(&p);
    }
}

fn display_name(dir: &Path) -> String {
    if dir == glib::home_dir() {
        return "Home".into();
    }
    dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.to_string_lossy().into_owned())
}

/// Folders first, then images, each in natural name order; hidden entries are left out.
fn read_folder(dir: &Path) -> Vec<Entry> {
    let mut entries: Vec<Entry> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .filter_map(|e| {
                    let path = e.path();
                    // Follows symlinks, so linked folders and images show up too.
                    let is_dir = path.is_dir();
                    (is_dir || (path.is_file() && is_image(&path))).then_some(Entry { path, is_dir })
                })
                .collect()
        })
        .unwrap_or_default();
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| {
            let name = |e: &Entry| e.path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            natural_cmp(&name(a), &name(b))
        })
    });
    entries
}

fn entry_row(e: &Entry) -> gtk::ListBoxRow {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    b.append(&gtk::Image::from_icon_name(if e.is_dir { "folder-symbolic" } else { "image-x-generic-symbolic" }));
    let name = e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let l = gtk::Label::new(Some(&name));
    l.set_xalign(0.0);
    l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    l.set_hexpand(true);
    b.append(&l);
    let row = gtk::ListBoxRow::builder().child(&b).build();
    row.set_tooltip_text(Some(&name));
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_listing_order_and_filter() {
        let dir = std::env::temp_dir().join(format!("safora-viewer-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Zeta")).unwrap();
        std::fs::create_dir_all(dir.join("alpha")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        for f in ["img10.png", "img2.JPG", "notes.txt", ".secret.png"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let names: Vec<String> =
            read_folder(&dir).iter().map(|e| e.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["alpha", "Zeta", "img2.JPG", "img10.png"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
