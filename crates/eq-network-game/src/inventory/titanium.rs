//! Layout reference: `EQEmu`'s Titanium `SerializeItem` and `patch_Titanium.conf`.
use super::{InventoryItem, InventorySlot, InventoryUpdate, ItemPlacement};
use anyhow::{ensure, Context, Result};
use std::collections::BTreeSet;

const MAX_BYTES: usize = 1024 * 1024;
const MAX_ITEMS: usize = 1024;

/// Decodes inventory-bearing packets; merchant, loot and chat-link views are excluded.
///
/// # Errors
/// Rejects malformed framing, excessive sizes, duplicate slots and invalid item fields.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<InventoryUpdate>> {
    match opcode {
        0x5394 => Ok(Some(InventoryUpdate::Snapshot(parse(body)?))),
        0x3397 => {
            ensure!(body.len() >= 4, "truncated item packet");
            let kind = u32::from_le_bytes(body[..4].try_into()?);
            if !matches!(kind, 0x67 | 0x69 | 0x6a | 0x6b) {
                return Ok(None);
            }
            let mut items = parse(&body[4..])?;
            ensure!(!items.is_empty(), "empty item update");
            if kind == 0x6b {
                // What an opened world container holds: Titanium numbers its
                // ten places 0 to 9 here, and 4000 to 4009 in moves.
                let index = items[0].slot.0;
                ensure!((0..=9).contains(&index), "invalid world container place");
                items.truncate(1);
                items[0].slot = InventorySlot(4000 + index);
                return Ok(Some(InventoryUpdate::Set(items)));
            }
            // One root item and optional bag contents, not another bulk snapshot.
            let root = items[0].slot;
            ensure!(
                items
                    .iter()
                    .skip(1)
                    .all(|item| item.slot.parent().is_some_and(|(p, _)| p == root)),
                "multiple root items in item update"
            );
            if kind == 0x67
                && !matches!(root.0,0..=30|251..=340|2000..=2015|2031..=2190|2500..=2501|2531..=2550)
            {
                return Ok(None);
            }
            if kind == 0x6a {
                ensure!(root == InventorySlot(30), "summoned item is not on cursor");
                return Ok(Some(InventoryUpdate::Cursor(items)));
            }
            Ok(Some(InventoryUpdate::Set(items)))
        }
        // `OP_ClearObject`: the open world container emptied.
        0x21ed => {
            ensure!(body.len() == 8, "invalid world container clear length");
            Ok(Some(InventoryUpdate::WorldEmptied))
        }
        0x420f | 0x4d81 | 0x1c4a => {
            ensure!(body.len() == 12, "invalid inventory mutation length");
            let from = i32::from_le_bytes(body[..4].try_into()?);
            let to = u32::from_le_bytes(body[4..8].try_into()?);
            let quantity = u32::from_le_bytes(body[8..12].try_into()?);
            // MoveItem to the invalid destination removes the whole instance.
            // P99's scribe response uses zero; EQEmu also uses the all-ones sentinel.
            // Do not apply this interpretation to DeleteItem or DeleteCharge.
            if opcode == 0x420f && to == u32::MAX && matches!(quantity, 0 | u32::MAX) {
                Ok(Some(InventoryUpdate::Remove(InventorySlot(from))))
            } else if matches!(opcode, 0x4d81 | 0x1c4a) && to == u32::MAX && quantity == u32::MAX {
                Ok(Some(InventoryUpdate::Consume {
                    slot: InventorySlot(from),
                    charge: opcode == 0x1c4a,
                }))
            } else {
                // DeleteItem/DeleteCharge have per-charge and resync variants. Retain
                // the last known contents explicitly as stale instead of guessing counts.
                Ok(Some(InventoryUpdate::Invalidated))
            }
        }
        _ => Ok(None),
    }
}

/// Parses one serialized item as if it lay in `location`, so that the bag
/// contents of a view outside the inventory (a trade partner's slot) get
/// addresses.
pub(crate) fn parse_at(body: &[u8], location: InventorySlot) -> Result<Vec<InventoryItem>> {
    ensure!(body.len() <= MAX_BYTES, "inventory packet too large");
    let text = std::str::from_utf8(body).context("inventory is not UTF-8")?;
    let mut parser = Parser {
        text,
        items: Vec::new(),
    };
    parser.item(0, Some(location))?;
    ensure!(parser.text.is_empty(), "more than one item in a view");
    Ok(parser.items)
}

