//! Reading books and notes. A readable item names its text on the server;
//! asking for that name brings the text back, which the official client
//! shows in its note or book window by the item's kind. Servers keep the
//! texts, so nothing is read from the client's files.
//!
//! Layout reference: `EQEmu`'s Titanium `BookRequest_Struct` and
//! `BookText_Struct` (`common/patches/titanium_structs.h`, translated in
//! `titanium.cpp`) and the Titanium opcode (`utils/patches/patch_Titanium.conf`);
//! `Client::ReadBook` (`zone/client.cpp`) for the rules.
use crate::command::EncodedCommand;
use anyhow::{ensure, Result};
use serde::Serialize;

/// `OP_ReadBook`: the request for a text, and the text that answers it.
pub const READ_OPCODE: u16 = 0x1496;

/// The window byte that asks for a window of its own.
const NEW_WINDOW: u8 = 0xff;

/// The longest text name servers read; theirs hold 20 bytes with the NUL.
pub const MAX_FILE: usize = 19;

/// The item class of what can be read (`ItemClassBook`), beside common
/// items (0) and containers (1).
const READABLE_CLASS: &str = "2";

/// What a readable item reads as.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Book {
    /// Its text's name on the server.
    pub file: String,
    /// The kind of window it opens: 0 a note or scroll, 1 a book.
    pub kind: u8,
}

impl Book {
    /// What an item's definition says it reads as, from its item class,
    /// its book flag and its text name: an item of the readable class with
    /// a text opens the book window when flagged as a book and the note
    /// window otherwise, as the Tattered Notes new characters carry do.
    /// None for anything else.
    #[must_use]
    pub fn from_item(class: &str, flag: &str, file: &str) -> Option<Self> {
        let file = file.trim();
        (class.trim() == READABLE_CLASS && !file.is_empty() && file != "0").then(|| Self {
            file: file.to_owned(),
            kind: u8::from(flag.trim() != "0"),
        })
    }

    /// Whether it opens the book window rather than the note window.
    #[must_use]
    pub const fn is_book(&self) -> bool {
        self.kind == 1
    }
}

/// A text the server sent to read.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BookText {
    /// The kind of window it is for: 0 a note or scroll, 1 a book.
    pub kind: u8,
    /// The text, with the server's markup as it came.
    pub text: String,
}

/// Encodes asking for a readable item's text, in a window of its own.
///
/// # Errors
/// Rejects a text name too long for servers to read.
pub fn titanium_request(book: &Book) -> Result<EncodedCommand> {
    ensure!(
        !book.file.is_empty() && book.file.len() <= MAX_FILE && book.file.is_ascii(),
        "not a book a server can find"
    );
    let mut body = vec![NEW_WINDOW, book.kind];
    body.extend_from_slice(book.file.as_bytes());
    body.push(0);
    Ok(EncodedCommand {
        opcode: READ_OPCODE,
        body,
    })
}

/// Decodes a text the server sent: the window byte, the kind and the text,
/// up to its NUL.
///
/// # Errors
/// Rejects a body too short to hold the window and the kind.
pub fn titanium_text(body: &[u8]) -> Result<BookText> {
    ensure!(body.len() >= 2, "invalid book text length");
    let text = &body[2..];
    let end = text
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(text.len());
    Ok(BookText {
        kind: body[1],
        text: String::from_utf8_lossy(&text[..end]).into_owned(),
    })
}

/// Decodes a Titanium book packet from the server: a text to read; None for
/// any other opcode.
///
/// # Errors
/// Rejects a text [`titanium_text`] cannot read.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<BookText>> {
    Ok(match opcode {
        READ_OPCODE => Some(titanium_text(body)?),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_readable_item_asks_for_its_text_by_name() {
        // A Tattered Note: the readable class, no book flag, a text.
        let note = Book::from_item("2", "0", "ranger01").unwrap();
        assert!(!note.is_book());
        assert_eq!(
            titanium_request(&note).unwrap(),
            EncodedCommand {
                opcode: READ_OPCODE,
                body: [&[0xff, 0][..], b"ranger01", &[0]].concat(),
            }
        );
        assert!(Book::from_item("2", "1", "Lore1").unwrap().is_book());
        // A common item or a container, even with a text, and a book
        // without one, cannot be read.
        assert_eq!(Book::from_item("0", "1", "ranger01"), None);
        assert_eq!(Book::from_item("1", "0", "ranger01"), None);
        assert_eq!(Book::from_item("2", "1", ""), None);
        let long = Book {
            file: "x".repeat(20),
            kind: 0,
        };
        assert!(titanium_request(&long).is_err());
    }

    #[test]
    fn the_answer_carries_the_kind_and_the_text() {
        let body = [&[0xff, 1][..], b"Page one^Page two", &[0]].concat();
        assert_eq!(
            decode(READ_OPCODE, &body).unwrap(),
            Some(BookText {
                kind: 1,
                text: "Page one^Page two".into(),
            })
        );
        assert_eq!(decode(0x1234, &body).unwrap(), None);
        assert!(titanium_text(&[0xff]).is_err());
    }
}
