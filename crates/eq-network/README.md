# eq-network

High-level blocking and cancellable native client for EverQuest-compatible
servers. It composes the transport, login, and game crates into a session that
can select a world and character, enter a zone, receive structured chat, send
commands, reconnect, and emit typed lifecycle events.

Applications own credential persistence, worker-thread policy, command queues,
and output formatting. See the
[workspace README](https://github.com/eq-p99-tools/eq-network#readme) for an
example and the extension model.

## Crate family

| Crate | Role |
| --- | --- |
| [`eq-network`](https://crates.io/crates/eq-network) | High-level client that composes the protocol layers into complete sessions. |
| [`eq-network-transport`](https://crates.io/crates/eq-network-transport) | Reliable UDP framing and transport sessions. |
| [`eq-network-login`](https://crates.io/crates/eq-network-login) | Login, server-list, and world-selection codecs built on the transport crate. |
| [`eq-network-game`](https://crates.io/crates/eq-network-game) | World and zone codecs used by the high-level client. |
