//! The Titanium program's own checks for Fishing, Pick Pockets, Begging,
//! Sense Traps, Disarm Traps and Pick Lock (read R14, sections 0, 3 and 4;
//! R15c), in
//! its order, each refusal with the program's own line. The shared gates
//! come first: the skill known, a frozen posture, and the ability's wait,
//! which the program passes over without a word. Its mount gate is not
//! followed: the session does not know when the player rides.
use super::{refuse, Abilities, Refusal};
use crate::client::session::feature::{Out, World};
use anyhow::Result;
use eq_network_game::{
    abilities::{Ability, TREASURE_CLASS},
    inventory::InventorySlot,
    request::Request,
    world::{PostureState, SpawnKind, SpawnState},
    GameDialect,
};
use std::time::{Duration, Instant};

/// The program's lines (`eqstr_us.txt` string IDs), read R14.
mod lines {
    /// A frozen player uses no ability (section 0).
    pub(super) const FROZEN: u32 = 0x3050;
    /// Fishing (section 4).
    pub(super) const ALREADY_FISHING: u32 = 0x9c;
    pub(super) const FISHING_IN_WATER: u32 = 0x9d;
    pub(super) const FISHING_HOLDING: u32 = 0x9e;
    pub(super) const FISHING_ATTACKING: u32 = 0x9f;
    pub(super) const NO_POLE: u32 = 0xa0;
    pub(super) const POLE_NOT_HELD: u32 = 0xa1;
    pub(super) const NO_BAIT: u32 = 0xa2;
    pub(super) const FISHING_CAST: u32 = 0xa3;
    /// Begging (section 3).
    pub(super) const BEG_NOT_AN_NPC: u32 = 0x3071;
    pub(super) const BEG_ATTACKING: u32 = 0x3072;
    pub(super) const BEG_TOO_FAR: u32 = 0x3073;
    pub(super) const BEG_UNSEEN: u32 = 0x3074;
    /// Pick Pockets (section 3).
    pub(super) const PICK_NO_TARGET: u32 = 0x3075;
    pub(super) const PICK_STUNNED: u32 = 0x3077;
    pub(super) const PICK_ATTACKING: u32 = 0x3078;
    pub(super) const PICK_YOURSELF: u32 = 0x3079;
    pub(super) const PICK_PLAYER: u32 = 0x307a;
    pub(super) const PICK_NOT_PVP: u32 = 0x307b;
    pub(super) const PICK_TOO_FAR: u32 = 0x307e;
}

/// How long the program holds its fishing flag after a cast (actor +0x120,
/// read R14 section 4), apart from Fishing's recovery.
const FISHING_FLAG: Duration = Duration::from_millis(2000);
/// A fishing pole's and bait's item types (read R14 section 4).
const POLE: u8 = 0x24;
const BAIT: u8 = 0x25;
/// The hands a pole is held in: primary and secondary.
const HANDS: [InventorySlot; 2] = [InventorySlot(13), InventorySlot(14)];

/// Whether the program's own checks for it run here.
pub(super) const fn checks(ability: Ability) -> bool {
    matches!(
        ability,
        Ability::Fishing
            | Ability::PickPockets
            | Ability::Begging
            | Ability::SenseTraps
            | Ability::DisarmTraps
            | Ability::PickLock
    )
}

/// What the player sends, and the line printed as they do.
struct Use {
    requests: Vec<Request>,
    string_id: Option<u32>,
}

impl Abilities {
    /// Uses one of the abilities whose checks run here ([`checks`]).
    pub(super) fn use_class_tool(
        &mut self,
        (session_id, ability): (u64, Ability),
        in_liquid: bool,
        world: &World,
        out: &mut Out<'_, '_>,
        now: Instant,
    ) -> Result<()> {
        let used = self.class_tool(ability, in_liquid, world, out, now);
        let Use {
            requests,
            string_id,
        } = match used {
            Ok(used) => used,
            Err(refusal) => return refuse(session_id, ability, refusal, out),
        };
        let Some((last, before)) = requests.split_last() else {
            return Ok(());
        };
        for request in before {
            out.request(request)?;
        }
        if ability == Ability::Fishing {
            self.fishing_until = Some(now + FISHING_FLAG);
        }
        self.send_request((session_id, ability), last, string_id, out, now)
    }

