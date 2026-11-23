//! Tracking on the `EQMac` wire (6.603 G3, source-derived, built and not
//! offered).
//!
//! TAKP and Quarm send no track list: the `EQMac` program builds the window
//! itself from the spawns it knows, and follows one on its own. Their
//! `Client::Handle_OP_Track` (TAKP 44afcbf, Quarm fc75fcf) only gives the
//! Tracking skill a chance to rise, reads no body, and **kicks** any player
//! not a Ranger, Druid or Bard. So [`press`] refuses every other class.
//!
//! No server type sends it, and nothing may until the `EQMac` program's
//! window and its sends are read (rows 10c, 10d): when it sends this, and
//! with what body, is unread.
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
