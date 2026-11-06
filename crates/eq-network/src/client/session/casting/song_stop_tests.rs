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
        assert!(output.sent[0].body.is_empty());
        assert!(output.events.is_empty());
        self.casting.guard.stop_deadline.unwrap()
    }

    fn tick(&mut self, now: Instant) -> Vec<ClientEvent> {
        let output = testing::run(|out| self.casting.tick(now, &mut self.world, out));
        output.result.unwrap();
        assert!(output.sent.is_empty());
        output.events
    }

    fn hear(&mut self, mut message: Message) -> Message {
        self.casting.explain(&mut message, &self.world);
        let output = testing::run(|out| self.casting.observe(&message, &mut self.world, out));
        output.result.unwrap();
        message
    }
}

fn began(caster_id: u16, spell_id: u16) -> Message {
    Message::Event(WorldEvent::Spell(SpellUpdate::Began {
        caster_id,
        spell_id,
        duration_ms: 3000,
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
    assert!(duplicate.sent.is_empty());
    assert_eq!(bard.casting.guard.stop_deadline, Some(deadline));

    assert!(bard.tick(deadline - Duration::from_millis(1)).is_empty());
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
    bard.hear(Message::Event(WorldEvent::Spell(SpellUpdate::Mana {
        spell_id: 202,
        keep_casting: false,
    })));
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
    assert!(matches!(bard.hear(began(8, 202)), Message::Event(_)));
    assert!(matches!(bard.hear(began(7, 202)), Message::Withheld));
    assert!(!bard.casting.guard.active());
    bard.cast(0, 202);
    assert!(matches!(bard.hear(began(7, 202)), Message::Event(_)));
    assert!(bard.casting.guard.active());
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
    assert!(refused.sent.is_empty());
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
    assert!(refused.sent.is_empty());
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
