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

pub fn read_battery() -> Option<Battery> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for e in dir.filter_map(|e| e.ok()) {
        let p = e.path();
        if std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "Battery") {
            if let Some(b) = std::fs::read_to_string(p.join("uevent")).ok().and_then(|u| parse_battery(&u)) {
                return Some(b);
            }
        }
    }
    None
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

pub fn wifi_connect(ssid: &str, password: Option<&str>) -> bool {
    let mut args = vec!["device", "wifi", "connect", ssid];
    if let Some(p) = password {
        args.push("password");
        args.push(p);
    }
    run("nmcli", &args).is_some()
}

/// Join a network that doesn't broadcast its name.
pub fn wifi_connect_hidden(ssid: &str, password: Option<&str>) -> bool {
    let mut args = vec!["device", "wifi", "connect", ssid, "hidden", "yes"];
    if let Some(p) = password.filter(|p| !p.is_empty()) {
        args.push("password");
        args.push(p);
    }
    run("nmcli", &args).is_some()
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
