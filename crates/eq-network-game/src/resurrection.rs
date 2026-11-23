//! Being resurrected: a resurrection cast on the player's corpse offers to
//! bring them back to it, and the player accepts or declines. The answer
//! repeats the offer with the player's choice; on acceptance the server
//! restores some of what death took and moves the player to the corpse.
//!
//! Layout reference: `EQEmu`'s `Resurrect_Struct` (`common/eq_packet_structs.h`,
//! which Titanium uses unchanged) and the Titanium opcodes
//! (`utils/patches/patch_Titanium.conf`); `WorldServer::HandleMessage` and
//! `Client::OPRezzAnswer` (`zone/worldserver.cpp`, `zone/client_process.cpp`)
//! for the rules.
use crate::command::EncodedCommand;
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_RezzRequest`: the offer.
pub const OFFER_OPCODE: u16 = 0x1035;
/// `OP_RezzAnswer`: the player's answer.
pub const ANSWER_OPCODE: u16 = 0x6219;
/// `EQMac`'s `OP_RezzRequest` (0x412a swapped): the offer, the same 228
/// bytes as Titanium's (`Resurrect_Struct`, TAKP 44afcbf and Quarm fc75fcf),
/// where the instance's place is `unused`.
pub const EQMAC_OFFER_OPCODE: u16 = 0x2a41;
/// `EQMac`'s `OP_RezzAnswer` (0x419b swapped): the player's answer.
pub const EQMAC_ANSWER_OPCODE: u16 = 0x9b41;
/// `EQMac`'s `OP_RezzComplete` (0x41ec swapped). TAKP sends its kept offer
/// back with the answer 1 once the player accepted, and the client moves
/// itself to the corpse (`Client::OPRezzAnswer`, `zone/client_process.cpp`);
/// Quarm sends it only to world.
pub const EQMAC_COMPLETE_OPCODE: u16 = 0xec41;

/// The reason the `EQMac` program gives its transfer request for a
/// resurrection (TAKP Batch 1 R8; TAKP's `ZC_ResurrectPlayer`).
const RESURRECTED: u32 = 9;

/// The length of the offer and the answer.
const LENGTH: usize = 228;
/// Where each part of the offer lies.
const ZONE: usize = 4;
const INSTANCE: usize = 6;
const POSITION: usize = 8;
const RECIPIENT: usize = 24;
const CASTER_NAME: usize = 92;
const SPELL: usize = 156;
const CORPSE_NAME: usize = 160;
const ACTION: usize = 224;
/// The length of each name's field.
const NAME: usize = 64;

/// An offer to resurrect the player.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResurrectionOffer {
    /// Who the offer names as its recipient.
    pub recipient: String,
    /// Who cast the resurrection.
    pub caster: String,
    /// The corpse it was cast on, as the server names it.
    pub corpse: String,
    /// The spell cast.
    pub spell_id: u32,
    /// The zone the corpse lies in, where acceptance takes the player.
    pub zone_id: u16,
    /// That zone's instance.
    pub instance_id: u16,
    /// Where the corpse lies; the offer carries no heading.
    pub position: crate::world::Position,
    /// The offer as it came, which the answer repeats.
    #[serde(skip)]
    body: Vec<u8>,
}

/// The text of a name field: its bytes up to the first NUL.
fn name(body: &[u8], at: usize) -> String {
    let field = &body[at..at + NAME];
    let end = field.iter().position(|byte| *byte == 0).unwrap_or(NAME);
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// Decodes an offer to resurrect the player.
///
/// # Errors
/// Rejects a malformed offer or one missing its recipient, caster or corpse.
pub fn titanium_offer(body: &[u8]) -> Result<ResurrectionOffer> {
    ensure!(body.len() == LENGTH, "invalid resurrection offer length");
    record(body)
}

/// A little-endian 32-bit field after the length check.
fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]])
}

/// Reads a 228-byte resurrection record, whatever its answer.
fn record(body: &[u8]) -> Result<ResurrectionOffer> {
    ensure!(body.len() == LENGTH, "invalid resurrection record length");
    let float =
        |at: usize| f32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]);
    let recipient = name(body, RECIPIENT);
    let caster = name(body, CASTER_NAME);
    let corpse = name(body, CORPSE_NAME);
    ensure!(
        !recipient.is_empty() && !caster.is_empty() && !corpse.is_empty(),
        "resurrection offer without a recipient, caster or corpse"
    );
    // The offer carries y before x.
    let (y, x, z) = (float(POSITION), float(POSITION + 4), float(POSITION + 8));
    ensure!(
        [x, y, z].iter().all(|value| value.is_finite()),
        "resurrection offer at no place"
    );
    Ok(ResurrectionOffer {
        recipient,
        caster,
        corpse,
        spell_id: word(body, SPELL),
        zone_id: u16::from_le_bytes([body[ZONE], body[ZONE + 1]]),
        instance_id: u16::from_le_bytes([body[INSTANCE], body[INSTANCE + 1]]),
        position: crate::world::Position {
            x,
            y,
            z,
            heading: 0.0,
        },
        body: body.to_vec(),
    })
}

impl ResurrectionOffer {
    /// Exact received bytes used to withhold an ambiguous replay.
    #[must_use]
    pub fn wire_identity(&self) -> &[u8] {
        &self.body
    }

    /// P99 presents an ordinary spell offer with a named corpse and destination.
    /// A missing spell or zone cannot identify that offer.
    #[must_use]
    pub const fn valid_p99_question(&self) -> bool {
        self.zone_id != 0 && !matches!(self.spell_id, 0 | 0xffff | u32::MAX)
    }

    /// Where TAKP's completion of this accepted offer sends the player: the
    /// corpse's zone and place, as the `EQMac` program's own transfer
    /// request for a resurrection. None unless the completion repeats this
    /// offer byte for byte but for its answer of 1, and names a zone. The
    /// record carries no heading, so the player faces heading 0 after it
    /// (inferred, as for TAKP's translocate).
    #[must_use]
    pub fn eqmac_move(&self, completed: &Self) -> Option<crate::zoning::ZoneOffer> {
        (self.body.len() == LENGTH
            && completed.body.len() == LENGTH
            && word(&self.body, ACTION) == 0
            && word(&completed.body, ACTION) == 1
            && self.body[..ACTION] == completed.body[..ACTION]
            && self.zone_id != 0)
            .then_some(crate::zoning::ZoneOffer {
                zone_id: self.zone_id,
                instance_id: 0,
                position: self.position,
                reason: RESURRECTED,
                to_bind: false,
                solicited: true,
            })
    }
}

/// Encodes the player's answer: the offer as it came, with the choice.
#[must_use]
pub fn titanium_answer(offer: &ResurrectionOffer, accept: bool) -> EncodedCommand {
    answer(ANSWER_OPCODE, offer, accept)
}

/// Encodes `EQMac`'s answer: every byte of the offer as it came, with only
/// the choice set. Quarm takes the zone and place from this packet
/// (`Client::Handle_OP_RezzAnswer` at fc75fcf), so nothing else may differ.
#[must_use]
pub fn eqmac_answer(offer: &ResurrectionOffer, accept: bool) -> EncodedCommand {
    answer(EQMAC_ANSWER_OPCODE, offer, accept)
}

fn answer(opcode: u16, offer: &ResurrectionOffer, accept: bool) -> EncodedCommand {
    let mut body = offer.body.clone();
    body[ACTION..ACTION + 4].copy_from_slice(&u32::from(accept).to_le_bytes());
    EncodedCommand { opcode, body }
}

/// Decodes `EQMac`'s offer: Titanium's layout, with no instance.
///
/// # Errors
/// As [`titanium_offer`].
pub fn eqmac_offer(body: &[u8]) -> Result<ResurrectionOffer> {
    Ok(ResurrectionOffer {
        instance_id: 0,
        ..titanium_offer(body)?
    })
}

/// Decodes TAKP's completion: its kept offer, with the answer 1.
///
/// # Errors
/// Rejects a record of the wrong length, one whose answer is not 1, and
/// one missing a name or place.
pub fn eqmac_completion(body: &[u8]) -> Result<ResurrectionOffer> {
    ensure!(
        body.len() == LENGTH,
        "invalid resurrection completion length"
    );
    ensure!(
        word(body, ACTION) == 1,
        "resurrection completion without an accepted answer"
    );
    Ok(ResurrectionOffer {
        instance_id: 0,
        ..record(body)?
    })
}

/// Decodes a Titanium resurrection packet from the server: an offer; None
/// for any other opcode.
///
/// # Errors
/// Rejects an offer [`titanium_offer`] cannot read.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<ResurrectionOffer>> {
    Ok(match opcode {
        OFFER_OPCODE => Some(titanium_offer(body)?),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An offer from Tester, with Resurrection (392), to the corpse of
    /// Example in zone 4.
    fn offer() -> Vec<u8> {
        let mut body = vec![0; LENGTH];
        body[ZONE..ZONE + 2].copy_from_slice(&4u16.to_le_bytes());
        body[POSITION..POSITION + 4].copy_from_slice(&1748.0f32.to_le_bytes());
        body[POSITION + 4..POSITION + 8].copy_from_slice(&263.0f32.to_le_bytes());
        body[POSITION + 8..POSITION + 12].copy_from_slice(&27.0f32.to_le_bytes());
        body[RECIPIENT..RECIPIENT + 7].copy_from_slice(b"Example");
        body[CASTER_NAME..CASTER_NAME + 6].copy_from_slice(b"Tester");
        body[SPELL..SPELL + 4].copy_from_slice(&392u32.to_le_bytes());
        body[CORPSE_NAME..CORPSE_NAME + 17].copy_from_slice(b"Example's corpse0");
        body
    }

    #[test]
    fn an_offer_names_the_caster_and_corpse_and_the_answer_repeats_it() {
        let body = offer();
        let offer = titanium_offer(&body).unwrap();
        assert_eq!(
            (offer.caster.as_str(), offer.corpse.as_str(), offer.spell_id),
            ("Tester", "Example's corpse0", 392)
        );
        assert_eq!((offer.zone_id, offer.instance_id), (4, 0));
        assert_eq!(
            [offer.position.x, offer.position.y, offer.position.z],
            [263.0, 1748.0, 27.0]
        );
        let accept = titanium_answer(&offer, true);
        assert_eq!(accept.opcode, ANSWER_OPCODE);
        assert_eq!(accept.body[..ACTION], body[..ACTION]);
        assert_eq!(accept.body[ACTION..], [1, 0, 0, 0]);
        assert_eq!(titanium_answer(&offer, false).body[ACTION..], [0, 0, 0, 0]);
        assert_eq!(decode(OFFER_OPCODE, &body).unwrap(), Some(offer));
        assert_eq!(decode(ANSWER_OPCODE, &body).unwrap(), None);
        assert!(titanium_offer(&body[..227]).is_err());
        assert!(titanium_offer(&[0; LENGTH]).is_err(), "no caster");
    }

    /// A Mac offer whose unnamed fields carry bytes, as nothing promises
    /// they are zero: the answer must return every one of them.
    fn eqmac_offer_body() -> Vec<u8> {
        let mut body = offer();
        body[..4].copy_from_slice(&[0xa1, 0xa2, 0xa3, 0xa4]);
        body[INSTANCE..INSTANCE + 2].copy_from_slice(&[0xb1, 0xb2]);
        body[20..24].copy_from_slice(&[0xc1, 0xc2, 0xc3, 0xc4]);
        body[88..92].copy_from_slice(&[0xd1, 0xd2, 0xd3, 0xd4]);
        body[RECIPIENT + 60] = 0xe1;
        body
    }

    #[test]
    fn an_eqmac_answer_is_the_offer_byte_for_byte_but_its_choice() {
        let body = eqmac_offer_body();
        let offer = eqmac_offer(&body).unwrap();
        assert_eq!(
            (offer.zone_id, offer.instance_id),
            (4, 0),
            "no Mac instance"
        );
        assert_eq!(offer.recipient, "Example");
        for (accept, action) in [(true, [1, 0, 0, 0]), (false, [0, 0, 0, 0])] {
            let answer = eqmac_answer(&offer, accept);
            assert_eq!(answer.opcode, EQMAC_ANSWER_OPCODE);
            let mut expected = body.clone();
            expected[ACTION..].copy_from_slice(&action);
            assert_eq!(answer.body, expected);
        }
        assert!(eqmac_offer(&body[..227]).is_err());
        let mut longer = body.clone();
        longer.push(0);
        assert!(eqmac_offer(&longer).is_err());
    }

    #[test]
    fn takp_completion_moves_only_for_the_same_offer_accepted() {
        let body = eqmac_offer_body();
        let offer = eqmac_offer(&body).unwrap();
        let mut done = body.clone();
        done[ACTION] = 1;
        let completed = eqmac_completion(&done).unwrap();
        assert_eq!(
            offer.eqmac_move(&completed),
            Some(crate::zoning::ZoneOffer {
                zone_id: 4,
                instance_id: 0,
                position: crate::world::Position {
                    x: 263.0,
                    y: 1748.0,
                    z: 27.0,
                    heading: 0.0,
                },
                reason: 9,
                to_bind: false,
                solicited: true,
            })
        );
        let mut elsewhere = done.clone();
        elsewhere[POSITION + 8..POSITION + 12].copy_from_slice(&28.0f32.to_le_bytes());
        assert_eq!(
            offer.eqmac_move(&eqmac_completion(&elsewhere).unwrap()),
            None
        );
        let mut other_zone = done.clone();
        other_zone[ZONE] = 5;
        assert_eq!(
            offer.eqmac_move(&eqmac_completion(&other_zone).unwrap()),
            None
        );
        assert!(
            eqmac_completion(&body).is_err(),
            "answer 0 is no completion"
        );
        assert!(eqmac_completion(&done[..227]).is_err());
        assert_eq!(completed.eqmac_move(&completed), None, "not an offer");
    }
}
