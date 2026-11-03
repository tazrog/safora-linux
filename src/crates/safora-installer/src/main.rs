//! safora-installer — graphical installer for Safora Linux.
//!
//! This is a thin GTK4 front end. All disk work happens in `safora-install-backend` (a root shell
//! script) which receives the answers as JSON on stdin so secrets never appear in `ps` or the
//! environment.

use std::{
    cell::RefCell,
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    rc::Rc,
};

use gtk4 as gtk;
use gtk::{glib, prelude::*};
use serde::Serialize;

const BACKEND: &str = "/usr/lib/safora/safora-install-backend";

#[derive(Serialize, Default, Clone)]
struct Plan {
    disk: String,
    encrypt: bool,
    passphrase: String,
    hostname: String,
    username: String,
    fullname: String,
    password: String,
    timezone: String,
    keyboard: String,
    uefi: bool,
    claude_code: bool,
    claude_web: bool,
    /// "none", "quick" (discard every block) or "full" (one pass of random data).
    wipe: String,
}

#[derive(Debug, Clone)]
struct Disk {
    path: String,
    label: String,
}

// ---------------------------------------------------------------- validation

pub fn valid_username(u: &str) -> bool {
    let mut chars = u.chars();
    matches!(chars.next(), Some('a'..='z' | '_'))
        && u.len() <= 32
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-'))
        && !["root", "live", "nobody", "daemon", "bin", "sys"].contains(&u)
}

pub fn valid_hostname(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 63
        && !h.starts_with('-')
        && !h.ends_with('-')
        && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// 0 (very weak) … 4 (strong). Length and character variety, with a small penalty for repeats.
pub fn passphrase_score(p: &str) -> u8 {
    let classes = [
        p.chars().any(|c| c.is_ascii_lowercase()),
        p.chars().any(|c| c.is_ascii_uppercase()),
        p.chars().any(|c| c.is_ascii_digit()),
        p.chars().any(|c| !c.is_ascii_alphanumeric()),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    let distinct = p.chars().collect::<std::collections::HashSet<_>>().len();
    let mut score = match p.chars().count() {
        0..=7 => 0,
        8..=11 => 1,
        12..=15 => 2,
        16..=19 => 3,
        _ => 4,
    };
    if classes <= 1 && score > 0 {
        score -= 1;
    }
    if distinct < 5 {
        score = score.min(1);
    }
    score
}

// ------------------------------------------------------------------- system

/// Device the live system was booted from ("" if unknown). With enough RAM archiso copies the system
/// to RAM and unmounts the medium (copytoram=auto), so fall back to the ISO's UUID on the kernel command line.
fn boot_medium() -> String {
    let run = |cmd: &str, args: &[&str]| {
        Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    let src = run("findmnt", &["-no", "SOURCE", "/run/archiso/bootmnt"]);
    if !src.is_empty() {
        return src;
    }
    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    match cmdline.split_whitespace().find_map(|a| a.strip_prefix("archisosearchuuid=")) {
        Some(uuid) if !uuid.is_empty() => run("blkid", &["-U", uuid]),
        _ => String::new(),
    }
}

fn list_disks() -> Vec<Disk> {
    let out = Command::new("lsblk").args(["-J", "-d", "-b", "-o", "NAME,SIZE,MODEL,TYPE,RO"]).output();
    let Ok(out) = out else { return vec![] };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else { return vec![] };

    // Never offer the medium we booted from.
    let boot_src = boot_medium();

    v["blockdevices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["type"] == "disk" && d["ro"] != true && d["ro"] != "1")
        .filter_map(|d| {
            let name = d["name"].as_str()?;
            if ["zram", "loop", "ram", "sr", "fd", "nbd"].iter().any(|p| name.starts_with(p)) {
                return None;
            }
            if !boot_src.is_empty() && boot_src.starts_with(&format!("/dev/{name}")) {
                return None;
            }
            let size = d["size"].as_u64().or_else(|| d["size"].as_str()?.parse().ok()).unwrap_or(0);
            let model = d["model"].as_str().unwrap_or("Disk").trim();
            Some(Disk {
                path: format!("/dev/{name}"),
                label: format!("{model}  ·  {:.1} GB  ·  /dev/{name}", size as f64 / 1e9),
            })
        })
        .collect()
}

fn timezones() -> Vec<String> {
    Command::new("timedatectl")
        .arg("list-timezones")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(String::from).collect())
        .filter(|v: &Vec<String>| !v.is_empty())
        .unwrap_or_else(|| vec!["UTC".into()])
}

fn keymaps() -> Vec<String> {
    // XKB layout names (what the compositor uses), from the xkb rules file.
    let text = std::fs::read_to_string("/usr/share/X11/xkb/rules/base.lst").unwrap_or_default();
    let mut in_layouts = false;
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with('!') {
            in_layouts = line.trim() == "! layout";
            continue;
        }
        if in_layouts {
            if let Some(code) = line.split_whitespace().next() {
                out.push(code.to_string());
            }
        }
    }
    if out.is_empty() {
        out.push("us".into());
    }
    out
}

// ---------------------------------------------------------------------- UI

struct Page {
    title: &'static str,
    subtitle: &'static str,
    body: gtk::Widget,
    /// Returns an error message if the page can't be left yet.
    validate: Box<dyn Fn() -> Result<(), String>>,
}

fn field(label: &str, w: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let l = gtk::Label::new(Some(label));
    l.set_xalign(0.0);
    l.add_css_class("dim-label");
    b.append(&l);
    b.append(w);
    b
}

fn vbox() -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, 14)
}

