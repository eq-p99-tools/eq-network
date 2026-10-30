use super::*;
use crate::books::Book;

/// A record of a made-up item of `class`, with `id`, in `EQMac` slot `slot`.
fn record(class: i16, id: i16, slot: i16) -> Vec<u8> {
    let mut record = vec![0; RECORD];
    record[..14].copy_from_slice(b"Synthetic item");
    // Neither no rent nor no drop.
    record[175] = 1;
    record[176] = 1;
    record[178..180].copy_from_slice(&class.to_le_bytes());
    record[180..182].copy_from_slice(&id.to_le_bytes());
    record[184..186].copy_from_slice(&slot.to_le_bytes());
    record
}

fn put(record: &mut [u8], at: usize, bytes: &[u8]) {
    record[at..at + bytes.len()].copy_from_slice(bytes);
}

/// The full inventory TAKP would send for these records: a count, then the
/// tagged records compressed.
fn inventory_body(records: &[Vec<u8>]) -> Vec<u8> {
    let mut data = Vec::new();
    for record in records {
        let tag = match i16::from_le_bytes([record[178], record[179]]) {
            0 => 16740u16,
            1 => 16742,
            _ => 16741,
        };
        data.extend_from_slice(&tag.to_le_bytes());
        data.extend_from_slice(record);
    }
    let mut body = vec![u8::try_from(records.len()).unwrap(), 0];
    body.extend(miniz_oxide::deflate::compress_to_vec_zlib(&data, 4));
    body
}

fn single_item(opcode: u16, record: &[u8]) -> InventoryItem {
    match decode(opcode, record).unwrap() {
        Some(InventoryUpdate::Set(items) | InventoryUpdate::Cursor(items)) => {
            assert_eq!(items.len(), 1);
            items.into_iter().next().unwrap()
        }
        other => panic!("{other:?}"),
    }
}

/// A made-up magic blade with statistics, restrictions and a worn effect,
/// in the primary slot.
fn blade() -> Vec<u8> {
    let mut blade = record(0, 1234, 13);
    put(&mut blade, 0, b"Synthetic blade\0");
    put(&mut blade, 64, b"*Synthetic blade\0");
    blade[174] = 25;
    blade[177] = 2;
    put(&mut blade, 182, &640u16.to_le_bytes());
    put(&mut blade, 188, &(1u32 << 13).to_le_bytes());
    put(&mut blade, 192, &1250i32.to_le_bytes());
    // STR, STA, CHA, DEX, INT, AGI and WIS, then the resists, MR first.
    for (offset, value) in [-3i8, 4, 5, 6, 7, 8, 9, 1, 2, 3, 4, 5]
        .into_iter()
        .enumerate()
    {
        blade[228 + offset] = value.to_le_bytes()[0];
    }
    put(&mut blade, 240, &25i16.to_le_bytes());
    put(&mut blade, 242, &10i16.to_le_bytes());
    put(&mut blade, 244, &7i16.to_le_bytes());
    blade[249] = 30;
    blade[250] = 9;
    blade[253] = 3;
    blade[254] = 1;
    put(&mut blade, 268, &5u32.to_le_bytes());
    put(&mut blade, 272, &3u32.to_le_bytes());
    // A worn effect, at level 30.
    blade[277] = 30;
    blade[279] = 2;
    put(&mut blade, 280, &123u16.to_le_bytes());
    blade[324] = 35;
    put(&mut blade, 352, &20i16.to_le_bytes());
    blade
}

#[test]
fn a_worn_item_reads_its_restrictions_bonuses_and_effect() {
    let item = single_item(PLACED_OPCODE, &blade());
    assert_eq!(item.slot, InventorySlot(13));
    let details = &item.details;
    assert_eq!(
        (details.id, details.name.as_str(), details.lore.as_str()),
        (1234, "Synthetic blade", "*Synthetic blade")
    );
    assert_eq!(details.flags, ["MAGIC", "LORE"]);
    assert_eq!(
        (
            details.weight_tenths,
            details.slots,
            details.classes,
            details.races
        ),
        (25, 1 << 13, 5, 3)
    );
    assert_eq!((details.price, details.icon), (Some(1250), Some(640)));
    assert_eq!(
        details.bonuses,
        Some(ItemBonuses {
            strength: -3,
            stamina: 4,
            agility: 8,
            dexterity: 6,
            charisma: 5,
            intelligence: 7,
            wisdom: 9,
            hit_points: 25,
            mana: 10,
            endurance: 0,
        })
    );
    assert_eq!(
        details.equipment,
        Some(EquipmentRules {
            required_level: 20,
            recommended_level: 35,
            worn: Some(WornEffect {
                spell_id: 123,
                effect_type: 2,
                level: 30,
                level2: 30,
            }),
        })
    );
    assert!(item.activation.effect.is_none());
    assert_eq!(
        (item.rules.size, item.rules.item_type, item.stack_count),
        (2, 3, None)
    );
}

