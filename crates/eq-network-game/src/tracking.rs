//! Tracking (skill 53) as the Titanium program does it (read R14 section 2,
//! R15a). Pressing Tracking opens or refreshes the track window, which asks
//! the server for its list (`OP_Track`, empty) and then sends a second empty
//! message (`OP_TrackUnknown`, which `EQEmu` ignores). The server's list
//! names spawns and their distances; the program keeps the server's order
//! and leaves out spawns it does not know. Picking an entry and pressing
//! Track follows it: `OP_TrackTarget` names it, and for six minutes the
//! program prints where it lies against the player's heading every two
//! seconds, sending nothing more. Stopping sends `OP_TrackTarget` naming no
//! one.
//!
//! Layout reference: `EQEmu`'s Titanium `Track_Struct` and
//! `TrackTarget_Struct` (`common/patches/titanium_structs.h`), and
//! `Client::Handle_OP_Track`, `Handle_OP_TrackTarget` and
//! `Handle_OP_TrackUnknown` (`zone/client_packet.cpp`).
use crate::{command::EncodedCommand, world::Position};
use serde::Serialize;
use std::time::Duration;

/// `OP_Track`: the list asked for, empty, and the server's answer.
pub const LIST_OPCODE: u16 = 0x5d11;
/// The empty message the program sends after every press (`EQEmu`'s
/// `OP_TrackUnknown`).
pub const PRESSED_OPCODE: u16 = 0x6177;
/// `OP_TrackTarget`: the spawn followed, or 0 to stop.
pub const TARGET_OPCODE: u16 = 0x7085;
/// The most entries the program's window takes.
pub const MOST_ENTRIES: usize = 1000;
/// An entry's length: the spawn id as a 16-bit word at +0, then the distance
/// as a float at +4.
const ENTRY: usize = 8;
/// How long the program follows a spawn: six minutes from Track.
pub const FOLLOWED_FOR: Duration = Duration::from_secs(360);
/// How often it prints where the followed spawn lies.
pub const REPORT_EVERY: Duration = Duration::from_millis(2000);

/// The program's lines (`eqstr_us.txt` string IDs), read R15a.
pub mod lines {
    /// Following began, naming the spawn.
    pub const FOLLOWING: u32 = 0x2f08;
    /// The followed spawn is gone.
    pub const GONE: u32 = 0x30d3;
    /// Six minutes passed.
    pub const EXPIRED: u32 = 0x3189;
    /// The five bearings, from straight ahead to straight behind; the middle
    /// three also name a side.
    pub const BEARINGS: [u32; 5] = [0x3184, 0x3185, 0x3186, 0x3187, 0x3188];
    /// The two side words the middle bearings name: right, then left.
    pub const SIDES: [u32; 2] = [0x3182, 0x3183];
}

/// One entry of the server's list.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Tracked {
    /// The spawn.
    pub spawn_id: u16,
    /// How far away it is, as the server measured it.
    pub distance: f32,
}

/// What the track window asks of the session.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum TrackAction {
    /// Ask for the list again: the window's refresh.
    Refresh,
    /// Follow this spawn from the list: the window's Track button.
    Follow(u16),
    /// Stop following: `/stoptracking` or the window's stop button.
    Stop,
}

/// What tracking tells the host.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum TrackingUpdate {
    /// The server's list, in its order, with spawns the session does not
    /// know left out.
    Listed(Vec<Tracked>),
    /// Following a spawn began; the line names it.
    Following {
        /// The spawn followed.
        spawn_id: u16,
        /// The program's line for it.
        string_id: u32,
        /// What the line names: the spawn.
        arguments: Vec<String>,
    },
    /// Where the followed spawn lies against the player's heading.
    Bearing {
        /// The program's line for it.
        string_id: u32,
        /// What the line names: the spawn, then the side, as a string ID in
        /// decimal, where the line names one.
        arguments: Vec<String>,
    },
    /// Following ended: gone or expired with the program's line, or stopped
    /// with none.
    Ended {
        /// The program's line for it.
        string_id: Option<u32>,
    },
}

/// Reads the server's list as the program does: as many 8-byte entries as
/// the body holds, up to [`MOST_ENTRIES`].
#[must_use]
pub fn titanium_list(body: &[u8]) -> Vec<Tracked> {
    body.as_chunks::<ENTRY>()
        .0
        .iter()
        .take(MOST_ENTRIES)
        .map(|entry| Tracked {
            spawn_id: u16::from_le_bytes([entry[0], entry[1]]),
            distance: f32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]),
        })
        .collect()
}

/// The server's list as news for the host.
#[must_use]
pub fn titanium_news(body: &[u8]) -> TrackingUpdate {
    TrackingUpdate::Listed(titanium_list(body))
}

/// Asks for the list.
#[must_use]
pub const fn titanium_refresh() -> EncodedCommand {
    EncodedCommand {
        opcode: LIST_OPCODE,
        body: Vec::new(),
    }
}

