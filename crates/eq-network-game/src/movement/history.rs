//! Titanium movement history: the records the program keeps of the player's
//! own positions, and the reports it sends of them (row 6.653).
//!
//! The original client records an entry for its own spawn every frame and
//! at every position update it sends, and reports the queued entries about
//! once a second (`OP_FloatListThing`). [`Recorder`] keeps that queue and
//! builds the reports; the movement session decides what is recorded and
//! when a report goes out. See `docs/protocol/movement-history.md` for the
//! evidence and remaining gaps.

use crate::command::EncodedCommand;
use crate::world::Position;
use anyhow::{ensure, Result};
use std::collections::VecDeque;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Titanium's `OP_FloatListThing`, handled as movement history by `EQEmu`.
pub const OPCODE: u16 = 0x6a1b;

/// Size of one packed record, excluding any packet trailer.
pub const RECORD_LEN: usize = 17;

/// Cause codes named by `EQEmu`'s movement-history handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Cause {
    /// A position recorded by the client's collision update.
    Collision,
    /// The server's `TeleportB` code.
    TeleportB,
    /// The server's `TeleportA` code.
    TeleportA,
    /// A zone-line record.
    ZoneLine,
    /// An unclassified cause, retained exactly as received.
    Unknown(u8),
}

impl From<u8> for Cause {
    fn from(value: u8) -> Self {
        match value {
            1 => Self::Collision,
            2 => Self::TeleportB,
            3 => Self::TeleportA,
            4 => Self::ZoneLine,
            other => Self::Unknown(other),
        }
    }
}

/// One record from the Titanium client's movement-history serializer.
///
/// Coordinates remain in wire order and retain non-finite values for diagnostics.
/// The record has no heading. This is decoded evidence, not a movement request.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Record {
    /// Y coordinate, the first float on the wire.
    pub y: f32,
    /// X coordinate, the second float on the wire.
    pub x: f32,
    /// Z coordinate, the third float on the wire.
    pub z: f32,
    /// Why the client recorded this value.
    pub cause: Cause,
    /// Low 16 bits of the client's millisecond clock; wraps every 65.536 seconds.
    pub timestamp_ms: u16,
    /// The client's wrapping 16-bit record sequence.
    pub sequence: u16,
}

impl Record {
    /// Decode exactly one record without interpreting an enclosing packet.
    ///
    /// # Errors
    /// Returns an error unless `bytes` contains exactly [`RECORD_LEN`] bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() == RECORD_LEN,
            "movement history record must be 17 bytes"
        );
        Ok(Self {
            y: f32::from_le_bytes(bytes[0..4].try_into()?),
            x: f32::from_le_bytes(bytes[4..8].try_into()?),
            z: f32::from_le_bytes(bytes[8..12].try_into()?),
            cause: bytes[12].into(),
            timestamp_ms: u16::from_le_bytes(bytes[13..15].try_into()?),
            sequence: u16::from_le_bytes(bytes[15..17].try_into()?),
        })
    }
}

/// The height a frame's (cause 1) and an update's (cause 2) record sits
/// above the player's position: the program adds a lift L (0x489a32), at
/// least 1.9. Both official captures on a local server show exactly 1.9
/// for a human. The read's other term, the model's own height (actor
/// virtual +0x28), is unread, so a larger model may differ (inferred).
pub const LIFT: f32 = 1.9;

/// How old the oldest queued record must be before an unforced report goes
/// out (0x48c1a3: game ms now minus the oldest record's time, at least 1000).
pub const REPORT_AGE: Duration = Duration::from_millis(1000);

/// The most records one report carries (0x4b6f1f stops at 10,000); the rest
/// wait for the next report.
pub const REPORT_CAP: usize = 10_000;

/// The most records the queue keeps if reports stop going out. The program's
/// pool only grows; this client drops the oldest beyond this bound and
/// counts them ([`Recorder::take_dropped`]), so a stalled send cannot grow
/// without limit. About eight minutes of frames at 65 a second.
pub const QUEUE_BOUND: usize = 32_768;

/// The cause the program writes for each kind of entry (the server's names
/// in [`Cause`]).
pub mod causes {
    /// A frame of the player's own movement (0x48a7f9).
    pub const FRAME: u8 = 1;
    /// A position update sent (0x4ac5bc, 0x4ac76a).
    pub const UPDATE: u8 = 2;
    /// An in-zone warp's landing (0x48ac4b), recorded without the lift.
    pub const LANDING: u8 = 3;
    /// Entering a zone-line region (0x48b60e).
    pub const ZONE_LINE: u8 = 4;
    /// Finding that region's zone is the current one (0x48b647).
    pub const SAME_ZONE: u8 = 5;
}

/// The game's millisecond clock: the program stamps records with ms since
/// the game started (0x46d655). This client starts it at the first record
/// of the process (inferred); only its low 16 bits are sent. The epoch is
/// process-wide, so a record stamped before it (an earlier admission's frame
/// offered later) saturates to 0.
fn game_ms(at: Instant) -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    let epoch = *EPOCH.get_or_init(|| at);
    u64::try_from(at.saturating_duration_since(epoch).as_millis()).unwrap_or(u64::MAX)
}

