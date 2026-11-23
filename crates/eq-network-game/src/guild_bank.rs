//! The guild bank as the server tells of it, read only (checklist 6.563).
//!
//! Stock `EQEmu` keeps the bank only in the guild hall, and at zone-in it
//! sends a client before RoF the bank's slots by its inventory type sizes,
//! which are 0 for Titanium (`GuildBankManager::SendGuildBank`,
//! `common/eq_limits.cpp`). So a Titanium client is never sent the bank's
//! list. What it learns is one slot at a time, as an item arrives, leaves or
//! changes there, which the server sends every guild member in the zone
//! (`GuildBankManager::SendGuildBankItemUpdate`,
//! `EntityList::QueueClientsGuildBankItemUpdate`). A host's view is the
//! slots seen since it entered the guild hall, and it is never complete.
//!
//! The server also answers the client's own guild bank requests with a bare
//! acknowledgement (`Client::GuildBankAck`), or a deposit's own answer
//! (`Client::GuildBankDepositAck`). Neither names the request it answers.
//! Nothing here sends a request yet, and nothing acts on an answer.
//!
//! The player's own banker flag arrives in the profile
//! ([`titanium_profile`](crate::guild_bank::titanium_profile)) and, as it
//! changes in the guild hall, in `OP_SetGuildRank` ([`crate::guild`]).
//!
//! Layout reference: `EQEmu`'s `GuildBankItemUpdate_Struct`,
//! `GuildBankAck_Struct`, `GuildBankDepositAck_Struct` and
//! `GuildBankClear_Struct` (`common/eq_packet_structs.h`). Titanium has no
//! translator for the opcode, so these go on the wire as they are.

use crate::world::until_nul;
use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use std::fmt;

/// `OP_GuildBank` (Titanium).
pub const OPCODE: u16 = 0xb4ab;

/// The action words the server sends (`GuildBankAction`, `zone/guild_mgr.h`).
const CLEAR: u32 = 0;
const ITEM_UPDATE: u32 = 1;
const DEPOSIT: u32 = 5;
const ACKNOWLEDGE: u32 = 10;

/// A name or text field's length.
const TEXT: usize = 64;
/// An item update's length.
const ITEM_UPDATE_LENGTH: usize = 228;

/// Where the Titanium profile keeps the player's banker flag: a 32-bit
/// `guildbanker` (`EQEmu`'s Titanium `PlayerProfile_Struct`), which `EQEmu`
/// sets only in the guild hall.
const PROFILE_BANKER: usize = 13056;

/// The bank's two areas: where members deposit, and where a banker keeps
/// what they promote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum GuildBankArea {
    /// Area 0, where deposits arrive.
    Deposit,
    /// Area 1, the bank proper.
    Main,
}

/// Who may take an item, as a banker set it (`GuildBankPermissions`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum GuildBankPermission {
    /// 0: bankers only.
    BankerOnly,
    /// 1: the one member the entry names.
    SingleMember,
    /// 2: anyone who can use it.
    PublicIfUsable,
    /// 3: anyone.
    Public,
    /// Another number: the server stores whatever a banker's request names.
    Unknown(u32),
}

impl GuildBankPermission {
    const fn from_wire(value: u32) -> Self {
        match value {
            0 => Self::BankerOnly,
            1 => Self::SingleMember,
            2 => Self::PublicIfUsable,
            3 => Self::Public,
            other => Self::Unknown(other),
        }
    }
}

/// An item in a slot of the bank, as the server last told of it.
///
/// `donor` and `permitted` are players' names. [`fmt::Debug`] leaves them
/// out, so a log of the update never carries them.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct GuildBankEntry {
    /// The area.
    pub area: GuildBankArea,
    /// The slot in the area.
    pub slot: u16,
    /// The item's number.
    pub item_id: u32,
    /// The item's icon.
    pub icon_id: u32,
    /// How many: a stack's count, or 1 for an item with charges, which the
    /// server sends as 1.
    pub quantity: u32,
    /// Who may take it.
    pub permission: GuildBankPermission,
    /// The member a [`GuildBankPermission::SingleMember`] entry names, as
    /// the server spells them; empty otherwise.
    pub permitted: String,
    /// Who deposited it, as the server spells them.
    pub donor: String,
    /// The item's name.
    pub name: String,
    /// Whether the item stacks (`allow_merge`).
    pub merges: bool,
    /// The server's `is_useable`, worked out for the player whose action
    /// caused the update, who may not be this player. Nothing should act on
    /// it for this player.
    pub usable_for_actor: bool,
}

impl fmt::Debug for GuildBankEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuildBankEntry")
            .field("area", &self.area)
            .field("slot", &self.slot)
            .field("item_id", &self.item_id)
            .field("icon_id", &self.icon_id)
            .field("quantity", &self.quantity)
            .field("permission", &self.permission)
            .field("name", &self.name)
            .field("merges", &self.merges)
            .field("usable_for_actor", &self.usable_for_actor)
            .finish_non_exhaustive()
    }
}

