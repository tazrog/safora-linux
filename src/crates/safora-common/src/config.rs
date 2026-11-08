//! User configuration, stored as TOML in `~/.config/safora/config.toml`.
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
    ("netspeed", "Network use"),
    ("temperature", "CPU temperature"),
    ("battery", "Battery"),
    ("weather", "Weather"),
    ("claude", "Claude Code usage"),
    ("tray", "App tray"),
    ("spacer", "Spacer"),
    ("separator", "Separator"),
];

/// Widgets that can appear more than once. Extra copies are named `kind#n` (e.g. `spacer#2`).
pub const REPEATABLE: &[&str] = &["spacer", "separator"];

/// The widget kind of a bar entry: `spacer#2` → `spacer`.
pub fn widget_kind(name: &str) -> &str {
    name.split('#').next().unwrap_or(name)
}

/// Display title of a bar entry (`spacer#2` → "Spacer 2").
pub fn widget_title(name: &str) -> String {
    let kind = widget_kind(name);
    let base = ALL_WIDGETS.iter().find(|(k, _)| *k == kind).map(|(_, t)| t.to_string()).unwrap_or_else(|| kind.to_string());
    match name.split_once('#') {
        Some((_, n)) => format!("{base} {n}"),
        None => base,
    }
}

/// Per-widget settings, stored as `[bar.widget.<name>]`. Empty / zero values mean "the widget's default";
/// each widget only reads the fields that apply to it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WidgetOpts {
    /// How a value is drawn: "text", "graph", "meter" or "icon".
    #[serde(skip_serializing_if = "String::is_empty")]
    pub display: String,
    /// Widget-specific format preset (e.g. "percent", "used-total"); for the clock also a custom strftime pattern.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub format: String,
    /// Hide the name in front of the value ("CPU", "RAM", ...).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hide_label: bool,
    /// Update interval in seconds (minutes for the weather).
    #[serde(skip_serializing_if = "is_zero")]
    pub interval: u32,
    /// Width in pixels of a graph, meter or spacer.
    #[serde(skip_serializing_if = "is_zero")]
    pub width: u32,
    /// "metric" or "imperial".
    #[serde(skip_serializing_if = "String::is_empty")]
    pub units: String,
    /// What to measure: the disk's mount point, the network interface, ...
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// Weather: the place, as typed by the user (e.g. "Berlin").
    #[serde(skip_serializing_if = "String::is_empty")]
    pub location: String,
    /// App tray: desktop file IDs, e.g. "firefox.desktop".
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub apps: Vec<String>,
}

/// strftime pattern for a clock format preset (or the user's own pattern).
pub fn clock_pattern(format: &str, h24: bool) -> String {
    let t = if h24 { "%H:%M" } else { "%I:%M %p" };
    let ts = if h24 { "%H:%M:%S" } else { "%I:%M:%S %p" };
    match format {
        "time" => t.into(),
        "seconds" => ts.into(),
        "date" => "%a %d %b %Y".into(),
        "iso" => format!("%Y-%m-%d {t}"),
        custom if custom.contains('%') => custom.into(),
        _ => format!("%a %d %b   {t}"),
    }
}

/// Apps the app tray shows until the user picks their own.
pub const DEFAULT_TRAY: &[&str] = &["firefox.desktop", "Alacritty.desktop", "pcmanfm.desktop", "safora-settings.desktop"];

fn is_zero(v: &u32) -> bool {
    *v == 0
}

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
            autostart: vec!["safora-bar".into()],
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
        Self { wallpaper: "/usr/share/backgrounds/safora/safora-default.png".into(), wallpaper_mode: WallpaperMode::Fill }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// Dark mode for the bar, windows borders and GTK applications.
    pub dark: bool,
    /// Color themes the user made and saved in Settings.
    pub themes: Vec<ThemeColors>,
}

impl Default for Appearance {
    fn default() -> Self {
        Self { dark: true, themes: Vec::new() }
    }
}

/// A named set of desktop colors: one of the presets, or a theme the user saved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeColors {
    pub name: String,
    /// Dark mode for GTK applications while this theme is in use.
    pub dark: bool,
    pub desktop: String,
    pub bar_background: String,
    pub bar_foreground: String,
    pub accent: String,
    pub border_active: String,
    pub border_inactive: String,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self::from_config("", &Config::default())
    }
}

