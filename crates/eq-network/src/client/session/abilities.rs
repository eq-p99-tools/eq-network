//! Using abilities: the session refuses, with the reason, an ability its
//! server type does not list, what servers would ignore without a word (an
//! unknown skill, no target, a target out of reach) and what they would
//! answer only with a complaint (a timer still running), and sends the rest.
//! It refuses a bandaging too where the server would take the bandage for
//! nothing (no one it could bandage, or a player too far away) or answer as
//! if the player had moved (no bandage at all).
//! It keeps the generation's unmodified recovery waits and tells
//! the host which abilities are offered and when a timer starts.
mod self_bandage;

use super::{
    actions::Resource,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    abilities::{eqmac, in_melee_range, player_size, Ability, Body, Recovery},
    appearance::AppearanceChange,
    bind_wound::{self, strings, BindWoundUpdate},
    inventory::InventoryUpdate,
    message::Message,
    request::Request,
    world::{PlayerState, Position, PostureState, SpawnKind, WorldEvent},
    GameDialect,
};
use std::{collections::BTreeMap, time::Instant};

/// Uses abilities and keeps their recovery timers.
pub(super) struct Abilities {
    /// The abilities the server type lists; the rest are refused.
    listed: &'static [Ability],
    /// When each timer the player started runs out.
    ready: BTreeMap<Recovery, Instant>,
    /// Set only by the server's own appearance or cancellation.
    sneaking: bool,
    self_bandage: Option<self_bandage::SelfBandage>,
}

impl Default for Abilities {
    /// Every ability listed.
    fn default() -> Self {
        Self::new(&Ability::ALL)
    }
}

/// Why the player cannot use an ability now: this library's words, and the
/// official client's string with what it names where that client words the
/// refusal itself.
struct Refusal {
    reason: String,
    official: Option<(u32, Vec<String>)>,
}

impl From<&'static str> for Refusal {
    fn from(reason: &'static str) -> Self {
        Self {
            reason: reason.into(),
            official: None,
        }
    }
}

impl Refusal {
    /// A refusal the official client words itself.
    fn official(reason: impl Into<String>, string_id: u32, arguments: Vec<String>) -> Self {
        Self {
            reason: reason.into(),
            official: Some((string_id, arguments)),
        }
    }
}

/// Tells the host that an ability was refused, and why.
fn refuse(
    session_id: u64,
    ability: Ability,
    refusal: Refusal,
    out: &mut Out<'_, '_>,
) -> Result<()> {
    let (string_id, arguments) = refusal
        .official
        .map_or((None, Vec::new()), |(id, arguments)| (Some(id), arguments));
    let diagnostic = format!("{ability:?} refused: {}", refusal.reason);
    out.log
        .send(ClientEvent::World(WorldEvent::AbilityRefused {
            session_id,
            reason: refusal.reason,
            string_id,
            arguments,
        }))?;
    out.log.diagnostic(diagnostic)
}

/// The official client's string (`eqstr_us.txt`) refusing an ability with
/// no target.
const SELECT_A_TARGET: u32 = 5825;
/// Its string refusing a target out of melee reach.
const OUT_OF_REACH: u32 = 124;

