//! System data sources and controls (/proc readers, nmcli, pactl, bluetoothctl).
//! Parsing is separated from I/O so it can be unit tested.

use std::process::Command;

// --------------------------------------------------------------------- helpers

/// Run a command with a 4 second ceiling and return stdout on success.
pub fn run(cmd: &str, args: &[&str]) -> Option<String> {
    run_for(4, cmd, args)
}

/// Like [`run`], with a custom time limit in seconds.
pub fn run_for(secs: u32, cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("timeout").arg(secs.to_string()).arg(cmd).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Start a command and forget about it.
pub fn spawn(cmd: &str, args: &[&str]) {
    if let Ok(mut child) = Command::new(cmd).args(args).spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

// ------------------------------------------------------------------- cpu / mem

/// `(idle, total)` jiffies from the aggregate `cpu` line of /proc/stat.
pub fn parse_cpu(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let v: Vec<u64> = line.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    if v.len() < 5 {
        return None;
    }
    let idle = v[3] + v[4]; // idle + iowait
    Some((idle, v.iter().take(8).sum()))
}

pub fn cpu_percent(prev: (u64, u64), now: (u64, u64)) -> f64 {
    let total = now.1.saturating_sub(prev.1);
    if total == 0 {
        return 0.0;
    }
    let idle = now.0.saturating_sub(prev.0);
    (total.saturating_sub(idle)) as f64 * 100.0 / total as f64
}

pub fn read_cpu() -> Option<(u64, u64)> {
    parse_cpu(&std::fs::read_to_string("/proc/stat").ok()?)
}

/// `(total_kib, available_kib)`.
pub fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let get = |key: &str| {
        text.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
    };
    Some((get("MemTotal:")?, get("MemAvailable:")?))
}

pub fn read_mem() -> Option<(u64, u64)> {
    parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

/// `(cached_kib, swap_used_kib, swap_total_kib)` — page cache (incl. buffers, reclaimable slab) and swap.
pub fn parse_mem_extra(text: &str) -> (u64, u64, u64) {
    let get = |key: &str| {
        text.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    };
    let cached = get("Cached:") + get("Buffers:") + get("SReclaimable:");
    let (st, sf) = (get("SwapTotal:"), get("SwapFree:"));
    (cached, st.saturating_sub(sf), st)
}

pub fn read_mem_extra() -> (u64, u64, u64) {
    parse_mem_extra(&std::fs::read_to_string("/proc/meminfo").unwrap_or_default())
}

// ----------------------------------------------------------------- ram cleaner

/// One program (all of its processes) and the memory it holds.
#[derive(Debug, Clone)]
pub struct AppMem {
    pub name: String,
    pub pids: Vec<u32>,
    /// Proportional set size in KiB (shared pages split fairly between their users).
    pub kib: u64,
}

/// Session plumbing that must never be offered for closing.
const PROTECTED: &[&str] = &[
    "systemd", "(sd-pam)", "sd-pam", "dbus-daemon", "dbus-broker", "dbus-broker-launch", "pipewire", "pipewire-pulse",
    "wireplumber", "Xwayland", "gnome-keyring-daemon", "ssh-agent", "gpg-agent", "polkit-agent", "at-spi-bus-launcher",
    "at-spi2-registryd", "xdg-desktop-portal", "xdg-desktop-portal-gtk", "xdg-desktop-portal-wlr", "xdg-document-portal",
    "xdg-permission-store", "dconf-service", "login", "agetty", "greetd", "seatd",
];

fn is_protected(name: &str) -> bool {
    name.starts_with("safora") || name.starts_with("gvfs") || name.starts_with("dbus-") || PROTECTED.contains(&name)
}

/// Value of a `Key:  1234 kB` style line.
fn status_field(text: &str, key: &str) -> Option<u64> {
    text.lines().find(|l| l.starts_with(key))?.split_whitespace().nth(1)?.parse().ok()
}

fn real_uid(status: &str) -> Option<u32> {
    status.lines().find(|l| l.starts_with("Uid:"))?.split_whitespace().nth(1)?.parse().ok()
}

fn parent_pid(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name may contain spaces and parentheses; fields resume after the last ')'.
    stat[stat.rfind(')')? + 1..].split_whitespace().nth(1)?.parse().ok()
}

/// Friendly program name: the executable's file name, or for interpreters the script they run.
fn program_name(pid: u32, comm: &str) -> String {
    let base = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok().map(|p| base(&p.to_string_lossy().replace(" (deleted)", "")));
    let exe = exe.filter(|e| !e.is_empty()).unwrap_or_else(|| comm.to_string());
    let interp = exe.starts_with("python") || exe.starts_with("perl") || exe.starts_with("ruby")
        || matches!(exe.as_str(), "sh" | "bash" | "dash" | "zsh" | "node" | "java" | "gjs" | "lua");
    if interp {
        if let Ok(cmd) = std::fs::read(format!("/proc/{pid}/cmdline")) {
            if let Some(script) = cmd.split(|b| *b == 0).skip(1).map(String::from_utf8_lossy).find(|a| !a.is_empty() && !a.starts_with('-')) {
                return base(&script);
            }
        }
    }
    exe
}

/// Programs owned by the current user, biggest first. Kernel threads, session plumbing, and this
/// process's own ancestors (the compositor, the session) are left out.
pub fn read_app_memory() -> Vec<AppMem> {
    let me = std::process::id();
    let uid = std::fs::read_to_string("/proc/self/status").ok().as_deref().and_then(real_uid);
    let mut ancestors = vec![me];
    let mut p = me;
    while let Some(pp) = parent_pid(p).filter(|pp| *pp > 1) {
        ancestors.push(pp);
        p = pp;
    }

    let mut apps: Vec<AppMem> = Vec::new();
    let Ok(dir) = std::fs::read_dir("/proc") else { return apps };
    for entry in dir.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        if ancestors.contains(&pid) {
            continue;
        }
        let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else { continue };
        if uid.is_some() && real_uid(&status) != uid {
            continue;
        }
        // Kernel threads have no VmRSS line.
        let Some(rss) = status_field(&status, "VmRSS:") else { continue };
        let comm = status.lines().next().and_then(|l| l.strip_prefix("Name:")).unwrap_or("").trim().to_string();
        let name = program_name(pid, &comm);
        if is_protected(&name) || is_protected(&comm) {
            continue;
        }
        let kib = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup"))
            .ok()
            .and_then(|s| status_field(&s, "Pss:"))
            .unwrap_or(rss);
        match apps.iter_mut().find(|a| a.name == name) {
            Some(a) => {
                a.pids.push(pid);
                a.kib += kib;
            }
            None => apps.push(AppMem { name, pids: vec![pid], kib }),
        }
    }
    apps.retain(|a| a.kib > 0);
    apps.sort_by(|a, b| b.kib.cmp(&a.kib));
    apps
}

/// Send `signal` ("TERM" or "KILL") to every pid.
pub fn signal_pids(pids: &[u32], signal: &str) {
    let mut args = vec![format!("-{signal}"), "--".to_string()];
    args.extend(pids.iter().map(|p| p.to_string()));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let _ = run("kill", &refs);
}

/// The subset of `pids` that still exist (and are not zombies).
pub fn alive_pids(pids: &[u32]) -> Vec<u32> {
    pids.iter()
        .copied()
        .filter(|p| {
            std::fs::read_to_string(format!("/proc/{p}/status"))
                .map(|s| !s.lines().any(|l| l.starts_with("State:") && l.contains('Z')))
                .unwrap_or(false)
        })
        .collect()
}

pub fn human_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i >= 3 { format!("{v:.1}{}", UNITS[i]) } else { format!("{v:.0}{}", UNITS[i]) }
}

