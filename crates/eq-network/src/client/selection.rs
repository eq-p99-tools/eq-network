//! The player's choices before a zone: the world from the login server's
//! list, then the character from the world's. Each choice is validated
//! against the one list it was made from.
use super::{ClientCommand, ClientEvent, Events};
use anyhow::{ensure, Context, Result};
use eq_network_game::{
    characters::CharacterChoice,
    servers::{ServerChoice, ServerRefusal},
    world::WorldEvent,
};
use std::sync::mpsc::Receiver;

/// The configured world's place in the login server's list. It must be
/// listed, and take players.
///
/// # Errors
/// Returns an error when the world is not listed, or is down or locked.
pub(super) fn configured_world(servers: &[ServerChoice], name: &str) -> Result<usize> {
    let index = servers
        .iter()
        .position(|server| server.name.eq_ignore_ascii_case(name))
        .context("configured server name was not found in the server list")?;
    ensure!(
        servers[index].status.open(),
        "configured server is unavailable or locked"
    );
    Ok(index)
}

/// The login server's list, published for the player to choose a world
/// from when none is configured.
pub(super) struct Worlds {
    id: u64,
    servers: Vec<ServerChoice>,
}

impl Worlds {
    /// Publishes the list.
    ///
    /// # Errors
    /// Returns an error when the host's event handler fails.
    pub fn publish(servers: Vec<ServerChoice>, log: &mut Events<'_>) -> Result<Self> {
        let worlds = Self {
            id: rand::random(),
            servers,
        };
        log.send(ClientEvent::World(WorldEvent::ServerSelection {
            selection_id: worlds.id,
            servers: worlds.servers.clone(),
        }))?;
        Ok(worlds)
    }

    /// The player's choice, from the host's queue: a world of this list
    /// that takes players. Stale choices and every other command are
    /// dropped without a packet, as the character list drops them.
    pub fn poll(&self, commands: Option<&Receiver<ClientCommand>>) -> Option<usize> {
        commands?
            .try_iter()
            .take(64)
            .find_map(|command| self.choose(&command))
    }

    fn choose(&self, command: &ClientCommand) -> Option<usize> {
        match command {
            ClientCommand::SelectServer {
                selection_id,
                index,
            } if *selection_id == self.id => self
                .servers
                .get(*index)
                .filter(|server| server.status.open())
                .map(|_| *index),
            _ => None,
        }
    }

    /// The name of the world at this place in the list.
    pub fn name(&self, index: usize) -> &str {
        &self.servers[index].name
    }

    /// Tells the player the login server refused their choice; the list
    /// stays up for another.
    ///
    /// # Errors
    /// Returns an error when the host's event handler fails.
    pub fn refused(&self, refusal: ServerRefusal, log: &mut Events<'_>) -> Result<()> {
        log.send(ClientEvent::World(WorldEvent::ServerRefused {
            selection_id: self.id,
            refusal,
        }))
    }
}

/// What the player chose on the selection screen.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Choice {
    /// Enter the world as this listed character.
    Enter(String),
    /// Create a new character first.
    Create(eq_network_game::creation::NewCharacter),
    /// List-scoped name request, carrying frontend identity, race and gender.
    GenerateName(u64, u64, u32, u32),
    /// An explicitly confirmed identity, validated by the deletion owner.
    Delete(eq_network_game::deletion::Request),
}

#[cfg(test)]
impl Choice {
    /// The character to enter, ignoring creation requests.
    fn entered(self) -> Option<String> {
        match self {
            Self::Enter(name) => Some(name),
            Self::Create(_) | Self::GenerateName(..) | Self::Delete(_) => None,
        }
    }
}

pub(super) struct Selection {
    id: u64,
    connection_id: u64,
    deletion: Option<eq_network_game::deletion::Rules>,
    characters: Vec<CharacterChoice>,
}

impl Selection {
    /// Publishes a new list; configured names retain automatic-entry behavior.
    pub fn new(
        characters: Vec<CharacterChoice>,
        configured: &str,
        creation: Option<eq_network_game::creation::Rules>,
        connection_id: u64,
        deletion: Option<eq_network_game::deletion::Rules>,
        log: &mut Events<'_>,
    ) -> Result<(Self, Option<String>)> {
        let selected = if configured.is_empty() {
            None
        } else {
            Some(
                characters
                    .iter()
                    .find(|entry| entry.name.eq_ignore_ascii_case(configured))
                    .context("configured character is absent from character selection")?
                    .name
                    .clone(),
            )
        };
        let selection = Self {
            id: next_identity()?,
            connection_id,
            deletion,
            characters,
        };
        log.send(ClientEvent::World(WorldEvent::CharacterSelection {
            selection_id: selection.id,
            connection_id,
            characters: selection.characters.clone(),
            creation,
            deletion,
        }))?;
        Ok((selection, selected))
    }

