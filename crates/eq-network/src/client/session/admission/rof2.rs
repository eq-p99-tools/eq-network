//! `RoF2`'s zone entry, as stock `EQEmu`'s zone runs it, built from its
//! public source and inferred for `RoF2` (6.501).
//!
//! The client enters its character (`OP_ZoneEntry`). For `RoF` and later
//! clients the zone answers with the profile, the player's spawn and the
//! rest of its greeting and then describes itself (`OP_NewZone`) unasked;
//! the client asks for the zone's spawns (`OP_ReqClientSpawn`), the zone
//! sends them and ends with `OP_WorldObjectsSent`, and the client's
//! `OP_ClientReady` completes the entry (`zone/client_packet.cpp`:
//! `Client::Handle_Connect_OP_ZoneEntry`,
//! `Client::Handle_Connect_OP_ReqClientSpawn`,
//! `Client::Handle_Connect_OP_ClientReady`; opcodes from
//! `utils/patches/patch_RoF2.conf`).
//!
//! The first profile and own spawn before the description are kept, and
//! the entry completes with the player read from them as stock `EQEmu`
//! writes them ([`eq_network_game::rof2_player`], 6.502); without either,
//! or with one refused, it completes with no player. Nothing is sent that
//! stock `EQEmu` doesn't need; what the `RoF2` program sends during the
//! entry, and which message prompts its spawn request and its ready (stock
//! `EQEmu` sending the description unasked, and its own ready first, is the
//! server's side only), are not read yet (6.501).
//!
//! The entry sends no server filters: `RoF2`'s filter packet is not built.
use super::{
    super::put_string, Admission, ClientEvent, ConnectionStage, ConnectionState, Context,
    Handshake, HandshakeComplete, Result, World, Zone,
};
use anyhow::{anyhow, ensure};
use eq_network_game::rof2_player::{self, OWN_SPAWN_OPCODE, PROFILE_OPCODE};
use eq_network_transport::Application;

/// `OP_ZoneEntry`: the client's entry (inferred, 6.501).
const ENTRY: u16 = 0x5089;
/// `OP_NewZone`, the zone's description (inferred, 6.501).
const NEW_ZONE: u16 = 0x1795;
/// `OP_ReqClientSpawn` (inferred, 6.501).
const SPAWN_REQUEST: u16 = 0x35fa;
/// `OP_WorldObjectsSent`, the last of the zone's answer to the spawn
/// request (inferred, 6.501).
const OBJECTS_SENT: u16 = 0x5ae2;
/// `OP_ClientReady` (inferred, 6.501).
const READY: u16 = 0x345d;

/// The entry's length: a word, the name's 64 bytes, two words
/// (`rof2_structs.h`, `ClientZoneEntry_Struct`; `rof2.cpp`,
/// `DECODE(OP_ZoneEntry)`, which takes exactly this length).
const ENTRY_LENGTH: usize = 76;
/// The description's length (`rof2_structs.h`, `NewZone_Struct`; `rof2.cpp`,
/// `ENCODE(OP_NewZone)`).
const NEW_ZONE_LENGTH: usize = 948;
/// Where the description keeps the zone's short name, and its field's
/// length.
const SHORT_NAME: usize = 64;
const SHORT_NAME_LENGTH: usize = 128;

/// How far `RoF2`'s entry has come.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    /// The entry was sent; the zone's greeting and description come next.
    Entering,
    /// The zone described itself and the client asked for its spawns.
    Requested,
    /// The client said it is ready.
    Complete,
}

/// `RoF2`'s entry in progress.
pub(in crate::client) struct Rof2Admission {
    stage: Stage,
    zone: Option<Zone>,
    /// The first profile before the description.
    profile: Option<Vec<u8>>,
    /// The first own spawn before the description.
    spawn: Option<Vec<u8>>,
    /// The profile's heading scale.
    revolution: f32,
}

/// The entry's body: the character's name in its 64-byte field. The word
/// before it and the two after are zero: what the `RoF2` program puts
/// there is not read (6.501), and stock `EQEmu` reads only the name.
///
/// # Errors
/// Refuses an empty name, one holding a NUL, and one too long for its field
/// with its NUL, rather than cut it.
fn entry(character: &str) -> Result<[u8; ENTRY_LENGTH]> {
    ensure!(!character.is_empty(), "no character to enter");
    let mut body = [0; ENTRY_LENGTH];
    put_string(&mut body[4..68], character)?;
    Ok(body)
}

/// The zone's short name from its description.
///
/// # Errors
/// Refuses any length but [`NEW_ZONE_LENGTH`], and a short name that is
/// empty, not ASCII lowercase letters, digits and underscores, or not
/// terminated inside its field.
fn short_name(body: &[u8]) -> Result<String> {
    ensure!(
        body.len() == NEW_ZONE_LENGTH,
        "invalid RoF2 zone description length"
    );
    let field = &body[SHORT_NAME..SHORT_NAME + SHORT_NAME_LENGTH];
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .context("unterminated RoF2 zone name")?;
    let name = &field[..end];
    ensure!(
        !name.is_empty()
            && name
                .iter()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_'),
        "invalid RoF2 zone name"
    );
    Ok(String::from_utf8_lossy(name).into_owned())
}

