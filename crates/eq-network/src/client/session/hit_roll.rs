//! The Titanium program's own roll when a bash or a kick hits the player:
//! the bash and kick branch of its hit routine 0x41c905 (reads R49-50 and
//! R49-50b, checklists 6.646 and 6.645). The program sends nothing for it;
//! the server rolls its own, as `EQEmu`'s `Mob::CommonDamage` does with the
//! same numbers (`zone/attack.cpp`), so the two rolls are independent.
//!
//! Its stun branch stuns the player in the program's one stun slot
//! ([`super::control::OwnControl`]), as a server stun does. Its interrupt
//! branch is the program's local cast clear (mode 1), which is 6.645's.

use super::{bleeding::ItemCount, feature::World};
use eq_network_game::{
    combat::{Damage, DamageOutcome},
    world::SpawnKind,
};
use rand::Rng as _;

/// The damage kind of a bash (`EQEmu`'s `SkillDamageTypes`), the record's
/// +0xb the program tests.
const BASH: u8 = 10;
/// The damage kind of every kick.
const KICK: u8 = 30;
/// A kick rolls only from a Warrior...
const WARRIOR: u8 = 1;
/// ...above this level.
const KICK_LEVEL: u8 = 54;
/// The races the program never rolls for (profile +0x1198): Ogre, and the
/// Oggok citizen.
const EXEMPT_RACES: [u32; 2] = [10, 93];
/// The stun-bash chance (SPA 0xe7), which lowers a bash's roll.
const STUN_BASH_CHANCE: u16 = 0xe7;
/// The frontal stun resist (SPA 0x125), the first save.
const FRONTAL_STUN_RESIST: u16 = 0x125;
/// The stun resist (SPA 0xc3), the second save.
const STUN_RESIST: u16 = 0xc3;

/// The stun the hit routine asks 0x417839 for (0x41d298).
pub(super) const HIT_STUN: std::time::Duration = std::time::Duration::from_millis(2000);
/// The line when the frontal stun resist saves the player.
pub(super) const AVOIDED_LINE: u32 = 0x1679;
/// The line when the stun resist saves the player.
pub(super) const SHOOK_OFF_LINE: u32 = 0x2375;

/// The program's random numbers (0x40cbb2), inclusive at both ends.
pub(super) trait HitDice: Send {
    /// A number from `low` to `high`, both included.
    fn roll(&mut self, low: i32, high: i32) -> i32;
}

/// The session's dice.
struct ThreadDice;

impl HitDice for ThreadDice {
    fn roll(&mut self, low: i32, high: i32) -> i32 {
        rand::thread_rng().gen_range(low..=high)
    }
}

/// What the roll came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HitRoll {
    /// The interrupt branch, with nothing interrupted (the second roll 1
    /// or less).
    Kept,
    /// The interrupt branch's interrupt (the second roll over 1), for a
    /// cast or a song held outside spells 859 to 1023 (6.645).
    Interrupt,
    /// The stun branch, saved by the frontal stun resist.
    Avoided,
    /// The stun branch, saved by the stun resist.
    ShookOff,
    /// The stun branch's 2000 ms stun.
    Stunned,
}

/// The player's facts the roll reads, from one hit.
struct Facts {
    /// A bash, rather than a kick.
    bash: bool,
    /// The stun-bash chance's total (bash only), and the two saves' totals.
    chance: i32,
    frontal: i32,
    resist: i32,
}

/// The roll's state for one admission.
pub(super) struct HitRoller {
    dice: Box<dyn HitDice>,
    /// The player's current HP in the server's last report, which leaves
    /// out what equipped items add.
    reported: Option<i32>,
    /// How the server type counts what the items add; none yet on
    /// Titanium, so the roll can't tell a hit that leaves the player alive
    /// from one that kills them, and rolls (inferred, the stricter side).
    items: Option<ItemCount>,
}

impl Default for HitRoller {
    fn default() -> Self {
        Self {
            dice: Box::new(ThreadDice),
            reported: None,
            items: None,
        }
    }
}

impl HitRoller {
    /// Rolls with these dice.
    #[cfg(test)]
    pub(super) fn with_dice(dice: impl HitDice + 'static) -> Self {
        Self {
            dice: Box::new(dice),
            ..Self::default()
        }
    }

    /// Counts the items' HP by this rule.
    #[cfg(test)]
    pub(super) const fn with_item_count(mut self, items: ItemCount) -> Self {
        self.items = Some(items);
        self
    }

    /// Keeps the server's last report of the player's HP. On stock `EQEmu`
    /// it comes before the hit's damage record (`Mob::CommonDamage` sends
    /// the HP update first), so the program applies the hit again to it.
    pub(super) const fn reported(&mut self, current: i32) {
        self.reported = Some(current);
    }

    /// The program's roll for a hit, where it rolls: a bash from a player
    /// or a kick from a Warrior above level 54 that leaves the player
    /// alive, a race other than Ogre or Oggok citizen. None otherwise.
    pub(super) fn roll(&mut self, damage: &Damage, world: &World) -> Option<HitRoll> {
        let facts = self.facts(damage, world)?;
        let threshold = 55 - if facts.bash { facts.chance } else { 0 };
        let first = self.dice.roll(0, 100).max(1);
        if first < 99 && first <= threshold {
            return Some(if self.dice.roll(0, 9) > 1 {
                HitRoll::Interrupt
            } else {
                HitRoll::Kept
            });
        }
        if facts.frontal >= self.dice.roll(1, 100) {
            return Some(HitRoll::Avoided);
        }
        // The victim's +0x70 term (param_1[0x1c], R49-50b 2) is untraced
        // and counts 0 (inferred).
        if self.dice.roll(0, 100) < facts.resist {
            return Some(HitRoll::ShookOff);
        }
        Some(HitRoll::Stunned)
    }

    fn facts(&self, damage: &Damage, world: &World) -> Option<Facts> {
        let DamageOutcome::Hit(amount) = damage.outcome else {
            return None;
        };
        if amount == 0 || !world.is_player(damage.target_id) {
            return None;
        }
        let attacker = world.spawns.get(damage.source_id)?;
        let bash = match damage.kind {
            // Spawn +0x218 set: a player the client keeps full data for
            // (R49-50b 1), taken as any player spawn (inferred).
            BASH if attacker.kind == SpawnKind::Player => true,
            KICK if attacker.class == Some(WARRIOR) && attacker.level > KICK_LEVEL => false,
            _ => return None,
        };
        let player = world.player.as_ref()?;
        if EXEMPT_RACES.contains(&player.race) || !self.alive_after(amount, world) {
            return None;
        }
        let sight = &world.sight;
        Some(Facts {
            bash,
            chance: if bash {
                sight.effect_total(STUN_BASH_CHANCE)
            } else {
                0
            },
            frontal: sight.effect_total(FRONTAL_STUN_RESIST),
            resist: sight.effect_total(STUN_RESIST),
        })
    }

    /// Whether the program's HP after the hit is above 0 (0x417e07, read
    /// after the damage applies): the last report with what the items add,
    /// less the damage. Unknown HP, or items the server type can't count,
    /// roll.
    fn alive_after(&self, amount: u32, world: &World) -> bool {
        let Some(reported) = self.reported else {
            return true;
        };
        let Some(Ok(items)) = self.items.map(|count| count(world)) else {
            return true;
        };
        i64::from(reported) + i64::from(items) - i64::from(amount) > 0
    }
}
