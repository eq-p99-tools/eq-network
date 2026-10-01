//! Typed client actions and their dialect-specific packet encoding.

use crate::{chat, chat::OutboundChat, GameDialect};
use anyhow::Result;

/// Player posture values used by Titanium appearance updates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Posture {
    /// Standing and ready to move.
    Standing,
    /// Sitting for recovery or spellbook use.
    Sitting,
    /// Ducking, which also interrupts casting.
    Ducking,
}

impl Posture {
    /// Titanium appearance parameter for this persistent stance.
    const fn titanium_value(self) -> u32 {
        match self {
            Self::Standing => 100,
            Self::Sitting => 110,
            Self::Ducking => 111,
        }
    }
}

/// An action requested by an application after zone admission.
///
/// Future movement, zoning, inventory, group, and character actions belong
/// here so application code never needs to carry raw opcodes or packet bytes.
///
/// Exhaustive on purpose: a new command is a decision for every table that
/// says what a command needs, which feature carries it out and how it is
/// refused, and each of those names it.
#[derive(Clone, Debug, PartialEq)]
pub enum GameCommand {
    /// Enter one occupied slot from the current world-server character list.
    SelectCharacter {
        /// Identity supplied with the character list.
        selection_id: u64,
        /// Server slot to enter.
        slot: u8,
    },
    /// Create a character from the current character-selection screen.
    CreateCharacter {
        /// Identity supplied with the character list.
        selection_id: u64,
        /// Requested name, race, class, deity, start zone and stats.
        character: crate::creation::NewCharacter,
    },
    /// Exchange two book slots after checking both expected contents.
    SwapSpell {
        /// Current zone admission.
        session_id: u64,
        /// Populated source book slot.
        from: u16,
        /// Destination book slot, possibly empty.
        to: u16,
        /// Expected source spell.
        from_spell: u32,
        /// Expected destination spell or empty slot.
        to_spell: Option<u32>,
        /// Queued requests expire.
        created: std::time::Instant,
    },
    /// Activate the click effect on a current carried or equipped item.
    UseItem(crate::inventory::ItemUse),
    /// Use an ordinary nearby door; no lockpicking or cursor-item action is implied.
    ClickDoor {
        /// Current zone admission.
        session_id: u64,
        /// ID from this zone's server-provided door table.
        door_id: u8,
        /// Requests expire rather than surviving stalls or reconnects.
        created: std::time::Instant,
    },
    /// Pick up a nearby item lying on the ground; it arrives on the cursor.
    PickUp {
        /// Current zone admission.
        session_id: u64,
        /// Object from this zone's server-provided table.
        drop_id: u32,
        /// Requests expire rather than surviving stalls or reconnects.
        created: std::time::Instant,
    },
    /// Request a transfer after entering a boundary in the local zone assets.
    CrossZoneLine {
        /// Current zone admission.
        session_id: u64,
        /// Asset-derived route; a reference is resolved by the network session.
        destination: crate::zoning::ZoneLineDestination,
        /// Most recently accepted position at the detected boundary.
        position: crate::world::Position,
        /// Stale crossings must not survive pauses or reconnects.
        created: std::time::Instant,
    },
    /// Scribe the scroll held on the cursor into an empty book slot.
    ScribeSpell {
        /// Current zone admission.
        session_id: u64,
        /// Inventory revision observed when selecting the scroll.
        revision: u64,
        /// Empty spellbook slot, zero through 399.
        slot: u16,
        /// Spell taught by the current cursor scroll.
        spell_id: u32,
        /// Request creation time.
        created: std::time::Instant,
    },
    /// Request deletion of a known book entry; applications should confirm this with the user.
    DeleteSpell {
        /// Current zone admission.
        session_id: u64,
        /// Book slot to delete, zero through 399.
        slot: u16,
        /// Expected spell; changed selections must not delete a replacement.
        spell_id: u32,
        /// Request creation time; queued deletions expire.
        created: std::time::Instant,
    },
    /// Clear a memorized gem, preserving the spell in the book.
    ForgetSpell {
        /// Current zone admission.
        session_id: u64,
        /// Gem to clear, zero through seven.
        gem: u8,
        /// Spell expected in that gem; stale selections are rejected.
        spell_id: u32,
        /// Request creation time.
        created: std::time::Instant,
    },
    /// Begin timed memorization from the admitted character's spellbook.
    MemorizeSpell {
        /// Current zone admission.
        session_id: u64,
        /// Destination gem, zero through seven.
        gem: u8,
        /// Spell already present in the server-provided book.
        spell_id: u32,
        /// Request creation time; queued actions expire.
        created: std::time::Instant,
    },
    /// Cast a memorized spell; the server decides completion and resource use.
    CastSpell {
        /// Current zone admission.
        session_id: u64,
        /// Zero-based memorized slot, 0 through 7.
        gem: u8,
        /// Spell currently occupying the slot.
        spell_id: u32,
        /// Explicit target, scoped to this zone.
        target_id: u16,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Change the active player's posture.
    SetPosture {
        /// Current zone admission.
        session_id: u64,
        /// Own spawn identifier from this admission.
        spawn_id: u16,
        /// Requested posture.
        posture: Posture,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Move an inventory item using the admitted session state.
    MoveInventory(crate::inventory::InventoryMove),
    /// Send one chat message.
    SendChat(OutboundChat),
    /// Inspect a preserved item link in the active zone session.
    InspectItem {
        /// Connection identifier from zone admission.
        session_id: u64,
        /// Exact hexadecimal link header received in chat.
        link_body: String,
    },
    /// Ask the server how a visible entity regards this character.
    Consider {
        /// Current zone admission.
        session_id: u64,
        /// Own spawn identifier from this admission.
        own_id: u16,
        /// Visible entity to consider.
        target_id: u16,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Sit-and-wait logout to character selection, abandoned by standing or moving.
    Camp {
        /// Current zone admission.
        session_id: u64,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Open a nearby corpse for looting.
    Loot {
        /// Current zone admission.
        session_id: u64,
        /// Corpse entity.
        corpse_id: u16,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Take one item from the open corpse.
    LootItem {
        /// Current zone admission.
        session_id: u64,
        /// Corpse entity.
        corpse_id: u16,
        /// Own spawn identifier from this admission.
        own_id: u16,
        /// Corpse slot listed by the server.
        slot: u16,
        /// Place directly into the inventory instead of on the cursor.
        auto: bool,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Close the loot window.
    EndLoot {
        /// Current zone admission.
        session_id: u64,
        /// Corpse entity.
        corpse_id: u16,
    },
    /// Open or close a merchant window.
    Shop {
        /// Current zone admission.
        session_id: u64,
        /// Merchant entity.
        merchant_id: u16,
        /// Own spawn identifier from this admission.
        own_id: u16,
        /// True opens, false closes.
        open: bool,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Buy units from a merchant slot; the server sets the price.
    Buy {
        /// Current zone admission.
        session_id: u64,
        /// Merchant entity.
        merchant_id: u16,
        /// Own spawn identifier from this admission.
        own_id: u16,
        /// Merchant list slot.
        slot: u32,
        /// Units to buy.
        quantity: u32,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Sell units from an inventory slot; the server sets the price.
    Sell {
        /// Current zone admission.
        session_id: u64,
        /// Merchant entity.
        merchant_id: u16,
        /// Inventory slot.
        slot: i32,
        /// Units to sell.
        quantity: u32,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Ask a nearby character to trade; their answer opens the window: the
    /// give window for an NPC, the trade window for a player.
    OfferTrade {
        /// Current zone admission.
        session_id: u64,
        /// The character to trade with.
        with_id: u16,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Click Give or Trade in the open window.
    AcceptTrade {
        /// Current zone admission.
        session_id: u64,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Close the give or trade window; what it held comes back.
    CancelTrade {
        /// Current zone admission.
        session_id: u64,
    },
    /// Move coins between the purse, the cursor, the bank and a trade window.
    MoveCoins {
        /// Current zone admission.
        session_id: u64,
        /// The move.
        transfer: crate::money::CoinTransfer,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Announce a jump the client is simulating; only sessions that accept falls
    /// send it.
    Jump {
        /// Current zone admission.
        session_id: u64,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Eat or drink the item in a slot by hand. The session eats and drinks
    /// on its own when the player turns hungry or thirsty.
    Consume {
        /// Current zone admission.
        session_id: u64,
        /// The item's slot.
        slot: crate::inventory::InventorySlot,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Use an ability: a strike or taunt at the server-side target, or one
    /// on the player.
    UseAbility {
        /// Current zone admission.
        session_id: u64,
        /// The ability.
        ability: crate::abilities::Ability,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Ask the world who is online: `/who all`.
    WhoAll {
        /// Current zone admission.
        session_id: u64,
        /// Which players to ask about.
        filter: crate::who::WhoFilter,
    },
    /// Start or stop melee auto-attack against the current server-side target.
    AutoAttack {
        /// Current zone admission.
        session_id: u64,
        /// Whether auto-attack should be on.
        enabled: bool,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Select or clear a target in the current zone session, without attacking.
    SelectTarget {
        /// Connection identifier supplied by zone admission.
        session_id: u64,
        /// None clears the target; zero is not a valid selected spawn.
        spawn_id: Option<u16>,
    },
    /// Install independently measured motion values for this admission.
    ConfigureMotion {
        /// Current zone-admission identifier.
        session_id: u64,
        /// World-to-wire conversion measured for the current movement mode.
        calibration: crate::movement::MotionCalibration,
        /// Creation time prevents queued configuration from surviving a reset.
        created: std::time::Instant,
    },
    /// Propose fresh motion for the current zone session.
    Move(crate::movement::MovementRequest),
}

impl GameCommand {
    /// The zone admission the command was made for, if it names one.
    #[must_use]
    pub const fn session_id(&self) -> Option<u64> {
        match self {
            Self::SelectCharacter { .. } | Self::CreateCharacter { .. } | Self::SendChat(_) => None,
            Self::UseItem(request) => Some(request.session_id),
            Self::MoveInventory(request) => Some(request.session_id),
            Self::Move(request) => Some(request.session_id),
            Self::SwapSpell { session_id, .. }
            | Self::ClickDoor { session_id, .. }
            | Self::PickUp { session_id, .. }
            | Self::CrossZoneLine { session_id, .. }
            | Self::ScribeSpell { session_id, .. }
            | Self::DeleteSpell { session_id, .. }
            | Self::ForgetSpell { session_id, .. }
            | Self::MemorizeSpell { session_id, .. }
            | Self::CastSpell { session_id, .. }
            | Self::SetPosture { session_id, .. }
            | Self::InspectItem { session_id, .. }
            | Self::Consider { session_id, .. }
            | Self::Camp { session_id, .. }
            | Self::Loot { session_id, .. }
            | Self::LootItem { session_id, .. }
            | Self::EndLoot { session_id, .. }
            | Self::Shop { session_id, .. }
            | Self::Buy { session_id, .. }
            | Self::Sell { session_id, .. }
            | Self::OfferTrade { session_id, .. }
            | Self::AcceptTrade { session_id, .. }
            | Self::CancelTrade { session_id }
            | Self::MoveCoins { session_id, .. }
            | Self::Jump { session_id, .. }
            | Self::AutoAttack { session_id, .. }
            | Self::Consume { session_id, .. }
            | Self::UseAbility { session_id, .. }
            | Self::WhoAll { session_id, .. }
            | Self::SelectTarget { session_id, .. }
            | Self::ConfigureMotion { session_id, .. } => Some(*session_id),
        }
    }

    /// What the zone session must let the player do for this command; None
    /// for the world server's commands, which come before any zone session.
    /// A front end greys out a command whose capability the session lacks.
    #[must_use]
    pub const fn capability(&self) -> Option<crate::world::Capability> {
        use crate::world::Capability;
        Some(match self {
            Self::SelectCharacter { .. } | Self::CreateCharacter { .. } => return None,
            Self::SwapSpell { .. }
            | Self::ScribeSpell { .. }
            | Self::DeleteSpell { .. }
            | Self::ForgetSpell { .. }
            | Self::MemorizeSpell { .. } => Capability::Spellbook,
            // An item's click effect is a cast.
            Self::CastSpell { .. } | Self::UseItem(_) => Capability::Casting,
            Self::ClickDoor { .. } => Capability::Doors,
            Self::PickUp { .. } => Capability::GroundItems,
            Self::CrossZoneLine { .. } => Capability::Zoning,
            Self::SetPosture { .. } | Self::Move(_) | Self::ConfigureMotion { .. } => {
                Capability::Moving
            }
            // Only a server that takes falls from the client lets the player jump.
            Self::Jump { .. } => Capability::Falling,
            Self::MoveInventory(_) | Self::Consume { .. } => Capability::Inventory,
            Self::SendChat(_) | Self::InspectItem { .. } => Capability::Talking,
            Self::Consider { .. } | Self::AutoAttack { .. } => Capability::Combat,
            Self::Camp { .. } => Capability::Camping,
            Self::Loot { .. } | Self::LootItem { .. } | Self::EndLoot { .. } => Capability::Looting,
            Self::Shop { .. } | Self::Buy { .. } | Self::Sell { .. } => Capability::Trading,
            Self::OfferTrade { .. } | Self::AcceptTrade { .. } | Self::CancelTrade { .. } => {
                Capability::Giving
            }
            // Coins go into a trade window only where the player can give.
            Self::MoveCoins { transfer, .. } => {
                if matches!(transfer.from, crate::money::CoinPlace::Trade)
                    || matches!(transfer.to, crate::money::CoinPlace::Trade)
                {
                    Capability::Giving
                } else {
                    Capability::Inventory
                }
            }
            Self::SelectTarget { .. } => Capability::Targeting,
            Self::UseAbility { .. } => Capability::Abilities,
            Self::WhoAll { .. } => Capability::Who,
        })
    }

    /// When the host made the command, if it says; commands that may wait,
    /// such as closing a loot window, do not.
    #[must_use]
    pub const fn created(&self) -> Option<std::time::Instant> {
        match self {
            Self::SelectCharacter { .. }
            | Self::CreateCharacter { .. }
            | Self::SendChat(_)
            | Self::InspectItem { .. }
            | Self::EndLoot { .. }
            | Self::CancelTrade { .. }
            | Self::WhoAll { .. }
            | Self::SelectTarget { .. } => None,
            Self::UseItem(request) => Some(request.created),
            Self::MoveInventory(request) => Some(request.created),
            Self::Move(request) => Some(request.created),
            Self::SwapSpell { created, .. }
            | Self::ClickDoor { created, .. }
            | Self::PickUp { created, .. }
            | Self::CrossZoneLine { created, .. }
            | Self::ScribeSpell { created, .. }
            | Self::DeleteSpell { created, .. }
            | Self::ForgetSpell { created, .. }
            | Self::MemorizeSpell { created, .. }
            | Self::CastSpell { created, .. }
            | Self::SetPosture { created, .. }
            | Self::Consider { created, .. }
            | Self::Camp { created, .. }
            | Self::Loot { created, .. }
            | Self::LootItem { created, .. }
            | Self::Shop { created, .. }
            | Self::Buy { created, .. }
            | Self::Sell { created, .. }
            | Self::OfferTrade { created, .. }
            | Self::AcceptTrade { created, .. }
            | Self::MoveCoins { created, .. }
            | Self::Jump { created, .. }
            | Self::AutoAttack { created, .. }
            | Self::Consume { created, .. }
            | Self::UseAbility { created, .. }
            | Self::ConfigureMotion { created, .. } => Some(*created),
        }
    }
}

/// A command encoded as one game application packet.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct EncodedCommand {
    /// Little-endian application opcode understood by the selected dialect.
    pub opcode: u16,
    /// Application body without the opcode or reliable-UDP framing.
    pub body: Vec<u8>,
}

/// Encode a typed client action for one game dialect.
///
/// `character` supplies the active character name for packet layouts that
/// repeat the sender identity.
///
/// # Errors
///
/// Returns an error when a command contains values that cannot be represented
/// by the selected dialect.
pub fn encode(
    dialect: GameDialect,
    command: &GameCommand,
    character: &str,
) -> Result<EncodedCommand> {
    match command {
        GameCommand::SelectCharacter { .. } | GameCommand::CreateCharacter { .. } => {
            anyhow::bail!("character selection requires the world controller")
        }
        GameCommand::CastSpell {
            gem,
            spell_id,
            target_id,
            ..
        } => {
            anyhow::ensure!(
                dialect == GameDialect::Titanium,
                "casting is not implemented for this dialect"
            );
            anyhow::ensure!(
                *gem < 8 && *spell_id != 0 && *spell_id != u32::MAX && *target_id != 0,
                "invalid spell cast"
            );
            let mut body = Vec::with_capacity(20);
            for value in [
                u32::from(*gem),
                *spell_id,
                u32::MAX,
                u32::from(*target_id),
                0,
            ] {
                body.extend_from_slice(&value.to_le_bytes());
            }
            Ok(EncodedCommand {
                opcode: 0x304b,
                body,
            })
        }
        GameCommand::SetPosture {
            spawn_id, posture, ..
        } => encode_posture(dialect, *spawn_id, *posture),
        GameCommand::InspectItem { link_body, .. } => {
            anyhow::ensure!(
                dialect == GameDialect::Titanium,
                "item inspection is not implemented for this dialect"
            );
            Ok(EncodedCommand {
                opcode: 0x53e5,
                body: crate::items::request(link_body)?.to_vec(),
            })
        }
        GameCommand::SelectTarget { spawn_id, .. } => {
            anyhow::ensure!(
                dialect == GameDialect::Titanium,
                "targeting is not implemented for this dialect"
            );
            anyhow::ensure!(
                *spawn_id != Some(0),
                "zero is reserved for clearing a target"
            );
            Ok(EncodedCommand {
                opcode: 0x6c47,
                body: u32::from(spawn_id.unwrap_or(0)).to_le_bytes().to_vec(),
            })
        }
        GameCommand::Consider { .. } | GameCommand::AutoAttack { .. } => {
            encode_combat(dialect, command)
        }
        GameCommand::Loot { .. }
        | GameCommand::LootItem { .. }
        | GameCommand::EndLoot { .. }
        | GameCommand::Shop { .. }
        | GameCommand::Buy { .. }
        | GameCommand::Sell { .. } => encode_trade(dialect, command),
        // What these send depends on the admitted session's state.
        GameCommand::MoveInventory(_)
        | GameCommand::ClickDoor { .. }
        | GameCommand::PickUp { .. }
        | GameCommand::OfferTrade { .. }
        | GameCommand::AcceptTrade { .. }
        | GameCommand::CancelTrade { .. }
        | GameCommand::MoveCoins { .. }
        | GameCommand::Move(_)
        | GameCommand::Jump { .. }
        | GameCommand::ConfigureMotion { .. }
        | GameCommand::CrossZoneLine { .. }
        | GameCommand::Consume { .. }
        | GameCommand::UseAbility { .. }
        | GameCommand::WhoAll { .. }
        | GameCommand::UseItem(_)
        | GameCommand::MemorizeSpell { .. }
        | GameCommand::ForgetSpell { .. }
        | GameCommand::DeleteSpell { .. }
        | GameCommand::SwapSpell { .. }
        | GameCommand::ScribeSpell { .. }
        | GameCommand::Camp { .. } => {
            anyhow::bail!("this command requires the admitted session controller")
        }
        GameCommand::SendChat(message) => Ok(EncodedCommand {
            opcode: match dialect {
                GameDialect::Titanium => 0x1004,
                GameDialect::EqMac => 0x0741,
            },
            body: chat::encode_outbound_for(dialect, message, character)?,
        }),
    }
}

/// The Titanium appearance packet that sets the own spawn's posture.
///
/// # Errors
/// Rejects a zero spawn ID.
pub fn titanium_posture(spawn_id: u16, posture: Posture) -> Result<EncodedCommand> {
    encode_posture(GameDialect::Titanium, spawn_id, posture)
}

/// Titanium `OP_Camp`, which starts the server's own camp timer.
#[must_use]
pub fn titanium_camp() -> EncodedCommand {
    EncodedCommand {
        opcode: 0x78c1,
        body: vec![0; 4],
    }
}

/// Titanium `OP_Logout`, sent once the client's camp timer completes.
#[must_use]
pub fn titanium_logout() -> EncodedCommand {
    EncodedCommand {
        opcode: 0x61ff,
        body: Vec::new(),
    }
}

fn encode_posture(dialect: GameDialect, spawn_id: u16, posture: Posture) -> Result<EncodedCommand> {
    anyhow::ensure!(
        dialect == GameDialect::Titanium,
        "posture is not implemented for this dialect"
    );
    anyhow::ensure!(spawn_id != 0, "posture requires an own-spawn ID");
    let mut body = spawn_id.to_le_bytes().to_vec();
    body.extend_from_slice(&14u16.to_le_bytes());
    body.extend_from_slice(&posture.titanium_value().to_le_bytes());
    Ok(EncodedCommand {
        opcode: 0x7c32,
        body,
    })
}

/// Titanium-only corpse and merchant requests.
fn encode_trade(dialect: GameDialect, command: &GameCommand) -> Result<EncodedCommand> {
    anyhow::ensure!(
        dialect == GameDialect::Titanium,
        "looting and merchants are not implemented for this dialect"
    );
    let (opcode, body) = match command {
        GameCommand::Loot { corpse_id, .. } => (
            crate::loot::REQUEST_OPCODE,
            crate::loot::request(*corpse_id)?.to_vec(),
        ),
        GameCommand::LootItem {
            corpse_id,
            own_id,
            slot,
            auto,
            ..
        } => (
            crate::loot::ITEM_OPCODE,
            crate::loot::item_request(*corpse_id, *own_id, *slot, *auto)?.to_vec(),
        ),
        GameCommand::EndLoot { corpse_id, .. } => (
            crate::loot::END_OPCODE,
            crate::loot::end(*corpse_id)?.to_vec(),
        ),
        GameCommand::Shop {
            merchant_id,
            own_id,
            open: true,
            ..
        } => (
            crate::merchant::REQUEST_OPCODE,
            crate::merchant::request(*merchant_id, *own_id, true)?.to_vec(),
        ),
        GameCommand::Shop {
            merchant_id,
            own_id,
            ..
        } => (
            crate::merchant::END_OPCODE,
            crate::merchant::end(*merchant_id, *own_id)?.to_vec(),
        ),
        GameCommand::Buy {
            merchant_id,
            own_id,
            slot,
            quantity,
            ..
        } => (
            crate::merchant::BUY_OPCODE,
            crate::merchant::buy(*merchant_id, *own_id, *slot, *quantity)?.to_vec(),
        ),
        GameCommand::Sell {
            merchant_id,
            slot,
            quantity,
            ..
        } => (
            crate::merchant::SELL_OPCODE,
            crate::merchant::sell(*merchant_id, *slot, *quantity)?.to_vec(),
        ),
        _ => anyhow::bail!("not a corpse or merchant action"),
    };
    Ok(EncodedCommand { opcode, body })
}

/// Titanium-only consider and auto-attack requests.
fn encode_combat(dialect: GameDialect, command: &GameCommand) -> Result<EncodedCommand> {
    anyhow::ensure!(
        dialect == GameDialect::Titanium,
        "combat actions are not implemented for this dialect"
    );
    match command {
        GameCommand::Consider {
            own_id, target_id, ..
        } => Ok(EncodedCommand {
            opcode: crate::combat::CONSIDER_OPCODE,
            body: crate::combat::consider_request(*own_id, *target_id)?.to_vec(),
        }),
        GameCommand::AutoAttack { enabled, .. } => Ok(EncodedCommand {
            opcode: crate::combat::AUTO_ATTACK_OPCODE,
            body: crate::combat::auto_attack(*enabled).to_vec(),
        }),
        _ => anyhow::bail!("not a combat action"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spell_cast_preserves_full_inventory_sentinel_and_rejects_invalid_gems() {
        let mut cast = GameCommand::CastSpell {
            session_id: 7,
            gem: 2,
            spell_id: 42,
            target_id: 19,
            created: std::time::Instant::now(),
        };
        let packet = encode(GameDialect::Titanium, &cast, "Example").unwrap();
        assert_eq!(packet.opcode, 0x304b);
        assert_eq!(packet.body.len(), 20);
        assert_eq!(&packet.body[8..12], &[255; 4]);
        assert_eq!(&packet.body[16..], &[0; 4]);
        assert!(encode(GameDialect::EqMac, &cast, "Example").is_err());
        if let GameCommand::CastSpell { gem, .. } = &mut cast {
            *gem = 8;
        }
        assert!(encode(GameDialect::Titanium, &cast, "Example").is_err());
    }

    #[test]
    fn posture_uses_own_spawn_and_appearance_type() {
        for (posture, value) in [
            (Posture::Standing, 100u32),
            (Posture::Sitting, 110),
            (Posture::Ducking, 111),
        ] {
            let command = GameCommand::SetPosture {
                session_id: 7,
                spawn_id: 19,
                posture,
                created: std::time::Instant::now(),
            };
            let packet = encode(GameDialect::Titanium, &command, "Example").unwrap();
            assert_eq!(packet.opcode, 0x7c32);
            assert_eq!(&packet.body[..4], &[19, 0, 14, 0]);
            assert_eq!(&packet.body[4..], &value.to_le_bytes());
            assert!(encode(GameDialect::EqMac, &command, "Example").is_err());
        }
    }

    #[test]
    fn targeting_uses_only_the_mouse_target_opcode_and_four_byte_id() {
        let command = GameCommand::SelectTarget {
            session_id: 9,
            spawn_id: Some(513),
        };
        let packet = encode(GameDialect::Titanium, &command, "Example").unwrap();
        assert_eq!(packet.opcode, 0x6c47);
        assert_eq!(packet.body, vec![1, 2, 0, 0]);
        assert!(encode(GameDialect::EqMac, &command, "Example").is_err());
        let clear = GameCommand::SelectTarget {
            session_id: 9,
            spawn_id: None,
        };
        assert_eq!(
            encode(GameDialect::Titanium, &clear, "Example")
                .unwrap()
                .body,
            vec![0; 4]
        );
    }

    #[test]
    fn consider_and_auto_attack_use_titanium_combat_opcodes_only() {
        let created = std::time::Instant::now();
        let consider = GameCommand::Consider {
            session_id: 1,
            own_id: 7,
            target_id: 9,
            created,
        };
        let packet = encode(GameDialect::Titanium, &consider, "Example").unwrap();
        assert_eq!((packet.opcode, packet.body.len()), (0x65ca, 28));
        assert_eq!(&packet.body[4..8], &[9, 0, 0, 0]);
        assert!(encode(GameDialect::EqMac, &consider, "Example").is_err());
        let attack = GameCommand::AutoAttack {
            session_id: 1,
            enabled: true,
            created,
        };
        let packet = encode(GameDialect::Titanium, &attack, "Example").unwrap();
        assert_eq!((packet.opcode, packet.body), (0x5e55, vec![1, 0, 0, 0]));
        assert!(encode(GameDialect::EqMac, &attack, "Example").is_err());
    }

    #[test]
    fn corpse_and_merchant_commands_use_their_titanium_opcodes() {
        let created = std::time::Instant::now();
        for (command, opcode, length) in [
            (
                GameCommand::Loot {
                    session_id: 1,
                    corpse_id: 9,
                    created,
                },
                0x6f90,
                4,
            ),
            (
                GameCommand::LootItem {
                    session_id: 1,
                    corpse_id: 9,
                    own_id: 7,
                    slot: 22,
                    auto: true,
                    created,
                },
                0x7081,
                16,
            ),
            (
                GameCommand::EndLoot {
                    session_id: 1,
                    corpse_id: 9,
                },
                0x2316,
                4,
            ),
            (
                GameCommand::Shop {
                    session_id: 1,
                    merchant_id: 9,
                    own_id: 7,
                    open: true,
                    created,
                },
                0x45f9,
                16,
            ),
            (
                GameCommand::Shop {
                    session_id: 1,
                    merchant_id: 9,
                    own_id: 7,
                    open: false,
                    created,
                },
                0x7e03,
                8,
            ),
            (
                GameCommand::Buy {
                    session_id: 1,
                    merchant_id: 9,
                    own_id: 7,
                    slot: 2,
                    quantity: 1,
                    created,
                },
                0x221e,
                24,
            ),
            (
                GameCommand::Sell {
                    session_id: 1,
                    merchant_id: 9,
                    slot: 23,
                    quantity: 1,
                    created,
                },
                0x0e13,
                16,
            ),
        ] {
            let packet = encode(GameDialect::Titanium, &command, "Example").unwrap();
            assert_eq!((packet.opcode, packet.body.len()), (opcode, length));
            assert!(encode(GameDialect::EqMac, &command, "Example").is_err());
        }
    }

    #[test]
    fn chat_command_selects_each_dialects_opcode_and_layout() {
        let command = GameCommand::SendChat(OutboundChat::Say("ok".into()));
        let titanium = encode(GameDialect::Titanium, &command, "Example").unwrap();
        let eqmac = encode(GameDialect::EqMac, &command, "Example").unwrap();

        assert_eq!(titanium.opcode, 0x1004);
        assert_eq!(eqmac.opcode, 0x0741);
        assert_ne!(titanium.body, eqmac.body);
    }
}
