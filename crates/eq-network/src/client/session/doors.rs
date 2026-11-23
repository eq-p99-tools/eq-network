//! Doors: the zone's door table and the player's clicks on them.
#[cfg(test)]
mod eqmac_tests;
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::{anyhow, ensure, Result};
use eq_network_game::{
    doors::{eqmac, titanium, Clicked, DoorUpdate, OutOfReach},
    inventory::InventorySlot,
    message::Message,
    request::Request,
    world::{Position, WorldEvent, WorldEvent::DoorAction},
    GameDialect,
};
use std::{collections::BTreeMap, time::Instant};

/// The zone's doors in this admission.
pub(super) struct Doors {
    table: eq_network_game::doors::Doors,
    /// When the table last gained or lost doors; older clicks are stale.
    changed_at: Instant,
    dialect: GameDialect,
    /// Local ordinary-hinge motion; sending a request never changes it.
    hinges: BTreeMap<u8, eqmac::Hinge>,
    /// Titanium doors whose own return is a click to the server, and when.
    return_clicks: BTreeMap<u8, Instant>,
}

impl Default for Doors {
    fn default() -> Self {
        Self {
            table: eq_network_game::doors::Doors::default(),
            changed_at: Instant::now(),
            dialect: GameDialect::Titanium,
            hinges: BTreeMap::new(),
            return_clicks: BTreeMap::new(),
        }
    }
}

impl Doors {
    /// Offers ordinary hinged doors whose request, motion and guards are read.
    /// Specialized or unknown mechanisms remain definitions without use.
    pub(super) fn eqmac() -> Self {
        Self {
            dialect: GameDialect::EqMac,
            ..Self::default()
        }
    }

