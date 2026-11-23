//! The chat server's own wire, for chat channels: the login the client sends
//! with the key from the world's offer ([`crate::channels`]), the commands it
//! sends as text, and what the server tells back.
//!
//! Layout reference: `EQEmu`'s `ucs/clientlist.cpp` (`Clientlist::Process`,
//! `ProcessOPMailCommand`, `Client::JoinChannels`, `SendChannelMessage`,
//! `GeneralChannelMessage`, `AnnounceJoin`) and `utils/patches/
//! mail_opcodes.conf`, which every client generation shares. What the
//! Titanium program itself sends in the bytes the server skips, and the
//! mailbox it names, are inferred (recording checklist 6.509, pending read
//! R49-8 a).
//!
//! The key is kept only in [`Login`], zeroed when it is dropped, and no
//! `Debug`, event or error carries it.

use crate::channels::{offer, ChatServer, ChatService};
use anyhow::{ensure, Result};
use serde::Serialize;
use zeroize::Zeroizing;

/// `OP_MailLogin`: the client's login, and the server's list of mailboxes.
pub const LOGIN_OPCODE: u16 = 0x01;
/// `OP_Mail`: a command from the client as text; from the server, the
/// channels joined, in slot order.
pub const COMMAND_OPCODE: u16 = 0x02;
/// `OP_ChannelMessage`: a channel's message, or a line from the server.
pub const MESSAGE_OPCODE: u16 = 0x03;
/// `OP_ChannelAnnounceJoin`: someone joined a channel.
pub const ANNOUNCE_JOIN_OPCODE: u16 = 0x04;
/// `OP_ChannelAnnounceLeave`: someone left a channel.
pub const ANNOUNCE_LEAVE_OPCODE: u16 = 0x05;
/// `OP_Buddy`: the friends list, which a chat-only connection is not sent.
pub const BUDDY_OPCODE: u16 = 0x06;
/// `OP_Ignore`.
pub const IGNORE_OPCODE: u16 = 0x15;
/// The mail opcodes, `OP_MailHeaderCount` (0x0d) to `OP_MailboxChange`
/// (0x14), which a chat-only connection is not sent.
pub const MAIL_OPCODES: std::ops::RangeInclusive<u16> = 0x0d..=0x14;

/// The most channels a character is on at once (`MAX_JOINED_CHANNELS`).
pub const MAX_CHANNELS: usize = 10;
/// The longest command text sent: the same bound as the zone's chat
/// (reviewer C3).
pub const MAX_COMMAND: usize = crate::chat::outbound_text_limit(crate::GameDialect::Titanium);
/// The longest string read from the server; this crate's own bound.
pub const MAX_TEXT: usize = 4096;

/// The letter before the key in a Titanium chat-channel offer
/// (`ucsTitaniumChat`).
const TITANIUM_CHANNELS_LETTER: u8 = b'B';
/// The digits of the key.
const KEY_DIGITS: usize = 8;
/// What the program puts before the world's name in the mailbox it logs in
/// with; the server keeps only the part after the last dot [I, R49-8 a].
const MAILBOX_PREFIX: &str = "SOE.EQ.";
/// The first byte of the client's login and commands, which the server
/// skips [I, R49-8 a].
const LEAD: u8 = 0;

/// What the client logs in to a Titanium chat server with: the server, and
/// the key, which only [`Login::body`] reads.
pub struct Login {
    server: ChatServer,
    key: Zeroizing<[u8; KEY_DIGITS]>,
}

impl std::fmt::Debug for Login {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Login")
            .field("server", &self.server)
            .finish_non_exhaustive()
    }
}

