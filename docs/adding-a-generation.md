# Adding a client generation

What it takes to add a third client generation (RoF2 is the planned one) or a new server type
next to Titanium (P99, stock `EQEmu`) and `EQMac` (Project Quarm, TAKP). These are pointers to
where each piece goes. They aren't facts about any generation.

## Two enums, one rule

- `GameDialect` (`eq-network-game/src/lib.rs`) is the client generation: the official program
  whose packets and rules we follow.
- `ServerProtocol` (`eq-network/src/client.rs`) is the server type. It is the server software
  together with the generation it speaks. `From<ServerProtocol> for GameDialect` maps each server
  type to its generation.

Both are exhaustive on purpose (API-CONTRACT.md, "Public API evolution"). Adding a variant breaks
every `match` on it in this repository and in eq-client, and each broken `match` is a decision
the new variant needs. `crates/eq-network/tests/exhaustive_checks.rs` refuses comparisons on one
variant, so no decision can hide behind a `==`, `!=`, `matches!` or `if let`. A yes/no rule
belongs in a named predicate whose body is an exhaustive `match`.

## Client-program rules and server-software rules

Keep the two kinds of rules apart. Two server types running the same server software with
different generations (for example stock `EQEmu` with Titanium and with RoF2) must **share** the
server's rules, not copy them.

- **Client-program rules** follow the official program of one generation: packet layouts, how far
  the player reaches, falls and jumps, posture and Feign Death, water, buff slots and targeting.
  They live on `GameDialect` accessors and on the generation's `Wire`
  (`eq-network/src/client/session/wire.rs`). Under the parity rule, each value comes from a read
  of that program. Until a read exists, the value is absent (`None`, or the `Wire` method's
  refusal), or it is labelled inferred with a recording checklist item.
- **Server-software rules** follow what the server does with what it is sent: how inventory moves
  settle (`MoveRules`), how it interprets item records (`ItemRules`), its spell landing flag, chat
  link growth, which features it offers. They are `ServerType` methods
  (`eq-network/src/client/session/servers.rs`). Feature constructors used by more than one server
  type sit in that file's `shared` module. A second server type for the same software calls the
  same `shared` constructors and returns the same rule values. Don't write a second copy of them.

## Steps

1. **The variant.** Add it to `GameDialect`, then follow the compiler through every `match` in
   eq-network-game and eq-network. Each accessor on `GameDialect` answers from a program read,
   or as absent, or as labelled inferred (see above).
2. **The wire.** Implement `Wire` for the generation (`wire.rs`). Every stage has a default that
   reads nothing or refuses ("this client generation cannot log in yet"). So a generation can be
   built one stage at a time: login, world, zone connection, admission, then messages, chat,
   coverage and encoding. The generation's reach, how it closes a trade window and how it numbers
   the player's own updates have no default: it states its own.
