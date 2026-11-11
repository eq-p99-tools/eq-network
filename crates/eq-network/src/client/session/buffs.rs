//! Own-player received slots and explicit beneficial-buff cancellation.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    buffs::{
        cancel::{self, Progress, Request},
        Applies, Buff, BuffUpdate,
    },
    command::EncodedCommand,
    message::Message,
    world::WorldEvent,
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

/// A local observation interval, never a resend or a server acknowledgement.
const WAIT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct Buffs {
    slots: Option<BTreeMap<u32, Buff>>,
    revision: u64,
    pending: Option<(Request, Instant)>,
    p99: bool,
}

impl Buffs {
    /// Uses the two Titanium packets for one ordinary P99 cancel action.
    pub(super) fn project1999() -> Self {
        Self {
            p99: true,
            ..Self::default()
        }
    }

    /// Revalidates the exact received slot and encodes the entire transaction
    /// before submitting either packet.
    fn cancellation_packets(
        &self,
        request: &Request,
        world: &World,
    ) -> Result<(EncodedCommand, Option<EncodedCommand>)> {
        anyhow::ensure!(
            self.pending.is_none(),
            "Wait for the buff cancellation observation"
        );
        request.check(self.revision, self.slots.as_ref())?;
        let own = world
            .player
            .as_ref()
            .map(|player| player.spawn_id)
            .ok_or_else(|| anyhow::anyhow!("The player is unavailable"))?;
        if self.p99 {
            let [fade, clear] = cancel::project1999_packets(own, request.slot, request.spell_id)?;
            Ok((fade, Some(clear)))
        } else {
            let buff = request
                .record(self.slots.as_ref())
                .ok_or_else(|| anyhow::anyhow!("The selected buff slot is unavailable"))?;
            Ok((cancel::titanium_packet(own, request.slot, buff)?, None))
        }
    }

    fn snapshot(&mut self, buffs: &[Option<Buff>]) {
        self.slots = Some(
            buffs
                .iter()
                .enumerate()
                .filter_map(|(slot, buff)| Some((u32::try_from(slot).ok()?, buff.clone()?)))
                .collect(),
        );
    }

    /// Applies a received buff update to the received slots, as the
    /// program applies it ([`Applies`]), and says which fade evidence it
    /// gives: a received slot's fade, or a fade of a spell no received slot
    /// held, which is a modelled slot's evidence.
    fn received_update(&mut self, update: &BuffUpdate) -> (Option<(u32, u32)>, Option<u32>) {
        let mut faded = None;
        let mut modelled_fade = None;
        let pending = self.pending.as_ref().map(|(request, _)| request.slot);
        if let Some(slots) = &mut self.slots {
            match (update.applies, &update.buff) {
                // A Titanium update by spell changes the record
                // where a received slot holds it, and adds none
                // (R20 (e)).
                (Applies::BySpell, buff) => {
                    if let Some(buff) = buff {
                        for held in slots.values_mut().filter(|held| {
                            held.spell_id == buff.spell_id && held.effect_type == update.record_type
                        }) {
                            *held = buff.clone();
                        }
                    }
                }
                // A Titanium fade removes the spell from every slot
                // of its type byte, whatever slot it names. A type-4
                // record (target types 13 and 20 on oneself) outlives a
                // type-2 fade, as in the program: a known parity item
                // (recording checklist 6.459).
                (Applies::FadeBySpell, _) => {
                    let removed: Vec<u32> = slots
                        .iter()
                        .filter(|(_, held)| {
                            held.spell_id == update.spell_id
                                && held.effect_type == update.record_type
                        })
                        .map(|(slot, _)| *slot)
                        .collect();
                    for slot in &removed {
                        slots.remove(slot);
                    }
                    if let Some(slot) = removed
                        .iter()
                        .find(|slot| Some(**slot) == pending)
                        .or(removed.first())
                    {
                        faded = Some((*slot, update.spell_id));
                    } else {
                        // No received slot held it: evidence for a
                        // modelled one.
                        modelled_fade = Some(update.spell_id);
                    }
                }
                (_, Some(buff)) => {
                    slots.insert(update.slot, buff.clone());
                }
                (_, None) if update.replaces_slot() => {
                    slots.remove(&update.slot);
                }
                (_, None) => {
                    if slots
                        .get(&update.slot)
                        .is_some_and(|buff| buff.spell_id == update.spell_id)
                    {
                        // Only an ordinary named fade supplies fade
                        // evidence.
                        slots.remove(&update.slot);
                        faded = Some((update.slot, update.spell_id));
                    } else {
                        modelled_fade = Some(update.spell_id);
                    }
                }
            }
        }
        (faded, modelled_fade)
    }

