//! Casting: spells from the memorized gems and items' click effects, each held
//! until the server answers it.
use super::{
    actions::Resource,
    feature::{Encoder, Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::{Context, Result};
use eq_network_game::{
    inventory::ItemUse, message::Message, spells::SpellUpdate, world::WorldEvent,
};
use std::time::{Duration, Instant};

/// Local retry policy, not a protocol timeout. Never resends a request automatically.
const ACK_TIMEOUT: Duration = Duration::from_secs(5);

/// Reports only cast rejections, preserving the caller's admission for host filtering.
pub(super) fn rejected(command: &ClientCommand, reason: &str) -> Option<crate::world::WorldEvent> {
    let ClientCommand::CastSpell {
        session_id,
        spell_id,
        ..
    } = command
    else {
        return None;
    };
    Some(crate::world::WorldEvent::CastRejected {
        session_id: *session_id,
        spell_id: *spell_id,
        reason: reason.into(),
    })
}

enum Phase {
    Awaiting { spell: u32, submitted: Instant },
    Casting { spell: u32 },
}

#[derive(Default)]
pub(super) struct CastGuard {
    phase: Option<Phase>,
}

impl CastGuard {
    /// Whether the shared action table refuses this command while a cast is held;
    /// movement and posture interruption stay available.
    #[cfg(test)]
    pub fn blocks(&self, command: &ClientCommand) -> bool {
        self.active() && super::actions::needs(command).contains(&super::actions::Resource::Casting)
    }

    /// A duration reaching zero does not authorize another cast: await a server result.
    pub fn active(&self) -> bool {
        self.phase.is_some()
    }

    /// What a cast in flight holds.
    pub fn hold(&self) -> Option<(Resource, &'static str)> {
        self.active().then_some((
            Resource::Casting,
            "Wait for the current cast to finish or interrupt it",
        ))
    }

    /// Records a request only after successful transport submission.
    pub fn submitted(&mut self, spell: u32, now: Instant) {
        self.phase = Some(Phase::Awaiting {
            spell,
            submitted: now,
        });
    }

    /// The unacknowledged request, distinct from a server-confirmed cast.
    pub fn pending(&self) -> Option<u32> {
        match self.phase {
            Some(Phase::Awaiting { spell, .. }) => Some(spell),
            _ => None,
        }
    }

    /// Allows a fresh user attempt after silence; confirmed casts never expire by time.
    pub fn expire(&mut self, now: Instant) -> bool {
        if matches!(self.phase, Some(Phase::Awaiting { submitted, .. })
            if now.checked_duration_since(submitted).is_some_and(|age| age >= ACK_TIMEOUT))
        {
            self.clear();
            true
        } else {
            false
        }
    }

    fn spell(&self) -> Option<u32> {
        match self.phase {
            Some(Phase::Awaiting { spell, .. } | Phase::Casting { spell }) => Some(spell),
            None => None,
        }
    }

    /// Clears state when this character dies or the owning zone session ends.
    pub fn clear(&mut self) {
        self.phase = None;
    }

    /// Only own-caster starts/interruptions and matching result notifications affect us.
    pub fn observe(&mut self, own_id: u16, update: &SpellUpdate) {
        match *update {
            SpellUpdate::Began {
                caster_id,
                spell_id,
                ..
            } if caster_id == own_id => {
                self.phase = Some(Phase::Casting {
                    spell: u32::from(spell_id),
                });
            }
            SpellUpdate::Interrupted { caster_id, .. } if caster_id == u32::from(own_id) => {
                self.clear();
            }
            SpellUpdate::Mana {
                spell_id,
                keep_casting: false,
            }
            | SpellUpdate::BarRefresh { spell_id, .. }
                if self.spell() == Some(spell_id) =>
            {
                self.clear();
            }
            _ => (),
        }
    }
}

/// Casts spells and item effects for the player, one at a time.
pub(super) struct Casting {
    guard: CastGuard,
    encoder: Encoder,
}

impl Casting {
    pub(super) fn new(encoder: Encoder) -> Self {
        Self {
            guard: CastGuard::default(),
            encoder,
        }
    }

    /// Casts a memorized spell at a target the player can see.
    fn cast(
        &mut self,
        command: &ClientCommand,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::CastSpell {
            gem,
            spell_id,
            target_id,
            ..
        } = command
        else {
            return Ok(());
        };
        let ready = world.player.as_ref().is_some_and(|player| {
            player.memorized_spells.get(usize::from(*gem)) == Some(&Some(*spell_id))
                && (*target_id == player.spawn_id || world.spawns.visible(*target_id).is_some())
        });
        let refusal = if ready {
            match self.encoder.encode(command) {
                Ok(packet) => {
                    out.send(&packet)?;
                    return self.submitted(*spell_id, world, out);
                }
                Err(error) => (
                    error.to_string(),
                    format!("Rejected invalid outbound client command: {error}"),
                ),
            }
        } else {
            (
                "The spell gem changed, or the target is unavailable".into(),
                "Rejected an unavailable spell or target".into(),
            )
        };
        if let Some(event) = rejected(command, &refusal.0) {
            out.log.send(ClientEvent::World(event))?;
        }
        out.log.diagnostic(refusal.1)
    }

    /// Casts an item's click effect at a target the player can see.
    fn use_item(&mut self, request: &ItemUse, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        let target_available = world.player.as_ref().is_some_and(|player| {
            request.target_id == player.spawn_id
                || world.spawns.visible(request.target_id).is_some()
        });
        let prepared = world
            .player
            .as_ref()
            .context("Character level is unavailable")
            .and_then(|player| {
                world.inventory.prepare_item_cast(
                    request,
                    world.session_id,
                    player.level,
                    target_available,
                    Instant::now(),
                )
            });
        let error = match prepared {
            Ok((spell_id, packet)) => {
                out.send(&packet)?;
                self.submitted(spell_id, world, out)?;
                None
            }
            Err(error) => Some(error.to_string()),
        };
        out.log.send(ClientEvent::World(WorldEvent::ItemUseAction {
            session_id: request.session_id,
            request_id: request.request_id,
            error,
        }))
    }

    /// Holds casting until the server answers the request.
    fn submitted(&mut self, spell_id: u32, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        self.guard.submitted(spell_id, Instant::now());
        out.log.send(ClientEvent::World(WorldEvent::CastPending {
            session_id: world.session_id,
            spell_id: Some(spell_id),
        }))
    }
}

impl Feature for Casting {
    fn holds(&self, _world: &World, _now: Instant) -> Vec<(Resource, &'static str)> {
        self.guard.hold().into_iter().collect()
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::CastSpell { .. } | ClientCommand::UseItem(_)
        )
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match command {
            ClientCommand::UseItem(request) => self.use_item(request, world, out),
            _ => self.cast(command, world, out),
        }
    }

    /// Releases a request the server never acknowledged, so the player may
    /// try again.
    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if !self.guard.expire(now) {
            return Ok(());
        }
        out.log.send(ClientEvent::World(WorldEvent::CastPending {
            session_id: world.session_id,
            spell_id: None,
        }))?;
        out.log
            .diagnostic("Cast acknowledgement timed out; a manual retry is available".into())
    }

    /// Follows the player's own casts; dying ends any cast.
    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match message {
            Message::Event(WorldEvent::Spell(update)) => {
                let Some(own) = world.player.as_ref().map(|player| player.spawn_id) else {
                    return Ok(());
                };
                let pending = self.guard.pending();
                self.guard.observe(own, update);
                if pending.is_some() && self.guard.pending().is_none() {
                    out.log.send(ClientEvent::World(WorldEvent::CastPending {
                        session_id: world.session_id,
                        spell_id: None,
                    }))?;
                }
            }
            Message::Event(WorldEvent::Death(death)) if world.is_player(death.spawn_id) => {
                self.guard.clear();
            }
            _ => (),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;

    #[test]
    fn a_memorized_spell_goes_out_and_holds_casting_until_answered() {
        let mut casting = Casting::new(Encoder::new(
            eq_network_game::GameDialect::TitaniumP99,
            "Tester",
        ));
        let mut world = World::new(5);
        let mut player = testing::player(7);
        player.memorized_spells[0] = Some(202);
        world.player = Some(player);
        let cast = |gem| ClientCommand::CastSpell {
            session_id: 5,
            gem,
            spell_id: 202,
            target_id: 7,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| casting.handle(&cast(0), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(WorldEvent::CastPending {
                spell_id: Some(202),
                ..
            })]
        ));
        assert_eq!(casting.holds(&world, Instant::now()).len(), 1);
        // A gem that does not hold the spell sends nothing and says why.
        let outcome = testing::run(|out| casting.handle(&cast(1), &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty());
        assert!(matches!(
            outcome.events[..],
            [
                ClientEvent::World(WorldEvent::CastRejected { .. }),
                ClientEvent::Diagnostic(_)
            ]
        ));
    }

    #[test]
    fn rejection_preserves_request_identity_without_changing_cast_exclusion() {
        let request = ClientCommand::CastSpell {
            session_id: 12,
            gem: 0,
            spell_id: 73,
            target_id: 7,
            created: Instant::now(),
        };
        let mut guard = CastGuard::default();
        guard.submitted(42, Instant::now());
        assert!(matches!(rejected(&request, "Target unavailable"),
            Some(crate::world::WorldEvent::CastRejected { session_id: 12, spell_id: 73, reason })
            if reason == "Target unavailable"));
        assert!(guard.blocks(&request));
        assert!(rejected(
            &ClientCommand::SelectTarget {
                session_id: 12,
                spawn_id: Some(7)
            },
            "Unavailable"
        )
        .is_none());
    }

    #[test]
    fn item_activations_share_cast_exclusion_without_blocking_posture_interrupts() {
        let now = Instant::now();
        let request = ClientCommand::UseItem(eq_network_game::inventory::ItemUse {
            request_id: 1,
            session_id: 12,
            revision: 1,
            slot: eq_network_game::inventory::InventorySlot(22),
            target_id: 7,
            created: now,
        });
        let mut guard = CastGuard::default();
        assert!(!guard.blocks(&request));
        guard.submitted(73, now);
        assert!(guard.blocks(&request));
        assert!(!guard.blocks(&ClientCommand::SetPosture {
            session_id: 12,
            spawn_id: 7,
            posture: eq_network_game::command::Posture::Ducking,
            created: now,
        }));
        guard.observe(
            7,
            &SpellUpdate::Interrupted {
                caster_id: 7,
                message_id: 1,
            },
        );
        assert!(!guard.blocks(&request));
    }

    fn begin(caster_id: u16) -> SpellUpdate {
        SpellUpdate::Began {
            caster_id,
            spell_id: 73,
            duration_ms: 0,
        }
    }

    #[test]
    fn submission_blocks_until_acknowledged_and_only_silent_requests_expire() {
        let now = Instant::now();
        let mut guard = CastGuard::default();
        guard.submitted(73, now);
        assert!(guard.active());
        assert_eq!(guard.pending(), Some(73));
        assert!(!guard.expire(now + Duration::from_millis(4999)));
        assert!(guard.expire(now + ACK_TIMEOUT));
        assert!(!guard.active());
        assert!(!guard.expire(now + ACK_TIMEOUT));
        guard.submitted(73, now);
        guard.observe(12, &begin(13));
        assert_eq!(guard.pending(), Some(73));
        guard.observe(12, &begin(12));
        assert_eq!(guard.pending(), None);
        assert!(guard.active());
        assert!(!guard.expire(now + Duration::from_secs(60)));
        guard.observe(
            12,
            &SpellUpdate::Interrupted {
                caster_id: 12,
                message_id: 1,
            },
        );
        assert!(!guard.active());
    }

    #[test]
    fn instant_cast_results_release_submission_without_a_begin_packet() {
        let mut guard = CastGuard::default();
        for update in [
            SpellUpdate::Mana {
                spell_id: 73,
                keep_casting: false,
            },
            SpellUpdate::BarRefresh {
                slot: 0,
                spell_id: 73,
                reduction_ms: 0,
            },
        ] {
            guard.submitted(73, Instant::now());
            guard.observe(12, &update);
            assert!(!guard.active());
            assert_eq!(guard.pending(), None);
        }
    }

    #[test]
    fn nearby_casts_and_unrelated_results_cannot_unlock_our_cast() {
        let mut guard = CastGuard::default();
        guard.observe(12, &begin(13));
        assert!(!guard.active());
        guard.observe(12, &begin(12));
        for update in [
            begin(13),
            SpellUpdate::Interrupted {
                caster_id: 13,
                message_id: 1,
            },
            SpellUpdate::Mana {
                spell_id: 73,
                keep_casting: true,
            },
            SpellUpdate::Mana {
                spell_id: 74,
                keep_casting: false,
            },
            SpellUpdate::BarRefresh {
                slot: 0,
                spell_id: 74,
                reduction_ms: 0,
            },
            SpellUpdate::Slot {
                slot: 0,
                spell_id: 73,
                mode: 1,
            },
        ] {
            guard.observe(12, &update);
            assert!(guard.active());
        }
        guard.observe(
            12,
            &SpellUpdate::Mana {
                spell_id: 73,
                keep_casting: false,
            },
        );
        assert!(!guard.active());
    }

    #[test]
    fn interrupt_refresh_and_lifecycle_reset_release_the_guard() {
        let mut guard = CastGuard::default();
        for update in [
            SpellUpdate::Interrupted {
                caster_id: 12,
                message_id: 1,
            },
            SpellUpdate::BarRefresh {
                slot: 0,
                spell_id: 73,
                reduction_ms: 0,
            },
        ] {
            guard.observe(12, &begin(12));
            assert!(guard.active());
            guard.observe(12, &update);
            assert!(!guard.active());
        }
        guard.observe(12, &begin(12));
        guard.clear();
        assert!(!guard.active());
    }

    #[test]
    fn confirmed_cast_blocks_spell_commands_but_allows_ducking_and_targeting() {
        let created = std::time::Instant::now();
        let commands = [
            ClientCommand::CastSpell {
                session_id: 1,
                gem: 0,
                spell_id: 73,
                target_id: 12,
                created,
            },
            ClientCommand::MemorizeSpell {
                session_id: 1,
                gem: 0,
                spell_id: 73,
                created,
            },
            ClientCommand::ForgetSpell {
                session_id: 1,
                gem: 0,
                spell_id: 73,
                created,
            },
            ClientCommand::DeleteSpell {
                session_id: 1,
                slot: 0,
                spell_id: 73,
                created,
            },
            ClientCommand::ScribeSpell {
                session_id: 1,
                revision: 0,
                slot: 0,
                spell_id: 73,
                created,
            },
        ];
        let mut guard = CastGuard::default();
        assert!(commands.iter().all(|command| !guard.blocks(command)));
        guard.observe(12, &begin(12));
        assert!(commands.iter().all(|command| guard.blocks(command)));
        assert!(!guard.blocks(&ClientCommand::SetPosture {
            session_id: 1,
            spawn_id: 12,
            posture: eq_network_game::command::Posture::Ducking,
            created,
        }));
        assert!(!guard.blocks(&ClientCommand::SelectTarget {
            session_id: 1,
            spawn_id: Some(13)
        }));
        guard.observe(
            12,
            &SpellUpdate::Interrupted {
                caster_id: 12,
                message_id: 1,
            },
        );
        assert!(commands.iter().all(|command| !guard.blocks(command)));
    }
}
