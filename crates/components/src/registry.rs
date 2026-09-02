//! Built-in component definitions by feature (§4.6): htop (1b), gpu (2b), clock, sources.

use gridwatch_ui::Registry;

pub fn builtin_components(reg: &mut Registry) {
    #[cfg(feature = "htop")]
    reg.register_component((crate::htop::DEF)());
    #[cfg(feature = "gpu")]
    reg.register_component((crate::gpu::DEF)());
    #[cfg(feature = "pins")]
    reg.register_component((crate::pins::DEF)());
    #[cfg(feature = "audio")]
    reg.register_component((crate::audio::DEF)());
    reg.register_component((crate::alerts::DEF)());
    reg.register_component((crate::clock::DEF)());
    reg.register_component((crate::sources_tile::DEF)());
}
