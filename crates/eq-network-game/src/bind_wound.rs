//! Binding wounds, as the official client's Bind Wound ability does: the
//! player bandages themselves, or another player close by, using up one of
//! the bandages they carry. `EQEmu` takes the bandage as the bandaging
//! starts and judges it ten seconds later, healing a little if the one
//! bandaged is still close and below the share of their hit points a
//! bandage reaches (`Client::BindWound`, `zone/client.cpp`).
//!
//! Layout reference: `BindWound_Struct` (`common/patches/titanium_structs.h`):
//! the spawn bandaged and a type, 16 bits each, each followed by 16 unused
//! bits. The client asks with type 0; the server answers with the same
//! opcode, its type saying what happened.
use crate::command::EncodedCommand;
use anyhow::{bail, ensure, Result};
use serde::Serialize;
use std::time::Duration;

/// `OP_Bind_Wound`, both ways.
pub const OPCODE: u16 = 0x601d;

/// How long a bandaging takes: `EQEmu`'s `bindwound_timer`.
pub const DURATION: Duration = Duration::from_secs(10);

/// How far apart, in units, the bandager and the one bandaged may be when
/// the bandaging ends: `EQEmu` judges it a failure beyond that, after the
/// bandage is gone, so the session asks only within it.
pub const REACH: f32 = 20.0;

/// The item type of a bandage (`EQ::item::ItemTypeBandage`).
pub const BANDAGE: u8 = 18;

/// The official client's strings (`eqstr_us.txt`) for Bind Wound, which a
/// host with the installed strings shows in place of this library's words.
pub mod strings {
    /// Refusing a target that is not a player.
    pub const NOT_A_PLAYER: u32 = 144;
    /// Refusing without a bandage.
    pub const NO_BANDAGES: u32 = 146;
    /// Refusing a player too far away, who is its one argument.
    pub const TOO_FAR: u32 = 420;
    /// Starting on the player themselves.
    pub const STARTED_ON_SELF: u32 = 12436;
    /// Starting on another player, who is its one argument.
    pub const STARTED_ON_OTHER: u32 = 12437;
}

/// Where a bandaging stands.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum BindWoundUpdate {
    /// The session asked the server to bandage someone: another player,
    /// by name, or the player themselves.
    Started {
        /// Whom, if not the player.
        target: Option<String>,
    },
    /// The server lets the player act again: as a bandaging starts, and
    /// after one failed.
    Unlocked,
    /// The bandaging ended, and how. What it healed arrives as hit points.
    Ended(BindWoundEnd),
}

/// How a bandaging ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum BindWoundEnd {
    /// It is done.
    Complete,
    /// The one bandaged died.
    Died,
    /// The one bandaged left the zone.
    Left,
    /// The one bandaged moved too far away.
    TheyMoved,
    /// The player moved: sitting or standing ends a bandaging. `EQEmu` also
    /// answers this way when the player has no bandage.
    YouMoved,
}

impl BindWoundEnd {
    /// The official client's string (`eqstr_us.txt`) for this ending.
    #[must_use]
    pub const fn string_id(self) -> u32 {
        match self {
            Self::Complete => 1432,
            Self::Died => 1433,
            Self::Left => 1434,
            Self::TheyMoved => 1435,
            Self::YouMoved => 1436,
        }
    }

    /// This ending in this library's words.
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Self::Complete => "The bandage is on.",
            Self::Died => "The one you were bandaging died.",
            Self::Left => "The one you were bandaging is gone.",
            Self::TheyMoved => "The one you were bandaging moved too far away.",
            Self::YouMoved => "You moved, so the bandaging failed.",
        }
    }
}

/// The request to bandage the spawn with this ID: the player's own to
/// bandage themselves.
#[must_use]
pub fn encode(target: u16) -> EncodedCommand {
    let mut body = target.to_le_bytes().to_vec();
    body.resize(8, 0);
    EncodedCommand {
        opcode: OPCODE,
        body,
    }
}

/// The server's answer about a bandaging, if this is one.
///
/// # Errors
/// Rejects a malformed answer or one of a type no server is known to send.
pub fn decode(opcode: u16, body: &[u8]) -> Result<Option<BindWoundUpdate>> {
    if opcode != OPCODE {
        return Ok(None);
    }
    ensure!(body.len() == 8, "invalid bind wound length");
    let ended = BindWoundUpdate::Ended;
    Ok(Some(match u16::from_le_bytes([body[4], body[5]]) {
        1 => ended(BindWoundEnd::Complete),
        3 => BindWoundUpdate::Unlocked,
        4 => ended(BindWoundEnd::Died),
        5 => ended(BindWoundEnd::Left),
        6 => ended(BindWoundEnd::TheyMoved),
        7 => ended(BindWoundEnd::YouMoved),
        other => bail!("unknown bind wound answer {other}"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_names_the_one_bandaged_and_answers_say_how_it_went() {
        assert_eq!(
            encode(0x1234),
            EncodedCommand {
                opcode: OPCODE,
                body: vec![0x34, 0x12, 0, 0, 0, 0, 0, 0],
            }
        );
        let answer = |kind: u8| decode(OPCODE, &[0, 0, 0, 0, kind, 0, 0, 0]);
        assert_eq!(answer(3).unwrap(), Some(BindWoundUpdate::Unlocked));
        assert_eq!(
            answer(1).unwrap(),
            Some(BindWoundUpdate::Ended(BindWoundEnd::Complete))
        );
        assert_eq!(
            answer(7).unwrap(),
            Some(BindWoundUpdate::Ended(BindWoundEnd::YouMoved))
        );
        assert!(answer(2).is_err());
        assert!(decode(OPCODE, &[0; 6]).is_err());
        assert_eq!(decode(0x1234, &[0; 8]).unwrap(), None);
    }

    #[test]
    fn the_endings_take_the_official_strings_in_order() {
        let endings = [
            BindWoundEnd::Complete,
            BindWoundEnd::Died,
            BindWoundEnd::Left,
            BindWoundEnd::TheyMoved,
            BindWoundEnd::YouMoved,
        ];
        for (index, ending) in endings.into_iter().enumerate() {
            assert_eq!(ending.string_id(), 1432 + u32::try_from(index).unwrap());
        }
        // Each says something of its own in this library's words too.
        let texts: std::collections::BTreeSet<_> = endings.map(BindWoundEnd::text).into();
        assert_eq!(texts.len(), endings.len());
    }
}