fn dropdown_from(items: &[String], select: &str) -> gtk::DropDown {
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    let d = gtk::DropDown::from_strings(&refs);
    // Search needs an expression that yields each item's text; without it typing filters nothing.
    d.set_expression(Some(gtk::PropertyExpression::new(gtk::StringObject::static_type(), gtk::Expression::NONE, "string")));
    // Match anywhere ("berlin" finds Europe/Berlin), not only at the start.
    d.set_search_match_mode(gtk::StringFilterMatchMode::Substring);
    if let Some(pos) = items.iter().position(|i| i == select) {
        d.set_selected(pos as u32);
    }
    d
}

fn selected_string(d: &gtk::DropDown) -> String {
    d.selected_item().and_downcast::<gtk::StringObject>().map(|s| s.string().to_string()).unwrap_or_default()
}

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder().application_id("org.safora.Installer").build();
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &gtk::Application) {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(true);
    }
    let uefi = std::path::Path::new("/sys/firmware/efi").exists();
    let disks = Rc::new(list_disks());

    // ---- Page: welcome
    let welcome = vbox();
    let intro = gtk::Label::new(Some(
        "Safora Linux is a private, security-focused Arch Linux system with the Safora tiling desktop.\n\n\
         This installer will:\n\
         •  erase the disk you choose and format it as ext4\n\
         •  optionally encrypt it with LUKS2 (strongly recommended)\n\
         •  create your user account and lock the root account\n\n\
         Nothing is changed on your disk until you confirm on the last page.",
    ));
    intro.set_wrap(true);
    intro.set_xalign(0.0);
    welcome.append(&intro);
    let mode = gtk::Label::new(Some(&format!("Firmware: {}", if uefi { "UEFI" } else { "BIOS (legacy)" })));
    mode.set_xalign(0.0);
    mode.add_css_class("dim-label");
    welcome.append(&mode);
    let tzs = timezones();
    let tz = dropdown_from(&tzs, "UTC");
    tz.set_enable_search(true);
    welcome.append(&field("Time zone", &tz));
    let kms = keymaps();
    let km = dropdown_from(&kms, "us");
    km.set_enable_search(true);
    welcome.append(&field("Keyboard layout", &km));

    // ---- Page: disk
    let disk_page = vbox();
    let disk_list = gtk::ListBox::new();
    disk_list.set_selection_mode(gtk::SelectionMode::Single);
    disk_list.add_css_class("boxed-list");
    for d in disks.iter() {
        let l = gtk::Label::new(Some(&d.label));
        l.set_xalign(0.0);
        l.set_margin_top(10);
        l.set_margin_bottom(10);
        l.set_margin_start(12);
        disk_list.append(&l);
    }
    if disks.is_empty() {
        let l = gtk::Label::new(Some("No installable disks were found."));
        l.set_margin_top(10);
        disk_page.append(&l);
    }
    if let Some(first) = disk_list.row_at_index(0) {
        disk_list.select_row(Some(&first));
    }
    disk_page.append(&disk_list);
    let warn = gtk::Label::new(Some("⚠  Everything on the selected disk will be permanently erased."));
    warn.set_xalign(0.0);
    disk_page.append(&warn);
    let wipe_modes = [
        "Normal: remove the old partitions (fast)",
        "Quick secure erase: discard every block (fast; best for SSD/NVMe)",
        "Full secure erase: overwrite the whole drive once with random data (slow; best for hard disks)",
    ];
    let wipe_values = ["none", "quick", "full"];
    let wipe = gtk::DropDown::from_strings(&wipe_modes);
    disk_page.append(&field("Erase the drive before installing", &wipe));
    let wipe_note = gtk::Label::new(Some(
        "A full overwrite can take from minutes to many hours depending on the drive's size and speed. Quick erase asks the drive to discard everything; \
         drives that cannot do a secure erase only get an ordinary discard, so use the full overwrite when old data must not be recoverable.",
    ));
    wipe_note.set_wrap(true);
    wipe_note.set_xalign(0.0);
    wipe_note.add_css_class("dim-label");
    disk_page.append(&wipe_note);

    // ---- Page: encryption
    let enc_page = vbox();
    let enc_switch = gtk::Switch::new();
    enc_switch.set_active(true);
    enc_switch.set_valign(gtk::Align::Center);
    let enc_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let enc_label = gtk::Label::new(Some("Encrypt the disk (LUKS2, AES-256, argon2id)"));
    enc_label.set_hexpand(true);
    enc_label.set_xalign(0.0);
    enc_row.append(&enc_label);
    enc_row.append(&enc_switch);
    enc_page.append(&enc_row);
    let pass1 = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    let pass2 = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    let strength = gtk::LevelBar::for_interval(0.0, 4.0);
    strength.set_mode(gtk::LevelBarMode::Continuous);
    let strength_text = gtk::Label::new(None);
    strength_text.set_xalign(0.0);
    let enc_fields = vbox();
    enc_fields.append(&field("Encryption passphrase", &pass1));
    enc_fields.append(&strength);
    enc_fields.append(&strength_text);
    enc_fields.append(&field("Repeat passphrase", &pass2));
    let note = gtk::Label::new(Some(
        "You will type this passphrase every time the computer starts. If you forget it, \
         your data cannot be recovered — there is no backdoor.\n\n\
         Note: a small unencrypted /boot partition holds the kernel; everything else is encrypted.",
    ));
    note.set_wrap(true);
    note.set_xalign(0.0);
    note.add_css_class("dim-label");
    enc_fields.append(&note);
    enc_page.append(&enc_fields);
    {
        let (st, stt) = (strength.clone(), strength_text.clone());
        pass1.connect_changed(move |e| {
            let s = passphrase_score(&e.text());
            st.set_value(s as f64);
            stt.set_text(["Too short", "Weak", "Fair", "Good", "Strong"][s as usize]);
        });
        let f = enc_fields.clone();
        enc_switch.connect_active_notify(move |s| f.set_sensitive(s.is_active()));
    }

    // ---- Page: account
    let acct = vbox();
    let full = gtk::Entry::new();
    let user = gtk::Entry::new();
    let host = gtk::Entry::new();
    host.set_text("safora");
    let up1 = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    let up2 = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    acct.append(&field("Your name", &full));
    acct.append(&field("Username", &user));
    acct.append(&field("Computer name", &host));
    acct.append(&field("Password", &up1));
    acct.append(&field("Repeat password", &up2));
    {
        let user = user.clone();
        full.connect_changed(move |e| {
            if !user.is_focus() {
                let guess: String = e.text().to_lowercase().chars().filter(|c| c.is_ascii_lowercase()).collect();
                user.set_text(&guess);
            }
        });
    }

    // ---- Page: confirm / progress / done
    let summary = gtk::Label::new(None);
    summary.set_wrap(true);
    summary.set_xalign(0.0);
    summary.set_selectable(false);
    let confirm = gtk::CheckButton::with_label("I understand that the selected disk will be erased");
    let confirm_page = vbox();
    confirm_page.append(&summary);
    confirm_page.append(&confirm);

    let progress = gtk::ProgressBar::new();
    progress.set_show_text(true);
    let status = gtk::Label::new(Some("Preparing…"));
    status.set_xalign(0.0);
    let log = gtk::TextView::new();
    log.set_editable(false);
    log.set_monospace(true);
    let log_sw = gtk::ScrolledWindow::builder().child(&log).vexpand(true).min_content_height(220).build();
    let run_page = vbox();
    run_page.append(&status);
    run_page.append(&progress);
    run_page.append(&log_sw);

    let done = gtk::Label::new(Some(
        "Safora is installed.\n\nRemove the installation medium, then restart.\n\
         If you encrypted the disk, you will be asked for your passphrase at startup.",
    ));
    done.set_wrap(true);
    done.set_xalign(0.0);
    done.set_valign(gtk::Align::Start);

    // ---- Assemble
    // ---- Page: connect (network + Bluetooth)
    let connect_box = vbox();
    connect_box.append(&safora_panels::network_panel());
    connect_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    connect_box.append(&safora_panels::bluetooth_panel());
    let connect_scroll = gtk::ScrolledWindow::builder().child(&connect_box).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();

    // ---- Page: extra software
    let extras = vbox();
    let claude_code = gtk::CheckButton::with_label("Install Claude Code (Anthropic's command-line coding assistant)");
    let claude_web = gtk::CheckButton::with_label("Add a Claude shortcut (claude.ai in Firefox) to the launcher");
    let claude_note = gtk::Label::new(Some(
        "Claude is an online service from Anthropic: what you type to it is sent over the internet, and you need your own Anthropic account. \
         Installing Claude Code downloads it from Anthropic (claude.ai), so it needs a working network connection on the previous page. \
         Nothing is installed unless you tick a box, and you can install it later with the command  safora-install-claude.",
    ));
    claude_note.set_wrap(true);
    claude_note.set_xalign(0.0);
    claude_note.add_css_class("dim-label");
    extras.append(&claude_code);
    extras.append(&claude_web);
    extras.append(&claude_note);

    let plan = Rc::new(RefCell::new(Plan { uefi, ..Default::default() }));

    let pages: Vec<Page> = vec![
        Page { title: "Welcome to Safora Linux", subtitle: "Simple by Nature, Private by default", body: welcome.upcast(), validate: Box::new(|| Ok(())) },
        Page {
            title: "Network and Bluetooth",
            subtitle: "Optional: connect to Wi-Fi (needed for online extras) and pair a keyboard or mouse.",
            body: connect_scroll.upcast(),
            validate: Box::new(|| Ok(())),
        },
        Page {
            title: "Choose a disk",
            subtitle: "Safora will use the whole disk.",
            body: disk_page.upcast(),
            validate: Box::new({
                let (list, disks) = (disk_list.clone(), disks.clone());
                move || {
                    if list.selected_row().is_some() && !disks.is_empty() { Ok(()) } else { Err("Select a disk to continue.".into()) }
                }
            }),
        },
        Page {
            title: "Disk encryption",
            subtitle: "Protect your files if your computer is lost or stolen.",
            body: enc_page.upcast(),
            validate: Box::new({
                let (sw, p1, p2) = (enc_switch.clone(), pass1.clone(), pass2.clone());
                move || {
                    if !sw.is_active() {
                        return Ok(());
                    }
                    let (a, b) = (p1.text().to_string(), p2.text().to_string());
                    if a.chars().count() < 8 {
                        Err("The passphrase must be at least 8 characters.".into())
                    } else if a != b {
                        Err("The passphrases do not match.".into())
                    } else {
                        Ok(())
                    }
                }
            }),
        },
        Page {
            title: "Create your account",
            subtitle: "The root account will be locked; you administer the system with sudo.",
            body: acct.upcast(),
            validate: Box::new({
                let (u, h, p1, p2) = (user.clone(), host.clone(), up1.clone(), up2.clone());
                move || {
                    if !valid_username(&u.text()) {
                        Err("Username: lowercase letters, digits, - and _ only, starting with a letter.".into())
                    } else if !valid_hostname(&h.text()) {
                        Err("Computer name: letters, digits and - only.".into())
                    } else if p1.text().chars().count() < 6 {
                        Err("The password must be at least 6 characters.".into())
                    } else if p1.text() != p2.text() {
                        Err("The passwords do not match.".into())
                    } else {
                        Ok(())
                    }
                }
            }),
        },
        Page {
            title: "Extra software",
            subtitle: "Everything here is optional.",
            body: extras.upcast(),
            validate: Box::new(|| Ok(())),
        },
        Page {
            title: "Ready to install",
            subtitle: "Review your choices.",
            body: confirm_page.upcast(),
            validate: Box::new({
                let c = confirm.clone();
                move || if c.is_active() { Ok(()) } else { Err("Tick the box to confirm.".into()) }
            }),
        },
        Page { title: "Installing", subtitle: "This usually takes a few minutes.", body: run_page.upcast(), validate: Box::new(|| Ok(())) },
        Page { title: "All done", subtitle: "", body: done.upcast(), validate: Box::new(|| Ok(())) },
    ];
    let capture_page = 5usize; // leaving "Extra software" captures every answer
    let last_input_page = 6usize;
    let run_idx = 7usize;
    let done_idx = 8usize;

    let win = gtk::ApplicationWindow::builder().application(app).title("Install Safora").default_width(720).default_height(560).build();
    let title = gtk::Label::new(None);
    title.add_css_class("title-1");
    title.set_xalign(0.0);
    let subtitle = gtk::Label::new(None);
    subtitle.set_xalign(0.0);
    subtitle.add_css_class("dim-label");
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    for (i, p) in pages.iter().enumerate() {
        stack.add_named(&p.body, Some(&i.to_string()));
    }
    let error = gtk::Label::new(None);
    error.set_xalign(0.0);
    error.add_css_class("error");
    let back = gtk::Button::with_label("Back");
    let next = gtk::Button::with_label("Next");
    next.add_css_class("suggested-action");
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&back);
    buttons.append(&next);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    for m in [root.clone()] {
        m.set_margin_top(24);
        m.set_margin_bottom(20);
        m.set_margin_start(32);
        m.set_margin_end(32);
    }
    root.append(&title);
    root.append(&subtitle);
    root.append(&stack);
    root.append(&error);
    root.append(&buttons);
    win.set_child(Some(&root));

    let current = Rc::new(RefCell::new(0usize));
    let pages = Rc::new(pages);

    let show = {
        let (stack, title, subtitle, back, next, error, pages, current) =
            (stack.clone(), title.clone(), subtitle.clone(), back.clone(), next.clone(), error.clone(), pages.clone(), current.clone());
        Rc::new(move |i: usize| {
            *current.borrow_mut() = i;
            stack.set_visible_child_name(&i.to_string());
            title.set_text(pages[i].title);
            subtitle.set_text(pages[i].subtitle);
            error.set_text("");
            back.set_visible(i > 0 && i < run_idx);
            next.set_visible(i != run_idx);
            next.set_label(match i {
                x if x == last_input_page => "Install",
                x if x == done_idx => "Restart now",
                _ => "Next",
            });
        })
    };
    show(0);

    back.connect_clicked({
        let (show, current) = (show.clone(), current.clone());
        move |_| {
            let i = *current.borrow();
            if i > 0 {
                show(i - 1);
            }
        }
    });

    next.connect_clicked({
        let (show, current, pages, error, plan, win) = (show.clone(), current.clone(), pages.clone(), error.clone(), plan.clone(), win.clone());
        let (disk_list, disks, enc_switch, pass1, tz, km, full, user, host, up1, claude_code, claude_web, wipe) =
            (disk_list.clone(), disks.clone(), enc_switch.clone(), pass1.clone(), tz.clone(), km.clone(), full.clone(), user.clone(), host.clone(), up1.clone(), claude_code.clone(), claude_web.clone(), wipe.clone());
        let (summary, progress, status, log, done_label_next) = (summary.clone(), progress.clone(), status.clone(), log.clone(), next.clone());
        move |_| {
            let i = *current.borrow();
            if i == done_idx {
                let _ = Command::new("systemctl").arg("reboot").spawn();
                return;
            }
            if let Err(msg) = (pages[i].validate)() {
                error.set_text(&msg);
                return;
            }
            error.set_text("");

            // Capture answers as the user moves forward, and show the summary before the final confirm.
            if i == capture_page {
                let disk = disk_list.selected_row().and_then(|r| disks.get(r.index() as usize)).cloned();
                let mut p = plan.borrow_mut();
                p.disk = disk.as_ref().map(|d| d.path.clone()).unwrap_or_default();
                p.encrypt = enc_switch.is_active();
                p.passphrase = pass1.text().to_string();
                p.wipe = wipe_values.get(wipe.selected() as usize).copied().unwrap_or("none").to_string();
                p.claude_code = claude_code.is_active();
                p.claude_web = claude_web.is_active();
                p.timezone = selected_string(&tz);
                p.keyboard = selected_string(&km);
                p.fullname = full.text().to_string();
                p.username = user.text().to_string();
                p.hostname = host.text().to_string();
                p.password = up1.text().to_string();
                summary.set_markup(&format!(
                    "<b>Disk</b>\n{}\n{}\n\n<b>Encryption</b>\n{}\n\n<b>Account</b>\n{} ({}) on computer “{}”\n\n<b>Locale</b>\n{} · keyboard {}\n\n\
                     <b>Extras</b>\n{}\n\nPartitions: {} /boot (ext4) and an ext4 root{}.",
                    glib::markup_escape_text(disk.as_ref().map(|d| d.label.as_str()).unwrap_or("?")),
                    match p.wipe.as_str() {
                        "quick" => "Secure erase first: discard every block",
                        "full" => "Secure erase first: overwrite the whole drive with random data (slow)",
                        _ => "No extra erase",
                    },
                    if p.encrypt { "LUKS2 full-disk encryption" } else { "None" },
                    glib::markup_escape_text(&p.fullname),
                    glib::markup_escape_text(&p.username),
                    glib::markup_escape_text(&p.hostname),
                    glib::markup_escape_text(&p.timezone),
                    glib::markup_escape_text(&p.keyboard),
                    match (p.claude_code, p.claude_web) {
                        (true, true) => "Claude Code and Claude web shortcut",
                        (true, false) => "Claude Code",
                        (false, true) => "Claude web shortcut",
                        _ => "None",
                    },
                    if p.uefi { "EFI system (FAT32, required by UEFI)," } else { "BIOS boot," },
                    if p.encrypt { " inside the encrypted container" } else { "" },
                ));
            }

            if i == last_input_page {
                show(run_idx);
                run_install(plan.borrow().clone(), &progress, &status, &log, &done_label_next, &show, done_idx, &win);
            } else {
                show(i + 1);
            }
        }
    });

    // Keyboard navigation: Alt+Right / Alt+Left move between pages.
    {
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_scope(gtk::ShortcutScope::Global);
        for (keys, button) in [("<Alt>Right", next.clone()), ("<Alt>Left", back.clone())] {
            if let Some(trigger) = gtk::ShortcutTrigger::parse_string(keys) {
                let action = gtk::CallbackAction::new(move |_, _| {
                    if button.is_visible() && button.is_sensitive() {
                        button.emit_clicked();
                    }
                    glib::Propagation::Stop
                });
                shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
            }
        }
        win.add_controller(shortcuts);
    }

    win.present();

    // Development aid: jump straight to the progress page with a dummy plan.
    if std::env::var_os("SAFORA_INSTALLER_SELFTEST").is_some() {
        show(run_idx);
        run_install(Plan::default(), &progress, &status, &log, &next, &show, done_idx, &win);
    }
}

