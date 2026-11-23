# Changelog

This project follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

- Each Bard song stop says the Titanium program's line (0x40f06d, read
  R49-21): `WorldEvent::SongStopped { session_id, spell_id, string_id }`,
  with 0x318f, at the player's stop, a gem click while singing, a forget's
  answer and the server's cast update ending the song. A stop down the
  program's non-song path (another class's song, or a timed item cast's
  end) says nothing. The Mac-era server types send none.
  - **Breaking:** the new `WorldEvent` variant. A gem click that stops the
    song no longer sends `CastRejected`; the stop's `SongStopped` says it.
- The Titanium buff slot finder's steps 4 and 6 and the effect-327 top, as
  read R49-22e reads 0x414071 (checklist 6.612).
  - **Breaking:** `buffs::placement::Finder` is now
    `fn(&Landing) -> Result<Placed, Unplaced>`. `Landing` holds the slots
    held, each as a `Spell` (id, band, Bard level, the twelve `Effect`s),
    the landing `Spell`, the step-2 blocker and `others_add_no_slots`
    (false until the worn items' and abilities' effect 327 are known).
    `Placed` gives the slot, the slot whose buff the program removes
    first, and the `Rule` that chose it. `Unplaced` gains `ValueUnknown`
    and `Refused(Rule)`.
  - Step 4 compares the landing with every held buff, effect by effect, in
    the read's order, and replaces a weaker conflict in place, nominates a
    slot where either side has +0x17d, or refuses. Values are 0x40f339's,
    as its read gives them: the formula table, the slow rule, the cap at
    |max| and the base's sign, at the held entry's stored level and the
    caster's (`Spell::level`, `Effect::max`). An unknown level a formula
    needs, the random formula 123, and 137 and 138 (the owner's hit
    points) leave the landing unslotted (`ValueUnknown`).
  - Step 6: a landing without +0x17d and no room evicts the first held
    spell with it, from slot 0; the player's +0x256 is taken as 0 [I].
  - The normal band names slots below 15 plus what the held buffs' effect
    327 adds (at most 10). Past that the top is unknown unless every slot
    to 25 is full.
  - A replace or an eviction is a prediction: the server's next buff state
    stays the authority. Any missing fact leaves the landing unslotted.
- `Capability::Melody`: where the server type takes a Bard's `/melody`
  (read R49-33 c): P99 and stock Titanium `EQEmu`; not the Mac-era server
  types, whose program is unread. The host runs the melody with `StopSong` and gem casts, and a
  test shows the stop followed at once by a gem press casts. **Breaking:** a
  new `Capability` variant.
- Say why a zone sent no inventory at admission (gap 3.11, checklist 6.437):
  stock `EQEmu` sends a Titanium client no `OP_CharInventory` exactly when
  the character holds nothing (its Titanium encoder drops an empty one), or
  when the character's inventory fails to load. The session's handling is
  unchanged (the player holds nothing), and so is its diagnostic; the docs
  now name the source. The Titanium program doesn't wait for the message
  either: it enters the zone once the zone server is connected, and takes
  the inventory whenever it arrives (read R49-36 a). Whether it clears the
  last zone's items at zone-in is still unread, so that the original client
  then holds nothing stays inferred.

- Slower water's own set in the swim grant, as read R49-26 reads the Titanium
  region classifier (0x435ec9; checklist 6.483).
  - New `water::WetKind` (`Normal` for `WT`, `W2` and `W3`; `Slower` for
    `SL`, `LA` and `VW`), `WaterRules::slower` and `WaterRules::set_for`.
    Zone 222's own set is level 5's only; its slower water keeps
    `LIQUID_6_TO_8`.
  - `SwimCalibration` carries `slower` beside `set`, with
    `SwimCalibration::set_for`. The session bound takes the larger of the
    two sets' caps on each axis (`WetCaps::covering`), which widens it only
    in poearthb. Breaking: a new required field.
- A Bard's song under the Titanium program's rules (read R49-21; gap 3.5,
  checklist 6.442, 6.542), on P99 and Titanium EQEmu only. See
  API-CONTRACT.md, "A Bard's song under the Titanium program's rules".
  - **Nothing is held because of a song.** While a Bard sings, a strike, an
    item use, a belt use, an item move, a ranged attack, a memorize and a
    forget go at once. The ranged attack, memorize and forget are inferred
    (read R49-48 a). Every other cast holds as before, including an item
    cast started alongside the song.
  - **A gem cast while singing stops the song and casts nothing**, as the
    program's gem click does. The host is told the cast was refused, with
    the reason "The song stopped; the gem did not cast".
  - **A forget's answer stops the song**, with the stop sent. A memorize's
    answer leaves it.
  - **An item cast goes alongside the song.** When the server ends an item
    cast of over 100 ms, the song ends here too, with nothing sent; an
    instant item's end leaves it (read R49-21e).
  - A gem's song is now always taken as the class's, so `StopSong` sends
    the stop for it even after its gem was forgotten or changed (read
    R49-21b: the song test reads the spell, not the gems).
  - The Mac-era server types keep the earlier holds: their program is
    unread.
  - **Labelled:** the 100 ms test at the player's own cast start compares
    the base time, which is what the server sends: for a Bard's song every
    adjustment is 0 (read R49-21a).

