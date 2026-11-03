# API and integration contracts

## Typed spell-provider policy and attempt identity

`Client::spell_table_policy()` returns `ProviderPolicy` before connecting,
resolved through the same selected server policy that owns table emission.
Hosts bind this value when creating their producer; no protocol-name branch
or first-table ordering assumption is needed. Other policies retain installed
definitions without inheriting a replay overlay.

`SessionStatus::run_id: Option<u64>` carries the numeric attempt ID shared with
`spell_tables::Context::run_id`. A real Connecting notification establishes
the attempt; a table's ID alone cannot rebind the host. It remains stable
across world/zone transports and changes on an actual retry. Provider epoch
is independent and source snapshots survive same-provider transport changes.

The string-taking `SessionStatus::new(state, session_id)` is already present in
the [v0.1.2 release source](https://github.com/eq-p99-tools/eq-network/blob/v0.1.2/crates/eq-network/src/client.rs).
Its signature and display label are preserved. It leaves `run_id` unknown;
`with_run_id(id)` supplies independently known identity. The extensible status
struct gains an additive field; display-only serialization omits that field.
There is no text parsing, hashing, default numeric ID or fabricated authority.
The production emitter always supplies Some from the attempt's sequence owner.

Known consumers checked here are the network production status emitter, network
synthetic lifecycle tests, and the graphical client's three bridge fixtures.
The latter now supply explicit typed IDs. The older standalone logger defines
its own status type rather than calling this constructor. Other external
consumers remain compatible with the old string constructor but must opt into
typed identity before using it as run authority. Hosts must treat an unknown
ID as unavailable for identity-dependent actions, without aborting chat or
other connection processing. `session_id` remains display/logging text only.

## Equipment contribution uncertainty

`Inventory::equipment_stats_uncertain()` tracks stat-bearing equipment histories
whose decoded direct bonuses or worn effects might change. Only readable rules,
explicitly zero direct bonuses and no worn effect establish neutrality. Affecting
or missing metadata stays uncertain through all intermediate local moves, even
when final placement equals its origin. Partial slot records carry no operation
revision and cannot retire this history, even when matching current placement.
A full `Snapshot` or existing policy-driven `Settled` starts a new contribution
baseline; this adds no EQMac settlement timer. Existing origins and ordered
predictions keep their slot-coverage behavior independently. The gap for affecting
or unreadable equipment remains conservative until such a baseline. The accessor
never establishes carried weight, receipt or server acceptance. It must be used
with a received, non-stale model whose complete history was recorded from its
baseline; cloning preserves that history, endpoint reconstruction does not.

## Ordered host inventory projections

`InventoryMove::sequence` optionally names a monotonically increasing local
request and its preceding queued move. Snapshot-based callers set it to `None`.
Projected callers advance the same `Inventory` planner locally after enqueue,
retain exact revision checks, and submit commands in order. A failed predecessor
cannot authorize a descendant whose revision happens to match a later update.
These identities are admission-local host metadata and add no wire fields.

`WorldEvent::InventoryAction` now carries the echoed sequence and an optional
post-send prediction. Apply that prediction once on successful local submission;
the worker no longer emits it as a separate `WorldEvent::Inventory` event.
Refusals have no prediction. Neither a local result nor the ordering identity
confirms server acceptance. Downstream request literals and event matches must
be migrated; this is a breaking development API change for the next minor
pre-1.0 release. Authoritative inventory updates remain ordinary inventory
events, with existing uncertainty, correction and snapshot rules unchanged.

## Local loot request disposition

`WorldEvent::LootRequest` adds an exhaustive event variant. Its `LootRequest`
describes the original open, item or close action, retaining corpse ID and every
item-request parameter; `session_id` is the requested admission, even for a stale
command. `created` echoes the original open/item creation `Instant` for local host
correlation. Close has no creation time in its existing command. The process-local
`created` field is omitted from serialized event logs; it is not a wire timestamp
or a new transaction identifier.

`LootRequestStatus::Refused` means nothing was sent. Shared admission/freshness/
policy checks, corpse/list/cursor guards and generation encoding all use this
single result path. The existing diagnostic is retained once for logging; hosts
should handle the typed result once rather than also presenting the diagnostic.
`Sent` follows successful transport submission only and proves no server
acceptance, inventory delivery or coin change. Transport failure remains an error,
without fabricating either disposition.

Hosts should match the admission, original action and available creation time
before clearing a pending request, and keep stale results from changing a newer
request. Local refusal must not retire an item-view row, fabricate a closed
corpse or trigger an automatic retry. Server `LootUpdate::Opened` denials,
`Taken { accepted: false }`, EQMac receipt echoes and completion remain on their
existing server-update path; an unqualified completion does not identify a cause.
Ordinary inventory/coin events remain the only holdings authority. No packet,
capability, inventory owner or automatic EQMac loot-all behavior changes here.

## Language-zero name selector

`WorldRules::language_zero_selector: Option<rules::LanguageZeroSelector>`
retains the complete received Titanium `OP_LogServer` first dword. The
wrapper's zero/nonzero test chooses between installed language-name strings;
no broader server feature is inferred. `None` is absence, while
`Some(LanguageZeroSelector(0))` is a received zero. Consumers must not use the
separate PvP or no-drop-trading values as substitutes.

The existing `WorldEvent::Rules` route carries this field. The EQMac decoder
leaves it absent pending verified wire mapping. Consumers retain the setting
with the same world's rules across zone/character admissions, replace it on
a received value, and clear it when that rules owner starts a new session.
An absent field in a later partial rules copy does not erase a received value.
Installed data remains the source of text. Complete `WorldRules` literals
must add the field or use its default for absence; this source API addition
requires downstream consumer validation before publication.

## EQMac admission zone identity

`quarm::profile_zone` reads the current zone from the supported encrypted
profile. Zero returns `None`; malformed layouts and values outside the
transfer interface's `u16` range are errors, never truncated aliases. The
bind point and zone-description name do not substitute for missing identity.
EQMac admission commits the validated current zone with no instance field
before zone-line commands can run. Unknown identity retains the existing
transfer refusal. No transfer packet layout or server policy changes.

## Camping and the movement owner's pending stop

`MotionSession::stop_pending` observes movement, a held moving update, and a
last moving submission whose real stop has not yet succeeded. Stationary
heartbeat repeats do not block camping. `is_trailing_stop` classifies a newer
same-admission request at the owner's exact point/heading; normal guard,
codec and submission checks still apply. Neither method sends traffic or
establishes a server acknowledgement.

`stop_context` supplies an equality-only admission/input-epoch guard for a
waiting intent. `accepted_stop` observes the real committed request stamp,
so a rejected trailing stop cannot later start camping on an idle heartbeat.
An accepted held stop still waits for its actual submission. These queries
do not infer success from the host's `MotionSent` notification.

Camp retains its own waiting intent while the existing paced stop is pending,
then starts its countdown only after camp submission succeeds. New movement,
refusal, withdrawal or admission/lifecycle loss cancels that waiting intent.
The established motion timer precedes the camp timer. Official stop/camp
ordering remains unverified under recording checklist6.380; no spacing,
repeat, wire format or movement authority changes.

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

`doors::eqmac::decode` and `objects::eqmac::decode` expose EQMac layouts through
the existing typed door/object updates; `message::eqmac` routes them on the
generation wire. `objects::eqmac::pickup` and `request::eqmac(Request::PickUp)`
encode the established EQMac click body. Server policy still controls whether
the session offers pickup or door interaction. This adds no event variants or
inventory writer. EQMac's refusal sentinel becomes the existing
`ObjectUpdate::Kept`, which identifies the actor but cannot identify the object.
`ObjectAction { error: None }` still means transport submission, and disappearance
still means visibility changed; only authoritative item packets grant inventory.
Requests older than an object's replacement are refused without a send. See
[the evidence and activation limits](docs/eqmac-world-objects.md).
One unresolved EQMac pickup holds only further pickup requests; unrelated
inventory actions retain their existing guards. Another actor's or unattributed
removal of the pending object, an authoritative inventory snapshot, or a
three-second local wait expiry cancels that hold without acknowledging success,
granting an item, settling a move, or sending a retry. Expiry is a local
duplicate-suppression bound, not a server response guarantee. After cancellation,
constant-size admission-scoped state records that late untagged/cursor results
cannot uniquely answer a newer pickup, even with a reused object ID. The newer
wait can still be cancelled by an authoritative snapshot or its own expiry;
ambiguous removal packets cannot cancel it. Only a new admission restores unique
result attribution.
The result rule is selected through the generation interface; the established
Titanium path is preserved. TAKP and Quarm offer `GroundItems` with this rule.
Their door definitions/actions are replayed at admission, but no `Doors`
capability, door-click command owner or unverified door sender is enabled.

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

## Movement-input acceptance

`GameCommand::SpeedInputs` carries the current admission, the opaque revision
from `WorldEvent::SpeedInputsRequired`, an opaque host `report_id`, and a typed
ready/unavailable state. Give each new logical report a fresh report identity;
retain that identity when retrying a full local queue. Recovery to identical
ready values after an unavailable report is a new report. Neither identity is
a wire counter, and consumers must not derive movement permission from it.

After normal admission/revision validation and generation-rule evaluation,
`WorldEvent::SpeedInputsAccepted(SpeedInputAcceptance)` echoes the exact ready
inputs and report identity with `granted`. A new report is acknowledged even
when its numeric calibration is unchanged. A later change in the accepted
report's grant publishes a new outcome; unchanged ticks do not repeat it.
Unavailable or obsolete reports never produce a ready acceptance. `granted`
is false without a current positive calibration or while lifecycle blocks
movement. This is a local session receipt, not a remote server acknowledgement.

Keep this receipt separate from `MotionState`, which alone carries movement
speeds. Withdrawals, input requirements and admission/lifecycle resets invalidate
cached acceptance. A frontend may clear a waiting status only for the exact
posted admission/revision/report/input identity with a current positive grant;
a cached prior speed, successful enqueue, or mismatching receipt is insufficient.

Migration: add `report_id` to speed-input command literals and handle the new
world-event variant in exhaustive matches. This changes the unreleased public
API and requires downstream validation; it is not a compatible 0.1.x patch.

## P99 spell-table facts and host providers

`spell_tables::decode` reads variable-length P99 source-ID registration and
destination/source replay records. `SpellId` validates the known 10,000-entry
registry domain for these messages only. Counts, full lengths and every ID
are validated before output allocation. Entry order and duplicates are kept;
an error returns no valid prefix. The packets carry IDs, not definition values.

`WorldEvent::SpellTable(spell_tables::Event)` is a new exhaustive-enum variant;
consumers must handle it. The P99 server policy emits complete tables or typed
unavailability directly from world and zone receive paths, before ordinary
profile/admission filtering. Other server policies do not consume these opcodes.
A failed event callback remains a session failure. No packets are sent for
these messages, and no capabilities or motion rules change.

An event's run ID/revision describes network order, not an installed-provider
epoch. It spans successive world/zone transports in one attempt; zone context
already names the admission session before its profile arrives. A new network
attempt may start new ordering without clearing the host's provider snapshots.
The run ID formatted as sixteen lower-case hexadecimal digits equals that
attempt's `SessionStatus.session_id`. The actual Connecting notification follows
the run reset; a host binds a successor from that lifecycle fact, never merely
from a new run ID appearing in table data. Producer epoch/policy must already
be captured before enqueue so a stale Connecting cannot rebind a replacement.

The host owns its installed-spell provider, typed cached projections, effective
definitions and a separate epoch/policy binding on its event bridge. Under the
chosen lifecycle policy, snapshots survive zone/world transport reconnects in
the same logical provider lifetime. Explicit provider replacement starts a new
epoch requiring actual registration and must not leak overlays to another
installation or server policy. This is an application lifecycle policy, not
a claim that every native teardown path was established.

Registration snapshots a first-seen source's CURRENT effective projection;
duplicates do not refresh it. Replay uses cached source projections in wire
order, retains destination ID/text and every excluded field, and must validate
the entire batch before atomically publishing changes. It must not copy an
entire source definition, chase live aliases or restore omitted destinations. Missing
source data and malformed events must surface as unavailable, never a silent
fallback or partial application. Hosts must route these facts before their
own player/profile gates. This networking API applies no installed definitions.

## Checked local movement preparation

`MotionSession::send_move_sample` keeps its existing signature and result
semantics. Internally it prepares one checked candidate while exclusively
borrowing its owner. Validation and the generation release decision precede
any send. Held and unchanged candidates commit locally without sending;
immediate candidates commit movement only after the existing send callback
succeeds. A checked candidate is not yet an accepted move or a server receipt.
Dropping it or failing the callback leaves motion state untouched; external
callback effects cannot be rolled back. No public observer/transaction API is
introduced, and the private plan certifies no external body or scene identity.
