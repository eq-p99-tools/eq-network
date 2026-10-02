//! The `EQMac` zone handshake, as Project Quarm's zones run it: the client
//! sends its data rate and its entry; the zone sends the player's profile,
//! their own spawn and the weather, and the client asks for the zone, then
//! its spawns; the zone reports experience, which the client echoes; and once
//! the zone says the avatar is ready, the client sends its filters, announces
//! its DLL version and sends its first position. The zone names the player's
//! spawn in an appearance of its own, and the player is admitted once that,
//! the own spawn and the ready avatar are all in.
use super::{Admission, Handshake, Zone};
use crate::client::session::{feature::World, put_string, ClientEvent, ConnectionStage};
use crate::world::PlayerState;
use anyhow::{bail, ensure, Result};
use eq_network_game::quarm::{
    self, ZONE_AVATAR_READY, ZONE_CHANGE_REQUEST, ZONE_CLIENT_UPDATE, ZONE_DATA_RATE, ZONE_ENTRY,
    ZONE_EXPERIENCE_READY, ZONE_LOGOUT, ZONE_NEW, ZONE_PLAYER_PROFILE, ZONE_REQUEST_NEW,
    ZONE_REQUEST_SPAWNS, ZONE_SERVER_FILTER, ZONE_SPAWN_APPEARANCE, ZONE_WEATHER,
};
use eq_network_transport::Application;
use std::time::Instant;

/// How far the `EQMac` handshake has come; it only moves forward.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
enum Step {
    /// The entry was sent.
    Entering,
    /// The weather came and the client asked for the zone.
    Requested,
    /// The zone described itself and the client asked for its spawns.
    Described,
    /// The client echoed the zone's experience report.
    Answered,
    /// The avatar is ready and the client sent its first position.
    Ready,
    /// The player is admitted.
    Admitted,
}

/// The `EQMac` handshake in progress.
pub(in crate::client::session) struct EqMacAdmission {
    character: String,
    profile: Option<Vec<u8>>,
    own_spawn: Option<quarm::OwnSpawn>,
    /// The player's spawn, as the zone's appearance named it.
    assigned_id: Option<u16>,
    zone: Option<String>,
    step: Step,
}

impl EqMacAdmission {
    /// Sends the client's data rate and asks the zone to admit the character.
    ///
    /// # Errors
    /// Returns an error when the entry cannot be sent.
    pub(in crate::client::session) fn start(
        character: &str,
        handshake: &mut Handshake<'_, '_>,
    ) -> Result<Self> {
        handshake
            .session
            .send(ZONE_DATA_RATE, &10.0f32.to_le_bytes())?;
        let mut entry = [0; 68];
        put_string(&mut entry[4..], character)?;
        handshake.session.send(ZONE_ENTRY, &entry)?;
        handshake
            .log
            .send(ClientEvent::Progress(ConnectionStage::LoadingCharacter))?;
        Ok(Self {
            character: character.to_owned(),
            profile: None,
            own_spawn: None,
            assigned_id: None,
            zone: None,
            step: Step::Entering,
        })
    }

    /// The admitted player: their profile, placed where their own spawn
    /// stands, under the spawn the zone named.
    fn player(&self, spawn_id: u16, spawn: &quarm::OwnSpawn) -> Result<PlayerState> {
        let mut player =
            quarm::profile(self.profile.as_deref().unwrap_or_default(), &self.character)?;
        player.spawn_id = spawn_id;
        player.position = spawn.position;
        player.size = spawn.size;
        player.walk_speed = spawn.walk_speed;
        player.run_speed = spawn.run_speed;
        Ok(player)
    }
}