impl Login {
    /// Reads a world's Titanium chat-channel offer (`OP_SetChatServer`)
    /// with its key; None for an offer of anything else, such as mail, or
    /// one without the Titanium channels letter.
    ///
    /// # Errors
    /// Rejects a malformed offer, as [`offer`] does, or a key that is not
    /// eight hex digits. No error repeats the offer.
    pub fn titanium(body: &[u8]) -> Result<Option<Self>> {
        let server = offer(ChatService::Channels, body)?;
        let text = crate::world::until_nul(body);
        let Some(field) = text.splitn(4, |&byte| byte == b',').nth(3) else {
            anyhow::bail!("incomplete chat server offer");
        };
        let Some((&letter, digits)) = field.split_first() else {
            anyhow::bail!("chat server offer has no key");
        };
        if letter != TITANIUM_CHANNELS_LETTER {
            return Ok(None);
        }
        ensure!(
            digits.len() == KEY_DIGITS && digits.iter().all(u8::is_ascii_hexdigit),
            "invalid chat server key"
        );
        let mut key = Zeroizing::new([0; KEY_DIGITS]);
        key.copy_from_slice(digits);
        Ok(Some(Self { server, key }))
    }

    /// The server offered, without its key.
    #[must_use]
    pub const fn server(&self) -> &ChatServer {
        &self.server
    }

    /// The login's bytes: the lead byte, the mailbox and its NUL, then the
    /// channels letter, the key and a NUL. Zeroed when dropped.
    #[must_use]
    pub fn body(&self) -> Zeroizing<Vec<u8>> {
        let mut body = Zeroizing::new(Vec::with_capacity(
            1 + MAILBOX_PREFIX.len()
                + self.server.world.len()
                + 1
                + self.server.character.len()
                + 2
                + KEY_DIGITS
                + 1,
        ));
        body.push(LEAD);
        body.extend_from_slice(MAILBOX_PREFIX.as_bytes());
        body.extend_from_slice(self.server.world.as_bytes());
        body.push(b'.');
        body.extend_from_slice(self.server.character.as_bytes());
        body.push(0);
        body.push(TITANIUM_CHANNELS_LETTER);
        body.extend_from_slice(&*self.key);
        body.push(0);
        body
    }
}

/// A command's bytes (`OP_Mail`): the lead byte, the text and a NUL.
///
/// # Errors
/// Refuses empty text, a NUL inside it, or more than [`MAX_COMMAND`] bytes.
pub fn command(text: &str) -> Result<Vec<u8>> {
    ensure!(!text.trim().is_empty(), "an empty chat channel command");
    ensure!(!text.contains('\0'), "a chat channel command holds a NUL");
    ensure!(
        text.len() <= MAX_COMMAND,
        "a chat channel command is longer than {MAX_COMMAND} bytes"
    );
    let mut body = Vec::with_capacity(text.len() + 2);
    body.push(LEAD);
    body.extend_from_slice(text.as_bytes());
    body.push(0);
    Ok(body)
}

/// What the chat server tells the client.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelEvent {
    /// Someone's message on a channel.
    Message {
        /// The channel's name.
        channel: String,
        /// Who sent it, without the world's name the server puts first.
        sender: String,
        /// What they said.
        text: String,
    },
    /// A line from the server itself: the channels list, a refusal, help.
    Notice {
        /// The line, as the server words it.
        text: String,
    },
    /// The channels the character is on, in slot order: the first is `/1`.
    Joined {
        /// Each channel's name.
        channels: Vec<String>,
    },
    /// Someone joined a channel the character is on.
    MemberJoined {
        /// The channel's name.
        channel: String,
        /// Who joined.
        name: String,
    },
    /// Someone left a channel the character is on.
    MemberLeft {
        /// The channel's name.
        channel: String,
        /// Who left.
        name: String,
    },
}

