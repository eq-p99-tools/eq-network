//! The in-game map. A front end draws it from the installation's own map
//! files and the player's position, so the session sends and hears nothing
//! for it. A server type offers it, or leaves it to the player where its own
//! client keeps the map off, as P99's does.
use super::feature::Feature;

/// Lets the player open the in-game map.
pub(super) struct Map;

impl Feature for Map {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Map]
    }
}
