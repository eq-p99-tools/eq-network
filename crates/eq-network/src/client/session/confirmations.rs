//! The server's questions the player answers: a sacrifice, which takes the
//! player's life for the caster's gain, and a translocate, which moves them.
//! Resurrection and translocate carry admission-scoped identities. The
//! server still owns the outcome, and transfers follow its movement messages.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    confirmations::{
        ConsentClosure, ConsentId, ConsentKind, ConsentOffer, SacrificeOffer, TranslocateOffer,
    },
    message::Message,
    request::Request,
    world::WorldEvent,
};

use std::time::{Duration, Instant};

/// Titanium's five-minute consent window. `eqgame.exe` opens translocate at
/// 0x4561bd and resurrection at 0x459b61. The timer at 0x547f0a dismisses
/// with result 2 (0x547eff). Both callback branches send a decline
/// (translocate at 0x499ef3 and resurrection at 0x499f3a).
pub(super) const TITANIUM_TIMEOUT: Duration = Duration::from_secs(300);

/// The latest live question of one kind in a zone feature.
pub(super) struct Pending<T> {
    sequence: u64,
    current: Option<ConsentOffer<T>>,
}

impl<T> Default for Pending<T> {
    fn default() -> Self {
        Self {
            sequence: 0,
            current: None,
        }
    }
}

impl<T: Clone + PartialEq> Pending<T> {
    /// Registers a new question. A duplicate of the pending packet retains
    /// its identity and deadline.
    pub(super) fn open(
        &mut self,
        offer: &T,
        world: &World,
        kind: ConsentKind,
        deadline: Instant,
    ) -> Option<ConsentOffer<T>> {
        if !world.ready()
            || world.lifecycle.blocks_motion()
            || self
                .current
                .as_ref()
                .is_some_and(|current| current.offer == *offer)
        {
            return None;
        }
        self.sequence = self.sequence.checked_add(1)?;
        let question = ConsentOffer {
            id: ConsentId {
                session_id: world.session_id,
                kind,
                sequence: self.sequence,
            },
            offer: offer.clone(),
            expires_at: deadline,
        };
        self.current = Some(question.clone());
        Some(question)
    }

    /// Consumes the exact offer once. A stale answer leaves its replacement.
    pub(super) fn take(&mut self, id: ConsentId, world: &World) -> Option<T> {
        if id.session_id != world.session_id || world.lifecycle.blocks_motion() {
            return None;
        }
        self.current
            .take_if(|current| current.id == id)
            .map(|current| current.offer)
    }

    /// Discards consent across death, zoning and logout.
    pub(super) fn invalidate(
        &mut self,
        message: &Message,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let invalid = world.lifecycle.blocks_motion()
            || matches!(
                message,
                Message::ZoneAnswer(_) | Message::Handoff(_) | Message::LoggedOut
            )
            || matches!(message, Message::Event(event) if world.owns_death(event));
        if let Some(current) = self.current.take_if(|_| invalid) {
            closed(out, current.id, ConsentClosure::Interrupted)?;
        }
        Ok(())
    }

    /// Takes an elapsed question once, or interrupts it if the admission
    /// can no longer answer.
    pub(super) fn elapsed(
        &mut self,
        now: Instant,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<Option<ConsentOffer<T>>> {
        if world.lifecycle.blocks_motion() {
            if let Some(current) = self.current.take() {
                closed(out, current.id, ConsentClosure::Interrupted)?;
            }
            return Ok(None);
        }
        Ok(self.current.take_if(|current| now >= current.expires_at))
    }
}

pub(super) fn closed(out: &mut Out<'_, '_>, id: ConsentId, closure: ConsentClosure) -> Result<()> {
    out.log.send(ClientEvent::World(WorldEvent::ConsentClosed {
        id,
        closure,
    }))
}

/// Answers offers to sacrifice the player.
#[derive(Default)]
pub(super) struct Sacrifice {
    /// The offer waiting for an answer, if any.
    offer: Option<SacrificeOffer>,
}

impl Feature for Sacrifice {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Sacrifice]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::AnswerSacrifice { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::AnswerSacrifice { accept, .. } = *command else {
            return Ok(());
        };
        let Some(offer) = self.offer.take() else {
            return actions::refuse(command, "No sacrifice is waiting for an answer.", out.log);
        };
        out.request(&Request::AnswerSacrifice { offer, accept })
    }

    fn observe(
        &mut self,
        message: &Message,
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Message::Event(WorldEvent::SacrificeOffer(offer)) = message {
            self.offer = Some(offer.clone());
        }
        Ok(())
    }
}

/// Answers offers to translocate the player.
pub(super) struct Translocate {
    pending: Pending<TranslocateOffer>,
    timeout: Duration,
}

