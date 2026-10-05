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
use crate::{command::EncodedCommand, inventory::InventorySlot, items::ItemDetails};
use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// `OP_Stamina`: how fed and watered the player is.
pub const STAMINA_OPCODE: u16 = 0x7a83;
/// `OP_Consume`: the player eats or drinks an item.
pub const CONSUME_OPCODE: u16 = 0x77d6;
/// At or below this much food or drink, servers count the player hungry or
/// thirsty, and the official client eats or drinks.
pub const HUNGRY: u32 = 3000;
/// TAKP's own: its client eats or drinks below 3000, so at 2999 or less
/// (inferred from TAKP's `Client::Hungry`, `zone/client.h`, whose comment
/// calls 3000 the auto-consume threshold; the client's own is unrecorded).
pub const TAKP_HUNGRY: u32 = 2999;
/// `OP_Stamina` on the `EQMac` wire (TAKP `patch_Mac.conf` 0x4157, its bytes
/// swapped).
pub const EQMAC_STAMINA_OPCODE: u16 = 0x5741;
/// `OP_Consume` on the `EQMac` wire (0x4156 swapped).
pub const EQMAC_CONSUME_OPCODE: u16 = 0x5641;
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

/// What a client eats and drinks on its own for a hungry or thirsty player.
/// Food and drink with modifiers can be worth more than a meal, so by
/// default it is left for the player to eat or drink by hand.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub enum AutoEat {
    /// Only food and drink without modifiers
    /// ([`ItemDetails::has_modifiers`]).
    #[default]
    Plain,
    /// The first food or drink carried, whatever it does, as the official
    /// client eats.
    Anything,
}

impl AutoEat {
    /// Whether a client eats or drinks this item on its own.
    #[must_use]
    pub fn takes(self, item: &ItemDetails) -> bool {
        self == Self::Anything || !item.has_modifiers()
    }
}

/// Why a hungry or thirsty player went without.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Shortage {
    /// They carry nothing to eat or drink.
    Nothing,
    /// They carry only food or drink with modifiers, which is theirs to eat
    /// or drink by hand ([`AutoEat::Plain`]).
    OnlyModified,
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

/// How fed and watered TAKP says the player is (`Stamina_Struct`, packed:
/// food and water as 16 bits each, from 0 to 32000, then a fatigue byte this
/// leaves out; `Client::SendStaminaUpdate`, `zone/client.cpp`). TAKP sends it
/// as food and water go down, as fatigue changes, and after every bite.
///
/// # Errors
/// Rejects a malformed length.
pub fn eqmac_nourishment(body: &[u8]) -> Result<Nourishment> {
    let [food_low, food_high, water_low, water_high, _fatigue] = *body else {
        anyhow::bail!("invalid EQMac stamina length");
    };
    let amount = |low, high| u32::try_from(i16::from_le_bytes([low, high]).max(0)).unwrap_or(0);
    Ok(Nourishment {
        food: amount(food_low, food_high),
        water: amount(water_low, water_high),
    })
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

/// The player eats or drinks the item in a slot on the `EQMac` wire
/// (`Consume_Struct`, which TAKP reads as it comes): the slot in `EQMac`'s
/// numbers, on the client's own or by hand, then -1, as its struct says the
/// official client sends there, then food or drink as 32 bits.
///
/// # Errors
/// Rejects a slot `EQMac` has no number for.
pub fn eqmac_consume(slot: InventorySlot, meal: Meal, by_hand: bool) -> Result<EncodedCommand> {
    let slot = slot
        .to_eqmac()
        .and_then(|slot| u32::try_from(slot).ok())
        .context("no EQMac inventory slot")?;
    let mut body = Vec::with_capacity(16);
    body.extend_from_slice(&slot.to_le_bytes());
    body.extend_from_slice(&(if by_hand { BY_HAND } else { ON_ITS_OWN }).to_le_bytes());
    body.extend_from_slice(&u32::MAX.to_le_bytes());
    body.extend_from_slice(&u32::from(meal.wire()).to_le_bytes());
    Ok(EncodedCommand {
        opcode: EQMAC_CONSUME_OPCODE,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takp_reports_food_and_drink_in_16_bits_and_takes_bites_in_its_numbers() {
        let mut body = 2500i16.to_le_bytes().to_vec();
        body.extend_from_slice(&32_000i16.to_le_bytes());
        body.push(40);
        assert_eq!(
            eqmac_nourishment(&body).unwrap(),
            Nourishment {
                food: 2500,
                water: 32_000
            }
        );
        // A negative count reads as none; other lengths are refused.
        body[..2].copy_from_slice(&(-5i16).to_le_bytes());
        assert_eq!(eqmac_nourishment(&body).unwrap().food, 0);
        assert!(eqmac_nourishment(&body[..4]).is_err());
        // A ration in the first place of the second pack's bag, by hand.
        let bite = eqmac_consume(InventorySlot(23).child(0).unwrap(), Meal::Food, true).unwrap();
        assert_eq!(bite.opcode, EQMAC_CONSUME_OPCODE);
        let words: Vec<u32> = bite
            .body
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| u32::from_le_bytes(*word))
            .collect();
        assert_eq!(words, [260, 999, u32::MAX, 1]);
        let sip = eqmac_consume(InventorySlot(22), Meal::Drink, false).unwrap();
        assert_eq!(&sip.body[4..8], &u32::MAX.to_le_bytes());
        assert_eq!(&sip.body[12..], &2u32.to_le_bytes());
        assert!(eqmac_consume(InventorySlot(0), Meal::Food, true).is_err());
    }

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
