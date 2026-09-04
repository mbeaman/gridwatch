//! The theme system (§7): semantic roles, Oklab gradient LUTs, glyph tiers,
//! border/title/widget specs. Components never see a colour literal.

mod color;
mod file;
mod gradient;

pub use color::{
    ColorMode, contrast_ratio, nearest_16, nearest_256, parse_color, relative_luminance,
};
pub use file::{
    BUILTIN_THEMES, EFFECT_KINDS, ThemeFile, WCAG_MUTED_MIN, WCAG_TEXT_MIN, autofix, build_theme,
    builtin, builtin_file, contrast_report, load_builtin, load_theme_file, merge, wcag_warnings,
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

    /// One fill glyph per segment index, for a meter whose segments cannot be
    /// told apart by colour (arc 10a, D60). Ordered by weight, so a meter
    /// still reads as "how much of this is the first thing" — under `mono`
    /// the MEM bar's used/buffers/shared/cache boundaries had vanished
    /// entirely and it read as one solid bar, far fuller than it was.
    pub fn segment(&self, i: usize) -> char {
        const UNICODE: [char; 7] = ['█', '▓', '▒', '░', '▄', '▀', '▌'];
        const ASCII: [char; 7] = ['#', '=', '*', '-', '+', ':', '.'];
        match self.tier {
            GlyphTier::Ascii => ASCII[i.min(ASCII.len() - 1)],
            _ => UNICODE[i.min(UNICODE.len() - 1)],
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

/// How a multi-segment meter tells its segments apart (arc 10a, D60).
///
/// The default is `Auto`, and it has to be: `ColorMode` can drop *any* theme
/// to monochrome at runtime (`--color mono`, `NO_COLOR`, a 16-colour
/// terminal), so a static `[widgets]` key cannot know whether this frame will
/// have colour to give.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SegmentedStyle {
    /// Glyphs when the resolved theme has no colour to give, one fill
    /// otherwise.
    #[default]
    Auto,
    /// One fill glyph; the segments are told apart by colour alone.
    Bar,
    /// A distinct glyph per segment, always.
    Glyphs,
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
    pub segmented: SegmentedStyle,
    pub table_header: HeaderStyle,
    pub big_number: PixelStyle,
}

/// One `[effects]` hook (§7, arc 4b — D54 seam 6): data only; the app maps
/// `kind` to a tachyonfx effect. Unknown kinds warn at load and are ignored.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectSpec {
    pub kind: String,
    pub duration_ms: u32,
    pub motion: Option<String>,
    pub lightness: Option<f32>,
    pub period_ms: Option<u32>,
    pub target: Option<String>,
}

/// The hooks a theme may declare (§7): each `None` plays nothing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectHooks {
    pub startup: Option<EffectSpec>,
    pub theme_swap: Option<EffectSpec>,
    pub focus: Option<EffectSpec>,
    pub alert: Option<EffectSpec>,
}

impl EffectHooks {
    pub fn any(&self) -> bool {
        self.startup.is_some()
            || self.theme_swap.is_some()
            || self.focus.is_some()
            || self.alert.is_some()
    }
}

/// `[flourish]` (§7, seam 7): retro decorations a theme opts into.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flourish {
    pub grid_floor: bool,
    pub sun: bool,
    /// The clock's pixel style, overriding `[widgets] big_number` for it.
    pub big_clock: Option<PixelStyle>,
    pub marquee: bool,
}

/// `[ambient.light]` (D31/D34): how printed content and empty-space trails
/// fade, in the ambient layer's own units.
#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    pub fade_s: f32,
    pub trail_ms: u32,
    pub sweep_s: f32,
    pub head: Color,
    pub floor: Color,
    pub relight_on_update: bool,
}

impl Default for Light {
    fn default() -> Light {
        Light {
            fade_s: 12.0,
            trail_ms: 900,
            sweep_s: 20.0,
            head: Color::Rgb(255, 255, 255),
            floor: Color::Rgb(0, 0, 0),
            relight_on_update: true,
        }
    }
}

/// `[ambient]` (§7, D28/D31/D34, seam 10): a showcase theme's layer — data
/// here, the painter in `gridwatch-app::ambient`.
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientSpec {
    pub kind: String,
    pub fps: u8,
    pub density: f32,
    pub speed: f32,
    pub reveal: Vec<String>,
    pub reveal_ms: u32,
    pub governor: bool,
    pub light: Light,
}

