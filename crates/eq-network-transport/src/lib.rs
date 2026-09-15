//! Reliable UDP sessions and wire helpers used by EverQuest-compatible servers.

/// Encoding and decoding for SOE combined transport packets.
pub mod combined;
/// SOE keyed CRC helpers.
pub mod crc;
/// Transport codec errors.
pub mod error;
/// SOE application-fragment assembly and encoding.
pub mod fragment;
/// Reliable UDP used by the `EQMac` client generation.
pub mod legacy;
/// Reliable UDP used by later SOE client generations.
pub mod modern;
/// Primitive SOE session and transport packet codecs.
pub mod soe;

pub use combined::{build_combined, CombinedPacket, SubPacket};
pub use fragment::FragmentAssembler;
pub use modern::Application;
pub use soe::{
    build_ack, build_disconnect, build_keepalive, build_session_request, build_session_response,
    get_sequence, set_sequence, transport_opcode, wrap_app_packet, SessionResponse, TransportOp,
};
