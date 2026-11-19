#![allow(
    unsafe_code,
    reason = "Local-time formatting uses the reentrant libc time API."
)]
use enzui_core::{
    calendar_semantics::CalendarLabels,
    civil_date::Date,
    localization::{CatalogBuilder, DateStyle, Locale, LocaleCalendarLabels, Localizer},
    time_of_day::TimeOfDay,
};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Supplies template values. System readings are sampled lazily and at most
/// every two seconds, so a scene without them never touches `/proc`.
pub(crate) struct Vars {
    pub public: bool,
    localizer: Localizer,
    labels: Option<LocaleCalendarLabels>,
    custom: HashMap<String, String>,
    cpu: Option<(u64, u64)>,
    cpu_percent: Option<f32>,
    sampled: Option<Instant>,
    mem: Option<(u64, u64)>,
    load: Option<String>,
    frozen_time: Option<i64>,
    demo: Option<f64>,
}

fn local_time(at: i64) -> Option<libc::tm> {
    let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
    let time = at as libc::time_t;
    // SAFETY: time is readable and tm provides properly aligned writable storage for one record.
    let result = unsafe { libc::localtime_r(&time, tm.as_mut_ptr()) };
    if result.is_null() {
        return None;
    }
    // SAFETY: a non-null localtime_r result means it initialized the complete tm record.
    Some(unsafe { tm.assume_init() })
}

fn strftime(format: &str, at: i64) -> String {
    let Ok(format) = std::ffi::CString::new(format) else {
        return String::new();
    };
    let Some(tm) = local_time(at) else {
        return String::new();
    };
    let mut buffer = [0u8; 128];
    // SAFETY: format is NUL-terminated, tm is initialized and buffer has the declared writable capacity.
    let n = unsafe {
        libc::strftime(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            format.as_ptr(),
            &tm,
        )
    };
    String::from_utf8_lossy(&buffer[..n]).into_owned()
}

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

impl Vars {
    pub(crate) fn new() -> Self {
        let mut catalog = CatalogBuilder::new();
        catalog
            .add_resource("es", "enzui-time-am = a. m.\nenzui-time-pm = p. m.\n")
            .expect("valid embedded day periods");
        let preferred = std::env::var("ENZUI_LOCALE")
            .ok()
            .and_then(|value| Locale::from_posix(&value).ok())
            .map_or_else(enzui_core::localization::preferred_locales, |locale| {
                vec![locale]
            });
        let localizer = Localizer::new(catalog.build(), preferred, Locale::default());
        let labels = LocaleCalendarLabels::new(&localizer).ok();
        Self {
            public: false,
            localizer,
            labels,
            custom: HashMap::new(),
            cpu: None,
            cpu_percent: None,
            sampled: None,
            mem: None,
            load: None,
            frozen_time: None,
            demo: None,
        }
    }

    pub(crate) fn set_custom(&mut self, name: String, value: String) -> Result<(), String> {
        let old = self
            .custom
            .get(&name)
            .map_or(0, |value| name.len() + value.len());
        let bytes: usize = self
            .custom
            .iter()
            .map(|(name, value)| name.len() + value.len())
            .sum();
        if (!self.custom.contains_key(&name) && self.custom.len() >= 64)
            || bytes - old + name.len() + value.len() > 32 * 1024
        {
            return Err("custom variables exceed their 64-entry/32-KiB budget".into());
        }
        self.custom.insert(name, value);
        Ok(())
    }

    pub(crate) fn unset_custom(&mut self, name: &str) {
        self.custom.remove(name);
    }

    /// Replaces system readings with smooth, deterministic ones derived from
    /// `t` seconds, so previews show dashboards in motion.
    pub(crate) fn set_demo(&mut self, t: f64) {
        self.demo = Some(t);
    }

