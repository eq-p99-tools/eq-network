# eq-network

High-level blocking and cancellable native client for EverQuest-compatible
servers. It composes the transport, login, and game crates into a session that
can select a world and character, enter a zone, receive structured chat, send
commands, reconnect, and emit typed lifecycle events.

Applications own credential persistence, worker-thread policy, command queues,
and output formatting. See the
[workspace README](https://github.com/eq-p99-tools/eq-network#readme) for an
example and the extension model.
