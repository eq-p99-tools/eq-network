//! What the session drops or does not understand of the servers' messages,
//! as records a host can count and keep ([`ClientEvent::Gap`]).
//!
//! Every place the session leaves a server message unread, or cannot read
//! one, counts it here, beside the diagnostic line it has always written.
//! The first of each kind of gap in a stage is told at once; repeats are
//! counted and told at most every [`INTERVAL`] and when the stage ends, so
//! a host adds the counts up into a list of what is left to read. Only the
//! first record of each kind of gap in a session carries a sample of the
//! body, and only at the zone: nothing of the login exchange or the
//! world's (the account, the character list) is ever sampled. No record
//! names a host, an address, the account or a character.
//!
//! [`ClientEvent::Gap`]: super::ClientEvent::Gap

use super::ServerProtocol;
use eq_network_game::GameDialect;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    fmt,
    time::{Duration, Instant},
};

/// The most body bytes a sample keeps.
pub const SAMPLE_BYTES: usize = 256;

/// The least time between two records of the same gap's repeats.
pub const INTERVAL: Duration = Duration::from_secs(10);

/// The most distinct kinds of [`GapKind::UnknownKind`] a stage counts
/// apart; a server sending more has the rest counted under
/// [`MORE_KINDS`], so its records stay bounded.
pub const MOST_KINDS: usize = 4096;

/// The kind the rest are counted under, past [`MOST_KINDS`].
pub const MORE_KINDS: u32 = u32::MAX;

/// A server message the session dropped or could not read.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Gap {
    /// The stage of the session that received it.
    pub stage: GapStage,
    /// The client generation the server speaks.
    pub dialect: GameDialect,
    /// The configured server type.
    pub server: ServerProtocol,
    /// The short zone name: empty at the login and the world, and at a
    /// zone until that zone has named itself.
    pub zone: String,
    /// The message's opcode on the wire.
    pub opcode: u16,
    /// Its name in the generation's table of what the servers send, when
    /// the table lists it.
    pub name: Option<&'static str>,
    /// How many arrivals this record counts: one for the first of its
    /// kind in a stage, then the repeats since the last record.
    pub count: u64,
    /// The shortest body among them, in bytes.
    pub min_length: usize,
    /// The longest.
    pub max_length: usize,
    /// What was left unread, and why.
    pub kind: GapKind,
    /// The first [`SAMPLE_BYTES`] of the body, hex-encoded: on the first
    /// record of this gap in the session, and at the zone only. It may
    /// hold what other players said or their names, so a host keeps it
    /// local.
    pub sample: Option<String>,
}

impl Gap {
    /// What tells this gap from another in one session: the stage, the
    /// opcode and the kind, without its free text.
    #[must_use]
    pub fn key(&self) -> (GapStage, u16, String) {
        (self.stage, self.opcode, self.kind.key())
    }
}

/// The stage of a session that received a message.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GapStage {
    /// The login server's exchange.
    #[default]
    Login,
    /// The world server's.
    World,
    /// A zone's.
    Zone,
}

impl fmt::Display for GapStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Login => "login",
            Self::World => "world",
            Self::Zone => "zone",
        })
    }
}

/// What the session left unread of a message, and why.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GapKind {
    /// The generation's table does not list it: no server is known to send
    /// it, and nothing reads it.
    Unlisted,
    /// The table lists it and nothing reads it yet, for this reason.
    Unread {
        /// Why it is left unread.
        reason: String,
    },
    /// The table lists it as read only at another stage.
    ReadElsewhere,
    /// The table lists it as holding nothing worth reading, for this
    /// reason: counted, though it is no gap.
    Skipped {
        /// Why nothing in it needs reading.
        reason: String,
    },
    /// It is read, all but these parts, which still wait for work to come.
    PartlyRead {
        /// The parts left unread.
        parts: Vec<&'static str>,
    },
    /// A kind of the message no server is known to send.
    UnknownKind {
        /// The kind, or [`MORE_KINDS`] for those past [`MOST_KINDS`] in a
        /// stage.
        kind: u32,
    },
    /// It could not be read.
    Unreadable {
        /// What it was about.
        part: String,
        /// Why it could not be read.
        error: String,
    },
}

