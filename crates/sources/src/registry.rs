//! Built-in source definitions by feature (§4.3). Arc 1b: the live cpu source
//! (procfs meters) with the seeded synth behind `--demo`.

use gridwatch_store::{SourceDef, demo};
use gridwatch_ui::Registry;

pub fn builtin_sources(reg: &mut Registry) {
    reg.register_source(SourceDef {
        info: demo::cpu_info(),
        start: crate::cpu::start,
        demo: demo::cpu_demo,
    });
}
