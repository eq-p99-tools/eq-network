# P99 carried manual combines

P99 offers manual combines in carried tradeskill containers. World-station
discovery, opening, closing and combining remain a separate, unoffered path.
Fishing and specialized door interactions are unchanged. This is source-backed
implementation with synthetic regression fixtures, not live P99 verification.

## Native request and reset

The inspected Titanium manual-combine sender at `0x4fc106` writes a full
32-bit inventory slot and sends four bytes with application opcode `0x0b40`.
Its send wrapper (`0x4912b1`) and serializer (`0x6007f0`) copy that word;
ordinary slot 23 is `17 00 00 00`, not `17 00 ff ff`. World-slot 1000 is
also representable by the native sender, but is not offered on P99 here.
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
covers opening, closing and world-slot combines. EQEmu offers both; P99 offers
only carried combines; current TAKP/Quarm offers remain unchanged. The
command's required capability, skin control, nearest-use selector and station
script lookup use that distinction. No station offer follows from a global
carried-feature check.

Combine waits for queued local item-move results before operating on the
worker's container state. Buttons grey while those moves or a combine wait;
script Combine clicks use the same queue prerequisite. Neither clearing a
local move queue nor a combine reset confirms server item placement.

Regression source covers the native word, metadata/identity/quantity guards,
real P99 policy and station rejection, no retry after silence, reset versus
separate inventory updates, fresh admissions and frontend queue/offer gates.
Controlled P99 checks remain: a cheap carried success and refusal, empty/full
hands, authoritative consumption/return/product, response ordering and safe
re-entry after an unanswered operation. World stations remain an explicit
parity gap for a separately evidenced follow-up.
