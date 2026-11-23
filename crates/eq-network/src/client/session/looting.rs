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
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

/// The `EQMac` servers' pause between item requests on one corpse: TAKP's
/// and Quarm's `Corpse::loot_cooldown_timer` is 10 ms (`zone/corpse.cpp`,
/// TAKP 44afcbf and Quarm fc75fcf). `Corpse::LootCorpseItem` checks it just
/// after its echo and ends the loot when a request comes sooner; a request
/// that passes restarts it.
pub(super) const EQMAC_ITEM_PACE: Duration = Duration::from_millis(10);

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
    /// The least time between item requests on one corpse, when the server
    /// ends the loot for one sent sooner.
    item_pace: Option<Duration>,
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

    /// Never sends an item request sooner than `pace` after the last one on
    /// the same corpse; one asked sooner is refused, so it can be asked
    /// again ([`EQMAC_ITEM_PACE`]).
    pub(super) fn with_item_pace(self, pace: Duration) -> Self {
        Self {
            item_pace: Some(pace),
            ..self
        }
    }

    /// [`Feature::handle`] at a given time.
    fn handle_at(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
        now: Instant,
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
                    last_item: None,
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
                if world.inventory.ground_drop_pending() {
                    return actions::refuse(
                        command,
                        "Wait for a fresh inventory snapshot after the drop.",
                        out.log,
                    );
                }
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
                if corpse.too_soon(self.item_pace, now) {
                    return actions::refuse(
                        command,
                        "Wait a moment before taking the next item.",
                        out.log,
                    );
                }
                if submit(command, out)? {
                    // Do not send the same non-idempotent request again.
                    // An echo does not put the item in the inventory.
                    corpse.submitted.insert(place);
                    corpse.last_item = Some(now);
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
}

/// A requested corpse, then its server-granted listing. Item records are
/// views only; this controller never writes inventory or coins.
struct Corpse {
    id: u16,
    listing: Listing,
    closing: bool,
    places: BTreeSet<u16>,
    submitted: BTreeSet<u16>,
    /// When the last item request on this corpse was sent.
    last_item: Option<Instant>,
}

impl Corpse {
    /// Whether an item request now would come sooner than `pace` after the
    /// last one on this corpse.
    fn too_soon(&self, pace: Option<Duration>, now: Instant) -> bool {
        pace.zip(self.last_item)
            .is_some_and(|(pace, last)| now.saturating_duration_since(last) < pace)
    }
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
        self.handle_at(command, world, out, Instant::now())
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

    /// The `EQMac` servers end the loot for an item request sooner than 10 ms
    /// after the last on the corpse, so none is sent sooner; one refused can
    /// be asked again once the pause has passed.
    #[test]
    fn item_requests_on_one_corpse_wait_for_the_servers_pause() {
        let mut looting = Looting::default().with_item_pace(EQMAC_ITEM_PACE);
        let mut world = World::new(5);
        world.player.admit(testing::player(7));
        world.own_spawn = Some(7);
        world.spawns.insert(testing::spawn(8, SpawnKind::NpcCorpse));
        let start = Instant::now();
        let open = ClientCommand::Loot {
            session_id: 5,
            corpse_id: 8,
            created: start,
        };
        testing::run(|out| looting.handle_at(&open, &mut world, out, start))
            .result
            .unwrap();
        let corpse = looting.corpse.as_mut().unwrap();
        corpse.listing = Listing::Ready;
        corpse.places.extend([0, 1, 2]);
        let take = |place| ClientCommand::LootItem {
            session_id: 5,
            corpse_id: 8,
            own_id: 7,
            place,
            auto: true,
            created: start,
        };
        let at = |millis| start + Duration::from_millis(millis);
        for (place, millis, sent) in [(0, 0, 1), (1, 9, 0), (1, 10, 1), (2, 15, 0), (2, 20, 1)] {
            let outcome =
                testing::run(|out| looting.handle_at(&take(place), &mut world, out, at(millis)));
            outcome.result.unwrap();
            assert_eq!(outcome.sent.len(), sent, "place {place} at {millis} ms");
        }
        // Without a pace, nothing waits.
        let mut looting = Looting::default();
        testing::run(|out| looting.handle_at(&open, &mut world, out, start))
            .result
            .unwrap();
        let corpse = looting.corpse.as_mut().unwrap();
        corpse.listing = Listing::Ready;
        corpse.places.extend([0, 1]);
        for place in [0, 1] {
            let outcome =
                testing::run(|out| looting.handle_at(&take(place), &mut world, out, start));
            assert_eq!(outcome.sent.len(), 1);
        }
    }

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
