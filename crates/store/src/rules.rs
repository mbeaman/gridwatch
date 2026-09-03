//! The rules engine (§9, brief arc 7 seam 4): `[[rules]]` turns any metric
//! into an alert, with two holds for hysteresis. Rules are **name-indexed**
//! and evaluated only over the keys a batch actually touched — never a scan
//! of the whole store — so a config with ten rules costs microseconds per
//! batch. A rule that matches several labels raises one alert per label
//! (`id = "<name>/<label>"`), which is what makes `nvme*` one rule instead
//! of three.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use crate::alert::{AlertEvent, AlertId, Severity, Transition};
use crate::key::{Label, MetricId};
use crate::source::SourceId;
use crate::ts::Ts;

/// The comparison a rule makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
    Ne,
    /// Raises when the key stops arriving (or never does).
    Absent,
}

impl Op {
    pub fn parse(s: &str) -> Option<Op> {
        Some(match s.trim() {
            ">" | "gt" => Op::Gt,
            ">=" | "ge" => Op::Ge,
            "<" | "lt" => Op::Lt,
            "<=" | "le" => Op::Le,
            "==" | "=" | "eq" => Op::Eq,
            "!=" | "ne" => Op::Ne,
            "absent" => Op::Absent,
            _ => return None,
        })
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Eq => "==",
            Op::Ne => "!=",
            Op::Absent => "absent",
        }
    }

    fn holds(self, lhs: f64, rhs: f64) -> bool {
        match self {
            Op::Gt => lhs > rhs,
            Op::Ge => lhs >= rhs,
            Op::Lt => lhs < rhs,
            Op::Le => lhs <= rhs,
            // A **relative** tolerance: `f64::EPSILON` is smaller than
            // the gap between representable numbers above 1.0, so an
            // absolute comparison made `== 84` unable to match a computed
            // 84 and `!=` always true. D57 amendment 18 said this had been
            // fixed; a partially-applied edit meant it had not, and the
            // arc 7b review caught the drift between the doc and the code.
            Op::Eq => near(lhs, rhs),
            Op::Ne => !near(lhs, rhs),
            Op::Absent => false,
        }
    }
}

/// Equal to within a relative tolerance (one part in a billion, with an
/// absolute floor so values near zero still compare sanely). NaN is never
/// equal to anything, including itself, which is what a rule wants.
fn near(a: f64, b: f64) -> bool {
    let diff = (a - b).abs();
    diff <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

/// A rule's right-hand side: a number, or another metric (`sensor.crit_c`
/// with the same label — "hot when it passes the chip's own limit").
#[derive(Clone, Debug, PartialEq)]
pub enum Rhs {
    Value(f64),
    /// A key name; the label comes from the left-hand side's match.
    Key(String),
}

/// One `[[rules]]` entry, parsed and validated.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub name: String,
    /// The metric name (`sensor.temp_c`).
    pub key: String,
    /// A glob over the label (`nvme*:Composite`, `*`, or empty for keys
    /// that carry no label).
    pub label: String,
    pub op: Op,
    pub rhs: Rhs,
    pub for_s: Duration,
    pub clear_s: Duration,
    pub severity: Severity,
    /// `{key} {label} {value} {threshold}` are substituted.
    pub message: String,
}

impl Rule {
    /// Does this rule watch that metric id?
    pub fn matches(&self, id: &MetricId) -> bool {
        if id.name != self.key {
            return false;
        }
        let label = label_text(&id.label);
        glob(&self.label, &label)
    }

    /// The alert id for one matching label.
    pub fn alert_id(&self, label: &str) -> AlertId {
        if label.is_empty() {
            AlertId::new(&self.name)
        } else {
            AlertId::new(&format!("{}/{label}", self.name))
        }
    }

    fn render(&self, label: &str, value: f64, threshold: f64) -> String {
        self.message
            .replace("{key}", &self.key)
            .replace("{label}", label)
            .replace("{value}", &format!("{value:.1}"))
            .replace("{threshold}", &format!("{threshold:.1}"))
    }
}

/// A label as text: `Label::Name` verbatim, `Label::Index` as its number,
/// `Label::None` as the empty string.
pub fn label_text(l: &Label) -> String {
    match l {
        Label::None => String::new(),
        Label::Index(i) => i.to_string(),
        Label::Name(n) => n.to_string(),
    }
}

