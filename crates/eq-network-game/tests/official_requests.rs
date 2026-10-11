//! Requests whose bytes follow the official client's, with synthetic values.
use eq_network_game::{
    doors,
    food::{self, Meal},
    inventory::InventorySlot,
};

#[test]
fn normal_door_click_uses_the_official_no_item_sentinel() {
    let packet = doors::titanium_click(9, 42);
    assert_eq!(packet.body[1], 3);
    assert_eq!(&packet.body[8..12], &u32::MAX.to_le_bytes());
    assert_eq!(&packet.body[12..14], &42_u16.to_le_bytes());
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
