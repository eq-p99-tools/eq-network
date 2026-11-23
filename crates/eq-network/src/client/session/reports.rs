//! Petitions, bug reports and feedback to the server's staff
//! ([`eq_network_game::reports`]).
//!
//! A person reads each of these, so a server type lists a report only after
//! its own live check (recording checklist 6.511e and f). Today that is
//! stock `EQEmu`'s petition, bug report and feedback, and TAKP's and
//! Quarm's petition and bug report. Stock `EQEmu` reads no feedback and
//! drops it; it is listed there because Titanium's program sends it all
//! the same (read R49-23), so the player's window does what the original's
//! does. `EQMac` feedback is built and tested, and listed nowhere yet.
//!
//! The session paces each kind on its own: a second report of a kind waits
//! [`BETWEEN`] after the first. This bound is this library's own (inferred,
//! 6.511a), so a held key cannot fill a staff queue.
//!
//! What the servers answer, so a host adds its own line only where the
//! server sends none (6.511b):
//! - Petition, stock `EQEmu` (`Client::Handle_OP_Petition`): nothing to the
//!   player on success. Refusals are "You already have a petition in the
//!   queue, you must wait for it to be answered or use /deletepetition to
//!   delete it." and "Error: World server disconnected".
//! - Bug report, stock `EQEmu` (`Client::Handle_OP_Bug`, `RegisterBug`):
//!   nothing on success (its note: "Client sends success message"). It
//!   answers "Bug reporting is disabled on this server." when
//!   `Bugs:ReportingSystemActive` is off, and "Failed to created your bug
//!   report." when the store fails.
//! - TAKP (not listed): a petition is refused with "Petition reporting is
//!   disabled on this server." under `Petitions:PetitionSystemActive`, and
//!   feedback is answered with "Thank you, NAME. Your feedback has been
//!   recieved."
//!
//! Neither the report's text nor any part of it goes into a diagnostic or a
//! refusal reason.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    reports::{self, ReportError, ReportKind},
    world::WorldEvent,
};
use std::time::{Duration, Instant};

/// The wait between two reports of a kind (inferred, 6.511a).
pub(super) const BETWEEN: Duration = Duration::from_secs(10);

/// Which reports a server type takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Listed {
    petitions: bool,
    bugs: bool,
    feedback: bool,
}

impl Listed {
    /// Stock `EQEmu`'s: petitions and bug reports, as its handlers read them,
    /// and feedback, which it drops as it drops the official program's.
    pub(super) const EQEMU: Self = Self {
        petitions: true,
        bugs: true,
        feedback: true,
    };

    /// TAKP's and Quarm's: petitions and bug reports, as their own
    /// `Handle_OP_Petition` and `Handle_OP_Bug` read them (6.604 E6,
    /// source-derived). Quarm keeps petitions off by rule and answers with
    /// a line. Tests never send either to a public server: they reach the
    /// server's staff.
    pub(super) const EQMAC: Self = Self {
        petitions: true,
        bugs: true,
        feedback: false,
    };
}

/// Sends the player's reports, one kind at a time.
pub(super) struct Reports {
    listed: Listed,
    /// When each kind last went out.
    sent: [Option<Instant>; 3],
}

impl Reports {
    pub(super) const fn new(listed: Listed) -> Self {
        Self {
            listed,
            sent: [None; 3],
        }
    }

    const fn slot(kind: ReportKind) -> usize {
        match kind {
            ReportKind::Petition => 0,
            ReportKind::Bug => 1,
            ReportKind::Feedback => 2,
        }
    }

    const fn listed(&self, kind: ReportKind) -> bool {
        match kind {
            ReportKind::Petition => self.listed.petitions,
            ReportKind::Bug => self.listed.bugs,
            ReportKind::Feedback => self.listed.feedback,
        }
    }
}

/// The kind a command sends, if it is a report.
const fn kind(command: &ClientCommand) -> Option<ReportKind> {
    match command {
        ClientCommand::SendPetition { .. } => Some(ReportKind::Petition),
        ClientCommand::ReportBug { .. } => Some(ReportKind::Bug),
        ClientCommand::SendFeedback { .. } => Some(ReportKind::Feedback),
        _ => None,
    }
}

/// The words for a kind, in a refusal.
const fn words(kind: ReportKind) -> &'static str {
    match kind {
        ReportKind::Petition => "petition",
        ReportKind::Bug => "bug report",
        ReportKind::Feedback => "feedback",
    }
}

