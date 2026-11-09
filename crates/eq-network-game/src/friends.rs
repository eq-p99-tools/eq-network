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

/// Asks which of these friends are online.
///
/// # Errors
/// Rejects an empty list, and a name that is empty, longer than the world
/// reads, or holds a comma, NUL or control character.
pub fn titanium_request(names: &[String]) -> Result<EncodedCommand> {
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
    Ok(EncodedCommand {
        opcode: FRIENDS_WHO_OPCODE,
        body,
    })
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
}
