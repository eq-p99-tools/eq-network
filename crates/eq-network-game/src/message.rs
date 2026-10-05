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
    /// A merchant's whole list, which replaces what it listed before, as
    /// `EQMac`'s comes on every change, its places numbered afresh. The
    /// feature keeping the merchant window tells the host the places that
    /// left and each item listed.
    MerchantList(Vec<crate::merchant::MerchantItem>),
    /// Coins the server added to the player's purse without saying what the
    /// purse holds, as `EQMac`'s money notices do one kind at a time; the
    /// feature keeping the purse adds them, and the host hears the purse.
    PurseAdded(crate::world::Coins),
    /// The destinations the zone numbered for its zone lines.
    ZonePoints(zoning::ZonePoints),
    /// The server's offer to move the player, to another zone or within this one.
    ZoneOffer(zoning::ZoneOffer),
    /// The server's answer to a transfer request, which the session reads
    /// against the pending request.
    ZoneAnswer(zoning::ZoneAnswer),
    /// The next zone's address.
    Handoff(Vec<u8>),
    /// The player's bind point, from their profile: where a client that asks
    /// for its own way home after death asks to go.
    Bind(zoning::BindPoint),
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
    /// The server's answer to a transfer request.
    ZoneAnswer,
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
            Self::ZoneAnswer => "Zone transfer answer",
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
        zoning::CHANGE_OPCODE => zoning::ZoneAnswer::titanium(body).map_or_else(
            |error| unreadable(Part::ZoneAnswer, &error),
            Message::ZoneAnswer,
        ),
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
/// [`crate::quarm::updates`] reads, the inventory, the server logging the
/// character out, its request that the client move, its answer to the
/// client's request, the zone's numbered destinations, and the player's
/// bind point from their profile.
#[must_use]
pub fn eqmac(opcode: u16, body: &[u8]) -> Vec<Message> {
    match opcode {
        crate::quarm::ZONE_LOGOUT | crate::quarm::ZONE_LOGOUT_REPLY => vec![Message::LoggedOut],
        crate::quarm::ZONE_CHANGE_REQUEST => vec![crate::quarm::zone_request(body).map_or_else(
            |error| unreadable(Part::ZoneOffer, &error),
            Message::ZoneOffer,
        )],
        crate::quarm::ZONE_CHANGE => vec![crate::quarm::zone_answer(body).map_or_else(
            |error| unreadable(Part::ZoneAnswer, &error),
            Message::ZoneAnswer,
        )],
        crate::quarm::ZONE_POINTS => vec![zoning::ZonePoints::decode_eqmac(body).map_or_else(
            |error| unreadable(Part::ZonePoints, &error),
            Message::ZonePoints,
        )],
        crate::quarm::ZONE_PLAYER_PROFILE => eqmac_profile(body),
        crate::money::EQMAC_PURSE_OPCODE => vec![crate::money::eqmac_purse_addition(body)
            .map_or_else(|error| unreadable(Part::World, &error), Message::PurseAdded)],
        crate::combat::EQMAC_CONSIDER_OPCODE => {
            vec![crate::combat::eqmac_consideration(body).map_or_else(
                |error| unreadable(Part::World, &error),
                |considered| Message::Event(WorldEvent::Consideration(considered)),
            )]
        }
        crate::items::EQMAC_LINK_OPCODE => vec![crate::items::eqmac_response(body).map_or_else(
            |error| unreadable(Part::World, &error),
            |details| Message::Event(WorldEvent::ItemDetails(details)),
        )],
        crate::merchant::EQMAC_STOCK_OPCODE => vec![crate::merchant::eqmac_list(body).map_or_else(
            |error| unreadable(Part::World, &error),
            Message::MerchantList,
        )],
        _ => match inventory::decode_eqmac(opcode, body) {
            Ok(Some(update)) => vec![Message::Event(WorldEvent::Inventory(update))],
            Err(error) => vec![unreadable(Part::Inventory, &error)],
            Ok(None) => crate::quarm::updates(opcode, body).map_or_else(
                |error| vec![unreadable(Part::World, &error)],
                |events| events.into_iter().map(Message::Event).collect(),
            ),
        },
    }
}