    pub fn scope(&self) -> eq_network_game::deletion::Scope {
        eq_network_game::deletion::Scope {
            connection_id: self.connection_id,
            selection_id: self.id,
        }
    }

    pub fn deletion(&self) -> Option<eq_network_game::deletion::Rules> {
        self.deletion
    }

    pub fn contains(&self, name: &str) -> bool {
        self.characters
            .iter()
            .filter(|entry| entry.name == name)
            .count()
            == 1
    }

    /// Ignores stale choices and all zone actions without emitting packets.
    pub fn poll(&self, commands: Option<&Receiver<ClientCommand>>) -> Option<Choice> {
        commands?
            .try_iter()
            .take(64)
            .find_map(|command| self.choose(command))
    }

    fn choose(&self, command: ClientCommand) -> Option<Choice> {
        match command {
            // Even stale requests receive a typed local refusal from the owner.
            ClientCommand::DeleteCharacter(request) => Some(Choice::Delete(request)),
            ClientCommand::SelectCharacter { selection_id, slot } if selection_id == self.id => {
                self.characters
                    .iter()
                    .find(|entry| entry.slot == slot)
                    .map(|entry| Choice::Enter(entry.name.clone()))
            }
            ClientCommand::GenerateName {
                selection_id,
                request_id,
                race,
                gender,
            } if selection_id == self.id => {
                Some(Choice::GenerateName(selection_id, request_id, race, gender))
            }
            ClientCommand::CreateCharacter {
                selection_id,
                character,
            } if selection_id == self.id => Some(Choice::Create(character)),
            _ => None,
        }
    }
}

/// Never reuse a list identity within this process, including on reconnect.
fn next_identity() -> Result<u64> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        value.checked_add(1)
    })
    .map_err(|_| anyhow::anyhow!("character-list identity exhausted"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eq_network_game::servers::ServerStatus;

    fn world(name: &str, status: ServerStatus) -> ServerChoice {
        ServerChoice {
            name: name.into(),
            status,
            players: None,
            preferred: false,
        }
    }

    #[test]
    fn a_configured_world_must_be_listed_and_up() {
        let servers = [
            world("Example Down", ServerStatus::Down),
            world("Example Locked", ServerStatus::Locked),
            world("Example Up", ServerStatus::Up),
        ];
        assert_eq!(configured_world(&servers, "example up").unwrap(), 2);
        for name in ["Example Down", "Example Locked"] {
            let error = configured_world(&servers, name).unwrap_err().to_string();
            assert!(error.contains("unavailable or locked"), "{error}");
        }
        let missing = configured_world(&servers, "Elsewhere").unwrap_err();
        assert!(missing.to_string().contains("not found"));
    }

    #[test]
    fn only_a_current_open_world_can_be_chosen() {
        let worlds = Worlds {
            id: 9,
            servers: vec![
                world("Example Up", ServerStatus::Up),
                world("Example Down", ServerStatus::Down),
            ],
        };
        let choice = |selection_id, index| ClientCommand::SelectServer {
            selection_id,
            index,
        };
        assert_eq!(worlds.choose(&choice(9, 0)), Some(0));
        // A stale list, a world that is down, and a place past the list.
        assert_eq!(worlds.choose(&choice(8, 0)), None);
        assert_eq!(worlds.choose(&choice(9, 1)), None);
        assert_eq!(worlds.choose(&choice(9, 2)), None);
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(choice(8, 0)).unwrap();
        sender
            .send(ClientCommand::SelectCharacter {
                selection_id: 9,
                slot: 0,
            })
            .unwrap();
        sender.send(choice(9, 0)).unwrap();
        assert_eq!(worlds.poll(Some(&receiver)), Some(0));
        assert_eq!(worlds.poll(Some(&receiver)), None);
        assert_eq!(worlds.poll(None), None);
        assert_eq!(worlds.name(0), "Example Up");
    }

    #[test]
    fn only_current_occupied_slots_can_enter_world() {
        let selection = Selection {
            id: 9,
            connection_id: 1,
            deletion: None,
            characters: vec![CharacterChoice {
                slot: 3,
                name: "Example".into(),
                level: None,
                zone_id: None,
                look: None,
            }],
        };
        for (id, slot, expected) in [(9, 3, Some("Example")), (8, 3, None), (9, 2, None)] {
            assert_eq!(
                selection
                    .choose(ClientCommand::SelectCharacter {
                        selection_id: id,
                        slot
                    })
                    .and_then(Choice::entered)
                    .as_deref(),
                expected
            );
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        sender
            .send(ClientCommand::SelectCharacter {
                selection_id: 8,
                slot: 3,
            })
            .unwrap();
        sender
            .send(ClientCommand::SelectCharacter {
                selection_id: 9,
                slot: 3,
            })
            .unwrap();
        assert_eq!(
            selection
                .poll(Some(&receiver))
                .and_then(Choice::entered)
                .as_deref(),
            Some("Example")
        );
        assert!(selection.poll(Some(&receiver)).is_none());
    }
}
