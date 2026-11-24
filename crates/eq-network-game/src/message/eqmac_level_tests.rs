use super::*;

#[test]
fn eqmac_level_records_preserve_high_water_and_raw_experience() {
    // Synthetic forms of a repeated level, loss, natural gain and large rise.
    for (current, previous, experience) in [
        (55_u8, 55_u8, 0_u32),
        (1, 55, 0),
        (51, 50, 120),
        (55, 1, 777),
    ] {
        let body: Vec<_> = [u32::from(current), u32::from(previous), experience]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        assert!(
            matches!(eqmac(0x9841, &body).as_slice(), [Message::Event(WorldEvent::Level { current: c, previous: p, experience: e })] if (*c, *p, *e) == (current, previous, experience))
        );
    }
}

#[test]
fn eqmac_level_malformed_records_are_diagnostics_without_a_level_event() {
    let valid: Vec<_> = [51_u32, 50, 123]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    let mut invalid: Vec<Vec<u8>> = (0..12).map(|length| valid[..length].to_vec()).collect();
    invalid.push([valid.as_slice(), &[0]].concat());
    for levels in [[0_u32, 1], [1, 0], [256, 1], [1, 256], [u32::MAX, 55]] {
        invalid.push(
            levels
                .into_iter()
                .chain([0])
                .flat_map(u32::to_le_bytes)
                .collect(),
        );
    }
    for body in invalid {
        assert!(matches!(
            eqmac(0x9841, &body).as_slice(),
            [Message::Unreadable {
                part: Part::World,
                ..
            }]
        ));
    }
}
