//! Component gate tests (§12): snapshot matrix, no-panic sweep, tier hygiene.

use gridwatch_ui::component::{Component, Size};
use gridwatch_ui::testkit::{
    assert_min_tier_fits, assert_never_panics, assert_tiers_well_formed, demo_store,
    real_grid_sizes, render_component, theme, view_snapshot,
};

fn clock() -> Box<dyn Component> {
    Box::new(gridwatch_components::clock::Clock)
}

fn sources() -> Box<dyn Component> {
    Box::new(gridwatch_components::sources_tile::SourcesTile)
}

#[test]
fn tiers_are_well_formed() {
    for mk in [clock, sources] {
        let c = mk();
        assert_tiers_well_formed(c.tiers());
        assert_min_tier_fits(c.tiers(), Size::new(8, 3));
    }
}

#[test]
fn never_panics_across_sizes() {
    let store = demo_store(42, 3);
    let th = theme("modern");
    assert_never_panics(&|| clock(), &store, &th);
    assert_never_panics(&|| sources(), &store, &th);
}

#[test]
fn view_snapshots_at_real_grid_sizes() {
    let store = demo_store(42, 3);
    let th = theme("modern");
    for (name, size) in real_grid_sizes() {
        let c = clock();
        insta::assert_yaml_snapshot!(
            format!("clock_{name}"),
            view_snapshot(c.as_ref(), &store, &th, size)
        );
        let s = sources();
        insta::assert_yaml_snapshot!(
            format!("sources_{name}"),
            view_snapshot(s.as_ref(), &store, &th, size)
        );
    }
}

#[test]
fn rendered_cells_snapshot_modern_only() {
    // Styled dumps at the reference theme only (§12.2): one per component at
    // one representative size; themes are covered by the role swatches.
    let store = demo_store(42, 3);
    let th = theme("modern");
    let (_, buf) = render_component(clock().as_ref(), &store, &th, Size::new(38, 8), false);
    insta::assert_snapshot!("clock_cells_2x1", gridwatch_ui::dump::cells(&buf));
    let (_, buf) = render_component(sources().as_ref(), &store, &th, Size::new(80, 20), false);
    insta::assert_snapshot!("sources_cells_4x2", gridwatch_ui::dump::cells(&buf));
}
