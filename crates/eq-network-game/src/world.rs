//! Typed, credential-free world state decoded from Titanium zone packets.

use anyhow::{ensure, Context, Result};
use serde::Serialize;

/// Server appearance stance, distinct from one-shot animation IDs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum PostureState {
    /// Normal upright stance.
    Standing,
    /// Animation frozen by the server.
    Frozen,
    /// Kneeling to loot.
    Looting,
    /// Seated stance.
    Sitting,
    /// Crouched stance.
    Ducking,
    /// Prone stance, which does not by itself establish death.
    Lying,
    /// Unsupported server posture, preserving its wire value.
    Unknown(u32),
}

impl From<u32> for PostureState {
    fn from(value: u32) -> Self {
        match value {
            100 => Self::Standing,
            102 => Self::Frozen,
            105 => Self::Looting,
            110 => Self::Sitting,
            111 => Self::Ducking,
            115 => Self::Lying,
            value => Self::Unknown(value),
        }
    }
}

/// Coordinates use the server's X/Y axes and its 0..512 heading convention.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Position {
    /// North/south coordinate.
    pub x: f32,
    /// East/west coordinate.
    pub y: f32,
    /// Server reference height (not necessarily foot height).
    pub z: f32,
    /// Direction in one 512-unit revolution.
    pub heading: f32,
}

/// Base profile attributes; bonuses and dialect-specific caps are applied separately.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct BaseAttributes {
    /// Profile strength before equipment and spell bonuses.
    pub strength: i32,
    /// Profile stamina attribute, distinct from current endurance or fatigue.
    pub stamina: i32,
    /// Profile charisma before bonuses.
    pub charisma: i32,
    /// Profile dexterity before bonuses.
    pub dexterity: i32,
    /// Profile intelligence before bonuses.
    pub intelligence: i32,
    /// Profile agility before bonuses.
    pub agility: i32,
    /// Profile wisdom before bonuses.
    pub wisdom: i32,
}

/// Character identity and saved state needed by a graphical consumer.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlayerState {
    /// Character name as the profile spells it.
    pub name: String,
    /// Base profile attributes, not effective stats or resource capacities.
    pub base_attributes: Option<BaseAttributes>,
    /// The active spawn, scoped to this zone connection.
    pub spawn_id: u16,
    /// Numeric race identifier.
    pub race: u32,
    /// Class decoded from the profile, when supported.
    pub class: Option<u32>,
    /// Profile deity identifier, when decoded; zero/unavailable values are None.
    pub deity: Option<u32>,
    /// Indexed profile skill values, when decoded; IDs retain the protocol's numbering.
    pub skills: Option<Vec<u32>>,
    /// Numeric gender identifier.
    pub gender: u32,
    /// Current level.
    pub level: u8,
    /// Saved server position.
    pub position: Position,
    /// Current mana; maximum is unknown until separately established.
    pub mana: u32,
    /// Current endurance, or None when the protocol does not provide it.
    pub endurance: Option<u32>,
    /// Eight memorized spell IDs; empty slots are None.
    pub memorized_spells: [Option<u32>; 8],
    /// Remaining per-gem reuse milliseconds from the admission profile, when decoded.
    /// None means the dialect has not supplied this information, not that all gems are ready.
    pub spell_refresh_ms: Option<[u32; 8]>,
    /// Model size announced in the player's spawn; zero means the racial default.
    pub size: f32,
    /// Server protocol walk-speed value; not world units per second.
    pub walk_speed: f32,
    /// Server protocol run-speed value; not world units per second.
    pub run_speed: f32,
    /// HP percentage, if valid in the spawn record.
    pub hp_percent: Option<u8>,
    /// Worn gear and features from the own spawn record; default where the
    /// dialect does not report them.
    pub appearance: crate::appearance::Appearance,
}

impl PlayerState {
    /// Updates only an existing skill slot; unknown IDs never grow profile storage.
    pub fn apply_skill(&mut self, skill_id: u32, value: u32) {
        if let Some(slot) = usize::try_from(skill_id).ok().and_then(|index| {
            self.skills
                .as_mut()
                .and_then(|skills| skills.get_mut(index))
        }) {
            *slot = value;
        }
    }
}

/// Server classification, retaining values not understood by this client.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum SpawnKind {
    /// Another player.
    Player,
    /// Non-player creature or character.
    Npc,
    /// A player's corpse.
    PlayerCorpse,
    /// A non-player corpse.
    NpcCorpse,
    /// A future or server-specific classification.
    Unknown(u8),
}

impl SpawnKind {
    /// What an entity becomes when it dies; Titanium corpses keep the spawn ID.
    #[must_use]
    pub const fn corpse(self) -> Self {
        match self {
            Self::Player => Self::PlayerCorpse,
            Self::Npc => Self::NpcCorpse,
            other => other,
        }
    }
}

/// A zone entity. Asset selection stays outside the protocol layer.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SpawnState {
    /// Server class/service identifier; absent when the dialect decoder has no verified field.
    pub class: Option<u8>,
    /// Zone-local identifier, reused only after removal.
    pub spawn_id: u16,
    /// Server-supplied display name.
    pub name: String,
    /// Player, creature, or corpse classification.
    pub kind: SpawnKind,
    /// Race/model identifier.
    pub race: u32,
    /// Model gender identifier.
    pub gender: u32,
    /// Authoritative location and direction.
    pub position: Position,
    /// Motion at the last report, in EQ units per second along X, Y and Z; zero
    /// when standing still or not reported.
    pub velocity: [f32; 3],
    /// Requested model size; zero selects the racial default.
    pub size: f32,
    /// Server visibility flag; consumers must not expose invisible entities.
    pub invisible: bool,
    /// Worn gear and features; default where the dialect does not report them.
    pub appearance: crate::appearance::Appearance,
}

/// Decode a decrypted Titanium spawn batch, without accepting partial records.
///
/// # Errors
/// Rejects malformed lengths, invalid identifiers, and non-finite sizes.
pub fn titanium_spawns(body: &[u8]) -> Result<Vec<SpawnState>> {
    ensure!(
        body.len().is_multiple_of(385) && body.len() / 385 <= 4096,
        "invalid Titanium spawn batch length"
    );
    body.as_chunks::<385>()
        .0
        .iter()
        .map(|record| {
            let spawn_id = u16::try_from(word(record, 340))?;
            ensure!(spawn_id != 0, "invalid spawn ID");
            let size = float(record, 75)?;
            ensure!(size >= 0.0, "negative spawn size");
            let name = &record[7..71];
            let end = name.iter().position(|b| *b == 0).unwrap_or(name.len());
            let (position, velocity) =
                titanium_motion([94, 98, 102, 106, 110].map(|at| word(record, at)))?;
            Ok(SpawnState {
                class: Some(record[331]),
                spawn_id,
                name: String::from_utf8_lossy(&name[..end]).into_owned(),
                kind: match record[83] {
                    0 => SpawnKind::Player,
                    1 => SpawnKind::Npc,
                    2 => SpawnKind::PlayerCorpse,
                    3 => SpawnKind::NpcCorpse,
                    other => SpawnKind::Unknown(other),
                },
                race: word(record, 284),
                gender: u32::from(record[334]),
                size,
                invisible: record[84] != 0,
                position,
                velocity,
                appearance: crate::appearance::titanium_spawn(record),
            })
        })
        .collect()
}

