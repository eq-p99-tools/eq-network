//! P99 list population notices and presence polls.
//!
//! Mode 1 carries the current population and offers a join confirmation;
//! mode 2 requests presence handling. The number in mode 1 is not a captcha.
//! The native numeric challenge is local UI state, and its slash-command
//! answer is consumed locally rather than sent as chat. Presence replies use
//! [`present`]; joining a list is a separate, explicit player action.
//!
//! These layouts are source-backed. Their acceptance on a deployed P99
//! server remains unverified; no periodic keepalive is defined here.

use crate::command::EncodedCommand;

/// Application opcode shared by list requests, notices and presence replies.
pub const OPCODE: u16 = 0x1253;

/// A recognized server list message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Poll {
    /// A population notice offering the player a separate join confirmation.
    Population {
        /// Number of players reported on the list.
        players: u32,
    },
    /// A request to handle a presence check; no server captcha is carried.
    Presence,
}

/// Reads the fields the native receiver consumes.
///
/// Mode 1 needs a four-byte little-endian population after its mode byte;
/// mode 2 consumes only its mode byte. Trailing bytes are ignored, matching
/// that receiver. Unknown modes and truncated populations return `None`.
#[must_use]
pub fn decode(body: &[u8]) -> Option<Poll> {
    let (&kind, rest) = body.split_first()?;
    match kind {
        1 => Some(Poll::Population {
            players: u32::from_le_bytes(rest.get(..4)?.try_into().ok()?),
        }),
        2 => Some(Poll::Presence),
        _ => None,
    }
}

/// The five-byte presence reply the native sender emits.
/// Sending it neither joins a list nor establishes server acceptance.
#[must_use]
pub fn present() -> EncodedCommand {
    EncodedCommand {
        opcode: OPCODE,
        body: vec![3, 0, 0, 0, 0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn population_is_not_a_presence_challenge_or_captcha() {
        assert_eq!(
            decode(&[1, 0x39, 0x30, 0, 0]),
            Some(Poll::Population { players: 12_345 })
        );
        assert_eq!(
            decode(&[1, 0, 0, 0, 0]),
            Some(Poll::Population { players: 0 })
        );
        assert_eq!(decode(&[2]), Some(Poll::Presence));
        assert_eq!(decode(&[2, 9, 9, 9, 9]), Some(Poll::Presence));
    }

    #[test]
    fn unknown_and_truncated_messages_are_not_promoted_to_polls() {
        for body in [&[][..], &[3, 0, 0, 0, 0], &[255], &[1], &[1, 0, 0]] {
            assert_eq!(decode(body), None);
        }
    }

    #[test]
    fn presence_uses_its_own_reply_and_never_the_join_confirmation() {
        assert_eq!(
            present(),
            EncodedCommand {
                opcode: OPCODE,
                body: vec![3, 0, 0, 0, 0],
            }
        );
    }
}
