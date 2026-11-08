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
    pub(crate) const fn from_wire(dialect: GameDialect, value: u32) -> Self {
        match dialect {
            GameDialect::Titanium => Self::Titanium(value),
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
}
