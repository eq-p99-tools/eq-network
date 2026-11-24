//! The actor's received appearance and the observer's ordinary See Invisible effect.
use crate::{
    world::{SpawnKind, SpawnState},
    GameDialect,
};
use serde::Serialize;

/// Raw appearance kind 3, with the interpreting client generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Invisibility {
    /// Titanium's parameter. Values other than 0 and 1 are not classified here.
    Titanium(u32),
    /// Mac-era parameter. Observer-dependent visibility is not projected for it.
    EqMac(u32),
}

impl Invisibility {
    /// The parameter as `dialect` reads it. `RoF2` reads as Titanium's,
    /// inferred (6.518): stock `EQEmu` sends both the same values
    /// (`Mob::SetInvisible` through `Mob::SendAppearancePacket`, which
    /// `ENCODE(OP_SpawnAppearance)` passes through), and only the Titanium
    /// program's reading of them is known.
    pub(crate) const fn from_wire(dialect: GameDialect, value: u32) -> Self {
        match dialect {
            GameDialect::Titanium | GameDialect::Rof2 => Self::Titanium(value),
            GameDialect::EqMac => Self::EqMac(value),
        }
    }

    /// Records the actor's appearance independently of any observer.
    pub const fn apply(self, spawn: &mut SpawnState) {
        match self {
            Self::Titanium(value) => {
                spawn.invisible = value != 0;
                spawn.appearance.titanium_invisibility = Some(value);
            }
            Self::EqMac(value) => {
                spawn.invisible = value != 0;
                spawn.appearance.titanium_invisibility = None;
            }
        }
    }
}

/// The invisibility part of eligibility, for a spawn the client already knows.
/// Titanium eqgame.exe 0x47e7ee reads the actor's ordinary flag and checks
/// the observer's positive SPA 13 (0x41d3c6). The handler at 0x45242f also
/// checks SPA 13 before clearing a selected actor on kind 3. The separate
/// GM gate and unclassified nonzero values stay hidden in this projection.
/// Distance, ray, entity kind and identity checks still belong to the caller.
#[must_use]
pub const fn can_see(spawn: &SpawnState, sees_invisible: bool) -> bool {
    !spawn.invisible
        || (sees_invisible
            && matches!(spawn.kind, SpawnKind::Player | SpawnKind::Npc)
            && matches!(spawn.appearance.titanium_invisibility, Some(1))
            && matches!(spawn.appearance.titanium_gm, Some(0)))
}

/// Look up the See Invisible property of an observed buff using the host's
/// installed and server-selected spell definitions. A landing has an icon
/// observation but no slot, so the catalog must also resolve it as lasting.
pub trait SightCatalog: std::fmt::Debug + Send + Sync {
    /// Whether the known application grants positive Titanium SPA 13.
    fn sees_invisible(&self, spell_id: u32, caster_level: u16, landed: bool) -> bool;

    /// Whether the known application grants a positive Titanium SPA 40
    /// (divine aura), under which the program ignores a stun (R49-6b:
    /// 0x417839 tests effect 40's total). False, the default, where the
    /// host does not know.
    fn grants_divine_aura(&self, _spell_id: u32, _caster_level: u16, _landed: bool) -> bool {
        false
    }

    /// The total of a Titanium SPA's values that the known application
    /// grants, as the program's buff sum (0x417b9c) counts them: the hit
    /// roll's stun-bash chance (0xe7) and its two stun saves (0x125, 0xc3;
    /// read R49-50b). 0, the default, where the host does not know.
    fn effect_total(&self, _spell_id: u32, _caster_level: u16, _landed: bool, _spa: u16) -> i32 {
        0
    }

    /// The spell's recast time from the installed spell data, which the
    /// Titanium program's knight specials wait out locally after the
    /// server refreshes their gem (R49-29 b: the spell's +0x1c). `None`,
    /// the default, where the host does not know.
    fn recast(&self, _spell_id: u32) -> Option<std::time::Duration> {
        None
    }
}
