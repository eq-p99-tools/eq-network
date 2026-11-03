# EQMac abilities: TAKP and Quarm

Inspection: 2026-11-02. This is source inspection and offline synthetic
validation, not a public-server compatibility claim. No game server was
contacted and no installed assets, identities, captures or credentials are
fixtures. Ordinary autoattack, spell/book operations, inventory commands and
movement-speed inputs are separate workstreams.

## Primary evidence

Quarm was inspected at
[`c2cd2651045ab063b236b08f3a9e83cbf10289e9`](https://github.com/SecretsOTheP/EQMacEmu/tree/c2cd2651045ab063b236b08f3a9e83cbf10289e9).
TAKP's maintained server source is `EQMacEmu/Server`, inspected independently
at its retrieved main head
[`e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f`](https://github.com/EQMacEmu/Server/tree/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f).
These repository revisions do not establish the public servers' deployed builds.

* Quarm's [opcode table](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/utils/patches/patch_Mac.conf)
  and [packed structures](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/eq_packet_structs.h#L1351)
  establish the ability packets. `ClientTarget_Struct` is at line 809,
  `CombatAbility_Struct` at 1351 and `ResetSkill_Struct` at 2751. The Mac patch
  has no translation replacing these ability layouts.
* Quarm [client handlers](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_packet.cpp)
  are CombatAbility 3691, FeignDeath 4686, Fishing 4742, Forage 4762,
  Hide 6294, Mend 6728, SenseHeading 8033, Sneak 9128 and Taunt 9634.
  The [special attack dispatcher](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/special_attacks.cpp#L418)
  determines class dispatch and the shared strike timer.
* TAKP's [handlers](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/zone/client_packet.cpp),
  [dispatcher](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/zone/special_attacks.cpp#L418),
  [structures](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/common/eq_packet_structs.h)
  and [opcodes](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/utils/patches/patch_Mac.conf)
  independently support the same implemented subset. Fishing differs between
  the repositories and is deliberately not inferred from ancestry.

## Every typed ability

All numeric body fields are little endian. `strike(s)` means target `u16`,
reserved `u16 = 0`, attack `u32 = 100`, skill `u32 = s` (12 bytes). Titanium
instead names its target as a `u32` in the same 12-byte strike body. Empty
bodies follow handlers that do not read any payload; they are synthetic
source-compatible requests, not newly captured official-client packets.

| Ability (skill) | Titanium request | EQMac request | Local eligibility / status | Local reuse seconds |
| --- | --- | --- | --- | --- |
| Kick (30) | `5ee8`, strike | `6041`, strike(30) | Warrior, ranger, monk, beastlord; profile-qualified | 8 |
| Bash (10) | `5ee8`, strike | `6041`, strike(10) | Warrior/cleric/paladin/shadow knight and known shield; or barbarian/troll/ogre slam | 8 |
| Backstab (8) | `5ee8`, strike | `6041`, strike(8) | Rogue, qualified skill and known primary 1H piercing weapon | 10 |
| Frenzy (74) | `5ee8`, strike | None implemented | **Inapplicable in inspected dispatchers:** no berserker/Frenzy class case; do not offer or encode | None offered |
| Flying Kick (26) | `5ee8`, strike | `6041`, strike(26) | Monk, qualified skill | 8.5 |
| Round Kick (38) | `5ee8`, strike | `6041`, strike(38) | Monk, qualified skill | 8.5 |
| Tiger Claw (52) | `5ee8`, strike | `6041`, strike(52) | Monk, qualified skill | 6 (Titanium 5.5) |
| Eagle Strike (23) | `5ee8`, strike | `6041`, strike(23) | Monk, qualified skill | 6 |
| Dragon Punch / Tail Rake (21) | `5ee8`, strike | `6041`, strike(21) | Monk, qualified skill | 6.2 |
| Taunt (73) | `5e48`, target `u32` | `3b41`, target `u16` | Warrior/paladin/ranger/shadow knight, qualified skill, NPC in melee reach | 6 |
| Hide (29) | `4312`, empty | `8641`, empty | Positive qualified skill; success is server-owned | 10 |
| Sneak (42) | `74e1`, empty | `8541`, empty | Positive qualified skill; already-sneaking requests refused | 10 |
| Forage (27) | `4796`, empty | `9440`, empty | Positive qualified skill; sitting refused | 100 |
| Fishing (55) | `0b36`, empty | Known opcode `8f41`, empty handler input | **Gap:** resource, location, delayed completion and refusal flow not ported; refused | None offered |
| Bind Wound (9) | `601d`, 8 bytes | Known `9340`, target `u16`, type `u8`, reserved byte | **Gap:** 4-byte request/reply and consumption/completion flow not ported; refused | None offered |
| Mend (32) | `14ef`, empty | `9d41`, empty | Monk, qualified skill (zero permitted); HP is never predicted | 360 |
| Feign Death (25) | `7489`, empty | `ac40`, 6 bytes: own spawn id, zero word, two zero bytes (program 0x4bdf2f) | Monk, qualified skill; intentional lying posture before attempt, no success assertion | 10 |
| Sense Heading (40) | `05ac`, empty | `8741`, empty | Qualified skill, including zero; existing consumer displays heading locally | 2 |

The new `abilities::eqmac` module owns packet layouts, eligibility, reset
records and range policy. `Request::Ability` dispatches there only on EQMac.
Both policies offer its explicit 15-element list, never `Ability::ALL`.

Quarm's profile builder at `client_packet.cpp:1908` distinguishes 254
(untrained or not yet eligible), 255 (unavailable), and 0 (eligible without
training). Values through 252 are ranks; 253 and other unknown values are
preserved in the profile but do not admit an action. Mend, Sense Heading and
qualified combat skills can start at zero; Hide/Sneak/Forage require positive
skill because their handlers call `HasSkill`. Class gates protect monk-only
handlers and strike dispatch even if a synthetic or stale profile contradicts
the class. Unknown class prevents class-specific abilities. Racial slam is
the dispatcher’s explicit exception to a missing/unavailable Bash skill.

The controller requires admission and visible, present, non-self targets;
taunt requires an NPC, strikes allow players subject to server PvP rules.
Equipment gates use authoritative, non-stale inventory snapshots and never
send an inventory command. Shieldless knight epic bashing remains a gap in
this conservative gate. Facing, line of sight, backstab positioning/AA,
fear/stun/mez/invulnerability, attack state, and server skill caps remain
server decisions where the session lacks the required authoritative inputs.
Existing casting/death/transfer command guards continue to apply.

## Recovery and state

The existing client's unmodified reuse table is retained, including its
previous program-read references (`program-read-gate-v2`, T7). Those archived
decompile artifacts were not relocated or independently re-read in this task;
the table is inherited implementation evidence. It is not a live measurement.
The inspected [server constants](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/features.h#L92)
produce different server floors: Mend 289s, Forage 49s, Sneak 6s, Hide 7s,
Feign Death 8s, Taunt 5s; the handlers subtract one from their constants.
Strike floors at 100 haste are Bash/Kick 7s, Backstab 9s, Flying/Round Kick 7s,
Tiger Claw 6s, Eagle Strike/Dragon Punch 5s. Tiger Claw's inherited 5.5s wait
was therefore raised to 6s on EQMac. Strike timers are shared and based on
the last strike used; other abilities have independent recovery groups.
Haste, slowing, AA and focus are not newly modeled. The server can reject
an attempt after the local wait, particularly under slowing. No HP changes
or success timers are fabricated; `AbilityUsed` means the request was sent.

`ResetSkill` (`3d42`) is eight bytes. The new `SkillReset` event retains skill,
padding and raw timer. Only timer zero also emits existing `SkillReady`.
Unknown/nonzero timers do not release anything. A known, offered skill
releases its actual recovery group, including the shared strike group;
unknown IDs and unsupported Frenzy cannot release it. The same operation in
the client consumer releases its displayed cooldown without an `AbilityUsed`
or a success notice. Existing Bind Wound completion still frees its own group.

Quarm's [Mend handler](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_packet.cpp#L6728)
starts `mend_reset_timer` when an attempt follows zoning before the persistent
server timer expires. [Client processing](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_process.cpp#L551)
then calls `ResetSkill`; [ResetAllSkills](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client.cpp#L1918)
uses the same exchange for GM timer clearing. Network recovery is scoped to
the zone session; the consumer retains its existing timers across zoning.
Server resets reconcile the waits when sent. Admission does not invent
unknown persistent remaining times, nor does this task claim exact resume
of every server timer after reconnecting.

`FeignDeath` sets server-internal `feigned` after its roll, with a failure
message but no positive success packet. `SetFeigned` does not send posture.
The [appearance handler](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_packet.cpp#L9205)
separately accepts animation 115 (lying), interrupts casting and excludes
the sender from its broadcast. After all local refusals, this client sends
that posture and reports the intentional local stance, then sends FeignDeath.
This ordering is the `EQMac` program's (skill dispatcher 0x4bdf2f, feign-death
case: posture setter 0x50be3c sends the lying appearance, then the feign
notice goes out); the setter sends nothing when the player already lies, and
the client does the same. Lying does **not** mean a successful feign; server failure
chat remains authoritative, and ordinary stand/sit/duck clears server feign.
An ability refused for eligibility or recovery sends neither packet.

Hide and Sneak requests never set local success. Sneak appearance (type 15)
and visibility (type 3) update existing entity/consumer state. Quarm even
sends a deceptive invisible appearance to non-rogues after failed Hide, so
visibility cannot be relabeled confirmed hiding. `CancelSneakHide` (`5a41`,
empty) clears observed sneaking and emits an existing own-spawn appearance
event for consumers, without releasing recovery or acknowledging the packet.
[CommonBreakSneakHide](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/attack.cpp#L4619)
clears server state before sending it. Spell invisibility is not guessed from
a sneak-only cancellation: only the visibility packet changes visibility.
Explicit user toggles to cancel active Hide/Sneak remain unimplemented.

EQMac melee reach is three-dimensional and based on model bounding radii and
Z offsets, unlike the shared Titanium formula. Quarm
[`Client::CombatRange`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/aggro.cpp#L1369)
and [TAKP's implementation](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/zone/aggro.cpp#L1346)
guarantee at least 16 units. This patch admits that conservative sphere for
both strikes and taunt. Exact extended reach for large models remains a gap;
using the old two-dimensional Titanium formula could admit other-floor targets.

## Offline checks and remaining public checks

Synthetic codec tests assert every supported opcode/body, generation-specific
taunt width, zero/missing target rejection, unsupported actions, sentinel and
class boundaries, complete reset retention, malformed resets and 3D range.
Both full TAKP/Quarm assemblies test admission/offered abilities, every
supported action, no-send refusals, equipment/target/visibility/range gates,
independent and shared recovery reset, nonzero/unknown resets, stealth
cancellation, no acknowledgement, deliberate feign posture and standing,
and no predicted HP/visibility/sneak success. Existing deterministic controller
tests check reuse expiry and Bind Wound completion. The separate client patch
tests displayed reset groups and the same EQMac eligibility rules.

Public validation remains open for all 15 abilities: successful and failed
Mend, post-zone early Mend/reset, feign success/failure with attack off and
standing afterward, Hide/Sneak outcomes and cancellation (including spell
invisibility), forage item delivery/refusal, each strike and shared cooldown,
taunt NPC/range rejection, skill training transitions and server reset after
GM timer clearing. Compare the feign posture/request order with an authorized
official-client trace. Large-model reach, haste/slowing/AA/focus recovery,
knight epic Bash, active-stealth toggles, complete persistent recovery,
fishing and Bind Wound remain implementation/verification gaps.

### Batch for the dedicated live-check chat

Run only after the coordinator authorizes the single live-check batch and
the combined stack is integrated. All rows apply separately to TAKP and
Quarm; success on one does not certify the other. Use an authorized test
character with the stated class/skills, a safe location, and private logs.
Do not attempt currently unsupported Fishing, Bind Wound or Frenzy on wire.

| Case / prerequisites | Steps | Observable result | Risk / cleanup |
| --- | --- | --- | --- |
| Monk, Mend trained or eligible at zero, non-full HP | Use Mend once, then repeat immediately; wait for server HP/message | One request, authoritative heal/failure/worsening only; repeat refused locally; no optimistic HP change | Mend can worsen wounds; use safe HP margin and let HP recover |
| Monk, persistent Mend reuse | Use Mend, zone/reconnect while recovering; when UI permits, retry before server timer ends, then await server reset | Early server refusal followed by `3d42` timer zero; both controller and button release; retry sends | Respect reconnect rules; return to safe parking, no rapid reconnect loop |
| Monk, Feign Death, autoattack off | Attempt once; note lying, server failure chat if any; stand; compare with authorized official-client wire | `f540` type14/115 then empty `ac40`; no success assertion; standing clears feign and lying; duplicate during reuse sends neither | Failed feign retains aggro; use harmless/no-engagement location, stand at end |
| Hide/Sneak skills, rogue and non-rogue where available | Attempt, observe appearance/chat; induce an authorized ordinary cancellation | Server appearance governs visual state; non-rogue Hide appearance is not success proof; cancel clears Sneak, sends no acknowledgement and preserves recovery | Cancellation can expose character; avoid hostile observers, finish visible/not sneaking |
| Sneak plus separately granted spell invisibility | Cause sneak-only cancellation and inspect incoming visibility packets | Client must not clear spell visibility solely because of `5a41` | Only with separately authorized spell test; clean up invisibility normally |
| Forage skill, inventory/cursor space | Try seated (refusal), stand, use once, repeat during reuse | No seated send; item/refusal comes from server; no locally invented item; independent recovery | May add cursor item; arrange inventory space before test and clean cursor afterward |
| Appropriate strike class/skill; shield or piercing weapon where required; authorized harmless NPC encounter | Test Kick, Bash/slam, Backstab and each monk strike; immediately try a different strike; test just inside/outside conservative 16-unit 3D reach | Exact `6041` skill request; server damage/miss only; shared recovery blocks second strike; no out-of-range send | Attacks create aggro; require designated target/encounter, stop attacks and disengage safely |
| Tank class and Taunt, authorized NPC encounter | Taunt near NPC; try player/corpse/missing/hidden/far targets | Two-byte `3b41` only for accepted NPC/range; refusal sends nothing; cooldown independent of strikes | Taunt changes aggro; tank/healing plan and disengagement required |
| Skill training transition / Sense Heading | Observe 254/255 unavailable skills; train through normal authorized flow, receive skill update; use Sense Heading | Menu changes from authoritative skill values; heading output matches orientation, no fabricated success roll | Training may spend points; obtain separate training authorization |
| Controlled server with authorized GM timer clearing | Start Mend and a strike, clear timers and observe reset IDs | Known reset frees only matching/shared group; no HP or success notice | Not a public-player requirement; never assume public GM access |

Publish only synthetic reproductions of defects. Keep any live identities,
coordinates, traces and credentials in the private batch record; send findings
through the coordinator before reviewer ownership freezes the PRs.