- Keep the Titanium server filters as the program keeps them (read R49-2,
  checklist 6.494b and c). The client's 28 filters (eqgame.exe table
  0x66cd78) go as words 1 to 28, each the index of its Options row's
  choice; word 0, an ini-only switch, stays 0, its default. The Titanium
  client sends the packet at zone-in and again on every Options change
  (0x53cc29 to 0x53cd7d), so every Titanium server type now offers
  `Capability::ServerFiltersAtOnce`, Project 1999 included; its first use
  there is attended. The `EQMac` client's side is unread and keeps the
  zone-in send.
  - New: `FilterCategory::{ItemSpeech, Strikethrough, Stuns,
    SwarmPetDeath}` (Titanium words 25 to 28) and
    `filters::sent_on_every_change(dialect)`.
  - `FilterCategory::modes(Titanium)` lists each row's choices in the
    client's order, so a mode's place is its index.
  - **Breaking:** `FilterCategory::ALL` has 28 kinds, and matches on
    `FilterCategory` gain four arms.
  - **Breaking:** the default bad-word mode is `Hide` (index 0, the
    program's default: its filter starts on), so the default Titanium
    packet's word 7 is 0. Titanium damage over time takes `Show` (0) and
    `Hide` (1), the client's choices, for the default `Show`; stock
    `EQEmu` reads 0 from a client before `SoD` as the player's own.
- Pet orders and charms, as read R49-22a reads the Titanium program (6.610,
  gap 3.5).
  - **An order with no pet is sent** on every server type, as the program
    sends it (0x49975b). The "You have no pet" refusal (13091, which is really
    the program's line for targeting a pet with none) is gone. EQEmu's,
    Quarm's (fc75fcf) and TAKP's (44afcbf) handlers return in silence. The
    guard on a changed pet identity stays: it is ours, and it only refuses.
  - **A Titanium charm of a race 108, 216 or 472 spawn is ignored**, as the
    program's `OP_Charm` handler ignores it (0x453d7b), with a diagnostic.
    New: `control::CHARM_IGNORED_RACES`. The +0x1b3 byte it also checks is
    unread.

- Read `/played`'s three Titanium profile fields (read R49-8 a, checklist 6.490b): the account's entitled days (+0x3314), the character's creation time (+0x32ec) and its played minutes (+0x32f4), as `WorldEvent::PlayedTime` with each profile. Breaking: a new `WorldEvent` variant.
- A Titanium door click names the item held on the cursor, as read R49-22b
  reads the program (checklist 6.537, gap 3.4).
  - Bytes 8 to 11 of `OP_ClickDoor` carry the cursor item's ID from the
    session's own inventory, or -1 with nothing there or the inventory not
    loaded.
  - Bytes 4 to 7 carry the player's Pick Lock value (at most 254) for a
    lockpick (item type 12, not a container), and 0 for anything else, keys
    included.
  - Byte 1 stays 3, as recorded: it is stack contents, not a field.
  - Stock `EQEmu` reads none of it, so keyed doors behave as before there.
    `EQMac` still refuses a click with an item on the cursor; RoF2 sends -1.
  - Not yet built: trapped door types (item -1 and the Disarm Traps value;
    the read lists only part of the types) and type 57 contact clicks.
  - **Breaking:** `doors::titanium_click` takes `held: Option<HeldItem>`, and
    `Request::ClickDoor` becomes `Request::ClickDoor { door_id, held }`. New:
    `doors::{HeldItem, LOCKPICK_ITEM_TYPE}`.
- Send and hear `/surname` as read R49-10 reads the Titanium program (6.507, the client's #616 follow-up).
  - A quoted surname's spaces now go as given; only an empty word, a control character, anything outside ASCII or 32 bytes and more are refused, in the encoder and the session alike. The session no longer trims the word.
  - Any answer word but 0 is taken (`accepted: true`), where 2 and more were refused before.
  - Every `WorldEvent::SurnameAnswer` reaches the host, asked or not, and nothing holds a second request: the program shows each answer and paces a request by its own question.
- Every Titanium door is a wall (6.607), as the program's collision move filter
  0x42cdb1 has it (portcullis follow-up read, correcting R1): a shut
  portcullis of open types 60-77 now stops the player like a hinged door.
  `Solidity::of` gives `wall: true` for every open type; its floor rule is
  unchanged. No API change.

- Word a spoken tongue's skill as the Titanium program does (6.495, reads
  R49-4 and R49-4b), for the client's languages slice.
  - **Breaking:** `Tongue::spoken` takes a `chat::Speaker` (the game master
    flag and whether log-server word 0 is set), and `Tongue::skill()` is now
    the wire's `u32`.
  - The skill is the profile's less intoxication >> 2, floored at 0, language
    0 included; 100 for a game master, or -1 under log-server word 0. New
    `Tongue::unslurred` gives the away reply's raw skill.
  - New `PlayerState::intoxication` (Titanium u32 at 128, `EQMac` byte at
    4968). The program's own offset is in its memory copy, so the match to
    the profile field is inferred. No later change is decoded.
  - New `WorldRules::listener_garble`: Titanium's `OP_LogServer` word at 16,
    the switch for the listener's garble stage. `EQEmu` (1d9cb5c) never sets
    it.
  - `EQMac` writes the skill cut to 16 bits (inferred; the Mac program isn't
    read).
- Read the Apply Poison wait from effect 234 as the program does (read
  R49-1f, 6.493b): the host reports the total with the new
  `GameCommand::PoisonWait { session_id, total_ms }`, and each tick waits
  that many ms where it is not 0, else 10 s; a negative total never sends.
  Untold or `None`, the wait stays 10 s. **Breaking:** a new variant on the
  exhaustive command enum.

- Follow the Titanium program's read R49-6 for duels, stuns, becoming a
  corpse, `/lfg`, the quest reward, stat increases and `/sheathe` and
  `/aggressive`.
  - **Breaking.** `WorldEvent::DuelNews(socials::DuelNews)` is new, and
    `Capability::WeaponStances` and `GameCommand::ToggleStance` are new.
    `GameCommand::ChallengeDuel`'s `target` is now the player's target, or
    0 for none. `Request::AnswerDuel` with `accept: false` is refused on
    Titanium, and `Request::AutoDeclineDuel` and `Request::PlayerState` are
    new. `WorldEvent::StatsChanged` now carries `stats::StatIncrease`.
  - **Duel.** The program has no challenge window and no decline. A
    challenge is news. `/duel` at the challenger accepts with
    `OP_RequestDuel`, then `OP_DuelAccept`; anywhere else it challenges.
    The session turns a challenge down on its own, as the program does
    (0x3ea0, 12 bytes, reason 1 or 2), and keeps no pace, time-out or
    distance test. The host's question stays, as a departure the host
    owns: a yes sends what `/duel` sends, and a no sends nothing. The
    Mac-era duel is held for row 11g. The ignore-list refusal (reason 0)
    is not sent.
  - **Duel ends (read R49-6d).** Only the player's side, the one challenged
    or accepted, ends a duel. Its death is `Won` (with the program's 8-byte
    0x3bad) or `Ended`. Its leaving is `Forfeited` (with 0x3bad) or
    `Lapsed`. Its program's own refusal, 0x3ea0, now read as
    `DuelUpdate::AutoDeclined`, is `TurnedDown`. New:
    `DuelNews::{TurnedDown, Lapsed, Forfeited}`, `Request::DuelOver` and
    `socials::titanium_duel_over`.
  - **Stun.** A stun under divine aura (effect 40) is ignored (0x417839).
    A stun during a stun doesn't extend it.
  - **Corpse.** `OP_BecomeCorpse` places the spawn and stops it.
  - **LFG.** Titanium's body says the program's range, 100, at +0xc.
  - **Quest reward.** The coin order is cited to `EQEmu`'s
    `QuestReward_Struct` and `AddMoneyToPP` (1d9cb5c7).
  - **Stances.** `/sheathe` and `/aggressive` flip state bit 2 or 8 and send
    `OP_PlayerStateAdd` or `OP_PlayerStateRemove`, on stock `EQEmu` only.
    The first P99 use is attended.
  - **Stats.** `OP_IncreaseStats` is read as the program reads it, 12 bytes
    `{spawn, stat, amount}` (`stats::StatIncrease`, which replaces
    `stats::StatChange`; breaking). `EQEmu`'s own 141-byte form is
    unreadable, as it is to the program, and each one is counted as a gap
    with its length.

- A Titanium trap or door record (type 253, the new `TRAP_DAMAGE_KIND`)
  keeps its spell field in `Damage::spell_id`, since the Titanium program
  picks the line it prints for one by that field (0x4a56e3, read R49-3b).
  Other kinds still drop it. A field of 0 or 0xFFFF reads as none.
- Report drowning and lava on every Titanium server, as the Titanium
  program does (read R25: 0x41c905 sends its own damage reports on any
  server it plays on). **Characters on public EQEmu servers and on P99
  can now drown and burn with this client**, where before only falls were
  reported there. The EQEmu and P99 policies take falls, drowning and lava
  whether or not the session is on the local network, and both read the AA
  facts the air counter needs, unlisted (the AA requests stay local only,
  6.581). A host withholds every report with its own setting (the client's
  Take Environmental Damage). TAKP and Quarm are unchanged.
  - From reads R49-3 and R49-3a: after each Titanium report, falls
    included, the session sends the program's damage record, `OP_Damage`
    0x5c78 (`hazards::titanium_damage_record`). `EQEmu` only relays it.
    P99's handling is unknown until the attended liquid row (6.491).
  - The record is skipped below the program's death floor, -10, where the
    session can count the player's item HP. No Titanium server type can
    yet (inferred, 6.491b).
  - No Titanium report in the freeze stance or for a carrier race, as the
    program skips them.
  - Zone 200's lava is its own type, 244 (`LiquidHazard::ZoneHeat`,
    `Hazard::ZoneHeat`), and Titanium freezing encodes as 246.
  - New `WorldEvent::HazardReported { session_id, hazard, amount, record }`
    after every report sent, for the program's own damage line.
  - **Breaking:** `WorldEvent::HazardReported`; `ZoneHeat` on `LiquidHazard`
    and `Hazard`; `Hazard` is `Serialize`.

- Apply Poison as the Titanium program does it (read R49-1, 6.493): the
  session sits a standing player, waits 10 s seated, then sends the request.
  Any posture change before then cancels it. The skill and posture refusals
  carry the program's string ids (0x305f, 0x305d); a bag slot and unknown
  skills are refused in this library's words; a command while one is under
  way is ignored, after the sit. The 10 s wait does not sum effect 234 yet
  (read R49-1f): slower where a total is below 10 s, faster where one is
  above 10 s or negative, until the follow-up. `EQMac` takes the same flow,
  inferred.
  - New: `WorldEvent::PoisonStarted { session_id, slot, string_id }` and
    `WorldEvent::PoisonInterrupted { session_id, string_id }`.
  - **Breaking:** new variants on the exhaustive `WorldEvent`. A host that
    showed its own line when it posted `ApplyPoison` should show
    `PoisonStarted` instead.

- Offer emotes, yells, buff cancels, book deletes and moves, surnames,
  petitions and bug reports on Quarm and TAKP (6.604 E1 to E6), from each
  server's own handlers (Quarm fc75fcf, TAKP 44afcbf). This is
  source-derived and not yet checked live.
  - New: `socials::{EQMAC_EMOTE_OPCODE, eqmac_emote, eqmac_social_emote,
    eqmac_heard_emote, eqmac_yell}`, `spells::eqmac_delete` and
    `buffs::cancel::eqmac_packet`.
  - **Emote.** 1026 bytes: a zero word, then the text as Titanium writes it.
    The social-emote word (0xffff) is inferred from Titanium's (row 11a).
    Animations stay Titanium only.
  - **Yell.** An empty `OP_YellForHelp` at 0xda41.
  - **Buff cancel.** One 20-byte record for a beneficial buff only, on both
    servers. TAKP fades any spell, but the session does not ask it to.
  - **Book.** A delete is 8 bytes and waits for the echo; the local book
    changes only on an echo with `success` 1. Both servers delete the spell in
    their database but keep it in the live profile until the next load. The
    move was already built and is now offered.
  - **Surname.** The 100-byte request. The session first refuses, as the
    servers' comments say the program does (inferred): a character under
    level 20, or a surname over 19 characters. A surname already held is
    the server's to refuse.
  - **Reports.** A petition is the text and a NUL. A bug report is now the
    1131-byte short record (it was 3340), with only the reporter and the
    text; the servers fill in the place and target. Quarm runs with
    petitions off, and says so.
  - Held for reads of the `EQMac` program: inspecting, titles and duels
    (rows 11e to 11g).

- Offer `/disc` and `/shield` on Quarm and TAKP (6.603 G1, G2), from each
  server's own handlers (Quarm fc75fcf, TAKP 44afcbf). This is
  source-derived and not yet checked live.
  - New: `disciplines::eqmac::{OPCODE, REUSE_QUESTION, number, encode}`,
    `shielding::{EQMAC_SHIELDING_OPCODE, eqmac_shield}`,
    `GameCommand::AskDisciplineReuse` and `Capability::DisciplineReuse`.
  - **/disc.** `UseDiscipline` sends the servers' number for the spell and
    class (4 bytes: the number, then zeros).
    - Only a spell with no number for the class is refused locally.
    - The level, the reuse timer and any active discipline are the
      server's to judge.
    - A bare `/disc` sends 0.
  - **/shield.** `/shield` sends the 2-byte target spawn ID, with the same
    500 ms pace as Titanium.
- Build TAKP's and Quarm's empty `OP_Track` without offering it (6.603 G3).
  - New: `tracking::eqmac::{OPCODE, TRACKERS, press}`.
  - Both servers kick any class but a Ranger, Druid or Bard, so `press`
    refuses the rest.
  - Nothing sends it until the `EQMac` program's window and its sends are
    read (rows 10c, 10d).

- Fix TAKP's cast interruptions (6.603 D0). Since 0802525, TAKP sends
  `OP_InterruptCast` (0x3542) as a 6-byte `TokenText_Struct`, and the Mac
  reader took only Quarm's 4 bytes, so every TAKP interruption was refused.
  - Both lengths now read the same on both wires.
  - A `fromWorld` other than 0 is refused and logged as not understood.
  - New: `spells::{eqmac_token, DISCIPLINE_READY}`.
  - The discipline-ready message (string 393) now reaches the host as a
    chat line with the server's string number and color. Before, it was
    dropped.

- A jump never stands a seated player or ends a feign held standing (6.599,
  Adam 11-23).
  - **Seated jump:** a sitting or looting player's `Jump` is refused with a
    diagnostic, and nothing is sent or stood, as Titanium's jump waits on
    the can-move flag (read R19 c, 0x4a1d74). The Mac-era client is inferred
    the same, and a read is requested. New: `feign::{JumpGate, TITANIUM_JUMP,
    EQMAC_JUMP}` and `GameDialect::jump_gate()`.
  - **Feign held standing:** with Move While Feigned (6.454), the press or a
    feign spell's landing reports the new `WorldEvent::FeignHeld(true)`. Any
    posture sent or reported, or death, reports `FeignHeld(false)`, before
    that posture's own report. A camp's owed stand reports none. Hosts drop
    the jump's crouch posture reports while a feign is held. The jump
    request alone clears no feign on EQEmu (1d9cb5c), Quarm (fc75fcf) or
    TAKP (44afcbf).
  - `FeignHeld(true)` means pressed or landed and held, not that the roll
    succeeded: after a failed feign it stays true until the next posture,
    and a standing-still jump in that window skips its crouch reports too
    (row 6.599a).
  - **Breaking:** `WorldEvent` gains `FeignHeld(bool)`.

- Read TAKP's and Quarm's boat control (6.602), from their own
  `Handle_OP_ControlBoat` (Quarm fc75fcf, TAKP 44afcbf). Neither server type
  offers `BoatControl` or `BoatRiding` yet: both wait for the `EQMac`
  program's steering and riding reads (rows 9a, 9b).
  - New: `boats::{EQMAC_OPCODE, eqmac}`, `ControlBoat::eqmac`,
    `boats::riders::{EQMAC_PROFILE_NAME, eqmac_profile_name}` and
    `quarm::profile_vehicle`.
  - The 4-byte control (16-bit boat, take or release, a byte only echoed)
    and its echo are read; a refusal is the `IN_USE` line and no echo.
  - The Mac profile's boat name (+3256) now arrives as `Message::VehicleName`
    after the profile's other messages.

- Offer `Tradeskills` and `WorldContainers` on Quarm and TAKP (6.598), from
  their own handlers (Quarm fc75fcf, TAKP 44afcbf).
  - New: `tradeskills::{EQMAC_COMBINE_OPCODE, eqmac_combine, eqmac_decode}`,
    `objects::eqmac::{ACTION_OPCODE, close}` and `Message::ContainerReleased`.
    The Mac inventory reader now takes `OP_ObjectItemPacket` (places 0 to 9
    as 4000 to 4009) and `OP_ClearObject`.
  - The 32-byte combine names only the container's slot, the one field both
    servers read; the rest is zero (open row 6.598a).
  - A Mac close leaves the container's items in it: the server's empty
    answer ends the use and takes the places out of view.
  - Mac moves into a world container go whole from the cursor, and out only
    onto the empty cursor, a rule Quarm and TAKP both accept. A pickup the
    server refuses for lore stays in the container on every server.
  - A Mac combine refused without a reset frees the player after 10 s
    (open row 6.598c). The session closes the container when the player dies
    (open row 6.598d).
  - Source-derived, not live.

- Offer `Training` on Quarm and TAKP (6.597), from their own handlers (Quarm
  fc75fcf, TAKP 44afcbf).
  - New: `training::{EQMAC_OFFER_OPCODE, EQMAC_TRAIN_OPCODE, EQMAC_END_OPCODE,
    EQMAC_SKILL_OPCODE, eqmac_open, eqmac_reply, eqmac_practice, eqmac_end,
    eqmac_skill, eqmac_practice_cost, PriceScale, SpawnSkill}` and
    `Message::SpawnSkill`.
  - Breaking: `TrainingOffer` gains `price_scale`, None on Titanium.
  - The Mac profile's practice points (+156) and `OP_SkillUpdate2` are now
    read; the silent update is the player's skill only for their own spawn.
  - A paid practice leaves the purse as the server's copper-first take, or
    uncertain; an unanswered one leaves the points unknown and a paid one the
    purse uncertain. Languages are held until the `EQMac` program's rule is
    read.
  - Source-derived, not live.

- Offer `Resurrection` and `Sacrifice` on Quarm and TAKP (6.596), from their
  own handlers (Quarm fc75fcf, TAKP 44afcbf).
  - New: `resurrection::{EQMAC_OFFER_OPCODE, EQMAC_ANSWER_OPCODE,
    EQMAC_COMPLETE_OPCODE, eqmac_offer, eqmac_answer, eqmac_completion}`,
    `ResurrectionOffer::eqmac_move` and `Message::ResurrectionCompleted`.
  - The Mac answer is the 228-byte offer byte for byte, with only the choice
    set. A Mac question stays open an hour before the session declines it.
  - TAKP's completion moves the player only for the offer this admission
    accepted, at the same zone and place; any other is recorded as a gap.
  - Source-derived, not live.

- Offer `Pets` on Quarm and TAKP (6.595), from their own handlers (Quarm
  fc75fcf, TAKP 44afcbf).
  - New: `pets::{EQMAC_COMMAND_OPCODE, eqmac_command}` and
    `PetCommand::on_eqmac`.
  - The Mac spawn record's `petOwnerId` now fills `SpawnState::pet_owner`.
  - Taunt on, no cast and focus are refused there.
  - Source-derived, not live.

- Offer `Corpses` on Quarm and TAKP (6.591), from their own handlers (Quarm
  fc75fcf, TAKP 44afcbf).
  - New functions: `corpses::{eqmac_consent, eqmac_summon, eqmac_drag,
    quarm_drop}`. The consent answer is read into `WorldEvent::Consent`.
  - `/deny` is refused, because a consent toggles there.
  - `/corpsedrop` works on Quarm only, on Quarm's own opcode.
  - `/corpse` names only a player's corpse in the zone, at most every 250 ms.
  - Source-derived, not live.

- Offer `Rolling`, `Assisting` and `Splitting` on Quarm and TAKP (6.585), each
  from its own handlers (Quarm fc75fcf, TAKP 44afcbf).
  - New functions: `socials::{eqmac_random, eqmac_assist}`,
    `split::{eqmac_split, eqmac_auto_split, eqmac_take, eqmac_refused}` and
    `quarm::profile_auto_split`.
  - Rolls and assists are read into the existing `WorldEvent::Roll` and
    `WorldEvent::Assisted`; an assist answer names the player's own spawn when
    the server does.
  - Rolls take ends of 0 to 32000, in order. One assist is in flight at a
    time.
  - After a split, the purse is known only when the server's give-back or its
    take plus the share proves it.
  - Emotes and yells stay unoffered there.
  - Source-derived, not live.

  **Breaking** (for exhaustive matches): `message::Message` gains
  `SplitRefused`.

- Offer `/who all` and friends on Quarm and TAKP (6.583), each from its own
  handlers (Quarm fc75fcf, TAKP 44afcbf): `who::eqmac::{request, decode,
  unmatched}` for the 140-byte request and the 16-bit answer, and
  `friends::eqmac_request`. The answer is the same `WorldEvent::WhoList`;
  the Mac LFG mark comes as the player's `tag` (12314) and AFK as the
  `rank` (12311), so `WhoPlayer` keeps its fields. One query at a time, at
  least a second apart; one asked sooner is refused. Source-derived, not
  live. `WhoPlayer`'s `Debug` now leaves out the account's name.

  **Breaking** (for exhaustive matches): `message::Message` gains
  `WhoUnmatched`, the `EQMac` zone's string 5029 in place of an answer.

- Search the Bazaar on TAKP and Quarm (6.580), from EQMacEmu's source,
  which both servers share: `OP_BazaarSearch` (0x1142) with the Mac-era
  88-byte search and 20-byte welcome request, and the server's 76-byte
  result rows, each in a packet of its own, and its end. Offered on TAKP
  and Quarm as source-derived, not live (row 6.580a open; first use
  attended). Sent only in the Bazaar (zone 151; ours, stricter, since the
  servers flag a search from elsewhere but answer it), with Titanium's pacing.
  A search with a level bound is refused: the Mac record has none. An item
  id past 32767 arrives wrapped and can't be trusted as an id.

  **Breaking:** `BazaarListing::trader`, `serial` and `stat` become
  `Option`, since a Mac row carries no serial or stat and names no trader
  out of the zone, and `BazaarListing` gains `listings`. New:
  `bazaar::eqmac::{search, welcome, answer, Answer, SEARCH_OPCODE,
  SEARCH_LENGTH, WELCOME_LENGTH, ROW_LENGTH, DONE_LENGTH}`.

- Offer `Looting` on TAKP (6.577), from TAKP's own corpse handlers at
  EQMacEmu/Server 44afcbf: the same `EQMac` packets and sizes as Quarm's,
  the echo before the server's checks, and an empty cursor before an item
  request, as TAKP's `CheckCursorEmptyWhenLooting` defaults. Source-derived,
  not yet checked live. On TAKP and Quarm, item requests on one corpse are
  now at least 10 ms apart, the servers' `loot_cooldown_timer`; one asked
  sooner is refused.

- List the AA requests on local sessions only (6.581): a local stock
  `EQEmu` session and a local TAKP session now offer
  `Capability::AlternateAdvancement`, under the same local-only gate as
  the AA facts, so the live rows 6.556 and 6.562 can run. Public `EQEmu`,
  P99, Quarm and RoF2 still list it nowhere, local or not. On TAKP only
  the split goes; Train and Activate stay refused until read R49-41b.

- Lift the hosts' script holds (6.578): a host may now send a report, a
  bandolier swap, a task's acceptance or a task's cancel from a script, a
  social or a hotbutton as well as from the player's own typing or click,
  since a script may do whatever the player can. Documentation only: the
  session never knew where a command came from, and its checks and paces
  hold whatever sends it.

- Offer Quarm giving and trading, groups, anonymous and roleplaying, and
  translocate answers (6.576), each from Quarm's own handlers at
  SecretsOTheP/EQMacEmu fc75fcf. Source-derived, not live. The encoders
  are TAKP's; tests pin each body to the size Quarm's handler checks
  (requests 4, refusal 6, group 193/128/129/128, appearance 8, translocate
  92). Quarm moves an accepted translocate with its own zone change.

- Read the guild bank as stock `EQEmu` tells a Titanium client of it
  (6.563, slice G1): `OP_GuildBank` (0xb4ab) item updates (a slot shown or
  emptied), the deposit answer, the bare acknowledgement and the clear, by
  action and exact length, and the profile's `guildbanker` word. Read only:
  nothing is sent and nothing acts on an answer. Stock `EQEmu` sends
  Titanium no list of the bank, so a host's view is the slots seen since it
  entered the guild hall and never complete. A deposit's donor and a single
  member's name stay out of `Debug` output.

  **Breaking** (for exhaustive matches in the client): `WorldEvent` gains
  `GuildBank`. New: `guild_bank::{GuildBankUpdate, GuildBankEntry,
  GuildBankArea, GuildBankPermission, OPCODE, titanium, titanium_profile}`.

- Read augments from Titanium item records (6.570). A common item's
  children are its augment sockets, which `EQEmu` serializes on worn,
  carried and bagged items; before, any augmented item made the whole
  inventory unreadable. **Breaking:** the new field
  `InventoryItem::augments` (an id per socket, `AUGMENT_SOCKETS` 5) and
  the new `WeightUnavailable::Augments`. What augments add is not summed
  yet, so carried weight reads unknown while a counted item is augmented,
  and `Inventory::equipment_stats_uncertain()` is true while a worn one
  is. A bandolier swap naming an id socketed in a held item is refused.
- Use a potion belt slot on Titanium (6.568, slice 4), from stock `EQEmu`'s
  source and not yet checked live. New `GameCommand::UseBeltItem` (with
  `bandolier::BeltUse`) under the new `Capability::UsingPotionBelt`, offered
  by no server type until its live rows pass (P99 never). The session casts
  the item the belt names from the first worn slot or pack holding it, on
  `OP_CastSpell` casting slot 11, through the whole item-use path; never
  from a bag, a bank or the cursor. New `inventory::titanium_belt_cast`,
  `spells::BELT_CAST_SLOT` and `request::Request::CastBeltItem`.
  **Breaking:** the new `GameCommand`, `Request` and `Capability` variants
  (`Capability::ALL` is 88 long).
- Swap to a bandolier set on Titanium (6.566, slice 3), from stock
  `EQEmu`'s source and not yet checked live. New
  `GameCommand::SwapBandolier` under the new `Capability::BandolierSwap`,
  offered by no server type until its live rows and read R49-45 d. The
  server makes the swap's moves without telling the client, so the new
  `bandolier::swap` works them out as `EQEmu` does and the session sends
  the swap only when every move is known, as an inventory prediction;
  otherwise it refuses and sends nothing. New `bandolier::swap_to` and
  `request::Request::SwapBandolier`. **Breaking:** the new `GameCommand`,
  `Request` and `Capability` variants (`Capability::ALL` is 87 long).
- Save and clear bandolier sets and set and clear potion belt slots on
  Titanium (6.564, slice 2), from stock `EQEmu`'s source and not yet
  checked live. New `GameCommand::SaveBandolier`, `ClearBandolier`,
  `SetPotionBelt` and `ClearPotionBelt` under the new
  `Capability::Bandoliers` and `Capability::PotionBelt`, which no server
  type offers until their live rows pass (P99 never). The server answers
  none, so after each send the session tells its own mirror as the whole
  `WorldEvent::Bandoliers` or `PotionBelt`. Sets, slots and names are
  bounded before sending, since the server bounds neither the set nor the
  name; a belt item must be worn or carried and be a potion or made for
  the belt. New `bandolier::{save, clear, set_belt, clear_belt}`, and
  `request::Request` variants for each.
- Tell the profile's bandolier sets and potion belt to the host at the
  admission (6.559's events were read but never told, as they come before
  the admission and no feature staged them). A bad field now tells the
  new `WorldEvent::BandoliersUnknown`.
- **Breaking:** the new `GameCommand`, `Request`, `WorldEvent`
  (`BandoliersUnknown`, `BandolierRefused`) and `Capability` variants
  (`Capability::ALL` is 86 long), and `inventory::ItemPlacement` gains
  `potion_belt`, read from Titanium's item record.

- Browse a trader's wares in the Bazaar on stock `EQEmu`, read only
  (6.558, slice 3), from source and not yet checked live. New
  `GameCommand::BrowseTrader` and `EndBrowse` under the new
  `Capability::BazaarBrowsing`, which no server type offers until its live
  row passes. New `BazaarEvent` variants: `Admitted`, `TraderSign`,
  `ShopEnded`, `Listed(TraderListing)`, `Delisted` and `BrowseEnded`. The
  session clicks only a player whose trader sign it saw go on or a search
  named, within reach, one click at a time, and ends every browse it closes
  and every late approval, since `EQEmu` records the player as the trader's
  one customer until then. Titanium's merchant item packets now come to
  the session as the new `Message::MerchantView` with their serial; the
  merchant window is unchanged. **Breaking:** the new `GameCommand`,
  `Request`, `Message`, `BazaarEvent` and `Capability` variants, and
  `BazaarEvent` is no longer `Eq`.
- Encode the AA requests for `EQMac` too, listed by no server type until
  a local TAKP live row (6.562): TAKP's 256-byte text on `OP_AAAction`
  (0x1442), `buy N`, `activate N`, `on N` and `off`, from fixed words and
  numbers only (`alternate_advancement::eqmac_action`). On `EQMac` the
  session sends only the split, on at level 51 or more (inferred); a train
  and an activation are refused as `Refusal::Unknown` until the Mac
  client's AA data (read R49-41b), since TAKP's buy checks neither level
  nor class, and an activation's target is refused. RoF2 refuses all three.
  New: `alternate_advancement::{eqmac_action, EQMAC_ACTION_OPCODE,
  EQMAC_ACTION_LENGTH}`; no new variants.


- Send Titanium's AA requests, built and tested but listed by no server
  type until their local `EQEmu` live row (6.556): training a rank,
  activating an owned rank's ability and the experience split, each as the
  16-byte `OP_AAAction` (0x0681) `EQEmu` reads. The session checks each
  against a bounded view of this admission's AA facts (at most 4096
  definitions; withdrawals followed), with only checks `EQEmu` makes too,
  and refuses what it can't tell. One train at a time until the server's
  answer or 5 s; activations and split changes at most one a second (ours,
  inferred). Which ids the program sends is read R49-39, inferred until then.

  **Breaking** (for exhaustive matches in the client): `GameCommand` gains
  `TrainAlternateAdvancement`, `ActivateAlternateAdvancement` and
  `SetAlternateAdvancementSplit`; `WorldEvent` gains
  `AlternateAdvancementRefused`; `Capability` gains `AlternateAdvancement`
  (`Capability::ALL` is now 83 long); `request::Request` gains
  `AlternateAdvancement`. New: `alternate_advancement::{Action, Refusal,
  ACTION_OPCODE, titanium_action}`.

