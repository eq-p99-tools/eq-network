//! Titanium's own door mechanisms: how far and how fast each open type moves,
//! which ones a player stands on or walks into, and when the client sends a
//! door back on its own.
//!
//! Read from `eqgame.exe` (Titanium): the per-frame door step 0x4368b3, the
//! floor-sweep filter 0x42d05f with its door test 0x490370, the wall contact
//! 0x42ce5d and the return timer 0x48faa5 (read R1, 11-09). The program moves
//! a door only on the server's 0x700d action; a click sends 0x043b and changes
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
    /// The floor sweep takes a door only when 0x490370 holds: types 30-45,
    /// 50-51, 54-59 and 153-155, and three zones' own types. Wall contact
    /// (0x42ce5d) skips 60-77.
    #[must_use]
    pub const fn of(open_type: u8, zone_id: u16) -> Self {
        let floor = matches!(open_type, 30..=45 | 50 | 51 | 54..=59 | 153..=155)
            || matches!((open_type, zone_id), (27, 154) | (26, 205 | 212) | (8, 208));
        Self {
            floor,
            wall: !matches!(open_type, 60..=77),
        }
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

/// When a door moved by an action at `at` goes back on its own, if it does:
/// the timer restarts with each state change, so it runs from the action and
/// again from arrival, and a delay shorter than the travel fires before the
/// door arrives (read R1b item 4).
#[must_use]
pub fn returns_at(door: &Door, at: Instant) -> Option<Instant> {
    if !away(door) {
        return None;
    }
    let delay = returns_after(door)?;
    let (from, to) = Vertical::of(door).map_or((0.0, 0.0), |motion| (0.0, motion.travel));
    let arrival = at + travel_time(door, from, to);
    Some(if at + delay < arrival {
        at + delay
    } else {
        arrival + delay
    })
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
pub const RIDER_REACH: f32 = 10.0 + 100.0 + 200.0;

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
        let home = if return_action(door) == OPEN {
            motion.travel
        } else {
            0.0
        };
        let returns = away(door)
            .then(|| returns_after(door))
            .flatten()
            .map(|delay| {
                (
                    if at + delay < arrival {
                        at + delay
                    } else {
                        arrival + delay
                    },
                    home,
                )
            });
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
}

impl Lifts {
    /// Follows a door's new definition, received at `now`.
    ///
    /// A door spawned open (state 1) stands at its full travel. One spawned
    /// opening (2) starts up from shut when it arrives; one spawned closing
    /// (3) or shut stands shut. A door spawned away from its rest goes back
    /// on its own timer from its arrival (recording checklist 6.423).
    pub fn spawned(&mut self, door: &Door, now: Instant) {
        let Some(motion) = Vertical::of(door) else {
            self.legs.remove(&door.id);
            return;
        };
        let (from, to) = match door.state_at_spawn {
            1 => (motion.travel, motion.travel),
            2 => (0.0, motion.travel),
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

    /// Forgets every door, as a zone's door removal does.
    pub fn clear(&mut self) {
        self.legs.clear();
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
    /// stood still.
    #[must_use]
    pub fn rise(&self, id: u8, from: Instant, to: Instant) -> f32 {
        self.legs
            .get(&id)
            .map_or(0.0, |leg| (leg.offset(to) - leg.offset(from)).max(0.0))
    }

    /// How far a player at `at` may rise between `from` and `to` standing on
    /// door `id` in zone `zone_id`: the door's own rise over the interval,
    /// when it is a floor the program stands players on and `at` is within
    /// [`RIDER_REACH`] of where the door stands at `from`. Anything else
    /// carries no one.
    #[must_use]
    pub fn carry(
        &self,
        id: u8,
        zone_id: u16,
        at: crate::world::Position,
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
            .hypot(door.z + leg.offset(from) - at.z);
        if distance > RIDER_REACH {
            return 0.0;
        }
        self.rise(id, from, to)
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
    fn the_floor_sweep_takes_only_the_programs_types_and_walls_skip_sixty_to_seventy_seven() {
        for floor in [30, 45, 50, 51, 54, 59, 153, 155] {
            assert!(Solidity::of(floor, 1).floor, "type {floor}");
        }
        for not in [0, 5, 8, 15, 29, 46, 52, 53, 60, 120, 156] {
            assert!(!Solidity::of(not, 1).floor, "type {not}");
        }
        assert!(Solidity::of(27, 154).floor && !Solidity::of(27, 1).floor);
        assert!(Solidity::of(26, 205).floor && Solidity::of(26, 212).floor);
        assert!(Solidity::of(8, 208).floor && !Solidity::of(8, 1).floor);
        for open_type in 60..=77 {
            assert!(!Solidity::of(open_type, 1).wall);
        }
        assert!(Solidity::of(59, 1).wall && Solidity::of(78, 1).wall);
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
        assert!(near(lifts.carry(3, 1, at(0.0, 14.0), from, to), 4.5));
        assert!(near(lifts.carry(3, 1, at(edge - 0.01, 5.0), from, to), 4.5));
        assert_eq!(lifts.carry(3, 1, at(edge + 0.01, 5.0), from, to), 0.0);
        assert_eq!(
            lifts.carry(3, 1, at(RIDER_REACH, 14.0), from, to),
            0.0 + 4.5 * 1.0
        );
        assert_eq!(
            lifts.carry(3, 1, at(RIDER_REACH + 0.01, 14.0), from, to),
            0.0
        );
        assert_eq!(lifts.carry(9, 1, at(0.0, 14.0), from, to), 0.0);
        // Once the lift stands still at the top it carries no one.
        let top = start + Duration::from_secs(6);
        assert_eq!(
            lifts.carry(3, 1, at(0.0, 50.0), top, top + Duration::from_secs(1)),
            0.0
        );
        // A vertical door the floor sweep skips carries no one.
        let mut step = door(65);
        step.id = 4;
        lifts.spawned(&step, start);
        step.action = Some(2);
        lifts.moved(&step, start);
        assert_eq!(lifts.carry(4, 1, at(0.0, 0.0), start, from), 0.0);
    }

    #[test]
    fn spawn_states_pose_open_shut_or_rising_from_shut() {
        let now = Instant::now();
        let mut lifts = Lifts::default();
        let mut lift = with_field(59, 18);
        lift.state_at_spawn = 1;
        lifts.spawned(&lift, now);
        assert_eq!(lifts.offset(3, now), Some(18.0));
        lift.state_at_spawn = 2;
        lifts.spawned(&lift, now);
        assert_eq!(lifts.offset(3, now), Some(0.0));
        assert!(near(
            lifts.offset(3, now + Duration::from_secs(1)).unwrap(),
            9.0
        ));
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
