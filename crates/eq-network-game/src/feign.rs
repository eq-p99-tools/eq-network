//! Feign Death and the lying posture, as each client generation's program
//! handles them. Read for Titanium in program reads R19 and R19b
//! (`eqgame.exe`); the Mac-era client keeps this library's earlier rules
//! until its own program is read.
//!
//! On the reference server (`EQEmu`) a feign is one flag
//! (`Mob::SetFeigned`). It sends nothing when the feign succeeds, so the
//! player lies down only by the client's own posture report, and it clears
//! the flag when the client reports any other posture, never when the player
//! is hit. Another server may clear it or stand the player itself; the
//! session follows such a stand through the posture report, as the program
//! does.
use crate::command::EncodedCommand;
use anyhow::{ensure, Result};

/// Titanium's `OP_FeignDeath`.
pub const TITANIUM_OPCODE: u16 = 0x7489;

/// Titanium's `OP_SpawnAppearance`.
const TITANIUM_APPEARANCE: u16 = 0x7c32;

/// The posture appearance type, and the lying posture's value.
const POSTURE_TYPE: u16 = 14;
const LYING: u32 = 115;

/// How a client generation's program treats Feign Death and a player who
/// lies on the ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LyingRules {
    /// The press turns auto attack off between the lying posture and the
    /// feign request (Titanium R19 a: 0x4a2f7a(0)). It sends nothing of its
    /// own (R19b 3).
    pub press_ends_attack: bool,
    /// A lying player's own movement moves nothing, and nothing the player
    /// moves stands them up: forward, back, strafe, keyboard turning and
    /// jump wait on the can-move flag, which is off unless the player stands
    /// or ducks (Titanium R19 c, 0x4a1d74). Turning by mouse look in the
    /// chase views goes on. Off, a move stands the player first.
    pub holds_still: bool,
    /// Any `OP_Damage` record of more than 0 to the player stands them when
    /// they sit or kneel to loot; lying and ducking are left alone
    /// (Titanium R19 b and R19b 2, 0x4a56e3, rechecked 11-10). Whatever
    /// the damage's type.
    pub hit_stands: bool,
    /// The line that refuses a cast while the player lies, if the program
    /// refuses one (Titanium R19 d, 0x411f3d: `eqstr_us.txt` 0x3099).
    pub casting_refusal: Option<u32>,
}

/// Titanium's rules (program reads R19 and R19b).
pub const TITANIUM: LyingRules = LyingRules {
    press_ends_attack: true,
    holds_still: true,
    hit_stands: true,
    casting_refusal: Some(0x3099),
};

/// The Mac-era client's rules: unread, so this library's earlier ones,
/// where a move stands a lying player and nothing else changes.
pub const EQMAC: LyingRules = LyingRules {
    press_ends_attack: false,
    holds_still: false,
    hit_stands: false,
    casting_refusal: None,
};

/// Titanium's lying posture report, which the press sends before the feign
/// request (R19 a): `OP_SpawnAppearance` {u16 spawn id, u16 type 14, u32
/// 115}.
///
/// # Errors
/// Rejects a zero spawn ID.
pub fn titanium_posture(spawn_id: u16) -> Result<EncodedCommand> {
    ensure!(spawn_id != 0, "the lying posture needs the player's spawn");
    let mut body = spawn_id.to_le_bytes().to_vec();
    body.extend(POSTURE_TYPE.to_le_bytes());
    body.extend(LYING.to_le_bytes());
    Ok(EncodedCommand {
        opcode: TITANIUM_APPEARANCE,
        body,
    })
}

/// Titanium's feign request, 12 bytes as the program builds it (R19 a):
/// {u32 own spawn id, u32 [0x923848], u8 0}, and three bytes this library
/// sends as zeros (unread). [0x923848] is written only by the program's own
/// damage reports (falls, lava, door contact, drowning), whose source is 0
/// (R19b 1 and its recheck), so it is sent as 0: inferred, recording
/// checklist 6.455.
///
/// # Errors
/// Rejects a zero spawn ID.
pub fn titanium_request(own_id: u16) -> Result<EncodedCommand> {
    ensure!(own_id != 0, "feign death needs the player's spawn");
    let mut body = u32::from(own_id).to_le_bytes().to_vec();
    body.extend([0; 8]);
    Ok(EncodedCommand {
        opcode: TITANIUM_OPCODE,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titanium_lies_down_with_its_posture_report() {
        let sent = titanium_posture(0x1234).unwrap();
        assert_eq!(sent.opcode, 0x7c32);
        assert_eq!(sent.body, [0x34, 0x12, 14, 0, 115, 0, 0, 0]);
        assert!(titanium_posture(0).is_err());
    }

    #[test]
    fn titanium_feigns_with_the_programs_twelve_bytes() {
        let sent = titanium_request(0x1234).unwrap();
        assert_eq!(sent.opcode, 0x7489);
        assert_eq!(sent.body, [0x34, 0x12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert!(titanium_request(0).is_err());
    }

    #[test]
    fn the_lying_request_goes_out_on_both_generations() {
        let sender = crate::request::Sender {
            name: "Player",
            spawn_id: Some(7),
        };
        let request = crate::request::Request::FeignDeathPosture;
        assert_eq!(
            crate::request::titanium(&request, sender).unwrap(),
            titanium_posture(7).unwrap()
        );
        assert_eq!(
            crate::request::eqmac(&request, sender).unwrap(),
            crate::abilities::eqmac::feign_posture(7).unwrap()
        );
    }

    #[test]
    fn only_titanium_is_read() {
        assert_eq!(
            TITANIUM,
            LyingRules {
                press_ends_attack: true,
                holds_still: true,
                hit_stands: true,
                casting_refusal: Some(0x3099),
            }
        );
        assert_eq!(
            EQMAC,
            LyingRules {
                press_ends_attack: false,
                holds_still: false,
                hit_stands: false,
                casting_refusal: None,
            }
        );
    }
}
