//! Claude Code usage limits for the bar's `claude` widget, without any network access of our own.
//!
//! Claude Code (the command-line tool) hands its status-line command a JSON description of the
//! session on stdin, including the plan's rate limits (`rate_limits.five_hour` / `seven_day`:
//! `used_percentage` and `resets_at`). `saforactl claude-status` is installed as that command: it
//! stores the two windows in `$XDG_RUNTIME_DIR/safora/claude-usage.json` (memory-backed, private,
//! gone at logout) and prints a short status line back to Claude Code. The bar only reads that file,
//! and only while a `claude` process is running, so the numbers change only when Claude Code does.

use std::{
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The status-line command Safora puts into Claude Code's settings.
pub const HOOK_COMMAND: &str = "saforactl claude-status";
/// Seconds between extra status-line runs while Claude Code sits idle (it also runs on every update).
pub const HOOK_REFRESH_SECS: u64 = 30;

/// One limit window (the 5-hour session or the week).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Window {
    /// 0 – 100.
    pub used: f64,
    /// Unix time (seconds) when the window starts over, if Claude Code said.
    pub resets_at: Option<i64>,
}

impl Window {
    /// Has the window started over since Claude Code last reported it?
    pub fn has_reset(&self, now: i64) -> bool {
        self.resets_at.is_some_and(|t| t <= now)
    }
    /// Percentage in use right now: a window that has reset is empty again.
    pub fn used_now(&self, now: i64) -> f64 {
        if self.has_reset(now) { 0.0 } else { self.used.clamp(0.0, 100.0) }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    /// Unix time of the status-line run that wrote this.
    pub updated: i64,
    /// False when plan limits don't apply (API key, Bedrock, Vertex): Claude Code sends none.
    pub available: bool,
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
    /// Model name Claude Code showed, e.g. "Opus 5.5".
    pub model: String,
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// RFC 3339 time ("2026-11-04T18:30:00Z", "…+01:00", fractions allowed) → Unix seconds.
fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?);
    let mut rest = &s[19..];
    if let Some(r) = rest.strip_prefix('.') {
        rest = r.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest {
        "Z" | "z" | "" => 0,
        o if o.len() == 6 && (o.starts_with('+') || o.starts_with('-')) && &o[3..4] == ":" => {
            let v = o[1..3].parse::<i64>().ok()? * 3600 + o[4..6].parse::<i64>().ok()? * 60;
            if o.starts_with('-') { -v } else { v }
        }
        _ => return None,
    };
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se - offset)
}

/// `resets_at` arrives as Unix seconds; accept milliseconds and RFC 3339 text as well.
fn parse_time(v: &Value) -> Option<i64> {
    let n = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => match s.trim().parse::<f64>() {
            Ok(n) => n,
            Err(_) => return parse_rfc3339(s.trim()),
        },
        _ => return None,
    };
    Some(if n > 1e11 { (n / 1000.0) as i64 } else { n as i64 })
}

fn window(v: &Value) -> Option<Window> {
    let used = v.get("used_percentage").or_else(|| v.get("utilization"))?.as_f64()?;
    Some(Window { used, resets_at: v.get("resets_at").and_then(parse_time) })
}

/// Turn the JSON Claude Code gives its status-line command into [`Usage`].
pub fn from_statusline(input: &Value, now: i64) -> Usage {
    let rl = input.get("rate_limits").filter(|v| v.is_object());
    let five_hour = rl.and_then(|r| r.get("five_hour")).and_then(window);
    let seven_day = rl.and_then(|r| r.get("seven_day")).and_then(window);
    let available = input.get("rate_limits_available").and_then(Value::as_bool).unwrap_or(five_hour.is_some() || seven_day.is_some());
    let model = input
        .get("model")
        .and_then(|m| m.get("display_name").or_else(|| m.get("id")))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Usage { updated: now, available, five_hour, seven_day, model }
}

/// "2 h 14 min", "3 d 4 h", "45 s".
pub fn span(secs: i64) -> String {
    let s = secs.max(0);
    let (d, h, m) = (s / 86_400, s % 86_400 / 3600, s % 3600 / 60);
    if d > 0 {
        format!("{d} d {h} h")
    } else if h > 0 {
        format!("{h} h {m} min")
    } else if m > 0 {
        format!("{m} min")
    } else {
        format!("{s} s")
    }
}

