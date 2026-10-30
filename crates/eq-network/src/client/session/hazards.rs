//! Damage the world does to the player. The client works it out, as the
//! official client does (a fall's after the player's Safe Fall skill, which
//! neither server applies; inferred), and reports it; the server takes the
//! amount as it is, lowers it by its own reductions (`EQEmu`'s fall damage
//! reductions from spells, items and AAs; TAKP's Acrobatics AA), and kills
//! a player it brings to nothing (`Client::Handle_OP_EnvDamage` in
//! `EQEmu`, `Handle_OP_Damage` in `EQMacEmu`). The session reports only
//! what the host tells it, and only the hazards its server type takes from
//! the client.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;
use eq_network_game::{hazards::Hazard, request::Request};

/// Reports the world's damage to the server.
pub(super) struct Hazards {
    /// The hazards the server type takes from the client.
    taken: &'static [Hazard],
}

impl Hazards {
    /// Reports these hazards, and refuses the rest.
    pub(super) fn new(taken: &'static [Hazard]) -> Self {
        Self { taken }
    }
}

impl Feature for Hazards {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::EnvironmentalDamage]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::EnvironmentalDamage { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::EnvironmentalDamage { hazard, amount, .. } = *command else {
            return Ok(());
        };
        if !self.taken.contains(&hazard) {
            return actions::refuse(
                command,
                &format!("This server does not take {hazard:?} damage from the client"),
                out.log,
            );
        }
        match out.encode(&Request::EnvironmentalDamage { hazard, amount }) {
            Ok(packet) => {
                out.send(&packet)?;
                out.log
                    .diagnostic(format!("Reported {amount} points of {hazard:?} damage"))
            }
            // The host works the amount out; one the packet cannot carry is
            // not worth the session.
            Err(error) => out.log.diagnostic(format!("Damage not reported: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, ClientEvent};
    use super::*;
    use eq_network_game::hazards;

    fn report(hazard: Hazard, amount: u32) -> ClientCommand {
        ClientCommand::EnvironmentalDamage {
            session_id: 5,
            hazard,
            amount,
        }
    }

    #[test]
    fn a_hazard_the_server_takes_is_reported_as_the_host_worked_it_out() {
        let mut feature = Hazards::new(&[Hazard::Falling]);
        let mut world = World::new(5);
        let outcome =
            testing::run(|out| feature.handle(&report(Hazard::Falling, 160), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [hazards::titanium_damage(7, Hazard::Falling, 160).unwrap()]
        );
        assert_eq!(outcome.unreliable, []);
        assert!(matches!(
            &outcome.events[..],
            [ClientEvent::Diagnostic(line)] if line.contains("160")
        ));
    }

    #[test]
    fn other_hazards_and_amounts_the_packet_cannot_carry_are_only_noted() {
        let mut feature = Hazards::new(&[Hazard::Falling]);
        let mut world = World::new(5);
        for command in [
            // Not taken from the client on this server type.
            report(Hazard::Drowning, 20),
            // Nothing to report.
            report(Hazard::Falling, 0),
            // More than the servers read as it is.
            report(Hazard::Falling, u32::MAX),
        ] {
            let outcome = testing::run(|out| feature.handle(&command, &mut world, out));
            outcome.result.unwrap();
            assert!(outcome.sent.is_empty(), "{command:?}");
            assert!(
                matches!(&outcome.events[..], [ClientEvent::Diagnostic(_)]),
                "{command:?}"
            );
        }
    }
}