/// Why the player cannot use the ability on their target, if they cannot.
fn target_refusal(
    ability: Ability,
    dialect: GameDialect,
    world: &World,
    (own_id, position, player): (u16, Position, &PlayerState),
) -> Option<Refusal> {
    let Some((target_id, target)) = world
        .target
        .and_then(|id| world.spawns.visible(id).map(|spawn| (id, spawn)))
    else {
        // The official client says so in its own words (eqstr 5825).
        return Some(Refusal::official(
            "Select a target for that first",
            SELECT_A_TARGET,
            Vec::new(),
        ));
    };
    // Servers taunt only an NPC, ignoring anything else after starting the
    // timer; a strike may also hit another player where the server allows it.
    let fits = match target.kind {
        SpawnKind::Npc => true,
        SpawnKind::Player => ability.strikes(),
        _ => false,
    };
    if target_id == own_id || !fits {
        return Some("You cannot use that on your target".into());
    }
    // Taunt reaches 150 units, and the server says so itself beyond that.
    if dialect == GameDialect::Titanium && !ability.strikes() {
        return None;
    }
    if dialect == GameDialect::EqMac {
        return (!eqmac::in_melee_range(position, target.position))
            .then(|| Refusal::official("Your target is out of reach", OUT_OF_REACH, Vec::new()));
    }
    // The player's own spawn carries the size the server gave them by race,
    // which counts for reach.
    let size = world
        .spawns
        .all()
        .find(|spawn| spawn.spawn_id == own_id)
        .map(|spawn| spawn.size)
        .filter(|size| *size > 0.0)
        .unwrap_or_else(|| player_size(player.race));
    let own = Body {
        race: player.race,
        size,
        x: position.x,
        y: position.y,
    };
    let theirs = Body {
        race: target.race,
        size: target.size,
        x: target.position.x,
        y: target.position.y,
    };
    // The official client's own words for this are eqstr 124.
    (!in_melee_range(own, theirs))
        .then(|| Refusal::official("Your target is out of reach", OUT_OF_REACH, Vec::new()))
}

impl Abilities {
    /// The abilities a server type lists.
    pub(super) fn new(listed: &'static [Ability]) -> Self {
        Self {
            listed,
            ready: BTreeMap::new(),
            sneaking: false,
            self_bandage: None,
        }
    }

    pub(super) fn with_self_bandage(listed: &'static [Ability]) -> Self {
        Self {
            self_bandage: Some(self_bandage::SelfBandage::default()),
            ..Self::new(listed)
        }
    }