    fn report(request: &Request, progress: Progress, out: &mut Out<'_, '_>) -> Result<()> {
        out.log
            .send(ClientEvent::World(WorldEvent::BuffCancellation(
                request.update(progress),
            )))
    }
}

impl Feature for Buffs {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::CancellingBuffs]
    }

    fn admit(&mut self, message: &Message, _world: &mut World) -> Result<()> {
        if let Message::Event(WorldEvent::BuffSnapshot(buffs)) = message {
            self.snapshot(buffs);
        }
        Ok(())
    }

    fn admitted(&mut self, _world: &mut World, _out: &mut Out<'_, '_>) -> Result<()> {
        self.revision = u64::from(self.slots.is_some());
        Ok(())
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::CancelBuff(_))
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::CancelBuff(request) = command else {
            return Ok(());
        };
        let (first, second) = match self.cancellation_packets(request, world) {
            Ok(packets) => packets,
            Err(error) => return actions::refuse(command, &error.to_string(), out.log),
        };
        out.send(&first)?;
        self.pending = Some((request.clone(), Instant::now()));
        if let Err(error) = second.map_or(Ok(()), |packet| out.send(&packet)) {
            Self::report(request, Progress::PartiallySubmitted, out)?;
            return Err(error);
        }
        Self::report(request, Progress::Submitted, out)
    }

    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        // Lifecycle owners can mark death/transfer after this feature observes
        // the same event. End the wait before any later timeout report.
        if world.lifecycle.is_dead() || world.lifecycle.blocks_motion() {
            self.pending = None;
            return Ok(());
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|(_, since)| now.saturating_duration_since(*since) >= WAIT)
        {
            let (request, _) = self.pending.take().expect("pending request");
            Self::report(&request, Progress::Unresolved, out)?;
        }
        Ok(())
    }

    fn connection_ended(&mut self, _world: &mut World) {
        *self = Self {
            p99: self.p99,
            ..Self::default()
        };
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if world.lifecycle.blocks_motion()
            || matches!(
                message,
                Message::ZoneOffer(_)
                    | Message::ZoneAnswer(_)
                    | Message::Handoff(_)
                    | Message::LoggedOut
            )
            || matches!(message, Message::Event(event) if world.owns_death(event))
        {
            self.pending = None;
        }
        let Message::Event(event) = message else {
            return Ok(());
        };
        // Keep reducing received slots even during a transfer: a refused
        // transfer resumes this admission without another profile snapshot.
        let (mut faded, mut modelled_fade) = (None, None);
        match event {
            WorldEvent::BuffSnapshot(buffs) => {
                self.snapshot(buffs);
                self.revision = self.revision.wrapping_add(1);
            }
            WorldEvent::Buff(update) if world.is_player(update.entity_id) => {
                self.revision = self.revision.wrapping_add(1);
                (faded, modelled_fade) = self.received_update(update);
            }
            WorldEvent::SpellEffect(effect)
                if world.is_player(effect.target_id) && effect.took_hold() =>
            {
                // A landing carries no slot. It invalidates the old assignment,
                // even when the host cannot yet classify the spell's duration.
                if let Some(slots) = &mut self.slots {
                    slots.retain(|_, buff| buff.spell_id != u32::from(effect.spell_id));
                }
            }
            _ => (),
        }
        if let Some((request, _)) = &self.pending {
            let modelled = matches!(request.placement, cancel::Placement::Modelled(_));
            let progress = if faded == Some((request.slot, request.spell_id))
                || (modelled && modelled_fade == Some(request.spell_id))
            {
                Some(Progress::FadeObserved)
            } else if modelled {
                // A modelled slot has no received record to change.
                None
            } else if self
                .slots
                .as_ref()
                .and_then(|slots| slots.get(&request.slot))
                .is_none_or(|buff| buff.spell_id != request.spell_id)
            {
                Some(Progress::Changed)
            } else {
                None
            };
            if let Some(progress) = progress {
                let (request, _) = self.pending.take().expect("pending request");
                Self::report(&request, progress, out)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;

    #[test]
    fn failing_the_first_or_second_send_keeps_the_submitted_prefix_explicit() {
        use super::super::feature::Sink;
        use eq_network_game::{command::EncodedCommand, request::Sender};
        struct Fails {
            accepted: usize,
            before_failure: usize,
        }
        impl Sink for Fails {
            fn send(&mut self, _packet: &EncodedCommand) -> Result<()> {
                anyhow::ensure!(
                    self.accepted < self.before_failure,
                    "constructed send failure"
                );
                self.accepted += 1;
                Ok(())
            }
            fn send_unreliable(&mut self, _packet: &EncodedCommand) -> Result<()> {
                anyhow::bail!("constructed send failure")
            }
            fn last_received_seconds(&self) -> u64 {
                0
            }
        }
        for (p99, before_failure) in [(false, 0), (true, 0), (true, 1)] {
            let (mut buffs, mut world, command) = fixture();
            buffs.p99 = p99;
            let config = crate::client::ClientConfig::new(
                "EXAMPLE",
                "EXAMPLE",
                "Constructed",
                "Constructed",
            );
            let mut events = Vec::new();
            let mut handler = |event| {
                events.push(event);
                Ok(())
            };
            let mut log = super::super::Events::new(&config, &mut handler);
            let mut sink = Fails {
                accepted: 0,
                before_failure,
            };
            let result = buffs.handle(
                &command,
                &mut world,
                &mut Out {
                    sink: &mut sink,
                    log: &mut log,
                    wire: &super::super::wire::Titanium,
                    sender: Sender {
                        name: "Constructed",
                        spawn_id: Some(7),
                    },
                },
            );
            assert!(result.is_err());
            assert_eq!(sink.accepted, before_failure);
            drop(log);
            if before_failure == 0 {
                assert!(events.is_empty());
                assert!(buffs.pending.is_none());
            } else {
                assert_eq!(progress(&events), [Progress::PartiallySubmitted]);
                assert!(buffs.pending.is_some());
            }
            assert_eq!(buffs.slots.as_ref().unwrap()[&3].spell_id, 42);
        }
    }

    fn buff(spell_id: u32) -> Buff {
        Buff {
            spell_id,
            caster_level: 12,
            effect_type: 2,
            bard_modifier: 10,
            duration_ticks: 30,
            counters: 0,
            caster_id: 7,
        }
    }

    fn fixture() -> (Buffs, World, ClientCommand) {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        let mut buffs = Buffs::default();
        buffs.snapshot(&[None, None, None, Some(buff(42))]);
        buffs.revision = 1;
        let command = ClientCommand::CancelBuff(Request {
            session_id: 5,
            revision: 1,
            slot: 3,
            spell_id: 42,
            beneficial: Some(true),
            cancellable: Some(true),
            created: Instant::now(),
            placement: eq_network_game::buffs::cancel::Placement::Received,
        });
        (buffs, world, command)
    }

    fn progress(events: &[ClientEvent]) -> Vec<Progress> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::BuffCancellation(update)) => {
                    Some(update.progress.clone())
                }
                _ => None,
            })
            .collect()
    }

    fn update(entity_id: u32, spell_id: u32, replacement: Option<Buff>) -> Message {
        Message::Event(WorldEvent::Buff(BuffUpdate {
            applies: eq_network_game::buffs::Applies::Received,
            record_type: 0,
            entity_id,
            slot: 3,
            spell_id,
            buff: replacement,
        }))
    }

    #[test]
    fn p99_pair_changes_no_membership_until_a_received_named_fade() {
        let (mut buffs, mut world, command) = fixture();
        buffs.p99 = true;
        let [first, clear] = cancel::project1999_packets(7, 3, 42).unwrap();
        let outcome = testing::run(|out| {
            buffs.handle(&command, &mut world, out)?;
            let packet = |body| eq_network_game::message::titanium(0x6a53, body).remove(0);
            buffs.observe(&packet(&clear.body), &mut world, out)?;
            buffs.observe(&update(8, 42, None), &mut world, out)?;
            buffs.observe(&update(7, 43, None), &mut world, out)?;
            assert_eq!(buffs.slots.as_ref().unwrap()[&3].spell_id, 42);
            let mut untyped = first.body.clone();
            untyped[28] = 1;
            let mut fade = untyped.clone();
            fade[4] = 2;
            // A fade whose record has no type byte matches no slot, as in
            // the program (R20 (e)); a server's fade carries type 2.
            buffs.observe(&packet(&untyped), &mut world, out)?;
            assert_eq!(buffs.slots.as_ref().unwrap()[&3].spell_id, 42);
            buffs.observe(&packet(&fade), &mut world, out)?;
            assert!(buffs.slots.as_ref().unwrap().is_empty());
            assert!(buffs.pending.is_none());
            Ok::<(), anyhow::Error>(())
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [first, clear]);
        assert_eq!(
            progress(&outcome.events),
            [Progress::Submitted, Progress::FadeObserved]
        );
    }

    #[test]
    fn a_modelled_slot_sends_its_record_and_a_fade_of_its_spell_anywhere_is_observed() {
        let (mut buffs, mut world, _) = fixture();
        let mut record = buff(278);
        record.caster_level = 0;
        record.caster_id = 0;
        // After the correction below, the slots' revision is 2.
        let command = ClientCommand::CancelBuff(Request {
            session_id: 5,
            revision: 2,
            slot: 0,
            spell_id: 278,
            beneficial: Some(true),
            cancellable: Some(true),
            created: Instant::now(),
            placement: cancel::Placement::Modelled(record.clone()),
        });
        let outcome = testing::run(|out| {
            // The server's duration correction for its own slot, 6: by
            // spell, it changes no received slot and adds none.
            let correction = Message::Event(WorldEvent::Buff(BuffUpdate {
                applies: Applies::BySpell,
                record_type: 2,
                entity_id: 7,
                slot: 6,
                spell_id: 278,
                buff: Some(buff(278)),
            }));
            buffs.observe(&correction, &mut world, out)?;
            assert!(!buffs.slots.as_ref().unwrap().contains_key(&6));
            buffs.handle(&command, &mut world, out)?;
            // It fades by spell, naming slot 3.
            buffs.observe(&update(7, 278, None), &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [cancel::titanium_packet(7, 0, &record).unwrap()]
        );
        assert_eq!(
            progress(&outcome.events),
            [Progress::Submitted, Progress::FadeObserved]
        );
        // A modelled slot a received record holds is refused.
        let ClientCommand::CancelBuff(mut request) = command else {
            unreachable!()
        };
        request.slot = 3;
        let outcome =
            testing::run(|out| buffs.handle(&ClientCommand::CancelBuff(request), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 0);
        assert!(matches!(
            progress(&outcome.events).as_slice(),
            [Progress::Refused(_)]
        ));
    }

    #[test]
    fn submission_keeps_membership_until_a_matching_received_fade() {
        let (mut buffs, mut world, command) = fixture();
        let outcome = testing::run(|out| {
            buffs.handle(&command, &mut world, out)?;
            assert_eq!(buffs.slots.as_ref().unwrap()[&3].spell_id, 42);
            buffs.observe(&update(8, 42, None), &mut world, out)?;
            assert!(buffs.pending.is_some());
            buffs.observe(&update(7, 42, None), &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(
            progress(&outcome.events),
            [Progress::Submitted, Progress::FadeObserved]
        );
        assert!(!buffs.slots.as_ref().unwrap().contains_key(&3));
        assert!(buffs.pending.is_none());
    }

    #[test]
    fn replacement_and_late_fade_never_remove_or_cancel_the_new_spell() {
        let (mut buffs, mut world, command) = fixture();
        let outcome = testing::run(|out| {
            buffs.handle(&command, &mut world, out)?;
            buffs.observe(&update(7, 43, Some(buff(43))), &mut world, out)?;
            buffs.observe(&update(7, 42, None), &mut world, out)?;
            buffs.handle(&command, &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(buffs.slots.as_ref().unwrap()[&3].spell_id, 43);
        let reports = progress(&outcome.events);
        assert_eq!(&reports[..2], [Progress::Submitted, Progress::Changed]);
        assert!(matches!(reports.last(), Some(Progress::Refused(_))));
    }

    #[test]
    fn echoes_and_unidentified_fades_leave_cancellation_unresolved() {
        let (mut buffs, mut world, command) = fixture();
        let outcome = testing::run(|out| {
            buffs.handle(&command, &mut world, out)?;
            buffs.handle(&command, &mut world, out)?;
            buffs.observe(&update(7, 42, Some(buff(42))), &mut world, out)?;
            for spell in [0, 0xffff, u32::MAX] {
                buffs.observe(&update(7, spell, None), &mut world, out)?;
            }
            let since = buffs.pending.as_ref().unwrap().1;
            buffs.tick(
                (since + WAIT)
                    .checked_sub(Duration::from_millis(1))
                    .unwrap(),
                &mut world,
                out,
            )?;
            assert!(buffs.pending.is_some());
            buffs.tick(since + WAIT, &mut world, out)?;
            buffs.tick(since + WAIT + WAIT, &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(buffs.slots.as_ref().unwrap()[&3].spell_id, 42);
        assert!(buffs.pending.is_none());
        assert!(matches!(
            progress(&outcome.events).as_slice(),
            [
                Progress::Submitted,
                Progress::Refused(_),
                Progress::Unresolved
            ]
        ));
    }

    #[test]
    fn harmful_unknown_stale_empty_and_unslotted_selections_are_not_sent() {
        let (mut buffs, mut world, mut command) = fixture();
        let outcome = testing::run(|out| {
            let ClientCommand::CancelBuff(request) = &mut command else {
                unreachable!()
            };
            request.beneficial = Some(false);
            buffs.handle(&command, &mut world, out)?;
            let ClientCommand::CancelBuff(request) = &mut command else {
                unreachable!()
            };
            request.beneficial = None;
            buffs.handle(&command, &mut world, out)?;
            let ClientCommand::CancelBuff(request) = &mut command else {
                unreachable!()
            };
            request.beneficial = Some(true);
            request.revision = 0;
            buffs.handle(&command, &mut world, out)?;
            let ClientCommand::CancelBuff(request) = &mut command else {
                unreachable!()
            };
            request.revision = 1;
            request.slot = 0;
            buffs.handle(&command, &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert_eq!(progress(&outcome.events).len(), 4);
    }

    #[test]
    fn disconnect_and_death_discard_pending_state() {
        for dead in [false, true] {
            let (mut buffs, mut world, command) = fixture();
            let outcome = testing::run(|out| {
                buffs.handle(&command, &mut world, out)?;
                if dead {
                    world.died();
                    // No further packet need arrive after the lifecycle owner
                    // marks death. The next timer must drop the pending wait.
                } else {
                    buffs.connection_ended(&mut world);
                }
                buffs.tick(Instant::now() + WAIT, &mut world, out)
            });
            outcome.result.unwrap();
            assert_eq!(progress(&outcome.events), [Progress::Submitted]);
            assert!(buffs.pending.is_none());
            if dead {
                assert_eq!(buffs.slots.as_ref().unwrap()[&3].spell_id, 42);
            } else {
                assert!(buffs.slots.is_none());
            }
        }
    }

    #[test]
    fn a_refused_transfer_keeps_reducing_received_slots_without_an_old_wait() {
        let (mut buffs, mut world, command) = fixture();
        let now = Instant::now();
        let outcome = testing::run(|out| {
            buffs.handle(&command, &mut world, out)?;
            world.lifecycle.offer(
                eq_network_game::zoning::ZoneOffer {
                    zone_id: 9,
                    instance_id: 0,
                    position: eq_network_game::world::Position::default(),
                    reason: 0,
                    to_bind: false,
                    solicited: true,
                },
                now,
            )?;
            buffs.observe(&update(7, 43, Some(buff(43))), &mut world, out)?;
            assert!(buffs.pending.is_none());
            assert_eq!(buffs.revision, 2);
            world.lifecycle.finish(false, now)?;
            buffs.tick(now + WAIT, &mut world, out)?;
            let ClientCommand::CancelBuff(mut request) = command.clone() else {
                unreachable!()
            };
            request.revision = 2;
            request.spell_id = 43;
            buffs.handle(&ClientCommand::CancelBuff(request), &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 2);
        assert_eq!(
            progress(&outcome.events),
            [Progress::Submitted, Progress::Submitted]
        );
        assert_eq!(buffs.slots.as_ref().unwrap()[&3].spell_id, 43);
    }
}
