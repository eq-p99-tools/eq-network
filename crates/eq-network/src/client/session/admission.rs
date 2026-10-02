//! The zone handshake: the entry the client sends, what the zone sends
//! before the player can act, the client's answers, and the player it
//! admits. Each client generation has its own, behind [`Admission`];
//! Titanium's is here. Everything else the zone says meanwhile goes to the
//! features, which stage it for the admission.
use super::{
    bail, chat, cstr, ensure, feature::World, le32, put_string, servers::Shield, ClientEvent,
    ConnectionStage, ConnectionState, Context, Events, Instant, Result,
};
use crate::world::{PlayerState, Position};
use eq_network_transport::{Application, Transport};

mod eqmac;
pub(super) use eqmac::EqMacAdmission;

const PROFILE_OPCODE: u16 = 0x75df;
const WEATHER_OPCODE: u16 = 0x254d;
const SPAWN_OPCODE: u16 = 0x7213;
const DESCRIPTION_OPCODE: u16 = 0x0920;
const EXPERIENCE_OPCODE: u16 = 0x0587;
const REJECTED_OPCODE: u16 = 0x1252;
/// The Titanium profile's length.
const PROFILE_LENGTH: usize = 19592;
/// Where the profile keeps the player's saved position and heading.
const SAVED_POSITION: [usize; 4] = [13116, 13120, 13124, 13128];

/// What a zone handshake works with besides the world: the connection, the
/// world's protection and file checksums, and the host's events.
pub(super) struct Handshake<'a, 'e> {
    /// The zone connection.
    pub(super) session: &'a mut dyn Transport,
    /// The world's protection, on servers that have one.
    pub(super) shield: &'a mut Option<Box<dyn Shield>>,
    /// The login session's key, which the protection decrypts with.
    pub(super) key: &'a [u8],
    /// The file checksums the protection answers with.
    pub(super) checksums: &'a mut [u8],
    /// The host's events.
    pub(super) log: &'a mut Events<'e>,
}

/// One client generation's zone handshake in progress.
pub(super) trait Admission {
    /// How far the handshake has come, for the host's diagnostics.
    fn stage(&self) -> &'static str;

    /// Takes one packet of the handshake, answering it as the client must.
    /// When it admits the player, returns them and their zone.
    ///
    /// # Errors
    /// Returns an error when the zone refuses the character or sends a
    /// malformed part of the handshake.
    fn read(
        &mut self,
        packet: &mut Application,
        world: &mut World,
        handshake: &mut Handshake<'_, '_>,
    ) -> Result<Option<(Result<PlayerState>, Zone)>>;
}

/// How far Titanium's handshake has come.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Stage {
    /// The entry was sent; the profile, the player's spawn and the weather
    /// arrive in any order.
    Entering,
    /// All three came and the client asked for the zone.
    Requested,
    /// The zone described itself; it reports experience next.
    Described,
    /// The client answered the first experience report; the second admits
    /// the player.
    Answered,
    /// The player is admitted.
    Admitted,
}

/// The zone the player was admitted to.
pub(super) struct Zone {
    /// Its short name.
    pub(super) name: String,
    /// Its far clip distance, when the description gives one.
    pub(super) far_clip: Option<f32>,
    /// Its sky and fog, when the description gives them.
    pub(super) sky: Option<eq_network_game::clock::ZoneSky>,
}

/// Titanium's handshake in progress.
pub(super) struct TitaniumAdmission {
    character: String,
    /// A full turn in the headings a saved profile carries.
    revolution: f32,
    profile: Option<Vec<u8>>,
    spawn: Option<Vec<u8>>,
    weather: bool,
    requested: bool,
    zone: Option<Zone>,
    /// How the zone's experience reports were answered.
    reports: Reports,
}

/// The zone's two experience reports after its description: the client
/// answers the first, and the second admits the player.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Reports {
    Awaited,
    Answered,
    Admitted,
}

impl TitaniumAdmission {
    /// Asks the zone to admit the character.
    ///
    /// # Errors
    /// Returns an error when the entry cannot be sent or the protection
    /// refuses it.
    pub(super) fn start(
        character: &str,
        revolution: f32,
        handshake: &mut Handshake<'_, '_>,
    ) -> Result<Self> {
        handshake.session.send(0x7752, &0u32.to_le_bytes())?;
        let mut entry = vec![0; 68];
        put_string(&mut entry[4..], character)?;
        if let Some(shield) = handshake.shield.as_mut() {
            shield.zone_entry(&entry)?;
        }
        handshake.session.send(SPAWN_OPCODE, &entry)?;
        handshake
            .log
            .send(ClientEvent::Progress(ConnectionStage::LoadingCharacter))?;
        Ok(Self {
            character: character.to_owned(),
            revolution,
            profile: None,
            spawn: None,
            weather: false,
            requested: false,
            zone: None,
            reports: Reports::Awaited,
        })
    }

