//! World-admission choices are validated against one immutable server list.
use super::{ClientCommand, ClientEvent, Events};
use anyhow::{Context, Result};
use eq_network_game::{characters::CharacterChoice, world::WorldEvent};
use std::sync::mpsc::Receiver;

/// What the player chose on the selection screen.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Choice {
    /// Enter the world as this listed character.
    Enter(String),
    /// Create a new character first.
    Create(eq_network_game::creation::NewCharacter),
}

impl Choice {
    /// The character to enter, ignoring creation requests.
    pub(super) fn entered(self) -> Option<String> {
        match self {
            Self::Enter(name) => Some(name),
            Self::Create(_) => None,
        }
    }
}

pub(super) struct Selection {
    id: u64,
    characters: Vec<CharacterChoice>,
}

impl Selection {
    /// Publishes a new list; configured names retain automatic-entry behavior.
    pub fn new(
        characters: Vec<CharacterChoice>,
        configured: &str,
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
            id: rand::random(),
            characters,
        };
        log.send(ClientEvent::World(WorldEvent::CharacterSelection {
            selection_id: selection.id,
            characters: selection.characters.clone(),
        }))?;
        Ok((selection, selected))
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
            ClientCommand::SelectCharacter { selection_id, slot } if selection_id == self.id => {
                self.characters
                    .iter()
                    .find(|entry| entry.slot == slot)
                    .map(|entry| Choice::Enter(entry.name.clone()))
            }
            ClientCommand::CreateCharacter {
                selection_id,
                character,
            } if selection_id == self.id => Some(Choice::Create(character)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_current_occupied_slots_can_enter_world() {
        let selection = Selection {
            id: 9,
            characters: vec![CharacterChoice {
                slot: 3,
                name: "Example".into(),
                level: None,
                zone_id: None,
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
