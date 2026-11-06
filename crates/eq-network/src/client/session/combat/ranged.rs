use super::{ClientCommand, World};
use crate::client::session::feature::Out;
use anyhow::Result;
use eq_network_game::{
    abilities::player_size,
    combat::ranged::{RangedAttack, RangedLoadout},
    request::Request,
    world::{Position, PostureState, SpawnKind},
};
use std::time::Instant;

/// How the server profile admits a skill to the ranged action.
pub(super) enum SkillAdmission {
    /// Stock `EQEmu` checks the class and level cap, which the profile does
    /// not carry. Its raw rank can be zero for an eligible first use.
    ServerCap,
    /// P99's deployed first-use behavior is unverified. Keep the local
    /// positive-rank prerequisite for ordinary P99 skill values.
    ReportedRank,
}

/// The local unmodified wait between manually submitted ranged attacks.
/// The server's attack and ranged timers remain independent.
pub(super) struct Shots {
    ready_at: Option<Instant>,
    skill_admission: SkillAdmission,
}

impl Shots {
    pub(super) const fn new(skill_admission: SkillAdmission) -> Self {
        Self {
            ready_at: None,
            skill_admission,
        }
    }

    pub(super) fn reset(&mut self) {
        self.ready_at = None;
    }

    /// Submit a shot using the target and inventory the session still holds.
    pub(super) fn send(
        &mut self,
        attack: RangedAttack,
        command: &ClientCommand,
        world: &World,
        out: &mut Out<'_, '_>,
        now: Instant,
    ) -> Result<bool> {
        let target_id = match self.aim(attack, world, now) {
            Ok(id) => id,
            Err(reason) => {
                crate::client::session::actions::refuse(command, reason, out.log)?;
                return Ok(false);
            }
        };
        out.request(&Request::RangedAttack {
            target_id,
            kind: attack.loadout.kind(),
        })?;
        self.ready_at = Some(now + attack.loadout.base_delay());
        Ok(true)
    }

    fn aim(&self, attack: RangedAttack, world: &World, now: Instant) -> Result<u16, &'static str> {
        let Some((own_id, position)) = world.player_at() else {
            return Err("Enter the zone before attacking");
        };
        let target_state = world.target.state();
        let Some(target_id) = target_state.spawn_id else {
            return Err("Select a ranged attack target first");
        };
        if !target_state.available || target_state != attack.target {
            return Err("The ranged attack target has changed");
        }
        let target = world
            .spawns
            .visible(target_id)
            .filter(|target| {
                target_id != own_id && matches!(target.kind, SpawnKind::Npc | SpawnKind::Player)
            })
            .ok_or("That spawn is not a ranged attack target")?;
        if world.control.stunned()
            || world
                .posture
                .current()
                .is_some_and(|posture| posture != PostureState::Standing)
        {
            return Err("You cannot make a ranged attack in your current state");
        }
        let loadout = RangedLoadout::current(&world.inventory)?;
        if loadout != attack.loadout {
            return Err("The ranged equipment or ammunition has changed");
        }
        let player = world
            .player
            .as_ref()
            .ok_or("Enter the zone before attacking")?;
        if matches!(self.skill_admission, SkillAdmission::ReportedRank) {
            let rank = player
                .skills
                .as_deref()
                .and_then(|skills| skills.get(usize::from(loadout.kind().skill())))
                .copied();
            match rank {
                Some(1..=252) => (),
                Some(0) => return Err("A positive ranged skill rank is required on this profile"),
                // A missing or outlying value is not evidence of a learned
                // skill. Do not assign `EQMac` sentinel meanings to P99.
                _ => return Err("The ranged skill rank is unavailable"),
            }
        }
        let size = world
            .spawns
            .all()
            .find(|spawn| spawn.spawn_id == own_id && spawn.size > 0.0)
            .map_or_else(|| player_size(player.race), |spawn| spawn.size);
        check_range(position, target.position, size.max(target.size), loadout)?;
        if self.ready_at.is_some_and(|ready| now < ready) {
            return Err("Your ranged weapon is still recovering");
        }
        Ok(target_id)
    }
}

/// Titanium's native minimum is horizontal. The maximum is checked against
/// `EQEmu`'s three-dimensional item and size bound. The server can apply its
/// own rules and sees positions at a different time.
fn check_range(
    from: Position,
    to: Position,
    size: f32,
    loadout: RangedLoadout,
) -> Result<(), &'static str> {
    let dx = from.x - to.x;
    let dy = from.y - to.y;
    let dz = from.z - to.z;
    let horizontal = dx.mul_add(dx, dy * dy);
    let distance = dz.mul_add(dz, horizontal);
    let size_allowance = if size < 10.0 {
        20.0
    } else if size < 15.0 {
        22.0 + 4.0 * (size - 10.0)
    } else if size <= 20.0 {
        44.0 + 5.8 * (size - 15.0)
    } else {
        77.0
    };
    let reach = f32::from(loadout.item_range()) + size_allowance;
    if !distance.is_finite() || distance > reach * reach {
        return Err("The target is out of ranged attack reach");
    }
    if horizontal < 30.0 * 30.0 {
        return Err("The target is too close for a ranged attack");
    }
    Ok(())
}
