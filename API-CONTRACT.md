# API and integration contracts

## Titanium liquid reports and their damage record (read R49-3)

Every Titanium session, `EQEmu` local or not and P99, offers `Drowning` and
`LavaHeat`, as the Titanium program reports them on any server. After each
accepted Titanium report, the session sends `OP_EnvDamage` and then the
program's damage record, `OP_Damage` 0x5c78
(`hazards::titanium_damage_record`, read R49-3 c). Falls get the same
record (R49-3a). `EQEmu` relays the record to every client in the zone,
the sender included for every type but a fall, and applies nothing. P99's
handling is unknown, and its first use there is the attended liquid row.
The record is skipped where the player's HP,
less the amount, would be below -10, the program's death path. The session
judges that from the server's last HP plus what the items add, and only
where it can count the items (inferred, 6.491b). No Titanium server type
counts them yet, so the record goes.

On Titanium the session also refuses any report in the freeze stance
(posture 102) and for a carrier race (0x8d, 0x48, 0x49, 0x72, 0x194), as
the program's shared report function does (R49-3 d, R49-3a).

`WorldEvent::HazardReported { session_id, hazard, amount, record }` follows
every report the session sent, falls included, so a front end can print the
program's own line (R49-3 e). `record` says whether the damage record
followed, so a front end can expect its echo.

Zone 200's lava is `LiquidHazard::ZoneHeat`, sent as type 244 under
`LavaHeat`. Velious water's cold is `LiquidHazard::Cold`, sent as
`Hazard::Freezing`, Titanium type 246, under the new `Capability::ColdWater`.
It's offered on every Titanium session and shares the feet tick with lava
(reads R49-3 b and R49-26: region kind 8, `VW`, at the origin). The host
works out the amount, 50 − cold resist / 3 with a floor of 10. TAKP and Quarm
don't offer it until a Mac read. **Breaking:** `LiquidHazard::Cold` and
`Capability::ColdWater` on exhaustive enums; `Capability::ALL` grows to 91.

**Breaking:**
- `WorldEvent::HazardReported`;
- `LiquidHazard::ZoneHeat` and `Hazard::ZoneHeat` on exhaustive enums;
- `Hazard` is now `Serialize`.

## An item's song and the host's spell data

`ItemUse::installed_class_spell: Option<bool>` is the host's word, from its
installed spell data, on whether the clicked item's spell is the player's
class's: the class's level for it (spells fields 104-119) is neither 0 nor
255, read R49-21b's song test. The program also needs the spell's byte
+0x1c4 clear, which no spells field is mapped to yet (R49-48 c). A host
without an installed record (no spell data, or P99's replayed definitions)
sends `None`. The session uses it only for a song it is already keeping
from that click: to choose between sending the Bard's stop and ending the
song here, and, under the Titanium song rules below, whether that song is
one a Bard is singing. It never starts a song, names a spell or sends a
stop with no song. `None` keeps the memorized test (6.442, 6.542).
**Breaking:** every `ItemUse` literal names the field.

## A Bard's song under the Titanium program's rules

The Titanium server types (P99 and Titanium EQEmu) follow read R49-21 while
a Bard sings, that is, while the session keeps a song of the class's spell
(a gem's, once its own cast start kept it, or an item's):

- **Nothing is held because of the song.** A strike, `UseItem`,
  `UseBeltItem`, any `MoveInventory`, `RangedAttack` (inferred),
  `MemorizeSpell` and `ForgetSpell` (both inferred, R49-48 a) go at once.
  The rest that wait on a cast still wait. Any other cast, including an
  item cast started alongside the song, holds as before.
- **A gem cast (`CastSpell`) while singing stops the song** (the empty
  `0x4839`, as `StopSong` sends) **and casts nothing.** The host gets the
  stop's `WorldEvent::SongStopped` (below) and no `CastRejected`. The gem
  timers the program resets on a stop are the host's to reset (6.542).
- **A forget's answer** (the server's gem clear, mode 2) **stops the song**
  the same way. A memorize's answer leaves it.
- **An item cast goes alongside the song**, which stays kept. When the
  server ends an item cast of over 100 ms (the program's item flag), the
  song ends too, with nothing sent; an instant item's end leaves it (read
  R49-21e).

- **Each song stop says the program's line** (0x40f06d, read R49-21):
  `WorldEvent::SongStopped` carries the song's spell and `string_id`
  0x318f. That covers the player's stop (`StopSong`, a Bard's
  `StopCasting`, a gem click while singing, a forget's answer) and the
  server's cast update ending the song for the song's own spell, the
  current one, or a memorized spell (taken as a song). A stop the
  program's non-song path makes says nothing: another class's song stopped
  here, or a Bard's other spell, such as a timed item cast, ending the
  song (read R49-21e). Of these the host hears only `Singing` none. The read has the program print it
  unless its Bard songs filter holds 1 (mine only, in read R49-2's values);
  the host decides by its own filter. The
  program's mode-1 stops (its own interrupt roll on a hit, and two
  others) print 0x318e; the session makes none of those, so it never
  sends that line.

The Mac-era server types keep the earlier rules: the song holds what any
cast holds, a gem click casts, the book's answers leave the song alone, and
no `SongStopped` is sent.
The Mac-era program is unread. First use on P99 is attended.

