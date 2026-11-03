//! Synthetic packets through the real generation decoder, zone dispatcher,
//! corpse controller and inventory owner, selected by Quarm's server policy.
use super::*;
use crate::client::{
    session::{feature::testing, wire::EqMac},
    ServerProtocol,
};
use eq_network_game::{
    food::AutoEat,
    inventory::{InventorySlot, InventoryUpdate},
    loot::{eqmac, LootUpdate},
    world::{Coins, SpawnKind, WorldEvent},
};

fn setup(kind: SpawnKind) -> (Features, World) {
    let server = servers::server_type(ServerProtocol::Quarm);
    let mut features = Features::new(server, "Tester", AutoEat::default());
    let mut world = World::new(5);
    world.own_spawn = Some(7);
    world.player.admit(testing::player(7));
    world.admitted = Some(Instant::now());
    world.spawns.insert(testing::spawn(9, kind));
    features
        .admit(
            &Message::Event(WorldEvent::Inventory(InventoryUpdate::Snapshot(Vec::new()))),
            &mut world,
        )
        .unwrap();
    testing::run_on(server.wire(), |out| features.admitted(&mut world, out))
        .result
        .unwrap();
    (features, world)
}

fn open() -> ClientCommand {
    ClientCommand::Loot {
        session_id: 5,
        corpse_id: 9,
        created: Instant::now(),
    }
}

fn take() -> ClientCommand {
    ClientCommand::LootItem {
        session_id: 5,
        corpse_id: 9,
        own_id: 7,
        place: 0,
        auto: false,
        created: Instant::now(),
    }
}

fn close() -> ClientCommand {
    ClientCommand::EndLoot {
        session_id: 5,
        corpse_id: 9,
    }
}

fn packet(
    features: &mut Features,
    world: &mut World,
    out: &mut Out<'_, '_>,
    opcode: u16,
    body: &[u8],
) -> Result<()> {
    let rules = servers::server_type(ServerProtocol::Quarm).item_rules();
    assert!(hear(
        out.wire.messages(opcode, body, rules),
        features,
        world,
        out,
        true
    )?
    .is_none());
    Ok(())
}

fn money(response: u8) -> [u8; 20] {
    let mut body = [0; 20];
    body[0] = response;
    body[4..8].copy_from_slice(&2u32.to_le_bytes());
    body[16..].copy_from_slice(&9u32.to_le_bytes());
    body
}

fn item(slot: i16) -> [u8; 360] {
    let mut body = [0; 360];
    body[..7].copy_from_slice(b"Fixture");
    body[180..182].copy_from_slice(&101i16.to_le_bytes());
    body[184..186].copy_from_slice(&slot.to_le_bytes());
    body
}

fn list(features: &mut Features, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
    packet(features, world, out, eqmac::MONEY_OPCODE, &money(1))?;
    packet(features, world, out, eqmac::VIEW_OPCODE, &item(0))?;
    packet(features, world, out, eqmac::REQUEST_OPCODE, &[9, 0])
}

#[test]
fn npc_and_player_corpse_delivery_belongs_to_inventory_even_after_close() {
    for kind in [SpawnKind::NpcCorpse, SpawnKind::PlayerCorpse] {
        let (mut features, mut world) = setup(kind);
        let outcome = testing::run_on(&EqMac, |out| -> Result<()> {
            // Only the real inventory owner establishes the initial purse.
            hear(
                vec![Message::Event(WorldEvent::Coins(Coins::default()))],
                &mut features,
                &mut world,
                out,
                true,
            )?;
            features.handle(&open(), &mut world, out)?;
            list(&mut features, &mut world, out)?;
            assert!(world.inventory.items().is_empty());
            assert_eq!(world.coins.purse.unwrap().platinum, 2);
            // A repeated success response must not add the same coins twice.
            packet(
                &mut features,
                &mut world,
                out,
                eqmac::MONEY_OPCODE,
                &money(1),
            )?;
            assert_eq!(world.coins.purse.unwrap().platinum, 2);
            features.handle(&take(), &mut world, out)?;
            features.handle(&take(), &mut world, out)?;
            packet(
                &mut features,
                &mut world,
                out,
                eqmac::ITEM_OPCODE,
                &eqmac::item_request(9, 7, 0, false)?,
            )?;
            assert!(world.inventory.items().is_empty());
            features.handle(&close(), &mut world, out)?;
            features.handle(&take(), &mut world, out)?;
            packet(&mut features, &mut world, out, eqmac::COMPLETE_OPCODE, &[])?;
            // A close is not a rollback: the real server delivery still lands.
            packet(&mut features, &mut world, out, 0x7841, &item(0))?;
            assert_eq!(
                world.inventory.items()[&InventorySlot::CURSOR].details.id,
                101
            );
            features.handle(&take(), &mut world, out)?;
            Ok(())
        });
        outcome.result.unwrap();
        assert_eq!(
            outcome
                .sent
                .iter()
                .map(|packet| packet.opcode)
                .collect::<Vec<_>>(),
            [eqmac::REQUEST_OPCODE, eqmac::ITEM_OPCODE, eqmac::END_OPCODE]
        );
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Loot(LootUpdate::Acknowledged { place: 0 }))
        )));
        assert!(!outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Loot(LootUpdate::Taken { .. }))
        )));
    }
}

