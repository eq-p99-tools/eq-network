//! A banker's coin change: the bank window's Change button asks the banker
//! nearest the player to turn the purse's and the bank's coins into the
//! fewest coins of the largest kinds (`OP_BankerChange`).
//!
//! `EQEmu`'s `Client::Handle_OP_BankerChange` takes the request at either
//! the full record's size or 4 bytes ("Titanium only sends 4 Bytes for
//! this"), and reads nothing from it. It refuses without a word when no
//! banker stands within `USE_NPC_RANGE2`, and otherwise converts the purse
//! and the bank separately, copper up through platinum, and answers with
//! both as they now stand: the record below. The official client's Change
//! button (0x4dd386) sends 4 bytes of whatever its own argument holds, which
//! mean nothing, and checks neither the coins nor the banker; this client
//! sends zeros.
//!
//! Layout reference: `EQEmu`'s Titanium `BankerChange_Struct`
//! (`common/patches/titanium_structs.h`), which Titanium's wire carries as
//! built, and `Client::Handle_OP_BankerChange` (`zone/client_packet.cpp`).
use crate::{command::EncodedCommand, world::Coins};
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_BankerChange`: the request and its answer (Titanium).
pub const CHANGE_OPCODE: u16 = 0x6a5b;
/// The answer's size: the purse's four kinds, then the bank's.
const ANSWER_LENGTH: usize = 32;
/// The request's size as `EQEmu` says the Titanium client sends it.
const REQUEST_LENGTH: usize = 4;

/// The purse and the bank as a banker left them after changing their
/// coins.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Changed {
    /// The purse.
    pub purse: Coins,
    /// The bank.
    pub bank: Coins,
}

/// Asks the banker nearest the player to change the coins (4 bytes).
#[must_use]
pub fn titanium_request() -> EncodedCommand {
    EncodedCommand {
        opcode: CHANGE_OPCODE,
        body: vec![0; REQUEST_LENGTH],
    }
}

fn coins(body: &[u8], at: usize) -> Coins {
    let word = |offset: usize| {
        u32::from_le_bytes([
            body[at + offset],
            body[at + offset + 1],
            body[at + offset + 2],
            body[at + offset + 3],
        ])
    };
    Coins {
        platinum: word(0),
        gold: word(4),
        silver: word(8),
        copper: word(12),
    }
}

/// Decodes the banker's answer.
///
/// # Errors
/// Rejects an answer of any size but the record's.
pub fn decode(body: &[u8]) -> Result<Changed> {
    ensure!(
        body.len() == ANSWER_LENGTH,
        "a banker's change is {ANSWER_LENGTH} bytes, not {}",
        body.len()
    );
    Ok(Changed {
        purse: coins(body, 0),
        bank: coins(body, 16),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_is_four_bytes_and_the_answer_names_purse_then_bank() {
        let request = titanium_request();
        assert_eq!(request.opcode, 0x6a5b);
        assert_eq!(request.body, [0; 4]);
        let mut body = Vec::new();
        for value in [12_u32, 3, 4, 5, 900, 0, 0, 7] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        let changed = decode(&body).unwrap();
        assert_eq!(
            changed.purse,
            Coins {
                platinum: 12,
                gold: 3,
                silver: 4,
                copper: 5
            }
        );
        assert_eq!(
            changed.bank,
            Coins {
                platinum: 900,
                gold: 0,
                silver: 0,
                copper: 7
            }
        );
        assert!(decode(&body[..31]).is_err());
        assert!(decode(&[0; 4]).is_err());
    }
}
