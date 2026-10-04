//! What one zone packet says, read once for every part of the session that
//! listens: news the host hears about too, and the exchanges only the session
//! itself takes part in.
use crate::{inventory, spells, world::WorldEvent, zoning};
use std::fmt;

/// `OP_LogoutReply`: the server ends the session after a logout.
const LOGOUT_REPLY_OPCODE: u16 = 0x3cdc;
/// `OP_PlayerProfile`: the player's saved state, sent as the zone admits them.
const PROFILE_OPCODE: u16 = 0x75df;

/// What a zone packet says.
#[derive(Debug)]
pub enum Message {
    /// News the host hears about too.
    Event(WorldEvent),
    /// The destinations the zone numbered for its zone lines.
    ZonePoints(zoning::ZonePoints),
    /// The server's offer to move the player, to another zone or within this one.
    ZoneOffer(zoning::ZoneOffer),
    /// The server's answer to a transfer request. It reads against the pending
    /// request, so it stays as it arrived.
    ZoneAnswer(Vec<u8>),
    /// The next zone's address.
    Handoff(Vec<u8>),
    /// The server logging the character out.
    LoggedOut,
    /// A message a session feature withholds, to pass on later as it was or
    /// as what the next one shows it to be; no other feature hears it, and
    /// the host hears nothing of it.
    Withheld,
    /// A packet that could not be read.
    Unreadable {
        /// What the packet was about.
        part: Part,
        /// Why it could not be read.
        error: String,
    },
}

/// What an unreadable packet was about, which decides what happens without it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Part {
    /// A spell or spellbook notice.
    Spells,
    /// The inventory, which can no longer be trusted.
    Inventory,
    /// The zone's numbered destinations.
    ZonePoints,
    /// The server's offer to move the player.
    ZoneOffer,
    /// Anything else in the world.
    World,
}

impl fmt::Display for Part {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Spells => "Spell notification",
            Self::Inventory => "Inventory update",
            Self::ZonePoints => "Zone-point table",
            Self::ZoneOffer => "Zone transfer offer",
            Self::World => "World update",
        })
    }
}

/// What one Titanium zone packet says. A mana update is both a spell notice
/// and a change in resources, so one packet can say two things; the player's
/// profile says what is in their spellbook, which buffs they wear and the
/// coins they carry.
#[must_use]
pub fn titanium(opcode: u16, body: &[u8]) -> Vec<Message> {
    let mut messages = Vec::new();
    match spells::decode(opcode, body) {
        Ok(Some(update)) => messages.push(Message::Event(WorldEvent::Spell(update))),
        Ok(None) => (),
        Err(error) => messages.push(unreadable(Part::Spells, &error)),
    }
    let message = match opcode {
        zoning::POINTS_OPCODE => zoning::ZonePoints::decode(body).map_or_else(
            |error| unreadable(Part::ZonePoints, &error),
            Message::ZonePoints,
        ),
        zoning::TO_BIND_OPCODE | zoning::MOVE_OPCODE => zoning::offer(opcode, body).map_or_else(
            |error| unreadable(Part::ZoneOffer, &error),
            Message::ZoneOffer,
        ),
        PROFILE_OPCODE => {
            messages.push(crate::buffs::titanium_profile(body).map_or_else(
                |error| unreadable(Part::World, &error),
                |buffs| Message::Event(WorldEvent::BuffSnapshot(buffs)),
            ));
            messages.push(crate::world::titanium_coins(body).map_or_else(
                |error| unreadable(Part::World, &error),
                |coins| Message::Event(WorldEvent::Coins(coins)),
            ));
            messages.push(crate::food::titanium_profile(body).map_or_else(
                |error| unreadable(Part::World, &error),
                |nourishment| Message::Event(WorldEvent::Nourishment(nourishment)),
            ));
            messages.push(crate::money::titanium_elsewhere(body).map_or_else(
                |error| unreadable(Part::World, &error),
                |(cursor, bank)| {
                    Message::Event(WorldEvent::CoinsElsewhere {
                        cursor,
                        bank,
                        given: crate::world::Coins::default(),
                        offered: crate::world::Coins::default(),
                    })
                },
            ));
            spells::SpellBook::titanium_profile(body).map_or_else(
                |error| unreadable(Part::Spells, &error),
                |book| Message::Event(WorldEvent::SpellBook(book)),
            )
        }
        zoning::CHANGE_OPCODE => Message::ZoneAnswer(body.to_vec()),
        zoning::HANDOFF_OPCODE => Message::Handoff(body.to_vec()),
        LOGOUT_REPLY_OPCODE => Message::LoggedOut,
        _ => match inventory::decode(opcode, body) {
            Ok(Some(update)) => Message::Event(WorldEvent::Inventory(update)),
            Err(error) => unreadable(Part::Inventory, &error),
            Ok(None) => match crate::world::titanium_update(opcode, body) {
                Ok(Some(event)) => Message::Event(event),
                Ok(None) => return messages,
                Err(error) => unreadable(Part::World, &error),
            },
        },
    };
    messages.push(message);
    messages
}

