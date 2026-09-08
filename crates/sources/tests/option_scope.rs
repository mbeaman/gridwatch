//! The nine keys whose *resolved value* changed when arc 13 made a
//! non-positive interval a rejection (D63 review amendment R2).
//!
//! Before arc 13, `refresh_ms = 0` meant two different things depending on
//! which source you wrote it under: cpu and gpu discarded it and kept their
//! default, while pins, sensors, mpris and net clamped it *up* to their
//! fastest allowed rate and used it — behaviour no reader of `config.toml`
//! could have predicted, and which nothing pinned. It is one rule now: a
//! non-positive interval is `Rejected` and the default stands. These cases
//! exist so the choice is visible and cannot drift back silently.

use gridwatch_store::{IssueKind, OptionIssue};

/// A source's id, one of its interval keys, and the pure reader its
/// `SourceDef` registers.
type Case = (
    &'static str,
    &'static str,
    fn(&toml::Table) -> Vec<OptionIssue>,
);

/// Every interval key the rule covers, for whichever sources this build has.
/// `clamping_only` keeps the sources whose range floor is above 1 ms, so the
/// `Adjusted` half can ask for a value that is positive *and* out of range.
#[allow(unused_mut, clippy::vec_init_then_push)]
fn interval_cases(clamping_only: bool) -> Vec<Case> {
    let mut cases: Vec<Case> = Vec::new();
    #[cfg(feature = "cpu")]
    cases.push(("cpu", "refresh_ms", gridwatch_sources::cpu::check));
    #[cfg(feature = "gpu")]
    cases.push(("gpu", "refresh_ms", gridwatch_sources::gpu::check));
    #[cfg(feature = "pins")]
    cases.push(("pins", "interval_ms", gridwatch_sources::pins::check));
    if !clamping_only {
        #[cfg(feature = "sensors")]
        cases.push(("sensors", "refresh_ms", gridwatch_sources::sensors::check));
        #[cfg(feature = "mpris")]
        cases.push(("mpris", "poll_ms", gridwatch_sources::mpris::check));
        #[cfg(feature = "net")]
        for k in ["refresh_ms", "link_ms", "conns_ms", "probe_ms"] {
            cases.push(("net", k, gridwatch_sources::net::check));
        }
    }
    assert!(!cases.is_empty(), "no source feature is on");
    cases
}

/// Every `*_ms` key, in every source, on the one rule.
#[test]
fn a_non_positive_interval_is_rejected_and_the_default_stands() {
    for (id, key, check) in interval_cases(false) {
        for value in ["0", "-1"] {
            let t: toml::Table = toml::from_str(&format!("{key} = {value}")).unwrap();
            let issues = check(&t);
            assert_eq!(
                issues.len(),
                1,
                "[sources.{id}] {key} = {value}: {issues:?}"
            );
            assert_eq!(
                issues[0].kind,
                IssueKind::Rejected,
                "[sources.{id}] {key} = {value} must be rejected, not clamped — the value is \
                 discarded, so the default stands (D63's rule)"
            );
            assert_eq!(issues[0].key, key);
            assert!(
                issues[0].text.contains("stands"),
                "the message must name what stands: {:?}",
                issues[0].text
            );
        }
    }
}

/// The other half of the same rule: a *positive* value below the range floor
/// is used after clamping, so it warns and never fails.
#[test]
fn a_positive_interval_out_of_range_is_clamped_and_used() {
    for (id, key, check) in interval_cases(true) {
        let t: toml::Table = toml::from_str(&format!("{key} = 1")).unwrap();
        let issues = check(&t);
        assert_eq!(issues.len(), 1, "[sources.{id}] {key} = 1: {issues:?}");
        assert_eq!(
            issues[0].kind,
            IssueKind::Adjusted,
            "[sources.{id}] {key} = 1 is used after a clamp, so it warns"
        );
        assert!(
            issues[0].text.contains("clamped to"),
            "{:?}",
            issues[0].text
        );
    }
}