/// A glob with `*` **anywhere**: a label pattern like `nvme*:Composite`
/// has to match `nvme#2:Composite`, so a leading/trailing-only rule is not
/// enough (review of my own first cut, caught by the fixture labels).
/// Every segment between the stars must appear in order.
pub fn glob(pattern: &str, name: &str) -> bool {
    if pattern.is_empty() {
        return name.is_empty();
    }
    if !pattern.contains('*') {
        return pattern == name;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let last = parts.len() - 1;
    let mut rest = name;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            // Before the first star: an anchored prefix.
            let Some(r) = rest.strip_prefix(part) else {
                return false;
            };
            rest = r;
        } else if i == last {
            // After the last star: an anchored suffix, and it may not
            // overlap what has already been consumed.
            return rest.len() >= part.len() && rest.ends_with(part);
        } else {
            let Some(at) = rest.find(part) else {
                return false;
            };
            rest = &rest[at + part.len()..];
        }
    }
    true
}

/// What a rule is doing about one label right now.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct State {
    /// The instant the condition first held (the `for_s` hold).
    since: Option<Ts>,
    /// The instant it first stopped holding (the `clear_s` hold).
    clear_since: Option<Ts>,
    raised: bool,
}

/// What labels a rule's key currently has, and when each last arrived —
/// the `absent` rules' view of the store.
pub type KnownLabels<'a> = &'a dyn Fn(&str, &str) -> Vec<(String, Ts)>;

/// The engine: the parsed rules and one state per (rule, label).
#[derive(Clone, Debug, Default)]
pub struct Rules {
    rules: Vec<Rule>,
    /// Keyed by **rule name and label**, not by index: a reload rebuilds
    /// the `Vec` and the states have to survive it (arc 7b review).
    states: BTreeMap<(String, String), State>,
    /// When this set of rules started watching — the clock a key that has
    /// never arrived is counted absent from.
    started: Option<Ts>,
}

/// What `config check` prints and what a parse error says.
#[derive(Clone, Debug, PartialEq)]
pub struct RuleError {
    pub name: String,
    pub problem: String,
}

impl std::fmt::Display for RuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[[rules]] {}: {}", self.name, self.problem)
    }
}

