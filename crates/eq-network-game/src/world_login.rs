//! The login info a client opens the world with: the account and session
//! key the login server handed out, and whether the client is between zones.
//!
//! Titanium's is written by the session, where the server's protection reads
//! it. `RoF2`'s is here, built from `EQEmu`'s source and inferred for `RoF2`
//! (6.497): `common/patches/rof2_structs.h`, `LoginInfo_Struct`, which
//! `world/client.cpp`'s `Client::HandleSendLoginInfoPacket` reads, and
//! `utils/patches/patch_RoF2.conf`, `OP_SendLoginInfo`.

use anyhow::{ensure, Result};
use zeroize::Zeroizing;

/// The opcode of `RoF2`'s login info (inferred, 6.497).
pub const ROF2_OPCODE: u16 = 0x7a09;

/// The length of `RoF2`'s login info (inferred, 6.497).
pub const ROF2_LENGTH: usize = 488;

/// Where `RoF2`'s login info says the client is between zones (inferred,
/// 6.497).
const ROF2_ZONING: usize = 188;

/// The longest account `EQEmu`'s world reads from the login info.
const ACCOUNT_LIMIT: usize = 18;

/// The longest key `EQEmu`'s world reads from the login info.
const KEY_LIMIT: usize = 15;

/// `RoF2`'s login info: the account, a NUL, the key and a NUL at the start of
/// the 64-byte login field, and the zoning flag. Every other byte is zero:
/// the world reads none of them, and what the `RoF2` client puts there is
/// not read yet (6.497).
///
/// # Errors
/// Refuses an empty account, an account or key longer than the world reads,
/// and one holding a NUL, rather than cut either short.
pub fn rof2(account: &str, key: &[u8], zoning: bool) -> Result<Zeroizing<Vec<u8>>> {
    ensure!(
        !account.is_empty() && account.len() <= ACCOUNT_LIMIT && !account.contains('\0'),
        "the account doesn't fit the RoF2 world login"
    );
    ensure!(
        !key.is_empty() && key.len() <= KEY_LIMIT && !key.contains(&0),
        "the session key doesn't fit the RoF2 world login"
    );
    let mut info = Zeroizing::new(vec![0; ROF2_LENGTH]);
    info[..account.len()].copy_from_slice(account.as_bytes());
    let key_at = account.len() + 1;
    info[key_at..key_at + key.len()].copy_from_slice(key);
    info[ROF2_ZONING] = u8::from(zoning);
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rof2s_login_info_puts_the_account_key_and_zoning_where_the_world_reads_them() {
        let info = rof2("12345", b"EXAMPLEKEY", true).unwrap();
        assert_eq!(info.len(), 488);
        assert_eq!(&info[..17], b"12345\0EXAMPLEKEY\0");
        assert_eq!(info[188], 1);
        let nonzero: Vec<usize> = (0..info.len()).filter(|at| info[*at] != 0).collect();
        assert_eq!(
            nonzero,
            [0, 1, 2, 3, 4, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 188]
        );
        assert_eq!(rof2("12345", b"EXAMPLEKEY", false).unwrap()[188], 0);
    }

    #[test]
    fn rof2s_login_info_refuses_what_does_not_fit_rather_than_cut_it() {
        let long_account = "1".repeat(19);
        assert!(rof2(&"1".repeat(18), b"KEY", false).is_ok());
        assert!(rof2(&"1".repeat(18), &[b'K'; 15], false).is_ok());
        for (account, key) in [
            ("", &b"EXAMPLEKEY"[..]),
            (long_account.as_str(), b"EXAMPLEKEY"),
            ("12\x0045", b"EXAMPLEKEY"),
            ("12345", b""),
            ("12345", &[b'K'; 16]),
            ("12345", b"EXAMPLE\0KEY"),
        ] {
            assert!(rof2(account, key, false).is_err(), "{account:?} {key:?}");
        }
    }
}
