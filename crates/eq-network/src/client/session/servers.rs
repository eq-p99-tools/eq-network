//! What each server type offers in the zone, and what differs between servers
//! that speak the same client generation.
//!
//! Every part of the game is a feature a [`ServerType`] may have, each absent
//! unless the server type implements it. The zone session builds only the
//! features its server type provides, so a new server type starts as an empty
//! implementation with every feature off, and gains them one at a time. Each
//! server type also names the client generation's [`Wire`] it speaks, which
//! its features sit on: P99 and `EQEmu` share Titanium's yet differ in their
//! features.

use anyhow::Result;

use super::{
    abilities::Abilities,
    camp::Camp,
    casting::Casting,
    character::Character,
    clock::Clock,
    combat::Combat,
    corpses::Corpses,
    doors::Doors,
    entities::Entities,
    exchange::Exchanges,
    feature::Feature,
    inventory::Belongings,
    looting::Looting,
    motion::Motion,
    objects::GroundObjects,
    pets::Pets,
    spellbook::Spellbook,
    talk::Talk,
    targeting::Targeting,
    training::Training,
    transfers::Transfers,
    who::Who,
    wire::{EqMac, Titanium, Wire},
    CharacterSession, Events, ServerProtocol, ZoneExit,
};
use crate::p99::{self, WorldCodec};

/// What a zone session builds its features with.
pub(super) struct Setup<'a> {
    /// The player's name, which the transfers feature reads the server's
    /// answers by.
    pub(super) character: &'a str,
    /// What the session may eat and drink on its own.
    pub(super) auto_eat: eq_network_game::food::AutoEat,
}

impl<'a> Setup<'a> {
    /// The setup for a zone session of this character.
    pub(super) fn new(character: &'a str, auto_eat: eq_network_game::food::AutoEat) -> Self {
        Self {
            character,
            auto_eat,
        }
    }
}

/// A feature as a server type provides it, ready for the zone session.
pub(super) type Provided = Option<Box<dyn Feature>>;

/// A server type: the client generation it speaks, what differs from other
/// servers that speak it, and the features it offers, each absent unless it
/// implements them.
pub(super) trait ServerType: Sync {
    /// The client generation's packets the server speaks.
    fn wire(&self) -> &'static dyn Wire;

    /// Protects a world connection, from the login body sent to it: P99's
    /// world approval, encrypted file manifests and checksum answers, and
    /// spawns encrypted with the login session key.
    ///
    /// # Errors
    /// Returns an error when the login body cannot key the protection.
    fn protect(&self, _login_info: &[u8]) -> Result<Option<Box<dyn Shield>>> {
        Ok(None)
    }

    /// A full turn in the headings a saved profile carries.
    fn profile_turn(&self) -> f32 {
        512.0
    }

    /// Whether the world expects the start-zone choice right after a
    /// character is created, before the character enters.
    fn start_choice(&self) -> bool {
        false
    }

    /// Runs the character's stay in a zone: the shared zone session, with
    /// the features this server type provides.
    ///
    /// # Errors
    /// Returns an error when the zone connection or admission fails.
    fn zone(
        &self,
        context: &CharacterSession<'_>,
        shield: &mut Option<Box<dyn Shield>>,
        (host, port): (&str, u16),
        checksums: Vec<u8>,
        log: &mut Events<'_>,
    ) -> Result<ZoneExit> {
        super::zone::run(context, shield, host, port, checksums, log)
    }

