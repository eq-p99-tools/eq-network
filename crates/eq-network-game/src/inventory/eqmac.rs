//! The inventory as TAKP sends it on the `EQMac` wire: items as fixed
//! 360-byte records where Titanium's servers send text, the full inventory
//! compressed behind a count, and slots numbered `EQMac`'s way, which
//! [`InventorySlot::from_eqmac`] turns into the one vocabulary.
//!
//! Layout reference: TAKP (`EQMacEmu/Server` 047d0f8)
//! `common/patches/mac_structs.h` `Item_Struct` and
//! `PlayerItemsPacket_Struct`, filled by `MacItem` in
//! `common/patches/mac.cpp`, whose `ENCODE(OP_ItemPacket)` and
//! `ENCODE(OP_CharInventory)` choose the opcodes; the opcodes are
//! `utils/patches/patch_Mac.conf`'s, their bytes swapped. In
//! `zone/inventory.cpp`, `PutItemInInventory` and `SwapItemResync` send an
//! item in a slot, `PushItemOnCursor` and `SaveCursor` an item onto the
//! cursor, and `DeleteItemInInventory` an emptied slot or a unit or charge
//! used up.
use super::{
    ClickEffect, ClickKind, InventoryItem, InventorySlot, InventoryUpdate, ItemActivation,
    ItemPlacement, MoveQuantity,
};
use crate::command::EncodedCommand;
use crate::items::{EquipmentRules, ItemBonuses, ItemDetails, ItemStat, WornEffect};
use anyhow::{anyhow, bail, ensure, Context, Result};
use std::collections::BTreeMap;

/// `OP_CharInventory`: everything the player holds, as the zone admits them.
const INVENTORY_OPCODE: u16 = 0xf641;
/// `OP_MerchantItemPacket`: an item now in a slot. TAKP sends it for every
/// item the server puts in place (its `ItemPacketTrade`), not only for
/// purchases.
const PLACED_OPCODE: u16 = 0x3140;
/// `OP_ItemPacket`, `OP_BookPacket` and `OP_ContainerPacket`: an item in a
/// slot, by its class. TAKP sends them only for item packets of kinds it
/// names no other opcode for (inferred unused), and tags each record of the
/// full inventory with them.
const ITEM_OPCODE: u16 = 0x6441;
const BOOK_OPCODE: u16 = 0x6541;
const CONTAINER_OPCODE: u16 = 0x6641;
/// `OP_SummonedItem`: an item onto the cursor.
const SUMMONED_OPCODE: u16 = 0x7841;
/// `OP_MoveItem`, which from the server only empties a slot.
const MOVE_OPCODE: u16 = 0x2c41;
/// `OP_DeleteCharge`: a unit or charge of the item in a slot used up.
const DELETE_CHARGE_OPCODE: u16 = 0x4741;

/// An item record.
const RECORD: usize = 360;
/// A record of the full inventory: its tag, then the item.
const TAGGED: usize = 2 + RECORD;
/// The most items a full inventory may hold.
const MAX_ITEMS: usize = 1024;
/// How many units make a full stack: `EQMac` items carry no stack size
/// (`common/item_data.h` `EQMAC_STACKSIZE`).
const STACK: u32 = 20;
/// The item types that stack, given charges (`common/item_data.cpp`
/// `IsStackable`): food, drink, combinables, bandages, small throwing
/// weapons, arrows, type 28, fishing bait and alcohol.
const STACKING_TYPES: [u8; 9] = [14, 15, 17, 18, 19, 27, 28, 37, 38];

/// What TAKP's item packet says about the inventory: the full inventory, an
/// item in a slot or onto the cursor, an emptied slot, or a unit or charge
/// used up. Merchant, loot, trade, world container and linked item views
/// are left out.
///
/// # Errors
/// Rejects malformed records and compression, slots `EQMac` does not
/// number, bag contents outside their bag, and a summoned item anywhere but
/// the cursor.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<InventoryUpdate>> {
    match opcode {
        INVENTORY_OPCODE => Ok(Some(InventoryUpdate::Snapshot(inventory(body)?))),
        PLACED_OPCODE | ITEM_OPCODE | BOOK_OPCODE | CONTAINER_OPCODE => {
            let item = single(body)?;
            Ok(item
                .slot
                .is_held()
                .then(|| InventoryUpdate::Set(vec![item])))
        }
        SUMMONED_OPCODE => {
            let item = single(body)?;
            ensure!(
                item.slot == InventorySlot::CURSOR,
                "summoned item is not on cursor"
            );
            Ok(Some(InventoryUpdate::Cursor(vec![item])))
        }
        MOVE_OPCODE | DELETE_CHARGE_OPCODE => {
            let ([from, to, quantity], []) = body.as_chunks::<4>() else {
                bail!("invalid EQMac inventory mutation length");
            };
            // From the server, both name a slot and leave the rest all
            // ones; anything else leaves the inventory unknown.
            if [to, quantity].iter().any(|word| **word != [0xff; 4]) {
                return Ok(Some(InventoryUpdate::Invalidated));
            }
            let slot = InventorySlot::from_eqmac(i32::from_le_bytes(*from))
                .context("unknown EQMac inventory slot")?;
            Ok(Some(if opcode == MOVE_OPCODE {
                InventoryUpdate::Remove(slot)
            } else {
                InventoryUpdate::Used(slot)
            }))
        }
        _ => Ok(None),
    }
}

