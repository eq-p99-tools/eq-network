//! Corpses that belong to players: consenting another player to drag them
//! (`/consent`, `/deny`), summoning one that lies close (`/corpse`), and
//! dragging one along (`/corpsedrag`, `/corpsedrop`). Servers answer a
//! consent with `OP_ConsentResponse`, for the owner and the one consented,
//! and say the rest in their own string table messages; a dragged corpse
//! moves as any spawn does.
//!
//! Layout reference: `EQEmu`'s `Consent_Struct`, `ConsentResponse_Struct`,
//! `CorpseDrag_Struct` and `GMSummon_Struct` (`common/eq_packet_structs.h`
//! and `common/patches/titanium_structs.h`), and `Client::Handle_OP_Consent`,
//! `Handle_OP_CorpseDrag`, `Handle_OP_CorpseDrop` and `OPGMSummon`
//! (`zone/client_packet.cpp`, `zone/client_process.cpp`) for the rules.
use crate::command::EncodedCommand;
use anyhow::{ensure, Result};
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
