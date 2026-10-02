//! Who is online: `/who all` asks the world server (`OP_WhoAllRequest`),
//! which answers with the players it found (`OP_WhoAllResponse`). The answer
//! carries string numbers, names and numbers rather than text: the client
//! words each line from its own string table. A Titanium client lists the
//! players in its own zone itself, so a plain `/who` asks nothing.
//!
//! Layout reference: `EQEmu`'s Titanium `Who_All_Struct`
//! (`common/patches/titanium_structs.h`) for the request, and
//! `ClientList::SendWhoAll` (`world/clientlist.cpp`), which writes the
//! answer field by field.
use crate::command::EncodedCommand;
use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// `OP_WhoAllRequest`: who is online, and where.
pub const REQUEST_OPCODE: u16 = 0x5cdd;
/// `OP_WhoAllResponse`: the players the world found.
pub const RESPONSE_OPCODE: u16 = 0x757b;
/// The longest name, guild or zone start a request carries.
pub const MAX_TEXT: usize = 63;
/// The request's length (`Who_All_Struct`).
const REQUEST_LENGTH: usize = 152;
/// A request number that asks about anyone.
const ANY: u32 = 0xffff_ffff;
/// The answer's heading, before the players.
const HEADING_LENGTH: usize = 64;

/// Which players a `/who all` asks about; the default asks about everyone.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct WhoFilter {
    /// The start of a name, guild name or zone short name; empty for any.
    pub text: String,
    /// A race number.
    pub race: Option<u32>,
    /// A class number, from 1 for a warrior.
    pub class: Option<u8>,
    /// The lowest and highest level, both included.
    pub levels: Option<(u8, u8)>,
    /// Only game masters.
    pub game_masters: bool,
}

/// Asks the world who is online (`Who_All_Struct`).
///
/// # Errors
/// Rejects text longer than [`MAX_TEXT`] bytes or holding a NUL.
pub fn request(filter: &WhoFilter) -> Result<EncodedCommand> {
    let text = filter.text.as_bytes();
    ensure!(
        text.len() <= MAX_TEXT && !text.contains(&0),
        "who text must be at most {MAX_TEXT} bytes without NUL"
    );
    let mut body = vec![0; REQUEST_LENGTH];
    body[..text.len()].copy_from_slice(text);
    let (low, high) = filter
        .levels
        .map_or((ANY, ANY), |(low, high)| (u32::from(low), u32::from(high)));
    for (offset, value) in [
        (64, filter.race.unwrap_or(ANY)),
        (68, filter.class.map_or(ANY, u32::from)),
        (72, low),
        (76, high),
        // Anything but "any" asks for game masters only.
        (80, if filter.game_masters { 1 } else { ANY }),
        (84, ANY),
    ] {
        body[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    Ok(EncodedCommand {
        opcode: REQUEST_OPCODE,
        body,
    })
}

/// The world's answer to `/who all`, in string numbers the client words.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WhoList {
    /// The heading's string, such as 5001, which introduces the list of
    /// players across the world.
    pub heading: u32,
    /// The line under the heading, as the world wrote it.
    pub rule: String,
    /// The closing line's string, such as 5036, which gives the number of
    /// players found, or 5033 when the world cut the list short.
    pub closing: u32,
    /// The number the closing line gives.
    pub count: u32,
    /// The players the world listed.
    pub players: Vec<WhoPlayer>,
}

/// One player in a `/who all` answer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WhoPlayer {
    /// The line's string: 5025 shows level, class, race and zone; 5024 hides
    /// them for an anonymous player and 5023 for a roleplaying one, who shows
    /// their guild; 5022 shows a game master what an anonymous player hides.
    pub line: u32,
    /// The player's name.
    pub name: String,
    /// The string before the line, such as a game master's rank (5007 to
    /// 5021).
    pub rank: Option<u32>,
    /// The guild, in angle brackets as the world writes it; empty for none.
    pub guild: String,
    /// The string after the line, such as 12313, which marks a player whose
    /// connection dropped.
    pub tag: Option<u32>,
    /// The zone, when shown: the string that words it (5006, which names the
    /// zone) and the zone number.
    pub zone: Option<(u32, u32)>,
    /// Class number; zero when hidden.
    pub class: u32,
    /// Level; zero when hidden.
    pub level: u32,
    /// Race number; zero when hidden.
    pub race: u32,
    /// For a game master asking: the string that words the account (5003,
    /// `(USER %1: PID %2)`) and the account's name.
    pub account: Option<(u32, String)>,
    /// For a game master asking: the player's account status.
    pub status: Option<u32>,
}

