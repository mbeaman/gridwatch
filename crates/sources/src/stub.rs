//! Placeholder live sources for kinds whose real implementation lands in a
//! later arc: report Unavailable with the arriving arc, then wait for stop.

use std::sync::Arc;
use std::time::Duration;

use gridwatch_store::{Source, SourceCtx, SourceInfo, SourceState, SourceStatus};

pub struct StubSource {
    pub info: SourceInfo,
    pub reason: &'static str,
    pub hint: &'static str,
}

impl Source for StubSource {
    fn info(&self) -> SourceInfo {
        self.info
    }

    fn run(self: Box<Self>, cx: SourceCtx) {
        cx.status(SourceStatus {
            state: SourceState::Unavailable,
            reason: Some(Arc::from(self.reason)),
            hint: Some(Arc::from(self.hint)),
            since: cx.clock.now(),
            last_sample: None,
            dropped: 0,
            restarts: 0,
        });
        loop {
            let deadline = cx.clock.now().plus(Duration::from_secs(1));
            if !cx.sleep_until(deadline) {
                return;
            }
            while cx.try_control().is_some() {}
            if cx.stopped() {
                return;
            }
        }
    }
}
