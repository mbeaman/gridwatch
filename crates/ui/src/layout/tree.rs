//! View-tree layout (§4.6, D65 §10): the split `renderer::stack` computes,
//! exposed so a test can measure **one drawing** rather than a whole tier.
//!
//! `assert_grows_with_area` counts a whole tier's cells, so a capped drawing
//! that is a small share of that tier's ink hides inside it — which is how
//! D62 passed `pins` and `gpu` while both were still broken. The unit that
//! answers "does this drawing use the room it was given" is the leaf, and the
//! leaf's rect is what `stack` already works out. Both live here so there is
//! one implementation of the arithmetic: `renderer::stack` calls `split`, and
//! so does `leaves`.

use ratatui_core::layout::Rect;

use crate::view::{Constraint, Dir, View};

/// The sizes a `View::Stack` hands its children along one axis: `Len`/`Min`
/// take their own size (clamped to what is left), then every `Fill` shares
/// the remainder by weight, the last taking the rounding.
pub fn split(constraints: &[Constraint], total: u16) -> Vec<u16> {
    let mut sizes = vec![0u16; constraints.len()];
    let mut used = 0u16;
    let mut fill_weight = 0u16;
    for (i, c) in constraints.iter().enumerate() {
        match c {
            Constraint::Len(l) | Constraint::Min(l) => {
                sizes[i] = (*l).min(total.saturating_sub(used));
                used += sizes[i];
            }
            Constraint::Fill(w) => fill_weight += *w.max(&1),
        }
    }
    let mut remaining = total.saturating_sub(used);
    for (i, c) in constraints.iter().enumerate() {
        if let Constraint::Fill(w) = c {
            let share = (remaining * w.max(&1))
                .checked_div(fill_weight)
                .unwrap_or(0);
            sizes[i] = share;
            fill_weight -= w.max(&1);
            remaining -= share;
        }
    }
    sizes
}

/// `split` as rects: one per child, laid out along `dir` inside `area`.
pub fn split_rects(dir: Dir, constraints: &[Constraint], area: Rect) -> Vec<Rect> {
    let total = if dir == Dir::V {
        area.height
    } else {
        area.width
    };
    let mut offset = 0u16;
    split(constraints, total)
        .into_iter()
        .map(|size| {
            let r = if dir == Dir::V {
                Rect {
                    x: area.x,
                    y: area.y + offset,
                    width: area.width,
                    height: size,
                }
            } else {
                Rect {
                    x: area.x + offset,
                    y: area.y,
                    width: size,
                    height: area.height,
                }
            };
            offset += size;
            r
        })
        .collect()
}

/// One drawable node of a view tree, with the rect it is given and whether
/// each axis of that rect **grows when the tile does**.
///
/// The axis rule is the one §4.6 states, read per axis: a `Len` (or `Min`)
/// anywhere in the chain pins that axis at a component's constant, and
/// nothing below can un-pin it — a `Fill(1)` inside a `Len(3)` band is three
/// rows tall on every terminal. So `fill_h` is true exactly when no vertical
/// `Len` stands between this leaf and the root, and `fill_w` likewise for
/// horizontal ones. The root leaf has both, because its rect *is* the tile's.
/// (D65 §10 says "the chain contains at least one `Fill`"; that phrasing
/// exempts a leaf whose only chain is inheritance — a whole-tier drawing —
/// and marks a `Fill` under a `Len` as growable, which it is not. This is the
/// same rule stated so it is right on both edges.)
#[derive(Clone, Copy, Debug)]
pub struct Leaf<'a> {
    pub view: &'a View,
    pub area: Rect,
    /// The width grows with the tile: no horizontal `Len` in the chain.
    pub fill_w: bool,
    /// The height grows with the tile: no vertical `Len` in the chain.
    pub fill_h: bool,
}

/// Every non-`Stack` node of `view`, with the rect `renderer::stack` would
/// give it inside `area`. Children `stack` would skip (a zero size) are
/// skipped here too, so a leaf's rect is always one the renderer draws into.
pub fn leaves(view: &View, area: Rect) -> Vec<Leaf<'_>> {
    let mut out = Vec::new();
    walk(view, area, true, true, &mut out);
    out
}

fn walk<'a>(view: &'a View, area: Rect, fill_w: bool, fill_h: bool, out: &mut Vec<Leaf<'a>>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    match view {
        View::Stack { dir, children } => {
            let constraints: Vec<Constraint> = children.iter().map(|(c, _)| *c).collect();
            let rects = split_rects(*dir, &constraints, area);
            for ((c, child), rect) in children.iter().zip(rects) {
                let pinned = matches!(c, Constraint::Len(_) | Constraint::Min(_));
                let (w, h) = match dir {
                    Dir::H => (fill_w && !pinned, fill_h),
                    Dir::V => (fill_w, fill_h && !pinned),
                };
                walk(child, rect, w, h, out);
            }
        }
        _ => out.push(Leaf {
            view,
            area,
            fill_w,
            fill_h,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::GradientId;

    fn spark() -> View {
        View::Sparkline {
            series: vec![Some(1.0)],
            gradient: GradientId::Load,
            max: None,
        }
    }

    fn r(w: u16, h: u16) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        }
    }

    #[test]
    fn a_len_band_pins_its_axis_and_a_fill_under_it_cannot_unpin_it() {
        let v = View::Stack {
            dir: Dir::V,
            children: vec![
                (Constraint::Len(1), View::Empty),
                (
                    Constraint::Len(3),
                    View::Stack {
                        dir: Dir::V,
                        children: vec![(Constraint::Fill(1), spark())],
                    },
                ),
                (Constraint::Fill(1), spark()),
            ],
        };
        let ls = leaves(&v, r(20, 10));
        assert_eq!(ls.len(), 3);
        // The `Fill` inside the `Len(3)`: three rows on every terminal.
        assert!(ls[1].fill_w && !ls[1].fill_h);
        assert_eq!(
            ls[1].area,
            Rect {
                x: 0,
                y: 1,
                width: 20,
                height: 3
            }
        );
        // The tier's own `Fill` band: six rows here, more on a taller tile.
        assert!(ls[2].fill_w && ls[2].fill_h);
        assert_eq!(ls[2].area.height, 6);
    }

    #[test]
    fn a_horizontal_len_pins_the_width_only() {
        let v = View::Stack {
            dir: Dir::H,
            children: vec![
                (Constraint::Len(8), spark()),
                (Constraint::Fill(1), spark()),
            ],
        };
        let ls = leaves(&v, r(40, 6));
        assert!(!ls[0].fill_w && ls[0].fill_h);
        assert_eq!(ls[0].area.width, 8);
        assert!(ls[1].fill_w && ls[1].fill_h);
        assert_eq!(ls[1].area.width, 32);
    }

    #[test]
    fn a_child_the_renderer_skips_is_not_a_leaf() {
        let v = View::Stack {
            dir: Dir::V,
            children: vec![
                (Constraint::Len(4), spark()),
                (Constraint::Fill(1), spark()),
            ],
        };
        assert_eq!(leaves(&v, r(10, 4)).len(), 1);
    }

    #[test]
    fn the_root_is_its_own_leaf_and_grows_on_both_axes() {
        let v = spark();
        let ls = leaves(&v, r(12, 5));
        assert_eq!(ls.len(), 1);
        assert!(ls[0].fill_w && ls[0].fill_h);
        assert_eq!(ls[0].area, r(12, 5));
    }
}
