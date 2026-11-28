//! The character's server filters: kept for every zone-in, from the
//! character list or a change mid-zone, and sent at once where the server
//! type reads a change then ([`eq_network_game::filters`]).
//!
//! A set is checked against the generation's packet when it is given, so a
//! kept set always encodes at zone-in.

use super::{actions, servers::ServerType, ClientCommand, ClientEvent, Events};
use anyhow::Result;
use eq_network_game::{filters, world::WorldEvent, GameDialect};
use eq_network_transport::Transport;
use std::cell::RefCell;

/// Takes the current character's filters at the character list, for its
/// first zone-in. Returns whether the command was this one.
pub(in crate::client) fn choose(
    command: &ClientCommand,
    selection_id: u64,
    dialect: GameDialect,
    kept: &RefCell<filters::ServerFilters>,
    log: &mut Events<'_>,
) -> Result<bool> {
    let ClientCommand::ChooseServerFilters {
        selection_id: requested,
        filters,
    } = command
    else {
        return Ok(false);
    };
    if *requested != selection_id {
        actions::refuse(
            command,
            "The command belongs to another character list",
            log,
        )?;
    } else if let Err(error) = filters.check(dialect) {
        actions::refuse(command, &error.to_string(), log)?;
    } else {
        kept.replace(filters.clone());
        log.send(ClientEvent::World(WorldEvent::ServerFiltersKept {
            session_id: None,
            sent: false,
        }))?;
    }
    Ok(true)
}

