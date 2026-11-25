//! The movement session's history (row 6.653): what it records, in which
//! order, and when it reports.
use super::tests::{airborne, request, titanium_flagged};
use super::*;
use crate::movement::history::{Cause, Record, LIFT, RECORD_LEN};
use crate::movement::TITANIUM_UPDATE_COUNTER;

fn titanium(start: Instant) -> MotionSession {
    titanium_flagged(start)
        .with_counter(TITANIUM_UPDATE_COUNTER)
        .with_history(true)
}

fn records(report: &EncodedCommand) -> Vec<Record> {
    assert_eq!(report.opcode, 0x6a1b);
    assert_eq!(report.body.len() % RECORD_LEN, 1);
    assert_eq!(report.body.last(), Some(&0));
    report.body[..report.body.len() - 1]
        .chunks(RECORD_LEN)
        .map(|chunk| Record::decode(chunk).unwrap())
        .collect()
}

/// Ticks every 50 ms through `ms`, as the zone session does, sending each
/// due update and report. Returns the updates' counters and the reports.
fn run(
    motion: &mut MotionSession,
    start: Instant,
    ms: std::ops::Range<u64>,
    mut make: impl FnMut(u64, Instant) -> Option<MovementRequest>,
) -> (Vec<u16>, Vec<(u64, Vec<Record>)>) {
    let (mut updates, mut reports) = (Vec::new(), Vec::new());
    for step in ms.step_by(50) {
        let now = start + Duration::from_millis(step);
        if let Some(request) = make(step, now) {
            motion
                .send_move_sample(&request, now, |sample| {
                    updates.push(sample.sequence);
                    Ok(())
                })
                .unwrap();
        }
        motion
            .tick_sample(now, |sample| {
                updates.push(sample.sequence);
                Ok(())
            })
            .unwrap();
        motion
            .report_history(now, false, |report| {
                reports.push((step, records(report)));
                Ok(())
            })
            .unwrap();
    }
    (updates, reports)
}

#[test]
fn standing_still_reports_each_update_alone_about_once_a_second() {
    let start = Instant::now();
    let mut motion = titanium(start);
    let (updates, reports) = run(&mut motion, start, 0..6000, |_, _| None);
    // The first frame at the spawn point is the only frame written, the
    // rest its duplicates; the first report also carries the first update.
    let (first_at, first) = &reports[0];
    assert_eq!(*first_at, 1000);
    let first: Vec<_> = first.iter().map(|r| (r.cause, r.sequence)).collect();
    assert_eq!(
        first,
        [(Cause::Collision, 0), (Cause::TeleportB, updates[0])]
    );
    // Then one report a second, each the one update sent in that second,
    // carrying the update's own counter, lifted.
    assert!(reports.len() >= 5);
    for ((_, report), sequence) in reports[1..].iter().zip(&updates[1..]) {
        let [record] = report[..] else {
            panic!("one record a report: {report:?}")
        };
        assert_eq!(
            (record.cause, record.sequence),
            (Cause::TeleportB, *sequence)
        );
        assert!((record.z - LIFT).abs() < f32::EPSILON);
    }
    for pair in reports.windows(2) {
        let gap = pair[1].0 - pair[0].0;
        assert!((1000..=1100).contains(&gap), "report gap {gap}");
    }
}

#[test]
fn moving_records_every_frame_lifted_in_order() {
    let start = Instant::now();
    let mut motion = titanium(start);
    // The host's samples come every 100 ms, the session's ticks every 50.
    #[allow(clippy::cast_precision_loss)]
    let (updates, reports) = run(&mut motion, start, 0..3000, |step, now| {
        (step % 100 == 0 && step > 0).then(|| request(now, step as f32 / 200.0))
    });
    let all: Vec<Record> = reports.into_iter().flat_map(|(_, report)| report).collect();
    // The frames between samples wait for the next sample and go along the
    // step, so the sequence runs on by one; only an update the gate lets
    // out at a tick, between samples, finds the frames since the last
    // sample at its place, duplicates consumed. The first frame is the
    // standing start, before the first sample.
    for pair in all.windows(2).skip(1) {
        let step = pair[1].sequence.wrapping_sub(pair[0].sequence);
        assert!(
            step == 1 || pair[1].cause == Cause::TeleportB,
            "{:?} then {:?}",
            pair[0],
            pair[1]
        );
        assert!(step >= 1);
    }
    let frames = all.iter().filter(|r| r.cause == Cause::Collision).count();
    let sent: Vec<u16> = all
        .iter()
        .filter(|r| r.cause == Cause::TeleportB)
        .map(|r| r.sequence)
        .collect();
    // Some 65 frames a second for the two seconds reported, and every
    // update sent reported with its counter.
    assert!((110..=135).contains(&frames), "{frames} frames");
    assert_eq!(sent, updates[..sent.len()]);
    for record in &all {
        assert!((record.z - LIFT).abs() < f32::EPSILON);
        assert!(record.y.abs() < f32::EPSILON);
    }
    // Frames go forward along the walk.
    let xs: Vec<f32> = all
        .iter()
        .filter(|r| r.cause == Cause::Collision)
        .map(|r| r.x)
        .collect();
    assert!(xs.windows(2).all(|pair| pair[1] > pair[0]));
}

