//! Admission-scoped, read-only AA environmental observations for local `EQEmu`.
use super::{
    feature::{Feature, Out, World},
    ClientEvent,
};
use anyhow::Result;
use eq_network_game::{alternate_advancement::Update, message::Message, world::WorldEvent};

const MAX_STAGED: usize = 4096;

#[derive(Default)]
pub(super) struct AlternateAdvancement {
    staged: Vec<Update>,
    overflowed: bool,
}

fn publish(update: Update, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
    out.log
        .send(ClientEvent::World(WorldEvent::AlternateAdvancement {
            session_id: world.session_id,
            update,
        }))
}

impl Feature for AlternateAdvancement {
    fn admit(&mut self, message: &Message, _world: &mut World) -> Result<()> {
        if let Message::AlternateAdvancement(update) = message {
            if self.staged.len() == MAX_STAGED {
                self.staged.clear();
                self.staged.push(Update::Selections(None));
                self.staged.push(Update::DefinitionsUnknown);
                self.overflowed = true;
            } else if !self.overflowed {
                self.staged.push(update.clone());
            }
        }
        Ok(())
    }

    fn admitted(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        for update in std::mem::take(&mut self.staged) {
            publish(update, world, out)?;
        }
        Ok(())
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Message::AlternateAdvancement(update) = message {
            publish(update.clone(), world, out)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;

    #[test]
    fn facts_stage_until_admission_and_are_stamped_by_the_receiving_feature() {
        let mut feature = AlternateAdvancement::default();
        let mut world = World::new(5);
        let message = Message::AlternateAdvancement(Update::Selections(Some(Vec::new())));
        feature.admit(&message, &mut world).unwrap();
        let outcome = testing::run(|out| feature.admitted(&mut world, out));
        outcome.result.unwrap();
        assert!(
            matches!(&outcome.events[..], [ClientEvent::World(WorldEvent::AlternateAdvancement { session_id: 5, update: Update::Selections(Some(selected)) })] if selected.is_empty())
        );
        let outcome = testing::run(|out| feature.observe(&message, &mut world, out));
        outcome.result.unwrap();
        assert!(matches!(
            &outcome.events[..],
            [ClientEvent::World(WorldEvent::AlternateAdvancement {
                session_id: 5,
                ..
            })]
        ));
    }
}