// ------------------------------------------------------------------------ disk

/// `(used_bytes, total_bytes)` from `df -P -B1 <path>` output.
pub fn parse_df(out: &str) -> Option<(u64, u64)> {
    let line = out.lines().nth(1)?;
    let f: Vec<&str> = line.split_whitespace().collect();
    Some((f.get(2)?.parse().ok()?, f.get(1)?.parse().ok()?))
}

pub fn read_disk(path: &str) -> Option<(u64, u64)> {
    parse_df(&run("df", &["-P", "-B1", path])?)
}

// ----------------------------------------------------------------------- audio

/// First volume percentage in `pactl get-sink-volume` output.
pub fn parse_volume(out: &str) -> Option<u32> {
    let pct = out.split('/').find(|p| p.trim().ends_with('%'))?;
    pct.trim().trim_end_matches('%').trim().parse().ok()
}

pub fn parse_mute(out: &str) -> bool {
    out.contains("yes")
}

pub struct Audio {
    pub volume: u32,
    pub muted: bool,
}

pub fn read_audio() -> Option<Audio> {
    let vol = parse_volume(&run("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])?)?;
    let muted = run("pactl", &["get-sink-mute", "@DEFAULT_SINK@"]).map(|o| parse_mute(&o)).unwrap_or(false);
    Some(Audio { volume: vol, muted })
}

pub fn set_volume(percent: u32) {
    let arg = format!("{}%", percent.min(150));
    spawn("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &arg]);
}

pub fn toggle_mute() {
    spawn("pactl", &["set-sink-mute", "@DEFAULT_SINK@", "toggle"]);
}

/// `(name, description)` for each output device.
pub fn read_sinks() -> Vec<(String, String)> {
    let Some(short) = run("pactl", &["list", "short", "sinks"]) else { return vec![] };
    let long = run("pactl", &["list", "sinks"]).unwrap_or_default();
    let descs: Vec<String> = long
        .lines()
        .filter_map(|l| l.trim().strip_prefix("Description:").map(|d| d.trim().to_string()))
        .collect();
    short
        .lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let name = l.split('\t').nth(1)?.to_string();
            let desc = descs.get(i).cloned().unwrap_or_else(|| name.clone());
            Some((name, desc))
        })
        .collect()
}

// --------------------------------------------------------------------- network

/// Split a `nmcli -t` line on unescaped colons.
pub fn split_nmcli(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    out.last_mut().unwrap().push(n);
                }
            }
            ':' => out.push(String::new()),
            c => out.last_mut().unwrap().push(c),
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub enum NetKind {
    Wifi,
    Ethernet,
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Net {
    pub kind: NetKind,
    pub name: String,
}

pub fn parse_net(out: &str) -> Net {
    let mut best = Net { kind: NetKind::None, name: String::new() };
    for line in out.lines() {
        let f = split_nmcli(line);
        if f.len() < 3 || f[1] != "connected" {
            continue;
        }
        let kind = match f[0].as_str() {
            "wifi" => NetKind::Wifi,
            "ethernet" => NetKind::Ethernet,
            _ => continue,
        };
        // Prefer wired over wireless.
        if best.kind == NetKind::None || kind == NetKind::Ethernet {
            best = Net { kind, name: f[2].clone() };
        }
    }
    best
}

pub fn read_net() -> Net {
    run("nmcli", &["-t", "-f", "TYPE,STATE,CONNECTION", "device"]).map(|o| parse_net(&o)).unwrap_or(Net {
        kind: NetKind::None,
        name: String::new(),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct WifiAp {
    pub in_use: bool,
    pub ssid: String,
    pub signal: u32,
    pub secured: bool,
}

pub fn parse_wifi(out: &str) -> Vec<WifiAp> {
    let mut list: Vec<WifiAp> = out
        .lines()
        .filter_map(|l| {
            let f = split_nmcli(l);
            if f.len() < 4 || f[1].is_empty() {
                return None;
            }
            Some(WifiAp {
                in_use: f[0] == "*",
                ssid: f[1].clone(),
                signal: f[2].parse().unwrap_or(0),
                secured: !f[3].is_empty() && f[3] != "--",
            })
        })
        .collect();
    list.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)));
    let mut seen = std::collections::HashSet::new();
    list.retain(|ap| seen.insert(ap.ssid.clone()));
    list
}

/// Visible networks. Without `rescan` this is NetworkManager's last scan (instant). With `rescan` the
/// adapter scans first, which can take well over the usual 4-second limit on real hardware; if the
/// scan still does not finish, the last known list is returned instead of nothing.
pub fn read_wifi(rescan: bool) -> Vec<WifiAp> {
    let list = |r: &str, secs| {
        run_for(secs, "nmcli", &["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan", r])
    };
    let out = if rescan { list("yes", 20).or_else(|| list("no", 4)) } else { list("no", 4) };
    out.map(|o| parse_wifi(&o)).unwrap_or_default()
}

pub fn wifi_enabled() -> bool {
    run("nmcli", &["radio", "wifi"]).is_some_and(|o| o.trim() == "enabled")
}

pub fn saved_connections() -> Vec<String> {
    run("nmcli", &["-t", "-f", "NAME", "connection", "show"])
        .map(|o| o.lines().map(|l| split_nmcli(l).concat()).collect())
        .unwrap_or_default()
}

// ------------------------------------------------------------------- bluetooth

pub struct Bt {
    pub available: bool,
    pub powered: bool,
    pub devices: Vec<BtDevice>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BtDevice {
    pub mac: String,
    pub name: String,
    pub connected: bool,
}

pub fn parse_bt_devices(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("Device ")?;
            let (mac, name) = rest.split_once(' ')?;
            Some((mac.to_string(), name.to_string()))
        })
        .collect()
}