#[test]
fn the_counter_counts_the_same_with_or_without_a_history() {
    let start = Instant::now();
    let walk = |motion: &mut MotionSession| {
        #[allow(clippy::cast_precision_loss)]
        let (updates, _) = run(motion, start, 0..4000, |step, now| {
            (step % 100 == 0 && step > 0 && step < 2000).then(|| request(now, step as f32 / 200.0))
        });
        updates
    };
    let kept = walk(&mut titanium(start));
    let mut silent = titanium_flagged(start)
        .with_counter(TITANIUM_UPDATE_COUNTER)
        .with_history(false);
    assert!(!silent.keeps_history());
    assert_eq!(walk(&mut silent), kept);
    assert!(!silent
        .report_history(start + Duration::from_secs(9), true, |_| panic!("reported"))
        .unwrap());
}

#[test]
fn only_the_history_entry_counter_keeps_a_history() {
    let start = Instant::now();
    assert!(!titanium_flagged(start).with_history(true).keeps_history());
    let mut motion = titanium_flagged(start)
        .with_history(true)
        .with_counter(TITANIUM_UPDATE_COUNTER);
    assert!(!motion.keeps_history());
    assert!(!motion
        .report_history(start + Duration::from_secs(5), true, |_| panic!("reported"))
        .unwrap());
}

#[test]
fn a_report_waits_a_second_unless_forced_and_is_never_empty() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut motion = titanium(start);
    let never = |_: &EncodedCommand| -> Result<()> { panic!("reported") };
    assert!(!motion.report_history(at(999), false, never).unwrap());
    // A failed send keeps the records for the next.
    assert!(motion
        .report_history(at(1000), false, |_| anyhow::bail!("synthetic"))
        .is_err());
    let mut sent = Vec::new();
    assert!(motion
        .report_history(at(1000), false, |report| {
            sent = records(report);
            Ok(())
        })
        .unwrap());
    assert_eq!(sent.len(), 1);
    // Nothing new is queued while standing between updates, so even the
    // forced flush has nothing to send.
    assert!(!motion.report_history(at(1010), true, never).unwrap());
    motion
        .send_move_sample(&request(at(1100), 0.5), at(1100), |_| Ok(()))
        .unwrap();
    // The zone change's flush sends at once.
    let mut flushed = Vec::new();
    assert!(motion
        .report_history(at(1110), true, |report| {
            flushed = records(report);
            Ok(())
        })
        .unwrap());
    assert!(flushed.iter().any(|r| r.cause == Cause::TeleportB));
}

#[test]
fn ending_the_history_drops_it_for_the_admission() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut motion = titanium(start);
    motion
        .send_move_sample(&request(at(300), 0.5), at(300), |_| Ok(()))
        .unwrap();
    let before = motion.counter.value(at(2000));
    motion.end_history();
    assert!(!motion.keeps_history());
    assert!(!motion
        .report_history(at(2000), true, |_| panic!("reported"))
        .unwrap());
    assert_eq!(motion.counter.value(at(2000)), before);
}

