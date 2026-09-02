//! The network tile (§8, brief arc 7 seam 3): the interfaces the net source
//! publishes, from an 8×3 rate pair to the zoom-only `full` with the route,
//! the probes and the connection table. The noise a workstation carries
//! (bridges, veths, docker) is hidden by default and `a` shows it; the
//! component never reads `/proc` — everything is `net.*` in the store.

mod view;

use std::borrow::Cow;

use gridwatch_store::keys::net::{self, Link, Probes};
use gridwatch_store::{Detail, KeyCode, KeyEvent, Label, Ts};
use gridwatch_ui::component::{
    BuildCx, BuildError, Chrome, Component, ComponentDef, Footprint, InputCx, KeyHint, Manifest,
    Outcome, Redraw, RenderCx, Size, TickCx, Tier,
};
use gridwatch_ui::view::View;
use serde::{Deserialize, Serialize};

pub static MANIFEST: Manifest = Manifest {
    kind: "net",
    name: "network",
    summary: "interface rates and link state, the default route and DNS, latency probes and the connection table",
    contract: 1,
    footprints: &[
        Footprint { w: 1, h: 1 },
        Footprint { w: 2, h: 1 },
        Footprint { w: 4, h: 2 },
        Footprint { w: 6, h: 3 },
    ],
    default_footprint: Footprint { w: 4, h: 2 },
    requires: &[],
    optional: &[
        gridwatch_store::Capability::PingSocket,
        gridwatch_store::Capability::NetRaw,
    ],
    sources: &[net::SOURCE],
    optional_sources: &[],
    chrome: Chrome::Themed,
    keys: &[
        KeyHint {
            key: "a",
            does: "every interface",
        },
        KeyHint {
            key: "s",
            does: "sort",
        },
        KeyHint {
            key: "↑/↓",
            does: "scroll",
        },
    ],
    example_options: "options = { interfaces = [\"en*\", \"wl*\"], sort = \"name\" }",
};

static TIERS: &[Tier] = &[
    Tier {
        name: "rates",
        min: Size::new(8, 3),
        adds: &["the default interface's rate pair", "a link dot"],
        zoom_only: false,
    },
    Tier {
        name: "sparks",
        min: Size::new(20, 5),
        adds: &["rx and tx sparklines", "the speed or SSID"],
        zoom_only: false,
    },
    Tier {
        name: "table",
        min: Size::new(48, 10),
        adds: &[
            "every shown interface",
            "drops and errors",
            "the probe strip",
        ],
        zoom_only: false,
    },
    Tier {
        name: "conns",
        min: Size::new(70, 16),
        adds: &["the connection table"],
        zoom_only: false,
    },
    Tier {
        name: "full",
        min: Size::new(100, 24),
        adds: &[
            "the route and DNS",
            "per-interface detail",
            "the probe statistics",
        ],
        zoom_only: true,
    },
];

pub const TIER_RATES: usize = 0;
pub const TIER_SPARKS: usize = 1;
pub const TIER_TABLE: usize = 2;
pub const TIER_CONNS: usize = 3;
pub const TIER_FULL: usize = 4;

/// What a workstation carries that nobody watches.
pub const DEFAULT_HIDE: &[&str] = &["lo", "veth*", "br-*", "docker*", "virbr*", "vnet*", "tap*"];

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    /// Busiest first (rx + tx).
    #[default]
    Traffic,
    Name,
}

impl Sort {
    pub fn next(self) -> Sort {
        match self {
            Sort::Traffic => Sort::Name,
            Sort::Name => Sort::Traffic,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Sort::Traffic => "traffic",
            Sort::Name => "name",
        }
    }
}

/// View-only instance options (§9).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    /// Globs to show; empty means everything the hide list leaves.
    pub interfaces: Vec<String>,
    pub hide: Vec<String>,
    pub sort: Sort,
    /// Reverse DNS is opt-in and not implemented yet (arc 7b/BACKLOG); the
    /// option exists so a config written today keeps working.
    pub rdns: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            interfaces: Vec::new(),
            hide: DEFAULT_HIDE.iter().map(|s| s.to_string()).collect(),
            sort: Sort::Traffic,
            rdns: false,
        }
    }
}

