# P99 and Quarm: remaining client gaps

Inspection date: 2026-11-02. Baselines: networking
[`6ae8956`](https://github.com/eq-p99-tools/eq-network/tree/6ae8956068415fa30934485ccdae1a102757b50c)
and desktop client
[`c03fb30`](https://github.com/eq-p99-tools/eq-client/tree/c03fb30f191808840e7ee4eedfb540956bea7663).
The initial inventory is a code, source and saved-capture inspection. The
follow-up implementation and limited live checks are recorded separately below;
they do not certify every interaction. Other agents' changes may supersede this
snapshot.

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
| 1 | P99 and Quarm selection previews are discarded by policy despite the existing decoders. Quarm also loses level and zone. | Preserve their decoded appearance. P99 capture comparison and Quarm source layout support this; a public Quarm preview rendered, while P99's integrated-client check remains outstanding. |
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
without a movement grant. The public Quarm preview was subsequently exercised;
P99 preview/jump and a controlled camp/cancellation check remain open.

Quarm now also offers the existing EQMac ground movement, turn, jump, fall,
environmental damage and zone-transfer features. The shared inventory owner
retains inventory/coin state and now offers guarded item/coin moves, including
personal banking, under Quarm's own policy. Merchants, automatic consumption
and exchanges remain disabled. Assembly tests exercise
the Mac position/jump wire, correction, and bind requests after server-reported
death. This does not close swimming, boats, pad teleport detection or the
client-reported bleed-out gap.

Quarm target selection/clearing is also enabled independently of combat.
Its source handlers `Handle_OP_TargetMouse`/`Handle_OP_TargetCommand` at the
same revision read a two-byte `ClientTarget_Struct` and echo clears or restore
server-selected targets. Synthetic tests check the exact wire, self/NPC/clear,
hidden/missing rejection and server changes. Public Quarm self-targeting was
exercised; NPC targeting and clearing remain unverified. The separate normal
combat follow-up now offers attack toggles and considering with typed local
submission/refusal events; only server records supply assessments and outcomes.
See [combat evidence and controlled live checklist](quarm-combat.md).

Quarm casting and the basic spellbook now reuse the checked EQMac paths: gem
casts, item click effects, scribing, memorizing and forgetting. Its own
[`Handle_OP_CastSpell`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_packet.cpp#L2921)
and [`OPMemorizeSpell`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_process.cpp#L1361)
use the same layouts and authoritative replies as TAKP. Casts remain held until
matching replies or interruptions; a silent unacknowledged request allows a
manual retry after five seconds, with no automatic resend. Server class, level,
zone and expansion restrictions remain authoritative. The existing zero cast
CRC remains an inference; the inspected handler does not read that field.

Six additional synthetic Quarm assembly tests cover exact gem/item cast bytes,
stale inventory/gem/target refusal, reply/interruption handling, memorizing and
forgetting without optimistic gem changes, cancellation on standing and scribing
with authoritative scroll consumption. Item click effects use the observed
inventory independently of the separate item-move capability. Moving/deleting
book entries remain disabled. These spell actions have not been
tested on public Quarm; the available live character is not a spell caster.

The combined follow-up also offers the source-backed EQMac ability subset on
Quarm and TAKP, with per-ability eligibility, recovery and server reset handling.
Frenzy is inapplicable in the inspected EQMac dispatcher; fishing, bind wound
and pickpocket remain unfinished. Ordinary autoattack/considering is offered
by the separate source-backed combat owner; live compatibility remains
unverified. See [ability evidence and limits](eqmac-abilities-parity.md).

Quarm inventory uses its wire stack flag independently of TAKP's rule, validates
placement and banker proximity, and keeps item predictions unconfirmed after
the two-second pacing delay. Contradictory corrections block item actions until
a complete snapshot. Coin transfers update a local ledger without fabricating
server acceptance. Banking/refusal tests belong on isolated disposable state;
see [inventory evidence and the deferred matrix](quarm-inventory-parity.md).

Movement now uses explicit ready/unavailable inputs and revision challenges.
Carried load includes generation-specific bag reduction, stack and coin rules;
effective STR includes supported equipment, food, spell effects and EQMac
fatigue. Missing or uncertain inputs withdraw movement, turning and jumping,
while bounded passive descent and stationary heartbeats remain. Corrections
cannot reuse old reports. Weight-preserving ordinary cursor moves can retain a
defensible load without becoming authoritative; weight/STR-changing predictions
wait for recovery. These changes preserve the existing numeric speed formulas.
Unsupported formulas/instruments and additional stat modifiers remain explicit
limits in [movement readiness](movement-input-readiness.md). This is offline
implementation, not new public-server compatibility evidence.

The source check at `c2cd265` covers `Handle_OP_ClientUpdate` (15/16-byte input),
`Handle_OP_Jump` (fatigue), `Handle_OP_Damage` (fall damage), and `zone/zoning.cpp`
(zone change and bind). `SpawnPositionUpdate_Struct` is the same 15-byte packed
layout already used for TAKP. `ENCODE(OP_CharInventory)` uses tagged 360-byte
Mac items. Movement timing and physics are inherited from the existing client
analysis cited in `movement/gate.rs` and `movement/speeds.rs`; the limited public
Quarm checks below do not establish full movement compatibility.

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

P99 offers carried manual combines and the ordinary forge path. The session
requires a matching server open before accepting moves or a combine for that
forge. Live P99 response ordering and custody remain to be verified.
[Manual combine evidence and limits](p99-manual-combines.md)

This table records the baseline above; completed fixes and follow-up checks are
called out separately so a new capability is not mistaken for live verification.

State after the 2026-11-03 stack, for a reader of the table: the Quarm
"Disabled" baseline rows for previews, walking/running, jump/fall, zoning,
death/return to bind, camping/posture, inventory/coin moves (guarded, no
merchants or meals), casting/spellbook (no book edits), targeting
and abilities (the supported subset) are now offered by policy, and P99 jumps
are enabled. P99 previews are capture-checked. Everything else in "What
remains" still stands, and the live evidence for the new rows is only what the
validation sections below record.

The separate combat follow-up additionally offers Quarm normal attack toggles
and considering. Its local request events are not server acknowledgments and
it adds no live combat/death evidence to this report.

| Area | P99 baseline | Quarm baseline | What remains |
| --- | --- | --- | --- |
| Login / continued admission / chat | Exercised previously | Exercised previously | Compatibility is scenario-specific. The follow-up Quarm session decoded extended stamina without a warning; broader scenarios remain open. |
| Character creation | UI, rules, codecs and synthetic tests exist | UI, rules, codecs and own-server source checks exist | Create/reject/refresh/enter on each public server; Quarm account-specific mule limits and live expansion settings need verification. |
| Character deletion | No client action | No client action | Typed request, confirmation, response handling and stale-list safeguards; do not treat this as server-unsupported. |
| Character previews | Disabled by policy | Disabled; level/zone stripped too | Fix 1, followed by public visual checks and broader model/gear coverage. |
| Walking/running/strafe/backward/turn | Offered, earlier scenarios exercised | Disabled although EQMac motion exists | Quarm must receive coherent inventory/coin inputs for carried weight, handle corrections and pair movement with transfer/death handling. A bare capability switch is insufficient. |
| Jump / fall | Falls offered, jump disabled | Both disabled | Fix 2 for P99. Quarm still needs its motion prerequisites, jump/fatigue and damage/death checks. |
| Movement accuracy | Bounded prediction and recent steep-slope fix | EQMac rules available behind policy | Weight reduction, effective STR/buff assumptions, roots, sneak, low HP, levitation and unusual speeds require scenario coverage. Full movement parity is not claimed. |
| Swimming, water/lava, drowning, boats | Bounded swimming uses the existing Titanium position stream. The client-side air meter requires resolved inputs. | Liquid hazard work deferred. | Live P99 wet movement, precise swim tuning, and P99 drowning/lava packet handling remain unverified. Local-only EQEmu offers Titanium liquid damage reports. Other region classes, whole-zone airlessness and boat attachment/control remain incomplete. |
| Zoning / teleports | Zone-line and server-directed handoffs implemented | Transfer feature disabled | Quarm source supports transfers; integrate world re-entry, bind/death requests and refusal recovery. P99 resolves same-zone pads from installed zone boundaries; the entry word and server receipt still need live validation (recording checklist 6.402). Quarm same-zone pad detection remains incomplete. |
| Death / respawn | Server death and offered-bind path implemented | Disabled; richer TAKP path exists | Verify lethal hits, fall/environmental deaths, bleed-out, same-zone bind and reconnect. EQMac client-reported death is held back without complete item-HP accounting; copying TAKP policy does not solve it. |
| Camping / posture | Offered | Disabled | Fix 3; Quarm walking remains separate. |
| Inventory / bank / coin moves | Offered; ground drops are limited to local-only EQEmu. P99 ground dropping awaits attended validation of item serialization and cursor updates | Disabled; EQMac codecs/reducers exist | Quarm needs its own policy and evidence. Shared-bank actions lack desktop paths. |
| Merchants / trade / give | Offered, some scenarios unverified | Disabled | Quotes offered by merchants are a separate missing policy on P99. Two-player trade needs distinct coverage from NPC giving. |
| Casting / spellbook / click effects | Offered; book moving enabled | Disabled | Quarm port of casting and authoritative bar recovery; P99 spell-action landing flag remains unset. Public scenario coverage for resisted casts, interruption, clicks, scribing and memorization. |
| Knight abilities | Lay on Hands and Harm Touch offered | Disabled | A submission starts an inferred 72-minute local estimate (recording checklist 6.399), separate from the received profile. P99 profile recovery and repeat use in one admission still need verification. [Classic daily reuse](https://groups.google.com/g/alt.games.everquest/c/5NdS-bggNek) [P99 player timing report](https://www.project1999.com/forums/archive/index.php/t-221620.html) |
| Warrior disciplines | Learned Evasive and Defensive offered | No activation | Verify P99's post-use timer delivery and active-slot updates on the wire. EQEmu's `Client::UseDiscipline` sends `OP_DisciplineTimer` for a successful start with positive reuse. A missing timer stays unknown. [EQEmu discipline timer](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/effects.cpp#L856-L885) |
| Cooldowns / buffs / resource bars | Partial | Partial | Linked reuse groups, complete effective resource denominators, buff replacement ambiguity and item-effect accounting. Quarm saved gem cooldowns are already decoded from profile offset 4972 and covered by synthetic tests; public behavior still needs verification. Unknown values must remain distinguishable from zero. |
| Target / consider / combat | Offered | Offered by policy (2026-11-03 stack); live unverified | Quarm attack, consider and death coverage still need the attended live check. |
| Abilities | Most offered. Bind Wound requires explicitly targeting yourself. Fishing excluded. | Disabled | Titanium `eqgame.exe` 0x41f4a0 requires a player target and also supports other players. P99 currently offers only the self-targeted case while reply and stock ordering await attended verification (recording checklist 4.5–4.7 and 6.7). A terminal or unanswered attempt releases unrelated inventory actions. Affected stock still needs covering inventory evidence. |
| Loot / corpses / consent | Offered | Manual looting offered by policy (2026-11-03 stack); live unverified | Quarm corpse commands, own-corpse recovery and the NPC loot flow still need the attended live check; no automatic loot-all. |
| Doors / ground objects | Offered | Source-backed ground pickups and receive-only door state on both TAKP/Quarm | Door interaction stays disabled pending ordinary padding/type/state review. Pickups use authoritative inventory delivery; an unanswered pickup holds only further pickups and expires locally after three seconds, a duplicate-suppression bound of ours rather than an official-client timing, and an ambiguous result keeps later waits for their full window; see [protocol evidence and live checklist](eqmac-world-objects.md). A map marker never grants an item. |
| Groups | Disabled | Disabled; TAKP implementation exists | Invitation, refusal, roster changes, leader changes, zoning and disband checks on each. |
| Raid group-leader removal | No policy offers it; generic EQEmu lacks the handler | No verified implementation | Remains a gap unless the specific public server proves it inapplicable. |
| Raids | Disabled; native raid-window restriction reported below | Disabled | Quarm has raid handlers and opcodes: this is a gap, not an established Quarm restriction. |
| Pets | Ordinary orders offered | Disabled | Public verification of orders and charm ownership. Hold, feign, spellhold and focus are withheld on Project 1999. EQMac orders still need wire/action integration. |
| Skill training | Source-backed; public exchange pending | Disabled | One opening per trainer ID and one unresolved practice per admission. Skills follow received updates. Point authority returns on fresh admission. |
| Reading / tradeskills | See individual capability | See individual capability | Book and carried-combine support varies by server. |
| Resurrection / translocate | Source-backed; public exchange pending | Disabled (TAKP translocate exists) | Offer-bound consent. Named translocate continuation and ordinary server-directed movement. Bind continuation without a server-directed destination remains unverified. |
| Sacrifice | Disabled | Disabled | Separate consent path. |
| Who / time of day | Offered | Disabled | Quarm handlers/policy; do not confuse received entity state with a complete `/who` answer. |
| AFK / anonymous / roleplay | Ordinary requests offered from Titanium source | Disabled; TAKP offers anon/roleplay | Check observer listing, server pacing and reconnect state on P99. Submitted and received state are separate. |
| Roll / emote / assist | Ordinary commands offered from Titanium source | Disabled | Check P99 request/reply and a partner's received chat and targets. |
| Show helm / regen ticks | Disabled | Disabled | Per-action codecs and server evidence remain open. |
| Hiding acknowledgement | P99 reply behavior unverified | EQMac ends hiding itself | P99 needs evidence; redundant EQMac acknowledgement is not a missing action. |
| Local map/editor/help/free camera | Available independently of gameplay policy | Same | Target actions still need targeting capability. Existing local UI checks do not certify the whole public-server gameplay matrix. |
| Art/UI completeness | Partial | Partial | Assets come from user installs; model variants, equipment tints, animations and UI coverage remain incomplete. |

### Project 1999 ordinary socials (6.404)

- Assist sends a reliable four-byte spawn ID and the same opcode carries a
  target ID in reply (Titanium `eqgame.exe` 0x46ae6c, 0x46aeab, 0x45f40a).
  One pending request owns the untagged reply. Its five-second selection
  window is an inferred local limit. An expired or superseded request still
  owns its eventual reply. A zero or unavailable target leaves selection
  alone. The session applies a valid selection in receive order and sends
  the ordinary target request for it, with no attack toggle: stock `EQEmu`
  exempts exactly that following target request from its range check
  (`Handle_OP_Assist`, `Handle_OP_TargetCommand`, zone/client_packet.cpp at
  4aceae18), and abilities act on the server's stored target. Without the
  request the server would keep the previous target.
- Dice sends two ordered 32-bit endpoints from 0 through 32,000 (0x4669ab).
  The receive path uses three words and a name at byte 12 (0x4611fd). The
  fixed name-field width follows the shared Titanium layout. Only a received
  result produces a roll.
- Plain emotes use the reliable 1028-byte record and text at byte 4
  (0x498449). The client bounds the leading-space wire string to 510 bytes
  and displays the local line after submission. Incoming emotes use the text
  at byte 4 (0x460d61, 0x454ccd). Language switching and native substitutions
  are outside the plain-text path.
- AFK, anonymous and roleplaying use eight-byte appearance updates with the
  own spawn ID (0x4699a9, 0x472578, 0x46a265), and the receiver consumes
  appearance types 24 and 21 (0x45242f). P99's 500 ms per-kind send spacing
  is local pacing inferred from EQEmu's receiver guard. Public P99 echo
  behavior and observer propagation await attended checks.

Creation slot counts were addressed by the now-merged
[client PR #325](https://github.com/eq-p99-tools/eq-client/pull/325), alongside
AltGr, free-camera settings and script fixes. The integration build includes it.
The steep-slope correction in client #323 is already in the inspected baseline.

The baseline ability codec/recovery gaps above are addressed for the supported
subset by the combined follow-up. The individual remaining exclusions and
live-check prerequisites are recorded in `eqmac-abilities-parity.md`.

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

## Follow-up validation, 2026-11-02

The combined networking code at `d713d0b4` passed 791 tests, formatting, strict
workspace/all-target/all-feature Clippy and warnings-as-errors API docs. The
desktop integration based on `9be679a` passed a native Windows build, formatting,
strict workspace/all-target Clippy and 1,299 tests; 32 ignored asset/live probes
were excluded from that count. Fixtures published here are synthetic.

The subsequent Quarm spell-support change passed all 797 networking tests,
including six new assembly scenarios. This is offline validation; the earlier
public session did not run the newly enabled spell actions.

The combined spell/ability/inventory/movement-input stack passed 833 native
Windows workspace/all-target/all-feature tests, strict Clippy, formatting and
warnings-as-errors API documentation. The combined regression retains all four
Quarm provisions and verifies that inventory pacing never restores a withdrawn
weight grant. The matching desktop validation belongs to its integration PR.

The historical live observations below belong to the earlier revisions recorded
above. They do not validate the later spell, ability, inventory or movement-input
changes. Exact combined heads are recorded in the integration PRs;
the later live batch remains pending.

- Local TAKP using Quarm policy rendered the character-selection preview.
  Further local checks stopped when Docker was shut down.
- Public Quarm rendered the preview and admitted the character. Authoritative
  inventory/coin state, self-targeting, brief forward/backward/lateral motion,
  jump/landing and incoming guild chat were exercised. No movement-proposal
  rejection or stamina decoding warning was logged; HP increased during those
  checks. Local acceptance is not an independent server-side speed measurement
  or proof of position persistence after reconnecting.
- The operator subsequently moved the character to a safer area and closed the
  client. They reported that `/camp desktop` appeared to return to selection.
  The log records a desktop-camp exit and application status 0; the visible
  transition still needs a focused retest. This is an unresolved observation,
  not a reproduced destination-loss defect.

## Live verification queue

Still open: P99 preview/jump in the integrated client; public creation, rejection
and refresh; Quarm NPC target/clear, walking/turning, camp cancellation and the
desktop-exit transition; boundary/teleport/refusal and death/bind paths; reconnect
position persistence and unusual movement states; Quarm gem/item casting,
interruption and spellbook actions on a suitable character. Check one scenario
at a time and respect public account reconnect cooldowns. The operator has
confirmed safe Quarm parking. Existing local EQEmu/TAKP success is useful evidence, but does not
certify public P99/Quarm.

Keep raw captures, session keys, credentials, names, positions and installed
assets private. Publish only synthetic tests, aggregate findings and source
references. Recheck main and overlapping PRs before publishing or integrating.
