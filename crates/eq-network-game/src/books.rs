//! Reading books and notes. A readable item names its text on the server;
//! asking for that name brings the text back, which the official client
//! shows in its note or book window by the item's kind. Servers keep the
//! texts, so nothing is read from the client's files.
//!
//! Layout reference: `EQEmu`'s Titanium `BookRequest_Struct` and
//! `BookText_Struct` (`common/patches/titanium_structs.h`, translated in
//! `titanium.cpp`) and the Titanium opcode (`utils/patches/patch_Titanium.conf`);
//! `Client::ReadBook` (`zone/client.cpp`) for the rules.
//! `EQMac` uses `BookRequest_Struct` and `BookText_Struct` from TAKP/Quarm
//! `common/eq_packet_structs.h`: a kind byte without Titanium's window byte.
use crate::command::EncodedCommand;
use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// `OP_ReadBook`: the request for a text, and the text that answers it.
pub const READ_OPCODE: u16 = 0x1496;

/// `OP_ReadBook` on `EQMac`: a kind and text name, or a kind and returned text.
pub const EQMAC_READ_OPCODE: u16 = 0xce40;

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
    /// The received kind: 0 a note or scroll, 1 a book. Other `EQMac` kind
    /// bytes are preserved rather than replaced by the item's Book flag.
    pub kind: u8,
}

impl Book {
    /// What an item's definition says it reads as, from its item class,
    /// its numeric Book flag and text name. Zero selects a note, one a book;
    /// other numeric values stay unsupported rather than becoming a book.
    /// None for an invalid flag, another class or an absent text name.
    #[must_use]
    pub fn from_item(class: &str, flag: &str, file: &str) -> Option<Self> {
        if class.trim() != READABLE_CLASS {
            return None;
        }
        Self::with_file(flag.trim().parse().ok()?, file)
    }

    /// Retain a Titanium readable item's exact metadata so a malformed or
    /// unsupported read can be refused rather than silently becoming a note.
    /// Titanium selects the kind from `Book`; `EQMac` uses `BookType`.
    pub(crate) fn from_titanium_item(class: &str, flag: &str, file: &str) -> Result<Option<Self>> {
        if class.trim() != READABLE_CLASS {
            return Ok(None);
        }
        Ok(Some(Self {
            kind: flag.trim().parse().context("invalid Titanium book flag")?,
            file: file.to_owned(),
        }))
    }

    /// `EQMac`'s native text request uses the `BookType` byte at item offset 228,
    /// not the independent `Book` flag at 230. Preserve the received kind.
    pub(crate) fn from_eqmac_item(class: i16, kind: u8, file: &str) -> Option<Self> {
        if class != 2 {
            return None;
        }
        Self::with_file(kind, file)
    }

