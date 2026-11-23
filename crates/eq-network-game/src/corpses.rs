//! Corpses that belong to players: a player becoming one, consenting
//! another player to drag them (`/consent`, `/deny`), summoning one that
//! lies close (`/corpse`), and dragging one along (`/corpsedrag`,
//! `/corpsedrop`). Servers answer a consent with `OP_ConsentResponse`, for
//! the owner and the one consented, and say the rest in their own string
//! table messages; a dragged corpse moves as any spawn does on Titanium,
//! and `EQMac`'s servers tell where any corpse moves (`OP_CorpsePosition`).
//!
//! Layout reference: `EQEmu`'s `BecomeCorpse_Struct`, `Consent_Struct`,
//! `ConsentResponse_Struct`, `CorpseDrag_Struct` and `GMSummon_Struct`
//! (`common/eq_packet_structs.h` and `common/patches/titanium_structs.h`),
//! and `Client::Death`, `Client::Handle_OP_Consent`, `Handle_OP_CorpseDrag`,
//! `Handle_OP_CorpseDrop` and `OPGMSummon` (`zone/attack.cpp`,
//! `zone/client_packet.cpp`, `zone/client_process.cpp`) for the rules.
use crate::command::EncodedCommand;
use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// `OP_Consent`: let a player drag the player's corpses.
pub const CONSENT_OPCODE: u16 = 0x1081;
/// `OP_ConsentDeny`: take that back.
pub const DENY_OPCODE: u16 = 0x4e8c;
/// `OP_ConsentResponse`: a consent given or taken back, in a zone with a
/// corpse.
pub const CONSENT_RESPONSE_OPCODE: u16 = 0x6380;
/// `OP_CorpseDrag`: start dragging a corpse.
pub const DRAG_OPCODE: u16 = 0x50c0;
/// `OP_CorpseDrop`: stop dragging one corpse, or all.
pub const DROP_OPCODE: u16 = 0x7c7c;
/// `OP_GMSummon`, which the official client also sends for `/corpse`.
pub const SUMMON_OPCODE: u16 = 0x1edc;
/// `OP_BecomeCorpse`: a player became a corpse where they lie.
pub const BECOME_CORPSE_OPCODE: u16 = 0x4dbc;
/// `OP_CorpsePosition`: a corpse moved (`EQMac`).
pub const EQMAC_POSITION_OPCODE: u16 = 0x2140;
/// `OP_Consent` on TAKP and Quarm (Mac 0x40b7). It toggles: a name already
/// consented is taken back.
pub const EQMAC_CONSENT_OPCODE: u16 = 0xb740;
/// `OP_ConsentResponse` on TAKP and Quarm (Mac 0x40d5), the same 161 bytes,
/// sent only to the player consented.
pub const EQMAC_CONSENT_RESPONSE_OPCODE: u16 = 0xd540;
/// `OP_GMSummon` on TAKP and Quarm (Mac 0x40c5).
pub const EQMAC_SUMMON_OPCODE: u16 = 0xc540;
/// `OP_CorpseDrag` on TAKP and Quarm (Mac 0x4114).
pub const EQMAC_DRAG_OPCODE: u16 = 0x1441;
/// `OP_CorpseDrop` on Quarm alone (0x1337 on its wire): Quarm's own
/// addition, which TAKP neither lists nor handles.
pub const QUARM_DROP_OPCODE: u16 = 0x3713;
/// The longest name a consent carries: `EQEmu` takes fewer than 64 bytes
/// with the NUL.
pub const MAX_NAME: usize = 62;
/// Name fields in the drag and summon requests.
const NAME_FIELD: usize = 64;

/// A consent given or taken back, as the server told the owner or the
/// player consented (`ConsentResponse_Struct`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Consent {
    /// The player let drag the corpses, or `group`, `raid` or `guild`.
    pub granted: String,
    /// The corpses' owner.
    pub owner: String,
    /// Given, or taken back.
    pub given: bool,
    /// The zone whose corpses it covers, by its long name.
    pub zone: String,
}

