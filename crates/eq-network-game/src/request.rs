//! What the zone session asks of the server, in no client generation's
//! terms, and the Titanium client's packet for each request: the outgoing
//! side of [`crate::message`]. Each generation encodes requests its own way,
//! and one it has not built yet is refused. A request says only what the
//! player wants; who asks comes with it as the [`Sender`](crate::request::Sender), for the packets
//! that repeat the player's name or spawn.
use crate::{
    abilities::{self, Ability},
    books::{self, Book},
    command::{self, EncodedCommand, GameCommand, Posture},
    confirmations::{self, SacrificeOffer, TranslocateOffer},
    corpses, doors, exchange,
    food::{self, Meal},
    group, guild,
    hazards::{self, Hazard},
    inventory::{self, InventorySlot, MoveQuantity},
    listing::{self, Anonymity},
    money::{CoinDestroy, CoinTransfer},
    movement::{self, PositionPacket},
    objects,
    pets::{self, PetCommand},
    raid,
    resurrection::{self, ResurrectionOffer},
    socials, spells, tradeskills, training,
    who::{self, WhoFilter},
    world::Position,
    zoning, GameDialect,
};
use anyhow::Result;

/// Who the session speaks for. Some packets repeat the player's name or
/// spawn, so the wire takes them from here rather than every request
/// carrying them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sender<'a> {
    /// The player's name.
    pub name: &'a str,
    /// The player's spawn, once the zone has spawned them.
    pub spawn_id: Option<u16>,
}

/// The player's own spawn for a duel's record, which names both players.
fn duelist(sender: Sender<'_>) -> Result<u16> {
    sender
        .spawn_id
        .filter(|&id| id != 0)
        .ok_or_else(|| anyhow::anyhow!("no spawn to answer a duel from"))
}

impl Sender<'_> {
    /// The player's spawn, or zero, which every packet's own check refuses.
    fn spawn(self) -> u16 {
        self.spawn_id.unwrap_or(0)
    }
}

