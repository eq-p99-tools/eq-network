//! Tracking on the `EQMac` wire (6.603 G3, offered 6.619; source-derived,
//! not live).
//!
//! TAKP and Quarm send no track list: the `EQMac` program builds the window
//! itself from the spawns it knows, and follows one on its own. Their
//! `Client::Handle_OP_Track` (TAKP 44afcbf, Quarm fc75fcf) only gives the
//! Tracking skill a chance to rise (a zero skill becomes one), reads no
//! body, and **kicks** any player not a Ranger, Druid or Bard. So [`press`]
//! refuses every other class.
//!
//! The program sends it empty, once per press of the Tracking ability,
//! after building its list; no skill roll sends another (TAKP program read
//! row 10d: the ability routine 0x4bdf2f, skill 0x35). It is offered as the
//! `EQMac` Tracking ability ([`crate::abilities::eqmac`]). The list itself
//! is the host's to build (row 10c).
use crate::command::EncodedCommand;
use anyhow::{ensure, Result};

/// `EQMac`'s `OP_Track`.
pub const OPCODE: u16 = 0x8441;

/// The classes both servers let send it, by the servers' class numbers:
/// Ranger, Druid and Bard.
pub const TRACKERS: [u32; 3] = [4, 6, 8];

/// `OP_Track` for a player of `class`, empty (row 10d).
///
/// # Errors
/// Refuses every class but a Ranger, Druid or Bard, whom both servers
/// would kick for it.
pub fn press(class: u32) -> Result<EncodedCommand> {
    ensure!(
        TRACKERS.contains(&class),
        "only a Ranger, Druid or Bard may track"
    );
    Ok(EncodedCommand {
        opcode: OPCODE,
        body: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_ranger_druid_or_bard_presses_tracking() {
        for class in TRACKERS {
            let packet = press(class).unwrap();
            assert_eq!((packet.opcode, packet.body), (0x8441, vec![]));
        }
        // Every other class, and every number no class has: a send would
        // be a kick.
        for class in (0..=u32::from(u16::MAX)).filter(|class| !TRACKERS.contains(class)) {
            assert!(press(class).is_err(), "{class}");
        }
        assert!(press(u32::MAX).is_err());
    }
}
