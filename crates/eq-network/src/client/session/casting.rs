//! Worker-side exclusion while awaiting a cast acknowledgement or result.
use super::ClientCommand;
use eq_network_game::spells::SpellUpdate;
use std::time::{Duration, Instant};

/// Local retry policy, not a protocol timeout. Never resends a request automatically.
const ACK_TIMEOUT: Duration = Duration::from_secs(5);

/// Reports only cast rejections, preserving the caller's admission for host filtering.
pub(super) fn rejected(command: &ClientCommand, reason: &str) -> Option<crate::world::WorldEvent> {
    let ClientCommand::CastSpell {
        session_id,
        spell_id,
        ..
    } = command
    else {
        return None;
    };
    Some(crate::world::WorldEvent::CastRejected {
        session_id: *session_id,
        spell_id: *spell_id,
        reason: reason.into(),
    })
}

enum Phase {
    Awaiting { spell: u32, submitted: Instant },
    Casting { spell: u32 },
}

#[derive(Default)]
pub(super) struct CastGuard {
    phase: Option<Phase>,
}

impl CastGuard {
    /// Excludes spell changes while leaving movement and posture interruption available.
    pub fn blocks(&self, command: &ClientCommand) -> bool {
        self.active()
            && matches!(
                command,
                ClientCommand::CastSpell { .. }
                    | ClientCommand::UseItem(_)
                    | ClientCommand::ScribeSpell { .. }
                    | ClientCommand::MemorizeSpell { .. }
                    | ClientCommand::ForgetSpell { .. }
                    | ClientCommand::DeleteSpell { .. }
                    | ClientCommand::SwapSpell { .. }
            )
    }

    /// A duration reaching zero does not authorize another cast: await a server result.
    pub fn active(&self) -> bool {
        self.phase.is_some()
    }

    /// Records a request only after successful transport submission.
    pub fn submitted(&mut self, spell: u32, now: Instant) {
        self.phase = Some(Phase::Awaiting {
            spell,
            submitted: now,
        });
    }

    /// The unacknowledged request, distinct from a server-confirmed cast.
    pub fn pending(&self) -> Option<u32> {
        match self.phase {
            Some(Phase::Awaiting { spell, .. }) => Some(spell),
            _ => None,
        }
    }

    /// Allows a fresh user attempt after silence; confirmed casts never expire by time.
    pub fn expire(&mut self, now: Instant) -> bool {
        if matches!(self.phase, Some(Phase::Awaiting { submitted, .. })
            if now.checked_duration_since(submitted).is_some_and(|age| age >= ACK_TIMEOUT))
        {
            self.clear();
            true
        } else {
            false
        }
    }

    fn spell(&self) -> Option<u32> {
        match self.phase {
            Some(Phase::Awaiting { spell, .. } | Phase::Casting { spell }) => Some(spell),
            None => None,
        }
    }

    /// Clears state when this character dies or the owning zone session ends.
    pub fn clear(&mut self) {
        self.phase = None;
    }

    /// Only own-caster starts/interruptions and matching result notifications affect us.
    pub fn observe(&mut self, own_id: u16, update: &SpellUpdate) {
        match *update {
            SpellUpdate::Began {
                caster_id,
                spell_id,
                ..
            } if caster_id == own_id => {
                self.phase = Some(Phase::Casting {
                    spell: u32::from(spell_id),
                });
            }
            SpellUpdate::Interrupted { caster_id, .. } if caster_id == u32::from(own_id) => {
                self.clear();
            }
            SpellUpdate::Mana {
                spell_id,
                keep_casting: false,
            }
            | SpellUpdate::BarRefresh { spell_id, .. }
                if self.spell() == Some(spell_id) =>
            {
                self.clear();
            }
            _ => (),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        guard.submitted(42, Instant::now());
        assert!(matches!(rejected(&request, "Target unavailable"),
            Some(crate::world::WorldEvent::CastRejected { session_id: 12, spell_id: 73, reason })
            if reason == "Target unavailable"));
        assert!(guard.blocks(&request));
        assert!(rejected(
            &ClientCommand::SelectTarget {
                session_id: 12,
                spawn_id: Some(7)
            },
            "Unavailable"
        )
        .is_none());
    }

    #[test]
    fn item_activations_share_cast_exclusion_without_blocking_posture_interrupts() {
        let now = Instant::now();
        let request = ClientCommand::UseItem(eq_network_game::inventory::ItemUse {
            request_id: 1,
            session_id: 12,
            revision: 1,
            slot: eq_network_game::inventory::InventorySlot(22),
            target_id: 7,
            created: now,
        });
        let mut guard = CastGuard::default();
        assert!(!guard.blocks(&request));
        guard.submitted(73, now);
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
        guard.submitted(73, now);
        assert!(guard.active());
        assert_eq!(guard.pending(), Some(73));
        assert!(!guard.expire(now + Duration::from_millis(4999)));
        assert!(guard.expire(now + ACK_TIMEOUT));
        assert!(!guard.active());
        assert!(!guard.expire(now + ACK_TIMEOUT));
        guard.submitted(73, now);
        guard.observe(12, &begin(13));
        assert_eq!(guard.pending(), Some(73));
        guard.observe(12, &begin(12));
        assert_eq!(guard.pending(), None);
        assert!(guard.active());
        assert!(!guard.expire(now + Duration::from_secs(60)));
        guard.observe(
            12,
            &SpellUpdate::Interrupted {
                caster_id: 12,
                message_id: 1,
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
            guard.submitted(73, Instant::now());
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
    fn interrupt_refresh_and_lifecycle_reset_release_the_guard() {
        let mut guard = CastGuard::default();
        for update in [
            SpellUpdate::Interrupted {
                caster_id: 12,
                message_id: 1,
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
            session_id: 1,
            spawn_id: Some(13)
        }));
        guard.observe(
            12,
            &SpellUpdate::Interrupted {
                caster_id: 12,
                message_id: 1,
            },
        );
        assert!(commands.iter().all(|command| !guard.blocks(command)));
    }
}
