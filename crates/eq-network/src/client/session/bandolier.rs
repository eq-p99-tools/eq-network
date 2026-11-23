//! The player's bandolier sets and potion belt on Titanium: the profile's
//! copy told at each admission (6.559), and the player's saving and
//! clearing of a set and setting and clearing of a belt slot (6.564). No
//! swap and no use from the belt is sent from here.
//!
//! Stock `EQEmu` answers none of the four (`Client::CreateBandolier`,
//! `Client::RemoveBandolier`, `Client::Handle_OP_PotionBelt`), so after
//! each send the session changes its own copy the way the server changes
//! its copy, and tells the host the whole new set list or belt. That is the
//! session's mirror; the next admission's profile is the server's word.
//!
//! The server bounds neither the set nor the name, and stores any item
//! number its item table has without asking whether the player holds it.
//! The session sends only sets and belt slots 1 to 4, names of 1 to 31
//! plain letters, and a belt item the player wears or carries that is a
//! potion or made for the belt.
//!
//! Every Titanium server tells the profile's copy; no server takes the
//! four commands until their local live rows pass (6.564 rows 1 to 5).
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    bandolier::{self, BandolierSet, BeltItem, BELT_SLOTS, SETS, WEAPON_SLOTS},
    inventory::InventoryItem,
    message::Message,
    request::Request,
    world::WorldEvent,
};
use std::time::{Duration, Instant};

/// The least time between two sends (ours, inferred, 6.564 E4).
const FLOOR: Duration = Duration::from_millis(250);
/// Why nothing is sent while the profile's copy could not be read.
const UNKNOWN: &str = "Your bandolier could not be read; it is read again when you zone.";

type Sets = [Option<BandolierSet>; SETS];
type Belt = [Option<BeltItem>; BELT_SLOTS];

/// What the session knows of the sets and the belt at this admission.
#[derive(Debug, Default)]
enum Known {
    /// No profile has said.
    #[default]
    Unread,
    /// The profile's copy could not be read.
    Unknown,
    /// The profile's copy, changed by what the session sent since.
    Read { sets: Box<Sets>, belt: Box<Belt> },
}

/// Tells the profile's sets and belt, and where offered, saves and clears
/// a set and sets and clears a belt slot.
#[derive(Debug, Default)]
pub(super) struct Bandolier {
    known: Known,
    /// Sets saved and cleared.
    sets_offered: bool,
    /// Belt slots set and cleared.
    belt_offered: bool,
    sent: Option<Instant>,
    /// Parts of the profile heard before the other.
    staged_sets: Option<Box<Sets>>,
    staged_belt: Option<Box<Belt>>,
}

impl Bandolier {
    /// Tells the profile's copy and takes no command.
    pub(super) fn reading() -> Self {
        Self::default()
    }

