//! The server filters: which kinds of message the player asks the server not
//! to send. The client sends one packet of words, one word per kind, at
//! zone-in (Titanium `OP_SetServerFilter` 0x6563, 29 words; `EQMac` 0xff41,
//! 17 words), and the servers read it again whenever it comes.
//!
//! Each word's place and what its values mean to the server are read from
//! the servers that take it: stock `EQEmu`'s `Client::ServerFilter` and its
//! `eqFilterType`, and TAKP's `Client::ServerFilter` and
//! `UpdateFilters_Struct`. Which of the official client's options sets each
//! word, the values those options give it, and when the official client
//! sends it again are not read yet (recording checklist 6.494b and c; read
//! request R49 part 2). So the kinds are named for what the servers do with
//! them, and `ServerFilters::default()` is the packet each generation's
//! client is known to send today, byte for byte.

use crate::{command::EncodedCommand, GameDialect};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Titanium's `OP_SetServerFilter`.
pub const TITANIUM_OPCODE: u16 = 0x6563;

/// Titanium's packet: 29 words.
const TITANIUM_WORDS: usize = 29;

/// `EQMac`'s packet: 17 words.
const EQMAC_WORDS: usize = 17;

/// A kind of message a server filter word covers, as the servers name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterCategory {
    /// Guild chat.
    GuildChat,
    /// Other players' social emotes.
    Socials,
    /// Group chat.
    GroupChat,
    /// Shouts.
    Shouts,
    /// Auctions.
    Auctions,
    /// Out-of-character chat.
    Ooc,
    /// The server's bad-word word. Only Titanium's packet has it.
    BadWords,
    /// Other players' spells.
    PcSpells,
    /// Creatures' spells.
    NpcSpells,
    /// Bard songs.
    BardSongs,
    /// Critical spell hits.
    SpellCrits,
    /// Critical melee hits.
    MeleeCrits,
    /// Spell damage. Only Titanium's packet has it.
    SpellDamage,
    /// The player's own misses.
    MyMisses,
    /// Other players' misses.
    OthersMisses,
    /// Other players' hits.
    OthersHits,
    /// Attacks that miss the player.
    MissedMe,
    /// Damage shields.
    DamageShields,
    /// Damage over time. Only Titanium's packet has it.
    DamageOverTime,
    /// Pets' hits. Only Titanium's packet has it.
    PetHits,
    /// Pets' misses. Only Titanium's packet has it.
    PetMisses,
    /// Focus effects. Only Titanium's packet has it.
    FocusEffects,
    /// Pets' spells. Only Titanium's packet has it.
    PetSpells,
    /// Heals over time. Only Titanium's packet has it.
    HealOverTime,
}

/// What the server does with one kind of message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterMode {
    /// Sends every such message.
    Show,
    /// Sends none.
    Hide,
    /// Sends only the player's own.
    MineOnly,
    /// Sends only those from the player's group.
    GroupOnly,
}

use FilterCategory as C;
use FilterMode::{GroupOnly, Hide, MineOnly, Show};

/// A word the server reads as "1 shows, 0 hides".
const ONE_SHOWS: &[(FilterMode, u32)] = &[(Show, 1), (Hide, 0)];
/// A word the server reads as "0 shows, anything else hides".
const ZERO_SHOWS: &[(FilterMode, u32)] = &[(Show, 0), (Hide, 1)];
/// Other players' spells: 0 shows, 1 hides, 2 shows the group's.
const GROUP_SPELLS: &[(FilterMode, u32)] = &[(Show, 0), (Hide, 1), (GroupOnly, 2)];
/// Bard songs: 0 shows, 1 the player's own, 2 the group's, 3 hides.
const SONGS: &[(FilterMode, u32)] = &[(Show, 0), (MineOnly, 1), (GroupOnly, 2), (Hide, 3)];
/// Crits and spell damage: 0 shows, 1 the player's own, 2 hides.
const MINE: &[(FilterMode, u32)] = &[(Show, 0), (MineOnly, 1), (Hide, 2)];
/// Titanium's damage over time: `EQEmu` reads 0 as the player's own (for a
/// client before `SoD`) and anything else as hidden.
const TITANIUM_DOT: &[(FilterMode, u32)] = &[(MineOnly, 0), (Hide, 1)];

