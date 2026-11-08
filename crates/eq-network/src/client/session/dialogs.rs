//! Admission-scoped Titanium dialogs and player inspection.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    inspection::{self, Inspection, PlayerInspection},
    message::Message,
    popups::{self, PopupOffer},
    world::{Capability, Position, SpawnKind, WorldEvent},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct Dialogs {
    next_serial: u64,
    popup: Option<PopupOffer>,
    pending: BTreeMap<u16, PendingInspection>,
    /// An unanswered request to a departed incarnation cannot be disambiguated
    /// from a new answer under the same wire spawn ID.
    retired: BTreeSet<u16>,
    latest: Option<u16>,
    note: Option<String>,
    last_reply: Option<Instant>,
}

struct PendingInspection {
    name: String,
    outstanding: u64,
}

fn in_range(own: Position, target: Position, inclusive: bool) -> bool {
    let distance_squared =
        (own.x - target.x).powi(2) + (own.y - target.y).powi(2) + (own.z - target.z).powi(2);
    let limit = inspection::REQUEST_RANGE.powi(2);
    if inclusive {
        distance_squared <= limit
    } else {
        distance_squared < limit
    }
}

fn event(out: &mut Out<'_, '_>, event: WorldEvent) -> Result<()> {
    out.log.send(ClientEvent::World(event))
}

impl Dialogs {
    fn close_popup(&mut self, out: &mut Out<'_, '_>) -> Result<()> {
        if let Some(offer) = self.popup.take() {
            event(
                out,
                WorldEvent::PopupClosed {
                    session_id: offer.session_id,
                    serial: offer.serial,
                },
            )?;
        }
        Ok(())
    }

    fn inspect(
        &mut self,
        command: &ClientCommand,
        target: eq_network_game::targets::TargetState,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let Some((own, position)) = world.player_at() else {
            return actions::refuse(command, "The player's spawn is unknown.", out.log);
        };
        let Some(id) = target
            .spawn_id
            .filter(|_| target.available && target == world.target.state())
        else {
            return actions::refuse(command, "The selected player has changed.", out.log);
        };
        let name = if id == own {
            out.sender.name
        } else {
            let Some(spawn) = world
                .visible_spawn(id)
                .filter(|spawn| spawn.kind == SpawnKind::Player)
            else {
                return actions::refuse(command, "Select a visible player to inspect.", out.log);
            };
            if !in_range(position, spawn.position, false) {
                return actions::refuse(
                    command,
                    "That player is too far away to inspect.",
                    out.log,
                );
            }
            &spawn.name
        };
        if self.retired.contains(&id) {
            return actions::refuse(
                command,
                "An earlier inspection used that player's recycled spawn ID.",
                out.log,
            );
        }
        out.send(&inspection::titanium_request(id, own)?)?;
        self.latest = Some(id);
        let pending = self.pending.entry(id).or_insert_with(|| PendingInspection {
            name: name.to_owned(),
            outstanding: 0,
        });
        pending.outstanding += 1;
        Ok(())
    }

    fn answer(&mut self, report: &Inspection, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        let Some((own, position)) = world.player_at() else {
            return Ok(());
        };
        let id = report.player_id;
        if report.recipient_id != own {
            return Ok(());
        }
        let Some(pending) = self.pending.get_mut(&id) else {
            return Ok(());
        };
        let name = pending.name.clone();
        pending.outstanding -= 1;
        let settled = pending.outstanding == 0;
        if settled {
            self.pending.remove(&id);
        }
        if self.latest != Some(id) {
            return Ok(());
        }
        if settled {
            self.latest = None;
        }
        let current = if id == own {
            name == out.sender.name
        } else {
            world.visible_spawn(id).is_some_and(|spawn| {
                spawn.kind == SpawnKind::Player
                    && spawn.name == name
                    && in_range(position, spawn.position, true)
            })
        };
        if current {
            event(
                out,
                WorldEvent::PlayerInspection(PlayerInspection {
                    session_id: world.session_id,
                    name,
                    report: report.clone(),
                }),
            )?;
        }
        Ok(())
    }

    fn retire(&mut self, id: u16) {
        if self.pending.remove(&id).is_some() {
            self.retired.insert(id);
        }
        if self.latest == Some(id) {
            self.latest = None;
        }
    }

    fn reply_to_inspection(
        &mut self,
        target: u16,
        inspector: u16,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let now = Instant::now();
        let Some(note) = self.note.as_deref() else {
            return Ok(());
        };
        // Titanium uses a single one-second reply timer, regardless of inspector.
        if !world.is_player(target)
            || self
                .last_reply
                .is_some_and(|sent| now.duration_since(sent) < Duration::from_secs(1))
            || (!world.is_player(inspector)
                && !world
                    .spawns
                    .all()
                    .any(|spawn| spawn.spawn_id == inspector && spawn.kind == SpawnKind::Player))
        {
            return Ok(());
        }
        let Ok(packet) = inspection::titanium_reply(inspector, target, out.sender.name, note)
        else {
            return out
                .log
                .diagnostic("The inspect reply could not be encoded.".into());
        };
        out.send(&packet)?;
        self.last_reply = Some(now);
        Ok(())
    }
}

