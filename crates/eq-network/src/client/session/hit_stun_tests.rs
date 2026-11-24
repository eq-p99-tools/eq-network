//! The Titanium program's own roll when a bash or a kick hits the player
//! (0x41c905, reads R49-50 and R49-50b; checklist 6.646).
use super::{
    control::Stuns,
    feature::{testing, World},
    hit_roll::{HitDice, HitRoll, HitRoller, AVOIDED_LINE, HIT_STUN, SHOOK_OFF_LINE},
    wire::{EqMac, Rof2, Wire},
    ClientEvent,
};
use eq_network_game::{
    buffs::Buff,
    combat::{Damage, DamageOutcome},
    control::StunState,
    message::Message,
    vision::SightCatalog,
    world::{SpawnKind, WorldEvent},
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const PLAYER: u16 = 7;
const BASHER: u16 = 40;
const KICKER: u16 = 41;
const BASH: u8 = 10;
const KICK: u8 = 30;

/// The numbers still to give, and each roll's range so far.
type Rolls = (VecDeque<i32>, Vec<(i32, i32)>);

/// Dice that give these numbers in turn and keep each roll's range.
#[derive(Clone, Default)]
struct Script(Arc<Mutex<Rolls>>);

impl Script {
    fn of(rolls: &[i32]) -> Self {
        let script = Self::default();
        script.0.lock().unwrap().0 = rolls.iter().copied().collect();
        script
    }

    fn ranges(&self) -> Vec<(i32, i32)> {
        self.0.lock().unwrap().1.clone()
    }
}

impl HitDice for Script {
    fn roll(&mut self, low: i32, high: i32) -> i32 {
        let mut script = self.0.lock().unwrap();
        script.1.push((low, high));
        script.0.pop_front().expect("a scripted roll")
    }
}

/// A catalog whose spell 90 gives each SPA its listed value.
#[derive(Debug)]
struct Totals(Vec<(u16, i32)>);

impl SightCatalog for Totals {
    fn sees_invisible(&self, _spell_id: u32, _caster_level: u16, _landed: bool) -> bool {
        false
    }

    fn effect_total(&self, spell_id: u32, _caster_level: u16, _landed: bool, spa: u16) -> i32 {
        self.0
            .iter()
            .find(|(listed, _)| spell_id == 90 && *listed == spa)
            .map_or(0, |(_, value)| *value)
    }
}

/// The player as spawn 7, a player spawn 40 who can bash and a level 55
/// Warrior NPC 41 who can kick.
fn fighting(dice: &Script) -> World {
    let mut world = World::new(5);
    world.own_spawn = Some(PLAYER);
    world.player.admit(testing::player(PLAYER));
    world
        .spawns
        .insert(testing::spawn(BASHER, SpawnKind::Player));
    let mut kicker = testing::spawn(KICKER, SpawnKind::Npc);
    kicker.class = Some(1);
    kicker.level = 55;
    world.spawns.insert(kicker);
    world.control.hit = HitRoller::with_dice(dice.clone());
    world
}

fn with_totals(world: &mut World, totals: Vec<(u16, i32)>) {
    world.sight = super::sight::Sight::new(Some(Arc::new(Totals(totals))));
    let buff = Buff {
        spell_id: 90,
        caster_level: 20,
        effect_type: 2,
        bard_modifier: 10,
        duration_ticks: 3,
        counters: 0,
        caster_id: u32::from(PLAYER),
    };
    world
        .sight
        .apply(&WorldEvent::BuffSnapshot(vec![Some(buff)]), Some(PLAYER));
}

fn hit(source_id: u16, kind: u8, amount: u32) -> Damage {
    Damage {
        target_id: PLAYER,
        source_id,
        kind,
        spell_id: None,
        outcome: DamageOutcome::Hit(amount),
        push: None,
    }
}

/// What the roll came to on this wire, and what it sent and said.
fn roll_on(
    wire: &'static dyn Wire,
    damage: &Damage,
    now: Instant,
    world: &mut World,
) -> testing::Outcome<Option<HitRoll>> {
    let outcome = testing::run_on(wire, |out| Stuns::hit(damage, now, world, out));
    let outcome = testing::Outcome {
        result: outcome.result.unwrap(),
        sent: outcome.sent,
        unreliable: outcome.unreliable,
        events: outcome.events,
        heard: outcome.heard,
    };
    // The program sends nothing for its roll, whatever it comes to.
    assert!(outcome.sent.is_empty() && outcome.unreliable.is_empty());
    outcome
}

fn roll(damage: &Damage, world: &mut World) -> testing::Outcome<Option<HitRoll>> {
    roll_on(&super::wire::Titanium, damage, Instant::now(), world)
}

#[test]
fn the_first_roll_takes_the_interrupt_branch_at_or_under_55_and_under_99() {
    // r1 = random[0, 100], 0 counted as 1; r1 < 99 and r1 <= 55 - t take
    // the interrupt branch, where r2 = random[0, 9] over 1 interrupts.
    for (rolls, expected) in [
        (vec![0, 2], HitRoll::Interrupt),
        (vec![55, 2], HitRoll::Interrupt),
        (vec![55, 1], HitRoll::Kept),
        (vec![1, 1], HitRoll::Kept),
        (vec![56, 100, 100], HitRoll::Stunned),
        (vec![98, 100, 100], HitRoll::Stunned),
        (vec![99, 100, 100], HitRoll::Stunned),
        (vec![100, 100, 100], HitRoll::Stunned),
    ] {
        let dice = Script::of(&rolls);
        let mut world = fighting(&dice);
        assert_eq!(
            roll(&hit(KICKER, KICK, 10), &mut world).result,
            Some(expected),
            "{rolls:?}"
        );
        let mut ranges = vec![(0, 100)];
        ranges.extend(match expected {
            HitRoll::Interrupt | HitRoll::Kept => vec![(0, 9)],
            _ => vec![(1, 100), (0, 100)],
        });
        assert_eq!(dice.ranges(), ranges, "{rolls:?}");
    }
}

#[test]
fn a_zero_first_roll_counts_as_one_against_a_lowered_threshold() {
    // A bash's stun-bash chance (SPA 0xe7) of 54 leaves a threshold of 1:
    // 0 counts as 1 and takes the branch, 2 does not.
    for (first, interrupt) in [(0, true), (1, true), (2, false)] {
        let dice = Script::of(&[first, 2, 100, 100]);
        let mut world = fighting(&dice);
        with_totals(&mut world, vec![(0xe7, 54)]);
        let rolled = roll(&hit(BASHER, BASH, 10), &mut world).result;
        assert_eq!(rolled == Some(HitRoll::Interrupt), interrupt, "{first}");
    }
}

#[test]
fn the_stun_bash_chance_lowers_a_bash_only() {
    // At 40 the kick still takes the branch at 55; the bash needs 15.
    let dice = Script::of(&[55, 2]);
    let mut world = fighting(&dice);
    with_totals(&mut world, vec![(0xe7, 40)]);
    assert_eq!(
        roll(&hit(KICKER, KICK, 10), &mut world).result,
        Some(HitRoll::Interrupt)
    );
    let dice = Script::of(&[16, 100, 100]);
    let mut world = fighting(&dice);
    with_totals(&mut world, vec![(0xe7, 40)]);
    assert_eq!(
        roll(&hit(BASHER, BASH, 10), &mut world).result,
        Some(HitRoll::Stunned)
    );
}

#[test]
fn the_interrupt_branch_has_the_casting_feature_clear_what_is_held() {
    // The session's own news, which the casting feature hears next
    // (6.645 S2); the host hears nothing of the roll itself.
    let dice = Script::of(&[10, 9]);
    let mut world = fighting(&dice);
    let heard = roll(&hit(KICKER, KICK, 10), &mut world);
    assert_eq!(heard.result, Some(HitRoll::Interrupt));
    assert!(heard.events.is_empty() && !world.control.stunned());
    assert!(matches!(world.take_news()[..], [Message::HitInterrupt]));
    // A kept roll makes no news.
    let dice = Script::of(&[10, 1]);
    let mut world = fighting(&dice);
    assert_eq!(
        roll(&hit(KICKER, KICK, 10), &mut world).result,
        Some(HitRoll::Kept)
    );
    assert!(world.take_news().is_empty());
}

#[test]
fn the_stun_branch_stuns_for_2000_ms_unless_a_resist_saves() {
    let now = Instant::now();
    let dice = Script::of(&[60, 100, 100]);
    let mut world = fighting(&dice);
    let heard = roll_on(
        &super::wire::Titanium,
        &hit(KICKER, KICK, 10),
        now,
        &mut world,
    );
    assert_eq!(heard.result, Some(HitRoll::Stunned));
    assert!(matches!(
        heard.events[..],
        [ClientEvent::World(WorldEvent::OwnStun {
            session_id: 5,
            revision: 1,
            state: StunState::Stunned,
        })]
    ));
    assert_eq!(world.control.deadline(), now.checked_add(HIT_STUN));
    // The frontal stun resist (SPA 0x125) saves at or above random[1, 100];
    // the stun resist (SPA 0xc3) when random[0, 100] is under it.
    for (rolls, totals, expected, line) in [
        (
            vec![60, 30],
            vec![(0x125, 30)],
            HitRoll::Avoided,
            AVOIDED_LINE,
        ),
        (
            vec![60, 31, 19],
            vec![(0x125, 30), (0xc3, 20)],
            HitRoll::ShookOff,
            SHOOK_OFF_LINE,
        ),
    ] {
        let dice = Script::of(&rolls);
        let mut world = fighting(&dice);
        with_totals(&mut world, totals);
        let heard = roll(&hit(KICKER, KICK, 10), &mut world);
        assert_eq!(heard.result, Some(expected));
        assert!(matches!(
            heard.events[..],
            [ClientEvent::World(WorldEvent::StunSaved { session_id: 5, string_id })]
                if string_id == line
        ));
        assert!(!world.control.stunned());
    }
    // At the stun resist's value, no save.
    let dice = Script::of(&[60, 31, 20]);
    let mut world = fighting(&dice);
    with_totals(&mut world, vec![(0x125, 30), (0xc3, 20)]);
    assert_eq!(
        roll(&hit(KICKER, KICK, 10), &mut world).result,
        Some(HitRoll::Stunned)
    );
}

#[test]
fn the_first_stun_stands_the_hits_or_the_servers() {
    let start = Instant::now();
    // The hit's first: a server stun during it moves nothing.
    let dice = Script::of(&[60, 100, 100]);
    let mut world = fighting(&dice);
    roll_on(
        &super::wire::Titanium,
        &hit(KICKER, KICK, 10),
        start,
        &mut world,
    );
    let deadline = world.control.deadline();
    let server = Message::Event(WorldEvent::Stun(Some(Duration::from_secs(5))));
    let later = start + Duration::from_millis(500);
    let heard = testing::run(|out| Stuns::observe_at(&server, later, &mut world, out));
    assert!(heard.events.is_empty());
    assert_eq!(world.control.deadline(), deadline);
    // The server's first: the hit's stun during it moves nothing.
    let dice = Script::of(&[60, 100, 100]);
    let mut world = fighting(&dice);
    testing::run(|out| Stuns::observe_at(&server, start, &mut world, out));
    let deadline = world.control.deadline();
    let heard = roll_on(
        &super::wire::Titanium,
        &hit(KICKER, KICK, 10),
        later,
        &mut world,
    );
    assert_eq!(heard.result, Some(HitRoll::Stunned));
    assert!(heard.events.is_empty());
    assert_eq!(world.control.deadline(), deadline);
    assert_eq!(deadline, start.checked_add(Duration::from_secs(5)));
}

#[test]
fn only_a_players_bash_or_a_warrior_over_54s_kick_rolls() {
    let dice = Script::default();
    let mut world = fighting(&dice);
    // A bash from an NPC, a kick from a player who isn't a Warrior over 54,
    // another kind, a miss, no damage, and a hit to someone else.
    let mut level_54 = testing::spawn(42, SpawnKind::Npc);
    level_54.class = Some(1);
    level_54.level = 54;
    world.spawns.insert(level_54);
    let mut rogue = testing::spawn(43, SpawnKind::Npc);
    rogue.class = Some(9);
    rogue.level = 60;
    world.spawns.insert(rogue);
    let elsewhere = Damage {
        target_id: 99,
        ..hit(KICKER, KICK, 10)
    };
    let missed = Damage {
        outcome: DamageOutcome::Miss,
        ..hit(KICKER, KICK, 10)
    };
    for damage in [
        hit(KICKER, BASH, 10),
        hit(BASHER, KICK, 10),
        hit(42, KICK, 10),
        hit(43, KICK, 10),
        hit(KICKER, 0, 10),
        hit(KICKER, KICK, 0),
        hit(77, KICK, 10),
        missed,
        elsewhere,
    ] {
        assert_eq!(roll(&damage, &mut world).result, None, "{damage:?}");
    }
    assert_eq!(dice.ranges(), []);
}

#[test]
fn an_ogre_or_an_oggok_citizen_never_rolls() {
    for race in [10, 93] {
        let dice = Script::default();
        let mut world = fighting(&dice);
        let mut player = testing::player(PLAYER);
        player.race = race;
        world.player.admit(player);
        assert_eq!(roll(&hit(KICKER, KICK, 10), &mut world).result, None);
    }
}

#[test]
fn a_hit_rolls_whatever_the_hp_report_says() {
    // The program rolls only when the hit leaves the player alive, but the
    // report leaves out the items' HP and no Titanium item count exists:
    // a hit the report calls lethal rolls too (inferred, a gap row).
    let dice = Script::of(&[60, 100, 100]);
    let mut world = fighting(&dice);
    let update = Message::Event(WorldEvent::HitPoints {
        spawn_id: PLAYER,
        current: 10,
        maximum: 100,
        items: eq_network_game::world::ItemHitPoints::LeftOut,
    });
    testing::run(|out| Stuns::observe_at(&update, Instant::now(), &mut world, out))
        .result
        .unwrap();
    assert_eq!(
        roll(&hit(KICKER, KICK, 10), &mut world).result,
        Some(HitRoll::Stunned)
    );
}

#[test]
fn the_mac_and_rof2_programs_roll_nothing_and_send_nothing() {
    for wire in [&EqMac as &'static dyn Wire, &Rof2] {
        let dice = Script::default();
        let mut world = fighting(&dice);
        let heard = roll_on(wire, &hit(KICKER, KICK, 10), Instant::now(), &mut world);
        assert_eq!(heard.result, None);
        assert!(heard.events.is_empty() && !world.control.stunned());
        assert_eq!(dice.ranges(), []);
    }
}

#[test]
fn a_damage_record_through_the_feature_rolls_as_the_program() {
    let dice = Script::of(&[60, 100, 100]);
    let mut world = fighting(&dice);
    let message = Message::Event(WorldEvent::Damage(hit(KICKER, KICK, 10)));
    let heard = testing::run(|out| Stuns::observe_at(&message, Instant::now(), &mut world, out));
    heard.result.unwrap();
    assert!(heard.sent.is_empty() && world.control.stunned());
}