impl FilterCategory {
    /// Every kind, in the order a file or a list shows them.
    pub const ALL: [Self; 24] = [
        C::GuildChat,
        C::Socials,
        C::GroupChat,
        C::Shouts,
        C::Auctions,
        C::Ooc,
        C::BadWords,
        C::PcSpells,
        C::NpcSpells,
        C::BardSongs,
        C::SpellCrits,
        C::MeleeCrits,
        C::SpellDamage,
        C::MyMisses,
        C::OthersMisses,
        C::OthersHits,
        C::MissedMe,
        C::DamageShields,
        C::DamageOverTime,
        C::PetHits,
        C::PetMisses,
        C::FocusEffects,
        C::PetSpells,
        C::HealOverTime,
    ];

    /// The kind's stable name, as a file or a command spells it.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            C::GuildChat => "guild_chat",
            C::Socials => "socials",
            C::GroupChat => "group_chat",
            C::Shouts => "shouts",
            C::Auctions => "auctions",
            C::Ooc => "ooc",
            C::BadWords => "bad_words",
            C::PcSpells => "pc_spells",
            C::NpcSpells => "npc_spells",
            C::BardSongs => "bard_songs",
            C::SpellCrits => "spell_crits",
            C::MeleeCrits => "melee_crits",
            C::SpellDamage => "spell_damage",
            C::MyMisses => "my_misses",
            C::OthersMisses => "others_misses",
            C::OthersHits => "others_hits",
            C::MissedMe => "missed_me",
            C::DamageShields => "damage_shields",
            C::DamageOverTime => "damage_over_time",
            C::PetHits => "pet_hits",
            C::PetMisses => "pet_misses",
            C::FocusEffects => "focus_effects",
            C::PetSpells => "pet_spells",
            C::HealOverTime => "heal_over_time",
        }
    }

    /// The kind a stable name names.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|category| category.key() == key)
    }

    /// The mode the client is known to send today, which
    /// `ServerFilters::default()` keeps: shown, except damage over time,
    /// whose Titanium 0 `EQEmu` reads as the player's own.
    #[must_use]
    pub const fn default_mode(self) -> FilterMode {
        match self {
            C::DamageOverTime => MineOnly,
            _ => Show,
        }
    }

    /// Where the kind's word sits in the generation's packet and the value
    /// each mode it takes is sent as; `None` where the packet has no such
    /// word.
    const fn word(self, dialect: GameDialect) -> Option<(usize, &'static [(FilterMode, u32)])> {
        match dialect {
            GameDialect::Titanium => Some(self.titanium_word()),
            GameDialect::EqMac => self.eqmac_word(),
            // A placeholder generation sends no filters (6.489).
            GameDialect::Rof2 => None,
        }
    }

    /// Titanium's 29 words, as `EQEmu`'s `eqFilterType` places them and its
    /// `Client::ServerFilter` reads them. Word 0 and words 25 to 28 are
    /// named by no kind here and stay 0.
    const fn titanium_word(self) -> (usize, &'static [(FilterMode, u32)]) {
        match self {
            C::GuildChat => (1, ONE_SHOWS),
            C::Socials => (2, ONE_SHOWS),
            C::GroupChat => (3, ONE_SHOWS),
            C::Shouts => (4, ONE_SHOWS),
            C::Auctions => (5, ONE_SHOWS),
            C::Ooc => (6, ONE_SHOWS),
            C::BadWords => (7, ONE_SHOWS),
            C::PcSpells => (8, GROUP_SPELLS),
            C::NpcSpells => (9, ZERO_SHOWS),
            C::BardSongs => (10, SONGS),
            C::SpellCrits => (11, MINE),
            C::MeleeCrits => (12, MINE),
            C::SpellDamage => (13, MINE),
            C::MyMisses => (14, ONE_SHOWS),
            C::OthersMisses => (15, ONE_SHOWS),
            C::OthersHits => (16, ONE_SHOWS),
            C::MissedMe => (17, ONE_SHOWS),
            C::DamageShields => (18, ZERO_SHOWS),
            C::DamageOverTime => (19, TITANIUM_DOT),
            C::PetHits => (20, ZERO_SHOWS),
            C::PetMisses => (21, ZERO_SHOWS),
            C::FocusEffects => (22, ZERO_SHOWS),
            C::PetSpells => (23, ZERO_SHOWS),
            C::HealOverTime => (24, ZERO_SHOWS),
        }
    }

    /// `EQMac`'s 17 words, as TAKP's `UpdateFilters_Struct` places them and
    /// its `Client::ServerFilter` reads them. Word 4, a separate "server
    /// filter" switch whose effect is unread, stays 0.
    const fn eqmac_word(self) -> Option<(usize, &'static [(FilterMode, u32)])> {
        Some(match self {
            C::DamageShields => (0, ZERO_SHOWS),
            C::NpcSpells => (1, ZERO_SHOWS),
            C::PcSpells => (2, GROUP_SPELLS),
            C::BardSongs => (3, SONGS),
            C::GuildChat => (5, ONE_SHOWS),
            C::Socials => (6, ONE_SHOWS),
            C::GroupChat => (7, ONE_SHOWS),
            C::Shouts => (8, ONE_SHOWS),
            C::Auctions => (9, ONE_SHOWS),
            C::Ooc => (10, ONE_SHOWS),
            C::MyMisses => (11, ONE_SHOWS),
            C::OthersMisses => (12, ONE_SHOWS),
            C::OthersHits => (13, ONE_SHOWS),
            C::MissedMe => (14, ONE_SHOWS),
            C::SpellCrits => (15, MINE),
            C::MeleeCrits => (16, MINE),
            C::BadWords
            | C::SpellDamage
            | C::DamageOverTime
            | C::PetHits
            | C::PetMisses
            | C::FocusEffects
            | C::PetSpells
            | C::HealOverTime => return None,
        })
    }

    /// The modes the generation's packet can say for this kind, in the
    /// order a list shows them; empty where the packet has no word for it.
    #[must_use]
    pub fn modes(self, dialect: GameDialect) -> Vec<FilterMode> {
        self.word(dialect)
            .map(|(_, values)| values.iter().map(|(mode, _)| *mode).collect())
            .unwrap_or_default()
    }
}