    fn class_tool(
        &self,
        ability: Ability,
        in_liquid: bool,
        world: &World,
        out: &Out<'_, '_>,
        now: Instant,
    ) -> Result<Use, Refusal> {
        let ((own_id, position), player) = self.usable(ability, world, GameDialect::Titanium)?;
        if world.posture.current() == Some(PostureState::Frozen) {
            return Err(Refusal::official(
                "You cannot do that while frozen",
                lines::FROZEN,
                Vec::new(),
            ));
        }
        // The program passes over a press while the wait runs, silently.
        if self.waiting(ability, now) {
            return Err("Ability recovery time not yet met.".into());
        }
        let target = world
            .target
            .and_then(|id| world.spawns.visible(id).map(|spawn| (id, spawn)));
        let plain = |ability: Ability| Use {
            requests: vec![Request::Ability {
                ability,
                target: *world.target,
            }],
            string_id: None,
        };
        match ability {
            Ability::Fishing => {
                self.fishing(in_liquid, world, now)?;
                // A standing player casts the pole as they fish (R15c): the
                // character's own posture, 100.
                let standing = world.posture.current() == Some(PostureState::Standing);
                let mut requests = Vec::new();
                if standing {
                    requests.push(Request::FishingCast);
                }
                requests.push(Request::Ability {
                    ability,
                    target: None,
                });
                Ok(Use {
                    requests,
                    string_id: Some(lines::FISHING_CAST),
                })
            }
            Ability::PickPockets | Ability::Begging => {
                let reach = out.wire.reaches().bandage;
                let target = if ability == Ability::PickPockets {
                    pick_pockets(target, own_id, world)?
                } else {
                    begging(target, world)?
                };
                // The program measures its 15 units as the bandage's reach
                // does (recording checklist 6.431, inferred).
                if !reach.admits(position, target.position) {
                    let line = if ability == Ability::PickPockets {
                        lines::PICK_TOO_FAR
                    } else {
                        lines::BEG_TOO_FAR
                    };
                    return Err(Refusal::official(
                        "Your target is too far away",
                        line,
                        Vec::new(),
                    ));
                }
                if ability == Ability::Begging && own_unseen(world, own_id) {
                    return Err(Refusal::official(
                        "You cannot beg while unseen",
                        lines::BEG_UNSEEN,
                        Vec::new(),
                    ));
                }
                let skill = usize::try_from(ability.skill()).unwrap_or(usize::MAX);
                let skill_value = player
                    .skills
                    .as_deref()
                    .and_then(|skills| skills.get(skill).copied())
                    .unwrap_or_default();
                Ok(Use {
                    requests: vec![Request::AskOf {
                        ability,
                        target: target.spawn_id,
                        skill_value,
                    }],
                    string_id: None,
                })
            }
            // Pick Lock acts only on a treasure, and otherwise does nothing
            // and says nothing (R14 section 3).
            Ability::PickLock => Ok(Use {
                requests: target
                    .filter(|(_, spawn)| spawn.class == Some(TREASURE_CLASS))
                    .map(|_| Request::AtTreasure(ability))
                    .into_iter()
                    .collect(),
                string_id: None,
            }),
            // No other check; a target of the treasure class takes the
            // program's other message (recording checklist 6.432).
            _ => Ok(
                if target.is_some_and(|(_, spawn)| spawn.class == Some(TREASURE_CLASS)) {
                    Use {
                        requests: vec![Request::AtTreasure(ability)],
                        string_id: None,
                    }
                } else {
                    plain(ability)
                },
            ),
        }
    }

