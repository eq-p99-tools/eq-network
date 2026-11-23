//! Asking which of the player's friends are online: `/who all friends`
//! sends the names on the player's friends list (`OP_FriendsWho`), and the
//! world answers with an ordinary who listing of those online
//! ([`crate::who`]).
//!
//! The list itself lives with the client, not the server. `EQEmu`'s
//! `ClientList::SendFriendsWho` (`world/clientlist.cpp`) reads the body as
//! a comma-delimited list of names ending at its NUL, gives up on any name
//! longer than 64 bytes, and leaves out game masters listed anonymous.
//!
//! Layout reference: `Client::Handle_OP_FriendsWho` and
//! `Client::FriendsWho` (`zone/client_packet.cpp`, `zone/client.cpp`).
use crate::command::EncodedCommand;
use anyhow::{ensure, Result};

/// `OP_FriendsWho`: the friends to look for (Titanium).
pub const FRIENDS_WHO_OPCODE: u16 = 0x48fe;
/// The longest name the world reads from the list.
const LONGEST_NAME: usize = 64;

/// `OP_FriendsWho` in the `EQMac` program's packets (its 0x41c5).
pub const EQMAC_FRIENDS_WHO_OPCODE: u16 = 0xc541;

/// Asks which of these friends are online.
///
/// # Errors
/// Rejects an empty list, and a name that is empty, longer than the world
/// reads, or holds a comma, NUL or control character.
pub fn titanium_request(names: &[String]) -> Result<EncodedCommand> {
    Ok(EncodedCommand {
        opcode: FRIENDS_WHO_OPCODE,
        body: list(names)?,
    })
}

/// Asks which of these friends are online, in the `EQMac` program's
/// opcode, with the same comma list ending in NUL. Quarm's
/// `Client::Handle_OP_FriendsWho` (fc75fcf) `strcpy`s the body with no
/// length check, so the NUL is what ends it: the zone's copy
/// (`Client::FriendsWho`) goes into a world packet sized
/// `sizeof(ServerFriendsWho_Struct)` plus the list's own `strlen`, so the
/// list always fits once it ends in NUL. TAKP's (44afcbf) reads no further
/// than the body. Both worlds copy each name into a 65-byte `Friend_` and
/// drop the whole list at a name past 64 bytes, which is refused here.
///
/// # Errors
/// As [`titanium_request`].
pub fn eqmac_request(names: &[String]) -> Result<EncodedCommand> {
    Ok(EncodedCommand {
        opcode: EQMAC_FRIENDS_WHO_OPCODE,
        body: list(names)?,
    })
}

/// The names as one comma list ending in its only NUL.
fn list(names: &[String]) -> Result<Vec<u8>> {
    ensure!(!names.is_empty(), "the friends list is empty");
    for name in names {
        ensure!(
            !name.is_empty()
                && name.len() <= LONGEST_NAME
                && !name.chars().any(|c| c == ',' || c.is_control()),
            "a friend's name cannot be sent"
        );
    }
    let mut body = names.join(",").into_bytes();
    body.push(0);
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friends_go_as_one_comma_list_ending_in_nul() {
        let request = titanium_request(&["Alpha".into(), "Beta".into()]).unwrap();
        assert_eq!(request.opcode, 0x48fe);
        assert_eq!(request.body, b"Alpha,Beta\0");
        assert!(titanium_request(&[]).is_err());
        assert!(titanium_request(&["A,B".into()]).is_err());
        assert!(titanium_request(&[String::new()]).is_err());
        assert!(titanium_request(&["x".repeat(65)]).is_err());
    }

    #[test]
    fn eqmac_friends_end_in_their_only_nul_and_no_name_passes_64_bytes() {
        let names = ["Alpha".to_owned(), "x".repeat(64)];
        let request = eqmac_request(&names).unwrap();
        assert_eq!(request.opcode, 0xc541);
        assert_eq!(request.body.last(), Some(&0));
        assert_eq!(
            request.body.iter().position(|byte| *byte == 0),
            Some(request.body.len() - 1)
        );
        assert_eq!(request.body.len(), 5 + 1 + 64 + 1);
        // Each name fits the worlds' 65-byte `Friend_` with its NUL.
        for name in request.body[..request.body.len() - 1].split(|byte| *byte == b',') {
            assert!(name.len() < 65);
        }
        assert!(eqmac_request(&["x".repeat(65)]).is_err());
        assert!(eqmac_request(&["A\0B".to_owned()]).is_err());
        assert!(eqmac_request(&[]).is_err());
    }
}