impl FilterMode {
    /// The mode's stable name, as a file or a command spells it.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Show => "show",
            Hide => "hide",
            MineOnly => "mine",
            GroupOnly => "group",
        }
    }

    /// The mode a stable name names.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        [Show, Hide, MineOnly, GroupOnly]
            .into_iter()
            .find(|mode| mode.key() == key)
    }
}

/// A mode for every kind of message: what the client asks the server.
/// Kinds left unset keep [`FilterCategory::default_mode`], so the default
/// is the packet sent today.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerFilters {
    /// The kinds set away from their default.
    modes: BTreeMap<FilterCategory, FilterMode>,
}

impl ServerFilters {
    /// The mode for a kind.
    #[must_use]
    pub fn get(&self, category: FilterCategory) -> FilterMode {
        self.modes
            .get(&category)
            .copied()
            .unwrap_or_else(|| category.default_mode())
    }

    /// Sets the mode for a kind. Whether a generation can say it is checked
    /// when the packet is built ([`encode`]).
    pub fn set(&mut self, category: FilterCategory, mode: FilterMode) {
        if mode == category.default_mode() {
            self.modes.remove(&category);
        } else {
            self.modes.insert(category, mode);
        }
    }

    /// These filters as the generation's packet can say them: a mode its
    /// word cannot carry goes back to the kind's default, so the result
    /// always passes [`Self::check`] (on a generation that sends filters).
    #[must_use]
    pub fn fitted(&self, dialect: GameDialect) -> Self {
        let mut fitted = Self::default();
        for (category, mode) in &self.modes {
            if category.modes(dialect).contains(mode) {
                fitted.set(*category, *mode);
            }
        }
        fitted
    }

