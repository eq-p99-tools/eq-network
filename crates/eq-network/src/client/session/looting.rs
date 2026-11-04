//! Looting: opening a corpse the player can see, taking its items and
//! closing it. Taken items arrive as item updates for the inventory.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    loot::{LootRequest, LootRequestStatus, LootResponse, LootUpdate},
    message::Message,
    world::{SpawnKind, WorldEvent},
};
use std::collections::BTreeSet;

/// Retains original host context even when shared validation rejects a command.
fn disposition(command: &ClientCommand, status: LootRequestStatus) -> Option<WorldEvent> {
    let (session_id, request) = match *command {
        ClientCommand::Loot {
            session_id,
            corpse_id,
            ..
        } => (session_id, LootRequest::Open { corpse_id }),
        ClientCommand::LootItem {
            session_id,
            corpse_id,
            own_id,
            place,
            auto,
            ..
        } => (
            session_id,
            LootRequest::Item {
                corpse_id,
                own_id,
                place,
                auto,
            },
        ),
        ClientCommand::EndLoot {
            session_id,
            corpse_id,
        } => (session_id, LootRequest::Close { corpse_id }),
        _ => return None,
    };
    Some(WorldEvent::LootRequest {
        session_id,
        created: command.created(),
        request,
        status,
    })
}

/// Shared and feature-local refusals use one typed event plus the existing diagnostic.
pub(super) fn rejected(command: &ClientCommand, reason: &str) -> Option<WorldEvent> {
    disposition(command, LootRequestStatus::Refused(reason.into()))
}

/// Encoding refusal is local; transport failure is not falsely reported as unsent.
fn submit(command: &ClientCommand, out: &mut Out<'_, '_>) -> Result<bool> {
    let packet = match out.encode_command(command) {
        Ok(packet) => packet,
        Err(error) => {
            actions::refuse(
                command,
                &format!("Rejected invalid outbound client command: {error}"),
                out.log,
            )?;
            return Ok(false);
        }
    };
    out.send(&packet)?;
    if let Some(event) = disposition(command, LootRequestStatus::Sent) {
        out.log.send(ClientEvent::World(event))?;
    }
    Ok(true)
}

/// Loots corpses.
#[derive(Default)]
pub(super) struct Looting {
    /// This admission's corpse request, including a close awaiting the server.
    corpse: Option<Corpse>,
    /// Admission owning the request; never carry it across a reconnect.
    session: Option<u64>,
    /// A server policy whose normal corpse rules require an empty item cursor.
    require_empty_cursor: bool,
}

impl Looting {
    /// Quarm's `CheckCursorEmptyWhenLooting` defaults true. Use the existing
    /// inventory projection, without treating a prediction as a move hold.
    pub(super) fn requiring_empty_cursor() -> Self {
        Self {
            require_empty_cursor: true,
            ..Self::default()
        }
    }
}

/// A requested corpse, then its server-granted listing. Item records are
/// views only; this controller never writes inventory or coins.
struct Corpse {
    id: u16,
    listing: Listing,
    closing: bool,
    places: BTreeSet<u16>,
    submitted: BTreeSet<u16>,
}

/// A grant precedes all item views; the matching open echo ends the list.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Listing {
    Requested,
    Receiving,
    Ready,
}

