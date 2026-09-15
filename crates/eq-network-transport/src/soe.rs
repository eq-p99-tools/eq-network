use crate::error::{ProtocolError, Result};

#[repr(u16)]
/// Known big-endian SOE transport opcodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportOp {
    /// Client request to begin a negotiated session.
    SessionRequest = 0x0001,
    /// Server response containing negotiated session parameters.
    SessionResponse = 0x0002,
    /// Length-prefixed collection of transport packets.
    Combined = 0x0003,
    /// Session shutdown notification.
    SessionDisconnect = 0x0005,
    /// Empty liveness packet.
    KeepAlive = 0x0006,
    /// Request for transport statistics.
    SessionStatRequest = 0x0007,
    /// Response containing transport statistics.
    SessionStatResponse = 0x0008,
    /// Sequenced application packet.
    Packet = 0x0009,
    /// Sequenced application fragment.
    Fragment = 0x000D,
    /// Negative acknowledgement for a missing sequence.
    OutOfOrder = 0x0011,
    /// Cumulative acknowledgement.
    Ack = 0x0015,
    /// Collection of length-prefixed application packets.
    AppCombined = 0x0019,
    /// Packet received outside an established session.
    OutOfSession = 0x001D,
}

impl TransportOp {
    #[must_use]
    /// Convert a raw opcode into a known transport variant.
    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            0x0001 => Some(Self::SessionRequest),
            0x0002 => Some(Self::SessionResponse),
            0x0003 => Some(Self::Combined),
            0x0005 => Some(Self::SessionDisconnect),
            0x0006 => Some(Self::KeepAlive),
            0x0007 => Some(Self::SessionStatRequest),
            0x0008 => Some(Self::SessionStatResponse),
            0x0009 => Some(Self::Packet),
            0x000D => Some(Self::Fragment),
            0x0011 => Some(Self::OutOfOrder),
            0x0015 => Some(Self::Ack),
            0x0019 => Some(Self::AppCombined),
            0x001D => Some(Self::OutOfSession),
            _ => None,
        }
    }

    #[must_use]
    /// Return the stable diagnostic name for this opcode.
    pub fn name(self) -> &'static str {
        match self {
            Self::SessionRequest => "SessionRequest",
            Self::SessionResponse => "SessionResponse",
            Self::Combined => "Combined",
            Self::SessionDisconnect => "SessionDisconnect",
            Self::KeepAlive => "KeepAlive",
            Self::SessionStatRequest => "SessionStatRequest",
            Self::SessionStatResponse => "SessionStatResponse",
            Self::Packet => "Packet",
            Self::Fragment => "Fragment",
            Self::OutOfOrder => "OutOfOrder",
            Self::Ack => "Ack",
            Self::AppCombined => "AppCombined",
            Self::OutOfSession => "OutOfSession",
        }
    }
}

#[must_use]
/// Read a big-endian transport opcode, returning zero for a truncated input.
pub fn transport_opcode(data: &[u8]) -> u16 {
    if data.len() < 2 {
        return 0;
    }
    u16::from_be_bytes([data[0], data[1]])
}

/// Read the 2-byte big-endian sequence at `offset + 2`.
///
/// Datagrams arrive from the network with arbitrary length, so a buffer too
/// short to hold the sequence field yields 0 rather than panicking.
#[must_use]
pub fn get_sequence(data: &[u8], offset: usize) -> u16 {
    if data.len() < offset + 4 {
        return 0;
    }
    u16::from_be_bytes([data[offset + 2], data[offset + 3]])
}

/// Write the 2-byte big-endian sequence at `offset + 2`.
///
/// No-op when `buf` is too short to hold the sequence field, so truncated or
/// malformed datagrams cannot panic the proxy task.
pub fn set_sequence(buf: &mut [u8], offset: usize, seq: u16) {
    if buf.len() < offset + 4 {
        return;
    }
    let bytes = seq.to_be_bytes();
    buf[offset + 2] = bytes[0];
    buf[offset + 3] = bytes[1];
}

#[must_use]
/// Encode a cumulative acknowledgement packet.
pub fn build_ack(sequence: u16) -> [u8; 4] {
    let mut out = [0u8; 4];
    out[0..2].copy_from_slice(&(TransportOp::Ack as u16).to_be_bytes());
    out[2..4].copy_from_slice(&sequence.to_be_bytes());
    out
}

#[must_use]
/// Encode an empty keepalive packet.
pub fn build_keepalive() -> [u8; 2] {
    (TransportOp::KeepAlive as u16).to_be_bytes()
}

#[must_use]
/// Encode a session-disconnect packet.
pub fn build_disconnect() -> [u8; 2] {
    (TransportOp::SessionDisconnect as u16).to_be_bytes()
}

#[must_use]
/// Encode the minimal transport header for a session request.
pub fn build_session_request() -> [u8; 2] {
    (TransportOp::SessionRequest as u16).to_be_bytes()
}

/// Minimal valid `SessionResponse` (17-byte wire format).
#[must_use]
pub fn build_session_response() -> Vec<u8> {
    let mut out = vec![0u8; 17];
    out[0..2].copy_from_slice(&(TransportOp::SessionResponse as u16).to_be_bytes());
    out[6..10].copy_from_slice(&1u32.to_be_bytes()); // encode_key
    out[13..17].copy_from_slice(&512u32.to_le_bytes()); // max_packet_size
    out
}

#[must_use]
/// Wrap an application payload in a sequenced transport packet.
pub fn wrap_app_packet(sequence: u16, app_payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + app_payload.len());
    out.extend_from_slice(&(TransportOp::Packet as u16).to_be_bytes());
    out.extend_from_slice(&sequence.to_be_bytes());
    out.extend_from_slice(app_payload);
    out
}

/// Negotiated parameters decoded from an SOE session response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionResponse {
    /// Code identifying the client's request.
    pub connect_code: u32,
    /// Key mixed into transport CRC calculations.
    pub encode_key: u32,
    /// Negotiated CRC suffix length.
    pub crc_bytes: u8,
    /// First transport encoding selector.
    pub encode_pass1: u8,
    /// Second transport encoding selector.
    pub encode_pass2: u8,
    /// Maximum datagram size advertised by the server.
    pub max_packet_size: u32,
}

/// Decode the fixed fields from a session negotiation response.
///
/// # Errors
///
/// Returns [`ProtocolError::TooShort`] when fewer than 17 bytes are available.
pub fn parse_session_response(data: &[u8]) -> Result<SessionResponse> {
    if data.len() < 17 {
        return Err(ProtocolError::TooShort {
            need: 17,
            got: data.len(),
        });
    }
    let connect_code = u32::from_be_bytes([data[2], data[3], data[4], data[5]]);
    let encode_key = u32::from_be_bytes([data[6], data[7], data[8], data[9]]);
    Ok(SessionResponse {
        connect_code,
        encode_key,
        crc_bytes: data[10],
        encode_pass1: data[11],
        encode_pass2: data[12],
        max_packet_size: u32::from_le_bytes([data[13], data[14], data[15], data[16]]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_ack_wire_format() {
        let ack = build_ack(0x1234);
        assert_eq!(transport_opcode(&ack), TransportOp::Ack as u16);
        assert_eq!(get_sequence(&ack, 0), 0x1234);
    }
}
