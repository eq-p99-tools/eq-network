# API and integration contracts

## Quarm loot receipt and delivery

`LootUpdate::Acknowledged { place }` is a new exhaustive-enum variant and
requires downstream match updates in the next minor pre-1.0 release. EQMac
servers echo item requests before checking whether they can deliver them.
Consumers must not treat this event as `Taken { accepted: true }`: ordinary
`InventoryUpdate` events establish received items, including late deliveries
after the loot window closes. No destination or successful transfer is
predicted by the loot feature. The existing inventory feature owns items and
coin grants. See `docs/eqmac-looting.md` for evidence and remaining live checks.

The loot controller correlates a visible corpse request, successful grant and
completed listing in one admission. It refuses stale-session commands, places
outside that listing and repeat requests for a place already submitted. Close
waits for the server's completion; death, zoning, corpse removal and reconnect
discard access state. No timeout or receipt echo fabricates item delivery.

## Public API evolution

The published 0.1.x line and the repository's unreleased development head are
different compatibility targets. Compare with the latest published API before
releasing; the workspace version alone is not evidence of compatibility.

- Preserve exhaustive command/event enums in the current API. Adding a variant
  can break downstream exhaustive matches and must be treated as an API change.
- Compatible 0.1.x fixes may use a patch release. Accumulated breaking changes
  require the next minor pre-1.0 line, migration notes and consumer validation.
- Do not retrofit `#[non_exhaustive]` as a supposedly compatible fix. Adding it
  is itself a breaking change. Choose it deliberately for extensible public
  boundaries in the next version; keep internal dispatch exhaustive.
- Carry typed payloads, quantities, slots and capabilities. Introduce an ID
  newtype where it prevents meaningful mixing; avoid wrappers without an invariant.

## Feature ownership and recovery

Inventory moves do not hold the inventory while awaiting another move's reply.
`MoveRules::settles_by_silence()` preserves the legacy Titanium refusal window;
both EQMac policies require authoritative evidence and never settle on a timer.
`Inventory::unresolved_predictions()` counts ordered changed-slot projections
whose uncertainty remains, not positive move acknowledgments. A covering slot
update resolves only its slots across affected projections; it never confirms
an unrelated move by order. EQMac contradictions invalidate until `Snapshot`.
See [inventory prediction semantics](docs/inventory-move-predictions.md).

One feature owns each command and its state. Wire generation determines layout;
server policy determines availability. New features start absent, and a frontend
must honor the session's offered capabilities and explicit player choices.

Resource holds cover an operation until its answer settles it. Where a server
refuses with silence, as `EQEmu` refuses a merchant offer, the hold ends after a
timeout, and the operation is remembered with what it acted on so that a late
answer still settles it. A late reply must not mutate an unrelated replacement
slot or release a newer operation's hold. When the wire cannot distinguish
outcomes, invalidate and reconcile rather than guess, replay, or invent a server
request identifier.

Closing a window, dying, zoning and reconnecting need explicit feature reset
semantics. A new admission invalidates old commands and proposals; a UI close
does not by itself prove that a server transaction failed. Tests should cover
late replies after each reset and successful recovery from authoritative state.

Use `Instant` for elapsed time and command age, wall time for log records.
Transport/event callbacks must return promptly. Ordered inventory and lifecycle
events cannot be silently dropped; a callback error is a session failure, not a
cosmetic warning. Hosts need a bounded event bridge with explicit overload
behavior, independent of asset decoding and rendering.

## Error contracts

Keep contextual `anyhow` errors at connection, I/O and executable boundaries.
Use typed refusal/validation reasons where an application needs a behavioral
decision. Applications must not parse a diagnostic sentence to decide whether
to retry or replay an action. Preserve unknown wire values when they are useful
for forward compatibility; reject malformed lengths and invalid domain values.

