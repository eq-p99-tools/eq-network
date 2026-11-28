//! What one zone packet says, read once for every part of the session that
//! listens: news the host hears about too, and the exchanges only the session
//! itself takes part in.
use crate::{inventory, spells, world::WorldEvent, zoning};
use std::fmt;

#[cfg(test)]
mod eqmac_level_tests;

/// `OP_LogoutReply`: the server ends the session after a logout. Sent by
/// `EQEmu` (`zone/client_process.cpp`, `zone/client_packet.cpp`
/// `Handle_OP_DeleteSpawn` and `Handle_OP_Logout`, each followed by a
/// disconnect but the zoning one); the official Titanium program doesn't
/// read it (Titanium sitting 6; Settle results section 7).
const LOGOUT_REPLY_OPCODE: u16 = 0x3cdc;
/// `OP_PlayerProfile`: the player's saved state, sent as the zone admits them.
const PROFILE_OPCODE: u16 = 0x75df;

/// What a zone packet says.
#[derive(Debug)]
pub enum Message {
    /// The `EQMac` zone's string 5029 in place of a who answer: no player
    /// matched ([`crate::who::eqmac::NO_RESULTS`]). The line itself still
    /// reaches the host as chat; this tells the session the query was
    /// answered.
    WhoUnmatched,
    /// The Titanium program's own roll on a hit interrupts what the player
    /// casts or sings (0x41c905's interrupt branch, read R49-50): its local
    /// clear, mode 1 of 0x40f8a0, which sends nothing. The session makes it
    /// for its casting feature to clear; the host hears what that changes.
    HitInterrupt,
    /// TAKP or Quarm refused the player's split, with string 12328 or 13112
    /// ([`crate::split::eqmac_refused`]); the additions that follow give the
    /// coins back. The line itself still reaches the host as chat.
    SplitRefused,
    /// Read-only Titanium AA facts, scoped and published by the server feature.
    AlternateAdvancement(crate::alternate_advancement::Update),
    /// News the host hears about too.
    Event(WorldEvent),
    /// A decoded trainer reply that still needs session checks.
    TrainingReply(crate::training::TrainingReply),
    /// A correlated P99 assist selection for the session's target owner.
    AssistSelection(u16),
    /// A skill's new value for a spawn (`EQMac`'s silent update), which is
    /// the player's own only when the spawn is theirs.
    SpawnSkill(crate::training::SpawnSkill),
    /// A decoded resurrection that still needs admission and consent checks.
    Resurrection(crate::resurrection::ResurrectionOffer),
    /// TAKP's completion of an accepted resurrection: its kept offer with
    /// the answer 1, which moves the player only for the offer this session
    /// accepted (6.596).
    ResurrectionCompleted(crate::resurrection::ResurrectionOffer),
    /// A decoded translocate that still needs admission and consent checks.
    TranslocateOffer(crate::confirmations::TranslocateOffer),
    /// A received Titanium completion that still needs exact consent correlation.
    TranslocateCompleted(crate::confirmations::TranslocateOffer),
    /// A merchant's whole list, which replaces what it listed before, as
    /// `EQMac`'s comes on every change, its places numbered afresh. The
    /// feature keeping the merchant window tells the host the places that
    /// left and each item listed.
    MerchantList(Vec<crate::merchant::MerchantItem>),
    /// A Titanium merchant item packet (`ItemPacketMerchant`), with the
    /// serial number its header carries: a trader's item while the player
    /// browses them, a merchant's otherwise. The feature keeping the browse
    /// takes it as the trader's; else the merchant's feature takes it as
    /// `WorldEvent::Merchant`, or as unreadable.
    MerchantView {
        /// The serial number, where the header carries one
        /// ([`crate::merchant::view_serial`]).
        serial: Option<u32>,
        /// The packet's length, for a gap.
        length: usize,
        /// The item as the merchant's list reads it, or why it could not
        /// be read.
        item: Result<Box<crate::merchant::MerchantItem>, String>,
    },
    /// Titanium merchant update whose wire entity must be checked against
    /// the open catalog or an outstanding transaction before publication.
    MerchantScoped {
        /// Merchant entity from the packet.
        merchant_id: u32,
        /// The stock removal or transaction result.
        update: crate::merchant::MerchantUpdate,
    },
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
    /// The vehicle the received Titanium profile names, or None.
    VehicleName(Option<String>),
    /// The server's word that a boat carries its riders into another zone
    /// (Titanium 0x0b71), for the session to match against the saved vehicle.
    VehicleTransfer(crate::boats::riders::Transfer),
    /// The next zone's address.
    Handoff(Vec<u8>),
    /// The player's bind point, from their profile: where a client that asks
    /// for its own way home after death asks to go.
    Bind(zoning::BindPoint),
    /// Ability-slot reuse received in a Titanium knight profile.
    KnightRefresh(std::time::Duration),
    /// A Titanium task journal message, which the session's journal
    /// checks against the entries it holds before the host hears of it.
    Journal {
        /// What it says.
        news: crate::tasks::News,
        /// Its body's length, for a gap that tells of it without its words.
        length: usize,
    },
    /// The server logging the character out.
    LoggedOut,
    /// A Titanium system notice identifying an invalid tradeskill recipe.
    TradeskillNoRecipe,
    /// `EQMac`'s empty `OP_ClickObjectAction`: TAKP and Quarm answer every
    /// close with it, after which the player uses no world container. What
    /// the container held stays in it (`Object::Close` saves it), so its
    /// places leave the player's view (6.598).
    ContainerReleased,
    /// The server refusing a scribe or memorization because the player has
    /// not reached the spell's level: `EQEmu`'s `SPELL_LEVEL_TO_LOW` (string
    /// 12048), the only answer its `OPMemorizeSpell` gives before returning.
    /// The spellbook feature ends the request it answers; the line itself
    /// still reaches the host as chat.
    BookLevelRefused {
        /// The line's arguments as the server sent them: the level, then
        /// the spell's name.
        arguments: Vec<String>,
    },
    /// A message a session feature withholds, to pass on later as it was or
    /// as what the next one shows it to be; no other feature hears it, and
    /// the host hears nothing of it.
    Withheld,
    /// A message of a kind no server is known to send: the kind, which
    /// nothing reads. The zone session tells of each kind of a message once
    /// and drops it.
    UnknownKind(u32),
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

/// The numbers [`rof2`] reads, for the session's coverage table.
pub const ROF2_READS: [u16; 67] = [
    crate::rof2_player::SPAWN_OPCODE,
    crate::rof2_player::POSITION_OPCODE,
    crate::rof2_player::DESPAWN_OPCODE,
    crate::rof2_looks::APPEARANCE_OPCODE,
    crate::rof2_looks::SIZE_OPCODE,
    crate::rof2_looks::HIT_POINTS_OPCODE,
    crate::rof2_looks::HEALTH_OPCODE,
    crate::rof2_looks::WEAR_CHANGE_OPCODE,
    crate::rof2_looks::ANIMATION_OPCODE,
    crate::rof2_looks::STATES_ADDED_OPCODE,
    crate::rof2_looks::STATES_REMOVED_OPCODE,
    crate::rof2_news::DEATH_OPCODE,
    crate::rof2_news::LEVEL_OPCODE,
    crate::rof2_news::EXPERIENCE_OPCODE,
    crate::rof2_news::STAMINA_OPCODE,
    crate::rof2_news::MONEY_OPCODE,
    crate::rof2_news::TIME_OPCODE,
    crate::rof2_news::MANA_OPCODE,
    crate::rof2_news::DAMAGE_OPCODE,
    crate::rof2_news::CONSIDER_OPCODE,
    crate::rof2_news::SKILL_OPCODE,
    crate::rof2_casting::BEGIN_CAST_OPCODE,
    crate::rof2_casting::ACTION_OPCODE,
    crate::rof2_casting::INTERRUPT_OPCODE,
    crate::rof2_casting::STUN_OPCODE,
    crate::rof2_casting::ILLUSION_OPCODE,
    crate::rof2_casting::FACE_OPCODE,
    crate::rof2_buffs::UPDATE_OPCODE,
    crate::rof2_buffs::ICONS_OPCODE,
    crate::rof2_buffs::TARGET_ICONS_OPCODE,
    crate::rof2_buffs::PET_OPCODE,
    crate::rof2_spellbook::MEMORIZE_OPCODE,
    crate::rof2_spellbook::SWAP_OPCODE,
    crate::rof2_spellbook::DELETE_OPCODE,
    crate::rof2_pools::MANA_OPCODE,
    crate::rof2_pools::ENDURANCE_OPCODE,
    crate::rof2_pools::MANA_PERCENT_OPCODE,
    crate::rof2_pools::ENDURANCE_PERCENT_OPCODE,
    crate::rof2_world::SPAWN_DOOR_OPCODE,
    crate::rof2_world::MOVE_DOOR_OPCODE,
    crate::rof2_world::GROUND_SPAWN_OPCODE,
    crate::rof2_world::CLICK_OBJECT_OPCODE,
    crate::rof2_world::WEATHER_OPCODE,
    crate::rof2_socials::ROLL_OPCODE,
    crate::rof2_socials::ASSIST_OPCODE,
    crate::rof2_socials::YELL_OPCODE,
    crate::rof2_socials::LEVEL_APPEARANCE_OPCODE,
    crate::rof2_socials::CAMERA_EFFECT_OPCODE,
    crate::rof2_socials::PLAY_MP3_OPCODE,
    crate::rof2_socials::SPELL_EFFECT_OPCODE,
    crate::rof2_answers::TARGET_OPCODE,
    crate::rof2_answers::TARGET_OF_TARGET_OPCODE,
    crate::rof2_answers::CHARM_OPCODE,
    crate::rof2_answers::CASH_REWARD_OPCODE,
    crate::rof2_answers::LINKED_REUSE_OPCODE,
    crate::rof2_answers::BEGGING_OPCODE,
    crate::rof2_answers::PICK_POCKET_OPCODE,
    crate::rof2_answers::BIND_WOUND_OPCODE,
    crate::rof2_notices::SOUND_OPCODE,
    crate::rof2_notices::RENAME_OPCODE,
    crate::rof2_notices::NEW_TITLES_OPCODE,
    crate::rof2_notices::BANKER_CHANGE_OPCODE,
    crate::rof2_loot_titles::MONEY_OPCODE,
    crate::rof2_loot_titles::LOOT_COMPLETE_OPCODE,
    crate::rof2_loot_titles::SURNAME_OPCODE,
    crate::rof2_loot_titles::TITLE_LIST_OPCODE,
    crate::rof2_loot_titles::TITLE_OPCODE,
];

/// What one `RoF2` zone packet says. Only its spawn records (6.510), its
/// spawns' movement and removal (6.514), how they look and how hurt they
/// are (6.518), the player's own news and the combat records (6.519), and
/// casting, stuns and illusions (6.522), buffs (6.527), the spell book and
/// gems (6.532), the mana and endurance pools (6.538), doors, ground objects
/// and the weather (6.539), rolls, assists, yells and effects (6.541), and
/// targets, charms, rewards, reuse and skill answers (6.544), and quest
/// rewards, renames, new titles and the banker's change (6.549), and corpse
/// money and the loot close, the surname answer and titles (6.552) are read
/// yet; a body a reader refuses is unreadable, told by its opcode and length
/// only.
#[must_use]
pub fn rof2(opcode: u16, body: &[u8]) -> Vec<Message> {
    match opcode {
        crate::rof2_player::SPAWN_OPCODE => vec![crate::rof2_player::spawn(body).map_or_else(
            |error| unreadable(Part::World, &error),
            |spawn| Message::Event(WorldEvent::Spawns(vec![spawn])),
        )],
        crate::rof2_player::POSITION_OPCODE => vec![crate::rof2_player::position(body)
            .map_or_else(|error| unreadable(Part::World, &error), Message::Event)],
        crate::rof2_player::DESPAWN_OPCODE => vec![crate::rof2_player::despawn(body)
            .map_or_else(|error| unreadable(Part::World, &error), Message::Event)],
        _ => rof2_readers(opcode, body).unwrap_or_default(),
    }
}

/// What `RoF2`'s readers past the spawn records say of a packet, each with
/// Titanium's part for its message; None when no reader takes its opcode.
fn rof2_readers(opcode: u16, body: &[u8]) -> Option<Vec<Message>> {
    let events = |read: anyhow::Result<Option<Vec<WorldEvent>>>, part| match read {
        Ok(Some(events)) => Some(events.into_iter().map(Message::Event).collect()),
        Ok(None) => None,
        Err(error) => Some(vec![unreadable(part, &error)]),
    };
    let one = |read: anyhow::Result<Option<WorldEvent>>, part| {
        events(read.map(|event| event.map(|event| vec![event])), part)
    };
    let spell = |read: anyhow::Result<Option<crate::spells::SpellUpdate>>| {
        one(
            read.map(|update| update.map(WorldEvent::Spell)),
            Part::Spells,
        )
    };
    events(crate::rof2_news::read(opcode, body), Part::World)
        .or_else(|| one(crate::rof2_looks::read(opcode, body), Part::World))
        .or_else(|| one(crate::rof2_buffs::read(opcode, body), Part::World))
        .or_else(|| one(crate::rof2_pools::read(opcode, body), Part::World))
        .or_else(|| one(crate::rof2_world::read(opcode, body), Part::World))
        .or_else(|| one(crate::rof2_socials::read(opcode, body), Part::World))
        .or_else(|| one(crate::rof2_answers::read(opcode, body), Part::World))
        .or_else(|| one(crate::rof2_notices::read(opcode, body), Part::World))
        .or_else(|| one(crate::rof2_loot_titles::read(opcode, body), Part::World))
        .or_else(|| spell(crate::rof2_spellbook::read(opcode, body)))
        .or_else(|| rof2_casting(opcode, body).map(|message| vec![message]))
}

/// What one of `RoF2`'s casting messages says, with Titanium's parts: a
/// spell's beginning or interruption is the spells part's, the rest the
/// world's. None for another opcode.
fn rof2_casting(opcode: u16, body: &[u8]) -> Option<Message> {
    let part = match opcode {
        crate::rof2_casting::BEGIN_CAST_OPCODE | crate::rof2_casting::INTERRUPT_OPCODE => {
            Part::Spells
        }
        _ => Part::World,
    };
    match crate::rof2_casting::read(opcode, body) {
        Ok(None) => None,
        Ok(Some(crate::rof2_casting::Casting::Event(event))) => Some(Message::Event(event)),
        Ok(Some(crate::rof2_casting::Casting::Action(read))) => Some(action(Ok(read))),
        Err(error) => Some(unreadable(part, &error)),
    }
}

/// What one Titanium zone packet says. A mana update is both a spell notice
/// and a change in resources, so one packet can say two things; the player's
/// profile says what is in their spellbook, which buffs they wear and the
/// coins they carry, and the zone's header its sky and the rest of its
/// description.
#[must_use]
pub fn titanium(opcode: u16, body: &[u8]) -> Vec<Message> {
    let mut messages = Vec::new();
    match spells::decode(opcode, body) {
        Ok(Some(update)) => messages.push(Message::Event(WorldEvent::Spell(update))),
        Ok(None) => (),
        Err(error) => messages.push(unreadable(Part::Spells, &error)),
    }
    let message = match opcode {
        opcode if crate::alternate_advancement::is_titanium(opcode) => {
            titanium_aa_message(opcode, body, &mut messages)
        }
        zoning::POINTS_OPCODE => zoning::ZonePoints::decode(body).map_or_else(
            |error| unreadable(Part::ZonePoints, &error),
            Message::ZonePoints,
        ),
        zoning::TO_BIND_OPCODE | zoning::MOVE_OPCODE => zoning::offer(opcode, body).map_or_else(
            |error| unreadable(Part::ZoneOffer, &error),
            Message::ZoneOffer,
        ),
        PROFILE_OPCODE => titanium_profile_messages(body, &mut messages),
        zoning::CHANGE_OPCODE => zoning::ZoneAnswer::titanium(body).map_or_else(
            |error| unreadable(Part::ZoneAnswer, &error),
            Message::ZoneAnswer,
        ),
        zoning::HANDOFF_OPCODE => Message::Handoff(body.to_vec()),
        crate::boats::riders::TRANSFER_OPCODE => crate::boats::riders::Transfer::titanium(body)
            .map_or_else(
                |error| unreadable(Part::ZoneOffer, &error),
                Message::VehicleTransfer,
            ),
        LOGOUT_REPLY_OPCODE => Message::LoggedOut,
        // EQEmu's TRADESKILL_NOCOMBINE (zone/string_ids.h) accompanies
        // the early return that sends a reset without a station clear.
        0x673c if body.len() == 12 && body[..4] == 334_u32.to_le_bytes() => {
            Message::TradeskillNoRecipe
        }
        // EQEmu's SPELL_LEVEL_TO_LOW (zone/string_ids.h), a formatted
        // message with the level and spell name as its arguments.
        crate::spells::TITANIUM_FORMATTED_OPCODE
            if body.len() >= 12 && body[4..8] == crate::spells::LEVEL_REFUSAL.to_le_bytes() =>
        {
            Message::BookLevelRefused {
                arguments: crate::chat::parse_for(
                    crate::GameDialect::Titanium,
                    opcode,
                    body,
                    false,
                )
                .ok()
                .flatten()
                .and_then(|line| line.arguments)
                .map(|arguments| {
                    arguments
                        .into_iter()
                        .map(|argument| argument.text)
                        .collect()
                })
                .unwrap_or_default(),
            }
        }
        crate::merchant::BUY_OPCODE
        | crate::merchant::SELL_OPCODE
        | crate::merchant::DELETE_OPCODE => match crate::merchant::decode_scoped(opcode, body) {
            Ok((merchant_id, update)) => Message::MerchantScoped {
                merchant_id,
                update,
            },
            Err(error) => unreadable(Part::World, &error),
        },
        crate::training::OFFER_OPCODE => crate::training::titanium_reply(body).map_or_else(
            |error| unreadable(Part::World, &error),
            Message::TrainingReply,
        ),
        crate::resurrection::OFFER_OPCODE => crate::resurrection::titanium_offer(body).map_or_else(
            |error| unreadable(Part::World, &error),
            Message::Resurrection,
        ),
        crate::confirmations::TRANSLOCATE_OPCODE => {
            use crate::confirmations::TitaniumTranslocate;
            match crate::confirmations::titanium_translocate_record(body) {
                Ok(TitaniumTranslocate::Offer(offer)) => Message::TranslocateOffer(offer),
                Ok(TitaniumTranslocate::Completed(offer)) => Message::TranslocateCompleted(offer),
                Err(error) => unreadable(Part::World, &error),
            }
        }
        crate::buffs::TITANIUM_ACTION_OPCODE => action(crate::buffs::titanium_action(body)),
        crate::bazaar::SEARCH_OPCODE => return bazaar(body, messages),
        crate::bazaar::BROWSE_OPCODE => bazaar_event(crate::bazaar::admission(body)),
        crate::bazaar::SIGN_OPCODE => bazaar_event(crate::bazaar::sign(body)),
        crate::bazaar::SHOP_END_OPCODE => bazaar_event(crate::bazaar::shop_end(body)),
        crate::zones::TITANIUM_HEADER_OPCODE => {
            messages.extend(crate::clock::titanium_zone_sky(body).map(sky));
            described(crate::zones::titanium_description(body))
        }
        _ => match titanium_other(opcode, body) {
            Some(message) => message,
            None => return messages,
        },
    };
    messages.push(message);
    messages
}

/// What a Titanium packet no other reader took says: the task journal, the
/// inventory, or the rest of the world; None when nothing reads it.
fn titanium_other(opcode: u16, body: &[u8]) -> Option<Message> {
    if let Some(news) = crate::tasks::titanium(opcode, body) {
        return Some(news.map_or_else(
            |error| unreadable(Part::World, &error),
            |news| Message::Journal {
                news,
                length: body.len(),
            },
        ));
    }
    match inventory::decode(opcode, body) {
        Ok(Some(update)) => Some(Message::Event(WorldEvent::Inventory(update))),
        Err(error) => Some(unreadable(Part::Inventory, &error)),
        Ok(None) => match crate::world::titanium_update(opcode, body) {
            Ok(Some(WorldEvent::Merchant(crate::merchant::MerchantUpdate::Item(item)))) => {
                Some(Message::MerchantView {
                    serial: crate::merchant::view_serial(body),
                    length: body.len(),
                    item: Ok(item),
                })
            }
            Ok(Some(event)) => Some(Message::Event(event)),
            Ok(None) => None,
            // Every reader before the merchant's leaves a merchant item
            // packet alone, so its fault is the merchant's.
            Err(error) if crate::merchant::is_item_packet(opcode, body) => {
                Some(Message::MerchantView {
                    serial: crate::merchant::view_serial(body),
                    length: body.len(),
                    item: Err(error.to_string()),
                })
            }
            Err(error) => Some(unreadable(Part::World, &error)),
        },
    }
}

fn titanium_aa_message(opcode: u16, body: &[u8], messages: &mut Vec<Message>) -> Message {
    match crate::alternate_advancement::titanium(opcode, body) {
        Ok(update) => Message::AlternateAdvancement(update),
        Err(error) => match crate::alternate_advancement::invalidated(opcode, body) {
            Some(update) => {
                messages.push(unreadable(Part::World, &error));
                Message::AlternateAdvancement(update)
            }
            None => unreadable(Part::World, &error),
        },
    }
}

fn titanium_profile_messages(body: &[u8], messages: &mut Vec<Message>) -> Message {
    match crate::alternate_advancement::titanium_profile(body) {
        Ok(updates) => messages.extend(updates.map(Message::AlternateAdvancement)),
        Err(error) => {
            messages.push(unreadable(Part::World, &error));
            messages.extend(
                [
                    crate::alternate_advancement::Update::Selections(None),
                    crate::alternate_advancement::Update::Points(None),
                ]
                .map(Message::AlternateAdvancement),
            );
        }
    }
    messages.extend(crate::knights::titanium_profile(body).map(Message::KnightRefresh));
    messages.push(Message::VehicleName(
        crate::boats::riders::titanium_profile_name(body),
    ));
    match crate::group::titanium_profile(body) {
        Ok(group) => messages.extend(group.map(|update| Message::Event(WorldEvent::Group(update)))),
        Err(error) => messages.push(unreadable(Part::World, &error)),
    }
    messages.push(crate::guild_bank::titanium_profile(body).map_or_else(
        |error| unreadable(Part::World, &error),
        |banker| Message::Event(WorldEvent::GuildBank(banker)),
    ));
    messages.push(crate::buffs::titanium_profile(body).map_or_else(
        |error| unreadable(Part::World, &error),
        |buffs| Message::Event(WorldEvent::BuffSnapshot(buffs)),
    ));
    messages.push(crate::world::titanium_coins(body).map_or_else(
        |error| unreadable(Part::World, &error),
        |coins| Message::Event(WorldEvent::Coins(coins)),
    ));
    messages.push(Message::Event(WorldEvent::CorpseAutoConsent(
        crate::corpses::titanium_auto_consent_profile(body),
    )));
    messages.push(crate::split::titanium_profile(body).map_or_else(
        |error| unreadable(Part::World, &error),
        |on| Message::Event(WorldEvent::AutoSplit(on)),
    ));
    messages.push(crate::played::titanium_profile(body).map_or_else(
        |error| unreadable(Part::World, &error),
        |played| Message::Event(WorldEvent::PlayedTime(played)),
    ));
    messages.push(crate::food::titanium_profile(body).map_or_else(
        |error| unreadable(Part::World, &error),
        |nourishment| Message::Event(WorldEvent::Nourishment(nourishment)),
    ));
    // A bad set or belt field withdraws only these two, and says they are
    // unknown (6.559, 6.564).
    match crate::bandolier::titanium_profile(body) {
        Ok((sets, belt)) => messages.extend([
            Message::Event(WorldEvent::Bandoliers(Box::new(sets))),
            Message::Event(WorldEvent::PotionBelt(Box::new(belt))),
        ]),
        Err(error) => messages.extend([
            unreadable(Part::World, &error),
            Message::Event(WorldEvent::BandoliersUnknown),
        ]),
    }
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
    messages.push(crate::world::titanium_bind_zone(body).map_or_else(
        |error| unreadable(Part::World, &error),
        |zone_id| Message::Event(WorldEvent::BoundIn { zone_id }),
    ));
    spells::SpellBook::titanium_profile(body).map_or_else(
        |error| unreadable(Part::Spells, &error),
        |book| Message::Event(WorldEvent::SpellBook(book)),
    )
}

/// What one `EQMac` zone packet says: the spawns and the player's news that
/// [`crate::quarm::updates`] reads, the inventory, the server logging the
/// character out, its request that the client move, Quarm's move of a dead
/// player to their bind point and TAKP's word that a translocate the player
/// accepted lets them go, a translocate's offer, the server's answer to the
/// client's request, the zone's doors and ground objects, numbered destinations and header,
/// and the player's bind point from their profile.
#[must_use]
pub fn eqmac(opcode: u16, body: &[u8]) -> Vec<Message> {
    eqmac_with(opcode, body, inventory::ItemRules::Takp)
}

/// Quarm's `EQMac` messages, with its flag-based inventory stackability.
#[must_use]
pub fn quarm(opcode: u16, body: &[u8]) -> Vec<Message> {
    eqmac_with(opcode, body, inventory::ItemRules::Quarm)
}

/// An `EQMac` zone packet's messages under the server's item rules.
///
/// All servers share the generation's packet layouts. Only inventory
/// stackability is interpreted by `item_rules`; other messages and malformed
/// packet diagnostics are read as by [`eqmac`]. The fixed-policy entry points
/// [`eqmac`] and [`quarm`] retain their existing behavior.
#[must_use]
pub fn eqmac_with(opcode: u16, body: &[u8], item_rules: inventory::ItemRules) -> Vec<Message> {
    match opcode {
        // TAKP LevelUpdate_Struct; the session applies the Mac rise-only rule.
        0x9841 => vec![crate::world::eqmac_level_update(body)
            .map_or_else(|error| unreadable(Part::World, &error), Message::Event)],
        crate::books::EQMAC_READ_OPCODE => vec![crate::books::eqmac_text(body).map_or_else(
            |error| unreadable(Part::World, &error),
            |text| Message::Event(WorldEvent::BookText(text)),
        )],
        crate::quarm::ZONE_LOGOUT_REPLY => vec![Message::LoggedOut],
        crate::quarm::ZONE_CHANGE_REQUEST => vec![crate::quarm::zone_request(body).map_or_else(
            |error| unreadable(Part::ZoneOffer, &error),
            Message::ZoneOffer,
        )],
        crate::quarm::ZONE_TO_BIND => vec![crate::quarm::to_bind(body).map_or_else(
            |error| unreadable(Part::ZoneOffer, &error),
            Message::ZoneOffer,
        )],
        crate::confirmations::EQMAC_TRANSLOCATE_OPCODE => {
            vec![match crate::confirmations::eqmac_translocate(body) {
                Ok(crate::confirmations::Translocate::Offer(offer)) => {
                    Message::TranslocateOffer(offer)
                }
                Ok(crate::confirmations::Translocate::Go(offer)) => Message::ZoneOffer(offer),
                Err(error) => unreadable(Part::World, &error),
            }]
        }
        crate::quarm::ZONE_CHANGE => vec![crate::quarm::zone_answer(body).map_or_else(
            |error| unreadable(Part::ZoneAnswer, &error),
            Message::ZoneAnswer,
        )],
        crate::quarm::ZONE_POINTS => vec![zoning::ZonePoints::decode_eqmac(body).map_or_else(
            |error| unreadable(Part::ZonePoints, &error),
            Message::ZonePoints,
        )],
        crate::quarm::ZONE_PLAYER_PROFILE => eqmac_profile(body),
        crate::bazaar::eqmac::SEARCH_OPCODE => vec![eqmac_bazaar(body)],
        opcode if crate::alternate_advancement::is_eqmac(opcode) => {
            match crate::alternate_advancement::eqmac(opcode, body) {
                Ok(update) => vec![Message::AlternateAdvancement(update)],
                Err(error) => std::iter::once(unreadable(Part::World, &error))
                    .chain(
                        crate::alternate_advancement::eqmac_invalidated(opcode)
                            .map(Message::AlternateAdvancement),
                    )
                    .collect(),
            }
        }
        crate::buffs::EQMAC_ACTION_OPCODE => vec![action(crate::buffs::eqmac_action(body))],
        crate::quarm::ZONE_NEW => crate::clock::eqmac_zone_sky(body)
            .map(sky)
            .into_iter()
            .chain([described(crate::zones::eqmac_description(body))])
            .collect(),
        _ if let Some(message) = eqmac_people(opcode, body) => vec![message],
        _ if let Some(message) = eqmac_training(opcode, body) => vec![message],
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
        crate::merchant::EQMAC_STOCK_OPCODE => {
            vec![
                crate::merchant::eqmac_list_with(body, item_rules).map_or_else(
                    |error| unreadable(Part::World, &error),
                    Message::MerchantList,
                ),
            ]
        }
        crate::who::eqmac::FORMATTED_OPCODE if crate::who::eqmac::unmatched(opcode, body) => {
            vec![Message::WhoUnmatched]
        }
        crate::who::eqmac::FORMATTED_OPCODE if crate::split::eqmac_refused(opcode, body) => {
            vec![Message::SplitRefused]
        }
        _ if let Some(message) = eqmac_loot(opcode, body, item_rules) => vec![message],
        _ if let Some(message) = eqmac_crafts_and_boats(opcode, body) => vec![message],
        _ if let Some(message) = eqmac_objects(opcode, body) => vec![message],
        _ if let Some(message) = eqmac_exchange(opcode, body) => vec![message],
        _ => match inventory::decode_eqmac_with(opcode, body, item_rules) {
            Ok(Some(update)) => vec![Message::Event(WorldEvent::Inventory(update))],
            Err(error) => vec![unreadable(Part::Inventory, &error)],
            Ok(None) => crate::quarm::updates(opcode, body).map_or_else(
                |error| vec![unreadable(Part::World, &error)],
                |events| events.into_iter().map(Message::Event).collect(),
            ),
        },
    }
}

/// What TAKP and Quarm say of other players: a die rolled near the player,
/// the answer to an assist, a consent given to the player, and an offer
/// to resurrect the player and TAKP's completion of it (6.585, 6.591,
/// 6.596); None for other messages.
fn eqmac_people(opcode: u16, body: &[u8]) -> Option<Message> {
    let read = |result: anyhow::Result<WorldEvent>| {
        result.map_or_else(|error| unreadable(Part::World, &error), Message::Event)
    };
    Some(match opcode {
        crate::corpses::EQMAC_CONSENT_RESPONSE_OPCODE => {
            read(crate::corpses::decode_consent(body).map(WorldEvent::Consent))
        }
        crate::socials::EQMAC_ROLL_OPCODE => read(crate::socials::roll(body).map(WorldEvent::Roll)),
        crate::socials::EQMAC_ASSIST_OPCODE => {
            read(crate::socials::eqmac_assist_answer(body).map(WorldEvent::Assisted))
        }
        crate::resurrection::EQMAC_OFFER_OPCODE => crate::resurrection::eqmac_offer(body)
            .map_or_else(
                |error| unreadable(Part::World, &error),
                Message::Resurrection,
            ),
        crate::resurrection::EQMAC_COMPLETE_OPCODE => crate::resurrection::eqmac_completion(body)
            .map_or_else(
                |error| unreadable(Part::World, &error),
                Message::ResurrectionCompleted,
            ),
        _ => return None,
    })
}

/// A guildmaster's answer and a skill's silent update on TAKP and Quarm
/// (6.597); None for other messages.
fn eqmac_training(opcode: u16, body: &[u8]) -> Option<Message> {
    Some(match opcode {
        crate::training::EQMAC_OFFER_OPCODE => crate::training::eqmac_reply(body).map_or_else(
            |error| unreadable(Part::World, &error),
            Message::TrainingReply,
        ),
        crate::training::EQMAC_SKILL_OPCODE => crate::training::eqmac_skill(body)
            .map_or_else(|error| unreadable(Part::World, &error), Message::SpawnSkill),
        _ => return None,
    })
}

/// TAKP's and Quarm's combine answer and close answer (6.598) and boat
/// control echo (6.602); None for other messages.
fn eqmac_crafts_and_boats(opcode: u16, body: &[u8]) -> Option<Message> {
    Some(match opcode {
        crate::objects::eqmac::ACTION_OPCODE if body.is_empty() => Message::ContainerReleased,
        crate::boats::EQMAC_OPCODE => crate::boats::ControlBoat::eqmac(body).map_or_else(
            |error| unreadable(Part::World, &error),
            |packet| Message::Event(WorldEvent::BoatControl(packet)),
        ),
        crate::tradeskills::EQMAC_COMBINE_OPCODE => crate::tradeskills::eqmac_decode(body)
            .map_or_else(
                |error| unreadable(Part::World, &error),
                |answer| Message::Event(WorldEvent::Combine(answer)),
            ),
        _ => return None,
    })
}

/// Corpse views are decoded before inventory packets, so their places can
/// never be mistaken for carried slots.
fn eqmac_loot(opcode: u16, body: &[u8], rules: inventory::ItemRules) -> Option<Message> {
    match crate::loot::eqmac::decode(opcode, body, rules) {
        Ok(Some(update)) => Some(Message::Event(WorldEvent::Loot(update))),
        Ok(None) => None,
        Err(error) => Some(unreadable(Part::World, &error)),
    }
}

/// The generation's door/object news. Layout failures are world diagnostics,
/// never an inventory grant or a partially applied object table.
fn eqmac_objects(opcode: u16, body: &[u8]) -> Option<Message> {
    let decoded = match crate::doors::eqmac::decode(opcode, body) {
        Ok(Some(update)) => Ok(Some(WorldEvent::Doors(update))),
        Ok(None) => crate::objects::eqmac::decode(opcode, body)
            .map(|update| update.map(WorldEvent::Objects)),
        Err(error) => Err(error),
    };
    match decoded {
        Ok(Some(event)) => Some(Message::Event(event)),
        Ok(None) => None,
        Err(error) => Some(unreadable(Part::World, &error)),
    }
}

/// An `EQMac` exchange packet as the session takes it: an exchange event,
/// an item record no reader here takes yet among them, which the session
/// reports as a diagnostic, never an error that closes the session; None
/// for other opcodes.
fn eqmac_exchange(opcode: u16, body: &[u8]) -> Option<Message> {
    use crate::exchange::eqmac::{self, TradeItem, TradeMessage};
    Some(match eqmac::decode(opcode, body) {
        Ok(None) => return None,
        Ok(Some(TradeMessage::Update(update) | TradeMessage::Item(TradeItem::Read(update)))) => {
            Message::Event(WorldEvent::Exchange(update))
        }
        Ok(Some(TradeMessage::Item(TradeItem::Unread { index, kind }))) => Message::Event(
            WorldEvent::Exchange(crate::exchange::ExchangeUpdate::Unread { index, kind }),
        ),
        Err(error) => unreadable(Part::World, &error),
    })
}

/// What the `EQMac` profile says beside what the admission reads of it: the
/// player's buffs and spellbook, and their coins.
fn eqmac_profile(body: &[u8]) -> Vec<Message> {
    let mut messages = events(Part::Spells, crate::quarm::profile_spells(body));
    messages.extend(events(Part::World, crate::quarm::profile_coins(body)));
    messages.push(crate::quarm::profile_nourishment(body).map_or_else(
        |error| unreadable(Part::World, &error),
        |value| Message::Event(WorldEvent::Nourishment(value)),
    ));
    messages.push(crate::quarm::profile_auto_split(body).map_or_else(
        |error| unreadable(Part::World, &error),
        |on| Message::Event(WorldEvent::AutoSplit(on)),
    ));
    match crate::quarm::bind_point(body) {
        // The move home after death takes the whole point; the client is
        // told its zone, as Titanium's profile tells it (6.524).
        Ok(bind) => messages.extend([
            Message::Bind(bind),
            Message::Event(WorldEvent::BoundIn {
                zone_id: bind.zone_id,
            }),
        ]),
        Err(error) => messages.push(unreadable(Part::World, &error)),
    }
    // The profile lists the player's group as each zone admits them.
    match crate::quarm::profile_group(body) {
        Ok(group) => {
            messages.extend(group.map(|update| Message::Event(WorldEvent::Group(update))));
        }
        Err(error) => messages.push(unreadable(Part::World, &error)),
    }
    // Its AA array and points (6.548), last.
    match crate::quarm::profile_aa(body) {
        Ok(updates) => messages.extend(updates.map(Message::AlternateAdvancement)),
        Err(error) => {
            messages.push(unreadable(Part::World, &error));
            messages.extend(
                [
                    crate::alternate_advancement::Update::MacSelections(None),
                    crate::alternate_advancement::Update::MacPoints(None),
                ]
                .map(Message::AlternateAdvancement),
            );
        }
    }
    // The boat the player rode, which both servers keep across zones (6.602).
    messages.push(Message::VehicleName(crate::quarm::profile_vehicle(body)));
    messages
}

/// What the server's `OP_BazaarSearch` adds to the packet's messages: an
/// event; nothing for the parcel costs `EQEmu` sends after a search's
/// results, which only `RoF` and later use (the coverage table's needless
/// part); or a kind no server is known to send Titanium.
fn bazaar(body: &[u8], mut messages: Vec<Message>) -> Vec<Message> {
    messages.extend(match crate::bazaar::answer(body) {
        Ok(crate::bazaar::Answer::Event(event)) => Some(Message::Event(WorldEvent::Bazaar(event))),
        Ok(crate::bazaar::Answer::DeliveryCosts) => None,
        Ok(crate::bazaar::Answer::Other(kind)) => Some(Message::UnknownKind(kind)),
        Err(error) => Some(unreadable(Part::World, &error)),
    });
    messages
}

/// What a Mac-era server's `OP_BazaarSearch` says: an event, a kind no
/// server is known to send, or why it could not be read.
fn eqmac_bazaar(body: &[u8]) -> Message {
    match crate::bazaar::eqmac::answer(body) {
        Ok(crate::bazaar::eqmac::Answer::Event(event)) => Message::Event(WorldEvent::Bazaar(event)),
        Ok(crate::bazaar::eqmac::Answer::Other(kind)) => Message::UnknownKind(kind.into()),
        Err(error) => unreadable(Part::World, &error),
    }
}

/// A Bazaar event, or why its record could not be read.
fn bazaar_event(event: anyhow::Result<crate::bazaar::BazaarEvent>) -> Message {
    event.map_or_else(
        |error| unreadable(Part::World, &error),
        |event| Message::Event(WorldEvent::Bazaar(event)),
    )
}

/// What an action says: a spell's, the spin, or a kind no server is known
/// to send.
fn action(action: anyhow::Result<crate::buffs::Action>) -> Message {
    match action {
        Ok(crate::buffs::Action::Spell(effect)) => Message::Event(WorldEvent::SpellEffect(effect)),
        Ok(crate::buffs::Action::Spin) => Message::Event(WorldEvent::Spun),
        Ok(crate::buffs::Action::Other(kind)) => Message::UnknownKind(kind.into()),
        Err(error) => unreadable(Part::World, &error),
    }
}

/// A zone's sky, as its header gives it.
fn sky(sky: crate::clock::ZoneSky) -> Message {
    Message::Event(WorldEvent::Sky(sky))
}

/// The rest of a zone's description, or why its header could not be read.
fn described(description: anyhow::Result<crate::zones::ZoneDescription>) -> Message {
    description.map_or_else(
        |error| unreadable(Part::World, &error),
        |description| Message::Event(WorldEvent::ZoneDescription(description)),
    )
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
    fn a_bad_bandolier_field_withdraws_only_the_sets_and_belt() {
        let has = |messages: &[Message], test: fn(&WorldEvent) -> bool| {
            messages
                .iter()
                .any(|message| matches!(message, Message::Event(event) if test(event)))
        };
        let profile = vec![0; 19592];
        let read = titanium(PROFILE_OPCODE, &profile);
        assert!(has(
            &read,
            |event| matches!(event, WorldEvent::Bandoliers(sets) if sets.iter().all(Option::is_none))
        ));
        assert!(has(&read, |event| matches!(
            event,
            WorldEvent::PotionBelt(_)
        )));
        let unreadable = |messages: &[Message]| {
            messages
                .iter()
                .filter(|message| matches!(message, Message::Unreadable { .. }))
                .count()
        };
        let before = unreadable(&read);
        // The first set's name with no end within its 32 bytes.
        let mut bad = profile;
        bad[6164..6196].fill(b'a');
        let read = titanium(PROFILE_OPCODE, &bad);
        assert_eq!(unreadable(&read), before + 1);
        assert!(!has(&read, |event| matches!(
            event,
            WorldEvent::Bandoliers(_) | WorldEvent::PotionBelt(_)
        )));
        assert!(has(&read, |event| matches!(
            event,
            WorldEvent::BandoliersUnknown
        )));
        assert!(has(&read, |event| matches!(event, WorldEvent::Coins(_))));
        assert!(has(&read, |event| matches!(
            event,
            WorldEvent::BuffSnapshot(_)
        )));
    }

    #[test]
    fn a_merchant_item_packet_carries_its_serial() {
        let mut fields = vec!["0".to_owned(); 159];
        fields[1] = "Test ration".into();
        fields[4] = "13005".into();
        fields[5] = "25".into();
        fields[11] = "500".into();
        fields[154] = "-1".into();
        let mut text = format!("1|0|30|1250|1|0|90210|0|0|0|0|\"{}\"", fields.join("|"));
        text.push_str(&"|".repeat(10));
        text.push('\0');
        let mut body = crate::merchant::ITEM_PACKET_KIND.to_le_bytes().to_vec();
        body.extend_from_slice(text.as_bytes());
        let [Message::MerchantView {
            serial: Some(90_210),
            length,
            item: Ok(item),
        }] = &titanium(0x3397, &body)[..]
        else {
            panic!("a merchant view");
        };
        assert_eq!((item.slot, item.price, item.quantity), (30, 1250, 1));
        assert_eq!(*length, body.len());
        // A merchant item the merchant's reader refuses is still the
        // merchant's view, so a browse can end on it.
        let broken = &body[..20];
        assert!(matches!(
            titanium(0x3397, broken)[..],
            [Message::MerchantView {
                serial: None,
                length: 20,
                item: Err(_),
            }]
        ));
    }

    #[test]
    fn a_trader_click_sign_and_shop_end_are_bazaar_events() {
        let answer: Vec<u8> = [312_u32, 0, 0, 1]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        assert!(matches!(
            titanium(crate::bazaar::BROWSE_OPCODE, &answer)[..],
            [Message::Event(WorldEvent::Bazaar(
                crate::bazaar::BazaarEvent::Admitted {
                    trader: 312,
                    approved: true
                }
            ))]
        ));
        assert!(matches!(
            titanium(crate::bazaar::BROWSE_OPCODE, &answer[..8])[..],
            [Message::Unreadable { .. }]
        ));
        let mut sign = vec![0; crate::bazaar::SIGN_LENGTH];
        sign[0] = 40;
        sign[4] = 1;
        assert!(matches!(
            titanium(crate::bazaar::SIGN_OPCODE, &sign)[..],
            [Message::Event(WorldEvent::Bazaar(
                crate::bazaar::BazaarEvent::TraderSign {
                    spawn: 40,
                    on: true
                }
            ))]
        ));
        assert!(matches!(
            titanium(crate::bazaar::SHOP_END_OPCODE, &[])[..],
            [Message::Event(WorldEvent::Bazaar(
                crate::bazaar::BazaarEvent::ShopEnded
            ))]
        ));
    }

    #[test]
    fn a_bazaar_answer_is_an_event_nothing_a_kind_or_unreadable() {
        let mut done = vec![0; 20];
        done[0] = 12;
        assert!(matches!(
            titanium(crate::bazaar::SEARCH_OPCODE, &done)[..],
            [Message::Event(WorldEvent::Bazaar(
                crate::bazaar::BazaarEvent::SearchDone
            ))]
        ));
        let mut costs = vec![0; 14];
        costs[0] = 29;
        assert!(titanium(crate::bazaar::SEARCH_OPCODE, &costs).is_empty());
        assert!(matches!(
            titanium(crate::bazaar::SEARCH_OPCODE, &[22, 0, 0, 0])[..],
            [Message::UnknownKind(22)]
        ));
        let mut short_row = vec![0; crate::bazaar::ROW_LENGTH - 1];
        short_row[0] = 7;
        assert!(matches!(
            titanium(crate::bazaar::SEARCH_OPCODE, &short_row)[..],
            [Message::Unreadable {
                part: Part::World,
                ..
            }]
        ));
    }

    #[test]
    fn an_eqmac_bazaar_answer_is_a_row_the_end_a_kind_or_unreadable() {
        let mut row = vec![0; crate::bazaar::eqmac::ROW_LENGTH];
        row[0] = 7;
        row[2] = 1;
        row[12..18].copy_from_slice(b"Rusty ");
        assert!(matches!(
            &eqmac(0x1142, &row)[..],
            [Message::Event(WorldEvent::Bazaar(
                crate::bazaar::BazaarEvent::Results(rows)
            ))] if rows.len() == 1
        ));
        let mut done = vec![0; crate::bazaar::eqmac::DONE_LENGTH];
        done[0] = 12;
        assert!(matches!(
            quarm(0x1142, &done)[..],
            [Message::Event(WorldEvent::Bazaar(
                crate::bazaar::BazaarEvent::SearchDone
            ))]
        ));
        assert!(matches!(
            eqmac(0x1142, &[22, 0, 0, 0])[..],
            [Message::UnknownKind(22)]
        ));
        assert!(matches!(
            eqmac(0x1142, &row[..crate::bazaar::eqmac::ROW_LENGTH - 1])[..],
            [Message::Unreadable {
                part: Part::World,
                ..
            }]
        ));
    }

    #[test]
    fn an_action_is_a_spells_the_spin_or_a_kind_no_server_sends() {
        let mut spin = [0u8; 31];
        spin[..4].copy_from_slice(&[0x0b, 0x0a, 0x0b, 0x0a]);
        assert!(matches!(
            titanium(crate::buffs::TITANIUM_ACTION_OPCODE, &spin)[..],
            [Message::Event(WorldEvent::Spun)]
        ));
        let mut melee = spin;
        melee[..4].copy_from_slice(&[7, 0, 8, 0]);
        melee[22] = 1;
        assert!(matches!(
            titanium(crate::buffs::TITANIUM_ACTION_OPCODE, &melee)[..],
            [Message::UnknownKind(1)]
        ));
        melee[22] = 231;
        assert!(matches!(
            titanium(crate::buffs::TITANIUM_ACTION_OPCODE, &melee)[..],
            [Message::Event(WorldEvent::SpellEffect(_))]
        ));
        assert!(matches!(
            titanium(crate::buffs::TITANIUM_ACTION_OPCODE, &melee[..30])[..],
            [Message::Unreadable {
                part: Part::World,
                ..
            }]
        ));
    }

    #[test]
    fn aa_timers_and_experience_are_read_and_an_unreadable_one_says_so() {
        use crate::alternate_advancement::{Update, PROGRESS_OPCODE, TIMER_OPCODE};
        assert!(matches!(
            titanium(TIMER_OPCODE, &[0; 12])[..],
            [Message::AlternateAdvancement(Update::Timer(_))]
        ));
        assert!(matches!(
            titanium(PROGRESS_OPCODE, &[0; 12])[..],
            [Message::AlternateAdvancement(Update::Progress(Some(_)))]
        ));
        // A short timer is only unreadable; short experience also withdraws
        // the last.
        assert!(matches!(
            &titanium(TIMER_OPCODE, &[0; 11])[..],
            [Message::Unreadable { part: Part::World, error }] if error.ends_with("length 11")
        ));
        assert!(matches!(
            &titanium(PROGRESS_OPCODE, &[0; 13])[..],
            [
                Message::Unreadable { part: Part::World, error },
                Message::AlternateAdvancement(Update::Progress(None)),
            ] if error.ends_with("length 13")
        ));
    }

    #[test]
    fn the_profile_says_what_is_in_the_spellbook() {
        let mut profile = vec![0; 19592];
        profile[12940..12946].copy_from_slice(b"Tester");
        profile[2312..2316].copy_from_slice(&73u32.to_le_bytes());
        profile[13256] = 1;
        assert!(matches!(
            &titanium(PROFILE_OPCODE, &profile)[..],
            [
                Message::AlternateAdvancement(crate::alternate_advancement::Update::Selections(Some(_))),
                Message::AlternateAdvancement(crate::alternate_advancement::Update::Points(Some(_))),
                Message::VehicleName(None),
                Message::Event(WorldEvent::Group(crate::group::GroupUpdate::Disbanded)),
                Message::Event(WorldEvent::GuildBank(
                    crate::guild_bank::GuildBankUpdate::Banker(false)
                )),
                Message::Event(WorldEvent::BuffSnapshot(_)),
                Message::Event(WorldEvent::Coins(_)),
                Message::Event(WorldEvent::CorpseAutoConsent([Some(false); 3])),
                Message::Event(WorldEvent::AutoSplit(true)),
                Message::Event(WorldEvent::PlayedTime(_)),
                Message::Event(WorldEvent::Nourishment(_)),
                Message::Event(WorldEvent::Bandoliers(_)),
                Message::Event(WorldEvent::PotionBelt(_)),
                Message::Event(WorldEvent::CoinsElsewhere { .. }),
                Message::Event(WorldEvent::BoundIn { zone_id: 0 }),
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
                Message::AlternateAdvancement(crate::alternate_advancement::Update::Selections(
                    None
                )),
                Message::AlternateAdvancement(crate::alternate_advancement::Update::Points(None)),
                Message::VehicleName(None),
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
                Message::Event(WorldEvent::BandoliersUnknown),
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
        // The primary bind's zone is the first word of `binds[5]` at 24;
        // the second bind is not it (6.524).
        let mut profile = vec![0; 19592];
        profile[24..28].copy_from_slice(&29u32.to_le_bytes());
        profile[44..48].copy_from_slice(&4u32.to_le_bytes());
        assert_eq!(crate::world::titanium_bind_zone(&profile).unwrap(), 29);
        profile[24..28].copy_from_slice(&70_000u32.to_le_bytes());
        assert!(crate::world::titanium_bind_zone(&profile).is_err());
        assert!(crate::world::titanium_bind_zone(&profile[..24]).is_err());
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
    fn a_zone_header_says_its_sky_and_the_rest_of_its_description() {
        let header = |length: usize| {
            let mut header = vec![0; length];
            header[96..103].copy_from_slice(b"Example");
            header[474] = 1;
            header
        };
        // As GM commands and quests send it again: EQEmu's 740 bytes, of
        // which Titanium takes 700, and EQMac's 572.
        for messages in [
            titanium(crate::zones::TITANIUM_HEADER_OPCODE, &header(740)),
            eqmac(crate::quarm::ZONE_NEW, &header(572)),
        ] {
            assert!(matches!(
                &messages[..],
                [
                    Message::Event(WorldEvent::Sky(sky)),
                    Message::Event(WorldEvent::ZoneDescription(description)),
                ] if sky.sky == 1 && description.long_name == "Example"
            ));
        }
        // A header too short for its sky says only why it is unreadable.
        assert!(matches!(
            &titanium(crate::zones::TITANIUM_HEADER_OPCODE, &header(740)[..400])[..],
            [Message::Unreadable {
                part: Part::World,
                ..
            }]
        ));
        assert!(matches!(
            &eqmac(crate::quarm::ZONE_NEW, &header(700))[..],
            [Message::Unreadable {
                part: Part::World,
                ..
            }]
        ));
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
        use crate::quarm::{ZONE_CHANGE_REQUEST, ZONE_LOGOUT, ZONE_LOGOUT_REPLY, ZONE_TO_BIND};
        assert!(matches!(
            eqmac(ZONE_LOGOUT_REPLY, &[])[..],
            [Message::LoggedOut]
        ));
        // The client's own logout, which no server sends, says nothing.
        assert!(eqmac(ZONE_LOGOUT, &[]).is_empty());
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
        // Quarm's move of a dead player to their bind point.
        let mut to_bind = [0; 152];
        to_bind[132..136].copy_from_slice(&4u32.to_le_bytes());
        assert!(matches!(
            &eqmac(ZONE_TO_BIND, &to_bind)[..],
            [Message::ZoneOffer(offer)] if offer.zone_id == 4 && offer.to_bind && offer.solicited
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
        // How fed and watered TAKP says the player is.
        assert!(matches!(
            eqmac(0x5741, &[0xb8, 0x0b, 0x70, 0x17, 0])[..],
            [Message::Event(WorldEvent::Nourishment(
                crate::food::Nourishment {
                    fatigue: Some(0),
                    food: 3000,
                    water: 6000
                }
            ))]
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
    fn a_swing_is_news_for_the_host_on_both_client_generations() {
        use crate::combat::{Animation, ANIMATION_OPCODE, EQMAC_ANIMATION_OPCODE};
        assert!(matches!(
            titanium(ANIMATION_OPCODE, &[9, 0, 10, 5])[..],
            [Message::Event(WorldEvent::Animation(Animation {
                spawn_id: 9,
                action: 5,
                ..
            }))]
        ));
        let mut record = [0; 12];
        record[0] = 9;
        record[4] = 8;
        record[8..].copy_from_slice(&1.0f32.to_le_bytes());
        assert!(matches!(
            eqmac(EQMAC_ANIMATION_OPCODE, &record)[..],
            [Message::Event(WorldEvent::Animation(Animation {
                spawn_id: 9,
                action: 8,
                ..
            }))]
        ));
        // One that cannot be read is the usual diagnostic, and nothing more.
        record[4..8].copy_from_slice(&(-1i32).to_le_bytes());
        for messages in [
            titanium(ANIMATION_OPCODE, &[9, 0, 10]),
            eqmac(EQMAC_ANIMATION_OPCODE, &record[..11]),
            eqmac(EQMAC_ANIMATION_OPCODE, &record),
        ] {
            assert!(matches!(
                messages[..],
                [Message::Unreadable {
                    part: Part::World,
                    ..
                }]
            ));
        }
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
    fn an_eqmac_translocate_is_an_offer_or_takps_word_to_go() {
        let mut body = vec![0; 92];
        body[..4].copy_from_slice(&4u32.to_le_bytes());
        body[12..18].copy_from_slice(b"Tester");
        let opcode = crate::confirmations::EQMAC_TRANSLOCATE_OPCODE;
        assert!(matches!(
            &eqmac(opcode, &body)[..],
            [Message::TranslocateOffer(offer)] if offer.zone_id == 4
        ));
        body[88..].copy_from_slice(&1u32.to_le_bytes());
        assert!(matches!(
            &eqmac(opcode, &body)[..],
            [Message::ZoneOffer(offer)] if offer.zone_id == 4 && offer.solicited && !offer.to_bind
        ));
        body[88..].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(matches!(
            &eqmac(opcode, &body)[..],
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
        // A profile that cannot be read leaves the spells, the coins, the
        // nourishment, the automatic splitting, the bind point, the group
        // and the AA unknown, and names no boat.
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
                    part: Part::World,
                    ..
                },
                Message::AlternateAdvancement(crate::alternate_advancement::Update::MacSelections(
                    None
                )),
                Message::AlternateAdvancement(crate::alternate_advancement::Update::MacPoints(
                    None
                )),
                Message::VehicleName(None),
            ]
        ));
    }

    #[test]
    fn eqmac_rolls_assists_and_split_refusals_are_read() {
        // A die rolled near the player, and the server's answer to an assist
        // (6.585); a malformed one is a gap, never a crash.
        let mut rolled = [0; 76];
        rolled[0..4].copy_from_slice(&1u32.to_le_bytes());
        rolled[4..8].copy_from_slice(&6u32.to_le_bytes());
        rolled[8..12].copy_from_slice(&4u32.to_le_bytes());
        rolled[12..18].copy_from_slice(b"Roller");
        assert!(matches!(
            &eqmac(crate::socials::EQMAC_ROLL_OPCODE, &rolled)[..],
            [Message::Event(WorldEvent::Roll(roll))]
                if roll.low == 1 && roll.high == 6 && roll.result == 4 && roll.name == "Roller"
        ));
        assert!(matches!(
            &eqmac(crate::socials::EQMAC_ASSIST_OPCODE, &[0x34, 0x12])[..],
            [Message::Event(WorldEvent::Assisted(
                crate::socials::Assisted {
                    target: Some(0x1234)
                }
            ))]
        ));
        for (opcode, body) in [
            (crate::socials::EQMAC_ROLL_OPCODE, &rolled[..75]),
            (crate::socials::EQMAC_ASSIST_OPCODE, &[0x34, 0x12, 0, 0][..]),
        ] {
            assert!(matches!(
                eqmac(opcode, body)[..],
                [Message::Unreadable {
                    part: Part::World,
                    ..
                }]
            ));
        }
        // A consent given to the player, as only the one consented hears it
        // on these servers (6.591).
        let mut consent = [0; 161];
        consent[..6].copy_from_slice(b"Helper");
        consent[64..69].copy_from_slice(b"Owner");
        consent[128] = 1;
        consent[129..138].copy_from_slice(b"all zones");
        assert!(matches!(
            &eqmac(crate::corpses::EQMAC_CONSENT_RESPONSE_OPCODE, &consent)[..],
            [Message::Event(WorldEvent::Consent(given))]
                if given.granted == "Helper" && given.owner == "Owner" && given.given
                    && given.zone == "all zones"
        ));
        assert!(matches!(
            eqmac(
                crate::corpses::EQMAC_CONSENT_RESPONSE_OPCODE,
                &consent[..160]
            )[..],
            [Message::Unreadable {
                part: Part::World,
                ..
            }]
        ));
        // A split refused, as the line's number says; the line itself goes
        // as chat.
        let mut refused = vec![0, 0];
        refused.extend_from_slice(&12328u16.to_le_bytes());
        refused.extend_from_slice(&[0; 8]);
        assert!(matches!(
            eqmac(crate::who::eqmac::FORMATTED_OPCODE, &refused)[..],
            [Message::SplitRefused]
        ));
    }

    #[test]
    fn eqmac_aa_packets_are_read_and_an_unreadable_one_says_so() {
        use crate::alternate_advancement::{
            Update, EQMAC_PROGRESS_OPCODE, EQMAC_RANKS_OPCODE, EQMAC_TIMER_OPCODE,
        };
        assert!(matches!(
            eqmac(EQMAC_RANKS_OPCODE, &[0; 227])[..],
            [Message::AlternateAdvancement(Update::MacRanks(Some(_)))]
        ));
        assert!(matches!(
            eqmac(EQMAC_PROGRESS_OPCODE, &[0; 8])[..],
            [Message::AlternateAdvancement(Update::MacProgress(Some(_)))]
        ));
        assert!(matches!(
            eqmac(EQMAC_TIMER_OPCODE, &[0; 12])[..],
            [Message::AlternateAdvancement(Update::MacTimer(_))]
        ));
        assert!(matches!(
            &eqmac(EQMAC_RANKS_OPCODE, &[0; 226])[..],
            [
                Message::Unreadable { part: Part::World, error },
                Message::AlternateAdvancement(Update::MacRanks(None)),
            ] if error.ends_with("length 226")
        ));
        assert!(matches!(
            &eqmac(EQMAC_TIMER_OPCODE, &[0; 256])[..],
            [Message::Unreadable { part: Part::World, error }] if error.ends_with("length 256")
        ));
    }
}
