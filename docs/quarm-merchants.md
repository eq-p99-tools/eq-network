# Bounded Quarm merchants

Quarm offers ordinary merchant browsing, purchases and sales through the same
inventory owner as item moves. These changes have source and synthetic-test
coverage prepared; their live-server behavior remains unverified.

The inspected public source is
[SecretsOTheP/EQMacEmu c2cd265](https://github.com/SecretsOTheP/EQMacEmu/tree/c2cd2651045ab063b236b08f3a9e83cbf10289e9).
Its [Mac adapters](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/patches/mac.cpp#L790)
and [structures](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/patches/mac_structs.h#L399)
match the existing EQMac request/buy/sell/end codecs. The
[stock builder](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_process.cpp#L965)
lists base prices before the opening rate; the existing BeforeRate model bounds
purchase cost against known purse coins. Stock decoding now uses Quarm's own
stack flag, keeping the legacy TAKP list API unchanged.

The [buy/sell handlers](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_packet.cpp#L8268)
retain authority over range, faction, race, level, lore and special-server rules.
Quarm purchases are deliberately bounded to known decoded quantities: one
nonstack/charged item, or up to the established stack limit (at most twenty).
They require a known empty cursor and general slot to avoid the server's
full-inventory ground-drop fallback. The guard does not attempt arbitrary
auto-placement or infer free space through unresolved moves. Sales require
established carried placement, quantity and pricing; NO DROP remains refused.

Submission does not establish a purchase, item delivery or new holdings.
Inventory updates establish purchased items independently of the purchase echo.
Sale echoes are reconciled against the remembered item. A zero-quantity refusal
releases the matching trade-kind hold without removing or invalidating items;
remembered sales remain available for conservative late-echo handling. An
unanswered timeout releases the local hold, supplies no success and never retries.

Special reimbursement/recharge merchants and arbitrary multi-item nonstack buys
are outside this bounded support. Price rounding, limited stock, full inventory,
charged items and late/refused responses need scenario-specific validation.
Merchants do not imply exchanges, giving or merchant-offer queries. Source and
synthetic results must not be presented as live compatibility. Future tests need
a legitimate merchant, known coins and a cheap ordinary item; no public refusal,
item destruction or GM fixture is required.
