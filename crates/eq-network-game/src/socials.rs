//! Rolling dice (`/random`), emoting (`/emote`) and assisting (`/assist`);
//! and what other players do to or for the player: yelling for help near
//! them, speaking a voice macro to them, asking to inspect them, and
//! challenging them to a duel.
//!
//! The servers pass a player's yell for help (`OP_YellForHelp`) to everyone
//! within 100 of them but the yeller. `EQEmu` passes a voice macro
//! (`OP_VoiceMacroOut`) to the one told, or to the speaker's group or raid,
//! under its rule `Chat:EnableVoiceMacros`, on by default, and quests send
//! one as a tell. Every server passes an inspect request
//! (`OP_InspectRequest`) on to the player inspected, for their client to
//! answer with what they wear; nothing here answers one yet.
//!
//! A challenge to a duel (`OP_RequestDuel`) reaches the player challenged
//! naming the challenger, and their acceptance reaches the challenger the
//! same way, naming them; `EQEmu` and Quarm then send both players
//! `OP_DuelAccept` (Quarm's `OP_DuelResponse2`), naming the challenger and
//! the one who accepted. TAKP passes the clients' own records between
//! them, and with them a client's answer turning a challenge down
//! (`OP_DuelResponse`), where `EQEmu` and Quarm send a line of text.
//! Nothing here answers a challenge yet.
//!
//! Layout reference: `EQEmu`'s Titanium `RandomReq_Struct`,
//! `RandomReply_Struct`, `Emote_Struct`, `EntityId_Struct` and
//! `Inspect_Struct` (`common/patches/titanium_structs.h`) and
//! `VoiceMacroOut_Struct` and `Duel_Struct` (`common/eq_packet_structs.h`),
//! which Titanium's wire carries as built; TAKP's and Quarm's
//! `Inspect_Struct`, Quarm's `Duel_Struct`, and TAKP's
//! `DuelChallenge_Struct` and `DuelResponse_Struct`
//! (`common/eq_packet_structs.h`). The rules are
//! `Client::Handle_OP_RandomReq`, `Handle_OP_Emote`, `Handle_OP_Assist`,
//! `Handle_OP_YellForHelp`, `Handle_OP_InspectRequest`,
//! `Handle_OP_RequestDuel` and `Handle_OP_DuelAccept` (Quarm's
//! `Handle_OP_DuelResponse2`), and TAKP's `Handle_OP_DuelResponse`
//! (`zone/client_packet.cpp`); a voice macro's relay is `EQEmu`'s
//! `WorldServer::HandleMessage` (`zone/worldserver.cpp`).
use crate::{
    command::EncodedCommand,
    world::{until_nul, WorldEvent},
};
use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// `OP_RandomReq`: a die to roll, from the lowest to the highest number.
pub const RANDOM_OPCODE: u16 = 0x5534;
/// `OP_RandomReply`: a die the server rolled for a player nearby.
pub const ROLL_OPCODE: u16 = 0x6cd5;
/// `OP_Emote`: what a player does, in their own words.
pub const EMOTE_OPCODE: u16 = 0x547a;
/// `OP_Assist`: whose target to take; the server answers with that target.
pub const ASSIST_OPCODE: u16 = 0x7709;
/// `OP_YellForHelp`: a player near the player yelled for help (Titanium).
pub const YELL_OPCODE: u16 = 0x61ef;
/// `EQMac`'s `OP_YellForHelp`.
pub const EQMAC_YELL_OPCODE: u16 = 0xda41;
/// `OP_VoiceMacroOut`: a voice macro spoken to the player (Titanium).
pub const VOICE_MACRO_OPCODE: u16 = 0x2ec6;
/// `OP_InspectRequest`: another player asks to inspect the player
/// (Titanium).
pub const INSPECT_REQUEST_OPCODE: u16 = 0x775d;
/// `EQMac`'s `OP_InspectRequest`.
pub const EQMAC_INSPECT_REQUEST_OPCODE: u16 = 0xb540;
/// `OP_RequestDuel`: a challenge to a duel, or its acceptance (Titanium).
pub const REQUEST_DUEL_OPCODE: u16 = 0x28e1;
/// `OP_DuelAccept`: a duel begins (Titanium).
pub const DUEL_ACCEPT_OPCODE: u16 = 0x1b09;
/// `EQMac`'s `OP_RequestDuel`.
pub const EQMAC_REQUEST_DUEL_OPCODE: u16 = 0xcf40;
/// `EQMac`'s `OP_DuelAccept`, which Quarm names `OP_DuelResponse2`.
pub const EQMAC_DUEL_ACCEPT_OPCODE: u16 = 0x5d41;
/// TAKP's `OP_DuelResponse`: a client's answer turning a challenge down.
pub const EQMAC_DUEL_RESPONSE_OPCODE: u16 = 0xd040;

