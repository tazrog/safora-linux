//! Optional bar widgets (network use, temperature, battery, weather, app tray, spacer, separator) and
//! the shared "metric" display used by every number widget: plain text, a scrolling graph, a meter
//! or an icon, chosen per widget in Safora Settings → Widgets.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Instant,
};

use gtk4 as gtk;
use gtk::{gio, glib, prelude::*};
use safora_common::config::WidgetOpts;

use crate::{
    sys,
    widgets::{label, menu_button, once, poll, popover_box, set_level, title},
    Ctx,
};

/// One update of a number widget.
pub struct Reading {
    /// 0.0 – 1.0, for the graph and the meter.
    pub frac: f64,
    /// Text shown on the bar.
    pub value: String,
    pub tooltip: String,
    /// "", "warn" or "crit".
    pub level: &'static str,
}

fn display(o: &WidgetOpts) -> &str {
    if o.display.is_empty() { "text" } else { &o.display }
}

/// A small scrolling line graph in the bar's accent color.
fn graph(accent: [f32; 4], width: i32) -> (gtk::DrawingArea, Rc<RefCell<VecDeque<f64>>>) {
    let area = gtk::DrawingArea::new();
    area.set_content_width(width);
    area.set_content_height(16);
    area.set_valign(gtk::Align::Center);
    area.add_css_class("graph");
    let points = (width / 3).max(4) as usize;
    let data = Rc::new(RefCell::new(VecDeque::from(vec![0.0; points])));
    let d = data.clone();
    area.set_draw_func(move |_, cr, w, h| {
        let d = d.borrow();
        let (w, h) = (w as f64, h as f64);
        let step = w / (d.len().max(2) - 1) as f64;
        let y = |v: f64| h - 1.0 - v.clamp(0.0, 1.0) * (h - 2.0);
        let [r, g, b, _] = accent.map(f64::from);
        // Faint baseline so an idle graph is still visible.
        cr.set_source_rgba(r, g, b, 0.25);
        cr.rectangle(0.0, h - 1.0, w, 1.0);
        let _ = cr.fill();
        cr.move_to(0.0, h);
        for (i, v) in d.iter().enumerate() {
            cr.line_to(i as f64 * step, y(*v));
        }
        cr.line_to(w, h);
        cr.close_path();
        cr.set_source_rgba(r, g, b, 0.30);
        let _ = cr.fill();
        for (i, v) in d.iter().enumerate() {
            if i == 0 { cr.move_to(0.0, y(*v)) } else { cr.line_to(i as f64 * step, y(*v)) }
        }
        cr.set_source_rgba(r, g, b, 1.0);
        cr.set_line_width(1.5);
        let _ = cr.stroke();
    });
    (area, data)
}

/// A slim horizontal meter (level bar) in the accent color.
fn meter(width: i32) -> gtk::LevelBar {
    let m = gtk::LevelBar::for_interval(0.0, 1.0);
    for o in ["low", "high", "full"] {
        m.remove_offset_value(Some(o));
    }
    m.set_width_request(width);
    m.set_valign(gtk::Align::Center);
    m.add_css_class("meter");
    m
}

/// A number widget: [name or icon] [graph | meter] [value], updated every `default_secs` (or the
/// user's interval). `compute` turns each sample into a [`Reading`] (it may keep state between calls).
pub fn metric<T: Send + 'static>(
    ctx: &Rc<Ctx>,
    o: &WidgetOpts,
    name: &str,
    icon: &str,
    default_secs: u32,
    work: impl Fn() -> T + Clone + Send + 'static,
    compute: impl FnMut(T) -> Option<Reading> + 'static,
) -> gtk::Widget {
    let mode = display(o).to_string();
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    b.add_css_class("metric");
    if mode == "icon" {
        b.append(&gtk::Image::from_icon_name(icon));
    } else if !o.hide_label {
        b.append(&gtk::Label::new(Some(name)));
    }
    let accent = safora_common::config::parse_color(&ctx.config.bar.accent);
    let g = (mode == "graph").then(|| graph(accent, if o.width > 0 { o.width as i32 } else { 48 }));
    let m = (mode == "meter").then(|| meter(if o.width > 0 { o.width as i32 } else { 40 }));
    if let Some((area, _)) = &g {
        b.append(area);
    }
    if let Some(m) = &m {
        b.append(m);
    }
    let value = gtk::Label::new(Some("–"));
    b.append(&value);

    let secs = if o.interval > 0 { o.interval } else { default_secs };
    let compute = RefCell::new(compute);
    let b2 = b.clone();
    poll(&b, secs as u64 * 1000, work, move |t| {
        let Some(r) = (compute.borrow_mut())(t) else { return };
        value.set_text(&r.value);
        b2.set_tooltip_text(Some(&r.tooltip));
        set_level(&value, r.level);
        if let Some((area, data)) = &g {
            let mut d = data.borrow_mut();
            d.pop_front();
            d.push_back(r.frac);
            area.queue_draw();
        }
        if let Some(m) = &m {
            m.set_value(r.frac.clamp(0.0, 1.0));
        }
    });
    b.upcast()
}