pub fn read_bt() -> Bt {
    let Some(show) = run("bluetoothctl", &["show"]) else {
        return Bt { available: false, powered: false, devices: vec![] };
    };
    let powered = show.lines().any(|l| l.trim() == "Powered: yes");
    let connected: Vec<String> = run("bluetoothctl", &["devices", "Connected"])
        .map(|o| parse_bt_devices(&o).into_iter().map(|d| d.0).collect())
        .unwrap_or_default();
    let devices = run("bluetoothctl", &["devices", "Paired"])
        .map(|o| parse_bt_devices(&o))
        .unwrap_or_default()
        .into_iter()
        .map(|(mac, name)| BtDevice { connected: connected.contains(&mac), mac, name })
        .collect();
    Bt { available: true, powered, devices }
}

// ------------------------------------------------------------------ network use

/// Total received and sent bytes from /proc/net/dev, for one interface or (empty `iface`) for every
/// interface except loopback.
pub fn parse_net_dev(text: &str, iface: &str) -> Option<(u64, u64)> {
    let mut found = false;
    let (mut rx, mut tx) = (0u64, 0u64);
    for line in text.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else { continue };
        let name = name.trim();
        if (iface.is_empty() && name == "lo") || (!iface.is_empty() && name != iface) {
            continue;
        }
        let f: Vec<u64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
        if f.len() >= 9 {
            rx += f[0];
            tx += f[8];
            found = true;
        }
    }
    found.then_some((rx, tx))
}

pub fn read_net_bytes(iface: &str) -> Option<(u64, u64)> {
    parse_net_dev(&std::fs::read_to_string("/proc/net/dev").ok()?, iface)
}

/// Network interfaces except loopback (for the settings).
pub fn net_interfaces() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("/sys/class/net")
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n != "lo").collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// A transfer rate, e.g. "1.2 MB/s" (or "9.6 Mb/s" with `bits`).
pub fn human_rate(bytes_per_sec: f64, bits: bool) -> String {
    let (mut v, unit) = if bits { (bytes_per_sec * 8.0, "b/s") } else { (bytes_per_sec, "B/s") };
    for prefix in ["", "k", "M", "G"] {
        if v < 1000.0 || prefix == "G" {
            return if prefix.is_empty() || v >= 100.0 { format!("{v:.0} {prefix}{unit}") } else { format!("{v:.1} {prefix}{unit}") };
        }
        v /= 1000.0;
    }
    unreachable!()
}

// ------------------------------------------------------------------ temperature

/// CPU temperature in °C: the package sensor of coretemp / k10temp / zenpower, else a thermal zone.
pub fn read_cpu_temp() -> Option<f64> {
    let milli = |p: std::path::PathBuf| std::fs::read_to_string(p).ok()?.trim().parse::<f64>().ok().map(|v| v / 1000.0);
    if let Ok(dir) = std::fs::read_dir("/sys/class/hwmon") {
        let mut hw: Vec<_> = dir.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        hw.sort();
        for want in ["coretemp", "k10temp", "zenpower", "cpu_thermal", "acpitz"] {
            for h in &hw {
                if std::fs::read_to_string(h.join("name")).is_ok_and(|n| n.trim() == want) {
                    if let Some(t) = milli(h.join("temp1_input")) {
                        return Some(t);
                    }
                }
            }
        }
    }
    let zones: Vec<_> = std::fs::read_dir("/sys/class/thermal").ok()?.filter_map(|e| e.ok()).map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("thermal_zone"))).collect();
    let pick = zones.iter().find(|z| std::fs::read_to_string(z.join("type")).is_ok_and(|t| t.trim() == "x86_pkg_temp")).or(zones.first())?;
    milli(pick.join("temp"))
}

// ---------------------------------------------------------------------- battery

#[derive(Debug, Clone, PartialEq)]
pub struct Battery {
    pub percent: u32,
    pub charging: bool,
    pub full: bool,
    /// Minutes until empty (discharging) or full (charging), when the hardware reports it.
    pub minutes: Option<u32>,
}

/// Parse a power_supply `uevent` file.
pub fn parse_battery(uevent: &str) -> Option<Battery> {
    let get = |k: &str| uevent.lines().find_map(|l| l.strip_prefix(&format!("POWER_SUPPLY_{k}=")).map(str::trim));
    let num = |k: &str| get(k).and_then(|v| v.parse::<f64>().ok());
    let percent = num("CAPACITY")? as u32;
    let status = get("STATUS").unwrap_or("Unknown");
    let charging = status == "Charging";
    // Energy (µWh / µW) or charge (µAh / µA) pairs, depending on the hardware.
    let (now, full, rate) = match (num("ENERGY_NOW"), num("ENERGY_FULL"), num("POWER_NOW")) {
        (Some(n), Some(f), r) => (Some(n), Some(f), r),
        _ => (num("CHARGE_NOW"), num("CHARGE_FULL"), num("CURRENT_NOW")),
    };
    let minutes = match (now, full, rate) {
        (Some(n), Some(f), Some(r)) if r > 0.0 => Some(((if charging { f - n } else { n } / r) * 60.0).max(0.0) as u32),
        _ => None,
    };
    Some(Battery { percent: percent.min(100), charging, full: status == "Full", minutes })
}

