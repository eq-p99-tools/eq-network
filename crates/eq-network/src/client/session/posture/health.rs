//! Titanium's received-HP unconscious transition, through the existing
//! posture setter (0x422769, 0x484df5, 0x484d8d). Death is never inferred.
use super::{ClientCommand, ClientEvent, OwnPosture, Posture, World, WorldEvent};
use crate::client::session::feature::Out;
use anyhow::Result;
use eq_network_game::{control::HitPointBasis, message::Message};
use std::time::Instant;

#[derive(Debug, Default)]
pub(super) struct Health {
    reported: Option<(i32, i32)>,
    accepted: Option<HitPointBasis>,
    local_damage: u64,
    unconscious: bool,
    revision: u64,
    changed: Option<Instant>,
}

pub(super) fn note(message: &Message, world: &mut World) {
    if let Message::Event(WorldEvent::HitPoints {
        spawn_id,
        current,
        maximum,
        ..
    }) = message
    {
        if world.is_player(*spawn_id) {
            world.posture.health.reported = Some((*current, *maximum));
            world.posture.health.local_damage = 0;
            world.posture.health.accepted = None;
        }
    }
}

pub(super) fn count(basis: HitPointBasis, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
    let refuse = |reason: &str, out: &mut Out<'_, '_>| out.log.diagnostic(reason.into());
    if !out.wire.dialect().models_unconscious() {
        return refuse(
            "Unconscious HP counts are not established for this generation",
            out,
        );
    }
    if !world.ready() || world.lifecycle.blocks_motion() {
        return refuse(
            "HP control cannot change during death or a zone transfer",
            out,
        );
    }
    if world.posture.health.reported != Some((basis.reported_current, basis.reported_maximum)) {
        return refuse("The HP count does not match the latest own HP report", out);
    }
    let Some(current) = basis.current() else {
        return refuse("The HP count overflows", out);
    };
    if basis.local_damage != world.posture.health.local_damage
        || basis.inventory_revision != world.inventory.revision()
        || !world.inventory.received()
        || world.inventory.stale()
        || world.inventory.equipment_stats_uncertain()
    {
        return refuse("The HP count predates local damage or equipment", out);
    }
    let Some(spawn_id) = world.own_spawn else {
        return refuse("The zone has not named the player's spawn", out);
    };
    let unconscious = current < 1;
    let changed = world.posture.health.unconscious != unconscious;
    if changed {
        if unconscious {
            // Same native posture routine as feign, without a feign request,
            // flag, success inference or any move-while-feigned departure.
            world.posture.lie_for_feign(spawn_id, out)?;
            world.body.suspend();
        } else {
            // Release only this hold. Lifecycle was checked above; a late
            // heal can never resume a dead or departing player.
            world.posture.health.unconscious = false;
            world.posture.set(spawn_id, Posture::Standing, out)?;
            world.body.resume(Instant::now());
        }
        let health = &mut world.posture.health;
        health.unconscious = unconscious;
        health.revision += 1;
        health.changed = Some(Instant::now());
    }
    world.posture.health.accepted = Some(basis);
    out.log.send(ClientEvent::World(WorldEvent::OwnUnconscious {
        session_id: world.session_id,
        revision: world.posture.health.revision,
        unconscious,
        basis,
    }))
}

/// Only a completed local environmental submission gets the program's local
/// HP subtraction (0x41c905). Failed sends and received damage echoes do not.
pub(in crate::client::session) fn local_damage(
    amount: u32,
    world: &mut World,
    out: &mut Out<'_, '_>,
) -> Result<()> {
    if !out.wire.dialect().models_unconscious() || world.lifecycle.blocks_motion() {
        return Ok(());
    }
    world.posture.health.local_damage = world
        .posture
        .health
        .local_damage
        .saturating_add(u64::from(amount));
    if let Some(mut basis) = world.posture.health.accepted {
        basis.local_damage = world.posture.health.local_damage;
        count(basis, world, out)?;
    }
    Ok(())
}