/// What the `EQMac` profile says beside what the admission reads of it: the
/// player's buffs and spellbook, and their coins.
fn eqmac_profile(body: &[u8]) -> Vec<Message> {
    let mut messages = events(Part::Spells, crate::quarm::profile_spells(body));
    messages.extend(events(Part::World, crate::quarm::profile_coins(body)));
    messages.push(
        crate::quarm::bind_point(body)
            .map_or_else(|error| unreadable(Part::World, &error), Message::Bind),
    );
    messages
}

/// The events a reading gives, or why it could not be read.
fn events(part: Part, read: anyhow::Result<Vec<WorldEvent>>) -> Vec<Message> {
    read.map_or_else(
        |error| vec![unreadable(part, &error)],
        |events| events.into_iter().map(Message::Event).collect(),
    )
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
    fn takps_merchant_list_is_one_message_for_the_session() {
        // A list that cannot be read says so; one that can is a
        // `MerchantList` (its records are read in `merchant::eqmac_list`).
        assert!(matches!(
            eqmac(crate::merchant::EQMAC_STOCK_OPCODE, &[0, 0])[..],
            [Message::Unreadable {
                part: Part::World,
                ..
            }]
        ));
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
        // An empty inventory, and an item packet that cannot be read, which
        // leaves the inventory untrustworthy.
        assert!(matches!(
            &eqmac(0xf641, &[0, 0])[..],
            [Message::Event(WorldEvent::Inventory(
                crate::inventory::InventoryUpdate::Snapshot(items)
            ))] if items.is_empty()
        ));
        assert!(matches!(
            eqmac(0x3140, &[0; 12])[..],
            [Message::Unreadable {
                part: Part::Inventory,
                ..
            }]
        ));
        // A linked item's answer describes it, and never fills a slot.
        let mut linked = [0; 360];
        linked[..14].copy_from_slice(b"Synthetic ring");
        linked[180..182].copy_from_slice(&42u16.to_le_bytes());
        assert!(matches!(
            &eqmac(crate::items::EQMAC_LINK_OPCODE, &linked)[..],
            [Message::Event(WorldEvent::ItemDetails(details))] if details.id == 42
        ));
        assert!(matches!(
            eqmac(crate::items::EQMAC_LINK_OPCODE, &linked[..300])[..],
            [Message::Unreadable {
                part: Part::World,
                ..
            }]
        ));
        assert!(eqmac(0xffff, &[]).is_empty());
    }

    #[test]
    fn takps_money_notice_adds_to_the_purse_and_names_no_one_else() {
        let mut notice = [0, 0, 2, 0, 0, 0, 0, 0];
        notice[4..].copy_from_slice(&5i32.to_le_bytes());
        assert!(matches!(
            eqmac(crate::money::EQMAC_PURSE_OPCODE, &notice)[..],
            [Message::PurseAdded(crate::world::Coins { gold: 5, .. })]
        ));
        notice[0] = 7;
        assert!(matches!(
            eqmac(crate::money::EQMAC_PURSE_OPCODE, &notice)[..],
            [Message::Unreadable {
                part: Part::World,
                ..
            }]
        ));
    }

    #[test]
    fn an_eqmac_cast_ending_is_a_spell_notice_and_a_change_in_mana() {
        assert!(matches!(
            eqmac(0x7f41, &[120, 0, 42, 0])[..],
            [
                Message::Event(WorldEvent::Spell(spells::SpellUpdate::Mana {
                    spell_id: 42,
                    keep_casting: false
                })),
                Message::Event(WorldEvent::Mana(120))
            ]
        ));
        // A profile that cannot be read leaves the spells, the coins and the
        // bind point unknown.
        assert!(matches!(
            eqmac(crate::quarm::ZONE_PLAYER_PROFILE, &[0; 7])[..],
            [
                Message::Unreadable {
                    part: Part::Spells,
                    ..
                },
                Message::Unreadable {
                    part: Part::World,
                    ..
                },
                Message::Unreadable {
                    part: Part::World,
                    ..
                }
            ]
        ));
    }
}
