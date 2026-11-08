//! Titanium item tags from serialized inventory records. The tag uses
//! different hash inputs for bags, books, charms, and ordinary items.
use crate::{chat::ItemLink, inventory::InventoryItem};
use anyhow::{ensure, Context, Result};
use std::fmt::Write;

pub(crate) fn hash(header: &[&str], fields: &[&str], empty_children: bool) -> Option<u32> {
    // The legacy header ends at the evolution flag for ordinary records.
    // An evolving record needs additional instance fields.
    if header.get(10) != Some(&"0") {
        return None;
    }
    let name = *fields.get(1)?;
    // The native hash uppercases text in its client locale. ASCII has the
    // same byte and character representation throughout that calculation.
    if name.is_empty() || name.len() >= 64 || !name.bytes().all(|b| matches!(b, b' '..=b'~')) {
        return None;
    }

    // Field order follows EQEmu's Titanium SerializeItem. Use the values
    // received by the client, including the static price and signed stats.
    let number = |index: usize| fields.get(index)?.parse::<i32>().ok();
    let byte = |index| u8::try_from(number(index)?).ok().map(i32::from);
    let item_class = byte(0)?;
    let id = number(4)?;
    let weight = byte(5)?;
    let price = number(10)?;
    let mut input = format!("{id}{name}");
    match item_class {
        1 => {
            let capacity = byte(97)?;
            let reduction = byte(99)?;
            write!(input, "{capacity:x}{reduction}{price:09X}{weight}").ok()?;
        }
        2 => {
            let book_type = i8::try_from(number(101)?).ok()?;
            write!(input, "{weight}{book_type}{price:09X}").ok()?;
        }
        0 if empty_children => {
            // The serialized child records carry installed augments. The
            // static socket types only describe which augments could fit.
            input.push_str("-1-1-1-1-1");
            // EQEmu writes Light as a signed byte. The link hash reads its
            // stored bits as an unsigned byte.
            let light = match number(43)? {
                value @ -128..=-1 => i8::try_from(value).ok()?.to_ne_bytes()[0],
                value => u8::try_from(value).ok()?,
            };
            let icon = number(11)?;
            let size = byte(8)?;
            let item_type = byte(56)?;
            let favor = number(109)?;
            let guild_favor = number(121)?;
            if number(73)? == 0 {
                let mana = number(29)?;
                let hp = number(28)?;
                let required_level = number(40)?.max(0);
                let ac = number(30)?;
                write!(input,
                    "{mana} {hp} {favor} {light} {icon} {price} {weight} {required_level} {size} {item_class} {item_type} {ac} {guild_favor}"
                ).ok()?;
            } else {
                write!(input,
                    "{light} {icon} {price} {size} {weight} {item_class} {item_type} {favor} {guild_favor}"
                ).ok()?;
            }
        }
        _ => return None,
    }
    Some(input.bytes().fold(0_u32, |hash, byte| {
        hash.wrapping_mul(31)
            .wrapping_add(u32::from(byte.to_ascii_uppercase()))
    }))
}

