//! The typed option reader every source's `Options::from_table` runs (D63).
//!
//! A source is configured only under `[sources.<id>]` (§9). Arc 11 made a
//! mistyped *name* there a failure; until this existed a mistyped *value* was
//! still read by nobody and reported by nobody — `refresh_ms = "1500"` left
//! the cpu source on its default cadence and said nothing, and `config check`
//! echoed the line back as though it had been accepted.
//!
//! So the reader *is* the declaration (D63 E1): there is no second table of
//! kinds to drift from it. Every method takes the key, what the key accepts
//! and the default the caller will keep, records the key in `asked` (whether
//! or not it is present — the tripwire needs the ask, not the hit), and
//! returns `Option<T>` where `None` means *absent or rejected*: either way
//! the caller keeps its own default and the issue list carries the reason.
//!
//! One mechanical rule decides the kind: a value that is **used** after
//! adjustment is [`gridwatch_store::IssueKind::Adjusted`] (a warning); a value that is
//! **discarded** so the default stands is [`gridwatch_store::IssueKind::Rejected`] (a
//! `config check` failure). The reader is **pure** — no `/dev`, `/sys`,
//! process or bus — because `gridwatch config check` runs it on any machine
//! and the shell runs it on the render thread.

use std::ops::RangeInclusive;

use gridwatch_store::OptionIssue;

/// What `[sources.audio] sink` accepts: a PipeWire object serial or a name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IntOrStr<'a> {
    Int(i64),
    Str(&'a str),
}

/// The article for a TOML type word, so the message reads as English.
fn a(type_str: &str) -> &'static str {
    match type_str {
        "integer" | "array" => "an",
        _ => "a",
    }
}

/// A float in a message: `30`, not `30.0`, so it matches what §9 shows.
pub fn num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// The reader behind one source's `Options::from_table` (D63).
pub struct Reader<'a> {
    id: &'static str,
    t: &'a toml::Table,
    asked: Vec<&'static str>,
    issues: Vec<OptionIssue>,
}

