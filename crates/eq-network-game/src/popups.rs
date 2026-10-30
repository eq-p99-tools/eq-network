//! What the server puts on the player's screen outside the chat window: a
//! popup window, and large text across the screen that fades in and out (a
//! marquee). Titanium's servers send both; `EQMac`'s send neither.
//!
//! `EQEmu` opens a popup (`OP_OnLevelMessage`, whatever its name) for
//! quests and for game masters' commands such as `#show version`. It shows
//! a marquee (`OP_Marquee`) for quests, bots, a spell effect and the
//! world's broadcasts, and, under its rule `Character:MarqueeHPUpdates`,
//! off by default, for the player's own health. Nothing here answers a
//! popup; its answer, `OP_PopupResponse`, waits for the client pieces.
//!
//! Layout reference: `EQEmu`'s Titanium `OnLevelMessage_Struct`
//! (`common/patches/titanium_structs.h`), which `ENCODE(OP_OnLevelMessage)`
//! fills (`common/patches/titanium.cpp`), and `ClientMarqueeMessage_Struct`
//! (`common/eq_packet_structs.h`), which Titanium's wire carries as built.
//! The senders are `Client::SendPopupToClient`, `Client::SendFullPopup`,
//! `Client::SendWindow` and `Client::SendMarqueeMessage`
//! (`zone/client.cpp`).
use crate::world::{until_nul, WorldEvent};
use anyhow::{ensure, Result};
use serde::Serialize;
use std::time::Duration;

/// `OP_OnLevelMessage`: a popup window (Titanium).
pub const POPUP_OPCODE: u16 = 0x1dde;
/// `OP_Marquee`: large text across the screen (Titanium).
pub const MARQUEE_OPCODE: u16 = 0x1d4d;

/// Titanium's popup record.
const POPUP_LENGTH: usize = 4244;
/// The popup's title field.
const TITLE: usize = 128;
/// The popup's text field.
const TEXT: usize = 4096;
/// The marquee's words before its text.
const MARQUEE_HEADER: usize = 24;

/// A popup window the server opens on the player's screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Popup {
    /// Its title.
    pub title: String,
    /// Its text, in the client's own HTML-like markup, as sent.
    pub text: String,
    /// Its buttons, as the sender gave them: 1 for two, by `EQEmu`'s notes
    /// on later clients' records. Titanium's record drops their names.
    pub buttons: u32,
    /// How long it stays open; None until the player closes it.
    pub duration: Option<Duration>,
    /// Its number, which the player's answer names.
    pub popup_id: u32,
}

/// Large text the server shows across the player's screen, fading in and
/// out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Marquee {
    /// The message type that colours it, as the chat's message types
    /// number them ([`crate::chat::ChatEvent::message_type`]).
    pub message_type: u32,
    /// How opaque it is and how many times it blinks as it fades, by
    /// `EQEmu`'s notes; `EQEmu` sends 510 unless a quest gives another.
    pub priority: u32,
    /// How long it takes to fade in.
    pub fade_in: Duration,
    /// How long it takes to fade out.
    pub fade_out: Duration,
    /// How long it shows.
    pub duration: Duration,
    /// Its text.
    pub text: String,
}

/// What Titanium's popup and marquee messages say; None for other
/// messages.
///
/// # Errors
/// Rejects malformed records of those messages.
pub(crate) fn titanium(opcode: u16, body: &[u8]) -> Result<Option<WorldEvent>> {
    Ok(Some(match opcode {
        POPUP_OPCODE => WorldEvent::Popup(titanium_popup(body)?),
        MARQUEE_OPCODE => WorldEvent::Marquee(titanium_marquee(body)?),
        _ => return Ok(None),
    }))
}

/// A little-endian word of a record.
fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]])
}

/// A text field's text, which a NUL ends within the field.
fn field(bytes: &[u8], what: &str) -> Result<String> {
    ensure!(bytes.contains(&0), "{what} without its end");
    Ok(String::from_utf8_lossy(until_nul(bytes)).into_owned())
}

/// Decodes Titanium's `OP_OnLevelMessage` (4,244 bytes): the title in 128
/// bytes and the text in 4,096, each ended by a NUL within its field; the
/// buttons, how long it stays open in milliseconds (0xffffffff until the
/// player closes it) and the popup's number, each 32 bits; then two words
/// `EQEmu`'s encoder always sets to 0 and 0xffffffff.
///
/// # Errors
/// Rejects a wrong length, and a title or text that fills its field.
fn titanium_popup(body: &[u8]) -> Result<Popup> {
    ensure!(body.len() == POPUP_LENGTH, "invalid popup length");
    let words = TITLE + TEXT;
    let duration = word(body, words + 4);
    Ok(Popup {
        title: field(&body[..TITLE], "popup title")?,
        text: field(&body[TITLE..words], "popup text")?,
        buttons: word(body, words),
        duration: (duration != u32::MAX).then(|| Duration::from_millis(duration.into())),
        popup_id: word(body, words + 8),
    })
}

