//! Training skills at a guildmaster. The session opens training only with a
//! guildmaster of the player's class within reach, and refuses a practice
//! the server would ignore without a word: no training open, a skill the
//! guildmaster does not teach or one at its cap, no practice point left or
//! coins too few. The server answers a practice only with the skill's new
//! value, so the session counts the practice points itself, from the
//! profile's on admission, and turns that answer into the practice it was,
//! with its cost, for the coins and the host. Levels above the highest the
//! player reached in this zone bring five points each, as they do on the
//! server, which counts from the highest level the character ever reached.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    message::Message,
    request::Request,
    training::{self, PracticeUpdate, TrainingOffer, TrainingRequest, TrainingUpdate},
    world::{SpawnKind, WorldEvent},
};
use std::{
    collections::{BTreeSet, VecDeque},
    time::{Duration, Instant},
};

/// How long a practice may go unanswered before the session stops waiting:
/// the server ignores one silently, for instance a skill the player's level
/// cannot train yet.
const ANSWER_WAIT: Duration = Duration::from_secs(5);

/// Practice points each level brings.
const POINTS_PER_LEVEL: u32 = 5;

/// Trains the player's skills at a guildmaster.
#[derive(Default)]
pub(super) struct Training {
    /// The guildmaster asked to train, or training, if any.
    trainer: Option<Trainer>,
    /// Practices sent and not yet answered, oldest first.
    pending: VecDeque<Practice>,
    /// Unspent practice points, once the zone has admitted the player.
    points: Option<u32>,
    /// The highest level the player reached in this zone.
    highest_level: u8,
    /// P99 follows the native admission and provisional practice rules.
    p99: bool,
    /// Awaited offers abandoned within this admission have no request token.
    abandoned: BTreeSet<u16>,
}

/// A guildmaster the player asked to train with.
struct Trainer {
    /// Their spawn.
    spawn_id: u16,
    /// Their answer, once it came.
    offer: Option<TrainingOffer>,
    requested_at: Instant,
}

/// A practice sent to the server.
struct Practice {
    /// The skill.
    skill: u32,
    /// Its value when the practice was sent.
    from: u32,
    /// What the practice costs, in copper.
    cost: u64,
    /// When it was sent.
    sent: Instant,
}

impl Training {
    /// P99 uses native admission fields and keeps skill and point authority separate.
    pub(super) fn project1999() -> Self {
        Self {
            p99: true,
            ..Self::default()
        }
    }

