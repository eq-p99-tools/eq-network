//! Casting: spells from the memorized gems and items' click effects, each held
//! until the server answers it.
use super::{
    actions::Resource,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::{Context, Result};
use eq_network_game::{
    bandolier::BeltUse,
    inventory::{InventorySlot, ItemUse},
    message::Message,
    request::Request,
    spells::{SpellUpdate, ITEM_CAST_SLOT},
    world::{PostureState, WorldEvent},
};
use std::time::{Duration, Instant};

mod disciplines;
mod knights;
use eq_network_game::knights::KnightAbility;

/// This library's words for a cast refused while the player lies.
const LYING: &str = "You can't cast lying down";

/// Local retry policy, not a protocol timeout. Never resends a request automatically.
const ACK_TIMEOUT: Duration = Duration::from_secs(5);

/// Allow ordered cast results already in flight to arrive. `EQEmu` can stop a
/// song without sending the Bard an interrupt or spell-bar refresh.
const SONG_STOP_WAIT: Duration = Duration::from_secs(1);

/// The line the program refuses a cast with while the player lies, if it
/// does (read R19 d and R19cd 2): every spell cast passes 0x411f3d's
/// posture check, gems and the knights' specials read, item clicks inferred.
/// The program lets a spell whose record sets its byte at +0x1fc through;
/// the session has no spell table, so it refuses every one (stricter;
/// recording checklist 6.455). A discipline is not refused: no posture
/// check was found on its path (6.455).
fn lying_refusal(world: &World, out: &Out<'_, '_>) -> Option<u32> {
    out.wire
        .dialect()
        .lying()
        .casting_refusal
        .filter(|_| world.posture.current() == Some(PostureState::Lying))
}

/// Reports only cast rejections, preserving the caller's admission for host filtering.
pub(super) fn rejected(command: &ClientCommand, reason: &str) -> Option<crate::world::WorldEvent> {
    let (session_id, spell_id) = match *command {
        ClientCommand::CastSpell {
            session_id,
            spell_id,
            ..
        }
        | ClientCommand::UseDiscipline {
            session_id,
            spell_id,
            ..
        } => (session_id, spell_id),
        ClientCommand::UseKnightAbility {
            session_id,
            ability,
            ..
        } => (session_id, ability.spell()),
        _ => return None,
    };
    Some(crate::world::WorldEvent::CastRejected {
        session_id,
        spell_id,
        reason: reason.into(),
        string_id: None,
    })
}

enum Phase {
    Awaiting {
        spell: u32,
        submitted: Instant,
    },
    /// The server began a cast: the spell it names, and the one asked for,
    /// which it can name differently. TAKP begins a Luclin port as spell
    /// 2935 (`Mob::DoCastSpell`) and ends it naming the port.
    Casting {
        spell: u32,
        requested: Option<u32>,
    },
}

#[derive(Clone, Copy)]
pub(super) enum Source {
    Gem(u8),
    Item,
    /// An item used from the potion belt, answered on its own slot.
    Belt,
    Knight(KnightAbility),
    Discipline,
}

impl Source {
    /// Match a different server spell ID only for the knight request that
    /// can receive that rank.
    fn matches_rank(source: Option<Self>, spell: u32) -> bool {
        matches!(source, Some(Self::Knight(ability)) if ability.matches_spell(spell))
    }

    fn accepts_refresh(source: Option<Self>, slot: u32) -> bool {
        source.is_none_or(|source| match source {
            Self::Gem(gem) => slot == u32::from(gem),
            Self::Item => slot == u32::from(ITEM_CAST_SLOT),
            // Only the belt's own slot is this cast's (6.568 B5).
            Self::Belt => slot == u32::from(eq_network_game::spells::BELT_CAST_SLOT),
            // EQEmu's ServerToTitaniumCastingSlot falls back to the
            // discipline slot for its internal Ability slot.
            Self::Knight(_) => {
                slot == u32::from(eq_network_game::knights::CAST_SLOT)
                    || slot == u32::from(eq_network_game::disciplines::CAST_SLOT)
            }
            Self::Discipline => slot == u32::from(eq_network_game::disciplines::CAST_SLOT),
        })
    }
}

enum Followup {
    LateBegin(u32),
    Superseded(u32),
}

#[derive(Default)]
pub(super) struct CastGuard {
    phase: Option<Phase>,
    source: Option<Source>,
    stop_deadline: Option<Instant>,
    luclin_port_begins: bool,
    /// Request provenance survives the acknowledgement wait without holding casting.
    unconfirmed: Option<(u32, Option<Source>)>,
    late_begin: Option<u32>,
}

impl CastGuard {
    /// Whether the shared action table refuses this command while a cast is held;
    /// movement and posture interruption stay available.
    #[cfg(test)]
    pub fn blocks(&self, command: &ClientCommand) -> bool {
        self.active() && super::actions::needs(command).contains(&super::actions::Resource::Casting)
    }

    /// A duration reaching zero does not authorize another cast: await a server result.
    pub fn active(&self) -> bool {
        self.phase.is_some()
    }

    /// What a cast in flight holds.
    pub fn hold(&self) -> Option<(Resource, &'static str)> {
        self.active().then_some((
            Resource::Casting,
            "Wait for the current cast to finish or interrupt it",
        ))
    }

    /// Records a request only after successful transport submission.
    pub fn submitted(&mut self, spell: u32, source: Option<Source>, now: Instant) {
        *self = Self {
            phase: Some(Phase::Awaiting {
                spell,
                submitted: now,
            }),
            source,
            luclin_port_begins: self.luclin_port_begins,
            late_begin: self.late_begin.filter(|&abandoned| abandoned != spell),
            ..Self::default()
        };
    }

    /// The spell named by the current hold.
    fn held_spell(&self) -> Option<u32> {
        match self.phase {
            Some(Phase::Awaiting { spell, .. } | Phase::Casting { spell, .. }) => Some(spell),
            None => None,
        }
    }

    /// The unacknowledged request, distinct from a server-confirmed cast.
    pub fn pending(&self) -> Option<u32> {
        match self.phase {
            Some(Phase::Awaiting { spell, .. }) => Some(spell),
            _ => None,
        }
    }

    /// Ends the local hold after a submitted stop received no matching result.
    fn expire_song_stop(&mut self, now: Instant) -> Option<u32> {
        if now < self.stop_deadline? {
            return None;
        }
        let spell = self.held_spell()?;
        let late_begin = self.pending();
        self.clear();
        self.late_begin = late_begin;
        Some(spell)
    }

    /// Allows a fresh user attempt after silence; ordinary begun casts stay held.
    pub fn expire(&mut self, now: Instant) -> Option<u32> {
        let Some(Phase::Awaiting { spell, submitted }) = self.phase else {
            return None;
        };
        if self.stop_deadline.is_some() || now.saturating_duration_since(submitted) < ACK_TIMEOUT {
            return None;
        }
        let source = self.source;
        self.clear();
        self.unconfirmed = Some((spell, source));
        Some(spell)
    }

    /// Whether a result for this spell answers the cast held: the spell
    /// requested, or the one the server began.
    fn answers(&self, spell_id: u32) -> bool {
        match self.phase {
            Some(Phase::Awaiting { spell, .. }) => {
                spell == spell_id || Source::matches_rank(self.source, spell_id)
            }
            Some(Phase::Casting { spell, requested }) => {
                spell == spell_id || requested == Some(spell_id)
            }
            None => false,
        }
    }

    /// Clears state when this character dies or the owning zone session ends.
    pub fn clear(&mut self) {
        self.release();
        self.unconfirmed = None;
    }

    /// Settles the observed hold independently of any timed-out request.
    fn release(&mut self) {
        self.phase = None;
        self.source = None;
        self.stop_deadline = None;
        self.late_begin = None;
    }

    fn matches_late_begin(&self, requested: u32, received: u32) -> bool {
        requested == received
            || self.unconfirmed.is_some_and(|(_, source)| Source::matches_rank(source, received))
            // EQMacEmu Mob::DoCastSpell reports IsLuclinPortSpell as 2935.
            // An exact 2935 request is handled by the first comparison.
            || (self.luclin_port_begins && received == 2935 && matches!(requested, 2062 | 2706..=2709))
    }

    /// Follows own starts and interruptions and matching spell results.
    /// Reports local provenance to the host before forwarding the received update.
    fn observe(&mut self, own_id: u16, update: &SpellUpdate) -> Option<Followup> {
        match *update {
            SpellUpdate::Began {
                caster_id,
                spell_id,
                ..
            } if caster_id == own_id => {
                let spell = u32::from(spell_id);
                if self.stop_deadline.is_some() && !self.answers(spell) {
                    let stopped = self.held_spell();
                    self.stop_deadline = None;
                    self.source = None;
                    self.late_begin = self.pending().or(self.late_begin);
                    self.phase = Some(Phase::Casting {
                        spell,
                        requested: None,
                    });
                    return stopped.map(Followup::Superseded);
                }
                if let Some((requested, source)) = self
                    .unconfirmed
                    .filter(|&(requested, _)| self.matches_late_begin(requested, spell))
                {
                    self.unconfirmed = None;
                    self.source = source;
                    self.phase = Some(Phase::Casting {
                        spell,
                        requested: Some(requested),
                    });
                    return Some(Followup::LateBegin(requested));
                }
                let requested = match self.phase {
                    Some(Phase::Awaiting { spell, .. }) => Some(spell),
                    Some(Phase::Casting {
                        spell: active,
                        requested,
                    }) if active == spell => requested,
                    _ => {
                        self.source = None;
                        None
                    }
                };
                self.phase = Some(Phase::Casting { spell, requested });
            }
            SpellUpdate::Interrupted { caster_id, .. } if caster_id == u32::from(own_id) => {
                // This message has no spell ID to settle a timed-out request.
                self.release();
            }
            SpellUpdate::Mana {
                spell_id,
                keep_casting: false,
            } => {
                if self.answers(spell_id) {
                    self.release();
                }
                if self.unconfirmed.is_some_and(|(spell, source)| {
                    spell == spell_id || Source::matches_rank(source, spell_id)
                }) {
                    self.unconfirmed = None;
                }
            }
            SpellUpdate::BarRefresh { slot, spell_id, .. } => {
                if self.answers(spell_id) && Source::accepts_refresh(self.source, slot) {
                    self.release();
                }
                if self.unconfirmed.is_some_and(|(spell, source)| {
                    (spell == spell_id || Source::matches_rank(source, spell_id))
                        && Source::accepts_refresh(source, slot)
                }) {
                    self.unconfirmed = None;
                }
            }
            _ => (),
        }
        None
    }
}

/// The program's song state (actor +0xf98 and +0x104c, read R16): the spell
/// of the player's own last cast start, and whether an item cast it. No song
/// ends on its own; the server's cast update, a stop, or a new cast ends it.
/// A gem's song is kept only over 100 ms once the player's own cast start
/// says its time. Under the Titanium program's rules (`SongRules`), a
/// forget's answer stops it and a memorize's leaves it (read R49-21c).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Song {
    spell: u32,
    item: bool,
    /// A gem's song whose own cast start has not come yet: only that first
    /// start decides it (6.542).
    awaiting_start: bool,
    /// For an item's song, the host's word from installed spell data on
    /// whether its spell is the class's (`ItemUse::installed_class_spell`);
    /// `None` falls back to the memorized test.
    installed_class_spell: Option<bool>,
}

