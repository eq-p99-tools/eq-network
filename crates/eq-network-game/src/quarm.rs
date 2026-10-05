//! TAKP/EQMac world state, normalized to the same units as Titanium.
//!
//! Layout references: SecretsOTheP/EQMacEmu `common/patches/mac_structs.h`,
//! `mac.cpp`, `common/eq_packet_structs.h`, and `common/packet_functions.cpp`.
//! Decoders retain only presentation fields, never the full character profile.

use crate::{
    command::EncodedCommand,
    spells::SpellUpdate,
    world::{
        BaseAttributes, ItemHitPoints, PlayerState, Position, SpawnKind, SpawnState, WorldEvent,
    },
    zoning::ZoneOffer,
};
use anyhow::{ensure, Context, Result};
use zeroize::Zeroizing;

pub(crate) const PROFILE_SIZE: usize = 8460;
const SPAWN_SIZE: usize = 224;

/// The client's data rate, the first thing it sends a zone.
pub const ZONE_DATA_RATE: u16 = 0xe841;
/// The client's zone entry, and the zone's answer: the player's own spawn.
pub const ZONE_ENTRY: u16 = 0x2840;
/// The player's profile.
pub const ZONE_PLAYER_PROFILE: u16 = 0x3640;
/// The zone's weather, after which the client asks for the zone.
pub const ZONE_WEATHER: u16 = 0x3641;
/// The client asking for the zone's description.
pub const ZONE_REQUEST_NEW: u16 = 0x5d40;
/// The zone's description.
pub const ZONE_NEW: u16 = 0x5b40;
/// The client asking for the zone's spawns.
pub const ZONE_REQUEST_SPAWNS: u16 = 0x0a40;
/// The zone's experience report, which the client echoes.
pub const ZONE_EXPERIENCE_READY: u16 = 0xd840;
/// The zone saying the player's avatar is ready.
pub const ZONE_AVATAR_READY: u16 = 0x6f40;
/// The client's chat and combat filters.
pub const ZONE_SERVER_FILTER: u16 = 0xff41;
/// A position update; the client's first completes the zone entry.
pub const ZONE_CLIENT_UPDATE: u16 = 0xf340;
/// A spawn appearance: the own spawn's ID, and the DLL version check.
pub const ZONE_SPAWN_APPEARANCE: u16 = 0xf540;
/// The client's logout once its camp timer completes, which the server
/// also sends to log the character out.
pub const ZONE_LOGOUT: u16 = 0x5041;
/// The client starting to camp (`OP_Camp`).
pub const ZONE_CAMP: u16 = 0x0742;
/// The server's answer to a logout (`OP_LogoutReply`), which ends the zone
/// connection.
pub const ZONE_LOGOUT_REPLY: u16 = 0x5941;
/// `OP_ZoneChange`: the client asking whether it may enter a zone, and the
/// server's answer (TAKP `utils/patches/patch_Mac.conf` lists 0x40a3, its
/// bytes swapped).
pub const ZONE_CHANGE: u16 = 0xa340;
/// `OP_SendZonepoints`: the zone's numbered destinations (0x40b4 swapped).
pub const ZONE_POINTS: u16 = 0xb440;
/// `OP_SaveOnZoneReq`: the client asking the zone it is leaving to save the
/// player (0x4155 swapped).
pub const ZONE_SAVE_ON_ZONE: u16 = 0x5541;
/// `OP_DeleteSpawn`: the client taking its own spawn out of the zone it is
/// leaving (0x4029 swapped).
pub const ZONE_DEPART: u16 = 0x2940;
/// `OP_Death`: someone in the zone died (0x404a swapped).
pub const ZONE_DEATH: u16 = 0x4a40;
/// The server asking the client to change zones.
pub const ZONE_CHANGE_REQUEST: u16 = 0x4d41;

// akplus-dll af2bd327, eqgame.cpp: DLL_VERSION and DLL_VERSION_MESSAGE_ID.
// This announcement is independent of the optional gameplay feature handshakes.
const DLL_VERSION: u16 = 7;
const DLL_MESSAGE_TYPE: u16 = 256;
const DLL_VERSION_FEATURE: u16 = 4;

