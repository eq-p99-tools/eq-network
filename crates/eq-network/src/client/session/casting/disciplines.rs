//! Titanium disciplines: any the server listed, cast with the discipline
//! slot (0x4bb39d, read R48), and the received reuse and active state.
use super::{rejected, Casting, Source};
use crate::client::session::{
    feature::{Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::{ensure, Context, Result};
use eq_network_game::{
    disciplines::{titanium_cast, Active, Timers},
    message::Message,
    world::WorldEvent,
};
use std::time::{Duration, Instant};

/// An inferred one-second local duplicate guard independent of observed reuse
/// (recording checklist 6.403).
/// The native `eqgame.exe` 0x4bb39d checks a received group timer in seconds
/// and supplies no standalone one-second retry rule.
const RETRY_DELAY: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(super) struct DisciplineState {
    reuse: Timers,
    linked: Timers,
    active: Active,
    retry_at: Option<Instant>,
    staged_active: bool,
}

impl DisciplineState {
    /// Track only received reuse and exact active slot changes.
    pub(super) fn observe(
        &mut self,
        message: &Message,
        own: Option<u16>,
        now: Instant,
        staging: bool,
    ) {
        let Message::Event(event) = message else {
            return;
        };
        let active_changed = match event {
            WorldEvent::DisciplineReuse(reuse) => {
                self.reuse.set(*reuse, now);
                false
            }
            WorldEvent::LinkedReuse(reuse) => {
                self.linked.set(*reuse, now);
                false
            }
            WorldEvent::Discipline(discipline) => {
                self.active = discipline.map_or(Active::Inactive, Active::EqMac);
                true
            }
            WorldEvent::Buff(update) => {
                own.is_some_and(|own| self.active.observe_titanium(own, update))
            }
            _ => false,
        };
        if staging && active_changed {
            self.staged_active = true;
        }
    }

    /// Publish staged reuse and active state without replaying a buff update
    /// that the worker's buff reducer never applied.
    pub(super) fn report(&mut self, session_id: u64, out: &mut Out<'_, '_>) -> Result<()> {
        let now = Instant::now();
        let updates = self
            .reuse
            .updates(now)
            .map(WorldEvent::DisciplineReuse)
            .chain(self.linked.updates(now).map(WorldEvent::LinkedReuse));
        for event in updates {
            out.log.send(ClientEvent::World(event))?;
        }
        if std::mem::take(&mut self.staged_active) {
            out.log
                .send(ClientEvent::World(WorldEvent::DisciplineAdmission {
                    session_id,
                    state: self.active,
                }))?;
        }
        Ok(())
    }

    /// The active effect is lost on own death; server reuse stays recorded.
    pub(super) fn clear_active(&mut self) {
        self.active = Active::Unknown;
    }

    /// A pending retry, an observed active discipline or an observed group
    /// wait can refuse an otherwise eligible request.
    fn check(&self, timer: u32, now: Instant) -> Result<()> {
        ensure!(
            !matches!(self.active, Active::Titanium(_)),
            "A discipline is already active"
        );
        ensure!(
            self.reuse
                .remaining(timer, now)
                .is_none_or(|wait| wait.is_zero()),
            "The shared discipline timer is still running"
        );
        ensure!(
            self.retry_at.is_none_or(|retry| now >= retry),
            "Wait before trying a discipline again"
        );
        Ok(())
    }
}

impl Casting {
    /// Send a discipline the server listed and leave the result to the
    /// server's cast stream, active slot and reuse notices. The spell, its
    /// timer and its target come from the host's spell data.
    pub(super) fn use_discipline(
        &mut self,
        command: &ClientCommand,
        (spell_id, timer, target_id): (u32, u32, u16),
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let now = Instant::now();
        let prepared = (|| {
            ensure!(
                self.use_disciplines,
                "Disciplines are unavailable on this server"
            );
            let player = world.player.as_ref().context("Not in the zone yet")?;
            let known = player
                .disciplines
                .as_deref()
                .context("The server hasn't listed your disciplines")?;
            ensure!(
                spell_id != 0 && known.contains(&spell_id),
                "The discipline is unavailable to your character"
            );
            ensure!(
                !world.control.stunned(),
                "Cannot use a discipline while stunned"
            );
            self.disciplines.check(timer, now)?;
            titanium_cast(spell_id, target_id)
        })();
        match prepared {
            Ok(packet) => {
                out.send(&packet)?;
                self.disciplines.retry_at = Some(now + RETRY_DELAY);
                self.submitted(spell_id, Source::Discipline, world, out)
            }
            Err(error) => {
                let reason = error.to_string();
                if let Some(event) = rejected(command, &reason) {
                    out.log.send(ClientEvent::World(event))?;
                }
                out.log.diagnostic(reason)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::session::feature::{testing, Feature};
    use eq_network_game::disciplines::{Discipline, Reuse};

    fn use_spell(spell_id: u32, target_id: u16) -> ClientCommand {
        ClientCommand::UseDiscipline {
            session_id: 1,
            spell_id,
            timer: 1,
            target_id,
            created: Instant::now(),
        }
    }

    #[test]
    fn any_listed_discipline_casts_at_its_target_and_an_unlisted_one_is_refused() {
        let mut world = World::new(1);
        world.player.admit(testing::player(7));
        let mut casting = Casting::with_titanium_combat(false, Duration::from_mins(72));
        // No list from the server yet: nothing is sent.
        let report = testing::run(|out| casting.handle(&use_spell(4585, 7), &mut world, out));
        report.result.unwrap();
        assert_eq!(report.sent, []);
        assert!(
            report.events.iter().any(|event| matches!(
                event,
                ClientEvent::World(WorldEvent::CastRejected { spell_id: 4585, .. })
            )),
            "{:?}",
            report.events
        );
        let mut player = testing::player(7);
        player.disciplines = Some(vec![4585, 0]);
        world.player.admit(player);
        // A spell the list doesn't hold, and the empty slot, are refused.
        for spell in [4586, 0] {
            let report = testing::run(|out| casting.handle(&use_spell(spell, 7), &mut world, out));
            report.result.unwrap();
            assert_eq!(report.sent, [], "{spell}");
        }
        // A listed one casts with the discipline slot at the target given.
        let report = testing::run(|out| casting.handle(&use_spell(4585, 300), &mut world, out));
        report.result.unwrap();
        assert_eq!(report.sent, [titanium_cast(4585, 300).unwrap()]);
    }

    #[test]
    fn staged_discipline_reuse_blocks_both_actions_but_same_linked_timer_does_not() {
        let mut world = World::new(1);
        let mut player = testing::player(7);
        player.class = Some(1);
        player.level = 55;
        player.disciplines = Some(vec![4499, 4503]);
        world.player.admit(player);
        let mut casting = Casting::with_titanium_combat(false, Duration::from_mins(72));
        let command = |discipline: Discipline| use_spell(discipline.spell(), 7);
        let timer = |remaining| Reuse {
            timer: 1,
            remaining,
        };
        let running = timer(Duration::from_secs(20));
        casting
            .admit(
                &Message::Event(WorldEvent::DisciplineReuse(running)),
                &mut world,
            )
            .unwrap();
        casting
            .admit(
                &Message::Event(WorldEvent::LinkedReuse(running)),
                &mut world,
            )
            .unwrap();
        for discipline in Discipline::SUPPORTED {
            let report = testing::run(|out| casting.handle(&command(discipline), &mut world, out));
            report.result.unwrap();
            assert_eq!(report.sent, []);
        }
        let admission = testing::run(|out| casting.admitted(&mut world, out));
        admission.result.unwrap();
        assert!(matches!(
            admission.events.as_slice(),
            [
                ClientEvent::World(WorldEvent::DisciplineReuse(_)),
                ClientEvent::World(WorldEvent::LinkedReuse(_))
            ]
        ));
        let reset = Message::Event(WorldEvent::DisciplineReuse(timer(Duration::ZERO)));
        testing::run(|out| casting.observe(&reset, &mut world, out))
            .result
            .unwrap();
        let report =
            testing::run(|out| casting.handle(&command(Discipline::Defensive), &mut world, out));
        report.result.unwrap();
        assert_eq!(report.sent.len(), 1);
        assert!(matches!(
            report.events.as_slice(),
            [ClientEvent::World(WorldEvent::CastPending {
                spell_id: Some(4499),
                ..
            })]
        ));
    }
}
