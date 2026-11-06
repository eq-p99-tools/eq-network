//! Combat: sizing up a spawn the player can see, and attacking.
mod ranged;

use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent, ServerProtocol,
};
use anyhow::Result;
use eq_network_game::{
    combat::{CombatRequest, CombatRequestStatus},
    message::Message,
    request::Request,
    world::WorldEvent,
};
use std::time::Instant;

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
        ClientCommand::RangedAttack(attack) => Some((
            attack.session_id,
            CombatRequest::RangedAttack {
                target_id: attack.target.spawn_id,
                kind: attack.loadout.kind(),
            },
        )),
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

/// Considers spawns, toggles melee auto attack and handles manual ranged shots.
pub(super) struct Combat {
    /// Whether the client generation sends a notice before each change of
    /// the toggle, as the `EQMac` program does.
    attack_notice: bool,
    shots: Option<ranged::Shots>,
}

impl Combat {
    pub(super) fn new(protocol: ServerProtocol) -> Self {
        use ranged::SkillAdmission;
        Self {
            attack_notice: !protocol.is_titanium(),
            shots: match protocol {
                ServerProtocol::EqEmu => Some(ranged::Shots::new(SkillAdmission::ServerCap)),
                ServerProtocol::Project1999 => {
                    Some(ranged::Shots::new(SkillAdmission::ReportedRank))
                }
                ServerProtocol::Quarm | ServerProtocol::Takp => None,
            },
        }
    }
}

impl Feature for Combat {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        let mut capabilities = vec![Capability::Combat];
        capabilities.extend(self.shots.as_ref().map(|_| Capability::RangedCombat));
        capabilities
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::Consider { .. }
                | ClientCommand::AutoAttack { .. }
                | ClientCommand::RangedAttack(_)
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
        let sent = if let ClientCommand::RangedAttack(attack) = *command {
            let Some(shots) = &mut self.shots else {
                return super::actions::refuse(command, "Ranged attacks are unavailable", out.log);
            };
            shots.send(attack, command, world, out, Instant::now())?
        } else {
            out.command(command)?
        };
        if sent {
            out.log.send(ClientEvent::World(WorldEvent::CombatRequest {
                session_id,
                request,
                status: CombatRequestStatus::Sent,
            }))?;
        }
        Ok(())
    }

    fn observe(&mut self, message: &Message, world: &mut World, _: &mut Out<'_, '_>) -> Result<()> {
        let (Some(shots), Message::Event(event)) = (&mut self.shots, message) else {
            return Ok(());
        };
        if world.owns_death(event) {
            shots.reset();
        }
        Ok(())
    }
}

#[cfg(test)]
/// A loaded synthetic weapon and its projectile stack for zone tests.
pub(super) fn test_ranged_inventory(
    kind: eq_network_game::combat::ranged::RangedKind,
) -> eq_network_game::inventory::Inventory {
    use eq_network_game::{
        combat::ranged::RangedKind,
        inventory::{Inventory, InventoryUpdate},
        items::ItemStat,
    };

    let mut weapon = super::feature::testing::item(11);
    weapon.rules.item_type = match kind {
        RangedKind::Archery => 5,
        RangedKind::Throwing => 19,
    };
    weapon.details.stats = vec![
        ItemStat {
            label: "Range".into(),
            value: 80,
        },
        ItemStat {
            label: "Delay".into(),
            value: 25,
        },
    ];
    let mut items = vec![weapon];
    let projectile = match kind {
        RangedKind::Archery => {
            let mut arrow = super::feature::testing::item(21);
            arrow.rules.item_type = 27;
            arrow.details.stats.push(ItemStat {
                label: "Range".into(),
                value: 25,
            });
            items.push(arrow);
            &mut items[1]
        }
        RangedKind::Throwing => &mut items[0],
    };
    projectile.stack_count = Some(2);
    let mut inventory = Inventory::default();
    inventory.apply(InventoryUpdate::Snapshot(items));
    inventory
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::world::SpawnKind;
    use std::time::Instant;

    #[test]
    fn a_local_death_clears_the_ranged_wait() {
        use eq_network_game::combat::ranged::{RangedAttack, RangedKind, RangedLoadout};

        let mut combat = Combat::new(ServerProtocol::EqEmu);
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        player.skills.as_mut().unwrap()[7] = 15;
        world.player.admit(player);
        world.admitted = Some(Instant::now());
        let mut target = testing::spawn(8, SpawnKind::Npc);
        target.position.x = 60.0;
        world.spawns.insert(target);
        world.target = 8.into();
        world.inventory = test_ranged_inventory(RangedKind::Archery).into();
        let attack = ClientCommand::RangedAttack(RangedAttack {
            session_id: 5,
            target: world.target.state(),
            loadout: RangedLoadout::current(&world.inventory).unwrap(),
            created: Instant::now(),
        });
        for expected in [1, 0] {
            let result = testing::run(|out| combat.handle(&attack, &mut world, out));
            result.result.unwrap();
            assert_eq!(result.sent.len(), expected);
        }
        let result = testing::run(|out| {
            // The lifecycle and target owners have their own reset tests.
            combat.observe(
                &Message::Event(WorldEvent::DeathBind { session_id: 5 }),
                &mut world,
                out,
            )?;
            combat.handle(&attack, &mut world, out)
        });
        result.result.unwrap();
        assert_eq!(result.sent.len(), 1);
    }

    #[test]
    fn the_eqmac_program_sends_its_notice_before_the_toggle() {
        let mut combat = Combat::new(ServerProtocol::Takp);
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
        let mut combat = Combat::new(ServerProtocol::Project1999);
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
