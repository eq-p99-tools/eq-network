//! How spawns look: their worn gear in EQ's nine texture slots, tints and
//! facial features, as Titanium spawn records and wear changes carry them;
//! their light, fly mode and the like, as both generations' records carry
//! some of them; and what changes how a spawn shows: appearance updates
//! (`OP_SpawnAppearance`) on both generations, Titanium's player states, and
//! both generations' illusions and face changes. Which textures and models
//! draw that is the client's business.
//!
//! Layout reference: `EQEmu`'s `SpawnAppearance_Struct`
//! (`common/eq_packet_structs.h`), the same on both generations, and the
//! servers' `AppearanceType` numbers (`EQEmu`'s and TAKP's
//! `common/eq_constants.h`); `EQEmu`'s `PlayerState_Struct`
//! (`common/eq_packet_structs.h`) and Titanium `Illusion_Struct`
//! (`common/patches/titanium_structs.h`); and TAKP's `Illusion_Struct` and
//! `FaceChange_Struct` (`common/eq_packet_structs.h`).
use crate::world::WorldEvent;
use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// `OP_WearChange`: one texture slot of a spawn changed.
pub const WEAR_CHANGE_OPCODE: u16 = 0x7441;
/// `OP_PlayerStateAdd`: a spawn took on states (Titanium).
pub const STATES_ADDED_OPCODE: u16 = 0x63da;
/// `OP_PlayerStateRemove`: a spawn left states (Titanium).
pub const STATES_REMOVED_OPCODE: u16 = 0x381d;
/// `OP_Illusion`: a spawn took on another look, or its own again
/// (Titanium).
pub const ILLUSION_OPCODE: u16 = 0x448d;
/// `OP_Illusion` on the `EQMac` wire.
pub const EQMAC_ILLUSION_OPCODE: u16 = 0x9140;
/// `OP_FaceChange`: someone's face changed (`EQMac`).
pub const EQMAC_FACE_CHANGE_OPCODE: u16 = 0x2542;

/// EQ's texture slots, in the order servers send them.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum TextureSlot {
    /// Helmet.
    Head,
    /// Chest armor or robe.
    Chest,
    /// Sleeves.
    Arms,
    /// Bracer (the first wrist slot).
    Wrist,
    /// Gloves.
    Hands,
    /// Leggings.
    Legs,
    /// Boots.
    Feet,
    /// Main-hand item.
    Primary,
    /// Off-hand item.
    Secondary,
}

impl TextureSlot {
    /// Every slot in wire order.
    pub const ALL: [Self; 9] = [
        Self::Head,
        Self::Chest,
        Self::Arms,
        Self::Wrist,
        Self::Hands,
        Self::Legs,
        Self::Feet,
        Self::Primary,
        Self::Secondary,
    ];

    /// The slot's wire index, 0 through 8.
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The slot with this wire index, if any.
    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// Whether this slot holds an item model instead of an armor material.
    #[must_use]
    pub const fn held(self) -> bool {
        matches!(self, Self::Primary | Self::Secondary)
    }
}

/// How a spawn's gear and features look.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "each flag is one the servers set on its own"
)]
pub struct Appearance {
    /// Per [`TextureSlot`]: the material an armor slot shows (0 cloth,
    /// 1 leather, 2 chain, 3 plate, robes from 10), or for the two held slots
    /// the number of the item's model (`IT10` is 10); zero is bare or empty.
    pub materials: [u32; 9],
    /// Per [`TextureSlot`], a dye or item tint as red, green and blue.
    pub tints: [Option<[u8; 3]>; 9],
    /// Face texture, 0 through 7 on classic models.
    pub face: u8,
    /// Hair style.
    pub hair_style: u8,
    /// Hair color.
    pub hair_color: u8,
    /// Beard style.
    pub beard: u8,
    /// Beard color.
    pub beard_color: u8,
    /// Whether a player shows their helm.
    pub show_helm: bool,
    /// The type of light the spawn carries, 0 for none.
    pub light: u8,
    /// How the spawn keeps to the ground.
    pub fly_mode: FlyMode,
    /// Open to player combat, from `EQMac`'s record and updates. Titanium's
    /// record does not say; `EQEmu` sends each such player's update to a
    /// player as they zone in.
    pub pvp: bool,
    /// Link-dead, from updates alone: `EQMac`'s record marks every NPC so.
    pub linkdead: bool,
    /// Sneaking, from updates alone: TAKP and Quarm never set the `EQMac`
    /// record's mark.
    pub sneaking: bool,
    /// A player's name drawn in an NPC's color, from updates alone.
    pub npc_name_color: bool,
    /// An NPC's tint, by its number in the server's tint list, from updates
    /// alone; 0 until one comes.
    pub npc_tint: u32,
    /// How damaged a destructible NPC shows, from updates alone; 0 until one
    /// comes, as for an unharmed one.
    pub damage_state: u8,
    /// What the spawn holds or suffers that shapes how it stands and moves,
    /// from Titanium's record and updates.
    pub states: SpawnStates,
}

/// What a spawn holds or suffers that shapes how it stands and moves: a set
/// of the servers' bits (`EQEmu`'s `PlayerState`, `zone/common.h`), which
/// name players but come for NPCs too.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct SpawnStates(u32);

