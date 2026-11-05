# Explicit cursor-item destruction

The Titanium action uses `OP_MoveItem` (`0x420f`), with three little-endian
32-bit words: cursor slot 30, destination `0xffffffff`, and count zero. This
destroys the whole instance or stack. The dedicated `DestroyCursor` request owns
those constants; ordinary item moves still cannot encode negative slots.
`OP_DeleteItem` is a separate unit-consumption operation and is not used.

The public protocol reference is EQEmu's
[Titanium layout](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/common/patches/titanium_structs.h)
and [inventory handler](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/inventory.cpp).
P99 selects the Titanium/EQEmu policy; P99 server acceptance remains a separate
validation question.

`DestroyItem` binds the intent to admission, inventory revision and complete
cursor metadata, including quantity and charges. The sole inventory owner
rechecks it and submits once. A failed transport send commits no prediction and
is never retried. This first slice supports ordinary items, whole stacks and
empty containers. It refuses populated containers, cursor queues, unavailable
state and unresolved inventory predictions. Populated-container deletion needs
a separate reviewed confirmation of the whole subtree before it is offered.

A submitted destruction is a local `DestroyCursor` prediction. Received slot
updates or a full snapshot can reconcile it; the ordinary movement silence
heuristic cannot confirm it. The original item's mass remains in a conservative
carried-weight bound until then. A late unversioned cursor removal after a new
cursor arrival marks inventory stale and preserves the new item for recovery.
There is no automatic retry, and a second destruction requires received state.

Currency destruction needs its own native wire, denomination/amount, purse and
cursor custody, confirmation, and refusal/recovery contract. Ground dropping
needs the world-object owner, exact item/contents/position intent, server object
identity and resulting inventory/object reconciliation. Neither is implemented
by this action. EQMac destruction is unavailable.