/// The longest cast the program does not keep as a song: a cast is the
/// song only over 100 ms (0x411f3d, read R16).
const SONG_CAST_MS: u32 = 100;

/// Casts spells and item effects for the player, one at a time.
#[derive(Default)]
pub(super) struct Casting {
    guard: CastGuard,
    song: Option<Song>,
    /// The program's item-cast flag (+0x104c, read R49-21e): an item cast
    /// of over 100 ms is in flight. Under the Titanium rules, the server's
    /// end of it for a Bard's non-song ends the song here.
    item_cast: bool,
    song_rules: SongRules,
    /// The song the host was last told a Bard sings (`WorldEvent::Singing`).
    told_singing: Option<u32>,
    song_stop: bool,
    cast_stop: bool,
    melody: MelodyUse,
    knights: Option<knights::Reuse>,
    disciplines: disciplines::DisciplineState,
    use_disciplines: DisciplineUse,
    belt_use: BeltUses,
    item_recasts: ItemRecasts,
}

/// The Titanium program's item recast stamps (0x411f3d step 6, set by
/// 0x4bac0d, read R49-34 k): seconds on its wall clock, `time()` at a base
/// tick plus the ticks since. An item with a recast is refused with 0xec
/// until the later of its own stamp and its recast type's.
#[derive(Default)]
struct ItemRecasts {
    /// Each item's stamp from this session's sends, by its slot and item.
    items: std::collections::BTreeMap<(InventorySlot, u32), u64>,
    /// The type timers 0-19.
    types: [u64; ITEM_RECAST_TYPES],
}

/// The program's recast types, 0 to 19 (0x5f0b70).
const ITEM_RECAST_TYPES: usize = 20;

impl ItemRecasts {
    /// When the item may be used again: the later of the server's own
    /// stamp for the instance ([I]: that the serialized recast timestamp is
    /// the program's item +0x54), this session's stamp for it, and its
    /// type's.
    fn ready(&self, slot: InventorySlot, item: &eq_network_game::inventory::InventoryItem) -> u64 {
        let Some(effect) = item.activation.effect.as_ref() else {
            return 0;
        };
        let own = self
            .items
            .get(&(slot, item.details.id))
            .copied()
            .unwrap_or(0);
        let kind = usize::try_from(effect.recast_type)
            .ok()
            .and_then(|kind| self.types.get(kind))
            .copied()
            .unwrap_or(0);
        u64::from(item.activation.recast_timestamp)
            .max(own)
            .max(kind)
    }

    /// The program stamps the item and its type at the send: now plus the
    /// recast delay.
    fn sent(
        &mut self,
        slot: InventorySlot,
        item: &eq_network_game::inventory::InventoryItem,
        now: u64,
    ) {
        let Some(effect) = item.activation.effect.as_ref() else {
            return;
        };
        if effect.recast_delay_seconds == 0 {
            return;
        }
        let ready = now + u64::from(effect.recast_delay_seconds);
        self.items.insert((slot, item.details.id), ready);
        if let Some(kind) = usize::try_from(effect.recast_type)
            .ok()
            .and_then(|kind| self.types.get_mut(kind))
        {
            *kind = ready;
        }
    }
}

/// The program's wall clock in seconds (0x576e80).
fn wall_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The program's line for an item used before its recast (colour 0x121).
const ITEM_RECAST_LINE: u32 = 0xec;

/// Which wire's disciplines the server type takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DisciplineUse {
    #[default]
    Unoffered,
    /// The listed spells, cast with the discipline slot (read R48).
    Titanium,
    /// TAKP's and Quarm's numbered `OP_Discipline` (6.603 G1).
    EqMac,
}