impl SpawnStates {
    /// 1, which `EQEmu` names open.
    pub const OPEN: Self = Self(1);
    /// 2, a weapon sheathed.
    pub const WEAPON_SHEATHED: Self = Self(2);
    /// 4, aggressive, as an NPC turns on someone.
    pub const AGGRESSIVE: Self = Self(4);
    /// 8, aggressive by force.
    pub const FORCED_AGGRESSIVE: Self = Self(8);
    /// 16, an instrument in hand.
    pub const INSTRUMENT: Self = Self(16);
    /// 32, stunned.
    pub const STUNNED: Self = Self(32);
    /// 64, a weapon in the primary hand.
    pub const PRIMARY_WEAPON: Self = Self(64);
    /// 128, a weapon in the secondary hand.
    pub const SECONDARY_WEAPON: Self = Self(128);

    /// The set with these bits.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// The set's bits.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether every state of `other` is in the set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The set with `other`'s states too.
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The set less `other`'s states.
    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl Appearance {
    /// The face and hair.
    #[must_use]
    pub const fn features(&self) -> Features {
        Features {
            face: self.face,
            hair_style: self.hair_style,
            hair_color: self.hair_color,
            beard: self.beard,
            beard_color: self.beard_color,
        }
    }

    /// Takes on a face and hair.
    pub const fn set_features(&mut self, features: Features) {
        self.face = features.face;
        self.hair_style = features.hair_style;
        self.hair_color = features.hair_color;
        self.beard = features.beard;
        self.beard_color = features.beard_color;
    }

    /// The material or held model number in one slot.
    #[must_use]
    pub const fn material(&self, slot: TextureSlot) -> u32 {
        self.materials[slot.index()]
    }

    /// The tint on one slot, if any.
    #[must_use]
    pub const fn tint(&self, slot: TextureSlot) -> Option<[u8; 3]> {
        self.tints[slot.index()]
    }

    /// Applies one slot's change.
    pub fn apply(&mut self, change: &WearChange) {
        self.materials[change.slot.index()] = change.material;
        self.tints[change.slot.index()] = change.tint;
    }

    /// Keeps what an appearance update changes here: an illusion's face
    /// and hair among it, while the size, race and gender are the spawn's.
    /// Kind 0, an area's regeneration and the kinds no server sends on its
    /// own change nothing.
    pub fn apply_update(&mut self, change: &AppearanceChange) {
        match *change {
            AppearanceChange::Pvp(on) => self.pvp = on,
            AppearanceChange::Light(light) => self.light = light,
            AppearanceChange::Sneaking(on) => self.sneaking = on,
            AppearanceChange::Linkdead(on) => self.linkdead = on,
            AppearanceChange::FlyMode(mode) => self.fly_mode = mode,
            AppearanceChange::NpcNameColor(on) => self.npc_name_color = on,
            AppearanceChange::NpcTint(tint) => self.npc_tint = tint,
            AppearanceChange::ShowHelm(on) => self.show_helm = on,
            AppearanceChange::DamageState(state) => self.damage_state = state,
            AppearanceChange::StatesAdded(states) => self.states = self.states.with(states),
            AppearanceChange::StatesRemoved(states) => self.states = self.states.without(states),
            AppearanceChange::Illusion(illusion) => self.set_features(illusion.features),
            AppearanceChange::Died(_)
            | AppearanceChange::Size(_)
            | AppearanceChange::AreaRegeneration { .. }
            | AppearanceChange::Other { .. } => (),
        }
    }
}

/// One texture slot of a spawn changed, as when it equips or removes gear.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct WearChange {
    /// Spawn whose look changed.
    pub spawn_id: u16,
    /// Slot that changed.
    pub slot: TextureSlot,
    /// New material or held model number, as in [`Appearance::materials`].
    pub material: u32,
    /// New tint, if any.
    pub tint: Option<[u8; 3]>,
}

/// Titanium's tint record: blue, green, red, and a flag set when the tint is used.
fn tint(bytes: &[u8]) -> Option<[u8; 3]> {
    (bytes[3] != 0).then_some([bytes[2], bytes[1], bytes[0]])
}

/// Reads the look from a decrypted 385-byte Titanium spawn record
/// (`EQEmu`'s `Spawn_Struct`).
pub(crate) fn titanium_spawn(record: &[u8]) -> Appearance {
    let word = |offset: usize| {
        u32::from_le_bytes([
            record[offset],
            record[offset + 1],
            record[offset + 2],
            record[offset + 3],
        ])
    };
    Appearance {
        materials: std::array::from_fn(|slot| word(197 + slot * 4)),
        tints: std::array::from_fn(|slot| tint(&record[348 + slot * 4..352 + slot * 4])),
        face: record[6],
        hair_style: record[145],
        hair_color: record[85],
        beard: record[156],
        beard_color: record[146],
        show_helm: record[139] != 0,
        light: record[330],
        fly_mode: FlyMode::from_wire(u32::from(record[333])),
        states: SpawnStates(word(152)),
        ..Appearance::default()
    }
}

