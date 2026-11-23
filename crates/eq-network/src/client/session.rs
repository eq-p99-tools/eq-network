mod abilities;
mod actions;
pub(super) mod admission;
mod alternate_advancement;
mod bandolier;
mod bazaar;
mod bleeding;
mod boats;
mod buffs;
mod camp;
mod casting;
mod character;
pub(super) mod chat_link;
mod chat_pace;
mod clock;
mod combat;
mod confirmations;
mod control;
mod corpses;
mod coverage;
mod creation;
mod deletion;
mod dialogs;
mod doors;
mod duel;
mod entities;
mod exchange;
pub(super) mod feature;
mod groups;
mod guilds;
pub(super) mod hazards;
mod hiding;
mod inventory;
mod journal;
mod lifecycle;
pub(super) mod list_poll;
mod listing;
pub(super) mod login;
mod looting;
mod map;
mod motion;
mod objects;
mod offers;
mod pets;
mod posture;
mod raids;
mod reading;
mod reports;
mod resurrection;
mod riders;
mod saves;
pub(super) mod server_filters;
mod servers;
mod shielding;
mod sight;
mod socials;
pub(super) mod spell_tables;
mod spellbook;
mod talk;
mod targeting;
mod ticks;
mod tracking;
mod tradeskills;
mod training;
mod transfers;
mod who;
mod wire;
mod world;
pub(super) mod zone;