/// Whose program's song rules the session follows (read R49-21; 6.442,
/// 6.542).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SongRules {
    /// Today's holds: a song holds what any cast holds, a gem click casts,
    /// and the spell book's answers leave the song alone. The Mac-era
    /// program is unread.
    #[default]
    Held,
    /// The Titanium program's (read R49-21): a song holds nothing a song
    /// doesn't, a gem click stops it and casts nothing, a forget's answer
    /// stops it, and an item cast goes alongside it.
    Titanium,
}

/// Whether the server type takes a Bard's `/melody` (read R49-33 c).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum MelodyUse {
    #[default]
    Unoffered,
    Offered,
}

/// Whether the server type takes items used from the potion belt (6.568).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum BeltUses {
    #[default]
    Unoffered,
    Offered,
}

const BARD_CLASS: u32 = 8;

impl Casting {
    pub(super) fn with_song_stop() -> Self {
        Self {
            song_stop: true,
            ..Self::default()
        }
    }

    /// Titanium policies with the class ability casting slot.
    pub(super) fn with_titanium_combat(song_stop: bool, knight_estimate: Duration) -> Self {
        let mut casting = if song_stop {
            Self::with_song_stop()
        } else {
            Self::default()
        };
        casting.knights = Some(knights::Reuse::new(knight_estimate));
        casting.use_disciplines = DisciplineUse::Titanium;
        casting.song_rules = SongRules::Titanium;
        casting
    }

    /// Takes TAKP's and Quarm's `/disc` by number, from their own
    /// `Handle_OP_Discipline` (6.603 G1, source-derived, not yet live).
    pub(super) fn with_eqmac_disciplines(mut self) -> Self {
        self.use_disciplines = DisciplineUse::EqMac;
        self
    }

    /// Offers Titanium's `/stopcast`. `EQEmu` only for now: it ends a cast
    /// without moving, and P99's own handling of it is unchecked.
    pub(super) fn with_cast_stop(mut self) -> Self {
        self.cast_stop = true;
        self
    }

    /// Offers a Bard's `/melody` (read R49-33 c), which the host runs with
    /// `StopSong` and gem casts. The Titanium server types (P99 and stock
    /// `EQEmu`) take it; the Mac-era program is unread.
    pub(super) fn with_melody(mut self) -> Self {
        self.melody = MelodyUse::Offered;
        self
    }