#[test]
fn a_correction_records_no_frame_between_the_two_places() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut motion = titanium(start);
    let there = Position {
        x: 400.0,
        ..Position::default()
    };
    motion.correct(there, at(500)).unwrap();
    let mut sent = Vec::new();
    motion
        .report_history(at(1500), false, |report| {
            sent = records(report);
            Ok(())
        })
        .unwrap();
    let frames: Vec<_> = sent
        .iter()
        .filter(|r| r.cause == Cause::Collision)
        .map(|r| (r.x, r.sequence))
        .collect();
    // One frame at the spawn, then the first frame at the new place with
    // the sequence that frame had.
    assert_eq!(frames, [(0.0, 0), (400.0, 32)]);
}

#[test]
fn a_pad_crossing_records_the_region_the_warp_the_entry_and_the_landing() {
    use crate::zoning::LocalPad;
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut motion = titanium(start);
    motion
        .send_move_sample(&request(at(300), 0.5), at(300), |_| Ok(()))
        .unwrap();
    let entry = motion.position();
    let landing = Position {
        x: -20.0,
        z: 6.0,
        ..entry
    };
    let mut reported = None;
    motion
        .cross_pad(LocalPad::new(entry, landing), at(400), at(400), |sample| {
            reported = Some(sample.sequence);
            Ok(())
        })
        .unwrap();
    let mut sent = Vec::new();
    motion
        .report_history(at(400), true, |report| {
            sent = records(report);
            Ok(())
        })
        .unwrap();
    let tail: Vec<_> = sent[sent.len() - 4..]
        .iter()
        .map(|r| (r.cause, r.sequence))
        .collect();
    let entry_sequence = reported.unwrap();
    // The warp's own frame at the old place is a duplicate of the last
    // frame there: consumed, its sequence gone by.
    assert_eq!(
        tail,
        [
            (Cause::ZoneLine, entry_sequence - 3),
            (Cause::Unknown(5), entry_sequence - 2),
            (Cause::TeleportB, entry_sequence),
            (Cause::TeleportA, entry_sequence + 1),
        ]
    );
    let [region, same_zone, update, landed] = sent[sent.len() - 4..] else {
        panic!("four records")
    };
    assert!((region.z - entry.z).abs() < f32::EPSILON);
    assert!((same_zone.x - entry.x).abs() < f32::EPSILON);
    assert!((update.z - (entry.z + LIFT)).abs() < f32::EPSILON);
    assert!(
        (landed.x - landing.x).abs() < f32::EPSILON && (landed.z - landing.z).abs() < f32::EPSILON
    );
    // The landing's update next carries the counter after all of them.
    let mut landed_update = None;
    motion
        .tick_sample(at(700), |sample| {
            landed_update = Some(sample.sequence);
            Ok(())
        })
        .unwrap();
    assert_eq!(landed_update, Some(45 + 4 + 2));
}

#[test]
fn a_jump_in_place_records_every_frame_of_its_rise_and_fall() {
    // Straight up and down: only Z moves, and the frames between the
    // host's samples still wait for the next one to place them.
    let start = Instant::now();
    let mut motion = airborne(start)
        .with_gate(Some(crate::movement::UpdateGate::Flagged {
            pack: |sample| sample.packet(),
            rule: crate::movement::TITANIUM_FLAGS,
        }))
        .with_counter(TITANIUM_UPDATE_COUNTER)
        .with_history(true);
    let heights = [2.0, 3.6, 4.6, 5.0, 4.6, 3.6, 2.0, 0.0];
    let take_off = 1000;
    let (_, mut reports) = run(&mut motion, start, 0..take_off, |_, _| None);
    motion.jumped();
    let (_, airborne_reports) = run(&mut motion, start, take_off..4000, |step, now| {
        let index = usize::try_from((step - take_off) / 100).ok()?;
        let z = *heights
            .get(index)
            .filter(|_| step % 100 == 0 && step > take_off)?;
        let mut request = request(now, 0.0);
        request.mode = MovementMode::Fall;
        request.position.z = z;
        Some(request)
    });
    reports.extend(airborne_reports);
    let airborne: Vec<Record> = reports
        .into_iter()
        .flat_map(|(_, report)| report)
        .filter(|record| record.cause == Cause::Collision && record.z > LIFT + 0.01)
        .collect();
    // About 65 frames a second over the 0.65 s above the take-off's lift.
    assert!(airborne.len() >= 38, "{} frames in the air", airborne.len());
    let rising = airborne
        .windows(2)
        .filter(|pair| pair[1].sequence == pair[0].sequence + 1);
    assert!(rising.count() >= airborne.len() - 4);
}
