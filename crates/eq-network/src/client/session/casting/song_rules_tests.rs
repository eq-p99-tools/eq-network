//! The Titanium program's rules for a Bard's song (read R49-21): what a
//! song holds, a gem click while singing, the spell book's answers and an
//! item cast alongside.
use super::super::actions::Held;
use super::super::feature::testing;
use super::*;
use eq_network_game::abilities::Ability;
use eq_network_game::inventory::{
    ClickEffect, ClickKind, Inventory, InventoryMove, InventorySlot, InventoryUpdate, MoveQuantity,
};

const SONG: u32 = 202;
const OTHER_SONG: u32 = 203;
const INSTANT_ITEM: u32 = 300;
const TIMED_ITEM: u32 = 301;

struct Singer {
    casting: Casting,
    world: World,
}

impl Singer {
    /// A player of this class, under these rules, with two gems and two
    /// clickable items: an instant one in slot 22 and a timed one in 23.
    fn new(casting: Casting, class: u32) -> Self {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        player.class = Some(class);
        player.memorized_spells[0] = Some(SONG);
        player.memorized_spells[1] = Some(OTHER_SONG);
        world.player.admit(player);
        let items = [(22, INSTANT_ITEM, 0), (23, TIMED_ITEM, 1500)].map(|(slot, spell, ms)| {
            let mut item = testing::item(slot);
            item.activation.effect = Some(ClickEffect {
                spell_id: spell,
                kind: ClickKind::Click,
                required_level: 0,
                effect_level: 1,
                cast_time_ms: ms,
                recast_delay_seconds: 0,
                recast_type: 0,
            });
            item
        });
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(items.to_vec()));
        world.inventory = inventory.into();
        Self { casting, world }
    }

    fn titanium_bard() -> Self {
        Self::new(
            Casting::with_titanium_combat(true, Duration::from_secs(60)),
            BARD_CLASS,
        )
    }

    fn run(&mut self, command: &ClientCommand) -> testing::Outcome<Result<()>> {
        let output = testing::run(|out| self.casting.handle(command, &mut self.world, out));
        assert!(output.result.is_ok());
        output
    }

    fn hear(&mut self, message: &Message) -> testing::Outcome<Result<()>> {
        let output = testing::run(|out| self.casting.observe(message, &mut self.world, out));
        assert!(output.result.is_ok());
        output
    }

    fn gem(&self, gem: u8, spell_id: u32) -> ClientCommand {
        ClientCommand::CastSpell {
            session_id: self.world.session_id,
            gem,
            spell_id,
            target_id: 7,
            created: Instant::now(),
        }
    }

    /// Sings the first gem's song until its own start keeps it.
    fn sing(&mut self) {
        let output = self.run(&self.gem(0, SONG));
        assert_eq!(output.sent.len(), 1);
        self.hear(&began(SONG, 3000));
    }

    fn use_item(&self, slot: i32) -> ClientCommand {
        ClientCommand::UseItem(ItemUse {
            installed_class_spell: Some(false),
            request_id: 1,
            session_id: 5,
            revision: self.world.inventory.revision(),
            slot: InventorySlot(slot),
            target_id: 7,
            created: Instant::now(),
        })
    }

    /// Why the session's holds refuse this command, if they do.
    fn refusal(&self, command: &ClientCommand) -> Option<&'static str> {
        Held::new(self.casting.holds(&self.world, Instant::now())).conflict(command)
    }
}

fn began(spell_id: u32, duration_ms: u32) -> Message {
    Message::Event(WorldEvent::Spell(SpellUpdate::Began {
        caster_id: 7,
        spell_id: u16::try_from(spell_id).unwrap(),
        duration_ms,
    }))
}

/// The songs whose stop the host heard of, each with the program's line.
fn stops(output: &testing::Outcome<Result<()>>) -> Vec<u32> {
    output
        .events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::World(WorldEvent::SongStopped {
                spell_id,
                string_id,
                ..
            }) => {
                assert_eq!(*string_id, 0x318f);
                Some(*spell_id)
            }
            _ => None,
        })
        .collect()
}

fn mana_ended(spell_id: u32) -> Message {
    Message::Event(WorldEvent::Spell(SpellUpdate::Mana {
        spell_id,
        keep_casting: false,
    }))
}