/// Coins in each denomination.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Coins {
    /// Platinum pieces.
    pub platinum: u32,
    /// Gold pieces.
    pub gold: u32,
    /// Silver pieces.
    pub silver: u32,
    /// Copper pieces.
    pub copper: u32,
}

impl Coins {
    /// Total value in copper pieces.
    #[must_use]
    pub fn total_copper(&self) -> u64 {
        u64::from(self.platinum) * 1000
            + u64::from(self.gold) * 100
            + u64::from(self.silver) * 10
            + u64::from(self.copper)
    }
}

/// Carried coins from the Titanium player profile.
///
/// # Errors
/// Rejects profiles with an unexpected length.
pub fn titanium_coins(profile: &[u8]) -> Result<Coins> {
    ensure!(profile.len() == 19592, "unexpected Titanium profile layout");
    Ok(Coins {
        platinum: word(profile, 4428),
        gold: word(profile, 4432),
        silver: word(profile, 4436),
        copper: word(profile, 4440),
    })
}

/// Progress of a camp request; the server confirms only the final logout.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum CampStatus {
    /// `OP_Camp` was sent; logout follows after the preparation time.
    Preparing,
    /// Standing or moving abandoned the attempt before logout.
    Abandoned,
    /// Preparation finished and `OP_Logout` was sent.
    LoggingOut,
    /// The zone connection ended; character selection follows.
    Camped,
    /// Local validation rejected the request before transmission.
    Rejected(String),
}

/// Something a zone session lets the player do. Which ones a session offers
/// depends on the server type and on what its client generation has been
/// built for; a front end greys out or hides the rest.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Capability {
    /// Casting memorized spells.
    Casting,
    /// Memorizing, scribing and forgetting spells.
    Spellbook,
    /// Moving and using items in the inventory and the bank.
    Inventory,
    /// Buying from and selling to merchants.
    Trading,
    /// Handing items to NPCs and trading with other players.
    Giving,
    /// Walking, running, sitting and standing.
    Moving,
    /// Jumping and falling, which the server takes from the client.
    Falling,
    /// Choosing a target.
    Targeting,
    /// Considering and attacking.
    Combat,
    /// Looting corpses.
    Looting,
    /// Talking on chat channels and inspecting linked items.
    Talking,
    /// Camping to the character list.
    Camping,
    /// Opening doors.
    Doors,
    /// Picking up items from the ground.
    GroundItems,
    /// Crossing zone lines and being moved between zones.
    Zoning,
    /// Using abilities: kick, bash, taunt, hide, sneak, forage and the like.
    Abilities,
    /// Asking who is online.
    Who,
}

impl Capability {
    /// Every capability, in order: what a session offers when its server and
    /// client generation support everything.
    pub const ALL: [Self; 17] = [
        Self::Casting,
        Self::Spellbook,
        Self::Inventory,
        Self::Trading,
        Self::Giving,
        Self::Moving,
        Self::Falling,
        Self::Targeting,
        Self::Combat,
        Self::Looting,
        Self::Talking,
        Self::Camping,
        Self::Doors,
        Self::GroundItems,
        Self::Zoning,
        Self::Abilities,
        Self::Who,
    ];
}

