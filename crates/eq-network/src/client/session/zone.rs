//! The zone session: admission, the player's commands and the zone's traffic
//! until the character leaves for another zone, the world or the character list.
use super::{
    actions, ensure,
    feature::{Feature, Out, World},
    servers, CharacterSession, ClientCommand, ClientEvent, ConnectionStage, ConnectionState,
    DecodeError, Duration, Events, Instant, RecordEvent, Result, Shield, ZoneExit,
};

use super::admission::{Handshake, HandshakeComplete, Zone};
use super::coverage::{Stage, Unheard};
use crate::client::PlayerAdmissionError;
use eq_network_game::{
    message::{Message, Part},
    world::{Capability, WorldEvent},
};
use eq_network_transport::Transport;

/// The zone's features, each offered every command, packet, timer and
/// event, and what the server type leaves to the player of what they let
/// the player do.
struct Features {
    provided: Vec<Box<dyn Feature>>,
    choices: Vec<crate::world::Capability>,
    /// The spell action flag that says a spell landed, as the server type
    /// says.
    spell_landing_flag: Option<u8>,
    answers_list_polls: bool,
    /// Whether the server type offers the explicit gameplay departures
    /// (movement and memorization options). Every server type offers them
    /// while the owner tests them; public parity needs them withheld on
    /// P99, Quarm and TAKP again before a release.
    gameplay_departures: bool,
}

impl Features {
    /// The features the server type provides.
    #[cfg(test)]
    fn new(
        server: &dyn servers::ServerType,
        name: &str,
        auto_eat: eq_network_game::food::AutoEat,
    ) -> Self {
        Self::from_setup(server, &servers::Setup::new(name, auto_eat))
    }

    fn from_setup(server: &dyn servers::ServerType, setup: &servers::Setup<'_>) -> Self {
        let mut features = Vec::new();
        let (mut offered, mut choices) = (Vec::new(), Vec::new());
        for provided in server.features(setup) {
            let listed = provided.feature.capabilities();
            if provided.choice {
                choices.extend(listed);
            } else {
                offered.extend(listed);
            }
            features.push(provided.feature);
        }
        // What one feature offers is offered, whatever another leaves to the
        // player.
        choices.retain(|capability| !offered.contains(capability));
        choices.sort_unstable();
        choices.dedup();
        Self {
            provided: features,
            choices,
            spell_landing_flag: server.spell_landing_flag(),
            answers_list_polls: server.answers_list_polls(),
            gameplay_departures: server.offers_gameplay_departures(),
        }
    }

    /// What the features let the player do, each once, but what the server
    /// type leaves to the player.
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        let mut capabilities: Vec<_> = self
            .provided
            .iter()
            .flat_map(|feature| feature.capabilities())
            .filter(|capability| !self.choices.contains(capability))
            .collect();
        if self.answers_list_polls {
            capabilities.push(crate::world::Capability::AnsweringListPolls);
        }
        if self.gameplay_departures {
            capabilities.push(crate::world::Capability::GameplayDepartures);
        }
        capabilities.sort_unstable();
        capabilities.dedup();
        capabilities
    }

    /// What the server type leaves to the player, each once.
    fn choices(&self) -> Vec<crate::world::Capability> {
        self.choices.clone()
    }

    /// Lets every feature shape the player the admission reports.
    fn shape(&mut self, player: &mut crate::world::PlayerState) {
        for feature in &mut self.provided {
            feature.shape(player);
        }
    }

    /// Lets every feature explain a message its own action caused.
    fn explain(&mut self, message: &mut Message, world: &World) {
        for feature in &mut self.provided {
            feature.explain(message, world);
        }
    }

    /// Lets every feature record a message from before the zone admitted the
    /// player.
    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        for feature in &mut self.provided {
            feature.admit(message, world)?;
        }
        Ok(())
    }

    /// Has every feature tell the host what it staged before admission.
    fn admitted(&mut self, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        for feature in &mut self.provided {
            feature.admitted(world, out)?;
        }
        Ok(())
    }

    /// What every feature's actions in flight hold.
    fn holds(&self, world: &World, now: Instant) -> Vec<(actions::Resource, &'static str)> {
        self.provided
            .iter()
            .flat_map(|feature| feature.holds(world, now))
            .collect()
    }

    /// The admitted library worker's action gate, independent of host UI state.
    fn refusal(
        &self,
        command: &ClientCommand,
        world: &World,
        now: Instant,
    ) -> Option<&'static str> {
        if !world.ready() || world.lifecycle.blocks_motion() {
            return Some(hold_reason(world));
        }
        if !self.gameplay_departures && Self::is_departure(command) {
            return Some("Gameplay departures are not offered on this server");
        }
        actions::stale(command, world.session_id, now)
            .or_else(|| actions::Held::new(self.holds(world, now)).conflict(command))
    }

    /// Whether a command chooses an explicit departure from the official
    /// client, which only a server type that offers them takes.
    const fn is_departure(command: &ClientCommand) -> bool {
        matches!(
            command,
            ClientCommand::SetMovementOptions { .. } | ClientCommand::SetBookOptions { .. }
        )
    }

    /// Lets every feature hear a command, then has its owner carry it out;
    /// true when a feature owns it.
    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<bool> {
        if out.sink.termination().is_some() {
            actions::refuse(command, "The zone connection ended", out.log)?;
            return Ok(true);
        }
        for feature in &mut self.provided {
            feature.notice(command, world, out)?;
        }
        let Some(owner) = self
            .provided
            .iter_mut()
            .find(|feature| feature.owns(command))
        else {
            return Ok(false);
        };
        owner.handle(command, world, out)?;
        Ok(true)
    }

    /// Runs the features' timers until one ends the session.
    fn tick(&mut self, now: Instant, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if out.sink.termination().is_some() {
            return Ok(());
        }
        for feature in &mut self.provided {
            feature.tick(now, world, out)?;
            if world.ending() {
                break;
            }
        }
        Ok(())
    }

    /// Lets every feature hear that the connection ended, until one takes
    /// it as the session's end.
    fn connection_ended(&mut self, world: &mut World) {
        for feature in &mut self.provided {
            feature.connection_ended(world);
            if world.ending() {
                break;
            }
        }
    }

    /// Lets every feature hear a message once the zone has admitted the
    /// player, until one ends the session.
    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        for feature in &mut self.provided {
            if let Err(error) = feature.observe(message, world, out) {
                // A response to completed incoming news may be refused by the
                // already ended transport. It was not submitted. Keep the
                // remaining observers/prefix readable, then receive reports
                // that same latched failure. Other errors still fail here.
                let refused = error
                    .downcast_ref::<eq_network_transport::modern::SessionTermination>()
                    .copied();
                if refused.is_none() || refused != out.sink.termination() {
                    return Err(error);
                }
                out.log
                    .diagnostic(format!("Response not submitted: {error}"))?;
            }
            if world.ending() {
                break;
            }
        }
        Ok(())
    }
}

/// Enter the zone, keep the character stationary, and collect communications.
pub(super) fn run(
    context: &CharacterSession<'_>,
    shield: &mut Option<Box<dyn Shield>>,
    host: &str,
    port: u16,
    checksums: Vec<u8>,
    log: &mut Events<'_>,
) -> Result<ZoneExit> {
    let config = &context.config;
    let server = servers::server_type(config.protocol);
    let mut session = server.wire().connect_zone(
        crate::client::endpoint(host, port, config.local_only)?,
        context.stop.flag(),
    )?;
    run_connected(context, server, &mut session, shield, checksums, log)
}

