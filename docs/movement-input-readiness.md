# Movement input readiness

The session owns movement permission and the generation's speed formula. The
host owns the installed spell data and calculates the carried load, effective
STR and movement modifier. Missing input is a state, never numeric zero.

## Host API migration

On `WorldEvent::SpeedInputsRequired { session_id, revision }`, send
`GameCommand::SpeedInputs { session_id, revision, report_id, state }` (also
exported as `ClientCommand::SpeedInputs`). The report fields are defined in
[`command.rs`](../crates/eq-network-game/src/command.rs); the input and receipt
types are in [`movement/inputs.rs`](../crates/eq-network-game/src/movement/inputs.rs).

`SpeedInputs` contains `rate: i16`, `weight: u16`, `capacity: u32` and
`eqmac_fatigue: Option<EqMacFatigueInputs>`. The state is one of:

| State | Session behavior |
| --- | --- |
| `Ready(inputs)` | Replace the inputs with the complete report and evaluate the grant. |
| `Bounded { inputs, missing }` | Use the reported inputs while retaining the unresolved category. This can establish a grant even without an earlier one. |
| `Unavailable(missing)` | With an active grant, keep its inputs and grant without a timeout. Without one, withdraw input and wait for a usable report. |

The host evaluates each provable term now and omits terms it cannot evaluate.
The desktop retains positive equipment/food or buff terms from its earlier
same-admission report where the corresponding state is unknown, and applies
known current roots, snares and weight. A known ended speed buff ends at once.
This bounded-input policy can temporarily allow more speed than the original
client until all inputs are proven (recording checklist 6.451). Neither a bound
nor a retained grant establishes inventory placement or delivery.

Send a changed state even if the revision has not changed. Allocate a fresh
`report_id` whenever the state or revision changes, including identical-value
recovery after an unavailable report; retries of one logical report reuse its
identity. Retry a full command queue. Reports from another admission or obsolete
revision are rejected. `WorldEvent::SpeedInputsAccepted` reports the local
session's exact input identity and whether it currently has a positive grant;
it is not a server acknowledgement. A root has no positive grant.

Admission needs fresh inputs. Inventory/coin changes, own buff changes, spell
landings, level changes and server position corrections request a fresh
revision. When a grant is already in force, these rechecks preserve its speeds
while the host prepares a report; a late report is logged without expiring the
grant. A server correction restarts prediction at the new position while held
input continues. Inventory revisions include predictions and invalidation, and
the session compares belongings before accepting motion commands and on ticks.
HP, sneak and posture remain session-owned inputs and still apply immediately.

On EQMac, `EqMacFatigueInputs { strength: i64, stamina: Option<u8> }` carries
effective strength before fatigue/final capping and effective stamina capped to
1..255 when known. The session applies its received fatigue before the final
STR cap.
Unknown fatigue or stamina uses the maximum ten-point penalty. When this
component is present, a fatigue-only update changes the grant without changing
the host report or revision; `capacity` is a stable maximum-penalty fallback.
With `eqmac_fatigue: None`, the legacy capacity and fatigue-triggered report
refresh remain. Titanium ignores the component.

A root grants no travel speed while preserving turning input. Stun, freeze,
posture, death and zoning still impose their own holds. When input is actually
withdrawn, the frontend clears autorun, taps and queued jumps. Stationary
heartbeats remain enabled; supported passive descent must keep X/Y/heading
unchanged, without ascent and within the existing fall/time/coordinate guards. Death and
transfer suspension stop motion entirely. `ConfigureMotion` cannot bypass the
input-report contract on generations with a known speed formula.

The speed formulas are unchanged: capacity zero still means no encumbrance;
`rate == -1000` means an actual root. Neither is an unavailable-state sentinel.
The existing 42.667 conversion, HP/sneak rules, caps and snare floor remain.

A zone sends its whole inventory, when it sends one, before the weather the
client waits for to request the zone, on the same ordered stream; admission
ends later on every generation. When a zone sent no inventory message by
admission, the player holds nothing, and the session logs a diagnostic. Stock
`EQEmu` sends a Titanium client no `OP_CharInventory` exactly when there is
nothing in it: `Client::BulkSendInventoryItems` puts one entry per item held
(and banked, unless `Inventory:LazyLoadBank` is on), and the Titanium encoder
drops a packet with no entries. It also sends none when the character's
inventory fails to load from its database. The Titanium program doesn't
wait for the message either: it enters the zone once the zone server is
connected, and takes the inventory whenever it arrives (read R49-36 a).
Whether it clears the last zone's items at zone-in, so that it too then
holds nothing, is unread (recording checklist 6.437). With
`Inventory:LazyLoadBank` on, a character holding nothing but with a bank gets
no message either, so the empty snapshot wrongly says the bank is empty too
(an open follow-up). An
unreadable or partial inventory at admission is news and stays unavailable; a
later snapshot replaces the empty one.

## Weight and fatigue data

