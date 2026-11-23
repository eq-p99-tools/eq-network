//! Rolling dice, emoting, assisting and yelling for help. The server rolls a die for every
//! player within 400 units, the one who asked among them, and answers an
//! assist with the target to take. It passes an emote on to everyone near
//! but the one who made it (`EQEmu`'s `Client::Handle_OP_Emote`), so the
//! session records the player's own emote as the others hear it, as the
//! official client shows it (inferred). The official client refuses to
//! assist the player themself (inferred from its string for it); `EQEmu`
//! answers with the player's own target. Titanium's `/yell` (0x46791a,
//! read R45) sends an empty `OP_YellForHelp` with no target or posture
//! test, and the server passes it on to the players near.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand,
};
use crate::client::RecordEvent;
use anyhow::Result;
use eq_network_game::{
    message::Message,
    request::Request,
    socials,
    world::{PostureState, WorldEvent},
};
use std::time::{Duration, Instant};

/// The official client's string for assisting oneself (`eqstr_us.txt`).
const ASSIST_SELF: u32 = 1403;
/// The Titanium random command's inclusive upper endpoint (0x4669ab).
const P99_ROLL_MAX: u32 = 32_000;
/// Inferred local lifetime of an assist selection (6.404). The request still
/// owns the untagged reply after this time, but can no longer change target.
const ASSIST_SELECTION: Duration = Duration::from_secs(5);

struct PendingAssist {
    assistee: u16,
    target_revision: u64,
    sent: Instant,
    superseded: bool,
}

/// How long an assist on TAKP or Quarm waits for its answer before another
/// may go. Both servers always answer, untagged (6.585, ours: the `EQMac`
/// program's own pace is unread, 6.585c).
const EQMAC_ASSIST_WAIT: Duration = Duration::from_secs(5);

/// Rolls, emotes, assists and yells.
#[derive(Default)]
pub(super) struct Socials {
    project1999: bool,
    assist: Option<PendingAssist>,
    /// TAKP and Quarm: rolls, assists, emotes and yells.
    eqmac: bool,
    /// There, when the last assist went, while it is unanswered.
    eqmac_assist: Option<Instant>,
}

impl Socials {
    pub(super) fn project1999() -> Self {
        Self {
            project1999: true,
            ..Self::default()
        }
    }

    /// TAKP and Quarm, each from its own `Handle_OP_RandomReq` and
    /// `Handle_OP_Assist` (Quarm fc75fcf, TAKP 44afcbf; 6.585): rolls, with
    /// P99's ends of 0 to 32000 in order (ours: the `EQMac` program's limit
    /// is unread, 6.585b), and one assist at a time; emotes and yells from
    /// their own `Handle_OP_Emote` and `Handle_OP_YellForHelp` (6.604 E1,
    /// E2). Emote animations stay Titanium's.
    pub(super) fn eqmac() -> Self {
        Self {
            eqmac: true,
            ..Self::default()
        }
    }

    /// Whether rolls take P99's ends, in order.
    const fn bounded_rolls(&self) -> bool {
        self.project1999 || self.eqmac
    }
}