    /// Checks the current trainer spawn and reach before interacting.
    fn eligible(trainer: u16, world: &World) -> Result<(), &'static str> {
        let Some(((_, position), player)) = world.player_at().zip(world.player.as_ref()) else {
            return Err("Not in the zone yet");
        };
        let Some(spawn) = world.spawns.visible(trainer) else {
            return Err("You must first target your guildmaster.");
        };
        let teaches = spawn.kind == SpawnKind::Npc
            && spawn
                .class
                .zip(player.class)
                .is_some_and(|(trainer, player)| training::trains(trainer, player));
        if !teaches {
            return Err("That is not your guildmaster.");
        }
        let (dx, dy, dz) = (
            spawn.position.x - position.x,
            spawn.position.y - position.y,
            spawn.position.z - position.z,
        );
        let distance = dx * dx + dy * dy + dz * dz;
        if !distance.is_finite() || distance > training::RANGE * training::RANGE {
            return Err("You are too far away from your guildmaster.");
        }
        Ok(())
    }

    /// Opens training with a guildmaster, leaving any other first on EQEmu.
    fn open(
        &mut self,
        command: &ClientCommand,
        trainer: u16,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Err(reason) = Self::eligible(trainer, world) {
            return actions::refuse(command, reason, out.log);
        }
        if self.p99 && self.abandoned.contains(&trainer) {
            return actions::refuse(
                command,
                "A previous training request has no confirmed outcome. Re-enter the zone before asking this guildmaster again.",
                out.log,
            );
        }
        if self
            .trainer
            .as_ref()
            .is_some_and(|open| open.spawn_id == trainer)
        {
            return Ok(());
        }
        if self.p99 && self.trainer.is_some() {
            return actions::refuse(
                command,
                "Finish training with the current guildmaster first.",
                out.log,
            );
        }
        if let Some(open) = self.trainer.take() {
            out.request(&Request::EndTraining(open.spawn_id))?;
        }
        out.request(&Request::OpenTraining(trainer))?;
        self.trainer = Some(Trainer {
            spawn_id: trainer,
            offer: None,
            requested_at: Instant::now(),
        });
        Ok(())
    }

    /// Why the player cannot practice a skill now, if they cannot, and what
    /// the practice costs if they can.
    fn practice(&self, skill: u32, world: &World) -> Result<(u16, u32, u64), &'static str> {
        let Some((trainer, offer)) = self.trainer.as_ref().and_then(|trainer| {
            trainer
                .offer
                .as_ref()
                .map(|offer| (trainer.spawn_id, offer))
        }) else {
            return Err("You are not training with a guildmaster.");
        };
        if self.p99 {
            Self::eligible(trainer, world)?;
            if !self.pending.is_empty() {
                return Err("Wait for the current practice to settle.");
            }
            if self.points.is_none() {
                return Err(
                    "The practice point balance is unknown. Re-enter the zone to refresh it.",
                );
            }
        }
        let cap = offer.cap(skill);
        if cap == 0 {
            return Err("Your guildmaster cannot train you in that skill.");
        }
        let unanswered = u32::try_from(self.pending.len()).unwrap_or(u32::MAX);
        if self.points.unwrap_or(0) <= unanswered {
            return Err("You do not have any practice points left.");
        }
        let value = world
            .player
            .as_ref()
            .and_then(|player| player.skills.as_ref())
            .and_then(|skills| skills.get(usize::try_from(skill).ok()?))
            .copied();
        let value = match value {
            Some(value) => value,
            None if self.p99 => return Err("Your current skill value is unknown."),
            None => 0,
        };
        if value >= cap {
            return Err("You cannot be trained any further in that skill.");
        }
        let cost = training::practice_cost(value);
        let owed: u64 = self.pending.iter().map(|practice| practice.cost).sum();
        if self.p99 && cost > 0 && world.coins.purse.is_none() {
            return Err("Your purse balance is unknown.");
        }
        let purse = world.coins.purse.map_or(0, |coins| coins.total_copper());
        if purse < owed.saturating_add(cost) {
            return Err("You cannot afford to train that skill.");
        }
        Ok((trainer, value, cost))
    }

    /// Practices a skill once.
    fn train(
        &mut self,
        command: &ClientCommand,
        skill: u32,
        world: &mut World,
        out: &mut Out<'_, '_>,
        now: Instant,
    ) -> Result<()> {
        let (trainer, from, cost) = match self.practice(skill, world) {
            Ok(practice) => practice,
            Err(reason) => return actions::refuse(command, reason, out.log),
        };
        out.request(&Request::Train { trainer, skill })?;
        self.pending.push_back(Practice {
            skill,
            from,
            cost,
            sent: now,
        });
        if self.p99 {
            world.happened(Message::Event(WorldEvent::Training(
                TrainingUpdate::Practice(PracticeUpdate::Requested {
                    skill,
                    estimated_cost: cost,
                }),
            )));
        }
        Ok(())
    }

    /// Leaves training; with none open there is nothing to leave.
    fn end(&mut self, out: &mut Out<'_, '_>) -> Result<()> {
        let Some(trainer) = self.trainer.take() else {
            return Ok(());
        };
        if self.p99 && trainer.offer.is_none() {
            self.abandoned.insert(trainer.spawn_id);
        } else {
            out.request(&Request::EndTraining(trainer.spawn_id))?;
        }
        out.log.send(ClientEvent::World(WorldEvent::Training(
            TrainingUpdate::Ended,
        )))
    }

    /// Drops an offer when its trainer or zone is no longer current.
    fn invalidate_trainer(&mut self, out: &mut Out<'_, '_>) -> Result<()> {
        if let Some(trainer) = self.trainer.take() {
            self.abandoned.insert(trainer.spawn_id);
            out.log.send(ClientEvent::World(WorldEvent::Training(
                TrainingUpdate::Ended,
            )))?;
        }
        Ok(())
    }

    /// Emits a practice projection separately from authoritative skill news.
    fn report_practice(update: PracticeUpdate, out: &mut Out<'_, '_>) -> Result<()> {
        out.log.send(ClientEvent::World(WorldEvent::Training(
            TrainingUpdate::Practice(update),
        )))
    }

    /// Tracks the received skill and retains point/cost uncertainty.
    fn observe_p99(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let trainer = self.trainer.as_ref().map(|trainer| trainer.spawn_id);
        match message {
            Message::TrainingReply(reply)
                if !reply.accepted
                    && trainer == Some(reply.offer.trainer)
                    && world.own_spawn.map(u32::from) == Some(reply.trainee)
                    && self
                        .trainer
                        .as_ref()
                        .is_some_and(|trainer| trainer.offer.is_none()) =>
            {
                self.trainer = None;
                out.log
                    .send(ClientEvent::World(WorldEvent::TrainingRefused {
                        session_id: world.session_id,
                        reason: "The guildmaster declined to open training.".into(),
                    }))?;
            }
            Message::Event(WorldEvent::Skill { skill_id, value }) => {
                let matches = self.pending.front().is_some_and(|practice| {
                    practice.skill == *skill_id
                        && practice.sent.elapsed() < ANSWER_WAIT
                        && (practice.from.checked_add(1) == Some(*value)
                            || (practice.from == 0 && (1..=252).contains(value)))
                });
                if matches {
                    let practice = self.pending.pop_front().expect("practice just checked");
                    self.points = self.points.and_then(|points| points.checked_sub(1));
                    world.happened(Message::Event(WorldEvent::Training(
                        TrainingUpdate::Practice(PracticeUpdate::SkillRaised {
                            skill: practice.skill,
                            value: *value,
                            estimated_cost: practice.cost,
                            estimated_points: self.points,
                        }),
                    )));
                }
            }
            Message::Event(WorldEvent::Level {
                current, previous, ..
            }) if current != previous && self.points.take().is_some() => {
                Self::report_practice(PracticeUpdate::PointsUncertain, out)?;
            }
            Message::Event(WorldEvent::Despawn(id)) if trainer == Some(*id) => {
                self.invalidate_trainer(out)?;
            }
            Message::Event(WorldEvent::Spawns(spawns))
                if spawns.iter().any(|spawn| trainer == Some(spawn.spawn_id)) =>
            {
                self.invalidate_trainer(out)?;
            }
            Message::Event(event) if world.owns_death(event) => {
                self.invalidate_trainer(out)?;
            }
            Message::ZoneOffer(_) => self.invalidate_trainer(out)?,
            _ => (),
        }
        Ok(())
    }

    /// Tells the host how many practice points are left.
    fn tell_points(&self, out: &mut Out<'_, '_>) -> Result<()> {
        match self.points {
            Some(points) => out
                .log
                .send(ClientEvent::World(WorldEvent::PracticePoints(points))),
            None => Ok(()),
        }
    }
}

