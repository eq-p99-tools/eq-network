# API and integration contracts

## Titanium lifts and door mechanisms

`doors::titanium` holds Titanium's own door rules, read from `eqgame.exe`:
`Vertical::of` (rise or sink, travel and speeds by open type, 0x4368b3), `Slide::of`
(type 15 and 78-81, 0x4366e8), `Solidity::of` (whether the floor sweep stands a
player on a door, 0x490370, and whether its wall stops them, 0x42ce5d),
`returns_after` and `returns_at` (the client's own return, 0x48faa5) and `field`
(door +0x66, the param's low 16 bits as an i16). The field stays as spawned; the
server message that changes it later (0x45e626) is not traced. `Solidity` is a
Titanium rule only; another generation keeps its own (`Solidity::BOTH`).

`Lifts` steps every vertical door from the received spawn and actions with the
program's rule, including the client's own return, so the session and a frontend
that follow the same updates agree on a door's height. Motion starts only on the
server's 0x700d action; a click sends 0x043b and predicts nothing. The return is
local and sends nothing, except for types 40 and 45, whose return the session sends
as `Request::ReturnDoor`: 0x043b with the door id, 0xfe and zeros, naming no player
(0x490b20, read R1c).

`MovementRequest::support` names what the frontend's floor sweep stands the
player on, as a `Support` (non-exhaustive; `Support::Door(id)` today), set with
`MovementRequest::supported_by`. While a request names the same lift, the session
keeps an account with it: the rider may rise in all as far as the lift has risen
since they first named it, less what they have risen while naming it, so a
frontend whose lift runs behind the session's spends late what it did not spend
early and never rises past the lift (`MovementGuard::lift_since`,
`MovementGuard::allow_lift_rise`, `Lifts::carry`). The door must be a floor the
program stands players on, with the last accepted position within `RIDER_REACH`
(310 units in 3D) of where the door stood at that sample, so a lift taller than
the reach carries its rider all the way. A request naming nothing, another door, or a lift that
sank over the interval closes the account; only a request that names a support
spends any of it. The rise composes with the ordinary grant: horizontal motion
keeps its speed budget, a still rider is passive, and a lift carries (or holds
at its top) a player whose input is withdrawn only straight up. Nothing
lift-specific is sent; the risen height goes out by the generation's ordinary
gate. `Lifts::returns_at` and `Lifts::sinks` give a vertical door's own return
time, from where it stood, and whether it sank over an interval.

## Friends who, guild requests, splitting and banker change

`EQEmu` and Project 1999 offer `Capability::FriendsWho`, `GuildRoster`,
`GuildManagement`, `BankerChange` and `Splitting`. Each sends what the
Titanium program sends, and the server decides what it allows.

- `GameCommand::WhoFriends` carries the host's friends list. The answer is an
  ordinary `WorldEvent::WhoList`. A list the packet cannot carry is not sent.
- `GameCommand::SplitCoins` names platinum, gold, silver and copper. It is
  refused outside a group, for nothing, and for more of a kind than the purse
  holds, each with the official client's string ID on `CoinsRefused`. It is
  withheld without a word while a banker's change is unanswered. A sent split
  takes the coins from the purse at once, as the official client does, and
  reports `WorldEvent::Coins`. If no money update follows within the wait,
  the purse becomes uncertain (`PurseUncertain`).
- `GameCommand::SetAutoSplit` sends the appearance update and reports
  `WorldEvent::AutoSplitSet`, as the server answers nothing. The Titanium
  profile's setting arrives at admission as `WorldEvent::AutoSplit`, so a
  host can toggle it as the official client does.
- `GameCommand::ChangeCoins` needs a banker within the bank reach. It holds
  coin moves until `WorldEvent::CoinsChanged` arrives, which the worker
  follows with `Coins` and `CoinsElsewhere`. If no answer arrives within the
  wait, `CoinsRefused` reports it and the coins stay as they were.
- `GameCommand::RequestGuildMotd` names the player's guild; outside one it
  is refused with the official string ID. The reply arrives as chat on
  `ChannelName::GuildMotdReply`, which the official client always shows.
  `ChannelName::GuildMotd` arrives unasked, and the official client shows
  it only when its text differs from the one shown last.
- `GameCommand::AnswerGuildInvite` answers the invitation of the inviter it
  names, once, with an `InviteAnswer`: accept, decline, or one of the
  official client's automatic answers (in another guild, another dialog
  open, the rank already held). The host decides which, as the official
  client does from its dialog and its state. `GameCommand::GuildInvite`
  says whether it offers an officer's rank. `SetGuildMotd` with 512 bytes or
  more sends an empty message, as the official client does. Guild removal, leader and demote requests name a member of the
  list the server sent, spelled as it spells them. The server decides who
  may do what and says so in its own lines.

## P99 self-bandage stock

A self-targeted P99 Bind Wound request can hold inventory while awaiting
stock evidence. A terminal reply or local expiry releases that hold.
`InventoryUpdate::BandageStockUncertain` lists possible spent slots still
awaiting a covering update. `Inventory::bandage_stock_uncertain` includes
a parent bag when any of its contents are uncertain. The older item count
stays visible and bounds carried weight from above. An expiry reports
`BindWoundUpdate::Unconfirmed { stock_uncertain }`. A received stock change
can settle the inventory side without resolving the bandaging outcome.
An expired P99 result retains ownership of its untagged reply until it
arrives or the admission ends. The held result does not extend the
inventory action hold.

## Station custody

`InventoryUpdate::WorldDetached` retires the displaced station's local
inventory. A matching `ObjectUpdate::Closed` retires an open view.
Carried slots remain provisional until covered by received inventory.
A pack receipt for the same serialized whole item can resolve a single
direct cursor-to-station route, separately from the station clear.

## P99 local pads

A P99 host can provide `InstalledZoneBoundaries` in `ClientConfig`.
The worker corroborates the entered boundary with the current approach,
admission and indexed zone-point record where required. A same-zone pad
submits an ordinary position report at entry and stages a local landing.
`LocalPadCrossed` describes that placement. Continue to use received
relocation events for server-directed placement.

## Titanium dialogs and player inspection

Project 1999 and EQEmu offer `Capability::ServerDialogs` and
`Capability::PlayerInspection`. Present a `PopupOpened` offer using its
admission and serial. Pass both to `GameCommand::RespondToPopup`.
`PopupClosed` identifies which presentation to dismiss. The worker sends a
reply only for a supported choice with a response ID. Closing and expiration
are local, and a successful send does not establish a quest outcome.

`GameCommand::InspectPlayer` uses the current available `TargetState`.
`WorldEvent::PlayerInspection` carries the equipment names, icons and note
received for the matching nearby player. These fields carry no inventory
custody. `GameCommand::SetInspectNote` supplies the admission's known note
for automatic replies. A known empty string can be sent. An absent note
leaves the automatic reply withheld.

## Titanium observer sight

`WorldEvent::Visibility` retains the dialect and raw invisibility value.
Titanium spawn appearance carries optional raw invisibility and GM fields.
Hosts can install a `vision::SightCatalog` with `Client::with_sight_catalog`
to evaluate observed own buffs using their effective spell definitions.
`WorldEvent::ObserverSight` reports a change within one admission. Reset
this projection on entry or loss of the world. A known ordinary invisible
player or NPC with a known non-GM flag can be visible to an observer with
See Invisible. Other values and incomplete metadata stay conservative.

## Local EQEmu fixture cleanup

`GameCommand::DepopLocalNpc` carries the current admission, creation time
and reconciled `TargetState`. The chat owner accepts it only when the session
uses `ServerProtocol::EqEmu` with `ClientConfig::local_only`. At the send it
requires the same available target revision and a visible NPC. The normal
admission and freshness guards also apply. A local refusal is diagnostic.

The wire command is the fixed `#depop` chat line. The server acts on its
current target and sends any resulting world changes independently. The
target guard covers state the session has observed at the send. A caller
must verify ownership and observe the exact fixture's removal.

## Manual Titanium ranged attacks

An admission offering `Capability::RangedCombat` can take
`GameCommand::RangedAttack`. Pass the current available `TargetState` from
`TargetChanged`, a `RangedLoadout::current` snapshot from the same admission's
inventory, and the admission ID and input time. Reconciled target identity
includes its revision. Any intervening inventory revision invalidates the
queued loadout, including a same-ID replacement or ammo update.

The zone checks readiness, posture, casting/inventory holds, equipped items and
range before it sends. Stock EQEmu applies its class/level skill cap, so a raw
rank of zero or an unavailable rank does not settle eligibility locally. P99
retains a conservative positive-rank prerequisite in the recognized 1–252
range while its first-use behavior is unverified. Local recovery compares
time since the last submission with the current weapon delay. Bows use the
first confirmed eligible quiver in general inventory when its reduction
leaves more than one second. Haste and slow remain outside the local
estimate. The server applies its own timers,
targeting rules and combat resolution. The `CombatRequest` disposition reports
only local submission or refusal. Existing received damage, inventory and chat events
carry the outcomes. The server chooses whether and from which stack to consume
a projectile. Hosts should issue a fresh intent for each explicit input.

## Pet orders

`WorldEvent::OwnPet` reports the current pet's `PetIdentity` (spawn ID and
an admission-scoped revision) whenever the session resolves exactly one
living spawn the server marks as owned by the player. The revision changes
on every owned re-spawn, death, charm or ownership change, including a
duplicate spawn report of the same ID. Pass the latest identity in
`ClientCommand::Pet { pet }` and the current `TargetState` for orders that
name the target; the session refuses an order whose identity or target no
longer matches ("Your pet has changed") before anything is sent, and
`SelectTarget::expected_pet` pins a pet-specific selection the same way.
`PetCommand::GuardMe` is Titanium command 6. The Project 1999 provider
selects the classic order set, which excludes Hold, Feign, NoCast and Focus.
The EQEmu provider selects the expanded set. Local submission leaves
execution and any further restrictions to the server.

