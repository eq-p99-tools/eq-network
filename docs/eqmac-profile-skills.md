# EQMac profile skill state

An admitted EQMac player could have a valid class, level and inventory but
an empty Skills window and ability list: the profile decoder explicitly
returned `skills: None`. Capability advertisement did not populate that
missing state. Existing ability tests injected skill vectors after profile
decoding, so they could not catch this admission defect.

The decoder now reads exactly 100 little-endian 16-bit words at byte offsets
2988 through 3187 in the validated 8460-byte profile. It widens the raw words
to the existing `Vec<u32>` without changing values. The following 25 innate
skills are a separate table and are not appended. No level/class-derived
ranks or defaults replace missing data.

Both pinned server sources specify this layout:

- [Quarm Mac profile](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/patches/mac_structs.h#L559)
  and [encoder](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/patches/mac.cpp#L184).
- [TAKP Mac profile](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/common/patches/mac_structs.h#L526)
  and [encoder](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/common/patches/mac.cpp#L104).

The encoder's old comment says "dword", but its destination structure and
the adjacent field offsets establish two-byte entries. Quarm's
[profile builder](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_packet.cpp#L1908)
distinguishes zero (eligible without training), 254 (untrained or not yet
eligible), and 255 (unavailable). Existing EQMac ability guards retain these
distinctions and reject unknown ranks; the decoder preserves unknown values
instead of turning them into usable skills.

Decoding the table also exposes Dual Wield to equipment validation. The
off-hand weapon gate previously accepted any positive value, which would
mistake EQMac 254/255 for training. TAKP and Quarm now share the ability
code's rank interpretation and still require a positive rank. The EQEmu
move policy keeps its existing positive-rank rule: numeric 254/255 are not
globally rejected for other generations. Tests cover all three move
policies, zero/sentinel/unknown values, valid ranks, shields and the existing
two-handed weapon exclusion. Raw profile values remain unchanged.

Normal `OP_SkillUpdate` (`8941`) also now produces the existing skill event.
Both servers send an eight-byte `SkillUpdate_Struct`: `u32 skillId`, then
`u32 value`. Quarm's [sender](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client.cpp#L1888)
and TAKP's [sender](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/zone/client.cpp#L1678)
use the same ordinary update. Unknown IDs/values remain in the event;
existing consumers only update an existing skill/language slot. Short or
oversized records are rejected. The distinct 12-byte silent `SkillUpdate2`
(`be41`) remains unimplemented; no ordinary skill-up notice is invented for it.

Synthetic tests cover the full table, both boundaries, zero and sentinel
preservation, values above one byte, unchanged eligibility restrictions,
malformed profiles, and normal updates through both EQMac policies. The
client follow-up tests an encoded synthetic profile through admission,
ability listing, a decoded update, re-admission and an absent table.
No live profile or capture is a fixture. The private live finding motivated
the fix; corrected public behavior still requires the coordinated rebuild
and retest. The existing Skills window lists positive learned ranks; this
change does not redefine its presentation of zero-rank skills.
