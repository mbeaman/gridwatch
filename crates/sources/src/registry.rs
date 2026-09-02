//! Built-in source definitions by feature (§4.3): the live cpu (arc 1b) and
//! gpu (arc 2b) sources, each with its seeded synth behind `--demo`.

use gridwatch_ui::Registry;

#[allow(unused_variables)] // with no source feature on, nothing registers
pub fn builtin_sources(reg: &mut Registry) {
    #[cfg(feature = "cpu")]
    reg.register_source(gridwatch_store::SourceDef {
        info: gridwatch_store::demo::cpu_info(),
        start: crate::cpu::start,
        demo: gridwatch_store::demo::cpu_demo,
    });
    #[cfg(feature = "gpu")]
    reg.register_source(gridwatch_store::SourceDef {
        info: gridwatch_store::demo::gpu_info(),
        start: crate::gpu::start,
        demo: gridwatch_store::demo::gpu_demo,
    });
}