impl Feature for Reports {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        [
            (self.listed.petitions, Capability::Petitioning),
            (self.listed.bugs, Capability::ReportingBugs),
            (self.listed.feedback, Capability::SendingFeedback),
        ]
        .into_iter()
        .filter_map(|(listed, capability)| listed.then_some(capability))
        .collect()
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        kind(command).is_some_and(|kind| self.listed(kind))
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let Some(kind) = kind(command) else {
            return Ok(());
        };
        let slot = Self::slot(kind);
        if self.sent[slot].is_some_and(|sent| sent.elapsed() < BETWEEN) {
            return actions::refuse(
                command,
                &format!("Wait a little before sending another {}.", words(kind)),
                out.log,
            );
        }
        let dialect = out.wire.dialect();
        let packet = match command {
            ClientCommand::SendPetition { text, .. } => reports::petition(dialect, text),
            ClientCommand::ReportBug { report, .. } => {
                reports::bug(dialect, out.sender.name, report)
            }
            ClientCommand::SendFeedback { text, place, .. } => {
                reports::feedback(dialect, out.sender.name, text, *place)
            }
            _ => return Ok(()),
        };
        // The encoder's refusal is a kind, never the text.
        let packet = match packet {
            Ok(packet) => packet,
            Err(error) => return actions::refuse(command, &refusal(kind, error), out.log),
        };
        out.send(&packet)?;
        self.sent[slot] = Some(Instant::now());
        let session_id = command.session_id().unwrap_or_default();
        out.log.send(ClientEvent::World(WorldEvent::ReportSent {
            session_id,
            report: kind,
        }))
    }
}