    /// Casting memorized spells and using items' effects.
    fn casting(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Memorizing, scribing and forgetting spells.
    fn spellbook(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// The inventory, the bank, coins, merchants, and food and drink.
    fn inventory(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// How the player moves: walking, running and stance, and on some
    /// servers jumps and falls.
    fn motion(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// The player's own record: level, experience, vitals and the like.
    fn character(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// The zone's spawns.
    fn entities(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Choosing a target.
    fn targeting(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Considering and attacking.
    fn combat(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Looting corpses.
    fn looting(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Giving items to NPCs and trading with players.
    fn exchange(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Skills used from the Actions window, such as kick or hide.
    fn abilities(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Speaking on the chat channels.
    fn talk(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Camping out to the character list.
    fn camp(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Opening doors.
    fn doors(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Items on the ground and the containers fixed in the zone.
    fn ground_items(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Zone lines, death and other transfers the server directs.
    fn transfers(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// The time of day in Norrath.
    fn clock(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Asking the world who is online.
    fn who(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Consenting to, summoning and dragging corpses.
    fn corpses(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Commanding the player's pet.
    fn pets(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Training skills at a guildmaster.
    fn training(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Every feature the server type provides, in the order the zone session
    /// offers them each command, packet and timer.
    fn features(&self, setup: &Setup<'_>) -> Vec<Box<dyn Feature>> {
        [
            self.casting(setup),
            self.spellbook(setup),
            self.inventory(setup),
            self.motion(setup),
            self.character(setup),
            self.entities(setup),
            self.targeting(setup),
            self.combat(setup),
            self.looting(setup),
            self.exchange(setup),
            self.abilities(setup),
            self.talk(setup),
            self.camp(setup),
            self.doors(setup),
            self.ground_items(setup),
            self.transfers(setup),
            self.clock(setup),
            self.who(setup),
            self.corpses(setup),
            self.pets(setup),
            self.training(setup),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

/// One protected connection's encryption and checks, keyed per world and
/// rekeyed per zone.
pub(in crate::client) trait Shield {
    /// Answers the world's approval challenge.
    ///
    /// # Errors
    /// Returns an error when the challenge is malformed.
    fn approve(&mut self, challenge: &[u8]) -> Result<Vec<u8>>;

    /// Decrypts a file manifest in place.
    ///
    /// # Errors
    /// Returns an error when the manifest is malformed.
    fn manifest(&mut self, body: &mut [u8]) -> Result<()>;

    /// Encrypts a file checksum answer in place.
    ///
    /// # Errors
    /// Returns an error when no manifest keyed the answer yet.
    fn answer(&self, body: &mut [u8]) -> Result<()>;

    /// Decrypts the file manifest a zone handoff carries.
    ///
    /// # Errors
    /// Returns an error when the handoff has no manifest.
    fn zone_manifest(&self, handoff: &[u8]) -> Result<Vec<u8>>;

    /// Rekeys for a zone from the zone entry sent to it.
    ///
    /// # Errors
    /// Returns an error when the entry is malformed.
    fn zone_entry(&mut self, entry: &[u8]) -> Result<()>;

    /// Decrypts the player's spawn and keys the zone's checksum answer from it.
    ///
    /// # Errors
    /// Returns an error when the spawn is malformed.
    fn player_spawn(&mut self, body: &mut [u8], session_key: &[u8]) -> Result<()>;

    /// Decrypts other spawns in place: `opcode` says whether the packet
    /// carries them.
    ///
    /// # Errors
    /// Returns an error without a session key.
    fn spawns(&self, opcode: u16, body: &mut [u8], session_key: &[u8]) -> Result<()>;
}

/// P99's V62 protection over Titanium.
impl Shield for WorldCodec {
    fn approve(&mut self, challenge: &[u8]) -> Result<Vec<u8>> {
        Self::approve(self, challenge)
    }

    fn manifest(&mut self, body: &mut [u8]) -> Result<()> {
        Self::manifest(self, body)
    }

    fn answer(&self, body: &mut [u8]) -> Result<()> {
        self.file_response(body)
    }

    fn zone_manifest(&self, handoff: &[u8]) -> Result<Vec<u8>> {
        Self::zone_manifest(self, handoff)
    }

    fn zone_entry(&mut self, entry: &[u8]) -> Result<()> {
        Self::zone_entry(self, entry)
    }

    fn player_spawn(&mut self, body: &mut [u8], session_key: &[u8]) -> Result<()> {
        p99::session_xor(body, session_key)?;
        self.zone_spawn(body)
    }

    fn spawns(&self, opcode: u16, body: &mut [u8], session_key: &[u8]) -> Result<()> {
        // New spawns and zone spawn batches; the XOR runs continuously over
        // the full batch, not per spawn.
        if matches!(opcode, 0x2e78 | 0x1860) {
            p99::session_xor(body, session_key)?;
        }
        Ok(())
    }
}

/// The features every Titanium server offers today, shared by the server
/// types that speak it; each server type still lists the ones it provides.
mod titanium {
    use super::{
        Abilities, Belongings, Camp, Casting, Character, Clock, Combat, Corpses, Doors, Entities,
        Exchanges, Feature, GroundObjects, Looting, Motion, Pets, Setup, Spellbook, Talk,
        Targeting, Training, Transfers, Who,
    };

    pub(super) fn casting() -> Box<dyn Feature> {
        Box::<Casting>::default()
    }

    pub(super) fn spellbook() -> Box<dyn Feature> {
        Box::<Spellbook>::default()
    }

    pub(super) fn inventory(setup: &Setup<'_>) -> Box<dyn Feature> {
        Box::new(Belongings::new(setup.auto_eat))
    }

    /// Moving, with or without the jumps and falls the server takes.
    pub(super) fn motion(falls: bool) -> Box<dyn Feature> {
        Box::new(Motion::new(falls))
    }

    pub(super) fn character() -> Box<dyn Feature> {
        Box::<Character>::default()
    }

    pub(super) fn entities() -> Box<dyn Feature> {
        Box::<Entities>::default()
    }

    pub(super) fn targeting() -> Box<dyn Feature> {
        Box::new(Targeting)
    }

    pub(super) fn combat() -> Box<dyn Feature> {
        Box::new(Combat)
    }

    pub(super) fn looting() -> Box<dyn Feature> {
        Box::new(Looting)
    }

    pub(super) fn exchange() -> Box<dyn Feature> {
        Box::new(Exchanges)
    }

    pub(super) fn abilities() -> Box<dyn Feature> {
        Box::<Abilities>::default()
    }

    pub(super) fn talk() -> Box<dyn Feature> {
        Box::new(Talk)
    }

    pub(super) fn camp() -> Box<dyn Feature> {
        Box::<Camp>::default()
    }

    pub(super) fn doors() -> Box<dyn Feature> {
        Box::<Doors>::default()
    }

    pub(super) fn ground_items() -> Box<dyn Feature> {
        Box::<GroundObjects>::default()
    }

    pub(super) fn transfers(setup: &Setup<'_>) -> Box<dyn Feature> {
        Box::new(Transfers::new(setup.character))
    }

    pub(super) fn clock() -> Box<dyn Feature> {
        Box::<Clock>::default()
    }

    pub(super) fn who() -> Box<dyn Feature> {
        Box::new(Who)
    }

    pub(super) fn corpses() -> Box<dyn Feature> {
        Box::new(Corpses)
    }

    pub(super) fn pets() -> Box<dyn Feature> {
        Box::new(Pets)
    }

    pub(super) fn training() -> Box<dyn Feature> {
        Box::<Training>::default()
    }
}

/// Project 1999: Titanium with V62 protection and 256-unit saved headings.
/// Jumps and falls wait until they are measured on P99.
struct Project1999;

impl ServerType for Project1999 {
    fn wire(&self) -> &'static dyn Wire {
        &Titanium
    }

    fn protect(&self, login_info: &[u8]) -> Result<Option<Box<dyn Shield>>> {
        Ok(Some(Box::new(WorldCodec::new(login_info)?)))
    }

    fn profile_turn(&self) -> f32 {
        256.0
    }

    fn casting(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::casting())
    }

    fn spellbook(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::spellbook())
    }

    fn inventory(&self, setup: &Setup<'_>) -> Provided {
        Some(titanium::inventory(setup))
    }

    fn motion(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::motion(false))
    }

    fn character(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::character())
    }

    fn entities(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::entities())
    }

    fn targeting(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::targeting())
    }

    fn combat(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::combat())
    }

    fn looting(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::looting())
    }

    fn exchange(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::exchange())
    }

    fn abilities(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::abilities())
    }

    fn talk(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::talk())
    }

    fn camp(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::camp())
    }

    fn doors(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::doors())
    }

    fn ground_items(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::ground_items())
    }

    fn transfers(&self, setup: &Setup<'_>) -> Provided {
        Some(titanium::transfers(setup))
    }

    fn clock(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::clock())
    }

    fn who(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::who())
    }

    fn corpses(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::corpses())
    }

    fn pets(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::pets())
    }
}

/// A stock `EQEmu` server speaking Titanium, which takes the player's jumps
/// and falls.
struct EqEmu;

impl ServerType for EqEmu {
    fn wire(&self) -> &'static dyn Wire {
        &Titanium
    }

    fn start_choice(&self) -> bool {
        true
    }

    fn casting(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::casting())
    }

    fn spellbook(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::spellbook())
    }

    fn inventory(&self, setup: &Setup<'_>) -> Provided {
        Some(titanium::inventory(setup))
    }

    fn motion(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::motion(true))
    }

    fn character(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::character())
    }

