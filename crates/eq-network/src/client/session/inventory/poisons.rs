//! A Rogue's poison put on the weapon, by the slot it sits in
//! ([`eq_network_game::poisons`]), as the Titanium program does it (read
//! R49-1, recording checklist 6.493): putting a poison on is a timed sit.
//!
//! - A right-click on a poison in a top-level slot (the cursor included)
//!   starts it. With no Apply Poison skill the program says 0x305f and stops;
//!   a standing player is sat down (0x483556), and one who is still not
//!   sitting is told 0x305d. One already starting or in flight is ignored
//!   without a word. Otherwise the wait starts, with 0x305e.
//! - The request goes out when the wait ends (the character tick,
//!   0x422617): effect 234's total in ms, the sum over every active buff and
//!   every usable AA, in place of 10 s when it is not 0 (read R49-1f). This
//!   session does not sum effect 234 yet and waits the 10 s always: the same
//!   for a player with no such effect, slower where a total is below 10 s.
//!   A total above 10 s, or a negative one (with which the program never
//!   sends), would make this session the faster: a known departure until
//!   the follow-up sums it.
//! - Any posture change before then cancels it, with 0x30d1. Leaving the
//!   zone drops it without a line, as the program's zoning clears it: this
//!   state ends with the zone session. The posture
//!   state counts its changes, so a stand and re-sit between two ticks
//!   cancels too. A change the server forces without a posture report is
//!   not seen (inferred: the servers report the stances they force). The
//!   program prints 0x30d1 in colour 0x111 (read R49-1f).
//! - The answer ends the hold; which line it brings is the host's. The
//!   program's hold is its general wait for a server answer, which other
//!   answers clear too (read R49-5 asks); this one is the poison's own.
//!
//! The servers decide the class, the weapon and the level, answer every
//! request and take the poison out with their own inventory record, so
//! nothing is taken out here. The Mac-era program is not read: its clients
//! take the same flow, which can only be slower (inferred). The gates
//! before the program's own (a current inventory, the slot's moves
//! confirmed, and the player's skills known) are this library's, in its
//! words.
use super::{meals::known_slot, Out, World};
use crate::client::ClientEvent;
use anyhow::Result;
use eq_network_game::{
    command::Posture,
    inventory::InventorySlot,
    poisons::{PoisonAnswer, POISON_ITEM_TYPE},
    request::Request,
    world::{PostureState, WorldEvent},
};
use std::time::{Duration, Instant};

/// How long the program waits, seated, before it sends when effect 234's
/// total is 0 (read R49-1 c, R49-1f). The total is not summed here yet, so
/// this wait is always used: see the module notes for where that departs.
pub(super) const WAIT: Duration = Duration::from_secs(10);
/// The Apply Poison skill, the program's only class test (read R49-1 b).
const APPLY_POISON_SKILL: usize = 6;
/// "You can't" lines and the start, as `eqstr_us.txt` string IDs (read
/// R49-1 b, d).
const NO_SKILL: u32 = 0x305f;
const NOT_SITTING: u32 = 0x305d;
const STARTED: u32 = 0x305e;
const INTERRUPTED: u32 = 0x30d1;

/// Where a poison stands, for this zone session.
#[derive(Default)]
pub(super) struct Poisons {
    /// The poison waiting to go out, when its wait began, and the posture
    /// change count then (after the session's own sit).
    starting: Option<(InventorySlot, Instant, u64)>,
    /// The slot named by the request the server has not answered yet.
    pending: Option<InventorySlot>,
    /// Whether an answer to no request in flight was reported already.
    stray_told: bool,
}