/// The emote's text field; the server reads at most 512 bytes of it.
const EMOTE_TEXT: usize = 1024;
/// The longest emote the server passes on whole.
pub const EMOTE_LONGEST: usize = 511;
/// A roll's or voice macro's name field.
const NAME: usize = 64;
/// Titanium's voice macro record.
const VOICE_MACRO: usize = 140;

/// A die the server rolled for a player near the player, the player among
/// them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Roll {
    /// Who rolled it.
    pub name: String,
    /// The lowest number it could have turned up.
    pub low: u32,
    /// The highest.
    pub high: u32,
    /// What it turned up.
    pub result: u32,
}

/// The server's answer to an assist.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Assisted {
    /// The target to take; None to keep the player's own.
    pub target: Option<u16>,
}

/// A voice macro another player spoke to the player, their group or their
/// raid.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VoiceMacro {
    /// Who spoke it.
    pub from: String,
    /// To whom.
    pub kind: VoiceKind,
    /// The voice it is spoken in, as sent: on `EQEmu`, twice the number of
    /// the speaker's playable race plus their gender.
    pub voice: u32,
    /// Which macro.
    pub number: u32,
}

/// To whom a voice macro was spoken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum VoiceKind {
    /// The player alone, as a tell.
    Tell,
    /// The player's group.
    Group,
    /// The player's raid.
    Raid,
    /// A kind this crate does not know, as sent.
    Other(u32),
}

impl VoiceKind {
    /// The kind a voice macro's word names.
    const fn from_word(word: u32) -> Self {
        match word {
            1 => Self::Tell,
            2 => Self::Group,
            3 => Self::Raid,
            other => Self::Other(other),
        }
    }
}

/// News of a duel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum DuelUpdate {
    /// Another player challenges the player to a duel; or, from the player
    /// the player challenged, accepts. Every server sends both alike, so
    /// which it is depends on whether the player challenged them.
    Challenge {
        /// The other player.
        from: u16,
    },
    /// A duel begins. TAKP sends it only to the challenger, whose official
    /// client ignores it, by TAKP's note.
    Accepted {
        /// Who challenged.
        challenger: u16,
        /// Who accepted.
        accepter: u16,
    },
    /// Another player's client turned a duel with the player down (TAKP;
    /// `EQEmu` and Quarm send a line of text instead).
    Refused {
        /// The other player.
        from: u16,
        /// Why.
        reason: DuelRefusal,
    },
}

/// Why a client turned a duel down, by TAKP's numbers for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum DuelRefusal {
    /// The player declined it.
    Declined,
    /// The player is dueling already.
    Dueling,
    /// The player is considering another challenge.
    Considering,
    /// A reason this crate does not know, as sent.
    Other(i8),
}

impl DuelRefusal {
    /// The reason a duel answer's byte names.
    const fn from_byte(byte: i8) -> Self {
        match byte {
            0 => Self::Declined,
            1 => Self::Dueling,
            2 => Self::Considering,
            other => Self::Other(other),
        }
    }
}

fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]])
}

/// A spawn named by a 32-bit word: never zero, and within 16 bits.
fn spawn(body: &[u8], at: usize, what: &str) -> Result<u16> {
    let spawn_id =
        u16::try_from(word(body, at)).with_context(|| format!("{what} spawn out of range"))?;
    ensure!(spawn_id != 0, "{what} without a spawn");
    Ok(spawn_id)
}

/// A spawn named by a 16-bit field: never zero.
fn short_spawn(body: &[u8], at: usize, what: &str) -> Result<u16> {
    let spawn_id = u16::from_le_bytes([body[at], body[at + 1]]);
    ensure!(spawn_id != 0, "{what} without a spawn");
    Ok(spawn_id)
}

