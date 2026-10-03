//! The server's regeneration tick, for a front end to show. No packet names
//! a tick, so the session learns it from the packets the server sends at
//! one, which differ by server type: a server type gives the feature its
//! [`Marks`], and a [`TickClock`] keeps to them. The feature starts afresh
//! in each zone, as the server's timer does.
use super::{
    feature::{Feature, Out, World},
    ClientEvent,
};
use anyhow::Result;
use eq_network_game::{message::Message, ticks::Cadence, world::WorldEvent};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// How far a mark may land from where a tick is due and still be taken for
/// it: the packet's way to the session, and the server's loop running late.
const JITTER: Duration = Duration::from_millis(100);
/// The most ticks a mark may follow the last tick by and still be taken for
/// one, while the slips between them add up.
const BRIDGE: u32 = 10;
/// The most ticks apart two marks after the last tick may land and still
/// show a tick between them.
const CONFIRM: u32 = 3;

/// Which of a server's packets may mark its regeneration tick, and how
/// often the tick comes.
pub(super) trait Marks {
    /// How often the server's tick comes.
    fn cadence(&self) -> Cadence;

    /// Learns what the admitted player starts with.
    fn admitted(&mut self, _world: &World) {}

    /// Hears a message: true when it may mark a tick.
    fn marks(&mut self, message: &Message, world: &World) -> bool;
}

/// `EQEmu`'s tick: six seconds, checked on a zone loop that runs every 32 ms.
const EQEMU: Cadence = Cadence {
    period_ms: 6000,
    slip_ms: 32,
};

/// `EQEmu`'s marks: the player's HP, mana or endurance rising. Each tick
/// regenerates all three, and the server reports each only when it changed
/// (`Mob::SendHPUpdate`, `Client::CheckManaEndUpdate`), so a tick shows
/// while any of them regenerates and nothing shows at full. Damage, casting
/// and a discipline's upkeep lower them, as does the two-second catch-up
/// that reports damage over time, so only a rise counts. A heal lands at
/// any moment; the clock passes over one that keeps to no tick.
#[derive(Default)]
pub(super) struct Regeneration {
    hp: Option<i32>,
    mana: Option<u32>,
    endurance: Option<u32>,
}

impl Marks for Regeneration {
    fn cadence(&self) -> Cadence {
        EQEMU
    }

    fn admitted(&mut self, world: &World) {
        if let Some(player) = world.player.as_ref() {
            self.mana = Some(player.mana);
            self.endurance = player.endurance;
        }
    }

    fn marks(&mut self, message: &Message, world: &World) -> bool {
        match message {
            Message::Event(WorldEvent::HitPoints {
                spawn_id, current, ..
            }) if world.is_player(*spawn_id) => rose(&mut self.hp, *current),
            Message::Event(WorldEvent::Resources { mana, endurance }) => {
                // Both, so that neither's last value goes stale.
                let mana = rose(&mut self.mana, *mana);
                rose(&mut self.endurance, *endurance) || mana
            }
            _ => false,
        }
    }
}

/// Keeps the latest value: true when it rose above the one before.
fn rose<T: PartialOrd + Copy>(last: &mut Option<T>, value: T) -> bool {
    let rose = last.is_some_and(|last| value > last);
    *last = Some(value);
    rose
}

/// Learns when the tick lands from marks, some of which the server sends at
/// a tick and some at other moments. A mark is taken for a tick once another
/// lands a whole number of ticks after it. From then on each mark that keeps
/// to the ticks moves the clock to it, and two after it that keep to each
/// other but not to the clock start it again from the later one.
#[derive(Default)]
struct TickClock {
    /// When the last mark taken for a tick arrived.
    last: Option<Instant>,
    /// The marks since then that kept to no tick, oldest first.
    loose: VecDeque<Instant>,
}