/// The player's words for an encoder's refusal.
fn refusal(kind: ReportKind, error: ReportError) -> String {
    let what = words(kind);
    match error {
        ReportError::Empty => format!("Say what the {what} is about."),
        ReportError::TooLong { .. } => format!("That {what} is too long."),
        ReportError::HoldsNul => format!("That {what} holds text that cannot be sent."),
        ReportError::NotRead => format!("This server does not take a {what}."),
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use crate::world::Capability;
    use eq_network_game::reports::{BugCategory, BugPlace, BugReport, BugTarget};

    fn petition(text: &str) -> ClientCommand {
        ClientCommand::SendPetition {
            session_id: 5,
            text: text.into(),
            created: Instant::now(),
        }
    }

    fn bug(text: &str) -> ClientCommand {
        ClientCommand::ReportBug {
            session_id: 5,
            report: BugReport {
                category: BugCategory::DoorsAndPortals,
                text: text.into(),
                place: Some(BugPlace {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                    heading: 128.0,
                }),
                target: Some(BugTarget {
                    spawn_id: 9,
                    name: "Synthetic_Door00".into(),
                }),
            },
            created: Instant::now(),
        }
    }

    fn feedback(text: &str) -> ClientCommand {
        ClientCommand::SendFeedback {
            session_id: 5,
            text: text.into(),
            place: None,
            created: Instant::now(),
        }
    }

    fn refusals(events: &[ClientEvent]) -> Vec<(ReportKind, String)> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::ReportRefused { report, reason, .. }) => {
                    Some((*report, reason.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn sent(events: &[ClientEvent]) -> Vec<ReportKind> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::ReportSent { report, .. }) => Some(*report),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn eqemu_lists_every_report_and_the_mac_servers_no_feedback() {
        let feature = Reports::new(Listed::EQEMU);
        assert_eq!(
            feature.capabilities(),
            [
                Capability::Petitioning,
                Capability::ReportingBugs,
                Capability::SendingFeedback
            ]
        );
        assert!(feature.owns(&petition("Help")));
        assert!(feature.owns(&bug("Stuck")));
        assert!(feature.owns(&feedback("Thanks")));
        assert!(!Reports::new(Listed::EQMAC).owns(&feedback("Thanks")));
    }

    #[test]
    fn a_petition_goes_once_then_waits_and_names_no_text_when_refused() {
        let mut feature = Reports::new(Listed::EQEMU);
        let mut world = World::new(5);
        let first = testing::run(|out| feature.handle(&petition("I am stuck."), &mut world, out));
        first.result.unwrap();
        assert_eq!(
            first.sent,
            [reports::petition(eq_network_game::GameDialect::Titanium, "I am stuck.").unwrap()]
        );
        assert_eq!(sent(&first.events), [ReportKind::Petition]);
        // A second petition within the wait is refused; a bug report is not.
        let second = testing::run(|out| feature.handle(&petition("Again."), &mut world, out));
        second.result.unwrap();
        assert_eq!(second.sent, []);
        assert_eq!(
            refusals(&second.events),
            [(
                ReportKind::Petition,
                "Wait a little before sending another petition.".to_owned()
            )]
        );
        let other = testing::run(|out| feature.handle(&bug("The door."), &mut world, out));
        other.result.unwrap();
        assert_eq!(other.sent.len(), 1);
        assert_eq!(other.sent[0].opcode, reports::TITANIUM_BUG_OPCODE);
        // Past the wait, a petition goes again.
        feature.sent[Reports::slot(ReportKind::Petition)] = Instant::now().checked_sub(BETWEEN * 2);
        let later = testing::run(|out| feature.handle(&petition("Later."), &mut world, out));
        later.result.unwrap();
        assert_eq!(later.sent.len(), 1);
    }

    #[test]
    fn a_report_the_packet_cannot_hold_is_refused_without_its_text() {
        let mut feature = Reports::new(Listed::EQEMU);
        let mut world = World::new(5);
        let secret = format!("secret {}", "x".repeat(3000));
        for (command, reason) in [
            (petition("   "), "Say what the petition is about."),
            (bug(&secret), "That bug report is too long."),
            (
                petition("a\0b"),
                "That petition holds text that cannot be sent.",
            ),
        ] {
            let outcome = testing::run(|out| feature.handle(&command, &mut world, out));
            outcome.result.unwrap();
            assert_eq!(outcome.sent, []);
            let refused = refusals(&outcome.events);
            assert_eq!(refused.len(), 1);
            assert_eq!(refused[0].1, reason);
            for event in &outcome.events {
                assert!(!format!("{event:?}").contains("secret"));
            }
        }
        // Nothing went, so nothing waits.
        assert_eq!(feature.sent, [None; 3]);
    }

    #[test]
    fn the_eqmac_wire_carries_its_own_layouts() {
        let mut feature = Reports::new(Listed {
            petitions: true,
            bugs: true,
            feedback: true,
        });
        let mut world = World::new(5);
        let outcome = testing::run_on(&super::super::wire::EqMac, |out| {
            feature.handle(&feedback("Thanks."), &mut world, out)?;
            feature.handle(&bug("The door."), &mut world, out)
        });
        outcome.result.unwrap();
        let opcodes: Vec<_> = outcome.sent.iter().map(|packet| packet.opcode).collect();
        assert_eq!(
            opcodes,
            [reports::EQMAC_FEEDBACK_OPCODE, reports::EQMAC_BUG_OPCODE]
        );
        assert_eq!(outcome.sent[0].body.len(), 1131);
        assert_eq!(outcome.sent[1].body.len(), 1131);
    }

    /// TAKP's opcode list writes each opcode as the reverse of the bytes this
    /// crate's `EQMac` tables give it. Rows the coverage table already holds
    /// prove the rule; the three report opcodes follow it, and no server
    /// sends any of them.
    #[test]
    fn the_eqmac_report_opcodes_follow_the_coverage_tables_byte_order() {
        let takp = |opcode: u16| opcode.swap_bytes();
        for (listed, name) in [
            (0x4011, "OP_PetitionRefresh"),
            (0x418e, "OP_PetitionCheckout"),
            (0x41ba, "OP_ApplyPoison"),
            (0x41da, "OP_YellForHelp"),
        ] {
            assert_eq!(
                super::super::coverage::eqmac(takp(listed)).map(|row| row.name),
                Some(name)
            );
        }
        for (listed, ours) in [
            (0x400e, reports::EQMAC_PETITION_OPCODE),
            (0x40b3, reports::EQMAC_BUG_OPCODE),
            (0x413c, reports::EQMAC_FEEDBACK_OPCODE),
        ] {
            assert_eq!(takp(listed), ours);
            assert!(super::super::coverage::eqmac(ours).is_none());
        }
        for ours in [
            reports::TITANIUM_PETITION_OPCODE,
            reports::TITANIUM_BUG_OPCODE,
        ] {
            assert!(super::super::coverage::titanium(ours).is_none());
        }
    }
}
