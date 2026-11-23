use super::*;

/// An item update as `SendGuildBankItemUpdate` fills it.
#[allow(clippy::too_many_arguments, reason = "one argument per field")]
fn update(
    slot: u16,
    area: u16,
    display: u32,
    item_id: u32,
    permissions: u32,
    names: (&str, &str, &str),
) -> Vec<u8> {
    let mut body = vec![0; ITEM_UPDATE_LENGTH];
    body[0..4].copy_from_slice(&ITEM_UPDATE.to_le_bytes());
    body[4..8].copy_from_slice(&7u32.to_le_bytes());
    body[8..10].copy_from_slice(&slot.to_le_bytes());
    body[10..12].copy_from_slice(&area.to_le_bytes());
    body[12..16].copy_from_slice(&display.to_le_bytes());
    body[16..20].copy_from_slice(&item_id.to_le_bytes());
    body[20..24].copy_from_slice(&1234u32.to_le_bytes());
    body[24..28].copy_from_slice(&20u32.to_le_bytes());
    body[28..32].copy_from_slice(&permissions.to_le_bytes());
    body[32] = 1;
    body[33] = 0;
    for (at, name) in [(34, names.0), (98, names.1), (162, names.2)] {
        body[at..at + name.len()].copy_from_slice(name.as_bytes());
    }
    body[226..228].copy_from_slice(&0xbeefu16.to_le_bytes());
    body
}

fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn read(body: &[u8]) -> Result<GuildBankUpdate> {
    Ok(titanium(OPCODE, body)?.expect("the guild bank opcode"))
}

#[test]
fn an_item_shown_in_the_deposit_area_reads_every_field() {
    let body = update(3, 0, 1, 13005, 1, ("Bone Chips", "Depositor", "Allowed"));
    assert_eq!(
        read(&body).unwrap(),
        GuildBankUpdate::Shown(GuildBankEntry {
            area: GuildBankArea::Deposit,
            slot: 3,
            item_id: 13005,
            icon_id: 1234,
            quantity: 20,
            permission: GuildBankPermission::SingleMember,
            permitted: "Allowed".into(),
            donor: "Depositor".into(),
            name: "Bone Chips".into(),
            merges: true,
            usable_for_actor: false,
        })
    );
}

#[test]
fn each_permission_and_the_main_area_read() {
    for (value, permission) in [
        (0, GuildBankPermission::BankerOnly),
        (1, GuildBankPermission::SingleMember),
        (2, GuildBankPermission::PublicIfUsable),
        (3, GuildBankPermission::Public),
        (9, GuildBankPermission::Unknown(9)),
    ] {
        let GuildBankUpdate::Shown(entry) =
            read(&update(199, 1, 1, 5, value, ("A", "", ""))).unwrap()
        else {
            panic!("an item shown");
        };
        assert_eq!(entry.area, GuildBankArea::Main);
        assert_eq!(entry.slot, 199);
        assert_eq!(entry.permission, permission);
        assert_eq!(entry.permitted, "");
    }
}

#[test]
fn a_removal_names_its_slot_and_the_item_that_left() {
    let body = update(12, 1, 0, 13005, 3, ("Bone Chips", "Depositor", ""));
    assert_eq!(
        read(&body).unwrap(),
        GuildBankUpdate::Removed {
            area: GuildBankArea::Main,
            slot: 12,
            item_id: 13005,
        }
    );
}

#[test]
fn the_deposit_answer_the_acknowledgement_and_the_clear_read() {
    assert_eq!(
        read(&words(&[5, 0, 0])).unwrap(),
        GuildBankUpdate::DepositAnswered { taken: true }
    );
    assert_eq!(
        read(&words(&[5, 0, 1])).unwrap(),
        GuildBankUpdate::DepositAnswered { taken: false }
    );
    assert_eq!(
        read(&words(&[10, 0])).unwrap(),
        GuildBankUpdate::Acknowledged
    );
    assert_eq!(
        read(&words(&[0, 0, 40, 200])).unwrap(),
        GuildBankUpdate::Cleared {
            deposit_slots: 40,
            main_slots: 200,
        }
    );
}

#[test]
fn a_wrong_length_for_each_action_is_refused() {
    let mut long_update = update(1, 0, 1, 5, 0, ("A", "", ""));
    long_update.push(0);
    for body in [
        long_update,
        update(1, 0, 1, 5, 0, ("A", "", ""))[..227].to_vec(),
        words(&[5, 0]),
        words(&[5, 0, 0, 0]),
        words(&[10]),
        words(&[10, 0, 0]),
        words(&[0, 0, 40]),
        vec![1, 0],
    ] {
        assert!(read(&body).is_err(), "{} bytes", body.len());
    }
}

#[test]
fn unknown_actions_areas_and_flags_are_refused() {
    for body in [
        words(&[3, 0, 0, 0]),
        words(&[7, 0, 0, 0, 0]),
        update(1, 2, 1, 5, 0, ("A", "", "")),
        update(1, 0, 2, 5, 0, ("A", "", "")),
        words(&[5, 0, 2]),
    ] {
        assert!(read(&body).is_err());
    }
}

#[test]
fn a_name_without_its_end_is_refused() {
    let full = "x".repeat(64);
    for names in [
        (full.as_str(), "", ""),
        ("A", full.as_str(), ""),
        ("A", "", full.as_str()),
    ] {
        assert!(read(&update(1, 0, 1, 5, 1, names)).is_err());
    }
}

#[test]
fn another_opcode_is_not_read() {
    assert_eq!(titanium(OPCODE + 1, &words(&[10, 0])).unwrap(), None);
}

#[test]
fn debug_output_leaves_out_players_names() {
    let GuildBankUpdate::Shown(entry) = read(&update(
        1,
        0,
        1,
        5,
        1,
        ("Bone Chips", "Depositor", "Allowed"),
    ))
    .unwrap() else {
        panic!("an item shown");
    };
    let text = format!("{:?}", GuildBankUpdate::Shown(entry));
    assert!(text.contains("Bone Chips"));
    assert!(!text.contains("Depositor"));
    assert!(!text.contains("Allowed"));
}

#[test]
fn the_profile_says_whether_the_player_is_a_banker() {
    let mut profile = vec![0; PROFILE_BANKER + 4];
    assert_eq!(
        titanium_profile(&profile).unwrap(),
        GuildBankUpdate::Banker(false)
    );
    profile[PROFILE_BANKER] = 1;
    assert_eq!(
        titanium_profile(&profile).unwrap(),
        GuildBankUpdate::Banker(true)
    );
    // K1: another word is unreadable, never "banker".
    profile[PROFILE_BANKER] = 2;
    assert!(titanium_profile(&profile).is_err());
    assert!(titanium_profile(&profile[..PROFILE_BANKER + 3]).is_err());
}

#[test]
fn the_world_reader_passes_guild_bank_news_on() {
    assert_eq!(
        crate::world::titanium_update(OPCODE, &words(&[10, 0])).unwrap(),
        Some(crate::world::WorldEvent::GuildBank(
            GuildBankUpdate::Acknowledged
        ))
    );
}