`PetCommand::project1999_available` is deprecated in favor of
`PetCommand::in_classic_order_set`. The compatibility alias returns the
same classification. The provider chooses the order set for each admission.

## Titanium swimming bound

Titanium EQEmu and Project 1999 offer `Capability::Swimming`. Quarm and
TAKP reject `MovementMode::Swim` samples. A swim sample is bounded by one
three-dimensional budget of `movement::swim_speed` of the unboosted grant,
with no run boost, and is refused while flying or stunned. The native wet
pair is read at Titanium `eqgame.exe` 0x4c052d and 0x4c0534, and the land
pair at 0x4c04c1 and 0x4c04cd. The shared three-axis budget, the vertical
stroke sharing the wet horizontal rate, and the absence of skill, race and
buff scaling are inferred (recording checklist 6.390). A submitted sample
establishes neither server acceptance nor water state. The host asserts water
from the installed zone.

## Local Titanium liquid reports

`Capability::ALL` grows from 50 to 52 with `Drowning` and `LavaHeat`;
`GameCommand` gains `LiquidDamage { session_id, hazard, amount, observation }`
and `WorldEvent` gains `LiquidHazardRevision { session_id, revision }`.
Only the Titanium `EQEmu` policy offers the two capabilities, and only on a
session whose connection is constrained to local endpoints; P99, Quarm, TAKP
and a remote `EQEmu` keep falls only and refuse `LiquidDamage` before
anything is sent. `EnvironmentalDamage` keeps the completed-fall path and
refuses every other hazard.

The session raises `LiquidHazardRevision` at admission and whenever an input
of the air or heat calculation changes: the player's own position, hit
points, appearance, posture, buffs and spell effects, death, inventory and
level updates, and zone transfer state. A `LiquidDamage` report is accepted
only when its `session_id` is the current admission, its `revision` is the
latest one raised, its `observation.position` equals the session body's
current X, Y and Z bit for bit, its `observation.sampled_at` is within 250
milliseconds, the player is admitted and not dead or transferring, its
`amount` is positive, and at least one second has passed since the last
accepted report of the same hazard. Anything else is refused with a
diagnostic and no wire change; the next revision is the host's signal to
sample again. An accepted report is sent as the Titanium environmental
record with damage type 251 (drowning) or 250 (lava); the Titanium program's
own liquid reports are unrecorded, so that layout is inferred (recording
checklist 6.392). `PlayerState.titanium_environment` carries the profile's
zone, base fire resistance and whether any alternate advancement rank is
present. Titanium `eqgame.exe` copies the profile dword at byte 12876 at
0x4b9f3c and uses it at 0x418756. The cited EQEmu Titanium encoder writes
a constant 25 there through its `unknown12864` block copy. The decoder
retains the received dword. The server owns effective resistance, health
and death. [Profile layout][titanium-profile] [EQEmu encoder][titanium-encoder]

