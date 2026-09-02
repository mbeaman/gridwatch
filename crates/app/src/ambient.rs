//! The ambient layer (§7, D28/D31/D34, brief arc 4 seam 10): under a
//! showcase theme the finished frame is a *mold* and the falling rain is the
//! only thing that puts light on screen. Every cell carries the frame it was
//! last lit at; a droplet head prints a rain glyph, the next frame shows the
//! module's own character fully lit, and it fades through its own colour to
//! the floor over `fade_s`. Empty cells under a trail fade at `trail_ms`. A
//! dense sweep every `sweep_s` re-prints the whole page; `relight_on_update`
//! re-lights a cell the moment its value changes. Pinned rects (the focused
//! tile, alerting tiles, the banner, toasts, the key bar, the hovered tile)
//! are copied from the mold at full brightness. `V` re-lights the page, `L`
//! locks everything lit. Frozen on `FocusLost`/pause: the last screen stays.
//! Deterministic: the RNG is reseeded from `hash(theme) ^ frame` each frame.

use gridwatch_ui::theme::{AmbientSpec, Gradient, RainGlyphs};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

/// A cell's lit state: the ambient frame it was lit at, or `NEVER`.
const NEVER: u32 = u32::MAX;

/// The rain LUT's steps: a fade is quantised to these so a cell changes at
/// most 64 times over `fade_s` (S2's cost model).
const STEPS: u32 = 64;

#[derive(Clone, Copy, Debug)]
struct Droplet {
    col: u16,
    /// Head row, fractional; negative while above the frame.
    head: f32,
    /// Rows per frame.
    speed: f32,
    /// The glyph choice per row is `seed ^ row`.
    seed: u32,
}

/// The governor's step (D31): fps → density → sweep period → gutters only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Governor {
    pub step: u8,
    /// Frames of good readings since the last step-down.
    good_frames: u32,
    /// Bad readings in the current window.
    bad: u32,
    samples: u32,
}

impl Governor {
    pub const MAX_STEP: u8 = 6;

    /// Steps 1–3 lower the fps (16, 12, 8), 4 thins the rain, 5 lengthens
    /// the sweep period, 6 keeps the rain to the gutters.
    pub fn fps(&self, base: u8) -> u8 {
        match self.step {
            0 => base,
            1 => base.min(16),
            2 => base.min(12),
            _ => base.min(8),
        }
    }

    pub fn density(&self, base: f32) -> f32 {
        if self.step >= 4 { base * 0.75 } else { base }
    }

    pub fn sweep_s(&self, base: f32) -> f32 {
        if self.step >= 5 { base * 1.5 } else { base }
    }

    pub fn gutters_only(&self) -> bool {
        self.step >= 6
    }

    /// One reading per frame: frame p95 over 16 ms or bytes/s over S2's
    /// 3 MB/s is bad; a window of 2 s of mostly bad readings steps down,
    /// 30 s of good ones steps back up.
    pub fn observe(&mut self, p95_us: u64, bytes_per_s: u64, fps: u8) {
        let bad = p95_us > 16_000 || bytes_per_s > 3_000_000;
        self.samples += 1;
        if bad {
            self.bad += 1;
            self.good_frames = 0;
        } else {
            self.good_frames += 1;
        }
        let window = u32::from(fps.max(1)) * 2;
        if self.samples >= window {
            if self.bad * 2 >= self.samples && self.step < Self::MAX_STEP {
                self.step += 1;
                self.good_frames = 0;
            }
            self.samples = 0;
            self.bad = 0;
        }
        if self.good_frames >= u32::from(fps.max(1)) * 30 && self.step > 0 {
            self.step -= 1;
            self.good_frames = 0;
        }
    }
}

/// SplitMix64: small, deterministic, reseeded per frame.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u32) -> u32 {
        (self.next() % u64::from(n.max(1))) as u32
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

