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
//!
//! TAKP's report leaves out what the player's items add, so a living
//! player in HP gear can show the threshold, and TAKP kills whoever the
//! client names. So the session takes a report only once it can count what
//! the items add, on the server type's own rule ([`ItemCount`]), and that
//! count with the server's report is at or below the threshold. Until a
//! server type has a count, the session refuses every report and names no
//! threshold, so a host asks for none.
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

/// How a server type counts the HP the player's items add, which its HP
/// report leaves out, from what the session knows of them; or why it
/// cannot. Only called once the player's items are known.
pub(super) type ItemCount = fn(&World) -> Result<i32, &'static str>;

/// Reporting that the player bled out, at the HP the server type takes as
/// death.
pub(super) struct BleedingOut {
    /// The HP at or below which the server leaves the player's death to the
    /// client.
    threshold: i32,
    /// How the server type counts the HP the player's items add; None
    /// until the session has its rule, which holds every report back.
    items: Option<ItemCount>,
    /// The player's current HP in the server's last report for them, which
    /// leaves out what equipped items add.
    reported: Option<i32>,
}

impl BleedingOut {
    pub(super) const fn new(threshold: i32, items: Option<ItemCount>) -> Self {
        Self {
            threshold,
            items,
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

    /// The player's HP, the server's last report with what their items add,
    /// if it is a bleed-out, or why not.
    fn dying(&self, world: &World) -> Result<i32, &'static str> {
        if world.lifecycle.is_dead() {
            return Err("The player is already dead");
        }
        let Some(count) = self.items else {
            return Err(
                "The session cannot count the HP the player's items add on this server yet",
            );
        };
        let Some(reported) = self.reported else {
            return Err("No HP report yet, so the player cannot have bled out");
        };
        if !world.inventory.received() || world.inventory.stale() {
            return Err("The player's items are not known, so their HP cannot be counted");
        }
        let current = reported.saturating_add(count(world)?);
        if current > self.threshold {
            return Err("The player's HP, with what their items add, is above the death threshold");
        }
        Ok(current)
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

    /// Tells the host the HP the server type takes as death, once the
    /// session can count what the player's items add there: a host asks
    /// for nothing it would refuse.
    fn admitted(&mut self, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if self.items.is_none() {
            return Ok(());
        }
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
        let current = match self.dying(world) {
            Ok(current) => current,
            Err(reason) => return actions::refuse(command, reason, out.log),
        };
        let Some(spawn_id) = world.own_spawn else {
            return actions::refuse(command, "The zone never named the player's spawn", out.log);
        };
        out.request(&Request::BledOut)?;
        out.log.diagnostic(format!(
            "Death: reported that the player bled out at {current} HP with what their items add (threshold {})",
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
    use super::super::{feature::testing, inventory::Carried, wire};
    use super::*;
    use eq_network_game::{
        command::EncodedCommand,
        inventory::{Inventory, InventoryUpdate},
        world::ItemHitPoints,
    };
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

    /// A server type's count that finds the player's items add `HP`.
    #[allow(clippy::unnecessary_wraps, reason = "the signature every count has")]
    const fn adding<const HP: i32>(_world: &World) -> Result<i32, &'static str> {
        Ok(HP)
    }

    /// An admitted player, spawn 7, whose items the session knows.
    fn world() -> World {
        let mut world = World::new(5);
        world.player.admit(testing::player(7));
        world.own_spawn = Some(7);
        world.admitted = Some(Instant::now());
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(Vec::new()));
        world.inventory = inventory.into();
        world
    }

    /// Runs a step against the `EQMac` wire, which carries the report.
    fn eqmac<R>(step: impl FnOnce(&mut Out<'_, '_>) -> R) -> testing::Outcome<R> {
        testing::run(|out| {
            out.wire = &wire::EqMac;
            step(out)
        })
    }

    /// Hears the server report this HP for this spawn.
    fn hear(bleeding: &mut BleedingOut, world: &mut World, spawn_id: u16, current: i32) {
        eqmac(|out| bleeding.observe(&report(spawn_id, current), world, out))
            .result
            .unwrap();
    }

    /// The host's report: what went out, and the session's diagnostics.
    fn ask(bleeding: &mut BleedingOut, world: &mut World) -> (Vec<EncodedCommand>, Vec<String>) {
        let outcome = eqmac(|out| bleeding.handle(&bled_out(), world, out));
        outcome.result.unwrap();
        let said = outcome
            .events
            .into_iter()
            .filter_map(|event| match event {
                ClientEvent::Diagnostic(said) => Some(said),
                _ => None,
            })
            .collect();
        (outcome.sent, said)
    }

    #[test]
    fn the_host_hears_the_threshold_as_the_zone_admits_the_player() {
        let mut bleeding = BleedingOut::new(-11, Some(adding::<0>));
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
    fn a_report_goes_out_only_at_the_threshold_with_what_the_items_add() {
        let mut bleeding = BleedingOut::new(-11, Some(adding::<25>));
        let mut world = world();
        // No report yet: nothing goes out.
        let (sent, said) = ask(&mut bleeding, &mut world);
        assert_eq!(sent, []);
        assert!(said[0].contains("No HP report"), "{said:?}");
        // The server's -35 is -10 with the items' 25; another spawn's
        // report is not the player's.
        hear(&mut bleeding, &mut world, 7, -35);
        hear(&mut bleeding, &mut world, 8, -50);
        let (sent, said) = ask(&mut bleeding, &mut world);
        assert_eq!(sent, []);
        assert!(said[0].contains("above the death threshold"), "{said:?}");
        let news = world.take_news();
        assert!(news.is_empty(), "{news:?}");
        // -36 is -11 with them: it goes out, and the player's death is news.
        hear(&mut bleeding, &mut world, 7, -36);
        let (sent, _) = ask(&mut bleeding, &mut world);
        assert_eq!(sent, [eq_network_game::quarm::bled_out(7).unwrap()]);
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
    fn no_report_goes_out_while_the_players_items_are_unknown() {
        // A living player in HP gear: the server's -11 leaves their items'
        // HP out.
        let mut bleeding = BleedingOut::new(-11, Some(adding::<0>));
        let mut world = world();
        world.inventory = Carried::default();
        hear(&mut bleeding, &mut world, 7, -11);
        let (sent, said) = ask(&mut bleeding, &mut world);
        assert_eq!(sent, []);
        assert!(said[0].contains("items are not known"), "{said:?}");
        let news = world.take_news();
        assert!(news.is_empty(), "{news:?}");
    }

    #[test]
    fn a_server_type_without_a_count_takes_no_report_and_names_no_threshold() {
        let mut bleeding = BleedingOut::new(-11, None);
        let mut world = world();
        let outcome = testing::run(|out| bleeding.admitted(&mut world, out));
        outcome.result.unwrap();
        assert!(outcome.events.is_empty(), "{:?}", outcome.events);
        // Knowing the player's items opens nothing.
        hear(&mut bleeding, &mut world, 7, -200);
        let (sent, said) = ask(&mut bleeding, &mut world);
        assert_eq!(sent, []);
        assert!(said[0].contains("cannot count"), "{said:?}");
        assert!(!world.lifecycle.is_dead());
    }

    #[test]
    fn a_report_before_admission_counts() {
        let mut bleeding = BleedingOut::new(-11, Some(adding::<0>));
        let mut world = world();
        bleeding.admit(&report(7, -12), &mut world).unwrap();
        let (sent, _) = ask(&mut bleeding, &mut world);
        assert_eq!(sent.len(), 1);
    }

    #[test]
    fn a_heal_after_the_report_rules_it_out() {
        let mut bleeding = BleedingOut::new(-11, Some(adding::<0>));
        let mut world = world();
        for current in [-20, 15] {
            hear(&mut bleeding, &mut world, 7, current);
        }
        let (sent, said) = ask(&mut bleeding, &mut world);
        assert_eq!(sent, []);
        assert_eq!(said.len(), 1);
    }

    #[test]
    fn the_dead_report_no_second_death() {
        let mut bleeding = BleedingOut::new(-11, Some(adding::<0>));
        let mut world = world();
        hear(&mut bleeding, &mut world, 7, -11);
        assert_eq!(ask(&mut bleeding, &mut world).0.len(), 1);
        // The zone loop hands the news on; transfers marks the death.
        world.take_news();
        world.died();
        let (sent, said) = ask(&mut bleeding, &mut world);
        assert_eq!(sent, []);
        assert!(said[0].contains("already dead"), "{said:?}");
    }
}
