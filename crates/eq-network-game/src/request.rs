//! What the zone session asks of the server, in no client generation's
//! terms, and the Titanium client's packet for each request: the outgoing
//! side of [`crate::message`]. Each generation encodes requests its own way,
//! and one it has not built yet is refused.
use crate::{
    abilities::Ability,
    command::{self, EncodedCommand, Posture},
    corpses, doors, exchange,
    objects::{self, ContainerView},
    pets::{self, PetCommand},
    spells,
    who::{self, WhoFilter},
    zoning::ZoneOffer,
};
use anyhow::Result;

/// What the zone session asks of the server.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Start the server's camp timer.
    Camp,
    /// Log the camped character out.
    Logout,
    /// Sit, stand or crouch the player.
    Posture {
        /// The player's spawn.
        spawn_id: u16,
        /// The stance to take.
        posture: Posture,
    },
    /// Ask the world who is online.
    Who(WhoFilter),
    /// Let a player drag the player's corpses, or take that back.
    Consent {
        /// Who may drag them.
        name: String,
        /// Whether consent is given or taken back.
        given: bool,
    },
    /// Summon one of the player's corpses to them.
    SummonCorpse {
        /// The corpse, by its name.
        corpse: String,
        /// The player's name.
        player: String,
    },
    /// Start dragging a corpse.
    DragCorpse {
        /// The corpse, by its name.
        corpse: String,
        /// The player's name.
        dragger: String,
    },
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
    /// Use a skill on the server's idea of the target.
    Ability {
        /// The skill.
        ability: Ability,
        /// The target the server has for the player; zero for none.
        target: u16,
    },
    /// Ask another character, or an NPC, to trade.
    Trade {
        /// The player's spawn.
        own_id: u16,
        /// The other character's spawn.
        with: u16,
    },
    /// Accept the open trade.
    AcceptTrade {
        /// The player's spawn.
        own_id: u16,
    },
    /// Close the open trade, or withdraw a request.
    CancelTrade {
        /// The player's spawn.
        own_id: u16,
    },
    /// Use a door within reach.
    ClickDoor {
        /// The door.
        door_id: u8,
        /// The player's spawn.
        player_id: u16,
    },
    /// Pick an item up from the ground.
    PickUp {
        /// The item on the ground.
        drop_id: u32,
        /// The player's spawn.
        player_id: u16,
    },
    /// Close a world container the server opened for the player.
    CloseContainer(ContainerView),
    /// Take a transfer the server offered or a zone line asked for.
    AnswerZoneOffer {
        /// Where to, as offered.
        offer: ZoneOffer,
        /// The player's name, which the answer repeats.
        character: String,
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
}

/// The Titanium client's packet for a request.
///
/// # Errors
/// Rejects a request whose values the packet cannot carry, such as a name
/// too long for its field.
pub fn titanium(request: &Request) -> Result<EncodedCommand> {
    Ok(match request {
        Request::Camp => command::titanium_camp(),
        Request::Logout => command::titanium_logout(),
        Request::Posture { spawn_id, posture } => command::titanium_posture(*spawn_id, *posture)?,
        Request::Who(filter) => who::request(filter)?,
        Request::Consent { name, given } => corpses::consent(name, *given)?,
        Request::SummonCorpse { corpse, player } => corpses::summon(corpse, player)?,
        Request::DragCorpse { corpse, dragger } => corpses::drag(corpse, dragger)?,
        Request::DropCorpse { corpse } => corpses::release(corpse.as_deref())?,
        Request::Pet { command, target } => pets::command(*command, *target),
        Request::Ability { ability, target } => ability.encode(*target),
        Request::Trade { own_id, with } => exchange::request(*own_id, *with)?,
        Request::AcceptTrade { own_id } => exchange::accept(*own_id)?,
        Request::CancelTrade { own_id } => exchange::cancel(*own_id)?,
        Request::ClickDoor { door_id, player_id } => doors::titanium_click(*door_id, *player_id),
        Request::PickUp { drop_id, player_id } => objects::titanium_pickup(*drop_id, *player_id),
        Request::CloseContainer(view) => view.close_packet(),
        Request::AnswerZoneOffer { offer, character } => offer.response(character)?,
        Request::Memorize { gem, spell_id } => spells::titanium_memorize(*gem, *spell_id),
        Request::Forget { gem, spell_id } => spells::titanium_forget(*gem, *spell_id),
        Request::Scribe { slot, spell_id } => spells::titanium_scribe(*slot, *spell_id),
        Request::DeleteSpell { slot } => spells::titanium_delete(*slot),
        Request::SwapSpells { from, to } => spells::titanium_swap(*from, *to),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titanium_requests_are_the_packets_the_codecs_build() {
        assert_eq!(titanium(&Request::Camp).unwrap(), command::titanium_camp());
        assert_eq!(
            titanium(&Request::Posture {
                spawn_id: 7,
                posture: Posture::Sitting,
            })
            .unwrap(),
            command::titanium_posture(7, Posture::Sitting).unwrap()
        );
        assert_eq!(
            titanium(&Request::Pet {
                command: PetCommand::Attack,
                target: Some(9),
            })
            .unwrap(),
            pets::command(PetCommand::Attack, Some(9))
        );
        assert_eq!(
            titanium(&Request::DropCorpse { corpse: None }).unwrap(),
            corpses::release(None).unwrap()
        );
        // What a packet cannot carry is refused, as the codec refuses it.
        assert!(titanium(&Request::Trade { own_id: 7, with: 7 }).is_err());
    }
}
