//! Food and drink. Servers count how fed and watered the player is and
//! report it now and then, at most 6000 of each (`OP_Stamina`: every 46
//! seconds on `EQEmu`, a little less each time); at 3000 or less the player
//! is hungry or thirsty, and the official client eats or drinks from the
//! inventory on its own. The player may also eat or drink an item by hand,
//! which counts half. Servers report no item eaten: the client takes it from
//! the inventory itself.
//!
//! Layout reference: `EQEmu`'s `Consume_Struct` and `Stamina_Struct`
//! (`common/eq_packet_structs.h`) and the Titanium profile's `thirst_level`
//! and `hunger_level` (`common/patches/titanium_structs.h`, offsets 5000 and
//! 5004); `Client::Handle_OP_Consume` (`zone/client_packet.cpp`),
//! `Client::Consume` and `Client::Hungry` (`zone/client.cpp`, `zone/client.h`)
//! and `Client::DoStaminaHungerUpdate` (`zone/client_process.cpp`) for the
//! rules.
use crate::{command::EncodedCommand, inventory::InventorySlot};
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_Stamina`: how fed and watered the player is.
pub const STAMINA_OPCODE: u16 = 0x7a83;
/// `OP_Consume`: the player eats or drinks an item.
pub const CONSUME_OPCODE: u16 = 0x77d6;
/// At or below this much food or drink, servers count the player hungry or
/// thirsty, and the official client eats or drinks.
pub const HUNGRY: u32 = 3000;
/// The most servers report; one who has this much can eat or drink no more.
pub const FULL: u32 = 6000;
/// `Consume_Struct.auto_consumed` for what the client eats on its own.
const ON_ITS_OWN: u32 = u32::MAX;
/// The same for a bite by hand, which servers count half.
const BY_HAND: u32 = 999;

/// Food or drink.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Meal {
    /// Food: an item of type 14 (`ItemTypeFood`).
    Food,
    /// Drink: an item of type 15 (`ItemTypeDrink`).
    Drink,
}

impl Meal {
    /// What an item of this type is, if food or drink.
    #[must_use]
    pub const fn of_item_type(item_type: u8) -> Option<Self> {
        match item_type {
            14 => Some(Self::Food),
            15 => Some(Self::Drink),
            _ => None,
        }
    }

    /// `Consume_Struct.type`.
    const fn wire(self) -> u8 {
        match self {
            Self::Food => 1,
            Self::Drink => 2,
        }
    }
}

/// How fed and watered the player is, as the server counts it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Nourishment {
    /// Food.
    pub food: u32,
    /// Drink.
    pub water: u32,
}

impl Nourishment {
    /// How much of one there is.
    #[must_use]
    pub const fn of(self, meal: Meal) -> u32 {
        match meal {
            Meal::Food => self.food,
            Meal::Drink => self.water,
        }
    }
}

fn word(body: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        body[offset],
        body[offset + 1],
        body[offset + 2],
        body[offset + 3],
    ])
}

/// Decodes `OP_Stamina`; other opcodes are not this codec's.
///
/// # Errors
/// Rejects a malformed length.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<Nourishment>> {
    if opcode != STAMINA_OPCODE {
        return Ok(None);
    }
    ensure!(body.len() == 8, "invalid stamina length");
    Ok(Some(Nourishment {
        food: word(body, 0),
        water: word(body, 4),
    }))
}

/// How fed and watered the Titanium profile says the player is.
///
/// # Errors
/// Rejects a profile too short to hold them.
pub fn titanium_profile(profile: &[u8]) -> Result<Nourishment> {
    ensure!(
        profile.len() >= 5008,
        "profile too short for food and drink"
    );
    Ok(Nourishment {
        food: word(profile, 5004),
        water: word(profile, 5000),
    })
}

/// The player eats or drinks the item in a slot (`Consume_Struct`): on the
/// client's own, or by hand.
#[must_use]
pub fn consume(slot: InventorySlot, meal: Meal, by_hand: bool) -> EncodedCommand {
    let mut body = Vec::with_capacity(16);
    body.extend_from_slice(&slot.0.to_le_bytes());
    body.extend_from_slice(&(if by_hand { BY_HAND } else { ON_ITS_OWN }).to_le_bytes());
    body.extend_from_slice(&[0; 4]);
    body.extend_from_slice(&[meal.wire(), 0, 0, 0]);
    EncodedCommand {
        opcode: CONSUME_OPCODE,
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_reports_food_and_drink_and_the_client_eats_by_slot() {
        let mut body = 2500u32.to_le_bytes().to_vec();
        body.extend_from_slice(&6000u32.to_le_bytes());
        assert_eq!(
            decode(STAMINA_OPCODE, &body).unwrap(),
            Some(Nourishment {
                food: 2500,
                water: 6000
            })
        );
        assert!(decode(STAMINA_OPCODE, &body[..7]).is_err());
        assert_eq!(decode(0x1234, &body).unwrap(), None);
        let mut profile = vec![0; 19592];
        profile[5000..5004].copy_from_slice(&4000u32.to_le_bytes());
        profile[5004..5008].copy_from_slice(&5000u32.to_le_bytes());
        assert_eq!(
            titanium_profile(&profile).unwrap(),
            Nourishment {
                food: 5000,
                water: 4000
            }
        );
        assert!(titanium_profile(&profile[..5007]).is_err());
        assert_eq!(
            consume(InventorySlot(251), Meal::Drink, false),
            EncodedCommand {
                opcode: CONSUME_OPCODE,
                body: vec![251, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, 2, 0, 0, 0],
            }
        );
        assert_eq!(
            consume(InventorySlot(22), Meal::Food, true).body[..8],
            [22, 0, 0, 0, 0xe7, 3, 0, 0]
        );
        assert_eq!(Meal::of_item_type(14), Some(Meal::Food));
        assert_eq!(Meal::of_item_type(15), Some(Meal::Drink));
        assert_eq!(Meal::of_item_type(11), None);
    }
}
