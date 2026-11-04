//! One deletion owner per world connection. Sending is never an acknowledgement.
use super::{ClientEvent, Events};
use crate::client::selection::Selection;
use anyhow::Result;
use eq_network_game::{
    deletion::{Observation, Presence, Progress, Refusal, Request, Unconfirmed, Update},
    world::WorldEvent,
};
use eq_network_transport::Transport;
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

pub(super) struct Owner {
    connection_id: u64,
    closed: bool,
    highest_request: u64,
    record: Option<Update>,
    started: Option<Instant>,
}

impl Owner {
    pub fn new() -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let connection_id = next_connection(&NEXT)?;
        Ok(Self {
            connection_id,
            closed: false,
            highest_request: 0,
            record: None,
            started: None,
        })
    }

    pub const fn connection_id(&self) -> u64 {
        self.connection_id
    }

    pub fn blocked(&self) -> bool {
        self.record
            .as_ref()
            .is_some_and(|record| record.progress.unresolved())
    }

    /// Reject locally, or submit exactly once through the ordinary transport.
    pub fn request(
        &mut self,
        request: Request,
        list: &Selection,
        busy: bool,
        transport: &mut dyn Transport,
        log: &mut Events<'_>,
    ) -> Result<()> {
        if self.closed {
            return tell(
                Update {
                    request,
                    progress: Progress::NotSent(Refusal::NotAtSelection),
                },
                log,
            );
        }
        if request.scope().connection_id != self.connection_id {
            return tell(
                Update {
                    request,
                    progress: Progress::NotSent(Refusal::StaleConnection),
                },
                log,
            );
        }
        if let Some(record) = self
            .record
            .as_ref()
            .filter(|record| record.request == request)
        {
            return tell(record.clone(), log);
        }
        if request.request_id() <= self.highest_request {
            return tell(
                Update {
                    request,
                    progress: Progress::NotSent(Refusal::Duplicate),
                },
                log,
            );
        }
        // A rejected new intent cannot become executable by replaying it later.
        self.highest_request = request.request_id();
        let refusal = if busy || self.blocked() {
            Some(Refusal::Busy)
        } else if request.scope() != list.scope() {
            Some(Refusal::StaleList)
        } else if list.deletion().is_none() {
            Some(Refusal::Unsupported)
        } else if !list.contains(request.name()) {
            Some(Refusal::NotListed)
        } else {
            None
        };
        if let Some(refusal) = refusal {
            let update = Update {
                request,
                progress: Progress::NotSent(refusal),
            };
            if !self.blocked() {
                self.record = Some(update.clone());
            }
            return tell(update, log);
        }
        let packet = list
            .deletion()
            .expect("capability checked above")
            .encode(&request);
        self.record = Some(Update {
            request,
            progress: Progress::Pending(None),
        });
        self.started = Some(Instant::now());
        if let Err(error) = transport.send(packet.opcode, &packet.body) {
            self.record
                .as_mut()
                .expect("recorded before sending")
                .progress = Progress::Unconfirmed {
                reason: Unconfirmed::SendFailed,
                observation: None,
            };
            self.publish(log)?;
            return Err(error);
        }
        self.publish(log)
    }

    /// Only a newly published full list can supply presence evidence. It cannot
    /// prove causality, server acceptance or permanent database removal.
    pub fn observe(&mut self, list: &Selection, log: &mut Events<'_>) -> Result<()> {
        if self.closed || list.scope().connection_id != self.connection_id {
            return Ok(());
        }
        let Some(record) = self
            .record
            .as_mut()
            .filter(|record| record.progress.unresolved())
        else {
            return Ok(());
        };
        if list.scope().selection_id <= record.request.scope().selection_id
            || record
                .progress
                .observation()
                .is_some_and(|seen| seen.scope.selection_id >= list.scope().selection_id)
        {
            return Ok(());
        }
        let observation = Observation {
            scope: list.scope(),
            presence: if list.contains(record.request.name()) {
                Presence::Present
            } else {
                Presence::Absent
            },
        };
        record.progress = match (observation.presence, &record.progress) {
            (Presence::Absent, _) => Progress::ObservedAbsent {
                scope: observation.scope,
            },
            (Presence::Present, Progress::Unconfirmed { reason, .. }) => Progress::Unconfirmed {
                reason: *reason,
                observation: Some(observation),
            },
            (Presence::Present, _) => Progress::Pending(Some(observation)),
        };
        self.publish(log)
    }

    /// A local deadline leaves selection held until a later list or disconnect.
    pub fn tick(&mut self, log: &mut Events<'_>) -> Result<()> {
        self.tick_at(Instant::now(), log)
    }

    fn tick_at(&mut self, now: Instant, log: &mut Events<'_>) -> Result<()> {
        if self
            .started
            .is_some_and(|start| now.saturating_duration_since(start) >= Duration::from_secs(60))
        {
            if let Some(record) = self.record.as_mut() {
                if let Progress::Pending(observation) = record.progress {
                    record.progress = Progress::Unconfirmed {
                        reason: Unconfirmed::TimedOut,
                        observation,
                    };
                    return self.publish(log);
                }
            }
        }
        Ok(())
    }

    /// Called on every return from either world lifecycle, including errors.
    pub fn ended(&mut self, log: &mut Events<'_>) -> Result<()> {
        self.closed = true;
        if let Some(record) = self
            .record
            .as_mut()
            .filter(|record| record.progress.unresolved())
        {
            record.progress = Progress::Unconfirmed {
                reason: Unconfirmed::ConnectionEnded,
                observation: record.progress.observation(),
            };
            return self.publish(log);
        }
        Ok(())
    }

    fn publish(&self, log: &mut Events<'_>) -> Result<()> {
        tell(
            self.record
                .as_ref()
                .expect("an update was recorded")
                .clone(),
            log,
        )
    }
}