/// Changes delivered to a graphical consumer, independent of its rendering engine.
///
/// Exhaustive on purpose: a front end should handle every kind of news, so a
/// new variant names itself in the consumer's build instead of falling into a
/// wildcard arm.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum WorldEvent {
    /// A server spell action; icon flags do not establish an authoritative buff slot.
    SpellEffect(crate::buffs::SpellEffect),
    /// Admission buff slots; holes retain their original indexes.
    BuffSnapshot(Vec<Option<crate::buffs::Buff>>),
    /// A server-owned slot replacement or fade for an entity.
    Buff(crate::buffs::BuffUpdate),
    /// The server's answer to a character creation request.
    CharacterCreation {
        /// Requested name.
        name: String,
        /// Whether the character now exists; a new list follows on success.
        accepted: bool,
    },
    /// The world's short name from its log-server settings. The official client
    /// names per-character files after it, such as `UI_<character>_<short name>.ini`.
    WorldName {
        /// For example `P1999Green`.
        short_name: String,
    },
    /// Available characters for this world connection; no zone has been entered.
    CharacterSelection {
        /// Fresh identity invalidates choices queued for an older connection.
        selection_id: u64,
        /// Occupied server slots, possibly empty on a new account.
        characters: Vec<crate::characters::CharacterChoice>,
    },
    /// Local item-use validation/submission result, not proof that its effect landed.
    ItemUseAction {
        /// Admission from the request.
        session_id: u64,
        /// Caller-generated correlation ID.
        request_id: u64,
        /// None means transport submission succeeded; otherwise a local rejection.
        error: Option<String>,
    },
    /// Server-defined doors and their latest action instructions.
    Doors(crate::doors::DoorUpdate),
    /// Result of validating/submitting a door request, not proof that the door opened.
    DoorAction {
        /// Current admission.
        session_id: u64,
        /// Requested door identifier.
        door_id: u8,
        /// Local rejection; None means submission to transport succeeded.
        error: Option<String>,
    },
    /// Items on the ground and world containers.
    Objects(crate::objects::ObjectUpdate),
    /// Result of validating/submitting a pickup, not proof that the item was taken.
    ObjectAction {
        /// Current admission.
        session_id: u64,
        /// Requested object.
        drop_id: u32,
        /// Local rejection; None means submission to transport succeeded.
        error: Option<String>,
    },
    /// A boundary request failed local admission or destination validation.
    ZoneLineRejected {
        /// Admission that submitted the request.
        session_id: u64,
        /// Credential-free reason for the rejection.
        reason: String,
    },
    /// Complete indexed spellbook for the current admission.
    SpellBook(crate::spells::SpellBook),
    /// Local progress of a spellbook request, separate from server confirmation.
    BookAction(crate::spells::BookActionStatus),
    /// Server-driven spell and casting state.
    Spell(crate::spells::SpellUpdate),
    /// Server-reported appearance posture.
    Posture {
        /// Entity whose stance changed.
        spawn_id: u16,
        /// Authoritative stance, independent of local requests.
        posture: PostureState,
    },
    /// Local transport submission awaiting acknowledgement; never implies casting began.
    CastPending {
        /// Zone admission that owns this request.
        session_id: u64,
        /// Pending spell, or None when acknowledged, interrupted, or timed out.
        spell_id: Option<u32>,
    },
    /// A cast request failed local validation; existing cast state is unchanged.
    CastRejected {
        /// Admission identifier from the rejected request.
        session_id: u64,
        /// Spell requested by the caller.
        spell_id: u32,
        /// Human-readable local rejection reason.
        reason: String,
    },
    /// Result of local validation and transport submission, not a server acknowledgment.
    InventoryAction {
        /// Admission identifier from the request.
        session_id: u64,
        /// Inventory revision the request referenced.
        revision: u64,
        /// Validation failure; None means the request was handed to transport.
        error: Option<String>,
    },
    /// Server inventory snapshot or incremental update.
    Inventory(crate::inventory::InventoryUpdate),
    /// Local session movement gate; None clears pending input and disables walking.
    MotionState {
        /// Current zone admission.
        session_id: u64,
        /// Calibrated world units per second, if movement is enabled.
        units_per_second: Option<f32>,
        /// Independently calibrated backward speed; None disables backing up.
        backward_units_per_second: Option<f32>,
        /// Optional separately calibrated walking speed.
        walk_units_per_second: Option<f32>,
        /// Optional separately calibrated sideways speed.
        strafe_units_per_second: Option<f32>,
        /// Whether this server's session accepts `MovementMode::Fall` samples.
        falls: bool,
    },
    /// Motion submitted to transport (not an acknowledgment by the server), or a
    /// sample the local movement guard refused.
    MotionSent {
        /// Current zone admission.
        session_id: u64,
        /// Position accepted by local motion validation; after a refusal, the last
        /// accepted one.
        position: Position,
        /// Why the guard refused the sample; None when it was sent.
        refused: Option<String>,
    },
    /// Server-provided item definition for a clicked chat link.
    ItemDetails(crate::items::ItemDetails),
    /// Server death notification; consumers compare its ID with their own spawn.
    Death(crate::zoning::Death),
    /// A transfer request is in progress; stop all old-zone input until its reply.
    ZoneTransfer(crate::zoning::ZoneOffer),
    /// The server rejected the pending transfer.
    ZoneTransferRejected {
        /// Admission whose pending transfer was rejected.
        session_id: u64,
        /// Server response code or explicit position-restoring cancellation.
        reason: crate::zoning::ZoneRejection,
    },
    /// Selection request was handed to transport; this is not a server acknowledgment.
    TargetSent(Option<u16>),
    /// Local validation rejected a target request before transmission.
    TargetRejected {
        /// Admission that originated the request.
        session_id: u64,
        /// Requested selection, or clearing the current target.
        spawn_id: Option<u16>,
        /// Validation reason; this is not a server rejection packet.
        reason: String,
    },
    /// A spawn's current health percentage.
    HealthPercent {
        /// Zone-local entity identifier.
        spawn_id: u16,
        /// Validated 0..100 percentage.
        percent: u8,
    },
    /// Initial or incremental spawn batch; IDs replace existing entries.
    Spawns(Vec<SpawnState>),
    /// One texture slot of a spawn's worn gear changed, the player's own included.
    WearChange(crate::appearance::WearChange),
    /// Server visibility changed; consumers must remove invisible entities.
    Visibility {
        /// Zone-local entity identifier.
        spawn_id: u16,
        /// Whether the server marks this entity invisible.
        invisible: bool,
    },
    /// Current mana on protocols without a combined endurance update.
    Mana(u32),
    /// An entity left the zone or was removed.
    Despawn(u16),
    /// A fully admitted zone session and its initial player state.
    Entered {
        /// What this session lets the player do; front ends grey out or hide
        /// the rest.
        capabilities: Vec<Capability>,
        /// Unique connection identifier, never reused after reconnect.
        session_id: u64,
        /// Zone asset short name.
        zone: String,
        /// Initial character state.
        player: Box<PlayerState>,
        /// The zone's far clip distance. The official client draws and targets
        /// nothing past it, and servers log targets beyond it as possible cheats.
        /// None when the dialect has not reported it.
        far_clip: Option<f32>,
    },
    /// Server-reported position, including corrections for the local player.
    Position {
        /// Spawn identifier.
        spawn_id: u16,
        /// Authoritative position.
        position: Position,
        /// Motion at this report, in EQ units per second along X, Y and Z; zero
        /// when standing still or not reported. Clients carry it forward until
        /// the next report, as the official client does.
        velocity: [f32; 3],
    },
    /// Current/max HP received from the server.
    HitPoints {
        /// Spawn identifier.
        spawn_id: u16,
        /// Current HP; negative while dying, or below what equipped items add
        /// when those are left out.
        current: i32,
        /// Maximum HP, never negative.
        maximum: i32,
        /// Both values leave out the HP equipped items add, which the client adds
        /// back itself: Titanium's own update, where `EQEmu` subtracts
        /// `itembonuses.HP` (zone/mob.cpp `Mob::SendHPUpdate`).
        without_items: bool,
    },
    /// Current mana and endurance; maxima remain unknown.
    Resources {
        /// Current mana.
        mana: u32,
        /// Current endurance.
        endurance: u32,
    },
    /// Experience progress within this level, on the Titanium 0..330 scale.
    Experience(u32),
    /// Own-character level change, including experience within the new level.
    Level {
        /// New server level.
        current: u8,
        /// Previous server level; may exceed the new level after experience loss.
        previous: u8,
        /// Experience on the Titanium 0..330 scale.
        experience: u32,
    },
    /// The server's assessment of a considered entity.
    Consideration(crate::combat::Consideration),
    /// Camp progress for the current admission.
    Camp(CampStatus),
    /// Carried coins, from admission or a server money update.
    Coins(Coins),
    /// The coins outside the purse: from the profile at admission, and after
    /// every change the session makes or hears of (the purse's own changes
    /// come as [`WorldEvent::Coins`]).
    CoinsElsewhere {
        /// On the cursor.
        cursor: Coins,
        /// In the bank.
        bank: Coins,
        /// In the open give or trade window, put there by the player.
        given: Coins,
        /// In the trade, put there by the other player.
        offered: Coins,
    },
    /// A coin move that was not sent, and why.
    CoinsRefused {
        /// Admission from the request.
        session_id: u64,
        /// Why it was not sent.
        reason: String,
    },
    /// Corpse loot session changes.
    Loot(crate::loot::LootUpdate),
    /// Merchant window changes.
    Merchant(crate::merchant::MerchantUpdate),
    /// A purchase or sale that was not sent, or that the merchant never answered.
    MerchantRefused {
        /// Admission from the request.
        session_id: u64,
        /// Why nothing was bought or sold.
        reason: String,
    },
    /// Give and trade window changes.
    Exchange(crate::exchange::ExchangeUpdate),
    /// A request to trade that was not sent, or that went unanswered.
    ExchangeRefused {
        /// Admission from the request.
        session_id: u64,
        /// Why no window opened.
        reason: String,
    },
    /// How fed and watered the player is, from the profile and the
    /// server's updates.
    Nourishment(crate::food::Nourishment),
    /// The player turned hungry or thirsty with nothing in the inventory
    /// the session eats or drinks on its own.
    NothingToEat {
        /// Why a hungry player went without food.
        food: Option<crate::food::Shortage>,
        /// Why a thirsty player went without drink.
        water: Option<crate::food::Shortage>,
    },
    /// An item was not eaten or drunk, and why.
    ConsumeRefused {
        /// Admission from the request.
        session_id: u64,
        /// Why not.
        reason: String,
    },
    /// The player used an ability; its timer runs this long before the
    /// server takes the next use.
    AbilityUsed {
        /// Admission from the request.
        session_id: u64,
        /// The ability.
        ability: crate::abilities::Ability,
        /// How long its recovery takes, at most.
        ready_in: std::time::Duration,
    },
    /// An ability was not used, and why: the server would have ignored it.
    AbilityRefused {
        /// Admission from the request.
        session_id: u64,
        /// Why not.
        reason: String,
    },
    /// The world's answer to `/who all`.
    WhoList(crate::who::WhoList),
    /// A melee, skill or spell damage record for any nearby entities.
    Damage(crate::combat::Damage),
    /// Own-character skill update; unknown skill IDs remain available to consumers.
    Skill {
        /// Protocol skill index (22 is dual wield).
        skill_id: u32,
        /// New server skill value.
        value: u32,
    },
}