/// What one `EQMac` zone packet says: the spawns and the player's news that
/// [`crate::quarm::updates`] reads, the server logging the character out,
/// and its request that the client move.
#[must_use]
pub fn eqmac(opcode: u16, body: &[u8]) -> Vec<Message> {
    match opcode {
        crate::quarm::ZONE_LOGOUT | crate::quarm::ZONE_LOGOUT_REPLY => vec![Message::LoggedOut],
        crate::quarm::ZONE_CHANGE_REQUEST => vec![crate::quarm::zone_request(body).map_or_else(
            |error| unreadable(Part::ZoneOffer, &error),
            Message::ZoneOffer,
        )],
        // The profile lists the player's group as each zone admits them.
        crate::quarm::ZONE_PLAYER_PROFILE => crate::quarm::profile_group(body).map_or_else(
            |error| vec![unreadable(Part::World, &error)],
            |group| {
                group
                    .map(|update| Message::Event(WorldEvent::Group(update)))
                    .into_iter()
                    .collect()
            },
        ),
        _ => crate::quarm::updates(opcode, body).map_or_else(
            |error| vec![unreadable(Part::World, &error)],
            |events| events.into_iter().map(Message::Event).collect(),
        ),
    }
}

fn unreadable(part: Part, error: &anyhow::Error) -> Message {
    Message::Unreadable {
        part,
        error: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mana_update_is_a_spell_notice_and_a_resource_change() {
        let mut body = vec![0; 16];
        body[..4].copy_from_slice(&120u32.to_le_bytes());
        body[4..8].copy_from_slice(&80u32.to_le_bytes());
        let messages = titanium(0x4839, &body);
        assert!(matches!(
            messages[..],
            [
                Message::Event(WorldEvent::Spell(_)),
                Message::Event(WorldEvent::Resources {
                    mana: 120,
                    endurance: 80
                })
            ]
        ));
    }

    #[test]
    fn the_profile_says_what_is_in_the_spellbook() {
        let mut profile = vec![0; 19592];
        profile[2312..2316].copy_from_slice(&73u32.to_le_bytes());
        assert!(matches!(
            &titanium(PROFILE_OPCODE, &profile)[..],
            [
                Message::Event(WorldEvent::BuffSnapshot(_)),
                Message::Event(WorldEvent::Coins(_)),
                Message::Event(WorldEvent::Nourishment(_)),
                Message::Event(WorldEvent::CoinsElsewhere { .. }),
                Message::Event(WorldEvent::SpellBook(book)),
            ] if book.slots()[0] == Some(73)
        ));
        assert!(matches!(
            titanium(PROFILE_OPCODE, &[0; 8])[..],
            [
                Message::Unreadable {
                    part: Part::World,
                    ..
                },
                Message::Unreadable {
                    part: Part::World,
                    ..
                },
                Message::Unreadable {
                    part: Part::World,
                    ..
                },
                Message::Unreadable {
                    part: Part::World,
                    ..
                },
                Message::Unreadable {
                    part: Part::Spells,
                    ..
                }
            ]
        ));
    }

    #[test]
    fn session_exchanges_and_unreadable_packets_say_what_they_are() {
        assert!(matches!(
            titanium(LOGOUT_REPLY_OPCODE, &[])[..],
            [Message::LoggedOut]
        ));
        assert!(matches!(
            &titanium(zoning::HANDOFF_OPCODE, &[1, 2])[..],
            [Message::Handoff(address)] if address == &[1, 2]
        ));
        assert!(matches!(
            titanium(zoning::MOVE_OPCODE, &[0; 3])[..],
            [Message::Unreadable {
                part: Part::ZoneOffer,
                ..
            }]
        ));
        assert!(matches!(
            titanium(0x5394, &[1])[..],
            [Message::Unreadable {
                part: Part::Inventory,
                ..
            }]
        ));
        assert!(titanium(0xffff, &[]).is_empty());
    }

    #[test]
    fn eqmac_packets_say_the_same_things_in_their_own_layouts() {
        use crate::quarm::{ZONE_CHANGE_REQUEST, ZONE_LOGOUT, ZONE_LOGOUT_REPLY};
        assert!(matches!(eqmac(ZONE_LOGOUT, &[])[..], [Message::LoggedOut]));
        assert!(matches!(
            eqmac(ZONE_LOGOUT_REPLY, &[])[..],
            [Message::LoggedOut]
        ));
        // A despawn, then a health update that says two things.
        assert!(matches!(
            eqmac(0x2940, &[9, 0])[..],
            [Message::Event(WorldEvent::Despawn(9))]
        ));
        let mut health = [0; 12];
        health[..4].copy_from_slice(&7u32.to_le_bytes());
        health[4..8].copy_from_slice(&50i32.to_le_bytes());
        health[8..].copy_from_slice(&100i32.to_le_bytes());
        assert!(matches!(
            eqmac(0xb240, &health)[..],
            [
                Message::Event(WorldEvent::HitPoints { spawn_id: 7, .. }),
                Message::Event(WorldEvent::HealthPercent {
                    spawn_id: 7,
                    percent: 50
                })
            ]
        ));
        let mut request = [0; 24];
        request[..4].copy_from_slice(&2u32.to_le_bytes());
        assert!(matches!(
            &eqmac(ZONE_CHANGE_REQUEST, &request)[..],
            [Message::ZoneOffer(offer)] if offer.zone_id == 2 && offer.solicited
        ));
        assert!(matches!(
            eqmac(ZONE_CHANGE_REQUEST, &[0; 3])[..],
            [Message::Unreadable {
                part: Part::ZoneOffer,
                ..
            }]
        ));
        assert!(matches!(
            eqmac(0x2940, &[9])[..],
            [Message::Unreadable {
                part: Part::World,
                ..
            }]
        ));
        assert!(eqmac(0xffff, &[]).is_empty());
    }
}
