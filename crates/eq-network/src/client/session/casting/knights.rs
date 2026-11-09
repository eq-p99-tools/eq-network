//! Class ability validation and received ability-slot recovery.
use super::{rejected, Casting, Source};
use crate::client::session::{
    feature::{Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::{bail, ensure, Context, Result};
use eq_network_game::{
    knights::{titanium_cast, KnightAbility, ReuseSource},
    targets::TargetState,
    world::{SpawnKind, WorldEvent},
};
use std::time::{Duration, Instant};

/// Titanium `eqgame.exe` 0x411f3d writes the local clock plus 10,000 ms to
/// the slot-9 button timer before the send.
const RETRY_DELAY: Duration = Duration::from_secs(10);

pub(super) struct Reuse {
    estimate: Duration,
    ready: Option<(Instant, ReuseSource)>,
    retry_at: Option<Instant>,
}

impl Reuse {
    pub(super) const fn new(estimate: Duration) -> Self {
        Self {
            estimate,
            ready: None,
            retry_at: None,
        }
    }

    /// Record remaining server reuse at receipt so admission delay cannot
    /// extend the timer.
    pub(super) fn received(&mut self, remaining: Duration, now: Instant) {
        if let Some(ready) = now.checked_add(remaining) {
            self.ready = Some((ready, ReuseSource::Profile));
        }
    }

    fn spend(&mut self, now: Instant) {
        if let (Some(retry_at), Some(ready)) =
            (now.checked_add(RETRY_DELAY), now.checked_add(self.estimate))
        {
            self.retry_at = Some(retry_at);
            self.ready = Some((ready, ReuseSource::LocalEstimate));
        }
    }

    fn check(&self, now: Instant) -> Result<()> {
        ensure!(
            self.ready.is_none_or(|(ready, _)| now >= ready),
            "Ability recovery time not yet met"
        );
        ensure!(
            self.retry_at.is_none_or(|retry| now >= retry),
            "Wait before trying the knight ability again"
        );
        Ok(())
    }
}

/// Validate the settled target revision and current incarnation immediately
/// before sending. A Paladin may select themself. A Shadow Knight cannot use
/// Harm Touch on themself.
fn aim(ability: KnightAbility, target: TargetState, world: &World) -> Result<u16> {
    let player = world.player.as_ref().context("Not in the zone yet")?;
    ensure!(
        KnightAbility::for_player(player.class, player.level) == Some(ability),
        "Your class does not have that ability"
    );
    ensure!(
        !world.control.stunned(),
        "Cannot use that ability while stunned"
    );
    ensure!(
        target.available && target == world.target.state(),
        "Target changed or is not ready"
    );
    let id = target.spawn_id.context("Select a target for that first")?;
    if id == player.spawn_id {
        ensure!(
            ability == KnightAbility::LayOnHands,
            "Harm Touch requires another target"
        );
    } else if !world
        .spawns
        .visible(id)
        .is_some_and(|spawn| matches!(spawn.kind, SpawnKind::Player | SpawnKind::Npc))
    {
        bail!("That target is no longer available for the ability");
    }
    Ok(id)
}

impl Casting {
    /// Publish the current estimate or observation with its provenance.
    pub(super) fn report_knight_reuse(
        &self,
        now: Instant,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let Some((ready, source)) = self.knights.as_ref().and_then(|reuse| reuse.ready) else {
            return Ok(());
        };
        out.log.send(ClientEvent::World(WorldEvent::KnightReuse {
            session_id: world.session_id,
            remaining: ready.saturating_duration_since(now),
            source,
        }))
    }

    /// Send a guarded ability and use the normal cast stream for its result.
    pub(super) fn knight(
        &mut self,
        command: &ClientCommand,
        ability: KnightAbility,
        target: TargetState,
        now: Instant,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let checked = self
            .knights
            .as_ref()
            .context("Knight abilities are unavailable on this server")
            .and_then(|reuse| reuse.check(now))
            .and_then(|()| aim(ability, target, world))
            .and_then(|id| titanium_cast(ability, id));
        match checked {
            Ok(packet) => {
                out.send(&packet)?;
                if let Some(reuse) = &mut self.knights {
                    reuse.spend(now);
                }
                self.report_knight_reuse(now, world, out)?;
                self.submitted(ability.spell(), Source::Knight(ability), world, out)
            }
            Err(error) => {
                let reason = error.to_string();
                if let Some(event) = rejected(command, &reason) {
                    out.log.send(ClientEvent::World(event))?;
                }
                out.log.diagnostic(reason)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::client::session::{
        feature::{testing, Feature},
        targeting::Target,
    };
    use eq_network_game::{message::Message, spells::SpellUpdate};

    #[test]
    fn self_heal_requires_the_right_class_and_settled_target() {
        let mut world = World::new(5);
        let mut player = testing::player(7);
        player.class = Some(3);
        player.level = 1;
        world.player.admit(player);
        world.target = Target::from(7);
        let mut casting = Casting::with_titanium_combat(false, Duration::from_mins(72));
        let command = |target, ability| ClientCommand::UseKnightAbility {
            session_id: 5,
            ability,
            target,
            created: Instant::now(),
        };
        let selected = world.target.state();
        let stale = TargetState {
            revision: 99,
            ..selected
        };
        let heal = command(selected, KnightAbility::LayOnHands);
        for command in [
            command(stale, KnightAbility::LayOnHands),
            command(selected, KnightAbility::HarmTouch),
        ] {
            let output = testing::run(|out| casting.handle(&command, &mut world, out));
            output.result.unwrap();
            assert_eq!(output.sent, []);
            assert!(matches!(
                output.events.first(),
                Some(ClientEvent::World(WorldEvent::CastRejected { .. }))
            ));
        }
        let output = testing::run(|out| casting.handle(&heal, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent.len(), 1);
        assert!(matches!(
            output.events.as_slice(),
            [
                ClientEvent::World(WorldEvent::KnightReuse {
                    source: ReuseSource::LocalEstimate,
                    ..
                }),
                ClientEvent::World(WorldEvent::CastPending {
                    spell_id: Some(87),
                    ..
                })
            ]
        ));

        let result = Message::Event(WorldEvent::Spell(SpellUpdate::Mana {
            spell_id: 87,
            keep_casting: false,
        }));
        testing::run(|out| casting.observe(&result, &mut world, out))
            .result
            .unwrap();
        let duplicate = testing::run(|out| casting.handle(&heal, &mut world, out));
        duplicate.result.unwrap();
        assert_eq!(duplicate.sent, []);
    }

    #[test]
    fn alternative_harm_touch_rank_can_settle_the_hold() {
        let mut guard = super::super::CastGuard::default();
        guard.submitted(
            88,
            Some(Source::Knight(KnightAbility::HarmTouch)),
            Instant::now(),
        );
        guard.observe(
            7,
            &SpellUpdate::Mana {
                spell_id: 2821,
                keep_casting: false,
            },
        );
        assert!(!guard.active());
    }

    #[test]
    fn an_alternate_rank_refresh_uses_the_class_or_eqemu_translated_slot() {
        for (slot, released) in [(8, false), (9, true), (10, true)] {
            let mut guard = super::super::CastGuard::default();
            guard.submitted(
                88,
                Some(Source::Knight(KnightAbility::HarmTouch)),
                Instant::now(),
            );
            guard.observe(
                7,
                &SpellUpdate::BarRefresh {
                    slot,
                    spell_id: 2821,
                    reduction_ms: 0,
                },
            );
            assert_eq!(!guard.active(), released);
        }
    }

    #[test]
    fn profile_timer_and_retry_delay_are_independent() {
        let now = Instant::now();
        let mut reuse = Reuse::new(Duration::from_mins(72));
        assert!(reuse.check(now).is_ok());
        reuse.retry_at = Some(now + RETRY_DELAY);
        reuse.received(Duration::ZERO, now);
        assert!(reuse.check(now).is_err());
        assert!(reuse.check(now + RETRY_DELAY).is_ok());
        reuse.received(Duration::from_secs(30), now);
        assert!(reuse.check(now + RETRY_DELAY).is_err());
        assert!(reuse.check(now + Duration::from_secs(30)).is_ok());
    }

    #[test]
    fn an_overflowing_spend_keeps_both_previous_holds() {
        let now = Instant::now();
        let mut reuse = Reuse::new(Duration::MAX);
        let retry_at = now + RETRY_DELAY;
        let ready = now + Duration::from_secs(30);
        reuse.retry_at = Some(retry_at);
        reuse.received(Duration::from_secs(30), now);

        reuse.spend(now + Duration::from_secs(1));

        assert_eq!(reuse.retry_at, Some(retry_at));
        assert_eq!(reuse.ready, Some((ready, ReuseSource::Profile)));
        assert!(reuse.check(retry_at).is_err());
    }

    #[test]
    fn a_sent_knight_ability_can_be_sent_again_after_the_same_admission_estimate() {
        let now = Instant::now();
        let estimate = Duration::from_mins(72);
        let ability = KnightAbility::LayOnHands;
        let mut world = World::new(5);
        let mut player = testing::player(7);
        player.class = Some(3);
        player.level = 1;
        world.player.admit(player);
        world.target = Target::from(7);
        let target = world.target.state();
        let mut casting = Casting::with_titanium_combat(false, estimate);
        let send = |casting: &mut Casting, world: &World, at| {
            testing::run(|out| {
                casting.knight(
                    &ClientCommand::UseKnightAbility {
                        session_id: 5,
                        ability,
                        target,
                        created: at,
                    },
                    ability,
                    target,
                    at,
                    world,
                    out,
                )
            })
        };
        let first = send(&mut casting, &world, now);
        first.result.unwrap();
        assert_eq!(first.sent, [titanium_cast(ability, 7).unwrap()]);
        assert!(first.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::KnightReuse {
                session_id: 5,
                remaining,
                source: ReuseSource::LocalEstimate,
            }) if *remaining == estimate
        )));
        let result = Message::Event(WorldEvent::Spell(SpellUpdate::Mana {
            spell_id: 87,
            keep_casting: false,
        }));
        testing::run(|out| casting.observe(&result, &mut world, out))
            .result
            .unwrap();
        let before_estimate = estimate.checked_sub(Duration::from_secs(1)).unwrap();
        for later in [RETRY_DELAY, before_estimate] {
            assert_eq!(send(&mut casting, &world, now + later).sent, []);
        }
        let again = send(&mut casting, &world, now + estimate);
        again.result.unwrap();
        assert_eq!(again.sent, [titanium_cast(ability, 7).unwrap()]);
    }

    #[test]
    fn an_authoritative_profile_replaces_the_estimate_but_keeps_the_short_hold() {
        let now = Instant::now();
        let mut reuse = Reuse::new(Duration::from_secs(4300));
        reuse.spend(now);
        reuse.received(Duration::MAX, now);
        assert_eq!(reuse.ready.unwrap().1, ReuseSource::LocalEstimate);
        reuse.received(Duration::from_secs(3), now);
        assert_eq!(reuse.ready.unwrap().1, ReuseSource::Profile);
        assert!(reuse.check(now + Duration::from_secs(3)).is_err());
        assert!(reuse.check(now + RETRY_DELAY).is_ok());
    }
}
