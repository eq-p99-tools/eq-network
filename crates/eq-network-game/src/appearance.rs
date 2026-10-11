//! How spawns look: their worn gear in EQ's nine texture slots, tints and
//! facial features, as Titanium spawn records and wear changes carry them.
//! Which textures and models draw that is the client's business.
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_WearChange`: one texture slot of a spawn changed.
pub const WEAR_CHANGE_OPCODE: u16 = 0x7441;

/// Appearance fields not already represented by posture, visibility, listing
/// or pet-owner events. These are reports, not movement permissions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum AppearanceChange {
    /// Light type emitted by this spawn.
    Light(u32),
    /// Whether the spawn is sneaking.
    Sneaking(bool),
    /// Server movement presentation mode (including levitation).
    FlyMode(FlyMode),
    /// Size in integral model units, as carried by the appearance packet.
    Size(u32),
    /// A kind or value not interpreted here. Spawn zero can carry server
    /// extensions, so it must not be treated as a real entity automatically.
    Unknown {
        /// Appearance discriminator.
        kind: u16,
        /// Original parameter, including sentinels.
        value: u32,
    },
}

/// The appearance protocol's movement presentation modes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum FlyMode {
    /// Ordinary grounded presentation.
    Ground,
    /// Free flight presentation.
    Flying,
    /// Levitation.
    Levitating,
    /// Water movement.
    Water,
    /// Floating.
    Floating,
    /// Levitation while running.
    RunningLevitation,
    /// Unsupported mode; not permission to fly.
    Unknown(u32),
}

/// Interprets the shared appearance kinds without discarding unknown values.
pub(crate) fn change(kind: u16, value: u32) -> AppearanceChange {
    match kind {
        5 => AppearanceChange::Light(value),
        15 if value <= 1 => AppearanceChange::Sneaking(value == 1),
        19 => AppearanceChange::FlyMode(match value {
            0 => FlyMode::Ground,
            1 => FlyMode::Flying,
            2 => FlyMode::Levitating,
            3 => FlyMode::Water,
            4 => FlyMode::Floating,
            5 => FlyMode::RunningLevitation,
            other => FlyMode::Unknown(other),
        }),
        29 => AppearanceChange::Size(value),
        _ => AppearanceChange::Unknown { kind, value },
    }
}

/// Size carried by an illusion. Keep the generations distinct: `EQMac`'s
/// integer -1 means racial default, not a negative scale or a float bit pattern.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum IllusionSize {
    /// Titanium's floating-point size.
    Titanium(f32),
    /// `EQMac`'s integral size, including its -1 racial-default sentinel.
    EqMac(i32),
}

/// Model and facial features in an illusion notification. Texture and face
/// sentinel values are retained; applying them is a separate state operation.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Illusion {
    /// Entity whose model changes.
    pub spawn_id: u16,
    /// Model race number.
    pub race: u32,
    /// Model gender, including any wire sentinel.
    pub gender: u8,
    /// Body texture, including 255's generation-specific meaning.
    pub texture: u8,
    /// Helmet texture, including 255's generation-specific meaning.
    pub helm_texture: u8,
    /// Face texture, without narrowing Titanium's 32-bit field.
    pub face: u32,
    /// Hair style.
    pub hair_style: u8,
    /// Hair color.
    pub hair_color: u8,
    /// Beard style.
    pub beard: u8,
    /// Beard color.
    pub beard_color: u8,
    /// Generation-specific model size.
    pub size: IllusionSize,
}

/// Reads an illusion, preserving each generation's field widths and sentinels.
///
/// # Errors
/// Rejects malformed lengths, invalid entity IDs and non-finite floating sizes.
pub fn illusion(dialect: crate::GameDialect, opcode: u16, body: &[u8]) -> Result<Option<Illusion>> {
    use crate::{
        wire_read::{spawn_id, word},
        GameDialect,
    };
    let (spawn, race, gender, face, features, size) = match (dialect, opcode) {
        (GameDialect::Titanium, 0x448d) => {
            ensure!(body.len() == 168, "invalid Titanium illusion length");
            let size = f32::from_bits(word(body, 84));
            ensure!(size.is_finite(), "invalid illusion size");
            (
                spawn_id(word(body, 0))?,
                word(body, 68),
                72,
                word(body, 76),
                80,
                IllusionSize::Titanium(size),
            )
        }
        (GameDialect::EqMac, 0x9140) => {
            ensure!(body.len() == 20, "invalid EQMac illusion length");
            let short = |at| u16::from_le_bytes([body[at], body[at + 1]]);
            (
                spawn_id(u32::from(short(0)))?,
                u32::from(short(2)),
                4,
                u32::from(short(8)),
                10,
                IllusionSize::EqMac(word(body, 16).cast_signed()),
            )
        }
        _ => return Ok(None),
    };
    Ok(Some(Illusion {
        spawn_id: spawn,
        race,
        gender: body[gender],
        texture: body[gender + 1],
        helm_texture: body[gender + 2],
        face,
        hair_style: body[features],
        hair_color: body[features + 1],
        beard: body[features + 2],
        beard_color: body[features + 3],
        size,
    }))
}

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
}

impl Appearance {
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

/// Reads `EQMac`'s padded twelve-byte wear update. Its material is only u16;
/// bytes 3 and 6..8 are padding and must not contribute to field values.
///
/// # Errors
/// Rejects malformed lengths, zero spawn IDs and unknown texture slots.
pub fn eqmac_wear_change(body: &[u8]) -> Result<WearChange> {
    ensure!(body.len() == 12, "invalid EQMac wear change length");
    let spawn_id = u16::from_le_bytes([body[0], body[1]]);
    ensure!(spawn_id != 0, "invalid wear change spawn ID");
    let slot = TextureSlot::from_index(usize::from(body[2]))
        .ok_or_else(|| anyhow::anyhow!("invalid wear change texture slot"))?;
    Ok(WearChange {
        spawn_id,
        slot,
        material: u32::from(u16::from_le_bytes([body[4], body[5]])),
        tint: tint(&body[8..12]),
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

    #[test]
    fn slots_round_trip_their_wire_index() {
        for (index, slot) in TextureSlot::ALL.into_iter().enumerate() {
            assert_eq!(slot.index(), index);
            assert_eq!(TextureSlot::from_index(index), Some(slot));
        }
        assert!(TextureSlot::Primary.held() && !TextureSlot::Chest.held());
    }
}