    fn localized_time(&self, format: &str, at: i64) -> String {
        let Some(tm) = local_time(at) else {
            return String::new();
        };
        let date = Date::new(tm.tm_year + 1900, (tm.tm_mon + 1) as u32, tm.tm_mday as u32);
        let time = TimeOfDay::new(
            tm.tm_hour as u32,
            tm.tm_min as u32,
            tm.tm_sec.min(59) as u32,
        );
        let mut output = String::new();
        let mut chars = format.chars();
        while let Some(ch) = chars.next() {
            if ch != '%' {
                output.push(ch);
                continue;
            }
            let mut spec = String::from("%");
            let mut symbol = chars.next().unwrap_or('%');
            while matches!(symbol, '-' | '_' | '0' | '^' | '#' | 'E' | 'O' | ':')
                || symbol.is_ascii_digit()
            {
                spec.push(symbol);
                let Some(next) = chars.next() else { break };
                symbol = next;
            }
            spec.push(symbol);
            let value = match (symbol, date, self.labels.as_ref()) {
                ('A', Some(date), Some(labels)) => Some(labels.weekday_name(date.weekday())),
                ('a', Some(date), Some(labels)) => Some(labels.weekday_short(date.weekday())),
                ('B', Some(date), Some(labels)) => Some(labels.month_name(date.month())),
                ('b' | 'h', Some(date), Some(labels)) => Some(labels.month_short(date.month())),
                ('x', Some(date), _) => self
                    .localizer
                    .locale()
                    .format_date(date, DateStyle::Short)
                    .ok(),
                ('p', _, _) => Some(self.localizer.text(if tm.tm_hour < 12 {
                    "enzui-time-am"
                } else {
                    "enzui-time-pm"
                })),
                ('X', _, _) => {
                    time.and_then(|time| self.localizer.locale().format_time(time, true).ok())
                }
                _ => None,
            };
            output.push_str(&value.unwrap_or_else(|| strftime(&spec, at)));
        }
        output
    }

    fn demo_value(&self, name: &str) -> Option<String> {
        let t = self.demo?;
        let wave = |a: f64, b: f64, c: f64| {
            a * (t / 9.0).sin() + b * (t / 2.3 + 1.0).sin() + c * (t / 31.0).sin()
        };
        Some(match name {
            "cpu" => format!("{:.0}", (32.0 + wave(18.0, 9.0, 12.0)).clamp(2.0, 98.0)),
            "mem" => format!("{:.0}", (54.0 + wave(3.0, 1.0, 8.0)).clamp(5.0, 95.0)),
            "mem_used" => format!("{:.1} GiB", 13.4 + wave(0.7, 0.2, 1.8) * 0.25),
            "battery" => "82".into(),
            "load" => format!("{:.2}", (1.4 + wave(0.6, 0.3, 0.5) * 0.1).max(0.05)),
            _ => return None,
        })
    }

    /// Pins `{time}`/`{date}` to a Unix timestamp (snapshots and tests).
    pub(crate) fn freeze_time(&mut self, at: i64) {
        self.frozen_time = Some(at);
    }

    pub(crate) fn now(&self) -> i64 {
        self.now_ms() / 1000
    }

