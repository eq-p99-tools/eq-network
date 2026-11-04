//! Ordinary hinged-door motion, separate from any successful click or server grant.
use super::super::Door;
use std::time::{Duration, Instant};

/// Whether the definition and latest action belong to ordinary use supported here.
/// Specialized mechanisms, parameters and unknown states remain visible but unused.
#[must_use]
pub fn ordinary(door: &Door) -> bool {
    door.id != super::super::STATIC_ID
        && door.open_type <= 8
        && door.incline == 0
        && door.size > 0
        && i16::try_from(door.size).is_ok()
        && door.parameter == 0
        && door.invert_state <= 1
        && door.state_at_spawn <= 1
        && matches!(door.action, None | Some(2 | 3))
}

/// An ordinary hinge's local motion and return to its server-defined resting pose.
/// Definitions/actions establish targets and their local return clocks;
/// sending a click never changes them.
/// This does not establish key eligibility, request acceptance or item ownership.
#[derive(Clone, Debug)]
pub struct Hinge {
    position: f32,
    target: f32,
    rest: f32,
    last: Instant,
    return_at: Option<Instant>,
    return_delay: Duration,
}

impl Hinge {
    /// Starts an ordinary definition at its placed heading, as the program does.
    /// An initially open definition swings toward its endpoint; an unsupported
    /// definition or action creates no usable motion state.
    #[must_use]
    pub fn new(door: &Door, now: Instant) -> Option<Self> {
        if !ordinary(door) {
            return None;
        }
        let target = if door.active_endpoint()? { 128.0 } else { 0.0 };
        let mut hinge = Self {
            position: if door.action == Some(3) { 128.0 } else { 0.0 },
            target,
            rest: if door.invert_state == 0 { 0.0 } else { 128.0 },
            last: now,
            return_at: None,
            return_delay: Duration::from_secs(if matches!(door.open_type, 0 | 5) {
                10
            } else {
                20
            }),
        };
        if hinge.settled() {
            hinge.schedule_return(now);
        }
        Some(hinge)
    }

    /// Takes a received open/close action, preserving a partially moved hinge.
    /// Unknown actions return false without inventing an endpoint.
    pub fn action(&mut self, action: u8, now: Instant) -> bool {
        let target = match action {
            2 => 128.0,
            3 => 0.0,
            _ => return false,
        };
        self.tick(now);
        self.target = target;
        self.return_at = None;
        if self.settled() {
            self.schedule_return(now);
        }
        true
    }

