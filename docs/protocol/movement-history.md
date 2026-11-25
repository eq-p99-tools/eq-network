# Titanium movement history

Status: sent on Titanium sessions, except after the server's validation
exchange (row 6.653; the P99 gate is 6.369). `EQMac`, TAKP, Quarm and RoF2
send none: their programs have no equivalent read. Labels below: [R] read in
the installed `eqgame.exe`, [C] seen in a recording of the official client
on a local server, [I] inferred.

## Opcode and record layout

EQEmu's [Titanium opcode table](https://github.com/EQEmu/Server/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/utils/patches/patch_Titanium.conf)
maps `OP_FloatListThing` to **0x6a1b**. Titanium's `0x6177` is
`OP_TrackUnknown`; the [SoF table](https://github.com/EQEmu/Server/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/utils/patches/patch_SoF.conf)
uses that same number for `OP_FloatListThing`. Opcode numbers must be interpreted
within their client generation.

The installed Titanium program's serializer at `0x4b6f1f` writes:

| Offset | Type | Meaning |
|---|---|---|
| 0 | f32 little-endian | Y |
| 4 | f32 little-endian | X |
| 8 | f32 little-endian | Z |
| 12 | u8 | Cause |
| 13 | u16 little-endian | Low 16 bits of the millisecond clock |
| 15 | u16 little-endian | Wrapping record sequence |

The [server structure](https://github.com/EQEmu/Server/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/common/eq_packet_structs.h)
groups the last four bytes into a single `timestamp`. The client writes two
separate words; treating them as one clock loses the sequence information.
Cause names 1–4 follow the server enum. Other codes remain unclassified in the
decoder, including codes the client serializer accepts but whose meaning needs
additional evidence.

## Client path

The program enables collection during zone entry (`0x4aba14` calls `0x4910fa`)
and resets the queue and its entry counter at zone entry, after the
zone-change flush and when the player's own spawn is created (`0x4b6ef8`).
Recording (`0x4b70db`) additionally checks game/player state. This is not an
empty heartbeat: every record is an entry the counter counts.

What this library records (`movement::history::Recorder`, owned by the
movement session next to its update counter, so a record's sequence is the
counter's value when it was recorded):

| Cause | When | Height | Label |
|---|---|---|---|
| 1 | Each frame the counter counts (65 a second). A moving player's frames, a jump's rise and fall included, are placed along the step between the host's samples; standing, they are at the player's place | Z + L | [R] `0x48a7f9`; [C] about 66 a second moving; placement [I] |
| 1, repeated | A frame at the place of the last frame written is consumed, not written; its sequence still goes by | | [R] `0x4b6f1f`; [C] standing reports carry no frames |
| 2 | Each position update sent, after the update copied the counter: the record's sequence is the update's offset 2 | Z + L | [R] `0x4ac5bc`, `0x4ac76a`; [C] 261 of 261 |
| 4, 5, 1, 2, 3 | A same-zone pad crossing: the region's two entries and the warp's frame at the old place, the entry update, then the landing | 4, 5 and 3 without L; 1 and 2 with L | [R] `0x48b60e`, `0x48b647`, `0x48aaa6`, `0x48ac4b`; 4 and 5 height [I] |

The frozen update (`0x4ac842`) records nothing and repeats the last counter.
Causes 7 and 8 and the spell-path entries (`0x415445`, `0x415d65`,
`0x415ec3`, `0x48ba54`) are left out, as the counter leaves them out: their
triggers in this session are unread (6.420). Nothing is recorded for a server
relocation or correction; the frames before a correction stay at the old
place.

L is 1.9 [C]: both recordings show record Z minus update Z of exactly 1.9 for
a human. The read takes the larger of 1.9 and the model's own height (actor
virtual `+0x28`), which is unread, so a larger model may differ [I]. The
clock word is the low 16 bits of a millisecond clock that starts at the first
record of the process; the program counts from game start [R], and only the
low word is on the wire.

Sending (`0x48c1a3`, opcode `0x6a1b`):

- An unforced report goes out once the oldest queued record is at least
  1,000 ms old [R]. The session checks after each accepted sample and on
  each tick. The serializer's own 2 s hold compares the tick count with the
  game clock and never holds a record back [R], so a report carries
  everything queued, oldest first, at most 10,000 records; the rest wait [R].
- The zone-change request is preceded by a forced report, whatever the
  records' age [R] `0x4ab056`; [C] in the same millisecond as `0x5dd8`.
- A report is never empty. Its body is `17 * count + 1` bytes; the program
  leaves the last byte unwritten [R] and recordings show 0 or a stale value
  [C]. This library writes 0 [I].
- Delivery is reliable on the zone stream, fragmented when large [C].
- Expect about one report a second, standing included: standing, each
  report is the one update sent that second [C].
- Records still queued when an admission ends without a zone change are
  dropped [I]. If reports cannot go out, the queue keeps at most 32,768
  records, dropping the oldest and saying so in the diagnostics.

The gate: the zone session reports only when the wire is Titanium's, the
counter rule is `HistoryEntries`, and the zone admission did not make the
validation exchange (it notes the exchange as it sends `0x1251`, before the
player is admitted). Where the exchange was made, the history is ended for the
admission and nothing is sent, while the update counter goes on counting
exactly as before (6.369).

## Server behavior and capture limits

EQEmu's [history handler](https://github.com/EQEmu/Server/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/cheat_manager.cpp)
refreshes its timer before validating the body, then rejects a body shorter than
17 bytes unless a port exemption already applies. A timer refresh is not evidence
that an empty body is a valid client report. Its port and warp handling also makes
invented causes inappropriate.

Observed public-server traffic contains ordinary position updates and no
history reports. The native WLD pad path at
`0x48b52c`, `0x48b42b`, and `0x48aaa6` submits a 36-byte position at
entry, snaps locally, and returns to the normal position stream. The local
pad implementation uses the existing position encoder and update gate.
Its event records local placement after entry submission. Every Titanium
update, the entry included, carries the movement-history entry counter
(`movement::UpdateCounter`): frames since admission at a fixed rate, one
entry for each earlier update, and for a pad the region's two entries and
the warp's entry at the old place before the entry report copies it. The
rate and the left-out spell-path entries are inferred (recording checklist
6.420). Live validation of pad crossings and server receipt is pending.

Remaining checks: the actor height term in L, the spell-path causes, and a
rerun of the official-client comparison on a local server after each change
to the sender.
