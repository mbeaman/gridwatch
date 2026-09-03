//! The wire messages (§4.7, `schema/exec.schema.json`). Serde types that
//! match the schema exactly: the schema is the contract a plugin author
//! reads, and these are what the host will accept.
//!
//! Everything a plugin sends is *untrusted input*. Two rules follow:
//! nothing here allocates from a number the plugin chose, and every string
//! that reaches the screen is bounded (the caps below mirror the schema's
//! `maxLength`, so a plugin that lies about its length is truncated rather
//! than believed).

use serde::{Deserialize, Serialize};

/// The contract number the host speaks.
pub const CONTRACT: u32 = 1;

/// Longest line the host will read. A plugin that writes more than this is
/// not talking to us any more (D39: a bounded reader, not a growing one).
pub const MAX_LINE: usize = 1024 * 1024;

/// What the host says first.
#[derive(Clone, Debug, Serialize)]
pub struct Hello {
    pub kind: &'static str,
    pub contract: u32,
    pub capabilities: Vec<String>,
    pub keys: Vec<String>,
}

impl Hello {
    pub fn new(capabilities: Vec<String>, keys: Vec<String>) -> Hello {
        Hello {
            kind: "hello",
            contract: CONTRACT,
            capabilities,
            keys,
        }
    }
}

/// What the host asks for, between hellos.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Ask {
    Render {
        instance: String,
        tier: usize,
        inner: Size,
        now: i64,
        focused: bool,
        captured: bool,
    },
    Key {
        instance: String,
        key: String,
        mods: Vec<&'static str>,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Size {
    pub w: u16,
    pub h: u16,
}

/// One tier as a plugin declares it.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct TierDecl {
    pub name: String,
    pub min: Size,
    #[serde(default)]
    pub adds: Vec<String>,
    #[serde(default)]
    pub zoom_only: bool,
}

/// One metric a plugin says it publishes.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Produces {
    pub key: String,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub help: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct KeyDecl {
    pub key: String,
    pub does: String,
}

/// `schema/manifest.schema.json`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Manifest {
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub summary: Option<String>,
    pub contract: u32,
    #[serde(default)]
    pub footprints: Vec<Size>,
    #[serde(default)]
    pub default_footprint: Option<Size>,
    pub tiers: Vec<TierDecl>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub produces: Vec<Produces>,
    #[serde(default)]
    pub keys: Vec<KeyDecl>,
    #[serde(default)]
    pub options: Vec<String>,
}

/// A label as the wire carries it: a name, an index, or nothing.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum WireLabel {
    Index(u16),
    Name(String),
}

/// What a plugin sends.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Says {
    Manifest {
        /// Boxed: a manifest is much larger than any other message, and
        /// every line the host reads would otherwise carry its size.
        manifest: Box<Manifest>,
    },
    Sample {
        key: String,
        #[serde(default)]
        label: Option<WireLabel>,
        #[serde(default)]
        at: Option<i64>,
        value: f64,
    },
    View {
        instance: String,
        tree: serde_json::Value,
    },
    Command {
        command: Cmd,
    },
    Status {
        state: State,
        #[serde(default)]
        reason: Option<String>,
        #[serde(default)]
        hint: Option<String>,
    },
    Log {
        #[serde(default)]
        level: Option<String>,
        text: String,
    },
}

/// The small set of side effects a plugin may ask for. Deliberately not
/// open-ended: a plugin may put a word on the screen and may not touch the
/// machine (D39).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Cmd {
    Toast { severity: Severity, text: String },
    Page(usize),
    Zoom(bool),
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warn,
    Crit,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Starting,
    Ok,
    Degraded,
    Unavailable,
    Stopped,
}