fn text(field: &[u8]) -> String {
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

fn name_field(name: &str) -> Result<[u8; NAME_FIELD]> {
    ensure!(
        !name.is_empty() && name.len() < NAME_FIELD && !name.contains('\0'),
        "invalid name for a corpse request"
    );
    let mut field = [0; NAME_FIELD];
    field[..name.len()].copy_from_slice(name.as_bytes());
    Ok(field)
}

/// Consents a player to drag the player's corpses, or takes it back
/// (`Consent_Struct`: the name and its NUL).
///
/// # Errors
/// Rejects an empty name, one longer than [`MAX_NAME`] bytes, or one holding
/// a NUL.
pub fn consent(name: &str, given: bool) -> Result<EncodedCommand> {
    ensure!(
        !name.is_empty() && name.len() <= MAX_NAME && !name.contains('\0'),
        "invalid consent name"
    );
    let mut body = name.as_bytes().to_vec();
    body.push(0);
    Ok(EncodedCommand {
        opcode: if given { CONSENT_OPCODE } else { DENY_OPCODE },
        body,
    })
}

/// Starts dragging a corpse (`CorpseDrag_Struct`): its spawn name and the
/// dragger's.
///
/// # Errors
/// Rejects names that do not fit their fields.
pub fn drag(corpse: &str, dragger: &str) -> Result<EncodedCommand> {
    let mut body = Vec::with_capacity(152);
    body.extend_from_slice(&name_field(corpse)?);
    body.extend_from_slice(&name_field(dragger)?);
    body.resize(152, 0);
    Ok(EncodedCommand {
        opcode: DRAG_OPCODE,
        body,
    })
}

/// Stops dragging one corpse, by its spawn name, or every corpse.
///
/// # Errors
/// Rejects a name that does not fit.
pub fn release(corpse: Option<&str>) -> Result<EncodedCommand> {
    let body = match corpse {
        Some(name) => {
            name_field(name)?;
            let mut body = name.as_bytes().to_vec();
            body.push(0);
            body
        }
        // One byte stops them all.
        None => vec![0],
    };
    Ok(EncodedCommand {
        opcode: DROP_OPCODE,
        body,
    })
}

/// Summons a corpse that lies close to the player (`GMSummon_Struct`, as the
/// official client sends it for `/corpse`): the corpse's spawn name and the
/// player's.
///
/// # Errors
/// Rejects names that do not fit their fields.
pub fn summon(corpse: &str, player: &str) -> Result<EncodedCommand> {
    let mut body = Vec::with_capacity(152);
    body.extend_from_slice(&name_field(corpse)?);
    body.extend_from_slice(&name_field(player)?);
    body.resize(152, 0);
    Ok(EncodedCommand {
        opcode: SUMMON_OPCODE,
        body,
    })
}

/// Consents a player on TAKP or Quarm: the name and its NUL on the `EQMac`
/// opcode, which both servers' `Handle_OP_Consent` read at 1 to 63 bytes
/// (Quarm fc75fcf, TAKP 44afcbf). Consenting a name already consented takes
/// it back; neither server has a separate deny.
///
/// # Errors
/// Rejects an empty name, one longer than [`MAX_NAME`] bytes, or one holding
/// a NUL.
pub fn eqmac_consent(name: &str) -> Result<EncodedCommand> {
    let mut command = consent(name, true)?;
    command.opcode = EQMAC_CONSENT_OPCODE;
    Ok(command)
}

/// Summons a corpse on TAKP or Quarm: the same 152-byte `GMSummon_Struct`
/// on the `EQMac` opcode. Both servers summon a corpse it names for anyone,
/// under `Corpse::Summon`'s consent, lock and range checks, and drop any
/// other name below GM status.
///
/// # Errors
/// Rejects names that do not fit their fields.
pub fn eqmac_summon(corpse: &str, player: &str) -> Result<EncodedCommand> {
    let mut command = summon(corpse, player)?;
    command.opcode = EQMAC_SUMMON_OPCODE;
    Ok(command)
}

/// Starts dragging a corpse on TAKP or Quarm: the same 152-byte
/// `CorpseDrag_Struct` on the `EQMac` opcode, the only size both servers
/// read.
///
/// # Errors
/// Rejects names that do not fit their fields.
pub fn eqmac_drag(corpse: &str, dragger: &str) -> Result<EncodedCommand> {
    let mut command = drag(corpse, dragger)?;
    command.opcode = EQMAC_DRAG_OPCODE;
    Ok(command)
}

/// Stops dragging on Quarm: one byte stops every corpse, and one corpse
/// takes the whole 152-byte `CorpseDrag_Struct` (Quarm's
/// `Handle_OP_CorpseDrop`), the dragger named too.
///
/// # Errors
/// Rejects names that do not fit their fields.
pub fn quarm_drop(corpse: Option<&str>, dragger: &str) -> Result<EncodedCommand> {
    let body = match corpse {
        Some(name) => drag(name, dragger)?.body,
        None => vec![0],
    };
    Ok(EncodedCommand {
        opcode: QUARM_DROP_OPCODE,
        body,
    })
}

/// Decodes `OP_ConsentResponse`.
///
/// # Errors
/// Rejects a malformed length.
pub fn decode_consent(body: &[u8]) -> Result<Consent> {
    ensure!(body.len() == 161, "invalid consent answer length");
    Ok(Consent {
        granted: text(&body[..64]),
        owner: text(&body[64..128]),
        given: body[128] != 0,
        zone: text(&body[129..161]),
    })
}

/// Decodes `OP_BecomeCorpse` (16 bytes: the spawn as a 32-bit ID, then
/// where it lies, y first): the spawn, and where it lies, x, y and z. `EQEmu`
/// sends it to the player just before word of their death, and to everyone
/// else in the zone as a player leaves a corpse, which keeps the player's
/// spawn ID (`Client::Death`, `Client::Sacrifice`, `EntityList::AddCorpse`).
///
/// # Errors
/// Rejects any length other than 16 bytes, a spawn that names none, and a
/// place that is not finite.
pub fn became_corpse(body: &[u8]) -> Result<(u16, [f32; 3])> {
    ensure!(body.len() == 16, "invalid corpse length");
    let word = |at: usize| u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]);
    let spawn_id = u16::try_from(word(0))
        .ok()
        .filter(|&id| id != 0)
        .context("corpse of no spawn")?;
    let [y, x, z] = [4, 8, 12].map(|at| f32::from_bits(word(at)));
    ensure!(
        [x, y, z].iter().all(|value| value.is_finite()),
        "corpse at no place"
    );
    Ok((spawn_id, [x, y, z]))
}

