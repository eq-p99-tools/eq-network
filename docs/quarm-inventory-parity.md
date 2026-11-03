# Quarm inventory and personal banking

This change enables item and coin moves through the existing `Belongings`
owner. It keeps the inventory and wallet observations already used by movement.
Quarm has its own placement/confirmation policy and item stackability decoder;
the EQMac record layout, slot translations and outbound codecs remain shared.

The desktop's inventory, cursor, quantity picker, equipment and personal-bank
controls already use `Capability::Inventory`. Its bank availability uses the
same banker predicate, and its status already labels predicted contents. No
client UI patch is needed; the integration dependency pin must include this PR.

This inventory change does not enable merchants, meals, exchanges/giving,
looting, object pickup or speed calculations. It does not implement dropping,
destruction or shared banking. The branch inherits casting, basic spellbook and
abilities from PRs #175 and #176, and preserves their provisions and tests.

## Evidence and behavior

Quarm references below pin **SecretsOTheP/EQMacEmu
`c2cd2651045ab063b236b08f3a9e83cbf10289e9`**. Source behavior is not a claim about
the running public server's revision or configuration. The earlier TAKP path
was cross-checked against [EQMacEmu/Server `047d0f8`, `SwapItem` and
`SwapItemResync`](https://github.com/EQMacEmu/Server/blob/047d0f8/zone/inventory.cpp).
Existing local TAKP inventory research was consulted privately; no capture or
private observation was converted into a public packet fixture.

| Evidence | Implementation consequence |
| --- | --- |
| [`Handle_OP_MoveItem`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_packet.cpp#L6797) requires the exact structure and checks casting and inaccessible bag slots. | Keep the existing one-second intent/session checks and casting/inventory holds. Reject missing/stale inventory, old revisions, invalid slots, orphaned children, cursor-bag access and unsupported actions before encoding. |
| [`MoveItem_Struct`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/patches/mac_structs.h#L384), [`DECODE(OP_MoveItem)`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/patches/mac.cpp#L743), and the [opcode table](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/utils/patches/patch_Mac.conf#L144). | `0x2c41`, three little-endian u32 words: from, to, quantity. EQMac cursor is 0 (library slot 30); bag contents are one lower than the library's Titanium vocabulary. No destroy sentinel is emitted. |
| [`SwapItem` stack handling](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/inventory.cpp#L2048). | Count zero moves/swaps a whole item; a positive count splits or merges stacks. Validate positive observed counts, capacity, source quantity, matching IDs and destination capacity. A merge uses the cursor and an explicit count; whole matching-stack swaps are rejected. |
| [`ItemInstance::IsStackable`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/item_instance.cpp#L174), [`MacItem`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/patches/mac.cpp#L1021) and the [item loader](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/shareddb.cpp#L905). | Quarm uses the wire stack flag at byte 276, where 1 means stackable. TAKP derives stackability from common-item type and positive maximum charges. Quarm still derives capacity (20 or 1) from that latter rule. The shared codec now selects these rules independently. Charges at 278 remain signed for non-stacks. |
| [Stack/GM fields](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/item_data.h#L272) and [`SwapItem` GM restriction](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/inventory.cpp#L1821). | Preserve raw GM flags and typed stack flags, including unknown values, in `ItemPlacement::eqmac`. Unknown flags remain observable but block that item's moves. Restricted GM items cannot be moved by this ordinary-client policy. The server can delete an unauthorized GM item, so this is a pre-send guard. |
| [`ItemInstance::IsSlotAllowed`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/item_instance.cpp#L521). | Reuse the TAKP worn-slot restrictions and existing conservative class/race/deity, two-hand, dual-wield, bag-size and quiver checks. Bags/books cannot be worn; arrows cannot enter the range slot; instruments conflict with an occupied other hand. |
| [`SaveCursor`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/zonedb.cpp#L4443) and [bag-content sends](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/inventory.cpp#L2200). | A hidden queued cursor item may arrive only after the cursor empties. Keep the cursor as an intermediate location, preserve moved container contents and accept server cursor refills/child updates. Serialize item actions with the existing two-second pacing delay. |
| [`SwapItemResync`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/inventory.cpp#L2236). | Refusal sends temporary Copper Coin records, then real roots/removals; some empty slots and bag contents are omitted. Ordinary clients are disconnected. A contradictory replacement invalidates action state until a complete snapshot, even if the delay expires or both roots later arrive. Do not retry the action. |
| [`OPMoveCoin`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_process.cpp#L1449) and [`MoveCoin_Struct`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/common/eq_packet_structs.h#L1186). | `0x2d41`, 20 bytes: from/to places, from/to denomination and signed amount. Places 0/1/2 are cursor/purse/personal bank; kinds 0/1/2/3 are copper/silver/gold/platinum. Whole-coin conversion retains the remainder (11 gold to platinum takes 10 gold). Validate balances, known destinations and signed destination capacity. No trade path is enabled. |
| [Item banking check](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/inventory.cpp#L1524), [coin banking check](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/client_process.cpp#L1525), [`GetClosestBanker`](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/zone/entity.cpp#L4531). | The server checks a class-40 NPC and distance for each move; these handlers have no bank-open handshake. The library recomputes visible banker proximity within **20 units in 3D**, with finite coordinates, before every bank item or coin action. This is a conservative client limit, not a measured public-server limit. The session also rejects dead, zoning or stale-admission actions. |

## Confirmation limits

Successful item moves and ordinary coin transfers receive no positive
acknowledgement in this source. Transport delivery is not game acceptance.
Quarm therefore **never emits `InventoryUpdate::Settled` from the timer**:
`predicted()` and prediction origins survive until covering server updates or a
snapshot. The two-second delay only limits overlap and gives cursor refills time
to arrive; it cannot prove that an unseen queued item or a late refusal does not
exist. Timing under delay/loss remains a live-test item.

Coin events after sending describe the local ledger. Authoritative purse/profile
updates replace it; there is no fabricated coin-acceptance reply. A banker that
moves or disappears after the last observation can still cause a silent refusal
and ledger drift. Do not automatically resend uncertain transfers. Re-admission
and comparison with authoritative balances are the recovery/verification path.

The old EQEmu/TAKP timeout behavior is unchanged. This PR's new conservative
policy is specific to Quarm. An unexpected replacement can leave Quarm item
actions unavailable until re-admission even if a GM account is not disconnected.

## Offline validation

Synthetic tests cover exact move bytes, root/bag/cursor/equipment routes, filled
bags through bank roots, splits/merges, all coin kinds and conversion remainders,
signed overflow, unknown balances, stale revisions/sessions/timestamps, missing
and invalidated inventory, invalid/cursor-bag/shared/trade/world slots, GM and
unknown stack flags, delayed corrections, incomplete bag resync, full-snapshot
recovery, cursor refill and matching updates that confirm only covered slots.

Quarm assembly tests exercise the actual policy and decoder. Independent
allowance tests cover all 16 combinations of items/coins/merchants/meals; the
Quarm assembly remains silent on meal/merchant/exchange commands and automatic
nourishment. Banker absence, wrong class, hidden state, nonfinite/out-of-range
position, boundary distance, despawn and admission/death/zoning gates are checked
at the library boundary.

Validation results are recorded in the PR. No public-server action, GUI,
emulator or Docker operation was performed for this assignment. Tests and
published records contain synthetic data only.

## Batched live-check handoff

These cases belong to the later dedicated live-check chat, after implementation
agents finish. They are not authorization to start that work here. Use a
disposable **non-GM** character on an isolated server running the pinned Quarm
source for banking and refusal cases; TAKP alone cannot validate the Quarm stack
flag difference. Keep all captures, names, positions and credentials private.
No deletion/destruction test is requested. Never run the banking/refusal cases
against a public character under this assignment.

| Server / prerequisites | Steps and expected observation | Risk / cleanup |
| --- | --- | --- |
| Isolated Quarm; inexpensive plain item, stack, two-slot bag, compatible equipment; full snapshot. | Pick up, split, merge, equip/unequip and move a filled bag. Wait between actions. Expect exact counts/locations and no resync warning; reconnect and compare authoritative contents. | Stop on the first contradiction/disconnect. Keep a baseline and restore ordinary item locations after a successful comparison. |
| Isolated Quarm; server-controlled queued cursor item; optional controlled delay. | Empty the visible cursor, observe the next item arriving, then place it. Exercise a delay beyond two seconds. Expect no duplicated/missing item; predictions stay labelled until authoritative refresh. | The unacknowledged refill race needs measurement. Use disposable items; stop and reconnect on mismatch, never retry blindly. |
| Isolated Quarm; nearby banker, small known balances in every denomination. | Deposit/withdraw items, filled bags and small coins; convert 11 gold to platinum. Repeat after leaving range/banker despawn. Expect valid operations to persist, invalid requests to send nothing, and one gold remainder. | Banking is local only. Check balances on reconnect and restore the baseline. A silent server refusal requires reconciliation, not a resend. |
| Isolated Quarm; server-controlled refusal/correction fixture. | Inject/force resync without submitting malformed public actions. Observe temporary roots, incomplete bag correction and disconnect. Expect all further item moves blocked until a full snapshot. | GM accounts mask the normal disconnect. Use status 0 and disposable state; reconnect and inspect before continuing. |
| Public Quarm, only in the later authorized batch; safe location, cheap carried item/stack and known baseline. | Verify current flag/charge decoding and a minimal carried cursor round-trip; reconnect and compare. Confirm merchants/meals/exchanges remain off at this branch's scope. | Public build/configuration remains unverified. No banking, destruction or deliberate refusal. Stop on a warning; preserve private evidence for the coordinator. |

Integration notes: `MoveRules::Quarm`, `ItemPlacement::eqmac` and the
`EqMac::{Takp, Quarm}` wire variants are the new types/selection points. Full
external `ItemPlacement` literals need `eqmac: None` (or `..Default::default()`).
The desktop's existing constructors already use defaults. Combined-stack
capability assertions must retain independent casting/ability additions.
