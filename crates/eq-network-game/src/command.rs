//! Typed client actions and their dialect-specific packet encoding.

use crate::{chat, chat::OutboundChat, GameDialect};
use anyhow::Result;

/// Player posture values used by Titanium appearance updates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Posture {
    /// Standing and ready to move.
    Standing,
    /// Sitting for recovery or spellbook use.
    Sitting,
    /// Ducking, which also interrupts casting.
    Ducking,
}

/// The geometry-owning host's answer to the stand attempt at jump launch.
///
/// Both programs try to stand a ducking player through their clearance-tested
/// posture routine, then jump whether that stand succeeded or not (Titanium
/// 0x4aba14, Mac 0x53df6e; ducked move/jump read 2026-11-24). The session has
/// no geometry and never chooses this answer for the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum JumpPosture {
    /// Keep the current posture, including ducking after a refused stand.
    Keep,
    /// The host found room to stand: send that posture before the jump if
    /// the player is still ducking. Other postures are not changed here.
    StandIfDucked,
}

impl Posture {
    /// The appearance parameter for this persistent stance, the same in
    /// Titanium and `EQMac`.
    pub(crate) const fn appearance(self) -> u32 {
        match self {
            Self::Standing => 100,
            Self::Sitting => 110,
            Self::Ducking => 111,
        }
    }
}

