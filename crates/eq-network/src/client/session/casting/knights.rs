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

/// This library's words for the slot-9 wait, beside the program's line.
const GEM_WAITING: &str = "Wait before trying the knight ability again";

/// The program's refusal while the slot-9 timer runs (R49-29 b, colour
/// 0x10e).
const GEM_WAIT_LINE: u32 = 0x2efd;

/// A knight special's two waits (read R49-29 b).
pub(super) struct Reuse {
    estimate: Duration,
    /// The server's long reuse, as the profile last gave it or as estimated
    /// at a send. Shown to the host; a gate only where the host's installed
    /// data has no recast for the special (today's hold, labelled).
    ready: Option<(Instant, ReuseSource)>,
    /// The program's slot-9 timer (actor +0x10c): 10 s from a send, and the
    /// spell's recast from the server's refresh of gem 9. Actor memory, so a
    /// new zone session starts without it.
    gem: Option<Instant>,
    /// The special last sent, for a refresh whose spell the host's data
    /// doesn't know.
    sent: Option<KnightAbility>,
}

impl Reuse {
    pub(super) const fn new(estimate: Duration) -> Self {
        Self {
            estimate,
            ready: None,
            gem: None,
            sent: None,
        }
    }

    /// Record remaining server reuse at receipt so admission delay cannot
    /// extend the timer.
    pub(super) fn received(&mut self, remaining: Duration, now: Instant) {
        if let Some(ready) = now.checked_add(remaining) {
            self.ready = Some((ready, ReuseSource::Profile));
        }
    }

    /// The server's `OP_MemorizeSpell` kind 3 for gem 9 (0x458ed8): the
    /// timer runs the spell's recast from now. The program takes 1 s off
    /// for each point of the player's effect 0x127, which isn't totalled
    /// here, so the wait is never shorter than the program's (inferred,
    /// 6.626). A spell the host's data doesn't know takes the sent
    /// special's recast; with neither, the timer stays as it was.
    pub(super) fn refreshed(
        &mut self,
        spell_id: u32,
        recast: impl Fn(u32) -> Option<Duration>,
        now: Instant,
    ) {
        let recast = recast(spell_id).or_else(|| recast(self.sent?.spell()));
        if let Some(until) = recast.and_then(|recast| now.checked_add(recast)) {
            self.gem = Some(until);
        }
    }

    fn spend(&mut self, ability: KnightAbility, now: Instant) {
        if let (Some(gem), Some(ready)) =
            (now.checked_add(RETRY_DELAY), now.checked_add(self.estimate))
        {
            self.gem = Some(gem);
            self.ready = Some((ready, ReuseSource::LocalEstimate));
        }
        self.sent = Some(ability);
    }

    /// Whether the program's slot-9 timer still runs.
    fn waiting(&self, now: Instant) -> bool {
        self.gem.is_some_and(|gem| now < gem)
    }

