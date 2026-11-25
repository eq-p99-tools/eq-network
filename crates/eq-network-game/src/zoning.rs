//! Titanium death, boundary destinations and zone-transfer layouts.
//!
//! Layout references: EQEmu common/patches/titanium_structs.h and
//! `common/eq_packet_structs.h`. Spatial boundary detection belongs to the client;
//! the network layer validates its request against current admission state.
use crate::{command::EncodedCommand, world::Position};
use anyhow::{ensure, Result};
use serde::Serialize;
mod points;
pub use points::{BoundaryRoute, LocalPad, ZoneLineDestination, ZonePoint, ZonePoints};

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
/// `OP_SaveOnZoneReq`: the client asking the zone it is leaving to save the
/// player (`EQEmu` `utils/patches/patch_Titanium.conf`).
pub const SAVE_ON_ZONE_OPCODE: u16 = 0x1540;
/// `OP_Save`: the client asking the zone to save the player on an event
/// (0x4a1350; read R49-22g), with the event's reason at +4.
pub const EVENT_SAVE_OPCODE: u16 = 0x736b;
/// `OP_DeleteSpawn`: the client taking its own spawn out of the zone it is
/// leaving; the server's copy tells the zone a spawn left.
pub const DEPART_OPCODE: u16 = 0x55bc;
/// Titanium's `OP_ZoneUnavail`: the world saying the zone the player is
/// entering is down.
pub const TITANIUM_UNAVAILABLE_OPCODE: u16 = 0x407c;
/// `EQMac`'s `OP_ZoneUnavail`.
pub const EQMAC_UNAVAILABLE_OPCODE: u16 = 0x0580;

/// A received repositioning within the current zone admission. The server
/// can issue this for a pad, spell, or scripted movement.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct SameZoneRelocation {
    /// Admission that supplied this server instruction.
    pub session_id: u64,
    /// The admitted player's spawn ID.
    pub spawn_id: u16,
    /// Absolute landing from the received move.
    pub position: Position,
}

/// A local WLD pad origin assignment after submitting its entry. The ordinary
/// movement stream will carry the landing. This event reports no server reply.
/// The host must finish its first physics step with
/// [`crate::command::GameCommand::FinishLocalPad`]. See that command's retry
/// and disconnect/re-entry contract; no deadline sends the raw landing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct LocalPadCrossing {
    /// Current zone admission.
    pub session_id: u64,
    /// The admitted player's spawn ID.
    pub spawn_id: u16,
    /// Local one-shot completion identity, scoped to this admission.
    pub revision: u64,
    /// Position submitted on entry.
    pub entry: Position,
    /// Local position after crossing.
    pub landing: Position,
}

/// The host's single arrival-floor observation and physical vertical speed.
/// This is local collision state, never a server-confirmed placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PadArrivalStep {
    /// Floor cached by the origin+1, radius0.1*S pad probe, or a real miss.
    pub floor: Option<f32>,
    /// Body origin-to-soles offset S used by that probe.
    pub stand_height: f32,
    /// Actual upward-positive speed before the first-step floor rule,
    /// in world units per second.
    pub vertical_speed: f32,
}

/// The movement owner accepted this pad's first physics step locally.
/// It sends no packet itself; the ordinary update gate owns the next send.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct LocalPadCompletion {
    /// The admission/entry/landing identity that was completed.
    pub crossing: LocalPadCrossing,
    /// Actual origin after the first-step floor rule.
    pub position: Position,
}

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

