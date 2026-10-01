use crate::error::{ProtocolError, Result};
use crate::soe::TransportOp;

/// Location and transport opcode of one packet inside a combined datagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubPacket {
    /// Byte offset of the sub-packet's transport opcode.
    pub offset: usize,
    /// Complete sub-packet length in bytes.
    pub length: usize,
    /// Raw big-endian transport opcode.
    pub transport_op: u16,
}

/// Parsed sub-packet boundaries from a combined transport datagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombinedPacket {
    /// Sub-packets in their original wire order.
    pub subs: Vec<SubPacket>,
}

impl CombinedPacket {
    /// Parse sub-packet boundaries from a combined transport packet, stopping
    /// at the first part that does not fit. Every part has a one-byte length.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::TooShort`] when the selected packet range does
    /// not contain a transport opcode.
    pub fn parse(buf: &[u8], start_index: usize, length: Option<usize>) -> Result<Self> {
        let end = start_index + length.unwrap_or_else(|| buf.len().saturating_sub(start_index));
        if end < start_index + 2 {
            return Err(ProtocolError::TooShort {
                need: 2,
                got: end.saturating_sub(start_index),
            });
        }
        let mut pos = start_index + 2;
        let mut subs = Vec::new();
        while pos < end.min(buf.len()) {
            let sublen = usize::from(buf[pos]);
            pos += 1;
            if sublen == 0 || pos + sublen > end.min(buf.len()) {
                break;
            }
            let op = if sublen >= 2 {
                u16::from_be_bytes([buf[pos], buf[pos + 1]])
            } else {
                0
            };
            subs.push(SubPacket {
                offset: pos,
                length: sublen,
                transport_op: op,
            });
            pos += sublen;
        }
        Ok(Self { subs })
    }

    #[must_use]
    /// Borrow the complete bytes for a parsed sub-packet.
    pub fn sub_bytes<'a>(&self, buf: &'a [u8], sub: &SubPacket) -> &'a [u8] {
        &buf[sub.offset..sub.offset + sub.length]
    }
}

/// Encode transport packets into one combined packet.
///
/// # Errors
///
/// Returns [`ProtocolError::CombinedSubPacketTooLong`] when a sub-packet is
/// longer than the 255 bytes a one-byte length can carry.
pub fn build_combined(sub_packets: &[&[u8]]) -> Result<Vec<u8>> {
    let mut out = (TransportOp::Combined as u16).to_be_bytes().to_vec();
    for sub in sub_packets {
        let length = u8::try_from(sub.len())
            .map_err(|_| ProtocolError::CombinedSubPacketTooLong { len: sub.len() })?;
        out.push(length);
        out.extend_from_slice(sub);
    }
    Ok(out)
}

#[must_use]
/// Split a combined packet into owned sub-packets, or return an empty vector
/// when its outer header is truncated.
pub fn split_combined(data: &[u8]) -> Vec<Vec<u8>> {
    CombinedPacket::parse(data, 0, None)
        .map(|cp| {
            cp.subs
                .iter()
                .map(|s| data[s.offset..s.offset + s.length].to_vec())
                .collect()
        })
        .unwrap_or_default()
}

/// The parts of an `OP_Combined` body, the bytes after its opcode. Every part
/// has a one-byte length: `EQEmu` combines only packets of up to 255 bytes
/// (`reliable_stream_connection.cpp`, `FlushBuffer`), so 255 is a length like
/// any other, not an escape.
pub(crate) fn parts(mut body: &[u8]) -> Result<Vec<&[u8]>> {
    let mut parts = Vec::new();
    while let Some((&length, rest)) = body.split_first() {
        let (part, rest) = take(rest, usize::from(length))?;
        parts.push(part);
        body = rest;
    }
    Ok(parts)
}

/// The parts of an `OP_AppCombined` body. A length of 255 escapes to the
/// two-byte length after it, and three of them to a four-byte one (`EQEmu`
/// `reliable_stream_connection.cpp`, `OP_AppCombined`).
pub(crate) fn app_parts(mut body: &[u8]) -> Result<Vec<&[u8]>> {
    let mut parts = Vec::new();
    while !body.is_empty() {
        let (length, rest) = match body {
            [0xff, 0xff, 0xff, a, b, c, d, rest @ ..] => {
                let length = u32::from_be_bytes([*a, *b, *c, *d]);
                (usize::try_from(length).unwrap_or(usize::MAX), rest)
            }
            [0xff, 0xff, 0xff, rest @ ..] => {
                return Err(ProtocolError::TooShort {
                    need: 4,
                    got: rest.len(),
                })
            }
            [0xff, a, b, rest @ ..] => (usize::from(u16::from_be_bytes([*a, *b])), rest),
            [0xff, rest @ ..] => {
                return Err(ProtocolError::TooShort {
                    need: 2,
                    got: rest.len(),
                })
            }
            [length, rest @ ..] => (usize::from(*length), rest),
            [] => unreachable!("the loop stops at the end"),
        };
        let (part, rest) = take(rest, length)?;
        parts.push(part);
        body = rest;
    }
    Ok(parts)
}

/// Splits off a part of `length` bytes.
fn take(bytes: &[u8], length: usize) -> Result<(&[u8], &[u8])> {
    if length > bytes.len() {
        return Err(ProtocolError::TooShort {
            need: length,
            got: bytes.len(),
        });
    }
    Ok(bytes.split_at(length))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soe::{build_ack, transport_opcode};

    #[test]
    fn roundtrip_combined() {
        let ack = build_ack(1);
        let packet = [0x00, 0x09, 0x00, 0x02, 0x02, 0x00];
        let combined = build_combined(&[&ack, &packet]).unwrap();
        assert_eq!(transport_opcode(&combined), TransportOp::Combined as u16);
        let subs = split_combined(&combined);
        assert_eq!(subs.len(), 2);
    }

    #[test]
    fn combined_parts_of_255_bytes_keep_a_one_byte_length() {
        let long = [7; 255];
        let ack = build_ack(1);
        let combined = build_combined(&[&long, &ack]).unwrap();
        assert_eq!(combined[2], 255);
        assert_eq!(split_combined(&combined), [long.to_vec(), ack.to_vec()]);
        assert_eq!(parts(&combined[2..]).unwrap(), [&long[..], &ack[..]]);
        assert_eq!(
            build_combined(&[&[0; 256]]),
            Err(ProtocolError::CombinedSubPacketTooLong { len: 256 })
        );
        assert!(parts(&[3, 1, 2]).is_err());
    }

    #[test]
    fn app_combined_lengths_escape_to_two_and_four_bytes() {
        let long = vec![9; 300];
        let mut combined = vec![2, 0x10, 0x04, 0xff, 0x01, 0x2c];
        combined.extend_from_slice(&long);
        combined.extend_from_slice(&[0xff, 0xff, 0xff, 0, 0, 0, 3, 1, 2, 3]);
        assert_eq!(
            app_parts(&combined).unwrap(),
            [&[0x10, 0x04][..], &long[..], &[1, 2, 3][..]]
        );
        for truncated in [&[0xff][..], &[0xff, 1], &[0xff, 0xff, 0xff, 0, 0, 0]] {
            assert!(app_parts(truncated).is_err());
        }
        assert!(app_parts(&[0xff, 0, 4, 1]).is_err());
    }
}
