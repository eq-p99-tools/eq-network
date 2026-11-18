use super::*;
use std::time::Duration;

fn vessel() -> Vessel {
    Vessel {
        spawn_id: 8,
        incarnation: 4,
    }
}
fn echo(id: u16, action: Action) -> ControlBoat {
    ControlBoat {
        boat_id: u32::from(id),
        action,
        unknown: [0; 3],
    }
}
fn submit(control: &mut Control, intent: Intent, now: Instant) {
    assert!(control.submitted(5, vessel(), intent, now, now + Duration::from_secs(5)));
}

#[test]
fn a_transmitted_request_is_not_control_and_only_its_live_echo_can_acquire() {
    let now = Instant::now();
    let mut control = Control::new(5);
    control.received(5, echo(8, Action::Acquire), now);
    assert_eq!(control.state(), ControlState::Unknown);
    submit(&mut control, Intent::Acquire, now);
    for (admission, packet) in [
        (4, echo(8, Action::Acquire)),
        (5, echo(9, Action::Acquire)),
        (5, echo(8, Action::Unknown(2))),
        (5, echo(8, Action::Release)),
    ] {
        control.received(admission, packet, now);
        assert!(matches!(control.state(), ControlState::Acquiring { .. }));
    }
    let mut altered = echo(8, Action::Acquire);
    altered.unknown = [1, 2, 3];
    control.received(5, altered, now);
    assert!(matches!(control.state(), ControlState::Acquiring { .. }));
    control.received(5, echo(8, Action::Acquire), now);
    assert_eq!(control.state(), ControlState::Acquired(vessel()));
}

#[test]
fn release_intent_stops_locally_before_an_echo_and_a_late_acquire_cannot_restore_it() {
    let now = Instant::now();
    let mut control = Control::new(5);
    submit(&mut control, Intent::Acquire, now);
    submit(&mut control, Intent::Release, now);
    control.received(5, echo(8, Action::Acquire), now);
    assert_eq!(control.state(), ControlState::Releasing(vessel()));
    control.received(5, echo(8, Action::Release), now);
    assert_eq!(control.state(), ControlState::Released(vessel()));
    control.received(5, echo(8, Action::Acquire), now);
    assert_eq!(control.state(), ControlState::Released(vessel()));
}

#[test]
fn expired_and_cancelled_requests_require_a_release_fence_before_same_id_retry() {
    let now = Instant::now();
    let mut control = Control::new(5);
    submit(&mut control, Intent::Acquire, now);
    control.received(5, echo(8, Action::Acquire), now + Duration::from_secs(5));
    assert_eq!(control.state(), ControlState::Unconfirmed(vessel()));
    let next = Vessel {
        incarnation: 7,
        ..vessel()
    };
    assert!(!control.submitted(5, next, Intent::Acquire, now, now + Duration::from_secs(5)));
    assert!(!control.may_acquire(next));
    control.cancel();
    control.received(5, echo(8, Action::Acquire), now);
    assert_eq!(control.state(), ControlState::Unconfirmed(vessel()));
    let mut next_admission = Control::new(6);
    assert!(!next_admission.submitted(5, next, Intent::Acquire, now, now + Duration::from_secs(5)));
    next_admission.received(5, echo(8, Action::Acquire), now);
    assert_eq!(next_admission.state(), ControlState::Unknown);
}

#[test]
fn support_loss_and_retirement_withdraw_the_exact_incarnation() {
    let now = Instant::now();
    let mut control = Control::new(5);
    submit(&mut control, Intent::Acquire, now);
    control.received(5, echo(8, Action::Acquire), now);
    assert_eq!(control.support_changed(Some(vessel())), None);
    assert_eq!(control.support_changed(None), Some(vessel()));
    control.received(5, echo(8, Action::Acquire), now);
    assert_eq!(control.state(), ControlState::Unconfirmed(vessel()));
    assert_eq!(control.support_changed(None), None);
    let mut other = Control::new(5);
    submit(&mut other, Intent::Acquire, now);
    other.retired(Vessel {
        incarnation: 2,
        ..vessel()
    });
    assert!(matches!(other.state(), ControlState::Acquiring { .. }));
    other.retired(vessel());
    other.received(5, echo(8, Action::Acquire), now);
    assert_eq!(other.state(), ControlState::Unconfirmed(vessel()));
}

#[test]
fn an_ordered_release_echo_fences_prior_acquires_and_allows_ordinary_reacquisition() {
    let now = Instant::now();
    let mut control = Control::new(5);
    submit(&mut control, Intent::Acquire, now);
    submit(&mut control, Intent::Release, now);
    // The reliable transport delivers the older acquire echo before release.
    control.received(5, echo(8, Action::Acquire), now);
    assert_eq!(control.state(), ControlState::Releasing(vessel()));
    control.received(5, echo(8, Action::Release), now);
    assert!(control.may_acquire(vessel()));
    submit(&mut control, Intent::Acquire, now);
    control.received(5, echo(8, Action::Acquire), now);
    assert_eq!(control.state(), ControlState::Acquired(vessel()));
}

#[test]
fn a_late_release_fence_does_not_restore_control_or_cancel_another_vessels_request() {
    let now = Instant::now();
    let mut control = Control::new(5);
    submit(&mut control, Intent::Acquire, now);
    submit(&mut control, Intent::Release, now);
    control.expire(now + Duration::from_secs(5));
    assert!(!control.may_acquire(vessel()));
    let other = Vessel {
        spawn_id: 9,
        incarnation: 5,
    };
    assert!(control.submitted(5, other, Intent::Acquire, now, now + Duration::from_secs(5)));
    control.received(5, echo(8, Action::Release), now);
    assert!(control.may_acquire(vessel()));
    assert_eq!(control.state(), ControlState::Acquiring { vessel: other });
    control.received(5, echo(9, Action::Acquire), now);
    assert_eq!(control.state(), ControlState::Acquired(other));
}
