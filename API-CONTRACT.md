# API and integration contracts

## Public API evolution

The published 0.1.x line and the repository's unreleased development head are
different compatibility targets. Compare with the latest published API before
releasing; the workspace version alone is not evidence of compatibility.

- Preserve exhaustive command/event enums in the current API. Adding a variant
  can break downstream exhaustive matches and must be treated as an API change.
- Compatible 0.1.x fixes may use a patch release. Accumulated breaking changes
  require the next minor pre-1.0 line, migration notes and consumer validation.
- Do not retrofit `#[non_exhaustive]` as a supposedly compatible fix. Adding it
  is itself a breaking change. Choose it deliberately for extensible public
  boundaries in the next version; keep internal dispatch exhaustive.
- Carry typed payloads, quantities, slots and capabilities. Introduce an ID
  newtype where it prevents meaningful mixing; avoid wrappers without an invariant.

### Next command-envelope revision

Repeated admission/time matches in `GameCommand` are a candidate for the next
breaking API revision, not permission to bypass validation now. The intended
boundary is an admission-scoped envelope containing a typed admission ID, creation
`Instant`, and typed action enum. Character selection has a different lifetime
and stays separate. A host-local request ID cannot create correlation absent from
the wire protocol.

Before implementing it, inventory every command's actual lifetime and expiry
semantics, migrate the client/logger/mobile/proxy consumers, and retain the tests
that require exactly one feature owner and matching capability/resource needs.
Do not replace those compiler-checked matches with a string registry or an
unconstrained macro that hides ownership. This document does not change the
current public command layout.

## Feature ownership and recovery

One feature owns each command and its state. Wire generation determines layout;
server policy determines availability. New features start absent, and a frontend
must honor the session's offered capabilities and explicit player choices.

Resource holds cover the operation's unresolved effects, not merely a UI timer.
Distinguish pending, uncertain, rejected and settled states. Retain the original
operation and relevant identity after an unanswered timeout. A late reply must
not mutate an unrelated replacement slot or release a newer operation's hold.
When the wire cannot distinguish outcomes, invalidate and reconcile rather than
guess, replay, or invent a server request identifier.

Closing a window, dying, zoning and reconnecting need explicit feature reset
semantics. A new admission invalidates old commands and proposals; a UI close
does not by itself prove that a server transaction failed. Tests should cover
late replies after each reset and successful recovery from authoritative state.

Use `Instant` for elapsed time and command age, wall time for log records.
Transport/event callbacks must return promptly. Ordered inventory and lifecycle
events cannot be silently dropped; a callback error is a session failure, not a
cosmetic warning. Hosts need a bounded event bridge with explicit overload
behavior, independent of asset decoding and rendering.

## Error contracts

Keep contextual `anyhow` errors at connection, I/O and executable boundaries.
Use typed refusal/validation reasons where an application needs a behavioral
decision. Applications must not parse a diagnostic sentence to decide whether
to retry or replay an action. Preserve unknown wire values when they are useful
for forward compatibility; reject malformed lengths and invalid domain values.

Do not collapse an unknown outcome into a refusal, or treat transport send
success as authoritative completion. A UI may explain uncertainty without
releasing an inventory hold or resending a non-idempotent action.

## Validation and release integration

This library workspace intentionally does not commit `Cargo.lock`. CI resolves
dependencies for each toolchain: stable format, Clippy, tests and docs on Linux;
native tests on Windows; Rust 1.88 minimum-version compilation. These checks neither load proprietary assets nor connect to a live
game server. A fresh downstream resolution and packaged-crate consumer remain
release checks, because a downstream application owns its dependency lockfile.

For each changed feature, record protocol generation, server policy, repository
revision, capability and scenario. Separate source-derived behavior, synthetic
codec/state-machine tests, and live evidence. Prior Quarm login/chat testing does
not authorize enabling every TAKP feature on Quarm.

Applications using a Git branch still resolve a specific commit in their
`Cargo.lock`. A networking merge is not integrated into an application until its
lockfile changes and that combination passes checks. Coordinate related open PRs
before advancing the pin. Never commit credentials, raw game captures, extracted
assets, or identifying account data as fixtures; construct synthetic records.