    /// How far the handshake has come.
    pub(super) const fn progress(&self) -> Stage {
        match self.reports {
            Reports::Admitted => Stage::Admitted,
            Reports::Answered => Stage::Answered,
            Reports::Awaited if self.zone.is_some() => Stage::Described,
            Reports::Awaited if self.requested => Stage::Requested,
            Reports::Awaited => Stage::Entering,
        }
    }

    /// The player's profile: where the server saved them, and their zone.
    fn profile(&mut self, body: &[u8], world: &mut World) -> Result<()> {
        ensure!(
            body.len() == PROFILE_LENGTH,
            "unexpected Titanium player profile size"
        );
        let float = |offset: usize| {
            f32::from_le_bytes(body[offset..offset + 4].try_into().expect("bounded"))
        };
        ensure!(
            SAVED_POSITION
                .iter()
                .all(|offset| float(*offset).is_finite()),
            "invalid player position"
        );
        // The player stands where the server saved them until they can move.
        world.body.place(Position {
            x: float(13116),
            y: float(13120),
            z: float(13124),
            heading: crate::world::profile_heading(float(13128), self.revolution),
        });
        world.zone = (
            u16::from_le_bytes(body[13276..13278].try_into()?),
            u16::from_le_bytes(body[13278..13280].try_into()?),
        );
        self.profile = Some(body.to_vec());
        Ok(())
    }
}

