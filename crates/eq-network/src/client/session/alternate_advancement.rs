//! Admission-scoped AA facts for local `EQEmu` (selections, definitions,
//! points, AA experience and reuse timers, checklist 6.547) and local TAKP
//! (ranks, points, AA experience and timers, 6.548); and Titanium's AA
//! requests, training, activating and the experience split (6.556), which
//! no server type lists until their own local `EQEmu` live row passes.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    alternate_advancement::{Action, Rank, Refusal, Update},
    message::Message,
    request::Request,
    world::WorldEvent,
};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

const MAX_STAGED: usize = 4096;
/// What an overflow stages in place of the lost facts: every staged kind of
/// either generation is withdrawn, points and experience too.
const OVERFLOW: [Update; 8] = [
    Update::Selections(None),
    Update::DefinitionsUnknown,
    Update::Points(None),
    Update::Progress(None),
    Update::MacRanks(None),
    Update::MacSelections(None),
    Update::MacPoints(None),
    Update::MacProgress(None),
];
/// The most definitions the view keeps, as many as staging: one past it is
/// not kept, so a train for it is refused, never sent blind.
const MAX_DEFINITIONS: usize = MAX_STAGED;
/// How long a train waits for the server's answer before another may go
/// (ours, inferred: the boat take's wait). `EQEmu` answers a purchase with
/// the next rank, `OP_RespondAA` and then `OP_AAExpUpdate`
/// (`Client::FinishAlternateAdvancementPurchase`), and a refused one with
/// nothing.
const TRAIN_WAIT: Duration = Duration::from_secs(5);
/// The least time between activations, and between split changes (ours,
/// inferred), so that a held key cannot flood the server.
const PACE: Duration = Duration::from_secs(1);

/// What this admission's published facts say, kept only to check a request
/// before it goes; never published itself.
#[derive(Default)]
struct View {
    /// The definitions the table offered, by id, at most [`MAX_DEFINITIONS`].
    definitions: HashMap<u32, Rank>,
    /// The latest selections' ids: exact owned ranks in `OP_RespondAA`, an
    /// ability's first rank in the profile (`EQEmu`'s `LoadAlternateAdvancement`).
    selections: Option<HashSet<u32>>,
    /// The latest unspent points, from the profile or the latest
    /// `OP_AAExpUpdate`, whichever came last.
    unspent: Option<u32>,
}

impl View {
    /// Follows a published fact, withdrawals too.
    fn apply(&mut self, update: &Update) {
        match update {
            Update::Unknown => *self = Self::default(),
            Update::Selections(selections) => {
                self.selections = selections
                    .as_ref()
                    .map(|selections| selections.iter().map(|selection| selection.id).collect());
            }
            Update::Definition {
                id,
                rank: Some(rank),
            } => {
                if self.definitions.len() < MAX_DEFINITIONS || self.definitions.contains_key(id) {
                    self.definitions.insert(*id, rank.clone());
                }
            }
            Update::Definition { id, rank: None } => {
                self.definitions.remove(id);
            }
            Update::DefinitionsUnknown => self.definitions.clear(),
            Update::Points(points) => self.unspent = points.as_ref().map(|points| points.aapoints),
            Update::Progress(progress) => {
                self.unspent = progress
                    .as_ref()
                    .map(|progress| u32::from(progress.unspent));
            }
            Update::Timer(_)
            | Update::MacRanks(_)
            | Update::MacSelections(_)
            | Update::MacPoints(_)
            | Update::MacProgress(_)
            | Update::MacTimer(_) => {}
        }
    }