/// A spell/equipment input change withdraws a previously known effect-328
/// cutoff until the host resolves the new inputs. It never wakes the player.
pub(super) fn invalidate_effect_floor(
    message: &Message,
    world: &mut World,
    out: &mut Out<'_, '_>,
) -> Result<()> {
    let affects = match message {
        Message::Event(WorldEvent::Buff(buff)) => world.is_player(buff.entity_id),
        Message::Event(WorldEvent::SpellEffect(effect)) => world.is_player(effect.target_id),
        Message::Event(WorldEvent::AlternateAdvancement { session_id, .. }) => {
            *session_id == world.session_id
        }
        Message::Event(
            WorldEvent::BuffSnapshot(_) | WorldEvent::Inventory(_) | WorldEvent::Level { .. },
        ) => true,
        _ => false,
    };
    if affects {
        if let Some(mut basis) = world
            .posture
            .health
            .accepted
            .filter(|basis| basis.damage_floor.is_some())
        {
            basis.damage_floor = None;
            world.posture.health.accepted = Some(basis);
            out.log
                .send(ClientEvent::World(WorldEvent::OwnUnconscious {
                    session_id: world.session_id,
                    revision: world.posture.health.revision,
                    unconscious: world.posture.health.unconscious,
                    basis,
                }))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::session::{
        feature::{testing, Feature},
        posture::Stances,
        wire,
    };
    use eq_network_game::{
        command,
        movement::{HeadingUpdate, MovementMode, MovementRequest},
        world::{ItemHitPoints, Position, PostureState},
    };

    fn world() -> World {
        let mut world = World::new(5);
        world.player.admit(testing::player(7));
        world.own_spawn = Some(7);
        world.admitted = Some(Instant::now());
        let mut inventory = eq_network_game::inventory::Inventory::default();
        inventory.apply(eq_network_game::inventory::InventoryUpdate::Snapshot(
            Vec::new(),
        ));
        world.inventory = inventory.into();
        world
    }

    fn report(world: &mut World, current: i32) {
        testing::run(|out| {
            Stances.observe(
                &Message::Event(WorldEvent::HitPoints {
                    spawn_id: 7,
                    current,
                    maximum: 100,
                    items: ItemHitPoints::LeftOut,
                }),
                world,
                out,
            )
        })
        .result
        .unwrap();
    }

    fn basis(current: i32, items: i64) -> HitPointBasis {
        HitPointBasis {
            reported_current: current,
            reported_maximum: 100,
            item_hit_points: items,
            inventory_revision: 1,
            local_damage: 0,
            damage_floor: Some(-10),
        }
    }

    #[test]
    fn received_hp_with_items_lies_once_wakes_once_and_never_infers_death() {
        let mut world = world();
        report(&mut world, -20);
        let alive = testing::run(|out| count(basis(-20, 25), &mut world, out));
        alive.result.unwrap();
        assert!(alive.sent.is_empty());
        assert!(!world.posture.unconscious());
        for current in [0, -1, -20] {
            report(&mut world, current);
            let output = testing::run(|out| count(basis(current, 0), &mut world, out));
            output.result.unwrap();
            if current == 0 {
                assert_eq!(
                    output.sent,
                    [eq_network_game::feign::titanium_posture(7).unwrap()]
                );
            } else {
                assert!(output.sent.is_empty());
            }
            assert!(output.unreliable.is_empty());
            assert!(world.posture.unconscious());
            assert_eq!(world.posture.current(), Some(PostureState::Lying));
            assert!(!world.lifecycle.is_dead());
        }
        let old = Instant::now();
        report(&mut world, 1);
        let wake = testing::run(|out| count(basis(1, 0), &mut world, out));
        wake.result.unwrap();
        assert_eq!(
            wake.sent,
            [command::titanium_posture(7, Posture::Standing).unwrap()]
        );
        assert!(!world.posture.unconscious());
        assert_eq!(world.posture.current(), Some(PostureState::Standing));
        let old_move = ClientCommand::Move(MovementRequest {
            session_id: 5,
            mode: MovementMode::Swim,
            heading: HeadingUpdate::Rate,
            position: Position::default(),
            support: None,
            vertical_speed: None,
            history: None,
            created: old,
        });
        assert!(world.posture.input_refusal(&old_move).is_some());
        let again = testing::run(|out| count(basis(1, 0), &mut world, out));
        again.result.unwrap();
        assert!(again.sent.is_empty());
        assert!(!world.lifecycle.is_dead());
    }

    #[test]
    fn stale_report_other_generation_death_and_transfer_cannot_change_control() {
        let mut world = world();
        report(&mut world, 1);
        let stale = testing::run(|out| count(basis(0, 0), &mut world, out));
        stale.result.unwrap();
        assert!(stale.sent.is_empty());
        assert!(!world.posture.unconscious());
        let other = testing::run_on(&wire::EqMac, |out| count(basis(1, -1), &mut world, out));
        other.result.unwrap();
        assert!(other.sent.is_empty());
        assert!(!world.posture.unconscious());
        report(&mut world, 0);
        testing::run(|out| count(basis(0, 0), &mut world, out))
            .result
            .unwrap();
        world
            .lifecycle
            .offer(
                eq_network_game::zoning::ZoneOffer {
                    zone_id: 2,
                    instance_id: 0,
                    position: Position::default(),
                    reason: 0,
                    to_bind: false,
                    solicited: true,
                },
                Instant::now(),
            )
            .unwrap();
        report(&mut world, 50);
        let transferring = testing::run(|out| count(basis(50, 0), &mut world, out));
        transferring.result.unwrap();
        assert!(transferring.sent.is_empty());
        assert!(world.posture.unconscious());
        world.lifecycle.finish(false, Instant::now()).unwrap();
        assert!(
            world.motion_held(),
            "a denied transfer does not wake the player"
        );
        world.lifecycle.mark_dead();
        report(&mut world, 50);
        let late = testing::run(|out| count(basis(50, 0), &mut world, out));
        late.result.unwrap();
        assert!(late.sent.is_empty());
        assert!(world.lifecycle.is_dead());
        assert!(world.posture.unconscious());
        assert!(World::new(6)
            .posture
            .input_refusal(&ClientCommand::SetPosture {
                session_id: 6,
                spawn_id: 7,
                posture: Posture::Standing,
                created: Instant::now(),
            })
            .is_none());
    }
}

impl OwnPosture {
    pub(in crate::client::session) fn damage_record_allowed(
        &self,
        amount: u32,
        inventory_revision: u64,
    ) -> Option<bool> {
        let basis = self
            .health
            .accepted
            .filter(|basis| basis.inventory_revision == inventory_revision)?;
        Some(basis.current()?.saturating_sub(i64::from(amount)) >= basis.damage_floor?)
    }

    /// A known cutoff never asserts death. Unknown effect totals and stale
    /// equipment do not invent one.
    pub(in crate::client::session) fn damage_held(&self, inventory_revision: u64) -> bool {
        self.health
            .accepted
            .filter(|basis| basis.inventory_revision == inventory_revision)
            .is_some_and(|basis| {
                basis
                    .current()
                    .zip(basis.damage_floor)
                    .is_some_and(|(current, floor)| current < floor)
            })
    }

    pub(in crate::client::session) const fn unconscious(&self) -> bool {
        self.health.unconscious
    }

    /// Input made before either transition cannot replay after waking.
    /// Environmental ticks, chat, cleanup and settings are not voluntary
    /// movement and do not acquire this hold.
    pub(in crate::client::session) fn input_refusal(
        &self,
        command: &ClientCommand,
    ) -> Option<&'static str> {
        if !matches!(
            command,
            ClientCommand::Move(_)
                | ClientCommand::GroundFrame { .. }
                | ClientCommand::Jump { .. }
                | ClientCommand::SetPosture { .. }
                | ClientCommand::FeignLanded { .. }
        ) {
            return None;
        }
        if self
            .health
            .changed
            .is_some_and(|changed| command.created().is_some_and(|created| created <= changed))
        {
            Some("Player input predates the current unconscious state")
        } else if self.unconscious() {
            Some("The player is unconscious")
        } else {
            None
        }
    }
}