// ----------------------------------------------------------------- network use

pub fn netspeed(ctx: &Rc<Ctx>, o: &WidgetOpts) -> gtk::Widget {
    let iface = o.source.clone();
    let bits = o.units == "bits";
    let show = if o.format.is_empty() { "both".to_string() } else { o.format.clone() };
    let prev = Cell::new(None::<(u64, u64, Instant)>);
    // The graph scales to the busiest moment of the last few minutes.
    let peak = Cell::new(64.0_f64 * 1024.0);
    metric(ctx, o, "NET", "network-transmit-receive-symbolic", 2, move || sys::read_net_bytes(&iface), move |now| {
        let (rx, tx) = now?;
        let t = Instant::now();
        let last = prev.replace(Some((rx, tx, t)));
        let (prx, ptx, pt) = last?;
        let dt = t.duration_since(pt).as_secs_f64().max(0.1);
        let down = rx.saturating_sub(prx) as f64 / dt;
        let up = tx.saturating_sub(ptx) as f64 / dt;
        let busiest = match show.as_str() { "down" => down, "up" => up, _ => down.max(up) };
        peak.set((peak.get() * 0.98).max(busiest).max(16.0 * 1024.0));
        let (d, u) = (sys::human_rate(down, bits), sys::human_rate(up, bits));
        let value = match show.as_str() {
            "down" => format!("↓ {d}"),
            "up" => format!("↑ {u}"),
            _ => format!("↓ {d}  ↑ {u}"),
        };
        Some(Reading { frac: busiest / peak.get(), value, tooltip: format!("Download {d}\nUpload {u}"), level: "" })
    })
}

// ----------------------------------------------------------------- temperature

pub fn temperature(ctx: &Rc<Ctx>, o: &WidgetOpts) -> gtk::Widget {
    let imperial = o.units == "imperial";
    metric(ctx, o, "TEMP", "sensors-temperature-symbolic", 3, sys::read_cpu_temp, move |t| {
        let Some(c) = t else {
            return Some(Reading { frac: 0.0, value: "n/a".into(), tooltip: "No temperature sensor found".into(), level: "" });
        };
        let shown = if imperial { format!("{:.0}°F", c * 9.0 / 5.0 + 32.0) } else { format!("{c:.0}°C") };
        let level = if c >= 90.0 { "crit" } else if c >= 80.0 { "warn" } else { "" };
        Some(Reading { frac: c / 100.0, value: shown.clone(), tooltip: format!("CPU temperature {shown}"), level })
    })
}

// --------------------------------------------------------------------- battery

fn battery_icon(b: &sys::Battery) -> String {
    if b.full {
        return "battery-level-100-charged-symbolic".into();
    }
    let level = (b.percent + 5) / 10 * 10;
    format!("battery-level-{}{}-symbolic", level.min(100), if b.charging { "-charging" } else { "" })
}

fn hm(minutes: u32) -> String {
    format!("{}:{:02}", minutes / 60, minutes % 60)
}

