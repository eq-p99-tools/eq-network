//! Reports to the server's staff: a petition, a bug report and, where the
//! server reads it, feedback.
//!
//! Each packet is laid out as the servers that read it take it:
//! - Petition: stock `EQEmu`'s and TAKP's `Client::Handle_OP_Petition`.
//!   The body is the text, ending in a NUL.
//! - Bug report: `Client::Handle_OP_Bug` and `Client::RegisterBug`, with
//!   `EQEmu`'s Titanium `BugReport_Struct` (8736 bytes) and TAKP's
//!   (3340 bytes). The servers take only these exact sizes.
//! - Feedback: TAKP's `Client::Handle_OP_Feedback` and `Feedback_Struct`
//!   (1131 bytes). Stock `EQEmu` has no handler for it.
//!
//! What the official client puts in the fields this session leaves empty
//! (the interface path, time played and system information) is unread,
//! as are its own forms and windows for these reports (recording checklist
//! 6.511a and c). The system information stays empty by choice.

use crate::{command::EncodedCommand, GameDialect};

use serde::Serialize;

/// Titanium's `OP_Petition`.
pub const TITANIUM_PETITION_OPCODE: u16 = 0x251f;
/// Titanium's `OP_Bug`.
pub const TITANIUM_BUG_OPCODE: u16 = 0x7ac2;
/// `EQMac`'s `OP_Petition` (TAKP's 0x400e, byte-swapped as this crate writes
/// `EQMac` opcodes).
pub const EQMAC_PETITION_OPCODE: u16 = 0x0e40;
/// `EQMac`'s `OP_Bug` (TAKP's 0x40b3).
pub const EQMAC_BUG_OPCODE: u16 = 0xb340;
/// `EQMac`'s `OP_Feedback` (TAKP's 0x413c).
pub const EQMAC_FEEDBACK_OPCODE: u16 = 0x3c41;

/// The longest petition, in bytes before its NUL. The servers take any
/// length; this bound is this client's own (inferred, 6.511a).
pub const PETITION_LONGEST: usize = 1023;
/// The longest feedback, in bytes before its NUL: `Feedback_Struct`'s
/// message field less its NUL.
pub const FEEDBACK_LONGEST: usize = 1023;

/// Titanium's `BugReport_Struct`.
const TITANIUM_BUG_LENGTH: usize = 8736;
/// `EQMac`'s `BugReport_Struct`.
const EQMAC_BUG_LENGTH: usize = 3340;
/// `EQMac`'s `Feedback_Struct`.
const FEEDBACK_LENGTH: usize = 1131;
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

/// Where the player stood when they reported, as the client knows it.
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

/// A bug report. How the official client sets the report's other flags
/// (repeatable, crash, character flags) is unread, so it carries none of
/// them (recording checklist 6.511c).
#[derive(Clone, Debug, PartialEq)]
pub struct BugReport {
    /// What it is about.
    pub category: BugCategory,
    /// What the player says.
    pub text: String,
    /// Where the player stood, when the client knows.
    pub place: Option<BugPlace>,
    /// The player's target, if any.
    pub target: Option<BugTarget>,
}

/// `Bug::InformationFlag::TargetInfo`: the report carries the target.
const TARGET_INFO: u32 = 4;