#[test]
fn permission_denial_and_ack_then_failure_never_deliver_an_item() {
    let (mut features, mut world) = setup(SpawnKind::PlayerCorpse);
    let outcome = testing::run_on(&EqMac, |out| -> Result<()> {
        hear(
            vec![Message::Event(WorldEvent::Coins(Coins::default()))],
            &mut features,
            &mut world,
            out,
            true,
        )?;
        for response in [0, 2, 3] {
            features.handle(&open(), &mut world, out)?;
            packet(
                &mut features,
                &mut world,
                out,
                eqmac::MONEY_OPCODE,
                &money(response),
            )?;
            // A list echo after denial is not an open grant.
            packet(
                &mut features,
                &mut world,
                out,
                eqmac::REQUEST_OPCODE,
                &[9, 0],
            )?;
            features.handle(&take(), &mut world, out)?;
            assert_eq!(world.coins.purse, Some(Coins::default()));
        }
        features.handle(&open(), &mut world, out)?;
        list(&mut features, &mut world, out)?;
        features.handle(&take(), &mut world, out)?;
        packet(
            &mut features,
            &mut world,
            out,
            eqmac::ITEM_OPCODE,
            &eqmac::item_request(9, 7, 0, false)?,
        )?;
        packet(&mut features, &mut world, out, eqmac::COMPLETE_OPCODE, &[])?;
        assert!(world.inventory.items().is_empty());
        features.handle(&take(), &mut world, out)?;
        Ok(())
    });
    outcome.result.unwrap();
    assert_eq!(
        outcome
            .sent
            .iter()
            .filter(|packet| packet.opcode == eqmac::ITEM_OPCODE)
            .count(),
        1
    );
}

#[test]
fn close_while_opening_and_new_admission_reject_late_loot_messages() {
    let (mut features, mut world) = setup(SpawnKind::NpcCorpse);
    let outcome = testing::run_on(&EqMac, |out| -> Result<()> {
        hear(
            vec![Message::Event(WorldEvent::Coins(Coins::default()))],
            &mut features,
            &mut world,
            out,
            true,
        )?;
        features.handle(&open(), &mut world, out)?;
        features.handle(&close(), &mut world, out)?;
        list(&mut features, &mut world, out)?;
        features.handle(&take(), &mut world, out)?;
        packet(&mut features, &mut world, out, eqmac::COMPLETE_OPCODE, &[])?;
        features.handle(&open(), &mut world, out)?;
        world.session_id = 6;
        packet(
            &mut features,
            &mut world,
            out,
            eqmac::MONEY_OPCODE,
            &money(1),
        )?;
        packet(&mut features, &mut world, out, eqmac::VIEW_OPCODE, &item(0))?;
        features.handle(&take(), &mut world, out)?;
        assert!(world.inventory.items().is_empty());
        assert_eq!(
            world.coins.purse,
            Some(Coins {
                platinum: 2,
                copper: 9,
                ..Coins::default()
            })
        );
        Ok(())
    });
    outcome.result.unwrap();
    assert_eq!(
        outcome
            .sent
            .iter()
            .map(|packet| packet.opcode)
            .collect::<Vec<_>>(),
        [
            eqmac::REQUEST_OPCODE,
            eqmac::END_OPCODE,
            eqmac::REQUEST_OPCODE
        ]
    );
}

