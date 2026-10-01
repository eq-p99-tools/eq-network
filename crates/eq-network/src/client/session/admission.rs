//! The Titanium zone handshake: the entry the client sends, what the zone
//! sends before the player can act, the client's answers, and the player it
//! admits. Everything else the zone says meanwhile goes to the features,
//! which stage it for the admission.
use super::{
    bail, chat, cstr, ensure, feature::World, le32, put_string, servers::Shield, ClientEvent,
    ConnectionStage, ConnectionState, Context, Events, Instant, Result,
};
use crate::world::{PlayerState, Position};
use eq_network_transport::{Application, Transport};

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

/// How far the handshake has come.
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
}

/// The handshake in progress.
pub(super) struct Admission {
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

impl Admission {
    /// Asks the zone to admit the character.
    ///
    /// # Errors
    /// Returns an error when the entry cannot be sent or the protection
    /// refuses it.
    pub(super) fn start(
        session: &mut dyn Transport,
        character: &str,
        revolution: f32,
        shield: &mut Option<Box<dyn Shield>>,
        log: &mut Events<'_>,
    ) -> Result<Self> {
        session.send(0x7752, &0u32.to_le_bytes())?;
        let mut entry = vec![0; 68];
        put_string(&mut entry[4..], character)?;
        if let Some(shield) = shield.as_mut() {
            shield.zone_entry(&entry)?;
        }
        session.send(SPAWN_OPCODE, &entry)?;
        log.send(ClientEvent::Progress(ConnectionStage::LoadingCharacter))?;
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
    pub(super) const fn stage(&self) -> Stage {
        match self.reports {
            Reports::Admitted => Stage::Admitted,
            Reports::Answered => Stage::Answered,
            Reports::Awaited if self.zone.is_some() => Stage::Described,
            Reports::Awaited if self.requested => Stage::Requested,
            Reports::Awaited => Stage::Entering,
        }
    }

    /// Takes one packet of the handshake, answering it as the client must.
    /// When it admits the player, returns them, built from their profile and
    /// spawn, and their zone.
    ///
    /// # Errors
    /// Returns an error when the zone refuses the character or sends a
    /// malformed profile, spawn or description.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn read(
        &mut self,
        packet: &mut Application,
        world: &mut World,
        shield: &mut Option<Box<dyn Shield>>,
        key: &[u8],
        checksums: &mut [u8],
        session: &mut dyn Transport,
        log: &mut Events<'_>,
    ) -> Result<Option<(Result<PlayerState>, Zone)>> {
        if self.reports == Reports::Admitted {
            return Ok(None);
        }
        let mut admitted = None;
        match packet.opcode {
            PROFILE_OPCODE => self.profile(&packet.body, world)?,
            WEATHER_OPCODE => self.weather = true,
            SPAWN_OPCODE if self.spawn.is_none() => {
                if let Some(shield) = shield.as_mut() {
                    shield.player_spawn(&mut packet.body, key)?;
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
                log.zone.clone_from(&name);
                log.status(
                    ConnectionState::Zoning,
                    world.packets,
                    Some(session.last_received_seconds()),
                )?;
                self.zone = Some(Zone {
                    name,
                    far_clip: crate::world::titanium_far_clip(&packet.body),
                });
                session.send(0x067a, &0u32.to_le_bytes())?;
                session.send(0x5e3a, &0u32.to_le_bytes())?;
                session.send(0x7752, &0u32.to_le_bytes())?;
                session.send(0x0322, &[])?;
            }
            EXPERIENCE_OPCODE if self.zone.is_some() && self.reports == Reports::Awaited => {
                session.send(EXPERIENCE_OPCODE, &[])?;
                self.reports = Reports::Answered;
            }
            EXPERIENCE_OPCODE if self.zone.is_some() => {
                session.send(0x6563, &chat::server_filters())?;
                session.send(0x5e20, &[])?;
                session.send(0x0c11, &1u32.to_le_bytes())?;
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
            if let Some(shield) = shield.as_ref() {
                shield.answer(checksums)?;
                session.send(0x1251, checksums)?;
            }
            session.send(0x7ac5, &[])?;
            session.send(0x367d, &[])?;
            session.send(0x5966, &[])?;
            self.requested = true;
            log.send(ClientEvent::Progress(ConnectionStage::EnteringWorld))?;
        }
        Ok(admitted)
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
        let mut admission =
            Admission::start(&mut wire, "Tester", 512.0, &mut shield, &mut log).unwrap();
        assert_eq!(wire.0, [0x7752, SPAWN_OPCODE]);
        let mut spawn = vec![0; 385];
        spawn[7..13].copy_from_slice(b"Tester");
        spawn[340..344].copy_from_slice(&9u32.to_le_bytes());
        let mut description = vec![0; 96];
        description[64..72].copy_from_slice(b"qeytoqrg");
        let mut read = |admission: &mut Admission, world: &mut World, opcode, body| {
            admission
                .read(
                    &mut packet(opcode, body),
                    world,
                    &mut shield,
                    &[],
                    &mut [],
                    &mut wire,
                    &mut log,
                )
                .unwrap()
                .map(|(_, zone)| zone.name)
        };
        // The profile, spawn and weather come in any order.
        assert!(read(&mut admission, &mut world, WEATHER_OPCODE, Vec::new()).is_none());
        assert!(read(&mut admission, &mut world, SPAWN_OPCODE, spawn).is_none());
        assert_eq!(world.own_spawn, Some(9));
        assert_eq!(admission.stage(), Stage::Entering);
        read(
            &mut admission,
            &mut world,
            PROFILE_OPCODE,
            vec![0; PROFILE_LENGTH],
        );
        assert_eq!(admission.stage(), Stage::Requested);
        // A report before the description is not one of the two.
        read(&mut admission, &mut world, EXPERIENCE_OPCODE, Vec::new());
        assert_eq!(admission.stage(), Stage::Requested);
        read(&mut admission, &mut world, DESCRIPTION_OPCODE, description);
        assert_eq!(admission.stage(), Stage::Described);
        read(&mut admission, &mut world, EXPERIENCE_OPCODE, Vec::new());
        assert_eq!(admission.stage(), Stage::Answered);
        assert!(world.admitted.is_none());
        let zone = read(&mut admission, &mut world, EXPERIENCE_OPCODE, Vec::new());
        assert_eq!(zone.as_deref(), Some("qeytoqrg"));
        assert_eq!(admission.stage(), Stage::Admitted);
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
        let mut admission =
            Admission::start(&mut wire, "Tester", 512.0, &mut shield, &mut log).unwrap();
        let mut other = vec![0; 385];
        other[7..12].copy_from_slice(b"Other");
        for (opcode, body) in [(SPAWN_OPCODE, other), (REJECTED_OPCODE, Vec::new())] {
            assert!(admission
                .read(
                    &mut packet(opcode, body),
                    &mut world,
                    &mut shield,
                    &[],
                    &mut [],
                    &mut wire,
                    &mut log,
                )
                .is_err());
        }
    }
}