    #[expect(
        clippy::match_same_arms,
        reason = "each RoF2 arm stays its own until a read of its program replaces it (6.489)"
    )]
    fn apply(&mut self, update: &DoorUpdate, now: Instant, world: &mut World) {
        self.table.apply(update);
        match self.dialect {
            GameDialect::Titanium => {
                self.follow(update, now, world);
                return;
            }
            // Titanium's, inferred for RoF2 (6.489).
            GameDialect::Rof2 => {
                self.follow(update, now, world);
                return;
            }
            GameDialect::EqMac => {}
        }
        match update {
            DoorUpdate::RemoveAll => self.hinges.clear(),
            DoorUpdate::Spawn(doors) => {
                for door in doors {
                    if let Some(hinge) = eqmac::Hinge::new(door, now) {
                        self.hinges.insert(door.id, hinge);
                    } else {
                        self.hinges.remove(&door.id);
                    }
                }
            }
            DoorUpdate::Move { id, action } => {
                let door = self.table.entries().get(id);
                if let Some(door) = door.filter(|door| eqmac::ordinary(door)) {
                    if let Some(hinge) = self.hinges.get_mut(id) {
                        hinge.action(*action, now);
                    } else if let Some(hinge) = eqmac::Hinge::new(door, now) {
                        self.hinges.insert(*id, hinge);
                    }
                } else {
                    self.hinges.remove(id);
                }
            }
        }
    }

    /// Follows Titanium's own door motion and returns from the received update.
    fn follow(&mut self, update: &DoorUpdate, now: Instant, world: &mut World) {
        match update {
            DoorUpdate::RemoveAll => {
                world.lifts.clear();
                self.return_clicks.clear();
            }
            DoorUpdate::Spawn(doors) => {
                for door in doors {
                    world.lifts.spawned(door, now);
                    // A type 40 or 45 door spawned away from its rest
                    // schedules no return click: whether the program sends
                    // one then is unread, and sending none is the cautious
                    // side (recording checklist 6.423).
                    self.return_clicks.remove(&door.id);
                }
            }
            DoorUpdate::Move { id, .. } => {
                let Some(door) = self.table.entries().get(id) else {
                    return;
                };
                world.lifts.moved(door, now);
                match titanium::returns_at(door, now).filter(|_| titanium::returns_by_click(door)) {
                    Some(at) => self.return_clicks.insert(*id, at),
                    None => self.return_clicks.remove(id),
                };
            }
        }
    }

    /// The request for a door in reach, clicked since the door table last
    /// changed.
    #[expect(
        clippy::match_same_arms,
        reason = "each RoF2 arm stays its own until a read of its program replaces it (6.489)"
    )]
    fn click(
        &self,
        command: &ClientCommand,
        world: &World,
        reach: eq_network_game::reach::Reach,
    ) -> Result<Request> {
        let ClientCommand::ClickDoor {
            door_id,
            point,
            created,
            ..
        } = command
        else {
            unreachable!("only door clicks are clicked");
        };
        ensure!(*created >= self.changed_at, "stale door request");
        let (spawn_id, position) = world
            .player_at()
            .ok_or_else(|| anyhow!("player is unavailable"))?;
        let raised = world.lifts.offset(*door_id, Instant::now()).unwrap_or(0.0);
        // The program measures a click from where its ray hit the door, within
        // the player's size and 20 (read R16b, R16e; 6.530). With no point,
        // the reach runs to the door's origin, which is stricter.
        let at_point = |point: Position| {
            self.table.check_click_at(
                *door_id,
                spawn_id,
                position,
                Clicked {
                    point,
                    size: world.player_size().unwrap_or(0.0),
                    raised,
                },
            )
        };
        let at_origin = || {
            self.table
                .check_click_raised(*door_id, spawn_id, position, reach, raised)
        };
        match self.dialect {
            GameDialect::Titanium => point.map_or_else(at_origin, at_point)?,
            // Titanium's, inferred for RoF2 (6.489).
            GameDialect::Rof2 => point.map_or_else(at_origin, at_point)?,
            // EQMac's read reach takes no clicked point.
            GameDialect::EqMac => at_origin()?,
        }
        match self.dialect {
            GameDialect::Titanium => {}
            // Titanium's, inferred for RoF2 (6.489).
            GameDialect::Rof2 => {}
            GameDialect::EqMac => {
                let door = self
                    .table
                    .entries()
                    .get(door_id)
                    .ok_or_else(|| anyhow!("door is no longer available"))?;
                ensure!(
                    eqmac::ordinary(door),
                    "this door mechanism is not supported"
                );
            }
        }
        if self.dialect.requires_clear_cursor_for_doors() {
            ensure!(
                world.inventory.received() && !world.inventory.stale(),
                "the inventory is not loaded"
            );
            ensure!(
                !world.inventory.items().contains_key(&InventorySlot::CURSOR),
                "put down the item on your cursor first"
            );
        }
        match self.dialect {
            GameDialect::Titanium => {}
            // Titanium's, inferred for RoF2 (6.489).
            GameDialect::Rof2 => {}
            GameDialect::EqMac => {
                let mut hinge = self
                    .hinges
                    .get(door_id)
                    .cloned()
                    .ok_or_else(|| anyhow!("door motion is unavailable"))?;
                hinge.tick(Instant::now());
                ensure!(hinge.settled(), "door is moving");
            }
        }
        Ok(Request::ClickDoor(*door_id))
    }
}

