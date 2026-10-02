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
    inventory::{self, InventorySlot, MoveQuantity},
    money::CoinTransfer,
    movement::{self, PositionPacket},
    objects,
    pets::{self, PetCommand},
    resurrection::{self, ResurrectionOffer},
    spells, training,
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
    /// Accept or decline a resurrection offer.
    AnswerResurrection {
        /// The offer, which the answer repeats.
        offer: ResurrectionOffer,
        /// True accepts.
        accept: bool,
    },
    /// Use a skill on the server's idea of the target.
    Ability {
        /// The skill.
        ability: Ability,
        /// The target the server has for the player, if any.
        target: Option<u16>,
    },
    /// Ask another character, or an NPC, to trade, by their spawn.
    Trade(u16),
    /// Accept the open trade.
    AcceptTrade,
    /// Close the open trade, or withdraw a request.
    CancelTrade,
    /// Use a door within reach.
    ClickDoor(u8),
    /// Pick an item up from the ground, by its drop.
    PickUp(u32),
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
        Request::ReadBook(book) => books::titanium_request(book)?,
        Request::Trade(with) => exchange::request(sender.spawn(), *with)?,
        Request::AcceptTrade => exchange::accept(sender.spawn())?,
        Request::CancelTrade => exchange::cancel(sender.spawn())?,
        Request::ClickDoor(door_id) => doors::titanium_click(*door_id, sender.spawn()),
        Request::PickUp(drop_id) => objects::titanium_pickup(*drop_id, sender.spawn()),
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
    })
}

/// The `EQMac` client's packet for a request from this sender: only the
/// host commands its generation encodes so far, which is chat.
///
/// # Errors
/// Refuses every other request, and a command the generation cannot carry.
pub fn eqmac(request: &Request, sender: Sender<'_>) -> Result<EncodedCommand> {
    match request {
        Request::Command(command) => command::encode(GameDialect::EqMac, command, sender.name),
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
    }
}