pub const OPTION_NAMES: &[&str] = &["interfaces", "hide", "sort", "rdns"];

/// One interface as the tile sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Iface {
    pub name: String,
    pub rx_bps: f64,
    pub tx_bps: f64,
    pub rx_drop: f64,
    pub tx_drop: f64,
    pub rx_err: f64,
    pub tx_err: f64,
    pub link: Option<Link>,
}

impl Iface {
    pub fn total(&self) -> f64 {
        self.rx_bps + self.tx_bps
    }

    /// The one-word state, or `—` when no link Record has arrived.
    pub fn state(&self) -> &'static str {
        self.link.as_ref().map(|l| l.state()).unwrap_or("—")
    }

    pub fn up(&self) -> bool {
        self.link.as_ref().is_some_and(|l| l.up && l.carrier)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub ifaces: Vec<Iface>,
    /// Every interface the source publishes, before the filter.
    pub all: usize,
    pub route: Option<net::Route>,
    pub probes: Option<Probes>,
    pub conns: Option<net::Conns>,
}

/// The project's one glob rule (`store::rules::glob`, D57 amendment 9): a
/// `*` anywhere, so `en*1` means what a person writing it expects. This was
/// a private copy that handled only a leading or trailing star.
pub use gridwatch_store::rules::glob;

impl Model {
    /// Derive from the store. `show_all` is the `a` key: the hide list off.
    pub fn refresh(
        &mut self,
        store: &gridwatch_store::Store,
        options: &Options,
        sort: Sort,
        show_all: bool,
    ) {
        let labels: Vec<Label> = store.labels(net::RX_BPS.id.name).cloned().collect();
        self.ifaces.clear();
        self.all = labels.len();
        for l in labels {
            let Label::Name(name) = &l else { continue };
            let iface = name.to_string();
            if !show_all {
                if !options.interfaces.is_empty()
                    && !options.interfaces.iter().any(|p| glob(p, &iface))
                {
                    continue;
                }
                if options.hide.iter().any(|p| glob(p, &iface)) {
                    continue;
                }
            }
            let last = |k: &gridwatch_store::Key<f64>| {
                store.last(&k.named(name)).map(|(_, v)| v).unwrap_or(0.0)
            };
            self.ifaces.push(Iface {
                rx_bps: last(&net::RX_BPS),
                tx_bps: last(&net::TX_BPS),
                rx_drop: last(&net::RX_DROP),
                tx_drop: last(&net::TX_DROP),
                rx_err: last(&net::RX_ERR),
                tx_err: last(&net::TX_ERR),
                link: store.record(&net::LINK.named(name)).map(|(_, l)| l.clone()),
                name: iface,
            });
        }
        match sort {
            Sort::Traffic => self.ifaces.sort_by(|a, b| {
                b.up()
                    .cmp(&a.up())
                    .then(b.total().total_cmp(&a.total()))
                    .then(a.name.cmp(&b.name))
            }),
            Sort::Name => self.ifaces.sort_by(|a, b| a.name.cmp(&b.name)),
        }
        self.route = store.record(&net::ROUTE).map(|(_, r)| r.clone());
        self.probes = store.record(&net::PROBE).map(|(_, p)| p.clone());
        self.conns = store.record(&net::CONNS).map(|(_, c)| c.clone());
    }

    /// The interface the default route uses, else the busiest.
    pub fn primary(&self) -> Option<&Iface> {
        self.route
            .as_ref()
            .and_then(|r| self.ifaces.iter().find(|i| i.name == r.default_iface))
            .or_else(|| self.ifaces.first())
    }

    /// How many interfaces the filter is hiding.
    pub fn hidden(&self) -> usize {
        self.all.saturating_sub(self.ifaces.len())
    }
}

pub struct Net {
    options: Options,
    model: Model,
    sort: Sort,
    show_all: bool,
    scroll: usize,
    seen: Option<Ts>,
}

impl Net {
    pub fn new(options: Options) -> Net {
        Net {
            sort: options.sort,
            options,
            model: Model::default(),
            show_all: false,
            scroll: 0,
            seen: None,
        }
    }

