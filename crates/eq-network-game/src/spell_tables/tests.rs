use super::*;

fn body(count: u32, words: &[u16]) -> Vec<u8> {
    let mut bytes = count.to_le_bytes().to_vec();
    bytes.extend(words.iter().flat_map(|word| word.to_le_bytes()));
    bytes
}

fn ids(values: &[u16]) -> Vec<SpellId> {
    values.iter().map(|&id| SpellId::new(id).unwrap()).collect()
}

#[test]
fn registration_has_two_byte_ids_and_preserves_order_and_duplicates() {
    assert_eq!(
        decode(REGISTRATION_OPCODE, &body(4, &[7, 9, 7, 0])).unwrap(),
        Some(Table::Registration(ids(&[7, 9, 7, 0])))
    );
}

#[test]
fn replay_has_destination_then_cached_source_and_retains_repeated_destinations() {
    assert_eq!(
        decode(REPLAY_OPCODE, &body(3, &[42, 9001, 7, 42, 42, 9002])).unwrap(),
        Some(Table::Replay(vec![
            Assignment::new(SpellId::new(42).unwrap(), SpellId::new(9001).unwrap()),
            Assignment::new(SpellId::new(7).unwrap(), SpellId::new(42).unwrap()),
            Assignment::new(SpellId::new(42).unwrap(), SpellId::new(9002).unwrap()),
        ]))
    );
}

#[test]
fn counts_are_variable_and_duplicate_lists_can_exceed_the_registry_entry_count() {
    assert_eq!(
        decode(REGISTRATION_OPCODE, &body(0, &[])).unwrap(),
        Some(Table::Registration(vec![]))
    );
    assert_eq!(
        decode(REPLAY_OPCODE, &body(0, &[])).unwrap(),
        Some(Table::Replay(vec![]))
    );
    let count = u32::from(RECORD_COUNT) + 1;
    let words = vec![1; usize::try_from(count).unwrap()];
    let Some(Table::Registration(decoded)) =
        decode(REGISTRATION_OPCODE, &body(count, &words)).unwrap()
    else {
        panic!()
    };
    assert_eq!(decoded.len(), words.len());
    assert!(decoded.iter().all(|id| id.get() == 1));
}

#[test]
fn recognized_tables_reject_missing_counts_short_entries_and_trailing_data() {
    for opcode in [REGISTRATION_OPCODE, REPLAY_OPCODE] {
        for length in 0..4 {
            assert_eq!(
                decode(opcode, &vec![0; length]),
                Err(DecodeError::MissingCount)
            );
        }
        assert!(matches!(
            decode(opcode, &body(1, &[])),
            Err(DecodeError::Length { actual: 4, .. })
        ));
        assert!(matches!(
            decode(opcode, &body(0, &[7])),
            Err(DecodeError::Length {
                expected: 4,
                actual: 6
            })
        ));
    }
    // The same body is a complete registration and a short replay.
    assert!(decode(REGISTRATION_OPCODE, &body(1, &[7])).is_ok());
    assert_eq!(
        decode(REPLAY_OPCODE, &body(1, &[7])),
        Err(DecodeError::Length {
            expected: 8,
            actual: 6
        })
    );
    assert!(decode(REGISTRATION_OPCODE, &body(1, &[7, 9])).is_err());
}

#[test]
fn enormous_counts_are_rejected_before_allocating_entries() {
    for opcode in [REGISTRATION_OPCODE, REPLAY_OPCODE] {
        assert!(matches!(
            decode(opcode, &u32::MAX.to_le_bytes()),
            Err(DecodeError::Length { .. } | DecodeError::CountOverflow)
        ));
    }
}

#[test]
fn a_bad_later_id_rejects_the_whole_table_and_identifies_its_role() {
    assert_eq!(
        decode(REGISTRATION_OPCODE, &body(2, &[9999, 10000])),
        Err(DecodeError::InvalidId {
            entry: 1,
            role: IdRole::Source,
            value: 10000
        })
    );
    assert_eq!(
        decode(REPLAY_OPCODE, &body(2, &[42, 9001, 10000, 9002])),
        Err(DecodeError::InvalidId {
            entry: 1,
            role: IdRole::Destination,
            value: 10000
        })
    );
    assert_eq!(
        decode(REPLAY_OPCODE, &body(2, &[42, 9001, 7, u16::MAX])),
        Err(DecodeError::InvalidId {
            entry: 1,
            role: IdRole::Source,
            value: u16::MAX
        })
    );
    assert_eq!(SpellId::new(10000), None);
    assert_eq!(SpellId::new(9999).unwrap().get(), 9999);
}

#[test]
fn unrelated_packets_are_not_spell_tables() {
    assert_eq!(decode(0x1236, &[]), Ok(None));
    assert_eq!(decode(0x1236, &[0xff; 8]), Ok(None));
}