/// The client's DLL version announcement, or a reply with the response bit
/// set; custom DLL messages use spawn ID zero.
#[must_use]
pub fn dll_version_message(response: bool) -> [u8; 8] {
    let mut body = [0; 8];
    body[2..4].copy_from_slice(&DLL_MESSAGE_TYPE.to_le_bytes());
    let parameter = (u32::from(response) << 31)
        | (u32::from(DLL_VERSION_FEATURE) << 16)
        | u32::from(DLL_VERSION);
    body[4..].copy_from_slice(&parameter.to_le_bytes());
    body
}

/// The reply to a well-formed DLL version request, during admission or
/// normal play; none for anything else.
#[must_use]
pub fn dll_version_reply(body: &[u8]) -> Option<[u8; 8]> {
    let body: &[u8; 8] = body.try_into().ok()?;
    let spawn_id = u16::from_le_bytes([body[0], body[1]]);
    let appearance = u16::from_le_bytes([body[2], body[3]]);
    let parameter = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
    // Comparing the full high word also excludes responses (bit 31), preventing loops.
    (spawn_id == 0
        && appearance == DLL_MESSAGE_TYPE
        && parameter >> 16 == u32::from(DLL_VERSION_FEATURE))
    .then(|| dll_version_message(true))
}

/// What the `EQMac` client answers by itself whenever it arrives, during
/// admission or normal play: Quarm's DLL version check.
#[must_use]
pub fn answer(opcode: u16, body: &[u8]) -> Option<EncodedCommand> {
    if opcode != ZONE_SPAWN_APPEARANCE {
        return None;
    }
    dll_version_reply(body).map(|reply| EncodedCommand {
        opcode: ZONE_SPAWN_APPEARANCE,
        body: reply.to_vec(),
    })
}

/// The server asking the client to move (`RequestClientZoneChange`): the
/// zone as a 32-bit ID where Titanium's request has a zone and an instance,
/// then where to and the reason the client echoes. The client keeps its
/// own coordinates for 999999.
///
/// # Errors
/// Rejects malformed requests and zone IDs beyond 16 bits.
pub fn zone_request(body: &[u8]) -> Result<ZoneOffer> {
    ensure!(body.len() == 24, "invalid EQMac zone request length");
    Ok(ZoneOffer {
        zone_id: u16::try_from(word(body, 0)).context("EQMac zone ID out of range")?,
        instance_id: 0,
        position: Position {
            x: float(body, 8)?,
            y: float(body, 4)?,
            z: float(body, 12)?,
            heading: float(body, 16)?,
        },
        reason: word(body, 20),
        to_bind: false,
        solicited: true,
    })
}

/// A death (`Death_Struct`, 20 bytes): who died, their killer and their
/// corpse as 16-bit IDs, then the level, spell, skill, damage and whether a
/// player died. `EQMac`'s death names no bind zone: the client knows its own
/// from the profile ([`bind_point`]).
///
/// # Errors
/// Rejects another length and a death of no one.
pub fn death(body: &[u8]) -> Result<crate::zoning::Death> {
    ensure!(body.len() == 20, "invalid EQMac death length");
    Ok(crate::zoning::Death {
        spawn_id: u32::from(valid_id(u32::from(short(body, 0)))?),
        killer_id: u32::from(short(body, 2)),
        corpse_id: u32::from(short(body, 4)),
        bind_zone_id: 0,
        corpse_name: None,
    })
}

/// The player's first bind point, from their profile: the zone at 3784 and
/// then y, x, z and heading at 3804, 3824, 3844 and 3864, each an array of
/// five (TAKP `common/patches/mac_structs.h` `PlayerProfile_Struct`). The
/// bind heading is on the 512 scale already, as TAKP writes it there
/// (`common/patches/mac.cpp` `ENCODE(OP_PlayerProfile)`).
///
/// # Errors
/// Rejects a profile that does not unpack, a zone beyond 16 bits, and a
/// position that is not finite.
pub fn bind_point(body: &[u8]) -> Result<crate::zoning::BindPoint> {
    decoded_bind(&unpack(body, true)?)
}

