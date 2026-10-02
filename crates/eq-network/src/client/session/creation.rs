//! Creating a character at the world server. Every client generation takes
//! the same steps: the world approves the name, then creates the character
//! and sends the new character list. A server type says whether it creates
//! characters, and its client generation builds the packets.
use super::{ClientEvent, Events};
use anyhow::{Context, Result};
use eq_network_game::{
    command::EncodedCommand,
    creation::{self, NewCharacter},
    world::WorldEvent,
};
use eq_network_transport::Transport;

/// How a client generation asks the world for a new character.
pub(super) trait Creation: Sync {
    /// Asks the world to approve the character's name.
    ///
    /// # Errors
    /// Refuses a character the server would refuse.
    fn approval(&self, character: &NewCharacter) -> Result<EncodedCommand>;

    /// Asks the world to create the character once it approved the name.
    ///
    /// # Errors
    /// Refuses a character the server would refuse.
    fn request(&self, character: &NewCharacter) -> Result<EncodedCommand>;
}

/// Titanium's creation packets, which P99 and `EQEmu` take.
pub(super) struct Titanium;

impl Creation for Titanium {
    fn approval(&self, character: &NewCharacter) -> Result<EncodedCommand> {
        creation::titanium_approval(character)
    }

    fn request(&self, character: &NewCharacter) -> Result<EncodedCommand> {
        creation::titanium_request(character)
    }
}

/// `EQMac`'s creation packets, which TAKP takes.
pub(super) struct EqMac;

impl Creation for EqMac {
    fn approval(&self, character: &NewCharacter) -> Result<EncodedCommand> {
        creation::eqmac_approval(character)
    }

    fn request(&self, character: &NewCharacter) -> Result<EncodedCommand> {
        creation::eqmac_request(character)
    }
}

/// A character the player asked for, as far as the world has taken it.
pub(super) struct Creating {
    creation: &'static dyn Creation,
    character: NewCharacter,
    /// Whether the world approved the name and the creation request went out.
    requested: bool,
}

impl Creating {
    /// Asks the world to approve the name of the character the player chose;
    /// refuses the character at once when the server type creates none, or
    /// the world would refuse it.
    ///
    /// # Errors
    /// Returns an error when the connection or the host's event handler fails.
    pub(super) fn start(
        creation: Option<&'static dyn Creation>,
        character: NewCharacter,
        session: &mut dyn Transport,
        log: &mut Events<'_>,
    ) -> Result<Option<Self>> {
        let approval = creation
            .context("this server does not create characters yet")
            .and_then(|creation| Ok((creation, creation.approval(&character)?)));
        match approval {
            Ok((creation, packet)) => {
                session.send(packet.opcode, &packet.body)?;
                Ok(Some(Self {
                    creation,
                    character,
                    requested: false,
                }))
            }
            Err(error) => {
                log.diagnostic(format!("Rejected character creation: {error}"))?;
                told(character.name, false, log)?;
                Ok(None)
            }
        }
    }

    /// The world's one-byte answer: an approved name sends the creation
    /// request, and a refused name or creation ends the attempt.
    ///
    /// # Errors
    /// Returns an error when the creation request cannot be built, or the
    /// connection or the host's event handler fails.
    pub(super) fn answered(
        mut self,
        answer: &[u8],
        session: &mut dyn Transport,
        log: &mut Events<'_>,
    ) -> Result<Option<Self>> {
        if answer.first() == Some(&1) && !self.requested {
            let packet = self.creation.request(&self.character)?;
            session.send(packet.opcode, &packet.body)?;
            self.requested = true;
            return Ok(Some(self));
        }
        told(self.character.name, false, log)?;
        Ok(None)
    }

    /// Whether the creation request went out, so the next character list
    /// is the one with the new character.
    pub(super) fn requested(&self) -> bool {
        self.requested
    }

    /// Tells the host the world created the character.
    ///
    /// # Errors
    /// Returns an error when the host's event handler fails.
    pub(super) fn created(self, log: &mut Events<'_>) -> Result<()> {
        told(self.character.name, true, log)
    }
}