#[allow(clippy::too_many_arguments)]
fn run_install(
    plan: Plan,
    progress: &gtk::ProgressBar,
    status: &gtk::Label,
    log: &gtk::TextView,
    _next: &gtk::Button,
    show: &Rc<impl Fn(usize) + 'static>,
    done_idx: usize,
    win: &gtk::ApplicationWindow,
) {
    // Don't let the window be closed mid-install.
    win.set_deletable(false);
    let (tx, rx) = async_channel::unbounded::<String>();
    let json = serde_json::to_string(&plan).unwrap_or_default();
    std::thread::spawn(move || {
        let child = Command::new("sudo")
            .args(["-n", "nice", "-n", "10", BACKEND])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send_blocking(format!("ERROR could not start the installer backend: {e}"));
                return;
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(json.as_bytes());
        }
        // stderr is forwarded as log lines.
        if let Some(err) = child.stderr.take() {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for l in BufReader::new(err).lines().map_while(Result::ok) {
                    let _ = tx.send_blocking(format!("LOG {l}"));
                }
            });
        }
        if let Some(out) = child.stdout.take() {
            for l in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send_blocking(l);
            }
        }
        let ok = child.wait().map(|s| s.success()).unwrap_or(false);
        let _ = tx.send_blocking(if ok { "DONE".into() } else { "FAILED".into() });
    });

    let (progress, status, log, show, win) = (progress.clone(), status.clone(), log.clone(), show.clone(), win.clone());
    glib::spawn_future_local(async move {
        let buf = log.buffer();
        let end_mark = buf.create_mark(Some("end"), &buf.end_iter(), false);
        let mut failed_msg: Option<String> = None;
        loop {
            // Take whatever has arrived, so a burst of messages costs one UI update, not thousands.
            let Ok(first) = rx.recv().await else { return };
            let mut batch = vec![first];
            while batch.len() < 500 {
                match rx.try_recv() {
                    Ok(l) => batch.push(l),
                    Err(_) => break,
                }
            }

            let mut text = String::new();
            let mut finished: Option<bool> = None;
            for line in batch {
                if let Some(rest) = line.strip_prefix("PROGRESS ") {
                    let (pct, msg) = rest.split_once(' ').unwrap_or((rest, ""));
                    if let Ok(p) = pct.parse::<f64>() {
                        progress.set_fraction(p / 100.0);
                        progress.set_text(Some(&format!("{p:.0}%")));
                    }
                    status.set_text(msg);
                    // Repeated percentage updates ("Copying the system (4%)") stay out of the log.
                    if !msg.starts_with("Copying the system (") && !msg.starts_with("Overwriting ") || msg.ends_with("long time)") {
                        text.push_str(&format!("==> {msg}\n"));
                    }
                } else if let Some(msg) = line.strip_prefix("ERROR ") {
                    failed_msg = Some(msg.to_string());
                    text.push_str(&format!("ERROR: {msg}\n"));
                } else if line == "DONE" {
                    finished = Some(true);
                } else if line == "FAILED" {
                    finished = Some(false);
                } else {
                    text.push_str(line.strip_prefix("LOG ").unwrap_or(&line));
                    text.push('\n');
                }
            }

            if !text.is_empty() {
                buf.insert(&mut buf.end_iter(), &text);
                // Keep the log bounded.
                if buf.line_count() > 600 {
                    let mut start = buf.start_iter();
                    let mut cut = buf.iter_at_line(200).unwrap_or_else(|| buf.end_iter());
                    buf.delete(&mut start, &mut cut);
                }
                buf.move_mark(&end_mark, &buf.end_iter());
                log.scroll_mark_onscreen(&end_mark);
            }

            match finished {
                Some(true) => {
                    progress.set_fraction(1.0);
                    win.set_deletable(true);
                    show(done_idx);
                    return;
                }
                Some(false) => {
                    win.set_deletable(true);
                    status.set_markup(&format!(
                        "<b>Installation failed.</b> {}",
                        glib::markup_escape_text(failed_msg.as_deref().unwrap_or("See the log below."))
                    ));
                    buf.insert(&mut buf.end_iter(), "\nInstallation failed. You can close this window and try again.\n");
                    return;
                }
                None => {}
            }
            // Cap UI updates at ~20 per second.
            glib::timeout_future(std::time::Duration::from_millis(50)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames() {
        assert!(valid_username("alice"));
        assert!(valid_username("_svc-1"));
        assert!(!valid_username("Alice"));
        assert!(!valid_username("1abc"));
        assert!(!valid_username("root"));
        assert!(!valid_username(""));
        assert!(!valid_username("a b"));
        assert!(!valid_username("a;rm"));
    }

    #[test]
    fn hostnames() {
        assert!(valid_hostname("safora"));
        assert!(valid_hostname("my-box-2"));
        assert!(!valid_hostname("-bad"));
        assert!(!valid_hostname("bad_name"));
        assert!(!valid_hostname(""));
    }

    #[test]
    fn passphrase_strength() {
        assert_eq!(passphrase_score("short"), 0);
        assert_eq!(passphrase_score("aaaaaaaaaaaa"), 1);
        assert!(passphrase_score("correct horse battery staple") >= 3);
        assert!(passphrase_score("Tr0ub4dor&3xyz!") >= 2);
    }
}