/// The first bind point in an unpacked profile.
fn decoded_bind(data: &[u8]) -> Result<crate::zoning::BindPoint> {
    ensure!(
        data.len() == PROFILE_SIZE,
        "unexpected EQMac profile layout"
    );
    Ok(crate::zoning::BindPoint {
        zone_id: u16::try_from(word(data, 3784)).context("EQMac bind zone out of range")?,
        position: Position {
            x: float(data, 3824)?,
            y: float(data, 3804)?,
            z: float(data, 3844)?,
            heading: float(data, 3864)?,
        },
    })
}

/// The client asking whether it may enter a zone (`ZoneChange_Struct`, 76
/// bytes: the name, the zone as 32 bits, the reason the server's request
/// gave, and a zero outcome). `EQMac`'s request carries no position: TAKP
/// finds the destination itself (`zone/zoning.cpp` `Handle_OP_ZoneChange`).
///
/// # Errors
/// Rejects a name its field cannot hold.
pub fn zone_change(character: &str, zone_id: u16, reason: u32) -> Result<EncodedCommand> {
    ensure!(
        !character.is_empty() && character.len() < 64 && !character.contains('\0'),
        "invalid character name"
    );
    let mut body = vec![0; 76];
    body[..character.len()].copy_from_slice(character.as_bytes());
    body[64..68].copy_from_slice(&u32::from(zone_id).to_le_bytes());
    body[68..72].copy_from_slice(&reason.to_le_bytes());
    Ok(EncodedCommand {
        opcode: ZONE_CHANGE,
        body,
    })
}

/// The server's answer to the client's request (`ZoneChange_Struct`): the
/// name, the zone, and the outcome, one for a success; no instance and no
/// position.
///
/// # Errors
/// Rejects another length, an unterminated name and a zone beyond 16 bits.
pub fn zone_answer(body: &[u8]) -> Result<crate::zoning::ZoneAnswer> {
    ensure!(body.len() == 76, "invalid EQMac zone answer length");
    Ok(crate::zoning::ZoneAnswer {
        character: crate::zoning::name(&body[..64])?,
        zone_id: u16::try_from(word(body, 64)).context("EQMac zone ID out of range")?,
        instance_id: 0,
        position: None,
        success: i32::from_le_bytes(body[72..76].try_into()?),
    })
}

/// The client asking the zone it is leaving to save the player. TAKP reads
/// nothing in it (`zone/client_packet.cpp` `Handle_OP_Save`, whose comment
/// says the payload is 192 bytes), so its 192 bytes stay zero (inferred:
/// the official `EQMac` client's own body is unrecorded).
#[must_use]
pub fn save_on_zone() -> EncodedCommand {
    EncodedCommand {
        opcode: ZONE_SAVE_ON_ZONE,
        body: vec![0; 192],
    }
}

/// The client taking its own spawn out of the zone it is leaving, which
/// TAKP waits for once the world approves the move (`zone/zoning.cpp`
/// `HandleZoneTransferResponse`, then `Handle_OP_DeleteSpawn`): the spawn
/// as 16 bits (`DeleteSpawn_Struct`).
#[must_use]
pub fn depart(spawn_id: u16) -> EncodedCommand {
    EncodedCommand {
        opcode: ZONE_DEPART,
        body: spawn_id.to_le_bytes().to_vec(),
    }
}

/// The skill a bleed-out report names: hand to hand, which TAKP itself
/// names for a death from a tick (`zone/attack.cpp`
/// `GenerateDeathPackets`). Inferred: the official client's is unrecorded.
const BLED_OUT_SKILL: u8 = 28;