/// What the zone session asks of the server.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Lie down for a feign attempt, as the press does before the feign
    /// request; this does not assert success.
    FeignDeathPosture,
    /// Start the server's camp timer.
    Camp,
    /// Log the camped character out.
    Logout,
    /// Sit, stand or crouch the player.
    Posture(Posture),
    /// Ask the world who is online.
    Who(WhoFilter),
    /// Let a player drag the player's corpses, or take that back.
    Consent {
        /// Who may drag them.
        name: String,
        /// Whether consent is given or taken back.
        given: bool,
    },
    /// Summon one of the player's corpses to them, by the corpse's name.
    SummonCorpse(String),
    /// Start dragging a corpse, by its name.
    DragCorpse(String),
    /// Let go of one dragged corpse, or of every one.
    DropCorpse {
        /// The corpse, by its name; every dragged corpse when absent.
        corpse: Option<String>,
    },
    /// Command the player's pet.
    Pet {
        /// What the pet should do.
        command: PetCommand,
        /// The spawn it acts on, for commands that take one.
        target: Option<u16>,
    },
    /// Ask a guildmaster to train.
    OpenTraining(u16),
    /// Practice a skill once with a guildmaster.
    Train {
        /// The guildmaster's spawn.
        trainer: u16,
        /// The skill's number.
        skill: u32,
    },
    /// Practice a typed entry in one of the trainer's two banks.
    Practice {
        /// The guildmaster's spawn.
        trainer: u16,
        /// The selected bank and entry.
        subject: training::TrainingSubject,
    },
    /// Leave training with a guildmaster.
    EndTraining(u16),
    /// Ask for a book's or note's text.
    ReadBook(Book),
    /// Combine what the tradeskill container in a pack slot holds.
    Combine(InventorySlot),
    /// Accept or decline a resurrection offer.
    AnswerResurrection {
        /// The offer, which the answer repeats.
        offer: ResurrectionOffer,
        /// True accepts.
        accept: bool,
    },
    /// Accept or decline a sacrifice offer.
    AnswerSacrifice {
        /// The offer, which the answer repeats.
        offer: SacrificeOffer,
        /// True accepts.
        accept: bool,
    },
    /// Accept or decline a translocate offer.
    AnswerTranslocate {
        /// The offer, which the answer repeats.
        offer: TranslocateOffer,
        /// True accepts.
        accept: bool,
    },
    /// Accept or decline a challenge to a duel.
    AnswerDuel {
        /// Who challenged.
        challenger: u16,
        /// True accepts.
        accept: bool,
    },
    /// The player took on (`on`) or left a stance's state
    /// ([`crate::appearance::titanium_player_state`]); the `EQMac` client's
    /// is not read.
    PlayerState {
        /// Which.
        stance: crate::appearance::Stance,
        /// Took it on, or left it.
        on: bool,
    },
    /// The Titanium program's own refusal of a challenge, by its reason
    /// ([`socials::titanium_duel_auto_decline`]); the `EQMac` client's is
    /// not read.
    AutoDeclineDuel {
        /// Who challenged.
        challenger: u16,
        /// Why.
        reason: socials::DuelAutoDecline,
    },
    /// The Titanium program's word that the duel is over, when the player
    /// kills the opponent or the opponent leaves
    /// ([`socials::titanium_duel_over`]); the `EQMac` client's is not read.
    DuelOver {
        /// The opponent.
        opponent: u16,
    },
    /// `OP_RequestDuel` naming the other player: a challenge, and on
    /// `EQMac`, by TAKP's note, also what its client sends before it
    /// accepts.
    DuelRequest {
        /// The other player.
        other: u16,
    },
    /// Invite a player into the player's group, by name.
    InviteToGroup {
        /// Who.
        name: String,
        /// Whether the player leads a group already, which the `EQMac`
        /// program sends its invitation differently for.
        leading: bool,
    },
    /// Join the group of the one who invited the player, by their name.
    FollowGroup(String),
    /// Decline an invitation, by the inviter's name.
    DeclineGroup(String),
    /// Leave or disband the group, as the server decides by its idea of the
    /// player's target.
    Disband,
    /// P99's source-backed ordinary Titanium group path, separate from the
    /// emulator compatibility senders above.
    TitaniumGroup(group::TitaniumRequest),
    /// Say the player is away from the keyboard (true), or back.
    SetAway(bool),
    /// Say how the player hides from `/who`.
    SetAnonymity(Anonymity),
    /// Whether the player's helm shows to others.
    ShowHelm(bool),
    /// Say the player looks for a group (true), or no longer.
    SetLooking(bool),
    /// Say the player is seen again, as a Titanium client answers the
    /// server's end of its hiding: its own appearance, invisibility 0
    /// ([`abilities::titanium_unhide`]).
    Unhide,
    /// Call for help to the players nearby (Titanium's `/yell`).
    YellForHelp,
    /// Play an emote's animation on the player's own spawn.
    EmoteAnimation(u8),
    /// Roll a die from the lowest to the highest number.
    Random {
        /// The lowest number.
        low: u32,
        /// The highest.
        high: u32,
    },
    /// Ask for this last name: `/surname` ([`crate::names::surname_request`]).
    Surname(String),
    /// Ask which titles the player may choose.
    RequestTitles,
    /// Choose a title before the name, or a suffix after it; 0 clears it.
    SetTitle {
        /// The title's number, from the server's list; 0 to clear.
        title_id: u32,
        /// Whether it is the suffix.
        suffix: bool,
    },
    /// Train, activate or set the split of the player's AAs
    /// ([`crate::alternate_advancement::titanium_action`]).
    AlternateAdvancement(crate::alternate_advancement::Action),
    /// Search the Bazaar ([`crate::bazaar::search`]).
    BazaarSearch(crate::bazaar::BazaarCriteria),
    /// Ask how many traders and items the Bazaar has
    /// ([`crate::bazaar::welcome`]).
    BazaarWelcome,
    /// Click on a trader to browse their wares ([`crate::bazaar::browse`]).
    BrowseTrader(u16),
    /// End the browse of a trader ([`crate::bazaar::end_browse`]).
    EndBrowse(u16),
    /// Save what is worn as a bandolier set ([`crate::bandolier::save`]).
    SaveBandolier {
        /// The set, 0 to 3.
        set: u8,
        /// Its name.
        name: String,
    },
    /// Clear a bandolier set ([`crate::bandolier::clear`]).
    ClearBandolier(u8),
    /// Swap to a bandolier set ([`crate::bandolier::swap_to`]).
    SwapBandolier(u8),
    /// Put an item in a potion belt slot ([`crate::bandolier::set_belt`]).
    SetPotionBelt {
        /// The belt slot, 0 to 3.
        slot: u8,
        /// The item's number.
        item_id: u32,
    },
    /// Empty a potion belt slot ([`crate::bandolier::clear_belt`]).
    ClearPotionBelt {
        /// The belt slot, 0 to 3.
        slot: u8,
        /// The item it held.
        item_id: u32,
    },
    /// Emote, in the player's own words.
    Emote(String),
    /// A social emote's text, sent as the program builds it.
    SocialEmote(String),
    /// Take the target of this spawn.
    Assist(u16),
    /// Ask which of these friends are online.
    WhoFriends(Vec<String>),
    /// Ask the banker nearest the player to change the coins.
    ChangeCoins,
    /// Share these coins from the purse with the group.
    SplitCoins(crate::world::Coins),
    /// Share looted coins with the group (true), or stop.
    AutoSplit(bool),
    /// Ask for the guild's message of the day, of the player's guild.
    GuildMotd(u32),
    /// Answer a guild invitation.
    AnswerGuildInvite {
        /// Who invited the player.
        inviter: String,
        /// The guild they invited the player into.
        guild_id: u32,
        /// The rank offered.
        rank: crate::listing::GuildRank,
        /// The answer.
        answer: crate::guild::InviteAnswer,
    },
    /// Invite a player into the player's guild, as a member or an officer.
    GuildInvite {
        /// Who is invited.
        name: String,
        /// The player's guild.
        guild_id: u32,
        /// Whether the offer is an officer's rank.
        officer: bool,
    },
    /// Remove a member from the player's guild.
    GuildRemove {
        /// Who is removed; the player themself leaves.
        name: String,
        /// The player's guild.
        guild_id: u32,
    },
    /// Set the guild's message of the day.
    SetGuildMotd(String),
    /// Hand the guild's lead to a member.
    GuildMakeLeader(String),
    /// Lower a member's rank.
    GuildDemote(String),
    /// Invite a player into the player's raid, by name.
    RaidInvite(String),
    /// Join the raid of the one who invited the player, by their name.
    RaidAccept(String),
    /// Leave the player's raid.
    RaidLeave,
    /// Lock the player's raid (true) or unlock it.
    RaidLock(bool),
    /// Move a member of the player's raid, by the name the server gave them,
    /// into a raid group or out of every group.
    RaidMove {
        /// Who moves.
        member: String,
        /// Where to: a raid group, 0 to 11, or none.
        group: Option<u8>,
    },
    /// Hand the lead of the player's raid to a member, by the name the
    /// server gave them.
    RaidMakeLeader(String),
    /// Remove a member from the player's raid, by the name the server gave
    /// them.
    RaidRemove(String),
    /// Use a skill on the server's idea of the target.
    Ability {
        /// The skill.
        ability: Ability,
        /// The target the server has for the player, if any.
        target: Option<u16>,
    },
    /// Pick Pockets or Begging at a target, with the player's own value in
    /// the skill, as the Titanium program sends them
    /// ([`abilities::titanium_ask`]).
    AskOf {
        /// Pick Pockets or Begging.
        ability: Ability,
        /// The target.
        target: u16,
        /// The player's value in the skill.
        skill_value: u32,
    },
    /// Sense Traps, Disarm Traps or Pick Lock at a target of the treasure class
    /// ([`Ability::titanium_at_treasure`]).
    AtTreasure(Ability),
    /// Ask for the tracking list ([`crate::tracking::titanium_refresh`]).
    TrackingList,
    /// The empty message after every press of Tracking
    /// ([`crate::tracking::titanium_pressed`]).
    TrackingPressed,
    /// Follow a spawn from the tracking list, or with 0 stop
    /// ([`crate::tracking::titanium_follow`]).
    TrackSpawn(u16),
    /// The player's own animation of casting a fishing pole, which the
    /// Titanium program sends before a standing player's Fishing
    /// ([`abilities::titanium_fishing_cast`]).
    FishingCast,
    /// Ask another character, or an NPC, to trade, by their spawn.
    Trade(u16),
    /// Take another player's request to trade, by the asker's spawn, or
    /// answer that another trade keeps the player busy.
    AnswerTrade {
        /// The one who asked.
        asker: u32,
        /// Whether the player is busy with another trade.
        busy: bool,
    },
    /// Accept the open trade with this partner, by spawn: `EQMac` names
    /// them, Titanium the player.
    AcceptTrade {
        /// The other side.
        with: u16,
    },
    /// Close the open trade, or withdraw a request, with this partner, by
    /// spawn: `EQMac` names them, Titanium the player.
    CancelTrade {
        /// The other side.
        with: u16,
    },
    /// Use a door within reach.
    ClickDoor {
        /// The door.
        door_id: u8,
        /// What the player holds on the cursor, if the session knows it;
        /// Titanium sends it, `EQMac` refuses a click with one (6.537).
        held: Option<doors::HeldItem>,
    },
    /// An `EQEmu` vessel control request; the session checks support and eligibility.
    ControlBoat {
        /// The selected vessel.
        spawn_id: u16,
        /// Acquire or release.
        intent: crate::boats::Intent,
    },
    /// Name the boat the player now stands on, by its raw spawn name
    /// (Titanium `OP_BoardBoat`).
    BoardBoat(String),
    /// Say the player no longer stands on a boat (Titanium `OP_LeaveBoat`).
    LeaveBoat,
    /// Say the vehicle the profile names was not found at arrival, by its
    /// raw saved name (Titanium 0x3710).
    VehicleMissing(String),
    /// The client's own return of a type 40 or 45 door, which Titanium sends
    /// as a click with no player ([`doors::titanium::return_click`]).
    ReturnDoor(u8),
    /// Pick an item up from the ground, by its drop.
    PickUp(u32),
    /// Open a world container, by its drop.
    OpenContainer(u32),
    /// Close a world container the server opened for the player, repeating
    /// the record the server sent for it.
    CloseContainer {
        /// The container.
        drop_id: u32,
        /// Its tradeskill kind.
        object_type: u32,
        /// The icon its window showed.
        icon: u32,
        /// The name its window showed.
        name: String,
    },
    /// Ask the zone the player is leaving to save them, once it approves
    /// the transfer. `EQMac` sends its own fixed save and ignores these.
    SaveOnZone(zoning::Save),
    /// Ask the zone to save the player on an event, with the event's reason
    /// byte (Titanium's `OP_Save`; read R49-22g). Only the Titanium client
    /// sends it.
    EventSave {
        /// Where the player is now, and the boat they ride.
        save: zoning::Save,
        /// The event's reason ([`zoning::save_reason`]).
        reason: u8,
    },
    /// Take the player's own spawn out of the zone they are leaving: the
    /// last word to it before the world server.
    Depart,
    /// Report that the player bled out: the client's own death report, for
    /// a death the server leaves to it.
    BledOut {
        /// The player's level, which the report carries.
        level: u8,
    },
    /// Take a transfer the server offered or a zone line asked for.
    AnswerZoneOffer {
        /// The zone, or zero for the bind point the server resolves.
        zone_id: u16,
        /// The zone's instance.
        instance_id: u16,
        /// Where in it.
        position: Position,
        /// The offer's reason, echoed back.
        reason: u32,
    },
    /// Memorize a scribed spell into a gem.
    Memorize {
        /// The gem.
        gem: u8,
        /// The spell.
        spell_id: u32,
    },
    /// Forget a gem's spell, keeping it in the book.
    Forget {
        /// The gem.
        gem: u8,
        /// The spell it holds.
        spell_id: u32,
    },
    /// Scribe the cursor's scroll into an empty book slot.
    Scribe {
        /// The book slot.
        slot: u16,
        /// The scroll's spell.
        spell_id: u32,
    },
    /// Delete a book entry.
    DeleteSpell {
        /// The book slot.
        slot: u16,
    },
    /// Exchange two book entries.
    SwapSpells {
        /// One book slot.
        from: u16,
        /// The other.
        to: u16,
    },
    /// A host command that needs nothing from the session's state, as the
    /// generation encodes it.
    Command(GameCommand),
    /// Cast an item's click effect.
    CastItem {
        /// The effect's spell.
        spell_id: u32,
        /// Where the item is.
        slot: InventorySlot,
        /// The spawn it is cast on.
        target_id: u16,
    },
    /// Cast the click effect of an item used from the potion belt, already
    /// checked as an item click is (Titanium only).
    CastBeltItem {
        /// The effect's spell.
        spell_id: u32,
        /// Where the item is.
        slot: InventorySlot,
        /// The spawn it is cast on.
        target_id: u16,
    },
    /// Move an item between slots.
    MoveItem {
        /// Where it is.
        from: InventorySlot,
        /// Where it goes.
        to: InventorySlot,
        /// The whole item, or part of a stack.
        quantity: MoveQuantity,
    },
    /// Explicit whole-cursor destruction; no caller-supplied sentinel.
    DestroyCursor,
    /// Drop the entire cursor item at the server's current player position.
    DropCursor,
    /// Move coins between places, or change them into another coin.
    MoveCoins(CoinTransfer),
    /// Destroy coins from the cursor.
    DestroyCoins(CoinDestroy),
    /// A Warrior's `/shield` of this player ([`crate::shielding`]).
    Shield(u16),
    /// Take an offered task ([`crate::tasks::offers`]).
    AcceptTask {
        /// The kind the offer was sent as.
        kind: crate::tasks::TaskKind,
        /// The task.
        task_id: u32,
        /// The offering NPC's spawn.
        giver: u16,
    },
    /// Cancel a journal entry ([`crate::tasks::offers`]).
    CancelTask(crate::tasks::TaskSlot),
    /// Ask for the done steps of the finished task at this index of the
    /// last finished list ([`crate::tasks::offers`]).
    TaskHistory(u32),
    /// `/tasktimers` ([`crate::tasks::offers`]).
    TaskTimers,
    /// Eat or drink.
    Consume {
        /// Where the food or drink is.
        slot: InventorySlot,
        /// Which it is.
        meal: Meal,
        /// Whether the player chose it, rather than hunger or thirst.
        by_hand: bool,
    },
    /// Put the poison in a slot on the weapon ([`crate::poisons`]).
    ApplyPoison {
        /// Where the poison is.
        slot: InventorySlot,
    },
    /// Where the player is: a position sample.
    Position(PositionPacket),
    /// The player jumped.
    Jump,
    /// The world hurt the player: the damage the client worked out.
    EnvironmentalDamage {
        /// What did it.
        hazard: Hazard,
        /// How much, before the server's own reductions.
        amount: u32,
    },
    /// The notice that Safe Fall lessened a fall.
    SafeFall,
    /// One step of dropping the player's invisibility, where the client
    /// generation sends its steps.
    DropInvisibility(crate::buffs::InvisibilityStep),
    /// One ranged attack already checked against the admitted target and loadout.
    RangedAttack {
        /// The target's zone spawn ID.
        target_id: u16,
        /// The skill selected by the loadout.
        kind: crate::combat::ranged::RangedKind,
    },
    /// The empty `OP_ManaChange`: ask the server to stop the current Bard
    /// song, or, from `/stopcast`, the current cast (R45 0x46a75a sends the
    /// same empty body).
    StopSong,
    /// The notice that goes before the auto-attack toggle, where the client
    /// generation sends one.
    AttackNotice {
        /// Whether attacking turns on.
        enabled: bool,
    },
    /// The notice that goes before the damage report, where the client
    /// generation sends one.
    EnvironmentalNotice {
        /// What did it.
        hazard: Hazard,
        /// How much.
        amount: u32,
    },
}

