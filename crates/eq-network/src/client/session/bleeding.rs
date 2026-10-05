//! The player's own report that they bled out, where the server leaves such
//! deaths to the client.
//!
//! TAKP tells a player killed by a blow that they died (`OP_Death`), but
//! not one who bleeds out while unconscious, nor one killed by a tick of
//! damage or by their own hand (`zone/attack.cpp` `GenerateDeathPackets`,
//! `CalcHPRegen` in `zone/client_mods.cpp`). The official client reports
//! such a death itself, and TAKP takes the report without checking the
//! player's HP (`zone/client_packet.cpp` `Handle_OP_Death`). The host decides
//! when to report, since only it adds up the HP equipped items give, which
//! the server's report leaves out; the session refuses a report the
//! server's own last word rules out. Once a report goes out, the player is
//! dead to every feature and to the host, as if the server had said so:
//! the session hears the death it made happen ([`World::happened`]) as it
//! hears the server's, so one feature, transfers, owns the death.
use super::{
    actions,
    feature::{Feature, Out, World},
    zoning, ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    message::Message,
    request::Request,
    world::{Capability, WorldEvent},
};

/// Reporting that the player bled out, at the HP the server type takes as
/// death.
pub(super) struct BleedingOut {
    /// The HP at or below which the server leaves the player's death to the
    /// client.
    threshold: i32,
    /// The player's current HP in the server's last report for them, which
    /// leaves out what equipped items add.
    reported: Option<i32>,
}

impl BleedingOut {
    pub(super) const fn new(threshold: i32) -> Self {
        Self {
            threshold,
            reported: None,
        }
    }

    /// Keeps the server's last word on the player's HP.
    fn note(&mut self, message: &Message, world: &World) {
        if let Message::Event(WorldEvent::HitPoints {
            spawn_id, current, ..
        }) = message
        {
            if world.is_player(*spawn_id) {
                self.reported = Some(*current);
            }
        }
    }

    /// The server's last report of the player's HP, if it allows a
    /// bleed-out, or why not. The report leaves out what equipped items add,
    /// so the player's HP is at least what it says: a report above the
    /// threshold means they live.
    fn dying(&self) -> Result<i32, &'static str> {
        match self.reported {
            None => Err("No HP report yet, so the player cannot have bled out"),
            Some(current) if current > self.threshold => {
                Err("The server's last HP report is above the death threshold")
            }
            Some(current) => Ok(current),
        }
    }
}

impl Feature for BleedingOut {
    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::BleedingOut]
    }

    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        self.note(message, world);
        Ok(())
    }

    /// Tells the host the HP the server type takes as death.
    fn admitted(&mut self, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        out.log.send(ClientEvent::World(WorldEvent::DeathThreshold(
            self.threshold,
        )))
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::BledOut { .. })
    }

    /// Reports the player's death, and makes it so for the whole session.
    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let reported = match self.dying() {
            Ok(reported) => reported,
            Err(reason) => return actions::refuse(command, reason, out.log),
        };
        let Some(spawn_id) = world.own_spawn else {
            return actions::refuse(command, "The zone never named the player's spawn", out.log);
        };
        out.request(&Request::BledOut)?;
        out.log.diagnostic(format!(
            "Death: reported that the player bled out, the server's last HP report {reported} (threshold {})",
            self.threshold
        ))?;
        // The death the server would have named: the player's corpse keeps
        // their spawn ID, as TAKP's own death packets say.
        world.happened(Message::Event(WorldEvent::Death(zoning::Death {
            spawn_id: u32::from(spawn_id),
            killer_id: 0,
            corpse_id: u32::from(spawn_id),
            bind_zone_id: 0,
            corpse_name: None,
        })));
        Ok(())
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        self.note(message, world);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, wire};
    use super::*;
    use eq_network_game::world::ItemHitPoints;
    use std::time::Instant;

    /// The server's report of this spawn's HP, without what items add.
    fn report(spawn_id: u16, current: i32) -> Message {
        Message::Event(WorldEvent::HitPoints {
            spawn_id,
            current,
            maximum: 100,
            items: ItemHitPoints::LeftOutOfCurrent,
        })
    }

    fn bled_out() -> ClientCommand {
        ClientCommand::BledOut {
            session_id: 5,
            created: Instant::now(),
        }
    }

    /// An admitted player, spawn 7.
    fn world() -> World {
        let mut world = World::new(5);
        world.player.admit(testing::player(7));
        world.own_spawn = Some(7);
        world.admitted = Some(Instant::now());
        world
    }

    /// Runs a step against the `EQMac` wire, which carries the report.
    fn eqmac<R>(step: impl FnOnce(&mut Out<'_, '_>) -> R) -> testing::Outcome<R> {
        testing::run(|out| {
            out.wire = &wire::EqMac;
            step(out)
        })
    }

    #[test]
    fn the_host_hears_the_threshold_as_the_zone_admits_the_player() {
        let mut bleeding = BleedingOut::new(-11);
        let mut world = world();
        let outcome = testing::run(|out| bleeding.admitted(&mut world, out));
        outcome.result.unwrap();
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(WorldEvent::DeathThreshold(-11))]
        ));
        assert_eq!(bleeding.capabilities(), [Capability::BleedingOut]);
        assert!(bleeding.owns(&bled_out()));
    }

    #[test]
    fn a_report_goes_out_only_when_the_server_puts_the_player_at_the_threshold() {
        let mut bleeding = BleedingOut::new(-11);
        let mut world = world();
        // No report yet, and then one above the threshold: nothing goes out.
        let outcome = eqmac(|out| bleeding.handle(&bled_out(), &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty());
        for (spawn_id, current) in [(7, -10), (8, -50)] {
            eqmac(|out| bleeding.observe(&report(spawn_id, current), &mut world, out))
                .result
                .unwrap();
        }
        let outcome = eqmac(|out| bleeding.handle(&bled_out(), &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty());
        assert!(world.take_news().is_empty());
        // At the threshold it goes out, and the player's death is news.
        eqmac(|out| bleeding.observe(&report(7, -11), &mut world, out))
            .result
            .unwrap();
        let outcome = eqmac(|out| bleeding.handle(&bled_out(), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [eq_network_game::quarm::bled_out(7).unwrap()]);
        let news = world.take_news();
        let [Message::Event(WorldEvent::Death(death))] = &news[..] else {
            panic!("{news:?}");
        };
        assert_eq!(
            (death.spawn_id, death.killer_id, death.corpse_id),
            (7, 0, 7)
        );
    }

    #[test]
    fn a_report_before_admission_counts() {
        let mut bleeding = BleedingOut::new(-11);
        let mut world = world();
        bleeding.admit(&report(7, -12), &mut world).unwrap();
        let outcome = eqmac(|out| bleeding.handle(&bled_out(), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
    }

    #[test]
    fn a_heal_after_the_report_rules_it_out() {
        let mut bleeding = BleedingOut::new(-11);
        let mut world = world();
        for current in [-20, 15] {
            eqmac(|out| bleeding.observe(&report(7, current), &mut world, out))
                .result
                .unwrap();
        }
        let outcome = eqmac(|out| bleeding.handle(&bled_out(), &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty());
        assert!(matches!(&outcome.events[..], [ClientEvent::Diagnostic(_)]));
    }
}