/// News of the guild bank.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum GuildBankUpdate {
    /// A slot holds this item now, replacing what it held before.
    Shown(GuildBankEntry),
    /// A slot is empty now. It names the item that was there.
    Removed {
        /// The area.
        area: GuildBankArea,
        /// The slot in the area.
        slot: u16,
        /// The item that left.
        item_id: u32,
    },
    /// The server's answer to a deposit: whether it took the cursor item.
    /// On `true` the server has deleted that item without telling the
    /// client otherwise.
    DepositAnswered {
        /// Whether the bank took it.
        taken: bool,
    },
    /// A bare acknowledgement of a guild bank request, which names nothing.
    Acknowledged,
    /// The bank's areas cleared to these sizes. Nothing on stock `EQEmu`
    /// sends it; read as its layout says.
    Cleared {
        /// The deposit area's slots.
        deposit_slots: u32,
        /// The main area's slots.
        main_slots: u32,
    },
    /// The player's own banker flag, from the profile.
    Banker(bool),
}

/// Reads an `OP_GuildBank` the server sent; None for another opcode.
///
/// # Errors
/// Rejects an action this doesn't know, a known action at another length,
/// an area other than 0 or 1, a `display` or `Fail` other than 0 or 1, and
/// a text field that doesn't end within its 64 bytes.
pub fn titanium(opcode: u16, body: &[u8]) -> Result<Option<GuildBankUpdate>> {
    if opcode != OPCODE {
        return Ok(None);
    }
    let action = word(body, 0).context("guild bank message without its action")?;
    let update = match action {
        ITEM_UPDATE => {
            sized(body, ITEM_UPDATE_LENGTH, "item update")?;
            item_update(body)?
        }
        DEPOSIT => {
            sized(body, 12, "deposit answer")?;
            GuildBankUpdate::DepositAnswered {
                taken: !flag(word(body, 8)?, "deposit answer's fail word")?,
            }
        }
        ACKNOWLEDGE => {
            sized(body, 8, "acknowledgement")?;
            GuildBankUpdate::Acknowledged
        }
        CLEAR => {
            sized(body, 16, "clear")?;
            GuildBankUpdate::Cleared {
                deposit_slots: word(body, 8)?,
                main_slots: word(body, 12)?,
            }
        }
        other => bail!("unknown guild bank action {other}"),
    };
    Ok(Some(update))
}

/// The player's banker flag in the Titanium profile.
///
/// # Errors
/// Rejects a profile too short to say, and a word other than 0 or 1.
pub fn titanium_profile(profile: &[u8]) -> Result<GuildBankUpdate> {
    let value = word(profile, PROFILE_BANKER).context("profile too short for the guild banker")?;
    Ok(GuildBankUpdate::Banker(flag(
        value,
        "profile's guild banker word",
    )?))
}

fn item_update(body: &[u8]) -> Result<GuildBankUpdate> {
    let slot = short(body, 8);
    let area = match short(body, 10) {
        0 => GuildBankArea::Deposit,
        1 => GuildBankArea::Main,
        other => bail!("unknown guild bank area {other}"),
    };
    let item_id = word(body, 16)?;
    if !flag(word(body, 12)?, "item update's display word")? {
        return Ok(GuildBankUpdate::Removed {
            area,
            slot,
            item_id,
        });
    }
    Ok(GuildBankUpdate::Shown(GuildBankEntry {
        area,
        slot,
        item_id,
        icon_id: word(body, 20)?,
        quantity: word(body, 24)?,
        permission: GuildBankPermission::from_wire(word(body, 28)?),
        merges: body[32] != 0,
        usable_for_actor: body[33] != 0,
        name: text(body, 34, "item name")?,
        donor: text(body, 34 + TEXT, "donor")?,
        permitted: text(body, 34 + 2 * TEXT, "permitted member")?,
    }))
}

fn sized(body: &[u8], length: usize, what: &str) -> Result<()> {
    ensure!(body.len() == length, "invalid guild bank {what} length");
    Ok(())
}

/// A 0 or 1 word as a flag.
fn flag(value: u32, what: &str) -> Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        other => bail!("guild bank {what} is {other}, not 0 or 1"),
    }
}

fn word(body: &[u8], at: usize) -> Result<u32> {
    let bytes = body
        .get(at..at + 4)
        .context("guild bank message too short")?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn short(body: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([body[at], body[at + 1]])
}

/// The text in the 64 bytes at `at`, which a NUL ends within them.
fn text(body: &[u8], at: usize, what: &str) -> Result<String> {
    let field = &body[at..at + TEXT];
    ensure!(field.contains(&0), "guild bank {what} without its end");
    Ok(String::from_utf8_lossy(until_nul(field)).into_owned())
}

#[cfg(test)]
mod tests;
