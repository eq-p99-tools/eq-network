//! A Warrior's `/shield` of another player. Stock `EQEmu`'s
//! `Client::Handle_OP_Shielding` checks the level (30), the class, its rule
//! and the 3-minute recast, then `Mob::ShieldAbility` the target, the
//! distance (15) and whether either side already shields or is shielded,
//! and answers each refusal, the start and the end with a line of its own,
//! so the session adds none. It holds no pending state: each word sends one
//! packet, paced so a held hotbutton or a script loop cannot stream them.
//! Stock `EQEmu` lists it from its source (checklist 6.517), and TAKP and
//! Quarm, whose own handlers make the same checks, from theirs (6.603 G2);
//! none is checked live yet.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;
use eq_network_game::request::Request;
use std::time::{Duration, Instant};

/// The least time between two shields sent: the settling time of the
/// other paced sends, twice `EQEmu`'s 250 ms receiver interval (inferred,
/// 6.517).
const PACE: Duration = Duration::from_millis(500);

/// Sends `/shield`, paced.
#[derive(Default)]
pub(super) struct Shielding {
    sent: Option<Instant>,
}

impl Feature for Shielding {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Shielding]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::Shield { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::Shield { spawn_id, .. } = command else {
            return Ok(());
        };
        if *spawn_id == 0 {
            return actions::refuse(command, "Choose a player to shield first.", out.log);
        }
        let now = Instant::now();
        if self
            .sent
            .is_some_and(|sent| now.saturating_duration_since(sent) < PACE)
        {
            return actions::refuse(command, "You just asked to shield.", out.log);
        }
        out.request(&Request::Shield(*spawn_id))?;
        self.sent = Some(now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, ClientEvent};
    use super::*;
    use eq_network_game::world::WorldEvent;

    fn shield(spawn_id: u16) -> ClientCommand {
        ClientCommand::Shield {
            session_id: 5,
            spawn_id,
            created: Instant::now(),
        }
    }

    fn refused(outcome: &testing::Outcome<Result<()>>) -> bool {
        outcome.sent.is_empty()
            && outcome.events.iter().any(|event| {
                matches!(
                    event,
                    ClientEvent::World(WorldEvent::SocialRefused { session_id: 5, .. })
                )
            })
    }

    #[test]
    fn a_shield_is_sent_for_a_spawn_and_paced() {
        let mut world = World::new(5);
        let mut feature = Shielding::default();
        let mut run =
            |command: ClientCommand| testing::run(|out| feature.handle(&command, &mut world, out));
        assert!(refused(&run(shield(0))));
        let sent = run(shield(0x0102));
        assert_eq!(sent.sent.len(), 1);
        assert_eq!(sent.sent[0].opcode, 0x3fe6);
        assert_eq!(sent.sent[0].body, [0x02, 0x01, 0, 0]);
        // A second at once is refused and sends nothing.
        assert!(refused(&run(shield(0x0102))));
        assert!(refused(&run(shield(9))));
    }

    #[test]
    fn a_shield_goes_again_once_the_pace_has_passed() {
        let mut world = World::new(5);
        let mut feature = Shielding {
            sent: Instant::now().checked_sub(PACE),
        };
        let sent = testing::run(|out| feature.handle(&shield(9), &mut world, out));
        assert_eq!(sent.sent.len(), 1);
    }

    #[test]
    fn a_mac_shield_is_two_bytes_and_paced_the_same() {
        use crate::client::{session::servers, ServerProtocol};
        for protocol in [ServerProtocol::Takp, ServerProtocol::Quarm] {
            let wire = servers::server_type(protocol).wire();
            let mut world = World::new(5);
            // 10 ms short of the pace: refused, nothing sent.
            let mut feature = Shielding {
                sent: Instant::now().checked_sub(PACE.saturating_sub(Duration::from_millis(10))),
            };
            let early = testing::run_on(wire, |out| feature.handle(&shield(9), &mut world, out));
            assert!(refused(&early), "{protocol:?}");
            // Once the pace has passed: the 2-byte `Shielding_Struct`.
            let mut feature = Shielding {
                sent: Instant::now().checked_sub(PACE),
            };
            let sent =
                testing::run_on(wire, |out| feature.handle(&shield(0x0102), &mut world, out));
            assert_eq!(sent.sent.len(), 1, "{protocol:?}");
            assert_eq!(
                (sent.sent[0].opcode, sent.sent[0].body.clone()),
                (0x4b42, vec![0x02, 0x01]),
                "{protocol:?}"
            );
        }
    }
}
