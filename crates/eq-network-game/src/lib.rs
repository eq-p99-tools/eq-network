//! World, zone, validation, and chat codecs for EverQuest-compatible servers.

/// Admission buffs and server-driven buff changes.
pub mod buffs;
/// World-server character selection lists.
pub mod characters;
/// Communication packet codecs and structured item-link extraction.
pub mod chat;
/// Consider, auto-attack and combat-damage records.
pub mod combat;
/// Typed client actions and dialect-specific application packet encoding.
pub mod command;
/// Server-defined interactive doors and their movement notifications.
pub mod doors;
/// Read-only inventory packets, slots, instances, and state.
pub mod inventory;
/// Read-only item inspection requests and definitions.
pub mod items;
/// Corpse looting requests, listings and acknowledgements.
pub mod loot;
/// Merchant windows, stock, purchases and sales.
pub mod merchant;
/// Movement wire layout and session-scoped motion validation.
pub mod movement;
/// Titanium/P99-V62 world and zone validation codec.
pub mod p99;
/// TAKP/EQMac player and entity presentation codecs.
pub mod quarm;
/// Server-driven spellbook and casting notifications.
pub mod spells;
/// Renderer-independent world-state packet decoding.
pub mod world;
/// Death and server-directed zone transfer codecs.
pub mod zoning;

use serde::{Deserialize, Serialize};

/// Game packet layout used by a client generation.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GameDialect {
    /// Titanium with the Project 1999 V62 patch.
    #[default]
    TitaniumP99,
    /// The Windows TAKP/EQMac client used by Project Quarm.
    EqMac,
}
