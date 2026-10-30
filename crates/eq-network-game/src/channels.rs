//! The chat server a world offers the client, for chat channels and for
//! mail: as the player enters the world, and on `EQEmu` again from the zone
//! when its chat server reconnects. The offer ends with the key the client
//! would log in to the chat server with; nothing here connects, so the key is
//! never read into a value, and nothing kept or told carries it.
//!
//! Layout reference: the text `EQEmu` writes in `world/client.cpp`
//! (`EnterWorld`) and `zone/client.cpp` (`ReconnectUCS`), "host,port,
//! world.character," then a connection letter and the key; TAKP's and
//! Quarm's `world/client.cpp` write the same without the letter.

use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// Titanium's `OP_SetChatServer`: the chat server's offer for channels.
pub const TITANIUM_CHANNELS_OPCODE: u16 = 0x00d7;
/// Titanium's `OP_SetChatServer2`: the chat server's offer for mail.
pub const TITANIUM_MAIL_OPCODE: u16 = 0x6536;
/// `EQMac`'s `OP_SetChatServer`: the chat server's offer, for channels.
pub const EQMAC_CHANNELS_OPCODE: u16 = 0x0980;

/// What an offered chat server is for.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatService {
    /// Chat channels.
    Channels,
    /// Mail.
    Mail,
}

/// A world's offer of its chat server, without the key it carries.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ChatServer {
    /// What the offer is for.
    pub service: ChatService,
    /// The chat server's host name or address.
    pub host: String,
    /// The chat server's port.
    pub port: u16,
    /// The world's short name, as the chat server knows the player by it.
    pub world: String,
    /// The character, as the chat server knows the player by it.
    pub character: String,
}

/// Reads a world's chat server offer for `service`: NUL-terminated text of
/// four fields between commas, the host, the port, the world and character
/// joined by a dot, and the key. The key's field is split off and left
/// unread.
///
/// # Errors
/// Rejects an offer missing a field, with a host that is not plain text, a
/// port that is not a nonzero number, or no world or character. No error
/// repeats what the offer says.
pub fn offer(service: ChatService, body: &[u8]) -> Result<ChatServer> {
    let text = crate::world::until_nul(body);
    // The fourth field, the key, stays where it is.
    let mut fields = text.splitn(4, |&byte| byte == b',');
    let (Some(host), Some(port), Some(login), Some(_)) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        anyhow::bail!("incomplete chat server offer");
    };
    ensure!(
        !host.is_empty() && host.iter().all(u8::is_ascii_graphic),
        "invalid chat server host"
    );
    let port = std::str::from_utf8(port)
        .ok()
        .and_then(|port| port.parse::<u16>().ok())
        .filter(|&port| port != 0)
        .context("invalid chat server port")?;
    // Character names have no dot; the last one ends the world's name.
    let dot = login
        .iter()
        .rposition(|&byte| byte == b'.')
        .context("chat server offer names no world")?;
    let (world, character) = (&login[..dot], &login[dot + 1..]);
    ensure!(
        [world, character]
            .iter()
            .all(|name| !name.is_empty() && name.iter().all(u8::is_ascii_graphic)),
        "invalid chat server world or character"
    );
    Ok(ChatServer {
        service,
        host: String::from_utf8_lossy(host).into_owned(),
        port,
        world: String::from_utf8_lossy(world).into_owned(),
        character: String::from_utf8_lossy(character).into_owned(),
    })
}

/// Reads Titanium's offers by opcode, as `EQEmu`'s zone sends them again
/// when its chat server reconnects: none for another opcode.
///
/// # Errors
/// Rejects a malformed offer, as [`offer`] does.
pub fn titanium(opcode: u16, body: &[u8]) -> Result<Option<ChatServer>> {
    let service = match opcode {
        TITANIUM_CHANNELS_OPCODE => ChatService::Channels,
        TITANIUM_MAIL_OPCODE => ChatService::Mail,
        _ => return Ok(None),
    };
    offer(service, body).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key no test should ever find outside the offer.
    const KEY: &str = "0123ABCD";

    fn body(text: &str) -> Vec<u8> {
        let mut body = text.as_bytes().to_vec();
        body.push(0);
        body
    }

    #[test]
    fn each_generation_offers_host_port_world_and_character() {
        let expected = |service| ChatServer {
            service,
            host: "chat.example.test".into(),
            port: 7778,
            world: "testworld".into(),
            character: "Tester".into(),
        };
        // EQEmu puts a connection letter before the key, TAKP and Quarm none.
        for (service, letter) in [(ChatService::Channels, "B"), (ChatService::Mail, "b")] {
            let offered = offer(
                service,
                &body(&format!(
                    "chat.example.test,7778,testworld.Tester,{letter}{KEY}"
                )),
            )
            .unwrap();
            assert_eq!(offered, expected(service));
        }
        let offered = offer(
            ChatService::Channels,
            &body(&format!("chat.example.test,7778,testworld.Tester,{KEY}")),
        )
        .unwrap();
        assert_eq!(offered, expected(ChatService::Channels));
    }

    #[test]
    fn the_key_never_leaves_the_offer() {
        let offered = offer(
            ChatService::Mail,
            &body(&format!("chat.example.test,7778,testworld.Tester,b{KEY}")),
        )
        .unwrap();
        assert!(!format!("{offered:?}").contains(KEY));
        assert!(!serde_json::to_string(&offered).unwrap().contains(KEY));
    }

    #[test]
    fn malformed_offers_are_refused_without_repeating_them() {
        for text in [
            "chat.example.test,7778,testworld.Tester",
            "chat.example.test,0,testworld.Tester,0123ABCD",
            "chat.example.test,77781,testworld.Tester,0123ABCD",
            "chat.example.test,7778,testworld,0123ABCD",
            "chat.example.test,7778,.Tester,0123ABCD",
            "chat example,7778,testworld.Tester,0123ABCD",
            ",7778,testworld.Tester,0123ABCD",
        ] {
            let error = offer(ChatService::Channels, &body(text))
                .unwrap_err()
                .to_string();
            assert!(
                ["example", "Tester", "testworld", KEY]
                    .iter()
                    .all(|part| !error.contains(part)),
                "{error}"
            );
        }
        assert!(offer(ChatService::Channels, &[]).is_err());
    }
}
