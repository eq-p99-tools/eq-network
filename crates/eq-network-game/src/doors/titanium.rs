//! Titanium's own door mechanisms: how far and how fast each open type moves,
//! which ones a player stands on or walks into, and when the client sends a
//! door back on its own.
//!
//! Read from `eqgame.exe` (Titanium): the per-frame door step 0x4368b3, the
//! floor-sweep filter 0x42d05f with its door test 0x490370, the wall filter
//! 0x42cdb1 and the return timer 0x48faa5 (read R1, 11-09, and the portcullis
//! follow-up read, 11-23, which corrects R1 on walls). The program moves a
//! door only on the server's 0x700d action; a click sends 0x043b and changes
//! nothing on its own.

use super::{Door, CLICK_OPCODE};
use crate::command::EncodedCommand;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Program units a door steps in a second for each unit of rate: each frame
/// steps elapsed ms x 0.02 x rate (0x4368b3).
const UNITS_PER_RATE: f32 = 20.0;

/// The door's own signed field at +0x66: the low 16 bits of the spawn
/// entry's param (entry bytes 64-65, set up by 0x490a43; read R1b). Some types
/// take their travel from it, and the return timer reads it.
#[must_use]
pub const fn field(door: &Door) -> i16 {
    #[allow(clippy::cast_possible_truncation)]
    let low = door.parameter as u16;
    low.cast_signed()
}

/// Whether the program's door mover passes this door by every frame: the
/// types 0x4903d4 holds for (50-58 but 53, and 153-155). It neither moves
/// nor times them, so they stand at their base and never go back on their
/// own (0x4368b3, read R49-28b). These are action doors (walk-in zone
/// points, teleports, levers), whose state is a mark rather than a pose,
/// inferred.
#[must_use]
pub const fn mover_skips(door: &Door) -> bool {
    matches!(door.open_type, 50..=52 | 54..=58 | 153..=155)
}

/// The program's state for a door the server has not moved since it
/// spawned (0x490670, read R49-28): spawn states 1 and 2 become 1 for the
/// [`mover_skips`] types and 2 (opening) for every other type, 3 becomes 0,
/// and any other state is kept. None once the server has moved the door:
/// the program's state then is the server's action as its mover steps it.
#[must_use]
pub const fn spawned_state(door: &Door) -> Option<u8> {
    if door.action.is_some() {
        return None;
    }
    Some(match door.state_at_spawn {
        1 | 2 if mover_skips(door) => 1,
        1 | 2 => 2,
        3 => 0,
        state => state,
    })
}

/// The state the program's mover sends a door back to on its own: the
/// door's rest state +0x4c, which 0x490a43 copies from the record's +0x3f
/// (read R49-28b), the byte the table calls `invert_state`.
#[must_use]
pub const fn rest_state(door: &Door) -> u8 {
    door.invert_state
}

/// Which end a Titanium door stands at or heads for: open (true) or at its
/// base (false). A door the server has moved follows its action
/// ([`Door::active_endpoint`]). One it hasn't follows its
/// [`spawned_state`]: a door spawned opening (2) runs from its base to its
/// end, and one in state 0, or a [`mover_skips`] door in state 1, stands at
/// its base (read R49-28b). Any other state, such as 4, is None.
#[must_use]
pub fn endpoint(door: &Door) -> Option<bool> {
    match spawned_state(door) {
        None => door.active_endpoint(),
        Some(0 | 1) => Some(false),
        Some(2) => Some(true),
        Some(_) => None,
    }
}

/// Whether the program's mover sends this door closing as soon as it stands
/// open: types 110-152 but 115, which it turns from state 1 to 3 every frame
/// (0x4368b3, read R49-28b).
const fn closes_once_open(door: &Door) -> bool {
    matches!(door.open_type, 0x6e..=0x98) && door.open_type != 0x73
}

/// When the program's mover sends a door back on its own, for a leg begun at
/// `at` that arrives at `arrival`, and whether it goes back toward its open
/// end (its rest is open) or its base.
///
/// - A [`mover_skips`] door never goes back.
/// - A door not moved since it spawned goes back once its delay has passed
///   since it spawned (+0x98, set at spawn), when its [`spawned_state`]
///   isn't its [`rest_state`]. That holds even while it is still opening
///   (read R49-28b). The program's close target is state 3 for rest 0 and
///   state 2 (opening) for rest 1. A rest state above 1, and the 60 s of a
///   door in state 4, aren't built: such a door doesn't go back.
/// - A door the server moved keeps read R1b's rule: the timer runs from the
///   action and again from arrival. Whether arrival re-arms +0x98 isn't in
///   read R49-28b.
/// - A [`closes_once_open`] door goes back closing at its arrival at the
///   open end, if nothing sent it back sooner. Only a vertical door's
///   arrival is known; one that swings or slides keeps its timer alone
///   (its swing's time isn't stepped here, recording checklist 6.424).
fn timed_return(door: &Door, at: Instant, arrival: Instant) -> Option<(Instant, bool)> {
    if mover_skips(door) {
        return None;
    }
    let timer = match spawned_state(door) {
        Some(state) => {
            let rest = rest_state(door);
            (state != rest && rest <= 1 && state != 4)
                .then(|| returns_after(door))
                .flatten()
                .map(|delay| at + delay)
        }
        None => away(door)
            .then(|| returns_after(door))
            .flatten()
            .map(|delay| {
                if at + delay < arrival {
                    at + delay
                } else {
                    arrival + delay
                }
            }),
    }
    .map(|when| (when, return_action(door) == OPEN));
    let once_open =
        (closes_once_open(door) && Vertical::of(door).is_some() && endpoint(door) == Some(true))
            .then_some((arrival, false));
    match (timer, once_open) {
        (Some(timer), Some(once_open)) => Some(if once_open.0 < timer.0 {
            once_open
        } else {
            timer
        }),
        (timer, once_open) => timer.or(once_open),
    }
}

/// Whether the program ignores a click on this door, saying nothing and
/// sending nothing: a door in state 1 whose [`field`] is -2 (0x4906c5, read
/// R49-28). The state is known only until the server first moves the door
/// ([`spawned_state`]); after that the rule is not applied.
#[must_use]
pub const fn ignores_clicks(door: &Door) -> bool {
    field(door) == -2 && matches!(spawned_state(door), Some(1))
}

