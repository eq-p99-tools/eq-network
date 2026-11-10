use super::super::feature::{testing, Sink};
use super::*;
use eq_network_game::command::EncodedCommand;

struct Bard {
    casting: Casting,
    world: World,
}

impl Bard {
    fn new() -> Self {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        player.class = Some(BARD_CLASS);
        player.memorized_spells[0] = Some(202);
        player.memorized_spells[1] = Some(203);
        world.player.admit(player);
        Self {
            casting: Casting::with_song_stop(),
            world,
        }
    }

    fn command(&mut self, command: &ClientCommand) -> testing::Outcome<Result<()>> {
        testing::run(|out| self.casting.handle(command, &mut self.world, out))
    }

    fn cast(&mut self, gem: u8, spell_id: u32) {
        let command = ClientCommand::CastSpell {
            session_id: self.world.session_id,
            gem,
            spell_id,
            target_id: 7,
            created: Instant::now(),
        };
        assert!(!self.casting.guard.blocks(&command));
        let output = self.command(&command);
        output.result.unwrap();
        assert_eq!(output.sent.len(), 1);
    }

    fn stop_command(&self) -> ClientCommand {
        ClientCommand::StopSong {
            session_id: self.world.session_id,
            created: Instant::now(),
        }
    }

    fn stop(&mut self) -> Instant {
        let stop = self.stop_command();
        let output = self.command(&stop);
        output.result.unwrap();
        assert_eq!(output.sent.len(), 1);
        assert_eq!(output.sent[0].opcode, 0x4839);
        assert_eq!(output.sent[0].body, [] as [u8; 0]);
        assert!(output.events.is_empty());
        self.casting.guard.stop_deadline.unwrap()
    }

    fn tick(&mut self, now: Instant) -> Vec<ClientEvent> {
        let output = testing::run(|out| self.casting.tick(now, &mut self.world, out));
        output.result.unwrap();
        assert_eq!(output.sent, []);
        output.events
    }

    fn hear(&mut self, mut message: Message) -> (Message, Vec<ClientEvent>) {
        self.casting.explain(&mut message, &self.world);
        let output = testing::run(|out| self.casting.observe(&message, &mut self.world, out));
        output.result.unwrap();
        (message, output.events)
    }
}

fn began(caster_id: u16, spell_id: u16) -> Message {
    Message::Event(WorldEvent::Spell(SpellUpdate::Began {
        caster_id,
        spell_id,
        duration_ms: 3000,
    }))
}

fn mana_ended(spell_id: u32) -> Message {
    Message::Event(WorldEvent::Spell(SpellUpdate::Mana {
        spell_id,
        keep_casting: false,
    }))
}

#[test]
fn a_silent_stop_of_a_begun_song_releases_casting_once_for_a_manual_recast() {
    let mut bard = Bard::new();
    bard.cast(0, 202);
    bard.hear(began(7, 202));
    let deadline = bard.stop();
    let stop = bard.stop_command();
    let duplicate = bard.command(&stop);
    duplicate.result.unwrap();
    assert_eq!(duplicate.sent, []);
    assert_eq!(bard.casting.guard.stop_deadline, Some(deadline));

    let before_deadline = deadline.checked_sub(Duration::from_millis(1)).unwrap();
    assert!(bard.tick(before_deadline).is_empty());
    assert!(bard.casting.guard.active());
    assert!(matches!(
        bard.tick(deadline).as_slice(),
        [ClientEvent::World(WorldEvent::SongStopUnconfirmed {
            session_id: 5,
            spell_id: Some(202)
        })]
    ));
    assert!(bard.tick(deadline).is_empty());
    bard.cast(1, 203);
    bard.hear(began(7, 203));
    bard.hear(Message::Event(WorldEvent::Spell(SpellUpdate::BarRefresh {
        slot: 1,
        spell_id: 203,
        reduction_ms: 0,
    })));
    assert!(!bard.casting.guard.active());
}

