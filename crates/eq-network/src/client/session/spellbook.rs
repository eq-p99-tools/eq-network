//! The spellbook and the memorized spells: scribing scrolls, memorizing and
//! forgetting spells, and deleting or swapping book entries.
//!
//! Scribing and memorizing sit the player and wait first, as the game
//! requires, and anything that disturbs the player meanwhile cancels them. A
//! waiting request is checked again against the book and cursor when it goes
//! out, and each submitted change holds the book until it is answered or
//! retired unconfirmed.
mod consumption;
mod edits;

use super::{
    actions::Resource,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::{bail, ensure, Context, Result};
use consumption::ScribeConsumption;
use edits::BookEdits;
use eq_network_game::{
    command::Posture,
    inventory::{Inventory, InventoryUpdate},
    message::Message,
    request::Request,
    spells::{self, BookActionStatus, SpellBook, SpellUpdate},
    world::{PostureState, WorldEvent},
};
use std::time::{Duration, Instant};

/// How long the player sits before a scribe or memorization goes out; not yet
/// measured against a live client.
const SITTING: Duration = Duration::from_secs(5);

/// What a scribe or memorization will ask for once the player has sat.
enum BookIntent {
    Memorize {
        gem: u8,
        spell_id: u32,
    },
    Scribe {
        revision: u64,
        slot: u16,
        spell_id: u32,
    },
}

/// A scribe or memorization waiting for the player to sit.
struct PendingBookAction {
    started: Instant,
    intent: BookIntent,
    requires_sitting: bool,
    delay: Duration,
}

impl PendingBookAction {
    /// Whether the player has sat long enough.
    fn ready(&self, now: Instant) -> bool {
        now.checked_duration_since(self.started)
            .is_some_and(|elapsed| elapsed >= self.delay)
    }

    /// The request, checked against the book and cursor as they are now
    /// rather than as they were when the player asked.
    fn request(&self, book: &SpellBook, inventory: &Inventory) -> Result<Request> {
        Ok(match self.intent {
            BookIntent::Memorize { gem, spell_id } => {
                book.check_memorize(gem, spell_id)?;
                Request::Memorize { gem, spell_id }
            }
            BookIntent::Scribe {
                revision,
                slot,
                spell_id,
            } => {
                book.check_scribe(inventory, revision, slot, spell_id)?;
                Request::Scribe { slot, spell_id }
            }
        })
    }
}

/// A book entry deleted, or two swapped, checked against the book as it is.
fn edit_request(command: &ClientCommand, book: Option<&SpellBook>, busy: bool) -> Result<Request> {
    ensure!(!busy, "book edit is busy");
    let book = book.context("spellbook unavailable")?;
    Ok(match *command {
        ClientCommand::DeleteSpell { slot, spell_id, .. } => {
            book.check_delete(slot, spell_id)?;
            Request::DeleteSpell { slot }
        }
        ClientCommand::SwapSpell {
            from,
            to,
            from_spell,
            to_spell,
            ..
        } => {
            book.check_swap(from, to, from_spell, to_spell)?;
            Request::SwapSpells { from, to }
        }
        _ => bail!("not a spellbook edit"),
    })
}

/// Tells the host how a spellbook change stands.
fn report(status: BookActionStatus, out: &mut Out<'_, '_>) -> Result<()> {
    out.log
        .send(ClientEvent::World(WorldEvent::BookAction(status)))
}

/// The player's spellbook and the changes to it in flight.
#[derive(Default)]
pub(super) struct Spellbook {
    /// What the server type lets the player do to the book's entries, as
    /// checked on that kind of server.
    edits: Edits,
    memorize_without_sitting: bool,
    /// The book as the server last described it.
    book: Option<SpellBook>,
    /// A scribe or memorization waiting for the player to sit.
    pending: Option<PendingBookAction>,
    /// The change waiting for the server's answer.
    answers: BookEdits,
    /// A scribe whose scroll the server takes from the cursor.
    consumption: ScribeConsumption,
}

/// What a server type lets the player do to the spellbook's entries
/// besides scribing them: each only once it has been checked on that kind
/// of server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Edits {
    /// Deleting a spell from the book.
    pub(super) deleting: bool,
    /// Moving a spell to another place in the book.
    pub(super) moving: bool,
}

impl Spellbook {
    pub(super) fn new(edits: Edits) -> Self {
        Self {
            edits,
            ..Self::default()
        }
    }

    /// Sits the player to scribe or memorize, once the request holds against
    /// the book (and, for a scroll, the cursor) as they are now.
    fn prepare(
        &mut self,
        intent: BookIntent,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let (unavailable, starting, refused) = match intent {
            BookIntent::Memorize { .. } => (
                "memorization is unavailable",
                "Memorizing spell",
                "memorization",
            ),
            BookIntent::Scribe { .. } => ("scribing is unavailable", "Scribing scroll", "scribing"),
        };
        let fast = self.memorize_without_sitting && matches!(intent, BookIntent::Memorize { .. });
        let pending = PendingBookAction {
            started: Instant::now(),
            requires_sitting: !fast,
            delay: if fast { Duration::ZERO } else { SITTING },
            intent,
        };
        let checked = self
            .book
            .as_ref()
            .filter(|_| self.pending.is_none())
            .context(unavailable)
            .and_then(|book| pending.request(book, &world.inventory));
        if let Err(error) = checked {
            report(BookActionStatus::Rejected(error.to_string()), out)?;
            return out.log.diagnostic(format!("Rejected {refused}: {error}"));
        }
        let Some(spawn_id) = world.player.as_ref().map(|player| player.spawn_id) else {
            return Ok(());
        };
        if pending.requires_sitting {
            world.posture.set(spawn_id, Posture::Sitting, out)?;
        }
        self.pending = Some(pending);
        report(BookActionStatus::Preparing, out)?;
        if fast {
            self.submit(Instant::now(), world, out)
        } else {
            out.log
                .diagnostic(format!("{starting}; movement or posture changes cancel it"))
        }
    }