/// Reads what an `EQMac` spawn record (TAKP's `mac_structs.h` `Spawn_Struct`,
/// 224 bytes) says of the look here: whether the spawn is open to player
/// combat at 92, its light at 94 and its fly mode at 100.
pub(crate) fn eqmac_spawn(record: &[u8]) -> Appearance {
    Appearance {
        pvp: record[92] != 0,
        light: record[94],
        fly_mode: FlyMode::from_wire(u32::from(record[100])),
        ..Appearance::default()
    }
}

/// Decodes Titanium's `OP_WearChange`.
///
/// # Errors
/// Rejects wrong lengths, a zero spawn ID and slots past the ninth.
pub fn titanium_wear_change(body: &[u8]) -> Result<WearChange> {
    ensure!(body.len() == 9, "invalid wear change length");
    let spawn_id = u16::from_le_bytes([body[0], body[1]]);
    ensure!(spawn_id != 0, "wear change without a spawn");
    let slot = TextureSlot::from_index(usize::from(body[8]))
        .ok_or_else(|| anyhow::anyhow!("wear change for an unknown slot"))?;
    Ok(WearChange {
        spawn_id,
        slot,
        material: u32::from(u16::from_le_bytes([body[2], body[3]])),
        tint: tint(&body[4..8]),
    })
}

/// The appearance kinds read here, by the servers' numbers. Those that say
/// how `/who` lists a player are [`crate::listing`]'s, and a pet's owner is
/// [`crate::pets`]'s.
const DIE: u16 = 0;
const INVISIBILITY: u16 = 3;
const PVP: u16 = 4;
const LIGHT: u16 = 5;
const POSTURE: u16 = 14;
const SNEAK: u16 = 15;
const LINKDEAD: u16 = 18;
const FLY_MODE: u16 = 19;
const SIZE: u16 = 29;
const NPC_NAME_COLOR: u16 = 31;
const AREA_HEALTH_REGENERATION: u16 = 35;
const AREA_MANA_REGENERATION: u16 = 36;
const AREA_ENDURANCE_REGENERATION: u16 = 37;
const NPC_TINT: u16 = 39;
const SHOW_HELM: u16 = 43;
const DAMAGE_STATE: u16 = 44;

/// How a spawn keeps to the ground, by the servers' numbers and names
/// (`AppearanceType::FlyMode`); how each looks is the client's business.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub enum FlyMode {
    /// 0, on the ground.
    #[default]
    Off,
    /// 1, flying.
    Flying,
    /// 2, levitating, as a levitation spell sets it.
    Levitating,
    /// 3, which the servers name water.
    Water,
    /// 4, floating.
    Floating,
    /// 5, levitating while running.
    LevitatingWhileRunning,
    /// A number past those, kept as sent.
    Other(u32),
}

impl FlyMode {
    /// The mode with this number.
    const fn from_wire(value: u32) -> Self {
        match value {
            0 => Self::Off,
            1 => Self::Flying,
            2 => Self::Levitating,
            3 => Self::Water,
            4 => Self::Floating,
            5 => Self::LevitatingWhileRunning,
            other => Self::Other(other),
        }
    }
}

/// What an area's regeneration multiplies.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Pool {
    /// Health.
    Health,
    /// Mana.
    Mana,
    /// Endurance.
    Endurance,
}

/// A face and hair, as [`Appearance`] holds them.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Features {
    /// Face texture.
    pub face: u8,
    /// Hair style.
    pub hair_style: u8,
    /// Hair color.
    pub hair_color: u8,
    /// Beard style.
    pub beard: u8,
    /// Beard color.
    pub beard_color: u8,
}

/// A spawn taking on another look, or its own again, as an illusion spell
/// lands or fades or a game master or quest changes its race, gender or
/// texture.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Illusion {
    /// The race whose model the spawn takes.
    pub race: u32,
    /// The model's gender.
    pub gender: u8,
    /// The body texture; None for a player race, whose gear shows.
    pub texture: Option<u8>,
    /// The helm texture; None for a player race, whose gear shows.
    pub helm_texture: Option<u8>,
    /// The size; 0 for the race's own, as `EQMac`'s record sends it.
    pub size: f32,
    /// The face and hair.
    pub features: Features,
}