    /// Fishing's own checks after the shared gates, in the program's order
    /// (read R14 section 4). There is no check for water close by; the
    /// server decides that.
    fn fishing(&self, in_liquid: bool, world: &World, now: Instant) -> Result<(), Refusal> {
        if self.fishing_until.is_some_and(|until| now < until) {
            return Err(Refusal::official(
                "You are already fishing",
                lines::ALREADY_FISHING,
                Vec::new(),
            ));
        }
        if in_liquid {
            return Err(Refusal::official(
                "You cannot fish while in the water",
                lines::FISHING_IN_WATER,
                Vec::new(),
            ));
        }
        let inventory = &world.inventory;
        if !inventory.received() || inventory.stale() {
            return Err("Equipment is not known yet".into());
        }
        let items = inventory.items();
        if items.contains_key(&InventorySlot::CURSOR) {
            return Err(Refusal::official(
                "Put down the item you are holding first",
                lines::FISHING_HOLDING,
                Vec::new(),
            ));
        }
        if world.attacking {
            return Err(Refusal::official(
                "You cannot fish while attacking",
                lines::FISHING_ATTACKING,
                Vec::new(),
            ));
        }
        let carried = |item_type: u8| {
            items.iter().any(|(slot, item)| {
                item.rules.item_type == item_type && (slot.is_equipment() || slot.is_carried())
            })
        };
        if !carried(POLE) {
            return Err(Refusal::official(
                "You have no fishing pole",
                lines::NO_POLE,
                Vec::new(),
            ));
        }
        if !HANDS.iter().any(|hand| {
            items
                .get(hand)
                .is_some_and(|item| item.rules.item_type == POLE)
        }) {
            return Err(Refusal::official(
                "Hold your fishing pole in your hand",
                lines::POLE_NOT_HELD,
                Vec::new(),
            ));
        }
        if !carried(BAIT) {
            return Err(Refusal::official(
                "You have no bait",
                lines::NO_BAIT,
                Vec::new(),
            ));
        }
        Ok(())
    }
}

/// Pick Pockets' checks of its target and the player, in the program's
/// order up to its reach (read R14 section 3). A GM target is passed over
/// silently there; the session does not know another player's GM flag.
fn pick_pockets<'w>(
    target: Option<(u16, &'w SpawnState)>,
    own_id: u16,
    world: &World,
) -> Result<&'w SpawnState, Refusal> {
    let Some((target_id, spawn)) = target else {
        return Err(Refusal::official(
            "Select a target for that first",
            lines::PICK_NO_TARGET,
            Vec::new(),
        ));
    };
    // The program takes only a player or an NPC, silently.
    if !matches!(spawn.kind, SpawnKind::Player | SpawnKind::Npc) {
        return Err("You cannot use that on your target".into());
    }
    if world.control.stunned() {
        return Err(Refusal::official(
            "You cannot do that while stunned",
            lines::PICK_STUNNED,
            Vec::new(),
        ));
    }
    if world.attacking {
        return Err(Refusal::official(
            "You cannot do that while attacking",
            lines::PICK_ATTACKING,
            Vec::new(),
        ));
    }
    if spawn.kind == SpawnKind::Player {
        if target_id == own_id {
            return Err(Refusal::official(
                "You cannot pick your own pockets",
                lines::PICK_YOURSELF,
                Vec::new(),
            ));
        }
        // The program allows a player's pockets only under its PvP rules
        // (read R16d): each spawn's flag, from its appearance updates. Its
        // first line also waits on a server ruleset of 4, which the session
        // does not read: it refuses as under any other (recording
        // checklist 6.438).
        if !spawn.appearance.pvp {
            return Err(Refusal::official(
                "That player is not open to player combat",
                lines::PICK_PLAYER,
                Vec::new(),
            ));
        }
        let Some(own) = world
            .spawns
            .visible(own_id)
            .filter(|own| own.appearance.pvp)
        else {
            return Err(Refusal::official(
                "You are not open to player combat",
                lines::PICK_NOT_PVP,
                Vec::new(),
            ));
        };
        // Then a level range of five each way, whose two lines (0x307c and
        // 0x307d) are not yet told apart by the read: refused in this
        // library's words (6.438).
        if own.level.abs_diff(spawn.level) > 5 {
            return Err("That player's level is too far from yours".into());
        }
    }
    Ok(spawn)
}

/// Begging's check of its target and the player before its reach (read R14
/// section 3).
fn begging<'w>(
    target: Option<(u16, &'w SpawnState)>,
    world: &World,
) -> Result<&'w SpawnState, Refusal> {
    let Some((_, spawn)) = target.filter(|(_, spawn)| spawn.kind == SpawnKind::Npc) else {
        return Err(Refusal::official(
            "You can beg only from an NPC",
            lines::BEG_NOT_AN_NPC,
            Vec::new(),
        ));
    };
    if world.attacking {
        return Err(Refusal::official(
            "You cannot beg while attacking",
            lines::BEG_ATTACKING,
            Vec::new(),
        ));
    }
    Ok(spawn)
}

/// Whether the player cannot be seen, which the program reads from its own
/// +0x250; the session reads the player's own spawn's invisibility
/// (recording checklist 6.430, inferred).
fn own_unseen(world: &World, own_id: u16) -> bool {
    world
        .spawns
        .all()
        .any(|spawn| spawn.spawn_id == own_id && spawn.invisible)
}

