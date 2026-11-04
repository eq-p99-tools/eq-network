//! Explicit character deletion at world selection, with observation-only results.
use crate::command::EncodedCommand;
use serde::Serialize;
use std::fmt;

/// The deletion capability offered for one character list by its server policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Rules {
    /// Titanium's native character-name request.
    Titanium,
    /// `EQMac`'s native character-name request, in this library's legacy notation.
    EqMac,
}

/// Connection and list that authorize a deletion intent.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Scope {
    /// Process-monotonic world-connection generation, never a wire field.
    pub connection_id: u64,
    /// Identity of this connection's authoritative character list.
    pub selection_id: u64,
}

/// A named, explicitly confirmed intent. Correlation identifiers are local only.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Request {
    scope: Scope,
    #[serde(rename = "request_id")]
    id: u64,
    name: String,
}

/// Why an explicit deletion intent cannot be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidRequest {
    /// No world connection has supplied this scope.
    InvalidScope,
    /// The caller must assign a positive local correlation identifier.
    InvalidId,
    /// Names must fit a native 64-byte name field as nonempty ASCII plus NUL.
    InvalidName,
    /// Confirmation must repeat the exact requested name.
    ConfirmationMismatch,
}

impl fmt::Display for InvalidRequest {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(match self {
            Self::InvalidScope => "Deletion needs a current world-connection scope",
            Self::InvalidId => "Deletion needs a positive request identifier",
            Self::InvalidName => "Deletion needs a listed ASCII name of 1 to 63 bytes",
            Self::ConfirmationMismatch => "Deletion confirmation must repeat the exact name",
        })
    }
}

impl std::error::Error for InvalidRequest {}

impl Request {
    /// Capture an explicit confirmation, without selecting or sending anything.
    ///
    /// # Errors
    /// Rejects an invalid local id, an unencodable name or a different confirmation.
    pub fn new(
        scope: Scope,
        request_id: u64,
        name: &str,
        confirmation: &str,
    ) -> Result<Self, InvalidRequest> {
        if scope.connection_id == 0 {
            return Err(InvalidRequest::InvalidScope);
        }
        if request_id == 0 {
            return Err(InvalidRequest::InvalidId);
        }
        if name.is_empty() || name.len() >= 64 || !name.is_ascii() || name.as_bytes().contains(&0) {
            return Err(InvalidRequest::InvalidName);
        }
        if name != confirmation {
            return Err(InvalidRequest::ConfirmationMismatch);
        }
        Ok(Self {
            scope,
            id: request_id,
            name: name.to_owned(),
        })
    }

    /// The exact list from which this identity was confirmed.
    #[must_use]
    pub const fn scope(&self) -> Scope {
        self.scope
    }
    /// Caller-assigned correlation id, never serialized to the server.
    #[must_use]
    pub const fn request_id(&self) -> u64 {
        self.id
    }
    /// The exact confirmed server spelling.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Rules {
    /// Encode only the native NUL-terminated name; the selection owner authorizes sends.
    #[must_use]
    pub fn encode(self, request: &Request) -> EncodedCommand {
        let mut body = request.name.as_bytes().to_vec();
        body.push(0);
        EncodedCommand {
            opcode: match self {
                Self::Titanium => 0x26c9,
                Self::EqMac => 0x5a40,
            },
            body,
        }
    }
}

/// A local rejection before a new deletion request reaches transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Refusal {
    /// This server policy offers no deletion capability.
    Unsupported,
    /// The confirmation names an obsolete list or world connection.
    StaleList,
    /// The confirmation belongs to an earlier world connection.
    StaleConnection,
    /// The exact name is absent from the current authoritative list.
    NotListed,
    /// Creation, entry or another unresolved deletion owns selection.
    Busy,
    /// The local request id has already been used by another intent.
    Duplicate,
    /// The command reached a stage other than character selection.
    NotAtSelection,
    /// The frontend could not enqueue the intent; no transport attempt was made.
    QueueUnavailable,
}

/// Why a submitted request has no conclusive local completion observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Unconfirmed {
    /// The local observation deadline expired; this authorizes no retry.
    TimedOut,
    /// The world connection ended before an absence observation.
    ConnectionEnded,
    /// Transport failed while submitting; delivery is uncertain.
    SendFailed,
}

