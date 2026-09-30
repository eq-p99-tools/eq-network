//! Camping: the official client's 30-second timer between `OP_Camp` and `OP_Logout`.
//!
//! The server only starts its own camp timer when it receives `OP_Camp`; the client
//! decides when to log out, and standing or moving abandons the attempt.
use super::{ClientCommand, ClientEvent, Events, Session};
use anyhow::Result;
use eq_network_game::{
    command::Posture,
    world::{CampStatus, WorldEvent},
};
use std::time::{Duration, Instant};

/// `OP_Camp`, a four-byte request.
pub(super) const CAMP_OPCODE: u16 = 0x78c1;
/// `OP_Logout`, an empty request sent once the camp timer completes.
pub(super) const LOGOUT_OPCODE: u16 = 0x61ff;
/// `OP_LogoutReply`, after which the zone connection ends.
pub(super) const LOGOUT_REPLY_OPCODE: u16 = 0x3cdc;

const CAMP_DURATION: Duration = Duration::from_secs(30);
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
enum Phase {
    #[default]
    Idle,
    Preparing(Instant),
    LoggingOut(Instant),
}

/// Local camp progress for one zone admission.
#[derive(Debug, Default)]
pub(super) struct Camp {
    phase: Phase,
}

impl Camp {
    /// Whether a camp or logout is in progress.
    pub(super) fn active(&self) -> bool {
        !matches!(self.phase, Phase::Idle)
    }

    /// Starts the timer after `OP_Camp` was handed to transport.
    pub(super) fn start(&mut self, now: Instant) {
        if matches!(self.phase, Phase::Idle) {
            self.phase = Phase::Preparing(now);
        }
    }

    /// Abandons preparation; returns false when nothing was being prepared.
    /// A logout already sent cannot be taken back.
    pub(super) fn cancel(&mut self) -> bool {
        if matches!(self.phase, Phase::Preparing(_)) {
            self.phase = Phase::Idle;
            true
        } else {
            false
        }
    }

    /// Whether the preparation time has elapsed and `OP_Logout` should be sent.
    pub(super) fn logout_due(&self, now: Instant) -> bool {
        matches!(self.phase, Phase::Preparing(started)
            if now.saturating_duration_since(started) >= CAMP_DURATION)
    }

    /// Records that `OP_Logout` was sent.
    pub(super) fn logout_sent(&mut self, now: Instant) {
        self.phase = Phase::LoggingOut(now);
    }

    /// Whether the logout reply should end the zone connection.
    pub(super) fn logging_out(&self) -> bool {
        matches!(self.phase, Phase::LoggingOut(_))
    }

    /// Whether a sent logout has waited too long for its reply.
    pub(super) fn reply_overdue(&self, now: Instant) -> bool {
        matches!(self.phase, Phase::LoggingOut(sent)
            if now.saturating_duration_since(sent) >= REPLY_TIMEOUT)
    }
}

/// Starts camping, or abandons preparation when the character stands or moves.
/// Returns true when the command was fully handled here.
pub(super) fn handle(
    camp: &mut Camp,
    session_id: u64,
    own_spawn: Option<u16>,
    command: &ClientCommand,
    session: &mut Session,
    log: &mut Events<'_>,
) -> Result<bool> {
    if let ClientCommand::Camp {
        session_id: requested,
        created,
    } = command
    {
        let now = Instant::now();
        if *requested != session_id
            || *created > now
            || now.duration_since(*created) >= Duration::from_secs(1)
        {
            log.send(ClientEvent::World(WorldEvent::Camp(CampStatus::Rejected(
                "Camp request expired".into(),
            ))))?;
        } else if !camp.active() {
            session.send(CAMP_OPCODE, &[0; 4])?;
            camp.start(now);
            log.send(ClientEvent::World(WorldEvent::Camp(CampStatus::Preparing)))?;
        }
        return Ok(true);
    }
    let abandons = matches!(
        command,
        ClientCommand::SetPosture {
            posture: Posture::Standing | Posture::Ducking,
            ..
        } | ClientCommand::Move(_)
            | ClientCommand::CrossZoneLine { .. }
            | ClientCommand::ClickDoor { .. }
    );
    if abandons && camp.cancel() {
        // Only a Standing appearance stops the server's own camp timer (EQEmu
        // client_packet.cpp, OP_SpawnAppearance); without it the server logs the
        // character out of its group and guild 29 seconds after /camp. Moving,
        // ducking or opening a door stands the camping character up first.
        let standing = matches!(
            command,
            ClientCommand::SetPosture {
                posture: Posture::Standing,
                ..
            }
        );
        if let (false, Some(spawn_id)) = (standing, own_spawn) {
            let stand = eq_network_game::command::titanium_posture(spawn_id, Posture::Standing)?;
            session.send(stand.opcode, &stand.body)?;
        }
        log.send(ClientEvent::World(WorldEvent::Camp(CampStatus::Abandoned)))?;
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camp_waits_thirty_seconds_and_standing_cancels_only_before_logout() {
        let now = Instant::now();
        let mut camp = Camp::default();
        assert!(!camp.active() && !camp.cancel());
        camp.start(now);
        assert!(camp.active());
        assert!(!camp.logout_due(now + Duration::from_secs(29)));
        assert!(camp.cancel());
        assert!(!camp.active());
        camp.start(now);
        camp.start(now + Duration::from_secs(20));
        assert!(camp.logout_due(now + CAMP_DURATION));
        camp.logout_sent(now + CAMP_DURATION);
        assert!(camp.logging_out() && !camp.logout_due(now + CAMP_DURATION));
        assert!(!camp.cancel());
        assert!(!camp.reply_overdue(now + CAMP_DURATION + Duration::from_secs(9)));
        assert!(camp.reply_overdue(now + CAMP_DURATION + REPLY_TIMEOUT));
    }
}