/// Titanium's answer taking a transfer: the character's name, the zone and
/// instance, where to, and the offer's reason echoed back.
///
/// # Errors
/// Rejects names that cannot fit the NUL-terminated Titanium name field, and
/// non-finite coordinates.
pub fn titanium_answer(
    character: &str,
    (zone_id, instance_id): (u16, u16),
    position: Position,
    reason: u32,
) -> Result<EncodedCommand> {
    ensure!(
        !character.is_empty() && character.len() < 64 && !character.contains('\0'),
        "invalid character name"
    );
    ensure!(finite(position), "invalid zone coordinates");
    let mut body = vec![0; 88];
    body[..character.len()].copy_from_slice(character.as_bytes());
    body[64..66].copy_from_slice(&zone_id.to_le_bytes());
    body[66..68].copy_from_slice(&instance_id.to_le_bytes());
    for (offset, value) in [(68, position.y), (72, position.x), (76, position.z)] {
        body[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    body[80..84].copy_from_slice(&reason.to_le_bytes());
    Ok(EncodedCommand {
        opcode: CHANGE_OPCODE,
        body,
    })
}

/// What the save before leaving a zone holds, as far as this client knows
/// it for certain. The default, with no place, is the save sent when the
/// session has none to give.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Save {
    /// The place and heading the profile holds as the player leaves: aboard
    /// a boat transfer the session still finds the boat for, the place in
    /// the boat's frame; else where the accepted zone request goes.
    pub position: Option<Position>,
    /// The raw name of the boat the player is aboard.
    pub vehicle: Option<String>,
}

/// Where the save keeps its fields (0x4976bf; R36b §1).
const SAVE_POSITION: usize = 0x05;
const SAVE_HEADING: usize = 0x11;
const SAVE_VEHICLE: usize = 0x6d;
/// How many bytes from +0x05 the save's checksum covers.
const SAVE_CHECKED: usize = 0xb8;
/// The byte 0x4a1350 stores its event's reason in; 0x4976bf leaves it 0
/// (read R49-22g §5).
const SAVE_REASON: usize = 0x04;

/// The event reasons 0x4a1350 stores at +4, for the events this session
/// saves on (read R49-22g §1; checklist 6.615).
pub mod save_reason {
    /// `/roleplay` (0x46a3fa).
    pub const ROLEPLAY: u8 = 0x0a;
    /// `/anonymous` (0x4726f1).
    pub const ANONYMOUS: u8 = 0x0b;
    /// The merchant window's close (0x537149); which window it is, is
    /// inferred.
    pub const MERCHANT_CLOSED: u8 = 0x14;
    /// The server's `OP_LootComplete` (0xa94) or `OP_EndLootRequest`
    /// (0x2316), whose handler saves (0x52b05d).
    pub const LOOT_ENDED: u8 = 0x15;
    /// A bind spell (SPA 25) landing on the player where the zone lets them
    /// bind (0x415354; reads R49-22i, R49-22j; see [`crate::binding`]).
    pub const BIND_LANDED: u8 = 0x0e;
}

/// Titanium's checksum 0x4b6c04 (read R36c): the reflected CRC-32 table
/// (0xEDB88320) from a seed of all ones, with no final inversion, which is
/// the bitwise NOT of the ordinary CRC-32.
#[must_use]
pub fn save_checksum(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    crc
}

/// Titanium's request that the zone save the player before they leave it
/// (`OP_SaveOnZoneReq`, 0xc0 bytes), as the program builds every one
/// (0x4976bf; reads R36b §1, R49-36b) and as 11 official-client saves in
/// Adam's P99 and local `EQEmu` recordings carry it:
///
/// - the place at +0x05, +0x09 and +0x0d, unaligned, in the zone request's
///   axis order (y, x, z; read R36c). That it is the accepted request's
///   destination is capture-derived: it equals the request's in all 10
///   recorded saves whose request decoded;
/// - the heading at +0x11 as a 32-bit integer, cut toward zero
///   (capture-derived: a heading of 479.5 saved 479, and a reason 6
///   transfer saved 0xffffffff, which a heading of -1 gives here);
/// - the vehicle name at +0x6d;
/// - the checksum at +0 over the 0xb8 bytes from +0x05 ([`save_checksum`]),
///   always, as in every recorded save.
///
/// Aboard, the place and heading are in the boat's frame. The 22 values at
/// +0x15 and the four at +0xad to +0xb9 aren't identified; the recordings'
/// bytes there change from save to save like leftover memory, and this
/// client leaves them zero (inferred). `EQEmu` reads nothing in the body
/// (`Handle_OP_SaveOnZoneReq` calls `Handle_OP_Save`); what Project 1999
/// reads is unrecorded (6.145).
#[must_use]
pub fn titanium_save_on_zone(save: &Save) -> EncodedCommand {
    titanium_save(save, SAVE_ON_ZONE_OPCODE, 0)
}

/// Titanium's event save (`OP_Save`, 0xc0 bytes), as 0x4a1350 builds it
/// (read R49-22g §5): the same layout as [`titanium_save_on_zone`]'s, with
/// the event's `reason` ([`save_reason`]) at +4. The place is the player's
/// own at the moment of the save, which 0x4a1350 copies from the spawn
/// first (R49-22h §4), in the same axis order.
#[must_use]
pub fn titanium_event_save(save: &Save, reason: u8) -> EncodedCommand {
    titanium_save(save, EVENT_SAVE_OPCODE, reason)
}

/// Both saves' body: one encoder (6.615 P4).
fn titanium_save(save: &Save, opcode: u16, reason: u8) -> EncodedCommand {
    let mut body = vec![0; 192];
    body[SAVE_REASON] = reason;
    if let Some(position) = save.position {
        // The zone request's order: y, then x, then z.
        for (index, value) in [position.y, position.x, position.z].into_iter().enumerate() {
            let at = SAVE_POSITION + index * 4;
            body[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        body[SAVE_HEADING..SAVE_HEADING + 4]
            .copy_from_slice(&whole(position.heading).to_le_bytes());
    }
    if let Some(name) = save
        .vehicle
        .as_deref()
        .filter(|name| name.len() < crate::boats::riders::NAME_LENGTH)
    {
        body[SAVE_VEHICLE..SAVE_VEHICLE + name.len()].copy_from_slice(name.as_bytes());
    }
    let checksum = save_checksum(&body[SAVE_POSITION..SAVE_POSITION + SAVE_CHECKED]);
    body[..4].copy_from_slice(&checksum.to_le_bytes());
    EncodedCommand { opcode, body }
}

/// The save's heading: the whole number cut toward zero, as the recorded
/// saves hold it; a heading too large for 32 bits, or not a number, takes
/// the nearest that fits, or 0.
fn whole(heading: f32) -> i32 {
    // Rust's cast cuts toward zero and saturates, NaN giving 0.
    #[allow(clippy::cast_possible_truncation)]
    let whole = heading as i32;
    whole
}

/// Titanium's departure: the player's own spawn, as a 32-bit ID.
#[must_use]
pub fn titanium_depart(spawn_id: u16) -> EncodedCommand {
    EncodedCommand {
        opcode: DEPART_OPCODE,
        body: u32::from(spawn_id).to_le_bytes().to_vec(),
    }
}

/// Where the player returns once they die, from their profile.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct BindPoint {
    /// The bind point's zone.
    pub zone_id: u16,
    /// Where in it.
    pub position: Position,
}

/// The reason a move home after death carries
/// (`ZC_RepopToHomeAtDeath`, TAKP `common/eq_constants.h`), which
/// Titanium's server offer gives too.
pub const HOME_AT_DEATH: u32 = 10;

impl BindPoint {
    /// The move home to this bind point, as the client asks for it.
    #[must_use]
    pub const fn offer(&self) -> ZoneOffer {
        ZoneOffer {
            zone_id: self.zone_id,
            instance_id: 0,
            position: self.position,
            reason: HOME_AT_DEATH,
            to_bind: true,
            solicited: true,
        }
    }
}

/// The zone's answer to a transfer request, as a client generation reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneAnswer {
    /// The character it names.
    pub character: String,
    /// The zone it answers for.
    pub zone_id: u16,
    /// The zone's instance; zero where the generation has none.
    pub instance_id: u16,
    /// Where in the zone, as x, y and z: Titanium's answer carries it, and
    /// `EQMac`'s does not.
    pub position: Option<(f32, f32, f32)>,
    /// One for a success, else the server's own code.
    pub success: i32,
}

impl ZoneAnswer {
    /// Titanium's 88-byte answer: the name, the zone and instance, where in
    /// it, and the outcome.
    ///
    /// # Errors
    /// Rejects another length and an unterminated name.
    pub fn titanium(body: &[u8]) -> Result<Self> {
        ensure!(body.len() == 88, "invalid zone response length");
        Ok(Self {
            character: name(&body[..64])?,
            zone_id: half(body, 64),
            instance_id: half(body, 66),
            position: Some((float(body, 72), float(body, 68), float(body, 76))),
            success: i32::from_le_bytes(body[84..88].try_into()?),
        })
    }
}

/// A NUL-terminated name in a fixed field.
///
/// # Errors
/// Rejects a name without its NUL.
pub(crate) fn name(field: &[u8]) -> Result<String> {
    let end = field
        .iter()
        .position(|v| *v == 0)
        .ok_or_else(|| anyhow::anyhow!("unterminated zone response name"))?;
    Ok(String::from_utf8_lossy(&field[..end]).into_owned())
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
        titanium_answer(
            character,
            (self.zone_id, self.instance_id),
            self.position,
            self.reason,
        )
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
    /// The corpse's name, which the session gives it from the spawn that
    /// died, by the client generation's rule ([`crate::world::corpse_name`]);
    /// None where the rule is not checked or the spawn was not seen.
    pub corpse_name: Option<String>,
}

/// Decode a Titanium death notification without interpreting NPC death as player death.
/// `RoF2` reads the same 32 bytes (6.519): stock `EQEmu` sends neither generation an encoded
/// one.
///
/// The bind zone is read at word 20, where both `common/patches/titanium_structs.h` and
/// `rof2_structs.h` declare it, with the attack skill at 12. Stock `EQEmu` has no encoder for
/// either generation and sends its own `Death_Struct` (`common/eq_packet_structs.h`), which has
/// the two the other way round. So on stock `EQEmu` this bind zone carries the killing blow's
/// damage type, and word 12 the bind zone (`Client::Death` in `zone/attack.cpp` fills
/// `bindzoneid` from the first bind). That is the server's quirk, the one an original client
/// meets too. Which word each program reads is a decompile question.
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
        corpse_name: None,
    })
}

