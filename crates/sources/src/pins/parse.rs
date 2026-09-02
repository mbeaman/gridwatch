//! The Prometheus text parser for astral-watch's exporter (digest §2b): the
//! format subset is `name{k="v",…} value` with `+Inf`/`-Inf`/`NaN`, so fifty
//! lines beat a `prometheus-parse` dependency (chrono, regex, itertools).

use astral_watch::decode::{PIN_COUNT, Pin, Reading};

/// `(name, labels, value)` of one sample line.
pub type Line<'a> = (&'a str, Vec<(&'a str, &'a str)>, f64);

/// One sample line: `(name, labels, value)`.
pub fn parse_line(l: &str) -> Option<Line<'_>> {
    let l = l.trim();
    if l.starts_with('#') || l.is_empty() {
        return None;
    }
    let (head, val) = l.rsplit_once(' ')?;
    let v = match val {
        "+Inf" => f64::INFINITY,
        "-Inf" => f64::NEG_INFINITY,
        "NaN" => f64::NAN,
        s => s.parse().ok()?,
    };
    let (name, labels) = match head.split_once('{') {
        Some((n, rest)) => (
            n,
            rest.trim_end_matches('}')
                .split(',')
                .filter(|s| !s.is_empty())
                .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k, v.trim_matches('"'))))
                .collect(),
        ),
        None => (head, Vec::new()),
    };
    Some((name, labels, v))
}

/// What one `/metrics` body says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scrape {
    pub reading: Option<Reading>,
    pub up: bool,
    /// `astral_watch_last_reading_age_seconds`, absent until a reading exists.
    pub age_s: Option<f64>,
    /// Conditions whose `alert_active{condition}` is 1.
    pub active: Vec<String>,
    pub version: Option<String>,
}

pub fn parse_metrics(text: &str) -> Scrape {
    let mut volts = [None; PIN_COUNT];
    let mut amps = [None; PIN_COUNT];
    let mut out = Scrape::default();
    for line in text.lines() {
        let Some((name, labels, v)) = parse_line(line) else {
            continue;
        };
        let label = |k: &str| labels.iter().find(|(lk, _)| *lk == k).map(|(_, v)| *v);
        let pin = || {
            label("pin")
                .and_then(|p| p.parse::<usize>().ok())
                .filter(|p| (1..=PIN_COUNT).contains(p))
                .map(|p| p - 1)
        };
        match name {
            "astral_watch_pin_volts" => {
                if let Some(i) = pin() {
                    volts[i] = Some(v);
                }
            }
            "astral_watch_pin_amps" => {
                if let Some(i) = pin() {
                    amps[i] = Some(v);
                }
            }
            "astral_watch_up" => out.up = v >= 1.0,
            "astral_watch_last_reading_age_seconds" => out.age_s = Some(v),
            "astral_watch_alert_active" => {
                if v >= 1.0
                    && let Some(c) = label("condition")
                {
                    out.active.push(c.to_string());
                }
            }
            "astral_watch_build_info" => {
                out.version = label("version").map(str::to_string);
            }
            _ => {}
        }
    }
    if volts.iter().all(Option::is_some) && amps.iter().all(Option::is_some) {
        let mut pins = [Pin {
            volts: 0.0,
            amps: 0.0,
        }; PIN_COUNT];
        for i in 0..PIN_COUNT {
            pins[i] = Pin {
                volts: volts[i].unwrap_or(0.0),
                amps: amps[i].unwrap_or(0.0),
            };
        }
        out.reading = Some(Reading { pins });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const SAMPLE: &str = r#"# TYPE astral_watch_pin_volts gauge
astral_watch_pin_volts{pin="1"} 12.06
astral_watch_pin_volts{pin="2"} 12.07
astral_watch_pin_volts{pin="3"} 12.06
astral_watch_pin_volts{pin="4"} 12.08
astral_watch_pin_volts{pin="5"} 12.06
astral_watch_pin_volts{pin="6"} 12.07
# TYPE astral_watch_pin_amps gauge
astral_watch_pin_amps{pin="1"} 1.72
astral_watch_pin_amps{pin="2"} 1.65
astral_watch_pin_amps{pin="3"} 1.55
astral_watch_pin_amps{pin="4"} 1.50
astral_watch_pin_amps{pin="5"} 1.42
astral_watch_pin_amps{pin="6"} 1.36
astral_watch_total_amps 9.2
astral_watch_total_watts 111
astral_watch_balance_ratio +Inf
astral_watch_last_reading_age_seconds 0.412
astral_watch_up 1
astral_watch_alert_active{condition="overload"} 0
astral_watch_alert_active{condition="imbalance_advisory"} 1
astral_watch_build_info{version="0.7.0"} 1
"#;

    #[test]
    fn a_metrics_body_becomes_a_reading_and_flags() {
        let s = parse_metrics(SAMPLE);
        let r = s.reading.expect("six pins present");
        assert!((r.total_amps() - 9.2).abs() < 1e-9);
        assert!(r.plausible());
        assert!(s.up);
        assert_eq!(s.age_s, Some(0.412));
        assert_eq!(s.active, vec!["imbalance_advisory"]);
        assert_eq!(s.version.as_deref(), Some("0.7.0"));
        // `+Inf` parses instead of poisoning the scrape.
        let (_, _, v) = parse_line("astral_watch_balance_ratio +Inf").unwrap();
        assert!(v.is_infinite());
    }

    #[test]
    fn a_partial_body_has_no_reading() {
        let s = parse_metrics("astral_watch_pin_amps{pin=\"1\"} 1.7\nastral_watch_up 1\n");
        assert!(s.reading.is_none() && s.up);
        assert!(parse_line("# comment").is_none());
        assert!(parse_line("garbage").is_none());
    }
}