- Read the player's bandolier sets and potion belt from the Titanium
  profile (6.559, slice 1). The new `bandolier` module reads the four sets
  (a name and four items) and four belt slots into the new
  `WorldEvent::Bandoliers` and `WorldEvent::PotionBelt`, on every Titanium
  server. A bad field withdraws only these two events. Nothing is sent.
  **Breaking:** new `WorldEvent` variants.
- Read an NPC's task offer and a finished task's steps, and send the
  player's task requests (checklist 6.555), from stock `EQEmu`'s source
  and not yet checked live. The new `tasks::offers` module reads
  `OP_TaskSelectWindow` (0x5e7c) and `OP_TaskHistoryReply` (0x397d), and
  encodes `OP_AcceptNewTask` (0x207f, 12 bytes), `OP_CancelTask` (0x3ba8),
  `OP_TaskHistoryRequest` (0x5df4) and `/tasktimers` (0x6a1d, empty body,
  inferred). New events for the client's ignore arms: `WorldEvent::TasksOffered`,
  `TaskHistory` and `TaskRefused`. New commands `GameCommand::AcceptTask`,
  `DeclineTasks`, `CancelTask`, `AskTaskHistory` and `ListTaskTimers` under
  the new `Capability::Tasks`, which only a local stock `EQEmu` session
  offers until the live row passes. The session accepts only a task in the
  current offer while its giver is an NPC it holds, and sends one cancel
  per held slot until the removal, against two faults in `EQEmu`'s
  handlers; declining sends nothing (inferred). Both new coverage rows keep
  no sample. **Breaking:** the new `News`, `Request`, `GameCommand`,
  `WorldEvent` and `Capability` variants.
- Read RoF2's corpse money and loot close, the surname answer, the title
  list and title changes as stock `EQEmu` sends them, labelled inferred for
  RoF2 (6.552). The new `rof2_loot_titles` module reads `OP_MoneyOnCorpse`
  (0x5f44), `OP_LootComplete` (0x55c4), `OP_Surname` (0x0423),
  `OP_SendTitleList` (0x2d08) and `OP_SetTitleReply` (0x4c21), which have no
  encoder for either generation, through Titanium's readers into Titanium's
  events, with Titanium's refusals. The corpse money record that ends a SoD
  and later listing (response 6, Loot All) gives no event: its coverage row
  is `Partly` with a part naming it, so the gap log tells each arrival, and
  lifting validate waits on the loot listing batch that decides its event.
  The last name and the titles reach the host in the event only.
- Search the Bazaar on stock `EQEmu`, read only (6.550, slice 1). The new
  `bazaar` module builds `OP_BazaarSearch` (0x1ee9) searches and welcome
  requests and reads the results, the search's end and the welcome.
  `GameCommand::BazaarSearch` and `BazaarWelcome` go only in the Bazaar,
  one at a time and 2 s apart (ours, inferred), and are refused with the
  new `WorldEvent::BazaarRefused`; answers come as `WorldEvent::Bazaar`.
  `Capability::BazaarSearching` is offered by no server type until the
  local live row passes. Gaps of the bazaar's and traders' messages keep no
  sample. **Breaking:** new `GameCommand`, `WorldEvent` and `Capability`
  variants.
- Read `EQMac`'s alternate advancement on local TAKP, read only (6.548):
  `OP_RespondAA` (0x1542) by Mac AA id, the profile's AA array and points,
  `OP_AAExpUpdate` (0x2342) and the server's reuse timers on `OP_AAAction`
  (0x1442), from TAKP's layouts. The first byte of the ranks is labelled
  the points spent and id 211's constant 3 is passed through, both
  inferred from TAKP's use. Quarm shares the decoding, assumed as TAKP, and
  isn't offered it until an attended Quarm row; remote TAKP waits for the
  local row. `EQMac` servers send no definitions, so nothing new enters the
  environmental sums.

  **Breaking** (for the client repin): `alternate_advancement::Update` gains
  `MacRanks(Option<MacRanks>)`, `MacSelections(Option<Vec<MacRank>>)`,
  `MacPoints(Option<MacPoints>)`, `MacProgress(Option<MacProgress>)` and
  `MacTimer(MacTimer)`, with those structs.

