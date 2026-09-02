//! The metric vocabulary (§4.1): one module per domain, `SOURCE` constants next
//! to their keys, Record types with their journal decoders.

pub mod audio;
pub mod cpu;
pub mod gpu;
pub mod media;
pub mod net;
pub mod pins;
pub mod sensors;
pub mod sys;
