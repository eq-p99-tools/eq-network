//! Class ability validation and received ability-slot recovery.
use super::{rejected, Casting, Source};
use crate::client::session::{
    feature::{Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::{bail, ensure, Context, Result};
use eq_network_game::{
    knights::{titanium_cast, KnightAbility},
    targets::TargetState,
    world::{SpawnKind, WorldEvent},
};
use std::time::{Duration, Instant};

/// Titanium holds the button for ten seconds after a send. This local retry
/// guard is separate from the server's class ability recovery timer.
const RETRY_DELAY: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(super) struct Reuse {
    profile_ready: Option<Instant>,
    retry_at: Option<Instant>,
}

impl Reuse {
    /// Record remaining server reuse at receipt so admission delay cannot
    /// extend the timer.
    pub(super) fn received(&mut self, remaining: Duration, now: Instant) {
        self.profile_ready = Some(now + remaining);
    }

    fn check(&self, now: Instant) -> Result<()> {
        if let Some(wait) = self
            .profile_ready
            .and_then(|ready| ready.checked_duration_since(now))
        {
            ensure!(wait.is_zero(), "Ability recovery time not yet met");
        }
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
    /// Publish received reuse for the new admission or a later profile update.
    pub(super) fn report_knight_reuse(&self, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        let Some(ready) = self.knights.as_ref().and_then(|reuse| reuse.profile_ready) else {
            return Ok(());
        };
        out.log.send(ClientEvent::World(WorldEvent::KnightReuse {
            session_id: world.session_id,
            remaining: ready.saturating_duration_since(Instant::now()),
        }))
    }

    /// Send a guarded ability and use the normal cast stream for its result.
    pub(super) fn knight(
        &mut self,
        command: &ClientCommand,
        ability: KnightAbility,
        target: TargetState,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let now = Instant::now();
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
                    reuse.retry_at = Some(now + RETRY_DELAY);
                }
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
        let mut casting = Casting::with_knights(false);
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
            assert!(output.sent.is_empty());
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
            [ClientEvent::World(WorldEvent::CastPending {
                spell_id: Some(87),
                ..
            })]
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
        assert!(duplicate.sent.is_empty());
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
    fn profile_timer_and_retry_delay_are_independent() {
        let now = Instant::now();
        let mut reuse = Reuse::default();
        assert!(reuse.check(now).is_ok());
        reuse.retry_at = Some(now + RETRY_DELAY);
        reuse.received(Duration::ZERO, now);
        assert!(reuse.check(now).is_err());
        assert!(reuse.check(now + RETRY_DELAY).is_ok());
        reuse.received(Duration::from_secs(30), now);
        assert!(reuse.check(now + RETRY_DELAY).is_err());
        assert!(reuse.check(now + Duration::from_secs(30)).is_ok());
    }
}
