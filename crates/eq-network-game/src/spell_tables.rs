//! P99 spell registration and replay records, independent of installed data.
//!
//! Registration names source spell IDs to snapshot; replay names destination
//! and cached source IDs. Neither packet carries numeric definition values.
//! The official-client captures show two-byte registration entries and
//! four-byte replay entries behind a four-byte count; that the replay pairs
//! read destination first, then the cached source, is an inference from
//! that layout (recording checklist 6.375), to be confirmed on a server that
//! sends a table whose effect is visible. The 10,000-entry registry is
//! initialized by the program at 0x4c7481.
//! A host owns its installed definitions, snapshots and selective projection;
//! decoding these records never changes a spell or an installed file.
use serde::Serialize;
use std::fmt;

/// Which installed-spell provider the selected network policy requires.
/// This is available before any packets or profile are received.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum ProviderPolicy {
    /// Use installed definitions without source registration or replay.
    Installed,
    /// Cache first-current sources and apply ordered zone replay tables.
    Project1999,
}

impl ProviderPolicy {
    /// Whether installed spell mechanics require server registration and replay.
    /// Hosts use this contract to gate table lifecycle handling and readiness.
    #[must_use]
    pub const fn requires_server_tables(self) -> bool {
        match self {
            Self::Installed => false,
            Self::Project1999 => true,
        }
    }
}

/// World registration of source spell IDs.
pub const REGISTRATION_OPCODE: u16 = 0x1235;
/// Zone replay of cached sources into destination spell IDs.
pub const REPLAY_OPCODE: u16 = 0x1234;
/// The native registry's number of addressable records; valid IDs are below it.
pub const RECORD_COUNT: u16 = 10_000;

/// A spell index inside the native registry, including its zero entry.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SpellId(u16);

impl SpellId {
    /// An index below [`RECORD_COUNT`], or `None` when it is outside the registry.
    #[must_use]
    pub const fn new(value: u16) -> Option<Self> {
        if value < RECORD_COUNT {
            Some(Self(value))
        } else {
            None
        }
    }

    /// The source or destination ID carried on the wire.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Which spell-table message a packet carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum Kind {
    /// IDs whose current definitions are snapshotted on first sight.
    Registration,
    /// Ordered assignments from cached source IDs to destination IDs.
    Replay,
}

impl Kind {
    /// The table kind for a recognized opcode, or `None` for another packet.
    #[must_use]
    pub const fn from_opcode(opcode: u16) -> Option<Self> {
        match opcode {
            REGISTRATION_OPCODE => Some(Self::Registration),
            REPLAY_OPCODE => Some(Self::Replay),
            _ => None,
        }
    }

    const fn width(self) -> usize {
        match self {
            Self::Registration => 2,
            Self::Replay => 4,
        }
    }
}

/// One assignment. Its source is a snapshot, never another live destination lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Assignment {
    /// The destination registry slot; it remains the effective record's ID.
    pub destination: SpellId,
    /// The previously snapshotted source, whose copyable fields are replayed.
    pub source: SpellId,
}

impl Assignment {
    /// An assignment between two validated spell IDs.
    #[must_use]
    pub const fn new(destination: SpellId, source: SpellId) -> Self {
        Self {
            destination,
            source,
        }
    }
}

/// A fully decoded table. Entry order and duplicates are retained.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum Table {
    /// Snapshot each first-seen source's current effective definition.
    Registration(Vec<SpellId>),
    /// Replay each source's cached projection, preserving destination text
    /// and other excluded fields. A repeated destination is written in order.
    Replay(Vec<Assignment>),
}

/// Which index in an entry was outside the native registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum IdRole {
    /// The cache source, also the only ID in a registration entry.
    Source,
    /// A replay's destination.
    Destination,
}

