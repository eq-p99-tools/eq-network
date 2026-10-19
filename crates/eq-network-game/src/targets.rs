//! The player's target as the server sets it, and whom that target targets.
//!
//! The servers set the player's target themselves at times, with
//! `OP_TargetCommand`: `EQEmu` as a sense spell or a corpse locator picks a
//! spawn (`zone/spell_effects.cpp`, `Client::LocateCorpse`) and from Perl's
//! `SendTargetCommand`; TAKP and Quarm as they refuse a mouse target,
//! putting back the last target or none, and they echo the player's own
//! `OP_TargetMouse` as the player clears their target
//! (`Client::Handle_OP_TargetCommand`). `EQEmu` tells the player whom their
//! target targets (`OP_TargetHoTT`) as the player's target changes and as
//! the target's own does (`EntityList::UpdateHoTT`).
//!
//! Layout reference: `ClientTarget_Struct`, a 32-bit spawn on Titanium
//! (`EQEmu`'s `common/eq_packet_structs.h`) and a 16-bit one on `EQMac`
//! (TAKP's).
use anyhow::{Context, Result};

/// `OP_TargetCommand`: the target the server sets, or 0 for none.
pub const TARGET_COMMAND_OPCODE: u16 = 0x1477;
/// `EQMac`'s `OP_TargetCommand`.
pub const EQMAC_TARGET_COMMAND_OPCODE: u16 = 0xfe41;
/// `EQMac`'s `OP_TargetMouse`, the player's own, which TAKP and Quarm echo
/// as the player clears their target.
pub const EQMAC_TARGET_MOUSE_OPCODE: u16 = 0x6241;
/// `OP_TargetHoTT`: whom the player's target targets, or 0 for no one.
pub const TARGET_OF_TARGET_OPCODE: u16 = 0x6a12;

/// Decodes Titanium's target: a spawn in 32 bits, none for 0.
///
/// # Errors
/// Rejects any length other than four bytes, and a spawn beyond 16 bits.
pub fn titanium_target(body: &[u8]) -> Result<Option<u16>> {
    let bytes: [u8; 4] = body.try_into().context("invalid target length")?;
    let spawn = u16::try_from(u32::from_le_bytes(bytes)).context("target beyond any spawn")?;
    Ok((spawn != 0).then_some(spawn))
}

/// Decodes `EQMac`'s target: a spawn in 16 bits, none for 0.
///
/// # Errors
/// Rejects any length other than two bytes.
pub fn eqmac_target(body: &[u8]) -> Result<Option<u16>> {
    let bytes: [u8; 2] = body.try_into().context("invalid EQMac target length")?;
    let spawn = u16::from_le_bytes(bytes);
    Ok((spawn != 0).then_some(spawn))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_read_each_generations_width_and_zero_is_none() {
        assert_eq!(titanium_target(&581u32.to_le_bytes()).unwrap(), Some(581));
        assert_eq!(titanium_target(&[0; 4]).unwrap(), None);
        assert!(titanium_target(&65536u32.to_le_bytes()).is_err());
        assert_eq!(eqmac_target(&581u16.to_le_bytes()).unwrap(), Some(581));
        assert_eq!(eqmac_target(&[0; 2]).unwrap(), None);
        for body in [&[0u8; 2][..], &[0; 5]] {
            assert!(titanium_target(body).is_err());
        }
        for body in [&[0u8; 1][..], &[0; 4]] {
            assert!(eqmac_target(body).is_err());
        }
    }
}
