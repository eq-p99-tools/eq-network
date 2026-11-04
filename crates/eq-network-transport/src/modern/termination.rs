//! Established inbound SOE termination controls, without application authority.
use std::fmt;

/// A peer explicitly ended this modern UDP session.
///
/// The reason describes the transport only, never the outcome of an application
/// request. A fresh login is a separate attempt governed by the caller's
/// reconnect policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SessionTermination {
    /// A disconnect matched the current connection code and passed negotiated
    /// framing checks. A native optional reason is retained without interpretation.
    PeerDisconnected {
        /// Opaque native reason code, when the peer supplied one.
        reason: Option<u16>,
    },
    /// The connected peer sent the stateless, zero-code out-of-session control.
    /// It has no authenticated session identity or application acknowledgement.
    OutOfSession,
}

impl SessionTermination {
    pub(super) fn disconnect(body: &[u8], connection: u32) -> Option<Self> {
        if !matches!(body.len(), 4 | 6)
            || u32::from_be_bytes(body.get(..4)?.try_into().ok()?) != connection
        {
            return None;
        }
        Some(Self::PeerDisconnected {
            reason: body
                .get(4..6)
                .map(|code| u16::from_be_bytes([code[0], code[1]])),
        })
    }

    pub(super) fn out_of_session(datagram: &[u8]) -> bool {
        // The manager has no session key/code: its six serialized bytes bypass
        // negotiated compression and CRC. Other layouts remain unverified.
        datagram == [0, 0x1d, 0, 0, 0, 0]
    }
}

impl fmt::Display for SessionTermination {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PeerDisconnected { reason: None } => {
                formatter.write_str("server closed the UDP session")
            }
            Self::PeerDisconnected { reason: Some(code) } => {
                write!(
                    formatter,
                    "server closed the UDP session (reason code {code})"
                )
            }
            Self::OutOfSession => {
                formatter.write_str("server no longer recognizes the UDP session")
            }
        }
    }
}

impl std::error::Error for SessionTermination {}
