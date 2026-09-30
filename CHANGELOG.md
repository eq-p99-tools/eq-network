# Changelog

This project follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Character-selection sessions and typed world state for graphical clients.
- P99 movement, targeting, doors, inventory operations, spellbook editing,
  casting, item activation, buff notifications, and zone/death handoff events.
- Quarm admission and entity presentation for graphical clients; outbound
  gameplay commands remain P99-only.
- Items on the ground (`objects`): Titanium ground objects are reported, and
  a nearby item can be picked up onto an empty cursor. World containers are
  not supported yet; one that opens for a click is closed again.
- Worn gear (`appearance`): spawns and the player carry their materials,
  tints and facial features from Titanium spawn records, and wear changes
  update them. Quarm reports none yet.
- Synthetic regression coverage for inventory reconciliation, scribe consumption,
  movement admission, cast state, and fresh-key world/zone handoffs.

### Changed

- Separate session helpers validate gameplay requests against current admission
  state and retain server corrections rather than treating predictions as acknowledgments.

### Known limitations

- Movement still requires calibration. Airborne movement (falls and jumps) is
  accepted only on stock EQEmu sessions, with provisional physics, until
  official-client falls and jumps are measured; fall damage is not reported.
  Complete server-specific buff reconciliation is not implemented.
- Latest scribe-consumption reconciliation and fresh-key zoning changes have
  offline regression coverage but still need fresh live verification.

## [0.1.2] - 2026-09-15

### Added

- Included the MIT license text in every published crate archive.

### Changed

- Documented the Project Quarm paths exercised against the live service through
  the Android client.
- Removed homepage metadata that duplicated the source repository URL.

## [0.1.1] - 2026-09-15

### Added

- Cross-links between all four crates in their published READMEs.

### Changed

- Replaced the first-release token fallback with crates.io trusted publishing
  over OIDC.

## [0.1.0] - 2026-09-15

### Added

- Initial transport, login, game-codec, and high-level client crates extracted
  from the Project 1999 proxy and headless logger applications.
- Titanium/P99-V62 and source-derived Windows TAKP/EQMac protocol support.
- Structured inbound chat and item links, outbound chat commands, cancellation,
  reconnect handling, channel filters, and validation assets.

[Unreleased]: https://github.com/eq-p99-tools/eq-network/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/eq-p99-tools/eq-network/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/eq-p99-tools/eq-network/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/eq-p99-tools/eq-network/releases/tag/v0.1.0
