//! Applying the user's font and dark-mode choices to GTK applications and the terminal.

use crate::Config;
use std::{fs, io, path::PathBuf};

/// Set `key=value` inside the `[section]` of an ini document, keeping everything else.
pub fn ini_set(text: &str, section: &str, key: &str, value: &str) -> String {
    let header = format!("[{section}]");
    let mut out: Vec<String> = Vec::new();
    let (mut in_section, mut done, mut seen_section) = (false, false, false);
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            if in_section && !done {
                out.push(format!("{key}={value}"));
                done = true;
            }
            in_section = t == header;
            seen_section |= in_section;
        } else if in_section && t.split('=').next().map(str::trim) == Some(key) {
            if !done {
                out.push(format!("{key}={value}"));
                done = true;
            }
            continue;
        }
        out.push(line.to_string());
    }
    if in_section && !done {
        out.push(format!("{key}={value}"));
    } else if !seen_section {
        if !out.is_empty() && !out.last().is_some_and(|l| l.is_empty()) {
            out.push(String::new());
        }
        out.push(header);
        out.push(format!("{key}={value}"));
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
}

const ALACRITTY_MARK: &str = "# managed by safora-settings";

/// Write the interface font into GTK 3/4 settings and the monospace font into Alacritty's config
/// (the latter only if the file doesn't exist or was created by us).
pub fn apply_system_fonts(cfg: &Config) -> io::Result<()> {
    let f = &cfg.fonts;
    let scale = cfg.accessibility.text_scale;
    let scaled = |n: i32| ((n as f64 * scale).round() as i32).max(6);
    let gtk_font = format!("{} {}", f.interface, scaled(f.interface_size));
    let (dark, icons) = if cfg.appearance.dark { ("true", "Papirus-Dark") } else { ("false", "Papirus-Light") };
    for dir in ["gtk-3.0", "gtk-4.0"] {
        let path = config_home().join(dir).join("settings.ini");
        let old = fs::read_to_string(&path).unwrap_or_default();
        let new = ini_set(&old, "Settings", "gtk-font-name", &gtk_font);
        let new = ini_set(&new, "Settings", "gtk-application-prefer-dark-theme", dark);
        let new = ini_set(&new, "Settings", "gtk-icon-theme-name", icons);
        let new = ini_set(&new, "Settings", "gtk-cursor-theme-size", &cfg.accessibility.cursor_size().to_string());
        let new = ini_set(&new, "Settings", "gtk-enable-animations", if cfg.accessibility.reduce_motion { "false" } else { "true" });
        if new != old {
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(&path, new)?;
        }
    }

    let path = config_home().join("alacritty").join("alacritty.toml");
    let ours = fs::read_to_string(&path).map(|t| t.starts_with(ALACRITTY_MARK)).unwrap_or(true);
    if ours {
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(
            &path,
            format!(
                "{ALACRITTY_MARK} (delete this line to take over the file)\n[font]\nsize = {}\n[font.normal]\nfamily = \"{}\"\n[window]\ndecorations = \"None\"\nopacity = 0.96\npadding = {{ x = 6, y = 6 }}\n",
                scaled(f.monospace_size),
                f.monospace.replace('"', "")
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_section_and_key() {
        assert_eq!(ini_set("", "Settings", "a", "1"), "[Settings]\na=1\n");
    }

    #[test]
    fn replaces_existing_key_only() {
        let t = "[Settings]\ngtk-theme-name=Adwaita\ngtk-font-name=Old 9\n[Other]\ngtk-font-name=keep\n";
        let r = ini_set(t, "Settings", "gtk-font-name", "Roboto 12");
        assert!(r.contains("gtk-theme-name=Adwaita"));
        assert!(r.contains("gtk-font-name=Roboto 12"));
        assert!(r.contains("[Other]\ngtk-font-name=keep"));
        assert_eq!(r.matches("gtk-font-name").count(), 2);
    }

    #[test]
    fn appends_to_last_section() {
        let r = ini_set("[Settings]\nx=1", "Settings", "y", "2");
        assert_eq!(r, "[Settings]\nx=1\ny=2\n");
    }
}
