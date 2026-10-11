//! Independently authored fixtures for formats identified during passive replay.
use eq_network_game::{chat, raid};

#[test]
fn normal_door_click_uses_the_official_no_item_sentinel() {
    let packet = eq_network_game::doors::titanium_click(9, 42);
    assert_eq!(packet.body[1], 3);
    assert_eq!(&packet.body[8..12], &u32::MAX.to_le_bytes());
    assert_eq!(&packet.body[12..14], &42_u16.to_le_bytes());
}

#[test]
fn manual_consumption_uses_the_official_reserved_word() {
    use eq_network_game::{
        food::{self, Meal},
        inventory::InventorySlot,
    };
    for meal in [Meal::Food, Meal::Drink] {
        let packet = food::consume(InventorySlot(23), meal, true);
        assert_eq!(&packet.body[4..8], &999_u32.to_le_bytes());
        assert_eq!(&packet.body[8..12], &u32::MAX.to_le_bytes());
    }
}

#[test]
fn zone_environment_preserves_axes_settings_and_rejects_bad_floats() {
    use eq_network_game::{environment, message, world::WorldEvent};
    let mut body = [0; 700];
    body[64..72].copy_from_slice(b"testzone");
    body[96..105].copy_from_slice(b"Test Zone");
    body[374] = 255;
    body[425..429].copy_from_slice(&[10, 20, 30, 40]);
    body[429..433].copy_from_slice(&[1, 2, 3, 4]);
    body[684..686].copy_from_slice(&42_u16.to_le_bytes());
    body[686..688].copy_from_slice(&7_u16.to_le_bytes());
    for (at, value) in [
        (420, 0.4_f32),
        (488, 1.25),
        (492, -12.0),
        (496, 27.0),
        (500, 3.0),
        (512, 10.0),
        (516, 800.0),
    ] {
        body[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    let zone = environment::titanium(&body).unwrap();
    assert_eq!(zone.safe_position, [27.0, -12.0, 3.0]);
    assert_eq!((zone.zone_id, zone.instance_id), (42, 7));
    assert_eq!(zone.short_name, "testzone");
    assert_eq!(zone.long_name, "Test Zone");
    assert_eq!(zone.gravity, 0.4);
    assert_eq!(zone.far_clip, 800.0);
    assert_eq!(zone.seasons[2].rain_chance, 30);
    assert_eq!(zone.seasons[2].rain_duration, 3);
    assert!(matches!(
        message::titanium(environment::TITANIUM_OPCODE, &body).as_slice(),
        [message::Message::Event(WorldEvent::ZoneEnvironment(_))]
    ));
    for length in 0..700 {
        assert!(environment::titanium(&body[..length]).is_err());
    }
    assert!(environment::titanium(&[body.as_slice(), &[0]].concat()).is_err());
    for offset in [388, 404, 420, 488, 492, 496, 500, 512, 516] {
        let mut bad = body;
        bad[offset..offset + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(environment::titanium(&bad).is_err());
    }
}

#[test]
fn destroying_cursor_coins_cannot_draw_from_other_places() {
    use eq_network_game::{
        money::{Coin, CoinDestruction, Wallet},
        request::{self, Request, Sender},
        world::Coins,
    };
    let mut wallet = Wallet {
        cursor: Coins {
            silver: 9,
            ..Coins::default()
        },
        purse: Some(Coins {
            silver: 100,
            ..Coins::default()
        }),
        ..Wallet::default()
    };
    let destruction = CoinDestruction {
        coin: Coin::Silver,
        amount: 7,
    };
    let request = Request::DestroyCoins(destruction);
    let sender = Sender {
        name: "Tester",
        spawn_id: Some(7),
    };
    let encoded = request::titanium(&request, sender).unwrap();
    assert_eq!(encoded.opcode, 0x7657);
    let expected: Vec<_> = [0_u32, u32::MAX, 1, 1, 7]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    assert_eq!(encoded.body, expected);
    assert!(request::eqmac(&request, sender).is_err());
    wallet.destroy(destruction).unwrap();
    assert_eq!(wallet.cursor.silver, 2);
    assert_eq!(wallet.purse.unwrap().silver, 100);
    let before = wallet;
    for amount in [0, 3, u32::MAX] {
        assert!(wallet
            .destroy(CoinDestruction {
                amount,
                ..destruction
            })
            .is_err());
        assert_eq!(wallet, before);
    }
    assert!(CoinDestruction {
        amount: 0,
        ..destruction
    }
    .encode()
    .is_err());
    assert!(CoinDestruction {
        amount: u32::MAX,
        ..destruction
    }
    .encode()
    .is_err());
}

#[test]
fn colored_system_text_is_not_silently_dropped() {
    let mut body = 335_u32.to_le_bytes().to_vec();
    body.extend_from_slice(b"A synthetic notification.\0");
    let event = chat::parse(0x0b2d, &body, false)
        .unwrap()
        .expect("colored text");
    assert_eq!(event.channel_name, chat::ChannelName::System);
    assert_eq!(event.message_type, Some(335));
    assert_eq!(event.message.unwrap().text, "A synthetic notification.");
}

#[test]
fn raid_absence_is_not_silently_dropped() {
    let mut body = [0; 136];
    body[..4].copy_from_slice(&10_u32.to_le_bytes());
    assert_eq!(
        raid::decode(raid::UPDATE_OPCODE, &body).unwrap(),
        Some(raid::RaidUpdate::NotInRaid)
    );
    for length in 0..136 {
        assert!(raid::decode(raid::UPDATE_OPCODE, &body[..length]).is_err());
    }
    assert!(raid::decode(raid::UPDATE_OPCODE, &[body.as_slice(), &[0]].concat()).is_err());
    assert_eq!(
        raid::decode(raid::UPDATE_OPCODE, &[body.as_slice(), &[0; 4]].concat()).unwrap(),
        Some(raid::RaidUpdate::NotInRaid)
    );
}

#[test]
fn colored_text_preserves_unknown_colors_and_item_links() {
    let mut body = u32::MAX.to_le_bytes().to_vec();
    body.extend_from_slice(b"Found \x12");
    body.extend_from_slice(&[b'0'; 45]);
    body.extend_from_slice(b"Test Item\x12\0");
    let event = chat::parse(0x0b2d, &body, false).unwrap().unwrap();
    assert_eq!(event.message_type, Some(u32::MAX));
    let message = event.message.unwrap();
    assert_eq!(message.text, "Found Test Item");
    assert_eq!(message.item_links.len(), 1);
    assert!(message.message_hex.is_none());
    for length in 0..5 {
        assert!(chat::parse(0x0b2d, &body[..length], false).is_err());
    }
    assert!(chat::parse(0x0b2d, &body[..body.len() - 1], false).is_err());
    assert!(
        chat::parse_for(eq_network_game::GameDialect::EqMac, 0x0b2d, &body, false)
            .unwrap()
            .is_none()
    );
    let empty = chat::parse(0x0b2d, &[0; 5], false).unwrap().unwrap();
    assert_eq!(empty.message.unwrap().text, "");
}
