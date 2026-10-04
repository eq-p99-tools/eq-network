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
//! `EQMac` reports the world's damage in the packet its server sends damage
//! records in, [`EQMAC_DAMAGE_OPCODE`](crate::combat::EQMAC_DAMAGE_OPCODE).
use crate::{combat::EQMAC_DAMAGE_OPCODE, command::EncodedCommand};
use anyhow::{ensure, Result};

/// `OP_EnvDamage`: damage the world did to the player, in Titanium's
/// 31-byte `EnvDamage2_Struct`.
pub const ENV_DAMAGE_OPCODE: u16 = 0x31b3;

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

/// The `EQMac` client's report (`OP_Damage`): the player's spawn as both
/// target and source, the damage type, no spell, the amount, and no push.
///
/// # Errors
/// Refuses a report for no spawn, no damage, and more than `i32::MAX`
/// points of it.
pub fn eqmac_damage(spawn_id: u16, hazard: Hazard, amount: u32) -> Result<EncodedCommand> {
    check(spawn_id, amount)?;
    let Some(code) = hazard.code(false) else {
        anyhow::bail!("no EQMac damage type is known for {hazard:?}");
    };
    let mut body = Vec::with_capacity(EQMAC_SIZE);
    body.extend_from_slice(&spawn_id.to_le_bytes());
    body.extend_from_slice(&spawn_id.to_le_bytes());
    body.extend_from_slice(&u16::from(code).to_le_bytes());
    body.extend_from_slice(&0u16.to_le_bytes());
    body.extend_from_slice(&amount.to_le_bytes());
    // Force, sequence and the push's angle.
    body.resize(EQMAC_SIZE, 0);
    Ok(EncodedCommand {
        opcode: EQMAC_DAMAGE_OPCODE,
        body,
    })
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
    fn eqmac_reports_the_player_as_target_and_source() {
        let fall = eqmac_damage(300, Hazard::Falling, 160).unwrap();
        assert_eq!(fall.opcode, EQMAC_DAMAGE_OPCODE);
        let mut expected = vec![0; 24];
        expected[..2].copy_from_slice(&300u16.to_le_bytes());
        expected[2..4].copy_from_slice(&300u16.to_le_bytes());
        expected[4] = 252;
        expected[8] = 160;
        assert_eq!(fall.body, expected);
        assert_eq!(eqmac_damage(300, Hazard::Freezing, 5).unwrap().body[4], 246);
    }

    #[test]
    fn a_report_needs_a_spawn_and_an_amount_the_servers_read_as_it_is() {
        for encode in [titanium_damage, eqmac_damage] {
            assert!(encode(0, Hazard::Falling, 10).is_err());
            assert!(encode(7, Hazard::Falling, 0).is_err());
            assert!(encode(7, Hazard::Falling, i32::MAX.unsigned_abs()).is_ok());
            assert!(encode(7, Hazard::Falling, i32::MAX.unsigned_abs() + 1).is_err());
        }
    }
}
