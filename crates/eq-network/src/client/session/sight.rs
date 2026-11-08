//! Received own-player observations and effective Titanium See Invisible.
use eq_network_game::{buffs::Buff, vision::SightCatalog, world::WorldEvent};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug, Default)]
pub(super) struct Sight {
    catalog: Option<Arc<dyn SightCatalog>>,
    slots: BTreeMap<u32, Buff>,
    landings: BTreeMap<u16, u16>,
    published: bool,
}

impl Sight {
    pub(super) fn new(catalog: Option<Arc<dyn SightCatalog>>) -> Self {
        Self {
            catalog,
            ..Self::default()
        }
    }

    pub(super) fn clear(&mut self) {
        self.slots.clear();
        self.landings.clear();
    }

    pub(super) fn apply(&mut self, event: &WorldEvent, own: Option<u16>) {
        if self.catalog.is_none() {
            return;
        }
        let own = own.map(u32::from);
        match event {
            WorldEvent::BuffSnapshot(buffs) => {
                self.slots = buffs
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, buff)| Some((u32::try_from(slot).ok()?, buff.clone()?)))
                    .collect();
                self.landings.clear();
            }
            WorldEvent::Buff(update) if Some(update.entity_id) == own => {
                if let Ok(id) = u16::try_from(update.spell_id) {
                    self.landings.remove(&id);
                }
                if let Some(buff) = &update.buff {
                    self.slots.insert(update.slot, buff.clone());
                } else if update.replaces_slot
                    || self
                        .slots
                        .get(&update.slot)
                        .is_some_and(|buff| buff.spell_id == update.spell_id)
                {
                    self.slots.remove(&update.slot);
                }
            }
            WorldEvent::SpellEffect(effect)
                if Some(u32::from(effect.target_id)) == own
                    && effect.effect_flag == 4
                    && !matches!(effect.spell_id, 0 | u16::MAX) =>
            {
                // A landing refresh has no slot. Keep one observation per
                // spell ID so a replay that arrives later can classify it.
                let id = u32::from(effect.spell_id);
                self.slots.retain(|_, buff| buff.spell_id != id);
                self.landings.insert(effect.spell_id, effect.caster_level);
            }
            _ => (),
        }
    }

    pub(super) fn sees_invisible(&self) -> bool {
        self.catalog.as_ref().is_some_and(|catalog| {
            self.slots.values().any(|buff| {
                catalog.sees_invisible(buff.spell_id, u16::from(buff.caster_level), false)
            }) || self
                .landings
                .iter()
                .any(|(&spell, &level)| catalog.sees_invisible(u32::from(spell), level, true))
        })
    }

    pub(super) fn changed(&mut self) -> Option<bool> {
        let visible = self.sees_invisible();
        (visible != std::mem::replace(&mut self.published, visible)).then_some(visible)
    }
}
