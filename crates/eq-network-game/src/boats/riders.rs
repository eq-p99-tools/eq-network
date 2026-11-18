//! Titanium's riding wire: telling the server which boat the player stands
//! on, and the server's word that a boat carries its riders into another
//! zone (reads R36, R36b).
//!
//! - Once a second the program names the boat under the player. The first
//!   linked tick sends `OP_BoardBoat` with the boat's raw spawn name; two
//!   unlinked ticks in a row forget the name and send `OP_LeaveBoat`
//!   (0x422198).
//! - The server's 0x0b71 names a boat and a zone. Only when that name
//!   matches the player's saved vehicle name does the program zone with the
//!   boat, with reason 2 (0x45c277).
//! - At arrival the received profile names the vehicle; the program looks
//!   it up among the spawns, and sends 0x3710 with the name when it is not
//!   there (0x4aba14, 0x4a8afa).
use crate::{command::EncodedCommand, movement::CarrierSample, world::Position};
use anyhow::{ensure, Result};
use serde::Serialize;

/// How the player arrives in a zone whose profile names a vehicle.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Arrival {
    /// The vehicle is here. The host stands the player at `local` in its
    /// frame, floor-sweeps from 1 above, retries at (0, 0, 25) in the
    /// frame, and offers the result (0x4aba14).
    Aboard {
        /// The vehicle's raw saved name.
        name: String,
        /// The received report of the spawn the name found.
        sample: CarrierSample,
        /// The profile's position fields as received: x from 13116, y from
        /// 13120, z from 13124, and the heading.
        profile: Position,
        /// Where the program restores the rider in the boat's frame from
        /// them (read R36c): the profile's y and z as x and y, and the z
        /// the departure saved.
        local: Position,
    },
    /// The vehicle is not here. The session asked for it (0x3710) and holds
    /// the player until the server carries them on, or 60 s pass.
    Waiting {
        /// The vehicle's raw saved name.
        name: String,
    },
    /// The player stands at the zone's safe point, the vehicle forgotten.
    SafePoint {
        /// The safe point.
        position: Position,
    },
    /// The host's placement on the vehicle was taken.
    Placed {
        /// Where the player now stands.
        position: Position,
    },
}

/// `OP_BoardBoat`: the boat the player now stands on, by its raw name.
pub const BOARD_OPCODE: u16 = 0x4298;
/// `OP_LeaveBoat`: the player no longer stands on a boat. Empty.
pub const LEAVE_OPCODE: u16 = 0x67c9;
/// The server's vehicle transfer: a boat name and a zone (R36 §2).
pub const TRANSFER_OPCODE: u16 = 0x0b71;
/// The client's word that its saved vehicle is missing at arrival (R36 §3).
pub const MISSING_OPCODE: u16 = 0x3710;
/// The vehicle transfer's fixed length: the name, then the zone.
const TRANSFER_LENGTH: usize = 0x44;
/// The name field's length in the transfer, the save and the profile.
pub const NAME_LENGTH: usize = 64;
/// The reason a zone request carries when the player zones with a boat.
pub const ZONE_REASON: u32 = 2;
/// Where the Titanium profile, counted from its checksum, keeps the vehicle
/// name (0x3374 in the program's buffer; R36b §5).
pub const PROFILE_NAME: std::ops::Range<usize> = 0x3378..0x3378 + NAME_LENGTH;

/// `OP_BoardBoat`: the raw spawn name, its NUL and three more bytes, which
/// the program copies from the spawn record after the name (R36b §2). This
/// client sends those three as zero; their meaning is unread (6.478).
///
/// # Errors
/// Rejects an empty name, one with a NUL, or one too long for the field.
pub fn titanium_board(name: &str) -> Result<EncodedCommand> {
    let mut body = checked(name)?;
    body.extend([0; 4]);
    Ok(EncodedCommand {
        opcode: BOARD_OPCODE,
        body,
    })
}

/// `OP_LeaveBoat`, with no body.
#[must_use]
pub fn titanium_leave() -> EncodedCommand {
    EncodedCommand {
        opcode: LEAVE_OPCODE,
        body: Vec::new(),
    }
}

/// 0x3710: the raw saved vehicle name and its NUL (R36b §3).
///
/// # Errors
/// Rejects an empty name, one with a NUL, or one too long for the field.
pub fn titanium_missing(name: &str) -> Result<EncodedCommand> {
    let mut body = checked(name)?;
    body.push(0);
    Ok(EncodedCommand {
        opcode: MISSING_OPCODE,
        body,
    })
}

fn checked(name: &str) -> Result<Vec<u8>> {
    ensure!(
        !name.is_empty() && name.len() < NAME_LENGTH && !name.as_bytes().contains(&0),
        "invalid vehicle name"
    );
    Ok(name.as_bytes().to_vec())
}

/// The server's vehicle transfer, as it came.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Transfer {
    /// The boat it names, raw.
    pub name: String,
    /// The zone the boat goes to.
    pub zone_id: u32,
}

