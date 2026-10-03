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
use eq_network_game::{request::Request, socials};

/// The official client's string for assisting oneself (`eqstr_us.txt`).
const ASSIST_SELF: u32 = 1403;

/// Rolls, emotes and assists.
pub(super) struct Socials;

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

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match command {
            ClientCommand::Random { low, high, .. } => out.request(&Request::Random {
                low: *low,
                high: *high,
            }),
            ClientCommand::Emote { text, .. } => {
                let text = text.trim();
                if text.is_empty() {
                    return actions::refuse(command, "Say what you do.", out.log);
                }
                if text.len() >= socials::EMOTE_LONGEST {
                    return actions::refuse(command, "That emote is too long.", out.log);
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
                out.request(&Request::Assist(*spawn_id))
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, ClientEvent};
    use super::*;
    use eq_network_game::world::WorldEvent;

    #[test]
    fn dice_emotes_and_assists_go_to_the_server() {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let run = |world: &mut World, command: ClientCommand| {
            testing::run(|out| Socials.handle(&command, world, out))
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
