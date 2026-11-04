# Modern session termination

The modern SOE transport reports explicit peer termination as
`modern::SessionTermination` through its existing `anyhow::Result` return type.
It ends the transport attempt, discards queued incoming applications, fragments,
reorder data and retransmissions, and prevents further application sends. A
duplicate cannot replace the first reason. Closing an already terminated session
sends no reply. This is transport evidence, not an acknowledgement or outcome of
a gameplay, creation or deletion request.

`Client::run` emits its ordinary Disconnected status once for that attempt. With
`RunOptions::reconnect` disabled, it returns the typed error. With reconnect
enabled, the existing configured delay precedes a fresh login; no terminal
packet shortens it. An opaque disconnect reason or out-of-session reply does not
prove permanent refusal. The existing InvalidCredentials rule still prevents
automatic retries. Cancellation, normal local closing, retry timings and the
legacy EQMac transport retain their existing contracts. The desktop already
disables automatic reconnect and can display the ordinary error text without a
new event or consumer interface.

## Accepted inbound controls

| Control | Established form and checks |
| --- | --- |
| Session disconnect (`0x0005`) | Negotiated CRC/compression framing, a four-byte big-endian connection code matching this session, and optionally a two-byte opaque reason. Only four/six-byte decoded bodies are accepted. |
| Out of session (`0x001d`) | Standalone six-byte datagram: opcode and zero connection code, from the UDP socket's connected peer. It is read before negotiated CRC/compression, which do not apply to this stateless reply. |

Malformed or wrong-code disconnects do not terminate or refresh receive liveness.
Malformed, nonzero-code, encoded and nested out-of-session forms are ignored.
An out-of-session datagram arriving before matching session negotiation does not
establish a session and is ignored. The zero-code reply has no authenticated
current-session identity: it cannot distinguish every stale packet if an endpoint
is reused. No invented request ID or timing heuristic is added to claim that it
can. Peer filtering is the UDP socket's existing endpoint restriction.

## Evidence and remaining limits

The pinned public SOE [connection implementation](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/common/net/reliable_stream_connection.cpp#L295)
sends out-of-session directly from its manager when no connection exists, with
code zero. Its [field serializer](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/common/net/reliable_stream_structs.h#L89)
and [packet writer](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/common/net/packet.h#L56)
produce six serialized bytes. The structure's `size()` constant is not the
serialized field count. The same connection source
[excludes this opcode from negotiated encoding](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/common/net/reliable_stream_connection.cpp#L944).
Native static inspection independently supports the encoding exception,
connection-code comparison and optional disconnect reason. Its optional session
resume branch requires another sender; this remains an open parity requirement.
Recognizing termination and starting a fresh login do not implement native resume.

Synthetic regressions cover framing variants, malformed/current/wrong codes,
peer filtering, duplicates, combined-packet ordering, queued-work withdrawal,
application send guards, retry policy and fresh-attempt reset. Test preparation
does not establish execution or live P99 compatibility. Public P99 loss/refusal
scenarios and additional control layouts remain unverified. No further world
refusal opcode, session-resume sender, private movement/history/system packet or
creation-policy change is enabled by this work.