    pub fn from_table(options: &toml::Table) -> Result<Net, BuildError> {
        let parsed: Options = options
            .clone()
            .try_into()
            .map_err(|e| BuildError(format!("[[components]] options: {e}")))?;
        Ok(Net::new(parsed))
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    pub fn sort(&self) -> Sort {
        self.sort
    }

    pub fn show_all(&self) -> bool {
        self.show_all
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    fn rebuild(&mut self, store: &gridwatch_store::Store) {
        self.model
            .refresh(store, &self.options, self.sort, self.show_all);
    }
}

impl Default for Net {
    fn default() -> Net {
        Net::new(Options::default())
    }
}

fn build(cx: &mut BuildCx<'_>) -> Result<Box<dyn Component>, BuildError> {
    Ok(Box::new(Net::from_table(cx.options)?))
}

pub const DEF: fn() -> ComponentDef = || ComponentDef {
    manifest: &MANIFEST,
    build,
};

impl Component for Net {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn title(&self, _max_width: u16, _cx: &TickCx<'_>) -> Cow<'static, str> {
        Cow::Borrowed("network")
    }

    fn tiers(&self) -> &'static [Tier] {
        TIERS
    }

    /// The connection table is table-tier work, like the process table.
    fn demand(&self, tier: usize) -> Detail {
        if tier >= TIER_CONNS {
            Detail::Table
        } else {
            Detail::Meters
        }
    }

    fn tick(&mut self, cx: &TickCx<'_>) -> Redraw {
        let Some(at) = cx.store.last_sample(net::SOURCE) else {
            return Redraw::No;
        };
        if self.seen == Some(at) {
            return Redraw::No;
        }
        self.seen = Some(at);
        self.rebuild(cx.store);
        Redraw::Yes
    }

    fn on_key(&mut self, key: KeyEvent, cx: &InputCx<'_>) -> Outcome {
        match key.code {
            KeyCode::Char('a') => {
                self.show_all = !self.show_all;
                self.rebuild(cx.store);
                self.scroll = 0;
                Outcome::Consumed
            }
            KeyCode::Char('s') => {
                self.sort = self.sort.next();
                self.rebuild(cx.store);
                Outcome::Consumed
            }
            KeyCode::Up => {
                self.scroll = self.scroll.saturating_sub(1);
                Outcome::Consumed
            }
            KeyCode::Down => {
                let rows = self
                    .model
                    .conns
                    .as_ref()
                    .map(|c| c.rows.len())
                    .unwrap_or(0)
                    .max(self.model.ifaces.len());
                self.scroll = (self.scroll + 1).min(rows.saturating_sub(1));
                Outcome::Consumed
            }
            _ => Outcome::Ignored,
        }
    }

    fn view(&self, cx: &RenderCx<'_>) -> View {
        view::render(self, cx)
    }

    fn signature(&self, tier: usize) -> &'static [&'static str] {
        match tier {
            TIER_RATES | TIER_SPARKS => &["↓"],
            TIER_TABLE => &["iface"],
            TIER_CONNS => &["conns"],
            _ => &["route"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_and_the_defaults() {
        assert!(glob("*", "eno1"));
        assert!(glob("en*", "eno1"));
        assert!(glob("*0", "wlp7s0"));
        assert!(!glob("en", "eno1"));
        assert!(DEFAULT_HIDE.iter().any(|p| glob(p, "veth123")));
        assert!(DEFAULT_HIDE.iter().any(|p| glob(p, "br-6bb7413a559e")));
        assert!(DEFAULT_HIDE.iter().any(|p| glob(p, "lo")));
        assert!(!DEFAULT_HIDE.iter().any(|p| glob(p, "eno1")));
        assert!(!DEFAULT_HIDE.iter().any(|p| glob(p, "wlp7s0")));
        let t: toml::Table = toml::from_str(r#"sort = "name""#).unwrap();
        assert_eq!(Net::from_table(&t).unwrap().sort(), Sort::Name);
        let t: toml::Table = toml::from_str("colour = 1").unwrap();
        assert!(Net::from_table(&t).is_err());
        assert_eq!(Sort::Traffic.next().name(), "name");
    }
}
