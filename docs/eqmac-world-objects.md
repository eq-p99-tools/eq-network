# EQMac doors and ground objects

The EQMac wire reads doors and ground objects independently of Titanium.
This is public-source inspection and synthetic validation. It does not establish
the revision deployed by TAKP or Quarm, or live compatibility. No installed
assets, captures or identities are test fixtures.

## Evidence

The layouts and handlers were inspected independently in TAKP
[`EQMacEmu/Server` at e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f](https://github.com/EQMacEmu/Server/tree/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f)
and Quarm
[`SecretsOTheP/EQMacEmu` at c2cd2651045ab063b236b08f3a9e83cbf10289e9](https://github.com/SecretsOTheP/EQMacEmu/tree/c2cd2651045ab063b236b08f3a9e83cbf10289e9).

- Packed layouts: TAKP's
  [`common/eq_packet_structs.h`](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/common/eq_packet_structs.h#L1910)
  and Quarm's
  [corresponding structures](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/eq_packet_structs.h#L1746).
  Both use packing of one byte. `Object_Struct` ends at byte **222**:
  the final field starts at 218 and contains two 16-bit values. Its old
  end-offset comment saying 224 is inconsistent with the packed fields.
- Door tables: TAKP's
  [`EntityList::SendZoneDoorsBulk`](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/zone/entity.cpp#L993)
  and Quarm's
  [same producer](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/entity.cpp#L986)
  prepend a count to zlib-compressed records. They apply inversion to the
  initial state before sending it. `DespawnAllDoors` sends an empty removal.
- Object state and pickup outcomes: TAKP's
  [`Object::CreateSpawnPacket`, `CreateDeSpawnPacket`, `Process`, `RandomSpawn`
  and `HandleClick`](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/zone/object.cpp)
  and Quarm's
  [independently inspected implementations](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/object.cpp)
  distinguish disappearance from refusal and deliver inventory separately.
- Requests and server eligibility: TAKP's
  [`Client::Handle_OP_ClickObject`](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/zone/client_packet.cpp#L2705)
  and Quarm's
  [own handler](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_packet.cpp#L3232).
  Quarm additionally checks server-side challenge, zone and player-drop rules.
  Those rules are not deduced from an object's model or duplicated by the client.

TAKP's
[`patch_Mac.conf`](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/utils/patches/patch_Mac.conf)
uses swapped opcode notation relative to Quarm's
[table](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/utils/patches/patch_Mac.conf).
After normalization they identify the same application opcodes. Neither Mac
patch translates these structures to Titanium layouts.

An independent offline EQMac program read found the ordinary door sender at
`0x521d91` and mouse caller at `0x547034`. The send at `0x521f00..0x521f2c`
writes ID at 0, a caller parameter at 2, held item at 4 and player ID at 6;
the ordinary no-held-item path supplies parameter 0 and held item `u16::MAX`.
Byte 1 is not written on this stack path. Additional type/state guards precede
the send, including exclusions for types 54/55/56. This read establishes the
meaningful fields, not a value for unwritten padding or all door behaviors.
The ordinary serializer preserves those arguments and the player's ID, and
chooses zero for unwritten byte 1. That deterministic padding is not a captured
official-client value. Both inspected `Handle_OP_ClickDoor` handlers require
eight bytes, look up the door, and leave keys, locks, movability and scripts
under server control. A locked refusal can arrive as an ordinary server message;
some nonmovable or scripted doors can ignore the request. Silence proves nothing.

Further program reads at `0x520f91`, `0x52169e` and `0x4a3237` establish the
ordinary definition/state mapping, local return delay and hinge motion. The
native clock supplies integer milliseconds; motion advances by
`8 * (elapsed_ms * 0.02)` heading units toward a quarter turn of 128. Types 5--7
reverse the swing direction. Types 0/5 return after ten seconds at the
non-resting endpoint; the other supported ordinary types use twenty seconds.
Inversion selects the resting endpoint. These local clocks and poses do not
acknowledge a request or replace a received action.

Ground click sends at `0x53189e..0x5318f6` and `0x53257a..0x5325d4` write the
drop ID and player's 16-bit ID; the serializer zeroes the upper player word
instead of copying uninitialized padding. No captured padding value is claimed.
The opposite-direction object-drop routine `0x530d7e` sends 224 bytes. This
does not change the packed 222-byte server placement: the inspected servers'
`Handle_OP_CreateObject` ignores that incoming drop body and drops the cursor
item. No drop sender is changed here, and the placement decoder rejects 224.

## Wire and state

All body integers and floats below are little endian. Offsets are zero based.

| Message | Opcode | Body |
| --- | --- | --- |
| Door definitions | `f741` | `u16 count`, then zlib of `count * 44` bytes, at most 255 records |
| Door action | `8e40` | door ID `u8`, raw action `u8` |
| Ordinary door click | `8d40` | door ID `u8`, zero padding `u8`, ordinary argument `u16` = 0, no-item `u16` = `ffff`, player ID `u16` |
| Remove all doors | `9b40` | empty |
| Ground placement | `2c40` | one packed 222-byte record |
| Pickup / object disappearance / refusal | `2b40` | drop ID `u32`, player ID `u32` |

A door record carries its 16-byte model at 0, y/x/z/heading floats at
16/20/24/28, incline/size `u16` at 32/34, ID/open type/state/inversion bytes
at 38/39/40/41 and parameter `u16` at 42. The shared table retains raw unknown
types, states, actions and parameters. It does not apply inversion a second
time. Movement for an unknown ID creates no door. Removal clears definitions
and actions; the next placement starts afresh. Repeated static IDs remain
separate visual definitions and cannot be clicked.

TAKP and Quarm offer ordinary door use only for types 0--8 with zero incline,
positive signed-16-bit size, parameter zero, canonical initial state/inversion,
and a known action (none, 2 or 3). The request requires a current placement,
known player, loaded/current empty cursor, a settled hinge, and the conservative
two-dimensional door reach of at most 20. The program's additional size-based
reach is not implemented. Specialized key/skill arguments, lifts, teleporters,
traps, nonzero parameters and unknown states remain decoded and visible without
ordinary use. Hidden server lock or movability rules cannot be learned from a
door definition, so an offered ordinary door can still refuse a request.

Sending an ordinary click neither changes its pose nor schedules a return.
Received actions set the motion target, and local motion suppresses another
click until settled. The local return animation sends no packet, creates no
server observation, and causes no automatic retry.

A ground placement carries drop ID at 12, heading/z/x/y floats at
140/144/148/152, a 16-byte model at 156 and raw object type `u32` at 212.
It contains no authoritative inventory item definition. Its other item-like
fields are not promoted into inventory or used as proof of obtainability.
The existing conservative item-model/fixture classifier remains a local
eligibility guard; the server can still refuse any candidate.

Placements replace the current object with that ID. A disappearance removes
it, including when the actor is zero (decay/removal). EQMac's `u32::MAX` drop
ID is a refusal, normalized to `ObjectUpdate::Kept`; it removes nothing.
Titanium's zero-drop-ID refusal remains specific to Titanium. Neither reply
names an inventory item, and a refusal does not identify which object was
clicked. No synthetic request ID or successful pickup completion is invented.

The object owner rejects a pickup made before the current placement, including
an ID reused after disappearance. It also requires the server table, the
existing loaded/current inventory and empty-cursor guards, a known player and
the generation's reach. The existing EQMac program-read reach is strictly
under 20 in three dimensions; its door reach is at most 20 in two dimensions,
without the still-unread size extensions (`reach.rs`, checklist 6.311).
A map marker is never a pickup request or inventory grant.

Only the existing inventory feature applies item arrivals. Sending a request,
observing disappearance or receiving another player's click does not create
an item. Item arrivals use the existing EQMac item decoder and inventory owner,
including `OP_SummonedItem` for the cursor. No prediction ledger is added here.

The generation-selected EQMac object owner permits one pending pickup and
holds only further pickups, not inventory moves or other inventory actions.
While result attribution is unique, an own refusal ends the wait and an own
removal plus separate authoritative cursor delivery ends it in either order.
Neither completion rule creates an item or acknowledges an inventory move.

Removal by another actor (or with no actor), an authoritative inventory snapshot,
or three seconds of local waiting cancels the pending wait. Cancellation is
neither success nor a claim that the server abandoned the request. The timer
bounds local duplicate suppression; it is not an official-client cadence or a
server response guarantee, and it sends no automatic retry.

A cancelled request may still produce an old untagged refusal or cursor packet.
One admission-scoped ambiguity bit retains that fact without accumulating
cancelled request records. Those packets, including old named removals, cannot
end a newer wait after a placement reuses the old ID. Later waits remain bounded
and may also be cancelled by an authoritative snapshot; ambiguous removals do
not cancel them. Only a new session/admission restores unique
result attribution. Authoritative item packets still go through the one
inventory owner; pickup cancellation never invalidates otherwise known items.
Consequently, after attribution is lost, each later pickup may retain its local
hold for the full three seconds. Elapsed time and cursor arrival cannot prove
that an older untagged result belongs to that later request. Each deadline uses
the action owner's successful submission time, rather than the age of a queued
UI intent; expiry remains local cancellation and sends no retry.
The established Titanium result behavior is preserved.

## Availability and remaining work

The reviewed inventory baseline provides the single inventory owner for both
TAKP and Quarm. Each server policy now offers `GroundItems` from its independently
inspected source and selects EQMac's result handling. Pickups use the inventory
owner's current cursor projection; an unrelated unresolved move does not create
a hold or an acknowledgment. Authoritative cursor deliveries do not settle moves.
Door definitions/actions are staged and replayed at admission. `Doors` and its
click owner are offered for the guarded ordinary subset described above;
neither availability nor local submission establishes server acceptance.

World containers, their item packets, automatic teleport-pad detection, and
corpse/loot operations are outside this change. No new line-of-sight, collision
or model-specific pickup permission is inferred from a placement. No new
inference label is allocated; the existing reach limitation remains unchanged.

## Offline coverage and controlled live checklist

Synthetic tests exercise real message/request dispatch, bounded decompression,
count mismatches, malformed tables and placements, generation isolation,
unknown fields, state replay/removal, stale requests, fixture/unknown/distant
refusals, the strict EQMac reach boundary, and denial versus disappearance
versus an authoritative cursor item through the single inventory owner, and
duplicate suppression without a timeout retry.
Ordinary-door regressions cover the eight-byte sender, admission offer, empty
cursor and stale/type/state guards, received motion, type-specific local return,
and a locked server message without a door action or predicted opening.
These are production paths with recording sinks; they open no game connection.

Future controlled checks belong to an attended session with an identified
build and explicit authorization:

1. Read and compare initial doors, actions, inverted endpoints, removal and
   re-admission; retain unknown door types without claiming universal animation.
2. Compare the source-backed ordinary eight-byte sender and local hinge timing
   with authorized controlled observations. Check reach, locked refusal, hidden
   nonmovability, initial/inverted poses and repeated static objects; do not
   interpret silence or transport submission as an opening.
3. Confirm 222-byte ground placements on each server independently, their
   disappearance/respawn and current-ID guard. Compare server-reported placement
   with visibility without treating a marker as proof that pickup is possible.
4. Pick up one permitted ordinary item with an
   empty cursor. Confirm separate authoritative item arrival, actual charges
   and inventory state; confirm another actor's removal grants no local item.
5. Check a permitted refusal scenario, such as a lore conflict, while observing
   server messages and the `u32::MAX` result. The object and inventory must
   remain intact. Never generate prohibited Quarm actions to probe a restriction.
6. Check a replacement ID, reach boundary and cursor occupancy without repeated
   speculative requests. Bags, queued cursor arrivals and multi-move interaction
   need the inventory owner's coverage before broader enablement.

Keep captures, screenshots, locations and identities private. Report source
compatibility, synthetic results and attended outcomes separately.
