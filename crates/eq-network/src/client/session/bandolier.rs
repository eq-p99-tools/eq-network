//! The player's bandolier sets and potion belt on Titanium: the profile's
//! copy told at each admission (6.559), and the player's saving and
//! clearing of a set and setting and clearing of a belt slot (6.564), and
//! the swap to a set (6.566). No use from the belt is sent from here.
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
//! A swap moves up to eight items and the server tells the client none of
//! the moves (`Client::SetBandolier`). The session works out every one
//! ([`bandolier::swap()`]) and applies them as its prediction, settled as an
//! ordinary move is, or refuses and sends nothing.
//!
//! Every Titanium server tells the profile's copy; no server takes the
//! commands until their local live rows pass (6.564 rows 1 to 5; 6.566
//! rows 1 to 4 and read R49-45 d for the swap).
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    bandolier::{self, BandolierSet, BeltItem, BELT_SLOTS, SETS, WEAPON_SLOTS},
    inventory::{InventoryItem, InventoryUpdate},
    merchant::MerchantUpdate,
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

/// Which commands a server takes, each once its live rows pass.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Offered {
    /// Sets saved and cleared.
    pub(super) sets: bool,
    /// Swaps to a set.
    pub(super) swap: bool,
    /// Belt slots set and cleared.
    pub(super) belt: bool,
}

/// Tells the profile's sets and belt, and where offered, saves and clears
/// a set, swaps to one, and sets and clears a belt slot.
#[derive(Debug, Default)]
pub(super) struct Bandolier {
    known: Known,
    offered: Offered,
    sent: Option<Instant>,
    /// The merchant whose window is open, if any.
    merchant: Option<u16>,
    /// Parts of the profile heard before the other.
    staged_sets: Option<Box<Sets>>,
    staged_belt: Option<Box<Belt>>,
}

impl Bandolier {
    /// Tells the profile's copy and takes no command.
    pub(super) fn reading() -> Self {
        Self::default()
    }

    /// Takes the commands `offered` names, as their live rows pass.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "offered on stock EQEmu once rows 6.564 1-5 and 6.566 1-4 pass"
        )
    )]
    pub(super) fn sending(offered: Offered) -> Self {
        Self {
            offered,
            ..Self::default()
        }
    }

    /// Records what a message says of the sets and the belt, and shows
    /// the belt to the other features (6.568 B1).
    fn hear(&mut self, message: &Message, world: &mut World) {
        let before = std::mem::discriminant(&self.known);
        let changed = self.heard(message);
        if changed || std::mem::discriminant(&self.known) != before {
            self.show_belt(world);
        }
    }

    /// The belt the other features read: this feature's copy, or `None`.
    fn show_belt(&self, world: &mut World) {
        world.belt = match &self.known {
            Known::Read { belt, .. } => Some(belt.clone()),
            Known::Unread | Known::Unknown => None,
        };
    }

    /// Records what a message says; whether the copy changed.
    fn heard(&mut self, message: &Message) -> bool {
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
                return true;
            }
            _ => return false,
        }
        // The profile tells the sets, then the belt.
        if let (Some(sets), Some(belt)) = (&self.staged_sets, &self.staged_belt) {
            self.known = Known::Read {
                sets: sets.clone(),
                belt: belt.clone(),
            };
            self.staged_sets = None;
            self.staged_belt = None;
            return true;
        }
        false
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

    /// Sends the swap to `set` when every move it makes is known, and
    /// applies them as the session's prediction.
    fn swap(
        command: &ClientCommand,
        set: u8,
        sets: &Sets,
        merchant: Option<u16>,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<bool> {
        if let Err(error) = bandolier::swap_to(set) {
            return refused(command, &format!("Choose {error}."), out);
        }
        let Some(chosen) = &sets[usize::from(set)] else {
            return refused(command, "That set is empty.", out);
        };
        // What the official client checks first is read R49-45 b; until
        // then these wait (inferred). A cast, a move and a trader's window
        // are held by `actions::needs`.
        let busy = if world.control.stunned() || world.lifecycle.is_dead() {
            Some("You can't swap sets now.")
        } else if world.exchange.is_some() {
            Some("Finish the trade before swapping sets.")
        } else if merchant.is_some() {
            Some("Close the merchant's window before swapping sets.")
        } else if world.container.is_open() {
            Some("Close the container before swapping sets.")
        } else {
            None
        };
        if let Some(reason) = busy {
            return refused(command, reason, out);
        }
        let items = match bandolier::swap(chosen, &world.inventory) {
            Ok(items) => items,
            Err(reason) => return refused(command, reason, out),
        };
        out.request(&Request::SwapBandolier(set))?;
        // The server sends no item updates for the swap; its own moves are
        // settled by silence as an ordinary move's are.
        super::inventory::change(InventoryUpdate::Prediction(items), world, out)?;
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
        [
            (self.offered.sets, crate::world::Capability::Bandoliers),
            (self.offered.swap, crate::world::Capability::BandolierSwap),
            (self.offered.belt, crate::world::Capability::PotionBelt),
        ]
        .into_iter()
        .filter_map(|(offered, capability)| offered.then_some(capability))
        .collect()
    }

    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        self.hear(message, world);
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
                self.offered.sets
            }
            ClientCommand::SwapBandolier { .. } => self.offered.swap,
            ClientCommand::SetPotionBelt { .. } | ClientCommand::ClearPotionBelt { .. } => {
                self.offered.belt
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
        if let ClientCommand::SwapBandolier { set, .. } = command {
            if Self::swap(command, *set, sets, self.merchant, world, out)? {
                self.sent = Some(now);
            }
            return Ok(());
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
            if matches!(event, WorldEvent::PotionBelt(_)) {
                self.show_belt(world);
            }
            out.log.send(ClientEvent::World(event))?;
        }
        Ok(())
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        // The zone tells the host these itself once the player is in.
        self.hear(message, world);
        match message {
            Message::Event(WorldEvent::Merchant(MerchantUpdate::Opened {
                merchant_id,
                accepted: true,
                ..
            })) => self.merchant = Some(*merchant_id),
            Message::Event(WorldEvent::Merchant(MerchantUpdate::Closed)) => self.merchant = None,
            Message::Event(WorldEvent::Despawn(id)) if self.merchant == Some(*id) => {
                self.merchant = None;
            }
            _ => (),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
