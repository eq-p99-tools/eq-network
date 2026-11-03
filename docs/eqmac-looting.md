# Quarm corpse access

This implementation is source-derived and tested with synthetic packets. It
has not been exercised on Quarm or an attended local server. No installed
assets, identities or captured packets are fixtures.

## Evidence

Quarm source at
[`c2cd2651045ab063b236b08f3a9e83cbf10289e9`](https://github.com/SecretsOTheP/EQMacEmu/tree/c2cd2651045ab063b236b08f3a9e83cbf10289e9)
establishes these layouts and server actions:

- `utils/patches/patch_Mac.conf`: open `4e40`, end `4f40`, money `5040`,
  item view `5240`, item request/echo `a040`, completion `4541`, using the
  transport's existing opcode convention.
- `common/eq_packet_structs.h`: a 20-byte money record and a 12-byte item
  request, with 16-bit corpse, looter and place, two reserved bytes and a
  32-bit automatic-placement flag.
- `zone/client_packet.cpp`, `Handle_OP_LootRequest` and
  `Handle_OP_EndLootRequest`: a 16-bit corpse ID in each request. The server
  decides reach and corpse availability.
- `zone/corpse.cpp`, `MakeLootRequestPackets`: ordinary own-player and NPC
  corpses use the same grant, coin and item-list path. The server transfers
  coins before sending the success response. With group auto-split it sends
  zero coins in that response and separate purse additions. Listed item
  places are zero-based; the echoed open request ends the list.
- `common/patches/mac.cpp`, `ENCODE(OP_ItemPacket)` and `MacItem`: loot
  views are fixed 360-byte item records at their listed place. They are
  separate from carried-item and cursor packets.
- `zone/corpse.cpp`, `LootCorpseItem`: the item echo is queued **before**
  cooldown, cursor, range, permission and item checks. It proves receipt,
  never successful delivery. Item failures close with `OP_LootComplete`;
  ordinary inventory updates report actual delivery, including bag contents.
- `common/ruletypes.h`, `CheckCursorEmptyWhenLooting`: true by default for
  player and NPC corpses. Quarm's policy requires a loaded, current inventory
  projection and an empty item cursor before an item request. A pending move
  does not add a separate hold when that projection already has an empty cursor.
- `zone/corpse.cpp`, `SendLootReqErrorPacket`, `SendEndLootErrorPacket`
  and `EndLoot`: open refusals use money responses 0 or 2; item failure and
  explicit close use empty completion packets. Response 0 can mean more
  than another looter; callers must not infer a specific permission cause
  from it. Server chat may supply further explanation.

These sources do not establish the deployed public-server revision or the
official program's exact UI timing. No new inference checklist ID is used.

## Ownership and lifecycle

The generation encodes/decodes loot. `ServerType` offers it for Quarm;
TAKP availability stays unchanged pending its own validation. Item-rule
selection follows the same server policy as ordinary inventory decoding.
Titanium packet formats retain their existing slot conversion.

The loot controller opens only a visible corpse, correlates a server grant
and completed listing, and accepts requests only for listed places on that
corpse and for the admitted player. It never predicts inventory changes,
chooses automatic destinations or writes the purse. The existing inventory
owner consumes successful coin grants and ordinary carried/cursor updates.
Duplicate open-success messages cannot add coins twice. An item request is
not retried automatically; its place cannot be requested twice in the same
listing. Different listed places have no added pacing timer.

`LootUpdate::Acknowledged` is informational. Consumers must handle this new
enum variant separately from `Taken`: it does not prove an item was received.
A close request stops new item requests but retains the session until the
server closes it. A late inventory delivery still belongs to the inventory
owner after close. Death, corpse removal, zone departure and disconnection
discard the loot controller's access state; a new admission rejects old
commands. No timer fabricates completion or retries a transfer.

The wire does not attach a request identifier to money or completion
messages. Correlation is limited to one corpse session on the ordered zone
connection. A reconnect creates a new admission; packets cannot establish
access to the previous admission's corpse.

## Limits and reviewer checklist

The server remains authoritative for ownership, NPC rights, group/raid
rights, legacy-item locks, self-found restrictions, lore conflicts, capacity
and range. No local imitation grants those permissions. Player-kill loot,
GM peeking, legacy contention, group auto-split and special expansion rules
are not certified. Corpse drag/summon/consent and client UI integration are
separate workstreams. Bag contents are delivered by existing inventory
packets; the corpse listing does not enumerate them as independent loot.

Controlled, attended checks before claiming live support:

1. On an ordinary NPC corpse, open, inspect list ordering, loot to cursor,
   move that item through the inventory owner, then use automatic placement.
   Compare request bytes and server inventory with the official client.
2. On an own player corpse, recover coins, equipment and a populated bag;
   confirm each item and its contents appear once. Close and reopen a corpse
   with more items than the visible list can show.
3. Refuse an unavailable/locked/ineligible corpse and a corpse another player
   is looting. Confirm failure never enables an item request or credits coins.
4. Exercise a lore/cursor/capacity refusal under controlled conditions;
   confirm receipt echo precedes failure without claiming delivery, and the
   server completion closes access. Do not retry automatically.
5. Close while opening and while an item is pending, then allow queued server
   replies to arrive. Confirm late delivered items remain in inventory, and
   closed-list items cannot send another request.
6. End a loot session by death, zoning and reconnect. Confirm old commands and
   old list places cannot act in the new admission. Keep all identities,
   packet captures and screenshots private.
