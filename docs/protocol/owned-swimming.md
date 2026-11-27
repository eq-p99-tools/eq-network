# Reset-anchored horizontal swimming (inert API)

`MotionSession::send_swim_sample` is an optional validation seam. No existing
session command or client calls it. It is not an ordinary water-entry fix.
Existing `send_move_sample` behavior, caps, report gate and wire format remain
unchanged. A later client integration must prove the matching reset and actual
complete controls before opting in; obtaining a receipt alone proves neither.

The first `SwimBasis` comes only from admission or a genuine server correction.
The client must match that same physical reset, including its zero horizontal
wet velocity and frame phase. A local pad is not that reset: it preserves the
client's wet state. After legacy motion, clipping, incomplete input, uncertain
entry/ownership, a grant change or explicit withdrawal, no new anchor is inferred.
Only another independently matched real reset can begin a new proved sequence.

`SwimControls` contains the opaque basis and a bounded, strictly ordered sequence
of actual `SwimControl` pieces. Each piece gives its ending time, canonical body
heading, existing water kind, normal input mode and horizontal share. The owner
derives all rates from calibration, including separately calibrated backward
input and the walking share of strafe. Forward plus strafe uses the ordinary
normalized forward/walk water rate; unsupported input combinations keep the old
path. The caller cannot supply velocity, phase, rate, a reset flag or a fitted arc.

The controls must cover the entire reset/accepted-state-to-request span, ending
at the request's heading, within 250 ms. Only observed idle pieces can precede
the initial calibration timestamp. That prefix earns no horizontal acceleration;
the existing guard still applies its unchanged calibrated interval budget. A
gap, partial declaration, unknown clock or stale basis cannot earn replay credit.
`withdraw_swim_controls` explicitly makes continuity unavailable.

The owner clones its accepted horizontal velocity and phase, runs the existing
water recurrence and integrates travel over each physical piece. It compares
the endpoint with that result and retains every existing total-horizontal,
vertical, elapsed, availability, session, transport and report-gate check. Only
that proved replay replaces the old ending-heading backward-displacement test.
Unqualified input or an unexplained endpoint runs the original checks and
invalidates continuity, even if the old path admits the endpoint. There is no
retry with a more permissive rule. A rejected replay or failed send cannot commit
its trial. A locally accepted held sample does commit, under the existing gate;
this is not a wire or server acknowledgement.

At a fixed heading, fresh backward input is at most its existing rate B (zero
without a grant). In normal water, `b_next <= .95*b + .05*B`, so starting inside
B stays inside B. An earned turn may carry an independently known larger
projection temporarily, but fixed-heading input cannot renew it. The model
integrates that finite coast; it neither truncates history at 90 frames nor
uses endpoint velocity times elapsed as the travel allowance. Other water sets
use their existing coefficients and caps, not a new universal damping rule.

The step and coefficients are read-backed for Titanium (`eqgame.exe` 0x4bfe0f);
the unchanged 60 Hz clock is this client's inferred model, not a newly proven
native frame rate. The ordinary collision path keeps horizontal velocity while
shortening travel, so this first envelope declines clipping instead of treating
achieved travel divided by time as an instantaneous seed. Ordinary land/fall
entry, shoreline/surface ownership and unmatched reset/phase remain on the old
guard until separately proved. They can still exhibit the original refusals.

Tests use synthetic controls and the real wet recurrence. They distinguish the
physical travelled path, the sum of report-interval chords, and net endpoint
displacement. Constant-turn examples are labelled synthetic angular conditions;
they are not reconstructions of live aggregate distances or refusal counts.
Removing validator loss does not imply making fast-turn path length equal
straight travel, or normalizing speed to meet a target distance.
