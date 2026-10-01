//! World, zone, validation, and chat codecs for EverQuest-compatible servers.

/// Worn gear and features in EQ's texture slots, and wear changes.
pub mod appearance;
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
/// Titanium character creation requests and stat rules.
pub mod creation;
/// Server-defined interactive doors and their movement notifications.
pub mod doors;
/// Handing items to another character: the give and trade windows.
pub mod exchange;
/// Food and drink: hunger, thirst, and eating and drinking.
pub mod food;
/// Read-only inventory packets, slots, instances, and state.
pub mod inventory;
/// Read-only item inspection requests and definitions.
pub mod items;
/// Corpse looting requests, listings and acknowledgements.
pub mod loot;
/// Merchant windows, stock, purchases and sales.
pub mod merchant;
/// Coins on the cursor and in the bank, and moving coins between places.
pub mod money;
/// Movement wire layout and session-scoped motion validation.
pub mod movement;
/// Items on the ground and world containers.
pub mod objects;
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

/// What one zone packet says, read once for the whole session.
pub mod message;

use serde::{Deserialize, Serialize};

/// Game packet layout used by a client generation.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GameDialect {
    /// Titanium, as stock `EQEmu` servers speak it and Project 1999 does
    /// under its V62 protection.
    #[default]
    // Settings saved before the rename named it after Project 1999.
    #[serde(alias = "titanium_p99")]
    Titanium,
    /// The Windows TAKP/EQMac client used by Project Quarm.
    EqMac,
}

#[cfg(test)]
mod tests {
    use super::GameDialect;

    #[test]
    fn titanium_reads_its_old_name_and_writes_its_new_one() {
        let old: GameDialect = serde_json::from_str("\"titanium_p99\"").unwrap();
        assert_eq!(old, GameDialect::Titanium);
        assert_eq!(
            serde_json::to_string(&GameDialect::Titanium).unwrap(),
            "\"titanium\""
        );
    }
}