/// Decodes `OP_Marquee`: the message type, a word `EQEmu` always sets to
/// 10, the priority, and how long the text takes to fade in and out and
/// how long it shows, in milliseconds, each 32 bits; then the text and its
/// NUL, which `EQEmu` sizes the record to fit.
///
/// # Errors
/// Rejects a record too short for its words and one that does not end its
/// text.
fn titanium_marquee(body: &[u8]) -> Result<Marquee> {
    ensure!(body.len() > MARQUEE_HEADER, "truncated marquee");
    ensure!(body.last() == Some(&0), "marquee text without its end");
    let millis = |at: usize| Duration::from_millis(word(body, at).into());
    Ok(Marquee {
        message_type: word(body, 0),
        priority: word(body, 8),
        fade_in: millis(12),
        fade_out: millis(16),
        duration: millis(20),
        text: String::from_utf8_lossy(until_nul(&body[MARQUEE_HEADER..])).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::titanium_update;

    /// A popup's record as `EQEmu`'s Titanium encoder writes it.
    fn popup(title: &[u8], text: &[u8], words: [u32; 3]) -> Vec<u8> {
        let mut body = vec![0; POPUP_LENGTH];
        body[..title.len()].copy_from_slice(title);
        body[TITLE..TITLE + text.len()].copy_from_slice(text);
        for (index, value) in words.into_iter().chain([0, u32::MAX]).enumerate() {
            let at = TITLE + TEXT + index * 4;
            body[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        body
    }

    #[test]
    fn popups_keep_their_title_text_buttons_and_time() {
        // A game master's command: one button, open until closed.
        let body = popup(
            b"Synthetic window",
            b"Line <c \"#00FF00\">one</c>",
            [0, u32::MAX, 0],
        );
        assert_eq!(
            titanium_update(POPUP_OPCODE, &body).unwrap(),
            Some(WorldEvent::Popup(Popup {
                title: "Synthetic window".into(),
                text: "Line <c \"#00FF00\">one</c>".into(),
                buttons: 0,
                duration: None,
                popup_id: 0,
            }))
        );
        // A quest's question with two buttons, open for ten seconds.
        let body = popup(b"A question", b"Will you help?", [1, 10_000, 42]);
        assert_eq!(
            titanium(POPUP_OPCODE, &body).unwrap(),
            Some(WorldEvent::Popup(Popup {
                title: "A question".into(),
                text: "Will you help?".into(),
                buttons: 1,
                duration: Some(Duration::from_secs(10)),
                popup_id: 42,
            }))
        );
        // The longest title and text fit with their NULs; one more byte
        // does not.
        let longest = popup(&[b'a'; TITLE - 1], &[b'b'; TEXT - 1], [0; 3]);
        assert!(titanium(POPUP_OPCODE, &longest).is_ok());
        for at in [TITLE - 1, TITLE + TEXT - 1] {
            let mut full = longest.clone();
            full[at] = b'c';
            assert!(titanium(POPUP_OPCODE, &full).is_err(), "{at}");
        }
        assert!(titanium(POPUP_OPCODE, &body[..POPUP_LENGTH - 1]).is_err());
        assert!(titanium(POPUP_OPCODE, &[body, vec![0]].concat()).is_err());
    }

    #[test]
    fn marquees_keep_their_look_times_and_text() {
        let mut body: Vec<u8> = [15_u32, 10, 510, 0, 3000, 5000]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        body.extend_from_slice(b"A synthetic announcement\0");
        assert_eq!(
            titanium_update(MARQUEE_OPCODE, &body).unwrap(),
            Some(WorldEvent::Marquee(Marquee {
                message_type: 15,
                priority: 510,
                fade_in: Duration::ZERO,
                fade_out: Duration::from_secs(3),
                duration: Duration::from_secs(5),
                text: "A synthetic announcement".into(),
            }))
        );
        assert!(titanium(MARQUEE_OPCODE, &body[..body.len() - 1]).is_err());
        assert!(titanium(MARQUEE_OPCODE, &body[..MARQUEE_HEADER]).is_err());
        assert!(titanium(MARQUEE_OPCODE, &[]).is_err());
        assert_eq!(titanium(0x1234, &body).unwrap(), None);
    }
}
