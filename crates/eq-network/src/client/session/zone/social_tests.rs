//! Ordinary socials through the assembled Project 1999 zone policy.
use super::super::feature::testing;
use super::*;
use eq_network_game::{
    listing::{Anonymity, ListingChange},
    socials::{self, ASSIST_OPCODE, EMOTE_OPCODE, ROLL_OPCODE},
    world::SpawnKind,
};

struct Visit {
    features: Features,
    world: World,
}

impl Visit {
    fn new() -> Self {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        world.admitted = Some(Instant::now());
        world.spawns.insert(testing::spawn(8, SpawnKind::Player));
        world.spawns.insert(testing::spawn(9, SpawnKind::Npc));
        let server = servers::server_type(crate::client::ServerProtocol::Project1999);
        let features = Features::new(server, "Tester", eq_network_game::food::AutoEat::default());
        Self { features, world }
    }

    fn command(&mut self, command: ClientCommand) -> testing::Outcome<Result<Option<ZoneExit>>> {
        testing::run(|out| {
            apply_commands(&[command], &mut self.features, &mut self.world, out, true)
        })
    }

    fn hear(&mut self, messages: Vec<Message>) -> testing::Outcome<Result<Option<ZoneExit>>> {
        testing::run(|out| hear(messages, &mut self.features, &mut self.world, out, true))
    }

    fn packet(&mut self, opcode: u16, body: &[u8]) -> testing::Outcome<Result<Option<ZoneExit>>> {
        self.hear(eq_network_game::message::titanium(opcode, body))
    }
}

/// The ordinary Titanium target request naming the spawn.
fn target_request(spawn_id: u16) -> eq_network_game::command::EncodedCommand {
    let select = ClientCommand::SelectTarget {
        session_id: 5,
        request_id: 0,
        spawn_id: Some(spawn_id),
        expected_pet: None,
    };
    let packet =
        eq_network_game::command::encode(eq_network_game::GameDialect::Titanium, &select, "Tester")
            .unwrap();
    assert_eq!(packet.opcode, 0x6c47);
    packet
}

#[test]
fn p99_assist_requires_a_live_request_and_does_not_send_a_combat_action() {
    let mut visit = Visit::new();
    let assist = ClientCommand::Assist {
        session_id: 5,
        spawn_id: 8,
    };
    let answer = 9u32.to_le_bytes();
    let unsolicited = visit.packet(ASSIST_OPCODE, &answer);
    assert!(unsolicited.result.unwrap().is_none());
    assert!(unsolicited.events.is_empty());

    let request = visit.command(assist.clone());
    assert!(request.result.unwrap().is_none());
    assert_eq!(request.sent, [socials::assist(8)]);
    assert_eq!(request.unreliable, []);
    assert!(request
        .events
        .iter()
        .all(|event| !matches!(event, ClientEvent::World(WorldEvent::Assisted(_)))));
    let malformed = visit.packet(ASSIST_OPCODE, &answer[..3]);
    assert!(malformed
        .events
        .iter()
        .all(|event| !matches!(event, ClientEvent::World(WorldEvent::Assisted(_)))));
    assert_eq!(visit.command(assist.clone()).sent, []);

    let reply = visit.packet(ASSIST_OPCODE, &answer);
    assert!(reply.result.unwrap().is_none());
    // The selection goes to the server as an ordinary target request.
    assert_eq!(reply.sent, [target_request(9)]);
    assert!(matches!(
        reply.events.as_slice(),
        [ClientEvent::World(WorldEvent::TargetChanged { state, .. })]
            if state.spawn_id == Some(9) && state.available
    ));

    assert_eq!(visit.command(assist.clone()).sent, [socials::assist(8)]);
    let _ = visit.hear(vec![Message::Event(WorldEvent::Target(Some(7)))]);
    let obsolete = visit.packet(ASSIST_OPCODE, &answer);
    assert!(obsolete.result.unwrap().is_none());
    assert!(obsolete.events.is_empty());
    assert_eq!(visit.command(assist).sent, [socials::assist(8)]);
    let no_target = visit.packet(ASSIST_OPCODE, &[0; 4]);
    assert!(no_target.events.is_empty());
    assert_eq!(*visit.world.target, Some(7));
}

