# EQMac stamina updates

`food::eqmac_nourishment` accepts two packed `OP_Stamina` layouts:

| Offset | Field | Width |
| --- | --- | --- |
| 0 | Food count, signed little-endian | 2 bytes |
| 2 | Water count, signed little-endian | 2 bytes |
| 4 | Fatigue | 1 byte |
| 5 | Quarm extension | 3 bytes, absent on TAKP |

TAKP sends five bytes. Quarm's `Stamina_Struct` contains eight bytes, and
`Client::SendStaminaUpdate` writes the extension as `0`, `1`, and
`is_client_tic`. Its Mac patch has no stamina conversion and defaults to
passing unlisted opcodes through. The extension is not part of either
nourishment count; this decoder does not interpret fatigue or tick state.

Source references:

- [Quarm structure](https://github.com/SecretsOTheP/EQMacEmu/blob/main/common/eq_packet_structs.h)
- [Quarm sender](https://github.com/SecretsOTheP/EQMacEmu/blob/main/zone/client.cpp)
- [Mac patch strategy](https://github.com/SecretsOTheP/EQMacEmu/blob/main/common/patches/mac.cpp)
- [Registered Mac conversions](https://github.com/SecretsOTheP/EQMacEmu/blob/main/common/patches/mac_ops.h)

The regression data is synthetic. The source-derived eight-byte layout
addresses a length rejection observed during a live Quarm session; the
corrected parser still needs a post-fix public-server check.
