//! Titanium boat-control observations and conservative request correlation.
//!
//! `EQEmu`'s `ControlBoat_Struct` is eight bytes and its control handler echoes
//! accepted requests. Its generic refusal has no boat or request identity.
//! The wire encoder supplies no eligibility or movement authority. In
//! particular, an uncorrelated echo and an expired request never grant control.

mod control;
pub mod riders;
pub use control::{Control, ControlState, Intent, Vessel};

use crate::{movement::CarrierSample, world::Position};
use anyhow::{ensure, Result};
use serde::Serialize;

/// A frontend intent, bound to its accepted rider position and current floor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Request {
    /// Acquire the currently supporting vessel.
    Acquire {
        /// Received report named by the current floor sweep.
        support: CarrierSample,
        /// Locally accepted player position at the sweep.
        position: Position,
    },
    /// Stop local control and release the selected vessel.
    Release,
    /// Refresh or withdraw the supporting vessel while control is outstanding.
    Support(Option<CarrierSample>),
}

/// Encodes an `EQEmu` control intent with canonical zero padding. Native Titanium
/// leaves the trailer uninitialized; zero is a compatibility choice because
/// `EQEmu` never reads it, not a claim about official fixed bytes.
///
/// # Errors
/// Rejects the zero entity ID. This does not establish vessel eligibility.
pub fn titanium(spawn_id: u16, intent: Intent) -> Result<crate::command::EncodedCommand> {
    ensure!(spawn_id != 0, "boat control requires a vessel ID");
    let mut body = vec![0; 8];
    body[..4].copy_from_slice(&u32::from(spawn_id).to_le_bytes());
    body[4] = u8::from(intent == Intent::Acquire);
    Ok(crate::command::EncodedCommand {
        opcode: TITANIUM_OPCODE,
        body,
    })
}

/// Titanium `OP_ControlBoat`; `EQMac`'s different opcode remains unsupported.
pub const TITANIUM_OPCODE: u16 = 0x2c81;

/// The received control byte. Only one selects the boat in Titanium's handler.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Action {
    /// Zero: a release echo, not evidence that this client previously owned it.
    Release,
    /// One: the server echoed an acquire request.
    Acquire,
    /// An unread value, preserved without treating nonzero as acquisition.
    Unknown(u8),
}

/// One exact received control packet, before any admission/request checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ControlBoat {
    /// Full wire entity word; not narrowed to a current zone spawn.
    pub boat_id: u32,
    /// The received action, including unsupported values.
    pub action: Action,
    /// Bytes 5..8: unread by the inspected `EQEmu` handler. Native Titanium's
    /// sender does not initialize them; they are not a nonce or ownership ID.
    pub unknown: [u8; 3],
}

impl ControlBoat {
    /// Reads the complete fixed layout without discarding unknown fields.
    ///
    /// # Errors
    /// Rejects every body length other than eight bytes.
    pub fn titanium(body: &[u8]) -> Result<Self> {
        ensure!(body.len() == 8, "invalid boat-control length");
        Ok(Self {
            boat_id: u32::from_le_bytes(body[..4].try_into()?),
            action: match body[4] {
                0 => Action::Release,
                1 => Action::Acquire,
                value => Action::Unknown(value),
            },
            unknown: body[5..8].try_into()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{message, world::WorldEvent};

    #[test]
    fn dispatch_preserves_unread_fields_and_never_coerces_a_flag() {
        let body = [0x78, 0x56, 0x34, 0x12, 2, 0xaa, 0xbb, 0xcc];
        let expected = ControlBoat {
            boat_id: 0x1234_5678,
            action: Action::Unknown(2),
            unknown: [0xaa, 0xbb, 0xcc],
        };
        assert!(matches!(
            message::titanium(TITANIUM_OPCODE, &body).as_slice(),
            [message::Message::Event(WorldEvent::BoatControl(notice))] if *notice == expected
        ));
        // Zero and a word too wide for a spawn are still received facts.
        assert_eq!(ControlBoat::titanium(&[0; 8]).unwrap().boat_id, 0);
        for length in (0..8).chain(9..12) {
            assert!(matches!(
                message::titanium(TITANIUM_OPCODE, &vec![0; length]).as_slice(),
                [message::Message::Unreadable {
                    part: message::Part::World,
                    ..
                }]
            ));
        }
    }
}
