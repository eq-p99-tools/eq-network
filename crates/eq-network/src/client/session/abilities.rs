//! Using abilities: the session refuses, with the reason, what servers
//! would ignore without a word (an unknown skill, no target, a target out
//! of reach) and what they would answer only with a complaint (a timer still
//! running), and sends the rest. It keeps each recovery timer, as long as the
//! server's at most, and tells the host when one starts.
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    abilities::{in_melee_range, player_size, Ability, Body, Recovery},
    request::Request,
    world::{PlayerState, Position, SpawnKind, WorldEvent},
};
use std::{collections::BTreeMap, time::Instant};

/// Uses abilities and keeps their recovery timers.
#[derive(Default)]
pub(super) struct Abilities {
    /// When each timer the player started runs out.
    ready: BTreeMap<Recovery, Instant>,
}

/// Why the player cannot use the ability on their target, if they cannot.
fn target_refusal(
    ability: Ability,
    world: &World,
    (own_id, position, player): (u16, Position, &PlayerState),
) -> Option<&'static str> {
    let Some((target_id, target)) = world
        .target
        .and_then(|id| world.spawns.visible(id).map(|spawn| (id, spawn)))
    else {
        return Some("You must first select a target for this ability!");
    };
    // Servers taunt only an NPC, ignoring anything else after starting the
    // timer; a strike may also hit another player where the server allows it.
    let fits = match target.kind {
        SpawnKind::Npc => true,
        SpawnKind::Player => ability.strikes(),
        _ => false,
    };
    if target_id == own_id || !fits {
        return Some("You cannot use that on your target");
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
    (!in_melee_range(own, theirs)).then_some("Your target is too far away, get closer!")
}

impl Abilities {
    /// Why the player cannot use the ability now, if they cannot.
    fn refusal(&self, ability: Ability, world: &World, now: Instant) -> Option<&'static str> {
        let Some(((own_id, position), player)) = world.player_at().zip(world.player.as_ref())
        else {
            return Some("Not in the zone yet");
        };
        let skills = player.skills.as_deref().unwrap_or_default();
        if !ability.known(skills, player.race) {
            return Some("You do not have that ability");
        }
        if ability.at_target() {
            if let Some(reason) = target_refusal(ability, world, (own_id, position, player)) {
                return Some(reason);
            }
        }
        let running = ability
            .recovery()
            .and_then(|recovery| self.ready.get(&recovery))
            .is_some_and(|ready| now < *ready);
        running.then_some("Ability recovery time not yet met.")
    }

    fn use_ability(
        &mut self,
        (session_id, ability): (u64, Ability),
        world: &World,
        out: &mut Out<'_, '_>,
        now: Instant,
    ) -> Result<()> {
        if let Some(reason) = self.refusal(ability, world, now) {
            out.log
                .send(ClientEvent::World(WorldEvent::AbilityRefused {
                    session_id,
                    reason: reason.into(),
                }))?;
            return out.log.diagnostic(format!("{ability:?} refused: {reason}"));
        }
        out.request(&Request::Ability {
            ability,
            target: world.target.unwrap_or(0),
        })?;
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

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::UseAbility { .. })
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
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::world::Position;

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
        assert_eq!(
            refused(&outcome.events),
            ["You must first select a target for this ability!"]
        );
        world.target = super::super::targeting::Target::from(43);
        let outcome = try_ability(&mut abilities, &world, Ability::Kick, now);
        assert_eq!(
            refused(&outcome.events),
            ["Your target is too far away, get closer!"]
        );
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
        assert_eq!(
            refused(&outcome.events),
            ["Your target is too far away, get closer!"]
        );
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