impl Transfer {
    /// Reads the fixed 0x44-byte body.
    ///
    /// # Errors
    /// Rejects any other length and an empty name.
    pub fn titanium(body: &[u8]) -> Result<Self> {
        ensure!(
            body.len() == TRANSFER_LENGTH,
            "invalid vehicle transfer length"
        );
        let name = until_nul(&body[..NAME_LENGTH]);
        ensure!(!name.is_empty(), "vehicle transfer names no boat");
        Ok(Self {
            name: String::from_utf8_lossy(name).into_owned(),
            zone_id: u32::from_le_bytes(body[NAME_LENGTH..].try_into()?),
        })
    }

    /// Whether this transfer names the player's saved vehicle: both names
    /// cleaned, then compared ignoring ASCII case (0x45c277, 0x621e9c).
    #[must_use]
    pub fn names(&self, saved: &str) -> bool {
        !saved.is_empty() && cleaned(&self.name).eq_ignore_ascii_case(&cleaned(saved))
    }
}

/// The vehicle name a Titanium profile carries, or None when the field is
/// empty, as `EQEmu` leaves it.
#[must_use]
pub fn titanium_profile_name(profile: &[u8]) -> Option<String> {
    let name = until_nul(profile.get(PROFILE_NAME)?);
    (!name.is_empty()).then(|| String::from_utf8_lossy(name).into_owned())
}

/// A name with its digits and hash signs taken out, as the program cleans
/// names before comparing them (0x492e6e; R36b §4).
#[must_use]
pub fn cleaned(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_ascii_digit() && *c != '#')
        .collect()
}

/// The spawn a saved vehicle name finds at arrival: both names cleaned,
/// the spawn's starting with the saved one ignoring ASCII case, the
/// shortest such spawn name winning (0x47981d). Ties keep the first.
#[must_use]
pub fn find<'a, I>(saved: &str, spawns: I) -> Option<u16>
where
    I: IntoIterator<Item = (u16, &'a str)>,
{
    let saved = cleaned(saved);
    if saved.is_empty() {
        return None;
    }
    spawns
        .into_iter()
        .filter_map(|(id, name)| {
            let name = cleaned(name);
            (name.len() >= saved.len()
                && name.as_bytes()[..saved.len()].eq_ignore_ascii_case(saved.as_bytes()))
            .then_some((name.len(), id))
        })
        .min_by_key(|(length, _)| *length)
        .map(|(_, id)| id)
}

fn until_nul(bytes: &[u8]) -> &[u8] {
    bytes
        .iter()
        .position(|byte| *byte == 0)
        .map_or(bytes, |end| &bytes[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_leave_and_missing_carry_the_raw_name_at_the_read_lengths() {
        let board = titanium_board("Sabrina00").unwrap();
        assert_eq!(board.opcode, BOARD_OPCODE);
        assert_eq!(board.body, b"Sabrina00\0\0\0\0");
        assert_eq!(board.body.len(), "Sabrina00".len() + 4);
        let missing = titanium_missing("Sabrina00").unwrap();
        assert_eq!(missing.opcode, MISSING_OPCODE);
        assert_eq!(missing.body, b"Sabrina00\0");
        assert_eq!(titanium_leave().body, Vec::<u8>::new());
        assert!(titanium_board("").is_err());
        assert!(titanium_board(&"x".repeat(64)).is_err());
        assert!(titanium_missing("a\0b").is_err());
    }

    #[test]
    fn a_transfer_names_the_saved_vehicle_once_cleaned_and_ignoring_case() {
        let mut body = vec![0; TRANSFER_LENGTH];
        body[..10].copy_from_slice(b"SirensBane");
        body[64..].copy_from_slice(&69u32.to_le_bytes());
        let transfer = Transfer::titanium(&body).unwrap();
        assert_eq!(transfer.zone_id, 69);
        assert!(transfer.names("sirensbane00"));
        assert!(transfer.names("#SIRENSBANE"));
        assert!(!transfer.names("SirensBan"));
        assert!(!transfer.names(""));
        assert!(Transfer::titanium(&body[..0x43]).is_err());
        assert!(Transfer::titanium(&[0; TRANSFER_LENGTH]).is_err());
    }

    #[test]
    fn arrival_finds_the_shortest_spawn_starting_with_the_saved_name() {
        let spawns = [
            (4, "SirensBane_Deck01"),
            (9, "SirensBane02"),
            (5, "Sabrina00"),
        ];
        assert_eq!(find("sirensbane00", spawns), Some(9));
        assert_eq!(find("Sabrina", spawns), Some(5));
        assert_eq!(find("Stormbreaker", spawns), None);
        assert_eq!(find("00", spawns), None);
    }

    #[test]
    fn the_profile_names_its_vehicle_only_when_the_field_is_set() {
        let mut profile = vec![0; 19592];
        assert_eq!(titanium_profile_name(&profile), None);
        profile[PROFILE_NAME][..9].copy_from_slice(b"Sabrina00");
        assert_eq!(
            titanium_profile_name(&profile).as_deref(),
            Some("Sabrina00")
        );
        assert_eq!(titanium_profile_name(&profile[..100]), None);
    }
}