impl Poisons {
    /// Starts putting on the poison in a slot, or says why not: a refusal in
    /// this library's words, and the program's string for it where it has
    /// one. One already starting or in flight is ignored, as the program
    /// ignores it.
    pub(super) fn apply(
        &mut self,
        slot: InventorySlot,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<Option<(&'static str, Option<u32>)>> {
        // The program cancels in its posture routine, at the change itself:
        // a stand since the last tick has already ended the wait.
        self.cancel_if_moved(world, out)?;
        let inventory = &world.inventory;
        if !inventory.received() || inventory.stale() {
            return Ok(Some(("Wait for a current inventory snapshot", None)));
        }
        if !known_slot(inventory, slot) {
            return Ok(Some(("Wait for that item's moves to be confirmed", None)));
        }
        let Some(item) = inventory.items().get(&slot) else {
            return Ok(Some(("There is no item there", None)));
        };
        if item.rules.item_type != POISON_ITEM_TYPE {
            return Ok(Some(("That is not a poison", None)));
        }
        // The program's item use passes only the slots below 0x20 (read
        // R49-1 a): what a bag holds is not used this way.
        if !(0..0x20).contains(&slot.0) {
            return Ok(Some(("Take the poison out of the bag to apply it", None)));
        }
        // Skills not yet reported are not the program's 0: they are refused
        // in this library's words.
        let Some(skill) = world
            .player
            .as_ref()
            .and_then(|player| player.skills.as_deref())
            .and_then(|skills| skills.get(APPLY_POISON_SKILL).copied())
        else {
            return Ok(Some(("Wait for your skills to be known", None)));
        };
        if skill == 0 {
            return Ok(Some((
                "You don't know how to apply poisons",
                Some(NO_SKILL),
            )));
        }
        let Some(spawn_id) = world.own_spawn else {
            return Ok(Some(("Not in the zone yet", None)));
        };
        if matches!(world.posture.current(), None | Some(PostureState::Standing)) {
            world.posture.set(spawn_id, Posture::Sitting, out)?;
        }
        if world.posture.current() != Some(PostureState::Sitting) {
            return Ok(Some((
                "You must be sitting to apply a poison",
                Some(NOT_SITTING),
            )));
        }
        // Only after the sit and its refusal, as the program's starter
        // orders them (read R49-1 b).
        if self.starting.is_some() || self.pending.is_some() {
            return out
                .log
                .diagnostic(
                    "A poison click while one is under way was ignored, as the program ignores it"
                        .into(),
                )
                .map(|()| None);
        }
        self.starting = Some((slot, Instant::now(), world.posture.changes()));
        out.log
            .send(ClientEvent::World(WorldEvent::PoisonStarted {
                session_id: world.session_id,
                slot,
                string_id: STARTED,
            }))
            .map(|()| None)
    }

    /// Sends a poison whose wait is over, or cancels one whose player left
    /// the seat first.
    pub(super) fn tick(
        &mut self,
        now: Instant,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.cancel_if_moved(world, out)?;
        let Some((slot, since, _)) = self.starting else {
            return Ok(());
        };
        if now.saturating_duration_since(since) <= WAIT {
            return Ok(());
        }
        self.starting = None;
        out.request(&Request::ApplyPoison { slot })?;
        self.pending = Some(slot);
        Ok(())
    }

    /// Ends a wait whose player changed stance since it began, with the
    /// program's 0x30d1.
    fn cancel_if_moved(&mut self, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        let Some((_, _, changes)) = self.starting else {
            return Ok(());
        };
        if world.posture.changes() == changes
            && world.posture.current() == Some(PostureState::Sitting)
        {
            return Ok(());
        }
        self.starting = None;
        out.log
            .send(ClientEvent::World(WorldEvent::PoisonInterrupted {
                session_id: world.session_id,
                string_id: INTERRUPTED,
            }))
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

    /// Moves the start of a wait under way back by `by`, for tests that
    /// cannot wait it out.
    #[cfg(test)]
    pub(super) fn backdate(&mut self, by: Duration) {
        if let Some((_, since, _)) = &mut self.starting {
            *since = since.checked_sub(by).unwrap_or(*since);
        }
    }
}
