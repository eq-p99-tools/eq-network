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

/// The `EQMac` program's own answer to an inspection (TAKP program read row
/// 11e: 0x4e769d), which TAKP and Quarm pass on unread to the one who asked
/// (`Client::Handle_OP_InspectAnswer`, TAKP 44afcbf, Quarm fc75fcf, both
/// checking only its 1,744-byte size).
pub mod eqmac {
    use super::Equipment;
    use crate::command::EncodedCommand;
    use anyhow::{ensure, Result};

    /// `EQMac`'s `OP_InspectAnswer` (the program's 0x40b6).
    pub const ANSWER_OPCODE: u16 = 0xb640;
    /// The answer's length.
    pub const LENGTH: usize = 0x6d0;
    /// How many worn places it names, in the program's order: the
    /// character's item pointers +0xd78 to +0xdcc, inferred to be `EQMac`'s
    /// slots 0 to 21.
    pub const PLACES: usize = 22;
    const NAME: usize = 64;
    const ICONS: usize = 0x584;
    const TEXT: usize = 0x5b0;
    const TEXT_LENGTH: usize = 256;
    /// The icon the program writes for an empty place.
    pub const NO_ICON: u16 = 0xffff;

    /// The answer to `requester` from the player (`own`): each worn place's
    /// item name and icon, then the player's inspect message.
    ///
    /// The program fills a buffer on its stack and leaves the 32-byte
    /// string at +0x6b0 (from the character's +0x62, inferred to be the
    /// title after the surname) and the bytes it never writes as they
    /// were; this answer leaves all of them zero, so no stale memory is
    /// sent (a labelled departure, 6.621). A name longer than the place
    /// holds is cut, as is the message; an icon past 16 bits, or none for
    /// an item, is written as the empty one.
    ///
    /// # Errors
    /// Refuses a zero spawn on either side, or other than 22 places.
    pub fn answer(
        requester: u16,
        own: u16,
        equipment: &[Equipment],
        text: &str,
    ) -> Result<EncodedCommand> {
        ensure!(requester != 0 && own != 0, "invalid inspection spawn");
        ensure!(equipment.len() == PLACES, "an answer names 22 places");
        let mut body = vec![0; LENGTH];
        body[..2].copy_from_slice(&requester.to_le_bytes());
        body[2..4].copy_from_slice(&own.to_le_bytes());
        for (index, place) in equipment.iter().enumerate() {
            let name = bounded(&place.name, NAME - 1);
            let start = 4 + index * NAME;
            body[start..start + name.len()].copy_from_slice(name);
            let icon = place
                .icon
                .and_then(|icon| u16::try_from(icon).ok())
                .unwrap_or(NO_ICON);
            body[ICONS + index * 2..ICONS + index * 2 + 2].copy_from_slice(&icon.to_le_bytes());
        }
        let text = bounded(text, TEXT_LENGTH - 1);
        body[TEXT..TEXT + text.len()].copy_from_slice(text);
        Ok(EncodedCommand {
            opcode: ANSWER_OPCODE,
            body,
        })
    }

    /// The text's bytes before any NUL, at most `most` of them.
    fn bounded(text: &str, most: usize) -> &[u8] {
        let bytes = text.as_bytes();
        let bytes = bytes
            .iter()
            .position(|byte| *byte == 0)
            .map_or(bytes, |nul| &bytes[..nul]);
        &bytes[..bytes.len().min(most)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mac_answer_is_the_programs_1744_bytes() {
        let mut equipment = vec![
            Equipment {
                name: String::new(),
                icon: None,
            };
            eqmac::PLACES
        ];
        equipment[0] = Equipment {
            name: "Cursor Thing".into(),
            icon: Some(0x1234),
        };
        equipment[2] = Equipment {
            name: "N".repeat(80),
            icon: Some(0x1_0000),
        };
        equipment[21] = Equipment {
            name: "Arrow".into(),
            icon: Some(500),
        };
        let answer = eqmac::answer(0x0102, 7, &equipment, "Hello there").unwrap();
        assert_eq!(answer.opcode, 0xb640);
        let body = answer.body;
        assert_eq!(body.len(), 1744);
        assert_eq!(&body[..4], &[2, 1, 7, 0]);
        assert_eq!(&body[4..17], b"Cursor Thing\0");
        // Cut to 63 bytes, leaving the place's last byte a NUL.
        assert_eq!(&body[4 + 128..4 + 128 + 63], "N".repeat(63).as_bytes());
        assert_eq!(body[4 + 128 + 63], 0);
        assert_eq!(&body[4 + 21 * 64..4 + 21 * 64 + 6], b"Arrow\0");
        let icon =
            |index: usize| u16::from_le_bytes([body[0x584 + index * 2], body[0x585 + index * 2]]);
        assert_eq!(
            (icon(0), icon(1), icon(2), icon(21)),
            (0x1234, 0xffff, 0xffff, 500)
        );
        assert_eq!(&body[0x5b0..0x5bc], b"Hello there\0");
        // The program's unwritten bytes and its 32-byte string are zero.
        assert!(body[0x6b0..].iter().all(|byte| *byte == 0));
        assert!(eqmac::answer(0, 7, &equipment, "").is_err());
        assert!(eqmac::answer(3, 0, &equipment, "").is_err());
        assert!(eqmac::answer(3, 7, &equipment[..21], "").is_err());
        let long = eqmac::answer(3, 7, &equipment, &"x".repeat(300)).unwrap();
        assert_eq!(long.body[0x5b0 + 254], b'x');
        assert_eq!(long.body[0x5b0 + 255], 0);
    }

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