pub fn battery(o: &WidgetOpts) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    let icon = gtk::Image::from_icon_name("battery-missing-symbolic");
    let text = gtk::Label::new(None);
    b.append(&icon);
    b.append(&text);
    b.set_visible(false);
    let format = o.format.clone();
    let secs = if o.interval > 0 { o.interval } else { 20 };
    let b2 = b.clone();
    poll(&b, secs as u64 * 1000, sys::read_battery, move |bat| {
        // Desktops have no battery: the widget simply stays hidden.
        let Some(bat) = bat else {
            b2.set_visible(false);
            return;
        };
        b2.set_visible(true);
        icon.set_icon_name(Some(&battery_icon(&bat)));
        let time = bat.minutes.filter(|m| *m > 0 && !bat.full).map(hm);
        let shown = match format.as_str() {
            "icon" => String::new(),
            "percent-time" => match &time {
                Some(t) => format!("{}%  {t}", bat.percent),
                None => format!("{}%", bat.percent),
            },
            _ => format!("{}%", bat.percent),
        };
        text.set_visible(!shown.is_empty());
        text.set_text(&shown);
        let state = if bat.full { "Fully charged".to_string() } else if bat.charging { "Charging".into() } else { "On battery".into() };
        let detail = match (&time, bat.charging) {
            (Some(t), true) => format!(" — full in {t}"),
            (Some(t), false) => format!(" — {t} left"),
            _ => String::new(),
        };
        b2.set_tooltip_text(Some(&format!("Battery {}%: {state}{detail}", bat.percent)));
        set_level(&text, if !bat.charging && bat.percent <= 10 { "crit" } else if !bat.charging && bat.percent <= 20 { "warn" } else { "" });
    });
    b.upcast()
}

// --------------------------------------------------------------------- weather

#[derive(Clone, Default)]
struct Place {
    query: String,
    name: String,
    lat: f64,
    lon: f64,
}