3. **Opcodes and codecs.** A coverage table next to `coverage/titanium.rs` and `coverage/eqmac.rs`
   (what the session does with each message the generation's servers send). A message decoder
   next to `message::titanium` and `message::eqmac_with`, and a request encoder next to
   `request::titanium` and `request::eqmac`. Opcodes stay in the generation's own modules.
4. **Login and world.** Next to `login::titanium` and `login::eqmac::login`, and next to
   `world::titanium`, with admission next to `TitaniumAdmission` and `EqMacAdmission`.
5. **The server type.** A `ServerProtocol` variant (with `default_endpoint`, its generation in
   `From<ServerProtocol> for GameDialect`, and a `FromStr` name once it can log in) and a
   `ServerType` implementation
   whose `wire()` returns the new wire. It offers no features until each one is checked on a live
   server of that type, as every server type starts. Server-software rules are shared, as above.
   `ServerType::item_rules()` keeps its default: it only says how the shared `EQMac` inventory
   records are read, and is inert for any other generation's wire.
6. **eq-client.** Repin, then follow the compiler there too. See eq-client's ARCHITECTURE.md,
   "Client generations".

## RoF2 status: placeholder

`GameDialect::Rof2` and `ServerProtocol::EqEmuRof2` exist so every decision above has an answer,
and nothing of RoF2 is built:

- `ClientConfig` validation refuses `EqEmuRof2`, so no session starts and nothing is sent. The
  variant has no `FromStr` name, so no setting can choose it.
- The `Rof2` wire keeps every refusing default. `EqEmuRof2` offers no features and copies no
  `EqEmu` rule.
- Every packet layout refuses it ("not built for RoF2 yet"). Program rules without a read are
  `None`, or Titanium's value on an arm of its own commented `inferred for RoF2 (6.489)`.
  Recording checklist item 6.489 is the list of those reads: grep the token. A read replaces one
  arm. Where a function carries `#[expect(clippy::match_same_arms)]` for its RoF2 arms, the
  expectation fails once no arm matches Titanium's any more, so it leaves with the last one.

Built since, from `EQEmu`'s public source and labelled inferred for `RoF2` (6.497), and still
unreachable because validation refuses the generation:

- The login stage: Titanium's exchange read through `RoF2`'s login server opcodes
  (`LoginOpcodes::ROF2`), skipping the expansion list newer clients may be sent. It is tested
  over a socket inside the crate.
- The character list (`characters::decode` for `Rof2`) and the 488-byte world login info
  (`world_login::rof2`). The session keeps only names from a `RoF2` list (`without_look`), since
  the layout is not checked against the client.
- The world stage against stock `EQEmu` (`world::rof2`, 6.499): the login info, the list, the
  entry (`OP_EnterWorld`) and the handoff (`OP_ZoneServerInfo`) or the zone-unavailable notice.
  Between zones it enters once the world's post-entry message arrives. It sends nothing stock
  `EQEmu` ignores; what the `RoF2` program sends there is a read still owed.
- The zone connection and entry (`admission::rof2`, 6.501): the entry, the zone's unasked
  description (its short name read), the spawn request, and the ready after the zone's objects.
  What the `RoF2` program sends during entry, and what prompts its spawn request and ready, is a
  read still owed. The handoff between zones still refuses.
- The player profile and own spawn (`rof2_player`, 6.502), each walked whole as the encoder
  writes it: every count the encoder's constant, every string ended in its field, and an exact
  end. The entry completes with the player read from them, or without either with no player
  (the session then only records). An admitted `RoF2` session still sends nothing in the zone:
  `EqEmuRof2` offers no feature, server filters or departures, and `Rof2::encode` refuses, so
  every command is refused. Gems 9 to 12, appearance and the `/who` listing are not read, and
  what the `RoF2` program reads in each field is a read still owed.
- Its spawn records (`rof2_player::spawn`, 6.510): stock `EQEmu` sends every spawn as one
  `OP_ZoneEntry` record, and one walker reads every branch the encoder writes (player,
  creature, corpse, treasure chest, destructible object, and the three-texture non-player
  races), with the packed position from the active `Spawn_Struct_Position`. `Rof2::messages`
  makes each record a spawn, its first zone decoder. The entry keeps the first record, the
  player's own, to itself, and any later one with the player's id, so the player is never
  another spawn. With no feature offered, a spawn read during the entry reaches no one yet.
  The motion deltas, animation and angle are not read, and velocity and forward input stay
  zero.
- Its coverage table (`coverage::rof2`, 6.508): a row for each message a `RoF2` stage reads,
  and nothing else, so the gap log tells every other message as unlisted. A test holds the
  rows to the stages' own opcodes in both directions.

Server-software checks follow the same rule. A comparison such as
`protocol == ServerProtocol::EqEmu` would silently answer "no" for a second `EQEmu` server type.
So these are exhaustive matches too, and the guard test covers `ServerProtocol` as well.
