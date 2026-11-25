//! The zone session's movement history reports (row 6.653) and their gate
//! after the server's validation exchange (6.369).
use super::super::feature::{testing, Feature, World};
use super::super::servers::{server_type, Setup};
use crate::client::ServerProtocol;
use eq_network_game::food::AutoEat;
use eq_network_game::movement::history::OPCODE as HISTORY_OPCODE;
use eq_network_game::movement::POSITION_OPCODE;
use std::time::{Duration, Instant};

/// A player admitted at `start` on `protocol`'s server; `validated` is
/// whether the zone admission made the validation exchange first.
fn admitted(
    protocol: ServerProtocol,
    validated: bool,
    start: Instant,
) -> (Box<dyn Feature>, World) {
    let server = server_type(protocol);
    let setup = Setup::new("Tester", AutoEat::default());
    let mut feature = server.motion(&setup).unwrap().feature;
    let mut world = World::new(5);
    world.admitted = Some(start);
    world.own_spawn = Some(7);
    world.validated = validated;
    world.player.admit(testing::player(7));
    testing::run_on(server.wire(), |out| feature.admitted(&mut world, out))
        .result
        .unwrap();
    (feature, world)
}

/// Ticks every 50 ms for `seconds`: the reliable history reports, and the
/// counters the position updates carried.
fn idle(
    protocol: ServerProtocol,
    feature: &mut Box<dyn Feature>,
    world: &mut World,
    start: Instant,
    seconds: u64,
) -> (Vec<(u64, usize)>, Vec<u16>) {
    idle_every(protocol, feature, world, (start, 50), seconds)
}

/// As [`idle`], ticking every `every` ms.
fn idle_every(
    protocol: ServerProtocol,
    feature: &mut Box<dyn Feature>,
    world: &mut World,
    (start, every): (Instant, usize),
    seconds: u64,
) -> (Vec<(u64, usize)>, Vec<u16>) {
    let wire = server_type(protocol).wire();
    let (mut reports, mut counters) = (Vec::new(), Vec::new());
    for step in (0..seconds * 1000).step_by(every) {
        let now = start + Duration::from_millis(step);
        let outcome = testing::run_on(wire, |out| feature.tick(now, world, out));
        outcome.result.unwrap();
        assert!(outcome
            .unreliable
            .iter()
            .all(|packet| packet.opcode != HISTORY_OPCODE));
        for packet in &outcome.sent {
            assert_eq!(packet.opcode, HISTORY_OPCODE, "only reports go reliably");
            assert_eq!(packet.body.len() % 17, 1);
            reports.push((step, packet.body.len() / 17));
        }
        for packet in &outcome.unreliable {
            if packet.opcode == POSITION_OPCODE {
                counters.push(u16::from_le_bytes([packet.body[2], packet.body[3]]));
            }
        }
    }
    (reports, counters)
}

#[test]
fn eqemu_titanium_reports_its_history_reliably_about_once_a_second_standing() {
    let start = Instant::now();
    let (mut feature, mut world) = admitted(ServerProtocol::EqEmu, false, start);
    let (reports, counters) = idle(ServerProtocol::EqEmu, &mut feature, &mut world, start, 10);
    assert!((9..=10).contains(&reports.len()), "{reports:?}");
    // Standing still, each report after the first is the one update sent.
    assert!(reports[1..].iter().all(|(_, records)| *records == 1));
    for pair in reports.windows(2) {
        assert!((1000..=1100).contains(&(pair[1].0 - pair[0].0)));
    }
    assert!(counters.len() >= 9);
}

#[test]
fn a_report_due_goes_before_the_update_of_the_same_pass() {
    // Ticks that come only as each update comes due: the report of the
    // last update still goes out alone, before the next one.
    let start = Instant::now();
    let (mut feature, mut world) = admitted(ServerProtocol::EqEmu, false, start);
    let (reports, counters) = idle_every(
        ServerProtocol::EqEmu,
        &mut feature,
        &mut world,
        (start, 1050),
        12,
    );
    assert!(reports.len() >= 10, "{reports:?}");
    assert!(
        reports[1..].iter().all(|(_, records)| *records == 1),
        "{reports:?}"
    );
    assert!(counters.len() >= 10);
}

#[test]
fn a_p99_session_after_the_validation_exchange_never_reports_and_still_counts() {
    let start = Instant::now();
    let (mut feature, mut world) = admitted(ServerProtocol::Project1999, true, start);
    let (reports, counters) = idle(
        ServerProtocol::Project1999,
        &mut feature,
        &mut world,
        start,
        60,
    );
    assert_eq!(reports, []);
    // The update counter goes on as the frames and updates go by (6.369).
    assert!(counters.len() >= 55);
    assert!(counters.windows(2).all(|pair| pair[1] > pair[0]));
    assert!(*counters.last().unwrap() > 60 * 60);
}

#[test]
fn a_validation_exchange_after_movement_started_still_closes_the_history() {
    let start = Instant::now();
    let (mut feature, mut world) = admitted(ServerProtocol::Project1999, false, start);
    // Fail closed: should the exchange come after the movement session.
    world.validated = true;
    let (reports, _) = idle(
        ServerProtocol::Project1999,
        &mut feature,
        &mut world,
        start,
        5,
    );
    assert_eq!(reports, []);
    world.validated = false;
    let (reports, _) = idle(
        ServerProtocol::Project1999,
        &mut feature,
        &mut world,
        start,
        5,
    );
    assert_eq!(
        reports,
        [],
        "an ended history stays ended for the admission"
    );
}

#[test]
fn the_mac_and_rof2_sessions_send_no_history() {
    for protocol in [ServerProtocol::Quarm, ServerProtocol::Takp] {
        let start = Instant::now();
        let (mut feature, mut world) = admitted(protocol, false, start);
        let (reports, _) = idle(protocol, &mut feature, &mut world, start, 5);
        assert_eq!(reports, [], "{protocol:?}");
    }
    // `RoF2`'s movement is not provided at all; its wire keeps none either.
    assert!(!server_type(ServerProtocol::EqEmuRof2)
        .wire()
        .sends_movement_history());
    assert!(server_type(ServerProtocol::Project1999)
        .wire()
        .sends_movement_history());
}
