//! Damage the world does to the player: falls, drowning, lava and freezing.
//! The client works the damage out and reports it, and the server takes the
//! amount on its word, applying only its own reductions.
//!
//! Layout references: `EQEmu`'s `EnvDamage2_Struct`
//! (`common/eq_packet_structs.h` and `common/patches/titanium_structs.h`)
//! and `EQMacEmu`'s `Damage_Struct` (`common/eq_packet_structs.h`); the
//! rules are `EQEmu`'s `Client::Handle_OP_EnvDamage` and `EQMacEmu`'s
//! `Client::Handle_OP_Damage` (`zone/client_packet.cpp`). What the official
//! clients put in the fields neither server reads is unrecorded, so these
//! packets name the player's spawn where a field names a spawn and leave
//! the rest at zero (inferred).
//!
//! `EQMac` reports the world's damage twice, as the program does (from the
//! program, TAKP Batch 1 R3 and R4; checklist 6.161 and 6.162): first
//! `OP_EnvDamage` ([`eqmac_notice`]), then the packet its server sends
//! damage records in, [`EQMAC_DAMAGE_OPCODE`](crate::combat::EQMAC_DAMAGE_OPCODE)
//! ([`eqmac_damage`]). `EQMacEmu`'s `Handle_OP_EnvDamage` does nothing, and
//! `Handle_OP_Damage` takes the amount once (`zone/client_packet.cpp`,
//! Settle results S1), so the pair costs the player the damage once.
use crate::{combat::EQMAC_DAMAGE_OPCODE, command::EncodedCommand};
use anyhow::{ensure, Result};

/// `OP_EnvDamage`: damage the world did to the player, in Titanium's
/// 31-byte `EnvDamage2_Struct`.
pub const ENV_DAMAGE_OPCODE: u16 = 0x31b3;
/// `EQMac`'s `OP_EnvDamage` (0x401e swapped), 36 bytes.
pub const EQMAC_ENV_DAMAGE_OPCODE: u16 = 0x1e40;
/// `OP_SafeFallSuccess` (Titanium), with no body.
pub const SAFE_FALL_OPCODE: u16 = 0x3b21;
/// `EQMac`'s `OP_SafeFallSuccess` (0x41ab swapped), with no body.
pub const EQMAC_SAFE_FALL_OPCODE: u16 = 0xab41;

/// `EQMac`'s `OP_EnvDamage` body.
const EQMAC_NOTICE_SIZE: usize = 36;

/// Titanium's `EnvDamage2_Struct`.
const TITANIUM_SIZE: usize = 31;
/// `EQMac`'s `Damage_Struct`.
const EQMAC_SIZE: usize = 24;

/// What in the world did the damage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Hazard {
    /// A fall.
    Falling,
    /// Running out of air under water.
    Drowning,
    /// Standing in lava.
    Lava,
    /// Bitter cold.
    Freezing,
}

impl Hazard {
    /// The damage type both generations put on the report, where the
    /// generation has one: lava, drowning and falling are 250, 251 and 252
    /// in both servers' code, and only `EQMacEmu` names freezing, 246.
    const fn code(self, titanium: bool) -> Option<u8> {
        match self {
            Self::Lava => Some(250),
            Self::Drowning => Some(251),
            Self::Falling => Some(252),
            Self::Freezing if titanium => None,
            Self::Freezing => Some(246),
        }
    }
}

/// Checks what every report carries: the player's spawn, and an amount
/// both servers read as it is (`EQMacEmu` reads a negative one as 31,337,
/// and `EQEmu` turns it signed for its own modifier).
fn check(spawn_id: u16, amount: u32) -> Result<()> {
    ensure!(
        spawn_id != 0,
        "a damage report needs the player's own spawn"
    );
    ensure!(amount != 0, "no damage to report");
    ensure!(
        i32::try_from(amount).is_ok(),
        "the servers read more than {} points of damage as negative",
        i32::MAX
    );
    Ok(())
}

/// The Titanium client's report (`OP_EnvDamage`): the player's spawn at 0,
/// the amount at 6, the damage type at 22, and 0xFFFF at 27, which the
/// struct calls constant; the rest is zero.
///
/// # Errors
/// Refuses freezing, for which no Titanium damage type is known, a report
/// for no spawn, no damage, and more than `i32::MAX` points of it.
pub fn titanium_damage(spawn_id: u16, hazard: Hazard, amount: u32) -> Result<EncodedCommand> {
    check(spawn_id, amount)?;
    let Some(code) = hazard.code(true) else {
        anyhow::bail!("no Titanium damage type is known for {hazard:?}");
    };
    let mut body = vec![0; TITANIUM_SIZE];
    body[..4].copy_from_slice(&u32::from(spawn_id).to_le_bytes());
    body[6..10].copy_from_slice(&amount.to_le_bytes());
    body[22] = code;
    body[27..29].copy_from_slice(&u16::MAX.to_le_bytes());
    Ok(EncodedCommand {
        opcode: ENV_DAMAGE_OPCODE,
        body,
    })
}

/// The `EQMac` client's first report (`OP_EnvDamage`), laid out as the
/// program builds it (from the program, TAKP Batch 1 R3): the player's
/// spawn at 0, zeros at 2 and 4, the amount at 8, three zero floats from
/// 12, the damage type at 24, 0xFFFF at 30, and 0 at 33. The bytes the
/// program never writes stay 0.
///
/// # Errors
/// Refuses a report for no spawn, no damage, and more than `i32::MAX`
/// points of it.
pub fn eqmac_notice(spawn_id: u16, hazard: Hazard, amount: u32) -> Result<EncodedCommand> {
    check(spawn_id, amount)?;
    let Some(code) = hazard.code(false) else {
        anyhow::bail!("no EQMac damage type is known for {hazard:?}");
    };
    let mut body = vec![0; EQMAC_NOTICE_SIZE];
    body[..2].copy_from_slice(&spawn_id.to_le_bytes());
    body[8..12].copy_from_slice(&amount.to_le_bytes());
    body[24] = code;
    body[30..32].copy_from_slice(&u16::MAX.to_le_bytes());
    Ok(EncodedCommand {
        opcode: EQMAC_ENV_DAMAGE_OPCODE,
        body,
    })
}

