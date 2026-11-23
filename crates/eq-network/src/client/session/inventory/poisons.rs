//! A Rogue's poison put on the weapon, by the slot it sits in
//! ([`eq_network_game::poisons`]). The servers decide the class, the skill,
//! the weapon and the level, answer every request and take the poison out
//! with their own inventory record, so nothing is taken out here.
//!
//! Which checks the Titanium program makes before it sends, and its lines,
//! are pending read R49-1 (recording checklist 6.493): here the gates are
//! the ones eating by hand has (a current inventory and the slot's moves
//! confirmed), then an item that is not there or not a poison, and one
//! request in flight at a time, since `EQEmu` takes a poison for each
//! request. All are refused in this library's words.
use super::{meals::known_slot, Out, World};
use crate::client::ClientEvent;
use anyhow::Result;
use eq_network_game::{
    inventory::InventorySlot,
    poisons::{PoisonAnswer, POISON_ITEM_TYPE},
    request::Request,
};

/// The poison request in flight, for this zone session.
#[derive(Default)]
pub(super) struct Poisons {
    /// The slot named by the request the server has not answered yet.
    pending: Option<InventorySlot>,
    /// Whether an answer to no request in flight was reported already.
    stray_told: bool,
}

impl Poisons {
    /// Sends the request for the poison in a slot, or says why not.
    pub(super) fn apply(
        &mut self,
        slot: InventorySlot,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<Option<&'static str>> {
        let inventory = &world.inventory;
        if !inventory.received() || inventory.stale() {
            return Ok(Some("Wait for a current inventory snapshot"));
        }
        if !known_slot(inventory, slot) {
            return Ok(Some("Wait for that item's moves to be confirmed"));
        }
        if self.pending.is_some() {
            return Ok(Some("Wait for the poison you are applying"));
        }
        let Some(item) = inventory.items().get(&slot) else {
            return Ok(Some("There is no item there"));
        };
        if item.rules.item_type != POISON_ITEM_TYPE {
            return Ok(Some("That is not a poison"));
        }
        out.request(&Request::ApplyPoison { slot })?;
        self.pending = Some(slot);
        Ok(None)
    }

    /// Hears the server's answer, which ends the request in flight. An answer
    /// naming another slot, or coming with none in flight, ends it too and
    /// is reported once a zone session.
    pub(super) fn answered(&mut self, answer: PoisonAnswer, out: &mut Out<'_, '_>) -> Result<()> {
        let matched = self.pending.take() == Some(answer.slot);
        if matched || self.stray_told {
            return Ok(());
        }
        self.stray_told = true;
        out.log.send(ClientEvent::Diagnostic(format!(
            "A poison answer for slot {} matched no request in flight",
            answer.slot.0
        )))
    }
}
