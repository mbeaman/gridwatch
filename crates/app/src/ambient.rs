//! The ambient layer (§7, D28/D31/D34, brief arc 4 seam 10): under a
//! showcase theme the finished frame is a *mold* and the falling rain is the
//! only thing that puts light on screen. Every cell carries the frame it was
//! last lit at; a droplet head prints a rain glyph, the next frame shows the
//! module's own character fully lit, and it fades through its own colour to
//! the floor over `fade_s`. Empty cells under a trail fade at `trail_ms`. A
//! dense sweep every `sweep_s` — and on the first frame, after a resize and
//! after a theme swap — re-prints the whole page; `relight_on_update`
//! re-lights a cell the moment its value changes. Pinned rects (the focused
//! tile, alerting tiles, the tab bar, the banner, toasts, the key bar, the
//! hovered tile, the page under an overlay) are the mold at full brightness
//! and start fading only when the pin lifts. `V` re-lights the page, `L`
//! locks every tile lit with rain in the gutters only. Frozen on
//! `FocusLost`/pause: the rain stops and the screen is drawn from the same
//! state, so toasts, a new banner and page changes still show. Deterministic:
//! the RNG is reseeded from `hash(theme) ^ frame` each frame, and nothing
//! here reads a clock. State: three small per-cell vectors, no frame-sized
//! buffers (≈ 17 bytes per cell).

use gridwatch_ui::theme::{AmbientSpec, Gradient, RainGlyphs};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

/// A cell's lit state: the ambient frame it was lit at, or `NEVER`.
const NEVER: u32 = u32::MAX;

/// The fade's steps: a fade is quantised to these so a cell changes at most
/// 64 times over `fade_s` (S2's cost model).
const STEPS: u32 = 64;

#[derive(Clone, Copy, Debug)]
struct Droplet {
    col: u16,
    /// Head row, fractional; negative while above the frame.
    head: f32,
    /// Rows per frame.
    speed: f32,
    /// Part of the sweep (one per column, all at one speed).
    sweep: bool,
    /// The glyph choice per row is `seed ^ row`.
    seed: u32,
}

/// The governor's step (D31): fps → density → sweep period → gutters only.
/// It is fed **once per second** with the last second's frame p95 and
/// bytes/s (the shell's readings), so its windows are in seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Governor {
    pub step: u8,
    /// Seconds of good readings since the last step-down.
    good_s: u32,
    /// Bad readings in the current window.
    bad: u32,
    samples: u32,
}

impl Governor {
    pub const MAX_STEP: u8 = 6;
    /// Seconds per window: both readings bad → step down.
    pub const WINDOW_S: u32 = 2;
    /// Good seconds to step back up.
    pub const RECOVER_S: u32 = 30;

    /// Steps 1–3 lower the fps (16, 12, 8), 4 thins the rain, 5 lengthens
    /// the sweep period, 6 keeps the rain to the gutters (tiles stay lit).
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

