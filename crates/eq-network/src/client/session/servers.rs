//! What each server type offers in the zone, and what differs between servers
//! that speak the same client generation.
//!
//! Every part of the game is a feature a [`ServerType`] may have, each absent
//! unless the server type implements it. The zone session builds only the
//! features its server type provides, so a new server type starts as an empty
//! implementation with every feature off, and gains them one at a time. Each
//! server type also names the client generation's [`Wire`] it speaks, which
//! its features sit on: P99 and `EQEmu` share Titanium's yet differ in their
//! features.
//!
//! A server type offers each feature it provides, or leaves it to the
//! player where its own client keeps the feature off ([`leave_to_player`]):
//! the session then lists what the feature lets the player do among the
//! player's choices, and a front end offers it only once the player turns it
//! on.

use anyhow::Result;

use super::{
    abilities::Abilities,
    camp::Camp,
    casting::Casting,
    character::Character,
    clock::Clock,
    combat::Combat,
    corpses::Corpses,
    creation::{self, Creation},
    doors::Doors,
    entities::Entities,
    exchange::Exchanges,
    feature::Feature,
    groups::Groups,
    hazards::Hazards,
    inventory::Belongings,
    listing::Listing,
    looting::Looting,
    map::Map,
    motion::Motion,
    objects::GroundObjects,
    offers::MerchantOffers,
    pets::Pets,
    raids::Raids,
    reading::Reading,
    resurrection::Resurrection,
    socials::Socials,
    spellbook::{Edits, Spellbook},
    talk::Talk,
    targeting::Targeting,
    tradeskills::Tradeskills,
    training::Training,
    transfers::Transfers,
    who::Who,
    wire::{EqMac, Titanium, Wire},
    CharacterSession, Events, ServerProtocol, ZoneExit,
};
use crate::p99::{self, WorldCodec};
use eq_network_game::{
    abilities::Ability, hazards::Hazard, inventory::MoveRules, merchant::Quotes, GameDialect,
};

/// What a zone session builds its features with.
pub(super) struct Setup<'a> {
    /// The player's name, which the transfers feature reads the server's
    /// answers by.
    pub(super) character: &'a str,
    /// What the session may eat and drink on its own.
    pub(super) auto_eat: eq_network_game::food::AutoEat,
}

impl<'a> Setup<'a> {
    /// The setup for a zone session of this character.
    pub(super) fn new(character: &'a str, auto_eat: eq_network_game::food::AutoEat) -> Self {
        Self {
            character,
            auto_eat,
        }
    }
}

/// A feature as a server type provides it, ready for the zone session.
pub(super) type Provided = Option<Provision>;

/// A feature a server type provides, and how the session lists it.
pub(super) struct Provision {
    /// The feature.
    pub(super) feature: Box<dyn Feature>,
    /// Whether the server type leaves it to the player rather than offering
    /// it.
    pub(super) choice: bool,
}

/// A feature the server type offers.
#[allow(
    clippy::unnecessary_wraps,
    reason = "a server type's accessors return it as it is"
)]
fn offer(feature: Box<dyn Feature>) -> Provided {
    Some(Provision {
        feature,
        choice: false,
    })
}

/// A feature the server type leaves to the player, as its own client keeps
/// it off: the session lists what it lets the player do among the player's
/// choices, and runs it all the same.
///
/// Only for a feature that sends and hears nothing, such as the map: the
/// session never hears whether the player turned it on, so it would take a
/// left feature's commands either way (eq-network#86).
#[allow(
    clippy::unnecessary_wraps,
    reason = "a server type's accessors return it as it is"
)]
fn leave_to_player(feature: Box<dyn Feature>) -> Provided {
    Some(Provision {
        feature,
        choice: true,
    })
}

/// A server type: the client generation it speaks, what differs from other
/// servers that speak it, and the features it offers, each absent unless it
/// implements them.
pub(super) trait ServerType: Sync {
    /// The client generation's packets the server speaks.
    fn wire(&self) -> &'static dyn Wire;