/// Why an entire table cannot be decoded. No valid prefix is returned on error.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum DecodeError {
    /// The four-byte count is incomplete.
    MissingCount,
    /// Count arithmetic cannot be represented on this platform.
    CountOverflow,
    /// The body is short or has uncounted trailing data.
    Length {
        /// Exact body length implied by the count and entry width.
        expected: usize,
        /// Received body length.
        actual: usize,
    },
    /// An ID is not inside the known registry.
    InvalidId {
        /// The zero-based entry that is invalid.
        entry: usize,
        /// Whether the source or destination ID was invalid.
        role: IdRole,
        /// The observed index, retained for diagnostics.
        value: u16,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCount => formatter.write_str("spell table has no complete count"),
            Self::CountOverflow => {
                formatter.write_str("spell table count exceeds addressable size")
            }
            Self::Length { expected, actual } => {
                write!(
                    formatter,
                    "spell table length {actual}, expected {expected}"
                )
            }
            Self::InvalidId { entry, role, value } => {
                write!(
                    formatter,
                    "spell table entry {entry} has invalid {role:?} ID {value}"
                )
            }
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decode a registration or replay packet; other opcodes return `None`.
///
/// Counts are variable. Registration uses `4 + count*2` bytes and replay
/// `4 + count*4`, with a little-endian count and little-endian IDs. Zero counts
/// and duplicate IDs are valid; all entries and their order are preserved.
///
/// # Errors
/// Rejects an incomplete count, overflow, length mismatch or index outside
/// [`RECORD_COUNT`]. An error invalidates the whole table, never just a suffix.
/// Missing installed definitions and unregistered sources are host concerns.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<Table>, DecodeError> {
    let Some(kind) = Kind::from_opcode(opcode) else {
        return Ok(None);
    };
    let count = body.get(..4).ok_or(DecodeError::MissingCount)?;
    let count = u32::from_le_bytes(count.try_into().map_err(|_| DecodeError::MissingCount)?);
    let expected = usize::try_from(count)
        .ok()
        .and_then(|count| count.checked_mul(kind.width()))
        .and_then(|bytes| bytes.checked_add(4))
        .ok_or(DecodeError::CountOverflow)?;
    if body.len() != expected {
        return Err(DecodeError::Length {
            expected,
            actual: body.len(),
        });
    }
    let id = |bytes: &[u8], entry, role| {
        let value = u16::from_le_bytes([bytes[0], bytes[1]]);
        SpellId::new(value).ok_or(DecodeError::InvalidId { entry, role, value })
    };
    // Check every index before allocating the output, including a bad suffix.
    for (entry, bytes) in body[4..].chunks_exact(kind.width()).enumerate() {
        match kind {
            Kind::Registration => {
                id(bytes, entry, IdRole::Source)?;
            }
            Kind::Replay => {
                id(&bytes[..2], entry, IdRole::Destination)?;
                id(&bytes[2..], entry, IdRole::Source)?;
            }
        }
    }
    // The immutable body has passed both complete-shape and domain checks.
    let validated_id = |bytes: &[u8]| SpellId(u16::from_le_bytes([bytes[0], bytes[1]]));
    Ok(Some(match kind {
        Kind::Registration => Table::Registration(
            body[4..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|bytes| validated_id(bytes))
                .collect(),
        ),
        Kind::Replay => Table::Replay(
            body[4..]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|bytes| Assignment::new(validated_id(&bytes[..2]), validated_id(&bytes[2..])))
                .collect(),
        ),
    }))
}

/// Where a table arrived, including the admission identity before a profile exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum Stage {
    /// A world connection; its identity does not define a provider reset.
    World,
    /// A zone connection, before or after admission.
    Zone {
        /// The same identity its eventual `WorldEvent::Entered` carries.
        session_id: u64,
    },
}

/// Network ordering context, distinct from the host's installed-provider epoch.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Context {
    /// One network connection attempt, shared by its world and zone transports.
    /// A reconnect may change this without replacing the host's provider cache.
    pub run_id: u64,
    /// Monotonically increasing table-event number within the attempt, from one.
    /// Malformed tables consume an event number too.
    pub revision: u64,
    /// World or the current zone's admission identity.
    pub stage: Stage,
}

impl Context {
    /// Network context for a table event. The producer owns ordering; a host
    /// binds the event stream to its own provider epoch before applying data.
    #[must_use]
    pub const fn new(run_id: u64, revision: u64, stage: Stage) -> Self {
        Self {
            run_id,
            revision,
            stage,
        }
    }
}

/// Why a recognized packet leaves the host's spell overlay unavailable.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum Unavailable {
    /// The complete table failed validation; no entries were delivered.
    Malformed(DecodeError),
    /// Registration arrived outside the world, or replay outside a zone.
    UnexpectedStage,
}

/// A table or an explicit failure. Hosts must not silently use stale base data
/// after an unavailable update or partially apply a failed batch.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub enum Update {
    /// One complete server table, delivered in receive order.
    Table(Table),
    /// The server table cannot be used; the host must surface unavailability.
    Unavailable {
        /// Which table failed.
        kind: Kind,
        /// Typed failure without a raw packet body.
        reason: Unavailable,
    },
}

/// A P99-policy table event delivered directly before ordinary admission gates.
/// The host owns typed snapshots, the selective numeric projection and its
/// provider lifetime; this event grants no spell, cast or movement permission.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Event {
    /// The event's receive order and world/zone context.
    pub context: Context,
    /// A complete table or visible failure.
    pub update: Update,
}

impl Event {
    /// A table event with explicit network ordering context.
    #[must_use]
    pub const fn new(context: Context, update: Update) -> Self {
        Self { context, update }
    }
}

#[cfg(test)]
mod tests;
