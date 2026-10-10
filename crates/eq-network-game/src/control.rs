//! The server's hold on the player: a stun, a charm, its own control of the
//! player on `EQMac`'s servers, and a warrior's berserk.
//!
//! A stun (`OP_Stun`) comes only to the stunned player, with its length, and
//! ends with a stun of 0; one that runs out sends nothing. A charm
//! (`OP_Charm`) comes to the whole zone as a player's charm takes hold or
//! ends, and as the server gives a player back the control it took for a
//! charm or a fear. `EQMac`'s servers say when they take the player's
//! control and give it back (`OP_FreezeClientControl`,
//! `OP_UnfreezeClientControl`, `OP_ReturnClientControl`); `EQEmu` tells a
//! Titanium client with the player's own posture instead
//! ([`PostureState::Frozen`](crate::world::PostureState::Frozen)). TAKP and
//! Quarm tell a warrior as they go berserk and calm down
//! (`OP_BerserkStart`, `OP_BerserkEnd`).
//!
//! Layout references: `EQEmu`'s Titanium `Stun_Struct` and `Charm_Struct`
//! (`common/patches/titanium_structs.h`) and TAKP's
//! (`common/eq_packet_structs.h`); the senders are `Client::Stun` and
//! `Client::UnStun` (`zone/spells.cpp`), `Client::AI_Start` and
//! `Client::AI_Stop` (`zone/mob_ai.cpp`), the charm effect and its fade
//! (`zone/spell_effects.cpp`) and TAKP's `Client::SendBerserkState`
//! (`zone/client.cpp`).
use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use std::time::Duration;

/// `OP_Stun`: a stun's length in milliseconds, or 0 as one ends.
pub const STUN_OPCODE: u16 = 0x1e51;
/// `EQMac`'s `OP_Stun`, the same four bytes.
pub const EQMAC_STUN_OPCODE: u16 = 0x5b41;
/// `OP_Charm`: the owner, the charmed spawn and the command, each 32 bits.
pub const CHARM_OPCODE: u16 = 0x12e5;
/// `EQMac`'s `OP_Charm`: the same fields, each 16 bits.
pub const EQMAC_CHARM_OPCODE: u16 = 0x4442;
/// `EQMac`'s `OP_FreezeClientControl`: the server takes the player's control.
pub const EQMAC_FREEZE_OPCODE: u16 = 0x6a40;
/// `EQMac`'s `OP_UnfreezeClientControl`: the server gives it back.
pub const EQMAC_UNFREEZE_OPCODE: u16 = 0x6b40;
/// `EQMac`'s `OP_ReturnClientControl`, which follows the unfreeze at once.
pub const EQMAC_RETURN_OPCODE: u16 = 0xb040;
/// `EQMac`'s `OP_BerserkStart`: the player, a warrior, goes berserk.
pub const EQMAC_BERSERK_START_OPCODE: u16 = 0xa941;
/// `EQMac`'s `OP_BerserkEnd`: the berserk ends.
pub const EQMAC_BERSERK_END_OPCODE: u16 = 0xaa41;

/// The length of `EQMac`'s freeze and unfreeze: the player's own name, which
/// the session has from its entry, in 65 bytes.
const EQMAC_FREEZE_LENGTH: usize = 65;

/// A charm taking hold or ending, as the whole zone hears it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Charm {
    /// The player who charmed the spawn, or whose charm ended; none when the
    /// server gives a player back the control it took (`Client::AI_Stop`),
    /// whether for a charm or a fear.
    pub owner_id: Option<u16>,
    /// The charmed spawn, or the player given back their control.
    pub pet_id: u16,
    /// True as the charm takes hold, false as it ends. The servers send a
    /// player's charm as it takes hold only when a player cast it; an NPC
    /// charming a player sends nothing until the player's control returns.
    pub charmed: bool,
}

