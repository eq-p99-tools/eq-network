//! A Warrior's `/shield` of another player: Titanium's `OP_Shielding`
//! (0x3fe6), its one field the target's spawn id (`Shielding_Struct`), as
//! stock `EQEmu`'s `Client::Handle_OP_Shielding` reads it. The server
//! checks the level, class, recast, target and distance and answers each
//! refusal, the start and the end with a line of its own; nothing comes
//! back on this opcode.
//!
//! TAKP's and Quarm's `EQMac` `OP_Shielding` (0x4b42) carries the same one
//! field in 16 bits, and their own `Client::Handle_OP_Shielding` drops any
//! other length (TAKP 44afcbf, Quarm fc75fcf; 6.603 G2, source-derived, not
//! yet checked live). Which spawn the `EQMac` program names is row 10b.

use crate::command::EncodedCommand;
use anyhow::{ensure, Result};

/// `OP_Shielding` (Titanium).
pub const SHIELDING_OPCODE: u16 = 0x3fe6;

/// Titanium's `/shield` of `target`: `OP_Shielding`, 4 bytes, the target's
/// spawn id widened to 32 bits.
///
/// # Errors
/// Returns an error for spawn id 0, which names no one.
pub fn titanium_shield(target: u16) -> Result<EncodedCommand> {
    ensure!(target != 0, "a shield names no one");
    Ok(EncodedCommand {
        opcode: SHIELDING_OPCODE,
        body: u32::from(target).to_le_bytes().to_vec(),
    })
}

/// `OP_Shielding` (`EQMac`, TAKP and Quarm).
pub const EQMAC_SHIELDING_OPCODE: u16 = 0x4b42;

/// `EQMac`'s `/shield` of `target`: `OP_Shielding`, 2 bytes, the target's
/// spawn id.
///
/// # Errors
/// Returns an error for spawn id 0, which names no one.
pub fn eqmac_shield(target: u16) -> Result<EncodedCommand> {
    ensure!(target != 0, "a shield names no one");
    Ok(EncodedCommand {
        opcode: EQMAC_SHIELDING_OPCODE,
        body: target.to_le_bytes().to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mac_shield_names_its_target_in_two_bytes() {
        let packet = eqmac_shield(0x1234).unwrap();
        assert_eq!(packet.opcode, 0x4b42);
        assert_eq!(packet.body, [0x34, 0x12]);
        assert_eq!(eqmac_shield(u16::MAX).unwrap().body, [0xff, 0xff]);
        assert!(eqmac_shield(0).is_err());
    }

    #[test]
    fn a_shield_names_its_target_in_four_bytes() {
        let packet = titanium_shield(0x1234).unwrap();
        assert_eq!(packet.opcode, 0x3fe6);
        assert_eq!(packet.body, [0x34, 0x12, 0, 0]);
        let widest = titanium_shield(u16::MAX).unwrap();
        assert_eq!(widest.body, [0xff, 0xff, 0, 0]);
        assert!(titanium_shield(0).is_err());
    }
}
