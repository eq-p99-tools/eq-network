# P99 and Quarm: remaining client gaps

Inspection date: 2026-11-02. Baselines: networking
[`6ae8956`](https://github.com/eq-p99-tools/eq-network/tree/6ae8956068415fa30934485ccdae1a102757b50c)
and desktop client
[`c03fb30`](https://github.com/eq-p99-tools/eq-client/tree/c03fb30f191808840e7ee4eedfb540956bea7663).
This is a code, source and saved-capture inspection. **No new live connections,
desktop interaction, character creation or movement were performed.** Other
agents' changes may supersede this snapshot.

## What counts as a gap

- **Missing:** required codec, state handling or client interaction is absent.
- **Disabled:** implementation exists for another policy but this server does
  not offer it. This is a client gap, not evidence the server lacks the feature.
- **Unverified:** implemented/offered, but the particular scenario lacks a
  matching public-server check. Source inspection and synthetic tests do not
  close this gap.
- **Server restriction / not applicable:** evidence establishes that the
  server or protocol does not provide the facility. These remain documented
  below, separate from implementation work. An uncertain restriction stays a gap.

The inventory below groups related capabilities; it is not a percentage of all
EverQuest gameplay or all opcodes. A parser accepting one payload does not prove
the whole interaction works. The policy inventory is
[`servers.rs`](../crates/eq-network/src/client/session/servers.rs); wire coverage
is separately tracked under `client/session/coverage`.

## First three fixes

Ranked by evidence, user-visible value and whether the fix can stand on its own
without live tests. Broad Quarm movement remains urgent, but needs more than
turning on its movement capability.

| Priority | Confirmed defect | Change / remaining verification |
| --- | --- | --- |
| 1 | P99 and Quarm selection previews are discarded by policy despite the existing decoders. Quarm also loses level and zone. | Preserve their decoded appearance. P99 capture comparison and Quarm source layout support this; current-client public visual checks remain outstanding. |
| 2 | P99 jumping is disabled with a comment that it has not been measured, although archived official-client traffic includes jumps. | Enable the existing bounded jump path after checking the saved notices and motion. Keep client-code-derived physics and capture evidence distinct; a live custom-client jump remains unverified. |
| 3 | Quarm has no graceful camping action; sitting/standing is coupled to movement, blocking reuse of camp controls. | Separate posture control from locomotion and enable Quarm camping using its own handlers and wire format. Test cancellation, timer, reply and admission boundaries offline; public logout verification remains outstanding. |

### Evidence for selection previews

The private October 11 official P99 recording and continuation contain five
1,704-byte character lists. The current Rust list/profile/spawn decoders were
replayed offline: ten occupied entries across repeated lists, two first-admission
comparisons covering 24 field groups, zero mismatches. Four later admissions
were excluded to avoid comparing stale selection equipment. An independent
offset comparison also checked eyes, facial fields and all nine materials/tints.
This verifies those recorded fields, not every race or rendered model.

Quarm source at
[`c2cd265`](https://github.com/SecretsOTheP/EQMacEmu/tree/c2cd2651045ab063b236b08f3a9e83cbf10289e9)
fills names, identity, level, zone, cosmetics, materials, item colors and held
models in [`WorldDatabase::GetCharSelectInfo`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/world/worlddb.cpp#L40).
Its [`CharacterSelect_Struct`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/eq_packet_structs.h#L90)
matches the existing 1,620-byte EQMac decoder. This is source compatibility, not
proof of the revision deployed by the public world. Public regression data is
synthetic; captures, identities, keys and installed assets stay private.

### Evidence for P99 jumping

Offline replay accepted 14,313 CRC-valid datagrams and recovered 54 datagrams
from 27 coalesced capture frames. Eight outgoing `0x0797` notices have empty
bodies. Subsequent 36-byte `0x14cb` updates include rising and falling vertical
motion. Terrain/steps and sparse updates prevent fitting a clean ballistic arc;
the recording does **not** establish exact takeoff speed, height or airtime.
The existing 44.55 units/s jump and generation-specific gravity come from the
client-code analysis documented in `movement.rs`. Reusing those rules is an
explicit inference about P99's patched client. Do not relax speed, admission,
correction, climb or fall guards to make a test pass.

### Changes completed in this stack

All three fixes above are implemented. Preview-policy tests preserve sparse
slots, equipment, level and zone while retaining malformed/duplicate rejection.
P99 jump tests check the recorded empty notice and admission prerequisite;
existing motion-guard tests cover rise limits and correction resets. Quarm tests
exercise sit/camp/logout/reply and cancellation through its actual feature set,
without a movement grant. Public-server validation remains open for all three.

`Capability::Posture` now governs `SetPosture`; `Moving` governs locomotion.
Consumers must update stance controls rather than advertise walking to enable
camping. All four server policies offer posture.

Quarm source evidence: `Handle_OP_Camp` starts a 35-second server fallback;
`Handle_OP_Logout` saves, clears the active account, replies and disconnects.
The reused client countdown is 30 seconds, with a 10-second EQMac reply timeout;
that client timing is inherited/inferred, not measured on public Quarm.
`Handle_OP_SpawnAppearance` processes sitting/standing and cancels camping when
standing. See [the inspected handlers](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_packet.cpp).

## Confirmed implementation and policy gaps

This table records the baseline above; the first three fixes are called out
separately so a new capability is not mistaken for live verification.

| Area | P99 baseline | Quarm baseline | What remains |
| --- | --- | --- | --- |
| Login / continued admission / chat | Exercised previously | Exercised previously | Compatibility is scenario-specific. Final Quarm extended-stamina change needs a public retest. |
| Character creation | UI, rules, codecs and synthetic tests exist | UI, rules, codecs and own-server source checks exist | Create/reject/refresh/enter on each public server; Quarm account-specific mule limits and live expansion settings need verification. |
| Character deletion | No client action | No client action | Typed request, confirmation, response handling and stale-list safeguards; do not treat this as server-unsupported. |
| Character previews | Disabled by policy | Disabled; level/zone stripped too | Fix 1, followed by public visual checks and broader model/gear coverage. |
| Walking/running/strafe/backward/turn | Offered, earlier scenarios exercised | Disabled although EQMac motion exists | Quarm must receive coherent inventory/coin inputs for carried weight, handle corrections and pair movement with transfer/death handling. A bare capability switch is insufficient. |
| Jump / fall | Falls offered, jump disabled | Both disabled | Fix 2 for P99. Quarm still needs its motion prerequisites, jump/fatigue and damage/death checks. |
| Movement accuracy | Bounded prediction and recent steep-slope fix | EQMac rules available behind policy | Weight reduction, effective STR/buff assumptions, roots, sneak, low HP, levitation and unusual speeds require scenario coverage. Full movement parity is not claimed. |
| Swimming, water/lava, drowning, boats | Incomplete | Incomplete/disabled | Frontend hazard guard is not swimming. Boat attachment/control, liquid movement and damage/resource/death handling need implementation and evidence. |
| Zoning / teleports | Zone-line and server-directed handoffs implemented | Transfer feature disabled | Quarm source supports transfers; integrate world re-entry, bind/death requests and refusal recovery. Same-zone teleporter/pad detection remains incomplete on both. |
| Death / respawn | Server death and offered-bind path implemented | Disabled; richer TAKP path exists | Verify lethal hits, fall/environmental deaths, bleed-out, same-zone bind and reconnect. EQMac client-reported death is held back without complete item-HP accounting; copying TAKP policy does not solve it. |
| Camping / posture | Offered | Disabled | Fix 3; Quarm walking remains separate. |
| Inventory / bank / coin moves | Offered | Disabled; EQMac codecs/reducers exist | Quarm needs its own policy/evidence. Item dropping/destruction and shared-bank actions lack desktop paths; server applicability must be checked separately. |
| Merchants / trade / give | Offered, some scenarios unverified | Disabled | Quotes offered by merchants are a separate missing policy on P99. Two-player trade needs distinct coverage from NPC giving. |
| Casting / spellbook / click effects | Offered; book moving enabled | Disabled | Quarm port of casting and authoritative bar recovery; P99 spell-action landing flag remains unset. Public scenario coverage for resisted casts, interruption, clicks, scribing and memorization. |
| Cooldowns / buffs / resource bars | Partial | Partial | Linked reuse groups, Quarm saved gem cooldowns, complete effective resource denominators, buff replacement ambiguity and item-effect accounting. Unknown values must remain distinguishable from zero. |
| Target / consider / combat | Offered | Disabled | Quarm codecs exist in TAKP paths, but policy and live combat/death coverage are missing. |
| Abilities | Most offered; fishing and bind wound excluded | Disabled | Evidence for each unavailable action, including resource/reuse and failure responses. |
| Loot / corpses / consent | Offered | Disabled | Quarm loot, corpse commands, own-corpse recovery and NPC loot flow. |
| Doors / ground objects | Offered | Disabled even in richer TAKP policy | EQMac decoding and interaction evidence; rendering a local marker is not a pickup implementation. |
| Groups | Disabled | Disabled; TAKP implementation exists | Invitation, refusal, roster changes, leader changes, zoning and disband checks on each. |
| Raid group-leader removal | No policy offers it; generic EQEmu lacks the handler | No verified implementation | Remains a gap unless the specific public server proves it inapplicable. |
| Raids | Disabled; native raid-window restriction reported below | Disabled | Quarm has raid handlers and opcodes: this is a gap, not an established Quarm restriction. |
| Pets | Offered | Disabled | Public verification of supported commands; EQMac wire/action integration. |
| Training / reading / tradeskills | Disabled | Disabled | Generic client paths exist; verify/implement each dialect's packets and eligibility. |
| Resurrection / sacrifice / translocate | Disabled | Disabled (TAKP translocate exists) | Source-backed offers, explicit consent, rejection and transfer/death lifecycles. |
| Who / time of day | Offered | Disabled | Quarm handlers/policy; do not confuse received entity state with a complete `/who` answer. |
| AFK / anonymous / roleplay | Disabled | Disabled; TAKP offers anon/roleplay | Server-specific appearance rules and replies; TAKP itself does not establish AFK support. |
| Roll / emote / assist / show helm / regen ticks | Disabled | Disabled | Per-action codecs/policy and evidence. Do not grant these merely because a sibling server does. |
| Hiding acknowledgement | P99 reply behavior unverified | EQMac ends hiding itself | P99 needs evidence; redundant EQMac acknowledgement is not a missing action. |
| Local map/editor/help/free camera | Available independently of gameplay policy | Same | Target actions still need targeting capability. Existing local UI checks do not certify the whole public-server gameplay matrix. |
| Art/UI completeness | Partial | Partial | Assets come from user installs; model variants, equipment tints, animations and UI coverage remain incomplete. |

Creation slot counts are already being addressed by
[client PR #325](https://github.com/eq-p99-tools/eq-client/pull/325), alongside
AltGr, free-camera settings and script fixes. This effort does not duplicate it.
The steep-slope correction in client #323 is already in the inspected baseline.

## Unsupported or inapplicable facilities

These entries are retained for visibility. Their scope matters: an unavailable
client window does not mean the underlying group activity is impossible.

| Facility | Server / reason / evidence | Treatment |
| --- | --- | --- |
| Native raid window | P99: documented as disabled in the server community's [key-binding reference](https://wiki.project1999.com/Key_Bindings). P99 server code is not public, so this is a reported client restriction, not a packet-level proof. Raiding as gameplay still exists. | Keep the native raid capability off; verify current policy before implementing an alternative. Do not apply this exclusion to Quarm. |
| EQMac tutorial / return-home flags in character lists | Quarm's `CharacterSelect_Struct` has no Titanium tutorial/home fields; its current creation path does not provide Titanium tutorial entry. | Do not invent those fields or send Titanium tutorial packets. A custom Quarm facility would need separate evidence. |
| Titanium endurance field / packets on EQMac | Quarm uses EQMac fatigue/stamina, not Titanium endurance updates; see its `PlayerProfile_Struct` and `Stamina_Struct`. | Display supported resources using their actual semantics. Missing mana maxima or fatigue handling remain client gaps. |
| P99 spellbook deletion | Current policy records an official-client check on 2026-10-03 in which deleting did not work. This is observational evidence, not proof of a permanently absent server handler. | Keep disabled and retain this reported restriction; reverify if policy changes. Book moving is supported and is distinct. |
| Extra acknowledgement for ended hiding on EQMac | The inspected TAKP/Quarm behavior clears hiding itself; the additional Titanium EQEmu exchange is not required. | No speculative reply. P99's uncertain acknowledgement remains in the gap list. |

Quarm's [opcode table](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/utils/patches/patch_Mac.conf#L276)
and [`Handle_OP_Logout`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_packet.cpp#L6574)
explicitly include raid participation. No blanket “P99 and Quarm do not support
raids” classification is justified.

## Saved-packet backlog

- Incoming zone `0x1234` (800 bytes, six observations) and world `0x1235`
  (800 bytes, two) remain unidentified. Adjacent numbers do not imply shared meaning.
- Outgoing `0x6c5e` (12 bytes, eight observations) remains uninterpreted.
- Tribute notices and parts of the profile (AA/leadership/tribute) remain
  unmodelled. Their presence does not prove P99 exposes that gameplay.
- `AutoAttack2` and `ConfirmDelete` were observed, but inspected EQEmu handlers
  are no-ops. P99-specific significance remains unverified.
- Movement-history records have a decoder; the inspected recordings contain
  no examples of the verified history opcode. Trailer semantics and when the
  patched client sends it remain unresolved. No fabricated periodic history
  keepalive is enabled.
- Empty admission messages and known ignored/no-op packets are tracked
  separately; they must not inflate a count of wholly missing gameplay.

## Deferred live verification

No live tests are part of this batch. When explicitly resumed, record exact
client/library revisions and test one scenario at a time: selection appearance;
creation/rejection/entry; stationary, forward/back/strafe, jump/land; boundary,
teleport, refusal and death/bind; then camp/reconnect. For Quarm, establish safe
parking first. Respect public account reconnect cooldowns. Existing local
EQEmu/TAKP success is useful evidence, but does not certify public P99/Quarm.

Keep raw captures, session keys, credentials, names, positions and installed
assets private. Publish only synthetic tests, aggregate findings and source
references. Recheck main and overlapping PRs before publishing or integrating.