    /// Protects a world connection, from the login body sent to it: P99's
    /// world approval, encrypted file manifests and checksum answers, and
    /// spawns encrypted with the login session key.
    ///
    /// # Errors
    /// Returns an error when the login body cannot key the protection.
    fn protect(&self, _login_info: &[u8]) -> Result<Option<Box<dyn Shield>>> {
        Ok(None)
    }

    /// A full turn in the headings a saved profile carries.
    fn profile_turn(&self) -> f32 {
        512.0
    }

    /// Whether the world expects the start-zone choice right after a
    /// character is created, before the character enters.
    fn start_choice(&self) -> bool {
        false
    }

    /// How the world creates characters, if it creates them for this
    /// server type.
    fn creation(&self) -> Option<&'static dyn Creation> {
        None
    }

    /// Runs the character's stay in a zone: the shared zone session, with
    /// the features this server type provides.
    ///
    /// # Errors
    /// Returns an error when the zone connection or admission fails.
    fn zone(
        &self,
        context: &CharacterSession<'_>,
        shield: &mut Option<Box<dyn Shield>>,
        (host, port): (&str, u16),
        checksums: Vec<u8>,
        log: &mut Events<'_>,
    ) -> Result<ZoneExit> {
        super::zone::run(context, shield, host, port, checksums, log)
    }

    /// Casting memorized spells and using items' effects.
    fn casting(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Memorizing, scribing and forgetting spells.
    fn spellbook(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// The inventory, the bank, coins, merchants, and food and drink.
    fn inventory(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// How the player moves: walking, running and stance, and on some
    /// servers jumps and falls.
    fn motion(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// The player's own record: level, experience, vitals and the like.
    fn character(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// The zone's spawns.
    fn entities(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Choosing a target.
    fn targeting(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Considering and attacking.
    fn combat(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Looting corpses.
    fn looting(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Giving items to NPCs and trading with players.
    fn exchange(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Skills used from the Actions window, such as kick or hide.
    fn abilities(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Speaking on the chat channels.
    fn talk(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Camping out to the character list.
    fn camp(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Opening doors.
    fn doors(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Items on the ground and the containers fixed in the zone.
    fn ground_items(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Zone lines, death and other transfers the server directs.
    fn transfers(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// The time of day in Norrath.
    fn clock(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Asking the world who is online.
    fn who(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Consenting to, summoning and dragging corpses.
    fn corpses(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Commanding the player's pet.
    fn pets(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Training skills at a guildmaster.
    fn training(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Accepting or declining a resurrection.
    fn resurrection(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Reading books and notes.
    fn reading(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Combining in the player's own tradeskill containers.
    fn tradeskills(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// The in-game map.
    fn map(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// What a merchant pays for an item sold to them, offered where the
    /// server type's rule for it has been checked against the purse.
    fn merchant_offers(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Groups: invitations, joining, leaving and disbanding.
    fn groups(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Away, anonymous and roleplaying: how `/who` lists the player.
    fn listing(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Rolling dice, emoting and assisting.
    fn socials(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Raids: invitations, joining, declining and leaving.
    fn raids(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Reporting the damage the world does to the player, which the client
    /// works out: falls, and drowning, lava and freezing.
    fn hazards(&self, _setup: &Setup<'_>) -> Provided {
        None
    }

    /// Every feature the server type provides, in the order the zone session
    /// offers them each command, packet and timer.
    fn features(&self, setup: &Setup<'_>) -> Vec<Provision> {
        [
            self.casting(setup),
            self.spellbook(setup),
            self.inventory(setup),
            self.motion(setup),
            self.character(setup),
            self.entities(setup),
            self.targeting(setup),
            self.combat(setup),
            self.looting(setup),
            self.exchange(setup),
            self.abilities(setup),
            self.talk(setup),
            self.camp(setup),
            self.doors(setup),
            self.ground_items(setup),
            self.transfers(setup),
            self.clock(setup),
            self.who(setup),
            self.corpses(setup),
            self.pets(setup),
            self.training(setup),
            self.resurrection(setup),
            self.reading(setup),
            self.tradeskills(setup),
            self.map(setup),
            self.merchant_offers(setup),
            self.groups(setup),
            self.listing(setup),
            self.socials(setup),
            self.raids(setup),
            self.hazards(setup),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

/// One protected connection's encryption and checks, keyed per world and
/// rekeyed per zone.
pub(in crate::client) trait Shield {
    /// Answers the world's approval challenge.
    ///
    /// # Errors
    /// Returns an error when the challenge is malformed.
    fn approve(&mut self, challenge: &[u8]) -> Result<Vec<u8>>;

    /// Decrypts a file manifest in place.
    ///
    /// # Errors
    /// Returns an error when the manifest is malformed.
    fn manifest(&mut self, body: &mut [u8]) -> Result<()>;

    /// Encrypts a file checksum answer in place.
    ///
    /// # Errors
    /// Returns an error when no manifest keyed the answer yet.
    fn answer(&self, body: &mut [u8]) -> Result<()>;

    /// Decrypts the file manifest a zone handoff carries.
    ///
    /// # Errors
    /// Returns an error when the handoff has no manifest.
    fn zone_manifest(&self, handoff: &[u8]) -> Result<Vec<u8>>;

    /// Rekeys for a zone from the zone entry sent to it.
    ///
    /// # Errors
    /// Returns an error when the entry is malformed.
    fn zone_entry(&mut self, entry: &[u8]) -> Result<()>;

    /// Decrypts the player's spawn and keys the zone's checksum answer from it.
    ///
    /// # Errors
    /// Returns an error when the spawn is malformed.
    fn player_spawn(&mut self, body: &mut [u8], session_key: &[u8]) -> Result<()>;

    /// Decrypts other spawns in place: `opcode` says whether the packet
    /// carries them.
    ///
    /// # Errors
    /// Returns an error without a session key.
    fn spawns(&self, opcode: u16, body: &mut [u8], session_key: &[u8]) -> Result<()>;
}

/// P99's V62 protection over Titanium.
impl Shield for WorldCodec {
    fn approve(&mut self, challenge: &[u8]) -> Result<Vec<u8>> {
        Self::approve(self, challenge)
    }

    fn manifest(&mut self, body: &mut [u8]) -> Result<()> {
        Self::manifest(self, body)
    }

    fn answer(&self, body: &mut [u8]) -> Result<()> {
        self.file_response(body)
    }

    fn zone_manifest(&self, handoff: &[u8]) -> Result<Vec<u8>> {
        Self::zone_manifest(self, handoff)
    }

    fn zone_entry(&mut self, entry: &[u8]) -> Result<()> {
        Self::zone_entry(self, entry)
    }

    fn player_spawn(&mut self, body: &mut [u8], session_key: &[u8]) -> Result<()> {
        p99::session_xor(body, session_key)?;
        self.zone_spawn(body)
    }

    fn spawns(&self, opcode: u16, body: &mut [u8], session_key: &[u8]) -> Result<()> {
        // New spawns and zone spawn batches; the XOR runs continuously over
        // the full batch, not per spawn.
        if matches!(opcode, 0x2e78 | 0x1860) {
            p99::session_xor(body, session_key)?;
        }
        Ok(())
    }
}

/// The features as built today, shared by the server types that provide
/// them; each server type still lists the ones it provides.
mod shared {
    use super::{
        Abilities, Ability, Belongings, Camp, Casting, Character, Clock, Combat, Corpses, Doors,
        Edits, Entities, Exchanges, Feature, GameDialect, GroundObjects, Groups, Hazard, Hazards,
        Listing, Looting, Map, MerchantOffers, Motion, MoveRules, Pets, Quotes, Raids, Reading,
        Resurrection, Setup, Socials, Spellbook, Talk, Targeting, Tradeskills, Training, Transfers,
        Who,
    };

    pub(super) fn casting() -> Box<dyn Feature> {
        Box::<Casting>::default()
    }

    pub(super) fn spellbook(edits: Edits) -> Box<dyn Feature> {
        Box::new(Spellbook::new(edits))
    }

    pub(super) fn inventory(setup: &Setup<'_>) -> Box<dyn Feature> {
        Box::new(Belongings::new(setup.auto_eat))
    }

    /// The inventory, whose items the player moves under the server type's
    /// rules and trades with merchants whose lists quote prices as
    /// `quotes` says, with no coin moves or meals yet.
    pub(super) fn shopping_inventory(rules: MoveRules, quotes: Quotes) -> Box<dyn Feature> {
        Box::new(Belongings::shopping(rules, quotes))
    }

    /// Moving, with or without the jumps and falls the server takes.
    pub(super) fn motion(falls: bool) -> Box<dyn Feature> {
        Box::new(Motion::new(falls))
    }

    pub(super) fn character() -> Box<dyn Feature> {
        Box::<Character>::default()
    }

    pub(super) fn entities(dialect: GameDialect) -> Box<dyn Feature> {
        Box::new(Entities::new(dialect))
    }

    pub(super) fn targeting() -> Box<dyn Feature> {
        Box::new(Targeting)
    }

    pub(super) fn combat() -> Box<dyn Feature> {
        Box::new(Combat)
    }

    pub(super) fn looting() -> Box<dyn Feature> {
        Box::new(Looting)
    }

    pub(super) fn exchange() -> Box<dyn Feature> {
        Box::new(Exchanges)
    }

    pub(super) fn abilities(listed: &'static [Ability]) -> Box<dyn Feature> {
        Box::new(Abilities::new(listed))
    }

    pub(super) fn talk() -> Box<dyn Feature> {
        Box::new(Talk)
    }

    pub(super) fn camp() -> Box<dyn Feature> {
        Box::<Camp>::default()
    }

    pub(super) fn doors() -> Box<dyn Feature> {
        Box::<Doors>::default()
    }

    pub(super) fn ground_items() -> Box<dyn Feature> {
        Box::<GroundObjects>::default()
    }

    pub(super) fn transfers(setup: &Setup<'_>) -> Box<dyn Feature> {
        Box::new(Transfers::new(setup.character))
    }

    pub(super) fn clock() -> Box<dyn Feature> {
        Box::<Clock>::default()
    }

    pub(super) fn who() -> Box<dyn Feature> {
        Box::new(Who)
    }

    pub(super) fn corpses() -> Box<dyn Feature> {
        Box::new(Corpses)
    }

    pub(super) fn pets() -> Box<dyn Feature> {
        Box::new(Pets)
    }

    pub(super) fn training() -> Box<dyn Feature> {
        Box::<Training>::default()
    }

    pub(super) fn resurrection() -> Box<dyn Feature> {
        Box::<Resurrection>::default()
    }

    pub(super) fn reading() -> Box<dyn Feature> {
        Box::new(Reading)
    }

    pub(super) fn tradeskills() -> Box<dyn Feature> {
        Box::<Tradeskills>::default()
    }

    pub(super) fn map() -> Box<dyn Feature> {
        Box::new(Map)
    }

    pub(super) fn merchant_offers() -> Box<dyn Feature> {
        Box::new(MerchantOffers)
    }

    pub(super) fn groups() -> Box<dyn Feature> {
        Box::<Groups>::default()
    }

    pub(super) fn listing() -> Box<dyn Feature> {
        Box::<Listing>::default()
    }

    pub(super) fn socials() -> Box<dyn Feature> {
        Box::new(Socials)
    }

    pub(super) fn raids() -> Box<dyn Feature> {
        Box::<Raids>::default()
    }

    /// Reporting the hazards the server type takes from the client.
    pub(super) fn hazards(taken: &'static [Hazard]) -> Box<dyn Feature> {
        Box::new(Hazards::new(taken))
    }
}

/// The abilities checked on P99: every one but fishing, which came later,
/// and binding wounds, not yet checked there.
const P99_ABILITIES: [Ability; 16] = [
    Ability::Kick,
    Ability::Bash,
    Ability::Backstab,
    Ability::Frenzy,
    Ability::FlyingKick,
    Ability::RoundKick,
    Ability::TigerClaw,
    Ability::EagleStrike,
    Ability::DragonPunch,
    Ability::Taunt,
    Ability::Hide,
    Ability::Sneak,
    Ability::Forage,
    Ability::Mend,
    Ability::FeignDeath,
    Ability::SenseHeading,
];

/// The hazards `EQEmu` takes from the client so far: falls, whose damage it
/// takes as the client reports it and lowers by the player's fall damage
/// reductions from spells, items and AAs (`Client::Handle_OP_EnvDamage`).
/// Drowning, lava and freezing wait until the official client's reports of
/// them are recorded.
const EQEMU_HAZARDS: [Hazard; 1] = [Hazard::Falling];

/// Project 1999: Titanium with V62 protection and 256-unit saved headings.
/// Jumps and falls wait until they are measured on P99.
struct Project1999;

impl ServerType for Project1999 {
    fn wire(&self) -> &'static dyn Wire {
        &Titanium
    }

    fn protect(&self, login_info: &[u8]) -> Result<Option<Box<dyn Shield>>> {
        Ok(Some(Box::new(WorldCodec::new(login_info)?)))
    }

    fn profile_turn(&self) -> f32 {
        256.0
    }

    fn creation(&self) -> Option<&'static dyn Creation> {
        Some(&creation::Titanium)
    }

    fn casting(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::casting())
    }

    /// Moving the book's spells works on P99 as on `EQEmu` (checked with the
    /// official client, 2026-10-03); deleting one does not there, so it
    /// stays off.
    fn spellbook(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::spellbook(Edits {
            deleting: false,
            moving: true,
        }))
    }

    fn inventory(&self, setup: &Setup<'_>) -> Provided {
        offer(shared::inventory(setup))
    }

    fn motion(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::motion(false))
    }

    fn character(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::character())
    }

    fn entities(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::entities(GameDialect::Titanium))
    }

    fn targeting(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::targeting())
    }

    fn combat(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::combat())
    }

    fn looting(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::looting())
    }

    fn exchange(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::exchange())
    }

    fn abilities(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::abilities(&P99_ABILITIES))
    }

    fn talk(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::talk())
    }

    fn camp(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::camp())
    }

    fn doors(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::doors())
    }

    fn ground_items(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::ground_items())
    }

    fn transfers(&self, setup: &Setup<'_>) -> Provided {
        offer(shared::transfers(setup))
    }

    fn clock(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::clock())
    }

    fn who(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::who())
    }

    fn corpses(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::corpses())
    }

    fn pets(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::pets())
    }

    /// P99's own client keeps the in-game map off, which is right for P99;
    /// the player may still turn it on (Adam, 2026-10-02).
    fn map(&self, _setup: &Setup<'_>) -> Provided {
        leave_to_player(shared::map())
    }
}

/// A stock `EQEmu` server speaking Titanium, which takes the player's jumps
/// and falls.
struct EqEmu;

impl ServerType for EqEmu {
    fn wire(&self) -> &'static dyn Wire {
        &Titanium
    }

    fn start_choice(&self) -> bool {
        true
    }

    fn creation(&self) -> Option<&'static dyn Creation> {
        Some(&creation::Titanium)
    }

    fn casting(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::casting())
    }

    fn spellbook(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::spellbook(Edits {
            deleting: true,
            moving: true,
        }))
    }

    fn inventory(&self, setup: &Setup<'_>) -> Provided {
        offer(shared::inventory(setup))
    }

    fn motion(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::motion(true))
    }

    fn character(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::character())
    }

    fn entities(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::entities(GameDialect::Titanium))
    }

    fn targeting(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::targeting())
    }

    fn combat(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::combat())
    }

    fn looting(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::looting())
    }

    fn exchange(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::exchange())
    }

    fn abilities(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::abilities(&Ability::ALL))
    }

    fn talk(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::talk())
    }

    fn camp(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::camp())
    }

    fn doors(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::doors())
    }

    fn ground_items(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::ground_items())
    }

    fn transfers(&self, setup: &Setup<'_>) -> Provided {
        offer(shared::transfers(setup))
    }

    fn clock(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::clock())
    }

    fn who(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::who())
    }

    fn corpses(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::corpses())
    }

    fn pets(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::pets())
    }

    fn training(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::training())
    }

    fn resurrection(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::resurrection())
    }

    fn reading(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::reading())
    }

    fn tradeskills(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::tradeskills())
    }

    fn map(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::map())
    }

    /// `EQEmu`'s merchants pay the price times how many are sold (a charged
    /// item counts as one), times the merchant's modifier (one at neutral
    /// standing; 1 / (0.95 x the rate it opened with)), then times 0.95,
    /// each product cut to whole copper and never rounded up, as its
    /// `Handle_OP_ShopPlayerSell` stores each into a whole number. Three
    /// sales were checked at a neutral merchant.
    fn merchant_offers(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::merchant_offers())
    }

    /// Checked live on `EQEmu` with two characters.
    fn groups(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::groups())
    }

    /// Checked live on `EQEmu` with two characters.
    fn listing(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::listing())
    }

    /// Checked live on `EQEmu` with two characters.
    fn socials(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::socials())
    }

    /// Checked live on `EQEmu` with two characters.
    fn raids(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::raids())
    }

    fn hazards(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::hazards(&EQEMU_HAZARDS))
    }
}

