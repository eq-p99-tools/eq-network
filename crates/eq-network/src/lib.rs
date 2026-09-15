//! Native `EverQuest` login, world, and zone client.

/// P99 client-file inventories and validation-manifest responses.
pub mod assets;
/// High-level login, world, and zone session engine.
pub mod client;

pub use eq_network_game::{chat, p99, GameDialect};
pub use eq_network_transport::legacy as old_transport;
pub use eq_network_transport::modern as transport;
