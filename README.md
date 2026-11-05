# Safora Linux

**Simple by Nature, Private by default.** Safora Linux is private, secure, safe and simple — and quick: nothing runs unless it is needed for security or the desktop.

An Arch-based, privacy- and security-focused Linux distribution with its own Rust Wayland
compositor, a GTK4 status bar and a graphical installer with full-disk encryption.

| Part | What it is |
|------|-----------|
| `src/crates/saforawm` | **saforawm** – auto-tiling Wayland compositor in Rust (built on [smithay](https://github.com/Smithay/smithay)). DRM/KMS + libinput on real hardware, nested window for development. |
| `src/crates/safora-bar` | **safora-bar** – GTK4 + layer-shell bar with pluggable widgets. |
| `src/crates/safora-settings` | **safora-settings** – GTK4 app for looks and layout (writes the config; changes apply live). |
| `src/crates/safora-installer` | **safora-installer** – GTK4 graphical installer front end. |
| `src/crates/safora-common` | Shared config + IPC types. |
| `distro/` | archiso profile, installer backend script, ISO and VirtualBox helper scripts. |

## Download

The installer ISO is attached to the [latest release](https://github.com/tazrog/safora-linux/releases/latest) (about 1.5 GB). Check it before writing it to a USB stick:

```bash
sha256sum -c safora-2026.11.01-x86_64.iso.sha256
```

Boot it in UEFI mode, run the installer from the live desktop, and choose disk encryption. It was tested in VirtualBox only; see *Tested* and *Known issues* below
before putting it on real hardware, and keep your data backed up.

## The desktop

* **Smart auto-tiling**: a new window splits the *focused* window along its longer side — wide windows split side by side, tall windows split top/bottom. Other layouts (spiral, master + stack, columns) cycle with `Mod+Space`.
  Dialogs float automatically. Rounded corners, window transparency, colored borders, configurable gaps.
* **Workspaces**: 1–12 (default 9). The bar always shows the workspace switcher; click or scroll to change.
* **Bar widgets** (each can be switched on/off, moved left/centre/right and reordered in *Safora Settings → Widgets*):
  workspaces (always on), open applications, date & time (calendar popover), CPU, RAM, disk, network
  (Wi-Fi list, join, passwords), Bluetooth (power, connect/disconnect paired devices), audio
  (PulseAudio volume, mute, output device; scroll to change), power menu (log out / suspend / restart / shut down).
  Optional widgets: **network use** (download/upload rate), **CPU temperature**, **battery** (hides itself on desktops),
  **weather** (off by default; asks open-meteo.com for the place you type, nothing else),
  **Claude Code usage** (off by default; 5-hour and weekly plan usage, see below), **app tray** (one-click launchers),
  and any number of **spacers** and **separators** to group widgets.
* **Widget settings**: every widget with options has a gear button in *Safora Settings → Widgets*. Number widgets
  (CPU, RAM, disk, temperature, network use) can be shown as text, a scrolling graph, a meter or an icon, with their own
  format (e.g. RAM used / percentage / used of total), units, drive or connection and update interval. The clock has
  format presets, a custom pattern and a live preview; network, audio, Bluetooth and battery can be icon-only.
  Settings are stored per widget as `[bar.widget.<name>]` in `~/.config/safora/config.toml`.
* **Claude Code usage widget**: shows how much of your Claude plan's 5-hour session and weekly limit is used (e.g. `✻ 5h 42% · wk 18%`);
  click it for both limits with meters, their reset times and a Refresh button. It uses no API and makes no network
  connection: Claude Code passes its own usage figures to its status-line command, `saforactl claude-status`, which keeps
  them in `/run/user/<uid>/safora/claude-usage.json` (memory only, gone at logout). Switching the widget on (or *Connect
  Claude Code* in its settings) adds that status line to `~/.claude/settings.json`; an existing status line of your own is
  only replaced if you confirm. The widget refreshes only while `claude` is running in a terminal; otherwise it shows the
  last values, dimmed. `saforactl claude-connect` / `claude-disconnect` do the same from a shell.
* **Wallpapers**: three included Safora wallpapers (default, dawn, aurora) or any PNG/JPEG/WebP via *Safora Settings → Wallpaper* (fill/fit/stretch/centre).
* **Looks & layout**: six theme presets, bar position/height/margin/radius/opacity/font/colors, gaps,
  borders, tiling mode, master ratio, desktop and border colors, keyboard layout, startup programs.
* **Settings pages**: Appearance (dark/light; Settings itself follows it), Wallpaper, Display, Bar, Widgets, Windows, Fonts, Sound, Network
  (networks are scanned automatically every few seconds, hidden networks can be joined by typing the SSID), Bluetooth, **Power** (screen-off and
  suspend timers, power mode), **Input devices** (keyboard layout and repeat, mouse/touchpad), **Date & time**, **Language**, **Accessibility**
  (text size, large pointer, high contrast, reduced motion), **Accounts** (add/remove users, change passwords, administrator status), Apps & startup, Shortcuts.
  Changes that need root (accounts, time zone, language) ask for your password and run through one validated helper, `/usr/lib/safora/safora-sysconf`.
* **Wayland only, latest GTK (GTK 4)**, **PulseAudio** for audio. No X11 and no XWayland.
* **Safora Settings** also has pages for **Display** (screen size and refresh rate; "Automatic" follows the monitor, or the window when running in a
  virtual machine), **Sound** (volume, mute, input/output devices), **Network** (Wi-Fi list, join with password, status) and **Bluetooth**
  (search, pair, connect, forget), plus a **Dark mode** switch (dark/light preset, GTK apps and icons follow).
* **VirtualBox**: the session runs `VBoxDRMClient` so full-screen mode and window resizing change the guest resolution.
  Use the **VMSVGA** graphics controller with **Enable 3D Acceleration** ticked (`distro/vbox-vm.sh` does this). With 3D off the
  guest display stops updating (black or half-drawn screen, also on text consoles); with VBoxSVGA a live resize shows a skewed picture.

Default shortcuts (`Mod` = Super, or Alt when nested): `Mod+Enter` terminal · `Mod+D` launcher ·
`Mod+Q` close · `Mod+F` fullscreen · `Mod+Shift+Space` float · `Mod+Space` layout ·
`Mod+J/K` or `Mod+Tab` focus · `Mod+Shift+J/K` reorder · `Mod+1…9` workspace ·
`Mod+Shift+1…9` move window · `Mod+S` settings · `Mod+Shift+R` reload · `Mod+Shift+E` quit ·
`Ctrl+Alt+F1…F6` virtual terminals. Add your own under `[keybinds]` in `~/.config/safora/config.toml`.

`saforactl state|workspace N|move N|layout|float|fullscreen|close|spawn CMD|reload|quit` controls a running session.

## The distro

* Kernel: **linux-hardened**. AppArmor enabled. UFW firewall on (deny incoming).
* sysctl hardening, rare network protocols and FireWire DMA modules blacklisted, core dumps disabled.
* NetworkManager with **random MAC addresses**, IPv6 privacy addresses, **DNS-over-TLS** (opportunistic) via systemd-resolved, DNSSEC.
* **zram swap** instead of a swap partition, so nothing sensitive is paged to disk. `/tmp` is `nosuid,nodev`. Default `umask 077`.
* Root account is locked; the installed user administers the machine through `sudo`.
* HTTPS-only package mirrors; no telemetry anywhere.
* **Firefox** is preconfigured by `/etc/firefox/policies/policies.json`: telemetry, studies, Pocket and sponsored content off,
  **DuckDuckGo** as the default (and only pre-installed) search engine, search suggestions off, strict tracking protection,
  HTTPS-only mode, WebRTC local-IP leak protection, and uBlock Origin installed from addons.mozilla.org on first start.
* **Boot theme** matching the desktop wallpaper, with the logo, *Safora Linux* and the tagline: Plymouth splash (including the
  disk-passphrase prompt), the GRUB menu of installed systems (`/boot/grub/themes/safora`) and the BIOS menu of the live ISO.
  All images are generated by `distro/art/make-art.py`.
* **Bluetooth keyboard at the unlock screen** (experimental, opt-in, untested on real hardware): no longer offered by the
  installer. On an installed, encrypted system it can be switched on or off (and updated after re-pairing) in *Safora Settings → Bluetooth*, which adds a `saforabt` initramfs hook. The adapter's firmware is included so it also works after a cold start. Trade-off: the pairing key is stored in the
  unencrypted `/boot`. A USB keyboard always works.
* **Secure erase before installing**: the disk page offers *Quick* (discard every block; a real secure erase where the drive supports it, otherwise an ordinary discard, which the installer says plainly) or *Full*
  (one pass of random data over the whole drive with `shred`; slow, best for hard disks). On SSDs, a full overwrite cannot reach spare flash cells, so prefer *Quick*, or the drive's own secure-erase/sanitize command, when old data must not be recoverable.
* Installer: GPT, **ext4** everywhere (the EFI system partition is FAT32 because UEFI requires it),
  optional **LUKS2** (AES-256-XTS, argon2id) around the root filesystem. A separate 1 GiB unencrypted
  `/boot` holds the kernel so GRUB doesn't need to unlock the volume.
  Passphrases reach the backend over stdin only (never argv/environment).

### Meerkat (meer10) and other Intel mini PCs

* Installed for Intel hardware: `intel-ucode`, Mesa with Vulkan (`vulkan-intel`) and VA-API video decode (`intel-media-driver`), SOF audio firmware
  (`sof-firmware`, `alsa-ucm-conf`), Wi-Fi 7/Bluetooth firmware (`linux-firmware-intel`), Thunderbolt/USB4 authorisation (`bolt`), firmware updates (`fwupd`), `wireless-regdb`.
* The installer tailors the install to the machine: it removes AMD microcode/GPU firmware on Intel CPUs, Intel microcode on AMD CPUs, and the VirtualBox guest tools on real hardware.
* Kernel command line on bare-metal Intel: `intel_iommu=on` (blocks DMA attacks through Thunderbolt/USB4). Encrypted disks use LUKS2 with the faster `no-read-workqueue,no-write-workqueue` mode.
* Vim and Git are installed (`EDITOR=vim`).
* **Several monitors**: every connected screen is lit and laid out left to right. Windows tile on the first (primary) screen; the other screens get the wallpaper and their own bar.
  `Mod+O` sends the focused window to the next screen (it floats there; sending it back to the first screen tiles it again). Fullscreen covers the screen the pointer is on.
  This is new and has only been exercised with one screen, so treat it as experimental; *Settings → Display → Use additional monitors* turns it off.
* **4K screens**: interface scale is 200% automatically on screens 3000+ pixels wide (or choose 100%/200% under Display).

### Screen lock

`Mod+L`, the power menu, `saforactl lock`, or the idle timer (default 5 minutes; Settings → Power) locks the screen. While locked, windows and the bar are not drawn,
no key binding or click reaches any window, and only `safora-lock` receives input. Its password check uses the same setuid helper PAM uses (`unix_chkpwd`),
with a growing delay after wrong passwords, and it can only unlock with a one-time secret the compositor gave it. If it crashes, the screen stays locked and it is restarted.
Suspend locks first. Ctrl+Alt+F1…F6 still switches to a text console (which needs a login). Not yet done: extra monitors just go black while locked (no lock UI on them), and locking on lid close.

### No telemetry

Safora sends nothing about you or your machine anywhere. No Safora program contains analytics, crash reporting or update checks. Beyond that:

* **Firefox**: telemetry, studies/Normandy, Pocket, sponsored suggestions, crash-report submission, captive-portal and connectivity probes, remote Safe Browsing download lookups and geolocation are off and locked by policy.
* **NetworkManager**: connectivity checking is off, so it never contacts a server on its own.
* **Tools you may add**: shells export `DISABLE_TELEMETRY`, `DISABLE_ERROR_REPORTING`, `DISABLE_AUTOUPDATER` and `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` (Claude Code) and turn off npm's update notifier and audit calls.
* **What does connect, because you asked it to**: package downloads from the Arch mirrors (`pacman`), network time (`systemd-timesyncd`, the Arch NTP pool), DNS-over-TLS to your configured resolver, the uBlock Origin download on Firefox's first start, and anything you browse. Firmware updates (`fwupd`) contact LVFS only when you run `fwupdmgr`.

### Lean by design

Always running: the compositor, the bar, NetworkManager, systemd-resolved/timesyncd, UFW, AppArmor, greetd, and gnome-keyring
(unlocked by your login password; it keeps passwords you choose to remember, e.g. *Remember forever* for an encrypted USB drive, encrypted on disk). Everything else starts on demand
(PulseAudio by socket, power-profiles-daemon by D-Bus) or only when the hardware exists (Bluetooth daemon needs an adapter, VirtualBox services need
VirtualBox). No file indexer, no notification daemon, no polkit agent, no auto-updater, no telemetry. The optional Claude Code is
downloaded (Anthropic's native installer, into the user's `~/.local/bin`) only if you tick that box; with auto-updates off, update it with `claude update`. The idle timer and display-mode check run on one 2-second tick.

### Tested

In VirtualBox 7.2 (UEFI and BIOS, 4 GB RAM): boot of the live ISO into the Safora session, a complete scripted
install with LUKS2, reboot, passphrase prompt, login, bar, launcher, `sudo`, UFW. Not tested on real hardware,
with an unencrypted install, with multiple monitors, or with mouse interaction inside the VM.

### Known issues

* Occasionally a small region of the bar (e.g. the CPU label) renders blank until its next update; a compositor
  repaint bug seen under VirtualBox's software renderer.
* On one live boot of the final ISO the bar and installer were not running (the compositor was fine; starting
  them with `saforactl spawn safora-bar` / `safora-installer` worked). Not reproduced in the other boots; cause unknown.

### Notes on the newest features

* Tested in VirtualBox: smart splits, wallpaper, rounded corners/borders, Plymouth splash with the passphrase prompt, Firefox policies
  (verified in `about:policies`, uBlock Origin installed), initramfs build with the Bluetooth hook and a normal boot with it enabled.
* Not tested: a real Bluetooth keyboard at the unlock screen (no adapter available), real hardware.
* `kernel.unprivileged_userns_clone=1` is set so Firefox's sandbox works on `linux-hardened`; see `99-safora-hardening.conf`.

### Honest limitations

* `linux-hardened` 7.2.10 crashed (`kernel BUG` in `post_alloc_hook`) inside VirtualBox when `cryptsetup`
  allocated its argon2 memory. The live boot entries and installs on virtual machines therefore add
  `init_on_alloc=0 init_on_free=0`. On real hardware the installer leaves the hardened defaults alone.

* `/boot` is not encrypted and Secure Boot signing is not set up, so an attacker with physical access
  can tamper with the kernel/initramfs ("evil maid"). LUKS still protects your data at rest.
* The screen locker is built into the compositor rather than using the `ext-session-lock` protocol, so third-party lockers (swaylock etc.) won't work. See *Screen lock* above.
* saforawm implements the protocols used by GTK4, Firefox and common terminals; screen capture/sharing,
  input-method, and multi-monitor layouts are not implemented (extra monitors stay dark).
* Packages from your wish list that need X11 (`lightdm`, `picom`, `feh`, `dmenu`, `slock`, `numlockx`, `lxsession`, …) are
  intentionally left out. Heavy extras (`vlc`, `mame`, `godot`, `stella`, `flatpak`, …) are not on the ISO; see
  `distro/extras.txt` and install them with `sudo pacman -S --needed - < /usr/share/safora/extras.txt`.

## Building

The unprivileged build sandbox (a full Arch root, with thousands of symlinks) lives outside this folder, in `~/.local/share/safora/archroot` (override with `SAFORA_SANDBOX`), so the project folder can be copied or backed up anywhere. The 17 symlinks of the ISO profile are listed in `distro/symlinks.txt` and created by `build-iso.sh`.

Everything below runs unprivileged except the final ISO step.

```bash
# 1. Build the Rust workspace inside the Arch sandbox (see scripts/sandbox.sh)
scripts/sandbox.sh bash -c 'cd /src && cargo build --release'

# 2. Build the ISO (needs root: mkarchiso installs packages into a chroot)
sudo bash distro/build-iso.sh       # → out/safora-YYYY.MM.DD-x86_64.iso  (FAST=1 for quick dev builds)

# 3. Test in a VM: QEMU/KVM from the sandbox (needs  pacman -S qemu-desktop edk2-ovmf  inside it), no root needed.
#    The ISO is attached as a USB stick with 6 GB RAM, like real hardware (archiso then copies itself to RAM).
scripts/qemu-vm.sh new              # fresh disk, boot the newest ISO;  `iso` = keep disk,  `disk` = boot the installed system
# or VirtualBox:
distro/vbox-vm.sh uefi              # or: bios
VBoxManage startvm Safora-Test-uefi
```

### Developing the compositor

```bash
scripts/sandbox.sh bash /scripts/nested-test.sh     # runs saforawm in a window on your desktop + bar + terminals
scripts/sandbox.sh cargo test                       # in /src: unit tests
```

## License

MIT, see [LICENSE](LICENSE). Bundled and installed software keeps its own licenses.
