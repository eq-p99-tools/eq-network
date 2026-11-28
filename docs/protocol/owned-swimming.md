# Reset-anchored horizontal swimming

`MotionSession::send_swim_sample` and the optional process-local `SwimFrame`
command validate complete ordinary swim controls. Existing clients use the
legacy movement path until they can establish the matching real reset and
actual ordered controls. A receipt alone proves neither. This is not an
ordinary water-entry fix. Existing caps, report gate and wire format remain.

The envelope requires existing swimming and absolute-heading snapshot
permissions, a calibrated wet model, and no root, hold, flight or carrier
ownership. No capability is enabled by this API. Liquid and input observations
must come from the scene/controller owners, not a reconstructed path.

The first opaque `SwimBasis` comes only from admission or genuine correction.
The client must match that physical reset, including zero horizontal wet
velocity and clock. A local pad preserves velocity and is not a reset. Legacy
motion, clipping, incomplete input, uncertain entry/ownership, grant changes
or withdrawal invalidate continuity. Only another independently matched real
reset can establish it again.

Each `SwimControl` records actual input facing, water kind, ordinary digital
input and an ordered ending cursor. Rates always come from calibration,
including backward and walking strafe. Full selected horizontal input is
independent of pitch's vertical contribution. Callers supply no velocity,
phase, fitted arc, arbitrary rate multiplier or reset assertion. Unsupported
input keeps the old guard. Vertical movement retains independent cap checks.

`SwimControls::new` requires complete wall-span observations and matching
final input/pose heading. `from_applied` instead records exact positive f32
durations at the real native calls and checks final pose independently. Its
shape is full fixed caller slices followed by at most one remainder, at most
64 pieces and 250 ms. Scheduling gaps add no fabricated idle physics. Cursor
rounding allowance is at most 32 ns; geometry tolerance remains 0.01. The request
still obeys wall freshness, cadence, calibrated elapsed budget and all caps.

The owner clones accepted horizontal velocity and raw clock, runs unchanged
native recurrence and integrates each actual call. Native `due += frame`
can leave tiny known overdue debt after a final remainder while the public
nominal `frame_phase` view is unavailable. Complete owned applied history
retains that raw state in a finite domain of at most two 10 us boundary windows
of debt. Intervening full calls must finish canonical; only a final remainder
can leave bounded debt. Arbitrary tiny-call tapes decline. Public nominal
phase behavior and legacy observations are unchanged. Unknown state never
becomes a guessed phase.

Only a matched endpoint replay replaces the old ending-heading backward
projection test. A producer that observes a frame longer than 150 ms can hold
its physical scene still. On the next applied frame it records both boundaries
of that pause. The worker matches the start to the preceding observation and
checks the resumed frame duration against its real endpoint. No simulation
steps or displacement accrue during the pause.

The replay uses the ordinary
total/vertical/time/session/availability/transport and
report checks. Unqualified input or unexplained travel uses existing
checks and invalidates continuity, even if the old guard admits it. There is
no retry with a more permissive rule. Trial commits only on matching local
acceptance; failed send does not commit. A held accepted sample can commit
under the existing gate. Neither local acceptance nor queued receipt is a
packet submission or server acknowledgement.

`SwimFrame` follows normal request ownership/lifecycle and the common position
sender and successful-submit counter. `WorldEvent::SwimState` queues actual
position and opaque process-local basis after the originating correction or
local result. Basis and timestamp are skipped in serialization. A producer
must retain reset/input/result chronology, restore its trial on refusal or
discard, and prevent stale results restoring a corrected epoch.

At fixed heading, fresh backward input stays at calibrated B, including zero.
For normal water, `b_next <= .95*b + .05*B`, so starting inside B stays inside.
Earned turn carry may temporarily project farther backward but fixed-heading
input cannot renew it. Native integrated coast is neither a truncated history
nor endpoint velocity times elapsed. Other sets retain their coefficients.

Coefficients follow the Titanium read at `eqgame.exe` 0x4bfe0f. The unchanged
60 Hz clock is this client's inferred model, not a measured native frame rate.
For fully submerged Titanium movement with a matching reset, a host can
supply an immutable body and collision snapshot through `SwimSceneHandle`.
The scene-backed path checks the floor, swept motion, water kind and clearance
for a vertical floor correction using the same body. An unknown collision
query declines the trial. A new scene needs its own matching reset. Ordinary
land or fall entry and surface transitions remain outside this replay path.
See the [scene interface](../../crates/eq-network-game/src/movement/swim_scene.rs)
for the required queries.

Synthetic regressions use the real recurrence and distinguish physical path,
accepted report chords and net endpoint. They are not reconstructions of live
aggregate rows. Removing validator loss does not imply fast turns travel the
same distance as straight input. `inspect_swim_replay` is read-only trial
diagnostics, not admission, commit, send or proof that all other guards pass.
