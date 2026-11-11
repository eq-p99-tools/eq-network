//! World, zone, validation, and chat codecs for EverQuest-compatible servers.

/// Abilities: kick, bash, taunt, hide, sneak, forage and the like.
pub mod abilities;
/// How spawns look: worn gear, features, lights and the like, and the
/// updates, states and illusions that change them.
pub mod appearance;
/// A banker's change of the purse's and the bank's coins into the fewest
/// coins of the largest kinds.
pub mod banker;
/// Binding wounds with bandages.
pub mod bind_wound;
/// Reading books and notes.
pub mod books;
/// Admission buffs and server-driven buff changes.
pub mod buffs;
/// The chat server a world offers for channels and mail.
pub mod channels;
/// World-server character selection lists.
pub mod characters;
/// Communication packet codecs and structured item-link extraction.
pub mod chat;
/// The time of day in Norrath, and how a zone's sky and fog look.
pub mod clock;
/// Consider, auto-attack and combat-damage records.
pub mod combat;
/// Typed client actions and dialect-specific application packet encoding.
pub mod command;
/// The server's questions the player answers: a sacrifice and a translocate.
pub mod confirmations;
/// The server's hold on the player: stuns, charms, its own control of the
/// player, and a warrior's berserk.
pub mod control;
/// Players' corpses: consent, summoning and dragging.
pub mod corpses;
/// Titanium character creation requests and stat rules.
pub mod creation;
/// List-scoped character deletion intents, native requests and observed outcomes.
pub mod deletion;
/// The player's disciplines and their reuse timers, and the reuse timers
/// groups of spells share.
pub mod disciplines;
/// Server-defined interactive doors and their movement notifications.
pub mod doors;
/// Effects the servers play on a spawn, on the player's screen, as a sound
/// or as a projectile's flight.
pub mod effects;
/// Handing items to another character: the give and trade windows.
pub mod exchange;
/// Feign Death and the lying posture, by client generation.
pub mod feign;
/// Food and drink: hunger, thirst, and eating and drinking.
pub mod food;
/// Asking which of the player's friends are online.
pub mod friends;
/// What game masters and the world tell the zone of a player: kicks,
/// kills, and a player made to look like an NPC; and, to game masters, the
/// petition queue.
pub mod gm;
/// Groups: invitations, joining, leaving and disbanding, and who is in the
/// player's group.
pub mod group;
/// The player's guild: its member list, its members' changes and
/// invitations.
pub mod guild;
/// Damage the world does to the player, which the client reports: falls,
/// drowning, lava and freezing.
pub mod hazards;
/// Inspecting another player's equipment.
pub mod inspection;
/// Read-only inventory packets, slots, instances, and state.
pub mod inventory;
/// Read-only item inspection requests and definitions.
pub mod items;
/// Titanium Paladin and Shadow Knight class abilities.
pub mod knights;
pub mod list_poll;
/// How `/who` lists a player: guild and rank, anonymity and flags, and guild
/// names.
pub mod listing;
/// Corpse looting requests, listings and acknowledgements.
pub mod loot;
/// Merchant windows, stock, purchases and sales.
pub mod merchant;
/// Coins on the cursor and in the bank, and moving coins between places.
pub mod money;
/// Movement wire layout and session-scoped motion validation.
pub mod movement;
/// Titles, last names and suffixes: the parts of a name around the first;
/// and renames.
pub mod names;
/// Items on the ground and world containers.
pub mod objects;
/// Titanium/P99-V62 world and zone validation codec.
pub mod p99;
/// Pets: commands, owners and buffs.
pub mod pets;
/// What the server puts on the player's screen outside the chat window:
/// popups and marquees.
pub mod popups;
/// TAKP/EQMac player and entity presentation codecs.
pub mod quarm;
/// Raids: invitations, joining, leaving, and the server's word on the raid.
pub mod raid;
/// How far each client generation lets the player reach doors, objects,
/// merchants and the one they bandage.
pub mod reach;
/// Being resurrected: the offer and the player's answer.
pub mod resurrection;
/// What a quest or a task gives the player.
pub mod rewards;
/// A world's rules, and the expansions it lets the account play.
pub mod rules;
/// The worlds a login server lists, and the player's choice among them.
pub mod servers;
/// Rolling dice, emoting and assisting; and other players' yells for help,
/// voice macros, requests to inspect the player and duels.
pub mod socials;
/// P99 spell-ID registration and cached-definition replay records and events.
pub mod spell_tables;
/// Server-driven spellbook and casting notifications.
pub mod spells;
/// Splitting coins with the player's group, at once or as they are looted.
pub mod split;
/// A quest's word on the player's base stats.
pub mod stats;
/// What machine the player enters the world on, as the world is told it.
pub mod sysinfo;
/// The player's target as the server sets it, and whom that target targets.
pub mod targets;
/// The server's regeneration tick and how often it comes.
pub mod ticks;
/// Tracking: the server's list and following a spawn.
pub mod tracking;
/// Tradeskill combines in the player's own containers.
pub mod tradeskills;
/// Training skills at a guildmaster.
pub mod training;
/// Received invisibility codes and observer eligibility.
pub mod vision;
/// Who is online: `/who all` and the world's answer.
pub mod who;
/// Renderer-independent world-state packet decoding.
pub mod world;
/// Zone numbers and short names.
pub mod zones;
/// Death and server-directed zone transfer codecs.
pub mod zoning;