    /// Deletes or swaps book entries at once.
    fn edit(&mut self, command: &ClientCommand, out: &mut Out<'_, '_>) -> Result<()> {
        let status = match edit_request(command, self.book.as_ref(), self.pending.is_some()) {
            Ok(request) => {
                out.request(&request)?;
                self.answers.sent(command, Instant::now());
                BookActionStatus::AwaitingReply
            }
            Err(error) => BookActionStatus::Rejected(error.to_string()),
        };
        report(status, out)
    }

    /// Requests a gem clear and holds the book until the matching update.
    fn forget(
        &mut self,
        command: &ClientCommand,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::ForgetSpell { gem, spell_id, .. } = *command else {
            return Ok(());
        };
        let checked = world
            .player
            .as_ref()
            .context("forget request is unavailable")
            .and_then(|player| spells::check_forget(&player.memorized_spells, gem, spell_id));
        match checked {
            Ok(()) => {
                out.request(&Request::Forget { gem, spell_id })?;
                self.answers.sent(command, Instant::now());
                report(BookActionStatus::Submitted, out)
            }
            Err(error) => {
                report(BookActionStatus::Rejected(error.to_string()), out)?;
                out.log
                    .diagnostic(format!("Rejected forgetting spell: {error}"))
            }
        }
    }

    /// Sends the scribe or memorization the player has sat long enough for.
    fn submit(&mut self, now: Instant, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| world.control.input_refusal(pending.started).is_some())
        {
            return self.cancel("Stun state changed during preparation", out);
        }
        let Some(pending) = self.pending.take_if(|pending| pending.ready(now)) else {
            return Ok(());
        };
        let request = self
            .book
            .as_ref()
            .context("spellbook unavailable")
            .and_then(|book| pending.request(book, &world.inventory));
        match request {
            Ok(request) => {
                out.request(&request)?;
                self.answers.prepared_sent(&pending.intent, now);
                self.consumption.sent(&pending.intent);
                report(BookActionStatus::AwaitingReply, out)
            }
            Err(error) => {
                report(BookActionStatus::Cancelled(error.to_string()), out)?;
                out.log
                    .diagnostic(format!("Cancelled spellbook action: {error}"))
            }
        }
    }

    /// Follows the server's changes to the book and gems, settling the change
    /// each one answers; the player starting a cast cancels a scribe or
    /// memorization still waiting.
    fn spell(
        &mut self,
        update: &SpellUpdate,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let answer = self.answers.observe(update);
        self.consumption
            .observe(update, answer == Some(BookActionStatus::Confirmed));
        // The character feature keeps the gems on the player's record.
        if let Some(book) = self.book.as_mut() {
            book.apply(update);
        }
        if let Some(status) = answer {
            report(status, out)?;
        }
        if matches!(update, SpellUpdate::Began { caster_id, .. } if world.is_player(*caster_id)) {
            self.cancel("Casting started", out)?;
        }
        Ok(())
    }

    /// Drops a scribe or memorization still waiting to go out, telling the
    /// host why.
    fn cancel(&mut self, reason: &str, out: &mut Out<'_, '_>) -> Result<()> {
        if self.pending.take().is_none() {
            return Ok(());
        }
        report(BookActionStatus::Cancelled(reason.into()), out)
    }
}