/// True for the computer's own battery; false for mains adapters and for batteries inside
/// keyboards, mice and other peripherals (which the kernel marks `scope=Device`).
pub fn is_system_battery(p: &std::path::Path) -> bool {
    std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "Battery")
        && !std::fs::read_to_string(p.join("scope")).is_ok_and(|s| s.trim() == "Device")
}

pub fn read_battery() -> Option<Battery> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for e in dir.filter_map(|e| e.ok()) {
        let p = e.path();
        if is_system_battery(&p) {
            if let Some(b) = std::fs::read_to_string(p.join("uevent")).ok().and_then(|u| parse_battery(&u)) {
                return Some(b);
            }
        }
    }
    None
}

/// Battery of a keyboard, mouse or other peripheral.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceBattery {
    pub name: String,
    /// Exact charge, when the device reports one.
    pub percent: Option<u32>,
    /// Coarse level ("Low", "Normal", "Full" …) for devices that report no percentage.
    pub level: Option<String>,
    pub charging: bool,
}

/// First MAC address found in `s` (e.g. inside "hid-aa:bb:cc:dd:ee:ff-battery"), upper-cased.
fn find_mac(s: &str) -> Option<String> {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(16)).filter_map(|i| s.get(i..i + 17)).find(|w| valid_mac(w)).map(str::to_uppercase)
}

