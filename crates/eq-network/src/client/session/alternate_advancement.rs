//! Admission-scoped, read-only AA environmental observations for Titanium servers.
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

fn publish(updates: Vec<Update>, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
    if updates.is_empty() {
        return Ok(());
    }
    out.log
        .send(ClientEvent::World(WorldEvent::AlternateAdvancement {
            session_id: world.session_id,
            updates,
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
        publish(std::mem::take(&mut self.staged), world, out)
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Message::AlternateAdvancement(update) = message {
            publish(vec![update.clone()], world, out)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use crate::client::ConnectionStage;
    use eq_network_game::alternate_advancement::{Rank, Selection};

    fn entered(out: &mut Out<'_, '_>) -> Result<()> {
        out.log.send(ClientEvent::World(WorldEvent::Entered {
            capabilities: vec![eq_network_game::world::Capability::Drowning],
            choices: Vec::new(),
            session_id: 5,
            zone: "synthetic".into(),
            zone_id: 1,
            player: Box::new(testing::player(7)),
            far_clip: None,
            spell_landing_flag: Some(4),
        }))
    }

    #[test]
    fn staged_facts_are_one_ordered_batch_between_admission_and_ready() {
        let mut feature = AlternateAdvancement::default();
        let mut world = World::new(5);
        let mut facts = vec![
            Update::Selections(Some(Vec::new())),
            Update::Selections(Some(vec![Selection { id: 1, value: 99 }])),
        ];
        facts.extend((1..=113).map(|id| Update::Definition {
            id,
            rank: Some(Rank {
                id,
                required_level: 1,
                sequence: id,
                rank: 1,
                classes: 2,
                effects: Vec::new(),
            }),
        }));
        let before = testing::run(|_| {
            for update in &facts {
                feature
                    .admit(&Message::AlternateAdvancement(update.clone()), &mut world)
                    .unwrap();
            }
        });
        assert!(before.events.is_empty());
        let outcome = testing::run(|out| {
            entered(out)?;
            feature.admitted(&mut world, out)?;
            out.log.send(ClientEvent::Progress(ConnectionStage::Ready))
        });
        outcome.result.unwrap();
        assert_eq!(facts.len(), 115);
        assert!(matches!(outcome.events.as_slice(), [
            ClientEvent::World(WorldEvent::Entered { session_id: 5, .. }),
            ClientEvent::World(WorldEvent::LiquidHazardRevision { session_id: 5, revision: 0 }),
            ClientEvent::World(WorldEvent::AlternateAdvancement { session_id: 5, updates }),
            ClientEvent::World(WorldEvent::LiquidHazardRevision { session_id: 5, revision: 1 }),
            ClientEvent::Progress(ConnectionStage::Ready),
        ] if updates == &facts));
    }

    #[test]
    fn empty_stage_sends_nothing_and_ongoing_facts_publish_immediately() {
        let mut feature = AlternateAdvancement::default();
        let mut world = World::new(5);
        let empty = testing::run(|out| feature.admitted(&mut world, out));
        empty.result.unwrap();
        assert!(empty.events.is_empty());
        let outcome = testing::run(|out| {
            entered(out)?;
            feature.observe(
                &Message::AlternateAdvancement(Update::Unknown),
                &mut world,
                out,
            )
        });
        outcome.result.unwrap();
        assert!(matches!(&outcome.events[2..], [
            ClientEvent::World(WorldEvent::AlternateAdvancement { session_id: 5, updates }),
            ClientEvent::World(WorldEvent::LiquidHazardRevision { session_id: 5, revision: 1 }),
        ] if updates == &[Update::Unknown]));
    }

    #[test]
    fn the_staging_bound_retains_exact_limit_then_withdraws_on_overflow() {
        for overflow in [false, true] {
            let mut feature = AlternateAdvancement::default();
            let mut world = World::new(5);
            let facts: Vec<_> = (1..=MAX_STAGED)
                .map(|id| Update::Definition {
                    id: u32::try_from(id).unwrap(),
                    rank: None,
                })
                .collect();
            for update in &facts {
                feature
                    .admit(&Message::AlternateAdvancement(update.clone()), &mut world)
                    .unwrap();
            }
            if overflow {
                for update in [Update::Unknown, Update::Selections(Some(Vec::new()))] {
                    feature
                        .admit(&Message::AlternateAdvancement(update), &mut world)
                        .unwrap();
                }
            }
            let outcome = testing::run(|out| feature.admitted(&mut world, out));
            outcome.result.unwrap();
            let expected = if overflow {
                vec![Update::Selections(None), Update::DefinitionsUnknown]
            } else {
                facts
            };
            assert!(matches!(outcome.events.as_slice(), [
                ClientEvent::World(WorldEvent::AlternateAdvancement { session_id: 5, updates }),
            ] if updates == &expected));
        }
    }
}
