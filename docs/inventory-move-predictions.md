# Inventory move predictions

Offline program reads identify EQMac's move routine at `0x422b1c` and
Titanium's at `0x51a9a8`: a valid move updates the local projection and sends
immediately, without a pending-move gate or pacing timer. Whole moves retain
the existing 12-byte `{from, to, 0}` packet and generation-specific slot mapping.
The implementation preserves existing slot ranges, bag identity and fit,
equipment eligibility, stack quantities and restricted-item checks.

This is program-read evidence for sending behavior, not evidence that silence
confirms acceptance. EQMac has no generic positive move ACK. The existing
Titanium refusal-window settlement remains unchanged; this change does not
establish it as a native-client acknowledgment mechanism.

The inventory remains owned by one session feature. Every successful send
immediately records its projection and changed slots in submission order.
Another valid move uses that projection, with the current revision. Transport
failure never commits a prediction; stale admissions, revisions and intents
remain refused. Replaying an already-applied intent cannot duplicate a send.

Server evidence resolves only what it reports. A matching replacement or
removal resolves its covered slots across the affected records. This may
retire several projections or a later projection first; neither case assigns
an ACK to an individual move. Duplicate replacements are idempotent. A reply
that disagrees with the current EQMac projection, including a late intermediate
state, is treated as a correction. Updates remain visible, but further actions
require a fresh complete snapshot; more replies or elapsed time cannot clear
that invalidation. A snapshot resets all prediction history and cursor queues.

An authoritative `Cursor` item delivery fills an empty cursor or queues behind
its current item. It does not acknowledge prior moves. A predicted empty
cursor is available for the next action, without proving the hidden server
queue is empty. Consumption retains the existing conservative identity traces:
an unambiguous whole non-stack item can be followed through several simple
relocations and removed at its origin when consumed. Independent uncertainty,
stack/quantity ambiguity, swaps and bag identity are not confirmed by that
receipt. Existing scribe receipt and duplicate-removal handling remain in use.

The same-stack no-send guard belongs to the whole-item swap path
(`plan_cursor_swap`); an explicit counted merge still uses `plan_merge`.
It must not become a blanket prohibition on moving onto matching stacks.
In EQMac, the merge helper call at `0x422e73` precedes the same-stack return
at `0x422ec5`; helper `0x4f0acd` sends the transferred count. Titanium has
the same order: merge call `0x51b0b6`, same-stack return `0x51b11b`, and
counted-send helper `0x464d65`. Both ordinary swap paths exchange the local
item pointers and immediately submit the 12-byte zero-count move.

Synthetic tests exercise the real zone feature assembly, refusal gate and
wire for rapid TAKP, Quarm and Titanium moves, including a return after 1.3
seconds, exact packet order, duplicate intents, partial slot evidence,
out-of-order correction, same-stack swap versus merge, consumption and scribe
recovery. Live rapid-move compatibility remains unverified. Review requires
controlled TAKP and EQEmu sequences with server refusals observed, plus a
Quarm sequence under the reviewer's attended-testing rules.