impl<'a> Reader<'a> {
    pub fn new(id: &'static str, t: &'a toml::Table) -> Reader<'a> {
        Reader {
            id,
            t,
            asked: Vec::new(),
            issues: Vec::new(),
        }
    }

    /// The source this reader is reading for — what `log` prefixes with.
    pub fn id(&self) -> &'static str {
        self.id
    }

    /// The keys this reader asked for, in the order it asked. A test per
    /// source pins it against `OPTION_NAMES`: the reader must never report a
    /// key the accepted set does not carry, and never leave one unread
    /// (D63 trap 3).
    pub fn asked(&self) -> &[&'static str] {
        &self.asked
    }

    /// Every issue found, in the order the keys were read.
    pub fn finish(self) -> Vec<OptionIssue> {
        self.issues
    }

    /// A note the caller composed — a cross-key clamp the reader cannot see
    /// on its own (`hi_hz` against `lo_hz`), or a completion (`chips` gaining
    /// `k10temp`). Always `Adjusted`: the value was used.
    pub fn adjusted(&mut self, key: &'static str, text: impl Into<String>) {
        self.issues.push(OptionIssue::adjusted(key, text));
    }

    /// The same for a value the caller **discarded** on a rule the reader
    /// cannot express (`sink`'s serial must not be negative). A `config
    /// check` failure, so the text must say what stands.
    pub fn rejected(&mut self, key: &'static str, text: impl Into<String>) {
        self.issues.push(OptionIssue::rejected(key, text));
    }

    /// Record the ask and hand back the raw value, if the key is present.
    fn get(&mut self, key: &'static str) -> Option<&'a toml::Value> {
        self.asked.push(key);
        self.t.get(key)
    }

    fn reject(&mut self, key: &'static str, text: String) {
        self.issues.push(OptionIssue::rejected(key, text));
    }

    /// `` `k` expects <what>, found a string ("x") — the default D stands ``
    fn wrong_type(&mut self, key: &'static str, what: &str, v: &toml::Value, default: &str) {
        let ty = v.type_str();
        self.reject(
            key,
            format!(
                "`{key}` expects {what}, found {} {ty} ({v}) — the default {default} stands",
                a(ty)
            ),
        );
    }

    fn unit(what: &str, unit: &str) -> String {
        if unit.is_empty() {
            what.to_string()
        } else {
            format!("{what} ({unit})")
        }
    }

    /// An integer with no range of its own.
    pub fn int(&mut self, key: &'static str, unit: &str, default: i64) -> Option<i64> {
        let v = self.get(key)?;
        match v.as_integer() {
            Some(n) => Some(n),
            None => {
                let what = Self::unit("an integer", unit);
                self.wrong_type(key, &what, &v.clone(), &default.to_string());
                None
            }
        }
    }

    /// An integer clamped into `range`. Out of range is **`Adjusted`** — the
    /// clamp exists on purpose and the value *was* used (P14's 500 ms floor,
    /// say) — with `note` appended to the parenthetical each source's warning
    /// carries today.
    pub fn int_in(
        &mut self,
        key: &'static str,
        range: RangeInclusive<i64>,
        unit: &str,
        note: &str,
        default: i64,
    ) -> Option<i64> {
        let n = self.int(key, unit, default)?;
        Some(self.clamp_into(key, n, range, note))
    }

    /// A cadence in milliseconds — the shape nine of the seven sources' keys
    /// share. `0` or less is meaningless and is **`Rejected`**, because a
    /// zero that clamped up to the range's floor would turn a nonsense config
    /// into the *fastest* poll the source allows; positive and out of range
    /// is **`Adjusted`**, clamped, because that is a mistyped magnitude and
    /// the clamp exists on purpose.
    pub fn int_ms(
        &mut self,
        key: &'static str,
        range: RangeInclusive<i64>,
        note: &str,
        default: i64,
    ) -> Option<i64> {
        let n = self.int(key, "milliseconds", default)?;
        if n <= 0 {
            self.reject(
                key,
                format!(
                    "`{key}` expects an integer > 0 (milliseconds), found {n} — \
                     the default {default} stands"
                ),
            );
            return None;
        }
        Some(self.clamp_into(key, n, range, note))
    }

    /// The `Adjusted` half of `int_in` / `int_ms`, shared so the message is
    /// written once.
    fn clamp_into(
        &mut self,
        key: &'static str,
        n: i64,
        range: RangeInclusive<i64>,
        note: &str,
    ) -> i64 {
        let c = n.clamp(*range.start(), *range.end());
        if c != n {
            let tail = if note.is_empty() {
                String::new()
            } else {
                format!(", {note}")
            };
            self.adjusted(
                key,
                format!(
                    "`{key}` = {n} clamped to {c} (accepts {}-{}{tail})",
                    range.start(),
                    range.end()
                ),
            );
        }
        c
    }

    /// An integer at or above `floor`. Below it the value is **discarded**,
    /// so it is `Rejected` and the default stands.
    pub fn int_min(
        &mut self,
        key: &'static str,
        floor: i64,
        unit: &str,
        default: i64,
    ) -> Option<i64> {
        let n = self.int(key, unit, default)?;
        if n < floor {
            self.reject(
                key,
                format!(
                    "`{key}` expects an integer >= {floor}, found {n} — the default {default} stands"
                ),
            );
            return None;
        }
        Some(n)
    }

    /// A number. An integer is accepted where a float is expected (§9 writes
    /// `lo_hz = 30`), and a non-finite one is discarded.
    pub fn float(&mut self, key: &'static str, unit: &str, default: f64) -> Option<f64> {
        let v = self.get(key)?;
        let d = num(default);
        let Some(f) = v.as_float().or_else(|| v.as_integer().map(|i| i as f64)) else {
            let what = Self::unit("a number", unit);
            self.wrong_type(key, &what, &v.clone(), &d);
            return None;
        };
        if !f.is_finite() {
            self.reject(
                key,
                format!("`{key}` expects a finite number, found {f} — the default {d} stands"),
            );
            return None;
        }
        Some(f)
    }

    pub fn bool(&mut self, key: &'static str, default: bool) -> Option<bool> {
        let v = self.get(key)?;
        match v.as_bool() {
            Some(b) => Some(b),
            None => {
                self.wrong_type(key, "a boolean", &v.clone(), &default.to_string());
                None
            }
        }
    }

    pub fn str(&mut self, key: &'static str, default: &str) -> Option<&'a str> {
        let v = self.get(key)?;
        match v.as_str() {
            Some(s) => Some(s),
            None => {
                self.wrong_type(key, "a string", &v.clone(), default);
                None
            }
        }
    }

    /// One of a fixed set of words. Anything else fell through to the default
    /// before this existed (`source = "i2x"` read as `auto`), so it is
    /// `Rejected` naming the choices.
    pub fn one_of(
        &mut self,
        key: &'static str,
        choices: &[&str],
        default: &str,
    ) -> Option<&'a str> {
        let s = self.str(key, default)?;
        if choices.contains(&s) {
            return Some(s);
        }
        self.reject(
            key,
            format!(
                "`{key}` expects one of {}, found \"{s}\" — {default} stands",
                choices.join(", ")
            ),
        );
        None
    }

    /// A list of strings. A non-string element discards the **whole** list —
    /// a partial list is a silent discard of the rest, which is the defect
    /// D63 ends. `allow_empty` is false where an empty list means "nothing at
    /// all" and the source cannot run on it (`chips = []`).
    pub fn str_list(
        &mut self,
        key: &'static str,
        allow_empty: bool,
        default: &[&str],
    ) -> Option<Vec<String>> {
        let v = self.get(key)?;
        let d = format!("{default:?}");
        let Some(list) = v.as_array() else {
            self.wrong_type(key, "a list of strings", &v.clone(), &d);
            return None;
        };
        let mut out = Vec::with_capacity(list.len());
        for (i, item) in list.iter().enumerate() {
            match item.as_str() {
                Some(s) => out.push(s.to_string()),
                None => {
                    self.reject(
                        key,
                        format!(
                            "`{key}` expects a list of strings, found {item} at [{i}] — \
                             the default {d} stands"
                        ),
                    );
                    return None;
                }
            }
        }
        if out.is_empty() && !allow_empty {
            self.reject(
                key,
                format!("`{key}` is an empty list — the default {d} stands"),
            );
            return None;
        }
        Some(out)
    }

    /// `[sources.audio] sink`: a PipeWire object serial, or a name. Two
    /// shapes on one key, which is why no table of kinds could declare it.
    pub fn int_or_str(&mut self, key: &'static str, default: &str) -> Option<IntOrStr<'a>> {
        let v = self.get(key)?;
        match v {
            toml::Value::Integer(n) => Some(IntOrStr::Int(*n)),
            toml::Value::String(s) => Some(IntOrStr::Str(s)),
            other => {
                self.wrong_type(key, "an integer or a string", &other.clone(), default);
                None
            }
        }
    }

    /// `[sources.sensors] rapl`: a boolean, or one of the spellings people
    /// write for one. `rapl = "maybe"` read as `true` before this existed.
    /// `words` is `(spelling, value)` in the order the message lists them.
    pub fn bool_or_words(
        &mut self,
        key: &'static str,
        words: &[(&str, bool)],
        default: bool,
    ) -> Option<bool> {
        let v = self.get(key)?;
        match v {
            toml::Value::Boolean(b) => Some(*b),
            toml::Value::String(s) => {
                if let Some((_, b)) = words.iter().find(|(w, _)| *w == s.as_str()) {
                    return Some(*b);
                }
                let names: Vec<&str> = words.iter().map(|(w, _)| *w).collect();
                self.reject(
                    key,
                    format!(
                        "`{key}` expects a boolean or one of {}, found \"{s}\" — \
                         the default {default} stands",
                        names.join(", ")
                    ),
                );
                None
            }
            other => {
                self.wrong_type(key, "a boolean", &other.clone(), &default.to_string());
                None
            }
        }
    }
}

/// What `start` does with the issues its reader found: one `warn!` each, once,
/// at the moment the source starts on its defaults. `check` never logs — it
/// runs on the render thread and inside `config check`, where the caller
/// prints. This is where the scattered `tracing::warn!`s of arcs 3–7 went.
pub fn log(id: &str, issues: &[OptionIssue]) {
    for i in issues {
        tracing::warn!("[sources.{id}] {}", i.text);
    }
}

/// The one issue a table must produce, as `(kind, text)`, or a panic naming
/// what it produced instead — the shape every source's option test asserts.
#[cfg(test)]
pub(crate) fn only_issue(issues: Vec<OptionIssue>) -> (gridwatch_store::IssueKind, String) {
    assert_eq!(issues.len(), 1, "{issues:?}");
    (issues[0].kind, issues[0].text.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridwatch_store::IssueKind;

    fn table(text: &str) -> toml::Table {
        text.parse().unwrap()
    }

    fn one(text: &str, f: impl FnOnce(&mut Reader<'_>)) -> OptionIssue {
        let t = table(text);
        let mut r = Reader::new("test", &t);
        f(&mut r);
        let mut issues = r.finish();
        assert_eq!(issues.len(), 1, "{issues:?}");
        issues.remove(0)
    }

    fn none(text: &str, f: impl FnOnce(&mut Reader<'_>)) {
        let t = table(text);
        let mut r = Reader::new("test", &t);
        f(&mut r);
        let issues = r.finish();
        assert!(issues.is_empty(), "{issues:?}");
    }

    /// The shape D63 pins verbatim, and the one the acceptance case checks.
    #[test]
    fn a_wrong_type_names_what_was_found_and_what_stands() {
        let i = one("refresh_ms = \"1500\"", |r| {
            r.int("refresh_ms", "milliseconds", 1500);
        });
        assert_eq!(i.kind, IssueKind::Rejected);
        assert_eq!(
            i.text,
            "`refresh_ms` expects an integer (milliseconds), found a string (\"1500\") \
             — the default 1500 stands"
        );
    }

    #[test]
    fn an_absent_key_is_asked_for_and_says_nothing() {
        let t = table("");
        let mut r = Reader::new("test", &t);
        assert_eq!(r.int("refresh_ms", "milliseconds", 1500), None);
        assert_eq!(r.asked(), ["refresh_ms"]);
        assert!(r.finish().is_empty());
    }

    #[test]
    fn a_clamp_is_adjusted_and_carries_the_range_and_the_note() {
        let i = one("interval_ms = 100", |r| {
            r.int_ms("interval_ms", 500..=5000, "P14", 500);
        });
        assert_eq!(i.kind, IssueKind::Adjusted);
        assert_eq!(
            i.text,
            "`interval_ms` = 100 clamped to 500 (accepts 500-5000, P14)"
        );
        none("interval_ms = 750", |r| {
            assert_eq!(r.int_ms("interval_ms", 500..=5000, "P14", 500), Some(750));
        });
        // Without a note the parenthetical is the range alone.
        let i = one("fps = 90", |r| {
            r.int_in("fps", 5..=60, "frames a second", "", 30);
        });
        assert_eq!(i.text, "`fps` = 90 clamped to 60 (accepts 5-60)");
    }

    /// A zero cadence is not a small one: clamping it up would turn nonsense
    /// into the fastest poll the source allows, so it is discarded (D63).
    #[test]
    fn a_zero_cadence_is_rejected_rather_than_clamped_up() {
        let i = one("refresh_ms = 0", |r| {
            r.int_ms("refresh_ms", 200..=60_000, "", 1500);
        });
        assert_eq!(i.kind, IssueKind::Rejected);
        assert_eq!(
            i.text,
            "`refresh_ms` expects an integer > 0 (milliseconds), found 0 — \
             the default 1500 stands"
        );
    }

    #[test]
    fn below_a_floor_the_value_is_discarded() {
        let i = one("device = -1", |r| {
            r.int_min("device", 0, "", 0);
        });
        assert_eq!(i.kind, IssueKind::Rejected);
        assert_eq!(
            i.text,
            "`device` expects an integer >= 0, found -1 — the default 0 stands"
        );
    }

    #[test]
    fn a_float_accepts_an_integer_and_refuses_a_nan() {
        none("lo_hz = 40", |r| {
            assert_eq!(r.float("lo_hz", "hertz", 30.0), Some(40.0));
        });
        let i = one("lo_hz = nan", |r| {
            r.float("lo_hz", "hertz", 30.0);
        });
        assert_eq!(i.kind, IssueKind::Rejected);
        assert_eq!(
            i.text,
            "`lo_hz` expects a finite number, found NaN — the default 30 stands"
        );
        let i = one("lo_hz = true", |r| {
            r.float("lo_hz", "hertz", 30.0);
        });
        assert_eq!(
            i.text,
            "`lo_hz` expects a number (hertz), found a boolean (true) — the default 30 stands"
        );
    }

    #[test]
    fn a_boolean_says_what_it_wanted() {
        none("public_ip = true", |r| {
            assert_eq!(r.bool("public_ip", false), Some(true));
        });
        let i = one("public_ip = \"yes\"", |r| {
            r.bool("public_ip", false);
        });
        assert_eq!(i.kind, IssueKind::Rejected);
        assert_eq!(
            i.text,
            "`public_ip` expects a boolean, found a string (\"yes\") — the default false stands"
        );
    }

    #[test]
    fn a_string_says_what_it_wanted() {
        let i = one("exporter = 9942", |r| {
            r.str("exporter", "127.0.0.1:9942");
        });
        assert_eq!(i.kind, IssueKind::Rejected);
        assert_eq!(
            i.text,
            "`exporter` expects a string, found an integer (9942) — \
             the default 127.0.0.1:9942 stands"
        );
    }

    #[test]
    fn a_choice_names_the_choices() {
        let i = one("source = \"i2x\"", |r| {
            r.one_of("source", &["auto", "i2c", "exporter"], "auto");
        });
        assert_eq!(i.kind, IssueKind::Rejected);
        assert_eq!(
            i.text,
            "`source` expects one of auto, i2c, exporter, found \"i2x\" — auto stands"
        );
        none("source = \"i2c\"", |r| {
            assert_eq!(
                r.one_of("source", &["auto", "i2c", "exporter"], "auto"),
                Some("i2c")
            );
        });
    }

    #[test]
    fn a_list_is_all_strings_or_none_of_it() {
        let i = one("probes = [\"gateway\", 5]", |r| {
            r.str_list("probes", true, &["gateway", "1.1.1.1"]);
        });
        assert_eq!(i.kind, IssueKind::Rejected);
        assert_eq!(
            i.text,
            "`probes` expects a list of strings, found 5 at [1] — \
             the default [\"gateway\", \"1.1.1.1\"] stands"
        );
        // An empty list is a value where one is allowed and a rejection where
        // it means "no chips at all" and the source cannot run on it.
        none("probes = []", |r| {
            assert_eq!(
                r.str_list("probes", true, &["gateway", "1.1.1.1"]),
                Some(Vec::new())
            );
        });
        let i = one("chips = []", |r| {
            r.str_list("chips", false, &["*"]);
        });
        assert_eq!(i.kind, IssueKind::Rejected);
        assert_eq!(
            i.text,
            "`chips` is an empty list — the default [\"*\"] stands"
        );
        let i = one("chips = 5", |r| {
            r.str_list("chips", false, &["*"]);
        });
        assert_eq!(
            i.text,
            "`chips` expects a list of strings, found an integer (5) — \
             the default [\"*\"] stands"
        );
    }

    #[test]
    fn int_or_str_takes_both_and_refuses_the_third() {
        none("sink = 42", |r| {
            assert_eq!(r.int_or_str("sink", "auto"), Some(IntOrStr::Int(42)));
        });
        none("sink = \"alsa_output.x\"", |r| {
            assert_eq!(
                r.int_or_str("sink", "auto"),
                Some(IntOrStr::Str("alsa_output.x"))
            );
        });
        let i = one("sink = true", |r| {
            r.int_or_str("sink", "auto");
        });
        assert_eq!(i.kind, IssueKind::Rejected);
        assert_eq!(
            i.text,
            "`sink` expects an integer or a string, found a boolean (true) — \
             the default auto stands"
        );
    }

    #[test]
    fn bool_or_words_names_the_spellings() {
        let words = &[
            ("on", true),
            ("off", false),
            ("yes", true),
            ("no", false),
            ("true", true),
            ("false", false),
        ];
        none("rapl = false", |r| {
            assert_eq!(r.bool_or_words("rapl", words, true), Some(false));
        });
        none("rapl = \"off\"", |r| {
            assert_eq!(r.bool_or_words("rapl", words, true), Some(false));
        });
        let i = one("rapl = \"maybe\"", |r| {
            r.bool_or_words("rapl", words, true);
        });
        assert_eq!(i.kind, IssueKind::Rejected);
        assert_eq!(
            i.text,
            "`rapl` expects a boolean or one of on, off, yes, no, true, false, \
             found \"maybe\" — the default true stands"
        );
        let i = one("rapl = 1", |r| {
            r.bool_or_words("rapl", words, true);
        });
        assert_eq!(
            i.text,
            "`rapl` expects a boolean, found an integer (1) — the default true stands"
        );
    }

    #[test]
    fn a_caller_composed_note_is_always_a_warning() {
        let i = one("", |r| {
            r.adjusted("chips", "chips excludes k10temp; adding it");
        });
        assert_eq!(i.kind, IssueKind::Adjusted);
        assert_eq!(i.text, "chips excludes k10temp; adding it");
    }
}