#[test]
fn stop_before_begin_preserves_order_and_withholds_an_abandoned_late_start() {
    let mut bard = Bard::new();
    bard.cast(0, 202);
    let deadline = bard.stop();
    bard.hear(began(7, 202));
    assert_eq!(bard.casting.guard.stop_deadline, Some(deadline));
    bard.hear(mana_ended(202));
    assert!(!bard.casting.guard.active());
    assert!(bard.tick(deadline).is_empty());

    bard.cast(0, 202);
    let deadline = bard.stop();
    assert!(matches!(
        bard.tick(deadline).as_slice(),
        [ClientEvent::World(WorldEvent::SongStopUnconfirmed {
            spell_id: Some(202),
            ..
        })]
    ));
    assert!(matches!(bard.hear(began(8, 202)).0, Message::Event(_)));
    assert!(matches!(bard.hear(began(7, 202)).0, Message::Withheld));
    assert!(!bard.casting.guard.active());
    bard.cast(0, 202);
    assert!(matches!(bard.hear(began(7, 202)).0, Message::Event(_)));
    assert!(bard.casting.guard.active());
}

#[test]
fn a_late_stopped_start_cannot_take_over_a_different_spell_retry() {
    for new_start_first in [false, true] {
        let mut bard = Bard::new();
        bard.cast(0, 202);
        let deadline = bard.stop();
        assert_eq!(bard.tick(deadline).len(), 1);
        bard.cast(1, 203);

        if new_start_first {
            assert!(matches!(bard.hear(began(7, 203)).0, Message::Event(_)));
        }
        let (old_start, events) = bard.hear(began(7, 202));
        assert!(matches!(old_start, Message::Withheld));
        assert!(events.is_empty());
        assert_eq!(
            bard.casting.guard.pending(),
            if new_start_first { None } else { Some(203) },
        );
        bard.hear(mana_ended(202));
        assert!(bard.casting.guard.answers(203));
        assert!(!bard.casting.guard.answers(202));

        if !new_start_first {
            assert!(matches!(bard.hear(began(7, 203)).0, Message::Event(_)));
        }
        bard.hear(mana_ended(203));
        assert!(!bard.casting.guard.active());
    }
}

#[test]
fn a_same_spell_retry_accepts_the_next_matching_start() {
    let mut bard = Bard::new();
    bard.cast(0, 202);
    let deadline = bard.stop();
    assert_eq!(bard.tick(deadline).len(), 1);

    bard.cast(0, 202);
    assert!(matches!(bard.hear(began(7, 202)).0, Message::Event(_)));
    assert!(bard.casting.guard.active());
    assert_eq!(bard.casting.guard.pending(), None);
}

#[test]
fn a_different_start_discards_the_stopped_hold_before_forwarding_the_start() {
    for old_began in [false, true] {
        let mut bard = Bard::new();
        bard.cast(0, 202);
        if old_began {
            bard.hear(began(7, 202));
        }
        let deadline = bard.stop();

        let (new_start, events) = bard.hear(began(7, 203));
        assert!(matches!(new_start, Message::Event(_)));
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::World(WorldEvent::CastSuperseded {
                session_id: 5,
                spell_id: 202,
            })]
        ));
        assert_eq!(bard.casting.guard.pending(), None);
        assert!(bard.tick(deadline).is_empty());
        if !old_began {
            assert!(matches!(bard.hear(began(7, 202)).0, Message::Withheld));
        }

        bard.hear(mana_ended(202));
        assert!(bard.casting.guard.answers(203));
        assert!(!bard.casting.guard.answers(202));
        bard.hear(mana_ended(203));
        assert!(!bard.casting.guard.active());
    }
}