#[cfg(test)]
mod tests {
    use super::super::super::feature::testing;
    use super::*;
    use crate::client::session::{feature::Feature, ClientEvent};
    use eq_network_game::{
        inventory::{Inventory, InventoryUpdate},
        message::Message,
        world::{Position, WorldEvent},
    };

    /// An admitted player (7) with these skills, standing beside an NPC (42)
    /// and a player (46) ten units away, and an NPC (43) twenty units away.
    fn player(skills: &[(usize, u32)]) -> World {
        let mut world = World::new(5);
        let mut player = testing::player(7);
        let mut values = vec![0; 100];
        for &(skill, value) in skills {
            values[skill] = value;
        }
        player.skills = Some(values);
        world.player.admit(player);
        world.own_spawn = Some(7);
        world.spawns.insert(testing::spawn(7, SpawnKind::Player));
        world.spawns.insert(testing::spawn(42, SpawnKind::Npc));
        for (id, kind, x) in [(43, SpawnKind::Npc, 20.0), (46, SpawnKind::Player, 10.0)] {
            let mut spawn = testing::spawn(id, kind);
            spawn.position = Position {
                x,
                ..Position::default()
            };
            world.spawns.insert(spawn);
        }
        world.posture.observed(PostureState::Standing);
        world
    }

    fn carrying(world: &mut World, items: &[(i32, u8)]) {
        let items = items
            .iter()
            .map(|&(slot, item_type)| {
                let mut item = testing::item(slot);
                item.rules.item_type = item_type;
                if slot == 22 {
                    item.bag_slots = 8;
                }
                item
            })
            .collect();
        let mut inventory = Inventory::default();
        inventory.apply(InventoryUpdate::Snapshot(items));
        world.inventory = inventory.into();
    }

    fn press(
        abilities: &mut Abilities,
        world: &World,
        (ability, in_liquid): (Ability, bool),
        now: Instant,
    ) -> testing::Outcome<Result<()>> {
        testing::run(|out| abilities.use_class_tool((5, ability), in_liquid, world, out, now))
    }

    /// The program's line for the one refusal, or for the use.
    fn line(outcome: &testing::Outcome<Result<()>>) -> Option<u32> {
        outcome.result.as_ref().unwrap();
        match &outcome.events[..] {
            [ClientEvent::World(WorldEvent::AbilityRefused { string_id, .. }), ..]
            | [ClientEvent::World(WorldEvent::AbilityUsed { string_id, .. })] => *string_id,
            other => panic!("{other:?}"),
        }
    }

    fn opcodes(outcome: &testing::Outcome<Result<()>>) -> Vec<u16> {
        outcome.sent.iter().map(|sent| sent.opcode).collect()
    }

    #[test]
    fn fishing_checks_run_in_the_programs_order_and_a_standing_cast_shows_the_pole() {
        let mut abilities = Abilities::new(&Ability::ALL);
        let mut world = player(&[]);
        let now = Instant::now();
        // Everything wrong at once: each fix shows the next line.
        carrying(&mut world, &[(30, 0), (22, 0), (251, POLE)]);
        world.attacking = true;
        let fish = |in_liquid| (Ability::Fishing, in_liquid);
        for (in_liquid, expected) in [(true, 0x9d), (false, 0x9e)] {
            let outcome = press(&mut abilities, &world, fish(in_liquid), now);
            assert_eq!(line(&outcome), Some(expected));
            assert_eq!(outcome.sent, []);
        }
        carrying(&mut world, &[(22, 0), (251, POLE)]);
        assert_eq!(
            line(&press(&mut abilities, &world, fish(false), now)),
            Some(0x9f)
        );
        world.attacking = false;
        // The pole is in a bag, not in hand.
        assert_eq!(
            line(&press(&mut abilities, &world, fish(false), now)),
            Some(0xa1)
        );
        carrying(&mut world, &[(22, 0)]);
        assert_eq!(
            line(&press(&mut abilities, &world, fish(false), now)),
            Some(0xa0)
        );
        carrying(&mut world, &[(14, POLE)]);
        assert_eq!(
            line(&press(&mut abilities, &world, fish(false), now)),
            Some(0xa2)
        );
        carrying(&mut world, &[(14, POLE), (22, 0), (252, BAIT)]);
        let cast = press(&mut abilities, &world, fish(false), now);
        assert_eq!(line(&cast), Some(0xa3));
        assert_eq!(opcodes(&cast), [0x2acf, 0x0b36]);
        assert_eq!(cast.sent[0].body, [7, 0, 5, 5]);
        assert_eq!(cast.sent[1].body, [0_u8; 0]);
        // The wait runs first and says nothing of the program's.
        let again = press(&mut abilities, &world, fish(false), now);
        assert_eq!(line(&again), None);
        assert_eq!(again.sent, []);
        // With the server's word that it is ready again, the program's own
        // two-second flag still holds the next cast.
        abilities
            .admit(&Message::Event(WorldEvent::SkillReady(55)), &mut world)
            .unwrap();
        let soon = now + Duration::from_millis(1500);
        assert_eq!(
            line(&press(&mut abilities, &world, fish(false), soon)),
            Some(0x9c)
        );
        // Sitting, the player fishes without casting the pole.
        world.posture.observed(PostureState::Sitting);
        let later = now + Duration::from_millis(2100);
        let seated = press(&mut abilities, &world, fish(false), later);
        assert_eq!(opcodes(&seated), [0x0b36]);
        assert_eq!(line(&seated), Some(0xa3));
    }