[titanium-profile]: https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/common/patches/titanium_structs.h#L883
[titanium-encoder]: https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/common/patches/titanium.cpp#L1600-L1606

## Retained base held models

`ItemDetails.base_held_model` keeps the checked optional base held model from
a Titanium item definition. A later item packet for the same slot with an
empty model field replaces it with `None`; the received wear appearance stays
separate and takes precedence in hosts.

## Casting, spellbook edits and Bard song stop

The worker emits `WorldEvent::CastPending` with the submitted spell and
admission. When a received spell update answers that request, the worker
clears the pending notice before forwarding the update on the same ordered
event stream. A cast may begin under a different spell ID and end under the
requested ID. After five seconds without a matching update, the worker
instead emits `WorldEvent::CastRequestUnconfirmed` for the unanswered
request and allows a fresh manual attempt. That event reports only the end
of the local wait. It precedes any later submission in that admission,
including a manual retry of the same spell. The worker retains the last
timed-out spell and gem/item source until a compatible own start, an
answering result naming the spell, a fresh submission, death, or the end of
the admission. An unrelated own start can hold casting independently.
An own interruption carries no spell ID. It releases the active hold
and leaves any separate timed-out request intact. Timed-out provenance has
no wall-clock expiration within those lifecycle bounds.

A late start matches the request by spell ID. Stock TAKP also accepts
the documented Luclin port rewrite from 2062 or 2706–2709 to 2935. The
rewrite is unverified on Project 1999 and Quarm. A late 2935 there begins
an independent hold that can end on a result for 2935, own interruption,
death, or departure. The late-port recording case captures the requested
spell, own start, and spell-bearing result or interruption on each server.