/// Reads what the chat server sent: None for what a chat-channel connection
/// leaves unread (the mailboxes, friends, ignore and mail).
///
/// # Errors
/// Rejects a body missing a string, a string longer than [`MAX_TEXT`], more
/// than [`MAX_CHANNELS`] channels joined, or an opcode the chat server does
/// not send.
pub fn read(opcode: u16, body: &[u8]) -> Result<Option<ChannelEvent>> {
    let event = match opcode {
        MESSAGE_OPCODE => {
            let mut strings = Strings(body);
            let (channel, sender, text) = (strings.next()?, strings.next()?, strings.next()?);
            if channel.is_empty() && sender.is_empty() {
                ChannelEvent::Notice { text }
            } else {
                let sender = sender
                    .rsplit_once('.')
                    .map_or(sender.as_str(), |(_, name)| name)
                    .to_owned();
                ChannelEvent::Message {
                    channel,
                    sender,
                    text,
                }
            }
        }
        COMMAND_OPCODE => {
            let list = Strings(body).next()?;
            let channels: Vec<String> = list
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect();
            ensure!(
                channels.len() <= MAX_CHANNELS,
                "more than {MAX_CHANNELS} chat channels joined"
            );
            ChannelEvent::Joined { channels }
        }
        ANNOUNCE_JOIN_OPCODE | ANNOUNCE_LEAVE_OPCODE => {
            let mut strings = Strings(body);
            let (channel, name) = (strings.next()?, strings.next()?);
            if opcode == ANNOUNCE_JOIN_OPCODE {
                ChannelEvent::MemberJoined { channel, name }
            } else {
                ChannelEvent::MemberLeft { channel, name }
            }
        }
        LOGIN_OPCODE | BUDDY_OPCODE | IGNORE_OPCODE => return Ok(None),
        opcode if MAIL_OPCODES.contains(&opcode) => return Ok(None),
        _ => anyhow::bail!("a chat server opcode {opcode:#06x} is not one it sends"),
    };
    Ok(Some(event))
}

/// The chat server link's state, as the session tells it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ChannelLink {
    /// The chat server took the login: commands go out from now on.
    Up,
    /// The link ended and is not tried again until the world offers a chat
    /// server anew; commands are refused meanwhile.
    Lost {
        /// Why.
        reason: LinkLoss,
    },
}

/// Why a chat server link ended. None of them names the server.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkLoss {
    /// The offered address was refused or could not be resolved.
    Address,
    /// No session could be made with the server.
    Connect,
    /// The server ended the link before it took the login.
    Login,
    /// The link failed after the login.
    Dropped,
    /// The server sent something this library cannot read.
    Malformed,
}

/// NUL-ended strings read in turn from a body.
struct Strings<'a>(&'a [u8]);

