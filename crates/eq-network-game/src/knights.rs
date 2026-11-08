//! Titanium Paladin and Shadow Knight actions use the ability casting slot.
use crate::command::EncodedCommand;
use anyhow::{ensure, Result};
use serde::Serialize;
use std::time::Duration;

/// The Titanium cast slot reserved for these two abilities.
pub const CAST_SLOT: u8 = 9;

/// Class abilities available from level one.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum KnightAbility {
    /// Paladin healing ability.
    LayOnHands,
    /// Shadow Knight offensive ability.
    HarmTouch,
}

impl KnightAbility {
    /// Name for buttons and feedback.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::LayOnHands => "Lay on Hands",
            Self::HarmTouch => "Harm Touch",
        }
    }

    /// Stable identifier for hotbar bindings and ordinary command names.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::LayOnHands => "layonhands",
            Self::HarmTouch => "harmtouch",
        }
    }

    /// Resolve a supported named action.
    #[must_use]
    pub fn named(key: &str) -> Option<Self> {
        match key {
            "layonhands" => Some(Self::LayOnHands),
            "harmtouch" => Some(Self::HarmTouch),
            _ => None,
        }
    }

    /// The class ability shown at this class and level, if any.
    #[must_use]
    pub const fn for_player(class: Option<u32>, level: u8) -> Option<Self> {
        if level == 0 {
            return None;
        }
        match class {
            Some(3) => Some(Self::LayOnHands),
            Some(5) => Some(Self::HarmTouch),
            _ => None,
        }
    }

    /// The spell ID the Titanium ability button sends. EQEmu can substitute
    /// 2821 for Harm Touch at level 40 or above.
    #[must_use]
    pub const fn spell(self) -> u32 {
        match self {
            Self::LayOnHands => 87,
            Self::HarmTouch => 88,
        }
    }

    /// Whether the server spell belongs to this ability, including the
    /// alternate Harm Touch rank.
    #[must_use]
    pub const fn matches_spell(self, spell: u32) -> bool {
        self.spell() == spell || (matches!(self, Self::HarmTouch) && spell == 2821)
    }
}

/// Read the Titanium profile's abilitySlotRefresh in milliseconds. An absent
/// field, non-knight profile or all-ones sentinel gives no reuse observation.
#[must_use]
pub fn titanium_profile(profile: &[u8]) -> Option<Duration> {
    if profile.len() != 19_592 {
        return None;
    }
    let class = u32::from_le_bytes(profile[12..16].try_into().ok()?);
    KnightAbility::for_player(Some(class), profile[20])?;
    let ms = u32::from_le_bytes(profile[168..172].try_into().ok()?);
    (ms != u32::MAX).then(|| Duration::from_millis(u64::from(ms)))
}

/// Encode the ordinary ability cast. Its class, target and reuse checks belong
/// to the admitted session. See EQEmu's Titanium CastSpell decoder and
/// Client::Handle_OP_CastSpell.
///
/// # Errors
/// Refuses an absent target.
pub fn titanium_cast(ability: KnightAbility, target: u16) -> Result<EncodedCommand> {
    ensure!(target != 0, "invalid knight ability target");
    Ok(crate::spells::titanium_cast(
        CAST_SLOT,
        ability.spell(),
        target,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ability_slot_uses_base_spell_and_full_no_item_sentinel() {
        let packet = titanium_cast(KnightAbility::HarmTouch, 123).unwrap();
        assert_eq!(packet.opcode, 0x304b);
        assert_eq!(
            packet.body,
            [9, 88, u32::MAX, 123, 0].map(u32::to_le_bytes).concat()
        );
        assert!(titanium_cast(KnightAbility::LayOnHands, 0).is_err());
        assert!(KnightAbility::HarmTouch.matches_spell(2821));
        assert!(!KnightAbility::LayOnHands.matches_spell(2821));
    }

    #[test]
    fn profile_distinguishes_reuse_zero_and_unavailable() {
        let mut profile = vec![0; 19_592];
        profile[12..16].copy_from_slice(&3u32.to_le_bytes());
        assert_eq!(titanium_profile(&profile), None);
        profile[20] = 1;
        assert_eq!(titanium_profile(&profile), Some(Duration::ZERO));
        profile[168..172].copy_from_slice(&42_000u32.to_le_bytes());
        assert_eq!(titanium_profile(&profile), Some(Duration::from_secs(42)));
        profile[168..172].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(titanium_profile(&profile), None);
        assert_eq!(titanium_profile(&profile[..172]), None);
    }
}