    /// Refuses a mode the generation's packet cannot say for its kind. A
    /// kind the packet has no word for is left out, whatever its mode.
    ///
    /// # Errors
    /// Names the first kind whose mode the packet cannot carry, and refuses
    /// a generation that sends no filters yet.
    pub fn check(&self, dialect: GameDialect) -> Result<()> {
        words(self, dialect).map(|_| ())
    }
}

/// The packet's words for the generation.
fn words(filters: &ServerFilters, dialect: GameDialect) -> Result<Vec<u32>> {
    let count = match dialect {
        GameDialect::Titanium => TITANIUM_WORDS,
        GameDialect::EqMac => EQMAC_WORDS,
        GameDialect::Rof2 => bail!("not built for RoF2 yet"),
    };
    let mut words = vec![0; count];
    for category in FilterCategory::ALL {
        let Some((index, values)) = category.word(dialect) else {
            continue;
        };
        let mode = filters.get(category);
        let Some((_, value)) = values.iter().find(|(taken, _)| *taken == mode) else {
            bail!(
                "the {dialect:?} filters cannot say {} for {}",
                mode.key(),
                category.key()
            );
        };
        words[index] = *value;
    }
    Ok(words)
}

/// The generation's filter packet for these modes.
///
/// # Errors
/// Refuses a mode the packet cannot say for its kind (see
/// [`ServerFilters::check`]), and a generation that sends no filters yet.
pub fn encode(dialect: GameDialect, filters: &ServerFilters) -> Result<EncodedCommand> {
    let opcode = match dialect {
        GameDialect::Titanium => TITANIUM_OPCODE,
        GameDialect::EqMac => crate::quarm::ZONE_SERVER_FILTER,
        GameDialect::Rof2 => bail!("not built for RoF2 yet"),
    };
    let body = words(filters, dialect)?
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    Ok(EncodedCommand { opcode, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(body: &[u8], index: usize) -> u32 {
        u32::from_le_bytes(body[index * 4..index * 4 + 4].try_into().unwrap())
    }

    #[test]
    fn the_defaults_are_the_packets_each_generation_sends_today() {
        // Titanium: the stock client's packet, words 1-7 and 14-17 set.
        let titanium = encode(GameDialect::Titanium, &ServerFilters::default()).unwrap();
        assert_eq!(titanium.opcode, 0x6563);
        assert_eq!(titanium.body.len(), 116);
        for index in 0..29 {
            let set = (1..=7).contains(&index) || (14..=17).contains(&index);
            assert_eq!(word(&titanium.body, index), u32::from(set), "word {index}");
        }
        // EQMac: words 5-14 set.
        let eqmac = encode(GameDialect::EqMac, &ServerFilters::default()).unwrap();
        assert_eq!(eqmac.opcode, 0xff41);
        assert_eq!(eqmac.body.len(), 68);
        for index in 0..17 {
            assert_eq!(
                word(&eqmac.body, index),
                u32::from((5..=14).contains(&index)),
                "word {index}"
            );
        }
        assert!(encode(GameDialect::Rof2, &ServerFilters::default()).is_err());
    }

    #[test]
    fn each_mode_goes_as_the_servers_read_it() {
        let mut filters = ServerFilters::default();
        filters.set(C::GuildChat, Hide);
        filters.set(C::PcSpells, GroupOnly);
        filters.set(C::BardSongs, MineOnly);
        filters.set(C::MeleeCrits, Hide);
        filters.set(C::DamageShields, Hide);
        filters.set(C::DamageOverTime, Hide);
        let titanium = encode(GameDialect::Titanium, &filters).unwrap().body;
        assert_eq!(
            [1, 8, 10, 12, 18, 19].map(|index| word(&titanium, index)),
            [0, 2, 1, 2, 1, 1]
        );
        let eqmac = encode(GameDialect::EqMac, &filters).unwrap().body;
        assert_eq!(
            [5, 2, 3, 16, 0].map(|index| word(&eqmac, index)),
            [0, 2, 1, 2, 1]
        );
        // EQMac has no damage-over-time word: the mode is left out.
        assert_eq!(word(&eqmac, 4), 0);
    }

    #[test]
    fn a_mode_a_word_cannot_say_is_refused() {
        let mut filters = ServerFilters::default();
        filters.set(C::MeleeCrits, GroupOnly);
        assert!(filters.check(GameDialect::Titanium).is_err());
        assert!(filters.check(GameDialect::EqMac).is_err());
        let mut filters = ServerFilters::default();
        filters.set(C::DamageOverTime, Show);
        assert!(filters.check(GameDialect::Titanium).is_err());
        // A kind the packet lacks takes any mode.
        assert!(filters.check(GameDialect::EqMac).is_ok());
    }

    #[test]
    fn a_set_fits_each_generation() {
        let mut filters = ServerFilters::default();
        filters.set(C::DamageOverTime, Hide);
        filters.set(C::MeleeCrits, GroupOnly);
        filters.set(C::Shouts, Hide);
        let titanium = filters.fitted(GameDialect::Titanium);
        assert_eq!(titanium.get(C::DamageOverTime), Hide);
        assert_eq!(titanium.get(C::MeleeCrits), Show);
        assert_eq!(titanium.get(C::Shouts), Hide);
        assert!(titanium.check(GameDialect::Titanium).is_ok());
        // EQMac has no damage-over-time word: back to the default.
        let eqmac = filters.fitted(GameDialect::EqMac);
        assert_eq!(eqmac.get(C::DamageOverTime), MineOnly);
        assert!(eqmac.check(GameDialect::EqMac).is_ok());
        assert_eq!(filters.fitted(GameDialect::Rof2), ServerFilters::default());
    }

    #[test]
    fn defaults_are_not_kept_and_names_read_back() {
        let mut filters = ServerFilters::default();
        filters.set(C::Shouts, Hide);
        filters.set(C::Shouts, Show);
        assert_eq!(filters, ServerFilters::default());
        for category in FilterCategory::ALL {
            assert_eq!(FilterCategory::from_key(category.key()), Some(category));
            assert!(category
                .modes(GameDialect::Titanium)
                .contains(&category.default_mode()));
            assert_eq!(category.modes(GameDialect::Rof2), Vec::new());
        }
        for mode in [Show, Hide, MineOnly, GroupOnly] {
            assert_eq!(FilterMode::from_key(mode.key()), Some(mode));
        }
        assert_eq!(C::SpellDamage.modes(GameDialect::EqMac), Vec::new());
        assert_eq!(C::BardSongs.modes(GameDialect::EqMac).len(), 4);
    }

    #[test]
    fn filters_keep_through_serde() {
        let mut filters = ServerFilters::default();
        filters.set(C::Ooc, Hide);
        let text = serde_json::to_string(&filters).unwrap();
        assert_eq!(text, r#"{"modes":{"ooc":"hide"}}"#);
        assert_eq!(
            serde_json::from_str::<ServerFilters>(&text).unwrap(),
            filters
        );
        assert_eq!(
            serde_json::from_str::<ServerFilters>("{}").unwrap(),
            ServerFilters::default()
        );
    }
}
