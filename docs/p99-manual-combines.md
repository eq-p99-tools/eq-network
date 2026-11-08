# P99 carried manual combines

P99 offers manual combines in carried tradeskill containers and an ordinary
forge path. The forge path uses the received object and requires the server
to open that placement for the player as an ordinary forge. Live P99
verification remains pending.

## Native request and reset

The inspected Titanium manual-combine sender at `0x4fc106` writes a full
32-bit inventory slot and sends four bytes with application opcode `0x0b40`.
Its send wrapper (`0x4912b1`) and serializer (`0x6007f0`) copy that word;
ordinary slot 23 is `17 00 00 00`, not `17 00 ff ff`. World-slot 1000 is
also names the server-opened world container.
The codec now accepts only carried pack slots or the world-container slot.
No EQMac sender or capability is added.

The native receive dispatch at `0x45bf96` recognizes this opcode and branches
to `0x45e922`, which only decrements/clamps the local transaction counter.
It does not provide a success verdict, consume a component or grant a product.
The existing decoder recognizes the empty reset used by the inspected server
reference; a nonempty body remains unavailable instead of acquiring guessed
fields or meaning. P99's deployed response shape/order still needs controlled
verification. No captured bytes or installed assets are regression fixtures.

The independently inspected public reference,
[`Object::HandleCombine`](https://github.com/EQEmu/EQEmu/blob/master/zone/tradeskills.cpp),
emits an empty reset on refusal paths and before its ordinary consumption and
result handling. It logs the high half of the four-byte structure as a tribute
slot but does not use it to determine a normal manual combine. This supports
stock-emulator compatibility of the native zero upper word; it does not prove
the revision or implementation deployed by P99.

## Prerequisites and inventory authority

The carried owner rechecks current loaded inventory, known player, pack slot,
container capacity/type, empty cursor and cursor coins. The native research
class and cultural race restrictions use decoded identity and container metadata,
not names or a recipe whitelist. Stacked components are refused separately
from a nonstackable tool's charges. An empty container is refused. The server
still decides recipe eligibility, restrictions, consumption, skill gains and
the final verdict.

A request older than an intervening inventory/coin update is refused rather
than applied to a replacement. Another combine cannot replace a pending one.
The owner holds item actions until a received reset; no silence timer, guessed
completion or automatic retry clears that hold. A new admission has a new
owner. This is a local operation gate, not positive item-move acknowledgment.

Only the existing inventory owner applies authoritative consumption, returned
components or cursor products. Sending, waiting, a reset and ordinary chat
create no inventory changes. A reset can accompany a refused recipe with no
inventory update. If no reset arrives, the outcome remains unknown; it is not
reported as success, and leaving/re-entering creates a fresh admission.

## Availability and frontend

`Tradeskills` covers carried manual combines. `WorldContainers` independently
covers opening, closing and world-slot combines. EQEmu offers its typed
stations. P99 offers the ordinary forge. A received Forge model with type
zero is eligible for the same explicit click as a typed forge. Only a matching
server reply with forge kind 17 makes the container usable. The command's
required capability, skin control, nearest-use selector and station script
lookup use that distinction.

Opening a P99 forge needs a current empty cursor. Place one whole item
from the cursor into each empty station slot. Withdrawing an item needs a
current empty cursor. Stack transfers and swaps are outside this P99
workflow. EQEmu keeps its existing station transfers.

Moves into the station are provisional. The station's separate item
updates and whole or single-place clears establish its contents. The
close request names the same open instance. Inspect subsequent inventory
updates for returns. If the P99 station stream cannot be attributed after
close, re-enter the zone before using another forge. EQEmu's ordered
open reply can fence an earlier station's stream after a close and clear.
Project 1999 has documented returning remaining items on close.
[Project 1999 patch notes](https://www.project1999.com/forums/showthread.php?mode=threaded&p=21513)

Combine waits for queued local item-move results before operating on the
worker's container state. Buttons grey while those moves or a combine wait;
script Combine clicks use the same queue prerequisite. Neither clearing a
local move queue nor a combine reset confirms server item placement.

Regression source covers the native word, metadata/identity/quantity guards,
real P99 policy and forge reply matching, no retry after silence, reset versus
separate inventory updates, fresh admissions and frontend queue/offer gates.
Controlled P99 checks remain: a cheap carried success and refusal, empty/full
hands, authoritative consumption/return/product, response ordering and safe
re-entry after an unanswered operation. For a forge, a bounded Metal Bits
combine uses two individual Small Pieces of Ore and a Water Flask in three
slots. Verify the actual placement, reset, consumption, cursor product and
close or return events separately. [Metal Bits recipe](https://wiki.project1999.com/Metal_Bits)
