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

/// The length of the offer and the answer.
const LENGTH: usize = 228;
/// Where each part of the offer lies.
const ZONE: usize = 4;
const INSTANCE: usize = 6;
const POSITION: usize = 8;
const CASTER_NAME: usize = 92;
const SPELL: usize = 156;
const CORPSE_NAME: usize = 160;
const ACTION: usize = 224;
/// The length of each name's field.
const NAME: usize = 64;

/// An offer to resurrect the player.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResurrectionOffer {
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
/// Rejects an offer of the wrong length, and one without a caster or a
/// corpse.
pub fn titanium_offer(body: &[u8]) -> Result<ResurrectionOffer> {
    ensure!(body.len() == LENGTH, "invalid resurrection offer length");
    let float =
        |at: usize| f32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]);
    let caster = name(body, CASTER_NAME);
    let corpse = name(body, CORPSE_NAME);
    ensure!(
        !caster.is_empty() && !corpse.is_empty(),
        "resurrection offer without a caster or a corpse"
    );
    // The offer carries y before x.
    let (y, x, z) = (float(POSITION), float(POSITION + 4), float(POSITION + 8));
    ensure!(
        [x, y, z].iter().all(|value| value.is_finite()),
        "resurrection offer at no place"
    );
    Ok(ResurrectionOffer {
        caster,
        corpse,
        spell_id: u32::from_le_bytes([
            body[SPELL],
            body[SPELL + 1],
            body[SPELL + 2],
            body[SPELL + 3],
        ]),
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

/// Encodes the player's answer: the offer as it came, with the choice.
#[must_use]
pub fn titanium_answer(offer: &ResurrectionOffer, accept: bool) -> EncodedCommand {
    let mut body = offer.body.clone();
    body[ACTION..ACTION + 4].copy_from_slice(&u32::from(accept).to_le_bytes());
    EncodedCommand {
        opcode: ANSWER_OPCODE,
        body,
    }
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

    /// Where the offer names the player.
    const PLAYER_NAME: usize = 24;

    /// An offer from Tester, with Resurrection (392), to the corpse of
    /// Example in zone 4.
    fn offer() -> Vec<u8> {
        let mut body = vec![0; LENGTH];
        body[ZONE..ZONE + 2].copy_from_slice(&4u16.to_le_bytes());
        body[POSITION..POSITION + 4].copy_from_slice(&1748.0f32.to_le_bytes());
        body[POSITION + 4..POSITION + 8].copy_from_slice(&263.0f32.to_le_bytes());
        body[POSITION + 8..POSITION + 12].copy_from_slice(&27.0f32.to_le_bytes());
        body[PLAYER_NAME..PLAYER_NAME + 7].copy_from_slice(b"Example");
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
}
