//! What the zone session asks of the server, in no client generation's
//! terms, and the Titanium client's packet for each request: the outgoing
//! side of [`crate::message`]. Each generation encodes requests its own way,
//! and one it has not built yet is refused. A request says only what the
//! player wants; who asks comes with it as the [`Sender`](crate::request::Sender), for the packets
//! that repeat the player's name or spawn.
use crate::{
    abilities::Ability,
    books::{self, Book},
    command::{self, EncodedCommand, GameCommand, Posture},
    corpses, doors, exchange,
    food::{self, Meal},
    group,
    hazards::{self, Hazard},
    inventory::{self, InventorySlot, MoveQuantity},
    listing::{self, Anonymity},
    money::CoinTransfer,
    movement::{self, PositionPacket},
    objects,
    pets::{self, PetCommand},
    raid,
    resurrection::{self, ResurrectionOffer},
    socials, spells, tradeskills, training,
    who::{self, WhoFilter},
    world::Position,
    zoning, GameDialect,
};
use anyhow::Result;

/// Who the session speaks for. Some packets repeat the player's name or
/// spawn, so the wire takes them from here rather than every request
/// carrying them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sender<'a> {
    /// The player's name.
    pub name: &'a str,
    /// The player's spawn, once the zone has spawned them.
    pub spawn_id: Option<u16>,
}

impl Sender<'_> {
    /// The player's spawn, or zero, which every packet's own check refuses.
    fn spawn(self) -> u16 {
        self.spawn_id.unwrap_or(0)
    }
}

