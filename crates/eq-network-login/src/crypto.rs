use cbc::cipher::{block_padding::NoPadding, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use des::Des;
use thiserror::Error;

type DesCbcEnc = cbc::Encryptor<Des>;
type DesCbcDec = cbc::Decryptor<Des>;

/// Default all-zero `DES` key used by the `EQEmu` login protocol.
pub const DEFAULT_DES_KEY: [u8; 8] = [0; 8];
/// Default all-zero `DES` initialization vector used by the `EQEmu` login protocol.
pub const DEFAULT_DES_IV: [u8; 8] = [0; 8];

/// DES-CBC key and initialization vector selected by a login protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesKeyIv {
    /// Eight-byte DES key.
    pub key: [u8; 8],
    /// Eight-byte CBC initialization vector.
    pub iv: [u8; 8],
}

impl Default for DesKeyIv {
    fn default() -> Self {
        Self {
            key: DEFAULT_DES_KEY,
            iv: DEFAULT_DES_IV,
        }
    }
}

/// Errors returned by legacy credential decryption.
#[derive(Debug, Error)]
pub enum CryptoError {
    /// Ciphertext was empty or not aligned to a DES block.
    #[error("ciphertext length {0} is not a multiple of 8")]
    InvalidLength(usize),
}

/// DES-CBC encrypt with zero/null padding to an 8-byte boundary.
///
/// # Panics
///
/// Panics only if the cipher rejects an internally allocated buffer whose
/// length was rounded to its required block size.
#[must_use]
pub fn des_encrypt(plaintext: &[u8], key_iv: DesKeyIv) -> Vec<u8> {
    let padded_len = plaintext.len().div_ceil(8) * 8;
    let mut padded = vec![0u8; padded_len];
    padded[..plaintext.len()].copy_from_slice(plaintext);
    let mut buf = padded;
    let cipher = DesCbcEnc::new(&key_iv.key.into(), &key_iv.iv.into());
    cipher
        .encrypt_padded_mut::<NoPadding>(&mut buf, padded_len)
        .expect("padded to block size");
    buf
}

/// Decrypt a complete DES-CBC credential block without removing zero padding.
///
/// # Errors
///
/// Returns [`CryptoError::InvalidLength`] when the ciphertext is empty or not
/// aligned to the eight-byte DES block size.
pub fn des_decrypt(ciphertext: &[u8], key_iv: DesKeyIv) -> Result<Vec<u8>, CryptoError> {
    if ciphertext.is_empty() || !ciphertext.len().is_multiple_of(8) {
        return Err(CryptoError::InvalidLength(ciphertext.len()));
    }
    let mut buf = ciphertext.to_vec();
    let cipher = DesCbcDec::new(&key_iv.key.into(), &key_iv.iv.into());
    cipher
        .decrypt_padded_mut::<NoPadding>(&mut buf)
        .map_err(|_| CryptoError::InvalidLength(ciphertext.len()))?;
    Ok(buf)
}