fn next_connection(counter: &AtomicU64) -> Result<u64> {
    counter
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map_err(|_| anyhow::anyhow!("world-connection identity exhausted"))
}

fn tell(update: Update, log: &mut Events<'_>) -> Result<()> {
    log.send(ClientEvent::World(WorldEvent::CharacterDeletion(update)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eq_network_game::{
        characters::CharacterChoice,
        deletion::{Rules, Scope},
    };

    #[derive(Default)]
    struct Wire {
        packets: Vec<(u16, Vec<u8>)>,
        fail: bool,
    }
    impl Transport for Wire {
        fn send(&mut self, opcode: u16, body: &[u8]) -> Result<()> {
            self.packets.push((opcode, body.to_vec()));
            anyhow::ensure!(!self.fail, "synthetic send failure");
            Ok(())
        }
        fn receive(&mut self) -> Result<Option<eq_network_transport::Application>> {
            Ok(None)
        }
        fn close(&mut self) -> Result<()> {
            Ok(())
        }
        fn last_received_seconds(&self) -> u64 {
            0
        }
    }
    fn list(owner: &Owner, names: &[&str], log: &mut Events<'_>) -> Selection {
        Selection::new(
            names
                .iter()
                .enumerate()
                .map(|(slot, name)| CharacterChoice {
                    slot: u8::try_from(slot).unwrap(),
                    name: (*name).into(),
                    level: None,
                    zone_id: None,
                    look: None,
                })
                .collect(),
            "",
            None,
            owner.connection_id(),
            Some(Rules::Titanium),
            log,
        )
        .unwrap()
        .0
    }
    fn request(list: &Selection, id: u64, name: &str) -> Request {
        Request::new(list.scope(), id, name, name).unwrap()
    }
    fn with_log(test: impl FnOnce(&mut Events<'_>)) -> Vec<Update> {
        let config = crate::client::ClientConfig::new("ACCOUNT", "PASSWORD", "Server", "");
        let mut updates = Vec::new();
        let mut handler = |event| {
            if let ClientEvent::World(WorldEvent::CharacterDeletion(update)) = event {
                updates.push(update);
            }
            Ok(())
        };
        test(&mut Events::new(&config, &mut handler));
        updates
    }

    #[test]
    fn connection_generations_stop_at_exhaustion_without_reuse() {
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(next_connection(&counter).unwrap(), u64::MAX - 1);
        assert!(next_connection(&counter).is_err());
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
        assert!(next_connection(&counter).is_err());
    }

    #[test]
    fn another_intent_cannot_replace_an_unresolved_owner() {
        let updates = with_log(|log| {
            let mut owner = Owner::new().unwrap();
            let first = list(&owner, &["Disposable", "Other"], log);
            let mut wire = Wire::default();
            owner
                .request(
                    request(&first, 1, "Disposable"),
                    &first,
                    false,
                    &mut wire,
                    log,
                )
                .unwrap();
            owner
                .request(request(&first, 2, "Other"), &first, false, &mut wire, log)
                .unwrap();
            assert_eq!(owner.record.as_ref().unwrap().request.name(), "Disposable");
            owner.ended(log).unwrap();
            assert_eq!(wire.packets.len(), 1);
        });
        assert_eq!(updates[1].progress, Progress::NotSent(Refusal::Busy));
        assert!(matches!(
            updates[2].progress,
            Progress::Unconfirmed {
                reason: Unconfirmed::ConnectionEnded,
                ..
            }
        ));
    }

    #[test]
    fn sends_once_and_only_a_fresh_absent_list_releases_selection() {
        let updates = with_log(|log| {
            let mut owner = Owner::new().unwrap();
            let first = list(&owner, &["Disposable"], log);
            let intent = request(&first, 1, "Disposable");
            let mut wire = Wire::default();
            owner
                .request(intent.clone(), &first, false, &mut wire, log)
                .unwrap();
            owner
                .request(intent.clone(), &first, false, &mut wire, log)
                .unwrap();
            owner.observe(&first, log).unwrap();
            assert!(owner.blocked());
            let present = list(&owner, &["Disposable"], log);
            owner.observe(&present, log).unwrap();
            owner.observe(&present, log).unwrap();
            assert!(owner.blocked());
            let absent = list(&owner, &[], log);
            owner.observe(&absent, log).unwrap();
            assert!(!owner.blocked());
            owner
                .request(intent, &absent, false, &mut wire, log)
                .unwrap();
            let later = list(&owner, &["Disposable"], log);
            owner.observe(&later, log).unwrap();
            assert_eq!(wire.packets, vec![(0x26c9, b"Disposable\0".to_vec())]);
            assert!(matches!(owner.record.as_ref().unwrap().progress,
                Progress::ObservedAbsent { scope } if scope == absent.scope()));
        });
        assert_eq!(updates.len(), 5);
        assert!(matches!(
            updates[2].progress,
            Progress::Pending(Some(Observation {
                presence: Presence::Present,
                ..
            }))
        ));
    }

    #[test]
    fn stale_wrong_identity_busy_and_conflicting_duplicates_never_send() {
        let updates = with_log(|log| {
            let mut owner = Owner::new().unwrap();
            let old = list(&owner, &["Disposable"], log);
            let current = list(&owner, &["Disposable"], log);
            let mut wire = Wire::default();
            for (intent, busy) in [
                (request(&old, 1, "Disposable"), false),
                (request(&current, 2, "disposable"), false),
                (request(&current, 3, "Disposable"), true),
                (request(&current, 3, "Other"), false),
            ] {
                owner
                    .request(intent, &current, busy, &mut wire, log)
                    .unwrap();
            }
            assert_eq!(wire.packets, Vec::new());
        });
        assert_eq!(
            updates.iter().map(|u| &u.progress).collect::<Vec<_>>(),
            vec![
                &Progress::NotSent(Refusal::StaleList),
                &Progress::NotSent(Refusal::NotListed),
                &Progress::NotSent(Refusal::Busy),
                &Progress::NotSent(Refusal::Duplicate)
            ]
        );
    }

    #[test]
    fn connection_generation_defeats_identical_names_and_reused_small_ids() {
        with_log(|log| {
            let old = Owner::new().unwrap();
            let mut fresh = Owner::new().unwrap();
            let current = list(&fresh, &["Disposable"], log);
            let old_intent = Request::new(
                Scope {
                    connection_id: old.connection_id(),
                    selection_id: current.scope().selection_id,
                },
                1,
                "Disposable",
                "Disposable",
            )
            .unwrap();
            let mut wire = Wire::default();
            fresh
                .request(old_intent, &current, false, &mut wire, log)
                .unwrap();
            assert_eq!(wire.packets, Vec::new());
            fresh
                .request(
                    request(&current, 1, "Disposable"),
                    &current,
                    false,
                    &mut wire,
                    log,
                )
                .unwrap();
            assert_eq!(wire.packets.len(), 1);
        });
    }

    #[test]
    fn timeout_still_present_late_absence_and_disconnect_are_distinct() {
        with_log(|log| {
            let mut owner = Owner::new().unwrap();
            let first = list(&owner, &["Disposable"], log);
            let mut wire = Wire::default();
            owner
                .request(
                    request(&first, 1, "Disposable"),
                    &first,
                    false,
                    &mut wire,
                    log,
                )
                .unwrap();
            owner
                .tick_at(owner.started.unwrap() + Duration::from_secs(61), log)
                .unwrap();
            let present = list(&owner, &["Disposable"], log);
            owner.observe(&present, log).unwrap();
            assert!(matches!(
                owner.record.as_ref().unwrap().progress,
                Progress::Unconfirmed {
                    reason: Unconfirmed::TimedOut,
                    observation: Some(Observation {
                        presence: Presence::Present,
                        ..
                    })
                }
            ));
            let absent = list(&owner, &[], log);
            owner.observe(&absent, log).unwrap();
            assert!(!owner.blocked());
            let first = list(&owner, &["Other"], log);
            owner
                .request(request(&first, 2, "Other"), &first, false, &mut wire, log)
                .unwrap();
            owner.ended(log).unwrap();
            let late = list(&owner, &[], log);
            owner.observe(&late, log).unwrap();
            assert!(matches!(
                owner.record.as_ref().unwrap().progress,
                Progress::Unconfirmed {
                    reason: Unconfirmed::ConnectionEnded,
                    ..
                }
            ));
        });
    }

    #[test]
    fn transport_error_is_uncertain_and_a_replay_does_not_resend() {
        with_log(|log| {
            let mut owner = Owner::new().unwrap();
            let first = list(&owner, &["Disposable"], log);
            let intent = request(&first, 1, "Disposable");
            let mut wire = Wire {
                fail: true,
                ..Wire::default()
            };
            assert!(owner
                .request(intent.clone(), &first, false, &mut wire, log)
                .is_err());
            owner
                .request(intent, &first, false, &mut wire, log)
                .unwrap();
            assert_eq!(wire.packets.len(), 1);
            assert!(matches!(
                owner.record.as_ref().unwrap().progress,
                Progress::Unconfirmed {
                    reason: Unconfirmed::SendFailed,
                    ..
                }
            ));
        });
    }
}