/// What the zone session asks of the server.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Start the server's camp timer.
    Camp,
    /// Log the camped character out.
    Logout,
    /// Sit, stand or crouch the player.
    Posture(Posture),
    /// Ask the world who is online.
    Who(WhoFilter),
    /// Let a player drag the player's corpses, or take that back.
    Consent {
        /// Who may drag them.
        name: String,
        /// Whether consent is given or taken back.
        given: bool,
    },
    /// Summon one of the player's corpses to them, by the corpse's name.
    SummonCorpse(String),
    /// Start dragging a corpse, by its name.
    DragCorpse(String),
    /// Let go of one dragged corpse, or of every one.
    DropCorpse {
        /// The corpse, by its name; every dragged corpse when absent.
        corpse: Option<String>,
    },
    /// Command the player's pet.
    Pet {
        /// What the pet should do.
        command: PetCommand,
        /// The spawn it acts on, for commands that take one.
        target: Option<u16>,
    },
    /// Ask a guildmaster to train.
    OpenTraining(u16),
    /// Practice a skill once with a guildmaster.
    Train {
        /// The guildmaster's spawn.
        trainer: u16,
        /// The skill's number.
        skill: u32,
    },
    /// Leave training with a guildmaster.
    EndTraining(u16),
    /// Ask for a book's or note's text.
    ReadBook(Book),
    /// Combine what the tradeskill container in a pack slot holds.
    Combine(InventorySlot),
    /// Accept or decline a resurrection offer.
    AnswerResurrection {
        /// The offer, which the answer repeats.
        offer: ResurrectionOffer,
        /// True accepts.
        accept: bool,
    },
    /// Invite a player into the player's group, by name.
    InviteToGroup(String),
    /// Join the group of the one who invited the player, by their name.
    FollowGroup(String),
    /// Decline an invitation, by the inviter's name.
    DeclineGroup(String),
    /// Leave or disband the group, as the server decides by its idea of the
    /// player's target.
    Disband,
    /// Say the player is away from the keyboard (true), or back.
    SetAway(bool),
    /// Say how the player hides from `/who`.
    SetAnonymity(Anonymity),
    /// Roll a die from the lowest to the highest number.
    Random {
        /// The lowest number.
        low: u32,
        /// The highest.
        high: u32,
    },
    /// Emote, in the player's own words.
    Emote(String),
    /// Take the target of this spawn.
    Assist(u16),
    /// Invite a player into the player's raid, by name.
    RaidInvite(String),
    /// Join the raid of the one who invited the player, by their name.
    RaidAccept(String),
    /// Leave the player's raid.
    RaidLeave,
    /// Lock the player's raid (true) or unlock it.
    RaidLock(bool),
    /// Move a member of the player's raid, by the name the server gave them,
    /// into a raid group or out of every group.
    RaidMove {
        /// Who moves.
        member: String,
        /// Where to: a raid group, 0 to 11, or none.
        group: Option<u8>,
    },
    /// Hand the lead of the player's raid to a member, by the name the
    /// server gave them.
    RaidMakeLeader(String),
    /// Remove a member from the player's raid, by the name the server gave
    /// them.
    RaidRemove(String),
    /// Use a skill on the server's idea of the target.
    Ability {
        /// The skill.
        ability: Ability,
        /// The target the server has for the player, if any.
        target: Option<u16>,
    },
    /// Ask another character, or an NPC, to trade, by their spawn.
    Trade(u16),
    /// Take another player's request to trade, by the asker's spawn, or
    /// answer that another trade keeps the player busy.
    AnswerTrade {
        /// The one who asked.
        asker: u32,
        /// Whether the player is busy with another trade.
        busy: bool,
    },
    /// Accept the open trade.
    AcceptTrade,
    /// Close the open trade, or withdraw a request.
    CancelTrade,
    /// Use a door within reach.
    ClickDoor(u8),
    /// Pick an item up from the ground, by its drop.
    PickUp(u32),
    /// Open a world container, by its drop.
    OpenContainer(u32),
    /// Close a world container the server opened for the player, repeating
    /// the record the server sent for it.
    CloseContainer {
        /// The container.
        drop_id: u32,
        /// Its tradeskill kind.
        object_type: u32,
        /// The icon its window showed.
        icon: u32,
        /// The name its window showed.
        name: String,
    },
    /// Ask the zone the player is leaving to save them, once it approves
    /// the transfer.
    SaveOnZone,
    /// Take the player's own spawn out of the zone they are leaving: the
    /// last word to it before the world server.
    Depart,
    /// Take a transfer the server offered or a zone line asked for.
    AnswerZoneOffer {
        /// The zone, or zero for the bind point the server resolves.
        zone_id: u16,
        /// The zone's instance.
        instance_id: u16,
        /// Where in it.
        position: Position,
        /// The offer's reason, echoed back.
        reason: u32,
    },
    /// Memorize a scribed spell into a gem.
    Memorize {
        /// The gem.
        gem: u8,
        /// The spell.
        spell_id: u32,
    },
    /// Forget a gem's spell, keeping it in the book.
    Forget {
        /// The gem.
        gem: u8,
        /// The spell it holds.
        spell_id: u32,
    },
    /// Scribe the cursor's scroll into an empty book slot.
    Scribe {
        /// The book slot.
        slot: u16,
        /// The scroll's spell.
        spell_id: u32,
    },
    /// Delete a book entry.
    DeleteSpell {
        /// The book slot.
        slot: u16,
    },
    /// Exchange two book entries.
    SwapSpells {
        /// One book slot.
        from: u16,
        /// The other.
        to: u16,
    },
    /// A host command that needs nothing from the session's state, as the
    /// generation encodes it.
    Command(GameCommand),
    /// Cast an item's click effect.
    CastItem {
        /// The effect's spell.
        spell_id: u32,
        /// Where the item is.
        slot: InventorySlot,
        /// The spawn it is cast on.
        target_id: u16,
    },
    /// Move an item between slots.
    MoveItem {
        /// Where it is.
        from: InventorySlot,
        /// Where it goes.
        to: InventorySlot,
        /// The whole item, or part of a stack.
        quantity: MoveQuantity,
    },
    /// Move coins between places, or change them into another coin.
    MoveCoins(CoinTransfer),
    /// Eat or drink.
    Consume {
        /// Where the food or drink is.
        slot: InventorySlot,
        /// Which it is.
        meal: Meal,
        /// Whether the player chose it, rather than hunger or thirst.
        by_hand: bool,
    },
    /// Where the player is: a position sample.
    Position(PositionPacket),
    /// The player jumped.
    Jump,
    /// The world hurt the player: the damage the client worked out.
    EnvironmentalDamage {
        /// What did it.
        hazard: Hazard,
        /// How much, before the server's own reductions.
        amount: u32,
    },
}