/// The travel a door takes from [`field`] (0x49030e): its size, except that
/// -1, -2 and -3 give none.
fn field_travel(door: &Door) -> f32 {
    match field(door) {
        -3..=-1 => 0.0,
        value => f32::from(value.unsigned_abs()),
    }
}

/// [`field_travel`], or 22 where that is none: types 75-81.
fn field_travel_or_22(door: &Door) -> f32 {
    match field_travel(door) {
        0.0 => 22.0,
        travel => travel,
    }
}

/// A door that rises or sinks along Z when it opens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertical {
    /// How far the door's Z moves from its placed height when fully open:
    /// positive rises, negative sinks.
    pub travel: f32,
    /// Units a second while opening.
    pub open_speed: f32,
    /// Units a second while closing.
    pub close_speed: f32,
}

impl Vertical {
    const fn new(travel: f32, open_rate: f32, close_rate: f32) -> Self {
        Self {
            travel,
            open_speed: open_rate * UNITS_PER_RATE,
            close_speed: close_rate * UNITS_PER_RATE,
        }
    }

    /// The door's vertical motion as the program steps it (0x4368b3).
    #[must_use]
    pub fn of(door: &Door) -> Option<Self> {
        Some(match door.open_type {
            59 => Self::new(field_travel(door), 0.45, 0.45),
            60..=62 => Self::new(8.0, 0.45, 0.25),
            65..=67 => Self::new(10.0, 0.45, 0.25),
            70..=72 => Self::new(14.0, 0.45, 0.25),
            75..=77 => Self::new(field_travel_or_22(door), 0.45, 0.25),
            78 | 79 => Self::new(field_travel_or_22(door), 0.45, 0.45),
            80 | 81 => Self::new(-field_travel_or_22(door), 0.45, 0.45),
            120 | 140 => Self::new(-10.0, 1.0, 0.25),
            145 => Self::new(-10.0, 0.25, 0.25),
            146 => Self::new(-10.0, 1.0, 1.0),
            // The program's other types in these ranges have no case of
            // their own and take its default step.
            63 | 64 | 68 | 69 | 73 | 74 | 141..=144 => Self::new(128.0, 0.25, 0.25),
            _ => return None,
        })
    }

    /// The speed a move toward `to` goes at: opening toward the travel,
    /// closing toward anything else.
    #[allow(
        clippy::float_cmp,
        reason = "A move's end is the travel itself or 0, assigned exactly"
    )]
    fn speed_toward(&self, to: f32) -> f32 {
        if to == self.travel {
            self.open_speed
        } else {
            self.close_speed
        }
    }

    /// How long the move from `from` to `to` takes.
    fn time(&self, from: f32, to: f32) -> Duration {
        Duration::from_secs_f32((to - from).abs() / self.speed_toward(to))
    }
}

/// A door that slides sideways when it opens (0x4366e8).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slide {
    /// How far it slides when fully open, along [`Slide::direction`].
    pub distance: f32,
    /// Units a second, both ways: 0.35 a 50 ms step.
    pub speed: f32,
    /// The direction it slides, in EQ X, Y and Z.
    pub direction: [f32; 3],
}

impl Slide {
    /// Type 15 slides 10 units; 78-81 slide [`field`]'s size or 22, and 79
    /// and 81 slide the other way.
    #[must_use]
    pub fn of(door: &Door) -> Option<Self> {
        let (distance, reversed) = match door.open_type {
            15 => (10.0, false),
            78 | 80 => (field_travel_or_22(door), false),
            79 | 81 => (field_travel_or_22(door), true),
            _ => return None,
        };
        // The axis follows the door's heading in 512ths of a turn.
        #[allow(clippy::float_cmp)]
        let [x, y] = if door.position.heading == 128.0 {
            [0.0, 1.0]
        } else if door.position.heading == 384.0 {
            [0.0, -1.0]
        } else if door.position.heading == 256.0 {
            [1.0, 0.0]
        } else {
            [-1.0, 0.0]
        };
        let sign = if reversed { -1.0 } else { 1.0 };
        Some(Self {
            distance,
            speed: 0.35 * UNITS_PER_RATE,
            direction: [x * sign, y * sign, 0.0],
        })
    }
}

/// Whether a door holds the player up, stops the player walking, or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Solidity {
    /// The floor sweep stands the player on it.
    pub floor: bool,
    /// The wall sweep stops the player at it.
    pub wall: bool,
}

impl Default for Solidity {
    fn default() -> Self {
        Self::BOTH
    }
}

impl Solidity {
    /// Both a floor and a wall: how a client without a read rule treats any
    /// door it draws solid.
    pub const BOTH: Self = Self {
        floor: true,
        wall: true,
    };

    /// The program's rule for a door of `open_type` in zone `zone_id`.
    ///
    /// The collision move 0x48acef stops the player at every door: its one
    /// filter, 0x42cdb1, collides with any door actor whatever its open type
    /// or state, so a shut portcullis (type 65) is a wall and a raised one is
    /// passed under by its pose (portcullis follow-up read, which corrects
    /// R1's "wall contact skips 60-77").
    ///
    /// The floor sweep takes a door only when 0x490370 holds: types 30-45,
    /// 50-51, 54-59 and 153-155, and three zones' own types. Its second
    /// filter 0x42ce5d is not an exclusion: a 0 there means keep looking,
    /// and the hit already written stands (read F4b), so it changes no
    /// door's floor.
    #[must_use]
    pub const fn of(open_type: u8, zone_id: u16) -> Self {
        let floor = matches!(open_type, 30..=45 | 50 | 51 | 54..=59 | 153..=155)
            || matches!((open_type, zone_id), (27, 154) | (26, 205 | 212) | (8, 208));
        Self { floor, wall: true }
    }
}

/// How long after its last state change the client sends a door back to its
/// spawn state on its own (0x48faa5); none means it never does.
#[must_use]
pub fn returns_after(door: &Door) -> Option<Duration> {
    let field = field(door);
    if matches!(field, -3 | -2) {
        return None;
    }
    Some(match door.open_type {
        59 if field > 170 => return None,
        40 | 45 | 50 if field == 32001 => Duration::from_mins(30),
        40 | 45 | 50 if field > 0 => Duration::from_millis(field.unsigned_abs().into()),
        0 | 5 => Duration::from_secs(10),
        35 | 36 | 116 | 145 | 151 => Duration::from_secs(6),
        57 | 58 | 140 => Duration::from_secs(2),
        105..=107 => return None,
        115 => Duration::from_millis(1),
        130 => Duration::from_millis(800),
        150 => Duration::from_secs(4),
        120 | 125 | 135 | 146 | 152 => Duration::from_secs(3),
        _ => Duration::from_secs(20),
    })
}

