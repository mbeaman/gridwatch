//! The grid layout engine (§6): a pure integer track function — no solver,
//! deterministic, invertible for the mouse.

mod edit;
mod grid;
mod page;

pub use edit::{EditError, insert_first_fit, move_by, remove, resize_by, swap};
// `unit_at`, `unit_rect`, `unit_tracks`, `footprint_cycle` are defined below.
pub use grid::{BorderMode, GridSpec, thresholds, tracks};
pub use page::{Page, PlaceTarget, Placement};

use ratatui_core::layout::Rect;

use crate::component::Size;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolveMode {
    Configured,
    Dense,
    Stack,
}

/// Mode is derived from the terminal size with 2-cell hysteresis on the way
/// back up; a starved cell never changes the mode (§6).
pub fn derive_mode(term: Size, spec: &GridSpec, chrome_rows: u16, prev: SolveMode) -> SolveMode {
    let (configured, dense) = thresholds(spec, chrome_rows);
    let hyst: u16 = 2;
    let fits = |need: Size, pad: u16| {
        term.w >= need.w.saturating_add(pad) && term.h >= need.h.saturating_add(pad)
    };
    match prev {
        SolveMode::Configured => {
            if fits(configured, 0) {
                SolveMode::Configured
            } else if fits(dense, 0) {
                SolveMode::Dense
            } else {
                SolveMode::Stack
            }
        }
        SolveMode::Dense => {
            if fits(configured, hyst) {
                SolveMode::Configured
            } else if fits(dense, 0) {
                SolveMode::Dense
            } else {
                SolveMode::Stack
            }
        }
        SolveMode::Stack => {
            if fits(configured, hyst) {
                SolveMode::Configured
            } else if fits(dense, hyst) {
                SolveMode::Dense
            } else {
                SolveMode::Stack
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Cell {
    /// Index into the page's placements.
    pub index: usize,
    pub outer: Rect,
    pub inner: Rect,
    /// True when the inner rect is starved below the grid's minimum unit — a
    /// defensive fallback that renders as a chip (§6).
    pub chip: bool,
}

#[derive(Clone, Debug)]
pub struct Solved {
    pub mode: SolveMode,
    pub cells: Vec<Cell>,
}

/// Map placements to rects (§6). `zoom` gives one placement the whole body;
/// `scroll` applies in Stack mode.
pub fn solve(
    spec: &GridSpec,
    page: &Page,
    body: Rect,
    mode: SolveMode,
    zoom: Option<usize>,
    scroll: u16,
) -> Solved {
    let min = spec.min_unit_inner;
    if let Some(zi) = zoom {
        let inner = shrink(body, 1);
        return Solved {
            mode,
            cells: page
                .place
                .iter()
                .enumerate()
                .filter(|(i, _)| *i == zi)
                .map(|(i, _)| Cell {
                    index: i,
                    outer: body,
                    inner,
                    chip: false,
                })
                .collect(),
        };
    }
    match mode {
        SolveMode::Stack => {
            let mut order: Vec<usize> = (0..page.place.len()).collect();
            order.sort_by_key(|i| std::cmp::Reverse(page.place[*i].priority));
            let tile_h = min.h + 2;
            let mut cells = Vec::new();
            let mut y = body.y as i32 - i32::from(scroll);
            for index in order {
                let outer = Rect {
                    x: body.x,
                    y: 0,
                    width: body.width,
                    height: tile_h,
                };
                if y + i32::from(tile_h) > i32::from(body.y) && y < i32::from(body.y + body.height)
                {
                    let vis_y = y.max(i32::from(body.y)) as u16;
                    let vis_h = ((y + i32::from(tile_h)).min(i32::from(body.y + body.height))
                        - i32::from(vis_y))
                    .max(0) as u16;
                    if vis_h >= 3 {
                        let outer = Rect {
                            y: vis_y,
                            height: vis_h,
                            ..outer
                        };
                        let inner = shrink(outer, 1);
                        cells.push(Cell {
                            index,
                            outer,
                            inner,
                            chip: false,
                        });
                    }
                }
                y += i32::from(tile_h);
            }
            Solved { mode, cells }
        }
        SolveMode::Configured | SolveMode::Dense => {
            let gap = if mode == SolveMode::Dense {
                0
            } else {
                u16::from(spec.gap)
            };
            let overlap = u16::from(mode == SolveMode::Dense);
            let cols = tracks_overlap(body.width, spec.columns, gap, overlap);
            let rows = tracks_overlap(body.height, spec.rows, gap, overlap);
            let cells = page
                .place
                .iter()
                .enumerate()
                .filter_map(|(index, p)| {
                    let (cx, cy) = (usize::from(p.at.0), usize::from(p.at.1));
                    let (cw, ch) = (usize::from(p.size.0), usize::from(p.size.1));
                    if cx + cw > cols.len() || cy + ch > rows.len() || cw == 0 || ch == 0 {
                        return None;
                    }
                    let x0 = cols[cx].0;
                    let y0 = rows[cy].0;
                    let x1 = cols[cx + cw - 1].0 + cols[cx + cw - 1].1;
                    let y1 = rows[cy + ch - 1].0 + rows[cy + ch - 1].1;
                    let outer = Rect {
                        x: body.x + x0,
                        y: body.y + y0,
                        width: x1 - x0,
                        height: y1 - y0,
                    };
                    let inner = shrink(outer, 1);
                    let chip = inner.width < min.w || inner.height < min.h;
                    Some(Cell {
                        index,
                        outer,
                        inner,
                        chip,
                    })
                })
                .collect();
            Solved { mode, cells }
        }
    }
}

fn shrink(r: Rect, by: u16) -> Rect {
    Rect {
        x: r.x + by.min(r.width / 2),
        y: r.y + by.min(r.height / 2),
        width: r.width.saturating_sub(by * 2),
        height: r.height.saturating_sub(by * 2),
    }
}

/// Like `tracks` but supports the dense mode's one-cell border overlap.
fn tracks_overlap(len: u16, n: u8, gap: u16, overlap: u16) -> Vec<(u16, u16)> {
    if overlap == 0 {
        return tracks(len, n, gap);
    }
    // Shared borders: allocate len + (n-1) then pull each start back by its index.
    let raw = tracks(len + (u16::from(n).saturating_sub(1)) * overlap, n, 0);
    raw.into_iter()
        .enumerate()
        .map(|(i, (s, w))| (s - i as u16 * overlap, w))
        .collect()
}

/// `(start, size)` per track (§6).
pub type Tracks = Vec<(u16, u16)>;

/// The column and row tracks the solver uses for a body in a mode (§6):
/// the inverse of `solve` for the mouse and the edit-mode ghost.
pub fn unit_tracks(spec: &GridSpec, body: Rect, mode: SolveMode) -> (Tracks, Tracks) {
    let gap = if mode == SolveMode::Dense {
        0
    } else {
        u16::from(spec.gap)
    };
    let overlap = u16::from(mode == SolveMode::Dense);
    (
        tracks_overlap(body.width, spec.columns, gap, overlap),
        tracks_overlap(body.height, spec.rows, gap, overlap),
    )
}

/// The grid unit under a screen position (§6, brief arc 4 seam 3): the
/// inverse of `tracks`. A position in a gutter maps to the nearest track on
/// its left/top so a drag never loses the pointer; outside the body → None.
/// Stack mode has no units.
pub fn unit_at(spec: &GridSpec, body: Rect, mode: SolveMode, x: u16, y: u16) -> Option<(u8, u8)> {
    if mode == SolveMode::Stack
        || x < body.x
        || y < body.y
        || x >= body.x + body.width
        || y >= body.y + body.height
    {
        return None;
    }
    let (cols, rows) = unit_tracks(spec, body, mode);
    // The first track that contains `v` wins (in dense mode a shared border
    // column belongs to the tile on its left, as `solve` drew it); a gutter
    // cell falls back to the track on its left.
    let find = |tracks: &[(u16, u16)], v: u16| -> Option<u8> {
        if let Some(i) = tracks.iter().position(|(s, w)| v >= *s && v < s + w) {
            return Some(i as u8);
        }
        tracks.iter().rposition(|(s, _)| v >= *s).map(|i| i as u8)
    };
    Some((find(&cols, x - body.x)?, find(&rows, y - body.y)?))
}

/// The outer rect a placement at `at` with `size` would occupy — the ghost
/// edit mode draws (§10). `None` when it does not fit the tracks.
pub fn unit_rect(
    spec: &GridSpec,
    body: Rect,
    mode: SolveMode,
    at: (u8, u8),
    size: (u8, u8),
) -> Option<Rect> {
    if mode == SolveMode::Stack || size.0 == 0 || size.1 == 0 {
        return None;
    }
    let (cols, rows) = unit_tracks(spec, body, mode);
    let (cx, cy) = (usize::from(at.0), usize::from(at.1));
    let (cw, ch) = (usize::from(size.0), usize::from(size.1));
    if cx + cw > cols.len() || cy + ch > rows.len() {
        return None;
    }
    let x0 = cols[cx].0;
    let y0 = rows[cy].0;
    let x1 = cols[cx + cw - 1].0 + cols[cx + cw - 1].1;
    let y1 = rows[cy + ch - 1].0 + rows[cy + ch - 1].1;
    Some(Rect {
        x: body.x + x0,
        y: body.y + y0,
        width: x1 - x0,
        height: y1 - y0,
    })
}

/// The next footprint after `current` in a manifest's list (`s` in edit
/// mode); the first when `current` is not listed; `None` for an empty list.
pub fn footprint_cycle(footprints: &[(u8, u8)], current: (u8, u8)) -> Option<(u8, u8)> {
    if footprints.is_empty() {
        return None;
    }
    let i = footprints.iter().position(|f| *f == current);
    Some(match i {
        Some(i) => footprints[(i + 1) % footprints.len()],
        None => footprints[0],
    })
}

/// Which placement contains a position (mouse → grid, §6).
pub fn hit(s: &Solved, x: u16, y: u16) -> Option<usize> {
    s.cells
        .iter()
        .find(|c| {
            x >= c.outer.x
                && x < c.outer.x + c.outer.width
                && y >= c.outer.y
                && y < c.outer.y + c.outer.height
        })
        .map(|c| c.index)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// Spatial focus: nearest cell centre in the given direction (§10).
pub fn focus_dir(s: &Solved, current: usize, dir: Direction) -> Option<usize> {
    let cur = s.cells.iter().find(|c| c.index == current)?;
    let (cx, cy) = centre(cur.outer);
    s.cells
        .iter()
        .filter(|c| c.index != current)
        .filter(|c| {
            let (x, y) = centre(c.outer);
            match dir {
                Direction::Up => y < cy,
                Direction::Down => y > cy,
                Direction::Left => x < cx,
                Direction::Right => x > cx,
            }
        })
        .min_by_key(|c| {
            let (x, y) = centre(c.outer);
            let dx = i32::from(x) - i32::from(cx);
            let dy = i32::from(y) - i32::from(cy);
            // Weight the off-axis distance so straight neighbours win.
            match dir {
                Direction::Up | Direction::Down => dy.abs() + dx.abs() * 3,
                Direction::Left | Direction::Right => dx.abs() + dy.abs() * 3,
            }
        })
        .map(|c| c.index)
}

fn centre(r: Rect) -> (u16, u16) {
    (r.x + r.width / 2, r.y + r.height / 2)
}