impl Feature for Socials {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        if self.eqmac {
            return vec![
                Capability::Rolling,
                Capability::Emoting,
                Capability::Assisting,
                Capability::Yelling,
            ];
        }
        vec![
            Capability::Rolling,
            Capability::Emoting,
            Capability::Assisting,
            Capability::Yelling,
        ]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        if self.eqmac {
            return matches!(
                command,
                ClientCommand::Random { .. }
                    | ClientCommand::Assist { .. }
                    | ClientCommand::Emote { .. }
                    | ClientCommand::SocialEmote { .. }
                    | ClientCommand::YellForHelp { .. }
            );
        }
        matches!(
            command,
            ClientCommand::Random { .. }
                | ClientCommand::Emote { .. }
                | ClientCommand::SocialEmote { .. }
                | ClientCommand::Assist { .. }
                | ClientCommand::YellForHelp { .. }
                | ClientCommand::EmoteAnimation { .. }
        )
    }

    fn explain(&mut self, message: &mut Message, world: &World) {
        if self.eqmac {
            if matches!(message, Message::Event(WorldEvent::Assisted(_))) {
                self.eqmac_assist = None;
            }
            return;
        }
        if !self.project1999 {
            return;
        }
        if let Some(pending) = &mut self.assist {
            pending.superseded |= matches!(
                message,
                Message::ZoneOffer(_) | Message::ZoneAnswer(_) | Message::Handoff(_)
            );
        }
        let Message::Event(event) = message else {
            return;
        };
        if let WorldEvent::Assisted(answer) = event {
            let current = self.assist.take().is_some_and(|pending| {
                !pending.superseded
                    && pending.sent.elapsed() < ASSIST_SELECTION
                    && pending.target_revision == world.target.state().revision
                    && world.visible(pending.assistee)
                    && !world.lifecycle.blocks_motion()
            });
            *message = answer
                .target
                .filter(|id| current && world.visible(*id))
                .map_or(Message::Withheld, Message::AssistSelection);
        } else if let Some(pending) = &mut self.assist {
            pending.superseded |= world.owns_death(event)
                || matches!(event, WorldEvent::Despawn(id) if *id == pending.assistee)
                || matches!(event, WorldEvent::Death(death) if death.spawn_id == u32::from(pending.assistee))
                || matches!(event, WorldEvent::Spawns(spawns) if spawns.iter().any(|spawn| spawn.spawn_id == pending.assistee));
        }
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match command {
            ClientCommand::Random { low, high, .. } => {
                let (mut low, mut high) = (*low, *high);
                if self.bounded_rolls() {
                    if low.max(high) > P99_ROLL_MAX {
                        return actions::refuse(
                            command,
                            "Use dice endpoints from 0 to 32000.",
                            out.log,
                        );
                    }
                    if low > high {
                        std::mem::swap(&mut low, &mut high);
                    }
                }
                out.request(&Request::Random { low, high })
            }
            ClientCommand::YellForHelp { .. } => out.request(&Request::YellForHelp),
            // The program sends the animation only while the player stands
            // still (0x49e0c1, read R47); the host checks the stillness, and
            // the session sends nothing for a player it knows isn't standing.
            ClientCommand::EmoteAnimation { animation, .. } => animate(*animation, world, out),
            ClientCommand::Emote { text, .. } => self.emote(command, text, out),
            ClientCommand::SocialEmote { text, .. } => self.social_emote(command, text, out),
            ClientCommand::Assist { spawn_id, .. } => {
                if world.own_spawn == Some(*spawn_id) {
                    return actions::refuse_officially(
                        command,
                        ("You cannot take your own target.", Some(ASSIST_SELF)),
                        out.log,
                    );
                }
                if self.eqmac {
                    return self.eqmac_assist(command, *spawn_id, out);
                }
                if !self.project1999 {
                    return out.request(&Request::Assist(*spawn_id));
                }
                if let Some(pending) = &self.assist {
                    let reason = if pending.sent.elapsed() < ASSIST_SELECTION {
                        "Wait for the previous assist reply."
                    } else {
                        "The previous assist is unanswered. Wait for its reply or re-enter the zone."
                    };
                    return actions::refuse(command, reason, out.log);
                }
                if world.visible_spawn(*spawn_id).is_none_or(|spawn| {
                    !matches!(
                        spawn.kind,
                        crate::world::SpawnKind::Player | crate::world::SpawnKind::Npc
                    )
                }) {
                    return actions::refuse(
                        command,
                        "Choose a nearby player or creature to assist.",
                        out.log,
                    );
                }
                out.request(&Request::Assist(*spawn_id))?;
                self.assist = Some(PendingAssist {
                    assistee: *spawn_id,
                    target_revision: world.target.state().revision,
                    sent: Instant::now(),
                    superseded: false,
                });
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// Sends an emote's animation, unless the player has no spawn or is known
/// not to stand.
fn animate(animation: u8, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
    if world.own_spawn.is_none()
        || world
            .posture
            .current()
            .is_some_and(|posture| posture != PostureState::Standing)
    {
        return out
            .log
            .diagnostic("An emote's animation needs the player standing".into());
    }
    out.request(&Request::EmoteAnimation(animation))
}

impl Socials {
    /// An ordinary `/emote`, which the player hears as the others do.
    fn emote(&self, command: &ClientCommand, text: &str, out: &mut Out<'_, '_>) -> Result<()> {
        let text = text.trim();
        if text.is_empty() {
            return actions::refuse(command, "Say what you do.", out.log);
        }
        if text.len() >= socials::EMOTE_LONGEST {
            return actions::refuse(command, "That emote is too long.", out.log);
        }
        if self.project1999 {
            let Ok(packet) = socials::project1999_emote(text) else {
                return actions::refuse(
                    command,
                    "That emote is too long or contains invalid text.",
                    out.log,
                );
            };
            out.send(&packet)?;
            let event = socials::local_emote(out.sender.name, text);
            let zone = out.log.zone.clone();
            return out.log.record(&zone, RecordEvent::Chat(event));
        }
        out.request(&Request::Emote(text.to_owned()))?;
        // The player hears their emote as the others do: the
        // servers pass it to everyone near but its maker.
        let (opcode, body) = if self.eqmac {
            (
                socials::EQMAC_EMOTE_OPCODE,
                socials::eqmac_heard_emote(out.sender.name, text),
            )
        } else {
            (
                socials::EMOTE_OPCODE,
                socials::heard_emote(out.sender.name, text),
            )
        };
        if let Some(event) = out.wire.chat(opcode, &body, false)? {
            let zone = out.log.zone.clone();
            out.log.record(&zone, RecordEvent::Chat(event))?;
        }
        Ok(())
    }

    /// Asks TAKP or Quarm to assist, one assist at a time until its answer
    /// or [`EQMAC_ASSIST_WAIT`].
    fn eqmac_assist(
        &mut self,
        command: &ClientCommand,
        spawn_id: u16,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if self
            .eqmac_assist
            .is_some_and(|sent| sent.elapsed() < EQMAC_ASSIST_WAIT)
        {
            return actions::refuse(command, "Wait for the previous assist reply.", out.log);
        }
        out.request(&Request::Assist(spawn_id))?;
        self.eqmac_assist = Some(Instant::now());
        Ok(())
    }

    /// A social emote's own line is the client's to print, so nothing is
    /// echoed; its text goes as the program built it.
    fn social_emote(
        &self,
        command: &ClientCommand,
        text: &str,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if text.trim().is_empty() {
            return actions::refuse(command, "Say what you do.", out.log);
        }
        if text.len() >= socials::EMOTE_LONGEST {
            return actions::refuse(command, "That emote is too long.", out.log);
        }
        if self.project1999 {
            // Its sender puts the space after the name back itself.
            let words = text.strip_prefix(' ').unwrap_or(text);
            let Ok(packet) = socials::project1999_emote(words) else {
                return actions::refuse(
                    command,
                    "That emote is too long or contains invalid text.",
                    out.log,
                );
            };
            return out.send(&packet);
        }
        out.request(&Request::SocialEmote(text.to_owned()))
    }
}

/// How long a `/surname` waits for its answer before another may go.
/// `EQEmu` refuses with a line of its own and sends no answer, so the wait
/// lapses rather than holding until the zone changes (inferred).
const SURNAME_WAIT: Duration = Duration::from_secs(10);

/// Asking for a last name: `/surname`, sent as typed. The server answers
/// with the request's record ([`WorldEvent::SurnameAnswer`]) or a line of
/// its own for each refusal (`EQEmu`'s `Client::Handle_OP_Surname`: its
/// timer, level 12940, length 12942 and filter 1374), so the session adds
/// no line; a surname it takes reaches the zone's spawns as their last
/// name. One request waits at a time, and an answer to none is withheld.
/// Stock `EQEmu` lists it from source (checklist 6.507), and TAKP and
/// Quarm from theirs ([`Surnames::eqmac`], 6.604 E5); none is checked live.
#[derive(Default)]
pub(super) struct Surnames {
    asked: Option<Instant>,
    eqmac: bool,
}

/// The level TAKP's and Quarm's `Handle_OP_Surname` say the client
/// requires before it asks (`SURNAME_LEVEL`).
const EQMAC_SURNAME_LEVEL: u8 = 20;
/// The longest surname they say the client sends: it cuts it to 19.
const EQMAC_SURNAME_LONGEST: usize = 19;

impl Surnames {
    /// TAKP and Quarm, from their own `Handle_OP_Surname` (Quarm fc75fcf,
    /// TAKP 44afcbf; 6.604 E5). Their comments say the client itself
    /// refuses under level 20 and cuts a surname to 19 characters, so the
    /// session refuses those first: inferred from those comments, and never
    /// more permissive than the servers, which refuse both too. Every other
    /// case (a surname already, the name filter, the content flag) is the
    /// server's line.
    pub(super) fn eqmac() -> Self {
        Self {
            eqmac: true,
            ..Self::default()
        }
    }

    fn waiting(&self) -> bool {
        self.asked
            .is_some_and(|asked| asked.elapsed() < SURNAME_WAIT)
    }
}

impl Feature for Surnames {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Surnaming]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::Surname { .. })
    }

    fn explain(&mut self, message: &mut Message, _world: &World) {
        match message {
            Message::ZoneOffer(_) | Message::ZoneAnswer(_) | Message::Handoff(_) => {
                self.asked = None;
            }
            Message::Event(WorldEvent::SurnameAnswer { .. }) => {
                if !self.waiting() {
                    *message = Message::Withheld;
                }
                self.asked = None;
            }
            _ => {}
        }
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::Surname { surname, .. } = command else {
            return Ok(());
        };
        if self.waiting() {
            return actions::refuse(
                command,
                "Wait for the answer to your last surname.",
                out.log,
            );
        }
        let surname = surname.trim();
        if surname.is_empty() || !surname.bytes().all(|byte| byte.is_ascii_graphic()) {
            return actions::refuse(command, "A surname is one word of plain text.", out.log);
        }
        if surname.len() >= eq_network_game::names::SURNAME_FIELD
            || (self.eqmac && surname.len() > EQMAC_SURNAME_LONGEST)
        {
            return actions::refuse(command, "That surname is too long.", out.log);
        }
        if self.eqmac
            && world
                .player
                .as_ref()
                .is_none_or(|player| player.level < EQMAC_SURNAME_LEVEL)
        {
            return actions::refuse(command, "You must be level 20 to take a surname.", out.log);
        }
        out.request(&Request::Surname(surname.to_owned()))?;
        self.asked = Some(Instant::now());
        Ok(())
    }
}