`Inventory::carried_weight_tenths(generation, coins)` requires a complete,
non-stale inventory. It includes equipment, general slots, their contents,
the current cursor and cursor bag; excludes bank, trade and queued cursor items;
and counts a stack's item weight once. `coins` counts carried denominations,
not their copper value. Coins contribute `coins / 4` tenths.
An absent cursor/bank profile projection is not published as empty coins; a
purse-only update cannot make the host's complete carried coin state ready.

Titanium reduces each bag child separately; EQMac reduces the aggregate bag
contents. EQMac coin-purse items 17201..17230 reduce coin weight only in general
root slots; the strongest reduction wins. Titanium omits coin-purse and Packrat
reductions, conservatively retaining more weight. Malformed/orphaned bag contents
are unavailable. Pending item moves are usable only if reconstructing the
prediction origins produces the same total, including coin-purse effects.
This method does not make predictions authoritative or settle them on a timer.

After a pending Titanium ground drop, the session independently checks the
report against the current carried-weight bound, including possible queued
cursor arrivals. A report below a known bound is raised to that bound and marked
`Bounded` with inventory unresolved. If no bound is available, a new grant is
refused; an existing grant can continue, keeping its weight as a floor while
applying usable reported terms. This does not acknowledge the drop or any
predicted placement. Fall-weight availability is a separate calculation.

Equipment contribution uncertainty is independent of carried weight. The inventory
owner remembers every stat-bearing slot whose unresolved history included nonzero
direct bonuses, a worn effect or missing metadata. Such a history stays gated
after a local round trip; equal final placement and silence cannot erase it.
Partial slot records carry no request revision and cannot retire affecting
history for a potentially newer pending move. A full snapshot (or the existing
policy-driven settlement where already supported) starts a new contribution
baseline. EQMac adds no timer; affecting/unknown equipment can remain unavailable
until a new snapshot. This deliberately does not claim general equipment
readiness. All transitions must have been recorded by this owner from the
baseline; its clone carries them, while endpoint reconstruction is no proof.
Known rules with explicitly zero direct bonuses and no worn effect are neutral
across that history, so they do not require withholding unchanged contributions.
This proof does not confirm a move, fill in missing definitions, establish load,
or grant movement without the session's normal input acceptance and calibration.

These calculation and authority rules do not offer inventory actions by
themselves. The [server policy](../crates/eq-network/src/client/session/servers.rs)
separately selects item/coin moves, merchants, meals, splitting and banker change.
P99 and Titanium EQEmu offer splitting/banker change; Quarm offers merchants and
meals but not those two actions or exchanges. Cursor dropping is offered only
for local-only Titanium EQEmu, and submission also checks the current admission,
actor, item and carried-coin prerequisites.

The item decoders retain bag weight reduction (EQMac byte 272; Titanium serialized
field 99). `Nourishment::fatigue` is optional: EQMac stamina/profile packets
preserve it; Titanium leaves it absent. The EQMac profile offsets are hunger 3416,
thirst 3420 and fatigue 4962. Admission republishes known nourishment without
enabling food-consumption actions.

## Public evidence and limits

New plain cursor arrivals after whole-item carried moves can use a conservative
weight bound while all placement predictions remain unresolved. For example,
an item weighing 3.7 moved through carried slots plus a new 5.0 arrival yields
an 8.7 bound; a server prefix that still has the arrival queued weighs less.
The bound does not certify the old item's endpoint. Set covers only the
reported root; Cursor does not acknowledge its earlier origin. Repeated IDs,
non-plain arrivals and ambiguous occupied-cursor traces remain unavailable. Every local
intermediate mass/purse change is recorded, so an affecting bank round trip
cannot become known merely by returning to the same endpoints. A later
unsupported change to rebased slots needs a full baseline; no EQMac timeout
settles that history. `carried_weight_is_conservative()` reports this property
separately from placement, equipment readiness and movement permission.

- [Quarm weight, STR and fatigue rules](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_mods.cpp):
  `CalcCurrentWeight`, `CalcSTR`, `CalculateFatiguePenalty`.
- [Quarm equipment and spell bonuses](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/bonuses.cpp)
  and [EQMac structures](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/patches/mac_structs.h).
- [EQEmu Titanium weight rounding](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/client_mods.cpp)
  and [item serialization](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/common/patches/titanium.cpp).
- [TAKP weight rules](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/zone/client_mods.cpp).

These are public-source rules and synthetic protocol tests, not live-server
compatibility evidence. Applying Titanium/EQEmu item rules to P99 is an inference;
private P99 modifications are not established here. The existing coin ledger
includes validated client coin transfers because these protocols do not send an
acknowledgement for each transfer. Adding cursor coins follows the installed
client model; the cited server weight functions explicitly list purse coins.

Final stack integration must preserve Quarm's unresolved prediction origins and
its correction-until-full-snapshot rule. A timeout must not produce authority.
The client companion documents calculation limits and the deferred live matrix.
