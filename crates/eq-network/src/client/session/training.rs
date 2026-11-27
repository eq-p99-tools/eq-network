//! Training skills at an in-reach guildmaster of the player's class.
//! `EQEmu` retains its existing skill and balance projection. P99 follows
//! Titanium local press-time point/coin spending (read 6.683). Received
//! skill values replace local projections without another spend.
//! TAKP and Quarm follow `EQEmu`'s projection with the guildmaster's price
//! scale, and train languages as the `EQMac` program does (6.597): its own
//! gate in its order, then the practice resolved at once, as neither server
//! says anything back.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    message::Message,
    request::Request,
    training::{
        self, PracticeUpdate, TrainingOffer, TrainingReply, TrainingRequest, TrainingSubject,
        TrainingUpdate,
    },
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

/// Mac 0x4e06ef adds five once on an accepted rise whose received prior
/// high-water level is lower. Missing balance authority stays missing.
pub(super) fn eqmac_level_points(points: Option<u32>, current: u8, previous: u8) -> Option<u32> {
    points.map(|points| {
        points.saturating_add(if previous < current {
            POINTS_PER_LEVEL
        } else {
            0
        })
    })
}

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
    /// P99 follows native admission and press-time practice spending.
    p99: bool,
    /// TAKP and Quarm: `EQEmu`'s projection, priced by the guildmaster's
    /// scale, with languages under the `EQMac` program's gate (6.597).
    eqmac: bool,
    /// `EQEmu` replies carry no request/incarnation token. Retiring an
    /// unanswered opening needs fresh admission before that ID can reopen.
    retired_trainers: BTreeSet<u16>,
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

/// The value TAKP and Quarm keep for a skill the player has not trained
/// (254) or cannot have (255): `Client::OPGMTrainSkill` takes 254 as a new
/// skill once its level is reached, and prices a new skill at nothing.
const EQMAC_UNTRAINED: u32 = 254;

/// The `EQMac` program's training lines (read 6.597): a language the player
/// can't learn yet (0x327c), no practice points (0x327d), a language at 100
/// (0x327e, naming it), one at the guildmaster's limit (0x327f), and coins
/// short of the cost (0x2f21, naming the guildmaster).
const EQMAC_LANGUAGE_TOO_YOUNG: u32 = 0x327c;
const EQMAC_NO_PRACTICES: u32 = 0x327d;
const EQMAC_LANGUAGE_MASTERED: u32 = 0x327e;
const EQMAC_TRAINER_LIMIT: u32 = 0x327f;
const EQMAC_CANNOT_AFFORD: u32 = 0x2f21;

impl Training {
    /// P99 uses native admission fields and keeps skill and point authority separate.
    pub(super) fn project1999() -> Self {
        Self {
            p99: true,
            ..Self::default()
        }
    }

    /// TAKP and Quarm (6.597).
    pub(super) fn eqmac() -> Self {
        Self {
            eqmac: true,
            ..Self::default()
        }
    }

