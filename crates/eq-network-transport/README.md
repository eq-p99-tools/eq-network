# eq-network-transport

SOE reliable-UDP framing and sessions used by EverQuest-compatible login,
world, and zone protocols. It owns sequence handling, acknowledgements,
fragmentation, compression, keyed CRCs, retransmission, and session statistics.

Application opcodes and packet bodies belong in the login and game crates.

## Crate family

| Crate | Role |
| --- | --- |
| [`eq-network`](https://crates.io/crates/eq-network) | High-level client that composes the protocol layers into complete sessions. |
| [`eq-network-transport`](https://crates.io/crates/eq-network-transport) | Reliable UDP framing and transport sessions. |
| [`eq-network-login`](https://crates.io/crates/eq-network-login) | Login, server-list, and world-selection codecs built on the transport crate. |
| [`eq-network-game`](https://crates.io/crates/eq-network-game) | World and zone codecs used by the high-level client. |