#[test]
fn p99_assist_selection_is_ordered_with_newer_target_and_spawn_updates() {
    let mut visit = Visit::new();
    let assist = ClientCommand::Assist {
        session_id: 5,
        spawn_id: 8,
    };
    assert_eq!(visit.command(assist.clone()).sent, [socials::assist(8)]);
    let mut messages = eq_network_game::message::titanium(ASSIST_OPCODE, &9u32.to_le_bytes());
    messages.push(Message::Event(WorldEvent::Despawn(9)));
    messages.push(Message::Event(WorldEvent::Spawns(vec![testing::spawn(
        9,
        SpawnKind::Npc,
    )])));
    messages.push(Message::Event(WorldEvent::Target(Some(7))));
    let received = visit.hear(messages);
    assert!(received.result.unwrap().is_none());
    assert_eq!(received.sent, [target_request(9)]);
    let selections: Vec<_> = received
        .events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::World(WorldEvent::TargetChanged { state, .. }) => {
                Some((state.spawn_id, state.available))
            }
            ClientEvent::World(WorldEvent::Assisted(_)) => {
                panic!("P99 must not hand the host a deferred assist selection")
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        selections,
        [
            (Some(9), true),
            (Some(9), false),
            (Some(9), false),
            (Some(7), true)
        ]
    );
    assert_eq!(*visit.world.target, Some(7));
}

#[test]
fn p99_dice_uses_native_bounds_and_a_received_result() {
    let mut visit = Visit::new();
    let random = |low, high| ClientCommand::Random {
        session_id: 5,
        low,
        high,
    };
    assert_eq!(visit.command(random(0, 32_001)).sent, []);
    let request = visit.command(random(32_000, 10));
    assert!(request.result.unwrap().is_none());
    assert_eq!(request.sent, [socials::random(10, 32_000)]);
    assert!(request.events.is_empty());
    let mut reply = [0; 76];
    reply[..4].copy_from_slice(&10u32.to_le_bytes());
    reply[4..8].copy_from_slice(&32_000u32.to_le_bytes());
    reply[8..12].copy_from_slice(&15u32.to_le_bytes());
    reply[12..18].copy_from_slice(b"Helper");
    assert!(
        matches!(visit.packet(ROLL_OPCODE, &reply).events.as_slice(), [
        ClientEvent::World(WorldEvent::Roll(roll))
    ] if roll.name == "Helper" && roll.low == 10 && roll.high == 32_000 && roll.result == 15)
    );
}

#[test]
fn p99_emote_sends_a_fixed_record_and_identifies_local_display() {
    let mut visit = Visit::new();
    let request = visit.command(ClientCommand::Emote {
        session_id: 5,
        text: " smiles 100%. ".into(),
    });
    assert!(request.result.unwrap().is_none());
    assert_eq!(request.sent.len(), 1);
    let packet = &request.sent[0];
    assert_eq!(packet.opcode, EMOTE_OPCODE);
    assert_eq!(packet.body.len(), 1028);
    assert_eq!(&packet.body[..4], &[0; 4]);
    assert_eq!(&packet.body[4..22], b" smiles 100&PCT;.\0");
    assert!(matches!(request.events.as_slice(), [
        ClientEvent::Record(record)
    ] if matches!(&record.event, RecordEvent::Chat(chat)
        if chat.message_type == Some(314)
            && chat.message.as_ref().is_some_and(|message| message.text == "Tester smiles 100%."))));
    let oversized = visit.command(ClientCommand::Emote {
        session_id: 5,
        text: "%".repeat(102),
    });
    assert_eq!(oversized.sent, []);
}

#[test]
fn p99_listing_submits_without_replacing_received_listing() {
    use eq_network_game::listing::{titanium_anonymity, titanium_away};

    let mut visit = Visit::new();
    let away = ClientCommand::ToggleAway { session_id: 5 };
    let requested = visit.command(away.clone());
    assert_eq!(requested.sent, [titanium_away(7, true).unwrap()]);
    assert!(matches!(
        requested.events.as_slice(),
        [ClientEvent::World(WorldEvent::ListingRequested {
            session_id: 5,
            change: ListingChange::Away(true)
        })]
    ));
    assert!(!visit.world.player.as_ref().unwrap().listing.away);
    assert_eq!(visit.command(away).sent, []);
    let packet = titanium_away(7, true).unwrap();
    let confirmed = visit.packet(packet.opcode, &packet.body);
    assert!(confirmed.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::Listing {
            spawn_id: 7,
            change: ListingChange::Away(true)
        })
    )));

    let mut anonymous = Visit::new();
    assert_eq!(
        anonymous
            .command(ClientCommand::ToggleAnonymous { session_id: 5 })
            .sent,
        [titanium_anonymity(7, Anonymity::Anonymous).unwrap()]
    );
    let mut roleplay = Visit::new();
    assert_eq!(
        roleplay
            .command(ClientCommand::ToggleRoleplay { session_id: 5 })
            .sent,
        [titanium_anonymity(7, Anonymity::Roleplaying).unwrap()]
    );
}