impl Feature for Dialogs {
    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::ServerDialogs, Capability::PlayerInspection]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::RespondToPopup { .. }
                | ClientCommand::InspectPlayer { .. }
                | ClientCommand::SetInspectNote { .. }
        )
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match command {
            ClientCommand::RespondToPopup {
                session_id,
                serial,
                action,
                ..
            } => {
                self.tick(Instant::now(), world, out)?;
                let Some(offer) = self.popup.as_ref().filter(|offer| {
                    offer.session_id == *session_id
                        && offer.serial == *serial
                        && offer.popup.allows(*action)
                }) else {
                    return actions::refuse(
                        command,
                        "That dialog or button is no longer available.",
                        out.log,
                    );
                };
                if let Some(packet) = popups::titanium_response(&offer.popup, *action) {
                    out.send(&packet)?;
                }
                self.close_popup(out)
            }
            ClientCommand::InspectPlayer { target, .. } => {
                self.inspect(command, *target, world, out)
            }
            ClientCommand::SetInspectNote { note, .. } => {
                if note
                    .as_ref()
                    .is_some_and(|note| note.len() >= 256 || note.as_bytes().contains(&0))
                {
                    return actions::refuse(
                        command,
                        "The inspect note must be under 256 bytes with no NUL.",
                        out.log,
                    );
                }
                self.note.clone_from(note);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if world.lifecycle.blocks_motion()
            || matches!(message, Message::ZoneOffer(_) | Message::Handoff(_))
        {
            self.retired
                .extend(std::mem::take(&mut self.pending).into_keys());
            self.latest = None;
            return self.close_popup(out);
        }
        let Message::Event(received) = message else {
            return Ok(());
        };
        match received {
            WorldEvent::Popup(popup) => {
                self.next_serial += 1;
                let offer = PopupOffer {
                    session_id: world.session_id,
                    serial: self.next_serial,
                    expires_at: popup.duration.map(|duration| Instant::now() + duration),
                    popup: popup.clone(),
                };
                self.popup = Some(offer.clone());
                event(out, WorldEvent::PopupOpened(offer))
            }
            WorldEvent::InspectRequest {
                target_id,
                inspector_id,
            } => self.reply_to_inspection(*target_id, *inspector_id, world, out),
            WorldEvent::InspectAnswer(report) => self.answer(report, world, out),
            WorldEvent::Despawn(id) => {
                self.retire(*id);
                Ok(())
            }
            WorldEvent::Spawns(spawns) => {
                for spawn in spawns {
                    self.retire(spawn.spawn_id);
                }
                Ok(())
            }
            WorldEvent::Death(death) => {
                if let Ok(id) = u16::try_from(death.spawn_id) {
                    self.retire(id);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if world.lifecycle.blocks_motion()
            || self
                .popup
                .as_ref()
                .is_some_and(|offer| offer.expires_at.is_some_and(|until| now >= until))
        {
            self.close_popup(out)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, targeting::Targeting};
    use super::*;
    use eq_network_game::popups::{Popup, PopupAction};

    #[test]
    fn an_old_button_cannot_answer_a_replacement_or_expired_dialog() {
        let mut feature = Dialogs::default();
        let mut world = World::new(5);
        let popup = Message::Event(WorldEvent::Popup(Popup {
            title: "Synthetic".into(),
            text: String::new(),
            buttons: 1,
            duration: Some(Duration::from_secs(10)),
            popup_id: 42,
            negative_id: 0,
        }));
        for _ in 0..2 {
            let opened = testing::run(|out| feature.observe(&popup, &mut world, out));
            opened.result.unwrap();
            assert_eq!(opened.sent, []);
        }
        let choose = |session_id, serial| ClientCommand::RespondToPopup {
            session_id,
            serial,
            action: PopupAction::Positive,
            created: Instant::now(),
        };
        for cmd in [choose(5, 1), choose(4, 2)] {
            let refused = testing::run(|out| feature.handle(&cmd, &mut world, out));
            refused.result.unwrap();
            assert_eq!(refused.sent, []);
            assert_eq!(feature.popup.as_ref().unwrap().serial, 2);
        }
        let sent = testing::run(|out| feature.handle(&choose(5, 2), &mut world, out));
        sent.result.unwrap();
        assert_eq!(sent.sent[0].body, [0, 0, 0, 0, 42, 0, 0, 0]);
        assert!(matches!(
            sent.events.as_slice(),
            [ClientEvent::World(WorldEvent::PopupClosed {
                session_id: 5,
                serial: 2
            })]
        ));
        let repeat = testing::run(|out| feature.handle(&choose(5, 2), &mut world, out));
        repeat.result.unwrap();
        assert_eq!(repeat.sent, []);

        testing::run(|out| feature.observe(&popup, &mut world, out))
            .result
            .unwrap();
        feature.popup.as_mut().unwrap().expires_at = Some(Instant::now());
        let expired = testing::run(|out| feature.handle(&choose(5, 3), &mut world, out));
        expired.result.unwrap();
        assert_eq!(expired.sent, []);
        assert!(feature.popup.is_none());
    }

    fn players() -> World {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        world.spawns.insert(testing::spawn(12, SpawnKind::Player));
        world
    }

    #[test]
    fn inspection_correlates_recipient_and_drops_a_recycled_spawn() {
        let mut feature = Dialogs::default();
        let mut world = players();
        testing::run(|out| {
            Targeting.handle(
                &ClientCommand::SelectTarget {
                    session_id: 5,
                    request_id: 1,
                    spawn_id: Some(12),
                    expected_pet: None,
                },
                &mut world,
                out,
            )
        })
        .result
        .unwrap();
        let inspect = ClientCommand::InspectPlayer {
            session_id: 5,
            target: world.target.state(),
            created: Instant::now(),
        };
        for _ in 0..2 {
            let request = testing::run(|out| feature.handle(&inspect, &mut world, out));
            request.result.unwrap();
            assert_eq!(request.sent, [inspection::titanium_request(12, 7).unwrap()]);
        }
        let reply = |recipient| {
            Message::Event(WorldEvent::InspectAnswer(Inspection {
                recipient_id: recipient,
                player_id: 12,
                equipment: Vec::new(),
                text: "Synthetic".into(),
            }))
        };
        let wrong = testing::run(|out| feature.observe(&reply(8), &mut world, out));
        wrong.result.unwrap();
        assert!(wrong.events.is_empty());
        let right = testing::run(|out| feature.observe(&reply(7), &mut world, out));
        right.result.unwrap();
        assert!(matches!(right.events.as_slice(), [
            ClientEvent::World(WorldEvent::PlayerInspection(PlayerInspection { session_id: 5, name, .. }))
        ] if name == "Spawn 12"));

        let replacement = Message::Event(WorldEvent::Spawns(vec![testing::spawn(
            12,
            SpawnKind::Player,
        )]));
        testing::run(|out| feature.observe(&replacement, &mut world, out))
            .result
            .unwrap();
        let refused = testing::run(|out| feature.handle(&inspect, &mut world, out));
        refused.result.unwrap();
        assert_eq!(refused.sent, []);
        let late = testing::run(|out| feature.observe(&reply(7), &mut world, out));
        late.result.unwrap();
        assert!(late.events.is_empty());
    }

    #[test]
    fn replacement_retires_an_outstanding_self_inspection() {
        let mut feature = Dialogs::default();
        let mut world = players();
        testing::run(|out| {
            Targeting.handle(
                &ClientCommand::SelectTarget {
                    session_id: 5,
                    request_id: 1,
                    spawn_id: Some(7),
                    expected_pet: None,
                },
                &mut world,
                out,
            )
        })
        .result
        .unwrap();
        let inspect = ClientCommand::InspectPlayer {
            session_id: 5,
            target: world.target.state(),
            created: Instant::now(),
        };
        let first = testing::run(|out| feature.handle(&inspect, &mut world, out));
        first.result.unwrap();
        assert_eq!(first.sent, [inspection::titanium_request(7, 7).unwrap()]);
        let replacement = Message::Event(WorldEvent::Spawns(vec![testing::spawn(
            7,
            SpawnKind::Player,
        )]));
        testing::run(|out| feature.observe(&replacement, &mut world, out))
            .result
            .unwrap();
        let refused = testing::run(|out| feature.handle(&inspect, &mut world, out));
        refused.result.unwrap();
        assert_eq!(refused.sent, []);
    }

    #[test]
    fn an_inspection_reply_needs_the_players_known_note_and_own_recipient() {
        let mut feature = Dialogs::default();
        let mut world = players();
        let request = |target_id| {
            Message::Event(WorldEvent::InspectRequest {
                target_id,
                inspector_id: 12,
            })
        };
        let unknown = testing::run(|out| feature.observe(&request(7), &mut world, out));
        unknown.result.unwrap();
        assert_eq!(unknown.sent, []);
        let config = ClientCommand::SetInspectNote {
            session_id: 5,
            note: Some("My note".into()),
        };
        testing::run(|out| feature.handle(&config, &mut world, out))
            .result
            .unwrap();
        let wrong = testing::run(|out| feature.observe(&request(8), &mut world, out));
        wrong.result.unwrap();
        assert_eq!(wrong.sent, []);
        let own = testing::run(|out| feature.observe(&request(7), &mut world, out));
        own.result.unwrap();
        assert_eq!(
            own.sent,
            [inspection::titanium_reply(12, 7, "Tester", "My note").unwrap()]
        );
        world.spawns.insert(testing::spawn(13, SpawnKind::Player));
        let other = Message::Event(WorldEvent::InspectRequest {
            target_id: 7,
            inspector_id: 13,
        });
        let throttled = testing::run(|out| feature.observe(&other, &mut world, out));
        throttled.result.unwrap();
        assert_eq!(throttled.sent, []);
    }
}
