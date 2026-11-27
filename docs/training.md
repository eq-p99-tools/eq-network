# Trainer skills and languages

## Unlisted empty message (6.684)

`0x3464` remains unlisted and unread, including an empty body. Normal gap
reporting retains its opcode, zero length and count. Retained timing places
one arrival after closing Training; timing alone cannot establish a practice
receipt, point spend or trainer-close response. The EQEmu source sends an
empty `OP_GMEndTrainingResponse` from `OPGMEndTraining`, but its Titanium table
does not map that response or identify `0x3464`. This is a hypothesis to
investigate, not an opcode classification. No training state consumes it.

References: [Titanium table](https://github.com/EQEmu/Server/blob/master/utils/patches/patch_Titanium.conf),
[trainer close](https://github.com/EQEmu/Server/blob/master/zone/client_process.cpp).

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
P99 retains its one unresolved practice and fresh-balance policy, even when
a language increase is received. Silence is never a successful practice.
Validate actual point and coin spending against the next received profile;
keep that separate from the projection and the received proficiency.

Pure regressions cover bank encoding/bounds, native race/class boundaries,
current offer/trainee scope, separate update IDs, language caps/unknown
values, timeout uncertainty and P99 pending balance behavior. Runtime and
attended P99 verification are separate checks.