    /// The admitted player and where they are, if the server type lists the
    /// ability and the player has it.
    fn usable<'w>(
        &self,
        ability: Ability,
        world: &'w World,
        dialect: GameDialect,
    ) -> Result<((u16, Position), &'w PlayerState), Refusal> {
        if !self.listed.contains(&ability) {
            return Err("Not available on this server".into());
        }
        let Some((at, player)) = world.player_at().zip(world.player.as_ref()) else {
            return Err("Not in the zone yet".into());
        };
        let skills = player.skills.as_deref().unwrap_or_default();
        let known = match dialect {
            GameDialect::Titanium => ability.known(skills, player.race),
            GameDialect::EqMac => eqmac::known(ability, skills, player.race, player.class),
            _ => false,
        };
        if !known {
            return Err("You do not have that ability".into());
        }
        Ok((at, player))
    }

    /// Whether the ability's timer still runs.
    fn waiting(&self, ability: Ability, now: Instant) -> bool {
        ability
            .recovery()
            .and_then(|recovery| self.ready.get(&recovery))
            .is_some_and(|ready| now < *ready)
    }

    /// The target the ability's packet names, which only a strike or taunt
    /// needs, or why the player cannot use it now.
    fn aim(
        &self,
        ability: Ability,
        world: &World,
        now: Instant,
        dialect: GameDialect,
    ) -> Result<Option<u16>, Refusal> {
        let ((own_id, position), player) = self.usable(ability, world, dialect)?;
        if dialect == GameDialect::EqMac {
            self.eqmac_refusal(ability, world, player)?;
        }
        if ability.at_target() {
            if let Some(refusal) =
                target_refusal(ability, dialect, world, (own_id, position, player))
            {
                return Err(refusal);
            }
        }
        if self.waiting(ability, now) {
            return Err("Ability recovery time not yet met.".into());
        }
        Ok(*world.target)
    }

    /// Refuse known `EQMac` prerequisites before sending or changing posture.
    fn eqmac_refusal(
        &self,
        ability: Ability,
        world: &World,
        player: &PlayerState,
    ) -> Result<(), Refusal> {
        if ability == Ability::Forage && world.posture.current() == Some(PostureState::Sitting) {
            return Err("You must stand to forage".into());
        }
        if ability == Ability::Sneak && self.sneaking {
            return Err("You are already sneaking".into());
        }
        let equipment = match ability {
            Ability::Bash if ![2, 9, 10].contains(&player.race) => {
                Some((14, 8, "You need a shield to bash"))
            }
            Ability::Backstab => Some((13, 2, "You need a piercing weapon to backstab")),
            _ => None,
        };
        if let Some((slot, item_type, reason)) = equipment {
            let inventory = &world.inventory;
            if !inventory.received() || inventory.stale() {
                return Err("Equipment is not known yet".into());
            }
            if inventory
                .items()
                .get(&eq_network_game::inventory::InventorySlot(slot))
                .is_none_or(|item| item.rules.item_type != item_type)
            {
                return Err(reason.into());
            }
        }
        Ok(())
    }

    /// Whom the player would bandage, with the name the host hears (none for
    /// the player themselves), or why they cannot: their target if it is
    /// another player close by, and themselves without one.
    fn bandage(
        &self,
        world: &World,
        now: Instant,
        reaches: &eq_network_game::reach::Reaches,
    ) -> Result<(u16, Option<String>), Refusal> {
        let ((own_id, position), _) =
            self.usable(Ability::BindWound, world, GameDialect::Titanium)?;
        if self.waiting(Ability::BindWound, now) {
            return Err("You are bandaging already".into());
        }
        // The server takes a bandage worn or carried, in a bag or not.
        let carried = world.inventory.items().iter().any(|(slot, item)| {
            item.rules.item_type == bind_wound::BANDAGE
                && (slot.is_equipment() || slot.is_carried())
        });
        if !carried {
            return Err(Refusal::official(
                "You have no bandage",
                strings::NO_BANDAGES,
                Vec::new(),
            ));
        }
        let Some(target_id) = (*world.target).filter(|id| *id != own_id) else {
            return Ok((own_id, None));
        };
        let Some(target) = world
            .spawns
            .visible(target_id)
            .filter(|spawn| matches!(spawn.kind, SpawnKind::Player))
        else {
            return Err(Refusal::official(
                "You can bandage only a player",
                strings::NOT_A_PLAYER,
                Vec::new(),
            ));
        };
        // The program's own reach (Titanium s8a T8, TAKP T8), 15 units,
        // inside the server's 20 ([`bind_wound::REACH`]).
        if !reaches.bandage.admits(position, target.position) {
            let name = target.name.clone();
            return Err(Refusal::official(
                format!("{name} is too far away to bandage"),
                reaches.too_far_to_bandage,
                vec![name],
            ));
        }
        Ok((target_id, Some(target.name.clone())))
    }

    fn use_ability(
        &mut self,
        (session_id, ability): (u64, Ability),
        world: &mut World,
        out: &mut Out<'_, '_>,
        now: Instant,
    ) -> Result<()> {
        if ability == Ability::BindWound {
            if let Some(state) = &self.self_bandage {
                let checked = self.usable(ability, world, GameDialect::Titanium).and_then(
                    |((actor, _), _)| state.check(world, actor).map(|stock| (actor, stock)),
                );
                let (actor, stock) = match checked {
                    Ok(checked) => checked,
                    Err(refusal) => return refuse(session_id, ability, refusal, out),
                };
                self.send(session_id, ability, Some(actor), out, now)?;
                if let Some(state) = &mut self.self_bandage {
                    state.submitted(actor, stock, now);
                }
                world.happened(Message::Event(WorldEvent::Inventory(
                    InventoryUpdate::BandageStockPending(true),
                )));
                return out.log.send(ClientEvent::World(WorldEvent::BindWound(
                    BindWoundUpdate::RequestedSelf,
                )));
            }
            return match self.bandage(world, now, out.wire.reaches()) {
                Ok((spawn_id, target)) => {
                    self.send(session_id, ability, Some(spawn_id), out, now)?;
                    out.log.send(ClientEvent::World(WorldEvent::BindWound(
                        BindWoundUpdate::Started { target },
                    )))
                }
                Err(refusal) => refuse(session_id, ability, refusal, out),
            };
        }
        match self.aim(ability, world, now, out.wire.dialect()) {
            Ok(target) => {
                if ability == Ability::FeignDeath && out.wire.dialect() == GameDialect::EqMac {
                    // All refusals precede this intentional stance change.
                    // Lying is visible even when the feign roll fails.
                    world
                        .posture
                        .lie_for_feign(world.own_spawn.unwrap_or(0), out)?;
                }
                self.send(session_id, ability, target, out, now)
            }
            Err(refusal) => refuse(session_id, ability, refusal, out),
        }
    }

    /// Sends an ability's packet and starts its timer, telling the host.
    fn send(
        &mut self,
        session_id: u64,
        ability: Ability,
        target: Option<u16>,
        out: &mut Out<'_, '_>,
        now: Instant,
    ) -> Result<()> {
        out.request(&Request::Ability { ability, target })?;
        let reuse = ability.reuse(out.wire.dialect());
        if let Some(recovery) = ability.recovery() {
            self.ready.insert(recovery, now + reuse);
        }
        out.log.send(ClientEvent::World(WorldEvent::AbilityUsed {
            session_id,
            ability,
            ready_in: reuse,
        }))
    }
}