#[test]
fn statistics_are_labelled_as_titaniums_are() {
    let item = single_item(PLACED_OPCODE, &blade());
    let stat = |label: &str| {
        item.details
            .stats
            .iter()
            .find(|stat| stat.label == label)
            .map(|stat| stat.value)
    };
    for (label, value) in [
        ("Cold", 3),
        ("Disease", 4),
        ("Poison", 5),
        ("Magic resist", 1),
        ("Fire", 2),
        ("STR", -3),
        ("AGI", 8),
        ("HP", 25),
        ("AC", 7),
        ("Required level", 20),
        ("Recommended level", 35),
        ("Delay", 30),
        ("Damage", 9),
        ("Worn spell", 123),
    ] {
        assert_eq!(stat(label), Some(value), "{label}");
    }
    assert_eq!(stat("Range"), None);
    assert_eq!(stat("Click spell"), None);
}

#[test]
fn the_one_effect_is_a_click_a_proc_or_a_scroll_by_its_type() {
    let mut wand = record(0, 77, 22);
    wand[246] = 5;
    wand[278] = 3;
    wand[277] = 12;
    wand[279] = 3;
    put(&mut wand, 280, &93u16.to_le_bytes());
    put(&mut wand, 292, &1500i32.to_le_bytes());
    let item = single_item(PLACED_OPCODE, &wand);
    assert_eq!(
        item.activation.effect,
        Some(ClickEffect {
            spell_id: 93,
            kind: ClickKind::Expendable,
            required_level: 0,
            effect_level: 12,
            cast_time_ms: 1500,
            recast_delay_seconds: 0,
            recast_type: 0,
        })
    );
    assert_eq!(
        (
            item.activation.maximum_charges,
            item.charges,
            item.stack_count
        ),
        (5, 3, None)
    );
    assert!(item
        .details
        .stats
        .iter()
        .any(|stat| stat.label == "Click spell" && stat.value == 93));
    // A proc names its spell, and casts nothing on a click.
    wand[279] = 0;
    let item = single_item(PLACED_OPCODE, &wand);
    assert!(item.activation.effect.is_none());
    assert!(item
        .details
        .stats
        .iter()
        .any(|stat| stat.label == "Proc spell" && stat.value == 93));
    // A scroll teaches its spell.
    let mut scroll = record(0, 78, 23);
    scroll[253] = 20;
    scroll[279] = 7;
    put(&mut scroll, 280, &202u16.to_le_bytes());
    let item = single_item(PLACED_OPCODE, &scroll);
    assert_eq!(item.scroll_spell, Some(202));
    assert!(item.activation.effect.is_none());
    // No effect at all reads as none, though its type byte is zero.
    let plain = single_item(PLACED_OPCODE, &record(0, 79, 24));
    assert_eq!(plain.details.stats, Vec::<ItemStat>::new());
    assert_eq!(plain.scroll_spell, None);
}

#[test]
fn stacks_follow_takps_rule_and_count_their_units() {
    let mut rations = record(0, 80, 22);
    rations[246] = 20;
    rations[253] = 14;
    rations[278] = 5;
    let item = single_item(PLACED_OPCODE, &rations);
    assert_eq!(
        (item.stack_count, item.charges, item.rules.stack_size),
        (Some(5), 0, 20)
    );
    // Only some types stack, and only with charges.
    rations[253] = 21;
    let potion = single_item(PLACED_OPCODE, &rations);
    assert_eq!(
        (potion.stack_count, potion.charges, potion.rules.stack_size),
        (None, 5, 1)
    );
    rations[253] = 14;
    rations[246] = 0;
    assert_eq!(single_item(PLACED_OPCODE, &rations).stack_count, None);
    // A stack cannot hold fewer than none.
    rations[246] = 20;
    rations[278] = 0xff;
    assert!(decode(PLACED_OPCODE, &rations).is_err());
}

