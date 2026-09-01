//! Built-in component definitions by feature (§4.6). Arc 1a: clock + sources.

use gridwatch_ui::Registry;

pub fn builtin_components(reg: &mut Registry) {
    reg.register_component((crate::clock::DEF)());
    reg.register_component((crate::sources_tile::DEF)());
}