/// `EQMac`'s `OP_MoveItem` for a move already planned (`MoveItem_Struct`,
/// `common/patches/mac_structs.h`): both slots in `EQMac`'s numbers, then
/// how many of a stack go, zero for a whole item. TAKP merges onto the same
/// item only with a count, and swaps whole items (`Client::SwapItem`,
/// `zone/inventory.cpp`).
///
/// # Errors
/// Rejects a slot `EQMac` has no number for.
pub fn move_item(
    from: InventorySlot,
    to: InventorySlot,
    quantity: MoveQuantity,
) -> Result<EncodedCommand> {
    let number = |slot: InventorySlot| {
        slot.to_eqmac()
            .and_then(|slot| u32::try_from(slot).ok())
            .context("no EQMac inventory slot")
    };
    let count = match quantity {
        MoveQuantity::Whole => 0,
        MoveQuantity::Count(count) => count.get(),
    };
    let mut body = Vec::with_capacity(12);
    for value in [number(from)?, number(to)?, count] {
        body.extend_from_slice(&value.to_le_bytes());
    }
    Ok(EncodedCommand {
        opcode: MOVE_OPCODE,
        body,
    })
}

/// The full inventory. Its first byte counts what TAKP meant to send,
/// including the items it leaves out (IDs above 32767) and wrapping past
/// 255, so the records say how many there are: tagged records, compressed
/// with zlib. An empty inventory comes as two zero bytes, uncompressed. Bag
/// contents are records of their own, at their own slots.
fn inventory(body: &[u8]) -> Result<Vec<InventoryItem>> {
    if body == [0, 0] {
        return Ok(Vec::new());
    }
    ensure!(body.len() > 2, "truncated EQMac inventory");
    let data =
        miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&body[2..], MAX_ITEMS * TAGGED)
            .map_err(|_| anyhow!("invalid or oversized EQMac inventory"))?;
    let (records, rest) = data.as_chunks::<TAGGED>();
    ensure!(rest.is_empty(), "partial EQMac inventory record");
    let items = records
        .iter()
        .map(|tagged| {
            let record = Record(&tagged[2..]);
            // Each record is tagged with the packet its class would come
            // as, which reads as opcodes do.
            ensure!(
                u16::from_be_bytes([tagged[0], tagged[1]]) == record.class()?.opcode(),
                "EQMac inventory record tagged for another class"
            );
            item(&record)
        })
        .collect::<Result<Vec<_>>>()?;
    let bags: BTreeMap<_, _> = items
        .iter()
        .map(|item| (item.slot, item.bag_slots))
        .collect();
    ensure!(bags.len() == items.len(), "duplicate inventory slot");
    ensure!(
        items.iter().all(|item| item
            .slot
            .parent()
            .is_none_or(|(bag, index)| bags.get(&bag).is_some_and(|slots| index < *slots))),
        "item outside container capacity"
    );
    Ok(items)
}

/// One item record, as a packet of its own carries it.
fn single(body: &[u8]) -> Result<InventoryItem> {
    ensure!(body.len() == RECORD, "invalid EQMac item length");
    item(&Record(body))
}

