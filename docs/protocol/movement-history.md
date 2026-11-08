# Titanium movement history

Status: record decoding only. Sessions do not emit movement-history packets.
The P99 DLL suppresses the opcode in P99 mode. Stock Titanium transmission
still needs its packet trailer and scheduling established.

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

The program enables collection during zone entry (`0x4aba14` calls `0x4910fa`).
Recording (`0x4b70db`) additionally checks game/player state. Collision updates,
teleports and zone-line paths enqueue records; this is not an empty heartbeat.
The serializer filters duplicate collision positions and normally waits until
records are at least 2,001 ms old. A forced flush bypasses the age check.

The sender (`0x48c1a3`) calls the normal send wrapper with opcode `0x6a1b`, a
populated buffer, its serialized length, and flag 1. No report is sent when the
serializer returns zero. The serializer returns `17 * count + 1`; the extra
byte's meaning and initialization remain unresolved. The record decoder therefore
does not guess a packet trailer, delivery policy, scheduler, or sending API.

## Server behavior and capture limits

EQEmu's [history handler](https://github.com/EQEmu/Server/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/cheat_manager.cpp)
refreshes its timer before validating the body, then rejects a body shorter than
17 bytes unless a port exemption already applies. A timer refresh is not evidence
that an empty body is a valid client report. Its port and warp handling also makes
invented causes inappropriate.

Observed P99 traffic contains ordinary position updates and no history
reports. The P99 client extension suppresses `0x6a1b` in P99 mode while
allowing `0x14cb`. The native WLD pad path at
`0x48b52c`, `0x48b42b`, and `0x48aaa6` submits a 36-byte position at
entry, snaps locally, and returns to the normal position stream. The local
pad implementation uses the existing position encoder and update gate.
Its event records local placement after entry submission. The entry uses
the ordinary session position sequence. The native pad path uses a movement
history cursor, so the entry word still needs a capture comparison
(recording checklist 6.402). Live
validation of pad crossings and server receipt is pending.

Before implementing stock Titanium transmission, capture a populated report
and establish its trailer, clock and sequence evolution, record eligibility,
flush boundaries, and delivery mode. Compare the sender with the official
client on a local server.