    /// Whether the player owns this rank, as far as the facts tell. `EQEmu`'s
    /// table offers each ability's owned rank and the next, or its first
    /// (`Client::SendAlternateAdvancementTable`), so a rank another offered
    /// rank follows is owned; otherwise the selections say, by the exact id
    /// either form names for it. A maxed ability above rank one has no
    /// follower and the profile names only its first rank, so it reads as
    /// unowned until `OP_RespondAA` names it.
    fn owned(&self, rank: &Rank) -> Option<bool> {
        let id = i64::from(rank.id);
        if self
            .definitions
            .values()
            .any(|other| i64::from(other.last_id) == id)
        {
            return Some(true);
        }
        self.selections
            .as_ref()
            .map(|selections| selections.contains(&rank.id))
    }

    /// Why a rank may not be bought now, if it may not: each check is one
    /// `EQEmu` makes too (`Mob::CanPurchaseAlternateAdvancementRank`), and
    /// what is not known refuses. Prerequisite abilities are the server's.
    fn train(&self, rank_id: u32, world: &World) -> Option<Refusal> {
        let Some(rank) = self.definitions.get(&rank_id) else {
            return Some(Refusal::NotOffered);
        };
        match self.owned(rank) {
            None => return Some(Refusal::Unknown),
            Some(true) => return Some(Refusal::Owned),
            Some(false) => {}
        }
        // The rank before is offered only while it is owned.
        if let Ok(previous) = u32::try_from(rank.last_id) {
            if !self.definitions.contains_key(&previous) {
                return Some(Refusal::PreviousNotOwned);
            }
        }
        let player = world.player.as_ref();
        let (Some(unspent), Some(level), Some(class)) = (
            self.unspent,
            player.map(|player| player.level).filter(|level| *level > 0),
            player.and_then(|player| player.class),
        ) else {
            return Some(Refusal::Unknown);
        };
        if rank.cost > unspent {
            return Some(Refusal::TooFewPoints);
        }
        if rank.required_level > u32::from(level) {
            return Some(Refusal::LevelTooLow);
        }
        if rank.classes & 1u32.checked_shl(class).unwrap_or(0) == 0 {
            return Some(Refusal::WrongClass);
        }
        None
    }

    /// Why a rank's ability may not be used now, if it may not.
    fn activate(&self, rank_id: u32) -> Option<Refusal> {
        let Some(rank) = self.definitions.get(&rank_id) else {
            return Some(Refusal::NotOffered);
        };
        match self.owned(rank) {
            None => Some(Refusal::Unknown),
            Some(false) => Some(Refusal::NotActivatable),
            Some(true) => (rank.spellid == -1).then_some(Refusal::NotActivatable),
        }
    }
}

/// A train in flight: when it went, and whether `OP_RespondAA` has come.
struct Training {
    sent: Instant,
    listed: bool,
}

#[derive(Default)]
pub(super) struct AlternateAdvancement {
    staged: Vec<Update>,
    overflowed: bool,
    /// Whether the session lists Titanium's AA requests. No server type
    /// does until their local `EQEmu` live row (6.556).
    sends: bool,
    view: View,
    training: Option<Training>,
    activated: Option<Instant>,
    split: Option<Instant>,
}

impl AlternateAdvancement {
    fn publish(
        &mut self,
        updates: Vec<Update>,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        for update in &updates {
            self.view.apply(update);
            if let Some(training) = &mut self.training {
                match update {
                    Update::Selections(_) => training.listed = true,
                    // The answer's last packet.
                    Update::Progress(_) if training.listed => self.training = None,
                    _ => {}
                }
            }
        }
        if updates.is_empty() {
            return Ok(());
        }
        out.log
            .send(ClientEvent::World(WorldEvent::AlternateAdvancement {
                session_id: world.session_id,
                updates,
            }))
    }

    fn waiting(&self) -> bool {
        self.training
            .as_ref()
            .is_some_and(|training| training.sent.elapsed() < TRAIN_WAIT)
    }
}

