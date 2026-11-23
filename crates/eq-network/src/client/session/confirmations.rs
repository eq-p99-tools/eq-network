//! The server's questions the player answers: a sacrifice, which takes the
//! player's life for the caster's gain, a translocate, which moves them,
//! and another player's challenge to a duel. Resurrection, translocate and
//! duel carry admission-scoped identities. The
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
    socials::{DuelQuestion, DuelUpdate},
    world::{SpawnKind, WorldEvent},
};

use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

/// Titanium's five-minute consent window. `eqgame.exe` opens translocate at
/// 0x4561bd and resurrection at 0x459b61. The timer at 0x547f0a dismisses
/// with result 2 (0x547eff). Both callback branches send a decline
/// (translocate at 0x499ef3 and resurrection at 0x499f3a).
pub(super) const TITANIUM_TIMEOUT: Duration = Duration::from_secs(300);

/// Bound the replay history without ever evicting an identity that could replay.
const MAX_ISSUED_CONSENT: usize = 1024;

/// The latest live question of one kind in a zone feature.
pub(super) struct Pending<T> {
    sequence: u64,
    current: Option<ConsentOffer<T>>,
    /// Exact P99 packets already issued as questions during this admission.
    issued_wire: BTreeSet<Box<[u8]>>,
}

impl<T> Default for Pending<T> {
    fn default() -> Self {
        Self {
            sequence: 0,
            current: None,
            issued_wire: BTreeSet::new(),
        }
    }
}

