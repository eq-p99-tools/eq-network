//! The player's target as the server sets it, and whom that target targets.
//!
//! The servers set the player's target themselves at times, with
//! `OP_TargetCommand`: `EQEmu` as a sense spell or a corpse locator picks a
//! spawn (`zone/spell_effects.cpp`, `Client::LocateCorpse`) and from Perl's
//! `SendTargetCommand`; TAKP and Quarm as they refuse a mouse target,
//! putting back the last target or none, and they echo the player's own
//! `OP_TargetMouse` as the player clears their target
//! (`Client::Handle_OP_TargetCommand`). `EQEmu` tells the player whom their
//! target targets (`OP_TargetHoTT`) as the player's target changes and as
//! the target's own does (`EntityList::UpdateHoTT`).
//!
//! Layout reference: `ClientTarget_Struct`, a 32-bit spawn on Titanium
//! (`EQEmu`'s `common/eq_packet_structs.h`) and a 16-bit one on `EQMac`
//! (TAKP's).
use anyhow::{Context, Result};

/// Session-owned target identity. A new revision invalidates queued actions,
/// even when the selected spawn ID has returned to the same value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct TargetState {
    /// Monotonic within one zone admission; not a wire acknowledgment.
    pub revision: u64,
    /// The raw selected ID, including an unresolved server selection.
    pub spawn_id: Option<u16>,
    /// Whether the selection names the current visible incarnation, or none.
    pub available: bool,
}

impl Default for TargetState {
    fn default() -> Self {
        Self {
            revision: 0,
            spawn_id: None,
            available: true,
        }
    }
}

/// `OP_TargetCommand`: the target the server sets, or 0 for none.
pub const TARGET_COMMAND_OPCODE: u16 = 0x1477;
/// `EQMac`'s `OP_TargetCommand`.
pub const EQMAC_TARGET_COMMAND_OPCODE: u16 = 0xfe41;
/// `EQMac`'s `OP_TargetMouse`, the player's own, which TAKP and Quarm echo
/// as the player clears their target.
pub const EQMAC_TARGET_MOUSE_OPCODE: u16 = 0x6241;
/// `OP_TargetHoTT`: whom the player's target targets, or 0 for no one.
pub const TARGET_OF_TARGET_OPCODE: u16 = 0x6a12;

/// Decodes Titanium's target: a spawn in 32 bits, none for 0.
///
/// # Errors
/// Rejects any length other than four bytes, and a spawn beyond 16 bits.
pub fn titanium_target(body: &[u8]) -> Result<Option<u16>> {
    let bytes: [u8; 4] = body.try_into().context("invalid target length")?;
    let spawn = u16::try_from(u32::from_le_bytes(bytes)).context("target beyond any spawn")?;
    Ok((spawn != 0).then_some(spawn))
}

/// Decodes `EQMac`'s target: a spawn in 16 bits, none for 0.
///
/// # Errors
/// Rejects any length other than two bytes.
pub fn eqmac_target(body: &[u8]) -> Result<Option<u16>> {
    let bytes: [u8; 2] = body.try_into().context("invalid EQMac target length")?;
    let spawn = u16::from_le_bytes(bytes);
    Ok((spawn != 0).then_some(spawn))
}

/// What a client generation's program refuses or hides by a spawn's body
/// type ([`crate::world::SpawnState::body_type`]). A spawn whose body type
/// is unknown is never refused, kept from use or hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetRules {
    /// The body types the program's target setter will not take: it puts
    /// the previous target back, with no line and nothing sent.
    refused: &'static [u32],
    /// The body types the program will not interact with.
    no_interaction: &'static [u32],
    /// The program hides every body type above this one.
    hidden_above: u32,
}

/// Titanium's rules (program read R44).
///
/// - Refused: 11, 60, 65 and 67. The target setter 0x4a6f0e (a left click on
///   the world, 0x4a91a4, and the use key) puts the previous target back at
///   0x4a6fd5-0x4a6ff7, and so does the view pick of key command 0x1b
///   (0x4c9585).
/// - No interaction: 11, 65 and 67. Interaction 0x4a7356 returns at once for
///   them.
/// - Hidden: above 64. The per-spawn visibility and name update 0x480409
///   returns early for them.
pub const TITANIUM_TARGETING: TargetRules = TargetRules {
    refused: &[11, 60, 65, 67],
    no_interaction: &[11, 65, 67],
    hidden_above: 0x40,
};

impl TargetRules {
    /// Whether the program's target setter refuses a spawn of this body type.
    #[must_use]
    pub fn refuses(&self, body_type: Option<u32>) -> bool {
        body_type.is_some_and(|body| self.refused.contains(&body))
    }

    /// Whether the program will interact with a spawn of this body type.
    #[must_use]
    pub fn interacts(&self, body_type: Option<u32>) -> bool {
        !body_type.is_some_and(|body| self.no_interaction.contains(&body))
    }

    /// Whether the program hides a spawn of this body type: no name label
    /// and no per-frame visibility update.
    #[must_use]
    pub const fn hides(&self, body_type: Option<u32>) -> bool {
        matches!(body_type, Some(body) if body > self.hidden_above)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_read_each_generations_width_and_zero_is_none() {
        assert_eq!(titanium_target(&581u32.to_le_bytes()).unwrap(), Some(581));
        assert_eq!(titanium_target(&[0; 4]).unwrap(), None);
        assert!(titanium_target(&65536u32.to_le_bytes()).is_err());
        assert_eq!(eqmac_target(&581u16.to_le_bytes()).unwrap(), Some(581));
        assert_eq!(eqmac_target(&[0; 2]).unwrap(), None);
        for body in [&[0u8; 2][..], &[0; 5]] {
            assert!(titanium_target(body).is_err());
        }
        for body in [&[0u8; 1][..], &[0; 4]] {
            assert!(eqmac_target(body).is_err());
        }
    }

    #[test]
    fn titanium_refuses_hides_and_keeps_from_use_the_body_types_it_reads() {
        let rules = TITANIUM_TARGETING;
        for body in [11, 60, 65, 67] {
            assert!(rules.refuses(Some(body)), "{body}");
        }
        for body in [0, 1, 10, 12, 21, 33, 59, 61, 64, 66, 68] {
            assert!(!rules.refuses(Some(body)), "{body}");
        }
        for body in [11, 65, 67] {
            assert!(!rules.interacts(Some(body)), "{body}");
        }
        assert!(rules.interacts(Some(60)) && rules.interacts(Some(1)));
        assert!(!rules.hides(Some(64)) && rules.hides(Some(65)));
        assert!(!rules.hides(Some(11)) && !rules.hides(Some(60)));
        assert!(rules.hides(Some(u32::MAX)));
    }

    #[test]
    fn an_unknown_body_type_is_never_refused_kept_from_use_or_hidden() {
        let rules = TITANIUM_TARGETING;
        assert!(!rules.refuses(None));
        assert!(rules.interacts(None));
        assert!(!rules.hides(None));
    }
}
