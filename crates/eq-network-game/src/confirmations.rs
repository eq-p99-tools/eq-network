//! The server's questions the player answers: whether to give their life to
//! a sacrifice, and whether to go where a translocate takes them.
//!
//! A sacrifice (`OP_Sacrifice`, 12 bytes: the caster's and the player's
//! spawns as 32-bit IDs, then the answer) and a translocate
//! (`OP_Translocate`, 92 bytes: the zone, the spell, a field every server
//! leaves 0, the caster's name, where to, y first, and the answer) come with
//! an answer of 0, and the player's answer repeats the offer with their
//! choice. Every server takes any answer to a sacrifice but 0 as a yes, and
//! only 1 to a translocate (`Client::Handle_OP_Sacrifice`,
//! `Client::Handle_OP_Translocate`). A no is 0, but `EQMac`'s to a
//! translocate, which TAKP's layout names -1. What the official clients
//! answer is unrecorded, so the answers here are inferred.
//!
//! On a yes, `EQEmu` and Quarm take the player's life or move them
//! themselves. TAKP sends a translocate's offer again with the answer 1, and
//! the client moves itself: to another zone through its own transfer
//! request, which TAKP grants with the destination it kept
//! (`Client::SetPendingZoneTransfer`), or within the zone by going there.
//! That second offer reads as the server's offer to move the player.
//!
//! Layout reference: `Sacrifice_Struct` and `Translocate_Struct` (`EQEmu`'s,
//! TAKP's and Quarm's `common/eq_packet_structs.h`).
use crate::{command::EncodedCommand, world::Position, zoning::ZoneOffer};
use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;

/// `OP_Sacrifice`: the offer, and the player's answer.
pub const SACRIFICE_OPCODE: u16 = 0x727a;
/// `OP_Translocate`: the offer, and the player's answer.
pub const TRANSLOCATE_OPCODE: u16 = 0x8258;
/// `EQMac`'s `OP_Sacrifice`.
pub const EQMAC_SACRIFICE_OPCODE: u16 = 0xea41;
/// `EQMac`'s `OP_Translocate`, and TAKP's word that the player may go.
pub const EQMAC_TRANSLOCATE_OPCODE: u16 = 0x0642;

/// The length of a sacrifice record.
const SACRIFICE: usize = 12;
/// Where a sacrifice record holds the answer.
const CONFIRM: usize = 8;
/// The length of a translocate record.
const TRANSLOCATE: usize = 92;
/// Where each part of a translocate record lies.
const ZONE: usize = 0;
const SPELL: usize = 4;
const CASTER: usize = 12;
const POSITION: usize = 76;
const COMPLETE: usize = 88;
/// The length of the caster's name field.
const NAME: usize = 64;

/// The reason TAKP's client is taken to give its transfer request once a
/// translocate lets it go (`ZC_TranslocateSpell2`, TAKP's
/// `common/eq_constants.h`: a zone command with the caster's name in the
/// packet), inferred: TAKP takes any reason but 0 there
/// (`Client::Handle_OP_ZoneChange`), and the official client's is
/// unrecorded.
const TRANSLOCATED: u32 = 8;

/// An offer to sacrifice the player.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SacrificeOffer {
    /// The spawn that cast the sacrifice.
    pub caster_id: u16,
    /// The offer as it came, which the answer repeats.
    #[serde(skip)]
    body: Vec<u8>,
}

/// An offer to translocate the player.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TranslocateOffer {
    /// Who cast the translocate, by name.
    pub caster: String,
    /// The spell cast.
    pub spell_id: u32,
    /// The zone it takes the player to.
    pub zone_id: u16,
    /// Where in that zone; the offer carries no heading.
    pub position: Position,
    /// The offer as it came, which the answer repeats.
    #[serde(skip)]
    body: Vec<u8>,
}

/// What an `EQMac` translocate record is.
#[derive(Clone, Debug, PartialEq)]
pub enum Translocate {
    /// An offer to translocate the player.
    Offer(TranslocateOffer),
    /// TAKP's word, after the player's yes, that the player may go: the
    /// server's offer to move them there.
    Go(ZoneOffer),
}

/// Decodes a sacrifice offer, as both generations lay it out. The player
/// is always the one the server sends it to, so their spawn is not read.
///
/// # Errors
/// Rejects any length other than 12 bytes, a caster that names no spawn,
/// and a record already answered.
pub fn sacrifice(body: &[u8]) -> Result<SacrificeOffer> {
    ensure!(body.len() == SACRIFICE, "invalid sacrifice length");
    let caster_id = u16::try_from(word(body, 0))
        .ok()
        .filter(|&id| id != 0)
        .context("sacrifice offer without a caster")?;
    ensure!(word(body, CONFIRM) == 0, "sacrifice record is not an offer");
    Ok(SacrificeOffer {
        caster_id,
        body: body.to_vec(),
    })
}

