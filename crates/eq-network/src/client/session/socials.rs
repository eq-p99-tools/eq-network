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

/// Rolls, emotes, assists and yells.
#[derive(Default)]
pub(super) struct Socials {
    project1999: bool,
    assist: Option<PendingAssist>,
}

impl Socials {
    pub(super) fn project1999() -> Self {
        Self {
            project1999: true,
            ..Self::default()
        }
    }
}

impl Feature for Socials {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        vec![
            Capability::Rolling,
            Capability::Emoting,
            Capability::Assisting,
            Capability::Yelling,
        ]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
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
                if self.project1999 {
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
            ClientCommand::Emote { text, .. } => {
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
                // The player hears their emote as the others do.
                let body = socials::heard_emote(out.sender.name, text);
                if let Some(event) = out.wire.chat(socials::EMOTE_OPCODE, &body, false)? {
                    let zone = out.log.zone.clone();
                    out.log.record(&zone, RecordEvent::Chat(event))?;
                }
                Ok(())
            }
            ClientCommand::SocialEmote { text, .. } => self.social_emote(command, text, out),
            ClientCommand::Assist { spawn_id, .. } => {
                if world.own_spawn == Some(*spawn_id) {
                    return actions::refuse_officially(
                        command,
                        ("You cannot take your own target.", Some(ASSIST_SELF)),
                        out.log,
                    );
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
/// Only stock `EQEmu` lists it, from source, not yet checked live
/// (checklist 6.507).
#[derive(Default)]
pub(super) struct Surnames {
    asked: Option<Instant>,
}

impl Surnames {
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
        _world: &mut World,
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
        if surname.len() >= eq_network_game::names::SURNAME_FIELD {
            return actions::refuse(command, "That surname is too long.", out.log);
        }
        out.request(&Request::Surname(surname.to_owned()))?;
        self.asked = Some(Instant::now());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, ClientEvent};
    use super::*;

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
