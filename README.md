# eq-network

Reusable Rust networking crates for native EverQuest-compatible clients. The
workspace contains the protocol code shared by
[`eq-client`](https://github.com/eq-p99-tools/eq-client), the graphical desktop
client,
[`p99-login-proxy`](https://github.com/eq-p99-tools/p99-login-proxy),
[`p99-logger-client`](https://github.com/rm-you/p99-logger-client), and the
native mobile client.

The high-level client logs in, selects a world, lists, creates and enters
characters, and runs a zone session: it reports typed world state
(`WorldEvent`, in a `ClientEvent::World`) and carries out typed commands
(`ClientCommand`), reconnects, and shuts down cooperatively. Chat arrives
structured, with item-link data, for consumers that only talk.

What a zone session offers depends on its server type (`ServerProtocol`), and
each session reports the `Capability` values it offers so that a front end can
grey out the rest. A server type gains a feature only once it has been checked
against that kind of server:

| Server type | Protocol | What a zone session offers |
| --- | --- | --- |
| `EqEmu` | Titanium, for a stock `EQEmu` server | Moving, jumping and falling (and a fall's damage), targeting, combat, casting and the spellbook (deleting and moving its spells too), the inventory and bank, merchants, handing items to NPCs, looting, chat, camping, doors, items on the ground, zoning, abilities, `/who`, corpses and consent, pets, training, resurrection, reading, tradeskill containers, the map and the time of day |
| `Project1999` | Titanium with P99's V62 protection | As `EqEmu`, except jumping and falling, fishing and binding wounds, deleting and moving the spellbook's spells, training, resurrection, reading, tradeskills and the map, which wait to be checked on P99 |
| `Takp` | Windows TAKP/`EQMac`, for a stock TAKP server | Character creation, entering the world, chat, moving and camping |
| `Quarm` | Windows TAKP/`EQMac` | Entering the world and chat; Quarm's features follow once they are checked on TAKP |

## Crates

| Crate | Responsibility |
| --- | --- |
| [`eq-network-transport`](crates/eq-network-transport) | SOE reliable-UDP framing, sequencing, fragmentation, compression, CRCs, and session I/O |
| [`eq-network-login`](crates/eq-network-login) | Login application packets, legacy credential encryption, and server-list codecs |
| [`eq-network-game`](crates/eq-network-game) | World and zone packets, validation, chat codecs, and protocol dialects |
| [`eq-network`](crates/eq-network) | Credentials, configuration, session lifecycle, commands, events, and bundled validation assets |

Applications normally depend only on `eq-network`. Proxies and protocol tools
can use the lower-level crates without pulling in the character-session engine.

A consumer that only listens runs the client with `Client::run`; one that
plays passes a command queue to `Client::run_with_commands`, whose commands the
session takes once the zone admits the player.

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

The zone session is built from features, one per part of the game (doors,
looting, pets and so on), each implementing the session's `Feature` trait: it
records what the server says about its part, carries out the host's commands
for it, and reports what changed. Each server type (`client/session/servers.rs`)
names the client generation's wire it speaks and provides the features it
offers; a new server type starts with every feature off.

New client actions follow one path:

1. Add a typed request and its packet codec to `eq-network-game`, for each
   client generation that has it.
2. Add a `ClientCommand` variant that describes the action without exposing an
   opcode or byte layout to applications, and the `Capability` it needs.
3. Carry the command out in the feature that owns that part of the game, and
   report typed `WorldEvent` changes or results. A refusal says why, and names
   the official client's string for it where it has one.
4. Provide the feature from each server type it has been checked against.
5. Add packet fixtures or synthetic unit tests at the codec boundary, then a
   session test for the state transition.

A server-directed zone handoff is a connection-state transition: the game codec
decodes the offer, while the session engine closes the old zone session,
connects to the assigned endpoint, and reports `ConnectionState::Zoning` until
the new zone is ready.

The library carries no text from the official client's string table
(`eqstr_us.txt`). Where the game words something, such as a refusal or a `/who`
list, events name the string's eqstr id, so a front end shows the official
words from the player's own installation, with the library's own words as the
fallback.

The enums a front end matches to stay in step with the game, `WorldEvent`,
`GameCommand` (`ClientCommand`) and `ClientEvent`, are exhaustive on purpose: a
new event or command is a breaking change that names itself in the consumer's
build, rather than news a wildcard arm silently ignores. Configuration, status
and other protocol enums are non-exhaustive, and consumers should include a
wildcard arm when matching them. New server families should be represented as
explicit dialect/profile variants rather than conditionals in application code.

## Compatibility and safety

- The minimum supported Rust version is 1.88.
- Public APIs follow semantic versioning. The crates begin at `0.1.0` while the
  extracted interfaces settle.
- Credentials are redacted from `Debug` output and zeroized when their owners
  are dropped. Applications remain responsible for secure persistence.
- Packet captures, logs, environment files, credentials, and official game
  text do not belong in the repository. Tests use synthetic values, made-up
  string tables, and generated packet bodies.
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
