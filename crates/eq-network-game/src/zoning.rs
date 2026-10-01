//! Titanium death, boundary destinations and zone-transfer layouts.
//!
//! Layout references: EQEmu common/patches/titanium_structs.h and
//! common/eq_packet_structs.h. Spatial boundary detection belongs to the client;
//! the network layer validates its request against current admission state.
use crate::{command::EncodedCommand, world::Position};
use anyhow::{ensure, Result};
use serde::Serialize;
mod points;
pub use points::{ZoneLineDestination, ZonePoint, ZonePoints};

/// `OP_ZoneChange`: the client's transfer request, and the server's answer.
pub const CHANGE_OPCODE: u16 = 0x5dd8;
/// `OP_SendZonepoints`: the destinations the zone numbered for its zone lines.
pub const POINTS_OPCODE: u16 = 0x3eba;
/// `OP_ZonePlayerToBind`: the server returning the player to their bind point.
pub const TO_BIND_OPCODE: u16 = 0x385e;
/// `OP_RequestClientZoneChange`: the server moving the player.
pub const MOVE_OPCODE: u16 = 0x7834;
/// `OP_ZoneServerInfo`: the next zone's address.
pub const HANDOFF_OPCODE: u16 = 0x61b6;

/// A pending destination selected by a server offer or local boundary.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ZoneOffer {
    /// Destination zone ID, or zero for a server-resolved bind destination.
    pub zone_id: u16,
    /// Destination instance ID.
    pub instance_id: u16,
    /// Server coordinates and heading; final admission remains authoritative.
    pub position: Position,
    /// Opaque request reason echoed in the response.
    pub reason: u32,
    /// Whether the server is returning the character to its bind point.
    pub to_bind: bool,
    /// Whether the server asked for the transfer, rather than the client crossing
    /// a zone line.
    pub solicited: bool,
}

impl ZoneOffer {
    /// A non-bind offer to this exact zone/instance relocates without a zone-change reply.
    #[must_use]
    pub fn local_position(&self, current_zone: (u16, u16)) -> Option<Position> {
        (!self.to_bind && self.zone_id != 0 && (self.zone_id, self.instance_id) == current_zone)
            .then_some(self.position)
    }

