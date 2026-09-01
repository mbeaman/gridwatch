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
}

impl FrameStats {
    /// One JSON object per heartbeat for `--stats-log` (P-gate evidence).
    pub fn json_line(&self) -> String {
        format!(
            r#"{{"frames":{},"p50_us":{},"p95_us":{},"changed_cells":{},"redraw_data":{},"redraw_anim":{},"redraw_heartbeat":{}}}"#,
            self.frames,
            self.p50_us(),
            self.p95_us(),
            self.changed_cells,
            self.redraw_data,
            self.redraw_anim,
            self.redraw_heartbeat
        )
    }
}
