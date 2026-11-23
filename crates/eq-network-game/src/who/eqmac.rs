//! `/who all` and its answer in the `EQMac` program's packets, read from
//! each server's own source: Quarm (SecretsOTheP/EQMacEmu fc75fcf) and TAKP
//! (EQMacEmu/Server 44afcbf), whose layouts match (6.583).
//!
//! - The request is `Who_All_Struct` (`common/eq_packet_structs.h`), 140
//!   bytes of 16-bit numbers, which `Client::Handle_OP_WhoAllRequest` drops
//!   at any other length.
//! - The answer is what `ClientList::SendWhoAll` and `SendFriendsWho`
//!   (`world/clientlist.cpp`) write: a 58-byte `WhoAllReturnStruct`
//!   heading, then each player's record, of 16-bit numbers and NUL-ended
//!   text. The zone (`zone/worldserver.cpp`, `ServerOP_WhoAllReply`) passes
//!   it on whole, except an answer of the heading alone, for which it sends
//!   string [`NO_RESULTS`] instead.
use super::{WhoFilter, WhoList, WhoPlayer, MAX_TEXT};
use crate::command::EncodedCommand;
use anyhow::{ensure, Context, Result};

/// `OP_WhoAllRequest` (the Mac program's 0x40f4).
pub const REQUEST_OPCODE: u16 = 0xf440;
/// `OP_WhoAllResponse` (the Mac program's 0x200b).
pub const RESPONSE_OPCODE: u16 = 0x0b20;
/// `OP_FormattedMessage`, which carries the zone's [`NO_RESULTS`].
pub const FORMATTED_OPCODE: u16 = 0x3642;
/// "There are no players in `EverQuest` that match those who filters."
/// (`zone/string_ids.h` `WHOALL_NO_RESULTS`), sent in place of an answer.
pub const NO_RESULTS: u16 = 5029;
/// `sizeof(Who_All_Struct)`.
const REQUEST_LENGTH: usize = 140;
/// `sizeof(WhoAllReturnStruct)`.
const HEADING_LENGTH: usize = 58;
/// A 16-bit field that asks about anyone.
const ANY: i16 = -1;