- Read RoF2's quest rewards and their sound, renames, the new titles
  notice and the banker's change as stock `EQEmu` sends them, labelled
  inferred for RoF2 (6.549). The new `rof2_notices` module reads `OP_Sound`
  (0x1a30, both the reward and the ding record), `OP_MobRename` (0x2c57),
  `OP_NewTitlesAvailable` (0x0d32) and `OP_BankerChange` (0x791e), which
  have no encoder for either generation, through Titanium's readers into
  Titanium's events, with Titanium's refusals. The rename's names reach the
  host in the event only. Coverage rows for all four; `OP_Sound` and
  `OP_MobRename` share Titanium's parts. Validate still refuses RoF2.
- Read the task journal stock `EQEmu` sends a Titanium player (gap list
  section 5, checklist 6.546), from source and not yet checked live. The
  new `tasks` module reads `OP_TaskDescription` (0x5ef7), `OP_TaskActivity`
  (0x682d), `OP_TaskActivityComplete` (0x54eb), `OP_CompletedTasks`
  (0x76a2) and the server's `OP_CancelTask` (0x3ba8), refusing a record
  whole rather than cutting it. A session journal, offered by stock
  `EQEmu` only, keeps the task and 19 quests per admission and tells the
  host five new events for its ignore arms: `WorldEvent::TaskDescribed`,
  `TaskStepUpdated`, `TaskConcluded`, `TaskRemoved` and `CompletedTasks`.
  A step for no entry of its task is told as a gap. The five coverage rows
  are read, the description's crystal parts and three step words partly;
  their gaps keep no sample, since the bodies carry the server's task
  text (new `sent_private` coverage rows). P99, TAKP, Quarm and RoF2 keep
  no journal; RoF2's coverage is unchanged. Nothing is sent yet.
  **Breaking:** `Message::Journal` and the five `WorldEvent` variants.
- Read the rest of Titanium's alternate advancement, read only (6.547):
  every `OP_SendAATable` header field (`Rank` gains `hotkey_sid`,
  `hotkey_sid2`, `title_sid`, `desc_sid`, `cost`, `prereq_skill`,
  `prereq_minpoints`, `kind` for the wire's `type`, `spellid`, `spell_type`,
  `spell_refresh`, `max_level`, `last_id`, `next_id` and `cost2`, each raw
  under its wire name), the profile's points spent, unspent and `expAA`,
  `OP_AAExpUpdate` (0x5f58) and the server's reuse timers on `OP_AAAction`
  (0x0681). The session publishes them as received and keeps no timers.
  The feature is still offered on local `EQEmu` only; it opens to every
  stock `EQEmu` session in a one-line follow-up once its local live row
  passes, and stays off P99, TAKP, Quarm and RoF2.

  **Breaking** (for the client repin):
  - `alternate_advancement::Rank` gains the fifteen fields above.
  - `alternate_advancement::Update` gains `Points(Option<Points>)`,
    `Progress(Option<Progress>)` and `Timer(Timer)`, with the new structs
    `Points`, `Progress` and `Timer`.
  - `alternate_advancement::titanium_profile` returns `[Update; 2]`
    (selections, then points), and `invalidated` returns `Option<Update>`
    (`None` for an unreadable timer).

- Take an item's song as the class's when the host's installed spell data
  says so (gap 3.5, checklist 6.542, part 2). The new
  `ItemUse::installed_class_spell: Option<bool>` is the host's word; with
  `Some(true)` a Bard's `/stopsong` sends the stop for an item's song of a
  class spell not memorized, as the program does (0x40db0b, read R16), and
  with `Some(false)` it ends here only. `None` keeps the memorized test.
  The word only chooses for a song the session already keeps; it never
  starts one or sends a stop alone. **Breaking:** `ItemUse` gains the field.
- Read RoF2's targets, charms, cash rewards, linked reuse timers and the
  answers to begging, pick pocketing and bandaging as stock `EQEmu` sends
  them, labelled inferred for RoF2 (6.544). The new `rof2_answers` module
  reads `OP_TargetCommand` (0x58e2), `OP_TargetHoTT` (0x0272), `OP_Charm`
  (0x5d92), `OP_CashReward` (0x5f7a), `OP_LinkedReuse` (0x1619),
  `OP_Begging` (0x6703), `OP_PickPocket` (0x39e8) and `OP_Bind_Wound`
  (0x0386), which have no encoder for either generation, through
  Titanium's readers into Titanium's events. Titanium's begging and pick
  pocket reader is now the named `abilities::ask_answer`, taking the
  ability, and its bandage reader `bind_wound::answer`. Coverage lists the
  eight. An admitted RoF2 session hands the host each and still sends and
  offers nothing.
- Keep a Bard's gem cast as the song only over 100 ms once the player's
  own `OP_BeginCast` says its time (gap 3.5, checklist 6.442 / 6.542).
  The 100 ms rule is the program's (0x411f3d, read R16); the time is the
  one the server sends (stock `EQEmu` sends the base cast time to the
  caster too). Until the start comes a gem cast is taken as a song, and with
  none it stays one, as before. Another spawn's start, another spell's, or
  an item's song (decided by its record's cast time) changes nothing.
  Whether 0x411f3d reads the base or the focused time, and whether P99
  echoes the start to the caster, are inferred (R49-21a). The song state
  still decides only what `/stopsong` and a Bard's `/stopcast` send; it
  holds nothing and never touches movement.

- Read RoF2's rolls, assists, yells for help and effects as stock `EQEmu`
  sends them, labelled inferred for RoF2 (6.541). The new `rof2_socials`
  module reads `OP_RandomReply` (0x106b), `OP_Assist` (0x4478),
  `OP_YellForHelp` (0x4e56), `OP_LevelAppearance` (0x3bc9),
  `OP_CameraEffect` (0x127f), `OP_PlayMP3` (0x5770) and `OP_SpellEffect`
  (0x5936), which have no encoder for either generation, through
  Titanium's readers into Titanium's events. Titanium's roll and assist
  readers are now the named `socials::roll` and `socials::assist_answer`,
  and its four effect readers are shared. Coverage lists the seven, the
  spell effect with Titanium's part. An admitted RoF2 session hands the
  host each and still sends and offers nothing.
- Bound a walking strafe by the walking strafe rate (checklist 6.540).
  `MovementMode::Strafe` carries the new `movement::Pace` (`Run`, the
  default, or `Walk`), as a diagonal already says Walk or Run. The session
  bounds a `Strafe(Pace::Walk)` by `StrafeCalibration::walk_units_per_second`
  where the grant has one, and by the one strafe rate where it has none
  (`EQMac`), so the guard only gets stricter; a running strafe and every
  other mode keep their bounds. Each grant's span in a sample takes the
  pace too. The wire is unchanged: the same move walking or running
  encodes the same packet.
  **Breaking:** `MovementMode::Strafe` is now `MovementMode::Strafe(Pace)`.

- Read RoF2's doors, ground objects and weather as stock `EQEmu` sends
  them, labelled inferred for RoF2 (6.539). The new `rof2_world` module
  reads `OP_SpawnDoor` (0x7291), 100-byte records with the size widened to
  32 bits, `OP_MoveDoor` (0x08e8), `OP_GroundSpawn` (0x6fca), the encoder's
  variable record of the name's length plus 101, `OP_ClickObject` (0x4aa1)
  and `OP_Weather` (0x661e) into Titanium's events, with Titanium's
  refusals. Titanium's door table, door move and object click are now the
  named `doors::door_table`, `doors::door_move` and `objects::click`, and
  `objects::text` is shared. Coverage lists the five; the ground object's
  zone, instance, tilts and size, and the 12-byte zone-in weather, are
  planned parts. An admitted RoF2 session hands the host each and still
  sends and offers nothing.
- Read RoF2's mana and endurance pools as stock `EQEmu` sends them,
  labelled inferred for RoF2 (6.538). The new `rof2_pools` module reads
  `OP_ManaUpdate` (0x3791) and `OP_EnduranceUpdate` (0x5f42), the server's
  10 bytes with the maximum, into the new `WorldEvent::ResourcePool`, and
  `OP_MobManaUpdate` (0x2404) and `OP_MobEnduranceUpdate` (0x1c81), 3
  bytes with a percent up to 100, into the new
  `WorldEvent::ResourcePercent`, each naming a `rof2_pools::Pool`. Titanium is
  sent none of the four. Coverage lists them. An admitted RoF2 session
  hands the host each and still sends and offers nothing.
- Cap the reconnect loop and drop what was queued before it (gap 3.11,
  checklist 6.534). With `RunOptions::reconnect` on (the default),
  `RunOptions::reconnect_attempts` (default 5, our bound, 6.534) limits the
  fresh logins in a row; the count starts again only after a run has stayed
  in its zone for the reconnect delay. Just before each fresh login, every
  command still queued is dropped unread and told as the new
  `ClientEvent::CommandDropped` (its kind and our reason only), so a chat
  line, a character choice or creation, or any other command from the ended
  session cannot act in the next. Commands sent after the fresh login says
  `Connecting` are read by it. The eq-client viewer turns reconnect off; the
  loop runs for hosts that keep the default and in this crate's tests.
  **Breaking:** `ClientEvent::Reconnecting` gains `attempt` and `attempts`,
  and `ClientEvent` gains `CommandDropped`.
- Read RoF2's spell book and gems as stock `EQEmu` sends them, labelled
  inferred for RoF2 (6.532). The new `rof2_spellbook` module reads
  `OP_MemorizeSpell` (0x217c), `OP_SwapSpell` (0x0efa) and `OP_DeleteSpell`
  (0x3358), which have no RoF2 encoder, through Titanium's readers, now the
  named `spells::memorized`, `book_swap` and `book_deletion`, with RoF2's
  720-slot book where Titanium's is 400. A bar refresh carries the server's
  casting slot and reduction. Coverage lists the three. An admitted RoF2
  session hands the host each and still sends and offers nothing.

- Measure a Titanium door click from the point clicked on the door
  (checklist 6.530, read R16b and R16e). `GameCommand::ClickDoor` gains
  `point: Option<Position>`, where the host's click ray hit the door's
  model as drawn. With a point, the session admits the click when the
  point is no more than the player's size and 20 from the player in 3-D
  (`doors::Doors::check_click_at`, `doors::Clicked`), so a lift resting
  above can be called from its deck by the part clicked. The size is the
  own spawn's as the server told it; where that is 0 or unheard, `EQEmu`'s
  race size (inferred); 0 for an unlisted race. A point that is not finite
  or lies more than 200 and the size from the door is refused in our own
  words (our guard). With no point the origin rule is unchanged, and
  `EQMac` ignores the point. Melee reach takes the player's size from the
  same helper, with the same numbers.
  **Breaking:** a new field on `GameCommand::ClickDoor`.

- A boat take asked again while one is pending goes again (6.533). The
  program sets nothing before a take's echo, so its held-boat test
  (`[0x905cf8]` in 0x4a7356) fails while the take waits and a second
  interaction sends a second take; gate 0x494c09 passes on a race-141 carrier
  before its busy checks. That is inferred from read R35's pieces in
  combination, not read as one path. The session now sends the same 8-byte
  `OP_ControlBoat` take for the same incarnation, leaving the pending entry,
  its 5 s deadline and the fence as the first take set them, so repeated
  clicks never extend the wait; stock `EQEmu` echoes each take, one echo
  grants and later ones change nothing. A take for another vessel while one
  is pending stays refused.
