//! The semantic view tree (§4.6, D32): components describe *what* is shown;
//! the theme's renderer decides *how*.

use std::borrow::Cow;
use std::fmt;

use gridwatch_store::Severity;
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use crate::theme::{GradientId, Role, Theme};

/// A themed text fragment — never a colour (§4.6).
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub role: Role,
    pub text: Cow<'static, str>,
    pub bold: bool,
}

impl Span {
    pub fn new(role: Role, text: impl Into<Cow<'static, str>>) -> Span {
        Span {
            role,
            text: text.into(),
            bold: false,
        }
    }

    pub fn bold(role: Role, text: impl Into<Cow<'static, str>>) -> Span {
        Span {
            role,
            text: text.into(),
            bold: true,
        }
    }
}

pub type Line = Vec<Span>;

/// Layout hint for `View::Stack` children (a serde-able mirror, not ratatui's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Constraint {
    Len(u16),
    Min(u16),
    Fill(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    H,
    V,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColWidth {
    Fixed(u16),
    Elastic,
}

#[derive(Clone, Debug)]
pub struct Column {
    pub title: Cow<'static, str>,
    pub width: ColWidth,
    pub right: bool,
}

#[derive(Clone, Debug)]
pub struct Series {
    pub label: Cow<'static, str>,
    pub gradient: GradientId,
    pub data: Vec<(f64, f64)>,
}

#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub x: (f64, f64),
    pub y: (f64, f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerHint {
    Auto,
    Braille,
    Block,
}

/// Bespoke surfaces (pins limit line, Winamp skin, audio scope): still style
/// only through theme roles, and describe themselves for text dumps.
pub trait Paint {
    fn paint(&self, area: Rect, theme: &Theme, buf: &mut Buffer);
}

/// The eleven-node tree (§4.6, D32/D37). Fractions are 0..=1.
pub enum View {
    /// Nothing (an empty tier slot); renders as blank space.
    Empty,
    Text(Vec<Line>),
    KeyValue(Vec<(Cow<'static, str>, Line, Option<Severity>)>),
    Gauge {
        label: Cow<'static, str>,
        value: f32,
        gradient: GradientId,
        text: Option<Cow<'static, str>>,
    },
    /// One horizontal multi-segment meter (htop's CPU/mem/swap bars); fractions sum ≤ 1 (D37).
    Segmented {
        label: Cow<'static, str>,
        segments: Vec<(Role, f32)>,
        text: Option<Cow<'static, str>>,
    },
    Bars {
        values: Vec<f32>,
        gradient: GradientId,
        labels: Option<Vec<Cow<'static, str>>>,
        peaks: Option<Vec<f32>>,
    },
    Sparkline {
        series: Vec<Option<f32>>,
        gradient: GradientId,
        max: Option<f32>,
    },
    Chart {
        series: Vec<Series>,
        bounds: Bounds,
        marker: MarkerHint,
    },
    Table {
        columns: Vec<Column>,
        rows: Vec<Vec<Line>>,
        selected: Option<usize>,
        sort: Option<(usize, SortDir)>,
        scroll: usize,
    },
    BigNumber {
        text: Cow<'static, str>,
        role: Role,
    },
    Stack {
        dir: Dir,
        children: Vec<(Constraint, View)>,
    },
    Custom {
        paint: Box<dyn Paint>,
        describe: Cow<'static, str>,
    },
}

impl fmt::Debug for View {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            View::Empty => f.write_str("Empty"),
            View::Text(lines) => write!(f, "Text({} lines)", lines.len()),
            View::KeyValue(rows) => write!(f, "KeyValue({} rows)", rows.len()),
            View::Gauge { label, value, .. } => write!(f, "Gauge({label}={value})"),
            View::Segmented {
                label, segments, ..
            } => write!(f, "Segmented({label}, {} segs)", segments.len()),
            View::Bars { values, .. } => write!(f, "Bars({})", values.len()),
            View::Sparkline { series, .. } => write!(f, "Sparkline({})", series.len()),
            View::Chart { series, .. } => write!(f, "Chart({} series)", series.len()),
            View::Table { rows, .. } => write!(f, "Table({} rows)", rows.len()),
            View::BigNumber { text, .. } => write!(f, "BigNumber({text})"),
            View::Stack { dir: d, children } => {
                write!(f, "Stack({d:?}, {} children)", children.len())
            }
            View::Custom { describe, .. } => write!(f, "Custom({describe})"),
        }
    }
}

/// The theme-parameterised renderer seam (§4.6, D32).
pub trait Renderer {
    fn render(&self, view: &View, area: Rect, theme: &Theme, buf: &mut Buffer);
}

/// Stable fingerprint of a view tree — the render-cache key's backstop term
/// (§5): whatever the snapshot serialisation pins, the cache invalidates on.
/// Called once per visible tile per frame; trees are small (a hand-rolled
/// walker replaces the serialisation if a perf row ever objects).
pub fn fingerprint(v: &View) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    crate::dump::view_value(v).to_string().hash(&mut h);
    h.finish()
}
