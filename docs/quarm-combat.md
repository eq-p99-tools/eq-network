# Normal attack and consider

The normal combat command and EQMac codecs already existed. This change adds
typed local request dispositions and admission/visibility guards, and retains
the existing typed authoritative server records. It does not add an attack
scheduler, hit calculation, resource formula, ability, or inventory action.

## Pinned source evidence

The references below are to Quarm's public server source at
[`SecretsOTheP/EQMacEmu`, c2cd2651045ab063b236b08f3a9e83cbf10289e9](https://github.com/SecretsOTheP/EQMacEmu/tree/c2cd2651045ab063b236b08f3a9e83cbf10289e9).
They establish source behavior, not the deployed public server revision.

- [`zone/client_packet.cpp:2384`, `Handle_OP_AutoAttack`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_packet.cpp#L2384)
  consumes a four-byte toggle and does not acknowledge it. It accepts a toggle
  without a target. Optional administrative and expansion rules can reject it;
  their configuration is not available to this session and acceptance is never
  claimed. A disable request is not held behind a cast or inventory move.
- [`zone/client_packet.cpp:3709`, `Handle_OP_AutoAttack2`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_packet.cpp#L3709)
  is a no-op. The inherited EQMac program-derived sequence sends this notice
  immediately before the toggle with the same body (existing checklist 6.157).
  This work does not introduce a new program-read or capture claim.
- [`zone/client_packet.cpp:3772`, `Handle_OP_Consider`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_packet.cpp#L3772)
  finds the named target and returns its faction and level assessment. It has
  no melee-range check and can assess the player. Missing targets get no reply.
  Optional extra engagement/ruleset messages stay ordinary server messages;
  they are not synthesized. The wire faction field is preserved as received.
- [`zone/client_process.cpp:275`, normal attack processing](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_process.cpp#L275)
  decides whether each swing may occur, including casting, posture, control
  effects, attack timers, target, range and line of sight. A toggle is not a
  strike. Reusing ability range/weapon gates for the toggle would conflate
  those operations. Existing targeting, equipment and ability owners remain
  responsible for their own actions; the server owns normal swing results.
- The pinned [Mac opcode table](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/utils/patches/patch_Mac.conf#L130)
  identifies the existing EQMac attack/consider opcodes. The library keeps its
  established byte order and existing four-byte/24-byte serializers.

## Authority and refusal

`WorldEvent::CombatRequest` reports local submission or refusal, with admission
and request kind. Shared command age/admission checks and unavailable-policy
checks use this event too. The combat owner requires an admitted, active,
living player and, for consider, the correct own ID and a visible target or
the player. It does not require consideration to change the selected target.

`Sent` is never an acknowledgment. There is no timer that converts it into
acceptance. `Consideration` is supplied only by a decoded server response, with
the response's own target ID, faction, color and optional HP fields. `Damage`
and `Animation` remain the server's reported outcomes and motions; unknown
values are retained and malformed packets do not create successful outcomes.
The session does not infer attack acceptance from either event.

Quarm offers normal combat through `ServerType::combat` on its separately
reviewed targeting and lifecycle baseline. There is no ad-hoc server branch
in the shared combat handler. Every unrelated feature policy remains as it was.

## Offline checks and controlled live checklist

Synthetic tests run the actual feature assembly, command guards, EQMac wire
and host forwarding. They cover notice/toggle order for on and off; untargeted
toggles; self/distant considerations without retargeting; unavailable, invisible,
stale, future, dead and zoning refusals; server-only consideration/hit/avoidance
records; unknown values; and malformed consideration packets, on both TAKP
and Quarm. The unavailable-policy refusal path is covered independently.

No live fights, server connections or desktop tests were performed for this
change. No new inference label is allocated. Public-server combat compatibility
and current-client toggle program gates remain unverified.

The reviewer may perform the following only in an independently authorized,
controlled session after the Quarm baseline and client pin are integrated:

1. In a safe location with no attackable target selected, consider the player
   and a visible distant spawn. Confirm requests/replies use the same target
   and ordinary targeting stays unchanged. Preserve any server refusal text.
2. With no selected target, turn normal attack on then immediately off. Check
   exactly one notice/toggle pair per command and no fabricated hit/success
   events. Check disable still sends while casting or after an item move.
3. With separate express authorization for a controlled combat target, inspect
   actual server damage/avoidance/animation, target removal and death/transfer
   gates. Stop on unexpected refusal/disconnect; do not automatically resend.
4. Compare the official client's guards and packet ordering before widening
   behavior. A source-derived/synthetic pass is not a live pass. Preserve raw
   captures and identities privately; report only bounded results publicly.