/// The glyph set the rain prints (`[glyphs] rain`): half-width katakana
/// (East Asian Width `H`, one cell in VTE) or plain ASCII.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RainGlyphs {
    #[default]
    Katakana,
    Ascii,
}

impl RainGlyphs {
    /// The characters a droplet may print, all one cell wide.
    pub fn chars(self) -> &'static [char] {
        const KATAKANA: &[char] = &[
            'ｦ', 'ｧ', 'ｨ', 'ｩ', 'ｪ', 'ｫ', 'ｬ', 'ｭ', 'ｮ', 'ｯ', 'ｰ', 'ｱ', 'ｲ', 'ｳ', 'ｴ', 'ｵ', 'ｶ',
            'ｷ', 'ｸ', 'ｹ', 'ｺ', 'ｻ', 'ｼ', 'ｽ', 'ｾ', 'ｿ', 'ﾀ', 'ﾁ', 'ﾂ', 'ﾃ', 'ﾄ', 'ﾅ', 'ﾆ', 'ﾇ',
            'ﾈ', 'ﾉ', 'ﾊ', 'ﾋ', 'ﾌ', 'ﾍ', 'ﾎ', 'ﾏ', 'ﾐ', 'ﾑ', 'ﾒ', 'ﾓ', 'ﾔ', 'ﾕ', 'ﾖ', 'ﾗ', 'ﾘ',
            'ﾙ', 'ﾚ', 'ﾛ', 'ﾜ', 'ﾝ', '･', ':', '"', '=', '*', '+', '<', '>', '¦', '|',
        ];
        // No digits in either set: a `5` beside a value reads as data
        // (review).
        const ASCII: &[char] = &[
            'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q',
            'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', ':', '.', '=', '*', '+', '<', '>', '|',
        ];
        match self {
            RainGlyphs::Katakana => KATAKANA,
            RainGlyphs::Ascii => ASCII,
        }
    }
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
    /// `[effects]` hooks (arc 4b); empty for every theme but the ones that
    /// declare them.
    pub effects: EffectHooks,
    /// `[flourish]` (arc 4b).
    pub flourish: Flourish,
    /// `[ambient]` (arc 4b): `Some` only for a showcase theme with a layer.
    pub ambient: Option<AmbientSpec>,
    /// The `rain` gradient (a ninth, optional): the ambient layer's palette.
    pub rain: Option<Gradient>,
    pub rain_glyphs: RainGlyphs,
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
            effects: EffectHooks::default(),
            flourish: Flourish::default(),
            ambient: None,
            rain: None,
            rain_glyphs: RainGlyphs::default(),
            kinds: std::collections::BTreeMap::new(),
            raw: colors,
        }
    }

    /// The 4b tables, set by the loader after `from_parts`.
    pub fn with_ambience(
        mut self,
        effects: EffectHooks,
        flourish: Flourish,
        ambient: Option<AmbientSpec>,
        rain: Option<Gradient>,
        rain_glyphs: RainGlyphs,
    ) -> Theme {
        self.effects = effects;
        self.flourish = flourish;
        self.ambient = ambient;
        self.rain = rain;
        self.rain_glyphs = rain_glyphs;
        self
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

    /// True when this theme has no colour to give: the render mode is
    /// `Mono`, or every accent and severity resolves to the same colour as
    /// the plain text (the `mono` theme paints everything `default`). A
    /// picture then has to be drawn as luminance, not as colour pairs.
    pub fn monochrome(&self) -> bool {
        if self.mode == ColorMode::Mono {
            return true;
        }
        let text = self.color(Role::Text);
        [
            Role::AccentPrimary,
            Role::AccentSecondary,
            Role::AccentTertiary,
            Role::Ok,
            Role::Warn,
            Role::Crit,
        ]
        .iter()
        .all(|r| self.color(*r) == text)
    }

    pub fn style(&self, r: Role) -> Style {
        Style::new().fg(self.color(r))
    }

    /// Whether a `View::Segmented` should draw a glyph per segment. `Auto`
    /// asks `monochrome()`, which is a property of the *resolved* theme —
    /// `--color mono` and `NO_COLOR` reach every theme, not just `mono`.
    pub fn segmented_glyphs(&self) -> bool {
        match self.widgets.segmented {
            SegmentedStyle::Bar => false,
            SegmentedStyle::Glyphs => true,
            SegmentedStyle::Auto => self.monochrome(),
        }
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
