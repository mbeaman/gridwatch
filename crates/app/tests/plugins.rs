//! The plugin host as the app uses it (§4.7, arc 8b, D58 seam 9).
//!
//! Every plugin here is a small Python script written by the test, so the
//! suite covers the shapes that matter — one that works, one that never
//! speaks, one that dies, one that floods, one that lies — without depending
//! on anything installed. The one exception is `plugins/examples/weather.py`,
//! which the repository ships and CI must be able to draw.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gridwatch_app::plugin::host;
use gridwatch_app::{Shell, config, probe, shot_frame};
use gridwatch_store::{Clock, SourceId, Ts, channels};
use gridwatch_ui::theme::load_builtin;
use gridwatch_ui::{ColorMode, Registry};

fn registry() -> Registry {
    let mut reg = Registry::default();
    gridwatch_components::builtin_components(&mut reg);
    gridwatch_sources::builtin_sources(&mut reg);
    reg
}

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// A directory of this test's own, named after the case so a failure is
/// findable. Removed and recreated, so a rerun is not a rerun of yesterday.
fn workdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gridwatch-plugin-test-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a work directory");
    dir
}

/// Write a Python plugin and return the argv that runs it.
fn python_plugin(dir: &Path, name: &str, body: &str) -> Vec<String> {
    let path = dir.join(format!("{name}.py"));
    let mut f = std::fs::File::create(&path).expect("write the plugin");
    f.write_all(body.as_bytes()).expect("write the plugin");
    vec!["python3".to_string(), path.to_string_lossy().into_owned()]
}

fn sect(id: &str, argv: Vec<String>) -> config::PluginSect {
    config::PluginSect {
        id: id.to_string(),
        argv,
        rss_mb: 256,
        cpu_secs: 60,
        // Short enough that a test of a plugin that never speaks is quick, and
        // long enough that starting python3 on a loaded machine is not a flake.
        hello_ms: 4_000,
        render_ms: 0,
    }
}

/// The preamble every well-behaved fixture plugin shares: read the hello,
/// declare one tier that fits 8x3.
const PREAMBLE: &str = r#"
import json, sys
def say(m):
    sys.stdout.write(json.dumps(m) + "\n"); sys.stdout.flush()
sys.stdin.readline()
say({"kind": "manifest", "manifest": {
    "kind": "probe", "name": "probe", "contract": 1,
    "tiers": [{"name": "badge", "min": {"w": 8, "h": 3}}],
    "produces": [{"key": "probe.value"}],
}})
"#;

fn start(plugins: &[config::PluginSect]) -> (host::Started, gridwatch_store::Inbox) {
    let (ch, inbox) = channels();
    let started = host::start(
        plugins,
        &probe::probe(),
        ch.data.clone(),
        ch.control.clone(),
        Ts::ZERO,
    );
    (started, inbox)
}

/// A shell over an explicit config, with the started plugins registered — the
/// same assembly `run_terminal` does, minus the terminal.
fn shell_with(loaded: &config::Loaded, started: &mut host::Started) -> Shell {
    let mut reg = registry();
    for def in std::mem::take(&mut started.defs) {
        reg.register_component(def);
    }
    let theme = load_builtin("mono", ColorMode::TrueColor).unwrap();
    let mut sh = Shell::new(
        reg,
        loaded,
        theme,
        probe::probe(),
        0,
        Clock::new_virtual(),
        BTreeMap::new(),
        BTreeMap::new(),
        false,
    );
    sh.attach_plugins(
        started.host.take(),
        loaded.config.plugins.iter().map(|p| p.id.clone()),
    );
    for id in &started.sources {
        sh.store.ensure_source(*id);
    }
    sh
}