/// Why a line was refused. Three of these stop a plugin (D58 seam 7), so
/// each one says enough to fix the plugin.
#[derive(Clone, Debug, PartialEq)]
pub enum Refused {
    /// Longer than `MAX_LINE`.
    TooLong(usize),
    /// Not JSON, or not a message this contract knows.
    Malformed(String),
    /// A manifest whose shape the host cannot place.
    BadManifest(String),
    /// A key name a plugin may not publish.
    BadKey(String),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::TooLong(n) => write!(f, "a {n}-byte line (the cap is {MAX_LINE})"),
            Refused::Malformed(e) => write!(f, "malformed: {e}"),
            Refused::BadManifest(e) => write!(f, "manifest: {e}"),
            Refused::BadKey(k) => write!(f, "key `{k}`: {}", KEY_RULE),
        }
    }
}

pub const KEY_RULE: &str = "must be `<source>.<metric>`, lower case, with `_` for spaces";

/// Is this a metric name a plugin may publish? The host prefixes it with
/// the plugin's id, so the only rule is shape.
pub fn key_is_sane(key: &str) -> bool {
    let Some((source, metric)) = key.split_once('.') else {
        return false;
    };
    id_is_sane(source) && id_is_sane(metric) && !key.contains("..")
}

/// One half of a metric name, and the shape a plugin's `id` must have: it
/// becomes the source half of every key the plugin publishes, so the two
/// rules are the same rule (§4.7).
pub fn id_is_sane(s: &str) -> bool {
    !s.is_empty()
        && s.starts_with(|c: char| c.is_ascii_lowercase())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Parse one line. This is the only door untrusted text comes through.
pub fn parse(line: &str) -> Result<Says, Refused> {
    if line.len() > MAX_LINE {
        return Err(Refused::TooLong(line.len()));
    }
    let says: Says = serde_json::from_str(line).map_err(|e| Refused::Malformed(e.to_string()))?;
    if let Says::Sample { key, .. } = &says
        && !key_is_sane(key)
    {
        return Err(Refused::BadKey(key.clone()));
    }
    Ok(says)
}

/// The rules a manifest must keep to be placeable, beyond its schema:
/// the tiers are poorest-first and the first one fits the smallest tile
/// this grid can make (§4.6). A plugin that gets this wrong would
/// otherwise draw into a rect it never agreed to.
pub const MIN_TILE: Size = Size { w: 8, h: 3 };

pub fn check_manifest(m: &Manifest) -> Result<(), Refused> {
    if m.contract != CONTRACT {
        return Err(Refused::BadManifest(format!(
            "contract {} (this host speaks {CONTRACT})",
            m.contract
        )));
    }
    if m.tiers.is_empty() {
        return Err(Refused::BadManifest("no tiers".into()));
    }
    let first = &m.tiers[0];
    if first.min.w > MIN_TILE.w || first.min.h > MIN_TILE.h {
        return Err(Refused::BadManifest(format!(
            "the first tier `{}` needs {}x{}, which does not fit the smallest tile ({}x{})",
            first.name, first.min.w, first.min.h, MIN_TILE.w, MIN_TILE.h
        )));
    }
    if first.zoom_only {
        return Err(Refused::BadManifest(
            "the first tier cannot be zoom-only: a tile must draw something on the grid".into(),
        ));
    }
    for pair in m.tiers.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if b.min.w < a.min.w || b.min.h < a.min.h {
            return Err(Refused::BadManifest(format!(
                "tier `{}` is smaller than `{}`; tiers are cumulative, poorest first",
                b.name, a.name
            )));
        }
    }
    for p in &m.produces {
        if !key_is_sane(&p.key) {
            return Err(Refused::BadKey(p.key.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/plugins")
            .join(name);
        std::fs::read_to_string(path)
            .expect("the fixture reads")
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(String::from)
            .collect()
    }

    /// Every line of the good fixture parses, and the manifest in it is
    /// placeable. The same file is validated against the JSON Schema by
    /// `scripts/check-schemas.py`, so the two agree by construction.
    #[test]
    fn the_handshake_fixture_parses_and_its_manifest_is_placeable() {
        let mut seen_manifest = false;
        for line in fixture("handshake.jsonl") {
            // The host's own lines are not `Says`; skip them here (the
            // schema covers both directions).
            if line.contains("\"kind\":\"hello\"")
                || line.contains("\"kind\":\"render\"")
                || line.contains("\"kind\":\"key\"")
            {
                continue;
            }
            match parse(&line) {
                Ok(Says::Manifest { manifest }) => {
                    seen_manifest = true;
                    assert_eq!(manifest.kind, "weather");
                    assert_eq!(manifest.tiers.len(), 2);
                    check_manifest(&manifest).expect("placeable");
                }
                Ok(_) => {}
                Err(e) => panic!("{line}\n  refused: {e}"),
            }
        }
        assert!(seen_manifest);
    }

    /// Every line of the bad fixture is refused, with a reason.
    #[test]
    fn the_bad_fixture_is_refused_line_by_line() {
        for line in fixture("bad.jsonl") {
            let r = parse(&line);
            assert!(r.is_err(), "accepted: {line}");
            let text = r.unwrap_err().to_string();
            assert!(!text.is_empty());
        }
    }

    #[test]
    fn a_key_a_plugin_may_publish() {
        assert!(key_is_sane("weather.temp_c"));
        assert!(key_is_sane("a.b"));
        assert!(!key_is_sane("Weather.temp"), "upper case");
        assert!(!key_is_sane("weather"), "no dot");
        assert!(!key_is_sane("weather."), "no metric");
        assert!(!key_is_sane(".temp"), "no source");
        assert!(!key_is_sane("weather..temp"));
        assert!(!key_is_sane("weather.temp c"), "a space");
        assert!(!key_is_sane("9weather.temp"), "starts with a digit");
        assert!(!key_is_sane("weather.temp-c"), "a dash");
    }

    #[test]
    fn a_line_longer_than_the_cap_is_refused_without_parsing_it() {
        let huge = format!(
            "{{\"kind\":\"log\",\"text\":\"{}\"}}",
            "x".repeat(MAX_LINE + 10)
        );
        assert!(matches!(parse(&huge), Err(Refused::TooLong(_))));
    }

    #[test]
    fn a_manifest_that_could_not_be_placed_is_refused() {
        let base = |tiers: Vec<TierDecl>| Manifest {
            kind: "x".into(),
            name: "x".into(),
            summary: None,
            contract: CONTRACT,
            footprints: Vec::new(),
            default_footprint: None,
            tiers,
            requires: Vec::new(),
            optional: Vec::new(),
            sources: Vec::new(),
            produces: Vec::new(),
            keys: Vec::new(),
            options: Vec::new(),
        };
        let tier = |name: &str, w, h, zoom| TierDecl {
            name: name.into(),
            min: Size { w, h },
            adds: Vec::new(),
            zoom_only: zoom,
        };
        // The smallest tile is 8x3, so a first tier that needs more cannot
        // be placed at all.
        let m = base(vec![tier("big", 40, 10, false)]);
        assert!(
            check_manifest(&m)
                .unwrap_err()
                .to_string()
                .contains("does not fit"),
        );
        // A first tier that only exists zoomed leaves the grid blank.
        let m = base(vec![tier("only", 8, 3, true)]);
        assert!(
            check_manifest(&m)
                .unwrap_err()
                .to_string()
                .contains("zoom-only")
        );
        // Tiers must grow.
        let m = base(vec![tier("a", 8, 3, false), tier("b", 8, 2, false)]);
        assert!(
            check_manifest(&m)
                .unwrap_err()
                .to_string()
                .contains("cumulative")
        );
        // A contract from the future.
        let mut m = base(vec![tier("a", 8, 3, false)]);
        m.contract = 99;
        assert!(check_manifest(&m).unwrap_err().to_string().contains("99"));
        // And a good one.
        let m = base(vec![tier("a", 8, 3, false), tier("b", 20, 6, false)]);
        assert_eq!(check_manifest(&m), Ok(()));
        // A produced key with a bad name is refused with the rule.
        let mut m = base(vec![tier("a", 8, 3, false)]);
        m.produces.push(Produces {
            key: "Weather.Temp".into(),
            unit: None,
            help: None,
        });
        assert!(
            check_manifest(&m)
                .unwrap_err()
                .to_string()
                .contains(KEY_RULE)
        );
    }
}