impl Feature for Doors {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        vec![Capability::Doors]
    }

    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        if let Message::Event(WorldEvent::Doors(update)) = message {
            self.apply(update, Instant::now(), world);
        }
        Ok(())
    }

    fn admitted(&mut self, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        out.log.send(ClientEvent::World(WorldEvent::Doors(
            self.table.admission(),
        )))
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::ClickDoor { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::ClickDoor { door_id, .. } = command else {
            return Ok(());
        };
        let (error, string_id) = match self.click(command, world, out.wire.reaches().doors) {
            Ok(request) => {
                out.request(&request)?;
                (None, None)
            }
            Err(error) => (
                Some(error.to_string()),
                error
                    .downcast_ref::<OutOfReach>()
                    .map(|_| OutOfReach::STRING),
            ),
        };
        out.log.send(ClientEvent::World(DoorAction {
            session_id: world.session_id,
            door_id: *door_id,
            error,
            string_id,
        }))
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Message::Event(WorldEvent::Doors(update)) = message {
            if matches!(update, DoorUpdate::Spawn(_) | DoorUpdate::RemoveAll) {
                self.changed_at = Instant::now();
            }
            self.apply(update, Instant::now(), world);
        }
        Ok(())
    }

    fn tick(&mut self, now: Instant, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        for hinge in self.hinges.values_mut() {
            hinge.tick(now);
        }
        let due: Vec<u8> = self
            .return_clicks
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            self.return_clicks.remove(&id);
            out.request(&Request::ReturnDoor(id))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::{doors::Door, world::Position};
    use std::time::Duration;

    fn titanium_door(id: u8, open_type: u8, parameter: u32) -> Door {
        Door {
            id,
            model: "LIFT".into(),
            position: Position::default(),
            incline: 0,
            size: 100,
            open_type,
            state_at_spawn: 0,
            invert_state: 0,
            parameter,
            action: None,
        }
    }

    #[test]
    fn titanium_follows_its_lifts_and_returns_forty_and_forty_five_by_a_click() {
        let mut doors = Doors::default();
        let mut world = World::new(5);
        let spawn = WorldEvent::Doors(DoorUpdate::Spawn(vec![
            titanium_door(1, 59, 45),
            titanium_door(2, 40, 300),
            titanium_door(3, 50, 300),
        ]));
        doors.admit(&Message::Event(spawn), &mut world).unwrap();
        let start = Instant::now();
        assert_eq!(world.lifts.offset(1, start), Some(0.0));
        for id in 1..=3 {
            let moved = WorldEvent::Doors(DoorUpdate::Move { id, action: 2 });
            let outcome =
                testing::run(|out| doors.observe(&Message::Event(moved), &mut world, out));
            outcome.result.unwrap();
        }
        let later = Instant::now() + Duration::from_secs(1);
        assert!(world.lifts.offset(1, later).unwrap() > 8.9);
        // Type 40's own return 300 ms after it opens is a click naming no
        // player; type 50's is a local move and sends nothing.
        let early =
            testing::run(|out| doors.tick(start + Duration::from_millis(100), &mut world, out));
        early.result.unwrap();
        assert_eq!(early.sent, []);
        let due = testing::run(|out| doors.tick(later, &mut world, out));
        due.result.unwrap();
        assert_eq!(
            due.sent,
            [eq_network_game::doors::titanium::return_click(2)]
        );
        let again = testing::run(|out| doors.tick(later + Duration::from_secs(1), &mut world, out));
        again.result.unwrap();
        assert_eq!(again.sent, []);
        let removed = WorldEvent::Doors(DoorUpdate::RemoveAll);
        testing::run(|out| doors.observe(&Message::Event(removed), &mut world, out))
            .result
            .unwrap();
        assert_eq!(world.lifts.offset(1, later), None);
    }

    #[test]
    fn a_lift_is_clicked_where_it_stands_now() {
        let mut doors = Doors::default();
        let mut world = World::new(5);
        // A lift spawned raised, 45 up.
        let mut lift = titanium_door(1, 59, 45);
        lift.state_at_spawn = 1;
        let spawn = WorldEvent::Doors(DoorUpdate::Spawn(vec![lift]));
        doors.admit(&Message::Event(spawn), &mut world).unwrap();
        let click = ClientCommand::ClickDoor {
            session_id: 5,
            door_id: 1,
            point: None,
            created: Instant::now(),
        };
        let at = |z: f32| {
            let mut player = testing::player(7);
            player.position.z = z;
            player
        };
        let at = |x: f32, z: f32| {
            let mut player = at(z);
            player.position.x = x;
            player
        };
        // Reach runs to where it stands now: used from its top, not from its
        // base, halfway, or 21 units across its top.
        for (x, z, sent) in [
            (0.0, 0.0, 0),
            (0.0, 45.0, 1),
            (0.0, 22.5, 0),
            (21.0, 45.0, 0),
        ] {
            world.player.admit(at(x, z));
            let clicked = testing::run(|out| doors.handle(&click, &mut world, out));
            clicked.result.unwrap();
            assert_eq!(clicked.sent.len(), sent, "from {x}, {z}");
        }
    }

    /// A Titanium door at the origin, admitted, with a player of `race` at
    /// `at`; the click names `point`.
    fn clicked_from(
        race: u32,
        at: Position,
        point: Option<Position>,
    ) -> (Doors, World, ClientCommand) {
        let mut doors = Doors::default();
        let mut world = World::new(5);
        let spawn = WorldEvent::Doors(DoorUpdate::Spawn(vec![titanium_door(1, 0, 0)]));
        doors.admit(&Message::Event(spawn), &mut world).unwrap();
        let mut player = testing::player(7);
        player.race = race;
        player.position = at;
        world.player.admit(player);
        let click = ClientCommand::ClickDoor {
            session_id: 5,
            door_id: 1,
            point,
            created: Instant::now(),
        };
        (doors, world, click)
    }

    fn place(x: f32, y: f32, z: f32) -> Position {
        Position {
            x,
            y,
            z,
            ..Position::default()
        }
    }

    /// What a click did: how many requests went out, and the string named.
    fn outcome(
        doors: &mut Doors,
        world: &mut World,
        click: &ClientCommand,
    ) -> (usize, Option<u32>, Option<String>) {
        let outcome = testing::run(|out| doors.handle(click, world, out));
        outcome.result.unwrap();
        let [ClientEvent::World(DoorAction {
            string_id, error, ..
        })] = &outcome.events[..]
        else {
            panic!("one door answer: {:?}", outcome.events);
        };
        (outcome.sent.len(), *string_id, error.clone())
    }

    #[test]
    fn a_titanium_click_reaches_from_its_point_within_the_players_size_and_twenty() {
        let point = place(0.0, 0.0, 10.0);
        // A size 6 player (race 1) at exactly 26 is sent; the next float out
        // is refused with the program's line (more than size + 20).
        for (race, limit) in [(1, 26.0_f32), (12, 23.0)] {
            let (mut doors, mut world, click) =
                clicked_from(race, place(limit, 0.0, 10.0), Some(point));
            assert_eq!(outcome(&mut doors, &mut world, &click), (1, None, None));
            let (mut doors, mut world, click) =
                clicked_from(race, place(limit.next_up(), 0.0, 10.0), Some(point));
            assert_eq!(
                outcome(&mut doors, &mut world, &click),
                (
                    0,
                    Some(OutOfReach::STRING),
                    Some("door is out of reach".into())
                ),
                "race {race}"
            );
        }
        // The limit is 3-D: 18 across and 18 up is 25.5.
        let (mut doors, mut world, click) = clicked_from(1, place(18.0, 0.0, 28.0), Some(point));
        assert_eq!(outcome(&mut doors, &mut world, &click).0, 1);
        let (mut doors, mut world, click) = clicked_from(1, place(19.0, 0.0, 29.0), Some(point));
        assert_eq!(outcome(&mut doors, &mut world, &click).0, 0);
        // A race the server table does not list counts size 0: 20 and no more.
        let (mut doors, mut world, click) = clicked_from(0, place(20.0, 0.0, 10.0), Some(point));
        assert_eq!(outcome(&mut doors, &mut world, &click).0, 1);
        let (mut doors, mut world, click) =
            clicked_from(0, place(20.0_f32.next_up(), 0.0, 10.0), Some(point));
        assert_eq!(outcome(&mut doors, &mut world, &click).0, 0);
    }

    #[test]
    fn the_players_own_spawn_size_sets_the_click_limit_as_it_changes() {
        let point = place(0.0, 0.0, 0.0);
        let (mut doors, mut world, click) = clicked_from(1, place(30.0, 0.0, 0.0), Some(point));
        // Race 1 counts 6 while the own spawn is unheard: 30 is past 26.
        assert_eq!(outcome(&mut doors, &mut world, &click).0, 0);
        // The server told size 10 for the own spawn: 30 is in reach.
        let mut own = testing::spawn(7, eq_network_game::world::SpawnKind::Player);
        own.size = 10.0;
        world.spawns.insert(own.clone());
        assert_eq!(outcome(&mut doors, &mut world, &click).0, 1);
        // Shrunk to 2: the server's size wins over the race's 6, so 22 and
        // no further.
        own.apply_appearance(&eq_network_game::appearance::AppearanceChange::Size(2.0));
        world.spawns.insert(own);
        assert_eq!(outcome(&mut doors, &mut world, &click).0, 0);
        let at = |x: f32| ClientCommand::ClickDoor {
            session_id: 5,
            door_id: 1,
            point: Some(place(x, 0.0, 0.0)),
            created: Instant::now(),
        };
        assert_eq!(outcome(&mut doors, &mut world, &at(8.0)).0, 1);
        assert_eq!(outcome(&mut doors, &mut world, &at(7.99)).0, 0);
    }

    #[test]
    fn a_lift_resting_above_is_called_from_the_point_clicked_on_it() {
        // Run 5's case: a lift raised 45, its origin 30 across from a player on
        // its deck; the edge they click is 24 away.
        let mut doors = Doors::default();
        let mut world = World::new(5);
        let mut lift = titanium_door(1, 59, 45);
        lift.state_at_spawn = 1;
        let spawn = WorldEvent::Doors(DoorUpdate::Spawn(vec![lift]));
        doors.admit(&Message::Event(spawn), &mut world).unwrap();
        let mut player = testing::player(7);
        player.race = 1;
        player.position = place(30.0, 0.0, 45.0);
        world.player.admit(player);
        let click = |point| ClientCommand::ClickDoor {
            session_id: 5,
            door_id: 1,
            point,
            created: Instant::now(),
        };
        assert_eq!(
            outcome(&mut doors, &mut world, &click(Some(place(6.0, 0.0, 45.0)))),
            (1, None, None)
        );
        // With no point the origin rule stands: 30 is past 20.
        assert_eq!(
            outcome(&mut doors, &mut world, &click(None)).1,
            Some(OutOfReach::STRING)
        );
    }

    #[test]
    fn a_click_with_no_point_keeps_the_origin_rule() {
        for (x, sent) in [(17.0, 1), (20.0, 1), (20.0_f32.next_up(), 0), (21.0, 0)] {
            let (mut doors, mut world, click) = clicked_from(1, place(x, 0.0, 0.0), None);
            assert_eq!(outcome(&mut doors, &mut world, &click).0, sent, "from {x}");
        }
    }

    #[test]
    fn a_point_that_is_no_place_or_far_from_the_door_is_refused_in_our_words() {
        let at = place(200.0, 0.0, 0.0);
        for point in [
            place(f32::NAN, 0.0, 0.0),
            place(0.0, f32::INFINITY, 0.0),
            // Next to the player, but past 200 + 6 from the door.
            place(206.0_f32.next_up(), 0.0, 0.0),
        ] {
            let mut doors = Doors::default();
            let mut world = World::new(5);
            let spawn = WorldEvent::Doors(DoorUpdate::Spawn(vec![titanium_door(1, 0, 0)]));
            doors.admit(&Message::Event(spawn), &mut world).unwrap();
            let mut player = testing::player(7);
            player.race = 1;
            player.position = at;
            world.player.admit(player);
            let click = ClientCommand::ClickDoor {
                session_id: 5,
                door_id: 1,
                point: Some(point),
                created: Instant::now(),
            };
            let (sent, string_id, error) = outcome(&mut doors, &mut world, &click);
            assert_eq!((sent, string_id), (0, None), "{point:?}");
            assert!(error.unwrap().starts_with("The point you clicked is not"));
        }
        // 206 from the door is still a point on it.
        let (mut doors, mut world, click) =
            clicked_from(1, place(200.0, 0.0, 0.0), Some(place(206.0, 0.0, 0.0)));
        assert_eq!(outcome(&mut doors, &mut world, &click).0, 1);
    }

    #[test]
    fn each_generation_reaches_a_door_as_its_program_does() {
        let mut doors = Doors::default();
        let spawn = WorldEvent::Doors(DoorUpdate::Spawn(vec![Door {
            id: 1,
            model: "DOOR".into(),
            // 15 across and 15 up: 21.2 away, 15 on the ground.
            position: Position {
                x: 15.0,
                z: 15.0,
                ..Position::default()
            },
            incline: 0,
            size: 100,
            open_type: 0,
            state_at_spawn: 0,
            invert_state: 0,
            parameter: 0,
            action: None,
        }]));
        let mut world = World::new(5);
        doors.admit(&Message::Event(spawn), &mut world).unwrap();
        world.player.admit(testing::player(7));
        let click = ClientCommand::ClickDoor {
            session_id: 5,
            door_id: 1,
            point: None,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| doors.handle(&click, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(DoorAction {
                string_id: Some(OutOfReach::STRING),
                ..
            })]
        ));
        // EQMac's conservative base door reach is 2-D.
        assert_eq!(
            doors
                .click(&click, &world, eq_network_game::reach::EQMAC.doors)
                .unwrap(),
            Request::ClickDoor(1)
        );
    }

    #[test]
    fn a_fresh_click_on_a_door_in_reach_sends_the_click_and_reports_it() {
        let mut doors = Doors::default();
        let spawn = WorldEvent::Doors(DoorUpdate::Spawn(vec![Door {
            id: 1,
            model: "DOOR".into(),
            position: Position::default(),
            incline: 0,
            size: 100,
            open_type: 0,
            state_at_spawn: 0,
            invert_state: 0,
            parameter: 0,
            action: None,
        }]));
        doors
            .admit(&Message::Event(spawn), &mut World::new(5))
            .unwrap();
        let mut world = World::new(5);
        let click = ClientCommand::ClickDoor {
            session_id: 5,
            door_id: 1,
            point: None,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| doors.handle(&click, &mut world, out));
        outcome.result.unwrap();
        assert!(
            outcome.sent.is_empty(),
            "expected nothing sent, got {:?}",
            outcome.sent
        );
        assert!(matches!(
            &outcome.events[..],
            [ClientEvent::World(DoorAction { door_id: 1, error: Some(error), .. })]
                if error == "player is unavailable"
        ));
        world.player.admit(testing::player(7));
        let outcome = testing::run(|out| doors.handle(&click, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, eq_network_game::doors::CLICK_OPCODE);
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(DoorAction {
                door_id: 1,
                error: None,
                ..
            })]
        ));
    }

    #[test]
    fn clicks_made_before_the_doors_last_changed_are_stale() {
        let doors = Doors::default();
        let world = World::new(5);
        let click = |session_id, created| ClientCommand::ClickDoor {
            session_id,
            door_id: 1,
            point: None,
            created,
        };
        let error = |command: &ClientCommand| {
            doors
                .click(command, &world, eq_network_game::reach::TITANIUM.doors)
                .unwrap_err()
                .to_string()
        };
        let before = doors
            .changed_at
            .checked_sub(Duration::from_millis(1))
            .unwrap();
        assert_eq!(error(&click(5, before)), "stale door request");
        assert_eq!(error(&click(5, Instant::now())), "player is unavailable");
    }

    #[test]
    fn the_snapshot_carries_every_static_object_and_none_is_clicked() {
        let door = |id, model: &str| Door {
            id,
            model: model.into(),
            position: Position::default(),
            incline: 0,
            size: 100,
            open_type: 0,
            state_at_spawn: 0,
            invert_state: 0,
            parameter: 0,
            action: None,
        };
        let table = vec![door(255, "ROCKA"), door(1, "DOOR"), door(255, "ROCKB")];
        let mut doors = Doors::default();
        let mut world = World::new(5);
        doors
            .admit(
                &Message::Event(WorldEvent::Doors(DoorUpdate::Spawn(table))),
                &mut world,
            )
            .unwrap();
        let outcome = testing::run(|out| doors.admitted(&mut world, out));
        outcome.result.unwrap();
        assert!(matches!(
            &outcome.events[..],
            [ClientEvent::World(WorldEvent::Doors(DoorUpdate::Spawn(sent)))]
                if sent.iter().map(|door| door.model.as_str()).collect::<Vec<_>>()
                    == ["DOOR", "ROCKA", "ROCKB"]
        ));
        world.player.admit(testing::player(7));
        let click = ClientCommand::ClickDoor {
            session_id: 5,
            door_id: 255,
            point: None,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| doors.handle(&click, &mut world, out));
        outcome.result.unwrap();
        assert!(
            outcome.sent.is_empty(),
            "expected nothing sent, got {:?}",
            outcome.sent
        );
        assert!(matches!(
            &outcome.events[..],
            [ClientEvent::World(DoorAction { door_id: 255, error: Some(error), .. })]
                if error == "a static object cannot be used"
        ));
    }
}