    /// Takes items used from the potion belt, once its live rows pass
    /// (6.568); P99 never.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "offered on stock EQEmu once rows 6.568 pass")
    )]
    pub(super) fn with_belt_use(mut self) -> Self {
        self.belt_use = BeltUses::Offered;
        self
    }

    /// Stock `EQMacEmu` rewrites the received start of selected Luclin ports.
    pub(super) fn with_luclin_port_begins() -> Self {
        Self {
            guard: CastGuard {
                luclin_port_begins: true,
                ..CastGuard::default()
            },
            ..Self::default()
        }
    }

    /// Stops a Bard's song as the program does (0x47020e, 0x40f06d mode 0;
    /// read R16): a song of the class's spell, from a gem or an item, sends
    /// the empty stop and ends here, whatever else is being cast; another
    /// song, from an item, ends here only; with no song nothing happens.
    /// Other classes stop nothing.
    fn stop_song(&mut self, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        if !self.song_stop {
            return out
                .log
                .diagnostic("Stopping songs is unavailable on this server".into());
        }
        if world
            .player
            .as_ref()
            .is_none_or(|p| p.class != Some(BARD_CLASS))
        {
            return out.log.diagnostic("Only Bards can stop a song".into());
        }
        let Some(song) = self.song else {
            return out.log.diagnostic("No song is playing".into());
        };
        if !class_song(song, world) {
            // Not a song by the program's test (0x40db0b), so 0x40f06d
            // takes its non-song path: no send and no line (read R49-21e).
            self.song = None;
            self.item_cast = false;
            return out
                .log
                .diagnostic("A song not of the class stops here only; nothing is sent".into());
        }
        out.request(&Request::StopSong)?;
        self.song = None;
        self.item_cast = false;
        self.report_song_stopped(song, world, out)?;
        if self.guard.active() {
            self.guard.stop_deadline = Some(Instant::now() + SONG_STOP_WAIT);
            Ok(())
        } else {
            report_unconfirmed_stop(world.session_id, None, out)
        }
    }

    /// Titanium's `/stopcast` (0x46a75a, read R45): with the player's own
    /// spawn present, a Bard stops the song as `/stopsong` does, and anyone
    /// else sends the empty `OP_ManaChange`, the same body as the song stop.
    /// The cast ends when the server answers (R25), as before. Without the
    /// own spawn the host shows the program's line. The program also
    /// refuses a mounted player, which this doesn't decode: the host must
    /// refuse `/stopcast` for one (6.487).
    fn stop_cast(&mut self, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        if !self.cast_stop {
            return out
                .log
                .diagnostic("Stopping casts is unavailable on this server".into());
        }
        let Some(player) = world.player.as_ref().filter(|_| world.own_spawn.is_some()) else {
            return out
                .log
                .diagnostic("No own spawn to stop casting; nothing is sent".into());
        };
        if player.class == Some(BARD_CLASS) {
            return self.stop_song(world, out);
        }
        out.request(&Request::StopSong)
    }

    /// The song a Bard is singing, under the Titanium program's rules
    /// (0x4eb9b7, read R49-21b): a song kept whose spell is the class's.
    /// A gem's song counts once its own cast start has kept it.
    fn singing(&self, world: &World) -> Option<Song> {
        if self.song_rules != SongRules::Titanium {
            return None;
        }
        world
            .player
            .as_ref()
            .filter(|player| player.class == Some(BARD_CLASS))?;
        self.song
            .filter(|song| !song.awaiting_start && class_song(*song, world))
    }

    /// Tells the host when the song a Bard sings changes, so its own gates
    /// can follow the session's.
    fn report_singing(&mut self, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        let singing = self.singing(world).map(|song| song.spell);
        if singing == self.told_singing {
            return Ok(());
        }
        self.told_singing = singing;
        out.log.send(ClientEvent::World(WorldEvent::Singing {
            session_id: world.session_id,
            spell_id: singing,
        }))
    }

    /// Tells the host a song stopped under the Titanium program's rules, with
    /// the line the program prints at the stop (0x40f06d, read R49-21): a
    /// stop of mode 0 (the player's) or 3 (the server's cast update) says
    /// 0x318f. The Mac-era program is unread, so it says nothing there.
    fn report_song_stopped(&self, song: Song, world: &World, out: &mut Out<'_, '_>) -> Result<()> {
        if self.song_rules != SongRules::Titanium {
            return Ok(());
        }
        out.log.send(ClientEvent::World(WorldEvent::SongStopped {
            session_id: world.session_id,
            spell_id: song.spell,
            string_id: SONG_STOP_LINE,
        }))
    }

    /// The server's cast update for a spell, keep-casting clear, ends the
    /// song as the program's 0x40f06d does (read R16): the current one for
    /// spell -1; any song for a spell of the player's class, or for another
    /// class's player; and for a Bard a song from an item, or the very spell
    /// named. A memorized spell is taken as the class's (6.442). Under the
    /// Titanium rules, a Bard's non-song also ends it while the item-cast
    /// flag is set, with nothing sent; an instant one leaves it (read
    /// R49-21e). The flag clears either way.
    fn song_reported(&mut self, spell: u32, world: &World) -> Option<Song> {
        let item_cast = std::mem::take(&mut self.item_cast);
        let song = self.song?;
        let player = world.player.as_ref();
        let bard = player.is_some_and(|p| p.class == Some(BARD_CLASS));
        let class_spell = player.is_some_and(|p| p.memorized_spells.contains(&Some(spell)));
        let item_cast = item_cast && self.song_rules == SongRules::Titanium;
        if spell == u32::MAX
            || !bard
            || class_spell
            || song.item
            || song.spell == spell
            || item_cast
        {
            self.song = None;
            // The program's stop (0x40f06d, mode 3) says its line only for
            // a song, or for another class's player; a Bard's non-song, the
            // item cast's end among them, clears it with no line (read
            // R49-21e). A Bard's memorized spell is taken as a song ([I],
            // 6.442), another spell as not.
            let ends_a_song = spell == u32::MAX || song.spell == spell || class_spell || !bard;
            return ends_a_song.then_some(song);
        }
        None
    }

    /// The player's own cast start, with the cast time the server sends
    /// (`EQEmu`'s base time, `Mob::DoCastSpell`), keeps a gem's song only over
    /// 100 ms, as 0x411f3d keeps a cast (read R16). Until it comes, a gem
    /// cast is taken as a song; with none, it stays one. For a Bard's song
    /// every adjustment is 0, so the program compares the base time too
    /// (read R49-21a). Only
    /// the first own start of that spell after the gem's send decides; a
    /// later one, such as an item's click of the same spell, changes
    /// nothing. An item's song was decided by its record's own cast time.
    fn song_begun(&mut self, spell: u32, duration_ms: u32) {
        let Some(song) = self.song.as_mut() else {
            return;
        };
        if song.item || song.spell != spell || !song.awaiting_start {
            return;
        }
        song.awaiting_start = false;
        if duration_ms <= SONG_CAST_MS {
            self.song = None;
        }
    }

    /// Casts a memorized spell at a target the player can see.
    fn cast(
        &mut self,
        command: &ClientCommand,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::CastSpell {
            session_id,
            gem,
            spell_id,
            target_id,
            ..
        } = command
        else {
            return Ok(());
        };
        // A gem click while singing stops the song and casts nothing
        // (0x4ecb56, read R49-21c). The gem timers the stop resets are the
        // host's (6.542).
        if self.singing(world).is_some() {
            self.stop_song(world, out)?;
            return out.log.diagnostic(SONG_STOPPED.into());
        }
        // The program refuses a cast while the player lies, and neither
        // stands them nor casts (read R19 d, 0x411f3d).
        if let Some(string_id) = lying_refusal(world, out) {
            out.log.send(ClientEvent::World(WorldEvent::CastRejected {
                session_id: *session_id,
                spell_id: *spell_id,
                reason: LYING.into(),
                string_id: Some(string_id),
            }))?;
            return out.log.diagnostic("Refused a cast: the player lies".into());
        }
        let ready = world.visible(*target_id)
            && world.player.as_ref().is_some_and(|player| {
                player.memorized_spells.get(usize::from(*gem)) == Some(&Some(*spell_id))
            });
        let refusal = if ready {
            match out.encode_command(command) {
                Ok(packet) => {
                    out.send(&packet)?;
                    // A gem cast is taken as a song until the player's own
                    // cast start says its time (`song_begun`, 6.442).
                    self.song = Some(Song {
                        spell: *spell_id,
                        item: false,
                        awaiting_start: true,
                        installed_class_spell: None,
                    });
                    return self.submitted(*spell_id, Source::Gem(*gem), world, out);
                }
                Err(error) => (
                    error.to_string(),
                    format!("Rejected invalid outbound client command: {error}"),
                ),
            }
        } else {
            (
                "The spell gem changed, or the target is unavailable".into(),
                "Rejected an unavailable spell or target".into(),
            )
        };
        if let Some(event) = rejected(command, &refusal.0) {
            out.log.send(ClientEvent::World(event))?;
        }
        out.log.diagnostic(refusal.1)
    }

    /// Uses the item a belt slot names from the first slot the player wears
    /// it in or carries it in a pack, by the whole item-click path (6.568
    /// B3). A bag is not a click slot (`check_item_cast`), nor the cursor
    /// or a bank (B4); the order is inferred until read R49-45 c.
    fn use_belt_item(
        &mut self,
        request: &BeltUse,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match belt_item_slot(request, world) {
            Ok(slot) => {
                let item = ItemUse {
                    request_id: request.request_id,
                    session_id: request.session_id,
                    revision: world.inventory.revision(),
                    slot,
                    target_id: request.target_id,
                    created: request.created,
                    installed_class_spell: request.installed_class_spell,
                };
                self.use_item(&item, Source::Belt, world, out)
            }
            Err(error) => out.log.send(ClientEvent::World(WorldEvent::ItemUseAction {
                session_id: request.session_id,
                request_id: request.request_id,
                error: Some(error.into()),
                string_id: None,
            })),
        }
    }

    /// Casts an item's click effect at a target the player can see.
    fn use_item(
        &mut self,
        request: &ItemUse,
        source: Source,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Some(string_id) = lying_refusal(world, out) {
            return out.log.send(ClientEvent::World(WorldEvent::ItemUseAction {
                session_id: request.session_id,
                request_id: request.request_id,
                error: Some(LYING.into()),
                string_id: Some(string_id),
            }));
        }
        // Titanium's program reads its item recasts first (step 6); the
        // Mac-era and RoF2 programs' aren't read.
        let program_lines = match out.wire.dialect() {
            eq_network_game::GameDialect::Titanium => true,
            eq_network_game::GameDialect::EqMac | eq_network_game::GameDialect::Rof2 => false,
        };
        let item = world.inventory.items().get(&request.slot).cloned();
        let now = wall_seconds();
        let recasting = item.as_ref().is_some_and(|item| {
            item.activation
                .effect
                .as_ref()
                .is_some_and(|effect| effect.recast_delay_seconds != 0)
                && now < self.item_recasts.ready(request.slot, item)
        });
        if program_lines && recasting {
            return out.log.send(ClientEvent::World(WorldEvent::ItemUseAction {
                session_id: request.session_id,
                request_id: request.request_id,
                error: Some("The item is not ready to use again yet".into()),
                string_id: Some(ITEM_RECAST_LINE),
            }));
        }
        let target_available = world.player.is_some() && world.visible(request.target_id);
        let prepared = world
            .player
            .as_ref()
            .context("Character level is unavailable")
            .and_then(|player| {
                world
                    .inventory
                    .check_item_use(request, player.level, target_available)
            })
            .and_then(|spell_id| {
                let (slot, target_id) = (request.slot, request.target_id);
                let packet = out.encode(&if matches!(source, Source::Belt) {
                    Request::CastBeltItem {
                        spell_id,
                        slot,
                        target_id,
                    }
                } else {
                    Request::CastItem {
                        spell_id,
                        slot,
                        target_id,
                    }
                })?;
                Ok((spell_id, packet))
            });
        let error = match prepared {
            Ok((spell_id, packet)) => {
                out.send(&packet)?;
                if let Some(item) = item.as_ref().filter(|_| program_lines) {
                    self.item_recasts.sent(request.slot, item, now);
                }
                let timed = world
                    .inventory
                    .click_cast_time_ms(request.slot)
                    .is_some_and(|ms| ms > SONG_CAST_MS);
                // While a Bard sings, the item cast goes alongside the song,
                // which stays kept (0x411f3d's item path, read R49-21d).
                let singing = self.singing(world).is_some();
                self.item_cast = timed;
                if timed && !singing {
                    self.song = Some(Song {
                        spell: spell_id,
                        item: true,
                        awaiting_start: false,
                        installed_class_spell: request.installed_class_spell,
                    });
                }
                self.submitted(spell_id, source, world, out)?;
                None
            }
            Err(error) => Some(error),
        };
        // Titanium's own line for a refusal its cast start says (read
        // R49-34 e 8).
        let string_id = error
            .as_ref()
            .and_then(|error| error.downcast_ref::<eq_network_game::inventory::ItemRefusal>())
            .filter(|_| program_lines)
            .map(|refusal| refusal.string_id);
        out.log.send(ClientEvent::World(WorldEvent::ItemUseAction {
            session_id: request.session_id,
            request_id: request.request_id,
            error: error.map(|error| error.to_string()),
            string_id,
        }))
    }

    /// Holds casting until the server answers the request.
    fn submitted(
        &mut self,
        spell_id: u32,
        source: Source,
        world: &World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.guard.submitted(spell_id, Some(source), Instant::now());
        out.log.send(ClientEvent::World(WorldEvent::CastPending {
            session_id: world.session_id,
            spell_id: Some(spell_id),
        }))
    }
}