    /// Whether a received value raised a skill from `from`: on TAKP and
    /// Quarm an untrained skill's 254 falls to its first trained value.
    fn raised(&self, from: u32, value: u32) -> bool {
        if self.eqmac && from >= EQMAC_UNTRAINED {
            value < EQMAC_UNTRAINED
        } else {
            value > from
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

    /// Opens training with a guildmaster, leaving any other first on `EQEmu`.
    fn open(
        &mut self,
        command: &ClientCommand,
        trainer: u16,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Err(reason) = Self::eligible(trainer, world) {
            return actions::refuse(command, reason, out.log);
        }
        if self.retired_trainers.contains(&trainer) {
            return actions::refuse(
                command,
                "Re-enter the zone before asking this retired guildmaster again.",
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
        if self.p99 {
            if self.trainer.is_some() {
                return actions::refuse(
                    command,
                    "Finish training with the current guildmaster first.",
                    out.log,
                );
            }
        }
        if let Some(open) = self.trainer.take() {
            if open.offer.is_none() {
                self.retired_trainers.insert(open.spawn_id);
            }
            self.retire_eqemu_languages(world, None);
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
    fn practice(
        &self,
        subject: TrainingSubject,
        world: &World,
    ) -> Result<(u16, u32, u64), &'static str> {
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
            if self.points.is_none() {
                return Err("The practice point balance is unknown.");
            }
        }
        // Recheck the live trainer for the new bank as well as the received offer.
        if matches!(subject, TrainingSubject::Language(_)) {
            Self::eligible(trainer, world)?;
        }
        let cap = offer.subject_cap(subject);
        if let TrainingSubject::Language(language) = subject {
            if self
                .pending
                .iter()
                .any(|practice| practice.skill == subject.update_id())
            {
                return Err("Wait for the current language practice outcome.");
            }
            if !world.player.as_ref().is_some_and(|player| {
                training::titanium_language_trainable(
                    language,
                    player.race,
                    player.class,
                    player.level,
                )
            }) {
                return Err("Your guildmaster cannot train you in that language yet.");
            }
        }
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
            .and_then(|player| subject.value(player));
        let value = match value {
            Some(value) => value,
            None if self.p99 || matches!(subject, TrainingSubject::Language(_)) => {
                return Err("Your current skill value is unknown.")
            }
            None => 0,
        };
        // TAKP's and Quarm's untrained skill is a new one, priced at nothing.
        let priced = if self.eqmac && value >= EQMAC_UNTRAINED {
            0
        } else {
            value
        };
        if priced >= cap {
            return Err("You cannot be trained any further in that skill.");
        }
        let cost = match offer.price_scale {
            Some(scale) if self.eqmac => training::eqmac_practice_cost(priced, scale),
            _ => training::practice_cost(priced),
        };
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
        subject: TrainingSubject,
        world: &mut World,
        out: &mut Out<'_, '_>,
        now: Instant,
    ) -> Result<()> {
        if let (true, TrainingSubject::Language(language)) = (self.eqmac, subject) {
            return self.train_eqmac_language(command, language, world, out);
        }
        let (trainer, from, cost) = match self.practice(subject, world) {
            Ok(practice) => practice,
            Err(reason) if self.p99 => {
                let language = matches!(subject, TrainingSubject::Language(_));
                let string_id = match reason {
                    "You do not have any practice points left." => Some(0x327d),
                    "You cannot afford to train that skill." => Some(0x2f21),
                    "Your guildmaster cannot train you in that language yet." => Some(0x327c),
                    "You cannot be trained any further in that skill." => {
                        Some(if language { 0x327f } else { 0x3283 })
                    }
                    _ => None,
                };
                let arguments = if string_id == Some(0x2f21) {
                    self.trainer
                        .as_ref()
                        .and_then(|trainer| world.spawns.visible(trainer.spawn_id))
                        .map(|trainer| vec![trainer.name.clone()])
                        .unwrap_or_default()
                } else if let (TrainingSubject::Language(id), Some(0x327f)) = (subject, string_id) {
                    vec![id.to_string()]
                } else {
                    Vec::new()
                };
                return out
                    .log
                    .send(ClientEvent::World(WorldEvent::TrainingRefused {
                        session_id: world.session_id,
                        reason: reason.into(),
                        string_id,
                        arguments,
                    }));
            }
            Err(reason) => return actions::refuse(command, reason, out.log),
        };
        let skill = subject.update_id();
        match subject {
            TrainingSubject::Skill(skill) => out.request(&Request::Train { trainer, skill })?,
            TrainingSubject::Language(_) => out.request(&Request::Practice { trainer, subject })?,
        }
        if self.p99 {
            // Native 0x56b48e spends at the press, never on skill-up.
            // Commit only after the transport accepts the request.
            self.points = self.points.map(|points| points.saturating_sub(1));
            world.happened(Message::Event(WorldEvent::Training(
                TrainingUpdate::Practiced {
                    subject,
                    from,
                    cost,
                },
            )));
            if let Some(points) = self.points {
                world.happened(Message::Event(WorldEvent::PracticePoints(points)));
            }
        } else {
            self.pending.push_back(Practice {
                skill,
                from,
                cost,
                sent: now,
            });
        }
        Ok(())
    }

    /// Leaves training; with none open there is nothing to leave.
    fn end(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        let Some(trainer) = self.trainer.take() else {
            return Ok(());
        };
        if !self.p99 || trainer.offer.is_some() {
            out.request(&Request::EndTraining(trainer.spawn_id))?;
        }
        if self.p99 {
            self.retire_practice(out)?;
        } else {
            if trainer.offer.is_none() {
                self.retired_trainers.insert(trainer.spawn_id);
            }
            self.retire_eqemu_languages(world, None);
        }
        out.log.send(ClientEvent::World(WorldEvent::Training(
            TrainingUpdate::Ended,
        )))
    }

    /// Drops an offer when its trainer or zone is no longer current.
    fn invalidate_trainer(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if self.p99 {
            self.retire_practice(out)?;
        } else {
            self.retire_eqemu_languages(world, None);
            // An answered offer has consumed its opening response. Only
            // an unanswered opening can be confused with a later request.
            if let Some(trainer) = self
                .trainer
                .as_ref()
                .filter(|trainer| trainer.offer.is_none())
            {
                self.retired_trainers.insert(trainer.spawn_id);
            }
        }
        if self.trainer.take().is_some() {
            out.log.send(ClientEvent::World(WorldEvent::Training(
                TrainingUpdate::Ended,
            )))?;
        }
        Ok(())
    }

    /// Ends the local observation interval while keeping the spend unresolved.
    fn retire_practice(&mut self, out: &mut Out<'_, '_>) -> Result<()> {
        if let Some(practice) = self.pending.pop_front() {
            Self::report_practice(
                PracticeUpdate::Unanswered {
                    skill: practice.skill,
                },
                out,
            )?;
        }
        Ok(())
    }

    /// Retires unresolved `EQEmu` language operations, and on TAKP and Quarm
    /// every unresolved practice. `EQEmu`'s ordinary skill projection remains
    /// unchanged. The event goes through the feature loop so the purse owner
    /// can withdraw potentially spent coins exactly once.
    fn retire_eqemu_languages(&mut self, world: &mut World, expired_at: Option<Instant>) {
        let mut retired = Vec::new();
        // TAKP and Quarm take a skill's cost and point without a word, so an
        // unanswered skill practice is as unknown as a language (6.597 F1).
        let eqmac = self.eqmac;
        self.pending.retain(|practice| {
            let retire = (eqmac || practice.skill >= training::SKILLS)
                && expired_at
                    .is_none_or(|now| now.saturating_duration_since(practice.sent) >= ANSWER_WAIT);
            if retire {
                retired.push((practice.skill, practice.cost));
            }
            !retire
        });
        if !retired.is_empty() {
            self.points = None;
        }
        for (skill, estimated_cost) in retired {
            world.happened(Message::Event(WorldEvent::Training(
                TrainingUpdate::Practice(PracticeUpdate::OutcomeUnknown {
                    skill,
                    estimated_cost,
                }),
            )));
        }
    }

    /// Both reply values require the same live, unexpired opening.
    fn p99_reply_current(&self, reply: &TrainingReply, world: &World) -> bool {
        world.ready()
            && !world.lifecycle.blocks_motion()
            && world.own_spawn.map(u32::from) == Some(reply.trainee)
            && self.trainer.as_ref().is_some_and(|trainer| {
                trainer.spawn_id == reply.offer.trainer
                    && trainer.offer.is_none()
                    && trainer.requested_at.elapsed() < ANSWER_WAIT
            })
            && Self::eligible(reply.offer.trainer, world).is_ok()
    }

    /// Emits local practice status separately from authoritative skill news.
    fn report_practice(update: PracticeUpdate, out: &mut Out<'_, '_>) -> Result<()> {
        out.log.send(ClientEvent::World(WorldEvent::Training(
            TrainingUpdate::Practice(update),
        )))
    }

    /// Tracks P99 trainer lifecycle; received skills never spend again.
    fn observe_p99(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let trainer = self.trainer.as_ref().map(|trainer| trainer.spawn_id);
        match message {
            Message::TrainingReply(reply)
                if !reply.accepted && self.p99_reply_current(reply, world) =>
            {
                self.trainer = None;
                out.log
                    .send(ClientEvent::World(WorldEvent::TrainingRefused {
                        session_id: world.session_id,
                        reason: "The guildmaster declined to open training.".into(),
                        string_id: None,
                        arguments: Vec::new(),
                    }))?;
            }
            Message::Event(WorldEvent::Level {
                current, previous, ..
            }) if current != previous && self.points.take().is_some() => {
                Self::report_practice(PracticeUpdate::PointsUncertain, out)?;
            }
            Message::Event(WorldEvent::Despawn(id)) if trainer == Some(*id) => {
                self.invalidate_trainer(world, out)?;
            }
            Message::Event(WorldEvent::Spawns(spawns))
                if spawns.iter().any(|spawn| trainer == Some(spawn.spawn_id)) =>
            {
                self.invalidate_trainer(world, out)?;
            }
            Message::Event(event) if world.owns_death(event) => {
                self.invalidate_trainer(world, out)?;
            }
            Message::ZoneOffer(_)
            | Message::ZoneAnswer(_)
            | Message::Handoff(_)
            | Message::LoggedOut => {
                self.invalidate_trainer(world, out)?;
            }
            _ => (),
        }
        Ok(())
    }

    /// Practices a language on TAKP or Quarm as the `EQMac` program does
    /// (0x47ea1f, 0x4efc95, 0x4ef6c7; reads 6.597a and its follow-up): its
    /// gate's checks in its order, each refusing with the program's line
    /// and sending nothing, then the coins, then the send. Neither server
    /// answers a language practice (TAKP 44afcbf and Quarm fc75fcf,
    /// `Client::OPGMTrainSkill` and `IncreaseLanguageSkill`: only string
    /// 449), so the practice is resolved at once, as the program resolves
    /// it: a point spent, the cost paid, the language one higher. The next
    /// profile the server sends overwrites that prediction either way.
    fn train_eqmac_language(
        &mut self,
        command: &ClientCommand,
        language: u8,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let (trainer, from, cost) = match self.eqmac_language(language, world) {
            Ok(practice) => practice,
            Err((reason, line, arguments)) => {
                return actions::refuse_naming(command, (reason, line), &arguments, out.log)
            }
        };
        let subject = TrainingSubject::Language(language);
        out.request(&Request::Practice { trainer, subject })?;
        world.happened(Message::Event(WorldEvent::Training(
            TrainingUpdate::Trained {
                skill: subject.update_id(),
                value: from.saturating_add(1),
                cost,
            },
        )));
        Ok(())
    }

    /// The `EQMac` program's checks before a language practice, in its
    /// order, with each refusal's words, the program's line and what that
    /// line names; or the guildmaster, the language's value and the cost the
    /// server will take.
    #[allow(clippy::type_complexity)]
    fn eqmac_language(
        &self,
        language: u8,
        world: &World,
    ) -> Result<(u16, u32, u64), (&'static str, Option<u32>, Vec<String>)> {
        let ours = |reason: &'static str| (reason, None, Vec::new());
        let Some((trainer, offer)) = self.trainer.as_ref().and_then(|trainer| {
            trainer
                .offer
                .as_ref()
                .map(|offer| (trainer.spawn_id, offer))
        }) else {
            return Err(ours("You are not training with a guildmaster."));
        };
        Self::eligible(trainer, world).map_err(ours)?;
        let Some(levels) = world.language_levels.as_ref() else {
            return Err(ours(
                "Language training needs the installed program's language table.",
            ));
        };
        let Some(player) = world.player.as_ref() else {
            return Err(ours("Not in the zone yet"));
        };
        let subject = TrainingSubject::Language(language);
        let Some(value) = subject.value(player) else {
            return Err(ours("Your current skill value is unknown."));
        };
        // The window lists only the languages the player already speaks.
        let cap = offer.subject_cap(subject);
        if value == 0 || cap == 0 {
            return Err(ours("Your guildmaster cannot train you in that language."));
        }
        if !training::eqmac_language_trainable(
            levels,
            language,
            player.race,
            player.class,
            player.level,
        ) {
            return Err((
                "Your guildmaster cannot train you in that language yet.",
                Some(EQMAC_LANGUAGE_TOO_YOUNG),
                Vec::new(),
            ));
        }
        let unanswered = u32::try_from(self.pending.len()).unwrap_or(u32::MAX);
        if self.points.unwrap_or(0) <= unanswered {
            return Err((
                "You do not have any practice points left.",
                Some(EQMAC_NO_PRACTICES),
                Vec::new(),
            ));
        }
        if value >= training::LANGUAGE_CAP {
            return Err((
                "You have mastered that language.",
                Some(EQMAC_LANGUAGE_MASTERED),
                vec![language.to_string()],
            ));
        }
        if value >= cap {
            return Err((
                "Your guildmaster cannot train you any further in that language.",
                Some(EQMAC_TRAINER_LIMIT),
                Vec::new(),
            ));
        }
        // The program checks the purse against its own figure; the server
        // then takes its own (TAKP 44afcbf and Quarm fc75fcf,
        // `TakeMoneyFromPP`) without saying so, which is what is paid here.
        let scale = offer.price_scale.map_or(1.0, training::PriceScale::get);
        let owed: u64 = self.pending.iter().map(|practice| practice.cost).sum();
        let purse = world.coins.purse.map_or(0, |coins| coins.total_copper());
        if purse < owed.saturating_add(training::eqmac_program_cost(value, scale)) {
            let name = world
                .spawns
                .visible(trainer)
                .map(|spawn| spawn.name.clone())
                .unwrap_or_default();
            return Err((
                "You cannot afford to train that language.",
                Some(EQMAC_CANNOT_AFFORD),
                vec![name],
            ));
        }
        let cost = offer
            .price_scale
            .map_or(0, |scale| training::eqmac_practice_cost(value, scale));
        Ok((trainer, value, cost))
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
            TrainingRequest::Train { skill } => self.train(
                command,
                TrainingSubject::Skill(skill),
                world,
                out,
                Instant::now(),
            ),
            TrainingRequest::TrainAt { trainer, skill } => {
                if self.trainer.as_ref().map(|offer| offer.spawn_id) != Some(trainer) {
                    return actions::refuse(
                        command,
                        "That training window is no longer current.",
                        out.log,
                    );
                }
                self.train(
                    command,
                    TrainingSubject::Skill(skill),
                    world,
                    out,
                    Instant::now(),
                )
            }
            TrainingRequest::PracticeAt { trainer, subject } => {
                if self.trainer.as_ref().map(|offer| offer.spawn_id) != Some(trainer) {
                    return actions::refuse(
                        command,
                        "That training window is no longer current.",
                        out.log,
                    );
                }
                self.train(command, subject, world, out, Instant::now())
            }
            TrainingRequest::End => self.end(world, out),
        }
    }

    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if let Some(trainer) = self.trainer.as_ref() {
            if trainer.offer.is_none()
                && now.saturating_duration_since(trainer.requested_at) >= ANSWER_WAIT
            {
                if !self.p99 {
                    self.retired_trainers.insert(trainer.spawn_id);
                }
                self.trainer = None;
                out.log.send(ClientEvent::World(WorldEvent::TrainingRefused {
                    session_id: world.session_id,
                    reason: if self.p99 { "The guildmaster has not answered. The outcome is unknown." } else { "The guildmaster has not answered. The outcome is unknown; re-enter the zone before asking this guildmaster again." }.into(),
                    string_id: None,
                    arguments: Vec::new(),
                }))?;
            } else if self.p99
                && (world.lifecycle.blocks_motion()
                    || Self::eligible(trainer.spawn_id, world).is_err())
            {
                self.invalidate_trainer(world, out)?;
            }
        }
        if !self.p99 {
            self.retire_eqemu_languages(world, Some(now));
            self.pending
                .retain(|practice| now.saturating_duration_since(practice.sent) < ANSWER_WAIT);
            return Ok(());
        }
        if self
            .pending
            .front()
            .is_some_and(|practice| now.saturating_duration_since(practice.sent) >= ANSWER_WAIT)
        {
            self.retire_practice(out)?;
        }
        Ok(())
    }

    /// Scopes opening replies and preserves `EQEmu`'s existing skill projection.
    fn explain(&mut self, message: &mut Message, world: &World) {
        if let Message::TrainingReply(reply) = message {
            let current = if self.p99 {
                reply.accepted && self.p99_reply_current(reply, world)
            } else {
                self.trainer.as_ref().is_some_and(|trainer| {
                    trainer.spawn_id == reply.offer.trainer
                        && trainer.offer.is_none()
                        && trainer.requested_at.elapsed() < ANSWER_WAIT
                }) && world.own_spawn.map(u32::from) == Some(reply.trainee)
                    && Self::eligible(reply.offer.trainer, world).is_ok()
            };
            if current {
                let mut offer = if self.p99 {
                    training::p99_offer(reply.offer.clone())
                } else {
                    reply.offer.clone()
                };
                // The `EQMac` program lists the languages the player already
                // speaks, each to the guildmaster's byte for it (TAKP and
                // Quarm send 201 for every one), and gates the practice
                // itself (6.597); without the installed table, none.
                let eqmac_caps = self.eqmac.then(|| {
                    (0..training::LANGUAGES)
                        .map(|language| {
                            let speaks = world.player.as_ref().is_some_and(|player| {
                                TrainingSubject::Language(language)
                                    .value(player)
                                    .is_some_and(|value| value > 0)
                            });
                            if speaks && world.language_levels.is_some() {
                                u32::from(reply.language_flags[usize::from(language)])
                            } else {
                                0
                            }
                        })
                        .collect()
                });
                offer.language_caps = (0..training::LANGUAGES)
                    .map(|language| {
                        if !self.eqmac
                            && reply.language_flags[usize::from(language)] != 0
                            && world.player.as_ref().is_some_and(|player| {
                                training::titanium_language_trainable(
                                    language,
                                    player.race,
                                    player.class,
                                    player.level,
                                )
                            })
                        {
                            training::LANGUAGE_CAP
                        } else {
                            0
                        }
                    })
                    .collect();
                if let Some(caps) = eqmac_caps {
                    offer.language_caps = caps;
                }
                *message = Message::Event(WorldEvent::Training(TrainingUpdate::Offered(offer)));
            }
            return;
        }
        if self.p99 {
            return;
        }
        let Message::Event(WorldEvent::Skill { skill_id, value }) = message else {
            return;
        };
        let Some(index) = self.pending.iter().position(|practice| {
            practice.skill == *skill_id
                && (practice.skill < training::SKILLS
                    || (practice.sent.elapsed() < ANSWER_WAIT
                        && *value == practice.from.saturating_add(1)))
        }) else {
            return;
        };
        let Some(practice) = self.pending.remove(index) else {
            return;
        };
        if self.raised(practice.from, *value) {
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
        } else {
            let trainer = self.trainer.as_ref().map(|trainer| trainer.spawn_id);
            let invalidated = match message {
                Message::Event(WorldEvent::Despawn(id)) => trainer == Some(*id),
                Message::Event(WorldEvent::Spawns(spawns)) => {
                    spawns.iter().any(|spawn| trainer == Some(spawn.spawn_id))
                }
                Message::Event(event) => world.owns_death(event),
                Message::ZoneOffer(_)
                | Message::ZoneAnswer(_)
                | Message::Handoff(_)
                | Message::LoggedOut => true,
                _ => false,
            };
            if invalidated {
                self.invalidate_trainer(world, out)?;
            }
        }
        let Message::Event(event) = message else {
            return Ok(());
        };
        match event {
            WorldEvent::PracticePoints(points) => {
                self.points = Some(*points);
                Ok(())
            }
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
            WorldEvent::Level {
                current, previous, ..
            } if !self.p99 && *current > self.highest_level => {
                let gained = u32::from(*current - self.highest_level);
                self.highest_level = *current;
                self.points = if self.eqmac {
                    eqmac_level_points(self.points, *current, *previous)
                } else {
                    self.points
                        .map(|points| points.saturating_add(gained * POINTS_PER_LEVEL))
                };
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
        player.race = 1;
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
        for (spawn_id, class, x) in [(42, 20, 5.0), (43, 21, 5.0), (44, 20, 300.0), (45, 20, 5.0)] {
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

    /// A Human warrior at the native threshold for language 5.
    fn language_warrior() -> (Training, World) {
        let (training, mut world) = warrior();
        world.player.corrected().unwrap().level = 20;
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
            TrainingOffer {
                trainer,
                caps,
                language_caps: Vec::new(),
                price_scale: None,
            },
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
    fn expired_eqemu_openings_retire_the_id_without_trapping_a_pending_window() {
        let (mut training, mut world) = warrior();
        open(&mut training, &mut world, 42).result.unwrap();
        training.trainer.as_mut().unwrap().requested_at =
            Instant::now().checked_sub(ANSWER_WAIT).unwrap();
        let outcome = testing::run(|out| training.tick(Instant::now(), &mut world, out));
        outcome.result.unwrap();
        assert!(training.trainer.is_none());
        assert!(refusals(&outcome.events)[0].contains("outcome is unknown"));
        assert!(matches!(
            hear(&mut training, &mut world, reply(42, 7, true)).0,
            Message::TrainingReply(_)
        ));
        let retry = open(&mut training, &mut world, 42);
        assert_eq!(retry.sent, []);
        assert!(refusals(&retry.events)[0].contains("Re-enter the zone"));
        // A different trainer remains usable; a fresh zone admission has a fresh feature.
        assert_eq!(open(&mut training, &mut world, 45).sent.len(), 1);
        let (mut next, mut next_world) = warrior();
        assert_eq!(open(&mut next, &mut next_world, 42).sent.len(), 1);
        assert!(matches!(
            hear(&mut next, &mut next_world, reply(42, 7, true)).0,
            Message::Event(WorldEvent::Training(TrainingUpdate::Offered(_)))
        ));
    }

    #[test]
    fn eqemu_replacement_separates_unanswered_opens_from_uncertain_spend() {
        for offered in [false, true] {
            for despawn_first in [false, true] {
                let (mut training, mut world) = language_warrior();
                world.player.corrected().unwrap().languages = Some(vec![20; 28]);
                open(&mut training, &mut world, 42).result.unwrap();
                if offered {
                    hear(&mut training, &mut world, reply(42, 7, true));
                    assert_eq!(
                        handle(
                            &mut training,
                            &mut world,
                            TrainingRequest::PracticeAt {
                                trainer: 42,
                                subject: TrainingSubject::Language(5)
                            }
                        )
                        .sent
                        .len(),
                        1
                    );
                }
                if despawn_first {
                    hear(
                        &mut training,
                        &mut world,
                        Message::Event(WorldEvent::Despawn(42)),
                    );
                }
                let mut replacement = testing::spawn(42, SpawnKind::Npc);
                replacement.class = Some(20);
                hear(
                    &mut training,
                    &mut world,
                    Message::Event(WorldEvent::Spawns(vec![replacement.clone()])),
                );
                world.spawns.insert(replacement);
                assert!(training.trainer.is_none());
                assert!(training.pending.is_empty());
                assert_eq!(training.points, if offered { None } else { Some(3) });
                assert!(matches!(
                    hear(&mut training, &mut world, reply(42, 7, true)).0,
                    Message::TrainingReply(_)
                ));
                assert_eq!(
                    handle(
                        &mut training,
                        &mut world,
                        TrainingRequest::PracticeAt {
                            trainer: 42,
                            subject: TrainingSubject::Language(5)
                        }
                    )
                    .sent,
                    []
                );
                let reopened = open(&mut training, &mut world, 42);
                if offered {
                    assert_eq!(reopened.sent, [training::titanium_open(42, 7)]);
                    hear(&mut training, &mut world, reply(42, 7, true));
                    // A fresh offer does not restore an unresolved practice balance.
                    assert_eq!(
                        handle(
                            &mut training,
                            &mut world,
                            TrainingRequest::PracticeAt {
                                trainer: 42,
                                subject: TrainingSubject::Language(5)
                            }
                        )
                        .sent,
                        []
                    );
                    assert_eq!(training.points, None);
                } else {
                    assert_eq!(reopened.sent, []);
                    assert!(refusals(&reopened.events)[0].contains("Re-enter the zone"));
                }
            }
        }
    }

    #[test]
    fn an_answered_eqemu_offer_can_reopen_after_no_spend_replacement() {
        for despawn_first in [false, true] {
            let (mut training, mut world) = language_warrior();
            world.player.corrected().unwrap().languages = Some(vec![20; 28]);
            open(&mut training, &mut world, 42).result.unwrap();
            hear(&mut training, &mut world, reply(42, 7, true));
            if despawn_first {
                hear(
                    &mut training,
                    &mut world,
                    Message::Event(WorldEvent::Despawn(42)),
                );
            }
            let mut replacement = testing::spawn(42, SpawnKind::Npc);
            replacement.class = Some(20);
            hear(
                &mut training,
                &mut world,
                Message::Event(WorldEvent::Spawns(vec![replacement.clone()])),
            );
            world.spawns.insert(replacement);
            assert!(training.trainer.is_none());
            assert_eq!(training.points, Some(3));
            assert!(!training.retired_trainers.contains(&42));
            let practice = TrainingRequest::PracticeAt {
                trainer: 42,
                subject: TrainingSubject::Language(5),
            };
            assert_eq!(handle(&mut training, &mut world, practice).sent, []);
            // With no pending Open, a reply cannot silently restore the old offer.
            assert!(matches!(
                hear(&mut training, &mut world, reply(42, 7, true)).0,
                Message::TrainingReply(_)
            ));
            assert_eq!(
                open(&mut training, &mut world, 42).sent,
                [training::titanium_open(42, 7)]
            );
            assert_eq!(
                handle(&mut training, &mut world, practice).sent,
                [],
                "fresh reply required"
            );
            assert!(matches!(
                hear(&mut training, &mut world, reply(42, 7, true)).0,
                Message::Event(WorldEvent::Training(TrainingUpdate::Offered(_)))
            ));
            assert_eq!(
                handle(&mut training, &mut world, practice).sent,
                [training::titanium_practice(42, TrainingSubject::Language(5)).unwrap()]
            );
        }
    }

    #[test]
    fn an_answered_eqemu_offer_can_reopen_after_same_admission_recovery() {
        let (mut training, mut world) = language_warrior();
        world.player.corrected().unwrap().languages = Some(vec![0; 28]);
        open(&mut training, &mut world, 42).result.unwrap();
        hear(&mut training, &mut world, reply(42, 7, true));
        let session_id = world.session_id;
        world.lifecycle.mark_dead();
        hear(
            &mut training,
            &mut world,
            Message::Event(WorldEvent::DeathBind { session_id }),
        );
        assert!(training.trainer.is_none());
        assert!(!training.retired_trainers.contains(&42));
        assert_eq!(training.points, Some(3));
        // Model fixture for the recovered world: retain the same admission,
        // trainer and Training owner. This is not a live recovery claim.
        world.lifecycle = super::super::lifecycle::ZoneLifecycle::default();
        assert_eq!(world.session_id, session_id);
        assert_eq!(
            open(&mut training, &mut world, 42).sent,
            [training::titanium_open(42, 7)]
        );
        let practice = TrainingRequest::PracticeAt {
            trainer: 42,
            subject: TrainingSubject::Language(5),
        };
        assert_eq!(handle(&mut training, &mut world, practice).sent, []);
        hear(&mut training, &mut world, reply(42, 7, true));
        assert_eq!(
            handle(&mut training, &mut world, practice).sent,
            [training::titanium_practice(42, TrainingSubject::Language(5)).unwrap()]
        );
    }

    /// Opens a received language offer with one withheld flag, retaining the
    /// exact cap projection assertion before tests issue a practice.
    fn language_offer_with_one_missing_flag() -> (Training, World) {
        let (mut training, mut world) = language_warrior();
        world.player.corrected().unwrap().languages = Some(vec![20; 28]);
        open(&mut training, &mut world, 42).result.unwrap();
        let mut reply_body = training::titanium_open(42, 7).body;
        reply_body[412] = 1;
        reply_body[413..445].fill(0xc9);
        reply_body[413 + 2] = 0;
        let reply =
            eq_network_game::message::titanium(training::OFFER_OPCODE, &reply_body).remove(0);
        let (offered, _) = hear(&mut training, &mut world, reply);
        let Message::Event(WorldEvent::Training(TrainingUpdate::Offered(offer))) = offered else {
            panic!("no offer");
        };
        let mut expected_caps = vec![100; 26];
        for language in [2, 4, 10, 12, 21, 22, 23, 24, 25] {
            expected_caps[language] = 0;
        }
        assert_eq!(offer.language_caps, expected_caps);
        (training, world)
    }

    #[test]
    fn eqemu_language_offers_use_native_and_server_bounds_and_correlate_the_update_bank() {
        let (mut training, mut world) = language_offer_with_one_missing_flag();
        let request = TrainingRequest::PracticeAt {
            trainer: 42,
            subject: TrainingSubject::Language(5),
        };
        assert_eq!(
            handle(&mut training, &mut world, request).sent,
            [training::titanium_practice(42, TrainingSubject::Language(5)).unwrap()]
        );
        assert_eq!(handle(&mut training, &mut world, request).sent, []);
        assert_eq!(
            handle(
                &mut training,
                &mut world,
                TrainingRequest::PracticeAt {
                    trainer: 42,
                    subject: TrainingSubject::Language(2)
                }
            )
            .sent,
            []
        );
        // The ordinary skill with the same index is not the language response.
        let mut ordinary = Message::Event(WorldEvent::Skill {
            skill_id: 5,
            value: 21,
        });
        training.explain(&mut ordinary, &world);
        assert!(matches!(
            ordinary,
            Message::Event(WorldEvent::Skill { skill_id: 5, .. })
        ));
        assert_eq!(training.pending.len(), 1);
        let (answer, events) = hear(
            &mut training,
            &mut world,
            Message::Event(WorldEvent::Skill {
                skill_id: 105,
                value: 21,
            }),
        );
        assert!(matches!(
            answer,
            Message::Event(WorldEvent::Training(TrainingUpdate::Trained {
                skill: 105,
                value: 21,
                cost: 10
            }))
        ));
        assert!(events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::PracticePoints(2)))));
        assert_eq!(
            handle(
                &mut training,
                &mut world,
                TrainingRequest::PracticeAt {
                    trainer: 43,
                    subject: TrainingSubject::Language(5)
                }
            )
            .sent,
            []
        );
        assert_eq!(
            handle(
                &mut training,
                &mut world,
                TrainingRequest::PracticeAt {
                    trainer: 42,
                    subject: TrainingSubject::Language(26)
                }
            )
            .sent,
            []
        );
        world
            .player
            .corrected()
            .unwrap()
            .languages
            .as_mut()
            .unwrap()[5] = 100;
        assert_eq!(handle(&mut training, &mut world, request).sent, []);
        world.player.corrected().unwrap().languages = None;
        assert_eq!(handle(&mut training, &mut world, request).sent, []);
        handle(&mut training, &mut world, TrainingRequest::End)
            .result
            .unwrap();
        assert_eq!(handle(&mut training, &mut world, request).sent, []);
    }

    #[test]
    fn language_silence_is_not_a_point_or_coin_receipt() {
        let (mut training, mut world) = language_warrior();
        world.player.corrected().unwrap().languages = Some(vec![0; 28]);
        open(&mut training, &mut world, 42).result.unwrap();
        hear(&mut training, &mut world, reply(42, 7, true));
        let request = TrainingRequest::PracticeAt {
            trainer: 42,
            subject: TrainingSubject::Language(5),
        };
        assert_eq!(handle(&mut training, &mut world, request).sent.len(), 1);
        let outcome =
            testing::run(|out| training.tick(Instant::now() + ANSWER_WAIT, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(training.points, None);
        assert!(!outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Training(TrainingUpdate::Trained { .. }))
        )));
        assert_eq!(handle(&mut training, &mut world, request).sent, []);
        assert_eq!(
            world.player.as_ref().unwrap().languages.as_ref().unwrap()[5],
            0
        );
    }

    #[test]
    fn language_eligibility_and_caps_apply_to_eqemu_and_p99() {
        for project1999 in [false, true] {
            for (language, level, value, flag, cap, allowed) in [
                (0, 1, Some(0), true, 100, true),
                (0, 1, Some(100), true, 100, false),
                (1, 9, Some(0), true, 0, false),
                (1, 10, Some(0), true, 100, true),
                (5, 19, Some(20), true, 0, false),
                (5, 20, Some(20), true, 100, true),
                (5, 20, Some(100), true, 100, false),
                (1, 10, Some(0), false, 0, false),
                (1, 10, None, true, 100, false),
                (25, 100, Some(0), true, 0, false),
            ] {
                let (mut training, mut world) = if project1999 { p99() } else { warrior() };
                let player = world.player.corrected().unwrap();
                player.level = level;
                player.languages = value.map(|value| vec![value; 32]);
                open(&mut training, &mut world, 42).result.unwrap();
                let Message::TrainingReply(mut answer) = reply(42, 7, true) else {
                    panic!("expected trainer reply");
                };
                answer.language_flags[usize::from(language)] = u8::from(flag);
                hear(&mut training, &mut world, Message::TrainingReply(answer));
                assert_eq!(
                    training
                        .trainer
                        .as_ref()
                        .unwrap()
                        .offer
                        .as_ref()
                        .unwrap()
                        .subject_cap(TrainingSubject::Language(language)),
                    cap,
                    "P99 {project1999}, language {language}, level {level}, value {value:?}"
                );
                let purse = world.coins.purse;
                let outcome = handle(
                    &mut training,
                    &mut world,
                    TrainingRequest::PracticeAt {
                        trainer: 42,
                        subject: TrainingSubject::Language(language),
                    },
                );
                outcome.result.unwrap();
                if allowed {
                    assert_eq!(
                        outcome.sent,
                        [
                            training::titanium_practice(42, TrainingSubject::Language(language))
                                .unwrap()
                        ]
                    );
                    let practice = training.pending.front().unwrap();
                    assert_eq!(practice.from, u32::from(value.unwrap()));
                    assert_eq!(practice.cost, if value == Some(20) { 10 } else { 0 });
                } else {
                    assert!(outcome.sent.is_empty(), "refusal must encode no command");
                    assert_eq!(refusals(&outcome.events).len(), 1);
                    assert!(training.pending.is_empty());
                    assert_eq!(training.points, Some(3));
                }
                assert_eq!(world.coins.purse, purse);
            }
        }
    }

    #[test]
    fn language_practice_rechecks_native_eligibility_after_the_offer() {
        for project1999 in [false, true] {
            for (race, level) in [(1, 19), (999, 20)] {
                let (mut training, mut world) = if project1999 { p99() } else { warrior() };
                let player = world.player.corrected().unwrap();
                player.level = 20;
                player.languages = Some(vec![20; 32]);
                open_and_admit(&mut training, &mut world, 42);
                assert_eq!(
                    training
                        .trainer
                        .as_ref()
                        .unwrap()
                        .offer
                        .as_ref()
                        .unwrap()
                        .subject_cap(TrainingSubject::Language(5)),
                    100
                );
                let player = world.player.corrected().unwrap();
                player.race = race;
                player.level = level;
                let outcome = handle(
                    &mut training,
                    &mut world,
                    TrainingRequest::PracticeAt {
                        trainer: 42,
                        subject: TrainingSubject::Language(5),
                    },
                );
                outcome.result.unwrap();
                assert_eq!(outcome.sent, []);
                assert_eq!(
                    refusals(&outcome.events),
                    ["Your guildmaster cannot train you in that language yet."]
                );
                assert!(training.pending.is_empty());
                assert_eq!(training.points, Some(3));
            }
        }
    }

    #[test]
    fn p99_language_training_uses_native_eligibility_and_spends_on_each_press() {
        let (mut training, mut world) = p99();
        let player = world.player.corrected().unwrap();
        player.race = 1;
        player.languages = Some(vec![0; 32]);
        open_and_admit(&mut training, &mut world, 42);
        let offer = training.trainer.as_ref().unwrap().offer.as_ref().unwrap();
        assert_eq!(
            (
                offer.language_caps[1],
                offer.language_caps[5],
                offer.language_caps[25]
            ),
            (100, 0, 0)
        );
        let request = TrainingRequest::PracticeAt {
            trainer: 42,
            subject: TrainingSubject::Language(1),
        };
        assert_eq!(handle(&mut training, &mut world, request).sent.len(), 1);
        let (answer, _) = hear(
            &mut training,
            &mut world,
            Message::Event(WorldEvent::Skill {
                skill_id: 101,
                value: 1,
            }),
        );
        assert!(matches!(
            answer,
            Message::Event(WorldEvent::Skill {
                skill_id: 101,
                value: 1
            })
        ));
        assert_eq!(training.points, Some(2));
        assert_eq!(handle(&mut training, &mut world, request).sent.len(), 1);
        assert_eq!(training.points, Some(1));
    }

    #[test]
    fn eqemu_does_not_accept_unrequested_or_wrong_player_language_offers() {
        let (mut training, mut world) = warrior();
        assert!(matches!(
            hear(&mut training, &mut world, reply(42, 7, true)).0,
            Message::TrainingReply(_)
        ));
        open(&mut training, &mut world, 42).result.unwrap();
        for message in [reply(43, 7, true), reply(42, 9, true)] {
            assert!(matches!(
                hear(&mut training, &mut world, message).0,
                Message::TrainingReply(_)
            ));
        }
        assert!(matches!(
            hear(&mut training, &mut world, reply(42, 7, true)).0,
            Message::Event(WorldEvent::Training(TrainingUpdate::Offered(_)))
        ));
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
        world.admitted = Some(Instant::now());
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

    fn reply(trainer: u16, recipient: u16, accepted: bool) -> Message {
        let mut body = training::titanium_open(trainer, recipient).body;
        body[412] = u8::from(accepted);
        body[413..445].fill(0xc9);
        body[8 + 30 * 4] = 200;
        eq_network_game::message::titanium(training::OFFER_OPCODE, &body).remove(0)
    }

    fn open(
        training: &mut Training,
        world: &mut World,
        trainer: u16,
    ) -> testing::Outcome<Result<()>> {
        handle(training, world, TrainingRequest::Open { trainer })
    }

    fn open_and_admit(training: &mut Training, world: &mut World, trainer: u16) {
        let opened = open(training, world, trainer);
        opened.result.unwrap();
        assert_eq!(opened.sent, [training::titanium_open(trainer, 7)]);
        assert!(matches!(
            hear(training, world, reply(trainer, 7, true)).0,
            Message::Event(WorldEvent::Training(TrainingUpdate::Offered(_)))
        ));
    }

    #[test]
    fn p99_requires_the_requested_trainer_recipient_and_admission_flag() {
        let (mut training, mut world) = p99();
        assert!(matches!(
            hear(&mut training, &mut world, reply(42, 7, true)).0,
            Message::TrainingReply(_)
        ));
        open(&mut training, &mut world, 42).result.unwrap();
        for message in [reply(43, 7, true), reply(42, 9, true), reply(42, 9, false)] {
            let (message, events) = hear(&mut training, &mut world, message);
            assert!(matches!(message, Message::TrainingReply(_)));
            assert!(events.is_empty());
        }
        assert!(matches!(
            hear(&mut training, &mut world, reply(42, 7, true)).0,
            Message::Event(WorldEvent::Training(TrainingUpdate::Offered(_)))
        ));
    }

    #[test]
    fn p99_answered_trainers_can_reopen_without_rezoning() {
        for accepted in [true, false] {
            let (mut training, mut world) = p99();
            open(&mut training, &mut world, 42).result.unwrap();
            hear(&mut training, &mut world, reply(42, 7, accepted));
            handle(&mut training, &mut world, TrainingRequest::End)
                .result
                .unwrap();
            assert_eq!(
                open(&mut training, &mut world, 42).sent,
                [training::titanium_open(42, 7)]
            );
            assert_eq!(training.points, Some(3));
        }
    }

    #[test]
    fn positive_and_negative_p99_opening_replies_share_the_deadline() {
        let (mut training, mut world) = p99();
        open(&mut training, &mut world, 42).result.unwrap();
        let now = Instant::now();
        training.trainer.as_mut().unwrap().requested_at = now.checked_sub(ANSWER_WAIT).unwrap();
        for late in [reply(42, 7, false), reply(42, 7, true)] {
            let (message, events) = hear(&mut training, &mut world, late);
            assert!(matches!(message, Message::TrainingReply(_)));
            assert!(events.is_empty());
        }
        let expired = testing::run(|out| training.tick(now, &mut world, out));
        assert_eq!(
            refusals(&expired.events),
            ["The guildmaster has not answered. The outcome is unknown."]
        );
        assert!(training.trainer.is_none());
        assert_eq!(
            open(&mut training, &mut world, 42).sent,
            [training::titanium_open(42, 7)]
        );
    }

    #[test]
    fn p99_failed_transport_commits_no_press_projection() {
        struct Broken;
        impl super::super::feature::Sink for Broken {
            fn send(&mut self, _: &eq_network_game::command::EncodedCommand) -> Result<()> {
                anyhow::bail!("synthetic send failure")
            }
            fn send_unreliable(
                &mut self,
                packet: &eq_network_game::command::EncodedCommand,
            ) -> Result<()> {
                self.send(packet)
            }
            fn last_received_seconds(&self) -> u64 {
                0
            }
        }
        let (mut training, mut world) = p99();
        open_and_admit(&mut training, &mut world, 42);
        let purse = world.coins.purse;
        let failed = testing::run(|out| {
            let mut broken = Broken;
            let mut failing = Out {
                sink: &mut broken,
                log: &mut *out.log,
                wire: out.wire,
                sender: out.sender,
            };
            training.handle(
                &ask(TrainingRequest::TrainAt {
                    trainer: 42,
                    skill: 30,
                }),
                &mut world,
                &mut failing,
            )
        });
        assert!(failed.result.is_err());
        assert_eq!(training.points, Some(3));
        assert_eq!(world.coins.purse, purse);
        assert_eq!(
            world.player.as_ref().unwrap().skills.as_ref().unwrap()[30],
            12
        );
        assert!(matches!(world.take_news().as_slice(), []));
        assert!(training.pending.is_empty());
    }

    #[test]
    fn p99_press_spends_once_and_received_skills_do_not_spend_again() {
        for from in [0, 12] {
            let (mut training, mut world) = p99();
            world.player.corrected().unwrap().skills.as_mut().unwrap()[30] = from;
            open_and_admit(&mut training, &mut world, 42);
            let request = TrainingRequest::TrainAt {
                trainer: 42,
                skill: 30,
            };
            for left in [2, 1, 0] {
                assert_eq!(handle(&mut training, &mut world, request).sent.len(), 1);
                assert_eq!(training.points, Some(left));
                assert!(training.pending.is_empty());
                assert!(matches!(world.take_news().as_slice(),
                    [Message::Event(WorldEvent::Training(TrainingUpdate::Practiced {
                        subject: TrainingSubject::Skill(30), from: got, cost: 0
                    })), Message::Event(WorldEvent::PracticePoints(got_points))]
                    if *got == from && *got_points == left));
                let (_, events) = hear(
                    &mut training,
                    &mut world,
                    Message::Event(WorldEvent::Skill {
                        skill_id: 30,
                        value: from + 1,
                    }),
                );
                assert!(events.is_empty());
                assert_eq!(training.points, Some(left));
            }
            let refused = handle(&mut training, &mut world, request);
            assert_eq!(refused.sent, []);
            assert!(refused.events.iter().any(|event| matches!(
                event,
                ClientEvent::World(WorldEvent::TrainingRefused {
                    string_id: Some(0x327d),
                    ..
                })
            )));
            hear(
                &mut training,
                &mut world,
                Message::Event(WorldEvent::PracticePoints(5)),
            );
            assert_eq!(training.points, Some(5));
            assert_eq!(handle(&mut training, &mut world, request).sent.len(), 1);
        }
    }

    #[test]
    fn p99_cancel_and_replacement_keep_the_local_balance_without_pending_spends() {
        let (mut training, mut world) = p99();
        open_and_admit(&mut training, &mut world, 42);
        handle(
            &mut training,
            &mut world,
            TrainingRequest::TrainAt {
                trainer: 42,
                skill: 30,
            },
        )
        .result
        .unwrap();
        world.take_news();
        handle(&mut training, &mut world, TrainingRequest::End)
            .result
            .unwrap();
        assert_eq!(training.points, Some(2));
        assert!(training.pending.is_empty());
        open_and_admit(&mut training, &mut world, 42);
        handle(
            &mut training,
            &mut world,
            TrainingRequest::TrainAt {
                trainer: 42,
                skill: 30,
            },
        )
        .result
        .unwrap();
        assert_eq!(training.points, Some(1));
        let (mut next, mut fresh) = p99();
        fresh.player.corrected().unwrap().practice_points = Some(7);
        testing::run(|out| next.admitted(&mut fresh, out))
            .result
            .unwrap();
        assert_eq!(next.points, Some(7));
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

    /// The warrior on TAKP or Quarm: kick at 50, tracking (skill 53)
    /// untrained at 254.
    fn mac_warrior() -> (Training, World) {
        let (_, mut world) = warrior();
        let player = world.player.corrected().unwrap();
        let skills = player.skills.as_mut().unwrap();
        skills[30] = 50;
        skills[53] = 254;
        let mut training = Training::eqmac();
        testing::run(|out| training.admitted(&mut world, out))
            .result
            .unwrap();
        (training, world)
    }

    /// TAKP's and Quarm's answer: kick to 200 and tracking to 100, at the
    /// price scale `scale`, every language flagged.
    fn mac_reply(trainer: u16, scale: f32) -> Message {
        let mut body = training::eqmac_open(trainer, 7).body;
        body[4 + 30 * 2..4 + 30 * 2 + 2].copy_from_slice(&200u16.to_le_bytes());
        body[4 + 53 * 2..4 + 53 * 2 + 2].copy_from_slice(&100u16.to_le_bytes());
        body[204..208].copy_from_slice(&scale.to_le_bytes());
        body[208] = 1;
        body[209..241].fill(201);
        eq_network_game::message::eqmac(training::EQMAC_OFFER_OPCODE, &body).remove(0)
    }

    fn mac_handle(
        training: &mut Training,
        world: &mut World,
        request: TrainingRequest,
    ) -> testing::Outcome<Result<()>> {
        testing::run_on(&super::super::wire::EqMac, |out| {
            training.handle(&ask(request), world, out)
        })
    }

    fn mac_open(training: &mut Training, world: &mut World) -> training::TrainingOffer {
        let opened = mac_handle(training, world, TrainingRequest::Open { trainer: 42 });
        opened.result.unwrap();
        assert_eq!(opened.sent, [training::eqmac_open(42, 7)]);
        let (offered, _) = hear(training, world, mac_reply(42, 1.25));
        let Message::Event(WorldEvent::Training(TrainingUpdate::Offered(offer))) = offered else {
            panic!("expected the offer, got {offered:?}");
        };
        assert_eq!(offer.cap(30), 200);
        assert_eq!(offer.price_scale.map(training::PriceScale::get), Some(1.25));
        offer
    }

    /// A made-up language table (no program values): language 0 at 1, 10
    /// and 23 at 99, language 5 at 1 for humans, everything else at 30.
    fn made_up_levels() -> training::LanguageLevels {
        let mut rows = [[30; 14]; training::EQMAC_TABLE_LANGUAGES];
        rows[0] = [1; 14];
        rows[10] = [99; 14];
        rows[23] = [99; 14];
        rows[5][0] = 1;
        training::LanguageLevels::checked(rows).unwrap()
    }

    /// The TAKP or Quarm warrior (a human) with the installed table, who
    /// speaks languages 5 and 6 at 20 and nothing else.
    fn mac_speaker() -> (Training, World) {
        let (training, mut world) = mac_warrior();
        world.language_levels = Some(made_up_levels());
        let mut languages = vec![0; 28];
        languages[5] = 20;
        languages[6] = 20;
        world.player.corrected().unwrap().languages = Some(languages);
        (training, world)
    }

    fn practice_language(language: u8) -> TrainingRequest {
        TrainingRequest::PracticeAt {
            trainer: 42,
            subject: TrainingSubject::Language(language),
        }
    }

    /// Each refusal's program line and what it names.
    fn refusal_lines(events: &[ClientEvent]) -> Vec<(Option<u32>, Vec<String>)> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::TrainingRefused {
                    string_id,
                    arguments,
                    ..
                }) => Some((*string_id, arguments.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_mac_practice_is_priced_by_the_guildmaster_s_scale() {
        let (mut training, mut world) = mac_warrior();
        mac_open(&mut training, &mut world);
        let kick = TrainingRequest::PracticeAt {
            trainer: 42,
            subject: TrainingSubject::Skill(30),
        };
        let sent = mac_handle(&mut training, &mut world, kick);
        sent.result.unwrap();
        assert_eq!(
            sent.sent,
            [training::eqmac_practice(42, 7, TrainingSubject::Skill(30)).unwrap()]
        );
        // 40^3 * 1.25 * 0.0099999998, cut: 799 copper.
        let (trained, events) = hear(
            &mut training,
            &mut world,
            Message::Event(WorldEvent::Skill {
                skill_id: 30,
                value: 51,
            }),
        );
        assert!(
            matches!(
                trained,
                Message::Event(WorldEvent::Training(TrainingUpdate::Trained {
                    skill: 30,
                    value: 51,
                    cost: 799
                }))
            ),
            "{trained:?}"
        );
        assert!(events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::PracticePoints(2)))));
    }

    #[test]
    fn a_mac_untrained_skill_is_free_and_rises_from_254() {
        let (mut training, mut world) = mac_warrior();
        mac_open(&mut training, &mut world);
        let tracking = TrainingRequest::PracticeAt {
            trainer: 42,
            subject: TrainingSubject::Skill(53),
        };
        mac_handle(&mut training, &mut world, tracking)
            .result
            .unwrap();
        let (trained, _) = hear(
            &mut training,
            &mut world,
            Message::Event(WorldEvent::Skill {
                skill_id: 53,
                value: 10,
            }),
        );
        assert!(
            matches!(
                trained,
                Message::Event(WorldEvent::Training(TrainingUpdate::Trained {
                    skill: 53,
                    value: 10,
                    cost: 0
                }))
            ),
            "{trained:?}"
        );
        // A refusal repeats the value and spends nothing.
        mac_handle(&mut training, &mut world, tracking)
            .result
            .unwrap();
        let unchanged = Message::Event(WorldEvent::Skill {
            skill_id: 53,
            value: 254,
        });
        let (heard, events) = hear(&mut training, &mut world, unchanged);
        assert!(
            matches!(
                heard,
                Message::Event(WorldEvent::Skill {
                    skill_id: 53,
                    value: 254
                })
            ),
            "{heard:?}"
        );
        assert!(
            training.pending.is_empty(),
            "the refusal closed the practice"
        );
        assert!(!events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::PracticePoints(_)))));
    }