- Read RoF2's buffs as stock `EQEmu` sends them, labelled inferred for
  RoF2 (6.527). The new `rof2_buffs` module reads `OP_Buff` (0x659c, 100
  bytes) into Titanium's `BuffUpdate`: fade word 1 is `FadeBySpell` and 2
  `BySpell` (inferred as Titanium's readings of the server's 1 and 0), the
  float bard modifier reads back as the server's byte, and the counters
  read 0, since the encoder sends none. `OP_BuffCreate` (0x3377) and
  `OP_TargetBuffs` (0x4f4b) give the new `WorldEvent::BuffIcons` (the
  spawn, the window, whole list or one slot, the tick timer and each
  slot's spell, ticks, hits and caster), and `OP_PetBuffWindow` (0x5882)
  gives Titanium's `PetBuffs`, refusing a body whose count and entries
  differ. Coverage lists the four. An admitted RoF2 session hands the host
  each and still sends and offers nothing.
- Pace all chat a host asks for (6.523). Every `SendChat`,
  `SendChatInAdmission` and `ChatChannel` goes through one pace that spans
  zones: at most 10 lines in any 5 s, and at most 12 shout, auction,
  out-of-character, tell and channel messages and chat server commands in
  any 60 s, below stock `EQEmu`'s disconnect at more than 20 a minute. Only
  `join`, `leave`, `leaveall`, `list`, `listall` and `announce`, which reach
  no one else, are left out of that count; an unknown word is counted. Both bounds are inferred
  (R49-17) and only ever make the client slower. A paced line is refused,
  not queued and not counted; zone chat is told by the new
  `WorldEvent::ChatRefused`, channel commands by `ChatChannelRefused`, with
  the whole seconds to wait and never the line. The chat link's own login
  is not counted. **Breaking:** `WorldEvent::ChatRefused` is a new variant.

- Read RoF2's casting, stuns and illusions as stock `EQEmu` sends them,
  labelled inferred for RoF2 (6.522). The new `rof2_casting` module reads
  `OP_BeginCast` (0x318f, 10 bytes, the spell in 32 bits refused past 16),
  `OP_Action` (0x744c, the 56-byte `ActionAlt_Struct`), `OP_InterruptCast`
  (0x048c, unencoded, through Titanium's reader, now the named
  `spells::interrupted`), `OP_Stun` (0x36a4, 8 bytes, the last four the
  encoder's constants), `OP_Illusion` (0x312a, 336 bytes, a 16-bit race)
  and `OP_SetFace` (0x1af3, the server's 24-byte face change with both eye
  colors), each giving Titanium's event for the same server values. The
  action's float instrument modifier reads back as the server's integer,
  `round(value * 10)` within a tolerance that grows with the value, up to
  100,000; the server's 0 to 9, which the encoder's unsigned arithmetic
  wraps alike, read 0. Coverage lists the six, with the Drakkin words as
  the illusion's and face change's part. An admitted RoF2 session hands
  the host each and still sends and offers nothing.
- Keep a late scribe answer's hold until its cursor removal (6.526). A
  scribe that timed out and then got its late book confirmation, after the
  player moved the scroll, lost its receipt to any other inventory update,
  so the cursor removal that followed applied as a plain removal. The
  receipt now passes updates that leave the cursor alone, so that removal
  still marks the inventory stale. Anything else naming the cursor, a
  snapshot, an invalidation or 5 s since the confirmation ends it; a
  cursor removal 5 s or more after the confirmation is now a plain removal.
- Tell the player's bind zone (checklist 6.524). `world::titanium_bind_zone`
  reads the primary bind's zone from Titanium's profile (`binds[5]` at byte
  24, 20-byte records, as `EQEmu`'s `titanium_structs.h` lays them out), and
  the `EQMac` profile's first bind, already read for the move home after
  death, gives the same. Each says it as the new
  `WorldEvent::BoundIn { zone_id }`, told once after the admission, after
  the buffs and experience. A zone past 16 bits is unreadable and gives
  none; the zone-in goes on. It needs no capability and sends nothing.
  Stock `EQEmu` sends a Titanium client nothing when a bind lands
  mid-session, so the zone is believed current only until the next zone-in
  (inferred, pending R49-6m). `EQMac`'s move home is unchanged.
  **Breaking:** a new variant on the exhaustive `WorldEvent`.

- Read RoF2's chat and the server's own lines as stock `EQEmu` sends them,
  labelled inferred for RoF2 (6.521). `chat::parse_for` with
  `GameDialect::Rof2` now reads `OP_ChannelMessage` (0x2b2d) in its own
  layout (two NUL-ended names, the language, channel and skill, the text
  and a 15-byte zero tail, exact length), and `OP_MOTD`, `OP_Emote`,
  `OP_SpecialMesg`, `OP_FormattedMessage`, `OP_SimpleMessage`,
  `OP_GuildMOTD`, its reply and `OP_ColoredText` in Titanium's shape under
  RoF2's numbers. Item links are 56 characters with the item at digits 1
  to 5. The RoF2 wire's `chat` reads them, and coverage lists the nine
  (`chat::ROF2_READS`). Outbound chat still refuses RoF2. An admitted RoF2
  session records the chat and still sends and offers nothing.
- Read RoF2's own-player news and combat records as stock `EQEmu` sends
  them, labelled inferred for RoF2 (6.519). The new `rof2_news` module reads
  `OP_Death` (0x6517), `OP_LevelUpdate` (0x1eec), `OP_ExpUpdate` (0x20ed),
  `OP_Stamina` (0x2a79), `OP_MoneyUpdate` (0x640c) and `OP_TimeOfDay`
  (0x5070), which go out unencoded with the bytes Titanium gets, through
  Titanium's readers; the level, experience and money arms of
  `world::titanium_update` are now named functions both call. It reads
  `OP_ManaChange` (0x5467) at the server's 20 bytes, giving Titanium's two
  events and leaving the slot; `OP_Damage` (0x6f15, 30 bytes, a spell past
  16 bits refused, the push at 13) through the new shared
  `combat::damage_from`; `OP_Consider` (0x742b, 20 bytes, no hit points);
  and `OP_SkillUpdate` (0x004c, 12 bytes). `zoning::death`'s doc now says
  that stock `EQEmu` sends the bind zone and attack skill the other way
  round from both programs' structs. Coverage lists the ten; the mana and
  endurance updates and the mob percents stay unlisted. An admitted RoF2
  session still sends and offers nothing, its own death included.
- Ask for and choose a title (gap 3.6, checklist 6.516). Stock `EQEmu`
  offers the new `Capability::Titling`; P99, TAKP and Quarm do not, and
  `EQMac` and RoF2 build neither request (RoF2's numbers are owed, inferred
  for RoF2, 6.489). `GameCommand::RequestTitles` sends `OP_RequestTitles`
  (0x5eba) with an empty body (inferred, pending read R49-14b), one at a
  time with a 10 s lapse and the session's 500 ms settling after an answer.
  The answer, `OP_SendTitleList` (0x3e89), is read into
  `WorldEvent::TitleList` of `names::TitleChoice` (id, prefix, suffix): the
  count is bounded by the body before anything is allocated, each text is
  capped at 63 bytes, a repeated id keeps its first entry, and a zero id, a
  missing NUL, a short body or trailing bytes refuse the whole list to the
  gap log. A list no request asked for is withheld and logged as a gap.
  `GameCommand::SetTitle` sends `OP_SetTitle` (0x1f22, 8 bytes, the kind as
  0 or 1 then the id) only for an id on this admission's latest list with
  text of that kind, or 0 to clear either kind, and at most one per 500 ms
  (the server tells each to the whole zone); anything else is refused,
  nothing sent. A new admission forgets the list and the request in flight.
  Coverage lists 0x3e89 as read by the zone decoder. Nothing in a client
  calls `SetTitle` yet: the title window waits for read R49-14.
  **Breaking:** new variants on the exhaustive `GameCommand`, `Request`,
  `WorldEvent` and `Capability` (`Capability::ALL` holds 80; command kinds
  125 and 126).
- Read how RoF2's spawns look and how hurt they are, as stock `EQEmu` sends
  them, labelled inferred for RoF2 (6.518). The new `rof2_looks` module
  reads `OP_SpawnAppearance` (0x0971) through the reader every generation
  shares, now with RoF2's dialect: invisibility reads as Titanium's values,
  inferred, since the server sends both generations the same ones. It also
  reads `OP_ChangeSize` (0x4707, the encoder's rewrite of a size
  appearance), `OP_HPUpdate` (0x2828, the spawn id first), `OP_MobHealth`
  (0x37b1), `OP_WearChange` (0x7994, its 32-bit material kept whole),
  `OP_Animation` (0x7177, the action before the speed) and
  `OP_PlayerStateAdd`/`Remove` (0x559a/0x2d25), each into the event
  Titanium's reader gives, with the same refusals. One player-state reader
  now takes the generation's opcode pair. The wear change's elite material,
  hero's forge model and two unknown words are not read. Coverage lists the
  eight as read by the zone decoder, and the rest stay unlisted. An admitted
  RoF2 session still sends nothing and offers nothing.
- A Warrior's `/shield` of another player (gap 3.6, checklist 6.517).
  `GameCommand::Shield` sends Titanium's 4-byte `OP_Shielding` (0x3fe6), the
  target's spawn id widened to 32 bits, as stock `EQEmu`'s
  `Client::Handle_OP_Shielding` reads it (`Shielding_Struct`). The server
  checks the level, class, rule, recast, target and distance and answers
  each with a line we already read, so nothing new is decoded. The session
  keeps no pending state: it refuses spawn id 0 and a second shield within
  500 ms (`SocialRefused`, nothing sent). Only stock `EQEmu` offers the new
  `Capability::Shielding`: P99 waits for an attended check, TAKP and Quarm
  for their own (no `EQMac` opcode is known, and the `EQMac` encoder refuses
  it), and RoF2 sends nothing yet.
  **Breaking:** new variants on the exhaustive `GameCommand`, `Request` and
  `Capability` (`Capability::ALL` holds 79; command kind 124), and the new
  `shielding` module.
- Say the player looks for a group with `/lfg` (gap 3.6, checklist 6.515).
  `GameCommand::LookForGroup { session_id, on }` sends Titanium's
  `OP_LFGCommand` (0x68ac), 80 bytes with only the flag set
  (`listing::titanium_looking`), as `Client::Handle_OP_LFGCommand` reads
  it, under the new `Capability::LookingForGroup`, which only stock `EQEmu`
  lists (from source, not yet checked live). The server tells the sender
  nothing, so the session keeps the flag from the admission's spawn record
  and each change sent, tells a change with `ListingSet`, and paces two
  changes 500 ms apart. `EQMac` refuses. **Breaking:** new variants on the
  exhaustive `GameCommand`, `Request` and `Capability` (`Capability::ALL`
  holds 78).
- Read RoF2's spawn movement and removal as stock `EQEmu` sends them,
  labelled inferred for RoF2 (6.514). `rof2_player::position` reads the
  24-byte `OP_ClientUpdate` (0x7dfc), which RoF2 gets unencoded, into
  `WorldEvent::Position`: its position from the same bit table as the
  spawn record's, now shared, and its velocity and forward input from the
  deltas and animation on Titanium's scales. The delta heading and the
  vehicle id are not read. `rof2_player::despawn` reads the 5-byte
  `OP_DeleteSpawn` (0x7280) into `WorldEvent::Despawn`; its last byte is
  the encoder's constant. Both refuse any other length and a zero id.
  Coverage lists both as read by the zone decoder; `OP_MobUpdate` and
  `OP_NPCMoveUpdate` stay unlisted, since stock `EQEmu` sends neither. An
  admitted RoF2 session still sends nothing and offers nothing.
- Destroy coins on the cursor (gap 3.x, checklist 6.512).
  `GameCommand::DestroyCoins` sends `OP_MoveCoin` from the cursor to slot -1
  with the same kind both sides (Titanium 0x7657, `EQMac` 0x2d41), as stock
  `EQEmu`'s and TAKP's `Client::OPMoveCoin` read it: taken from the source,
  added nowhere. That the Titanium program sends it this way is inferred
  until read R49-11. It names one kind and exactly what the cursor holds of
  it, or it is refused on `CoinsRefused`; the coins leave the cursor at once
  (`Wallet::destroy`) and `CoinsElsewhere` reports it. Only stock `EQEmu`
  offers the new `Capability::DestroyingCoins`: P99 waits for an attended
  check, TAKP and Quarm for their own, and RoF2 sends nothing yet.
  **Breaking:** new variants on the exhaustive `GameCommand`, `Request` and
  `Capability` (`Capability::ALL` holds 77), and `money::CoinDestroy`.
- Open the chat server's channels link (recording checklist 6.509; the
  Titanium program's own side is pending read R49-12). From the world's
  Titanium chat-channel offer (`OP_SetChatServer`, letter `B`), where the
  server type opens one (`EQEmu`; not Project 1999, withheld until Adam
  has attended a session there, nor Quarm, TAKP or RoF2), the session
  opens one link per character on its own thread: the offered address
  through the zone handoff's checks, then the
  modern transport, then `OP_MailLogin` with the key, which stays zeroized in
  `chat_server::Login` and in no event, log or `Debug` (the transport's
  resend queue holds the login until the server acknowledges it). A login
  unanswered for 30 seconds (our bound, inferred) ends the link; closing
  never waits on the offered host's resolution. The link survives zone
  changes and ends on the character list, a camp, a stop and the session's
  end; a second offer replaces it; nothing retries. `GameCommand::ChatChannel
  { text }` (behind `Capability::ChatChannels`, offered only in a zone whose
  world opened a link) sends the text the chat server takes, refused with
  `WorldEvent::ChatChannelRefused` while the link is not up and never kept
  for later. What the server says arrives as `WorldEvent::Channel` (a
  message, a server line, the channels joined in slot order, a member
  joining or leaving) and `WorldEvent::ChannelLink` (up, or lost and why).
  Queues are bounded both ways; mail, friends and ignore opcodes go to the
  gap log by opcode and length. New `eq_network_game::chat_server`.

  Breaking: new variants on the exhaustive command, event and capability
  enums (`Capability::ALL` holds 76).
- Read RoF2's spawn records as stock `EQEmu` writes them, labelled inferred
  for RoF2 (6.510). Stock `EQEmu` sends every spawn a RoF2 client sees as
  one `OP_ZoneEntry` record. `eq_network_game::rof2_player::spawn` walks
  any of them whole, choosing each branch from the record's own bytes as
  the encoder chooses it: players, creatures and corpses, treasure chests
  and destructible objects with their three strings and fixed block, and
  non-players of races given three textures in place of nine tints and
  nine textures. It refuses contradictory bits, a property count the
  branch doesn't take, a GUID of any length but the encoder's, a string
  that doesn't end in its field, a cut and a trailing byte. The packed
  position is read from the active `Spawn_Struct_Position`; its deltas,
  animation and angle are not, and appearance and the `/who` listing stay
  default. `Rof2::messages` makes each record one spawn, and a refused
  record unreadable by opcode and length. The own-spawn reader is now a
  caller of the same walker. `Rof2Admission` keeps the first record, the
  player's own, and any later one with the player's id to itself
  (`Admission::kept`), so the player is never another spawn. The coverage
  row for 0x5089 lists the zone decoder as a reader. An admitted RoF2
  session still sends nothing and offers nothing. Still unreachable: a
  RoF2 configuration is refused before a socket opens.
- Challenge another player to a duel (6.492b). **Breaking:**
  `GameCommand::ChallengeDuel { session_id, target, created }` is a new
  variant, under `Capability::Dueling`, so exhaustive matches on
  `GameCommand` take an arm. `Request::DuelRequest` now encodes on Titanium
  (`socials::titanium_duel_request`: `OP_RequestDuel`, the other player then
  the player, each 32 bits, the order `EQEmu`'s handler needs; inferred,
  pending R49-6h). The duel feature sends a challenge only at a player in
  its spawn list other than the player, one at a time and five seconds
  apart, and refuses anything else with `WorldEvent::SocialRefused`, since
  `EQEmu` looks the one challenged up unchecked. The challenge is remembered
  for sixty seconds at most, and ends sooner on the server's
  `OP_DuelAccept`, the other client's refusal, the one challenged leaving,
  or a zone change. While it lasts, the record `EQEmu` sends the challenger
  before the duel starts opens no question and sends nothing; today it
  would have asked the player to accept their own duel. Both bounds and
  the refusal lines are ours, labelled. Only stock `EQEmu` offers dueling,
  as before.
- Send petitions, bug reports and feedback to the server's staff (gap list
  3.6, recording checklist 6.511), laid out from stock `EQEmu`'s and TAKP's
  handlers: `eq_network_game::reports` encodes Titanium's `OP_Petition`
  (0x251f) and `OP_Bug` (0x7ac2, 8736 bytes) and `EQMac`'s petition, bug
  (3340 bytes) and feedback (1131 bytes). Stock `EQEmu` lists
  `Capability::Petitioning` and `Capability::ReportingBugs`; feedback, P99,
  Quarm and TAKP list nothing until their own live checks. A bug report
  carries its category, the player's place and heading and the target, and
  leaves the interface path, time played and system information empty.
  Over-long text is refused, never cut, and two reports of a kind are 10
  seconds apart (inferred). No refusal or diagnostic repeats a report's
  text. **Breaking:** new variants on the exhaustive `GameCommand`
  (`SendPetition`, `ReportBug`, `SendFeedback`), `WorldEvent`
  (`ReportSent`, `ReportRefused`) and `Capability` (`Petitioning`,
  `ReportingBugs`, `SendingFeedback`; `Capability::ALL` grows to 75).

- Ask for a last name with `/surname` (gap 3.6, checklist 6.507).
  `GameCommand::Surname` sends `OP_Surname`'s 100-byte record (Titanium
  0x4668, `EQMac` 0xc441) with the surname as typed, under the new
  `Capability::Surnaming`, which only stock `EQEmu` lists (from
  `Client::Handle_OP_Surname`, not yet checked live; P99, TAKP and Quarm wait
  for their own checks). The encoder refuses only what the record cannot
  carry. One request waits at a time. The server's answer comes as
  `WorldEvent::SurnameAnswer`, and one to no request is withheld. `EQMac`'s
  196-byte `OP_GMLastName` now decodes to `WorldEvent::LastName`. Coverage:
  both `OP_Surname` rows and `EQMac`'s `OP_GMLastName` are read.
  **Breaking:** new variants on the exhaustive `GameCommand`, `Request`,
  `WorldEvent` and `Capability` (`Capability::ALL` holds 72).
- List what RoF2's stages read in a coverage table of its own (6.508), so
  the gap log no longer tells the login replies, the character list, the
  world's entry and handoff, and the zone's description, profile, own
  spawn and objects as unlisted. The description, profile and own spawn
  are listed as partly read, naming what the entry leaves unread. Every
  other RoF2 message stays unlisted. Nothing new is read.
- Read RoF2's player profile and own spawn as stock `EQEmu` writes them,
  labelled inferred for RoF2 (6.502): `eq_network_game::rof2_player`
  walks each serialised stream whole, refusing any count but the
  encoder's, a string that doesn't end in its field, a cut and a trailing
  byte, and `Rof2Admission` completes with the player read from the first
  profile and own spawn before the zone's description. Gems 9 to 12,
  appearance and the `/who` listing are not read. An admitted RoF2 session
  still sends nothing in the zone, and the RoF2 server type no longer
  offers server filters or gameplay departures, which it cannot honour.
  Still unreachable: a RoF2 configuration is refused before a socket opens.
- Build RoF2's zone connection and entry against stock `EQEmu` from its
  public source, labelled inferred for RoF2 (6.501). `Rof2::connect_zone`
  speaks the modern transport, and `Rof2Admission` sends the 76-byte entry,
  reads the zone's 948-byte description (only its short name, terminated
  and checked) that the zone sends RoF clients unasked, asks for the zone's
  spawns, and sends its ready once the zone's objects are sent. The profile
  and spawn are not read, so the entry completes with no player snapshot:
  the session only records, sending nothing in the zone, or ends with the
  admission error when a player is at the controls. Still unreachable: a
  RoF2 configuration is refused before a socket opens, and the handoff
  between zones still refuses.