/// A profile heading on the 0..512 scale live movement uses. Project 1999 saves
/// headings on a 256-unit revolution (verified against the matching official
/// client profile and position sample); stock `EQEmu` saves the 512-unit heading
/// itself (`m_pp.heading = m_Position.w`).
#[must_use]
pub fn profile_heading(raw: f32, revolution: f32) -> f32 {
    (raw * 512.0 / revolution).rem_euclid(512.0)
}

/// Decode the profile and validated, decrypted own-spawn record. `revolution`
/// is the profile's heading scale, see [`profile_heading`].
///
/// # Errors
/// Rejects wrong layouts and non-finite coordinates or speeds.
pub fn titanium_player(profile: &[u8], spawn: &[u8], revolution: f32) -> Result<PlayerState> {
    ensure!(
        profile.len() == 19592 && spawn.len() == 385,
        "unexpected Titanium player layout"
    );
    let position = Position {
        x: float(profile, 13116)?,
        y: float(profile, 13120)?,
        z: float(profile, 13124)?,
        heading: profile_heading(float(profile, 13128)?, revolution),
    };
    let spawn_id = u16::try_from(word(spawn, 340))?;
    ensure!(spawn_id != 0, "invalid own-spawn ID");
    let size = float(spawn, 75)?;
    let walk_speed = float(spawn, 324)?;
    let run_speed = float(spawn, 233)?;
    ensure!(
        size >= 0.0 && walk_speed >= 0.0 && run_speed >= 0.0,
        "invalid spawn dimensions or speeds"
    );
    Ok(PlayerState {
        name: String::from_utf8_lossy(until_nul(&profile[12940..13004])).into_owned(),
        base_attributes: Some(BaseAttributes {
            strength: i32::try_from(word(profile, 2236))?,
            stamina: i32::try_from(word(profile, 2240))?,
            charisma: i32::try_from(word(profile, 2244))?,
            dexterity: i32::try_from(word(profile, 2248))?,
            intelligence: i32::try_from(word(profile, 2252))?,
            agility: i32::try_from(word(profile, 2256))?,
            wisdom: i32::try_from(word(profile, 2260))?,
        }),
        spawn_id,
        race: word(profile, 8),
        class: Some(word(profile, 12)).filter(|value| (1..=16).contains(value)),
        deity: Some(word(profile, 124)).filter(|value| *value != 0),
        skills: Some(
            (0..100)
                .map(|index| word(profile, 4460 + index * 4))
                .collect(),
        ),
        gender: word(profile, 4),
        level: profile[20],
        position,
        mana: word(profile, 2228),
        endurance: Some(word(profile, 6148)),
        spell_refresh_ms: Some(std::array::from_fn(|index| word(profile, 132 + index * 4))),
        memorized_spells: std::array::from_fn(|index| {
            let id = word(profile, 4360 + index * 4);
            (id != u32::MAX && id != 0xffff && id != 0).then_some(id)
        }),
        size,
        walk_speed,
        run_speed,
        hp_percent: (spawn[86] <= 100).then_some(spawn[86]),
        appearance: crate::appearance::titanium_spawn(spawn),
    })
}

/// Reads the far clip distance (`maxclip`) from a Titanium zone header
/// (`OP_NewZone`), a float at offset 516; None when missing or not positive.
#[must_use]
pub fn titanium_far_clip(new_zone: &[u8]) -> Option<f32> {
    let bytes = new_zone.get(516..520)?;
    let value = f32::from_le_bytes(bytes.try_into().ok()?);
    (value.is_finite() && value > 0.0).then_some(value)
}

/// Reads the world's short name from a Titanium log-server settings body
/// (`OP_LogServer`), which carries it NUL-terminated in 32 bytes at offset 32.
///
/// # Errors
/// Rejects truncated bodies and names that are empty or not plain ASCII.
pub fn titanium_world_name(body: &[u8]) -> Result<String> {
    let field = body.get(32..64).context("truncated log-server settings")?;
    let name = until_nul(field);
    ensure!(
        !name.is_empty() && name.iter().all(u8::is_ascii_graphic),
        "invalid world short name"
    );
    Ok(String::from_utf8_lossy(name).into_owned())
}

