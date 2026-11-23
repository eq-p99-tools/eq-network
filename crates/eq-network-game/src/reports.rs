//! Reports to the server's staff: a petition, a bug report and, where the
//! server reads it, feedback.
//!
//! Each packet is laid out as the servers that read it take it, and on
//! Titanium as the program builds it (read R49-23, eqgame.exe):
//! - Petition: stock `EQEmu`'s and TAKP's `Client::Handle_OP_Petition`.
//!   The body is the text, ending in a NUL. Titanium's program sends at
//!   most 1023 bytes of it (0x4649eb), from `/petition` and `/guidehelp`
//!   in their text form ([`petition_form`](crate::reports::petition_form)).
//! - Bug report: `Client::Handle_OP_Bug` and `Client::RegisterBug`, with
//!   `EQEmu`'s Titanium `BugReport_Struct` (8736 bytes) and TAKP's
//!   (3340 bytes). The servers take only these exact sizes. Titanium's
//!   program builds it from a zeroed buffer on its window's Send
//!   (0x4e9f7a), with at most 1024 bytes of text.
//! - Feedback: Titanium's program sends 1148 bytes on its window's Send
//!   (0x50157c): the name, an empty field, the player's place and heading
//!   and the text. Stock `EQEmu` has no handler for it and drops it, as it
//!   drops the program's own. `EQMac`'s is TAKP's
//!   `Client::Handle_OP_Feedback` and `Feedback_Struct` (1131 bytes).
//!
//! What this library leaves zero, where the program fills a field from
//! something this library does not have:
//! - The bug report's 128 bytes at 160 (copied from a program buffer that
//!   is not identified; `EQEmu` names it `ui_path`), the time played at 308
//!   (a character field `EQEmu` names `time_played`) and the word at 464
//!   (1 when an unidentified program check is false). The window's four
//!   check boxes are [`BugReport`](crate::reports::BugReport)'s.
//! - The bug report's system information at 4638, where the program writes
//!   a line naming the processor, the video card and its driver: this
//!   library never sends it or anything about the machine, by choice.
//!
//! The program also holds a petition back while one of two of its own
//! flags is set (0x923798, 0x923884). What sets them is unread, so this
//! library never holds one back for them.

use crate::{command::EncodedCommand, GameDialect};

use serde::Serialize;

/// Titanium's `OP_Petition`.
pub const TITANIUM_PETITION_OPCODE: u16 = 0x251f;
/// Titanium's `OP_Bug`.
pub const TITANIUM_BUG_OPCODE: u16 = 0x7ac2;
/// Titanium's `OP_Feedback`.
pub const TITANIUM_FEEDBACK_OPCODE: u16 = 0x5306;
/// `EQMac`'s `OP_Petition` (TAKP's 0x400e, byte-swapped as this crate writes
/// `EQMac` opcodes).
pub const EQMAC_PETITION_OPCODE: u16 = 0x0e40;
/// `EQMac`'s `OP_Bug` (TAKP's 0x40b3).
pub const EQMAC_BUG_OPCODE: u16 = 0xb340;
/// `EQMac`'s `OP_Feedback` (TAKP's 0x413c).
pub const EQMAC_FEEDBACK_OPCODE: u16 = 0x3c41;

/// The longest petition, in bytes before its NUL. The servers take any
/// length; Titanium's program sends at most this (read R49-23, 0x4649eb).
pub const PETITION_LONGEST: usize = 1023;
/// The longest feedback, in bytes before its NUL: `EQMac`'s
/// `Feedback_Struct` message field less its NUL. Titanium's field holds
/// more; how much the program's window takes is unread, so this bound
/// serves both (inferred).
pub const FEEDBACK_LONGEST: usize = 1023;

