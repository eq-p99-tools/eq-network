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
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum GameCommand {
    /// Enter one occupied slot from the current world-server character list.
    SelectCharacter {
        /// Identity supplied with the character list.
        selection_id: u64,
        /// Server slot to enter.
        slot: u8,
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
        GameCommand::SelectCharacter { .. } => {
            anyhow::bail!("character selection requires the world controller")
        }
        GameCommand::UseItem(_)
        | GameCommand::MemorizeSpell { .. }
        | GameCommand::ForgetSpell { .. }
        | GameCommand::DeleteSpell { .. }
        | GameCommand::SwapSpell { .. }
        | GameCommand::ScribeSpell { .. } => {
            anyhow::bail!("item and spellbook actions require the admitted session controller")
        }
        GameCommand::CastSpell {
            gem,
            spell_id,
            target_id,
            ..
        } => {
            anyhow::ensure!(
                dialect == GameDialect::TitaniumP99,
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
                dialect == GameDialect::TitaniumP99,
                "item inspection is not implemented for this dialect"
            );
            Ok(EncodedCommand {
                opcode: 0x53e5,
                body: crate::items::request(link_body)?.to_vec(),
            })
        }
        GameCommand::SelectTarget { spawn_id, .. } => {
            anyhow::ensure!(
                dialect == GameDialect::TitaniumP99,
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
        GameCommand::MoveInventory(_) => {
            anyhow::bail!("inventory moves require the admitted session controller")
        }
        GameCommand::ClickDoor { .. } => {
            anyhow::bail!("door interaction requires the admitted session controller")
        }
        GameCommand::Move(_)
        | GameCommand::ConfigureMotion { .. }
        | GameCommand::CrossZoneLine { .. } => {
            anyhow::bail!("movement requires the admitted session controller")
        }
        GameCommand::SendChat(message) => Ok(EncodedCommand {
            opcode: match dialect {
                GameDialect::TitaniumP99 => 0x1004,
                GameDialect::EqMac => 0x0741,
            },
            body: chat::encode_outbound_for(dialect, message, character)?,
        }),
    }
}

/// Titanium appearance update for the player's own stance.
fn encode_posture(dialect: GameDialect, spawn_id: u16, posture: Posture) -> Result<EncodedCommand> {
    anyhow::ensure!(
        dialect == GameDialect::TitaniumP99,
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

/// Titanium-only consider and auto-attack requests.
fn encode_combat(dialect: GameDialect, command: &GameCommand) -> Result<EncodedCommand> {
    anyhow::ensure!(
        dialect == GameDialect::TitaniumP99,
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
        let packet = encode(GameDialect::TitaniumP99, &cast, "Example").unwrap();
        assert_eq!(packet.opcode, 0x304b);
        assert_eq!(packet.body.len(), 20);
        assert_eq!(&packet.body[8..12], &[255; 4]);
        assert_eq!(&packet.body[16..], &[0; 4]);
        assert!(encode(GameDialect::EqMac, &cast, "Example").is_err());
        if let GameCommand::CastSpell { gem, .. } = &mut cast {
            *gem = 8;
        }
        assert!(encode(GameDialect::TitaniumP99, &cast, "Example").is_err());
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
            let packet = encode(GameDialect::TitaniumP99, &command, "Example").unwrap();
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
        let packet = encode(GameDialect::TitaniumP99, &command, "Example").unwrap();
        assert_eq!(packet.opcode, 0x6c47);
        assert_eq!(packet.body, vec![1, 2, 0, 0]);
        assert!(encode(GameDialect::EqMac, &command, "Example").is_err());
        let clear = GameCommand::SelectTarget {
            session_id: 9,
            spawn_id: None,
        };
        assert_eq!(
            encode(GameDialect::TitaniumP99, &clear, "Example")
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
        let packet = encode(GameDialect::TitaniumP99, &consider, "Example").unwrap();
        assert_eq!((packet.opcode, packet.body.len()), (0x65ca, 28));
        assert_eq!(&packet.body[4..8], &[9, 0, 0, 0]);
        assert!(encode(GameDialect::EqMac, &consider, "Example").is_err());
        let attack = GameCommand::AutoAttack {
            session_id: 1,
            enabled: true,
            created,
        };
        let packet = encode(GameDialect::TitaniumP99, &attack, "Example").unwrap();
        assert_eq!((packet.opcode, packet.body), (0x5e55, vec![1, 0, 0, 0]));
        assert!(encode(GameDialect::EqMac, &attack, "Example").is_err());
    }

    #[test]
    fn chat_command_selects_each_dialects_opcode_and_layout() {
        let command = GameCommand::SendChat(OutboundChat::Say("ok".into()));
        let titanium = encode(GameDialect::TitaniumP99, &command, "Example").unwrap();
        let eqmac = encode(GameDialect::EqMac, &command, "Example").unwrap();

        assert_eq!(titanium.opcode, 0x1004);
        assert_eq!(eqmac.opcode, 0x0741);
        assert_ne!(titanium.body, eqmac.body);
    }
}