/// The slot a belt use casts from: the belt mirror's item, in the first
/// worn slot (0 to 21) or pack (22 to 29) holding it whose custody is
/// known (inferred until read R49-45 c).
fn belt_item_slot(request: &BeltUse, world: &World) -> Result<InventorySlot, &'static str> {
    let belt = world
        .belt
        .as_ref()
        .ok_or("Your potion belt is not known; it is read again when you zone.")?;
    let held = belt
        .get(usize::from(request.belt_slot))
        .ok_or("Choose a belt slot from 1 to 4.")?
        .as_ref()
        .ok_or("That belt slot is empty.")?;
    let inventory = &world.inventory;
    if !inventory.received() || inventory.stale() {
        return Err("Wait for current inventory contents");
    }
    (0..=29)
        .map(InventorySlot)
        .find(|slot| {
            inventory
                .items()
                .get(slot)
                .is_some_and(|item| item.details.id == held.item_id)
        })
        .filter(|slot| inventory.custody_known(*slot))
        .ok_or("You carry that item nowhere it can be used from the belt.")
}

impl Feature for Casting {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        let mut offered = vec![Capability::Casting];
        if self.song_stop {
            offered.push(Capability::StoppingSongs);
        }
        if self.cast_stop {
            offered.push(Capability::StoppingCasts);
        }
        if self.melody == MelodyUse::Offered {
            offered.push(Capability::Melody);
        }
        if self.knights.is_some() {
            offered.push(Capability::KnightAbilities);
        }
        match self.use_disciplines {
            DisciplineUse::Unoffered => {}
            DisciplineUse::Titanium => offered.push(Capability::UsingDisciplines),
            DisciplineUse::EqMac => {
                offered.extend([Capability::UsingDisciplines, Capability::DisciplineReuse]);
            }
        }
        if self.belt_use == BeltUses::Offered {
            offered.push(Capability::UsingPotionBelt);
        }
        offered
    }

    /// A cast in flight holds casting; a Bard's own song, under the
    /// Titanium program's rules, holds only what a song holds (read
    /// R49-21d). An item cast alongside the song holds as any cast.
    fn holds(&self, world: &World, _now: Instant) -> Vec<(Resource, &'static str)> {
        let Some(hold) = self.guard.hold() else {
            return Vec::new();
        };
        if self
            .singing(world)
            .is_some_and(|song| self.guard.held_spell() == Some(song.spell))
        {
            return vec![(Resource::Song, hold.1)];
        }
        vec![hold]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        if matches!(command, ClientCommand::UseBeltItem(_)) {
            return self.belt_use == BeltUses::Offered;
        }
        matches!(
            command,
            ClientCommand::CastSpell { .. }
                | ClientCommand::UseItem(_)
                | ClientCommand::DropInvisibility { .. }
                | ClientCommand::StopSong { .. }
                | ClientCommand::StopCasting { .. }
                | ClientCommand::UseKnightAbility { .. }
                | ClientCommand::UseDiscipline { .. }
                | ClientCommand::AskDisciplineReuse { .. }
        )
    }

    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        let now = Instant::now();
        if let (Some(reuse), Message::KnightRefresh(remaining)) = (&mut self.knights, message) {
            reuse.received(*remaining, now);
        }
        let own = (self.use_disciplines == DisciplineUse::Titanium)
            .then(|| {
                world
                    .player
                    .as_ref()
                    .map(|player| player.spawn_id)
                    .or(world.own_spawn)
            })
            .flatten();
        self.disciplines.observe(message, own, now, !world.ready());
        Ok(())
    }

    fn admitted(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        self.report_knight_reuse(Instant::now(), world, out)?;
        self.disciplines.report(world.session_id, out)
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let handled = match command {
            ClientCommand::StopSong { .. } => self.stop_song(world, out),
            ClientCommand::StopCasting { .. } => self.stop_cast(world, out),
            ClientCommand::UseKnightAbility {
                ability, target, ..
            } => self.knight(command, *ability, *target, Instant::now(), world, out),
            ClientCommand::AskDisciplineReuse { .. } => self.ask_discipline_reuse(command, out),
            ClientCommand::UseDiscipline {
                spell_id,
                timer,
                target_id,
                timed_self,
                ..
            } => self.use_discipline(
                command,
                (*spell_id, *timer, *target_id, *timed_self),
                world,
                out,
            ),
            ClientCommand::UseItem(request) => self.use_item(request, Source::Item, world, out),
            ClientCommand::UseBeltItem(request) => self.use_belt_item(request, world, out),
            ClientCommand::DropInvisibility { steps, .. } => drop_invisibility(steps, out),
            _ => self.cast(command, world, out),
        };
        handled?;
        self.report_singing(world, out)
    }

    /// Releases a local hold when the server sends no matching reply.
    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        self.report_singing(world, out)?;
        if let Some(spell) = self.guard.expire_song_stop(now) {
            return report_unconfirmed_stop(world.session_id, Some(spell), out);
        }
        let Some(spell_id) = self.guard.expire(now) else {
            return Ok(());
        };
        out.log
            .send(ClientEvent::World(WorldEvent::CastRequestUnconfirmed {
                session_id: world.session_id,
                spell_id,
            }))?;
        out.log
            .diagnostic("Cast acknowledgement timed out; a manual retry is available".into())
    }

    /// Withholds a start for an abandoned request even while a different
    /// spell is pending or casting.
    fn explain(&mut self, message: &mut Message, world: &World) {
        let Message::Event(WorldEvent::Spell(SpellUpdate::Began {
            caster_id,
            spell_id,
            ..
        })) = message
        else {
            return;
        };
        if world.is_player(*caster_id) && self.guard.late_begin == Some(u32::from(*spell_id)) {
            self.guard.late_begin = None;
            *message = Message::Withheld;
        }
    }

    /// Follows the player's own casts; dying ends any cast.
    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.admit(message, world)?;
        if let Message::Event(WorldEvent::Spell(SpellUpdate::Mana {
            spell_id,
            keep_casting: false,
        })) = message
        {
            if let Some(song) = self.song_reported(*spell_id, world) {
                self.report_song_stopped(song, world, out)?;
            }
        }
        // A forget's answer stops the song as the program's 0x308e
        // handler does (kind 2, 0x4eb9b7, read R49-21c); a memorize's
        // leaves it.
        if let Message::Event(WorldEvent::Spell(SpellUpdate::Slot { mode: 2, .. })) = message {
            if self.singing(world).is_some() {
                self.stop_song(world, out)?;
            }
        }
        if let Message::Event(WorldEvent::Spell(SpellUpdate::Began {
            caster_id,
            spell_id,
            duration_ms,
        })) = message
        {
            if world.is_player(*caster_id) {
                self.song_begun(u32::from(*spell_id), *duration_ms);
            }
        }
        match message {
            Message::KnightRefresh(_) => self.report_knight_reuse(Instant::now(), world, out)?,
            Message::Event(WorldEvent::Spell(update)) => {
                let Some(own) = world.player.as_ref().map(|player| player.spawn_id) else {
                    return Ok(());
                };
                let pending = self.guard.pending();
                let event = match self.guard.observe(own, update) {
                    Some(Followup::LateBegin(requested_spell)) => Some(WorldEvent::CastLateBegin {
                        session_id: world.session_id,
                        requested_spell,
                    }),
                    Some(Followup::Superseded(spell_id)) => Some(WorldEvent::CastSuperseded {
                        session_id: world.session_id,
                        spell_id,
                    }),
                    None if pending.is_some() && self.guard.pending().is_none() => {
                        Some(WorldEvent::CastPending {
                            session_id: world.session_id,
                            spell_id: None,
                        })
                    }
                    None => None,
                };
                if let Some(event) = event {
                    out.log.send(ClientEvent::World(event))?;
                }
            }
            Message::Event(event) if world.owns_death(event) => {
                self.disciplines.clear_active();
                self.guard.clear();
            }
            _ => (),
        }
        self.report_singing(world, out)
    }
}

