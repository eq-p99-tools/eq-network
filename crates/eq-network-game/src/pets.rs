//! Pets: the player's commands to their pet (`OP_PetCommands`), whose pet a
//! spawn is (the spawn record's owner, and charm's appearance updates), and
//! the pet's buffs (`OP_PetBuffWindow`). The pet's answers are the server's
//! own string table messages.
//!
//! Layout reference: `EQEmu`'s `PetCommand_Struct` and `PetBuff_Struct`
//! (`common/patches/titanium_structs.h`), the Titanium command numbers its
//! decoder translates (`DECODE(OP_PetCommands)` in
//! `common/patches/titanium.cpp`), and `Client::Handle_OP_PetCommands`
//! (`zone/client_packet.cpp`), which aims an attack at the target the
//! command names.
use crate::command::EncodedCommand;
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_PetCommands`: a command to the player's pet.
pub const COMMAND_OPCODE: u16 = 0x10a1;
/// `OP_PetBuffWindow`: the pet's buffs.
pub const BUFFS_OPCODE: u16 = 0x4e31;
/// Buff slots a pet has.
pub const BUFF_SLOTS: usize = 30;

/// A command to a pet, numbered as the Titanium client sends it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum PetCommand {
    /// Stop attacking.
    BackOff,
    /// Release the pet.
    GetLost,
    /// Say how hurt it is.
    Health,
    /// Guard where it stands.
    GuardHere,
    /// Attack the target the command names.
    Attack,
    /// Follow the player.
    Follow,
    /// Sit down.
    Sit,
    /// Stand up.
    Stand,
    /// Taunt or stop taunting.
    Taunt,
    /// Hold, attacking only when told.
    Hold,
    /// Taunt.
    TauntOn,
    /// Stop taunting.
    TauntOff,
    /// Say whose pet it, or the target, is.
    Leader,
    /// Feign death.
    Feign,
    /// Cast spells, or stop casting them.
    NoCast,
    /// Cast only on its target, or on anyone.
    Focus,
}

impl PetCommand {
    /// Every command, in the Titanium client's order.
    pub const ALL: [Self; 16] = [
        Self::BackOff,
        Self::GetLost,
        Self::Health,
        Self::GuardHere,
        Self::Attack,
        Self::Follow,
        Self::Sit,
        Self::Stand,
        Self::Taunt,
        Self::Hold,
        Self::TauntOn,
        Self::TauntOff,
        Self::Leader,
        Self::Feign,
        Self::NoCast,
        Self::Focus,
    ];

    /// The Titanium client's number for the command.
    #[must_use]
    pub const fn wire(self) -> u32 {
        match self {
            Self::BackOff => 1,
            Self::GetLost => 2,
            Self::Health => 4,
            Self::GuardHere => 5,
            Self::Attack => 7,
            Self::Follow => 8,
            Self::Sit => 9,
            Self::Stand => 10,
            Self::Taunt => 11,
            Self::Hold => 12,
            Self::TauntOn => 13,
            Self::TauntOff => 14,
            Self::Leader => 16,
            Self::Feign => 17,
            Self::NoCast => 18,
            Self::Focus => 19,
        }
    }

    /// Whether the command names the player's target: an attack aims at it,
    /// and asking for a leader asks about it.
    #[must_use]
    pub const fn names_target(self) -> bool {
        matches!(self, Self::Attack | Self::Leader)
    }
}

/// A command to the player's pet (`PetCommand_Struct`), naming a target
/// where the command takes one.
#[must_use]
pub fn command(command: PetCommand, target: Option<u16>) -> EncodedCommand {
    let mut body = command.wire().to_le_bytes().to_vec();
    body.extend_from_slice(&u32::from(target.unwrap_or(0)).to_le_bytes());
    EncodedCommand {
        opcode: COMMAND_OPCODE,
        body,
    }
}

/// One of a pet's buffs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct PetBuff {
    /// The spell.
    pub spell_id: u32,
    /// Ticks left; negative for a buff that does not wear off.
    pub ticks: i32,
}

/// A pet's buffs, in their slots.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PetBuffs {
    /// The pet's spawn.
    pub pet: u16,
    /// Each slot's buff.
    pub slots: Vec<Option<PetBuff>>,
}

fn word(body: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        body[offset],
        body[offset + 1],
        body[offset + 2],
        body[offset + 3],
    ])
}

/// Decodes `OP_PetBuffWindow` (`PetBuff_Struct`: the pet, 30 spells, 30 tick
/// counts and how many there are).
///
/// # Errors
/// Rejects a malformed length or pet.
pub fn decode_buffs(body: &[u8]) -> Result<PetBuffs> {
    ensure!(body.len() == 248, "invalid pet buff length");
    let pet = u16::try_from(word(body, 0))?;
    Ok(PetBuffs {
        pet,
        slots: (0..BUFF_SLOTS)
            .map(|slot| {
                let spell_id = word(body, 4 + slot * 4);
                (spell_id != 0 && spell_id != u32::MAX).then(|| PetBuff {
                    spell_id,
                    ticks: word(body, 124 + slot * 4).cast_signed(),
                })
            })
            .collect(),
    })
}

/// Whose pet a Titanium spawn record (`Spawn_Struct`, 385 bytes) says the
/// spawn is: its owner's spawn at offset 189.
pub(crate) fn titanium_owner(record: &[u8]) -> Option<u16> {
    u16::try_from(word(record, 189))
        .ok()
        .filter(|owner| *owner != 0)
}

/// The owner a charm's appearance update (`AppearanceType::Pet`, 25) gives a
/// spawn; None as the charm breaks.
pub(crate) const PET_APPEARANCE: u16 = 25;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_carry_the_titanium_number_and_the_target() {
        assert_eq!(
            command(PetCommand::Attack, Some(42)),
            EncodedCommand {
                opcode: COMMAND_OPCODE,
                body: vec![7, 0, 0, 0, 42, 0, 0, 0],
            }
        );
        assert_eq!(
            command(PetCommand::BackOff, None).body,
            [1, 0, 0, 0, 0, 0, 0, 0]
        );
        let numbers: Vec<u32> = PetCommand::ALL.map(PetCommand::wire).into();
        assert_eq!(
            numbers,
            [1, 2, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 18, 19]
        );
        assert!(PetCommand::Attack.names_target() && !PetCommand::Follow.names_target());
    }

    #[test]
    fn pet_buffs_fill_their_slots() {
        let mut body = vec![0; 248];
        body[..4].copy_from_slice(&77u32.to_le_bytes());
        body[8..12].copy_from_slice(&312u32.to_le_bytes());
        body[128..132].copy_from_slice(&(-1i32).to_le_bytes());
        body[244..248].copy_from_slice(&1u32.to_le_bytes());
        let buffs = decode_buffs(&body).unwrap();
        assert_eq!(buffs.pet, 77);
        assert_eq!(buffs.slots.len(), BUFF_SLOTS);
        assert_eq!(buffs.slots[0], None);
        assert_eq!(
            buffs.slots[1],
            Some(PetBuff {
                spell_id: 312,
                ticks: -1
            })
        );
        assert!(decode_buffs(&body[..247]).is_err());
        let mut record = vec![0; 385];
        assert_eq!(titanium_owner(&record), None);
        record[189..193].copy_from_slice(&9u32.to_le_bytes());
        assert_eq!(titanium_owner(&record), Some(9));
    }
}
