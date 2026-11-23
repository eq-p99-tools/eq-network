# eq-network

Reusable Rust networking crates for native EverQuest-compatible clients. The
workspace holds the protocol code shared by
[`eq-client`](https://github.com/eq-p99-tools/eq-client), the graphical desktop
client,
[`p99-login-proxy`](https://github.com/eq-p99-tools/p99-login-proxy),
[`p99-logger-client`](https://github.com/eq-p99-tools/p99-logger-client), and the
native mobile client.

The high-level client logs in, plays on the configured world or the one the
player chooses from the login server's list, lists, creates and enters
characters, and runs a zone session: it reports typed world state
(`WorldEvent`, in a `ClientEvent::World`) and carries out typed commands
(`ClientCommand`), reconnects, and shuts down cooperatively. Chat arrives
structured, with item-link data, for consumers that only talk.

## Server types

| Server type | Protocol | For |
| --- | --- | --- |
| `Project1999` | Titanium, with P99's protection | Project 1999 |
| `EqEmu` | Titanium | A stock EQEmu server |
| `Quarm` | Windows TAKP/`EQMac` | Project Quarm |
| `Takp` | Windows TAKP/`EQMac` | A stock TAKP server |
| `EqEmuRof2` | RoF2 (placeholder) | Nothing yet: a configuration naming it is refused before anything is sent, and it has no name to choose it by |

Each server type names the client generation's wire it speaks and provides the
features it offers. Every session reports the `Capability` values it offers,
so a front end can grey out the rest. A server type gains a feature only once
it has been checked against that kind of server, and the session answers each
server the way that server's original client does.

### What each server type offers

This table lists the capabilities each server type's features offer on `main`.
[`servers.rs`](crates/eq-network/src/client/session/servers.rs) decides. P99's
earliest listings predate the checked-first rule and still wait for their
check on P99. A front end goes by the capabilities its own session
reports, from the revision of this library that its lockfile resolves. Offered
is not the same as verified live: the
[public-server gap report](docs/public-server-gaps.md) separates implemented,
disabled, unverified and server-restricted features, with the evidence for
each.

| Capability | What it covers | P99 | EQEmu | Quarm | TAKP |
| --- | --- | :-: | :-: | :-: | :-: |
| **Moving and the world** | | | | | |
| `Moving` | Walking and running | Yes | Yes | Yes | Yes |
| `Jumping` | Jumping | Yes | Yes | Yes | Yes |
| `Falling` | Falling, which the server takes from the client | Yes | Yes | Yes | Yes |
| `Posture` | Sitting, standing and ducking | Yes | Yes | Yes | Yes |
| `Swimming` | Swimming in the zone's water | Yes | Yes | – | – |
| `EnvironmentalDamage` | Reporting falls, and drowning or lava where offered | Yes | Yes | Yes | Yes |
| `Zoning` | Crossing zone lines and being moved between zones | Yes | Yes | Yes | Yes |
| `Camping` | Camping to the character list | Yes | Yes | Yes | Yes |
| `Doors` | Opening doors | Yes | Yes | Yes | Yes |
| `GroundItems` | Picking up items from the ground | Yes | Yes | Yes | Yes |
| `BoatRiding` | Riding a boat and zoning with it | Yes | Yes | – | – |
| `BoatControl` | Taking and releasing a boat you stand on | – | Yes | – | – |
| `Map` | The in-game map, drawn from the installation's map files | Left to the player | Yes | – | – |
| **Combat, abilities and spells** | | | | | |
| `Targeting` | Choosing a target | Yes | Yes | Yes | Yes |
| `Combat` | Considering and attacking | Yes | Yes | Yes | Yes |
| `RangedCombat` | Firing a bow or throwing a weapon | Yes | Yes | – | – |
| `Abilities` | Kick, bash, taunt, hide, sneak, forage and the like | Yes | Yes | Yes | Yes |
| `KnightAbilities` | Lay on Hands and Harm Touch | Yes | Yes | – | – |
| `UsingDisciplines` | Disciplines the server lists, for every melee class | Yes | Yes | – | – |
| `Tracking` | The tracking window | Yes | Yes | – | – |
| `Pets` | Commanding a pet (source-derived on TAKP and Quarm, not yet checked live) | Yes | Yes | Yes | Yes |
| `Casting` | Casting memorized spells | Yes | Yes | Yes | Yes |
| `Spellbook` | Memorizing, scribing and forgetting spells | Yes | Yes | Yes | Yes |
| `MovingSpells` | Moving a spell within the spellbook | Yes | Yes | – | – |
| `DeletingSpells` | Deleting a spell from the spellbook | – | Yes | – | – |
| `CancellingBuffs` | Cancelling a beneficial buff | Yes | Yes | – | – |
| `StoppingSongs` | Stopping a Bard song | Yes | Yes | – | – |
| **Items and coin** | | | | | |
| `Inventory` | Moving and using items in the inventory and the bank | Yes | Yes | Yes | Yes |
| `Meals` | Eating and drinking | Yes | Yes | Yes | Yes |
| `Trading` | Buying from and selling to merchants | Yes | Yes | Yes | Yes |
| `MerchantOffers` | Showing what a merchant pays | – | Yes | – | – |
| `Giving` | Handing items to NPCs and trading with players | Yes | Yes | Yes | Yes |
| `Looting` | Looting corpses | Yes | Yes | Yes | Yes |
| `Corpses` | Consenting, summoning and dragging corpses (source-derived on TAKP and Quarm, not yet checked live; /corpsedrop on Quarm only, /deny on neither) | Yes | Yes | Yes | Yes |
| `Tradeskills` | Combining in your own containers | Yes | Yes | – | – |
| `WorldContainers` | Combining in world containers such as forges | Yes | Yes | – | – |
| `Reading` | Reading books and notes | Yes | Yes | Yes | Yes |
| `BankerChange` | Changing coins at a banker | Yes | Yes | – | – |
| `Splitting` | Sharing coin with the group (source-derived on TAKP and Quarm, not yet checked live) | Yes | Yes | Yes | Yes |
| `BazaarSearching` | Searching the Bazaar's traders, read only (source-derived on TAKP and Quarm, not yet checked live) | – | – | Yes | Yes |
| **People** | | | | | |
| `Talking` | Chat channels and item links | Yes | Yes | Yes | Yes |
| `Who` | `/who` | Yes | Yes | Yes | Yes |
| `FriendsWho` | Friends online | Yes | Yes | Yes | Yes |
| `Grouping` | Groups | Yes | Yes | Yes | Yes |
| `Raiding` | Raids | – | Yes | – | – |
| `Listing` | AFK, anonymous and roleplaying | Yes | Yes | Yes | Yes |
| `Rolling` | `/random` (source-derived on TAKP and Quarm, not yet checked live) | Yes | Yes | Yes | Yes |
| `Emoting` | `/emote` | Yes | Yes | – | – |
| `Assisting` | `/assist` (source-derived on TAKP and Quarm, not yet checked live) | Yes | Yes | Yes | Yes |
| `Petitioning` | Petitions to the server's staff | – | Yes | – | – |
| `ReportingBugs` | Bug reports | – | Yes | – | – |
| `SendingFeedback` | Feedback, which only TAKP-family servers read | – | – | – | – |
| `PlayerInspection` | Inspecting other players | Yes | Yes | – | – |
| `GuildRoster` | A guild member's side of a guild | Yes | Yes | – | – |
| `GuildManagement` | An officer's and a leader's side of a guild | Yes | Yes | – | – |
| **Server prompts and events** | | | | | |
| `Training` | Training skills at a guildmaster | Yes | Yes | – | – |
| `Resurrection` | Accepting or declining a resurrection | Yes | Yes | – | – |
| `Translocate` | Accepting or declining a translocate | Yes | Yes | Yes | Yes |
| `Sacrifice` | Accepting or declining a sacrifice | – | Yes | – | – |
| `ServerDialogs` | Answering server dialogs | Yes | Yes | – | – |
| `BleedingOut` | Reporting a death the server leaves to the client | – | – | – | Yes |
| `ServerTicks` | Showing when the regeneration tick lands | – | Yes | – | – |
| `ShowingHelm` | Showing or hiding your helm to others | – | Yes | – | – |

Some capabilities depend on the session, not only on the server type:

- `GameplayDepartures`: opt-in departures from the original client's movement
  and memorization, offered where the server type permits them. Every server
  type lists them today while they are under test.
- `AnsweringListPolls`: P99 list presence replies, off by default
  (`ClientConfig::answer_list_polls`). A runtime choice follows the character
  across zones, and returning to character selection restores the startup
  option. Replies wait 3 to 60 seconds and never join a list; whether the
  deployed server accepts them is still unverified.
- `Drowning`, `LavaHeat` and `GroundDropping`: offered on a stock EQEmu session
  only when it is restricted to the local network.
- `RaidGroupLeaders`: not offered by any server type yet.
- `Talking`: all chat a host asks for is paced, at most 10 lines in any 5 s
  and 12 shout, auction, out-of-character, tell, channel messages and chat
  server commands but those that reach no one else in any
  60 s, on every server type. The numbers are inferred, and the pace is a
  departure that only makes the client slower; see "Chat pacing" in
  `API-CONTRACT.md`.
- `SendingFeedback`, and petitions and bug reports on Quarm and TAKP: built
  and tested, and listed by no server type until each has its own live check,
  since a person reads every report.

On P99 the map is left to the player: the session runs it but lists it among
the player's choices. The world is also told what machine the player entered
on, once a week, just before entering the world, when the host supplies it
(`ClientConfig::sysinfo`). No other server type sends that report.

More detail on particular areas:
[boat riding](docs/boat-riding.md), [boat control](docs/boat-control.md),
[training](docs/training.md), [P99 reading](docs/p99-reading.md),
[P99 manual combines](docs/p99-manual-combines.md),
[EQMac world objects](docs/eqmac-world-objects.md),
[EQMac looting](docs/eqmac-looting.md), [Quarm combat](docs/quarm-combat.md),
[Quarm meals](docs/quarm-meals.md), [Quarm merchants](docs/quarm-merchants.md)
and [session termination](docs/session-termination.md).
[API and integration contracts](API-CONTRACT.md) says how features, commands
and their answers behave.

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
session takes once the zone admits the player. Before that, the queue carries
the player's choices from the lists the session sends: the world from the login
server's (`WorldEvent::ServerSelection`, answered with
`ClientCommand::SelectServer`) when the configuration names none, and the
character from the world's (`WorldEvent::CharacterSelection`, answered with
`ClientCommand::SelectCharacter`) when it names none.

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

## Character creation

`WorldEvent::CharacterSelection.creation` offers a list-scoped `creation::Rules`
policy: classic P99, Titanium EQEmu, or EQMac TAKP/Quarm. Frontends use its race,
class, deity and starting-city choices with the world's expansion and PvP reports.
Titanium sends a city-choice index; EQMac sends the actual starting zone. The
server remains authoritative, including custom database restrictions.

`NewCharacter.face` is the base face. `cosmetics` holds hair, beard, eye and tattoo
choices. `Rules::appearance(race, gender)` offers valid controls, reconciles race
changes and composes the packed face detail for each wire format. Zero cosmetics
retain the default look for existing callers. `tutorial` opts into the Titanium
world's tutorial for the created character's first entry.

`ClientCommand::GenerateName` and `WorldEvent::NameSuggestion` carry both list and
request identities. Titanium uses the world's 72-byte name request; EQMac policies
select `creation::names::local`, because their worlds have no mapped opcode for
that request. The local phonemes are original data, not the official client tables.
Suggestions do not reserve names. World requests are single-flight; after a timeout,
a late response must be consumed before another request can go out.
Name approval precedes creation, and success requires the new name in a refreshed
list. A 60-second unanswered attempt ends the connection: reconnect to inspect
whether it succeeded before retrying. The wire has no request ID for late replies.

The Quarm path was checked independently against
[`SecretsOTheP/EQMacEmu`'s world handlers](https://github.com/SecretsOTheP/EQMacEmu/blob/main/world/client.cpp)
and [creation record](https://github.com/SecretsOTheP/EQMacEmu/blob/main/common/eq_packet_structs.h).
Its 76-byte approval body, 8452-byte creation body and refusal/list sequence match
this codec. This source and synthetic-test evidence is not a live creation check.

## Compatibility and safety

- The minimum supported Rust version is 1.99.
- Public APIs follow semantic versioning. The crates stay at `0.1.x` while the
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
[`eq-client`](https://github.com/eq-p99-tools/eq-client) pins this repository
by Git revision instead, so changes reach it without a release.

## License

MIT. See [LICENSE](LICENSE).

This project is not affiliated with Daybreak Game Company, Project 1999, The
Al'Kabor Project, or Project Quarm.