/// Titanium's `BugReport_Struct`.
const TITANIUM_BUG_LENGTH: usize = 8736;
/// `EQMac`'s `IntelBugStruct`: the reporter at 0 and the text at 107. TAKP
/// and Quarm take it alongside their 3340-byte `BugReport_Struct`, and fill
/// in the player's place and target themselves (6.604 E6). Which layout the
/// `EQMac` program sends is row 11d.
const EQMAC_BUG_LENGTH: usize = 1131;
/// `EQMac`'s `Feedback_Struct`.
const FEEDBACK_LENGTH: usize = 1131;
/// Titanium's feedback (read R49-23, 0x50157c).
const TITANIUM_FEEDBACK_LENGTH: usize = 1148;
/// A name field, in every layout here.
const NAME_FIELD: usize = 64;

/// What a bug report is about, by the names the servers know. `EQEmu`'s
/// Titanium decoder maps the name to its category number and takes an
/// unknown name as Other (`Bug::GetID`). Luclin-on categories (trading cards,
/// mercenaries) are left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BugCategory {
    /// Anything else.
    Other,
    /// What the screen shows.
    Video,
    /// Sound and music.
    Audio,
    /// Where creatures walk.
    Pathing,
    /// A quest.
    Quest,
    /// Tradeskills.
    Tradeskills,
    /// Spell stacking.
    SpellStacking,
    /// Doors and portals.
    DoorsAndPortals,
    /// Items.
    Items,
    /// A creature.
    Npc,
    /// Conversations.
    Dialogs,
}

impl BugCategory {
    /// Every category, in the servers' order.
    pub const ALL: [Self; 11] = [
        Self::Other,
        Self::Video,
        Self::Audio,
        Self::Pathing,
        Self::Quest,
        Self::Tradeskills,
        Self::SpellStacking,
        Self::DoorsAndPortals,
        Self::Items,
        Self::Npc,
        Self::Dialogs,
    ];

    /// The name the servers know the category by.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Other => "Other",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Pathing => "Pathing",
            Self::Quest => "Quest",
            Self::Tradeskills => "Tradeskills",
            Self::SpellStacking => "Spell Stacking",
            Self::DoorsAndPortals => "Doors and Portals",
            Self::Items => "Items",
            Self::Npc => "NPC",
            Self::Dialogs => "Dialogs",
        }
    }

    /// The category a word names, by its server name or that name's first
    /// word, letter case aside ("doors", "spell" and "npc" all name one).
    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        let word = word.trim();
        if word.is_empty() {
            return None;
        }
        Self::ALL.into_iter().find(|category| {
            let name = category.name();
            name.eq_ignore_ascii_case(word)
                || name
                    .split(' ')
                    .next()
                    .is_some_and(|first| first.eq_ignore_ascii_case(word))
        })
    }
}

/// Where the player stood when they reported, as the client knows it: for a
/// bug report, and for Titanium's feedback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BugPlace {
    /// The player's x.
    pub x: f32,
    /// The player's y.
    pub y: f32,
    /// The player's z.
    pub z: f32,
    /// The player's heading in the game's 512 units.
    pub heading: f32,
}

/// The player's target when they reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BugTarget {
    /// The target's spawn.
    pub spawn_id: u16,
    /// The target's name as the server sent it.
    pub name: String,
}

/// A bug report, as the Titanium program's window gathers it (read R49-23,
/// the window's Send 0x4e9f7a): its four check boxes set the mask at 532,
/// worth 1 (it can be repeated), 2 (a crash), 4 (the target) and 8 (the
/// character's flags), as `EQEmu`'s `Bug::InformationFlag` names them.
#[derive(Clone, Debug, PartialEq)]
pub struct BugReport {
    /// What it is about.
    pub category: BugCategory,
    /// What the player says.
    pub text: String,
    /// Where the player stood, when the client knows.
    pub place: Option<BugPlace>,
    /// The player's target, if any: the window's target box.
    pub target: Option<BugTarget>,
    /// The window's "can be repeated" box.
    pub repeatable: bool,
    /// The window's crash box.
    pub crash: bool,
    /// The window's box for sending the character's flags.
    pub character_flags: bool,
}