/// How long a request for the title list waits for its answer before
/// another may go (inferred: `EQEmu` always answers, so this only frees a
/// request whose answer was lost).
const TITLES_WAIT: Duration = Duration::from_secs(10);
/// The least time between an answered request for the list, which draws an
/// answer of any size, and the next: the session's usual settling.
const TITLES_SETTLING: Duration = super::listing::SETTLING;

/// Choosing a title or suffix: asking for the list the player may choose
/// from, and choosing from it. `EQEmu` answers the request with the list
/// (`Client::Handle_OP_RequestTitles`) and ignores, with no line, a choice
/// of a title the character may not have (`Client::Handle_OP_SetTitle`), so
/// the session sends only a title this admission's latest list named, of a
/// kind it has text for, or 0 to clear either kind. A list nobody asked for
/// is withheld and told as a gap. Only stock `EQEmu` lists it, from source,
/// not yet checked live; nothing in a front end calls it until the program's
/// title window is read (R49-14).
#[derive(Default)]
pub(super) struct Titles {
    asked: Option<Instant>,
    answered: Option<Instant>,
    chosen: Option<Instant>,
    listed: Option<Vec<eq_network_game::names::TitleChoice>>,
    unasked: Option<usize>,
}

impl Titles {
    fn waiting(&self) -> bool {
        self.asked
            .is_some_and(|asked| asked.elapsed() < TITLES_WAIT)
    }