pub(crate) fn link(item: &InventoryItem) -> Result<ItemLink> {
    let hash = item
        .rules
        .titanium_link_hash
        .context("This item lacks the metadata needed to make a link")?;
    let item_id = item.details.id;
    ensure!(
        (1..0xfffff).contains(&item_id),
        "That item ID cannot be linked"
    );
    // Ordinary tags have no augment IDs and no evolution fields.
    let body = format!("0{item_id:05X}{:025}{:06}{hash:08X}", 0, 0);
    let label = &item.details.name;
    Ok(ItemLink {
        start: 0,
        end: body.len() + label.len() + 2,
        text_start: 0,
        text_end: label.len(),
        body,
        text: label.clone(),
        item_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{Inventory, InventorySlot, InventoryUpdate};

    fn record(class: &str, charm: &str) -> Vec<&'static str> {
        let mut fields = vec!["0"; 159];
        for (index, value) in [
            (1, "Test Item"),
            (4, "291"),
            (5, "7"),
            (8, "3"),
            (10, "164"),
            (11, "501"),
            (28, "41"),
            (29, "-22"),
            (30, "-5"),
            (40, "12"),
            (43, "2"),
            (56, "10"),
            (97, "10"),
            (99, "65"),
            (101, "-3"),
            (109, "88"),
            (121, "73"),
        ] {
            fields[index] = value;
        }
        if class == "1" {
            fields[0] = "1";
        } else {
            fields[97] = "0";
        }
        if class == "2" {
            fields[0] = "2";
        }
        if charm == "1" {
            fields[73] = "1";
        }
        fields
    }

    fn inventory(class: &str, charm: &str) -> Inventory {
        let fields = record(class, charm);
        let wire = wire(&fields, &"|".repeat(10));
        let update = crate::inventory::decode(0x5394, wire.as_bytes())
            .unwrap()
            .unwrap();
        let mut inventory = Inventory::default();
        inventory.apply(update);
        inventory
    }

    fn wire(fields: &[&str], children: &str) -> String {
        format!(
            "0|0|23|999|1|0|45|0|0|0|0|\"{}\"{children}\0",
            fields.join("|"),
        )
    }

    #[test]
    fn four_native_hash_families_use_owned_serialized_fields() {
        for (class, charm, expected) in [
            ("0", "0", 0xb007_0342_u32),
            ("0", "1", 0x8fcb_a275),
            ("1", "0", 0x54dc_bc1d),
            ("2", "0", 0xc2bb_8e9d),
        ] {
            let link = inventory(class, charm).items()[&InventorySlot(23)]
                .titanium_link()
                .unwrap();
            assert_eq!(link.body.len(), 45);
            assert_eq!(&link.body[..6], "000123");
            assert_eq!(&link.body[37..], format!("{expected:08X}"));
            assert_eq!(link.text, "Test Item");
            assert!(crate::items::chat_link(crate::GameDialect::Titanium, &link).is_ok());
        }
    }

    #[test]
    fn incomplete_or_unsupported_inputs_have_no_hash() {
        let header = ["0"; 11];
        let mut fields = record("0", "0");
        assert_eq!(hash(&header[..10], &fields, true), None);
        let mut evolving = header;
        evolving[10] = "1";
        assert_eq!(hash(&evolving, &fields, true), None);
        assert_eq!(hash(&header, &fields, false), None);
        fields[1] = "Épée";
        assert_eq!(hash(&header, &fields, true), None);
    }

    #[test]
    fn static_sockets_can_have_an_empty_instance() {
        let mut fields = record("0", "0");
        for index in [84, 86, 88, 90, 92] {
            fields[index] = "1";
        }
        let empty = wire(&fields, &"|".repeat(10));
        let item_packet = [0x69_u32.to_le_bytes().as_slice(), empty.as_bytes()].concat();
        let mut inventory = Inventory::default();
        for (opcode, body) in [(0x5394, empty.as_bytes()), (0x3397, item_packet.as_slice())] {
            inventory.apply(crate::inventory::decode(opcode, body).unwrap().unwrap());
            let link = inventory.items()[&InventorySlot(23)]
                .titanium_link()
                .unwrap();
            assert_eq!(&link.body[6..37], "0".repeat(31));
            assert_eq!(&link.body[37..], "B0070342");
        }

        let malformed = wire(&fields, &"|".repeat(9));
        assert!(crate::inventory::decode(0x5394, malformed.as_bytes()).is_err());

        let mut child = record("0", "0");
        child[4] = "292";
        let children = format!(
            "|\"0|0|0|0|1|0|46|0|0|0|0|\\\"{}\\\"{}\"{}",
            child.join("|"),
            "|".repeat(10),
            "|".repeat(9)
        );
        let augmented = wire(&fields, &children);
        assert!(crate::inventory::decode(0x5394, augmented.as_bytes()).is_err());

        // A nonempty common record still cannot produce an empty-augment
        // hash even when its static capacity allows this child to be parsed.
        fields[97] = "1";
        let nonempty = wire(&fields, &children);
        let update = crate::inventory::decode(0x5394, nonempty.as_bytes())
            .unwrap()
            .unwrap();
        inventory.apply(update);
        assert!(inventory.items()[&InventorySlot(23)]
            .titanium_link()
            .is_err());
    }

    #[test]
    fn signed_light_keeps_its_unsigned_native_bits() {
        let header = ["0"; 11];
        let mut fields = record("0", "0");
        fields[43] = "255";
        assert_eq!(hash(&header, &fields, true), Some(0x65d7_8022));
        fields[43] = "-1";
        assert_eq!(hash(&header, &fields, true), Some(0x65d7_8022));
        for invalid in ["-129", "256"] {
            fields[43] = invalid;
            assert_eq!(hash(&header, &fields, true), None);
        }
    }

    #[test]
    fn link_metadata_is_independent_of_placement_and_custody() {
        let mut inventory = inventory("0", "0");
        let mut item = inventory.items()[&InventorySlot(23)].clone();
        let body = item.titanium_link().unwrap().body;
        item.slot = InventorySlot(4000);
        assert_eq!(item.titanium_link().unwrap().body, body);

        item.slot = InventorySlot(24);
        inventory.apply(InventoryUpdate::Prediction(vec![item]));
        assert!(inventory.predicted());
        assert!(!inventory.custody_known(InventorySlot(24)));
        assert_eq!(
            inventory.items()[&InventorySlot(24)]
                .titanium_link()
                .unwrap()
                .body,
            body
        );
        inventory.apply(InventoryUpdate::Settled);
        assert!(!inventory.custody_known(InventorySlot(24)));
        assert_eq!(
            inventory.items()[&InventorySlot(24)]
                .titanium_link()
                .unwrap()
                .body,
            body
        );
    }
}