fn one_plugin_config(id: &str, kind: &str, argv: &[String]) -> String {
    let argv = argv
        .iter()
        .map(|a| format!("{a:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "schema = 1\ntheme = \"mono\"\n\
         [[plugins]]\nid = \"{id}\"\nargv = [{argv}]\nhello_ms = 4000\nrender_ms = 0\n\
         [[components]]\nid = \"tile\"\nkind = \"{id}.{kind}\"\n"
    )
}

const ONE_TILE_LAYOUT: &str = "schema = 1\n\n[grid]\ncolumns = 12\nrows = 6\n\n\
     [[pages]]\nname = \"P\"\nplace = [{ id = \"tile\", at = [0, 0], size = [4, 2] }]\n";

/// The rendered cells with their style tags stripped — what a person sees.
fn plain(cells: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in cells.chars() {
        match c {
            '[' => in_tag = true,
            ']' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Draw until the plugin has answered, or give up. Two frames are needed at
/// minimum: the first asks, the second shows.
fn draw_until(sh: &mut Shell, w: u16, h: u16, want: &str) -> String {
    let mut last = String::new();
    for _ in 0..12 {
        let buf = shot_frame(sh, w, h);
        last = plain(&gridwatch_ui::dump::cells(&buf));
        if last.contains(want) {
            return last;
        }
        sh.settle_plugins(Duration::from_millis(500));
    }
    last
}

/// The whole path in one test: a plugin declares itself, the manifest becomes
/// a placeable kind, the tile draws the tree the plugin sent, and the sample
/// it published reaches the store under the id *this config* gave it.
#[test]
fn a_plugin_declares_itself_draws_a_tile_and_publishes_a_metric() {
    let dir = workdir("happy");
    let argv = python_plugin(
        &dir,
        "happy",
        &format!(
            r#"{PREAMBLE}
say({{"kind": "sample", "key": "probe.value", "value": 42.5}})
for line in sys.stdin:
    ask = json.loads(line)
    if ask.get("kind") == "render":
        say({{"kind": "view", "instance": ask["instance"],
             "tree": {{"text": [[{{"role": "text", "text": "HELLO-FROM-PLUGIN"}}]]}}}})
"#
        ),
    );
    let (mut started, inbox) = start(&[sect("probe", argv.clone())]);
    assert!(started.warnings.is_empty(), "{:?}", started.warnings);
    let kinds: Vec<&str> = started.defs.iter().map(|d| d.manifest.kind).collect();
    assert_eq!(kinds, ["probe.probe"], "the kind is <id>.<manifest kind>");
    assert_eq!(started.sources, [SourceId("probe")]);

    let loaded = config::load_texts(&one_plugin_config("probe", "probe", &argv), ONE_TILE_LAYOUT)
        .expect("the config parses");
    let mut sh = shell_with(&loaded, &mut started);
    let frame = draw_until(&mut sh, 120, 40, "HELLO-FROM-PLUGIN");
    assert!(
        frame.contains("HELLO-FROM-PLUGIN"),
        "the plugin's tree was not drawn:\n{frame}"
    );

    // The sample went out as an ordinary batch on the data channel, under the
    // configured id — not the name the manifest wrote.
    let mut seen = None;
    for b in inbox.data.try_iter() {
        if b.source == SourceId("probe") {
            seen = Some(b.samples[0].id.name);
        }
    }
    assert_eq!(seen, Some("probe.value"), "no sample under `probe`");
}

/// A plugin that never says anything costs its `hello_ms` and no more, says
/// so, and leaves a placement chipped with the truth rather than "arrives in
/// a later arc".
#[test]
fn a_plugin_that_never_speaks_is_given_up_on_and_says_why() {
    let dir = workdir("silent");
    // Reads its hello and then blocks forever without a manifest.
    let argv = python_plugin(&dir, "silent", "import sys\nsys.stdin.read()\n");
    let mut sect = sect("silent", argv.clone());
    sect.hello_ms = 300;
    let began = std::time::Instant::now();
    let (mut started, _inbox) = start(&[sect]);
    assert!(
        began.elapsed() < Duration::from_secs(3),
        "a silent plugin must not hold startup for longer than hello_ms"
    );
    assert!(
        started.defs.is_empty(),
        "a plugin with no manifest has no tile"
    );
    assert!(
        started.warnings.iter().any(|w| w.contains("no manifest")),
        "{:?}",
        started.warnings
    );

    let mut config = one_plugin_config("silent", "probe", &argv);
    config = config.replace("hello_ms = 4000", "hello_ms = 300");
    let loaded = config::load_texts(&config, ONE_TILE_LAYOUT).unwrap();
    let mut sh = shell_with(&loaded, &mut started);
    let frame = plain(&gridwatch_ui::dump::cells(&shot_frame(&mut sh, 120, 40)));
    assert!(
        frame.contains("this plugin sent no manifest"),
        "the chip should name the real reason:\n{frame}"
    );
}

/// A manifest the host cannot place is refused by the rule it broke, and the
/// plugin gets no tile — the check that already existed, now reached through
/// the whole start path.
#[test]
fn a_manifest_that_would_not_fit_the_smallest_tile_is_refused() {
    let dir = workdir("toobig");
    let argv = python_plugin(
        &dir,
        "toobig",
        r#"
import json, sys
def say(m):
    sys.stdout.write(json.dumps(m) + "\n"); sys.stdout.flush()
sys.stdin.readline()
say({"kind": "manifest", "manifest": {
    "kind": "huge", "name": "huge", "contract": 1,
    "tiers": [{"name": "big", "min": {"w": 80, "h": 24}}],
}})
sys.stdin.read()
"#,
    );
    let (started, _inbox) = start(&[sect("toobig", argv)]);
    assert!(started.defs.is_empty());
    assert!(
        started.warnings.iter().any(|w| w.contains("no manifest")),
        "a refused manifest leaves the plugin without a tile: {:?}",
        started.warnings
    );
}

/// Three malformed lines stop a plugin instead of restarting it, and the tile
/// says so in the plugin's place.
#[test]
fn three_bad_lines_stop_the_plugin_and_the_tile_says_so() {
    let dir = workdir("strikes");
    let argv = python_plugin(
        &dir,
        "strikes",
        &format!(
            r#"{PREAMBLE}
for i in range(6):
    sys.stdout.write("this is not json\n"); sys.stdout.flush()
sys.stdin.read()
"#
        ),
    );
    let (mut started, _inbox) = start(&[sect("probe", argv.clone())]);
    assert_eq!(
        started.defs.len(),
        1,
        "the manifest came before the garbage"
    );
    let loaded =
        config::load_texts(&one_plugin_config("probe", "probe", &argv), ONE_TILE_LAYOUT).unwrap();
    let mut sh = shell_with(&loaded, &mut started);
    let frame = draw_until(&mut sh, 120, 40, "malformed");
    assert!(
        frame.contains("3 malformed messages"),
        "a struck-out plugin's tile should say what happened:\n{frame}"
    );
}

/// A `status` of `unavailable` is a plugin saying it cannot work here. It is
/// not a strike, and its reason and hint are what the tile shows.
#[test]
fn an_unavailable_status_shows_the_plugin_s_own_words() {
    let dir = workdir("unavailable");
    let argv = python_plugin(
        &dir,
        "unavailable",
        &format!(
            r#"{PREAMBLE}
say({{"kind": "status", "state": "unavailable",
     "reason": "NO-SENSOR-HERE", "hint": "plug one in"}})
sys.stdin.read()
"#
        ),
    );
    let (mut started, _inbox) = start(&[sect("probe", argv.clone())]);
    let loaded =
        config::load_texts(&one_plugin_config("probe", "probe", &argv), ONE_TILE_LAYOUT).unwrap();
    let mut sh = shell_with(&loaded, &mut started);
    let frame = draw_until(&mut sh, 120, 40, "NO-SENSOR-HERE");
    assert!(
        frame.contains("NO-SENSOR-HERE") && frame.contains("plug one in"),
        "the reason and the fix both belong on the tile:\n{frame}"
    );
}

/// A tree the contract does not accept is refused **by name**, on the tile,
/// so the plugin author reads a sentence instead of seeing a blank square.
#[test]
fn a_tree_the_contract_refuses_says_which_shape_was_wrong() {
    let dir = workdir("badtree");
    let argv = python_plugin(
        &dir,
        "badtree",
        &format!(
            r#"{PREAMBLE}
for line in sys.stdin:
    ask = json.loads(line)
    if ask.get("kind") == "render":
        say({{"kind": "view", "instance": ask["instance"],
             "tree": {{"text": [[{{"role": "chartreuse", "text": "x"}}]]}}}})
"#
        ),
    );
    let (mut started, _inbox) = start(&[sect("probe", argv.clone())]);
    let loaded =
        config::load_texts(&one_plugin_config("probe", "probe", &argv), ONE_TILE_LAYOUT).unwrap();
    let mut sh = shell_with(&loaded, &mut started);
    let frame = draw_until(&mut sh, 120, 40, "chartreuse");
    assert!(
        frame.contains("chartreuse"),
        "the refusal should name the role the plugin invented:\n{frame}"
    );
}

/// The read-rate budget, which is what makes a flooding plugin free (P22).
///
/// This is the regression that cost 62 % of a core when it was missing, and
/// the number to assert is the one that regressed: how many messages the host
/// reads. A test that only checked the shell kept drawing would have passed
/// before the fix — the flood never stalled the render thread, it burned a
/// different one.
#[test]
fn a_flooding_plugin_is_read_no_faster_than_the_budget() {
    use gridwatch_app::plugin::supervise;
    let dir = workdir("flood");
    let argv = python_plugin(
        &dir,
        "flood",
        &format!(
            r#"{PREAMBLE}
import itertools
for i in itertools.count():
    say({{"kind": "sample", "key": "probe.value", "value": i % 100}})
"#
        ),
    );
    let mut plugin = gridwatch_app::plugin::Plugin::spawn(
        gridwatch_app::plugin::PluginConfig::new("flood", argv),
        gridwatch_app::plugin::proto::Hello::new(Vec::new(), Vec::new()),
    );
    assert!(matches!(
        plugin.next_report(Duration::from_secs(5)),
        Some(gridwatch_app::plugin::Report::Ready(_))
    ));
    // Drain as fast as the host would, for two windows.
    let window = Duration::from_millis(2_000);
    let began = std::time::Instant::now();
    let mut read = 0usize;
    while began.elapsed() < window {
        read += plugin.drain().len();
        std::thread::sleep(Duration::from_millis(20));
    }
    let dropped = plugin.dropped();
    plugin.stop();
    // Generous on purpose: the two seconds measured here overlap three of the
    // reader's own one-second windows, and a loaded runner shifts where the
    // boundaries fall. It is still decisive — the unthrottled reader this
    // replaced took *hundreds of thousands* of messages in the same window,
    // two orders of magnitude past this ceiling.
    let ceiling = supervise::MAX_MSGS_PER_SEC as usize * 5;
    assert!(
        read + dropped as usize <= ceiling,
        "read {read} + dropped {dropped} messages in 2 s against a budget of \
         {} a second — the reader is not throttled",
        supervise::MAX_MSGS_PER_SEC
    );
    assert!(
        read > 0,
        "a throttled reader still has to read: got nothing in 2 s"
    );
}

/// A plugin that floods from its very first line still gets a tile.
///
/// The drop-oldest rule cost one: the manifest is the first thing in the
/// queue, so a plugin writing thousands of samples behind it evicted its own
/// declaration before the host read it, and the tile silently never appeared.
/// CI caught it as an intermittent failure of the budget test above, which is
/// the only reason it is a test here rather than a bug someone hits in a year.
#[test]
fn a_flood_cannot_evict_its_own_manifest() {
    let dir = workdir("evict");
    let argv = python_plugin(
        &dir,
        "evict",
        &format!(
            r#"{PREAMBLE}
import itertools
for i in itertools.count():
    say({{"kind": "sample", "key": "probe.value", "value": i % 100}})
"#
        ),
    );
    // Give the child a clear head start, so the queue is certainly full and
    // certainly has dropped by the time anything is read.
    let plugin = gridwatch_app::plugin::Plugin::spawn(
        gridwatch_app::plugin::PluginConfig::new("evict", argv),
        gridwatch_app::plugin::proto::Hello::new(Vec::new(), Vec::new()),
    );
    std::thread::sleep(Duration::from_millis(1_200));
    assert!(
        plugin.dropped() > 0,
        "the flood should have filled the queue"
    );
    let manifest = plugin
        .drain()
        .into_iter()
        .any(|r| matches!(r, gridwatch_app::plugin::Report::Ready(_)));
    assert!(
        manifest,
        "the manifest was dropped to make room for samples — the plugin would \
         have got no tile"
    );
}

/// The queue is bounded and says so: a plugin that outruns a host which is not
/// draining loses the oldest reports rather than growing the host (seam 7).
#[test]
fn the_queue_drops_rather_than_growing() {
    use gridwatch_app::plugin::supervise;
    let dir = workdir("queue");
    let argv = python_plugin(
        &dir,
        "queue",
        &format!(
            r#"{PREAMBLE}
import itertools
for i in itertools.count():
    say({{"kind": "sample", "key": "probe.value", "value": i % 100}})
"#
        ),
    );
    let plugin = gridwatch_app::plugin::Plugin::spawn(
        gridwatch_app::plugin::PluginConfig::new("queue", argv),
        gridwatch_app::plugin::proto::Hello::new(Vec::new(), Vec::new()),
    );
    assert!(matches!(
        plugin.next_report(Duration::from_secs(5)),
        Some(gridwatch_app::plugin::Report::Ready(_))
    ));
    // Nobody drains for a second: the queue fills and then drops.
    std::thread::sleep(Duration::from_millis(1_200));
    let waiting = plugin.drain().len();
    assert!(
        waiting <= supervise::QUEUE_DEPTH,
        "{waiting} reports were waiting behind a queue {} deep",
        supervise::QUEUE_DEPTH
    );
    assert!(
        plugin.dropped() > 0,
        "a plugin that outran an idle host should have had reports dropped"
    );
}

/// A plugin cannot leak the host's address space one metric name at a time:
/// names are interned once and capped.
#[test]
fn distinct_metric_names_are_capped() {
    assert_eq!(host::MAX_METRIC_NAMES, 256);
    let dir = workdir("names");
    let argv = python_plugin(
        &dir,
        "names",
        &format!(
            r#"{PREAMBLE}
for i in range(1000):
    say({{"kind": "sample", "key": "probe.n%d" % i, "value": 1.0}})
sys.stdin.read()
"#
        ),
    );
    let (started, inbox) = start(&[sect("probe", argv)]);
    assert_eq!(started.defs.len(), 1);
    // Give the host thread a moment to drain what the child wrote at once.
    std::thread::sleep(Duration::from_millis(800));
    let mut names = std::collections::BTreeSet::new();
    for b in inbox.data.try_iter() {
        for s in &b.samples {
            names.insert(s.id.name);
        }
    }
    assert!(
        names.len() <= host::MAX_METRIC_NAMES,
        "{} distinct names got through a cap of {}",
        names.len(),
        host::MAX_METRIC_NAMES
    );
}

/// The example the repository ships draws, at both of its tiers. This is the
/// end-to-end row the brief asks CI for (D58 seam 8).
#[test]
fn the_example_plugin_draws() {
    let dir = workdir("weather");
    let py = repo("plugins/examples/weather.py");
    assert!(py.exists(), "the example plugin is missing");
    std::fs::write(
        dir.join("config.toml"),
        format!(
            "schema = 1\ntheme = \"mono\"\n\n[[plugins]]\nid = \"weather\"\n\
             argv = [\"python3\", {:?}]\nhello_ms = 4000\nrender_ms = 0\n\n\
             [[components]]\nid = \"outside\"\nkind = \"weather.weather\"\n",
            py.to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("layout.toml"),
        "schema = 1\n\n[grid]\ncolumns = 12\nrows = 6\n\n[[pages]]\nname = \"Plugin\"\n\
         place = [{ id = \"outside\", at = [0, 0], size = [4, 2] }]\n",
    )
    .unwrap();
    let frame = gridwatch_app::shot(
        registry(),
        1,
        160,
        40,
        "mono",
        1,
        "cells",
        Some(dir.as_path()),
    )
    .expect("the shot renders");
    let frame = plain(&frame);
    assert!(
        frame.contains("°C"),
        "the example plugin's tile drew nothing:\n{}",
        &frame[..frame.len().min(2000)]
    );
    assert!(
        frame.contains("outside"),
        "the tile is titled by the plugin's manifest name"
    );
}

/// `shot` without `--config` starts nothing and stays byte-identical: the
/// determinism promise D41 makes is not weakened by the flag existing.
#[test]
fn a_shot_without_a_config_is_unchanged() {
    let a = gridwatch_app::shot(registry(), 3, 120, 40, "mono", 1, "cells", None).unwrap();
    let b = gridwatch_app::shot(registry(), 3, 120, 40, "mono", 1, "cells", None).unwrap();
    assert_eq!(a, b);
}

/// The runaway rule, as arithmetic and as a measurement — the two halves of
/// the check that stops a plugin which simply spins (D58 seam 7). Testing the
/// policy directly is what keeps a core from being burnt for ten seconds on
/// every run; the live half was watched by hand and is recorded in
/// PERFORMANCE.md.
#[test]
fn the_runaway_rule_stops_a_spinning_plugin() {
    use gridwatch_app::plugin::supervise;
    let ten = Duration::from_secs(10);
    // Under the window: never, whatever it used.
    assert!(supervise::runaway(Duration::from_secs(9), Duration::from_secs(9)).is_none());
    // Under the share: a busy tenth of a core is a working plugin.
    assert!(supervise::runaway(ten, Duration::from_secs(1)).is_none());
    // At and over it: stopped, and the reason says the number.
    let why = supervise::runaway(ten, Duration::from_secs(10)).expect("a full core is a runaway");
    assert!(
        why.contains("100%") && why.contains("the ceiling is 50%"),
        "{why}"
    );
    assert!(supervise::runaway(ten, Duration::from_secs(5)).is_some());

    // And the measurement the rule is fed: a child that spins really does
    // read as most of a core, over a window short enough to be free.
    let dir = workdir("cpu");
    let argv = python_plugin(
        &dir,
        "cpu",
        &format!(
            r#"{PREAMBLE}
x = 0
while True:
    x = (x * 31 + 7) % 1000003
"#
        ),
    );
    let mut plugin = gridwatch_app::plugin::Plugin::spawn(
        gridwatch_app::plugin::PluginConfig::new("cpu", argv),
        gridwatch_app::plugin::proto::Hello::new(Vec::new(), Vec::new()),
    );
    // Wait for the manifest, so the child is past its imports and spinning.
    assert!(matches!(
        plugin.next_report(Duration::from_secs(5)),
        Some(gridwatch_app::plugin::Report::Ready(_))
    ));
    let before = plugin.cpu_used().expect("the child's CPU reads");
    let window = Duration::from_millis(600);
    std::thread::sleep(window);
    let after = plugin.cpu_used().expect("the child's CPU reads");
    let share = after.saturating_sub(before).as_secs_f64() / window.as_secs_f64();
    plugin.stop();
    assert!(
        share > supervise::RUNAWAY_SHARE,
        "a spinning child read as {share:.2} of a core, which the rule would not catch"
    );
}