impl Translocate {
    pub(super) fn new(timeout: Duration) -> Self {
        Self {
            pending: Pending::default(),
            timeout,
        }
    }
}

impl Feature for Translocate {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Translocate]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::AnswerTranslocate { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::AnswerTranslocate {
            offer_id, accept, ..
        } = *command
        else {
            return Ok(());
        };
        self.tick(Instant::now(), world, out)?;
        let Some(offer) = self.pending.take(offer_id, world) else {
            return actions::refuse(
                command,
                "That translocate offer is no longer current.",
                out.log,
            );
        };
        out.request(&Request::AnswerTranslocate { offer, accept })?;
        closed(out, offer_id, ConsentClosure::AnswerSent { accept })
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.pending.invalidate(message, world, out)?;
        let Message::TranslocateOffer(offer) = message else {
            return Ok(());
        };
        if let Some(question) = self.pending.open(
            offer,
            world,
            ConsentKind::Translocate,
            Instant::now() + self.timeout,
        ) {
            out.log
                .send(ClientEvent::World(WorldEvent::TranslocateOffer(question)))?;
        }
        Ok(())
    }

    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if let Some(ConsentOffer { id, offer, .. }) = self.pending.elapsed(now, world, out)? {
            out.request(&Request::AnswerTranslocate {
                offer,
                accept: false,
            })?;
            closed(out, id, ConsentClosure::Expired)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::confirmations::{
        eqmac_translocate_answer, sacrifice, titanium_sacrifice_answer, titanium_translocate,
        titanium_translocate_answer,
    };

    #[test]
    fn a_sacrifice_is_answered_once_and_only_on_the_players_word() {
        let mut feature = Sacrifice::default();
        let mut world = World::new(5);
        let answer = |accept| ClientCommand::AnswerSacrifice {
            session_id: 5,
            accept,
        };
        let outcome = testing::run(|out| feature.handle(&answer(true), &mut world, out));
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::SacrificeRefused { .. })
        )));
        // A sacrifice from spawn 581 to the player, spawn 7.
        let body: Vec<u8> = [581u32, 7, 0]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let offer = sacrifice(&body).unwrap();
        let outcome = testing::run(|out| {
            feature.observe(
                &Message::Event(WorldEvent::SacrificeOffer(offer.clone())),
                &mut world,
                out,
            )
        });
        outcome.result.unwrap();
        // The offer alone sends nothing.
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        let outcome = testing::run(|out| feature.handle(&answer(false), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [titanium_sacrifice_answer(&offer, false)]);
        // The offer is spent.
        let outcome = testing::run(|out| feature.handle(&answer(true), &mut world, out));
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
    }

    fn translocate_record(zone: u32) -> Vec<u8> {
        let mut body = vec![0; 92];
        body[..4].copy_from_slice(&zone.to_le_bytes());
        body[12..18].copy_from_slice(b"Tester");
        body
    }

    fn answer(offer_id: ConsentId, accept: bool) -> ClientCommand {
        ClientCommand::AnswerTranslocate {
            session_id: 5,
            offer_id,
            accept,
        }
    }

    #[test]
    fn replacement_and_duplicate_translocates_require_the_current_identity() {
        let mut feature = Translocate::new(TITANIUM_TIMEOUT);
        let mut world = World::new(5);
        world.admitted = Some(Instant::now());
        let offer = |zone| titanium_translocate(&translocate_record(zone)).unwrap();
        let mut see = |offer: &TranslocateOffer| {
            let result = testing::run(|out| {
                feature.observe(&Message::TranslocateOffer(offer.clone()), &mut world, out)
            });
            result.result.unwrap();
            assert_eq!(
                result.sent,
                [] as [eq_network_game::command::EncodedCommand; 0]
            );
            result.events
        };
        let first = offer(4);
        let second = offer(5);
        let id = |events: Vec<ClientEvent>| match events.as_slice() {
            [ClientEvent::World(WorldEvent::TranslocateOffer(offer))] => offer.id,
            _ => panic!("expected a question"),
        };
        let older = id(see(&first));
        assert!(see(&first).is_empty());
        let current = id(see(&second));
        assert_ne!(older, current);
        let other_admission = ConsentId {
            session_id: 4,
            ..current
        };
        for stale in [older, other_admission] {
            let outcome = testing::run(|out| feature.handle(&answer(stale, true), &mut world, out));
            outcome.result.unwrap();
            assert_eq!(
                outcome.sent,
                [] as [eq_network_game::command::EncodedCommand; 0]
            );
            assert!(matches!(
                outcome.events.as_slice(),
                [
                    ClientEvent::World(WorldEvent::TranslocateRefused { session_id: 5, offer_id, .. }),
                    ClientEvent::Diagnostic(message),
                ] if *offer_id == stale
                    && message == "Rejected AnswerTranslocate command: That translocate offer is no longer current."
            ));
        }
        let result = testing::run_on(&super::super::wire::EqMac, |out| {
            feature.handle(&answer(current, false), &mut world, out)
        });
        result.result.unwrap();
        assert_eq!(result.sent, [eqmac_translocate_answer(&second, false)]);
        assert!(matches!(
            result.events.as_slice(),
            [ClientEvent::World(WorldEvent::ConsentClosed { id, closure: ConsentClosure::AnswerSent { accept: false } })] if *id == current
        ));
        let repeated = testing::run(|out| feature.handle(&answer(current, true), &mut world, out));
        repeated.result.unwrap();
        assert_eq!(
            repeated.sent,
            [] as [eq_network_game::command::EncodedCommand; 0]
        );
    }

    #[test]
    fn an_unanswered_titanium_translocate_declines_once_at_its_deadline() {
        let mut feature = Translocate::new(TITANIUM_TIMEOUT);
        let mut world = World::new(5);
        world.admitted = Some(Instant::now());
        let offer = titanium_translocate(&translocate_record(4)).unwrap();
        let opened =
            testing::run(|out| feature.observe(&Message::TranslocateOffer(offer), &mut world, out));
        opened.result.unwrap();
        assert!(opened.sent.is_empty());
        let [ClientEvent::World(WorldEvent::TranslocateOffer(question))] = opened.events.as_slice()
        else {
            panic!("expected the question");
        };
        let deadline = question.expires_at;
        let before = deadline - Duration::from_nanos(1);
        let early = testing::run(|out| feature.tick(before, &mut world, out));
        early.result.unwrap();
        assert!(early.sent.is_empty() && early.events.is_empty());

        let expired = testing::run(|out| feature.tick(deadline, &mut world, out));
        expired.result.unwrap();
        assert_eq!(
            expired.sent,
            [titanium_translocate_answer(&question.offer, false)]
        );
        assert!(matches!(
            expired.events.as_slice(),
            [ClientEvent::World(WorldEvent::ConsentClosed { id, closure: ConsentClosure::Expired })]
                if *id == question.id
        ));
        let repeated = testing::run(|out| feature.tick(deadline, &mut world, out));
        repeated.result.unwrap();
        assert!(repeated.sent.is_empty() && repeated.events.is_empty());

        let late = testing::run(|out| feature.handle(&answer(question.id, true), &mut world, out));
        late.result.unwrap();
        assert!(late.sent.is_empty());
        assert!(matches!(
            late.events.first(),
            Some(ClientEvent::World(WorldEvent::TranslocateRefused { offer_id, .. }))
                if *offer_id == question.id
        ));
    }

    #[test]
    fn death_or_transfer_discards_consent_even_if_the_zone_later_resumes() {
        let offer = titanium_translocate(&translocate_record(4)).unwrap();
        let now = Instant::now();
        let transfer = eq_network_game::zoning::ZoneOffer {
            zone_id: 8,
            instance_id: 0,
            position: eq_network_game::world::Position::default(),
            reason: 0,
            to_bind: false,
            solicited: false,
        };
        for interrupt in [
            Message::Event(WorldEvent::DeathBind { session_id: 5 }),
            Message::ZoneOffer(transfer),
        ] {
            let mut feature = Translocate::new(TITANIUM_TIMEOUT);
            let mut world = World::new(5);
            world.admitted = Some(now);
            let opened = testing::run(|out| {
                feature.observe(&Message::TranslocateOffer(offer.clone()), &mut world, out)
            });
            let [ClientEvent::World(WorldEvent::TranslocateOffer(question))] =
                opened.events.as_slice()
            else {
                panic!("expected the question");
            };
            if let Message::ZoneOffer(offer) = &interrupt {
                world.transfer_offered(offer.clone(), now).unwrap();
            }
            let reset = testing::run(|out| feature.observe(&interrupt, &mut world, out));
            reset.result.unwrap();
            assert_eq!(
                reset.sent,
                [] as [eq_network_game::command::EncodedCommand; 0]
            );
            assert!(matches!(
                reset.events.as_slice(),
                [ClientEvent::World(WorldEvent::ConsentClosed { id, closure: ConsentClosure::Interrupted })]
                    if *id == question.id
            ));
            if world.lifecycle.pending().is_some() {
                world.transfer_refused(now).unwrap();
            }
            let result =
                testing::run(|out| feature.handle(&answer(question.id, true), &mut world, out));
            result.result.unwrap();
            assert_eq!(
                result.sent,
                [] as [eq_network_game::command::EncodedCommand; 0]
            );
        }
    }
}