/// Decodes a stun, on either generation: its length, or none as one ends.
///
/// `EQEmu` sends every stun to the player, even one shorter than the stun
/// it keeps running, and TAKP and Quarm every one, though they don't chain
/// a player's stuns; TAKP's and Quarm's crippling blow staggers a player
/// with a stun of 0. A stun that runs out sends nothing.
///
/// # Errors
/// Rejects any length other than four bytes.
pub fn stun(body: &[u8]) -> Result<Option<Duration>> {
    let bytes: [u8; 4] = body.try_into().context("invalid stun length")?;
    let milliseconds = u32::from_le_bytes(bytes);
    Ok((milliseconds != 0).then(|| Duration::from_millis(milliseconds.into())))
}

/// Decodes Titanium's charm: the owner, the charmed spawn and the command,
/// each 32 bits.
///
/// # Errors
/// Rejects any length other than 12 bytes, a spawn beyond 16 bits, no
/// charmed spawn, and a command other than 0 or 1.
pub fn titanium_charm(body: &[u8]) -> Result<Charm> {
    ensure!(body.len() == 12, "invalid charm length");
    let [owner, pet, command] = [0, 4, 8]
        .map(|at| u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]));
    charm(
        u16::try_from(owner).context("charm owner beyond any spawn")?,
        u16::try_from(pet).context("charmed spawn beyond any spawn")?,
        command,
    )
}

/// Decodes `EQMac`'s charm: the owner, the charmed spawn and the command,
/// each 16 bits.
///
/// # Errors
/// Rejects any length other than six bytes, no charmed spawn, and a command
/// other than 0 or 1.
pub fn eqmac_charm(body: &[u8]) -> Result<Charm> {
    ensure!(body.len() == 6, "invalid EQMac charm length");
    let [owner, pet, command] = [0, 2, 4].map(|at| u16::from_le_bytes([body[at], body[at + 1]]));
    charm(owner, pet, command.into())
}

fn charm(owner: u16, pet: u16, command: u32) -> Result<Charm> {
    ensure!(pet != 0, "charm without a charmed spawn");
    let charmed = match command {
        0 => false,
        1 => true,
        _ => bail!("unknown charm command"),
    };
    Ok(Charm {
        owner_id: (owner != 0).then_some(owner),
        pet_id: pet,
        charmed,
    })
}

/// Decodes `EQMac`'s word that the server took the player's control (true)
/// or gave it back (false), from a charm, a fear or link death: its freeze,
/// and the unfreeze and return that end it back to back. Quarm sends the
/// unfreeze and return to every player as a zone shuts down, whether it
/// holds them or not. None for another opcode.
///
/// # Errors
/// Rejects a freeze or unfreeze of a length other than 65 bytes, and a
/// return with a body.
pub fn eqmac_control(opcode: u16, body: &[u8]) -> Result<Option<bool>> {
    let (controlled, length) = match opcode {
        EQMAC_FREEZE_OPCODE => (true, EQMAC_FREEZE_LENGTH),
        EQMAC_UNFREEZE_OPCODE => (false, EQMAC_FREEZE_LENGTH),
        EQMAC_RETURN_OPCODE => (false, 0),
        _ => return Ok(None),
    };
    ensure!(body.len() == length, "invalid EQMac control length");
    Ok(Some(controlled))
}