/// Rolls a die from the lowest to the highest number; the server orders
/// them, and rolls 0 to 100 for two zeros.
#[must_use]
pub fn random(low: u32, high: u32) -> EncodedCommand {
    let mut body = low.to_le_bytes().to_vec();
    body.extend_from_slice(&high.to_le_bytes());
    EncodedCommand {
        opcode: RANDOM_OPCODE,
        body,
    }
}

/// Emotes: the server puts the player's name before the text, as it is,
/// so the text starts with a space.
///
/// # Errors
/// Rejects an empty emote, one the server would cut, and one with a NUL.
pub fn emote(text: &str) -> Result<EncodedCommand> {
    ensure!(!text.trim().is_empty(), "an emote needs words");
    ensure!(
        text.len() < EMOTE_LONGEST,
        "an emote is at most {} bytes",
        EMOTE_LONGEST - 1
    );
    ensure!(!text.contains('\0'), "an emote cannot hold a NUL");
    let mut body = vec![0; 4 + EMOTE_TEXT];
    let words = format!(" {text}");
    body[4..4 + words.len()].copy_from_slice(words.as_bytes());
    Ok(EncodedCommand {
        opcode: EMOTE_OPCODE,
        body,
    })
}

/// An emote as the server passes it on to the players near the one who
/// made it: their name, then their words.
#[must_use]
pub fn heard_emote(name: &str, text: &str) -> Vec<u8> {
    let mut body = vec![0; 4];
    body.extend_from_slice(name.as_bytes());
    body.push(b' ');
    body.extend_from_slice(text.as_bytes());
    body.push(0);
    body
}

/// Takes the target of the spawn assisted.
#[must_use]
pub fn assist(spawn_id: u16) -> EncodedCommand {
    EncodedCommand {
        opcode: ASSIST_OPCODE,
        body: u32::from(spawn_id).to_le_bytes().to_vec(),
    }
}

/// Decodes a die rolled nearby; None for any other opcode.
///
/// # Errors
/// Rejects a malformed length.
pub fn decode_roll(opcode: u16, body: &[u8]) -> Result<Option<Roll>> {
    if opcode != ROLL_OPCODE {
        return Ok(None);
    }
    ensure!(body.len() == 12 + NAME, "invalid roll length");
    let field = &body[12..12 + NAME];
    let end = field.iter().position(|byte| *byte == 0).unwrap_or(NAME);
    Ok(Some(Roll {
        name: String::from_utf8_lossy(&field[..end]).into_owned(),
        low: word(body, 0),
        high: word(body, 4),
        result: word(body, 8),
    }))
}

/// Decodes the server's answer to an assist; None for any other opcode.
///
/// # Errors
/// Rejects a malformed length or spawn ID.
pub fn decode_assist(opcode: u16, body: &[u8]) -> Result<Option<Assisted>> {
    if opcode != ASSIST_OPCODE {
        return Ok(None);
    }
    ensure!(body.len() == 4, "invalid assist length");
    let target = u16::try_from(word(body, 0))?;
    Ok(Some(Assisted {
        target: (target != 0).then_some(target),
    }))
}

/// What Titanium's messages from other players say: a yell for help, a
/// voice macro, an inspect request and a duel's news; None for other
/// messages.
///
/// # Errors
/// Rejects malformed records of those messages.
pub(crate) fn titanium(opcode: u16, body: &[u8]) -> Result<Option<WorldEvent>> {
    Ok(Some(match opcode {
        YELL_OPCODE => yell(body)?,
        VOICE_MACRO_OPCODE => WorldEvent::VoiceMacro(voice_macro(body)?),
        INSPECT_REQUEST_OPCODE => titanium_inspect_request(body)?,
        REQUEST_DUEL_OPCODE => WorldEvent::Duel(titanium_challenge(body)?),
        DUEL_ACCEPT_OPCODE => WorldEvent::Duel(titanium_accepted(body)?),
        _ => return Ok(None),
    }))
}

/// Decodes `OP_YellForHelp`, the same on both generations (4 bytes): the
/// yeller's spawn, 32 bits.
///
/// # Errors
/// Rejects a wrong length and a spawn that is zero or past 16 bits.
pub(crate) fn yell(body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == 4, "invalid yell for help length");
    Ok(WorldEvent::YellForHelp {
        spawn_id: spawn(body, 0, "yell for help")?,
    })
}

