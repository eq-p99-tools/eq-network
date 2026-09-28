//! Click-cast metadata from Titanium's serialized item definition.
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::time::{Duration, Instant};

/// One explicit item-use intent, bound to the selected inventory and zone admission.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemUse {
    /// Caller-generated correlation ID, unique within an admission.
    pub request_id: u64,
    /// Owning zone admission.
    pub session_id: u64,
    /// Inventory revision at selection time.
    pub revision: u64,
    /// Occupied equipment or main inventory slot.
    pub slot: super::InventorySlot,
    /// Explicit own or known zone spawn ID.
    pub target_id: u16,
    /// Local creation time; stalls and reconnects do not replay requests.
    pub created: Instant,
}

/// Effect activation category; unknown values retain their original byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ClickKind {
    /// Ordinary click effect (1).
    Click,
    /// Consumes a charge or item when activated (3).
    Expendable,
    /// Requires the item to be equipped (4).
    Equipped,
    /// Alternate click category (5); eligibility still needs server validation.
    Click2,
    /// A category without implemented click semantics.
    Unknown(u8),
}

/// Static effect metadata, separate from the item's remaining instance charges.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ClickEffect {
    /// Spell ID supplied by the item definition.
    pub spell_id: u32,
    /// Activation category, including unknown categories.
    pub kind: ClickKind,
    /// `Click.Level2`, checked by `EQEmu`'s cast request handler.
    pub required_level: u8,
    /// Original `Click.Level` field, retained independently.
    pub effect_level: u8,
    /// Serialized base cast time in milliseconds.
    pub cast_time_ms: u32,
    /// Serialized reuse duration in seconds.
    pub recast_delay_seconds: u32,
    /// Reuse group identifier; zero is preserved.
    pub recast_type: u32,
}

/// Activation data from an inventory instance, without predicting use or cooldown expiry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ItemActivation {
    /// Static maximum charges, preserving negative unlimited-charge sentinels.
    pub maximum_charges: i32,
    /// None when the effect ID is a known absent-spell sentinel.
    pub effect: Option<ClickEffect>,
    /// Raw instance recast timestamp. No local clock conversion is assumed.
    pub recast_timestamp: u32,
}

impl ItemActivation {
    /// Field positions follow Titanium `SerializeItem`, not another client's item layout.
    pub(super) fn decode(fields: &[&str], recast_timestamp: &str) -> Result<Self> {
        let number = |index: usize| -> Result<u32> {
            fields
                .get(index)
                .context("truncated click metadata")?
                .parse()
                .context("invalid click metadata")
        };
        let id = fields
            .get(134)
            .context("missing click spell")?
            .parse::<i64>()
            .context("invalid click spell")?;
        let effect = if matches!(id, -1 | 0 | 0xffff | 0xffff_ffff) {
            None
        } else {
            let kind = match u8::try_from(number(135)?)? {
                1 => ClickKind::Click,
                3 => ClickKind::Expendable,
                4 => ClickKind::Equipped,
                5 => ClickKind::Click2,
                value => ClickKind::Unknown(value),
            };
            Some(ClickEffect {
                spell_id: u32::try_from(id).context("click spell ID out of range")?,
                kind,
                required_level: u8::try_from(number(136)?)?,
                effect_level: u8::try_from(number(137)?)?,
                cast_time_ms: number(60)?,
                recast_delay_seconds: number(119)?,
                recast_type: number(120)?,
            })
        };
        Ok(Self {
            maximum_charges: fields
                .get(55)
                .context("missing maximum charges")?
                .parse()
                .context("invalid maximum charges")?,
            effect,
            recast_timestamp: recast_timestamp
                .parse()
                .context("invalid item recast timestamp")?,
        })
    }
}

