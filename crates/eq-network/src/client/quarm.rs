mod presentation;

use super::session::{
    admission::{Admission, EqMacAdmission, Handshake},
    feature::World,
    record_chat, CharacterSession,
};
use super::{ClientEvent, ConnectionStage, ConnectionState, Events};
use crate::old_transport::OldSession;
use anyhow::{bail, ensure, Result};
use eq_network_game::{
    command,
    quarm::{dll_version_reply, ZONE_CHANGE_REQUEST, ZONE_LOGOUT, ZONE_SPAWN_APPEARANCE},
};
use std::time::{Duration, Instant};

/// Complete the `EQMac` zone admission sequence and collect communications.
// The linear handshake mirrors the order required by the legacy zone server.
#[allow(clippy::too_many_lines)]
pub(in crate::client) fn zone(
    context: &CharacterSession<'_>,
    log: &mut Events<'_>,
    host: &str,
    port: u16,
) -> Result<()> {
    let config = &context.config;
    let stop = context.stop;
    let commands = context.commands;
    let mut session = OldSession::connect_cancellable(
        super::endpoint(host, port, config.local_only)?,
        stop.flag(),
    )?;
    let session_id = rand::random();
    // The shared session's EQMac handshake, which this loop runs until Quarm
    // moves onto that session; its world only serves the handshake.
    let mut world = World::new(session_id);
    let mut shield = None;
    let mut admission = EqMacAdmission::start(
        &config.character,
        &mut Handshake {
            session: &mut session,
            shield: &mut shield,
            key: &[],
            checksums: &mut [],
            log: &mut *log,
        },
    )?;

    let mut presentation = presentation::Presentation::default();
    let mut rejected_layouts = std::collections::HashSet::new();
    let connected = Instant::now();
    let mut ready = false;
    let mut zone_name = String::new();
    let mut packets = 0u64;
    let mut progress = Instant::now();
    loop {
        if stop.is_cancelled()
            || context
                .duration
                .is_some_and(|duration| connected.elapsed() >= duration)
        {
            session.close()?;
            return Ok(());
        }
        ensure!(
            ready || connected.elapsed() < Duration::from_secs(60),
            "zone admission timed out while {}",
            admission.stage()
        );
        if progress.elapsed() >= Duration::from_secs(30) {
            log.status(
                if ready && session.last_received_seconds() < 60 {
                    ConnectionState::Connected
                } else {
                    ConnectionState::Zoning
                },
                packets,
                Some(session.last_received_seconds()),
            )?;
            log.diagnostic(format!(
                "Quarm zone session: {packets} application packets, {} communication records",
                log.messages
            ))?;
            progress = Instant::now();
        }
        if ready {
            if let Some(commands) = commands {
                for command in commands.try_iter().take(64) {
                    match command::encode(config.protocol.into(), &command, &config.character) {
                        Ok(packet) => session.send(packet.opcode, &packet.body)?,
                        Err(error) => log.diagnostic(format!(
                            "Rejected invalid outbound client command: {error}"
                        ))?,
                    }
                }
            }
        }
        let Some(mut packet) = session.receive()? else {
            continue;
        };
        packets += 1;
        world.packets = packets;
        if !ready {
            log.diagnostic(format!(
                "Quarm zone received 0x{:04x} ({} bytes)",
                packet.opcode,
                packet.body.len()
            ))?;
        }
        if ready {
            match packet.opcode {
                ZONE_SPAWN_APPEARANCE => {
                    if let Some(response) = dll_version_reply(&packet.body) {
                        session.send(ZONE_SPAWN_APPEARANCE, &response)?;
                    }
                }
                ZONE_LOGOUT => bail!("server logged the character out"),
                ZONE_CHANGE_REQUEST => {
                    bail!("server requested a new zone; reconnecting through world")
                }
                _ => (),
            }
        } else if let Some((_, zone)) = admission.read(
            &mut packet,
            &mut world,
            &mut Handshake {
                session: &mut session,
                shield: &mut shield,
                key: &[],
                checksums: &mut [],
                log: &mut *log,
            },
        )? {
            zone_name = zone.name;
            ready = true;
            log.send(ClientEvent::Progress(ConnectionStage::Ready))?;
            log.status(
                ConnectionState::Connected,
                packets,
                Some(session.last_received_seconds()),
            )?;
            log.diagnostic(format!(
                "Quarm zone login sequence complete for {zone_name}; waiting for ongoing server traffic"
            ))?;
        }
        match presentation.receive(packet.opcode, &packet.body, &config.character) {
            Ok(events) => {
                for event in events {
                    log.send(ClientEvent::World(event))?;
                }
            }
            Err(error) if rejected_layouts.insert(packet.opcode) => {
                log.diagnostic(format!(
                    "Quarm world-state decode rejected 0x{:04x} ({} bytes): {error}",
                    packet.opcode,
                    packet.body.len()
                ))?;
            }
            Err(_) => (),
        }
        if ready {
            for event in presentation.enter(session_id, &zone_name) {
                log.send(ClientEvent::World(event))?;
            }
        }
        record_chat(config, &zone_name, &packet, log)?;
    }
}

#[cfg(test)]
mod zone_tests;

#[cfg(test)]
mod tests {
    use super::super::ClientConfig;

    pub(super) fn config() -> ClientConfig {
        ClientConfig::for_protocol(
            super::super::ServerProtocol::Quarm,
            "EXAMPLE_ACCOUNT",
            "EXAMPLE_PASSWORD",
            "The Project Quarm Server",
            "ExampleCharacter",
        )
    }
}
