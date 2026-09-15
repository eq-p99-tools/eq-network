use crate::crypto::{des_decrypt, des_encrypt, DesKeyIv};
use crate::error::{LoginError, Result};
use eq_network_transport::build_combined;
use eq_network_transport::TransportOp;
use std::fmt;
use zeroize::Zeroizing;

/// Fixed byte length of a login application header.
pub const LOGIN_BASE_SIZE: usize = 10;
/// Offset at which an encrypted login result begins.
pub const LOGIN_RESULT_HEADER_SIZE: usize = 12;
/// Decrypted status value identifying a rejected login.
pub const LOGIN_RESULT_FAILURE_STATUS: u32 = 0xFFFF_FFFF;

const ACK_END: usize = 7;
const LOGIN_SUB_HEADER: usize = 6;
const ENC_OFFSET: usize = LOGIN_SUB_HEADER + LOGIN_BASE_SIZE;

#[repr(u16)]
/// Known little-endian login-server application opcodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppOp {
    /// Server invitation to start authentication.
    SessionReady = 0x0001,
    /// Client credential request.
    Login = 0x0002,
    /// Client acknowledgement that login has completed.
    LoginComplete = 0x0003,
    /// Client request for available world servers.
    ServerListRequest = 0x0004,
    /// Client request to enter a selected world server.
    PlayEverquestRequest = 0x000D,
    /// Client request to enter login-server chat.
    EnterChat = 0x000F,
    /// Login-server chat message.
    ChatMessage = 0x0016,
    /// Server authentication result.
    LoginAccepted = 0x0017,
    /// Server response containing available worlds.
    ServerListResponse = 0x0018,
    /// Server response to world selection.
    PlayEverquestResponse = 0x0021,
    /// Response to a login-server poll.
    PollResponse = 0x0011,
    /// Login-server poll request.
    Poll = 0x0029,
}

#[must_use]
/// Read the little-endian application opcode, returning zero when truncated.
pub fn app_opcode(app_payload: &[u8]) -> u16 {
    if app_payload.len() < 2 {
        return 0;
    }
    u16::from_le_bytes([app_payload[0], app_payload[1]])
}

/// Common header shared by `EQEmu` login application packets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginBase {
    /// Login protocol sequence marker.
    pub sequence: i32,
    /// Whether the application body is compressed.
    pub compressed: u8,
    /// Credential encryption selector.
    pub encrypt_type: i8,
    /// Unknown field retained from the wire format.
    pub unk3: u32,
}

#[must_use]
/// Parse a common login application header.
pub fn parse_login_base(data: &[u8]) -> Option<LoginBase> {
    if data.len() < LOGIN_BASE_SIZE {
        return None;
    }
    Some(LoginBase {
        sequence: i32::from_le_bytes(data[0..4].try_into().ok()?),
        compressed: data[4],
        encrypt_type: data[5].cast_signed(),
        unk3: u32::from_le_bytes(data[6..10].try_into().ok()?),
    })
}

#[must_use]
/// Encode a NUL-delimited account name and password using legacy DES-CBC.
pub fn encrypt_login_credentials(username: &str, password: &str, key_iv: DesKeyIv) -> Vec<u8> {
    let mut plaintext = Zeroizing::new(username.as_bytes().to_vec());
    plaintext.push(0);
    plaintext.extend_from_slice(password.as_bytes());
    plaintext.push(0);
    des_encrypt(&plaintext, key_iv)
}

fn decrypt_credentials(encrypted: &[u8], key_iv: DesKeyIv) -> Option<(String, String)> {
    let decrypted = Zeroizing::new(des_decrypt(encrypted, key_iv).ok()?);
    // Wire format is `username\0password\0`. Split positionally on the first two
    // NULs so an empty username is preserved rather than promoting the password
    // into the username slot.
    let first_nul = decrypted.iter().position(|&b| b == 0);
    let (username_bytes, rest) = match first_nul {
        Some(idx) => (&decrypted[..idx], &decrypted[idx + 1..]),
        None => (&decrypted[..], &decrypted[decrypted.len()..]),
    };
    let password_bytes = match rest.iter().position(|&b| b == 0) {
        Some(idx) => &rest[..idx],
        None => rest,
    };
    let username = String::from_utf8_lossy(username_bytes).to_string();
    let password = String::from_utf8_lossy(password_bytes).to_string();
    Some((username, password))
}

/// Decoded proxy login request retaining enough structure for safe rewriting.
#[derive(Clone)]
pub struct LoginPacket {
    buf: Zeroizing<Vec<u8>>,
    username: Zeroizing<String>,
    password: Zeroizing<String>,
    sub2_offset: usize,
    sub2_len: usize,
    enc_offset: usize,
}

