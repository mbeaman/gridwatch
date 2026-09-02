//! gridwatch-ui: contracts, view tree, layout engine, themes, default renderer
//! (ARCHITECTURE §4.6–§7). ratatui-core/-widgets only — no crossterm.

#![forbid(unsafe_code)]

pub mod component;
pub mod dump;
pub mod halfblock;
pub mod layout;
pub mod overlay;
pub mod renderer;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
pub mod theme;
pub mod view;

pub use component::{
    Action, BuildCx, BuildError, Chrome, Command, Component, ComponentDef, Footprint, HERO,
    InputCx, KeyHint, Manifest, Outcome, PANEL, Redraw, RedrawPolicy, Registry, RenderCx, Size,
    TILE, TickCx, Tier, WIDE, pick_tier,
};
pub use theme::{ColorMode, GradientId, PerfClass, Role, Theme, ThemeError, load_builtin};
pub use view::{
    Bounds, ColWidth, Column, Constraint, Dir, Line, MarkerHint, Paint, Renderer, Series, SortDir,
    Span, View,
};