    pub(crate) fn now_ms(&self) -> i64 {
        self.frozen_time.map_or_else(
            || {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as i64)
            },
            |time| time * 1000,
        )
    }

    pub(crate) fn day_fraction(&self) -> f32 {
        let Some(tm) = local_time(self.now()) else {
            return 0.5;
        };
        (tm.tm_hour as f32 * 3600.0 + tm.tm_min as f32 * 60.0 + tm.tm_sec as f32) / 86_400.0
    }

    fn sample(&mut self) {
        if self
            .sampled
            .is_some_and(|at| at.elapsed() < Duration::from_millis(1900))
        {
            return;
        }
        self.sampled = Some(Instant::now());
        if let Some(line) = read("/proc/stat").and_then(|s| s.lines().next().map(str::to_owned)) {
            let values: Vec<u64> = line
                .split_whitespace()
                .skip(1)
                .filter_map(|v| v.parse().ok())
                .collect();
            if values.len() >= 4 {
                let idle = values[3] + values.get(4).copied().unwrap_or(0);
                let total: u64 = values.iter().sum();
                let (last_idle, last_total) = self.cpu.unwrap_or((0, 0));
                let dt = total.saturating_sub(last_total);
                if dt > 0 {
                    self.cpu_percent =
                        Some(100.0 * (1.0 - idle.saturating_sub(last_idle) as f32 / dt as f32));
                }
                self.cpu = Some((idle, total));
            }
        }
        if let Some(info) = read("/proc/meminfo") {
            let field = |name: &str| {
                info.lines()
                    .find(|l| l.starts_with(name))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|v| v.parse::<u64>().ok())
            };
            if let (Some(total), Some(available)) = (field("MemTotal:"), field("MemAvailable:")) {
                self.mem = Some((total, available));
            }
        }
        self.load =
            read("/proc/loadavg").and_then(|s| s.split_whitespace().next().map(str::to_owned));
    }

    pub(crate) fn value(&mut self, name: &str, arg: Option<&str>) -> String {
        if self.public && !matches!(name, "time" | "date") {
            return String::new();
        }
        if let Some(value) = self.demo_value(name) {
            return value;
        }
        match name {
            "time" => self.localized_time(arg.unwrap_or("%H:%M"), self.now()),
            "date" => self.localized_time(arg.unwrap_or("%A, %-d %B"), self.now()),
            "cpu" => {
                self.sample();
                self.cpu_percent
                    .map_or_else(|| "–".into(), |p| format!("{:.0}", p.clamp(0.0, 100.0)))
            }
            "mem" => {
                self.sample();
                self.mem.map_or_else(
                    || "–".into(),
                    |(t, a)| format!("{:.0}", 100.0 * (1.0 - a as f64 / t.max(1) as f64)),
                )
            }
            "mem_used" => {
                self.sample();
                self.mem.map_or_else(
                    || "–".into(),
                    |(t, a)| format!("{:.1} GiB", t.saturating_sub(a) as f64 / 1048576.0),
                )
            }
            "load" => {
                self.sample();
                self.load.clone().unwrap_or_else(|| "–".into())
            }
            "uptime" => read("/proc/uptime")
                .and_then(|s| {
                    s.split_whitespace()
                        .next()
                        .and_then(|v| v.parse::<f64>().ok())
                })
                .map_or_else(
                    || "–".into(),
                    |s| {
                        let minutes = (s / 60.0) as u64;
                        let (d, h, m) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
                        if d > 0 {
                            format!("{d}d {h}h")
                        } else if h > 0 {
                            format!("{h}h {m}m")
                        } else {
                            format!("{m}m")
                        }
                    },
                ),
            "battery" => std::fs::read_dir("/sys/class/power_supply")
                .ok()
                .and_then(|dir| {
                    dir.flatten()
                        .filter(|e| e.file_name().to_string_lossy().starts_with("BAT"))
                        .find_map(|e| std::fs::read_to_string(e.path().join("capacity")).ok())
                })
                .map_or_else(|| "–".into(), |s| s.trim().to_owned()),
            "host" => {
                read("/proc/sys/kernel/hostname").map_or_else(String::new, |s| s.trim().to_owned())
            }
            "user" => std::env::var("USER").unwrap_or_default(),
            "var" => arg
                .and_then(|a| self.custom.get(a).cloned())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_variables_reject_over_budget_and_reclaim_capacity() {
        let mut vars = Vars::new();
        for i in 0..64 {
            vars.set_custom(format!("key{i}"), "a".into()).unwrap();
        }
        assert!(vars.set_custom("overflow".into(), "b".into()).is_err());
        vars.set_custom("key0".into(), "replacement".into())
            .unwrap();
        assert!(vars.set_custom("key0".into(), "x".repeat(32768)).is_err());
        assert_eq!(vars.custom["key0"], "replacement");
        vars.unset_custom("key1");
        vars.set_custom("restored".into(), "capacity".into())
            .unwrap();
    }

    #[test]
    fn wallpaper_dates_follow_the_selected_language_without_os_locale_data() {
        let localizer = Localizer::new(
            CatalogBuilder::new().build(),
            vec!["es".parse().expect("Spanish locale")],
            Locale::default(),
        );
        let mut vars = Vars::new();
        vars.labels = LocaleCalendarLabels::new(&localizer).ok();
        vars.localizer = localizer;
        vars.freeze_time(1_792_370_880);
        assert_eq!(vars.value("date", Some("%B %Y")), "octubre 2026");
        assert_eq!(
            vars.value("time", Some("%% %H:%M")),
            format!("% {}", strftime("%H:%M", vars.now()))
        );
    }
}
