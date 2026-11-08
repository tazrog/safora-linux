#!/usr/bin/python3
"""
USB Burner — a small GTK 4 tool to write Linux distro images (ISO/IMG) to USB
drives so they can be booted.

The GUI runs unprivileged. The actual write is performed by
`usbburner-helper`, launched through sudo (the app asks for your password).
"""

import hashlib
import json
import os
import subprocess
import sys
import threading
import time

try:
    import gi
    gi.require_version("Gtk", "4.0")
except (ImportError, AttributeError, ValueError):
    sys.exit("USB Burner needs the GTK 4 Python bindings.\n"
             "Install them with:  sudo pacman -S python-gobject gtk4")

gi.require_version("Pango", "1.0")
from gi.repository import Gdk, Gio, GLib, Gtk, Pango  # noqa: E402

APP_ID = "io.github.usbburner"
HERE = os.path.dirname(os.path.realpath(__file__))
HELPER = os.path.join(HERE, "usbburner-helper")

CSS = b"""
.title-label { font-size: 1.4em; font-weight: bold; }
.dim { opacity: 0.7; }
.status-ok { color: #2ec27e; font-weight: bold; }
.status-err { color: #e01b24; font-weight: bold; }
"""


def human(n):
    n = float(n)
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if n < 1000 or unit == "TB":
            return f"{n:.1f} {unit}" if unit != "B" else f"{int(n)} B"
        n /= 1000
    return f"{n:.1f} TB"


def truthy(v):
    return v is True or v in ("1", 1, "true")


SYSTEM_MOUNTS = {"/", "/boot", "/boot/efi", "/efi", "/home", "/usr", "/var", "[SWAP]"}


def _walk(node):
    yield node
    for c in node.get("children", []) or []:
        yield from _walk(c)


