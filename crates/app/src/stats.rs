//! Frame statistics for the F12 HUD and `--stats-log` (P6, P8, P19).

use std::time::Duration;

#[derive(Default)]
pub struct FrameStats {
    samples_us: Vec<u64>,
    pub changed_cells: u64,
    pub frames: u64,
    pub redraw_data: u64,
    pub redraw_anim: u64,
    pub redraw_heartbeat: u64,
    /// P18: milliseconds from the shell starting to the first drawn frame, and
    /// to every source having published at least one sample.
    pub first_frame_ms: u64,
    pub sources_live_ms: u64,
    /// The effects painter's cost on the last frame (P20's evidence).
    pub fx_us: u64,
    /// The ambient layer's governor step (0 = full rain), for S4's evidence.
    pub rain_step: u8,
}

impl FrameStats {
    pub fn record_frame(&mut self, took: Duration) {
        self.frames += 1;
        if self.samples_us.len() >= 512 {
            self.samples_us.remove(0);
        }
        self.samples_us.push(took.as_micros() as u64);
    }

    fn percentile(&self, p: f64) -> u64 {
        if self.samples_us.is_empty() {
            return 0;
        }
        let mut v = self.samples_us.clone();
        v.sort_unstable();
        let i = ((v.len() - 1) as f64 * p).round() as usize;
        v[i]
    }

    pub fn p50_us(&self) -> u64 {
        self.percentile(0.50)
    }

    pub fn p95_us(&self) -> u64 {
        self.percentile(0.95)
    }

    /// p95 over the last `n` frames (the governor's 2 s window, S4).
    pub fn p95_recent_us(&self, n: usize) -> u64 {
        let start = self.samples_us.len().saturating_sub(n.max(1));
        let mut v: Vec<u64> = self.samples_us[start..].to_vec();
        if v.is_empty() {
            return 0;
        }
        v.sort_unstable();
        let i = ((v.len() - 1) as f64 * 0.95).round() as usize;
        v[i]
    }
}

impl FrameStats {
    /// One JSON object per second for `--stats-log` (P-gate evidence). `bytes`
    /// is the terminal writer's own total, so P6's "the HUD counter must agree
    /// with Δwchar within 5 %" can be checked from a single run.
    pub fn json_line(&self, bytes: u64) -> String {
        format!(
            r#"{{"frames":{},"p50_us":{},"p95_us":{},"changed_cells":{},"bytes":{},"redraw_data":{},"redraw_anim":{},"redraw_heartbeat":{},"first_frame_ms":{},"sources_live_ms":{},"fx_us":{},"rain_step":{}}}"#,
            self.frames,
            self.p50_us(),
            self.p95_us(),
            self.changed_cells,
            bytes,
            self.redraw_data,
            self.redraw_anim,
            self.redraw_heartbeat,
            self.first_frame_ms,
            self.sources_live_ms,
            self.fx_us,
            self.rain_step
        )
    }
}