    #[test]
    fn a_frozen_player_uses_none_of_them() {
        let mut abilities = Abilities::new(&Ability::ALL);
        let mut world = player(&[(48, 20), (62, 20)]);
        world.posture.observed(PostureState::Frozen);
        for ability in [Ability::Fishing, Ability::PickPockets, Ability::SenseTraps] {
            let outcome = press(&mut abilities, &world, (ability, false), Instant::now());
            assert_eq!(line(&outcome), Some(0x3050), "{ability:?}");
            assert_eq!(outcome.sent, []);
        }
    }

    #[test]
    fn pick_pockets_asks_of_an_npc_close_by_with_the_players_own_skill() {
        let mut abilities = Abilities::new(&Ability::ALL);
        let mut world = player(&[(48, 37)]);
        let pick = (Ability::PickPockets, false);
        let now = Instant::now();
        assert_eq!(
            line(&press(&mut abilities, &world, pick, now)),
            Some(0x3075)
        );
        world.target = crate::client::session::targeting::Target::from(43);
        world.attacking = true;
        assert_eq!(
            line(&press(&mut abilities, &world, pick, now)),
            Some(0x3078)
        );
        world.attacking = false;
        assert_eq!(
            line(&press(&mut abilities, &world, pick, now)),
            Some(0x307e)
        );
        // A player's pockets: never your own, and only with both open to
        // player combat.
        world.target = crate::client::session::targeting::Target::from(7);
        assert_eq!(
            line(&press(&mut abilities, &world, pick, now)),
            Some(0x3079)
        );
        world.target = crate::client::session::targeting::Target::from(46);
        assert_eq!(
            line(&press(&mut abilities, &world, pick, now)),
            Some(0x307a)
        );
        let set = |world: &mut World, id, level: Option<u8>| {
            let mut spawn = world.spawns.visible(id).unwrap().clone();
            spawn.appearance.pvp = true;
            spawn.level = level.unwrap_or(spawn.level);
            world.spawns.insert(spawn);
        };
        let open = |world: &mut World, id| set(world, id, None);
        open(&mut world, 46);
        assert_eq!(
            line(&press(&mut abilities, &world, pick, now)),
            Some(0x307b)
        );
        open(&mut world, 7);
        // Within five levels each way, its line not yet told apart.
        let own_level = world.spawns.visible(7).unwrap().level;
        set(&mut world, 46, Some(own_level + 6));
        assert_eq!(line(&press(&mut abilities, &world, pick, now)), None);
        assert_eq!(press(&mut abilities, &world, pick, now).sent, []);
        set(&mut world, 46, Some(own_level + 5));
        let mut fresh = Abilities::new(&Ability::ALL);
        let player_picked = press(&mut fresh, &world, pick, now);
        assert_eq!(line(&player_picked), None);
        assert_eq!(opcodes(&player_picked), [0x2ad8]);
        world.target = crate::client::session::targeting::Target::from(42);
        let picked = press(&mut abilities, &world, pick, now);
        assert_eq!(line(&picked), None);
        assert_eq!(
            picked.sent[0].body,
            [42, 0, 0, 0, 7, 0, 0, 0, 37, 0, 0, 0, 0xff, 0, 0, 0, 0, 0]
        );
        assert_eq!(opcodes(&picked), [0x2ad8]);
        // Its own ten-second wait, apart from Begging's.
        assert_eq!(press(&mut abilities, &world, pick, now).sent, []);
        let mut beggar = player(&[(48, 37), (67, 5)]);
        beggar.target = crate::client::session::targeting::Target::from(42);
        assert_eq!(
            opcodes(&press(
                &mut abilities,
                &beggar,
                (Ability::Begging, false),
                now
            )),
            [0x13e7]
        );
        let later = now + Duration::from_secs(10);
        assert_eq!(
            opcodes(&press(&mut abilities, &world, pick, later)),
            [0x2ad8]
        );
    }

