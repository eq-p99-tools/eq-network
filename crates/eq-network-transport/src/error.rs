use thiserror::Error;

/// Validation failures reported by pure transport codecs.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    /// The selected input range cannot contain the required packet header.
    #[error("buffer too short: need {need} bytes, got {got}")]
    TooShort {
        /// Minimum required byte count.
        need: usize,
        /// Available byte count.
        got: usize,
    },
    /// A combined sub-packet exceeds its extended length field.
    #[error("combined sub-packet length {len} exceeds wire capacity")]
    CombinedSubPacketTooLong {
        /// Requested sub-packet length.
        len: usize,
    },
    /// An application payload exceeds the fragment length field.
    #[error("application length {len} exceeds fragment wire capacity")]
    ApplicationTooLong {
        /// Requested application length.
        len: usize,
    },
    /// A negotiated packet size cannot hold a fragment header and payload byte.
    #[error("maximum packet size {size} cannot hold a fragment header")]
    PacketSizeTooSmall {
        /// Negotiated maximum packet size.
        size: usize,
    },
}

/// Result returned by pure transport codecs.
pub type Result<T> = std::result::Result<T, ProtocolError>;
