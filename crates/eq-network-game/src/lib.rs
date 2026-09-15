//! World, zone, validation, and chat codecs for EverQuest-compatible servers.

/// Communication packet codecs and structured item-link extraction.
pub mod chat;
/// Typed client actions and dialect-specific application packet encoding.
pub mod command;
/// Titanium/P99-V62 world and zone validation codec.
pub mod p99;

use serde::{Deserialize, Serialize};

/// Game packet layout used by a client generation.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GameDialect {
    /// Titanium with the Project 1999 V62 patch.
    #[default]
    TitaniumP99,
    /// The Windows TAKP/EQMac client used by Project Quarm.
    EqMac,
}
