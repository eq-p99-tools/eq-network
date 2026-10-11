# Source-derived notification coverage

The follow-up [P99 capture validation](capture-validation.md) records observed
layouts, additional implementations and the remaining evidence gaps. Counts
below describe the source-derived pass before that follow-up.

This change extends the **receive codecs**. It does not make every decoded
operation playable. Server policy still controls available commands. Receiving a
stun, charm, illusion, reuse timer or confirmation is separate from maintaining
that state, restricting inputs, rendering it, or answering an offer.

## Reference revisions

The implementations use packet-layout facts from these public revisions, with
independently written Rust and synthetic test records:

- [EQEmu 4aceae18](https://github.com/EQEmu/EQEmu/tree/4aceae18b94ffaafc08e2b17bc41cd72c77f795d)
- [TAKP d8d589a0](https://github.com/EQMacEmu/Server/tree/d8d589a0ce486ee47ec990d3c3bbbf604b2572a1)
- [Quarm 6fc03b5e](https://github.com/SecretsOTheP/EQMacEmu/tree/6fc03b5ea553dab41aeeeadbdea7d54a27d83373)

For Titanium, consult `common/patches/titanium_structs.h`,
`common/eq_packet_structs.h`, `common/patches/titanium.cpp` and
`common/patches/titanium_ops.h`. For EQMac, the ordinary notices below are
passthrough packets described by `common/eq_packet_structs.h`; check
`common/patches/mac_ops.h` and `mac.cpp` before assuming a translation occurs.
Opcode maps are `utils/patches/patch_Titanium.conf` and `patch_Mac.conf`.
TAKP's map prints the EQMac opcode bytes in the opposite order from this
library; Quarm's map already uses this library's order.

P99's implementation is not public. Shared Titanium layouts are candidates for
P99 until a real example agrees. Do not treat emulator source or a successful
synthetic test as proof that P99 sends a particular message.

## Newly decoded opcodes

Numbers below use the library's opcode convention. A dash means this change
does not add that generation's path; it does not assert that the message cannot
exist there.

| Notice | Titanium | EQMac | Body and source writer |
| --- | --- | --- | --- |
| Stun / unstun | `1e51` | `5b41` | 4 bytes, duration **milliseconds**, zero clears; `Client::Stun/UnStun`, `zone/spells.cpp` |
| Charm relationship | `12e5` | `4442` | 12 / 6 bytes, three u32 / u16 fields; attach/release in `zone/spell_effects.cpp` |
| Forced target | `1477` | `fe41` | 4 / 2 bytes; `Client::SendTargetCommand`, `zone/client.cpp`; distinct from local `TargetSent` |
| Discipline timer | `53df` | — | 12 bytes; timer group and remaining **seconds**, `Client::SendDisciplineTimer`, `zone/effects.cpp`, and `SendDisciplineTimers`, `zone/client.cpp` |
| Discipline list | `7180` | — | 100 u32 slots; `Client::SendDisciplineUpdate`, `zone/effects.cpp` |
| Linked spell reuse | `6a00` | — | 12 bytes; timer group, end, start; `Client::SetLinkedSpellReuseTimer`, `zone/spells.cpp` |
| Activated discipline | — | `f241` | 4 bytes, first byte is ID; `Client::UseDiscipline`, `zone/effects.cpp` |
| Translocate exchange | `8258` | `0642` | 92 bytes, destination Y/X/Z at 76/80/84; `Translocate_Struct` and confirmation sender / `Handle_OP_Translocate` |
| Sacrifice exchange | `727a` | `ea41` | 12 bytes; `Client::SacrificeConfirm`, `zone/client.cpp` |
| Weather | `254d` | `3641` | 8-byte type/intensity; `Zone::weatherSend` / `Zone::ChangeWeather`, `zone/zone.cpp`; Titanium also has 12-byte admission form from `Handle_Connect_OP_ZoneEntry`, `zone/client_packet.cpp` |
| Illusion | `448d` | `9140` | 168 / 20 bytes; `Mob::SendIllusionPacket`, `zone/mob.cpp`; Titanium has an explicit patch encoder |
| Roll result | existing | `1640` | Shared 76-byte `RandomReply_Struct`; `Handle_OP_RandomReq`, `zone/client_packet.cpp` |
| Assist result | existing | `0042` | 2-byte entity, **-1 means no target**; `Handle_OP_Assist`, `zone/client_packet.cpp` |
| Wear change | existing | `9240` | 12 bytes with padding at 3 and 6..8; `Mob::SendWearChange`, `zone/mob.cpp` |
| Level update | existing | `9841` | Shared 12-byte layout, 0..330 experience; `Client::SetLevel`, `zone/exp.cpp`; EQMac's previous-level field can be the saved highest level |
| Skill update | existing | `8941` | Shared 8-byte layout; `Client::SetSkill`, `zone/client.cpp`; the separate `SkillUpdate2` format is still pending |
| Book text | existing | `ce40` | Kind byte then terminated text, **no Titanium window byte**; `Client::ReadBook/QuestReadBook`, `zone/client.cpp` |
| Corpse consent | existing | `d540` | Shared 161-byte `ConsentResponse_Struct`; this adds no consent command capability |

There are **25 new zone opcode paths: 10 Titanium and 15 EQMac**. Weather was
already read by admission, so this is not 25 previously unknown protocol
numbers. A malformed recognized packet counts as recognition in the opcode
census, not successful decoding. Opcode counts are not a completeness score.

The same public entry points (`message::titanium`, `message::eqmac`,
`world::titanium_update`, `quarm::updates`) route these to typed events. No
login-server or world-server decoder is changed.

## Subtypes and uncertain semantics

Both generations now retain appearance kinds not otherwise interpreted, including
spawn-zero extension records. Light, sneak, fly-mode and integral size updates
have typed forms. Existing posture, visibility, listing and pet-owner events keep
their existing routes. Unknown kinds/values remain `AppearanceChange::Unknown`;
they must not be applied as a known state change. A fly-mode report is not
authorization to submit uncalibrated flying movement.

Titanium weather's admission type values (0 clear, 0x31 rain, 2 snow) differ from
the eight-byte update (0 rain, 1 snow, zero intensity stops precipitation). The
admission form supplies no intensity. The `Clock` session feature stages its
latest weather report and emits it after admission. P99/EQEmu install this
feature today; EQMac's clock/admission policy is separate work.

Existing `OP_Damage` and `OP_Action` readers now retain their three push fields:
force, heading and pitch. Titanium offsets are 11/15/19 for damage and 10/14/18
for actions; EQMac uses 12/16/20 for both. The Titanium patch explicitly copies
these fields. Non-spell action kinds now emit `WorldEvent::Action`; spell kinds
still emit exactly one `SpellEffect`, with its push and generation-specific
details (including EQMac's signed tap amount). None of these fields applies
movement automatically. Reject non-finite values, preserve finite wire units,
and calibrate their interpretation before changing physics.

Illusion sizes deliberately retain their generation: floating-point for Titanium,
signed integer for EQMac, where -1 selects the racial default. Texture and face
sentinels remain intact. The typed report does not replace a base race or mutate
an entity's cached appearance automatically.

Linked reuse timestamps come from `Timer::GetCurrentTime() / 1000`. They are
**not Unix timestamps or local `Instant`s**. Preserve their order and wrapping
values; a future tracker needs a documented clock/correlation policy. Discipline
timer groups and linked spell groups are not spell gem indexes.

Charm notifications describe an owner/pet relationship; they do not alone prove
whether the local player is under AI control. Translocate offers are not
`ZoneOffer`s. Nothing in these receive codecs sends a confirmation, starts a
transfer, or accepts sacrifice. Unknown commands and completion values survive.

## Validation and integration boundary

`tests/source_notices.rs` exercises all 18 control/reuse/confirmation/weather/
illusion opcode paths through the public message API. It checks every truncated
length, an extra trailing byte, field widths, signed and unknown sentinels,
coordinate order, finite floats, timer units and dialect differences.
`tests/eqmac_notices.rs` covers the seven additional EQMac paths and padding.
All fixture names and records are invented; no captured body is a fixture.
`tests/combat_motion.rs` covers the differing action/damage offsets, non-spell
routing, signed tap amounts and rejection of non-finite push components.

These are additions to the deliberately exhaustive `WorldEvent` API:
`Control`, `Reuse`, `Confirmation`, `Weather`, `Illusion`, `Appearance` and `Action`.
`Damage` and `SpellEffect` gain motion fields; `SpellEffect` also retains action
details. Both now derive `PartialEq` rather than `Eq` because they contain floats.
Downstream exhaustive matches need an explicit handling decision before updating
their dependency. This joins the unreleased breaking changes for the next minor
pre-1.0 release; it is not a compatible 0.1.x patch.

Before claiming playable support, add session/consumer owners for control
restrictions, forced-target reconciliation, discipline/reuse tracking, illusion
application and confirmation prompts. They need admission/reset handling,
monotonic elapsed-time tracking, stale-command checks, and explicit answers bound
to the current session. Receive-side coverage must not silently enable commands.
The existing coverage-table draft PR also needs its reader rows updated after
these codecs land; its all-opcode recognition test will otherwise flag them.

## Next P99 capture

Start at the **server list after account authentication, before pressing Play**.
This includes the world login and fresh transport handshakes needed to derive
session-specific decoding context. Starting only at character select can miss
that context. World login still carries identifying/session data: keep captures
private even though the password exchange was omitted.

First capture a baseline: enter the zone, stand still, move/turn, change posture,
zone once, stand still, and camp normally. Capture all zone ports across handoffs;
do not limit the capture to the world-server port. Mark actions with timestamps
and leave a short idle gap between them. Verify transport context and decryptable
spawns before spending time on a longer session.

Then use short directed sets, with free exploration between them:

1. Inventory/casting: equip/unequip, bag/cursor moves, split a stack, autoinventory,
   cast/interrupt, sit/stand, buff application/fade, read an owned note/book.
2. Appearance/control: sneak/hide ending, a normal illusion or levitation,
   observable stun/recovery, and sense/assist target changes where available.
3. Exchanges: ordinary merchant and trainer use; group invitation
   accept/decline/leave with a willing helper; a normal tradeskill combine.
4. Later, with a suitable character/helper: discipline reuse, charm release and
   translocate offer/decline/accept. Sacrifice and resurrection are optional
   dedicated sessions, not prerequisites for the baseline.

Record scenario and refusal/cancellation cases, not just successful packets.
Audit missing handshakes, sequence gaps, CRC failures and encrypted skips before
interpreting absence. Compare action/subtype counts within recognized opcodes as
well as unknown opcode counts. Publish only independently constructed synthetic
fixtures and sanitized findings, never raw captures, keys, identities or assets.