/// Decodes `EQMac`'s `OP_CorpsePosition` (16 bytes: the corpse, two unused
/// bytes, then where it lies, y first): the corpse, and where it lies, x, y
/// and z. TAKP and Quarm send it to the zone as a corpse moves
/// (`Mob::SendRealPosition`): summoned, returned, a game master's
/// `#summon`, and on Quarm all the while it is dragged.
///
/// # Errors
/// Rejects any length other than 16 bytes, a corpse that names no spawn,
/// and a place that is not finite.
pub fn eqmac_position(body: &[u8]) -> Result<(u16, [f32; 3])> {
    ensure!(body.len() == 16, "invalid EQMac corpse position length");
    let spawn_id = u16::from_le_bytes([body[0], body[1]]);
    ensure!(spawn_id != 0, "corpse position of no spawn");
    let [y, x, z] = [4, 8, 12]
        .map(|at| f32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]));
    ensure!(
        [x, y, z].iter().all(|value| value.is_finite()),
        "corpse at no place"
    );
    Ok((spawn_id, [x, y, z]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_eqmac_corpse_moves_without_turning() {
        let mut body = 9u16.to_le_bytes().to_vec();
        body.extend_from_slice(&[0, 0]);
        for value in [2.0f32, 1.0, 3.0] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        assert_eq!(eqmac_position(&body).unwrap(), (9, [1.0, 2.0, 3.0]));
        assert_eq!(
            crate::quarm::updates(EQMAC_POSITION_OPCODE, &body).unwrap(),
            [crate::world::WorldEvent::CorpseMoved {
                spawn_id: 9,
                position: [1.0, 2.0, 3.0]
            }]
        );
        assert!(eqmac_position(&body[..15]).is_err());
        let mut nowhere = body.clone();
        nowhere[12..16].copy_from_slice(&f32::INFINITY.to_le_bytes());
        assert!(eqmac_position(&nowhere).is_err());
        body[..2].fill(0);
        assert!(eqmac_position(&body).is_err());
    }

    #[test]
    fn a_player_becomes_a_corpse_where_they_lie() {
        let mut body = Vec::new();
        body.extend_from_slice(&7u32.to_le_bytes());
        for value in [1748.0f32, 263.0, 27.0] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        assert_eq!(became_corpse(&body).unwrap(), (7, [263.0, 1748.0, 27.0]));
        assert!(became_corpse(&body[..15]).is_err());
        let mut none = body.clone();
        none[..4].copy_from_slice(&0u32.to_le_bytes());
        assert!(became_corpse(&none).is_err(), "no spawn");
        let mut far = body.clone();
        far[..4].copy_from_slice(&0x1_0000u32.to_le_bytes());
        assert!(became_corpse(&far).is_err(), "beyond 16 bits");
        body[4..8].copy_from_slice(&f32::INFINITY.to_le_bytes());
        assert!(became_corpse(&body).is_err(), "no place");
    }

    #[test]
    fn consents_name_a_player_and_answers_say_where() {
        assert_eq!(
            consent("Helper", true).unwrap(),
            EncodedCommand {
                opcode: CONSENT_OPCODE,
                body: b"Helper\0".to_vec(),
            }
        );
        assert_eq!(consent("Helper", false).unwrap().opcode, DENY_OPCODE);
        assert!(consent("", true).is_err());
        assert!(consent(&"x".repeat(63), true).is_err());
        let mut body = vec![0; 161];
        body[..6].copy_from_slice(b"Helper");
        body[64..69].copy_from_slice(b"Owner");
        body[128] = 1;
        body[129..145].copy_from_slice(b"The Qeynos Hills");
        assert_eq!(
            decode_consent(&body).unwrap(),
            Consent {
                granted: "Helper".into(),
                owner: "Owner".into(),
                given: true,
                zone: "The Qeynos Hills".into(),
            }
        );
        assert!(decode_consent(&body[..160]).is_err());
    }

    #[test]
    fn drags_drops_and_summons_name_the_corpse() {
        let drag = drag("Owner's corpse0", "Helper").unwrap();
        assert_eq!(drag.opcode, DRAG_OPCODE);
        assert_eq!(drag.body.len(), 152);
        assert_eq!(&drag.body[..16], b"Owner's corpse0\0");
        assert_eq!(&drag.body[64..71], b"Helper\0");
        assert_eq!(
            release(Some("Owner's corpse0")).unwrap().body,
            b"Owner's corpse0\0"
        );
        assert_eq!(release(None).unwrap().body, [0]);
        let summon = summon("Owner's corpse0", "Owner").unwrap();
        assert_eq!(summon.opcode, SUMMON_OPCODE);
        assert_eq!(summon.body.len(), 152);
        assert_eq!(&summon.body[64..70], b"Owner\0");
        assert!(super::drag("", "Helper").is_err());
    }

    #[test]
    fn eqmac_corpse_requests_use_the_mac_opcodes_and_sizes() {
        let consent = eqmac_consent("Helper").unwrap();
        assert_eq!(consent.opcode, 0xb740);
        assert_eq!(consent.body, b"Helper\0");
        assert!(eqmac_consent(&"a".repeat(MAX_NAME + 1)).is_err());
        assert_eq!(eqmac_consent(&"a".repeat(MAX_NAME)).unwrap().body.len(), 63);
        let summon = eqmac_summon("Owner's corpse0", "Owner").unwrap();
        assert_eq!(summon.opcode, 0xc540);
        assert_eq!(
            summon.body,
            super::summon("Owner's corpse0", "Owner").unwrap().body
        );
        assert_eq!(summon.body.len(), 152);
        let drag = eqmac_drag("Owner's corpse0", "Helper").unwrap();
        assert_eq!(drag.opcode, 0x1441);
        assert_eq!(drag.body.len(), 152);
        assert_eq!(&drag.body[..16], b"Owner's corpse0\0");
        assert_eq!(&drag.body[64..71], b"Helper\0");
        let all = quarm_drop(None, "Helper").unwrap();
        assert_eq!(all.opcode, 0x3713);
        assert_eq!(all.body, [0]);
        let one = quarm_drop(Some("Owner's corpse0"), "Helper").unwrap();
        assert_eq!(one.body, drag.body);
        assert!(quarm_drop(Some(""), "Helper").is_err());
    }
}