/// The library's words for a refusal; a host may word it itself.
const fn words(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::NotOffered => "That ability isn't offered to you.",
        Refusal::Owned => "You already have that ability.",
        Refusal::PreviousNotOwned => "Train the rank before it first.",
        Refusal::Unknown => "Your abilities aren't known yet.",
        Refusal::TooFewPoints => "You don't have enough ability points.",
        Refusal::LevelTooLow => "You aren't high enough level for that ability.",
        Refusal::WrongClass => "That ability isn't for your class.",
        Refusal::AwaitingAnswer => "Wait for the answer to the last training.",
        Refusal::NotActivatable => "You don't have that ability to use.",
        Refusal::TooSoon => "Wait a moment before trying again.",
        Refusal::OutOfRange => "That value is out of range.",
    }
}

fn refuse(command: &ClientCommand, refusal: Refusal, out: &mut Out<'_, '_>) -> Result<()> {
    let reason = words(refusal);
    out.log.send(ClientEvent::World(
        WorldEvent::AlternateAdvancementRefused {
            session_id: command.session_id().unwrap_or_default(),
            reason: reason.to_owned(),
            refusal: Some(refusal),
        },
    ))?;
    out.log
        .diagnostic(format!("Rejected {} command: {reason}", command.name()))
}

impl Feature for AlternateAdvancement {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        if self.sends {
            vec![crate::world::Capability::AlternateAdvancement]
        } else {
            Vec::new()
        }
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        self.sends
            && matches!(
                command,
                ClientCommand::TrainAlternateAdvancement { .. }
                    | ClientCommand::ActivateAlternateAdvancement { .. }
                    | ClientCommand::SetAlternateAdvancementSplit { .. }
            )
    }

    fn admit(&mut self, message: &Message, _world: &mut World) -> Result<()> {
        if let Message::AlternateAdvancement(update) = message {
            if self.staged.len() == MAX_STAGED {
                self.staged.clear();
                self.staged.extend(OVERFLOW);
                self.overflowed = true;
            } else if !self.overflowed {
                self.staged.push(update.clone());
            }
        }
        Ok(())
    }

    /// A new admission's view starts from its own staged facts.
    fn admitted(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        self.view = View::default();
        self.training = None;
        self.activated = None;
        self.split = None;
        let staged = std::mem::take(&mut self.staged);
        self.publish(staged, world, out)
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Message::AlternateAdvancement(update) = message {
            self.publish(vec![update.clone()], world, out)?;
        }
        Ok(())
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let action = match *command {
            ClientCommand::TrainAlternateAdvancement { rank_id, .. } => {
                if self.waiting() {
                    return refuse(command, Refusal::AwaitingAnswer, out);
                }
                if let Some(refusal) = self.view.train(rank_id, world) {
                    return refuse(command, refusal, out);
                }
                Action::Train { rank_id }
            }
            ClientCommand::ActivateAlternateAdvancement {
                rank_id, target, ..
            } => {
                if self.activated.is_some_and(|at| at.elapsed() < PACE) {
                    return refuse(command, Refusal::TooSoon, out);
                }
                if target == Some(0) {
                    return refuse(command, Refusal::OutOfRange, out);
                }
                if let Some(refusal) = self.view.activate(rank_id) {
                    return refuse(command, refusal, out);
                }
                Action::Activate { rank_id, target }
            }
            ClientCommand::SetAlternateAdvancementSplit { percentage, .. } => {
                if self.split.is_some_and(|at| at.elapsed() < PACE) {
                    return refuse(command, Refusal::TooSoon, out);
                }
                // `EQEmu` would quietly set 0 instead; the program's own
                // bounds, and any level floor, are read R49-39c.
                if percentage > 100 {
                    return refuse(command, Refusal::OutOfRange, out);
                }
                Action::Split { percentage }
            }
            _ => return actions::refuse(command, "Not an AA request", out.log),
        };
        out.request(&Request::AlternateAdvancement(action))?;
        let now = Instant::now();
        match action {
            Action::Train { .. } => {
                self.training = Some(Training {
                    sent: now,
                    listed: false,
                });
            }
            Action::Activate { .. } => self.activated = Some(now),
            Action::Split { .. } => self.split = Some(now),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use crate::client::ConnectionStage;
    use eq_network_game::alternate_advancement::{Points, Progress, Rank, Selection, Timer};

    fn entered(out: &mut Out<'_, '_>) -> Result<()> {
        out.log.send(ClientEvent::World(WorldEvent::Entered {
            capabilities: vec![eq_network_game::world::Capability::Drowning],
            choices: Vec::new(),
            session_id: 5,
            zone: "synthetic".into(),
            zone_id: 1,
            player: Box::new(testing::player(7)),
            far_clip: None,
            spell_landing_flag: Some(4),
        }))
    }

    #[test]
    fn staged_facts_are_one_ordered_batch_between_admission_and_ready() {
        let mut feature = AlternateAdvancement::default();
        let mut world = World::new(5);
        let mut facts = vec![
            Update::Selections(Some(Vec::new())),
            Update::Selections(Some(vec![Selection { id: 1, value: 99 }])),
        ];
        facts.extend((1..=113).map(|id| Update::Definition {
            id,
            rank: Some(Rank {
                id,
                hotkey_sid: 0,
                hotkey_sid2: 0,
                title_sid: 0,
                desc_sid: 0,
                required_level: 1,
                cost: 1,
                sequence: id,
                rank: 1,
                prereq_skill: 0,
                prereq_minpoints: 0,
                kind: 1,
                spellid: -1,
                spell_type: 0,
                spell_refresh: 0,
                classes: 2,
                max_level: 1,
                last_id: -1,
                next_id: -1,
                cost2: 1,
                effects: Vec::new(),
            }),
        }));
        // Points, experience and a timer stage in received order too.
        facts.extend([
            Update::Points(Some(Points {
                aapoints_spent: 1,
                aapoints: 2,
                exp_aa: 3,
            })),
            Update::Progress(Some(Progress {
                experience: 165,
                unspent: 2,
                percentage: 50,
            })),
            Update::Timer(Timer {
                begin: 1,
                ability: 9,
                end: 2,
            }),
        ]);
        let before = testing::run(|_| {
            for update in &facts {
                feature
                    .admit(&Message::AlternateAdvancement(update.clone()), &mut world)
                    .unwrap();
            }
        });
        assert!(before.events.is_empty());
        let outcome = testing::run(|out| {
            entered(out)?;
            feature.admitted(&mut world, out)?;
            out.log.send(ClientEvent::Progress(ConnectionStage::Ready))
        });
        outcome.result.unwrap();
        assert_eq!(facts.len(), 118);
        assert!(matches!(outcome.events.as_slice(), [
            ClientEvent::World(WorldEvent::Entered { session_id: 5, .. }),
            ClientEvent::World(WorldEvent::LiquidHazardRevision { session_id: 5, revision: 0 }),
            ClientEvent::World(WorldEvent::AlternateAdvancement { session_id: 5, updates }),
            ClientEvent::World(WorldEvent::LiquidHazardRevision { session_id: 5, revision: 1 }),
            ClientEvent::Progress(ConnectionStage::Ready),
        ] if updates == &facts));
    }

    #[test]
    fn empty_stage_sends_nothing_and_ongoing_facts_publish_immediately() {
        let mut feature = AlternateAdvancement::default();
        let mut world = World::new(5);
        let empty = testing::run(|out| feature.admitted(&mut world, out));
        empty.result.unwrap();
        assert!(empty.events.is_empty());
        let outcome = testing::run(|out| {
            entered(out)?;
            feature.observe(
                &Message::AlternateAdvancement(Update::Unknown),
                &mut world,
                out,
            )
        });
        outcome.result.unwrap();
        assert!(matches!(&outcome.events[2..], [
            ClientEvent::World(WorldEvent::AlternateAdvancement { session_id: 5, updates }),
            ClientEvent::World(WorldEvent::LiquidHazardRevision { session_id: 5, revision: 1 }),
        ] if updates == &[Update::Unknown]));
    }

    #[test]
    fn the_staging_bound_retains_exact_limit_then_withdraws_on_overflow() {
        for overflow in [false, true] {
            let mut feature = AlternateAdvancement::default();
            let mut world = World::new(5);
            // Points and experience staged before an overflow are withdrawn
            // with it, not silently lost.
            let mut facts: Vec<_> = (1..MAX_STAGED - 1)
                .map(|id| Update::Definition {
                    id: u32::try_from(id).unwrap(),
                    rank: None,
                })
                .collect();
            facts.extend([
                Update::Points(Some(Points {
                    aapoints_spent: 1,
                    aapoints: 2,
                    exp_aa: 3,
                })),
                Update::Progress(Some(Progress {
                    experience: 1,
                    unspent: 2,
                    percentage: 3,
                })),
            ]);
            assert_eq!(facts.len(), MAX_STAGED);
            for update in &facts {
                feature
                    .admit(&Message::AlternateAdvancement(update.clone()), &mut world)
                    .unwrap();
            }
            if overflow {
                for update in [Update::Unknown, Update::Selections(Some(Vec::new()))] {
                    feature
                        .admit(&Message::AlternateAdvancement(update), &mut world)
                        .unwrap();
                }
            }
            let outcome = testing::run(|out| feature.admitted(&mut world, out));
            outcome.result.unwrap();
            let expected = if overflow { OVERFLOW.to_vec() } else { facts };
            assert!(matches!(outcome.events.as_slice(), [
                ClientEvent::World(WorldEvent::AlternateAdvancement { session_id: 5, updates }),
            ] if updates == &expected));
        }
    }

    /// A rank definition with the fields the gates read; the rest zero.
    fn rank(id: u32, last_id: i32, cost: u32, required_level: u32, spellid: i32) -> Rank {
        Rank {
            id,
            hotkey_sid: 0,
            hotkey_sid2: 0,
            title_sid: 0,
            desc_sid: 0,
            required_level,
            cost,
            sequence: id,
            rank: 1,
            prereq_skill: 0,
            prereq_minpoints: 0,
            kind: 1,
            spellid,
            spell_type: 0,
            spell_refresh: 0,
            // A Wizard's bit (class 12).
            classes: 1 << 12,
            max_level: 3,
            last_id,
            next_id: -1,
            cost2: cost,
            effects: Vec::new(),
        }
    }

    fn offer(id: u32, rank: Rank) -> Update {
        Update::Definition {
            id,
            rank: Some(rank),
        }
    }

    fn points(aapoints: u32) -> Update {
        Update::Points(Some(Points {
            aapoints_spent: 0,
            aapoints,
            exp_aa: 0,
        }))
    }

    fn progress(unspent: u16) -> Update {
        Update::Progress(Some(Progress {
            experience: 0,
            unspent,
            percentage: 0,
        }))
    }

    /// An admitted level-51 Wizard whose sends are listed, with the facts
    /// staged and published.
    fn admitted(facts: &[Update]) -> (AlternateAdvancement, World) {
        let mut world = World::new(5);
        let mut player = testing::player(7);
        player.level = 51;
        player.class = Some(12);
        world.player.admit(player);
        let mut feature = AlternateAdvancement {
            sends: true,
            ..AlternateAdvancement::default()
        };
        for update in facts {
            feature
                .admit(&Message::AlternateAdvancement(update.clone()), &mut world)
                .unwrap();
        }
        testing::run(|out| feature.admitted(&mut world, out))
            .result
            .unwrap();
        (feature, world)
    }

    fn run(
        feature: &mut AlternateAdvancement,
        world: &mut World,
        command: &ClientCommand,
    ) -> testing::Outcome<Result<()>> {
        let outcome = testing::run(|out| feature.handle(command, world, out));
        outcome.result.as_ref().unwrap();
        outcome
    }

    /// The refusal told, if any; a refused request sends nothing, and a
    /// sent one is told no refusal.
    fn refusal(outcome: &testing::Outcome<Result<()>>) -> Option<Refusal> {
        let refusal = outcome.events.iter().find_map(|event| match event {
            ClientEvent::World(WorldEvent::AlternateAdvancementRefused {
                session_id: 5,
                refusal,
                ..
            }) => *refusal,
            _ => None,
        });
        assert_eq!(refusal.is_some(), outcome.sent.is_empty());
        refusal
    }

    fn train(rank_id: u32) -> ClientCommand {
        ClientCommand::TrainAlternateAdvancement {
            session_id: 5,
            rank_id,
        }
    }

    fn observe(feature: &mut AlternateAdvancement, world: &mut World, update: Update) {
        testing::run(|out| feature.observe(&Message::AlternateAdvancement(update), world, out))
            .result
            .unwrap();
    }

    /// The table of a Wizard who owns rank 10 of one ability (11 follows
    /// it) and none of another (20), plus ranks the gates refuse.
    fn table() -> Vec<Update> {
        vec![
            // The profile's form: an ability's first rank, by its id.
            Update::Selections(Some(vec![Selection { id: 10, value: 1 }])),
            offer(10, rank(10, -1, 1, 51, 2_000)),
            offer(11, rank(11, 10, 3, 51, 2_000)),
            offer(20, rank(20, -1, 2, 51, -1)),
            // Its rank before is not in the table.
            offer(31, rank(31, 30, 1, 51, -1)),
            offer(40, rank(40, -1, 9, 51, -1)),
            offer(50, rank(50, -1, 1, 52, -1)),
            offer(
                60,
                Rank {
                    classes: 1 << 1,
                    ..rank(60, -1, 1, 1, -1)
                },
            ),
            points(3),
        ]
    }

    #[test]
    fn the_sends_are_listed_by_no_server_type_yet() {
        let feature = AlternateAdvancement::default();
        assert_eq!(feature.capabilities(), Vec::new());
        assert!(!feature.owns(&train(11)));
    }

    #[test]
    fn a_train_goes_only_for_an_offered_unowned_rank_the_player_can_buy() {
        let (mut feature, mut world) = admitted(&table());
        for (rank_id, expected) in [
            (99, Refusal::NotOffered),
            // Owned by the selection, and by the rank that follows it.
            (10, Refusal::Owned),
            (31, Refusal::PreviousNotOwned),
            (40, Refusal::TooFewPoints),
            (50, Refusal::LevelTooLow),
            (60, Refusal::WrongClass),
        ] {
            let outcome = run(&mut feature, &mut world, &train(rank_id));
            assert_eq!(refusal(&outcome), Some(expected), "{rank_id}");
        }
        let sent = run(&mut feature, &mut world, &train(11));
        assert_eq!(
            (sent.sent[0].opcode, sent.sent[0].body.as_slice()),
            (
                0x0681,
                [3, 0, 0, 0, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0].as_slice()
            )
        );
        // One train in flight: the next waits for the answer.
        let waiting = run(&mut feature, &mut world, &train(20));
        assert_eq!(refusal(&waiting), Some(Refusal::AwaitingAnswer));
        // OP_RespondAA names the exact owned rank, and AAExpUpdate ends
        // the answer with the points left.
        observe(
            &mut feature,
            &mut world,
            Update::Selections(Some(vec![Selection { id: 11, value: 4 }])),
        );
        let still = run(&mut feature, &mut world, &train(20));
        assert_eq!(refusal(&still), Some(Refusal::AwaitingAnswer));
        observe(&mut feature, &mut world, progress(0));
        let poor = run(&mut feature, &mut world, &train(20));
        assert_eq!(refusal(&poor), Some(Refusal::TooFewPoints));
        observe(&mut feature, &mut world, progress(2));
        let bought = run(&mut feature, &mut world, &train(20));
        assert_eq!(bought.sent[0].body[4], 20);
        assert_eq!(refusal(&bought), None);
        // An answer that never comes frees the next train after the wait.
        feature.training.as_mut().unwrap().sent = Instant::now().checked_sub(TRAIN_WAIT).unwrap();
        let after = run(&mut feature, &mut world, &train(11));
        assert_eq!(refusal(&after), Some(Refusal::Owned));
    }

    #[test]
    fn what_is_not_known_or_withdrawn_refuses_a_train() {
        // No selections, no points.
        for missing in [0, 1] {
            let mut facts = table();
            facts.remove(if missing == 0 { 0 } else { facts.len() - 1 });
            let (mut feature, mut world) = admitted(&facts);
            let outcome = run(&mut feature, &mut world, &train(20));
            assert_eq!(refusal(&outcome), Some(Refusal::Unknown));
        }
        // An unknown level or class.
        for player in [0, 1] {
            let (mut feature, mut world) = admitted(&table());
            let known = world.player.corrected().unwrap();
            if player == 0 {
                known.level = 0;
            } else {
                known.class = None;
            }
            let outcome = run(&mut feature, &mut world, &train(20));
            assert_eq!(refusal(&outcome), Some(Refusal::Unknown));
        }
        // The current level counts: a level gained this admission.
        let (mut feature, mut world) = admitted(&table());
        world.player.corrected().unwrap().level = 52;
        let outcome = run(&mut feature, &mut world, &train(50));
        assert_eq!(refusal(&outcome), None);
        // Each withdrawal clears its part of the view.
        for withdrawal in [
            Update::Definition { id: 20, rank: None },
            Update::DefinitionsUnknown,
            Update::Unknown,
        ] {
            let (mut feature, mut world) = admitted(&table());
            observe(&mut feature, &mut world, withdrawal);
            let outcome = run(&mut feature, &mut world, &train(20));
            assert_eq!(refusal(&outcome), Some(Refusal::NotOffered));
        }
        for withdrawal in [
            Update::Selections(None),
            Update::Points(None),
            Update::Progress(None),
        ] {
            let (mut feature, mut world) = admitted(&table());
            observe(&mut feature, &mut world, withdrawal);
            let outcome = run(&mut feature, &mut world, &train(20));
            assert_eq!(refusal(&outcome), Some(Refusal::Unknown));
        }
        // An overflowed stage withdraws everything.
        let mut facts = table();
        facts.extend(
            (1000..1000 + u32::try_from(MAX_STAGED).unwrap())
                .map(|id| offer(id, rank(id, -1, 1, 1, -1))),
        );
        let (mut feature, mut world) = admitted(&facts);
        let outcome = run(&mut feature, &mut world, &train(20));
        assert_eq!(refusal(&outcome), Some(Refusal::NotOffered));
    }

    #[test]
    fn the_view_keeps_at_most_its_bound_of_definitions() {
        let (mut feature, mut world) = admitted(&table());
        let kept = feature.view.definitions.len();
        let first = 1000;
        let last = first + u32::try_from(MAX_DEFINITIONS - kept).unwrap();
        for id in first..=last {
            observe(&mut feature, &mut world, offer(id, rank(id, -1, 1, 1, -1)));
        }
        assert_eq!(feature.view.definitions.len(), MAX_DEFINITIONS);
        let past = run(&mut feature, &mut world, &train(last));
        assert_eq!(refusal(&past), Some(Refusal::NotOffered));
        // A kept one is still replaced in place.
        observe(&mut feature, &mut world, offer(20, rank(20, -1, 1, 1, -1)));
        let kept = run(&mut feature, &mut world, &train(20));
        assert_eq!(refusal(&kept), None);
    }

    #[test]
    fn an_activation_goes_for_an_owned_rank_with_an_ability_at_a_pace() {
        let (mut feature, mut world) = admitted(&table());
        let activate = |rank_id, target| ClientCommand::ActivateAlternateAdvancement {
            session_id: 5,
            rank_id,
            target,
        };
        for (rank_id, target, expected) in [
            (99, None, Refusal::NotOffered),
            // Not owned; owned with no ability; spawn 0.
            (11, None, Refusal::NotActivatable),
            (20, None, Refusal::NotActivatable),
            (10, Some(0), Refusal::OutOfRange),
        ] {
            let outcome = run(&mut feature, &mut world, &activate(rank_id, target));
            assert_eq!(refusal(&outcome), Some(expected), "{rank_id}");
        }
        let used = run(&mut feature, &mut world, &activate(10, Some(300)));
        assert_eq!(
            used.sent[0].body,
            [0, 0, 0, 0, 10, 0, 0, 0, 44, 1, 0, 0, 0, 0, 0, 0]
        );
        let again = run(&mut feature, &mut world, &activate(10, None));
        assert_eq!(refusal(&again), Some(Refusal::TooSoon));
        feature.activated = Instant::now().checked_sub(PACE);
        let later = run(&mut feature, &mut world, &activate(10, None));
        assert_eq!(refusal(&later), None);
        assert_eq!(later.sent.len(), 1);
    }

    #[test]
    fn a_maxed_ability_above_rank_one_is_unowned_until_the_exact_ids_come() {
        // Rank 72 is the last of an ability whose first rank is 70: the
        // table offers it alone, with nothing after it, and the profile
        // names only the first rank (F1).
        let (mut feature, mut world) = admitted(&[
            Update::Selections(Some(vec![Selection { id: 70, value: 3 }])),
            offer(72, rank(72, 71, 1, 51, 3_000)),
            points(3),
        ]);
        let activate = ClientCommand::ActivateAlternateAdvancement {
            session_id: 5,
            rank_id: 72,
            target: None,
        };
        let early = run(&mut feature, &mut world, &activate);
        assert_eq!(refusal(&early), Some(Refusal::NotActivatable));
        // Nor can it be bought: the rank before is not offered.
        let bought = run(&mut feature, &mut world, &train(72));
        assert_eq!(refusal(&bought), Some(Refusal::PreviousNotOwned));
        // OP_RespondAA names the exact owned rank.
        observe(
            &mut feature,
            &mut world,
            Update::Selections(Some(vec![Selection { id: 72, value: 6 }])),
        );
        let used = run(&mut feature, &mut world, &activate);
        assert_eq!(refusal(&used), None);
        assert_eq!(used.sent[0].body[4], 72);
    }

    #[test]
    fn a_split_of_zero_turns_it_off_and_above_one_hundred_is_refused() {
        let (mut feature, mut world) = admitted(&[]);
        let split = |percentage| ClientCommand::SetAlternateAdvancementSplit {
            session_id: 5,
            percentage,
        };
        let refused = run(&mut feature, &mut world, &split(101));
        assert_eq!(refusal(&refused), Some(Refusal::OutOfRange));
        let off = run(&mut feature, &mut world, &split(0));
        assert_eq!(
            off.sent[0].body,
            [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        let soon = run(&mut feature, &mut world, &split(50));
        assert_eq!(refusal(&soon), Some(Refusal::TooSoon));
        feature.split = Instant::now().checked_sub(PACE);
        let set = run(&mut feature, &mut world, &split(50));
        assert_eq!(
            set.sent[0].body,
            [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 50, 0, 0, 0]
        );
    }

    #[test]
    fn a_request_for_an_earlier_admission_is_refused_without_a_reason_kind() {
        let command = train(11);
        let reason = actions::stale(
            &ClientCommand::TrainAlternateAdvancement {
                session_id: 4,
                rank_id: 11,
            },
            5,
            Instant::now(),
        )
        .unwrap();
        let outcome = testing::run(|out| actions::refuse(&command, reason, out.log));
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::AlternateAdvancementRefused {
                session_id: 5,
                refusal: None,
                ..
            })
        )));
    }
}