#[test]
fn bags_and_books_read_their_own_middles() {
    let mut bag = record(1, 81, 23);
    bag[268] = 5;
    bag[269] = 8;
    bag[271] = 2;
    // What sits where a common item keeps its type and statistics.
    bag[253] = 0x55;
    bag[240] = 0x55;
    let item = single_item(CONTAINER_OPCODE, &bag);
    assert_eq!(item.bag_slots, 8);
    assert_eq!(
        (
            item.rules.bag_type,
            item.rules.bag_size,
            item.rules.item_type
        ),
        (5, 2, 0)
    );
    assert_eq!(item.details.bonuses, Some(ItemBonuses::default()));
    assert_eq!(item.details.stats, Vec::<ItemStat>::new());
    assert_eq!(
        (item.details.classes, item.details.races),
        (u32::MAX, u32::MAX)
    );
    assert!(item.book.is_none());
    bag[269] = 11;
    assert!(decode(PLACED_OPCODE, &bag).is_err());

    let mut book = record(2, 82, 24);
    book[230] = 1;
    put(&mut book, 231, b"SynthText\0");
    let item = single_item(BOOK_OPCODE, &book);
    assert_eq!(
        item.book,
        Some(Book {
            file: "SynthText".into(),
            kind: 1,
        })
    );
    assert_eq!(item.bag_slots, 0);
    book[230] = 0;
    assert_eq!(single_item(PLACED_OPCODE, &book).book.unwrap().kind, 0);
    // Only class 2 reads.
    let mut note = record(0, 83, 25);
    put(&mut note, 231, b"SynthText\0");
    assert!(single_item(PLACED_OPCODE, &note).book.is_none());
    put(&mut note, 178, &3i16.to_le_bytes());
    assert!(decode(PLACED_OPCODE, &note).is_err());
}

#[test]
fn flags_name_no_rent_no_drop_and_lore() {
    let mut item = record(0, 84, 22);
    item[175] = 0;
    item[176] = 0;
    put(&mut item, 64, b"*\0");
    assert_eq!(
        single_item(PLACED_OPCODE, &item).details.flags,
        ["NO RENT", "NO DROP", "LORE"]
    );
    // A bag's magic byte is something else.
    let mut bag = record(1, 85, 22);
    bag[254] = 1;
    assert_eq!(
        single_item(PLACED_OPCODE, &bag).details.flags,
        Vec::<String>::new()
    );
}

#[test]
fn records_without_identity_or_with_bad_numbers_are_refused() {
    for (at, bytes) in [
        (180, 0i16.to_le_bytes().to_vec()),
        (180, (-5i16).to_le_bytes().to_vec()),
        (0, vec![0]),
        (352, (-1i16).to_le_bytes().to_vec()),
        (192, (-1i32).to_le_bytes().to_vec()),
        (184, 31i16.to_le_bytes().to_vec()),
    ] {
        let mut item = record(0, 86, 22);
        put(&mut item, at, &bytes);
        assert!(decode(PLACED_OPCODE, &item).is_err(), "offset {at}");
    }
    assert!(decode(PLACED_OPCODE, &record(0, 86, 22)[..359]).is_err());
}

#[test]
fn the_full_inventory_lands_at_titanium_slots_with_bag_contents_inside() {
    let mut bag = record(1, 90, 23);
    bag[269] = 4;
    // Index 2 of the bag in the second pack slot.
    let inside = record(0, 91, 262);
    let cursor = record(0, 92, 0);
    let worn = record(0, 93, 13);
    let book = record(2, 94, 2007);
    let body = inventory_body(&[cursor, worn, bag, inside, book]);
    let Some(InventoryUpdate::Snapshot(items)) = decode(INVENTORY_OPCODE, &body).unwrap() else {
        panic!("not a snapshot");
    };
    let slots: Vec<_> = items.iter().map(|item| item.slot).collect();
    assert_eq!(
        slots,
        [
            InventorySlot::CURSOR,
            InventorySlot(13),
            InventorySlot(23),
            InventorySlot(23).child(2).unwrap(),
            InventorySlot(2007),
        ]
    );
    assert_eq!(items[3].details.id, 91);
    // The count byte does not decide how many there are.
    let mut miscounted = body.clone();
    miscounted[0] = 200;
    assert_eq!(
        decode(INVENTORY_OPCODE, &miscounted).unwrap(),
        decode(INVENTORY_OPCODE, &body).unwrap()
    );
    // Nothing held comes uncompressed.
    assert_eq!(
        decode(INVENTORY_OPCODE, &[0, 0]).unwrap(),
        Some(InventoryUpdate::Snapshot(Vec::new()))
    );
}

