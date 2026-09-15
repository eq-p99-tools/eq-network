# eq-network-login

Login application packet codecs for EverQuest-compatible login servers. It
parses and builds authentication, server-list, and world-selection messages and
supports the legacy DES-CBC credential format required on the wire.

Decoded credential owners redact their `Debug` output and zeroize sensitive
buffers on drop. The crate performs no network I/O.

## Crate family

| Crate | Role |
| --- | --- |
| [`eq-network`](https://crates.io/crates/eq-network) | High-level client that composes the protocol layers into complete sessions. |
| [`eq-network-transport`](https://crates.io/crates/eq-network-transport) | Reliable UDP framing and transport sessions. |
| [`eq-network-login`](https://crates.io/crates/eq-network-login) | Login, server-list, and world-selection codecs built on the transport crate. |
| [`eq-network-game`](https://crates.io/crates/eq-network-game) | World and zone codecs used by the high-level client. |
