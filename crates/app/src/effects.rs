//! The effects layer (§7, brief arc 4 seam 6): a theme's `[effects]` hooks
//! mapped to tachyonfx and painted over the finished frame buffer, after
//! tiles, chrome and overlays and before the HUD. Event effects are bounded
//! (≤ 600 ms), area-scoped, and switched off for the run by the budget
//! watchdog when their cost exceeds `[effects] budget_ms` on average.
//! `Effect` is `!Send`; everything here lives on the render thread.

use std::time::{Duration, Instant};

use gridwatch_ui::theme::{EffectHooks, EffectSpec, Role, Theme};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use tachyonfx::{
    CellFilter, Duration as FxDuration, Effect, EffectRenderer, Interpolation, Motion, fx,
};

/// The frame rate the repeating alert pulse is drawn at when nothing else
/// animates (P1 while a banner is up).
pub const PULSE_FPS: u16 = 8;

/// Which hook fired (the shell's events).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hook {
    Startup,
    ThemeSwap,
    Focus,
    Alert,
}

struct Running {
    hook: Hook,
    effect: Effect,
    area: Rect,
}

/// The painter: what is running, when it last ticked, and the watchdog.
pub struct Effects {
    enabled: bool,
    budget_ms: u32,
    running: Vec<Running>,
    /// The run clock at the last paint (`Ts` as a `Duration`): virtual under
    /// replay and in tests, so two replays tick their effects identically.
    last_tick: Option<Duration>,
    /// Paint cost per frame, the last 60 frames (≈ 2 s at 30 fps).
    costs_us: Vec<u64>,
    tripped: bool,
    /// The watchdog's one-time message, for the shell to toast and log.
    pub notice: Option<String>,
}

