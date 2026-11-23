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

use super::samples::{pack, record_body, Packed, Record};

/// Every branch's sample, by name.
fn branches() -> [(&'static str, Record); 6] {
    [
        ("creature", Record::creature()),
        ("player", Record::player()),
        ("dressed NPC", Record::dressed_npc()),
        ("treasure", Record::treasure()),
        ("destructible", Record::destructible()),
        (
            "corpse",
            Record {
                npc: 3,
                ..Record::creature()
            },
        ),
    ]
}

#[test]
fn every_branch_is_as_long_as_eqemu_sizes_it_and_reads_back() {
    for (branch, record) in branches() {
        let body = record_body(&record);
        assert_eq!(body.len(), record.encoder_length(), "{branch}");
        let spawn = spawn(&body).unwrap_or_else(|error| panic!("{branch}: {error}"));
        assert_eq!(spawn.name, record.name, "{branch}");
        assert_eq!(u32::from(spawn.spawn_id), record.spawn_id, "{branch}");
        assert_eq!(spawn.level, record.level, "{branch}");
        assert_eq!(spawn.race, record.race, "{branch}");
        assert_eq!(spawn.class, Some(record.class), "{branch}");
        assert_eq!(spawn.hp_percent, Some(record.hp), "{branch}");
        assert!((spawn.size - record.size).abs() < f32::EPSILON, "{branch}");
        assert!((spawn.walk_speed - record.speeds[0]).abs() < f32::EPSILON);
        assert!((spawn.run_speed - record.speeds[1]).abs() < f32::EPSILON);
        assert_eq!(
            spawn.posture,
            Some(PostureState::from(u32::from(record.stand))),
            "{branch}"
        );
        assert_eq!(spawn.name_parts.title, record.title, "{branch}");
        assert_eq!(spawn.name_parts.last_name, record.last_name, "{branch}");
        assert_eq!(spawn.name_parts.suffix, record.suffix, "{branch}");
        assert_eq!(spawn.velocity, [0.0; 3], "{branch}");
        assert!(spawn.forward.abs() < f32::EPSILON, "{branch}");
        assert_eq!(
            spawn.appearance,
            crate::appearance::Appearance::default(),
            "{branch}"
        );
        assert_eq!(
            spawn.listing,
            crate::listing::Listing::default(),
            "{branch}"
        );
        assert_eq!(
            spawn.position,
            Position {
                x: 10.0,
                y: -20.0,
                z: 3.0,
                heading: 256.0
            },
            "{branch}"
        );
    }
}

#[test]
fn each_branch_reads_its_own_fields() {
    let read = |record: &Record| spawn(&record_body(record)).unwrap();
    // A creature: female, invisible, an NPC with a body type.
    let creature = read(&Record::creature());
    assert_eq!(creature.kind, SpawnKind::Npc);
    assert_eq!(creature.gender, 1);
    assert!(creature.invisible);
    assert_eq!(creature.body_type, Some(21));
    assert_eq!(creature.pet_owner, None);
    let player = read(&Record::player());
    assert_eq!(player.kind, SpawnKind::Player);
    assert_eq!((player.gender, player.invisible), (0, false));
    // Corpses and the NPC byte past 3.
    for (npc, kind) in [
        (2, SpawnKind::PlayerCorpse),
        (3, SpawnKind::NpcCorpse),
        (4, SpawnKind::Unknown(4)),
        (0xff, SpawnKind::Unknown(0xff)),
    ] {
        let record = Record {
            npc,
            ..Record::creature()
        };
        assert_eq!(read(&record).kind, kind);
    }
    // A pet's owner; one past 16 bits is no owner.
    for (owner, read_as) in [(123, Some(123)), (0x1_0000, None)] {
        let record = Record {
            pet_owner: owner,
            ..Record::creature()
        };
        assert_eq!(read(&record).pet_owner, read_as);
    }
    // A treasure chest has a body type; a destructible object has none,
    // and its bitfields are the encoder's constant, so neither its gender
    // nor any invisibility is read from them.
    let treasure = read(&Record::treasure());
    assert_eq!(
        (treasure.kind, treasure.body_type),
        (SpawnKind::Npc, Some(21))
    );
    let destructible = read(&Record::destructible());
    assert_eq!(destructible.body_type, None);
    assert_eq!((destructible.gender, destructible.invisible), (0, false));
    // A destructible object's suffix bit is always set; a suffix follows
    // only when the record holds more than its tail.
    let suffixed = Record {
        suffix: "of Oak",
        ..Record::destructible()
    };
    assert_eq!(record_body(&suffixed).len(), suffixed.encoder_length());
    assert_eq!(read(&suffixed).name_parts.suffix, "of Oak");
    // A destructible object of the treasure class: one block, as a
    // destructible.
    let both = Record {
        other: DESTRUCTIBLE | TREASURE,
        ..Record::destructible()
    };
    assert_eq!(read(&both).body_type, None);
    // A rewritten body type comes as race 127 and body type 11, and is read
    // as sent: race 127 is not dressed.
    let rewritten = Record {
        race: 127,
        body_type: 11,
        ..Record::creature()
    };
    let rewritten = read(&rewritten);
    assert_eq!((rewritten.race, rewritten.body_type), (127, Some(11)));
    // Every dressed race, and the races either side of each.
    for race in [0, 1, 12, 128, 130, 330, 522] {
        let record = Record {
            race,
            dressed: true,
            ..Record::creature()
        };
        assert_eq!(read(&record).race, race);
    }
    for race in [13, 127, 129, 131, 329, 331, 521, 523] {
        let record = Record {
            race,
            ..Record::creature()
        };
        assert_eq!(read(&record).race, race);
        let dressed = Record {
            dressed: true,
            ..record
        };
        assert!(spawn(&record_body(&dressed)).is_err(), "race {race}");
    }
    // A player of any race is dressed.
    let player = Record {
        race: 36,
        ..Record::player()
    };
    assert_eq!(read(&player).race, 36);
    // A bad walk or run speed costs only that speed.
    let slow = Record {
        speeds: [f32::NAN, -1.0],
        ..Record::creature()
    };
    let slow = read(&slow);
    assert_eq!((slow.walk_speed, slow.run_speed), (0.0, 0.0));
}

#[test]
#[allow(clippy::too_many_lines)]
fn a_record_whose_bits_contradict_each_other_is_refused() {
    for (what, record) in [
        // One of a destructible's own bits without the rest of 0xe1.
        (
            "0x80 alone",
            Record {
                other: 0x80,
                ..Record::destructible()
            },
        ),
        (
            "0x40 alone",
            Record {
                other: 0x40 | 0x20 | 0x01,
                ..Record::destructible()
            },
        ),
        (
            "0xe1 without the buyer bit",
            Record {
                other: 0xe0,
                ..Record::destructible()
            },
        ),
        // A destructible's bitfields not the encoder's constant.
        (
            "destructible bitfields",
            Record {
                bitfields: 0,
                ..Record::destructible()
            },
        ),
        // A property count the branch doesn't take.
        (
            "destructible with a body type",
            Record {
                properties: 1,
                ..Record::destructible()
            },
        ),
        (
            "creature without one",
            Record {
                properties: 0,
                ..Record::creature()
            },
        ),
        (
            "two properties",
            Record {
                properties: 2,
                ..Record::creature()
            },
        ),
        // Zero and past 16 bits.
        (
            "spawn id 0",
            Record {
                spawn_id: 0,
                ..Record::creature()
            },
        ),
        (
            "spawn id past 16 bits",
            Record {
                spawn_id: 0x1_0000,
                ..Record::creature()
            },
        ),
        (
            "negative size",
            Record {
                size: -1.0,
                ..Record::creature()
            },
        ),
        (
            "size not finite",
            Record {
                size: f32::INFINITY,
                ..Record::creature()
            },
        ),
        // A flagged title or suffix that is not written.
        (
            "flagged title missing",
            Record {
                other: TITLE,
                ..Record::creature()
            },
        ),
        (
            "flagged suffix missing",
            Record {
                other: SUFFIX,
                ..Record::creature()
            },
        ),
        // A title written that no bit flags.
        (
            "unflagged title",
            Record {
                title: "Sir",
                ..Record::creature()
            },
        ),
    ] {
        assert!(spawn(&record_body(&record)).is_err(), "{what}");
    }
}

#[test]
fn every_branch_refuses_a_cut_at_every_offset_and_a_trailing_byte() {
    for (branch, record) in branches() {
        let body = record_body(&record);
        for end in 0..body.len() {
            assert!(spawn(&body[..end]).is_err(), "{branch} cut at {end}");
        }
        let mut long = body.clone();
        long.push(0);
        assert!(spawn(&long).is_err(), "{branch}");
    }
}

#[test]
fn every_string_must_end_in_its_field() {
    // A name of 63 letters fits its 64 bytes; 64 do not.
    for (length, fits) in [(63, true), (64, false)] {
        let name: &'static str = "a".repeat(length).leak();
        let body = record_body(&Record {
            name,
            ..Record::creature()
        });
        assert_eq!(spawn(&body).is_ok(), fits, "name of {length}");
    }
    // The last name, title and suffix fit 31 letters and their NUL.
    for (length, fits) in [(31, true), (32, false)] {
        let part: &'static str = "b".repeat(length).leak();
        for record in [
            Record {
                last_name: part,
                ..Record::player()
            },
            Record {
                title: part,
                ..Record::player()
            },
            Record {
                suffix: part,
                ..Record::player()
            },
        ] {
            assert_eq!(
                spawn(&record_body(&record)).is_ok(),
                fits,
                "part of {length}"
            );
        }
    }
    // A treasure chest's three strings end in their 64 bytes.
    for (length, fits) in [(63, true), (64, false)] {
        let model: &'static str = "c".repeat(length).leak();
        let record = Record {
            object: Some([model, "", ""]),
            ..Record::treasure()
        };
        assert_eq!(
            spawn(&record_body(&record)).is_ok(),
            fits,
            "model of {length}"
        );
    }
    // The GUID the encoder writes is 16 characters; any other length is
    // refused.
    let body = record_body(&Record::creature());
    let at = body
        .windows(17)
        .position(|window| window == b"0000000000000000\0")
        .unwrap();
    let mut short = body.clone();
    short.remove(at);
    assert!(spawn(&short).is_err());
    let mut long = body[..at].to_vec();
    long.push(b'0');
    long.extend_from_slice(&body[at..]);
    assert!(spawn(&long).is_err());
    // A name the error never quotes.
    let unprintable = Record {
        name: "Bad\u{1}Name",
        ..Record::creature()
    };
    let error = spawn(&record_body(&unprintable)).unwrap_err().to_string();
    assert!(!error.contains("Bad"), "{error}");
}

#[test]
fn our_position_writer_puts_each_field_in_its_own_bits() {
    // Our table's bit positions, as words: each field alone.
    let alone = |packed: Packed| pack(&packed);
    assert_eq!(
        alone(Packed {
            angle: 1,
            ..Packed::default()
        }),
        [1, 0, 0, 0, 0]
    );
    assert_eq!(
        alone(Packed {
            y: 1,
            ..Packed::default()
        }),
        [1 << 12, 0, 0, 0, 0]
    );
    assert_eq!(
        alone(Packed {
            delta_z: 1,
            ..Packed::default()
        }),
        [0, 1, 0, 0, 0]
    );
    assert_eq!(
        alone(Packed {
            delta_x: 1,
            ..Packed::default()
        }),
        [0, 1 << 13, 0, 0, 0]
    );
    assert_eq!(
        alone(Packed {
            x: 1,
            ..Packed::default()
        }),
        [0, 0, 1, 0, 0]
    );
    assert_eq!(
        alone(Packed {
            heading: 1,
            ..Packed::default()
        }),
        [0, 0, 1 << 19, 0, 0]
    );
    assert_eq!(
        alone(Packed {
            delta_heading: 1,
            ..Packed::default()
        }),
        [0, 0, 0, 1, 0]
    );
    assert_eq!(
        alone(Packed {
            z: 1,
            ..Packed::default()
        }),
        [0, 0, 0, 1 << 10, 0]
    );
    assert_eq!(
        alone(Packed {
            animation: 1,
            ..Packed::default()
        }),
        [0, 0, 0, 0, 1]
    );
    assert_eq!(
        alone(Packed {
            delta_y: 1,
            ..Packed::default()
        }),
        [0, 0, 0, 0, 1 << 10]
    );
}

#[test]
fn the_packed_position_reads_each_field_from_its_own_bits() {
    // Each coordinate alone, so a swapped pair fails.
    let read = |packed: Packed| packed_position(pack(&packed));
    let at = |x: f32, y: f32, z: f32, heading: f32| Position { x, y, z, heading };
    assert_eq!(
        read(Packed {
            x: 8,
            ..Packed::default()
        }),
        at(1.0, 0.0, 0.0, 0.0)
    );
    assert_eq!(
        read(Packed {
            y: 8,
            ..Packed::default()
        }),
        at(0.0, 1.0, 0.0, 0.0)
    );
    assert_eq!(
        read(Packed {
            z: 8,
            ..Packed::default()
        }),
        at(0.0, 0.0, 1.0, 0.0)
    );
    assert_eq!(
        read(Packed {
            heading: 4,
            ..Packed::default()
        }),
        at(0.0, 0.0, 0.0, 1.0)
    );
    // Negatives, and both 19-bit extremes.
    let low = -(1 << 18);
    let high = (1 << 18) - 1;
    assert_eq!(
        read(Packed {
            x: -1,
            y: -12,
            z: low,
            ..Packed::default()
        }),
        at(-0.125, -1.5, -32768.0, 0.0)
    );
    assert_eq!(
        read(Packed {
            x: high,
            y: low,
            z: high,
            ..Packed::default()
        }),
        at(32767.875, -32768.0, 32767.875, 0.0)
    );
    // The heading's top: 2047 quarters, the last before a full turn.
    assert_eq!(
        read(Packed {
            heading: 2047,
            ..Packed::default()
        })
        .heading,
        511.75
    );
    // The fields not read change nothing.
    let unread = Packed {
        angle: 0xfff,
        delta_x: -1,
        delta_y: -1,
        delta_z: -1,
        delta_heading: -1,
        animation: -1,
        ..Packed::default()
    };
    assert_eq!(read(unread), at(0.0, 0.0, 0.0, 0.0));
    // Padding bits change nothing either.
    let mut padded = pack(&Packed {
        x: 8,
        ..Packed::default()
    });
    padded[0] |= 1 << 31;
    padded[1] |= 0x3f << 26;
    padded[2] |= 1 << 31;
    padded[3] |= 0x7 << 29;
    padded[4] |= 0x1ff << 23;
    assert_eq!(packed_position(padded), at(1.0, 0.0, 0.0, 0.0));
}

#[test]
fn the_own_spawn_is_a_players_record_walked_as_every_record_is() {
    // A player's record from the other writer is the own spawn too.
    let other = record_body(&Record::player());
    let own = own_spawn(&other).unwrap();
    assert_eq!((own.id, own.title.as_str()), (77, "Lady"));
    // What is not a player's is refused.
    for record in [
        Record::creature(),
        Record::treasure(),
        Record::destructible(),
        Record {
            npc: 2,
            ..Record::player()
        },
        Record {
            other: TREASURE,
            object: Some(["", "", ""]),
            title: "",
            suffix: "",
            ..Record::player()
        },
    ] {
        let body = record_body(&record);
        assert!(spawn(&body).is_ok());
        assert!(own_spawn(&body).is_err());
    }
    // The own spawn's id, as the entry keeps it.
    assert_eq!(spawn_id(&spawn_body(&Sample::default())), Some(4321));
    assert_eq!(spawn_id(&[0; 3]), None);
}

#[test]
fn rof2s_zone_decoder_makes_each_record_one_spawn_and_a_refused_one_unreadable() {
    use crate::message::{rof2, Message, Part};
    use crate::world::WorldEvent;
    let body = record_body(&Record::creature());
    let messages = rof2(SPAWN_OPCODE, &body);
    let [Message::Event(WorldEvent::Spawns(spawns))] = messages.as_slice() else {
        panic!("{messages:?}");
    };
    assert_eq!(spawns.len(), 1);
    assert_eq!(spawns[0], spawn(&body).unwrap());
    // A refused record is unreadable in fixed words, naming no one.
    let messages = rof2(SPAWN_OPCODE, &body[..body.len() - 1]);
    let [Message::Unreadable {
        part: Part::World,
        error,
    }] = messages.as_slice()
    else {
        panic!("{messages:?}");
    };
    assert!(!error.contains("a_rat"), "{error}");
    // Nothing else is read yet.
    assert!(rof2(PROFILE_OPCODE, &profile_body(&Sample::default()).bytes).is_empty());
}

use super::samples::{despawn_body, update_body};
use crate::world::WorldEvent;

/// An update's position, velocity and forward input.
fn moved(packed: Packed) -> (Position, [f32; 3], f32) {
    match position(&update_body(9, 0, &packed)).unwrap() {
        WorldEvent::Position {
            spawn_id: 9,
            position,
            velocity,
            forward,
        } => (position, velocity, forward),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_update_reads_its_position_from_the_records_table() {
    let at = |x: f32, y: f32, z: f32, heading: f32| Position { x, y, z, heading };
    // Each coordinate alone, so a swapped pair fails.
    assert_eq!(
        moved(Packed {
            x: 8,
            ..Packed::default()
        })
        .0,
        at(1.0, 0.0, 0.0, 0.0)
    );
    assert_eq!(
        moved(Packed {
            y: 8,
            ..Packed::default()
        })
        .0,
        at(0.0, 1.0, 0.0, 0.0)
    );
    assert_eq!(
        moved(Packed {
            z: 8,
            ..Packed::default()
        })
        .0,
        at(0.0, 0.0, 1.0, 0.0)
    );
    assert_eq!(
        moved(Packed {
            heading: 4,
            ..Packed::default()
        })
        .0,
        at(0.0, 0.0, 0.0, 1.0)
    );
    // Negatives and both 19-bit extremes.
    let low = -(1 << 18);
    let high = (1 << 18) - 1;
    assert_eq!(
        moved(Packed {
            x: low,
            y: high,
            z: -1,
            ..Packed::default()
        })
        .0,
        at(-32768.0, 32767.875, -0.125, 0.0)
    );
    // The heading is unsigned 12 bits.
    assert!(
        (moved(Packed {
            heading: 0xfff,
            ..Packed::default()
        })
        .0
        .heading
            - 1023.75)
            .abs()
            < f32::EPSILON
    );
    // The first 12 bits, the delta heading and the vehicle id change
    // nothing.
    let unread = Packed {
        angle: 0xfff,
        delta_heading: -1,
        ..Packed::default()
    };
    assert_eq!(moved(unread), (at(0.0, 0.0, 0.0, 0.0), [0.0; 3], 0.0));
    let aboard = update_body(9, 0xffff, &Packed::default());
    assert_eq!(
        position(&aboard).unwrap(),
        position(&update_body(9, 0, &Packed::default())).unwrap()
    );
}

#[test]
fn an_updates_motion_is_signed_and_on_titaniums_scales() {
    let unit = crate::world::DELTA_UNITS_PER_SECOND;
    // Each delta alone, at both of its 13-bit extremes.
    for (value, read) in [(1, 1.0), (-1, -1.0), (4095, 4095.0), (-4096, -4096.0)] {
        assert_eq!(
            moved(Packed {
                delta_x: value,
                ..Packed::default()
            })
            .1,
            [read * unit, 0.0, 0.0]
        );
        assert_eq!(
            moved(Packed {
                delta_y: value,
                ..Packed::default()
            })
            .1,
            [0.0, read * unit, 0.0]
        );
        assert_eq!(
            moved(Packed {
                delta_z: value,
                ..Packed::default()
            })
            .1,
            [0.0, 0.0, read * unit]
        );
    }
    // The animation, at both of its 10-bit extremes.
    let input = crate::world::ANIMATION_INPUT;
    for (value, read) in [(1, 1.0), (-1, -1.0), (511, 511.0), (-512, -512.0)] {
        let (_, velocity, forward) = moved(Packed {
            animation: value,
            ..Packed::default()
        });
        assert_eq!((velocity, forward), ([0.0; 3], read * input));
    }
    // Zero is exactly zero.
    let (_, velocity, forward) = moved(Packed::default());
    assert!(velocity.iter().all(|value| value.to_bits() == 0) && forward.to_bits() == 0);
}

#[test]
fn an_update_and_a_removal_refuse_what_they_cannot_trust() {
    let update = update_body(
        9,
        0,
        &Packed {
            x: 8,
            ..Packed::default()
        },
    );
    assert_eq!(update.len(), 24);
    let removal = despawn_body(9);
    assert_eq!(despawn(&removal).unwrap(), WorldEvent::Despawn(9));
    for (what, body, read) in [
        (
            "update",
            &update,
            position as fn(&[u8]) -> Result<WorldEvent>,
        ),
        ("removal", &removal, despawn),
    ] {
        for end in 0..body.len() {
            assert!(read(&body[..end]).is_err(), "{what} cut at {end}");
        }
        let mut long = body.clone();
        long.push(0);
        assert!(read(&long).is_err(), "{what} with a trailing byte");
    }
    assert!(position(&update_body(0, 0, &Packed::default())).is_err());
    for id in [0, 0x1_0000, u32::MAX] {
        assert!(despawn(&despawn_body(id)).is_err(), "{id}");
    }
    // The removal's last byte is the encoder's constant, and not read.
    let mut zero = despawn_body(9);
    zero[4] = 0;
    assert_eq!(despawn(&zero).unwrap(), WorldEvent::Despawn(9));
}

#[test]
fn rof2s_zone_decoder_reads_movement_and_removal_as_one_event_each() {
    use crate::message::{rof2, Message, Part};
    let update = update_body(9, 0, &Packed::default());
    assert!(matches!(
        rof2(POSITION_OPCODE, &update).as_slice(),
        [Message::Event(WorldEvent::Position { spawn_id: 9, .. })]
    ));
    assert!(matches!(
        rof2(DESPAWN_OPCODE, &despawn_body(9)).as_slice(),
        [Message::Event(WorldEvent::Despawn(9))]
    ));
    for opcode in [POSITION_OPCODE, DESPAWN_OPCODE] {
        assert!(matches!(
            rof2(opcode, &[0; 3]).as_slice(),
            [Message::Unreadable {
                part: Part::World,
                ..
            }]
        ));
    }
    // Stock EQEmu sends neither OP_MobUpdate nor OP_NPCMoveUpdate.
    assert!(rof2(0x2c84, &[0; 24]).is_empty());
    assert!(rof2(0x5892, &[0; 24]).is_empty());
}