/// Batteries the kernel exposes for HID peripherals (USB / wireless receivers, Logitech
/// HID++, classic Bluetooth). Returns `(mac if any, battery)`.
fn sysfs_device_batteries() -> Vec<(Option<String>, DeviceBattery)> {
    let Ok(dir) = std::fs::read_dir("/sys/class/power_supply") else { return vec![] };
    let mut out = vec![];
    for e in dir.flatten() {
        let p = e.path();
        let read = |f: &str| std::fs::read_to_string(p.join(f)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        if read("type").as_deref() != Some("Battery") || read("scope").as_deref() != Some("Device") {
            continue;
        }
        let file = e.file_name().to_string_lossy().into_owned();
        // The HID device's input node usually carries the friendliest name.
        let input_name = std::fs::read_dir(p.join("device/input"))
            .ok()
            .and_then(|d| d.flatten().find_map(|i| std::fs::read_to_string(i.path().join("name")).ok()))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let name = read("model_name").or(input_name).unwrap_or_else(|| file.clone());
        let percent = read("capacity").and_then(|c| c.parse::<u32>().ok()).map(|c| c.min(100));
        let level = read("capacity_level").filter(|l| l != "Unknown");
        if percent.is_none() && level.is_none() {
            continue; // device has not reported yet
        }
        let charging = read("status").as_deref() == Some("Charging");
        out.push((find_mac(&file), DeviceBattery { name, percent, level, charging }));
    }
    out
}

/// "Battery Percentage: 0x5a (90)" from `bluetoothctl info`.
pub fn parse_bt_battery(info: &str) -> Option<u32> {
    let l = info.lines().find_map(|l| l.trim().strip_prefix("Battery Percentage:"))?;
    let v = l.rsplit_once('(').map(|(_, r)| r.trim_end_matches(')').trim()).unwrap_or(l.trim());
    v.parse::<u32>().ok().map(|v| v.min(100))
}

/// Every BlueZ D-Bus object with its interfaces and properties (`GetManagedObjects`).
fn bluez_objects() -> serde_json::Value {
    run("busctl", &["--system", "--json=short", "call", "org.bluez", "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects"])
        .and_then(|o| serde_json::from_str::<serde_json::Value>(&o).ok())
        .map(|mut v| v["data"][0].take())
        .unwrap_or_default()
}

/// Object paths of the GATT Battery Level characteristics (0x2A19) of the device `mac`.
pub fn gatt_battery_paths(objects: &serde_json::Value, mac: &str) -> Vec<String> {
    let dev = format!("/dev_{}/", mac.to_uppercase().replace(':', "_"));
    let Some(map) = objects.as_object() else { return vec![] };
    let mut paths: Vec<String> = map
        .iter()
        .filter(|(path, _)| path.contains(&dev))
        .filter(|(_, ifaces)| {
            ifaces["org.bluez.GattCharacteristic1"]["UUID"]["data"].as_str().is_some_and(|u| u.starts_with("00002a19-"))
        })
        .map(|(path, _)| path.clone())
        .collect();
    paths.sort();
    paths
}

/// Ask the device for its current battery level (one byte, 0–100).
fn read_gatt_battery(path: &str) -> Option<u32> {
    let out = run("busctl", &["--system", "--json=short", "call", "org.bluez", path, "org.bluez.GattCharacteristic1", "ReadValue", "a{sv}", "0"])?;
    let v: serde_json::Value = serde_json::from_str(&out).ok()?;
    v["data"][0][0].as_u64().filter(|&b| b <= 100).map(|b| b as u32)
}

/// BlueZ's `Battery1.Percentage` for the device `mac`.
fn bluez_battery1(objects: &serde_json::Value, mac: &str) -> Option<u32> {
    let suffix = format!("/dev_{}", mac.to_uppercase().replace(':', "_"));
    let map = objects.as_object()?;
    map.iter()
        .find(|(p, _)| p.ends_with(&suffix))
        .and_then(|(_, i)| i["org.bluez.Battery1"]["Percentage"]["data"].as_u64())
        .map(|v| v.min(100) as u32)
}

/// Last seen (direct read, BlueZ, shown) per device MAC.
static BT_BATTERY_SEEN: std::sync::Mutex<Vec<(String, Option<u32>, Option<u32>, u32)>> = std::sync::Mutex::new(Vec::new());

/// Pick the reading that changed most recently; a source stuck on an old value loses as
/// soon as the other one moves. Without history, the direct read wins: BlueZ's value is
/// read once at connect and goes stale on devices that never send notifications.
fn pick_bt_battery(mac: &str, live: Option<u32>, bluez: Option<u32>) -> Option<u32> {
    let mut seen = BT_BATTERY_SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let prev = seen.iter().position(|s| s.0 == mac);
    let shown = match prev.map(|i| seen[i].clone()) {
        Some((_, pl, pb, shown)) => {
            if live.is_some() && live != pl {
                live
            } else if bluez.is_some() && bluez != pb {
                bluez
            } else {
                Some(shown)
            }
        }
        None => live.or(bluez),
    }?;
    let entry = (mac.to_string(), live, bluez, shown);
    match prev {
        Some(i) => seen[i] = entry,
        None => seen.push(entry),
    }
    Some(shown)
}

/// Batteries of connected keyboards, mice and other peripherals, from the kernel and BlueZ.
pub fn read_device_batteries() -> Vec<DeviceBattery> {
    let sysfs = sysfs_device_batteries();
    let mut out: Vec<DeviceBattery> = vec![];
    // Bluetooth LE devices report through BlueZ only; prefer BlueZ's name for BT devices.
    let connected = run("bluetoothctl", &["devices", "Connected"]).map(|o| parse_bt_devices(&o)).unwrap_or_default();
    let objects = if connected.is_empty() { serde_json::Value::Null } else { bluez_objects() };
    let mut bt_macs = vec![];
    for (mac, name) in connected {
        // Two sources, and either can go stale depending on the firmware: a direct read of
        // the device's Battery Level, and BlueZ's Battery1 (connect-time read + notifications).
        let live = gatt_battery_paths(&objects, &mac).iter().find_map(|c| read_gatt_battery(c));
        let bluez = bluez_battery1(&objects, &mac)
            .or_else(|| run("bluetoothctl", &["info", &mac]).and_then(|i| parse_bt_battery(&strip_ansi(&i))));
        let Some(pct) = pick_bt_battery(&mac, live, bluez) else {
            continue;
        };
        bt_macs.push(mac.to_uppercase());
        out.push(DeviceBattery { name, percent: Some(pct), level: None, charging: false });
    }
    for (mac, b) in sysfs {
        if mac.is_some_and(|m| bt_macs.contains(&m)) {
            continue;
        }
        out.push(b);
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_parsing() {
        let a = parse_cpu("cpu  100 0 100 700 100 0 0 0 0 0\ncpu0 1 1 1 1 1").unwrap();
        let b = parse_cpu("cpu  200 0 200 750 150 0 0 0 0 0").unwrap();
        // delta: total 300, idle 100 → 66.7 % busy
        assert!((cpu_percent(a, b) - 66.667).abs() < 0.01);
        assert_eq!(cpu_percent(a, a), 0.0);
        assert!(parse_cpu("garbage").is_none());
    }

    #[test]
    fn meminfo() {
        let (t, a) = parse_meminfo("MemTotal:  8000 kB\nMemFree: 1 kB\nMemAvailable:  2000 kB\n").unwrap();
        assert_eq!((t, a), (8000, 2000));
        let x = parse_mem_extra("Buffers: 10 kB\nCached:  100 kB\nSwapCached: 5 kB\nSwapTotal: 800 kB\nSwapFree: 300 kB\nSReclaimable: 20 kB\n");
        assert_eq!(x, (130, 500, 800));
        assert!(is_protected("saforawm") && is_protected("pipewire") && !is_protected("firefox"));
    }

    #[test]
    fn df() {
        let out = "Filesystem 1-blocks Used Available Capacity Mounted on\n/dev/sda1 1000 400 600 40% /\n";
        assert_eq!(parse_df(out), Some((400, 1000)));
    }

    #[test]
    fn volume() {
        let out = "Volume: front-left: 42000 /  64% / -11.99 dB,   front-right: 42000 /  64% / -11.99 dB\n";
        assert_eq!(parse_volume(out), Some(64));
        assert!(parse_mute("Mute: yes"));
        assert!(!parse_mute("Mute: no"));
    }

    #[test]
    fn nmcli_escapes() {
        assert_eq!(split_nmcli(r"wifi:connected:My\:Net"), vec!["wifi", "connected", "My:Net"]);
    }

    #[test]
    fn network_prefers_wired() {
        let out = "wifi:connected:Home\nethernet:connected:Wired connection 1\nlo:unmanaged:\n";
        assert_eq!(parse_net(out), Net { kind: NetKind::Ethernet, name: "Wired connection 1".into() });
        assert_eq!(parse_net("wifi:disconnected:\n").kind, NetKind::None);
    }

    #[test]
    fn wifi_list() {
        let out = " :Cafe:55:WPA2\n*:Home:80:WPA2\n :Open:30:\n :Cafe:40:WPA2\n";
        let aps = parse_wifi(out);
        assert_eq!(aps[0].ssid, "Home");
        assert!(aps[0].in_use);
        assert_eq!(aps.len(), 3);
        assert!(!aps.iter().find(|a| a.ssid == "Open").unwrap().secured);
    }

    #[test]
    fn bt_devices() {
        let d = parse_bt_devices("Device AA:BB:CC:DD:EE:FF My Headset\nnoise\n");
        assert_eq!(d, vec![("AA:BB:CC:DD:EE:FF".to_string(), "My Headset".to_string())]);
    }

    #[test]
    fn bytes() {
        assert_eq!(human_bytes(512), "512B");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0G");
    }

    #[test]
    fn wifi_list_dedupes_by_name_not_adjacency() {
        let out = "*:Home:80:WPA2\n:Cafe:70:WPA2\n:Home:40:WPA2\n:Cafe:30:WPA2\n";
        let aps = parse_wifi(out);
        assert_eq!(aps.len(), 2);
        assert!(aps[0].in_use && aps[0].ssid == "Home");
    }

    #[test]
    fn net_dev() {
        let t = "Inter-|   Receive |  Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n    lo: 500 5 0 0 0 0 0 0 500 5 0 0 0 0 0 0\n  wlan0: 1000 10 0 0 0 0 0 0 2000 20 0 0 0 0 0 0\n  eth0: 30 1 0 0 0 0 0 0 40 1 0 0 0 0 0 0\n";
        assert_eq!(parse_net_dev(t, ""), Some((1030, 2040)));
        assert_eq!(parse_net_dev(t, "wlan0"), Some((1000, 2000)));
        assert_eq!(parse_net_dev(t, "nope"), None);
        assert_eq!(human_rate(512.0, false), "512 B/s");
        assert_eq!(human_rate(1500.0, false), "1.5 kB/s");
        assert_eq!(human_rate(1_250_000.0, true), "10.0 Mb/s");
    }

    #[test]
    fn battery() {
        let u = "POWER_SUPPLY_STATUS=Discharging\nPOWER_SUPPLY_CAPACITY=50\nPOWER_SUPPLY_ENERGY_NOW=20000000\nPOWER_SUPPLY_ENERGY_FULL=40000000\nPOWER_SUPPLY_POWER_NOW=10000000\n";
        assert_eq!(parse_battery(u), Some(Battery { percent: 50, charging: false, full: false, minutes: Some(120) }));
        let c = "POWER_SUPPLY_STATUS=Charging\nPOWER_SUPPLY_CAPACITY=75\nPOWER_SUPPLY_CHARGE_NOW=3000\nPOWER_SUPPLY_CHARGE_FULL=4000\nPOWER_SUPPLY_CURRENT_NOW=2000\n";
        assert_eq!(parse_battery(c).unwrap().minutes, Some(30));
        assert_eq!(parse_battery("POWER_SUPPLY_STATUS=Full\n"), None);
        assert_eq!(parse_bt_battery("\tName: Mouse\n\tBattery Percentage: 0x5a (90)\n"), Some(90));
        assert_eq!(parse_bt_battery("\tName: Mouse\n"), None);
        assert_eq!(find_mac("hid-aa:bb:cc:dd:ee:0f-battery").as_deref(), Some("AA:BB:CC:DD:EE:0F"));
        assert_eq!(find_mac("hidpp_battery_0"), None);
        let objs: serde_json::Value = serde_json::from_str(r#"{
            "/org/bluez/hci0/dev_C4_C5_EE_1D_4E_8F/service0021/char0022": {"org.bluez.GattCharacteristic1": {"UUID": {"type": "s", "data": "00002a19-0000-1000-8000-00805f9b34fb"}}},
            "/org/bluez/hci0/dev_C4_C5_EE_1D_4E_8F/service0021/char0026": {"org.bluez.GattCharacteristic1": {"UUID": {"type": "s", "data": "00002a4d-0000-1000-8000-00805f9b34fb"}}},
            "/org/bluez/hci0/dev_11_22_33_44_55_66/service0010/char0011": {"org.bluez.GattCharacteristic1": {"UUID": {"type": "s", "data": "00002a19-0000-1000-8000-00805f9b34fb"}}}
        }"#).unwrap();
        assert_eq!(gatt_battery_paths(&objs, "c4:c5:ee:1d:4e:8f"), vec!["/org/bluez/hci0/dev_C4_C5_EE_1D_4E_8F/service0021/char0022"]);
        assert_eq!(bluez_battery1(&serde_json::json!({"/org/bluez/hci0/dev_C4_C5_EE_1D_4E_8F": {"org.bluez.Battery1": {"Percentage": {"type": "y", "data": 100}}}}), "c4:c5:ee:1d:4e:8f"), Some(100));
        let m = "TEST:PICK";
        assert_eq!(pick_bt_battery(m, Some(94), Some(100)), Some(94)); // no history: direct read
        assert_eq!(pick_bt_battery(m, Some(94), Some(100)), Some(94)); // nothing moved
        assert_eq!(pick_bt_battery(m, Some(93), Some(100)), Some(93)); // direct read moved
        assert_eq!(pick_bt_battery(m, Some(93), Some(90)), Some(90)); // BlueZ moved
        assert_eq!(pick_bt_battery(m, Some(93), Some(90)), Some(90));
    }
}

// ------------------------------------------------------------ bluetooth controls

pub fn bt_power(on: bool) {
    spawn("bluetoothctl", &["power", if on { "on" } else { "off" }]);
}

/// Scan for `secs` seconds (non-blocking).
pub fn bt_scan(secs: u32) {
    spawn("bluetoothctl", &["--timeout", &secs.to_string(), "scan", "on"]);
}

/// All devices BlueZ currently knows about (paired or just discovered), with paired/connected flags.
pub fn bt_all_devices() -> Vec<(BtDevice, bool)> {
    let all = run("bluetoothctl", &["devices"]).map(|o| parse_bt_devices(&o)).unwrap_or_default();
    let paired: Vec<String> = run("bluetoothctl", &["devices", "Paired"])
        .map(|o| parse_bt_devices(&o).into_iter().map(|d| d.0).collect())
        .unwrap_or_default();
    let connected: Vec<String> = run("bluetoothctl", &["devices", "Connected"])
        .map(|o| parse_bt_devices(&o).into_iter().map(|d| d.0).collect())
        .unwrap_or_default();
    all.into_iter()
        .map(|(mac, name)| {
            let is_paired = paired.contains(&mac);
            (BtDevice { connected: connected.contains(&mac), mac, name }, is_paired)
        })
        .collect()
}

/// A MAC address as printed by bluetoothctl: six hex pairs.
pub fn valid_mac(s: &str) -> bool {
    let p: Vec<&str> = s.split(':').collect();
    p.len() == 6 && p.iter().all(|x| x.len() == 2 && x.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Remove ANSI escape sequences and carriage returns from bluetoothctl output.
/// An escape sequence cut off at the end of `raw` is dropped, so the result for a
/// growing buffer only ever grows (earlier offsets stay valid).
fn strip_ansi(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut it = raw.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\x1b' => {
                if it.peek() == Some(&'[') {
                    it.next();
                    // parameters/intermediates until a final byte in @..~
                    while let Some(&n) = it.peek() {
                        it.next();
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                } else {
                    it.next();
                }
            }
            '\r' => out.push('\n'),
            _ => out.push(c),
        }
    }
    out
}

/// An interactive `bluetoothctl` session with its built-in pairing agent, so the
/// passkey confirmations / PIN requests BlueZ sends during pairing get answered.
/// (A one-shot `bluetoothctl pair` has no agent and BlueZ rejects the pairing with
/// "No agent available".)
struct BtSession {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    raw: Vec<u8>,
    /// Offset in the stripped text up to which output has been consumed.
    seen: usize,
}

impl BtSession {
    fn start() -> Option<Self> {
        use std::{io::Read, process::Stdio};
        let mut child = Command::new("bluetoothctl")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let mut stdout = child.stdout.take()?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Some(BtSession { child, stdin, rx, raw: Vec::new(), seen: 0 })
    }

    /// Output so far without escape codes (a UTF-8 character cut off at the end is left out).
    fn text(&self) -> String {
        let valid = match std::str::from_utf8(&self.raw) {
            Ok(t) => t,
            Err(e) => std::str::from_utf8(&self.raw[..e.valid_up_to()]).unwrap_or(""),
        };
        strip_ansi(valid)
    }

    fn send(&mut self, cmd: &str) {
        use std::io::Write;
        let _ = writeln!(self.stdin, "{cmd}");
        let _ = self.stdin.flush();
    }

    /// Wait until one of `wanted` appears in new output; returns its index.
    /// Agent prompts that show up meanwhile are answered; codes the user must type
    /// on the device are passed to `on_msg`.
    fn wait_for(&mut self, wanted: &[&str], secs: u64, on_msg: &dyn Fn(String)) -> Option<usize> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        loop {
            let text = self.text();
            let new = &text[self.seen.min(text.len())..];

            // Earliest match among the wanted strings and the agent prompts.
            const PROMPTS: [&str; 6] = [
                "(yes/no):",          // Confirm passkey / Authorize service / Accept pairing
                "Enter PIN code:",    // legacy device wants a PIN from us
                "Enter passkey",      // device displays a passkey for us to enter
                "Passkey: ",          // we display a passkey for the user to type
                "PIN code: ",         // we display a PIN for the user to type
                "Request canceled",
            ];
            let mut best: Option<(usize, usize, bool)> = None; // (pos, idx, is_wanted)
            for (i, w) in wanted.iter().enumerate() {
                if let Some(p) = new.find(w) {
                    if best.map_or(true, |b| p < b.0) {
                        best = Some((p, i, true));
                    }
                }
            }
            for (i, w) in PROMPTS.iter().enumerate() {
                if let Some(p) = new.find(w) {
                    if best.map_or(true, |b| p < b.0) {
                        best = Some((p, i, false));
                    }
                }
            }
            if let Some((p, i, is_wanted)) = best {
                let key = if is_wanted { wanted[i] } else { PROMPTS[i] };
                let after_start = p + key.len();
                if is_wanted {
                    self.seen += after_start;
                    return Some(i);
                }
                // Display prompts carry the code after the key: wait for the rest of the line.
                let rest = &new[after_start..];
                let code: Option<String> = if i == 3 || i == 4 {
                    match rest.find('\n') {
                        Some(e) => Some(rest[..e].trim().to_string()),
                        None => None, // incomplete: wait for more output
                    }
                } else {
                    Some(String::new())
                };
                if let Some(code) = code {
                    self.seen += after_start + if i == 3 || i == 4 { code.len() } else { 0 };
                    match i {
                        0 => self.send("yes"),
                        1 => self.send("0000"),
                        2 => {
                            // We cannot read a code off the device; let the attempt fail cleanly.
                            on_msg("This device shows a code to type on the computer, which isn't supported here.".into());
                            self.send("");
                        }
                        3 | 4 => on_msg(format!("Type {code} on the device, then press Enter")),
                        _ => {}
                    }
                    continue;
                }
            }

            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return None;
            }
            match self.rx.recv_timeout(left.min(std::time::Duration::from_millis(200))) {
                Ok(chunk) => self.raw.extend_from_slice(&chunk),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return None, // bluetoothctl exited
            }
        }
    }

    /// Wait for at least one prompt-free moment so earlier output is consumed.
    fn drain(&mut self, ms: u64) {
        let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
        while let Some(left) = end.checked_duration_since(std::time::Instant::now()) {
            match self.rx.recv_timeout(left) {
                Ok(chunk) => self.raw.extend_from_slice(&chunk),
                Err(_) => break,
            }
        }
        self.seen = self.text().len();
    }
}