impl Strings<'_> {
    fn next(&mut self) -> Result<String> {
        let end = self
            .0
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(self.0.len());
        ensure!(
            end < self.0.len() || !self.0.is_empty(),
            "a chat server string is missing"
        );
        ensure!(end <= MAX_TEXT, "a chat server string is too long");
        let text = String::from_utf8_lossy(&self.0[..end]).into_owned();
        self.0 = self.0.get(end + 1..).unwrap_or_default();
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key no test should ever find outside the login body.
    const KEY: &str = "0123ABCD";

    fn offered(text: &str) -> Vec<u8> {
        let mut body = text.as_bytes().to_vec();
        body.push(0);
        body
    }

    fn strings(parts: &[&str]) -> Vec<u8> {
        parts
            .iter()
            .flat_map(|part| part.bytes().chain([0]))
            .collect()
    }

    #[test]
    fn a_titanium_channels_offer_logs_in_with_its_mailbox_and_key() {
        let login = Login::titanium(&offered(&format!(
            "chat.example.test,7778,testworld.Tester,B{KEY}"
        )))
        .unwrap()
        .unwrap();
        assert_eq!(login.server().port, 7778);
        let mut expected = vec![0];
        expected.extend_from_slice(b"SOE.EQ.testworld.Tester\0B");
        expected.extend_from_slice(KEY.as_bytes());
        expected.push(0);
        assert_eq!(*login.body(), expected);
        // The server's check: nine bytes between the mailbox's NUL and the
        // last NUL.
        let mailbox_end = 1 + expected[1..].iter().position(|&b| b == 0).unwrap();
        assert_eq!(expected.len() - mailbox_end - 2, 9);
        assert!(!format!("{login:?}").contains(KEY));
    }

    #[test]
    fn only_a_titanium_channels_offer_with_a_whole_key_is_taken() {
        // Mail, and the generations without a letter, are not connected.
        for text in [
            format!("chat.example.test,7778,testworld.Tester,b{KEY}"),
            format!("chat.example.test,7778,testworld.Tester,C{KEY}"),
        ] {
            assert!(Login::titanium(&offered(&text)).unwrap().is_none());
        }
        for text in [
            "chat.example.test,7778,testworld.Tester,B0123ABC".to_owned(),
            "chat.example.test,7778,testworld.Tester,B0123ABCDE".to_owned(),
            "chat.example.test,7778,testworld.Tester,B0123ABCX".to_owned(),
            "chat.example.test,7778,testworld.Tester,".to_owned(),
            "chat.example.test,7778,testworld.Tester".to_owned(),
        ] {
            let error = Login::titanium(&offered(&text)).unwrap_err().to_string();
            assert!(!error.contains("0123"), "{error}");
        }
    }

    #[test]
    fn commands_go_as_text_after_the_lead_byte() {
        assert_eq!(
            command("join test1, test2").unwrap(),
            b"\0join test1, test2\0"
        );
        assert_eq!(command("2 hello").unwrap(), b"\x002 hello\0");
        assert!(command("").is_err());
        assert!(command("   ").is_err());
        assert!(command("a\0b").is_err());
        assert!(command(&"x".repeat(MAX_COMMAND)).is_ok());
        assert!(command(&"x".repeat(MAX_COMMAND + 1)).is_err());
    }

    #[test]
    fn messages_lines_lists_and_announcements_are_read() {
        assert_eq!(
            read(
                MESSAGE_OPCODE,
                &strings(&["test1", "testworld.Other", "hi there"])
            )
            .unwrap(),
            Some(ChannelEvent::Message {
                channel: "test1".into(),
                sender: "Other".into(),
                text: "hi there".into(),
            })
        );
        assert_eq!(
            read(MESSAGE_OPCODE, &strings(&["", "", "Channels: 1=test1(2)"])).unwrap(),
            Some(ChannelEvent::Notice {
                text: "Channels: 1=test1(2)".into()
            })
        );
        assert_eq!(
            read(COMMAND_OPCODE, &strings(&["test1,test2"])).unwrap(),
            Some(ChannelEvent::Joined {
                channels: vec!["test1".into(), "test2".into()]
            })
        );
        assert_eq!(
            read(COMMAND_OPCODE, &strings(&[""])).unwrap(),
            Some(ChannelEvent::Joined { channels: vec![] })
        );
        assert_eq!(
            read(ANNOUNCE_JOIN_OPCODE, &strings(&["test1", "Other"])).unwrap(),
            Some(ChannelEvent::MemberJoined {
                channel: "test1".into(),
                name: "Other".into()
            })
        );
        assert_eq!(
            read(ANNOUNCE_LEAVE_OPCODE, &strings(&["test1", "Other"])).unwrap(),
            Some(ChannelEvent::MemberLeft {
                channel: "test1".into(),
                name: "Other".into()
            })
        );
        for opcode in [LOGIN_OPCODE, BUDDY_OPCODE, IGNORE_OPCODE, 0x0d, 0x14] {
            assert_eq!(read(opcode, &[1, 0, 0, 0]).unwrap(), None);
        }
        assert!(read(0x30, &[]).is_err());
    }

    #[test]
    fn short_long_and_crowded_bodies_are_refused_without_panicking() {
        let whole = strings(&["test1", "testworld.Other", "hi"]);
        for end in 0..whole.len() - 1 {
            // Every cut either reads (the last string may lack its NUL) or
            // is refused; none panics.
            let _ = read(MESSAGE_OPCODE, &whole[..end]);
        }
        assert!(read(MESSAGE_OPCODE, &[]).is_err());
        assert!(read(MESSAGE_OPCODE, &strings(&["a"])).is_err());
        let long = "x".repeat(MAX_TEXT + 1);
        assert!(read(MESSAGE_OPCODE, &strings(&["", "", &long])).is_err());
        let crowded = (0..=MAX_CHANNELS)
            .map(|n| format!("c{n}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(read(COMMAND_OPCODE, &strings(&[&crowded])).is_err());
    }
}