impl super::Inventory {
    /// Validates an admitted item-use intent before encoding its current effect.
    /// `target_available` must come from the controller's current zone spawn state.
    ///
    /// # Errors
    /// Rejects old admissions, future/expired requests, unavailable targets and invalid inventory.
    pub fn prepare_item_cast(
        &self,
        request: &ItemUse,
        session_id: u64,
        level: u8,
        target_available: bool,
        now: Instant,
    ) -> Result<(u32, [u8; 20])> {
        ensure!(
            request.session_id == session_id,
            "Item use belongs to an old admission"
        );
        ensure!(
            now.checked_duration_since(request.created)
                .is_some_and(|age| age < Duration::from_secs(1)),
            "Item-use request expired or has a future timestamp"
        );
        ensure!(target_available, "Item-use target is unavailable");
        let body =
            self.item_cast_packet(request.revision, request.slot, level, request.target_id)?;
        let spell_id = self.items[&request.slot]
            .activation
            .effect
            .as_ref()
            .context("Item effect unavailable")?
            .spell_id;
        Ok((spell_id, body))
    }

    /// Builds a Titanium item cast from the current instance, without consuming charges.
    /// The session controller must additionally validate admission, age, target and cast state.
    ///
    /// # Errors
    /// Rejects stale state, unsupported slots/categories, insufficient level or depleted charges.
    pub fn item_cast_packet(
        &self,
        revision: u64,
        slot: super::InventorySlot,
        level: u8,
        target_id: u16,
    ) -> Result<[u8; 20]> {
        ensure!(
            self.received && !self.stale(),
            "Wait for current inventory contents"
        );
        ensure!(
            revision == self.revision,
            "Inventory changed; select the item again"
        );
        // Titanium's AllowClickCastFromBag is false; bank and cursor are not ordinary click slots.
        ensure!(
            (0..=29).contains(&slot.0),
            "Place the item in equipment or a main inventory slot"
        );
        ensure!(target_id != 0, "Item casting requires a target");
        let item = self.items.get(&slot).context("The item slot is empty")?;
        let effect = item
            .activation
            .effect
            .as_ref()
            .context("The item has no click effect")?;
        ensure!(
            !matches!(effect.spell_id, 0 | 0xffff | u32::MAX),
            "Invalid item spell ID"
        );
        ensure!(
            !matches!(effect.kind, ClickKind::Unknown(_)),
            "Unsupported item click category"
        );
        ensure!(
            effect.kind != ClickKind::Equipped || slot.0 <= 21,
            "Equip this item before using it"
        );
        ensure!(
            level >= effect.required_level,
            "Level is too low for this item effect"
        );
        ensure!(item.charges >= -1, "Unknown item charge state");
        ensure!(item.stack_count != Some(0), "The item stack is empty");
        ensure!(
            item.activation.maximum_charges <= 0 || item.charges != 0,
            "The item has no charges remaining"
        );
        ensure!(
            effect.kind != ClickKind::Expendable || item.stack_count.is_some() || item.charges != 0,
            "The expendable item has no charges remaining"
        );
        let mut body = [0; 20];
        for (bytes, value) in body.chunks_exact_mut(4).zip([
            10,
            effect.spell_id,
            u32::try_from(slot.0)?,
            u32::from(target_id),
            0,
        ]) {
            bytes.copy_from_slice(&value.to_le_bytes());
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinels_unknown_categories_and_instance_recast_are_preserved() {
        let mut fields = vec!["0"; 159];
        for sentinel in ["-1", "0", "65535", "4294967295"] {
            fields[134] = sentinel;
            let value = ItemActivation::decode(&fields, "123456").unwrap();
            assert!(value.effect.is_none());
            assert_eq!(value.recast_timestamp, 123_456);
        }
        fields[134] = "73";
        for (wire, expected) in [
            ("1", ClickKind::Click),
            ("3", ClickKind::Expendable),
            ("4", ClickKind::Equipped),
            ("5", ClickKind::Click2),
            ("253", ClickKind::Unknown(253)),
        ] {
            fields[135] = wire;
            assert_eq!(
                ItemActivation::decode(&fields, "0")
                    .unwrap()
                    .effect
                    .unwrap()
                    .kind,
                expected
            );
        }
        fields[135] = "256";
        assert!(ItemActivation::decode(&fields, "0").is_err());
        fields[135] = "1";
        for invalid in ["-2", "4294967296", "bad"] {
            fields[134] = invalid;
            assert!(ItemActivation::decode(&fields, "0").is_err());
        }
        fields[134] = "73";
        assert!(ItemActivation::decode(&fields[..134], "0").is_err());
        assert!(ItemActivation::decode(&fields, "-1").is_err());
    }
}
