//! Titanium player inspection. The packet contains display fields and spawn
//! identities. It contains no item IDs or inventory instances.
use crate::{command::EncodedCommand, socials::INSPECT_REQUEST_OPCODE, world::until_nul};
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_InspectAnswer` in both directions for Titanium.
pub const INSPECT_ANSWER_OPCODE: u16 = 0x2403;
/// Titanium refuses to start an inspection at or beyond this distance.
/// Uncited; recording checklist 6.400.
pub const REQUEST_RANGE: f32 = 20.0;
const SLOTS: usize = 22;
const NAME: usize = 64;
const ICONS: usize = 8 + SLOTS * NAME;
const TEXT: usize = ICONS + SLOTS * 4;
const LENGTH: usize = TEXT + 288;

/// One display entry in Titanium equipment-slot order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Equipment {
    /// The name sent for this slot, if any.
    pub name: String,
    /// Its icon ID. The empty icon sentinel is absent.
    pub icon: Option<u32>,
}

/// An equipment report as sent, before correlating it to a request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Inspection {
    /// The spawn receiving the answer.
    pub recipient_id: u16,
    /// The player whose equipment is being shown.
    pub player_id: u16,
    /// Display entries in equipment-slot order, including empty entries.
    pub equipment: Vec<Equipment>,
    /// The inspect message sent with the report.
    pub text: String,
}

/// A report correlated to an admitted request and a current player spawn.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlayerInspection {
    /// The admission that requested and received this view.
    pub session_id: u64,
    /// The name received for the current spawn.
    pub name: String,
    /// The corresponding report.
    pub report: Inspection,
}

fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(body[at..at + 4].try_into().expect("fixed inspection field"))
}

/// Decodes the exact 1,792-byte Titanium record.
///
/// # Errors
/// Rejects the wrong length or invalid spawn identities.
pub fn titanium_answer(body: &[u8]) -> Result<Inspection> {
    ensure!(body.len() == LENGTH, "invalid inspection length");
    let recipient_id = u16::try_from(word(body, 0))?;
    let player_id = u16::try_from(word(body, 4))?;
    ensure!(
        recipient_id != 0 && player_id != 0,
        "invalid inspection spawn"
    );
    let equipment = (0..SLOTS)
        .map(|index| {
            let start = 8 + index * NAME;
            let icon = word(body, ICONS + index * 4);
            Equipment {
                name: String::from_utf8_lossy(until_nul(&body[start..start + NAME])).into_owned(),
                icon: (icon != u32::MAX).then_some(icon),
            }
        })
        .collect();
    Ok(Inspection {
        recipient_id,
        player_id,
        equipment,
        text: String::from_utf8_lossy(until_nul(&body[TEXT..])).into_owned(),
    })
}

/// Builds the request from the target's spawn and the admitted player's spawn.
///
/// # Errors
/// Both spawn IDs must be known.
pub fn titanium_request(target: u16, player: u16) -> Result<EncodedCommand> {
    ensure!(target != 0 && player != 0, "invalid inspection spawn");
    Ok(EncodedCommand {
        opcode: INSPECT_REQUEST_OPCODE,
        body: [
            u32::from(target).to_le_bytes(),
            u32::from(player).to_le_bytes(),
        ]
        .concat(),
    })
}

/// Replies to a received request using the native placeholder equipment fields.
/// `EQEmu` fills equipment server-side. The note is the player's own configured text.
///
/// # Errors
/// Both spawn IDs and bounded NUL-free text are required.
pub fn titanium_reply(inspector: u16, own: u16, name: &str, note: &str) -> Result<EncodedCommand> {
    ensure!(inspector != 0 && own != 0, "invalid inspection spawn");
    ensure!(
        note.len() < 256 && !note.as_bytes().contains(&0),
        "invalid inspect note"
    );
    ensure!(
        !name.is_empty() && name.len() < 32 && !name.as_bytes().contains(&0),
        "invalid inspect name"
    );
    let mut body = vec![0; LENGTH];
    body[..4].copy_from_slice(&u32::from(inspector).to_le_bytes());
    body[4..8].copy_from_slice(&u32::from(own).to_le_bytes());
    body[ICONS..TEXT].fill(u8::MAX);
    body[TEXT..TEXT + note.len()].copy_from_slice(note.as_bytes());
    body[TEXT + 256..TEXT + 256 + name.len()].copy_from_slice(name.as_bytes());
    Ok(EncodedCommand {
        opcode: INSPECT_ANSWER_OPCODE,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_and_reply_keep_reciprocal_spawns_and_no_local_equipment() {
        let request = titanium_request(312, 12).unwrap();
        assert_eq!(request.body, [56, 1, 0, 0, 12, 0, 0, 0]);
        let reply = titanium_reply(12, 312, "Player", "A note").unwrap();
        assert_eq!(reply.body.len(), 1792);
        assert_eq!(&reply.body[1760..1767], b"Player\0");
        let answer = titanium_answer(&reply.body).unwrap();
        assert_eq!((answer.recipient_id, answer.player_id), (12, 312));
        assert_eq!(answer.text, "A note");
        assert!(answer
            .equipment
            .iter()
            .all(|slot| slot.name.is_empty() && slot.icon.is_none()));
    }

    #[test]
    fn received_slots_do_not_fabricate_names_or_icons() {
        let mut body = titanium_reply(12, 312, "Player", "").unwrap().body;
        body[8..12].copy_from_slice(b"Boot");
        body[ICONS + 4..ICONS + 8].copy_from_slice(&33u32.to_le_bytes());
        let answer = titanium_answer(&body).unwrap();
        assert_eq!(
            answer.equipment[0],
            Equipment {
                name: "Boot".into(),
                icon: None
            }
        );
        assert_eq!(
            answer.equipment[1],
            Equipment {
                name: String::new(),
                icon: Some(33)
            }
        );
        assert!(titanium_answer(&body[..LENGTH - 1]).is_err());
        body[2] = 1;
        assert!(titanium_answer(&body).is_err());
    }
}