/// A change in how a spawn shows, from an appearance update of a kind the
/// listing, visibility, posture and pet events leave out.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum AppearanceChange {
    /// Kind 0, with its value as sent. `EQEmu`'s own note says it "causes the
    /// client to keel over and zone to bind point" (unverified); `EQEmu`
    /// sends it only for a legacy spell effect, with the lying posture's
    /// number (115), and TAKP and Quarm only on a game master's word.
    Died(u32),
    /// Kind 4: the spawn is open to player combat, or no longer.
    Pvp(bool),
    /// Kind 5: the type of light the spawn carries, 0 for none.
    Light(u8),
    /// Kind 15: sneaking, or no longer.
    Sneaking(bool),
    /// Kind 18: link-dead, or back; the servers also mark a charmed or
    /// feared player link-dead so the tag shows by their name.
    Linkdead(bool),
    /// Kind 19: how the spawn keeps to the ground.
    FlyMode(FlyMode),
    /// Kind 29: the spawn's new size, in whole units.
    Size(f32),
    /// Kind 31: a player's name drawn in an NPC's color, or no longer.
    NpcNameColor(bool),
    /// Kinds 35 to 37, to the player alone: what the area multiplies the
    /// pool's regeneration by, sent in thousandths; 1 is no change.
    AreaRegeneration {
        /// Health, mana or endurance.
        pool: Pool,
        /// The multiplier.
        multiplier: f64,
    },
    /// Kind 39: an NPC's tint, by its number in the server's tint list.
    NpcTint(u32),
    /// Kind 43: whether a player shows their helm.
    ShowHelm(bool),
    /// Kind 44: how damaged a destructible NPC shows; `EQEmu` sends 0 to 4 as
    /// its health falls by quarters.
    DamageState(u8),
    /// States the spawn took on, from Titanium's `OP_PlayerStateAdd`, which
    /// `EQEmu` sends the zone, the spawn's own player included, as a stun
    /// lands, an NPC turns aggressive or takes up a looted weapon, and as
    /// another player says theirs changed.
    StatesAdded(SpawnStates),
    /// States the spawn left, from Titanium's `OP_PlayerStateRemove`, sent
    /// the same way.
    StatesRemoved(SpawnStates),
    /// Another look, or the spawn's own again, from `OP_Illusion`, which
    /// every server sends the zone, the spawn's own player included.
    Illusion(Illusion),
    /// A kind no server sends on its own, kept as a game master's
    /// `#appearance` sends it; most are the client's own reports and
    /// settings.
    Other {
        /// The kind's number.
        kind: u16,
        /// Its value as sent.
        value: u32,
    },
}

impl AppearanceChange {
    /// The change an update of this kind makes, to this value.
    fn read(kind: u16, value: u32) -> Result<Self> {
        let on = value != 0;
        Ok(match kind {
            DIE => Self::Died(value),
            PVP => Self::Pvp(on),
            LIGHT => Self::Light(u8::try_from(value).context("light type out of range")?),
            SNEAK => Self::Sneaking(on),
            LINKDEAD => Self::Linkdead(on),
            FLY_MODE => Self::FlyMode(FlyMode::from_wire(value)),
            SIZE => Self::Size(f32::from(
                u16::try_from(value).context("appearance size out of range")?,
            )),
            NPC_NAME_COLOR => Self::NpcNameColor(on),
            AREA_HEALTH_REGENERATION | AREA_MANA_REGENERATION | AREA_ENDURANCE_REGENERATION => {
                Self::AreaRegeneration {
                    pool: match kind {
                        AREA_HEALTH_REGENERATION => Pool::Health,
                        AREA_MANA_REGENERATION => Pool::Mana,
                        _ => Pool::Endurance,
                    },
                    multiplier: f64::from(value) / 1000.0,
                }
            }
            NPC_TINT => Self::NpcTint(value),
            SHOW_HELM => Self::ShowHelm(on),
            DAMAGE_STATE => {
                Self::DamageState(u8::try_from(value).context("damage state out of range")?)
            }
            kind => Self::Other { kind, value },
        })
    }
}

/// Decodes `OP_SpawnAppearance` (Titanium's 0x7c32, `EQMac`'s 0xf540): a
/// spawn, a kind and a value. A record naming no spawn is the server
/// addressing the client itself, never a spawn's look: the player's own spawn
/// ID as they zone in, which the admission reads, and Quarm's client library's
/// handshakes, which [`crate::quarm`] answers.
///
/// # Errors
/// Rejects a wrong length, a value out of its kind's range, and a listing,
/// visibility, posture or pet update naming no spawn.
pub(crate) fn update(body: &[u8]) -> Result<Option<WorldEvent>> {
    ensure!(body.len() == 8, "invalid appearance length");
    let spawn_id = u16::from_le_bytes([body[0], body[1]]);
    let kind = u16::from_le_bytes([body[2], body[3]]);
    let value = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
    let listing = crate::listing::appearance(kind, value)?;
    if spawn_id == 0 {
        ensure!(
            listing.is_none()
                && !matches!(kind, INVISIBILITY | POSTURE | crate::pets::PET_APPEARANCE),
            "invalid appearance spawn ID"
        );
        return Ok(None);
    }
    Ok(Some(match (kind, listing) {
        (_, Some(change)) => WorldEvent::Listing { spawn_id, change },
        (INVISIBILITY, None) => WorldEvent::Visibility {
            spawn_id,
            invisible: value != 0,
        },
        (POSTURE, None) => WorldEvent::Posture {
            spawn_id,
            posture: value.into(),
        },
        (crate::pets::PET_APPEARANCE, None) => WorldEvent::PetOwner {
            spawn_id,
            owner: u16::try_from(value).ok().filter(|owner| *owner != 0),
        },
        (kind, None) => WorldEvent::Appearance {
            spawn_id,
            change: AppearanceChange::read(kind, value)?,
        },
    }))
}

