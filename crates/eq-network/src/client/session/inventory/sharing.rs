//! Coins the player hands to the group or to a banker: `/split`,
//! `/autosplit`, and the bank window's Change button.
//!
//! The official client's `/split` (0x46849f, 0x4a1a75; program read R6)
//! takes platinum, gold, silver and copper in that order, refuses outside a
//! group and when nothing or more than the purse holds of a kind is named,
//! each with its own line, says nothing while a banker's change is
//! unanswered, and takes the coins from the purse before it sends them.
//! `EQEmu`'s `Client::Handle_OP_Split` agrees that "the client removes the
//! money on its own". A split that goes through gives the player their own
//! share with a money update (`Group::SplitMoney`, `AddMoneyToPP` with its
//! update); one the server refuses leaves the server's purse untouched and
//! sends none. So a split no money update follows within [`SPLIT_ANSWER`]
//! leaves the purse unknown until one comes, rather than at a count the
//! server may not share.
//!
//! The Change button (0x4dd386) sends its 4 bytes with no check of the
//! coins, and counts the request as unanswered until the reply. A banker's
//! change answers with the purse and the bank as they now stand
//! (`Client::Handle_OP_BankerChange`), and the session holds coin moves
//! until it does. The server says nothing when no banker stands near
//! enough; the session asks only within its own, closer, reach, so a change
//! still unanswered after [`CHANGE_ANSWER`] is taken as refused, the coins
//! as they were.
//!
//! Both waits are inferred (recording checklist 6.410).
use super::{actor, purse_uncertain, tell_coins};
use crate::client::session::{
    actions,
    feature::{Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    money::Coin,
    request::Request,
    world::{Coins, WorldEvent},
};
use std::time::{Duration, Instant};

/// How long a split waits for the money update that gives the player their
/// share.
pub(super) const SPLIT_ANSWER: Duration = Duration::from_secs(3);
/// How long a banker's change waits for its answer.
pub(super) const CHANGE_ANSWER: Duration = Duration::from_secs(5);

/// The official client's line for a split outside a group (0x3028).
const NO_GROUP: u32 = eq_network_game::split::NO_GROUP;
/// Its line for a split of nothing (0x3029).
const NOTHING: u32 = 0x3029;
/// Its lines for naming more platinum, gold, silver or copper than the
/// purse holds (0x302c-0x302f).
const TOO_MUCH: [u32; 4] = [0x302c, 0x302d, 0x302e, 0x302f];

/// The split and the change in flight.
#[derive(Debug, Default)]
pub(super) struct Sharing {
    split: Option<Instant>,
    change: Option<Instant>,
}

/// Tells the host a banker's change went unanswered.
fn refused(session_id: u64, reason: &str, out: &mut Out<'_, '_>) -> Result<()> {
    out.log.send(ClientEvent::World(WorldEvent::CoinsRefused {
        session_id,
        reason: reason.into(),
        string_id: None,
    }))?;
    out.log
        .diagnostic(format!("Coin request refused: {reason}"))
}

impl Sharing {
    /// Whether a banker's change holds the coins.
    pub(super) const fn changing(&self) -> bool {
        self.change.is_some()
    }

    /// Splits coins from the purse with the group.
    pub(super) fn split(
        &mut self,
        command: &ClientCommand,
        coins: Coins,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if !world.grouped {
            return actions::refuse_officially(
                command,
                ("You are not in a group.", Some(NO_GROUP)),
                out.log,
            );
        }
        if self.changing() {
            // The official client says nothing while a change is unanswered.
            return out
                .log
                .diagnostic("Split withheld while a banker's change is unanswered".into());
        }
        if coins.is_empty() {
            return actions::refuse_officially(
                command,
                ("Name the coins to split.", Some(NOTHING)),
                out.log,
            );
        }
        let Some(mut purse) = world.coins.purse else {
            return actions::refuse(command, "Your purse is not known yet.", out.log);
        };
        for (coin, line) in Coin::ALL.into_iter().zip(TOO_MUCH) {
            let held = purse.of_mut(coin);
            let Some(left) = held.checked_sub(coins.of(coin)) else {
                return actions::refuse_officially(
                    command,
                    ("You do not have that many coins.", Some(line)),
                    out.log,
                );
            };
            *held = left;
        }
        let Ok(packet) = out.encode(&Request::SplitCoins(coins)) else {
            return actions::refuse(command, "That split cannot be sent.", out.log);
        };
        out.send(&packet)?;
        world.coins.0.purse = Some(purse);
        self.split = Some(Instant::now());
        tell_coins(world, true, out)
    }

    /// Turns sharing of looted coins on or off.
    pub(super) fn auto_split(
        command: &ClientCommand,
        on: bool,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match out.encode(&Request::AutoSplit(on)) {
            Ok(packet) => out.send(&packet),
            Err(_) => actions::refuse(command, "Your character is not ready yet.", out.log),
        }
    }

    /// Asks the banker nearest the player to change the coins.
    pub(super) fn change(
        &mut self,
        command: &ClientCommand,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if self.changing() {
            return actions::refuse(command, "Wait for the banker's change.", out.log);
        }
        if !actor(world).is_some_and(|actor| actor.bank_access) {
            return actions::refuse(command, "Stand near a banker to use the bank", out.log);
        }
        let packet = out.encode(&Request::ChangeCoins)?;
        out.send(&packet)?;
        self.change = Some(Instant::now());
        Ok(())
    }

    /// Follows the news that settles a split or a change: a money update, or
    /// the banker's answer.
    pub(super) fn heard(&mut self, event: &WorldEvent) {
        match event {
            WorldEvent::Coins(_) => self.split = None,
            WorldEvent::CoinsChanged(_) => self.change = None,
            _ => (),
        }
    }

    /// Ends the waits that ran out.
    pub(super) fn tick(
        &mut self,
        now: Instant,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let expired = |since: Option<Instant>, wait| {
            since.is_some_and(|since| now.saturating_duration_since(since) >= wait)
        };
        if expired(self.split, SPLIT_ANSWER) {
            self.split = None;
            purse_uncertain(world, out)?;
        }
        if expired(self.change, CHANGE_ANSWER) {
            self.change = None;
            refused(
                world.session_id,
                "The banker did not change your coins.",
                out,
            )?;
        }
        Ok(())
    }

    /// Starts the waits over, as at a new admission.
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Lets tests run a wait out.
    #[cfg(test)]
    pub(super) fn backdate(&mut self, by: Duration) {
        for since in [&mut self.split, &mut self.change].into_iter().flatten() {
            *since -= by;
        }
    }
}
