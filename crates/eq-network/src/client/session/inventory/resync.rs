//! `EQMac` rejection recovery is a resync followed by a disconnect, not a move ACK.
//! Both `EQMac` servers do it the same way: Quarm (`SecretsOTheP` c2cd265,
//! `docs/quarm-inventory-parity.md`) and TAKP (`EQMacEmu/Server` e517d0b,
//! `zone/inventory.cpp` `Client::SwapItemResync`, line 1561), so
//! `MoveRules::Takp` and `MoveRules::Quarm` share this module.
use eq_network_game::inventory::{Inventory, InventoryUpdate};

/// Whether a server replacement contradicts any locally predicted slot it
/// covers. A resync's root-only bag packet cannot certify its contents.
pub(super) fn contradicts(inventory: &Inventory, update: &InventoryUpdate) -> bool {
    let (root, items) = match update {
        InventoryUpdate::Set(items) => {
            let Some(item) = items.first() else {
                return false;
            };
            (item.slot, items.as_slice())
        }
        InventoryUpdate::Remove(slot) => (*slot, &[][..]),
        _ => return false,
    };
    inventory.prediction_origins().any(|(slot, _)| {
        (slot == root || slot.parent().is_some_and(|(parent, _)| parent == root))
            && inventory.items().get(&slot) != items.iter().find(|item| item.slot == slot)
    })
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use eq_network_game::{
        food::AutoEat,
        inventory::MoveQuantity,
        money::{Coin, CoinPlace, CoinTransfer},
    };

    #[test]
    fn item_coin_merchant_and_meal_permissions_are_independent() {
        let commands = [
            ClientCommand::MoveInventory(InventoryMove {
                session_id: 5,
                revision: 0,
                from: InventorySlot(22),
                to: InventorySlot::CURSOR,
                quantity: MoveQuantity::Whole,
                created: Instant::now(),
            }),
            ClientCommand::MoveCoins {
                session_id: 5,
                transfer: CoinTransfer {
                    from: CoinPlace::Purse,
                    to: CoinPlace::Cursor,
                    coin: Coin::Gold,
                    into: Coin::Gold,
                    amount: 1,
                },
                created: Instant::now(),
            },
            ClientCommand::Shop {
                session_id: 5,
                merchant_id: 9,
                own_id: 7,
                open: true,
                created: Instant::now(),
            },
            ClientCommand::Consume {
                session_id: 5,
                slot: InventorySlot(22),
                created: Instant::now(),
            },
        ];
        for bits in 0..16 {
            let allows = Allowances {
                items: bits & 1 != 0,
                coins: bits & 2 != 0,
                merchants: bits & 4 != 0,
                meals: bits & 8 != 0,
            };
            let belongings = Belongings::under(
                MoveRules::Quarm,
                Quotes::of(eq_network_game::GameDialect::EqMac),
                allows,
                AutoEat::Anything,
            );
            for (bit, command) in commands.iter().enumerate() {
                assert_eq!(belongings.owns(command), bits & (1 << bit) != 0);
            }
        }
    }
}