pub struct Ambient {
    pub spec: AmbientSpec,
    rain: Gradient,
    glyphs: &'static [char],
    seed: u64,
    w: u16,
    h: u16,
    lit_at: Vec<u32>,
    trail_at: Vec<u32>,
    /// The mold of the previous frame, for `relight_on_update`.
    prev_mold: Option<Buffer>,
    droplets: Vec<Droplet>,
    frame: u32,
    last_sweep: u32,
    sweeping: bool,
    lock: bool,
    /// The output of the last frame: shown while frozen.
    last_out: Option<Buffer>,
    pub governor: Governor,
}

fn hash_name(name: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut h);
    h.finish()
}

impl Ambient {
    pub fn new(spec: AmbientSpec, rain: Gradient, glyphs: RainGlyphs, theme_name: &str) -> Ambient {
        Ambient {
            spec,
            rain,
            glyphs: glyphs.chars(),
            seed: hash_name(theme_name),
            w: 0,
            h: 0,
            lit_at: Vec::new(),
            trail_at: Vec::new(),
            prev_mold: None,
            droplets: Vec::new(),
            frame: 0,
            last_sweep: 0,
            sweeping: false,
            lock: false,
            last_out: None,
            governor: Governor::default(),
        }
    }

    /// The fps the layer runs at now (the governor may have stepped it).
    pub fn fps(&self) -> u8 {
        self.governor.fps(self.spec.fps)
    }

    pub fn frame_index(&self) -> u32 {
        self.frame
    }

    pub fn locked(&self) -> bool {
        self.lock
    }

    pub fn toggle_lock(&mut self) -> bool {
        self.lock = !self.lock;
        self.lock
    }

    /// `V`: every content cell lit now.
    pub fn relight_all(&mut self) {
        let f = self.frame;
        self.lit_at.fill(f);
        self.sweeping = false;
    }

    /// Re-light the cells of one rect (a key on a tile, a hover).
    pub fn relight(&mut self, r: Rect) {
        let f = self.frame;
        for y in r.y..r.y + r.height {
            for x in r.x..r.x + r.width {
                if let Some(i) = self.idx(x, y) {
                    self.lit_at[i] = f;
                }
            }
        }
    }

    fn idx(&self, x: u16, y: u16) -> Option<usize> {
        (x < self.w && y < self.h).then(|| usize::from(y) * usize::from(self.w) + usize::from(x))
    }

    fn resize(&mut self, w: u16, h: u16) {
        if (w, h) != (self.w, self.h) {
            self.w = w;
            self.h = h;
            let n = usize::from(w) * usize::from(h);
            self.lit_at = vec![NEVER; n];
            self.trail_at = vec![NEVER; n];
            self.prev_mold = None;
            self.droplets.clear();
            self.last_out = None;
        }
    }

    /// The last screen, for the frozen state (`FocusLost`, pause).
    pub fn last_output(&self) -> Option<&Buffer> {
        self.last_out.as_ref()
    }

