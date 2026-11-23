# Titanium EQEmu boat acquire/release

`Capability::BoatControl` offers acquire/release only on the EQEmu policy.
P99 and EQMac remain unverified gaps. This does not enable vessel steering.

The eight-byte `OP_ControlBoat` (`0x2c81`) contains a little-endian u32 entity
word, a one-byte action (0 release, 1 acquire), and three unread bytes. The
decoder retains unknown actions, the full entity word and the trailer. The
sender chooses zero padding because the EQEmu handler ignores it. Native
Titanium's sender does not initialize that padding; zero is a compatibility
choice rather than an official fixed value or a request nonce.

The inspected server requires an NPC of race 141 or 502. It grants acquisition
when the vessel has no target, or already targets the caller with no hate.
Otherwise it sends generic string ID 1406 and no echo. Release clears the
target only if it is the caller. The handler echoes accepted acquisitions and
valid release requests; a release echo does not establish prior ownership.
Quest signals may follow either control operation. There is no range, class,
level or skill eligibility check in this handler.

This first Titanium client slice selects race 141 because the native
interaction requires that NPC race and the player's current floor link.
Race 502 support remains a client gap. The host binds a current floor sweep
and the last accepted rider position to one received carrier incarnation.
The worker independently retains support only from accepted movement proposals,
clears it on correction/suspension/admission, and rechecks that incarnation
along with its received report and accepted rider position. A queue acceptance or transmitted acquire remains `Acquiring`.
Only its live unambiguous acquire echo becomes `Acquired`.

A five-second local deadline is a conservative client policy, not a server
timer. Unanswered acquisition triggers release; silence never becomes refusal
or permission. The generic in-use line lacks request/vessel identity and is
not treated as a correlated result. Timeout, release, cancellation and spawn
retirement prevent a late acquire echo from restoring local control. The wire has no nonce, so an unresolved same-ID acquire cannot be retried
until a release echo fences the older exchange. That boundary is explicitly
refused before sending. The one exception is the same take asked again while
it is still `Acquiring` the same incarnation, as the program's second
interaction sends one (6.533, inferred from read R35 in combination): the
same 8-byte take goes again with no new pending entry, deadline or fence, so
repeated takes never extend the five-second wait, and the first matching
echo grants while later ones find nothing pending. A completed reliable, ordered release permits ordinary
release/reacquire cycles. EQEmu uses default reliable stream 0 for the echo,
processes received applications in order, and this client's modern transport
delivers only the next reliable sequence while discarding past duplicates.
Thus all earlier acquire echoes precede the matching release echo. A late
release still supplies this fence after the local wait expires, without
restoring control or cancelling another vessel's pending acquisition. Timeout
or spawn replacement alone supplies no such fence.

Losing the floor link requests release while the same vessel incarnation is
still available. Camp, zone offers, own death and departure also stop/release
control where the connection permits; connection loss simply withdraws local
knowledge. A new admission starts a new owner. Raw packet observations remain
separate from this correlated state.

Primary references: EQEmu/Server
[`f5ec714c`](https://github.com/EQEmu/Server/tree/f5ec714c8835b2afd2be1812bb9033ff70a17a64),
`common/eq_packet_structs.h` (`ControlBoat_Struct`),
`utils/patches/patch_Titanium.conf`, `zone/client_packet.cpp`
(`Handle_OP_ControlBoat`, `Handle_OP_LeaveBoat`, `Handle_OP_ClientUpdate`), and
`zone/mob.cpp` (`IsControllableBoat`), `zone/client.h` (default reliable
`QueuePacket`), `zone/client_process.cpp` (ordered packet handling), and
`common/net/eqstream.cpp` / `common/net/reliable_stream_connection.cpp`
(reliable stream 0 ordering). Native Titanium `eqgame.exe` interaction
is at 0x4a7356, acquire reception at 0x45cff0–0x45d066, and floor-link loss at
0x4a1d74. Native movement dynamics/cadence and control across zone admissions
are still unread; the permissive server position handler supplies no substitute
for that contract. Source and synthetic coverage are not live validation.