/// The empty message after every press.
#[must_use]
pub const fn titanium_pressed() -> EncodedCommand {
    EncodedCommand {
        opcode: PRESSED_OPCODE,
        body: Vec::new(),
    }
}

/// Follows a spawn, or with 0 stops (4 bytes, the id as a 32-bit word).
#[must_use]
pub fn titanium_follow(spawn_id: u16) -> EncodedCommand {
    EncodedCommand {
        opcode: TARGET_OPCODE,
        body: u32::from(spawn_id).to_le_bytes().to_vec(),
    }
}

/// The program's line for where `to` lies against `from`'s heading, naming
/// `name` (read R15a): the angle off the heading, wrapped to a half turn
/// of the program's 512-unit circle, picks one of five bearings at 32, 96,
/// 160 and 224, and the side the middle three name comes from the sign of
/// a cross product. The program's own bearing routine (0x46e42f) is unread;
/// this measures the bearing as the heading's own direction runs, x by its
/// sine and y by its cosine. Turning left raises the heading, so a spawn
/// the way the heading grows lies on the left and gets 0x3183, and one the
/// other way 0x3182. This was checked against 22 side lines of a local
/// run, which fixed the side (recording checklist 6.441; the program's sign
/// test itself is unread).
#[must_use]
pub fn titanium_bearing(from: Position, to: Position, name: &str) -> (u32, Vec<String>) {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let bearing = dx.atan2(dy).to_degrees() / 360.0 * 512.0;
    let off = (bearing - from.heading).rem_euclid(512.0);
    let angle = if off > 256.0 { 512.0 - off } else { off };
    let band = [32.0, 96.0, 160.0, 224.0]
        .iter()
        .take_while(|&&edge| angle >= edge)
        .count();
    let string_id = lines::BEARINGS[band];
    let mut arguments = vec![name.to_owned()];
    if (1..=3).contains(&band) {
        // Under half a turn on, the way the heading grows: the left.
        let side = lines::SIDES[usize::from(off < 256.0)];
        arguments.push(side.to_string());
    }
    (string_id, arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_read_in_eight_byte_entries_in_the_servers_order() {
        let mut body = Vec::new();
        for (id, distance) in [(42_u32, 12.5_f32), (7, 300.0)] {
            body.extend_from_slice(&id.to_le_bytes());
            body.extend_from_slice(&distance.to_le_bytes());
        }
        body.push(9);
        assert_eq!(
            titanium_list(&body),
            [
                Tracked {
                    spawn_id: 42,
                    distance: 12.5
                },
                Tracked {
                    spawn_id: 7,
                    distance: 300.0
                }
            ]
        );
        assert_eq!(titanium_list(&vec![0; 8 * 1001]).len(), 1000);
        assert_eq!(titanium_follow(42).body, [42, 0, 0, 0]);
        assert_eq!(titanium_follow(0).opcode, 0x7085);
        assert_eq!(titanium_refresh().opcode, 0x5d11);
        assert_eq!(titanium_pressed().opcode, 0x6177);
    }

    #[test]
    fn a_bearing_picks_one_of_five_lines_and_the_middle_ones_a_side() {
        let at = |x, y, heading| Position {
            x,
            y,
            z: 0.0,
            heading,
        };
        // Heading 0 faces +y.
        let me = at(0.0, 0.0, 0.0);
        assert_eq!(
            titanium_bearing(me, at(0.0, 10.0, 0.0), "a"),
            (0x3184, vec!["a".to_owned()])
        );
        assert_eq!(
            titanium_bearing(me, at(0.0, -10.0, 0.0), "a"),
            (0x3188, vec!["a".to_owned()])
        );
        // Heading 128 faces +x and turning left raises the heading, so +x
        // lies on the left of one facing +y.
        let (line, left) = titanium_bearing(me, at(10.0, 0.0, 0.0), "a");
        assert_eq!((line, left[1].as_str()), (0x3186, "12675"));
        let (line, right) = titanium_bearing(me, at(-10.0, 0.0, 0.0), "a");
        assert_eq!((line, right[1].as_str()), (0x3186, "12674"));
        // As a local run stood it: the spawn 49 degrees to the left, ahead.
        assert_eq!(
            titanium_bearing(at(523.0, -447.0, 341.6), at(466.0, -427.0, 0.0), "a"),
            (0x3185, vec!["a".to_owned(), "12675".to_owned()])
        );
        // And 81 degrees to the right.
        assert_eq!(
            titanium_bearing(at(523.0, -447.0, 15.1), at(466.0, -427.0, 0.0), "a"),
            (0x3186, vec!["a".to_owned(), "12674".to_owned()])
        );
        // Turned to face the spawn, it lies ahead.
        assert_eq!(
            titanium_bearing(at(0.0, 0.0, 128.0), at(10.0, 0.0, 0.0), "a").0,
            0x3184
        );
        // Between ahead and abeam.
        assert_eq!(titanium_bearing(me, at(10.0, 10.0, 0.0), "a").0, 0x3185);
        assert_eq!(titanium_bearing(me, at(10.0, -10.0, 0.0), "a").0, 0x3187);
    }
}