/// A string number, unless the answer leaves it out.
fn string(number: u32) -> Option<u32> {
    (number != 0 && number != ANY).then_some(number)
}

/// Reads the answer's fields in order.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn number(&mut self) -> Result<u32> {
        let (bytes, rest) = self
            .0
            .split_first_chunk::<4>()
            .context("who answer ends inside a number")?;
        self.0 = rest;
        Ok(u32::from_le_bytes(*bytes))
    }

    fn text(&mut self) -> Result<String> {
        let end = self
            .0
            .iter()
            .position(|byte| *byte == 0)
            .context("who answer ends inside a name")?;
        let text = String::from_utf8_lossy(&self.0[..end]).into_owned();
        self.0 = &self.0[end + 1..];
        Ok(text)
    }

    fn player(&mut self) -> Result<WhoPlayer> {
        let line = self.number()?;
        let pid = self.number()?;
        let name = self.text()?;
        let rank = string(self.number()?);
        let guild = self.text()?;
        let status = Some(self.number()?).filter(|status| *status != ANY);
        let tag = string(self.number()?);
        let zone_line = self.number()?;
        let zone = self.number()?;
        let class = self.number()?;
        let level = self.number()?;
        let race = self.number()?;
        let account = self.text()?;
        self.number()?;
        Ok(WhoPlayer {
            line,
            name,
            rank,
            guild,
            tag,
            zone: string(zone_line).map(|words| (words, zone)),
            class,
            level,
            race,
            account: string(pid).map(|words| (words, account)),
            status,
        })
    }
}

