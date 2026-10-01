//! What differs between servers that speak the same client protocol.
//!
//! Every difference is a feature a [`ServerType`] may have. The session asks
//! for a feature and does nothing when it is absent, so each server type
//! implements only what it does, and a new server type starts as an empty
//! implementation with every feature off.

use anyhow::Result;

use super::ServerProtocol;
use crate::p99::{self, WorldCodec};

/// A server type's features, each absent unless the server has it.
pub(super) trait ServerType: Sync {
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

    /// Whether the server takes jumps and falls from the client.
    fn falls(&self) -> bool {
        false
    }
}

/// One protected connection's encryption and checks, keyed per world and
/// rekeyed per zone.
pub(super) trait Shield {
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

/// Project 1999: Titanium with V62 protection and 256-unit saved headings.
struct Project1999;

impl ServerType for Project1999 {
    fn protect(&self, login_info: &[u8]) -> Result<Option<Box<dyn Shield>>> {
        Ok(Some(Box::new(WorldCodec::new(login_info)?)))
    }

    fn profile_turn(&self) -> f32 {
        256.0
    }
}

/// A stock `EQEmu` server speaking Titanium.
struct EqEmu;

impl ServerType for EqEmu {
    fn start_choice(&self) -> bool {
        true
    }

    fn falls(&self) -> bool {
        true
    }
}

/// Project Quarm, whose features are not built on this interface yet.
struct Quarm;

impl ServerType for Quarm {}

/// The features of a server protocol's server type.
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

    /// A server type that implements nothing.
    struct Empty;

    impl ServerType for Empty {}

    #[test]
    fn a_new_server_type_starts_with_every_feature_off() {
        let empty: &dyn ServerType = &Empty;
        assert!(empty.protect(&[0; 464]).unwrap().is_none());
        assert!((empty.profile_turn() - 512.0).abs() < f32::EPSILON);
        assert!(!empty.start_choice() && !empty.falls());
        // Quarm has not built any of these on the interface yet.
        let quarm = server_type(ServerProtocol::Quarm);
        assert!(quarm.protect(&[0; 464]).unwrap().is_none());
        assert!(!quarm.start_choice() && !quarm.falls());
    }

    #[test]
    fn p99_protects_and_halves_saved_headings_while_eqemu_takes_falls() {
        let p99 = server_type(ServerProtocol::Project1999);
        assert!(p99.protect(&[0; 464]).unwrap().is_some());
        assert!(p99.protect(&[0; 10]).is_err());
        assert!((p99.profile_turn() - 256.0).abs() < f32::EPSILON);
        assert!(!p99.start_choice() && !p99.falls());
        let eqemu = server_type(ServerProtocol::EqEmu);
        assert!(eqemu.protect(&[0; 464]).unwrap().is_none());
        assert!((eqemu.profile_turn() - 512.0).abs() < f32::EPSILON);
        assert!(eqemu.start_choice() && eqemu.falls());
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