impl Feature for Abilities {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Abilities]
    }

    /// Tells the host which abilities this server type offers.
    fn admitted(&mut self, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        out.log
            .send(ClientEvent::World(WorldEvent::AbilitiesOffered(
                self.listed.to_vec(),
            )))
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::UseAbility { .. })
    }

    fn holds(&self, _world: &World, _now: Instant) -> Vec<(Resource, &'static str)> {
        self.self_bandage
            .as_ref()
            .filter(|state| state.holds_inventory())
            .map(|_| (Resource::Inventory, "Wait for the bandage stock update"))
            .into_iter()
            .collect()
    }

    fn tick(&mut self, now: Instant, _world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
        if self
            .self_bandage
            .as_mut()
            .is_some_and(|state| state.timed_out(now))
        {
            out.log.send(ClientEvent::World(WorldEvent::BindWound(
                BindWoundUpdate::Unconfirmed,
            )))?;
        }
        Ok(())
    }

    /// Authoritative reset frees the shared recovery group, not just the
    /// named button. Stealth cancellation changes state, never cooldowns.
    fn admit(&mut self, message: &Message, world: &mut World) -> Result<()> {
        match message {
            Message::Event(WorldEvent::BindWound(
                BindWoundUpdate::Ended(_) | BindWoundUpdate::Released,
            )) => {
                self.ready.remove(&Recovery::BindWound);
            }
            Message::Event(WorldEvent::SkillReady(skill)) => {
                if let Some(recovery) = Ability::from_skill(*skill)
                    .filter(|ability| self.listed.contains(ability))
                    .and_then(Ability::recovery)
                {
                    self.ready.remove(&recovery);
                }
            }
            Message::Event(WorldEvent::Appearance {
                spawn_id,
                change: AppearanceChange::Sneaking(on),
            }) if world.is_player(*spawn_id) => self.sneaking = *on,
            Message::Event(WorldEvent::HideAndSneakEnded) => self.sneaking = false,
            _ => (),
        }
        Ok(())
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if self
            .self_bandage
            .as_mut()
            .is_some_and(|state| state.observe(message, world))
        {
            world.happened(Message::Event(WorldEvent::Inventory(
                InventoryUpdate::BandageStockPending(false),
            )));
        }
        self.admit(message, world)
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::UseAbility {
            session_id,
            ability,
            target,
            ..
        } = *command
        else {
            return Ok(());
        };
        if session_id != world.session_id {
            return refuse(
                session_id,
                ability,
                "Ability belongs to another admission".into(),
                out,
            );
        }
        if (ability.at_target() || ability == Ability::BindWound)
            && target.is_none_or(|target| !target.available || target != world.target.state())
        {
            return refuse(
                session_id,
                ability,
                "Target changed or is not ready; choose it again".into(),
                out,
            );
        }
        self.use_ability((session_id, ability), world, out, Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, targeting::Target};
    use super::*;
    use eq_network_game::{
        bind_wound::BindWoundEnd,
        inventory::{Inventory, InventorySlot, InventoryUpdate},
        world::Position,
        GameDialect,
    };
    use std::time::Duration;

    /// An admitted warrior (7) with kick and taunt, beside an NPC (42), with
    /// another (43) twenty units away.
    fn warrior() -> World {
        let mut world = World::new(5);
        let mut player = testing::player(7);
        let mut skills = vec![0; 100];
        skills[30] = 25;
        skills[73] = 10;
        player.skills = Some(skills);
        player.race = 1;
        world.player.admit(player);
        world.own_spawn = Some(7);
        world.spawns.insert(testing::spawn(42, SpawnKind::Npc));
        let mut far = testing::spawn(43, SpawnKind::Npc);
        far.position = Position {
            x: 20.0,
            ..Position::default()
        };
        world.spawns.insert(far);
        world
    }

    /// The warrior with a bandage in the bag in their first pack slot, beside
    /// one player (46) ten units away and another (47) sixteen units away,
    /// past the program's 15 though inside the server's 20.
    fn bandager() -> World {
        let mut world = warrior();
        let mut bag = testing::item(22);
        bag.bag_slots = 8;
        let mut bandage = testing::item(251);
        bandage.rules.item_type = bind_wound::BANDAGE;
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![bag, bandage]));
        world.inventory = inventory.into();
        for (id, x) in [(46, 10.0), (47, 16.0)] {
            let mut player = testing::spawn(id, SpawnKind::Player);
            player.position = Position {
                x,
                ..Position::default()
            };
            world.spawns.insert(player);
        }
        world
    }

    /// The official string and its arguments of each refusal.
    fn official(events: &[ClientEvent]) -> Vec<(Option<u32>, Vec<String>)> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::AbilityRefused {
                    string_id,
                    arguments,
                    ..
                }) => Some((*string_id, arguments.clone())),
                _ => None,
            })
            .collect()
    }

    fn refused(events: &[ClientEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                ClientEvent::World(WorldEvent::AbilityRefused { reason, .. }) => {
                    Some(reason.clone())
                }
                _ => None,
            })
            .collect()
    }

    fn try_ability(
        abilities: &mut Abilities,
        world: &mut World,
        ability: Ability,
        now: Instant,
    ) -> testing::Outcome<Result<()>> {
        testing::run(|out| abilities.use_ability((5, ability), world, out, now))
    }

    #[test]
    fn an_ability_the_server_type_does_not_list_is_refused_and_the_host_hears_the_list() {
        const LISTED: [Ability; 2] = [Ability::Kick, Ability::Taunt];
        let mut abilities = Abilities::new(&LISTED);
        let mut world = warrior();
        let outcome = testing::run(|out| abilities.admitted(&mut world, out));
        assert!(matches!(
            &outcome.events[..],
            [ClientEvent::World(WorldEvent::AbilitiesOffered(offered))] if offered == &LISTED
        ));
        // Anyone could fish, but this server type does not list it.
        let outcome = try_ability(&mut abilities, &mut world, Ability::Fishing, Instant::now());
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert_eq!(refused(&outcome.events), ["Not available on this server"]);
    }

    #[test]
    fn a_strike_needs_the_skill_and_a_target_in_reach() {
        let mut abilities = Abilities::default();
        let mut world = warrior();
        let now = Instant::now();
        let outcome = try_ability(&mut abilities, &mut world, Ability::Kick, now);
        assert!(
            outcome.sent.is_empty(),
            "no target, sent {:?}",
            outcome.sent
        );
        assert_eq!(refused(&outcome.events), ["Select a target for that first"]);
        world.target = super::super::targeting::Target::from(43);
        let outcome = try_ability(&mut abilities, &mut world, Ability::Kick, now);
        assert_eq!(refused(&outcome.events), ["Your target is out of reach"]);
        let outcome = try_ability(&mut abilities, &mut world, Ability::Bash, now);
        assert_eq!(refused(&outcome.events), ["You do not have that ability"]);
        world.target = super::super::targeting::Target::from(42);
        let outcome = try_ability(&mut abilities, &mut world, Ability::Kick, now);
        assert_eq!(outcome.sent, [Ability::Kick.encode(42)]);
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(WorldEvent::AbilityUsed {
                ability: Ability::Kick,
                ..
            })]
        ));
    }

    #[test]
    fn reach_counts_the_players_size_and_taunt_wants_an_npc() {
        let now = Instant::now();
        let at = |x| Position {
            x,
            ..Position::default()
        };
        // A human counts as size 6: twelve units of reach against a size 6
        // creature, where a gnome, counted small, has sixteen.
        let mut world = warrior();
        let mut near = testing::spawn(45, SpawnKind::Npc);
        near.position = at(13.0);
        near.size = 6.0;
        world.spawns.insert(near);
        world.target = super::super::targeting::Target::from(45);
        let outcome = try_ability(&mut Abilities::default(), &mut world, Ability::Kick, now);
        assert_eq!(refused(&outcome.events), ["Your target is out of reach"]);
        world.player.corrected().unwrap().race = 12;
        let outcome = try_ability(&mut Abilities::default(), &mut world, Ability::Kick, now);
        assert_eq!(outcome.sent, [Ability::Kick.encode(45)]);
        // Taunt reaches past melee range, but never another player.
        world.target = super::super::targeting::Target::from(43);
        let outcome = try_ability(&mut Abilities::default(), &mut world, Ability::Taunt, now);
        assert_eq!(outcome.sent, [Ability::Taunt.encode(43)]);
        world.spawns.insert(testing::spawn(46, SpawnKind::Player));
        world.target = super::super::targeting::Target::from(46);
        let outcome = try_ability(&mut Abilities::default(), &mut world, Ability::Taunt, now);
        assert_eq!(
            refused(&outcome.events),
            ["You cannot use that on your target"]
        );
        let outcome = try_ability(&mut Abilities::default(), &mut world, Ability::Kick, now);
        assert_eq!(outcome.sent, [Ability::Kick.encode(46)]);
    }

    #[test]
    fn bandaging_needs_a_bandage_and_another_player_close_by() {
        let now = Instant::now();
        // Without a bandage the server would answer as if the player moved.
        let outcome = try_ability(
            &mut Abilities::default(),
            &mut warrior(),
            Ability::BindWound,
            now,
        );
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert_eq!(
            official(&outcome.events),
            [(Some(strings::NO_BANDAGES), vec![])]
        );
        // With one and no target, the player bandages themselves.
        let mut world = bandager();
        let outcome = try_ability(
            &mut Abilities::default(),
            &mut world,
            Ability::BindWound,
            now,
        );
        assert_eq!(outcome.sent, [bind_wound::encode(7)]);
        assert!(matches!(
            &outcome.events[..],
            [
                ClientEvent::World(WorldEvent::AbilityUsed {
                    ability: Ability::BindWound,
                    ..
                }),
                ClientEvent::World(WorldEvent::BindWound(BindWoundUpdate::Started {
                    target: None
                })),
            ]
        ));
        // An NPC is not for bandaging, nor a player out of reach.
        world.target = Target::from(42);
        let outcome = try_ability(
            &mut Abilities::default(),
            &mut world,
            Ability::BindWound,
            now,
        );
        assert_eq!(
            official(&outcome.events),
            [(Some(strings::NOT_A_PLAYER), vec![])]
        );
        world.target = Target::from(47);
        let outcome = try_ability(
            &mut Abilities::default(),
            &mut world,
            Ability::BindWound,
            now,
        );
        assert_eq!(
            official(&outcome.events),
            [(
                Some(eq_network_game::reach::TITANIUM.too_far_to_bandage),
                vec!["Spawn 47".to_owned()]
            )]
        );
        world.target = Target::from(46);
        let outcome = try_ability(
            &mut Abilities::default(),
            &mut world,
            Ability::BindWound,
            now,
        );
        assert_eq!(outcome.sent, [bind_wound::encode(46)]);
        assert!(outcome.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::BindWound(BindWoundUpdate::Started {
                target: Some(name)
            })) if name == "Spawn 46"
        )));
    }

    #[test]
    fn a_bandaging_holds_the_next_until_it_ends() {
        let mut abilities = Abilities::default();
        let mut world = bandager();
        let now = Instant::now();
        try_ability(&mut abilities, &mut world, Ability::BindWound, now)
            .result
            .unwrap();
        let outcome = try_ability(&mut abilities, &mut world, Ability::BindWound, now);
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert_eq!(refused(&outcome.events), ["You are bandaging already"]);
        // The server's answer that it ended frees the next at once.
        let ended = Message::Event(WorldEvent::BindWound(BindWoundUpdate::Ended(
            BindWoundEnd::YouMoved,
        )));
        testing::run(|out| abilities.observe(&ended, &mut world, out))
            .result
            .unwrap();
        let outcome = try_ability(&mut abilities, &mut world, Ability::BindWound, now);
        assert_eq!(outcome.sent, [bind_wound::encode(7)]);
    }

    #[test]
    fn a_self_bandage_waits_for_the_result_and_a_received_stock_update() {
        let mut abilities = Abilities::with_self_bandage(&Ability::ALL);
        let mut world = bandager();
        let now = Instant::now();
        world.admitted = Some(now);
        world.posture.observed(PostureState::Standing);
        assert_eq!(
            try_ability(&mut abilities, &mut world, Ability::BindWound, now).sent,
            []
        );

        world.target = Target::from(7);
        let sent = try_ability(&mut abilities, &mut world, Ability::BindWound, now);
        assert_eq!(sent.sent, [bind_wound::encode(7)]);
        assert!(sent.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::BindWound(BindWoundUpdate::RequestedSelf))
        )));
        assert_eq!(world.inventory.items().len(), 2);
        let answer = Message::Event(WorldEvent::BindWound(BindWoundUpdate::Unlocked));
        testing::run(|out| abilities.observe(&answer, &mut world, out))
            .result
            .unwrap();
        assert_eq!(
            try_ability(&mut abilities, &mut world, Ability::BindWound, now).sent,
            []
        );

        let answer = Message::Event(WorldEvent::BindWound(BindWoundUpdate::Ended(
            BindWoundEnd::YouMoved,
        )));
        testing::run(|out| abilities.observe(&answer, &mut world, out))
            .result
            .unwrap();
        assert_eq!(
            try_ability(&mut abilities, &mut world, Ability::BindWound, now).sent,
            []
        );
        // Server inventory state is applied by its owner before abilities observe it.
        let mut inventory = (*world.inventory).clone();
        let snapshot = InventoryUpdate::Snapshot(inventory.items().values().cloned().collect());
        inventory.apply(snapshot.clone());
        world.inventory = inventory.into();
        let answer = Message::Event(WorldEvent::Inventory(snapshot));
        testing::run(|out| abilities.observe(&answer, &mut world, out))
            .result
            .unwrap();
        assert_eq!(
            try_ability(&mut abilities, &mut world, Ability::BindWound, now).sent,
            [bind_wound::encode(7)]
        );
    }

    #[test]
    fn a_self_bandage_timeout_does_not_consume_stock_or_start_another_attempt() {
        let mut abilities = Abilities::with_self_bandage(&Ability::ALL);
        let mut world = bandager();
        let now = Instant::now();
        world.admitted = Some(now);
        world.posture.observed(PostureState::Sitting);
        world.target = Target::from(7);
        assert_eq!(
            try_ability(&mut abilities, &mut world, Ability::BindWound, now).sent,
            [bind_wound::encode(7)]
        );
        let later = now + Duration::from_secs(15);
        let timeout = testing::run(|out| abilities.tick(later, &mut world, out));
        assert_eq!(timeout.sent, []);
        assert!(matches!(
            timeout.events.as_slice(),
            [ClientEvent::World(WorldEvent::BindWound(
                BindWoundUpdate::Unconfirmed
            ))]
        ));
        assert_eq!(world.inventory.items().len(), 2);
        assert_eq!(
            abilities.holds(&world, later),
            [(Resource::Inventory, "Wait for the bandage stock update")]
        );
        assert_eq!(
            try_ability(&mut abilities, &mut world, Ability::BindWound, later).sent,
            []
        );

        let mut inventory = (*world.inventory).clone();
        inventory.apply(InventoryUpdate::BandageStockPending(true));
        let consumed = InventoryUpdate::Consume {
            slot: InventorySlot(251),
            charge: false,
        };
        inventory.apply(consumed.clone());
        world.inventory = inventory.into();
        let delayed = Message::Event(WorldEvent::Inventory(consumed));
        testing::run(|out| abilities.observe(&delayed, &mut world, out))
            .result
            .unwrap();
        assert_eq!(abilities.holds(&world, later), []);
        assert!(world.take_news().iter().any(|news| matches!(
            news,
            Message::Event(WorldEvent::Inventory(InventoryUpdate::BandageStockPending(
                false
            )))
        )));
        assert!(!world.inventory.items().contains_key(&InventorySlot(251)));
        assert_eq!(
            try_ability(&mut abilities, &mut world, Ability::BindWound, later).sent,
            []
        );
    }

    #[test]
    fn a_silent_move_of_possible_stock_cannot_authorize_self_bandaging() {
        let now = Instant::now();
        for spare in [false, true] {
            let mut world = bandager();
            world.admitted = Some(now);
            world.posture.observed(PostureState::Standing);
            world.target = Target::from(7);
            let mut inventory = (*world.inventory).clone();
            let mut original: Vec<_> = inventory.items().values().cloned().collect();
            if spare {
                let mut item = testing::item(24);
                item.rules.item_type = bind_wound::BANDAGE;
                original.push(item);
            }
            inventory.apply(InventoryUpdate::Snapshot(original.clone()));
            let moved = original
                .iter_mut()
                .find(|item| item.slot == InventorySlot(251))
                .unwrap();
            moved.slot = InventorySlot::CURSOR;
            inventory.apply(InventoryUpdate::Prediction(original));
            inventory.apply(InventoryUpdate::Settled);
            assert!(!inventory.predicted());
            world.inventory = inventory.into();
            let mut abilities = Abilities::with_self_bandage(&Ability::ALL);
            let refused = try_ability(&mut abilities, &mut world, Ability::BindWound, now);
            assert_eq!(refused.sent, []);
            assert_eq!(official(&refused.events), [(None, vec![])]);
            assert!(refused.events.iter().any(|event| matches!(
                event,
                ClientEvent::World(WorldEvent::AbilityRefused { reason, .. })
                    if reason == "Bandage stock is not current"
            )));
        }
    }

    #[test]
    fn strikes_share_a_timer_and_other_abilities_keep_their_own() {
        let mut abilities = Abilities::default();
        let mut world = warrior();
        world.target = super::super::targeting::Target::from(42);
        let now = Instant::now();
        try_ability(&mut abilities, &mut world, Ability::Kick, now)
            .result
            .unwrap();
        let outcome = try_ability(&mut abilities, &mut world, Ability::Kick, now);
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert_eq!(
            refused(&outcome.events),
            ["Ability recovery time not yet met."]
        );
        // Taunt waits on its own timer.
        let outcome = try_ability(&mut abilities, &mut world, Ability::Taunt, now);
        assert_eq!(outcome.sent, [Ability::Taunt.encode(42)]);
        // The official client waits longer than the emulator's own timer.
        let emulator = now + Duration::from_secs(4);
        let outcome = try_ability(&mut abilities, &mut world, Ability::Kick, emulator);
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        let outcome = try_ability(
            &mut abilities,
            &mut world,
            Ability::Kick,
            now + Ability::Kick.reuse(GameDialect::Titanium),
        );
        assert_eq!(outcome.sent.len(), 1);
        // A Barbarian bashes without the skill; an untargeted ability needs
        // no target.
        let mut world = warrior();
        world.player.corrected().unwrap().race = 2;
        world.target = super::super::targeting::Target::from(42);
        let outcome = try_ability(&mut Abilities::default(), &mut world, Ability::Bash, now);
        assert_eq!(outcome.sent, [Ability::Bash.encode(42)]);
        let outcome = try_ability(&mut Abilities::default(), &mut world, Ability::Hide, now);
        assert_eq!(refused(&outcome.events), ["You do not have that ability"]);
    }
}