- Record every server message the session drops or cannot read (6.500).
  `ClientEvent::Gap(Box<Gap>)` tells of each one, beside the diagnostic
  line it always wrote: a message no table row lists, one listed but left
  unread (with the row's reason), one read only at another stage, one read
  all but parts still to come, a kind no server is known to send, and one
  the decoders could not read, at the login, the world or the zone, plus
  chat decode errors, unreadable world news, character lists and login
  replies. Messages holding nothing worth reading are counted as
  `GapKind::Skipped`, which `GapKind::is_gap` keeps out of a gap list. The
  first of each gap in a stage is told at once; repeats are counted and
  told at most every 10 s and when the stage ends, with their count and
  shortest and longest body. Only the zone's first record of each gap in
  an attempt carries a sample, its first 256 bytes in hex; the login and
  world never do. No record names a host, an address, the account or a
  character. **Breaking:** a new variant on the exhaustive `ClientEvent`.
- Build RoF2's world stage against stock `EQEmu` from its public source,
  labelled inferred for RoF2 (6.499): the 488-byte login info, the character
  list into the shared selection, `OP_EnterWorld` (refusing a name it would
  have to cut), and the handoff (`OP_ZoneServerInfo`, its address terminated
  inside its field) or the zone-unavailable notice. Between zones it enters
  the character in play once the world's post-entry message arrives. It
  sends nothing stock `EQEmu` ignores. Still unreachable: a RoF2
  configuration is refused before a socket opens, and the RoF2 handoff,
  zone connection and admission still refuse.

- Take the server filters from the host and send a change again (parity
  item 6). The filter packet (Titanium `OP_SetServerFilter` 0x6563, 29
  words; `EQMac` 0xff41, 17 words) was a fixed array sent at zone-in.
  `eq_network_game::filters` now names each word's kind (`FilterCategory`)
  and mode (`FilterMode`: show, hide, mine only, group only), placed and
  valued as the servers that read it do (`EQEmu`'s and TAKP's
  `Client::ServerFilter`); `ServerFilters::default()` is today's packet
  byte for byte on both generations, and RoF2 refuses.
  `FilterCategory::modes(dialect)` says which kinds and modes a generation
  takes; `ServerFilters::fitted` and `check` use it.
  `ClientConfig::server_filters` is the character's starting set, checked in
  `validate`. `GameCommand::ChooseServerFilters` at the character list keeps
  a set for the character entered next, so its first zone-in carries it on
  every server type. `GameCommand::SetServerFilters` mid-zone keeps a
  change for every later zone-in, and sends it at once only where the
  server type reads one then (`EQEmu`); elsewhere it waits for the next
  zone-in (6.494e). The session answers `WorldEvent::ServerFiltersKept` or
  `ServerFiltersRefused`; capabilities `ServerFilters` (every server type)
  and `ServerFiltersAtOnce` (`EQEmu`). Which official option sets each word,
  and when the official client sends the packet again, are unread (6.494b
  and c). **Breaking:** `chat::server_filters` and `quarm::server_filters`
  are removed; `ClientConfig` gains the public field `server_filters`; the
  new variants on the exhaustive `GameCommand`, `WorldEvent` and
  `Capability` enums.

- Build RoF2's login stage, character list and world login info from
  `EQEmu`'s public source, labelled inferred for RoF2 (6.497). The login is
  Titanium's exchange read through the login server's opcodes for clients
  newer than Titanium, and skips the expansion list it may send them.
  `characters::decode` reads RoF2's variable-length list, bounding its count
  by the body before keeping anything. `world_login::rof2` writes the
  488-byte login info, and refuses an account or key that doesn't fit
  rather than cut it. Nothing reaches any of it yet: a RoF2 configuration is
  still refused before a socket opens, and the RoF2 world stage still
  refuses.

- Put a Rogue's poison on the weapon (`OP_ApplyPoison`, Titanium 0x0c2c and
  `EQMac` 0xba41): `GameCommand::ApplyPoison { slot }` sends the 8-byte
  `{slot, success 0}` the servers take, in each generation's slot numbering,
  behind the new `Capability::ApplyingPoison`. One request is in flight at a
  time; the slot must hold an item of type 42. The server's answer arrives as
  `WorldEvent::PoisonAnswered` and its own inventory record removes the
  poison. Refusals come as `WorldEvent::ApplyPoisonRefused`. The program's
  side (what starts it, its gates, its lines and any wait) is pending read
  R49-1. **Breaking:** new variants on the exhaustive command, event and
  capability enums.