/// The Titanium client's packet for a request from this sender.
///
/// # Errors
/// Rejects a request whose values the packet cannot carry, such as a name
/// too long for its field, and one that needs a spawn the player lacks.
pub fn titanium(request: &Request, sender: Sender<'_>) -> Result<EncodedCommand> {
    Ok(match request {
        Request::Camp => command::titanium_camp(),
        Request::Logout => command::titanium_logout(),
        Request::Posture(posture) => command::titanium_posture(sender.spawn(), *posture)?,
        Request::Who(filter) => who::request(filter)?,
        Request::Consent { name, given } => corpses::consent(name, *given)?,
        Request::SummonCorpse(corpse) => corpses::summon(corpse, sender.name)?,
        Request::DragCorpse(corpse) => corpses::drag(corpse, sender.name)?,
        Request::DropCorpse { corpse } => corpses::release(corpse.as_deref())?,
        Request::Pet { command, target } => pets::command(*command, *target),
        Request::Ability { ability, target } => ability.encode(target.unwrap_or(0)),
        Request::OpenTraining(trainer) => training::titanium_open(*trainer, sender.spawn()),
        Request::Train { trainer, skill } => training::titanium_train(*trainer, *skill)?,
        Request::EndTraining(trainer) => training::titanium_end(*trainer, sender.spawn()),
        Request::AnswerResurrection { offer, accept } => {
            resurrection::titanium_answer(offer, *accept)
        }
        Request::InviteToGroup(name) => group::invite(name, sender.name)?,
        Request::FollowGroup(inviter) => group::follow(inviter, sender.name)?,
        Request::DeclineGroup(inviter) => group::decline(inviter, sender.name)?,
        Request::Disband => group::disband(sender.name)?,
        Request::SetAway(away) => listing::titanium_away(sender.spawn(), *away)?,
        Request::SetAnonymity(anonymity) => {
            listing::titanium_anonymity(sender.spawn(), *anonymity)?
        }
        Request::Random { low, high } => socials::random(*low, *high),
        Request::Emote(text) => socials::emote(text)?,
        Request::Assist(spawn_id) => socials::assist(*spawn_id),
        Request::RaidInvite(name) => raid::invite(name, sender.name)?,
        Request::RaidAccept(inviter) => raid::accept(inviter, sender.name)?,
        Request::RaidLeave => raid::remove(sender.name, sender.name)?,
        Request::RaidLock(locked) => raid::lock(sender.name, *locked)?,
        Request::RaidMove { member, group } => raid::move_member(sender.name, member, *group)?,
        Request::RaidMakeLeader(member) => raid::make_leader(sender.name, member)?,
        Request::RaidRemove(member) => raid::remove(sender.name, member)?,
        Request::ReadBook(book) => books::titanium_request(book)?,
        Request::Combine(container) => tradeskills::titanium_combine(*container)?,
        Request::Trade(with) => exchange::request(sender.spawn(), *with)?,
        Request::AnswerTrade { asker, busy: false } => {
            exchange::acknowledge(sender.spawn(), *asker)?
        }
        Request::AnswerTrade { asker, busy: true } => exchange::busy(sender.spawn(), *asker)?,
        Request::AcceptTrade => exchange::accept(sender.spawn())?,
        Request::CancelTrade => exchange::cancel(sender.spawn())?,
        Request::ClickDoor(door_id) => doors::titanium_click(*door_id, sender.spawn()),
        // A click opens a world container as it picks an item up.
        Request::PickUp(drop_id) | Request::OpenContainer(drop_id) => {
            objects::titanium_pickup(*drop_id, sender.spawn())
        }
        Request::CloseContainer {
            drop_id,
            object_type,
            icon,
            name,
        } => objects::titanium_close(
            u32::from(sender.spawn()),
            *drop_id,
            (*object_type, *icon),
            name,
        ),
        Request::AnswerZoneOffer {
            zone_id,
            instance_id,
            position,
            reason,
        } => zoning::titanium_answer(sender.name, (*zone_id, *instance_id), *position, *reason)?,
        Request::SaveOnZone => zoning::titanium_save_on_zone(),
        Request::Depart => zoning::titanium_depart(sender.spawn()),
        Request::Memorize { gem, spell_id } => spells::titanium_memorize(*gem, *spell_id),
        Request::Forget { gem, spell_id } => spells::titanium_forget(*gem, *spell_id),
        Request::Scribe { slot, spell_id } => spells::titanium_scribe(*slot, *spell_id),
        Request::DeleteSpell { slot } => spells::titanium_delete(*slot),
        Request::SwapSpells { from, to } => spells::titanium_swap(*from, *to),
        Request::Command(command) => command::encode(GameDialect::Titanium, command, sender.name)?,
        Request::CastItem {
            spell_id,
            slot,
            target_id,
        } => inventory::titanium_item_cast(*spell_id, *slot, *target_id)?,
        Request::MoveItem { from, to, quantity } => {
            inventory::titanium_move(*from, *to, *quantity)?
        }
        Request::MoveCoins(transfer) => transfer.encode()?,
        Request::Consume {
            slot,
            meal,
            by_hand,
        } => food::consume(*slot, *meal, *by_hand),
        Request::Position(sample) => sample.packet()?,
        Request::Jump => movement::titanium_jump(),
        Request::EnvironmentalDamage { hazard, amount } => {
            hazards::titanium_damage(sender.spawn(), *hazard, *amount)?
        }
    })
}

