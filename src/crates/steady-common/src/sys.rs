//! System data sources and controls (/proc readers, nmcli, pactl, bluetoothctl).
//! Parsing is separated from I/O so it can be unit tested.

use std::process::Command;

// --------------------------------------------------------------------- helpers

/// Run a command with a 4 second ceiling and return stdout on success.
pub fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("timeout").arg("4").arg(cmd).args(args).output().ok()?;
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

/// Visible networks. With `rescan` the adapter is asked to scan first (takes a few seconds).
pub fn read_wifi(rescan: bool) -> Vec<WifiAp> {
    let rescan = if rescan { "yes" } else { "no" };
    run("nmcli", &["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan", rescan])
        .map(|o| parse_wifi(&o))
        .unwrap_or_default()
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

/// Pair, trust and connect (blocking, up to ~30 s). Returns true on success.
pub fn bt_pair(mac: &str) -> bool {
    if !valid_mac(mac) {
        return false;
    }
    let ok = Command::new("timeout")
        .args(["30", "bluetoothctl", "pair", mac])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ok {
        let _ = run("bluetoothctl", &["trust", mac]);
        let _ = Command::new("timeout").args(["15", "bluetoothctl", "connect", mac]).output();
    }
    ok
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