impl GapKind {
    /// Whether it is a gap to close, which a skipped message is not.
    #[must_use]
    pub const fn is_gap(&self) -> bool {
        match self {
            Self::Skipped { .. } => false,
            Self::Unlisted
            | Self::Unread { .. }
            | Self::ReadElsewhere
            | Self::PartlyRead { .. }
            | Self::UnknownKind { .. }
            | Self::Unreadable { .. } => true,
        }
    }

    /// Its kind without free text that changes from one record to the
    /// next, such as a decoder's error.
    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Self::Unlisted => "unlisted".to_owned(),
            Self::Unread { .. } => "unread".to_owned(),
            Self::ReadElsewhere => "read_elsewhere".to_owned(),
            Self::Skipped { .. } => "skipped".to_owned(),
            Self::PartlyRead { .. } => "partly_read".to_owned(),
            Self::UnknownKind { kind } => format!("unknown_kind:{kind}"),
            Self::Unreadable { part, .. } => format!("unreadable:{part}"),
        }
    }
}

/// What a session has counted of the gaps in the stage under way.
#[derive(Debug, Default)]
pub(super) struct Tally {
    /// The stage under way.
    pub(super) stage: GapStage,
    /// Whether the zone under way has named itself.
    pub(super) zone_named: bool,
    /// The distinct kinds of [`GapKind::UnknownKind`] counted in the stage,
    /// past which the rest are counted as one ([`MOST_KINDS`]).
    kinds: usize,
    /// Each gap's last record, and the repeats counted since.
    seen: HashMap<(GapStage, u16, String), Seen>,
    /// The gaps already sampled in the session.
    sampled: HashSet<(GapStage, u16, String)>,
    /// When the next repeats fall due, when any wait.
    next_due: Option<Instant>,
}

/// A gap's last record, and the repeats counted since.
#[derive(Debug)]
struct Seen {
    told: Instant,
    repeats: Option<Gap>,
}

impl Tally {
    /// Counts one arrival of `gap` at `at`, returning the record to tell:
    /// the first of its kind in the stage, with its sample where one is
    /// kept, or the repeats once [`INTERVAL`] has passed since the last.
    pub(super) fn count(&mut self, mut gap: Gap, body: &[u8], at: Instant) -> Option<Gap> {
        if matches!(gap.kind, GapKind::UnknownKind { .. }) && !self.seen.contains_key(&gap.key()) {
            if self.kinds >= MOST_KINDS {
                gap.kind = GapKind::UnknownKind { kind: MORE_KINDS };
            } else {
                self.kinds += 1;
            }
        }
        let key = gap.key();
        let Some(seen) = self.seen.get_mut(&key) else {
            gap.sample = self.sample(&gap, body);
            self.seen.insert(
                key,
                Seen {
                    told: at,
                    repeats: None,
                },
            );
            return Some(gap);
        };
        match seen.repeats.as_mut() {
            Some(repeats) => {
                repeats.count += 1;
                repeats.min_length = repeats.min_length.min(gap.min_length);
                repeats.max_length = repeats.max_length.max(gap.max_length);
            }
            None => seen.repeats = Some(gap),
        }
        if at.duration_since(seen.told) >= INTERVAL {
            seen.told = at;
            return seen.repeats.take();
        }
        let due = seen.told + INTERVAL;
        self.next_due = Some(self.next_due.map_or(due, |next| next.min(due)));
        None
    }