use super::{
    CancellationToken, ClientCommand, ClientConfig, ClientEvent, ClientIdentity, ConnectionStage,
    ConnectionState, DecodeError, Events, GapKind, GapStage, RecordEvent, RunOptions,
    ServerProtocol,
};
use crate::{assets::Assets, transport::Application};
use anyhow::{bail, ensure, Context, Result};
use eq_network_game::zoning;
use servers::Shield;
use std::{
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

use login::Credentials;

/// The selected server type owns the host's chat budget for Titanium links.
pub(super) fn titanium_chat_link_expansion(
    protocol: ServerProtocol,
) -> Option<eq_network_game::chat::LinkExpansion> {
    servers::server_type(protocol).titanium_chat_link_expansion()
}

/// The selected server type owns the host's consider presentation policy.
pub(super) fn consider_npcs_by_level(protocol: ServerProtocol) -> bool {
    servers::server_type(protocol).consider_npcs_by_level()
}

/// The selected server type owns the host and worker's placement policy.
pub(super) fn inventory_move_rules(
    protocol: ServerProtocol,
) -> Option<eq_network_game::inventory::MoveRules> {
    servers::server_type(protocol).inventory_move_rules()
}

/// Run one complete login/world/zone attempt with fresh session credentials,
/// each stage the way the configured server's type runs it.
pub(super) fn run(
    config: &ClientConfig,
    identity: &ClientIdentity,
    assets: &Assets,
    stop: &CancellationToken,
    options: &RunOptions,
    commands: Option<&Receiver<ClientCommand>>,
    log: &mut Events<'_>,
) -> Result<()> {
    ensure!(!stop.is_cancelled(), "shutdown requested");
    let server = servers::server_type(config.protocol);
    let (credentials, ip) = staged(log, GapStage::Login, |log| {
        server.wire().login(config, stop, commands, log)
    })?;
    let mut context = CharacterSession::new(
        config,
        identity,
        &credentials,
        stop,
        options.zone_duration,
        commands,
    );
    let mut destination = staged(log, GapStage::World, |log| {
        server
            .wire()
            .world(&context, assets, &ip, (options.world_only, false), log)
    })?;
    while let Some(mut next_zone) = destination {
        context.config.character.clone_from(&next_zone.character);
        let exit = staged(log, GapStage::Zone, |log| {
            server.zone(
                &context,
                &mut next_zone.shield,
                (&next_zone.host, next_zone.port),
                next_zone.checksums,
                log,
            )
        })?;
        context.on_zone_exit(&exit);
        match exit {
            ZoneExit::Stopped => return Ok(()),
            ZoneExit::World => {
                destination = staged(log, GapStage::World, |log| {
                    server
                        .wire()
                        .world(&context, assets, &ip, (false, true), log)
                })?;
            }
            ZoneExit::CharacterSelect => {
                destination = staged(log, GapStage::World, |log| {
                    server
                        .wire()
                        .world(&context, assets, &ip, (false, false), log)
                })?;
            }
            ZoneExit::Direct(packet) => {
                let (host, port, checksums) =
                    server
                        .wire()
                        .handoff(next_zone.shield.as_deref(), &packet, assets, log)?;
                destination = Some(ZoneDestination {
                    host,
                    port,
                    checksums,
                    shield: next_zone.shield,
                    character: context.config.character.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Runs one stage of the session, then tells what it still had to tell of
/// what it left unread, whether it ended well or not; the stage's own error
/// wins over the host's.
fn staged<T>(
    log: &mut Events<'_>,
    stage: GapStage,
    run: impl FnOnce(&mut Events<'_>) -> Result<T>,
) -> Result<T> {
    log.begin_stage(stage)?;
    let ran = run(log);
    let ended = log.end_stage();
    let ran = ran?;
    ended?;
    Ok(ran)
}

/// What a character's stay in the world and its zones works with.
pub(in crate::client) struct CharacterSession<'a> {
    pub(in crate::client) config: ClientConfig,
    pub(in crate::client) identity: &'a ClientIdentity,
    pub(in crate::client) credentials: &'a Credentials,
    pub(in crate::client) stop: &'a CancellationToken,
    pub(in crate::client) duration: Option<Duration>,
    pub(in crate::client) commands: Option<&'a Receiver<ClientCommand>>,
    /// The second the session last told the world what machine it runs on,
    /// or zero when it never has: the official gatherer rereads its saved
    /// stamp on every pass, so a zone crossing back through the world must
    /// not report twice inside the week while the host is still saving the
    /// first report's time.
    pub(in crate::client) sysinfo_reported_at: std::cell::Cell<u64>,
    /// Current character's opt-in; zone crossings retain it.
    pub(in crate::client) answer_list_polls: std::cell::Cell<bool>,
    /// Current character's server filters; zone crossings retain them.
    pub(in crate::client) server_filters:
        std::cell::RefCell<eq_network_game::filters::ServerFilters>,
    /// What the riding wire keeps across zones, as the program's globals
    /// do (reads R36 §4, R36c).
    pub(in crate::client) riding: std::sync::Arc<riders::Riding>,
    /// The character's chat server link, opened from the world's offer
    /// where the server type opens one, kept across zones.
    pub(in crate::client) chat: std::sync::Arc<chat_link::ChatLink>,
    /// The character's one pace for all chat, kept across zones (6.523).
    pub(in crate::client) chat_pace: chat_pace::SharedPace,
}

impl<'a> CharacterSession<'a> {
    fn new(
        config: &ClientConfig,
        identity: &'a ClientIdentity,
        credentials: &'a Credentials,
        stop: &'a CancellationToken,
        duration: Option<Duration>,
        commands: Option<&'a Receiver<ClientCommand>>,
    ) -> Self {
        Self {
            config: config.clone(),
            identity,
            credentials,
            stop,
            duration,
            commands,
            sysinfo_reported_at: std::cell::Cell::new(0),
            answer_list_polls: std::cell::Cell::new(config.answer_list_polls),
            server_filters: std::cell::RefCell::new(config.server_filters.clone()),
            riding: std::sync::Arc::default(),
            chat: std::sync::Arc::default(),
            chat_pace: chat_pace::SharedPace::default(),
        }
    }

    fn on_zone_exit(&mut self, exit: &ZoneExit) {
        if matches!(exit, ZoneExit::CharacterSelect) {
            // Show the list and forget the departing character's runtime choice.
            self.config.character.clear();
            self.answer_list_polls.set(self.config.answer_list_polls);
            self.server_filters
                .replace(self.config.server_filters.clone());
            self.riding = std::sync::Arc::default();
            // A camp or the character list ends the character's chat link.
            self.chat.close();
        }
    }
}

/// Where the world sends the character next.
pub(in crate::client) struct ZoneDestination {
    pub(in crate::client) character: String,
    pub(in crate::client) host: String,
    pub(in crate::client) port: u16,
    pub(in crate::client) checksums: Vec<u8>,
    /// The protection the world connection started, kept across zones.
    pub(in crate::client) shield: Option<Box<dyn Shield>>,
}

/// How a zone session ends.
pub(in crate::client) enum ZoneExit {
    Stopped,
    World,
    Direct(Vec<u8>),
    CharacterSelect,
}

/// Records the communication a packet carries, as the server's client
/// generation reads it; an unreadable one is recorded as such, and counted
/// as a gap. Says whether the packet was a communication, read or not.
///
/// # Errors
/// Returns an error when the host's record handler fails.
pub(in crate::client) fn record_chat(
    config: &ClientConfig,
    zone: &str,
    packet: &Application,
    log: &mut Events<'_>,
) -> Result<bool> {
    match servers::server_type(config.protocol).wire().chat(
        packet.opcode,
        &packet.body,
        config.include_raw,
    ) {
        Ok(Some(event)) => log.record(zone, RecordEvent::Chat(event)).map(|()| true),
        Ok(None) => Ok(false),
        Err(error) => {
            log.gap(packet.opcode, None, &packet.body, chat_unreadable(&error))?;
            log.record(
                zone,
                RecordEvent::DecodeError(DecodeError {
                    kind: "decode_error",
                    opcode: packet.opcode,
                    payload_hex: hex::encode(&packet.body),
                    error: error.to_string(),
                }),
            )?;
            Ok(true)
        }
    }
}

/// The gap a communication the generation's reader could not read leaves.
pub(in crate::client) fn chat_unreadable(error: &anyhow::Error) -> GapKind {
    GapKind::Unreadable {
        part: "communication".to_owned(),
        error: error.to_string(),
    }
}

fn put_string(destination: &mut [u8], value: &str) -> Result<()> {
    ensure!(
        value.len() < destination.len() && !value.contains('\0'),
        "string exceeds protocol field size"
    );
    destination[..value.len()].copy_from_slice(value.as_bytes());
    Ok(())
}
fn cstr(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.iter().position(|&v| v == 0).unwrap_or(bytes.len())]
}
fn le32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}
