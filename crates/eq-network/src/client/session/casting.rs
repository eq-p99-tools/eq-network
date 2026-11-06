//! Casting: spells from the memorized gems and items' click effects, each held
//! until the server answers it.
use super::{
    actions::Resource,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::{Context, Result};
use eq_network_game::{
    inventory::ItemUse,
    message::Message,
    request::Request,
    spells::{SpellUpdate, ITEM_CAST_SLOT},
    world::WorldEvent,
};
use std::time::{Duration, Instant};

/// Local retry policy, not a protocol timeout. Never resends a request automatically.
const ACK_TIMEOUT: Duration = Duration::from_secs(5);

/// Allow ordered cast results already in flight to arrive. `EQEmu` can stop a
/// song without sending the Bard an interrupt or spell-bar refresh.
const SONG_STOP_WAIT: Duration = Duration::from_secs(1);

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
    Awaiting {
        spell: u32,
        submitted: Instant,
    },
    /// The server began a cast: the spell it names, and the one asked for,
    /// which it can name differently. TAKP begins a Luclin port as spell
    /// 2935 (`Mob::DoCastSpell`) and ends it naming the port.
    Casting {
        spell: u32,
        requested: Option<u32>,
    },
}

#[derive(Clone, Copy)]
pub(super) enum Source {
    Gem(u8),
    Item,
}

#[derive(Default)]
pub(super) struct CastGuard {
    phase: Option<Phase>,
    source: Option<Source>,
    stop_deadline: Option<Instant>,
    late_begin: Option<u32>,
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
    pub fn submitted(&mut self, spell: u32, source: Option<Source>, now: Instant) {
        *self = Self {
            phase: Some(Phase::Awaiting {
                spell,
                submitted: now,
            }),
            source,
            late_begin: self.late_begin.filter(|&abandoned| abandoned != spell),
            ..Self::default()
        };
    }

    /// The spell named by the current hold.
    fn held_spell(&self) -> Option<u32> {
        match self.phase {
            Some(Phase::Awaiting { spell, .. } | Phase::Casting { spell, .. }) => Some(spell),
            None => None,
        }
    }

    /// The unacknowledged request, distinct from a server-confirmed cast.
    pub fn pending(&self) -> Option<u32> {
        match self.phase {
            Some(Phase::Awaiting { spell, .. }) => Some(spell),
            _ => None,
        }
    }

    /// Ends the local hold after a submitted stop received no matching result.
    fn expire_song_stop(&mut self, now: Instant) -> Option<u32> {
        if now < self.stop_deadline? {
            return None;
        }
        let spell = self.held_spell()?;
        let late_begin = self.pending();
        self.clear();
        self.late_begin = late_begin;
        Some(spell)
    }

    /// Allows a fresh user attempt after silence; ordinary begun casts stay held.
    pub fn expire(&mut self, now: Instant) -> bool {
        if self.stop_deadline.is_none()
            && matches!(self.phase, Some(Phase::Awaiting { submitted, .. })
            if now.checked_duration_since(submitted).is_some_and(|age| age >= ACK_TIMEOUT))
        {
            self.clear();
            true
        } else {
            false
        }
    }

    /// Whether a result for this spell answers the cast held: the spell
    /// requested, or the one the server began.
    fn answers(&self, spell_id: u32) -> bool {
        match self.phase {
            Some(Phase::Awaiting { spell, .. }) => spell == spell_id,
            Some(Phase::Casting { spell, requested }) => {
                spell == spell_id || requested == Some(spell_id)
            }
            None => false,
        }
    }