/// The `EQMac` client's packet for a request from this sender: camping,
/// logging out, its stance and position, the world's damage, an item's
/// click, and the host commands its generation encodes so far: chat and
/// casting from a gem.
///
/// # Errors
/// Refuses every other request, and a command the generation cannot carry.
pub fn eqmac(request: &Request, sender: Sender<'_>) -> Result<EncodedCommand> {
    match request {
        Request::Camp => Ok(crate::quarm::camp()),
        Request::Logout => Ok(crate::quarm::logout()),
        Request::Posture(posture) => crate::quarm::posture(sender.spawn(), *posture),
        Request::Position(sample) => crate::quarm::client_update(sample),
        Request::EnvironmentalDamage { hazard, amount } => {
            hazards::eqmac_damage(sender.spawn(), *hazard, *amount)
        }
        Request::Command(command) => command::encode(GameDialect::EqMac, command, sender.name),
        // EQMac has no instances, and its request carries no position.
        Request::AnswerZoneOffer {
            zone_id,
            instance_id: 0,
            reason,
            ..
        } => crate::quarm::zone_change(sender.name, *zone_id, *reason),
        Request::SaveOnZone => Ok(crate::quarm::save_on_zone()),
        Request::Depart => Ok(crate::quarm::depart(sender.spawn())),
        Request::CastItem {
            spell_id,
            slot,
            target_id,
        } => spells::eqmac_item_cast(*spell_id, *slot, *target_id),
        _ => anyhow::bail!("the EQMac client cannot send {request:?} yet"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A player with a spawn.
    const PLAYER: Sender<'static> = Sender {
        name: "Tester",
        spawn_id: Some(7),
    };

    #[test]
    fn each_generation_departs_and_answers_a_zone_offer_its_own_way() {
        let answer = |instance_id| Request::AnswerZoneOffer {
            zone_id: 4,
            instance_id,
            position: crate::world::Position::default(),
            reason: 0,
        };
        assert_eq!(
            eqmac(&answer(0), PLAYER).unwrap(),
            crate::quarm::zone_change("Tester", 4, 0).unwrap()
        );
        // EQMac has no instances.
        assert!(eqmac(&answer(2), PLAYER).is_err());
        assert_eq!(
            titanium(&Request::SaveOnZone, PLAYER).unwrap(),
            crate::zoning::titanium_save_on_zone()
        );
        assert_eq!(
            titanium(&Request::Depart, PLAYER).unwrap(),
            crate::zoning::titanium_depart(7)
        );
        assert_eq!(
            eqmac(&Request::SaveOnZone, PLAYER).unwrap(),
            crate::quarm::save_on_zone()
        );
        assert_eq!(
            eqmac(&Request::Depart, PLAYER).unwrap(),
            crate::quarm::depart(7)
        );
    }

    #[test]
    fn titanium_requests_are_the_packets_the_codecs_build() {
        assert_eq!(
            titanium(&Request::Camp, PLAYER).unwrap(),
            command::titanium_camp()
        );
        assert_eq!(
            titanium(&Request::Posture(Posture::Sitting), PLAYER).unwrap(),
            command::titanium_posture(7, Posture::Sitting).unwrap()
        );
        assert_eq!(
            titanium(
                &Request::Pet {
                    command: PetCommand::Attack,
                    target: Some(9),
                },
                PLAYER
            )
            .unwrap(),
            pets::command(PetCommand::Attack, Some(9))
        );
        assert_eq!(
            titanium(&Request::DropCorpse { corpse: None }, PLAYER).unwrap(),
            corpses::release(None).unwrap()
        );
        // Training names the guildmaster, and opening and leaving the player.
        assert_eq!(
            titanium(&Request::OpenTraining(42), PLAYER).unwrap(),
            training::titanium_open(42, 7)
        );
        assert_eq!(
            titanium(
                &Request::Train {
                    trainer: 42,
                    skill: 30
                },
                PLAYER
            )
            .unwrap(),
            training::titanium_train(42, 30).unwrap()
        );
        assert_eq!(
            titanium(&Request::EndTraining(42), PLAYER).unwrap(),
            training::titanium_end(42, 7)
        );
        // What a packet cannot carry is refused, as the codec refuses it.
        assert!(titanium(&Request::Trade(7), PLAYER).is_err());
        // A packet repeating the sender takes their name and spawn.
        assert_eq!(
            titanium(&Request::ClickDoor(3), PLAYER).unwrap(),
            doors::titanium_click(3, 7)
        );
        assert_eq!(
            titanium(&Request::SummonCorpse("Tester's corpse4".into()), PLAYER).unwrap(),
            corpses::summon("Tester's corpse4", "Tester").unwrap()
        );
        // Without a spawn, those packets are refused as their codecs refuse
        // a zero spawn.
        let unspawned = Sender {
            spawn_id: None,
            ..PLAYER
        };
        assert!(titanium(&Request::AcceptTrade, unspawned).is_err());
        // Another player's request is taken, or answered as busy.
        let answer = |busy| Request::AnswerTrade { asker: 50, busy };
        assert_eq!(
            titanium(&answer(false), PLAYER).unwrap(),
            exchange::acknowledge(7, 50).unwrap()
        );
        assert_eq!(
            titanium(&answer(true), PLAYER).unwrap(),
            exchange::busy(7, 50).unwrap()
        );
        assert!(eqmac(&answer(false), PLAYER).is_err());
    }

    #[test]
    fn each_generation_reports_the_worlds_damage_in_its_own_packet() {
        let fall = Request::EnvironmentalDamage {
            hazard: Hazard::Falling,
            amount: 160,
        };
        assert_eq!(
            titanium(&fall, PLAYER).unwrap(),
            hazards::titanium_damage(7, Hazard::Falling, 160).unwrap()
        );
        assert_eq!(
            eqmac(&fall, PLAYER).unwrap(),
            hazards::eqmac_damage(7, Hazard::Falling, 160).unwrap()
        );
        let unspawned = Sender {
            spawn_id: None,
            ..PLAYER
        };
        assert!(titanium(&fall, unspawned).is_err());
        assert!(eqmac(&fall, unspawned).is_err());
    }
}
