//! Supervisor regressions (§4.3, §11): restart counters must never regress,
//! `[sources.<id>]` options must reach the ctx, and a restarted source must
//! still hear its control channel (zero-poll parking included).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use gridwatch_sources::spawn_source;
use gridwatch_store::{
    Clock, ControlMsg, Source, SourceCtx, SourceId, SourceInfo, SourceState, SourceStatus, channels,
};

struct Flaky {
    attempts: Arc<AtomicU32>,
}

impl Source for Flaky {
    fn info(&self) -> SourceInfo {
        unreachable!("never called by the supervisor")
    }

    fn run(self: Box<Self>, cx: SourceCtx) {
        let n = self.attempts.fetch_add(1, Ordering::SeqCst);
        if n < 2 {
            panic!("boom {n}");
        }
        // Third generation: options survived the restarts, and the ctx stamps
        // the supervisor's restart count over whatever we claim here.
        let probe = cx
            .options
            .get("probe")
            .and_then(|v| v.as_str())
            .unwrap_or("missing")
            .to_string();
        cx.status(SourceStatus {
            state: SourceState::Ok,
            reason: Some(Arc::from(probe.as_str())),
            hint: None,
            since: cx.clock.now(),
            last_sample: None,
            dropped: 0,
            restarts: 0, // deliberately wrong: SourceCtx::status must overwrite
        });
        // Park long: only a live (re-pointed) control channel or the stop flag
        // via that channel wakes this before the deadline.
        while cx.sleep_until(cx.clock.now().plus(Duration::from_secs(30))) {}
    }
}

#[test]
fn restarts_options_and_controls_survive_a_panic() {
    let (ch, inbox) = channels();
    let attempts = Arc::new(AtomicU32::new(0));
    let mk_attempts = attempts.clone();
    let mut options = toml::Table::new();
    options.insert("probe".into(), toml::Value::String("42".into()));
    let handle = spawn_source(
        SourceId("flaky"),
        move || {
            Box::new(Flaky {
                attempts: mk_attempts.clone(),
            }) as Box<dyn Source>
        },
        ch.clone(),
        Clock::real_starting_now(),
        options,
    );
    let mut seen = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while seen.len() < 3 && std::time::Instant::now() < deadline {
        if let Ok(ControlMsg::Status(_, s)) = inbox.control.recv_timeout(Duration::from_millis(200))
        {
            seen.push(s);
        }
    }
    assert_eq!(seen.len(), 3, "expected 3 statuses (2 panics + 1 ok)");
    assert_eq!(seen[0].restarts, 1);
    assert_eq!(seen[1].restarts, 2);
    assert_eq!(seen[2].state, SourceState::Ok);
    assert_eq!(seen[2].restarts, 2, "ctx must stamp the supervisor count");
    assert_eq!(
        seen[2].reason.as_deref(),
        Some("42"),
        "options must reach ctx"
    );
    // The parked third generation must hear Stop promptly on its new channel.
    let t0 = std::time::Instant::now();
    handle.shutdown();
    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "stop did not reach the restarted source"
    );
}