/// Project Quarm, which speaks `EQMac`. Its features come as they are
/// checked, first on TAKP and then on Quarm.
struct Quarm;

impl ServerType for Quarm {
    fn wire(&self) -> &'static dyn Wire {
        &EqMac
    }

    fn character(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::character())
    }

    fn entities(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::entities(GameDialect::EqMac))
    }

    fn talk(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::talk())
    }
}

/// A stock TAKP (`EQMacEmu`) server speaking `EQMac`, where the `EQMac`
/// features are checked first.
struct Takp;

impl ServerType for Takp {
    fn wire(&self) -> &'static dyn Wire {
        &EqMac
    }

    fn creation(&self) -> Option<&'static dyn Creation> {
        Some(&creation::EqMac)
    }

    fn character(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::character())
    }

    /// What TAKP's item packets say the player holds, item moves under
    /// TAKP's rules, which disconnect a player whose move they refuse, and
    /// merchants, whose lists TAKP quotes before their rate. Coin moves and
    /// meals wait.
    fn inventory(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::shopping_inventory(
            MoveRules::Takp,
            Quotes::of(GameDialect::EqMac),
        ))
    }

    fn entities(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::entities(GameDialect::EqMac))
    }

    fn talk(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::talk())
    }

    fn camp(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::camp())
    }

    /// TAKP takes no falls from the client, as P99 does not.
    fn motion(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::motion(false))
    }

    /// Zone lines and the server's moves, with `EQMac`'s zone change and the
    /// world's re-entry between zones.
    fn transfers(&self, setup: &Setup<'_>) -> Provided {
        offer(shared::transfers(setup))
    }

    fn targeting(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::targeting())
    }

    fn combat(&self, _setup: &Setup<'_>) -> Provided {
        offer(shared::combat())
    }
}

