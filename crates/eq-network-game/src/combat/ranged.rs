//! Titanium's manual ranged attack. The packet names a target and a skill.
//! The server resolves equipment, ammunition, and the attack's outcome.

use crate::{
    command::EncodedCommand,
    inventory::{Inventory, InventoryItem, InventorySlot},
    targets::TargetState,
};
use serde::Serialize;
use std::time::{Duration, Instant};

const BOW: u8 = 5;
const LARGE_THROWING: u8 = 7;
const SMALL_THROWING: u8 = 19;
const ARROW: u8 = 27;
const CONTAINER_CLASS: u8 = 1;
const QUIVER_BAG_TYPE: u8 = 2;
const RANGED: InventorySlot = InventorySlot(11);
const AMMO: InventorySlot = InventorySlot(21);

/// The skill selected by the equipped ranged weapon.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum RangedKind {
    /// Fire an arrow from a bow.
    Archery,
    /// Throw a weapon from the ranged slot.
    Throwing,
}

impl RangedKind {
    /// Index in the Titanium skill list and value in the wire request.
    #[must_use]
    pub const fn skill(self) -> u8 {
        match self {
            Self::Archery => 7,
            Self::Throwing => 51,
        }
    }
}

/// The current equipment behind a single ranged intent. The inventory
/// revision guards even replacements with the same item ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RangedLoadout {
    revision: u64,
    weapon: u32,
    ammo: Option<u32>,
    kind: RangedKind,
    range: u16,
    delay: u8,
    quiver_weight_reduction: Option<u8>,
}

impl RangedLoadout {
    /// Read the equipped bow and arrow or equipped throwing weapon. Spare
    /// stacks are the server's to choose, even when the equipped stack has one unit.
    ///
    /// # Errors
    /// Refuses uncertain inventory, missing or mismatched equipment, and
    /// missing weapon delay or range.
    pub fn current(inventory: &Inventory) -> Result<Self, &'static str> {
        if !inventory.received() || inventory.stale() || inventory.unresolved_predictions() > 0 {
            return Err("Ranged equipment is still being resolved");
        }
        let weapon = inventory
            .items()
            .get(&RANGED)
            .filter(|item| item.bag_slots == 0)
            .ok_or("Equip a ranged weapon first")?;
        let (kind, ammo) = match weapon.rules.item_type {
            BOW => {
                let arrow = inventory
                    .items()
                    .get(&AMMO)
                    .filter(|item| item.rules.item_type == ARROW && has_projectile(item))
                    .ok_or("Equip arrows to shoot the bow")?;
                (RangedKind::Archery, Some(arrow))
            }
            LARGE_THROWING | SMALL_THROWING if has_projectile(weapon) => {
                (RangedKind::Throwing, None)
            }
            LARGE_THROWING | SMALL_THROWING => return Err("There is nothing equipped to throw"),
            _ => return Err("The ranged slot does not hold a bow or throwing weapon"),
        };
        let range = property(weapon, "Range");
        let delay = property(weapon, "Delay");
        if range == 0 || delay == 0 {
            return Err("The ranged weapon's reach or delay is unavailable");
        }
        Ok(Self {
            revision: inventory.revision(),
            weapon: weapon.details.id,
            ammo: ammo.map(|item| item.details.id),
            kind,
            range: u16::from(range) + ammo.map_or(0, |item| u16::from(property(item, "Range"))),
            delay,
            quiver_weight_reduction: if kind == RangedKind::Archery {
                confirmed_quiver_weight_reduction(inventory)
            } else {
                None
            },
        })
    }

    /// The action selected by the equipped weapon.
    #[must_use]
    pub const fn kind(self) -> RangedKind {
        self.kind
    }

    /// Item range, including the equipped arrow's range for archery.
    #[must_use]
    pub const fn item_range(self) -> u16 {
        self.range
    }

    /// Titanium's unmodified weapon delay, in tenths of a second.
    #[must_use]
    pub fn base_delay(self) -> Duration {
        Duration::from_millis(u64::from(self.delay) * 100)
    }

    /// The local Titanium reuse estimate from weapon delay and verified quiver
    /// placement. The native haste/slow rate is unavailable to this estimate.
    #[must_use]
    pub fn local_reuse_delay(self) -> Duration {
        let base = self.base_delay();
        let Some(weight_reduction) = self.quiver_weight_reduction else {
            return base;
        };
        // Titanium 0x47f01e truncates delay_ms * WR * float32(0.0025)
        // and adds one. The constant is just below 1/400 and the game
        // loop uses x87 round-toward-zero (0x4acf33). For delay_ms =
        // 100 * delay, this is ceil(delay * WR / 4) for positive byte
        // values, including products exactly divisible by four.
        let reduction = (u64::from(self.delay) * u64::from(weight_reduction)).div_ceil(4);
        let adjusted = base.saturating_sub(Duration::from_millis(reduction));
        if adjusted > Duration::from_secs(1) {
            adjusted
        } else {
            base
        }
    }
}

fn confirmed_quiver_weight_reduction(inventory: &Inventory) -> Option<u8> {
    // 0x47f01e uses the first eligible general slot, regardless of contents.
    for slot in (22..=29).map(InventorySlot) {
        if !inventory.custody_known(slot) {
            return None;
        }
        let Some(item) = inventory.items().get(&slot) else {
            continue;
        };
        let rules = &item.rules;
        if rules.bag_type == QUIVER_BAG_TYPE && rules.bag_weight_reduction > 0 {
            // An earlier unreadable class could itself be the first quiver.
            if rules.titanium_item_class? == CONTAINER_CLASS {
                return Some(rules.bag_weight_reduction);
            }
        }
    }
    None
}

fn has_projectile(item: &InventoryItem) -> bool {
    item.bag_slots == 0 && item.stack_count != Some(0)
}

fn property(item: &InventoryItem, name: &str) -> u8 {
    item.details
        .stats
        .iter()
        .find(|stat| stat.label == name)
        .and_then(|stat| u8::try_from(stat.value).ok())
        .unwrap_or_default()
}

/// An explicit request for one ordinary ranged attack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RangedAttack {
    /// The active zone admission.
    pub session_id: u64,
    /// The reconciled current selection, including its incarnation revision.
    pub target: TargetState,
    /// The loadout seen when the request was made.
    pub loadout: RangedLoadout,
    /// When the host made the request.
    pub created: Instant,
}

/// Titanium's 12-byte combat ability record for the ranged slot, matching
/// `EQEmu`'s `CombatAbility_Struct` and `Client::OPCombatAbility`.
pub(crate) fn titanium(target_id: u16, kind: RangedKind) -> anyhow::Result<EncodedCommand> {
    anyhow::ensure!(target_id != 0, "a ranged attack requires a target");
    let mut body = vec![0; 12];
    body[..4].copy_from_slice(&u32::from(target_id).to_le_bytes());
    body[4] = 11;
    body[8] = kind.skill();
    Ok(EncodedCommand {
        opcode: crate::abilities::COMBAT_OPCODE,
        body,
    })
}
