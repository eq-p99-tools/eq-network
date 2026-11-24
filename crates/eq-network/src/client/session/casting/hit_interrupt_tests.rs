//! The Titanium program's local clear on its own hit roll's interrupt
//! (mode 1 of 0x40f8a0, reads R49-50 and R49-21d; checklist 6.645 S2).
use super::super::feature::testing;
use super::*;

const SONG: u32 = 202;
const OTHER_SONG: u32 = 203;
const EXEMPT: u32 = 900;

fn caster(class: u32) -> (Casting, World) {
    let mut world = World::new(5);
    world.own_spawn = Some(7);
    let mut player = testing::player(7);
    player.class = Some(class);
    player.memorized_spells[0] = Some(SONG);
    player.memorized_spells[1] = Some(OTHER_SONG);
    player.memorized_spells[2] = Some(EXEMPT);
    world.player.admit(player);
    (
        Casting::with_titanium_combat(true, Duration::from_secs(60)),
        world,
    )
}

fn gem(gem: u8, spell_id: u32) -> ClientCommand {
    ClientCommand::CastSpell {
        session_id: 5,
        gem,
        spell_id,
        target_id: 7,
        created: Instant::now(),
    }
}

fn spell(update: SpellUpdate) -> Message {
    Message::Event(WorldEvent::Spell(update))
}

fn began(spell_id: u32) -> Message {
    spell(SpellUpdate::Began {
        caster_id: 7,
        spell_id: u16::try_from(spell_id).unwrap(),
        duration_ms: 3000,
    })
}

fn refreshed(gem: u32, spell_id: u32) -> Message {
    spell(SpellUpdate::BarRefresh {
        slot: gem,
        spell_id,
        reduction_ms: 0,
    })
}

fn run(
    casting: &mut Casting,
    world: &mut World,
    step: impl FnOnce(&mut Casting, &mut World, &mut Out<'_, '_>) -> Result<()>,
) -> testing::Outcome<Result<()>> {
    let output = testing::run(|out| step(casting, world, out));
    assert!(output.result.is_ok());
    output
}

fn cast(casting: &mut Casting, world: &mut World, gem_index: u8, spell_id: u32) {
    let output = run(casting, world, |casting, world, out| {
        casting.handle(&gem(gem_index, spell_id), world, out)
    });
    assert_eq!(output.sent.len(), 1);
    run(casting, world, |casting, world, out| {
        casting.observe(&began(spell_id), world, out)
    });
}

/// The roll's interrupt, as the session makes it.
fn interrupt(casting: &mut Casting, world: &mut World) -> testing::Outcome<Result<()>> {
    let output = run(casting, world, |casting, world, out| {
        casting.observe(&Message::HitInterrupt, world, out)
    });
    // The program's mode 1 sends nothing.
    assert!(output.sent.is_empty() && output.unreliable.is_empty());
    output
}

/// The song stop lines and cast clears the host heard.
fn heard(output: &testing::Outcome<Result<()>>) -> Vec<(&'static str, u32, Option<u32>)> {
    output
        .events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::World(WorldEvent::SongStopped {
                spell_id,
                string_id,
                ..
            }) => Some(("stopped", *spell_id, Some(*string_id))),
            ClientEvent::World(WorldEvent::CastCleared { spell_id, .. }) => {
                Some(("cleared", *spell_id, None))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_pulsing_bard_song_ends_abruptly_and_the_next_gem_press_casts() {
    let (mut casting, mut world) = caster(BARD_CLASS);
    cast(&mut casting, &mut world, 0, SONG);
    run(&mut casting, &mut world, |casting, world, out| {
        casting.observe(&refreshed(0, SONG), world, out)
    });
    assert!(!casting.guard.active() && casting.singing(&world).is_some());
    let output = interrupt(&mut casting, &mut world);
    assert_eq!(heard(&output), [("stopped", SONG, Some(0x318e))]);
    assert_eq!(casting.song, None);
    // The host hears it no longer sings.
    assert!(output.events.iter().any(|event| matches!(
        event,
        ClientEvent::World(WorldEvent::Singing { spell_id: None, .. })
    )));
    // Not singing now, a gem press casts rather than stops.
    let output = run(&mut casting, &mut world, |casting, world, out| {
        casting.handle(&gem(1, OTHER_SONG), world, out)
    });
    assert_eq!(output.sent.len(), 1);
    assert_ne!(output.sent[0].opcode, 0x4839);
}

#[test]
fn a_bard_song_in_its_cast_window_ends_abruptly_and_its_cast_clears() {
    let (mut casting, mut world) = caster(BARD_CLASS);
    cast(&mut casting, &mut world, 0, SONG);
    assert!(casting.guard.active());
    let output = interrupt(&mut casting, &mut world);
    assert_eq!(
        heard(&output),
        [("stopped", SONG, Some(0x318e)), ("cleared", SONG, None)]
    );
    assert!(!casting.guard.active());
    // The server finishes the song's cast all the same: its update finds
    // no song here and says nothing. The program would print 0x318f again
    // (0x456fd1 finds the song by the packet's spell): listed, not built.
    let output = run(&mut casting, &mut world, |casting, world, out| {
        casting.observe(
            &spell(SpellUpdate::Mana {
                spell_id: SONG,
                keep_casting: false,
            }),
            world,
            out,
        )
    });
    assert_eq!(heard(&output), []);
}

#[test]
fn another_classs_cast_clears_with_no_line() {
    let (mut casting, mut world) = caster(2);
    cast(&mut casting, &mut world, 0, SONG);
    let output = interrupt(&mut casting, &mut world);
    assert_eq!(heard(&output), [("cleared", SONG, None)]);
    assert!(!casting.guard.active() && casting.song.is_none());
}

#[test]
fn spells_859_to_1023_and_nothing_held_are_left_alone() {
    let (mut casting, mut world) = caster(BARD_CLASS);
    cast(&mut casting, &mut world, 2, EXEMPT);
    let output = interrupt(&mut casting, &mut world);
    assert_eq!(heard(&output), []);
    assert!(casting.guard.active() && casting.song.is_some());
    let (mut casting, mut world) = caster(BARD_CLASS);
    let output = interrupt(&mut casting, &mut world);
    assert!(output.events.is_empty());
}

#[test]
fn a_request_awaiting_its_answer_clears_its_pending_hold_too() {
    let (mut casting, mut world) = caster(2);
    let output = run(&mut casting, &mut world, |casting, world, out| {
        casting.handle(&gem(0, SONG), world, out)
    });
    assert_eq!(output.sent.len(), 1);
    assert_eq!(casting.guard.pending(), Some(SONG));
    let output = interrupt(&mut casting, &mut world);
    assert!(matches!(
        output.events[..],
        [
            ClientEvent::World(WorldEvent::CastPending { spell_id: None, .. }),
            ClientEvent::World(WorldEvent::CastCleared { spell_id: SONG, .. }),
        ]
    ));
    assert_eq!(casting.guard.pending(), None);
}