/// One queued entry, before it is written.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Entry {
    at: Instant,
    y: f32,
    x: f32,
    z: f32,
    cause: u8,
    sequence: u16,
}

impl Entry {
    fn write(&self, body: &mut Vec<u8>) {
        body.extend_from_slice(&self.y.to_le_bytes());
        body.extend_from_slice(&self.x.to_le_bytes());
        body.extend_from_slice(&self.z.to_le_bytes());
        body.push(self.cause);
        let clock = (game_ms(self.at) & 0xffff) as u16;
        body.extend_from_slice(&clock.to_le_bytes());
        body.extend_from_slice(&self.sequence.to_le_bytes());
    }
}

/// The player's own movement history for one admission, and its reports.
///
/// Every entry the update counter counts is offered here, so a record's
/// sequence is the counter's value when it was recorded. A frame at the
/// same place as the last frame written is consumed but not written: its
/// sequence still went by, as in the program's serializer (0x4b6f1f).
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    queue: VecDeque<Entry>,
    /// The last frame written, as recorded (with the lift).
    last_frame: Option<[f32; 3]>,
    dropped: u64,
}

impl Recorder {
    /// An empty history, as the program's reset leaves it (0x4b6ef8).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one entry of `cause` at `position`, with the lift for frames
    /// and updates.
    pub fn record(&mut self, cause: u8, position: Position, sequence: u16, at: Instant) {
        let lifted = matches!(cause, causes::FRAME | causes::UPDATE);
        let z = if lifted {
            position.z + LIFT
        } else {
            position.z
        };
        if cause == causes::FRAME {
            let place = [position.y, position.x, z];
            if self.last_frame == Some(place) {
                return;
            }
            self.last_frame = Some(place);
        }
        if self.queue.len() >= QUEUE_BOUND {
            self.queue.pop_front();
            self.dropped += 1;
        }
        self.queue.push_back(Entry {
            at,
            y: position.y,
            x: position.x,
            z,
            cause,
            sequence,
        });
    }

    /// How many records wait for a report.
    #[must_use]
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// Records dropped for the queue bound since the last call.
    pub fn take_dropped(&mut self) -> u64 {
        std::mem::take(&mut self.dropped)
    }

    /// The report due at `now`: the queued records, oldest first and at
    /// most [`REPORT_CAP`], once the oldest is [`REPORT_AGE`] old, or at
    /// once when `forced` (the zone change's flush, 0x4ab056). None when
    /// nothing is queued or nothing is due. The records stay queued until
    /// [`Self::reported`] says the report went out.
    #[must_use]
    pub fn report(&self, now: Instant, forced: bool) -> Option<(EncodedCommand, usize)> {
        let oldest = self.queue.front()?;
        if !forced && now.saturating_duration_since(oldest.at) < REPORT_AGE {
            return None;
        }
        let count = self.queue.len().min(REPORT_CAP);
        let mut body = Vec::with_capacity(count * RECORD_LEN + 1);
        for entry in self.queue.iter().take(count) {
            entry.write(&mut body);
        }
        // The program leaves the byte after the records unwritten (0x4b6f1f
        // returns 17 x count + 1); this client writes 0 there (inferred).
        body.push(0);
        Some((
            EncodedCommand {
                opcode: OPCODE,
                body,
            },
            count,
        ))
    }

