# eq-network-game

Pure world and zone packet codecs for Titanium/P99-V62 and Windows
TAKP/EQMac-compatible game servers. It provides validation packets, structured
chat and item links, outbound chat, and the protocol dialect boundary used for
future movement, zoning, inventory, and character actions.

The crate does not open sockets or retain credentials.

## Crate family

| Crate | Role |
| --- | --- |
| [`eq-network`](https://crates.io/crates/eq-network) | High-level client that composes the protocol layers into complete sessions. |
| [`eq-network-transport`](https://crates.io/crates/eq-network-transport) | Reliable UDP framing and transport sessions. |
| [`eq-network-login`](https://crates.io/crates/eq-network-login) | Login, server-list, and world-selection codecs built on the transport crate. |
| [`eq-network-game`](https://crates.io/crates/eq-network-game) | World and zone codecs used by the high-level client. |