impl<T: Clone + PartialEq> Pending<T> {
    /// Registers a new question. A duplicate of the pending packet retains
    /// its identity and deadline. P99 supplies an exact wire identity because
    /// no established nonce distinguishes a replay from a fresh identical offer.
    pub(super) fn open(
        &mut self,
        offer: &T,
        world: &World,
        kind: ConsentKind,
        deadline: Instant,
        wire_identity: Option<&[u8]>,
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
        let next_sequence = self.sequence.checked_add(1)?;
        if let Some(bytes) = wire_identity {
            if self.issued_wire.len() >= MAX_ISSUED_CONSENT || self.issued_wire.contains(bytes) {
                return None;
            }
            self.issued_wire.insert(bytes.into());
        }
        self.sequence = next_sequence;
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
    pub(super) fn take(&mut self, id: ConsentId, world: &World) -> Option<ConsentOffer<T>> {
        if id.session_id != world.session_id || !world.ready() || world.lifecycle.blocks_motion() {
            return None;
        }
        self.current.take_if(|current| current.id == id)
    }

    /// Discards consent across death, zoning and logout.
    pub(super) fn invalidate(
        &mut self,
        message: &Message,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Some(current) = self.current.take_if(|_| interrupted(message, world)) {
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

/// Both open questions and accepted continuations lose their admission
/// context on any transfer or own death.
pub(super) fn interrupted(message: &Message, world: &World) -> bool {
    world.lifecycle.blocks_motion()
        || matches!(
            message,
            Message::ZoneOffer(_)
                | Message::ZoneAnswer(_)
                | Message::Handoff(_)
                | Message::LoggedOut
        )
        || matches!(message, Message::Event(event) if world.owns_death(event))
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
    p99: bool,
    /// One explicit Yes awaiting a matching received continuation.
    accepted: Option<ConsentOffer<TranslocateOffer>>,
}

impl Translocate {
    pub(super) fn new(timeout: Duration) -> Self {
        Self {
            pending: Pending::default(),
            timeout,
            p99: false,
            accepted: None,
        }
    }

    /// P99's Titanium continuation requires the same accepted offer.
    pub(super) fn project1999() -> Self {
        Self {
            p99: true,
            ..Self::new(TITANIUM_TIMEOUT)
        }
    }

    /// Follows one received completion for a still-current acceptance.
    fn complete(&mut self, completed: &TranslocateOffer, world: &mut World) {
        let Some(accepted) = self.accepted.as_ref() else {
            return;
        };
        let Some((_, position)) = world.player_at() else {
            return;
        };
        if Instant::now() >= accepted.expires_at || accepted.id.session_id != world.session_id {
            self.accepted = None;
            return;
        }
        if let Some(move_offer) =
            accepted
                .offer
                .p99_completion(completed, world.zone, position.heading)
        {
            self.accepted = None;
            world.happened(Message::ZoneOffer(move_offer));
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
        let Some(question) = self.pending.take(offer_id, world) else {
            return actions::refuse(
                command,
                "That translocate offer is no longer current.",
                out.log,
            );
        };
        out.request(&Request::AnswerTranslocate {
            offer: question.offer.clone(),
            accept,
        })?;
        if self.p99 && accept {
            self.accepted = Some(question);
        }
        closed(out, offer_id, ConsentClosure::AnswerSent { accept })
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.pending.invalidate(message, world, out)?;
        if interrupted(message, world) {
            self.accepted = None;
        }
        if let Message::TranslocateCompleted(completed) = message {
            if self.p99 {
                self.complete(completed, world);
            }
            return Ok(());
        }
        let Message::TranslocateOffer(offer) = message else {
            return Ok(());
        };
        if self.p99 && (self.accepted.is_some() || !offer.valid_p99_question()) {
            return Ok(());
        }
        if let Some(question) = self.pending.open(
            offer,
            world,
            ConsentKind::Translocate,
            Instant::now() + self.timeout,
            self.p99.then(|| offer.wire_identity()),
        ) {
            out.log
                .send(ClientEvent::World(WorldEvent::TranslocateOffer(question)))?;
        }
        Ok(())
    }

    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if world.lifecycle.blocks_motion()
            || self
                .accepted
                .as_ref()
                .is_some_and(|question| now >= question.expires_at)
        {
            self.accepted = None;
        }
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

/// How long a challenge the player sent is remembered with no answer:
/// ours, inferred (checklist 6.492b). `EQEmu` tells the challenger a
/// decline only in a line of text, so nothing else may end it; the
/// official program's own bound is pending R49-6h.
const CHALLENGE_REMEMBERED: Duration = Duration::from_secs(60);

/// The shortest wait between two challenges the player sends, so a held
/// hotbutton cannot send a stream of them: ours, inferred (checklist
/// 6.492b, pending R49-6h).
const CHALLENGE_PACE: Duration = Duration::from_secs(5);

/// A challenge the player sent and nothing has ended yet.
#[derive(Clone, Copy, Debug)]
struct Challenged {
    /// The player challenged.
    target: u16,
    /// When it went.
    sent: Instant,
}

/// Answers another player's challenge to a duel, once, on the player's
/// word, and sends the player's own challenge. The question has no local
/// deadline ([`DuelQuestion`](eq_network_game::socials::DuelQuestion)).
///
/// Every server passes the challenged player's acceptance back to the
/// challenger in the record a challenge comes in: `EQEmu`'s
/// `Handle_OP_DuelAccept` sends `OP_RequestDuel` naming the accepter before
/// `OP_DuelAccept`, and TAKP relays the accepter's own `OP_RequestDuel`.
/// So the challenge the player sent is remembered, and that record from the
/// one challenged opens no question and sends nothing; the duel begins
/// only on the server's `OP_DuelAccept`.
pub(super) struct Duel {
    sequence: u64,
    current: Option<DuelQuestion>,
    /// Whether a yes first sends the generation's own challenge back
    /// ([`Request::DuelRequest`]): the `EQMac` client's, by TAKP's note
    /// (order inferred, 6.492). No server type lists that yet.
    echoes_challenge: bool,
    /// The one challenge the player has out, if any.
    challenged: Option<Challenged>,
    /// When the player last sent a challenge.
    last_challenge: Option<Instant>,
}

impl Duel {
    /// A duel answered on a server type's wire; `echoes_challenge` as the
    /// field says.
    pub(super) const fn new(echoes_challenge: bool) -> Self {
        Self {
            sequence: 0,
            current: None,
            echoes_challenge,
            challenged: None,
            last_challenge: None,
        }
    }

    /// Closes the question waiting, if any matches, as interrupted.
    fn interrupt(
        &mut self,
        out: &mut Out<'_, '_>,
        matches: impl FnOnce(&DuelQuestion) -> bool,
    ) -> Result<()> {
        if let Some(current) = self.current.take_if(|current| matches(current)) {
            closed(out, current.id, ConsentClosure::Interrupted)?;
        }
        Ok(())
    }

    /// Forgets the player's challenge once its bound has passed or the one
    /// challenged has left the zone.
    fn forget_challenge(&mut self, now: Instant, world: &World) {
        self.challenged.take_if(|challenged| {
            now.saturating_duration_since(challenged.sent) >= CHALLENGE_REMEMBERED
                || world.spawns.get(challenged.target).is_none()
        });
    }

    /// Answers the question the player was asked.
    fn answer(
        &mut self,
        command: &ClientCommand,
        (offer_id, accept): (ConsentId, bool),
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let answerable = offer_id.session_id == world.session_id
            && world.ready()
            && !world.lifecycle.blocks_motion();
        let Some(question) = self
            .current
            .take_if(|current| answerable && current.id == offer_id)
        else {
            return actions::refuse(
                command,
                "That challenge to a duel is no longer current.",
                out.log,
            );
        };
        let challenger = question.challenger;
        if accept && self.echoes_challenge {
            out.request(&Request::DuelRequest { other: challenger })?;
        }
        out.request(&Request::AnswerDuel { challenger, accept })?;
        closed(out, offer_id, ConsentClosure::AnswerSent { accept })
    }

    /// Sends the player's challenge, only at a player in the zone other
    /// than themselves, one at a time and paced. `EQEmu`'s handler looks the
    /// one challenged up with no check that they exist, so nothing else
    /// may reach [`Request::DuelRequest`] as a challenge. One who leaves
    /// between this check and the server's read is the same race the
    /// official client has.
    fn challenge(
        &mut self,
        command: &ClientCommand,
        (session_id, target): (u64, u16),
        now: Instant,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        // Every line here is this client's, pending R49-6h. The server
        // says its own when the other player is considering another duel
        // or the player already accepted one (`DUEL_CONSIDERING`,
        // `DUEL_INPROGRESS`), so neither is checked here.
        let refusal = if session_id != world.session_id
            || !world.ready()
            || world.lifecycle.blocks_motion()
        {
            Some("You can't challenge anyone to a duel now.")
        } else if world.is_player(target) {
            Some("You can't challenge yourself to a duel.")
        } else if world
            .spawns
            .get(target)
            .is_none_or(|spawn| spawn.kind != SpawnKind::Player)
        {
            Some("You can only challenge another player to a duel.")
        } else if self.challenged.is_some() {
            Some("You have already challenged someone to a duel.")
        } else if self
            .last_challenge
            .is_some_and(|last| now.saturating_duration_since(last) < CHALLENGE_PACE)
        {
            Some("You must wait a moment before challenging again.")
        } else {
            None
        };
        if let Some(reason) = refusal {
            return actions::refuse(command, reason, out.log);
        }
        out.request(&Request::DuelRequest { other: target })?;
        self.challenged = Some(Challenged { target, sent: now });
        self.last_challenge = Some(now);
        Ok(())
    }
}

impl Feature for Duel {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Dueling]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::AnswerDuel { .. } | ClientCommand::ChallengeDuel { .. }
        )
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let now = Instant::now();
        self.tick(now, world, out)?;
        match *command {
            ClientCommand::AnswerDuel {
                offer_id, accept, ..
            } => self.answer(command, (offer_id, accept), world, out),
            ClientCommand::ChallengeDuel {
                session_id, target, ..
            } => self.challenge(command, (session_id, target), now, world, out),
            _ => Ok(()),
        }
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if interrupted(message, world) {
            self.interrupt(out, |_| true)?;
            self.challenged = None;
        }
        self.forget_challenge(Instant::now(), world);
        let Message::Event(WorldEvent::Duel(update)) = message else {
            return Ok(());
        };
        match *update {
            // The one the player challenged accepting, in the record a
            // challenge comes in: no question, and nothing sent. The duel
            // begins on the server's own word.
            DuelUpdate::Challenge { from }
                if self
                    .challenged
                    .is_some_and(|challenged| challenged.target == from) => {}
            DuelUpdate::Challenge { from } if !world.is_player(from) => {
                // The same challenger's record again keeps its question.
                if !world.ready()
                    || world.lifecycle.blocks_motion()
                    || self
                        .current
                        .is_some_and(|current| current.challenger == from)
                {
                    return Ok(());
                }
                let Some(sequence) = self.sequence.checked_add(1) else {
                    return Ok(());
                };
                self.sequence = sequence;
                // A newer challenge replaces one waiting, as the server
                // keeps only the latest.
                self.interrupt(out, |_| true)?;
                let question = DuelQuestion {
                    id: ConsentId {
                        session_id: world.session_id,
                        kind: ConsentKind::Duel,
                        sequence,
                    },
                    challenger: from,
                };
                self.current = Some(question);
                out.log
                    .send(ClientEvent::World(WorldEvent::DuelOffer(question)))?;
            }
            // The duel began, or the challenger's client called it off:
            // the challenge waiting from them is no longer a question, and
            // the player's own challenge to them is over.
            DuelUpdate::Accepted {
                challenger: from,
                accepter,
            } => {
                self.interrupt(out, |current| current.challenger == from)?;
                self.challenged
                    .take_if(|challenged| world.is_player(from) && challenged.target == accepter);
            }
            DuelUpdate::Refused { from, .. } => {
                self.interrupt(out, |current| current.challenger == from)?;
                self.challenged
                    .take_if(|challenged| challenged.target == from);
            }
            DuelUpdate::Challenge { .. } => (),
        }
        Ok(())
    }

    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if world.lifecycle.blocks_motion() {
            self.interrupt(out, |_| true)?;
            self.challenged = None;
        }
        self.forget_challenge(now, world);
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
    fn replay_history_saturation_never_evicts_an_issued_question() {
        let mut world = World::new(5);
        world.admitted = Some(Instant::now());
        let mut pending = Pending::<usize>::default();
        let deadline = Instant::now() + TITANIUM_TIMEOUT;
        for id in 0..MAX_ISSUED_CONSENT {
            assert!(pending
                .open(
                    &id,
                    &world,
                    ConsentKind::Resurrection,
                    deadline,
                    Some(&id.to_le_bytes())
                )
                .is_some());
        }
        let extra = MAX_ISSUED_CONSENT;
        assert!(pending
            .open(
                &extra,
                &world,
                ConsentKind::Resurrection,
                deadline,
                Some(&extra.to_le_bytes())
            )
            .is_none());
        assert!(pending
            .issued_wire
            .contains(0usize.to_le_bytes().as_slice()));
        assert_eq!(pending.current.as_ref().unwrap().offer, extra - 1);
    }

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
        assert_eq!(opened.sent, []);
        let [ClientEvent::World(WorldEvent::TranslocateOffer(question))] = opened.events.as_slice()
        else {
            panic!("expected the question");
        };
        let deadline = question.expires_at;
        let before = deadline
            .checked_sub(Duration::from_nanos(1))
            .expect("the fixture deadline has a preceding nanosecond");
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
        assert_eq!(late.sent, []);
        assert!(matches!(
            late.events.first(),
            Some(ClientEvent::World(WorldEvent::TranslocateRefused { offer_id, .. }))
                if *offer_id == question.id
        ));
    }

    #[test]
    fn p99_completion_requires_a_current_exact_yes_and_is_consumed_once() {
        use eq_network_game::confirmations::TRANSLOCATE_OPCODE;

        let message = |zone, answer: u32| {
            let mut body = translocate_record(zone);
            body[4..8].copy_from_slice(&2000u32.to_le_bytes());
            body[76..80].copy_from_slice(&12.0f32.to_le_bytes());
            body[80..84].copy_from_slice(&34.0f32.to_le_bytes());
            body[88..].copy_from_slice(&answer.to_le_bytes());
            eq_network_game::message::titanium(TRANSLOCATE_OPCODE, &body).remove(0)
        };
        let mut feature = Translocate::project1999();
        let mut world = World::new(5);
        let mut player = testing::player(7);
        player.position.heading = 64.0;
        world.player.admit(player);
        world.zone = (8, 0);
        world.admitted = Some(Instant::now());
        let complete = message(4, 1);
        testing::run(|out| feature.observe(&complete, &mut world, out))
            .result
            .unwrap();
        assert!(world.take_news().is_empty());

        let offered = testing::run(|out| feature.observe(&message(4, 0), &mut world, out));
        let [ClientEvent::World(WorldEvent::TranslocateOffer(question))] =
            offered.events.as_slice()
        else {
            panic!("expected the named question");
        };
        assert_eq!(offered.sent, []);
        let selected =
            testing::run(|out| feature.handle(&answer(question.id, true), &mut world, out));
        assert_eq!(
            selected.sent,
            [titanium_translocate_answer(&question.offer, true)]
        );
        assert!(world.take_news().is_empty());
        assert!(world.lifecycle.pending().is_none());

        testing::run(|out| feature.observe(&message(7, 1), &mut world, out))
            .result
            .unwrap();
        assert!(world.take_news().is_empty());
        testing::run(|out| feature.observe(&complete, &mut world, out))
            .result
            .unwrap();
        assert!(matches!(
            world.take_news().as_slice(),
            [Message::ZoneOffer(move_offer)] if move_offer.zone_id == 4
                && move_offer.reason == 8
                && move_offer.position.heading.to_bits() == 64.0_f32.to_bits()
                && move_offer.position.x.to_bits() == 34.0_f32.to_bits()
                && move_offer.position.y.to_bits() == 12.0_f32.to_bits()
        ));
        testing::run(|out| feature.observe(&complete, &mut world, out))
            .result
            .unwrap();
        assert!(world.take_news().is_empty());

        let replay = testing::run(|out| feature.observe(&message(4, 0), &mut world, out));
        replay.result.unwrap();
        assert!(replay.events.is_empty());
        let offered = testing::run(|out| feature.observe(&message(7, 0), &mut world, out));
        let [ClientEvent::World(WorldEvent::TranslocateOffer(question))] =
            offered.events.as_slice()
        else {
            panic!("expected a distinct offer");
        };
        let selected =
            testing::run(|out| feature.handle(&answer(question.id, true), &mut world, out));
        selected.result.unwrap();
        let expired = testing::run(|out| feature.tick(question.expires_at, &mut world, out));
        expired.result.unwrap();
        assert_eq!(expired.sent, []);
        testing::run(|out| feature.observe(&message(7, 1), &mut world, out))
            .result
            .unwrap();
        assert!(world.take_news().is_empty());
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

    fn duel_world() -> World {
        let mut world = World::new(5);
        world.admitted = Some(Instant::now());
        world.own_spawn = Some(7);
        world
    }

    fn duel(update: DuelUpdate) -> Message {
        Message::Event(WorldEvent::Duel(update))
    }

    fn duel_answer(offer_id: ConsentId, accept: bool) -> ClientCommand {
        ClientCommand::AnswerDuel {
            session_id: 5,
            offer_id,
            accept,
        }
    }

    /// The question one record opens, if it opens one.
    fn asked(feature: &mut Duel, world: &mut World, update: DuelUpdate) -> Option<DuelQuestion> {
        let outcome = testing::run(|out| feature.observe(&duel(update), world, out));
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        outcome.events.iter().find_map(|event| match event {
            ClientEvent::World(WorldEvent::DuelOffer(question)) => Some(*question),
            _ => None,
        })
    }

    #[test]
    fn a_challenge_is_asked_once_and_answered_once_on_the_players_word() {
        use eq_network_game::socials::{titanium_duel_accept, titanium_duel_decline};
        let mut feature = Duel::new(false);
        let mut world = duel_world();
        let question = asked(&mut feature, &mut world, DuelUpdate::Challenge { from: 12 })
            .expect("a challenge is a question");
        assert_eq!(question.challenger, 12);
        assert_eq!(question.id.kind, ConsentKind::Duel);
        // The same record again keeps the question and its identity.
        assert_eq!(
            asked(&mut feature, &mut world, DuelUpdate::Challenge { from: 12 }),
            None
        );
        // An answer for another admission sends nothing.
        let stale = ConsentId {
            session_id: 4,
            ..question.id
        };
        let outcome =
            testing::run(|out| feature.handle(&duel_answer(stale, true), &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::DuelRefused { offer_id, .. }) if *offer_id == stale
        )));
        let outcome =
            testing::run(|out| feature.handle(&duel_answer(question.id, false), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [titanium_duel_decline(12, 7)]);
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::ConsentClosed {
                id,
                closure: ConsentClosure::AnswerSent { accept: false }
            }) if *id == question.id
        )));
        // The question is spent.
        let outcome =
            testing::run(|out| feature.handle(&duel_answer(question.id, true), &mut world, out));
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        // A new challenge is a new question, and a yes accepts it.
        let next = asked(&mut feature, &mut world, DuelUpdate::Challenge { from: 12 }).unwrap();
        assert_ne!(next.id, question.id);
        let outcome =
            testing::run(|out| feature.handle(&duel_answer(next.id, true), &mut world, out));
        assert_eq!(outcome.sent, [titanium_duel_accept(12, 7)]);
    }

    #[test]
    fn a_challenge_waits_without_a_deadline_until_something_ends_it() {
        let mut feature = Duel::new(false);
        let mut world = duel_world();
        // The player's own record, as a server passes an acceptance back,
        // asks nothing.
        assert_eq!(
            asked(&mut feature, &mut world, DuelUpdate::Challenge { from: 7 }),
            None
        );
        let question = asked(&mut feature, &mut world, DuelUpdate::Challenge { from: 12 }).unwrap();
        // Long after, nothing declines on the player's behalf.
        let outcome = testing::run(|out| {
            feature.tick(Instant::now() + Duration::from_hours(24), &mut world, out)
        });
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert!(outcome.events.is_empty(), "events {:?}", outcome.events);
        // Another player's duel starting leaves it; the challenger's ends it.
        let close = |feature: &mut Duel, world: &mut World, update| {
            let outcome = testing::run(|out| feature.observe(&duel(update), world, out));
            outcome.result.unwrap();
            outcome.events
        };
        let other = DuelUpdate::Accepted {
            challenger: 30,
            accepter: 31,
        };
        assert!(close(&mut feature, &mut world, other).is_empty());
        let events = close(
            &mut feature,
            &mut world,
            DuelUpdate::Refused {
                from: 12,
                reason: eq_network_game::socials::DuelRefusal::Considering,
            },
        );
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::World(WorldEvent::ConsentClosed {
                id,
                closure: ConsentClosure::Interrupted
            })] if *id == question.id
        ));
        // A zone change ends one too.
        let question = asked(&mut feature, &mut world, DuelUpdate::Challenge { from: 12 }).unwrap();
        let outcome = testing::run(|out| feature.observe(&Message::LoggedOut, &mut world, out));
        outcome.result.unwrap();
        assert!(matches!(
            outcome.events.as_slice(),
            [ClientEvent::World(WorldEvent::ConsentClosed {
                id,
                closure: ConsentClosure::Interrupted
            })] if *id == question.id
        ));
        // A newer challenger replaces the one waiting.
        let first = asked(&mut feature, &mut world, DuelUpdate::Challenge { from: 12 }).unwrap();
        let outcome = testing::run(|out| {
            feature.observe(&duel(DuelUpdate::Challenge { from: 13 }), &mut world, out)
        });
        assert!(matches!(
            outcome.events.as_slice(),
            [
                ClientEvent::World(WorldEvent::ConsentClosed { id, closure: ConsentClosure::Interrupted }),
                ClientEvent::World(WorldEvent::DuelOffer(DuelQuestion { challenger: 13, .. })),
            ] if *id == first.id
        ));
    }

    /// A world with another player (12), a third (13), an NPC (20) and a
    /// player's corpse (21) in the zone.
    fn arena() -> World {
        use eq_network_game::world::SpawnKind;
        let mut world = duel_world();
        for (id, kind) in [
            (12, SpawnKind::Player),
            (13, SpawnKind::Player),
            (20, SpawnKind::Npc),
            (21, SpawnKind::PlayerCorpse),
        ] {
            world.spawns.insert(testing::spawn(id, kind));
        }
        world
    }

    fn challenge(target: u16) -> ClientCommand {
        ClientCommand::ChallengeDuel {
            session_id: 5,
            target,
            created: Instant::now(),
        }
    }

    /// What one challenge sends, and its refusal if it was refused.
    fn challenged(
        feature: &mut Duel,
        world: &mut World,
        target: u16,
    ) -> (
        Vec<eq_network_game::command::EncodedCommand>,
        Option<String>,
    ) {
        let outcome = testing::run(|out| feature.handle(&challenge(target), world, out));
        outcome.result.unwrap();
        let refused = outcome.events.iter().find_map(|event| match event {
            ClientEvent::World(WorldEvent::SocialRefused { reason, .. }) => Some(reason.clone()),
            _ => None,
        });
        (outcome.sent, refused)
    }

    /// Lets the pacing wait pass, as if the last challenge went long ago.
    fn rested(feature: &mut Duel) {
        feature.last_challenge = None;
    }

    #[test]
    fn a_challenge_goes_only_to_another_player_in_the_zone_one_at_a_time() {
        use eq_network_game::socials::titanium_duel_request;
        let mut feature = Duel::new(false);
        let mut world = arena();
        // The player, a spawn not in the zone, an NPC and a corpse: nothing
        // reaches the wire, since `EQEmu` looks the first word up unchecked.
        for target in [7, 99, 20, 21] {
            let (sent, refused) = challenged(&mut feature, &mut world, target);
            assert!(sent.is_empty(), "{target} sent {sent:?}");
            assert!(refused.is_some(), "{target} was not refused");
        }
        let (sent, refused) = challenged(&mut feature, &mut world, 12);
        assert_eq!(refused, None);
        assert_eq!(sent, [titanium_duel_request(12, 7)]);
        // One at a time, whoever the next is.
        rested(&mut feature);
        let (sent, refused) = challenged(&mut feature, &mut world, 13);
        assert!(sent.is_empty(), "sent {sent:?}");
        assert_eq!(
            refused.as_deref(),
            Some("You have already challenged someone to a duel.")
        );
        // Another admission's command sends nothing.
        let outcome = testing::run(|out| {
            feature.handle(
                &ClientCommand::ChallengeDuel {
                    session_id: 4,
                    target: 13,
                    created: Instant::now(),
                },
                &mut world,
                out,
            )
        });
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
    }

    #[test]
    fn the_acceptance_echo_answers_nothing_and_the_server_starts_the_duel() {
        let mut feature = Duel::new(false);
        let mut world = arena();
        assert!(challenged(&mut feature, &mut world, 12).1.is_none());
        // `EQEmu` sends the accepter's `OP_RequestDuel` first: no question,
        // no command, no event.
        let outcome = testing::run(|out| {
            feature.observe(&duel(DuelUpdate::Challenge { from: 12 }), &mut world, out)
        });
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert!(outcome.events.is_empty(), "events {:?}", outcome.events);
        assert!(feature.challenged.is_some());
        // A third player's challenge is still a question.
        let question = asked(&mut feature, &mut world, DuelUpdate::Challenge { from: 13 })
            .expect("another player's challenge asks");
        assert_eq!(question.challenger, 13);
        // The server's `OP_DuelAccept` begins the duel and ends the challenge.
        let outcome = testing::run(|out| {
            feature.observe(
                &duel(DuelUpdate::Accepted {
                    challenger: 7,
                    accepter: 12,
                }),
                &mut world,
                out,
            )
        });
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert!(feature.challenged.is_none());
        // A later challenge from them is a real one.
        assert!(asked(&mut feature, &mut world, DuelUpdate::Challenge { from: 12 }).is_some());
    }

    #[test]
    fn the_players_challenge_ends_on_its_bound_a_departure_a_refusal_or_a_zone() {
        let mut feature = Duel::new(false);
        let mut world = arena();
        let challenge_again = |feature: &mut Duel, world: &mut World| {
            rested(feature);
            let (sent, refused) = challenged(feature, world, 12);
            assert_eq!(refused, None);
            assert_eq!(sent.len(), 1);
            assert!(feature.challenged.is_some());
        };
        // Its bound, ours: still out just before, gone at it.
        challenge_again(&mut feature, &mut world);
        let sent = feature.challenged.unwrap().sent;
        let tick = |feature: &mut Duel, world: &mut World, at| {
            testing::run(|out| feature.tick(at, world, out))
                .result
                .unwrap();
        };
        tick(&mut feature, &mut world, sent + Duration::from_secs(59));
        assert!(feature.challenged.is_some());
        tick(&mut feature, &mut world, sent + CHALLENGE_REMEMBERED);
        assert!(feature.challenged.is_none());
        // The one challenged leaving the zone.
        challenge_again(&mut feature, &mut world);
        let mut gone = arena();
        gone.spawns = super::super::entities::Spawns::default();
        tick(&mut feature, &mut gone, Instant::now());
        assert!(feature.challenged.is_none());
        // Their client turning it down.
        challenge_again(&mut feature, &mut world);
        testing::run(|out| {
            feature.observe(
                &duel(DuelUpdate::Refused {
                    from: 12,
                    reason: eq_network_game::socials::DuelRefusal::Considering,
                }),
                &mut world,
                out,
            )
        })
        .result
        .unwrap();
        assert!(feature.challenged.is_none());
        // A zone change.
        challenge_again(&mut feature, &mut world);
        testing::run(|out| feature.observe(&Message::LoggedOut, &mut world, out))
            .result
            .unwrap();
        assert!(feature.challenged.is_none());
    }

    #[test]
    fn challenges_are_paced() {
        let mut feature = Duel::new(false);
        let mut world = arena();
        assert!(challenged(&mut feature, &mut world, 12).1.is_none());
        feature.challenged = None;
        let (sent, refused) = challenged(&mut feature, &mut world, 12);
        assert!(sent.is_empty(), "sent {sent:?}");
        assert_eq!(
            refused.as_deref(),
            Some("You must wait a moment before challenging again.")
        );
        // Once the wait has passed it goes.
        feature.last_challenge = Instant::now().checked_sub(CHALLENGE_PACE);
        assert!(challenged(&mut feature, &mut world, 12).1.is_none());
    }

    #[test]
    fn the_eqmac_yes_sends_its_challenge_back_first() {
        use eq_network_game::socials::{eqmac_duel_accept, eqmac_duel_decline, eqmac_duel_request};
        let mut feature = Duel::new(true);
        let mut world = duel_world();
        let question = asked(&mut feature, &mut world, DuelUpdate::Challenge { from: 12 }).unwrap();
        let outcome = testing::run_on(&super::super::wire::EqMac, |out| {
            feature.handle(&duel_answer(question.id, true), &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [eqmac_duel_request(12, 7), eqmac_duel_accept(12, 7)]
        );
        let question = asked(&mut feature, &mut world, DuelUpdate::Challenge { from: 12 }).unwrap();
        let outcome = testing::run_on(&super::super::wire::EqMac, |out| {
            feature.handle(&duel_answer(question.id, false), &mut world, out)
        });
        assert_eq!(outcome.sent, [eqmac_duel_decline(12, 7)]);
    }
}
