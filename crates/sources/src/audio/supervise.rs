//! The child's lifecycle and the silence rule as pure state machines (brief
//! arc 5 seam 2), so both are tested without PipeWire. The source thread
//! asks `Policy` what to do with the child and `Silence` what the data means;
//! neither touches a process.

use std::time::{Duration, Instant};

use gridwatch_store::Level;

pub const BACKOFF_MIN: Duration = Duration::from_millis(250);
pub const BACKOFF_MAX: Duration = Duration::from_secs(5);
/// The child is killed this long after the demand drops to `Hidden`.
pub const KILL_AFTER_HIDDEN: Duration = Duration::from_secs(10);
/// No frame for this long ⇒ silence (`node.passive` on an idle sink delivers
/// nothing — never a restart).
pub const NO_FRAME_SILENCE: Duration = Duration::from_millis(250);
/// RMS below the floor for this long ⇒ silence.
pub const BELOW_FLOOR_SILENCE: Duration = Duration::from_millis(500);
/// The publish cadence while silent.
pub const SILENT_PERIOD: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Nothing to change.
    Keep,
    /// Start the child now.
    Spawn,
    /// Stop the child (hidden long enough).
    Kill,
    /// The child is gone; start it again at the instant.
    RespawnAt(Instant),
}

/// When to spawn, kill and respawn. `running` is what the caller knows about
/// the child; the policy never assumes.
#[derive(Clone, Debug)]
pub struct Policy {
    backoff: Duration,
    hidden_since: Option<Instant>,
    respawn_at: Option<Instant>,
}

impl Default for Policy {
    fn default() -> Policy {
        Policy {
            backoff: BACKOFF_MIN,
            hidden_since: None,
            respawn_at: None,
        }
    }
}

impl Policy {
    /// Called every loop with the demand and whether a child is running.
    pub fn decide(&mut self, level: Level, running: bool, now: Instant) -> Action {
        if level == Level::Hidden {
            let since = *self.hidden_since.get_or_insert(now);
            self.respawn_at = None;
            if running && now.saturating_duration_since(since) >= KILL_AFTER_HIDDEN {
                return Action::Kill;
            }
            return Action::Keep;
        }
        self.hidden_since = None;
        if running {
            return Action::Keep;
        }
        match self.respawn_at {
            Some(at) if now < at => Action::Keep,
            Some(_) => {
                self.respawn_at = None;
                Action::Spawn
            }
            None => Action::Spawn,
        }
    }

    /// The child exited or closed its stdout: schedule a respawn with the
    /// doubling backoff.
    pub fn on_exit(&mut self, now: Instant) -> Action {
        let at = now + self.backoff;
        self.backoff = (self.backoff * 2).min(BACKOFF_MAX);
        self.respawn_at = Some(at);
        Action::RespawnAt(at)
    }

    /// Frames arrived: the child is healthy, the backoff resets.
    pub fn on_frames(&mut self) {
        self.backoff = BACKOFF_MIN;
    }

    /// A deliberate kill (sink change, `Restart`): respawn at once, no backoff.
    pub fn on_killed(&mut self) {
        self.respawn_at = None;
        self.backoff = BACKOFF_MIN;
    }

    pub fn backoff(&self) -> Duration {
        self.backoff
    }
}

/// The silence rule: > 250 ms without a frame, or RMS below the floor for
/// 500 ms, ⇒ silent; one frame above the floor ⇒ not silent within a tick.
#[derive(Clone, Debug, Default)]
pub struct Silence {
    below_since: Option<Instant>,
    pub silent: bool,
    pub since: Option<Instant>,
}