#[test]
fn a_different_own_start_or_death_invalidates_the_old_stop() {
    let mut bard = Bard::new();
    bard.cast(0, 202);
    bard.hear(began(7, 202));
    let deadline = bard.stop();
    bard.hear(began(7, 203));
    assert!(bard.tick(deadline).is_empty());
    let stop = bard.stop_command();
    let refused = bard.command(&stop);
    refused.result.unwrap();
    assert_eq!(refused.sent, []);
    assert!(bard.casting.guard.active());

    let mut bard = Bard::new();
    bard.cast(0, 202);
    let deadline = bard.stop();
    bard.hear(Message::Event(WorldEvent::Death(
        eq_network_game::zoning::Death {
            spawn_id: 7,
            killer_id: 9,
            corpse_id: 7,
            bind_zone_id: 2,
            corpse_name: None,
        },
    )));
    assert!(bard.tick(deadline).is_empty());
    assert!(!bard.casting.guard.active());
    assert_eq!(bard.casting.guard.late_begin, None);
}

#[test]
fn a_refused_or_failed_song_stop_leaves_the_existing_hold() {
    struct Fails;
    impl Sink for Fails {
        fn send(&mut self, _: &EncodedCommand) -> Result<()> {
            anyhow::bail!("synthetic stop transport failure")
        }
        fn send_unreliable(&mut self, packet: &EncodedCommand) -> Result<()> {
            self.send(packet)
        }
        fn last_received_seconds(&self) -> u64 {
            0
        }
    }

    let mut bard = Bard::new();
    bard.cast(0, 202);
    bard.hear(began(7, 202));
    let stop = bard.stop_command();

    bard.casting.song_stop = false;
    let refused = bard.command(&stop);
    refused.result.unwrap();
    assert_eq!(refused.sent, []);
    assert!(matches!(
        refused.events.as_slice(),
        [ClientEvent::Diagnostic(_)]
    ));
    assert!(bard.tick(Instant::now() + SONG_STOP_WAIT).is_empty());
    assert!(bard.casting.guard.active());
    bard.casting.song_stop = true;

    let failed = testing::run(|out| {
        bard.casting.handle(
            &stop,
            &mut bard.world,
            &mut Out {
                sink: &mut Fails,
                log: &mut *out.log,
                wire: out.wire,
                sender: out.sender,
            },
        )
    });
    assert!(failed.result.is_err());
    assert!(failed.events.is_empty());
    assert!(bard.tick(Instant::now() + SONG_STOP_WAIT).is_empty());
    assert!(bard.casting.guard.active());
}

fn refreshed(gem: u32, spell_id: u32) -> Message {
    Message::Event(WorldEvent::Spell(SpellUpdate::BarRefresh {
        slot: gem,
        spell_id,
        reduction_ms: 0,
    }))
}

/// Sings a gem's song through its cast, so that it plays on with no cast held.
fn sung(bard: &mut Bard, gem: u8, spell_id: u32) {
    bard.cast(gem, spell_id);
    bard.hear(began(7, u16::try_from(spell_id).unwrap()));
    bard.hear(refreshed(u32::from(gem), spell_id));
    assert!(!bard.casting.guard.active());
}

fn nothing_sent(bard: &mut Bard, reason: &str) {
    let stop = bard.stop_command();
    let output = bard.command(&stop);
    output.result.unwrap();
    assert_eq!(output.sent, []);
    assert!(matches!(
        output.events.as_slice(),
        [ClientEvent::Diagnostic(said)] if said == reason
    ));
}

fn stopped_with_no_cast_held(bard: &mut Bard) {
    let stop = bard.stop_command();
    let output = bard.command(&stop);
    output.result.unwrap();
    assert_eq!(output.sent, [eq_network_game::spells::titanium_stop_song()]);
    assert!(matches!(
        output.events.as_slice(),
        [ClientEvent::World(WorldEvent::SongStopUnconfirmed {
            spell_id: None,
            ..
        })]
    ));
}

#[test]
fn a_song_from_a_gem_plays_on_after_its_cast_and_stops_once() {
    let mut bard = Bard::new();
    nothing_sent(&mut bard, "No song is playing");
    sung(&mut bard, 0, 202);
    stopped_with_no_cast_held(&mut bard);
    nothing_sent(&mut bard, "No song is playing");
}