    /// One reading per second: frame p95 over 16 ms or bytes/s over S2's
    /// 3 MB/s is bad; a 2 s window of bad readings steps down; 30 s of good
    /// readings step back up.
    pub fn observe(&mut self, p95_us: u64, bytes_per_s: u64) {
        let bad = p95_us > 16_000 || bytes_per_s > 3_000_000;
        self.samples += 1;
        if bad {
            self.bad += 1;
            self.good_s = 0;
        } else {
            self.good_s += 1;
        }
        if self.samples >= Self::WINDOW_S {
            if self.bad * 2 >= self.samples && self.step < Self::MAX_STEP {
                self.step += 1;
                self.good_s = 0;
            }
            self.samples = 0;
            self.bad = 0;
        }
        if self.good_s >= Self::RECOVER_S && self.step > 0 {
            self.step -= 1;
            self.good_s = 0;
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

/// What the shell hands the layer each frame besides the mold.
pub struct Pins<'a> {
    /// Rects that are the mold at full light.
    pub pinned: &'a [Rect],
    /// Every tile's outer rect: under `L` (and the governor's last step)
    /// these are lit and the rain keeps to the gutters between them.
    pub tiles: &'a [Rect],
}

pub struct Ambient {
    pub spec: AmbientSpec,
    rain: Gradient,
    glyphs: &'static [char],
    seed: u64,
    /// The theme's own background (mode-converted) and the floor: a blank
    /// cell painted in either is empty, whatever the colour mode.
    empty_bgs: [Color; 2],
    w: u16,
    h: u16,
    lit_at: Vec<u32>,
    trail_at: Vec<u32>,
    /// A fingerprint of each mold cell last frame, for `relight_on_update`.
    prev: Vec<u64>,
    droplets: Vec<Droplet>,
    frame: u32,
    last_sweep: u32,
    sweeping: bool,
    lock: bool,
    /// `reveal = ["key"]`: a rect lit until the frame in `.1`.
    reveal: Option<(Rect, u32)>,
    pub governor: Governor,
}

fn hash_name(name: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut h);
    h.finish()
}

/// A colour as a small integer key, without formatting (this runs for
/// every cell of every frame).
fn color_key(c: Color) -> u64 {
    match c {
        Color::Reset => 1,
        Color::Rgb(r, g, b) => {
            0x100_0000 | (u64::from(r) << 16) | (u64::from(g) << 8) | u64::from(b)
        }
        Color::Indexed(i) => 0x200_0000 | u64::from(i),
        Color::Black => 0x300_0000,
        Color::Red => 0x300_0001,
        Color::Green => 0x300_0002,
        Color::Yellow => 0x300_0003,
        Color::Blue => 0x300_0004,
        Color::Magenta => 0x300_0005,
        Color::Cyan => 0x300_0006,
        Color::Gray => 0x300_0007,
        Color::DarkGray => 0x300_0008,
        Color::LightRed => 0x300_0009,
        Color::LightGreen => 0x300_000a,
        Color::LightYellow => 0x300_000b,
        Color::LightBlue => 0x300_000c,
        Color::LightMagenta => 0x300_000d,
        Color::LightCyan => 0x300_000e,
        Color::White => 0x300_000f,
    }
}

/// FNV-1a over the glyph bytes and the two colour keys — cheap and stable.
fn cell_fingerprint(c: &ratatui::buffer::Cell) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in c.symbol().bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    for k in [color_key(c.fg), color_key(c.bg)] {
        h ^= k;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h.max(1)
}

fn in_any(rects: &[Rect], x: u16, y: u16) -> bool {
    rects
        .iter()
        .any(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
}

impl Ambient {
    pub fn new(
        spec: AmbientSpec,
        rain: Gradient,
        glyphs: RainGlyphs,
        theme_name: &str,
        theme_bg: Color,
    ) -> Ambient {
        let floor = spec.light.floor;
        Ambient {
            spec,
            rain,
            glyphs: glyphs.chars(),
            seed: hash_name(theme_name),
            empty_bgs: [theme_bg, floor],
            w: 0,
            h: 0,
            lit_at: Vec::new(),
            trail_at: Vec::new(),
            prev: Vec::new(),
            droplets: Vec::new(),
            frame: 0,
            last_sweep: 0,
            sweeping: false,
            lock: false,
            reveal: None,
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

    /// Whether the theme's `reveal` list names an event.
    pub fn reveals(&self, what: &str) -> bool {
        self.spec.reveal.iter().any(|r| r == what)
    }

    /// `V`: every cell lit now, and the next frame sweeps.
    pub fn relight_all(&mut self) {
        let f = self.frame;
        self.lit_at.fill(f);
        self.request_sweep();
    }

    /// The next frame starts a sweep (a resize, a theme swap, `V`).
    pub fn request_sweep(&mut self) {
        self.sweeping = false;
        self.last_sweep = self.frame.wrapping_sub(u32::MAX / 4);
    }

    /// `reveal = ["key"]`: keep `r` lit for `reveal_ms` from now.
    pub fn reveal_for(&mut self, r: Rect) {
        let frames = (u64::from(self.spec.reveal_ms) * u64::from(self.fps().max(1)) / 1000) as u32;
        self.reveal = Some((r, self.frame.wrapping_add(frames.max(1))));
    }

    /// Re-light the cells of one rect now.
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
            self.prev = vec![0; n];
            self.droplets.clear();
            self.reveal = None;
            // A fresh page: the first frame sweeps (review: a resize left
            // the page dark until the next 20 s sweep).
            self.request_sweep();
        }
    }

    fn is_empty(&self, c: &ratatui::buffer::Cell) -> bool {
        c.symbol() == " " && (c.bg == Color::Reset || self.empty_bgs.contains(&c.bg))
    }

    /// One ambient frame (D34): read the mold, advance the rain (unless
    /// `advance` is false — frozen: the rain stands still and the picture is
    /// drawn from the same state, so pinned rects still update), write
    /// `buf` in place. `pins.pinned` rects are the mold at full light and
    /// keep their cells lit for when the pin lifts.
    pub fn frame(&mut self, buf: &mut Buffer, pins: Pins<'_>, advance: bool) {
        let area = *buf.area();
        self.resize(area.width, area.height);
        if self.w == 0 || self.h == 0 {
            return;
        }
        let n = usize::from(self.w) * usize::from(self.h);
        if advance {
            self.frame = self.frame.wrapping_add(1);
        }
        let frame = self.frame;
        let fps = u32::from(self.fps().max(1));
        let mut rng = Rng::new(self.seed ^ u64::from(frame));
        let lock_tiles = self.lock || self.governor.gutters_only();

        // The content mask and the relight-on-update pass, in one read.
        let mut content = vec![false; n];
        for (i, c) in buf.content().iter().enumerate() {
            let is_content = !self.is_empty(c);
            content[i] = is_content;
            if advance && self.spec.light.relight_on_update {
                let fp = cell_fingerprint(c);
                if self.prev[i] != 0 && self.prev[i] != fp && is_content {
                    self.lit_at[i] = frame;
                }
                self.prev[i] = fp;
            }
        }

        if advance {
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
                        sweep: true,
                        seed: rng.next() as u32,
                    });
                }
            }
            // The steady rain: the pool holds density × columns droplets.
            let pool = ((self.governor.density(self.spec.density) * self.w as f32) as usize).max(1);
            let steady = self.droplets.iter().filter(|d| !d.sweep).count();
            for _ in steady..pool {
                let col = rng.below(u32::from(self.w)) as u16;
                self.droplets.push(Droplet {
                    col,
                    head: -(rng.unit() * self.h as f32),
                    speed: (0.3 + rng.unit() * 0.6) * self.spec.speed,
                    sweep: false,
                    seed: rng.next() as u32,
                });
            }
        }

        // Advance the droplets; a head passing a cell lights it (content)
        // or starts a trail (empty). Heads this frame are remembered for
        // the draw. Under a lock the rain keeps to the gutters: a head over
        // a tile lights nothing and prints nothing.
        // Per-cell head seed (+1; 0 = no head) — a map, not a scan per cell.
        let mut heads: Vec<u32> = vec![0; n];
        if advance {
            let h = self.h as f32;
            for d in &mut self.droplets {
                let before = d.head;
                d.head += d.speed;
                let from = before.floor() as i32 + 1;
                let to = d.head.floor() as i32;
                for row in from.max(0)..=to.min(self.h as i32 - 1) {
                    let y = row as u16;
                    if lock_tiles && in_any(pins.tiles, d.col, y) {
                        continue;
                    }
                    let i = usize::from(y) * usize::from(self.w) + usize::from(d.col);
                    if content[i] {
                        self.lit_at[i] = frame;
                    } else {
                        self.trail_at[i] = frame;
                    }
                    heads[i] = (d.seed ^ row as u32).max(1);
                }
            }
            let was_sweeping = self.sweeping;
            self.droplets.retain(|d| d.head < h + 1.0);
            if was_sweeping && !self.droplets.iter().any(|d| d.sweep) {
                self.sweeping = false;
            }
            if let Some((_, until)) = self.reveal
                && frame.wrapping_sub(until) < u32::MAX / 2
            {
                self.reveal = None;
            }
        }

        // Draw in place: everything from the mold cell and the lit/trail
        // state; a pinned cell stays the mold and is marked lit so it fades
        // from full light when the pin lifts (review).
        let floor = self.spec.light.floor;
        let head_col = self.spec.light.head;
        let fade_frames = (self.spec.light.fade_s * fps as f32).max(1.0);
        let trail_frames = (self.spec.light.trail_ms as f32 / 1000.0 * fps as f32).max(1.0);
        let reveal_rect = self.reveal.map(|(r, _)| r);
        for y in 0..self.h {
            for x in 0..self.w {
                let i = usize::from(y) * usize::from(self.w) + usize::from(x);
                let pinned_here = in_any(pins.pinned, x, y)
                    || reveal_rect.is_some_and(|r| in_any(&[r], x, y))
                    || (lock_tiles && in_any(pins.tiles, x, y));
                if pinned_here {
                    if content[i] {
                        self.lit_at[i] = frame;
                    }
                    continue; // the mold cell stays as drawn
                }
                let Some(dst) = buf.cell_mut((area.x + x, area.y + y)) else {
                    continue;
                };
                if heads[i] != 0 {
                    let seed = heads[i];
                    let g = self.glyphs[(seed as usize) % self.glyphs.len()];
                    dst.set_char(g);
                    dst.set_fg(head_col);
                    dst.set_bg(floor);
                    dst.modifier = Modifier::BOLD;
                    continue;
                }
                if content[i] && self.lit_at[i] != NEVER {
                    let age = frame.wrapping_sub(self.lit_at[i]) as f32;
                    let t = age / fade_frames;
                    if t < 1.0 {
                        let q = (t * STEPS as f32).floor() / STEPS as f32;
                        let fg = fade(dst.fg, floor, q, &self.rain);
                        let bg = if dst.bg == Color::Reset {
                            floor
                        } else {
                            fade(dst.bg, floor, q, &self.rain)
                        };
                        dst.set_fg(fg);
                        dst.set_bg(bg);
                        continue;
                    }
                }
                if !content[i] && self.trail_at[i] != NEVER {
                    let age = frame.wrapping_sub(self.trail_at[i]) as f32;
                    let t = age / trail_frames;
                    if t < 1.0 {
                        let seed = (self.seed as u32) ^ ((u32::from(x) << 16) | u32::from(y));
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
    }
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
    use gridwatch_ui::theme::{Role, load_builtin};
    use ratatui::style::Style;

    const TEXT: Color = Color::Rgb(182, 255, 201);

    fn layer() -> Ambient {
        let t = load_builtin("matrix", ColorMode::TrueColor).unwrap();
        Ambient::new(
            t.ambient.clone().unwrap(),
            t.rain.clone().unwrap(),
            t.rain_glyphs,
            &t.name,
            t.color(Role::Bg),
        )
    }

    fn mold(w: u16, h: u16) -> Buffer {
        let mut b = Buffer::empty(Rect::new(0, 0, w, h));
        for y in 0..h {
            b.set_string(2, y, "TEXT", Style::new().fg(TEXT));
        }
        b
    }

    fn run(l: &mut Ambient, m: &Buffer, pinned: &[Rect], advance: bool) -> Buffer {
        let mut out = m.clone();
        l.frame(&mut out, Pins { pinned, tiles: &[] }, advance);
        out
    }

    /// Determinism (§12.2): two layers from the same theme, fed the same
    /// molds, produce identical buffers frame for frame.
    #[test]
    fn two_runs_from_the_same_seed_are_identical() {
        let (mut a, mut b) = (layer(), layer());
        let m = mold(40, 12);
        for _ in 0..60 {
            assert_eq!(run(&mut a, &m, &[], true), run(&mut b, &m, &[], true));
        }
    }

    /// Readability: a pinned rect is the mold at every frame of a sweep cycle
    /// with no rain glyph over it; the first frame sweeps, so every content
    /// cell reaches full light inside the first seconds; a frozen frame
    /// changes nothing but the pinned rects.
    #[test]
    fn pins_first_sweep_and_freeze() {
        let mut l = layer();
        let m = mold(40, 12);
        let pin = Rect::new(2, 3, 4, 1);
        let fps = u32::from(l.fps());
        let cycle = (l.spec.light.sweep_s * fps as f32) as u32 + fps * 3;
        let mut lit_once = vec![false; 40 * 12];
        for f in 0..cycle {
            let out = run(&mut l, &m, &[pin], true);
            for x in pin.x..pin.x + pin.width {
                assert_eq!(out.cell((x, pin.y)).unwrap(), m.cell((x, pin.y)).unwrap());
            }
            for (i, c) in out.content().iter().enumerate() {
                if !l.is_empty(&m.content()[i]) && c.fg == TEXT {
                    lit_once[i] = true;
                }
            }
            if f == fps * 3 {
                let lit = lit_once.iter().filter(|b| **b).count();
                assert!(lit >= 40, "only {lit} cells lit after the first sweep");
            }
        }
        for (i, c) in m.content().iter().enumerate() {
            if !l.is_empty(c) {
                assert!(lit_once[i], "cell {i} never reached full light");
            }
        }
        let a = run(&mut l, &m, &[pin], false);
        let b = run(&mut l, &m, &[pin], false);
        assert_eq!(a, b, "a frozen frame is stable");
        let mut toast = m.clone();
        toast.set_string(10, 11, "paused", Style::new().fg(TEXT));
        let c = run(&mut l, &toast, &[pin, Rect::new(10, 11, 6, 1)], false);
        assert_eq!(
            c.cell((10, 11)).unwrap().symbol(),
            "p",
            "a pinned toast shows while frozen"
        );
    }

    /// Fade: a lit cell walks toward the floor and then stops changing;
    /// re-light on update lights exactly the changed cells; `V` lights all.
    #[test]
    fn fade_stops_relight_is_exact_and_v_lights_all() {
        let mut l = layer();
        let m = mold(40, 12);
        let _ = run(&mut l, &m, &[], true);
        l.droplets.clear();
        l.spec.density = 0.0;
        l.sweeping = true; // no sweep interrupts the watch
        l.relight(Rect::new(2, 5, 1, 1));
        let fps = u32::from(l.fps());
        let mut greens = Vec::new();
        for _ in 0..(fps * 14) {
            let out = run(&mut l, &m, &[], true);
            let Color::Rgb(_, g, _) = out.cell((2, 5)).unwrap().fg else {
                panic!("rgb")
            };
            greens.push(g);
        }
        assert!(
            greens.windows(2).all(|w| w[1] <= w[0]),
            "monotone: {greens:?}"
        );
        assert_eq!(*greens.last().unwrap(), 0, "reached the floor");
        let settled = greens.iter().rposition(|g| *g > 0).unwrap();
        assert!(greens[settled + 1..].iter().all(|g| *g == 0), "stays there");
        let before = l.lit_at.clone();
        let mut m2 = m.clone();
        m2.set_string(2, 8, "ABCD", Style::new().fg(TEXT)); // every cell differs from "TEXT"
        let _ = run(&mut l, &m2, &[], true);
        let changed: Vec<usize> = (0..l.lit_at.len())
            .filter(|i| l.lit_at[*i] != before[*i])
            .collect();
        assert_eq!(
            changed,
            vec![8 * 40 + 2, 8 * 40 + 3, 8 * 40 + 4, 8 * 40 + 5]
        );
        l.relight_all();
        assert!(l.lit_at.iter().all(|v| *v == l.frame));
    }

    /// Composition: with the pool emptied and no sweep due, an unlit content
    /// cell is dark; under `L` every tile cell is the mold and a head over a
    /// tile prints nothing.
    #[test]
    fn composition_and_lock() {
        let mut l = layer();
        let m = mold(30, 8);
        let _ = run(&mut l, &m, &[], true);
        l.droplets.clear();
        l.spec.density = 0.0;
        l.sweeping = true;
        l.lit_at.fill(NEVER);
        let out = run(&mut l, &m, &[], true);
        for (i, c) in out.content().iter().enumerate() {
            if !l.is_empty(&m.content()[i]) {
                assert_eq!(c.symbol(), " ", "an unlit content cell shows something");
            }
        }
        l.toggle_lock();
        l.spec.density = 0.5;
        let tile = Rect::new(0, 0, 30, 4);
        for _ in 0..40 {
            let mut out = m.clone();
            l.frame(
                &mut out,
                Pins {
                    pinned: &[],
                    tiles: &[tile],
                },
                true,
            );
            for y in 0..4 {
                for x in 0..30 {
                    assert_eq!(
                        out.cell((x, y)).unwrap(),
                        m.cell((x, y)).unwrap(),
                        "rain in a locked tile"
                    );
                }
            }
        }
    }

    #[test]
    fn governor_steps_down_and_recovers_in_seconds() {
        let mut g = Governor::default();
        g.observe(20_000, 0);
        g.observe(20_000, 0);
        assert_eq!(g.step, 1, "two bad seconds step down");
        assert_eq!(g.fps(24), 16);
        for _ in 0..30 {
            g.observe(5_000, 0);
        }
        assert_eq!(g.step, 0, "thirty good seconds step up");
        for _ in 0..12 {
            g.observe(0, 5_000_000);
        }
        assert_eq!(g.step, Governor::MAX_STEP);
        assert!(g.gutters_only());
    }
}