- Speak in a chosen language (parity item 9). `chat::Tongue` (private fields,
  made only by `Tongue::COMMON` or `Tongue::spoken` from the player's profile)
  travels on `SendChatInAdmission { tongue }`, and `chat::encode_outbound_in`
  writes its language and the speaker's skill on both generations;
  `encode_outbound_for` and `SendChat` stay in the common tongue. A received
  channel message keeps its language and skill as sent
  (`ChatEvent::language`, new `ChatEvent::language_skill`). What the official
  client puts in the skill field, and on which channels, waits for program
  read R49-4b; RoF2 refuses. **Breaking:** `SendChatInAdmission` constructors
  need `tongue`; the JSON of every channel message gains `language` and
  `language_skill`.

- Answer another player's challenge to a duel. On stock `EQEmu` a challenge
  now also comes as `WorldEvent::DuelOffer` (`socials::DuelQuestion`, with an
  admission-scoped `ConsentId` of the new `ConsentKind::Duel`), and
  `GameCommand::AnswerDuel` sends the player's yes (`OP_DuelAccept`) or no
  (`OP_DuelDecline` 0x3bad) as `Client::Handle_OP_DuelAccept` and
  `Handle_OP_DuelDecline` read them: the challenger, then the player. The
  question has no local deadline; zoning, death, logout, a newer challenge,
  the duel's start and the challenger's refusal close it as interrupted.
  `EQMac` encoders for TAKP and Quarm (`OP_RequestDuel` then `OP_DuelAccept`
  for a yes, `OP_DuelResponse` for a no) are built and tested but no server
  type lists them yet; P99, TAKP and Quarm wait for live checks. The field
  order the official programs write is inferred (checklist 6.492). New
  `Capability::Dueling`, `Request::AnswerDuel`, `Request::DuelRequest` and
  `WorldEvent::DuelRefused`. **Breaking:** the new variants on
  `ConsentKind`, `Capability`, `GameCommand`, `Request` and `WorldEvent`.

- Add RoF2 as a placeholder client generation and server type, so each
  decision a third generation needs has an answer before any of it is built.
  `GameDialect::Rof2` and `ServerProtocol::EqEmuRof2` (stock `EQEmu` with the
  RoF2 client) exist, and nothing pretends they work. A configuration naming
  `EqEmuRof2` is refused when the client is made, before a socket opens. Its
  wire keeps every refusing default, its server type offers no features, and
  it has no `FromStr` name, so `EQ_PROTOCOL` and similar settings cannot
  choose it. Every packet layout refuses it ("not built for RoF2 yet"). Each
  program rule without a read is absent (`None`), or Titanium's value on an
  arm of its own marked `inferred for RoF2 (6.489)`; grep that token for the
  owed reads. `Wire::reaches()`, `Wire::window_closes()` and
  `Wire::update_counter()` lose their defaults, so each generation states
  its own. Titanium and `EQMac`
  answer as before. **Breaking:** the new variants on both exhaustive enums.

- Place the cursor item as the Titanium program's `/autoinventory` does
  (read R46). `Inventory::auto_inventory_destination` tries the worn slots
  first (onto the same item, else the first empty slot the item fits and
  the player can wear), then a general stack, a stack inside a bag (not
  for a bag), the first empty general slot and the first empty bag slot
  (not for a bag). It plans one move and its quantity; what a stack has no
  room for stays on the cursor. `auto_store_destination` keeps its rule
  for the inventory window's button, sharing the same checks.

- Make every client-generation and server-type decision exhaustive, so a
  third generation (such as RoF2) or a new server type breaks each place
  that must decide what it does, instead of taking another generation's
  branch. `GameDialect` and `ServerProtocol` are no longer
  `#[non_exhaustive]`, every comparison on one of their variants is a
  `match` naming them all, and a test refuses new comparisons in production
  code. No behaviour changes. **Breaking:** `ServerProtocol::is_titanium()`
  is removed; match on `GameDialect::from(protocol)` instead. Matches on
  either enum in other crates no longer need a `_` arm, and one left in
  place now warns as unreachable.

- Send a social emote as the Titanium worker builds it, with no local
  echo (read R47b). The program prints a social emote's own line itself,
  so the session's echo for every emote doubled it. `GameCommand::SocialEmote`
  and `Request::SocialEmote` send `OP_Emote` (0x547a) with the header
  0xffffffff and the text unchanged, leading and trailing spaces kept;
  `socials::social_emote` encodes it. Empty, too-long or NUL text is
  refused. On Project 1999 the one leading space is dropped and the text
  goes through the P99 sender, which keeps its zero header. `/emote` keeps
  its echo and its sender. **Breaking:** the new `GameCommand` and
  `Request` variants.

- Cast a discipline as the program does after read R48b.
  `disciplines::titanium_cast` takes target 0, which the cast carries when
  nothing is targeted, and the host now sends the current target for a
  self discipline too. `GameCommand::UseDiscipline` gains `timed_self`:
  the session refuses on an active discipline only for a spell with a
  duration that targets its caster (0x4bb039), so any other discipline goes
  to the server while one runs. **Breaking:** the new field.

- Settle a cursor destroy by the refusal window, as a move is settled.
  `EQEmu` deletes the destroyed item without sending any record, so the
  prediction used to stay until the next full snapshot, and every destroy
  after the first in a zone was refused ("Wait for received inventory
  state..."). The session now starts the two-second refusal window when it
  sends a destroy, and `InventoryUpdate::Settled` ends the prediction and
  drops the item's mass from the carried-weight bound. A resent cursor item
  inside the window still restores it.

- Send an emote command's animation and any listed discipline, as Titanium
  does (reads R47 and R48). `GameCommand::EmoteAnimation` sends
  `OP_Animation` (0x2acf) with the player's spawn, speed 10 and the
  animation, under `Capability::Emoting`; the program sends it after the
  emote's `OP_Emote` only while the player stands still, so the host checks
  the stillness and the session sends nothing for a player with no spawn or
  known not to stand. `GameCommand::UseDiscipline` now takes the spell, its
  reuse timer and the cast's target from the host's spell data, in place of
  the two Warrior disciplines: the session refuses a spell the server
  hasn't listed (or no list at all) and one whose timer runs, and sends the
  ordinary cast with the discipline slot (10), as 0x4bb39d does. The target
  the program gives that cast is unread; the host sends the player's own
  spawn for a self spell and its gem-cast choice otherwise (recording
  checklist 6.488). `disciplines::titanium_cast` takes a spell and a
  target. **Breaking:** `UseDiscipline`'s fields changed, and exhaustive
  matches on `GameCommand` and `Request` need the new variants.

- Send Titanium's `/yell` and `/stopcast` (read R45). `GameCommand::YellForHelp`
  sends `OP_YellForHelp` (0x61ef) with an empty body and no target or
  posture test, under the new `Capability::Yelling`, on both Titanium
  servers. `GameCommand::StopCasting` stops a Bard's song as
  `GameCommand::StopSong` does and otherwise sends the same empty
  `OP_ManaChange` (0x4839); the cast ends when the server answers. It is
  under the new `Capability::StoppingCasts`, offered on `EQEmu` only and held
  off P99 until checked there. The program also refuses a mounted
  player, which the session doesn't decode, so the host must refuse
  `/stopcast` for one (recording checklist 6.487). The
  Mac-era generation offers neither. **Breaking:** exhaustive matches on
  `GameCommand`, `Capability` and `Request` need the new variants.

- Decode each Titanium spawn's body type and say what the program does by
  it (read R44): `SpawnState::body_type` is the dword the program copies
  from record +0x14f, `None` for the Mac-era records, whose layout is
  unread there. `GameDialect::targeting` gives Titanium's
  `targets::TargetRules`: the target setter refuses body types 11, 60, 65
  and 67, silently and with nothing sent; interaction refuses 11, 65 and 67;
  body types above 64 are hidden. This lets a front end stop selecting the
  controller spawns `EQEmu` treats as untargetable. **Breaking:** exhaustive
  constructions of `SpawnState` need the new field.

- Swim as the Titanium program does (Batch M slice 4; reads R33, R33b, R39
  and R41): `movement::water` holds the per-frame wet step of `eqgame.exe`
  0x4bfe0f (`WetVelocity`, `WetInput`, `WetCaps`, the coefficient sets, the
  0.4 gravity that also applies in water, the 500 ms jump push), stepped at a
  fixed 1/60 s, the frame phase carried from sample to sample (recording checklist 6.483).
  `GameDialect::water` gives the generation's `WaterRules`: `WT` water takes
  level 5's set, poearthb its own, and thegrey has no breathable air.
  `SpeedState` gains `swimming_skill`; `MotionCalibration` and
  `WorldEvent::MotionState` gain `swim` (`SwimCalibration`), whose forward
  inputs add the Swimming skill x 0.01 standing. The session bounds a
  Titanium swim sample per axis instead of one 3-axis budget, lets a rooted
  swimmer sink, and lets a sample leaving the water rise on after a swim sample. Level swimming
  at run 0.7 and skill 100 goes from 4.7 to 17 units a second, and looking
  straight up rises about 34. **Breaking:** exhaustive constructions of
  `SpeedState`, `MotionCalibration` and `WorldEvent::MotionState` need the
  new fields.

- Restore a rider arriving aboard where the Titanium program does (read
  R36c): its profile loader keeps the profile's y and z as the boat-frame x
  and y and the departure's saved z as z. `Arrival::Aboard` gains `profile`,
  the fields as received, beside `local`, the point used. The departure z
  is kept across zones with the aboard flag, forgotten with the vehicle and
  at character change. **Breaking:** exhaustive constructions of
  `Arrival::Aboard` need `profile`.

- Fill a boat transfer's Titanium save checksum (0x4b6c04, read R36c: the
  reflected CRC-32 table from a seed of all ones, no final inversion) as
  `zoning::save_checksum`; the save's y, x, z order is now read, not
  inferred. Ordinary transfers still send the all-zero save.
- A refused transfer that carried a rider on from the arrival wait forgets the
  boat and stands the player at the zone's safe point (0x496d00), publishing
  `Arrival::SafePoint`, so a host's hold ends.

- Speak Titanium's boat-riding wire on EQEmu and P99 (`Capability::BoatRiding`):
  `OP_BoardBoat` on the first linked one-second tick, `OP_LeaveBoat` after two
  unlinked ticks, zoning with reason 2 when the server's vehicle transfer
  (`0x0b71`) names the saved boat, a boat transfer's save with the place in
  the boat's frame and the vehicle name (none when carried on from the
  arrival wait), and an
  arrival aboard from the profile's vehicle name: `WorldEvent::Arrival`, one
  `GameCommand::ArrivalPlacement`, `0x3710` and a 60 s hold when the boat is
  missing, then the zone's safe point. See [boat riding](docs/boat-riding.md).
- **Breaking:** `Request::SaveOnZone` carries a `zoning::Save`, and
  `zoning::titanium_save_on_zone` takes one; `Save::default()` is the empty
  save ordinary transfers still send. Exhaustive matches must handle `Message::VehicleName`,
  `Message::VehicleTransfer`, `WorldEvent::Arrival`, `Capability::BoatRiding`,
  `GameCommand::ArrivalPlacement` and the new `Request` variants.

- Deliver staged environmental AA inputs as one ordered admission batch and
  one following liquid revision, retaining every fact and withdrawal.

- Observe local EQEmu Titanium AA selections and definitions for environmental
  decisions, preserving signed effects, exact selected IDs and unknown data.

- Decode Titanium boat-control echoes with unread fields retained. Offer an
  EQEmu acquire/release owner bound to current rider support, with explicit
  pending, acquired, release and unconfirmed states; no vessel movement sender.

- Keep the worker's guild membership current from received own-spawn listing
  changes, so ordinary guild requests follow joins and leaves without a relog.

- Add typed skill/language training subjects, `titanium_practice`, and current
  language offers from received availability flags and native race/class/level
  eligibility on every server with supported language training.
  Existing ordinary-skill training entry points remain available.
- Unresolved EQEmu language operations withdraw points and potentially spent
  purse balances; free operations preserve known coins. Late skill updates
  remain proficiency news. Expired or invalidated unanswered openings retire
  that trainer ID until fresh admission; answered offers can request a new offer.
- **Breaking:** `TrainingOffer` requires `language_caps`; `TrainingReply` requires
  `language_flags`. Exhaustive matches must handle `TrainingRequest::PracticeAt`,
  `Request::Practice` and `PracticeUpdate::OutcomeUnknown`.

- Add `SendChatInAdmission` for callers that must bind queued chat to its
  originating zone visit. Stale requests are refused before feature handling;
  ordinary chat payloads, capabilities and timing remain unchanged.
- Replace the unmaintained `chrono` dependency with `jiff`, preserving integer
  Unix seconds and UTC RFC3339 timestamps with exactly three fractional digits.

- Document read 6.475: Titanium's movement, vehicle and physics paths do not
  stand the player solely for passive displacement. Positive damage has its
  own posture path. Successfully submitted local fall, drowning and lava
  reports now share the received-damage sitting/kneeling and stay-seated
  policy, without synthesizing damage or hit points. Refused, unsent and zero
  reports stand no one; lying and Mac-era posture behavior are unchanged.
  Mounted state and an unidentified guard remain separate parity gaps.
  Regressions cover fast carrier translation and yaw
  during a final named walk-off and ordinary steps after it.

- **Breaking:** remove `UpdateGate::Changed`. Hosts choosing a movement gate
  use `UpdateGate::Flagged` with the generation's `FlagRule` and packing.
  `UpdateGate` is non-exhaustive. The production gates already use the
  native rules; counter, pad and transaction tests now use flagged gates too.

### Added

- Titanium carrier support: `Support::Spawn`, canonical receipt-time carrier
  reports, shared report-driven poses and boat-relative movement validation.
  Carrier translation and yaw preserve the rider's own motion budget and
  update pacing; superseded reports and rider reach are bounded. Pitch and
  boat zoning remain unsupported, and live rides remain unverified.

- **Breaking:** a ducking player moves ducked on Titanium, as its program
  does (program read R23; checklist 6.460).
  - `GameDialect::ducking()` gives `feign::DuckRules { moves_ducked,
    jump_stands }`: both true on Titanium, both false on the Mac-era
    generation, whose program is unread and keeps the earlier rule.
  - `SpeedState` gains `ducking`. On Titanium a ducking player takes the
    sneaking branch: forward and back on half the walk speed whatever the
    run flag, the buffs' modifier added after the halving, back in water
    on the land rule, a strafe halved. `EQMac` takes no account of it.
  - On Titanium a ducking or sneaking player strafes on the selected base,
    halved: the run speed running, the walk speed walking, both rates in
    the grant as standing (program read R28; 0.35 and 0.23 of input for a
    0.7 run and 0.46 walk). Sneaking no longer takes the Mac-era rule there.
    A grounded diagonal, ducked or sneaking, now has rates: its sideways
    part is half the raw base as standing, and its forward part the ducked
    one. The Mac-era rule is unchanged.
  - The session sends no stand for a ducked step where `moves_ducked`
    holds. The grant follows the stance when it changes, in place, with
    nothing withdrawn, and a stand goes back the same way.
  - A jump while ducking sends the standing report, then the jump, where
    `jump_stands` holds. A refused jump stands no one.
  - Ducked headroom, and whether the jump after a stand refused under a
    low ceiling still launches, are the host's (the second is inferred).