impl Effects {
    pub fn new(enabled: bool, budget_ms: u32) -> Effects {
        Effects {
            enabled,
            budget_ms: budget_ms.max(1),
            running: Vec::new(),
            last_tick: None,
            costs_us: Vec::new(),
            tripped: false,
            notice: None,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled && !self.tripped
    }

    pub fn running(&self) -> bool {
        !self.running.is_empty()
    }

    pub fn tripped(&self) -> bool {
        self.tripped
    }

    /// Fire a hook over `area` with the theme's spec for it — nothing when
    /// the theme declares none, or effects are off. A hook already running
    /// is replaced (the alert pulse is one effect, not a stack).
    pub fn trigger(
        &mut self,
        hook: Hook,
        hooks: &EffectHooks,
        theme: &Theme,
        area: Rect,
        swap_from: Option<(Color, Color)>,
    ) {
        if !self.enabled() || area.width == 0 || area.height == 0 {
            return;
        }
        let spec = match hook {
            Hook::Startup => hooks.startup.as_ref(),
            Hook::ThemeSwap => hooks.theme_swap.as_ref(),
            Hook::Focus => hooks.focus.as_ref(),
            Hook::Alert => hooks.alert.as_ref(),
        };
        let Some(spec) = spec else { return };
        let Some(effect) = build(hook, spec, theme, swap_from) else {
            return;
        };
        self.cancel(hook);
        if self.running.is_empty() {
            // The first tick after an idle gap must be zero, not the gap
            // (review: a 200 ms fade fired after a 1 s idle finished inside
            // its first tick and never showed).
            self.last_tick = None;
        }
        self.running.push(Running { hook, effect, area });
    }

    pub fn cancel_all(&mut self) {
        self.running.clear();
    }

    pub fn cancel(&mut self, hook: Hook) {
        self.running.retain(|r| r.hook != hook);
    }

    pub fn is_running(&self, hook: Hook) -> bool {
        self.running.iter().any(|r| r.hook == hook)
    }

    /// An *event* effect (startup, theme swap, focus) is mid-flight: those
    /// deserve the full fps for their ≤ 600 ms. The alert pulse alone does
    /// not — it repeats for as long as the banner is up, and the shell runs
    /// it at `PULSE_FPS` so an active alert costs a fraction of P1 (review
    /// measurement: 30 fps for the pulse was 2.65 % of a core).
    pub fn running_event(&self) -> bool {
        self.running.iter().any(|r| r.hook != Hook::Alert)
    }

    /// Process every running effect over the buffer for the time since the
    /// last paint; drop the finished ones; run the watchdog.
    pub fn paint(&mut self, buf: &mut Buffer, now: Duration) {
        if self.running.is_empty() {
            self.last_tick = Some(now);
            self.costs_us.push(0); // the HUD's cost column reads "nothing ran"
            if self.costs_us.len() > 60 {
                self.costs_us.remove(0);
            }
            return;
        }
        let t0 = Instant::now();
        let elapsed = self
            .last_tick
            .map(|t| now.saturating_sub(t))
            .unwrap_or_default();
        self.last_tick = Some(now);
        let tick = FxDuration::from_millis(elapsed.as_millis().min(u128::from(u32::MAX)) as u32);
        // An effect whose area left the buffer (a shrink) can never finish:
        // drop it rather than pin the frame loop at full fps (review).
        let frame_area = *buf.area();
        self.running
            .retain(|r| r.area.intersection(frame_area).area() > 0);
        for r in &mut self.running {
            let area = r.area.intersection(frame_area);
            buf.render_effect(&mut r.effect, area, tick);
        }
        self.running.retain(|r| !r.effect.done());
        let cost = t0.elapsed().as_micros() as u64;
        self.costs_us.push(cost);
        if self.costs_us.len() > 60 {
            self.costs_us.remove(0);
        }
        // The watchdog: a full window's average above the budget switches
        // event effects off for the run (P20), once, with a notice.
        if !self.tripped && self.costs_us.len() >= 60 {
            let avg = self.costs_us.iter().sum::<u64>() / self.costs_us.len() as u64;
            if avg > u64::from(self.budget_ms) * 1000 {
                self.tripped = true;
                self.running.clear();
                self.notice = Some(format!(
                    "effects off: {:.1} ms per frame over the {} ms budget",
                    avg as f64 / 1000.0,
                    self.budget_ms
                ));
            }
        }
    }

    /// The last paint's cost in microseconds (the HUD's effect column).
    pub fn last_cost_us(&self) -> u64 {
        self.costs_us.last().copied().unwrap_or(0)
    }
}

fn motion(s: Option<&str>) -> Motion {
    match s.unwrap_or("left_to_right") {
        "right_to_left" => Motion::RightToLeft,
        "up_to_down" | "top_to_bottom" => Motion::UpToDown,
        "down_to_up" | "bottom_to_top" => Motion::DownToUp,
        _ => Motion::LeftToRight,
    }
}

/// The spec → tachyonfx mapping (seam 6). `swap_from` is the previous
/// theme's `(fg, bg)` for a theme swap.
pub fn build(
    hook: Hook,
    spec: &EffectSpec,
    theme: &Theme,
    swap_from: Option<(Color, Color)>,
) -> Option<Effect> {
    let ms = spec.duration_ms.clamp(50, 600);
    let bg = theme.color(Role::Bg);
    let effect = match (hook, spec.kind.as_str()) {
        (_, "sweep_in") => fx::sweep_in(
            motion(spec.motion.as_deref()),
            12,
            0,
            bg,
            (ms, Interpolation::QuadOut),
        ),
        (_, "fade_in") => fx::fade_from(bg, bg, (ms, Interpolation::QuadOut)),
        (Hook::ThemeSwap, "fade") | (Hook::ThemeSwap, "dissolve") => {
            let (fg, from_bg) = swap_from.unwrap_or((theme.color(Role::Text), bg));
            if spec.kind == "dissolve" {
                fx::coalesce((ms, Interpolation::QuadOut))
            } else {
                fx::fade_from(fg, from_bg, (ms, Interpolation::QuadOut))
            }
        }
        (_, "fade") => fx::fade_from_fg(
            theme.color(Role::BorderFocused),
            (ms, Interpolation::QuadOut),
        ),
        (_, "dissolve") => fx::coalesce((ms, Interpolation::QuadOut)),
        (_, "hsl_pulse") => {
            let lightness = spec.lightness.unwrap_or(25.0);
            let half = spec.period_ms.unwrap_or(900).max(100) / 2;
            let pulse = fx::repeating(fx::ping_pong(fx::hsl_shift(
                Some([0.0, 0.0, lightness]),
                None,
                (half, Interpolation::SineInOut),
            )));
            // `target = "crit_fg"` (§9's excerpt): only the Crit-coloured
            // cells pulse; anything else pulses the whole area.
            match spec.target.as_deref() {
                Some("crit_fg") => pulse.with_filter(CellFilter::FgColor(theme.color(Role::Crit))),
                _ => pulse,
            }
        }
        _ => return None,
    };
    Some(effect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridwatch_ui::ColorMode;
    use gridwatch_ui::theme::load_builtin;
    use ratatui::style::Style;

    fn buffer() -> Buffer {
        let mut b = Buffer::empty(Rect::new(0, 0, 40, 10));
        for y in 0..10 {
            b.set_string(
                0,
                y,
                "XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX",
                Style::new().fg(Color::Rgb(200, 200, 200)),
            );
        }
        b
    }

    /// Each hook builds from retrowave's spec, touches only its area, and a
    /// bounded one finishes inside its duration; the pulse never finishes.
    #[test]
    fn hooks_build_and_stay_inside_their_area_and_duration() {
        let t = load_builtin("retrowave", ColorMode::TrueColor).unwrap();
        let mut fx = Effects::new(true, 4);
        let area = Rect::new(5, 2, 10, 3);
        let t0 = Duration::from_secs(10);
        // Idle first, then the trigger: the first tick must be zero (review).
        fx.paint(&mut buffer(), Duration::from_secs(5));
        fx.trigger(Hook::Focus, &t.effects, &t, area, None);
        assert!(fx.running());
        let mut buf = buffer();
        let untouched = buf.cell((0, 0)).unwrap().clone();
        fx.paint(&mut buf, t0);
        assert!(fx.running(), "the idle gap did not finish the fade");
        fx.paint(&mut buf, t0 + std::time::Duration::from_millis(50));
        assert_eq!(buf.cell((0, 0)).unwrap(), &untouched, "outside the area");
        let inside = buf.cell((5, 2)).unwrap();
        assert_ne!(
            inside.fg,
            Color::Rgb(200, 200, 200),
            "the fade changed the fg"
        );
        // 200 ms later the 200 ms focus fade is done.
        fx.paint(&mut buf, t0 + std::time::Duration::from_millis(400));
        assert!(!fx.running(), "a bounded effect ends");
        // An effect whose area is off the buffer is dropped, not kept forever.
        fx.trigger(Hook::Focus, &t.effects, &t, Rect::new(50, 50, 10, 3), None);
        fx.paint(&mut buf, t0 + std::time::Duration::from_millis(450));
        assert!(!fx.running(), "an off-buffer effect lingered");
        fx.trigger(Hook::Alert, &t.effects, &t, Rect::new(0, 0, 40, 1), None);
        for i in 0..100 {
            fx.paint(
                &mut buf,
                t0 + std::time::Duration::from_millis(500 + i * 100),
            );
        }
        assert!(fx.is_running(Hook::Alert), "the pulse repeats");
        fx.cancel(Hook::Alert);
        assert!(!fx.running());
        // Disabled: nothing runs.
        let mut off = Effects::new(false, 4);
        off.trigger(Hook::Startup, &t.effects, &t, area, None);
        assert!(!off.running());
        // A theme without hooks: nothing runs.
        let mono = load_builtin("mono", ColorMode::Mono).unwrap();
        fx.trigger(Hook::Startup, &mono.effects, &mono, area, None);
        assert!(!fx.running());
    }

    /// The watchdog trips on a sustained overrun and says so once.
    #[test]
    fn watchdog_trips_on_a_sustained_overrun() {
        let t = load_builtin("retrowave", ColorMode::TrueColor).unwrap();
        let mut fx = Effects::new(true, 4);
        // Feed a full window of over-budget costs directly.
        fx.costs_us = vec![9_000; 59];
        fx.trigger(Hook::Alert, &t.effects, &t, Rect::new(0, 0, 40, 1), None);
        let mut buf = buffer();
        fx.paint(&mut buf, Duration::from_secs(1));
        // One more real paint (cheap) keeps the average above 4 ms → tripped.
        assert!(fx.tripped(), "{:?}", fx.notice);
        assert!(
            fx.notice
                .as_deref()
                .unwrap_or("")
                .starts_with("effects off:")
        );
        assert!(!fx.enabled());
        fx.trigger(Hook::Focus, &t.effects, &t, Rect::new(0, 0, 4, 1), None);
        assert!(!fx.running(), "tripped: nothing starts again this run");
    }
}
