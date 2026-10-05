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
    /// Enter one occupied slot from the current world-server character list.
    SelectCharacter {
        /// Identity supplied with the character list.
        selection_id: u64,
        /// Server slot to enter.
        slot: u8,
    },
    /// Create a character from the current character-selection screen.
    CreateCharacter {
        /// Identity supplied with the character list.
        selection_id: u64,
        /// Requested name, race, class, deity, start zone and stats.
        character: crate::creation::NewCharacter,
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
    /// Use an ordinary nearby door; no lockpicking or cursor-item action is implied.
    ClickDoor {
        /// Current zone admission.
        session_id: u64,
        /// ID from this zone's server-provided door table.
        door_id: u8,
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
    },
    /// Request a transfer after entering a boundary in the local zone assets.
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
    /// Move an inventory item using the admitted session state.
    MoveInventory(crate::inventory::InventoryMove),
    /// Send one chat message.
    SendChat(OutboundChat),
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
    /// Announce a jump the client is simulating; only sessions that accept falls
    /// send it.
    Jump {
        /// Current zone admission.
        session_id: u64,
        /// Reject delayed actions instead of replaying them after a stall.
        created: std::time::Instant,
    },
    /// Report damage the world did to the player, as the host worked it
    /// out. The server lowers the amount by its own reductions, such as the
    /// fall damage reductions of spells, items and AAs, so the host leaves
    /// those out. It never expires, since the damage was done.
    EnvironmentalDamage {
        /// Current zone admission.
        session_id: u64,
        /// What did it.
        hazard: crate::hazards::Hazard,
        /// How much, before the server's own reductions.
        amount: u32,
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
    /// What the session may eat and drink on its own from now on. Each zone
    /// session starts with the client's configured choice, so a host that
    /// lets the player change it says so again after each admission.
    AutoEat {
        /// Current zone admission.
        session_id: u64,
        /// What the session may eat and drink on its own.
        auto_eat: crate::food::AutoEat,
    },
    /// Use an ability: a strike or taunt at the server-side target, or one
    /// on the player.
    UseAbility {
        /// Current zone admission.
        session_id: u64,
        /// The ability.
        ability: crate::abilities::Ability,
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
        /// The player's target, which an attack aims at.
        target: Option<u16>,
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
        /// Where the item is carried.
        slot: crate::inventory::InventorySlot,
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
    /// Accept or decline the resurrection offered last.
    AnswerResurrection {
        /// Current zone admission.
        session_id: u64,
        /// True accepts.
        accept: bool,
    },
    /// Invite a player into the player's group, by name: `/invite`.
    InviteToGroup {
        /// Current zone admission.
        session_id: u64,
        /// Who is invited.
        name: String,
    },
    /// Join the group of whoever invited the player last: `/follow`.
    FollowGroup {
        /// Current zone admission.
        session_id: u64,
    },
    /// Decline the invitation waiting for an answer.
    DeclineGroup {
        /// Current zone admission.
        session_id: u64,
    },
    /// Leave the group, or, as its leader, remove the targeted member or
    /// disband it, as the server decides; with an invitation waiting,
    /// decline it: `/disband`.
    Disband {
        /// Current zone admission.
        session_id: u64,
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
    /// Roll a die from the lowest to the highest number: `/random`.
    Random {
        /// Current zone admission.
        session_id: u64,
        /// The lowest number.
        low: u32,
        /// The highest.
        high: u32,
    },
    /// Emote, in the player's own words: `/emote`.
    Emote {
        /// Current zone admission.
        session_id: u64,
        /// What the player does, after their name.
        text: String,
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
    /// Select or clear a target in the current zone session, without attacking.
    SelectTarget {
        /// Connection identifier supplied by zone admission.
        session_id: u64,
        /// None clears the target; zero is not a valid selected spawn.
        spawn_id: Option<u16>,
    },
    /// Install independently measured motion values for this admission.
    ConfigureMotion {
        /// Current zone-admission identifier.
        session_id: u64,
        /// World-to-wire conversion measured for the current movement mode.
        calibration: crate::movement::MotionCalibration,
        /// Creation time prevents queued configuration from surviving a reset.
        created: std::time::Instant,
    },
    /// Propose fresh motion for the current zone session.
    Move(crate::movement::MovementRequest),
}

impl GameCommand {
    /// The zone admission the command was made for, if it names one.
    #[must_use]
    pub const fn session_id(&self) -> Option<u64> {
        match self {
            Self::SelectCharacter { .. } | Self::CreateCharacter { .. } | Self::SendChat(_) => None,
            Self::UseItem(request) => Some(request.session_id),
            Self::MoveInventory(request) => Some(request.session_id),
            Self::Move(request) => Some(request.session_id),
            Self::SwapSpell { session_id, .. }
            | Self::ClickDoor { session_id, .. }
            | Self::PickUp { session_id, .. }
            | Self::OpenContainer { session_id, .. }
            | Self::CloseContainer { session_id }
            | Self::CrossZoneLine { session_id, .. }
            | Self::ScribeSpell { session_id, .. }
            | Self::DeleteSpell { session_id, .. }
            | Self::ForgetSpell { session_id, .. }
            | Self::MemorizeSpell { session_id, .. }
            | Self::CastSpell { session_id, .. }
            | Self::SetPosture { session_id, .. }
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
            | Self::Jump { session_id, .. }
            | Self::EnvironmentalDamage { session_id, .. }
            | Self::AutoAttack { session_id, .. }
            | Self::Consume { session_id, .. }
            | Self::AutoEat { session_id, .. }
            | Self::UseAbility { session_id, .. }
            | Self::WhoAll { session_id, .. }
            | Self::Pet { session_id, .. }
            | Self::Training { session_id, .. }
            | Self::AnswerResurrection { session_id, .. }
            | Self::InviteToGroup { session_id, .. }
            | Self::FollowGroup { session_id }
            | Self::DeclineGroup { session_id }
            | Self::Disband { session_id }
            | Self::ToggleAway { session_id }
            | Self::ToggleAnonymous { session_id }
            | Self::ToggleRoleplay { session_id }
            | Self::Random { session_id, .. }
            | Self::Emote { session_id, .. }
            | Self::Assist { session_id, .. }
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
            | Self::ConfigureMotion { session_id, .. } => Some(*session_id),
        }
    }

    /// What the zone session must let the player do for this command; None
    /// for the world server's commands, which come before any zone session.
    /// A front end greys out a command whose capability the session lacks.
    #[must_use]
    pub const fn capability(&self) -> Option<crate::world::Capability> {
        use crate::world::Capability;
        Some(match self {
            Self::SelectCharacter { .. } | Self::CreateCharacter { .. } => return None,
            Self::ScribeSpell { .. } | Self::ForgetSpell { .. } | Self::MemorizeSpell { .. } => {
                Capability::Spellbook
            }
            Self::DeleteSpell { .. } => Capability::DeletingSpells,
            Self::SwapSpell { .. } => Capability::MovingSpells,
            // An item's click effect is a cast.
            Self::CastSpell { .. } | Self::UseItem(_) => Capability::Casting,
            Self::ClickDoor { .. } => Capability::Doors,
            Self::PickUp { .. } => Capability::GroundItems,
            Self::CrossZoneLine { .. } => Capability::Zoning,
            Self::SetPosture { .. } | Self::Move(_) | Self::ConfigureMotion { .. } => {
                Capability::Moving
            }
            // Only a server that takes falls from the client lets the player jump.
            Self::Jump { .. } => Capability::Falling,
            Self::EnvironmentalDamage { .. } => Capability::EnvironmentalDamage,
            Self::MoveInventory(_) | Self::Consume { .. } | Self::AutoEat { .. } => {
                Capability::Inventory
            }
            Self::SendChat(_) | Self::InspectItem { .. } => Capability::Talking,
            Self::Consider { .. } | Self::AutoAttack { .. } => Capability::Combat,
            Self::Camp { .. } => Capability::Camping,
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
            Self::SelectTarget { .. } => Capability::Targeting,
            Self::UseAbility { .. } => Capability::Abilities,
            Self::WhoAll { .. } => Capability::Who,
            Self::Pet { .. } => Capability::Pets,
            Self::Training { .. } => Capability::Training,
            Self::AnswerResurrection { .. } => Capability::Resurrection,
            Self::InviteToGroup { .. }
            | Self::FollowGroup { .. }
            | Self::DeclineGroup { .. }
            | Self::Disband { .. } => Capability::Grouping,
            Self::ToggleAway { .. }
            | Self::ToggleAnonymous { .. }
            | Self::ToggleRoleplay { .. } => Capability::Listing,
            Self::Random { .. } => Capability::Rolling,
            Self::Emote { .. } => Capability::Emoting,
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
            Self::Combine { .. } | Self::OpenContainer { .. } | Self::CloseContainer { .. } => {
                Capability::Tradeskills
            }
            Self::Consent { .. }
            | Self::SummonCorpse { .. }
            | Self::DragCorpse { .. }
            | Self::DropCorpse { .. } => Capability::Corpses,
        })
    }

    /// When the host made the command, if it says; commands that may wait,
    /// such as closing a loot window, do not.
    #[must_use]
    pub const fn created(&self) -> Option<std::time::Instant> {
        match self {
            Self::SelectCharacter { .. }
            | Self::CreateCharacter { .. }
            | Self::SendChat(_)
            | Self::InspectItem { .. }
            | Self::EndLoot { .. }
            | Self::CancelTrade { .. }
            | Self::AutoEat { .. }
            | Self::EnvironmentalDamage { .. }
            | Self::WhoAll { .. }
            | Self::Pet { .. }
            | Self::AnswerResurrection { .. }
            | Self::InviteToGroup { .. }
            | Self::FollowGroup { .. }
            | Self::DeclineGroup { .. }
            | Self::Disband { .. }
            | Self::ToggleAway { .. }
            | Self::ToggleAnonymous { .. }
            | Self::ToggleRoleplay { .. }
            | Self::Random { .. }
            | Self::Emote { .. }
            | Self::Assist { .. }
            | Self::RaidInvite { .. }
            | Self::RaidAccept { .. }
            | Self::RaidDecline { .. }
            | Self::RaidLeave { .. }
            | Self::RaidLock { .. }
            | Self::RaidMove { .. }
            | Self::RaidMakeLeader { .. }
            | Self::RaidRemove { .. }
            | Self::ReadItem { .. }
            | Self::CloseContainer { .. }
            | Self::Consent { .. }
            | Self::SummonCorpse { .. }
            | Self::DragCorpse { .. }
            | Self::DropCorpse { .. }
            | Self::SelectTarget { .. } => None,
            Self::UseItem(request) => Some(request.created),
            Self::MoveInventory(request) => Some(request.created),
            Self::Move(request) => Some(request.created),
            Self::SwapSpell { created, .. }
            | Self::ClickDoor { created, .. }
            | Self::PickUp { created, .. }
            | Self::OpenContainer { created, .. }
            | Self::CrossZoneLine { created, .. }
            | Self::ScribeSpell { created, .. }
            | Self::DeleteSpell { created, .. }
            | Self::ForgetSpell { created, .. }
            | Self::MemorizeSpell { created, .. }
            | Self::CastSpell { created, .. }
            | Self::SetPosture { created, .. }
            | Self::Consider { created, .. }
            | Self::Camp { created, .. }
            | Self::BledOut { created, .. }
            | Self::Loot { created, .. }
            | Self::LootItem { created, .. }
            | Self::Shop { created, .. }
            | Self::Buy { created, .. }
            | Self::Sell { created, .. }
            | Self::OfferTrade { created, .. }
            | Self::AcceptTrade { created, .. }
            | Self::MoveCoins { created, .. }
            | Self::Jump { created, .. }
            | Self::AutoAttack { created, .. }
            | Self::Consume { created, .. }
            | Self::UseAbility { created, .. }
            | Self::Training { created, .. }
            | Self::Combine { created, .. }
            | Self::ConfigureMotion { created, .. } => Some(*created),
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

/// `OP_CastSpell`: a memorized gem's spell at a target.
fn encode_cast(
    dialect: GameDialect,
    gem: u8,
    spell_id: u32,
    target_id: u16,
) -> Result<EncodedCommand> {
    anyhow::ensure!(
        dialect == GameDialect::Titanium,
        "casting is not implemented for this dialect"
    );
    anyhow::ensure!(
        gem < 8 && spell_id != 0 && spell_id != u32::MAX && target_id != 0,
        "invalid spell cast"
    );
    let mut body = Vec::with_capacity(20);
    for value in [u32::from(gem), spell_id, u32::MAX, u32::from(target_id), 0] {
        body.extend_from_slice(&value.to_le_bytes());
    }
    Ok(EncodedCommand {
        opcode: 0x304b,
        body,
    })
}

/// `EQMac`'s merchant requests (`crate::merchant`'s `eqmac_*`); looting is
/// not built for it yet.
fn encode_eqmac_trade(command: &GameCommand) -> Result<EncodedCommand> {
    let (opcode, body) = match command {
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
            slot,
            quantity,
            ..
        } => (
            crate::merchant::EQMAC_SELL_OPCODE,
            crate::merchant::eqmac_sell(*merchant_id, *slot, *quantity)?.to_vec(),
        ),
        _ => anyhow::bail!("looting is not implemented for the EQMac client"),
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
pub fn encode(
    dialect: GameDialect,
    command: &GameCommand,
    character: &str,
) -> Result<EncodedCommand> {
    match command {
        GameCommand::SelectCharacter { .. } | GameCommand::CreateCharacter { .. } => {
            anyhow::bail!("character selection requires the world controller")
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
        | GameCommand::ClickDoor { .. }
        | GameCommand::PickUp { .. }
        | GameCommand::OpenContainer { .. }
        | GameCommand::CloseContainer { .. }
        | GameCommand::OfferTrade { .. }
        | GameCommand::AcceptTrade { .. }
        | GameCommand::CancelTrade { .. }
        | GameCommand::MoveCoins { .. }
        | GameCommand::Move(_)
        | GameCommand::Jump { .. }
        | GameCommand::EnvironmentalDamage { .. }
        | GameCommand::ConfigureMotion { .. }
        | GameCommand::CrossZoneLine { .. }
        | GameCommand::Consume { .. }
        | GameCommand::AutoEat { .. }
        | GameCommand::UseAbility { .. }
        | GameCommand::WhoAll { .. }
        | GameCommand::Pet { .. }
        | GameCommand::Training { .. }
        | GameCommand::AnswerResurrection { .. }
        | GameCommand::InviteToGroup { .. }
        | GameCommand::FollowGroup { .. }
        | GameCommand::DeclineGroup { .. }
        | GameCommand::Disband { .. }
        | GameCommand::ToggleAway { .. }
        | GameCommand::ToggleAnonymous { .. }
        | GameCommand::ToggleRoleplay { .. }
        | GameCommand::Random { .. }
        | GameCommand::Emote { .. }
        | GameCommand::Assist { .. }
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
        | GameCommand::MemorizeSpell { .. }
        | GameCommand::ForgetSpell { .. }
        | GameCommand::DeleteSpell { .. }
        | GameCommand::SwapSpell { .. }
        | GameCommand::ScribeSpell { .. }
        | GameCommand::Camp { .. }
        | GameCommand::BledOut { .. } => {
            anyhow::bail!("this command requires the admitted session controller")
        }
        GameCommand::SendChat(message) => Ok(EncodedCommand {
            opcode: match dialect {
                GameDialect::Titanium => 0x1004,
                GameDialect::EqMac => 0x0741,
            },
            body: chat::encode_outbound_for(dialect, message, character)?,
        }),
    }
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
    anyhow::ensure!(
        dialect == GameDialect::Titanium,
        "posture is not implemented for this dialect"
    );
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
    if dialect == GameDialect::EqMac {
        return encode_eqmac_trade(command);
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
            GameDialect::Titanium,
            GameCommand::Consider {
                own_id, target_id, ..
            },
        ) => Ok(EncodedCommand {
            opcode: combat::CONSIDER_OPCODE,
            body: combat::consider_request(*own_id, *target_id)?.to_vec(),
        }),
        (
            GameDialect::EqMac,
            GameCommand::Consider {
                own_id, target_id, ..
            },
        ) => Ok(EncodedCommand {
            opcode: combat::EQMAC_CONSIDER_OPCODE,
            body: combat::eqmac_consider_request(*own_id, *target_id)?.to_vec(),
        }),
        (dialect, GameCommand::AutoAttack { enabled, .. }) => Ok(EncodedCommand {
            opcode: match dialect {
                GameDialect::Titanium => combat::AUTO_ATTACK_OPCODE,
                GameDialect::EqMac => combat::EQMAC_AUTO_ATTACK_OPCODE,
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
        assert!(encode(GameDialect::EqMac, &cast, "Example").is_err());
        if let GameCommand::CastSpell { gem, .. } = &mut cast {
            *gem = 8;
        }
        assert!(encode(GameDialect::Titanium, &cast, "Example").is_err());
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
            session_id: 9,
            spawn_id: Some(513),
        };
        let packet = encode(GameDialect::Titanium, &command, "Example").unwrap();
        assert_eq!(packet.opcode, 0x6c47);
        assert_eq!(packet.body, vec![1, 2, 0, 0]);
        let packet = encode(GameDialect::EqMac, &command, "Example").unwrap();
        assert_eq!((packet.opcode, packet.body), (0x6241, vec![1, 2]));
        let clear = GameCommand::SelectTarget {
            session_id: 9,
            spawn_id: None,
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
        // EQMac's merchant requests, in its own opcodes and lengths; its
        // looting is not built.
        let eqmac = [
            None,
            None,
            None,
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
}