/// The player's own report that they bled out (`Death_Struct`, 20 bytes,
/// laid out as [`death`] reads it). TAKP leaves to the client the deaths it
/// does not announce to the one who died, and takes this report without
/// checking the player's HP (`zone/client_packet.cpp` `Handle_OP_Death`), so
/// it goes out only for a player whose HP reached the server's threshold.
/// Every field but the spawn is inferred until the official client's
/// bleed-out is recorded: no killer, as `EQMacEmu` notes the official client
/// names none for a bleed-out; no damage; no spell (0xFFFF, TAKP's
/// `SPELL_UNKNOWN`); hand to hand (28), which TAKP itself names for a death
/// from a tick; and no corpse, level or player flag.
///
/// # Errors
/// Rejects a report without the player's spawn.
pub fn bled_out(spawn_id: u16) -> Result<EncodedCommand> {
    ensure!(spawn_id != 0, "a death report needs the player's spawn");
    let mut body = vec![0; 20];
    body[..2].copy_from_slice(&spawn_id.to_le_bytes());
    body[8..10].copy_from_slice(&u16::MAX.to_le_bytes());
    body[10] = BLED_OUT_SKILL;
    Ok(EncodedCommand {
        opcode: ZONE_DEATH,
        body,
    })
}

/// The client starting to camp. TAKP reads nothing in it; the official
/// client's body is unrecorded.
#[must_use]
pub fn camp() -> EncodedCommand {
    EncodedCommand {
        opcode: ZONE_CAMP,
        body: Vec::new(),
    }
}

/// The client's logout once its camp timer completes.
#[must_use]
pub fn logout() -> EncodedCommand {
    EncodedCommand {
        opcode: ZONE_LOGOUT,
        body: Vec::new(),
    }
}

/// The player's stance, as Titanium's: an appearance of the player's own
/// spawn, of type 14 (animation), with the stance's value.
///
/// # Errors
/// Refuses a stance for no spawn.
pub fn posture(spawn_id: u16, posture: crate::command::Posture) -> Result<EncodedCommand> {
    ensure!(spawn_id != 0, "a stance needs the player's own spawn");
    let mut body = spawn_id.to_le_bytes().to_vec();
    body.extend_from_slice(&14u16.to_le_bytes());
    body.extend_from_slice(&posture.appearance().to_le_bytes());
    Ok(EncodedCommand {
        opcode: ZONE_SPAWN_APPEARANCE,
        body,
    })
}

/// The player's position (`OP_ClientUpdate`, TAKP's 15-byte
/// `SpawnPositionUpdate_Struct`): the spawn, its speed, its heading in
/// halves (0 to 255), its turn, where it stands as whole units (z in tenths,
/// cut toward zero as the client cuts them), and its velocity packed as the
/// client packs it: sixteenths, x in 10 bits, z and y in 11.
///
/// # Errors
/// Refuses a sample [`PositionPacket::check`] refuses.
///
/// [`PositionPacket::check`]: crate::movement::PositionPacket::check
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Clamped and masked first.
pub fn client_update(sample: &crate::movement::PositionPacket) -> Result<EncodedCommand> {
    sample.check()?;
    let whole = |value: f32| {
        value
            .trunc()
            .clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
    };
    let small = |value: i16| value.clamp(i16::from(i8::MIN), i16::from(i8::MAX)) as i8;
    let packed = |value: f32, low: f32, high: f32, mask: u32| {
        (((value.clamp(low, high) * 16.0) as i32) as u32) & mask
    };
    let position = sample.position;
    let mut body = Vec::with_capacity(15);
    body.extend_from_slice(&sample.spawn_id.to_le_bytes());
    body.extend_from_slice(&small(sample.animation).to_le_bytes());
    body.push((position.heading.rem_euclid(512.0) / 2.0) as u8);
    body.extend_from_slice(&small(sample.delta_heading).to_le_bytes());
    for value in [position.y, position.x, position.z * 10.0] {
        body.extend_from_slice(&whole(value).to_le_bytes());
    }
    let [x, y, z] = sample.delta;
    let velocity = (packed(x, -32.0, 31.0, 0x3ff) << 22)
        | (packed(z, -64.0, 63.0, 0x7ff) << 11)
        | packed(y, -64.0, 63.0, 0x7ff);
    body.extend_from_slice(&velocity.to_le_bytes());
    Ok(EncodedCommand {
        opcode: ZONE_CLIENT_UPDATE,
        body,
    })
}

/// The client's filters: every chat and combat category on.
#[must_use]
pub fn server_filters() -> [u8; 68] {
    let mut filters = [0; 68];
    for index in 5..=14 {
        filters[index * 4] = 1;
    }
    filters
}

