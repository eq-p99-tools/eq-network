# Scribe consumption and movement readiness

A plain scroll pickup preserves carried weight while the move remains predicted.
After a successful scribe, the server removes the scroll. Previously the current
inventory lost it but the pre-move alternative still contained it. Comparing
those unequal loads correctly withheld movement, but that obsolete alternative
could persist until a fresh admission.

Inventory now traces plain whole-item relocations separately from pending slot
origins. Authoritative consumption removes the traced item from both alternatives.
It resolves only the consumed origin and an originally empty destination; another
item predicted into the origin, intermediate slot history and independent moves
remain unresolved. A normal pickup followed by confirmed scribing therefore
recovers calculable load without a snapshot or a timer acknowledgement.

The weight calculator and host unavailable-state guards are unchanged. Unresolved
banking, bag reductions or other weight-changing moves still prevent a numerical
grant. Only exact two-slot relocations of non-stack items without contents are
traced. Swaps, split/merge operations, ambiguous bulk projections and conflicting
server replacements do not acquire guessed identity. Matching replacements
retain the trace; snapshots and invalidation discard it. This is intentionally
not a general inference engine for every inventory history.

## Source ordering

Quarm references pin `SecretsOTheP/EQMacEmu` at
`c2cd2651045ab063b236b08f3a9e83cbf10289e9`:

- [`OPMemorizeSpell`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_process.cpp#L1361)
  checks the cursor scroll's spell, calls `ScribeSpell`, then deletes the cursor
  item with client notification.
- [`ScribeSpell`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/spells.cpp#L4722)
  records the book entry and sends the corresponding book update.
- [`DeleteItemInInventory`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/inventory.cpp#L897)
  deletes the item, calls `SaveCursor`, then sends the slot-removal packet.
- [`SaveCursor`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/zonedb.cpp#L4443)
  can send the newly exposed queued cursor item before that removal packet.

The same scribe/delete ordering was checked in TAKP
[`OPMemorizeSpell`](https://github.com/EQMacEmu/Server/blob/047d0f8/zone/client_process.cpp#L1249)
and [`DeleteItemInInventory`](https://github.com/EQMacEmu/Server/blob/047d0f8/zone/inventory.cpp#L484).

The diagnostic “a unit used” describes the normalized `InventoryUpdate::Consume`.
For scribing, the wire packet is `OP_MoveItem` with the cursor as source and
all-ones destination/quantity. It is not `OP_DeleteCharge`. The existing book
receipt supplies the consumption meaning only after a matching confirmed scribe.
The receipt now tolerates a queued cursor addition between confirmation and
removal, preserving the original scroll until it is consumed and the queue advances.
Other intervening inventory changes still cancel the correlation. A removal
before the book result remains a correction; a late result cannot repair it.

## Regression coverage and limits

All fixtures are synthetic. Tests exercise actual Quarm decoding, feature
dispatch, book correlation, forwarded host inventory events and fresh movement
input revisions for two consecutive scribes. They also cover cursor refill on
either side of removal, duplicate book results and an additional removal while
the cursor is empty, out-of-order removal, independent carried/banked moves,
origin-slot reuse, intermediate moves, ambiguous identical-item projections,
stacks, swaps, matching replacements and conflicting corrections.

An additional slot removal after a new item occupies the cursor is a new
authoritative slot update, not assumed to be a duplicate consumption of the old
scroll. This layer does not add payload-based packet deduplication. Transport
ordering and sequence handling remain unchanged. Stack consumption and more
complex identity histories can remain uncertain until covering server updates.

The companion client change pins this library and checks the existing host
calculation/report path; it does not grant movement from a timeout or suppress
unavailability. No live server, GUI or emulator was used for this fix.

## Dedicated retest

On the combined client artifact, admit with authoritative inventory and known
load, then pick up and scribe an inexpensive usable scroll. Confirm the new book
entry, consumed cursor item, current load and ordinary movement readiness without
re-entry. Repeat with a second scroll. If a controlled fixture can expose a queued
cursor item, verify it survives consumption and contributes its own weight.
Stop on any correction, disconnect or unknown inventory state; do not retry an
uncertain action. Preserve evidence privately and use clean camp for normal
cleanup. Banking/refusal experiments remain outside this public retest.