impl Drop for BtSession {
    fn drop(&mut self) {
        self.send("pairable off");
        self.send("quit");
        std::thread::sleep(std::time::Duration::from_millis(300));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Pair, trust and connect (blocking, up to about a minute).
/// `on_msg` receives progress text, including a code the user has to type on a keyboard.
/// Returns `Err(reason)` on failure.
pub fn bt_pair(mac: &str, on_msg: &dyn Fn(String)) -> Result<(), String> {
    if !valid_mac(mac) {
        return Err("Invalid device address".into());
    }
    let mut s = BtSession::start().ok_or("bluetoothctl could not be started")?;
    if s.wait_for(&["Agent registered", "Agent is already registered"], 8, on_msg).is_none() {
        return Err("Bluetooth service is not responding".into());
    }
    s.send("power on");
    s.send("pairable on");
    s.send("default-agent");
    s.wait_for(&["Default agent request successful", "Failed to request default agent"], 5, on_msg);
    s.drain(300);

    // Discovered-but-unpaired devices expire ~30 s after a search ends; find it again if needed.
    let dev = format!("Device {mac}");
    s.send(&format!("info {mac}"));
    if s.wait_for(&[dev.as_str(), "not available"], 5, on_msg) == Some(1) {
        on_msg("Looking for the device…".into());
        s.send("scan on");
        let found = s.wait_for(&[dev.as_str()], 20, on_msg).is_some();
        s.send("scan off");
        if !found {
            return Err("Device not found — is it in pairing mode?".into());
        }
    } else {
        s.send("scan off"); // pairing while discovering is unreliable on many adapters
    }
    s.drain(500);

    on_msg("Pairing…".into());
    s.send(&format!("pair {mac}"));
    match s.wait_for(&["Pairing successful", "AlreadyExists", "Failed to pair", "not available"], 45, on_msg) {
        Some(0) | Some(1) => {}
        Some(2) => {
            let text = s.text();
            let reason = text[s.seen.min(text.len())..].lines().next().unwrap_or("").trim().to_string();
            return Err(if reason.contains("AuthenticationFailed") || reason.contains("AuthenticationRejected") {
                "Authentication failed — wrong code or pairing rejected".into()
            } else if reason.contains("ConnectionAttemptFailed") || reason.contains("Page Timeout") {
                "Device didn't answer — keep it in pairing mode and close by".into()
            } else if reason.is_empty() {
                "Pairing failed".into()
            } else {
                format!("Pairing failed{reason}")
            });
        }
        Some(_) => return Err("Device disappeared — put it back in pairing mode".into()),
        None => return Err("Pairing timed out".into()),
    }

    s.send(&format!("trust {mac}"));
    s.wait_for(&["trust succeeded", "Failed to set trusted"], 5, on_msg);
    on_msg("Connecting…".into());
    s.send(&format!("connect {mac}"));
    // Paired is what matters; some devices (e.g. keyboards) connect by themselves later.
    s.wait_for(&["Connection successful", "Failed to connect"], 15, on_msg);
    Ok(())
}

pub fn bt_connect(mac: &str, connect: bool) {
    if valid_mac(mac) {
        spawn("bluetoothctl", &[if connect { "connect" } else { "disconnect" }, mac]);
    }
}

pub fn bt_remove(mac: &str) {
    if valid_mac(mac) {
        spawn("bluetoothctl", &["remove", mac]);
    }
}

// ------------------------------------------------------------------ audio extras

pub fn set_default_sink(name: &str) {
    spawn("pactl", &["set-default-sink", name]);
}

pub fn default_sink() -> String {
    run("pactl", &["get-default-sink"]).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// `(name, description)` of capture devices (monitors of outputs excluded).
pub fn read_sources() -> Vec<(String, String)> {
    let Some(short) = run("pactl", &["list", "short", "sources"]) else { return vec![] };
    let long = run("pactl", &["list", "sources"]).unwrap_or_default();
    let descs: Vec<String> = long
        .lines()
        .filter_map(|l| l.trim().strip_prefix("Description:").map(|d| d.trim().to_string()))
        .collect();
    short
        .lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let name = l.split('\t').nth(1)?.to_string();
            if name.ends_with(".monitor") {
                return None;
            }
            Some((name.clone(), descs.get(i).cloned().unwrap_or(name)))
        })
        .collect()
}

pub fn set_default_source(name: &str) {
    spawn("pactl", &["set-default-source", name]);
}

pub fn source_volume() -> Option<u32> {
    parse_volume(&run("pactl", &["get-source-volume", "@DEFAULT_SOURCE@"])?)
}

pub fn set_source_volume(percent: u32) {
    spawn("pactl", &["set-source-volume", "@DEFAULT_SOURCE@", &format!("{}%", percent.min(150))]);
}

// ----------------------------------------------------------------- network extras

/// Join a Wi-Fi network and wait (up to 45 s) until it is up. `Err` carries nmcli's reason.
pub fn wifi_connect(ssid: &str, password: Option<&str>) -> Result<(), String> {
    let mut args = vec!["--wait", "40", "device", "wifi", "connect", ssid];
    if let Some(p) = password.filter(|p| !p.is_empty()) {
        args.push("password");
        args.push(p);
    }
    nmcli_connect(&args)
}

/// Join a network that doesn't broadcast its name.
pub fn wifi_connect_hidden(ssid: &str, password: Option<&str>) -> Result<(), String> {
    let mut args = vec!["--wait", "40", "device", "wifi", "connect", ssid, "hidden", "yes"];
    if let Some(p) = password.filter(|p| !p.is_empty()) {
        args.push("password");
        args.push(p);
    }
    nmcli_connect(&args)
}

fn nmcli_connect(args: &[&str]) -> Result<(), String> {
    let out = Command::new("timeout").arg("45").arg("nmcli").args(args).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    if out.status.code() == Some(124) {
        return Err("Timed out waiting for the network.".into());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let msg = err.lines().map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or("Could not connect.");
    Err(msg.strip_prefix("Error: ").unwrap_or(msg).to_string())
}

/// Whether claude.ai (the internet, not just a local network) can be reached. Safora turns off NetworkManager's own
/// connectivity checks, so NetworkManager only rules out "no network at all"; otherwise one HEAD request to
/// claude.ai, the host the Claude Code install downloads from, decides.
pub fn internet_online() -> bool {
    if run("nmcli", &["-t", "networking", "connectivity"]).as_deref().map(str::trim) == Some("none") {
        return false;
    }
    run_for(8, "curl", &["-sS", "-o", "/dev/null", "--max-time", "6", "-I", "https://claude.ai"]).is_some()
}

pub fn wifi_forget(name: &str) {
    spawn("nmcli", &["connection", "delete", "id", name]);
}

pub fn set_wifi(on: bool) {
    spawn("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]);
}

/// First IPv4 address of the active connection, if any.
pub fn ip_address() -> Option<String> {
    let out = run("nmcli", &["-t", "-f", "IP4.ADDRESS", "device", "show"])?;
    out.lines().find_map(|l| l.split_once(':').map(|(_, v)| v.trim().to_string())).filter(|v| !v.is_empty())
}
