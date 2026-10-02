//! Using abilities: the session refuses, with the reason, an ability its
//! server type does not list, what servers would ignore without a word (an
//! unknown skill, no target, a target out of reach) and what they would
//! answer only with a complaint (a timer still running), and sends the rest.
//! It refuses a bandaging too where the server would take the bandage for
//! nothing (no one it could bandage, or a player too far away) or answer as
//! if the player had moved (no bandage at all).
//! It keeps each recovery timer, as long as the server's at most, and tells
//! the host which abilities are offered and when a timer starts.
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    abilities::{in_melee_range, player_size, Ability, Body, Recovery},
    bind_wound::{self, strings, BindWoundUpdate},
    message::Message,
    request::Request,
    world::{PlayerState, Position, SpawnKind, WorldEvent},
};
use std::{collections::BTreeMap, time::Instant};

/// Uses abilities and keeps their recovery timers.
pub(super) struct Abilities {
    /// The abilities the server type lists; the rest are refused.
    listed: &'static [Ability],
    /// When each timer the player started runs out.
    ready: BTreeMap<Recovery, Instant>,
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
    if !ability.strikes() {
        return None;
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
        }
    }

    /// The admitted player and where they are, if the server type lists the
    /// ability and the player has it.
    fn usable<'w>(
        &self,
        ability: Ability,
        world: &'w World,
    ) -> Result<((u16, Position), &'w PlayerState), Refusal> {
        if !self.listed.contains(&ability) {
            return Err("Not available on this server".into());
        }
        let Some((at, player)) = world.player_at().zip(world.player.as_ref()) else {
            return Err("Not in the zone yet".into());
        };
        let skills = player.skills.as_deref().unwrap_or_default();
        if !ability.known(skills, player.race) {
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
    fn aim(&self, ability: Ability, world: &World, now: Instant) -> Result<Option<u16>, Refusal> {
        let ((own_id, position), player) = self.usable(ability, world)?;
        if ability.at_target() {
            if let Some(refusal) = target_refusal(ability, world, (own_id, position, player)) {
                return Err(refusal);
            }
        }
        if self.waiting(ability, now) {
            return Err("Ability recovery time not yet met.".into());
        }
        Ok(*world.target)
    }

    /// Whom the player would bandage, with the name the host hears (none for
    /// the player themselves), or why they cannot: their target if it is
    /// another player close by, and themselves without one.
    fn bandage(&self, world: &World, now: Instant) -> Result<(u16, Option<String>), Refusal> {
        let ((own_id, position), _) = self.usable(Ability::BindWound, world)?;
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
        let apart = [
            target.position.x - position.x,
            target.position.y - position.y,
            target.position.z - position.z,
        ];
        if apart.iter().map(|axis| axis * axis).sum::<f32>() > bind_wound::REACH.powi(2) {
            let name = target.name.clone();
            return Err(Refusal::official(
                format!("{name} is too far away to bandage"),
                strings::TOO_FAR,
                vec![name],
            ));
        }
        Ok((target_id, Some(target.name.clone())))
    }

    fn use_ability(
        &mut self,
        (session_id, ability): (u64, Ability),
        world: &World,
        out: &mut Out<'_, '_>,
        now: Instant,
    ) -> Result<()> {
        if ability == Ability::BindWound {
            return match self.bandage(world, now) {
                Ok((spawn_id, target)) => {
                    self.send(session_id, ability, Some(spawn_id), out, now)?;
                    out.log.send(ClientEvent::World(WorldEvent::BindWound(
                        BindWoundUpdate::Started { target },
                    )))
                }
                Err(refusal) => refuse(session_id, ability, refusal, out),
            };
        }
        match self.aim(ability, world, now) {
            Ok(target) => self.send(session_id, ability, target, out, now),
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
        if let Some(recovery) = ability.recovery() {
            self.ready.insert(recovery, now + ability.reuse());
        }
        out.log.send(ClientEvent::World(WorldEvent::AbilityUsed {
            session_id,
            ability,
            ready_in: ability.reuse(),
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

    /// A bandaging that ended frees the next at once.
    fn observe(
        &mut self,
        message: &Message,
        _world: &mut World,
        _out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let Message::Event(WorldEvent::BindWound(BindWoundUpdate::Ended(_))) = message {
            self.ready.remove(&Recovery::BindWound);
        }
        Ok(())
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
            ..
        } = *command
        else {
            return Ok(());
        };
        self.use_ability((session_id, ability), world, out, Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{feature::testing, targeting::Target};
    use super::*;
    use eq_network_game::{
        bind_wound::BindWoundEnd,
        inventory::{Inventory, InventoryUpdate},
        world::Position,
    };

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
    /// one player (46) ten units away and another (47) thirty units away.
    fn bandager() -> World {
        let mut world = warrior();
        let mut bag = testing::item(22);
        bag.bag_slots = 8;
        let mut bandage = testing::item(251);
        bandage.rules.item_type = bind_wound::BANDAGE;
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(vec![bag, bandage]));
        world.inventory = inventory.into();
        for (id, x) in [(46, 10.0), (47, 30.0)] {
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
        world: &World,
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
        let outcome = try_ability(&mut abilities, &world, Ability::Fishing, Instant::now());
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert_eq!(refused(&outcome.events), ["Not available on this server"]);
    }

    #[test]
    fn a_strike_needs_the_skill_and_a_target_in_reach() {
        let mut abilities = Abilities::default();
        let mut world = warrior();
        let now = Instant::now();
        let outcome = try_ability(&mut abilities, &world, Ability::Kick, now);
        assert!(
            outcome.sent.is_empty(),
            "no target, sent {:?}",
            outcome.sent
        );
        assert_eq!(refused(&outcome.events), ["Select a target for that first"]);
        world.target = super::super::targeting::Target::from(43);
        let outcome = try_ability(&mut abilities, &world, Ability::Kick, now);
        assert_eq!(refused(&outcome.events), ["Your target is out of reach"]);
        let outcome = try_ability(&mut abilities, &world, Ability::Bash, now);
        assert_eq!(refused(&outcome.events), ["You do not have that ability"]);
        world.target = super::super::targeting::Target::from(42);
        let outcome = try_ability(&mut abilities, &world, Ability::Kick, now);
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
        let outcome = try_ability(&mut Abilities::default(), &world, Ability::Kick, now);
        assert_eq!(refused(&outcome.events), ["Your target is out of reach"]);
        world.player.corrected().unwrap().race = 12;
        let outcome = try_ability(&mut Abilities::default(), &world, Ability::Kick, now);
        assert_eq!(outcome.sent, [Ability::Kick.encode(45)]);
        // Taunt reaches past melee range, but never another player.
        world.target = super::super::targeting::Target::from(43);
        let outcome = try_ability(&mut Abilities::default(), &world, Ability::Taunt, now);
        assert_eq!(outcome.sent, [Ability::Taunt.encode(43)]);
        world.spawns.insert(testing::spawn(46, SpawnKind::Player));
        world.target = super::super::targeting::Target::from(46);
        let outcome = try_ability(&mut Abilities::default(), &world, Ability::Taunt, now);
        assert_eq!(
            refused(&outcome.events),
            ["You cannot use that on your target"]
        );
        let outcome = try_ability(&mut Abilities::default(), &world, Ability::Kick, now);
        assert_eq!(outcome.sent, [Ability::Kick.encode(46)]);
    }

    #[test]
    fn bandaging_needs_a_bandage_and_another_player_close_by() {
        let now = Instant::now();
        // Without a bandage the server would answer as if the player moved.
        let outcome = try_ability(
            &mut Abilities::default(),
            &warrior(),
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
        let outcome = try_ability(&mut Abilities::default(), &world, Ability::BindWound, now);
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
        let outcome = try_ability(&mut Abilities::default(), &world, Ability::BindWound, now);
        assert_eq!(
            official(&outcome.events),
            [(Some(strings::NOT_A_PLAYER), vec![])]
        );
        world.target = Target::from(47);
        let outcome = try_ability(&mut Abilities::default(), &world, Ability::BindWound, now);
        assert_eq!(
            official(&outcome.events),
            [(Some(strings::TOO_FAR), vec!["Spawn 47".to_owned()])]
        );
        world.target = Target::from(46);
        let outcome = try_ability(&mut Abilities::default(), &world, Ability::BindWound, now);
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
        try_ability(&mut abilities, &world, Ability::BindWound, now)
            .result
            .unwrap();
        let outcome = try_ability(&mut abilities, &world, Ability::BindWound, now);
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert_eq!(refused(&outcome.events), ["You are bandaging already"]);
        // The server's answer that it ended frees the next at once.
        let ended = Message::Event(WorldEvent::BindWound(BindWoundUpdate::Ended(
            BindWoundEnd::YouMoved,
        )));
        testing::run(|out| abilities.observe(&ended, &mut world, out))
            .result
            .unwrap();
        let outcome = try_ability(&mut abilities, &world, Ability::BindWound, now);
        assert_eq!(outcome.sent, [bind_wound::encode(7)]);
    }

    #[test]
    fn strikes_share_a_timer_and_other_abilities_keep_their_own() {
        let mut abilities = Abilities::default();
        let mut world = warrior();
        world.target = super::super::targeting::Target::from(42);
        let now = Instant::now();
        try_ability(&mut abilities, &world, Ability::Kick, now)
            .result
            .unwrap();
        let outcome = try_ability(&mut abilities, &world, Ability::Kick, now);
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
        assert_eq!(
            refused(&outcome.events),
            ["Ability recovery time not yet met."]
        );
        // Taunt waits on its own timer.
        let outcome = try_ability(&mut abilities, &world, Ability::Taunt, now);
        assert_eq!(outcome.sent, [Ability::Taunt.encode(42)]);
        let outcome = try_ability(
            &mut abilities,
            &world,
            Ability::Kick,
            now + Ability::Kick.reuse(),
        );
        assert_eq!(outcome.sent.len(), 1);
        // A Barbarian bashes without the skill; an untargeted ability needs
        // no target.
        let mut world = warrior();
        world.player.corrected().unwrap().race = 2;
        world.target = super::super::targeting::Target::from(42);
        let outcome = try_ability(&mut Abilities::default(), &world, Ability::Bash, now);
        assert_eq!(outcome.sent, [Ability::Bash.encode(42)]);
        let outcome = try_ability(&mut Abilities::default(), &world, Ability::Hide, now);
        assert_eq!(refused(&outcome.events), ["You do not have that ability"]);
    }
}