/// The Titanium client's packet for a request from this sender.
///
/// # Errors
/// Rejects a request whose values the packet cannot carry, such as a name
/// too long for its field, and one that needs a spawn the player lacks.
#[allow(clippy::too_many_lines, reason = "one arm per request")]
pub fn titanium(request: &Request, sender: Sender<'_>) -> Result<EncodedCommand> {
    Ok(match request {
        Request::Camp => command::titanium_camp(),
        Request::Logout => command::titanium_logout(),
        Request::Posture(posture) => command::titanium_posture(sender.spawn(), *posture)?,
        Request::Who(filter) => who::request(filter)?,
        Request::Consent { name, given } => corpses::consent(name, *given)?,
        Request::SummonCorpse(corpse) => corpses::summon(corpse, sender.name)?,
        Request::DragCorpse(corpse) => corpses::drag(corpse, sender.name)?,
        Request::DropCorpse { corpse } => corpses::release(corpse.as_deref())?,
        Request::Pet { command, target } => pets::command(*command, *target),
        // The program's 12 bytes (read R19 a), where the server reads none.
        Request::Ability {
            ability: Ability::FeignDeath,
            ..
        } => crate::feign::titanium_request(sender.spawn())?,
        Request::Ability { ability, target } => ability
            .encode(target.unwrap_or(0))
            .ok_or_else(|| anyhow::anyhow!("{ability:?} asks of its target with the skill"))?,
        Request::AskOf {
            ability,
            target,
            skill_value,
        } => abilities::titanium_ask(*ability, *target, sender.spawn(), *skill_value)?,
        Request::AtTreasure(ability) => ability
            .titanium_at_treasure()
            .ok_or_else(|| anyhow::anyhow!("{ability:?} has no treasure message"))?,
        Request::FishingCast => abilities::titanium_fishing_cast(sender.spawn())?,
        Request::TrackingList => crate::tracking::titanium_refresh(),
        Request::TrackingPressed => crate::tracking::titanium_pressed(),
        Request::TrackSpawn(spawn_id) => crate::tracking::titanium_follow(*spawn_id),
        Request::OpenTraining(trainer) => training::titanium_open(*trainer, sender.spawn()),
        Request::Train { trainer, skill } => training::titanium_train(*trainer, *skill)?,
        Request::Practice { trainer, subject } => training::titanium_practice(*trainer, *subject)?,
        Request::EndTraining(trainer) => training::titanium_end(*trainer, sender.spawn()),
        Request::AnswerResurrection { offer, accept } => {
            resurrection::titanium_answer(offer, *accept)
        }
        Request::AnswerSacrifice { offer, accept } => {
            confirmations::titanium_sacrifice_answer(offer, *accept)
        }
        Request::AnswerTranslocate { offer, accept } => {
            confirmations::titanium_translocate_answer(offer, *accept)
        }
        Request::AnswerDuel { challenger, accept } => {
            // The program has no decline (R49-6a): a no is the host's own.
            anyhow::ensure!(*accept, "the Titanium client sends no decline to a duel");
            socials::titanium_duel_accept(*challenger, duelist(sender)?)
        }
        Request::PlayerState { stance, on } => {
            crate::appearance::titanium_player_state(duelist(sender)?, stance.states(), *on)
        }
        Request::DuelOver { opponent } => socials::titanium_duel_over(*opponent, duelist(sender)?),
        Request::AutoDeclineDuel { challenger, reason } => {
            socials::titanium_duel_auto_decline(*challenger, duelist(sender)?, *reason)
        }
        Request::DuelRequest { other } => socials::titanium_duel_request(*other, duelist(sender)?),
        Request::InviteToGroup { name, .. } => group::invite(name, sender.name)?,
        Request::FollowGroup(inviter) => group::follow(inviter, sender.name)?,
        Request::DeclineGroup(inviter) => group::decline(inviter, sender.name)?,
        Request::Disband => group::disband(sender.name)?,
        Request::TitaniumGroup(request) => group::titanium(request, sender.name)?,
        Request::SetAway(away) => listing::titanium_away(sender.spawn(), *away)?,
        Request::SetAnonymity(anonymity) => {
            listing::titanium_anonymity(sender.spawn(), *anonymity)?
        }
        Request::ShowHelm(show) => listing::titanium_show_helm(sender.spawn(), *show)?,
        Request::SetLooking(on) => listing::titanium_looking(*on),
        Request::Unhide => abilities::titanium_unhide(sender.spawn())?,
        Request::YellForHelp => socials::titanium_yell(),
        Request::EmoteAnimation(animation) => {
            socials::titanium_emote_animation(sender.spawn(), *animation)?
        }
        Request::Random { low, high } => socials::random(*low, *high),
        Request::Surname(surname) => EncodedCommand {
            opcode: crate::names::SURNAME_OPCODE,
            body: crate::names::surname_request(sender.name, surname)?,
        },
        // `EQEmu`'s handler reads no body; the program's is not read yet
        // (inferred empty, program read R49-14b).
        Request::RequestTitles => EncodedCommand {
            opcode: crate::names::REQUEST_TITLES_OPCODE,
            body: Vec::new(),
        },
        Request::BazaarSearch(criteria) => EncodedCommand {
            opcode: crate::bazaar::SEARCH_OPCODE,
            body: crate::bazaar::search(criteria)?,
        },
        Request::BazaarWelcome => EncodedCommand {
            opcode: crate::bazaar::SEARCH_OPCODE,
            body: crate::bazaar::welcome(),
        },
        Request::BrowseTrader(trader) => EncodedCommand {
            opcode: crate::bazaar::BROWSE_OPCODE,
            body: crate::bazaar::browse(*trader).to_vec(),
        },
        Request::EndBrowse(trader) => EncodedCommand {
            opcode: crate::bazaar::TRADER_OPCODE,
            body: crate::bazaar::end_browse(*trader).to_vec(),
        },
        Request::SaveBandolier { set, name } => EncodedCommand {
            opcode: crate::bandolier::OPCODE,
            body: crate::bandolier::save(*set, name)?.to_vec(),
        },
        Request::ClearBandolier(set) => EncodedCommand {
            opcode: crate::bandolier::OPCODE,
            body: crate::bandolier::clear(*set)?.to_vec(),
        },
        Request::SwapBandolier(set) => EncodedCommand {
            opcode: crate::bandolier::OPCODE,
            body: crate::bandolier::swap_to(*set)?.to_vec(),
        },
        Request::SetPotionBelt { slot, item_id } => EncodedCommand {
            opcode: crate::bandolier::BELT_OPCODE,
            body: crate::bandolier::set_belt(*slot, *item_id)?.to_vec(),
        },
        Request::ClearPotionBelt { slot, item_id } => EncodedCommand {
            opcode: crate::bandolier::BELT_OPCODE,
            body: crate::bandolier::clear_belt(*slot, *item_id)?.to_vec(),
        },
        Request::SetTitle { title_id, suffix } => EncodedCommand {
            opcode: crate::names::SET_TITLE_OPCODE,
            body: crate::names::set_title(*title_id, *suffix),
        },
        Request::AlternateAdvancement(action) => EncodedCommand {
            opcode: crate::alternate_advancement::ACTION_OPCODE,
            body: crate::alternate_advancement::titanium_action(*action)?.to_vec(),
        },
        Request::Emote(text) => socials::emote(text)?,
        Request::SocialEmote(text) => socials::social_emote(text)?,
        Request::Assist(spawn_id) => socials::assist(*spawn_id),
        Request::WhoFriends(names) => crate::friends::titanium_request(names)?,
        Request::ChangeCoins => crate::banker::titanium_request(),
        Request::SplitCoins(coins) => crate::split::titanium_split(*coins)?,
        Request::AutoSplit(on) => crate::split::titanium_auto_split(sender.spawn(), *on)?,
        Request::GuildMotd(guild_id) => guild::titanium_get_motd(*guild_id, sender.name)?,
        Request::AnswerGuildInvite {
            inviter,
            guild_id,
            rank,
            answer,
        } => guild::titanium_answer_invite(inviter, sender.name, *guild_id, *rank, *answer)?,
        Request::GuildInvite {
            name,
            guild_id,
            officer,
        } => guild::titanium_invite(
            name,
            sender.name,
            *guild_id,
            if *officer {
                crate::listing::GuildRank::Officer
            } else {
                crate::listing::GuildRank::Member
            },
        )?,
        Request::GuildRemove { name, guild_id } => {
            guild::titanium_remove(name, sender.name, *guild_id)?
        }
        Request::SetGuildMotd(motd) => guild::titanium_set_motd(sender.name, motd)?,
        Request::GuildMakeLeader(member) => guild::titanium_make_leader(sender.name, member)?,
        Request::GuildDemote(member) => guild::titanium_demote(sender.name, member)?,
        Request::RaidInvite(name) => raid::invite(name, sender.name)?,
        Request::RaidAccept(inviter) => raid::accept(inviter, sender.name)?,
        Request::RaidLeave => raid::remove(sender.name, sender.name)?,
        Request::RaidLock(locked) => raid::lock(sender.name, *locked)?,
        Request::RaidMove { member, group } => raid::move_member(sender.name, member, *group)?,
        Request::RaidMakeLeader(member) => raid::make_leader(sender.name, member)?,
        Request::RaidRemove(member) => raid::remove(sender.name, member)?,
        Request::ReadBook(book) => books::titanium_request(book)?,
        Request::Combine(container) => tradeskills::titanium_combine(*container)?,
        Request::Trade(with) => exchange::request(sender.spawn(), *with)?,
        Request::AnswerTrade { asker, busy: false } => {
            exchange::acknowledge(sender.spawn(), *asker)?
        }
        Request::AnswerTrade { asker, busy: true } => exchange::busy(sender.spawn(), *asker)?,
        Request::AcceptTrade { .. } => exchange::accept(sender.spawn())?,
        Request::CancelTrade { .. } => exchange::cancel(sender.spawn())?,
        Request::ClickDoor { door_id, held } => {
            doors::titanium_click(*door_id, sender.spawn(), *held)
        }
        Request::ControlBoat { spawn_id, intent } => crate::boats::titanium(*spawn_id, *intent)?,
        Request::BoardBoat(name) => crate::boats::riders::titanium_board(name)?,
        Request::LeaveBoat => crate::boats::riders::titanium_leave(),
        Request::VehicleMissing(name) => crate::boats::riders::titanium_missing(name)?,
        Request::ReturnDoor(door_id) => doors::titanium::return_click(*door_id),
        // A click opens a world container as it picks an item up.
        Request::PickUp(drop_id) | Request::OpenContainer(drop_id) => {
            objects::titanium_pickup(*drop_id, sender.spawn())
        }
        Request::CloseContainer {
            drop_id,
            object_type,
            icon,
            name,
        } => objects::titanium_close(
            u32::from(sender.spawn()),
            *drop_id,
            (*object_type, *icon),
            name,
        ),
        Request::AnswerZoneOffer {
            zone_id,
            instance_id,
            position,
            reason,
        } => zoning::titanium_answer(sender.name, (*zone_id, *instance_id), *position, *reason)?,
        Request::SaveOnZone(save) => zoning::titanium_save_on_zone(save),
        Request::EventSave { save, reason } => zoning::titanium_event_save(save, *reason),
        Request::RangedAttack { target_id, kind } => {
            crate::combat::ranged::titanium(*target_id, *kind)?
        }
        Request::Depart => zoning::titanium_depart(sender.spawn()),
        Request::EnvironmentalNotice { .. }
        | Request::AttackNotice { .. }
        | Request::DropInvisibility(_)
        | Request::BledOut { .. } => anyhow::bail!("the Titanium client cannot send {request:?}"),
        Request::FeignDeathPosture => crate::feign::titanium_posture(sender.spawn())?,
        Request::StopSong => spells::titanium_stop_song(),
        Request::Memorize { gem, spell_id } => spells::titanium_memorize(*gem, *spell_id),
        Request::Forget { gem, spell_id } => spells::titanium_forget(*gem, *spell_id),
        Request::Scribe { slot, spell_id } => spells::titanium_scribe(*slot, *spell_id),
        Request::DeleteSpell { slot } => spells::titanium_delete(*slot),
        Request::SwapSpells { from, to } => spells::titanium_swap(*from, *to),
        Request::Command(command) => command::encode(GameDialect::Titanium, command, sender.name)?,
        Request::CastItem {
            spell_id,
            slot,
            target_id,
        } => inventory::titanium_item_cast(*spell_id, *slot, *target_id)?,
        Request::CastBeltItem {
            spell_id,
            slot,
            target_id,
        } => inventory::titanium_belt_cast(*spell_id, *slot, *target_id)?,
        Request::MoveItem { from, to, quantity } => {
            inventory::titanium_move(*from, *to, *quantity)?
        }
        Request::DestroyCursor => inventory::titanium_destroy(),
        Request::DropCursor => inventory::titanium_ground_drop(),
        Request::MoveCoins(transfer) => transfer.encode()?,
        Request::DestroyCoins(destroy) => destroy.encode()?,
        Request::Shield(target) => crate::shielding::titanium_shield(*target)?,
        Request::AcceptTask {
            kind,
            task_id,
            giver,
        } => crate::tasks::offers::titanium_accept(*kind, *task_id, *giver)?,
        Request::CancelTask(at) => crate::tasks::offers::titanium_cancel(*at)?,
        Request::TaskHistory(index) => crate::tasks::offers::titanium_history(*index)?,
        Request::TaskTimers => crate::tasks::offers::titanium_timers(),
        Request::Consume {
            slot,
            meal,
            by_hand,
        } => food::consume(*slot, *meal, *by_hand),
        Request::ApplyPoison { slot } => crate::poisons::titanium_request(*slot)?,
        Request::Position(sample) => sample.packet()?,
        Request::Jump => movement::titanium_jump(),
        Request::EnvironmentalDamage { hazard, amount } => {
            hazards::titanium_damage(sender.spawn(), *hazard, *amount)?
        }
        Request::SafeFall => hazards::safe_fall(true),
    })
}

