//! The theme system (§7): semantic roles, Oklab gradient LUTs, glyph tiers,
//! border/title/widget specs. Components never see a colour literal.

mod color;
mod file;
mod gradient;

pub use color::{
    ColorMode, contrast_ratio, nearest_16, nearest_256, parse_color, relative_luminance,
};
pub use file::{
    BUILTIN_THEMES, ThemeFile, WCAG_MUTED_MIN, WCAG_TEXT_MIN, build_theme, builtin,
    contrast_report, load_builtin, load_theme_file, merge, wcag_warnings,
};
pub use gradient::Gradient;

use ratatui_core::style::{Color, Modifier, Style};
use ratatui_widgets::block::Block;
use ratatui_widgets::borders::{BorderType, Borders as BorderFlags};

use gridwatch_store::Severity;

use crate::component::Chrome;
use crate::view::{Renderer, Span};

/// The 19 semantic colour roles (§7) — plain arrays, no enum-map (D11).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Bg,
    Surface,
    Panel,
    Border,
    BorderFocused,
    Title,
    Text,
    TextMuted,
    TextGhost,
    AccentPrimary,
    AccentSecondary,
    AccentTertiary,
    Ok,
    Warn,
    Crit,
    Info,
    SelectionFg,
    SelectionBg,
    Cursor,
}

pub const ROLES: [Role; 19] = [
    Role::Bg,
    Role::Surface,
    Role::Panel,
    Role::Border,
    Role::BorderFocused,
    Role::Title,
    Role::Text,
    Role::TextMuted,
    Role::TextGhost,
    Role::AccentPrimary,
    Role::AccentSecondary,
    Role::AccentTertiary,
    Role::Ok,
    Role::Warn,
    Role::Crit,
    Role::Info,
    Role::SelectionFg,
    Role::SelectionBg,
    Role::Cursor,
];

impl Role {
    pub fn index(self) -> usize {
        ROLES.iter().position(|r| *r == self).expect("role listed")
    }
}

/// The 8 named gradients (§7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientId {
    Load,
    Temp,
    Power,
    Mem,
    NetRx,
    NetTx,
    Audio,
    Title,
}

pub const GRADIENTS: [GradientId; 8] = [
    GradientId::Load,
    GradientId::Temp,
    GradientId::Power,
    GradientId::Mem,
    GradientId::NetRx,
    GradientId::NetTx,
    GradientId::Audio,
    GradientId::Title,
];

impl GradientId {
    pub fn index(self) -> usize {
        GRADIENTS
            .iter()
            .position(|g| *g == self)
            .expect("gradient listed")
    }

    pub fn name(self) -> &'static str {
        match self {
            GradientId::Load => "load",
            GradientId::Temp => "temp",
            GradientId::Power => "power",
            GradientId::Mem => "mem",
            GradientId::NetRx => "netrx",
            GradientId::NetTx => "nettx",
            GradientId::Audio => "audio",
            GradientId::Title => "title",
        }
    }
}

/// Showcase themes may spend budget on ambience while focused (§7, D28).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PerfClass {
    #[default]
    Quiet,
    Showcase,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlyphTier {
    Ascii,
    #[default]
    Unicode,
    Nerd,
}

/// How `View::Chart` draws a line (§7 `[glyphs] chart_marker`): braille dots
/// (2×4 per cell, the default on the unicode tier), lower-eighth block
/// columns, or a plain dot per point. The ascii tier always falls back to `*`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChartMarker {
    #[default]
    Braille,
    Block,
    Dot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphSet {
    pub tier: GlyphTier,
    pub marker: ChartMarker,
}

impl GlyphSet {
    /// Lower-eighths for bars/sparklines; ASCII fallback per tier.
    pub fn eighths(&self) -> [char; 8] {
        match self.tier {
            GlyphTier::Ascii => [' ', '.', '.', ':', ':', '|', '|', '#'],
            _ => ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'],
        }
    }

    pub fn full(&self) -> char {
        match self.tier {
            GlyphTier::Ascii => '#',
            _ => '█',
        }
    }

    pub fn partial_h(&self, frac8: usize) -> char {
        // Left-partial for horizontal bars.
        const H: [char; 8] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
        match self.tier {
            GlyphTier::Ascii => {
                if frac8 >= 4 {
                    '#'
                } else {
                    '-'
                }
            }
            _ => H[frac8.min(7)],
        }
    }