/// Asks the world who is online (`Who_All_Struct`): `whom` at +0, then
/// race, class, lowest and highest level, the game-master lookup and the
/// guild at +64 to +74, each -1 for any. The guild stays -1: a guild
/// number or the LFG lookup (-3) isn't asked here.
///
/// # Errors
/// Rejects text longer than [`MAX_TEXT`] bytes or holding a NUL, and a
/// race number past 16 bits.
pub fn request(filter: &WhoFilter) -> Result<EncodedCommand> {
    let text = filter.text.as_bytes();
    ensure!(
        text.len() <= MAX_TEXT && !text.contains(&0),
        "who text must be at most {MAX_TEXT} bytes without NUL"
    );
    let mut body = vec![0; REQUEST_LENGTH];
    body[..text.len()].copy_from_slice(text);
    let race = match filter.race {
        Some(race) => i16::try_from(race).context("race number past 16 bits")?,
        None => ANY,
    };
    let (low, high) = filter
        .levels
        .map_or((ANY, ANY), |(low, high)| (i16::from(low), i16::from(high)));
    for (offset, value) in [
        (64, race),
        (66, filter.class.map_or(ANY, i16::from)),
        (68, low),
        (70, high),
        (72, if filter.game_masters { 1 } else { ANY }),
        (74, ANY),
    ] {
        body[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    Ok(EncodedCommand {
        opcode: REQUEST_OPCODE,
        body,
    })
}

/// Whether a formatted message is the zone's [`NO_RESULTS`], sent in place
/// of a who answer: its string number at +2.
#[must_use]
pub fn unmatched(opcode: u16, body: &[u8]) -> bool {
    opcode == FORMATTED_OPCODE
        && body.get(2..4).map(|id| u16::from_le_bytes([id[0], id[1]])) == Some(NO_RESULTS)
}

/// A 16-bit string number, widened as Titanium's reader gives it: 0xffff,
/// which the servers write for none, becomes `u32::MAX`.
fn wide(number: u16) -> u32 {
    if number == u16::MAX {
        u32::MAX
    } else {
        u32::from(number)
    }
}

/// A string number, unless the answer leaves it out.
fn string(number: u16) -> Option<u32> {
    (number != 0 && number != u16::MAX).then_some(u32::from(number))
}

/// Reads the answer's fields in order. Its errors name no field's value.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn short(&mut self) -> Result<u16> {
        let (bytes, rest) = self
            .0
            .split_first_chunk::<2>()
            .context("who answer ends inside a number")?;
        self.0 = rest;
        Ok(u16::from_le_bytes(*bytes))
    }

    fn word(&mut self) -> Result<u32> {
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

    /// One player's record: format, account string, name, rank, guild,
    /// status, an unknown word, the flag (12314 LFG), zone string, zone,
    /// class, level, race, account and the ending word (211).
    fn player(&mut self) -> Result<WhoPlayer> {
        let line = self.short()?;
        let pid = self.short()?;
        let name = self.text()?;
        let rank = string(self.short()?);
        let guild = self.text()?;
        let status = Some(self.short()?)
            .filter(|status| *status != u16::MAX)
            .map(u32::from);
        self.short()?;
        let tag = string(self.short()?);
        let zone_line = self.short()?;
        let zone = self.word()?;
        let class = u32::from(self.short()?);
        let level = u32::from(self.short()?);
        let race = u32::from(self.short()?);
        let account = self.text()?;
        self.short()?;
        Ok(WhoPlayer {
            line: wide(line),
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

/// Decodes `OP_WhoAllResponse`: the heading's string at +4 (5001, or 0xffff
/// for friends), the rule at +6, the closing string at +36, the count at
/// +48 and the number of players listed at +56. Worlds size the answer for
/// every player they counted and zero what they then left out, so the list
/// ends at the listed number or the first zeroed player.
///
/// # Errors
/// Rejects an answer too short for its heading, or one that ends inside a
/// player.
pub fn decode(body: &[u8]) -> Result<WhoList> {
    ensure!(body.len() >= HEADING_LENGTH, "who answer too short");
    let short = |offset: usize| u16::from_le_bytes([body[offset], body[offset + 1]]);
    let rule = &body[6..33];
    let end = rule
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(rule.len());
    let count = u32::from_le_bytes([body[48], body[49], body[50], body[51]]);
    let listed = short(56);
    let mut reader = Reader(&body[HEADING_LENGTH..]);
    let mut players = Vec::new();
    while players.len() < usize::from(listed) && reader.0.iter().any(|byte| *byte != 0) {
        players.push(reader.player()?);
    }
    Ok(WhoList {
        heading: wide(short(4)),
        rule: String::from_utf8_lossy(&rule[..end]).into_owned(),
        closing: wide(short(36)),
        count,
        players,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: u16 = u16::MAX;

    /// A heading as `SendWhoAll` writes it.
    fn answer(
        heading: u16,
        closing: u16,
        count: u16,
        players: &[&[u8]],
        padding: usize,
    ) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&77u32.to_le_bytes());
        body.extend_from_slice(&heading.to_le_bytes());
        body.extend_from_slice(b"---------------------------");
        body.push(0x0a);
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&closing.to_le_bytes());
        body.extend_from_slice(&[0; 10]);
        body.extend_from_slice(&u32::from(count).to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&count.to_le_bytes());
        assert_eq!(body.len(), HEADING_LENGTH);
        for player in players {
            body.extend_from_slice(player);
        }
        body.resize(body.len() + padding, 0);
        body
    }

    /// One player's record, as `SendWhoAll` writes it.
    struct Record<'a> {
        line: u16,
        pid: u16,
        name: &'a str,
        rank: u16,
        guild: &'a str,
        status: u16,
        flag: u16,
        zone: (u16, u32),
        numbers: [u16; 3],
        account: &'a str,
    }

    impl Record<'_> {
        fn bytes(&self) -> Vec<u8> {
            let mut body = Vec::new();
            for value in [self.line, self.pid] {
                body.extend_from_slice(&value.to_le_bytes());
            }
            body.extend_from_slice(self.name.as_bytes());
            body.push(0);
            body.extend_from_slice(&self.rank.to_le_bytes());
            body.extend_from_slice(self.guild.as_bytes());
            body.push(0);
            for value in [self.status, NONE, self.flag, self.zone.0] {
                body.extend_from_slice(&value.to_le_bytes());
            }
            body.extend_from_slice(&self.zone.1.to_le_bytes());
            for value in self.numbers {
                body.extend_from_slice(&value.to_le_bytes());
            }
            body.extend_from_slice(self.account.as_bytes());
            body.push(0);
            body.extend_from_slice(&211u16.to_le_bytes());
            body
        }
    }

    const OPEN: Record<'static> = Record {
        line: 5025,
        pid: NONE,
        name: "Tester",
        rank: NONE,
        guild: "<Guild>",
        status: NONE,
        flag: 12314,
        zone: (5006, 4),
        numbers: [12, 50, 5],
        account: "",
    };

    #[test]
    fn a_request_is_the_140_bytes_both_servers_read() {
        let anyone = request(&WhoFilter::default()).unwrap();
        assert_eq!(anyone.opcode, 0xf440);
        assert_eq!(anyone.body.len(), 140);
        assert!(anyone.body[..64].iter().all(|byte| *byte == 0));
        assert!(anyone.body[64..76].iter().all(|byte| *byte == 0xff));
        assert!(anyone.body[76..].iter().all(|byte| *byte == 0));
        let narrowed = request(&WhoFilter {
            text: "qeynos".into(),
            race: Some(2),
            class: Some(12),
            levels: Some((40, 50)),
            game_masters: true,
        })
        .unwrap();
        assert_eq!(&narrowed.body[..7], b"qeynos\0");
        let numbers: Vec<i16> = narrowed.body[64..76]
            .chunks(2)
            .map(|chunk| i16::from_le_bytes(chunk.try_into().unwrap()))
            .collect();
        assert_eq!(numbers, [2, 12, 40, 50, 1, -1]);
        assert!(request(&WhoFilter {
            text: "x".repeat(64),
            ..WhoFilter::default()
        })
        .is_err());
        assert!(request(&WhoFilter {
            race: Some(70_000),
            ..WhoFilter::default()
        })
        .is_err());
    }

    #[test]
    fn an_answer_lists_its_players_and_stops_at_the_padding() {
        let anonymous = Record {
            line: 5023,
            name: "Hidden",
            guild: "<Guild>",
            flag: NONE,
            zone: (NONE, 0),
            numbers: [0, 0, 0],
            ..OPEN
        };
        let body = answer(5001, 5036, 3, &[&OPEN.bytes(), &anonymous.bytes()], 31);
        let list = decode(&body).unwrap();
        assert_eq!(list.heading, 5001);
        assert_eq!(list.rule, "-".repeat(27));
        assert_eq!((list.closing, list.count), (5036, 3));
        assert_eq!(
            list.players,
            [
                WhoPlayer {
                    line: 5025,
                    name: "Tester".into(),
                    rank: None,
                    guild: "<Guild>".into(),
                    tag: Some(12314),
                    zone: Some((5006, 4)),
                    class: 12,
                    level: 50,
                    race: 5,
                    account: None,
                    status: None,
                },
                WhoPlayer {
                    line: 5023,
                    name: "Hidden".into(),
                    rank: None,
                    guild: "<Guild>".into(),
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
        // An AFK player's mark is the rank (12311).
        let away = Record {
            rank: 12311,
            ..OPEN
        };
        let list = decode(&answer(5001, 5028, 1, &[&away.bytes()], 0)).unwrap();
        assert_eq!(list.players[0].rank, Some(12311));
        // A game master sees status and the account.
        let master = Record {
            pid: 5003,
            status: 80,
            account: "acct",
            ..OPEN
        };
        let list = decode(&answer(5001, 5028, 1, &[&master.bytes()], 0)).unwrap();
        assert_eq!(list.players[0].account, Some((5003, "acct".into())));
        assert_eq!(list.players[0].status, Some(80));
        // Cut short: the closing line says so.
        let list = decode(&answer(5001, 5033, 1, &[&OPEN.bytes()], 0)).unwrap();
        assert_eq!(list.closing, 5033);
        // Ended inside a player, or short of a heading.
        let whole = answer(5001, 5036, 2, &[&OPEN.bytes(), &OPEN.bytes()], 0);
        assert!(decode(&whole[..whole.len() - 5]).is_err());
        assert!(decode(&whole[..57]).is_err());
    }

    #[test]
    fn a_friends_answer_has_no_heading_string() {
        // `SendFriendsWho` writes 0xffff for the heading, and 0xffff for the
        // rank, status and flag words.
        let friend = Record {
            rank: NONE,
            status: NONE,
            flag: NONE,
            ..OPEN
        };
        let list = decode(&answer(NONE, 5028, 1, &[&friend.bytes()], 0)).unwrap();
        assert_eq!(list.heading, u32::MAX);
        assert_eq!(list.players.len(), 1);
        assert_eq!(list.players[0].tag, None);
    }

    #[test]
    fn the_zones_no_results_string_is_told_apart() {
        // `Message_StringID`'s formatted message: an unknown word, then the
        // string number.
        let mut body = vec![0, 0];
        body.extend_from_slice(&5029u16.to_le_bytes());
        body.extend_from_slice(&[0; 8]);
        assert!(unmatched(FORMATTED_OPCODE, &body));
        body[2] = 0xa6;
        assert!(!unmatched(FORMATTED_OPCODE, &body));
        assert!(!unmatched(RESPONSE_OPCODE, &body));
        assert!(!unmatched(FORMATTED_OPCODE, &[0, 0]));
    }
}
