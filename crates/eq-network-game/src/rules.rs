//! What a world tells the client of its rules as the player logs in
//! (`OP_LogServer`), and which expansions it lets the account play
//! (`OP_ExpansionInfo`). Quarm's zones send their part of the rules again at
//! every zone-in, saying whether that zone allows player-versus-player
//! combat.
//!
//! Layout reference: `EQEmu`'s `LogServer_Struct` (`common/eq_packet_structs.h`),
//! which reaches the Titanium client as it is; TAKP's and Quarm's
//! `LogServer_Struct` and Quarm's `RuleSets_Struct`; and `EQEmu`'s
//! `ExpansionBitmask` (`common/emu_versions.h`) for the expansions.

use anyhow::{ensure, Result};
use serde::Serialize;

/// Titanium's `OP_LogServer`, from the world as the player logs in.
pub const TITANIUM_LOG_SERVER_OPCODE: u16 = 0x0fa6;
/// `EQMac`'s `OP_LogServer`: from the world as the player logs in, and on
/// Quarm from the zone at every zone-in.
pub const EQMAC_LOG_SERVER_OPCODE: u16 = 0xc341;
/// Titanium's `OP_ExpansionInfo`, from the world before the character list.
pub const TITANIUM_EXPANSIONS_OPCODE: u16 = 0x04ec;
/// `EQMac`'s `OP_ExpansionInfo`, from the world before the character list.
pub const EQMAC_EXPANSIONS_OPCODE: u16 = 0xd841;

/// The length of Titanium's log-server settings.
const TITANIUM_LENGTH: usize = 264;
/// The length of `EQMac`'s log-server settings from the world.
const EQMAC_WORLD_LENGTH: usize = 244;
/// The length of the rules Quarm's zones send (`RuleSets_Struct`).
const EQMAC_ZONE_LENGTH: usize = 24;

/// A world's rules, as far as it tells the client of them. Each generation's
/// copy carries its own set beside the two every copy carries; what a copy
/// does not carry is `None`.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct WorldRules {
    /// The world's player-versus-player ruleset, by the server's own number;
    /// zero for none. Quarm's zones send 1 where the zone itself allows it.
    pub pvp: u32,
    /// Whether the player may trade no-drop items.
    pub no_drop_trades: bool,
    /// Titanium: whether the petition window is open to players.
    pub petition_window: Option<bool>,
    /// Titanium: whether voice macros are enabled.
    pub voice_macros: Option<bool>,
    /// Titanium: whether the world runs mail.
    pub mail: Option<bool>,
    /// Titanium: whether the tutorial offers its way home (`EQEmu` sends the
    /// opposite, `disable_tutorial_go_home`).
    pub tutorial_way_home: Option<bool>,
    /// `EQMac`: the flag TAKP's struct names `auto_identify`, which TAKP and
    /// Quarm leave off.
    pub auto_identify: Option<bool>,
    /// `EQMac`: whether character creation offers its name generator.
    pub name_generator: Option<bool>,
    /// `EQMac`: the flag TAKP's struct names `Gibberish`, which TAKP and
    /// Quarm set.
    pub gibberish: Option<bool>,
    /// `EQMac`: whether the world is a test server.
    pub test_server: Option<bool>,
    /// `EQMac`'s world copy: the world's locale number.
    pub locale: Option<u32>,
    /// `EQMac`'s world copy: whether the world filters profanity.
    pub profanity_filter: Option<bool>,
}

/// Reads Titanium's log-server settings (`EQEmu`'s `LogServer_Struct`, 264
/// bytes): the player-versus-player ruleset at 4 and no-drop trading at 8, one byte each,
/// then one byte each from 260 for the petition window, voice macros, mail
/// and the tutorial's way home turned off. The world's short name at 32 is
/// [`crate::world::titanium_world_name`]'s.
///
/// # Errors
/// Rejects any other length.
pub fn titanium_rules(body: &[u8]) -> Result<WorldRules> {
    ensure!(
        body.len() == TITANIUM_LENGTH,
        "invalid log-server settings length"
    );
    Ok(WorldRules {
        pvp: u32::from(body[4]),
        no_drop_trades: body[8] != 0,
        petition_window: Some(body[260] != 0),
        voice_macros: Some(body[261] != 0),
        mail: Some(body[262] != 0),
        tutorial_way_home: Some(body[263] == 0),
        ..WorldRules::default()
    })
}