/// Decodes `EQMac`'s word that the player, a warrior, went berserk (true) or
/// calmed down (false): TAKP and Quarm send it as their health falls below
/// 35% and rises above 45% (their rules `Combat:BerserkerFrenzyStart` and
/// `End`). None for another opcode.
///
/// # Errors
/// Rejects either with a body.
pub fn eqmac_berserk(opcode: u16, body: &[u8]) -> Result<Option<bool>> {
    let berserk = match opcode {
        EQMAC_BERSERK_START_OPCODE => true,
        EQMAC_BERSERK_END_OPCODE => false,
        _ => return Ok(None),
    };
    ensure!(body.is_empty(), "invalid EQMac berserk length");
    Ok(Some(berserk))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stuns_carry_their_length_and_zero_ends_one() {
        assert_eq!(
            stun(&2500u32.to_le_bytes()).unwrap(),
            Some(Duration::from_millis(2500))
        );
        assert_eq!(stun(&[0; 4]).unwrap(), None);
        for body in [&[][..], &[0; 3], &[0; 5]] {
            assert!(stun(body).is_err());
        }
    }

    #[test]
    fn charms_read_each_generation_s_layout() {
        let mut titanium = [0u8; 12];
        titanium[..4].copy_from_slice(&553u32.to_le_bytes());
        titanium[4..8].copy_from_slice(&581u32.to_le_bytes());
        titanium[8..].copy_from_slice(&1u32.to_le_bytes());
        let held = Charm {
            owner_id: Some(553),
            pet_id: 581,
            charmed: true,
        };
        assert_eq!(titanium_charm(&titanium).unwrap(), held);
        let mut eqmac = [0u8; 6];
        eqmac[..2].copy_from_slice(&553u16.to_le_bytes());
        eqmac[2..4].copy_from_slice(&581u16.to_le_bytes());
        eqmac[4..].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(eqmac_charm(&eqmac).unwrap(), held);
        // The server gives the player back their control: no owner.
        titanium[..4].copy_from_slice(&0u32.to_le_bytes());
        titanium[8..].copy_from_slice(&0u32.to_le_bytes());
        eqmac[..2].copy_from_slice(&0u16.to_le_bytes());
        eqmac[4..].copy_from_slice(&0u16.to_le_bytes());
        let returned = Charm {
            owner_id: None,
            pet_id: 581,
            charmed: false,
        };
        assert_eq!(titanium_charm(&titanium).unwrap(), returned);
        assert_eq!(eqmac_charm(&eqmac).unwrap(), returned);
        // An unknown command, no charmed spawn, a spawn beyond 16 bits and
        // the wrong lengths are refused.
        titanium[8..].copy_from_slice(&2u32.to_le_bytes());
        assert!(titanium_charm(&titanium).is_err());
        titanium[8..].copy_from_slice(&0u32.to_le_bytes());
        titanium[4..8].copy_from_slice(&65536u32.to_le_bytes());
        assert!(titanium_charm(&titanium).is_err());
        eqmac[2..4].copy_from_slice(&0u16.to_le_bytes());
        assert!(eqmac_charm(&eqmac).is_err());
        assert!(titanium_charm(&[0; 6]).is_err());
        assert!(eqmac_charm(&[0; 12]).is_err());
    }

    #[test]
    fn eqmac_control_and_berserk_read_their_opcodes_alone() {
        let name = [0u8; 65];
        assert_eq!(
            eqmac_control(EQMAC_FREEZE_OPCODE, &name).unwrap(),
            Some(true)
        );
        assert_eq!(
            eqmac_control(EQMAC_UNFREEZE_OPCODE, &name).unwrap(),
            Some(false)
        );
        assert_eq!(
            eqmac_control(EQMAC_RETURN_OPCODE, &[]).unwrap(),
            Some(false)
        );
        assert_eq!(eqmac_control(EQMAC_STUN_OPCODE, &[]).unwrap(), None);
        assert!(eqmac_control(EQMAC_FREEZE_OPCODE, &name[..64]).is_err());
        assert!(eqmac_control(EQMAC_RETURN_OPCODE, &[0]).is_err());
        assert_eq!(
            eqmac_berserk(EQMAC_BERSERK_START_OPCODE, &[]).unwrap(),
            Some(true)
        );
        assert_eq!(
            eqmac_berserk(EQMAC_BERSERK_END_OPCODE, &[]).unwrap(),
            Some(false)
        );
        assert_eq!(eqmac_berserk(EQMAC_RETURN_OPCODE, &[]).unwrap(), None);
        assert!(eqmac_berserk(EQMAC_BERSERK_END_OPCODE, &[0]).is_err());
    }
}