/// Tells the host how the world took the character.
fn told(name: String, accepted: bool, log: &mut Events<'_>) -> Result<()> {
    log.send(ClientEvent::World(WorldEvent::CharacterCreation {
        name,
        accepted,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ClientConfig;
    use eq_network_game::creation::{
        APPROVE_NAME_OPCODE, CREATE_OPCODE, EQMAC_APPROVE_NAME_OPCODE, EQMAC_CREATE_OPCODE,
    };

    /// A connection that keeps what the client sent.
    #[derive(Default)]
    struct Kept(Vec<(u16, Vec<u8>)>);

    impl Transport for Kept {
        fn send(&mut self, opcode: u16, body: &[u8]) -> Result<()> {
            self.0.push((opcode, body.to_vec()));
            Ok(())
        }

        fn receive(&mut self) -> Result<Option<crate::transport::Application>> {
            Ok(None)
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }

        fn last_received_seconds(&self) -> u64 {
            0
        }
    }

    /// A human cleric of Rodcet Nife starting in North Qeynos.
    fn cleric() -> NewCharacter {
        NewCharacter::with_points_in("Testcleric", (1, 2, 0), (212, 2), 4).unwrap()
    }

    /// What the host heard about the creation.
    fn outcomes(events: &[ClientEvent]) -> Vec<(String, bool)> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::CharacterCreation { name, accepted }) => {
                    Some((name.clone(), *accepted))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_approved_name_sends_the_creation_and_the_new_list_reports_it() {
        let config = ClientConfig::new("ACCOUNT", "PASSWORD", "Test Server", "");
        let mut events = Vec::new();
        let mut handler = |event| {
            events.push(event);
            Ok(())
        };
        let mut log = Events::new(&config, &mut handler);
        let mut kept = Kept::default();
        let attempt = Creating::start(Some(&EqMac), cleric(), &mut kept, &mut log)
            .unwrap()
            .expect("the name goes to the world");
        assert!(!attempt.requested());
        let attempt = attempt
            .answered(&[1], &mut kept, &mut log)
            .unwrap()
            .expect("an approved name sends the creation");
        assert!(attempt.requested());
        attempt.created(&mut log).unwrap();
        drop(log);
        let sent: Vec<(u16, usize)> = kept.0.iter().map(|(op, body)| (*op, body.len())).collect();
        assert_eq!(
            sent,
            [(EQMAC_APPROVE_NAME_OPCODE, 78), (EQMAC_CREATE_OPCODE, 8452)]
        );
        assert_eq!(outcomes(&events), [("Testcleric".into(), true)]);
    }

    #[test]
    fn a_refusal_or_a_server_without_creation_ends_the_attempt() {
        let config = ClientConfig::new("ACCOUNT", "PASSWORD", "Test Server", "");
        let mut events = Vec::new();
        let mut handler = |event| {
            events.push(event);
            Ok(())
        };
        let mut log = Events::new(&config, &mut handler);
        let mut kept = Kept::default();
        // A server type that creates no characters sends nothing.
        assert!(Creating::start(None, cleric(), &mut kept, &mut log)
            .unwrap()
            .is_none());
        assert_eq!(kept.0.len(), 0, "{:?}", kept.0);
        // A refused name ends the attempt.
        let attempt = Creating::start(Some(&Titanium), cleric(), &mut kept, &mut log)
            .unwrap()
            .unwrap();
        assert!(attempt
            .answered(&[0], &mut kept, &mut log)
            .unwrap()
            .is_none());
        // So does a refused creation, which the world reports on the same
        // opcode after the name was approved.
        let attempt = Creating::start(Some(&Titanium), cleric(), &mut kept, &mut log)
            .unwrap()
            .unwrap()
            .answered(&[1], &mut kept, &mut log)
            .unwrap()
            .unwrap();
        assert!(attempt
            .answered(&[0], &mut kept, &mut log)
            .unwrap()
            .is_none());
        drop(log);
        let sent: Vec<u16> = kept.0.iter().map(|(opcode, _)| *opcode).collect();
        assert_eq!(
            sent,
            [APPROVE_NAME_OPCODE, APPROVE_NAME_OPCODE, CREATE_OPCODE]
        );
        assert_eq!(
            outcomes(&events),
            [
                ("Testcleric".into(), false),
                ("Testcleric".into(), false),
                ("Testcleric".into(), false)
            ]
        );
    }
}