    /// Encodes the pending transfer request, preserving its destination and reason.
    ///
    /// # Errors
    /// Rejects names that cannot fit the NUL-terminated Titanium name field.
    pub fn response(&self, character: &str) -> Result<EncodedCommand> {
        ensure!(
            !character.is_empty() && character.len() < 64 && !character.contains('\0'),
            "invalid character name"
        );
        ensure!(finite(self.position), "invalid zone coordinates");
        let mut body = vec![0; 88];
        body[..character.len()].copy_from_slice(character.as_bytes());
        body[64..66].copy_from_slice(&self.zone_id.to_le_bytes());
        body[66..68].copy_from_slice(&self.instance_id.to_le_bytes());
        for (offset, value) in [
            (68, self.position.y),
            (72, self.position.x),
            (76, self.position.z),
        ] {
            body[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        body[80..84].copy_from_slice(&self.reason.to_le_bytes());
        Ok(EncodedCommand {
            opcode: CHANGE_OPCODE,
            body,
        })
    }
}

/// One death notification; other entities dying must not interrupt the player.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Death {
    /// Entity that died.
    pub spawn_id: u32,
    /// Entity that dealt the fatal blow, or zero.
    pub killer_id: u32,
    /// Corpse entity ID.
    pub corpse_id: u32,
    /// Bind destination reported by the server; not an instruction to zone yet.
    pub bind_zone_id: u32,
}

/// Decode a Titanium death notification without interpreting NPC death as player death.
///
/// # Errors
/// Rejects malformed lengths.
pub fn death(body: &[u8]) -> Result<Death> {
    ensure!(body.len() == 32, "invalid Titanium death length");
    Ok(Death {
        spawn_id: word(body, 0),
        killer_id: word(body, 4),
        corpse_id: word(body, 8),
        bind_zone_id: word(body, 20),
    })
}

/// Decode a server zone offer; bind packets use X/Y order, solicited packets Y/X.
///
/// # Errors
/// Rejects malformed layouts, unterminated labels, and non-finite coordinates.
pub fn offer(opcode: u16, body: &[u8]) -> Result<ZoneOffer> {
    let to_bind = opcode == TO_BIND_OPCODE;
    ensure!(to_bind || opcode == MOVE_OPCODE, "not a server zone offer");
    if to_bind {
        ensure!(
            body.len() >= 21 && body[20..].contains(&0),
            "invalid bind destination"
        );
    } else {
        ensure!(body.len() == 24, "invalid zone offer length");
    }
    let position = Position {
        x: float(body, if to_bind { 4 } else { 8 }),
        y: float(body, if to_bind { 8 } else { 4 }),
        z: float(body, 12),
        heading: float(body, 16),
    };
    ensure!(finite(position), "invalid zone coordinates");
    Ok(ZoneOffer {
        zone_id: half(body, 0),
        instance_id: half(body, 2),
        position,
        reason: if to_bind { 10 } else { word(body, 20) },
        to_bind,
        solicited: true,
    })
}

/// Verify a zone response belongs to the pending request and active character.
///
/// # Errors
/// Rejects malformed responses, another character, or a mismatched destination.
pub fn approved(body: &[u8], character: &str, pending: &ZoneOffer) -> Result<bool> {
    ensure!(body.len() == 88, "invalid zone response length");
    let end = body[..64]
        .iter()
        .position(|v| *v == 0)
        .ok_or_else(|| anyhow::anyhow!("unterminated zone response name"))?;
    ensure!(
        body[..end].eq_ignore_ascii_case(character.as_bytes()),
        "zone response is for another character"
    );
    let success = i32::from_le_bytes(body[84..88].try_into()?);
    if success != 1 {
        return Ok(false);
    }
    ensure!(
        (pending.zone_id == 0 || half(body, 64) == pending.zone_id)
            && half(body, 66) == pending.instance_id,
        "zone approval changed destination"
    );
    Ok(true)
}

/// Server disposition of a pending zone transfer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum ZoneRejection {
    /// The server cancelled zoning and supplied a position in the current zone.
    Cancelled,
    /// Exact signed response code; unknown server-specific codes are preserved.
    Server(i32),
}

impl std::fmt::Display for ZoneRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Cancelled => {
                return formatter.write_str("Zone transfer cancelled; position restored")
            }
            Self::Server(0) => "Zone transfer rejected",
            Self::Server(-1) => "Destination zone is not ready",
            Self::Server(-6) => "Destination requires an unavailable expansion",
            Self::Server(-7) => "Character does not meet the destination entry requirements",
            Self::Server(_) => "Zone transfer rejected",
        };
        let Self::Server(code) = self else {
            unreachable!()
        };
        write!(formatter, "{message} (server code {code})")
    }
}

/// Server disposition of a pending zone transfer.
#[derive(Clone, Debug, PartialEq)]
pub enum ZoneReply {
    /// Connect to the world server for a destination handoff.
    Approved,
    /// Stay in the current zone without an explicit correction.
    Denied(ZoneRejection),
    /// Zoning was cancelled and the server supplied rewind coordinates.
    Rewind(Position),
}