impl Feature for Spellbook {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        let mut offered = vec![Capability::Spellbook];
        if self.edits.deleting {
            offered.push(Capability::DeletingSpells);
        }
        if self.edits.moving {
            offered.push(Capability::MovingSpells);
        }
        offered
    }

    /// The book arrives with the player's profile, before the zone admits them.
    fn admit(&mut self, message: &Message, _world: &mut World) -> Result<()> {
        if let Message::Event(WorldEvent::SpellBook(book)) = message {
            self.book = Some(book.clone());
        }
        Ok(())
    }

    fn admitted(&mut self, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        match &self.book {
            Some(book) => out
                .log
                .send(ClientEvent::World(WorldEvent::SpellBook(book.clone()))),
            None => Ok(()),
        }
    }

    /// A scroll being scribed holds the inventory, since the server takes it
    /// from the cursor when it answers; any change in flight holds the book.
    fn holds(&self, _world: &World, now: Instant) -> Vec<(Resource, &'static str)> {
        let mut held = Vec::new();
        let awaiting_cursor = self.consumption.awaiting_cursor(now);
        let scribing = self.answers.scribing()
            || awaiting_cursor
            || self.consumption.unconfirmed()
            || self
                .pending
                .as_ref()
                .is_some_and(|pending| matches!(pending.intent, BookIntent::Scribe { .. }));
        if scribing {
            // Moving items meanwhile desynchronized P99's inventory and
            // logged the character out.
            held.push((Resource::Inventory, "Wait for scribing to finish"));
        }
        if awaiting_cursor || self.pending.is_some() || self.answers.outstanding() {
            held.push((Resource::Spellbook, "Wait for the current spellbook change"));
        }
        held
    }

    /// Moving, casting or changing posture cancels a scribe or memorization
    /// still waiting for the player to sit.
    fn notice(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if matches!(
            command,
            ClientCommand::Move(_)
                | ClientCommand::CastSpell { .. }
                | ClientCommand::UseItem(_)
                | ClientCommand::SetPosture { .. }
        ) {
            self.cancel("Movement, casting or posture changed", out)?;
        }
        Ok(())
    }

    /// Deleting and moving a book's spells only where the server type has
    /// them checked; the zone session refuses them elsewhere.
    fn owns(&self, command: &ClientCommand) -> bool {
        match command {
            ClientCommand::ScribeSpell { .. }
            | ClientCommand::MemorizeSpell { .. }
            | ClientCommand::ForgetSpell { .. }
            | ClientCommand::SetBookOptions { .. } => true,
            ClientCommand::DeleteSpell { .. } => self.edits.deleting,
            ClientCommand::SwapSpell { .. } => self.edits.moving,
            _ => false,
        }
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if self.answers.ambiguous(command) {
            report(
                BookActionStatus::Rejected(
                    "An earlier edit could have the same reply. Re-enter the zone to retry this edit."
                        .into(),
                ),
                out,
            )?;
            return out.log.diagnostic(format!(
                "Rejected {}: an earlier unanswered edit could have the same reply",
                command.name()
            ));
        }
        if let ClientCommand::MemorizeSpell { created, .. }
        | ClientCommand::ScribeSpell { created, .. } = command
        {
            if let Some(reason) = world.control.input_refusal(*created) {
                return report(BookActionStatus::Rejected(reason.into()), out);
            }
        }
        match *command {
            ClientCommand::SetBookOptions {
                session_id,
                memorize_without_sitting,
            } => {
                if session_id == world.session_id
                    && self.memorize_without_sitting != memorize_without_sitting
                {
                    self.cancel("Spellbook preparation option changed", out)?;
                    self.memorize_without_sitting = memorize_without_sitting;
                }
                Ok(())
            }
            ClientCommand::ScribeSpell {
                revision,
                slot,
                spell_id,
                ..
            } => self.prepare(
                BookIntent::Scribe {
                    revision,
                    slot,
                    spell_id,
                },
                world,
                out,
            ),
            ClientCommand::MemorizeSpell { gem, spell_id, .. } => {
                self.prepare(BookIntent::Memorize { gem, spell_id }, world, out)
            }
            ClientCommand::ForgetSpell { .. } => self.forget(command, world, out),
            _ => self.edit(command, out),
        }
    }

    /// Reports an unanswered edit, cancels unsent work on death or zoning,
    /// and submits an eligible prepared request.
    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        // Silence ends this local wait. Only distinguishable requests can
        // take a new reply, and cursor custody has its own hold.
        if let Some(status) = self.answers.timeout(now) {
            report(status, out)?;
        }
        if world.lifecycle.is_dead() {
            self.cancel("Character died", out)
        } else if world.lifecycle.pending().is_some() {
            self.cancel("Zone transfer started", out)
        } else {
            self.submit(now, world, out)
        }
    }

    /// The server's changes to the book and gems, and whatever the server
    /// does to the player that cancels a scribe or memorization still waiting.
    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match message {
            Message::Event(WorldEvent::SpellBook(book)) => {
                self.book = Some(book.clone());
                Ok(())
            }
            Message::Event(WorldEvent::Spell(update)) => self.spell(update, world, out),
            Message::Event(WorldEvent::Posture { spawn_id, posture })
                if world.is_player(*spawn_id)
                    && *posture != PostureState::Sitting
                    && self
                        .pending
                        .as_ref()
                        .is_some_and(|pending| pending.requires_sitting) =>
            {
                self.cancel("Server changed character posture", out)
            }
            Message::Event(WorldEvent::Position { spawn_id, .. }) if world.is_player(*spawn_id) => {
                self.cancel("Server corrected character position", out)
            }
            Message::Event(event) if world.owns_death(event) => self.cancel("Character died", out),
            Message::ZoneOffer(offer) if offer.local_position(world.zone).is_some() => {
                self.cancel("Server relocated character", out)
            }
            _ => Ok(()),
        }
    }

    /// A confirmed scribe's cursor removal is the server using up the scroll.
    fn explain(&mut self, message: &mut Message, world: &World) {
        if let Message::Event(WorldEvent::Inventory(update)) = message {
            let received = std::mem::replace(update, InventoryUpdate::Invalidated);
            *update = self.consumption.reconcile(&world.inventory, received);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{actions::Held, feature::testing};
    use super::*;
    use eq_network_game::{inventory::InventorySlot, world::Position, zoning};

    /// A book with spell 73 in its first slot, or an empty one.
    fn book(spell: Option<u32>) -> SpellBook {
        let mut profile = vec![0; 19592];
        if let Some(spell) = spell {
            profile[2312..2316].copy_from_slice(&spell.to_le_bytes());
        }
        SpellBook::titanium_profile(&profile).unwrap()
    }

    /// The spellbook of admitted player 7.
    fn admitted(spell: Option<u32>) -> (Spellbook, World) {
        let mut spellbook = Spellbook::default();
        let mut world = World::new(5);
        spellbook
            .admit(
                &Message::Event(WorldEvent::SpellBook(book(spell))),
                &mut world,
            )
            .unwrap();
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        (spellbook, world)
    }

    fn memorize(gem: u8) -> ClientCommand {
        ClientCommand::MemorizeSpell {
            session_id: 5,
            gem,
            spell_id: 73,
            created: Instant::now(),
        }
    }

    /// Player 7 sitting to memorize spell 73 into gem 2.
    fn memorizing() -> (Spellbook, World) {
        let (mut spellbook, mut world) = admitted(Some(73));
        testing::run(|out| spellbook.handle(&memorize(2), &mut world, out))
            .result
            .unwrap();
        (spellbook, world)
    }

    /// The spellbook statuses among the host's events.
    fn statuses(events: &[ClientEvent]) -> Vec<&BookActionStatus> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::BookAction(status)) => Some(status),
                _ => None,
            })
            .collect()
    }

    fn cancelled(reason: &str) -> BookActionStatus {
        BookActionStatus::Cancelled(reason.into())
    }

    #[test]
    fn deleting_and_moving_are_taken_and_offered_only_where_the_server_type_has_them() {
        use crate::world::Capability;
        let delete = ClientCommand::DeleteSpell {
            session_id: 1,
            slot: 0,
            spell_id: 73,
            created: Instant::now(),
        };
        let swap = ClientCommand::SwapSpell {
            session_id: 1,
            from: 0,
            to: 1,
            from_spell: 73,
            to_spell: None,
            created: Instant::now(),
        };
        let unchecked = Spellbook::new(Edits::default());
        assert!(!unchecked.owns(&delete));
        assert!(!unchecked.owns(&swap));
        assert_eq!(unchecked.capabilities(), [Capability::Spellbook]);
        let deleting = Spellbook::new(Edits {
            deleting: true,
            moving: false,
        });
        assert!(deleting.owns(&delete));
        assert!(!deleting.owns(&swap));
        assert_eq!(
            deleting.capabilities(),
            [Capability::Spellbook, Capability::DeletingSpells]
        );
        let both = Spellbook::new(Edits {
            deleting: true,
            moving: true,
        });
        assert!(both.owns(&delete) && both.owns(&swap));
        assert_eq!(
            both.capabilities(),
            [
                Capability::Spellbook,
                Capability::DeletingSpells,
                Capability::MovingSpells
            ]
        );
    }

    #[test]
    fn memorizing_sits_the_player_and_asks_once_they_have_sat_long_enough() {
        let (mut spellbook, mut world) = admitted(Some(73));
        let outcome = testing::run(|out| spellbook.handle(&memorize(2), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [eq_network_game::command::titanium_posture(7, Posture::Sitting).unwrap()]
        );
        assert_eq!(statuses(&outcome.events), [&BookActionStatus::Preparing]);
        assert_eq!(
            spellbook.holds(&world, Instant::now()),
            [(Resource::Spellbook, "Wait for the current spellbook change")]
        );
        let sat = Instant::now();
        let outcome = testing::run(|out| spellbook.tick(sat, &mut world, out));
        outcome.result.unwrap();
        assert!(
            outcome.sent.is_empty(),
            "expected nothing sent, got {:?}",
            outcome.sent
        );
        let outcome = testing::run(|out| spellbook.tick(sat + SITTING, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, spells::MEMORIZE_OPCODE);
        assert_eq!(
            statuses(&outcome.events),
            [&BookActionStatus::AwaitingReply]
        );
        // The server's answer releases the book; the character feature fills
        // the gem.
        let answer = Message::Event(WorldEvent::Spell(SpellUpdate::Slot {
            slot: 2,
            spell_id: 73,
            mode: 1,
        }));
        let outcome = testing::run(|out| spellbook.observe(&answer, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(statuses(&outcome.events), [&BookActionStatus::Confirmed]);
        assert!(
            spellbook.holds(&world, Instant::now()).is_empty(),
            "a hold remains: {:?}",
            spellbook.holds(&world, Instant::now())
        );
    }

    #[test]
    fn takps_256_slot_book_memorizes_in_takps_own_packets() {
        let eqmac = &super::super::wire::EqMac;
        // TAKP's unpacked profile, spell 73 in the book's first slot.
        let mut profile = vec![0; 8460];
        profile[1846..1848].copy_from_slice(&73i16.to_le_bytes());
        let book = SpellBook::eqmac_profile(&profile).unwrap();
        assert_eq!(book.slots().len(), 256);
        let mut spellbook = Spellbook::default();
        let mut world = World::new(5);
        spellbook
            .admit(&Message::Event(WorldEvent::SpellBook(book)), &mut world)
            .unwrap();
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        let outcome = testing::run_on(eqmac, |out| spellbook.handle(&memorize(2), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [eq_network_game::quarm::posture(7, Posture::Sitting).unwrap()]
        );
        let sat = Instant::now();
        let outcome = testing::run_on(eqmac, |out| spellbook.tick(sat + SITTING, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [spells::eqmac_memorize(2, 73)]);
        // TAKP answers with the same packet, which releases the book.
        let answers =
            eq_network_game::message::eqmac(spells::EQMAC_MEMORIZE_OPCODE, &outcome.sent[0].body);
        assert_eq!(answers.len(), 1);
        let outcome = testing::run_on(eqmac, |out| spellbook.observe(&answers[0], &mut world, out));
        outcome.result.unwrap();
        assert_eq!(statuses(&outcome.events), [&BookActionStatus::Confirmed]);
        assert_eq!(
            spellbook.holds(&world, Instant::now()),
            Vec::<(Resource, &str)>::new()
        );
    }

    #[test]
    fn a_request_the_book_cannot_take_is_refused_without_sitting() {
        let (mut spellbook, mut world) = admitted(None);
        let outcome = testing::run(|out| spellbook.handle(&memorize(2), &mut world, out));
        outcome.result.unwrap();
        assert!(
            outcome.sent.is_empty(),
            "expected nothing sent, got {:?}",
            outcome.sent
        );
        assert!(matches!(
            statuses(&outcome.events)[..],
            [BookActionStatus::Rejected(_)]
        ));
        assert!(
            spellbook.holds(&world, Instant::now()).is_empty(),
            "a hold remains: {:?}",
            spellbook.holds(&world, Instant::now())
        );
    }

    #[test]
    fn the_player_moving_casting_or_standing_cancels_a_waiting_request_once() {
        let created = Instant::now();
        for command in [
            ClientCommand::SetPosture {
                session_id: 5,
                spawn_id: 7,
                posture: Posture::Standing,
                created,
            },
            ClientCommand::CastSpell {
                session_id: 5,
                gem: 0,
                spell_id: 202,
                target_id: 7,
                created,
            },
        ] {
            let (mut spellbook, mut world) = memorizing();
            for expected in [
                vec![cancelled("Movement, casting or posture changed")],
                vec![],
            ] {
                let outcome = testing::run(|out| spellbook.notice(&command, &mut world, out));
                outcome.result.unwrap();
                assert_eq!(
                    statuses(&outcome.events),
                    expected.iter().collect::<Vec<_>>()
                );
            }
        }
        // Commands that leave the player seated change nothing.
        let (mut spellbook, mut world) = memorizing();
        let target = ClientCommand::SelectTarget {
            request_id: 1,
            session_id: 5,
            spawn_id: Some(9),
            expected_pet: None,
        };
        let outcome = testing::run(|out| spellbook.notice(&target, &mut world, out));
        assert!(
            statuses(&outcome.events).is_empty(),
            "expected no status, got {:?}",
            statuses(&outcome.events)
        );
    }

    #[test]
    fn the_server_disturbing_the_player_cancels_a_waiting_request() {
        let player = |event| Message::Event(event);
        let relocation = Message::ZoneOffer(zoning::ZoneOffer {
            zone_id: 2,
            instance_id: 0,
            position: Position::default(),
            reason: 0,
            to_bind: false,
            solicited: true,
        });
        for (message, reason) in [
            (
                player(WorldEvent::Posture {
                    spawn_id: 7,
                    posture: PostureState::Standing,
                }),
                "Server changed character posture",
            ),
            (
                player(WorldEvent::Position {
                    spawn_id: 7,
                    position: Position::default(),
                    velocity: [0.0; 3],
                    forward: 0.0,
                }),
                "Server corrected character position",
            ),
            (
                player(WorldEvent::Death(zoning::Death {
                    spawn_id: 7,
                    killer_id: 0,
                    corpse_id: 8,
                    bind_zone_id: 2,
                    corpse_name: None,
                })),
                "Character died",
            ),
            (
                player(WorldEvent::Spell(SpellUpdate::Began {
                    caster_id: 7,
                    spell_id: 202,
                    duration_ms: 0,
                })),
                "Casting started",
            ),
            (relocation, "Server relocated character"),
        ] {
            let (mut spellbook, mut world) = memorizing();
            world.zone = (2, 0);
            let outcome = testing::run(|out| spellbook.observe(&message, &mut world, out));
            outcome.result.unwrap();
            assert_eq!(statuses(&outcome.events), [&cancelled(reason)], "{reason}");
            assert!(
                spellbook.holds(&world, Instant::now()).is_empty(),
                "a hold remains for {reason}: {:?}",
                spellbook.holds(&world, Instant::now())
            );
        }
        // Others standing, and the player sitting, leave the request waiting.
        let (mut spellbook, mut world) = memorizing();
        for (spawn_id, posture) in [(8, PostureState::Standing), (7, PostureState::Sitting)] {
            let message = player(WorldEvent::Posture { spawn_id, posture });
            let outcome = testing::run(|out| spellbook.observe(&message, &mut world, out));
            assert!(
                statuses(&outcome.events).is_empty(),
                "expected no status for {posture:?}, got {:?}",
                statuses(&outcome.events)
            );
        }
        assert!(
            !spellbook.holds(&world, Instant::now()).is_empty(),
            "another spawn's posture must not release the hold"
        );
    }

    #[test]
    fn dying_or_zoning_cancels_a_waiting_request_before_it_goes_out() {
        let later = Instant::now() + SITTING;
        let (mut spellbook, mut world) = memorizing();
        world.lifecycle.mark_dead();
        let outcome = testing::run(|out| spellbook.tick(later, &mut world, out));
        outcome.result.unwrap();
        assert!(
            outcome.sent.is_empty(),
            "expected nothing sent, got {:?}",
            outcome.sent
        );
        assert_eq!(statuses(&outcome.events), [&cancelled("Character died")]);

        let (mut spellbook, mut world) = memorizing();
        world
            .lifecycle
            .offer(
                zoning::ZoneOffer {
                    zone_id: 3,
                    instance_id: 0,
                    position: Position::default(),
                    reason: 0,
                    to_bind: false,
                    solicited: true,
                },
                Instant::now(),
            )
            .unwrap();
        let outcome = testing::run(|out| spellbook.tick(later, &mut world, out));
        outcome.result.unwrap();
        assert!(
            outcome.sent.is_empty(),
            "expected nothing sent, got {:?}",
            outcome.sent
        );
        assert_eq!(
            statuses(&outcome.events),
            [&cancelled("Zone transfer started")]
        );
    }

    #[test]
    fn a_scribed_scroll_holds_the_inventory_until_the_server_uses_it_up() {
        let (mut spellbook, mut world) = admitted(None);
        world.inventory = consumption::tests::scroll_on_cursor().into();
        let scribe = ClientCommand::ScribeSpell {
            session_id: 5,
            revision: world.inventory.revision(),
            slot: 2,
            spell_id: 73,
            created: Instant::now(),
        };
        testing::run(|out| spellbook.handle(&scribe, &mut world, out))
            .result
            .unwrap();
        let held = Held::new(spellbook.holds(&world, Instant::now()));
        let pickup = ClientCommand::PickUp {
            session_id: 5,
            drop_id: 71,
            created: Instant::now(),
        };
        assert_eq!(held.conflict(&pickup), Some("Wait for scribing to finish"));
        assert_eq!(
            held.conflict(&memorize(0)),
            Some("Wait for the current spellbook change")
        );
        let outcome = testing::run(|out| spellbook.tick(Instant::now() + SITTING, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        let answer = Message::Event(WorldEvent::Spell(SpellUpdate::Slot {
            slot: 2,
            spell_id: 73,
            mode: 0,
        }));
        testing::run(|out| spellbook.observe(&answer, &mut world, out))
            .result
            .unwrap();
        // Answered, but the scroll is still on the cursor until the server
        // takes it.
        let held = Held::new(spellbook.holds(&world, Instant::now()));
        assert_eq!(held.conflict(&pickup), Some("Wait for scribing to finish"));
        let mut removal = Message::Event(WorldEvent::Inventory(InventoryUpdate::Remove(
            InventorySlot(30),
        )));
        spellbook.explain(&mut removal, &world);
        assert!(matches!(
            removal,
            Message::Event(WorldEvent::Inventory(InventoryUpdate::Consume {
                slot: InventorySlot(30),
                charge: false
            }))
        ));
        assert!(
            spellbook.holds(&world, Instant::now()).is_empty(),
            "a hold remains: {:?}",
            spellbook.holds(&world, Instant::now())
        );
    }

    #[test]
    fn an_unconfirmed_scribe_releases_the_book_but_preserves_cursor_custody() {
        let (mut spellbook, mut world) = admitted(Some(42));
        world.inventory = consumption::tests::scroll_on_cursor().into();
        let scribe = ClientCommand::ScribeSpell {
            session_id: 5,
            revision: world.inventory.revision(),
            slot: 2,
            spell_id: 73,
            created: Instant::now(),
        };
        testing::run(|out| spellbook.handle(&scribe, &mut world, out))
            .result
            .unwrap();
        let submitted = Instant::now() + SITTING;
        let output = testing::run(|out| spellbook.tick(submitted, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent.len(), 1);
        let timed_out = submitted + Duration::from_secs(31);
        let output = testing::run(|out| spellbook.tick(timed_out, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent, []);
        assert_eq!(statuses(&output.events), [&BookActionStatus::Unconfirmed]);
        assert_eq!(
            spellbook.holds(&world, timed_out),
            [(Resource::Inventory, "Wait for scribing to finish")]
        );

        spellbook.memorize_without_sitting = true;
        let memorize = ClientCommand::MemorizeSpell {
            session_id: 5,
            gem: 0,
            spell_id: 42,
            created: Instant::now(),
        };
        let output = testing::run(|out| spellbook.handle(&memorize, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent.len(), 1);
        let scribed = Message::Event(WorldEvent::Spell(SpellUpdate::Slot {
            slot: 2,
            spell_id: 73,
            mode: 0,
        }));
        let output = testing::run(|out| spellbook.observe(&scribed, &mut world, out));
        output.result.unwrap();
        assert!(output.events.is_empty());
        assert!(spellbook.answers.outstanding());
        assert_eq!(spellbook.book.as_ref().unwrap().slots()[2], Some(73));

        let mut unrelated = Message::Event(WorldEvent::Inventory(InventoryUpdate::Remove(
            InventorySlot(29),
        )));
        spellbook.explain(&mut unrelated, &world);
        assert!(spellbook.consumption.unconfirmed());
        let mut removal = Message::Event(WorldEvent::Inventory(InventoryUpdate::Remove(
            InventorySlot(30),
        )));
        spellbook.explain(&mut removal, &world);
        assert!(matches!(
            removal,
            Message::Event(WorldEvent::Inventory(InventoryUpdate::Remove(
                InventorySlot(30)
            )))
        ));
        assert_eq!(
            spellbook.holds(&world, timed_out),
            [(Resource::Spellbook, "Wait for the current spellbook change")]
        );
    }

    #[test]
    fn book_edits_need_an_idle_admitted_book() {
        let now = Instant::now();
        let book = book(Some(42));
        for command in [
            ClientCommand::SwapSpell {
                session_id: 7,
                from: 0,
                to: 1,
                from_spell: 42,
                to_spell: None,
                created: now,
            },
            ClientCommand::DeleteSpell {
                session_id: 7,
                slot: 0,
                spell_id: 42,
                created: now,
            },
        ] {
            assert!(edit_request(&command, Some(&book), false).is_ok());
            assert!(edit_request(&command, Some(&book), true).is_err());
            assert!(edit_request(&command, None, false).is_err());
        }
    }

    #[test]
    fn fast_memorization_skips_local_delay_but_keeps_book_and_reply_authority() {
        let (mut spellbook, mut world) = admitted(Some(73));
        let option = ClientCommand::SetBookOptions {
            session_id: 5,
            memorize_without_sitting: true,
        };
        let output = testing::run(|out| {
            spellbook.handle(&option, &mut world, out)?;
            spellbook.handle(&memorize(2), &mut world, out)
        });
        output.result.unwrap();
        assert_eq!(output.sent, [spells::titanium_memorize(2, 73)]);
        assert!(spellbook.pending.is_none());
        assert!(
            spellbook.answers.outstanding(),
            "submission is not a confirmed gem change"
        );
        assert_eq!(
            statuses(&output.events),
            [
                &BookActionStatus::Preparing,
                &BookActionStatus::AwaitingReply
            ]
        );
        let (mut spellbook, mut world) = admitted(None);
        spellbook.memorize_without_sitting = true;
        let output = testing::run(|out| spellbook.handle(&memorize(2), &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent, []);
        assert!(!spellbook.answers.outstanding());
    }

    #[test]
    fn disabling_fast_memorization_restores_preparation_and_scribing_is_unchanged() {
        let (mut spellbook, mut world) = admitted(Some(73));
        spellbook.memorize_without_sitting = true;
        let output = testing::run(|out| {
            spellbook.handle(
                &ClientCommand::SetBookOptions {
                    session_id: 5,
                    memorize_without_sitting: false,
                },
                &mut world,
                out,
            )?;
            spellbook.handle(&memorize(2), &mut world, out)
        });
        output.result.unwrap();
        assert_eq!(
            output.sent,
            [eq_network_game::command::titanium_posture(7, Posture::Sitting).unwrap()]
        );
        assert_eq!(spellbook.pending.as_ref().unwrap().delay, SITTING);
        let output = testing::run(|out| {
            spellbook.handle(
                &ClientCommand::SetBookOptions {
                    session_id: 5,
                    memorize_without_sitting: true,
                },
                &mut world,
                out,
            )
        });
        output.result.unwrap();
        assert!(spellbook.pending.is_none());
        assert_eq!(output.sent, []);
        let (mut spellbook, mut world) = admitted(None);
        spellbook.memorize_without_sitting = true;
        world.inventory = consumption::tests::scroll_on_cursor().into();
        let scribe = ClientCommand::ScribeSpell {
            session_id: 5,
            revision: world.inventory.revision(),
            slot: 0,
            spell_id: 73,
            created: Instant::now(),
        };
        let output = testing::run(|out| spellbook.handle(&scribe, &mut world, out));
        output.result.unwrap();
        assert_eq!(
            output.sent,
            [eq_network_game::command::titanium_posture(7, Posture::Sitting).unwrap()]
        );
        assert_eq!(spellbook.pending.as_ref().unwrap().delay, SITTING);
    }

    #[test]
    fn a_timed_out_forget_does_not_own_the_next_edit_on_the_same_gem() {
        let mut character = super::super::character::Character::default();
        let (mut spellbook, mut world) = admitted(Some(73));
        spellbook.memorize_without_sitting = true;
        let mut player = world.player.as_ref().unwrap().clone();
        player.memorized_spells[2] = Some(73);
        world.player.admit(player);
        let forget = |spell_id| ClientCommand::ForgetSpell {
            session_id: 5,
            gem: 2,
            spell_id,
            created: Instant::now(),
        };
        let output = testing::run(|out| spellbook.handle(&forget(73), &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent.len(), 1);
        assert_eq!(statuses(&output.events), [&BookActionStatus::Submitted]);

        let late = Instant::now() + Duration::from_secs(31);
        let output = testing::run(|out| spellbook.tick(late, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent, []);
        assert_eq!(statuses(&output.events), [&BookActionStatus::Unconfirmed]);
        assert_eq!(spellbook.holds(&world, late), []);

        let refresh = |spellbook: &mut Spellbook,
                       character: &mut super::super::character::Character,
                       world: &mut World,
                       spell_id,
                       mode| {
            let update = Message::Event(WorldEvent::Spell(SpellUpdate::Slot {
                slot: 2,
                spell_id,
                mode,
            }));
            let output = testing::run(|out| {
                character.observe(&update, world, out)?;
                spellbook.observe(&update, world, out)
            });
            output.result.unwrap();
            output.events
        };
        let _ = refresh(&mut spellbook, &mut character, &mut world, 74, 1);
        assert_eq!(world.player.as_ref().unwrap().memorized_spells[2], Some(74));
        let refused = testing::run(|out| spellbook.handle(&forget(74), &mut world, out));
        refused.result.unwrap();
        assert_eq!(refused.sent, []);
        assert!(matches!(
            statuses(&refused.events)[..],
            [BookActionStatus::Rejected(_)]
        ));

        assert!(refresh(&mut spellbook, &mut character, &mut world, 73, 2).is_empty());
        assert_eq!(world.player.as_ref().unwrap().memorized_spells[2], None);
        let output = testing::run(|out| spellbook.handle(&memorize(2), &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent.len(), 1);
        assert_eq!(
            statuses(&output.events),
            [
                &BookActionStatus::Preparing,
                &BookActionStatus::AwaitingReply
            ]
        );
        for spell_id in [73, 73, 0xffff] {
            assert!(refresh(&mut spellbook, &mut character, &mut world, spell_id, 2).is_empty());
            assert_eq!(world.player.as_ref().unwrap().memorized_spells[2], None);
            assert!(spellbook.answers.outstanding());
        }
        let settled = refresh(&mut spellbook, &mut character, &mut world, 73, 1);
        assert_eq!(statuses(&settled), [&BookActionStatus::Confirmed]);
        assert_eq!(spellbook.holds(&world, late), []);
        assert_eq!(world.player.as_ref().unwrap().memorized_spells[2], Some(73));
    }

    #[test]
    fn a_live_forget_matches_the_requested_spell_or_an_empty_sentinel() {
        for confirmation in [73, 0xffff] {
            let (mut spellbook, mut world) = admitted(Some(73));
            world.player.corrected().unwrap().memorized_spells[2] = Some(73);
            let command = ClientCommand::ForgetSpell {
                session_id: 5,
                gem: 2,
                spell_id: 73,
                created: Instant::now(),
            };
            testing::run(|out| spellbook.handle(&command, &mut world, out))
                .result
                .unwrap();
            let update = Message::Event(WorldEvent::Spell(SpellUpdate::Slot {
                slot: 2,
                spell_id: 74,
                mode: 2,
            }));
            let output = testing::run(|out| spellbook.observe(&update, &mut world, out));
            output.result.unwrap();
            assert!(output.events.is_empty());
            assert!(spellbook.answers.outstanding());
            let update = Message::Event(WorldEvent::Spell(SpellUpdate::Slot {
                slot: 2,
                spell_id: confirmation,
                mode: 2,
            }));
            let output = testing::run(|out| spellbook.observe(&update, &mut world, out));
            output.result.unwrap();
            assert_eq!(statuses(&output.events), [&BookActionStatus::Confirmed]);
        }
    }

    #[test]
    fn both_prepared_and_fast_memorization_retain_stun_gates() {
        for fast in [false, true] {
            let (mut spellbook, mut world) = admitted(Some(73));
            spellbook.memorize_without_sitting = fast;
            testing::run(|out| {
                super::super::control::Stuns::observe_at(
                    &Message::Event(WorldEvent::Stun(None)),
                    Instant::now(),
                    &mut world,
                    out,
                )
            })
            .result
            .unwrap();
            let output = testing::run(|out| spellbook.handle(&memorize(2), &mut world, out));
            output.result.unwrap();
            assert_eq!(output.sent, []);
            assert!(matches!(
                statuses(&output.events)[..],
                [BookActionStatus::Rejected(_)]
            ));
        }
    }

    #[test]
    fn a_waiting_request_is_checked_against_the_book_when_it_goes_out() {
        let now = Instant::now();
        let pending = PendingBookAction {
            started: now,
            requires_sitting: true,
            delay: SITTING,
            intent: BookIntent::Memorize {
                gem: 2,
                spell_id: 73,
            },
        };
        assert!(!pending.ready(now));
        assert!(!pending.ready(now + Duration::from_millis(4999)));
        assert!(pending.ready(now + SITTING));
        assert!(!pending.ready(now.checked_sub(Duration::from_secs(1)).unwrap()));
        let mut book = book(Some(73));
        let inventory = Inventory::default();
        assert_eq!(
            pending.request(&book, &inventory).unwrap(),
            Request::Memorize {
                gem: 2,
                spell_id: 73
            }
        );
        book.apply(&SpellUpdate::Slot {
            slot: 0,
            spell_id: 0xffff,
            mode: 0,
        });
        assert!(pending.request(&book, &inventory).is_err());
    }
}