impl BugReport {
    /// The report's optional-information mask, as `RegisterBug` reads it:
    /// only whether it carries the target.
    fn mask(&self) -> u32 {
        if self.target.is_some() {
            TARGET_INFO
        } else {
            0
        }
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

/// The longest bug report text the generation's packet holds, in bytes
/// before its NUL; none for a generation not built yet.
#[must_use]
pub const fn bug_longest(dialect: GameDialect) -> Option<usize> {
    match dialect {
        GameDialect::Titanium => Some(2049),
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
            // ui_path at 160 stays empty (6.511c).
            body[288..292].copy_from_slice(&place.x.to_le_bytes());
            body[292..296].copy_from_slice(&place.y.to_le_bytes());
            body[296..300].copy_from_slice(&place.z.to_le_bytes());
            // A whole number of the game's units (inferred, 6.511c).
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a heading in 0 to 512"
            )]
            let heading = place.heading.rem_euclid(512.0) as u32;
            body[300..304].copy_from_slice(&heading.to_le_bytes());
            // time_played at 308 stays zero (6.511c).
            if let Some(target) = &report.target {
                body[320..324].copy_from_slice(&u32::from(target.spawn_id).to_le_bytes());
                put_name(&mut body, 468, &target.name);
            }
            body[532..536].copy_from_slice(&report.mask().to_le_bytes());
            put(&mut body, 2588, 2050, text)?;
            // system_info at 4638 stays empty by choice (6.511c).
            Ok(EncodedCommand {
                opcode: TITANIUM_BUG_OPCODE,
                body,
            })
        }
        GameDialect::EqMac => {
            let mut body = vec![0; EQMAC_BUG_LENGTH];
            put(&mut body, 0, NAME_FIELD, report.category.name())?;
            put_name(&mut body, 64, reporter);
            body[160..164].copy_from_slice(&place.x.to_le_bytes());
            body[164..168].copy_from_slice(&place.y.to_le_bytes());
            body[168..172].copy_from_slice(&place.z.to_le_bytes());
            body[172..176].copy_from_slice(&place.heading.to_le_bytes());
            if let Some(target) = &report.target {
                put_name(&mut body, 192, &target.name);
            }
            body[256..260].copy_from_slice(&report.mask().to_le_bytes());
            put(&mut body, 2312, 1024, text)?;
            Ok(EncodedCommand {
                opcode: EQMAC_BUG_OPCODE,
                body,
            })
        }
        // Not built for RoF2 yet (6.489).
        GameDialect::Rof2 => Err(ReportError::NotRead),
    }
}

/// Feedback from the named character, on a generation whose servers read it.
///
/// # Errors
/// Refuses an empty or over-long text, one holding a NUL, Titanium (no
/// server reads it) and a generation not built yet.
pub fn feedback(
    dialect: GameDialect,
    name: &str,
    text: &str,
) -> Result<EncodedCommand, ReportError> {
    match dialect {
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
        // No stock EQEmu handler reads it on Titanium, and RoF2 is not
        // built yet (6.489).
        GameDialect::Titanium | GameDialect::Rof2 => Err(ReportError::NotRead),
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
        }
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
        assert_eq!(
            feedback(GameDialect::Titanium, "Reporter", "Hi").unwrap_err(),
            ReportError::NotRead
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
    fn eqmac_bug_reports_fill_the_servers_struct() {
        let mut plain = report();
        plain.target = None;
        let packet = bug(GameDialect::EqMac, "Reporter", &plain).unwrap();
        assert_eq!(packet.opcode, 0xb340);
        let body = &packet.body;
        assert_eq!(body.len(), 3340);
        assert_eq!(text_at(body, 0), "Pathing");
        assert_eq!(text_at(body, 64), "Reporter");
        assert_eq!(&body[160..164], &10.5f32.to_le_bytes());
        assert_eq!(&body[172..176], &256.4f32.to_le_bytes());
        assert_eq!(text_at(body, 192), "");
        assert_eq!(&body[256..260], &0u32.to_le_bytes());
        assert_eq!(text_at(body, 2312), "The guard walks into the wall.");
    }

    #[test]
    fn bug_reports_refuse_what_their_field_cannot_hold() {
        for (dialect, longest) in [(GameDialect::Titanium, 2049), (GameDialect::EqMac, 1023)] {
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
    fn feedback_goes_only_where_a_server_reads_it() {
        let packet = feedback(GameDialect::EqMac, "Reporter", "Nice boats.").unwrap();
        assert_eq!(packet.opcode, 0x3c41);
        assert_eq!(packet.body.len(), 1131);
        assert_eq!(text_at(&packet.body, 0), "Reporter");
        assert_eq!(text_at(&packet.body, 107), "Nice boats.");
        assert!(feedback(GameDialect::EqMac, "Reporter", &"x".repeat(1024)).is_err());
        assert!(feedback(GameDialect::Titanium, "Reporter", "Nice boats.").is_err());
        assert!(feedback(GameDialect::Rof2, "Reporter", "Nice boats.").is_err());
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