Report a refusal only where the server refused, or where its silence is how it
refuses, and never treat transport send success as authoritative completion. A
timeout never authorizes resending a non-idempotent action.

## Validation and release integration

Normal combat requests emit `WorldEvent::CombatRequest { session_id, request,
status }`. `combat::CombatRequest` distinguishes `Consider { target_id }` from
`AutoAttack { enabled }`; `CombatRequestStatus::Sent` means transport submission
only. `Refused(reason)` means nothing was sent, including an expired request,
an obsolete admission or a server policy that does not offer combat. The reason
is display text; applications must use the status variant for control flow.
Exhaustive `WorldEvent` matches must add the new event. These are unreleased
API additions, not a compatible published patch.

No authoritative autoattack acknowledgment exists on the inspected EQMac path.
A host may display its requested toggle separately; it must not label `Sent`
as server-confirmed combat, a hit or a successful consideration. Only existing
`Consideration`, `Damage` and `Animation` events carry server results. A consider
answer names its own target; it must not be applied to whichever target the host
has since selected. Unknown con/damage values remain observable. Neither target
range nor weapon eligibility is recalculated by the normal toggle owner.
See [combat evidence and limits](docs/quarm-combat.md).

This library workspace intentionally does not commit `Cargo.lock`. CI resolves
dependencies for each toolchain: stable format, Clippy, tests and docs on Linux;
native tests on Windows. These checks
neither load proprietary assets nor connect to a live game server. A fresh
downstream resolution and packaged-crate consumer remain release checks,
because a downstream application owns its dependency lockfile.

For each changed feature, record protocol generation, server policy, repository
revision, capability and scenario. Separate source-derived behavior, synthetic
codec/state-machine tests, and live evidence. A check on one server type does not
list a feature on another: one checked on TAKP waits for its own check on Quarm.

Applications using a Git branch still resolve a specific commit in their
`Cargo.lock`. A networking merge is not integrated into an application until its
lockfile changes and that combination passes checks. Coordinate related open PRs
before advancing the pin. Never commit credentials, raw game captures, extracted
assets, or identifying account data as fixtures; construct synthetic records.

## Independent posture and locomotion

`GameCommand::SetPosture` requires `Capability::Posture`. `Moving` covers
locomotion and its speed inputs; it must not be required for sitting, standing
or ducking. All current server policies offer posture independently of
movement readiness. Hosts should gate stance buttons and hotkeys
using the command capability, and send the usual sit then camp sequence.

## Movement inputs are the host's, by revision

The session grants no walking, turning or jumping until the host answers the
latest `WorldEvent::SpeedInputsRequired` revision with `GameCommand::SpeedInputs`.
A host answers every request, with `SpeedInputState::Unavailable` and its
reason when it cannot yet calculate; an unanswered or older revision keeps the
player still, which is the slow side, never zero load. Hosts should surface the
`SpeedInputUnavailable` reason to the player. Exhaustive matches add
`SpeedInputsRequired` and `SkillReset`; `ItemPlacement` literals add `eqmac`
and `bag_weight_reduction`.

## EQMac inventory decoding policy

`inventory::ItemRules` selects the server's interpretation of the shared EQMac
item layout. `Takp` derives stackability from item type and maximum charges;
`Quarm` uses the flag at byte 276 and retains the same stack-capacity rule.
`inventory::decode_eqmac_with(opcode, body, rules)` and
`message::eqmac_with(opcode, body, rules)` accept that policy explicitly. The
session obtains it from its server type and uses one EQMac generation wire.

These APIs are additive. `inventory::decode_eqmac` and `message::eqmac` keep
TAKP semantics; `inventory::decode_quarm` and `message::quarm` keep Quarm
semantics. Both policies retain raw stack and GM observations in
`inventory::EqMacItemRules`, including unknown values. Decoding policy does
not enable inventory capabilities, grant a move, or change pacing, prediction
or reconciliation. Existing move rules remain responsible for validation.
