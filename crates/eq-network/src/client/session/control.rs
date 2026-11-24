//! Own-player stun interpretation. Titanium's opcode arm (0x45cc38) calls
//! 0x417839 even for zero: an idle ordinary player gets at least 1,000 ms,
//! and an active timer ignores overlapping requests. The play loop at
//! 0x422198 expires it strictly after its deadline. P99's receive dispatcher
//! passes this opcode through; stock `EQEmu` sends the same Titanium duration.
//! This is not TAKP, charm, fear or mez policy.
//!
//! The program's own hit roll ([`super::hit_roll`]) stuns through the same
//! slot: the first stun stands, the hit's or the server's (R49-50b 4).
use super::{
    feature::{Feature, Out, World},
    hit_roll::{HitRoll, AVOIDED_LINE, HIT_STUN, SHOOK_OFF_LINE},
    ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    control::StunState, message::Message, movement::MovementRequest, world::WorldEvent,
};
use std::time::{Duration, Instant};

/// Facts owned by one zone admission, never by host input or a buff guess.
#[derive(Default)]
pub(super) struct OwnControl {
    deadline: Option<Instant>,
    revision: u64,
    /// Commands queued before a transition must not replay after release.
    changed: Option<Instant>,
    /// The program's own roll on a bash or a kick.
    pub(super) hit: super::hit_roll::HitRoller,
}

impl OwnControl {
    pub(super) const fn stunned(&self) -> bool {
        self.deadline.is_some()
    }

    /// When the running stun ends.
    #[cfg(test)]
    pub(super) const fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    fn state(&self) -> StunState {
        if self.stunned() {
            StunState::Stunned
        } else {
            StunState::Clear
        }
    }

    fn transition(&mut self, now: Instant) {
        self.revision += 1;
        self.changed = Some(now);
    }

    /// Native overlap is ignored, including zero (0x417839 returns while
    /// the actor's +0xd0 expiry is set: no extension; R49-6b); zero while
    /// idle is a minimum-duration stun, not an explicit release.
    fn receive(&mut self, duration: Option<Duration>, now: Instant) -> bool {
        if self.stunned() {
            return false;
        }
        let duration = duration.unwrap_or_default().max(Duration::from_secs(1));
        self.deadline = now.checked_add(duration);
        if self.deadline.is_none() {
            return false;
        }
        self.transition(now);
        true
    }

    fn expire(&mut self, now: Instant) -> bool {
        if self.deadline.is_some_and(|deadline| now > deadline) {
            self.deadline = None;
            self.transition(now);
            true
        } else {
            false
        }
    }

    pub(super) fn input_refusal(&self, created: Instant) -> Option<&'static str> {
        if self.changed.is_some_and(|changed| created <= changed) {
            Some("Player input predates the current stun state")
        } else if self.stunned() {
            Some("The player is stunned")
        } else {
            None
        }
    }

    /// Explicit opt-in skips only the active stun's motion restriction.
    /// Input queued before a stun transition remains stale in either mode.
    pub(super) fn motion_refusal(
        &self,
        request: &MovementRequest,
        passive: bool,
        allow_stunned: bool,
    ) -> Option<&'static str> {
        let refusal = self.input_refusal(request.created)?;
        if self.stunned()
            && self.changed.is_none_or(|changed| request.created > changed)
            && (passive || allow_stunned)
        {
            None
        } else {
            Some(refusal)
        }
    }

    fn event(&self, session_id: u64) -> WorldEvent {
        WorldEvent::OwnStun {
            session_id,
            revision: self.revision,
            state: self.state(),
        }
    }
}

/// Whether a generation's program rolls its own stun and interrupt on a
/// hit: the Titanium program's 0x41c905 (read R49-50). The others are
/// unread and roll nothing.
const fn rolls_on_hits(dialect: eq_network_game::GameDialect) -> bool {
    match dialect {
        eq_network_game::GameDialect::Titanium => true,
        eq_network_game::GameDialect::EqMac | eq_network_game::GameDialect::Rof2 => false,
    }
}

/// Enabled only by server policies with the verified Titanium handler.
pub(super) struct Stuns;

impl Stuns {
    fn publish(world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if world.control.stunned() {
            world.body.stop_voluntary();
        }
        out.log
            .send(ClientEvent::World(world.control.event(world.session_id)))
    }

