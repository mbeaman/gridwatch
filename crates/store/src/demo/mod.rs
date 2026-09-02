//! Seeded synthetic sources (§4.3, §12.5): `--demo` and the ui testkit share
//! this generator, so snapshots and demo mode can never drift apart.

mod procs;
mod synth;

pub use procs::{KERNEL_THREADS, proc_table};
pub use synth::{CpuSynth, XorShift, cpu_demo, cpu_info};