/// How long a door takes to reach its other end, which restarts its return
/// timer: the longer of its rise and its slide. A hinged door's swing time is
/// not read, so it counts as none (recording checklist 6.424).
fn travel_time(door: &Door, from: f32, to: f32) -> Duration {
    let vertical = Vertical::of(door).map_or(Duration::ZERO, |motion| motion.time(from, to));
    let slide = Slide::of(door).map_or(Duration::ZERO, |slide| {
        Duration::from_secs_f32(slide.distance / slide.speed)
    });
    vertical.max(slide)
}

/// When a door spawned, or moved by an action, at `at` goes back on its
/// own, if it does (`timed_return`). A door spawned away from its rest
/// counts its delay from its spawn (read R49-28b). One the server moved
/// restarts it with each state change, so it runs from the action and
/// again from arrival, and a delay shorter than the travel fires before the
/// door arrives (read R1b item 4).
#[must_use]
pub fn returns_at(door: &Door, at: Instant) -> Option<Instant> {
    let (from, to) = Vertical::of(door).map_or((0.0, 0.0), |motion| (0.0, motion.travel));
    timed_return(door, at, at + travel_time(door, from, to)).map(|(when, _)| when)
}

/// Whether the client's own return of this door is a click to the server
/// rather than a local move: types 40 and 45 (0x490b20, read R1c). Every other
/// type goes back locally and sends nothing.
#[must_use]
pub const fn returns_by_click(door: &Door) -> bool {
    matches!(door.open_type, 40 | 45)
}

/// The click the client's return of a type 40 or 45 door sends: 0x043b with the
/// door id, 0xfe where a player click has its own arguments, and zeros. It
/// names no player and skips the reach check; the door then moves only on the
/// server's action, as for a player's click (0x4906c5, read R1c).
#[must_use]
pub fn return_click(id: u8) -> EncodedCommand {
    let mut body = vec![0; 16];
    body[0] = id;
    body[4..8].copy_from_slice(&0xfe_i32.to_le_bytes());
    EncodedCommand {
        opcode: CLICK_OPCODE,
        body,
    }
}

/// The server's open and close actions.
const OPEN: u8 = 2;
const CLOSE: u8 = 3;

/// Whether a door stands away from its rest: open, or for a door the
/// server inverts (normally open) shut, by the latest action or, before
/// one, the state it spawned in. The client sends such a door back on its
/// own.
#[must_use]
pub fn away(door: &Door) -> bool {
    let inverted = door.invert_state != 0;
    match door.action {
        Some(action) => action == if inverted { CLOSE } else { OPEN },
        None => (door.state_at_spawn != 0) != inverted,
    }
}

/// The action the client's own return applies: a door goes back to its rest,
/// which for an inverted door is the plain open action.
#[must_use]
pub const fn return_action(door: &Door) -> u8 {
    if door.invert_state == 0 {
        CLOSE
    } else {
        OPEN
    }
}

/// How near a spawn must stand to a moving door for the program to sweep it
/// onto the door's new floor (0x4368b3): the spawn's model radius, or 10,
/// plus the door model's radius, plus 200. The door's radius is a call on its
/// model (read R1b), which the session does not have; this takes the
/// program's value for a door without one, 100, and the 10, measured in three
/// dimensions from the door's current position (recording checklist 6.426).
/// A door whose radius the host gave ([`Lifts::set_radii`]) takes that
/// radius instead of the 100.
pub const RIDER_REACH: f32 = rider_reach(100.0);

/// The rider reach for a door model of this radius: the player's 10, the
/// door's radius, and 200 (0x4368b3).
#[must_use]
pub const fn rider_reach(radius: f32) -> f32 {
    10.0 + radius + 200.0
}

/// One vertical door's motion since its last received action.
#[derive(Clone, Debug, PartialEq)]
struct Leg {
    door: Door,
    motion: Vertical,
    /// Where the door stood when the leg began, along its travel.
    from: f32,
    /// Where the leg ends: 0 shut, the travel open.
    to: f32,
    /// When the leg began.
    at: Instant,
    /// When the client sends it back to its spawn end, if it does.
    returns: Option<(Instant, f32)>,
}

impl Leg {
    fn new(door: &Door, motion: Vertical, from: f32, to: f32, at: Instant) -> Self {
        let arrival = at + motion.time(from, to);
        let returns = timed_return(door, at, arrival)
            .map(|(when, open)| (when, if open { motion.travel } else { 0.0 }));
        Self {
            door: door.clone(),
            motion,
            from,
            to,
            at,
            returns,
        }
    }

    fn along(motion: &Vertical, from: f32, to: f32, since: Duration) -> f32 {
        let span = (to - from).abs();
        from + (motion.speed_toward(to) * since.as_secs_f32())
            .min(span)
            .copysign(to - from)
    }

    /// The door's offset along Z at `now`, with its own return.
    fn offset(&self, now: Instant) -> f32 {
        match self.returns {
            Some((back, home)) if now >= back => {
                let turned = Self::along(&self.motion, self.from, self.to, back - self.at);
                Self::along(&self.motion, turned, home, now - back)
            }
            _ => Self::along(
                &self.motion,
                self.from,
                self.to,
                now.saturating_duration_since(self.at),
            ),
        }
    }
}

/// Every vertical door's motion in the zone, stepped from the received actions
/// as the program steps it, with the client's own returns. The session and the
/// client keep one each from the same updates at the same received times, so
/// both see a door at the same height at the same time.
#[derive(Clone, Debug, Default)]
pub struct Lifts {
    legs: BTreeMap<u8, Leg>,
    /// The host's model radius for each door, with the model it holds for.
    radii: BTreeMap<u8, (String, f32)>,
}