impl TickClock {
    /// Hears a mark that arrived at `at`: true when it is taken for a tick.
    fn mark(&mut self, at: Instant, cadence: Cadence) -> bool {
        if let Some(last) = self.last {
            // The same tick's other marks.
            if at.saturating_duration_since(last) < JITTER {
                return false;
            }
            if keeps_to(cadence, last, at, BRIDGE) {
                return self.take(at);
            }
        }
        if self
            .loose
            .iter()
            .any(|&earlier| keeps_to(cadence, earlier, at, CONFIRM))
        {
            return self.take(at);
        }
        if self
            .loose
            .back()
            .is_none_or(|&back| at.saturating_duration_since(back) >= JITTER)
        {
            self.loose.push_back(at);
        }
        let reach = (cadence.period() + cadence.slip()) * CONFIRM + JITTER;
        while self
            .loose
            .front()
            .is_some_and(|&front| at.saturating_duration_since(front) > reach)
        {
            self.loose.pop_front();
        }
        false
    }

    /// Takes a mark for a tick.
    fn take(&mut self, at: Instant) -> bool {
        self.last = Some(at);
        self.loose.clear();
        true
    }
}

/// Whether `at` lands a whole number of ticks after `from`, at most `most`
/// of them, each one period and up to the slip long.
fn keeps_to(cadence: Cadence, from: Instant, at: Instant, most: u32) -> bool {
    let since = at.saturating_duration_since(from);
    (1..=most).any(|ticks| {
        since + JITTER >= cadence.period() * ticks
            && since <= (cadence.period() + cadence.slip()) * ticks + JITTER
    })
}

/// Learns the server's regeneration tick and tells the host of each one.
pub(super) struct Ticks {
    marks: Box<dyn Marks>,
    clock: TickClock,
}

impl Ticks {
    /// A tick that the server type's marks show.
    pub(super) fn new(marks: Box<dyn Marks>) -> Self {
        Self {
            marks,
            clock: TickClock::default(),
        }
    }

    /// Hears a message that arrived at `now`: the news of a tick, when it
    /// marks one.
    fn hear(&mut self, message: &Message, world: &World, now: Instant) -> Option<WorldEvent> {
        let cadence = self.marks.cadence();
        (self.marks.marks(message, world) && self.clock.mark(now, cadence))
            .then_some(WorldEvent::ServerTick(cadence))
    }
}

