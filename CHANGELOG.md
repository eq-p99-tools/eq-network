# Changelog

This project follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Breaking:** movement is no longer stopped while the player's speeds
  are worked out again. The program reads its speed modifier live and stops
  input only for a stun, a freeze (posture 0x66) or a posture other than
  standing or ducking (read R7, 0x4a1d74). A buff on the player (each Bard
  song pulse), a buff list, a level, a spell landing, an EqMac fatigue change,
  a change in belongings and a server's correction now ask the host for new
  inputs while the grant in force goes on: no `MotionState` withdrawal and no
  stop sent. A correction still moves the player and stops them there, then
  grants the speeds in force again at once. `SpeedInputState` gains
  `Conservative { inputs, missing }`: the slowest inputs still possible while
  `missing` is unproven, taken only while a grant is in force (recording
  checklist 6.451; without a grant they withdraw as `Unavailable`). An
  `Unavailable` answer, or an answer later than 1 s, holds the grant in force
  at the walk rate instead of withdrawing it (6.452). A report under a
  pending ground drop's weight bound is taken at the bound rather than
  withdrawing. Root, stun, freeze, death and zone transfer still stop
  movement as before. The same rules apply on every server type; only the
  waits are removed. Both sides of 6.451: a grant kept across the host's
  round trip may briefly run at the old, faster rate after a snare lands;
  conservative inputs run slower than the program until the exact ones land.

- **Breaking:** Titanium lifts. `doors::titanium` holds the program's door
  rules: vertical travel and speeds by open type, the type 15 and 78-81
  slides, floor and wall solidity, the client's own return times, and the
  +0x66 field (the param's low 16 bits). `Lifts` steps each vertical door
  from the received updates, and the session keeps one for the zone.
  `MovementRequest` gains `support`, a non-exhaustive `Support`
  (`Support::Door` today) set with `MovementRequest::supported_by`; while a
  sample names the same floor door, the rider may rise in all as far as it
  has risen since they named it (`MovementGuard::lift_since`,
  `MovementGuard::allow_lift_rise`, `MotionSession::allow_lift_rise`), and
  only a sample naming a support spends any of it. `Lifts::returns_at` and
  `Lifts::sinks` give a vertical door's own return and descent. Types 40 and 45 return by a click with
  no player, `Request::ReturnDoor`. A door's click reach runs in 3-D to its
  origin where it stands now (`Doors::check_click_raised`), within 20. The
  program measures from the clicked point on the door as drawn to the
  player, within the player's size and 20 (0x4906c5, 0x435b55, 0x644368;
  read R16b, R16e), so ours is short of it where a part of the model is
  nearer than the origin, and by the size (checklist 6.436). `WorldEvent::Entered` gains `zone_id`
  (serde default 0), the zone number the program's door rules name. The
  lift rider radius and a hinged door's return start are inferred
  (recording checklist 6.424, 6.426).

- **Breaking:** Titanium backs up in water, unless ducking or sneaking, only
  by the buffs' speed modifier (0x4a2bd8-0x4a2c06), so not at all without
  one; a slow that would go the other way creeps at 0.01, and -1000 or less
  roots. `MotionCalibration` gains `swim_backward_units_per_second` and
  `WorldEvent::MotionState` the same field (both serde default None, which
  keeps the old rule for old streams); the session always fills it on
  Titanium, and the guard bounds a swim stroke's component behind its
  heading by it. `EQMac`'s rule is unread and keeps backing up in water as
  any swim stroke.
