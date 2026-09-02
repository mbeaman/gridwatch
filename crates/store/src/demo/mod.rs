//! Seeded synthetic sources (§4.3, §12.5): `--demo` and the ui testkit share
//! this generator, so snapshots and demo mode can never drift apart.

mod audio;
mod gpu;
mod pins;
mod procs;
mod synth;

pub use audio::{AudioSynth, audio_demo, audio_info, audio_sink, band_of};
pub use gpu::{GpuSynth, gpu_demo, gpu_info, gpu_procs};
pub use pins::{
    OVERLOAD_RAISE_S, OVERLOAD_RESOLVE_S, PinsSynth, pins_demo, pins_info, pins_source_info,
};
pub use procs::{KERNEL_THREADS, proc_table};
pub use synth::{CpuSynth, XorShift, cpu_demo, cpu_info};