/// Whether a song kept is of the class's spell: the host's installed word
/// for an item's song (6.542), else a memorized spell (6.442). A gem's song
/// is always the class's: only the class's spells memorize [I], and the
/// gem may have been forgotten since.
fn class_song(song: Song, world: &World) -> bool {
    song.installed_class_spell.unwrap_or_else(|| {
        !song.item
            || world
                .player
                .as_ref()
                .is_some_and(|p| p.memorized_spells.contains(&Some(song.spell)))
    })
}

/// What the log says when a gem click stopped the song instead; the host
/// is told by `WorldEvent::SongStopped`.
const SONG_STOPPED: &str = "The song stopped; the gem did not cast";

/// The program's line at a song stop of mode 0 or 3 (0x40f06d, read R49-21).
const SONG_STOP_LINE: u32 = 0x318f;

fn report_unconfirmed_stop(
    session_id: u64,
    spell_id: Option<u32>,
    out: &mut Out<'_, '_>,
) -> Result<()> {
    out.log
        .send(ClientEvent::World(WorldEvent::SongStopUnconfirmed {
            session_id,
            spell_id,
        }))
}

/// Sends the steps of an invisibility drop in the host's order, where the
/// client generation sends them. Every step is encoded before any is sent,
/// so a generation that sends none, or a step it cannot carry, sends
/// nothing and is only noted.
fn drop_invisibility(
    steps: &[eq_network_game::buffs::InvisibilityStep],
    out: &mut Out<'_, '_>,
) -> Result<()> {
    let packets: Result<Vec<_>> = steps
        .iter()
        .map(|step| out.encode(&Request::DropInvisibility(*step)))
        .collect();
    match packets {
        Ok(packets) => packets.iter().try_for_each(|packet| out.send(packet)),
        Err(error) => out
            .log
            .diagnostic(format!("Invisibility not dropped: {error}")),
    }
}

#[cfg(test)]
mod song_stop_tests;

#[cfg(test)]
mod song_rules_tests;