/// The text shown at the bottom of Claude Code.
pub fn status_text(u: &Usage, now: i64) -> String {
    let mut parts = Vec::new();
    if !u.model.is_empty() {
        parts.push(u.model.clone());
    }
    if let Some(w) = u.five_hour {
        let left = w.resets_at.filter(|_| !w.has_reset(now)).map(|t| format!(" (resets in {})", span(t - now))).unwrap_or_default();
        parts.push(format!("5h {:.0}%{left}", w.used_now(now)));
    }
    if let Some(w) = u.seven_day {
        parts.push(format!("week {:.0}%", w.used_now(now)));
    }
    parts.join(" · ")
}

// ------------------------------------------------------------------- the data file

fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_dir()) {
        Some(d) => d.join("safora"),
        None => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| "/tmp".into()).join(".cache/safora"),
    }
}

pub fn usage_path() -> PathBuf {
    runtime_dir().join("claude-usage.json")
}

/// Write `data` to `path` through a private temporary file, so readers never see half a file.
fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(data)?;
    drop(f);
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

pub fn save(u: &Usage) -> std::io::Result<()> {
    let path = usage_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    write_private(&path, &serde_json::to_vec(u).unwrap_or_default())
}

pub fn load() -> Option<Usage> {
    serde_json::from_slice(&fs::read(usage_path()).ok()?).ok()
}

/// Is Claude Code running for this user right now (in any terminal)?
pub fn running() -> bool {
    let Ok(me) = fs::metadata("/proc/self").map(|m| m.uid()) else { return false };
    let Ok(dir) = fs::read_dir("/proc") else { return false };
    dir.flatten().any(|e| {
        e.file_name().to_str().is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
            && e.metadata().is_ok_and(|m| m.uid() == me)
            && is_claude_process(&e.path())
    })
}

/// The native installer's binary shows up as `claude` (or, started by its real path, as the version
/// number under `…/claude/versions/`); an npm install runs as `node …/@anthropic-ai/claude-code/cli.js`.
fn is_claude_process(proc_dir: &std::path::Path) -> bool {
    let Ok(comm) = fs::read_to_string(proc_dir.join("comm")) else { return false };
    let comm = comm.trim_end();
    if comm == "claude" {
        return true;
    }
    if fs::read_link(proc_dir.join("exe")).is_ok_and(|p| p.to_string_lossy().contains("/claude/versions/")) {
        return true;
    }
    comm == "node"
        && fs::read(proc_dir.join("cmdline")).is_ok_and(|c| c.split(|&b| b == 0).take(3).any(|a| String::from_utf8_lossy(a).contains("claude-code")))
}

// ------------------------------------------------- hooking into Claude Code's settings

/// `~/.claude/settings.json` (or under `$CLAUDE_CONFIG_DIR`).
pub fn settings_path() -> PathBuf {
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".claude"));
    dir.join("settings.json")
}

#[derive(Debug, Clone, PartialEq)]
pub enum Hook {
    /// Claude Code reports to Safora.
    Connected,
    /// No status line set yet.
    Missing,
    /// The user already has their own status line (its command).
    Other(String),
    /// The settings file exists but isn't valid JSON; Safora leaves it alone.
    Unreadable,
}

fn hook_of(settings: &Value) -> Hook {
    match settings.get("statusLine") {
        None | Some(Value::Null) => Hook::Missing,
        Some(s) => {
            let cmd = s.get("command").and_then(Value::as_str).unwrap_or_default().trim().to_string();
            if cmd == HOOK_COMMAND || cmd.ends_with(&format!("/{HOOK_COMMAND}")) { Hook::Connected } else { Hook::Other(cmd) }
        }
    }
}

fn read_settings() -> Result<Value, Hook> {
    match fs::read_to_string(settings_path()) {
        Err(_) => Ok(Value::Object(Default::default())),
        Ok(t) if t.trim().is_empty() => Ok(Value::Object(Default::default())),
        Ok(t) => serde_json::from_str::<Value>(&t).ok().filter(Value::is_object).ok_or(Hook::Unreadable),
    }
}