impl Lifts {
    /// Follows a door's new definition, received at `now`.
    ///
    /// Every door spawns at its base, as the program places it, and the
    /// program's mover runs one spawned opening from there ([`endpoint`]):
    /// a door spawned in state 1 or 2 rises (or sinks) to its travel at its
    /// opening rate, and one spawned in state 3 or 0 stands at its base. A
    /// door spawned away from its rest goes back once its delay has passed
    /// since spawn, even mid-rise (`timed_return`; read R49-28b, recording
    /// checklist 6.423).
    pub fn spawned(&mut self, door: &Door, now: Instant) {
        let Some(motion) = Vertical::of(door) else {
            self.legs.remove(&door.id);
            return;
        };
        let (from, to) = match endpoint(door) {
            Some(true) => (0.0, motion.travel),
            _ => (0.0, 0.0),
        };
        self.legs
            .insert(door.id, Leg::new(door, motion, from, to, now));
    }

    /// Follows the server's action on a door, received at `now`, from where
    /// the door stands then.
    pub fn moved(&mut self, door: &Door, now: Instant) {
        let Some(leg) = self.legs.get(&door.id) else {
            return;
        };
        let from = leg.offset(now);
        let motion = leg.motion;
        let to = match door.active_endpoint() {
            Some(true) => motion.travel,
            Some(false) => 0.0,
            None => from,
        };
        self.legs
            .insert(door.id, Leg::new(door, motion, from, to, now));
    }

    /// Forgets every door's motion, as a zone's door removal does. The
    /// host's radii stay: a server can send the table again within a zone,
    /// and the host, whose list hasn't changed, doesn't send it again. Each
    /// radius holds only while its door draws the model it was given for.
    pub fn clear(&mut self) {
        self.legs.clear();
    }

    /// Takes the host's model radius for each door it draws, replacing the
    /// last list. A radius that is not a positive number is left out.
    pub fn set_radii(&mut self, radii: &[crate::doors::DoorRadius]) {
        self.radii = radii
            .iter()
            .filter(|door| door.radius.is_finite() && door.radius > 0.0)
            .map(|door| (door.door_id, (door.model.clone(), door.radius)))
            .collect();
    }

    /// How near a player must stand to door `door` for its motion to carry
    /// them: [`rider_reach`] of the host's radius while the door draws the
    /// model the radius was given for, else [`RIDER_REACH`].
    #[must_use]
    pub fn reach(&self, door: &Door) -> f32 {
        self.radii
            .get(&door.id)
            .filter(|(model, _)| *model == door.model)
            .map_or(RIDER_REACH, |(_, radius)| rider_reach(*radius))
    }

    /// How far door `id` stands above (or below) its placed height at `now`.
    #[must_use]
    pub fn offset(&self, id: u8, now: Instant) -> Option<f32> {
        self.legs.get(&id).map(|leg| leg.offset(now))
    }

    /// Door `id`'s definition as last followed, if it is a vertical door.
    #[must_use]
    pub fn door(&self, id: u8) -> Option<&Door> {
        self.legs.get(&id).map(|leg| &leg.door)
    }

    /// The door's motion, if it is a vertical door.
    #[must_use]
    pub fn motion(&self, id: u8) -> Option<Vertical> {
        self.legs.get(&id).map(|leg| leg.motion)
    }

    /// How far door `id` rose between `from` and `to`; none if it sank or
    /// stood still. Only the door's current leg is kept, so an instant
    /// before that leg began counts as its start: callers keep their
    /// intervals within one leg, as the guard's account does by closing
    /// when the door sinks.
    #[must_use]
    pub fn rise(&self, id: u8, from: Instant, to: Instant) -> f32 {
        self.legs
            .get(&id)
            .map_or(0.0, |leg| (leg.offset(to) - leg.offset(from)).max(0.0))
    }

    /// How far a player standing on door `id` in zone `zone_id` may rise
    /// between `from` and `to`: the door's own rise over the interval, when
    /// it is a floor the program stands players on and the player, at `at`
    /// at the instant `then`, is within [`Self::reach`] of where the door
    /// stood then. Anything else carries no one.
    #[must_use]
    pub fn carry(
        &self,
        id: u8,
        zone_id: u16,
        (at, then): (crate::world::Position, Instant),
        from: Instant,
        to: Instant,
    ) -> f32 {
        let Some(leg) = self.legs.get(&id) else {
            return 0.0;
        };
        if !Solidity::of(leg.door.open_type, zone_id).floor {
            return 0.0;
        }
        let door = leg.door.position;
        let distance = (door.x - at.x)
            .hypot(door.y - at.y)
            .hypot(door.z + leg.offset(then) - at.z);
        if distance > self.reach(&leg.door) {
            return 0.0;
        }
        self.rise(id, from, to)
    }

    /// Whether door `id` sinks at all between `from` and `to`.
    #[must_use]
    pub fn sinks(&self, id: u8, from: Instant, to: Instant) -> bool {
        self.legs
            .get(&id)
            .is_some_and(|leg| leg.offset(to) < leg.offset(from) - f32::EPSILON)
    }

    /// When door `id` goes back to its rest on its own, if it is a vertical
    /// door that does: from its arrival, or during a long travel, after its
    /// delay (0x48faa5).
    #[must_use]
    pub fn returns_at(&self, id: u8) -> Option<Instant> {
        self.legs.get(&id)?.returns.map(|(at, _)| at)
    }

