//! Config loading (§9): two files, layered defaults ← file ← `GRIDWATCH_*` env
//! ← CLI; strict parsing with spans; the layout file is the only one edit mode
//! will ever write (arc 4).

use std::path::{Path, PathBuf};
use std::time::Duration;

use gridwatch_store::Retention;
use gridwatch_ui::component::Size;
use gridwatch_ui::layout::{BorderMode, GridSpec, Page, PlaceTarget, Placement};
use serde::Deserialize;

pub const DEFAULT_CONFIG: &str = include_str!("defaults/config.toml");
pub const DEFAULT_LAYOUT: &str = include_str!("defaults/layout.toml");

#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "config: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

/// `byte` → 1-based `(line, col)` in `text` (§9: errors report
/// `file:line:col`, and the reload toast names the line).
pub fn line_col(text: &str, byte: usize) -> (usize, usize) {
    let byte = byte.min(text.len());
    let before = &text[..byte];
    let line = before.matches('\n').count() + 1;
    let col = before
        .rsplit('\n')
        .next()
        .map(|l| l.chars().count())
        .unwrap_or(0)
        + 1;
    (line, col)
}

fn parse<T: serde::de::DeserializeOwned>(what: &str, text: &str) -> Result<T, ConfigError> {
    toml::from_str(text).map_err(|e| {
        let at = e
            .span()
            .map(|s| {
                let (l, c) = line_col(text, s.start);
                format!(":{l}:{c}")
            })
            .unwrap_or_default();
        // toml's message repeats the span as `at line N column M` on its own
        // lines; the first line is the message itself.
        let msg = e.message().to_string();
        ConfigError(format!("{what}{at}: {msg}"))
    })
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ConfigFile {
    pub schema: u32,
    pub theme: String,
    pub fps: u16,
    pub fps_max: u16,
    pub color: String,
    pub mouse: bool,
    pub readonly: bool,
    /// **Retired (D63).** Nothing ever read it: the confirm bar asks whatever
    /// `Action::confirm()` says it must, unconditionally (D58 seam 2), and
    /// `readonly` is the flag that exists. It parses for one more minor so
    /// every existing install's config still loads under
    /// `deny_unknown_fields`; a `Some` is one warning line and one toast.
    pub confirm_kill: Option<bool>,
    pub store: StoreSect,
    pub effects: EffectsSect,
    pub perf: PerfSect,
    #[serde(rename = "sources")]
    pub sources: toml::Table,
    #[serde(rename = "components")]
    pub components: Vec<InstanceSect>,
    /// Not read (D63): recording is `--record FILE` with `--tables on` and
    /// `--record-input`. The loader says so; the section survives one more minor
    /// so a config carrying it still loads.
    pub record: toml::Table,
    /// Parsed and ignored until the rules arc (§9): alert rules.
    pub rules: Vec<toml::Table>,
    /// The `exec` plugins to start (§4.7, arc 8b). **Restart-only**: a hot
    /// reload reports a change here and does not apply it, which is what
    /// bounds the one manifest leaked per plugin to one per process.
    pub plugins: Vec<PluginSect>,
}

impl Default for ConfigFile {
    // Structural, NOT parsed from `DEFAULT_CONFIG`: serde's container-level
    // `default` calls `Self::default()` inside `visit_map`, so a Default that
    // parses TOML recurses forever. A test below pins TOML == this.
    fn default() -> ConfigFile {
        let mut cpu = toml::Table::new();
        cpu.insert("refresh_ms".into(), toml::Value::Integer(1500));
        let mut sources = toml::Table::new();
        sources.insert("cpu".into(), toml::Value::Table(cpu));
        let inst = |id: &str, kind: &str| InstanceSect {
            id: id.into(),
            kind: kind.into(),
            options: toml::Table::new(),
        };
        ConfigFile {
            schema: 1,
            theme: "retrowave".into(),
            fps: 30,
            fps_max: 60,
            color: "auto".into(),
            mouse: true,
            readonly: false,
            confirm_kill: None,
            store: StoreSect::default(),
            effects: EffectsSect::default(),
            perf: PerfSect::default(),
            sources,
            record: toml::Table::new(),
            rules: Vec::new(),
            plugins: Vec::new(),
            components: vec![
                inst("cpu", "htop"),
                inst("gpu", "gpu"),
                inst("pins", "pins"),
                inst("lan", "net"),
                inst("viz", "audio"),
                inst("amp", "winamp"),
                inst("temps", "sensors"),
                // On page 3 rather than the Overview, which is full (D66).
                inst("drives", "disk"),
            ],
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct StoreSect {
    /// How long the store keeps a scalar's points (§4.2, D63): `1m`-`1h`,
    /// shipped `10m`. `max_age` is this, and `max_len` is derived from it.
    pub history: String,
    /// **Retired (D63).** There is no byte accounting in `gridwatch-store`,
    /// and no honest policy for exceeding a cap: shrinking `max_age` would
    /// make a chart's window depend on the machine's series count, and
    /// refusing samples would make the newest data the casualty. What stays
    /// is the measurement without the policy — `Store::footprint()` on the
    /// `F12` HUD and as `store_bytes` in `--stats-log`.
    pub max_mb: Option<u32>,
}

/// The shipped `[store] history`, in one place: the TOML, this structural
/// default and `Retention::default()` must agree, and a test pins all three.
pub const DEFAULT_HISTORY: &str = "10m";

impl Default for StoreSect {
    fn default() -> StoreSect {
        StoreSect {
            history: DEFAULT_HISTORY.into(),
            max_mb: None,
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct EffectsSect {
    pub enabled: bool,
    pub budget_ms: u32,
}

impl Default for EffectsSect {
    fn default() -> EffectsSect {
        EffectsSect {
            enabled: true,
            budget_ms: 4,
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PerfSect {
    pub unfocused_fps: u16,
    /// **Retired (D63).** It named a mechanism that was never built:
    /// `SourceCtx::next_deadline` aligns to multiples of each source's *own*
    /// cadence from the epoch, and there is no 250 ms grid. P5 is met without
    /// one (arc 7a measured 33 wake-ups/s with every source live).
    pub phase_ms: Option<u64>,
}

impl Default for PerfSect {
    fn default() -> PerfSect {
        PerfSect {
            unfocused_fps: 2,
            phase_ms: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceSect {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub options: toml::Table,
}

/// One `[[plugins]]` entry (§4.7, arc 8b). `argv` is a program and its
/// arguments, never a shell string: nothing in a config file is interpreted.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSect {
    pub id: String,
    pub argv: Vec<String>,
    /// `RLIMIT_AS` for the child, in MiB.
    #[serde(default = "default_rss_mb")]
    pub rss_mb: u64,
    /// `RLIMIT_CPU` for the child, in seconds.
    #[serde(default = "default_cpu_secs")]
    pub cpu_secs: u64,
    /// How long startup waits for this plugin's manifest. Every plugin is
    /// spawned before any is waited on, so N plugins cost the longest wait,
    /// not the sum.
    #[serde(default = "default_hello_ms")]
    pub hello_ms: u64,
    /// The floor between two `render` asks for the same unchanged tile.
    #[serde(default = "default_render_ms")]
    pub render_ms: u64,
}

fn default_rss_mb() -> u64 {
    256
}

fn default_cpu_secs() -> u64 {
    600
}

fn default_hello_ms() -> u64 {
    2_000
}

fn default_render_ms() -> u64 {
    1_000
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutFile {
    pub schema: u32,
    pub grid: GridSect,
    pub pages: Vec<PageSect>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct GridSect {
    pub columns: u8,
    pub rows: u8,
    pub gap: u8,
    pub borders: String,
    pub cell_aspect: f32,
    pub min_unit_inner: MinUnit,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinUnit {
    pub cols: u16,
    pub rows: u16,
}

impl Default for GridSect {
    fn default() -> GridSect {
        GridSect {
            columns: 12,
            rows: 6,
            gap: 1,
            borders: "each".into(),
            cell_aspect: 0.5,
            min_unit_inner: MinUnit { cols: 8, rows: 3 },
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageSect {
    pub name: String,
    #[serde(default)]
    pub hotkey: Option<String>,
    pub place: Vec<PlaceSect>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceSect {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    pub at: [u8; 2],
    pub size: [u8; 2],
    #[serde(default)]
    pub view: Option<String>,
    #[serde(default)]
    pub priority: Option<i32>,
}

/// The floor and the ceiling `[store] history` is clamped into (D63). The
/// floor because the sweep runs every `max_age / 10` floored at 10 s, so a
/// sub-minute retention prunes every ring to its newest point before a chart
/// could draw; the ceiling from arithmetic against P17 — at one hour the held
/// points on torch's ≈ 150 series come to ≈ 12 MB and `Ring::new`'s
/// preallocation saturates at 4 096 slots, which lands RSS near 50 of the
/// 60 MB budget.
pub const MIN_HISTORY: Duration = Duration::from_secs(60);
pub const MAX_HISTORY: Duration = Duration::from_secs(3600);

/// `[store] history`: `<n>s`, `<n>m`, `<n>h`, or a compound like `1h30m`.
/// Integer arithmetic only — determinism is config, not clock (D63 trap 1),
/// so this and `Retention::for_history` between them must produce the same
/// numbers on every machine and in every replay.
pub fn parse_history(text: &str) -> Result<Duration, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("is empty — expected a duration like \"10m\" or \"1h30m\"".into());
    }
    let mut total: u64 = 0;
    let mut digits = String::new();
    let mut any = false;
    for c in t.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let unit = match c {
            's' => 1u64,
            'm' => 60,
            'h' => 3600,
            _ => {
                return Err(format!(
                    "is not a duration: '{c}' is not one of s, m, h — \
                     write \"10m\", \"90s\" or \"1h30m\""
                ));
            }
        };
        if digits.is_empty() {
            return Err(format!("is not a duration: '{c}' has no number before it"));
        }
        let n: u64 = digits
            .parse()
            .map_err(|_| format!("is not a duration: {digits} is too large"))?;
        total = total
            .checked_add(n.checked_mul(unit).ok_or("is too long a duration")?)
            .ok_or("is too long a duration")?;
        digits.clear();
        any = true;
    }
    if !digits.is_empty() {
        return Err(format!(
            "is not a duration: {digits} has no unit — write \"{digits}s\", \
             \"{digits}m\" or \"{digits}h\""
        ));
    }
    if !any {
        return Err("is not a duration — expected a number and a unit (s, m, h)".into());
    }
    Ok(Duration::from_secs(total))
}

/// A duration in the words `config.toml` writes, for a message: `600s` reads
/// back as `10m`.
fn say_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s > 0 && s.is_multiple_of(3600) {
        format!("{}h", s / 3600)
    } else if s > 0 && s.is_multiple_of(60) {
        format!("{}m", s / 60)
    } else {
        format!("{s}s")
    }
}

pub struct Loaded {
    pub config: ConfigFile,
    pub grid: GridSpec,
    pub pages: Vec<Page>,
    /// The `[[rules]]` that parsed (arc 7b); the ones that did not are in
    /// `warnings` and `config check` prints them as errors.
    pub rules: Vec<gridwatch_store::rules::Rule>,
    /// `[store] history` resolved (D63): what `Store::new` is built with.
    /// Retention is set once, at start — a reload that changes `[store]`
    /// toasts "restart to apply", as `[sources.*]` does.
    pub retention: Retention,
    pub warnings: Vec<String>,
    pub config_path: Option<PathBuf>,
    pub layout_path: Option<PathBuf>,
}

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|p| p.join("gridwatch"))
}

/// The two file texts (or the embedded defaults) and their paths — what
/// `load` parses, and what a hot reload re-reads (§9).
pub fn read_texts() -> Result<(String, String), ConfigError> {
    let (c, l, _, _) = read_all()?;
    Ok((c, l))
}

type Read = (String, String, Option<PathBuf>, Option<PathBuf>);

fn read_all() -> Result<Read, ConfigError> {
    let dir = config_dir();
    let config_path = dir
        .as_ref()
        .map(|d| d.join("config.toml"))
        .filter(|p| p.exists());
    let layout_path = dir
        .as_ref()
        .map(|d| d.join("layout.toml"))
        .filter(|p| p.exists());
    let config_text = match &config_path {
        Some(p) => {
            std::fs::read_to_string(p).map_err(|e| ConfigError(format!("{}: {e}", p.display())))?
        }
        None => DEFAULT_CONFIG.to_string(),
    };
    let layout_text = match &layout_path {
        Some(p) => {
            std::fs::read_to_string(p).map_err(|e| ConfigError(format!("{}: {e}", p.display())))?
        }
        None => DEFAULT_LAYOUT.to_string(),
    };
    Ok((config_text, layout_text, config_path, layout_path))
}

/// Where `w` writes (§9): `layout.toml` in the config dir, whether or not
/// it exists yet — the only file edit mode ever writes.
pub fn layout_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("layout.toml"))
}

/// The files the watcher stats (§9): `config.toml` and `layout.toml` in the
/// config dir, whether or not they exist yet — a file that appears is a
/// change too.
pub fn watched_paths() -> Vec<PathBuf> {
    config_dir()
        .map(|d| vec![d.join("config.toml"), d.join("layout.toml")])
        .unwrap_or_default()
}

pub fn load() -> Result<Loaded, ConfigError> {
    let (config_text, layout_text, config_path, layout_path) = read_all()?;
    load_from(&config_text, &layout_text, config_path, layout_path, true)
}

/// Embedded defaults only — hermetic and env-free, so `shot --seed N` stays
/// byte-deterministic across machines (§12.5, D41).
pub fn load_embedded() -> Result<Loaded, ConfigError> {
    load_from(DEFAULT_CONFIG, DEFAULT_LAYOUT, None, None, false)
}

/// Load `config.toml` + `layout.toml` from a named directory — what
/// `shot --config DIR` uses. The env layer is off, as it is for every load
/// that is not the live app, so a screenshot says what the files say.
pub fn load_dir(dir: &Path) -> Result<Loaded, ConfigError> {
    let read = |name: &str, fallback: &str| -> Result<(String, Option<PathBuf>), ConfigError> {
        let path = dir.join(name);
        if !path.exists() {
            return Ok((fallback.to_string(), None));
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| ConfigError(format!("{}: {e}", path.display())))?;
        Ok((text, Some(path)))
    };
    let (config_text, config_path) = read("config.toml", DEFAULT_CONFIG)?;
    let (layout_text, layout_path) = read("layout.toml", DEFAULT_LAYOUT)?;
    load_from(&config_text, &layout_text, config_path, layout_path, false)
}

/// Load an explicit pair of documents — how fixtures (and, from arc 3, the hot
/// reload) get a `Loaded` without touching `$XDG_CONFIG_HOME`.
pub fn load_texts(config_text: &str, layout_text: &str) -> Result<Loaded, ConfigError> {
    load_from(config_text, layout_text, None, None, false)
}

fn load_from(
    config_text: &str,
    layout_text: &str,
    config_path: Option<PathBuf>,
    layout_path: Option<PathBuf>,
    env_layer: bool,
) -> Result<Loaded, ConfigError> {
    let mut warnings = Vec::new();
    let mut config: ConfigFile = parse("config.toml", config_text)?;
    let layout: LayoutFile = parse("layout.toml", layout_text)?;
    if !config.record.is_empty() {
        // Eleven arcs of "arrives in arc 2" for a section that was never
        // going to arrive: recording is a flag, not a table (D63 E2).
        warnings.push(
            "[record] is not a section of config.toml — recording is `--record FILE` \
             (with `--tables on` and `--record-input`); the table is ignored"
                .into(),
        );
    }
    // The three keys nothing ever read (D63 E2). They parse for one more
    // minor version, because every existing install's config carries them and
    // `deny_unknown_fields` would otherwise refuse the file outright.
    if config.confirm_kill.is_some() {
        warnings.push(
            "`confirm_kill` is retired — nothing ever read it: the confirm bar asks whatever \
             an action says it must (D58), and `readonly` is the flag that blanks kill \
             and renice. Delete the line."
                .into(),
        );
    }
    if config.store.max_mb.is_some() {
        warnings.push(
            "`[store] max_mb` is retired — nothing ever read it, and there is no byte cap to \
             read it: `[store] history` bounds what the store keeps, and the F12 HUD \
             and `--stats-log` report the bytes it holds. Delete the line."
                .into(),
        );
    }
    if config.perf.phase_ms.is_some() {
        warnings.push(
            "`[perf] phase_ms` is retired — nothing ever read it and the grid it named was \
             never built: each source wakes on multiples of its own cadence. \
             Delete the line."
                .into(),
        );
    }
    // `[store] history` is live (D63 E2): a string that is not a duration is
    // a load error naming the file, as `borders = "nonsense"` is; an
    // out-of-range one is clamped with a warning, because the value *is*
    // usable at the edge of the range and refusing to start would be worse.
    let history = match parse_history(&config.store.history) {
        Ok(d) => d,
        Err(why) => {
            return Err(ConfigError(format!(
                "config.toml: [store] history = \"{}\" {why}",
                config.store.history
            )));
        }
    };
    let history = if history < MIN_HISTORY || history > MAX_HISTORY {
        let c = history.clamp(MIN_HISTORY, MAX_HISTORY);
        warnings.push(format!(
            "[store] history = \"{}\" clamped to {} (accepts {}-{})",
            config.store.history,
            say_duration(c),
            say_duration(MIN_HISTORY),
            say_duration(MAX_HISTORY)
        ));
        c
    } else {
        history
    };
    let retention = Retention::for_history(history);
    // `[[rules]]` are parsed here so a bad rule is a warning at load and an
    // error in `config check` — never a surprise at the first sample.
    let (rules, rule_errors) = gridwatch_store::rules::parse_all(&config.rules, &|k| {
        gridwatch_store::key::lookup(k).is_some()
    });
    warnings.extend(rule_errors.iter().map(|e| e.to_string()));
    // `[[plugins]]` (§4.7, arc 8b): an entry that cannot be started is dropped
    // with a warning rather than failing the load — one mistyped plugin should
    // not cost you the dashboard. The schema says the same thing in CI; this
    // says it to the person whose config was never validated by anything.
    let mut plugin_errors = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    config.plugins.retain(|pl| {
        let bad = if pl.argv.is_empty() || pl.argv[0].is_empty() {
            Some(format!(
                "[[plugins]] '{}': argv is empty — nothing to run",
                pl.id
            ))
        } else if !crate::plugin::proto::id_is_sane(&pl.id) {
            Some(format!(
                "[[plugins]] '{}': an id must be lower case letters, digits and _ — \
                 it namespaces this plugin's metric keys",
                pl.id
            ))
        } else if seen.contains(&pl.id) {
            Some(format!(
                "[[plugins]] '{}': a second entry with that id — ignored",
                pl.id
            ))
        } else {
            None
        };
        match bad {
            Some(w) => {
                plugin_errors.push(w);
                false
            }
            None => {
                seen.push(pl.id.clone());
                true
            }
        }
    });
    warnings.extend(plugin_errors);
    if config.schema != 1 || layout.schema != 1 {
        return Err(ConfigError("unsupported schema (expected 1)".into()));
    }
    // Env layer (D39: env between file and CLI; skipped for embedded loads).
    if env_layer {
        if let Ok(t) = std::env::var("GRIDWATCH_THEME") {
            config.theme = t;
        }
        if let Ok(f) = std::env::var("GRIDWATCH_FPS")
            && let Ok(v) = f.parse()
        {
            config.fps = v;
        }
        if let Ok(c) = std::env::var("GRIDWATCH_COLOR") {
            config.color = c;
        }
    }

    let borders = match layout.grid.borders.as_str() {
        "each" => BorderMode::Each,
        "shared" => BorderMode::Shared,
        "none" => BorderMode::None,
        other => return Err(ConfigError(format!("unknown borders mode '{other}'"))),
    };
    let grid = GridSpec {
        columns: layout.grid.columns,
        rows: layout.grid.rows,
        gap: layout.grid.gap,
        borders,
        cell_aspect: layout.grid.cell_aspect,
        min_unit_inner: Size::new(
            layout.grid.min_unit_inner.cols,
            layout.grid.min_unit_inner.rows,
        ),
    };
    if grid.columns == 0 || grid.rows == 0 {
        return Err(ConfigError("grid columns/rows must be ≥ 1".into()));
    }
    let known_kinds: Vec<&str> = Vec::new(); // filled by the shell against the registry
    let _ = known_kinds;
    let mut pages = Vec::new();
    for p in &layout.pages {
        let mut place = Vec::new();
        for (i, ps) in p.place.iter().enumerate() {
            let target = match (&ps.id, &ps.kind) {
                (Some(id), None) => PlaceTarget::Id(id.clone()),
                (None, Some(k)) => PlaceTarget::Kind(k.clone()),
                _ => {
                    return Err(ConfigError(format!(
                        "page '{}' placement {i}: exactly one of `id` or `kind`",
                        p.name
                    )));
                }
            };
            let placement = Placement {
                target,
                at: (ps.at[0], ps.at[1]),
                size: (ps.size[0], ps.size[1]),
                view: ps.view.clone(),
                priority: ps.priority.unwrap_or(0),
            };
            if !placement.in_bounds(grid.columns, grid.rows) {
                return Err(ConfigError(format!(
                    "page '{}' placement '{}' out of the {}x{} grid",
                    p.name,
                    placement.target.label(),
                    grid.columns,
                    grid.rows
                )));
            }
            for prev in &place {
                if placement.overlaps(prev) {
                    return Err(ConfigError(format!(
                        "page '{}': '{}' overlaps '{}'",
                        p.name,
                        placement.target.label(),
                        Placement::target_label(prev)
                    )));
                }
            }
            place.push(placement);
        }
        pages.push(Page {
            name: p.name.clone(),
            hotkey: p.hotkey.as_ref().and_then(|h| h.chars().next()),
            place,
        });
    }
    if pages.is_empty() {
        return Err(ConfigError("layout has no pages".into()));
    }
    // Duplicate instance ids are a config error; instances of unknown kinds
    // become placeholder chips at solve time (§6).
    let mut seen = std::collections::BTreeSet::new();
    for inst in &config.components {
        if !seen.insert(inst.id.clone()) {
            return Err(ConfigError(format!("duplicate component id '{}'", inst.id)));
        }
    }
    if config.fps == 0 || config.fps > 120 {
        warnings.push(format!("fps {} clamped into 1..=120", config.fps));
        config.fps = config.fps.clamp(1, 120);
    }
    Ok(Loaded {
        config,
        grid,
        pages,
        rules,
        retention,
        warnings,
        config_path,
        layout_path,
    })
}

trait TargetLabel {
    fn target_label(p: &Placement) -> String;
}

impl TargetLabel for Placement {
    fn target_label(p: &Placement) -> String {
        p.target.label().to_string()
    }
}

/// The colour ladder (§7): CLI > config > NO_COLOR (→ mono theme) > COLORTERM > TERM.
/// The environment is snapshotted into `ColorEnv` so the ladder is unit-testable.
pub fn resolve_color(
    cli: Option<&str>,
    cfg: &str,
    env: &ColorEnv,
) -> (gridwatch_ui::ColorMode, bool) {
    use gridwatch_ui::ColorMode::*;
    let pick = |s: &str| match s {
        "truecolor" | "always" => Some(TrueColor),
        "256" => Some(Ansi256),
        "16" => Some(Ansi16),
        "never" | "mono" => Some(Mono),
        _ => None,
    };
    if let Some(m) = cli.and_then(pick) {
        return (m, m == Mono);
    }
    if cfg != "auto"
        && let Some(m) = pick(cfg)
    {
        return (m, m == Mono);
    }
    if env.no_color {
        return (Mono, true);
    }
    if env
        .colorterm
        .as_deref()
        .is_some_and(|v| v.contains("truecolor") || v.contains("24bit"))
    {
        return (TrueColor, false);
    }
    if env.term.as_deref().is_some_and(|v| v.contains("256")) {
        return (Ansi256, false);
    }
    (Ansi16, false)
}

#[derive(Clone, Debug, Default)]
pub struct ColorEnv {
    pub no_color: bool,
    pub colorterm: Option<String>,
    pub term: Option<String>,
}

impl ColorEnv {
    pub fn capture() -> ColorEnv {
        ColorEnv {
            no_color: std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()),
            colorterm: std::env::var("COLORTERM").ok(),
            term: std::env::var("TERM").ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The embedded TOML and the structural Default must never drift (§9).
    #[test]
    fn embedded_default_matches_structural() {
        let parsed: ConfigFile = parse("embedded default config", DEFAULT_CONFIG).unwrap();
        assert_eq!(parsed, ConfigFile::default());
    }

    /// D63 trap 1 — determinism is config, not clock. Four values must agree
    /// or a replay, a `shot` and a live run could keep different windows:
    /// the shipped TOML, the structural default, `parse_history` over it, and
    /// `Retention::default()`, which every store test is written against.
    #[test]
    fn the_shipped_history_and_the_default_retention_are_the_same_thing() {
        assert_eq!(StoreSect::default().history, DEFAULT_HISTORY);
        let loaded = load_from(DEFAULT_CONFIG, DEFAULT_LAYOUT, None, None, false).unwrap();
        assert_eq!(loaded.config.store.history, DEFAULT_HISTORY);
        let d = Retention::default();
        for r in [
            loaded.retention,
            Retention::for_history(parse_history(DEFAULT_HISTORY).unwrap()),
        ] {
            assert_eq!(
                (r.max_len, r.max_age, r.max_uncatalogued),
                (d.max_len, d.max_age, d.max_uncatalogued)
            );
        }
    }

    #[test]
    fn parse_history_takes_the_forms_section_9_writes() {
        for (text, secs) in [
            ("10m", 600),
            ("1h", 3600),
            ("90s", 90),
            ("1h30m", 5400),
            ("  1h  ", 3600),
            ("2h30m10s", 9010),
        ] {
            assert_eq!(parse_history(text), Ok(Duration::from_secs(secs)), "{text}");
        }
        for text in ["ten", "", "10", "10x", "m10", "1h 30m", "-5m"] {
            assert!(parse_history(text).is_err(), "{text} must not parse");
        }
        // The message says what to write instead, in every case.
        assert!(parse_history("ten").unwrap_err().contains("s, m, h"));
        assert!(parse_history("10").unwrap_err().contains("\"10m\""));
    }

    /// A string that is not a duration is a load error naming the file, as
    /// `borders = "nonsense"` is — nothing else in `config.toml` fails a load
    /// on a value, and this one must, because there is no sane fallback that
    /// is not the silent ten minutes D63 exists to end.
    #[test]
    fn a_history_that_is_not_a_duration_refuses_the_load() {
        let text = DEFAULT_CONFIG.replace("history = \"10m\"", "history = \"ten\"");
        let Err(e) = load_from(&text, DEFAULT_LAYOUT, None, None, false) else {
            panic!("a non-duration must not load");
        };
        assert!(e.0.contains("config.toml"), "{}", e.0);
        assert!(e.0.contains("[store] history = \"ten\""), "{}", e.0);
    }

    /// Out of range is a clamp with a warning, in both directions: the value
    /// *is* usable at the edge, so refusing to start would be worse.
    #[test]
    fn a_history_out_of_range_is_clamped_and_says_so() {
        for (text, want_secs, said) in [("30s", 60, "clamped to 1m"), ("6h", 3600, "clamped to 1h")]
        {
            let cfg = DEFAULT_CONFIG.replace("history = \"10m\"", &format!("history = \"{text}\""));
            let loaded = load_from(&cfg, DEFAULT_LAYOUT, None, None, false).unwrap();
            assert_eq!(
                loaded.retention.max_age,
                Duration::from_secs(want_secs),
                "{text}"
            );
            assert!(
                loaded.warnings.iter().any(|w| w.contains(said)),
                "{text}: {:?}",
                loaded.warnings
            );
            assert!(
                loaded.warnings.iter().any(|w| w.contains("accepts 1m-1h")),
                "{text}: {:?}",
                loaded.warnings
            );
        }
    }

    /// `history = "1h"` resolves to the numbers the ROADMAP names.
    #[test]
    fn an_hour_of_history_is_an_hour_of_points() {
        let cfg = DEFAULT_CONFIG.replace("history = \"10m\"", "history = \"1h\"");
        let loaded = load_from(&cfg, DEFAULT_LAYOUT, None, None, false).unwrap();
        assert_eq!(loaded.retention.max_age, Duration::from_secs(3600));
        assert_eq!(loaded.retention.max_len, 14_400);
        assert_eq!(loaded.retention.max_uncatalogued, 512);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    /// D63 E2: three keys nothing ever read. They still parse — every
    /// existing install's config carries them and `deny_unknown_fields` would
    /// otherwise refuse the file — and each one says so, once.
    #[test]
    fn a_retired_key_parses_and_says_it_is_retired() {
        for (from, to, key) in [
            (
                "readonly = false",
                "readonly = false\nconfirm_kill = true",
                "`confirm_kill` is retired",
            ),
            (
                "[store]",
                "[store]\nmax_mb = 32",
                "`[store] max_mb` is retired",
            ),
            (
                "[perf]",
                "[perf]\nphase_ms = 250",
                "`[perf] phase_ms` is retired",
            ),
        ] {
            let line = to;
            let cfg = DEFAULT_CONFIG.replace(from, to);
            let loaded = load_from(&cfg, DEFAULT_LAYOUT, None, None, false)
                .unwrap_or_else(|e| panic!("{line}: {e}"));
            let hits: Vec<&String> = loaded.warnings.iter().filter(|w| w.contains(key)).collect();
            assert_eq!(hits.len(), 1, "{line}: {:?}", loaded.warnings);
            assert!(hits[0].contains("Delete the line."), "{}", hits[0]);
        }
        // And the shipped default carries none of them, so it is silent.
        let loaded = load_from(DEFAULT_CONFIG, DEFAULT_LAYOUT, None, None, false).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    /// Every rung of the §7 ladder, driven through the injected env snapshot.
    #[test]
    fn color_ladder_env_rungs() {
        use gridwatch_ui::ColorMode::*;
        let noisy = ColorEnv {
            no_color: true,
            colorterm: Some("truecolor".into()),
            term: Some("xterm-256color".into()),
        };
        assert_eq!(resolve_color(None, "auto", &noisy), (Mono, true));
        let tc = ColorEnv {
            no_color: false,
            colorterm: Some("truecolor".into()),
            term: None,
        };
        assert_eq!(resolve_color(None, "auto", &tc), (TrueColor, false));
        let c256 = ColorEnv {
            no_color: false,
            colorterm: None,
            term: Some("xterm-256color".into()),
        };
        assert_eq!(resolve_color(None, "auto", &c256), (Ansi256, false));
        assert_eq!(
            resolve_color(Some("16"), "truecolor", &noisy),
            (Ansi16, false)
        );
    }

    /// §9: a parse error names the file, the line and the column.
    #[test]
    fn parse_errors_name_the_line() {
        let cfg = "schema = 1\ntheme = \"mono\"\nfps = \"thirty\"\n";
        let Err(err) = load_from(cfg, DEFAULT_LAYOUT, None, None, false) else {
            panic!("a string fps was accepted");
        };
        assert!(
            err.to_string().starts_with("config: config.toml:3:"),
            "{err}"
        );
        assert_eq!(line_col("ab\ncd\nef", 4), (2, 2));
    }

    /// A zero-column grid would underflow thresholds(); it must be rejected.
    #[test]
    fn zero_grid_rejected() {
        let layout = "schema = 1\npages = []\n[grid]\ncolumns = 0\n";
        let Err(err) = load_from(DEFAULT_CONFIG, layout, None, None, false) else {
            panic!("a zero-column grid was accepted");
        };
        assert!(err.to_string().contains("columns"), "{err}");
    }

    /// §9: [record] and [[rules]] parse today and warn until their arcs.
    #[test]
    fn record_and_rules_parse_with_warning() {
        let cfg = "schema = 1\n[record]\nring_mb = 8\n[[rules]]\nid = \"x\"\n";
        let loaded = load_from(cfg, DEFAULT_LAYOUT, None, None, false).unwrap();
        // Eleven arcs of "arrives in arc 2" for a section that was never
        // going to arrive: it says what to write instead now (D63).
        let rec: Vec<&String> = loaded
            .warnings
            .iter()
            .filter(|w| w.contains("[record]"))
            .collect();
        assert_eq!(rec.len(), 1, "{:?}", loaded.warnings);
        assert!(rec[0].contains("`--record FILE`"), "{}", rec[0]);
        assert!(!rec[0].contains("arc 2"), "{}", rec[0]);
        assert!(loaded.warnings.iter().any(|w| w.contains("rules")));
    }

    /// A partial user config layers over the defaults without recursing.
    #[test]
    fn partial_config_layers() {
        let cfg: ConfigFile = parse("partial", "schema = 1\ntheme = \"mono\"\n").unwrap();
        assert_eq!(cfg.theme, "mono");
        assert_eq!(cfg.fps, 30);
        // A config that names no components inherits the whole default list —
        // asserted against that list rather than against its length, which is
        // incidental and went stale the first time a component was added (D66).
        assert_eq!(cfg.components, ConfigFile::default().components);
    }
}
