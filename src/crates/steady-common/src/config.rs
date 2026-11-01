//! User configuration, stored as TOML in `~/.config/steady/config.toml`.
//!
//! Every field has a default so a partial (or missing) file always loads.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Every widget the bar knows about, in the order the settings app lists them.
/// `workspaces` is mandatory: the bar always shows it.
pub const ALL_WIDGETS: &[(&str, &str)] = &[
    ("workspaces", "Workspaces"),
    ("apps", "Open applications"),
    ("clock", "Date & time"),
    ("cpu", "CPU usage"),
    ("ram", "Memory"),
    ("disk", "Disk space"),
    ("network", "Network"),
    ("bluetooth", "Bluetooth"),
    ("audio", "Audio (PulseAudio)"),
    ("power", "Power options"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TileMode {
    /// Smart split: each new window splits the focused window's tile along its longer side
    /// (wider than tall → side by side, taller than wide → stacked).
    Smart,
    /// Binary space partition: each new window halves the remaining space.
    Dwindle,
    /// One large master window on the left, the rest stacked on the right.
    Master,
    /// Equal-width columns.
    Columns,
}

impl TileMode {
    pub fn next(self) -> Self {
        match self {
            Self::Smart => Self::Dwindle,
            Self::Dwindle => Self::Master,
            Self::Master => Self::Columns,
            Self::Columns => Self::Smart,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BarPosition {
    Top,
    Bottom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub terminal: String,
    pub launcher: String,
    pub workspaces: usize,
    /// Modifier for window-manager keybindings: "super" or "alt".
    pub modifier: String,
    /// Focus the window under the pointer without clicking.
    pub focus_follows_mouse: bool,
    /// XKB keyboard layout (e.g. "us", "de") and optional variant.
    pub keyboard_layout: String,
    pub keyboard_variant: String,
    /// Commands run once when the session starts.
    pub autostart: Vec<String>,
}

impl Default for General {
    fn default() -> Self {
        Self {
            terminal: "alacritty".into(),
            launcher: "fuzzel".into(),
            workspaces: 9,
            modifier: "super".into(),
            focus_follows_mouse: true,
            keyboard_layout: "us".into(),
            keyboard_variant: String::new(),
            autostart: vec!["steady-bar".into()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub mode: TileMode,
    pub gap_inner: i32,
    pub gap_outer: i32,
    pub border_width: i32,
    /// Fraction of the area given to the master window in `master` mode.
    pub master_ratio: f64,
}

impl Default for Layout {
    fn default() -> Self {
        Self { mode: TileMode::Smart, gap_inner: 8, gap_outer: 8, border_width: 2, master_ratio: 0.55 }
    }
}

/// Look of application windows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Windows {
    /// Corner radius in pixels (0 = square corners).
    pub corner_radius: i32,
    /// Opacity of the focused window, 0.3 – 1.0.
    pub opacity_active: f64,
    /// Opacity of unfocused windows, 0.3 – 1.0.
    pub opacity_inactive: f64,
}

impl Default for Windows {
    fn default() -> Self {
        Self { corner_radius: 10, opacity_active: 1.0, opacity_inactive: 0.94 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WallpaperMode {
    /// Scale to cover the screen, cropping the overflow.
    Fill,
    /// Scale to fit inside the screen, leaving bars of the background color.
    Fit,
    Stretch,
    Center,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Desktop {
    /// Image file (png, jpg, webp). Empty = plain background color.
    pub wallpaper: String,
    pub wallpaper_mode: WallpaperMode,
}

impl Default for Desktop {
    fn default() -> Self {
        Self { wallpaper: "/usr/share/backgrounds/steady/steady-default.png".into(), wallpaper_mode: WallpaperMode::Fill }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// Dark mode for the bar, windows borders and GTK applications.
    pub dark: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self { dark: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    /// "auto" follows the monitor's preferred mode (and a virtual machine window's size);
    /// otherwise "WIDTHxHEIGHT" or "WIDTHxHEIGHT@HERTZ".
    pub mode: String,
    /// "auto" (200% on 4K-class screens), "1" or "2".
    pub scale: String,
    /// Light up every connected monitor (extended desktop). Turn off to use only the first one.
    pub extra_monitors: bool,
}

impl Default for Display {
    fn default() -> Self {
        Self { mode: "auto".into(), scale: "auto".into(), extra_monitors: true }
    }
}

/// Interface scale (1 or 2) for a screen that is `width` pixels wide.
pub fn scale_for(setting: &str, width: u32) -> i32 {
    match setting.trim() {
        "1" => 1,
        "2" => 2,
        _ => if width >= 3000 { 2 } else { 1 },
    }
}

/// Parse a display mode string into (width, height, optional refresh in Hz).
pub fn parse_mode(s: &str) -> Option<(u32, u32, Option<f64>)> {
    let s = s.trim();
    let (size, hz) = match s.split_once('@') {
        Some((a, b)) => (a, Some(b.trim().parse::<f64>().ok()?)),
        None => (s, None),
    };
    let (w, h) = size.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?, hz))
}

/// Screen blanking, suspend and power mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Power {
    /// Minutes without input before the screen goes dark (0 = never).
    pub screen_off_minutes: u32,
    /// Minutes without input before the machine suspends (0 = never).
    pub suspend_minutes: u32,
    /// Minutes without input before the screen locks (0 = never).
    pub lock_minutes: u32,
    /// "performance", "balanced" or "power-saver" (applied with powerprofilesctl when available).
    pub profile: String,
}

impl Default for Power {
    fn default() -> Self {
        Self { screen_off_minutes: 10, suspend_minutes: 0, lock_minutes: 5, profile: "balanced".into() }
    }
}

/// Keyboard, mouse and touchpad behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Input {
    /// Delay before a held key starts repeating (ms) and repeats per second.
    pub repeat_delay_ms: i32,
    pub repeat_rate: i32,
    pub natural_scroll: bool,
    pub tap_to_click: bool,
    /// Pointer speed, -1.0 (slow) to 1.0 (fast).
    pub pointer_speed: f64,
    pub left_handed: bool,
    pub disable_while_typing: bool,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            repeat_delay_ms: 300,
            repeat_rate: 30,
            natural_scroll: false,
            tap_to_click: true,
            pointer_speed: 0.0,
            left_handed: false,
            disable_while_typing: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Accessibility {
    /// Multiplies every font size (1.0 – 2.0).
    pub text_scale: f64,
    pub large_cursor: bool,
    /// Turn off interface animations.
    pub reduce_motion: bool,
}

impl Default for Accessibility {
    fn default() -> Self {
        Self { text_scale: 1.0, large_cursor: false, reduce_motion: false }
    }
}

impl Accessibility {
    pub fn cursor_size(&self) -> u32 {
        if self.large_cursor { 48 } else { 24 }
    }
}

/// Fonts used by GTK applications and the bar.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Fonts {
    pub interface: String,
    pub interface_size: i32,
    pub monospace: String,
    pub monospace_size: i32,
}

impl Default for Fonts {
    fn default() -> Self {
        Self { interface: "Roboto".into(), interface_size: 11, monospace: "Roboto Mono".into(), monospace_size: 11 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Colors {
    /// Desktop background (hex `#rrggbb`).
    pub background: String,
    pub border_active: String,
    pub border_inactive: String,
}

impl Default for Colors {
    fn default() -> Self {
        Self { background: "#0e1319".into(), border_active: "#f2b84b".into(), border_inactive: "#2b3846".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Bar {
    pub position: BarPosition,
    pub height: i32,
    /// Margin around the floating bar in pixels (0 = flush with the screen edge).
    pub margin: i32,
    pub radius: i32,
    pub font: String,
    pub font_size: i32,
    pub background: String,
    pub foreground: String,
    pub accent: String,
    /// 0.0 – 1.0
    pub opacity: f64,
    /// 24-hour clock (false = 12-hour with AM/PM).
    pub clock_24h: bool,
    pub left: Vec<String>,
    pub center: Vec<String>,
    pub right: Vec<String>,
}

impl Default for Bar {
    fn default() -> Self {
        Self {
            position: BarPosition::Top,
            height: 34,
            margin: 6,
            radius: 10,
            font: "Roboto".into(),
            font_size: 11,
            background: "#141b22".into(),
            foreground: "#e6edf3".into(),
            accent: "#f2b84b".into(),
            opacity: 0.94,
            clock_24h: true,
            left: vec!["workspaces".into(), "apps".into()],
            center: vec!["clock".into()],
            right: vec![
                "cpu".into(),
                "ram".into(),
                "disk".into(),
                "network".into(),
                "bluetooth".into(),
                "audio".into(),
                "power".into(),
            ],
        }
    }
}

impl Bar {
    pub fn is_enabled(&self, widget: &str) -> bool {
        self.left.iter().chain(&self.center).chain(&self.right).any(|w| w == widget)
    }

    /// Remove `widget` from every section.
    pub fn remove(&mut self, widget: &str) {
        for list in [&mut self.left, &mut self.center, &mut self.right] {
            list.retain(|w| w != widget);
        }
    }

    /// The bar must always show the workspace switcher; put it back if it was removed.
    pub fn ensure_workspaces(&mut self) {
        if !self.is_enabled("workspaces") {
            self.left.insert(0, "workspaces".into());
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub layout: Layout,
    pub windows: Windows,
    pub desktop: Desktop,
    pub fonts: Fonts,
    pub appearance: Appearance,
    pub display: Display,
    pub power: Power,
    pub input: Input,
    pub accessibility: Accessibility,
    pub colors: Colors,
    pub bar: Bar,
    /// Extra keybindings merged over the defaults, e.g. `"Super+Return" = "spawn:alacritty"`.
    pub keybinds: BTreeMap<String, String>,
}

impl Config {
    pub fn dir() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("/etc"));
        base.join("steady")
    }

    pub fn path() -> PathBuf {
        Self::dir().join("config.toml")
    }

    /// Load the user config, falling back to `/etc/steady/config.toml`, then defaults.
    pub fn load() -> Self {
        for path in [Self::path(), PathBuf::from("/etc/steady/config.toml")] {
            if let Ok(text) = std::fs::read_to_string(&path) {
                match toml::from_str::<Config>(&text) {
                    Ok(mut cfg) => {
                        cfg.bar.ensure_workspaces();
                        cfg.general.workspaces = cfg.general.workspaces.clamp(1, 20);
                        cfg.windows.corner_radius = cfg.windows.corner_radius.clamp(0, 40);
                        cfg.windows.opacity_active = cfg.windows.opacity_active.clamp(0.3, 1.0);
                        cfg.windows.opacity_inactive = cfg.windows.opacity_inactive.clamp(0.3, 1.0);
                        cfg.input.pointer_speed = cfg.input.pointer_speed.clamp(-1.0, 1.0);
                        cfg.input.repeat_rate = cfg.input.repeat_rate.clamp(1, 100);
                        cfg.input.repeat_delay_ms = cfg.input.repeat_delay_ms.clamp(100, 1500);
                        cfg.accessibility.text_scale = cfg.accessibility.text_scale.clamp(1.0, 2.0);
                        return cfg;
                    }
                    Err(e) => eprintln!("steady: {}: {e}", path.display()),
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        // Write-then-rename so the bar's file watcher never sees a half-written file.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }
}

/// Parse `#rrggbb` / `#rrggbbaa` into 0.0–1.0 RGBA.
pub fn parse_color(s: &str) -> [f32; 4] {
    let h = s.trim().trim_start_matches('#');
    let byte = |i: usize| h.get(i..i + 2).and_then(|v| u8::from_str_radix(v, 16).ok());
    match (byte(0), byte(2), byte(4)) {
        (Some(r), Some(g), Some(b)) => {
            let a = byte(6).unwrap_or(255);
            [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0]
        }
        _ => [0.0, 0.0, 0.0, 1.0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip() {
        let text = toml::to_string_pretty(&Config::default()).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.bar.right, Config::default().bar.right);
    }

    #[test]
    fn partial_file_loads() {
        let cfg: Config = toml::from_str("[layout]\ngap_inner = 20\n").unwrap();
        assert_eq!(cfg.layout.gap_inner, 20);
        assert_eq!(cfg.layout.border_width, 2);
    }

    #[test]
    fn workspaces_cannot_be_removed() {
        let mut bar = Bar::default();
        bar.remove("workspaces");
        bar.ensure_workspaces();
        assert!(bar.is_enabled("workspaces"));
    }

    #[test]
    fn display_modes_parse() {
        assert_eq!(parse_mode("1920x1080"), Some((1920, 1080, None)));
        assert_eq!(parse_mode("1280x720@59.94"), Some((1280, 720, Some(59.94))));
        assert_eq!(parse_mode("auto"), None);
        assert_eq!(parse_mode("axb"), None);
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#ff0000"), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(parse_color("garbage"), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn new_sections_have_sane_defaults() {
        let cfg: Config = toml::from_str("[power]\nsuspend_minutes = 30\n").unwrap();
        assert_eq!(cfg.power.suspend_minutes, 30);
        assert_eq!(cfg.power.screen_off_minutes, 10);
        assert_eq!(cfg.power.lock_minutes, 5);
        assert_eq!(cfg.input.repeat_rate, 30);
        assert_eq!(cfg.accessibility.cursor_size(), 24);
    }
}