/// What Titanium's messages on how spawns look say: appearance updates,
/// wear changes and player states; None for other messages.
///
/// # Errors
/// Rejects malformed records of those messages.
pub(crate) fn titanium(opcode: u16, body: &[u8]) -> Result<Option<WorldEvent>> {
    Ok(match opcode {
        crate::listing::APPEARANCE_OPCODE => update(body)?,
        WEAR_CHANGE_OPCODE => Some(WorldEvent::WearChange(titanium_wear_change(body)?)),
        STATES_ADDED_OPCODE | STATES_REMOVED_OPCODE => Some(titanium_states(opcode, body)?),
        ILLUSION_OPCODE => Some(titanium_illusion(body)?),
        _ => None,
    })
}

/// A texture, unless 0xff, with which the servers leave a player race's
/// gear to show.
fn texture(byte: u8) -> Option<u8> {
    (byte != u8::MAX).then_some(byte)
}

/// Decodes Titanium's `OP_Illusion` (168 bytes): the spawn in 32 bits, its
/// name at 4, the race at 68, gender, texture and helm texture at 72 to 74,
/// the face in 32 bits at 76, hair and beard at 80 to 83 and the size at 84.
/// `EQEmu`'s Titanium encoder sends a face change (`OP_SetFace`) as one of
/// race -1, which reads as [`WorldEvent::FaceChanged`].
///
/// # Errors
/// Rejects a wrong length, a spawn ID that is zero or past 16 bits, another
/// negative race, a face past 255 and a size that is negative or not a
/// number.
pub(crate) fn titanium_illusion(body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == 168, "invalid illusion length");
    let word = |at: usize| u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]);
    let spawn_id = u16::try_from(word(0)).context("illusion spawn ID out of range")?;
    ensure!(spawn_id != 0, "illusion without a spawn");
    let features = Features {
        face: u8::try_from(word(76)).context("illusion face out of range")?,
        hair_style: body[80],
        hair_color: body[81],
        beard: body[82],
        beard_color: body[83],
    };
    let race = word(68).cast_signed();
    if race == -1 {
        return Ok(WorldEvent::FaceChanged {
            spawn_id: Some(spawn_id),
            features,
            eye_colors: None,
        });
    }
    let size = f32::from_bits(word(84));
    ensure!(size.is_finite() && size >= 0.0, "invalid illusion size");
    Ok(WorldEvent::Appearance {
        spawn_id,
        change: AppearanceChange::Illusion(Illusion {
            race: u32::try_from(race).context("invalid illusion race")?,
            gender: body[72],
            texture: texture(body[73]),
            helm_texture: texture(body[74]),
            size,
            features,
        }),
    })
}

/// Decodes `EQMac`'s `OP_Illusion` (20 bytes): the spawn, the race,
/// gender, texture and helm texture, the face, hair and beard, and the size
/// in whole units at 16, -1 for the race's own (TAKP and Quarm
/// `Mob::SendIllusionPacket`).
///
/// # Errors
/// Rejects a wrong length, a zero spawn ID, a negative race, a face past
/// 255 and a size below -1 or past 16 bits.
pub(crate) fn eqmac_illusion(body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == 20, "invalid EQMac illusion length");
    let spawn_id = u16::from_le_bytes([body[0], body[1]]);
    ensure!(spawn_id != 0, "illusion without a spawn");
    let size = match i32::from_le_bytes([body[16], body[17], body[18], body[19]]) {
        -1 => 0.0,
        size => f32::from(u16::try_from(size).context("invalid illusion size")?),
    };
    Ok(WorldEvent::Appearance {
        spawn_id,
        change: AppearanceChange::Illusion(Illusion {
            race: u32::try_from(i16::from_le_bytes([body[2], body[3]]))
                .context("invalid illusion race")?,
            gender: body[4],
            texture: texture(body[5]),
            helm_texture: texture(body[6]),
            size,
            features: Features {
                face: u8::try_from(i16::from_le_bytes([body[8], body[9]]))
                    .context("illusion face out of range")?,
                hair_style: body[10],
                hair_color: body[11],
                beard: body[12],
                beard_color: body[13],
            },
        }),
    })
}

/// Decodes `EQMac`'s `OP_FaceChange` (7 bytes): hair and beard colors, two
/// eye colors, hair style, beard and face. TAKP and Quarm relay a player's
/// own face change to the whole zone, that player included, and the record
/// names no one.
///
/// # Errors
/// Rejects a wrong length.
pub(crate) fn eqmac_face_change(body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == 7, "invalid EQMac face change length");
    Ok(WorldEvent::FaceChanged {
        spawn_id: None,
        features: Features {
            face: body[6],
            hair_style: body[4],
            hair_color: body[0],
            beard: body[5],
            beard_color: body[1],
        },
        eye_colors: Some([body[2], body[3]]),
    })
}

