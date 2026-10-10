//! How spawns look: their worn gear in EQ's nine texture slots, tints and
//! facial features, as Titanium spawn records and wear changes carry them,
//! and the appearance updates (`OP_SpawnAppearance`) that change how a spawn
//! shows on both generations. Which textures and models draw that is the
//! client's business.
//!
//! Layout reference: `EQEmu`'s `SpawnAppearance_Struct`
//! (`common/eq_packet_structs.h`), the same on both generations, and the
//! servers' `AppearanceType` numbers (`EQEmu`'s and TAKP's
//! `common/eq_constants.h`).
use crate::world::WorldEvent;
use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// `OP_WearChange`: one texture slot of a spawn changed.
pub const WEAR_CHANGE_OPCODE: u16 = 0x7441;

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
    fn slots_round_trip_their_wire_index() {
        for (index, slot) in TextureSlot::ALL.into_iter().enumerate() {
            assert_eq!(slot.index(), index);
            assert_eq!(TextureSlot::from_index(index), Some(slot));
        }
        assert!(TextureSlot::Primary.held() && !TextureSlot::Chest.held());
    }
}