pub fn hook() -> Hook {
    read_settings().map(|s| hook_of(&s)).unwrap_or_else(|h| h)
}

fn write_settings(v: &Value) -> Result<(), String> {
    // Write through a symlinked settings file (e.g. kept in a dotfiles repo) instead of replacing the link.
    let path = fs::canonicalize(settings_path()).unwrap_or_else(|_| settings_path());
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let mut text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    text.push('\n');
    write_private(&path, text.as_bytes()).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Make Claude Code report its usage to Safora. Replaces another status line only when `replace`.
pub fn connect(replace: bool) -> Result<(), String> {
    let mut s = read_settings().map_err(|_| format!("{} is not valid JSON; fix or remove it first", settings_path().display()))?;
    match hook_of(&s) {
        Hook::Connected => return Ok(()),
        Hook::Other(cmd) if !replace => return Err(format!("Claude Code already runs its own status line ({cmd})")),
        _ => {}
    }
    s["statusLine"] = serde_json::json!({ "type": "command", "command": HOOK_COMMAND, "refreshInterval": HOOK_REFRESH_SECS });
    write_settings(&s)
}

/// Remove Safora's status line from Claude Code again (someone else's is left alone).
pub fn disconnect() -> Result<(), String> {
    let Ok(mut s) = read_settings() else { return Ok(()) };
    if hook_of(&s) != Hook::Connected {
        return Ok(());
    }
    if let Some(o) = s.as_object_mut() {
        o.remove("statusLine");
    }
    write_settings(&s)?;
    let _ = fs::remove_file(usage_path());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_statusline_input() {
        let v: Value = serde_json::from_str(
            r#"{"model":{"id":"claude-opus-5-5","display_name":"Opus 5.5"},"rate_limits_available":true,
                "rate_limits":{"five_hour":{"used_percentage":42.4,"resets_at":1793860200},
                               "seven_day":{"used_percentage":18,"resets_at":"2026-11-09T09:00:00Z"}}}"#,
        )
        .unwrap();
        let u = from_statusline(&v, 1_793_852_000);
        assert!(u.available);
        assert_eq!(u.model, "Opus 5.5");
        assert_eq!(u.five_hour, Some(Window { used: 42.4, resets_at: Some(1_793_860_200) }));
        assert_eq!(u.seven_day.unwrap().resets_at, Some(1_794_214_800));
        assert_eq!(status_text(&u, 1_793_852_000), "Opus 5.5 · 5h 42% (resets in 2 h 16 min) · week 18%");
        // After the reset the window counts as empty.
        assert_eq!(u.five_hour.unwrap().used_now(1_793_860_200), 0.0);

        let none = from_statusline(&serde_json::json!({"rate_limits_available": false, "rate_limits": null}), 5);
        assert!(!none.available && none.five_hour.is_none());
    }

    #[test]
    fn times() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-11-04T19:30:00.123+01:00"), parse_rfc3339("2026-11-04T18:30:00Z"));
        assert_eq!(parse_rfc3339("2026-11-09T09:00:00Z"), Some(1_794_214_800));
        assert_eq!(parse_time(&serde_json::json!(1_793_860_200_000u64)), Some(1_793_860_200));
        assert_eq!(span(3 * 86_400 + 5 * 3600), "3 d 5 h");
        assert_eq!(span(59), "59 s");
    }

    #[test]
    fn hook_detection() {
        assert_eq!(hook_of(&serde_json::json!({})), Hook::Missing);
        assert_eq!(hook_of(&serde_json::json!({"statusLine": {"type": "command", "command": HOOK_COMMAND}})), Hook::Connected);
        assert_eq!(hook_of(&serde_json::json!({"statusLine": {"command": "/usr/bin/saforactl claude-status"}})), Hook::Connected);
        assert_eq!(hook_of(&serde_json::json!({"statusLine": {"command": "~/bin/mine.sh"}})), Hook::Other("~/bin/mine.sh".into()));
    }
}