/// Presence in a particular authoritative character-list snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Presence {
    /// The identity is listed; this is not a correlated server refusal.
    Present,
    /// The identity is absent; this is not a correlated deletion ACK.
    Absent,
}

/// A received list observation, never proof of which operation caused it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Observation {
    /// The fresh list to which this observation belongs.
    pub scope: Scope,
    /// Whether the exact confirmed name occurs in that list.
    pub presence: Presence,
}

/// Local lifecycle and received-list evidence for one explicit request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum Progress {
    /// No new packet was submitted for this intent.
    NotSent(Refusal),
    /// Submitted, with optional evidence that still leaves the request unresolved.
    Pending(Option<Observation>),
    /// No causal result is available; explicit attention is required, never retry.
    Unconfirmed {
        /// The local event that left the submitted request unresolved.
        reason: Unconfirmed,
        /// Most recent received-list evidence, if any.
        observation: Option<Observation>,
    },
    /// The identity was absent in this refreshed list; not a protocol deletion ACK.
    ObservedAbsent {
        /// Exact connection and refreshed list in which the name was absent.
        scope: Scope,
    },
}

impl Progress {
    /// Whether selection must remain held against another mutation or entry.
    #[must_use]
    pub const fn unresolved(&self) -> bool {
        matches!(self, Self::Pending(_) | Self::Unconfirmed { .. })
    }

    /// The specific received-list observation, where one is available.
    #[must_use]
    pub const fn observation(&self) -> Option<Observation> {
        match self {
            Self::Pending(observation) | Self::Unconfirmed { observation, .. } => *observation,
            Self::ObservedAbsent { scope } => Some(Observation {
                scope: *scope,
                presence: Presence::Absent,
            }),
            Self::NotSent(_) => None,
        }
    }
}

/// A typed result keyed to the full local intent, separate from server acceptance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Update {
    /// The validated intent, including its original list and local correlation id.
    pub request: Request,
    /// Local progress and explicitly scoped list observations.
    pub progress: Progress,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_generations_send_only_the_confirmed_name_and_terminator() {
        let request = Request::new(
            Scope {
                connection_id: 7,
                selection_id: 1,
            },
            8,
            "Disposable",
            "Disposable",
        )
        .unwrap();
        for (rules, opcode) in [(Rules::Titanium, 0x26c9), (Rules::EqMac, 0x5a40)] {
            let packet = rules.encode(&request);
            assert_eq!(packet.opcode, opcode);
            assert_eq!(packet.body, b"Disposable\0");
            assert_eq!(packet.body.len(), request.name().len() + 1);
        }
        assert_eq!(
            Rules::EqMac.encode(&request).opcode.to_be_bytes(),
            [0x5a, 0x40]
        );
        assert_eq!(0x405a_u16.to_le_bytes(), [0x5a, 0x40]);
        let report = serde_json::to_value(&request).unwrap();
        assert_eq!(report["request_id"], 8);
        assert_eq!(report["scope"]["connection_id"], 7);
        assert!(
            report.get("id").is_none(),
            "internal naming must not change the report contract"
        );
    }

    #[test]
    fn identity_confirmation_and_native_name_bounds_are_explicit() {
        let scope = Scope {
            connection_id: 1,
            selection_id: 1,
        };
        assert_eq!(
            Request::new(scope, 1, "Disposable", "Other"),
            Err(InvalidRequest::ConfirmationMismatch)
        );
        assert_eq!(
            Request::new(scope, 1, "Disposable", "disposable"),
            Err(InvalidRequest::ConfirmationMismatch)
        );
        assert_eq!(
            Request::new(scope, 0, "Disposable", "Disposable"),
            Err(InvalidRequest::InvalidId)
        );
        for name in ["", "a\0b", "é", &"a".repeat(64)] {
            assert_eq!(
                Request::new(scope, 1, name, name),
                Err(InvalidRequest::InvalidName)
            );
        }
        assert!(Request::new(
            Scope {
                selection_id: 0,
                ..scope
            },
            1,
            &"a".repeat(63),
            &"a".repeat(63)
        )
        .is_ok());
    }
}
