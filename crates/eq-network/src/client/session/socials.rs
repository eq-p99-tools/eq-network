//! Rolling dice, emoting and assisting. The server rolls a die for every
//! player within 400 units, the one who asked among them, and answers an
//! assist with the target to take. It passes an emote on to everyone near
//! but the one who made it (`EQEmu`'s `Client::Handle_OP_Emote`), so the
//! session records the player's own emote as the others hear it, as the
//! official client shows it (inferred). The official client refuses to
//! assist the player themself (inferred from its string for it); `EQEmu`
//! answers with the player's own target.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand,
};
use crate::client::RecordEvent;
use anyhow::Result;
use eq_network_game::{message::Message, request::Request, socials, world::WorldEvent};
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

/// Rolls, emotes and assists.
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
        ]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::Random { .. }
                | ClientCommand::Emote { .. }
                | ClientCommand::Assist { .. }
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

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, ClientEvent};
    use super::*;

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
}