fn gem_answer(mode: u32) -> Message {
    Message::Event(WorldEvent::Spell(SpellUpdate::Slot {
        slot: 1,
        spell_id: OTHER_SONG,
        mode,
    }))
}

fn strike() -> ClientCommand {
    ClientCommand::UseAbility {
        session_id: 5,
        target: None,
        ability: Ability::Kick,
        in_liquid: false,
        created: Instant::now(),
    }
}

fn bag_move() -> ClientCommand {
    ClientCommand::MoveInventory(InventoryMove {
        sequence: None,
        session_id: 5,
        revision: 1,
        from: InventorySlot(24),
        to: InventorySlot(25),
        quantity: MoveQuantity::Whole,
        created: Instant::now(),
    })
}

fn memorize() -> ClientCommand {
    ClientCommand::MemorizeSpell {
        preparation: None,
        session_id: 5,
        gem: 2,
        spell_id: 204,
        created: Instant::now(),
    }
}

fn forget() -> ClientCommand {
    ClientCommand::ForgetSpell {
        session_id: 5,
        gem: 1,
        spell_id: OTHER_SONG,
        created: Instant::now(),
    }
}

fn scribe() -> ClientCommand {
    ClientCommand::ScribeSpell {
        session_id: 5,
        revision: 1,
        slot: 1,
        spell_id: 73,
        created: Instant::now(),
    }
}

/// The commands a song lets go at once (read R49-21d; memorize inferred,
/// R49-48 a). A forget waits on no cast at all (R49-34 d).
fn released(singer: &Singer) -> [ClientCommand; 6] {
    [
        strike(),
        singer.use_item(22),
        bag_move(),
        memorize(),
        forget(),
        singer.gem(1, OTHER_SONG),
    ]
}

#[test]
fn a_kept_song_lets_strikes_items_moves_and_the_spell_book_go_at_once() {
    let mut bard = Singer::titanium_bard();
    bard.sing();
    assert!(bard.casting.guard.active());
    for command in released(&bard) {
        assert_eq!(bard.refusal(&command), None, "{command:?}");
    }
    // What the program's side of is unread still waits.
    assert!(bard.refusal(&scribe()).is_some());
}

fn singing(events: &[ClientEvent]) -> Vec<Option<u32>> {
    events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::World(WorldEvent::Singing {
                session_id: 5,
                spell_id,
            }) => Some(*spell_id),
            _ => None,
        })
        .collect()
}

#[test]
fn the_host_is_told_when_a_bard_starts_and_stops_singing() {
    let mut bard = Singer::titanium_bard();
    // The gem's send is no song yet: its own start decides.
    let output = bard.run(&bard.gem(0, SONG));
    assert_eq!(singing(&output.events), []);
    assert_eq!(singing(&bard.hear(&began(SONG, 3000)).events), [Some(SONG)]);
    // Unchanged news tells nothing again.
    assert_eq!(singing(&bard.hear(&began(SONG, 3000)).events), []);
    let stop = ClientCommand::StopSong {
        session_id: 5,
        created: Instant::now(),
    };
    assert_eq!(singing(&bard.run(&stop).events), [None]);

    // Neither a non-Bard nor the Mac rules ever tell it.
    let mut warrior = Singer::new(
        Casting::with_titanium_combat(true, Duration::from_secs(60)),
        1,
    );
    let mut mac = Singer::new(Casting::with_song_stop(), BARD_CLASS);
    for singer in [&mut warrior, &mut mac] {
        singer.run(&singer.gem(0, SONG));
        assert_eq!(singing(&singer.hear(&began(SONG, 3000)).events), []);
    }
}

#[test]
fn another_classs_cast_end_says_no_song_stop_line() {
    // A cleric's gem cast, ended by the server's cast update (spell named
    // or -1) or stopped by `/stopcast`, is no song: no stop line (6.542).
    for ended in [SONG, u32::MAX] {
        let mut cleric = Singer::new(
            Casting::with_titanium_combat(true, Duration::from_secs(60)),
            2,
        );
        cleric.sing();
        let output = cleric.hear(&mana_ended(ended));
        assert_eq!(stops(&output), Vec::<u32>::new(), "{ended}");
        assert_eq!(cleric.casting.song, None);
        // A Bard's own song end still says it.
        let mut bard = Singer::titanium_bard();
        bard.sing();
        assert_eq!(stops(&bard.hear(&mana_ended(ended))), [SONG], "{ended}");
    }
}