/// Decode supported ongoing state updates. Unknown opcodes remain available to other codecs.
///
/// # Errors
/// Rejects truncated recognized packets rather than indexing arbitrary bytes.
pub fn titanium_update(opcode: u16, body: &[u8]) -> Result<Option<WorldEvent>> {
    if let Some(event) = titanium_views(opcode, body)? {
        return Ok(Some(event));
    }
    Ok(Some(match opcode {
        0x6a53 => WorldEvent::Buff(crate::buffs::titanium_update(body)?),
        0x7c32 => return appearance(body),
        0x6d44 => {
            ensure!(body.len() == 12, "invalid level update length");
            let current = u8::try_from(word(body, 0))?;
            let previous = u8::try_from(word(body, 4))?;
            ensure!(current != 0 && previous != 0, "invalid zero level");
            WorldEvent::Level {
                current,
                previous,
                experience: word(body, 8).min(330),
            }
        }
        0x6a93 => {
            ensure!(body.len() == 8, "invalid skill update length");
            WorldEvent::Skill {
                skill_id: word(body, 0),
                value: word(body, 4),
            }
        }
        0x667c => WorldEvent::ItemDetails(crate::items::response(body)?),
        0x3397 if body.get(..4) == Some(&[0, 0, 0, 0]) => {
            WorldEvent::ItemDetails(crate::items::response(body)?)
        }
        0x6160 => WorldEvent::Death(crate::zoning::death(body)?),
        crate::combat::CONSIDER_OPCODE => {
            WorldEvent::Consideration(crate::combat::consideration(body)?)
        }
        crate::combat::DAMAGE_OPCODE => WorldEvent::Damage(crate::combat::damage(body)?),
        crate::food::STAMINA_OPCODE => {
            WorldEvent::Nourishment(crate::food::decode(opcode, body)?.unwrap_or_default())
        }
        crate::who::RESPONSE_OPCODE => WorldEvent::WhoList(crate::who::decode(body)?),
        0x0695 => {
            ensure!(
                body.len() == 3 && body[2] <= 100,
                "invalid entity health update"
            );
            WorldEvent::HealthPercent {
                spawn_id: u16::from_le_bytes([body[0], body[1]]),
                percent: body[2],
            }
        }
        0x2e78 | 0x1860 => WorldEvent::Spawns(titanium_spawns(body)?),
        0x55bc => {
            ensure!(body.len() == 4, "invalid despawn length");
            WorldEvent::Despawn(u16::try_from(word(body, 0))?)
        }
        0x14cb => titanium_position(body)?,
        0x3a2b => p99_compact_position(body)?,
        crate::appearance::WEAR_CHANGE_OPCODE => {
            WorldEvent::WearChange(crate::appearance::titanium_wear_change(body)?)
        }
        0x3bcf => {
            ensure!(body.len() == 10, "invalid hit-point update length");
            // Both fields are signed: the server sends only the player's own HP,
            // less what equipped items add, so the current value is negative while
            // dying and also while below that item bonus.
            let [current, maximum] =
                [0, 4].map(|offset| i32::from_le_bytes(word(body, offset).to_le_bytes()));
            ensure!(maximum >= 0, "negative maximum hit points");
            WorldEvent::HitPoints {
                current,
                maximum,
                spawn_id: u16::from_le_bytes([body[8], body[9]]),
                without_items: true,
            }
        }
        0x4839 => {
            ensure!(body.len() >= 16, "truncated mana update");
            WorldEvent::Resources {
                mana: word(body, 0),
                endurance: word(body, 4),
            }
        }
        0x267c => WorldEvent::Coins(money_update(body)?),
        0x5ecd => {
            ensure!(body.len() >= 8, "truncated experience update");
            WorldEvent::Experience(word(body, 0).min(330))
        }
        _ => return Ok(None),
    }))
}

/// Spell actions, doors, ground objects, loot, merchant, exchange and
/// inventory packets, each owned by its codec.
fn titanium_views(opcode: u16, body: &[u8]) -> Result<Option<WorldEvent>> {
    if opcode == 0x497c {
        return Ok(crate::buffs::titanium_spell_effect(body)?.map(WorldEvent::SpellEffect));
    }
    Ok(if let Some(update) = crate::doors::decode(opcode, body)? {
        Some(WorldEvent::Doors(update))
    } else if let Some(update) = crate::objects::decode(opcode, body)? {
        Some(WorldEvent::Objects(update))
    } else if let Some(update) = crate::loot::decode(opcode, body)? {
        Some(WorldEvent::Loot(update))
    } else if let Some(update) = crate::merchant::decode(opcode, body)? {
        Some(WorldEvent::Merchant(update))
    } else if let Some(update) = crate::exchange::decode(opcode, body)? {
        Some(WorldEvent::Exchange(update))
    } else {
        crate::inventory::decode(opcode, body)?.map(WorldEvent::Inventory)
    })
}

/// `OP_MoneyUpdate`: carried coins after a purchase, sale or loot.
fn money_update(body: &[u8]) -> Result<Coins> {
    ensure!(body.len() == 16, "invalid money update length");
    let coin = |offset: usize| u32::try_from(word(body, offset).cast_signed());
    Ok(Coins {
        platinum: coin(0)?,
        gold: coin(4)?,
        silver: coin(8)?,
        copper: coin(12)?,
    })
}

/// Decodes persistent appearance state without interpreting unrelated update kinds.
pub(crate) fn appearance(body: &[u8]) -> Result<Option<WorldEvent>> {
    ensure!(body.len() == 8, "invalid appearance length");
    let kind = u16::from_le_bytes([body[2], body[3]]);
    if !matches!(kind, 3 | 14) {
        return Ok(None);
    }
    let spawn_id = u16::from_le_bytes([body[0], body[1]]);
    ensure!(spawn_id != 0, "invalid appearance spawn ID");
    Ok(Some(if kind == 14 {
        WorldEvent::Posture {
            spawn_id,
            posture: word(body, 4).into(),
        }
    } else {
        WorldEvent::Visibility {
            spawn_id,
            invisible: word(body, 4) != 0,
        }
    }))
}

/// EQ units per second for one unit of a Titanium motion delta, fitted on
/// official-client P99 recordings: the median over 2,253 straight,
/// constant-velocity stretches of NPC movement (10th to 90th percentile 0.1429
/// to 0.1448). Newer `EQEmu` servers send no deltas.
const DELTA_UNITS_PER_SECOND: f32 = 0.144;

/// EQ units per second for one unit of a moving spawn's animation value, which
/// is its speed: `EQEmu` moves NPCs `speed * 0.4 * 1.45` units per second
/// (`zone/mob_movement_manager.cpp`), and P99's recorded NPCs match (0.57).
const ANIMATION_UNITS_PER_SECOND: f32 = 0.4 * 1.45;

/// A server position update for another spawn.
fn titanium_position(body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == 22, "invalid server position length");
    let (position, velocity) = titanium_motion([2, 6, 10, 14, 18].map(|at| word(body, at)))?;
    Ok(WorldEvent::Position {
        spawn_id: u16::from_le_bytes([body[0], body[1]]),
        position,
        velocity,
    })
}

/// A Titanium position block: five words with the position, heading, animation
/// and motion deltas, as in server position updates and spawn records.
fn titanium_motion(words: [u32; 5]) -> Result<(Position, [f32; 3])> {
    let position = Position {
        x: signed_position(words[0] >> 10),
        y: signed_position(words[1]),
        z: signed_position(words[2]),
        heading: f32::from(u16::try_from((words[3] >> 13) & 0xfff)?) / 4.0,
    };
    let deltas = [
        signed_bits(words[3], 0, 13),
        signed_bits(words[2], 19, 13),
        signed_bits(words[4], 0, 13),
    ];
    let animation = signed_bits(words[1], 19, 10);
    Ok((position, velocity(position.heading, deltas, animation)))
}

/// Motion in EQ units per second: from a report's deltas, or, from servers that
/// send none, from its animation speed along the heading.
fn velocity(heading: f32, deltas: [i16; 3], animation: i16) -> [f32; 3] {
    if deltas != [0; 3] {
        return deltas.map(|delta| f32::from(delta) * DELTA_UNITS_PER_SECOND);
    }
    let speed = f32::from(animation) * ANIMATION_UNITS_PER_SECOND;
    // Heading 0 faces +Y and a quarter turn (128) faces +X.
    let angle = heading / 512.0 * std::f32::consts::TAU;
    [angle.sin() * speed, angle.cos() * speed, 0.0]
}

