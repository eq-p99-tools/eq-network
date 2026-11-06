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
        Buff,
    },
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
}

impl Buffs {
    fn snapshot(&mut self, buffs: &[Option<Buff>]) {
        self.slots = Some(
            buffs
                .iter()
                .enumerate()
                .filter_map(|(slot, buff)| Some((u32::try_from(slot).ok()?, buff.clone()?)))
                .collect(),
        );
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
        let checked = (|| {
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
            let buff = &self.slots.as_ref().expect("checked slots")[&request.slot];
            cancel::titanium_packet(own, request.slot, buff)
        })();
        let packet = match checked {
            Ok(packet) => packet,
            Err(error) => return actions::refuse(command, &error.to_string(), out.log),
        };
        out.send(&packet)?;
        self.pending = Some((request.clone(), Instant::now()));
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
        *self = Self::default();
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let Message::Event(event) = message else {
            return Ok(());
        };
        if world.lifecycle.is_dead() || world.lifecycle.blocks_motion() {
            self.pending = None;
        }
        // Keep reducing received slots even during a transfer: a refused
        // transfer resumes this admission without another profile snapshot.
        let mut faded = None;
        match event {
            WorldEvent::BuffSnapshot(buffs) => {
                self.snapshot(buffs);
                self.revision = self.revision.wrapping_add(1);
            }
            WorldEvent::Buff(update) if world.is_player(update.entity_id) => {
                self.revision = self.revision.wrapping_add(1);
                if let Some(slots) = &mut self.slots {
                    if let Some(buff) = &update.buff {
                        slots.insert(update.slot, buff.clone());
                    } else {
                        // An explicit fade names its spell as well as its slot.
                        // Never let a late old-spell fade remove a replacement.
                        if slots
                            .get(&update.slot)
                            .is_some_and(|buff| buff.spell_id == update.spell_id)
                        {
                            slots.remove(&update.slot);
                        }
                        faded = Some((update.slot, update.spell_id));
                    }
                }
            }
            WorldEvent::SpellEffect(effect)
                if world.is_player(effect.target_id) && effect.effect_flag == 4 =>
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
            let progress = if faded == Some((request.slot, request.spell_id)) {
                Some(Progress::ObservedRemoval)
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
    use eq_network_game::buffs::BuffUpdate;

    #[test]
    fn a_failed_transport_send_creates_no_pending_or_success_report() {
        use super::super::feature::Sink;
        use eq_network_game::{command::EncodedCommand, request::Sender};
        struct Fails;
        impl Sink for Fails {
            fn send(&mut self, _packet: &EncodedCommand) -> Result<()> {
                anyhow::bail!("constructed send failure")
            }
            fn send_unreliable(&mut self, _packet: &EncodedCommand) -> Result<()> {
                anyhow::bail!("constructed send failure")
            }
            fn last_received_seconds(&self) -> u64 {
                0
            }
        }
        let (mut buffs, mut world, command) = fixture();
        let config =
            crate::client::ClientConfig::new("EXAMPLE", "EXAMPLE", "Constructed", "Constructed");
        let mut events = Vec::new();
        let mut handler = |event| {
            events.push(event);
            Ok(())
        };
        let mut log = super::super::Events::new(&config, &mut handler);
        let result = buffs.handle(
            &command,
            &mut world,
            &mut Out {
                sink: &mut Fails,
                log: &mut log,
                wire: &super::super::wire::Titanium,
                sender: Sender {
                    name: "Constructed",
                    spawn_id: Some(7),
                },
            },
        );
        assert!(result.is_err());
        drop(log);
        assert!(events.is_empty());
        assert!(buffs.pending.is_none());
        assert_eq!(buffs.slots.as_ref().unwrap()[&3].spell_id, 42);
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
            entity_id,
            slot: 3,
            spell_id,
            buff: replacement,
        }))
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
            [Progress::Submitted, Progress::ObservedRemoval]
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
    fn silence_and_echo_are_unresolved_with_no_retry_or_predicted_fade() {
        let (mut buffs, mut world, command) = fixture();
        let outcome = testing::run(|out| {
            buffs.handle(&command, &mut world, out)?;
            buffs.handle(&command, &mut world, out)?;
            buffs.observe(&update(7, 42, Some(buff(42))), &mut world, out)?;
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