- **Breaking:** buffs that land on the player get the slot the Titanium
  program gives them (program reads R3 and R20). Click-off now works for
  them, and the buff order matches the program's.
  - `buffs::placement::titanium_player` ports the slot finder (0x414071)
    as far as it is read:
    - a held spell keeps its first slot;
    - otherwise the lowest empty slot of its band is taken;
    - the bands come from spells-file fields 83, 154 and 168;
    - the normal band ends at 15 while the effect-327 sources are unread.
    A held effect-148 spell leaves a new landing without a slot (a held
    spell's refresh keeps its slot), as does a full band, with the reason.
    Steps 2 (partly read), 4 and 6 and the top past 15 are unported
    (recording checklist 6.457). `GameDialect::buff_placement` gives the
    finder: Titanium's, and none for the Mac-era generation.
  - `BuffUpdate` carries how the program applies an update (`Applies`)
    and the record's type byte. On Titanium:
    - action 3 writes its slot;
    - action 1 fades by spell;
    - any other action updates by spell in place and adds nothing.
    A fade matches only the same nonzero type byte, so a type-4 record
    outlives a type-2 fade, as in the program (a known parity item,
    recording checklist 6.459).
    The Mac-era parser keeps today's meaning (`Applies::Received`).
    `BuffUpdate::replaces_slot` is now a method.
  - `cancel::Request` carries its `Placement`. A modelled slot sends the
    record the program would hold there (R20 (d); fields not read are 0),
    and a fade of its spell in any slot counts as the fade observed. The
    modelled slot and record, and the public-server pair with a modelled
    slot, are inferred until checked live (recording checklist 6.458).
  - The session's received slots follow the same rules.
  - `SpellEffect::took_hold` is the one test that a spell took hold: an
    action of kind 231 whose flag carries `buffs::TOOK_HOLD` (0x04), as the
    Titanium program tests it (0x47f614). A flag with other bits set
    besides 4 now counts; the session's buff slots use it.

- **Breaking:** the player stands at zone entry, as the Titanium program
  does (program reads R24 and R24b). `SpawnState::posture` and
  `PlayerState::posture` carry the record byte the program reads as the
  spawn's posture (+0x57); a host constructing either adds the field. The
  new `GameDialect::entry_stand()` (`feign::EntryStand`) says whether the
  generation stands the player as the zone is set up, and whether that
  stand sends. On Titanium the posture before it is a report staged
  before admission, else the own record's: when that is standing the
  session reports a lone `Standing` for the player, the posture they enter
  with, and sends nothing, so the first stand after a zone-in no longer
  sends one the program skips; otherwise it reports that posture, then
  `Standing`, a change like any other, and sends one standing report. The
  send rests on two inferred links in the read (checklist 6.461). The
  Mac-era client is unread and keeps an unknown posture. Other spawns' record postures are decoded but not yet used.
- **Breaking:** a root is a grant at no speed, not a withdrawal. The
  program zeroes the speed and the velocity once and blocks nothing else
  (read R18, 0x410fdb; read R22): a rooted player turns, by keys and by
  mouse, and nothing carries them. `MotionSession::root` grants the player
  rooted, and the session tells the host with a `MotionState` of 0 units a
  second. A song pulse or any other change while rooted withdraws nothing,
  and the fade grants the speed again in place. `Speeds::rooted` says a
  rule's None comes from a root. The new `GameDialect::root()` rules
  (`movement::RootRules`): on Titanium the autorun latch survives a root,
  so the player runs again by themselves once it ends (R22 b), and a jump
  is refused while rooted (R22 c; the program's exact -1000 edge, which
  roots and still jumps, is not modelled). Mouse-look pitch, which the
  program holds while rooted in camera modes 0-2 (R22 d), is not modelled.
  The Mac-era client is unread and keeps the earlier result: the root
  drops the latch and refuses a jump. Stun, freeze, death, transfer and an
  unproven start still withdraw. Recording checklist 6.451.
- A speed change for a player already granted applies in place.
  `MotionSession::calibrate` no longer resets the guard's stamps when a
  grant, at a speed or rooted, is already in force. The next sample's
  allowance runs at each speed in force over its own span of the sample's
  interval, so every change while moving no longer refuses the sample
  under way as stale, too frequent or over its budget. Run 2026-11-11
  rows 4 and 5 held 0.12 s and 0.38 s at each snare. Time rooted earns no
  distance, and distance made before a root is still taken within its
  allowance. More than 16 changes before one sample count the earlier
  time as rooted, the slower side. A correction, an admission and a
  withdrawal still start afresh.

- **Breaking:** Feign Death by a spell lies the player down, as the Titanium
  program does on the landing (program read R21): the new host command
  `GameCommand::FeignLanded` sends one lying posture report, or none when
  the player already lies, and stops the movement in hand, with no feign
  request and no auto attack change. Only the host holds spell data, so it
  sends the command for a spell with effect 74 that took hold on the player
  (`SpellEffect::took_hold`, the one landing test).
  `GameDialect::spell_feign_lies()` says whether the generation's program
  lies the player down; the Mac-era client's is unread (checklist 6.455).
  `PostureOptions::feign_standing` covers the spell too: the landing sends
  nothing and the next stand is owed (checklist 6.454). The dismount and the mount buff's removal the program
  also does are not modelled (6.455).
- **Breaking:** Feign Death and the lying posture as the Titanium program
  handles them (program reads R19, R19b and R19cd), through the new
  `GameDialect::lying()` rules (`feign::LyingRules`). The Mac-era client
  keeps the earlier rules until its program is read.
  - The press lies the player down with `OP_SpawnAppearance` (type 14, 115)
    before the feign request, as the program does; until now Titanium sent
    only the request, so the player never lay down and moving never ended
    the feign. A press while lying sends the request alone.
    `Request::FeignDeathPosture` now encodes on Titanium.
  - The feign request carries the program's 12 bytes
    (`feign::titanium_request`): the player's own id, then a word the
    program fills only from its own damage reports, sent as 0, and zeros
    (checklist 6.455, inferred). `Ability::encode` returns `None` for Feign
    Death.
  - While the player lies, a step that goes anywhere is refused and nothing
    stands them to move; a turn alone, a fall and a lift's carry go on, and
    jumping is refused. Lying down stops the movement in hand. The grant is
    not withdrawn.
  - Any `OP_Damage` record of more than 0 to the player stands a sitter or a
    looter, whatever its type; lying and ducking players stay as they are.
  - `GameDialect::posture_ends_attack()` tells a host whether a posture other
    than standing ends auto attack: not on Titanium (read R19cd 3), still
    on the Mac-era client (unread, checklist 6.455).
  - A cast while lying is refused with the program's line, whether a gem,
    a knight's special or an item click (items inferred, checklist 6.455):
    `WorldEvent::CastRejected` and `WorldEvent::ItemUseAction` have a new
    `string_id`. Disciplines are not refused (the program does not check).
  - `GameDialect::sit_toggle()` (`feign::SitToggle`) tells a host how the
    sit/stand toggle chooses: Titanium's stands a lying or looting player
    and refuses under a mesmerize with line 0x3050 (reads R19, R19b 5); the
    Mac-era client's is unread and does neither (checklist 6.455).
  - On Titanium a posture request for the posture the player already has
    sends nothing, as the program's posture routine does (read R19); the
    Mac-era client still sends it (`GameDialect::skips_unchanged_posture()`,
    unread, checklist 6.455). A stand the server still needs goes out once
    whatever the posture: the one that ends a feign pressed standing, and
    the one that stops a camp's timer.
  - Two departures in `MovementOptions`'s new `posture`
    (`movement::PostureOptions`), both off by default and on both
    generations: `feign_standing`, where the Feign Death press sends the
    feign request alone and the player stays standing, and the next stand
    asked for still goes out once to end the feign (checklist 6.454),
    and `stay_seated_when_hit`, where a hit leaves a sitter or a looter
    seated (checklist 6.453).
- **Breaking:** movement is no longer stopped while the player's speeds
  are worked out again. The program stops input only for a stun, a freeze
  (posture 0x66) or a posture other than standing or ducking (read R7,
  0x4a1d74); it recomputes its speed modifier on buff and effect changes and
  once a second, and input reads the result as it stands, without waiting
  (read R18, 0x410fdb); a server position update to the player's own spawn
  touches no input (R18, 0x451aba). A buff on the player (each Bard song
  pulse), a buff list, a level, a spell landing, an EqMac fatigue change, a
  change in belongings and a server's correction now ask the host for new
  inputs while the grant in force goes on: no `MotionState` withdrawal and no
  stop sent. A correction, a same-zone relocation (a summon or GM move) and
  a local pad crossing take one path: they move the player and stop them
  there, then grant the speeds in force again at once, with no withdrawal
  (read R18: 0x451aba and the same-zone warp 0x48aaa6 touch no input).
  `SameZoneRelocated` and `LocalPadCrossed` now reach the host after the
  grant rather than after a withdrawal; without a grant both still wait. With a grant in force, an
  `Unavailable` answer, or none, keeps the last inputs in force with no time
  limit, noted once in the diagnostics. `SpeedInputState` gains
  `Bounded { inputs, missing }`: the inputs from every term the host can
  prove, each unproven term taken from the last report where one exists and
  left out where none does, with every term it can prove slower applied (a
  root or a snare it can evaluate, a weight it knows). The host proves the
  terms, and the session takes a bound as reported, with or without a grant
  in force: a bound is faster than the last one only where a proven slower
  term ended, and without a grant it starts one, so a root that fades while
  a song runs, or a zone-in with a term unproven from the start, moves at
  once. Only `Unavailable`, with nothing provable, still waits. A report
  under a pending ground drop's weight bound is taken at the bound; with a
  grant in force and no bound to check, it is taken as a bound with the
  weight no lower than in force, so a root in it still applies. The first
  exact report replaces kept or bounded inputs in place, noted in the
  diagnostics. Root, stun, freeze, death and zone transfer still stop
  movement as before. The same rules apply on every server type; only the
  waits are removed. Recording checklist 6.451, a stated excess by Adam's
  decision of 11-10: while the speed inputs are unproven, the last grant is
  kept without a time limit and can be faster than the official client
  would allow; where no earlier report exists, unproven terms are left out.

- **Breaking:** Titanium class tools (Batch B2). On EQEmu and P99:
  - Fishing is offered on P99 too. Before it is sent, the program's checks
    run in its order, each refusing with the program's own line: already
    fishing (its own 2 s flag, apart from the 12 s wait), in water, an item
    held, attacking, no pole, the pole not in hand, no bait. A standing
    player sends the pole's cast animation first, and the cast's line comes
    with `WorldEvent::AbilityUsed`'s new `string_id`.
  - New abilities `PickPockets`, `Begging`, `SenseTraps`, `DisarmTraps`,
    `PickLock` and `Tracking`.
    - Pick Pockets and Begging send the program's 18-byte ask, carrying the
      player's own skill value (`Request::AskOf`,
      `abilities::titanium_ask`).
    - The trap skills send the program's treasure messages at a target of
      class 62 (`Request::AtTreasure`; checklist 6.432, inferred). Pick Lock
      has only that message, and does nothing and says nothing at any
      other target; `Ability::encode` returns `None` for it.
    - Each waits ten seconds on its own timer, the program's value. A press
      while a class tool's wait runs, Fishing's 12 s too, sends nothing. The
      program passes over it in silence; the session refuses it as for any
      other ability, with no line of the program's, so the host says its
      own (a chat-only departure, checklist 6.447).
    - The server's answer to a beg or a pick comes as the new
      `WorldEvent::AskAnswered` (`abilities::titanium_answer`), read as the
      program reads it: nothing, coins of one kind, or on a pick an item by
      name. `AskAnswer::line` gives the program's line and what it names.
      The session adds the coins to the purse, as the program does, and
      tells `WorldEvent::Coins`; the server sends no money update.
    - A player's pockets are picked only with both open to player combat,
      each spawn's flag from its appearance updates, refused otherwise with
      the program's line for whichever is not, and then only within five
      levels each way, refused in this library's words since the read does
      not yet tell the two lines apart; its ruleset term is unread (6.438).
    - Begging's unseen check reads the player's own invisibility (6.430),
      and the 15-unit reach is measured as the bandage's (6.431).
  - Tracking, as the program does it (`tracking`, `Capability::Tracking`).
    - Pressing it asks for the list and then sends the program's empty
      second message. The new `GameCommand::Track` refreshes the list,
      follows a spawn from it, or stops (`TrackAction`).
    - The server's list comes as `WorldEvent::Tracking` with
      `TrackingUpdate::Listed`: the server's order, at most 1,000 entries,
      leaving out spawns the session does not know.
    - Following names the spawn to the server and starts a six-minute
      limit. The session tells the host every two seconds where the spawn
      lies against the player's heading, as one of the program's five
      bearing lines (`TrackingUpdate::Bearing`), and sends nothing more.
      The side word follows the heading's sense, turning left raising it:
      a spawn the way the heading grows lies on the left. This was checked
      against 22 side lines of a local run, which fixed the side; the
      program's own bearing routine and sign test are unread (6.441).
    - Following ends with the program's line when the spawn is gone or
      six minutes pass. Stopping while following sends the program's stop
      and says nothing.
    - The Ranger's sort and colour filter are not offered.
  - A frozen player uses none of these, with the program's line. The
    program's mount gate, which allows only Tracking and Fishing of these
    while riding, is not followed, since the host does not tell the session
    when the player rides; a mounted player's request goes out where the
    program refuses it (6.443).
- Between a beg or a pick and its answer, the Titanium program holds
  casting and some window actions until the server replies (its
  reply-pending count, read at 0x9248d0); the session does not yet
  (checklist 6.452).
- **Breaking:** a Bard's `/stopsong` follows the Titanium program's song
  state (read R16) and is offered on P99 too. The song is the player's last
  own cast start: every gem cast, and an item cast over 100 ms. No song ends
  on its own. The server's cast update with keep-casting clear ends it as
  the program does: spell -1 or a spell of the class always, and for a Bard
  an item's song or the very spell named; a new cast takes its place.
  `StopSong` sends the empty request only for a song of a class spell,
  even while another cast is held, as the program does. An item's song of
  another spell ends locally with nothing sent. With no song, or for any
  other class, nothing is sent. A memorized spell is taken as the class's,
  and every gem cast as over 100 ms, since the session has no spell table:
  so an item's song of a class spell that is not memorized ends locally
  where the program sends the stop. The program's memorize path, where a
  Bard memorizing a spell stops the song with the stop sent and restarts
  that gem's, is not followed (checklist 6.442).
  - `GameCommand::UseAbility` gains `in_liquid`, the host's own liquid test,
    which Fishing refuses on. `Ability::encode` returns `None` for the two
    asks. The session follows the auto-attack toggle it sent, and the
    player's death turns it off.
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

- **Breaking:** a scribe or memorization the server refuses for the
  player's level ends at once. `EQEmu` answers such a request with only its
  `SPELL_LEVEL_TO_LOW` line (string 12048) and leaves the cursor alone; the
  session now decodes that Titanium formatted message as the new
  `Message::BookLevelRefused { arguments }` (a host matching `Message`
  exhaustively adds the arm), reports `BookActionStatus::Rejected` naming
  the server's line and its arguments, and releases both the spellbook and
  the cursor custody hold. The refusal is an answer, so the same request can
  be sent again. The line still reaches the host as chat. Before, the scroll
  stayed held ("Wait for scribing to finish") until the next zone.
- A scribe left unanswered for 30 seconds now also releases its cursor
  custody with `BookActionStatus::Unconfirmed` (inferred, recording
  checklist 6.462), so a refusal the session does not read, such as a
  plain-text one, no longer holds the cursor until the next zone. A late
  book update for it holds the cursor again for its removal; a late cursor
  removal is a consumption while the model still holds the scroll there,
  and otherwise marks the inventory stale (`InventoryUpdate::Invalidated`).
  The same edit stays refused until the next admission.
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
