# Ordinary beneficial-buff cancellation

`ClientCommand::CancelBuff` carries the current admission, received buff-slot
revision, exact slot and spell, creation time, and the host spell provider's
explicit beneficial and cancellable classifications. Unknown classification is
not permission. The host must preserve the ordinary spell-effect 101 exclusion.
The network worker has no installed/effective spell catalog and does not infer
classification from a buff record's effect type or an icon. The server retains
its own eligibility check.

The session revalidates received membership, rejects stale and duplicate-spell
selections, and sends only for its own player. An observed landing without a slot
invalidates an older assignment for that spell; it never supplies a guessed slot.
Only Titanium EQEmu offers `CancellingBuffs`. P99 remains an unresolved evidence
gap: the variant below is not offered by its policy pending source/capture
follow-up. Quarm and TAKP are deferred.

The initialized request uses Titanium `OP_Buff` (`0x6a53`), 32 bytes: own entity
at 0, the received 20-byte buff record at 4, slot at 24 and zero at 28. The unused
record byte is zero. This is the public
[Titanium packet layout](https://github.com/EQEmu/Server/blob/master/common/patches/titanium_structs.h)
and [decode contract](https://github.com/EQEmu/Server/blob/master/common/patches/titanium.cpp).
The [EQEmu handler](https://github.com/EQEmu/Server/blob/master/zone/client_packet.cpp)
selects by spell ID, echoes sentinel/detrimental requests, and otherwise calls
`BuffFadeBySpellID`. Therefore the client refuses duplicate slots containing the
same spell instead of promising that the server cancels only one slot.

Submission does not remove received membership or constitute an acknowledgement.
A received fade matching the slot and spell during the five-second local wait
is reported as `FadeObserved`. It can be natural expiry. Replacement is
`Changed`; no matching change before the wait ends is `Unresolved`. None of
these results retries the request. A rejected echoed current record remains
present. Death, transfer and disconnect discard the worker's selection;
reports retain their admission for host filtering.

The open EQEmu fade sender names the spell and slot. P99's use of sentinel
spell IDs for slot clears is unverified. The worker keeps the received spell
field and requires an exact match before removing a slotted buff; a sentinel
alone cannot identify the buff or remove a replacement.

This implementation sends one initialized current-record request, which the
EQEmu handler supports, and awaits received membership. A client-side sender
layout alone leaves both request sequencing and the server's fade contract
unresolved. P99 remains gated pending that evidence and an attended check.

Regression fixtures are constructed. Domain checks, worker submission/fade/echo/
replacement/timeout/reset/send-failure cases and selected-policy lifecycle gates
do not substitute for local or P99 observations.