/// An item record as an item in the slot it names.
fn item(record: &Record<'_>) -> Result<InventoryItem> {
    let class = record.class()?;
    let details = definition(record)?;
    let slot = InventorySlot::from_eqmac(i32::from(record.short(184)))
        .context("unknown EQMac inventory slot")?;
    let common = class == Class::Common;
    let container = class == Class::Container;
    let item_type = if common { record.byte(253) } else { 0 };
    let maximum_charges = if common {
        i32::from(record.signed_byte(246))
    } else {
        0
    };
    let stackable = common && maximum_charges > 0 && STACKING_TYPES.contains(&item_type);
    // A stack's quantity where anything else keeps its charges (-1 for
    // unlimited).
    let charges = i32::from(record.signed_byte(278));
    let bag_slots = if container { record.byte(269) } else { 0 };
    ensure!(bag_slots <= 10, "too many bag slots");
    let effect = Effect::of(record);
    let click = match effect.spell(&[1, 3, 4, 5]) {
        Some(spell_id) => Some(ClickEffect {
            spell_id: u32::from(spell_id),
            kind: ClickKind::of(u8::try_from(effect.kind)?),
            // TAKP sends one level for an effect, its Level, or its Level2
            // where that is zero, so the level a click needs is not known.
            required_level: 0,
            effect_level: effect.level,
            cast_time_ms: u32::try_from(record.signed_word(292))
                .context("invalid EQMac cast time")?,
            // `MacItem` leaves the recast time out.
            recast_delay_seconds: 0,
            recast_type: 0,
        }),
        None => None,
    };
    Ok(InventoryItem {
        activation: ItemActivation {
            maximum_charges,
            effect: click,
            recast_timestamp: 0,
        },
        scroll_spell: effect.spell(&[7]).map(u32::from),
        book: crate::books::Book::from_item(
            &record.short(178).to_string(),
            if record.signed_byte(230) == 0 {
                "0"
            } else {
                "1"
            },
            &record.text(231, 30),
        ),
        rules: ItemPlacement {
            stack_size: if stackable { STACK } else { 1 },
            size: record.byte(177),
            bag_size: if container { record.byte(271) } else { 0 },
            bag_type: if container { record.byte(268) } else { 0 },
            item_type,
            deity_mask: record.word(348),
        },
        slot,
        details,
        stack_count: if stackable {
            Some(u32::try_from(charges).context("invalid EQMac stack quantity")?)
        } else {
            None
        },
        charges: if stackable { 0 } else { charges },
        bag_slots,
    })
}

/// What an item record says about the item itself, for inspection and for
/// what it adds when worn. Only common items carry statistics and class
/// and race restrictions: bags and books restrict no class or race.
fn definition(record: &Record<'_>) -> Result<ItemDetails> {
    let middle = Middle {
        record,
        common: record.class()? == Class::Common,
    };
    let id = u32::try_from(record.short(180))
        .ok()
        .filter(|id| *id != 0)
        .context("missing EQMac item ID")?;
    let name = record.text(0, 64);
    ensure!(!name.is_empty(), "missing EQMac item name");
    let lore = record.text(64, 80);
    let effect = Effect::of(record);
    Ok(ItemDetails {
        equipment: Some(EquipmentRules {
            required_level: u32::try_from(record.short(352))
                .context("invalid EQMac required level")?,
            recommended_level: u32::from(record.byte(324)),
            // TAKP applies worn effects for common items alone
            // (`Client::AddItemBonuses`).
            worn: effect
                .spell(&[2])
                .filter(|_| middle.common)
                .map(|spell_id| WornEffect {
                    spell_id: u32::from(spell_id),
                    effect_type: 2,
                    level: u32::from(effect.level),
                    level2: u32::from(effect.level),
                }),
        }),
        bonuses: Some(ItemBonuses {
            strength: middle.signed(228),
            stamina: middle.signed(229),
            agility: middle.signed(233),
            dexterity: middle.signed(231),
            charisma: middle.signed(230),
            intelligence: middle.signed(232),
            wisdom: middle.signed(234),
            hit_points: middle.short(240),
            mana: middle.short(242),
            // `EQMac` has no endurance.
            endurance: 0,
        }),
        id,
        name,
        flags: flags(&middle, &lore),
        lore,
        weight_tenths: u32::from(record.byte(174)),
        slots: record.word(188),
        classes: if middle.common {
            record.word(268)
        } else {
            u32::MAX
        },
        races: if middle.common {
            record.word(272)
        } else {
            u32::MAX
        },
        stats: statistics(&middle, &effect),
        price: Some(u32::try_from(record.signed_word(192)).context("invalid EQMac item price")?),
        icon: Some(u32::from(record.unsigned_short(182))),
    })
}

/// The item's flags, in Titanium's order.
fn flags(middle: &Middle<'_, '_>, lore: &str) -> Vec<String> {
    let record = middle.record;
    [
        (record.byte(175) == 0, "NO RENT"),
        (record.byte(176) == 0, "NO DROP"),
        (middle.common && record.byte(254) != 0, "MAGIC"),
        (lore.starts_with('*'), "LORE"),
    ]
    .into_iter()
    .filter(|(set, _)| *set)
    .map(|(_, flag)| flag.to_owned())
    .collect()
}