/// Decrypt the full 64-bit words, preserve the tail, then inflate with a strict limit.
fn unpack(body: &[u8], profile: bool) -> Result<Zeroizing<Vec<u8>>> {
    ensure!(
        (8..=65536).contains(&body.len()),
        "invalid EQMac compressed length"
    );
    let mut bytes = Zeroizing::new(body.to_vec());
    let (constant, mut key, first_rotation, second_rotation) = if profile {
        (0x4224_37a9u64, 0x6593_65e7u64, 7, 39)
    } else {
        (0x6593_65e7u64, 0, 14, 29)
    };
    let (words, _) = bytes.as_chunks_mut::<8>();
    for word in words.iter_mut() {
        let plain = u64::from_le_bytes(*word)
            .wrapping_add(key)
            .rotate_right(first_rotation)
            .wrapping_sub(constant)
            .rotate_right(second_rotation);
        key = key.wrapping_add(plain).wrapping_sub(constant);
        *word = plain.to_le_bytes();
    }
    words.swap(0, words.len() / 2);
    let limit = if profile {
        PROFILE_SIZE
    } else {
        SPAWN_SIZE * 4096
    };
    miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&bytes, limit)
        .map(Zeroizing::new)
        .map_err(|_| anyhow::anyhow!("invalid or oversized EQMac compressed payload"))
}

/// Decode only the rendering fields of the encrypted player profile.
///
/// # Errors
/// Rejects malformed compression, unexpected layouts, and a different character.
/// The returned spawn ID is zero until the session receives the `SpawnID` appearance.
pub fn profile(body: &[u8], character: &str) -> Result<PlayerState> {
    let data = unpack(body, true)?;
    decoded_profile(&data, character)
}

/// Projects validated profile fields without retaining the decompressed payload.
fn decoded_profile(data: &[u8], character: &str) -> Result<PlayerState> {
    ensure!(
        data.len() == PROFILE_SIZE,
        "unexpected EQMac profile layout"
    );
    ensure!(
        cstr(&data[6..70]).eq_ignore_ascii_case(character.as_bytes()),
        "profile character mismatch"
    );
    Ok(PlayerState {
        name: String::from_utf8_lossy(cstr(&data[6..70])).into_owned(),
        base_attributes: Some(BaseAttributes {
            strength: signed_attribute(data, 164),
            stamina: signed_attribute(data, 166),
            charisma: signed_attribute(data, 168),
            dexterity: signed_attribute(data, 170),
            intelligence: signed_attribute(data, 172),
            agility: signed_attribute(data, 174),
            wisdom: signed_attribute(data, 176),
        }),
        deity: None,
        class: Some(u32::from(short(data, 144))),
        spawn_id: 0,
        race: u32::from(short(data, 142)),
        gender: u32::from(data[140]),
        level: data[148],
        position: Position {
            x: float(data, 2908)?,
            y: float(data, 2904)?,
            z: float(data, 2912)?,
            heading: (float(data, 2916)? * 2.0).rem_euclid(512.0),
        },
        mana: u32::from(short(data, 158)),
        // EQMac uses fatigue, not Titanium endurance; no equivalent value is supplied.
        endurance: None,
        skills: None,
        practice_points: None,
        // What is left of each gem's reuse timer, in milliseconds
        // (`spellSlotRefresh`).
        spell_refresh_ms: Some(std::array::from_fn(|index| word(data, 4972 + index * 4))),
        memorized_spells: std::array::from_fn(|i| {
            let id = short(data, 2870 + i * 2);
            (id != 0 && id != u16::MAX).then_some(u32::from(id))
        }),
        size: 0.0,
        walk_speed: 0.0,
        run_speed: 0.0,
        hp_percent: None,
        appearance: crate::appearance::Appearance::default(),
        // EQMac /who fields are not decoded yet.
        listing: crate::listing::Listing::default(),
        name_parts: crate::names::NameParts::default(),
    })
}

/// Reads a signed `EQMac` profile attribute after the profile length check.
fn signed_attribute(data: &[u8], offset: usize) -> i32 {
    i32::from(i16::from_le_bytes([data[offset], data[offset + 1]]))
}

