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

The generation-selected EQMac object owner permits one unresolved pickup, since a refusal cannot name
its object. It holds inventory actions while the incoming cursor item is
unknown. An own refusal clears the request. An own removal waits for the separate
cursor delivery, in either arrival order. This is conservative request
correlation, not a claim about the official client's click rate or an item-move
acknowledgement. Silence retains the hold until an answer or a new zone session;
no timer releases it or retries an uncertain pickup. A fresh object placement,
removal by another actor or another actor's refusal cannot answer the pending
request: a later refusal has no object ID and must not clear a newer click.
An inventory snapshot in the same session is not a request-stream barrier:
it cannot prove that an old untagged refusal will never arrive. Only a new
session/admission unconditionally resets this attribution uncertainty. The
known inventory is not invalidated merely because pickup attribution is unknown.
The established Titanium result behavior is preserved.

## Availability and remaining work

The reviewed inventory baseline provides the single inventory owner for both
TAKP and Quarm. Each server policy now offers `GroundItems` from its independently
inspected source and selects EQMac's result handling. Pickups use the inventory
owner's current cursor projection; an unrelated unresolved move does not create
a hold or an acknowledgment. Authoritative cursor deliveries do not settle moves. Door definitions/actions are staged and replayed at admission,
but `Doors` interaction capability and click ownership remain unavailable:
the program read establishes the meaningful ordinary fields, but its unwritten
padding and additional type/state guards still require integration review.
Ignoring fields in the server is not evidence for invented official-client bytes.

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
These are production paths with recording sinks; they open no game connection.

Future controlled checks belong to an attended session with an identified
build and explicit authorization:

1. Read and compare initial doors, actions, inverted endpoints, removal and
   re-admission; retain unknown door types without claiming universal animation.
2. Establish the official eight-byte ordinary door-click fields before enabling
   interaction. Check in-range and out-of-range refusal, locked doors, and
   repeated static objects after the sender is evidenced.
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