    #[test]
    fn mac_languages_stay_shut_without_the_installed_table() {
        let (mut training, mut world) = mac_warrior();
        world.player.corrected().unwrap().languages = Some(vec![20; 28]);
        let offer = mac_open(&mut training, &mut world);
        assert_eq!(offer.language_caps, [0; training::LANGUAGES as usize]);
        let held = mac_handle(&mut training, &mut world, practice_language(5));
        held.result.unwrap();
        assert_eq!(held.sent, []);
        assert_eq!(
            refusals(&held.events),
            ["Language training needs the installed program's language table."]
        );
        assert_eq!(refusal_lines(&held.events), [(None, Vec::new())]);
        assert!(world.take_news().is_empty());
    }

    #[test]
    fn the_mac_window_lists_only_the_languages_the_player_speaks() {
        let (mut training, mut world) = mac_speaker();
        let offer = mac_open(&mut training, &mut world);
        let listed: Vec<usize> = (0..offer.language_caps.len())
            .filter(|language| offer.language_caps[*language] != 0)
            .collect();
        assert_eq!(listed, [5, 6]);
        assert_eq!(offer.language_caps[5], 201);
        let unspoken = mac_handle(&mut training, &mut world, practice_language(7));
        assert_eq!(unspoken.sent, []);
        assert_eq!(refusal_lines(&unspoken.events), [(None, Vec::new())]);
    }