#[test]
fn a_non_song_cast_a_non_bard_and_the_mac_rules_hold_as_before() {
    // A gem cast of 100 ms or less is no song once its start says so.
    let mut instant = Singer::titanium_bard();
    instant.run(&instant.gem(0, SONG));
    instant.hear(&began(SONG, 100));
    // A song still awaiting its own start is held as any cast.
    let mut awaiting = Singer::titanium_bard();
    awaiting.run(&awaiting.gem(0, SONG));
    let mut warrior = Singer::new(
        Casting::with_titanium_combat(true, Duration::from_secs(60)),
        1,
    );
    warrior.sing();
    let mut mac = Singer::new(Casting::default(), BARD_CLASS);
    mac.sing();
    for singer in [&instant, &awaiting, &warrior, &mac] {
        assert!(singer.casting.guard.active());
        for command in released(singer) {
            // A gem's forget waits on no cast, only on a request the
            // server hasn't acknowledged (read R49-34 d).
            let held = if matches!(command, ClientCommand::ForgetSpell { .. }) {
                std::ptr::eq(singer, &raw const awaiting)
                    .then_some("Wait for the server to acknowledge the cast")
            } else {
                Some("Wait for the current cast to finish or interrupt it")
            };
            assert_eq!(singer.refusal(&command), held, "{command:?}");
        }
    }
}

#[test]
fn a_gem_click_while_singing_stops_the_song_and_casts_nothing() {
    let mut bard = Singer::titanium_bard();
    bard.sing();
    let output = bard.run(&bard.gem(1, OTHER_SONG));
    assert_eq!(output.sent.len(), 1);
    assert_eq!(output.sent[0].opcode, 0x4839);
    assert_eq!(output.sent[0].body, [] as [u8; 0]);
    // The host hears the stop with the program's line, and no refusal.
    assert_eq!(stops(&output), [SONG]);
    assert!(!output
        .events
        .iter()
        .any(|event| matches!(event, ClientEvent::World(WorldEvent::CastRejected { .. }))));
    assert!(output.events.iter().any(|event| matches!(
        event,
        ClientEvent::Diagnostic(text) if text == SONG_STOPPED
    )));
    assert_eq!(bard.casting.song, None);
    // The next click casts.
    bard.hear(&mana_ended(SONG));
    let output = bard.run(&bard.gem(1, OTHER_SONG));
    assert_eq!(output.sent.len(), 1);
    assert_ne!(output.sent[0].opcode, 0x4839);
}

#[test]
fn the_mac_rules_cast_a_gem_clicked_while_singing() {
    let mut bard = Singer::new(Casting::with_song_stop(), BARD_CLASS);
    bard.sing();
    bard.hear(&mana_ended(SONG));
    let output = bard.run(&bard.gem(1, OTHER_SONG));
    assert_eq!(output.sent.len(), 1);
    assert_ne!(output.sent[0].opcode, 0x4839);
}

#[test]
fn a_forgets_answer_stops_the_song_and_a_memorizes_leaves_it() {
    let mut bard = Singer::titanium_bard();
    bard.sing();
    let output = bard.hear(&gem_answer(1));
    assert_eq!(output.sent, []);
    assert!(bard.casting.song.is_some());
    let output = bard.hear(&gem_answer(2));
    assert_eq!(output.sent.len(), 1);
    assert_eq!(output.sent[0].opcode, 0x4839);
    assert_eq!(stops(&output), [SONG]);
    assert_eq!(bard.casting.song, None);
    // With no song, a forget's answer sends nothing and says nothing.
    let output = bard.hear(&gem_answer(2));
    assert_eq!(output.sent, []);
    assert_eq!(stops(&output), Vec::<u32>::new());

    let mut mac = Singer::new(Casting::with_song_stop(), BARD_CLASS);
    mac.sing();
    assert_eq!(mac.hear(&gem_answer(2)).sent, []);
    assert!(mac.casting.song.is_some());
}