impl Feature for Training {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Training]
    }

    fn admitted(&mut self, world: &mut World, _out: &mut Out<'_, '_>) -> Result<()> {
        if let Some(player) = world.player.as_ref() {
            self.points = player.practice_points;
            self.highest_level = player.level;
        }
        Ok(())
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::Training { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::Training { request, .. } = command else {
            return Ok(());
        };
        match *request {
            TrainingRequest::Open { trainer } => self.open(command, trainer, world, out),
            TrainingRequest::Train { skill } => {
                self.train(command, skill, world, out, Instant::now())
            }
            TrainingRequest::TrainAt { trainer, skill } => {
                if self.trainer.as_ref().map(|offer| offer.spawn_id) != Some(trainer) {
                    return actions::refuse(
                        command,
                        "That training window is no longer current.",
                        out.log,
                    );
                }
                self.train(command, skill, world, out, Instant::now())
            }
            TrainingRequest::End => self.end(out),
        }
    }

    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if !self.p99 {
            self.pending
                .retain(|practice| now.saturating_duration_since(practice.sent) < ANSWER_WAIT);
            return Ok(());
        }
        if let Some(trainer) = self.trainer.as_ref() {
            if trainer.offer.is_none()
                && now.saturating_duration_since(trainer.requested_at) >= ANSWER_WAIT
            {
                self.abandoned.insert(trainer.spawn_id);
                self.trainer = None;
                out.log
                    .send(ClientEvent::World(WorldEvent::TrainingRefused {
                        session_id: world.session_id,
                        reason: "The guildmaster has not answered. The outcome is unknown.".into(),
                    }))?;
            } else if world.lifecycle.blocks_motion()
                || Self::eligible(trainer.spawn_id, world).is_err()
            {
                self.invalidate_trainer(out)?;
            }
        }
        if self
            .pending
            .front()
            .is_some_and(|practice| now.saturating_duration_since(practice.sent) >= ANSWER_WAIT)
        {
            let practice = self.pending.pop_front().expect("practice just checked");
            self.points = None;
            Self::report_practice(
                PracticeUpdate::Unanswered {
                    skill: practice.skill,
                },
                out,
            )?;
        }
        Ok(())
    }

    /// A skill's new value that answers a practice is that practice: it
    /// spends a point and its cost when the skill rose, and nothing when the
    /// guildmaster refused, which the server says in chat.
    fn explain(&mut self, message: &mut Message, world: &World) {
        if let Message::TrainingReply(reply) = message {
            let pending = self.trainer.as_ref().is_some_and(|trainer| {
                trainer.spawn_id == reply.offer.trainer
                    && trainer.offer.is_none()
                    && trainer.requested_at.elapsed() < ANSWER_WAIT
            });
            if !self.p99 {
                *message = Message::Event(WorldEvent::Training(TrainingUpdate::Offered(
                    reply.offer.clone(),
                )));
            } else if pending
                && reply.accepted
                && world.own_spawn.map(u32::from) == Some(reply.trainee)
                && Self::eligible(reply.offer.trainer, world).is_ok()
                && !world.lifecycle.blocks_motion()
            {
                *message = Message::Event(WorldEvent::Training(TrainingUpdate::Offered(
                    training::p99_offer(reply.offer.clone()),
                )));
            }
            return;
        }
        if self.p99 {
            return;
        }
        let Message::Event(WorldEvent::Skill { skill_id, value }) = message else {
            return;
        };
        let Some(index) = self
            .pending
            .iter()
            .position(|practice| practice.skill == *skill_id)
        else {
            return;
        };
        let Some(practice) = self.pending.remove(index) else {
            return;
        };
        if *value > practice.from {
            *message = Message::Event(WorldEvent::Training(TrainingUpdate::Trained {
                skill: practice.skill,
                value: *value,
                cost: practice.cost,
            }));
        }
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if self.p99 {
            self.observe_p99(message, world, out)?;
        }
        let Message::Event(event) = message else {
            return Ok(());
        };
        match event {
            WorldEvent::Training(TrainingUpdate::Offered(offer)) => {
                if let Some(trainer) = self
                    .trainer
                    .as_mut()
                    .filter(|trainer| trainer.spawn_id == offer.trainer)
                {
                    trainer.offer = Some(offer.clone());
                }
                Ok(())
            }
            WorldEvent::Training(TrainingUpdate::Trained { .. }) => {
                self.points = self.points.map(|points| points.saturating_sub(1));
                self.tell_points(out)
            }
            WorldEvent::Level { current, .. } if !self.p99 && *current > self.highest_level => {
                let gained = u32::from(*current - self.highest_level);
                self.highest_level = *current;
                self.points = self
                    .points
                    .map(|points| points.saturating_add(gained * POINTS_PER_LEVEL));
                self.tell_points(out)
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::world::{Coins, Position};

    /// An admitted level 10 warrior (class 1) with 3 practice points, kick
    /// at 12 and 10 gold, beside their guildmaster (42, class 20); a cleric
    /// guildmaster (43) stands beside them too, and their own (44) far off.
    fn warrior() -> (Training, World) {
        let mut world = World::new(5);
        let mut player = testing::player(7);
        player.class = Some(1);
        player.level = 10;
        player.practice_points = Some(3);
        let mut skills = vec![0; 100];
        skills[30] = 12;
        player.skills = Some(skills);
        world.player.admit(player);
        world.own_spawn = Some(7);
        world.coins = eq_network_game::money::Wallet {
            purse: Some(Coins {
                gold: 10,
                ..Coins::default()
            }),
            ..eq_network_game::money::Wallet::default()
        }
        .into();
        for (spawn_id, class, x) in [(42, 20, 5.0), (43, 21, 5.0), (44, 20, 300.0)] {
            let mut guildmaster = testing::spawn(spawn_id, SpawnKind::Npc);
            guildmaster.class = Some(class);
            guildmaster.position = Position {
                x,
                ..Position::default()
            };
            world.spawns.insert(guildmaster);
        }
        let mut training = Training::default();
        testing::run(|out| training.admitted(&mut world, out))
            .result
            .unwrap();
        (training, world)
    }

    fn ask(request: TrainingRequest) -> ClientCommand {
        ClientCommand::Training {
            session_id: 5,
            request,
            created: Instant::now(),
        }
    }

    fn refusals(events: &[ClientEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::TrainingRefused { reason, .. }) => {
                    Some(reason.clone())
                }
                _ => None,
            })
            .collect()
    }

    /// The guildmaster's answer: kick trains to 200, and nothing else.
    fn offer(trainer: u16) -> Message {
        let mut caps = vec![0; 100];
        caps[30] = 200;
        Message::Event(WorldEvent::Training(TrainingUpdate::Offered(
            TrainingOffer { trainer, caps },
        )))
    }

    fn handle(
        training: &mut Training,
        world: &mut World,
        request: TrainingRequest,
    ) -> testing::Outcome<Result<()>> {
        testing::run(|out| training.handle(&ask(request), world, out))
    }

    #[test]
    fn only_the_players_own_guildmaster_in_reach_opens_training() {
        let (mut training, mut world) = warrior();
        for (trainer, reason) in [
            (43, "That is not your guildmaster."),
            (44, "You are too far away from your guildmaster."),
            (99, "You must first target your guildmaster."),
        ] {
            let outcome = handle(&mut training, &mut world, TrainingRequest::Open { trainer });
            assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
            assert_eq!(refusals(&outcome.events), [reason]);
        }
        let outcome = handle(
            &mut training,
            &mut world,
            TrainingRequest::Open { trainer: 42 },
        );
        assert_eq!(outcome.sent, [training::titanium_open(42, 7)]);
        // Before the answer, there is nothing to practice yet.
        let outcome = handle(
            &mut training,
            &mut world,
            TrainingRequest::Train { skill: 30 },
        );
        assert_eq!(
            refusals(&outcome.events),
            ["You are not training with a guildmaster."]
        );
    }

    #[test]
    fn a_practice_spends_a_point_and_its_cost_when_the_skill_rises() {
        let (mut training, mut world) = warrior();
        handle(
            &mut training,
            &mut world,
            TrainingRequest::Open { trainer: 42 },
        )
        .result
        .unwrap();
        testing::run(|out| training.observe(&offer(42), &mut world, out))
            .result
            .unwrap();
        // A skill the guildmaster does not teach is refused.
        let outcome = handle(
            &mut training,
            &mut world,
            TrainingRequest::Train { skill: 10 },
        );
        assert_eq!(
            refusals(&outcome.events),
            ["Your guildmaster cannot train you in that skill."]
        );
        let outcome = handle(
            &mut training,
            &mut world,
            TrainingRequest::Train { skill: 30 },
        );
        assert_eq!(outcome.sent, [training::titanium_train(42, 30).unwrap()]);
        // The server's answer is the practice, with its cost: 2 cubed over
        // a hundred is nothing yet.
        let mut answer = Message::Event(WorldEvent::Skill {
            skill_id: 30,
            value: 13,
        });
        training.explain(&mut answer, &world);
        assert!(matches!(
            answer,
            Message::Event(WorldEvent::Training(TrainingUpdate::Trained {
                skill: 30,
                value: 13,
                cost: 0
            }))
        ));
        let outcome = testing::run(|out| training.observe(&answer, &mut world, out));
        assert!(outcome
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::PracticePoints(2)))));
        // A refusal at the level's cap answers with the same value and costs
        // nothing.
        handle(
            &mut training,
            &mut world,
            TrainingRequest::Train { skill: 30 },
        )
        .result
        .unwrap();
        let mut refused = Message::Event(WorldEvent::Skill {
            skill_id: 30,
            value: 12,
        });
        training.explain(&mut refused, &world);
        assert!(matches!(refused, Message::Event(WorldEvent::Skill { .. })));
        assert!(training.pending.is_empty());
        // Leaving tells the server and the host.
        let outcome = handle(&mut training, &mut world, TrainingRequest::End);
        assert_eq!(outcome.sent, [training::titanium_end(42, 7)]);
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Training(TrainingUpdate::Ended))
        )));
    }

    #[test]
    fn practices_stop_at_the_points_and_coins_the_player_has() {
        let (mut training, mut world) = warrior();
        handle(
            &mut training,
            &mut world,
            TrainingRequest::Open { trainer: 42 },
        )
        .result
        .unwrap();
        testing::run(|out| training.observe(&offer(42), &mut world, out))
            .result
            .unwrap();
        // Three points: three practices go out unanswered, the fourth waits.
        for _ in 0..3 {
            let outcome = handle(
                &mut training,
                &mut world,
                TrainingRequest::Train { skill: 30 },
            );
            assert_eq!(outcome.sent.len(), 1);
        }
        let outcome = handle(
            &mut training,
            &mut world,
            TrainingRequest::Train { skill: 30 },
        );
        assert_eq!(
            refusals(&outcome.events),
            ["You do not have any practice points left."]
        );
        // Unanswered practices are forgotten after a while.
        let later = Instant::now() + ANSWER_WAIT;
        testing::run(|out| training.tick(later, &mut world, out))
            .result
            .unwrap();
        assert!(training.pending.is_empty());
        // A skill of 110 costs a hundred platinum, more than 10 gold.
        if let Some(player) = world.player.corrected() {
            player.skills.as_mut().unwrap()[30] = 110;
        }
        let outcome = handle(
            &mut training,
            &mut world,
            TrainingRequest::Train { skill: 30 },
        );
        assert_eq!(
            refusals(&outcome.events),
            ["You cannot afford to train that skill."]
        );
    }

    fn p99() -> (Training, World) {
        let (_, mut world) = warrior();
        let mut training = Training::project1999();
        testing::run(|out| training.admitted(&mut world, out))
            .result
            .unwrap();
        (training, world)
    }

    fn hear(
        training: &mut Training,
        world: &mut World,
        mut message: Message,
    ) -> (Message, Vec<ClientEvent>) {
        training.explain(&mut message, world);
        let outcome = testing::run(|out| training.observe(&message, world, out));
        outcome.result.unwrap();
        (message, outcome.events)
    }

    fn reply(trainer: u16, trainee: u16, accepted: bool) -> Message {
        let mut body = training::titanium_open(trainer, trainee).body;
        body[412] = u8::from(accepted);
        body[8 + 30 * 4] = 200;
        eq_network_game::message::titanium(training::OFFER_OPCODE, &body).remove(0)
    }

    #[test]
    fn p99_requires_the_requested_trainer_recipient_and_admission_flag() {
        let (mut training, mut world) = p99();
        assert!(matches!(
            hear(&mut training, &mut world, reply(42, 7, true)).0,
            Message::TrainingReply(_)
        ));
        handle(
            &mut training,
            &mut world,
            TrainingRequest::Open { trainer: 42 },
        )
        .result
        .unwrap();
        for message in [reply(43, 7, true), reply(42, 9, true)] {
            assert!(matches!(
                hear(&mut training, &mut world, message).0,
                Message::TrainingReply(_)
            ));
        }
        let (_, events) = hear(&mut training, &mut world, reply(42, 7, false));
        assert_eq!(
            refusals(&events),
            ["The guildmaster declined to open training."]
        );
        assert!(training.trainer.is_none());
        handle(
            &mut training,
            &mut world,
            TrainingRequest::Open { trainer: 42 },
        )
        .result
        .unwrap();
        assert!(matches!(
            hear(&mut training, &mut world, reply(42, 7, true)).0,
            Message::Event(WorldEvent::Training(TrainingUpdate::Offered(_)))
        ));
        hear(
            &mut training,
            &mut world,
            Message::Event(WorldEvent::Despawn(42)),
        );
        assert!(training.trainer.is_none());
        assert!(matches!(
            hear(&mut training, &mut world, reply(42, 7, true)).0,
            Message::TrainingReply(_)
        ));
    }

    #[test]
    fn p99_practice_keeps_skill_updates_separate_and_blocks_an_unanswered_retry() {
        let (mut training, mut world) = p99();
        handle(
            &mut training,
            &mut world,
            TrainingRequest::Open { trainer: 42 },
        )
        .result
        .unwrap();
        hear(&mut training, &mut world, reply(42, 7, true));
        let request = TrainingRequest::TrainAt {
            trainer: 42,
            skill: 30,
        };
        assert!(handle(
            &mut training,
            &mut world,
            TrainingRequest::TrainAt {
                trainer: 43,
                skill: 30
            }
        )
        .sent
        .is_empty());
        assert_eq!(
            handle(&mut training, &mut world, request).sent,
            [training::titanium_train(42, 30).unwrap()]
        );
        assert_eq!(training.points, Some(3));
        assert!(matches!(
            world.take_news().as_slice(),
            [Message::Event(WorldEvent::Training(
                TrainingUpdate::Practice(PracticeUpdate::Requested {
                    skill: 30,
                    estimated_cost: 0,
                })
            ))]
        ));
        assert!(handle(&mut training, &mut world, request).sent.is_empty());
        let skill = |skill_id, value| Message::Event(WorldEvent::Skill { skill_id, value });
        hear(&mut training, &mut world, skill(29, 13));
        hear(&mut training, &mut world, skill(30, 12));
        assert!(world.take_news().is_empty());
        assert!(matches!(
            hear(&mut training, &mut world, skill(30, 13)).0,
            Message::Event(WorldEvent::Skill {
                skill_id: 30,
                value: 13
            })
        ));
        assert!(matches!(
            world.take_news().as_slice(),
            [Message::Event(WorldEvent::Training(
                TrainingUpdate::Practice(PracticeUpdate::SkillRaised {
                    skill: 30,
                    value: 13,
                    estimated_points: Some(2),
                    ..
                })
            ))]
        ));
        handle(&mut training, &mut world, request).result.unwrap();
        let result =
            testing::run(|out| training.tick(Instant::now() + ANSWER_WAIT, &mut world, out));
        assert!(matches!(
            result.events.as_slice(),
            [ClientEvent::World(WorldEvent::Training(
                TrainingUpdate::Practice(PracticeUpdate::Unanswered { skill: 30 })
            ))]
        ));
        assert!(result.sent.is_empty());
        assert!(handle(&mut training, &mut world, request).sent.is_empty());
        assert_eq!(training.points, None);
    }

    #[test]
    fn levels_beyond_the_highest_bring_five_points_each() {
        let (mut training, mut world) = warrior();
        let level = |current, previous| {
            Message::Event(WorldEvent::Level {
                current,
                previous,
                experience: 0,
            })
        };
        let outcome = testing::run(|out| training.observe(&level(12, 10), &mut world, out));
        assert!(outcome
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::PracticePoints(13)))));
        // Losing a level and gaining it back brings nothing.
        testing::run(|out| training.observe(&level(11, 12), &mut world, out))
            .result
            .unwrap();
        let outcome = testing::run(|out| training.observe(&level(12, 11), &mut world, out));
        assert!(outcome.events.is_empty());
        assert_eq!(training.points, Some(13));
    }
}