The worker emits `WorldEvent::CastLateBegin` with the request spell before
forwarding the compatible own `SpellUpdate::Began`. Starts carry no
request ID, so overlapping retries with the same spell or alias remain
ambiguous. An already begun cast still awaits a matching result. Own
death clears cast state through the death event. Exhaustive `WorldEvent`
matches must handle `CastLateBegin` and `CastRequestUnconfirmed`.
[EQMacEmu start](https://github.com/EQMacEmu/Server/blob/d8d589a0ce486ee47ec990d3c3bbbf604b2572a1/zone/spells.cpp#L501-L511),
[EQMacEmu port spell IDs](https://github.com/EQMacEmu/Server/blob/d8d589a0ce486ee47ec990d3c3bbbf604b2572a1/common/spdat.cpp#L1305-L1311)

Stock EQEmu and EQMacEmu use spell-bearing `SpellUpdate::Mana` to
re-enable ordinary casts, including item clicks. That update settles a
matching cast when `keep_casting` is false. Standalone item-slot
`BarRefresh` remains a worker compatibility path. The paired host's
refresh handling covers memorized gems, so standalone item refresh is
outside the delayed-item completion contract.
[EQEmu spell bar](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/spells.cpp#L5377-L5390),
[EQMacEmu spell bar](https://github.com/EQMacEmu/Server/blob/d8d589a0ce486ee47ec990d3c3bbbf604b2572a1/zone/spells.cpp#L3945-L3957)

An unanswered book edit emits terminal `BookActionStatus::Unconfirmed` once
after 30 seconds and releases the spellbook hold. A fresh edit can proceed
if its expected reply is distinguishable from every timed-out edit in the
admission. The worker refuses a wire-identical retry until a new admission.
That includes a forget of the same gem even when the spell changed, because
an empty sentinel identifies only the gem. An in-zone slot update or book
snapshot changes the current state without retiring that reply ambiguity.
Late updates still go through the ordinary state appliers and produce no
result for the timed-out edit. A mode-2 update clears its valid gem slot
independently of spell identity, and repeating it against an empty gem keeps
that gem empty. Only the matching gem and spell or an empty sentinel can
confirm an active forget. A repeated application-level book swap can apply
the same delta again because its wire record carries only the two slots.
An unconfirmed scribe retains its inventory hold until a received whole-slot
removal or replacement accounts for the cursor scroll, or the admission
ends. A unit decrement or late book insertion alone cannot release custody.

Titanium EQEmu offers `Capability::StoppingSongs` and handles an explicit
`ClientCommand::StopSong { session_id, created }` for the current Bard.
The wire request is empty `OP_ManaChange` (`0x4839`), as implemented by the
[server handler](https://github.com/EQEmu/EQEmu/blob/master/zone/client_packet.cpp)
and [Titanium opcode table](https://github.com/EQEmu/EQEmu/blob/master/utils/patches/patch_Titanium.conf).
After submitting a stop for a tracked gem cast, the worker allows one
second for matching cast news. An own interrupt or matching mana or gem
refresh follows the ordinary cast path. With no such update, the worker
releases its local hold and emits `WorldEvent::SongStopUnconfirmed` with
the admission and released spell. When no cast is held, submission emits
the same event without a spell. This event reports local state only. It
leaves buffs and cooldowns to the existing server updates, and a retry
requires a fresh user command. An active item cast or a cast of unknown
source blocks song stop. A duplicate while the worker awaits the cast update
produces a diagnostic without another send. The P99 policy keeps song stop
gated.

An own start for a different spell during the stop wait supersedes the
stopped hold. The worker emits `WorldEvent::CastSuperseded` with the
old spell before forwarding the new start. Consumers should discard the
identified hold and its request alias while preserving a different pending
spell. A late start for an abandoned
pending spell is withheld during a retry of a different spell.
Titanium has no cast request ID for distinguishing same-spell starts
after a same-spell retry.

## P99 readable items

`GameCommand::ReadItem { session_id, revision, slot, created }` now binds the
intent to the worker inventory revision and enqueue time. `Book::from_item`
preserves numeric kind values rather than coercing nonzero flags into a book.
Actual Titanium item decoding retains unsupported read metadata for refusal or
rejects malformed numeric fields. Titanium text decoding requires a supported
note/book kind and terminating NUL. Submission does not supply `BookText`;
supported server text opens the result. See `docs/p99-reading.md`.

## Zone handshake and player admission

`Client::run_with_commands` requires a usable player snapshot after each zone
handshake, including transfers. The worker delivers `WorldEvent::Entered` and
staged world updates before `ConnectionStage::Ready`. If the wire handshake
completes but the player snapshot cannot be decoded, the worker closes the zone
without taking player commands, reports `ConnectionState::Disconnected`, and
returns `PlayerAdmissionError` without an automatic retry.

The receiver belongs to the host. Pending commands stay queued at the failed
admission and get no per-command refusal from that attempt. Use a fresh command
queue for a separate run. Interactive world or character selection polls may
discard leftover gameplay commands before another admission. If a pending
gameplay command reaches a ready zone, the worker rejects old admission IDs
where the command carries one. Legacy `SendChat` carries neither an admission
ID nor a creation time and can be sent from the later zone if it reaches that
zone's queue drain.

`Client::run` can still deliver communications from that zone. It sends the
additive `ConnectionStage::Recording` milestone with `Connected` transport
health. This outcome has no `Entered`, `Ready`, or live gameplay feature
processing. Servers offering movement retain the stationary heartbeat only
with the nonzero own-spawn ID and position read during the handshake (Titanium's
validated own spawn and saved profile position, or EQMac's assigned ID and own
spawn position). Transfer traffic or a death, despawn, or position update for
an already known own spawn withdraws the position repeat for that zone, even
if it arrived before handshake completion. The first zone offer in recording
mode yields a diagnostic, including when the offer cannot be decoded. The
listener continues on its current connection while the transport remains
active. The progress enum is non-exhaustive, and its new serialized value is
`recording`. Errors in the handshake itself continue to follow the existing
session error policy.

## Modern session termination

`eq_network_transport::modern::SessionTermination` is an additive typed error
inside the existing `anyhow::Result` APIs. `PeerDisconnected` retains an opaque
optional native reason; `OutOfSession` reports the established stateless control.
Neither reports acceptance or failure of an application request.

Once observed, the first termination immediately prevents all sends and network
I/O, and discards incomplete fragment/reorder and outbound work. Fully completed,
ordered applications before the terminal control drain exactly once in order;
later combined suffixes are ignored. After that prefix, repeated receives retain
the same terminal error. Explicit local close still refuses staged applications.
`Client::run` reports Disconnected once for an attempt ending in that error. With
reconnect disabled it returns the typed error; with reconnect enabled it retains
the configured delay before starting a fresh login. Unknown terminal reasons do
not establish permanent refusal. The existing permanent credential-refusal rule,
other retry timings and legacy EQMac contracts are unchanged.
Hosts can downcast the error or show its credential-free Display text.
`Transport::termination()` is an additive read-only accessor, defaulting to None
for other transports and forwarded by boxed transports. The zone stops timers,
queued actions and automatic replies while draining. A matching latched typed
send refusal from an observer is noted as not submitted and deferred until the
prefix drains; unrelated failures still propagate. Existing receive signatures
are unchanged. See
[evidence and packet limits](docs/session-termination.md).

## Camp completion evidence

`CampStatus::Camped` now means an actual `Message::LoggedOut` was received.
The additive `CampStatus::ClosedWithoutReply` instead names the documented local
logout-close deadline, with unchanged timing and local selection/desktop routing.
It does not confirm server logout, cleanup or a reconnect cooldown waiver.
Consumers must handle both local completion destinations and keep their evidence
distinct. Ordinary camp preparation and the explicit ExitAtOnce opt-in still
share the same logout/reply owner. A terminal error without a logout reply is
still an error, not either completion event.

## Meal command availability

`Capability::Meals` is separate from `Inventory`: it gates `Consume` and
`AutoEat`. The inventory feature offers it exactly when its existing
`Allowances::meals` is enabled. Hosts retain the local automatic-food preference
but post it only for an admission offering meals. Protocol and server-specific
allowance policies remain with the existing inventory owner.

## Command diagnostic identity

`GameCommand::name()` returns its exhaustive variant name as a static string,
without command fields. Shared refusal diagnostics use that name; their
structured result events retain the original reason and request identity.

## EQMac wear updates

`appearance::EQMAC_WEAR_CHANGE_OPCODE` and `eqmac_wear_change` decode the
12-byte `EQMac` record into the existing `WearChange`/`WorldEvent::WearChange`.
The shared EQMac message path delivers it for TAKP and Quarm. The player's
record and named spawns apply the received slot's material/model and tint;
zero selects bare armor or an empty held slot. Hosts consuming the typed event need
no new event variant. A visible equipment update does not acknowledge an
inventory move or create an inventory item.

The slot is byte 2, the 16-bit material/model is at 4, and the tint is at 8;
alignment bytes are ignored. Length must be exactly 12, the spawn ID nonzero,
and the texture slot 0 through 8. Titanium retains its separate 9-byte codec
and slot validation. See TAKP's
[`WearChange_Struct`](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/common/eq_packet_structs.h)
and [`Mob::GetEquipmentMaterial`/`SendWearChange`](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/zone/mob.cpp).

## EQMac note and book reading

TAKP and Quarm now offer `Capability::Reading`. Existing `ReadItem`,
`ReadRefused` and `BookText` events are reused. New game codecs are
`books::eqmac_request`, `books::eqmac_text` and `EQMAC_READ_OPCODE`.
EQMac item decoding takes the raw kind byte from BookType at offset 228 rather
than deriving it from Book at 230; unknown kind bytes remain numeric.
Only received text populates reading state. Encoding refusal remains a
local `ReadRefused`; transport errors remain errors, and submission does
not produce a fabricated read-success event or change holdings.

## Inventory planning policy and arrival weight bounds

`Client::inventory_move_rules()` returns `Option<MoveRules>` before connecting.
The selected `ServerType` supplies this same value to its worker inventory
factory. Hosts must use it for queued placement projections, rather than
assuming `EqEmu` rules from the installed generation. None means there is no
inventory movement policy; it is not permission to choose a fallback.

`Inventory::carried_weight_tenths` can retain a conservative upper bound after
a distinct plain cursor arrival follows a recorded weight-neutral history. An
originally occupied cursor additionally needs a plain whole-item relocation trace.
`carried_weight_is_conservative()` identifies that property state and does not
grant readiness. A server prefix where the arrival remains queued weighs less.
`can_rebase_cursor_arrival(update)` exposes the same read-only classification
to the session's resync check; callers still apply the actual update through
Inventory and retain all other correction guards. It is not move permission.
The old item's endpoint remains speculative: Cursor leaves its placement
origins intact; Set covers only its observed slot. Neither resolves other
prediction journals or equipment contribution history.

Every recorded prediction must preserve item weight and the coin-purse
coefficient in both dialects before this bound can be used. Affecting intermediate
histories remain unavailable even after equal endpoints; a full Snapshot or an
existing policy-driven Settled establishes a new baseline. Repeated IDs,
non-plain arrivals and ambiguous occupied-cursor traces cannot start this rebase. Later
unsupported changes to rebased slots invalidate the bound. No new silence
timer, packet meaning or per-move acknowledgement is introduced.

## Named character deletion at world selection

`WorldEvent::CharacterSelection` now requires the actual `connection_id` and
`deletion: Option<deletion::Rules>`. Each world connection has a checked,
process-monotonic generation; each published list has a fresh monotonic identity.
Hosts must preserve both in `deletion::Scope`, invalidate unanswered confirmations
on any new list, and invalidate all prior intents on reconnect. Do not invent a
default scope. `GameCommand::DeleteCharacter(Request)` requires a positive local
request ID, an encodable name and an exact repeated confirmation. The world owner
checks that scope, capability, exact unique listed identity and selection-stage
ownership before sending. IDs and confirmation text are never wire fields.

`WorldEvent::CharacterDeletion(Update)` carries the full original request and
`Progress`: `NotSent(Refusal)`, `Pending(Option<Observation>)`,
`Unconfirmed { reason, observation }`, or `ObservedAbsent { scope }`. An observation
names the received list and connection. Still present is not a correlated refusal;
absent is not a deletion ACK, causal success or database purge. No transport send,
silence, timeout or disconnect establishes absence. Exact intent replays never
resend. Ambiguous attempts hold creation/entry; the host must retain explicit
Back/Disconnect recovery. Ending the connection reports unconfirmed. The frontend
may report `NotSent(QueueUnavailable)` if it could not enqueue a request at all.
Exhaustive command/event matches and complete selection literals must migrate.

Pinned public source: [EQEmu world handler](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/world/client.cpp),
[TAKP world handler](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/world/client.cpp),
and [Quarm world handler](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/world/client.cpp)
refresh the character list after an ownership-checked operation, without a
correlated deletion response. TAKP/Quarm may mark instead of purge characters.
The native request is the selected ASCII name followed by NUL, not a padded
64-byte record (Titanium sender address 0x4f0e94; EQMac sender 0x53b21b). Titanium
uses application opcode 0x26c9. The library's big-endian legacy application writer
uses 0x5a40, producing bytes 5a40; current TAKP spells the host opcode 0x405a and
writes it little-endian. P99 server internals and live behavior are unverified;
its policy uses the evidenced Titanium request with observation-only outcomes.

## Typed spell-provider policy and attempt identity

`Client::spell_table_policy()` returns `ProviderPolicy` before connecting,
resolved through the same selected server policy that owns table emission.
Hosts bind this value when creating their producer; no protocol-name branch
or first-table ordering assumption is needed. Other policies retain installed
definitions without inheriting a replay overlay.

`ProviderPolicy::requires_server_tables()` is the network-owned semantic
predicate for whether installed mechanics require registration/replay facts.
Hosts use it for lifecycle routing and initial readiness instead of matching
a provider variant. This additive const method preserves the enum variants,
serialization and existing policy accessor.

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

## Ordinary cursor drops

`Capability::GroundDropping` offers `GameCommand::DropItem` on local-only
Titanium EQEmu. Public servers await separate validation of item serialization,
eligibility and cursor updates. Pass the current admission,
own spawn, inventory revision and exact received cursor item in
`InventoryDrop` after explicit user consent. Submission requires current
carried item and coin evidence sufficient to bound the load.
`Inventory::plan_ground_drop` requires a positively identified ordinary
droppable instance and an unqueued cursor. A predicted move settled by silence
leaves the drop unavailable until the server identifies the new cursor.
Stock EQEmu's ordinary move to cursor saves the slot without a matching item
packet, so a later received update or snapshot is needed.
[EQEmu move and persistence](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/inventory.cpp#L2210-L2230)

Titanium sends an empty `OP_GroundSpawn`. The server chooses the cursor item
and position. A successful `InventoryAction` with
`InventoryUpdate::GroundDropPending` reports local submission and fences
cursor custody. Apply it once at the matching admission and inventory
revision. A fresh received inventory snapshot releases the fence. Stock
Titanium EQEmu can send the object without sending cursor removal. A bare
cursor removal or quantity update identifies the slot only, and may refer to
another item after a silent drop. Preserve the last view and mark inventory
stale on such an update. Re-enter the world for a fresh inventory baseline.

During the hold, a received limbo item may already be the server's current
cursor. Both drop submission and bound calculations require known purse and
cursor coin balances. The worker receives the cursor balance with the bank
balance and forwards them in `CoinsElsewhere`. Bank coins add no carried
weight. On a complete, otherwise unambiguous inventory,
`Inventory::carried_weight_tenths` takes the heaviest of the displayed and
queued cursor groups, plus the ordinary non-cursor load. It counts one
possible cursor group. The worker validates fresh speed inputs against that
bound before granting active movement. A conflicting cursor update or
invalidation withdraws active movement until received recovery. Supported
passive descent and ordinary completed-fall reporting continue during the hold.
For a landing, `Inventory::ground_drop_weight_tenths` also preserves the
bound through slot-only reductions and complete cursor-only replacements,
including a temporary resync token. It retains the heaviest candidate seen.
An unreadable inventory update or a carried-slot change that can increase
weight invalidates this proof. A host with unknown load can report only a
landing cost established independently of load. A landing whose cost
depends on an unknown load during the hold is not reported. That is more
lenient than the official client, and the local-only EQEmu drop fence keeps
it off public servers.
[EQEmu cursor buffer](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/inventory.cpp#L916-L957)
[EQEmu weight calculation](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/client_mods.cpp#L720-L784)

Process received ground spawns and removals through the existing object
owner. The spawn carries no cursor instance correlation, and pickup delivery
still comes from received inventory. Downstream exhaustive command, capability
and inventory-update matches and `ItemPlacement` literals need migration.

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

## Session-owned EQMac fatigue

`SpeedInputs` adds optional `eqmac_fatigue: Option<EqMacFatigueInputs>` with
signed unpenalized `strength: i64` and effective `stamina: Option<u8>`.
Existing rate, weight and final capacity (including the explicit zero sentinel)
retain their meaning. Only EQMac with this component derives capacity from
current session fatigue; legacy absent components retain the existing host
refresh/withdrawal path. Other generations use final capacity directly.

Known stamina is capped to 1..=255; missing/zero stamina or missing fatigue
retains the conservative maximum ten-point penalty. Subtract with saturation
before the final 1..=255 cap. Known nonpositive strength never becomes the
zero/no-encumbrance sentinel. The host supplies one proven prerequisite
baseline and a stable conservative fallback capacity. Session-derived
capacity does not rewrite the exact accepted host report or its receipt.
Unchanged fatigue/derived speed stays quiet; true prerequisite invalidation,
root, correction, death and transfers retain their guards.

## EQMac admission zone identity

`quarm::profile_zone` reads the current zone from the supported encrypted
profile. Zero or a value outside the transfer interface's `u16` range returns
`None`, never a truncated alias. Malformed compression/layouts remain errors. The
bind point and zone-description name do not substitute for missing identity.
EQMac admission keeps an otherwise valid profile when its current zone is
unknown, and commits representable current zones with no instance field
before zone-line commands can run. Unknown identity retains the existing
transfer refusal. No transfer packet layout or server policy changes.

## Quarm death-bind handoff

Quarm's source-defined death-only bind offer produces admission-scoped
`WorldEvent::DeathBind` before the transfer. It carries lifecycle knowledge,
not a death packet: no killer, corpse or kill credit is synthesized. Ordinary
death packets retain their full details. Duplicate bind offers do not repeat
death or departure; dead state survives an approved departure. This
classification is enabled by the Quarm provider, not generic bind offers.

For Quarm only, approval of a solicited pending bind transfer to the exact
current zone accepts its documented reply stand-in 185. Character, instance
and success checks remain; normal transfers and other profiles retain exact
destination validation. The stand-in is not stored as the real destination.

## Movement proposal results

`WorldEvent::MotionSent` reports the worker's current accepted position and
the local outcome of the proposal. A locally accepted sample can be submitted
immediately, held for pacing, or left unchanged by the update gate. The event
describes worker state only. `WorldEvent::MotionRefused` reports a discard
without an accepted position to publish. Both events carry the original
admission and process-local `created` stamp. The stamp is omitted from
serialized logs. Match both fields to complete a pending proposal. A Move
dequeued by zone dispatch or selection has one local terminal result while
the event handler can receive it. A fatal zone error refuses Moves already
drained into the abandoned batch. Commands still in the host queue when the
worker stops have no per-request result. Exhaustive consumers must handle
`MotionRefused`. `MotionSent` constructors and exact field patterns must
include `created`.

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

A moving player stops only by the host's own request at the current point
(zero displacement and no turn), or by a stun, withdrawal or correction,
each of which holds a stop for the update gate. A gap without input, however
long, sends nothing new: the program's movement and sending share one loop,
so a stall there only delays the next update. A host that stops driving
movement while the player moves must submit that stop first.

Camp retains its own waiting intent while the existing paced stop is pending,
then starts its countdown only after camp submission succeeds. New movement,
refusal, withdrawal or admission/lifecycle loss cancels that waiting intent.
Cancellation reports `CampStatus::Abandoned { by: CampEnd::Other }` when its
admission/movement context is lost or a transfer arrives. It clears the local
intent before notification, so overlapping invalidation/transfer/death events
report once and a failing callback leaves no intent to replay. Player actions
retain their existing typed causes; no cancellation sends a camp packet.
The established motion timer precedes the camp timer. Official stop/camp
ordering remains unverified under recording checklist 6.380; no spacing,
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
Their door definitions/actions are replayed at admission. Both offer `Doors`
with a click owner restricted to ordinary EQMac types 0--8 and the documented
definition, empty-cursor, freshness, motion and reach guards. Hidden server
locks, movability and scripts retain authority; submission is not an opening.
Additive APIs `doors::eqmac::click` and `request::eqmac(Request::ClickDoor)`
encode the eight-byte ordinary no-held-item request with the player's ID.
`doors::eqmac::ordinary` identifies supported definitions, and `Hinge` exposes
their local motion/return clocks for the network guard and frontend pose.
Only received definitions/actions set those targets; a click predicts no state.
Specialized or unknown mechanisms remain visible without ordinary use. No
event variants or definition fields change.

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

Incoming world-update rejection diagnostics retain the original reason and
append `opcode` and `payload_len` for passive diagnosis. The length covers the
application body only, excluding opcode and transport framing. No body bytes are
added; message counts, typed events and decoder acceptance rules are unchanged.

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

Hosts can read `Client::consider_npcs_by_level` before connecting. Every
current server policy prefers the reply's color. The policy affects display
only and leaves the received color and faction available separately. The
`combat::Consideration::faction` docs give the wire order and cite the pinned
server swaps.

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

`GameCommand::FeignLanded` also requires `Capability::Posture`. It is the
host's statement that a spell with the feign effect (74) landed on the
player: only the host holds spell data. Send it only for a
`WorldEvent::SpellEffect` on the player's own spawn for which
`SpellEffect::took_hold()` holds (the one landing test: kind 231 and the
flag carrying 0x04, as Titanium's handler 0x47f614 tests it), whose spell
carries effect 74, and where `GameDialect::spell_feign_lies()` holds. A
server type's `spell_landing_flag` answers a different question, which
servers have been checked. The session changes the posture only: one lying report, or none
when the player already lies, with the movement in hand stopped; no feign
request and no auto attack change. With `PostureOptions::feign_standing`
it sends nothing and owes the next stand. Another spawn's id is refused.

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

A change in the player's belongings (inventory, purse or cursor coins) while
the player holds a grant asks for the inputs again without withdrawing: the
last inputs stay in force until the answer to the new revision lands; health,
stance and fatigue changes still apply at once. An answer the session refuses, or no answer within one
second, withdraws movement as any other input requirement does. Every other
input requirement withdraws at once.

After normal admission/revision validation and generation-rule evaluation,
`WorldEvent::SpeedInputsAccepted(SpeedInputAcceptance)` echoes the exact ready
inputs and report identity with `granted`. A new report is acknowledged even
when its numeric calibration is unchanged. A later change in the accepted
report's grant publishes a new outcome; unchanged ticks do not repeat it.
Unavailable or obsolete reports never produce a ready acceptance. `granted`
is false without a current positive calibration or while lifecycle blocks
movement. This is a local session receipt, not a remote server acknowledgement.

Keep this receipt separate from `MotionState`, which alone carries movement
speeds. `MotionState.jump_factor` is the share of the generation's
jump speed a jump starts with now (the health and weight factor, 1 where the
session does not work speeds out itself); a frontend launches its jumps at it,
and the session's guard bounds an announced jump's rise by the factor the jump
started with. `MotionState.strafe_walk_units_per_second`, where present, is
the strafe rate with walking selected: Titanium strafes on the selected base
speed, so `strafe_units_per_second` is then the running rate, which bounds a
`MovementMode::Strafe` sample either way; a frontend keeps a walking strafe to
the walking rate, and halves either while the player ducks.
`MotionState.swim_backward_units_per_second`, where present, is how fast the
player backs up in water, in the land's units per second before the water's
share (`movement::swim_speed`) applies: Titanium backs up in water, unless
ducking or sneaking, only by the buffs' speed modifier, so 0 without one. The
guard bounds the component of a `MovementMode::Swim` stroke behind the
heading it ends facing by it; a frontend keeps backing up in water to it. A
ducking or sneaking player's backing in water goes as a
`MovementMode::Backward` sample and takes the land's backward rule by its
mode.
Withdrawals, input requirements and admission/lifecycle resets invalidate
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

`eq_network_game::buffs::scale_instrument_effect` takes an already evaluated
signed magnitude and a received modifier. The result is a host projection
for movement-related effects. An unresolved modifier returns `None`. The
host's neutral handling of a received zero is a compatibility inference.
Unpatched Titanium scales a stored zero to zero at `eqgame.exe` 0x4102a6–0x4102cb.
The zero-modifier recording case compares the received action and buff slot
with the movement effect on the deployed client.

`MotionSession::send_move_sample` keeps its existing signature and result
semantics. Internally it prepares one checked candidate while exclusively
borrowing its owner. Validation and the generation release decision precede
any send. Held and unchanged candidates commit locally without sending;
immediate candidates commit movement only after the existing send callback
succeeds. A checked candidate is not yet an accepted move or a server receipt.
Dropping it or failing the callback leaves motion state untouched; external
callback effects cannot be rolled back. No public observer/transaction API is
introduced, and the private plan certifies no external body or scene identity.

### Absolute Titanium heading samples

`MovementRequest` has a typed `heading` field. Existing callers using struct
literals can set `HeadingUpdate::Rate` or use `MovementRequest::new` to keep the
existing elapsed-time turn check. `HeadingUpdate::Snapshot` carries a completed
mouse turn as an absolute heading with a zero continuing-turn hint. The zone
owner enables it for Titanium. It shares the existing packet cadence, admission,
position and calibration checks. If both ground axes are active after the turn
has completed, the snapshot may carry a grounded diagonal step. The same signed
longitudinal and lateral limits apply in the snapshot heading. A rate sample
keeps the prior accepted heading when it uses those limits.

## Carried combines and world containers

**Breaking:** `Capability::WorldContainers` is a new variant and
`Capability::ALL` grows from 43 to 44. `Tradeskills` offers carried manual
combines; `OpenContainer`, `CloseContainer` and `Combine(WORLD_CONTAINER)`
require `WorldContainers`. `tradeskills::combine_capability` supplies the
same distinction to frontends. EQEmu offers both; P99 offers carried combines
and the guarded ordinary forge (`WorldContainers` scoped to one-unit placements
into empty forge slots and whole withdrawals to an empty cursor), with no change
to the current TAKP/Quarm offers. Exhaustive capability
matches and station controls must handle the new variant.

`titanium_combine` retains its signature but writes the native full 32-bit
slot, replacing the old invented high-word sentinel, and rejects nonpack,
nonworld slots. No EQMac combine encoding is introduced. Additive
`tradeskills::{check_carried, CarriedRefusal}` exposes the source-backed local
metadata/identity prerequisites without changing any inventory.
`CombineUpdate::Answered` means a received local transaction reset, not a
successful craft or item-move acknowledgment. Components, returned tools and
products require separate authoritative inventory updates. See
[the source contract and limits](docs/p99-manual-combines.md).

## Optional list presence replies

`ClientConfig::answer_list_polls` starts false. P99 offers
`Capability::AnsweringListPolls`; the choice does not enable itself from that
capability. `ClientCommand::AnswerListPolls { session_id, enabled }` changes
future reply policy for the current character, retaining it across zone
crossings. Returning to character selection or reconnecting restores the
configured startup value. A stale admission cannot change the choice.
Disabling cancels an owed reply before its send, including while dead or
zoning. Death, transfer, disconnect and a fresh zone owner drop any owed
answer; re-enabling never answers an earlier poll.

Mode 1 reports population and never joins a list automatically. Mode 2 alone
schedules one presence reply when enabled and admitted. A newer list message
replaces or invalidates an older owed answer, including unreadable messages.
The reply is sent once after a uniformly selected whole-second delay in
3..=60 seconds, on the next loop turn. This is application policy, not native
timing or evidence of server acceptance. No periodic keepalive, fabricated
captcha chat, successful queue membership or delivery receipt is inferred.
Only diagnostic events are produced by this handler.

## Explicit session control options

`Capability::GameplayDepartures` gates every explicit departure from the
official client's behaviour. Only a server type whose
`ServerType::offers_gameplay_departures` is true offers it; `Capability::ALL`
grows from 46 to 47. `SetMovementOptions` and `SetBookOptions` carry that
capability, and a zone session that does not offer it refuses them with
"Gameplay departures are not offered on this server" through the command's
ordinary refusal event, before any owner sees them. A host greys the controls
out there like any other unoffered command. Every checked server type offers
the departures while the owner tests them (2026-11-06); a departure on a
public server is faster or more permissive than the official client, and the
accessor is the one place to withhold it per server type.

`movement::MovementOptions` starts with both toggles off and ordinary running
speed. The admission-scoped
`SetMovementOptions { session_id, request_id, options, created }` command keeps ordinary movement
unchanged when off. `walk_while_stunned` permits fresh movement input during a
current stun without clearing it or admitting pre-transition input; casting,
items and jumping retain their independent stun gates. `run_speed_multiplier`
scales forward running, including the forward component of grounded diagonals,
and the horizontal budget for a Fall sample. `RunSpeedMultiplier` holds 100
through 140 percent in one-percentage-point steps, with 100 as the default.
Calibration events retain the source-derived speeds. The host applies the
same multiplier when proposing running movement. `fly` permits ascent/descent
within the current mode's unscaled elapsed speed budget. `posture`
(`PostureOptions`) holds two departures, on both generations: with
`feign_standing` the Feign Death press sends the feign request alone,
without the lying posture report both programs send first, so the player
stays standing and moves as any standing player, and a feign spell's
landing (`FeignLanded`) sends nothing; with
`stay_seated_when_hit` an `OP_Damage` hit leaves a sitter or a looter
seated where the generation's program stands them (`GameDialect::lying()`).
These options never
bypass session identity, input freshness, finite coordinates, fixed bounds, current
known speed, corrections, death or transfer gates. Changes invalidate pre-toggle queued input using the caller stamp and preserve
the accepted sample time for same-frame FIFO movement. A change of flight or
stun permissions keeps the ordinary paced stop; a speed-only change leaves a
held moving sample on its send schedule and applies the new budget to later
samples. Flight toggles preserve a tracked jump, so switching back to
grounded movement cannot qualify a sample for diagonal credit during that
ascent. The tracker expires as accepted samples pass the jump apex. It does
not establish a landing. None represents a native movement
rule, safe server threshold or detection guarantee.

`SetBookOptions { session_id, memorize_without_sitting }` is an explicit
memorization departure: no automatic sitting and no local five-second
preparation. Scribing retains its existing posture and delay. Spell ownership,
current book, gem, inventory, action holds, stun and server-reply handling
remain. A changed option cancels pending preparation rather than converting
it in flight. A new admission starts with default book and movement choices,
so a frontend reapplies its saved options after admission.

The native Titanium memorization sender (`0x556b72`) owns a variable local
countdown before sending ordinary mode-1 `OP_MemorizeSpell`; skipping our
local timer is an application departure. The inspected public
[EQEmu request handler](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/client_process.cpp#L1145)
checks packet/spell/class/level/ownership and does not reject on posture or
preparation time. Its
[timing detector](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/cheat_manager.cpp#L318)
can record a rule-gated fast-memorization event; the packet handler then calls
the normal action handler. This does not establish deployed server behavior,
absence of monitoring or safe timing. Confirmation still requires server
book/gem state; public-server acceptance remains unverified.

`ExitAtOnce { session_id, created }` is an explicit immediate departure. It
sends the ordinary Logout request without Camp, automatic sitting or the
countdown, then uses the same reply/close owner as normal camping. It does
not bypass admission, life state, session identity or freshness checks and
never resends while already logging out. Neither submission nor the local
close timeout establishes deployed server acceptance. Public-server behavior
and server-side cleanup remain live-validation items.

Movement policy changes report `MovementOptionsApplied { session_id,
request_id, options }` after local installation or `MovementOptionsRefused {
session_id, request_id, reason }` on refusal. The host gates opt-in motion on
confirmed local worker policy and matches its own request identity; enqueue
alone establishes nothing. Identical reassertions can produce feedback but
never reset input epochs. These events are not server acknowledgments and
never imply a movement speed grant. Expired changes can be retried with a new
identity and caller stamp without accepting old motion or reasserting policy
on every render frame.


## Resurrection and translocate consent

A live session exposes `WorldEvent::Resurrection` and
`WorldEvent::TranslocateOffer` with `confirmations::ConsentOffer<T>` where
its server policy supports the operation. Read the decoded fields from
`offer`. Copy `id` into `AnswerResurrection::offer_id` or
`AnswerTranslocate::offer_id` and use the admission from the same token as
`session_id`. A fresh admission can reuse a sequence. The ID combines its
admission, question kind and sequence and has no wire representation.

An identical pending offer keeps its original token and deadline. A distinct
offer of the same kind replaces it. On P99, an exact packet already issued
as a question is withheld for the rest of that admission, including after a
replacement, answer, expiry or interrupted transfer. The packet has no
established discriminator for an identical new offer. Each kind retains
up to 1,024 identities and withholds new questions after that limit. A stale
or repeated answer produces a refusal naming the attempted token and leaves
a replacement untouched.
Death, an outstanding zone transfer and logout invalidate old consent.
The in-process `expires_at` is a monotonic local deadline omitted from
serialization. The Titanium EQEmu and P99 policies use a five-minute local answer
window and submits a decline when it elapses during the same live admission.
Titanium `eqgame.exe` sets the timer at 0x4561bd (translocate) and 0x459b61
(resurrection). Expiry at 0x547f0a passes dismissal result 2 at 0x547eff.
The translocate and resurrection callbacks send a decline at 0x499ef3
and 0x499f3a, respectively. TAKP translocate retains its one-hour
window (6.253). A server may apply its own timing rule.

`ConsentClosed` distinguishes a submitted answer, a local expiry with a
submitted decline, and a lifecycle interruption. Movement and recovery
follow the ordinary server messages and admission. EQEmu translocate uses
the server's pending destination and resurrection echoes the offered packet with
the answer selected. P99 uses the same offer-bound consent controls. An
exact named translocate completion can enter the regular zone transfer
path after a matching Yes during the original offer lifetime. Bind spell
consent follows explicit server-directed moves. A bind completion alone
does not establish the current bind coordinates.

Raw wire readers should use `message::titanium` or `message::eqmac`
for consent records. The corresponding `Message::Resurrection` or
`Message::TranslocateOffer` carries decoded facts without a session
token. `Message::TranslocateCompleted` carries a Titanium continuation for
session correlation. `world::titanium_update` leaves consent to the session.
TAKP's distinct server move still enters the existing zone-transfer path.

See EQEmu's [resurrection offer routing](https://github.com/EQEmu/EQEmu/blob/master/zone/worldserver.cpp#L865-L893),
[resurrection processing](https://github.com/EQEmu/EQEmu/blob/master/zone/client_process.cpp#L971-L1055)
and [translocate processing](https://github.com/EQEmu/EQEmu/blob/master/zone/client_packet.cpp#L14888-L14952).