/// An action requested by an application after zone admission.
///
/// Future movement, zoning, inventory, group, and character actions belong
/// here so application code never needs to carry raw opcodes or packet bytes.
///
/// Exhaustive on purpose: a new command is a decision for every table that
/// says what a command needs, which feature carries it out and how it is
/// refused, and each of those names it.
#[derive(Clone, Debug, PartialEq)]
pub enum GameCommand {
    /// Play on one world of the login server's current list.
    SelectServer {
        /// Identity supplied with the list.
        selection_id: u64,
        /// The world's place in the list, from zero.
        index: usize,
    },
    /// Enter one occupied slot from the current world-server character list.
    SelectCharacter {
        /// Identity supplied with the character list.
        selection_id: u64,
        /// Server slot to enter.
        slot: u8,
    },
    /// The server filters for the character entered next from the current
    /// character list: its first zone-in sends them, on every server type.
    /// A later return to the list forgets them again.
    ChooseServerFilters {
        /// Identity supplied with the character list.
        selection_id: u64,
        /// The character's filters.
        filters: crate::filters::ServerFilters,
    },
    /// Create a character from the current character-selection screen.
    CreateCharacter {
        /// Identity supplied with the character list.
        selection_id: u64,
        /// Requested name, race, class, deity, start zone and stats.
        character: crate::creation::NewCharacter,
    },
    /// Delete an explicitly named and confirmed identity from the current list.
    DeleteCharacter(crate::deletion::Request),
    /// Ask the world for a character name suggestion (Titanium).
    GenerateName {
        /// Identity supplied with the character list.
        selection_id: u64,
        /// Frontend identity, returned with the answer.
        request_id: u64,
        /// Player race.
        race: u32,
        /// Male (0) or female (1).
        gender: u32,
    },
    /// Exchange two book slots after checking both expected contents.
    SwapSpell {
        /// Current zone admission.
        session_id: u64,
        /// Populated source book slot.
        from: u16,
        /// Destination book slot, possibly empty.
        to: u16,
        /// Expected source spell.
        from_spell: u32,
        /// Expected destination spell or empty slot.
        to_spell: Option<u32>,
        /// Queued requests expire.
        created: std::time::Instant,
    },
    /// Activate the click effect on a current carried or equipped item.
    UseItem(crate::inventory::ItemUse),
    /// Use the item a potion belt slot names, as an item click from the
    /// first slot the player wears or carries it in (6.568).
    UseBeltItem(crate::bandolier::BeltUse),
    /// Acquire or release a supported vessel, or refresh its floor link.
    BoatControl {
        /// Current zone admission.
        session_id: u64,
        /// Source-backed control intent; steering is separate.
        request: crate::boats::Request,
        /// Queued input expires.
        created: std::time::Instant,
    },
    /// Stand the arriving player where the host's floor sweep found the
    /// vehicle the profile names ([`crate::boats::riders::Arrival::Aboard`]),
    /// or say it found no floor there, which sends them to the safe point.
    ArrivalPlacement {
        /// Current zone admission.
        session_id: u64,
        /// The swept world position on the vehicle, or None.
        position: Option<crate::world::Position>,
        /// The time the sweep posed the vehicle at.
        created: std::time::Instant,
    },
    /// Use an ordinary nearby door; no lockpicking or cursor-item action is implied.
    ClickDoor {
        /// Current zone admission.
        session_id: u64,
        /// ID from this zone's server-provided door table.
        door_id: u8,
        /// Where the host's click ray hit the door's model as drawn, in
        /// server axes; the session measures the reach from it, as the
        /// Titanium program does (0x49812e, 0x4906c5; read R16b, R16e).
        /// None where the host cast no ray (a key, a script): the reach then
        /// runs to the door's origin, which is stricter.
        point: Option<crate::world::Position>,
        /// Requests expire rather than surviving stalls or reconnects.
        created: std::time::Instant,
    },
    /// Pick up a nearby item lying on the ground; it arrives on the cursor.
    PickUp {
        /// Current zone admission.
        session_id: u64,
        /// Object from this zone's server-provided table.
        drop_id: u32,
        /// Requests expire rather than surviving stalls or reconnects.
        created: std::time::Instant,
    },
    /// Open a world container within reach, such as a forge; the server
    /// answers with what it holds, or that someone else is using it.
    OpenContainer {
        /// Current zone admission.
        session_id: u64,
        /// Object from this zone's server-provided table.
        drop_id: u32,
        /// Requests expire rather than surviving stalls or reconnects.
        created: std::time::Instant,
    },
    /// Close the world container open for the player; the server puts what
    /// it still holds back in the inventory.
    CloseContainer {
        /// Current zone admission.
        session_id: u64,
        /// The open object's ID.
        drop_id: u32,
        /// Host intent time, scoped to the current open of this object.
        created: std::time::Instant,
    },
    /// Request a transfer after entering a boundary in the local zone assets.
    ///
    /// A same-zone pad emits `WorldEvent::LocalPadCrossed`, not a new zone
    /// admission. Its host must perform the single arrival probe and submit
    /// `FinishLocalPad`, then observe `LocalPadCompleted` or `LocalPadRefused`.
    /// Until completion the session holds ordinary movement updates. Hosts
    /// without collision/body observations must not initiate local pads;
    /// if observation fails after crossing, offer disconnect/re-entry.
    CrossZoneLine {
        /// Current zone admission.
        session_id: u64,
        /// Asset-derived route; a reference is resolved by the network session.
        destination: crate::zoning::ZoneLineDestination,
        /// Most recently accepted position at the detected boundary.
        position: crate::world::Position,
        /// Stale crossings must not survive pauses or reconnects.
        created: std::time::Instant,
    },
    /// Complete only the pending local pad's first physics step. No update
    /// is forced; the existing movement gate retains its ordinary deadline.
    /// A refusal does not consume a still-current revision: correct the
    /// rejected observation and resubmit, or disconnect/re-enter if the host
    /// cannot supply it. A correction, death or rezone cancels the revision;
    /// do not retry it after that cancellation.
    FinishLocalPad {
        /// Current zone admission.
        session_id: u64,
        /// Revision supplied by that admission's `LocalPadCrossed` event.
        revision: u64,
        /// The single actual arrival-floor observation and body state.
        step: crate::zoning::PadArrivalStep,
        /// Host observation time; stale completions cannot move a new owner.
        created: std::time::Instant,
    },
    /// Scribe the scroll held on the cursor into an empty book slot.
    ScribeSpell {
        /// Current zone admission.
        session_id: u64,
        /// Inventory revision observed when selecting the scroll.
        revision: u64,
        /// Empty spellbook slot, zero through 399.
        slot: u16,
        /// Spell taught by the current cursor scroll.
        spell_id: u32,
        /// Request creation time.
        created: std::time::Instant,
    },
    /// Request deletion of a known book entry; applications should confirm this with the user.
    DeleteSpell {
        /// Current zone admission.
        session_id: u64,
        /// Book slot to delete, zero through 399.
        slot: u16,
        /// Expected spell; changed selections must not delete a replacement.
        spell_id: u32,
        /// Request creation time; queued deletions expire.
        created: std::time::Instant,
    },
    /// Clear a memorized gem, preserving the spell in the book.
    ForgetSpell {
        /// Current zone admission.
        session_id: u64,
        /// Gem to clear, zero through seven.
        gem: u8,
        /// Spell expected in that gem; stale selections are rejected.
        spell_id: u32,
        /// Request creation time.
        created: std::time::Instant,
    },
    /// Begin timed memorization from the admitted character's spellbook.
    MemorizeSpell {
        /// Current zone admission.
        session_id: u64,
        /// Destination gem, zero through seven.
        gem: u8,
        /// Spell already present in the server-provided book.
        spell_id: u32,
        /// Request creation time; queued actions expire.
        created: std::time::Instant,
    },
    /// Cancel an explicit beneficial buff, with current provider classification.
    CancelBuff(crate::buffs::cancel::Request),
    /// Request a manual stop of the current Bard song.
    StopSong {
        /// Current zone admission.
        session_id: u64,
        /// When the player requested the stop.
        created: std::time::Instant,
    },
    /// Titanium's `/stopcast` (0x46a75a, read R45): a Bard stops the song
    /// as [`GameCommand::StopSong`] does; anyone else sends the empty
    /// `OP_ManaChange`, and the cast ends when the server answers. The
    /// program refuses a mounted player, which the session doesn't decode:
    /// the host must refuse this for one (recording checklist 6.487).
    StopCasting {
        /// Current zone admission.
        session_id: u64,
        /// When the player asked.
        created: std::time::Instant,
    },
    /// Titanium's `/yell` (0x46791a, read R45): a call for help to the
    /// players nearby, with no target or posture test.
    YellForHelp {
        /// Current zone admission.
        session_id: u64,
        /// When the player asked.
        created: std::time::Instant,
    },
    /// Send a petition to the server's staff ([`crate::reports::petition`]).
    /// A person reads it, so the session sends one of a kind in any 10
    /// seconds, whether it was typed or came from a script.
    SendPetition {
        /// Current zone admission.
        session_id: u64,
        /// What the player asks.
        text: String,
        /// When the player asked.
        created: std::time::Instant,
    },
    /// Send a bug report ([`crate::reports::bug`]); the session names the
    /// reporter.
    ReportBug {
        /// Current zone admission.
        session_id: u64,
        /// The report.
        report: crate::reports::BugReport,
        /// When the player asked.
        created: std::time::Instant,
    },
    /// Send feedback ([`crate::reports::feedback`]); the session names the
    /// sender.
    SendFeedback {
        /// Current zone admission.
        session_id: u64,
        /// What the player says.
        text: String,
        /// Where the player stands, when the client knows; Titanium's
        /// feedback carries it.
        place: Option<crate::reports::BugPlace>,
        /// When the player asked.
        created: std::time::Instant,
    },
    /// Cast a memorized spell; the server decides completion and resource use.
    CastSpell {
        /// Current zone admission.
        session_id: u64,
        /// Zero-based memorized slot, 0 through 7.
        gem: u8,
        /// Spell currently occupying the slot.
        spell_id: u32,
        /// Explicit target, scoped to this zone.
        target_id: u16,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Change the active player's posture.
    SetPosture {
        /// Current zone admission.
        session_id: u64,
        /// Own spawn identifier from this admission.
        spawn_id: u16,
        /// Requested posture.
        posture: Posture,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// The host's statement that a spell with the feign effect (74) landed
    /// on the player, which the generation's program lies them down for.
    /// Only the host holds the spell data, so only it can tell. Send it only
    /// for a [`crate::buffs::SpellEffect`] on the player's own spawn that
    /// [took hold](crate::buffs::SpellEffect::took_hold) and whose spell
    /// carries effect 74, where [`crate::GameDialect::spell_feign_lies`]
    /// holds.
    /// The session does the posture alone, as Titanium's landing does (read
    /// R21: 0x414913's effect loop calls 0x40db88, the posture routine with
    /// 115): one lying report, or none when the player already lies, and
    /// the movement in hand stopped; no feign request and no auto attack
    /// change. With [`crate::movement::PostureOptions::feign_standing`] on,
    /// it sends nothing, owes the next stand and reports the feign held
    /// ([`crate::world::WorldEvent::FeignHeld`]). Another spawn's id is
    /// refused.
    FeignLanded {
        /// Current zone admission.
        session_id: u64,
        /// Own spawn identifier from this admission.
        spawn_id: u16,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Move an inventory item using the admitted session state.
    MoveInventory(crate::inventory::InventoryMove),
    /// Explicitly destroy the confirmed whole cursor item or stack.
    DestroyItem(Box<crate::inventory::InventoryDestroy>),
    /// Drop the exact current ordinary cursor item into the zone.
    DropItem(Box<crate::inventory::InventoryDrop>),
    /// Send one chat message in the common tongue.
    SendChat(OutboundChat),
    /// Send chat only in the named admission, in the tongue the player
    /// speaks. This adds a local queue guard, with the same wire message
    /// and timing as ordinary chat.
    SendChatInAdmission {
        /// The originating zone admission; a later visit must refuse it.
        session_id: u64,
        /// The ordinary message, including any local fixture command.
        message: OutboundChat,
        /// The tongue it is spoken in ([`chat::Tongue::COMMON`] for none
        /// chosen).
        tongue: chat::Tongue,
    },
    /// Remove the selected NPC in a network-constrained local `EQEmu` fixture.
    DepopLocalNpc {
        /// Current zone admission.
        session_id: u64,
        /// The available target and its reconciled selection revision.
        target: crate::targets::TargetState,
        /// When the host made the request.
        created: std::time::Instant,
    },
    /// Inspect a preserved item link in the active zone session.
    InspectItem {
        /// Connection identifier from zone admission.
        session_id: u64,
        /// Exact hexadecimal link header received in chat.
        link_body: String,
    },
    /// Ask the server how a visible entity regards this character.
    Consider {
        /// Current zone admission.
        session_id: u64,
        /// Own spawn identifier from this admission.
        own_id: u16,
        /// Visible entity to consider.
        target_id: u16,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Sit-and-wait logout to character selection, abandoned by standing or moving.
    Camp {
        /// Current zone admission.
        session_id: u64,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Report that the player bled out: the server left their death to the
    /// client, and their HP, with what equipped items add, reached its
    /// threshold ([`crate::world::WorldEvent::DeathThreshold`]) on the
    /// server's report, with no death named for them. A host sends it once;
    /// the player is then dead, as if the server had said so.
    BledOut {
        /// Current zone admission.
        session_id: u64,
        /// Reject delayed reports instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Open a nearby corpse for looting.
    Loot {
        /// Current zone admission.
        session_id: u64,
        /// Corpse entity.
        corpse_id: u16,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Take one item from the open corpse.
    LootItem {
        /// Current zone admission.
        session_id: u64,
        /// Corpse entity.
        corpse_id: u16,
        /// Own spawn identifier from this admission.
        own_id: u16,
        /// The item's place on the corpse, from 0, as the loot listed it.
        place: u16,
        /// Place directly into the inventory instead of on the cursor.
        auto: bool,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Close the loot window.
    EndLoot {
        /// Current zone admission.
        session_id: u64,
        /// Corpse entity.
        corpse_id: u16,
    },
    /// Open or close a merchant window.
    Shop {
        /// Current zone admission.
        session_id: u64,
        /// Merchant entity.
        merchant_id: u16,
        /// Own spawn identifier from this admission.
        own_id: u16,
        /// True opens, false closes.
        open: bool,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Buy units from a merchant slot; the server sets the price.
    Buy {
        /// Current zone admission.
        session_id: u64,
        /// Merchant entity.
        merchant_id: u16,
        /// Own spawn identifier from this admission.
        own_id: u16,
        /// Merchant list slot.
        slot: u32,
        /// Units to buy.
        quantity: u32,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Sell units from an inventory slot; the server sets the price.
    Sell {
        /// Current zone admission.
        session_id: u64,
        /// Merchant entity.
        merchant_id: u16,
        /// Own spawn identifier from this admission, which the `EQMac`
        /// program names in its sale.
        own_id: u16,
        /// Inventory slot.
        slot: i32,
        /// Units to sell.
        quantity: u32,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Ask a nearby character to trade; their answer opens the window: the
    /// give window for an NPC, the trade window for a player.
    OfferTrade {
        /// Current zone admission.
        session_id: u64,
        /// The character to trade with.
        with_id: u16,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Click Give or Trade in the open window.
    AcceptTrade {
        /// Current zone admission.
        session_id: u64,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Close the give or trade window; what it held comes back.
    CancelTrade {
        /// Current zone admission.
        session_id: u64,
    },
    /// Move coins between the purse, the cursor, the bank and a trade window.
    MoveCoins {
        /// Current zone admission.
        session_id: u64,
        /// The move.
        transfer: crate::money::CoinTransfer,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Destroy coins of one kind from the cursor.
    DestroyCoins {
        /// Current zone admission.
        session_id: u64,
        /// What is destroyed.
        destroy: crate::money::CoinDestroy,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Announce a jump the client is simulating; only sessions that accept falls
    /// send it.
    Jump {
        /// Current zone admission.
        session_id: u64,
        /// The host's clearance-tested posture decision for this launch.
        posture: JumpPosture,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Report damage from a completed fall, as the host worked it
    /// out. The server lowers the amount by its own reductions, such as the
    /// fall damage reductions of spells, items and AAs, so the host leaves
    /// those out. It never expires, since the damage was done.
    EnvironmentalDamage {
        /// Current zone admission.
        session_id: u64,
        /// What did it.
        hazard: crate::hazards::Hazard,
        /// How much, before the server's own reductions; none where Safe
        /// Fall took it all.
        amount: u32,
        /// Whether the player's Safe Fall skill lessened a fall, which both
        /// programs announce before the damage, even when nothing is left of
        /// it ([`crate::hazards::safe_fall`]).
        safe_fall: bool,
    },
    /// Report a fresh liquid tick from the current worker observation.
    LiquidDamage {
        /// Current zone admission.
        session_id: u64,
        /// Which local liquid calculation produced the amount.
        hazard: crate::hazards::LiquidHazard,
        /// Positive damage before server reductions.
        amount: u32,
        /// Session view used to calculate this tick.
        observation: crate::hazards::LiquidObservation,
    },
    /// Eat or drink the item in a slot by hand. The session eats and drinks
    /// on its own when the player turns hungry or thirsty.
    Consume {
        /// Current zone admission.
        session_id: u64,
        /// The item's slot.
        slot: crate::inventory::InventorySlot,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Send a command to the chat server's channels, as text it takes
    /// ([`crate::chat_server::command`]): `join a, b`, `leave a`, `list`,
    /// `2 hello` and the rest. Only while the link is up
    /// ([`crate::chat_server::ChannelLink`]); recording checklist 6.509.
    ChatChannel {
        /// Current zone admission.
        session_id: u64,
        /// The command, as the chat server takes it.
        text: String,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Put the poison in a slot on the weapon, as a right-click on it does
    /// ([`crate::poisons`]; read R49-1, recording checklist 6.493): the
    /// session sits a standing player, waits 10 s seated, then sends it.
    /// [`crate::world::WorldEvent::PoisonStarted`] says it began;
    /// [`crate::world::WorldEvent::PoisonInterrupted`] that a posture change
    /// stopped it. A command while one is under way is ignored, as the
    /// program ignores it.
    ApplyPoison {
        /// Current zone admission.
        session_id: u64,
        /// The poison's slot.
        slot: crate::inventory::InventorySlot,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Effect 234's total for the player, in ms, as the host sums it: the
    /// effect's value from every active buff at its caster's level, type-4
    /// records negated, and from every AA the player's class may use at
    /// their level (read R49-1f; 0x41002f and 0x40feaa). Worn items do not
    /// count. `None` when the host cannot resolve something that could
    /// carry the effect, such as a buff whose spell it does not know.
    ///
    /// The host's own report, posted whenever the total changes; the
    /// Apply Poison wait reads the latest one on each tick (checklist
    /// 6.493b). Each zone session starts untold, which waits 10 s.
    PoisonWait {
        /// Current zone admission.
        session_id: u64,
        /// The total in ms, or `None` when it cannot be summed.
        total_ms: Option<i64>,
    },
    /// What the session may eat and drink on its own from now on. Each zone
    /// session starts with the client's configured choice, so a host that
    /// lets the player change it says so again after each admission.
    AutoEat {
        /// Current zone admission.
        session_id: u64,
        /// What the session may eat and drink on its own.
        auto_eat: crate::food::AutoEat,
    },
    /// Explicit movement departures. Defaults preserve ordinary movement.
    SetMovementOptions {
        /// Current zone admission.
        session_id: u64,
        /// Caller identity returned in local application/refusal feedback.
        request_id: u64,
        /// Choices applied only after the movement prerequisites are known.
        options: crate::movement::MovementOptions,
        /// Caller stamp: subsequent same-frame movement keeps its elapsed budget.
        created: std::time::Instant,
    },
    /// Change the automatic sitting and local preparation delay for memorization.
    SetBookOptions {
        /// Current zone admission.
        session_id: u64,
        /// Memorize without automatic sitting or local delay; scribing is unchanged.
        memorize_without_sitting: bool,
    },
    /// Send the ordinary logout without the camp preparation/countdown.
    /// Server acceptance is not implied; the ordinary reply lifecycle applies.
    ExitAtOnce {
        /// Current zone admission.
        session_id: u64,
        /// Reject expired requests rather than replaying an immediate logout.
        created: std::time::Instant,
    },
    /// Change the current character's server filters. Every later zone-in
    /// sends them; where the server type takes a change mid-zone
    /// ([`crate::world::Capability::ServerFiltersAtOnce`]) they are sent at
    /// once too. When the official client sends them again is unread
    /// (recording checklist 6.494c).
    SetServerFilters {
        /// Current zone admission.
        session_id: u64,
        /// The character's filters.
        filters: crate::filters::ServerFilters,
    },
    /// Enable or disable automatic P99 list presence replies for this
    /// character until returning to character selection. Disabling cancels
    /// an owed reply, including while dead or transferring zones.
    AnswerListPolls {
        /// Current zone admission.
        session_id: u64,
        /// Whether future presence polls may be answered automatically.
        enabled: bool,
    },
    /// Use an ability on the unchanged expected target, or on the player.
    UseAbility {
        /// Current zone admission.
        session_id: u64,
        /// The ability.
        ability: crate::abilities::Ability,
        /// Target state displayed when requested. Targeted actions require
        /// an available, unchanged session projection.
        target: Option<crate::targets::TargetState>,
        /// Whether the player stands in liquid, as the host's world shows
        /// it: the Titanium program refuses Fishing in water (read R15c).
        in_liquid: bool,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Let a player drag the player's corpses, or take it back: `/consent`
    /// and `/deny`.
    Consent {
        /// Current zone admission.
        session_id: u64,
        /// The player, or `group`, `raid` or `guild`.
        name: String,
        /// Given, or taken back.
        given: bool,
    },
    /// Summon a corpse lying close: `/corpse`.
    SummonCorpse {
        /// Current zone admission.
        session_id: u64,
        /// The corpse's spawn.
        spawn_id: u16,
    },
    /// Start dragging a corpse: `/corpsedrag`.
    DragCorpse {
        /// Current zone admission.
        session_id: u64,
        /// The corpse's spawn.
        spawn_id: u16,
    },
    /// Stop dragging a corpse, or every corpse: `/corpsedrop`.
    DropCorpse {
        /// Current zone admission.
        session_id: u64,
        /// The corpse's spawn; None for all of them.
        spawn_id: Option<u16>,
    },
    /// Command the player's pet: `/pet`, or the pet window's buttons.
    Pet {
        /// Current zone admission.
        session_id: u64,
        /// The command.
        command: crate::pets::PetCommand,
        /// Current owned pet when the order is made. A targeted Leader
        /// query can address another pet and does not require this.
        pet: Option<crate::pets::PetIdentity>,
        /// The authoritative target selection when the order is made.
        target: Option<crate::targets::TargetState>,
        /// When the host made the request.
        created: std::time::Instant,
    },
    /// Open training with a guildmaster, practice a skill there, or leave.
    Training {
        /// Current zone admission.
        session_id: u64,
        /// What the player asks.
        request: crate::training::TrainingRequest,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Read the book or note in an inventory slot.
    ReadItem {
        /// Current zone admission.
        session_id: u64,
        /// Worker revision at selection, for an intervening pending move.
        revision: u64,
        /// Exact readable item and container observed at selection.
        selection: Box<crate::books::ReadSelection>,
        /// Reject delayed selections rather than reading a replacement item.
        created: std::time::Instant,
    },
    /// Combine what a carried tradeskill container holds.
    Combine {
        /// Current zone admission.
        session_id: u64,
        /// The pack slot the container is in.
        container: crate::inventory::InventorySlot,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Accept or decline the identified resurrection offer.
    AnswerResurrection {
        /// Identity from the offer event.
        offer_id: crate::confirmations::ConsentId,
        /// Current zone admission.
        session_id: u64,
        /// True accepts.
        accept: bool,
    },
    /// Accept or decline the sacrifice offered last, which takes the
    /// player's life: only ever on the player's own choice.
    AnswerSacrifice {
        /// Current zone admission.
        session_id: u64,
        /// True accepts.
        accept: bool,
    },
    /// Accept or decline the identified translocate offer. A Yes requires
    /// the player's own choice.
    AnswerTranslocate {
        /// Identity from the offer event.
        offer_id: crate::confirmations::ConsentId,
        /// Current zone admission.
        session_id: u64,
        /// True accepts.
        accept: bool,
    },
    /// Accept or decline the identified challenge to a duel, on the
    /// player's own choice.
    AnswerDuel {
        /// Identity from the offer event.
        offer_id: crate::confirmations::ConsentId,
        /// Current zone admission.
        session_id: u64,
        /// True accepts.
        accept: bool,
    },
    /// Titanium's `/duel` at the player's target (R49-6h, 0x472a67): a
    /// challenge, or the acceptance of the target's own. The session tells
    /// what it did as [`crate::socials::DuelNews`], with the program's
    /// refusals for no player target, the player themselves, a duel on and
    /// another challenge waiting.
    ChallengeDuel {
        /// Current zone admission.
        session_id: u64,
        /// The player's target, or 0 for none.
        target: u16,
        /// When the player asked.
        created: std::time::Instant,
    },
    /// `EQMac`'s `/decline` (TAKP program read 11h, 0x4fc966): turns the
    /// waiting challenge down and tells the challenger. The session tells
    /// what it did as [`crate::socials::DuelNews`]. The Titanium program
    /// has no such command.
    DeclineDuel {
        /// Current zone admission.
        session_id: u64,
        /// When the player asked.
        created: std::time::Instant,
    },
    /// Choose a button on the exact popup that was displayed.
    RespondToPopup {
        /// Current zone admission.
        session_id: u64,
        /// The worker's local dialog serial.
        serial: u64,
        /// The explicit button action.
        action: crate::popups::PopupAction,
        /// When the choice was made.
        created: std::time::Instant,
    },
    /// Inspect the currently selected player, if that selection still holds.
    InspectPlayer {
        /// Current zone admission.
        session_id: u64,
        /// The worker's confirmed selection, including its incarnation revision.
        target: crate::targets::TargetState,
        /// When inspection was requested.
        created: std::time::Instant,
    },
    /// Inspect a player by their spawn, without selecting them: the
    /// Titanium program's right-click on a player, or on the player
    /// themself, sends its inspect request for the clicked spawn and never
    /// targets (read R49-16, R49-16e). The caller says the program's
    /// too-far line; the session still refuses a spawn out of reach.
    InspectSpawn {
        /// Current zone admission.
        session_id: u64,
        /// The clicked spawn: another visible player, or the player's own.
        spawn_id: u16,
        /// When inspection was requested.
        created: std::time::Instant,
    },
    /// Configure the known native inspect note for this character and admission.
    SetInspectNote {
        /// Current zone admission.
        session_id: u64,
        /// User-authored note. None leaves the note unknown and sends no automatic replies.
        note: Option<String>,
    },
    /// Drop the player's invisibility as the client generation's program
    /// does before a cast, a song or an item's click: the steps in order.
    /// It never expires, since the cast that follows needs it done.
    DropInvisibility {
        /// Current zone admission.
        session_id: u64,
        /// The steps, in the program's order.
        steps: Vec<crate::buffs::InvisibilityStep>,
    },
    /// Invite a player into the player's group, by name: `/invite`.
    InviteToGroup {
        /// Current zone admission.
        session_id: u64,
        /// Who is invited.
        name: String,
    },
    /// Ask to join the displayed invitation's group: `/follow`.
    FollowGroup {
        /// Current zone admission.
        session_id: u64,
        /// The inviter shown when the player chose to accept.
        inviter: String,
        /// Identity of that displayed invitation; None authorizes no answer.
        invitation_id: Option<crate::group::InvitationId>,
    },
    /// Decline the invitation waiting for an answer.
    DeclineGroup {
        /// Current zone admission.
        session_id: u64,
        /// The inviter shown when the player chose to decline.
        inviter: String,
        /// Identity of that displayed invitation; None authorizes no answer.
        invitation_id: Option<crate::group::InvitationId>,
    },
    /// Leave the group, or, as its leader, remove the targeted member or
    /// disband it, as the server decides; with an invitation waiting,
    /// decline it: `/disband`.
    Disband {
        /// Current zone admission.
        session_id: u64,
    },
    /// Say the player looks for a group, or no longer: `/lfg`. `on` sets
    /// it; None turns it the other way (checklist 6.515, the word forms
    /// pending R49-6i).
    LookForGroup {
        /// Current zone admission.
        session_id: u64,
        /// The state asked for, or None to turn it over.
        on: Option<bool>,
    },
    /// Turn away from the keyboard, or back: `/afk`.
    ToggleAway {
        /// Current zone admission.
        session_id: u64,
    },
    /// Turn anonymous, or open again: `/anonymous`.
    ToggleAnonymous {
        /// Current zone admission.
        session_id: u64,
    },
    /// Turn roleplaying, or open again: `/roleplay`.
    ToggleRoleplay {
        /// Current zone admission.
        session_id: u64,
    },
    /// Show or hide the player's helm to others: the Titanium client's
    /// option, sent on every change of it.
    ShowHelm {
        /// Current zone admission.
        session_id: u64,
        /// Whether the helm shows.
        show: bool,
    },
    /// Roll a die from the lowest to the highest number: `/random`.
    Random {
        /// Current zone admission.
        session_id: u64,
        /// The lowest number.
        low: u32,
        /// The highest.
        high: u32,
    },
    /// Ask the server for this last name: `/surname`. The case is left as
    /// given, since `EQEmu` and TAKP set it themselves; the server answers
    /// with [`crate::world::WorldEvent::SurnameAnswer`] or a line of its own.
    Surname {
        /// Current zone admission.
        session_id: u64,
        /// The surname: one word of printable ASCII, sent as typed.
        surname: String,
    },
    /// Ask which titles the player may choose; the server answers with
    /// [`crate::world::WorldEvent::TitleList`].
    RequestTitles {
        /// Current zone admission.
        session_id: u64,
    },
    /// Choose a title before the name, or a suffix after it, from the
    /// server's latest list; 0 clears it. The server tells the zone of the
    /// change ([`crate::world::WorldEvent::TitleChanged`]).
    SetTitle {
        /// Current zone admission.
        session_id: u64,
        /// The title's number from the list; 0 to clear.
        title_id: u32,
        /// Whether it is the suffix.
        suffix: bool,
    },
    /// Buy this AA rank: a definition the server's table offered this
    /// admission ([`crate::alternate_advancement::Action::Train`]). Spends
    /// points, which nothing gives back; the server answers a purchase with
    /// its own lines and the new table, and a refused one with nothing.
    TrainAlternateAdvancement {
        /// Current zone admission.
        session_id: u64,
        /// The rank's definition id.
        rank_id: u32,
    },
    /// Use an owned AA rank's ability, on a target or none
    /// ([`crate::alternate_advancement::Action::Activate`]); the server
    /// answers a reuse wait with its own line.
    ActivateAlternateAdvancement {
        /// Current zone admission.
        session_id: u64,
        /// The owned rank's definition id.
        rank_id: u32,
        /// The target's spawn, or None.
        target: Option<u16>,
    },
    /// The share of experience that goes to AAs, 0 (off) to 100
    /// ([`crate::alternate_advancement::Action::Split`]).
    SetAlternateAdvancementSplit {
        /// Current zone admission.
        session_id: u64,
        /// The share, 0 to 100.
        percentage: u8,
    },
    /// Search the Bazaar for traders' items; the server answers with
    /// [`crate::bazaar::BazaarEvent::Results`] when anything matched, then
    /// [`crate::bazaar::BazaarEvent::SearchDone`]. Only in the Bazaar, and
    /// one at a time.
    BazaarSearch {
        /// Current zone admission.
        session_id: u64,
        /// What to look for.
        criteria: crate::bazaar::BazaarCriteria,
    },
    /// Ask how many traders and items the Bazaar has; the server answers
    /// with [`crate::bazaar::BazaarEvent::Welcome`].
    BazaarWelcome {
        /// Current zone admission.
        session_id: u64,
    },
    /// Browse a trader's wares in the Bazaar: the server answers with
    /// [`crate::bazaar::BazaarEvent::Admitted`] and, when approved, their
    /// list as [`crate::bazaar::BazaarEvent::Listed`]. Only a player whose
    /// trader sign is on, within reach, with no merchant or browse open,
    /// one click at a time.
    BrowseTrader {
        /// Current zone admission.
        session_id: u64,
        /// The trader's spawn.
        trader: u16,
    },
    /// Save what is worn in the primary, secondary, range and ammo slots as
    /// bandolier set `set` (0 to 3) under `name` (1 to 31 printable
    /// bytes). The server answers nothing; the session tells its own copy
    /// of the sets as [`crate::world::WorldEvent::Bandoliers`].
    SaveBandolier {
        /// Current zone admission.
        session_id: u64,
        /// The set, 0 to 3.
        set: u8,
        /// Its name.
        name: String,
    },
    /// Clear bandolier set `set` (0 to 3).
    ClearBandolier {
        /// Current zone admission.
        session_id: u64,
        /// The set, 0 to 3.
        set: u8,
    },
    /// Swap to bandolier set `set` (0 to 3): the server puts the set's
    /// items in the weapon slots and returns what they held, telling the
    /// client nothing; the session predicts every move or refuses
    /// ([`crate::bandolier::swap()`]), whether from a click, a key, a social
    /// or a script.
    SwapBandolier {
        /// Current zone admission.
        session_id: u64,
        /// The set, 0 to 3.
        set: u8,
    },
    /// Put an item the player carries, a potion or one made for the belt,
    /// in potion belt slot `slot` (0 to 3). The session tells its own copy
    /// of the belt as [`crate::world::WorldEvent::PotionBelt`].
    SetPotionBelt {
        /// Current zone admission.
        session_id: u64,
        /// The belt slot, 0 to 3.
        slot: u8,
        /// The item's number.
        item_id: u32,
    },
    /// Empty potion belt slot `slot` (0 to 3).
    ClearPotionBelt {
        /// Current zone admission.
        session_id: u64,
        /// The belt slot, 0 to 3.
        slot: u8,
    },
    /// End the browse of a trader; the session tells
    /// [`crate::bazaar::BazaarEvent::BrowseEnded`] at once, as the server
    /// answers nothing.
    EndBrowse {
        /// Current zone admission.
        session_id: u64,
    },
    /// Emote, in the player's own words: `/emote`.
    Emote {
        /// Current zone admission.
        session_id: u64,
        /// What the player does, after their name.
        text: String,
    },
    /// A social emote such as `/smile`, its text exactly as Titanium's
    /// worker 0x49e0c1 builds it, leading space and all (read R47b). Sent as
    /// it is, with no local echo: the client prints the player's own line,
    /// and the server sends the others theirs.
    SocialEmote {
        /// Current zone admission.
        session_id: u64,
        /// The text after the player's name, as the program sends it.
        text: String,
    },
    /// The animation an emote command plays (Titanium 0x49e0c1, read R47),
    /// sent after its [`GameCommand::SocialEmote`]: the program sends it only
    /// while the player stands still, and the session sends nothing unless
    /// the player stands.
    EmoteAnimation {
        /// Current zone admission.
        session_id: u64,
        /// The animation's number.
        animation: u8,
        /// When the player asked.
        created: std::time::Instant,
    },
    /// Take the target of this spawn: `/assist`.
    Assist {
        /// Current zone admission.
        session_id: u64,
        /// Whose target to take.
        spawn_id: u16,
    },
    /// Invite a player into the player's raid, by name: `/raidinvite`.
    RaidInvite {
        /// Current zone admission.
        session_id: u64,
        /// Who is invited.
        name: String,
    },
    /// Join the raid of whoever invited the player last: `/raidaccept`.
    RaidAccept {
        /// Current zone admission.
        session_id: u64,
    },
    /// Decline the raid invitation waiting for an answer: `/raiddecline`.
    RaidDecline {
        /// Current zone admission.
        session_id: u64,
    },
    /// Leave the player's raid: `/raiddisband`.
    RaidLeave {
        /// Current zone admission.
        session_id: u64,
    },
    /// Lock the player's raid, so that its leader may move members between
    /// raid groups, or unlock it: the Raid window's Lock and Unlock.
    RaidLock {
        /// Current zone admission.
        session_id: u64,
        /// Lock (true) or unlock.
        locked: bool,
    },
    /// Move a member of the player's raid into a raid group, 0 to 11, or
    /// out of every group: the Raid window's group buttons.
    RaidMove {
        /// Current zone admission.
        session_id: u64,
        /// Who moves.
        name: String,
        /// Where to: a raid group, or none.
        group: Option<u8>,
    },
    /// Hand the lead of the player's raid to a member: `/makeraidleader`.
    RaidMakeLeader {
        /// Current zone admission.
        session_id: u64,
        /// The new leader.
        name: String,
    },
    /// Remove a member from the player's raid: the Raid window's Disband
    /// with a member chosen.
    RaidRemove {
        /// Current zone admission.
        session_id: u64,
        /// Who is removed; the player themself leaves.
        name: String,
    },
    /// Ask which of the player's friends are online: `/who all friends`.
    WhoFriends {
        /// Current zone admission.
        session_id: u64,
        /// The names on the player's friends list.
        names: Vec<String>,
    },
    /// Ask the banker nearest the player to change the purse's and the
    /// bank's coins into the fewest coins of the largest kinds: the bank
    /// window's Change button.
    ChangeCoins {
        /// Current zone admission.
        session_id: u64,
    },
    /// Tracking's window: ask for the list again, follow a spawn from it,
    /// or stop following (`/stoptracking`); see [`crate::tracking`].
    Track {
        /// Current zone admission.
        session_id: u64,
        /// What to do.
        action: crate::tracking::TrackAction,
    },
    /// Share coins from the purse with the player's group: `/split`.
    SplitCoins {
        /// Current zone admission.
        session_id: u64,
        /// The coins to share, which leave the purse at once.
        coins: crate::world::Coins,
    },
    /// Share the coins the player loots with their group, or stop:
    /// `/autosplit`.
    SetAutoSplit {
        /// Current zone admission.
        session_id: u64,
        /// Whether looted coins are shared.
        on: bool,
    },
    /// Ask for the guild's message of the day: `/getguildmotd`.
    RequestGuildMotd {
        /// Current zone admission.
        session_id: u64,
    },
    /// Answer a guild invitation that waits, by who sent it.
    AnswerGuildInvite {
        /// Current zone admission.
        session_id: u64,
        /// Who invited the player.
        inviter: String,
        /// The answer, as the official client gives it.
        answer: crate::guild::InviteAnswer,
    },
    /// Invite a player into the player's guild, as a member or, by its
    /// leader, as an officer: `/guildinvite`.
    GuildInvite {
        /// Current zone admission.
        session_id: u64,
        /// Who is invited.
        name: String,
        /// Whether the offer is an officer's rank.
        officer: bool,
    },
    /// Remove a member from the player's guild, or, naming the player,
    /// leave it: `/guildremove`.
    GuildRemove {
        /// Current zone admission.
        session_id: u64,
        /// Who is removed.
        name: String,
    },
    /// Set the guild's message of the day: `/guildmotd`.
    SetGuildMotd {
        /// Current zone admission.
        session_id: u64,
        /// The new message.
        motd: String,
    },
    /// Hand the lead of the player's guild to a member: `/guildleader`.
    GuildMakeLeader {
        /// Current zone admission.
        session_id: u64,
        /// The new leader.
        name: String,
    },
    /// Lower a guild member's rank, as the guild window's Demote button
    /// does; the Titanium client has no slash command for it.
    GuildDemote {
        /// Current zone admission.
        session_id: u64,
        /// Who is demoted.
        name: String,
    },
    /// Ask the world who is online: `/who all`.
    WhoAll {
        /// Current zone admission.
        session_id: u64,
        /// Which players to ask about.
        filter: crate::who::WhoFilter,
    },
    /// Start or stop melee auto-attack against the current server-side target.
    AutoAttack {
        /// Current zone admission.
        session_id: u64,
        /// Whether auto-attack should be on.
        enabled: bool,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Fire or throw once with the current ranged loadout.
    RangedAttack(crate::combat::ranged::RangedAttack),
    /// Use a discipline the server listed for the player (Titanium
    /// 0x4bb39d, read R48): the ordinary cast with the discipline slot. The
    /// host judges class, level and the spell's own refusals from its spell
    /// data; the session refuses a spell the server didn't list, one whose
    /// timer runs, and a timed self one while a discipline is active. On
    /// TAKP and Quarm the session sends the servers' number for the spell
    /// and class ([`crate::disciplines::eqmac`]) and refuses only a spell
    /// with none; the timer and target fields are unused there.
    UseDiscipline {
        /// Admission where the player selected the discipline.
        session_id: u64,
        /// The discipline's spell, which must be in the server's list.
        spell_id: u32,
        /// The reuse timer the spell names (its spell data's timer slot).
        timer: u32,
        /// The cast's target: the current target's spawn, or 0 with none,
        /// for a self spell too (read R48b a).
        target_id: u16,
        /// Whether the spell has a duration and targets its caster: only
        /// such a discipline is refused while one is active (0x4bb039,
        /// read R48b b).
        timed_self: bool,
        /// When the player requested it.
        created: std::time::Instant,
    },
    /// Titanium's `/sheathe` (0x465dea) or `/aggressive` (0x465e51): flip
    /// one bit of the player's own state word and tell the server (read
    /// R49-6n). No line; the server's echo changes how the player stands.
    ToggleStance {
        /// Current zone admission.
        session_id: u64,
        /// Which one.
        stance: crate::appearance::Stance,
        /// When the player asked.
        created: std::time::Instant,
    },
    /// Ask when a discipline may next be used: `EQMac`'s bare `/disc`
    /// ([`crate::disciplines::eqmac::REUSE_QUESTION`]). The server answers
    /// with a line; the session keeps no timer of its own.
    AskDisciplineReuse {
        /// Current zone admission.
        session_id: u64,
        /// When the player asked.
        created: std::time::Instant,
    },
    /// Cast a Paladin or Shadow Knight class ability at the selected target.
    UseKnightAbility {
        /// Admission in which the ability was chosen.
        session_id: u64,
        /// Which class ability was selected.
        ability: crate::knights::KnightAbility,
        /// The settled target selection, including an explicit self target.
        target: crate::targets::TargetState,
        /// When the player requested the action.
        created: std::time::Instant,
    },
    /// A Warrior's `/shield` of another player
    /// ([`crate::shielding`]). The server checks the level, class, recast,
    /// target and distance, and answers each case with a line of its own.
    Shield {
        /// Current zone admission.
        session_id: u64,
        /// The player to shield.
        spawn_id: u16,
        /// When the player asked.
        created: std::time::Instant,
    },
    /// Take a task the last offer holds ([`crate::tasks::offers`]). The
    /// giver must still be in the zone. A host may send it from a click, a
    /// social, a hotbutton or a script.
    AcceptTask {
        /// Current zone admission.
        session_id: u64,
        /// The offered task.
        task_id: u32,
        /// When the player chose it.
        created: std::time::Instant,
    },
    /// Close the last offer without taking a task.
    DeclineTasks {
        /// Current zone admission.
        session_id: u64,
    },
    /// Cancel a journal entry, losing its progress
    /// ([`crate::tasks::offers`]). A host asks the player first when it
    /// comes from a click; a script's own step asks nothing.
    CancelTask {
        /// Current zone admission.
        session_id: u64,
        /// The entry.
        at: crate::tasks::TaskSlot,
        /// When the player chose it.
        created: std::time::Instant,
    },
    /// Ask for the done steps of the finished task at this index of the
    /// last finished list ([`crate::tasks::offers`]).
    AskTaskHistory {
        /// Current zone admission.
        session_id: u64,
        /// The index in the last finished list.
        index: u32,
        /// When the player asked.
        created: std::time::Instant,
    },
    /// `/tasktimers`: the server answers with lines of its own.
    ListTaskTimers {
        /// Current zone admission.
        session_id: u64,
        /// When the player asked.
        created: std::time::Instant,
    },
    /// Select or clear a target in the current zone session, without attacking.
    SelectTarget {
        /// Connection identifier supplied by zone admission.
        session_id: u64,
        /// Host selection sequence; correlates results with pending intent.
        request_id: u64,
        /// None clears the target; zero is not a valid selected spawn.
        spawn_id: Option<u16>,
        /// For `/pet target`, the owned pet this choice was made for.
        /// Ordinary target choices leave this unset.
        expected_pet: Option<crate::pets::PetIdentity>,
    },
    /// Install independently measured motion values for this admission.
    /// Generations with a known speed rule reject this override and require
    /// [`Self::SpeedInputs`]. This is only for a generation whose rule is unread.
    ConfigureMotion {
        /// Current zone-admission identifier.
        session_id: u64,
        /// World-to-wire conversion measured for the current movement mode.
        calibration: crate::movement::MotionCalibration,
        /// Creation time prevents queued configuration from surviving a reset.
        created: std::time::Instant,
    },
    /// What the player's speeds depend on that the host works out itself:
    /// the movement rate of their buffs and what they carry. Until the first
    /// report the player cannot move on a server whose speeds the session
    /// works out itself.
    SpeedInputs {
        /// Current zone-admission identifier.
        session_id: u64,
        /// Echo the latest `WorldEvent::SpeedInputsRequired` revision.
        revision: u64,
        /// Opaque host identity for this logical report. Retries reuse it;
        /// changing the state or revision requires a fresh identity, including
        /// recovery to identical values after an unavailable report.
        report_id: u64,
        /// Complete inputs or explicit withdrawal, never guessed zeroes.
        state: crate::movement::SpeedInputState,
    },
    /// Read the worker's submitted-position counter without changing motion.
    MovementPacketSnapshot {
        /// Admission this observation must belong to.
        session_id: u64,
        /// Host-chosen correlation identity, not an admission or report stamp.
        request_id: u64,
    },
    /// How far the player's body climbs in one 50 ms program step, in world
    /// units, which the host works out from the player's model and the
    /// client generation's floor sweep: the session's movement guard lets a
    /// grounded sample rise that far in each step it spans
    /// ([`crate::movement::MovementGuard::set_climb`]). Until the host sends
    /// one, a grounded sample rises one riser.
    ClimbReach {
        /// Current zone-admission identifier.
        session_id: u64,
        /// The climb a step, in world units; a value that is not a positive
        /// number is ignored.
        per_step: f32,
    },
    /// The radius of each door model the host draws, at the door's size,
    /// which the session's lift carry measures its rider reach with
    /// ([`crate::doors::titanium::Lifts::set_radii`]). Each list replaces the
    /// last; a door it doesn't name, or one now drawing another model, keeps
    /// the program's reach for a door without a model.
    DoorRadii {
        /// Current zone-admission identifier.
        session_id: u64,
        /// The radii, by door.
        radii: Vec<crate::doors::DoorRadius>,
    },
    /// Propose fresh motion for the current zone session.
    Move(crate::movement::MovementRequest),
    /// An ordinary water step with the actual ordered integrator calls from
    /// an independently matched reset. Uncertain entry uses `Move` instead.
    SwimFrame {
        /// Genuine physics-end stamp and collision-resolved proposal.
        request: crate::movement::MovementRequest,
        /// Complete applied controls, never a path fitted to the endpoint.
        controls: crate::movement::SwimControls,
    },
    /// A host's completed ordinary Forward ground frame, before its next input.
    /// Position, current velocity and observed Forward input are separate facts.
    /// The ordinary guard, transaction, packing and report gate remain in force.
    /// A frame inside the minimum interval waits for a fresh observation;
    /// only admitted/refused frames publish `MotionSent`, never a false acceptance.
    GroundFrame {
        /// Genuine creation stamp, admission and collision-resolved position.
        request: crate::movement::MovementRequest,
        /// Current owned velocity and actually observed input, not interval travel.
        state: crate::movement::GroundState,
    },
}

impl GameCommand {
    /// The command variant's name, without any payload, for diagnostics.
    /// Exhaustive so new commands cannot silently lose their diagnostic identity.
    #[must_use]
    #[allow(
        clippy::too_many_lines,
        reason = "every kind of command named, so that a new one is a decision here"
    )]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::SelectServer { .. } => "SelectServer",
            Self::SelectCharacter { .. } => "SelectCharacter",
            Self::ChooseServerFilters { .. } => "ChooseServerFilters",
            Self::SetServerFilters { .. } => "SetServerFilters",
            Self::DeleteCharacter(_) => "DeleteCharacter",
            Self::CreateCharacter { .. } => "CreateCharacter",
            Self::GenerateName { .. } => "GenerateName",
            Self::SwapSpell { .. } => "SwapSpell",
            Self::UseItem(..) => "UseItem",
            Self::UseBeltItem(..) => "UseBeltItem",
            Self::ClickDoor { .. } => "ClickDoor",
            Self::BoatControl { .. } => "BoatControl",
            Self::ArrivalPlacement { .. } => "ArrivalPlacement",
            Self::PickUp { .. } => "PickUp",
            Self::OpenContainer { .. } => "OpenContainer",
            Self::CloseContainer { .. } => "CloseContainer",
            Self::CrossZoneLine { .. } => "CrossZoneLine",
            Self::FinishLocalPad { .. } => "FinishLocalPad",
            Self::ScribeSpell { .. } => "ScribeSpell",
            Self::DeleteSpell { .. } => "DeleteSpell",
            Self::ForgetSpell { .. } => "ForgetSpell",
            Self::MemorizeSpell { .. } => "MemorizeSpell",
            Self::CancelBuff(_) => "CancelBuff",
            Self::CastSpell { .. } => "CastSpell",
            Self::StopSong { .. } => "StopSong",
            Self::StopCasting { .. } => "StopCasting",
            Self::YellForHelp { .. } => "YellForHelp",
            Self::SendPetition { .. } => "SendPetition",
            Self::ReportBug { .. } => "ReportBug",
            Self::SendFeedback { .. } => "SendFeedback",
            Self::EmoteAnimation { .. } => "EmoteAnimation",
            Self::SocialEmote { .. } => "SocialEmote",
            Self::SetPosture { .. } => "SetPosture",
            Self::FeignLanded { .. } => "FeignLanded",
            Self::MoveInventory(..) => "MoveInventory",
            Self::DestroyItem(..) => "DestroyItem",
            Self::DropItem(..) => "DropItem",
            Self::SendChat(..) => "SendChat",
            Self::SendChatInAdmission { .. } => "SendChatInAdmission",
            Self::DepopLocalNpc { .. } => "DepopLocalNpc",
            Self::InspectItem { .. } => "InspectItem",
            Self::Consider { .. } => "Consider",
            Self::Camp { .. } => "Camp",
            Self::BledOut { .. } => "BledOut",
            Self::Loot { .. } => "Loot",
            Self::LootItem { .. } => "LootItem",
            Self::EndLoot { .. } => "EndLoot",
            Self::Shop { .. } => "Shop",
            Self::Buy { .. } => "Buy",
            Self::Sell { .. } => "Sell",
            Self::OfferTrade { .. } => "OfferTrade",
            Self::AcceptTrade { .. } => "AcceptTrade",
            Self::CancelTrade { .. } => "CancelTrade",
            Self::MoveCoins { .. } => "MoveCoins",
            Self::DestroyCoins { .. } => "DestroyCoins",
            Self::Jump { .. } => "Jump",
            Self::EnvironmentalDamage { .. } => "EnvironmentalDamage",
            Self::LiquidDamage { .. } => "LiquidDamage",
            Self::Consume { .. } => "Consume",
            Self::ApplyPoison { .. } => "ApplyPoison",
            Self::ChatChannel { .. } => "ChatChannel",
            Self::AutoEat { .. } => "AutoEat",
            Self::PoisonWait { .. } => "PoisonWait",
            Self::AnswerListPolls { .. } => "AnswerListPolls",
            Self::SetMovementOptions { .. } => "SetMovementOptions",
            Self::SetBookOptions { .. } => "SetBookOptions",
            Self::ExitAtOnce { .. } => "ExitAtOnce",
            Self::UseAbility { .. } => "UseAbility",
            Self::UseKnightAbility { .. } => "UseKnightAbility",
            Self::Shield { .. } => "Shield",
            Self::AskDisciplineReuse { .. } => "AskDisciplineReuse",
            Self::ToggleStance { .. } => "ToggleStance",
            Self::AcceptTask { .. } => "AcceptTask",
            Self::DeclineTasks { .. } => "DeclineTasks",
            Self::CancelTask { .. } => "CancelTask",
            Self::AskTaskHistory { .. } => "AskTaskHistory",
            Self::ListTaskTimers { .. } => "ListTaskTimers",
            Self::UseDiscipline { .. } => "UseDiscipline",
            Self::Consent { .. } => "Consent",
            Self::SummonCorpse { .. } => "SummonCorpse",
            Self::DragCorpse { .. } => "DragCorpse",
            Self::DropCorpse { .. } => "DropCorpse",
            Self::Pet { .. } => "Pet",
            Self::Training { .. } => "Training",
            Self::ReadItem { .. } => "ReadItem",
            Self::Combine { .. } => "Combine",
            Self::AnswerResurrection { .. } => "AnswerResurrection",
            Self::AnswerSacrifice { .. } => "AnswerSacrifice",
            Self::AnswerTranslocate { .. } => "AnswerTranslocate",
            Self::AnswerDuel { .. } => "AnswerDuel",
            Self::ChallengeDuel { .. } => "ChallengeDuel",
            Self::DeclineDuel { .. } => "DeclineDuel",
            Self::RespondToPopup { .. } => "RespondToPopup",
            Self::InspectPlayer { .. } => "InspectPlayer",
            Self::InspectSpawn { .. } => "InspectSpawn",
            Self::SetInspectNote { .. } => "SetInspectNote",
            Self::DropInvisibility { .. } => "DropInvisibility",
            Self::InviteToGroup { .. } => "InviteToGroup",
            Self::FollowGroup { .. } => "FollowGroup",
            Self::DeclineGroup { .. } => "DeclineGroup",
            Self::Disband { .. } => "Disband",
            Self::ToggleAway { .. } => "ToggleAway",
            Self::LookForGroup { .. } => "LookForGroup",
            Self::ToggleAnonymous { .. } => "ToggleAnonymous",
            Self::ToggleRoleplay { .. } => "ToggleRoleplay",
            Self::ShowHelm { .. } => "ShowHelm",
            Self::Random { .. } => "Random",
            Self::Surname { .. } => "Surname",
            Self::RequestTitles { .. } => "RequestTitles",
            Self::SetTitle { .. } => "SetTitle",
            Self::TrainAlternateAdvancement { .. } => "TrainAlternateAdvancement",
            Self::ActivateAlternateAdvancement { .. } => "ActivateAlternateAdvancement",
            Self::SetAlternateAdvancementSplit { .. } => "SetAlternateAdvancementSplit",
            Self::BazaarSearch { .. } => "BazaarSearch",
            Self::BazaarWelcome { .. } => "BazaarWelcome",
            Self::BrowseTrader { .. } => "BrowseTrader",
            Self::EndBrowse { .. } => "EndBrowse",
            Self::SaveBandolier { .. } => "SaveBandolier",
            Self::ClearBandolier { .. } => "ClearBandolier",
            Self::SwapBandolier { .. } => "SwapBandolier",
            Self::SetPotionBelt { .. } => "SetPotionBelt",
            Self::ClearPotionBelt { .. } => "ClearPotionBelt",
            Self::Emote { .. } => "Emote",
            Self::Assist { .. } => "Assist",
            Self::RaidInvite { .. } => "RaidInvite",
            Self::RaidAccept { .. } => "RaidAccept",
            Self::RaidDecline { .. } => "RaidDecline",
            Self::RaidLeave { .. } => "RaidLeave",
            Self::RaidLock { .. } => "RaidLock",
            Self::RaidMove { .. } => "RaidMove",
            Self::RaidMakeLeader { .. } => "RaidMakeLeader",
            Self::RaidRemove { .. } => "RaidRemove",
            Self::WhoAll { .. } => "WhoAll",
            Self::WhoFriends { .. } => "WhoFriends",
            Self::ChangeCoins { .. } => "ChangeCoins",
            Self::Track { .. } => "Track",
            Self::SplitCoins { .. } => "SplitCoins",
            Self::SetAutoSplit { .. } => "SetAutoSplit",
            Self::RequestGuildMotd { .. } => "RequestGuildMotd",
            Self::AnswerGuildInvite { .. } => "AnswerGuildInvite",
            Self::GuildInvite { .. } => "GuildInvite",
            Self::GuildRemove { .. } => "GuildRemove",
            Self::SetGuildMotd { .. } => "SetGuildMotd",
            Self::GuildMakeLeader { .. } => "GuildMakeLeader",
            Self::GuildDemote { .. } => "GuildDemote",
            Self::AutoAttack { .. } => "AutoAttack",
            Self::RangedAttack(_) => "RangedAttack",
            Self::SelectTarget { .. } => "SelectTarget",
            Self::ConfigureMotion { .. } => "ConfigureMotion",
            Self::SpeedInputs { .. } => "SpeedInputs",
            Self::ClimbReach { .. } => "ClimbReach",
            Self::MovementPacketSnapshot { .. } => "MovementPacketSnapshot",
            Self::DoorRadii { .. } => "DoorRadii",
            Self::Move(..) => "Move",
            Self::GroundFrame { .. } => "GroundFrame",
            Self::SwimFrame { .. } => "SwimFrame",
        }
    }

    /// The zone admission the command was made for, if it names one.
    #[must_use]
    #[allow(
        clippy::too_many_lines,
        reason = "every kind of command named, so that a new one is a decision here"
    )]
    pub const fn session_id(&self) -> Option<u64> {
        match self {
            Self::SelectServer { .. }
            | Self::SelectCharacter { .. }
            | Self::ChooseServerFilters { .. }
            | Self::GenerateName { .. }
            | Self::DeleteCharacter(_)
            | Self::CreateCharacter { .. }
            | Self::SendChat(_) => None,
            Self::CancelBuff(request) => Some(request.session_id),
            Self::RangedAttack(request) => Some(request.session_id),
            Self::UseItem(request) => Some(request.session_id),
            Self::UseBeltItem(request) => Some(request.session_id),
            Self::MoveInventory(request) => Some(request.session_id),
            Self::DestroyItem(request) => Some(request.session_id),
            Self::DropItem(request) => Some(request.session_id),
            Self::Move(request)
            | Self::GroundFrame { request, .. }
            | Self::SwimFrame { request, .. } => Some(request.session_id),
            Self::SwapSpell { session_id, .. }
            | Self::SendChatInAdmission { session_id, .. }
            | Self::BoatControl { session_id, .. }
            | Self::ArrivalPlacement { session_id, .. }
            | Self::ClickDoor { session_id, .. }
            | Self::PickUp { session_id, .. }
            | Self::OpenContainer { session_id, .. }
            | Self::CloseContainer { session_id, .. }
            | Self::CrossZoneLine { session_id, .. }
            | Self::FinishLocalPad { session_id, .. }
            | Self::ScribeSpell { session_id, .. }
            | Self::DeleteSpell { session_id, .. }
            | Self::ForgetSpell { session_id, .. }
            | Self::MemorizeSpell { session_id, .. }
            | Self::CastSpell { session_id, .. }
            | Self::SetPosture { session_id, .. }
            | Self::FeignLanded { session_id, .. }
            | Self::InspectItem { session_id, .. }
            | Self::Consider { session_id, .. }
            | Self::Camp { session_id, .. }
            | Self::BledOut { session_id, .. }
            | Self::Loot { session_id, .. }
            | Self::LootItem { session_id, .. }
            | Self::EndLoot { session_id, .. }
            | Self::Shop { session_id, .. }
            | Self::Buy { session_id, .. }
            | Self::Sell { session_id, .. }
            | Self::OfferTrade { session_id, .. }
            | Self::AcceptTrade { session_id, .. }
            | Self::CancelTrade { session_id }
            | Self::MoveCoins { session_id, .. }
            | Self::DestroyCoins { session_id, .. }
            | Self::Jump { session_id, .. }
            | Self::EnvironmentalDamage { session_id, .. }
            | Self::LiquidDamage { session_id, .. }
            | Self::AutoAttack { session_id, .. }
            | Self::Consume { session_id, .. }
            | Self::ApplyPoison { session_id, .. }
            | Self::ChatChannel { session_id, .. }
            | Self::AutoEat { session_id, .. }
            | Self::PoisonWait { session_id, .. }
            | Self::AnswerListPolls { session_id, .. }
            | Self::SetServerFilters { session_id, .. }
            | Self::SetMovementOptions { session_id, .. }
            | Self::SetBookOptions { session_id, .. }
            | Self::ExitAtOnce { session_id, .. }
            | Self::UseAbility { session_id, .. }
            | Self::UseKnightAbility { session_id, .. }
            | Self::Shield { session_id, .. }
            | Self::AskDisciplineReuse { session_id, .. }
            | Self::ToggleStance { session_id, .. }
            | Self::AcceptTask { session_id, .. }
            | Self::DeclineTasks { session_id }
            | Self::CancelTask { session_id, .. }
            | Self::AskTaskHistory { session_id, .. }
            | Self::ListTaskTimers { session_id, .. }
            | Self::UseDiscipline { session_id, .. }
            | Self::WhoAll { session_id, .. }
            | Self::Pet { session_id, .. }
            | Self::Training { session_id, .. }
            | Self::AnswerResurrection { session_id, .. }
            | Self::AnswerSacrifice { session_id, .. }
            | Self::AnswerTranslocate { session_id, .. }
            | Self::AnswerDuel { session_id, .. }
            | Self::RespondToPopup { session_id, .. }
            | Self::InspectPlayer { session_id, .. }
            | Self::InspectSpawn { session_id, .. }
            | Self::SetInspectNote { session_id, .. }
            | Self::DropInvisibility { session_id, .. }
            | Self::StopSong { session_id, .. }
            | Self::StopCasting { session_id, .. }
            | Self::YellForHelp { session_id, .. }
            | Self::ChallengeDuel { session_id, .. }
            | Self::DeclineDuel { session_id, .. }
            | Self::SendPetition { session_id, .. }
            | Self::ReportBug { session_id, .. }
            | Self::SendFeedback { session_id, .. }
            | Self::EmoteAnimation { session_id, .. }
            | Self::SocialEmote { session_id, .. }
            | Self::InviteToGroup { session_id, .. }
            | Self::FollowGroup { session_id, .. }
            | Self::DeclineGroup { session_id, .. }
            | Self::Disband { session_id }
            | Self::ToggleAway { session_id }
            | Self::LookForGroup { session_id, .. }
            | Self::ToggleAnonymous { session_id }
            | Self::ToggleRoleplay { session_id }
            | Self::ShowHelm { session_id, .. }
            | Self::Random { session_id, .. }
            | Self::Surname { session_id, .. }
            | Self::RequestTitles { session_id }
            | Self::SetTitle { session_id, .. }
            | Self::TrainAlternateAdvancement { session_id, .. }
            | Self::ActivateAlternateAdvancement { session_id, .. }
            | Self::SetAlternateAdvancementSplit { session_id, .. }
            | Self::BazaarSearch { session_id, .. }
            | Self::BazaarWelcome { session_id }
            | Self::BrowseTrader { session_id, .. }
            | Self::EndBrowse { session_id }
            | Self::SaveBandolier { session_id, .. }
            | Self::ClearBandolier { session_id, .. }
            | Self::SwapBandolier { session_id, .. }
            | Self::SetPotionBelt { session_id, .. }
            | Self::ClearPotionBelt { session_id, .. }
            | Self::Emote { session_id, .. }
            | Self::Assist { session_id, .. }
            | Self::WhoFriends { session_id, .. }
            | Self::ChangeCoins { session_id, .. }
            | Self::Track { session_id, .. }
            | Self::SplitCoins { session_id, .. }
            | Self::SetAutoSplit { session_id, .. }
            | Self::RequestGuildMotd { session_id, .. }
            | Self::AnswerGuildInvite { session_id, .. }
            | Self::GuildInvite { session_id, .. }
            | Self::GuildRemove { session_id, .. }
            | Self::SetGuildMotd { session_id, .. }
            | Self::GuildMakeLeader { session_id, .. }
            | Self::GuildDemote { session_id, .. }
            | Self::RaidInvite { session_id, .. }
            | Self::RaidAccept { session_id }
            | Self::RaidDecline { session_id }
            | Self::RaidLeave { session_id }
            | Self::RaidLock { session_id, .. }
            | Self::RaidMove { session_id, .. }
            | Self::RaidMakeLeader { session_id, .. }
            | Self::RaidRemove { session_id, .. }
            | Self::ReadItem { session_id, .. }
            | Self::Combine { session_id, .. }
            | Self::Consent { session_id, .. }
            | Self::SummonCorpse { session_id, .. }
            | Self::DragCorpse { session_id, .. }
            | Self::DropCorpse { session_id, .. }
            | Self::SelectTarget { session_id, .. }
            | Self::DepopLocalNpc { session_id, .. }
            | Self::ConfigureMotion { session_id, .. }
            | Self::SpeedInputs { session_id, .. }
            | Self::MovementPacketSnapshot { session_id, .. }
            | Self::ClimbReach { session_id, .. }
            | Self::DoorRadii { session_id, .. } => Some(*session_id),
        }
    }

    /// What the zone session must let the player do for this command; None
    /// for the world server's commands, which come before any zone session.
    /// A front end greys out a command whose capability the session lacks.
    #[must_use]
    #[allow(
        clippy::too_many_lines,
        reason = "every kind of command named, so that a new one is a decision here"
    )]
    pub const fn capability(&self) -> Option<crate::world::Capability> {
        use crate::world::Capability;
        Some(match self {
            Self::SelectServer { .. }
            | Self::SelectCharacter { .. }
            | Self::ChooseServerFilters { .. }
            | Self::GenerateName { .. }
            | Self::DeleteCharacter(_)
            | Self::CreateCharacter { .. } => return None,
            Self::ScribeSpell { .. } | Self::ForgetSpell { .. } | Self::MemorizeSpell { .. } => {
                Capability::Spellbook
            }
            // Explicit departures from the official client's behaviour are
            // offered only where the server type permits them.
            Self::SetMovementOptions { .. } | Self::SetBookOptions { .. } => {
                Capability::GameplayDepartures
            }
            Self::CancelBuff(_) => Capability::CancellingBuffs,
            Self::RangedAttack(_) => Capability::RangedCombat,
            Self::StopSong { .. } => Capability::StoppingSongs,
            Self::StopCasting { .. } => Capability::StoppingCasts,
            Self::YellForHelp { .. } => Capability::Yelling,
            Self::SendPetition { .. } => Capability::Petitioning,
            Self::ReportBug { .. } => Capability::ReportingBugs,
            Self::SendFeedback { .. } => Capability::SendingFeedback,
            Self::DeleteSpell { .. } => Capability::DeletingSpells,
            Self::SwapSpell { .. } => Capability::MovingSpells,
            // An item's click effect is a cast.
            Self::CastSpell { .. } | Self::UseItem(_) | Self::DropInvisibility { .. } => {
                Capability::Casting
            }
            Self::UseBeltItem(_) => Capability::UsingPotionBelt,
            Self::BoatControl { .. } => Capability::BoatControl,
            Self::ArrivalPlacement { .. } => Capability::BoatRiding,
            Self::ClickDoor { .. } | Self::DoorRadii { .. } => Capability::Doors,
            Self::PickUp { .. } => Capability::GroundItems,
            Self::CrossZoneLine { .. } | Self::FinishLocalPad { .. } => Capability::Zoning,
            Self::SetPosture { .. } | Self::FeignLanded { .. } => Capability::Posture,
            Self::Move(_)
            | Self::GroundFrame { .. }
            | Self::SwimFrame { .. }
            | Self::ConfigureMotion { .. }
            | Self::SpeedInputs { .. }
            | Self::ClimbReach { .. }
            | Self::MovementPacketSnapshot { .. } => Capability::Moving,
            // Only a server that takes falls from the client lets the player jump.
            Self::Jump { .. } => Capability::Jumping,
            Self::EnvironmentalDamage { .. } => Capability::EnvironmentalDamage,
            Self::LiquidDamage { hazard, .. } => match hazard {
                crate::hazards::LiquidHazard::Drowning => Capability::Drowning,
                crate::hazards::LiquidHazard::Lava | crate::hazards::LiquidHazard::ZoneHeat => {
                    Capability::LavaHeat
                }
                crate::hazards::LiquidHazard::Cold => Capability::ColdWater,
            },
            Self::MoveInventory(_) | Self::DestroyItem(_) => Capability::Inventory,
            Self::DropItem(_) => Capability::GroundDropping,
            Self::Consume { .. } | Self::AutoEat { .. } => Capability::Meals,
            Self::ApplyPoison { .. } | Self::PoisonWait { .. } => Capability::ApplyingPoison,
            Self::ChatChannel { .. } => Capability::ChatChannels,
            Self::AnswerListPolls { .. } => Capability::AnsweringListPolls,
            Self::SetServerFilters { .. } => Capability::ServerFilters,
            Self::SendChat(_)
            | Self::SendChatInAdmission { .. }
            | Self::InspectItem { .. }
            | Self::DepopLocalNpc { .. } => Capability::Talking,
            Self::Consider { .. } | Self::AutoAttack { .. } => Capability::Combat,
            Self::Camp { .. } | Self::ExitAtOnce { .. } => Capability::Camping,
            Self::BledOut { .. } => Capability::BleedingOut,
            Self::Loot { .. } | Self::LootItem { .. } | Self::EndLoot { .. } => Capability::Looting,
            Self::Shop { .. } | Self::Buy { .. } | Self::Sell { .. } => Capability::Trading,
            Self::OfferTrade { .. } | Self::AcceptTrade { .. } | Self::CancelTrade { .. } => {
                Capability::Giving
            }
            // Coins go into a trade window only where the player can give.
            Self::MoveCoins { transfer, .. } => {
                if matches!(transfer.from, crate::money::CoinPlace::Trade)
                    || matches!(transfer.to, crate::money::CoinPlace::Trade)
                {
                    Capability::Giving
                } else {
                    Capability::Inventory
                }
            }
            Self::DestroyCoins { .. } => Capability::DestroyingCoins,
            Self::SelectTarget { .. } => Capability::Targeting,
            Self::UseAbility { .. } => Capability::Abilities,
            Self::UseKnightAbility { .. } => Capability::KnightAbilities,
            Self::Shield { .. } => Capability::Shielding,
            Self::AskDisciplineReuse { .. } => Capability::DisciplineReuse,
            Self::ToggleStance { .. } => Capability::WeaponStances,
            Self::AcceptTask { .. }
            | Self::DeclineTasks { .. }
            | Self::CancelTask { .. }
            | Self::AskTaskHistory { .. }
            | Self::ListTaskTimers { .. } => Capability::Tasks,
            Self::UseDiscipline { .. } => Capability::UsingDisciplines,
            Self::WhoAll { .. } => Capability::Who,
            Self::WhoFriends { .. } => Capability::FriendsWho,
            Self::ChangeCoins { .. } => Capability::BankerChange,
            Self::Track { .. } => Capability::Tracking,
            Self::SplitCoins { .. } | Self::SetAutoSplit { .. } => Capability::Splitting,
            Self::RequestGuildMotd { .. } | Self::AnswerGuildInvite { .. } => {
                Capability::GuildRoster
            }
            Self::GuildInvite { .. }
            | Self::GuildRemove { .. }
            | Self::SetGuildMotd { .. }
            | Self::GuildMakeLeader { .. }
            | Self::GuildDemote { .. } => Capability::GuildManagement,
            Self::Pet { .. } => Capability::Pets,
            Self::Training { .. } => Capability::Training,
            Self::AnswerResurrection { .. } => Capability::Resurrection,
            Self::AnswerSacrifice { .. } => Capability::Sacrifice,
            Self::AnswerTranslocate { .. } => Capability::Translocate,
            Self::AnswerDuel { .. } | Self::ChallengeDuel { .. } => Capability::Dueling,
            Self::DeclineDuel { .. } => Capability::DecliningDuels,
            Self::RespondToPopup { .. } => Capability::ServerDialogs,
            Self::InspectPlayer { .. } | Self::InspectSpawn { .. } => Capability::PlayerInspection,
            Self::SetInspectNote { .. } => Capability::InspectAnswer,
            Self::InviteToGroup { .. }
            | Self::FollowGroup { .. }
            | Self::DeclineGroup { .. }
            | Self::Disband { .. } => Capability::Grouping,
            Self::ToggleAway { .. }
            | Self::ToggleAnonymous { .. }
            | Self::ToggleRoleplay { .. } => Capability::Listing,
            Self::LookForGroup { .. } => Capability::LookingForGroup,
            Self::ShowHelm { .. } => Capability::ShowingHelm,
            Self::Random { .. } => Capability::Rolling,
            Self::Surname { .. } => Capability::Surnaming,
            Self::RequestTitles { .. } | Self::SetTitle { .. } => Capability::Titling,
            Self::TrainAlternateAdvancement { .. }
            | Self::ActivateAlternateAdvancement { .. }
            | Self::SetAlternateAdvancementSplit { .. } => Capability::AlternateAdvancement,
            Self::BazaarSearch { .. } | Self::BazaarWelcome { .. } => Capability::BazaarSearching,
            Self::BrowseTrader { .. } | Self::EndBrowse { .. } => Capability::BazaarBrowsing,
            Self::SaveBandolier { .. } | Self::ClearBandolier { .. } => Capability::Bandoliers,
            Self::SwapBandolier { .. } => Capability::BandolierSwap,
            Self::SetPotionBelt { .. } | Self::ClearPotionBelt { .. } => Capability::PotionBelt,
            Self::Emote { .. } | Self::SocialEmote { .. } | Self::EmoteAnimation { .. } => {
                Capability::Emoting
            }
            Self::Assist { .. } => Capability::Assisting,
            Self::RaidInvite { .. }
            | Self::RaidAccept { .. }
            | Self::RaidDecline { .. }
            | Self::RaidLeave { .. }
            | Self::RaidLock { .. }
            | Self::RaidMove { .. }
            | Self::RaidMakeLeader { .. }
            | Self::RaidRemove { .. } => Capability::Raiding,
            Self::ReadItem { .. } => Capability::Reading,
            Self::Combine { container, .. } => crate::tradeskills::combine_capability(*container),
            Self::OpenContainer { .. } | Self::CloseContainer { .. } => Capability::WorldContainers,
            Self::Consent { .. }
            | Self::SummonCorpse { .. }
            | Self::DragCorpse { .. }
            | Self::DropCorpse { .. } => Capability::Corpses,
        })
    }

    /// When the host made the command, if it says; commands that may wait,
    /// such as closing a loot window, do not.
    #[must_use]
    #[allow(
        clippy::too_many_lines,
        reason = "every kind of command named, so that a new one is a decision here"
    )]
    pub const fn created(&self) -> Option<std::time::Instant> {
        match self {
            Self::LiquidDamage { observation, .. } => Some(observation.sampled_at),
            Self::SelectServer { .. }
            | Self::SelectCharacter { .. }
            | Self::ChooseServerFilters { .. }
            | Self::GenerateName { .. }
            | Self::DeleteCharacter(_)
            | Self::CreateCharacter { .. }
            | Self::SendChat(_)
            | Self::SendChatInAdmission { .. }
            | Self::InspectItem { .. }
            | Self::EndLoot { .. }
            | Self::CancelTrade { .. }
            | Self::AutoEat { .. }
            | Self::PoisonWait { .. }
            | Self::AnswerListPolls { .. }
            | Self::SetServerFilters { .. }
            | Self::SetBookOptions { .. }
            | Self::EnvironmentalDamage { .. }
            // The latest report stands however long it waited.
            | Self::SpeedInputs { .. }
            | Self::ClimbReach { .. }
            | Self::MovementPacketSnapshot { .. }
            | Self::DoorRadii { .. }
            | Self::WhoAll { .. }
            | Self::AnswerResurrection { .. }
            | Self::AnswerSacrifice { .. }
            | Self::AnswerTranslocate { .. }
            | Self::AnswerDuel { .. }
            | Self::SetInspectNote { .. }
            | Self::DropInvisibility { .. }
            | Self::InviteToGroup { .. }
            | Self::FollowGroup { .. }
            | Self::DeclineGroup { .. }
            | Self::Disband { .. }
            | Self::ToggleAway { .. }
            | Self::LookForGroup { .. }
            | Self::ToggleAnonymous { .. }
            | Self::ToggleRoleplay { .. }
            | Self::ShowHelm { .. }
            | Self::Random { .. }
            | Self::Surname { .. }
            | Self::RequestTitles { .. }
            | Self::SetTitle { .. }
            | Self::TrainAlternateAdvancement { .. }
            | Self::ActivateAlternateAdvancement { .. }
            | Self::SetAlternateAdvancementSplit { .. }
            | Self::BazaarSearch { .. }
            | Self::BazaarWelcome { .. }
            | Self::BrowseTrader { .. }
            | Self::EndBrowse { .. }
            | Self::SaveBandolier { .. }
            | Self::ClearBandolier { .. }
            | Self::SwapBandolier { .. }
            | Self::SetPotionBelt { .. }
            | Self::ClearPotionBelt { .. }
            | Self::Emote { .. }
            | Self::SocialEmote { .. }
            | Self::Assist { .. }
            | Self::WhoFriends { .. }
            | Self::ChangeCoins { .. }
            | Self::Track { .. }
            | Self::SplitCoins { .. }
            | Self::SetAutoSplit { .. }
            | Self::RequestGuildMotd { .. }
            | Self::AnswerGuildInvite { .. }
            | Self::GuildInvite { .. }
            | Self::GuildRemove { .. }
            | Self::SetGuildMotd { .. }
            | Self::GuildMakeLeader { .. }
            | Self::GuildDemote { .. }
            | Self::RaidInvite { .. }
            | Self::RaidAccept { .. }
            | Self::RaidDecline { .. }
            | Self::RaidLeave { .. }
            | Self::RaidLock { .. }
            | Self::RaidMove { .. }
            | Self::RaidMakeLeader { .. }
            | Self::RaidRemove { .. }
            | Self::Consent { .. }
            | Self::SummonCorpse { .. }
            | Self::DragCorpse { .. }
            | Self::DropCorpse { .. }
            | Self::DeclineTasks { .. }
            | Self::SelectTarget { .. } => None,
            Self::CancelBuff(request) => Some(request.created),
            Self::RangedAttack(request) => Some(request.created),
            Self::UseItem(request) => Some(request.created),
            Self::UseBeltItem(request) => Some(request.created),
            Self::MoveInventory(request) => Some(request.created),
            Self::DestroyItem(request) => Some(request.created),
            Self::DropItem(request) => Some(request.created),
            Self::Move(request) | Self::GroundFrame { request, .. } | Self::SwimFrame { request, .. } => Some(request.created),
            Self::SwapSpell { created, .. }
            | Self::RespondToPopup { created, .. }
            | Self::InspectPlayer { created, .. }
            | Self::InspectSpawn { created, .. }
            | Self::BoatControl { created, .. }
            | Self::ArrivalPlacement { created, .. }
            | Self::ClickDoor { created, .. }
            | Self::PickUp { created, .. }
            | Self::OpenContainer { created, .. }
            | Self::CloseContainer { created, .. }
            | Self::CrossZoneLine { created, .. }
            | Self::FinishLocalPad { created, .. }
            | Self::ScribeSpell { created, .. }
            | Self::DeleteSpell { created, .. }
            | Self::ForgetSpell { created, .. }
            | Self::MemorizeSpell { created, .. }
            | Self::CastSpell { created, .. }
            | Self::StopSong { created, .. }
            | Self::StopCasting { created, .. }
            | Self::YellForHelp { created, .. }
            | Self::ChallengeDuel { created, .. }
            | Self::DeclineDuel { created, .. }
            | Self::SendPetition { created, .. }
            | Self::ReportBug { created, .. }
            | Self::SendFeedback { created, .. }
            | Self::EmoteAnimation { created, .. }
            | Self::SetPosture { created, .. }
            | Self::FeignLanded { created, .. }
            | Self::Consider { created, .. }
            | Self::DepopLocalNpc { created, .. }
            | Self::Camp { created, .. }
            | Self::ExitAtOnce { created, .. }
            | Self::BledOut { created, .. }
            | Self::Loot { created, .. }
            | Self::LootItem { created, .. }
            | Self::Shop { created, .. }
            | Self::Buy { created, .. }
            | Self::Sell { created, .. }
            | Self::OfferTrade { created, .. }
            | Self::AcceptTrade { created, .. }
            | Self::MoveCoins { created, .. }
            | Self::DestroyCoins { created, .. }
            | Self::Jump { created, .. }
            | Self::AutoAttack { created, .. }
            | Self::Consume { created, .. }
            | Self::ApplyPoison { created, .. }
            | Self::ChatChannel { created, .. }
            | Self::UseAbility { created, .. }
            | Self::UseKnightAbility { created, .. }
            | Self::Shield { created, .. }
            | Self::AskDisciplineReuse { created, .. }
            | Self::ToggleStance { created, .. }
            | Self::AcceptTask { created, .. }
            | Self::CancelTask { created, .. }
            | Self::AskTaskHistory { created, .. }
            | Self::ListTaskTimers { created, .. }
            | Self::UseDiscipline { created, .. }
            | Self::Pet { created, .. }
            | Self::Training { created, .. }
            | Self::ReadItem { created, .. }
            | Self::Combine { created, .. }
            | Self::ConfigureMotion { created, .. }
            | Self::SetMovementOptions { created, .. } => Some(*created),
        }
    }
}

/// A command encoded as one game application packet.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct EncodedCommand {
    /// Little-endian application opcode understood by the selected dialect.
    pub opcode: u16,
    /// Application body without the opcode or reliable-UDP framing.
    pub body: Vec<u8>,
}

/// `OP_CastSpell`: a memorized gem's spell at a target. Titanium's is five
/// 32-bit words: the gem, the spell, no item, the target and 0.
/// `EQMac`'s is TAKP's packed `CastSpell_Struct`
/// ([`crate::spells::eqmac_cast`]).
fn encode_cast(
    dialect: GameDialect,
    gem: u8,
    spell_id: u32,
    target_id: u16,
) -> Result<EncodedCommand> {
    anyhow::ensure!(
        gem < 8 && spell_id != 0 && spell_id != u32::MAX && target_id != 0,
        "invalid spell cast"
    );
    match dialect {
        GameDialect::Titanium => Ok(crate::spells::titanium_cast(gem, spell_id, target_id)),
        GameDialect::EqMac => crate::spells::eqmac_cast(gem, spell_id, target_id),
        GameDialect::Rof2 => anyhow::bail!("not built for RoF2 yet"),
    }
}

/// `EQMac`'s corpse and merchant requests, in the generation's layouts.
fn encode_eqmac_trade(command: &GameCommand) -> Result<EncodedCommand> {
    let (opcode, body) = match command {
        GameCommand::Loot { corpse_id, .. } => (
            crate::loot::eqmac::REQUEST_OPCODE,
            crate::loot::eqmac::request(*corpse_id)?.to_vec(),
        ),
        GameCommand::LootItem {
            corpse_id,
            own_id,
            place,
            auto,
            ..
        } => (
            crate::loot::eqmac::ITEM_OPCODE,
            crate::loot::eqmac::item_request(*corpse_id, *own_id, *place, *auto)?.to_vec(),
        ),
        GameCommand::EndLoot { corpse_id, .. } => (
            crate::loot::eqmac::END_OPCODE,
            crate::loot::eqmac::request(*corpse_id)?.to_vec(),
        ),
        GameCommand::Shop {
            merchant_id,
            own_id,
            open: true,
            ..
        } => (
            crate::merchant::EQMAC_REQUEST_OPCODE,
            crate::merchant::eqmac_request(*merchant_id, *own_id)?.to_vec(),
        ),
        GameCommand::Shop {
            merchant_id,
            own_id,
            ..
        } => (
            crate::merchant::EQMAC_END_OPCODE,
            crate::merchant::eqmac_end(*merchant_id, *own_id)?.to_vec(),
        ),
        GameCommand::Buy {
            merchant_id,
            own_id,
            slot,
            quantity,
            ..
        } => (
            crate::merchant::EQMAC_BUY_OPCODE,
            crate::merchant::eqmac_buy(*merchant_id, *own_id, *slot, *quantity)?.to_vec(),
        ),
        GameCommand::Sell {
            merchant_id,
            own_id,
            slot,
            quantity,
            ..
        } => (
            crate::merchant::EQMAC_SELL_OPCODE,
            crate::merchant::eqmac_sell(*merchant_id, *own_id, *slot, *quantity)?.to_vec(),
        ),
        _ => anyhow::bail!("not an EQMac corpse or merchant command"),
    };
    Ok(EncodedCommand { opcode, body })
}

/// Encode a typed client action for one game dialect.
///
/// `character` supplies the active character name for packet layouts that
/// repeat the sender identity.
///
/// # Errors
///
/// Returns an error when a command contains values that cannot be represented
/// by the selected dialect.
#[allow(
    clippy::too_many_lines,
    reason = "every kind of command named, so that a new one is a decision here"
)]
pub fn encode(
    dialect: GameDialect,
    command: &GameCommand,
    character: &str,
) -> Result<EncodedCommand> {
    match command {
        GameCommand::SelectServer { .. } => {
            anyhow::bail!("choosing a world requires the login controller")
        }
        GameCommand::SelectCharacter { .. }
        | GameCommand::ChooseServerFilters { .. }
        | GameCommand::DeleteCharacter(_)
        | GameCommand::CreateCharacter { .. }
        | GameCommand::GenerateName { .. } => {
            anyhow::bail!("character selection requires the world controller")
        }
        GameCommand::CancelBuff(_) => {
            anyhow::bail!("buff cancellation requires the admitted buff-slot owner")
        }
        GameCommand::RangedAttack(_) => {
            anyhow::bail!("a ranged attack requires the admitted target and loadout")
        }
        GameCommand::DepopLocalNpc { .. } => {
            anyhow::bail!("local fixture cleanup requires the admitted target")
        }
        GameCommand::CastSpell {
            gem,
            spell_id,
            target_id,
            ..
        } => encode_cast(dialect, *gem, *spell_id, *target_id),
        GameCommand::SetPosture {
            spawn_id, posture, ..
        } => encode_posture(dialect, *spawn_id, *posture),
        GameCommand::InspectItem { link_body, .. } => Ok(match dialect {
            GameDialect::Titanium => EncodedCommand {
                opcode: 0x53e5,
                body: crate::items::request(link_body)?.to_vec(),
            },
            GameDialect::EqMac => EncodedCommand {
                opcode: crate::items::EQMAC_LINK_OPCODE,
                body: crate::items::eqmac_request(link_body)?.to_vec(),
            },
            GameDialect::Rof2 => anyhow::bail!("not built for RoF2 yet"),
        }),
        GameCommand::SelectTarget { spawn_id, .. } => encode_target(dialect, *spawn_id),
        GameCommand::Consider { .. } | GameCommand::AutoAttack { .. } => {
            encode_combat(dialect, command)
        }
        GameCommand::Loot { .. }
        | GameCommand::LootItem { .. }
        | GameCommand::EndLoot { .. }
        | GameCommand::Shop { .. }
        | GameCommand::Buy { .. }
        | GameCommand::Sell { .. } => encode_trade(dialect, command),
        // What these send depends on the admitted session's state.
        GameCommand::MoveInventory(_)
        | GameCommand::DestroyItem(_)
        | GameCommand::DropItem(_)
        | GameCommand::BoatControl { .. }
        | GameCommand::ArrivalPlacement { .. }
        | GameCommand::ClickDoor { .. }
        | GameCommand::PickUp { .. }
        | GameCommand::OpenContainer { .. }
        | GameCommand::CloseContainer { .. }
        | GameCommand::OfferTrade { .. }
        | GameCommand::AcceptTrade { .. }
        | GameCommand::CancelTrade { .. }
        | GameCommand::MoveCoins { .. }
        | GameCommand::DestroyCoins { .. }
        | GameCommand::Move(_)
        | GameCommand::GroundFrame { .. }
        | GameCommand::SwimFrame { .. }
        | GameCommand::Jump { .. }
        | GameCommand::EnvironmentalDamage { .. }
        | GameCommand::LiquidDamage { .. }
        | GameCommand::ConfigureMotion { .. }
        | GameCommand::SpeedInputs { .. }
        | GameCommand::FeignLanded { .. }
        | GameCommand::ClimbReach { .. }
        | GameCommand::MovementPacketSnapshot { .. }
        | GameCommand::DoorRadii { .. }
        | GameCommand::CrossZoneLine { .. }
        | GameCommand::FinishLocalPad { .. }
        | GameCommand::Consume { .. }
        | GameCommand::ApplyPoison { .. }
        | GameCommand::ChatChannel { .. }
        | GameCommand::AutoEat { .. }
        | GameCommand::PoisonWait { .. }
        | GameCommand::AnswerListPolls { .. }
        | GameCommand::SetServerFilters { .. }
        | GameCommand::SetMovementOptions { .. }
        | GameCommand::SetBookOptions { .. }
        | GameCommand::ExitAtOnce { .. }
        | GameCommand::UseAbility { .. }
        | GameCommand::UseKnightAbility { .. }
        | GameCommand::Shield { .. }
        | GameCommand::AskDisciplineReuse { .. }
        | GameCommand::ToggleStance { .. }
        | GameCommand::AcceptTask { .. }
        | GameCommand::DeclineTasks { .. }
        | GameCommand::CancelTask { .. }
        | GameCommand::AskTaskHistory { .. }
        | GameCommand::ListTaskTimers { .. }
        | GameCommand::UseDiscipline { .. }
        | GameCommand::WhoAll { .. }
        | GameCommand::Pet { .. }
        | GameCommand::Training { .. }
        | GameCommand::AnswerResurrection { .. }
        | GameCommand::AnswerSacrifice { .. }
        | GameCommand::AnswerTranslocate { .. }
        | GameCommand::AnswerDuel { .. }
        | GameCommand::ChallengeDuel { .. }
        | GameCommand::DeclineDuel { .. }
        | GameCommand::RespondToPopup { .. }
        | GameCommand::InspectPlayer { .. }
        | GameCommand::InspectSpawn { .. }
        | GameCommand::SetInspectNote { .. }
        | GameCommand::DropInvisibility { .. }
        | GameCommand::StopSong { .. }
        | GameCommand::StopCasting { .. }
        | GameCommand::YellForHelp { .. }
        | GameCommand::SendPetition { .. }
        | GameCommand::ReportBug { .. }
        | GameCommand::SendFeedback { .. }
        | GameCommand::EmoteAnimation { .. }
        | GameCommand::InviteToGroup { .. }
        | GameCommand::FollowGroup { .. }
        | GameCommand::DeclineGroup { .. }
        | GameCommand::Disband { .. }
        | GameCommand::ToggleAway { .. }
        | GameCommand::LookForGroup { .. }
        | GameCommand::ToggleAnonymous { .. }
        | GameCommand::ToggleRoleplay { .. }
        | GameCommand::ShowHelm { .. }
        | GameCommand::Random { .. }
        | GameCommand::Surname { .. }
        | GameCommand::RequestTitles { .. }
        | GameCommand::SetTitle { .. }
        | GameCommand::TrainAlternateAdvancement { .. }
        | GameCommand::ActivateAlternateAdvancement { .. }
        | GameCommand::SetAlternateAdvancementSplit { .. }
        | GameCommand::BazaarSearch { .. }
        | GameCommand::BazaarWelcome { .. }
        | GameCommand::BrowseTrader { .. }
        | GameCommand::EndBrowse { .. }
        | GameCommand::SaveBandolier { .. }
        | GameCommand::ClearBandolier { .. }
        | GameCommand::SwapBandolier { .. }
        | GameCommand::SetPotionBelt { .. }
        | GameCommand::ClearPotionBelt { .. }
        | GameCommand::Emote { .. }
        | GameCommand::SocialEmote { .. }
        | GameCommand::Assist { .. }
        | GameCommand::WhoFriends { .. }
        | GameCommand::ChangeCoins { .. }
        | GameCommand::Track { .. }
        | GameCommand::SplitCoins { .. }
        | GameCommand::SetAutoSplit { .. }
        | GameCommand::RequestGuildMotd { .. }
        | GameCommand::AnswerGuildInvite { .. }
        | GameCommand::GuildInvite { .. }
        | GameCommand::GuildRemove { .. }
        | GameCommand::SetGuildMotd { .. }
        | GameCommand::GuildMakeLeader { .. }
        | GameCommand::GuildDemote { .. }
        | GameCommand::RaidInvite { .. }
        | GameCommand::RaidAccept { .. }
        | GameCommand::RaidDecline { .. }
        | GameCommand::RaidLeave { .. }
        | GameCommand::RaidLock { .. }
        | GameCommand::RaidMove { .. }
        | GameCommand::RaidMakeLeader { .. }
        | GameCommand::RaidRemove { .. }
        | GameCommand::ReadItem { .. }
        | GameCommand::Combine { .. }
        | GameCommand::Consent { .. }
        | GameCommand::SummonCorpse { .. }
        | GameCommand::DragCorpse { .. }
        | GameCommand::DropCorpse { .. }
        | GameCommand::UseItem(_)
        | GameCommand::UseBeltItem(_)
        | GameCommand::MemorizeSpell { .. }
        | GameCommand::ForgetSpell { .. }
        | GameCommand::DeleteSpell { .. }
        | GameCommand::SwapSpell { .. }
        | GameCommand::ScribeSpell { .. }
        | GameCommand::Camp { .. }
        | GameCommand::BledOut { .. } => {
            anyhow::bail!("this command requires the admitted session controller")
        }
        GameCommand::SendChat(message) => {
            encode_chat(dialect, message, character, chat::Tongue::COMMON)
        }
        GameCommand::SendChatInAdmission {
            message, tongue, ..
        } => encode_chat(dialect, message, character, *tongue),
    }
}

/// `OP_ChannelMessage`: a chat message in a tongue.
fn encode_chat(
    dialect: GameDialect,
    message: &OutboundChat,
    character: &str,
    tongue: chat::Tongue,
) -> Result<EncodedCommand> {
    Ok(EncodedCommand {
        opcode: match dialect {
            GameDialect::Titanium => 0x1004,
            GameDialect::EqMac => 0x0741,
            GameDialect::Rof2 => anyhow::bail!("not built for RoF2 yet"),
        },
        body: chat::encode_outbound_in(dialect, message, character, tongue)?,
    })
}

/// `OP_TargetMouse`: the player's target, or none.
fn encode_target(dialect: GameDialect, spawn_id: Option<u16>) -> Result<EncodedCommand> {
    anyhow::ensure!(
        spawn_id != Some(0),
        "zero is reserved for clearing a target"
    );
    let target = spawn_id.unwrap_or(0);
    Ok(match dialect {
        GameDialect::Titanium => EncodedCommand {
            opcode: 0x6c47,
            body: u32::from(target).to_le_bytes().to_vec(),
        },
        // TAKP's ClientTarget_Struct holds a 16-bit spawn ID.
        GameDialect::EqMac => EncodedCommand {
            opcode: 0x6241,
            body: target.to_le_bytes().to_vec(),
        },
        GameDialect::Rof2 => anyhow::bail!("not built for RoF2 yet"),
    })
}

/// The Titanium appearance packet that sets the own spawn's posture.
///
/// # Errors
/// Rejects a zero spawn ID.
pub fn titanium_posture(spawn_id: u16, posture: Posture) -> Result<EncodedCommand> {
    encode_posture(GameDialect::Titanium, spawn_id, posture)
}

/// Titanium `OP_Camp`, which starts the server's own camp timer.
#[must_use]
pub fn titanium_camp() -> EncodedCommand {
    EncodedCommand {
        opcode: 0x78c1,
        body: vec![0; 4],
    }
}

/// Titanium `OP_Logout`, sent once the client's camp timer completes.
#[must_use]
pub fn titanium_logout() -> EncodedCommand {
    EncodedCommand {
        opcode: 0x61ff,
        body: Vec::new(),
    }
}

fn encode_posture(dialect: GameDialect, spawn_id: u16, posture: Posture) -> Result<EncodedCommand> {
    match dialect {
        GameDialect::Titanium => {}
        GameDialect::EqMac => anyhow::bail!("posture is not implemented for this dialect"),
        GameDialect::Rof2 => anyhow::bail!("not built for RoF2 yet"),
    }
    anyhow::ensure!(spawn_id != 0, "posture requires an own-spawn ID");
    let mut body = spawn_id.to_le_bytes().to_vec();
    body.extend_from_slice(&14u16.to_le_bytes());
    body.extend_from_slice(&posture.appearance().to_le_bytes());
    Ok(EncodedCommand {
        opcode: 0x7c32,
        body,
    })
}

/// Corpse and merchant requests in Titanium's layouts, and merchant
/// requests in `EQMac`'s.
fn encode_trade(dialect: GameDialect, command: &GameCommand) -> Result<EncodedCommand> {
    match dialect {
        GameDialect::Titanium => {}
        GameDialect::EqMac => return encode_eqmac_trade(command),
        GameDialect::Rof2 => anyhow::bail!("not built for RoF2 yet"),
    }
    let (opcode, body) = match command {
        GameCommand::Loot { corpse_id, .. } => (
            crate::loot::REQUEST_OPCODE,
            crate::loot::request(*corpse_id)?.to_vec(),
        ),
        GameCommand::LootItem {
            corpse_id,
            own_id,
            place,
            auto,
            ..
        } => (
            crate::loot::ITEM_OPCODE,
            crate::loot::item_request(*corpse_id, *own_id, *place, *auto)?.to_vec(),
        ),
        GameCommand::EndLoot { corpse_id, .. } => (
            crate::loot::END_OPCODE,
            crate::loot::end(*corpse_id)?.to_vec(),
        ),
        GameCommand::Shop {
            merchant_id,
            own_id,
            open: true,
            ..
        } => (
            crate::merchant::REQUEST_OPCODE,
            crate::merchant::request(*merchant_id, *own_id, true)?.to_vec(),
        ),
        GameCommand::Shop {
            merchant_id,
            own_id,
            ..
        } => (
            crate::merchant::END_OPCODE,
            crate::merchant::end(*merchant_id, *own_id)?.to_vec(),
        ),
        GameCommand::Buy {
            merchant_id,
            own_id,
            slot,
            quantity,
            ..
        } => (
            crate::merchant::BUY_OPCODE,
            crate::merchant::buy(*merchant_id, *own_id, *slot, *quantity)?.to_vec(),
        ),
        GameCommand::Sell {
            merchant_id,
            slot,
            quantity,
            ..
        } => (
            crate::merchant::SELL_OPCODE,
            crate::merchant::sell(*merchant_id, *slot, *quantity)?.to_vec(),
        ),
        _ => anyhow::bail!("not a corpse or merchant action"),
    };
    Ok(EncodedCommand { opcode, body })
}

/// Consider and auto-attack requests, in each generation's layout.
fn encode_combat(dialect: GameDialect, command: &GameCommand) -> Result<EncodedCommand> {
    use crate::combat;
    match (dialect, command) {
        (
            dialect,
            GameCommand::Consider {
                own_id, target_id, ..
            },
        ) => Ok(match dialect {
            GameDialect::Titanium => EncodedCommand {
                opcode: combat::CONSIDER_OPCODE,
                body: combat::consider_request(*own_id, *target_id)?.to_vec(),
            },
            GameDialect::EqMac => EncodedCommand {
                opcode: combat::EQMAC_CONSIDER_OPCODE,
                body: combat::eqmac_consider_request(*own_id, *target_id)?.to_vec(),
            },
            GameDialect::Rof2 => anyhow::bail!("not built for RoF2 yet"),
        }),
        (dialect, GameCommand::AutoAttack { enabled, .. }) => Ok(EncodedCommand {
            opcode: match dialect {
                GameDialect::Titanium => combat::AUTO_ATTACK_OPCODE,
                GameDialect::EqMac => combat::EQMAC_AUTO_ATTACK_OPCODE,
                GameDialect::Rof2 => anyhow::bail!("not built for RoF2 yet"),
            },
            body: combat::auto_attack(*enabled).to_vec(),
        }),
        _ => anyhow::bail!("not a combat action"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spell_cast_preserves_full_inventory_sentinel_and_rejects_invalid_gems() {
        let mut cast = GameCommand::CastSpell {
            session_id: 7,
            gem: 2,
            spell_id: 42,
            target_id: 19,
            created: std::time::Instant::now(),
        };
        let packet = encode(GameDialect::Titanium, &cast, "Example").unwrap();
        assert_eq!(packet.opcode, 0x304b);
        assert_eq!(packet.body.len(), 20);
        assert_eq!(&packet.body[8..12], &[255; 4]);
        assert_eq!(&packet.body[16..], &[0; 4]);
        // EQMac's: the gem, the spell, no item and the target in 16 bits,
        // then a CRC of 0.
        let packet = encode(GameDialect::EqMac, &cast, "Example").unwrap();
        assert_eq!(packet.opcode, 0x7e41);
        assert_eq!(packet.body, [2, 0, 42, 0, 255, 255, 19, 0, 0, 0, 0, 0]);
        if let GameCommand::CastSpell { gem, .. } = &mut cast {
            *gem = 8;
        }
        assert!(encode(GameDialect::Titanium, &cast, "Example").is_err());
        assert!(encode(GameDialect::EqMac, &cast, "Example").is_err());
    }

    #[test]
    fn each_generation_asks_about_a_linked_item_in_its_own_packet() {
        let inspect = |link_body: &str| GameCommand::InspectItem {
            session_id: 7,
            link_body: link_body.into(),
        };
        let titanium = format!("0{:05X}{}", 42, "0".repeat(39));
        let packet = encode(GameDialect::Titanium, &inspect(&titanium), "Example").unwrap();
        assert_eq!((packet.opcode, packet.body.len()), (0x53e5, 44));
        let packet = encode(GameDialect::EqMac, &inspect("0000042"), "Example").unwrap();
        assert_eq!((packet.opcode, packet.body.len()), (0x6442, 66));
        assert_eq!(&packet.body[..2], &[42, 0]);
        // Each reads only its own links, and never a say link.
        assert!(encode(GameDialect::EqMac, &inspect(&titanium), "Example").is_err());
        assert!(encode(GameDialect::Titanium, &inspect("0000042"), "Example").is_err());
        assert!(encode(GameDialect::EqMac, &inspect("0032769"), "Example").is_err());
    }

    #[test]
    fn posture_uses_own_spawn_and_appearance_type() {
        for (posture, value) in [
            (Posture::Standing, 100u32),
            (Posture::Sitting, 110),
            (Posture::Ducking, 111),
        ] {
            let command = GameCommand::SetPosture {
                session_id: 7,
                spawn_id: 19,
                posture,
                created: std::time::Instant::now(),
            };
            let packet = encode(GameDialect::Titanium, &command, "Example").unwrap();
            assert_eq!(packet.opcode, 0x7c32);
            assert_eq!(&packet.body[..4], &[19, 0, 14, 0]);
            assert_eq!(&packet.body[4..], &value.to_le_bytes());
            assert!(encode(GameDialect::EqMac, &command, "Example").is_err());
        }
    }

    #[test]
    fn targeting_uses_each_generations_mouse_target_and_id_width() {
        let command = GameCommand::SelectTarget {
            request_id: 1,
            session_id: 9,
            spawn_id: Some(513),
            expected_pet: None,
        };
        let packet = encode(GameDialect::Titanium, &command, "Example").unwrap();
        assert_eq!(packet.opcode, 0x6c47);
        assert_eq!(packet.body, vec![1, 2, 0, 0]);
        let packet = encode(GameDialect::EqMac, &command, "Example").unwrap();
        assert_eq!((packet.opcode, packet.body), (0x6241, vec![1, 2]));
        let clear = GameCommand::SelectTarget {
            request_id: 1,
            session_id: 9,
            spawn_id: None,
            expected_pet: None,
        };
        assert_eq!(
            encode(GameDialect::Titanium, &clear, "Example")
                .unwrap()
                .body,
            vec![0; 4]
        );
    }

    #[test]
    fn consider_and_auto_attack_use_each_generations_combat_packets() {
        let created = std::time::Instant::now();
        let consider = GameCommand::Consider {
            session_id: 1,
            own_id: 7,
            target_id: 9,
            created,
        };
        let packet = encode(GameDialect::Titanium, &consider, "Example").unwrap();
        assert_eq!((packet.opcode, packet.body.len()), (0x65ca, 28));
        assert_eq!(&packet.body[4..8], &[9, 0, 0, 0]);
        let packet = encode(GameDialect::EqMac, &consider, "Example").unwrap();
        assert_eq!((packet.opcode, packet.body.len()), (0x3741, 24));
        assert_eq!(&packet.body[..4], &[7, 0, 9, 0]);
        let attack = GameCommand::AutoAttack {
            session_id: 1,
            enabled: true,
            created,
        };
        let packet = encode(GameDialect::Titanium, &attack, "Example").unwrap();
        assert_eq!((packet.opcode, packet.body), (0x5e55, vec![1, 0, 0, 0]));
        let packet = encode(GameDialect::EqMac, &attack, "Example").unwrap();
        assert_eq!((packet.opcode, packet.body), (0x5141, vec![1, 0, 0, 0]));
    }

    #[test]
    fn corpse_and_merchant_commands_use_each_generations_opcodes() {
        let created = std::time::Instant::now();
        // Each generation's corpse and merchant opcodes and lengths.
        let eqmac = [
            Some((0x4e40, 2)),
            Some((0xa040, 12)),
            Some((0x4f40, 2)),
            Some((0x0b40, 12)),
            Some((0x3740, 4)),
            Some((0x3540, 16)),
            Some((0x2740, 16)),
        ];
        for ((command, opcode, length), eqmac) in [
            (
                GameCommand::Loot {
                    session_id: 1,
                    corpse_id: 9,
                    created,
                },
                0x6f90,
                4,
            ),
            (
                GameCommand::LootItem {
                    session_id: 1,
                    corpse_id: 9,
                    own_id: 7,
                    place: 0,
                    auto: true,
                    created,
                },
                0x7081,
                16,
            ),
            (
                GameCommand::EndLoot {
                    session_id: 1,
                    corpse_id: 9,
                },
                0x2316,
                4,
            ),
            (
                GameCommand::Shop {
                    session_id: 1,
                    merchant_id: 9,
                    own_id: 7,
                    open: true,
                    created,
                },
                0x45f9,
                16,
            ),
            (
                GameCommand::Shop {
                    session_id: 1,
                    merchant_id: 9,
                    own_id: 7,
                    open: false,
                    created,
                },
                0x7e03,
                8,
            ),
            (
                GameCommand::Buy {
                    session_id: 1,
                    merchant_id: 9,
                    own_id: 7,
                    slot: 2,
                    quantity: 1,
                    created,
                },
                0x221e,
                24,
            ),
            (
                GameCommand::Sell {
                    session_id: 1,
                    merchant_id: 9,
                    own_id: 7,
                    slot: 23,
                    quantity: 1,
                    created,
                },
                0x0e13,
                16,
            ),
        ]
        .into_iter()
        .zip(eqmac)
        {
            let packet = encode(GameDialect::Titanium, &command, "Example").unwrap();
            assert_eq!((packet.opcode, packet.body.len()), (opcode, length));
            assert_eq!(
                encode(GameDialect::EqMac, &command, "Example")
                    .ok()
                    .map(|packet| (packet.opcode, packet.body.len())),
                eqmac
            );
        }
    }

    #[test]
    fn chat_command_selects_each_dialects_opcode_and_layout() {
        let command = GameCommand::SendChat(OutboundChat::Say("ok".into()));
        let titanium = encode(GameDialect::Titanium, &command, "Example").unwrap();
        let eqmac = encode(GameDialect::EqMac, &command, "Example").unwrap();

        assert_eq!(titanium.opcode, 0x1004);
        assert_eq!(eqmac.opcode, 0x0741);
        assert_ne!(titanium.body, eqmac.body);
    }

    #[test]
    fn admission_scoped_chat_keeps_ordinary_wire_bytes_and_timing() {
        let message = OutboundChat::Say("synthetic words".into());
        let scoped = GameCommand::SendChatInAdmission {
            session_id: 7,
            message: message.clone(),
            tongue: chat::Tongue::COMMON,
        };
        let ordinary = GameCommand::SendChat(message);
        assert_eq!(scoped.session_id(), Some(7));
        assert_eq!(ordinary.session_id(), None);
        assert_eq!(scoped.created(), None);
        assert_eq!(scoped.capability(), ordinary.capability());
        for dialect in [GameDialect::Titanium, GameDialect::EqMac] {
            assert_eq!(
                encode(dialect, &scoped, "Example").unwrap(),
                encode(dialect, &ordinary, "Example").unwrap()
            );
        }
    }
}
