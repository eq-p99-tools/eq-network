//! Synthetic received tables, with no installed geometry or connection.
use super::*;
use crate::client::session::feature::testing;
use std::collections::BTreeMap;

fn table(entries: &[(u32, u16)]) -> Message {
    let mut bytes = Vec::new();
    bytes.extend(u32::try_from(entries.len()).unwrap().to_le_bytes());
    for &(number, zone) in entries {
        bytes.extend(number.to_le_bytes());
        // These are remote arrival values. Only reference and zone are projected.
        for value in [999_999.0_f32, 700.0, -42.0, 999.0] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(zone.to_le_bytes());
        bytes.extend(7_u16.to_le_bytes());
    }
    Message::ZonePoints(zoning::ZonePoints::decode(&bytes).unwrap())
}

fn projected(events: &[ClientEvent], session: u64, entries: &[(u32, u16)]) {
    let [ClientEvent::World(WorldEvent::ZoneLineZones { session_id, zones })] = events else {
        panic!("expected one zone mapping event: {events:?}");
    };
    assert_eq!(*session_id, session);
    assert_eq!(*zones, entries.iter().copied().collect::<BTreeMap<_, _>>());
}

#[test]
fn zone_references_are_published_only_after_admission() {
    let mut feature = Transfers::new("Tester", Home::Offered);
    let mut world = World::new(5);
    feature
        .admit(&table(&[(3, 54), (54, 61)]), &mut world)
        .unwrap();
    let outcome = testing::run(|out| feature.admitted(&mut world, out));
    outcome.result.unwrap();
    assert_eq!(outcome.sent, []);
    projected(&outcome.events, 5, &[(3, 54), (54, 61)]);
}

#[test]
fn received_tables_replace_instead_of_extending_the_zone_mapping() {
    let mut feature = Transfers::new("Tester", Home::Offered);
    let mut world = World::new(5);
    for entries in [vec![(3, 54)], vec![(4, 61)], vec![]] {
        let outcome = testing::run(|out| feature.observe(&table(&entries), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, []);
        projected(&outcome.events, 5, &entries);
    }
}

#[test]
fn an_unreadable_table_clears_the_projected_mapping_and_internal_lookup() {
    let mut feature = Transfers::new("Tester", Home::Offered);
    let mut world = World::new(5);
    feature.admit(&table(&[(3, 54)]), &mut world).unwrap();
    let outcome = testing::run(|out| {
        feature.observe(
            &Message::Unreadable {
                part: Part::ZonePoints,
                error: "synthetic bad table".into(),
            },
            &mut world,
            out,
        )
    });
    outcome.result.unwrap();
    projected(&outcome.events, 5, &[]);
    assert!(feature.points.get(3).is_none());
    let again = testing::run(|out| feature.admitted(&mut world, out));
    again.result.unwrap();
    projected(&again.events, 5, &[]);
}

#[test]
fn a_new_zone_without_a_table_publishes_no_previous_zone_references() {
    let mut old = Transfers::new("Tester", Home::Offered);
    let mut world = World::new(5);
    old.admit(&table(&[(3, 54)]), &mut world).unwrap();
    let mut next = Transfers::new("Tester", Home::Offered);
    let mut next_world = World::new(6);
    let outcome = testing::run(|out| next.admitted(&mut next_world, out));
    outcome.result.unwrap();
    projected(&outcome.events, 6, &[]);
}