    /// Clears state when this character dies or the owning zone session ends.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Follows own starts and interruptions and matching spell results.
    /// Returns the stopped hold if a different own spell begins.
    fn observe(&mut self, own_id: u16, update: &SpellUpdate) -> Option<u32> {
        match *update {
            SpellUpdate::Began {
                caster_id,
                spell_id,
                ..
            } if caster_id == own_id => {
                let spell = u32::from(spell_id);
                let mut requested = self.pending();
                let superseded = if self.stop_deadline.is_some() && !self.answers(spell) {
                    self.stop_deadline = None;
                    self.source = None;
                    self.late_begin = requested.take().or(self.late_begin);
                    self.held_spell()
                } else {
                    None
                };
                self.phase = Some(Phase::Casting { spell, requested });
                return superseded;
            }
            SpellUpdate::Interrupted { caster_id, .. } if caster_id == u32::from(own_id) => {
                self.clear();
            }
            SpellUpdate::Mana {
                spell_id,
                keep_casting: false,
            } if self.answers(spell_id) => {
                self.clear();
            }
            SpellUpdate::BarRefresh { slot, spell_id, .. }
                if self.answers(spell_id)
                    && self.source.is_none_or(|source| {
                        slot == match source {
                            Source::Gem(gem) => u32::from(gem),
                            Source::Item => u32::from(ITEM_CAST_SLOT),
                        }
                    }) =>
            {
                self.clear();
            }
            _ => (),
        }
        None
    }
}

/// Casts spells and item effects for the player, one at a time.
#[derive(Default)]
pub(super) struct Casting {
    guard: CastGuard,
    song_stop: bool,
}

const BARD_CLASS: u32 = 8;

impl Casting {
    pub(super) fn with_song_stop() -> Self {
        Self {
            song_stop: true,
            ..Self::default()
        }
    }