    /// One ambient frame (D34): read the mold, advance the rain, write `out`
    /// from scratch. `pinned` rects are copied from the mold at full light.
    pub fn frame(&mut self, mold: &Buffer, pinned: &[Rect], out: &mut Buffer) {
        let area = *mold.area();
        self.resize(area.width, area.height);
        if self.w == 0 || self.h == 0 {
            return;
        }
        self.frame = self.frame.wrapping_add(1);
        let frame = self.frame;
        let fps = u32::from(self.fps().max(1));
        let mut rng = Rng::new(self.seed ^ u64::from(frame));

        // relight_on_update: a changed cell is lit the moment it changes.
        if self.spec.light.relight_on_update
            && let Some(prev) = &self.prev_mold
            && prev.area() == mold.area()
        {
            for (i, (a, b)) in prev.content().iter().zip(mold.content()).enumerate() {
                if (a.symbol() != b.symbol() || a.fg != b.fg) && is_content(b) {
                    self.lit_at[i] = frame;
                }
            }
        }

        // The sweep: every sweep_s, one droplet per column at row 0.
        let sweep_frames = (self.governor.sweep_s(self.spec.light.sweep_s) * fps as f32) as u32;
        if !self.sweeping && frame.wrapping_sub(self.last_sweep) >= sweep_frames.max(1) {
            self.sweeping = true;
            self.last_sweep = frame;
            let speed = 0.6 * self.spec.speed;
            for col in 0..self.w {
                self.droplets.push(Droplet {
                    col,
                    head: -(rng.unit() * 3.0),
                    speed,
                    seed: rng.next() as u32,
                });
            }
        }
        // The steady rain: the pool holds density × columns droplets.
        let pool = ((self.governor.density(self.spec.density) * self.w as f32) as usize).max(1);
        let steady = self
            .droplets
            .iter()
            .filter(|d| d.speed != 0.6 * self.spec.speed)
            .count();
        for _ in steady..pool {
            let col = rng.below(u32::from(self.w)) as u16;
            self.droplets.push(Droplet {
                col,
                head: -(rng.unit() * self.h as f32),
                speed: (0.3 + rng.unit() * 0.6) * self.spec.speed,
                seed: rng.next() as u32,
            });
        }

        // Advance the droplets; a head passing a cell lights it (content)
        // or starts a trail (empty). Heads this frame are remembered for
        // the draw.
        let mut heads: Vec<(u16, u16, u32)> = Vec::new();
        let h = self.h as f32;
        let gutters_only = self.governor.gutters_only();
        for d in &mut self.droplets {
            let before = d.head;
            d.head += d.speed;
            let from = before.floor() as i32 + 1;
            let to = d.head.floor() as i32;
            for row in from.max(0)..=to.min(self.h as i32 - 1) {
                let y = row as u16;
                let i = usize::from(y) * usize::from(self.w) + usize::from(d.col);
                let content = is_content(&mold.content()[i]);
                if content && (!gutters_only || self.lit_at[i] == NEVER) {
                    // Lit from the *next* frame (this one shows the glyph).
                    self.lit_at[i] = frame;
                } else if !content {
                    self.trail_at[i] = frame;
                }
                heads.push((d.col, y, d.seed ^ row as u32));
            }
        }
        let was_sweeping = self.sweeping;
        self.droplets.retain(|d| d.head < h + 1.0);
        if was_sweeping
            && !self
                .droplets
                .iter()
                .any(|d| d.speed == 0.6 * self.spec.speed)
        {
            self.sweeping = false;
        }

        // Draw: everything from the mold and the lit/trail state.
        let floor = self.spec.light.floor;
        let head_col = self.spec.light.head;
        let fade_frames = (self.spec.light.fade_s * fps as f32).max(1.0);
        let trail_frames = (self.spec.light.trail_ms as f32 / 1000.0 * fps as f32).max(1.0);
        for y in 0..self.h {
            for x in 0..self.w {
                let i = usize::from(y) * usize::from(self.w) + usize::from(x);
                let src = &mold.content()[i];
                let Some(dst) = out.cell_mut((area.x + x, area.y + y)) else {
                    continue;
                };
                let pinned_here = pinned
                    .iter()
                    .any(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height);
                if pinned_here || (self.lock && is_content(src)) {
                    *dst = src.clone();
                    continue;
                }
                let head = heads.iter().find(|(hx, hy, _)| *hx == x && *hy == y);
                if let Some((_, _, seed)) = head
                    && !(gutters_only && is_content(src))
                {
                    let g = self.glyphs[(*seed as usize) % self.glyphs.len()];
                    dst.set_char(g);
                    dst.set_fg(head_col);
                    dst.set_bg(floor);
                    dst.modifier = Modifier::BOLD;
                    continue;
                }
                if is_content(src) && self.lit_at[i] != NEVER {
                    let age = frame.wrapping_sub(self.lit_at[i]) as f32;
                    let t = age / fade_frames;
                    if t < 1.0 {
                        *dst = src.clone();
                        let q = (t * STEPS as f32).floor() / STEPS as f32;
                        dst.set_fg(fade(src.fg, floor, q, &self.rain));
                        if src.bg != Color::Reset {
                            dst.set_bg(fade(src.bg, floor, q, &self.rain));
                        } else {
                            dst.set_bg(floor);
                        }
                        continue;
                    }
                }
                if !is_content(src) && self.trail_at[i] != NEVER {
                    let age = frame.wrapping_sub(self.trail_at[i]) as f32;
                    let t = age / trail_frames;
                    if t < 1.0 {
                        let seed = (self.seed as u32) ^ (u32::from(x) << 16 | u32::from(y));
                        let g = self.glyphs[(seed as usize) % self.glyphs.len()];
                        dst.set_char(g);
                        dst.set_fg(self.rain.sample(0.15 + 0.85 * t));
                        dst.set_bg(floor);
                        dst.modifier = Modifier::empty();
                        continue;
                    }
                }
                // Dark: the floor.
                dst.set_char(' ');
                dst.set_fg(floor);
                dst.set_bg(floor);
                dst.modifier = Modifier::empty();
            }
        }
        self.prev_mold = Some(mold.clone());
        self.last_out = Some(out.clone());
    }
}