    fn entities(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::entities())
    }

    fn targeting(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::targeting())
    }

    fn combat(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::combat())
    }

    fn looting(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::looting())
    }

    fn exchange(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::exchange())
    }

    fn abilities(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::abilities())
    }

    fn talk(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::talk())
    }

    fn camp(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::camp())
    }

    fn doors(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::doors())
    }

    fn ground_items(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::ground_items())
    }

    fn transfers(&self, setup: &Setup<'_>) -> Provided {
        Some(titanium::transfers(setup))
    }

    fn clock(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::clock())
    }

    fn who(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::who())
    }

    fn corpses(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::corpses())
    }

    fn pets(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::pets())
    }

    fn training(&self, _setup: &Setup<'_>) -> Provided {
        Some(titanium::training())
    }
}

/// Project Quarm, which speaks `EQMac` and provides no feature on this
/// interface yet: its own zone loop runs it until it moves onto the shared
/// session.
struct Quarm;

impl ServerType for Quarm {
    fn wire(&self) -> &'static dyn Wire {
        &EqMac
    }

    /// Quarm's own zone loop, until Quarm moves onto the shared session.
    fn zone(
        &self,
        context: &CharacterSession<'_>,
        _shield: &mut Option<Box<dyn Shield>>,
        (host, port): (&str, u16),
        _checksums: Vec<u8>,
        log: &mut Events<'_>,
    ) -> Result<ZoneExit> {
        crate::client::quarm::zone(context, log, host, port)?;
        Ok(ZoneExit::Stopped)
    }
}