    /// Stops a Bard's song. An active hold must belong to a known gem cast
    /// because the empty server request can interrupt other cast sources.
    fn stop_song(&mut self, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        if !self.song_stop {
            return out
                .log
                .diagnostic("Stopping songs is unavailable on this server".into());
        }
        if world
            .player
            .as_ref()
            .is_none_or(|p| p.class != Some(BARD_CLASS))
        {
            return out.log.diagnostic("Only Bards can stop a song".into());
        }
        if self.guard.active() && !matches!(self.guard.source, Some(Source::Gem(_))) {
            return out
                .log
                .diagnostic("Wait for the current cast to finish before stopping a song".into());
        }
        if self.guard.stop_deadline.is_some() {
            return out
                .log
                .diagnostic("A song stop is already awaiting a cast update".into());
        }
        out.request(&Request::StopSong)?;
        if self.guard.active() {
            self.guard.stop_deadline = Some(Instant::now() + SONG_STOP_WAIT);
            Ok(())
        } else {
            report_unconfirmed_stop(world.session_id, None, out)
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
        let ready = world.visible(*target_id)
            && world.player.as_ref().is_some_and(|player| {
                player.memorized_spells.get(usize::from(*gem)) == Some(&Some(*spell_id))
            });
        let refusal = if ready {
            match out.encode_command(command) {
                Ok(packet) => {
                    out.send(&packet)?;
                    return self.submitted(*spell_id, Source::Gem(*gem), world, out);
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
        let target_available = world.player.is_some() && world.visible(request.target_id);
        let prepared = world
            .player
            .as_ref()
            .context("Character level is unavailable")
            .and_then(|player| {
                world
                    .inventory
                    .check_item_use(request, player.level, target_available)
            })
            .and_then(|spell_id| {
                let packet = out.encode(&Request::CastItem {
                    spell_id,
                    slot: request.slot,
                    target_id: request.target_id,
                })?;
                Ok((spell_id, packet))
            });
        let error = match prepared {
            Ok((spell_id, packet)) => {
                out.send(&packet)?;
                self.submitted(spell_id, Source::Item, world, out)?;
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
    fn submitted(
        &mut self,
        spell_id: u32,
        source: Source,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.guard.submitted(spell_id, Some(source), Instant::now());
        out.log.send(ClientEvent::World(WorldEvent::CastPending {
            session_id: world.session_id,
            spell_id: Some(spell_id),
        }))
    }
}

impl Feature for Casting {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        if self.song_stop {
            vec![Capability::Casting, Capability::StoppingSongs]
        } else {
            vec![Capability::Casting]
        }
    }

    fn holds(&self, _world: &World, _now: Instant) -> Vec<(Resource, &'static str)> {
        self.guard.hold().into_iter().collect()
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::CastSpell { .. }
                | ClientCommand::UseItem(_)
                | ClientCommand::DropInvisibility { .. }
                | ClientCommand::StopSong { .. }
        )
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match command {
            ClientCommand::StopSong { .. } => self.stop_song(world, out),
            ClientCommand::UseItem(request) => self.use_item(request, world, out),
            ClientCommand::DropInvisibility { steps, .. } => drop_invisibility(steps, out),
            _ => self.cast(command, world, out),
        }
    }

    /// Releases a local hold when the server sends no matching reply.
    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if let Some(spell) = self.guard.expire_song_stop(now) {
            return report_unconfirmed_stop(world.session_id, Some(spell), out);
        }
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

    /// Withholds a start for an abandoned request even while a different
    /// spell is pending or casting.
    fn explain(&mut self, message: &mut Message, world: &World) {
        let Message::Event(WorldEvent::Spell(SpellUpdate::Began {
            caster_id,
            spell_id,
            ..
        })) = message
        else {
            return;
        };
        if world.is_player(*caster_id) && self.guard.late_begin == Some(u32::from(*spell_id)) {
            self.guard.late_begin = None;
            *message = Message::Withheld;
        }
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
                if let Some(spell_id) = self.guard.observe(own, update) {
                    out.log
                        .send(ClientEvent::World(WorldEvent::CastSuperseded {
                            session_id: world.session_id,
                            spell_id,
                        }))?;
                } else if pending.is_some() && self.guard.pending().is_none() {
                    out.log.send(ClientEvent::World(WorldEvent::CastPending {
                        session_id: world.session_id,
                        spell_id: None,
                    }))?;
                }
            }
            Message::Event(event) if world.owns_death(event) => {
                self.guard.clear();
            }
            _ => (),
        }
        Ok(())
    }
}

fn report_unconfirmed_stop(
    session_id: u64,
    spell_id: Option<u32>,
    out: &mut Out<'_, '_>,
) -> Result<()> {
    out.log
        .send(ClientEvent::World(WorldEvent::SongStopUnconfirmed {
            session_id,
            spell_id,
        }))
}

/// Sends the steps of an invisibility drop in the host's order, where the
/// client generation sends them. Every step is encoded before any is sent,
/// so a generation that sends none, or a step it cannot carry, sends
/// nothing and is only noted.
fn drop_invisibility(
    steps: &[eq_network_game::buffs::InvisibilityStep],
    out: &mut Out<'_, '_>,
) -> Result<()> {
    let packets: Result<Vec<_>> = steps
        .iter()
        .map(|step| out.encode(&Request::DropInvisibility(*step)))
        .collect();
    match packets {
        Ok(packets) => packets.iter().try_for_each(|packet| out.send(packet)),
        Err(error) => out
            .log
            .diagnostic(format!("Invisibility not dropped: {error}")),
    }
}

#[cfg(test)]
mod song_stop_tests;

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;

    #[test]
    fn a_received_interrupt_resolves_a_requested_song_stop() {
        let mut casting = Casting::with_song_stop();
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        player.class = Some(BARD_CLASS);
        player.memorized_spells[2] = Some(202);
        world.player.admit(player);
        let now = Instant::now();
        let cast = ClientCommand::CastSpell {
            session_id: 5,
            gem: 2,
            spell_id: 202,
            target_id: 7,
            created: now,
        };
        let stop = ClientCommand::StopSong {
            session_id: 5,
            created: now,
        };
        assert!(casting.owns(&stop));
        assert!(!casting.guard.blocks(&stop));

        testing::run(|out| casting.handle(&cast, &mut world, out))
            .result
            .unwrap();
        let output = testing::run(|out| casting.handle(&stop, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent.len(), 1);
        assert_eq!(output.sent[0].opcode, 0x4839);
        assert_eq!(output.sent[0].body, [] as [u8; 0]);
        assert!(output.events.is_empty());
        assert_eq!(casting.guard.pending(), Some(202));

        let duplicate = testing::run(|out| casting.handle(&stop, &mut world, out));
        duplicate.result.unwrap();
        assert!(duplicate.sent.is_empty());
        assert!(matches!(
            duplicate.events.as_slice(),
            [ClientEvent::Diagnostic(reason)]
                if reason == "A song stop is already awaiting a cast update"
        ));

        let refresh = |slot| {
            Message::Event(WorldEvent::Spell(SpellUpdate::BarRefresh {
                slot,
                spell_id: 202,
                reduction_ms: 0,
            }))
        };
        testing::run(|out| casting.observe(&refresh(1), &mut world, out))
            .result
            .unwrap();
        assert_eq!(casting.guard.pending(), Some(202));

        let interrupted = Message::Event(WorldEvent::Spell(SpellUpdate::Interrupted {
            caster_id: 7,
            message_id: 1,
            caster_name: None,
        }));
        testing::run(|out| casting.observe(&interrupted, &mut world, out))
            .result
            .unwrap();
        assert!(!casting.guard.active());

        // An already pulsing song need not be an active cast.
        let output = testing::run(|out| casting.handle(&stop, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent.len(), 1);
        assert!(matches!(
            output.events.as_slice(),
            [ClientEvent::World(WorldEvent::SongStopUnconfirmed {
                session_id: 5,
                spell_id: None
            })]
        ));
        let output =
            testing::run(|out| casting.tick(Instant::now() + SONG_STOP_WAIT, &mut world, out));
        output.result.unwrap();
        assert!(output.events.is_empty());

        let unavailable = testing::run(|out| Casting::default().handle(&stop, &mut world, out));
        unavailable.result.unwrap();
        assert!(unavailable.sent.is_empty());
        assert!(matches!(
            unavailable.events.as_slice(),
            [ClientEvent::Diagnostic(reason)] if reason == "Stopping songs is unavailable on this server"
        ));

        world.player.admit(testing::player(7));
        let output = testing::run(|out| casting.handle(&stop, &mut world, out));
        output.result.unwrap();
        assert_eq!(
            output.sent,
            [] as [eq_network_game::command::EncodedCommand; 0]
        );
        assert!(matches!(
            output.events.as_slice(),
            [ClientEvent::Diagnostic(reason)] if reason == "Only Bards can stop a song"
        ));

        // The requested gem's refresh still completes the normal cast path.
        casting.guard.submitted(202, Some(Source::Gem(2)), now);
        testing::run(|out| casting.observe(&refresh(2), &mut world, out))
            .result
            .unwrap();
        assert!(!casting.guard.active());
    }

    #[test]
    fn a_same_spell_gem_refresh_cannot_expose_an_item_cast_to_stop_song() {
        use eq_network_game::inventory::{
            ClickEffect, ClickKind, Inventory, InventorySlot, InventoryUpdate,
        };

        let mut casting = Casting::with_song_stop();
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        player.class = Some(BARD_CLASS);
        player.memorized_spells[2] = Some(202);
        world.player.admit(player);

        let slot = InventorySlot(22);
        let mut item = testing::item(slot.0);
        item.activation.effect = Some(ClickEffect {
            spell_id: 202,
            kind: ClickKind::Expendable,
            required_level: 0,
            effect_level: 1,
            cast_time_ms: 1000,
            recast_delay_seconds: 0,
            recast_type: 0,
        });
        item.charges = 1;
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![item]));
        world.inventory = inventory.into();
        let now = Instant::now();
        let item = ClientCommand::UseItem(ItemUse {
            request_id: 1,
            session_id: 5,
            revision: world.inventory.revision(),
            slot,
            target_id: 7,
            created: now,
        });
        let stop = ClientCommand::StopSong {
            session_id: 5,
            created: now,
        };

        let output = testing::run(|out| casting.handle(&item, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent.len(), 1);
        assert_eq!(casting.guard.pending(), Some(202));

        let refresh = |slot| {
            Message::Event(WorldEvent::Spell(SpellUpdate::BarRefresh {
                slot,
                spell_id: 202,
                reduction_ms: 0,
            }))
        };
        testing::run(|out| casting.observe(&refresh(2), &mut world, out))
            .result
            .unwrap();
        let output = testing::run(|out| casting.handle(&stop, &mut world, out));
        output.result.unwrap();
        assert_eq!(
            output.sent,
            [] as [eq_network_game::command::EncodedCommand; 0]
        );
        assert!(casting.guard.active());

        testing::run(|out| casting.observe(&refresh(10), &mut world, out))
            .result
            .unwrap();
        assert!(!casting.guard.active());
        let output = testing::run(|out| casting.handle(&stop, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent.len(), 1);
        assert_eq!(output.sent[0].opcode, 0x4839);
        assert_eq!(output.sent[0].body, [] as [u8; 0]);
    }

    #[test]
    fn a_memorized_spell_goes_out_and_holds_casting_until_answered() {
        let mut casting = Casting::default();
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        player.memorized_spells[0] = Some(202);
        world.player.admit(player);
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
        assert!(
            outcome.sent.is_empty(),
            "expected nothing sent, got {:?}",
            outcome.sent
        );
        assert!(matches!(
            outcome.events[..],
            [
                ClientEvent::World(WorldEvent::CastRejected { .. }),
                ClientEvent::Diagnostic(_)
            ]
        ));
    }

    #[test]
    fn takps_wire_casts_a_gem_in_its_own_packet_and_the_spell_bar_ends_the_hold() {
        let eqmac = &super::super::wire::EqMac;
        let mut casting = Casting::default();
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        player.memorized_spells[0] = Some(202);
        world.player.admit(player);
        let cast = ClientCommand::CastSpell {
            session_id: 5,
            gem: 0,
            spell_id: 202,
            target_id: 7,
            created: Instant::now(),
        };
        let outcome = testing::run_on(eqmac, |out| casting.handle(&cast, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, 0x7e41);
        assert_eq!(
            outcome.sent[0].body,
            [0, 0, 202, 0, 255, 255, 7, 0, 0, 0, 0, 0]
        );
        assert!(casting.guard.active());
        // TAKP's spell bar coming back, as the EQMac wire reads it.
        for message in eq_network_game::message::eqmac(0x7f41, &[90, 0, 202, 0]) {
            testing::run_on(eqmac, |out| casting.observe(&message, &mut world, out))
                .result
                .unwrap();
        }
        assert!(!casting.guard.active());
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
        guard.submitted(42, None, Instant::now());
        assert!(matches!(rejected(&request, "Target unavailable"),
            Some(crate::world::WorldEvent::CastRejected { session_id: 12, spell_id: 73, reason })
            if reason == "Target unavailable"));
        assert!(guard.blocks(&request));
        assert!(rejected(
            &ClientCommand::SelectTarget {
                request_id: 1,
                session_id: 12,
                spawn_id: Some(7),
                expected_pet: None,
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
        guard.submitted(73, None, now);
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
                caster_name: None,
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
        guard.submitted(73, None, now);
        assert!(guard.active());
        assert_eq!(guard.pending(), Some(73));
        assert!(!guard.expire(now + Duration::from_millis(4999)));
        assert!(guard.expire(now + ACK_TIMEOUT));
        assert!(!guard.active());
        assert!(!guard.expire(now + ACK_TIMEOUT));
        guard.submitted(73, None, now);
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
                caster_name: None,
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
            guard.submitted(73, None, Instant::now());
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
                caster_name: None,
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
    fn a_cast_begun_under_another_spell_ends_with_the_requested_ones_result() {
        // TAKP begins a Luclin port as spell 2935 and ends it naming the port.
        let begun_as = |spell_id| SpellUpdate::Began {
            caster_id: 12,
            spell_id,
            duration_ms: 10_000,
        };
        let ended = |spell_id| SpellUpdate::Mana {
            spell_id,
            keep_casting: false,
        };
        for end in [73, 2935] {
            let mut guard = CastGuard::default();
            guard.submitted(73, None, Instant::now());
            guard.observe(12, &begun_as(2935));
            guard.observe(12, &ended(74));
            assert!(guard.active());
            guard.observe(12, &ended(end));
            assert!(!guard.active());
        }
    }

    #[test]
    fn interrupt_refresh_and_lifecycle_reset_release_the_guard() {
        let mut guard = CastGuard::default();
        for update in [
            SpellUpdate::Interrupted {
                caster_id: 12,
                message_id: 1,
                caster_name: None,
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
            request_id: 1,
            session_id: 1,
            spawn_id: Some(13),
            expected_pet: None,
        }));
        guard.observe(
            12,
            &SpellUpdate::Interrupted {
                caster_id: 12,
                message_id: 1,
                caster_name: None,
            },
        );
        assert!(commands.iter().all(|command| !guard.blocks(command)));
    }
}