    #[test]
    fn the_mac_language_gate_refuses_in_the_programs_order() {
        let (mut training, mut world) = mac_speaker();
        mac_open(&mut training, &mut world);
        training.points = Some(0);
        let refused = |training: &mut Training, world: &mut World, language| {
            let outcome = mac_handle(training, world, practice_language(language));
            outcome.result.unwrap();
            assert_eq!(outcome.sent, [], "nothing is sent on a refusal");
            assert!(world.take_news().is_empty(), "nothing is resolved");
            refusal_lines(&outcome.events)
        };
        // Language 6 at level 10 against the table's 30, before the points.
        assert_eq!(
            refused(&mut training, &mut world, 6),
            [(Some(EQMAC_LANGUAGE_TOO_YOUNG), Vec::new())]
        );
        world.player.corrected().unwrap().level = 30;
        assert_eq!(
            refused(&mut training, &mut world, 6),
            [(Some(EQMAC_NO_PRACTICES), Vec::new())]
        );
        training.points = Some(3);
        world
            .player
            .corrected()
            .unwrap()
            .languages
            .as_mut()
            .unwrap()[6] = 100;
        assert_eq!(
            refused(&mut training, &mut world, 6),
            [(Some(EQMAC_LANGUAGE_MASTERED), vec!["6".to_owned()])]
        );
        world
            .player
            .corrected()
            .unwrap()
            .languages
            .as_mut()
            .unwrap()[6] = 25;
        let offer = training.trainer.as_mut().unwrap().offer.as_mut().unwrap();
        offer.language_caps[6] = 25;
        assert_eq!(
            refused(&mut training, &mut world, 6),
            [(Some(EQMAC_TRAINER_LIMIT), Vec::new())]
        );
    }