/// The player's buffs and spellbook from their profile, as the admission's
/// buff table and book.
///
/// # Errors
/// Rejects malformed compression and unexpected layouts.
pub fn profile_spells(body: &[u8]) -> Result<Vec<WorldEvent>> {
    decoded_spells(&unpack(body, true)?)
}

/// The buffs and spellbook in an unpacked profile.
fn decoded_spells(data: &[u8]) -> Result<Vec<WorldEvent>> {
    Ok(vec![
        WorldEvent::BuffSnapshot(crate::buffs::eqmac_profile(data)?),
        WorldEvent::SpellBook(crate::spells::SpellBook::eqmac_profile(data)?),
    ])
}

/// The player's coins from their profile: those they carry, and those on
/// the cursor and in the bank.
///
/// # Errors
/// Rejects malformed compression, unexpected layouts and negative counts.
pub fn profile_coins(body: &[u8]) -> Result<Vec<WorldEvent>> {
    decoded_coins(&unpack(body, true)?)
}

/// The coins in an unpacked profile.
fn decoded_coins(data: &[u8]) -> Result<Vec<WorldEvent>> {
    let (cursor, bank) = crate::money::eqmac_elsewhere(data)?;
    Ok(vec![
        WorldEvent::Coins(crate::money::eqmac_coins(data)?),
        WorldEvent::CoinsElsewhere {
            cursor,
            bank,
            given: crate::world::Coins::default(),
            offered: crate::world::Coins::default(),
        },
    ])
}

/// The separate, uncompressed own-character zone-entry projection.
#[derive(Clone, Debug)]
pub struct OwnSpawn {
    /// Corrected zone-entry position, which can differ from the saved profile.
    pub position: Position,
    /// Server model dimensions.
    pub size: f32,
    /// Protocol walk speed, not world units per second.
    pub walk_speed: f32,
    /// Protocol run speed, not world units per second.
    pub run_speed: f32,
}

/// Decode the own-character entry without retaining names or unused profile bytes.
///
/// # Errors
/// Rejects wrong lengths, character mismatch, and invalid dimensions or positions.
pub fn own_spawn(body: &[u8], character: &str) -> Result<OwnSpawn> {
    ensure!(body.len() == 356, "unexpected EQMac own-spawn layout");
    ensure!(
        cstr(&body[5..69]).eq_ignore_ascii_case(character.as_bytes()),
        "own-spawn character mismatch"
    );
    Ok(OwnSpawn {
        position: Position {
            x: float(body, 80)?,
            y: float(body, 76)?,
            z: float(body, 84)?,
            heading: (float(body, 88)? * 2.0).rem_euclid(512.0),
        },
        size: nonnegative(body, 244)?,
        walk_speed: nonnegative(body, 260)?,
        run_speed: nonnegative(body, 264)?,
    })
}

/// A spawn's class in the server's numbering, the one Titanium's spawns use,
/// from the Mac client's: TAKP's patch sends a banker (40) as 16, a merchant
/// (41) as 32 and the guildmaster classes (20 to 34) three lower
/// (`common/patches/mac.cpp`, `ENCODE(OP_ZoneSpawns)`); the player classes
/// pass as they are.
const fn server_class(mac: u8) -> u8 {
    match mac {
        16 => 40,
        17..=31 => mac + 3,
        32 => 41,
        other => other,
    }
}

/// Decode a compressed initial or incremental spawn batch.
///
/// # Errors
/// Rejects invalid compression, partial records, zero IDs, and invalid model sizes.
pub fn spawns(body: &[u8]) -> Result<Vec<SpawnState>> {
    decoded_spawns(&unpack(body, false)?)
}

