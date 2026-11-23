//! The readers against bodies built from the field list by [`super::samples`].
use super::samples::{profile_body, set_length, spawn_body, Sample};
use super::*;

fn sample_player(sample: &Sample) -> Result<PlayerState> {
    player(&profile_body(sample).bytes, &spawn_body(sample), 512.0)
}

#[test]
fn rof2s_player_reads_every_field_from_the_place_it_is_named_for() {
    let sample = Sample::default();
    let player = sample_player(&sample).unwrap();
    assert_eq!(player.name, "Tester");
    assert_eq!(player.gender, 1);
    assert_eq!(player.race, 6);
    assert_eq!(player.class, Some(12));
    assert_eq!(player.level, 35);
    assert_eq!(player.deity, Some(201));
    assert_eq!(player.practice_points, Some(7));
    assert_eq!(player.mana, 900);
    assert_eq!(player.endurance, Some(450));
    assert_eq!(
        player.base_attributes,
        Some(BaseAttributes {
            strength: 75,
            stamina: 80,
            charisma: 85,
            dexterity: 90,
            intelligence: 95,
            agility: 100,
            wisdom: 105,
        })
    );
    let skills = player.skills.unwrap();
    assert_eq!(skills.len(), 100);
    assert_eq!((skills[0], skills[99]), (0, 297));
    let disciplines = player.disciplines.unwrap();
    assert_eq!(disciplines.len(), 300);
    assert_eq!(&disciplines[..4], [5000, 5001, 5002, 0]);
    let languages = player.languages.unwrap();
    assert_eq!(languages.len(), 32);
    assert_eq!((languages[0], languages[27], languages[28]), (100, 73, 0));
    // The profile writes y before x.
    assert_eq!(
        player.position,
        Position {
            x: 10.5,
            y: -20.25,
            z: 3.0,
            heading: 128.0
        }
    );
    // Gems 1 to 8; empty markers are no spell.
    assert_eq!(
        player.memorized_spells,
        [
            Some(11),
            None,
            Some(13),
            None,
            Some(15),
            Some(16),
            Some(17),
            Some(18)
        ]
    );
    assert_eq!(
        player.spell_refresh_ms,
        Some([1000, 1001, 1002, 1003, 1004, 1005, 1006, 1007])
    );
    assert_eq!(player.spawn_id, 4321);
    assert!((player.size - 2.5).abs() < f32::EPSILON);
    assert!((player.walk_speed - 0.46).abs() < f32::EPSILON);
    assert!((player.run_speed - 0.7).abs() < f32::EPSILON);
    assert_eq!(player.hp_percent, Some(100));
    assert_eq!(player.posture, Some(PostureState::Sitting));
    assert_eq!(player.name_parts.title, "Sir");
    assert_eq!(player.name_parts.last_name, "Smith");
    assert_eq!(player.name_parts.suffix, "the Bold");
    assert_eq!(player.titanium_environment, None);
    assert_eq!(player.appearance, crate::appearance::Appearance::default());
    assert_eq!(player.listing, crate::listing::Listing::default());
    // A heading saved on a 256-unit revolution reads on the 512 scale.
    let profile = profile_body(&sample).bytes;
    let half = player_heading(&profile, &spawn_body(&sample), 256.0);
    assert!((half - 256.0).abs() < f32::EPSILON);
}

fn player_heading(profile: &[u8], spawn: &[u8], revolution: f32) -> f32 {
    player(profile, spawn, revolution).unwrap().position.heading
}

#[test]
fn rof2s_own_spawn_is_as_long_as_eqemu_sizes_it() {
    // `ENCODE(OP_ZoneSpawns)` sizes a player's record at 206 + 216 bytes,
    // the name and last name, and each title or suffix with its NUL.
    let sample = Sample::default();
    let expected =
        206 + 216 + "Tester".len() + "Smith".len() + "Sir".len() + 1 + "the Bold".len() + 1;
    assert_eq!(spawn_body(&sample).len(), expected);
    let bare = Sample {
        other: 0,
        ..Sample::default()
    };
    assert_eq!(spawn_body(&bare).len(), 206 + 216 + 6 + 5);
    let player = sample_player(&bare).unwrap();
    assert_eq!(player.name_parts.title, "");
    assert_eq!(player.name_parts.suffix, "");
}

#[test]
fn a_cut_at_every_offset_refuses_and_never_panics() {
    let sample = Sample::default();
    let profile = profile_body(&sample).bytes;
    let spawn = spawn_body(&sample);
    for end in 0..profile.len() {
        let mut cut = profile[..end].to_vec();
        assert!(super::profile(&cut).is_err(), "profile cut at {end}");
        // With the length word set to match, the cut is still refused.
        if end >= 9 {
            set_length(&mut cut);
            assert!(super::profile(&cut).is_err(), "profile cut at {end}");
        }
    }
    for end in 0..spawn.len() {
        assert!(own_spawn(&spawn[..end]).is_err(), "spawn cut at {end}");
    }
}