struct Weather {
    place: String,
    temp: f64,
    feels: f64,
    humidity: f64,
    wind: f64,
    code: i64,
    day: bool,
    /// (weekday, code, min, max)
    days: Vec<(String, i64, f64, f64)>,
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn fetch_json(url: &str) -> Result<serde_json::Value, String> {
    let out = sys::run_for(15, "curl", &["-fsS", "--max-time", "12", "-A", "safora-bar", url]).ok_or("no connection to open-meteo.com")?;
    serde_json::from_str(&out).map_err(|_| "unexpected answer from open-meteo.com".to_string())
}

/// Resolve the location typed in the settings ("Berlin", "Paris, France" or "52.5,13.4").
fn find_place(query: &str) -> Result<Place, String> {
    if let Some((a, b)) = query.split_once(',') {
        if let (Ok(lat), Ok(lon)) = (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
            return Ok(Place { query: query.into(), name: format!("{lat:.2}, {lon:.2}"), lat, lon });
        }
    }
    // The geocoder matches a place name; a trailing ", country" narrows it down.
    let (city, country) = match query.split_once(',') {
        Some((c, k)) => (c.trim(), k.trim().to_lowercase()),
        None => (query.trim(), String::new()),
    };
    let v = fetch_json(&format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=10&language=en&format=json",
        url_encode(city)
    ))?;
    let results = v["results"].as_array().cloned().unwrap_or_default();
    let pick = results
        .iter()
        .find(|r| {
            country.is_empty()
                || ["country", "country_code", "admin1"].iter().any(|k| r[*k].as_str().is_some_and(|s| s.to_lowercase() == country))
        })
        .or(results.first())
        .ok_or_else(|| format!("place “{query}” not found"))?;
    let name = [pick["name"].as_str(), pick["admin1"].as_str(), pick["country"].as_str()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
    Ok(Place {
        query: query.into(),
        name,
        lat: pick["latitude"].as_f64().ok_or("bad coordinates")?,
        lon: pick["longitude"].as_f64().ok_or("bad coordinates")?,
    })
}

fn fetch_weather(query: &str, imperial: bool, cache: &Mutex<Option<Place>>) -> Result<Weather, String> {
    let cached = cache.lock().ok().and_then(|c| c.clone()).filter(|p| p.query == query);
    let place = match cached {
        Some(p) => p,
        None => {
            let p = find_place(query)?;
            if let Ok(mut c) = cache.lock() {
                *c = Some(p.clone());
            }
            p
        }
    };
    let (tu, wu) = if imperial { ("fahrenheit", "mph") } else { ("celsius", "kmh") };
    let v = fetch_json(&format!(
        "https://api.open-meteo.com/v1/forecast?latitude={:.4}&longitude={:.4}\
         &current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,is_day\
         &daily=weather_code,temperature_2m_max,temperature_2m_min&forecast_days=3&timezone=auto\
         &temperature_unit={tu}&wind_speed_unit={wu}",
        place.lat, place.lon
    ))?;
    let c = &v["current"];
    let num = |x: &serde_json::Value| x.as_f64().unwrap_or(f64::NAN);
    let d = &v["daily"];
    let days = (0..3)
        .filter_map(|i| {
            let date = d["time"][i].as_str()?;
            let day = glib::DateTime::from_iso8601(&format!("{date}T12:00:00Z"), None)
                .ok()
                .and_then(|t| t.format("%A").ok())
                .map(|s| s.to_string())
                .unwrap_or_else(|| date.to_string());
            Some((if i == 0 { "Today".to_string() } else { day }, d["weather_code"][i].as_i64()?, num(&d["temperature_2m_min"][i]), num(&d["temperature_2m_max"][i])))
        })
        .collect();
    Ok(Weather {
        place: place.name,
        temp: num(&c["temperature_2m"]),
        feels: num(&c["apparent_temperature"]),
        humidity: num(&c["relative_humidity_2m"]),
        wind: num(&c["wind_speed_10m"]),
        code: c["weather_code"].as_i64().unwrap_or(-1),
        day: c["is_day"].as_i64().unwrap_or(1) == 1,
        days,
    })
}

/// WMO weather code → (description, icon).
fn condition(code: i64, day: bool) -> (&'static str, &'static str) {
    match code {
        0 => ("Clear", if day { "weather-clear-symbolic" } else { "weather-clear-night-symbolic" }),
        1 => ("Mostly clear", if day { "weather-few-clouds-symbolic" } else { "weather-few-clouds-night-symbolic" }),
        2 => ("Partly cloudy", if day { "weather-few-clouds-symbolic" } else { "weather-few-clouds-night-symbolic" }),
        3 => ("Overcast", "weather-overcast-symbolic"),
        45 | 48 => ("Fog", "weather-fog-symbolic"),
        51..=57 => ("Drizzle", "weather-showers-scattered-symbolic"),
        61..=67 | 80..=82 => ("Rain", "weather-showers-symbolic"),
        71..=77 | 85 | 86 => ("Snow", "weather-snow-symbolic"),
        95..=99 => ("Thunderstorm", "weather-storm-symbolic"),
        _ => ("Unknown", "weather-severe-alert-symbolic"),
    }
}

pub fn weather(o: &WidgetOpts) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    let icon = gtk::Image::from_icon_name("weather-overcast-symbolic");
    let text = gtk::Label::new(Some("–"));
    content.append(&icon);
    content.append(&text);
    let (pop, body) = popover_box();
    body.set_width_request(250);

    let query = o.location.trim().to_string();
    if query.is_empty() {
        text.set_text("Set location");
        text.add_css_class("muted");
        body.append(&label("Choose your town in Safora Settings → Widgets → Weather (gear button)."));
        let open = gtk::Button::with_label("Open settings");
        let p = pop.clone();
        open.connect_clicked(move |_| {
            p.popdown();
            let _ = std::process::Command::new("safora-settings").env("SAFORA_SETTINGS_PAGE", "widgets").env("SAFORA_SETTINGS_WIDGET", "weather").spawn();
        });
        body.append(&open);
        return menu_button(&content, &pop).upcast();
    }

    let imperial = o.units == "imperial";
    let unit = if imperial { "°F" } else { "°C" };
    let wind_unit = if imperial { "mph" } else { "km/h" };
    let format = o.format.clone();
    let minutes = if o.interval > 0 { o.interval.max(10) } else { 30 };
    let cache: Arc<Mutex<Option<Place>>> = Arc::new(Mutex::new(None));

    let details = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let refresh_btn = gtk::Button::with_label("Refresh");
    body.append(&details);
    body.append(&refresh_btn);
    let credit = label("Weather data by Open-Meteo.com");
    credit.add_css_class("muted");
    body.append(&credit);