/// The spawns in an unpacked batch (TAKP `common/patches/mac_structs.h`
/// `Spawn_Struct`, 224 bytes each): among them the class at 87, in the Mac
/// client's numbering and read back into the server's ([`server_class`]),
/// and the level at 89.
fn decoded_spawns(data: &[u8]) -> Result<Vec<SpawnState>> {
    ensure!(
        !data.is_empty() && data.len().is_multiple_of(SPAWN_SIZE),
        "partial EQMac spawn batch"
    );
    data.as_chunks::<SPAWN_SIZE>()
        .0
        .iter()
        .map(|record| {
            let spawn_id = valid_id(u32::from(short(record, 76)))?;
            Ok(SpawnState {
                class: Some(server_class(record[87])),
                spawn_id,
                name: String::from_utf8_lossy(cstr(&record[127..191])).into_owned(),
                kind: match record[86] {
                    0 | 10 => SpawnKind::Player,
                    1 => SpawnKind::Npc,
                    2 => SpawnKind::PlayerCorpse,
                    3 => SpawnKind::NpcCorpse,
                    other => SpawnKind::Unknown(other),
                },
                race: u32::from(short(record, 84)),
                gender: u32::from(record[88]),
                size: nonnegative(record, 28)?,
                invisible: record[90] != 0,
                appearance: crate::appearance::Appearance::default(),
                position: Position {
                    x: signed(record, 9),
                    y: signed(record, 7),
                    z: signed(record, 11) / 10.0,
                    heading: f32::from(record[5]) * 2.0,
                },
                // EQMac motion fields are not decoded yet.
                velocity: [0.0; 3],
                level: record[89],
                // Nor are its other /who fields.
                listing: crate::listing::Listing::default(),
                name_parts: crate::names::NameParts::default(),
                pet_owner: None,
                hp_percent: None,
            })
        })
        .collect()
}

/// Decode a single or batched authoritative position update.
fn position(body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == 15, "unexpected EQMac position layout");
    Ok(WorldEvent::Position {
        spawn_id: valid_id(u32::from(short(body, 0)))?,
        position: Position {
            x: signed(body, 7),
            y: signed(body, 5),
            z: signed(body, 9) / 10.0,
            heading: f32::from(body[3]) * 2.0,
        },
        // EQMac motion fields are not decoded yet.
        velocity: [0.0; 3],
    })
}