#[cfg(test)]
mod cast_stop_tests;

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;

    #[test]
    fn a_received_interrupt_resolves_a_requested_song_stop() {
        let mut casting = Casting::with_song_stop();
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        player.class = Some(BARD_CLASS);
        player.memorized_spells[2] = Some(202);
        world.player.admit(player);
        let now = Instant::now();
        let cast = ClientCommand::CastSpell {
            session_id: 5,
            gem: 2,
            spell_id: 202,
            target_id: 7,
            created: now,
        };
        let stop = ClientCommand::StopSong {
            session_id: 5,
            created: now,
        };
        assert!(casting.owns(&stop));
        assert!(!casting.guard.blocks(&stop));

        testing::run(|out| casting.handle(&cast, &mut world, out))
            .result
            .unwrap();
        let output = testing::run(|out| casting.handle(&stop, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent.len(), 1);
        assert_eq!(output.sent[0].opcode, 0x4839);
        assert_eq!(output.sent[0].body, [] as [u8; 0]);
        assert!(output.events.is_empty());
        assert_eq!(casting.guard.pending(), Some(202));

        let duplicate = testing::run(|out| casting.handle(&stop, &mut world, out));
        duplicate.result.unwrap();
        assert_eq!(duplicate.sent, []);
        assert!(matches!(
            duplicate.events.as_slice(),
            [ClientEvent::Diagnostic(reason)] if reason == "No song is playing"
        ));

        let refresh = |slot| {
            Message::Event(WorldEvent::Spell(SpellUpdate::BarRefresh {
                slot,
                spell_id: 202,
                reduction_ms: 0,
            }))
        };
        testing::run(|out| casting.observe(&refresh(1), &mut world, out))
            .result
            .unwrap();
        assert_eq!(casting.guard.pending(), Some(202));

        let interrupted = Message::Event(WorldEvent::Spell(SpellUpdate::Interrupted {
            caster_id: 7,
            message_id: 1,
            caster_name: None,
        }));
        testing::run(|out| casting.observe(&interrupted, &mut world, out))
            .result
            .unwrap();
        assert!(!casting.guard.active());

        // The song stopped with the first stop; an interrupt does not bring
        // it back (read R16).
        let output = testing::run(|out| casting.handle(&stop, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent, []);
        let output =
            testing::run(|out| casting.tick(Instant::now() + SONG_STOP_WAIT, &mut world, out));
        output.result.unwrap();
        assert!(output.events.is_empty());

        let unavailable = testing::run(|out| Casting::default().handle(&stop, &mut world, out));
        unavailable.result.unwrap();
        assert_eq!(unavailable.sent, []);
        assert!(matches!(
            unavailable.events.as_slice(),
            [ClientEvent::Diagnostic(reason)] if reason == "Stopping songs is unavailable on this server"
        ));

        world.player.admit(testing::player(7));
        let output = testing::run(|out| casting.handle(&stop, &mut world, out));
        output.result.unwrap();
        assert_eq!(
            output.sent,
            [] as [eq_network_game::command::EncodedCommand; 0]
        );
        assert!(matches!(
            output.events.as_slice(),
            [ClientEvent::Diagnostic(reason)] if reason == "Only Bards can stop a song"
        ));

        // The requested gem's refresh still completes the normal cast path.
        casting.guard.submitted(202, Some(Source::Gem(2)), now);
        testing::run(|out| casting.observe(&refresh(2), &mut world, out))
            .result
            .unwrap();
        assert!(!casting.guard.active());
    }

    #[test]
    fn an_item_song_of_a_class_spell_stops_as_the_program_stops_it() {
        use eq_network_game::inventory::{
            ClickEffect, ClickKind, Inventory, InventorySlot, InventoryUpdate,
        };

        let mut casting = Casting::with_song_stop();
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        player.class = Some(BARD_CLASS);
        player.memorized_spells[2] = Some(202);
        world.player.admit(player);

        let slot = InventorySlot(22);
        let mut item = testing::item(slot.0);
        item.activation.effect = Some(ClickEffect {
            spell_id: 202,
            kind: ClickKind::Expendable,
            required_level: 0,
            effect_level: 1,
            cast_time_ms: 1000,
            recast_delay_seconds: 0,
            recast_type: 0,
        });
        item.charges = 1;
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![item]));
        world.inventory = inventory.into();
        let now = Instant::now();
        let item = ClientCommand::UseItem(ItemUse {
            installed_class_spell: None,
            request_id: 1,
            session_id: 5,
            revision: world.inventory.revision(),
            slot,
            target_id: 7,
            created: now,
        });
        let stop = ClientCommand::StopSong {
            session_id: 5,
            created: now,
        };

        let output = testing::run(|out| casting.handle(&item, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent.len(), 1);
        assert_eq!(casting.guard.pending(), Some(202));

        let refresh = |slot| {
            Message::Event(WorldEvent::Spell(SpellUpdate::BarRefresh {
                slot,
                spell_id: 202,
                reduction_ms: 0,
            }))
        };
        // The gem's refresh does not end the item's cast.
        testing::run(|out| casting.observe(&refresh(2), &mut world, out))
            .result
            .unwrap();
        assert!(casting.guard.active());
        // The item casts a spell of the class, which the program stops with
        // the empty request even while the item's cast is held (read R16).
        let output = testing::run(|out| casting.handle(&stop, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent.len(), 1);
        assert_eq!(output.sent[0].opcode, 0x4839);
        assert_eq!(output.sent[0].body, [] as [u8; 0]);

        testing::run(|out| casting.observe(&refresh(10), &mut world, out))
            .result
            .unwrap();
        assert!(!casting.guard.active());
        let output = testing::run(|out| casting.handle(&stop, &mut world, out));
        output.result.unwrap();
        assert_eq!(output.sent, []);
    }

    #[test]
    fn a_memorized_spell_goes_out_and_holds_casting_until_answered() {
        let mut casting = Casting::default();
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        player.memorized_spells[0] = Some(202);
        world.player.admit(player);
        let cast = |gem| ClientCommand::CastSpell {
            session_id: 5,
            gem,
            spell_id: 202,
            target_id: 7,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| casting.handle(&cast(0), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(WorldEvent::CastPending {
                spell_id: Some(202),
                ..
            })]
        ));
        assert_eq!(casting.holds(&world, Instant::now()).len(), 1);
        // A gem that does not hold the spell sends nothing and says why.
        let outcome = testing::run(|out| casting.handle(&cast(1), &mut world, out));
        outcome.result.unwrap();
        assert!(
            outcome.sent.is_empty(),
            "expected nothing sent, got {:?}",
            outcome.sent
        );
        assert!(matches!(
            outcome.events[..],
            [
                ClientEvent::World(WorldEvent::CastRejected { .. }),
                ClientEvent::Diagnostic(_)
            ]
        ));
    }

    /// The Titanium program refuses a cast while the player lies, with its
    /// own line, and neither stands them nor casts (read R19 d).
    #[test]
    fn titanium_refuses_a_cast_while_the_player_lies() {
        let mut casting = Casting::default();
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        player.memorized_spells[0] = Some(202);
        world.player.admit(player);
        world.posture.observed(PostureState::Lying);
        let cast = ClientCommand::CastSpell {
            session_id: 5,
            gem: 0,
            spell_id: 202,
            target_id: 7,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| casting.handle(&cast, &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert!(matches!(
            outcome.events[..],
            [
                ClientEvent::World(WorldEvent::CastRejected {
                    spell_id: 202,
                    string_id: Some(0x3099),
                    ..
                }),
                ClientEvent::Diagnostic(_)
            ]
        ));
        assert_eq!(world.posture.current(), Some(PostureState::Lying));
        assert_eq!(casting.holds(&world, Instant::now()).len(), 0);
        // The Mac-era client's rule is unread, so it casts as before.
        let outcome = testing::run_on(&super::super::wire::EqMac, |out| {
            casting.handle(&cast, &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
    }

    #[test]
    fn an_item_send_stamps_the_item_and_its_type_with_its_recast() {
        // Read R49-34 k: now plus the delay, in seconds, for the item and
        // its recast type; another item of the type waits too.
        let mut recasts = ItemRecasts::default();
        let mut item = testing::item(23);
        item.activation.effect = Some(eq_network_game::inventory::ClickEffect {
            spell_id: 73,
            kind: eq_network_game::inventory::ClickKind::Click,
            required_level: 0,
            effect_level: 1,
            cast_time_ms: 0,
            recast_delay_seconds: 30,
            recast_type: 4,
        });
        let slot = InventorySlot(23);
        assert_eq!(recasts.ready(slot, &item), 0);
        recasts.sent(slot, &item, 1000);
        assert_eq!(recasts.ready(slot, &item), 1030);
        let mut other = item.clone();
        other.details.id += 1;
        assert_eq!(recasts.ready(InventorySlot(24), &other), 1030);
        other.activation.effect.as_mut().unwrap().recast_type = 5;
        assert_eq!(recasts.ready(InventorySlot(24), &other), 0);
        // A type past the program's 20 keeps only the item's own stamp.
        item.activation.effect.as_mut().unwrap().recast_type = 40;
        recasts.sent(InventorySlot(25), &item, 2000);
        assert_eq!(recasts.ready(InventorySlot(25), &item), 2030);
    }

    #[test]
    fn titanium_refuses_an_item_click_while_the_player_lies() {
        let mut casting = Casting::default();
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        world.posture.observed(PostureState::Lying);
        let item = ClientCommand::UseItem(ItemUse {
            installed_class_spell: None,
            request_id: 3,
            session_id: 5,
            revision: 1,
            slot: eq_network_game::inventory::InventorySlot(22),
            target_id: 7,
            created: Instant::now(),
        });
        let outcome = testing::run(|out| casting.handle(&item, &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert!(matches!(
            &outcome.events[..],
            [ClientEvent::World(WorldEvent::ItemUseAction {
                request_id: 3,
                error: Some(_),
                string_id: Some(0x3099),
                ..
            })]
        ));
        assert_eq!(casting.holds(&world, Instant::now()).len(), 0);
    }

    #[test]
    fn takps_wire_casts_a_gem_in_its_own_packet_and_the_spell_bar_ends_the_hold() {
        let eqmac = &super::super::wire::EqMac;
        let mut casting = Casting::default();
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        player.memorized_spells[0] = Some(202);
        world.player.admit(player);
        let cast = ClientCommand::CastSpell {
            session_id: 5,
            gem: 0,
            spell_id: 202,
            target_id: 7,
            created: Instant::now(),
        };
        let outcome = testing::run_on(eqmac, |out| casting.handle(&cast, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
        assert_eq!(outcome.sent[0].opcode, 0x7e41);
        assert_eq!(
            outcome.sent[0].body,
            [0, 0, 202, 0, 255, 255, 7, 0, 0, 0, 0, 0]
        );
        assert!(casting.guard.active());
        // TAKP's spell bar coming back, as the EQMac wire reads it.
        for message in eq_network_game::message::eqmac(0x7f41, &[90, 0, 202, 0]) {
            testing::run_on(eqmac, |out| casting.observe(&message, &mut world, out))
                .result
                .unwrap();
        }
        assert!(!casting.guard.active());
    }

    #[test]
    fn rejection_preserves_request_identity_without_changing_cast_exclusion() {
        let request = ClientCommand::CastSpell {
            session_id: 12,
            gem: 0,
            spell_id: 73,
            target_id: 7,
            created: Instant::now(),
        };
        let mut guard = CastGuard::default();
        guard.submitted(42, None, Instant::now());
        assert!(matches!(rejected(&request, "Target unavailable"),
            Some(crate::world::WorldEvent::CastRejected { session_id: 12, spell_id: 73, reason, .. })
            if reason == "Target unavailable"));
        assert!(guard.blocks(&request));
        assert!(rejected(
            &ClientCommand::SelectTarget {
                request_id: 1,
                session_id: 12,
                spawn_id: Some(7),
                expected_pet: None,
            },
            "Unavailable"
        )
        .is_none());
    }

    #[test]
    fn item_activations_share_cast_exclusion_without_blocking_posture_interrupts() {
        let now = Instant::now();
        let request = ClientCommand::UseItem(eq_network_game::inventory::ItemUse {
            installed_class_spell: None,
            request_id: 1,
            session_id: 12,
            revision: 1,
            slot: eq_network_game::inventory::InventorySlot(22),
            target_id: 7,
            created: now,
        });
        let mut guard = CastGuard::default();
        assert!(!guard.blocks(&request));
        guard.submitted(73, None, now);
        assert!(guard.blocks(&request));
        assert!(!guard.blocks(&ClientCommand::SetPosture {
            session_id: 12,
            spawn_id: 7,
            posture: eq_network_game::command::Posture::Ducking,
            created: now,
        }));
        guard.observe(
            7,
            &SpellUpdate::Interrupted {
                caster_id: 7,
                message_id: 1,
                caster_name: None,
            },
        );
        assert!(!guard.blocks(&request));
    }

    fn begin(caster_id: u16) -> SpellUpdate {
        SpellUpdate::Began {
            caster_id,
            spell_id: 73,
            duration_ms: 0,
        }
    }

    #[test]
    fn submission_blocks_until_acknowledged_and_only_silent_requests_expire() {
        let now = Instant::now();
        let mut guard = CastGuard::default();
        guard.submitted(73, None, now);
        assert!(guard.active());
        assert_eq!(guard.pending(), Some(73));
        assert_eq!(guard.expire(now + Duration::from_millis(4999)), None);
        assert_eq!(guard.expire(now + ACK_TIMEOUT), Some(73));
        assert!(!guard.active());
        assert_eq!(guard.expire(now + ACK_TIMEOUT), None);
        guard.submitted(73, None, now);
        guard.observe(12, &begin(13));
        assert_eq!(guard.pending(), Some(73));
        guard.observe(12, &begin(12));
        assert_eq!(guard.pending(), None);
        assert!(guard.active());
        assert_eq!(guard.expire(now + Duration::from_secs(60)), None);
        guard.observe(
            12,
            &SpellUpdate::Interrupted {
                caster_id: 12,
                message_id: 1,
                caster_name: None,
            },
        );
        assert!(!guard.active());
    }

    #[test]
    fn instant_cast_results_release_submission_without_a_begin_packet() {
        let mut guard = CastGuard::default();
        for update in [
            SpellUpdate::Mana {
                spell_id: 73,
                keep_casting: false,
            },
            SpellUpdate::BarRefresh {
                slot: 0,
                spell_id: 73,
                reduction_ms: 0,
            },
        ] {
            guard.submitted(73, None, Instant::now());
            guard.observe(12, &update);
            assert!(!guard.active());
            assert_eq!(guard.pending(), None);
        }
    }

    #[test]
    fn nearby_casts_and_unrelated_results_cannot_unlock_our_cast() {
        let mut guard = CastGuard::default();
        guard.observe(12, &begin(13));
        assert!(!guard.active());
        guard.observe(12, &begin(12));
        for update in [
            begin(13),
            SpellUpdate::Interrupted {
                caster_id: 13,
                message_id: 1,
                caster_name: None,
            },
            SpellUpdate::Mana {
                spell_id: 73,
                keep_casting: true,
            },
            SpellUpdate::Mana {
                spell_id: 74,
                keep_casting: false,
            },
            SpellUpdate::BarRefresh {
                slot: 0,
                spell_id: 74,
                reduction_ms: 0,
            },
            SpellUpdate::Slot {
                slot: 0,
                spell_id: 73,
                mode: 1,
            },
        ] {
            guard.observe(12, &update);
            assert!(guard.active());
        }
        guard.observe(
            12,
            &SpellUpdate::Mana {
                spell_id: 73,
                keep_casting: false,
            },
        );
        assert!(!guard.active());
    }

    #[test]
    fn a_cast_begun_under_another_spell_ends_with_the_requested_ones_result() {
        // TAKP begins a Luclin port as spell 2935 and ends it naming the port.
        let begun_as = |spell_id| SpellUpdate::Began {
            caster_id: 12,
            spell_id,
            duration_ms: 10_000,
        };
        let ended = |spell_id| SpellUpdate::Mana {
            spell_id,
            keep_casting: false,
        };
        for end in [73, 2935] {
            let mut guard = CastGuard::default();
            guard.submitted(73, None, Instant::now());
            guard.observe(12, &begun_as(2935));
            guard.observe(12, &ended(74));
            assert!(guard.active());
            guard.observe(12, &ended(end));
            assert!(!guard.active());
        }
    }

    #[test]
    fn interrupt_refresh_and_lifecycle_reset_release_the_guard() {
        let mut guard = CastGuard::default();
        for update in [
            SpellUpdate::Interrupted {
                caster_id: 12,
                message_id: 1,
                caster_name: None,
            },
            SpellUpdate::BarRefresh {
                slot: 0,
                spell_id: 73,
                reduction_ms: 0,
            },
        ] {
            guard.observe(12, &begin(12));
            assert!(guard.active());
            guard.observe(12, &update);
            assert!(!guard.active());
        }
        guard.observe(12, &begin(12));
        guard.clear();
        assert!(!guard.active());
    }

    #[test]
    fn confirmed_cast_blocks_spell_commands_but_allows_ducking_and_targeting() {
        let created = std::time::Instant::now();
        let commands = [
            ClientCommand::CastSpell {
                session_id: 1,
                gem: 0,
                spell_id: 73,
                target_id: 12,
                created,
            },
            ClientCommand::MemorizeSpell {
                session_id: 1,
                gem: 0,
                spell_id: 73,
                created,
            },
            ClientCommand::ForgetSpell {
                session_id: 1,
                gem: 0,
                spell_id: 73,
                created,
            },
            ClientCommand::DeleteSpell {
                session_id: 1,
                slot: 0,
                spell_id: 73,
                created,
            },
            ClientCommand::ScribeSpell {
                session_id: 1,
                revision: 0,
                slot: 0,
                spell_id: 73,
                created,
            },
        ];
        let mut guard = CastGuard::default();
        assert!(commands.iter().all(|command| !guard.blocks(command)));
        guard.observe(12, &begin(12));
        assert!(commands.iter().all(|command| guard.blocks(command)));
        assert!(!guard.blocks(&ClientCommand::SetPosture {
            session_id: 1,
            spawn_id: 12,
            posture: eq_network_game::command::Posture::Ducking,
            created,
        }));
        assert!(!guard.blocks(&ClientCommand::SelectTarget {
            request_id: 1,
            session_id: 1,
            spawn_id: Some(13),
            expected_pet: None,
        }));
        guard.observe(
            12,
            &SpellUpdate::Interrupted {
                caster_id: 12,
                message_id: 1,
                caster_name: None,
            },
        );
        assert!(commands.iter().all(|command| !guard.blocks(command)));
    }
}