    /// The first `count` records went out in a report.
    pub fn reported(&mut self, count: usize) {
        self.queue.drain(..count.min(self.queue.len()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32) -> Position {
        Position {
            x,
            y: -147.0,
            z: 18.97,
            heading: 0.0,
        }
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

    #[test]
    fn an_update_record_is_lifted_and_keeps_its_sequence_and_axes() {
        let start = Instant::now();
        let mut recorder = Recorder::new();
        recorder.record(causes::UPDATE, at(18.0), 463, start);
        let (report, count) = recorder.report(start + REPORT_AGE, false).unwrap();
        assert_eq!(count, 1);
        let [record] = records(&report)[..] else {
            panic!("one record")
        };
        // Wire order Y, X, Z as in the captured reports.
        assert_eq!([record.y, record.x], [-147.0, 18.0]);
        assert!((record.z - (18.97 + 1.9)).abs() < 1e-4);
        assert_eq!(record.cause, Cause::TeleportB);
        assert_eq!(record.sequence, 463);
    }

    #[test]
    fn a_landing_is_not_lifted() {
        let start = Instant::now();
        let mut recorder = Recorder::new();
        recorder.record(causes::LANDING, at(5.0), 9, start);
        let (report, _) = recorder.report(start, true).unwrap();
        assert!((records(&report)[0].z - 18.97).abs() < 1e-6);
        assert_eq!(records(&report)[0].cause, Cause::TeleportA);
    }

    #[test]
    fn a_frame_where_the_last_frame_was_is_consumed_unwritten() {
        let start = Instant::now();
        let mut recorder = Recorder::new();
        recorder.record(causes::FRAME, at(1.0), 1, start);
        recorder.record(causes::FRAME, at(1.0), 2, start);
        // An update between does not end the run of the same place.
        recorder.record(causes::UPDATE, at(1.0), 3, start);
        recorder.record(causes::FRAME, at(1.0), 4, start);
        recorder.record(causes::FRAME, at(2.0), 5, start);
        let (report, count) = recorder.report(start, true).unwrap();
        assert_eq!(count, 3);
        let sequences: Vec<_> = records(&report).iter().map(|r| r.sequence).collect();
        assert_eq!(sequences, [1, 3, 5]);
    }

    #[test]
    fn a_report_waits_for_its_oldest_record_to_age_unless_forced() {
        let start = Instant::now();
        let mut recorder = Recorder::new();
        assert!(recorder.report(start, true).is_none(), "never empty");
        recorder.record(causes::UPDATE, at(1.0), 0, start);
        assert!(recorder
            .report(start + Duration::from_millis(999), false)
            .is_none());
        assert!(recorder
            .report(start + Duration::from_millis(999), true)
            .is_some());
        let (_, count) = recorder.report(start + REPORT_AGE, false).unwrap();
        recorder.reported(count);
        assert_eq!(recorder.queued(), 0);
        assert!(recorder.report(start + REPORT_AGE * 5, true).is_none());
    }

    #[test]
    fn one_report_carries_at_most_ten_thousand_and_the_rest_wait() {
        let start = Instant::now();
        let mut recorder = Recorder::new();
        for i in 0..10_005u16 {
            recorder.record(causes::FRAME, at(f32::from(i)), i, start);
        }
        let (report, count) = recorder.report(start + REPORT_AGE, false).unwrap();
        assert_eq!(count, REPORT_CAP);
        assert_eq!(report.body.len(), REPORT_CAP * RECORD_LEN + 1);
        recorder.reported(count);
        assert_eq!(recorder.queued(), 5);
        let (report, _) = recorder.report(start + REPORT_AGE, false).unwrap();
        assert_eq!(records(&report)[0].sequence, 10_000);
    }

    #[test]
    fn a_stalled_queue_drops_its_oldest_beyond_the_bound_and_counts_them() {
        let start = Instant::now();
        let mut recorder = Recorder::new();
        let extra = 3;
        for i in 0..QUEUE_BOUND + extra {
            let sequence = u16::try_from(i & 0xffff).unwrap();
            recorder.record(causes::UPDATE, at(1.0), sequence, start);
        }
        assert_eq!(recorder.queued(), QUEUE_BOUND);
        assert_eq!(recorder.take_dropped(), 3);
        assert_eq!(recorder.take_dropped(), 0);
        let (report, _) = recorder.report(start, true).unwrap();
        assert_eq!(records(&report)[0].sequence, 3);
    }

    #[test]
    fn the_clock_word_is_the_low_sixteen_bits_of_game_milliseconds() {
        let start = Instant::now();
        let mut recorder = Recorder::new();
        recorder.record(causes::UPDATE, at(1.0), 0, start);
        recorder.record(
            causes::UPDATE,
            at(1.0),
            1,
            start + Duration::from_millis(1234),
        );
        let (report, _) = recorder.report(start, true).unwrap();
        let clocks: Vec<_> = records(&report).iter().map(|r| r.timestamp_ms).collect();
        assert_eq!(clocks[1].wrapping_sub(clocks[0]), 1234);
    }

    #[test]
    fn distinguishes_axes_clock_and_sequence() {
        let record = Record::decode(&[
            0x00, 0x00, 0x00, 0x40, // Y = 2
            0x00, 0x00, 0x80, 0xbf, // X = -1
            0x00, 0x00, 0x40, 0x40, // Z = 3
            4, 0xfe, 0xff, 0x34, 0x12,
        ])
        .unwrap();
        assert_eq!([record.y, record.x, record.z], [2.0, -1.0, 3.0]);
        assert_eq!(record.cause, Cause::ZoneLine);
        assert_eq!(record.timestamp_ms, 65534);
        assert_eq!(record.sequence, 0x1234);
    }

    #[test]
    fn rejects_partial_records_and_packet_trailers() {
        for length in 0..=35 {
            assert_eq!(
                Record::decode(&vec![0; length]).is_ok(),
                length == RECORD_LEN
            );
        }
    }

    #[test]
    fn preserves_unknown_causes_and_non_finite_coordinates() {
        let mut bytes = [0; RECORD_LEN];
        bytes[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
        for cause in 0..=255 {
            bytes[12] = cause;
            let record = Record::decode(&bytes).unwrap();
            assert!(record.y.is_nan());
            let expected = match cause {
                1 => Cause::Collision,
                2 => Cause::TeleportB,
                3 => Cause::TeleportA,
                4 => Cause::ZoneLine,
                other => Cause::Unknown(other),
            };
            assert_eq!(record.cause, expected);
        }
    }
}
