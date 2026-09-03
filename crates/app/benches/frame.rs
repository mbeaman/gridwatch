//! The three costs the design rests on (D59 seam 3, arc 9a).
//!
//! `docs/PERFORMANCE.md` gates the *product* — CPU, wake-ups, bytes to the
//! terminal — measured on a real run with `pidstat`. That is the number that
//! matters and it is the one a person feels. These benches are the layer
//! underneath: the three functions whose cost every one of those ceilings
//! assumes, isolated so a regression in one is legible instead of showing up
//! as "the dashboard got slower".
//!
//! **This is not a commit gate.** `scripts/gate.sh` does not run it: a timing
//! assertion on a machine that is also running a game is a flake generator,
//! and a red build that means "the laptop was busy" teaches people to ignore
//! red builds. Run it by hand and record the numbers:
//!
//! ```console
//! $ cargo bench -p gridwatch-app
//! ```
//!
//! then put them in `PERFORMANCE.md`'s bench table with the date and the
//! machine, where a human compares them.

use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use gridwatch_store::{Agg, Msg, Store, keys};
use gridwatch_ui::{ColorMode, Registry};

/// A store with `ticks` of the cpu synth already applied — the shape a running
/// dashboard's store has, rather than an empty one.
fn warm_store(ticks: usize) -> Store {
    let mut store = Store::default();
    let mut synth = gridwatch_store::demo::CpuSynth::new(1);
    for i in 0..ticks {
        let at = gridwatch_store::Ts((i as u64 + 1) * 1_500_000_000);
        store.apply(&Msg::Batch(
            synth.tick_at(at, gridwatch_store::Detail::Table),
        ));
    }
    store
}

fn registry() -> Registry {
    let mut reg = Registry::default();
    gridwatch_components::builtin_components(&mut reg);
    gridwatch_sources::builtin_sources(&mut reg);
    reg
}

/// `Store::apply` over one realistic batch.
///
/// Every sample the machine produces goes through here on the render thread,
/// and the rules engine runs inside it (arc 7b), so this is the per-message
/// cost of the whole data path. The batch is the cpu source's own — around
/// forty scalars plus the process table.
fn apply(c: &mut Criterion) {
    let mut synth = gridwatch_store::demo::CpuSynth::new(1);
    let msg = Msg::Batch(synth.tick_at(
        gridwatch_store::Ts(60_000_000_000),
        gridwatch_store::Detail::Table,
    ));
    let mut group = c.benchmark_group("store");
    group.bench_function("apply/cpu batch", |b| {
        // A minute of history first, so retention and the series maps are the
        // size they are in a running dashboard rather than empty.
        let mut store = warm_store(40);
        b.iter(|| {
            std::hint::black_box(store.apply(std::hint::black_box(&msg)));
        });
    });
    group.finish();
}

/// `resample` over a full ring into a chart's buckets.
///
/// Every chart tier calls this once per series per render — the gpu tile's
/// ten-minute band is six of them — so it is the cost that decides whether a
/// chart tier is cheap enough to leave on screen.
fn resample(c: &mut Criterion) {
    let store = warm_store(600);
    let mut out: Vec<Option<f64>> = Vec::new();
    let mut group = c.benchmark_group("store");
    for buckets in [60usize, 120, 240] {
        group.bench_function(format!("resample/{buckets} buckets"), |b| {
            b.iter(|| {
                store.resample(
                    &keys::cpu::TOTAL_PCT,
                    Duration::from_secs(600),
                    std::hint::black_box(buckets),
                    Agg::Avg,
                    &mut out,
                );
                std::hint::black_box(out.len());
            });
        });
    }
    group.finish();
}

/// One whole frame of the Overview, solved and drawn, at the two sizes the
/// layout thresholds actually separate.
///
/// This is `shot_frame` — the same path `gridwatch shot` and the determinism
/// test use — so it includes the layout solve, every visible tile's `tick`,
/// `view` and render, the chrome and the diff. In the running app the render
/// cache means most frames are a blit of unchanged tiles, so this is the
/// *worst* case: everything re-rendered from nothing.
fn frame(c: &mut Criterion) {
    let mut group = c.benchmark_group("render");
    group.sample_size(30);
    for (w, h, what) in [
        (250u16, 70u16, "250x70 configured"),
        (120, 40, "120x40 dense"),
    ] {
        group.bench_function(format!("frame/{what}"), |b| {
            // A fresh shell per iteration would measure `Shell::new`; one
            // shell reused measures the frame, which is the question.
            let mut shell = gridwatch_app::headless_shell(registry(), "retrowave", 1)
                .expect("a headless shell");
            gridwatch_app::feed_synth(&mut shell, 1, 40);
            b.iter(|| {
                std::hint::black_box(gridwatch_app::shot_frame(&mut shell, w, h));
            });
        });
    }
    group.finish();
}

/// The theme loader, because every `t` press pays it and the WCAG gate runs
/// inside it.
fn theme(c: &mut Criterion) {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../themes/retrowave.toml"),
    )
    .expect("the retrowave theme");
    c.bench_function("theme/load retrowave", |b| {
        b.iter(|| {
            let file =
                gridwatch_ui::theme::load_theme_file(std::hint::black_box(&text)).expect("parses");
            std::hint::black_box(
                gridwatch_ui::theme::build_theme(&file, None, ColorMode::TrueColor)
                    .expect("builds"),
            );
        });
    });
}

criterion_group!(benches, apply, resample, frame, theme);
criterion_main!(benches);