impl fmt::Debug for LoginPacket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoginPacket")
            .field("buffer_length", &self.buf.len())
            .field("username", &"[REDACTED]")
            .field("password", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl LoginPacket {
    #[must_use]
    /// Decode a combined login request and its credential block.
    pub fn parse(buf: &[u8], key_iv: DesKeyIv) -> Option<Self> {
        if buf.len() < 30 || !buf.starts_with(&[0x00, 0x03, 0x04, 0x00, 0x15]) {
            return None;
        }
        if buf.len() <= ACK_END {
            return None;
        }
        let sub2_len = buf[ACK_END] as usize;
        let sub2_start = ACK_END + 1;
        if sub2_start + sub2_len > buf.len() || sub2_len < LOGIN_SUB_HEADER {
            return None;
        }
        let sub2 = &buf[sub2_start..sub2_start + sub2_len];
        let transport_op = u16::from_be_bytes([sub2[0], sub2[1]]);
        if transport_op != TransportOp::Packet as u16 {
            return None;
        }
        if app_opcode(&sub2[4..]) != AppOp::Login as u16 {
            return None;
        }
        if sub2.len() <= ENC_OFFSET {
            return None;
        }
        let encrypted = &sub2[ENC_OFFSET..];
        let (username, password) = decrypt_credentials(encrypted, key_iv)?;
        Some(Self {
            buf: Zeroizing::new(buf.to_vec()),
            username: Zeroizing::new(username),
            password: Zeroizing::new(password),
            sub2_offset: sub2_start,
            sub2_len,
            enc_offset: ENC_OFFSET,
        })
    }

    /// Return the decoded account name.
    #[must_use]
    pub fn username(&self) -> &str {
        self.username.as_str()
    }

    /// Return the decoded password for an immediate routing or rewrite decision.
    ///
    /// Callers should avoid copying, retaining, or logging this value.
    #[must_use]
    pub fn password(&self) -> &str {
        self.password.as_str()
    }

    /// Replace the decoded credentials and return a newly encoded packet.
    ///
    /// # Errors
    ///
    /// Returns an error when the rewritten packet exceeds its wire length field.
    pub fn rewrite_credentials(
        &self,
        new_user: &str,
        new_pass: &str,
        key_iv: DesKeyIv,
    ) -> Result<Vec<u8>> {
        let new_enc = encrypt_login_credentials(new_user, new_pass, key_iv);
        self.splice_encrypted_credentials(&new_enc)
    }

    /// Replace the encrypted credential block without decrypting it.
    ///
    /// # Errors
    ///
    /// Returns an error when the rewritten packet exceeds its wire length field.
    pub fn splice_encrypted_credentials(&self, encrypted: &[u8]) -> Result<Vec<u8>> {
        let new_sub_len = self.enc_offset + encrypted.len();
        let length = u8::try_from(new_sub_len)
            .map_err(|_| LoginError::SubPacketLengthOverflow { len: new_sub_len })?;
        let abs_start = self.sub2_offset + self.enc_offset;
        let abs_end = self.sub2_offset + self.sub2_len;
        let mut out = Vec::new();
        out.extend_from_slice(&self.buf[..abs_start]);
        out.extend_from_slice(encrypted);
        out.extend_from_slice(&self.buf[abs_end..]);
        out[self.sub2_offset - 1] = length;
        Ok(out)
    }
}

#[must_use]
/// Return whether a login-accepted packet encodes the standard bad-password result.
pub fn is_bad_password_login_result(app_payload: &[u8], key_iv: DesKeyIv) -> bool {
    if app_payload.len() < LOGIN_RESULT_HEADER_SIZE {
        return false;
    }
    if app_opcode(app_payload) != AppOp::LoginAccepted as u16 {
        return false;
    }
    let Some(base) = parse_login_base(&app_payload[2..]) else {
        return false;
    };
    if base.sequence != 3 || base.encrypt_type != 2 {
        return false;
    }
    let mut encrypted = &app_payload[LOGIN_RESULT_HEADER_SIZE..];
    if !encrypted.is_empty() && encrypted.len() % 8 == 1 {
        encrypted = &encrypted[..encrypted.len() - 1];
    }
    if encrypted.is_empty() || !encrypted.len().is_multiple_of(8) {
        return false;
    }
    let Ok(decrypted) = des_decrypt(encrypted, key_iv) else {
        return false;
    };
    if decrypted.len() < 12 {
        return false;
    }
    let status = u32::from_le_bytes([decrypted[8], decrypted[9], decrypted[10], decrypted[11]]);
    if status != LOGIN_RESULT_FAILURE_STATUS {
        return false;
    }
    decrypted[12..].iter().all(|&b| b == 0)
}

/// Build an Ack transport sub-packet (`opcode || seq`).
fn ack_sub(seq: u16) -> Vec<u8> {
    let mut sub = Vec::with_capacity(4);
    sub.extend_from_slice(&(TransportOp::Ack as u16).to_be_bytes());
    sub.extend_from_slice(&seq.to_be_bytes());
    sub
}

/// Build a Packet transport sub-packet (`opcode || seq || payload`).
fn packet_sub(seq: u16, payload: &[u8]) -> Vec<u8> {
    let mut sub = Vec::with_capacity(4 + payload.len());
    sub.extend_from_slice(&(TransportOp::Packet as u16).to_be_bytes());
    sub.extend_from_slice(&seq.to_be_bytes());
    sub.extend_from_slice(payload);
    sub
}

/// Encode the standard combined ACK and login request packet.
///
/// # Errors
///
/// Returns an error when the encoded credentials exceed a transport length field.
pub fn build_login_combined(username: &str, password: &str, key_iv: DesKeyIv) -> Result<Vec<u8>> {
    let encrypted = encrypt_login_credentials(username, password, key_iv);
    let mut base_flat = Vec::new();
    base_flat.extend_from_slice(&3i32.to_le_bytes());
    base_flat.push(0);
    base_flat.push(2);
    base_flat.extend_from_slice(&0u32.to_le_bytes());
    let mut app_payload = Vec::new();
    app_payload.extend_from_slice(&(AppOp::Login as u16).to_le_bytes());
    app_payload.extend_from_slice(&base_flat);
    app_payload.extend_from_slice(&encrypted);
    Ok(build_combined(&[
        &ack_sub(0),
        &packet_sub(1, &app_payload),
    ])?)
}

/// Encode a synthetic login-accepted response, primarily for proxy testing.
///
/// # Errors
///
/// Returns an error when the application packet exceeds a transport length field.
pub fn build_login_accepted_combined(
    account_id: u32,
    status: u32,
    server_seq: u16,
    key_iv: DesKeyIv,
) -> Result<Vec<u8>> {
    let mut plaintext = Vec::new();
    plaintext.extend_from_slice(&account_id.to_le_bytes());
    plaintext.extend_from_slice(&0u32.to_le_bytes());
    plaintext.extend_from_slice(&status.to_le_bytes());
    plaintext.extend_from_slice(&[0u8; 20]);
    let encrypted = des_encrypt(&plaintext, key_iv);
    let mut base_flat = Vec::new();
    base_flat.extend_from_slice(&3i32.to_le_bytes());
    base_flat.push(0);
    base_flat.push(2);
    base_flat.extend_from_slice(&0u32.to_le_bytes());
    let mut app_payload = Vec::new();
    app_payload.extend_from_slice(&(AppOp::LoginAccepted as u16).to_le_bytes());
    app_payload.extend_from_slice(&base_flat);
    app_payload.extend_from_slice(&encrypted);
    Ok(build_combined(&[
        &ack_sub(1),
        &packet_sub(server_seq, &app_payload),
    ])?)
}

/// Combine an acknowledgement and application packet.
///
/// # Errors
///
/// Returns an error when the application packet exceeds a transport length field.
pub fn build_combined_ack_then_packet(
    ack_seq: u16,
    packet_seq: u16,
    app_payload: &[u8],
) -> Result<Vec<u8>> {
    Ok(build_combined(&[
        &ack_sub(ack_seq),
        &packet_sub(packet_seq, app_payload),
    ])?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decrypt_round_trips_username_and_password() {
        let key_iv = DesKeyIv::default();
        let encrypted = encrypt_login_credentials("example-user", "example-password", key_iv);
        let (username, password) = decrypt_credentials(&encrypted, key_iv).unwrap();
        assert_eq!(username, "example-user");
        assert_eq!(password, "example-password");
    }

    #[test]
    fn decrypt_preserves_empty_username() {
        let key_iv = DesKeyIv::default();
        let encrypted = encrypt_login_credentials("", "example-password", key_iv);
        let (username, password) = decrypt_credentials(&encrypted, key_iv).unwrap();
        assert_eq!(username, "");
        assert_eq!(password, "example-password");
    }

    #[test]
    fn decrypt_handles_missing_password_terminator() {
        let key_iv = DesKeyIv::default();
        // `username\0password` with no trailing NUL still yields both fields.
        let encrypted = des_encrypt(b"example-user\0example-password", key_iv);
        let (username, password) = decrypt_credentials(&encrypted, key_iv).unwrap();
        assert_eq!(username, "example-user");
        assert_eq!(password, "example-password");
    }
}