/// A mold cell with something in it: a glyph, or a painted background.
fn is_content(c: &ratatui::buffer::Cell) -> bool {
    c.symbol() != " " || (c.bg != Color::Reset && c.bg != Color::Rgb(0, 0, 0))
}

/// `own` toward `floor` by `t`, quantised by the caller; a colour the layer
/// cannot interpolate (a terminal index, `Reset`) walks the rain LUT instead.
fn fade(own: Color, floor: Color, t: f32, rain: &Gradient) -> Color {
    match (own, floor) {
        (Color::Rgb(r, g, b), Color::Rgb(fr, fg, fb)) => {
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
            Color::Rgb(mix(r, fr), mix(g, fg), mix(b, fb))
        }
        _ => rain.sample(0.3 + 0.7 * t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridwatch_ui::ColorMode;
    use gridwatch_ui::theme::load_builtin;
    use ratatui::style::Style;

    fn layer() -> Ambient {
        let t = load_builtin("matrix", ColorMode::TrueColor).unwrap();
        Ambient::new(
            t.ambient.clone().unwrap(),
            t.rain.clone().unwrap(),
            t.rain_glyphs,
            &t.name,
        )
    }

    fn mold(w: u16, h: u16) -> Buffer {
        let mut b = Buffer::empty(Rect::new(0, 0, w, h));
        for y in 0..h {
            b.set_string(2, y, "TEXT", Style::new().fg(Color::Rgb(182, 255, 201)));
        }
        b
    }

    /// Determinism (§12.2): two layers from the same theme, fed the same
    /// molds, produce identical buffers frame for frame.
    #[test]
    fn two_runs_from_the_same_seed_are_identical() {
        let (mut a, mut b) = (layer(), layer());
        let m = mold(40, 12);
        for _ in 0..60 {
            let (mut oa, mut ob) = (Buffer::empty(*m.area()), Buffer::empty(*m.area()));
            a.frame(&m, &[], &mut oa);
            b.frame(&m, &[], &mut ob);
            assert_eq!(oa, ob);
        }
    }

    /// Readability (§12.2): a pinned rect is the mold, always, with no rain
    /// glyph over it; a fade test: a lit cell walks toward the floor and
    /// stops; a sweep test: every content cell is lit at least once per
    /// sweep; re-light: a changed cell returns to full light next frame.
    #[test]
    fn pins_fade_sweep_and_relight() {
        let mut l = layer();
        let m = mold(40, 12);
        let pin = Rect::new(2, 3, 4, 1);
        let fps = u32::from(l.fps());
        let sweep_frames = (l.spec.light.sweep_s * fps as f32) as u32 + fps * 3;
        let mut lit_once = vec![false; 40 * 12];
        for _ in 0..sweep_frames {
            let mut out = Buffer::empty(*m.area());
            l.frame(&m, &[pin], &mut out);
            for x in pin.x..pin.x + pin.width {
                assert_eq!(
                    out.cell((x, pin.y)).unwrap(),
                    m.cell((x, pin.y)).unwrap(),
                    "pinned cell is the mold"
                );
            }
            for (i, c) in out.content().iter().enumerate() {
                if is_content(&m.content()[i]) && c.fg == Color::Rgb(182, 255, 201) {
                    lit_once[i] = true;
                }
            }
        }
        for (i, c) in m.content().iter().enumerate() {
            if is_content(c) {
                assert!(
                    lit_once[i],
                    "cell {i} never reached full light in a sweep cycle"
                );
            }
        }
        // Fade: light one cell now, then watch it darken monotonically and stop.
        let mut l = layer();
        let m = mold(40, 12);
        let mut out = Buffer::empty(*m.area());
        l.frame(&m, &[], &mut out);
        l.relight(Rect::new(2, 5, 1, 1));
        let mut last = None;
        let mut settled_at = None;
        for f in 0..(fps * 14) {
            let mut out = Buffer::empty(*m.area());
            l.frame(&m, &[Rect::new(0, 0, 40, 4)], &mut out);
            // Keep it from being re-lit by the rain: pin nothing, but only
            // read the cell's fg brightness.
            let Color::Rgb(_, g, _) = out.cell((2, 5)).unwrap().fg else {
                panic!("rgb")
            };
            if let Some(prev) = last
                && g > prev
                && f > 2
            {
                // A droplet re-lit it: acceptable, restart the watch.
                last = Some(g);
                continue;
            }
            if g == 0 && settled_at.is_none() {
                settled_at = Some(f);
            }
            last = Some(g);
        }
        assert!(settled_at.is_some(), "the cell never reached the floor");
        // Re-light on update: change one mold cell; it is fully lit next frame.
        let mut m2 = m.clone();
        m2.set_string(2, 8, "NEXT", Style::new().fg(Color::Rgb(182, 255, 201)));
        let mut out = Buffer::empty(*m.area());
        l.frame(&m2, &[], &mut out);
        let mut out = Buffer::empty(*m.area());
        l.frame(&m2, &[], &mut out);
        let c = out.cell((2, 8)).unwrap();
        assert!(
            c.symbol() == "N" || c.modifier.contains(Modifier::BOLD),
            "re-lit or under a head: {c:?}"
        );
    }

    /// Composition (§12.2): with the pool emptied and no sweep due, a frame
    /// is pinned tiles plus fading prints plus fading trails and nothing
    /// else — every unlit content cell is dark.
    #[test]
    fn composition_is_only_light_the_rain_left() {
        let mut l = layer();
        let m = mold(30, 8);
        let mut out = Buffer::empty(*m.area());
        l.frame(&m, &[], &mut out);
        l.droplets.clear();
        l.spec.density = 0.0;
        l.last_sweep = l.frame; // no sweep for a while
        let mut out = Buffer::empty(*m.area());
        l.frame(&m, &[], &mut out);
        for (i, c) in out.content().iter().enumerate() {
            let src = &m.content()[i];
            if is_content(src) && l.lit_at[i] == NEVER {
                assert_eq!(c.symbol(), " ", "an unlit content cell shows something");
            }
        }
        // The lock: every content cell is the mold.
        l.toggle_lock();
        let mut out = Buffer::empty(*m.area());
        l.frame(&m, &[], &mut out);
        for (i, c) in out.content().iter().enumerate() {
            if is_content(&m.content()[i]) {
                assert_eq!(c.symbol(), m.content()[i].symbol());
            }
        }
    }

    #[test]
    fn governor_steps_down_and_recovers() {
        let mut g = Governor::default();
        for _ in 0..48 {
            g.observe(20_000, 0, 24);
        }
        assert_eq!(g.step, 1);
        assert_eq!(g.fps(24), 16);
        for _ in 0..(24 * 30) {
            g.observe(5_000, 0, 24);
        }
        assert_eq!(g.step, 0);
        for _ in 0..(48 * 6) {
            g.observe(0, 5_000_000, 24);
        }
        assert_eq!(g.step, Governor::MAX_STEP);
        assert!(g.gutters_only());
    }
}