    /// Advances local animation and its return clock; no packet or acknowledgment
    /// is produced. A clock sample older than the last one changes nothing.
    pub fn tick(&mut self, now: Instant) {
        if now < self.last {
            return;
        }
        let elapsed = now.duration_since(self.last);
        let milliseconds = u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX);
        // Keep sub-millisecond time so frequent clock samples cannot stop motion.
        self.last += Duration::from_millis(u64::from(milliseconds));
        if self.return_at.is_some_and(|at| now > at) {
            self.return_at = None;
            self.target = self.rest;
        }
        if self.settled() {
            return;
        }
        // The program converts its integer millisecond delta to binary32,
        // multiplies by 0.02, then advances these types by 8 heading units.
        #[allow(clippy::cast_precision_loss)]
        let step = (milliseconds as f32 * 0.02) * 8.0;
        let remaining = self.target - self.position;
        if step >= remaining.abs() {
            self.position = self.target;
            self.schedule_return(now);
        } else {
            self.position += step.copysign(remaining);
        }
    }

    /// Whether the hinge is at an endpoint, so its ordinary click guard permits use.
    #[must_use]
    #[allow(clippy::float_cmp)] // tick clamps to the exact target endpoint.
    pub fn settled(&self) -> bool {
        self.position == self.target
    }

    /// Local pose from 0 at the placed heading to 1 at a quarter turn.
    #[must_use]
    pub fn fraction(&self) -> f32 {
        self.position / 128.0
    }

    /// The current animation target, including a local return to the resting pose.
    /// This is a presentation target, not a new server observation.
    #[must_use]
    pub fn endpoint(&self) -> bool {
        self.target != 0.0
    }

    /// Whether the local return clock has passed its delay.
    #[must_use]
    pub fn return_due(&self, now: Instant) -> bool {
        self.return_at.is_some_and(|at| now > at)
    }

    #[allow(clippy::float_cmp)] // Both targets are the exact endpoints 0 and 128.
    fn schedule_return(&mut self, now: Instant) {
        self.return_at = (self.target != self.rest).then(|| now + self.return_delay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Position;

    fn door(open_type: u8) -> Door {
        Door {
            id: 7,
            model: "DOOR".into(),
            position: Position::default(),
            incline: 0,
            size: 100,
            open_type,
            state_at_spawn: 0,
            invert_state: 0,
            parameter: 0,
            action: None,
        }
    }

    #[test]
    fn specialized_and_unknown_definitions_remain_visible_without_ordinary_use() {
        let now = Instant::now();
        for open_type in [9, 40, 53, 54, 55, 56, 57, 58, 105, 115, 120, 255] {
            assert!(!ordinary(&door(open_type)));
        }
        let change: [fn(&mut Door); 10] = [
            |door| door.id = 255,
            |door| door.incline = 1,
            |door| door.size = 0,
            |door| door.size = 32768,
            |door| door.parameter = 1,
            |door| door.parameter = 0xfffe,
            |door| door.parameter = 0xffff,
            |door| door.invert_state = 2,
            |door| door.state_at_spawn = 2,
            |door| door.action = Some(251),
        ];
        for change in change {
            let mut unsupported = door(0);
            change(&mut unsupported);
            assert!(Hinge::new(&unsupported, now).is_none());
        }
        for open_type in 0..=8 {
            assert!(ordinary(&door(open_type)));
        }
    }

    #[test]
    fn received_actions_move_a_hinge_before_its_type_specific_local_return() {
        let start = Instant::now();
        for (open_type, delay) in [(0, 10), (5, 10), (1, 20), (8, 20)] {
            let mut hinge = Hinge::new(&door(open_type), start).unwrap();
            assert!(hinge.settled());
            assert!(hinge.action(2, start));
            assert!(!hinge.settled());
            hinge.tick(start + Duration::from_millis(400));
            assert_eq!(hinge.fraction(), 0.5);
            hinge.tick(start + Duration::from_millis(800));
            assert!(hinge.settled());
            assert_eq!(hinge.fraction(), 1.0);
            let due = start + Duration::from_millis(800) + Duration::from_secs(delay);
            assert!(!hinge.return_due(due));
            hinge.tick(due);
            assert!(hinge.settled());
            assert!(hinge.return_due(due + Duration::from_millis(1)));
            hinge.tick(due + Duration::from_millis(1));
            assert!(!hinge.settled());
            assert!(!hinge.endpoint());
            hinge.tick(due + Duration::from_millis(801));
            assert!(hinge.settled());
            assert_eq!(hinge.fraction(), 0.0);
            assert!(!hinge.return_due(due + Duration::from_secs(60)));
        }
    }

    #[test]
    fn inverted_resting_pose_and_partial_reversal_do_not_reinterpret_server_actions() {
        let start = Instant::now();
        let mut inverted = door(1);
        inverted.invert_state = 1;
        inverted.state_at_spawn = 1;
        let mut hinge = Hinge::new(&inverted, start).unwrap();
        hinge.tick(start + Duration::from_millis(800));
        assert_eq!(hinge.fraction(), 1.0);
        assert!(!hinge.return_due(start + Duration::from_secs(60)));
        assert!(hinge.action(3, start + Duration::from_millis(800)));
        hinge.tick(start + Duration::from_millis(1200));
        assert_eq!(hinge.fraction(), 0.5);
        assert!(hinge.action(2, start + Duration::from_millis(1200)));
        hinge.tick(start + Duration::from_millis(1600));
        assert_eq!(hinge.fraction(), 1.0);
        assert!(hinge.settled());
        assert!(!hinge.return_due(start + Duration::from_secs(60)));
        assert!(!hinge.action(251, start + Duration::from_secs(60)));
        assert_eq!(hinge.fraction(), 1.0);
    }

    #[test]
    fn submillisecond_samples_and_old_clock_samples_cannot_lose_motion_time() {
        let start = Instant::now();
        let mut hinge = Hinge::new(&door(0), start).unwrap();
        hinge.action(2, start);
        hinge.tick(start + Duration::from_micros(500));
        assert_eq!(hinge.fraction(), 0.0);
        hinge.tick(start + Duration::from_millis(1));
        assert!(hinge.fraction() > 0.0);
        let fraction = hinge.fraction();
        hinge.tick(start);
        assert_eq!(hinge.fraction(), fraction);
        hinge.tick(start + Duration::from_millis(800));
        assert!(hinge.settled());
    }
}