/// Reads `EQMac`'s rules: the world's log-server settings (244 bytes) or the
/// part of them Quarm's zones send (`RuleSets_Struct`, 24 bytes). Both open
/// with six 32-bit words: no-drop trading, the player-versus-player ruleset, auto-identify,
/// the name generator, gibberish and the test server; the world's copy goes
/// on with the locale and the profanity filter. The world's short name at 32
/// is [`eqmac_world_name`]'s; the address, port, password and e-mail of the
/// server the client would report errors to are left unread, since nothing
/// here reports to it.
///
/// # Errors
/// Rejects any other length.
pub fn eqmac_rules(body: &[u8]) -> Result<WorldRules> {
    let from_world = body.len() == EQMAC_WORLD_LENGTH;
    ensure!(
        from_world || body.len() == EQMAC_ZONE_LENGTH,
        "invalid EQMac log-server settings length"
    );
    let flag = |offset| word(body, offset) != 0;
    Ok(WorldRules {
        pvp: word(body, 4),
        no_drop_trades: flag(0),
        auto_identify: Some(flag(8)),
        name_generator: Some(flag(12)),
        gibberish: Some(flag(16)),
        test_server: Some(flag(20)),
        locale: from_world.then(|| word(body, 24)),
        profanity_filter: from_world.then(|| flag(28)),
        ..WorldRules::default()
    })
}

/// Reads the world's short name from `EQMac`'s log-server settings, which
/// keep it where Titanium's do: NUL-terminated in 32 bytes at 32.
///
/// # Errors
/// Rejects any length but the world's copy, and names that are empty or not
/// plain ASCII.
pub fn eqmac_world_name(body: &[u8]) -> Result<String> {
    ensure!(
        body.len() == EQMAC_WORLD_LENGTH,
        "invalid EQMac log-server settings length"
    );
    crate::world::titanium_world_name(body)
}

/// The expansions a world lets the account play: one bit each, in the order
/// they came out, the first expansion in the lowest bit.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Expansions(pub u32);

impl Expansions {
    /// Whether the account may play the `nth` expansion, counting the first
    /// as 1.
    #[must_use]
    pub const fn includes(self, nth: u32) -> bool {
        nth >= 1 && nth <= u32::BITS && self.0 & (1 << (nth - 1)) != 0
    }
}

/// Reads `OP_ExpansionInfo`, the same in both generations
/// (`ExpansionInfo_Struct`): the expansions as one 32-bit mask.
///
/// # Errors
/// Rejects any length but 4.
pub fn expansions(body: &[u8]) -> Result<Expansions> {
    ensure!(body.len() == 4, "invalid expansion mask length");
    Ok(Expansions(word(body, 0)))
}

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("layout checked by caller"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titaniums_settings_carry_its_own_rules() {
        let mut body = [0_u8; 264];
        body[4] = 2;
        body[8] = 1;
        body[32..41].copy_from_slice(b"testworld");
        body[260] = 1;
        body[262] = 1;
        assert_eq!(
            titanium_rules(&body).unwrap(),
            WorldRules {
                pvp: 2,
                no_drop_trades: true,
                petition_window: Some(true),
                voice_macros: Some(false),
                mail: Some(true),
                tutorial_way_home: Some(true),
                ..WorldRules::default()
            }
        );
        body[263] = 1;
        assert_eq!(
            titanium_rules(&body).unwrap().tutorial_way_home,
            Some(false)
        );
        assert!(titanium_rules(&body[..263]).is_err());
        assert!(titanium_rules(&[0; 265]).is_err());
    }

    #[test]
    fn eqmacs_world_and_zone_copies_share_their_first_six_words() {
        let mut world = [0_u8; 244];
        for (offset, value) in [(0, 1), (4, 4), (12, 1), (16, 1), (24, 3), (28, 1)] {
            world[offset..offset + 4].copy_from_slice(&u32::to_le_bytes(value));
        }
        world[32..41].copy_from_slice(b"testworld");
        assert_eq!(
            eqmac_rules(&world).unwrap(),
            WorldRules {
                pvp: 4,
                no_drop_trades: true,
                auto_identify: Some(false),
                name_generator: Some(true),
                gibberish: Some(true),
                test_server: Some(false),
                locale: Some(3),
                profanity_filter: Some(true),
                ..WorldRules::default()
            }
        );
        assert_eq!(eqmac_world_name(&world).unwrap(), "testworld");
        // A zone's copy: the same six words, and nothing after them.
        let zone = eqmac_rules(&world[..24]).unwrap();
        assert_eq!(
            (zone.pvp, zone.locale, zone.profanity_filter),
            (4, None, None)
        );
        assert!(eqmac_world_name(&world[..24]).is_err());
        assert!(eqmac_rules(&world[..243]).is_err());
        assert!(eqmac_rules(&[0; 264]).is_err());
    }

    #[test]
    fn the_expansion_mask_counts_from_the_first_expansion() {
        let owned = expansions(&15_u32.to_le_bytes()).unwrap();
        assert!((1..=4).all(|nth| owned.includes(nth)));
        assert!(!owned.includes(5) && !owned.includes(0) && !owned.includes(33));
        assert!(Expansions(1 << 31).includes(32));
        assert!(expansions(&[0; 3]).is_err());
        assert!(expansions(&[0; 5]).is_err());
    }
}
