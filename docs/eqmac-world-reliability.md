# EQMac world selection reliability

## Cause and source evidence

The legacy client previously sent its first reliable application at ARQ zero
with `SEQStart`. During selection, that application is the world login. The
world loop continues calling the transport while selection or an unsubmitted
creation form is open; opening or changing that form sends no application.

The inspected Quarm source is SecretsOTheP/EQMacEmu at
[`c2cd2651045ab063b236b08f3a9e83cbf10289e9`](https://github.com/SecretsOTheP/EQMacEmu/tree/c2cd2651045ab063b236b08f3a9e83cbf10289e9):

- [`EQOldPacket::ReturnPacket`, lines 768-782](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/eq_packet.cpp#L768)
  omits ARSP when `CACK.dwARQ` is zero. Its `0xffff` exception does not repair
  idle world login: [`MakeEQPacket`, lines 2292-2305](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/eq_stream.cpp#L2292)
  constructs the pure ACK without setting that opcode.
- [`ProcessPacket`, lines 2002-2014](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/eq_stream.cpp#L2002)
  also drops `SEQStart` when its ARQ equals the expected next value. A new
  stream has last CACK zero, so starting at **one with SEQStart is rejected**.
  Starting at two establishes last CACK one, then accepts and acknowledges two.
- [`IncomingARQ`, lines 1918-1932](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/eq_stream.cpp#L1918)
  records the next reliable request. A later nonzero client request therefore
  permits a cumulative ACK that retires login as well.
- Retrying the original zero-valued sequence-start login resets the server's
  receive origin again. [`HandleSendLoginInfoPacket`, lines 303-326](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/world/client.cpp#L303)
  then sends world settings, approval, entry, expansions and character list
  again. This explains the repeated bootstrap messages without assuming that
  repeated application opcodes alone establish an ACK defect.

TAKP was compared separately at EQMacEmu/Server
[`e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f`](https://github.com/EQMacEmu/Server/tree/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f).
That revision uses rdplib, rather than Quarm's old stream implementation.
[`rdp.c`, lines 458-478](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/submodules/rdplib/src/rdp.c#L458)
requires SYN/sequence-start to establish a connection;
[`rx.c`, lines 371-375](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/submodules/rdplib/src/rx.c#L371)
derives the receive origin from that first message ID. The nonzero startup
therefore retains the sequence-start flag. Shared ancestry is not evidence
that TAKP has Quarm's omitted-zero-ACK behavior.

These are pinned source findings and synthetic validation. The deployed public
server revision is unknown. Earlier private official-client capture/decompile
notes cover Titanium/P99 and do not prove an EQMac startup value. No live
transport capture is used as a fixture or claimed as packet-level proof here.

## Change and offline coverage

`OldSession` starts outgoing reliable ARQ at two. Incoming ARQ zero remains
valid, including after wraparound. A repeated sequence-start at the same ARQ
does not rewind delivery or clear buffered successors; a distinct sequence-start
still establishes a new origin. New world connections use new sockets/state.
No retry interval, retry budget, selection deadline or creation deadline changes.
Titanium/P99's modern transport is unchanged.

The synthetic world peer models both Quarm startup checks and drives the actual
world loop and UDP transport. Separate initial-selection and camp-return-context
cases each idle for 31 seconds, beyond the existing send retry budget, while
reliable keepalives exercise ACK pumping. They then approve and create a synthetic
cleric, reassemble the 8452-byte request, publish the refreshed list, select the
new character, and accept the zone destination. A third world case uses TAKP's
policy, requires SYN on connection startup, and sends a SYN at a distinct server
origin before the same dwell/creation/handoff flow. It models the inspected
rdplib handshake constraints, not the entire rdplib implementation.
Camp-return coverage starts at
the fresh world stage with the previous character in logging context; it does
not simulate the preceding zone's camp countdown or drive the graphical form.

Transport regressions cover a nonzero sequence-start, server startup at zero
without sequence-start, reordered fragments, duplicate starts, distinct starts,
cumulative/selective ACKs, wraparound, byte-identical retries without consuming
ARQ, and exhaustion of the unchanged retry budget. Mutation checks restore
startup zero, startup one, and the former duplicate-start handling independently;
the corresponding regression fails for each.

## Live retest still required

Use the single consolidated build after integrating the profile-skills fix:

1. Dwell at Quarm selection for at least 90 seconds; check no disconnect or
   repeated login/bootstrap cycle.
2. Open the actual creation form, select Cleric and edit choices for at least
   90 seconds. Complete the user-authorized creation, confirm the new list and
   enter the character normally.
3. Exercise normal admission and ordinary zone traffic, then normal camp.
   Dwell at returned selection for at least 90 seconds; reopen the form and
   enter normally. Retain clean zone/camp behavior.
4. Check P99 login, selection dwell, admission and normal camp with its unchanged
   modern transport. Separately exercise TAKP startup/dwell when its fixture is
   available; source comparison is not live compatibility validation.