/// The server type of a server protocol.
pub(super) fn server_type(protocol: ServerProtocol) -> &'static dyn ServerType {
    match protocol {
        ServerProtocol::Project1999 => &Project1999,
        ServerProtocol::EqEmu => &EqEmu,
        ServerProtocol::Quarm => &Quarm,
        ServerProtocol::Takp => &Takp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Capability;
    use eq_network_game::{
        command::titanium_camp,
        food::AutoEat,
        request::{Request, Sender},
    };

    /// A player with a spawn.
    const SENDER: Sender<'static> = Sender {
        name: "Tester",
        spawn_id: Some(7),
    };

    /// A server type that implements nothing but the wire it speaks.
    struct Empty;

    impl ServerType for Empty {
        fn wire(&self) -> &'static dyn Wire {
            &Titanium
        }
    }

    /// What a server type's features let the player do, each once: what it
    /// offers, or else what it leaves to the player.
    fn listed(server: &dyn ServerType, choices: bool) -> Vec<Capability> {
        let setup = Setup::new("Tester", AutoEat::default());
        let mut capabilities: Vec<_> = server
            .features(&setup)
            .iter()
            .filter(|provided| provided.choice == choices)
            .flat_map(|provided| provided.feature.capabilities())
            .collect();
        capabilities.sort_unstable();
        capabilities.dedup();
        capabilities
    }

    /// What a server type offers, each once.
    fn offers(server: &dyn ServerType) -> Vec<Capability> {
        listed(server, false)
    }

    #[test]
    fn a_new_server_type_starts_with_every_feature_off() {
        let empty: &dyn ServerType = &Empty;
        let setup = Setup::new("Tester", AutoEat::default());
        assert!(empty.features(&setup).is_empty());
        assert!(empty.protect(&[0; 464]).unwrap().is_none());
        assert!((empty.profile_turn() - 512.0).abs() < f32::EPSILON);
        assert!(!empty.start_choice());
        assert!(empty.creation().is_none());
    }

    #[test]
    fn eqmac_servers_provide_the_features_built_for_them() {
        // Quarm and TAKP speak EQMac: they see the zone's spawns, keep the
        // player's record and talk. TAKP also camps, moves, zones and
        // moves items; Quarm will once each is checked there.
        let setup = Setup::new("Tester", AutoEat::default());
        let quarm = server_type(ServerProtocol::Quarm);
        assert_eq!(quarm.features(&setup).len(), 3);
        assert_eq!(offers(quarm), [Capability::Talking]);
        let takp = server_type(ServerProtocol::Takp);
        assert_eq!(takp.features(&setup).len(), 9);
        assert_eq!(
            offers(takp),
            [
                Capability::Inventory,
                Capability::Trading,
                Capability::Moving,
                Capability::Targeting,
                Capability::Combat,
                Capability::Talking,
                Capability::Camping,
                Capability::Zoning
            ]
        );
        for server in [quarm, takp] {
            assert_eq!(
                server.wire().encode(&Request::Camp, SENDER).unwrap(),
                eq_network_game::quarm::camp()
            );
            assert!(server.protect(&[0; 464]).unwrap().is_none());
            assert!(!server.start_choice());
        }
    }

    #[test]
    fn each_server_type_creates_characters_in_its_generation_or_not_at_all() {
        let character = eq_network_game::creation::NewCharacter::with_points_in(
            "Testcleric",
            (1, 2, 0),
            (212, 2),
            4,
        )
        .unwrap();
        let approval = |protocol| {
            server_type(protocol)
                .creation()
                .map(|creation| creation.approval(&character).unwrap().opcode)
        };
        let titanium = Some(eq_network_game::creation::APPROVE_NAME_OPCODE);
        assert_eq!(approval(ServerProtocol::Project1999), titanium);
        assert_eq!(approval(ServerProtocol::EqEmu), titanium);
        assert_eq!(
            approval(ServerProtocol::Takp),
            Some(eq_network_game::creation::EQMAC_APPROVE_NAME_OPCODE)
        );
        // Not yet checked on Quarm.
        assert_eq!(approval(ServerProtocol::Quarm), None);
    }

    #[test]
    fn p99_protects_and_halves_saved_headings_while_eqemu_takes_falls() {
        let p99 = server_type(ServerProtocol::Project1999);
        assert_eq!(
            p99.wire().encode(&Request::Camp, SENDER).unwrap(),
            titanium_camp()
        );
        assert!(p99.protect(&[0; 464]).unwrap().is_some());
        assert!(p99.protect(&[0; 10]).is_err());
        assert!((p99.profile_turn() - 256.0).abs() < f32::EPSILON);
        assert!(!p99.start_choice());
        assert!(!offers(p99).contains(&Capability::Falling));
        let eqemu = server_type(ServerProtocol::EqEmu);
        assert_eq!(
            eqemu.wire().encode(&Request::Camp, SENDER).unwrap(),
            titanium_camp()
        );
        assert!(eqemu.protect(&[0; 464]).unwrap().is_none());
        assert!((eqemu.profile_turn() - 512.0).abs() < f32::EPSILON);
        assert!(eqemu.start_choice());
        assert!(offers(eqemu).contains(&Capability::Falling));
    }

    #[test]
    fn p99_lists_every_ability_but_fishing_and_binding_wounds() {
        assert!(!P99_ABILITIES.contains(&Ability::Fishing));
        assert!(!P99_ABILITIES.contains(&Ability::BindWound));
        assert_eq!(P99_ABILITIES.len(), Ability::ALL.len() - 2);
    }

    #[test]
    fn p99_and_eqemu_provide_every_feature_one_each() {
        // Training, resurrection, reading, tradeskills, the map, deleting
        // spells, merchants' offers, groups, the player's listing, dice,
        // emotes, assisting, raids and the world's damage are checked on
        // EQEmu alone so far.
        for (protocol, count) in [
            (ServerProtocol::Project1999, 21),
            (ServerProtocol::EqEmu, 31),
        ] {
            let server = server_type(protocol);
            let setup = Setup::new("Tester", AutoEat::default());
            assert_eq!(server.features(&setup).len(), count, "{protocol:?}");
        }
        for capability in [
            Capability::Training,
            Capability::Resurrection,
            Capability::Reading,
            Capability::Tradeskills,
            Capability::Map,
            Capability::DeletingSpells,
            Capability::MerchantOffers,
            Capability::Grouping,
            Capability::Listing,
            Capability::Rolling,
            Capability::Emoting,
            Capability::Assisting,
            Capability::Raiding,
            Capability::EnvironmentalDamage,
        ] {
            assert!(!offers(server_type(ServerProtocol::Project1999)).contains(&capability));
            assert!(offers(server_type(ServerProtocol::EqEmu)).contains(&capability));
        }
        // Moving the book's spells is checked on both.
        for protocol in [ServerProtocol::Project1999, ServerProtocol::EqEmu] {
            assert!(offers(server_type(protocol)).contains(&Capability::MovingSpells));
        }
    }

    #[test]
    fn p99_leaves_the_map_to_the_player_where_eqemu_offers_it() {
        let p99 = server_type(ServerProtocol::Project1999);
        assert_eq!(listed(p99, true), [Capability::Map]);
        assert!(!offers(p99).contains(&Capability::Map));
        let eqemu = server_type(ServerProtocol::EqEmu);
        assert!(offers(eqemu).contains(&Capability::Map));
        // Every other server type leaves nothing to the player.
        for server in [
            eqemu,
            server_type(ServerProtocol::Quarm),
            server_type(ServerProtocol::Takp),
            &Empty,
        ] {
            assert_eq!(listed(server, true), Vec::<Capability>::new());
        }
    }

    #[test]
    fn only_spawn_packets_are_decrypted_with_the_session_key() {
        let shield = WorldCodec::new(&[0; 464]).unwrap();
        let plain = vec![1, 2, 3, 4];
        let mut other = plain.clone();
        shield.spawns(0x14cb, &mut other, b"0123456789").unwrap();
        assert_eq!(other, plain);
        for opcode in [0x2e78, 0x1860] {
            let mut spawns = plain.clone();
            shield.spawns(opcode, &mut spawns, b"0123456789").unwrap();
            assert_ne!(spawns, plain);
            p99::session_xor(&mut spawns, b"0123456789").unwrap();
            assert_eq!(spawns, plain);
        }
    }
}