impl Rof2Admission {
    /// Asks the zone to admit the character.
    ///
    /// # Errors
    /// Returns an error when the name doesn't fit or the entry cannot be
    /// sent.
    pub(in crate::client) fn start(
        character: &str,
        revolution: f32,
        handshake: &mut Handshake<'_, '_>,
    ) -> Result<Self> {
        handshake.session.send(ENTRY, &entry(character)?)?;
        handshake
            .log
            .send(ClientEvent::Progress(ConnectionStage::LoadingCharacter))?;
        Ok(Self {
            stage: Stage::Entering,
            zone: None,
            profile: None,
            spawn: None,
            revolution,
        })
    }
}

impl Admission for Rof2Admission {
    fn stage(&self) -> &'static str {
        match self.stage {
            Stage::Entering => "Entering",
            Stage::Requested => "Requested",
            Stage::Complete => "Complete",
        }
    }

    fn read(
        &mut self,
        packet: &mut Application,
        world: &mut World,
        handshake: &mut Handshake<'_, '_>,
    ) -> Result<Option<HandshakeComplete>> {
        match (self.stage, packet.opcode) {
            (Stage::Entering, PROFILE_OPCODE) if self.profile.is_none() => {
                self.profile = Some(packet.body.clone());
                Ok(None)
            }
            (Stage::Entering, OWN_SPAWN_OPCODE) if self.spawn.is_none() => {
                self.spawn = Some(packet.body.clone());
                Ok(None)
            }
            (_, PROFILE_OPCODE | OWN_SPAWN_OPCODE) => {
                // A second one, or one after the description, changes
                // nothing.
                handshake.log.diagnostic(format!(
                    "RoF2 zone sent 0x{:04x} ({} bytes) again or after its description, left unread",
                    packet.opcode,
                    packet.body.len()
                ))?;
                Ok(None)
            }
            (Stage::Entering, NEW_ZONE) => {
                let name = short_name(&packet.body)?;
                handshake.log.name_zone(&name);
                handshake.log.status(
                    ConnectionState::Zoning,
                    world.packets,
                    Some(handshake.session.last_received_seconds()),
                )?;
                // Far clip, sky and the rest of the description are not
                // read yet.
                self.zone = Some(Zone {
                    name,
                    far_clip: None,
                    sky: None,
                    description: None,
                });
                handshake.session.send(SPAWN_REQUEST, &[])?;
                self.stage = Stage::Requested;
                handshake
                    .log
                    .send(ClientEvent::Progress(ConnectionStage::EnteringWorld))?;
                Ok(None)
            }
            (Stage::Requested, OBJECTS_SENT) => {
                handshake.session.send(READY, &[])?;
                self.stage = Stage::Complete;
                let player = match (self.profile.take(), self.spawn.take()) {
                    (Some(profile), Some(spawn)) => {
                        rof2_player::player(&profile, &spawn, self.revolution)
                    }
                    _ => Err(anyhow!("the RoF2 profile or own spawn did not come")),
                };
                Ok(self
                    .zone
                    .take()
                    .map(|zone| HandshakeComplete { player, zone }))
            }
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{ClientConfig, Events};
    use crate::world::PlayerState;
    use eq_network_game::rof2_player::samples::{profile_body, spawn_body, Sample};
    use eq_network_transport::Transport;

    /// A connection that remembers what the client sent, bodies and all.
    #[derive(Default)]
    struct Wire(Vec<(u16, Vec<u8>)>);

    impl Transport for Wire {
        fn send(&mut self, opcode: u16, body: &[u8]) -> Result<()> {
            self.0.push((opcode, body.to_vec()));
            Ok(())
        }

        fn receive(&mut self) -> Result<Option<Application>> {
            Ok(None)
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }

        fn last_received_seconds(&self) -> u64 {
            0
        }
    }

    fn description(name: &[u8]) -> Vec<u8> {
        let mut body = vec![0; NEW_ZONE_LENGTH];
        body[..6].copy_from_slice(b"Tester");
        body[SHORT_NAME..SHORT_NAME + name.len()].copy_from_slice(name);
        body
    }

    /// Runs an entry: `before` reaches the admission ahead of the
    /// description, `after` between it and the objects. What it completed
    /// with, and everything the client sent.
    fn run(
        before: &[(u16, Vec<u8>)],
        after: &[(u16, Vec<u8>)],
    ) -> (Result<PlayerState>, Vec<(u16, Vec<u8>)>) {
        let config = ClientConfig::new("ACCOUNT", "PASSWORD", "Test Server", "Tester");
        let mut handler = |_| Ok(());
        let mut log = Events::new(&config, &mut handler);
        let mut wire = Wire::default();
        let mut world = World::new(5);
        let mut shield = None;
        let mut checksums = [];
        let mut handshake = Handshake {
            session: &mut wire,
            shield: &mut shield,
            key: &[],
            checksums: &mut checksums,
            // RoF2's entry sends no filters: its filter packet is not built.
            filters: &eq_network_game::filters::ServerFilters::default(),
            log: &mut log,
        };
        let mut admission = Rof2Admission::start("Tester", 512.0, &mut handshake).unwrap();
        let mut read = |admission: &mut Rof2Admission, opcode, body| {
            admission
                .read(
                    &mut Application { opcode, body },
                    &mut world,
                    &mut handshake,
                )
                .unwrap()
        };
        // Objects before the description finish nothing.
        assert!(read(&mut admission, OBJECTS_SENT, Vec::new()).is_none());
        for (opcode, body) in before {
            assert!(read(&mut admission, *opcode, body.clone()).is_none());
        }
        assert_eq!(admission.stage(), "Entering");
        assert!(read(&mut admission, NEW_ZONE, description(b"qeynos2")).is_none());
        assert_eq!(admission.stage(), "Requested");
        // A second description asks for nothing more.
        assert!(read(&mut admission, NEW_ZONE, description(b"qeynos2")).is_none());
        for (opcode, body) in after {
            assert!(read(&mut admission, *opcode, body.clone()).is_none());
        }
        let HandshakeComplete { player, zone } =
            read(&mut admission, OBJECTS_SENT, Vec::new()).unwrap();
        assert_eq!(zone.name, "qeynos2");
        assert_eq!(admission.stage(), "Complete");
        assert!(read(&mut admission, OBJECTS_SENT, Vec::new()).is_none());
        drop(read);
        (player, wire.0)
    }

    fn profile() -> (u16, Vec<u8>) {
        (PROFILE_OPCODE, profile_body(&Sample::default()).bytes)
    }

    fn spawn() -> (u16, Vec<u8>) {
        (OWN_SPAWN_OPCODE, spawn_body(&Sample::default()))
    }

    #[test]
    fn rof2s_entry_asks_for_spawns_after_the_description_and_is_ready_after_the_objects() {
        // A second profile and spawn, before the description or after it,
        // change nothing.
        let (player, sent) = run(
            &[
                profile(),
                spawn(),
                (PROFILE_OPCODE, vec![0; 30]),
                (OWN_SPAWN_OPCODE, vec![0; 3]),
            ],
            &[
                (PROFILE_OPCODE, vec![0; 30]),
                (OWN_SPAWN_OPCODE, vec![0; 3]),
            ],
        );
        let player = player.unwrap();
        assert_eq!((player.name.as_str(), player.spawn_id), ("Tester", 4321));
        let opcodes: Vec<u16> = sent.iter().map(|(opcode, _)| *opcode).collect();
        assert_eq!(opcodes, [ENTRY, SPAWN_REQUEST, READY]);
        let (_, body) = &sent[0];
        assert_eq!(body.len(), ENTRY_LENGTH);
        assert_eq!(&body[..11], b"\0\0\0\0Tester\0");
        assert!(body[11..].iter().all(|byte| *byte == 0));
        assert!(sent[1].1.is_empty() && sent[2].1.is_empty());
    }

    #[test]
    fn rof2s_entry_completes_with_no_player_without_a_true_profile_and_spawn() {
        let (_, garbage) = profile();
        let mut garbage = garbage;
        // The bind count.
        garbage[24] ^= 1;
        for (before, after) in [
            (vec![profile()], vec![]),
            (vec![spawn()], vec![]),
            (vec![], vec![profile(), spawn()]),
            (vec![(PROFILE_OPCODE, garbage), spawn()], vec![]),
        ] {
            let (player, sent) = run(&before, &after);
            assert!(player.is_err());
            assert_eq!(sent.len(), 3);
        }
    }

    #[test]
    fn rof2s_description_refuses_what_it_cannot_trust() {
        assert_eq!(short_name(&description(b"qeynos2")).unwrap(), "qeynos2");
        let whole = description(b"qeynos2");
        let mut long = whole.clone();
        long.push(0);
        assert!(short_name(&whole[..NEW_ZONE_LENGTH - 1]).is_err());
        assert!(short_name(&long).is_err());
        let mut unterminated = whole.clone();
        unterminated[SHORT_NAME..SHORT_NAME + SHORT_NAME_LENGTH].fill(b'a');
        assert!(short_name(&unterminated).is_err());
        for name in [&b""[..], b"../qeynos", b"Qeynos", b"qey nos"] {
            assert!(short_name(&description(name)).is_err(), "{name:?}");
        }
    }

    #[test]
    fn rof2s_entry_refuses_a_name_it_would_have_to_cut() {
        assert!(entry(&"A".repeat(63)).is_ok());
        for name in ["", "Tes\0ter", &"A".repeat(64)] {
            assert!(entry(name).is_err(), "{name:?}");
        }
    }
}
