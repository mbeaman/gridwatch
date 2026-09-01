//! Built-in source definitions by feature (§4.3). Arc 1a ships the cpu slot:
//! live = stub (the real procfs source is arc 1b), demo = the seeded synth.

use gridwatch_store::{Source, SourceDef, demo};
use gridwatch_ui::Registry;

fn cpu_start(_options: &toml::Table) -> Box<dyn Source> {
    Box::new(crate::stub::StubSource {
        info: demo::cpu_info(),
        reason: "cpu source arrives in arc 1b",
        hint: "run with --demo for synthetic data",
    })
}

pub fn builtin_sources(reg: &mut Registry) {
    reg.register_source(SourceDef {
        info: demo::cpu_info(),
        start: cpu_start,
        demo: demo::cpu_demo,
    });
}