- **Breaking:** Titanium strafes on the selected base speed: the run speed
  running, the walk speed walking, times the health and weight multiplier
  and halved when slowed, as its program does (the R5 audit's S1). The
  strafe calibration's rate is the running one, which bounds a strafe;
  `StrafeCalibration` gains `walk_units_per_second` and
  `WorldEvent::MotionState` gains `strafe_walk_units_per_second` (both serde
  default None) for the walking rate. Sneaking and `EQMac` keep the walk
  speed's rule.
- **Breaking:** A hurt or burdened player's jump starts at the program's
  health and weight factor times the jump speed, as Titanium launches its
  jumps (0x4193c5). `Speeds` gains `jump_factor`, the multiplier that also
  slows running; `WorldEvent::MotionState` carries it as `jump_factor`
  (serde default 1), and the movement guard bounds an announced jump's rise
  by the factor it started with (`MovementGuard::set_jump_factor`,
  `MotionSession::set_jump_factor`). That `EQMac`'s jump takes the factor
  too is inferred (recording checklist 6.414).
- **Breaking:** Add friends who, guild requests, splitting and a banker's
  coin change on the Titanium wire, offered on `EQEmu` and Project 1999:
  `Capability::FriendsWho`, `GuildRoster`, `GuildManagement`,
  `BankerChange` and `Splitting` (`Capability::ALL` grows to 62).
  `GameCommand` gains `WhoFriends`, `ChangeCoins`, `SplitCoins`,
  `SetAutoSplit`, `RequestGuildMotd`, `AnswerGuildInvite`, `GuildInvite`,
  `GuildRemove`, `SetGuildMotd`, `GuildMakeLeader` and `GuildDemote`, and
  `Request` the matching variants; guild invitations are answered by
  inviter with `guild::InviteAnswer`, whose wire values follow the Titanium
  program (read R11). `WorldEvent::CoinsChanged` carries a
  banker's answer, `WorldEvent::GuildRefused` a refused guild request,
  `WorldEvent::AutoSplit` the profile's automatic splitting and
  `WorldEvent::AutoSplitSet` a change of it, and
  `WorldEvent::CoinsRefused` gains `string_id`. The asked-for message of the
  day decodes as `ChannelName::GuildMotdReply`. Layouts and checks follow
  `EQEmu`'s handlers and the Titanium program (read R6); the split and
  change answer waits and the guild member-name rule are inferred
  (recording checklist 6.410, 6.411). A waiting invitation and the member
  list do not outlive the zone connection. Exhaustive command, capability
  and event matches need the new variants.

- **Breaking:** Add `MovementMode::Shoreline` for bounded Titanium shoreline
  transitions. Hosts submit a frame trace from local collision, and the worker
  replays it to bound the elapsed time and motion before sending ordinary
  position updates. As for ordinary swimming, the host's water and ceiling
  flags are trusted. Each frame carries at most `SHORELINE_FRAME_MILLIS`
  (50 ms); that bound, the 1.5 s arc lifetime and the per-frame damping are
  inferred (recording checklist 6.407). Exhaustive matches on movement mode
  need the new variant.

- **Breaking:** Offer ordinary assist, dice, emotes and AFK/anonymity/roleplay
  controls on Project 1999 from Titanium source. Handle
  `WorldEvent::ListingRequested` separately from received listing state.
  `Message::AssistSelection` is a new variant. An accepted P99 assist reply
  sends the ordinary target request, so the server's stored target follows
  the selection. Public P99 social exchanges still await attended validation.

- **Breaking:** Resolve P99 same-zone pads against installed boundaries
  and current zone points. `BoundaryRoute::SameZone` carries a resolved
  route, `ClientConfig` accepts an installed-boundary provider and
  `WorldEvent::LocalPadCrossed` reports the local placement.
- **Breaking:** Add Titanium server dialogs and player inspection. Hosts can
  respond to a current popup, inspect a current player and configure an
  admission-scoped inspect note. Exhaustive command, capability and event
  matches need to handle the new variants.
- Offer bounded swimming on Project 1999 through the existing Titanium
  position stream. Local movement validation, packet pacing and received
  server placements retain their separate roles. Live wet movement and
  environmental damage handling remain to be verified.
- **Breaking:** Offer ordinary forge interaction and self-targeted Bind Wound
  on Project 1999.
  `GameCommand::CloseContainer` gains `drop_id` and `created`,
  `InventoryUpdate::BandageStockPending`, `BandageStockUncertain` and four
  `BindWoundUpdate` variants are new. `BindWoundUpdate::Unconfirmed` carries
  `stock_uncertain`. `InventoryUpdate::WorldDetached` retires a displaced
  station view. `ItemPlacement` gains `titanium_link_hash`.
  A terminal or unanswered request releases other inventory actions while
  affected bandage slots still await received inventory evidence.
  A forge candidate needs a matching server open before it can take items.
  Station placement, close and combine resets do not grant inventory items.
- Build ordinary Titanium item links from checked item records.
  Evolving records, items with installed augments and labels outside ASCII
  need a received link.
- Stamp world container closes with the object ID and intent time. Decode
  whole-container clears separately from the native one-slot form.
- Compose ordinary chat with genuine received item-link metadata for the
  selected client dialect. The helper rejects quest links and malformed data.
- **Breaking:** Report same-zone server relocation separately from normal position updates.
  A same-zone boundary on local-only EQEmu releases the crossing hold and
  lets normal movement reach a server-owned trigger. The landing is applied
  only on receipt of the server's move instruction. Exhaustive world event
  matches need to handle these two outcomes.

- **Breaking:** Offer the learned, self-targeted Titanium Warrior disciplines Evasive and
  Defensive. Stage received discipline and linked spell timers during
  admission, and use the shared discipline timer and active buff slot to
  guard both actions. Exact empty replacements of the discipline slot clear
  its active state. `BuffUpdate` gains `replaces_slot`. Staged active state
  is published as `WorldEvent::DisciplineAdmission` without advancing the
  buff-slot revision. Downstream exhaustive matches need to include the new
  command, capability and event.

- **Breaking:** Added guarded Titanium knight ability casts and the
  admission's received ability-slot recovery, including the server's
  alternate Harm Touch rank. The local post-send estimate permits a later
  attempt during the same admission. A received profile replaces the estimate.
  `WorldEvent::KnightReuse` distinguishes the two sources. Downstream
  exhaustive matches need to include the new command, capability and event.
- Offer ordinary Project 1999 resurrection and translocate consent using
  admission-scoped offer identities and Titanium's five-minute timeout.
  A named translocate completion can start a move only after a matching
  Yes. Byte-identical prompts already issued in the same P99 admission
  cannot authorize another answer. Received movement and recovery remain
  authoritative.
- **Breaking:** Use a two-record beneficial-buff cancellation sequence on
  Project 1999 (its source is not yet cited; recording checklist 6.396). Keep the selected slot until received updates change it, report a
  partially submitted pair separately, and never resend automatically.
  Consumers matching `buffs::cancel::Progress` exhaustively must handle the
  new `PartiallySubmitted` variant.
- **Breaking:** Offer skill training on Project 1999 through an addressed
  trainer reply. Titanium opcode `0x238f` now decodes to
  `Message::TrainingReply` instead of a training-offered world event.
  Each trainer ID gets one opening request per admission. A submitted
  practice withdraws the point balance until a fresh admission. Received
  skill changes remain authoritative and carry no receipt for the spend.
  The displayed price is an estimate. Paid requests invalidate the purse
  projection until a fresh balance arrives.
  Consumers matching training requests, training updates or decoded messages
  exhaustively must handle the new variants.

- `Client::titanium_chat_link_expansion` exposes how the selected server's
  chat handler grows Titanium item links before its own buffer limit, as
  `chat::LinkExpansion`. Only stock EQEmu reports one.
- `Client::consider_npcs_by_level` exposes the selected server's NPC
  consider presentation policy. All current policies use the received
  color. The consider reply documents the faction code swaps in EQEmu
  and EQMacEmu.
- `buffs::scale_instrument_effect` provides the host's bounded Titanium
  instrument scaling for movement-related buff magnitudes. The existing
  neutral values and unavailable modifiers keep their meaning.

- **Breaking:** Submit an ordinary Titanium cursor drop on local-only EQEmu through
  `Capability::GroundDropping` and `GameCommand::DropItem`. Check the
  received droppable instance, actor and inventory revision before sending.
  `InventoryUpdate::GroundDropPending` holds custody until a received
  inventory snapshot. Ground objects use their
  existing independent update and pickup paths. Downstream exhaustive
  matches and `ItemPlacement` literals need migration. P99 drop support
  awaits validation.
- Offer Titanium drowning and lava heat reports on local-only EQEmu sessions
  through `Capability::Drowning` and `Capability::LavaHeat`. Retain the
  profile's zone, base fire resistance and AA presence and the Titanium
  item's signed fire resistance. Require a fresh host sample of the current
  worker inputs and position before sending the existing environmental record.
  The server owns health and death. Deployed Project 1999 liquid reporting
  awaits validation. The liquid record layout is inferred (recording
  checklist 6.392). The base fire resistance is the profile dword at byte
  12876, which the Titanium program copies at `eqgame.exe` 0x4b9f3c and uses
  at 0x418756; the cited EQEmu Titanium encoder writes a constant 25 there.
- Retain a checked optional base held model from Titanium item definitions.
  The item model and received wear appearance remain separate.
- One explicit Titanium bow shot or throw through `GameCommand::RangedAttack`
  and `Capability::RangedCombat`. The session checks the target revision,
  loadout, reach and unmodified weapon delay. Damage and ammunition changes
  come through the existing server events.
- `ClientCommand::StopSong` is offered by Titanium EQEmu through
  `Capability::StoppingSongs`. The worker checks Bard class and current
  admission. A silent stop of a tracked gem cast releases the local hold
  after one second and emits `WorldEvent::SongStopUnconfirmed`. Received
  cast results continue through the ordinary spell updates.
- **Breaking:** `WorldEvent::CastRequestUnconfirmed` reports the existing
  cast acknowledgement timeout with its admission and spell. A
  `CastPending` clear precedes a received update that answers the request.
- **Breaking:** `WorldEvent::MotionRefused` identifies a discarded movement
  request by admission and request stamp. `MotionSent` describes local
  worker acceptance or refusal, including accepted samples held for pacing.
- `BookActionStatus::Unconfirmed` releases the spellbook after 30 seconds
  without a matching result. Distinguishable edits can proceed. A retry with
  the same possible reply requires a fresh admission. A timed-out scribe
  still guards cursor custody until received whole-slot evidence.
- Apply authoritative gem clears by slot. Match the old spell or an empty
  sentinel separately when deciding whether a pending forget was answered.
- `WorldEvent::OwnPet` reports the current pet's identity and revision for the
  admission. `ClientCommand::Pet` now takes that identity, a target revision
  for targeted orders, and a creation time. `SelectTarget::expected_pet`
  pins a pet-specific selection to the same identity. Charm, ownership changes and
  death update the existing spawn table. Stale pet and target orders are
  refused before sending. Guard Me uses Titanium command 6, and the
  Project 1999 policy refuses hold, feign, spellhold and focus.
- Grant bounded swimming for Titanium EQEmu sessions. Swim proposals use the
  existing position update and share one speed budget across all three axes.
  A motion submission does not establish server acceptance or water state.

- Default-off P99 list presence replies (`ClientConfig::answer_list_polls`,
  `ClientCommand::AnswerListPolls`, `Capability::AnsweringListPolls`). Runtime
  disable cancels an owed reply. Only server-driven presence polls schedule
  the five-byte reply; population notices never join a list or become a
  captcha. The 3..=60 second delay is application policy. Source layouts and
  synthetic tests do not establish deployed server acceptance or timing.

- Explicit admission-scoped movement and spellbook choices:
  `movement::MovementOptions`, `ClientCommand::SetMovementOptions`, and
  `ClientCommand::SetBookOptions`. All departures start off. Stun movement
  retains transition freshness; a 10% forward-run increase and bounded
  vertical movement retain the ordinary movement guards. Memorizing without
  automatic sitting skips the local preparation delay and retains reply
  handling; server acceptance remains unverified.
- `ClientCommand::ExitAtOnce` submits ordinary Logout without camp preparation
  and uses the existing reply/close lifecycle. Server acceptance and cleanup
  remain unverified on deployed public servers.
- `Capability::GameplayDepartures` and `ServerType::offers_gameplay_departures`:
  the explicit movement and memorization departures are offered only where the
  server type permits them. `SetMovementOptions` and `SetBookOptions` need that
  capability; a zone session that does not offer it refuses them ("Gameplay
  departures are not offered on this server"). Every checked server type
  offers them while the owner tests them (2026-11-06); narrowing that to the
  player's own servers is one override per server type. A departure on a
  public server is faster or more permissive than the official client.
- Explicit cursor-item destruction (`ClientCommand::DestroyItem`,
  `Request::DestroyCursor`, `InventoryUpdate::DestroyCursor`) on the Titanium
  inventory policy, and gated ordinary beneficial-buff cancellation
  (`ClientCommand::CancelBuff`, `WorldEvent::BuffCancellation`,
  `Capability::CancellingBuffs`, `EQEmu` only). See
  `docs/cursor-destruction.md` and `docs/buff-cancellation.md`.
- Grounded Titanium diagonal movement (`MovementMode::GroundedDiagonal`,
  `movement::{GroundedDiagonal, DiagonalRates, DiagonalAlong, StrafeSide}`)
  with its own settled heading budget, refused during jumps, and absolute
  mouse heading snapshots (`movement::HeadingUpdate`). Titanium's
  `eqgame.exe` establishes the raw lateral half-rate and the independent
  heading and current yaw fields. The sampled diagonal endpoint and
  completed-facing snapshot with zero continuing yaw are inferred (recording
  checklist 6.388). Live P99 acceptance of this pair remains unverified.
- Network-owned dialect accessors for heading snapshots, fatigue-based
  carrying capacity, ordinary door cursor checks, and reading window kinds.

### Deprecated

- `PetCommand::project1999_available`, in favour of
  `PetCommand::in_classic_order_set`. The alias returns the same
  classification; the provider chooses the order set for each admission.

### Changed

- **Breaking:** A moving player stops only by the host's own stop (a request
  at the current point with no turn), or by a stun, withdrawal or
  correction, which now each hold a stop for the update gate. A gap without
  input no longer sends an inferred stop after 250 ms: in the program the
  movement and the sending share one loop, so a stall only delays the next
  update, and a client frame stall had made a straight run send a stop and
  then a speed change about every 300 ms. Hosts must submit a stop before
  they stop driving movement; a camp waits for it.
- **Breaking:** Titanium's own update goes out on its program's flag rule
  (0x4aba14, 0x4a2ef2, 0x4a2f3f, 0x4a2f67, 0x4231c2): a heading change of 2
  or more units, or a speed change of 0.2 or more (8 in the forward field),
  each from the last value that flagged one; every strafing sample; a jump;
  and the 1 s character tick; then more than 280 ms after the last update.
  Running straight now sends once a second. While the server holds the
  player frozen (posture 102, 0x4ac842), updates come only from the tick,
  more than 560 ms apart, with the counter field repeating the last value
  and no history entry; while stunned without that posture, and unless a
  departure moves the player, nothing goes out. `UpdateGate::Flagged` gains
  `rule: FlagRule` (`EQMAC_FLAGS`, `TITANIUM_FLAGS`), and
  `MotionSession::set_hold` takes `movement::Hold`. Spell and contact pushes,
  which the program also flags, are the server's moves here, and a
  recording's few short straight gaps they don't explain are open
  (recording checklist 6.421).
- Titanium's own position update carries the program's movement-history
  entry counter in its counter field (0x14cb offset 2; 0x4ac5c1, 0x4b70db)
  in place of one more per update: the frames since admission at a fixed
  65 a second, plus one entry for each earlier update, and for a local pad
  the region's two entries and the warp's entry at the old place (R5 audit
  U1). The per-frame history sender stays gated off on public servers by
  our policy, and the counter counts regardless. The rate and the left-out
  spell-path entries are inferred (recording checklist 6.420). `EQMac`'s
  field is unread and keeps one more per update. New
  `movement::UpdateCounter`, `TITANIUM_UPDATE_COUNTER` and
  `MotionSession::with_counter`.
- **Breaking:** Report the raw dialect-tagged state in `WorldEvent::Visibility`
  and the observer's current Titanium sight in `WorldEvent::ObserverSight`.
  `AppearanceChange` can report the Titanium GM flag, and item placement
  records carry optional Titanium Light and item-class metadata.
- **Breaking:** Struct literals and exhaustive matches need the new fields:
  `WorldEvent::InspectRequest` gains `target_id`, `popups::Popup` gains
  `negative_id`, `Appearance` gains `titanium_invisibility` and
  `titanium_gm`, and `Capability::ALL` grows from 55 to 57.
- **Breaking:** Preserve the unanswered spell and cast source for a
  compatible own start after the local cast wait. Match the spell ID or
  stock TAKP's documented Luclin port rewrite. Unrelated own starts keep
  separate cast state. Handle `WorldEvent::CastLateBegin` before the
  received spell update.
- **Breaking:** Both movement results now carry the original local request
  stamp. A Move discarded at selection or abandoned in a failed zone batch
  receives a typed refusal while the event stream can still deliver it.
  Supply or match `WorldEvent::MotionSent.created`.

- **Breaking:** Replace `Inventory::owned_titanium_link` with metadata-based
  `InventoryItem::titanium_link`,
  including for a record displayed during an inventory move.
- **Breaking:** Keep queued book and note requests valid across unrelated inventory updates.
  `ReadItem` takes a captured selection so the worker can check the item and
  container before asking for the text.
- **Breaking:** `MovementOptions::run_boost` and `MovementOptions::RUN_BOOST`
  are replaced by `run_speed_multiplier: RunSpeedMultiplier`, a bounded 100
  through 140 percent forward run speed multiplier with 100 as the default.
  The worker budgets the same forward component and fall drift as the host;
  flight keeps its unscaled vertical budget. The option remains a gameplay
  departure, offered only where `SetMovementOptions` is.
- A speed-only change of the run speed multiplier no longer stops voluntary
  motion: the held paced sample keeps its send deadline and the moving state,
  and the new budget applies to later samples. Changing flight or stun
  permissions still stops motion first.
- The Titanium translocate decline carries -1, as the Titanium program sends
  it (`eqgame.exe` 0x499f08), instead of 0. `EQEmu` accepts only 1 as a yes,
  so the outcome is unchanged.
- **Breaking:** `GameCommand` gains `LiquidDamage` for periodic reports and
  `WorldEvent` gains `LiquidHazardRevision`. `EnvironmentalDamage` retains
  the completed-fall path. `Capability::ALL` grows from 50 to 52 with
  `Drowning` and `LavaHeat`. Struct literals for `PlayerState` and
  `ItemBonuses` gain the optional `titanium_environment` and
  `fire_resistance` fields, respectively.
- **Breaking:** scope resurrection and translocate consent to an admission
  and a specific offer. `WorldEvent::Resurrection` and
  `WorldEvent::TranslocateOffer` carried the bare offer and now carry
  `confirmations::ConsentOffer<T>`; `AnswerResurrection`, `AnswerTranslocate`
  and the two `*Refused` events gain `offer_id`; `WorldEvent` gains
  `ConsentClosed`; `Message` gains `Resurrection`, `TranslocateOffer` and
  `MerchantScoped`, and `world::titanium_update` no longer emits consent as
  world events. Hosts can observe the worker's `ConsentClosed` disposition
  independently of server results. The raw offers decode through `message`
  before session policy and recipient checks.
- **Breaking:** pet orders carry the pet's identity. `ClientCommand::Pet`
  was `{ session_id, command, target: Option<u16> }` and any spawn owned by
  the player sufficed; it is now `{ session_id, command, pet:
  Option<PetIdentity>, target: Option<TargetState>, created }`, refused with
  "Your pet has changed" when the identity no longer matches the current pet,
  and subject to the ordinary input expiry. `SelectTarget` gains
  `expected_pet`; `PetCommand` gains `GuardMe` (`ALL` grows from 16 to 17).
- **Breaking:** exhaustive public enums grow. `GameCommand` gains `StopSong`,
  `RangedAttack` and `DepopLocalNpc`; `WorldEvent` gains `OwnPet`,
  `CastSuperseded` and `SongStopUnconfirmed`; `Capability` gains
  `RangedCombat`, `StoppingSongs` and `Swimming` (`Capability::ALL` grows
  from 47 to 50); `MovementMode` gains `Swim`; `BookActionStatus` gains
  `Unconfirmed`; `Request` gains `RangedAttack` and `StopSong`; and
  `ItemDetails` gains the public field `base_held_model: Option<NonZeroU16>`,
  which breaks struct literals.

- Restore the configured P99 list reply preference on return to character
  selection. A runtime choice follows the same character across zones.

- **Breaking:** rename `buffs::cancel::Progress::ObservedRemoval` to
  `FadeObserved`. The event observes a matching received fade during the
  local wait; natural expiry can produce it too. No cancellation ACK or
  slot-only sentinel fallback is inferred.
- Expose the inventory policy's cursor destruction support via
  `MoveRules::supports_cursor_destruction`. Document its Titanium bag slot
  bound and the explicit P99 buff cancellation policy.
- `PlayerAdmissionError` and the additive `ConnectionStage::Recording`
  milestone (serialized `recording`); `ConnectionStage` is non-exhaustive, so
  no match breaks.

- Interpret received own-player stuns on stock Titanium `EQEmu` using the
  existing Titanium timer, including zero, overlap and admission boundaries.
  Malformed packets remain rejected. Synthetic receive-path tests do not
  establish live server receipt or movement acceptance.

- **Breaking:** the exhaustive `ClientCommand` enum gains `AnswerListPolls`,
  `SetMovementOptions`, `SetBookOptions`, `ExitAtOnce`, `CancelBuff` and
  `DestroyItem`; `WorldEvent` gains `BuffCancellation`; `Capability` gains
  `AnsweringListPolls`, `CancellingBuffs` and `GameplayDepartures`
  (`Capability::ALL` grows from 44 to 47); `MovementMode` gains
  `GroundedDiagonal`; `MovementRequest` gains `heading: HeadingUpdate`, and
  `MotionCalibration` and `WorldEvent::MotionState` gain `diagonal:
  DiagonalRates`, so struct literals must supply them. New local worker
  feedback is `WorldEvent::MovementOptionsApplied` / `MovementOptionsRefused`;
  these are separate from server replies and speed-input grants.
- Commands queued behind one that kills or transfers the player in the same
  batch are refused ("The player is dead" / "The player is between zones")
  instead of being dropped, as they were before the batch was drained eagerly.
  The old zone continues refusing gameplay commands after approving departure.
  Commands already drained when a command ends the session get a refusal too.

- Keep movement load conservatively bounded through known carried-stack splits
  and merges, including independently refused or partial moves. Placement stays
  unconfirmed; unknown, bag, bank, purse and correction history remains guarded.

- **Breaking:** correlate target selections and their admission-scoped results.
  `GameCommand::SelectTarget` gains `request_id`; `WorldEvent::TargetSent`,
  `TargetRejected` and the new `TargetChanged` carry `request_id` and a
  `targets::TargetState` (revision, spawn and availability).
  Publish session target identity and availability without reflecting server
  changes as outgoing requests. Targeted abilities must name the unchanged
  target version they were requested for (`UseAbility.target:
  Option<TargetState>`); stale or unresolved intent is refused.
- **Breaking:** `WorldEvent::OwnStun { session_id, revision, state }` reports
  the session's interpreted own-player stun as a `control::StunState`
  (`Clear` or `Stunned`), with increasing revisions within an admission;
  `CampEnd::Stunned` is a new variant. Movement submitted while stunned, or
  predating the current stun state, is refused.
- **Breaking:** group invitations carry consent identity.
  `GroupUpdate::Invited` gains `kind: InvitationKind` and
  `id: Option<group::InvitationId>`; `GameCommand::FollowGroup` and
  `DeclineGroup` require the displayed `inviter` and `invitation_id`;
  `GroupUpdate::Formed` names its `leader`. Titanium's acceptance of an
  invitation to an existing group uses `group::FOLLOW2_OPCODE`.
- **Breaking:** `WorldEvent::PurseUncertain { session_id }` and
  `WorldEvent::MerchantUnanswered { session_id, reason }` are new variants: a
  purse projection that cannot be established stays unknown until an
  authoritative update, and a sent purchase or sale whose outcome stays
  unknown after waiting is reported without an item or coin change, and
  without authorizing a retry.

- Offer source-checked P99 note/book reading through the ordinary inventory
  action. Retain Titanium's Book flag separately from EQMac's BookType rule;
  refuse invalid metadata, stale selections and unsupported text kinds instead
  of opening a guessed note. Only received server text supplies a read result.
- **Breaking:** `GameCommand::ReadItem` now requires `revision` and `created`.
  Select from the submitted worker inventory and retain the enqueue time.
  See `docs/p99-reading.md` for the wire contract and remaining live checks.

- **Breaking:** separate carried combines from world-station actions with
  `Capability::WorldContainers` (`ALL` grows to 44). P99 offers only carried
  manual combines, with current container, component, identity and cursor
  guards. EQEmu keeps both offers; TAKP/Quarm are unchanged. Titanium's
  four-byte combine request writes the full native slot with a zero upper
  word, replacing the invented tribute sentinel. A received reset clears
  only the local gate; consumption/products remain authoritative inventory
  news. Additive `tradeskills::{combine_capability, check_carried, CarriedRefusal}`.

- Return a typed modern `SessionTermination` for a matching peer disconnect or
  the established stateless out-of-session control. Deliver complete, ordered
  applications preceding termination exactly once before its error; discard
  unfinished work and later combined suffixes, and stop all network I/O.
  Refuse application sends immediately. Caller-configured fresh-login
  retries retain their existing delay; disabling reconnect returns the typed
  terminal error. See [the contract](docs/session-termination.md).
- Add read-only `Transport::termination()` and stop zone outbound work while
  its completed application prefix drains. **Breaking:** distinguish a received
  camp logout reply (`CampStatus::Camped`) from the unchanged local close
  deadline (the new `CampStatus::ClosedWithoutReply` variant); the deadline
  does not confirm server cleanup.

- **Breaking:** add `Capability::Meals` for consuming food/drink and choosing
  automatic meals, separate from inventory moves. Inventory owners offer it
  only when their existing meal policy handles those commands.

- Name the rejected command variant in refusal diagnostics without logging its
  payload; structured refusal events keep their existing reason and identity.
- Offer note/book reading on TAKP and Quarm using EQMac's kind/name request
  and kind/text reply, without Titanium's window byte. Read the item's
  BookType byte separately from its Book flag. Invalid text names produce
  a local read refusal; only a server text reply supplies readable content.
- Expose `Client::inventory_move_rules()` from the same selected server policy
  used by worker inventory validation so hosts can predict with matching rules.
- Preserve a conservative carried-weight bound for a distinct plain cursor
  arrival after whole-item carried moves, without confirming placement or
  discarding unrelated predictions. `Inventory::carried_weight_is_conservative()`
  distinguishes this bound. Affecting intermediate mass histories stay unknown
  until a full inventory baseline or existing policy settlement.
- Offer guarded ordinary EQMac door use on TAKP and Quarm. Encode the
  source-backed eight-byte no-held-item request and player ID; retain server
  authority for locks, scripts and movability. Shared ordinary hinge motion
  guards clicks during movement and models native ten/twenty-second local
  return timing without predicting acceptance or sending retries. Specialized
  and unknown mechanisms remain visible without use. Additive APIs:
  `doors::eqmac::{click, ordinary, Hinge}`.
- **Breaking:** character-selection events require `connection_id` and
  `deletion: Option<deletion::Rules>`. Consumers must preserve the actual
  connection/list scope. New `GameCommand::DeleteCharacter(Request)` and
  `WorldEvent::CharacterDeletion(Update)` variants use exact named confirmation
  and one world-stage owner on all four checked server policies. The request
  sends only the native NUL-terminated name. Results distinguish local refusal,
  pending, unconfirmed, and absence observed in a specific refreshed list;
  absence is not a correlated deletion acknowledgment or proof of database purge.
  A pending/uncertain attempt blocks entry/creation until a fresh absence
  observation or explicit disconnect; it never automatically retries.

- **Breaking:** inventory move requests can carry ordered local projection
  identities: `InventoryMove` gains `sequence: Option<MoveSequence>`, and
  `WorldEvent::InventoryAction` gains `sequence` and
  `update: Option<InventoryUpdate>`. Local move results atomically carry the
  worker's prediction in that `update` instead of emitting a separate
  `WorldEvent::Inventory` prediction event. Exact revision and predecessor
  checks reject stale dependent moves without treating local submission as a
  server acknowledgment.

- **Breaking:** `WorldRules` gains optional `language_zero_selector`; complete
  struct literals must initialize it or use `..WorldRules::default()` for
  absence.

- Prepare one checked movement candidate inside its existing owner before
  committing a sent, held or unchanged sample. Preserve public method
  signatures, generation gates, wire delivery and failure behavior.
- **Breaking:** `GameCommand::SpeedInputs` now carries `revision` and a
  `movement::SpeedInputState` (`Ready(SpeedInputs)` or
  `Unavailable(SpeedInputUnavailable)`) instead of bare rate, weight and
  capacity. The session asks for them with the new
  `WorldEvent::SpeedInputsRequired { session_id, revision }` on admission and
  whenever a position correction, level, buff, own spell landing (by the
  server type's landing flag), `EQMac` fatigue, inventory revision or coin
  change could alter them; until the host answers the current revision it
  withdraws walking, turning and jumping, keeps bounded passive descent and
  heartbeats, and rejects an older revision. `ConfigureMotion` is refused on
  every current wire. An unknown input is never taken as zero.
- **Breaking:** `GameCommand::SetPosture` requires the new
  `Capability::Posture` (`Capability::ALL` grows to 42). `Moving` covers
  locomotion only; hosts gating stance controls on it must switch.
- **Breaking:** `WorldEvent::SkillReset(abilities::eqmac::SkillReset)` is a
  new variant carrying `EQMac`'s raw eight-byte skill reset; `SkillReady`
  still follows only a zero timer.
- **Breaking:** `items::ItemPlacement` gains `eqmac: Option<EqMacItemRules>`
  (Quarm's stack and GM flags) and `bag_weight_reduction: u8`; literals and
  exhaustive matches must add them. `inventory::MoveRules::Quarm` joins the
  rules.
- `EQMac` legacy transport starts outgoing reliable sequences at ARQ 2 and
  no longer rewinds delivery on a retransmitted `SEQStart` at the same ARQ,
  so the server's cumulative acknowledgement reaches the first packet and an
  idle world session is kept. Keepalive, retry (3 s, ten attempts) and
  silence (60 s) timers are unchanged.

### Fixed

- A player the zone admits without any inventory message now holds an empty
  inventory instead of an unknown one, so movement is granted after a death
  that left everything on the corpse; the original client then holds nothing
  too (inferred, recording checklist 6.437). An unreadable or partial
  inventory at admission stays unavailable, a later snapshot replaces the
  empty one, and the session logs the empty admission as a diagnostic.
- Eating, drinking, looting or any other change in the player's belongings
  no longer withdraws movement while the session waits for the new speed
  inputs. The last inputs stay in force (health, stance and fatigue still
  apply at once) until the answer lands, so a jump, a crouch or autorun
  under way goes on. A refused answer, or none
  within a second, withdraws as before.
- Retire a displaced station's local view and settle unrelated Titanium
  moves independently. Carried slots a station route touched settle one
  refusal window after the station clears or detaches, so a combine product
  pushed on the cursor or a close no longer pauses movement for the rest of
  the zone. A P99 forge combine reset limits the remaining hold to station
  and cursor actions while any clear is pending.
- Let a received same-instance return resolve a whole cursor handoff. Once
  the trade slots empty, a later pickup keeps the given item's weight as an
  upper bound, and one refusal window after the close retires the handoff,
  so a give the receiver kept no longer pauses movement or holds the forge
  until zoning. `Inventory::handoff_returns_unsettled` and
  `Inventory::handoff_returns_settleable` report this state.
- Distinguish stock evidence from a missing P99 bandaging result at expiry.
- Keep both knight reuse deadlines if computing their replacements overflows.
- Bind Wound's own button time now runs out in full after every use, as the
  official client's does, whatever ended the bandaging; the self-bandage path
  checks it too, so a moved or released P99 self-bandage cannot be resent at
  once.
- One received decrease from a self-bandage settles every carried bandage
  stack, so a second stack no longer holds the inventory until the result
  window ends or stays uncertain for the rest of the zone.
- A player inspection waits for its answer before another request goes to
  the same player, for up to five seconds (inferred, recording checklist
  6.400), so a held key or script cannot repeat requests every frame.
- Let unrelated EQEmu inventory moves settle during an unresolved station
  transfer. After a combine reset, keep any remaining hold on the station
  and cursor until covering evidence arrives.
- Let a P99 forge use the ordered empty cursor projection for opening and
  whole-item withdrawal. Received item placement remains separate.
- Use the current weapon delay and eligible quiver when checking Titanium
  ranged recovery. Haste and slow remain outside the local estimate.
- Keep absolute coordinates on received same-zone moves. Zone-point
  preservation applies to zone-point routes.

- Keep reporting completed falls while a cursor drop awaits inventory evidence.
  A drop requires enough item and coin evidence to establish carried weight.
  `Inventory::ground_drop_weight_tenths` retains the heaviest received
  cursor group through cursor-only resync for landing reports.
  `Inventory::carried_weight_tenths` returns the bound during a clean
  Titanium drop hold. Fresh movement inputs must meet that bound, capped
  at `MAX_CARRIED_WEIGHT` in whole units.
  Ground drops are restricted to local-only EQEmu pending live validation.
  A landing whose cost depends on a load the hold cannot bound is not
  reported, which the local-only fence keeps off public servers.
- Keep merchant commands and Titanium receipts scoped to the relevant shop.
  Check stock, quantity, and sale custody before sending. Preserve an
  unanswered transaction across a shop close and await server evidence.
  A Titanium refusal with a merchant ID can resolve that shop's sole matching
  action. Despawn or replacement retires the catalog and releases the pending
  trade as unknown while retaining any late result. A late refusal of an
  already released trade leaves the host's other unanswered action alone.
- Retire item and coin escrow when a trade cancel is sent. Returned holdings
  come from server inventory and money updates. Withhold exchange replies
  that cannot apply to the current stage or partner.
- Check that a bank conversion's source holds the whole offered amount,
  including the amount the conversion leaves behind.

- Keep tracking jump ascent across flight toggles so toggling flight off
  cannot open grounded diagonal movement during a tracked jump. The jump's
  own rise allowance also survives the toggle; it expires at the apex as
  before.
- Report the first zone offer in a record-only zone session, even if the offer
  is unreadable. The listener stays on the current connection.

- Report `ConnectionStage::Recording` when a completed zone handshake permits
  communication records but the player snapshot cannot be decoded. Record-only
  runs retain active connection health. Interactive runs instead close the zone
  and return `PlayerAdmissionError` without reconnecting. `Ready` now requires
  an admitted player. Serialized progress consumers may see `recording`.
  The record-only heartbeat stops using the entry position when an own update
  or transfer supersedes it, including during the handshake.

- Keep tracking jump ascent when run boost or stunned-movement options
  change. Boosted forward movement that becomes a fall keeps its horizontal
  budget across the transition.
- A local modern transport close discards staged applications and ignores later
  peer controls. If peer termination was observed first, its ordered,
  completed application prefix still drains once before the terminal error.
- **Breaking:** re-evaluate EQMac fatigue from current host strength/stamina
  prerequisites in the session, avoiding an unnecessary withdrawal and host
  round trip on each stamina tick. `SpeedInputs` gains
  `eqmac_fatigue: Option<EqMacFatigueInputs>`, which preserves legacy capacity
  semantics and conservative unknowns; real readiness losses still withdraw.
- **Breaking:** follow Quarm's same-zone death-bind approval without treating
  its documented reply stand-in as an unsolicited destination change. The new
  `WorldEvent::DeathBind` reports bind-only death knowledge before transfer
  without inventing killer/corpse details; dead state is retained through
  departure and duplicate departure offers are ignored.

- EQMac admission records the profile's current zone identity before handling
  zone-line requests. Zero and oversized IDs stay unknown without failing an
  otherwise valid admission; bind points and zone names do not supply guessed
  identities. Transfers still refuse unknown current-zone identity.

- Defer a camp requested just after movement until the existing motion owner
  submits its real paced stop. A trailing stationary stop can finish the
  pending intent; new movement or a failed/refused stop cannot start camping.
  Official ordering remains unverified (recording checklist 6.380).

- Report a typed abandoned outcome when a waiting camp loses its admission
  or movement context, or a zone transfer arrives. Clear the intent before
  notifying, so overlapping events cannot repeat it or start a countdown.

- Keep contribution readiness through equipment predictions only when all
  affected definitions explicitly have zero direct bonuses and no worn effect.
  Retain affecting or unreadable intermediate equipment uncertainty until
  a full authoritative baseline, including after a local round trip or matching
  partial slot update. Existing protocol settlement policy is unchanged.

- Preserve the original world-update rejection reason while appending the
  received opcode and application payload length to its existing diagnostic.
  Packet bodies are not added, and decoder acceptance rules are unchanged.
- A grant after a fresh movement calibration carries its
  `WorldEvent::SpeedInputsAccepted` receipt again. A same-zone relocation
  (an in-zone teleport), a server correction or a refused transfer's rewind
  withdraw the host's movement and clear the calibration without asking for
  the inputs again; the receipt was only re-sent when its contents changed,
  so a host that treats a withdrawal as ending its receipt waited for one
  that never came while the player already moved.
- Scope unresolved EQMac ground-pickup waits to further pickups so unrelated
  inventory actions can continue. Cancel the wait on foreign/unattributed object
  removal, an authoritative inventory snapshot or a three-second local expiry,
  preserving ambiguity so late results cannot settle a newer pickup. No automatic
  retry, inventory grant or move acknowledgement follows cancellation.

- Send valid inventory moves immediately in submission order on TAKP and
  Quarm, retaining ordered unresolved predictions without treating silence as
  acceptance. Preserve slot, container, fit, stack and restricted-item guards;
  contradictory EQMac corrections require a fresh snapshot.

- Reject ground pickup requests made before an object's current placement,
  including IDs reused after disappearance, through the existing object owner.

- Reconcile a scroll the server deleted after a confirmed scribe with the
  move that put it on the cursor, so a second scribe is not read as a
  correction and a current movement grant is recovered without re-entry.
- Decode the full 100-word `EQMac` profile skill block and the eight-byte
  skill update, so Quarm and TAKP offer the abilities the character has
  trained and gate the off-hand by the trained skill. 254 and 255 remain the
  "cannot train" and "untrained" sentinels.
- Enable Quarm target selection and clearing through the existing EQMac
  feature, retaining visibility checks and authoritative server changes.
  Target selection alone sends no attack request; combat is offered separately.
- Enable Quarm's shared EQMac ground movement, turns, jumps, falls, zone
  handoffs and return to bind after a server-reported death. Follow its
  inventory and coins for carried weight without offering item actions.
  Source-derived; public movement validation remains pending. Swimming,
  boats and client-reported bleed-out remain incomplete.
- Offer Quarm camping with independent sitting/standing/ducking. Posture
  commands now require `Capability::Posture`, which all four policies provide;
  consumers must stop using `Moving` to gate stance controls. Its camp flow
  is source-derived and offline-tested.
- Enable P99's existing bounded jump path after checking eight archived
  official-client jump notices; precise physics remains client-code-derived,
  and live custom-client validation is still pending.
- Preserve P99 and Quarm character-selection appearance instead of stripping
  it; Quarm also retains the supplied level and zone. P99 is capture-checked,
  Quarm source-checked; public visual validation remains outstanding.
- Decode Quarm's eight-byte stamina update alongside TAKP's five-byte form,
  preserving the shared food/water counts and rejecting other lengths.

### Added

- Add `ProviderPolicy::requires_server_tables()` so hosts can follow the
  network-owned registration/replay requirement without matching policy names.
- Decode `EQMac` wear changes into the existing typed appearance updates,
  including held item models, removal and tint changes. Validate its 12-byte
  layout independently of Titanium's 9-byte layout; retain strict spawn and
  texture-slot validation.

- Offer Quarm food/drink through the existing EQMac inventory owner, withholding
  consumption of stale or moved items and automatic meals during merchant holds.
- Offer bounded Quarm merchant purchases/sales, preserving its stack rules,
  known price/capacity guards and zero-quantity sale refusals without delivery claims.
- Expose `Client::spell_table_policy()` and `ProviderPolicy` from the same
  network policy that emits spell tables, before the first event arrives.
- Add optional numeric `SessionStatus::run_id`, matching spell-table context
  across world/zone transports, and `with_run_id`. Network statuses always
  supply it. Preserve the released string-taking constructor as display-only;
  an unknown ID stays unknown and is never reconstructed from its label.

- **Breaking:** Add `LootRequest`, `LootRequestStatus` and the exhaustive
  `WorldEvent::LootRequest` variant for local open/item/close submission and
  refusal, including shared validation and generation-encoding refusal. Preserve
  original admission/action parameters and process-local creation time for host
  correlation, with one typed result and the existing refusal diagnostic. Consumer
  integration is required in the next minor pre-1.0 release. Server loot denials
  and receipts remain separate from local disposition and inventory delivery.

- Preserve Titanium's raw `OP_LogServer` language-zero name selector in
  `WorldRules::language_zero_selector`, distinguishing an absent field from
  received zero. The EQMac mapping remains absent until independently verified.

- Add source-derived EQMac loot open/close, money, listing and item-receipt
  codecs, and offer Quarm looting through its server policy and existing
  inventory owner. Live validation remains pending.
- Track corpse access through grant, listing and server completion; reject
  stale or unlisted item requests and avoid duplicate coin grants. Preserve
  ordinary inventory deliveries after close, without predicting loot success.
- **Breaking:** `LootUpdate::Acknowledged` is a new variant of an exhaustive
  enum, so downstream matches need an arm: an EQMac receipt echo is not a successful item
  transfer. Document source evidence, limitations and controlled review checks
  in `docs/eqmac-looting.md`.

- Typed local speed-input acceptance receipts, including unchanged-speed
  acceptance and the current positive-grant outcome. Hosts supply an opaque
  `report_id` on `GameCommand::SpeedInputs` and handle
  `WorldEvent::SpeedInputsAccepted`. Receipts echo the admission, revision,
  report identity and exact ready inputs; they never replace movement guards
  or acknowledge a remote packet. This is an unreleased breaking API change.

- Quarm offers normal attack and consider through the shared EQMac combat
  owner. `combat::CombatRequest`, `CombatRequestStatus` and
  `WorldEvent::CombatRequest` distinguish transport submission from local
  refusal, including stale-admission and policy refusals. Self-consider is
  supported; unavailable spawns and inactive admissions are refused. Server
  consideration, damage and animation remain authoritative; no attack result
  or acceptance acknowledgment is synthesized. Validation is source-derived
  and synthetic; controlled live verification remains outstanding.

- `inventory::ItemRules`, `inventory::decode_eqmac_with` and
  `message::eqmac_with` let server policy select TAKP or Quarm item
  stackability while sharing one EQMac generation wire. Existing decoder
  entry points, raw stack/GM observations and inventory actions retain
  their behavior.

- EQMac door and ground-object codecs with bounded compressed door tables,
  raw state/actions, generation-specific refusal sentinels and typed updates.
  The established eight-byte pickup request uses the EQMac opcode; object
  messages never fabricate inventory. TAKP and Quarm policies offer ground
  pickups and replay door definitions/actions while leaving door clicks
  unavailable; see `docs/eqmac-world-objects.md` for sender limits.
  A generation-selected EQMac result controller suppresses unresolved duplicate
  pickups and holds unknown incoming inventory until separate authoritative
  results arrive, without optimistic items or timeout retries. The established
  Titanium result behavior is preserved.

- Typed P99 spell-table registration/replay decoding and ordered
  `WorldEvent::SpellTable` delivery across world and zone transports,
  including before profile admission. Complete shape and registry-ID
  validation produce explicit unavailable events for malformed tables;
  order and duplicates are retained. Installed snapshots and selective
  definition application remain host-owned, with no new game requests.

- Quarm casting, gem memorization and spellbook reading through the shared
  `EQMac` features, without book moves or deletion; item casts use the
  current inventory and never predict charge consumption. Source-derived
  from Quarm's cast and memorize handlers.
- Typed `EQMac` abilities (`abilities::eqmac`): fifteen supported abilities
  with per-ability opcodes and bodies, class and skill-rank gates, a shared
  strike recovery released only by the server's reset, melee range 16 units
  for strikes and taunt, and Tiger Claw reuse 6 s. Offered on Quarm and TAKP.
  Feign Death sends the lying appearance before the feign notice unless the
  player already lies, and the notice carries the program's six-byte body
  (own spawn id, zero word, two zero bytes), as the `EQMac` program does.
- Guarded Quarm inventory and coin moves (`MoveRules::Quarm`): a prediction
  stays unsettled until a covering authoritative update, a contradicting
  replacement invalidates until a fresh snapshot, and no absent bank reads as
  zero. `Inventory::carried_weight_tenths` ports the servers'
  `CalcCurrentWeight` (a stack counts once, `EQMac` reduces each bag's
  contents, Titanium each item, coins at a quarter) and reports
  `WeightUnavailable` instead of guessing.
- `food::eqmac_profile` reads hunger, thirst and fatigue from the `EQMac`
  profile; the `Nourishment` fatigue feeds the speed inputs.
- `movement::history::Record` decodes Titanium's 17-byte movement-history
  records, preserving separate clock/sequence fields and unknown causes.
  Protocol notes document the verified opcode and remaining capture gaps;
  sessions do not send speculative history keepalives.

- Character-list creation policies (`creation::Rules`) with era/expansion-aware
  races, classes, deities and generation-specific starting cities for P99,
  Titanium EQEmu, TAKP and Quarm. Quarm creation is source-derived from its own
  world handlers; this does not claim a new live check.
- Complete creation appearance policies and packet fields: base face, packed
  head detail, hair, beard, both eyes and barbarian tattoos, including absent
  attachment sentinels and race-specific palettes. `NewCharacter.tutorial`
  controls Titanium's first-entry tutorial preference.
- Name suggestions: Titanium world requests and local generation for EQMac
  policies, with stale-reply guards and single-flight timeout handling.
  API migration: `NewCharacter` literals add `face: 0`,
  `cosmetics: Cosmetics::default()` and `tutorial: false`; `CharacterSelection`
  builders add `creation: Option<creation::Rules>`. Exhaustive command/event
  matches add `GenerateName` / `NameSuggestion`.
  These are unreleased API changes, not a compatible published 0.1.x patch.
- Creation timeout recovery: after 60 seconds without completion, reconnect
  before retrying. Success requires the requested name in the refreshed list;
  that list waits for explicit character selection instead of automatic entry.

- `zones::ZoneDescription::classic_model_scale_flag`: the word at offset 592
  of Titanium's `OP_NewZone`, as it came. While it is not zero the Titanium
  program draws the classic character models at 1/1.3 of their usual scale
  and rescales every spawn when it changes between zones (program read N1b:
  copied at 0x45bff5, used by 0x47a0e9). `EQMac` has no such rule, so its
  headers leave it None. A new public field: code that builds a
  `ZoneDescription` by hand adds it.

- `movement::between_reports` and `movement::ZBetween`: where each client
  generation puts a carried spawn's height between the server's reports, as
  a field of its step's table. Both are `ZBetween::OnFloor`, standing a
  spawn on the ground on the floor the host finds, which is what eq-client
  already did. Read for Titanium (R-z: the gravity path from init 0x4850a0,
  the landing check 0x48b6c5 snapping feet up to the floor each step) and in
  part for `EQMac` (on-ground 0x50ba07, floor 0x4b3a84, landing 0x4b479e;
  the gravity flag's setters untraced). The program lags the floor a little
  going downhill, falling under gravity (checklist 6.335). Which spawns keep
  their carried height instead, by movement mode, is checklist 6.337.

### Changed

- The movement guard lets a grounded sample rise the player's own climb in
  each 50 ms program step it spans, once the host sends it with the new
  `ClimbReach` command (`MotionSession::set_climb`), instead of one riser
  a sample whatever its length: a quarter-second sample on a 25° hill
  climbed at a fifth of the run. In both programs a step stands the feet
  on the floor the sweep from the spawn's origin finds, a stand height
  above them, so a step climbs up to that height less the sweep's radius
  (read 6.344: Titanium 0x48a6f2 / 0x48a0fc, TAKP 0x4b3a84). Until the
  host sends a climb, a grounded sample rises `MAX_GROUNDED_STEP` as
  before; descents and the fall rule are unchanged. New
  `movement::program_steps` and `movement::climb_reach` give the budget,
  for eq-client's walk to count the same.

- P99 now offers falls and reports fall damage (`OP_EnvDamage` type 252) by
  the Titanium program's rules. In the program the floor is whatever surface
  lies under the player, at any angle, and a step whose floor drops away
  falls (read R-s of 11-01), so a P99 player can now walk off a cliff or
  down a steep face. That P99's server takes the report as it takes the
  official client's is inferred (checklist 6.332). Jumps are still off.

- Titanium carries other spawns with the program's own step (Titanium read R2
  of 10-31), the same one as `EQMac`, from a per-generation table:
  - A position update's motion deltas are 1/64 of a unit per 50 ms, 0.3125
    units a second each, not the 0.144 fitted on P99's NPCs. That fit was
    right for NPCs only because their halved damping takes them to their
    input's pace within a call or two; P99's players were carried at under
    half their speed and pulled forward at each report.
  - Its animation value is the forward input, over 40
    (`WorldEvent::Position::forward`, and the new `SpawnState::forward` for
    a spawn record, inferred, checklist 6.327). It no longer becomes a
    velocity along the heading, so a newer `EQEmu` NPC's velocity is zero
    and it moves on its input alone, 0.582 units a second for each unit.
  - `carry` keeps 0.625 of the speed each call, adds 0.8 of the input
    along the heading, and halves an NPC's horizontal share while it moves,
    as on `EQMac`. Titanium's ground values are inferred from TAKP's
    (checklist 6.320), and so is halving every NPC (checklist 6.321). Races
    0x48, 0x49 and 0x194 drop their vertical motion; Titanium has no stale
    rule.

- Batch 3b-2, from the TAKP program (read R1 of 10-31):
  - `EQMac` carries an NPC at the program's own pace between reports. While
    an NPC moves, its horizontal speed keeps half as much each step, so it
    settles at 2.327 units a second for each unit of the speed byte, which
    is the server's pace. Before this it moved at a player's 4.267, ran
    ahead of the server and was pulled back about once a second. Every
    NPC is halved until the program's skip for linked NPCs is read
    (checklist 6.316).
  - The `EQMac` step runs once a call, as the program's frame loop runs it.
    Each call updates the speed once and moves by it times the frame's
    time in 50 ms spans, at most 3. The steady speed doesn't change; only
    the climb to it after a report goes by frames.
  - `Carried::reported` takes the spawn's kind. `Carried::pending` is
    gone.

- Batch 3b, from the official programs (Titanium sittings 8i and 9p, TAKP
  read N4):
  - The movement guard lets a sample rise as the client generation's jump
    does once the session has announced one (`MovementGuard::jumped`,
    `MotionSession::with_jump`, `Wire::jump_speed`): 44.55 units a second
    less the zone's gravity (`movement::gravity_per_second`, 160 at the
    default 0.4), over the next sample's whole interval and on to the top.
    A correction ends it.
  - TAKP gives to NPCs and trades in the `EQMac` window's own packets
    (`exchange::eqmac`): `Exchanges` is offered there. The player half
    rides the same packets and is not yet checked live (checklist 6.307,
    6.308). `Request::AcceptTrade` and `Request::CancelTrade` name the
    partner (`with`). `ExchangeUpdate` gains `AcceptsReset` (an accept
    taken back moves both sides to open again) and `Withdrawn`;
    `ExchangeUpdate::Busy` carries the refusal's reason. An item record the
    program reads in a layout not yet decoded (276 and 264 bytes) arrives
    as `ExchangeUpdate::Unread` with its slot, so the window shows the slot
    taken, and the session reports it as a diagnostic (eq-network's only
    log route; the client logs it). It never closes the session.
    On TAKP (`Wire::window_closes`, `exchange::WindowCloses`; read N4 1d, 2
    and 3d) the close answers the server's cancel for an NPC as well, and
    a window whose partner is past 20 closes with a cancel and
    `ExchangeUpdate::Cancelled` naming the player (checklist 6.311).
  - `CampStatus::Abandoned` says what ended the camp (`CampEnd`: standing,
    moving, or other), so the client can say the program's line for each.
  - The player's own update goes out as each generation's program lets it
    (`movement::UpdateGate`, `Wire::update_gate`, replacing
    `with_spacing`): Titanium only after a change, more than 280 ms apart;
    `EQMac` on its 1 s timer, a speed change of 0.2, a heading change of 2,
    a turn or a jump, more than 150 ms apart, with an unchanged update
    repeated 6 times and then held (TAKP gate read Q1, path A).
  - Reach per generation (`reach::Reaches`, `Wire::reaches`; Titanium
    sitting 8a, TAKP read T6 and T8): TAKP doors at 20 in 2-D, ground items
    and containers under 20 in 3-D, merchants at most 20 in 3-D, and an open
    container closed beyond 20 (`ObjectUpdate::Closed`); Bind Wound at 15
    on both, with each program's line (12418 on Titanium, 420 on TAKP).
    `Objects::check_open`, `check_pickup` and `Doors::check_click` take the
    reach. A door out of reach reports string 1462
    (`WorldEvent::DoorAction::string_id`, `doors::OutOfReach`). Neither
    door reach counts its size terms (checklist 6.311); Titanium's merchant
    and container reaches are unread (6.312).

- Batch 3a, from the official programs (Titanium sittings 8-9, TAKP sitting
  2 and the 10-31 roleplay read):
  - The movement guard holds a fall to 99 × the zone's gravity a step, 20
    steps a second (`movement::fall_speed`; 792 units a second at the
    default 0.4), on both client generations, from the zone header's
    gravity once it comes (`Wire::fall_speed` takes it). Derived, not read:
    one step per update until the update rate is chosen (checklist 6.294).
  - TAKP jumps: the `EQMac` notice (`quarm::jump`, no body) and
    `Capability::Jumping`.
  - `Anonymity` is the two-bit value both programs keep:
    `Anonymity::AnonymousRoleplaying` is new. `/roleplay` flips the
    roleplaying bit and keeps the anonymous one, and `/anonymous` writes 1,
    or 0 when already anonymous. Neither refuses while the other is on.
  - TAKP lists the player as anonymous or roleplaying
    (`listing::eqmac_anonymity`); `/afk` is refused there until the `EQMac`
    away update is read.
  - The Titanium show-helm option: `ClientCommand::ShowHelm` sends
    appearance type 43 on each change (`listing::titanium_show_helm`,
    `Capability::ShowingHelm`), offered on `EQEmu`.
  - TAKP answers translocates (`Capability::Translocate`).
  - Camping reports `CampStatus::Countdown` at 25, 20, 15, 10 and 5 seconds
    left; Titanium closes the zone connection 3 seconds after its logout
    (`Wire::logout_close`), and other generations keep waiting 10 seconds.

- From the official programs, read statically (TAKP Batch 1, Titanium
  sittings 5 and 6, and the Titanium login read L1 part one; servers'
  handling cited from `EQMacEmu` and `EQEmu`):
  - Titanium sends the player's own position at most once per 280 ms
    (`MotionSession::with_spacing`, `TITANIUM_POSITION_SPACING`); a sample
    accepted sooner is held and the latest goes out once the spacing
    passes. `EQMac` keeps its pace until its gate is read.
  - TAKP takes falls and their reports. `EQMac` reports the world's damage
    as `OP_EnvDamage` then `OP_Damage` with no source and no spell
    (`hazards::eqmac_notice`). `Capability::Jumping` is new and separate
    from `Capability::Falling`: TAKP falls without jumps until the `EQMac`
    jump notice is read.
  - Both generations send `OP_SafeFallSuccess` before a fall's damage when
    the player's Safe Fall lessened it; `ClientCommand::EnvironmentalDamage`
    gains `safe_fall`, and a report of 0 with it sends only the notice.
  - TAKP's bleed-out report carries type 23, the player's level and the
    player flag (`Request::BledOut { level }`, `quarm::bled_out`).
  - TAKP's merchant request carries only the merchant and the player; its
    buy and sale carry the count as 32 bits, and the sale names the
    player's own spawn (`ClientCommand::Sell` gains `own_id`).
  - TAKP sends `OP_AutoAttack2` before each change of auto attack, invites
    with `OP_GroupInvite2` when the player leads a group, and drops
    invisibility as the program does (`ClientCommand::DropInvisibility`,
    `buffs::InvisibilityStep`).
  - TAKP's word to go after a translocate asks with reason 4 when it names
    no caster, 8 when it does.
  - The session response's maximum packet size is read and written
    big-endian, as `EQEmu` writes it and the Titanium login program reads
    it; it read 512 as 131,072 before.
  - A Titanium login refused with `EQEmu`'s error string 105 reads as
    `LoginError::InvalidCredentials`, so a wrong password says so on
    `EQEmu` rather than that the login was rejected.

- `GameTime::hour` is documented as what it always held: hours since the
  day began, the wire's 1 to 24 less one, the calendar both generations
  keep. It no longer says it counts from midnight, since how a client names
  the hour is the client's: the Mac client shows the wire's hour as sent,
  24 as 12 AM, and turns the day as the wire goes from 24 to 1 (from the
  program, the official TAKP client read statically, report
  `program-read-takp-time-20261030`; recording checklist item 6.155), and
  the Titanium client's `/time` shows the wire's hour less one, 1 as 12 AM
  (from the program, the official Titanium client read statically, report
  `titanium-program-p0-time-20261030T2051`; items 1.2 and 6.115). The
  decoding is unchanged.

- `WorldEvent::CharacterCreation` says how the world took the character
  (`creation::CreationOutcome`: `Created`, `NameRefused`,
  `CreationRefused`, or `NotSent` when nothing went to the world) in place
  of `accepted`. Both generations' worlds refuse a name and a creation with
  the same one-byte reply, which the session now tells apart by when it
  comes; a creation refused after its name was approved read as a refused
  name before (seen on local TAKP).

- The session's spawn table keeps a character's new last name
  (`WorldEvent::LastName`, applied to the spawn of that name), which it
  passed over before.

- The session's spawn table keeps what each spawn's updates change: its
  look, size, `/who` listing and level (`SpawnState::apply_listing`), and
  whose pet it is, where before it kept the listing, the level and the
  owner from the spawn records alone. A charm's owner comes only by
  update, so the session refused pet commands for a charmed pet as if the
  player had none (found reading the code; not checked live). A host
  keeps a look or listing it holds apart from a spawn, as its own
  player's, the same way with `Appearance::apply_update` and
  `Listing::apply`, which keeps a guild rank only with a guild.

- The message tables say what each generation's profile leaves unread
  apart: `EQMac`'s skills, practice points and deity, which they had
  counted as read, are planned with `EQMac`'s catch-up on the world, and
  its profile has no leadership or tribute to list.

- The message tables no longer list Titanium's `OP_RezzComplete`, which
  `EQEmu` sends only to the world server, and list `EQMac`'s as TAKP's
  alone, as Quarm too sends it only to the world server; it is planned
  with `EQMac`'s catch-up on exchanges, since TAKP's client moves itself
  to the corpse on it. `EQMac`'s `OP_Translocate` names TAKP as a sender
  too.

- Quarm's move of a dead player to their bind point (`OP_GMGoto`) is read
  as a zone offer home. Quarm's session does not follow zones yet, so it
  ends there, as it does on Quarm's other moves, where before it stayed on
  in a zone the server had moved the player out of.
- An `EQMac` session no longer reads the client's own logout
  (`OP_Logout`), which neither TAKP nor Quarm sends; their logout reply
  still ends the session. Titanium's `OP_PreLogoutReply` and Quarm's
  shared bank offer are listed as left unread with nothing worth reading:
  the first is empty, and the message after it says what it begins; only
  Quarm's own client library answers the second.
- A world's word that the zone being entered is down (`OP_ZoneUnavail`)
  ends the attempt at once, on both generations, telling
  `WorldEvent::ZoneUnavailable` with the zone's short name where the world
  gives one, where before the world stage waited out its 60-second
  handshake. The delay before the next attempt still applies.
- Between zones, a Titanium world that names a character other than the
  one in play (`OP_EnterWorld`) ends the attempt, as an `EQMac` world's
  already did. From the character list, a name there is `EQEmu`'s
  automatic login: it is told as a diagnostic, and the session keeps to
  its own choice.
- The login loops tell once, as a diagnostic, of each message they leave
  unread and why, as the zone session does. `EQMac`'s `OP_ApproveWorld` is
  listed as left unread with nothing in it worth reading, since TAKP and
  Quarm always send it with an answer of 0 and no name.

- A zone session now tells, as a diagnostic, of each inventory update the
  zone sends once the player is admitted: what changed and in which slots
  ("Zone sent an inventory update: the world container emptied"), so a log
  shows the server's side of every change. A closed world container, whose
  `OP_ClearObject` left no trace before, now shows.

- A P99 login shows only P99's worlds from `EQEmu`'s public login server's
  list: those whose lowercased name starts with "project 1999" or "an
  interesting" (the "An Interesting Hole" test world), as the SSO login
  proxy keeps them. Other worlds are hidden, and a configured world the
  filter hides ends the login saying so. Every other server type is shown
  the whole list.

- Titanium door clicks now send 3 in the second byte and -1 as the held
  item, and eating or drinking by hand sends -1 in the request's third
  word, as the official client does (taken from #121). Eating on its own
  still sends 0 there, since what the official client sends then is
  unrecorded.

- A configured world that TAKP's list shows down or locked ends the login
  at once, as one on a Titanium list always has, instead of asking the
  login server for it.

- `Client::new` checks that a TAKP account and password fit `EQMac`'s
  19-byte login fields, as it already did for Quarm, instead of the login
  failing on them later.

- A coin move that would leave a place holding more than 2,147,483,647
  coins of a kind is refused before it goes out, on every server: TAKP
  kicks for it, and `EQEmu`'s count would wrap.
- `Message` gains `MerchantList`: a merchant's whole list, which replaces
  what it listed before, as `EQMac`'s comes. The feature keeping the
  merchant window tells the host the places gone and each item, as
  `MerchantUpdate`s, so a host sees no new event.
- Merchant trades, on every server: a purchase goes out only for an item
  on the open merchant's list whose most possible cost the purse covers
  (`EQEmu` charges what it lists; for a list quoted before the rate, a
  price a copper higher times the rate, rounded up), since TAKP logs a
  purchase it refuses for want of coins as a possible hack; a NO DROP item
  is refused before a sale goes out, as both servers ignore it without a
  word; a sale's echo adds its price to the purse, platinum first (TAKP
  sends no money update after a sale, and `EQEmu`'s replaces the purse with
  the same coins); and an echo of nothing bought ends a pending purchase as
  refused at once (TAKP's answer to a refused purchase; `EQEmu` sends none).
- `InventoryUpdate` gains `Used`: a unit or charge the server used up
  without saying which, as TAKP's `OP_DeleteCharge` says it. Hosts that
  match `InventoryUpdate` exhaustively need an arm for it.
- A dead player on TAKP asks for their bind point once a death pause is
  over, one value in the transfers feature, which is zero: at once, as
  Adam chose. The official client's wait is unrecorded (inferred). A death
  while a transfer is under way waits for its answer, and a second word of
  the same death changes nothing.

- A merchant's echo settles only the trade it answers. Echoes carry no
  request id, so each is matched by its slot, and a sale's echo removes the
  units only while the slot still holds the item that was offered, less what
  earlier echoes removed from it. A sale released unanswered after the
  three-second hold is remembered, so its echo still settles it if it comes
  late; the release itself stays, because `EQEmu` answers a refused offer
  with silence. An echo for an item the slot no longer holds, or one that
  answers no trade, leaves the inventory untrusted
  (`InventoryUpdate::Invalidated`) instead of removing what the slot holds
  now, and a purchase's echo releases only its own purchase.
- `Message` gains `PurseAdded`: coins the server added to the player's
  purse without saying what the purse holds, as TAKP's money notices do.
  The feature keeping the purse applies it, and a host hears the purse.

- An item move from anywhere but the cursor itself waits for a cast to
  end, exactly as `EQEmu` and TAKP require: both disconnect a player who
  makes one during a cast that is not a bard song
  (`Client::Handle_OP_MoveItem`, "Inventory desync"). The session let the
  numbers 30 to 39 through, a range in which a move can only name the
  cursor, so no move it sends changes. Both servers let a bard move items
  while singing, but the session cannot tell a song from a spell, so a
  bard waits too. That the official client refuses such a move, rather
  than interrupting the cast, is inferred.

- `WorldEvent::HitPoints` says which values leave out the HP equipped items
  add with `items: ItemHitPoints`, in place of `without_items`. Titanium's
  own update leaves them out of both (`LeftOut`). `EQMac`'s, as TAKP sends
  it, leaves them out of the current alone and counts them in the maximum
  (`LeftOutOfCurrent`), so a host adds item HP back to the current only.
  It was read as counting them in both, which showed the player's HP low
  whenever gear adds HP. The health percent that `EQMac`'s update also
  gives is right for others, but for the player it leaves item HP out, so
  a host should take the player's health from the hit points.

- Zoning departs as the official client does. Once a zone approves a
  transfer, the session sends `Request::SaveOnZone` and then
  `Request::Depart` (`OP_SaveOnZoneReq`, then `OP_DeleteSpawn` for the
  player's own spawn), and goes on to the world server when the zone
  answers with a logout, the connection ends, or two seconds pass
  (inferred). A repeated answer while the player departs only goes into the
  diagnostics: `EQEmu` answers a move to another zone twice, and TAKP
  answers a repeated request again. `Message::ZoneAnswer` now carries a
  `zoning::ZoneAnswer`, read per client generation, and the zone session's
  features hear when the connection ends (`Feature::connection_ended`).
- TAKP lists zoning: `EQMac`'s zone change (76 bytes, no position), its
  answer, the zone points, the save and the departure, and the world
  stage's re-entry between zones (the login's zoning flag, then entering
  the character the world names).
- `EQMac` deaths: `OP_Death` (20 bytes) is read as `WorldEvent::Death`,
  and the profile's first bind point as the new `Message::Bind`. A dead
  player on TAKP asks for their bind point at once (the zone change for the
  bind zone, with reason 10, `ZC_RepopToHomeAtDeath`), since TAKP holds
  the move home until the client asks and removes a dead client that never
  does; Titanium's servers still offer it (`transfers::Home`).
- A tell's echo on channel 14 is `ChannelName::TellEcho` on the `EQMac`
  wire too: TAKP echoes a delivered tell to its sender on
  `ChatChannel_TellEcho` (14), as `EQEmu` does on Titanium's.

- `EQMac`'s time of day: `OP_TimeOfDay` (0xf240, 6 bytes with a 16-bit
  year) is read as `WorldEvent::TimeOfDay` (`clock::decode_eqmac`), and
  TAKP lists the clock, which keeps the time the zone sends while
  admitting the player until the host can hear it.

- `WorldEvent::Entered` gains `choices`: what the session leaves to the
  player, none of it among `capabilities`. A server type may leave any
  feature it provides to the player where its own client keeps it off; a
  front end then offers what the feature lets the player do only once the
  player turns it on. P99 leaves the in-game map (`Capability::Map`) to the
  player this way, as its own client keeps the map off.

- P99 offers `Capability::MovingSpells`: moving a spell in the book works
  there as on `EQEmu`, checked with the official client. Deleting one does
  not, so `DeletingSpells` stays off on P99.

### Added

- What the world is told of the machine it is entered on, on Titanium:
  `sysinfo::SysInfo` and its `encode` build the 344-byte `OP_SystemInfo`
  (0x1C4C) record, and a world connection sends it just before entering
  the world, once a week (`sysinfo::REPORT_SECONDS`), while its server
  type is listed for it (`servers.rs` `sends_system_info`, watched on
  P99; silent everywhere else until watched). `ClientConfig::sysinfo`
  holds the host's `sysinfo::Machine`: its saved stamp, and the machine
  (processor, memory, display, adapter) measured when a report is due,
  just before it goes out. It is none by default, which keeps a session
  silent as it always was. The engine's version and flags are
  constants from the captures, the dwords at 188 and 192 go out zero as
  both captures show, and the operating system is Windows XP Service
  Pack 2 by Adam's choice. The session tells the host when it spoke
  (`WorldEvent::SystemInfoReported`) so the host can keep the stamp that
  decides the next week. Hosts that match `WorldEvent` exhaustively need
  an arm for it. Based on eq-network #150.

- What game masters and the world tell the zone of a player, on both
  generations: `WorldEvent::Gm` (`gm::GmNotice`). `Kicked` and `Killed`
  name the player and the game master, None from the world, from
  `OP_GMKick` and `OP_GMKill`: the world's reach only the player they name,
  and a game master's `/kick` or `/kill` of a player in the zone reaches
  everyone in it, the server leaving the named player's client to leave
  or die. `BecameNpc` carries the highest level of player the server lets
  attack the player, from `OP_GMBecomeNPC`, a game master's `/becomenpc`.
  On `EQMac`, `Petition` carries a petition in the queue
  (`gm::PetitionEntry`), or None for the empty queue's record, from
  `OP_PetitionRefresh`, which TAKP and Quarm send game masters; its time
  is read at either width a server's own time has. Nothing acts on a
  notice yet. The message tables list `EQMac`'s `OP_PetitionCheckout` and
  `OP_SoulMarkUpdate` as answers to requests eq-network never sends.
  Hosts that match `WorldEvent` exhaustively need an arm for it.

- `EQMac`'s renames of a player by a game master (`OP_GMNameChange`) read
  as `WorldEvent::Renamed`, which the session names from the spawn with
  the old name, as it does Titanium's. After TAKP's and Quarm's
  `#set name`, both names are the new one, and the player returns under
  it.

- Titles, on both generations: `WorldEvent::NewTitles`, from Titanium's
  `OP_NewTitlesAvailable` and `EQMac`'s empty `OP_SetTitle`, which the
  servers send as the player earns titles to choose from; and
  `WorldEvent::TitleChanged` (the spawn and a `names::TitleChange`), from
  Titanium's `OP_SetTitleReply`, which `EQEmu` sends the whole zone with a
  character's new title or suffix as it is set or cleared, and `EQMac`'s
  8-byte `OP_SetTitle`, which names the character but says neither which
  changed nor to what (`TitleChange::Unspecified`). The session's spawn
  table keeps a title or suffix in the spawn's name parts. The message
  tables list Titanium's `OP_SendTitleList` as an answer to a request
  eq-network never sends. Hosts that match `WorldEvent` exhaustively need
  arms for them.

- Guilds: `WorldEvent::Guild` (`guild::GuildUpdate`). On Titanium, the
  member list reads as `GuildUpdate::Members` (`guild::GuildMember`: each
  member's name, level, class, rank, banker and alt flags, when they were
  last on, their tribute as `guild::GuildTribute`, their public note, and
  their zone, None while offline), from `OP_GuildMemberList`, which
  `EQEmu` sends as the player enters a zone, when their list is out of
  date, as they join and as they reload their interface. Members' changes
  read as `MemberLocation` (a member entering a zone or leaving the game),
  `MemberAdded`, `MemberRemoved`, `MemberLevel`, `MemberNote`, `MemberRank`
  (a member's rank and banker and alt flags, or in the guild hall the
  player's own banker flag) and `Disbanded`, from the messages `EQEmu`
  sends the guild's members in the zone as the world tells of each; and
  `MemberRenamed` and `Renamed`, from messages nothing in `EQEmu` sends
  today. On both generations, an invitation to join the guild, or to take
  a higher rank in it, reads as `Invite` (the inviter, the guild and the
  rank offered), from `OP_GuildInvite`, which every server passes on from
  the inviter's client; nothing answers it yet. On `EQMac`, a member's
  removal reads as `MemberRemoved`, from `OP_GuildRemove`, which TAKP and
  Quarm send only when a removal fails, with a line saying it succeeded.
  The session keeps no roster. The message tables list Titanium's
  `OP_GetGuildMOTDReply` and `EQMac`'s `OP_GuildAdded` as answers to
  requests eq-network never sends. Hosts that match `WorldEvent`
  exhaustively need an arm for it.

- Duels, on both generations: `WorldEvent::Duel` (`socials::DuelUpdate`).
  `Challenge` names the other player, from `OP_RequestDuel`, which every
  server sends to the player challenged and, as their acceptance, to the
  challenger; `Accepted` names the challenger and the one who accepted,
  from `OP_DuelAccept` (Quarm's `OP_DuelResponse2`), which `EQEmu` and
  Quarm send both players as a duel begins; and `Refused` names the other
  player and why (`socials::DuelRefusal`: declined, dueling already, or
  considering another challenge), from TAKP's `OP_DuelResponse`, which
  passes on a client's answer turning a challenge down. Nothing answers a
  challenge yet. Hosts that match `WorldEvent` exhaustively need an arm for
  it.

- What other players do to or for the player, on both generations: a yell
  for help near the player (`WorldEvent::YellForHelp`, the yeller's spawn),
  from `OP_YellForHelp`, which every server passes to those within 100 of
  the yeller; and a request to inspect the player
  (`WorldEvent::InspectRequest`, the inspector's spawn), from
  `OP_InspectRequest`, which every server passes on for the player's client
  to answer. Nothing answers it yet, so the inspector still sees no window.
  On Titanium, a voice macro spoken to the player, their group or their
  raid (`WorldEvent::VoiceMacro` with `socials::VoiceMacro`: the speaker, to
  whom as `socials::VoiceKind`, the voice and the macro's number), from
  `OP_VoiceMacroOut`. Hosts that match `WorldEvent` exhaustively need arms
  for them.

- Popups and marquees on Titanium: `WorldEvent::Popup` (`popups::Popup`:
  the title, the text in the client's own markup, the buttons, how long it
  stays open, None until the player closes it, and its number), from
  `OP_OnLevelMessage`, which `EQEmu` sends for quests and game masters'
  commands such as `#show version`; and `WorldEvent::Marquee`
  (`popups::Marquee`: the message type that colours it, its priority, how
  long it takes to fade in and out and how long it shows, and its text),
  from `OP_Marquee`, which `EQEmu` sends for quests, bots, a spell effect
  and the world's broadcasts. Nothing answers a popup yet. Hosts that
  match `WorldEvent` exhaustively need arms for them.

- Coloured text and `EQMac`'s spell and world lines, read as chat on
  `ChannelName::System`, where before the session dropped them. On
  Titanium, `OP_ColoredText`, with its colour in `ChatEvent::message_type`
  (below 256 for the consider colours), which `EQEmu` sends for quests,
  auras and expeditions and as a buff's fade line. On `EQMac`,
  `OP_SpellTextMessage`, a spell line another player's client sends, which
  TAKP and Quarm pass on to those near it; TAKP's
  `OP_SpellTextMessageStringID`, another caster's interruption or regained
  concentration by the official client's string number (`string_id`), with
  the caster's name as its argument; and `OP_MultiLineMsg`, the world's
  text for one player, which nothing in TAKP's or Quarm's world sends
  today. The spell lines name the spawn they are about in a new field,
  `ChatEvent::source_spawn_id`.

- Effects the servers play: `WorldEvent::Effect` (`effects::Effect`). On
  Titanium, a spell's nimbus on a spawn (`Effect::Nimbus`: the effect, the
  spawn it plays from and the one it is drawn on, how long it plays and
  how long its last part waits), from `OP_SpellEffect`, which `EQEmu`
  sends to the zone as a spell with one lands, to a player zoning in for
  each one a spawn carries, and for quests; particles on a spawn
  (`Effect::Particles`: up to five `effects::Particle`s, each an effect and
  the two values sent with it, kept as sent since `EQEmu`'s comments
  disagree on what they do), from `OP_LevelAppearance`, which `EQEmu`
  sends as an NPC rises in level and for the effects it keeps on a spawn;
  a camera shake (`Effect::CameraShake`: how long and how hard), from
  `OP_CameraEffect`, which a game master's `#camerashake` and quests send;
  and a sound file to play (`Effect::Sound` with `effects::Cue::File`),
  from `OP_PlayMP3`, which only quests send. On `EQMac`, a sound to play
  by number (`Cue::Number`), from `OP_PlaySound`, which a game master's
  `#playsound` and quests send. On both, a projectile (`Effect::Projectile`
  with `effects::Projectile`: the spawn it flies from and the one it flies
  at, where it starts, its velocity, its launch angle in 512ths of a turn
  on both wires, its tilt and arc, the item, its type, the skill and the
  model, and on `EQMac` its light and behavior), from Titanium's
  `OP_SomeItemPacketMaybe` and `EQMac`'s `OP_Projectile`, which the
  servers send to the players near an arrow, a thrown item, an NPC's
  ranged attack or a spell's bolt (TAKP and Quarm send a bolt to the whole
  zone). Hosts that match `WorldEvent` exhaustively need an arm for it.

- Every kind of appearance update on a spawn, on both generations:
  `WorldEvent::Appearance` (`appearance::AppearanceChange`), from the kinds
  of `OP_SpawnAppearance` the session dropped: PvP, light, sneaking,
  link-dead, fly mode (`appearance::FlyMode`), size, a player's name in an
  NPC's color, an NPC's tint, show helm, a destructible NPC's damage state,
  an area's regeneration, which comes to the player alone, kind 0 with its
  value (`EQEmu`'s note says it makes the client keel over and go to its
  bind point; unverified), and, as `Other`, the kinds no server sends on
  its own, which a game master's `#appearance` can. A guild rank joins the
  listing: `ListingChange::GuildRank` and `Listing::guild_rank`
  (`listing::GuildRank`), from the update and from Titanium's spawn record.
  A record naming no spawn is still left to the admission and Quarm's
  handshakes, and a `/who` level past 255 is now the usual
  unreadable-packet diagnostic, where before it was dropped. Hosts that
  match `WorldEvent` or `ListingChange` exhaustively need arms for them,
  and hosts that build `Listing` the field.
- How a spawn shows beyond its gear, in `Appearance`: `light`,
  `fly_mode`, `pvp`, `linkdead`, `sneaking`, `npc_name_color`, `npc_tint`
  and `damage_state`, from Titanium's spawn record (light and fly mode)
  and `EQMac`'s (PvP, light and fly mode) and kept current from appearance
  updates by `SpawnState::apply_appearance`, which takes a new size too.
  `EQMac`'s record marks every NPC link-dead and never marks sneaking, so
  those two come from updates alone. Hosts that build `Appearance` without
  `..Appearance::default()` need the fields.
- What a spawn holds or suffers that shapes how it stands and moves, on
  Titanium: `Appearance::states`, a set of `EQEmu`'s bits
  (`appearance::SpawnStates`: a weapon in either hand, an instrument,
  sheathed, aggressive, stunned), from the spawn record and from
  `OP_PlayerStateAdd` and `OP_PlayerStateRemove`, read as
  `AppearanceChange::StatesAdded` and `StatesRemoved`. `EQEmu` sends them
  to the zone as a stun lands and ends, as an NPC turns aggressive or
  takes up a looted weapon, and as another player says theirs changed.
- Illusions and face changes, on both generations: a spawn taking on
  another look, or its own again, as `AppearanceChange::Illusion`
  (`appearance::Illusion`: the race, gender, body and helm textures, none
  for a player race, whose gear shows, the size, 0 for the race's own,
  and the face and hair as `appearance::Features`), from `OP_Illusion`;
  and a face or hair change as `WorldEvent::FaceChanged`, from `EQEmu`'s
  illusion of race -1 on Titanium, which names the spawn, and `EQMac`'s
  `OP_FaceChange`, which TAKP and Quarm relay from a player to the whole
  zone and which names no one but has both eyes' colors. The session's
  spawn table takes an illusion's race, gender, size, face and hair, and a
  named face change's face and hair. Hosts that match `WorldEvent`
  exhaustively need an arm for `FaceChanged`.
- A spawn's new name, on Titanium: `WorldEvent::Renamed` (the old name
  and the new), from `OP_MobRename`, which `EQEmu` sends as an NPC's
  corpse takes the server's name, from the one the client gives it, and
  for the temporary names and renames of game masters, quests and pets'
  owners. The record names no spawn: the session names it before anyone
  hears it, from the spawn with the old name or the NPC corpse whose name
  by `world::corpse_name` is the new one, and renames it in its table.
  And a corpse's move, on `EQMac`: `WorldEvent::CorpseMoved` (the corpse
  and where it lies, without a heading), from `OP_CorpsePosition`, which
  TAKP and Quarm send as a corpse is summoned, returned or moved by a game
  master, and on Quarm as it is dragged; the session's table moves it and
  keeps its heading. Hosts that match `WorldEvent` exhaustively need arms
  for both.

- A quest's word on the player's base stats, on Titanium:
  `WorldEvent::StatsChanged` (`stats::StatChange`: each stat by name, 0
  for one the quest left alone), from `OP_IncreaseStats`, which `EQEmu`
  sends for a quest's `setstat` and `incstat`. A value is the stat's new
  value from the first and its rise from the second, which the record does
  not tell apart, so the session changes nothing on it; the next profile
  has the stats as the server keeps them. Hosts that match `WorldEvent`
  exhaustively need an arm for it.
- A skill's button ready again, on `EQMac`: `WorldEvent::SkillReady` (the
  skill's number), from `OP_ResetSkill`, which TAKP and Quarm send for
  Mend when the player pressed it too soon after a zone, since the official
  client then starts its full wait anew, and for each skill with a
  recovery timer on a GM's `#cleartimers`. Hosts that match `WorldEvent`
  exhaustively need an arm for it.
- The player's disciplines and reuse timers. On Titanium: the disciplines
  the player knows (`WorldEvent::Disciplines`: a spell ID in each of the
  list's 100 slots, 0 for an empty one), from `OP_DisciplineUpdate`, which
  `EQEmu` sends once the zone has admitted a player who knows any and as
  the list changes; a discipline reuse timer (`WorldEvent::DisciplineReuse`,
  a `disciplines::Reuse`: the timer and how long until it is ready), from
  `OP_DisciplineTimer`, whose copies at zone-in of timers 11 to 19 carry
  `EQEmu`'s other reuse timers; and a reuse timer a group of spells shares
  (`WorldEvent::LinkedReuse`), from `OP_LinkedReuse`. On `EQMac`: the
  discipline the player began, or none as it ends
  (`WorldEvent::Discipline`), from `OP_DisciplineChange`. Hosts that match
  `WorldEvent` exhaustively need arms for the four.
- The player's languages, on both generations: `PlayerState::languages`,
  each language's skill by its number, from the profile (Titanium's 28,
  and the 26 TAKP and Quarm fill of `EQMac`'s room for 32), kept current
  from skill updates for skill 100 and up, which every server sends for a
  language (`world::LANGUAGE_SKILLS`); `EQMac`'s skill updates wait for
  its catch-up on the world. On Titanium, the profile's list of
  the disciplines the player knows: `PlayerState::disciplines`, which
  `EQEmu` leaves empty in the profile and the session keeps current from
  `WorldEvent::Disciplines`. Hosts that build `PlayerState` need the two
  fields.

- The player's target as the server sets it, on both generations:
  `WorldEvent::Target`, from `OP_TargetCommand` (`EQEmu`'s sense spells,
  its corpse locator and Perl; TAKP's and Quarm's refusal of a mouse
  target, which puts back the last one) and from TAKP's and Quarm's echo
  of the player clearing their target (`OP_TargetMouse`, now listed); the
  session's target follows it. On Titanium, whom the player's target
  targets: `WorldEvent::TargetOfTarget`, from `OP_TargetHoTT`. Hosts that
  match `WorldEvent` exhaustively need arms for both.
- What a quest or a task gives the player, on both generations:
  `WorldEvent::Reward` (`rewards::Reward`: the giver, the coins, the
  experience and the items), from `EQEmu`'s quest reward, which rides
  `OP_Sound`, and its coins alone (`OP_CashReward`, from a task or a
  quest's `givecash`), and from TAKP's and Quarm's `OP_Reward`. Every
  server adds the coins to the purse without a money update, so the
  session adds them to the purse it keeps and tells the host the purse
  they make (inferred: the official client's purse after a reward is
  unrecorded). `EQEmu`'s fixed `OP_Sound` record, which gives nothing, is
  `WorldEvent::Ding`: the sound the official client plays for a quest's
  `ding`. Hosts that match `WorldEvent` exhaustively need arms for both.
- Offers the player answers, on both generations, answered only on the
  player's word: a sacrifice (`WorldEvent::SacrificeOffer`, naming its
  caster) from `OP_Sacrifice`, answered with `GameCommand::AnswerSacrifice`,
  and a translocate (`WorldEvent::TranslocateOffer`: the caster, the spell
  and where it takes the player) from `OP_Translocate`, answered with
  `GameCommand::AnswerTranslocate`. The answer repeats the offer with the
  player's choice: 1 for yes, and 0 for no, but -1 for a translocate on
  `EQMac` (inferred: what the official clients answer is unrecorded). An
  answer with no offer waiting is refused (`WorldEvent::SacrificeRefused`,
  `WorldEvent::TranslocateRefused`). `EQEmu` offers both
  (`Capability::Sacrifice`, `Capability::Translocate`); P99, TAKP and Quarm
  wait for their live checks. TAKP answers a yes to a translocate with the
  offer again, which reads as the server's offer to move the player there;
  it carries no heading, so the player faces heading 0 after it, and the
  transfer request that follows gives reason 8 (both inferred). Hosts that
  match `WorldEvent` exhaustively need arms for the four events.
- A player becoming a corpse, on Titanium: `WorldEvent::BecameCorpse` (the
  spawn, which the corpse keeps, and where it lies), from
  `OP_BecomeCorpse`, which `EQEmu` sends the player just before word of
  their death, and everyone else in the zone as a player leaves a corpse.
  Hosts that match `WorldEvent` exhaustively need an arm for it.
- Each character a world lists carries how it looks
  (`CharacterChoice::look`, a `CharacterLook`): race, gender, class,
  deity, its worn gear as a spawn's `Appearance` (armour materials and
  tints, the held items' model numbers, face, hair and beard), eye colours
  and, on Titanium, whether the world offers Go Home and the tutorial,
  read from both generations' lists at the emulators' layouts. A server
  type gives them only where its list is checked to carry them
  (`EQEmu` and TAKP); elsewhere `look` is `None`, and an `EQMac` list's
  level and zone stay `None` as before.

- The server's hold on the player, on both generations, read from the
  emulators' sources and not yet checked live: a stun
  (`WorldEvent::Stun`: its length, or none as one ends) from `OP_Stun`; a
  player's charm taking hold or ending, and the server giving a player
  back the control it took for a charm or a fear (`WorldEvent::Charm`, a
  `control::Charm`), from `OP_Charm`; and on `EQMac`, the server taking
  the player's control and giving it back (`WorldEvent::Controlled`) from
  `OP_FreezeClientControl`, `OP_UnfreezeClientControl` and
  `OP_ReturnClientControl`, and a warrior going berserk and calming down
  (`WorldEvent::Berserk`) from `OP_BerserkStart` and `OP_BerserkEnd`, which
  the table now lists. `EQEmu` tells a Titanium client of its control
  with the player's own posture, which the session already reads
  (`PostureState::Frozen`). Hosts that match `WorldEvent` exhaustively
  need arms for the four.
- The push a hit or a spell gives, on both generations: `Damage` and
  `SpellEffect` gain `push` (`combat::Push`: the force, the heading in
  `Position`'s 512-unit turn and the pitch, as the server sends them),
  none when the server sends no force and no pitch. Nothing applies it
  yet. `Damage` and `SpellEffect` no longer implement `Eq`, since the push
  is made of floats. A push that is not a finite number reads as none, and
  the record carrying it still reads.
- The spin a quest gives the player, on both generations:
  `WorldEvent::Spun`, from `Mob::Spin`'s fixed record in `OP_Action`, which
  the session now reads whole. `buffs::Action` says what an action is (a
  spell's, the spin, or another kind), from `buffs::titanium_action` and
  `buffs::eqmac_action`; `message::titanium` and `message::eqmac` read
  `OP_Action` instead of `world::titanium_update` and `quarm::updates`.
  No server is known to send an action of another kind: `Message` gains
  `UnknownKind`, which the zone session drops, telling of each message's
  kind once a session as a diagnostic that names only the opcode and the
  kind. Hosts that match `WorldEvent` exhaustively need an arm for `Spun`.
- The server's end of the player's hiding and sneaking, on both
  generations: `WorldEvent::HideAndSneakEnded`, from `OP_CancelSneakHide`.
  `Request` gains `Unhide`, which `abilities::titanium_unhide` encodes.
  Hosts that match `WorldEvent` exhaustively need an arm for it.

- The zone's weather, on both generations: `WorldEvent::Weather`
  (`clock::Weather`: clear, or rain or snow with the server's intensity),
  once the player is in the zone and whenever it changes.
- The rest of the zone's header: `WorldEvent::ZoneDescription`
  (`zones::ZoneDescription`: the long name, the zone's type, gravity, the
  safe point, `max_z`, the underworld height, the nearest and farthest
  clip, the experience multiplier and four sets of weather odds, with
  `EQMac`'s music, sky lock and graveyard time), right after the zone's
  sky, and both again whenever the server sends the header anew, as GM
  commands and quests that change the zone do. `EQMac` sessions now read
  the sky, fog and far clip from the zone header as Titanium's do, so
  `WorldEvent::Sky` and `Entered.far_clip` come on TAKP and Quarm too.
  Hosts that match `WorldEvent` exhaustively need arms for both new
  events.

- What a world tells of itself as the player logs in, on both
  generations: its rules (`WorldEvent::Rules`, a `rules::WorldRules`: the
  player-versus-player ruleset by the server's number, whether no-drop
  items trade, and each generation's own flags) from `OP_LogServer`, which
  Quarm's zones also send, with the zone's own ruleset, at every zone-in;
  the expansions it lets the account play (`WorldEvent::Expansions`, a
  `rules::Expansions` mask with `includes`) from `OP_ExpansionInfo`; and
  its chat server (`WorldEvent::ChatServer`, a `channels::ChatServer`:
  channels or mail, the host and port, and the world and character the
  chat server knows the player by) from `OP_SetChatServer` and Titanium's
  `OP_SetChatServer2`, which `EQEmu`'s zones send again when their chat
  server reconnects. The offer's key is never read into a value, so
  nothing told, kept or recorded carries it, and nothing connects to the
  chat server. `EQMac` sessions now tell `WorldEvent::WorldName` too, from
  their world's `OP_LogServer`. Hosts that match `WorldEvent` exhaustively
  need arms for these and for `ZoneUnavailable`.

- A record of every message each client generation's servers send: one
  table per generation, one row per opcode, naming the message, the server
  types that send it and the parts of the session that read it, or why it
  is left unread (work still to come, an answer to a request eq-network
  never sends, parked work, or nothing in it worth reading), taken from
  `EQEmu`'s, TAKP's and Quarm's sources: Titanium's 258 messages, 156 of
  them read, and `EQMac`'s 177, 110 read. A zone session now tells once, as
  a diagnostic, of each message it leaves unread with the reason (but one
  with nothing worth reading, and an empty one under opcode 0, where
  `EQEmu` sends what Titanium has no opcode for, such as the empty
  OP_FinishWindow pair after a container closes or a trade ends) and of
  each the table does not list, where
  before a message nothing read left no trace once the zone admitted the
  player. A test holds each table to its generation's decoders over all
  65,536 opcodes.

- `SpawnState` gains `walk_speed` and `run_speed`: the spawn record's walk
  and run speeds, server protocol values as `PlayerState`'s are, read from
  Titanium spawn records at the offsets the player's own record uses. A
  bad value is zero rather than dropping the spawn; `EQMac` spawns report
  zero until their record's offsets are checked.
- `world::ANIMATION_UNITS_PER_SECOND` is public: the units a second one
  speed value moves, so a host can turn a record's speed into units.

- `WorldEvent::Entered` gains `spell_landing_flag`: the effect flag of the
  spell action that says a spell landed on its target, so a front end can
  draw the landing. `EQEmu` sends 4 for each target (`Mob::SpellOnTarget`);
  P99, Quarm and TAKP say None until each is checked (TAKP sends 4 only once
  a buff takes hold). Front ends that build the event add the field.

- Swings and the other one-shot motions the server asks clients to play:
  `WorldEvent::Animation` (`combat::Animation`: the spawn, the servers'
  animation number and the speed as a multiple of normal), from Titanium's
  4-byte `OP_Animation` (`combat::animation`) and `EQMac`'s 12-byte one
  (`combat::eqmac_animation`), read from `EQEmu`'s and TAKP's source and not
  yet checked live. Both servers send one to every client near a spawn as
  it swings, kicks or uses a monk attack, the player's own included. It
  sends nothing and gives the player nothing to do, so it is read as
  `OP_Damage` is: by each client generation's wire, for every server that
  speaks it. The speed scale (Titanium's byte over 10, `EQMac`'s float as
  sent) is inferred from the servers' defaults. A record of the wrong
  length, an `EQMac` action outside 0 to 65535 or an `EQMac` speed that is
  negative or not a number is the usual unreadable-packet diagnostic.
  `Animation` is a new variant of the exhaustive `WorldEvent`.

- Groups on TAKP (`Capability::Grouping`), read from EQMacEmu's source and
  checked live with two characters. The `EQMac` client's group packets have Titanium's
  layouts with their own opcodes (`group::eqmac_invite`, `eqmac_follow`,
  `eqmac_decline`, `eqmac_disband` and `decode_eqmac`); its invitation is
  193 bytes, the names and 65 more (zero, inferred). TAKP also has its own
  ways: it removes a player from a group with a quiet update naming them,
  read as `GroupUpdate::Left`, and lists the player's group in the profile
  (`quarm::profile_group`) as each zone admits them, with the leader named
  afterwards. The session tells the host a group listed before the
  admission once the zone admits the player.
- The login server's list, for the player to choose a world from. When no
  world is configured (`ClientConfig::server` empty, which needs a command
  queue), both client generations' logins send `WorldEvent::ServerSelection`
  with each world's name, whether it is up, down or locked, its players
  where the list gives them, and whether the list marks it preferred
  (`servers::ServerChoice`), then play on the world the player picks with
  `ClientCommand::SelectServer`. A world that is down or locked can't be
  picked. A world the login server refuses comes back as
  `WorldEvent::ServerRefused`, with Titanium's login string id or TAKP's
  own words (`servers::ServerRefusal`), and the list stays up for another
  choice. The login's 45-second limit pauses while the player chooses, as
  the world's does at the character list, and records name the world
  chosen. TAKP's list is read for each world's status and players as
  EQMacEmu's login server writes them (`ServerStatus::eqmac`), inferred
  until checked on a TAKP server. `ServerSelection`, `ServerRefused` and
  `SelectServer` are new variants of the exhaustive `WorldEvent` and
  `GameCommand`.

- Eating and drinking on TAKP: `food::eqmac_nourishment` reads TAKP's
  5-byte `OP_Stamina` (food and water as 16 bits, then a fatigue byte), and
  `food::eqmac_consume` builds its 16-byte `OP_Consume` in `EQMac`'s slot
  numbers, with the -1 TAKP's struct says the official client sends. TAKP's
  session eats and drinks on its own once food or water falls below 3000,
  where TAKP's own client does (`food::TAKP_HUNGRY`, inferred from TAKP's
  `Client::Hungry`; `EQEmu`'s at 3000), and by hand.

- Inspecting linked items on TAKP: `GameCommand::InspectItem` on the
  `EQMac` wire sends `items::eqmac_request`'s 66-byte request
  (`OP_ItemLinkResponse`: the item as 16 bits from the link's six decimal
  digits, then 64 bytes for a name TAKP does not read), refusing say links
  (IDs above 0x8000), for which TAKP has the player say the link's phrase.
  The answer, the item's 360-byte record, is `WorldEvent::ItemDetails`
  (`items::eqmac_response`).
- Coin moves on TAKP, in its own packet (`money::EQMAC_MOVE_OPCODE`,
  `CoinTransfer::encode_eqmac`): the same five signed words as Titanium's,
  between the purse, the cursor, the bank beside a banker and the trade
  window, converting as the server does (11 gold into platinum takes 10 and
  adds 1). TAKP answers a move no more than `EQEmu` does, so the purse,
  cursor and bank change when the move is sent, and the next profile
  corrects them; coins still never leave the trade window.
- Merchants on TAKP (`Capability::Trading`, beside its item moves; coin
  moves and meals wait, as the inventory feature's allowances for the
  server type say): buying and selling through TAKP's own packets. TAKP's
  packets price trades before the merchant's rate
  (`merchant::Quotes::BeforeRate`): the session tells each listed item at
  the price TAKP charges, `int(price x rate)` a unit, and prices a sale's
  echo, which carries no price, as TAKP added it to the purse,
  `int(price / rate + 0.5)` a unit of the item the sale offered
  (`Quotes::sale_price`, inferred), before the ledger and the host hear
  them; an echo answering no sale the session remembers adds nothing, and
  a list arriving with no window open is dropped. TAKP's whole list
  (`merchant::eqmac_list`) replaces the last, as `Message::MerchantList`;
  the session tells the host the places gone, then each item.
  `MerchantOffers` stays off on TAKP, whose rule for what a merchant pays
  is its own. Carries #104 until it merges.
- TAKP's merchant packets, read and built as the merchant news and requests
  Titanium's give, for the same merchant feature (nothing offered yet):
  `merchant::eqmac_request`, `eqmac_end`, `eqmac_buy` and `eqmac_sell`
  build `EQMac`'s 12-, 4- and 16-byte requests (`GameCommand::Shop`, `Buy`
  and `Sell` on the `EQMac` wire), a sale naming the slot in `EQMac`'s
  numbers; `merchant::decode_eqmac` reads the open answer and its rate,
  the whole compressed list (each item through the `EQMac` item reader, its
  price before the merchant's rate and no count), a place gone from the
  list, the purchase and sale echoes (a refused purchase echoed with
  nothing bought, a sale's echo pricing nothing, inferred) and the window's
  closing; an open answer's rate that is not finite and above 0 is refused. The
  unknown bytes, prices and player IDs TAKP never reads go as 0, and the
  open request's rate as 1 (inferred).
- Moving items on TAKP (`Capability::Inventory`, with no coins, merchants
  or meals yet): `inventory::eqmac_move` encodes `EQMac`'s 12-byte
  `OP_MoveItem` in `EQMac`'s slot numbers (`InventorySlot::to_eqmac`), and
  `Inventory::plan_move_for` plans a move under a server type's
  `MoveRules`. TAKP's keep bags, books and arrows out of more worn slots
  and an instrument from either hand while the other is full, have no
  charm slot, and make a move onto a cursor an unsettled move emptied wait,
  as TAKP may refill it from a queue it said nothing of. TAKP answers a
  move it refuses at once with a resync of its two slots, disconnecting a
  player below status 10, and answers no move it accepts, so its session
  moves one item at a time, stricter than the official client: each move
  holds the inventory until it settles. `Inventory::plan_move` keeps
  planning under `EQEmu`'s rules.

- TAKP's inventory: `inventory::decode_eqmac` reads `EQMac`'s item packets
  as TAKP sends them into the same `InventoryUpdate`s as Titanium's: the
  full inventory (`OP_CharInventory`: 360-byte item records, compressed
  with zlib behind a count that does not count them), an item put in a
  slot (`OP_MerchantItemPacket`, which TAKP sends for every item it
  places), an item onto the cursor (`OP_SummonedItem`), an emptied slot
  (`OP_MoveItem`), and a unit or charge used up (`OP_DeleteCharge`), which
  is the new `InventoryUpdate::Used`, as `EQMac` servers do not say whether
  a stack unit or a charge went. Each item carries its statistics, worn
  effect and level rules, so a host can count what TAKP's equipped items
  add. `InventorySlot::from_eqmac` gives `EQMac`'s slots Titanium's
  numbers (the cursor is 0 on `EQMac`, and bag contents one lower), and
  `InventorySlot::is_held` names the slots an item update may fill. TAKP's
  session follows the inventory but offers no `Capability::Inventory` yet:
  moving items waits, as TAKP disconnects a player whose move it refuses.
  `inventory::takp_item_hit_points` is the one count of what TAKP adds for
  the player's items (`itembonuses.HP`, which its own HP update leaves out
  of the current HP), for a host's gauge and a bleed-out check alike: the
  worn items from ear to waist and the first food and the first drink
  carried, by TAKP's equip, level and worn-effect rules, with a worn
  effect's HP from the caller's spell data. It lists what it could not
  settle (`UnsettledItem`): an inventory it cannot trust, a worn effect
  without spell data, and an item above the player's required level, which
  TAKP counts only for an account of status 80 or more.

- TAKP's spell state, read from the `EQMac` wire as the events Titanium's
  packets give (Quarm sends the same packets, unchecked there). The
  profile gives the buffs (`buffs::eqmac_profile`), the 256-slot
  spellbook (`SpellBook::eqmac_profile`) and each gem's reuse time left
  (`PlayerState::spell_refresh_ms`). `spells::decode_eqmac` reads a cast
  beginning, with its base cast time, which the player's focus effects can
  shorten; an interruption; a gem's refresh, with no reuse adjustment; and
  the spellbook's replies. `buffs::eqmac_spell_effect` reads a spell taking
  hold and `buffs::eqmac_update` a buff's fade or corrected duration.
  `OP_ManaChange`, which TAKP sends only as the spell bar comes back, ends
  the cast (`SpellUpdate::Mana`, `keep_casting: false`) besides giving the
  mana, and `OP_ManaUpdate` gives the player's mana between casts. TAKP's
  server sends an interruption and a fade to the player alone without
  naming anyone, so the session names the player in them. A fade names no
  slot either (`buffs::UNKNOWN_SLOT`): a host finds the buff by its spell.
  The book reaches a host once TAKP offers the spellbook.
- Casting on TAKP (`Capability::Casting`): a memorized gem's spell
  (`GameCommand::CastSpell` on the `EQMac` wire, `spells::eqmac_cast`) and
  an item's click effect from a worn or general slot
  (`Request::CastItem`, `spells::eqmac_item_cast`), in TAKP's 12-byte
  `OP_CastSpell` with 16-bit fields and a CRC of 0 (inferred: TAKP never
  reads it, and the official client's is unrecorded).
  The cast is held, as on the other servers, until the spell bar comes back
  or the cast is interrupted. A hold now also ends on a result for the
  spell asked for when the server began a different one: TAKP begins a
  Luclin port as spell 2935 and ends it naming the port.
- TAKP's spellbook (`Capability::Spellbook`): memorizing, forgetting and
  scribing in TAKP's own `OP_MemorizeSpell` (`spells::eqmac_memorize`,
  `spells::eqmac_forget`, `spells::eqmac_scribe`), one change in flight at
  a time as on every server; TAKP logs two in one server tick as a possible
  hack. The book has TAKP's 256 slots, and a scribe past them is refused,
  since TAKP would use the scroll up and scribe nothing. Moving a spell in
  the book (`spells::eqmac_swap`, built) waits for its check on TAKP.
  Deleting stays off: TAKP reports a deletion done even for an empty slot
  and keeps the spell memorizable until the player zones.
- TAKP's coins from the profile: those the player carries
  (`money::eqmac_coins`, `WorldEvent::Coins`) and those on the cursor and
  in the bank (`money::eqmac_elsewhere`, `WorldEvent::CoinsElsewhere`),
  signed 32-bit counts where a negative one is refused. `EQMac` has no
  shared bank. TAKP's money notice (`OP_TradeMoneyUpdate` from trader 0,
  `money::eqmac_purse_addition`) says what it added to the purse, one kind
  at a time; the session adds it (`Message::PurseAdded`) and tells the
  purse it makes. TAKP never announces what it takes, and a notice naming
  another trader is refused. All of it reaches a host through the
  inventory feature, once TAKP offers it; loot coins and merchants come
  with those features.
- Reporting that the player bled out (`Capability::BleedingOut`, TAKP
  only). TAKP announces no death to a player who bleeds out, nor to one
  killed by a tick of damage or by their own hand, and waits for the
  client's own report. As the zone admits the player,
  `WorldEvent::DeathThreshold` tells a host the HP at or below which the
  server takes the player as dead (-11 on TAKP), once the session counts
  what the player's items add there. A host whose HP, with
  what equipped items add, reaches it on the server's report, with no
  death named for the player, sends `GameCommand::BledOut`, and the
  session sends `Request::BledOut`, `EQMac`'s 20-byte `OP_Death` naming
  the player (`quarm::bled_out`; Titanium sends none), only when the
  server's last report with what the player's items add is at or below
  the threshold. TAKP kills the player it names without checking their
  HP, and its report leaves item HP out, so a living player in HP gear can
  show the threshold: the session refuses the report while it does not
  know the player's items, and counts what they add by the server type's
  own rule. TAKP adds worn effects, the first food carried and a GM's
  items below their level too, and its count comes with the `EQMac`
  inventory, so until then TAKP takes no report at all and names no
  threshold, and a host asks for none. A dead player's report is refused
  too. Every field but the player's spawn is inferred
  until the official client's bleed-out is recorded: no killer, damage or
  spell, hand to hand (28), and no corpse, level or player flag. The
  session then takes the player as dead just as if the server had said
  so: every feature and the host hear one `WorldEvent::Death`, and the
  transfers feature alone marks the player dead. `Capability::ALL` grows
  to 36.

- `EQMac` spawns say their class and level (TAKP's `Spawn_Struct` at 87
  and 89), so a TAKP banker (class 40) is known as one, and a front end
  can show levels.
- The world's damage to the player (`Capability::EnvironmentalDamage`,
  `EqEmu` only, and there falls alone): `GameCommand::EnvironmentalDamage`
  reports the damage a host worked out, by its `hazards::Hazard` (falling,
  drowning, lava or freezing), in its client generation's packet:
  Titanium's 31-byte `OP_EnvDamage` (`hazards::titanium_damage`) or
  `EQMac`'s 24-byte `OP_Damage` (`hazards::eqmac_damage`). The server takes
  the amount as it is and applies its own reductions, such as `EQEmu`'s
  fall damage reductions from spells, items and AAs, so a host leaves
  those out. Each server type lists the hazards it takes from the client
  and the session refuses the rest: `EQEmu` takes falls, while drowning,
  lava and freezing wait until the official client's reports of them are
  recorded. What the official clients put in the fields neither server
  reads is unrecorded (inferred: the player's spawn, zeros, and
  Titanium's constant 0xFFFF).

- Targeting, considering and attacking on TAKP (`Capability::Targeting`
  and `Capability::Combat`), in `EQMac`'s own packets: the target as a
  16-bit spawn, TAKP's 24-byte consider request and answer
  (`combat::eqmac_consider_request`, `eqmac_consideration`), the
  auto-attack toggle, and its 24-byte damage records
  (`combat::eqmac_damage`), whose types are Titanium's.

- The server's regeneration tick (`Capability::ServerTicks`, `EqEmu` only):
  `WorldEvent::ServerTick` tells a front end that the tick landed just
  now, with its `ticks::Cadence` (period and slip), so that it can count
  down to the next one. No packet names a tick. `EQEmu` regenerates the
  player's HP, mana and endurance on one six-second timer, restarted from
  the pass of its 32 ms zone loop that it fires on and started afresh in
  each zone, and reports each value only when it changed. So the session
  takes a rise in any of them for a possible tick, and a rise that lands a
  whole number of ticks after another for a tick, passing over heals at
  other moments; nothing shows at full, and the next zone starts with
  nothing known. The session sends nothing for it.

- `RaidUpdate::Listed`: a member the server lists as the player joins,
  enters a zone or moves, told apart from one who joins. `EQEmu` sends each
  list in one burst after the raid's creation, with no packet marking its
  end (the leader comes last in some orders and early in others, and the
  join's list holds health updates), so the session counts every member
  added after a creation as listed until a second passes without a raid
  update. The raid the player forms by inviting lists nothing (inferred:
  what the official client tells the inviter is not checked). The session
  also holds a `Disbanded` as it holds a removal: the raid's end, the
  player's removal and the raid listed again, as `EQEmu` tells a member
  moved in another zone, is nothing; anything else tells them in order.

- The raid leader's commands (`Capability::Raiding`, `EqEmu` only):
  `GameCommand::RaidLock` (lock or unlock), `RaidMove` (into a raid group,
  0 to 11, or out of every group), `RaidMakeLeader` (`/makeraidleader`)
  and `RaidRemove` (removing oneself leaves), with the codec's `lock`,
  `move_member` and `make_leader`. The server's lock updates arrive as
  `RaidUpdate::Locked`, with the name the update gives: the leader's as
  they lock or unlock the raid, the member's own as they join it or enter
  a zone while it is locked; the session says `RaidUpdate::Locking` as it
  asks, so that the leader's own answer reads apart from the one they get
  on entering a zone. `EQEmu` moves a member by taking them out and
  adding them back, so the session holds each `Removed` until the next
  message and reports `RaidUpdate::Moved` when the same member is added
  back; the raid listed again to the player taken out drops the removal,
  and anything else, or 300 ms with no message, reports it after all,
  before that message. `Message::Withheld` stands for a message a feature
  holds back. `EQEmu` checks only that the one handing on the lead leads
  the raid; the session refuses all four commands from a member who does
  not lead it, a move while the raid is unlocked, into the member's own
  group or into a full one, and a member it does not know, the last as
  string 5082 naming them (`RaidRefused` gains `arguments`). It also
  refuses an invitation while the raid is locked, in this library's words
  (string 8870 announces the leader locking the raid). That the
  official client refuses these is inferred from its raid notes, its Raid
  window's tips and its strings. `Capability::RaidGroupLeaders`, taking a
  raid group leader's mark from a member, is offered by no server type:
  `EQEmu` has no handler for it.

- Raids (`Capability::Raiding`, `EqEmu` only): `GameCommand::RaidInvite`
  (`/raidinvite`, by name), `RaidAccept`, `RaidDecline` and `RaidLeave`
  (`/raiddisband` for the player), with the codec `raid`. The server's word
  arrives as `WorldEvent::Raid(RaidUpdate)`: an invitation, the raid's
  leader on joining or entering a zone (`Created`), each member with their
  raid group, class and level (`Added`), someone leaving (`Removed`), the
  player's end in the raid (`Disbanded`) and a new leader. The server keeps
  no invitations and answers none of these, so the session keeps the one
  waiting, says what it sent or answered (`Inviting`, `Accepting`,
  `Declining`, `Leaving`; declining sends nothing), and records the player's
  own raid chat as the others hear it, since the server does not pass it
  back, as the official client shows it (inferred). It refuses as `WorldEvent::RaidRefused` an invitation that names
  no one, one to the player themself (which `EQEmu` would turn into a
  broken raid), one to a member and one from a member who is not the
  leader, naming the official client's string for each (that it refuses
  them itself is inferred; `EQEmu` checks none of the inviter's side, though
  it refuses an invitee already in a raid and a grouped one who does not
  lead their group), and an answer or
  a leave with nothing to answer or leave.

- Dice, emotes and assisting (`Capability::Rolling`, `Emoting` and
  `Assisting`, `EqEmu` only): `GameCommand::Random` (`/random`), `Emote`
  (`/emote`) and `Assist` (`/assist`), with the codec `socials`. The server's
  roll for any player nearby arrives as `WorldEvent::Roll` and its answer to
  an assist as `WorldEvent::Assisted`, the target to take. The server passes
  an emote on to everyone near but the one who made it, so the session
  records the player's own emote as the others hear it, as the official
  client shows it (inferred). It refuses as
  `WorldEvent::SocialRefused` an empty or overlong emote and assisting the
  player themself, naming the official client's string for the last (that
  the official client refuses it is inferred; `EQEmu` answers with the
  player's own target).

- The player's listing (`Capability::Listing`, `EqEmu` only):
  `GameCommand::ToggleAway` (`/afk`), `ToggleAnonymous` (`/anonymous`) and
  `ToggleRoleplay` (`/roleplay`), sent as the player's own appearance
  update (`listing::titanium_away`, `titanium_anonymity`). Servers do not
  echo it, so the session keeps the player's listing (the admission's, then
  each change, its own or the server's), reports each change it sends as
  `WorldEvent::ListingSet`, and refuses as `WorldEvent::ListingRefused` a
  second change of a kind within 500 ms (`EQEmu` drops one within 250 ms
  of its last receipt, which jitter can shorten), `/anonymous`
  while roleplaying and `/roleplay` while anonymous, naming the official
  client's string for the last two. That the official client refuses those
  itself is inferred from its having the strings; `EQEmu` takes either.

- Groups (`Capability::Grouping`, `EqEmu` only): `GameCommand::InviteToGroup`
  (`/invite`, by name), `FollowGroup` (join the group of whoever invited the
  player last), `DeclineGroup`, and `Disband` (leave, or as the leader remove
  the targeted member or disband, as the server decides by its idea of the
  target; with an invitation waiting, decline it). The server's word arrives
  as `WorldEvent::Group(GroupUpdate)`: an invitation, the invitee's
  acceptance or refusal, the player forming a group, a member joining or
  leaving, the full member list with its leader, a new leader, and the
  group's end. The session says what it sent the same way (`Inviting`,
  `Following`, `Declining`), since the server does not answer it, and
  refuses as `WorldEvent::GroupRefused` an invitation that names no one,
  one from a member who is not the leader, and one to a full group, with the
  official client's string for each. That the official client refuses these
  itself is inferred from its having the strings; `EQEmu` lets a member who
  is not the leader invite, and an invitation to a full group through to a
  failed follow. The Titanium codec
  (`group`) reads `EQEmu`'s group structs, which arrive longer than the
  Titanium client's own.

- `Capability::MerchantOffers`: a front end may show what a merchant pays
  for an item sold to them, worked out from the item's `price` and the
  merchant's `rate` by the server type's rule. `EqEmu` offers it: the price
  times how many are sold (a charged item counts as one), times the
  merchant's modifier (one at neutral standing; 1 / (0.95 x the rate it
  opened with)), then times 0.95, each product cut to whole copper and never
  rounded up; three sales checked at a neutral merchant. Other server types
  wait for a check.

- A special message (`OP_SpecialMesg`) on the Titanium wire says how its
  speaker speaks: `ChatEvent::speak_mode` (`SpeakMode`: `Raw` for a plain
  server line, `Say`, `Shout`, `EmoteAlt`, `Emote` or `Group`, as `EQEmu`'s
  `Journal::SpeakMode` numbers them, and `Other` for a new one),
  `journal_mode`, `language` and `target_spawn_id`, so a front end can word
  an NPC's quest dialogue as the official client shows it. They are None for
  every other message and for the `EQMac` layout until it is checked on TAKP.

- Tell echoes (`ChannelName::TellEcho`): on the Titanium wire, the server's
  echo of a tell the player sent (channel 14) has a channel of its own, with
  the player as its sender and the one told as its target, so a front end
  can say whom the player told. The `EQMac` generation keeps channel 14
  unknown until it is checked there.
- Character-selection sessions and typed world state for graphical clients.
- P99 movement, targeting, doors, inventory operations, spellbook editing,
  casting, item activation, buff notifications, and zone/death handoff events.
- Quarm admission and entity presentation for graphical clients; outbound
  gameplay commands remain P99-only.
- Items on the ground (`objects`): Titanium ground objects are reported, and
  a nearby item can be picked up onto an empty cursor. A world container that
  opens for a click the session did not ask for is closed again.
- Worn gear (`appearance`): spawns and the player carry their materials,
  tints and facial features from Titanium spawn records, and wear changes
  update them. Quarm reports none yet.
- Handing items to NPCs (`exchange`): `OfferTrade` asks a character within
  reach while the player holds an item, the NPC's answer opens the give
  window, items go into its four trade slots from the cursor, and
  `AcceptTrade` (Give) or `CancelTrade` ends it. The trade slots
  (`InventorySlot::is_trade`) take only what servers accept, and empty when
  the window closes. `Capability::Giving` reports it.
- Trades between players (`exchange`): another player's request opens the
  window at once, as in the official client (`ExchangeUpdate::Taken`), or
  hears that the player is busy while another trade is under way. Their
  items (`ExchangeUpdate::Offered`, in slots from `THEIR_FIRST_SLOT`) and
  coins (`ExchangeUpdate::Coins`, kept as the wallet's `offered`) reach the
  host; Trade may be clicked again after anything put in undid it. When the
  other player closes the window the session closes it too, since `EQEmu`
  returns only the canceller's items. NO DROP items, and bags holding one,
  are refused before a move that `EQEmu` answers by disconnecting, and coins
  put in a trade stay there.
- Food and drink (`food`): the profile and the server's stamina updates say
  how fed and watered the player is (`Nourishment`). At 3000 or less, as
  `EQEmu` counts hungry and thirsty, the session eats and drinks from the
  inventory on its own as the official client does (each general slot,
  then the bag in it), takes the bite from the inventory as the server does
  silently, and tells the host when there is nothing to eat or drink
  (`NothingToEat`). By default it leaves food and drink with modifiers
  (`ItemDetails::has_modifiers`) for the player, and says when that is all
  that is left (`Shortage::OnlyModified`); `ClientConfig::auto_eat` set to
  `AutoEat::Anything` eats whatever comes first, as the official client
  does. `AutoEat` changes that while a zone session runs; each one starts
  with the configured choice. `Consume` eats or drinks an item by hand,
  refused with the official client's words when the player is full
  (`ConsumeRefused`).
- Who is online (`who`): `WhoAll` asks the world by name, guild or zone
  start, race, class, levels or game masters (`WhoFilter`,
  `Capability::Who`), and the answer reaches the host as `WhoList`: the
  string numbers that word its heading, lines and closing count, with each
  player's name, guild, level, class, race and zone as the world shows
  them. `zones` maps zone numbers to short names. For the zone's own list,
  which Titanium clients build themselves, spawns carry their level and
  the player and spawns their `/who` listing (`listing::Listing`: guild,
  anonymity, game master, away and looking-for-group flags), kept current
  by `WorldEvent::Listing`; the world's and zones' guild lists reach the
  host as `GuildNames`.
- Time of day (`clock`): the time in Norrath reaches the host as
  `WorldEvent::TimeOfDay` (`GameTime`, hours 0 to 23 from midnight) as the
  zone admits the player and whenever it changes; `GameTime::after` runs it
  on, a minute every three real seconds. Right after the admission,
  `WorldEvent::Sky` says how the zone's sky and fog look (`ZoneSky`: sky
  type, time type and the header's four fog colors and distances).
- Players' corpses (`corpses`): `Consent` lets a player drag the player's
  corpses or takes it back (`/consent`, `/deny`), refused with the
  official client's words for an empty name or the player's own; the
  server's answer reaches the host as `WorldEvent::Consent`, for the owner
  and the one consented. `SummonCorpse` (`/corpse`), `DragCorpse`
  (`/corpsedrag`) and `DropCorpse` (`/corpsedrop`, one corpse or all) name
  a player's corpse by its spawn, as servers know it
  (`Capability::Corpses`); anything else is refused (`CorpseRefused`).
- Pets (`pets`): `Pet` sends a command to the player's pet (`PetCommand`,
  numbered as the Titanium client sends them), naming the player's target
  for an attack, under `Capability::Pets`; without a pet the session refuses
  it in the official client's words (`PetRefused`). Spawns carry whose pet
  they are (`SpawnState::pet_owner`), charm's appearance updates move it
  (`WorldEvent::PetOwner`), and the pet's buffs reach the host as
  `WorldEvent::PetBuffs`. Spawns also carry their health when the record was
  sent (`SpawnState::hp_percent`), since the server reports a pet's health
  only when it changes.
- Abilities (`abilities`): `UseAbility` uses kick, bash, backstab, frenzy,
  the monk strikes and taunt on the target, and hide, sneak, forage,
  fishing (`OP_Fishing`, which anyone can try; the server checks the pole,
  the bait and the water), mend, feign death and sense heading on the
  player (`Capability::Abilities`). The
  session refuses what servers ignore without a word (an unknown skill, no
  target, a strike's target out of melee reach as `EQEmu`'s `CombatRange`
  measures it with the player's size by race, a taunt at anything but an
  NPC) and a use whose recovery timer still runs (`AbilityRefused`), and
  tells the host when a timer starts (`AbilityUsed`). Each server type lists
  the abilities it offers (`AbilitiesOffered`, at admission): `EQEmu` all of
  them, P99 every one but fishing until it is checked there; the session
  refuses the rest ("Not available on this server"). Strikes share one
  timer, as on the server, and wait for a cast to end.
- Coins (`money`): `MoveCoins` moves coins between the purse, the cursor, the
  bank (near a banker) and an open give window, changing kind as servers do
  (`CoinTransfer::amounts`). The session keeps the coins (`Wallet`): servers
  answer no coin move, so it refuses one a place cannot cover before sending
  (`CoinsRefused`), changes the purse for loot coins and purchases kind by
  kind as `EQEmu` does, takes each money update as the truth about the purse,
  and tells the host every change as `Coins` (the purse) and `CoinsElsewhere`
  (the cursor, the bank and a trade window's coins). Asking to trade needs an
  item or coins on the cursor.
- Training at a guildmaster (`training`), on `EQEmu` for now
  (`Capability::Training`): `Training` opens training with a guildmaster of
  the player's class within 200 units, practices a skill and leaves
  (`TrainingRequest`). The guildmaster's answer reaches the host as
  `TrainingUpdate::Offered`, with how far each skill can be trained. The
  session refuses a practice the server would ignore silently
  (`TrainingRefused`): no training open, a skill the guildmaster does not
  teach or one at its cap, no practice point left or too few coins. Servers
  answer a practice only with the skill's new value, which the session turns
  into `TrainingUpdate::Trained` with the practice's cost; the coins pay for
  it, and the session counts practice points itself (`PracticePoints`), from
  the profile's (`PlayerState::practice_points`) plus five for each level
  past the highest reached in the zone.
- Resurrection (`resurrection`), on `EQEmu` for now
  (`Capability::Resurrection`): an offer to resurrect the player
  (`OP_RezzRequest`) reaches the host as `WorldEvent::Resurrection`, with the
  caster, the corpse, the spell and where the corpse lies, and
  `AnswerResurrection` accepts or declines it, repeating the offer as the
  server expects (`OP_RezzAnswer`). An answer with no offer waiting is
  refused (`ResurrectionRefused`). On acceptance the server moves the player
  to the corpse as it moves them anywhere.
- Reading (`books`), on `EQEmu` for now (`Capability::Reading`): carried
  items say what they read as (`InventoryItem::book`: a Titanium item of the
  readable class with a text name, in the book window when flagged as a
  book and the note window otherwise), `ReadItem` asks for a book's or
  note's text (`OP_ReadBook`), and the text reaches the host as
  `WorldEvent::BookText` with the kind of window it is for. An item that is
  not readable is refused (`ReadRefused`).
- Tradeskill combines (`tradeskills`), on `EQEmu` for now
  (`Capability::Tradeskills`): `Combine` asks the server to combine what a
  tradeskill container in a pack slot holds (`OP_TradeSkillCombine`), and
  the session holds the inventory until the server answers
  (`WorldEvent::Combine`). The components leaving and what was made arriving
  are ordinary inventory news. A bag that is not a tradeskill container is
  refused (`CombineRefused`), and so is a combine while the cursor holds an
  item or coins, as the official client refuses it; the refusal names the
  official client's string for that (`CombineRefused::string_id`). World
  containers such as forges and ovens open within reach (`OpenContainer`,
  answered as `ObjectUpdate::Container`, or in use by someone else), hold
  what the player puts in their ten slots (`InventorySlot::is_world`, items
  in from the cursor and out whole onto an empty cursor), combine
  (`tradeskills::WORLD_CONTAINER`) and close (`CloseContainer`), when the
  server puts what they still hold back in the inventory
  (`InventoryUpdate::WorldEmptied`). `tradeskills::type_name` names a
  container type's string in the installed client, for a container whose
  server sends no name.
- The in-game map (`Capability::Map`), on `EQEmu` for now: a front end draws
  it from the installation's map files and the player's position, so the
  session sends nothing for it; the server type decides whether it is
  offered.
- TAKP servers (`ServerProtocol::Takp`, `takp`): a stock `EQMacEmu` server,
  such as a local test server, speaking the same `EQMac` protocol as Project
  Quarm. `ServerProtocol::is_stock` tells stock emulator servers (`EQEmu` and
  TAKP) from public ones. TAKP creates characters with the `EQMac` client's
  packets (`creation::eqmac_approval`, `creation::eqmac_request`); the
  `EQMac` creation asks for the start zone's safe point. Quarm does not create
  characters yet, and refuses `CreateCharacter` with `CharacterCreation`
  instead of ignoring it.
- Synthetic regression coverage for inventory reconciliation, scribe consumption,
  movement admission, cast state, and fresh-key world/zone handoffs.

### Changed

- A special message with no speaker, as every plain server line comes,
  has no `sender` rather than an empty one, so a sender always names someone.
- `Death` carries `corpse_name`: the name the server gives the corpse of a
  spawn that dies in view, which the session adds from the spawn table by the
  client generation's rule (`world::corpse_name`: on Titanium servers
  `EQEmu`'s `CalcCorpseName` form; None for `EQMac` until it is checked on
  TAKP), so a front end need not rename the corpse itself.
- `ItemDetails` carries the item's `price` (its base value in copper, from
  which merchants price it) and its `icon`, decoded from the Titanium item
  record, so an item opened from a link has its picture too; None where a
  generation's record is not checked. `InventoryItem::icon` is gone: the
  picture's one source is `details.icon`.
- A corpse's items are addressed by their place on it, from 0, rather than by
  the server's slot: `LootUpdate::Item { place, item }`,
  `LootUpdate::Taken { place, .. }` and `GameCommand::LootItem { place, .. }`.
  The Titanium wire numbers a corpse's items from 22 through 52 as one run
  (the Titanium patch's own `CORPSE_BEGIN`, the first carried slot), and the
  loot encoding and decoding apply it, so a front end knows nothing of it.
- `ChatEvent::message_type` keeps the message type the server gives a
  formatted, simple or special message on the Titanium wire (EQEmu's `MT_*`
  numbers, by which the official client colours the line); None for channel
  messages and for the `EQMac` layouts until they are checked on TAKP.
- `SpellUpdate::Interrupted` carries `caster_name`: the name the server
  sends to those near another caster whose spell stopped (Titanium
  `InterruptCast_Struct`'s label), so a front end can say whose it was;
  None on the caster's own notice.
- Deleting a spell from the spellbook (`DeleteSpell`) and moving one to
  another place in it (`SwapSpell`) each have a capability of their own,
  `Capability::DeletingSpells` and `Capability::MovingSpells`, instead of
  riding `Capability::Spellbook`. A server type offers each only once it has
  been checked there: `EqEmu` offers both, and `Project1999` waits for a
  check, so its sessions refuse both as unavailable.
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
- Quarm and TAKP zones run on the shared zone session instead of Quarm's own
  loop: `message::eqmac` reads their zone packets, `quarm::answer` answers
  Quarm's DLL version checks at any time, and they provide the spawns, the
  player's record and talk as features. A command none of their features
  takes is refused as unavailable instead of being dropped with a
  diagnostic, and a server request to change zones (`quarm::zone_request`)
  ends the session, as it did before, until zoning is built for them.
- `eq-network-game`: `message::titanium` reads a zone packet into `Message`s
  once for the whole session. Encoders return whole packets
  (`EncodedCommand`): `Doors::click_packet`, `Objects::pickup_packet`,
  `ContainerView::close_packet`, `ZoneOffer::response`, and the new
  `command::titanium_camp` and `titanium_logout`. `GameCommand::session_id`
  and `created` say which admission a command names and when it was made.

### Fixed

- The movement guard (`MovementGuard::accept`) and the wire velocity and
  turn rate (`MotionSession`) measure a sample from the previous accepted
  request's `created` stamp to its own, not from when each was processed,
  so a sample delayed in processing is no longer refused (22-38% of the
  player's steps were, on both wires). The 0.25 s cap, 0.05 s floor and
  staleness check apply as before; the distance tolerance is 0.01 units,
  for f32 rounding.
- `EQMac` spawns and the player's own zone entry carry their gear, tints,
  face, hair and beard (`Appearance`), which were left at their defaults:
  every `EQMac` character drew bare-headed and undressed, the player's own
  helm hidden while the character list showed it (seen on local TAKP).
  `Appearance::show_helm` is true on `EQMac`, since TAKP and Quarm send a
  head material of 0 for a player who hides their helm.

- A damage shield's record (`OP_Damage` of kinds 244 to 249,
  `combat::DAMAGE_SHIELD_KINDS`) reads as the points the shield dealt,
  `DamageOutcome::Hit`. Both servers send those points negated, so a
  shield of 1 to 6 points read as a block, parry, riposte, dodge,
  invulnerability or rune, and a larger one as an unknown code. A spell
  that took hold and dealt nothing (kind 231, value 0) reads as the new
  `DamageOutcome::Landed` rather than a miss, and TAKP's and Quarm's
  lifetap record to its caster, which names the caster as both target and
  source, as the new `DamageOutcome::Tapped` with the points taken. Hosts
  that match `DamageOutcome` exhaustively need arms for both.
- On `EQEmu`, a player whose hiding the server ended (a spell on them, a
  trap, looting, an item's use or a trade) stayed hidden from the zone,
  NPCs included, until they moved: `EQEmu` keeps the player hidden until
  the client answers its `OP_CancelSneakHide`, which its source says the
  Titanium client does, and the session never did. It now answers as
  `EQEmu` expects, with the player's own appearance, invisibility 0
  (`Request::Unhide`), on `EQEmu` alone, inferred from its source and not
  yet checked live or against the official client. P99 hears nothing back
  until its hiding is checked live there, and TAKP and Quarm end the
  hiding themselves.
- A zone's door table is no longer refused whole when it holds two or more
  static objects. `EQEmu` lists each of a zone's static objects (an object
  of type 0) in the door table with door ID 255, so that ID repeats, and
  the decoder refused the table at the second one: the player got no doors
  in that zone. The decoder now keeps every such record, still refuses any
  other repeated ID, and takes tables of up to 500 doors, the most `EQEmu`
  lists (it refused more than 256). `Doors` keeps every static object for
  the admission snapshot, and `entries` the last one listed, as before; a
  click on one is refused before it goes out, since nothing uses a static
  object. Found reading `EQEmu`'s sources; which zones it hit is not known.
- On Windows, a session no longer ends with "Overlapped I/O operation is in
  progress" (os error 997) or its cancelled twin (995). std's sockets are
  overlapped there, and their read timeout (`SO_RCVTIMEO`) leaves a socket
  whose read timed out in an indeterminate state, so a read that only ran
  out of time can come back as either error; both transports now treat it
  as the timeout it is. Seen live twice in half an hour on an idle TAKP
  session, and once on `EQEmu`; the cause is inferred from Microsoft's and
  Rust's documentation, not reproduced on demand.
- Vah Shir can equip what names their race: the move planner refused
  every equip for race 130 as a race it did not know.

- TAKP's bankers and merchants are known as such, so the bank and merchant
  windows open at them. `EQMac` spawns now say their class in the server's
  numbering, as Titanium's do: TAKP's patch sends a banker (40) as 16, a
  merchant (41) as 32 and the guildmaster classes (20 to 34) three lower
  (`common/patches/mac.cpp`, `ENCODE(OP_ZoneSpawns)`).
- Combined transport packets (`OP_Combined`) give every part a one-byte
  length, as `EQEmu` does, so a part of exactly 255 bytes no longer ends the
  session with "invalid combined length". `build_combined` refuses parts
  longer than 255 bytes instead of writing a length servers misread, and
  `OP_AppCombined` accepts four-byte lengths.

### Known limitations

- Movement still requires calibration. Airborne movement (falls and jumps) is
  accepted only on stock EQEmu sessions, with provisional physics, until
  official-client falls and jumps are measured. Fall damage is reported only
  there, as the host works it out; drowning, lava and freezing are not
  reported anywhere yet.
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