    /// Whether door `id` moves at all between `from` and `to`.
    #[must_use]
    pub fn moves(&self, id: u8, from: Instant, to: Instant) -> bool {
        self.legs
            .get(&id)
            .is_some_and(|leg| (leg.offset(to) - leg.offset(from)).abs() > f32::EPSILON)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn door(open_type: u8) -> Door {
        Door {
            id: 3,
            model: "LIFT".into(),
            position: crate::world::Position::default(),
            incline: 0,
            size: 100,
            open_type,
            state_at_spawn: 0,
            invert_state: 0,
            parameter: 0,
            action: None,
        }
    }

    fn with_field(open_type: u8, field: i16) -> Door {
        Door {
            parameter: u32::from(field.cast_unsigned()) | 0x0005_0000,
            ..door(open_type)
        }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn a_spawned_door_takes_the_programs_state() {
        let spawned = |open_type: u8, state: u8| {
            let mut door = door(open_type);
            door.state_at_spawn = state;
            spawned_state(&door)
        };
        // 0x4903d4's types keep 1 for spawn states 1 and 2; others take 2.
        for open_type in [50, 51, 52, 54, 58, 153, 155] {
            assert_eq!(spawned(open_type, 1), Some(1), "type {open_type}");
            assert_eq!(spawned(open_type, 2), Some(1), "type {open_type}");
        }
        for open_type in [0, 53, 59, 65, 152, 156] {
            assert_eq!(spawned(open_type, 1), Some(2), "type {open_type}");
            assert_eq!(spawned(open_type, 2), Some(2), "type {open_type}");
        }
        assert_eq!(spawned(50, 3), Some(0));
        assert_eq!(spawned(50, 0), Some(0));
        assert_eq!(spawned(50, 7), Some(7));
        // After the server moves it, the state is the unread mover's.
        let mut moved = door(50);
        moved.action = Some(2);
        assert_eq!(spawned_state(&moved), None);
    }

    #[test]
    fn only_state_1_with_minus_2_ignores_clicks() {
        let marked = |open_type: u8, field: i16, state: u8| {
            let mut door = with_field(open_type, field);
            door.state_at_spawn = state;
            ignores_clicks(&door)
        };
        assert!(marked(50, -2, 1));
        assert!(marked(153, -2, 2));
        assert!(!marked(50, -2, 0));
        assert!(!marked(50, -1, 1));
        assert!(!marked(50, -3, 1));
        // A type the program puts in state 2.
        assert!(!marked(0, -2, 1));
    }

    #[test]
    fn the_field_is_the_params_low_half_signed_and_sets_the_travel() {
        assert_eq!(field(&with_field(59, -40)), -40);
        assert_eq!(Vertical::of(&with_field(59, -40)).unwrap().travel, 40.0);
        assert_eq!(Vertical::of(&with_field(59, -2)).unwrap().travel, 0.0);
        assert_eq!(Vertical::of(&with_field(75, -1)).unwrap().travel, 22.0);
        assert_eq!(Vertical::of(&with_field(75, 0)).unwrap().travel, 22.0);
        assert_eq!(Vertical::of(&with_field(76, 30)).unwrap().travel, 30.0);
        assert_eq!(Vertical::of(&with_field(80, 30)).unwrap().travel, -30.0);
    }

    #[test]
    fn rates_are_twenty_units_a_second_for_each_unit_of_rate() {
        let lift = Vertical::of(&with_field(59, 100)).unwrap();
        assert!(near(lift.open_speed, 9.0) && near(lift.close_speed, 9.0));
        let step = Vertical::of(&door(60)).unwrap();
        assert_eq!(step.travel, 8.0);
        assert!(near(step.open_speed, 9.0) && near(step.close_speed, 5.0));
        let sink = Vertical::of(&door(146)).unwrap();
        assert_eq!(sink.travel, -10.0);
        assert_eq!((sink.open_speed, sink.close_speed), (20.0, 20.0));
        assert_eq!(Vertical::of(&door(141)).unwrap().travel, 128.0);
        for none in [0, 5, 15, 58, 82, 119] {
            assert_eq!(Vertical::of(&door(none)), None, "type {none}");
        }
    }

    #[test]
    fn slides_follow_the_heading_axis_and_seventy_nine_and_eighty_one_reverse() {
        let mut slide = door(15);
        for (heading, direction) in [
            (128.0, [0.0, 1.0, 0.0]),
            (384.0, [0.0, -1.0, 0.0]),
            (256.0, [1.0, 0.0, 0.0]),
            (0.0, [-1.0, 0.0, 0.0]),
            (100.0, [-1.0, 0.0, 0.0]),
        ] {
            slide.position.heading = heading;
            let found = Slide::of(&slide).unwrap();
            assert_eq!((found.distance, found.direction), (10.0, direction));
            assert!(near(found.speed, 7.0));
        }
        let mut reversed = with_field(79, 0);
        reversed.position.heading = 128.0;
        let found = Slide::of(&reversed).unwrap();
        assert_eq!((found.distance, found.direction), (22.0, [0.0, -1.0, 0.0]));
        assert_eq!(Slide::of(&with_field(78, 12)).unwrap().distance, 12.0);
        assert_eq!(Slide::of(&door(59)), None);
    }

    #[test]
    fn the_floor_sweep_takes_only_the_programs_types_and_every_door_is_a_wall() {
        for floor in [30, 45, 50, 51, 54, 59, 153, 155] {
            assert!(Solidity::of(floor, 1).floor, "type {floor}");
        }
        for not in [0, 5, 8, 15, 29, 46, 52, 53, 60, 65, 120, 156] {
            assert!(!Solidity::of(not, 1).floor, "type {not}");
        }
        assert!(Solidity::of(27, 154).floor && !Solidity::of(27, 1).floor);
        assert!(Solidity::of(26, 205).floor && Solidity::of(26, 212).floor);
        assert!(Solidity::of(8, 208).floor && !Solidity::of(8, 1).floor);
        // 0x42cdb1 collides with every door actor: a shut portcullis (65)
        // stops the player like a hinged door, in any zone.
        for open_type in 0..=u8::MAX {
            for zone in [1, 59, 154] {
                assert!(Solidity::of(open_type, zone).wall, "type {open_type}");
            }
        }
        assert_eq!(Solidity::default(), Solidity::BOTH);
    }

    #[test]
    fn the_programs_return_times_by_type_and_field() {
        let after = |open_type, field| returns_after(&with_field(open_type, field));
        assert_eq!(after(0, 0), Some(Duration::from_secs(10)));
        assert_eq!(after(5, 0), Some(Duration::from_secs(10)));
        assert_eq!(after(1, 0), Some(Duration::from_secs(20)));
        assert_eq!(after(59, 170), Some(Duration::from_secs(20)));
        assert_eq!(after(59, 171), None);
        assert_eq!(after(5, -2), None);
        assert_eq!(after(5, -3), None);
        assert_eq!(after(5, -1), Some(Duration::from_secs(10)));
        assert_eq!(after(40, 1500), Some(Duration::from_millis(1500)));
        assert_eq!(after(45, 32001), Some(Duration::from_mins(30)));
        assert_eq!(after(50, 0), Some(Duration::from_secs(20)));
        assert_eq!(after(116, 0), Some(Duration::from_secs(6)));
        assert_eq!(after(58, 0), Some(Duration::from_secs(2)));
        assert_eq!(after(106, 0), None);
        assert_eq!(after(115, 0), Some(Duration::from_millis(1)));
        assert_eq!(after(130, 0), Some(Duration::from_millis(800)));
        assert_eq!(after(150, 0), Some(Duration::from_secs(4)));
        assert_eq!(after(152, 0), Some(Duration::from_secs(3)));
    }

    #[test]
    fn only_forty_and_forty_five_return_by_a_click_naming_no_player() {
        assert!(returns_by_click(&door(40)) && returns_by_click(&door(45)));
        assert!(!returns_by_click(&door(50)) && !returns_by_click(&door(59)));
        let click = return_click(9);
        assert_eq!(click.opcode, 0x043b);
        assert_eq!(
            click.body,
            [9, 0, 0, 0, 0xfe, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn a_door_away_from_its_rest_goes_back_and_an_inverted_one_by_the_open_action() {
        let mut plain = door(5);
        assert!(!away(&plain));
        plain.state_at_spawn = 1;
        assert!(away(&plain));
        plain.action = Some(3);
        assert!(!away(&plain));
        assert_eq!(return_action(&plain), 3);
        let mut inverted = door(5);
        inverted.invert_state = 1;
        assert!(away(&inverted));
        inverted.state_at_spawn = 1;
        assert!(!away(&inverted));
        inverted.action = Some(3);
        assert!(away(&inverted));
        assert_eq!(return_action(&inverted), 2);
    }

    #[test]
    fn a_lift_says_when_it_sinks_and_when_it_goes_back_from_where_it_stood() {
        let start = Instant::now();
        let mut lifts = Lifts::default();
        // Spawned open (state 1), 90 up: back down 20 s from its spawn.
        let mut open = Door {
            parameter: 90,
            state_at_spawn: 1,
            ..door(59)
        };
        lifts.spawned(&open, start);
        assert_eq!(lifts.returns_at(3), Some(start + Duration::from_secs(20)));
        assert!(!lifts.sinks(3, start, start + Duration::from_secs(19)));
        assert!(lifts.sinks(
            3,
            start + Duration::from_secs(19),
            start + Duration::from_secs(21)
        ));
        // Opened again mid-return from 45 up: 5 s to the top, then 20 s.
        open.action = Some(2);
        let at = start + Duration::from_secs(25);
        lifts.moved(&open, at);
        assert!((lifts.offset(3, at).unwrap() - 45.0).abs() < 0.01);
        assert_eq!(lifts.returns_at(3), Some(at + Duration::from_secs(25)));
        assert_eq!(
            returns_at(&open, at),
            Some(at + Duration::from_secs(30)),
            "the table's full-travel estimate differs"
        );
    }

    #[test]
    fn a_door_returns_after_arrival_or_during_a_long_travel() {
        let at = Instant::now();
        let mut hinge = door(5);
        hinge.action = Some(2);
        assert_eq!(returns_at(&hinge, at), Some(at + Duration::from_secs(10)));
        hinge.action = Some(3);
        assert_eq!(returns_at(&hinge, at), None);
        // 90 units at 9 a second is 10 s, so 20 s after arrival.
        let mut lift = with_field(59, 90);
        lift.action = Some(2);
        let back = returns_at(&lift, at).unwrap();
        assert!(
            back > at + Duration::from_millis(29_990) && back < at + Duration::from_millis(30_010)
        );
        // A 0.8 s delay on a slower door fires before it arrives.
        let mut quick = with_field(130, 0);
        quick.action = Some(2);
        assert_eq!(
            returns_at(&quick, at),
            Some(at + Duration::from_millis(800))
        );
    }

    #[test]
    fn a_lift_rises_on_the_servers_open_and_its_own_return_brings_it_down() {
        let start = Instant::now();
        let mut lifts = Lifts::default();
        let mut lift = with_field(59, 45);
        lifts.spawned(&lift, start);
        assert_eq!(lifts.offset(3, start), Some(0.0));
        lift.action = Some(2);
        lifts.moved(&lift, start);
        let half = start + Duration::from_millis(2500);
        assert!(near(lifts.offset(3, half).unwrap(), 22.5));
        assert!(near(lifts.rise(3, start, half), 22.5));
        let top = start + Duration::from_secs(5);
        assert!(near(lifts.offset(3, top).unwrap(), 45.0));
        assert!(!lifts.moves(3, top, top + Duration::from_secs(19)));
        // 20 s after arriving it goes back down at its closing speed.
        let back = top + Duration::from_secs(20);
        assert!(near(
            lifts.offset(3, back + Duration::from_secs(1)).unwrap(),
            36.0
        ));
        assert_eq!(lifts.rise(3, back, back + Duration::from_secs(1)), 0.0);
        assert!(near(
            lifts.offset(3, back + Duration::from_secs(9)).unwrap(),
            0.0
        ));
    }

    #[test]
    fn the_servers_close_during_the_own_return_neither_jumps_nor_doubles_the_step() {
        let start = Instant::now();
        let mut lifts = Lifts::default();
        let mut lift = with_field(59, 45);
        lifts.spawned(&lift, start);
        lift.action = Some(2);
        lifts.moved(&lift, start);
        let returning = start + Duration::from_secs(26);
        let before = lifts.offset(3, returning).unwrap();
        assert!(near(before, 36.0));
        lift.action = Some(3);
        lifts.moved(&lift, returning);
        assert!(near(lifts.offset(3, returning).unwrap(), before));
        let later = returning + Duration::from_secs(1);
        assert!(near(lifts.offset(3, later).unwrap(), 27.0));
    }

    #[test]
    fn a_close_part_way_up_runs_back_from_there_at_the_closing_speed() {
        let start = Instant::now();
        let mut lifts = Lifts::default();
        let mut step = door(65);
        lifts.spawned(&step, start);
        step.action = Some(2);
        lifts.moved(&step, start);
        let turn = start + Duration::from_millis(500);
        step.action = Some(3);
        lifts.moved(&step, turn);
        let later = turn + Duration::from_millis(400);
        assert!(near(lifts.offset(3, later).unwrap(), 2.5));
        assert_eq!(lifts.rise(3, turn, later), 0.0);
    }

    #[test]
    fn a_lift_carries_only_a_rider_within_reach_of_a_floor_it_stands_players_on() {
        let start = Instant::now();
        let mut lifts = Lifts::default();
        let mut lift = with_field(59, 45);
        lift.position.z = 5.0;
        lifts.spawned(&lift, start);
        lift.action = Some(2);
        lifts.moved(&lift, start);
        let from = start + Duration::from_secs(1);
        let to = from + Duration::from_millis(500);
        let at = |x: f32, z: f32| crate::world::Position {
            x,
            z,
            ..crate::world::Position::default()
        };
        // At 1 s the lift stands 9 up, at z 14: a rider 310 away in 3D rides.
        let edge = (RIDER_REACH * RIDER_REACH - 9.0 * 9.0).sqrt();
        assert!(near(
            lifts.carry(3, 1, (at(0.0, 14.0), from), from, to),
            4.5
        ));
        assert!(near(
            lifts.carry(3, 1, (at(edge - 0.01, 5.0), from), from, to),
            4.5
        ));
        assert_eq!(
            lifts.carry(3, 1, (at(edge + 0.01, 5.0), from), from, to),
            0.0
        );
        assert_eq!(
            lifts.carry(3, 1, (at(RIDER_REACH, 14.0), from), from, to),
            0.0 + 4.5 * 1.0
        );
        assert_eq!(
            lifts.carry(3, 1, (at(RIDER_REACH + 0.01, 14.0), from), from, to),
            0.0
        );
        assert_eq!(lifts.carry(9, 1, (at(0.0, 14.0), from), from, to), 0.0);
        // Once the lift stands still at the top it carries no one.
        let top = start + Duration::from_secs(6);
        assert_eq!(
            lifts.carry(
                3,
                1,
                (at(0.0, 50.0), top),
                top,
                top + Duration::from_secs(1)
            ),
            0.0
        );
        // A vertical door the floor sweep skips carries no one.
        let mut step = door(65);
        step.id = 4;
        lifts.spawned(&step, start);
        step.action = Some(2);
        lifts.moved(&step, start);
        assert_eq!(lifts.carry(4, 1, (at(0.0, 0.0), start), start, from), 0.0);
    }

    #[test]
    fn a_door_takes_the_hosts_radius_while_it_draws_that_model() {
        let radius = |door_id, model: &str, radius| crate::doors::DoorRadius {
            door_id,
            model: model.into(),
            radius,
        };
        let mut lifts = Lifts::default();
        let lift = door(59);
        assert_eq!(lifts.reach(&lift), RIDER_REACH);
        assert!(near(RIDER_REACH, 310.0));
        lifts.set_radii(&[radius(3, "LIFT", 40.0), radius(4, "LIFT", 55.0)]);
        assert!(near(lifts.reach(&lift), 250.0));
        // Another model under the same ID keeps the program's no-model reach.
        let other = Door {
            model: "GATE".into(),
            ..door(59)
        };
        assert_eq!(lifts.reach(&other), RIDER_REACH);
        // Each list replaces the last, and a radius that is not a positive
        // number is left out.
        for bad in [0.0, -5.0, f32::NAN, f32::INFINITY] {
            lifts.set_radii(&[radius(3, "LIFT", bad)]);
            assert_eq!(lifts.reach(&lift), RIDER_REACH, "{bad}");
        }
        // A door removal forgets the motion but not the radii, which the
        // host sends once for a list that doesn't change.
        lifts.set_radii(&[radius(3, "LIFT", 40.0)]);
        lifts.clear();
        assert!(near(lifts.reach(&lift), 250.0));
    }

    #[test]
    fn a_lift_carries_a_rider_within_its_own_reach() {
        let start = Instant::now();
        let mut lifts = Lifts::default();
        let mut lift = with_field(59, 45);
        lifts.spawned(&lift, start);
        lifts.set_radii(&[crate::doors::DoorRadius {
            door_id: 3,
            model: "LIFT".into(),
            radius: 40.0,
        }]);
        lift.action = Some(2);
        lifts.moved(&lift, start);
        let from = start + Duration::from_secs(1);
        let to = from + Duration::from_millis(500);
        // At 1 s the lift stands 9 up; a rider at that height is judged by
        // the 10 + 40 + 200 = 250 reach, not the no-model 310.
        let at = |x: f32| crate::world::Position {
            x,
            z: 9.0,
            ..crate::world::Position::default()
        };
        assert!(near(lifts.carry(3, 1, (at(249.99), from), from, to), 4.5));
        assert_eq!(lifts.carry(3, 1, (at(250.01), from), from, to), 0.0);
        assert_eq!(lifts.carry(3, 1, (at(300.0), from), from, to), 0.0);
    }

    #[test]
    fn a_tall_lift_carries_its_rider_all_the_way_up_judged_where_it_stands_now() {
        let start = Instant::now();
        let mut lifts = Lifts::default();
        // Field 400: 400 up at 9 a second, past the rider reach.
        let mut lift = with_field(59, 400);
        lifts.spawned(&lift, start);
        lift.action = Some(2);
        lifts.moved(&lift, start);
        let mut z = 0.0;
        for step in 1..=450_u64 {
            let then = start + Duration::from_millis((step - 1) * 100);
            let now = start + Duration::from_millis(step * 100);
            let rider = crate::world::Position {
                z,
                ..crate::world::Position::default()
            };
            // The account runs from the start; the reach from the last sample.
            let risen = lifts.carry(3, 1, (rider, then), start, now);
            assert!(risen >= lifts.rise(3, start, now) - 1e-3, "stranded at {z}");
            z = lifts.offset(3, now).unwrap();
        }
        assert!((z - 400.0).abs() < 1e-3);
        // Judged at the start instead, the rider is out of reach by now.
        let top = crate::world::Position {
            z: 400.0,
            ..crate::world::Position::default()
        };
        assert_eq!(
            lifts.carry(3, 1, (top, start), start, start + Duration::from_secs(45)),
            0.0
        );
    }

    fn spawned(open_type: u8, state: u8, rest: u8) -> Door {
        Door {
            state_at_spawn: state,
            invert_state: rest,
            ..door(open_type)
        }
    }

    #[test]
    fn a_hinged_door_spawned_opening_swings_open_and_closes_its_delay_after_spawn() {
        // Read R49-28b, "State 2, hinged": states 1 and 2 both open, and the
        // door closes its delay after spawn when its rest is 0.
        let at = Instant::now();
        for state in [1, 2] {
            for (open_type, delay) in [(0, 10), (5, 10), (1, 20), (8, 20)] {
                let hinge = spawned(open_type, state, 0);
                assert_eq!(spawned_state(&hinge), Some(2));
                assert_eq!(endpoint(&hinge), Some(true));
                assert_eq!(
                    returns_at(&hinge, at),
                    Some(at + Duration::from_secs(delay)),
                    "{open_type} {state}"
                );
                assert_eq!(return_action(&hinge), CLOSE);
            }
        }
        // At rest already: shut (0, or 3 which spawns as 0) with rest 0.
        for state in [0, 3] {
            let hinge = spawned(0, state, 0);
            assert_eq!(endpoint(&hinge), Some(false));
            assert_eq!(returns_at(&hinge, at), None, "{state}");
        }
        // A door whose rest is open goes back opening (state 2 for rest 1).
        let inverted = spawned(0, 0, 1);
        assert_eq!(
            returns_at(&inverted, at),
            Some(at + Duration::from_secs(10))
        );
        assert_eq!(return_action(&inverted), OPEN);
        // Not built, so no return: a rest above 1, and state 4's 60 s.
        assert_eq!(returns_at(&spawned(0, 2, 2), at), None);
        let four = spawned(0, 4, 0);
        assert_eq!(endpoint(&four), None);
        assert_eq!(returns_at(&four, at), None);
    }

    #[test]
    fn a_lift_spawned_opening_goes_back_its_delay_after_spawn_even_mid_rise() {
        let start = Instant::now();
        let mut lifts = Lifts::default();
        // Type 60 rises 8 in about 0.89 s; its 20 s run from spawn, not from
        // its arrival.
        lifts.spawned(&spawned(60, 2, 0), start);
        assert_eq!(lifts.returns_at(3), Some(start + Duration::from_secs(20)));
        assert!(near(
            lifts.offset(3, start + Duration::from_secs(19)).unwrap(),
            8.0
        ));
        let closing = start + Duration::from_secs(21);
        assert!(near(lifts.offset(3, closing).unwrap(), 8.0 - 0.25 * 20.0));
        // Type 75 rising 200 at 9 a second is still rising at 20 s: it turns
        // there, at 180, and goes down at 0.25 x 20.
        let mut tall = with_field(75, 200);
        tall.state_at_spawn = 2;
        lifts.spawned(&tall, start);
        assert_eq!(lifts.returns_at(3), Some(start + Duration::from_secs(20)));
        let turned = start + Duration::from_secs(20);
        assert!(near(lifts.offset(3, turned).unwrap(), 180.0));
        assert!(near(
            lifts.offset(3, turned + Duration::from_secs(1)).unwrap(),
            175.0
        ));
        // Never: +0x66 of -2 or -3, and type 59 with +0x66 above 170.
        for never in [with_field(60, -2), with_field(60, -3), with_field(59, 171)] {
            let never = Door {
                state_at_spawn: 2,
                ..never
            };
            lifts.spawned(&never, start);
            assert_eq!(lifts.returns_at(3), None, "{}", never.open_type);
        }
    }

    #[test]
    fn the_movers_skipped_types_stand_at_their_base_and_never_go_back() {
        let at = Instant::now();
        for open_type in [50, 51, 52, 54, 55, 56, 57, 58, 153, 154, 155] {
            for state in [1, 2] {
                let skipped = spawned(open_type, state, 0);
                assert!(mover_skips(&skipped));
                assert_eq!(spawned_state(&skipped), Some(1));
                assert_eq!(endpoint(&skipped), Some(false));
                assert_eq!(returns_at(&skipped, at), None, "{open_type}");
            }
            // Nor after an action: the mover never times them.
            let mut moved = with_field(open_type, 500);
            moved.action = Some(2);
            assert_eq!(returns_at(&moved, at), None, "{open_type}");
        }
        assert!(!mover_skips(&door(53)));
        assert_eq!(spawned_state(&spawned(53, 1, 0)), Some(2));
    }

    #[test]
    fn a_door_of_types_110_to_152_closes_as_soon_as_it_stands_open() {
        let start = Instant::now();
        let mut lifts = Lifts::default();
        // Type 140 sinks 10 at 1 x 20 a second: open at 0.5 s, and closing
        // at once, at 0.25 x 20, though its delay is 2 s.
        lifts.spawned(&spawned(140, 2, 0), start);
        let open = start + Duration::from_millis(500);
        assert_eq!(lifts.returns_at(3), Some(open));
        assert!(near(lifts.offset(3, open).unwrap(), -10.0));
        assert!(near(
            lifts.offset(3, open + Duration::from_millis(500)).unwrap(),
            -7.5
        ));
        // After the server's open too.
        let mut moved = spawned(140, 0, 0);
        lifts.spawned(&moved, start);
        moved.action = Some(2);
        lifts.moved(&moved, start);
        assert_eq!(lifts.returns_at(3), Some(open));
        // A door that swings has no stepped arrival here and keeps its timer.
        let swing = spawned(130, 2, 0);
        assert_eq!(
            returns_at(&swing, start),
            Some(start + Duration::from_millis(800))
        );
        assert!(!closes_once_open(&door(115)));
    }

    #[test]
    fn every_spawned_lift_starts_at_its_base_and_one_opening_rises() {
        let now = Instant::now();
        let mut lifts = Lifts::default();
        let mut lift = with_field(59, 18);
        // States 1 and 2 are both opening (2) for a type the mover steps:
        // placed at the base, it rises at 0.45 x 20 a second (read R49-28b).
        for state in [1, 2] {
            lift.state_at_spawn = state;
            lifts.spawned(&lift, now);
            assert_eq!(lifts.offset(3, now), Some(0.0), "{state}");
            assert!(near(
                lifts.offset(3, now + Duration::from_secs(1)).unwrap(),
                9.0
            ));
            assert!(near(
                lifts.offset(3, now + Duration::from_secs(3)).unwrap(),
                18.0
            ));
        }
        lift.state_at_spawn = 3;
        lifts.spawned(&lift, now);
        assert_eq!(lifts.offset(3, now + Duration::from_secs(1)), Some(0.0));
        // An action on a door that isn't vertical, or unknown, changes nothing.
        lifts.spawned(&door(5), now);
        assert_eq!(lifts.offset(3, now), None);
        lifts.moved(&door(5), now);
        assert_eq!(lifts.offset(3, now), None);
        lifts.spawned(&lift, now);
        lifts.clear();
        assert_eq!(lifts.offset(3, now), None);
    }
}