impl Silence {
    /// `frame_age`: time since the last frame (`None` = never). `rms_db`: of
    /// the frames drained this tick, when any arrived. Returns `true` when the
    /// state changed.
    pub fn observe(
        &mut self,
        now: Instant,
        frame_age: Option<Duration>,
        rms_db: Option<f64>,
        floor_db: f64,
    ) -> bool {
        let no_frames = frame_age.is_none_or(|a| a > NO_FRAME_SILENCE);
        let below = match rms_db {
            Some(db) if db >= floor_db => {
                self.below_since = None;
                false
            }
            Some(_) => {
                let since = *self.below_since.get_or_insert(now);
                now.saturating_duration_since(since) >= BELOW_FLOOR_SILENCE
            }
            None => self
                .below_since
                .is_some_and(|s| now.saturating_duration_since(s) >= BELOW_FLOOR_SILENCE),
        };
        let silent = no_frames || below;
        let changed = silent != self.silent;
        if changed {
            self.silent = silent;
            self.since = Some(now);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ms: u64) -> Instant {
        // A fixed origin so the arithmetic is readable.
        thread_local! { static ORIGIN: Instant = Instant::now(); }
        ORIGIN.with(|o| *o + Duration::from_millis(ms))
    }

    #[test]
    fn spawns_on_first_visible_demand_and_kills_ten_seconds_after_hidden() {
        let mut p = Policy::default();
        assert_eq!(p.decide(Level::Hidden, false, t(0)), Action::Keep);
        assert_eq!(p.decide(Level::Visible, false, t(10)), Action::Spawn);
        assert_eq!(p.decide(Level::Visible, true, t(20)), Action::Keep);
        assert_eq!(p.decide(Level::Hidden, true, t(1_000)), Action::Keep);
        assert_eq!(p.decide(Level::Hidden, true, t(10_999)), Action::Keep);
        assert_eq!(p.decide(Level::Hidden, true, t(11_000)), Action::Kill);
        // Visible again before the kill: the timer resets.
        let mut p = Policy::default();
        p.decide(Level::Hidden, true, t(0));
        p.decide(Level::Focused, true, t(5_000));
        p.decide(Level::Hidden, true, t(6_000));
        assert_eq!(p.decide(Level::Hidden, true, t(15_000)), Action::Keep);
        assert_eq!(p.decide(Level::Hidden, true, t(16_000)), Action::Kill);
        // The source maps `Paused` onto `Hidden` before asking (review):
        // nothing is published and the child falls under the same timer.
        let mut q = Policy::default();
        assert_eq!(q.decide(Level::Hidden, true, t(0)), Action::Keep);
        assert_eq!(q.decide(Level::Hidden, true, t(10_000)), Action::Kill);
    }

    #[test]
    fn respawn_only_on_exit_with_doubling_backoff_and_never_on_no_data() {
        let mut p = Policy::default();
        assert_eq!(p.on_exit(t(0)), Action::RespawnAt(t(250)));
        assert_eq!(p.decide(Level::Visible, false, t(100)), Action::Keep);
        assert_eq!(p.decide(Level::Visible, false, t(250)), Action::Spawn);
        assert_eq!(p.on_exit(t(300)), Action::RespawnAt(t(800)));
        assert_eq!(p.on_exit(t(800)), Action::RespawnAt(t(1_800)));
        p.on_exit(t(0));
        p.on_exit(t(0));
        p.on_exit(t(0));
        assert_eq!(p.backoff(), BACKOFF_MAX, "capped at 5 s");
        p.on_frames();
        assert_eq!(p.backoff(), BACKOFF_MIN, "frames reset it");
        // No data with a running child is not a policy event at all.
        let mut q = Policy::default();
        for ms in (0..60_000).step_by(1_000) {
            assert_eq!(q.decide(Level::Visible, true, t(ms)), Action::Keep);
        }
        // Hidden cancels a pending respawn; visible again spawns at once.
        let mut r = Policy::default();
        r.on_exit(t(0));
        r.decide(Level::Hidden, false, t(10));
        assert_eq!(r.decide(Level::Visible, false, t(20)), Action::Spawn);
        r.on_killed();
        assert_eq!(r.decide(Level::Visible, false, t(21)), Action::Spawn);
    }

    #[test]
    fn silence_after_no_frames_or_a_floor_dwell_and_back_at_once() {
        let mut s = Silence::default();
        let floor = -65.0;
        assert!(s.observe(t(0), None, None, floor), "silent from the start");
        assert!(s.silent);
        assert!(
            s.observe(t(100), Some(Duration::from_millis(5)), Some(-20.0), floor),
            "one loud frame ends it"
        );
        assert!(!s.silent);
        assert!(!s.observe(t(200), Some(Duration::from_millis(200)), None, floor));
        assert!(s.observe(t(500), Some(Duration::from_millis(251)), None, floor));
        assert!(s.silent);
        // Frames flowing but below the floor: 500 ms dwell.
        let mut s = Silence::default();
        s.observe(t(0), Some(Duration::ZERO), Some(-20.0), floor);
        assert!(!s.observe(t(100), Some(Duration::ZERO), Some(-80.0), floor));
        assert!(!s.observe(t(500), Some(Duration::ZERO), Some(-80.0), floor));
        assert!(s.observe(t(600), Some(Duration::ZERO), Some(-80.0), floor));
        assert_eq!(s.since, Some(t(600)));
        assert!(s.observe(t(650), Some(Duration::ZERO), Some(-60.0), floor));
        assert!(!s.silent);
    }
}