impl Rules {
    pub fn new(rules: Vec<Rule>) -> Rules {
        Rules {
            rules,
            states: BTreeMap::new(),
            started: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Whether any rule watches for a key that stopped arriving. The frame
    /// loop asks before doing the work `tick` needs (review: `tick_rules`
    /// ran a store walk every frame even when no rule wanted one).
    pub fn has_absent(&self) -> bool {
        self.rules.iter().any(|r| r.op == Op::Absent)
    }

    pub fn list(&self) -> &[Rule] {
        &self.rules
    }

    /// Take over from the rules this set replaces (a config reload).
    ///
    /// A rule that is still here keeps its state, so an alert that is
    /// already raised is neither re-raised nor un-acknowledged; a rule
    /// that is gone has its raised alerts **resolved**, because nothing
    /// will ever clear them otherwise. Before this, editing either config
    /// file — a theme name, a layout tweak — re-fired active alerts or
    /// stranded them until restart (arc 7b review, D57 amendment 25).
    pub fn adopt(&mut self, old: Rules, at: Ts, source: SourceId) -> Vec<AlertEvent> {
        self.started = old.started;
        let mut out = Vec::new();
        for ((name, label), state) in old.states {
            match self.rules.iter().find(|r| r.name == name) {
                Some(_) => {
                    self.states.insert((name, label), state);
                }
                None if state.raised => {
                    out.push(AlertEvent {
                        id: if label.is_empty() {
                            AlertId::new(&name)
                        } else {
                            AlertId::new(&format!("{name}/{label}"))
                        },
                        source,
                        severity: Severity::Info,
                        transition: Transition::Resolved,
                        title: Arc::from(name.as_str()),
                        detail: Arc::from("the rule was removed"),
                        at,
                    });
                }
                None => {}
            }
        }
        out
    }

    /// Evaluate the rules a batch's samples touch. `lookup` reads another
    /// metric for a rule whose right-hand side is a key.
    pub fn observe(
        &mut self,
        source: SourceId,
        at: Ts,
        samples: &[(MetricId, f64)],
        lookup: &dyn Fn(&str, &Label) -> Option<f64>,
    ) -> Vec<AlertEvent> {
        let mut out = Vec::new();
        for (id, value) in samples {
            let label = label_text(&id.label);
            for rule in self.rules.iter() {
                if rule.op == Op::Absent || !rule.matches(id) {
                    continue;
                }
                let threshold = match &rule.rhs {
                    Rhs::Value(v) => Some(*v),
                    Rhs::Key(k) => lookup(k, &id.label),
                };
                let Some(threshold) = threshold else {
                    // The right-hand side has not arrived yet: not a
                    // reason to raise anything.
                    continue;
                };
                let holds = rule.op.holds(*value, threshold);
                let st = self
                    .states
                    .entry((rule.name.clone(), label.clone()))
                    .or_default();
                if let Some(ev) = step(rule, st, holds, at, &label, *value, threshold, source) {
                    out.push(ev);
                }
            }
        }
        out
    }

    /// The `absent` rules, on the frame's clock rather than a batch's.
    ///
    /// A **threshold** rule is not touched here: it changes state only
    /// when a sample arrives, so one that is raised when its source dies
    /// stays raised until the source comes back. That is deliberate — the
    /// alternative is inventing a clear from no data — and the tile's
    /// `STALE` badge is what tells a person the source stopped (the doc
    /// comment used to claim otherwise; arc 7b review, D57 amendment 27).
    pub fn tick(&mut self, at: Ts, source: SourceId, known: KnownLabels<'_>) -> Vec<AlertEvent> {
        // The clock the never-seen case counts from: a key that has never
        // arrived is treated as last seen when the rules were installed,
        // so `for_s` still means what it says.
        let started = *self.started.get_or_insert(at);
        let mut out = Vec::new();
        for rule in self.rules.iter() {
            if rule.op != Op::Absent {
                continue;
            }
            let mut labels = known(&rule.key, &rule.label);
            if labels.is_empty() && !rule.label.contains('*') {
                // An exact label (including the empty one, for a key that
                // carries none) is a name we can watch for even before it
                // exists.
                // Nothing has ever published it: that is the absence the
                // rule was written for, dated from installation.
                labels.push((rule.label.clone(), started));
            }
            for (label, last) in labels {
                // `for_s` is how long the key may be missing before this
                // counts as absent; it is not a *second* hold on top, so
                // the state machine sees zero holds. Only the two
                // durations differ, so this borrows the rule rather than
                // cloning its four strings per label per frame.
                let gone = at.since(last) > rule.for_s;
                let immediate = Rule {
                    for_s: Duration::ZERO,
                    clear_s: Duration::ZERO,
                    ..rule.clone()
                };
                let st = self
                    .states
                    .entry((rule.name.clone(), label.clone()))
                    .or_default();
                if let Some(ev) = step(&immediate, st, gone, at, &label, 0.0, 0.0, source) {
                    out.push(ev);
                }
            }
        }
        out
    }

    /// Everything currently raised, for the tests and `config check`.
    pub fn raised(&self) -> Vec<(String, String)> {
        self.states
            .iter()
            .filter(|(_, s)| s.raised)
            .map(|((name, label), _)| (name.clone(), label.clone()))
            .collect()
    }
}

/// One rule/label's state machine: the two holds, and the event a
/// transition produces.
#[allow(clippy::too_many_arguments)]
fn step(
    rule: &Rule,
    st: &mut State,
    holds: bool,
    at: Ts,
    label: &str,
    value: f64,
    threshold: f64,
    source: SourceId,
) -> Option<AlertEvent> {
    if holds {
        st.clear_since = None;
        let since = *st.since.get_or_insert(at);
        if !st.raised && at.since(since) >= rule.for_s {
            st.raised = true;
            return Some(AlertEvent {
                id: rule.alert_id(label),
                source,
                severity: rule.severity,
                transition: Transition::Raised,
                title: Arc::from(rule.name.as_str()),
                detail: Arc::from(rule.render(label, value, threshold).as_str()),
                at,
            });
        }
    } else {
        st.since = None;
        let clear_since = *st.clear_since.get_or_insert(at);
        if st.raised && at.since(clear_since) >= rule.clear_s {
            st.raised = false;
            return Some(AlertEvent {
                id: rule.alert_id(label),
                source,
                severity: rule.severity,
                transition: Transition::Resolved,
                title: Arc::from(rule.name.as_str()),
                detail: Arc::from(rule.render(label, value, threshold).as_str()),
                at,
            });
        }
    }
    None
}

/// Split `sensor.temp_c{nvme*:Composite}` into name and label glob.
pub fn split_key(text: &str) -> (String, String) {
    match text.split_once('{') {
        Some((name, rest)) => (
            name.trim().to_string(),
            rest.trim_end().trim_end_matches('}').to_string(),
        ),
        None => (text.trim().to_string(), "*".to_string()),
    }
}

/// Parse one `[[rules]]` table. `known_key` says whether a metric name is
/// in the catalogue — an unknown one is an error, an unknown label is not
/// (labels appear at runtime).
pub fn parse_rule(t: &toml::Table, known_key: &dyn Fn(&str) -> bool) -> Result<Rule, RuleError> {
    let name = t
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let err = |problem: &str| RuleError {
        name: if name.is_empty() {
            "<unnamed>".to_string()
        } else {
            name.clone()
        },
        problem: problem.to_string(),
    };
    if name.is_empty() {
        return Err(err("every rule needs a `name`"));
    }
    let Some(key_text) = t.get("key").and_then(|v| v.as_str()) else {
        return Err(err("no `key`"));
    };
    let (key, label) = split_key(key_text);
    if !known_key(&key) {
        return Err(err(&format!(
            "`{key}` is not a key this build publishes (see `gridwatch keys`)"
        )));
    }
    let op = t
        .get("op")
        .and_then(|v| v.as_str())
        .and_then(Op::parse)
        .ok_or_else(|| err("`op` must be one of > >= < <= == != absent"))?;
    let rhs = match t.get("value") {
        Some(toml::Value::Float(f)) => Rhs::Value(*f),
        Some(toml::Value::Integer(i)) => Rhs::Value(*i as f64),
        Some(toml::Value::String(s)) => {
            let (k, _) = split_key(s);
            if !known_key(&k) {
                return Err(err(&format!("`value` names an unknown key `{k}`")));
            }
            Rhs::Key(k)
        }
        Some(_) => return Err(err("`value` must be a number or a key name")),
        None if op == Op::Absent => Rhs::Value(0.0),
        None => return Err(err("no `value`")),
    };
    let secs = |k: &str| {
        t.get(k)
            .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
            .filter(|f| f.is_finite() && *f >= 0.0)
            .map(Duration::from_secs_f64)
    };
    let for_s = secs("for_s").unwrap_or(Duration::ZERO);
    // An `absent` rule compares the frame clock against the last sample's
    // stamp, and the frame is always later — so `for_s = 0` is
    // permanently true. A rule that says "tell me when this stops" needs
    // to say how long counts as stopped (arc 7b review, D57 amendment 26).
    if op == Op::Absent && for_s.is_zero() {
        return Err(err(
            "`absent` needs a `for_s`: how long the key may be missing before it counts",
        ));
    }
    // A `for_s` that is not a number at all (`for_s = "30"`) silently
    // became zero; say so instead.
    for k in ["for_s", "clear_s"] {
        if let Some(v) = t.get(k)
            && secs(k).is_none()
        {
            return Err(err(&format!(
                "`{k}` must be a non-negative number of seconds, not {v}"
            )));
        }
    }
    let severity = match t.get("severity").and_then(|v| v.as_str()).unwrap_or("warn") {
        "info" => Severity::Info,
        "warn" => Severity::Warn,
        "crit" => Severity::Crit,
        other => return Err(err(&format!("unknown severity `{other}`"))),
    };
    Ok(Rule {
        message: t
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("{key} {label} is {value} ({op} {threshold})")
            .replace("{op}", op.symbol()),
        clear_s: secs("clear_s").unwrap_or(for_s),
        name,
        key,
        label,
        op,
        rhs,
        for_s,
        severity,
    })
}

/// Parse every rule, collecting each problem rather than stopping at the
/// first: `config check` should say all of them.
pub fn parse_all(
    tables: &[toml::Table],
    known_key: &dyn Fn(&str) -> bool,
) -> (Vec<Rule>, Vec<RuleError>) {
    let mut rules = Vec::new();
    let mut errors = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for t in tables {
        match parse_rule(t, known_key) {
            Ok(r) => {
                if seen.contains(&r.name) {
                    errors.push(RuleError {
                        name: r.name.clone(),
                        problem: "two rules share this name".into(),
                    });
                    continue;
                }
                seen.push(r.name.clone());
                rules.push(r);
            }
            Err(e) => errors.push(e),
        }
    }
    (rules, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(k: &str) -> bool {
        crate::key::lookup(k).is_some()
    }

    fn rule(toml_text: &str) -> Rule {
        let t: toml::Table = toml::from_str(toml_text).unwrap();
        parse_rule(&t, &known).expect("the rule parses")
    }

    fn id(name: &'static str, label: &str) -> MetricId {
        MetricId {
            name,
            label: if label.is_empty() {
                Label::None
            } else {
                Label::Name(Arc::from(label))
            },
        }
    }

    #[test]
    fn a_rule_raises_after_its_hold_and_clears_after_the_other() {
        let r = rule(
            r#"name = "gpu-hot"
key = "gpu.temp_c"
op = ">"
value = 84
for_s = 30
clear_s = 10
severity = "crit"
message = "gpu is {value}°C (over {threshold})""#,
        );
        assert_eq!(r.for_s, Duration::from_secs(30));
        assert_eq!(r.clear_s, Duration::from_secs(10));
        assert_eq!(r.severity, Severity::Crit);
        let mut rules = Rules::new(vec![r]);
        let src = SourceId("gpu");
        let hot = |t: u64, v: f64| (Ts(t * 1_000_000_000), vec![(id("gpu.temp_c", "0"), v)]);
        let none = |_: &str, _: &Label| None;
        // Hot, but not for long enough.
        for t in 0..30 {
            let (at, s) = hot(t, 90.0);
            assert!(rules.observe(src, at, &s, &none).is_empty(), "at {t}s");
        }
        let (at, s) = hot(30, 90.0);
        let ev = rules.observe(src, at, &s, &none);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].transition, Transition::Raised);
        assert_eq!(ev[0].id.0.as_ref(), "gpu-hot/0");
        assert_eq!(ev[0].severity, Severity::Crit);
        assert!(ev[0].detail.contains("90.0"), "{}", ev[0].detail);
        assert!(ev[0].detail.contains("84.0"), "{}", ev[0].detail);
        // It does not raise twice.
        let (at, s) = hot(40, 91.0);
        assert!(rules.observe(src, at, &s, &none).is_empty());
        assert_eq!(rules.raised().len(), 1);
        // Cool, but not for long enough to clear.
        // Cool from t = 41; the clear hold is ten seconds, so it resolves
        // at t = 51 and not a tick earlier.
        for t in 41..=51 {
            let (at, s) = hot(t, 70.0);
            let ev = rules.observe(src, at, &s, &none);
            if t < 51 {
                assert!(ev.is_empty(), "cleared too early at {t}s");
            } else {
                assert_eq!(ev[0].transition, Transition::Resolved, "at {t}s");
            }
        }
        assert!(rules.raised().is_empty());
        // And it can raise again.
        for t in 52..=82 {
            let (at, s) = hot(t, 95.0);
            let ev = rules.observe(src, at, &s, &none);
            if t == 82 {
                assert_eq!(ev[0].transition, Transition::Raised, "at {t}s");
            }
        }
    }

    #[test]
    fn one_alert_per_label_and_a_metric_right_hand_side() {
        let r = rule(
            r#"name = "nvme-crit"
key = "sensor.temp_c{nvme*:Composite}"
op = ">="
value = "sensor.crit_c"
severity = "crit""#,
        );
        assert_eq!(r.key, "sensor.temp_c");
        assert_eq!(r.label, "nvme*:Composite");
        assert_eq!(r.rhs, Rhs::Key("sensor.crit_c".into()));
        let mut rules = Rules::new(vec![r]);
        let crit = |name: &str, label: &Label| -> Option<f64> {
            (name == "sensor.crit_c").then(|| match label_text(label).as_str() {
                "nvme:Composite" => 84.85,
                "nvme#2:Composite" => 87.85,
                _ => 80.0,
            })
        };
        let samples = vec![
            (id("sensor.temp_c", "nvme:Composite"), 85.0),
            (id("sensor.temp_c", "nvme#2:Composite"), 60.0),
            (id("sensor.temp_c", "k10temp:Tctl"), 99.0),
        ];
        let ev = rules.observe(SourceId("sensors"), Ts(1), &samples, &crit);
        // Only the nvme that passed *its own* threshold, and the k10temp
        // label does not match the glob at all.
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].id.0.as_ref(), "nvme-crit/nvme:Composite");
        // The second drive raises separately when it gets hot.
        let ev = rules.observe(
            SourceId("sensors"),
            Ts(2),
            &[(id("sensor.temp_c", "nvme#2:Composite"), 90.0)],
            &crit,
        );
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].id.0.as_ref(), "nvme-crit/nvme#2:Composite");
        assert_eq!(rules.raised().len(), 2);
        // A right-hand side that has not arrived raises nothing.
        let ev = rules.observe(
            SourceId("sensors"),
            Ts(3),
            &[(id("sensor.temp_c", "nvme#9:Composite"), 200.0)],
            &|_, _| None,
        );
        assert!(ev.is_empty());
    }

    #[test]
    fn absent_raises_when_the_key_stops_arriving() {
        let r = rule(
            r#"name = "link-down"
key = "net.rx_bps{eno1}"
op = "absent"
for_s = 10
severity = "warn""#,
        );
        let mut rules = Rules::new(vec![r]);
        let src = SourceId("net");
        // Fresh: nothing.
        let fresh = |_: &str, _: &str| vec![("eno1".to_string(), Ts(100_000_000_000))];
        assert!(rules.tick(Ts(100_000_000_000), src, &fresh).is_empty());
        assert!(rules.tick(Ts(105_000_000_000), src, &fresh).is_empty());
        // Ten seconds without a sample: raised.
        let ev = rules.tick(Ts(111_000_000_000), src, &fresh);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].transition, Transition::Raised);
        assert_eq!(ev[0].severity, Severity::Warn);
        // It comes back: resolved.
        let back = |_: &str, _: &str| vec![("eno1".to_string(), Ts(112_000_000_000))];
        let ev = rules.tick(Ts(112_000_000_000), src, &back);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].transition, Transition::Resolved);
    }

    #[test]
    fn parsing_says_what_is_wrong_and_never_takes_two_of_a_name() {
        let bad = |text: &str| {
            let t: toml::Table = toml::from_str(text).unwrap();
            parse_rule(&t, &known).unwrap_err().problem
        };
        assert!(bad(r#"key = "gpu.temp_c""#).contains("name"));
        assert!(bad(r#"name = "x""#).contains("key"));
        assert!(
            bad(r#"name = "x"
key = "nonsense.key"
op = ">"
value = 1"#)
            .contains("not a key")
        );
        assert!(
            bad(r#"name = "x"
key = "gpu.temp_c"
op = "~"
value = 1"#)
            .contains("op")
        );
        assert!(
            bad(r#"name = "x"
key = "gpu.temp_c"
op = ">""#)
            .contains("value")
        );
        assert!(
            bad(r#"name = "x"
key = "gpu.temp_c"
op = ">"
value = 1
severity = "loud""#)
            .contains("severity")
        );
        assert!(
            bad(r#"name = "x"
key = "gpu.temp_c"
op = ">"
value = "not.a.key""#)
            .contains("unknown key")
        );
        // Two rules of a name: the second is refused, the first stands.
        let one: toml::Table = toml::from_str(
            r#"name = "dup"
key = "gpu.temp_c"
op = ">"
value = 1"#,
        )
        .unwrap();
        let (rules, errors) = parse_all(&[one.clone(), one], &known);
        assert_eq!(rules.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].problem.contains("share this name"));
        assert!(errors[0].to_string().starts_with("[[rules]] dup:"));
        // The defaults: warn, no hold, clear_s follows for_s.
        let r = rule(
            r#"name = "d"
key = "gpu.temp_c"
op = ">"
value = 1
for_s = 5"#,
        );
        assert_eq!(r.severity, Severity::Warn);
        assert_eq!(r.clear_s, Duration::from_secs(5));
        assert_eq!(r.label, "*");
        assert!(r.message.contains('>'), "{}", r.message);
        assert_eq!(split_key("a.b"), ("a.b".into(), "*".into()));
        assert_eq!(split_key("a.b{c:d}"), ("a.b".into(), "c:d".into()));
        assert!(glob("*", "anything"));
        assert!(glob("nvme*", "nvme#2:Composite"));
        assert!(
            glob("nvme*:Composite", "nvme#2:Composite"),
            "a star in the middle"
        );
        assert!(glob("nvme*:Composite", "nvme:Composite"));
        assert!(!glob("nvme*:Composite", "nvme:Sensor 1"));
        assert!(!glob("nvme*:Composite", "spd5118:Composite"));
        assert!(glob("*:Tctl", "k10temp:Tctl"));
        assert!(glob("k10temp:*", "k10temp:Tccd1"));
        assert!(!glob("", "x"));
        assert!(glob("", ""));
    }
}