    let apply = {
        let (icon, text, details, content) = (icon.clone(), text.clone(), details.clone(), content.clone());
        Rc::new(move |w: Result<Weather, String>| {
            while let Some(c) = details.first_child() {
                details.remove(&c);
            }
            match w {
                Ok(w) => {
                    let (desc, icon_name) = condition(w.code, w.day);
                    icon.set_icon_name(Some(icon_name));
                    let temp = format!("{:.0}{unit}", w.temp);
                    let shown = match format.as_str() {
                        "icon" => String::new(),
                        "temp-condition" => format!("{temp} {desc}"),
                        _ => temp.clone(),
                    };
                    text.set_visible(!shown.is_empty());
                    text.set_text(&shown);
                    text.remove_css_class("muted");
                    content.set_tooltip_text(Some(&format!("{}: {temp}, {desc}", w.place)));
                    details.append(&title(&w.place));
                    details.append(&label(&format!("{desc}, {temp} (feels like {:.0}{unit})", w.feels)));
                    details.append(&label(&format!("Humidity {:.0}%   Wind {:.0} {wind_unit}", w.humidity, w.wind)));
                    for (day, code, lo, hi) in w.days {
                        let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                        r.append(&gtk::Image::from_icon_name(condition(code, true).1));
                        let d = label(&day);
                        d.set_hexpand(true);
                        r.append(&d);
                        r.append(&label(&format!("{lo:.0}° / {hi:.0}°")));
                        details.append(&r);
                    }
                }
                Err(e) => {
                    icon.set_icon_name(Some("weather-severe-alert-symbolic"));
                    text.set_visible(true);
                    text.set_text("–");
                    text.add_css_class("muted");
                    content.set_tooltip_text(Some(&format!("Weather unavailable: {e}")));
                    details.append(&label(&format!("Weather unavailable: {e}")));
                }
            }
        })
    };
    let work = {
        let (query, cache) = (query.clone(), cache.clone());
        move || fetch_weather(&query, imperial, &cache)
    };
    {
        let apply = apply.clone();
        poll(&content, minutes as u64 * 60_000, work.clone(), move |w| apply(w));
    }
    refresh_btn.connect_clicked(move |_| {
        let apply = apply.clone();
        once(work.clone(), move |w| apply(w));
    });
    menu_button(&content, &pop).upcast()
}

// -------------------------------------------------------------------- app tray

pub fn tray(o: &WidgetOpts) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    let ids: Vec<String> = if o.apps.is_empty() { safora_common::config::DEFAULT_TRAY.iter().map(|s| s.to_string()).collect() } else { o.apps.clone() };
    let installed = gio::AppInfo::all();
    for id in ids {
        let Some(info) = installed.iter().find(|a| a.id().is_some_and(|i| i == id.as_str())).cloned() else { continue };
        let img = match info.icon() {
            Some(i) => gtk::Image::from_gicon(&i),
            None => gtk::Image::from_icon_name("application-x-executable-symbolic"),
        };
        let btn = gtk::Button::new();
        btn.set_child(Some(&img));
        btn.set_tooltip_text(Some(&info.display_name()));
        btn.add_css_class("tray-app");
        btn.connect_clicked(move |_| {
            if let Err(e) = info.launch(&[], None::<&gio::AppLaunchContext>) {
                eprintln!("safora-bar: cannot start {}: {e}", info.display_name());
            }
        });
        b.append(&btn);
    }
    b.upcast()
}

// ------------------------------------------------------------ spacer / separator

pub fn spacer(o: &WidgetOpts) -> gtk::Widget {
    let s = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    s.set_width_request(if o.width > 0 { o.width as i32 } else { 16 });
    s.upcast()
}

pub fn separator() -> gtk::Widget {
    let s = gtk::Separator::new(gtk::Orientation::Vertical);
    s.set_valign(gtk::Align::Fill);
    s.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_and_conditions() {
        assert_eq!(url_encode("São Paulo"), "S%C3%A3o%20Paulo");
        assert_eq!(condition(0, true).0, "Clear");
        assert_eq!(condition(63, true).1, "weather-showers-symbolic");
        assert_eq!(condition(1234, true).0, "Unknown");
        assert_eq!(hm(135), "2:15");
        let b = sys::Battery { percent: 47, charging: true, full: false, minutes: None };
        assert_eq!(battery_icon(&b), "battery-level-50-charging-symbolic");
    }
}