impl Feature for Ticks {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::ServerTicks]
    }

    fn admitted(&mut self, world: &mut World, _out: &mut Out<'_, '_>) -> Result<()> {
        self.marks.admitted(world);
        Ok(())
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match self.hear(message, world, Instant::now()) {
            Some(tick) => out.log.send(ClientEvent::World(tick)),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;

    /// The player's HP report.
    fn hp(current: i32) -> Message {
        Message::Event(WorldEvent::HitPoints {
            spawn_id: 7,
            current,
            maximum: 100,
            without_items: true,
        })
    }

    /// The player's mana and endurance report.
    fn resources(mana: u32, endurance: u32) -> Message {
        Message::Event(WorldEvent::Resources { mana, endurance })
    }

    /// A world that admitted the player as spawn 7.
    fn admitted() -> World {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        world
    }

    /// Milliseconds after a start.
    fn at(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    #[test]
    fn only_the_players_rising_hp_mana_or_endurance_may_mark_a_tick() {
        let world = admitted();
        let mut marks = Regeneration::default();
        marks.admitted(&world);
        // The first HP report only gives the value it rises from.
        assert!(!marks.marks(&hp(50), &world));
        assert!(marks.marks(&hp(52), &world));
        // Damage, and the catch-up that reports damage over time.
        assert!(!marks.marks(&hp(40), &world));
        assert!(!marks.marks(&hp(40), &world));
        // Another spawn's HP is not the player's regeneration.
        let other = Message::Event(WorldEvent::HitPoints {
            spawn_id: 9,
            current: 90,
            maximum: 100,
            without_items: true,
        });
        assert!(!marks.marks(&other, &world));
        // The admission gives mana and endurance to rise from: a zeroed
        // profile starts with none of either.
        assert!(marks.marks(&resources(3, 0), &world));
        assert!(marks.marks(&resources(3, 2), &world));
        // A cast or a discipline's upkeep lowers them.
        assert!(!marks.marks(&resources(1, 2), &world));
        assert!(!marks.marks(&resources(1, 1), &world));
        // A rise in either is a mark, though the other fell.
        assert!(marks.marks(&resources(4, 0), &world));
        assert!(!marks.marks(&Message::LoggedOut, &world));
    }

    #[test]
    fn a_mark_is_a_tick_once_another_lands_a_tick_later() {
        let mut clock = TickClock::default();
        let start = Instant::now();
        // The first tick's HP and mana arrive together; nothing is known yet.
        assert!(!clock.mark(start, EQEMU));
        assert!(!clock.mark(at(start, 1), EQEMU));
        // A heal between ticks.
        assert!(!clock.mark(at(start, 2_500), EQEMU));
        // The next tick, one period and part of a pass later.
        assert!(clock.mark(at(start, 6_016), EQEMU));
        // Its mana arrives with it.
        assert!(!clock.mark(at(start, 6_017), EQEMU));
        // Each tick after keeps to the clock, slipping up to a pass each.
        assert!(clock.mark(at(start, 12_048), EQEMU));
        assert!(clock.mark(at(start, 18_049), EQEMU));
        // A heal between them does not.
        assert!(!clock.mark(at(start, 20_000), EQEMU));
        assert!(clock.mark(at(start, 24_070), EQEMU));
    }

    #[test]
    fn the_clock_bridges_silent_ticks_and_starts_again_when_marks_leave_it() {
        let mut clock = TickClock::default();
        let start = Instant::now();
        assert!(!clock.mark(start, EQEMU));
        assert!(clock.mark(at(start, 6_016), EQEMU));
        // At full, five ticks pass silently, then regeneration shows again.
        assert!(clock.mark(at(start, 6_016 + 5 * 6_016), EQEMU));
        // A mark long after the last tick is no tick by itself...
        let late = 6_016 + 5 * 6_016 + 20 * 6_016 + 3_000;
        assert!(!clock.mark(at(start, late), EQEMU));
        // ...but two a tick apart start the clock again from the later.
        assert!(clock.mark(at(start, late + 6_016), EQEMU));
        assert!(clock.mark(at(start, late + 12_032), EQEMU));
    }

    #[test]
    fn two_marks_that_keep_to_each_other_but_not_to_the_clock_move_it() {
        let mut clock = TickClock::default();
        let start = Instant::now();
        // Two heals happen to land a tick apart, and are taken for ticks.
        assert!(!clock.mark(start, EQEMU));
        assert!(clock.mark(at(start, 6_000), EQEMU));
        // The real ticks keep to none of them, until two keep to each other.
        assert!(!clock.mark(at(start, 9_000), EQEMU));
        assert!(clock.mark(at(start, 15_016), EQEMU));
        assert!(clock.mark(at(start, 21_032), EQEMU));
    }

    #[test]
    fn a_tick_is_told_once_its_marks_show_it() {
        let world = admitted();
        let mut ticks = Ticks::new(Box::<Regeneration>::default());
        let start = Instant::now();
        assert!(ticks.hear(&hp(50), &world, start).is_none());
        assert!(ticks.hear(&hp(52), &world, at(start, 6_000)).is_none());
        // Damage a tick later marks nothing.
        assert!(ticks.hear(&hp(30), &world, at(start, 12_016)).is_none());
        assert_eq!(
            ticks.hear(&hp(33), &world, at(start, 12_032)),
            Some(WorldEvent::ServerTick(EQEMU))
        );
        assert_eq!(
            ticks.capabilities(),
            [crate::world::Capability::ServerTicks]
        );
        let outcome = testing::run(|out| ticks.observe(&hp(20), &mut admitted(), out));
        outcome.result.unwrap();
        assert!(outcome.events.is_empty() && outcome.sent.is_empty());
    }
}