/// A two's complement bitfield of at most 16 bits.
fn signed_bits(value: u32, shift: u32, width: u32) -> i16 {
    let bits = (value >> shift) & ((1 << width) - 1);
    let value = i32::try_from(bits).expect("masked to 16 bits")
        - if bits >> (width - 1) == 1 {
            1 << width
        } else {
            0
        };
    i16::try_from(value).expect("a 16-bit field fits i16")
}

/// P99 only (absent from `EQEmu`'s Titanium table): a spawn's position and
/// heading with no motion, as when it stops walking. Its 80 bits hold Y, Z and
/// X as 19-bit fixed-point values, then the heading at bit 64. Checked against
/// official-client recordings: after a standing update it repeats that position
/// exactly.
fn p99_compact_position(body: &[u8]) -> Result<WorldEvent> {
    ensure!(body.len() == 12, "invalid compact position length");
    let mut bits = [0; 16];
    bits[..10].copy_from_slice(&body[2..]);
    let bits = u128::from_le_bytes(bits);
    // Truncation keeps the low 32 bits, which hold each 19-bit field.
    #[allow(clippy::cast_possible_truncation)]
    let field = |shift: u32| (bits >> shift) as u32;
    Ok(WorldEvent::Position {
        spawn_id: u16::from_le_bytes([body[0], body[1]]),
        position: Position {
            x: signed_position(field(38)),
            y: signed_position(field(0)),
            z: signed_position(field(19)),
            heading: f32::from(u16::try_from(field(64) & 0xfff)?) / 4.0,
        },
        velocity: [0.0; 3],
    })
}

