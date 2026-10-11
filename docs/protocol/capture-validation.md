# P99 capture validation

This pass compares the codecs with an official-client P99 session and the pinned
public server revisions in [source-derived coverage](source-derived.md). Capture
replay is offline. It establishes packet layouts for the observed cases, not
successful live execution by this library or complete support for a subsystem.

## Additions confirmed by the recording

| Path | Evidence | Implementation |
| --- | --- | --- |
| Colored system text, `0x0b2d` | Two messages; u32 type followed by NUL-terminated text | Existing `ChatEvent` with system channel, original message type and structured item-link handling |
| No raid, action 10 of `0x1f21` | Six 136-byte notices | `RaidUpdate::NotInRaid`; clear membership without inventing a disband event; stage admission notice until ready |
| Zone description, `0x0920` | Six 700-byte records | `WorldEvent::ZoneEnvironment`: names, IDs, sky/fog, seasonal weather, gravity, experience multiplier, safe point and view distances |
| Cursor coin destruction, `0x7657` | Cursor source 0 and destination -1 in the official request | `CoinDestruction`, `Request::DestroyCoins`, `GameCommand::DestroyCoins` and session wallet handling |
| Ordinary door click, `0x043b` | Four requests with byte 1 equal to 3 and item ID equal to -1 | Encoder preserves this marker and no-item sentinel |
| Manual food/drink consumption, `0x77d6` | Two requests with the reserved word at offset 8 equal to -1 | Corrected manual encoding; automatic encoding unchanged |

Zone fields retain wire units where semantics are not established. Safe-point
coordinates are normalized from Y/X/Z to X/Y/Z; the event does not reposition
the player, change physics, or treat seasonal settings as current precipitation.
Unknown zone bytes and the redundant character name are not exposed. Interpreted
floats must be finite and the fixed record must be exactly 700 bytes.

The absence notice also accepts the 140-byte common EQEmu structure that its
Titanium adapter passes through. Only the 136-byte form was observed on P99.
Other lengths are rejected. The click marker is preserved without assigning it
an unverified meaning, and keyed-door behavior was not captured.

Coin destruction is restricted to a positive, signed-wire-range amount already
held on the cursor, without denomination conversion. It uses existing inventory
capability, queue, session and expiry checks. The session prepares a candidate
wallet and commits it only after a successful send. Unsupported generations and
invalid amounts send nothing and leave balances intact. No UI command is added.

## Byte comparisons and replay

Reconstructing selected outbound requests through the library produced **43
byte-for-byte matches**:

| Request | Matching records |
| --- | ---: |
| Door click | 4 |
| Inventory move | 16 |
| Taunt | 8 |
| Combat ability (kick) | 9 |
| Coin move/destruction | 4 |
| Manual consumption | 2 |

Before the encoder corrections, the door and manual-consumption reserved fields
differed. Matching these packets does not establish parity for all commands,
locked doors, automatic eating/drinking, or other client generations.

Final passive replay decoded 1,301 spawn records and 13,711 other messages with
zero application/spawn decoder errors. The transport audit validated 14,313
protocol datagrams with no remaining CRC rejections, reliable sequence gaps or
fragment omissions. These are record counts across characters and zones, not
unique players or actions. A six-second interval between the two recording files
is missing, regardless of the clean sequence audit.

The private capture reader needed to separate coalesced records, validating each
constituent's keyed CRC. No production transport change was justified by this
capture representation. Session decoding context was obtained from the recorded
world connection and retained across the two files.

## Remaining evidence gaps

The zone census still has 41 unhandled messages:

| Opcode | Count | Disposition |
| --- | ---: | --- |
| `0x0587` | 12 | Empty admission experience acknowledgement, consumed by admission |
| `0x1234` | 6 | Unclassified 800-byte P99 payload; leave uninterpreted |
| `0x4665` | 6 | Tribute compatibility timer; no tribute feature added |
| `0x5639` | 6 | Tribute compatibility record; no tribute feature added |
| `0x5996` | 6 | Empty AA admission compatibility notice |
| `0x711e` | 5 | Empty pre-logout notice; no additional gameplay event inferred |

Appearance kind 16 is also retained as an unknown subtype; spawn assignment
already has a separate admission path. Opcode recognition alone is not a
completeness score. The sample includes combat, death, inventory, spell, merchant
and loot events handled by existing readers, but does not prove all fields or
rejection cases for those systems.

Useful next samples are normal player trade and cancellation, group changes,
bank operations, trainer use, tradeskill combinations, buff fades/control
effects, and resurrection or translocation with a willing helper. Use short
timestamped scenarios and a fresh connection for session context. Prioritize
missing scenarios over repeating movement and inventory coverage already present.

## API and testing boundary

Downstream exhaustive matches must handle `WorldEvent::ZoneEnvironment`,
`RaidUpdate::NotInRaid`, `GameCommand::DestroyCoins` and `Request::DestroyCoins`,
along with the source-derived additions. These belong to the next breaking
pre-1.0 minor release. Consumer dependency pins and UIs are not changed here.

`tests/captured_protocol.rs` contains independently authored synthetic records,
not captured bodies. It covers truncation, invalid lengths, non-finite settings,
unknown text colors, item links, coin limits and the corrected request fields.
Session tests cover admission delivery, raid reset, coin refusal and wallet
updates, plus stale-command checks. No account, session key, character name,
asset or raw packet from the recording is included in repository fixtures.

Public layout references at the pinned EQEmu revision include
`common/patches/titanium_structs.h` (`NewZone_Struct`, `ColoredText_Struct`,
`ClickDoor_Struct`, `Consume_Struct`, `MoveCoin_Struct`), `common/raid.h`
(`raidNoRaid`), `zone/client.cpp` (`SendColoredText`), and
`zone/client_process.cpp` (`OPMoveCoin`). No server implementation code is copied.
