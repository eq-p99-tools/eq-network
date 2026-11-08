//! Titanium item tags from serialized inventory records. The tag uses
//! different hash inputs for bags, books, charms, and ordinary items.
use crate::{chat::ItemLink, inventory::InventoryItem};
use anyhow::{ensure, Context, Result};

pub(crate) fn hash(header: &[&str], fields: &[&str]) -> Option<u32> {
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
            input.push_str(&format!("{capacity:x}{reduction}{price:09X}{weight}"));
        }
        2 => {
            let book_type = i8::try_from(number(101)?).ok()?;
            input.push_str(&format!("{weight}{book_type}{price:09X}"));
        }
        0 => {
            // This inventory projection does not model augment custody.
            // An item with any augment socket needs an instance provider.
            for index in [84, 86, 88, 90, 92] {
                if number(index)? != 0 {
                    return None;
                }
            }
            input.push_str("-1-1-1-1-1");
            let light = byte(43)?;
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
                input.push_str(&format!(
                    "{mana} {hp} {favor} {light} {icon} {price} {weight} {required_level} {size} {item_class} {item_type} {ac} {guild_favor}"
                ));
            } else {
                input.push_str(&format!(
                    "{light} {icon} {price} {size} {weight} {item_class} {item_type} {favor} {guild_favor}"
                ));
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
        let wire = format!(
            "0|0|23|999|1|0|45|0|0|0|0|\"{}\"{}\0",
            fields.join("|"),
            "|".repeat(10)
        );
        let update = crate::inventory::decode(0x5394, wire.as_bytes())
            .unwrap()
            .unwrap();
        let mut inventory = Inventory::default();
        inventory.apply(update);
        inventory
    }

    #[test]
    fn four_native_hash_families_use_owned_serialized_fields() {
        for (class, charm, expected) in [
            ("0", "0", 0xb007_0342_u32),
            ("0", "1", 0x8fcb_a275),
            ("1", "0", 0x54dc_bc1d),
            ("2", "0", 0xc2bb_8e9d),
        ] {
            let link = inventory(class, charm)
                .owned_titanium_link(InventorySlot(23))
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
        assert_eq!(hash(&header[..10], &fields), None);
        let mut evolving = header;
        evolving[10] = "1";
        assert_eq!(hash(&evolving, &fields), None);
        fields[84] = "1";
        assert_eq!(hash(&header, &fields), None);
        fields[84] = "0";
        fields[1] = "Épée";
        assert_eq!(hash(&header, &fields), None);
    }

    #[test]
    fn an_inventory_prediction_and_silence_do_not_establish_link_custody() {
        let mut inventory = inventory("0", "0");
        let mut item = inventory.items()[&InventorySlot(23)].clone();
        item.slot = InventorySlot(24);
        inventory.apply(InventoryUpdate::Prediction(vec![item.clone()]));
        assert!(inventory.owned_titanium_link(item.slot).is_err());
        inventory.apply(InventoryUpdate::Settled);
        assert!(inventory.owned_titanium_link(item.slot).is_err());
        inventory.apply(InventoryUpdate::Set(vec![item]));
        assert!(inventory.owned_titanium_link(InventorySlot(24)).is_ok());
        assert!(inventory.owned_titanium_link(InventorySlot(4000)).is_err());
    }
}