impl Admission for TitaniumAdmission {
    fn stage(&self) -> &'static str {
        match self.progress() {
            Stage::Entering => "Entering",
            Stage::Requested => "Requested",
            Stage::Described => "Described",
            Stage::Answered => "Answered",
            Stage::Admitted => "Admitted",
        }
    }

    /// Takes one packet of the handshake, answering it as the client must.
    /// When it admits the player, returns them, built from their profile and
    /// spawn, and their zone.
    fn read(
        &mut self,
        packet: &mut Application,
        world: &mut World,
        handshake: &mut Handshake<'_, '_>,
    ) -> Result<Option<(Result<PlayerState>, Zone)>> {
        if self.reports == Reports::Admitted {
            return Ok(None);
        }
        let mut admitted = None;
        match packet.opcode {
            PROFILE_OPCODE => self.profile(&packet.body, world)?,
            WEATHER_OPCODE => self.weather = true,
            SPAWN_OPCODE if self.spawn.is_none() => {
                if let Some(shield) = handshake.shield.as_mut() {
                    shield.player_spawn(&mut packet.body, handshake.key)?;
                }
                ensure!(
                    packet.body.len() == 385
                        && cstr(&packet.body[7..71])
                            .eq_ignore_ascii_case(self.character.as_bytes()),
                    "zone returned a different character spawn"
                );
                let id = u16::try_from(le32(&packet.body[340..344]))
                    .context("spawn ID exceeds Titanium position field")?;
                world.body.own(id);
                world.own_spawn = Some(id);
                self.spawn = Some(packet.body.clone());
            }
            DESCRIPTION_OPCODE => {
                ensure!(packet.body.len() >= 96, "truncated zone description");
                let name = String::from_utf8_lossy(cstr(&packet.body[64..96])).into_owned();
                handshake.log.zone.clone_from(&name);
                handshake.log.status(
                    ConnectionState::Zoning,
                    world.packets,
                    Some(handshake.session.last_received_seconds()),
                )?;
                self.zone = Some(Zone {
                    name,
                    far_clip: crate::world::titanium_far_clip(&packet.body),
                    sky: eq_network_game::clock::titanium_zone_sky(&packet.body),
                });
                handshake.session.send(0x067a, &0u32.to_le_bytes())?;
                handshake.session.send(0x5e3a, &0u32.to_le_bytes())?;
                handshake.session.send(0x7752, &0u32.to_le_bytes())?;
                handshake.session.send(0x0322, &[])?;
            }
            EXPERIENCE_OPCODE if self.zone.is_some() && self.reports == Reports::Awaited => {
                handshake.session.send(EXPERIENCE_OPCODE, &[])?;
                self.reports = Reports::Answered;
            }
            EXPERIENCE_OPCODE if self.zone.is_some() => {
                handshake.session.send(0x6563, &chat::server_filters())?;
                handshake.session.send(0x5e20, &[])?;
                handshake.session.send(0x0c11, &1u32.to_le_bytes())?;
                world.admitted = Some(Instant::now());
                self.reports = Reports::Admitted;
                let player = crate::world::titanium_player(
                    self.profile.as_deref().unwrap_or_default(),
                    self.spawn.as_deref().unwrap_or_default(),
                    self.revolution,
                );
                admitted = self.zone.take().map(|zone| (player, zone));
            }
            REJECTED_OPCODE => bail!("server rejected zone validation"),
            _ => (),
        }
        if self.profile.is_some() && self.spawn.is_some() && self.weather && !self.requested {
            if let Some(shield) = handshake.shield.as_ref() {
                shield.answer(handshake.checksums)?;
                handshake.session.send(0x1251, handshake.checksums)?;
            }
            handshake.session.send(0x7ac5, &[])?;
            handshake.session.send(0x367d, &[])?;
            handshake.session.send(0x5966, &[])?;
            self.requested = true;
            handshake
                .log
                .send(ClientEvent::Progress(ConnectionStage::EnteringWorld))?;
        }
        Ok(admitted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ClientConfig;

    /// A connection that remembers what the client sent.
    #[derive(Default)]
    struct Wire(Vec<u16>);

    impl Transport for Wire {
        fn send(&mut self, opcode: u16, _body: &[u8]) -> Result<()> {
            self.0.push(opcode);
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

    fn packet(opcode: u16, body: Vec<u8>) -> Application {
        Application { opcode, body }
    }

    #[test]
    fn the_handshake_answers_the_zone_in_turn_and_admits_on_the_second_report() {
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
            log: &mut log,
        };
        let mut admission = TitaniumAdmission::start("Tester", 512.0, &mut handshake).unwrap();
        let mut spawn = vec![0; 385];
        spawn[7..13].copy_from_slice(b"Tester");
        spawn[340..344].copy_from_slice(&9u32.to_le_bytes());
        let mut description = vec![0; 96];
        description[64..72].copy_from_slice(b"qeytoqrg");
        let mut read = |admission: &mut TitaniumAdmission, world: &mut World, opcode, body| {
            admission
                .read(&mut packet(opcode, body), world, &mut handshake)
                .unwrap()
                .map(|(_, zone)| zone.name)
        };
        // The profile, spawn and weather come in any order.
        assert!(read(&mut admission, &mut world, WEATHER_OPCODE, Vec::new()).is_none());
        assert!(read(&mut admission, &mut world, SPAWN_OPCODE, spawn).is_none());
        assert_eq!(world.own_spawn, Some(9));
        assert_eq!(admission.progress(), Stage::Entering);
        read(
            &mut admission,
            &mut world,
            PROFILE_OPCODE,
            vec![0; PROFILE_LENGTH],
        );
        assert_eq!(admission.progress(), Stage::Requested);
        // A report before the description is not one of the two.
        read(&mut admission, &mut world, EXPERIENCE_OPCODE, Vec::new());
        assert_eq!(admission.progress(), Stage::Requested);
        read(&mut admission, &mut world, DESCRIPTION_OPCODE, description);
        assert_eq!(admission.progress(), Stage::Described);
        read(&mut admission, &mut world, EXPERIENCE_OPCODE, Vec::new());
        assert_eq!(admission.progress(), Stage::Answered);
        assert!(world.admitted.is_none());
        let zone = read(&mut admission, &mut world, EXPERIENCE_OPCODE, Vec::new());
        assert_eq!(zone.as_deref(), Some("qeytoqrg"));
        assert_eq!(admission.progress(), Stage::Admitted);
        assert!(world.admitted.is_some());
        // Once admitted, the handshake takes nothing more.
        assert!(read(&mut admission, &mut world, EXPERIENCE_OPCODE, Vec::new()).is_none());
    }

    #[test]
    fn a_different_character_or_a_rejection_ends_the_admission() {
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
            log: &mut log,
        };
        let mut admission = TitaniumAdmission::start("Tester", 512.0, &mut handshake).unwrap();
        let mut other = vec![0; 385];
        other[7..12].copy_from_slice(b"Other");
        for (opcode, body) in [(SPAWN_OPCODE, other), (REJECTED_OPCODE, Vec::new())] {
            assert!(admission
                .read(&mut packet(opcode, body), &mut world, &mut handshake)
                .is_err());
        }
    }
}
