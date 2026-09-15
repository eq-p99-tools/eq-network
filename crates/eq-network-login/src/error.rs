use thiserror::Error;

/// Errors returned while rewriting a login packet.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LoginError {
    /// A rewritten credential block exceeds its one-byte length field.
    #[error("login sub-packet length {len} exceeds one-byte wire capacity")]
    SubPacketLengthOverflow {
        /// Requested sub-packet length.
        len: usize,
    },
    /// A nested transport packet could not be encoded.
    #[error(transparent)]
    Transport(#[from] eq_network_transport::error::ProtocolError),
}

/// Result returned by login packet encoders.
pub type Result<T> = std::result::Result<T, LoginError>;
