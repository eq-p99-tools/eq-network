//! Requests whose bytes follow the official client's, and the values kept
//! where the official client's are unrecorded; the IDs and slots are made up.
use eq_network_game::{
    doors,
    food::{self, Meal},
    inventory::InventorySlot,
};

#[test]
fn normal_door_click_uses_the_official_no_item_sentinel() {
    let packet = doors::titanium_click(9, 42, None);
    assert_eq!(packet.body[1], 3);
    assert_eq!(&packet.body[8..12], &u32::MAX.to_le_bytes());
    assert_eq!(&packet.body[12..14], &42_u16.to_le_bytes());
}

/// Read R49-22b (0x4906c5): the cursor item's ID in place of the sentinel,
/// and the skill only for a lockpick, at most 254.
#[test]
fn a_door_click_holding_an_item_carries_its_id_and_a_lockpick_its_skill() {
    let ordinary = doors::titanium_click(9, 42, None);
    let key = doors::titanium_click(
        9,
        42,
        Some(doors::HeldItem {
            id: 0x0102_0304,
            skill: 0,
        }),
    );
    assert_eq!(&key.body[8..12], &0x0102_0304_u32.to_le_bytes());
    assert_eq!(key.opcode, ordinary.opcode);
    assert_eq!(key.body[..8], ordinary.body[..8]);
    assert_eq!(key.body[12..], ordinary.body[12..]);
    assert_eq!(&key.body[4..8], &[0; 4], "a key carries no skill");
    for (skill, sent) in [(120, 120_u32), (300, 254)] {
        let pick = doors::titanium_click(
            9,
            42,
            Some(doors::HeldItem {
                id: 0x0102_0304,
                skill,
            }),
        );
        assert_eq!(&pick.body[4..8], &sent.to_le_bytes());
        assert_eq!(&pick.body[8..12], &0x0102_0304_u32.to_le_bytes());
    }
}

#[test]
fn manual_consumption_uses_the_official_reserved_word() {
    for meal in [Meal::Food, Meal::Drink] {
        let packet = food::consume(InventorySlot(23), meal, true);
        assert_eq!(&packet.body[4..8], &999_u32.to_le_bytes());
        assert_eq!(&packet.body[8..12], &u32::MAX.to_le_bytes());
    }
}

#[test]
fn consumption_on_its_own_keeps_the_reserved_word_zero() {
    for meal in [Meal::Food, Meal::Drink] {
        let packet = food::consume(InventorySlot(23), meal, false);
        assert_eq!(&packet.body[4..8], &u32::MAX.to_le_bytes());
        assert_eq!(&packet.body[8..12], &[0; 4]);
    }
}