/// The `EQMac` client's second report (`OP_Damage`), as the program sends
/// it after [`eqmac_notice`] (from the program, TAKP Batch 1 R3): the
/// player's spawn as the target, no source, the damage type as a byte at 4
/// (5 is never written and stays 0), no spell (0xFFFF), the amount at 8,
/// and no push.
///
/// # Errors
/// Refuses a report for no spawn, no damage, and more than `i32::MAX`
/// points of it.
pub fn eqmac_damage(spawn_id: u16, hazard: Hazard, amount: u32) -> Result<EncodedCommand> {
    check(spawn_id, amount)?;
    let Some(code) = hazard.code(false) else {
        anyhow::bail!("no EQMac damage type is known for {hazard:?}");
    };
    let mut body = vec![0; EQMAC_SIZE];
    body[..2].copy_from_slice(&spawn_id.to_le_bytes());
    body[4] = code;
    body[6..8].copy_from_slice(&u16::MAX.to_le_bytes());
    body[8..12].copy_from_slice(&amount.to_le_bytes());
    Ok(EncodedCommand {
        opcode: EQMAC_DAMAGE_OPCODE,
        body,
    })
}

/// The notice that the player's Safe Fall skill lessened a fall
/// (`OP_SafeFallSuccess`, no body). Both programs send it whenever the
/// player has the skill and the fall would hurt, before the damage, even
/// when the skill leaves none (from the program: Titanium sitting 6, T2, and
/// TAKP Batch 1 R3; checklist 6.147). Both servers only check the skill for
/// a raise on it (`EQMacEmu` and `EQEmu` `Handle_OP_SafeFallSuccess` in
/// `zone/client_packet.cpp`, Settle results S3).
#[must_use]
pub const fn safe_fall(titanium: bool) -> EncodedCommand {
    EncodedCommand {
        opcode: if titanium {
            SAFE_FALL_OPCODE
        } else {
            EQMAC_SAFE_FALL_OPCODE
        },
        body: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titanium_reports_the_amount_and_type_where_the_server_reads_them() {
        let fall = titanium_damage(7, Hazard::Falling, 160).unwrap();
        assert_eq!(fall.opcode, ENV_DAMAGE_OPCODE);
        let mut expected = vec![0; 31];
        expected[0] = 7;
        expected[6] = 160;
        expected[22] = 252;
        expected[27] = 0xff;
        expected[28] = 0xff;
        assert_eq!(fall.body, expected);
        let lava = titanium_damage(7, Hazard::Lava, 300).unwrap();
        assert_eq!((lava.body[6], lava.body[7], lava.body[22]), (44, 1, 250));
        assert_eq!(
            titanium_damage(7, Hazard::Drowning, 1).unwrap().body[22],
            251
        );
        // No Titanium type is known for freezing.
        assert!(titanium_damage(7, Hazard::Freezing, 10).is_err());
    }

    #[test]
    fn eqmac_reports_as_the_program_lays_its_pair_out() {
        let notice = eqmac_notice(300, Hazard::Falling, 160).unwrap();
        assert_eq!(notice.opcode, EQMAC_ENV_DAMAGE_OPCODE);
        let mut expected = vec![0; 36];
        expected[..2].copy_from_slice(&300u16.to_le_bytes());
        expected[8] = 160;
        expected[24] = 252;
        expected[30..32].copy_from_slice(&[0xff, 0xff]);
        assert_eq!(notice.body, expected);
        let fall = eqmac_damage(300, Hazard::Falling, 160).unwrap();
        assert_eq!(fall.opcode, EQMAC_DAMAGE_OPCODE);
        let mut expected = vec![0; 24];
        expected[..2].copy_from_slice(&300u16.to_le_bytes());
        expected[4] = 252;
        expected[6..8].copy_from_slice(&[0xff, 0xff]);
        expected[8] = 160;
        assert_eq!(fall.body, expected);
        assert_eq!(eqmac_damage(300, Hazard::Freezing, 5).unwrap().body[4], 246);
        assert_eq!(
            eqmac_notice(300, Hazard::Drowning, 5).unwrap().body[24],
            251
        );
    }

    #[test]
    fn safe_fall_is_an_empty_notice_on_both_generations() {
        assert_eq!(safe_fall(true).opcode, SAFE_FALL_OPCODE);
        assert_eq!(safe_fall(false).opcode, EQMAC_SAFE_FALL_OPCODE);
        assert!(safe_fall(true).body.is_empty() && safe_fall(false).body.is_empty());
    }

    #[test]
    fn a_report_needs_a_spawn_and_an_amount_the_servers_read_as_it_is() {
        for encode in [titanium_damage, eqmac_notice, eqmac_damage] {
            assert!(encode(0, Hazard::Falling, 10).is_err());
            assert!(encode(7, Hazard::Falling, 0).is_err());
            assert!(encode(7, Hazard::Falling, i32::MAX.unsigned_abs()).is_ok());
            assert!(encode(7, Hazard::Falling, i32::MAX.unsigned_abs() + 1).is_err());
        }
    }
}