    pub(super) fn observe_at(
        message: &Message,
        now: Instant,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if world.control.expire(now) {
            Self::publish(world, out)?;
        }
        match message {
            Message::Event(WorldEvent::Stun(duration)) => Self::stun(*duration, now, world, out)?,
            Message::Event(WorldEvent::Damage(damage)) => {
                Self::hit(damage, now, world, out)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// A stun into the one slot: 0x417839 ignores it while effect 40's total
    /// is positive (divine aura; R49-6b), as far as the host's catalog
    /// knows, and while a stun runs.
    fn stun(
        duration: Option<Duration>,
        now: Instant,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if !world.sight.divine_aura() && world.control.receive(duration, now) {
            Self::publish(world, out)?;
        }
        Ok(())
    }

    /// The Titanium program's own roll on a hit (0x41c905, read R49-50):
    /// its stun branch stuns or says a save, and its interrupt branch has
    /// the casting feature clear what is held. Nothing is sent. What the
    /// roll came to, where it rolled.
    pub(super) fn hit(
        damage: &eq_network_game::combat::Damage,
        now: Instant,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<Option<HitRoll>> {
        if !rolls_on_hits(out.wire.dialect()) {
            return Ok(None);
        }
        let roll =
            super::hit_roll::facts(damage, world).map(|facts| world.control.hit.roll(&facts));
        match roll {
            Some(HitRoll::Stunned) => Self::stun(Some(HIT_STUN), now, world, out)?,
            Some(HitRoll::Avoided | HitRoll::ShookOff) => {
                let string_id = if roll == Some(HitRoll::Avoided) {
                    AVOIDED_LINE
                } else {
                    SHOOK_OFF_LINE
                };
                out.log.send(ClientEvent::World(WorldEvent::StunSaved {
                    session_id: world.session_id,
                    string_id,
                }))?;
            }
            // The casting feature clears what is held (6.645 S2).
            Some(HitRoll::Interrupt) => world.happened(Message::HitInterrupt),
            Some(HitRoll::Kept) | None => {}
        }
        Ok(roll)
    }
}

impl Feature for Stuns {
    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        let now = Instant::now();
        world.control.expire(now);
        if let Message::Event(WorldEvent::Stun(duration)) = message {
            if !world.sight.divine_aura() {
                world.control.receive(*duration, now);
            }
        }
        Ok(())
    }

    fn admitted(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        world.control.expire(Instant::now());
        Self::publish(world, out)
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        Self::observe_at(message, Instant::now(), world, out)
    }

    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if world.control.expire(now) {
            Self::publish(world, out)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::session::feature::testing;

    #[test]
    fn a_failed_host_callback_does_not_replay_the_stun_or_generate_traffic() {
        let start = Instant::now();
        let mut world = World::new(9);
        let message = Message::Event(WorldEvent::Stun(None));
        let output = testing::run(|out| {
            let config =
                crate::client::ClientConfig::new("ACCOUNT", "PASSWORD", "Test Server", "Tester");
            let mut handler = |_| anyhow::bail!("synthetic event callback failure");
            let mut log = crate::client::Events::new(&config, &mut handler);
            let mut failing = Out {
                sink: out.sink,
                log: &mut log,
                wire: out.wire,
                sender: out.sender,
            };
            Stuns::observe_at(&message, start, &mut world, &mut failing)
        });
        assert!(output.result.is_err());
        assert!(output.sent.is_empty() && output.unreliable.is_empty());
        assert!(world.control.stunned());
        let output = testing::run(|out| {
            Stuns::observe_at(
                &message,
                start + Duration::from_millis(100),
                &mut world,
                out,
            )
        });
        output.result.unwrap();
        assert!(output.events.is_empty() && output.sent.is_empty() && output.unreliable.is_empty());
        assert_eq!(world.control.revision, 1);
    }

    #[test]
    fn zero_and_short_stuns_have_a_one_second_floor_and_expire_strictly_afterward() {
        let start = Instant::now();
        for duration in [
            None,
            Some(Duration::from_millis(1)),
            Some(Duration::from_millis(999)),
        ] {
            let mut control = OwnControl::default();
            assert!(control.receive(duration, start));
            assert_eq!(control.state(), StunState::Stunned);
            assert!(!control.expire(start + Duration::from_secs(1)));
            assert!(control.expire(start + Duration::from_millis(1001)));
            assert_eq!(control.state(), StunState::Clear);
            assert_eq!(control.revision, 2);
            assert!(!control.expire(start + Duration::from_secs(10)));
        }
    }

    #[test]
    fn a_stun_under_divine_aura_is_ignored() {
        // 0x417839 returns while effect 40's total is positive (R49-6b).
        #[derive(Debug)]
        struct Catalog;
        impl eq_network_game::vision::SightCatalog for Catalog {
            fn sees_invisible(&self, _spell_id: u32, _caster_level: u16, _landed: bool) -> bool {
                false
            }
            fn grants_divine_aura(&self, spell_id: u32, _caster_level: u16, _landed: bool) -> bool {
                spell_id == 99
            }
        }
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.sight = super::super::sight::Sight::new(Some(std::sync::Arc::new(Catalog)));
        let aura = eq_network_game::buffs::Buff {
            spell_id: 99,
            caster_level: 20,
            effect_type: 2,
            bard_modifier: 10,
            duration_ticks: 3,
            counters: 0,
            caster_id: 7,
        };
        world
            .sight
            .apply(&WorldEvent::BuffSnapshot(vec![Some(aura)]), Some(7));
        let stun = Message::Event(WorldEvent::Stun(Some(Duration::from_secs(3))));
        let now = Instant::now();
        let heard = testing::run(|out| Stuns::observe_at(&stun, now, &mut world, out));
        assert!(heard.events.is_empty() && !world.control.stunned());
        // Without it, the same stun holds.
        world
            .sight
            .apply(&WorldEvent::BuffSnapshot(vec![None]), Some(7));
        testing::run(|out| Stuns::observe_at(&stun, now, &mut world, out));
        assert!(world.control.stunned());
    }

    #[test]
    fn overlap_does_not_shorten_extend_or_release_the_active_timer() {
        let start = Instant::now();
        let mut control = OwnControl::default();
        assert!(control.receive(Some(Duration::from_secs(3)), start));
        for duration in [
            None,
            Some(Duration::from_millis(10)),
            Some(Duration::from_secs(90)),
        ] {
            assert!(!control.receive(duration, start + Duration::from_secs(1)));
            assert_eq!(control.revision, 1);
        }
        assert!(!control.expire(start + Duration::from_secs(3)));
        assert!(control.expire(start + Duration::from_millis(3001)));
        // A fresh idle zero is another minimum-duration stun, never a release.
        assert!(control.receive(None, start + Duration::from_secs(4)));
        assert!(!control.expire(start + Duration::from_secs(5)));
        assert!(control.expire(start + Duration::from_millis(5001)));
    }

    #[test]
    fn expiry_and_new_admission_do_not_replay_old_input_or_other_control_events() {
        let start = Instant::now();
        let mut world = World::new(9);
        world.admitted = Some(start);
        let output = testing::run(|out| {
            Stuns::observe_at(
                &Message::Event(WorldEvent::Stun(Some(Duration::from_secs(2)))),
                start,
                &mut world,
                out,
            )
        });
        output.result.unwrap();
        assert!(matches!(
            output.events[..],
            [ClientEvent::World(WorldEvent::OwnStun {
                session_id: 9,
                revision: 1,
                state: StunState::Stunned,
            })]
        ));
        for event in [
            WorldEvent::Berserk(true),
            WorldEvent::Controlled(false),
            WorldEvent::Charm(eq_network_game::control::Charm {
                owner_id: None,
                pet_id: 55,
                charmed: false,
            }),
        ] {
            let output = testing::run(|out| {
                Stuns::observe_at(
                    &Message::Event(event),
                    start + Duration::from_secs(1),
                    &mut world,
                    out,
                )
            });
            output.result.unwrap();
            assert!(output.events.is_empty());
            assert!(world.control.stunned());
        }
        let end = start + Duration::from_millis(2001);
        let output = testing::run(|out| Stuns.tick(end, &mut world, out));
        output.result.unwrap();
        assert!(matches!(
            output.events[..],
            [ClientEvent::World(WorldEvent::OwnStun {
                session_id: 9,
                revision: 2,
                state: StunState::Clear,
            })]
        ));
        assert!(world
            .control
            .input_refusal(start + Duration::from_secs(1))
            .is_some());
        assert!(world.control.input_refusal(end).is_some());
        assert!(world
            .control
            .input_refusal(end + Duration::from_millis(1))
            .is_none());
        let next = World::new(10);
        assert!(!next.control.stunned());
        assert_eq!(next.control.revision, 0);
        assert!(next.control.input_refusal(start).is_none());
    }
}
