//! Built-in source definitions by feature (§4.3). Arc 1b: the live cpu source
//! (procfs meters) with the seeded synth behind `--demo`.

use gridwatch_ui::Registry;

#[allow(unused_variables)] // with no source feature on, nothing registers
pub fn builtin_sources(reg: &mut Registry) {
    #[cfg(feature = "cpu")]
    reg.register_source(gridwatch_store::SourceDef {
        info: gridwatch_store::demo::cpu_info(),
        start: crate::cpu::start,
        demo: gridwatch_store::demo::cpu_demo,
    });
}