/// `Bug::InformationFlag::Repeatable`: the bug can be made to happen again.
const REPEATABLE: u32 = 1;
/// `Bug::InformationFlag::Crash`: the bug crashed the game.
const CRASH: u32 = 2;
/// `Bug::InformationFlag::TargetInfo`: the report carries the target.
const TARGET_INFO: u32 = 4;
/// `Bug::InformationFlag::CharacterFlags`: the server is to note the
/// character's flags.
const CHARACTER_FLAGS: u32 = 8;

impl BugReport {
    /// The report's optional-information mask, as `RegisterBug` reads it:
    /// a bit for each of the window's check boxes.
    fn mask(&self) -> u32 {
        [
            (self.repeatable, REPEATABLE),
            (self.crash, CRASH),
            (self.target.is_some(), TARGET_INFO),
            (self.character_flags, CHARACTER_FLAGS),
        ]
        .into_iter()
        .filter(|(set, _)| *set)
        .fold(0, |mask, (_, bit)| mask | bit)
    }
}

/// Which report a session sent or refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum ReportKind {
    /// A petition.
    Petition,
    /// A bug report.
    Bug,
    /// Feedback.
    Feedback,
}

/// How a generation's program takes `/petition`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum PetitionForm {
    /// The command shows or hides the petition window, and its words are
    /// ignored.
    Window,
    /// The command sends its words as a petition ([`petition`]).
    Text,
}

/// How the generation's program takes `/petition` on a world with these
/// rules (`None` before the world has sent any).
///
/// Titanium's program shows its window when the world turns the window on
/// and otherwise sends the words (read R49-23, 0x4683af; the flag at
/// 0x923d64). The world's log-server settings set it, read here as
/// [`crate::rules::WorldRules::petition_window`] (`EQEmu` names the byte
/// `enable_petition_wnd`; that it is the program's flag is inferred). The
/// program also takes the flag from a world packet, 0x3f0d, which no
/// server source here sends, so it is not read. `EQMac`'s program is
/// unread, and the text form stands for it, as it does for `RoF2`, which is
/// not built yet (6.489).
#[must_use]
pub const fn petition_form(
    dialect: GameDialect,
    rules: Option<&crate::rules::WorldRules>,
) -> PetitionForm {
    match dialect {
        GameDialect::Titanium => match rules {
            Some(crate::rules::WorldRules {
                petition_window: Some(true),
                ..
            }) => PetitionForm::Window,
            _ => PetitionForm::Text,
        },
        GameDialect::EqMac | GameDialect::Rof2 => PetitionForm::Text,
    }
}

/// The longest bug report text the generation's program sends, in bytes
/// before its NUL; none for a generation not built yet. Titanium's field
/// holds 2049, and the program sends at most 1024 (read R49-23, 0x4e9f7a).
#[must_use]
pub const fn bug_longest(dialect: GameDialect) -> Option<usize> {
    match dialect {
        GameDialect::Titanium => Some(1024),
        GameDialect::EqMac => Some(1023),
        GameDialect::Rof2 => None,
    }
}

/// Why a report cannot be sent. None of these repeats the report's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportError {
    /// The text is empty once trimmed.
    Empty,
    /// The text holds a NUL, which would end it early.
    HoldsNul,
    /// The text is longer than its field holds, in bytes before its NUL.
    TooLong {
        /// The most the field holds.
        longest: usize,
    },
    /// No server of the generation reads this report, or the generation is
    /// not built yet.
    NotRead,
}

impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("the report is empty"),
            Self::HoldsNul => f.write_str("the report holds a NUL"),
            Self::TooLong { longest } => {
                write!(f, "the report is longer than {longest} bytes")
            }
            Self::NotRead => f.write_str("no server of this generation reads the report"),
        }
    }
}

impl std::error::Error for ReportError {}

/// Text that fits: not empty, no NUL, at most `longest` bytes.
fn checked(text: &str, longest: usize) -> Result<&str, ReportError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(ReportError::Empty);
    }
    if text.contains('\0') {
        return Err(ReportError::HoldsNul);
    }
    if text.len() > longest {
        return Err(ReportError::TooLong { longest });
    }
    Ok(text)
}