/// Decodes Titanium's `OP_VoiceMacroOut` (140 bytes): the speaker's name in
/// 64 bytes, ended by a NUL within them; to whom, a word `EQEmu` leaves
/// zero, the voice and the macro's number, each 32 bits; then 60 bytes it
/// leaves zero.
///
/// # Errors
/// Rejects a wrong length and a name that fills its field.
fn voice_macro(body: &[u8]) -> Result<VoiceMacro> {
    ensure!(body.len() == VOICE_MACRO, "invalid voice macro length");
    let name = &body[..NAME];
    ensure!(name.contains(&0), "voice macro speaker without its end");
    Ok(VoiceMacro {
        from: String::from_utf8_lossy(until_nul(name)).into_owned(),
        kind: VoiceKind::from_word(word(body, NAME)),
        voice: word(body, NAME + 8),
        number: word(body, NAME + 12),
    })
}

/// Decodes Titanium's `OP_InspectRequest` (8 bytes): the player's own
/// spawn, then the inspector's, as their client names it, each 32 bits.
///
/// # Errors
/// Rejects a wrong length and an inspector that is zero or past 16 bits.
fn titanium_inspect_request(body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == 8, "invalid inspect request length");
    Ok(WorldEvent::InspectRequest {
        inspector_id: spawn(body, 4, "inspect request")?,
    })
}

/// Decodes `EQMac`'s `OP_InspectRequest` (4 bytes): the player's own spawn,
/// then the inspector's, as their client names it, each 16 bits.
///
/// # Errors
/// Rejects a wrong length and an inspector of zero.
pub(crate) fn eqmac_inspect_request(body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == 4, "invalid inspect request length");
    Ok(WorldEvent::InspectRequest {
        inspector_id: short_spawn(body, 2, "inspect request")?,
    })
}

/// Decodes Titanium's `OP_RequestDuel` (8 bytes): two spawns, each 32
/// bits, the second the other player. `EQEmu` names them in both.
///
/// # Errors
/// Rejects a wrong length and a second spawn that is zero or past 16 bits.
fn titanium_challenge(body: &[u8]) -> Result<DuelUpdate> {
    ensure!(body.len() == 8, "invalid duel request length");
    Ok(DuelUpdate::Challenge {
        from: spawn(body, 4, "duel request")?,
    })
}

/// Decodes Titanium's `OP_DuelAccept` (8 bytes): the challenger's spawn,
/// then the accepter's, each 32 bits.
///
/// # Errors
/// Rejects a wrong length and a spawn that is zero or past 16 bits.
fn titanium_accepted(body: &[u8]) -> Result<DuelUpdate> {
    ensure!(body.len() == 8, "invalid duel acceptance length");
    Ok(DuelUpdate::Accepted {
        challenger: spawn(body, 0, "duel acceptance")?,
        accepter: spawn(body, 4, "duel acceptance")?,
    })
}

/// Decodes `EQMac`'s `OP_RequestDuel` (4 bytes): two spawns, each 16 bits,
/// the second the other player. Quarm names them in both; TAKP passes the
/// sender's record on, whose first is the player's own spawn.
///
/// # Errors
/// Rejects a wrong length and a second spawn of zero.
pub(crate) fn eqmac_challenge(body: &[u8]) -> Result<DuelUpdate> {
    ensure!(body.len() == 4, "invalid duel request length");
    Ok(DuelUpdate::Challenge {
        from: short_spawn(body, 2, "duel request")?,
    })
}

/// Decodes `EQMac`'s `OP_DuelAccept` (4 bytes): the challenger's spawn,
/// then the accepter's, each 16 bits.
///
/// # Errors
/// Rejects a wrong length and a spawn of zero.
pub(crate) fn eqmac_accepted(body: &[u8]) -> Result<DuelUpdate> {
    ensure!(body.len() == 4, "invalid duel acceptance length");
    Ok(DuelUpdate::Accepted {
        challenger: short_spawn(body, 0, "duel acceptance")?,
        accepter: short_spawn(body, 2, "duel acceptance")?,
    })
}