    #[test]
    fn a_mac_language_practice_needs_the_programs_figure_in_coins() {
        let (mut training, mut world) = mac_speaker();
        mac_open(&mut training, &mut world);
        // 50 over 10, cubed, at 1.25 and 0.01: 1,562 copper of the 1,000.
        world
            .player
            .corrected()
            .unwrap()
            .languages
            .as_mut()
            .unwrap()[5] = 60;
        let short = mac_handle(&mut training, &mut world, practice_language(5));
        short.result.unwrap();
        assert_eq!(short.sent, []);
        let name = world.spawns.visible(42).unwrap().name.clone();
        assert_eq!(
            refusal_lines(&short.events),
            [(Some(EQMAC_CANNOT_AFFORD), vec![name])]
        );
        // 40 over: 800 copper, which the purse holds.
        world
            .player
            .corrected()
            .unwrap()
            .languages
            .as_mut()
            .unwrap()[5] = 50;
        let paid = mac_handle(&mut training, &mut world, practice_language(5));
        paid.result.unwrap();
        assert_eq!(paid.sent.len(), 1);
    }

    #[test]
    fn a_mac_language_practice_is_resolved_at_once_as_the_program_does() {
        let (mut training, mut world) = mac_speaker();
        mac_open(&mut training, &mut world);
        let sent = mac_handle(&mut training, &mut world, practice_language(5));
        sent.result.unwrap();
        assert_eq!(
            sent.sent,
            [training::eqmac_practice(42, 7, TrainingSubject::Language(5)).unwrap()]
        );
        assert_eq!(refusals(&sent.events), Vec::<String>::new());
        assert!(training.pending.is_empty(), "no answer is awaited");
        let mut news = world.take_news();
        assert_eq!(news.len(), 1, "{news:?}");
        let resolve = news.remove(0);
        let Message::Event(WorldEvent::Training(TrainingUpdate::Trained { skill, value, cost })) =
            &resolve
        else {
            panic!("expected the resolve, got {resolve:?}");
        };
        assert_eq!(*skill, TrainingSubject::Language(5).update_id());
        assert_eq!(*value, 21);
        // The server's own figure, which it takes without a word: 10 over
        // 10, cubed, at 1.25, cut.
        assert_eq!(
            *cost,
            training::eqmac_practice_cost(20, training::PriceScale::new(1.25))
        );
        let (_, events) = hear(&mut training, &mut world, resolve);
        assert!(events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::PracticePoints(2)))));
    }

    /// The resolve is the program's prediction (G1, G5): the character
    /// applies it, and the next profile the server sends replaces it,
    /// raised or not, with nothing more said.
    #[test]
    fn the_next_profile_overwrites_a_mac_language_prediction() {
        for kept in [21u8, 20] {
            let (mut training, mut world) = mac_speaker();
            mac_open(&mut training, &mut world);
            mac_handle(&mut training, &mut world, practice_language(5))
                .result
                .unwrap();
            let news = world.take_news();
            let Message::Event(WorldEvent::Training(TrainingUpdate::Trained {
                skill, value, ..
            })) = &news[0]
            else {
                panic!("expected the resolve, got {news:?}");
            };
            let player = world.player.corrected().unwrap();
            player.apply_skill(*skill, *value);
            assert_eq!(player.languages.as_ref().unwrap()[5], 21);
            let mut profile = player.clone();
            profile.languages.as_mut().unwrap()[5] = kept;
            world.player.admit(profile);
            let player = world.player.as_ref().unwrap();
            assert_eq!(player.languages.as_ref().unwrap()[5], kept);
        }
    }

    #[test]
    fn leaving_mac_training_sends_the_four_byte_goodbye() {
        let (mut training, mut world) = mac_warrior();
        mac_open(&mut training, &mut world);
        let left = mac_handle(&mut training, &mut world, TrainingRequest::End);
        left.result.unwrap();
        assert_eq!(left.sent, [training::eqmac_end(42, 7)]);
    }

    #[test]
    fn an_unanswered_mac_practice_leaves_the_purse_and_points_unknown() {
        let (mut training, mut world) = mac_warrior();
        mac_open(&mut training, &mut world);
        let kick = TrainingRequest::PracticeAt {
            trainer: 42,
            subject: TrainingSubject::Skill(30),
        };
        mac_handle(&mut training, &mut world, kick).result.unwrap();
        let sent = training.pending.front().unwrap().sent;
        let early = (sent + ANSWER_WAIT)
            .checked_sub(Duration::from_millis(1))
            .unwrap();
        testing::run(|out| training.tick(early, &mut world, out))
            .result
            .unwrap();
        assert!(world.take_news().is_empty());
        assert_eq!(training.points, Some(3));
        testing::run(|out| training.tick(sent + ANSWER_WAIT, &mut world, out))
            .result
            .unwrap();
        assert!(training.pending.is_empty());
        assert_eq!(training.points, None, "the server may have taken a point");
        let news = world.take_news();
        assert!(
            matches!(
                news.as_slice(),
                [Message::Event(WorldEvent::Training(
                    TrainingUpdate::Practice(PracticeUpdate::OutcomeUnknown {
                        skill: 30,
                        estimated_cost: 799
                    })
                ))]
            ),
            "{news:?}"
        );
        // A late answer is plain skill news, never a practice.
        let (late, _) = hear(
            &mut training,
            &mut world,
            Message::Event(WorldEvent::Skill {
                skill_id: 30,
                value: 51,
            }),
        );
        assert!(matches!(
            late,
            Message::Event(WorldEvent::Skill {
                skill_id: 30,
                value: 51
            })
        ));
    }
}