    /// The long hold, kept only where the host's data has no recast for
    /// the special: the press waits out the server's reuse in this
    /// library's words, as before (6.626).
    fn check(&self, now: Instant, recast_known: bool) -> Result<()> {
        ensure!(
            recast_known || self.ready.is_none_or(|(ready, _)| now >= ready),
            "Ability recovery time not yet met"
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
        if let Some(string_id) = super::lying_refusal(world, out) {
            if let Some(WorldEvent::CastRejected {
                session_id,
                spell_id,
                ..
            }) = rejected(command, super::LYING)
            {
                out.log.send(ClientEvent::World(WorldEvent::CastRejected {
                    session_id,
                    spell_id,
                    reason: super::LYING.into(),
                    string_id: Some(string_id),
                }))?;
            }
            return out
                .log
                .diagnostic("Refused a knight's special: the player lies".into());
        }
        // The program's slot-9 timer: while it runs, its line and nothing
        // sent (R49-29 b). The server still refuses a press its own long
        // reuse holds.
        if self
            .knights
            .as_ref()
            .is_some_and(|reuse| reuse.waiting(now))
        {
            if let Some(WorldEvent::CastRejected {
                session_id,
                spell_id,
                ..
            }) = rejected(command, GEM_WAITING)
            {
                out.log.send(ClientEvent::World(WorldEvent::CastRejected {
                    session_id,
                    spell_id,
                    reason: GEM_WAITING.into(),
                    string_id: Some(GEM_WAIT_LINE),
                }))?;
            }
            return out.log.diagnostic(GEM_WAITING.into());
        }
        let recast_known = world.sight.recast(ability.spell()).is_some();
        let checked = self
            .knights
            .as_ref()
            .context("Knight abilities are unavailable on this server")
            .and_then(|reuse| reuse.check(now, recast_known))
            .and_then(|()| aim(ability, target, world))
            .and_then(|id| titanium_cast(ability, id));
        match checked {
            Ok(packet) => {
                out.send(&packet)?;
                if let Some(reuse) = &mut self.knights {
                    reuse.spend(ability, now);
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
    fn titanium_refuses_a_knights_special_while_the_player_lies() {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        player.class = Some(3);
        player.level = 1;
        world.player.admit(player);
        world.target = Target::from(7);
        world.posture.observed(crate::world::PostureState::Lying);
        let mut casting = Casting::with_titanium_combat(false, Duration::from_mins(72));
        let heal = ClientCommand::UseKnightAbility {
            session_id: 5,
            ability: KnightAbility::LayOnHands,
            target: world.target.state(),
            created: Instant::now(),
        };
        let output = testing::run(|out| casting.handle(&heal, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent, []);
        assert!(matches!(
            output.events.as_slice(),
            [
                ClientEvent::World(WorldEvent::CastRejected {
                    spell_id: 87,
                    string_id: Some(0x3099),
                    ..
                }),
                ClientEvent::Diagnostic(_)
            ]
        ));
        // Nothing was spent: the reuse timer still allows the special.
        world.posture.observed(crate::world::PostureState::Standing);
        let output = testing::run(|out| casting.handle(&heal, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent.len(), 1);
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
    fn without_an_installed_recast_the_profile_timer_still_holds_the_press() {
        let now = Instant::now();
        let mut reuse = Reuse::new(Duration::from_mins(72));
        assert!(reuse.check(now, false).is_ok());
        reuse.received(Duration::from_secs(30), now);
        assert!(reuse.check(now + RETRY_DELAY, false).is_err());
        assert!(reuse.check(now + Duration::from_secs(30), false).is_ok());
        // With the installed recast the long timer is only shown.
        assert!(reuse.check(now, true).is_ok());
    }

    #[test]
    fn an_overflowing_spend_keeps_both_previous_holds() {
        let now = Instant::now();
        let mut reuse = Reuse::new(Duration::MAX);
        let gem = now + RETRY_DELAY;
        let ready = now + Duration::from_secs(30);
        reuse.gem = Some(gem);
        reuse.received(Duration::from_secs(30), now);

        reuse.spend(KnightAbility::LayOnHands, now + Duration::from_secs(1));

        assert_eq!(reuse.gem, Some(gem));
        assert_eq!(reuse.ready, Some((ready, ReuseSource::Profile)));
        assert!(reuse.waiting(now));
        assert!(!reuse.waiting(gem));
    }

    #[test]
    fn the_gem_refresh_waits_the_spells_recast_or_the_sent_specials() {
        let now = Instant::now();
        let recast = |spell| (spell == 87).then_some(Duration::from_secs(30));
        let mut reuse = Reuse::new(Duration::from_mins(72));
        reuse.spend(KnightAbility::LayOnHands, now);
        let later = now + Duration::from_secs(2);
        reuse.refreshed(87, recast, later);
        assert_eq!(reuse.gem, Some(later + Duration::from_secs(30)));
        // A rank the data doesn't know takes the sent special's recast.
        reuse.refreshed(2821, recast, now);
        assert_eq!(reuse.gem, Some(now + Duration::from_secs(30)));
        // With no recast at all the timer stays.
        let mut unknown = Reuse::new(Duration::from_mins(72));
        unknown.spend(KnightAbility::HarmTouch, now);
        unknown.refreshed(88, recast, later);
        assert_eq!(unknown.gem, Some(now + RETRY_DELAY));
    }

    #[derive(Debug)]
    struct Recasts;

    impl eq_network_game::vision::SightCatalog for Recasts {
        fn sees_invisible(&self, _: u32, _: u16, _: bool) -> bool {
            false
        }

        fn recast(&self, spell_id: u32) -> Option<Duration> {
            matches!(spell_id, 87 | 88).then_some(Duration::from_secs(30))
        }
    }

    #[test]
    fn with_the_installed_recast_the_press_waits_the_gem_and_not_the_server() {
        let now = Instant::now();
        let ability = KnightAbility::LayOnHands;
        let mut world = World::new(5);
        world.sight = crate::client::session::sight::Sight::new(Some(std::sync::Arc::new(Recasts)));
        let mut player = testing::player(7);
        player.class = Some(3);
        player.level = 1;
        world.player.admit(player);
        world.target = Target::from(7);
        let target = world.target.state();
        let mut casting = Casting::with_titanium_combat(false, Duration::from_mins(72));
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
        assert_eq!(send(&mut casting, &world, now).sent.len(), 1);
        let result = Message::Event(WorldEvent::Spell(SpellUpdate::Mana {
            spell_id: 87,
            keep_casting: false,
        }));
        testing::run(|out| casting.observe(&result, &mut world, out))
            .result
            .unwrap();
        // Inside the 10 s from the send: the program's line, nothing sent.
        let early = send(&mut casting, &world, now + Duration::from_secs(5));
        early.result.unwrap();
        assert_eq!(early.sent, []);
        assert!(early.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::CastRejected {
                spell_id: 87,
                string_id: Some(0x2efd),
                ..
            })
        )));
        // The server's refresh of gem 9 starts the 30 s recast.
        let refreshed = now + Duration::from_secs(3);
        casting.knights.as_mut().unwrap().refreshed(
            87,
            |spell| world.sight.recast(spell),
            refreshed,
        );
        let waiting = send(&mut casting, &world, refreshed + Duration::from_secs(29));
        assert_eq!(waiting.sent, []);
        // After it the press goes, though the server's long reuse runs: the
        // server is left to refuse it, as the program leaves it.
        let again = send(&mut casting, &world, refreshed + Duration::from_secs(30));
        again.result.unwrap();
        assert_eq!(again.sent, [titanium_cast(ability, 7).unwrap()]);
        // A new zone session's casting starts with no wait.
        let mut fresh = Casting::with_titanium_combat(false, Duration::from_mins(72));
        assert_eq!(
            send(&mut fresh, &world, refreshed + Duration::from_secs(31))
                .sent
                .len(),
            1
        );
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
        reuse.spend(KnightAbility::LayOnHands, now);
        reuse.received(Duration::MAX, now);
        assert_eq!(reuse.ready.unwrap().1, ReuseSource::LocalEstimate);
        reuse.received(Duration::from_secs(3), now);
        assert_eq!(reuse.ready.unwrap().1, ReuseSource::Profile);
        assert!(reuse.check(now + Duration::from_secs(3), false).is_ok());
        assert!(reuse.waiting(now + Duration::from_secs(3)));
        assert!(!reuse.waiting(now + RETRY_DELAY));
    }
}
