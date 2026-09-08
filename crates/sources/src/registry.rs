//! Built-in source definitions by feature (§4.3): the live cpu (arc 1b) and
//! gpu (arc 2b) sources, each with its seeded synth behind `--demo`.
//!
//! Each `check` is the source's own `Options::from_table(t).1` — the reader
//! `start` runs, without starting anything (D63). The placeholder every
//! source registered until arc 13 is gone.

use gridwatch_ui::Registry;

#[allow(unused_variables)] // with no source feature on, nothing registers
pub fn builtin_sources(reg: &mut Registry) {
    #[cfg(feature = "cpu")]
    reg.register_source(gridwatch_store::SourceDef {
        info: gridwatch_store::demo::cpu_info(),
        start: crate::cpu::start,
        demo: gridwatch_store::demo::cpu_demo,
        options: crate::cpu::OPTION_NAMES,
        check: crate::cpu::check,
    });
    #[cfg(feature = "gpu")]
    reg.register_source(gridwatch_store::SourceDef {
        info: gridwatch_store::demo::gpu_info(),
        start: crate::gpu::start,
        demo: gridwatch_store::demo::gpu_demo,
        options: crate::gpu::OPTION_NAMES,
        check: crate::gpu::check,
    });
    #[cfg(feature = "pins")]
    reg.register_source(gridwatch_store::SourceDef {
        info: gridwatch_store::demo::pins_source_info(),
        start: crate::pins::start,
        demo: gridwatch_store::demo::pins_demo,
        options: crate::pins::OPTION_NAMES,
        check: crate::pins::check,
    });
    #[cfg(feature = "audio")]
    reg.register_source(gridwatch_store::SourceDef {
        info: gridwatch_store::demo::audio_info(),
        start: crate::audio::start,
        demo: gridwatch_store::demo::audio_demo,
        options: crate::audio::OPTION_NAMES,
        check: crate::audio::check,
    });
    #[cfg(feature = "sensors")]
    reg.register_source(gridwatch_store::SourceDef {
        info: gridwatch_store::demo::sensors_info_static(),
        start: crate::sensors::start,
        demo: gridwatch_store::demo::sensors_demo,
        options: crate::sensors::OPTION_NAMES,
        check: crate::sensors::check,
    });
    #[cfg(feature = "mpris")]
    reg.register_source(gridwatch_store::SourceDef {
        info: gridwatch_store::demo::media_info(),
        start: crate::mpris::start,
        demo: gridwatch_store::demo::media_demo,
        options: crate::mpris::OPTION_NAMES,
        check: crate::mpris::check,
    });
    #[cfg(feature = "net")]
    reg.register_source(gridwatch_store::SourceDef {
        info: gridwatch_store::demo::net_info(),
        start: crate::net::start,
        demo: gridwatch_store::demo::net_demo,
        options: crate::net::OPTION_NAMES,
        check: crate::net::check,
    });
}