/// The server type of a server protocol.
pub(super) fn server_type(protocol: ServerProtocol) -> &'static dyn ServerType {
    match protocol {
        ServerProtocol::Project1999 => &Project1999,
        ServerProtocol::EqEmu => &EqEmu,
        ServerProtocol::Quarm => &Quarm,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Capability;
    use eq_network_game::{
        command::titanium_camp,
        food::AutoEat,
        request::{Request, Sender},
    };

    /// A player with a spawn.
    const SENDER: Sender<'static> = Sender {
        name: "Tester",
        spawn_id: Some(7),
    };

    /// A server type that implements nothing but the wire it speaks.
    struct Empty;

    impl ServerType for Empty {
        fn wire(&self) -> &'static dyn Wire {
            &Titanium
        }
    }

    /// What a server type's features let the player do, each once.
    fn offers(server: &dyn ServerType) -> Vec<Capability> {
        let setup = Setup::new("Tester", AutoEat::default());
        let mut capabilities: Vec<_> = server
            .features(&setup)
            .iter()
            .flat_map(|feature| feature.capabilities())
            .collect();
        capabilities.sort_unstable();
        capabilities.dedup();
        capabilities
    }

    #[test]
    fn a_new_server_type_starts_with_every_feature_off() {
        let empty: &dyn ServerType = &Empty;
        let setup = Setup::new("Tester", AutoEat::default());
        assert!(empty.features(&setup).is_empty());
        assert!(empty.protect(&[0; 464]).unwrap().is_none());
        assert!((empty.profile_turn() - 512.0).abs() < f32::EPSILON);
        assert!(!empty.start_choice());
        // Quarm speaks EQMac and has built none of them on the interface yet.
        let quarm = server_type(ServerProtocol::Quarm);
        assert!(quarm.wire().encode(&Request::Camp, SENDER).is_err());
        let setup = Setup::new("Tester", AutoEat::default());
        assert!(quarm.features(&setup).is_empty());
        assert!(quarm.protect(&[0; 464]).unwrap().is_none());
        assert!(!quarm.start_choice());
    }

    #[test]
    fn p99_protects_and_halves_saved_headings_while_eqemu_takes_falls() {
        let p99 = server_type(ServerProtocol::Project1999);
        assert_eq!(
            p99.wire().encode(&Request::Camp, SENDER).unwrap(),
            titanium_camp()
        );
        assert!(p99.protect(&[0; 464]).unwrap().is_some());
        assert!(p99.protect(&[0; 10]).is_err());
        assert!((p99.profile_turn() - 256.0).abs() < f32::EPSILON);
        assert!(!p99.start_choice());
        assert!(!offers(p99).contains(&Capability::Falling));
        let eqemu = server_type(ServerProtocol::EqEmu);
        assert_eq!(
            eqemu.wire().encode(&Request::Camp, SENDER).unwrap(),
            titanium_camp()
        );
        assert!(eqemu.protect(&[0; 464]).unwrap().is_none());
        assert!((eqemu.profile_turn() - 512.0).abs() < f32::EPSILON);
        assert!(eqemu.start_choice());
        assert!(offers(eqemu).contains(&Capability::Falling));
    }

    #[test]
    fn p99_and_eqemu_provide_every_feature_one_each() {
        // Training is checked on EQEmu alone so far.
        for (protocol, count) in [
            (ServerProtocol::Project1999, 20),
            (ServerProtocol::EqEmu, 21),
        ] {
            let server = server_type(protocol);
            let setup = Setup::new("Tester", AutoEat::default());
            assert_eq!(server.features(&setup).len(), count, "{protocol:?}");
        }
        assert!(!offers(server_type(ServerProtocol::Project1999)).contains(&Capability::Training));
        assert!(offers(server_type(ServerProtocol::EqEmu)).contains(&Capability::Training));
    }

    #[test]
    fn only_spawn_packets_are_decrypted_with_the_session_key() {
        let shield = WorldCodec::new(&[0; 464]).unwrap();
        let plain = vec![1, 2, 3, 4];
        let mut other = plain.clone();
        shield.spawns(0x14cb, &mut other, b"0123456789").unwrap();
        assert_eq!(other, plain);
        for opcode in [0x2e78, 0x1860] {
            let mut spawns = plain.clone();
            shield.spawns(opcode, &mut spawns, b"0123456789").unwrap();
            assert_ne!(spawns, plain);
            p99::session_xor(&mut spawns, b"0123456789").unwrap();
            assert_eq!(spawns, plain);
        }
    }
}