#[test]
fn an_item_cast_goes_alongside_the_song_and_its_end_follows_the_item_flag() {
    // An instant item: its end leaves the song (read R49-21e).
    let mut bard = Singer::titanium_bard();
    bard.sing();
    let output = bard.run(&bard.use_item(22));
    assert_eq!(output.sent.len(), 1);
    assert_eq!(bard.casting.guard.pending(), Some(INSTANT_ITEM));
    assert_eq!(bard.casting.song.map(|song| song.spell), Some(SONG));
    // The item cast holds as any cast while it is in flight.
    assert!(bard.refusal(&strike()).is_some());
    bard.hear(&mana_ended(INSTANT_ITEM));
    assert!(!bard.casting.guard.active());
    assert_eq!(bard.casting.song.map(|song| song.spell), Some(SONG));

    // An item cast of over 100 ms sets the item flag: its end ends the
    // song here, with nothing sent.
    let mut bard = Singer::titanium_bard();
    bard.sing();
    bard.run(&bard.use_item(23));
    assert_eq!(bard.casting.song.map(|song| song.spell), Some(SONG));
    let output = bard.hear(&mana_ended(TIMED_ITEM));
    assert_eq!(output.sent, []);
    assert_eq!(bard.casting.song, None);
    // The program's non-song path says no line (read R49-21e): of the
    // song, the host hears only that it ended (the item cast's own release
    // comes as ever).
    let told: Vec<_> = output
        .events
        .iter()
        .filter(|event| {
            matches!(
                event,
                ClientEvent::World(WorldEvent::Singing { .. } | WorldEvent::SongStopped { .. })
            )
        })
        .collect();
    assert!(
        matches!(
            told[..],
            [ClientEvent::World(WorldEvent::Singing {
                spell_id: None,
                ..
            })]
        ),
        "{told:?}"
    );
}

#[test]
fn a_melody_step_the_stop_then_the_next_gem_casts() {
    // R49-33 c through the session: after the song's own refresh, the
    // host's StopSong and then the next gem's press go out, and the press
    // casts (the click-while-singing rule doesn't take it).
    let mut bard = Singer::titanium_bard();
    bard.sing();
    bard.hear(&Message::Event(WorldEvent::Spell(
        SpellUpdate::BarRefresh {
            slot: 0,
            spell_id: SONG,
            reduction_ms: 0,
        },
    )));
    assert!(!bard.casting.guard.active());
    assert!(bard.casting.song.is_some());
    let stop = ClientCommand::StopSong {
        session_id: 5,
        created: Instant::now(),
    };
    let output = bard.run(&stop);
    assert_eq!(output.sent.len(), 1);
    assert_eq!(output.sent[0].opcode, 0x4839);
    let output = bard.run(&bard.gem(1, OTHER_SONG));
    assert_eq!(output.sent.len(), 1);
    assert_ne!(output.sent[0].opcode, 0x4839);
    assert_eq!(bard.casting.guard.pending(), Some(OTHER_SONG));
}

#[test]
fn each_stop_says_the_programs_line_and_the_mac_rules_say_none() {
    // The player's own stop.
    let mut bard = Singer::titanium_bard();
    bard.sing();
    let output = bard.run(&ClientCommand::StopSong {
        session_id: bard.world.session_id,
        created: std::time::Instant::now(),
    });
    assert_eq!(stops(&output), [SONG]);
    // The server's cast update ending the song (mode 3).
    let mut bard = Singer::titanium_bard();
    bard.sing();
    assert_eq!(stops(&bard.hear(&mana_ended(SONG))), [SONG]);
    // An update that leaves the song says nothing.
    let mut bard = Singer::titanium_bard();
    bard.sing();
    assert_eq!(stops(&bard.hear(&gem_answer(1))), Vec::<u32>::new());
    // The Mac-era program is unread: its stops say nothing.
    let mut mac = Singer::new(Casting::with_song_stop(), BARD_CLASS);
    mac.sing();
    let output = mac.run(&ClientCommand::StopSong {
        session_id: mac.world.session_id,
        created: std::time::Instant::now(),
    });
    assert_eq!(output.sent.len(), 1);
    assert_eq!(stops(&output), Vec::<u32>::new());
}