/// Writes text into a NUL-ended field, which must hold it.
fn put(body: &mut [u8], at: usize, field: usize, text: &str) -> Result<(), ReportError> {
    if text.len() >= field {
        return Err(ReportError::TooLong { longest: field - 1 });
    }
    body[at..at + text.len()].copy_from_slice(text.as_bytes());
    Ok(())
}

/// Writes a name, cut to its field, as the servers' `strncpy` does.
fn put_name(body: &mut [u8], at: usize, name: &str) {
    let name = name.as_bytes();
    let length = name.len().min(NAME_FIELD - 1);
    body[at..at + length].copy_from_slice(&name[..length]);
}

/// A petition: the text and its NUL.
///
/// # Errors
/// Refuses an empty or over-long text, one holding a NUL, and a generation
/// not built yet.
pub fn petition(dialect: GameDialect, text: &str) -> Result<EncodedCommand, ReportError> {
    let opcode = match dialect {
        GameDialect::Titanium => TITANIUM_PETITION_OPCODE,
        GameDialect::EqMac => EQMAC_PETITION_OPCODE,
        // Not built for RoF2 yet (6.489).
        GameDialect::Rof2 => return Err(ReportError::NotRead),
    };
    let text = checked(text, PETITION_LONGEST)?;
    let mut body = text.as_bytes().to_vec();
    body.push(0);
    Ok(EncodedCommand { opcode, body })
}

/// A bug report from the named character.
///
/// # Errors
/// Refuses an empty or over-long text, one holding a NUL, and a generation
/// not built yet.
pub fn bug(
    dialect: GameDialect,
    reporter: &str,
    report: &BugReport,
) -> Result<EncodedCommand, ReportError> {
    let Some(longest) = bug_longest(dialect) else {
        return Err(ReportError::NotRead);
    };
    let text = checked(&report.text, longest)?;
    let place = report.place.unwrap_or(BugPlace {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        heading: 0.0,
    });
    match dialect {
        GameDialect::Titanium => {
            let mut body = vec![0; TITANIUM_BUG_LENGTH];
            put(&mut body, 0, NAME_FIELD, report.category.name())?;
            put_name(&mut body, 64, reporter);
            // The 128 bytes at 160 stay empty: the program's source buffer
            // is not identified (read R49-23).
            body[288..292].copy_from_slice(&place.x.to_le_bytes());
            body[292..296].copy_from_slice(&place.y.to_le_bytes());
            body[296..300].copy_from_slice(&place.z.to_le_bytes());
            // A whole number of the game's units, as `EQEmu` reads the
            // word; what the program puts here is not identified (read
            // R49-23).
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a heading in 0 to 512"
            )]
            let heading = place.heading.rem_euclid(512.0) as u32;
            body[300..304].copy_from_slice(&heading.to_le_bytes());
            // The time played at 308 and the word at 464 stay zero (read
            // R49-23).
            if let Some(target) = &report.target {
                body[320..324].copy_from_slice(&u32::from(target.spawn_id).to_le_bytes());
                put_name(&mut body, 468, &target.name);
            }
            body[532..536].copy_from_slice(&report.mask().to_le_bytes());
            put(&mut body, 2588, 2050, text)?;
            // The system information at 4638 stays empty by choice: never
            // the machine's processor, card or driver.
            Ok(EncodedCommand {
                opcode: TITANIUM_BUG_OPCODE,
                body,
            })
        }
        // Only the reporter and the player's words: every other byte stays
        // zero, and the server adds the place and target.
        GameDialect::EqMac => {
            let mut body = vec![0; EQMAC_BUG_LENGTH];
            put_name(&mut body, 0, reporter);
            put(&mut body, 107, 1024, text)?;
            Ok(EncodedCommand {
                opcode: EQMAC_BUG_OPCODE,
                body,
            })
        }
        // Not built for RoF2 yet (6.489).
        GameDialect::Rof2 => Err(ReportError::NotRead),
    }
}