impl ThemeColors {
    /// The colors `cfg` uses right now, under the name `name`.
    pub fn from_config(name: &str, cfg: &Config) -> Self {
        Self {
            name: name.into(),
            dark: cfg.appearance.dark,
            desktop: cfg.colors.background.clone(),
            bar_background: cfg.bar.background.clone(),
            bar_foreground: cfg.bar.foreground.clone(),
            accent: cfg.bar.accent.clone(),
            border_active: cfg.colors.border_active.clone(),
            border_inactive: cfg.colors.border_inactive.clone(),
        }
    }

    /// Switch `cfg` to these colors (and this theme's dark/light mode).
    pub fn apply(&self, cfg: &mut Config) {
        cfg.appearance.dark = self.dark;
        cfg.colors.background = self.desktop.clone();
        cfg.bar.background = self.bar_background.clone();
        cfg.bar.foreground = self.bar_foreground.clone();
        cfg.bar.accent = self.accent.clone();
        cfg.colors.border_active = self.border_active.clone();
        cfg.colors.border_inactive = self.border_inactive.clone();
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
    /// "performance", "balanced" or "power-saver" (applied through power-profiles-daemon when available).
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
        Self { background: "#15101f".into(), border_active: "#3dd6d0".into(), border_inactive: "#2e2542".into() }
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
    /// Settings of individual widgets, keyed by their name in `left`/`center`/`right`.
    pub widget: BTreeMap<String, WidgetOpts>,
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
            background: "#1b1528".into(),
            foreground: "#ece8f5".into(),
            accent: "#3dd6d0".into(),
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
            widget: BTreeMap::new(),
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

    /// Settings of the widget `name` (defaults when none are stored).
    pub fn opts(&self, name: &str) -> WidgetOpts {
        self.widget.get(name).cloned().unwrap_or_default()
    }

    /// A free name for one more copy of a repeatable widget (`spacer`, `spacer#2`, ...).
    pub fn new_instance(&self, kind: &str) -> String {
        if !self.is_enabled(kind) {
            return kind.to_string();
        }
        (2..).map(|n| format!("{kind}#{n}")).find(|n| !self.is_enabled(n)).unwrap_or_else(|| kind.to_string())
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
        base.join("safora")
    }

    pub fn path() -> PathBuf {
        Self::dir().join("config.toml")
    }

    /// Load the user config, falling back to `/etc/safora/config.toml`, then defaults.
    pub fn load() -> Self {
        for path in [Self::path(), PathBuf::from("/etc/safora/config.toml")] {
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
                    Err(e) => eprintln!("safora: {}: {e}", path.display()),
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
    fn custom_theme_roundtrip() {
        let mut cfg = Config::default();
        let mut t = ThemeColors::from_config("Mine", &cfg);
        t.accent = "#ff0000".into();
        t.dark = false;
        cfg.appearance.themes.push(t.clone());
        let back: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!(back.appearance.themes, vec![t.clone()]);
        let mut applied = Config::default();
        t.apply(&mut applied);
        assert_eq!(applied.bar.accent, "#ff0000");
        assert!(!applied.appearance.dark);
        assert_eq!(ThemeColors::from_config("Mine", &applied), t);
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

    #[test]
    fn widget_options_round_trip() {
        let mut cfg = Config::default();
        cfg.bar.right.push("spacer".into());
        let extra = cfg.bar.new_instance("spacer");
        assert_eq!(extra, "spacer#2");
        cfg.bar.right.push(extra.clone());
        cfg.bar.widget.insert("cpu".into(), WidgetOpts { display: "graph".into(), width: 60, ..Default::default() });
        cfg.bar.widget.insert(extra.clone(), WidgetOpts { width: 40, ..Default::default() });
        let text = toml::to_string(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.bar.opts("cpu").display, "graph");
        assert_eq!(back.bar.opts("spacer#2").width, 40);
        assert_eq!(back.bar.opts("ram"), WidgetOpts::default());
        assert_eq!(widget_kind("spacer#2"), "spacer");
        assert_eq!(widget_title("spacer#2"), "Spacer 2");
        assert_eq!(widget_title("cpu"), "CPU usage");
    }

    #[test]
    fn old_configs_without_widget_settings_still_load() {
        let cfg: Config = toml::from_str("[bar]\nright = [\"cpu\", \"clock\"]\n").unwrap();
        assert!(cfg.bar.widget.is_empty());
        assert_eq!(cfg.bar.opts("cpu"), WidgetOpts::default());
    }
}
