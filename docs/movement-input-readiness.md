# Movement input readiness

The session owns movement permission and the generation's speed formula. The
host owns the installed spell data and calculates the carried load, effective
STR and movement modifier. Missing input is a state, never numeric zero.

## Host API migration

On `WorldEvent::SpeedInputsRequired { session_id, revision }`, send
`GameCommand::SpeedInputs { session_id, revision, state }`. `state` is either
`SpeedInputState::Ready(SpeedInputs { rate, weight, capacity })` or
`Unavailable(SpeedInputUnavailable)`. Send a changed state even if the revision
has not changed, for example after spell data becomes available. Retry a full
command queue. Reports from an obsolete revision cannot restore a grant.

Admission, server position correction, inventory/coin changes, own buff changes,
level changes and changed EQMac fatigue withdraw the cached inputs and request
a fresh revision. Inventory revisions include predictions and invalidation.
The session compares belongings before accepting motion commands and on ticks,
including changes published directly by the inventory owner. HP and sneak stay
session-owned inputs to the existing speed formula.

`Unavailable` clears held movement and the timing/jump epoch. Stationary
heartbeats remain enabled. Servers supporting falls accept only bounded passive
descent while input is withdrawn: unchanged X/Y/heading, no ascent, and existing
fall/time/coordinate guards. Jump and ordinary locomotion are rejected. Death and
transfer suspension continue to stop motion entirely. `ConfigureMotion` cannot
bypass this contract on generations with a known speed formula.

The speed formulas are unchanged: capacity zero still means no encumbrance;
`rate == -1000` means an actual root. Neither is an unavailable-state sentinel.
The existing 42.667 conversion, HP/sneak rules, caps and snare floor remain.

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
