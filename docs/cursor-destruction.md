# Explicit cursor-item destruction

The Titanium action uses `OP_MoveItem` (`0x420f`), with three little-endian
32-bit words: cursor slot 30, destination `0xffffffff`, and count zero. This
destroys the whole instance or stack. The dedicated `DestroyCursor` request owns
those constants; ordinary item moves still cannot encode negative slots.
`OP_DeleteItem` is a separate unit-consumption operation and is not used.

The public protocol reference is EQEmu's
[Titanium layout](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/common/patches/titanium_structs.h)
and [inventory handler](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/inventory.cpp).
The action is offered on EQEmu and P99 (both Titanium policies). P99 server
acceptance remains unverified.

`DestroyItem` binds the intent to admission, inventory revision and complete
cursor metadata, including quantity and charges. The sole inventory owner
rechecks it and submits once. A failed transport send commits no prediction and
is never retried. This first slice supports ordinary items, whole stacks and
empty containers. It refuses populated containers, cursor queues, unavailable
state and unresolved inventory predictions. Populated-container deletion needs
a separate reviewed confirmation of the whole subtree before it is offered.
The [Titanium slot layout](https://github.com/EQEmu/EQEmu/blob/fa08ea4445c0494f8017cbfd320406cf2394caf3/common/patches/titanium_limits.h#L208-L217)
allocates ten cursor bag child addresses (331 through 340). A larger reported
capacity cannot be checked completely and is refused.

A submitted destruction is a local `DestroyCursor` prediction. Received slot
updates or a full snapshot can reconcile it; the ordinary movement silence
heuristic cannot confirm it. The original item's mass remains in a conservative
carried-weight bound until then. A late unversioned cursor removal after a new
cursor arrival marks inventory stale and preserves the new item for recovery.
There is no automatic retry, and a second destruction requires received state.

Currency destruction needs its own native wire, denomination/amount, purse and
cursor custody, confirmation, and refusal/recovery contract. The separate
[ordinary ground drop](../API-CONTRACT.md#ordinary-cursor-drops) uses the
received cursor and the existing world-object owner. EQMac destruction is
unavailable.
