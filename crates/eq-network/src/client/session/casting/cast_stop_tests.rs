//! Titanium's `/stopcast` (0x46a75a, read R45).
use super::super::feature::testing;
use super::*;

fn caster(class: u32) -> (Casting, World) {
    let mut world = World::new(5);
    world.own_spawn = Some(7);
    let mut player = testing::player(7);
    player.class = Some(class);
    player.memorized_spells[0] = Some(202);
    world.player.admit(player);
    (
        Casting::with_titanium_combat(true, Duration::from_secs(4300)).with_cast_stop(),
        world,
    )
}

fn stop_cast(world: &World) -> ClientCommand {
    ClientCommand::StopCasting {
        session_id: world.session_id,
        created: Instant::now(),
    }
}

#[test]
fn a_cast_stop_sends_the_empty_mana_change_mid_cast_and_holds_nothing() {
    let (mut casting, mut world) = caster(2);
    let cast = ClientCommand::CastSpell {
        session_id: 5,
        gem: 0,
        spell_id: 202,
        target_id: 7,
        created: Instant::now(),
    };
    testing::run(|out| casting.handle(&cast, &mut world, out))
        .result
        .unwrap();
    assert!(casting.guard.active());
    let stop = stop_cast(&world);
    assert!(!casting.guard.blocks(&stop));
    assert_eq!(super::super::actions::needs(&stop), []);
    let output = testing::run(|out| casting.handle(&stop, &mut world, out));
    output.result.unwrap();
    assert_eq!(output.sent, [eq_network_game::spells::titanium_stop_song()]);
    assert_eq!(
        (output.sent[0].opcode, output.sent[0].body.len()),
        (0x4839, 0)
    );
    // The cast ends when the server answers, as before.
    assert!(casting.guard.active());
    // With no cast it still sends: the handler has no cast test.
    let (mut idle, mut world) = caster(2);
    let output = testing::run(|out| idle.handle(&stop, &mut world, out));
    assert_eq!(output.sent, [eq_network_game::spells::titanium_stop_song()]);
}

#[test]
fn a_bards_cast_stop_is_the_song_stop() {
    let (mut casting, mut world) = caster(BARD_CLASS);
    let stop = stop_cast(&world);
    let output = testing::run(|out| casting.handle(&stop, &mut world, out));
    output.result.unwrap();
    assert_eq!(output.sent, []);
    assert!(matches!(
        output.events.as_slice(),
        [ClientEvent::Diagnostic(said)] if said == "No song is playing"
    ));
}

#[test]
fn without_the_own_spawn_or_the_offer_nothing_is_sent() {
    let (mut casting, mut world) = caster(2);
    world.own_spawn = None;
    let stop = stop_cast(&world);
    let output = testing::run(|out| casting.handle(&stop, &mut world, out));
    assert_eq!(output.sent, []);
    let mut plain = Casting::with_titanium_combat(true, Duration::from_secs(4300));
    let (_, mut world) = caster(2);
    assert!(!plain
        .capabilities()
        .contains(&crate::world::Capability::StoppingCasts));
    let output = testing::run(|out| plain.handle(&stop, &mut world, out));
    assert_eq!(output.sent, []);
}
