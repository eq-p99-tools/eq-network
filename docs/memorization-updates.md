# Memorization updates

Hosts may set `MemorizeSpell.preparation` to `Some(MemorizationInputs)` with
the installed spell's level for the current class. The worker uses received
character level and Meditate with the Titanium countdown. While the worker
reports `BookActionStatus::Preparing` and `MemorizationProgress`, supply
`PollSpellbook` from ordinary client updates with the monotonic update time
and current admission ID. Stop polling after cancellation or submission.
Headless callers use the same update clock and command.

Progress and request readiness use one 92-point counter. The first eligible
poll steps immediately. Later polls step once after the strict elapsed
threshold; a long stall does not cause a burst of steps. The counter reaching
zero permits one request. It does not confirm a gem change: wait for the
matching server update. Normal posture, movement and book arbitration remain
in force. The explicit fast memorization option still bypasses preparation.

`BardMemorization` accepts verified effective capped INT and DEX and known AA
286 selection state. Base profile attributes are insufficient. Unknown native
inputs, including missing Bard stats, use the existing five-second approximate
timing and mark progress `approximate`. The precise formula is selected only
when its inputs are known. Other dialects use that approximate fallback.
`preparation: None` keeps the legacy worker delay without host polling for
existing callers. Scribing is unaffected.

This is client request timing, not a new server capability or a claim of live
compatibility. No packet layout or server capability offer changes here.