    #[test]
    fn begging_wants_an_npc_close_by_and_the_player_seen() {
        let mut abilities = Abilities::new(&Ability::ALL);
        let mut world = player(&[(67, 12)]);
        let beg = (Ability::Begging, false);
        let now = Instant::now();
        assert_eq!(line(&press(&mut abilities, &world, beg, now)), Some(0x3071));
        world.target = crate::client::session::targeting::Target::from(46);
        assert_eq!(line(&press(&mut abilities, &world, beg, now)), Some(0x3071));
        world.target = crate::client::session::targeting::Target::from(43);
        world.attacking = true;
        assert_eq!(line(&press(&mut abilities, &world, beg, now)), Some(0x3072));
        world.attacking = false;
        assert_eq!(line(&press(&mut abilities, &world, beg, now)), Some(0x3073));
        world.target = crate::client::session::targeting::Target::from(42);
        let mut unseen = world.spawns.get(7).unwrap().clone();
        unseen.invisible = true;
        world.spawns.insert(unseen);
        assert_eq!(line(&press(&mut abilities, &world, beg, now)), Some(0x3074));
        let mut seen = world.spawns.get(7).unwrap().clone();
        seen.invisible = false;
        world.spawns.insert(seen);
        let begged = press(&mut abilities, &world, beg, now);
        assert_eq!(opcodes(&begged), [0x13e7]);
        assert_eq!(begged.sent[0].body[8], 12);
    }

    #[test]
    fn the_trap_skills_send_their_treasure_messages_only_at_a_treasure() {
        let mut abilities = Abilities::new(&Ability::ALL);
        let mut world = player(&[(17, 5), (62, 5)]);
        let now = Instant::now();
        // No target is needed.
        let sensed = press(&mut abilities, &world, (Ability::SenseTraps, false), now);
        assert_eq!(opcodes(&sensed), [0x5666]);
        let mut chest = testing::spawn(50, SpawnKind::Npc);
        chest.class = Some(TREASURE_CLASS);
        world.spawns.insert(chest);
        world.target = crate::client::session::targeting::Target::from(50);
        let disarmed = press(&mut abilities, &world, (Ability::DisarmTraps, false), now);
        assert_eq!(opcodes(&disarmed), [0x41b9]);
        let later = now + Duration::from_secs(10);
        let sensed = press(&mut abilities, &world, (Ability::SenseTraps, false), later);
        assert_eq!(opcodes(&sensed), [0x07a8]);
        // Without the skill, nothing goes.
        let untrained = player(&[]);
        let outcome = press(
            &mut abilities,
            &untrained,
            (Ability::SenseTraps, false),
            later,
        );
        assert_eq!(outcome.sent, []);
    }

    #[test]
    fn pick_lock_acts_only_on_a_treasure_and_otherwise_says_nothing() {
        let mut abilities = Abilities::new(&Ability::ALL);
        let mut world = player(&[(35, 5)]);
        let pick = (Ability::PickLock, false);
        let now = Instant::now();
        for target in [None, Some(42)] {
            world.target = target.map_or_else(Default::default, |id| {
                crate::client::session::targeting::Target::from(id)
            });
            let outcome = press(&mut abilities, &world, pick, now);
            outcome.result.as_ref().unwrap();
            assert_eq!((outcome.sent.len(), outcome.events.len()), (0, 0));
        }
        let mut chest = testing::spawn(50, SpawnKind::Npc);
        chest.class = Some(TREASURE_CLASS);
        world.spawns.insert(chest);
        world.target = crate::client::session::targeting::Target::from(50);
        let picked = press(&mut abilities, &world, pick, now);
        assert_eq!(opcodes(&picked), [0x069d]);
        assert_eq!(picked.sent[0].body, Vec::<u8>::new());
    }
}