#[test]
fn death_removal_and_connection_end_discard_corpse_access() {
    use eq_network_game::zoning::Death;
    for reset in [
        Some(WorldEvent::Death(Death {
            spawn_id: 7,
            killer_id: 0,
            corpse_id: 12,
            bind_zone_id: 2,
            corpse_name: None,
        })),
        Some(WorldEvent::Despawn(9)),
        None,
    ] {
        let (mut features, mut world) = setup(SpawnKind::NpcCorpse);
        let outcome = testing::run_on(&EqMac, |out| -> Result<()> {
            features.handle(&open(), &mut world, out)?;
            list(&mut features, &mut world, out)?;
            if let Some(event) = reset {
                hear(
                    vec![Message::Event(event)],
                    &mut features,
                    &mut world,
                    out,
                    true,
                )?;
            } else {
                features.connection_ended(&mut world);
            }
            features.handle(&take(), &mut world, out)?;
            Ok(())
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert!(world.inventory.items().is_empty());
    }
}

#[test]
fn group_coin_notices_and_bag_contents_use_existing_inventory_paths() {
    let (mut features, mut world) = setup(SpawnKind::PlayerCorpse);
    let outcome = testing::run_on(&EqMac, |out| -> Result<()> {
        hear(
            vec![Message::Event(WorldEvent::Coins(Coins::default()))],
            &mut features,
            &mut world,
            out,
            true,
        )?;
        features.handle(&open(), &mut world, out)?;
        let mut grant = [0; 20];
        grant[0] = 1;
        packet(&mut features, &mut world, out, eqmac::MONEY_OPCODE, &grant)?;
        packet(
            &mut features,
            &mut world,
            out,
            eq_network_game::money::EQMAC_PURSE_OPCODE,
            &[0, 0, 3, 0, 2, 0, 0, 0],
        )?;
        assert_eq!(world.coins.purse.unwrap().platinum, 2);
        let mut bag = item(0);
        bag[178..180].copy_from_slice(&1i16.to_le_bytes());
        bag[269] = 4;
        bag[271] = 10;
        packet(&mut features, &mut world, out, eqmac::VIEW_OPCODE, &bag)?;
        packet(
            &mut features,
            &mut world,
            out,
            eqmac::REQUEST_OPCODE,
            &[9, 0],
        )?;
        assert!(world.inventory.items().is_empty());
        let mut automatic = take();
        if let ClientCommand::LootItem { auto, .. } = &mut automatic {
            *auto = true;
        }
        features.handle(&automatic, &mut world, out)?;
        packet(
            &mut features,
            &mut world,
            out,
            eqmac::ITEM_OPCODE,
            &eqmac::item_request(9, 7, 0, true)?,
        )?;
        bag[184..186].copy_from_slice(&22i16.to_le_bytes());
        packet(&mut features, &mut world, out, 0x3140, &bag)?;
        packet(&mut features, &mut world, out, 0x3140, &item(250))?;
        assert_eq!(world.inventory.items()[&InventorySlot(22)].bag_slots, 4);
        assert_eq!(world.inventory.items()[&InventorySlot(251)].details.id, 101);
        Ok(())
    });
    outcome.result.unwrap();
    assert_eq!(outcome.sent.len(), 2);
    assert_eq!(&outcome.sent[1].body[8..], &[1, 0, 0, 0]);
}

#[test]
fn occupied_or_unknown_item_cursor_refuses_a_loot_item_without_consuming_its_place() {
    let (mut features, mut world) = setup(SpawnKind::NpcCorpse);
    let outcome = testing::run_on(&EqMac, |out| -> Result<()> {
        features.handle(&open(), &mut world, out)?;
        list(&mut features, &mut world, out)?;
        packet(&mut features, &mut world, out, 0x7841, &item(0))?;
        features.handle(&take(), &mut world, out)?;
        let mut remove = [0xff; 12];
        remove[..4].copy_from_slice(&0u32.to_le_bytes());
        packet(&mut features, &mut world, out, 0x2c41, &remove)?;
        hear(
            vec![Message::Event(WorldEvent::Inventory(
                InventoryUpdate::Invalidated,
            ))],
            &mut features,
            &mut world,
            out,
            true,
        )?;
        features.handle(&take(), &mut world, out)?;
        hear(
            vec![Message::Event(WorldEvent::Inventory(
                InventoryUpdate::Snapshot(Vec::new()),
            ))],
            &mut features,
            &mut world,
            out,
            true,
        )?;
        features.handle(&take(), &mut world, out)?;
        Ok(())
    });
    outcome.result.unwrap();
    assert_eq!(
        outcome
            .sent
            .iter()
            .map(|packet| packet.opcode)
            .collect::<Vec<_>>(),
        [eqmac::REQUEST_OPCODE, eqmac::ITEM_OPCODE]
    );
}
