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
- Handing items to NPCs (`exchange`): `OfferTrade` asks a character within
  reach while the player holds an item, the NPC's answer opens the give
  window, items go into its four trade slots from the cursor, and
  `AcceptTrade` (Give) or `CancelTrade` ends it. The trade slots
  (`InventorySlot::is_trade`) take only what servers accept, and empty when
  the window closes. `Capability::Giving` reports it; coins and trades
  between players are not supported yet.
- Synthetic regression coverage for inventory reconciliation, scribe consumption,
  movement admission, cast state, and fresh-key world/zone handoffs.

### Changed

- `WorldEvent`, `GameCommand` and `ClientEvent` are exhaustive, so a front end
  handles every kind of news and command instead of ignoring new ones in a
  wildcard arm. `GameCommand::capability` names what each command needs of the
  zone session, and `Capability::ALL` lists every capability.
- Commands that arrive while the player is dead or between zones are refused
  through the result each one waits for, instead of being dropped unanswered.
- A command's session and age are checked once, by the zone session, before
  any feature takes it: `Inventory::submit_move` and
  `Inventory::prepare_item_cast` no longer take the session ID or the time,
  and `MotionSession::calibrate_fresh` checks only that a calibration postdates
  the latest reset.
- `InventorySlot` names its ranges (`CURSOR`, `is_equipment`, `is_pack`,
  `is_carried`, `is_in_cursor_bag`), so consumers stop spelling slot numbers.
- Inside the zone session each piece of shared state has one writer: the
  inventory feature for the inventory, the character feature for the server's
  news about the player (now including the gems), the player's stance for every
  sit, stand and crouch, and the world for the session's end and for stopping
  and restarting movement around death and transfers.
- Separate session helpers validate gameplay requests against current admission
  state and retain server corrections rather than treating predictions as acknowledgments.
- Server types (`client::servers`): each difference between servers that speak
  the same protocol is a feature a server type has, absent unless it says
  otherwise, so a new server type starts with every feature off. P99's V62
  protection and 256-unit saved headings, and `EQEmu`'s falls, jumps and
  post-creation start choice, moved behind it; nothing on the wire changed.
- Zone features (`client::session`): the zone session is a set of features
  behind one interface (doors, ground objects, camping and zone transfers so
  far). Each hears every host command, which exactly one of them carries out,
  and every message read from the zone; commands are checked for freshness
  in one place.
- `eq-network-game`: `message::titanium` reads a zone packet into `Message`s
  once for the whole session. Encoders return whole packets
  (`EncodedCommand`): `Doors::click_packet`, `Objects::pickup_packet`,
  `ContainerView::close_packet`, `ZoneOffer::response`, and the new
  `command::titanium_camp` and `titanium_logout`. `GameCommand::session_id`
  and `created` say which admission a command names and when it was made.

### Fixed

- Combined transport packets (`OP_Combined`) give every part a one-byte
  length, as `EQEmu` does, so a part of exactly 255 bytes no longer ends the
  session with "invalid combined length". `build_combined` refuses parts
  longer than 255 bytes instead of writing a length servers misread, and
  `OP_AppCombined` accepts four-byte lengths.

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