/// Decode ongoing entity and HUD packets; unknown opcodes produce no events.
///
/// # Errors
/// Rejects malformed recognized packets, invalid IDs, and invalid health values.
pub fn updates(opcode: u16, body: &[u8]) -> Result<Vec<WorldEvent>> {
    Ok(match opcode {
        0x5f41 | 0x6b42 => vec![WorldEvent::Spawns(spawns(body)?)],
        0xf340 => vec![position(body)?],
        crate::merchant::EQMAC_REQUEST_OPCODE
        | crate::merchant::EQMAC_DELETE_OPCODE
        | crate::merchant::EQMAC_BUY_OPCODE
        | crate::merchant::EQMAC_SELL_OPCODE
        | crate::merchant::EQMAC_END_CONFIRM_OPCODE => crate::merchant::decode_eqmac(opcode, body)?
            .unwrap_or_default()
            .into_iter()
            .map(WorldEvent::Merchant)
            .collect(),
        0x9f40 => {
            ensure!(body.len() >= 4, "truncated EQMac movement batch");
            let count = usize::try_from(word(body, 0))?;
            ensure!(
                count <= 4096 && body.len() == 4 + count * 15,
                "invalid EQMac movement batch length"
            );
            body[4..]
                .as_chunks::<15>()
                .0
                .iter()
                .map(|record| position(record))
                .collect::<Result<_>>()?
        }
        0x2940 => {
            ensure!(body.len() == 2, "invalid EQMac despawn length");
            vec![WorldEvent::Despawn(valid_id(u32::from(short(body, 0)))?)]
        }
        ZONE_DEATH => vec![WorldEvent::Death(death(body)?)],
        0xb240 => {
            ensure!(body.len() == 12, "invalid EQMac health length");
            let spawn_id = valid_id(word(body, 0))?;
            let current = i32::from_le_bytes(body[4..8].try_into()?);
            let maximum = i32::from_le_bytes(body[8..12].try_into()?);
            ensure!(
                maximum > 0 && current <= maximum,
                "invalid EQMac health values"
            );
            // Right for others; the player's own update leaves out what
            // items add, so the host takes the player's health from the hit
            // points instead.
            let percent = u8::try_from(i64::from(current.max(0)) * 100 / i64::from(maximum))?;
            vec![
                WorldEvent::HitPoints {
                    spawn_id,
                    current,
                    maximum,
                    // Read as the player's own update, the only one with
                    // real values: others' carry a percent over 100.
                    items: ItemHitPoints::LeftOutOfCurrent,
                },
                WorldEvent::HealthPercent { spawn_id, percent },
            ]
        }
        0x7f41 => {
            ensure!(body.len() == 4, "invalid EQMac mana update");
            // The mana left and the spell, as the spell bar comes back: TAKP
            // sends it only then (`Mob::SendSpellBarEnable`), so it ends
            // every cast, whether the spell landed or not.
            vec![
                WorldEvent::Spell(SpellUpdate::Mana {
                    spell_id: u32::from(short(body, 2)),
                    keep_casting: false,
                }),
                WorldEvent::Mana(u32::from(short(body, 0))),
            ]
        }
        0x1942 => {
            // The player's own mana between casts (`Client::SendManaUpdate`):
            // their spawn, then the mana.
            ensure!(body.len() == 4, "invalid EQMac own mana update");
            vec![WorldEvent::Mana(u32::from(short(body, 2)))]
        }
        crate::spells::EQMAC_BEGIN_OPCODE
        | crate::spells::EQMAC_INTERRUPT_OPCODE
        | crate::spells::EQMAC_MEMORIZE_OPCODE
        | crate::spells::EQMAC_DELETE_OPCODE
        | crate::spells::EQMAC_SWAP_OPCODE => crate::spells::decode_eqmac(opcode, body)?
            .map(WorldEvent::Spell)
            .into_iter()
            .collect(),
        crate::buffs::EQMAC_ACTION_OPCODE => crate::buffs::eqmac_spell_effect(body)?
            .map(WorldEvent::SpellEffect)
            .into_iter()
            .collect(),
        crate::buffs::EQMAC_BUFF_OPCODE => {
            vec![WorldEvent::Buff(crate::buffs::eqmac_update(body)?)]
        }
        crate::food::EQMAC_STAMINA_OPCODE => {
            vec![WorldEvent::Nourishment(crate::food::eqmac_nourishment(
                body,
            )?)]
        }
        0xf540 => crate::world::appearance(body)?.into_iter().collect(),
        crate::combat::EQMAC_DAMAGE_OPCODE => {
            vec![WorldEvent::Damage(crate::combat::eqmac_damage(body)?)]
        }
        0x9941 => {
            ensure!(
                body.len() == 4 && word(body, 0) <= 330,
                "invalid EQMac experience update"
            );
            vec![WorldEvent::Experience(word(body, 0))]
        }
        _ => Vec::new(),
    })
}

/// Read the separate own-spawn ID announcement, leaving unrelated appearances alone.
///
/// # Errors
/// Rejects truncated appearance packets and IDs outside the nonzero u16 range.
pub fn assigned_id(body: &[u8]) -> Result<Option<u16>> {
    ensure!(body.len() == 8, "invalid EQMac appearance length");
    if short(body, 2) == 16 {
        Ok(Some(valid_id(word(body, 4))?))
    } else {
        Ok(None)
    }
}

fn valid_id(value: u32) -> Result<u16> {
    let id = u16::try_from(value).context("EQMac spawn ID out of range")?;
    ensure!(id != 0, "zero EQMac spawn ID");
    Ok(id)
}
fn cstr(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())]
}
fn short(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}
fn signed(bytes: &[u8], at: usize) -> f32 {
    f32::from(i16::from_le_bytes([bytes[at], bytes[at + 1]]))
}
fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("validated layout"))
}
fn float(bytes: &[u8], at: usize) -> Result<f32> {
    let value = f32::from_le_bytes(bytes[at..at + 4].try_into()?);
    ensure!(
        value.is_finite(),
        "non-finite EQMac coordinate or dimension"
    );
    Ok(value)
}
fn nonnegative(bytes: &[u8], at: usize) -> Result<f32> {
    let value = float(bytes, at)?;
    ensure!(value >= 0.0, "negative EQMac dimension or speed");
    Ok(value)
}

#[cfg(test)]
mod tests;