#[test]
fn a_twisted_next_song_is_the_one_playing() {
    let mut bard = Bard::new();
    sung(&mut bard, 0, 202);
    sung(&mut bard, 1, 203);
    assert_eq!(
        bard.casting.song,
        Some(Song {
            spell: 203,
            item: false
        })
    );
    stopped_with_no_cast_held(&mut bard);
}

#[test]
fn a_fizzled_next_cast_ends_the_song_as_the_program_does() {
    // The next cast's start takes the song's place, and the server's update
    // for that spell clears it (read R16b), so the old song does not survive.
    let mut bard = Bard::new();
    sung(&mut bard, 0, 202);
    bard.cast(1, 203);
    bard.hear(mana_ended(203));
    assert!(!bard.casting.guard.active());
    nothing_sent(&mut bard, "No song is playing");
}

#[test]
fn the_servers_update_ends_a_song_as_the_program_reads_it() {
    let mut bard = Bard::new();
    // Keep-casting set changes nothing.
    sung(&mut bard, 0, 202);
    bard.hear(Message::Event(WorldEvent::Spell(SpellUpdate::Mana {
        spell_id: 202,
        keep_casting: true,
    })));
    assert!(bard.casting.song.is_some());
    // Spell -1 ends the current song; so does any spell of the class.
    bard.hear(mana_ended(u32::MAX));
    assert_eq!(bard.casting.song, None);
    sung(&mut bard, 0, 202);
    bard.hear(mana_ended(203));
    assert_eq!(bard.casting.song, None);
    // Another class's spell ends a Bard's gem song only when it is the one
    // playing.
    sung(&mut bard, 0, 202);
    bard.hear(mana_ended(999));
    assert!(bard.casting.song.is_some());
}

#[test]
fn an_items_song_not_of_the_class_stops_here_only() {
    use eq_network_game::inventory::{
        ClickEffect, ClickKind, Inventory, InventorySlot, InventoryUpdate, ItemUse,
    };
    let mut bard = Bard::new();
    let slot = InventorySlot(22);
    let mut inventory = Inventory::default();
    let mut items = Vec::new();
    for (slot, cast_time_ms) in [(slot, 1000), (InventorySlot(23), 100)] {
        let mut item = testing::item(slot.0);
        item.activation.effect = Some(ClickEffect {
            spell_id: 999,
            kind: ClickKind::Expendable,
            required_level: 0,
            effect_level: 1,
            cast_time_ms,
            recast_delay_seconds: 0,
            recast_type: 0,
        });
        item.charges = 1;
        items.push(item);
    }
    inventory.apply(InventoryUpdate::Snapshot(items));
    bard.world.inventory = inventory.into();
    let used = |bard: &mut Bard, slot| {
        let command = ClientCommand::UseItem(ItemUse {
            request_id: 1,
            session_id: 5,
            revision: bard.world.inventory.revision(),
            slot,
            target_id: 7,
            created: Instant::now(),
        });
        let output = bard.command(&command);
        output.result.unwrap();
        bard.hear(refreshed(10, 999));
    };
    // A cast of 100 ms or less is no song (read R16).
    used(&mut bard, InventorySlot(23));
    assert_eq!(bard.casting.song, None);
    used(&mut bard, slot);
    assert_eq!(
        bard.casting.song,
        Some(Song {
            spell: 999,
            item: true
        })
    );
    nothing_sent(
        &mut bard,
        "A song not of the class stops here only; nothing is sent",
    );
    nothing_sent(&mut bard, "No song is playing");
}

#[test]
fn only_a_bard_stops_a_song() {
    let mut bard = Bard::new();
    sung(&mut bard, 0, 202);
    let mut player = testing::player(7);
    player.class = Some(1);
    player.memorized_spells[0] = Some(202);
    bard.world.player.admit(player);
    nothing_sent(&mut bard, "Only Bards can stop a song");
}
