//! Combat: sizing up a spawn the player can see, and attacking.
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    combat::{CombatRequest, CombatRequestStatus},
    request::Request,
    world::WorldEvent,
};

/// The normal combat request carried by a command, with its admission.
fn request(command: &ClientCommand) -> Option<(u64, CombatRequest)> {
    match *command {
        ClientCommand::Consider {
            session_id,
            target_id,
            ..
        } => Some((session_id, CombatRequest::Consider { target_id })),
        ClientCommand::AutoAttack {
            session_id,
            enabled,
            ..
        } => Some((session_id, CombatRequest::AutoAttack { enabled })),
        _ => None,
    }
}

/// Gives shared admission, freshness and policy refusals the same typed result
/// as the combat owner's own guards, without claiming a server response.
pub(super) fn rejected(command: &ClientCommand, reason: &str) -> Option<WorldEvent> {
    let (session_id, request) = request(command)?;
    Some(WorldEvent::CombatRequest {
        session_id,
        request,
        status: CombatRequestStatus::Refused(reason.into()),
    })
}

/// Considers spawns and turns auto attack on and off.
pub(super) struct Combat {
    /// Whether the client generation sends a notice before each change of
    /// the toggle, as the `EQMac` program does.
    pub(super) attack_notice: bool,
}

impl Feature for Combat {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        vec![Capability::Combat]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::Consider { .. } | ClientCommand::AutoAttack { .. }
        )
    }

    /// Considers a visible spawn, including the player. An attack toggle does
    /// not require a target or melee range: the server checks those on each
    /// swing, not when it receives the toggle. No result is predicted here.
    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let Some((session_id, request)) = request(command) else {
            return Ok(());
        };
        let reason = if !world.ready() || world.own_spawn.is_none() || world.player.is_none() {
            Some("The player has not been admitted yet")
        } else if world.lifecycle.is_dead() {
            Some("The player is dead")
        } else if world.lifecycle.blocks_motion() {
            Some("The player is between zones")
        } else {
            None
        };
        if let Some(reason) = reason {
            return super::actions::refuse(command, reason, out.log);
        }
        if let ClientCommand::Consider {
            own_id, target_id, ..
        } = *command
        {
            if !world.is_player(own_id) || !world.visible(target_id) {
                return super::actions::refuse(
                    command,
                    "The considered spawn or player is unavailable",
                    out.log,
                );
            }
        }
        if let ClientCommand::AutoAttack { enabled, .. } = *command {
            if self.attack_notice {
                out.request(&Request::AttackNotice { enabled })?;
            }
        }
        if out.command(command)? {
            out.log.send(ClientEvent::World(WorldEvent::CombatRequest {
                session_id,
                request,
                status: CombatRequestStatus::Sent,
            }))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::world::SpawnKind;
    use std::time::Instant;

    #[test]
    fn the_eqmac_program_sends_its_notice_before_the_toggle() {
        let mut combat = Combat {
            attack_notice: true,
        };
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        world.admitted = Some(Instant::now());
        let attack = ClientCommand::AutoAttack {
            session_id: 5,
            enabled: true,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| {
            out.wire = &super::super::wire::EqMac;
            combat.handle(&attack, &mut world, out)
        });
        outcome.result.unwrap();
        let opcodes: Vec<_> = outcome.sent.iter().map(|packet| packet.opcode).collect();
        assert_eq!(
            opcodes,
            [
                eq_network_game::combat::EQMAC_AUTO_ATTACK2_OPCODE,
                eq_network_game::combat::EQMAC_AUTO_ATTACK_OPCODE
            ]
        );
        assert!(outcome
            .sent
            .iter()
            .all(|packet| packet.body == [1, 0, 0, 0]));
    }

    #[test]
    fn the_player_considers_only_a_spawn_they_can_see() {
        let mut combat = Combat {
            attack_notice: false,
        };
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        world.admitted = Some(Instant::now());
        world.spawns.insert(testing::spawn(8, SpawnKind::Npc));
        let consider = |own_id, target_id| ClientCommand::Consider {
            session_id: 5,
            own_id,
            target_id,
            created: Instant::now(),
        };
        for (command, sent) in [
            (consider(7, 8), 1),
            (consider(7, 7), 1),
            (consider(7, 9), 0),
            (consider(6, 8), 0),
        ] {
            let outcome = testing::run(|out| combat.handle(&command, &mut world, out));
            outcome.result.unwrap();
            assert_eq!(outcome.sent.len(), sent, "{command:?}");
        }
        let attack = ClientCommand::AutoAttack {
            session_id: 5,
            enabled: true,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| combat.handle(&attack, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
    }
}
