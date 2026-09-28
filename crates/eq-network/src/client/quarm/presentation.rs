//! Admission staging for Quarm's renderer-independent world events.

use anyhow::{ensure, Result};
use eq_network_game::{
    quarm,
    world::{PlayerState, PostureState, SpawnState, WorldEvent},
};
use std::collections::BTreeMap;

/// Holds only decoded presentation fields while admission packets arrive.
#[derive(Default)]
pub(super) struct Presentation {
    player: Option<PlayerState>,
    own_spawn: Option<quarm::OwnSpawn>,
    assigned_id: Option<u16>,
    spawns: BTreeMap<u16, SpawnState>,
    postures: BTreeMap<u16, PostureState>,
    pending: Vec<WorldEvent>,
    entered: bool,
}

impl Presentation {
    /// Stages admission state and returns ongoing events once admission is published.
    pub(super) fn receive(
        &mut self,
        opcode: u16,
        body: &[u8],
        character: &str,
    ) -> Result<Vec<WorldEvent>> {
        match opcode {
            0x3640 if !self.entered => self.player = Some(quarm::profile(body, character)?),
            0x2840 if !self.entered => self.own_spawn = Some(quarm::own_spawn(body, character)?),
            0xf540 => {
                if let Some(id) = quarm::assigned_id(body)? {
                    self.assigned_id = Some(id);
                }
            }
            _ => (),
        }
        let events = quarm::updates(opcode, body)?;
        if self.entered {
            return Ok(events);
        }
        for event in events {
            match event {
                WorldEvent::Spawns(spawns) => {
                    for spawn in spawns {
                        self.postures.remove(&spawn.spawn_id);
                        self.spawns.insert(spawn.spawn_id, spawn);
                    }
                    ensure!(self.spawns.len() <= 4096, "too many initial Quarm entities");
                }
                WorldEvent::Visibility {
                    spawn_id,
                    invisible,
                } => {
                    if let Some(spawn) = self.spawns.get_mut(&spawn_id) {
                        spawn.invisible = invisible;
                    }
                }
                WorldEvent::Despawn(id) => {
                    self.spawns.remove(&id);
                    self.postures.remove(&id);
                }
                WorldEvent::Posture { spawn_id, posture } => {
                    self.postures.insert(spawn_id, posture);
                    ensure!(
                        self.postures.len() <= 4096,
                        "too many initial Quarm postures"
                    );
                }
                WorldEvent::Position { spawn_id, position } => {
                    if let Some(spawn) = self.spawns.get_mut(&spawn_id) {
                        spawn.position = position;
                    }
                    if self.assigned_id == Some(spawn_id) {
                        if let Some(spawn) = self.own_spawn.as_mut() {
                            spawn.position = position;
                        }
                    }
                }
                event => {
                    ensure!(
                        self.pending.len() < 512,
                        "too many initial Quarm state updates"
                    );
                    self.pending.push(event);
                }
            }
        }
        Ok(Vec::new())
    }

    /// Publishes a complete player before the initial entity batch, exactly once.
    pub(super) fn enter(&mut self, session_id: u64, zone: &str) -> Vec<WorldEvent> {
        if self.entered {
            return Vec::new();
        }
        let (Some(player), Some(spawn), Some(id)) =
            (&self.player, &self.own_spawn, self.assigned_id)
        else {
            return Vec::new();
        };
        let mut player = player.clone();
        player.spawn_id = id;
        player.position = spawn.position;
        player.size = spawn.size;
        player.walk_speed = spawn.walk_speed;
        player.run_speed = spawn.run_speed;
        self.entered = true;
        let mut events = vec![
            WorldEvent::Entered {
                session_id,
                zone: zone.into(),
                player: Box::new(player),
            },
            WorldEvent::Spawns(std::mem::take(&mut self.spawns).into_values().collect()),
        ];
        events.extend(
            std::mem::take(&mut self.postures)
                .into_iter()
                .map(|(spawn_id, posture)| WorldEvent::Posture { spawn_id, posture }),
        );
        events.append(&mut self.pending);
        self.player = None;
        self.own_spawn = None;
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eq_network_game::world::{Position, SpawnKind};

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the ordered integration scenario and its assertions together"
    )]
    fn admission_waits_for_all_fields_then_flushes_current_entities_once() {
        let mut state = Presentation {
            player: Some(PlayerState {
                base_attributes: None,
                deity: None,
                class: None,
                spawn_id: 0,
                race: 128,
                gender: 0,
                level: 1,
                position: Position::default(),
                mana: 10,
                endurance: None,
                skills: None,
                spell_refresh_ms: None,
                memorized_spells: [None; 8],
                size: 0.0,
                walk_speed: 0.0,
                run_speed: 0.0,
                hp_percent: None,
            }),
            ..Presentation::default()
        };
        assert!(state.enter(5, "example").is_empty());
        state
            .receive(0xf540, &[0, 0, 16, 0, 7, 0, 0, 0], "Example")
            .unwrap();
        assert!(state.enter(5, "example").is_empty());
        let position = Position {
            x: 12.0,
            y: -42.0,
            z: 3.0,
            heading: 128.0,
        };
        state.own_spawn = Some(quarm::OwnSpawn {
            position,
            size: 6.0,
            walk_speed: 0.3,
            run_speed: 0.7,
        });
        for id in [8, 9] {
            state.spawns.insert(
                id,
                SpawnState {
                    class: None,
                    spawn_id: id,
                    name: "ExampleNpc".into(),
                    kind: SpawnKind::Npc,
                    race: 54,
                    gender: 2,
                    position: Position::default(),
                    size: 6.0,
                    invisible: false,
                },
            );
        }
        for id in [7, 8, 9] {
            state
                .receive(0xf540, &[id, 0, 14, 0, 110, 0, 0, 0], "Example")
                .unwrap();
        }
        state
            .receive(0xf540, &[8, 0, 14, 0, 111, 0, 0, 0], "Example")
            .unwrap();
        state.receive(0x2940, &[9, 0], "Example").unwrap();
        state
            .receive(0x9941, &165u32.to_le_bytes(), "Example")
            .unwrap();
        let events = state.enter(5, "example");
        let WorldEvent::Entered {
            player,
            session_id,
            zone,
        } = &events[0]
        else {
            panic!("missing admission")
        };
        assert_eq!(
            (*session_id, zone.as_str(), player.spawn_id),
            (5, "example", 7)
        );
        assert_eq!(player.position, position);
        let WorldEvent::Spawns(spawns) = &events[1] else {
            panic!("missing initial entities")
        };
        assert_eq!(spawns.len(), 1);
        assert_eq!(spawns[0].spawn_id, 8);
        assert_eq!(events.len(), 5);
        assert_eq!(
            events[2],
            WorldEvent::Posture {
                spawn_id: 7,
                posture: PostureState::Sitting,
            }
        );
        assert_eq!(
            events[3],
            WorldEvent::Posture {
                spawn_id: 8,
                posture: PostureState::Ducking,
            }
        );
        assert_eq!(events[4], WorldEvent::Experience(165));
        assert!(state.postures.is_empty());
        assert!(state.enter(5, "example").is_empty());
        assert_eq!(
            state
                .receive(0xf540, &[7, 0, 14, 0, 100, 0, 0, 0], "Example")
                .unwrap(),
            vec![WorldEvent::Posture {
                spawn_id: 7,
                posture: PostureState::Standing
            }]
        );
        assert_eq!(
            state.receive(0x2940, &[8, 0], "Example").unwrap(),
            vec![WorldEvent::Despawn(8)]
        );
    }
}