impl Feature for Looting {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        use crate::world::Capability;
        vec![Capability::Looting]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::Loot { .. }
                | ClientCommand::LootItem { .. }
                | ClientCommand::EndLoot { .. }
        )
    }

    /// Opens only a corpse the player can see; the server judges the rest.
    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if command.session_id() != Some(world.session_id) {
            return actions::refuse(
                command,
                "That loot command belongs to an earlier session.",
                out.log,
            );
        }
        if self.session != Some(world.session_id) {
            self.corpse = None;
            self.session = Some(world.session_id);
        }
        if let ClientCommand::Loot { corpse_id, .. } = *command {
            if self.corpse.is_some() {
                return actions::refuse(command, "Close the current corpse first.", out.log);
            }
            let corpse = world.spawns.visible(corpse_id).is_some_and(|spawn| {
                matches!(spawn.kind, SpawnKind::NpcCorpse | SpawnKind::PlayerCorpse)
            });
            if !corpse {
                return actions::refuse(command, "Rejected looting an unavailable corpse", out.log);
            }
            if submit(command, out)? {
                self.corpse = Some(Corpse {
                    id: corpse_id,
                    listing: Listing::Requested,
                    closing: false,
                    places: BTreeSet::new(),
                    submitted: BTreeSet::new(),
                });
            }
            return Ok(());
        }
        let Some(corpse) = self.corpse.as_mut() else {
            return actions::refuse(command, "No corpse is open.", out.log);
        };
        match *command {
            ClientCommand::LootItem {
                corpse_id,
                own_id,
                place,
                ..
            } => {
                if self.require_empty_cursor
                    && (!world.inventory.received()
                        || world.inventory.stale()
                        || world
                            .inventory
                            .items()
                            .contains_key(&eq_network_game::inventory::InventorySlot::CURSOR))
                {
                    return actions::refuse(
                        command,
                        "Load your inventory and clear the item cursor before looting.",
                        out.log,
                    );
                }
                if corpse.id != corpse_id
                    || !world.is_player(own_id)
                    || corpse.listing != Listing::Ready
                    || corpse.closing
                    || !corpse.places.contains(&place)
                    || corpse.submitted.contains(&place)
                {
                    return actions::refuse(
                        command,
                        "That item is not available on the open corpse.",
                        out.log,
                    );
                }
                if submit(command, out)? {
                    // Do not send the same non-idempotent request again.
                    // An echo does not put the item in the inventory.
                    corpse.submitted.insert(place);
                }
            }
            ClientCommand::EndLoot { corpse_id, .. } => {
                if corpse.id != corpse_id || corpse.closing {
                    return actions::refuse(command, "That corpse is not open.", out.log);
                }
                if submit(command, out)? {
                    corpse.closing = true;
                }
            }
            _ => (),
        }
        Ok(())
    }

    /// Correlates loot-only messages before the inventory owner hears coins.
    /// A close request retains correlation until the server closes: coins
    /// already transferred still count, and late item deliveries still pass
    /// through the ordinary inventory owner.
    fn explain(&mut self, message: &mut Message, world: &World) {
        let Message::Event(WorldEvent::Loot(update)) = message else {
            return;
        };
        if self.session != Some(world.session_id) {
            self.corpse = None;
        }
        let Some(corpse) = &mut self.corpse else {
            *message = Message::Withheld;
            return;
        };
        match update {
            LootUpdate::Opened {
                response: LootResponse::Normal,
                ..
            } if corpse.listing == Listing::Requested => {
                corpse.listing = Listing::Receiving;
            }
            LootUpdate::Opened { response, .. } if *response != LootResponse::Normal => {
                self.corpse = None;
            }
            LootUpdate::Item { place, .. } if corpse.listing == Listing::Receiving => {
                corpse.places.insert(*place);
            }
            LootUpdate::Listed { corpse_id }
                if *corpse_id == corpse.id && corpse.listing == Listing::Receiving =>
            {
                corpse.listing = Listing::Ready;
            }
            LootUpdate::Taken { place, accepted } if corpse.submitted.contains(place) => {
                if *accepted {
                    corpse.places.remove(place);
                } else {
                    // An explicit Titanium refusal permits a later manual
                    // attempt. EQMac's receipt echo makes no such claim.
                    corpse.submitted.remove(place);
                }
            }
            LootUpdate::Acknowledged { place } if corpse.submitted.contains(place) => (),
            LootUpdate::Closed => self.corpse = None,
            _ => *message = Message::Withheld,
        }
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let reset = match message {
            Message::Event(event) if world.owns_death(event) => true,
            Message::Event(WorldEvent::ZoneTransfer(_)) | Message::LoggedOut => true,
            Message::Event(WorldEvent::Despawn(id)) => {
                self.corpse.as_ref().is_some_and(|corpse| corpse.id == *id)
            }
            _ => false,
        };
        if reset && self.corpse.take().is_some() {
            out.log
                .send(ClientEvent::World(WorldEvent::Loot(LootUpdate::Closed)))?;
        }
        Ok(())
    }

    fn connection_ended(&mut self, _world: &mut World) {
        self.corpse = None;
        self.session = None;
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use std::time::Instant;

    #[test]
    fn only_a_corpse_the_player_can_see_is_opened() {
        let mut looting = Looting::default();
        let mut world = World::new(5);
        world.spawns.insert(testing::spawn(8, SpawnKind::NpcCorpse));
        world.spawns.insert(testing::spawn(9, SpawnKind::Npc));
        let loot = |corpse_id| ClientCommand::Loot {
            session_id: 5,
            corpse_id,
            created: Instant::now(),
        };
        for (corpse_id, sent) in [(8, 1), (9, 0), (10, 0)] {
            let outcome = testing::run(|out| looting.handle(&loot(corpse_id), &mut world, out));
            outcome.result.unwrap();
            assert_eq!(outcome.sent.len(), sent, "corpse {corpse_id}");
        }
        let close = ClientCommand::EndLoot {
            session_id: 5,
            corpse_id: 8,
        };
        let outcome = testing::run(|out| looting.handle(&close, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
    }
}