    /// The repeats due by `at`, each told at most every [`INTERVAL`].
    pub(super) fn due(&mut self, at: Instant) -> Vec<Gap> {
        if self.next_due.is_none_or(|due| at < due) {
            return Vec::new();
        }
        self.next_due = None;
        let mut due = Vec::new();
        for seen in self.seen.values_mut() {
            if seen.repeats.is_none() {
                continue;
            }
            let next = seen.told + INTERVAL;
            if at >= next {
                seen.told = at;
                due.extend(seen.repeats.take());
            } else {
                self.next_due = Some(self.next_due.map_or(next, |due| due.min(next)));
            }
        }
        due
    }

    /// Every repeat still to tell, as the stage ends; the next stage counts
    /// its gaps afresh, but samples each only once a session.
    pub(super) fn drain(&mut self) -> Vec<Gap> {
        self.next_due = None;
        self.kinds = 0;
        let mut left: Vec<Gap> = self
            .seen
            .drain()
            .filter_map(|(_, seen)| seen.repeats)
            .collect();
        left.sort_by_key(Gap::key);
        left
    }

    /// A sample of `body` for the first gap of its key in the session, at
    /// the zone only.
    fn sample(&mut self, gap: &Gap, body: &[u8]) -> Option<String> {
        match gap.stage {
            GapStage::Login | GapStage::World => None,
            GapStage::Zone => self
                .sampled
                .insert(gap.key())
                .then(|| hex::encode(&body[..body.len().min(SAMPLE_BYTES)])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gap(stage: GapStage, kind: GapKind, length: usize) -> Gap {
        Gap {
            stage,
            dialect: GameDialect::Titanium,
            server: ServerProtocol::EqEmu,
            zone: String::new(),
            opcode: 0x1234,
            name: None,
            count: 1,
            min_length: length,
            max_length: length,
            kind,
            sample: None,
        }
    }

    #[test]
    fn only_the_zone_s_first_gap_of_a_kind_is_sampled() {
        let mut tally = Tally::default();
        let at = Instant::now();
        let body = [1, 2, 3, 4];
        for stage in [GapStage::Login, GapStage::World] {
            let told = tally
                .count(gap(stage, GapKind::Unlisted, 4), &body, at)
                .unwrap();
            assert_eq!(told.sample, None);
        }
        let told = tally
            .count(gap(GapStage::Zone, GapKind::Unlisted, 4), &body, at)
            .unwrap();
        assert_eq!(told.sample.as_deref(), Some("01020304"));
        let unreadable = |error: &str| {
            gap(
                GapStage::Zone,
                GapKind::Unreadable {
                    part: "world".into(),
                    error: error.into(),
                },
                4,
            )
        };
        assert!(tally
            .count(unreadable("short"), &body, at)
            .unwrap()
            .sample
            .is_some());
        // Another error of the same part is the same gap.
        assert!(tally.count(unreadable("other"), &body, at).is_none());
        // The next zone counts afresh, but samples nothing twice.
        assert_eq!(tally.drain().len(), 1);
        let again = tally
            .count(gap(GapStage::Zone, GapKind::Unlisted, 4), &body, at)
            .unwrap();
        assert_eq!(again.sample, None);
    }

    #[test]
    fn a_sample_keeps_at_most_its_limit() {
        let mut tally = Tally::default();
        let body = vec![0xab; SAMPLE_BYTES * 2];
        let told = tally
            .count(
                gap(GapStage::Zone, GapKind::Unlisted, body.len()),
                &body,
                Instant::now(),
            )
            .unwrap();
        assert_eq!(told.sample.unwrap().len(), SAMPLE_BYTES * 2);
    }

    #[test]
    fn a_thousand_arrivals_are_told_in_a_handful_of_records_that_count_them_all() {
        let mut tally = Tally::default();
        let start = Instant::now();
        let mut told = Vec::new();
        for arrival in 0..1000u64 {
            // A hundred a second, for ten seconds.
            let at = start + Duration::from_millis(arrival * 10);
            let length = usize::try_from(arrival % 7).unwrap();
            told.extend(tally.count(
                gap(GapStage::Zone, GapKind::Unlisted, length),
                &[0; 8][..length],
                at,
            ));
            told.extend(tally.due(at));
        }
        told.extend(tally.drain());
        assert!(told.len() <= 3, "{} records", told.len());
        assert_eq!(told.iter().map(|gap| gap.count).sum::<u64>(), 1000);
        assert_eq!(told.iter().map(|gap| gap.min_length).min(), Some(0));
        assert_eq!(told.iter().map(|gap| gap.max_length).max(), Some(6));
        assert!(told[1..].iter().all(|gap| gap.sample.is_none()));
    }

    #[test]
    fn repeats_wait_their_interval_and_the_stage_s_end() {
        let mut tally = Tally::default();
        let start = Instant::now();
        let first = gap(GapStage::Zone, GapKind::Unlisted, 2);
        assert!(tally.count(first.clone(), &[0, 0], start).is_some());
        assert!(tally.count(first.clone(), &[0, 0], start).is_none());
        assert_eq!(tally.due(start + Duration::from_secs(9)), []);
        let due = tally.due(start + INTERVAL);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].count, 1);
        assert_eq!(tally.due(start + INTERVAL * 3), []);
        // A repeat long after the last record is told at once.
        assert_eq!(
            tally
                .count(first.clone(), &[0, 0], start + INTERVAL * 3)
                .map(|gap| gap.count),
            Some(1)
        );
        assert!(tally.count(first, &[0, 0], start + INTERVAL * 3).is_none());
        let left = tally.drain();
        assert_eq!(left.len(), 1);
        assert_eq!(tally.drain(), []);
    }

    #[test]
    fn a_server_s_endless_kinds_are_counted_apart_only_up_to_a_bound() {
        let mut tally = Tally::default();
        let at = Instant::now();
        let mut told = Vec::new();
        for kind in 0..u32::try_from(MOST_KINDS + 100).unwrap() {
            let unknown = gap(GapStage::Zone, GapKind::UnknownKind { kind }, 1);
            told.extend(tally.count(unknown, &[], at));
        }
        told.extend(tally.drain());
        // Each kind apart once, then the rest's first record and their repeats.
        assert_eq!(told.len(), MOST_KINDS + 2);
        let more: Vec<_> = told
            .iter()
            .filter(|gap| gap.kind == GapKind::UnknownKind { kind: MORE_KINDS })
            .collect();
        assert_eq!(more.iter().map(|gap| gap.count).sum::<u64>(), 100);
        // A kind counted apart stays apart past the bound.
        let mut tally = Tally::default();
        for kind in 0..u32::try_from(MOST_KINDS).unwrap() {
            tally.count(
                gap(GapStage::Zone, GapKind::UnknownKind { kind }, 1),
                &[],
                at,
            );
        }
        tally.count(
            gap(GapStage::Zone, GapKind::UnknownKind { kind: 0 }, 1),
            &[],
            at,
        );
        assert_eq!(tally.drain()[0].kind, GapKind::UnknownKind { kind: 0 });
    }

    #[test]
    fn a_skipped_message_is_counted_but_no_gap() {
        assert!(!GapKind::Skipped { reason: "x".into() }.is_gap());
        assert!(GapKind::Unlisted.is_gap());
        assert!(GapKind::PartlyRead { parts: vec!["x"] }.is_gap());
    }

    #[test]
    fn a_gap_serializes_flat_with_its_kind() {
        let mut record = gap(GapStage::Zone, GapKind::UnknownKind { kind: 7 }, 4);
        record.zone = "qeynos".into();
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(value["stage"], "zone");
        assert_eq!(value["dialect"], "titanium");
        assert_eq!(value["server"], "eq_emu");
        assert_eq!(value["kind"]["type"], "unknown_kind");
        assert_eq!(value["kind"]["kind"], 7);
        assert_eq!(value["opcode"], 0x1234);
        assert_eq!(value["count"], 1);
    }
}