    /// Takes the set commands, the belt commands, or both, as their live
    /// rows pass.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "offered on stock EQEmu once rows 6.564 1-5 pass")
    )]
    pub(super) fn sending(sets: bool, belt: bool) -> Self {
        Self {
            sets_offered: sets,
            belt_offered: belt,
            ..Self::default()
        }
    }

    /// Records what a message says of the sets and the belt.
    fn hear(&mut self, message: &Message) {
        match message {
            Message::Event(WorldEvent::Bandoliers(sets)) => {
                self.staged_sets = Some(sets.clone());
            }
            Message::Event(WorldEvent::PotionBelt(belt)) => {
                self.staged_belt = Some(belt.clone());
            }
            Message::Event(WorldEvent::BandoliersUnknown) => {
                self.known = Known::Unknown;
                self.staged_sets = None;
                self.staged_belt = None;
                return;
            }
            _ => return,
        }
        // The profile tells the sets, then the belt.
        if let (Some(sets), Some(belt)) = (&self.staged_sets, &self.staged_belt) {
            self.known = Known::Read {
                sets: sets.clone(),
                belt: belt.clone(),
            };
            self.staged_sets = None;
            self.staged_belt = None;
        }
    }

    fn save(
        command: &ClientCommand,
        set: u8,
        name: &str,
        sets: &mut Sets,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<bool> {
        if let Err(error) = bandolier::save(set, name) {
            return refused(command, &format!("Choose {error}."), out);
        }
        let inventory = &world.inventory;
        if !inventory.received()
            || inventory.stale()
            || inventory.predicted()
            || inventory.unresolved_predictions() > 0
            || !WEAPON_SLOTS
                .iter()
                .all(|slot| inventory.custody_known(*slot))
        {
            return refused(
                command,
                "Wait until your inventory is known to save a set.",
                out,
            );
        }
        out.request(&Request::SaveBandolier {
            set,
            name: name.to_owned(),
        })?;
        // The server copies the set's name, and the id, icon and name of
        // what is worn in each weapon slot, an empty slot as nothing
        // (`Client::CreateBandolier`).
        sets[usize::from(set)] = Some(BandolierSet {
            name: name.to_owned(),
            items: WEAPON_SLOTS.map(|slot| inventory.items().get(&slot).map(belt_item)),
        });
        Ok(true)
    }

    fn clear(
        command: &ClientCommand,
        set: u8,
        sets: &mut Sets,
        out: &mut Out<'_, '_>,
    ) -> Result<bool> {
        if let Err(error) = bandolier::clear(set) {
            return refused(command, &format!("Choose {error}."), out);
        }
        let held = &mut sets[usize::from(set)];
        if held.is_none() {
            return refused(command, "That set is already empty.", out);
        }
        out.request(&Request::ClearBandolier(set))?;
        // The server empties the name and each item's number and icon; the
        // item names it leaves are no item (`Client::RemoveBandolier`).
        *held = None;
        Ok(true)
    }

    fn set_belt(
        command: &ClientCommand,
        (slot, item_id): (u8, u32),
        belt: &mut Belt,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<bool> {
        if let Err(error) = bandolier::set_belt(slot, item_id) {
            return refused(command, &format!("Choose {error}."), out);
        }
        let inventory = &world.inventory;
        if !inventory.received() || inventory.stale() {
            return refused(
                command,
                "Wait until your inventory is known to fill the belt.",
                out,
            );
        }
        // Worn, or in the packs and their bags: never the cursor, nor
        // either bank (inferred, R49-45 c).
        let Some(item) = inventory.items().values().find(|item| {
            item.details.id == item_id
                && (item.slot.is_equipment() || item.slot.is_carried())
                && inventory.custody_known(item.slot)
        }) else {
            return refused(command, "You do not carry that item.", out);
        };
        if item.rules.item_type != bandolier::POTION && !item.rules.potion_belt {
            return refused(command, "Only a potion goes on the belt.", out);
        }
        out.request(&Request::SetPotionBelt { slot, item_id })?;
        // The server takes the id, icon and name from its own item record
        // (`Client::Handle_OP_PotionBelt`); the session from the one it holds.
        belt[usize::from(slot)] = Some(belt_item(item));
        Ok(true)
    }

    fn clear_belt(
        command: &ClientCommand,
        slot: u8,
        belt: &mut Belt,
        out: &mut Out<'_, '_>,
    ) -> Result<bool> {
        let Some(held) = belt.get_mut(usize::from(slot)) else {
            return refused(command, "Choose a belt slot from 1 to 4.", out);
        };
        let Some(item) = held else {
            return refused(command, "That belt slot is already empty.", out);
        };
        out.request(&Request::ClearPotionBelt {
            slot,
            item_id: item.item_id,
        })?;
        *held = None;
        Ok(true)
    }
}

/// What a set or the belt keeps of an item.
fn belt_item(item: &InventoryItem) -> BeltItem {
    BeltItem {
        item_id: item.details.id,
        icon: item.details.icon.unwrap_or(0),
        name: item.details.name.clone(),
    }
}

/// Refuses in the session's words; nothing was sent.
fn refused(command: &ClientCommand, reason: &str, out: &mut Out<'_, '_>) -> Result<bool> {
    actions::refuse(command, reason, out.log)?;
    Ok(false)
}

impl Feature for Bandolier {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        let mut offered = Vec::new();
        if self.sets_offered {
            offered.push(crate::world::Capability::Bandoliers);
        }
        if self.belt_offered {
            offered.push(crate::world::Capability::PotionBelt);
        }
        offered
    }

    fn admit(&mut self, message: &Message, _world: &mut World) -> Result<()> {
        self.hear(message);
        Ok(())
    }

    fn admitted(&mut self, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        match &self.known {
            Known::Unread => Ok(()),
            Known::Unknown => out
                .log
                .send(ClientEvent::World(WorldEvent::BandoliersUnknown)),
            Known::Read { sets, belt } => {
                out.log
                    .send(ClientEvent::World(WorldEvent::Bandoliers(sets.clone())))?;
                out.log
                    .send(ClientEvent::World(WorldEvent::PotionBelt(belt.clone())))
            }
        }
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        match command {
            ClientCommand::SaveBandolier { .. } | ClientCommand::ClearBandolier { .. } => {
                self.sets_offered
            }
            ClientCommand::SetPotionBelt { .. } | ClientCommand::ClearPotionBelt { .. } => {
                self.belt_offered
            }
            _ => false,
        }
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let now = Instant::now();
        let Known::Read { sets, belt } = &mut self.known else {
            return actions::refuse(command, UNKNOWN, out.log);
        };
        if self
            .sent
            .is_some_and(|sent| now.saturating_duration_since(sent) < FLOOR)
        {
            return actions::refuse(command, "You just changed your bandolier.", out.log);
        }
        let (sent, event) = match command {
            ClientCommand::SaveBandolier { set, name, .. } => (
                Self::save(command, *set, name, sets, world, out)?,
                WorldEvent::Bandoliers(sets.clone()),
            ),
            ClientCommand::ClearBandolier { set, .. } => (
                Self::clear(command, *set, sets, out)?,
                WorldEvent::Bandoliers(sets.clone()),
            ),
            ClientCommand::SetPotionBelt { slot, item_id, .. } => (
                Self::set_belt(command, (*slot, *item_id), belt, world, out)?,
                WorldEvent::PotionBelt(belt.clone()),
            ),
            ClientCommand::ClearPotionBelt { slot, .. } => (
                Self::clear_belt(command, *slot, belt, out)?,
                WorldEvent::PotionBelt(belt.clone()),
            ),
            _ => return Ok(()),
        };
        if sent {
            self.sent = Some(now);
            out.log.send(ClientEvent::World(event))?;
        }
        Ok(())
    }

    fn observe(
        &mut self,
        message: &Message,
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        // The zone tells the host these itself once the player is in.
        self.hear(message);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