/// Feedback from the named character, standing at `place` when the client
/// knows it. Titanium's carries the place and heading (read R49-23,
/// 0x50157c): the position from the spawn's 0x30, 0x34 and 0x38 at 96, 100
/// and 104, the heading rounded to a whole number at 108, and the text at
/// 120. `EQMac`'s carries only the name and the text.
///
/// # Errors
/// Refuses an empty or over-long text, one holding a NUL, and a generation
/// not built yet.
pub fn feedback(
    dialect: GameDialect,
    name: &str,
    text: &str,
    place: Option<BugPlace>,
) -> Result<EncodedCommand, ReportError> {
    match dialect {
        GameDialect::Titanium => {
            let text = checked(text, FEEDBACK_LONGEST)?;
            let place = place.unwrap_or(BugPlace {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                heading: 0.0,
            });
            let mut body = vec![0; TITANIUM_FEEDBACK_LENGTH];
            put_name(&mut body, 0, name);
            // The field at 64 is the program's empty string.
            body[96..100].copy_from_slice(&place.x.to_le_bytes());
            body[100..104].copy_from_slice(&place.y.to_le_bytes());
            body[104..108].copy_from_slice(&place.z.to_le_bytes());
            // Rounded, in the game's 512 units; the word's width is
            // inferred.
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a heading in 0 to 512"
            )]
            let heading = (place.heading.rem_euclid(512.0).round() as u32) % 512;
            body[108..112].copy_from_slice(&heading.to_le_bytes());
            put(&mut body, 120, TITANIUM_FEEDBACK_LENGTH - 120, text)?;
            Ok(EncodedCommand {
                opcode: TITANIUM_FEEDBACK_OPCODE,
                body,
            })
        }
        GameDialect::EqMac => {
            let text = checked(text, FEEDBACK_LONGEST)?;
            let mut body = vec![0; FEEDBACK_LENGTH];
            put_name(&mut body, 0, name);
            put(&mut body, 107, 1024, text)?;
            Ok(EncodedCommand {
                opcode: EQMAC_FEEDBACK_OPCODE,
                body,
            })
        }
        // Not built for RoF2 yet (6.489).
        GameDialect::Rof2 => Err(ReportError::NotRead),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_at(body: &[u8], at: usize) -> &str {
        let end = body[at..].iter().position(|byte| *byte == 0).unwrap();
        std::str::from_utf8(&body[at..at + end]).unwrap()
    }

    fn report() -> BugReport {
        BugReport {
            category: BugCategory::Pathing,
            text: "  The guard walks into the wall.  ".into(),
            place: Some(BugPlace {
                x: 10.5,
                y: -20.25,
                z: 3.0,
                heading: 256.4,
            }),
            target: Some(BugTarget {
                spawn_id: 321,
                name: "Guard_Alpha00".into(),
            }),
            repeatable: false,
            crash: false,
            character_flags: false,
        }
    }

    #[test]
    fn each_check_box_sets_its_bit_in_the_mask() {
        let mut report = report();
        assert_eq!(report.mask(), 4);
        report.target = None;
        assert_eq!(report.mask(), 0);
        let mut one = report.clone();
        one.repeatable = true;
        assert_eq!(one.mask(), 1);
        let mut one = report.clone();
        one.crash = true;
        assert_eq!(one.mask(), 2);
        let mut one = report.clone();
        one.character_flags = true;
        assert_eq!(one.mask(), 8);
        report.repeatable = true;
        report.crash = true;
        report.character_flags = true;
        report.target = Some(BugTarget {
            spawn_id: 1,
            name: "Someone".into(),
        });
        assert_eq!(report.mask(), 15);
        let packet = bug(GameDialect::Titanium, "Reporter", &report).unwrap();
        assert_eq!(&packet.body[532..536], &15u32.to_le_bytes());
    }

    #[test]
    fn a_petition_is_its_text_and_a_nul() {
        let titanium = petition(GameDialect::Titanium, " Stuck under the world. ").unwrap();
        assert_eq!(titanium.opcode, 0x251f);
        assert_eq!(titanium.body, b"Stuck under the world.\0");
        let eqmac = petition(GameDialect::EqMac, "Help").unwrap();
        assert_eq!(eqmac.opcode, 0x0e40);
        assert_eq!(eqmac.body, b"Help\0");
        for (bad, error) in [
            ("", ReportError::Empty),
            ("   ", ReportError::Empty),
            ("a\0b", ReportError::HoldsNul),
        ] {
            assert_eq!(
                petition(GameDialect::Titanium, bad).unwrap_err(),
                error,
                "{bad:?}"
            );
        }
        assert_eq!(
            petition(GameDialect::Titanium, &"x".repeat(PETITION_LONGEST + 1)).unwrap_err(),
            ReportError::TooLong {
                longest: PETITION_LONGEST
            }
        );
        assert!(petition(GameDialect::Titanium, &"x".repeat(PETITION_LONGEST)).is_ok());
        assert!(petition(GameDialect::Titanium, &"x".repeat(PETITION_LONGEST + 1)).is_err());
        assert!(petition(GameDialect::Rof2, "Help").is_err());
    }

    #[test]
    fn titanium_bug_reports_fill_the_servers_struct() {
        let packet = bug(GameDialect::Titanium, "Reporter", &report()).unwrap();
        assert_eq!(packet.opcode, 0x7ac2);
        let body = &packet.body;
        assert_eq!(body.len(), 8736);
        assert_eq!(text_at(body, 0), "Pathing");
        assert_eq!(text_at(body, 64), "Reporter");
        assert_eq!(text_at(body, 160), "");
        assert_eq!(&body[288..292], &10.5f32.to_le_bytes());
        assert_eq!(&body[292..296], &(-20.25f32).to_le_bytes());
        assert_eq!(&body[296..300], &3.0f32.to_le_bytes());
        assert_eq!(&body[300..304], &256u32.to_le_bytes());
        assert_eq!(&body[308..312], &[0; 4]);
        assert_eq!(&body[320..324], &321u32.to_le_bytes());
        assert_eq!(text_at(body, 468), "Guard_Alpha00");
        assert_eq!(&body[532..536], &4u32.to_le_bytes());
        assert_eq!(text_at(body, 2588), "The guard walks into the wall.");
        assert!(body[4638..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn eqmac_bug_reports_carry_only_the_reporter_and_the_words() {
        // A target and a place on the report go nowhere: the server adds
        // its own.
        let packet = bug(GameDialect::EqMac, "Reporter", &report()).unwrap();
        assert_eq!(packet.opcode, 0xb340);
        let body = &packet.body;
        assert_eq!(body.len(), 1131);
        assert_eq!(text_at(body, 0), "Reporter");
        assert!(body["Reporter".len()..107].iter().all(|byte| *byte == 0));
        let words = "The guard walks into the wall.";
        assert_eq!(text_at(body, 107), words);
        assert!(body[107 + words.len()..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn bug_reports_refuse_what_their_field_cannot_hold() {
        for (dialect, longest) in [(GameDialect::Titanium, 1024), (GameDialect::EqMac, 1023)] {
            assert_eq!(bug_longest(dialect), Some(longest));
            let mut long = report();
            long.text = "x".repeat(longest);
            assert!(bug(dialect, "Reporter", &long).is_ok());
            long.text.push('x');
            assert!(bug(dialect, "Reporter", &long).is_err());
            long.text = String::new();
            assert!(bug(dialect, "Reporter", &long).is_err());
        }
        assert!(bug(GameDialect::Rof2, "Reporter", &report()).is_err());
        // A long name is cut to its field, as the servers' copy does.
        let packet = bug(GameDialect::Titanium, &"n".repeat(80), &report()).unwrap();
        assert_eq!(text_at(&packet.body, 64).len(), 63);
    }

    #[test]
    fn feedback_is_laid_out_as_each_generation_sends_it() {
        let place = report().place;
        let packet = feedback(GameDialect::EqMac, "Reporter", "Nice boats.", place).unwrap();
        assert_eq!(packet.opcode, 0x3c41);
        assert_eq!(packet.body.len(), 1131);
        assert_eq!(text_at(&packet.body, 0), "Reporter");
        assert_eq!(text_at(&packet.body, 107), "Nice boats.");
        assert!(feedback(GameDialect::EqMac, "Reporter", &"x".repeat(1024), None).is_err());
        assert!(feedback(GameDialect::Rof2, "Reporter", "Nice boats.", None).is_err());

        let packet = feedback(GameDialect::Titanium, "Reporter", " Nice boats. ", place).unwrap();
        assert_eq!(packet.opcode, 0x5306);
        let body = &packet.body;
        assert_eq!(body.len(), 1148);
        assert_eq!(text_at(body, 0), "Reporter");
        assert!(body["Reporter".len()..96].iter().all(|byte| *byte == 0));
        assert_eq!(&body[96..100], &10.5f32.to_le_bytes());
        assert_eq!(&body[100..104], &(-20.25f32).to_le_bytes());
        assert_eq!(&body[104..108], &3.0f32.to_le_bytes());
        assert_eq!(&body[108..112], &256u32.to_le_bytes());
        assert!(body[112..120].iter().all(|byte| *byte == 0));
        assert_eq!(text_at(body, 120), "Nice boats.");
        assert!(body[120 + "Nice boats.".len()..]
            .iter()
            .all(|byte| *byte == 0));
        // Rounded, and a heading of nearly 512 is 0.
        let mut near = place.unwrap();
        near.heading = 511.7;
        let body = feedback(GameDialect::Titanium, "Reporter", "Hi", Some(near))
            .unwrap()
            .body;
        assert_eq!(&body[108..112], &0u32.to_le_bytes());
        assert!(feedback(GameDialect::Titanium, "Reporter", "", None).is_err());
        assert!(feedback(GameDialect::Titanium, "Reporter", &"x".repeat(1024), None).is_err());
    }

    #[test]
    fn titanium_takes_the_petition_window_from_the_world() {
        use crate::rules::WorldRules;
        let on = WorldRules {
            petition_window: Some(true),
            ..WorldRules::default()
        };
        let off = WorldRules {
            petition_window: Some(false),
            ..WorldRules::default()
        };
        assert_eq!(
            petition_form(GameDialect::Titanium, Some(&on)),
            PetitionForm::Window
        );
        assert_eq!(
            petition_form(GameDialect::Titanium, Some(&off)),
            PetitionForm::Text
        );
        assert_eq!(
            petition_form(GameDialect::Titanium, None),
            PetitionForm::Text
        );
        assert_eq!(
            petition_form(GameDialect::EqMac, Some(&on)),
            PetitionForm::Text
        );
        assert_eq!(
            petition_form(GameDialect::Rof2, Some(&on)),
            PetitionForm::Text
        );
    }

    #[test]
    fn categories_are_named_as_the_servers_know_them() {
        assert_eq!(
            BugCategory::from_word("pathing"),
            Some(BugCategory::Pathing)
        );
        assert_eq!(
            BugCategory::from_word("DOORS"),
            Some(BugCategory::DoorsAndPortals)
        );
        assert_eq!(
            BugCategory::from_word("doors and portals"),
            Some(BugCategory::DoorsAndPortals)
        );
        assert_eq!(
            BugCategory::from_word("spell"),
            Some(BugCategory::SpellStacking)
        );
        assert_eq!(BugCategory::from_word("npc"), Some(BugCategory::Npc));
        assert_eq!(BugCategory::from_word("guard"), None);
        assert_eq!(BugCategory::from_word(""), None);
        assert_eq!(BugCategory::ALL[0].name(), "Other");
    }
}
