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

/// Inferred until the click builder is read (6.537): the held item's ID in
/// place of the sentinel, every other byte as an ordinary click's.
#[test]
fn a_door_click_holding_an_item_carries_its_id_and_nothing_else_changes() {
    let ordinary = doors::titanium_click(9, 42, None);
    let holding = doors::titanium_click(9, 42, Some(0x0102_0304));
    assert_eq!(&holding.body[8..12], &0x0102_0304_u32.to_le_bytes());
    assert_eq!(holding.opcode, ordinary.opcode);
    assert_eq!(holding.body[..8], ordinary.body[..8]);
    assert_eq!(holding.body[12..], ordinary.body[12..]);
    assert_eq!(&holding.body[4..8], &[0; 4], "picklockskill stays zero");
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