/// Parses serialized Titanium items, as used by inventory, loot and merchant views.
pub(crate) fn parse(body: &[u8]) -> Result<Vec<InventoryItem>> {
    ensure!(body.len() <= MAX_BYTES, "inventory packet too large");
    let text = std::str::from_utf8(body).context("inventory is not UTF-8")?;
    let mut parser = Parser {
        text,
        items: Vec::new(),
    };
    while !parser.text.is_empty() {
        parser.item(0, None)?;
    }
    let mut slots = BTreeSet::new();
    ensure!(
        parser.items.iter().all(|item| slots.insert(item.slot)),
        "duplicate inventory slot"
    );
    Ok(parser.items)
}

struct Parser<'a> {
    text: &'a str,
    items: Vec<InventoryItem>,
}
impl<'a> Parser<'a> {
    fn take(&mut self, token: &str) -> Result<()> {
        self.text = self
            .text
            .strip_prefix(token)
            .context("invalid item framing")?;
        Ok(())
    }
    fn field(&mut self) -> Result<&'a str> {
        let (value, rest) = self.text.split_once('|').context("truncated item header")?;
        self.text = rest;
        Ok(value)
    }

    fn item(&mut self, depth: usize, location: Option<InventorySlot>) -> Result<()> {
        ensure!(
            depth <= 1 && self.items.len() < MAX_ITEMS,
            "inventory nesting or item limit exceeded"
        );
        let wrapper = format!("{}\"", "\\".repeat(depth.saturating_sub(1)));
        let quote = format!("{}\"", "\\".repeat(depth));
        if depth > 0 {
            self.take(&wrapper)?;
        }
        let mut header = Vec::with_capacity(11);
        for _ in 0..11 {
            header.push(self.field()?);
        }
        self.take(&quote)?;
        let end = self
            .text
            .find(&quote)
            .context("unterminated item definition")?;
        let fields: Vec<_> = self.text[..end].split('|').collect();
        self.text = &self.text[end..];
        self.take(&quote)?;
        let details = crate::items::definition(&fields)?;
        let number =
            |i: usize| -> Result<u32> { fields[i].parse().context("invalid inventory number") };
        let bag_slots = u8::try_from(number(97)?).context("invalid bag size")?;
        ensure!(bag_slots <= 10, "too many bag slots");
        let serialized_slot: i32 = header[2].parse().context("invalid inventory slot")?;
        let slot = location.unwrap_or(InventorySlot(serialized_slot));
        self.items.push(InventoryItem {
            activation: super::ItemActivation::decode(&fields, header[7])?,
            scroll_spell: {
                let id = fields[154].parse::<i64>().context("invalid scroll spell")?;
                u32::try_from(id)
                    .ok()
                    .filter(|id| !matches!(*id, 0 | 0xffff | u32::MAX))
            },
            book: crate::books::Book::from_item(fields[0], fields[100], fields[102]),
            rules: ItemPlacement {
                stack_size: number(131)?,
                size: u8::try_from(number(8)?)?,
                bag_size: u8::try_from(number(98)?)?,
                bag_type: u8::try_from(number(96)?)?,
                item_type: u8::try_from(number(56)?)?,
                deity_mask: number(31)?,
            },
            slot,
            details,
            stack_count: match number(133)? {
                0 => None,
                1 => Some(header[0].parse().context("invalid stack quantity")?),
                _ => anyhow::bail!("invalid stackable flag"),
            },
            charges: header[8].parse().context("invalid item charges")?,
            bag_slots,
        });
        for index in 0..10 {
            self.take("|")?;
            if self.text.starts_with(&quote) {
                ensure!(index < bag_slots, "item outside container capacity");
                let child = slot.child(index).context("unsupported container address")?;
                self.item(depth + 1, Some(child))?;
            }
        }
        if depth > 0 {
            self.take(&wrapper)?;
        } else {
            self.take("\0")?;
        }
        Ok(())
    }
}