#[test]
fn a_full_inventory_that_does_not_add_up_is_refused() {
    let mut bag = record(1, 90, 23);
    bag[269] = 2;
    let inside = |index: i16| record(0, 91, 260 + index);
    for records in [
        // Past the bag's last place, and in a bag that is not there.
        vec![bag.clone(), inside(2)],
        vec![inside(0)],
        // Two items in one slot.
        vec![record(0, 92, 22), record(0, 93, 22)],
    ] {
        assert!(decode(INVENTORY_OPCODE, &inventory_body(&records)).is_err());
    }
    assert!(decode(INVENTORY_OPCODE, &inventory_body(&[bag.clone(), inside(1)])).is_ok());
    // A record tagged for another class.
    let mut body = inventory_body(&[bag]);
    let mut data = miniz_oxide::inflate::decompress_to_vec_zlib(&body[2..]).unwrap();
    data[0] = 0x64;
    body.truncate(2);
    body.extend(miniz_oxide::deflate::compress_to_vec_zlib(&data, 4));
    assert!(decode(INVENTORY_OPCODE, &body).is_err());
    // A partial record, a broken stream, and a lone count.
    data.pop();
    let mut partial = vec![1, 0];
    partial.extend(miniz_oxide::deflate::compress_to_vec_zlib(&data, 4));
    for body in [partial, vec![1, 0, 1, 2, 3, 4], vec![1]] {
        assert!(decode(INVENTORY_OPCODE, &body).is_err());
    }
}

#[test]
fn single_items_are_set_in_place_or_summoned_onto_the_cursor() {
    let pack = record(0, 95, 22);
    for opcode in [PLACED_OPCODE, ITEM_OPCODE, BOOK_OPCODE, CONTAINER_OPCODE] {
        assert!(matches!(
            decode(opcode, &pack).unwrap(),
            Some(InventoryUpdate::Set(items)) if items[0].slot == InventorySlot(22)
        ));
    }
    // Bag contents are one higher than `EQMac` numbers them.
    let inside = record(0, 96, 329);
    assert!(matches!(
        decode(PLACED_OPCODE, &inside).unwrap(),
        Some(InventoryUpdate::Set(items)) if items[0].slot == InventorySlot(330)
    ));
    // The trade slots are not the inventory's to follow.
    assert_eq!(decode(PLACED_OPCODE, &record(0, 97, 3000)).unwrap(), None);
    let summoned = record(0, 98, 0);
    assert!(matches!(
        decode(SUMMONED_OPCODE, &summoned).unwrap(),
        Some(InventoryUpdate::Cursor(items)) if items[0].slot == InventorySlot::CURSOR
    ));
    assert!(decode(SUMMONED_OPCODE, &pack).is_err());
    assert_eq!(decode(0x1234, &pack).unwrap(), None);
}

#[test]
fn the_server_empties_slots_and_uses_up_units() {
    let change = |slot: i32, to: u32, quantity: u32| {
        let mut body = slot.to_le_bytes().to_vec();
        body.extend_from_slice(&to.to_le_bytes());
        body.extend_from_slice(&quantity.to_le_bytes());
        body
    };
    assert_eq!(
        decode(MOVE_OPCODE, &change(262, u32::MAX, u32::MAX)).unwrap(),
        Some(InventoryUpdate::Remove(InventorySlot(263)))
    );
    assert_eq!(
        decode(DELETE_CHARGE_OPCODE, &change(0, u32::MAX, u32::MAX)).unwrap(),
        Some(InventoryUpdate::Used(InventorySlot::CURSOR))
    );
    // A move the server reports some other way leaves the inventory unknown.
    assert_eq!(
        decode(MOVE_OPCODE, &change(22, 23, 0)).unwrap(),
        Some(InventoryUpdate::Invalidated)
    );
    assert!(decode(MOVE_OPCODE, &change(8000, u32::MAX, u32::MAX)).is_err());
    assert!(decode(DELETE_CHARGE_OPCODE, &[0; 8]).is_err());
}

#[test]
fn moves_go_out_in_eqmac_numbers() {
    let words = |command: &EncodedCommand| -> Vec<u32> {
        command
            .body
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| u32::from_le_bytes(*word))
            .collect()
    };
    let pick_up = move_item(
        InventorySlot(22),
        InventorySlot::CURSOR,
        MoveQuantity::Whole,
    )
    .unwrap();
    assert_eq!(pick_up.opcode, MOVE_OPCODE);
    assert_eq!(words(&pick_up), [22, 0, 0]);
    // Bag contents are one lower than Titanium numbers them, and a count
    // moves part of a stack.
    let five = MoveQuantity::Count(std::num::NonZeroU32::new(5).unwrap());
    let banked = move_item(
        InventorySlot(22).child(3).unwrap(),
        InventorySlot(2000).child(0).unwrap(),
        five,
    )
    .unwrap();
    assert_eq!(words(&banked), [253, 2030, 5]);
    // EQMac has no charm slot.
    assert!(move_item(InventorySlot(0), InventorySlot::CURSOR, MoveQuantity::Whole).is_err());
}