impl Admission for EqMacAdmission {
    fn stage(&self) -> &'static str {
        match self.step {
            Step::Entering => "Entering",
            Step::Requested => "Requested",
            Step::Described => "Described",
            Step::Answered => "Answered",
            Step::Ready => "Ready",
            Step::Admitted => "Admitted",
        }
    }

    fn read(
        &mut self,
        packet: &mut Application,
        world: &mut World,
        handshake: &mut Handshake<'_, '_>,
    ) -> Result<Option<(Result<PlayerState>, Zone)>> {
        if self.step == Step::Admitted {
            return Ok(None);
        }
        match packet.opcode {
            ZONE_PLAYER_PROFILE => self.profile = Some(packet.body.clone()),
            ZONE_ENTRY if self.own_spawn.is_none() => {
                let spawn = quarm::own_spawn(&packet.body, &self.character)?;
                // The player stands where the zone put them until they can move.
                world.body.place(spawn.position);
                self.own_spawn = Some(spawn);
            }
            ZONE_SPAWN_APPEARANCE => {
                if let Some(reply) = quarm::dll_version_reply(&packet.body) {
                    handshake.session.send(ZONE_SPAWN_APPEARANCE, &reply)?;
                }
                // Other appearances, malformed ones too, are the features' news.
                if let Ok(Some(id)) = quarm::assigned_id(&packet.body) {
                    world.body.own(id);
                    world.own_spawn = Some(id);
                    self.assigned_id = Some(id);
                }
            }
            ZONE_WEATHER if self.step == Step::Entering => {
                handshake.session.send(ZONE_REQUEST_NEW, &[])?;
                self.step = Step::Requested;
            }
            ZONE_NEW if self.zone.is_none() => {
                ensure!(packet.body.len() >= 96, "truncated EQMac zone description");
                let name =
                    String::from_utf8_lossy(crate::client::session::cstr(&packet.body[64..96]))
                        .into_owned();
                handshake.log.zone.clone_from(&name);
                handshake.log.status(
                    crate::client::ConnectionState::Zoning,
                    world.packets,
                    Some(handshake.session.last_received_seconds()),
                )?;
                handshake.session.send(ZONE_REQUEST_SPAWNS, &[])?;
                self.zone = Some(name);
                self.step = self.step.max(Step::Described);
            }
            ZONE_EXPERIENCE_READY if self.step == Step::Described => {
                handshake.session.send(ZONE_EXPERIENCE_READY, &[])?;
                self.step = Step::Answered;
            }
            ZONE_AVATAR_READY if self.step == Step::Answered => {
                ensure!(
                    self.profile.is_some(),
                    "zone became ready before player profile"
                );
                handshake
                    .session
                    .send(ZONE_SERVER_FILTER, &quarm::server_filters())?;
                // The first position completes the entry and triggers the
                // server's version check.
                handshake
                    .session
                    .send(ZONE_SPAWN_APPEARANCE, &quarm::dll_version_message(false))?;
                handshake.session.send(ZONE_CLIENT_UPDATE, &[0; 15])?;
                self.step = Step::Ready;
                handshake
                    .log
                    .send(ClientEvent::Progress(ConnectionStage::EnteringWorld))?;
            }
            ZONE_LOGOUT => bail!("server logged the character out"),
            ZONE_CHANGE_REQUEST => bail!("server requested a new zone during admission"),
            _ => (),
        }
        let (Step::Ready, Some(spawn), Some(spawn_id), Some(zone)) = (
            self.step,
            self.own_spawn.as_ref(),
            self.assigned_id,
            self.zone.as_ref(),
        ) else {
            return Ok(None);
        };
        let player = self.player(spawn_id, spawn);
        let zone = Zone {
            name: zone.clone(),
            // The EQMac zone header's clip and sky fields have not been verified.
            far_clip: None,
            sky: None,
        };
        world.admitted = Some(Instant::now());
        self.step = Step::Admitted;
        Ok(Some((player, zone)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{session::Events, ClientConfig};
    use eq_network_transport::Transport;

    /// The codec crate's synthetic, encrypted profile for "Example".
    const PROFILE: &str = "ae05f5a6083907c71a8fa0b9df18601d2fadb4d6b80a3099d83162b5c95015d3bd8bd662ad969f16538260e8249c5a9d1c78803e51a3f8d3d48f5ca3fb28b2bff7a48403366d5c880b3422059ae86574971744def74907496b270882";

    /// A connection that remembers what the client sent.
    #[derive(Default)]
    struct Recorded(Vec<(u16, Vec<u8>)>);

    impl Transport for Recorded {
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

    #[test]
    fn the_handshake_answers_the_zone_in_turn_and_admits_once_the_spawn_is_named() {
        let config = ClientConfig::new("ACCOUNT", "PASSWORD", "Test Server", "Example");
        let mut handler = |_| Ok(());
        let mut log = Events::new(&config, &mut handler);
        let mut wire = Recorded::default();
        let mut world = World::new(5);
        let mut shield = None;
        let mut checksums = [];
        let mut handshake = Handshake {
            session: &mut wire,
            shield: &mut shield,
            key: &[],
            checksums: &mut checksums,
            log: &mut log,
        };
        let mut admission = EqMacAdmission::start("Example", &mut handshake).unwrap();
        let mut read = |admission: &mut EqMacAdmission, world: &mut World, opcode, body: &[u8]| {
            admission
                .read(
                    &mut Application {
                        opcode,
                        body: body.to_vec(),
                    },
                    world,
                    &mut handshake,
                )
                .unwrap()
        };
        let mut own = [0; 356];
        own[5..12].copy_from_slice(b"Example");
        own[80..84].copy_from_slice(&42.0f32.to_le_bytes());
        let mut description = [0; 96];
        description[64..71].copy_from_slice(b"example");
        // A version check during the entry is answered at once.
        assert!(read(
            &mut admission,
            &mut world,
            ZONE_SPAWN_APPEARANCE,
            &[0, 0, 0, 1, 0, 0, 4, 0]
        )
        .is_none());
        assert!(read(
            &mut admission,
            &mut world,
            ZONE_PLAYER_PROFILE,
            &hex::decode(PROFILE).unwrap()
        )
        .is_none());
        assert!(read(&mut admission, &mut world, ZONE_ENTRY, &own).is_none());
        assert!(read(&mut admission, &mut world, ZONE_WEATHER, &[]).is_none());
        assert_eq!(admission.stage(), "Requested");
        assert!(read(&mut admission, &mut world, ZONE_NEW, &description).is_none());
        assert!(read(&mut admission, &mut world, ZONE_EXPERIENCE_READY, &[]).is_none());
        assert!(read(&mut admission, &mut world, ZONE_AVATAR_READY, &[]).is_none());
        // Ready, but the zone has not named the player's spawn yet.
        assert_eq!(admission.stage(), "Ready");
        assert!(world.admitted.is_none());
        let (player, zone) = read(
            &mut admission,
            &mut world,
            ZONE_SPAWN_APPEARANCE,
            &[0, 0, 16, 0, 7, 0, 0, 0],
        )
        .expect("the named spawn admits the player");
        let player = player.unwrap();
        assert_eq!((player.spawn_id, player.level), (7, 12));
        assert!((player.position.x - 42.0).abs() < f32::EPSILON);
        assert_eq!(zone.name, "example");
        assert_eq!(world.own_spawn, Some(7));
        assert!(world.admitted.is_some());
        // Once admitted, the handshake takes nothing more.
        assert!(read(&mut admission, &mut world, ZONE_AVATAR_READY, &[]).is_none());
        drop(read);
        let sent: Vec<u16> = wire.0.iter().map(|(opcode, _)| *opcode).collect();
        assert_eq!(
            sent,
            [
                ZONE_DATA_RATE,
                ZONE_ENTRY,
                ZONE_SPAWN_APPEARANCE,
                ZONE_REQUEST_NEW,
                ZONE_REQUEST_SPAWNS,
                ZONE_EXPERIENCE_READY,
                ZONE_SERVER_FILTER,
                ZONE_SPAWN_APPEARANCE,
                ZONE_CLIENT_UPDATE,
            ]
        );
        assert_eq!(wire.0[2].1, quarm::dll_version_message(true));
        assert_eq!(wire.0[7].1, quarm::dll_version_message(false));
    }

    #[test]
    fn readiness_before_the_profile_or_a_logout_ends_the_admission() {
        let config = ClientConfig::new("ACCOUNT", "PASSWORD", "Test Server", "Example");
        let mut handler = |_| Ok(());
        let mut log = Events::new(&config, &mut handler);
        let mut wire = Recorded::default();
        let mut world = World::new(5);
        let mut shield = None;
        let mut checksums = [];
        let mut handshake = Handshake {
            session: &mut wire,
            shield: &mut shield,
            key: &[],
            checksums: &mut checksums,
            log: &mut log,
        };
        let mut admission = EqMacAdmission::start("Example", &mut handshake).unwrap();
        let mut description = [0; 96];
        description[64..71].copy_from_slice(b"example");
        for (opcode, body) in [
            (ZONE_WEATHER, Vec::new()),
            (ZONE_NEW, description.to_vec()),
            (ZONE_EXPERIENCE_READY, Vec::new()),
        ] {
            admission
                .read(
                    &mut Application { opcode, body },
                    &mut world,
                    &mut handshake,
                )
                .unwrap();
        }
        for opcode in [ZONE_AVATAR_READY, ZONE_LOGOUT] {
            let mut packet = Application {
                opcode,
                body: Vec::new(),
            };
            assert!(admission
                .read(&mut packet, &mut world, &mut handshake)
                .is_err());
        }
    }
}