/// The item's statistics that are not zero, labelled and ordered as
/// Titanium's are.
fn statistics(middle: &Middle<'_, '_>, effect: &Effect) -> Vec<ItemStat> {
    let record = middle.record;
    let spell = |kinds: &[i8]| effect.spell(kinds).map_or(0, i32::from);
    [
        ("Cold", middle.signed(237)),
        ("Disease", middle.signed(238)),
        ("Poison", middle.signed(239)),
        ("Magic resist", middle.signed(235)),
        ("Fire", middle.signed(236)),
        ("STR", middle.signed(228)),
        ("STA", middle.signed(229)),
        ("AGI", middle.signed(233)),
        ("DEX", middle.signed(231)),
        ("CHA", middle.signed(230)),
        ("INT", middle.signed(232)),
        ("WIS", middle.signed(234)),
        ("HP", middle.short(240)),
        ("Mana", middle.short(242)),
        ("AC", middle.short(244)),
        ("Required level", i32::from(record.short(352))),
        ("Delay", middle.unsigned(249)),
        ("Recommended level", i32::from(record.byte(324))),
        ("Range", middle.unsigned(252)),
        ("Damage", middle.unsigned(250)),
        ("Click spell", spell(&[1, 3, 4, 5])),
        ("Proc spell", spell(&[0])),
        ("Worn spell", spell(&[2])),
        ("Focus spell", i32::from(record.short(358)).max(0)),
        ("Scroll spell", spell(&[7])),
    ]
    .into_iter()
    .filter(|(_, value)| *value != 0)
    .map(|(label, value)| ItemStat {
        label: label.into(),
        value,
    })
    .collect()
}

/// The middle of a record, which holds a common item's statistics and
/// restrictions; a bag's or a book's holds their own fields there, read as
/// none of those.
struct Middle<'r, 'a> {
    record: &'r Record<'a>,
    common: bool,
}

impl Middle<'_, '_> {
    fn signed(&self, at: usize) -> i32 {
        if self.common {
            i32::from(self.record.signed_byte(at))
        } else {
            0
        }
    }
    fn unsigned(&self, at: usize) -> i32 {
        if self.common {
            i32::from(self.record.byte(at))
        } else {
            0
        }
    }
    fn short(&self, at: usize) -> i32 {
        if self.common {
            i32::from(self.record.short(at))
        } else {
            0
        }
    }
}

/// An item's class, which decides what the middle of its record holds:
/// statistics, a bag's capacity, or a book's text name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Class {
    Common,
    Container,
    Book,
}

impl Class {
    /// The packet an item of this class comes as, which also tags its
    /// record in the full inventory.
    const fn opcode(self) -> u16 {
        match self {
            Self::Common => ITEM_OPCODE,
            Self::Container => CONTAINER_OPCODE,
            Self::Book => BOOK_OPCODE,
        }
    }
}

/// The one spell effect a record carries. `MacItem` sends an item's click
/// effect, else its scroll's, its proc's or its worn one, with the type
/// that says which; the copy past the record's middle holds it for every
/// class.
struct Effect {
    /// The spell, zero for none.
    spell: u16,
    /// The effect's type: 0 a proc, 1, 3, 4 and 5 clicks, 2 worn, 7 a
    /// scroll's.
    kind: i8,
    /// The effect's level.
    level: u8,
}

impl Effect {
    fn of(record: &Record<'_>) -> Self {
        Self {
            spell: record.unsigned_short(280),
            kind: record.signed_byte(279),
            level: record.byte(277),
        }
    }

    /// The spell, when the effect is one of these types.
    fn spell(&self, kinds: &[i8]) -> Option<u16> {
        (self.spell != 0 && kinds.contains(&self.kind)).then_some(self.spell)
    }
}

/// One item record, read at `Item_Struct`'s offsets.
struct Record<'a>(&'a [u8]);

impl Record<'_> {
    fn byte(&self, at: usize) -> u8 {
        self.0[at]
    }
    fn signed_byte(&self, at: usize) -> i8 {
        i8::from_le_bytes([self.0[at]])
    }
    fn short(&self, at: usize) -> i16 {
        i16::from_le_bytes([self.0[at], self.0[at + 1]])
    }
    fn unsigned_short(&self, at: usize) -> u16 {
        u16::from_le_bytes([self.0[at], self.0[at + 1]])
    }
    fn word(&self, at: usize) -> u32 {
        u32::from_le_bytes(
            self.0[at..at + 4]
                .try_into()
                .expect("record length checked"),
        )
    }
    fn signed_word(&self, at: usize) -> i32 {
        i32::from_le_bytes(
            self.0[at..at + 4]
                .try_into()
                .expect("record length checked"),
        )
    }
    /// A text field, up to its first NUL.
    fn text(&self, at: usize, length: usize) -> String {
        let field = &self.0[at..at + length];
        let end = field.iter().position(|b| *b == 0).unwrap_or(length);
        String::from_utf8_lossy(&field[..end]).into_owned()
    }
    fn class(&self) -> Result<Class> {
        Ok(match self.short(178) {
            0 => Class::Common,
            1 => Class::Container,
            2 => Class::Book,
            other => bail!("unknown EQMac item class {other}"),
        })
    }
}

mod hit_points;
pub use hit_points::{takp_item_hit_points, ItemHitPointCount, UnsettledItem, Wearer};

#[cfg(test)]
mod tests;