/// Interprets a zone-change response. A success naming the current zone instead
/// of the requested one means two things in `EQEmu` (zone/zoning.cpp): for a zone
/// line the client crossed, the server cancelled and supplies rewind coordinates
/// (`SendZoneCancel`); for a transfer the server asked for, such as an evacuation
/// or succor within the zone (offered with a stand-in zone), the server moved the
/// character and expects it to zone back in through world (`DoZoneSuccess`).
///
/// # Errors
/// Rejects malformed replies, unexpected destinations and invalid rewind coordinates.
pub fn reply(
    body: &[u8],
    character: &str,
    pending: &ZoneOffer,
    current_zone: (u16, u16),
    heading: f32,
) -> Result<ZoneReply> {
    ensure!(body.len() == 88, "invalid zone response length");
    let returned = (half(body, 64), half(body, 66));
    if pending.zone_id != 0
        && returned == current_zone
        && returned != (pending.zone_id, pending.instance_id)
        && word(body, 84) == 1
    {
        let current = ZoneOffer {
            zone_id: current_zone.0,
            instance_id: current_zone.1,
            ..pending.clone()
        };
        approved(body, character, &current)?;
        if pending.solicited {
            return Ok(ZoneReply::Approved);
        }
        let position = Position {
            x: float(body, 72),
            y: float(body, 68),
            z: float(body, 76),
            heading,
        };
        ensure!(finite(position), "invalid zone cancellation position");
        return Ok(ZoneReply::Rewind(position));
    }
    Ok(if approved(body, character, pending)? {
        ZoneReply::Approved
    } else {
        ZoneReply::Denied(ZoneRejection::Server(i32::from_le_bytes(
            body[84..88].try_into()?,
        )))
    })
}

