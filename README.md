# eq-network

Reusable Rust networking crates for native EverQuest-compatible clients. The
workspace contains the protocol code shared by
[`p99-login-proxy`](https://github.com/eq-p99-tools/p99-login-proxy),
[`p99-logger-client`](https://github.com/rm-you/p99-logger-client), and the
native mobile client.

The current high-level client can log in, select a world and character, enter a
zone, receive structured chat (including item-link data), send chat, reconnect,
and shut down cooperatively. It supports the Titanium/P99-V62 and Windows
TAKP/EQMac protocol families. Live Project Quarm testing has exercised login,
world and character selection, zone entry, received chat, outbound tells, and
the required DLL version announcement through the Android client.

## Crates

| Crate | Responsibility |
| --- | --- |
| [`eq-network-transport`](crates/eq-network-transport) | SOE reliable-UDP framing, sequencing, fragmentation, compression, CRCs, and session I/O |
| [`eq-network-login`](crates/eq-network-login) | Login application packets, legacy credential encryption, and server-list codecs |
| [`eq-network-game`](crates/eq-network-game) | World and zone packets, validation, chat codecs, and protocol dialects |
| [`eq-network`](crates/eq-network) | Credentials, configuration, session lifecycle, commands, events, and bundled validation assets |

Applications normally depend only on `eq-network`. Proxies and protocol tools
can use the lower-level crates without pulling in the character-session engine.

```rust,no_run
use eq_network::client::{
    CancellationToken, Client, ClientConfig, ClientIdentity, RunOptions,
};

# fn main() -> anyhow::Result<()> {
let config = ClientConfig::new(
    "EXAMPLE_ACCOUNT",
    "EXAMPLE_PASSWORD",
    "Project 1999: Green (Velious, PvE)",
    "ExampleCharacter",
);
let client = Client::new(config, ClientIdentity::new("example-host", "example-user"))?;
let cancel = CancellationToken::default();
client.run(&cancel, RunOptions::default(), |event| {
    println!("{event:?}");
    Ok(())
})?;
# Ok(())
# }
```

## Architecture and extension points

Each layer owns one kind of change. Wire framing belongs in
`eq-network-transport`; login opcodes belong in `eq-network-login`; game packet
layouts and per-client-generation differences belong in `eq-network-game`;
connection policy and state transitions belong in `eq-network`.

New client actions follow one path:

1. Add a typed request and its packet codec to `eq-network-game`.
2. Add a `ClientCommand` variant that describes the action without exposing an
   opcode or byte layout to applications.
3. Handle that command in the applicable session driver and emit typed
   `ClientEvent` state changes or results.
4. Add packet fixtures or synthetic unit tests at the codec boundary, then a
   session test for the state transition.

Movement, inventory operations, group actions, and zone requests can use this
path. A server-directed zone handoff is a connection-state transition: the game
codec decodes the offer, while the session engine closes the old zone session,
connects to the assigned endpoint, and reports `ConnectionState::Zoning` until
the new zone is ready.

Public protocol, command, event, and state enums are non-exhaustive. Consumers
should include a wildcard arm when matching them. New server families should be
represented as explicit dialect/profile variants rather than conditionals in
application code.

## Compatibility and safety

- The minimum supported Rust version is 1.88.
- Public APIs follow semantic versioning. The crates begin at `0.1.0` while the
  extracted interfaces settle.
- Credentials are redacted from `Debug` output and zeroized when their owners
  are dropped. Applications remain responsible for secure persistence.
- Packet captures, logs, environment files, and credentials do not belong in
  the repository. Tests use synthetic values and generated packet bodies.
- Legacy DES-CBC exists only for wire compatibility and provides no modern
  confidentiality or integrity.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the required checks and extension
guidelines.

## Publishing

The crates are published in dependency order: `eq-network-transport`,
`eq-network-login`, `eq-network-game`, then `eq-network`. Their initial releases
were bootstrapped with a crates.io token because trusted publishing requires an
existing crate. Later releases use the manual `publish.yml` workflow and
short-lived crates.io OIDC credentials.

## License

MIT. See [LICENSE](LICENSE).

This project is not affiliated with Daybreak Game Company, Project 1999, The
Al'Kabor Project, or Project Quarm.