/// Takes a change of the current character's filters mid-zone: kept for
/// every later zone-in, and sent at once where the server type reads it.
/// Returns whether the command was this one.
pub(super) fn set(
    server: &dyn ServerType,
    command: &ClientCommand,
    session_id: u64,
    kept: &RefCell<filters::ServerFilters>,
    session: &mut dyn Transport,
    log: &mut Events<'_>,
) -> Result<bool> {
    let ClientCommand::SetServerFilters {
        session_id: requested,
        filters,
    } = command
    else {
        return Ok(false);
    };
    if *requested != session_id {
        actions::refuse(
            command,
            "The command belongs to another zone admission",
            log,
        )?;
        return Ok(true);
    }
    let packet = match filters::encode(server.wire().dialect(), filters) {
        Ok(packet) => packet,
        Err(error) => {
            actions::refuse(command, &error.to_string(), log)?;
            return Ok(true);
        }
    };
    kept.replace(filters.clone());
    let sent = server.server_filters_at_once();
    if sent {
        session.send(packet.opcode, &packet.body)?;
    }
    log.send(ClientEvent::World(WorldEvent::ServerFiltersKept {
        session_id: Some(session_id),
        sent,
    }))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{ClientConfig, ServerProtocol};
    use eq_network_game::filters::{FilterCategory, FilterMode, ServerFilters};

    /// A zone connection that notes what it sent.
    #[derive(Default)]
    struct Sent(Vec<(u16, Vec<u8>)>);

    impl Transport for Sent {
        fn send(&mut self, opcode: u16, body: &[u8]) -> Result<()> {
            self.0.push((opcode, body.to_vec()));
            Ok(())
        }
        fn send_unreliable(&mut self, opcode: u16, body: &[u8]) -> Result<()> {
            self.send(opcode, body)
        }
        fn receive(&mut self) -> Result<Option<eq_network_transport::Application>> {
            Ok(None)
        }
        fn last_received_seconds(&self) -> u64 {
            0
        }
        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    fn hidden_ooc() -> ServerFilters {
        let mut filters = ServerFilters::default();
        filters.set(FilterCategory::Ooc, FilterMode::Hide);
        filters
    }

    /// Runs one command through [`set`], returning the packets and events.
    fn run_set(
        protocol: ServerProtocol,
        command: &ClientCommand,
        kept: &RefCell<ServerFilters>,
    ) -> (Vec<(u16, Vec<u8>)>, Vec<ClientEvent>) {
        let config = ClientConfig::for_protocol(protocol, "u", "p", "", "");
        let mut events = Vec::new();
        let mut handler = |event| {
            events.push(event);
            Ok(())
        };
        let mut log = Events::new(&config, &mut handler);
        let mut wire = Sent::default();
        let server = super::super::servers::server_type(protocol);
        assert!(set(server, command, 7, kept, &mut wire, &mut log).unwrap());
        drop(log);
        (wire.0, events)
    }

    #[test]
    fn eqemu_sends_a_change_at_once_and_keeps_it() {
        let kept = RefCell::default();
        let command = ClientCommand::SetServerFilters {
            session_id: 7,
            filters: hidden_ooc(),
        };
        let (sent, events) = run_set(ServerProtocol::EqEmu, &command, &kept);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, 0x6563);
        assert_eq!(sent[0].1[6 * 4..7 * 4], [0, 0, 0, 0]);
        assert_eq!(*kept.borrow(), hidden_ooc());
        assert!(events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::ServerFiltersKept {
                session_id: Some(7),
                sent: true
            })
        )));
    }

    #[test]
    fn a_titanium_change_goes_at_once_on_every_server_type() {
        // The Titanium client sends the packet again on every change
        // (eqgame.exe 0x53cc29 to 0x53cd7d), whatever the server.
        let kept = RefCell::default();
        let command = ClientCommand::SetServerFilters {
            session_id: 7,
            filters: hidden_ooc(),
        };
        let (sent, _) = run_set(ServerProtocol::Project1999, &command, &kept);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, 0x6563);
    }

    #[test]
    fn elsewhere_a_change_waits_for_the_next_zone_in() {
        // The EQMac client's side is unread.
        for protocol in [ServerProtocol::Quarm, ServerProtocol::Takp] {
            let kept = RefCell::default();
            let command = ClientCommand::SetServerFilters {
                session_id: 7,
                filters: hidden_ooc(),
            };
            let (sent, events) = run_set(protocol, &command, &kept);
            assert_eq!(sent, Vec::new(), "{protocol:?}");
            assert_eq!(*kept.borrow(), hidden_ooc(), "{protocol:?}");
            assert!(events.iter().any(|event| matches!(
                event,
                ClientEvent::World(WorldEvent::ServerFiltersKept { sent: false, .. })
            )));
        }
    }

    #[test]
    fn a_set_the_packet_cannot_say_or_another_admissions_is_refused() {
        let mut bad = ServerFilters::default();
        bad.set(FilterCategory::MeleeCrits, FilterMode::GroupOnly);
        for command in [
            ClientCommand::SetServerFilters {
                session_id: 7,
                filters: bad,
            },
            ClientCommand::SetServerFilters {
                session_id: 8,
                filters: hidden_ooc(),
            },
        ] {
            let kept = RefCell::default();
            let (sent, events) = run_set(ServerProtocol::EqEmu, &command, &kept);
            assert_eq!(sent, Vec::new());
            assert_eq!(*kept.borrow(), ServerFilters::default());
            assert!(events.iter().any(|event| matches!(
                event,
                ClientEvent::World(WorldEvent::ServerFiltersRefused { .. })
            )));
        }
    }

    #[test]
    fn the_character_list_keeps_a_set_for_the_first_zone_in() {
        let config = ClientConfig::for_protocol(ServerProtocol::Project1999, "u", "p", "", "");
        let mut handler = |_| Ok(());
        let mut log = Events::new(&config, &mut handler);
        let kept = RefCell::default();
        let choose_on = |selection_id, filters, kept: &RefCell<_>, log: &mut Events<'_>| {
            choose(
                &ClientCommand::ChooseServerFilters {
                    selection_id,
                    filters,
                },
                3,
                GameDialect::Titanium,
                kept,
                log,
            )
            .unwrap()
        };
        assert!(choose_on(4, hidden_ooc(), &kept, &mut log));
        assert_eq!(*kept.borrow(), ServerFilters::default());
        assert!(choose_on(3, hidden_ooc(), &kept, &mut log));
        assert_eq!(*kept.borrow(), hidden_ooc());
        let mut bad = ServerFilters::default();
        bad.set(FilterCategory::DamageOverTime, FilterMode::MineOnly);
        assert!(choose_on(3, bad, &kept, &mut log));
        assert_eq!(*kept.borrow(), hidden_ooc());
        assert!(!choose(
            &ClientCommand::SelectCharacter {
                selection_id: 3,
                slot: 0,
                player_trade: eq_network_game::exchange::PlayerTrade::Everyone,
            },
            3,
            GameDialect::Titanium,
            &kept,
            &mut log,
        )
        .unwrap());
    }
}
