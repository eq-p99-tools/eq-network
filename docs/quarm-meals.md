# Quarm meals

Quarm offers the existing EQMac food/drink path through the inventory owner.
This is source-backed support with synthetic regression coverage prepared;
public-server behavior has not been validated for this change.

The inspected public source is
[SecretsOTheP/EQMacEmu at c2cd265](https://github.com/SecretsOTheP/EQMacEmu/tree/c2cd2651045ab063b236b08f3a9e83cbf10289e9):
its [consume handler](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_packet.cpp#L3949)
reads the 16-byte slot/automatic-or-manual/type record, validates the actual
food/drink item, removes one unit without an item update, then sends stamina.
The [packed structure](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/eq_packet_structs.h#L1157)
and opcode table agree with the existing EQMac encoder. Automatic consumption
uses the existing below-3000 threshold; manual consumption retains the existing
6000-fullness guard. No new client fullness calculation is invented.

The same inventory owner keeps the local consumption ledger. A sent command or
stamina report is not an item-delivery acknowledgement. Existing report barriers
prevent duplicate automatic bites. Automatic meals wait during merchant holds,
missing/stale inventory and unresolved movement of the chosen item or its bag;
manual consumption also refuses unknown slots. Unknown state does not become an
empty inventory. By default automatic meals preserve items with modifiers.

The shared meal capability and host setting synchronization are supplied by the
capability integration. This policy change does not enable merchants, exchanges,
new senders, retries or a resynchronization timer. Validate manual food/drink,
automatic plain food, modifier preservation, full refusal and inventory persistence
in a later authorized session with suitable existing consumable fixtures.