/// Reads the world's word that the zone the player is entering is down
/// (`ZoneUnavail_Struct`, 24 bytes on every server): the zone's short name,
/// NUL-terminated in the first 16 bytes, and none when the world could not
/// name it.
///
/// # Errors
/// Rejects any length but 24, and a name that is not plain ASCII.
pub fn unavailable(body: &[u8]) -> Result<Option<String>> {
    ensure!(body.len() == 24, "invalid zone-unavailable length");
    let name = crate::world::until_nul(&body[..16]);
    ensure!(
        name.iter().all(u8::is_ascii_graphic),
        "invalid unavailable zone name"
    );
    Ok((!name.is_empty()).then(|| String::from_utf8_lossy(name).into_owned()))
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
        reason: if to_bind {
            HOME_AT_DEATH
        } else {
            word(body, 20)
        },
        to_bind,
        solicited: true,
    })
}

/// Verify a zone response belongs to the pending request and active character.
///
/// # Errors
/// Rejects another character, or a mismatched destination.
pub fn approved(answer: &ZoneAnswer, character: &str, pending: &ZoneOffer) -> Result<bool> {
    ensure!(
        answer.character.eq_ignore_ascii_case(character),
        "zone response is for another character"
    );
    if answer.success != 1 {
        return Ok(false);
    }
    ensure!(
        (pending.zone_id == 0 || answer.zone_id == pending.zone_id)
            && answer.instance_id == pending.instance_id,
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
/// of the requested one means two things in `EQEmu` (`zone/zoning.cpp`): for a zone
/// line the client crossed, the server cancelled and supplies rewind coordinates
/// (`SendZoneCancel`); for a transfer the server asked for, such as an evacuation
/// or succor within the zone (offered with a stand-in zone), the server moved the
/// character and expects it to zone back in through world (`DoZoneSuccess`).
/// An answer without coordinates, as `EQMac`'s, cancels without a rewind.
///
/// # Errors
/// Rejects unexpected destinations and invalid rewind coordinates.
pub fn reply(
    answer: &ZoneAnswer,
    character: &str,
    pending: &ZoneOffer,
    current_zone: (u16, u16),
    heading: f32,
) -> Result<ZoneReply> {
    let returned = (answer.zone_id, answer.instance_id);
    if pending.zone_id != 0
        && returned == current_zone
        && returned != (pending.zone_id, pending.instance_id)
        && answer.success == 1
    {
        let current = ZoneOffer {
            zone_id: current_zone.0,
            instance_id: current_zone.1,
            ..pending.clone()
        };
        approved(answer, character, &current)?;
        if pending.solicited {
            return Ok(ZoneReply::Approved);
        }
        let Some((x, y, z)) = answer.position else {
            return Ok(ZoneReply::Denied(ZoneRejection::Cancelled));
        };
        let position = Position { x, y, z, heading };
        ensure!(finite(position), "invalid zone cancellation position");
        return Ok(ZoneReply::Rewind(position));
    }
    Ok(if approved(answer, character, pending)? {
        ZoneReply::Approved
    } else {
        ZoneReply::Denied(ZoneRejection::Server(answer.success))
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
    fn an_unavailable_zone_is_named_when_the_world_can() {
        let mut body = [0_u8; 24];
        body[..5].copy_from_slice(b"misty");
        assert_eq!(unavailable(&body).unwrap().as_deref(), Some("misty"));
        // The name may fill its field without a terminator.
        body[..16].copy_from_slice(b"averylongzonenam");
        assert_eq!(
            unavailable(&body).unwrap().as_deref(),
            Some("averylongzonenam")
        );
        assert_eq!(unavailable(&[0; 24]).unwrap(), None);
        body[..6].copy_from_slice(b"mi\x1bsty");
        assert!(unavailable(&body).is_err());
        assert!(unavailable(&[0; 16]).is_err());
        assert!(unavailable(&[0; 25]).is_err());
    }

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
        let read = |body: &[u8]| ZoneAnswer::titanium(body).unwrap();
        for code in [0_i32, -1, -2, -3, -6, -7, -12345, 42, i32::MIN] {
            body[84..88].copy_from_slice(&code.to_le_bytes());
            let rejection = ZoneRejection::Server(code);
            assert_eq!(
                reply(&read(&body), "Example", &pending, (7, 0), 0.0).unwrap(),
                ZoneReply::Denied(rejection)
            );
            assert!(rejection
                .to_string()
                .contains(&format!("server code {code}")));
            assert!(reply(&read(&body), "Other", &pending, (7, 0), 0.0).is_err());
        }
        body[84..88].copy_from_slice(&1_i32.to_le_bytes());
        assert_eq!(
            reply(&read(&body), "Example", &pending, (7, 0), 0.0).unwrap(),
            ZoneReply::Approved
        );
        assert!(ZoneAnswer::titanium(&body[..87]).is_err());
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
        let answer = ZoneAnswer::titanium(&body).unwrap();
        assert_eq!(
            reply(&answer, "Example", &pending, (7, 0), 64.0).unwrap(),
            ZoneReply::Rewind(Position {
                x: -4.0,
                y: 12.0,
                z: 0.0,
                heading: 64.0
            })
        );
        assert!(reply(&answer, "Other", &pending, (7, 0), 64.0).is_err());
        assert!(reply(&answer, "Example", &pending, (8, 0), 64.0).is_err());
        let bind = ZoneOffer {
            zone_id: 0,
            to_bind: true,
            ..pending.clone()
        };
        assert_eq!(
            reply(&answer, "Example", &bind, (7, 0), 64.0).unwrap(),
            ZoneReply::Approved
        );
        // An answer without coordinates cancels where the player stands.
        let unplaced = ZoneAnswer {
            position: None,
            ..answer.clone()
        };
        assert_eq!(
            reply(&unplaced, "Example", &pending, (7, 0), 64.0).unwrap(),
            ZoneReply::Denied(ZoneRejection::Cancelled)
        );
        body[72..76].copy_from_slice(&f32::NAN.to_le_bytes());
        let answer = ZoneAnswer::titanium(&body).unwrap();
        assert!(reply(&answer, "Example", &pending, (7, 0), 64.0).is_err());
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
        let answer = ZoneAnswer::titanium(&body).unwrap();
        assert_eq!(
            reply(&answer, "Example", &pending, (7, 0), 64.0).unwrap(),
            ZoneReply::Approved
        );
        assert!(reply(&answer, "Example", &pending, (8, 0), 64.0).is_err());
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
        let read = |body: &[u8]| ZoneAnswer::titanium(body).unwrap();
        assert!(!approved(&read(&response), "Example", &request).unwrap());
        response[84..].copy_from_slice(&1i32.to_le_bytes());
        assert!(approved(&read(&response), "Example", &request).unwrap());
        assert!(approved(&read(&response), "Other", &request).is_err());
        response[64] = 10;
        assert!(approved(&read(&response), "Example", &request).is_err());
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
    fn titanium_departs_with_a_save_and_its_own_spawn() {
        // A save with no place is still checksummed, as the program always
        // checksums it.
        let empty = titanium_save_on_zone(&Save::default());
        assert_eq!((empty.opcode, empty.body.len()), (0x1540, 192));
        assert_eq!(&empty.body[..4], &save_checksum(&[0; 0xb8]).to_le_bytes());
        assert!(empty.body[4..].iter().all(|byte| *byte == 0));
        // An ordinary save carries the place and heading, with no vehicle.
        let ordinary = titanium_save_on_zone(&Save {
            position: Some(Position {
                x: 10.0,
                y: 20.0,
                z: -3.5,
                heading: 127.5,
            }),
            vehicle: None,
        });
        assert_eq!(&ordinary.body[5..9], &20.0f32.to_le_bytes());
        assert_eq!(&ordinary.body[9..13], &10.0f32.to_le_bytes());
        assert_eq!(&ordinary.body[13..17], &(-3.5f32).to_le_bytes());
        assert_eq!(&ordinary.body[17..21], &127u32.to_le_bytes());
        assert_eq!(
            &ordinary.body[..4],
            &save_checksum(&ordinary.body[5..5 + 0xb8]).to_le_bytes()
        );
        assert!(ordinary.body[21..].iter().all(|byte| *byte == 0));
        let aboard = titanium_save_on_zone(&Save {
            position: Some(Position {
                x: 1.5,
                y: -2.0,
                z: 20.0,
                heading: 128.0,
            }),
            vehicle: Some("Sabrina00".into()),
        });
        assert_eq!(aboard.body.len(), 0xc0);
        assert_eq!(
            &aboard.body[..4],
            &save_checksum(&aboard.body[5..5 + 0xb8]).to_le_bytes()
        );
        assert_eq!(aboard.body[4], 0);
        // The save writes its position as the zone request does.
        let request = titanium_answer(
            "Tester",
            (4, 0),
            Position {
                x: 1.5,
                y: -2.0,
                z: 20.0,
                heading: 128.0,
            },
            2,
        )
        .unwrap();
        assert_eq!(&aboard.body[5..17], &request.body[68..80]);
        assert_eq!(&aboard.body[5..9], &(-2.0f32).to_le_bytes());
        assert_eq!(&aboard.body[9..13], &1.5f32.to_le_bytes());
        assert_eq!(&aboard.body[13..17], &20.0f32.to_le_bytes());
        // The heading, cut to a whole number, then the unidentified values
        // stay zero.
        assert_eq!(&aboard.body[17..21], &128u32.to_le_bytes());
        assert!(aboard.body[21..0x6d].iter().all(|byte| *byte == 0));
        assert_eq!(&aboard.body[0x6d..0x6d + 10], b"Sabrina00\0");
        assert!(aboard.body[0x6d + 9..].iter().all(|byte| *byte == 0));
        assert_eq!(
            titanium_depart(7),
            EncodedCommand {
                opcode: 0x55bc,
                body: vec![7, 0, 0, 0],
            }
        );
    }

    #[test]
    fn the_saves_heading_is_cut_toward_zero() {
        // As recorded: 479.5 saved 479, and -1 saved 0xffffffff.
        assert_eq!(
            [479.5, 10.9, 0.0, -1.0, -1.5, f32::NAN, 1e10].map(whole),
            [479, 10, 0, -1, -1, 0, i32::MAX]
        );
        let save = titanium_save_on_zone(&Save {
            position: Some(Position {
                heading: -1.0,
                ..Position::default()
            }),
            vehicle: None,
        });
        assert_eq!(&save.body[17..21], &[0xff; 4]);
    }

    #[test]
    fn an_event_save_is_the_zone_saves_body_with_its_reason() {
        let save = Save {
            position: Some(Position {
                x: 10.0,
                y: -20.5,
                z: 3.25,
                heading: 479.5,
            }),
            vehicle: Some("Boat".into()),
        };
        let zone = titanium_save_on_zone(&save);
        for (reason, value) in [
            (save_reason::ROLEPLAY, 0x0a),
            (save_reason::ANONYMOUS, 0x0b),
            (save_reason::MERCHANT_CLOSED, 0x14),
            (save_reason::LOOT_ENDED, 0x15),
        ] {
            let event = titanium_event_save(&save, reason);
            assert_eq!(event.opcode, 0x736b);
            assert_eq!(event.body.len(), 0xc0);
            assert_eq!(event.body[4], value);
            // The checksum covers +5 on, so the reason leaves it as is.
            let mut unreasoned = event.body.clone();
            unreasoned[4] = 0;
            assert_eq!(unreasoned, zone.body);
        }
        assert_eq!(zone.body[4], 0);
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

    #[test]
    fn the_save_checksum_is_the_inverse_of_the_ordinary_crc32() {
        // The ordinary CRC-32 check value of "123456789" is 0xcbf43926.
        assert_eq!(save_checksum(b"123456789"), !0xcbf4_3926);
        assert_eq!(save_checksum(&[]), u32::MAX);
    }
}