    fn settling(&self) -> bool {
        self.answered
            .is_some_and(|answered| answered.elapsed() < TITLES_SETTLING)
    }
}

impl Feature for Titles {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Titling]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::RequestTitles { .. } | ClientCommand::SetTitle { .. }
        )
    }

    /// A new admission forgets the list and any request in flight.
    fn admitted(&mut self, _world: &mut World, _out: &mut Out<'_, '_>) -> Result<()> {
        *self = Self::default();
        Ok(())
    }

    fn explain(&mut self, message: &mut Message, _world: &World) {
        match message {
            Message::ZoneOffer(_) | Message::ZoneAnswer(_) | Message::Handoff(_) => {
                self.asked = None;
            }
            Message::Event(WorldEvent::TitleList(titles)) => {
                if self.waiting() {
                    self.listed = Some(titles.clone());
                    self.answered = Some(Instant::now());
                } else {
                    self.unasked = Some(eq_network_game::names::title_list_length(titles));
                    *message = Message::Withheld;
                }
                self.asked = None;
            }
            _ => {}
        }
    }

    fn tick(&mut self, _now: Instant, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if let Some(length) = self.unasked.take() {
            out.log.gap_unsampled(
                eq_network_game::names::TITLE_LIST_OPCODE,
                Some("OP_SendTitleList"),
                length,
                crate::client::GapKind::Unread {
                    reason: "a title list no request asked for".to_owned(),
                },
            )?;
        }
        Ok(())
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match command {
            ClientCommand::RequestTitles { .. } => {
                if self.waiting() || self.settling() {
                    return actions::refuse(command, "Wait for the list of titles.", out.log);
                }
                out.request(&Request::RequestTitles)?;
                self.asked = Some(Instant::now());
                Ok(())
            }
            ClientCommand::SetTitle {
                title_id, suffix, ..
            } => {
                let has_text = |title: &eq_network_game::names::TitleChoice| {
                    title.id == *title_id
                        && !if *suffix {
                            &title.suffix
                        } else {
                            &title.prefix
                        }
                        .is_empty()
                };
                let listed = self
                    .listed
                    .as_ref()
                    .is_some_and(|listed| listed.iter().any(has_text));
                // Each choice the server takes it tells the whole zone.
                if self
                    .chosen
                    .is_some_and(|chosen| chosen.elapsed() < TITLES_SETTLING)
                {
                    return actions::refuse(
                        command,
                        "Wait a moment before choosing again.",
                        out.log,
                    );
                }
                if *title_id != 0 && !listed {
                    return actions::refuse(
                        command,
                        "Choose a title from the latest list, of a kind it has.",
                        out.log,
                    );
                }
                out.request(&Request::SetTitle {
                    title_id: *title_id,
                    suffix: *suffix,
                })?;
                self.chosen = Some(Instant::now());
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, ClientEvent};
    use super::*;

    #[test]
    fn titles_are_asked_for_once_and_chosen_only_from_the_latest_list() {
        use eq_network_game::names::{set_title, TitleChoice};
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut feature = Titles::default();
        let refused = |outcome: &testing::Outcome<Result<()>>| {
            outcome.sent.is_empty()
                && outcome.events.iter().any(|event| {
                    matches!(
                        event,
                        ClientEvent::World(WorldEvent::SocialRefused { session_id: 5, .. })
                    )
                })
        };
        let choose = |feature: &mut Titles, world: &mut World, title_id, suffix| {
            let command = ClientCommand::SetTitle {
                session_id: 5,
                title_id,
                suffix,
            };
            testing::run(|out| feature.handle(&command, world, out))
        };
        // Nothing listed yet: only clearing goes, of either kind.
        assert!(refused(&choose(&mut feature, &mut world, 7, false)));
        for suffix in [false, true] {
            let cleared = choose(&mut feature, &mut world, 0, suffix);
            assert_eq!(cleared.sent[0].body, set_title(0, suffix));
            // The next choice, a clear too, waits out the settling.
            assert!(refused(&choose(&mut feature, &mut world, 0, suffix)));
            feature.chosen = Instant::now().checked_sub(TITLES_SETTLING);
        }

        // A list nobody asked for is withheld, names nothing, and is a gap.
        let list = || {
            Message::Event(WorldEvent::TitleList(vec![TitleChoice {
                id: 7,
                prefix: "Lord".into(),
                suffix: String::new(),
            }]))
        };
        let mut unasked = list();
        feature.explain(&mut unasked, &world);
        assert!(matches!(unasked, Message::Withheld));
        let told = testing::run(|out| feature.tick(Instant::now(), &mut world, out));
        assert!(told
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::Gap(_))));
        assert!(refused(&choose(&mut feature, &mut world, 7, false)));

        let ask = ClientCommand::RequestTitles { session_id: 5 };
        let asked = testing::run(|out| feature.handle(&ask, &mut world, out));
        assert_eq!(
            (asked.sent[0].opcode, asked.sent[0].body.len()),
            (eq_network_game::names::REQUEST_TITLES_OPCODE, 0)
        );
        // A second waits for the answer.
        assert!(refused(&testing::run(
            |out| feature.handle(&ask, &mut world, out)
        )));
        let mut answer = list();
        feature.explain(&mut answer, &world);
        assert!(matches!(answer, Message::Event(_)));
        // Answered, the next still waits out the settling.
        assert!(refused(&testing::run(
            |out| feature.handle(&ask, &mut world, out)
        )));
        let chosen = choose(&mut feature, &mut world, 7, false);
        assert_eq!(
            (chosen.sent[0].opcode, &chosen.sent[0].body),
            (
                eq_network_game::names::SET_TITLE_OPCODE,
                &set_title(7, false)
            )
        );
        feature.chosen = None;
        // Not listed, or a kind the title has no text for.
        assert!(refused(&choose(&mut feature, &mut world, 9, false)));
        assert!(refused(&choose(&mut feature, &mut world, 7, true)));

        // Settled, a fresh request goes; one whose answer never comes lapses.
        feature.answered = Instant::now().checked_sub(TITLES_SETTLING);
        assert_eq!(
            testing::run(|out| feature.handle(&ask, &mut world, out))
                .sent
                .len(),
            1
        );
        assert!(refused(&testing::run(
            |out| feature.handle(&ask, &mut world, out)
        )));
        feature.asked = Instant::now().checked_sub(TITLES_WAIT);
        assert_eq!(
            testing::run(|out| feature.handle(&ask, &mut world, out))
                .sent
                .len(),
            1
        );
        // A new admission forgets the list and the request in flight.
        testing::run(|out| feature.admitted(&mut world, out));
        assert!(feature.listed.is_none() && feature.asked.is_none() && feature.answered.is_none());
        assert!(refused(&choose(&mut feature, &mut world, 7, false)));
    }

    #[test]
    fn a_surname_goes_to_the_server_as_typed_one_at_a_time() {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut feature = Surnames::default();
        let mut run = |world: &mut World, surname: &str| {
            let command = ClientCommand::Surname {
                session_id: 5,
                surname: surname.into(),
            };
            testing::run(|out| feature.handle(&command, world, out))
        };
        let refused = |outcome: &testing::Outcome<Result<()>>| {
            outcome.sent.is_empty()
                && outcome.events.iter().any(|event| {
                    matches!(
                        event,
                        ClientEvent::World(WorldEvent::SocialRefused { session_id: 5, .. })
                    )
                })
        };
        for word in ["", "Ex ample", "Exämple", &"a".repeat(32)] {
            assert!(refused(&run(&mut world, word)), "{word:?}");
        }
        let asked = run(&mut world, " ex-emplum ");
        let sender = eq_network_game::request::Sender {
            name: "Tester",
            spawn_id: Some(7),
        };
        assert_eq!(
            asked.sent,
            [
                eq_network_game::request::titanium(&Request::Surname("ex-emplum".into()), sender)
                    .unwrap()
            ]
        );
        // A second waits for the first one's answer.
        assert!(refused(&run(&mut world, "Exemplum")));
    }

    #[test]
    fn a_mac_surname_is_100_bytes_and_refused_first_only_where_the_servers_say_the_client_does() {
        let wire = &super::super::wire::EqMac;
        let refused = |outcome: &testing::Outcome<Result<()>>| {
            outcome.sent.is_empty()
                && outcome.events.iter().any(|event| {
                    matches!(
                        event,
                        ClientEvent::World(WorldEvent::SocialRefused { session_id: 5, .. })
                    )
                })
        };
        let at_level = |level| {
            let mut world = World::new(5);
            let mut player = testing::player(7);
            player.level = level;
            world.player.admit(player);
            world
        };
        let ask = |feature: &mut Surnames, world: &mut World, surname: &str| {
            let command = ClientCommand::Surname {
                session_id: 5,
                surname: surname.into(),
            };
            testing::run_on(wire, |out| feature.handle(&command, world, out))
        };
        // Under level 20, and over 19 characters: refused before sending.
        assert!(refused(&ask(
            &mut Surnames::eqmac(),
            &mut at_level(19),
            "Exemplum"
        )));
        assert!(refused(&ask(
            &mut Surnames::eqmac(),
            &mut at_level(20),
            &"a".repeat(20)
        )));
        // 19 characters at level 20 go, in the 100-byte record.
        let mut feature = Surnames::eqmac();
        let outcome = ask(&mut feature, &mut at_level(20), &"a".repeat(19));
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, 0xc441);
        assert_eq!(outcome.sent[0].body.len(), 100);
        assert_eq!(&outcome.sent[0].body[68..87], "a".repeat(19).as_bytes());
        // Titanium keeps its own bounds.
        let outcome = testing::run(|out| {
            Surnames::default().handle(
                &ClientCommand::Surname {
                    session_id: 5,
                    surname: "a".repeat(25),
                },
                &mut at_level(1),
                out,
            )
        });
        assert_eq!(outcome.sent.len(), 1);
    }

    #[test]
    fn an_answer_releases_the_wait_and_one_to_no_request_is_withheld() {
        let world = World::new(5);
        let answer = || {
            Message::Event(WorldEvent::SurnameAnswer {
                accepted: true,
                last_name: "Exemplum".into(),
            })
        };
        let mut feature = Surnames::default();
        let mut unasked = answer();
        feature.explain(&mut unasked, &world);
        assert!(matches!(unasked, Message::Withheld));

        feature.asked = Some(Instant::now());
        let mut heard = answer();
        feature.explain(&mut heard, &world);
        assert!(matches!(heard, Message::Event(_)));
        assert!(!feature.waiting());

        // A wait that lapsed holds nothing, and its late answer is withheld.
        feature.asked = Instant::now().checked_sub(SURNAME_WAIT);
        assert!(!feature.waiting());
        let mut late = answer();
        feature.explain(&mut late, &world);
        assert!(matches!(late, Message::Withheld));
    }

    #[test]
    fn a_late_assist_answer_releases_the_wait_without_selecting_a_target() {
        let mut feature = Socials::project1999();
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world
            .spawns
            .insert(testing::spawn(8, crate::world::SpawnKind::Player));
        feature.assist = Some(PendingAssist {
            assistee: 8,
            target_revision: world.target.state().revision,
            sent: Instant::now().checked_sub(ASSIST_SELECTION).unwrap(),
            superseded: false,
        });
        let command = ClientCommand::Assist {
            session_id: 5,
            spawn_id: 8,
        };
        let held = testing::run(|out| feature.handle(&command, &mut world, out));
        assert_eq!(held.sent, []);
        let mut answer =
            Message::Event(WorldEvent::Assisted(socials::Assisted { target: Some(7) }));
        feature.explain(&mut answer, &world);
        assert!(matches!(answer, Message::Withheld));
        let next = testing::run(|out| feature.handle(&command, &mut world, out));
        assert_eq!(next.sent, [socials::assist(8)]);
    }

    #[test]
    fn eqmac_socials_roll_assist_emote_and_yell_in_the_mac_packets() {
        use crate::world::Capability;
        let wire = &super::super::wire::EqMac;
        let mut feature = Socials::eqmac();
        assert_eq!(
            feature.capabilities(),
            [
                Capability::Rolling,
                Capability::Emoting,
                Capability::Assisting,
                Capability::Yelling
            ]
        );
        // An emote's animation stays Titanium's.
        assert!(!feature.owns(&ClientCommand::EmoteAnimation {
            session_id: 5,
            animation: 0x4d,
            created: Instant::now(),
        }));
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let roll = |low, high| ClientCommand::Random {
            session_id: 5,
            low,
            high,
        };
        let run = |feature: &mut Socials, command: &ClientCommand, world: &mut World| {
            let outcome = testing::run_on(wire, |out| feature.handle(command, world, out));
            assert!(outcome.result.is_ok());
            outcome
        };
        // Ends in order, at most 32000 (ours, 6.585b).
        assert_eq!(
            run(&mut feature, &roll(6, 1), &mut world).sent,
            [socials::eqmac_random(1, 6)]
        );
        assert_eq!(run(&mut feature, &roll(0, 32_001), &mut world).sent, []);
        let assist = |spawn_id| ClientCommand::Assist {
            session_id: 5,
            spawn_id,
        };
        assert_eq!(
            run(&mut feature, &assist(300), &mut world).sent,
            [socials::eqmac_assist(300)]
        );
        // One at a time until the answer.
        assert_eq!(run(&mut feature, &assist(301), &mut world).sent, []);
        // The server names the player's own spawn: it passes on as named.
        let mut answer =
            Message::Event(WorldEvent::Assisted(socials::Assisted { target: Some(7) }));
        feature.explain(&mut answer, &world);
        assert!(matches!(
            answer,
            Message::Event(WorldEvent::Assisted(socials::Assisted { target: Some(7) }))
        ));
        assert_eq!(
            run(&mut feature, &assist(301), &mut world).sent,
            [socials::eqmac_assist(301)]
        );
        // Or the wait runs out.
        feature.eqmac_assist = Instant::now().checked_sub(EQMAC_ASSIST_WAIT);
        assert_eq!(run(&mut feature, &assist(302), &mut world).sent.len(), 1);
        // Never oneself.
        feature.eqmac_assist = None;
        assert_eq!(run(&mut feature, &assist(7), &mut world).sent, []);
    }

    #[test]
    fn a_mac_emote_is_1026_bytes_and_heard_as_the_others_hear_it() {
        let wire = &super::super::wire::EqMac;
        let mut feature = Socials::eqmac();
        let mut world = World::new(5);
        let emote = ClientCommand::Emote {
            session_id: 5,
            text: "waves.".into(),
        };
        let outcome = testing::run_on(wire, |out| feature.handle(&emote, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        let packet = &outcome.sent[0];
        assert_eq!(packet.opcode, 0x1540);
        assert_eq!(packet.body.len(), 1026);
        assert_eq!(&packet.body[..9], b"\0\0 waves.");
        assert!(packet.body[9..].iter().all(|byte| *byte == 0));
        // The player's own line, as the Mac reader reads the others'.
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::Record(record) if matches!(
                &record.event,
                RecordEvent::Chat(chat)
                    if chat.message.as_ref().is_some_and(|line| line.text.ends_with(" waves."))
            )
        )));
        // A social emote leads with 0xffff and goes as built, unechoed.
        let social = ClientCommand::SocialEmote {
            session_id: 5,
            text: " nudges you.".into(),
        };
        let outcome = testing::run_on(wire, |out| feature.handle(&social, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(&outcome.sent[0].body[..14], b"\xff\xff nudges you.");
        assert_eq!(outcome.sent[0].body.len(), 1026);
        // Too long, or no words: nothing goes.
        for text in ["   ".to_owned(), "a".repeat(socials::EMOTE_LONGEST)] {
            let emote = ClientCommand::Emote {
                session_id: 5,
                text,
            };
            let outcome = testing::run_on(wire, |out| feature.handle(&emote, &mut world, out));
            assert_eq!(outcome.sent, []);
        }
        // A yell is empty on its own opcode.
        let yell = ClientCommand::YellForHelp {
            session_id: 5,
            created: Instant::now(),
        };
        let outcome = testing::run_on(wire, |out| feature.handle(&yell, &mut world, out));
        assert_eq!(outcome.sent, [socials::eqmac_yell()]);
        assert_eq!(
            (outcome.sent[0].opcode, outcome.sent[0].body.len()),
            (0xda41, 0)
        );
    }

    #[test]
    fn dice_emotes_and_assists_go_to_the_server() {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let run = |world: &mut World, command: ClientCommand| {
            testing::run(|out| Socials::default().handle(&command, world, out))
        };
        let rolled = run(
            &mut world,
            ClientCommand::Random {
                session_id: 5,
                low: 1,
                high: 6,
            },
        );
        assert_eq!(rolled.sent, [socials::random(1, 6)]);
        let emoted = run(
            &mut world,
            ClientCommand::Emote {
                session_id: 5,
                text: " waves. ".into(),
            },
        );
        assert_eq!(emoted.sent, [socials::emote("waves.").unwrap()]);
        // The player hears it as the others do.
        assert!(emoted
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::Record(_))));
        let refused = run(
            &mut world,
            ClientCommand::Emote {
                session_id: 5,
                text: "  ".into(),
            },
        );
        assert_eq!(refused.sent, []);
        // A social emote goes as it was built, trailing space too, and
        // nothing is echoed: the client prints its own line.
        let social = run(
            &mut world,
            ClientCommand::SocialEmote {
                session_id: 5,
                text: " nudges ".into(),
            },
        );
        assert_eq!(social.sent, [socials::social_emote(" nudges ").unwrap()]);
        assert!(!social
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::Record(_))));
        let silent = run(
            &mut world,
            ClientCommand::SocialEmote {
                session_id: 5,
                text: " ".into(),
            },
        );
        assert_eq!(silent.sent, []);
        let assisted = run(
            &mut world,
            ClientCommand::Assist {
                session_id: 5,
                spawn_id: 300,
            },
        );
        assert_eq!(assisted.sent, [socials::assist(300)]);
        // Not oneself.
        let own = run(
            &mut world,
            ClientCommand::Assist {
                session_id: 5,
                spawn_id: 7,
            },
        );
        assert_eq!(own.sent, []);
        assert!(own.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::SocialRefused {
                string_id: Some(ASSIST_SELF),
                ..
            })
        )));
    }

    #[test]
    fn an_emote_animation_goes_out_only_for_a_spawned_player_not_known_to_sit() {
        let animate = ClientCommand::EmoteAnimation {
            session_id: 5,
            animation: 0x4d,
            created: Instant::now(),
        };
        let mut feature = Socials::default();
        assert!(feature.owns(&animate));
        let mut world = World::new(5);
        let report = testing::run(|out| feature.handle(&animate, &mut world, out));
        report.result.unwrap();
        assert_eq!(report.sent, []);
        world.own_spawn = Some(7);
        world.posture.observed(PostureState::Sitting);
        let report = testing::run(|out| feature.handle(&animate, &mut world, out));
        report.result.unwrap();
        assert_eq!(report.sent, []);
        world.posture.observed(PostureState::Standing);
        let report = testing::run(|out| feature.handle(&animate, &mut world, out));
        report.result.unwrap();
        assert_eq!(
            report.sent,
            [socials::titanium_emote_animation(7, 0x4d).unwrap()]
        );
    }

    #[test]
    fn a_yell_sends_the_empty_yell_for_help_with_no_target_or_posture_test() {
        let yell = ClientCommand::YellForHelp {
            session_id: 5,
            created: Instant::now(),
        };
        for mut feature in [Socials::default(), Socials::project1999()] {
            // No target, no own spawn, sitting or not: the handler tests none.
            let mut world = World::new(5);
            let yelled = testing::run(|out| feature.handle(&yell, &mut world, out));
            assert_eq!(yelled.sent, [socials::titanium_yell()]);
            assert!(feature.owns(&yell));
            assert!(feature
                .capabilities()
                .contains(&crate::world::Capability::Yelling));
        }
        let packet = socials::titanium_yell();
        assert_eq!((packet.opcode, packet.body.len()), (0x61ef, 0));
    }

    #[test]
    fn a_project1999_social_emote_drops_one_leading_space_and_echoes_nothing() {
        let mut feature = Socials::project1999();
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let social = |text: &str| ClientCommand::SocialEmote {
            session_id: 5,
            text: text.into(),
        };
        let nudged = testing::run(|out| feature.handle(&social(" nudges "), &mut world, out));
        assert_eq!(
            nudged.sent,
            [socials::project1999_emote("nudges ").unwrap()]
        );
        assert!(!nudged
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::Record(_))));
        // Past the P99 sender's 510-byte bound, under the general one.
        let long = format!(" {}", "a".repeat(600));
        let refused = testing::run(|out| feature.handle(&social(&long), &mut world, out));
        assert_eq!(refused.sent, []);
    }
}
