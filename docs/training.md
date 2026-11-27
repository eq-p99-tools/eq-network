# Trainer skills and languages

## Default-ignored message (6.684)

Titanium's native dispatcher 0x45b8f0/0x45f687 sends 0x3464 to its default
return at 0x461757 (read 6.684). The retained inbound body was empty. Its
purpose remains unknown; the session quietly counts it as skipped and never
uses it as a practice, balance, skill or trainer-close acknowledgement.
The EQEmu server's empty OP_GMEndTrainingResponse after Done is a hypothesis
about its origin, not an established name for the opcode.

## Native local practice (6.683)

P99 follows Titanium 0x56b48e: after normal eligibility and successful local
request submission, spend one point and the computed price locally. A skill
already above zero increments locally; a zero skill and a language keep their
value until received news. The first skill press prints 0x2f22; languages use
0x2f22/0x2f23. Ordinary skill increases above zero print nothing at the press.
The received 0x6a93 sets the skill and prints 0x2f3b without another spend.
These are projections, not server receipts. Absolute coin/point updates and a
fresh profile replace them. Failed submission commits no projection. Closing,
reopening, timeout or UI reload neither refunds nor reapplies a spend. No
per-zone practice lock is introduced. Native no-points/coin refusal lines
remain separate from diagnostic bookkeeping, which produces no invented chat.

## Skills and languages

`TrainingSubject` keeps ordinary skills (bank 0) and languages (bank 1)
separate. Legacy `TrainingRequest::Train`/`TrainAt`, `Request::Train` and
`titanium_train` remain ordinary-skill APIs. `PracticeAt` selects a typed
subject from the active trainer offer; `titanium_practice` encodes the
12-byte request with the bank at byte 4 and the bank-local ID at byte 8.
IDs are checked before encoding. Received language updates use ID 100 plus
the language ID, rather than the request's bank-local ID.

The 448-byte trainer reply contains 100 ordinary skill caps and 32 raw
language availability flags at bytes 413..445. Native eqgame 0x452387 passes
those flags to 0x56b897, which admits nonzero entries at a fixed cap of 100.
`TrainingReply::language_flags` preserves the raw bytes; `language_caps` is
projected only for the pending, current trainer and trainee. Zero flags
stay unavailable.

Every server with supported language training uses native Titanium eqgame
0x4bbeda, its 25-by-15 minimum-level table at 0x78f010, and the class exceptions
in that function. These checks filter received availability and are repeated
before practice. Current proficiency does not bypass eligibility. The table
columns are races 1..12, 128, 130 and 330. Language 25 has no native eligibility
entry; unknown races remain unavailable. The ordinary guildmaster class/reach
checks still apply. Unknown language proficiency is not zero, even when the
language is eligible. Native eqgame 0x56b48e separates request
banks; 0x4bdad1 computes the cubic price above proficiency 10. None of these
source reads establishes a P99 runtime result.

Both banks share the existing transaction model. On EQEmu, a matching
received rise permits the existing point/purse projection; a language rise
must be exactly one point and within the observation interval. These
updates contain no balance or price receipt. An unanswered language timeout,
trainer replacement or early trainer close withdraws point authority and,
for a paid request, purse authority. A free
request preserves known coins. A later language update remains proficiency
news and cannot restore points or purse. Further practices need fresh admission.
Timed-out or invalidated unanswered openings retire their trainer ID until
next admission: replies carry no request or incarnation token, so a retry
cannot safely distinguish the outstanding old answer. An already answered
offer simply invalidates on replacement or death; a fresh request and reply
can reopen that trainer in the same admission. Any unresolved language spend
independently keeps point/purse authority unknown until fresh balances.
P99 instead uses the native press-time projection described above; a skill
increase is never interpreted as a spend receipt.
Validate actual point and coin spending against the next received profile;
keep that separate from the projection and the received proficiency.

Pure regressions cover bank encoding/bounds, native race/class boundaries,
current offer/trainee scope, separate update IDs, language caps/unknown
values, timeout uncertainty and native P99 press-time balances. Runtime and
attended P99 verification are separate checks.
