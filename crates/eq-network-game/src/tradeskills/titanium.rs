//! The native manual-combine predicates, independent of a recipe's verdict.
#[cfg(test)]
mod tests;
use crate::inventory::{Inventory, InventorySlot};

/// A local prerequisite that prevents submitting a carried combine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CarriedRefusal {
    /// Fallback explanation when installed strings are unavailable.
    pub reason: &'static str,
    /// Native string, where the program supplies one without arguments.
    pub string_id: Option<u32>,
}

impl CarriedRefusal {
    const fn new(reason: &'static str, string_id: Option<u32>) -> Self {
        Self { reason, string_id }
    }
}

/// Checks the known inventory and native carried-container prerequisites.
/// Race/class restrictions describe the container, not recipe eligibility.
/// No item is consumed, result predicted or server acceptance inferred.
///
/// # Errors
/// Refuses unavailable inventory, noncarried/unknown containers, occupied
/// hands, restricted containers, stacked components and empty containers.
pub fn check_carried(
    inventory: &Inventory,
    slot: InventorySlot,
    race: u32,
    class: Option<u32>,
    cursor_has_coins: bool,
) -> Result<(), CarriedRefusal> {
    if !inventory.received() || inventory.stale() {
        return Err(CarriedRefusal::new("The inventory is not current", None));
    }
    let container = inventory
        .items()
        .get(&slot)
        .filter(|item| super::can_combine_in(item) && (1..=10).contains(&item.bag_slots))
        .ok_or_else(|| CarriedRefusal::new("That is not a carried tradeskill container", None))?;
    let kind = container.rules.bag_type;
    let required_class = match kind {
        24 => Some(12),
        25 => Some(13),
        26 => Some(11),
        27 => Some(14),
        _ => None,
    };
    if required_class.is_some_and(|required| class != Some(required)) {
        return Err(CarriedRefusal::new(
            "This research container requires another class",
            Some(12010),
        ));
    }
    let restricted = match kind {
        31 => Some((5, 12011)),
        32 => Some((6, 12012)),
        33 => Some((10, 12013)),
        34 => Some((8, 12014)),
        35 => Some((12, 12015)),
        36 => Some((2, 12016)),
        38 => Some((128, 12017)),
        39 | 40 => Some((1, 12018)),
        45 => Some((128, 12019)),
        42 => Some((3, 12020)),
        41 => Some((11, 12021)),
        43 => Some((4, 12022)),
        44 => Some((4, 12023)),
        _ => None,
    };
    if let Some((_, string_id)) = restricted.filter(|(required, _)| race != *required) {
        return Err(CarriedRefusal::new(
            "This cultural container requires another race",
            Some(string_id),
        ));
    }
    if inventory.items().contains_key(&InventorySlot::CURSOR) || cursor_has_coins {
        return Err(CarriedRefusal::new(
            "Your cursor must be empty to combine",
            Some(super::HANDS_FULL),
        ));
    }
    let mut occupied = false;
    for index in 0..container.bag_slots {
        let Some(component) = slot
            .child(index)
            .and_then(|child| inventory.items().get(&child))
        else {
            continue;
        };
        if component.stack_count.is_some_and(|count| count > 1) {
            return Err(CarriedRefusal::new(
                "Separate stacked components before combining",
                Some(8504),
            ));
        }
        if component.stack_count == Some(0) {
            return Err(CarriedRefusal::new(
                "A component's quantity is unavailable",
                None,
            ));
        }
        occupied = true;
    }
    if !occupied {
        return Err(CarriedRefusal::new(
            "The container has no components",
            Some(12025),
        ));
    }
    Ok(())
}