// The linear handshake keeps packet ordering and state transitions together.
#[allow(clippy::too_many_lines)]
fn run_connected(
    context: &CharacterSession<'_>,
    server: &dyn servers::ServerType,
    session: &mut impl Transport,
    shield: &mut Option<Box<dyn Shield>>,
    mut checksums: Vec<u8>,
    log: &mut Events<'_>,
) -> Result<ZoneExit> {
    let config = &context.config;
    let credentials = context.credentials;
    let stop = context.stop;
    let duration = context.duration;
    log.liquid_inputs = super::hazards::Observations::default();
    let mut admission = server.wire().admit(
        &config.character,
        server.profile_turn(),
        &mut Handshake {
            session,
            shield,
            key: &credentials.key,
            checksums: &mut checksums,
            log: &mut *log,
        },
    )?;
    let connected = Instant::now();
    let mut progress = Instant::now();
    let session_id = rand::random();
    let mut world = World::for_zone(session_id, config);
    let mut recording_only = false;
    let mut reported_recording_offer = false;
    let mut entry_position_superseded = false;
    let mut unheard = Unheard::new(Stage::Zone);
    let mut list_replies = super::list_poll::Replies::default();
    let mut setup = servers::Setup::new(&config.character, config.auto_eat);
    setup.local_only = world.local_eqemu();
    let mut features = Features::from_setup(server, &setup);
    let offered = features.capabilities();
    let follows_zones = offered.contains(&Capability::Zoning);
    let repeats_position = offered.contains(&Capability::Moving);
    loop {
        if stop.is_cancelled() || duration.is_some_and(|limit| connected.elapsed() >= limit) {
            session.close()?;
            // The outer run reports Stopped after the session has closed.
            return Ok(ZoneExit::Stopped);
        }
        if progress.elapsed() >= Duration::from_secs(30) {
            log.status(
                if (world.ready() || recording_only)
                    && world.lifecycle.pending().is_none()
                    && session.last_received_seconds() < 60
                {
                    ConnectionState::Connected
                } else {
                    ConnectionState::Zoning
                },
                world.packets,
                Some(session.last_received_seconds()),
            )?;
            log.diagnostic(format!(
                "Zone session: {} application packets, {} communication records",
                world.packets, log.messages
            ))?;
            progress = Instant::now();
        }
        ensure!(
            world.ready() || recording_only || connected.elapsed() < Duration::from_secs(60),
            "zone admission timed out while {}",
            admission.stage()
        );
        let speaker = sender(&config.character, &world);
        {
            let mut out = Out {
                sink: session,
                log: &mut *log,
                wire: server.wire(),
                sender: speaker,
            };
            if recording_only {
                // Preserve the listener's stationary heartbeat from wire facts.
                if repeats_position
                    && !entry_position_superseded
                    && out.sink.termination().is_none()
                {
                    world.body.heartbeat(Instant::now(), &mut out)?;
                }
            } else {
                features.tick(Instant::now(), &mut world, &mut out)?;
                if let Some(exit) = settle(&mut features, &mut world, &mut out, follows_zones)? {
                    session.close()?;
                    return Ok(exit);
                }
            }
        }
        let mut commands = Vec::new();
        if world.ready() || world.lifecycle.blocks_motion() {
            if let Some(queue) = context.commands {
                for command in queue.try_iter().take(64) {
                    if !list_replies.configure(
                        server,
                        &command,
                        session_id,
                        &context.answer_list_polls,
                        log,
                    )? {
                        commands.push(command);
                    }
                }
            }
        }
        // Apply opt-outs before any due send. Death and transfers invalidate
        // a queued reply; a fresh admission never inherits one.
        let may_reply = context.answer_list_polls.get()
            && world.ready()
            && !world.lifecycle.blocks_motion()
            && session.termination().is_none();
        list_replies.tick(Instant::now(), may_reply, session, log)?;
        let exit = apply_commands(
            &commands,
            &mut features,
            &mut world,
            &mut Out {
                sink: session,
                log: &mut *log,
                wire: server.wire(),
                sender: speaker,
            },
            follows_zones,
        )?;
        if let Some(exit) = exit {
            session.close()?;
            return Ok(exit);
        }
        let received = match session.receive() {
            Ok(received) => received,
            Err(error) => {
                // A feature may expect the zone to close the connection.
                if !recording_only {
                    features.connection_ended(&mut world);
                }
                return match world.take_exit() {
                    Some(exit) => Ok(exit),
                    None => Err(error),
                };
            }
        };
        let Some(mut packet) = received else {
            continue;
        };
        world.packets += 1;
        if let Some(shield) = shield.as_ref() {
            shield.spawns(packet.opcode, &mut packet.body, &credentials.key)?;
        }
        if super::spell_tables::receive(
            server,
            eq_network_game::spell_tables::Stage::Zone { session_id },
            packet.opcode,
            &packet.body,
            log,
        )? {
            continue;
        }
        if list_replies.observe(
            server,
            may_reply,
            packet.opcode,
            &packet.body,
            Instant::now(),
            log,
        )? {
            continue;
        }
        // What the generation's client answers by itself, at any time.
        if let Some(reply) = server
            .wire()
            .answer(packet.opcode, &packet.body)
            .filter(|_| session.termination().is_none())
        {
            session.send(reply.opcode, &reply.body)?;
        }
        if !world.ready() && !recording_only {
            log.diagnostic(format!(
                "Zone received 0x{:04x} ({} bytes)",
                packet.opcode,
                packet.body.len()
            ))?;
        }
        // What the zone leaves unread still leaves a trace, once a session.
        unheard.tell(server.wire(), packet.opcode, packet.body.len(), log)?;
        if let Some(HandshakeComplete { player, zone }) = admission.read(
            &mut packet,
            &mut world,
            &mut Handshake {
                session,
                shield,
                key: &credentials.key,
                checksums: &mut checksums,
                log: &mut *log,
            },
        )? {
            let player = match player {
                Ok(player) => player,
                Err(error) => {
                    log.diagnostic(format!("Player presentation unavailable: {error}"))?;
                    if context.commands.is_some() {
                        // Keep the admission error even if closing the wire fails.
                        let _ = session.close();
                        return Err(PlayerAdmissionError.into());
                    }
                    recording_only = true;
                    log.send(ClientEvent::Progress(ConnectionStage::Recording))?;
                    log.status(
                        ConnectionState::Connected,
                        world.packets,
                        Some(session.last_received_seconds()),
                    )?;
                    continue;
                }
            };
            let speaker = sender(&config.character, &world);
            let exit = {
                let mut out = Out {
                    sink: session,
                    log: &mut *log,
                    wire: server.wire(),
                    sender: speaker,
                };
                admit(player, &zone, &mut features, &mut world, &mut out)?;
                settle(&mut features, &mut world, &mut out, follows_zones)?
            };
            if let Some(exit) = exit {
                session.close()?;
                return Ok(exit);
            }
        }
        // Everything else is read once, in the server's client generation, the
        // same way before and after admission, and heard by every feature;
        // a kind no server is known to send leaves a trace, once a session.
        let messages = unheard.sift(
            server.wire(),
            packet.opcode,
            read_messages(
                server.wire(),
                packet.opcode,
                &packet.body,
                server.item_rules(),
            ),
            log,
        )?;
        if context.commands.is_none() && !world.ready() && !entry_position_superseded {
            // A later handshake packet cannot restore a superseded fallback.
            entry_position_superseded = messages
                .iter()
                .any(|message| supersedes_entry_position(message, &world));
        }
        let speaker = sender(&config.character, &world);
        if recording_only {
            // Keep the communication stream without giving the player
            // features a live world or ignoring an explicit logout.
            if !reported_recording_offer {
                let offer = messages.iter().find_map(|message| match message {
                    Message::ZoneOffer(_) => {
                        Some("Zone offer received; record-only mode cannot follow it")
                    }
                    Message::Unreadable {
                        part: Part::ZoneOffer,
                        ..
                    } => Some("Unreadable zone offer received; record-only mode cannot follow it"),
                    _ => None,
                });
                if let Some(offer) = offer {
                    log.diagnostic(offer.into())?;
                    reported_recording_offer = true;
                }
            }
            ensure!(
                !messages
                    .iter()
                    .any(|message| matches!(message, Message::LoggedOut)),
                "server logged the character out"
            );
        } else if let Some(exit) = hear(
            messages,
            &mut features,
            &mut world,
            &mut Out {
                sink: session,
                log: &mut *log,
                wire: server.wire(),
                sender: speaker,
            },
            follows_zones,
        )? {
            session.close()?;
            return Ok(exit);
        }
        let zone = log.zone.clone();
        match server
            .wire()
            .chat(packet.opcode, &packet.body, config.include_raw)
        {
            Ok(Some(event)) => log.record(&zone, RecordEvent::Chat(event))?,
            Ok(None) => (),
            Err(error) => log.record(
                &zone,
                RecordEvent::DecodeError(DecodeError {
                    kind: "decode_error",
                    opcode: packet.opcode,
                    payload_hex: hex::encode(&packet.body),
                    error: error.to_string(),
                }),
            )?,
        }
    }
}

/// A listener can repeat the entry position only until the zone supersedes it.
fn supersedes_entry_position(message: &Message, world: &World) -> bool {
    let own_spawn = world.own_spawn.filter(|&id| id != 0);
    match message {
        Message::ZoneOffer(_)
        | Message::ZoneAnswer(_)
        | Message::Handoff(_)
        | Message::Unreadable {
            part: Part::ZoneOffer | Part::ZoneAnswer,
            ..
        } => true,
        Message::Event(WorldEvent::Position { spawn_id, .. } | WorldEvent::Despawn(spawn_id)) => {
            own_spawn == Some(*spawn_id)
        }
        Message::Event(WorldEvent::Death(death)) => {
            own_spawn.map(u32::from) == Some(death.spawn_id)
        }
        Message::Event(event) => world.owns_death(event),
        _ => false,
    }
}

/// Who the session speaks for: the character it logged in as, and their
/// spawn once the zone has admitted them.
fn sender<'a>(name: &'a str, world: &World) -> eq_network_game::request::Sender<'a> {
    eq_network_game::request::Sender {
        name,
        spawn_id: world.player.as_ref().map(|player| player.spawn_id),
    }
}

/// How many messages the session's own news may add while one step is
/// heard, before the features are taken to answer each other without end.
const MOST_NEWS: usize = 16;

/// Keeps opcode and body length on existing world rejection diagnostics.
/// The decoder's reason and message count are preserved; no body is logged.
fn read_messages(
    wire: &dyn super::wire::Wire,
    opcode: u16,
    body: &[u8],
    item_rules: eq_network_game::inventory::ItemRules,
) -> Vec<Message> {
    let mut messages = wire.messages(opcode, body, item_rules);
    for message in &mut messages {
        if let Message::Unreadable {
            part: eq_network_game::message::Part::World,
            error,
        } = message
        {
            *error = format!(
                "{error} (opcode=0x{opcode:04x}, payload_len={})",
                body.len()
            );
        }
    }
    messages
}

/// Lets every feature hear messages, the zone's or what the session itself
/// made happen ([`World::happened`]), in order, what a message makes happen
/// before the message after it, and tells the host each event once the zone
/// has admitted the player. Returns how the session ends, once a feature
/// decides.
///
/// # Errors
/// Returns an error when a feature fails, when the server logs the player
/// out or moves them where no feature follows, and when the news does not
/// settle.
fn hear(
    messages: Vec<Message>,
    features: &mut Features,
    world: &mut World,
    out: &mut Out<'_, '_>,
    follows_zones: bool,
) -> Result<Option<ZoneExit>> {
    let mut queue = std::collections::VecDeque::from(messages);
    let mut news = 0;
    while let Some(mut message) = queue.pop_front() {
        features.explain(&mut message, world);
        if let Message::Unreadable { part, error } = &message {
            out.log.diagnostic(format!("{part} rejected: {error}"))?;
        }
        if world.ready() {
            features.observe(&message, world, out)?;
        } else {
            features.admit(&message, world)?;
        }
        if let Some(exit) = world.take_exit() {
            return Ok(Some(exit));
        }
        ensure!(
            !matches!(message, Message::LoggedOut),
            "server logged the character out"
        );
        // The server moving the player where no feature follows ends the
        // session.
        ensure!(
            follows_zones || !matches!(message, Message::ZoneOffer(_)),
            "server requested a new zone, which this server type cannot follow yet"
        );
        // Before the admission, the features staged what they need of it.
        if let (Message::Event(event), true) = (message, world.ready()) {
            out.log.send(ClientEvent::World(event))?;
        }
        let made = world.take_news();
        news += made.len();
        ensure!(news <= MOST_NEWS, "the session's own news did not settle");
        for message in made.into_iter().rev() {
            queue.push_front(message);
        }
    }
    Ok(None)
}

/// How the session ends once a step into the features decided it, or else
/// once every feature has heard what the step made happen.
///
/// # Errors
/// Returns an error as [`hear`] does.
fn settle(
    features: &mut Features,
    world: &mut World,
    out: &mut Out<'_, '_>,
    follows_zones: bool,
) -> Result<Option<ZoneExit>> {
    if let Some(exit) = world.take_exit() {
        return Ok(Some(exit));
    }
    let news = world.take_news();
    hear(news, features, world, out, follows_zones)
}

/// Why a command is refused when no feature of the server type takes it:
/// the words the client greys out such a control with.
const UNOFFERED: &str = "Not available on this server";

/// Refuses a command no feature of the server type takes, through the event
/// its caller waits for, so the player hears why instead of nothing.
fn unoffered(command: &ClientCommand, log: &mut Events<'_>) -> Result<()> {
    actions::refuse(command, UNOFFERED, log)
}

/// Why death or a zone transfer holds every command.
fn hold_reason(world: &World) -> &'static str {
    if world.lifecycle.is_dead() {
        "The player is dead"
    } else {
        "The player is between zones"
    }
}

/// Recheck the lifecycle for each command in the already drained batch.
/// A prior command can change it directly or through the news it settles.
fn apply_commands(
    commands: &[ClientCommand],
    features: &mut Features,
    world: &mut World,
    out: &mut Out<'_, '_>,
    follows_zones: bool,
) -> Result<Option<ZoneExit>> {
    let mut pending = commands.iter();
    while let Some(command) = pending.next() {
        if let Some(reason) = features.refusal(command, world, Instant::now()) {
            actions::refuse(command, reason, out.log)?;
            continue;
        }
        out.sender = sender(out.sender.name, world);
        let handled = features.handle(command, world, out)?;
        if let Some(exit) = settle(features, world, out, follows_zones)? {
            let reason = if world.lifecycle.blocks_motion() {
                hold_reason(world)
            } else {
                "The zone connection ended"
            };
            for command in pending {
                actions::refuse(command, reason, out.log)?;
            }
            return Ok(Some(exit));
        }
        if !handled {
            unoffered(command, out.log)?;
        }
    }
    Ok(None)
}