    pub fn empty(&self) -> char {
        match self.tier {
            GlyphTier::Ascii => '.',
            _ => '░',
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BorderKind {
    Plain,
    #[default]
    Rounded,
    Double,
    Thick,
}

impl BorderKind {
    pub fn to_border_type(self) -> BorderType {
        match self {
            BorderKind::Plain => BorderType::Plain,
            BorderKind::Rounded => BorderType::Rounded,
            BorderKind::Double => BorderType::Double,
            BorderKind::Thick => BorderType::Thick,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BorderSpec {
    pub set: BorderKind,
    pub focused: BorderKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TitleStyle {
    #[default]
    Plain,
    Gradient,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TitleSpec {
    pub style: TitleStyle,
    pub upper: bool,
    pub bold: bool,
}

impl Default for TitleSpec {
    fn default() -> TitleSpec {
        TitleSpec {
            style: TitleStyle::Plain,
            upper: false,
            bold: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GaugeStyle {
    #[default]
    Bar,
    Line,
    Block,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BarStyle {
    #[default]
    Eighths,
    Shade,
    Dots,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HeaderStyle {
    Underline,
    #[default]
    Reverse,
    Plain,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PixelStyle {
    #[default]
    Quadrant,
    Sextant,
    Full,
}

/// Per-theme widget-form choices (§7, D32): themes own form, not just paint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WidgetSet {
    pub gauge: GaugeStyle,
    pub bars: BarStyle,
    pub sparkline: BarStyle,
    pub table_header: HeaderStyle,
    pub big_number: PixelStyle,
}

#[derive(Debug)]
pub struct ThemeError(pub String);

impl std::fmt::Display for ThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "theme: {}", self.0)
    }
}

impl std::error::Error for ThemeError {}

#[derive(Clone)]
pub struct Theme {
    pub name: String,
    pub class: PerfClass,
    pub mode: ColorMode,
    pub glyphs: GlyphSet,
    pub borders: BorderSpec,
    pub title: TitleSpec,
    pub widgets: WidgetSet,
    colors: [Color; 19],
    gradients: [Gradient; 8],
    /// Warnings produced at load (ignored tables, the WCAG gate, etc.).
    pub warnings: Vec<String>,
    /// Per-kind derived themes from `[components.<kind>]` (D52), built once
    /// at load and shared; empty for a derived theme itself.
    kinds: std::collections::BTreeMap<String, std::sync::Arc<Theme>>,
    /// The declared (pre-mode) colours the contrast report judges.
    raw: [Color; 19],
}

impl Theme {
    #[allow(clippy::too_many_arguments)] // assembled once by the theme-file loader
    pub fn from_parts(
        name: String,
        class: PerfClass,
        mode: ColorMode,
        colors: [Color; 19],
        gradients: [Gradient; 8],
        glyphs: GlyphSet,
        borders: BorderSpec,
        title: TitleSpec,
        widgets: WidgetSet,
        warnings: Vec<String>,
    ) -> Theme {
        Theme {
            name,
            class,
            mode,
            glyphs,
            borders,
            title,
            widgets,
            colors,
            gradients,
            warnings,
            kinds: std::collections::BTreeMap::new(),
            raw: colors,
        }
    }

    pub fn with_kinds(
        mut self,
        kinds: std::collections::BTreeMap<String, std::sync::Arc<Theme>>,
    ) -> Theme {
        self.kinds = kinds;
        self
    }

    pub fn with_raw_colors(mut self, raw: [Color; 19]) -> Theme {
        self.raw = raw;
        self
    }

    /// The theme a component kind renders with (§7, D52): the base unless
    /// `[components.<kind>]` overrode a gradient for it.
    pub fn for_kind(&self, kind: &str) -> &Theme {
        self.kinds.get(kind).map(|t| &**t).unwrap_or(self)
    }

    /// The kinds that have a derived theme.
    pub fn overridden_kinds(&self) -> impl Iterator<Item = &str> {
        self.kinds.keys().map(String::as_str)
    }

    /// Every WCAG pair with its ratio (`config check --theme`).
    pub fn contrast_report(&self) -> Vec<String> {
        contrast_report(&self.raw)
    }

    pub fn color(&self, r: Role) -> Color {
        self.colors[r.index()]
    }

    pub fn style(&self, r: Role) -> Style {
        Style::new().fg(self.color(r))
    }

    pub fn span_style(&self, s: &Span) -> Style {
        let mut st = self.style(s.role);
        if s.bold {
            st = st.add_modifier(Modifier::BOLD);
        }
        st
    }

    pub fn gradient(&self, g: GradientId) -> &Gradient {
        &self.gradients[g.index()]
    }

    /// Colour + glyph for a severity; Crit adds BOLD|REVERSED so it survives
    /// mono / NO_COLOR (§7).
    pub fn severity(&self, s: Severity) -> (Style, &'static str) {
        match s {
            Severity::Info => (self.style(Role::Info), "ℹ"),
            Severity::Warn => (self.style(Role::Warn).add_modifier(Modifier::BOLD), "▲"),
            Severity::Crit => (
                self.style(Role::Crit)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED),
                "‼",
            ),
        }
    }

    /// The tile frame (shell-owned chrome); `None` for Borderless/Custom.
    pub fn block(&self, focused: bool, _dense: bool, chrome: Chrome) -> Option<Block<'static>> {
        match chrome {
            Chrome::Borderless | Chrome::Custom => None,
            Chrome::Themed => {
                let kind = if focused {
                    self.borders.focused
                } else {
                    self.borders.set
                };
                let style = if focused {
                    self.style(Role::BorderFocused)
                } else {
                    self.style(Role::Border)
                };
                Some(
                    Block::new()
                        .borders(BorderFlags::ALL)
                        .border_type(kind.to_border_type())
                        .border_style(style),
                )
            }
        }
    }

    pub fn renderer(&self) -> &dyn Renderer {
        &crate::renderer::DEFAULT_RENDERER
    }
}