/// Reads a translocate record: the offer, and its answer.
fn translocate(body: &[u8]) -> Result<(TranslocateOffer, u32)> {
    ensure!(body.len() == TRANSLOCATE, "invalid translocate length");
    let caster =
        String::from_utf8_lossy(crate::world::until_nul(&body[CASTER..CASTER + NAME])).into_owned();
    ensure!(!caster.is_empty(), "translocate offer without a caster");
    let float = |at| f32::from_bits(word(body, at));
    // The record carries y before x.
    let (y, x, z) = (float(POSITION), float(POSITION + 4), float(POSITION + 8));
    ensure!(
        [x, y, z].iter().all(|value| value.is_finite()),
        "translocate offer to no place"
    );
    let offer = TranslocateOffer {
        caster,
        spell_id: word(body, SPELL),
        zone_id: u16::try_from(word(body, ZONE)).context("translocate zone out of range")?,
        position: Position {
            x,
            y,
            z,
            heading: 0.0,
        },
        body: body.to_vec(),
    };
    Ok((offer, word(body, COMPLETE)))
}

/// Decodes Titanium's translocate offer.
///
/// # Errors
/// Rejects any length other than 92 bytes, an offer without a caster, to a
/// zone beyond 16 bits or to a place that is not finite, and a record
/// already answered.
pub fn titanium_translocate(body: &[u8]) -> Result<TranslocateOffer> {
    let (offer, complete) = translocate(body)?;
    ensure!(complete == 0, "translocate record is not an offer");
    Ok(offer)
}

/// Decodes `EQMac`'s translocate record: an offer, or with the answer 1,
/// TAKP's word that the player may go, which reads as the server's offer to
/// move them there. That move carries no heading, so the player faces
/// heading 0 after it (inferred: where the official client faces is
/// unrecorded).
///
/// # Errors
/// As [`titanium_translocate`], but for an answer of 1.
pub fn eqmac_translocate(body: &[u8]) -> Result<Translocate> {
    let (offer, complete) = translocate(body)?;
    Ok(match complete {
        0 => Translocate::Offer(offer),
        1 => Translocate::Go(ZoneOffer {
            zone_id: offer.zone_id,
            instance_id: 0,
            position: offer.position,
            reason: TRANSLOCATED,
            to_bind: false,
            solicited: true,
        }),
        _ => bail!("translocate record is not an offer"),
    })
}

/// Encodes Titanium's answer to a sacrifice: the offer, with 1 for yes and
/// 0 for no.
#[must_use]
pub fn titanium_sacrifice_answer(offer: &SacrificeOffer, accept: bool) -> EncodedCommand {
    answer(SACRIFICE_OPCODE, &offer.body, CONFIRM, u32::from(accept))
}

/// Encodes `EQMac`'s answer to a sacrifice, as Titanium's.
#[must_use]
pub fn eqmac_sacrifice_answer(offer: &SacrificeOffer, accept: bool) -> EncodedCommand {
    answer(
        EQMAC_SACRIFICE_OPCODE,
        &offer.body,
        CONFIRM,
        u32::from(accept),
    )
}

/// Encodes Titanium's answer to a translocate: the offer, with 1 for yes
/// and 0 for no.
#[must_use]
pub fn titanium_translocate_answer(offer: &TranslocateOffer, accept: bool) -> EncodedCommand {
    answer(TRANSLOCATE_OPCODE, &offer.body, COMPLETE, u32::from(accept))
}

/// Encodes `EQMac`'s answer to a translocate: the offer, with 1 for yes and
/// -1 for no.
#[must_use]
pub fn eqmac_translocate_answer(offer: &TranslocateOffer, accept: bool) -> EncodedCommand {
    let value = if accept { 1 } else { -1i32 };
    answer(
        EQMAC_TRANSLOCATE_OPCODE,
        &offer.body,
        COMPLETE,
        value.cast_unsigned(),
    )
}

/// The offer as it came, with the answer at `at`.
fn answer(opcode: u16, offer: &[u8], at: usize, value: u32) -> EncodedCommand {
    let mut body = offer.to_vec();
    body[at..at + 4].copy_from_slice(&value.to_le_bytes());
    EncodedCommand { opcode, body }
}