#[allow(clippy::cast_possible_wrap, clippy::cast_precision_loss)]
fn signed_position(value: u32) -> f32 {
    (((value & 0x7ffff) << 13) as i32 >> 13) as f32 / 8.0
}
fn until_nul(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len())]
}
fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("layout checked by caller"),
    )
}
fn float(bytes: &[u8], offset: usize) -> Result<f32> {
    let value = f32::from_bits(word(bytes, offset));
    ensure!(value.is_finite(), "non-finite world field");
    Ok(value)
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_capability_is_listed_once_in_order() {
        use super::Capability;
        // A new capability is a compile error here until it has a place.
        let place = |capability| match capability {
            Capability::Casting => 0,
            Capability::Spellbook => 1,
            Capability::Inventory => 2,
            Capability::Trading => 3,
            Capability::Giving => 4,
            Capability::Moving => 5,
            Capability::Falling => 6,
            Capability::Targeting => 7,
            Capability::Combat => 8,
            Capability::Looting => 9,
            Capability::Talking => 10,
            Capability::Camping => 11,
            Capability::Doors => 12,
            Capability::GroundItems => 13,
            Capability::Zoning => 14,
            Capability::Abilities => 15,
            Capability::Who => 16,
        };
        for (index, capability) in Capability::ALL.into_iter().enumerate() {
            assert_eq!(place(capability), index, "{capability:?}");
        }
    }

    #[test]
    fn appearance_postures_preserve_known_and_unknown_states() {
        use super::PostureState;
        let mut packet = [0u8; 8];
        packet[..2].copy_from_slice(&73u16.to_le_bytes());
        packet[2..4].copy_from_slice(&14u16.to_le_bytes());
        for (value, expected) in [
            (100u32, PostureState::Standing),
            (102, PostureState::Frozen),
            (105, PostureState::Looting),
            (110, PostureState::Sitting),
            (111, PostureState::Ducking),
            (115, PostureState::Lying),
            (999, PostureState::Unknown(999)),
        ] {
            packet[4..].copy_from_slice(&value.to_le_bytes());
            assert!(matches!(super::titanium_update(0x7c32, &packet).unwrap(),
                Some(super::WorldEvent::Posture { spawn_id: 73, posture }) if posture == expected));
            assert_eq!(
                crate::quarm::updates(0xf540, &packet).unwrap(),
                vec![super::WorldEvent::Posture {
                    spawn_id: 73,
                    posture: expected
                }]
            );
        }
        assert!(crate::quarm::updates(0xf540, &packet[..7]).is_err());
        packet[..2].fill(0);
        assert!(crate::quarm::updates(0xf540, &packet).is_err());
    }
    #[test]
    fn titanium_visibility_updates_require_complete_appearance_packets() {
        let mut packet = [0u8; 8];
        packet[..2].copy_from_slice(&73u16.to_le_bytes());
        packet[2..4].copy_from_slice(&3u16.to_le_bytes());
        for value in [0u32, 1, 2] {
            packet[4..].copy_from_slice(&value.to_le_bytes());
            assert!(matches!(super::titanium_update(0x7c32, &packet).unwrap(),
                Some(super::WorldEvent::Visibility { spawn_id: 73, invisible }) if invisible == (value != 0)));
        }
        for length in 0..8 {
            assert!(super::titanium_update(0x7c32, &packet[..length]).is_err());
        }
        assert!(super::titanium_update(0x7c32, &[0; 9]).is_err());
        packet[2..4].copy_from_slice(&99u16.to_le_bytes());
        assert!(super::titanium_update(0x7c32, &packet).unwrap().is_none());
        packet[2..4].copy_from_slice(&3u16.to_le_bytes());
        packet[..2].fill(0);
        assert!(super::titanium_update(0x7c32, &packet).is_err());
    }
    #[test]
    fn level_updates_support_gains_and_losses_and_reject_invalid_layouts() {
        for (current, previous) in [(2u8, 1u8), (1, 2)] {
            let body: Vec<_> = [u32::from(current), u32::from(previous), 99]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect();
            assert_eq!(
                super::titanium_update(0x6d44, &body).unwrap(),
                Some(super::WorldEvent::Level {
                    current,
                    previous,
                    experience: 99
                })
            );
            for length in 0..12 {
                assert!(super::titanium_update(0x6d44, &body[..length]).is_err());
            }
        }
        for levels in [[0u32, 1], [1, 0], [256, 1], [1, 256]] {
            let body: Vec<_> = levels
                .into_iter()
                .chain([0])
                .flat_map(u32::to_le_bytes)
                .collect();
            assert!(super::titanium_update(0x6d44, &body).is_err());
        }
        assert!(super::titanium_update(0x6d44, &[0; 13]).is_err());
    }
    use super::*;
    #[test]
    #[allow(clippy::float_cmp)] // Exactly representable fixed-point fixture values.
    fn decodes_signed_coordinates_and_distinguishes_heading_scale() {
        let mut body = [0; 22];
        body[..2].copy_from_slice(&42u16.to_le_bytes());
        body[2..6].copy_from_slice(&((0x7ffffu32 - 7) << 10).to_le_bytes());
        body[6..10].copy_from_slice(&80u32.to_le_bytes());
        body[14..18].copy_from_slice(&(1024u32 << 13).to_le_bytes());
        let Some(WorldEvent::Position {
            spawn_id,
            position,
            velocity,
        }) = titanium_update(0x14cb, &body).unwrap()
        else {
            panic!("position");
        };
        assert_eq!(spawn_id, 42);
        assert_eq!(position.x, -1.0);
        assert_eq!(position.y, 10.0);
        assert_eq!(position.heading, 256.0);
        assert_eq!(velocity, [0.0; 3]);
        assert!(titanium_update(0x14cb, &body[..21]).is_err());
    }
    #[test]
    fn position_updates_carry_motion_from_deltas_or_animation_speed() {
        let motion = |deltas: [i32; 3], animation: i32, heading: u32| {
            let bits = |value: i32, width: u32| u32::try_from(value & ((1 << width) - 1)).unwrap();
            let mut body = [0; 22];
            body[6..10].copy_from_slice(&(bits(animation, 10) << 19).to_le_bytes());
            body[10..14].copy_from_slice(&(bits(deltas[1], 13) << 19).to_le_bytes());
            body[14..18].copy_from_slice(&(bits(deltas[0], 13) | heading << 13).to_le_bytes());
            body[18..22].copy_from_slice(&bits(deltas[2], 13).to_le_bytes());
            let Some(WorldEvent::Position { velocity, .. }) =
                titanium_update(0x14cb, &body).unwrap()
            else {
                panic!("position");
            };
            velocity
        };
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3);
        // P99 sends deltas: 0.144 units per second each.
        assert!(close(motion([100, -50, 8], 20, 0), [14.4, -7.2, 1.152]));
        // Newer EQEmu sends none: animation speed along the heading, here a
        // quarter turn (128, sent as 512), which faces +X.
        assert!(close(motion([0; 3], 20, 512), [11.6, 0.0, 0.0]));
        assert!(close(motion([0; 3], 20, 0), [0.0, 11.6, 0.0]));
        // Standing still.
        assert!(close(motion([0; 3], 0, 512), [0.0; 3]));
    }
    #[test]
    #[allow(clippy::float_cmp)] // Exactly representable fixed-point fixture values.
    fn p99_compact_positions_carry_coordinates_and_heading_without_motion() {
        // Coordinates in eighths of a unit, as 19-bit two's complement.
        let field = |eighths: i32| u128::try_from(eighths & 0x7ffff).unwrap();
        let bits = field(100) | field(-24) << 19 | field(-2002) << 38 | 512u128 << 64;
        let mut body = [0; 12];
        body[..2].copy_from_slice(&42u16.to_le_bytes());
        body[2..].copy_from_slice(&bits.to_le_bytes()[..10]);
        assert_eq!(
            titanium_update(0x3a2b, &body).unwrap(),
            Some(WorldEvent::Position {
                spawn_id: 42,
                position: Position {
                    x: -250.25,
                    y: 12.5,
                    z: -3.0,
                    heading: 128.0,
                },
                velocity: [0.0; 3],
            })
        );
        assert!(titanium_update(0x3a2b, &body[..11]).is_err());
    }
    #[test]
    fn profile_and_spawn_keep_distinct_axes_and_allow_default_model_size() {
        // Entirely synthetic data: never embed profiles captured from a real account.
        let mut profile = vec![0; 19592];
        let mut spawn = vec![0; 385];
        for (offset, value) in [
            (13116, 11.0f32),
            (13120, 22.0),
            (13124, 33.0),
            (13128, 28.125),
        ] {
            profile[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        profile[8..12].copy_from_slice(&1u32.to_le_bytes());
        profile[20] = 3;
        for (index, value) in [71u32, 82, 93, 104, 115, 126, 137].into_iter().enumerate() {
            let offset = 2236 + index * 4;
            profile[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        profile[124..128].copy_from_slice(&215u32.to_le_bytes());
        profile[4548..4552].copy_from_slice(&75u32.to_le_bytes());
        for index in 0..8 {
            let offset = 132 + index * 4;
            profile[offset..offset + 4]
                .copy_from_slice(&(1000u32 + u32::try_from(index).unwrap()).to_le_bytes());
        }
        profile[4360..4364].copy_from_slice(&42u32.to_le_bytes());
        profile[4364..4368].copy_from_slice(&u32::MAX.to_le_bytes());
        spawn[340..344].copy_from_slice(&7u32.to_le_bytes());
        profile[12940..12947].copy_from_slice(b"Example");
        let mut player = titanium_player(&profile, &spawn, 256.0).unwrap();
        assert_eq!(player.name, "Example");
        // Stock EQEmu saves the heading on the 512-unit scale itself.
        let stock = titanium_player(&profile, &spawn, 512.0).unwrap();
        assert!((stock.position.heading - 28.125).abs() < 0.001);
        assert_eq!(
            player.base_attributes,
            Some(BaseAttributes {
                strength: 71,
                stamina: 82,
                charisma: 93,
                dexterity: 104,
                intelligence: 115,
                agility: 126,
                wisdom: 137,
            })
        );
        assert!((player.position.x - 11.0).abs() < 0.001);
        assert!((player.position.y - 22.0).abs() < 0.001);
        assert!((player.position.heading - 56.25).abs() < 0.001);
        assert_eq!(player.memorized_spells[0], Some(42));
        assert_eq!(player.memorized_spells[1], None);
        assert_eq!(player.deity, Some(215));
        assert_eq!(player.skills.as_ref().unwrap().len(), 100);
        assert_eq!(player.skills.as_ref().unwrap()[22], 75);
        player.apply_skill(22, 1);
        assert_eq!(player.skills.as_ref().unwrap()[22], 1);
        player.apply_skill(u32::MAX, 999);
        assert_eq!(player.skills.as_ref().unwrap().len(), 100);
        player.apply_skill(22, 0);
        assert_eq!(player.skills.as_ref().unwrap()[22], 0);
        assert_eq!(
            player.spell_refresh_ms,
            Some([1000, 1001, 1002, 1003, 1004, 1005, 1006, 1007])
        );
        profile[13116..13120].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(titanium_player(&profile, &spawn, 256.0).is_err());
    }

    #[test]
    fn the_far_clip_comes_from_the_zone_header() {
        // Synthetic header shaped like Titanium's 700-byte OP_NewZone.
        let mut header = vec![0; 700];
        header[516..520].copy_from_slice(&450.0f32.to_le_bytes());
        assert_eq!(titanium_far_clip(&header), Some(450.0));
        assert_eq!(titanium_far_clip(&header[..519]), None);
        header[516..520].copy_from_slice(&0.0f32.to_le_bytes());
        assert_eq!(titanium_far_clip(&header), None);
    }

    #[test]
    fn world_short_names_come_from_log_server_settings() {
        // Synthetic body shaped like a captured Titanium OP_LogServer (266 bytes).
        let mut body = vec![0; 266];
        body[32..44].copy_from_slice(b"ExampleWorld");
        assert_eq!(titanium_world_name(&body).unwrap(), "ExampleWorld");
        assert!(titanium_world_name(&body[..63]).is_err());
        body[32..44].fill(0);
        assert!(titanium_world_name(&body).is_err());
        body[32..36].copy_from_slice(b"a b\x01");
        assert!(titanium_world_name(&body).is_err());
    }

    #[test]
    fn skill_updates_preserve_unknown_ids_and_reject_partial_packets() {
        let body = [22, 0, 0, 0, 75, 0, 0, 0];
        assert_eq!(
            titanium_update(0x6a93, &body).unwrap(),
            Some(WorldEvent::Skill {
                skill_id: 22,
                value: 75
            })
        );
        for length in 0..8 {
            assert!(titanium_update(0x6a93, &body[..length]).is_err());
        }
        let mut unknown = body;
        unknown[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            titanium_update(0x6a93, &unknown).unwrap(),
            Some(WorldEvent::Skill {
                skill_id: u32::MAX,
                value: 75
            })
        );
    }

    #[test]
    fn target_health_preserves_id_and_rejects_invalid_percentages() {
        assert_eq!(
            titanium_update(0x0695, &[0x34, 0x12, 75]).unwrap(),
            Some(WorldEvent::HealthPercent {
                spawn_id: 0x1234,
                percent: 75
            })
        );
        for body in [&[][..], &[1], &[1, 2], &[1, 2, 101], &[1, 2, 100, 0]] {
            assert!(titanium_update(0x0695, body).is_err());
        }
    }

    #[test]
    fn resource_updates_reject_truncation_and_preserve_values() {
        let mut hp = [0; 10];
        hp[..4].copy_from_slice(&27u32.to_le_bytes());
        hp[4..8].copy_from_slice(&40u32.to_le_bytes());
        hp[8..].copy_from_slice(&7u16.to_le_bytes());
        assert_eq!(
            titanium_update(0x3bcf, &hp).unwrap(),
            Some(WorldEvent::HitPoints {
                spawn_id: 7,
                current: 27,
                maximum: 40,
                without_items: true,
            })
        );
        // Negative HP, dying or below the equipped item bonus, stays negative.
        hp[..4].copy_from_slice(&(-2i32).to_le_bytes());
        assert_eq!(
            titanium_update(0x3bcf, &hp).unwrap(),
            Some(WorldEvent::HitPoints {
                spawn_id: 7,
                current: -2,
                maximum: 40,
                without_items: true,
            })
        );
        hp[4..8].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(titanium_update(0x3bcf, &hp).is_err());
        for (opcode, minimum) in [(0x3bcf, 10), (0x4839, 16), (0x5ecd, 8)] {
            for length in 0..minimum {
                assert!(titanium_update(opcode, &vec![0; length]).is_err());
            }
        }
        let mut experience = [0; 8];
        experience[..4].copy_from_slice(&165u32.to_le_bytes());
        assert_eq!(
            titanium_update(0x5ecd, &experience).unwrap(),
            Some(WorldEvent::Experience(165))
        );
    }

    #[test]
    fn batched_spawn_decode_preserves_ids_kinds_and_negative_coordinates() {
        let mut body = vec![0; 385 * 2];
        for (index, record) in body.as_chunks_mut::<385>().0.iter_mut().enumerate() {
            record[7..14].copy_from_slice(b"Fixture");
            record[83] = if index == 0 { 1 } else { 3 };
            record[331] = if index == 0 { 40 } else { 255 };
            record[75..79].copy_from_slice(&6f32.to_le_bytes());
            record[284..288].copy_from_slice(&42u32.to_le_bytes());
            record[340..344].copy_from_slice(&u32::try_from(index + 10).unwrap().to_le_bytes());
            record[94..98].copy_from_slice(&((0x7ffffu32 - 7) << 10).to_le_bytes());
            record[106..110].copy_from_slice(&(1024u32 << 13).to_le_bytes());
        }
        // Synthetic credentials only; prove the odd record length does not reset XOR phase.
        let clear = body.clone();
        crate::p99::session_xor(&mut body, b"FAKEKEY123").unwrap();
        let mut wrong_phase = body.clone();
        for record in wrong_phase.as_chunks_mut::<385>().0 {
            crate::p99::session_xor(record, b"FAKEKEY123").unwrap();
        }
        assert_ne!(wrong_phase, clear);
        crate::p99::session_xor(&mut body, b"FAKEKEY123").unwrap();
        assert_eq!(body, clear);
        let spawns = titanium_spawns(&body).unwrap();
        assert_eq!(spawns.len(), 2);
        assert_eq!(spawns[0].kind, SpawnKind::Npc);
        assert_eq!(spawns[0].class, Some(40));
        assert_eq!(spawns[1].class, Some(255));
        assert_eq!(spawns[1].kind, SpawnKind::NpcCorpse);
        assert_eq!(spawns[1].spawn_id, 11);
        assert!((spawns[0].position.x + 1.0).abs() < 0.001);
        assert!((spawns[0].position.heading - 256.0).abs() < 0.001);
        assert!(titanium_spawns(&body[..769]).is_err());
        assert_eq!(
            titanium_update(0x55bc, &11u32.to_le_bytes()).unwrap(),
            Some(WorldEvent::Despawn(11))
        );
        assert!(titanium_update(0x55bc, &[0; 3]).is_err());
        body[75..79].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(titanium_spawns(&body).is_err());
    }

    #[test]
    fn unknown_packet_is_not_misclassified() {
        assert!(titanium_update(0xffff, &[]).unwrap().is_none());
    }

    #[test]
    fn deaths_turn_entities_into_matching_corpses_and_coins_total_copper() {
        assert_eq!(SpawnKind::Npc.corpse(), SpawnKind::NpcCorpse);
        assert_eq!(SpawnKind::Player.corpse(), SpawnKind::PlayerCorpse);
        assert_eq!(SpawnKind::NpcCorpse.corpse(), SpawnKind::NpcCorpse);
        assert_eq!(SpawnKind::Unknown(9).corpse(), SpawnKind::Unknown(9));
        let mut money = [0u8; 16];
        money[..4].copy_from_slice(&1i32.to_le_bytes());
        money[12..].copy_from_slice(&5i32.to_le_bytes());
        let Some(WorldEvent::Coins(coins)) = titanium_update(0x267c, &money).unwrap() else {
            panic!("money update");
        };
        assert_eq!(coins.total_copper(), 1005);
        money[4..8].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(titanium_update(0x267c, &money).is_err());
        let mut profile = vec![0; 19592];
        profile[4432..4436].copy_from_slice(&3u32.to_le_bytes());
        assert_eq!(titanium_coins(&profile).unwrap().gold, 3);
        assert!(titanium_coins(&profile[1..]).is_err());
    }
}
