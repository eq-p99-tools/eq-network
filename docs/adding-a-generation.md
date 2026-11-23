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
- Its spawns' movement and removal (`rof2_player::position` and `despawn`, 6.514): stock
  `EQEmu` sends the server's 24-byte `PlayerPositionUpdateServer_Struct` unencoded under
  `OP_ClientUpdate`, its five words laid out as the spawn record's position, which one table
  reads for both; the velocity and forward input are Titanium's scales, inferred. The 5-byte
  `OP_DeleteSpawn` gives the spawn id; its last byte is the encoder's constant. The delta
  heading and the vehicle id are not read. `OP_MobUpdate` and `OP_NPCMoveUpdate` stay unlisted,
  since stock `EQEmu` sends neither.
- How its spawns look and how hurt they are (`rof2_looks`, 6.518): the appearance, through the
  reader every generation shares with `RoF2`'s dialect (its invisibility values read as
  Titanium's, inferred; kind 20 through the `/who` listing as everywhere); `OP_ChangeSize`, the
  encoder's rewrite of a size appearance; the player's own hit points with the spawn id first;
  the health percent; the wear change with its 32-bit material; the animation with the action
  before the speed (Titanium's speed scale, inferred); and the player states, through the shared
  reader with `RoF2`'s opcode pair. Each keeps the refusals of Titanium's reader for the same
  values. What the `RoF2` program does with the invisibility values, the appearance kinds, the
  elite material and hero's forge model, the size message's constants and the animation speed
  is a read still owed. `OP_Illusion` is the casting batch's (6.522).
- The player's own news and the combat records (`rof2_news`, 6.519): the death, level,
  experience, food and water, coins and time of day go out unencoded, the bytes Titanium gets,
  and Titanium's readers read them; the mana change is the server's 20 bytes with the slot that
  Titanium's encoder drops (not read); the damage record carries a 32-bit spell id, refused past
  16 bits, and the push at 13; the consider answer carries no hit points; the skill update ends
  in four constants. The death record's bind zone and attack skill sit the other way round on
  stock `EQEmu` from where both programs' structs declare them, for both generations. The mana
  and endurance updates with their maxima, and the group and raid percents, stay unlisted.
- Its chat and the server's own lines (`chat`'s `RoF2` reading, 6.521): the channel message is
  its own layout, two NUL-ended names, five fixed fields, the text and a 15-byte zero tail;
  every other line keeps Titanium's shape under `RoF2`'s numbers. Links are 56 characters,
  passed through unchanged, with the item at Titanium's digits. Outbound chat still refuses
  `RoF2`.
- Its casting, stuns and illusions (`rof2_casting`, 6.522): the begin-cast record with the spell
  in 32 bits first; the action in its 56-byte form, whose instrument modifier is a float the
  encoder makes from the server's integer, read back as that integer, with the server's 0 to 9
  wrapped alike and read 0; the interruption, unencoded, through Titanium's reader; the stun's
  duration and four constants; the illusion's 336 bytes with a 16-bit race; and the face change
  as its own message with both eye colors, where Titanium's encoder sends an illusion of race
  -1. What the `RoF2` program does with the action's gem, slot and item-cast type, the stun's
  constants, the illusion's bytes 70 and 71 and its constant, and the Drakkin words is a read
  still owed.
- Its buffs (`rof2_buffs`, 6.527): the buff update in its 100-byte form, whose fade word the
  encoder narrows to 1 or 2 (read as Titanium's 1 and 0, inferred) and which carries no
  counters; the icon messages, its own window and a target's, which Titanium is not sent and
  which give `WorldEvent::BuffIcons`; and the pet's window in the icon messages' shape, read
  into Titanium's pet buffs. The server's buff slots are `RoF2`'s own. How the `RoF2` program
  applies the fade words, and what it does with the update's hit count, location and slot
  data, is a read still owed.
- Its spell book and gems (`rof2_spellbook`, 6.532): the memorize, swap and deletion messages go
  out unencoded, the server's own structs, and Titanium's readers read them with a 720-slot book.
  A bar refresh's slot is the server's casting slot, which Titanium's encoder renumbers, and it
  carries the reduction Titanium's encoder drops. The profile's book and gems 9 to 12 stay the
  profile's planned part.
- Its coverage table (`coverage::rof2`, 6.508): a row for each message a `RoF2` stage reads,
  and nothing else, so the gap log tells every other message as unlisted. A test holds the
  rows to the stages' own opcodes in both directions.

Server-software checks follow the same rule. A comparison such as
`protocol == ServerProtocol::EqEmu` would silently answer "no" for a second `EQEmu` server type.
So these are exhaustive matches too, and the guard test covers `ServerProtocol` as well.