/// Decodes TAKP's `OP_DuelResponse` (6 bytes): the player's own spawn and
/// the other player's, each 16 bits, the reason as a signed byte, and a
/// byte of padding.
///
/// # Errors
/// Rejects a wrong length and another player of zero.
pub(crate) fn eqmac_refused(body: &[u8]) -> Result<DuelUpdate> {
    ensure!(body.len() == 6, "invalid duel answer length");
    Ok(DuelUpdate::Refused {
        from: short_spawn(body, 2, "duel answer")?,
        reason: DuelRefusal::from_byte(body[4].cast_signed()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::titanium_update;

    #[test]
    fn requests_are_the_servers_structs() {
        assert_eq!(random(1, 6).body, [1, 0, 0, 0, 6, 0, 0, 0]);
        let waved = emote("waves.").unwrap();
        assert_eq!((waved.opcode, waved.body.len()), (EMOTE_OPCODE, 1028));
        assert_eq!(&waved.body[..11], b"\0\0\0\0 waves.");
        assert!(emote(" ").is_err());
        assert!(emote(&"a".repeat(EMOTE_LONGEST)).is_err());
        assert_eq!(assist(300).body, [44, 1, 0, 0]);
        assert_eq!(heard_emote("Tester", "waves."), b"\0\0\0\0Tester waves.\0");
    }

    #[test]
    fn the_servers_answers_name_the_roll_and_the_target() {
        let mut roll = vec![0; 76];
        roll[..4].copy_from_slice(&1u32.to_le_bytes());
        roll[4..8].copy_from_slice(&6u32.to_le_bytes());
        roll[8..12].copy_from_slice(&4u32.to_le_bytes());
        roll[12..18].copy_from_slice(b"Tester");
        assert_eq!(
            decode_roll(ROLL_OPCODE, &roll).unwrap(),
            Some(Roll {
                name: "Tester".into(),
                low: 1,
                high: 6,
                result: 4
            })
        );
        assert!(decode_roll(ROLL_OPCODE, &roll[..70]).is_err());
        assert_eq!(decode_roll(ASSIST_OPCODE, &roll).unwrap(), None);
        assert_eq!(
            decode_assist(ASSIST_OPCODE, &[44, 1, 0, 0]).unwrap(),
            Some(Assisted { target: Some(300) })
        );
        assert_eq!(
            decode_assist(ASSIST_OPCODE, &[0; 4]).unwrap(),
            Some(Assisted { target: None })
        );
        assert!(decode_assist(ASSIST_OPCODE, &[0; 5]).is_err());
    }

    #[test]
    fn yells_for_help_name_the_yeller_on_both_generations() {
        let yelled = WorldEvent::YellForHelp { spawn_id: 300 };
        assert_eq!(
            titanium_update(YELL_OPCODE, &[44, 1, 0, 0]).unwrap(),
            Some(yelled.clone())
        );
        assert_eq!(
            crate::quarm::updates(EQMAC_YELL_OPCODE, &[44, 1, 0, 0]).unwrap(),
            [yelled]
        );
        for bad in [&[44, 1, 0][..], &[0; 4], &[0, 0, 1, 0]] {
            assert!(yell(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn voice_macros_name_the_speaker_whom_and_which() {
        let macro_record = |kind: u32| {
            let mut body = vec![0; VOICE_MACRO];
            body[..6].copy_from_slice(b"Tester");
            for (at, value) in [(64, kind), (72, 5), (76, 12)] {
                body[at..at + 4].copy_from_slice(&u32::to_le_bytes(value));
            }
            body
        };
        assert_eq!(
            titanium_update(VOICE_MACRO_OPCODE, &macro_record(2)).unwrap(),
            Some(WorldEvent::VoiceMacro(VoiceMacro {
                from: "Tester".into(),
                kind: VoiceKind::Group,
                voice: 5,
                number: 12,
            }))
        );
        assert_eq!(
            [1, 3, 9].map(|kind| voice_macro(&macro_record(kind)).unwrap().kind),
            [VoiceKind::Tell, VoiceKind::Raid, VoiceKind::Other(9)]
        );
        let mut nameless_end = macro_record(1);
        nameless_end[..NAME].fill(b'a');
        assert!(voice_macro(&nameless_end).is_err());
        assert!(voice_macro(&macro_record(1)[..VOICE_MACRO - 1]).is_err());
    }

    #[test]
    fn inspect_requests_name_the_inspector_on_both_generations() {
        // The player's own spawn, then the inspector's.
        let asked = WorldEvent::InspectRequest { inspector_id: 300 };
        assert_eq!(
            titanium_update(INSPECT_REQUEST_OPCODE, &[7, 0, 0, 0, 44, 1, 0, 0]).unwrap(),
            Some(asked.clone())
        );
        assert_eq!(
            crate::quarm::updates(EQMAC_INSPECT_REQUEST_OPCODE, &[7, 0, 44, 1]).unwrap(),
            [asked]
        );
        for bad in [
            &[7, 0, 0, 0, 0, 0, 0, 0][..],
            &[7, 0, 0, 0, 0, 0, 1, 0],
            &[7; 7],
        ] {
            assert!(titanium_inspect_request(bad).is_err(), "{bad:?}");
        }
        for bad in [&[7, 0, 0, 0][..], &[7, 0, 44]] {
            assert!(eqmac_inspect_request(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn titanium_duels_name_the_other_player_then_both() {
        let words = |first: u32, second: u32| {
            let mut body = first.to_le_bytes().to_vec();
            body.extend_from_slice(&second.to_le_bytes());
            body
        };
        // A challenge, or its acceptance, names the other player twice;
        // only the second is read.
        for first in [300, 0] {
            assert_eq!(
                titanium_update(REQUEST_DUEL_OPCODE, &words(first, 300)).unwrap(),
                Some(WorldEvent::Duel(DuelUpdate::Challenge { from: 300 }))
            );
        }
        assert_eq!(
            titanium_update(DUEL_ACCEPT_OPCODE, &words(300, 7)).unwrap(),
            Some(WorldEvent::Duel(DuelUpdate::Accepted {
                challenger: 300,
                accepter: 7,
            }))
        );
        for bad in [words(300, 0), words(300, 70_000)] {
            assert!(titanium_challenge(&bad).is_err());
        }
        for bad in [words(0, 7), words(300, 0), words(70_000, 7)] {
            assert!(titanium_accepted(&bad).is_err());
        }
        assert!(titanium_challenge(&words(300, 300)[..7]).is_err());
        assert!(titanium_accepted(&words(300, 7)[..7]).is_err());
    }

    #[test]
    fn eqmac_duels_name_the_other_player_and_takp_says_why_not() {
        let updates = |opcode: u16, body: &[u8]| crate::quarm::updates(opcode, body).unwrap();
        let duel = |update: DuelUpdate| [WorldEvent::Duel(update)];
        // Quarm names the challenger twice; TAKP passes the sender's record
        // on, the player's own spawn first.
        for body in [[44, 1, 44, 1], [7, 0, 44, 1]] {
            assert_eq!(
                updates(EQMAC_REQUEST_DUEL_OPCODE, &body),
                duel(DuelUpdate::Challenge { from: 300 })
            );
        }
        assert_eq!(
            updates(EQMAC_DUEL_ACCEPT_OPCODE, &[44, 1, 7, 0]),
            duel(DuelUpdate::Accepted {
                challenger: 300,
                accepter: 7,
            })
        );
        assert_eq!(
            updates(EQMAC_DUEL_RESPONSE_OPCODE, &[7, 0, 44, 1, 0, 0]),
            duel(DuelUpdate::Refused {
                from: 300,
                reason: DuelRefusal::Declined,
            })
        );
        assert_eq!(
            [1, 2, 0xff].map(|byte| eqmac_refused(&[7, 0, 44, 1, byte, 0]).unwrap()),
            [
                DuelRefusal::Dueling,
                DuelRefusal::Considering,
                DuelRefusal::Other(-1)
            ]
            .map(|reason| DuelUpdate::Refused { from: 300, reason })
        );
        assert!(eqmac_challenge(&[44, 1, 0, 0]).is_err());
        assert!(eqmac_challenge(&[44, 1, 44]).is_err());
        assert!(eqmac_accepted(&[0, 0, 7, 0]).is_err());
        assert!(eqmac_accepted(&[44, 1, 0, 0]).is_err());
        assert!(eqmac_refused(&[7, 0, 0, 0, 0, 0]).is_err());
        assert!(eqmac_refused(&[7, 0, 44, 1, 0]).is_err());
    }
}