`Capability::Melody` says where the server type takes a Bard's `/melody`
(read R49-33 c): the Titanium server types, P99 (by the owner's word) and
stock `EQEmu`. The Mac-era program is unread, so TAKP and Quarm don't. The host keeps the melody
list and runs each step on the player's own gem refresh as `StopSong` then
the next gem's `CastSpell`; the session adds no command for it. The stop
ends the song, so the press that follows casts.

## Server filters

`ClientConfig::server_filters` is the set a character starts with; the
default is the packet each generation's client is known to send.
`ClientConfig::validate` refuses a mode the generation's packet cannot say
(`ServerFilters::check`), as do both commands below, so a kept set always
encodes at zone-in. `FilterCategory::modes(dialect)` is the one statement of
which kinds and modes a generation takes: empty for a kind its packet lacks
(left out, whatever its mode) and for RoF2. `ServerFilters::fitted(dialect)`
returns a set reset to each kind's default wherever the generation cannot
say its mode.

At the character list, `GameCommand::ChooseServerFilters { selection_id,
filters }` keeps a set for the character entered next; send it before
`SelectCharacter`. The character's first zone-in sends it on every server
type. A return to the character list restores the config's set, so the host
sends `ChooseServerFilters` again before each entry. In a zone,
`GameCommand::SetServerFilters { session_id, filters }` keeps a change for
every later zone-in of the character. It goes to the zone at once where
the session offers `Capability::ServerFiltersAtOnce`: every Titanium server
type, since the Titanium client sends the packet again on every change in
its Options window (`filters::sent_on_every_change`). The `EQMac` client's
side is unread, so there the next zone-in carries a change. Every server
type offers `Capability::ServerFilters`.
Each command answers `WorldEvent::ServerFiltersKept { session_id, sent }`
(`session_id` is `None` at the character list) or
`WorldEvent::ServerFiltersRefused`, after which the prior set stands.

The kinds are named for what the servers do with each word. On Titanium
each kind is one row of the client's filter table (eqgame.exe 0x66cd78), in
the table's order, and each mode is the index of that row's choice in the
client's own order: `FilterCategory::modes(Titanium)` lists the choices in
that order, so a host can store and show the index. `ServerFilters::default()`
is the table's defaults: every kind shown but bad words, whose filter
starts on. Word 0, an ini-only switch defaulting to 0, stays 0. The
`EQMac` client's options are unread, so its kinds keep the values the
servers read.

## Petitions, bug reports and feedback

`GameCommand::SendPetition { session_id, text, created }`,
`GameCommand::ReportBug { session_id, report, created }` and
`GameCommand::SendFeedback { session_id, text, place, created }` send a report to
the server's staff (`eq_network_game::reports`), under
`Capability::Petitioning`, `Capability::ReportingBugs` and
`Capability::SendingFeedback`. A person reads each one. A host may send one
the player types or one from a script, since a script may do whatever the
player can; the pace below holds either way.
Stock `EQEmu` lists petitions and bug reports, and so do Quarm and TAKP from
their own handlers (6.604 E6, source-derived, not yet checked live). Stock
`EQEmu` also lists feedback: it has no handler and drops it, but Titanium's
program sends it all the same (read R49-23). The Mac servers list no
feedback, and P99 lists none of the three, until their own live checks
(recording checklist 6.511). On Titanium, feedback carries the sender's
name, the place and rounded heading the host gives, and the text, in the
program's 1148 bytes.

How Titanium's program takes `/petition` follows the world's petition-window
byte (`WorldRules::petition_window`): with the window on it shows the window
and ignores the words, and otherwise it sends them.
`reports::petition_form(dialect, rules)` says which, per generation; `EQMac`
and RoF2 take the text form until their programs are read.

On `EQMac` a petition is the text and a
NUL; a bug report is the servers' 1131-byte short record, carrying only the
reporter's name and the player's text, and the servers fill in the place and
target themselves. Quarm runs with petitions off, and answers with a line.

A `BugReport` carries its category (the servers' names), the player's text,
and, when the host knows them, the player's place and heading and the
target's spawn and name. The session names the reporter. The fields the
program fills from what this library lacks (the 128 bytes at 160, time
played, the word at 464, and three of the window's four check boxes) go
zero. The system information, where the program names the machine's
processor, video card and driver, always stays empty. Text is trimmed,
must not be empty or hold a NUL, and must fit what the program sends (a
petition 1023 bytes, a bug report 1024 on Titanium and 1023 on `EQMac`,
feedback 1023);
longer text is refused, never cut. RoF2 refuses every report.

The session sends one report of a kind and refuses another of that kind for
10 seconds (inferred). A sent report answers
`WorldEvent::ReportSent { session_id, report }`; a refused one
`WorldEvent::ReportRefused { session_id, report, reason }`, whose reason
never repeats the text. The servers' own answers arrive as chat: stock
`EQEmu` says nothing to the player when a petition or bug report is taken,
so a host may show its own line on `ReportSent`, and the server's refusals
(a petition already queued, bug reporting off) follow as ordinary lines.

## Own-player update gates

`UpdateGate::Flagged` applies the generation's `FlagRule` and packet packing.
The provisional `UpdateGate::Changed` has been removed. Titanium uses
`TITANIUM_FLAGS`, and the Mac-era generation uses `EQMAC_FLAGS`; neither
production rule changes in this cleanup. A gate holds an accepted sample
until it is due and more than its spacing has passed. A later host sample
replaces the held one without advancing the outgoing counter; that counter
is sampled only when the update goes out. Exhaustive host matches on
`UpdateGate` need a fallback because the enum is non-exhaustive.

## Titanium carrier support

`movement::Carriers` holds only received Titanium carrier reports for the races
read at `eqgame.exe` 0x47a741. The session publishes `WorldEvent::Carriers` after
the ordinary spawn event, preserving its receipt clock for a delayed frontend.
`CarrierSample` names the spawn, report serial and incarnation; duplicate spawn
IDs follow the ordinary table's last-record rule. Replacement, death, despawn
and a new admission retire the old identity.

`CarrierSample`, `CarrierReport` and `CarrierUpdate` are non-exhaustive. Hosts
obtain the sample from `CarrierReport::sample` and include a fallback when
matching carrier updates; report identities are not assembled from spawn IDs.

`Support::Spawn(sample)` names the report actually used by a frontend's floor
sweep. The session resolves that report itself, calls `allow_carrier`, and
checks both accepted world points in their own carrier frames. Only the rider's
relative movement spends the ordinary budget. First attachment gets no carrier
allowance; walking off names the old support for that final step and the next
unsupported sample closes it. A rigid boat turn changes absolute world heading
without creating player yaw or raising the heading-input flag.

Inferred bounds and behavior: the actual model's authored collision is a
frontend choice (6.467); the existing spawn carry step evaluated in fixed 50 ms
calls, with a 12 s hold, supplies the shared between-report pose (6.468); rider
reach is 600 units and superseded reports remain usable for at most 2 s after
their successor, within a 64-report history (6.469). A still rider's existing
1 s tick sends ordinary world coordinates with zero own velocity, yaw and
animation (6.470). No piloting opcode or route is invented. Pitch and continuing
boat yaw are absent from the decoded position API. Boat zoning is described
under [boat riding](#boat-riding); source and synthetic coverage do not
establish a witnessed ride.

The worker does not call stand-to-move solely for passive fall, lift or carrier
displacement. Titanium's movement, vehicle and physics paths do not call its
posture setter at `eqgame.exe` 0x483556 (read 6.475, including movement step
0x4bfe0f and vehicle placement 0x4aba14). Voluntary movement keeps the existing
stand-to-move policy; this read establishes no Mac-era rule.

Positive own damage has a separate stand path for sitting or kneeling players
(0x4a56e3, call at 0x4a6d6c); lying is left alone. Both received damage and a
successfully submitted local fall, drowning or lava report use that posture
policy and the stay-seated option. The native local reporter also calls that
path (0x41c905, read R31 / 6.475). Refused, unencodable and unsent reports do not
stand the player; zero damage, including a fall fully absorbed by Safe Fall,
does not stand them either. A later server damage event does not send a second
stand while they already stand. Local reporting creates no received damage
event, hit-point change or acknowledgement. This Titanium read establishes no
Mac-era rule. The native early return for spawn+0x244 > 1 remains unidentified,
and mounted state is not supplied to the session; these remain damage-parity
gaps.

## Chat server channels

Where the server type opens one (`EQEmu` Titanium only), the world's
Titanium chat-channel offer opens one chat server link for the character,
kept across zones and ended by the character list, a camp, a stop or the
session's end; a later offer replaces it, and nothing retries.
`Capability::ChatChannels` is part of a zone's `Entered` capabilities only
when its world opened a link; it is never added or removed mid-zone.
Whether the link is up is told by `WorldEvent::ChannelLink`: `Up` once the
chat server takes the login, `Lost { reason }` once when it ends (the
reason names no address; a login unanswered for 30 seconds ends it, and
so does a logged-in link the server has sent nothing on for more than 30
seconds, checked every 5, as `Lost { reason: Dropped }`; anything the
transport takes counts, its acks and keepalives among them). Each
is told once, in the zone that hears it, and not again at the next zone's
`Entered`: the host keeps the link's state across zones until `Lost` or the
character list. `GameCommand::ChatChannel { session_id, text,
created }` carries the chat server's own command text (`join a, b`,
`leave a`, `leaveall`, `list`, `announce on`, `2 hello`, and the owner and
moderator words, passed through unchecked); it is refused with
`WorldEvent::ChatChannelRefused { session_id, reason }` before the login is
taken, after the link is lost, for another admission, empty or over the
zone chat's length, or when 16 commands already wait, and is never queued
for later. `WorldEvent::Channel` carries `ChannelEvent::Message { channel,
sender, text }` (the sender without the `<world>.` the server puts first,
an inferred strip), `Notice { text }` (the server's own lines, as sent),
`Joined { channels }` (slot order: the first is `/1`), and `MemberJoined`
/ `MemberLeft { channel, name }`. All text is the server's and untrusted:
print it through the host's ordinary chat path. Hosts receive
`WorldEvent::ChatServer` as before; it carries the offered host and port,
so a host that records events should not publish it.

## Admission-scoped chat

`GameCommand::SendChatInAdmission { session_id, message, tongue }` binds queued chat to
the originating zone admission. The session refuses a mismatched admission
before notifying or invoking features, and the talk handler checks again before
encoding. This is a local queue guard; the message uses the ordinary chat wire
layout, capability and timing. It adds no server acknowledgement or retry.
`SendChat` remains available for callers intentionally using unscoped chat,
always in the common tongue.
The pure `encode` helper does not own a session and cannot perform this guard;
hosts using it directly must validate their own admission first.

## Chat pacing

One pace holds all the chat a host asks for: `SendChat` and `SendChatInAdmission`
on every zone channel and tells, and `ChatChannel` commands and messages on the chat
server link, whether typed, from a social or hotbutton, or from a script. It is the
character's, so it spans zones. At most 10 lines of any chat go in any 5 s, and at most
12 shout, auction, out-of-character, tell and channel messages (`2 hello`, `#name hello`)
and chat server commands in any 60 s; only `join`, `leave`, `leaveall`, `list`, `listall`
and `announce`, which reach no one else, are left out of that count, and an unknown word
is counted. Both windows slide, so any span of that length holds no more; stock
`EQEmu` counts those kinds 4 to 12 a minute and disconnects a client past 20. The
numbers are this library's, inferred until the official client's own chat pacing is
read (recording checklist 6.523, R49-17); the official client has no known limiter, so
this is a departure that only makes the client slower, on every server type.

A paced line is refused, never kept for later, and a refused line is not counted.
Zone chat is told by `WorldEvent::ChatRefused { session_id, reason }`, a channel
command by `ChatChannelRefused`; the reason says how many whole seconds to wait (never
0) and never repeats the line. Only what a host asked for is counted: the chat link's
own login and the local fixture's depop are not.

## Door clicks and the clicked point

`GameCommand::ClickDoor` carries `point: Option<Position>`: where the host's own
click ray hit the door's model as drawn, in server axes. A host must send a point
only from its own ray hit on the door it names, never a made-up one. On Titanium
(and RoF2, inferred, 6.489) the session admits the click when the 3-D distance from
that point to the player is no more than the player's size and 20, the program's
rule (0x49812e, 0x4906c5, 0x435b55, 0x644368; read R16b, R16e; checklist 6.530).
The size is the player's own spawn's size as the server told it, kept through size
appearances and illusions; where that is 0 or not yet heard, `EQEmu`'s size for
the race (inferred for the program's model default), and 0 for an unlisted race.
Past the limit it answers `DoorAction` with string 1462, as before.

A point that is not finite, or lies more than 200 and the player's size from the
door's origin where it stands now, is refused in our own words with no string
(our guard, not the program's). With no point (a key, a script) the reach is
today's: 3-D to the door's origin where it stands now, within 20, which is
stricter than the program. `EQMac` ignores the point and keeps its read 2-D rule.

## The item held in a door click

A Titanium door click names what the player holds on the cursor, as read R49-22b
reads the program (checklist 6.537; 0x49812e and 0x4906c5).
`Request::ClickDoor { door_id, held }` and
`doors::titanium_click(id, player_id, held: Option<HeldItem>)` write 16 bytes:
- the door at +0;
- the skill at +4, at most 254;
- the cursor item's ID at +8, or -1 for an empty cursor;
- the clicker at +12.

`HeldItem::skill` is the player's Pick Lock value when the cursor holds a
lockpick (`LOCKPICK_ITEM_TYPE`, 12, and not a container). Anything else, keys
included, carries 0. Bytes 1 to 3 are not a field: the program leaves stack
contents there. The official client's recordings show 3 in byte 1, and the
encoder keeps it.

The session takes the item and the skill from its own tracked inventory and
profile, never from the host. No item on the cursor, or an inventory not yet
loaded or made stale, sends -1, and a stack or an item with charges is named by
its ID alone. The click never moves, drops or gives the item, and the ID appears
in no event or diagnostic the host hears. The program refuses no keyed or locked
door itself, and neither does the session: the lock lines are the server's.

Not yet built:
- A trapped door type sends item -1 and the Disarm Traps value. The read names
  only part of that list of types, so a follow-up waits for the rest.
- A type 57 door's contact click. Door +0xa4, +0xb4 and +0xb8 are not key
  data, which corrects R16e.

Stock `EQEmu` reads none of these fields: it takes the key from its own copy of
the inventory and the key ring, and answers with its own lines and
`OP_MoveDoor`. `EQMac`'s program asks for a clear cursor first, so its session
still refuses a click with an item there, and its encoder refuses a `held` item.
RoF2 sends -1 until its click is read.

## Titanium lifts and door mechanisms

`doors::titanium` holds Titanium's own door rules, read from `eqgame.exe`:
`Vertical::of` (rise or sink, travel and speeds by open type, 0x4368b3), `Slide::of`
(type 15 and 78-81, 0x4366e8), `Solidity::of` (whether the floor sweep stands a
player on a door, 0x490370, and whether it stops them: every door does,
0x42cdb1),
`returns_after` and `returns_at` (the client's own return, 0x48faa5) and `field`
(door +0x66, the param's low 16 bits as an i16). The field stays as spawned; the
server message that changes it later (0x45e626) is not traced. `Solidity` is a
Titanium rule only; another generation keeps its own (`Solidity::BOTH`).

`Lifts` steps every vertical door from the received spawn and actions with the
program's rule, including the client's own return, so the session and a frontend
that follow the same updates agree on a door's height. Motion starts only on the
server's 0x700d action; a click sends 0x043b and predicts nothing. The return is
local and sends nothing, except for types 40 and 45, whose return the session sends
as `Request::ReturnDoor`: 0x043b with the door id, 0xfe and zeros, naming no player
(0x490b20, read R1c).

`MovementRequest::support` names what the frontend's floor sweep stands the
player on, as a `Support` (non-exhaustive; `Support::Door(id)` or `Support::Spawn(sample)`), set with
`MovementRequest::supported_by`. While a request names the same lift, the session
keeps an account with it: the rider may rise in all as far as the lift has risen
since they first named it, less what they have risen while naming it, so a
frontend whose lift runs behind the session's spends late what it did not spend
early and never rises past the lift (`MovementGuard::lift_since`,
`MovementGuard::allow_lift_rise`, `Lifts::carry`). The door must be a floor the
program stands players on, with the last accepted position within `RIDER_REACH`
(310 units in 3D) of where the door stood at that sample, so a lift taller than
the reach carries its rider all the way. A request naming nothing, another door, or a lift that
sank over the interval closes the account; only a request that names a support
spends any of it. The rise composes with the ordinary grant: horizontal motion
keeps its speed budget, a still rider is passive, and a lift carries (or holds
at its top) a player whose input is withdrawn only straight up. Nothing
lift-specific is sent; the risen height goes out by the generation's ordinary
gate. `Lifts::returns_at` and `Lifts::sinks` give a vertical door's own return
time, from where it stood, and whether it sank over an interval.

## Friends who, guild requests, splitting and banker change

`EQEmu` and Project 1999 offer `Capability::FriendsWho`, `GuildRoster`,
`GuildManagement`, `BankerChange` and `Splitting`. Each sends what the
Titanium program sends, and the server decides what it allows.

- `GameCommand::WhoFriends` carries the host's friends list. The answer is an
  ordinary `WorldEvent::WhoList`. A list the packet cannot carry is not sent.
- `GameCommand::SplitCoins` names platinum, gold, silver and copper. It is
  refused outside a group, for nothing, and for more of a kind than the purse
  holds, each with the official client's string ID on `CoinsRefused`. It is
  withheld without a word while a banker's change is unanswered. A sent split
  takes the coins from the purse at once, as the official client does, and
  reports `WorldEvent::Coins`. If no money update follows within the wait,
  the purse becomes uncertain (`PurseUncertain`).
- `GameCommand::SetAutoSplit` sends the appearance update and reports
  `WorldEvent::AutoSplitSet`, as the server answers nothing. The Titanium
  profile's setting arrives at admission as `WorldEvent::AutoSplit`, so a
  host can toggle it as the official client does.
- `WorldEvent::BoundIn { zone_id }` tells the zone of the player's primary
  bind point, once after each admission, from that zone-in's profile, on
  both generations (Titanium's `binds[0]` at byte 24, `EQMac`'s first bind).
  It needs no capability and sends nothing. A zone the reader can't take
  gives none, and the zone-in goes on. The server sends no change when a
  bind lands mid-session, so a host should take it as current only until
  the next zone-in (inferred, pending R49-6m).
- `GameCommand::ChangeCoins` needs a banker within the bank reach. It holds
  coin moves until `WorldEvent::CoinsChanged` arrives, which the worker
  follows with `Coins` and `CoinsElsewhere`. If no answer arrives within the
  wait, `CoinsRefused` reports it and the coins stay as they were.
- `GameCommand::DestroyCoins` destroys coins of one kind from the cursor,
  under `Capability::DestroyingCoins`, which only stock `EQEmu` offers
  (`Client::OPMoveCoin` takes slot -1 as destroyed). It must name exactly
  what the cursor holds of that kind; anything else is refused on
  `CoinsRefused` and nothing is sent. A host destroying several kinds sends
  one command per kind. No answer comes, so the worker takes the coins from
  the cursor at once and reports `CoinsElsewhere`. It waits, refused, while a
  banker's change is unanswered. The host asks the player first.
- `GameCommand::RequestGuildMotd` names the player's guild; outside one it
  is refused with the official string ID. The reply arrives as chat on
  `ChannelName::GuildMotdReply`, which the official client always shows.
  `ChannelName::GuildMotd` arrives unasked, and the official client shows
  it only when its text differs from the one shown last.
- `GameCommand::AnswerGuildInvite` answers the invitation of the inviter it
  names, once, with an `InviteAnswer`: accept, decline, or one of the
  official client's automatic answers (in another guild, another dialog
  open, the rank already held). The host decides which, as the official
  client does from its dialog and its state. `GameCommand::GuildInvite`
  says whether it offers an officer's rank. `SetGuildMotd` with 512 bytes or
  more sends an empty message, as the official client does. Guild removal, leader and demote requests name a member of the
  list the server sent, spelled as it spells them. The server decides who
  may do what and says so in its own lines.

Received listing changes for the current player's spawn also update the
worker's admitted guild membership. Joining or leaving during an admission
therefore changes the guild used by ordinary requests without requiring a
relog. Another spawn's listing cannot change the player's membership; guild
names and permission outcomes still require their own received evidence.

## P99 self-bandage stock

A self-targeted P99 Bind Wound request can hold inventory while awaiting
stock evidence. A terminal reply or local expiry releases that hold.
`InventoryUpdate::BandageStockUncertain` lists possible spent slots still
awaiting a covering update. `Inventory::bandage_stock_uncertain` includes
a parent bag when any of its contents are uncertain. The older item count
stays visible and bounds carried weight from above. An expiry reports
`BindWoundUpdate::Unconfirmed { stock_uncertain }`. A received stock change
can settle the inventory side without resolving the bandaging outcome.
An expired P99 result retains ownership of its untagged reply until it
arrives or the admission ends. The held result does not extend the
inventory action hold.

## Station custody

`InventoryUpdate::WorldDetached` retires the displaced station's local
inventory. A matching `ObjectUpdate::Closed` retires an open view.
Carried slots remain provisional until covered by received inventory.
A pack receipt for the same serialized whole item can resolve a single
direct cursor-to-station route, separately from the station clear.

## P99 local pads

A P99 host can provide `InstalledZoneBoundaries` in `ClientConfig`.
The worker corroborates the entered boundary with the current approach,
admission and indexed zone-point record where required. A same-zone pad
submits an ordinary position report at entry and stages a local landing.
`LocalPadCrossed` describes that placement. Continue to use received
relocation events for server-directed placement.

## Titanium dialogs and player inspection

Project 1999 and EQEmu offer `Capability::ServerDialogs` and
`Capability::PlayerInspection`. Present a `PopupOpened` offer using its
admission and serial. Pass both to `GameCommand::RespondToPopup`.
`PopupClosed` identifies which presentation to dismiss. The worker sends a
reply only for a supported choice with a response ID. Closing and expiration
are local, and a successful send does not establish a quest outcome.

`GameCommand::InspectPlayer` uses the current available `TargetState`.
`WorldEvent::PlayerInspection` carries the equipment names, icons and note
received for the matching nearby player. These fields carry no inventory
custody. `GameCommand::SetInspectNote` supplies the admission's known note
for automatic replies. A known empty string can be sent. An absent note
leaves the automatic reply withheld.

## Titanium observer sight

`WorldEvent::Visibility` retains the dialect and raw invisibility value.
Titanium spawn appearance carries optional raw invisibility and GM fields.
Hosts can install a `vision::SightCatalog` with `Client::with_sight_catalog`
to evaluate observed own buffs using their effective spell definitions.
`WorldEvent::ObserverSight` reports a change within one admission. Reset
this projection on entry or loss of the world. A known ordinary invisible
player or NPC with a known non-GM flag can be visible to an observer with
See Invisible. Other values and incomplete metadata stay conservative.

## Local EQEmu fixture cleanup

`GameCommand::DepopLocalNpc` carries the current admission, creation time
and reconciled `TargetState`. The chat owner accepts it only when the session
uses `ServerProtocol::EqEmu` with `ClientConfig::local_only`. At the send it
requires the same available target revision and a visible NPC. The normal
admission and freshness guards also apply. A local refusal is diagnostic.

The wire command is the fixed `#depop` chat line. The server acts on its
current target and sends any resulting world changes independently. The
target guard covers state the session has observed at the send. A caller
must verify ownership and observe the exact fixture's removal.

## Manual Titanium ranged attacks

An admission offering `Capability::RangedCombat` can take
`GameCommand::RangedAttack`. Pass the current available `TargetState` from
`TargetChanged`, a `RangedLoadout::current` snapshot from the same admission's
inventory, and the admission ID and input time. Reconciled target identity
includes its revision. Any intervening inventory revision invalidates the
queued loadout, including a same-ID replacement or ammo update.

The zone checks readiness, posture, casting/inventory holds, equipped items and
range before it sends. Stock EQEmu applies its class/level skill cap, so a raw
rank of zero or an unavailable rank does not settle eligibility locally. P99
retains a conservative positive-rank prerequisite in the recognized 1–252
range while its first-use behavior is unverified. Local recovery compares
time since the last submission with the current weapon delay. Bows use the
first confirmed eligible quiver in general inventory when its reduction
leaves more than one second. Haste and slow remain outside the local
estimate. The server applies its own timers,
targeting rules and combat resolution. The `CombatRequest` disposition reports
only local submission or refusal. Existing received damage, inventory and chat events
carry the outcomes. The server chooses whether and from which stack to consume
a projectile. Hosts should issue a fresh intent for each explicit input.

## Pet orders

`WorldEvent::OwnPet` reports the current pet's `PetIdentity` (spawn ID and
an admission-scoped revision) whenever the session resolves exactly one
living spawn the server marks as owned by the player. The revision changes
on every owned re-spawn, death, charm or ownership change, including a
duplicate spawn report of the same ID. Pass the latest identity in
`ClientCommand::Pet { pet }` and the current `TargetState` for orders that
name the target; the session refuses an order whose identity or target no
longer matches ("Your pet has changed") before anything is sent, and
`SelectTarget::expected_pet` pins a pet-specific selection the same way.
That guard is the session's own, not the program's, and it only ever refuses.
An order with no pet (`pet: None` while the session knows of none) goes out:
the program sends every `/pet` order without asking whether there is a pet
(eqgame.exe 0x49975b, read R49-22a), and EQEmu's, Quarm's (fc75fcf) and
TAKP's (44afcbf) handlers return in silence when there is none. The Mac
program is inferred the same (6.610).

The Titanium session ignores a `WorldEvent::Charm` whose pet spawn's race is
in `control::CHARM_IGNORED_RACES` (108, 216 and 472), as the program's
`OP_Charm` handler does (0x453d7b), and says so in a diagnostic. The handler
also ignores a pet whose byte +0x1b3 is set; that field is unread (row
22a-i). The handler prints no line and changes no target, and neither does
the session. `EQMac` and RoF2 keep applying every charm until their
programs are read.
`PetCommand::GuardMe` is Titanium command 6. The Project 1999 provider
selects the classic order set, which excludes Hold, Feign, NoCast and Focus.
The EQEmu provider selects the expanded set. Local submission leaves
execution and any further restrictions to the server.

`PetCommand::project1999_available` is deprecated in favor of
`PetCommand::in_classic_order_set`. The compatibility alias returns the
same classification. The provider chooses the order set for each admission.

## Titanium swimming bound

Titanium EQEmu and Project 1999 offer `Capability::Swimming`. Quarm and
TAKP reject `MovementMode::Swim` samples. A swim sample is refused while
flying or stunned, and takes no run boost.

On Titanium the grant carries `MotionCalibration::swim` (and
`WorldEvent::MotionState::swim`): the swimmer's forward, walking, strafe and
backing inputs in the program's speed units, and the water set of the
zone's `WT` regions (`movement::water`, from `GameDialect::water`). A
standing swimmer's forward inputs add the Swimming skill (skill 50) x 0.01,
an unknown skill adding nothing; sneaking or ducked inputs keep the land
speeds (`eqgame.exe` 0x4a1d74, read R33). A stroke is bounded on each axis
by `SwimCalibration::caps`: horizontally by `c s / (1 - h)`, rising by the
stroke and the damped jump push, sinking by the stroke and the water's
gravity over this client's 1/60 s frame, all from the step at 0x4bfe0f
(reads R33, R39 and R41). The bound has no carry term: a frontend that
integrates `movement::water::WetVelocity` clamps the velocity it enters
water with to the caps. A rooted swimmer keeps sinking within the gravity
term. After a swim sample, a land-mode or `MovementMode::Fall` sample may
rise as a body leaving the water at the rising cap would under the jump's
gravity.

Without `swim` (`EQMac`, and streams from before it) a swim sample keeps one
three-dimensional budget of `movement::swim_speed` of the unboosted grant
(recording checklist 6.390). A submitted sample establishes neither server
acceptance nor water state. The host asserts water from the installed zone.

## Local Titanium liquid reports

`Capability::ALL` grows from 50 to 52 with `Drowning` and `LavaHeat`;
`GameCommand` gains `LiquidDamage { session_id, hazard, amount, observation }`
and `WorldEvent` gains `LiquidHazardRevision { session_id, revision }`.
Every Titanium policy, `EQEmu` local or not and P99, offers the two
capabilities (see "Titanium liquid reports and their damage record"); Quarm
and TAKP keep falls only and refuse `LiquidDamage` before anything is sent. `EnvironmentalDamage` keeps the completed-fall path and
refuses every other hazard.

The session raises `LiquidHazardRevision` at admission and whenever an input
of the air or heat calculation changes: the player's own position, hit
points, appearance, posture, buffs and spell effects, death, inventory and
level updates, and zone transfer state. A `LiquidDamage` report is accepted
only when its `session_id` is the current admission, its `revision` is the
latest one raised, its `observation.position` equals the session body's
current X, Y and Z bit for bit, its `observation.sampled_at` is within 250
milliseconds, the player is admitted and not dead or transferring, its
`amount` is positive, and at least one second has passed since the last
accepted report of the same hazard. Anything else is refused with a
diagnostic and no wire change; the next revision is the host's signal to
sample again. An accepted report is sent as the Titanium environmental
record with damage type 251 (drowning) or 250 (lava), 244 for zone 200's
lava, laid out as the Titanium program builds it (read R49-3 c). `PlayerState.titanium_environment` carries the profile's
zone, base fire resistance and whether any alternate advancement rank is
present. Titanium `eqgame.exe` copies the profile dword at byte 12876 at
0x4b9f3c and uses it at 0x418756. The cited EQEmu Titanium encoder writes
a constant 25 there through its `unknown12864` block copy. The decoder
retains the received dword. The server owns effective resistance, health
and death. [Profile layout][titanium-profile] [EQEmu encoder][titanium-encoder]

[titanium-profile]: https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/common/patches/titanium_structs.h#L883
[titanium-encoder]: https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/common/patches/titanium.cpp#L1600-L1606

## Local Titanium environmental AA observations

The local-endpoint EQEmu policy publishes read-only
`WorldEvent::AlternateAdvancement { session_id, updates }` ordered facts. The complete
profile AA selection array and `OP_RespondAA` replace selected records;
`OP_SendAATable` supplies individual definitions with required level, class
mask and signed effects. Exact selected definition IDs determine membership.
The raw selection value is retained and never used as an effect multiplier.
An unowned definition grants no rank, and a partial table does not establish
missing selected definitions. Pre-admission facts are staged and then stamped
with the receiving admission in one batch, after `Entered` and before `Ready`.
Core applies every fact in order, including withdrawals; the batch is not a
complete-table assertion. One `LiquidHazardRevision` follows the whole batch.
Post-admission facts publish immediately as singleton batches. This reduces
admission event amplification without enlarging the viewer queue or proving
that all possible queue saturation is prevented.

Since checklist 6.547 the facts also carry the rest of the Titanium AA
state, each field raw under its wire name: every `OP_SendAATable` header
field (the string ids, `cost`, `cost2`, the prerequisite pair, `type`,
`spellid`, the timer id `spell_type`, `spell_refresh`, `max_level`,
`last_id`, `next_id`); the profile's `aapoints_spent`, `aapoints` and `expAA`
as `Update::Points`; the 12-byte `OP_AAExpUpdate` 0x5f58 as
`Update::Progress` (experience as the server scales it to 0..330, unspent
points and the split percentage); and the server's 12-byte `OP_AAAction`
0x0681 as `Update::Timer` {`begin`, `ability`, `end`}, where `ability` is a
definition's `spell_type` and `end` is the server's clock when it sent the
timer, not a deadline. The session does no timer arithmetic and keeps no
timers; it publishes each as received. Points, experience and timers change
no effect, so they bring no `LiquidHazardRevision`.

Since checklist 6.548 a local TAKP session publishes `EQMac`'s AA facts the
same way, from TAKP's layouts, keyed by the Mac AA id the client knows:
`OP_RespondAA` 0x1542 (227 bytes: a first byte TAKP fills with the points
spent, inferred from its use, then 226 rank bytes, id `r + 1` at `r + 1`;
id 211's constant 3, TAKP's Fleet of Foot workaround, is passed through) as
`Update::MacRanks`; the profile's AA array (nonzero ids only, the value as
sent, which TAKP makes the rank times 16) as `Update::MacSelections` and its
`aapoints`, `expAA` and `perAA` as `Update::MacPoints`; `OP_AAExpUpdate`
0x2342 (8 bytes) as `Update::MacProgress`; and the server's 12-byte
`OP_AAAction` 0x1442, keyed by the Mac AA id, as `Update::MacTimer`. `EQMac`
servers send no definitions, so no Mac fact enters the environmental sums.
Quarm shares the decoding, assumed as TAKP, but isn't offered it until its
own attended row; remote TAKP waits for the local row. An unreadable Mac
packet withdraws as its Titanium kind does, and a timer withdraws nothing.

Since checklist 6.556 the session can send Titanium's AA requests. Since
6.581 `Capability::AlternateAdvancement` is listed under the same gate as
the facts: only on a local stock `EQEmu` or local TAKP session, until their
live rows (6.556, 6.562) pass. Public `EQEmu`, P99, Quarm and RoF2 never
list it, local or not, and there the three commands are refused as not
offered. On a local TAKP session only the split goes (below): Train and
Activate stay refused until the client's AA data is read (R49-41b).
`ClientCommand::TrainAlternateAdvancement { rank_id }` buys a
rank, `ActivateAlternateAdvancement { rank_id, target }` uses an owned
rank's ability, and `SetAlternateAdvancementSplit { percentage }` sets the
share of experience that goes to AAs, each as one 16-byte `OP_AAAction`
0x0681 {action, ability, target, exp}: train is action 3 with the rank id,
activate action 0 with the owned rank id and the target's spawn or 0, a
split of 1 to 100 action 1, and 0 action 2 (off). Which ids and actions the
program itself sends is read R49-39, inferred until then.

The session checks each request against a bounded, never published view of
what this admission's facts said: the definitions offered (at most 4096;
one past that is not kept, so a train for it is refused), the latest
selections and the latest unspent points (`Update::Points` or
`Update::Progress`, whichever came last). It follows every withdrawal and a
new admission starts it afresh. Each check is one EQEmu makes too, and what
is not known refuses. A train goes only for an offered rank the player does
not own, whose rank before it the table shows owned, which costs no more
than the unspent points, whose level is no higher than the player's current
level and whose class mask has the player's class. EQEmu's table offers each
ability's owned rank and the next, or its first, so a rank another offered
rank follows is owned; otherwise the selections name it, by exact id in
`OP_RespondAA` and by an ability's first rank in the profile. So a maxed
ability above rank one, which has no following rank in the table and which
the profile names only by its first rank, reads as not owned until an
`OP_RespondAA` names its exact rank: until then it can't be activated. The
local live row checks that the zone entry brings an `OP_RespondAA` (our
client sends `OP_UpdateAA`); if it brings none, the offer waits on a fix.
Prerequisite abilities are left to the server. One train is in flight at a time, until
EQEmu's answer (`OP_RespondAA`, then `OP_AAExpUpdate`) or 5 seconds. An
activation goes for an owned rank with an ability (`spellid` not -1), at
most one a second; a split change at most one a second, and above 100 is
refused where EQEmu would quietly set 0. The waits and paces are ours,
inferred. A refusal is `WorldEvent::AlternateAdvancementRefused
{ session_id, reason, refusal }`: `refusal` is an
`alternate_advancement::Refusal` for the host to word, and None where the
session refused the command as it refuses any (an earlier admission, not
offered); `reason` says the same in this library's words. The server's own
answers (the gain line, the reuse wait, the split's on and off lines) reach
the host as its other lines do.

Since checklist 6.562 the same three commands also encode for the `EQMac`
generation, still listed by no server type: TAKP's `OP_AAAction` 0x1442 is
256 bytes of text, NUL-padded and built from fixed words and numbers only:
`buy N`, `activate N` (N the Mac AA id, 1 to 226, as `Update::MacRanks`
names it; TAKP picks the rank to buy itself), `on N` for a split of 1 to
100 and `off` for 0. The exact bytes are read R49-41a, inferred until then.
On `EQMac` the session sends only the split for now. TAKP's buy checks
neither level nor class, which the Mac client keeps itself (read R49-41b),
so a train is refused as `Refusal::Unknown` until the session has the
client's AA data; an activation is refused the same way, since which ids
are passive is that data too and TAKP answers a passive one with a line
while the player sits, feigns or rides a boat. An activation's `target`
must be None on `EQMac`, refused as `Refusal::OutOfRange` otherwise: TAKP's
text names no target and uses the server's stored one, which the session's
own selection does not prove. Turning the split on needs a known level of
51 or more (`Refusal::LevelTooLow`, or `Refusal::Unknown` when the level
isn't known): TAKP takes `on N` at any level and turns it off again below
51 at the next experience change, so ours is 51, inferred until R49-41.
Turning it off goes at any level. The capability waits for a local TAKP
live row; Quarm, assumed as TAKP, waits for its own attended row, and P99
is never offered it.

Malformed packets withdraw their affected observation. An unreadable
profile withdraws its points with its selections (`Update::Points(None)`),
unreadable experience withdraws the last (`Update::Progress(None)`), and an
unreadable timer withdraws nothing: no timer id in it can be trusted. Each
unreadable line carries no payload bytes; the profile, experience and
timer lines name the length. An unidentified
malformed definition withdraws cached definitions; `OP_ClearAA` withdraws the
whole AA view until replacement facts arrive. These withdrawals never supply
an empty selection. The local effect-list bound is 256 records per definition;
staging and definition retention are separately bounded at 4096. Exceeding a
bound withholds observations. This feature sends no packets, offers no AA
commands, and is absent from P99, Quarm, RoF2, remote EQEmu and remote TAKP
feature sets.

[EQEmu Titanium AA layouts](https://github.com/EQEmu/EQEmu/blob/f5ec714c8835b2afd2be1812bb9033ff70a17a64/common/patches/titanium_structs.h#L3229-L3261)
[EQEmu AA encoders](https://github.com/EQEmu/EQEmu/blob/f5ec714c8835b2afd2be1812bb9033ff70a17a64/common/patches/titanium.cpp#L1749-L1816)

## Retained base held models

`ItemDetails.base_held_model` keeps the checked optional base held model from
a Titanium item definition. A later item packet for the same slot with an
empty model field replaces it with `None`; the received wear appearance stays
separate and takes precedence in hosts.

## Casting, spellbook edits and Bard song stop

The worker emits `WorldEvent::CastPending` with the submitted spell and
admission. When a received spell update answers that request, the worker
clears the pending notice before forwarding the update on the same ordered
event stream. A cast may begin under a different spell ID and end under the
requested ID. After five seconds without a matching update, the worker
instead emits `WorldEvent::CastRequestUnconfirmed` for the unanswered
request and allows a fresh manual attempt. That event reports only the end
of the local wait. It precedes any later submission in that admission,
including a manual retry of the same spell. The worker retains the last
timed-out spell and gem/item source until a compatible own start, an
answering result naming the spell, a fresh submission, death, or the end of
the admission. An unrelated own start can hold casting independently.
An own interruption carries no spell ID. It releases the active hold
and leaves any separate timed-out request intact. Timed-out provenance has
no wall-clock expiration within those lifecycle bounds.

A late start matches the request by spell ID. Stock TAKP also accepts
the documented Luclin port rewrite from 2062 or 2706–2709 to 2935. The
rewrite is unverified on Project 1999 and Quarm. A late 2935 there begins
an independent hold that can end on a result for 2935, own interruption,
death, or departure. The late-port recording case captures the requested
spell, own start, and spell-bearing result or interruption on each server.

The worker emits `WorldEvent::CastLateBegin` with the request spell before
forwarding the compatible own `SpellUpdate::Began`. Starts carry no
request ID, so overlapping retries with the same spell or alias remain
ambiguous. An already begun cast still awaits a matching result. Own
death clears cast state through the death event. Exhaustive `WorldEvent`
matches must handle `CastLateBegin` and `CastRequestUnconfirmed`.
[EQMacEmu start](https://github.com/EQMacEmu/Server/blob/d8d589a0ce486ee47ec990d3c3bbbf604b2572a1/zone/spells.cpp#L501-L511),
[EQMacEmu port spell IDs](https://github.com/EQMacEmu/Server/blob/d8d589a0ce486ee47ec990d3c3bbbf604b2572a1/common/spdat.cpp#L1305-L1311)

Stock EQEmu and EQMacEmu use spell-bearing `SpellUpdate::Mana` to
re-enable ordinary casts, including item clicks. That update settles a
matching cast when `keep_casting` is false. Standalone item-slot
`BarRefresh` remains a worker compatibility path. The paired host's
refresh handling covers memorized gems, so standalone item refresh is
outside the delayed-item completion contract.
[EQEmu spell bar](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/spells.cpp#L5377-L5390),
[EQMacEmu spell bar](https://github.com/EQMacEmu/Server/blob/d8d589a0ce486ee47ec990d3c3bbbf604b2572a1/zone/spells.cpp#L3945-L3957)

An unanswered book edit emits terminal `BookActionStatus::Unconfirmed` once
after 30 seconds and releases the spellbook hold. A fresh edit can proceed
if its expected reply is distinguishable from every timed-out edit in the
admission. The worker refuses a wire-identical retry until a new admission.
That includes a forget of the same gem even when the spell changed, because
an empty sentinel identifies only the gem. An in-zone slot update or book
snapshot changes the current state without retiring that reply ambiguity.
Late updates still go through the ordinary state appliers and produce no
result for the timed-out edit. A mode-2 update clears its valid gem slot
independently of spell identity, and repeating it against an empty gem keeps
that gem empty. Only the matching gem and spell or an empty sentinel can
confirm an active forget. A repeated application-level book swap can apply
the same delta again because its wire record carries only the two slots.
An unconfirmed scribe holds the inventory until a received whole-slot
removal or replacement accounts for the cursor scroll, its edit reaches the
30-second `Unconfirmed`, or the admission ends. A unit decrement or late
book insertion alone cannot release custody. The timeout releases only the
custody of the scribe that timed out (inferred, recording checklist 6.462).
After it, the exact late book insertion holds the cursor again for the
removal that follows; that removal becomes `InventoryUpdate::Consume` while
the model still holds the scroll on the cursor at the scribe's revision, and
`InventoryUpdate::Invalidated` otherwise, marking the inventory stale until
the next zone-in's inventory (nothing refreshes it mid-zone on either
generation). The late hold passes updates that leave the cursor alone (a
`Set`, `Remove`, `Consume`, `Used` or `Deduct` of another slot), since the
server sends the confirmation and the removal from one handler and answers
later requests after both; anything else naming the cursor, a `Snapshot`,
an `Invalidated`, or 5 s since the confirmation ends it, and the update then
passes unchanged (recording checklist 6.526). Known
limit of an unanswered edit: its reply identity keeps refusing a
wire-identical retry until the next admission.

The server's level refusal (Titanium formatted string 12048, decoded as
`Message::BookLevelRefused { arguments }`) is a definite answer to an
outstanding scribe or memorization: the worker reports
`BookActionStatus::Rejected` naming that line and its arguments, releases
the book and the scribe's cursor custody, and retires nothing, so the same
request can be sent again. It is the only answer that frees such a retry.
With no scribe or memorization outstanding, or a scribe already confirmed,
the line changes nothing. The `EqMac` wire's refusal is unread and falls to
the timeout.

Titanium EQEmu offers `Capability::StoppingSongs` and handles an explicit
`ClientCommand::StopSong { session_id, created }` for the current Bard.
The wire request is empty `OP_ManaChange` (`0x4839`), as implemented by the
[server handler](https://github.com/EQEmu/EQEmu/blob/master/zone/client_packet.cpp)
and [Titanium opcode table](https://github.com/EQEmu/EQEmu/blob/master/utils/patches/patch_Titanium.conf).
After submitting a stop for a tracked gem cast, the worker allows one
second for matching cast news. An own interrupt or matching mana or gem
refresh follows the ordinary cast path. With no such update, the worker
releases its local hold and emits `WorldEvent::SongStopUnconfirmed` with
the admission and released spell. When no cast is held, submission emits
the same event without a spell. This event reports local state only. It
leaves buffs and cooldowns to the existing server updates, and a retry
requires a fresh user command. An active item cast or a cast of unknown
source blocks song stop. A duplicate while the worker awaits the cast update
produces a diagnostic without another send. The P99 policy keeps song stop
gated.

An own start for a different spell during the stop wait supersedes the
stopped hold. The worker emits `WorldEvent::CastSuperseded` with the
old spell before forwarding the new start. Consumers should discard the
identified hold and its request alias while preserving a different pending
spell. A late start for an abandoned
pending spell is withheld during a retry of a different spell.
Titanium has no cast request ID for distinguishing same-spell starts
after a same-spell retry.

## P99 readable items

`GameCommand::ReadItem { session_id, revision, slot, created }` now binds the
intent to the worker inventory revision and enqueue time. `Book::from_item`
preserves numeric kind values rather than coercing nonzero flags into a book.
Actual Titanium item decoding retains unsupported read metadata for refusal or
rejects malformed numeric fields. Titanium text decoding requires a supported
note/book kind and terminating NUL. Submission does not supply `BookText`;
supported server text opens the result. See `docs/p99-reading.md`.

## Zone handshake and player admission

`Client::run_with_commands` requires a usable player snapshot after each zone
handshake, including transfers. The worker delivers `WorldEvent::Entered` and
staged world updates before `ConnectionStage::Ready`. If the wire handshake
completes but the player snapshot cannot be decoded, the worker closes the zone
without taking player commands, reports `ConnectionState::Disconnected`, and
returns `PlayerAdmissionError` without an automatic retry.

The receiver belongs to the host. Pending commands stay queued at the failed
admission and get no per-command refusal from that attempt. Use a fresh command
queue for a separate run. Interactive world or character selection polls may
discard leftover gameplay commands before another admission. If a pending
gameplay command reaches a ready zone, the worker rejects old admission IDs
where the command carries one. Legacy `SendChat` carries neither an admission
ID nor a creation time and can be sent from the later zone if it reaches that
zone's queue drain.

`Client::run` can still deliver communications from that zone. It sends the
additive `ConnectionStage::Recording` milestone with `Connected` transport
health. This outcome has no `Entered`, `Ready`, or live gameplay feature
processing. Servers offering movement retain the stationary heartbeat only
with the nonzero own-spawn ID and position read during the handshake (Titanium's
validated own spawn and saved profile position, or EQMac's assigned ID and own
spawn position). Transfer traffic or a death, despawn, or position update for
an already known own spawn withdraws the position repeat for that zone, even
if it arrived before handshake completion. The first zone offer in recording
mode yields a diagnostic, including when the offer cannot be decoded. The
listener continues on its current connection while the transport remains
active. The progress enum is non-exhaustive, and its new serialized value is
`recording`. Errors in the handshake itself continue to follow the existing
session error policy.

## Titanium event saves

On stock `EQEmu` Titanium, four existing actions now put one more packet on
the wire, after their own: `ClientCommand::ToggleRoleplay` and
`ClientCommand::ToggleAnonymous` (after the appearance update), the player's
`ClientCommand::Shop` close of an open merchant's window (after the close),
and the server's end of an open loot (`LootUpdate::Closed`). Each sends
`OP_Save` (0x736b, 192 bytes): the zone save's body with the event's reason
at +4 (`zoning::save_reason`: 0x0a, 0x0b, 0x14, 0x15), carrying the own
spawn's place at that moment, the heading cut to a whole number. Only stock
`EQEmu` sends it. P99 is built and held off until an official-client capture
there shows it; Quarm, TAKP and RoF2 send none, and `Request::EventSave` is
refused on `EQMac`. It is withheld, with a diagnostic and nothing sent, when
the player has no own spawn or place yet, is dead, or has a merchant's window
or a trade open. A refused or paced toggle, and a loot close with no loot
open, send no save. Stock `EQEmu` saves the character on any `OP_Save` and
reads nothing in the body. Reads R49-22g and R49-22h, checklist 6.615.

## Modern session termination

`eq_network_transport::modern::SessionTermination` is an additive typed error
inside the existing `anyhow::Result` APIs. `PeerDisconnected` retains an opaque
optional native reason; `OutOfSession` reports the established stateless control.
Neither reports acceptance or failure of an application request.

Once observed, the first termination immediately prevents all sends and network
I/O, and discards incomplete fragment/reorder and outbound work. Fully completed,
ordered applications before the terminal control drain exactly once in order;
later combined suffixes are ignored. After that prefix, repeated receives retain
the same terminal error. Explicit local close still refuses staged applications.
`Client::run` reports Disconnected once for an attempt ending in that error. With
reconnect disabled it returns the typed error; with reconnect enabled it retains
the configured delay before starting a fresh login. Unknown terminal reasons do
not establish permanent refusal. The existing permanent credential-refusal rule,
other retry timings and legacy EQMac contracts are unchanged.
Hosts can downcast the error or show its credential-free Display text.
`Transport::termination()` is an additive read-only accessor, defaulting to None
for other transports and forwarded by boxed transports. The zone stops timers,
queued actions and automatic replies while draining. A matching latched typed
send refusal from an observer is noted as not submitted and deferred until the
prefix drains; unrelated failures still propagate. Existing receive signatures
are unchanged. See
[evidence and packet limits](docs/session-termination.md).

## Reconnect count and queued commands

With `RunOptions::reconnect` on (the default), `Client::run` and
`run_with_commands` try at most `RunOptions::reconnect_attempts` fresh logins in
a row, 5 by default (our bound, 6.534: no read or server source gives one),
then return the last failure; the count starts again once a run has stayed in
its zone for at least the reconnect delay, so a server that admits and drops at
once is still bounded. Each `ClientEvent::Reconnecting` says which `attempt` it
is of how many (`attempts`); the last before the run ends says `attempt ==
attempts`. Just before each fresh login says `ConnectionState::Connecting`,
every command still in the host's queue is dropped unread and told once as
`ClientEvent::CommandDropped { command, reason }`, with the command's kind and
our reason only, never its text, recipient or a character name: nothing asked
for in the ended session acts in the next one. A command sent after that point
is read by the new login, even one composed before the host saw
`Reconnecting`, so hosts should send the admission-scoped forms (such as
`SendChatInAdmission`). When the run ends instead (reconnect off, a confirmed
rejection or the end of the count), the queue stays the host's. The eq-client
viewer turns reconnect off; the loop runs for hosts that keep the default, such
as headless or scripted runs and this crate's tests.

## Camp completion evidence

`CampStatus::Camped` now means an actual `Message::LoggedOut` was received.
The additive `CampStatus::ClosedWithoutReply` instead names the documented local
logout-close deadline, with unchanged timing and local selection/desktop routing.
It does not confirm server logout, cleanup or a reconnect cooldown waiver.
Consumers must handle both local completion destinations and keep their evidence
distinct. Ordinary camp preparation and the explicit ExitAtOnce opt-in still
share the same logout/reply owner. A terminal error without a logout reply is
still an error, not either completion event.

## Meal command availability

`Capability::Meals` is separate from `Inventory`: it gates `Consume` and
`AutoEat`. The inventory feature offers it exactly when its existing
`Allowances::meals` is enabled. Hosts retain the local automatic-food preference
but post it only for an admission offering meals. Protocol and server-specific
allowance policies remain with the existing inventory owner.

## Applying a poison

`Capability::ApplyingPoison` gates `GameCommand::ApplyPoison { slot }`. The
inventory feature offers it with `Inventory`, on Titanium and `EQMac`
admissions; RoF2 offers no features. The session follows the Titanium
program's flow (read R49-1, recording checklist 6.493); `EQMac` takes the
same flow, inferred, as its program is not read.

- It refuses, with `WorldEvent::ApplyPoisonRefused` and nothing sent, in this
  library's words with no `string_id`: while the inventory is not current,
  while the slot has moves unconfirmed, when the slot holds no item of type
  42, when the slot is inside a bag (the program's item use passes only
  top-level slots, the cursor included), and while the player's skills are
  not yet known.
- With the Apply Poison skill at 0 it refuses with `string_id` 0x305f.
- A standing player is sat down through the usual posture request and
  `WorldEvent::Posture`. A player still not sitting (ducking, lying,
  feigning) is refused with `string_id` 0x305d.
- A command while a poison is waiting or in flight is ignored, as the
  program ignores it: a diagnostic, and no world event. It is ignored only
  after the sit and the 0x305d refusal, in the program's order, so a
  standing player's click while one is in flight still sits them.
- Otherwise the wait starts, with `WorldEvent::PoisonStarted { slot,
  string_id: 0x305e }`. The request goes out once the seated wait passes
  the delay, read each tick as the program reads it (read R49-1f): effect
  234's total in ms where that is not 0, else 10 s. A negative total never
  sends; the wait then ends only with a posture or zone change.
- The host reports that total with `GameCommand::PoisonWait { total_ms }`
  (6.493b), offered with `Capability::ApplyingPoison`, whenever it changes
  and after each admission: effect 234 from every active buff at its
  caster's level (type-4 records negated) and every AA the class may use at
  its level; worn items do not count. It is the host's own report, never a
  script's. `None` means something that could carry the effect is unknown,
  such as a buff's spell. Untold or `None`, the session waits 10 s, which
  departs where a total is not 0: slower below 10 s, faster above it or when
  negative.
- Any posture change before the send, a stand and re-sit between two ticks
  included, cancels it with `WorldEvent::PoisonInterrupted { string_id:
  0x30d1 }` and nothing sent. A click checks this first, so a stand and a
  click before the next tick cancel and start anew. Leaving the zone drops
  it silently, as the program's zoning does.

It never removes the poison itself: the server's own inventory record does,
applied or not. `WorldEvent::PoisonAnswered { slot, applied }` is the
server's answer and ends the request in flight; the program says 0x3269 when
`applied`, else 0x326a, in colour 0x10e. The program's lines for 0x305f,
0x305d, 0x305e and 0x30d1 are in colour 0x111.

## Command diagnostic identity

`GameCommand::name()` returns its exhaustive variant name as a static string,
without command fields. Shared refusal diagnostics use that name; their
structured result events retain the original reason and request identity.

## EQMac wear updates

`appearance::EQMAC_WEAR_CHANGE_OPCODE` and `eqmac_wear_change` decode the
12-byte `EQMac` record into the existing `WearChange`/`WorldEvent::WearChange`.
The shared EQMac message path delivers it for TAKP and Quarm. The player's
record and named spawns apply the received slot's material/model and tint;
zero selects bare armor or an empty held slot. Hosts consuming the typed event need
no new event variant. A visible equipment update does not acknowledge an
inventory move or create an inventory item.

The slot is byte 2, the 16-bit material/model is at 4, and the tint is at 8;
alignment bytes are ignored. Length must be exactly 12, the spawn ID nonzero,
and the texture slot 0 through 8. Titanium retains its separate 9-byte codec
and slot validation. See TAKP's
[`WearChange_Struct`](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/common/eq_packet_structs.h)
and [`Mob::GetEquipmentMaterial`/`SendWearChange`](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/zone/mob.cpp).

## EQMac note and book reading

TAKP and Quarm now offer `Capability::Reading`. Existing `ReadItem`,
`ReadRefused` and `BookText` events are reused. New game codecs are
`books::eqmac_request`, `books::eqmac_text` and `EQMAC_READ_OPCODE`.
EQMac item decoding takes the raw kind byte from BookType at offset 228 rather
than deriving it from Book at 230; unknown kind bytes remain numeric.
Only received text populates reading state. Encoding refusal remains a
local `ReadRefused`; transport errors remain errors, and submission does
not produce a fabricated read-success event or change holdings.

## Inventory planning policy and arrival weight bounds

`Client::inventory_move_rules()` returns `Option<MoveRules>` before connecting.
The selected `ServerType` supplies this same value to its worker inventory
factory. Hosts must use it for queued placement projections, rather than
assuming `EqEmu` rules from the installed generation. None means there is no
inventory movement policy; it is not permission to choose a fallback.

`Inventory::carried_weight_tenths` can retain a conservative upper bound after
a distinct plain cursor arrival follows a recorded weight-neutral history. An
originally occupied cursor additionally needs a plain whole-item relocation trace.
`carried_weight_is_conservative()` identifies that property state and does not
grant readiness. A server prefix where the arrival remains queued weighs less.
`can_rebase_cursor_arrival(update)` exposes the same read-only classification
to the session's resync check; callers still apply the actual update through
Inventory and retain all other correction guards. It is not move permission.
The old item's endpoint remains speculative: Cursor leaves its placement
origins intact; Set covers only its observed slot. Neither resolves other
prediction journals or equipment contribution history.

Every recorded prediction must preserve item weight and the coin-purse
coefficient in both dialects before this bound can be used. Affecting intermediate
histories remain unavailable even after equal endpoints; a full Snapshot or an
existing policy-driven Settled establishes a new baseline. Repeated IDs,
non-plain arrivals and ambiguous occupied-cursor traces cannot start this rebase. Later
unsupported changes to rebased slots invalidate the bound. No new silence
timer, packet meaning or per-move acknowledgement is introduced.

## Named character deletion at world selection

`WorldEvent::CharacterSelection` now requires the actual `connection_id` and
`deletion: Option<deletion::Rules>`. Each world connection has a checked,
process-monotonic generation; each published list has a fresh monotonic identity.
Hosts must preserve both in `deletion::Scope`, invalidate unanswered confirmations
on any new list, and invalidate all prior intents on reconnect. Do not invent a
default scope. `GameCommand::DeleteCharacter(Request)` requires a positive local
request ID, an encodable name and an exact repeated confirmation. The world owner
checks that scope, capability, exact unique listed identity and selection-stage
ownership before sending. IDs and confirmation text are never wire fields.

`WorldEvent::CharacterDeletion(Update)` carries the full original request and
`Progress`: `NotSent(Refusal)`, `Pending(Option<Observation>)`,
`Unconfirmed { reason, observation }`, or `ObservedAbsent { scope }`. An observation
names the received list and connection. Still present is not a correlated refusal;
absent is not a deletion ACK, causal success or database purge. No transport send,
silence, timeout or disconnect establishes absence. Exact intent replays never
resend. Ambiguous attempts hold creation/entry; the host must retain explicit
Back/Disconnect recovery. Ending the connection reports unconfirmed. The frontend
may report `NotSent(QueueUnavailable)` if it could not enqueue a request at all.
Exhaustive command/event matches and complete selection literals must migrate.

Pinned public source: [EQEmu world handler](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/world/client.cpp),
[TAKP world handler](https://github.com/EQMacEmu/Server/blob/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f/world/client.cpp),
and [Quarm world handler](https://github.com/SecretsOTheP/EQMacEmu/blob/c2cd2651045ab063b236b08f3a9e83cbf10289e9/world/client.cpp)
refresh the character list after an ownership-checked operation, without a
correlated deletion response. TAKP/Quarm may mark instead of purge characters.
The native request is the selected ASCII name followed by NUL, not a padded
64-byte record (Titanium sender address 0x4f0e94; EQMac sender 0x53b21b). Titanium
uses application opcode 0x26c9. The library's big-endian legacy application writer
uses 0x5a40, producing bytes 5a40; current TAKP spells the host opcode 0x405a and
writes it little-endian. P99 server internals and live behavior are unverified;
its policy uses the evidenced Titanium request with observation-only outcomes.

## Typed spell-provider policy and attempt identity

`Client::spell_table_policy()` returns `ProviderPolicy` before connecting,
resolved through the same selected server policy that owns table emission.
Hosts bind this value when creating their producer; no protocol-name branch
or first-table ordering assumption is needed. Other policies retain installed
definitions without inheriting a replay overlay.

`ProviderPolicy::requires_server_tables()` is the network-owned semantic
predicate for whether installed mechanics require registration/replay facts.
Hosts use it for lifecycle routing and initial readiness instead of matching
a provider variant. This additive const method preserves the enum variants,
serialization and existing policy accessor.

`SessionStatus::run_id: Option<u64>` carries the numeric attempt ID shared with
`spell_tables::Context::run_id`. A real Connecting notification establishes
the attempt; a table's ID alone cannot rebind the host. It remains stable
across world/zone transports and changes on an actual retry. Provider epoch
is independent and source snapshots survive same-provider transport changes.

The string-taking `SessionStatus::new(state, session_id)` is already present in
the [v0.1.2 release source](https://github.com/eq-p99-tools/eq-network/blob/v0.1.2/crates/eq-network/src/client.rs).
Its signature and display label are preserved. It leaves `run_id` unknown;
`with_run_id(id)` supplies independently known identity. The extensible status
struct gains an additive field; display-only serialization omits that field.
There is no text parsing, hashing, default numeric ID or fabricated authority.
The production emitter always supplies Some from the attempt's sequence owner.

Known consumers checked here are the network production status emitter, network
synthetic lifecycle tests, and the graphical client's three bridge fixtures.
The latter now supply explicit typed IDs. The older standalone logger defines
its own status type rather than calling this constructor. Other external
consumers remain compatible with the old string constructor but must opt into
typed identity before using it as run authority. Hosts must treat an unknown
ID as unavailable for identity-dependent actions, without aborting chat or
other connection processing. `session_id` remains display/logging text only.

## Equipment contribution uncertainty

`Inventory::equipment_stats_uncertain()` tracks stat-bearing equipment histories
whose decoded direct bonuses or worn effects might change. Only readable rules,
explicitly zero direct bonuses and no worn effect establish neutrality. Affecting
or missing metadata stays uncertain through all intermediate local moves, even
when final placement equals its origin. Partial slot records carry no operation
revision and cannot retire this history, even when matching current placement.
A full `Snapshot` or existing policy-driven `Settled` starts a new contribution
baseline; this adds no EQMac settlement timer. Existing origins and ordered
predictions keep their slot-coverage behavior independently. The gap for affecting
or unreadable equipment remains conservative until such a baseline. The accessor
never establishes carried weight, receipt or server acceptance. It must be used
with a received, non-stale model whose complete history was recorded from its
baseline; cloning preserves that history, endpoint reconstruction does not.

## Augments

Titanium serializes a common item's augment sockets as the item's children
(`EQEmu`'s `GetAugment` is `GetItem` on a common item; 5 sockets), on a
worn or carried item and two deep on an item in a bag. The reader keeps
each socket's augment id in `InventoryItem::augments` and places no
augment in the inventory (6.570); before, any augmented item made the
whole inventory unreadable. A child of a common record is read as an
augment even where the record claims room (ours, 6.570 G1: only a bag has
contents in `EQEmu`). An augment past socket 4, an augment with children,
or a child of any other non-bag class is still unreadable. An augmented
record carries no link hash. **What augments add is not summed yet, so
anything derived from an augmented item reads as unknown, never as less**:
`Inventory::carried_weight_tenths` is `Err(WeightUnavailable::Augments)`
while any counted item is augmented (the Titanium program adds augments'
weight), and `equipment_stats_uncertain()` is true while any worn item is.
`EQMac` has no augments.

## Ordered host inventory projections

`InventoryMove::sequence` optionally names a monotonically increasing local
request and its preceding queued move. Snapshot-based callers set it to `None`.
Projected callers advance the same `Inventory` planner locally after enqueue,
retain exact revision checks, and submit commands in order. A failed predecessor
cannot authorize a descendant whose revision happens to match a later update.
These identities are admission-local host metadata and add no wire fields.

`WorldEvent::InventoryAction` now carries the echoed sequence and an optional
post-send prediction. Apply that prediction once on successful local submission;
the worker no longer emits it as a separate `WorldEvent::Inventory` event.
Refusals have no prediction. Neither a local result nor the ordering identity
confirms server acceptance. Downstream request literals and event matches must
be migrated; this is a breaking development API change for the next minor
pre-1.0 release. Authoritative inventory updates remain ordinary inventory
events, with existing uncertainty, correction and snapshot rules unchanged.

## Ordinary cursor drops

`Capability::GroundDropping` offers `GameCommand::DropItem` on local-only
Titanium EQEmu. Public servers await separate validation of item serialization,
eligibility and cursor updates. Pass the current admission,
own spawn, inventory revision and exact received cursor item in
`InventoryDrop` after explicit user consent. Submission requires current
carried item and coin evidence sufficient to bound the load.
`Inventory::plan_ground_drop` requires a positively identified ordinary
droppable instance and an unqueued cursor. A predicted move settled by silence
leaves the drop unavailable until the server identifies the new cursor.
Stock EQEmu's ordinary move to cursor saves the slot without a matching item
packet, so a later received update or snapshot is needed.
[EQEmu move and persistence](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/inventory.cpp#L2210-L2230)

Titanium sends an empty `OP_GroundSpawn`. The server chooses the cursor item
and position. A successful `InventoryAction` with
`InventoryUpdate::GroundDropPending` reports local submission and fences
cursor custody. Apply it once at the matching admission and inventory
revision. A fresh received inventory snapshot releases the fence. Stock
Titanium EQEmu can send the object without sending cursor removal. A bare
cursor removal or quantity update identifies the slot only, and may refer to
another item after a silent drop. Preserve the last view and mark inventory
stale on such an update. Re-enter the world for a fresh inventory baseline.

During the hold, a received limbo item may already be the server's current
cursor. Both drop submission and bound calculations require known purse and
cursor coin balances. The worker receives the cursor balance with the bank
balance and forwards them in `CoinsElsewhere`. Bank coins add no carried
weight. On a complete, otherwise unambiguous inventory,
`Inventory::carried_weight_tenths` takes the heaviest of the displayed and
queued cursor groups, plus the ordinary non-cursor load. It counts one
possible cursor group. The worker validates fresh speed inputs against that
bound before granting active movement. A conflicting cursor update or
invalidation withdraws active movement until received recovery. Supported
passive descent and ordinary completed-fall reporting continue during the hold.
For a landing, `Inventory::ground_drop_weight_tenths` also preserves the
bound through slot-only reductions and complete cursor-only replacements,
including a temporary resync token. It retains the heaviest candidate seen.
An unreadable inventory update or a carried-slot change that can increase
weight invalidates this proof. A host with unknown load can report only a
landing cost established independently of load. A landing whose cost
depends on an unknown load during the hold is not reported. That is more
lenient than the official client, and the local-only EQEmu drop fence keeps
it off public servers.
[EQEmu cursor buffer](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/inventory.cpp#L916-L957)
[EQEmu weight calculation](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/client_mods.cpp#L720-L784)

Process received ground spawns and removals through the existing object
owner. The spawn carries no cursor instance correlation, and pickup delivery
still comes from received inventory. Downstream exhaustive command, capability
and inventory-update matches and `ItemPlacement` literals need migration.

## Local loot request disposition

`WorldEvent::LootRequest` adds an exhaustive event variant. Its `LootRequest`
describes the original open, item or close action, retaining corpse ID and every
item-request parameter; `session_id` is the requested admission, even for a stale
command. `created` echoes the original open/item creation `Instant` for local host
correlation. Close has no creation time in its existing command. The process-local
`created` field is omitted from serialized event logs; it is not a wire timestamp
or a new transaction identifier.

`LootRequestStatus::Refused` means nothing was sent. Shared admission/freshness/
policy checks, corpse/list/cursor guards and generation encoding all use this
single result path. The existing diagnostic is retained once for logging; hosts
should handle the typed result once rather than also presenting the diagnostic.
`Sent` follows successful transport submission only and proves no server
acceptance, inventory delivery or coin change. Transport failure remains an error,
without fabricating either disposition.

Hosts should match the admission, original action and available creation time
before clearing a pending request, and keep stale results from changing a newer
request. Local refusal must not retire an item-view row, fabricate a closed
corpse or trigger an automatic retry. Server `LootUpdate::Opened` denials,
`Taken { accepted: false }`, EQMac receipt echoes and completion remain on their
existing server-update path; an unqualified completion does not identify a cause.
Ordinary inventory/coin events remain the only holdings authority. No packet,
capability, inventory owner or automatic EQMac loot-all behavior changes here.

## Spoken languages

`chat::Tongue` is the language a message is spoken in and the skill it carries.
Its fields are private: a tongue is `Tongue::COMMON` (language 0, skill 100),
`Tongue::spoken(&PlayerState, language, Speaker)` for a typed line, or
`Tongue::unslurred(&PlayerState, language, game_master)` for the away reply.
Both are `None` when the profile lacks the language or holds no skill in it.
No caller can send a language or a skill of its own.

The skill is worded as the Titanium program words it (read R49-4b):
- the profile's skill less `PlayerState::intoxication >> 2`, never below 0,
  language 0 included (`unslurred` takes nothing off);
- 100 when `Speaker::game_master`;
- -1 (`u32::MAX`) when the speaker is a game master and
  `Speaker::log_server_word_zero` is set.

`Tongue::skill()` is the wire's u32. `SendChatInAdmission` carries one;
`chat::encode_outbound_in` writes it (Titanium: language u32 at 128, skill u32
at 144; `EQMac`: u16s at 128 and 134, the skill cut to 16 bits, inferred);
`encode_outbound_for` and `SendChat` write the common tongue. RoF2 is refused.
The program sends the same tongue on every channel: `EQEmu` and TAKP take the
language from every channel, use their own record of the skill, and turn OOC,
GM, broadcast and petition chat back to the common tongue themselves.

`PlayerState::intoxication: Option<u32>` is the profile's (Titanium u32 at
128, `EQMac` byte at 4968; `None` on RoF2). No later change of it is decoded.

`WorldRules::listener_garble: Option<bool>` is Titanium's `OP_LogServer` word
at 16, which turns on the program's second garble stage by the listener's own
skill (read R49-4c). `EQEmu` leaves it 0. The `EQMac` word at the same offset
is `WorldRules::gibberish`, which TAKP's program reads the same way (read
R49-4m).

A received `OP_ChannelMessage` now fills `ChatEvent::language` and
`ChatEvent::language_skill` on both generations, as sent: `EQEmu` sends 27 for a
tongue the listener knows too little of (skill under 24) and the lower of the
two skills; TAKP sends the language and the speaker's skill. Values past 254
read 255. No garbling or learning happens here: the servers raise language
skill themselves and send the skill update `PlayerState::languages` follows.
The JSON of a channel message gains the keys `language` and `language_skill`.

## Language-zero name selector

`WorldRules::language_zero_selector: Option<rules::LanguageZeroSelector>`
retains the complete received Titanium `OP_LogServer` first dword. The
wrapper's zero/nonzero test chooses between installed language-name strings;
no broader server feature is inferred. `None` is absence, while
`Some(LanguageZeroSelector(0))` is a received zero. Consumers must not use the
separate PvP or no-drop-trading values as substitutes.

The existing `WorldEvent::Rules` route carries this field. The EQMac decoder
leaves it absent pending verified wire mapping. Consumers retain the setting
with the same world's rules across zone/character admissions, replace it on
a received value, and clear it when that rules owner starts a new session.
An absent field in a later partial rules copy does not erase a received value.
Installed data remains the source of text. Complete `WorldRules` literals
must add the field or use its default for absence; this source API addition
requires downstream consumer validation before publication.

## Session-owned EQMac fatigue

`SpeedInputs` adds optional `eqmac_fatigue: Option<EqMacFatigueInputs>` with
signed unpenalized `strength: i64` and effective `stamina: Option<u8>`.
Existing rate, weight and final capacity (including the explicit zero sentinel)
retain their meaning. Only EQMac with this component derives capacity from
current session fatigue; legacy absent components retain the existing host
refresh/withdrawal path. Other generations use final capacity directly.

Known stamina is capped to 1..=255; missing/zero stamina or missing fatigue
retains the conservative maximum ten-point penalty. Subtract with saturation
before the final 1..=255 cap. Known nonpositive strength never becomes the
zero/no-encumbrance sentinel. The host supplies one proven prerequisite
baseline and a stable conservative fallback capacity. Session-derived
capacity does not rewrite the exact accepted host report or its receipt.
Unchanged fatigue/derived speed stays quiet; true prerequisite invalidation,
root, correction, death and transfers retain their guards.

## EQMac admission zone identity

`quarm::profile_zone` reads the current zone from the supported encrypted
profile. Zero or a value outside the transfer interface's `u16` range returns
`None`, never a truncated alias. Malformed compression/layouts remain errors. The
bind point and zone-description name do not substitute for missing identity.
EQMac admission keeps an otherwise valid profile when its current zone is
unknown, and commits representable current zones with no instance field
before zone-line commands can run. Unknown identity retains the existing
transfer refusal. No transfer packet layout or server policy changes.

## Quarm death-bind handoff

Quarm's source-defined death-only bind offer produces admission-scoped
`WorldEvent::DeathBind` before the transfer. It carries lifecycle knowledge,
not a death packet: no killer, corpse or kill credit is synthesized. Ordinary
death packets retain their full details. Duplicate bind offers do not repeat
death or departure; dead state survives an approved departure. This
classification is enabled by the Quarm provider, not generic bind offers.

For Quarm only, approval of a solicited pending bind transfer to the exact
current zone accepts its documented reply stand-in 185. Character, instance
and success checks remain; normal transfers and other profiles retain exact
destination validation. The stand-in is not stored as the real destination.

## Movement proposal results

`WorldEvent::MotionSent` reports the worker's current accepted position and
the local outcome of the proposal. A locally accepted sample can be submitted
immediately, held for pacing, or left unchanged by the update gate. The event
describes worker state only. `WorldEvent::MotionRefused` reports a discard
without an accepted position to publish. Both events carry the original
admission and process-local `created` stamp. The stamp is omitted from
serialized logs. Match both fields to complete a pending proposal. A Move
dequeued by zone dispatch or selection has one local terminal result while
the event handler can receive it. A fatal zone error refuses Moves already
drained into the abandoned batch. Commands still in the host queue when the
worker stops have no per-request result. Exhaustive consumers must handle
`MotionRefused`. `MotionSent` constructors and exact field patterns must
include `created`.

## Camping and the movement owner's pending stop

`MotionSession::stop_pending` observes movement, a held moving update, and a
last moving submission whose real stop has not yet succeeded. Stationary
heartbeat repeats do not block camping. `is_trailing_stop` classifies a newer
same-admission request at the owner's exact point/heading; normal guard,
codec and submission checks still apply. Neither method sends traffic or
establishes a server acknowledgement.

`stop_context` supplies an equality-only admission/input-epoch guard for a
waiting intent. `accepted_stop` observes the real committed request stamp,
so a rejected trailing stop cannot later start camping on an idle heartbeat.
An accepted held stop still waits for its actual submission. These queries
do not infer success from the host's `MotionSent` notification.

A moving player stops only by the host's own request at the current point
(zero displacement and no turn), or by a stun, withdrawal or correction,
each of which holds a stop for the update gate. A gap without input, however
long, sends nothing new: the program's movement and sending share one loop,
so a stall there only delays the next update. A host that stops driving
movement while the player moves must submit that stop first.

Camp retains its own waiting intent while the existing paced stop is pending,
then starts its countdown only after camp submission succeeds. New movement,
refusal, withdrawal or admission/lifecycle loss cancels that waiting intent.
Cancellation reports `CampStatus::Abandoned { by: CampEnd::Other }` when its
admission/movement context is lost or a transfer arrives. It clears the local
intent before notification, so overlapping invalidation/transfer/death events
report once and a failing callback leaves no intent to replay. Player actions
retain their existing typed causes; no cancellation sends a camp packet.
The established motion timer precedes the camp timer. Official stop/camp
ordering remains unverified under recording checklist 6.380; no spacing,
repeat, wire format or movement authority changes.

## Quarm loot receipt and delivery

`LootUpdate::Acknowledged { place }` is a new exhaustive-enum variant and
requires downstream match updates in the next minor pre-1.0 release. EQMac
servers echo item requests before checking whether they can deliver them.
Consumers must not treat this event as `Taken { accepted: true }`: ordinary
`InventoryUpdate` events establish received items, including late deliveries
after the loot window closes. No destination or successful transfer is
predicted by the loot feature. The existing inventory feature owns items and
coin grants. See `docs/eqmac-looting.md` for evidence and remaining live checks.

The loot controller correlates a visible corpse request, successful grant and
completed listing in one admission. It refuses stale-session commands, places
outside that listing and repeat requests for a place already submitted. Close
waits for the server's completion; death, zoning, corpse removal and reconnect
discard access state. No timeout or receipt echo fabricates item delivery.

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
- `GameDialect` (the client generation) and `ServerProtocol` (the server
  type) are deliberately exhaustive. Adding a variant to either is a breaking
  change by design: every host and library `match` on them must then decide
  what the new generation or server type does. Do not put `#[non_exhaustive]`
  back on them. Decisions on them are exhaustive matches, or named predicates
  whose bodies are; a comparison on one variant (`==`, `!=`, `matches!`,
  `if let`) is refused by `crates/eq-network/tests/exhaustive_checks.rs`.
  [docs/adding-a-generation.md](docs/adding-a-generation.md) lists what a new
  generation needs.
- Carry typed payloads, quantities, slots and capabilities. Introduce an ID
  newtype where it prevents meaningful mixing; avoid wrappers without an invariant.

## Feature ownership and recovery

Inventory moves do not hold the inventory while awaiting another move's reply.
`MoveRules::settles_by_silence()` preserves the legacy Titanium refusal window;
both EQMac policies require authoritative evidence and never settle on a timer.
`Inventory::unresolved_predictions()` counts ordered changed-slot projections
whose uncertainty remains, not positive move acknowledgments. A covering slot
update resolves only its slots across affected projections; it never confirms
an unrelated move by order. EQMac contradictions invalidate until `Snapshot`.
See [inventory prediction semantics](docs/inventory-move-predictions.md).

`doors::eqmac::decode` and `objects::eqmac::decode` expose EQMac layouts through
the existing typed door/object updates; `message::eqmac` routes them on the
generation wire. `objects::eqmac::pickup` and `request::eqmac(Request::PickUp)`
encode the established EQMac click body. Server policy still controls whether
the session offers pickup or door interaction. This adds no event variants or
inventory writer. EQMac's refusal sentinel becomes the existing
`ObjectUpdate::Kept`, which identifies the actor but cannot identify the object.
`ObjectAction { error: None }` still means transport submission, and disappearance
still means visibility changed; only authoritative item packets grant inventory.
Requests older than an object's replacement are refused without a send. See
[the evidence and activation limits](docs/eqmac-world-objects.md).
One unresolved EQMac pickup holds only further pickup requests; unrelated
inventory actions retain their existing guards. Another actor's or unattributed
removal of the pending object, an authoritative inventory snapshot, or a
three-second local wait expiry cancels that hold without acknowledging success,
granting an item, settling a move, or sending a retry. Expiry is a local
duplicate-suppression bound, not a server response guarantee. After cancellation,
constant-size admission-scoped state records that late untagged/cursor results
cannot uniquely answer a newer pickup, even with a reused object ID. The newer
wait can still be cancelled by an authoritative snapshot or its own expiry;
ambiguous removal packets cannot cancel it. Only a new admission restores unique
result attribution.
The result rule is selected through the generation interface; the established
Titanium path is preserved. TAKP and Quarm offer `GroundItems` with this rule.
Their door definitions/actions are replayed at admission. Both offer `Doors`
with a click owner restricted to ordinary EQMac types 0--8 and the documented
definition, empty-cursor, freshness, motion and reach guards. Hidden server
locks, movability and scripts retain authority; submission is not an opening.
Additive APIs `doors::eqmac::click` and `request::eqmac(Request::ClickDoor)`
encode the eight-byte ordinary no-held-item request with the player's ID.
`doors::eqmac::ordinary` identifies supported definitions, and `Hinge` exposes
their local motion/return clocks for the network guard and frontend pose.
Only received definitions/actions set those targets; a click predicts no state.
Specialized or unknown mechanisms remain visible without ordinary use. No
event variants or definition fields change.

One feature owns each command and its state. Wire generation determines layout;
server policy determines availability. New features start absent, and a frontend
must honor the session's offered capabilities and explicit player choices.

Resource holds cover an operation until its answer settles it. Where a server
refuses with silence, as `EQEmu` refuses a merchant offer, the hold ends after a
timeout, and the operation is remembered with what it acted on so that a late
answer still settles it. A late reply must not mutate an unrelated replacement
slot or release a newer operation's hold. When the wire cannot distinguish
outcomes, invalidate and reconcile rather than guess, replay, or invent a server
request identifier.

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

Incoming world-update rejection diagnostics retain the original reason and
append `opcode` and `payload_len` for passive diagnosis. The length covers the
application body only, excluding opcode and transport framing. No body bytes are
added; message counts, typed events and decoder acceptance rules are unchanged.

Report a refusal only where the server refused, or where its silence is how it
refuses, and never treat transport send success as authoritative completion. A
timeout never authorizes resending a non-idempotent action.

## Validation and release integration

Normal combat requests emit `WorldEvent::CombatRequest { session_id, request,
status }`. `combat::CombatRequest` distinguishes `Consider { target_id }` from
`AutoAttack { enabled }`; `CombatRequestStatus::Sent` means transport submission
only. `Refused(reason)` means nothing was sent, including an expired request,
an obsolete admission or a server policy that does not offer combat. The reason
is display text; applications must use the status variant for control flow.
Exhaustive `WorldEvent` matches must add the new event. These are unreleased
API additions, not a compatible published patch.

No authoritative autoattack acknowledgment exists on the inspected EQMac path.
A host may display its requested toggle separately; it must not label `Sent`
as server-confirmed combat, a hit or a successful consideration. Only existing
`Consideration`, `Damage` and `Animation` events carry server results. A consider
answer names its own target; it must not be applied to whichever target the host
has since selected. Unknown con/damage values remain observable. Neither target
range nor weapon eligibility is recalculated by the normal toggle owner.
See [combat evidence and limits](docs/quarm-combat.md).

Hosts can read `Client::consider_npcs_by_level` before connecting. Every
current server policy prefers the reply's color. The policy affects display
only and leaves the received color and faction available separately. The
`combat::Consideration::faction` docs give the wire order and cite the pinned
server swaps.

This library workspace intentionally does not commit `Cargo.lock`. CI resolves
dependencies for each toolchain: stable format, Clippy, tests and docs on Linux;
native tests on Windows. These checks
neither load proprietary assets nor connect to a live game server. A fresh
downstream resolution and packaged-crate consumer remain release checks,
because a downstream application owns its dependency lockfile.

For each changed feature, record protocol generation, server policy, repository
revision, capability and scenario. Separate source-derived behavior, synthetic
codec/state-machine tests, and live evidence. A check on one server type does not
list a feature on another: one checked on TAKP waits for its own check on Quarm.

A Mac server type may also list a feature from a read of that server's own
handlers at a named commit, never from another server's: the offer is
labelled "source-derived, not live" in its server type, its checklist row
stays open until a live row, and a live defect withdraws the offer on that
server first. Quarm's source is public (SecretsOTheP/EQMacEmu). Quarm offers
`Giving`, `Grouping`, `Listing` and `Translocate` this way, from its handlers
at fc75fcf (6.576). Its own refusals (self-found and challenge modes, Iksar
zones, GM checks) answer as its lines. A Quarm translocate is answered as on
TAKP, but Quarm sends no second offer after a yes: it moves the player with
its ordinary zone change.

What Quarm sends back on those four, from the same commit: its group packets
use the same layouts as TAKP's. Its trade cancel names who closed at +0 and
carries a fixed 1665 at +2, which is not read. It echoes the player's own
cancel back to them (with their own ID, or the NPC's on a give window), and
echoes the `EQMac` program's second cancel too; the window is already closed,
so the session withholds each echo and sends nothing more. On an NPC's give
window it sends an empty `OP_TradeReset` (0x1040) before the NPC's
acknowledgement and after a cancel; nothing reads it yet, so it is recorded
as the planned exchanges gap, never as an error.

TAKP offers `Looting` the same way, from its handlers at EQMacEmu/Server
44afcbf (6.577): the same 2-byte open and close, 12-byte item request, echo
before the checks and empty-cursor rule as Quarm's. Both servers end the loot
for an item request sooner than 10 ms after the last on the corpse
(`loot_cooldown_timer`), so the session never sends one sooner and refuses
one asked sooner. Delivery and coins stay with the inventory owner.

Quarm and TAKP offer `Who` and `FriendsWho` the same way, each from its own
handlers (6.583): the 140-byte `Who_All_Struct` of 16-bit fields, and the
friends' comma list ending in its only NUL (Quarm `strcpy`s it), with no name
past the 64 bytes the worlds read. The answer, `OP_WhoAllResponse` (0x0b20),
is read into the same `WorldEvent::WhoList`; when nobody matched, the zone
sends string 5029 instead, which reaches the host as chat. Its account name,
sent only to a game master, stays out of `WhoPlayer`'s `Debug`, and an answer
that doesn't read is a gap with no sample. One who or friends query is in
flight at a time: the next waits for the answer, the 5029 line or 5 seconds,
and comes at least a second after the last, a pace of the session's own (open
row 6.583a for the program's). One asked sooner is refused and may be asked
again. A plain `/who` stays the host's own list of the zone, as on Titanium
(open row 6.583b).

Quarm and TAKP offer `Rolling`, `Assisting` and `Splitting` the same way
(6.585), from handlers the two servers share at Quarm fc75fcf and TAKP 44afcbf.
- **Rolling.** A roll is Titanium's 8 bytes on the `EQMac` opcode (0xe741). Its
  `OP_RandomReply` (0x1640) is read into the same `WorldEvent::Roll`. The
  session sends ends of 0 to 32000, in order; that limit is ours, as P99's
  program sets it (open row 6.585b for the `EQMac` program's).
- **Assisting.** An assist is 2 bytes (0x0042). The server always answers on
  the same opcode, into `WorldEvent::Assisted`, with the assistee's target, or
  with the player's own spawn when there is none or the assistee stands past
  250. That spawn is passed on as named (open row 6.585a). Only one assist is
  in flight at a time, for up to 5 seconds.
- **Emotes and yells** are not offered on these servers yet.
- **Splitting.** `/split` is the 16-byte record on 0x3141, and `/autosplit` is
  appearance type 28. The profile's byte at 3348 says whether automatic
  splitting is on.
  - Like the program, the session takes the coins from the purse first.
  - A refusal sends its line (12328 or 13112, which arrives as
    `Message::SplitRefused`) and then four purse additions, platinum to
    copper, zeros included. The purse is known again only if each kind came
    back as asked.
  - A split that goes through takes the coins' worth from copper first and
    makes change upward without telling the client
    (`split::eqmac_take`). It then pays the share as the same four additions,
    so the purse becomes what the take left plus the share.
  - With fewer than four additions within 3 seconds, the purse is unknown.
    Quarm pays a self-found splitter nothing.
  - One split is in flight at a time; another asked before the answer is
    refused. The purse settles only if nothing else moved it during the
    wait, and is otherwise unknown.
  - What the `EQMac` program's purse shows after a split that made change is
    unread (open row 6.585d).

Quarm and TAKP offer `Corpses` the same way (6.591), from Quarm fc75fcf and TAKP
44afcbf.
- **Consent.** A consent is the name and its NUL on 0xb740. It toggles on
  these servers, and neither has a deny, so `ClientCommand::Consent { given:
  false }` is refused (open row 6.591a). The answer on 0xd540 is the same
  161 bytes, read into `WorldEvent::Consent`, and only the player consented
  gets it.
- **`/corpse`** is `OP_GMSummon` (0xc540, 152 bytes). It is sent only for a
  player's corpse the session sees in the zone, and at most every 250 ms. A
  `/corpse` sooner than that is withheld without a refusal. That floor is ours
  (open row 6.591c), and the Mac program's `/corpse` opcode is open row
  6.591b.
- **`/corpsedrag`** is 152 bytes on 0x1441.
- **`/corpsedrop`** is offered on Quarm alone, on its own opcode (0x1337 on its
  wire): one byte drops all corpses, and the 152-byte record drops one. TAKP
  refuses it.
- **Refusals.** The local refusals of a consent cite no string numbers here.

Quarm and TAKP offer `Pets` the same way (6.595), from both servers' identical
`Handle_OP_PetCommands` and `zone/pets.h` at Quarm fc75fcf and TAKP 44afcbf.
- **Commands.** Each is the 8-byte command and target on 0x4542, with
  Titanium's numbers. Taunt on (13), no cast (18) and focus (19) fall to the
  servers' default and are refused locally. Hold and feign go as asked, and
  the server decides by the player's AAs.
- **Ownership.** A spawn's owner comes from its record's `petOwnerId` (+18)
  in the zone's spawn list and in a new spawn, and from charm.
- **Quarm alone** refuses every command but get lost while the player is
  invisible or hidden, in the pet's own words.
- **Pace.** None (open row 6.595a).

Quarm and TAKP offer `Resurrection` and `Sacrifice` the same way (6.596), from
`Handle_OP_RezzAnswer` and `Handle_OP_Sacrifice` at Quarm fc75fcf and TAKP
44afcbf.
- **Resurrection.** The offer (0x2a41) and the answer (0x9b41) are
  Titanium's 228-byte `Resurrect_Struct`, with no instance. The answer is the
  offer byte for byte, with only the choice set: Quarm takes the zone and place
  from it. Quarm moves the player itself on a yes.
- **TAKP's completion.** TAKP sends its kept offer again with the answer 1
  (0xec41), and the client moves itself. The session follows it only for the
  offer it accepted in this admission, at the same zone and place, as a
  solicited transfer with reason 9 and heading 0 (inferred). Anything else is
  recorded as a gap. TAKP's rez effect, which the `EQMac` program applies
  itself, is not shown (open row 6.596b).
- **Time limit.** A question stays open an hour, then the session declines it
  (inferred, open row 6.596a).
- **Sacrifice.** The 12-byte offer is returned with only the answer set (1 or
  0), on the player's word alone. The Mac program's no is inferred as 0 (open
  row 6.596c).

Quarm and TAKP offer `Training` the same way (6.597), from the identical
`Client::OPGMTraining`, `OPGMTrainSkill` and `OPGMEndTraining` at Quarm
fc75fcf and TAKP 44afcbf.
- **Packets.**
  - Open and answer (0x9c40): 244 bytes, the guildmaster's and player's 16-bit
    spawns. The answer adds each skill's most as a 16-bit value, the price
    scale (f32 at +204), `success` (+208) and 32 language flags.
  - Practice (0x4041): Titanium's 12 bytes with the player's spawn at +2.
  - Goodbye (0x9d40): 4 bytes.
- **Results.** A practice is answered by `OP_SkillUpdate2` (0xbe41), read as
  the player's skill only when it names their spawn.
- **Cost and points.** The server takes the cost and a point without a word.
  - The cost is the cube of the value over 10, times the price scale, times
    0.0099999998, cut to a whole number. A new skill (254) costs nothing.
  - The session projects the purse as the server's copper-first take, or
    leaves it uncertain.
  - Points come from the profile (+156), and each level adds 5.
- **Languages** follow the `EQMac` program (0x47ea1f, 0x4efc95, 0x4ef6c7;
  read 6.597). Both servers accept any language 0 to 25, check no race,
  class or coins, take the server's cost and a point silently and answer
  with only string 449 (`IncreaseLanguageSkill`), so the program's gate is
  the only one.
  - The host hands in the program's minimum-level table as
    `ClientConfig::installed_language_levels`, read from the installed
    program and checked by `LanguageLevels::checked`. Without it, language
    training stays shut.
  - The offer lists only the languages the player already speaks, each to
    the guildmaster's byte for it.
  - A practice runs the gate in the program's order, each refusal a
    `TrainingRefused` with the program's line and nothing sent: too young
    for the table (0x327c), no points (0x327d), mastered at 100 (0x327e,
    naming the language's number), at the guildmaster's limit (0x327f),
    then short of the program's figure in coins (0x2f21, naming the
    guildmaster).
  - A sent practice is resolved at once, as the program does:
    `Trained { skill: 100 + n, value: from + 1, cost }`, with the server's
    figure as the cost, a point spent and the purse paid. This is a
    prediction; the next profile replaces it either way.
- **Caps.** An offer's caps are each skill's most at the server's top level,
  not at the player's level, so a host shows a larger number than the player
  can reach now. A practice at the player's current most is refused with the
  value repeated, which closes it with no spend.
- **Unanswered.** A practice the server leaves unanswered for 5 s leaves the
  points unknown and, when it had a cost, the purse uncertain: the server
  takes both without a word. A late answer is plain skill news. An untrained
  skill the player's level has not reached is likely dropped without an
  answer (`MaxSkill` 0); the `EQMac` program would not ask (open row 6.597d).

Quarm and TAKP offer `Tradeskills` and `WorldContainers` (6.598), from
`Object::HandleCombine`, `Object::HandleClick` and
`Handle_OP_ClickObjectAction` at Quarm fc75fcf and TAKP 44afcbf.
- **Combine (0x0541).** 32 bytes (`Combine_Struct`). Both servers read only
  the container's slot, a 16-bit value at +4: 1000 for the open world
  container, else the pack slot. The other fields are left zero; what the
  `EQMac` program writes there is unread (open row 6.598a). The answer is
  always empty.
- **Open (0xd740).** The click is the 8-byte `OP_ClickObject`. The 20-byte
  answer carries the player, the container, `open` (0 when another player
  uses it), the type at +10, 10 places at +12 and the icon at +16. An answer
  with `open` 0 holds nothing. The items arrive as `OP_ObjectItemPacket`
  (0xfb40) at places 0 to 9, which moves name 4000 to 4009.
- **Close.** The session repeats the answer with `open` cleared. Both servers
  answer with an empty `OP_ClickObjectAction` (`Message::ContainerReleased`).
  Unlike `EQEmu`, neither returns the container's items: they stay in it,
  and its places leave the view. The session closes the container when the
  player dies, as neither server does before the player leaves the zone
  (inferred, open row 6.598d).
- **Combine results.** In a world container, `OP_ClearObject` (8 bytes,
  `Clear` 1) comes before the reset; in a carried kit each component is
  deleted with `OP_MoveItem`, and a recipe that replaces its kit deletes the
  kit too. Refusals for class, race or (Quarm only) era send a line and no
  reset. The session frees the player after 10 s, by time alone (open row
  6.598c).
- **Moves.** Into a world container only whole from the cursor, and out only
  onto the empty cursor. Quarm refuses other world moves as a possible hack;
  TAKP accepts them, so this stricter rule holds on both. A cursor item over
  a different one in a place swaps, as both servers do; the same stack is
  refused rather than predicted, because both servers move it whole.
- **Lore.** A pickup the server refuses with a delete of its destination and
  the duplicate-lore line leaves the item in its place, on every server.

TAKP's and Quarm's boat control is read but not offered (6.602), from the
identical `Handle_OP_BoardBoat`, `Handle_OP_LeaveBoat` and
`Handle_OP_ControlBoat` at Quarm fc75fcf and TAKP 44afcbf.
- **Control (0x2641).** 4 bytes: the boat's 16-bit spawn, 1 to take or 0 to
  give up, and a byte the servers only echo (sent as zero, open row 6.602b).
  A grant or a release is echoed; a take while another player steers is
  refused with the `IN_USE` line and no echo, so the take's wait runs out and
  the session gives it up. Leaving the boat, a zone offer and the player's
  death each give control back.
- **Not offered.** `BoatControl` and `BoatRiding` stay off on both server
  types until the `EQMac` program's steering (a position update under the
  boat's spawn moves it on both servers) and its board, leave and arrival
  wire are read (open rows 9a and 9b).
- **Profile.** The Mac profile's `boat` (+3256, 32 bytes) arrives as
  `Message::VehicleName`, last among the profile's messages. Both servers
  keep it across zones and clear it on `OP_LeaveBoat` and in a zone without
  boats.
- **Casting aboard.** Both servers interrupt every cast while the profile
  names a boat (`Handle_OP_CastSpell`). Whether the `EQMac` program refuses
  first is unread (open row 9c).

Applications using a Git branch still resolve a specific commit in their
`Cargo.lock`. A networking merge is not integrated into an application until its
lockfile changes and that combination passes checks. Coordinate related open PRs
before advancing the pin. Never commit credentials, raw game captures, extracted
assets, or identifying account data as fixtures; construct synthetic records.

## Independent posture and locomotion

`GameCommand::SetPosture` requires `Capability::Posture`. `Moving` covers
locomotion and its speed inputs; it must not be required for sitting, standing
or ducking. All current server policies offer posture independently of
movement readiness. Hosts should gate stance buttons and hotkeys
using the command capability, and send the usual sit then camp sequence.

A step stands a seated player first. A ducking player moves ducked where
`GameDialect::ducking().moves_ducked` holds (Titanium, program read R23): the
step sends no stand, and the grant follows the stance when it changes, in
place, at the sneaking speeds (`SpeedState::ducking`), so a frontend moves at
the `MotionState` it then receives. A stand from a duck goes back the same
way. Where `jump_stands` holds, a jump while ducking sends the standing report
before the jump, and the host hears the player's `WorldEvent::Posture`
Standing; a refused jump stands no one. Elsewhere a step stands a ducking
player first, as before. Headroom is the frontend's: Titanium moves a ducking
player under a clearance of their size plus one, and refuses a stand under a
lower ceiling. The session cannot see the ceiling, so a host that holds the
player ducked for headroom withholds the jump, which would send the stand.

`GameCommand::FeignLanded` also requires `Capability::Posture`. It is the
host's statement that a spell with the feign effect (74) landed on the
player: only the host holds spell data. Send it only for a
`WorldEvent::SpellEffect` on the player's own spawn for which
`SpellEffect::took_hold()` holds (the one landing test: kind 231 and the
flag carrying 0x04, as Titanium's handler 0x47f614 tests it), whose spell
carries effect 74, and where `GameDialect::spell_feign_lies()` holds. A
server type's `spell_landing_flag` answers a different question, which
servers have been checked. The session changes the posture only: one
lying report, or none when the player already lies, with the movement in
hand stopped; no feign request and no auto attack change. With
`PostureOptions::feign_standing` it sends nothing and owes the next
stand. Another spawn's id is refused.

At admission, where `GameDialect::entry_stand().stands` holds (Titanium,
program reads R24 and R24b), the player stands. The posture before that
stand is a report staged before admission, else the own record's
`PlayerState::posture`. After the other staged postures the session
reports, for the player:
- a lone `WorldEvent::Posture` of `Standing` when the posture before was
  standing: the posture the player enters with, not a change, and nothing
  is sent;
- otherwise that posture, then `Standing`: the program's stand, which is a
  change of posture like any other (the program clears the autorun latch
  on it), with one standing report sent where the rule sends.
A report after admission replaces either as usual. The Mac-era client's
program is unread: its own posture stays unknown until set.

## Movement inputs are the host's, by revision

The session grants no walking, turning or jumping until the host answers the
latest `WorldEvent::SpeedInputsRequired` revision with `GameCommand::SpeedInputs`.
A host answers every request, with `SpeedInputState::Unavailable` and its
reason when it cannot yet calculate; an unanswered or older revision keeps the
player still, which is the slow side, never zero load. Hosts should surface the
`SpeedInputUnavailable` reason to the player. Exhaustive matches add
`SpeedInputsRequired` and `SkillReset`; `ItemPlacement` literals add `eqmac`
and `bag_weight_reduction`.

## EQMac inventory decoding policy

`inventory::ItemRules` selects the server's interpretation of the shared EQMac
item layout. `Takp` derives stackability from item type and maximum charges;
`Quarm` uses the flag at byte 276 and retains the same stack-capacity rule.
`inventory::decode_eqmac_with(opcode, body, rules)` and
`message::eqmac_with(opcode, body, rules)` accept that policy explicitly. The
session obtains it from its server type and uses one EQMac generation wire.

These APIs are additive. `inventory::decode_eqmac` and `message::eqmac` keep
TAKP semantics; `inventory::decode_quarm` and `message::quarm` keep Quarm
semantics. Both policies retain raw stack and GM observations in
`inventory::EqMacItemRules`, including unknown values. Decoding policy does
not enable inventory capabilities, grant a move, or change pacing, prediction
or reconciliation. Existing move rules remain responsible for validation.

## Movement-input acceptance

`GameCommand::SpeedInputs` carries the current admission, the opaque revision
from `WorldEvent::SpeedInputsRequired`, an opaque host `report_id`, and a typed
ready/unavailable state. Give each new logical report a fresh report identity;
retain that identity when retrying a full local queue. Recovery to identical
ready values after an unavailable report is a new report. Neither identity is
a wire counter, and consumers must not derive movement permission from it.

A change in the player's belongings (inventory, purse or cursor coins) while
the player holds a grant asks for the inputs again without withdrawing: the
last inputs stay in force until the answer to the new revision lands; health,
stance and fatigue changes still apply at once. An answer the session refuses, or no answer within one
second, withdraws movement as any other input requirement does. Every other
input requirement withdraws at once.

After normal admission/revision validation and generation-rule evaluation,
`WorldEvent::SpeedInputsAccepted(SpeedInputAcceptance)` echoes the exact ready
inputs and report identity with `granted`. A new report is acknowledged even
when its numeric calibration is unchanged. A later change in the accepted
report's grant publishes a new outcome; unchanged ticks do not repeat it.
Unavailable or obsolete reports never produce a ready acceptance. `granted`
is false without a current positive calibration or while lifecycle blocks
movement. This is a local session receipt, not a remote server acknowledgement.

Keep this receipt separate from `MotionState`, which alone carries movement
speeds. `MotionState.jump_factor` is the share of the generation's
jump speed a jump starts with now (the health and weight factor, 1 where the
session does not work speeds out itself); a frontend launches its jumps at it,
and the session's guard bounds an announced jump's rise by the factor the jump
started with. `MotionState.strafe_walk_units_per_second`, where present, is
the strafe rate with walking selected: Titanium strafes on the selected base
speed, so `strafe_units_per_second` is then the running rate. A frontend
names the pace selected in `MovementMode::Strafe(Pace)`: the guard bounds a
`Strafe(Pace::Walk)` sample by the walking rate where the grant has one, and by
the one strafe rate where it has none (`EQMac`), and a `Strafe(Pace::Run)`
sample by the running rate (checklist 6.540). The pace changes nothing on the
wire. **Breaking (6.540):** `MovementMode::Strafe` became `Strafe(Pace)`. A ducking player's grant already carries the ducked rates
where the generation moves them ducked (see below), so a frontend halves
nothing for ducking. On Titanium a ducking or sneaking player strafes on the
selected base halved, both rates given as standing, and a grounded diagonal's
sideways rates are the standing ones (program read R28).
`MotionState.swim_backward_units_per_second`, where present, is how fast the
player backs up in water, in the land's units per second before the water's
share (`movement::swim_speed`) applies: Titanium backs up in water, unless
ducking or sneaking, only by the buffs' speed modifier, so 0 without one. The
guard bounds the component of a `MovementMode::Swim` stroke behind the
heading it ends facing by it; a frontend keeps backing up in water to it. A
ducking or sneaking player's backing in water goes as a
`MovementMode::Backward` sample and takes the land's backward rule by its
mode.
Withdrawals, input requirements and admission/lifecycle resets invalidate
cached acceptance. A frontend may clear a waiting status only for the exact
posted admission/revision/report/input identity with a current positive grant;
a cached prior speed, successful enqueue, or mismatching receipt is insufficient.

Migration: add `report_id` to speed-input command literals and handle the new
world-event variant in exhaustive matches. This changes the unreleased public
API and requires downstream validation; it is not a compatible 0.1.x patch.

## P99 spell-table facts and host providers

`spell_tables::decode` reads variable-length P99 source-ID registration and
destination/source replay records. `SpellId` validates the known 10,000-entry
registry domain for these messages only. Counts, full lengths and every ID
are validated before output allocation. Entry order and duplicates are kept;
an error returns no valid prefix. The packets carry IDs, not definition values.

`WorldEvent::SpellTable(spell_tables::Event)` is a new exhaustive-enum variant;
consumers must handle it. The P99 server policy emits complete tables or typed
unavailability directly from world and zone receive paths, before ordinary
profile/admission filtering. Other server policies do not consume these opcodes.
A failed event callback remains a session failure. No packets are sent for
these messages, and no capabilities or motion rules change.

An event's run ID/revision describes network order, not an installed-provider
epoch. It spans successive world/zone transports in one attempt; zone context
already names the admission session before its profile arrives. A new network
attempt may start new ordering without clearing the host's provider snapshots.
The run ID formatted as sixteen lower-case hexadecimal digits equals that
attempt's `SessionStatus.session_id`. The actual Connecting notification follows
the run reset; a host binds a successor from that lifecycle fact, never merely
from a new run ID appearing in table data. Producer epoch/policy must already
be captured before enqueue so a stale Connecting cannot rebind a replacement.

The host owns its installed-spell provider, typed cached projections, effective
definitions and a separate epoch/policy binding on its event bridge. Under the
chosen lifecycle policy, snapshots survive zone/world transport reconnects in
the same logical provider lifetime. Explicit provider replacement starts a new
epoch requiring actual registration and must not leak overlays to another
installation or server policy. This is an application lifecycle policy, not
a claim that every native teardown path was established.

Registration snapshots a first-seen source's CURRENT effective projection;
duplicates do not refresh it. Replay uses cached source projections in wire
order, retains destination ID/text and every excluded field, and must validate
the entire batch before atomically publishing changes. It must not copy an
entire source definition, chase live aliases or restore omitted destinations. Missing
source data and malformed events must surface as unavailable, never a silent
fallback or partial application. Hosts must route these facts before their
own player/profile gates. This networking API applies no installed definitions.

## Checked local movement preparation

`eq_network_game::buffs::scale_instrument_effect` takes an already evaluated
signed magnitude and a received modifier. The result is a host projection
for movement-related effects. An unresolved modifier returns `None`. The
host's neutral handling of a received zero is a compatibility inference.
Unpatched Titanium scales a stored zero to zero at `eqgame.exe` 0x4102a6–0x4102cb.
The zero-modifier recording case compares the received action and buff slot
with the movement effect on the deployed client.

`MotionSession::send_move_sample` keeps its existing signature and result
semantics. Internally it prepares one checked candidate while exclusively
borrowing its owner. Validation and the generation release decision precede
any send. Held and unchanged candidates commit locally without sending;
immediate candidates commit movement only after the existing send callback
succeeds. A checked candidate is not yet an accepted move or a server receipt.
Dropping it or failing the callback leaves motion state untouched; external
callback effects cannot be rolled back. No public observer/transaction API is
introduced, and the private plan certifies no external body or scene identity.

### Absolute Titanium heading samples

`MovementRequest` has a typed `heading` field. Existing callers using struct
literals can set `HeadingUpdate::Rate` or use `MovementRequest::new` to keep the
existing elapsed-time turn check. `HeadingUpdate::Snapshot` carries a completed
mouse turn as an absolute heading with a zero continuing-turn hint. The zone
owner enables it for Titanium. It shares the existing packet cadence, admission,
position and calibration checks. If both ground axes are active after the turn
has completed, the snapshot may carry a grounded diagonal step. The same signed
longitudinal and lateral limits apply in the snapshot heading. A rate sample
keeps the prior accepted heading when it uses those limits.

## Carried combines and world containers

**Breaking:** `Capability::WorldContainers` is a new variant and
`Capability::ALL` grows from 43 to 44. `Tradeskills` offers carried manual
combines; `OpenContainer`, `CloseContainer` and `Combine(WORLD_CONTAINER)`
require `WorldContainers`. `tradeskills::combine_capability` supplies the
same distinction to frontends. EQEmu offers both; P99 offers carried combines
and the guarded ordinary forge (`WorldContainers` scoped to one-unit placements
into empty forge slots and whole withdrawals to an empty cursor). TAKP and
Quarm offer both from 6.598. Exhaustive capability
matches and station controls must handle the new variant.

`titanium_combine` retains its signature but writes the native full 32-bit
slot, replacing the old invented high-word sentinel, and rejects nonpack,
nonworld slots. Additive
`tradeskills::{check_carried, CarriedRefusal}` exposes the source-backed local
metadata/identity prerequisites without changing any inventory.
`CombineUpdate::Answered` means a received local transaction reset, not a
successful craft or item-move acknowledgment. Components, returned tools and
products require separate authoritative inventory updates. See
[the source contract and limits](docs/p99-manual-combines.md).

## Optional list presence replies

`ClientConfig::answer_list_polls` starts false. P99 offers
`Capability::AnsweringListPolls`; the choice does not enable itself from that
capability. `ClientCommand::AnswerListPolls { session_id, enabled }` changes
future reply policy for the current character, retaining it across zone
crossings. Returning to character selection or reconnecting restores the
configured startup value. A stale admission cannot change the choice.
Disabling cancels an owed reply before its send, including while dead or
zoning. Death, transfer, disconnect and a fresh zone owner drop any owed
answer; re-enabling never answers an earlier poll.

Mode 1 reports population and never joins a list automatically. Mode 2 alone
schedules one presence reply when enabled and admitted. A newer list message
replaces or invalidates an older owed answer, including unreadable messages.
The reply is sent once after a uniformly selected whole-second delay in
3..=60 seconds, on the next loop turn. This is application policy, not native
timing or evidence of server acceptance. No periodic keepalive, fabricated
captcha chat, successful queue membership or delivery receipt is inferred.
Only diagnostic events are produced by this handler.

## Explicit session control options

`Capability::GameplayDepartures` gates every explicit departure from the
official client's behaviour. Only a server type whose
`ServerType::offers_gameplay_departures` is true offers it; `Capability::ALL`
grows from 46 to 47. `SetMovementOptions` and `SetBookOptions` carry that
capability, and a zone session that does not offer it refuses them with
"Gameplay departures are not offered on this server" through the command's
ordinary refusal event, before any owner sees them. A host greys the controls
out there like any other unoffered command. Every checked server type offers
the departures while the owner tests them (2026-11-06); a departure on a
public server is faster or more permissive than the official client, and the
accessor is the one place to withhold it per server type.

One departure is not opt-in and not gated: chat pacing (see "Chat pacing"),
inferred, which only ever makes the client slower and holds on every server
type.

`movement::MovementOptions` starts with both toggles off and ordinary running
speed. The admission-scoped
`SetMovementOptions { session_id, request_id, options, created }` command keeps ordinary movement
unchanged when off. `walk_while_stunned` permits fresh movement input during a
current stun without clearing it or admitting pre-transition input; casting,
items and jumping retain their independent stun gates. `run_speed_multiplier`
scales forward running, including the forward component of grounded diagonals,
and the horizontal budget for a Fall sample. `RunSpeedMultiplier` holds 100
through 140 percent in one-percentage-point steps, with 100 as the default.
Calibration events retain the source-derived speeds. The host applies the
same multiplier when proposing running movement. `fly` permits ascent/descent
within the current mode's unscaled elapsed speed budget. `posture`
(`PostureOptions`) holds two departures, on both generations: with
`feign_standing` the Feign Death press sends the feign request alone,
without the lying posture report both programs send first, so the player
stays standing and moves as any standing player, and a feign spell's
landing (`FeignLanded`) sends nothing; with
`stay_seated_when_hit` an `OP_Damage` hit or submitted local environmental
report leaves a sitter or a looter seated where the generation's program stands
them (`GameDialect::lying()`).
These options never
bypass session identity, input freshness, finite coordinates, fixed bounds, current
known speed, corrections, death or transfer gates. Changes invalidate pre-toggle queued input using the caller stamp and preserve
the accepted sample time for same-frame FIFO movement. A change of flight or
stun permissions keeps the ordinary paced stop; a speed-only change leaves a
held moving sample on its send schedule and applies the new budget to later
samples. Flight toggles preserve a tracked jump, so switching back to
grounded movement cannot qualify a sample for diagonal credit during that
ascent. The tracker expires as accepted samples pass the jump apex. It does
not establish a landing. None represents a native movement
rule, safe server threshold or detection guarantee.

A sample's velocity fields carry the player's own velocity in the
generation's wire scale (checklist 6.617). A swim stroke carries all three,
as before. On land, `MovementRequest::vertical_speed` is the host's own
vertical speed at the moment of the sample, in units a second with up
positive: some while the host has the player in the air, and none on the
ground and on the sample that lands. A rider's speed is its own and leaves
out the boat. Where the generation sends it
(`GameDialect::sends_airborne_speed`: Titanium, and RoF2 inferred), the
session sends that speed. It is bounded up to the read jump's take-off speed
(0 without a read jump) and down to the fall limit derived from the zone's
gravity (`movement::fall_speed`), and a non-number is refused. With none
given, the session sends 0.

Titanium builds its update from the actor's own velocity fields
(`eqgame.exe` 0x465244, read R49-27 (c)), and other clients' pickers draw the
rise and the fall from that vertical speed (0x44fa8f). Capture-derived from
kept official recordings: the field is the speed at the moment of each
update (a jump sends +1.968, then -0.341 a tick, where the interval means
were +0.994 and -0.497), and it is 0 on every grounded update and on the
landing.

Departures and unread cases:
- **Bumpy ground:** the program also sends small values (-0.11 to -0.72 a
  tick) while walking over bumpy ground. The session sends 0 there.
- **Flight and levitation:** the session sends 0, unread.
- **Mac era:** the program's field is unread, so the session keeps sending 0
  on land.

Sending 0 in the air was our departure until now, and the change applies on
every Titanium server type, P99 included.

`SetBookOptions { session_id, memorize_without_sitting }` is an explicit
memorization departure: no automatic sitting and no local five-second
preparation. Scribing retains its existing posture and delay. Spell ownership,
current book, gem, inventory, action holds, stun and server-reply handling
remain. A changed option cancels pending preparation rather than converting
it in flight. A new admission starts with default book and movement choices,
so a frontend reapplies its saved options after admission.

The native Titanium memorization sender (`0x556b72`) owns a variable local
countdown before sending ordinary mode-1 `OP_MemorizeSpell`; skipping our
local timer is an application departure. The inspected public
[EQEmu request handler](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/client_process.cpp#L1145)
checks packet/spell/class/level/ownership and does not reject on posture or
preparation time. Its
[timing detector](https://github.com/EQEmu/EQEmu/blob/4aceae18b94ffaafc08e2b17bc41cd72c77f795d/zone/cheat_manager.cpp#L318)
can record a rule-gated fast-memorization event; the packet handler then calls
the normal action handler. This does not establish deployed server behavior,
absence of monitoring or safe timing. Confirmation still requires server
book/gem state; public-server acceptance remains unverified.

`ExitAtOnce { session_id, created }` is an explicit immediate departure. It
sends the ordinary Logout request without Camp, automatic sitting or the
countdown, then uses the same reply/close owner as normal camping. It does
not bypass admission, life state, session identity or freshness checks and
never resends while already logging out. Neither submission nor the local
close timeout establishes deployed server acceptance. Public-server behavior
and server-side cleanup remain live-validation items.

Movement policy changes report `MovementOptionsApplied { session_id,
request_id, options }` after local installation or `MovementOptionsRefused {
session_id, request_id, reason }` on refusal. The host gates opt-in motion on
confirmed local worker policy and matches its own request identity; enqueue
alone establishes nothing. Identical reassertions can produce feedback but
never reset input epochs. These events are not server acknowledgments and
never imply a movement speed grant. Expired changes can be retried with a new
identity and caller stamp without accepting old motion or reasserting policy
on every render frame.


## Looking for a group

Where the server type takes `/lfg` (`Capability::LookingForGroup`: stock
`EQEmu` so far, from its source and not yet checked live), send
`GameCommand::LookForGroup` with `on: Some(true)` or `Some(false)` to set
the flag, or `None` to turn it over. The session sends Titanium's 80-byte
`OP_LFGCommand` with only the flag set; no level range or comment. `EQEmu`
tells every other client and never the sender, so the session keeps the
player's flag itself: from the admission's spawn record, then each change it
sent, or the server's own word for the player's spawn. A change is told with
`WorldEvent::ListingSet { change: ListingChange::Looking(on) }`; asking for
the state the player is already in sends again but tells nothing. Two
changes are at least 500 ms apart; a refusal is
`WorldEvent::ListingRefused` and sends nothing (checklist 6.515).

## Duel challenges

Where the server type answers duels (`Capability::Dueling`: stock `EQEmu`
so far), the session keeps a duel as the Titanium program does (read
R49-6): the challenge the player sent, the one waiting for them, and
whether a duel is on. It tells the host what happened in
`WorldEvent::DuelNews` with a `socials::DuelNews`, one for each line the
program shows; the host words them from the installed string table.

`GameCommand::ChallengeDuel` is `/duel`. Put the player's target in
`target`, or 0 for none. It always acts on that target:

- At no player, or at the player themself, it sends nothing and tells
  `NoPlayerTarget` or `NotYourself`.
- During a duel it sends nothing and tells `AlreadyDueling`.
- At the player who challenged them, it accepts: `OP_RequestDuel`, then
  `OP_DuelAccept`, each naming the challenger and then the player. The
  news is `Accepting`.
- While another player's challenge waits, it sends nothing and tells
  `ConsideringAnother`.
- Otherwise it challenges (`OP_RequestDuel` only) and tells `Challenging`.
  A second challenge replaces the first. The program has no distance test
  and no time-out, so neither does the session.

A challenge reaching the player is `Challenged`, unless the program would
turn it down on its own. Then the session sends the program's 12-byte
`OP` 0x3ea0 refusal (`socials::DUEL_AUTO_DECLINE_OPCODE`) with a reason:
1 while a duel is on, 2 while another challenge waits or the player has
challenged someone else. Reason 0, the ignore list, is not taken, because
the session keeps no ignore list. The same challenger again, or the
opponent during the duel, changes nothing. A challenge from the one the
player challenged starts the duel with nothing sent (`Begun`).

The player's side of the duel is the one they challenged or accepted
(read R49-6d), and only it ends one:

- **Its death:** `Won` when the player killed them during the duel, and
  the session sends the program's 8-byte 0x3bad {opponent, player}
  (`socials::titanium_duel_over`). Otherwise `Ended`, which also covers a
  challenge that was never accepted. A challenger's death changes nothing.
- **Its leaving the zone:** `Forfeited` during the duel, also with 0x3bad,
  or `Lapsed` before it.
- **Its own program's refusal** (0x3ea0, `DuelUpdate::AutoDeclined`):
  `TurnedDown`, with the reason. This ends the player's side and keeps a
  challenge waiting for the player. `EQEmu` never passes this on.

`EQEmu` takes 0x3bad as its 12-byte decline, so it drops the 8-byte form.
The player's own death, zoning and logging out forget the duel.

A waiting challenge also comes as `WorldEvent::DuelOffer` with a
`socials::DuelQuestion`. The program has no such window, so the host shows
it only as a setting that says it departs from the original. Answer it
with `GameCommand::AnswerDuel`, copying `id` into `offer_id`. A yes sends
exactly what `/duel` at the challenger sends. A no forgets the challenge
here and sends nothing: Titanium has no decline. It closes with
`WorldEvent::ConsentClosed`, `AnswerSent` after an answer and
`Interrupted` when the duel state is forgotten. A stale answer, or one naming a
challenger gone from the zone, is refused and sends nothing (ours:
`EQEmu` looks the one named up without checking they exist). Outside a zone, `/duel` and the answer are refused;
the session makes no posture or motion test, as the program makes none. `Request::AnswerDuel` with `accept: false` is refused
on Titanium.

`EQEmu` maps 0x3bad, not 0x3ea0, to its decline, so on stock `EQEmu` a
refusal the program sends is never told to the challenger. The challenger
stays challenging until they zone or die, as with the original client.
The first P99 use of 0x3ea0 and of the two-packet accept is attended.

## Weapon stances

Where the server type takes them (`Capability::WeaponStances`: stock
`EQEmu`; not yet offered on P99, whose first use is attended, nor on the
Mac servers, whose program forms are unread), `GameCommand::ToggleStance`
is `/sheathe` (`Stance::Sheathed`, state bit 2) or `/aggressive`
(`Stance::Aggressive`, bit 8). The session flips that bit in the player's
state word and sends `OP_PlayerStateAdd` (0x63da) or
`OP_PlayerStateRemove` (0x381d), 8 bytes {own spawn, bit}, as the program
does (R49-6n). Nothing answers and no line is shown. The server passes it
to the others and not back, so the session keeps the player's word itself
until a new zone or the server's own word on the player's states.

## Shielding

Where the server type takes `/shield` (`Capability::Shielding`: stock
`EQEmu`, TAKP and Quarm, each from its own source and not yet checked live;
TAKP's and Quarm's `OP_Shielding` is the 16-bit spawn id alone), send
`GameCommand::Shield` with the spawn id of the player to shield. The server
checks the level (30), the class (Warrior), its rule, the 3-minute recast,
the target (no NPC, not oneself) and the distance (15), and answers each
refusal, the start and the end with a line of its own; nothing answers on
`OP_Shielding`. The session refuses only spawn id 0 and a shield within
500 ms of the last one (`WorldEvent::SocialRefused`, nothing sent). Checks
the original makes before it sends (oneself, the class, the level) are the
host's: they may only be stricter than the server's.

## Disciplines on TAKP and Quarm

TAKP and Quarm send a Mac player no list of disciplines and no reuse
timers. Where they are offered (`Capability::UsingDisciplines`, from both
servers' own `Handle_OP_Discipline`, not yet checked live), send
`GameCommand::UseDiscipline` by spell as on Titanium. The session sends the
servers' number for that spell and the player's class (`OP_Discipline`,
the number and three zero bytes), and refuses only a spell with no number
for the class (`WorldEvent::CastRejected`, nothing sent) and a second try
within one second. The server judges the level, its reuse timer, any
discipline still active and the player's state, and answers each with a
line of its own. `OP_DisciplineChange` reports the discipline begun and
ended. Nothing waits for a cast.

`GameCommand::AskDisciplineReuse` (`Capability::DisciplineReuse`) is the
bare `/disc`: it sends the number 0, under the same one-second guard. The
server answers with a line either way; the one that says a discipline is
ready reaches the host as chat with string 393 and the server's color.

## Surnames

Where the server type takes `/surname` (`Capability::Surnaming`: stock
`EQEmu`, TAKP and Quarm, each from its own source and not yet checked live),
send
`GameCommand::Surname` with the word the host's `/surname` settled on (the
Titanium program's is the first word, quotes taken whole, cut to 19 bytes:
read R49-10). It goes as given, a quoted word's spaces included, its case
left for the server to set. Only what the record cannot carry is refused
(`WorldEvent::SocialRefused`): an empty word, a control character, anything
outside ASCII, or 32 bytes and more. Nothing holds a second request: the
program paces it by its own question. The server answers with
`WorldEvent::SurnameAnswer` (`accepted`, any word but 0 as the program reads
it, and the surname it names) or with a line of its own for each refusal;
every answer reaches the host, asked or not, as the program shows each. The name a taken surname gives the spawns comes from the zone's
own `WorldEvent::LastName`, not from the answer. `EQMac`'s request and
answer (100 bytes) are listed by TAKP and Quarm (6.604 E5), and `EQMac`'s
`OP_GMLastName` gives `WorldEvent::LastName` too. There the session also
refuses first what the servers' comments say the `EQMac` program refuses
(inferred, never more than the server does): a character under level 20,
and a surname over 19 characters. A character who already has a surname is
the server's to refuse, with its own line. A surname stays for
good, or for days, so a host sends it only on the player's own word,
answered yes.

## Titles

Where the server type offers titles (`Capability::Titling`: stock `EQEmu`
so far, from its source and not yet checked live), send
`GameCommand::RequestTitles` to ask for the list the character may choose
from. It goes with an empty body (inferred, pending read R49-14b; the server
reads none of it). One request waits at a time, until its answer, a zone
change, or 10 s, and the next waits the session's 500 ms settling after an
answer. The answer is `WorldEvent::TitleList`: each `TitleChoice` has a
nonzero `id`, a `prefix` and a `suffix`, either possibly empty, each at most
63 bytes, with text read as the `TitleChanged` decoder reads it. A repeated
id keeps its first entry. A list that is short, overruns, lacks a NUL, holds
a zero id or has trailing bytes is refused whole and goes to the gap log by
opcode and length; a list no request asked for is withheld and goes there
too. `GameCommand::SetTitle { title_id, suffix }` chooses from the latest
list of this admission: the id must be on it and have text of the kind
asked (`suffix` true for the suffix, false for the title), or it is refused
with `WorldEvent::SocialRefused` and nothing is sent. `title_id` 0 clears
either kind and needs no list. Each choice the server takes it tells the
whole zone, so two choices, clears included, go at least 500 ms apart; one
sooner is refused and nothing is sent. A new admission forgets the list and any
request in flight. The server says nothing back to a choice it ignores; the
name a choice gives the spawn comes from the zone's own
`WorldEvent::TitleChanged`. Nothing in the client calls `SetTitle` until the
program's title window is read (R49-14). `EQMac` and RoF2 build neither
request: `EQMac` has no titles, and RoF2's numbers are owed (inferred for
RoF2, 6.489).

## Bandolier sets and the potion belt

Every Titanium server's profile, P99's included, carries the player's four
bandolier sets and four potion belt slots (`EQEmu`'s
`PlayerProfile_Struct`: `bandoliers` at byte 6164, 320 bytes each;
`potionbelt` at 12564, 72 bytes a slot). At each admission the session
reads them into `WorldEvent::Bandoliers` and `WorldEvent::PotionBelt`
(6.559). A set is its name and its primary, secondary, range and ammo
items; an item is its id, icon and name. An item numbered 0 is `None`
whatever its name (`EQEmu`'s remove leaves the name), and a set with
neither a name nor an item is `None`. A name with no end within its field
gives one unreadable gap and withdraws only these two events; the rest of
the profile still reads, and the session tells the new
`WorldEvent::BandoliersUnknown` instead of the two. Both are told at the
admission, after `Entered`. P99's era has no bandolier and its bytes
there are unverified, so a host shows them only with the windows that come
later. `EQMac` has neither, and RoF2's 20 sets and 5 slots are owed.

Saving and clearing a set (`GameCommand::SaveBandolier`, `ClearBandolier`,
under `Capability::Bandoliers`) and setting and clearing a belt slot
(`SetPotionBelt`, `ClearPotionBelt`, under `Capability::PotionBelt`) are
built and tested from stock `EQEmu`'s source (6.564), and no server type
offers them until their local live rows pass; P99 never does. Sets and
slots are numbered 0 to 3. Titanium sends `OP_Bandolier` (0x6f0c, 40
bytes) and `OP_PotionBelt` (0x0719, 12 bytes). A save stores the name (1
to 31 printable ASCII bytes) and the id, icon and name of what is worn in
the primary, secondary, range and ammo slots; a belt slot takes an item
the player wears or carries in a pack or a bag in one (never the cursor or
a bank) that is a potion (item type 21) or carries the item's potion-belt
flag (`ItemPlacement::potion_belt`). The server answers none of them, so
after each send the session tells the whole new `WorldEvent::Bandoliers`
or `WorldEvent::PotionBelt`. **That event is the session's own mirror of
what it sent; the next admission's profile is the server's word.** A
refusal is `WorldEvent::BandolierRefused` in the session's words, with
nothing sent: sets unknown, a set, slot or name out of bounds, a save
while the inventory is not known or while casting (inferred until read
R49-45 b), a clear of a set or slot already empty, a belt item not carried
or not for the belt, and a second send within 250 ms (ours, inferred). Using
the belt is described below.

Swapping to a set (`GameCommand::SwapBandolier`, under its own
`Capability::BandolierSwap`) is built and tested from source (6.566) and
offered by no server type until its local live rows pass and read
R49-45 d confirms the program's order; P99 never. It sends `OP_Bandolier`
action 2. Stock `EQEmu` then moves up to eight items (`SetBandolier`,
`MoveItemToInventory`) and sends no item update, so the session works out
every move (`bandolier::swap`) and tells the host the whole result as
`WorldEvent::Inventory(InventoryUpdate::Prediction(..))`, settled by the
ordinary refusal window. A host may send a swap from a click, a key, a
social or a script, since a script may do whatever the player can. The swap
is refused, with nothing sent (`BandolierRefused`), when any move can't be
worked out exactly: an inventory not known or with a move in flight or
custody unknown, anything on the cursor, an item record lacking what the
moves read, a return that would reach the cursor (ours: `EQEmu` pushes it
there), an empty set, or nothing to swap; and while stunned, dead,
trading, at a merchant, with a world container or a trader's window open,
casting, or within 250 ms of the last bandolier send (inferred until read
R49-45 b). `EQEmu`'s `HasItem` also matches an augment socketed in an
item, so a set naming an id in any worn or carried item's
`InventoryItem::augments` is refused, and so is a pack item that is not a
bag but has room or contents (6.570 G3). **On `EQEmu` the server
confirms no move, so the slots of any item move, the player's own or a
swap's, keep projected custody after they settle until the server next
sends them; a swap is refused after any item move until then.** Regaining
that custody is a gate for offering the swap, beside its live rows and
read R49-45 d.

Using a belt slot (`GameCommand::UseBeltItem(BeltUse)`, under its own
`Capability::UsingPotionBelt`) is built and tested from source (6.568) and
offered by no server type until its local live rows pass; P99 never. The
host names only the belt slot (0 to 3); the item is the one the session's
belt holds there, so a host can't use an item the belt does not name. The
session casts it from the first slot holding that item, worn 0 to 21 and
then packs 22 to 29 (inferred until read R49-45 c), by Titanium's
`OP_CastSpell` on casting slot 11 (`EQEmu`'s `CastingSlot::PotionBelt`).
**Stock `EQEmu` casts the item at whatever slot that names, the bank and the
cursor included; the session never names a bank, the cursor or a bag**,
since an item click is never sent from a bag either. Past the choice of
slot it is the whole item-use path of `GameCommand::UseItem`: the same
holds (`Casting`, `Inventory`), the same checks of charges, click effect,
level, song class and target, and the same answers
(`WorldEvent::ItemUseAction`, then the cast's own events). A refresh on
slot 11 answers the cast; one on the item slot (10) does not. It is refused
in the session's words, with nothing sent, when the belt is unknown or not
yet read, the belt slot is out of bounds or empty, the inventory is not
known or is stale, the item is not worn or in a pack, or the first slot
holding it has custody unknown (as after an `EQEmu` move until the server
sends it again). Hosts and scripts get no wider rule than for `UseItem`.

## Guild bank

Stock `EQEmu` keeps a guild bank only in the guild hall (zone 345), and
never sends a Titanium client the bank's list: `SendGuildBank` sends a
client before RoF the slots of its inventory type sizes, which are 0 for
Titanium. What the server does send, to every guild member in the zone, is
one slot at a time as an item arrives, leaves or changes there
(`SendGuildBankItemUpdate`). The session reads `OP_GuildBank` (0xb4ab) into
`WorldEvent::GuildBank` (6.563):

- `Shown(GuildBankEntry)`: a slot of the deposit or main area holds this
  item now, replacing what it held. `quantity` is the stack's count, or 1
  for an item with charges (the server sends 1). `usable_for_actor` is the
  server's `is_useable`, worked out for the player whose action caused the
  update, so for anyone else it is not this player's answer; nothing should
  act on it for this player.
- `Removed { area, slot, item_id }`: that slot is empty now.
- `DepositAnswered { taken }` and `Acknowledged`: the server's answers to a
  deposit and to the other requests. Neither names the request it answers.
  The session sends no guild bank request yet and acts on neither; in
  particular a taken deposit moves no cursor item until the deposit slice.
- `Cleared { deposit_slots, main_slots }`: read as its layout says; nothing
  on stock `EQEmu` sends it.
- `Banker(bool)`: the player's own banker flag from the profile's
  `guildbanker` word (byte 13056), which `EQEmu` sets only in the guild
  hall. A word other than 0 or 1 is an unreadable gap, never "banker". In
  the guild hall the flag also changes through `OP_SetGuildRank`
  (`GuildUpdate::MemberRank` with no guild); a host takes the latest of the
  two.

**The view is never complete.** A host's view of the bank is the slots it
has seen through `Shown` and `Removed` since it entered the guild hall; it
starts empty at every zone change. A slot never seen may hold anything.

`GuildBankEntry`'s `donor` and `permitted` are players' names. They are in
the event only: its `Debug` output leaves them out, and a host should not
log them either. Any other action, a known action at another length, an
area other than 0 or 1, and a `display` or `Fail` word other than 0 or 1
are unreadable gaps. P99's era has no guild bank; `EQMac` has no such
message, and RoF2's bank is another layout, owed.


## Bazaar search

Stock `EQEmu` on Titanium has a Bazaar, and TAKP and Quarm have the
Mac-era one (below); P99's era has none, and RoF2's search is unread.
`Capability::BazaarSearching` is built and tested from each server's
source. TAKP and Quarm offer it from source, not live yet (6.580a stays
open, and the first use on either is attended); stock `EQEmu` offers it
once its local live row passes (6.550a). No buy, browse or trader mode is
sent: a later buy must send the listed price exactly, since stock `EQEmu`
charges the price the client sends.

`GameCommand::BazaarSearch { criteria }` sends `OP_BazaarSearch` (0x1ee9)
action 7 as `BazaarSearch_Struct`, 108 bytes. Each `BazaarCriteria` field
left `None` is sent as the value the server takes as open: all ones for
the class, race, stat, slot and item type, 0 for a price, 1 and 100 for the
levels (what the official client sends for an open criterion is owed to
read R49-42a). The trader is a spawn id, 0 for all. The name is at most 63
bytes of printable ASCII; prices are whole platinum from 1 (the server
counts them in thousands of copper); levels are 1 to 100; each least must
not pass its most. `GameCommand::BazaarWelcome` sends action 9, padded to
20 bytes (inferred). Each is refused with `WorldEvent::BazaarRefused` and
nothing is sent outside the Bazaar (zone 151; `EQEmu` searches only the
player's own zone and instance, so elsewhere it finds nothing, and where
the original lets a player search is owed to R49-42), while another is in
flight (until its answer, a zone change, a new admission or 10 s), within
2 s of the last (ours, inferred), or when the criteria are out of bounds.

The server's answers come as `WorldEvent::Bazaar`: `Results` with each
`BazaarListing` (the trader's spawn, item id, serial number, name and the
count the server adds as `"Name(count)"`, `None` where the field cut it
off, the price in copper and the searched stat), then `SearchDone`, which
alone answers a search that found nothing; and `Welcome { traders, items }`.
The host names traders from its own spawn list. Results that are not whole
96-byte rows, more than 1000 rows, a row of another action or of a spawn past
16 bits, and an action no server is known to send are gaps; an answer
nobody asked for is withheld and a gap too. The bazaar's and traders'
gaps keep no sample, only opcode and length. The parcel delivery costs
`EQEmu` sends after results (action 29) are read and dropped: only RoF and
later have parcels.

### On TAKP and Quarm

TAKP's and Quarm's servers share `Client::Handle_OP_BazaarSearch` and
`Client::SendBazaarResults` (EQMacEmu `zone/client_packet.cpp`,
`zone/trading.cpp`), and the same commands send their records on
`OP_BazaarSearch` (0x1142). The search is `BazaarSearch_Struct`, 88 bytes:
action 7, then the trader, class, race, stat, slot and item type as 16-bit
fields, the name, and the least and most price in whole platinum (0 for
no bound). The server reads class, race and slot as bit positions in the
item's masks, counted from 1 for the class and race (the race is its
mask bit, Human 1 to Iksar 13) and from 0 for the slot, all ones as any, the stat as 0 to 20, and the
item type with its own special values (0, 31, 46 to 49). Its record has no
level bound, so a search with either level is refused, as is a criterion
past 16 bits or equal to all ones. The welcome request is action 9 in 20
bytes. The handler drops any other size. Neither server refuses a search
from outside the Bazaar: the handler builds a "Bazaar Search hack" note
when the zone is not 151 (`Zones::BAZAAR`) and answers all the same. The
session sends one only in zone 151, our own stricter rule, taken because
the servers flag it; the same one in flight, 10 s wait and 2 s floor hold.

Each result comes in a packet of its own, a 76-byte row, and the end after
the last (20 bytes, action 12). So the host hears one `Results` with one
`BazaarListing` per row, then `SearchDone`. A Mac row's listing has
`serial` and `stat` `None`, since the row carries neither; `trader` is
`None` where the server sends 0, a trader not in the zone; `listings` is
how many listings the server grouped in the row; the name's `" (N)"`
gives `count`, as on Titanium. **The item id is as the server sent it:
`SendBazaarResults` stores it through a 16-bit signed field, so an id past
32767 arrives wrapped and can't be trusted as an id** (nor the count, which
the server picks by that id's stacking). Rows past 1000 in one search are
withheld and a gap. When a search reaches `Bazaar:MaxSearchResults`, the
server's yellow line comes as ordinary chat.

## Browsing a trader

`Capability::BazaarBrowsing` is built and tested from stock `EQEmu`'s
source and offered by no server type until its local live row passes
(6.558a). Nothing is bought.

`GameCommand::BrowseTrader { trader }` sends `OP_TraderShop` (0x35e8) as
Titanium's `TraderClick_Struct` `{trader, 0, 0, 0}`, 16 bytes. It is
refused with `WorldEvent::BazaarRefused`, nothing sent, outside the Bazaar;
for a spawn that is not a player the session saw turn their trader sign on
(`OP_BecomeTrader`) or a search answer named as a trader since the zone-in,
and has not seen leave; beyond the generation's merchant reach, or 20 in
3-D where that is unread (inferred, R49-42 d; `EQEmu` checks no distance);
while a browse or a merchant's window is open, or a merchant's opening
waits for its answer; while another click waits (10 s); and within 1 s of
the last click (ours, inferred). A merchant may
not open while a browse is open or a click waits. `EQEmu` does not check
that the spawn clicked is a trader, and records the player as their one
customer until the browse ends, which is why the session clicks only known
traders and ends every browse it closes.

The server's admission comes as `BazaarEvent::Admitted { trader, approved }`
(a refusal's line is chat). After an approval, the trader's items come as
`BazaarEvent::Listed(TraderListing)`: the trader, the item's serial number
(the seventh field of the merchant item header), the price in copper, the
count and the item. A listing of a serial already listed replaces it (a
price cut or a re-list). `BazaarEvent::Delisted { trader, serial }` takes
one out (`OP_TraderDelItem`, or a price of 0). Outside a browse these
packets stay the merchant's, unchanged. `GameCommand::EndBrowse` sends
`OP_Trader` (0x524e) action 4 as `Trader_ShowItems_Struct`, 20 bytes, which
the server does not answer, and `BazaarEvent::BrowseEnded { trader }`
follows at once. A zone offer, answer or handoff also ends an open browse
with our end, sent while this zone's connection still carries it, since a
refused zone change keeps the player as the trader's customer; a click in
flight then stops waiting. A browse ends without our end when the server's
`OP_ShopEnd` says the trader left trader mode (our end would free the
trader's next customer), when their sign goes off or they despawn, and at a
new admission (the server forgets the customer with the zone, inferred). A
list past 80 items or an item that cannot be read is a gap and ends the
browse with our end, so no half list is kept. An admission no click asked
for is withheld and a gap; an approval that comes after its click stopped
waiting is ended at once. After either approval, the run of merchant item
packets that follows is that trader's list: it is withheld, and told once
as a gap.

Other players' signs come as `BazaarEvent::TraderSign { spawn, on }`; their
name is not read. Titanium's spawn records carry no trader flag, so a
trader who began before the player zoned in is known only from a search
answer until their sign changes (inferred; how the program knows them is
owed to R49-42 d). The trader rows' gaps keep no sample.

## Task journal

Stock `EQEmu` sessions (Titanium) read the task journal the zone sends
(gap list section 5, recording checklist 6.546, from `EQEmu`'s source and
not yet checked live). P99, TAKP, Quarm and RoF2 keep none; there is no
capability and nothing is sent. The host hears, each with the admission's
`session_id`:

- `WorldEvent::TaskDescribed { description }`: an entry's `TaskDescription`
  in its `TaskSlot` (the one `TaskKind::Task` slot 0, or a `TaskKind::Quest`
  slot 0 to 18), with its title, the server's open-journal flag, duration,
  duration code and start time as sent (the session does no clock
  arithmetic), description and `Rewards`. A description of another task in
  a slot forgets that slot's earlier steps.
- `WorldEvent::TaskStepUpdated { activity }`: one step, `Step::Hidden` or
  `Step::Shown(Objective)` with its goal and count, for an entry the
  session holds. `EQEmu` sends a description before its steps every time,
  so a step for a slot holding no entry of its task is stale: the host
  does not hear it, and it goes to the gap log by opcode and length.
- `WorldEvent::TaskConcluded { completion }`: a step ended
  (`stage_completed`), or the whole task completed or failed. `EQEmu` then
  sends the entry again (a step) or takes it out (the task).
- `WorldEvent::TaskRemoved { at }`: the server took the entry out
  (`OP_CancelTask`), the only way an entry leaves the journal here.
- `WorldEvent::CompletedTasks { list }`: the tasks finished, at most 50,
  replacing the earlier list.

What the zone sends before it admits the player is told at the admission,
the task first, then the quests by slot, each description before its
steps, then the finished list. Up to 64 stale messages heard before the
admission go to the gap log there; past that only their count is told, as
one diagnostic. Each zone starts empty and sends the whole journal again. A message that is short, overruns, has a string without its
end or too long, counts past what it carries, names a shared task (kind 1,
after Titanium), a slot past the journal or a step past 20 is refused
whole, never cut, and goes to the gap log by opcode and length only: no
journal message's gap keeps a sample, and no diagnostic or error carries a
task's words. Inferred until read R49-38: the official program's use of
each field, and whether it clears its journal at zone-in.

### Offers and requests

The same sessions read an NPC's offer and a finished task's steps
(checklist 6.555, from `EQEmu`'s source and not yet checked live), each
heard only after the admission, its words in the event alone:

- `WorldEvent::TasksOffered { offers }`: `TaskOffers` from `OP_TaskSelectWindow`,
  1 to 40 `OfferedTask`s from the giver's spawn, with the steps the window
  shows. It replaces any earlier offer, which stands until the player takes
  one or declines, the next offer, or the zone's end.
- `WorldEvent::TaskHistory { history }`: a finished task's done steps, as
  asked for.

`Capability::Tasks` covers five commands. It is offered only on a local
stock `EQEmu` session until its live row passes, because a wrong
acceptance can bring down a zone other players share. Each refusal is
`WorldEvent::TaskRefused { reason }`, and nothing is sent:

- `GameCommand::AcceptTask { task_id }` sends `OP_AcceptNewTask` (12 bytes:
  the kind as offered, the task and the giver) only for a task in the
  current offer, not already held, while the giver is an NPC spawn the
  session holds. `EQEmu`'s `CanAcceptNewTask` dereferences the giver
  without a null check, so a giver gone from the zone is refused here; one
  that leaves between this check and the server's read can't be guarded.
  One send ends the offer, as `EQEmu` clears its own.
- `GameCommand::DeclineTasks` ends the offer and sends nothing, since
  `EQEmu` ignores an acceptance of task 0 (inferred until read R49-38).
- `GameCommand::CancelTask { at }` sends `OP_CancelTask` (8 bytes) only for
  a slot that exists and holds an entry, once until the server's removal or
  the zone's end. `EQEmu`'s `RemoveTask` doesn't check the slot, so a quest
  slot past 18 or a second cancel would corrupt its count of quests.
- `GameCommand::AskTaskHistory { index }` sends `OP_TaskHistoryRequest`
  for an index in the last finished list. `GameCommand::ListTaskTimers`
  sends `/tasktimers` with an empty body, which `EQEmu` doesn't read (the
  program's body is inferred until read R49-38); the server answers in
  chat. Each is paced at one per second, this library's floor (inferred).

A host may send `AcceptTask` and `CancelTask` from a click, a social, a
hotbutton or a script, since a script may do whatever the player can. A
cancel loses the entry's progress, so a host confirms a click first.

## Resurrection and translocate consent

A live session exposes `WorldEvent::Resurrection` and
`WorldEvent::TranslocateOffer` with `confirmations::ConsentOffer<T>` where
its server policy supports the operation. Read the decoded fields from
`offer`. Copy `id` into `AnswerResurrection::offer_id` or
`AnswerTranslocate::offer_id` and use the admission from the same token as
`session_id`. A fresh admission can reuse a sequence. The ID combines its
admission, question kind and sequence and has no wire representation.

An identical pending offer keeps its original token and deadline. A distinct
offer of the same kind replaces it. On P99, an exact packet already issued
as a question is withheld for the rest of that admission, including after a
replacement, answer, expiry or interrupted transfer. The packet has no
established discriminator for an identical new offer. Each kind retains
up to 1,024 identities and withholds new questions after that limit. A stale
or repeated answer produces a refusal naming the attempted token and leaves
a replacement untouched.
Death, an outstanding zone transfer and logout invalidate old consent.
The in-process `expires_at` is a monotonic local deadline omitted from
serialization. The Titanium EQEmu and P99 policies use a five-minute local answer
window and submits a decline when it elapses during the same live admission.
Titanium `eqgame.exe` sets the timer at 0x4561bd (translocate) and 0x459b61
(resurrection). Expiry at 0x547f0a passes dismissal result 2 at 0x547eff.
The translocate and resurrection callbacks send a decline at 0x499ef3
and 0x499f3a, respectively. TAKP translocate retains its one-hour
window (6.253). A server may apply its own timing rule.

`ConsentClosed` distinguishes a submitted answer, a local expiry with a
submitted decline, and a lifecycle interruption. Movement and recovery
follow the ordinary server messages and admission. EQEmu translocate uses
the server's pending destination and resurrection echoes the offered packet with
the answer selected. P99 uses the same offer-bound consent controls. An
exact named translocate completion can enter the regular zone transfer
path after a matching Yes during the original offer lifetime. Bind spell
consent follows explicit server-directed moves. A bind completion alone
does not establish the current bind coordinates.

Raw wire readers should use `message::titanium` or `message::eqmac`
for consent records. The corresponding `Message::Resurrection` or
`Message::TranslocateOffer` carries decoded facts without a session
token. `Message::TranslocateCompleted` carries a Titanium continuation for
session correlation. `world::titanium_update` leaves consent to the session.
TAKP's distinct server move still enters the existing zone-transfer path.

See EQEmu's [resurrection offer routing](https://github.com/EQEmu/EQEmu/blob/master/zone/worldserver.cpp#L865-L893),
[resurrection processing](https://github.com/EQEmu/EQEmu/blob/master/zone/client_process.cpp#L971-L1055)
and [translocate processing](https://github.com/EQEmu/EQEmu/blob/master/zone/client_packet.cpp#L14888-L14952).

## Boat acquire/release

`GameCommand::BoatControl` is offered by `Capability::BoatControl` on EQEmu.
`boats::Request::Acquire` binds the current floor report and accepted rider
position; `Release` withdraws local control; `Support` refreshes or withdraws
the floor link. `WorldEvent::BoatControl` is only a decoded packet.
`WorldEvent::BoatControlState` carries the admission-scoped correlated state.
No state enables vessel position requests. An `Acquire` for the incarnation
already being taken sends the same take again and changes nothing else: the
pending request, its deadline and its fence stay the first one's, one echo
grants, and later echoes change nothing (6.533, inferred from read R35). A
take for another vessel while one is pending is still refused. See
[boat control](docs/boat-control.md) for echo ambiguity, timeout policy and
source/validation limits.

## Boat riding

`Capability::BoatRiding` is offered on EQEmu and P99. The session names the
boat underfoot to the server on its own, and zones with a boat when the
server's vehicle transfer names the saved one; hosts send nothing for either.
When a received profile names a vehicle, the session publishes
`WorldEvent::Arrival` and holds movement:

- `Arrival::Aboard` names the boat, the received report of the spawn found,
  the profile's position fields as received (`profile`) and the restore
  point in the boat's frame that the program's loader makes of them
  (`local`). The host stands the player at `local`,
  floor-sweeps from one unit above, retries at (0, 0, 25) in the frame, and
  sends one `GameCommand::ArrivalPlacement` with the result. It must send
  `None` when it found no floor, or when the floor its sweep found is not
  the named carrier's: the session cannot see the link, and the program
  takes only a floor that links the carrier. The session takes only a placement where that sweep could
  land, then publishes `Arrival::Placed`; any other answer, a second one, or
  one after death changes nothing or ends at the safe point. With no answer
  within 60 s the session uses the safe point.
- `Arrival::Waiting` means the boat was not found and the server was asked for
  it. The session ends the hold itself, by a transfer or after 60 s.
- `Arrival::SafePoint` gives the position the session placed the player at.

Moves sent while an arrival is held are refused. See
[boat riding](docs/boat-riding.md) for the wire and the inferred bounds.

## Server message gaps

`ClientEvent::Gap(Box<Gap>)` tells of a server message the session dropped
or could not read. `Gap` names its `stage` (login, world or zone), the
configured `server`, its `dialect`, the short `zone` (empty at the login
and the world, and at a zone until that zone has named itself), the `opcode`, the generation's table `name` where one lists it, and
the `kind`:

- `Unlisted`: no table row lists the opcode.
- `Unread { reason }`: listed, and left unread for the row's reason.
- `ReadElsewhere`: listed as read only at another stage.
- `PartlyRead { parts }`: read all but these parts, which work to come reads.
- `UnknownKind { kind }`: a kind of the message no server is known to send.
  A stage counts at most `MOST_KINDS` (4,096) kinds apart; the rest are
  counted under `MORE_KINDS`.
- `Unreadable { part, error }`: the decoders could not read it.
- `Skipped { reason }`: listed as holding nothing worth reading. Counted, but
  `GapKind::is_gap` is false.

Each record counts arrivals (`count`, `min_length`, `max_length`). The first
arrival of a gap (stage, opcode and `GapKind::key`) in a stage is told at once;
repeats are counted in the session and told at most every `GAP_INTERVAL`
(10 s) and when the stage ends. A host sums the counts. The first record of a
gap in an attempt carries `sample`, the first `SAMPLE_BYTES` (256) of the body
in hex, at the zone only; login and world records never carry one, and their
`Unreadable` errors are the decoders' fixed words. A zone sample may hold what
other players said or their names, so a host keeps it local. No record names
a host, an address, the account or a character. The diagnostic lines the
session wrote before are unchanged.