/// `EQMac`'s door click: its program asks for a clear cursor first, so a
/// click holding an item is refused (the session checks the cursor too).
fn eqmac_door(
    door_id: u8,
    held: Option<doors::HeldItem>,
    sender: Sender<'_>,
) -> Result<EncodedCommand> {
    anyhow::ensure!(held.is_none(), "EQMac's program asks for a clear cursor");
    doors::eqmac::click(door_id, sender.spawn())
}

/// The `EQMac` client's packet for a request from this sender: camping,
/// logging out, its stance and position, the world's damage, ordinary door use,
/// ground pickups, item moves,
/// eating and drinking, an item's click, the spellbook's changes but
/// deleting, the answers to a sacrifice and a translocate, and the host
/// commands its generation encodes so far: chat and casting from a gem.
///
/// # Errors
/// Refuses every other request, and a command the generation cannot carry.
#[allow(clippy::too_many_lines, reason = "one arm per request")]
pub fn eqmac(request: &Request, sender: Sender<'_>) -> Result<EncodedCommand> {
    match request {
        Request::ReadBook(book) => books::eqmac_request(book),
        Request::Ability { ability, target } => {
            abilities::eqmac::encode(*ability, *target, sender.spawn())
        }
        Request::FeignDeathPosture => abilities::eqmac::feign_posture(sender.spawn()),
        Request::Shield(target) => crate::shielding::eqmac_shield(*target),
        Request::Camp => Ok(crate::quarm::camp()),
        Request::Jump => Ok(crate::quarm::jump()),
        Request::Consent { name, given: true } => corpses::eqmac_consent(name),
        // Neither Mac server has a deny: a consent given again takes it back.
        Request::Consent { given: false, .. } => {
            anyhow::bail!("TAKP and Quarm take a consent back only by giving it again")
        }
        Request::SummonCorpse(corpse) => corpses::eqmac_summon(corpse, sender.name),
        Request::DragCorpse(corpse) => corpses::eqmac_drag(corpse, sender.name),
        // Quarm's own opcode; the session sends it only to Quarm.
        Request::DropCorpse { corpse } => corpses::quarm_drop(corpse.as_deref(), sender.name),
        Request::Pet { command, target } if command.on_eqmac() => {
            Ok(pets::eqmac_command(*command, *target))
        }
        Request::Random { low, high } => Ok(socials::eqmac_random(*low, *high)),
        Request::Assist(spawn_id) => Ok(socials::eqmac_assist(*spawn_id)),
        Request::SplitCoins(coins) => crate::split::eqmac_split(*coins),
        Request::AutoSplit(on) => crate::split::eqmac_auto_split(sender.spawn(), *on),
        Request::SetAnonymity(anonymity) => {
            crate::listing::eqmac_anonymity(sender.spawn(), *anonymity)
        }
        Request::Logout => Ok(crate::quarm::logout()),
        Request::Posture(posture) => crate::quarm::posture(sender.spawn(), *posture),
        Request::Position(sample) => crate::quarm::client_update(sample),
        Request::EnvironmentalDamage { hazard, amount } => {
            hazards::eqmac_damage(sender.spawn(), *hazard, *amount)
        }
        Request::EnvironmentalNotice { hazard, amount } => {
            hazards::eqmac_notice(sender.spawn(), *hazard, *amount)
        }
        Request::SafeFall => Ok(hazards::safe_fall(false)),
        Request::DropInvisibility(crate::buffs::InvisibilityStep::State(state)) => {
            crate::quarm::invisibility(sender.spawn(), *state)
        }
        Request::DropInvisibility(step) => Ok(EncodedCommand {
            opcode: crate::buffs::EQMAC_BUFF_OPCODE,
            body: crate::buffs::eqmac_fade(sender.spawn(), *step)?.to_vec(),
        }),
        Request::AttackNotice { enabled } => Ok(EncodedCommand {
            opcode: crate::combat::EQMAC_AUTO_ATTACK2_OPCODE,
            body: crate::combat::auto_attack(*enabled).to_vec(),
        }),
        Request::Command(command) => command::encode(GameDialect::EqMac, command, sender.name),
        Request::InviteToGroup { name, leading } => {
            group::eqmac_invite(name, sender.name, *leading)
        }
        Request::FollowGroup(inviter) => group::eqmac_follow(inviter, sender.name),
        Request::DeclineGroup(inviter) => group::eqmac_decline(inviter, sender.name),
        Request::Disband => group::eqmac_disband(sender.name),
        // EQMac has no instances, and its request carries no position.
        Request::AnswerZoneOffer {
            zone_id,
            instance_id: 0,
            reason,
            ..
        } => crate::quarm::zone_change(sender.name, *zone_id, *reason),
        Request::SaveOnZone(_) => Ok(crate::quarm::save_on_zone()),
        // The Mac program's event saves are unread (6.615).
        Request::EventSave { .. } => anyhow::bail!("Event saves are unavailable for EQMac"),
        Request::DestroyCursor => anyhow::bail!("Cursor destruction is unavailable for EQMac"),
        Request::DropCursor => anyhow::bail!("Cursor dropping is unavailable for EQMac"),
        Request::MoveItem { from, to, quantity } => {
            crate::inventory::eqmac_move(*from, *to, *quantity)
        }
        // A click opens a world container as it picks an item up (6.598).
        Request::PickUp(drop_id) | Request::OpenContainer(drop_id) => {
            Ok(objects::eqmac::pickup(*drop_id, sender.spawn()))
        }
        Request::CloseContainer {
            drop_id,
            object_type,
            icon,
            ..
        } => Ok(objects::eqmac::close(
            sender.spawn(),
            *drop_id,
            *object_type,
            *icon,
        )),
        Request::Combine(container) => tradeskills::eqmac_combine(*container),
        Request::ControlBoat { spawn_id, intent } => crate::boats::eqmac(*spawn_id, *intent),
        Request::ClickDoor { door_id, held } => eqmac_door(*door_id, *held, sender),
        Request::Consume {
            slot,
            meal,
            by_hand,
        } => food::eqmac_consume(*slot, *meal, *by_hand),
        Request::ApplyPoison { slot } => crate::poisons::eqmac_request(*slot),
        Request::Depart => Ok(crate::quarm::depart(sender.spawn())),
        Request::CastItem {
            spell_id,
            slot,
            target_id,
        } => spells::eqmac_item_cast(*spell_id, *slot, *target_id),
        Request::Memorize { gem, spell_id } => Ok(spells::eqmac_memorize(*gem, *spell_id)),
        Request::Forget { gem, spell_id } => Ok(spells::eqmac_forget(*gem, *spell_id)),
        Request::Scribe { slot, spell_id } => spells::eqmac_scribe(*slot, *spell_id),
        Request::SwapSpells { from, to } => spells::eqmac_swap(*from, *to),
        Request::DeleteSpell { slot } => spells::eqmac_delete(*slot),
        Request::Emote(text) => socials::eqmac_emote(text),
        Request::SocialEmote(text) => socials::eqmac_social_emote(text),
        Request::YellForHelp => Ok(socials::eqmac_yell()),
        Request::MoveCoins(transfer) => transfer.encode_eqmac(),
        Request::DestroyCoins(destroy) => destroy.encode_eqmac(),
        Request::BledOut { level } => crate::quarm::bled_out(sender.spawn(), *level),
        Request::AnswerSacrifice { offer, accept } => {
            Ok(confirmations::eqmac_sacrifice_answer(offer, *accept))
        }
        Request::AnswerResurrection { offer, accept } => {
            Ok(resurrection::eqmac_answer(offer, *accept))
        }
        Request::OpenTraining(trainer) => Ok(training::eqmac_open(*trainer, sender.spawn())),
        Request::Train { trainer, skill } => training::eqmac_practice(
            *trainer,
            sender.spawn(),
            training::TrainingSubject::Skill(*skill),
        ),
        Request::Practice { trainer, subject } => {
            training::eqmac_practice(*trainer, sender.spawn(), *subject)
        }
        Request::EndTraining(trainer) => Ok(training::eqmac_end(*trainer, sender.spawn())),
        Request::AnswerTranslocate { offer, accept } => {
            Ok(confirmations::eqmac_translocate_answer(offer, *accept))
        }
        Request::AnswerDuel { challenger, accept } => {
            let own = duelist(sender)?;
            Ok(if *accept {
                socials::eqmac_duel_accept(*challenger, own)
            } else {
                socials::eqmac_duel_decline(*challenger, own)
            })
        }
        Request::DuelRequest { other } => Ok(socials::eqmac_duel_request(*other, duelist(sender)?)),
        Request::Trade(with) => exchange::eqmac::request(sender.spawn(), *with),
        Request::AnswerTrade { asker, busy: false } => {
            exchange::eqmac::acknowledge(sender.spawn(), *asker)
        }
        Request::AnswerTrade { asker, busy: true } => exchange::eqmac::busy(sender.spawn(), *asker),
        Request::AcceptTrade { with } => exchange::eqmac::accept(*with),
        Request::CancelTrade { with } => exchange::eqmac::cancel(sender.spawn(), *with),
        Request::AlternateAdvancement(action) => Ok(EncodedCommand {
            opcode: crate::alternate_advancement::EQMAC_ACTION_OPCODE,
            body: crate::alternate_advancement::eqmac_action(*action)?.to_vec(),
        }),
        Request::Surname(surname) => Ok(EncodedCommand {
            opcode: crate::names::EQMAC_SURNAME_OPCODE,
            body: crate::names::surname_request(sender.name, surname)?,
        }),
        Request::Who(filter) => who::eqmac::request(filter),
        Request::WhoFriends(names) => crate::friends::eqmac_request(names),
        Request::BazaarSearch(criteria) => Ok(EncodedCommand {
            opcode: crate::bazaar::eqmac::SEARCH_OPCODE,
            body: crate::bazaar::eqmac::search(criteria)?,
        }),
        Request::BazaarWelcome => Ok(EncodedCommand {
            opcode: crate::bazaar::eqmac::SEARCH_OPCODE,
            body: crate::bazaar::eqmac::welcome(),
        }),
        _ => anyhow::bail!("the EQMac client cannot send {request:?} yet"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A player with a spawn.
    const PLAYER: Sender<'static> = Sender {
        name: "Tester",
        spawn_id: Some(7),
    };

    #[test]
    fn lfg_goes_only_on_titanium() {
        assert_eq!(
            titanium(&Request::SetLooking(true), PLAYER).unwrap(),
            crate::listing::titanium_looking(true)
        );
        // TAKP's `OP_LFGCommand` is a world-stage message of its own, not
        // read (checklist 6.515).
        assert!(eqmac(&Request::SetLooking(true), PLAYER).is_err());
    }

    #[test]
    fn eqmac_trades_in_its_own_bodies_naming_the_partner() {
        assert_eq!(
            eqmac(&Request::Trade(42), PLAYER).unwrap(),
            exchange::eqmac::request(7, 42).unwrap()
        );
        let answer = |busy| Request::AnswerTrade { asker: 50, busy };
        assert_eq!(
            eqmac(&answer(false), PLAYER).unwrap(),
            exchange::eqmac::acknowledge(7, 50).unwrap()
        );
        assert_eq!(
            eqmac(&answer(true), PLAYER).unwrap(),
            exchange::eqmac::busy(7, 50).unwrap()
        );
        assert_eq!(
            eqmac(&Request::AcceptTrade { with: 42 }, PLAYER).unwrap(),
            exchange::eqmac::accept(42).unwrap()
        );
        assert_eq!(
            eqmac(&Request::CancelTrade { with: 42 }, PLAYER).unwrap(),
            exchange::eqmac::cancel(7, 42).unwrap()
        );
        // Titanium names the player, whoever the partner is.
        assert_eq!(
            titanium(&Request::CancelTrade { with: 42 }, PLAYER).unwrap(),
            exchange::cancel(7).unwrap()
        );
    }

    #[test]
    fn each_generation_moves_items_in_its_own_numbers() {
        let pick_up = Request::MoveItem {
            from: InventorySlot(22),
            to: InventorySlot::CURSOR,
            quantity: MoveQuantity::Whole,
        };
        assert_eq!(
            titanium(&pick_up, PLAYER).unwrap(),
            inventory::titanium_move(
                InventorySlot(22),
                InventorySlot::CURSOR,
                MoveQuantity::Whole
            )
            .unwrap()
        );
        let mac = eqmac(&pick_up, PLAYER).unwrap();
        assert_eq!(mac.opcode, 0x2c41);
        assert_eq!(&mac.body[4..8], &[0; 4]);
        let charm = Request::MoveItem {
            from: InventorySlot(0),
            to: InventorySlot::CURSOR,
            quantity: MoveQuantity::Whole,
        };
        assert!(eqmac(&charm, PLAYER).is_err());
    }

    #[test]
    fn each_generation_departs_and_answers_a_zone_offer_its_own_way() {
        let answer = |instance_id| Request::AnswerZoneOffer {
            zone_id: 4,
            instance_id,
            position: crate::world::Position::default(),
            reason: 0,
        };
        assert_eq!(
            eqmac(&answer(0), PLAYER).unwrap(),
            crate::quarm::zone_change("Tester", 4, 0).unwrap()
        );
        // EQMac has no instances.
        assert!(eqmac(&answer(2), PLAYER).is_err());
        assert_eq!(
            titanium(&Request::SaveOnZone(zoning::Save::default()), PLAYER).unwrap(),
            crate::zoning::titanium_save_on_zone(&zoning::Save::default())
        );
        assert_eq!(
            titanium(&Request::Depart, PLAYER).unwrap(),
            crate::zoning::titanium_depart(7)
        );
        assert_eq!(
            eqmac(&Request::SaveOnZone(zoning::Save::default()), PLAYER).unwrap(),
            crate::quarm::save_on_zone()
        );
        assert_eq!(
            eqmac(&Request::Depart, PLAYER).unwrap(),
            crate::quarm::depart(7)
        );
    }

    #[test]
    fn only_the_eqmac_client_drops_invisibility_as_its_program_does() {
        use crate::buffs::InvisibilityStep;
        let state = eqmac(
            &Request::DropInvisibility(InvisibilityStep::State(0)),
            PLAYER,
        )
        .unwrap();
        assert_eq!(state.opcode, crate::quarm::ZONE_SPAWN_APPEARANCE);
        assert_eq!(state.body, [7, 0, 3, 0, 0, 0, 0, 0]);
        let fade = InvisibilityStep::Fade {
            slot: 1,
            spell_id: 42,
        };
        let buff = eqmac(&Request::DropInvisibility(fade), PLAYER).unwrap();
        assert_eq!(buff.opcode, crate::buffs::EQMAC_BUFF_OPCODE);
        assert_eq!(buff.body, crate::buffs::eqmac_fade(7, fade).unwrap());
        assert!(titanium(&Request::DropInvisibility(fade), PLAYER).is_err());
    }

    #[test]
    fn only_the_eqmac_client_reports_that_it_bled_out() {
        assert_eq!(
            eqmac(&Request::BledOut { level: 34 }, PLAYER).unwrap(),
            crate::quarm::bled_out(7, 34).unwrap()
        );
        // Without a spawn there is no one to report.
        let unspawned = Sender {
            spawn_id: None,
            ..PLAYER
        };
        assert!(eqmac(&Request::BledOut { level: 34 }, unspawned).is_err());
        assert!(titanium(&Request::BledOut { level: 34 }, PLAYER).is_err());
    }

    #[test]
    fn only_titanium_sends_the_task_requests() {
        use crate::tasks::{TaskKind, TaskSlot};
        let requests = [
            Request::AcceptTask {
                kind: TaskKind::Quest,
                task_id: 9,
                giver: 77,
            },
            Request::CancelTask(TaskSlot {
                kind: TaskKind::Quest,
                slot: 3,
            }),
            Request::TaskHistory(2),
            Request::TaskTimers,
        ];
        let opcodes: Vec<_> = requests
            .iter()
            .map(|request| titanium(request, PLAYER).unwrap().opcode)
            .collect();
        assert_eq!(opcodes, [0x207f, 0x3ba8, 0x5df4, 0x6a1d]);
        for request in &requests {
            assert!(eqmac(request, PLAYER).is_err(), "{request:?}");
        }
    }

    #[test]
    fn each_generation_sends_a_shield_in_its_own_width() {
        let packet = titanium(&Request::Shield(42), PLAYER).unwrap();
        assert_eq!((packet.opcode, packet.body), (0x3fe6, vec![42, 0, 0, 0]));
        assert!(titanium(&Request::Shield(0), PLAYER).is_err());
        // TAKP's and Quarm's 2-byte `Shielding_Struct` (6.603 G2).
        let packet = eqmac(&Request::Shield(0x0142), PLAYER).unwrap();
        assert_eq!((packet.opcode, packet.body), (0x4b42, vec![0x42, 0x01]));
        assert!(eqmac(&Request::Shield(0), PLAYER).is_err());
    }

    #[test]
    fn each_generation_asks_for_a_surname_in_the_same_record_under_its_own_opcode() {
        let body = crate::names::surname_request(PLAYER.name, "Exemplum").unwrap();
        for (packet, opcode) in [
            (
                titanium(&Request::Surname("Exemplum".into()), PLAYER),
                0x4668,
            ),
            (eqmac(&Request::Surname("Exemplum".into()), PLAYER), 0xc441),
        ] {
            let packet = packet.unwrap();
            assert_eq!((packet.opcode, &packet.body), (opcode, &body));
        }
        assert!(titanium(&Request::Surname("Ex\tample".into()), PLAYER).is_err());
        // A quoted word's spaces go (read R49-10).
        assert!(titanium(&Request::Surname("Ex ample".into()), PLAYER).is_ok());
        assert!(eqmac(&Request::Surname(String::new()), PLAYER).is_err());
    }

    #[test]
    fn each_generation_sends_its_own_aa_action() {
        use crate::alternate_advancement::Action;
        let train = Request::AlternateAdvancement(Action::Train { rank_id: 9 });
        let sent = titanium(&train, PLAYER).unwrap();
        assert_eq!(
            (sent.opcode, sent.body),
            (0x0681, vec![3, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
        );
        assert!(titanium(
            &Request::AlternateAdvancement(Action::Split { percentage: 101 }),
            PLAYER
        )
        .is_err());
        // TAKP's is 256 bytes of text on its own opcode (N4).
        let text = eqmac(&train, PLAYER).unwrap();
        assert_eq!((text.opcode, text.body.len()), (0x1442, 256));
        assert_eq!(&text.body[..6], b"buy 9\0");
    }

    #[test]
    fn titanium_and_eqmac_search_the_bazaar_and_only_titanium_browses() {
        let criteria = crate::bazaar::BazaarCriteria::default();
        let searched = titanium(&Request::BazaarSearch(criteria.clone()), PLAYER).unwrap();
        assert_eq!(
            (searched.opcode, searched.body.len()),
            (0x1ee9, crate::bazaar::SEARCH_LENGTH)
        );
        let welcomed = titanium(&Request::BazaarWelcome, PLAYER).unwrap();
        assert_eq!(
            (welcomed.opcode, welcomed.body),
            (0x1ee9, crate::bazaar::welcome())
        );
        let long = crate::bazaar::BazaarCriteria {
            name: "a".repeat(64),
            ..criteria.clone()
        };
        assert!(titanium(&Request::BazaarSearch(long), PLAYER).is_err());
        let mac = eqmac(&Request::BazaarSearch(criteria.clone()), PLAYER).unwrap();
        assert_eq!(
            (mac.opcode, mac.body),
            (0x1142, crate::bazaar::eqmac::search(&criteria).unwrap())
        );
        let mac_welcome = eqmac(&Request::BazaarWelcome, PLAYER).unwrap();
        assert_eq!(
            (mac_welcome.opcode, mac_welcome.body),
            (0x1142, crate::bazaar::eqmac::welcome())
        );
        let bounded = crate::bazaar::BazaarCriteria {
            min_level: Some(10),
            ..criteria
        };
        assert!(eqmac(&Request::BazaarSearch(bounded), PLAYER).is_err());
        let clicked = titanium(&Request::BrowseTrader(312), PLAYER).unwrap();
        assert_eq!(
            (clicked.opcode, clicked.body),
            (0x35e8, crate::bazaar::browse(312).to_vec())
        );
        let ended = titanium(&Request::EndBrowse(312), PLAYER).unwrap();
        assert_eq!(
            (ended.opcode, ended.body),
            (0x524e, crate::bazaar::end_browse(312).to_vec())
        );
        assert!(eqmac(&Request::BrowseTrader(312), PLAYER).is_err());
        assert!(eqmac(&Request::EndBrowse(312), PLAYER).is_err());
    }

    #[test]
    fn only_titanium_saves_sets_and_sets_the_belt() {
        let saved = titanium(
            &Request::SaveBandolier {
                set: 1,
                name: "Melee".into(),
            },
            PLAYER,
        )
        .unwrap();
        assert_eq!(
            (saved.opcode, saved.body),
            (0x6f0c, crate::bandolier::save(1, "Melee").unwrap().to_vec())
        );
        let cleared = titanium(&Request::ClearBandolier(1), PLAYER).unwrap();
        assert_eq!(cleared.body, crate::bandolier::clear(1).unwrap().to_vec());
        let belt = titanium(
            &Request::SetPotionBelt {
                slot: 2,
                item_id: 13_005,
            },
            PLAYER,
        )
        .unwrap();
        assert_eq!((belt.opcode, belt.body.len()), (0x0719, 12));
        let swapped = titanium(&Request::SwapBandolier(3), PLAYER).unwrap();
        assert_eq!(
            (swapped.opcode, swapped.body),
            (0x6f0c, crate::bandolier::swap_to(3).unwrap().to_vec())
        );
        assert!(titanium(&Request::SwapBandolier(4), PLAYER).is_err());
        assert!(eqmac(&Request::SwapBandolier(0), PLAYER).is_err());
        assert!(titanium(&Request::ClearBandolier(4), PLAYER).is_err());
        assert!(eqmac(&Request::ClearBandolier(1), PLAYER).is_err());
        assert!(eqmac(
            &Request::ClearPotionBelt {
                slot: 0,
                item_id: 1
            },
            PLAYER
        )
        .is_err());
    }

    #[test]
    fn only_titanium_asks_for_and_chooses_titles() {
        let asked = titanium(&Request::RequestTitles, PLAYER).unwrap();
        assert_eq!((asked.opcode, asked.body.len()), (0x5eba, 0));
        let chosen = titanium(
            &Request::SetTitle {
                title_id: 7,
                suffix: true,
            },
            PLAYER,
        )
        .unwrap();
        assert_eq!(
            (chosen.opcode, chosen.body),
            (0x1f22, vec![1, 0, 0, 0, 7, 0, 0, 0])
        );
        assert!(eqmac(&Request::RequestTitles, PLAYER).is_err());
        assert!(eqmac(
            &Request::SetTitle {
                title_id: 7,
                suffix: false
            },
            PLAYER
        )
        .is_err());
    }

    #[test]
    fn titanium_requests_are_the_packets_the_codecs_build() {
        assert_eq!(
            titanium(&Request::Camp, PLAYER).unwrap(),
            command::titanium_camp()
        );
        // `/yell` and `/stopcast` send empty bodies (R45); the Mac-era
        // generation sends an empty yell at its own opcode (6.604 E2).
        for (request, opcode) in [(Request::YellForHelp, 0x61ef), (Request::StopSong, 0x4839)] {
            let packet = titanium(&request, PLAYER).unwrap();
            assert_eq!((packet.opcode, packet.body.len()), (opcode, 0));
        }
        let packet = eqmac(&Request::YellForHelp, PLAYER).unwrap();
        assert_eq!((packet.opcode, packet.body.len()), (0xda41, 0));
        // An emote's animation is the 4-byte animation record (R47).
        let packet = titanium(&Request::EmoteAnimation(0x4d), PLAYER).unwrap();
        assert_eq!(packet.opcode, 0x2acf);
        assert_eq!(&packet.body[2..], [10, 0x4d]);
        assert!(eqmac(&Request::EmoteAnimation(0x4d), PLAYER).is_err());
        assert_eq!(
            titanium(&Request::Posture(Posture::Sitting), PLAYER).unwrap(),
            command::titanium_posture(7, Posture::Sitting).unwrap()
        );
        assert_eq!(
            titanium(
                &Request::Pet {
                    command: PetCommand::Attack,
                    target: Some(9),
                },
                PLAYER
            )
            .unwrap(),
            pets::command(PetCommand::Attack, Some(9))
        );
        assert_eq!(
            titanium(&Request::DropCorpse { corpse: None }, PLAYER).unwrap(),
            corpses::release(None).unwrap()
        );
        // Training names the guildmaster, and opening and leaving the player.
        assert_eq!(
            titanium(&Request::OpenTraining(42), PLAYER).unwrap(),
            training::titanium_open(42, 7)
        );
        assert_eq!(
            titanium(
                &Request::Train {
                    trainer: 42,
                    skill: 30
                },
                PLAYER
            )
            .unwrap(),
            training::titanium_train(42, 30).unwrap()
        );
        assert_eq!(
            titanium(
                &Request::Practice {
                    trainer: 42,
                    subject: training::TrainingSubject::Language(5)
                },
                PLAYER
            )
            .unwrap(),
            training::titanium_practice(42, training::TrainingSubject::Language(5)).unwrap()
        );
        assert_eq!(
            titanium(&Request::EndTraining(42), PLAYER).unwrap(),
            training::titanium_end(42, 7)
        );
        // What a packet cannot carry is refused, as the codec refuses it.
        assert!(titanium(&Request::Trade(7), PLAYER).is_err());
        // A packet repeating the sender takes their name and spawn.
        for held in [None, Some(doors::HeldItem { id: 1001, skill: 0 })] {
            let door = Request::ClickDoor { door_id: 3, held };
            assert_eq!(
                titanium(&door, PLAYER).unwrap(),
                doors::titanium_click(3, 7, held)
            );
            // EQMac's program refuses a click with an item on the cursor.
            assert_eq!(eqmac(&door, PLAYER).is_ok(), held.is_none());
        }
        assert_eq!(
            titanium(&Request::SummonCorpse("Tester's corpse4".into()), PLAYER).unwrap(),
            corpses::summon("Tester's corpse4", "Tester").unwrap()
        );
        // Without a spawn, those packets are refused as their codecs refuse
        // a zero spawn.
        let unspawned = Sender {
            spawn_id: None,
            ..PLAYER
        };
        assert!(titanium(&Request::AcceptTrade { with: 42 }, unspawned).is_err());
        // Another player's request is taken, or answered as busy.
        let answer = |busy| Request::AnswerTrade { asker: 50, busy };
        assert_eq!(
            titanium(&answer(false), PLAYER).unwrap(),
            exchange::acknowledge(7, 50).unwrap()
        );
        assert_eq!(
            titanium(&answer(true), PLAYER).unwrap(),
            exchange::busy(7, 50).unwrap()
        );
    }

    #[test]
    fn mac_training_requests_name_the_player() {
        // TAKP and Quarm (6.597): the Mac program's own bytes, the player's
        // spawn in every one; the session holds languages, not the codec.
        assert_eq!(
            eqmac(&Request::OpenTraining(42), PLAYER).unwrap(),
            training::eqmac_open(42, 7)
        );
        assert_eq!(
            eqmac(
                &Request::Train {
                    trainer: 42,
                    skill: 30
                },
                PLAYER
            )
            .unwrap(),
            training::eqmac_practice(42, 7, training::TrainingSubject::Skill(30)).unwrap()
        );
        assert_eq!(
            eqmac(
                &Request::Practice {
                    trainer: 42,
                    subject: training::TrainingSubject::Language(5)
                },
                PLAYER
            )
            .unwrap(),
            training::eqmac_practice(42, 7, training::TrainingSubject::Language(5)).unwrap()
        );
        assert_eq!(
            eqmac(&Request::EndTraining(42), PLAYER).unwrap(),
            training::eqmac_end(42, 7)
        );
    }
    #[test]
    fn each_generation_reports_the_worlds_damage_in_its_own_packet() {
        let fall = Request::EnvironmentalDamage {
            hazard: Hazard::Falling,
            amount: 160,
        };
        assert_eq!(
            titanium(&fall, PLAYER).unwrap(),
            hazards::titanium_damage(7, Hazard::Falling, 160).unwrap()
        );
        assert_eq!(
            eqmac(&fall, PLAYER).unwrap(),
            hazards::eqmac_damage(7, Hazard::Falling, 160).unwrap()
        );
        let unspawned = Sender {
            spawn_id: None,
            ..PLAYER
        };
        assert!(titanium(&fall, unspawned).is_err());
        assert!(eqmac(&fall, unspawned).is_err());
    }
}