def list_usb_drives():
    """Return a list of dicts describing removable whole-disk devices."""
    try:
        out = subprocess.run(
            ["lsblk", "-J", "-b", "-o",
             "NAME,PATH,TYPE,TRAN,RM,HOTPLUG,SIZE,RO,MODEL,VENDOR,LABEL,MOUNTPOINTS"],
            check=True, capture_output=True, text=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        return []
    drives = []
    for d in json.loads(out).get("blockdevices", []):
        if d.get("type") != "disk" or truthy(d.get("ro")):
            continue
        if not (d.get("tran") == "usb" or truthy(d.get("rm")) or truthy(d.get("hotplug"))):
            continue
        size = int(d.get("size") or 0)
        if size == 0:  # empty card reader slot
            continue
        mounts = [m for n in _walk(d) for m in (n.get("mountpoints") or []) if m]
        if any(m in SYSTEM_MOUNTS for m in mounts):
            continue
        vendor = (d.get("vendor") or "").strip()
        model = (d.get("model") or "").strip()
        name = " ".join(x for x in (vendor, model) if x) or "USB drive"
        holders = sorted({n.get("type") for n in _walk(d)
                          if n is not d and n.get("type") != "part"})
        label = f"{name} — {human(size)}  ({d['path']})"
        if holders:
            label += f"  [{'/'.join(holders)} in use]"
        elif mounts:
            label += "  [mounted]"
        drives.append({
            "holders": holders,
            "path": d["path"],
            "size": size,
            "name": name,
            "mounts": mounts,
            "label": label,
        })
    return drives


class BurnerWindow(Gtk.ApplicationWindow):
    def __init__(self, app):
        super().__init__(application=app, title="USB Burner")
        self.set_default_size(560, 0)
        self.set_resizable(False)

        self.image_path = None
        self.drives = []
        self.proc = None
        self.busy = False
        self.stage = None
        self.t_start = 0.0

        header = Gtk.HeaderBar()
        about_btn = Gtk.Button(icon_name="help-about-symbolic", tooltip_text="About")
        about_btn.connect("clicked", self.on_about)
        header.pack_end(about_btn)
        self.set_titlebar(header)

        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14,
                       margin_top=18, margin_bottom=18, margin_start=18, margin_end=18)
        self.set_child(root)

        title = Gtk.Label(label="Create a bootable USB drive", xalign=0)
        title.add_css_class("title-label")
        root.append(title)
        sub = Gtk.Label(label="Write a Linux ISO or disk image to a USB stick.",
                        xalign=0)
        sub.add_css_class("dim")
        root.append(sub)

        grid = Gtk.Grid(column_spacing=12, row_spacing=12)
        root.append(grid)

        # --- Step 1: image --------------------------------------------------
        grid.attach(self._step_label("1. Image"), 0, 0, 1, 1)
        img_box = Gtk.Box(spacing=8, hexpand=True)
        self.image_label = Gtk.Label(label="No image selected", xalign=0, hexpand=True,
                                     ellipsize=Pango.EllipsizeMode.END)
        self.image_label.add_css_class("dim")
        self.choose_btn = Gtk.Button(label="Choose…")
        self.choose_btn.connect("clicked", self.on_choose_image)
        img_box.append(self.image_label)
        img_box.append(self.choose_btn)
        grid.attach(img_box, 1, 0, 1, 1)

        # --- Step 2: device -------------------------------------------------
        grid.attach(self._step_label("2. Drive"), 0, 1, 1, 1)
        dev_box = Gtk.Box(spacing=8, hexpand=True)
        self.drive_model = Gtk.StringList()
        self.drive_dd = Gtk.DropDown(model=self.drive_model, hexpand=True)
        self.drive_dd.connect("notify::selected", lambda *_: self.update_sensitivity())
        self.refresh_btn = Gtk.Button(icon_name="view-refresh-symbolic",
                                      tooltip_text="Rescan drives")
        self.refresh_btn.connect("clicked", lambda *_: self.refresh_drives())
        dev_box.append(self.drive_dd)
        dev_box.append(self.refresh_btn)
        grid.attach(dev_box, 1, 1, 1, 1)

        # --- Options --------------------------------------------------------
        grid.attach(self._step_label("SHA-256"), 0, 2, 1, 1)
        self.sha_entry = Gtk.Entry(placeholder_text="Optional: paste the checksum from the distro website",
                                   hexpand=True)
        grid.attach(self.sha_entry, 1, 2, 1, 1)

        self.verify_chk = Gtk.CheckButton(label="Verify drive after writing", active=True)
        grid.attach(self.verify_chk, 1, 3, 1, 1)

        # --- Progress -------------------------------------------------------
        self.progress = Gtk.ProgressBar(show_text=True, text="")
        root.append(self.progress)
        self.status = Gtk.Label(label="Select an image and a drive.", xalign=0, wrap=True)
        root.append(self.status)

        # --- Buttons --------------------------------------------------------
        btn_box = Gtk.Box(spacing=8, halign=Gtk.Align.END)
        self.cancel_btn = Gtk.Button(label="Cancel", sensitive=False)
        self.cancel_btn.connect("clicked", self.on_cancel)
        self.write_btn = Gtk.Button(label="Write to USB")
        self.write_btn.add_css_class("destructive-action")
        self.write_btn.connect("clicked", self.on_write)
        btn_box.append(self.cancel_btn)
        btn_box.append(self.write_btn)
        root.append(btn_box)

        self.connect("close-request", self.on_close_request)

        self.refresh_drives()
        GLib.timeout_add_seconds(3, self._poll_drives)

    # ------------------------------------------------------------------ util
    def _step_label(self, text):
        lbl = Gtk.Label(label=text, xalign=0)
        lbl.add_css_class("heading")
        return lbl

    def set_status(self, text, kind=None):
        self.status.set_text(text)
        for c in ("status-ok", "status-err"):
            self.status.remove_css_class(c)
        if kind:
            self.status.add_css_class(f"status-{kind}")

    def selected_drive(self):
        i = self.drive_dd.get_selected()
        if self.drives and i != Gtk.INVALID_LIST_POSITION and i < len(self.drives):
            return self.drives[i]
        return None

    def update_sensitivity(self):
        idle = not self.busy
        for w in (self.choose_btn, self.drive_dd, self.refresh_btn,
                  self.sha_entry, self.verify_chk):
            w.set_sensitive(idle)
        self.write_btn.set_sensitive(idle and bool(self.image_path) and bool(self.selected_drive()))
        self.cancel_btn.set_sensitive(self.busy)

    # ---------------------------------------------------------------- drives
    def refresh_drives(self):
        prev = self.selected_drive()
        prev_path = prev["path"] if prev else None
        drives = list_usb_drives()
        if [d["label"] for d in drives] == [d["label"] for d in self.drives]:
            return
        self.drives = drives
        self.drive_model.splice(0, self.drive_model.get_n_items(),
                                [d["label"] for d in drives] or ["No USB drives found"])
        sel = 0
        for i, d in enumerate(drives):
            if d["path"] == prev_path:
                sel = i
        self.drive_dd.set_selected(sel)
        self.update_sensitivity()

    def _poll_drives(self):
        if not self.busy:
            self.refresh_drives()
        return True

    # ----------------------------------------------------------------- image
    def on_choose_image(self, _btn):
        dialog = Gtk.FileDialog(title="Choose a disk image")
        filters = Gio.ListStore.new(Gtk.FileFilter)
        f = Gtk.FileFilter(name="Disk images (*.iso, *.img, compressed)")
        for pat in ("*.iso", "*.ISO", "*.img", "*.IMG", "*.raw",
                    "*.img.xz", "*.iso.xz", "*.img.gz", "*.img.bz2", "*.img.zst", "*.iso.zst"):
            f.add_pattern(pat)
        filters.append(f)
        allf = Gtk.FileFilter(name="All files")
        allf.add_pattern("*")
        filters.append(allf)
        dialog.set_filters(filters)
        dialog.set_default_filter(f)
        dialog.open(self, None, self._on_image_chosen)

    def _on_image_chosen(self, dialog, result):
        try:
            gfile = dialog.open_finish(result)
        except GLib.Error:
            return
        path = gfile.get_path() if gfile else None
        if not path:
            return
        self.image_path = path
        size = os.path.getsize(path)
        self.image_label.set_text(f"{os.path.basename(path)}  ({human(size)})")
        self.image_label.set_tooltip_text(path)
        self.image_label.remove_css_class("dim")
        self.set_status("Ready.")
        self.update_sensitivity()

    # ----------------------------------------------------------------- write
    def on_write(self, _btn):
        drive = self.selected_drive()
        if not drive or not self.image_path:
            return
        img_size = os.path.getsize(self.image_path)
        compressed = self.image_path.lower().endswith((".xz", ".gz", ".bz2", ".zst"))
        if not compressed and img_size > drive["size"]:
            self.set_status(f"The image ({human(img_size)}) does not fit on this drive "
                            f"({human(drive['size'])}).", "err")
            return

        if drive["holders"]:
            self.set_status(f"{drive['path']} has an open {'/'.join(drive['holders'])} volume. "
                            "Unmount and lock it in your file manager first.", "err")
            return

        expected = self.sha_entry.get_text().strip().lower().split()
        expected = expected[0] if expected else ""
        if expected and (len(expected) != 64 or any(c not in "0123456789abcdef" for c in expected)):
            self.set_status("The SHA-256 checksum should be 64 hex characters.", "err")
            return

        mounted = ""
        if drive["mounts"]:
            mounted = "\n\nIts partitions will be unmounted: " + ", ".join(drive["mounts"])
        if drive["size"] > 128e9:
            mounted += ("\n\n⚠ This is a large drive. It may be an external hard disk "
                        "rather than a USB stick, so make sure it is the right one.")
        alert = Gtk.AlertDialog(
            message=f"Erase {drive['name']} ({human(drive['size'])})?",
            detail=(f"ALL data on {drive['path']} will be permanently destroyed and replaced "
                    f"with:\n{os.path.basename(self.image_path)}{mounted}"),
            buttons=["Cancel", "Erase and Write"],
            cancel_button=0, default_button=0, modal=True,
        )
        alert.choose(self, None, self._on_confirm, drive, expected)

    def _on_confirm(self, alert, result, drive, expected):
        try:
            choice = alert.choose_finish(result)
        except GLib.Error:
            return
        if choice != 1:
            return
        self.busy = True
        self.update_sensitivity()
        self.progress.set_fraction(0)
        if expected:
            threading.Thread(target=self._checksum_then_write,
                             args=(drive, expected), daemon=True).start()
        else:
            self._start_helper(drive)

    def _checksum_then_write(self, drive, expected):
        GLib.idle_add(self.set_status, "Checking image checksum…")
        GLib.idle_add(self.progress.set_text, "Checksum")
        h = hashlib.sha256()
        total = os.path.getsize(self.image_path) or 1
        done = 0
        last = 0.0
        self._checksum_cancel = False
        with open(self.image_path, "rb") as f:
            while chunk := f.read(4 * 1024 * 1024):
                if self._checksum_cancel:
                    GLib.idle_add(self._finish, False, "Cancelled.")
                    return
                h.update(chunk)
                done += len(chunk)
                now = time.monotonic()
                if now - last > 0.1:
                    last = now
                    GLib.idle_add(self.progress.set_fraction, done / total)
        if h.hexdigest() != expected:
            GLib.idle_add(self._finish, False,
                          "Checksum MISMATCH — the image is corrupt or not the file you expected. "
                          "Nothing was written.")
            return
        GLib.idle_add(self.set_status, "Checksum OK.", "ok")
        GLib.idle_add(self._start_helper, drive)

    def _start_helper(self, drive, error=None):
        if not os.access(HELPER, os.X_OK):
            self._finish(False, f"Helper not found or not executable: {HELPER}")
            return False
        self.set_status("Waiting for your password…")
        self.progress.set_text("Authorizing")
        self._ask_password(drive, error)
        return False

    def _ask_password(self, drive, error=None):
        """Modal dialog asking for the user's password (used for sudo)."""
        dlg = Gtk.Window(transient_for=self, modal=True, resizable=False,
                         title="Authentication Required")
        dlg.set_default_size(380, 0)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12,
                      margin_top=18, margin_bottom=18, margin_start=18, margin_end=18)
        dlg.set_child(box)
        msg = Gtk.Label(xalign=0, wrap=True,
                        label=f"Writing to {drive['path']} needs administrator rights.\n"
                              f"Enter the password for {GLib.get_user_name()}:")
        box.append(msg)
        entry = Gtk.PasswordEntry(show_peek_icon=True, hexpand=True)
        box.append(entry)
        if error:
            err = Gtk.Label(label=error, xalign=0, wrap=True)
            err.add_css_class("status-err")
            box.append(err)
        btns = Gtk.Box(spacing=8, halign=Gtk.Align.END)
        cancel = Gtk.Button(label="Cancel")
        ok = Gtk.Button(label="Authenticate")
        ok.add_css_class("suggested-action")
        btns.append(cancel)
        btns.append(ok)
        box.append(btns)

        state = {"done": False}

        def submit(*_):
            pw = entry.get_text()
            if not pw:
                return
            state["done"] = True
            entry.set_text("")
            dlg.destroy()
            self.set_status("Checking password…")
            threading.Thread(target=self._authenticate_then_run,
                             args=(drive, pw), daemon=True).start()

        def abort(*_):
            if state["done"]:
                return False
            state["done"] = True
            dlg.destroy()
            self._finish(False, "Authorization was cancelled.")
            return False

        ok.connect("clicked", submit)
        entry.connect("activate", submit)
        cancel.connect("clicked", abort)
        dlg.connect("close-request", abort)
        dlg.present()
        entry.grab_focus()

    def _authenticate_then_run(self, drive, password):
        # Step 1: check the password with `sudo -v`. stdin hits EOF after one
        # line, so a wrong password makes sudo exit instead of re-prompting.
        env = dict(os.environ, LC_ALL="C")
        try:
            r = subprocess.run(["sudo", "-S", "-v", "-p", ""], input=password + "\n",
                               capture_output=True, text=True, env=env, timeout=60)
        except (OSError, subprocess.TimeoutExpired) as e:
            GLib.idle_add(self._finish, False, f"Could not run sudo: {e}")
            return
        if r.returncode != 0:
            err = r.stderr.strip()
            if "incorrect password" in err or "Sorry, try again" in err:
                GLib.idle_add(self._start_helper, drive, "Incorrect password. Try again.")
            elif "not in the sudoers" in err or "not allowed" in err or "may not run" in err:
                GLib.idle_add(self._finish, False,
                              f"{GLib.get_user_name()} is not allowed to use sudo on this system.")
            else:
                GLib.idle_add(self._finish, False, err or "sudo failed.")
            return
        GLib.idle_add(self._run_helper, drive, password)

    def _run_helper(self, drive, password):
        # Step 2: run the helper. If sudo's credential cache is used it won't
        # read the password, and the helper ignores that line (it only acts on
        # "cancel" or EOF).
        cmd = ["sudo", "-S", "-p", "", "--", HELPER, self.image_path, drive["path"]]
        if self.verify_chk.get_active():
            cmd.append("--verify")
        try:
            self.proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                         stderr=subprocess.PIPE, text=True, bufsize=1)
            self.proc.stdin.write(password + "\n")
            self.proc.stdin.flush()
        except OSError as e:
            self._finish(False, f"Could not start sudo: {e}")
            return False
        del password
        self.set_status("Starting…")
        self.t_start = time.monotonic()
        threading.Thread(target=self._read_helper, args=(self.proc,), daemon=True).start()
        return False

    def _read_helper(self, proc):
        error = None
        done_ok = False
        last_ui = 0.0
        for line in proc.stdout:
            parts = line.rstrip("\n").split(" ", 1)
            kind, rest = parts[0], (parts[1] if len(parts) > 1 else "")
            if kind == "PROGRESS":
                now = time.monotonic()
                if now - last_ui < 0.15:
                    continue
                last_ui = now
                done, total = (int(x) for x in rest.split())
                GLib.idle_add(self._on_progress, done, total)
            elif kind == "STAGE":
                GLib.idle_add(self._on_stage, rest)
            elif kind == "INFO":
                GLib.idle_add(self.set_status, rest)
            elif kind == "ERROR":
                error = rest
            elif kind == "DONE":
                done_ok = True
        rc = proc.wait()
        stderr = proc.stderr.read().strip()
        if done_ok and rc == 0:
            GLib.idle_add(self._finish, True,
                          "Done! You can now remove the drive and boot from it.")
        else:
            if not error:
                error = stderr or f"Writer exited with code {rc}."
            GLib.idle_add(self._finish, False, error)

    def _on_stage(self, stage):
        self.stage = stage
        self.t_start = time.monotonic()
        label = {"write": "Writing", "sync": "Flushing to drive",
                 "verify": "Verifying"}.get(stage, stage)
        self.progress.set_text(label)
        self.set_status(f"{label}…")
        if stage == "sync":
            self.progress.pulse()

    def _on_progress(self, done, total):
        frac = done / total if total else 0
        self.progress.set_fraction(min(frac, 1.0))
        elapsed = time.monotonic() - self.t_start
        speed = done / elapsed if elapsed > 0.5 else 0
        label = {"write": "Writing", "verify": "Verifying",
                 "sync": "Flushing"}.get(self.stage, "")
        self.progress.set_text(f"{label} {frac * 100:.0f}%")
        if speed > 0 and self.stage in ("write", "verify"):
            eta = (total - done) / speed
            self.set_status(f"{label}: {human(done)} of {human(total)} — "
                            f"{human(speed)}/s — about {int(eta // 60)}m {int(eta % 60)}s left")

    def _finish(self, ok, message):
        self.busy = False
        self.proc = None
        self.stage = None
        if ok:
            self.progress.set_fraction(1.0)
            self.progress.set_text("Complete")
        else:
            self.progress.set_text("Failed")
        self.set_status(message, "ok" if ok else "err")
        self.update_sensitivity()
        self.refresh_drives()
        if ok:
            self.get_application().send_notification(
                "done", Gio.Notification.new("USB drive is ready"))
        return False

    def on_cancel(self, _btn):
        self._checksum_cancel = True
        if self.proc and self.proc.poll() is None:
            try:
                self.proc.stdin.write("cancel\n")
                self.proc.stdin.flush()
                self.proc.stdin.close()
            except (OSError, ValueError):
                pass
            self.set_status("Cancelling…")

    def on_close_request(self, _win):
        if not self.busy:
            return False
        alert = Gtk.AlertDialog(
            message="A write is in progress",
            detail="Closing now will abort the write and leave the drive unusable.",
            buttons=["Keep Writing", "Abort and Quit"], cancel_button=0, default_button=0,
        )

        def on_choice(a, res):
            try:
                if a.choose_finish(res) == 1:
                    self.on_cancel(None)
                    self.busy = False
                    self.close()
            except GLib.Error:
                pass

        alert.choose(self, None, on_choice)
        return True

    def on_about(self, _btn):
        Gtk.AboutDialog(
            transient_for=self, modal=True, program_name="USB Burner",
            version="1.0", logo_icon_name="drive-removable-media",
            comments="Write Linux distribution images to USB drives for booting.",
            license_type=Gtk.License.GPL_3_0,
        ).present()


class BurnerApp(Gtk.Application):
    def __init__(self):
        super().__init__(application_id=APP_ID, flags=Gio.ApplicationFlags.DEFAULT_FLAGS)

    def do_startup(self):
        Gtk.Application.do_startup(self)
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_display(
            Gdk.Display.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

    def do_activate(self):
        win = self.props.active_window or BurnerWindow(self)
        win.present()


def main():
    return BurnerApp().run(sys.argv)


if __name__ == "__main__":
    sys.exit(main())