/// Decodes `OP_WhoAllResponse`. Worlds size the answer for every player they
/// counted and zero what they then left out, so the list ends at the first
/// zeroed player.
///
/// # Errors
/// Rejects an answer too short for its heading, or one that ends inside a
/// player.
pub fn decode(body: &[u8]) -> Result<WhoList> {
    ensure!(body.len() >= HEADING_LENGTH, "who answer too short");
    let number = |offset: usize| {
        u32::from_le_bytes([
            body[offset],
            body[offset + 1],
            body[offset + 2],
            body[offset + 3],
        ])
    };
    let rule = &body[8..35];
    let end = rule
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(rule.len());
    let count = number(60);
    let mut reader = Reader(&body[HEADING_LENGTH..]);
    let mut players = Vec::new();
    while players.len() < usize::try_from(count).unwrap_or(usize::MAX)
        && reader.0.iter().any(|byte| *byte != 0)
    {
        players.push(reader.player()?);
    }
    Ok(WhoList {
        heading: number(4),
        rule: String::from_utf8_lossy(&rule[..end]).into_owned(),
        closing: number(40),
        count,
        players,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An answer as `EQEmu`'s world writes it: a heading, then each player.
    fn answer(count: u32, players: &[&[u8]], padding: usize) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&77u32.to_le_bytes());
        body.extend_from_slice(&5001u32.to_le_bytes());
        body.extend_from_slice(b"---------------------------\n");
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&5036u32.to_le_bytes());
        body.extend_from_slice(&[0; 8]);
        body.extend_from_slice(&count.to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&count.to_le_bytes());
        for player in players {
            body.extend_from_slice(player);
        }
        body.resize(body.len() + padding, 0);
        body
    }

    #[allow(clippy::too_many_arguments, reason = "one argument per field")]
    fn player(
        line: u32,
        name: &str,
        rank: u32,
        guild: &str,
        zone: (u32, u32),
        numbers: [u32; 3],
        pid: u32,
        account: &str,
    ) -> Vec<u8> {
        let mut body = Vec::new();
        for value in [line, pid] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        body.extend_from_slice(name.as_bytes());
        body.push(0);
        body.extend_from_slice(&rank.to_le_bytes());
        body.extend_from_slice(guild.as_bytes());
        body.push(0);
        // A game master asking sees each player's status.
        let status = if pid == ANY { ANY } else { 255 };
        for value in [
            status, ANY, zone.0, zone.1, numbers[0], numbers[1], numbers[2],
        ] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        body.extend_from_slice(account.as_bytes());
        body.push(0);
        body.extend_from_slice(&207u32.to_le_bytes());
        body
    }

    #[test]
    fn a_request_asks_about_anyone_unless_narrowed() {
        let anyone = request(&WhoFilter::default()).unwrap();
        assert_eq!(anyone.opcode, REQUEST_OPCODE);
        assert_eq!(anyone.body.len(), 152);
        assert!(anyone.body[..64].iter().all(|byte| *byte == 0));
        assert!(anyone.body[64..88].iter().all(|byte| *byte == 0xff));
        let narrowed = request(&WhoFilter {
            text: "qeynos".into(),
            race: Some(2),
            class: Some(12),
            levels: Some((40, 50)),
            game_masters: true,
        })
        .unwrap();
        assert_eq!(&narrowed.body[..7], b"qeynos\0");
        let numbers: Vec<u32> = narrowed.body[64..88]
            .chunks(4)
            .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()))
            .collect();
        assert_eq!(numbers, [2, 12, 40, 50, 1, ANY]);
        assert!(request(&WhoFilter {
            text: "x".repeat(64),
            ..WhoFilter::default()
        })
        .is_err());
    }

    #[test]
    fn an_answer_lists_its_players_and_stops_at_the_padding() {
        let open = player(
            5025,
            "Tester",
            ANY,
            "<Guild>",
            (5006, 4),
            [12, 50, 5],
            ANY,
            "",
        );
        let anonymous = player(5024, "Hidden", ANY, "", (ANY, 0), [0, 0, 0], ANY, "");
        let body = answer(3, &[&open, &anonymous], 49);
        let list = decode(&body).unwrap();
        assert_eq!(list.heading, 5001);
        assert_eq!(list.rule, "-".repeat(27));
        assert_eq!(list.closing, 5036);
        assert_eq!(list.count, 3);
        assert_eq!(
            list.players,
            [
                WhoPlayer {
                    line: 5025,
                    name: "Tester".into(),
                    rank: None,
                    guild: "<Guild>".into(),
                    tag: None,
                    zone: Some((5006, 4)),
                    class: 12,
                    level: 50,
                    race: 5,
                    account: None,
                    status: None,
                },
                WhoPlayer {
                    line: 5024,
                    name: "Hidden".into(),
                    rank: None,
                    guild: String::new(),
                    tag: None,
                    zone: None,
                    class: 0,
                    level: 0,
                    race: 0,
                    account: None,
                    status: None,
                },
            ]
        );
        // A game master sees ranks and accounts.
        let master = player(5025, "Guide", 5015, "", (5006, 1), [1, 65, 1], 5003, "acct");
        let list = decode(&answer(1, &[&master], 0)).unwrap();
        assert_eq!(list.players[0].rank, Some(5015));
        assert_eq!(list.players[0].account, Some((5003, "acct".into())));
        assert_eq!(list.players[0].status, Some(255));
        assert!(decode(&body[..63]).is_err());
        assert!(decode(&body[..body.len() - 60]).is_err());
    }
}