fn half(body: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(body[offset..offset + 2].try_into().unwrap())
}
fn word(body: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(body[offset..offset + 4].try_into().unwrap())
}
fn float(body: &[u8], offset: usize) -> f32 {
    f32::from_bits(word(body, offset))
}
fn finite(p: Position) -> bool {
    [p.x, p.y, p.z, p.heading].iter().all(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn denial_preserves_signed_codes_and_validates_character_before_reporting() {
        let pending = ZoneOffer {
            zone_id: 42,
            instance_id: 0,
            position: Position::default(),
            reason: 0,
            to_bind: false,
            solicited: false,
        };
        let mut body = pending.response("Example").unwrap().body;
        for code in [0_i32, -1, -2, -3, -6, -7, -12345, 42, i32::MIN] {
            body[84..88].copy_from_slice(&code.to_le_bytes());
            let rejection = ZoneRejection::Server(code);
            assert_eq!(
                reply(&body, "Example", &pending, (7, 0), 0.0).unwrap(),
                ZoneReply::Denied(rejection)
            );
            assert!(rejection
                .to_string()
                .contains(&format!("server code {code}")));
            assert!(reply(&body, "Other", &pending, (7, 0), 0.0).is_err());
        }
        body[84..88].copy_from_slice(&1_i32.to_le_bytes());
        assert_eq!(
            reply(&body, "Example", &pending, (7, 0), 0.0).unwrap(),
            ZoneReply::Approved
        );
        assert!(reply(&body[..87], "Example", &pending, (7, 0), 0.0).is_err());
    }
    #[test]
    fn same_zone_offer_is_local_but_bind_and_instance_changes_require_transfer() {
        let position = Position {
            x: -4.0,
            y: 8.0,
            z: 2.0,
            heading: 64.0,
        };
        let mut offer = ZoneOffer {
            zone_id: 22,
            instance_id: 3,
            position,
            reason: 1,
            to_bind: false,
            solicited: true,
        };
        assert_eq!(offer.local_position((22, 3)), Some(position));
        assert!(offer.local_position((22, 0)).is_none());
        assert!(offer.local_position((21, 3)).is_none());
        offer.to_bind = true;
        assert!(offer.local_position((22, 3)).is_none());
        offer.to_bind = false;
        offer.zone_id = 0;
        assert!(offer.local_position((0, 3)).is_none());
    }

    #[test]
    fn current_zone_success_is_a_rewind_not_a_handoff() {
        let pending = ZoneOffer {
            zone_id: 42,
            instance_id: 0,
            position: Position::default(),
            reason: 0,
            to_bind: false,
            solicited: false,
        };
        let mut body = pending.response("Example").unwrap().body;
        body[64..66].copy_from_slice(&7u16.to_le_bytes());
        body[68..72].copy_from_slice(&12.0f32.to_le_bytes());
        body[72..76].copy_from_slice(&(-4.0f32).to_le_bytes());
        body[84..88].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            reply(&body, "Example", &pending, (7, 0), 64.0).unwrap(),
            ZoneReply::Rewind(Position {
                x: -4.0,
                y: 12.0,
                z: 0.0,
                heading: 64.0
            })
        );
        assert!(reply(&body, "Other", &pending, (7, 0), 64.0).is_err());
        assert!(reply(&body, "Example", &pending, (8, 0), 64.0).is_err());
        let bind = ZoneOffer {
            zone_id: 0,
            to_bind: true,
            ..pending.clone()
        };
        assert_eq!(
            reply(&body, "Example", &bind, (7, 0), 64.0).unwrap(),
            ZoneReply::Approved
        );
        body[72..76].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(reply(&body, "Example", &pending, (7, 0), 64.0).is_err());
    }

    #[test]
    fn a_same_zone_evacuation_zones_back_in_through_world() {
        // EQEmu offers an evacuation within zone 7 as a transfer to stand-in
        // zone 1, then answers success for zone 7 without coordinates.
        let mut offer = [0; 24];
        offer[..2].copy_from_slice(&1u16.to_le_bytes());
        let pending = super::offer(0x7834, &offer).unwrap();
        assert!(pending.solicited);
        let mut body = pending.response("Example").unwrap().body;
        body[64..66].copy_from_slice(&7u16.to_le_bytes());
        body[68..80].fill(0);
        body[84..88].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            reply(&body, "Example", &pending, (7, 0), 64.0).unwrap(),
            ZoneReply::Approved
        );
        assert!(reply(&body, "Example", &pending, (8, 0), 64.0).is_err());
    }
    #[test]
    fn server_offers_preserve_axes_reason_and_instance() {
        let mut body = [0; 24];
        body[..2].copy_from_slice(&9u16.to_le_bytes());
        body[2..4].copy_from_slice(&3u16.to_le_bytes());
        for (offset, value) in [(4, -2f32), (8, 5.0), (12, 8.0), (16, 64.0)] {
            body[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        body[20..].copy_from_slice(&42u32.to_le_bytes());
        let request = offer(0x7834, &body).unwrap();
        assert_eq!(
            (request.position.x, request.position.y, request.reason),
            (5.0, -2.0, 42)
        );
        let response = request.response("Example").unwrap();
        assert_eq!(response.opcode, CHANGE_OPCODE);
        let mut response = response.body;
        assert_eq!(&response[68..72], &(-2f32).to_le_bytes());
        assert!(!approved(&response, "Example", &request).unwrap());
        response[84..].copy_from_slice(&1i32.to_le_bytes());
        assert!(approved(&response, "Example", &request).unwrap());
        assert!(approved(&response, "Other", &request).is_err());
        response[64] = 10;
        assert!(approved(&response, "Example", &request).is_err());
        body[20..].fill(0);
        let bind = offer(0x385e, &body).unwrap();
        assert_eq!(
            (bind.position.x, bind.position.y, bind.reason),
            (-2.0, 5.0, 10)
        );
        body[4..8].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(offer(0x385e, &body).is_err());
    }
    #[test]
    fn truncation_never_becomes_an_offer_or_death() {
        for len in 0..24 {
            assert!(offer(0x7834, &vec![0; len]).is_err());
        }
        for len in 0..21 {
            assert!(offer(0x385e, &vec![0; len]).is_err());
        }
        for len in 0..32 {
            assert!(death(&vec![0; len]).is_err());
        }
        let mut body = [0; 32];
        body[..4].copy_from_slice(&19u32.to_le_bytes());
        body[20..24].copy_from_slice(&9u32.to_le_bytes());
        assert_eq!(death(&body).unwrap().bind_zone_id, 9);
        assert_eq!(death(&body).unwrap().spawn_id, 19);
    }
}