#[test]
fn rof2s_profile_refuses_every_count_and_length_it_cannot_trust() {
    let sample = Sample::default();
    let body = profile_body(&sample);
    assert!(super::profile(&body.bytes).is_ok());
    assert_eq!(body.counts.len(), 32);
    for offset in &body.counts {
        for change in [1u32, u32::MAX] {
            let mut wrong = body.bytes.clone();
            let count = u32::from_le_bytes(wrong[*offset..*offset + 4].try_into().unwrap());
            wrong[*offset..*offset + 4].copy_from_slice(&count.wrapping_add(change).to_le_bytes());
            assert!(super::profile(&wrong).is_err(), "count at {offset}");
        }
    }
    // A trailing byte, with and without the length word following it.
    let mut long = body.bytes.clone();
    long.push(0);
    assert!(super::profile(&long).is_err());
    set_length(&mut long);
    assert!(super::profile(&long).is_err());
    // The length word wrong by one.
    let mut wrong = body.bytes.clone();
    wrong[4] ^= 1;
    assert!(super::profile(&wrong).is_err());
    // The checksum is not checked.
    let mut checksum = body.bytes.clone();
    checksum[0] ^= 0xff;
    assert!(super::profile(&checksum).is_ok());
}

#[test]
fn rof2s_profile_refuses_strings_that_do_not_end_in_their_field() {
    let sample = Sample::default();
    let body = profile_body(&sample).bytes;
    // The first bandolier's name is empty: a lone NUL right after its
    // count. Replace it with 64 letters and no NUL.
    let bandoliers = profile_body(&sample).counts[18];
    let at = bandoliers + 4;
    assert_eq!(body[at], 0);
    let mut unterminated = body[..at].to_vec();
    unterminated.extend(std::iter::repeat_n(b'a', 64));
    unterminated.extend_from_slice(&body[at + 1..]);
    set_length(&mut unterminated);
    assert!(super::profile(&unterminated).is_err());
    // 63 letters and a NUL fit.
    let mut longest = body[..at].to_vec();
    longest.extend(std::iter::repeat_n(b'a', 63));
    longest.extend_from_slice(&body[at..]);
    set_length(&mut longest);
    assert!(super::profile(&longest).is_ok());
    // A name filling its 64-byte field has no NUL in it.
    let name = Sample {
        name: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ..Sample::default()
    };
    assert!(sample_player(&name).is_ok());
    let at = profile_body(&name).counts[20] + 4;
    let mut full = profile_body(&name).bytes;
    full[at + 63] = b'A';
    assert!(super::profile(&full).is_err());
    // An empty name is refused, and nothing names the player in an error.
    let empty = Sample {
        name: "",
        ..Sample::default()
    };
    let error = player(
        &profile_body(&empty).bytes,
        &spawn_body(&Sample::default()),
        512.0,
    )
    .unwrap_err()
    .to_string();
    assert!(!error.contains("Tester"), "{error}");
}

#[test]
fn rof2s_player_refuses_values_it_cannot_trust() {
    for refused in [
        Sample {
            position: [f32::NAN, 0.0, 0.0, 0.0],
            ..Sample::default()
        },
        Sample {
            position: [0.0, 0.0, 0.0, f32::INFINITY],
            ..Sample::default()
        },
        Sample {
            spawn_id: 0,
            ..Sample::default()
        },
        Sample {
            spawn_id: 0x1_0000,
            ..Sample::default()
        },
        Sample {
            speeds: [-1.0, 0.7],
            ..Sample::default()
        },
        Sample {
            speeds: [0.46, f32::NAN],
            ..Sample::default()
        },
        Sample {
            strength: 0x8000_0000,
            ..Sample::default()
        },
        // An NPC, a treasure chest and a destructible object are not the
        // player.
        Sample {
            npc: 1,
            ..Sample::default()
        },
        Sample {
            other: 0x04,
            ..Sample::default()
        },
        Sample {
            other: 0xe1,
            ..Sample::default()
        },
        // Without the single body-type property the record is not a
        // player's.
        Sample {
            properties: 0,
            ..Sample::default()
        },
        // A flagged title or suffix that is empty.
        Sample {
            title: "",
            ..Sample::default()
        },
        Sample {
            suffix: "",
            ..Sample::default()
        },
    ] {
        assert!(sample_player(&refused).is_err());
    }
    // A flagged title that is missing: the flag set, no title written.
    let sample = Sample::default();
    let mut spawn = spawn_body(&Sample {
        other: 0,
        ..Sample::default()
    });
    let other = spawn.iter().position(|byte| *byte == 0).unwrap() + 1 + 4 + 1 + 4 + 1 + 4;
    spawn[other] = TITLE;
    assert!(player(&profile_body(&sample).bytes, &spawn, 512.0).is_err());
    // A trailing byte.
    let mut long = spawn_body(&sample);
    long.push(0);
    assert!(own_spawn(&long).is_err());
}
