//! `EQMac`'s world stage, as the `MacPC` world server runs it: the client logs
//! in with the account's session key, the world lists the characters, the
//! client enters one, and the world hands it off to its zone. The world sends
//! no file manifest and has no protection.
use crate::{
    client::{
        selection::{Choice, Selection},
        session::{
            cstr, login::Credentials, put_string, record_chat, CharacterSession, ZoneDestination,
        },
        ClientEvent, ConnectionStage, Events,
    },
    old_transport::OldSession,
};
use anyhow::{ensure, Context, Result};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

const WORLD_LOGIN: u16 = 0x5818;
const WORLD_CHARACTER_LIST: u16 = 0x4740;
const WORLD_ENTER: u16 = 0x0180;
const WORLD_ZONE_SERVER: u16 = 0x0480;

/// `EQMac`'s world stage: authenticate to the `MacPC` world server, choose a
/// character, and follow its handoff.
pub(in crate::client::session) fn world(
    context: &CharacterSession<'_>,
    ip: &str,
    world_only: bool,
    log: &mut Events<'_>,
) -> Result<Option<ZoneDestination>> {
    let config = &context.config;
    let stop = context.stop;
    let mut session = OldSession::connect_cancellable(
        crate::client::endpoint(ip, 9000, config.local_only)?,
        stop.flag(),
    )?;
    session.send(WORLD_LOGIN, &*world_login(context.credentials)?)?;
    let mut deadline = Instant::now() + Duration::from_secs(60);
    let mut entered = false;
    let mut selection = None;
    let mut chosen = None;
    loop {
        ensure!(!stop.is_cancelled(), "shutdown requested");
        if !entered {
            if let Some(choice) = selection
                .as_ref()
                .and_then(|list: &Selection| list.poll(context.commands))
                .and_then(Choice::entered)
            {
                chosen = Some(choice);
            }
            if let Some(name) = chosen.take().filter(|_| !entered) {
                let mut enter = [0; 64];
                put_string(&mut enter, &name)?;
                session.send(WORLD_ENTER, &enter)?;
                log.character.clone_from(&name);
                chosen = Some(name);
                entered = true;
                deadline = Instant::now() + Duration::from_secs(60);
                log.send(ClientEvent::Progress(ConnectionStage::ConnectingZone))?;
            }
        }
        ensure!(
            selection.is_some() && !entered || Instant::now() < deadline,
            "world handshake timed out"
        );
        let Some(packet) = session.receive()? else {
            continue;
        };
        record_chat(config, "", &packet, log)?;
        log.diagnostic(format!(
            "Quarm world received 0x{:04x} ({} bytes)",
            packet.opcode,
            packet.body.len()
        ))?;
        match packet.opcode {
            WORLD_CHARACTER_LIST if !entered && selection.is_none() => {
                let entries =
                    eq_network_game::characters::decode(config.protocol.into(), &packet.body)?;
                log.send(ClientEvent::Progress(ConnectionStage::SelectingCharacter))?;
                if world_only {
                    session.close()?;
                    return Ok(None);
                }
                let (list, automatic) = Selection::new(entries, &config.character, log)?;
                selection = Some(list);
                chosen = automatic;
            }
            WORLD_ZONE_SERVER if entered => {
                let (host, port) = zone_destination(&packet.body)?;
                session.close()?;
                return Ok(Some(ZoneDestination {
                    character: chosen.context("zone handoff before character selection")?,
                    host,
                    port,
                    // EQMac's world sends no file manifest, and has no protection.
                    checksums: Vec::new(),
                    shield: None,
                }));
            }
            _ => (),
        }
    }
}

#[cfg(test)]
fn character_exists(body: &[u8], character: &str) -> Result<bool> {
    ensure!(body.len() >= 640, "truncated EQMac character list");
    Ok(body[..640]
        .as_chunks::<64>()
        .0
        .iter()
        .any(|name| cstr(name).eq_ignore_ascii_case(character.as_bytes())))
}

fn world_login(credentials: &Credentials) -> Result<Zeroizing<[u8; 200]>> {
    let mut body = Zeroizing::new([0; 200]);
    put_string(&mut body[..127], &credentials.account)?;
    let key_start = credentials.account.len() + 1;
    ensure!(
        key_start + credentials.key.len() <= 127,
        "TAKP session fields are too large"
    );
    body[key_start..key_start + credentials.key.len()].copy_from_slice(&credentials.key);
    Ok(body)
}

/// Read the world-to-zone endpoint advertised by an `EQMac` world server.
pub(in crate::client::session) fn zone_destination(body: &[u8]) -> Result<(String, u16)> {
    ensure!(body.len() >= 130, "truncated EQMac zone handoff");
    let host = std::str::from_utf8(cstr(&body[..128]))?.to_owned();
    // Unlike the Titanium handoff, EQMac carries this port in network byte order.
    let port = u16::from_be_bytes(body[128..130].try_into().unwrap());
    ensure!(!host.is_empty() && port != 0, "invalid EQMac zone endpoint");
    Ok((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_login_is_the_200_byte_windows_eqmac_form() {
        let credentials = Credentials {
            account: "LS#12345".into(),
            key: *b"ABCDEFGHIJ",
        };
        let body = world_login(&credentials).unwrap();
        assert_eq!(body.len(), 200);
        assert_eq!(cstr(&body[..127]), b"LS#12345");
        assert_eq!(&body[9..19], b"ABCDEFGHIJ");
        assert!(body[19..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn zone_handoff_uses_network_byte_order_for_the_port() {
        // EQMacEmu world/client.cpp serializes ntohs(GetCPort()) in this field.
        let mut body = [0; 130];
        put_string(&mut body[..128], "203.0.113.20").unwrap();
        body[128..].copy_from_slice(&7000u16.to_be_bytes());
        assert_eq!(
            zone_destination(&body).unwrap(),
            ("203.0.113.20".into(), 7000)
        );
        assert!(zone_destination(&body[..129]).is_err());
        body[128..].fill(0);
        assert!(zone_destination(&body).is_err());
    }

    #[test]
    fn character_list_uses_ten_fixed_64_byte_name_fields() {
        let mut body = vec![0; 1620];
        put_string(&mut body[3 * 64..4 * 64], "ExampleCharacter").unwrap();
        assert!(character_exists(&body, "examplecharacter").unwrap());
        assert!(!character_exists(&body, "MissingCharacter").unwrap());
        assert!(character_exists(&body[..639], "ExampleCharacter").is_err());
    }
}