/// Tells the host the zone admitted the player, as the features shape them,
/// and lets every feature tell what it staged.
fn admit(
    mut player: crate::world::PlayerState,
    zone: &Zone,
    features: &mut Features,
    world: &mut World,
    out: &mut Out<'_, '_>,
) -> Result<()> {
    features.shape(&mut player);
    out.sender.spawn_id = Some(player.spawn_id);
    world.player.admit(player.clone());
    world.admitted = Some(Instant::now());
    out.log
        .send(ClientEvent::World(crate::world::WorldEvent::Entered {
            capabilities: features.capabilities(),
            choices: features.choices(),
            session_id: world.session_id,
            zone: zone.name.clone(),
            player: Box::new(player),
            far_clip: zone.far_clip,
            spell_landing_flag: features.spell_landing_flag,
        }))?;
    if let Some(sky) = zone.sky {
        out.log
            .send(ClientEvent::World(crate::world::WorldEvent::Sky(sky)))?;
    }
    if let Some(description) = &zone.description {
        out.log.send(ClientEvent::World(
            crate::world::WorldEvent::ZoneDescription(description.clone()),
        ))?;
    }
    world.gravity = zone.description.as_ref().map(|zone| zone.gravity);
    features.admitted(world, out)?;
    out.log
        .send(ClientEvent::Progress(ConnectionStage::Ready))?;
    out.status(ConnectionState::Connected, world)?;
    out.log.diagnostic(format!(
        "Zone login sequence complete for {}; waiting for ongoing server traffic",
        zone.name
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eq_network_game::{
        chat::OutboundChat,
        inventory::{InventoryMove, InventorySlot, ItemUse, MoveQuantity},
        movement::{DiagonalRates, MotionCalibration, MovementMode, MovementRequest},
    };

    /// How many kinds of command there are.
    const KINDS: usize = 87;

    /// The kinds the login and world servers take, before any zone session.
    const BEFORE_ZONES: [usize; 4] = [0, 1, 70, 75];

    /// Which kind of command this is. A new command is a compile error here
    /// until it has a number, and then a test failure until the list below
    /// has one of it, so no command can go without an owner unnoticed.
    fn kind(command: &ClientCommand) -> usize {
        match command {
            // The world server's, before any zone session.
            ClientCommand::SelectCharacter { .. } => 0,
            ClientCommand::DeleteCharacter(_) => 75,
            ClientCommand::CreateCharacter { .. } | ClientCommand::GenerateName { .. } => 1,
            ClientCommand::SwapSpell { .. } => 2,
            ClientCommand::UseItem(_) => 3,
            ClientCommand::ClickDoor { .. } => 4,
            ClientCommand::PickUp { .. } => 5,
            ClientCommand::CrossZoneLine { .. } => 6,
            ClientCommand::ScribeSpell { .. } => 7,
            ClientCommand::DeleteSpell { .. } => 8,
            ClientCommand::ForgetSpell { .. } => 9,
            ClientCommand::MemorizeSpell { .. } => 10,
            ClientCommand::CastSpell { .. } => 11,
            ClientCommand::CancelBuff(_) => 81,
            ClientCommand::RangedAttack(_) => 82,
            ClientCommand::StopSong { .. } => 83,
            ClientCommand::DepopLocalNpc { .. } => 84,
            ClientCommand::SetPosture { .. } => 12,
            ClientCommand::MoveInventory(_) => 13,
            ClientCommand::DestroyItem(_) => 80,
            ClientCommand::DropItem(_) => 86,
            ClientCommand::SendChat(_) => 14,
            ClientCommand::InspectItem { .. } => 15,
            ClientCommand::Consider { .. } => 16,
            ClientCommand::Camp { .. } => 17,
            ClientCommand::Loot { .. } => 18,
            ClientCommand::LootItem { .. } => 19,
            ClientCommand::EndLoot { .. } => 20,
            ClientCommand::Shop { .. } => 21,
            ClientCommand::Buy { .. } => 22,
            ClientCommand::Sell { .. } => 23,
            ClientCommand::Jump { .. } => 24,
            ClientCommand::AutoAttack { .. } => 25,
            ClientCommand::SelectTarget { .. } => 26,
            ClientCommand::ConfigureMotion { .. } => 27,
            ClientCommand::Move(_) => 28,
            ClientCommand::OfferTrade { .. } => 29,
            ClientCommand::AcceptTrade { .. } => 30,
            ClientCommand::CancelTrade { .. } => 31,
            ClientCommand::MoveCoins { .. } => 32,
            ClientCommand::UseAbility { .. } => 33,
            ClientCommand::Consume { .. } => 34,
            ClientCommand::WhoAll { .. } => 35,
            ClientCommand::Consent { .. } => 36,
            ClientCommand::SummonCorpse { .. } => 37,
            ClientCommand::DragCorpse { .. } => 38,
            ClientCommand::DropCorpse { .. } => 39,
            ClientCommand::Pet { .. } => 40,
            ClientCommand::AutoEat { .. } => 41,
            ClientCommand::AnswerListPolls { .. } => 76,
            ClientCommand::SetMovementOptions { .. } => 77,
            ClientCommand::SetBookOptions { .. } => 78,
            ClientCommand::ExitAtOnce { .. } => 79,
            ClientCommand::Training { .. } => 42,
            ClientCommand::AnswerResurrection { .. } => 43,
            ClientCommand::ReadItem { .. } => 44,
            ClientCommand::Combine { .. } => 45,
            ClientCommand::OpenContainer { .. } => 46,
            ClientCommand::CloseContainer { .. } => 47,
            ClientCommand::InviteToGroup { .. } => 48,
            ClientCommand::FollowGroup { .. } => 49,
            ClientCommand::DeclineGroup { .. } => 50,
            ClientCommand::Disband { .. } => 51,
            ClientCommand::ToggleAway { .. } => 52,
            ClientCommand::ToggleAnonymous { .. } => 53,
            ClientCommand::ToggleRoleplay { .. } => 54,
            ClientCommand::Random { .. } => 55,
            ClientCommand::Emote { .. } => 56,
            ClientCommand::Assist { .. } => 57,
            ClientCommand::RaidInvite { .. } => 58,
            ClientCommand::RaidAccept { .. } => 59,
            ClientCommand::RaidDecline { .. } => 60,
            ClientCommand::RaidLeave { .. } => 61,
            ClientCommand::RaidLock { .. } => 62,
            ClientCommand::RaidMove { .. } => 63,
            ClientCommand::RaidMakeLeader { .. } => 64,
            ClientCommand::RaidRemove { .. } => 65,
            ClientCommand::EnvironmentalDamage { .. } => 66,
            ClientCommand::BledOut { .. } => 67,
            ClientCommand::AnswerSacrifice { .. } => 68,
            ClientCommand::AnswerTranslocate { .. } => 69,
            // The login server's, before the world.
            ClientCommand::SelectServer { .. } => 70,
            ClientCommand::DropInvisibility { .. } => 71,
            ClientCommand::SpeedInputs { .. } => 72,
            ClientCommand::ShowHelm { .. } => 73,
            ClientCommand::ClimbReach { .. } => 74,
            ClientCommand::LiquidDamage { .. } => 85,
        }
    }

    /// One of every command a zone session takes.
    #[allow(
        clippy::too_many_lines,
        reason = "one literal for each kind of command"
    )]
    fn zone_commands() -> Vec<ClientCommand> {
        let (session_id, created) = (1, Instant::now());
        let position = crate::world::Position::default();
        let inventory = super::super::combat::test_ranged_inventory(
            eq_network_game::combat::ranged::RangedKind::Archery,
        );
        vec![
            ClientCommand::StopSong {
                session_id,
                created,
            },
            ClientCommand::RangedAttack(eq_network_game::combat::ranged::RangedAttack {
                session_id,
                target: eq_network_game::targets::TargetState::default(),
                loadout: eq_network_game::combat::ranged::RangedLoadout::current(&inventory)
                    .unwrap(),
                created,
            }),
            ClientCommand::CancelBuff(eq_network_game::buffs::cancel::Request {
                session_id,
                revision: 1,
                slot: 3,
                spell_id: 42,
                beneficial: Some(true),
                cancellable: Some(true),
                created,
            }),
            ClientCommand::SwapSpell {
                session_id,
                from: 0,
                to: 1,
                from_spell: 202,
                to_spell: None,
                created,
            },
            ClientCommand::UseItem(ItemUse {
                request_id: 1,
                session_id,
                revision: 1,
                slot: InventorySlot(22),
                target_id: 7,
                created,
            }),
            ClientCommand::ClickDoor {
                session_id,
                door_id: 1,
                created,
            },
            ClientCommand::PickUp {
                session_id,
                drop_id: 1,
                created,
            },
            ClientCommand::CrossZoneLine {
                session_id,
                destination: eq_network_game::zoning::ZoneLineDestination::Reference(1),
                position,
                created,
            },
            ClientCommand::ScribeSpell {
                session_id,
                revision: 1,
                slot: 0,
                spell_id: 202,
                created,
            },
            ClientCommand::DeleteSpell {
                session_id,
                slot: 0,
                spell_id: 202,
                created,
            },
            ClientCommand::ForgetSpell {
                session_id,
                gem: 0,
                spell_id: 202,
                created,
            },
            ClientCommand::MemorizeSpell {
                session_id,
                gem: 0,
                spell_id: 202,
                created,
            },
            ClientCommand::CastSpell {
                session_id,
                gem: 0,
                spell_id: 202,
                target_id: 7,
                created,
            },
            ClientCommand::SetPosture {
                session_id,
                spawn_id: 7,
                posture: eq_network_game::command::Posture::Sitting,
                created,
            },
            ClientCommand::MoveInventory(InventoryMove {
                sequence: None,
                session_id,
                revision: 1,
                from: InventorySlot(22),
                to: InventorySlot(30),
                quantity: MoveQuantity::Whole,
                created,
            }),
            ClientCommand::DestroyItem(Box::new(eq_network_game::inventory::InventoryDestroy {
                session_id,
                revision: 1,
                created,
                item: super::super::feature::testing::item(30),
            })),
            ClientCommand::DropItem(Box::new(eq_network_game::inventory::InventoryDrop {
                session_id,
                own_id: 7,
                revision: 1,
                created,
                item: super::super::feature::testing::item(30),
            })),
            ClientCommand::SendChat(OutboundChat::Say("Hail".into())),
            ClientCommand::DepopLocalNpc {
                session_id,
                target: eq_network_game::targets::TargetState::default(),
                created,
            },
            ClientCommand::InspectItem {
                session_id,
                link_body: String::new(),
            },
            ClientCommand::Consider {
                session_id,
                own_id: 7,
                target_id: 8,
                created,
            },
            ClientCommand::Camp {
                session_id,
                created,
            },
            ClientCommand::BledOut {
                session_id,
                created,
            },
            ClientCommand::Loot {
                session_id,
                corpse_id: 8,
                created,
            },
            ClientCommand::LootItem {
                session_id,
                corpse_id: 8,
                own_id: 7,
                place: 0,
                auto: true,
                created,
            },
            ClientCommand::EndLoot {
                session_id,
                corpse_id: 8,
            },
            ClientCommand::Shop {
                session_id,
                merchant_id: 9,
                own_id: 7,
                open: true,
                created,
            },
            ClientCommand::Buy {
                session_id,
                merchant_id: 9,
                own_id: 7,
                slot: 0,
                quantity: 1,
                created,
            },
            ClientCommand::Sell {
                session_id,
                merchant_id: 9,
                own_id: 7,
                slot: 22,
                quantity: 1,
                created,
            },
            ClientCommand::Jump {
                session_id,
                created,
            },
            ClientCommand::AutoAttack {
                session_id,
                enabled: true,
                created,
            },
            ClientCommand::SelectTarget {
                request_id: 1,
                session_id,
                spawn_id: None,
                expected_pet: None,
            },
            ClientCommand::ConfigureMotion {
                session_id,
                calibration: MotionCalibration {
                    diagonal: DiagonalRates::default(),
                    units_per_second: 6.0,
                    velocity_scale: 0.05,
                    animation: 12,
                    backward: None,
                    walk: None,
                    strafe: None,
                },
                created,
            },
            ClientCommand::SpeedInputs {
                session_id,
                revision: 1,
                report_id: 1,
                state: eq_network_game::movement::SpeedInputState::Ready(
                    eq_network_game::movement::SpeedInputs {
                        eqmac_fatigue: None,
                        rate: 0,
                        weight: 0,
                        capacity: 100,
                    },
                ),
            },
            ClientCommand::Move(MovementRequest {
                mode: MovementMode::Forward,
                heading: eq_network_game::movement::HeadingUpdate::Rate,
                session_id,
                position,
                created,
            }),
            ClientCommand::OfferTrade {
                session_id,
                with_id: 9,
                created,
            },
            ClientCommand::AcceptTrade {
                session_id,
                created,
            },
            ClientCommand::CancelTrade { session_id },
            ClientCommand::MoveCoins {
                session_id,
                transfer: eq_network_game::money::CoinTransfer {
                    from: eq_network_game::money::CoinPlace::Purse,
                    to: eq_network_game::money::CoinPlace::Cursor,
                    coin: eq_network_game::money::Coin::Gold,
                    into: eq_network_game::money::Coin::Gold,
                    amount: 2,
                },
                created,
            },
            ClientCommand::UseAbility {
                target: None,
                session_id,
                ability: eq_network_game::abilities::Ability::Kick,
                created,
            },
            ClientCommand::Consume {
                session_id,
                slot: InventorySlot(22),
                created,
            },
            ClientCommand::AnswerListPolls {
                session_id,
                enabled: false,
            },
            ClientCommand::SetMovementOptions {
                session_id,
                request_id: 1,
                options: eq_network_game::movement::MovementOptions::default(),
                created,
            },
            ClientCommand::SetBookOptions {
                session_id,
                memorize_without_sitting: false,
            },
            ClientCommand::ExitAtOnce {
                session_id,
                created,
            },
            ClientCommand::AutoEat {
                session_id,
                auto_eat: eq_network_game::food::AutoEat::Anything,
            },
            ClientCommand::WhoAll {
                session_id,
                filter: eq_network_game::who::WhoFilter::default(),
            },
            ClientCommand::Consent {
                session_id,
                name: "Helper".into(),
                given: true,
            },
            ClientCommand::SummonCorpse {
                session_id,
                spawn_id: 4,
            },
            ClientCommand::DragCorpse {
                session_id,
                spawn_id: 4,
            },
            ClientCommand::DropCorpse {
                session_id,
                spawn_id: None,
            },
            ClientCommand::Pet {
                session_id,
                command: eq_network_game::pets::PetCommand::Follow,
                pet: None,
                target: None,
                created,
            },
            ClientCommand::Training {
                session_id,
                request: eq_network_game::training::TrainingRequest::End,
                created,
            },
            ClientCommand::AnswerResurrection {
                session_id,
                offer_id: eq_network_game::confirmations::ConsentId {
                    session_id,
                    kind: eq_network_game::confirmations::ConsentKind::Resurrection,
                    sequence: 1,
                },
                accept: true,
            },
            ClientCommand::AnswerSacrifice {
                session_id,
                accept: false,
            },
            ClientCommand::AnswerTranslocate {
                session_id,
                offer_id: eq_network_game::confirmations::ConsentId {
                    session_id,
                    kind: eq_network_game::confirmations::ConsentKind::Translocate,
                    sequence: 1,
                },
                accept: true,
            },
            ClientCommand::DropInvisibility {
                session_id,
                steps: vec![eq_network_game::buffs::InvisibilityStep::State(0)],
            },
            ClientCommand::InviteToGroup {
                session_id,
                name: "Friend".into(),
            },
            ClientCommand::FollowGroup {
                session_id,
                inviter: "Leader".into(),
                invitation_id: eq_network_game::group::InvitationId::new(1),
            },
            ClientCommand::DeclineGroup {
                session_id,
                inviter: "Leader".into(),
                invitation_id: eq_network_game::group::InvitationId::new(1),
            },
            ClientCommand::Disband { session_id },
            ClientCommand::ToggleAway { session_id },
            ClientCommand::ToggleAnonymous { session_id },
            ClientCommand::ToggleRoleplay { session_id },
            ClientCommand::ShowHelm {
                session_id,
                show: true,
            },
            ClientCommand::ClimbReach {
                session_id,
                per_step: 3.15,
            },
            ClientCommand::Random {
                session_id,
                low: 1,
                high: 6,
            },
            ClientCommand::Emote {
                session_id,
                text: "waves.".into(),
            },
            ClientCommand::Assist {
                session_id,
                spawn_id: 300,
            },
            ClientCommand::RaidInvite {
                session_id,
                name: "Friend".into(),
            },
            ClientCommand::RaidAccept { session_id },
            ClientCommand::RaidDecline { session_id },
            ClientCommand::RaidLeave { session_id },
            ClientCommand::RaidLock {
                session_id,
                locked: true,
            },
            ClientCommand::RaidMove {
                session_id,
                name: "Friend".into(),
                group: Some(0),
            },
            ClientCommand::RaidMakeLeader {
                session_id,
                name: "Friend".into(),
            },
            ClientCommand::RaidRemove {
                session_id,
                name: "Friend".into(),
            },
            ClientCommand::ReadItem {
                session_id,
                revision: 0,
                slot: InventorySlot(23),
                created,
            },
            ClientCommand::Combine {
                session_id,
                container: InventorySlot(23),
                created,
            },
            ClientCommand::OpenContainer {
                session_id,
                drop_id: 9,
                created,
            },
            ClientCommand::CloseContainer { session_id },
            ClientCommand::EnvironmentalDamage {
                session_id,
                hazard: eq_network_game::hazards::Hazard::Falling,
                amount: 160,
                safe_fall: false,
            },
            ClientCommand::LiquidDamage {
                session_id,
                hazard: eq_network_game::hazards::LiquidHazard::Drowning,
                amount: 13,
                observation: eq_network_game::hazards::LiquidObservation {
                    revision: 0,
                    position: eq_network_game::world::Position::default(),
                    sampled_at: created,
                },
            },
        ]
    }

    #[test]
    fn the_list_has_one_of_every_command_a_zone_takes() {
        let commands = zone_commands();
        let kinds: std::collections::BTreeSet<_> = commands.iter().map(kind).collect();
        assert_eq!(kinds.len(), commands.len(), "one of each");
        assert_eq!(
            kinds,
            (0..KINDS)
                .filter(|kind| !BEFORE_ZONES.contains(kind))
                .collect()
        );
    }

    #[test]
    fn public_and_remote_providers_cannot_send_liquid_damage() {
        use super::super::feature::testing;
        use crate::client::{ClientConfig, ServerProtocol};
        use eq_network_game::hazards::{LiquidHazard, LiquidObservation};
        use eq_network_game::world::Capability;

        for (protocol, local_only) in [
            (ServerProtocol::Project1999, false),
            (ServerProtocol::EqEmu, false),
            (ServerProtocol::Quarm, false),
            (ServerProtocol::Takp, false),
            (ServerProtocol::Quarm, true),
            (ServerProtocol::Takp, true),
        ] {
            let mut config = ClientConfig::new("Example", "Fixture", "Server", "Tester");
            config.protocol = protocol;
            config.local_only = local_only;
            let mut world = World::for_zone(5, &config);
            let player = testing::player(7);
            let position = player.position;
            world.own_spawn = Some(7);
            world.body.own(7);
            world.body.place(position);
            world.player.admit(player.clone());
            world.admitted = Some(Instant::now());
            let mut setup = servers::Setup::new("Tester", config.auto_eat);
            setup.local_only = local_only;
            let server = servers::server_type(protocol);
            let mut features = Features::from_setup(server, &setup);
            let capabilities = features.capabilities();
            assert!(!capabilities.contains(&Capability::Drowning));
            assert!(!capabilities.contains(&Capability::LavaHeat));
            let outcome = testing::run_on(server.wire(), |out| -> Result<()> {
                out.log.send(ClientEvent::World(WorldEvent::Entered {
                    capabilities,
                    choices: Vec::new(),
                    session_id: 5,
                    zone: "example".to_owned(),
                    player: Box::new(player),
                    far_clip: None,
                    spell_landing_flag: None,
                }))?;
                let commands = [LiquidHazard::Drowning, LiquidHazard::Lava].map(|hazard| {
                    ClientCommand::LiquidDamage {
                        session_id: 5,
                        hazard,
                        amount: 13,
                        observation: LiquidObservation {
                            revision: 0,
                            position,
                            sampled_at: Instant::now(),
                        },
                    }
                });
                assert!(apply_commands(&commands, &mut features, &mut world, out, true)?.is_none());
                Ok(())
            });
            outcome.result.unwrap();
            assert!(outcome.sent.is_empty(), "{protocol:?}");
            assert!(outcome.unreliable.is_empty(), "{protocol:?}");
        }
    }

    #[test]
    fn liquid_ticks_follow_current_worker_inputs_and_completed_falls_survive_corrections() {
        use super::super::feature::testing;
        use crate::client::{ClientConfig, ServerProtocol};
        use eq_network_game::hazards::{Hazard, LiquidHazard, LiquidObservation};

        let mut config = ClientConfig::new("Example", "Fixture", "Local", "Tester");
        config.local_only = true;
        config.protocol = ServerProtocol::EqEmu;
        let mut world = World::for_zone(5, &config);
        let player = testing::player(7);
        let origin = player.position;
        world.own_spawn = Some(7);
        world.body.own(7);
        world.body.place(origin);
        world.player.admit(player.clone());
        world.admitted = Some(Instant::now());
        let mut setup = servers::Setup::new("Tester", config.auto_eat);
        setup.local_only = true;
        let mut features = Features::from_setup(servers::server_type(config.protocol), &setup);

        let outcome = testing::run(|out| -> Result<bool> {
            out.log.send(ClientEvent::World(WorldEvent::Entered {
                capabilities: features.capabilities(),
                choices: Vec::new(),
                session_id: 5,
                zone: "example".to_owned(),
                player: Box::new(player),
                far_clip: None,
                spell_landing_flag: None,
            }))?;
            let liquid = |hazard, revision, position, sampled_at| ClientCommand::LiquidDamage {
                session_id: 5,
                hazard,
                amount: 13,
                observation: LiquidObservation {
                    revision,
                    position,
                    sampled_at,
                },
            };
            let drown = LiquidHazard::Drowning;
            let lava = LiquidHazard::Lava;
            let first = liquid(drown, 0, origin, Instant::now());
            assert!(apply_commands(&[first], &mut features, &mut world, out, true)?.is_none());
            let corrected = eq_network_game::world::Position {
                z: origin.z + 25.0,
                ..origin
            };
            hear(
                vec![Message::Event(WorldEvent::Position {
                    spawn_id: 7,
                    position: corrected,
                    velocity: [0.0; 3],
                    forward: 0.0,
                })],
                &mut features,
                &mut world,
                out,
                true,
            )?;
            // Buff news supersedes samples even when their position is current.
            hear(
                vec![Message::Event(WorldEvent::BuffSnapshot(vec![Some(
                    eq_network_game::buffs::Buff {
                        spell_id: 86,
                        caster_level: 20,
                        effect_type: 2,
                        bard_modifier: 10,
                        duration_ticks: 5,
                        counters: 0,
                        caster_id: 7,
                    },
                )]))],
                &mut features,
                &mut world,
                out,
                true,
            )?;
            let now = Instant::now();
            let expired = now.checked_sub(Duration::from_secs(2));
            if let Some(expired) = expired {
                let tick = liquid(drown, 2, corrected, expired);
                assert!(apply_commands(&[tick], &mut features, &mut world, out, true)?.is_none());
            }
            let current = liquid(lava, 2, corrected, now);
            let commands = [
                liquid(drown, 0, origin, now),
                liquid(drown, 1, corrected, now),
                liquid(lava, 2, origin, now),
                current.clone(),
                current,
                ClientCommand::EnvironmentalDamage {
                    session_id: 5,
                    hazard: Hazard::Falling,
                    amount: 160,
                    safe_fall: false,
                },
            ];
            assert!(apply_commands(&commands, &mut features, &mut world, out, true)?.is_none());
            Ok(expired.is_some())
        });
        assert_liquid_tick_outcome(outcome);
    }

    fn assert_liquid_tick_outcome(outcome: super::super::feature::testing::Outcome<Result<bool>>) {
        use eq_network_game::hazards::{self, Hazard};

        let expiry_covered = outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [
                hazards::titanium_damage(7, Hazard::Drowning, 13).unwrap(),
                hazards::titanium_damage(7, Hazard::Lava, 13).unwrap(),
                hazards::titanium_damage(7, Hazard::Falling, 160).unwrap(),
            ]
        );
        assert_eq!(outcome.unreliable, []);
        let rejected: Vec<_> = outcome
            .events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::Diagnostic(line) => {
                    line.strip_prefix("Rejected LiquidDamage command: ")
                }
                _ => None,
            })
            .collect();
        let mut expected = vec!["The liquid observation is no longer current"; 3];
        expected.push("The liquid tick has no new damage");
        if expiry_covered {
            expected.insert(0, "The request expired");
        }
        assert_eq!(rejected, expected);
        let buffs = outcome
            .events
            .iter()
            .position(|event| matches!(event, ClientEvent::World(WorldEvent::BuffSnapshot(_))))
            .unwrap();
        assert!(matches!(
            outcome.events.get(buffs + 1),
            Some(ClientEvent::World(WorldEvent::LiquidHazardRevision {
                session_id: 5,
                revision: 2
            }))
        ));
    }

    #[test]
    fn discarded_moves_reply_to_the_original_request_without_publishing_a_position() {
        use super::super::feature::testing;
        use crate::client::ServerProtocol;
        use eq_network_game::{food::AutoEat, world::Position};

        let server = servers::server_type(ServerProtocol::EqEmu);
        let mut features =
            Features::from_setup(server, &servers::Setup::new("Tester", AutoEat::default()));
        let mut world = World::new(5);
        let created = Instant::now();
        for (ready, session_id, reason) in [
            (false, 5, "The player is between zones"),
            (true, 4, "That was meant for an earlier zone visit"),
            (true, 5, "The player cannot move yet"),
        ] {
            world.admitted = ready.then_some(created);
            let command = ClientCommand::Move(MovementRequest::new(
                MovementMode::Forward,
                session_id,
                Position::default(),
                created,
            ));
            let outcome = testing::run_on(server.wire(), |out| {
                apply_commands(&[command], &mut features, &mut world, out, true)
            });
            assert!(outcome.result.unwrap().is_none());
            assert_eq!(outcome.sent, []);
            assert_eq!(outcome.unreliable, []);
            assert!(matches!(
                &outcome.events[..],
                [
                    ClientEvent::World(WorldEvent::MotionRefused {
                        session_id: requested, created: stamp, reason: refusal,
                    }),
                    ClientEvent::Diagnostic(diagnostic),
                ] if *requested == session_id && *stamp == created
                    && refusal == reason && diagnostic.contains(reason)
            ));
        }
    }

    #[test]
    fn exactly_one_feature_owns_each_command_a_zone_takes() {
        // Every server type, so that each command has an owner somewhere:
        // EQEmu has every feature but the bleed-out report, which TAKP has.
        let servers: Vec<_> = [
            crate::client::ServerProtocol::EqEmu,
            crate::client::ServerProtocol::Project1999,
            crate::client::ServerProtocol::Quarm,
            crate::client::ServerProtocol::Takp,
        ]
        .into_iter()
        .map(|protocol| {
            let mut setup =
                servers::Setup::new("Tester", eq_network_game::food::AutoEat::default());
            setup.local_only = protocol == crate::client::ServerProtocol::EqEmu;
            Features::from_setup(servers::server_type(protocol), &setup)
        })
        .collect();
        for command in zone_commands() {
            if matches!(command, ClientCommand::AnswerListPolls { .. }) {
                // Session control is owned before the gameplay feature gates.
                assert!(servers.iter().any(|features| features.answers_list_polls));
                continue;
            }
            if Features::is_departure(&command) {
                // Departures are gated by the server type before their owner.
                assert!(servers.iter().any(|features| features.gameplay_departures));
                continue;
            }
            let needed = command
                .capability()
                .expect("a zone command needs a capability");
            // A feature may own an optional command without offering it.
            let mut offered = false;
            for features in &servers {
                let owners: Vec<_> = features
                    .provided
                    .iter()
                    .filter(|feature| feature.owns(&command))
                    .collect();
                assert!(owners.len() <= 1, "{command:?}");
                offered |= owners
                    .first()
                    .is_some_and(|owner| owner.capabilities().contains(&needed));
            }
            assert!(
                offered,
                "{command:?} needs {needed:?}, which no server type's owner offers"
            );
        }
        for selection in [
            ClientCommand::DeleteCharacter(
                eq_network_game::deletion::Request::new(
                    eq_network_game::deletion::Scope {
                        connection_id: 1,
                        selection_id: 1,
                    },
                    1,
                    "Disposable",
                    "Disposable",
                )
                .unwrap(),
            ),
            ClientCommand::SelectServer {
                selection_id: 1,
                index: 0,
            },
            ClientCommand::SelectCharacter {
                selection_id: 1,
                slot: 0,
            },
        ] {
            for features in &servers {
                assert!(!features
                    .provided
                    .iter()
                    .any(|feature| feature.owns(&selection)));
            }
        }
    }

    #[test]
    fn immediate_exit_and_option_requests_keep_life_session_and_freshness_gates() {
        use super::super::feature::testing;
        let features = Features::new(
            servers::server_type(crate::client::ServerProtocol::EqEmu),
            "Tester",
            eq_network_game::food::AutoEat::default(),
        );
        let now = Instant::now();
        let mut world = World::new(5);
        world.player.admit(testing::player(7));
        world.own_spawn = Some(7);
        world.admitted = Some(now);
        let exit = ClientCommand::ExitAtOnce {
            session_id: 5,
            created: now,
        };
        assert_eq!(features.refusal(&exit, &world, now), None);
        assert!(features
            .refusal(&exit, &world, now + Duration::from_secs(1))
            .is_some());
        let foreign = ClientCommand::ExitAtOnce {
            session_id: 6,
            created: now,
        };
        assert!(features.refusal(&foreign, &world, now).is_some());
        world.lifecycle.mark_dead();
        assert!(features.refusal(&exit, &world, now).is_some());
        let options = ClientCommand::SetMovementOptions {
            session_id: 5,
            request_id: 7,
            options: eq_network_game::movement::MovementOptions::default(),
            created: now,
        };
        let reason = features.refusal(&options, &world, now).unwrap();
        let output = testing::run(|out| actions::refuse(&options, reason, out.log));
        output.result.unwrap();
        assert_eq!(output.sent, []);
        assert!(matches!(
            output.events.first(),
            Some(ClientEvent::World(
                crate::world::WorldEvent::MovementOptionsRefused {
                    session_id: 5,
                    request_id: 7,
                    ..
                }
            ))
        ));
    }

    #[test]
    fn the_explicit_gameplay_departures_are_gated_by_the_server_type() {
        use super::super::feature::testing;
        use crate::world::Capability;
        let now = Instant::now();
        let mut world = World::new(5);
        world.player.admit(testing::player(7));
        world.own_spawn = Some(7);
        world.admitted = Some(now);
        let options = ClientCommand::SetMovementOptions {
            session_id: 5,
            request_id: 7,
            options: eq_network_game::movement::MovementOptions::default(),
            created: now,
        };
        let book = ClientCommand::SetBookOptions {
            session_id: 5,
            memorize_without_sitting: true,
        };
        assert_eq!(options.capability(), Some(Capability::GameplayDepartures));
        assert_eq!(book.capability(), Some(Capability::GameplayDepartures));
        // Every checked server type offers them while the owner tests them.
        for protocol in [
            crate::client::ServerProtocol::EqEmu,
            crate::client::ServerProtocol::Project1999,
            crate::client::ServerProtocol::Quarm,
            crate::client::ServerProtocol::Takp,
        ] {
            let features = Features::new(
                servers::server_type(protocol),
                "Tester",
                eq_network_game::food::AutoEat::default(),
            );
            assert!(
                features
                    .capabilities()
                    .contains(&Capability::GameplayDepartures),
                "{protocol:?}"
            );
            assert_eq!(
                features.refusal(&options, &world, now),
                None,
                "{protocol:?}"
            );
            assert_eq!(features.refusal(&book, &world, now), None, "{protocol:?}");
        }
        // A server type that withholds them refuses the option commands
        // before any owner sees them, and ordinary movement stays ungated.
        let mut withheld = Features::new(
            servers::server_type(crate::client::ServerProtocol::Project1999),
            "Tester",
            eq_network_game::food::AutoEat::default(),
        );
        withheld.gameplay_departures = false;
        assert!(!withheld
            .capabilities()
            .contains(&Capability::GameplayDepartures));
        for command in [&options, &book] {
            let reason = withheld
                .refusal(command, &world, now)
                .unwrap_or_else(|| panic!("takes {command:?}"));
            assert!(reason.contains("not offered"), "{reason}");
        }
        let jump = ClientCommand::Jump {
            session_id: 5,
            created: now,
        };
        assert_eq!(withheld.refusal(&jump, &world, now), None);
    }

    #[test]
    fn each_zone_hands_the_wire_the_players_new_spawn() {
        use crate::client::session::feature::testing;
        use eq_network_game::{
            command::{titanium_posture, Posture},
            request::Request,
        };
        let zone = super::super::admission::Zone {
            name: "qeynos".into(),
            far_clip: None,
            sky: None,
            description: None,
        };
        // The player zones twice, and each zone gives them a different spawn.
        for spawn_id in [7u16, 9] {
            let mut world = World::new(5);
            let mut features = Features::new(
                servers::server_type(crate::client::ServerProtocol::EqEmu),
                "Tester",
                eq_network_game::food::AutoEat::default(),
            );
            let outcome = testing::run(|out| {
                // Before admission the session speaks for no spawn.
                out.sender = sender("Tester", &world);
                assert_eq!(out.sender.spawn_id, None);
                admit(
                    testing::player(spawn_id),
                    &zone,
                    &mut features,
                    &mut world,
                    out,
                )?;
                // The admission turn itself, and every turn built after it,
                // speak for the spawn this zone gave the player.
                assert_eq!(out.sender.spawn_id, Some(spawn_id));
                assert_eq!(sender("Tester", &world).spawn_id, Some(spawn_id));
                out.request(&Request::Posture(Posture::Sitting))
            });
            outcome.result.unwrap();
            assert_eq!(
                outcome.sent.last(),
                Some(&titanium_posture(spawn_id, Posture::Sitting).unwrap())
            );
        }
    }

    #[test]
    fn the_admission_tells_the_zone_then_its_sky_then_its_description() {
        use crate::client::session::feature::testing;
        use crate::world::WorldEvent;
        let mut header = vec![0; 700];
        header[96..103].copy_from_slice(b"Example");
        let zone = super::super::admission::Zone {
            name: "qeynos".into(),
            far_clip: Some(450.0),
            sky: eq_network_game::clock::titanium_zone_sky(&header),
            description: eq_network_game::zones::titanium_description(&header).ok(),
        };
        let mut world = World::new(5);
        let mut features = Features::new(
            servers::server_type(crate::client::ServerProtocol::EqEmu),
            "Tester",
            eq_network_game::food::AutoEat::default(),
        );
        let outcome =
            testing::run(|out| admit(testing::player(7), &zone, &mut features, &mut world, out));
        outcome.result.unwrap();
        let told: Vec<String> = outcome
            .events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::Entered { far_clip, .. }) => {
                    Some(format!("entered, far clip {far_clip:?}"))
                }
                ClientEvent::World(WorldEvent::Sky(_)) => Some("sky".into()),
                ClientEvent::World(WorldEvent::ZoneDescription(description)) => {
                    Some(description.long_name.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(told, ["entered, far clip Some(450.0)", "sky", "Example"]);
    }

    /// Admits the player, spawn 7, with these features on the `EQMac` wire,
    /// the profile naming the bind point first; tells the session it knows
    /// the player's items, as the inventory feature would; hears the server
    /// put the player at -11 HP; and asks, as a host would, that the player
    /// bled out. Then the session hears its own news, and the death pause
    /// of a dead player runs out.
    fn bleed_out(
        mut features: Features,
    ) -> (
        World,
        crate::client::session::feature::testing::Outcome<Result<()>>,
    ) {
        use crate::client::session::{feature::testing, wire};
        use crate::world::{ItemHitPoints, WorldEvent};
        use eq_network_game::inventory::{Inventory, InventoryUpdate};
        let zone = super::super::admission::Zone {
            name: "qeynos".into(),
            far_clip: None,
            sky: None,
            description: None,
        };
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let bind = eq_network_game::zoning::BindPoint {
            zone_id: 2,
            position: crate::world::Position::default(),
        };
        let outcome = testing::run(|out| -> Result<()> {
            out.wire = &wire::EqMac;
            assert!(hear(
                vec![Message::Bind(bind)],
                &mut features,
                &mut world,
                out,
                true
            )?
            .is_none());
            admit(testing::player(7), &zone, &mut features, &mut world, out)?;
            let mut inventory = Inventory::default();
            inventory.apply(InventoryUpdate::Snapshot(Vec::new()));
            world.inventory = inventory.into();
            let report = Message::Event(WorldEvent::HitPoints {
                spawn_id: 7,
                current: -11,
                maximum: 100,
                items: ItemHitPoints::LeftOutOfCurrent,
            });
            assert!(hear(vec![report], &mut features, &mut world, out, true)?.is_none());
            let bled_out = ClientCommand::BledOut {
                session_id: 5,
                created: Instant::now(),
            };
            assert!(features.handle(&bled_out, &mut world, out)?);
            assert!(settle(&mut features, &mut world, out, true)?.is_none());
            if world.lifecycle.is_dead() {
                features.tick(Instant::now(), &mut world, out)?;
            }
            Ok(())
        });
        (world, outcome)
    }

    #[test]
    fn a_bleed_out_report_is_the_players_death_to_every_feature_and_the_host() {
        use crate::client::session::{
            bleeding::BleedingOut,
            transfers::{Home, Transfers},
        };
        use crate::world::WorldEvent;
        use eq_network_game::quarm;
        /// A count that finds the player's items add nothing.
        #[allow(clippy::unnecessary_wraps, reason = "the signature every count has")]
        const fn naked(_world: &World) -> Result<i32, &'static str> {
            Ok(0)
        }
        let features = Features {
            provided: vec![
                Box::new(Transfers::new("Tester", Home::Asked)),
                Box::new(BleedingOut::new(-11, Some(naked))),
            ],
            choices: Vec::new(),
            spell_landing_flag: None,
            answers_list_polls: false,
            gameplay_departures: false,
        };
        let (world, outcome) = bleed_out(features);
        outcome.result.unwrap();
        // Transfers heard the death the session made happen, and once the
        // death pause was over, the player asked their way home.
        assert!(world.lifecycle.is_dead());
        let level = world.player.as_ref().unwrap().level;
        assert_eq!(outcome.sent[0], quarm::bled_out(7, level).unwrap());
        assert_eq!(outcome.sent[1].opcode, quarm::ZONE_CHANGE);
        assert_eq!(outcome.sent.len(), 2);
        // The host heard the threshold at admission, and then the death, as
        // it hears the server's.
        let threshold = outcome
            .events
            .iter()
            .position(|event| matches!(event, ClientEvent::World(WorldEvent::DeathThreshold(-11))));
        let death = outcome.events.iter().position(|event| {
            matches!(event, ClientEvent::World(WorldEvent::Death(death)) if death.spawn_id == 7)
        });
        assert!(threshold.unwrap() < death.unwrap());
    }

    #[test]
    fn takp_takes_no_bleed_out_report_until_it_counts_what_the_items_add() {
        let (world, outcome) = bleed_out(Features::new(
            servers::server_type(crate::client::ServerProtocol::Takp),
            "Tester",
            eq_network_game::food::AutoEat::default(),
        ));
        outcome.result.unwrap();
        // Knowing the items is not enough: TAKP's -11 leaves out what they
        // add, so the player may well be alive. The host heard no threshold,
        // so it asks for nothing.
        assert_eq!(outcome.sent, []);
        assert!(!world.lifecycle.is_dead());
        assert!(!outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(crate::world::WorldEvent::DeathThreshold(_))
        )));
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::Diagnostic(said) if said.contains("cannot count")
        )));
    }

    #[test]
    fn an_unconfirmed_cast_precedes_the_same_spell_retry_and_its_begin() {
        use super::super::{casting::Casting, feature::testing};
        use eq_network_game::spells::SpellUpdate;

        let mut features = Features {
            provided: vec![Box::new(Casting::default())],
            choices: Vec::new(),
            spell_landing_flag: None,
            answers_list_polls: false,
            gameplay_departures: false,
        };
        let mut world = World::new(5);
        world.admitted = Some(Instant::now());
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        player.memorized_spells[0] = Some(202);
        world.player.admit(player);
        let cast = || ClientCommand::CastSpell {
            session_id: 5,
            gem: 0,
            spell_id: 202,
            target_id: 7,
            created: Instant::now(),
        };
        let began = WorldEvent::Spell(SpellUpdate::Began {
            caster_id: 7,
            spell_id: 2935,
            duration_ms: 3000,
        });
        let outcome = testing::run(|out| -> Result<()> {
            apply_commands(&[cast()], &mut features, &mut world, out, true)?;
            assert_eq!(
                features.refusal(&cast(), &world, Instant::now()),
                Some("Wait for the current cast to finish or interrupt it")
            );
            let timeout = Instant::now() + Duration::from_secs(5);
            features.tick(timeout, &mut world, out)?;
            features.tick(timeout, &mut world, out)?;
            apply_commands(&[cast()], &mut features, &mut world, out, true)?;
            hear(
                vec![Message::Event(began.clone())],
                &mut features,
                &mut world,
                out,
                true,
            )?;
            assert_eq!(
                features.refusal(&cast(), &world, Instant::now()),
                Some("Wait for the current cast to finish or interrupt it")
            );
            Ok(())
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 2);
        assert!(matches!(
            outcome.events.as_slice(),
            [
                ClientEvent::World(WorldEvent::CastPending {
                    session_id: 5,
                    spell_id: Some(202)
                }),
                ClientEvent::World(WorldEvent::CastRequestUnconfirmed {
                    session_id: 5,
                    spell_id: 202
                }),
                ClientEvent::Diagnostic(reason),
                ClientEvent::World(WorldEvent::CastPending {
                    session_id: 5,
                    spell_id: Some(202)
                }),
                ClientEvent::World(WorldEvent::CastPending {
                    session_id: 5,
                    spell_id: None
                }),
                ClientEvent::World(update)
            ] if reason == "Cast acknowledgement timed out; a manual retry is available"
                && update == &began
        ));
    }

    #[test]
    fn news_that_never_settles_ends_the_session() {
        use crate::client::session::feature::{testing, Feature};
        /// A feature that answers every message with another.
        struct Echo;
        impl Feature for Echo {
            fn observe(
                &mut self,
                _message: &Message,
                world: &mut World,
                _out: &mut Out<'_, '_>,
            ) -> Result<()> {
                world.happened(Message::Withheld);
                Ok(())
            }
        }
        let mut features = Features {
            provided: vec![Box::new(Echo)],
            choices: Vec::new(),
            spell_landing_flag: None,
            answers_list_polls: false,
            gameplay_departures: false,
        };
        let mut world = World::new(5);
        world.admitted = Some(Instant::now());
        let outcome = testing::run(|out| {
            hear(
                vec![Message::Withheld],
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        let Err(error) = outcome.result else {
            panic!("news that never settles must end the session");
        };
        assert_eq!(error.to_string(), "the session's own news did not settle");
    }

    #[test]
    fn meal_capability_matches_the_owner_that_accepts_configuration() {
        use crate::client::{session::feature::testing, ServerProtocol};
        use eq_network_game::{food::AutoEat, world::Capability};
        for (protocol, meals) in [
            (ServerProtocol::Quarm, true),
            (ServerProtocol::Takp, true),
            (ServerProtocol::EqEmu, true),
            (ServerProtocol::Project1999, true),
        ] {
            let server = servers::server_type(protocol);
            let mut features = Features::new(server, "Tester", AutoEat::default());
            assert!(features.capabilities().contains(&Capability::Inventory));
            assert_eq!(features.capabilities().contains(&Capability::Meals), meals);
            let command = ClientCommand::AutoEat {
                session_id: 5,
                auto_eat: AutoEat::Anything,
            };
            let consume = ClientCommand::Consume {
                session_id: 5,
                slot: eq_network_game::inventory::InventorySlot(22),
                created: Instant::now(),
            };
            assert_eq!(command.capability(), Some(Capability::Meals));
            assert_eq!(consume.capability(), Some(Capability::Meals));
            assert_eq!(
                features
                    .provided
                    .iter()
                    .any(|feature| feature.owns(&consume)),
                meals
            );
            let mut world = World::new(5);
            let outcome = testing::run_on(server.wire(), |out| {
                let handled = features.handle(&command, &mut world, out)?;
                assert_eq!(handled, meals);
                if !handled {
                    unoffered(&command, out.log)?;
                }
                Ok::<_, anyhow::Error>(())
            });
            outcome.result.unwrap();
            assert!(
                outcome.sent.is_empty(),
                "changing the preference sends no packet"
            );
            if meals {
                assert!(outcome.events.is_empty());
            } else {
                assert!(
                    matches!(outcome.events.as_slice(), [ClientEvent::Diagnostic(message)]
                    if message == "Rejected AutoEat command: Not available on this server")
                );
            }
        }
    }

    #[test]
    fn combined_quarm_inventory_capabilities_keep_one_owner_and_exchanges_disabled() {
        use crate::client::ServerProtocol;
        use eq_network_game::{food::AutoEat, inventory::InventorySlot, world::Capability};
        let server = servers::server_type(ServerProtocol::Quarm);
        let features = Features::new(server, "Tester", AutoEat::default());
        let capabilities = features.capabilities();
        for capability in [
            Capability::Inventory,
            Capability::Meals,
            Capability::Trading,
            Capability::Reading,
        ] {
            assert!(capabilities.contains(&capability), "{capability:?}");
        }
        assert!(!capabilities.contains(&Capability::Giving));
        for command in [
            ClientCommand::AutoEat {
                session_id: 5,
                auto_eat: AutoEat::Anything,
            },
            ClientCommand::Consume {
                session_id: 5,
                slot: InventorySlot(22),
                created: Instant::now(),
            },
            ClientCommand::Buy {
                session_id: 5,
                merchant_id: 9,
                own_id: 7,
                slot: 0,
                quantity: 1,
                created: Instant::now(),
            },
        ] {
            assert_eq!(
                features
                    .provided
                    .iter()
                    .filter(|feature| feature.owns(&command))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn ordinary_eqmac_doors_compose_with_reading_and_inventory_features() {
        use crate::client::ServerProtocol;
        use eq_network_game::{food::AutoEat, world::Capability};
        for protocol in [ServerProtocol::Quarm, ServerProtocol::Takp] {
            let features =
                Features::new(servers::server_type(protocol), "Tester", AutoEat::default());
            for capability in [
                Capability::Doors,
                Capability::Reading,
                Capability::Inventory,
                Capability::Trading,
                Capability::Meals,
            ] {
                assert!(
                    features.capabilities().contains(&capability),
                    "{protocol:?} {capability:?}"
                );
            }
            let click = ClientCommand::ClickDoor {
                session_id: 5,
                door_id: 3,
                created: Instant::now(),
            };
            assert_eq!(
                features
                    .provided
                    .iter()
                    .filter(|feature| feature.owns(&click))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn a_command_no_feature_takes_is_refused_as_unavailable() {
        use crate::client::session::feature::testing;
        // No owner is installed in this fixture, independently of server offers.
        let mut features = Features {
            provided: Vec::new(),
            choices: Vec::new(),
            spell_landing_flag: None,
            answers_list_polls: false,
            gameplay_departures: false,
        };
        let click = ClientCommand::ClickDoor {
            session_id: 5,
            door_id: 3,
            created: Instant::now(),
        };
        let mut world = World::new(5);
        let outcome = testing::run(|out| {
            let handled = features.handle(&click, &mut world, out)?;
            assert!(!handled);
            unoffered(&click, out.log)
        });
        outcome.result.unwrap();
        assert!(matches!(
            &outcome.events[..],
            [ClientEvent::World(crate::world::WorldEvent::DoorAction {
                session_id: 5,
                door_id: 3,
                error: Some(reason),
                ..
            }), ..] if reason == UNOFFERED
        ));
    }

    #[test]
    fn only_the_offering_server_turns_a_wire_record_into_answerable_consent() {
        use crate::client::{session::feature::testing, ServerProtocol};
        use eq_network_game::confirmations::{ConsentClosure, TRANSLOCATE_OPCODE};

        let mut body = vec![0; 92];
        body[..4].copy_from_slice(&4u32.to_le_bytes());
        body[12..18].copy_from_slice(b"Helper");
        let offer = eq_network_game::confirmations::titanium_translocate(&body).unwrap();
        for protocol in [ServerProtocol::EqEmu, ServerProtocol::Project1999] {
            let server = servers::server_type(protocol);
            let mut features =
                Features::new(server, "Tester", eq_network_game::food::AutoEat::default());
            let mut world = World::new(5);
            world.admitted = Some(Instant::now());
            let result = testing::run(|out| {
                hear(
                    read_messages(
                        server.wire(),
                        TRANSLOCATE_OPCODE,
                        &body,
                        eq_network_game::inventory::ItemRules::Takp,
                    ),
                    &mut features,
                    &mut world,
                    out,
                    true,
                )
            });
            assert!(result.result.unwrap().is_none());
            assert_eq!(
                result.sent,
                [] as [eq_network_game::command::EncodedCommand; 0]
            );
            if protocol == ServerProtocol::Project1999 {
                assert!(result.events.is_empty());
                continue;
            }
            let [ClientEvent::World(WorldEvent::TranslocateOffer(question))] =
                result.events.as_slice()
            else {
                panic!("expected a scoped consent offer");
            };
            let commands = [ClientCommand::AnswerTranslocate {
                session_id: 5,
                offer_id: question.id,
                accept: true,
            }];
            let answered =
                testing::run(|out| apply_commands(&commands, &mut features, &mut world, out, true));
            assert!(answered.result.unwrap().is_none());
            assert_eq!(
                answered.sent,
                [eq_network_game::confirmations::titanium_translocate_answer(
                    &offer, true
                )]
            );
            assert!(matches!(
                answered.events.as_slice(),
                [ClientEvent::World(WorldEvent::ConsentClosed { id, closure: ConsentClosure::AnswerSent { accept: true } })]
                    if *id == question.id
            ));
            assert!(!world.lifecycle.blocks_motion());
        }
    }

    #[test]
    fn a_titanium_zone_reports_everything_its_features_offer() {
        use crate::world::Capability;
        let features = |protocol| {
            Features::new(
                servers::server_type(protocol),
                "Tester",
                eq_network_game::food::AutoEat::default(),
            )
        };
        let p99 = features(crate::client::ServerProtocol::Project1999);
        // P99 leaves the map to the player, which is not offered until then.
        assert_eq!(p99.choices(), [Capability::Map]);
        let p99 = p99.capabilities();
        assert!(!p99.contains(&Capability::Map));
        for capability in [
            Capability::Casting,
            Capability::Spellbook,
            Capability::Inventory,
            Capability::Trading,
            Capability::Giving,
            Capability::Moving,
            Capability::Targeting,
            Capability::Combat,
            Capability::RangedCombat,
            Capability::Looting,
            Capability::Talking,
            Capability::Camping,
            Capability::Doors,
            Capability::GroundItems,
            Capability::Zoning,
            Capability::Abilities,
            Capability::Who,
            Capability::Corpses,
            Capability::Pets,
        ] {
            assert!(p99.contains(&capability), "{capability:?}");
        }
        assert!(p99.contains(&Capability::Falling));
        assert!(p99.contains(&Capability::Jumping));
        assert!(p99.contains(&Capability::Reading));
        assert!(!p99.contains(&Capability::GroundDropping));
        assert!(!p99.contains(&Capability::Training));
        let eqemu = features(crate::client::ServerProtocol::EqEmu).capabilities();
        assert!(!eqemu.contains(&Capability::GroundDropping));
        assert!(eqemu.contains(&Capability::Falling));
        assert!(eqemu.contains(&Capability::Training));
        assert!(eqemu.contains(&Capability::Resurrection));
        assert!(eqemu.contains(&Capability::Reading));
        assert!(eqemu.contains(&Capability::Tradeskills));
        assert!(p99.contains(&Capability::Tradeskills));
        assert!(eqemu.contains(&Capability::WorldContainers));
        assert!(!p99.contains(&Capability::WorldContainers));
        assert!(eqemu.contains(&Capability::Map));
        assert!(eqemu.contains(&Capability::DeletingSpells));
        assert!(eqemu.contains(&Capability::MovingSpells));
        assert!(!p99.contains(&Capability::DeletingSpells));
        assert!(p99.contains(&Capability::MovingSpells));
        assert!(eqemu.contains(&Capability::MerchantOffers));
        assert!(!p99.contains(&Capability::MerchantOffers));
        assert!(eqemu.contains(&Capability::Grouping));
        assert!(p99.contains(&Capability::Grouping));
        assert!(eqemu.contains(&Capability::Listing));
        assert!(!p99.contains(&Capability::Listing));
        for capability in [
            Capability::Rolling,
            Capability::Emoting,
            Capability::Assisting,
            Capability::Raiding,
            Capability::Sacrifice,
            Capability::Translocate,
            Capability::ServerTicks,
            Capability::ShowingHelm,
            Capability::CancellingBuffs,
            Capability::StoppingSongs,
        ] {
            assert!(eqemu.contains(&capability));
            assert!(!p99.contains(&capability));
        }
        assert!(p99.contains(&Capability::EnvironmentalDamage));
        let mut setup = servers::Setup::new("Tester", eq_network_game::food::AutoEat::default());
        setup.local_only = true;
        let local = Features::from_setup(
            servers::server_type(crate::client::ServerProtocol::EqEmu),
            &setup,
        )
        .capabilities();
        assert!(local.contains(&Capability::GroundDropping));
        for liquid in [Capability::Drowning, Capability::LavaHeat] {
            assert!(!p99.contains(&liquid));
            assert!(!eqemu.contains(&liquid));
            assert!(local.contains(&liquid));
        }
        assert!(p99.contains(&Capability::AnsweringListPolls));
        assert!(!eqemu.contains(&Capability::AnsweringListPolls));
        assert!(eqemu.contains(&Capability::GameplayDepartures));
        assert!(p99.contains(&Capability::GameplayDepartures));
        assert!(eqemu.contains(&Capability::Swimming));
        assert!(!p99.contains(&Capability::Swimming));
        assert_eq!(eqemu.len(), p99.len() + 17);
    }

    #[test]
    fn an_eqmac_zone_reports_everything_its_features_offer() {
        use crate::world::Capability;
        let features = |protocol| {
            Features::new(
                servers::server_type(protocol),
                "Tester",
                eq_network_game::food::AutoEat::default(),
            )
            .capabilities()
        };
        // Both EQMac policies move, jump, fall, zone, camp, cast spells and
        // move inventory, read, shop and eat. TAKP also offers the social
        // actions checked there.
        assert_eq!(
            features(crate::client::ServerProtocol::Quarm),
            [
                Capability::Casting,
                Capability::Spellbook,
                Capability::Inventory,
                Capability::Trading,
                Capability::Moving,
                Capability::Falling,
                Capability::Targeting,
                Capability::Combat,
                Capability::Looting,
                Capability::Talking,
                Capability::Camping,
                Capability::Doors,
                Capability::GroundItems,
                Capability::Zoning,
                Capability::Abilities,
                Capability::Reading,
                Capability::EnvironmentalDamage,
                Capability::Jumping,
                Capability::Posture,
                Capability::Meals,
                Capability::GameplayDepartures
            ]
        );
        assert_eq!(
            features(crate::client::ServerProtocol::Takp),
            [
                Capability::Casting,
                Capability::Spellbook,
                Capability::Inventory,
                Capability::Trading,
                Capability::Giving,
                Capability::Moving,
                Capability::Falling,
                Capability::Targeting,
                Capability::Combat,
                Capability::Talking,
                Capability::Camping,
                Capability::Doors,
                Capability::GroundItems,
                Capability::Zoning,
                Capability::Abilities,
                Capability::Reading,
                Capability::Grouping,
                Capability::Listing,
                Capability::EnvironmentalDamage,
                Capability::BleedingOut,
                Capability::Translocate,
                Capability::Jumping,
                Capability::Posture,
                Capability::Meals,
                Capability::GameplayDepartures
            ]
        );
    }

    #[test]
    fn quarm_can_sit_camp_and_return_to_selection_without_locomotion() {
        use super::super::feature::testing;
        use eq_network_game::{command::Posture, quarm, world::Capability};
        let server = servers::server_type(crate::client::ServerProtocol::Quarm);
        let mut features =
            Features::new(server, "Tester", eq_network_game::food::AutoEat::default());
        // Keep proving that posture and camping do not depend on movement,
        // even though the full Quarm policy now provides movement too.
        features
            .provided
            .retain(|feature| !feature.capabilities().contains(&Capability::Moving));
        assert!(!features.capabilities().contains(&Capability::Moving));
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        world.admitted = Some(Instant::now());
        let sit = ClientCommand::SetPosture {
            session_id: 5,
            spawn_id: 7,
            posture: Posture::Sitting,
            created: Instant::now(),
        };
        let camp = ClientCommand::Camp {
            session_id: 5,
            created: Instant::now(),
        };
        let before = Instant::now();
        let output = testing::run_on(server.wire(), |out| -> Result<()> {
            features.admitted(&mut world, out)?;
            assert!(features.handle(&sit, &mut world, out)?);
            assert!(features.handle(&camp, &mut world, out)?);
            // A duplicate request cannot reset the countdown or send twice.
            assert!(features.handle(&camp, &mut world, out)?);
            features.tick(before + Duration::from_secs(29), &mut world, out)
        });
        output.result.unwrap();
        assert_eq!(
            output.sent,
            [quarm::posture(7, Posture::Sitting).unwrap(), quarm::camp()]
        );
        assert_eq!(
            output.unreliable,
            Vec::<eq_network_game::command::EncodedCommand>::new()
        );
        assert!(world.body.position().is_none());
        let output = testing::run_on(server.wire(), |out| {
            features.tick(Instant::now() + Duration::from_secs(31), &mut world, out)
        });
        output.result.unwrap();
        assert_eq!(output.sent, [quarm::logout()]);
        let output = testing::run_on(server.wire(), |out| {
            features.observe(&Message::LoggedOut, &mut world, out)
        });
        output.result.unwrap();
        assert!(matches!(world.exit(), Some(ZoneExit::CharacterSelect)));
    }

    #[test]
    fn quarm_standing_cancels_camp() {
        use super::super::feature::testing;
        use eq_network_game::{
            command::Posture,
            quarm,
            world::{CampStatus, WorldEvent},
        };
        let server = servers::server_type(crate::client::ServerProtocol::Quarm);
        let mut features =
            Features::new(server, "Tester", eq_network_game::food::AutoEat::default());
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        world.admitted = Some(Instant::now());
        let output = testing::run_on(server.wire(), |out| -> Result<()> {
            features.admitted(&mut world, out)?;
            features.handle(
                &ClientCommand::Camp {
                    session_id: 5,
                    created: Instant::now(),
                },
                &mut world,
                out,
            )?;
            features.handle(
                &ClientCommand::SetPosture {
                    session_id: 5,
                    spawn_id: 7,
                    posture: Posture::Standing,
                    created: Instant::now(),
                },
                &mut world,
                out,
            )?;
            features.tick(Instant::now() + Duration::from_secs(40), &mut world, out)
        });
        output.result.unwrap();
        assert_eq!(
            output.sent,
            [quarm::camp(), quarm::posture(7, Posture::Standing).unwrap()]
        );
        assert!(output.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Camp(CampStatus::Abandoned { .. }))
        )));
        assert!(world.exit().is_none());
    }
    #[test]
    #[allow(clippy::too_many_lines)] // One policy/decode/consent/admission regression flow.
    fn p99_group_policy_dispatch_preserves_consent_admission_and_native_modes() {
        use super::super::feature::testing;
        use crate::client::ServerProtocol;
        use eq_network_game::{
            group::{self, GroupUpdate},
            world::{Capability, WorldEvent},
        };
        let server = servers::server_type(ServerProtocol::Project1999);
        let mut features =
            Features::new(server, "Tester", eq_network_game::food::AutoEat::default());
        assert!(features.capabilities().contains(&Capability::Grouping));
        let mut world = World::new(5);
        let mut player = testing::player(7);
        player.name = "Tester".into();
        world.player.admit(player);
        world.admitted = Some(Instant::now());
        let invite_packet = group::invite("Tester", "Leader").unwrap();
        let offered = testing::run_on(server.wire(), |out| {
            hear(
                server
                    .wire()
                    .messages(0x12d6, &invite_packet.body, server.item_rules()),
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        offered.result.unwrap();
        assert_eq!(offered.sent, []);
        let invitation_id = offered.events.iter().find_map(|event| match event {
            ClientEvent::World(WorldEvent::Group(GroupUpdate::Invited { id, .. })) => *id,
            _ => None,
        });
        assert!(invitation_id.is_some());
        let accept = ClientCommand::FollowGroup {
            session_id: 5,
            inviter: "Leader".into(),
            invitation_id,
        };
        assert_eq!(
            features
                .provided
                .iter()
                .filter(|feature| feature.owns(&accept))
                .count(),
            1
        );
        let stale = ClientCommand::FollowGroup {
            session_id: 4,
            inviter: "Leader".into(),
            invitation_id,
        };
        let stale_out = testing::run_on(server.wire(), |out| {
            let reason = features
                .refusal(&stale, &world, Instant::now())
                .expect("old admission is refused");
            actions::refuse(&stale, reason, out.log)
        });
        stale_out.result.unwrap();
        assert_eq!(stale_out.sent, []);
        assert_eq!(features.refusal(&accept, &world, Instant::now()), None);
        let answered = testing::run_on(server.wire(), |out| {
            features.handle(&accept, &mut world, out)
        });
        assert!(answered.result.unwrap());
        assert_eq!(answered.sent.len(), 1);
        assert_eq!(answered.sent[0].opcode, 0x42c9);
        assert!(!answered.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::Group(
                GroupUpdate::Members { .. } | GroupUpdate::Formed { .. }
            ))
        )));
        let mut roster = vec![0; 452];
        roster[..4].copy_from_slice(&7u32.to_le_bytes());
        roster[4..10].copy_from_slice(b"Tester");
        roster[68..74].copy_from_slice(b"Leader");
        roster[388..394].copy_from_slice(b"Leader");
        let members = testing::run_on(server.wire(), |out| {
            hear(
                server.wire().messages(0x2dd6, &roster, server.item_rules()),
                &mut features,
                &mut world,
                out,
                true,
            )
        });
        members.result.unwrap();
        assert_eq!(members.sent, []);
        let invite = ClientCommand::InviteToGroup {
            session_id: 5,
            name: "Other".into(),
        };
        let nonleader = testing::run_on(server.wire(), |out| {
            features.handle(&invite, &mut world, out)
        });
        assert!(nonleader.result.unwrap());
        assert_eq!(nonleader.sent, []);
        assert!(nonleader.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::GroupRefused {
                string_id: Some(12267),
                ..
            })
        )));
        // A new admission builds a new owner; old consent cannot carry over.
        let mut next = Features::new(server, "Tester", eq_network_game::food::AutoEat::default());
        let mut next_world = World::new(6);
        let mut player = testing::player(8);
        player.name = "Tester".into();
        next_world.player.admit(player);
        next_world.admitted = Some(Instant::now());
        assert!(next.refusal(&accept, &next_world, Instant::now()).is_some());
        let unassigned = ClientCommand::FollowGroup {
            session_id: 6,
            inviter: "Leader".into(),
            invitation_id: None,
        };
        let no_offer = testing::run_on(server.wire(), |out| {
            next.handle(&unassigned, &mut next_world, out)
        });
        assert!(no_offer.result.unwrap());
        assert_eq!(no_offer.sent, []);
        assert!(no_offer
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::GroupRefused { .. }))));
        let malformed = testing::run_on(server.wire(), |out| {
            hear(
                server
                    .wire()
                    .messages(0x1b48, &[0; 128], server.item_rules()),
                &mut next,
                &mut next_world,
                out,
                true,
            )
        });
        malformed.result.unwrap();
        assert_eq!(malformed.sent, []);
    }
}

#[cfg(test)]
mod eqmac_tests;

#[cfg(test)]
mod world_object_tests;

#[cfg(test)]
mod quarm_tests;

#[cfg(test)]
mod ability_tests;

#[cfg(test)]
mod quarm_inventory_tests;

#[cfg(test)]
mod item_move_tests;

#[cfg(test)]
mod reading_tests;

#[cfg(test)]
mod combat_tests;

#[cfg(test)]
mod loot_tests;

#[cfg(test)]
mod diagnostic_tests;

#[cfg(test)]
mod target_tests;

#[cfg(test)]
mod pet_tests;

#[cfg(test)]
mod manual_combine_tests;

#[cfg(test)]
mod terminal_tests;

#[cfg(test)]
mod admission_tests;

#[cfg(test)]
mod command_batch_tests;

#[cfg(test)]
mod stun_tests;

#[cfg(test)]
mod buff_tests;