fn word(body: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        body[offset..offset + 4]
            .try_into()
            .expect("checked record length"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sacrifice from spawn `caster` to spawn 9, answered `confirm`.
    fn sacrifice_record(caster: u32, confirm: u32) -> Vec<u8> {
        [caster, 9, confirm]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    #[test]
    fn a_sacrifice_names_its_caster_and_the_answer_repeats_it() {
        let body = sacrifice_record(581, 0);
        let offer = sacrifice(&body).unwrap();
        assert_eq!(offer.caster_id, 581);
        let yes = titanium_sacrifice_answer(&offer, true);
        assert_eq!(yes.opcode, SACRIFICE_OPCODE);
        assert_eq!(yes.body[..CONFIRM], body[..CONFIRM]);
        assert_eq!(yes.body[CONFIRM..], [1, 0, 0, 0]);
        assert_eq!(
            titanium_sacrifice_answer(&offer, false).body[CONFIRM..],
            [0; 4]
        );
        let yes = eqmac_sacrifice_answer(&offer, true);
        assert_eq!(
            (yes.opcode, &yes.body[CONFIRM..]),
            (EQMAC_SACRIFICE_OPCODE, &[1, 0, 0, 0][..])
        );
        assert_eq!(
            eqmac_sacrifice_answer(&offer, false).body[CONFIRM..],
            [0; 4]
        );
        for length in [11, 13] {
            assert!(sacrifice(&vec![0; length]).is_err());
        }
        // No caster, a caster beyond any spawn's 16 bits, and an answer.
        for (caster, confirm) in [(0, 0), (0x1_0000, 0), (581, 1)] {
            assert!(sacrifice(&sacrifice_record(caster, confirm)).is_err());
        }
    }

    /// A translocate from Tester with spell 2000 to (263, 1748, 27) in zone
    /// 4, answered `complete`.
    fn translocate_record(complete: i32) -> Vec<u8> {
        let mut body = vec![0; TRANSLOCATE];
        body[ZONE..ZONE + 4].copy_from_slice(&4u32.to_le_bytes());
        body[SPELL..SPELL + 4].copy_from_slice(&2000u32.to_le_bytes());
        body[CASTER..CASTER + 6].copy_from_slice(b"Tester");
        for (index, value) in [1748.0f32, 263.0, 27.0].into_iter().enumerate() {
            let at = POSITION + index * 4;
            body[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        body[COMPLETE..].copy_from_slice(&complete.to_le_bytes());
        body
    }

    #[test]
    fn a_translocate_names_its_caster_spell_and_place_and_the_answer_repeats_it() {
        let body = translocate_record(0);
        let offer = titanium_translocate(&body).unwrap();
        assert_eq!(
            (offer.caster.as_str(), offer.spell_id, offer.zone_id),
            ("Tester", 2000, 4)
        );
        assert_eq!(
            offer.position,
            Position {
                x: 263.0,
                y: 1748.0,
                z: 27.0,
                heading: 0.0,
            }
        );
        let yes = titanium_translocate_answer(&offer, true);
        assert_eq!(yes.opcode, TRANSLOCATE_OPCODE);
        assert_eq!(yes.body[..COMPLETE], body[..COMPLETE]);
        assert_eq!(yes.body[COMPLETE..], [1, 0, 0, 0]);
        assert_eq!(
            titanium_translocate_answer(&offer, false).body[COMPLETE..],
            [0; 4]
        );
        // EQMac's no is -1.
        assert_eq!(
            eqmac_translocate(&body).unwrap(),
            Translocate::Offer(offer.clone())
        );
        let no = eqmac_translocate_answer(&offer, false);
        assert_eq!(
            (no.opcode, &no.body[COMPLETE..]),
            (EQMAC_TRANSLOCATE_OPCODE, &[0xff; 4][..])
        );
        assert_eq!(
            eqmac_translocate_answer(&offer, true).body[COMPLETE..],
            [1, 0, 0, 0]
        );
        assert!(titanium_translocate(&body[..91]).is_err());
    }

    #[test]
    fn takps_word_after_a_yes_is_a_move_and_titanium_reads_no_answered_record() {
        let go = translocate_record(1);
        assert_eq!(
            eqmac_translocate(&go).unwrap(),
            Translocate::Go(ZoneOffer {
                zone_id: 4,
                instance_id: 0,
                position: Position {
                    x: 263.0,
                    y: 1748.0,
                    z: 27.0,
                    heading: 0.0,
                },
                reason: TRANSLOCATED,
                to_bind: false,
                solicited: true,
            })
        );
        assert!(titanium_translocate(&go).is_err());
        assert!(eqmac_translocate(&translocate_record(-1)).is_err());
    }

    #[test]
    fn a_translocate_without_a_caster_zone_or_place_is_unreadable() {
        let mut body = translocate_record(0);
        body[CASTER] = 0;
        assert!(titanium_translocate(&body).is_err(), "no caster");
        let mut body = translocate_record(0);
        body[ZONE..ZONE + 4].copy_from_slice(&0x1_0000u32.to_le_bytes());
        assert!(titanium_translocate(&body).is_err(), "zone beyond 16 bits");
        let mut body = translocate_record(0);
        body[POSITION..POSITION + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(eqmac_translocate(&body).is_err(), "no place");
    }
}
