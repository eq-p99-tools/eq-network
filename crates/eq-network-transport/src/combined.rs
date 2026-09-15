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
    /// Parse sub-packet boundaries from a combined transport packet.
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
        while pos < end {
            if pos >= buf.len() {
                break;
            }
            let mut sublen = buf[pos] as usize;
            pos += 1;
            if sublen == 0xFF {
                if pos + 2 > end {
                    break;
                }
                sublen = u16::from_be_bytes([buf[pos], buf[pos + 1]]) as usize;
                pos += 2;
            }
            if sublen == 0 || pos + sublen > end {
                break;
            }
            let op = if pos + 2 <= buf.len() {
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
/// Returns [`ProtocolError::CombinedSubPacketTooLong`] when a sub-packet cannot
/// be represented by the two-byte extended length field.
pub fn build_combined(sub_packets: &[&[u8]]) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    for sub in sub_packets {
        let slen = sub.len();
        if slen >= 0xFF {
            body.push(0xFF);
            let length = u16::try_from(slen)
                .map_err(|_| ProtocolError::CombinedSubPacketTooLong { len: slen })?;
            body.extend_from_slice(&length.to_be_bytes());
        } else {
            body.push(slen.to_le_bytes()[0]);
        }
        body.extend_from_slice(sub);
    }
    let mut out = Vec::with_capacity(2 + body.len());
    out.extend_from_slice(&(TransportOp::Combined as u16).to_be_bytes());
    out.extend_from_slice(&body);
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
}