/// What one zone packet says, read once for the whole session.
pub mod message;
/// What the zone session asks of the server, and the Titanium packet for it.
pub mod request;

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

impl GameDialect {
    /// Whether the session accepts a completed absolute facing with no
    /// continuing turn hint in its movement requests.
    #[must_use]
    pub const fn supports_heading_snapshots(self) -> bool {
        matches!(self, Self::Titanium)
    }

    /// Whether carrying capacity depends on the current fatigue and stamina.
    #[must_use]
    pub const fn uses_fatigue_for_carry_capacity(self) -> bool {
        matches!(self, Self::EqMac)
    }

    /// Whether an ordinary door click requires a current inventory and an
    /// empty cursor.
    #[must_use]
    pub const fn requires_clear_cursor_for_doors(self) -> bool {
        matches!(self, Self::EqMac)
    }

    /// How this generation's program treats Feign Death and a lying player
    /// ([`feign::LyingRules`]).
    #[must_use]
    pub const fn lying(self) -> feign::LyingRules {
        match self {
            Self::Titanium => feign::TITANIUM,
            Self::EqMac => feign::EQMAC,
        }
    }

    /// Whether a change of the player's posture from standing turns auto
    /// attack off. Titanium's posture routine does not, for any posture
    /// (program read R19cd 3): auto attack stays on through a sit, a duck
    /// or a lie, the attack key alone refusing to turn it on (line 0x3051),
    /// and the server decides the swings; its Feign Death press turns it
    /// off ([`feign::LyingRules::press_ends_attack`]). The Mac-era client's
    /// program is unread, so it keeps this library's earlier rule that any
    /// posture but standing ends auto attack (recording checklist 6.455).
    #[must_use]
    pub const fn posture_ends_attack(self) -> bool {
        matches!(self, Self::EqMac)
    }

    /// The slot finder this generation's program runs for a buff landing on
    /// the player ([`buffs::placement`]): Titanium's (program reads R3 and
    /// R20); none for the Mac-era generation, whose program is unread, so
    /// its landings stay unslotted as before.
    #[must_use]
    pub fn buff_placement(self) -> Option<buffs::placement::Finder> {
        match self {
            Self::Titanium => Some(buffs::placement::titanium_player),
            Self::EqMac => None,
        }
    }

    /// How this generation's sit/stand toggle chooses
    /// ([`feign::SitToggle`]).
    #[must_use]
    pub const fn sit_toggle(self) -> feign::SitToggle {
        match self {
            Self::Titanium => feign::TITANIUM_SIT_TOGGLE,
            Self::EqMac => feign::EQMAC_SIT_TOGGLE,
        }
    }

    /// Whether asking for the posture the player already has sends nothing.
    /// Titanium's posture routine makes no change then (program read R19,
    /// 0x483556). The Mac-era client's is read only for lying, so it keeps
    /// this library's earlier rule of sending every request but a second lie
    /// (recording checklist 6.455).
    #[must_use]
    pub const fn skips_unchanged_posture(self) -> bool {
        matches!(self, Self::Titanium)
    }

    /// Whether a spell with the feign effect (74) that took hold on the
    /// player ([`buffs::SpellEffect::took_hold`]) lies them down.
    /// Titanium's landing sends effect 74 to 0x40db88, which is the
    /// posture routine with 115 for the player's own spawn, with no
    /// feign request, no auto attack change and no success test (program
    /// read R21). The Mac-era client's is unread, so it keeps this library's
    /// earlier rule of leaving the posture alone (recording checklist 6.455).
    #[must_use]
    pub const fn spell_feign_lies(self) -> bool {
        matches!(self, Self::Titanium)
    }
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

    #[test]
    fn only_the_unread_mac_client_ends_auto_attack_on_a_posture() {
        assert!(!GameDialect::Titanium.posture_ends_attack());
        assert!(GameDialect::EqMac.posture_ends_attack());
    }

    #[test]
    fn only_titanium_places_a_landing_buff() {
        let held = std::collections::BTreeMap::new();
        let band = Some(crate::buffs::placement::Band::default());
        let finder = GameDialect::Titanium.buff_placement().unwrap();
        assert_eq!(finder(&held, 278, band, false), Ok(0));
        assert!(GameDialect::EqMac.buff_placement().is_none());
    }

    #[test]
    fn only_titaniums_sit_toggle_is_read() {
        let titanium = GameDialect::Titanium.sit_toggle();
        assert!(titanium.stands_lying_and_looting);
        assert_eq!(titanium.mesmerize_refusal, Some(0x3050));
        let eqmac = GameDialect::EqMac.sit_toggle();
        assert!(!eqmac.stands_lying_and_looting);
        assert_eq!(eqmac.mesmerize_refusal, None);
    }

    #[test]
    fn only_titaniums_spell_feign_is_read() {
        assert!(GameDialect::Titanium.spell_feign_lies());
        assert!(!GameDialect::EqMac.spell_feign_lies());
    }

    #[test]
    fn only_titanium_is_read_to_skip_an_unchanged_posture() {
        assert!(GameDialect::Titanium.skips_unchanged_posture());
        assert!(!GameDialect::EqMac.skips_unchanged_posture());
    }
}