/// Decodes Titanium's `OP_PlayerStateAdd` or `OP_PlayerStateRemove`: the
/// spawn in 32 bits, then the states (`PlayerState_Struct`).
///
/// # Errors
/// Rejects a wrong length and a spawn ID that is zero or past 16 bits.
pub(crate) fn titanium_states(opcode: u16, body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == 8, "invalid player state length");
    let spawn_id = u16::try_from(u32::from_le_bytes([body[0], body[1], body[2], body[3]]))
        .context("player state spawn ID out of range")?;
    ensure!(spawn_id != 0, "player state without a spawn");
    let states = SpawnStates(u32::from_le_bytes([body[4], body[5], body[6], body[7]]));
    Ok(WorldEvent::Appearance {
        spawn_id,
        change: if opcode == STATES_ADDED_OPCODE {
            AppearanceChange::StatesAdded(states)
        } else {
            AppearanceChange::StatesRemoved(states)
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_records_carry_materials_tints_and_features() {
        let mut record = [0u8; 385];
        record[6] = 3;
        record[85] = 2;
        record[139] = 1;
        record[145] = 4;
        record[146] = 5;
        record[156] = 6;
        // Chain chest, a held IT10 and a red dye on the chest.
        record[197 + 4..197 + 8].copy_from_slice(&2u32.to_le_bytes());
        record[197 + 28..197 + 32].copy_from_slice(&10u32.to_le_bytes());
        record[348 + 4..348 + 8].copy_from_slice(&[0x10, 0x20, 0xc0, 0xff]);
        record[330] = 6;
        record[333] = 1;
        record[152..156].copy_from_slice(&(64u32 | 4).to_le_bytes());
        let look = titanium_spawn(&record);
        assert_eq!(look.material(TextureSlot::Chest), 2);
        assert_eq!(look.material(TextureSlot::Primary), 10);
        assert_eq!(look.tint(TextureSlot::Chest), Some([0xc0, 0x20, 0x10]));
        assert_eq!(look.tint(TextureSlot::Head), None);
        assert_eq!(
            (
                look.face,
                look.hair_color,
                look.hair_style,
                look.beard_color,
                look.beard
            ),
            (3, 2, 4, 5, 6)
        );
        assert!(look.show_helm);
        assert_eq!((look.light, look.fly_mode), (6, FlyMode::Flying));
        assert_eq!(
            look.states,
            SpawnStates::PRIMARY_WEAPON.with(SpawnStates::AGGRESSIVE)
        );
    }

    #[test]
    fn player_states_come_and_go_by_the_bit() {
        let body = |spawn_id: u32, states: u32| {
            let mut body = spawn_id.to_le_bytes().to_vec();
            body.extend_from_slice(&states.to_le_bytes());
            body
        };
        let stunned = SpawnStates::STUNNED;
        for (opcode, change) in [
            (STATES_ADDED_OPCODE, AppearanceChange::StatesAdded(stunned)),
            (
                STATES_REMOVED_OPCODE,
                AppearanceChange::StatesRemoved(stunned),
            ),
        ] {
            assert_eq!(
                crate::world::titanium_update(opcode, &body(73, 32)).unwrap(),
                Some(WorldEvent::Appearance {
                    spawn_id: 73,
                    change
                })
            );
            assert!(titanium_states(opcode, &body(73, 32)[..7]).is_err());
            assert!(titanium_states(opcode, &body(0, 32)).is_err());
            assert!(titanium_states(opcode, &body(70_000, 32)).is_err());
        }
        let mut look = Appearance::default();
        let held = SpawnStates::from_bits(64 | 128);
        look.apply_update(&AppearanceChange::StatesAdded(held));
        look.apply_update(&AppearanceChange::StatesAdded(stunned));
        look.apply_update(&AppearanceChange::StatesRemoved(
            SpawnStates::SECONDARY_WEAPON,
        ));
        assert!(look
            .states
            .contains(SpawnStates::PRIMARY_WEAPON.with(stunned)));
        assert!(!look.states.contains(SpawnStates::SECONDARY_WEAPON));
        assert_eq!(look.states.bits(), 64 | 32);
    }

    #[test]
    fn eqmac_records_carry_pvp_light_and_fly_mode() {
        let mut record = [0u8; 224];
        // TAKP marks every NPC link-dead, which is not read.
        record[98] = 1;
        record[92] = 1;
        record[94] = 3;
        record[100] = 2;
        assert_eq!(
            eqmac_spawn(&record),
            Appearance {
                pvp: true,
                light: 3,
                fly_mode: FlyMode::Levitating,
                ..Appearance::default()
            }
        );
    }

    #[test]
    fn lasting_updates_change_the_look() {
        let mut look = Appearance::default();
        for change in [
            AppearanceChange::Pvp(true),
            AppearanceChange::Light(2),
            AppearanceChange::Sneaking(true),
            AppearanceChange::Linkdead(true),
            AppearanceChange::FlyMode(FlyMode::Water),
            AppearanceChange::NpcNameColor(true),
            AppearanceChange::NpcTint(5),
            AppearanceChange::ShowHelm(true),
            AppearanceChange::DamageState(2),
        ] {
            look.apply_update(&change);
        }
        let changed = Appearance {
            show_helm: true,
            light: 2,
            fly_mode: FlyMode::Water,
            pvp: true,
            linkdead: true,
            sneaking: true,
            npc_name_color: true,
            npc_tint: 5,
            damage_state: 2,
            ..Appearance::default()
        };
        assert_eq!(look, changed);
        for change in [
            AppearanceChange::Died(115),
            AppearanceChange::Size(3.0),
            AppearanceChange::AreaRegeneration {
                pool: Pool::Mana,
                multiplier: 2.0,
            },
            AppearanceChange::Other { kind: 2, value: 9 },
        ] {
            look.apply_update(&change);
        }
        assert_eq!(look, changed);
    }

    #[test]
    fn wear_changes_update_one_slot() {
        let body = [9, 0, 3, 0, 0x30, 0x20, 0x10, 0xff, 5];
        let change = titanium_wear_change(&body).unwrap();
        assert_eq!(
            change,
            WearChange {
                spawn_id: 9,
                slot: TextureSlot::Legs,
                material: 3,
                tint: Some([0x10, 0x20, 0x30]),
            }
        );
        let mut look = Appearance::default();
        look.apply(&change);
        assert_eq!(look.material(TextureSlot::Legs), 3);
        assert_eq!(look.tint(TextureSlot::Legs), Some([0x10, 0x20, 0x30]));
        let untinted = [9, 0, 0, 0, 1, 2, 3, 0, 7];
        let change = titanium_wear_change(&untinted).unwrap();
        assert_eq!((change.slot, change.tint), (TextureSlot::Primary, None));
        assert!(titanium_wear_change(&body[..8]).is_err());
        assert!(titanium_wear_change(&[0, 0, 3, 0, 0, 0, 0, 0, 5]).is_err());
        assert!(titanium_wear_change(&[9, 0, 3, 0, 0, 0, 0, 0, 9]).is_err());
    }

    fn record(spawn_id: u16, kind: u16, value: u32) -> Vec<u8> {
        let mut body = spawn_id.to_le_bytes().to_vec();
        body.extend_from_slice(&kind.to_le_bytes());
        body.extend_from_slice(&value.to_le_bytes());
        body
    }

    #[test]
    fn appearance_updates_read_every_kind_on_a_spawn() {
        let change = |kind, value| match update(&record(73, kind, value)).unwrap() {
            Some(WorldEvent::Appearance {
                spawn_id: 73,
                change,
            }) => change,
            other => panic!("{other:?}"),
        };
        assert_eq!(change(0, 115), AppearanceChange::Died(115));
        assert_eq!(change(4, 1), AppearanceChange::Pvp(true));
        assert_eq!(change(5, 2), AppearanceChange::Light(2));
        assert_eq!(change(15, 0), AppearanceChange::Sneaking(false));
        assert_eq!(change(18, 1), AppearanceChange::Linkdead(true));
        for (value, mode) in [
            (0, FlyMode::Off),
            (1, FlyMode::Flying),
            (2, FlyMode::Levitating),
            (3, FlyMode::Water),
            (4, FlyMode::Floating),
            (5, FlyMode::LevitatingWhileRunning),
            (6, FlyMode::Other(6)),
        ] {
            assert_eq!(change(19, value), AppearanceChange::FlyMode(mode));
        }
        assert_eq!(change(29, 12), AppearanceChange::Size(12.0));
        assert_eq!(change(31, 1), AppearanceChange::NpcNameColor(true));
        for (kind, pool) in [(35, Pool::Health), (36, Pool::Mana), (37, Pool::Endurance)] {
            assert_eq!(
                change(kind, 2500),
                AppearanceChange::AreaRegeneration {
                    pool,
                    multiplier: 2.5
                }
            );
        }
        assert_eq!(change(39, 17), AppearanceChange::NpcTint(17));
        assert_eq!(change(43, 0), AppearanceChange::ShowHelm(false));
        assert_eq!(change(44, 3), AppearanceChange::DamageState(3));
        // What no server sends on its own is kept as a game master sent it.
        assert_eq!(
            change(17, 90),
            AppearanceChange::Other {
                kind: 17,
                value: 90
            }
        );
        assert_eq!(
            change(16, 5),
            AppearanceChange::Other { kind: 16, value: 5 }
        );
        for (kind, value) in [(5, 256), (29, 65_536), (44, 256)] {
            assert!(update(&record(73, kind, value)).is_err(), "{kind}");
        }
    }

    #[test]
    fn records_naming_no_spawn_are_the_servers_word_to_the_client() {
        // The player's own spawn ID, Quarm's client library's handshakes and
        // any other kind: none is a spawn's look.
        for (kind, value) in [(16, 73), (256, 4 << 16 | 7), (257, 1), (4, 1), (29, 6)] {
            assert_eq!(update(&record(0, kind, value)).unwrap(), None, "{kind}");
            assert_eq!(
                crate::quarm::updates(0xf540, &record(0, kind, value)).unwrap(),
                []
            );
        }
        // The listing, visibility, posture and pet always name one.
        for kind in [1, 3, 14, 20, 21, 22, 23, 24, 25] {
            assert!(update(&record(0, kind, 1)).is_err(), "{kind}");
        }
    }

    #[test]
    fn guild_ranks_join_the_listing() {
        use crate::listing::{GuildRank, ListingChange};
        assert_eq!(
            update(&record(73, 23, 2)).unwrap(),
            Some(WorldEvent::Listing {
                spawn_id: 73,
                change: ListingChange::GuildRank(GuildRank::Leader)
            })
        );
    }

    #[test]
    fn titanium_illusions_change_the_look_and_race_minus_one_the_face() {
        let mut body = vec![0u8; 168];
        body[..4].copy_from_slice(&73u32.to_le_bytes());
        body[4..9].copy_from_slice(b"Synth");
        body[68..72].copy_from_slice(&60i32.to_le_bytes());
        body[72] = 1;
        body[73] = 2;
        body[74] = 0xff;
        body[76..80].copy_from_slice(&3u32.to_le_bytes());
        body[80..84].copy_from_slice(&[4, 5, 6, 7]);
        body[84..88].copy_from_slice(&5.5f32.to_le_bytes());
        let features = Features {
            face: 3,
            hair_style: 4,
            hair_color: 5,
            beard: 6,
            beard_color: 7,
        };
        assert_eq!(
            crate::world::titanium_update(ILLUSION_OPCODE, &body).unwrap(),
            Some(WorldEvent::Appearance {
                spawn_id: 73,
                change: AppearanceChange::Illusion(Illusion {
                    race: 60,
                    gender: 1,
                    texture: Some(2),
                    helm_texture: None,
                    size: 5.5,
                    features,
                })
            })
        );
        // `EQEmu`'s face change: race -1, the rest 0xff and size 0.
        let mut face = body.clone();
        face[4..68].fill(0);
        face[68..72].copy_from_slice(&(-1i32).to_le_bytes());
        face[72..75].fill(0xff);
        face[84..88].fill(0);
        assert_eq!(
            titanium_illusion(&face).unwrap(),
            WorldEvent::FaceChanged {
                spawn_id: Some(73),
                features,
                eye_colors: None
            }
        );
        let broken = |at: std::ops::Range<usize>, bytes: &[u8]| {
            let mut broken = body.clone();
            broken[at].copy_from_slice(bytes);
            titanium_illusion(&broken).is_err()
        };
        assert!(broken(0..4, &0u32.to_le_bytes()));
        assert!(broken(0..4, &70_000u32.to_le_bytes()));
        assert!(broken(68..72, &(-2i32).to_le_bytes()));
        assert!(broken(76..80, &256u32.to_le_bytes()));
        assert!(broken(84..88, &(-1.0f32).to_le_bytes()));
        assert!(broken(84..88, &f32::NAN.to_le_bytes()));
        assert!(titanium_illusion(&body[..167]).is_err());
    }

    #[test]
    fn eqmac_illusions_size_by_the_race_at_minus_one_and_face_changes_name_no_one() {
        let mut body = [0u8; 20];
        body[..2].copy_from_slice(&73u16.to_le_bytes());
        body[2..4].copy_from_slice(&75i16.to_le_bytes());
        body[4] = 2;
        body[5] = 0xff;
        body[6] = 0xff;
        body[7] = 0xff;
        body[8..10].copy_from_slice(&1i16.to_le_bytes());
        body[10..14].copy_from_slice(&[2, 3, 4, 5]);
        body[16..20].copy_from_slice(&(-1i32).to_le_bytes());
        let illusion = |size| {
            vec![WorldEvent::Appearance {
                spawn_id: 73,
                change: AppearanceChange::Illusion(Illusion {
                    race: 75,
                    gender: 2,
                    texture: None,
                    helm_texture: None,
                    size,
                    features: Features {
                        face: 1,
                        hair_style: 2,
                        hair_color: 3,
                        beard: 4,
                        beard_color: 5,
                    },
                }),
            }]
        };
        assert_eq!(
            crate::quarm::updates(EQMAC_ILLUSION_OPCODE, &body).unwrap(),
            illusion(0.0)
        );
        body[16..20].copy_from_slice(&12i32.to_le_bytes());
        assert_eq!(
            crate::quarm::updates(EQMAC_ILLUSION_OPCODE, &body).unwrap(),
            illusion(12.0)
        );
        for (at, bytes) in [
            (0, 0i16.to_le_bytes()),
            (2, (-1i16).to_le_bytes()),
            (8, 256i16.to_le_bytes()),
        ] {
            let mut broken = body;
            broken[at..at + 2].copy_from_slice(&bytes);
            assert!(eqmac_illusion(&broken).is_err(), "{at}");
        }
        let mut broken = body;
        broken[16..20].copy_from_slice(&(-2i32).to_le_bytes());
        assert!(eqmac_illusion(&broken).is_err());
        assert!(eqmac_illusion(&body[..19]).is_err());
        assert_eq!(
            crate::quarm::updates(EQMAC_FACE_CHANGE_OPCODE, &[1, 2, 3, 4, 5, 6, 7]).unwrap(),
            [WorldEvent::FaceChanged {
                spawn_id: None,
                features: Features {
                    face: 7,
                    hair_style: 5,
                    hair_color: 1,
                    beard: 6,
                    beard_color: 2,
                },
                eye_colors: Some([3, 4]),
            }]
        );
        assert!(eqmac_face_change(&[0; 8]).is_err());
    }

    #[test]
    fn slots_round_trip_their_wire_index() {
        for (index, slot) in TextureSlot::ALL.into_iter().enumerate() {
            assert_eq!(slot.index(), index);
            assert_eq!(TextureSlot::from_index(index), Some(slot));
        }
        assert!(TextureSlot::Primary.held() && !TextureSlot::Chest.held());
    }
}
