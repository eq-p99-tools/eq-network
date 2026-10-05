# Changelog

This project follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

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
- A dead player on TAKP asks for their bind point once a death pause is
  over, one value in the transfers feature, which is zero for now: at once,
  as Adam chose while that choice stays open. The official client's wait
  is unrecorded (inferred). A death while a transfer is under way waits for
  its answer, and a second word of the same death changes nothing.
- A tell's echo on channel 14 is `ChannelName::TellEcho` on the `EQMac`
  wire too: TAKP echoes a delivered tell to its sender on
  `ChatChannel_TellEcho` (14), as `EQEmu` does on Titanium's.

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