    fn with_file(kind: u8, file: &str) -> Option<Self> {
        let file = file.trim();
        (!file.is_empty() && file != "0").then(|| Self {
            file: file.to_owned(),
            kind,
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
/// Rejects unsupported kinds and invalid, missing or overlong text names.
pub fn titanium_request(book: &Book) -> Result<EncodedCommand> {
    // Native inventory invocation passes window 0xff (0x51a53c), and
    // 0x4b1db0 uses the Book byte for note/book, not the signed BookType.
    ensure!(book.kind <= 1, "unsupported Titanium readable item kind");
    ensure!(
        !book.file.is_empty()
            && book.file != "0"
            && book.file.len() <= MAX_FILE
            && book.file.is_ascii()
            && !book.file.contains('\0'),
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
/// Rejects unsupported kinds, missing terminators and truncated headers/text.
pub fn titanium_text(body: &[u8]) -> Result<BookText> {
    ensure!(body.len() >= 3, "invalid book text length");
    // Native 0x45c617 routes 0/1 to note/book; type 2 is item information.
    // The window byte is not consulted for these two fixed windows.
    ensure!(body[1] <= 1, "unsupported Titanium text kind");
    let text = &body[2..];
    let end = text
        .iter()
        .position(|byte| *byte == 0)
        .context("unterminated Titanium book text")?;
    Ok(BookText {
        kind: body[1],
        text: String::from_utf8_lossy(&text[..end]).into_owned(),
    })
}

/// Encodes `EQMac`'s kind, text name and NUL. The native `SendTextRequestMsg`
/// sends the name's length plus two; the server's name array holds 20 bytes.
///
/// # Errors
/// Rejects an empty, non-ASCII, NUL-containing or overlong text name.
pub fn eqmac_request(book: &Book) -> Result<EncodedCommand> {
    ensure!(
        !book.file.is_empty()
            && book.file.len() <= MAX_FILE
            && book.file.is_ascii()
            && !book.file.contains('\0'),
        "not a book a server can find"
    );
    let mut body = vec![book.kind];
    body.extend_from_slice(book.file.as_bytes());
    body.push(0);
    Ok(EncodedCommand {
        opcode: EQMAC_READ_OPCODE,
        body,
    })
}

/// Decodes `EQMac`'s kind byte and server text, without a window byte.
///
/// # Errors
/// Rejects a body too short to contain a kind and text.
pub fn eqmac_text(body: &[u8]) -> Result<BookText> {
    ensure!(body.len() >= 2, "invalid EQMac book text length");
    let text = &body[1..];
    let end = text
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(text.len());
    Ok(BookText {
        kind: body[0],
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
    fn titanium_reads_preserve_metadata_and_refuse_unsupported_or_invalid_names() {
        for kind in [0, 1] {
            let book = Book::from_titanium_item("2", &kind.to_string(), "SyntheticText")
                .unwrap()
                .unwrap();
            assert_eq!(
                titanium_request(&book).unwrap().body,
                [vec![0xff, kind], b"SyntheticText\0".to_vec()].concat()
            );
        }
        assert!(Book::from_titanium_item("2", "unknown", "Text").is_err());
        for flag in [2, 255] {
            let book = Book::from_titanium_item("2", &flag.to_string(), "Text")
                .unwrap()
                .unwrap();
            assert_eq!(book.kind, flag);
            assert!(titanium_request(&book).is_err());
        }
        for file in ["", "0", "a\0b", "\u{e9}", "twentycharacterslong!"] {
            let book = Book::from_titanium_item("2", "0", file).unwrap().unwrap();
            assert_eq!(book.file, file);
            assert!(titanium_request(&book).is_err(), "{file:?}");
        }
        let book = Book::from_titanium_item("2", "0", " Text ")
            .unwrap()
            .unwrap();
        assert_eq!(titanium_request(&book).unwrap().body, b"\xff\0 Text \0");
        let book = Book {
            kind: 1,
            file: "x".repeat(MAX_FILE),
        };
        assert_eq!(titanium_request(&book).unwrap().body.len(), MAX_FILE + 3);
    }

    #[test]
    fn titanium_text_uses_supported_kind_and_terminator_not_a_guessed_window() {
        for kind in [0, 1] {
            for window in [0, 7, 255] {
                let body = [vec![window, kind], b"Synthetic`text\0ignored".to_vec()].concat();
                assert_eq!(
                    titanium_text(&body).unwrap(),
                    BookText {
                        kind,
                        text: "Synthetic`text".into()
                    }
                );
                assert_eq!(titanium_text(&[window, kind, 0]).unwrap().text, "");
            }
        }
        for body in [
            &[][..],
            &[255][..],
            &[255, 0][..],
            &[255, 1, b'x'][..],
            &[255, 2, 0][..],
            &[0, 255, 0][..],
        ] {
            assert!(titanium_text(body).is_err(), "{body:?}");
            assert!(matches!(
                crate::message::titanium(READ_OPCODE, body).as_slice(),
                [crate::message::Message::Unreadable {
                    part: crate::message::Part::World,
                    ..
                }]
            ));
        }
    }

    #[test]
    fn eqmac_requests_and_replies_have_no_titanium_window_byte() {
        for kind in [0, 1, 2] {
            let book = Book {
                file: "SyntheticText".into(),
                kind,
            };
            assert_eq!(
                eqmac_request(&book).unwrap(),
                EncodedCommand {
                    opcode: EQMAC_READ_OPCODE,
                    body: [vec![kind], b"SyntheticText\0".to_vec()].concat(),
                }
            );
            let body = [vec![kind], b"Page one^Page two\0ignored".to_vec()].concat();
            assert_eq!(
                eqmac_text(&body).unwrap(),
                BookText {
                    kind,
                    text: "Page one^Page two".into()
                }
            );
            for messages in [
                crate::message::eqmac(EQMAC_READ_OPCODE, &body),
                crate::message::quarm(EQMAC_READ_OPCODE, &body),
            ] {
                assert!(
                    matches!(messages.as_slice(), [crate::message::Message::Event(crate::world::WorldEvent::BookText(text))] if text.kind == kind && text.text == "Page one^Page two")
                );
            }
        }
        for body in [&[][..], &[0][..]] {
            assert!(eqmac_text(body).is_err());
            assert!(matches!(
                crate::message::eqmac(EQMAC_READ_OPCODE, body).as_slice(),
                [crate::message::Message::Unreadable {
                    part: crate::message::Part::World,
                    ..
                }]
            ));
        }
        assert_eq!(eqmac_text(&[0, 0]).unwrap().text, "");
    }

    #[test]
    fn eqmac_text_names_fit_the_server_array_without_truncation_or_embedded_nul() {
        let mut book = Book {
            file: "x".repeat(MAX_FILE),
            kind: 0,
        };
        assert_eq!(eqmac_request(&book).unwrap().body.len(), 21);
        for file in [
            String::new(),
            "x".repeat(20),
            "a\0b".into(),
            "\u{e9}".into(),
        ] {
            book.file = file;
            assert!(eqmac_request(&book).is_err());
        }
    }

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
