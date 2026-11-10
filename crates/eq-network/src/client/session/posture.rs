//! The player's own stance. Titanium servers never echo it (`EQEmu`'s
//! `Mob::SetAppearance` skips the sender), so the session keeps the stance it
//! last sent or was told, reports it to the client, and stands the character up
//! before it moves, as the official client does; a server leaves a seated
//! character seated while it walks.
use super::{
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent, Events,
};
use anyhow::Result;
use eq_network_game::{
    combat::{Damage, DamageOutcome},
    command::Posture,
    message::Message,
    request::Request,
    world::{PostureState, WorldEvent},
};

/// Explicit stance commands, independent of locomotion. Camping clients can
/// sit and stand without being granted walking, falling or jumping.
pub(super) struct Stances;

impl Feature for Stances {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Posture]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::SetPosture { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::SetPosture {
            spawn_id, posture, ..
        } = *command
        else {
            return Ok(());
        };
        if !world.is_player(spawn_id) {
            return out
                .log
                .diagnostic("Rejected a posture for another spawn".into());
        }
        world.posture.set(spawn_id, posture, out)
    }

    fn observe(
        &mut self,
        message: &Message,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        match message {
            Message::Event(WorldEvent::Posture { spawn_id, posture })
                if world.is_player(*spawn_id) =>
            {
                world.posture.observed(*posture);
                // The program applies a server's posture for the player as
                // its own (read R19 f), so a lying report holds them still
                // as the press does.
                if *posture == PostureState::Lying && out.wire.dialect().lying().holds_still {
                    world.body.stop_voluntary();
                }
                Ok(())
            }
            Message::Event(WorldEvent::Damage(damage)) => stand_when_hit(damage, world, out),
            // Death ends any feign the server held.
            Message::Event(WorldEvent::Death(death))
                if world.own_spawn.map(u32::from) == Some(death.spawn_id) =>
            {
                world.posture.forget_owed_stand();
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// Stands the player up when a hit lands while they sit or kneel to loot,
/// as the Titanium program's `OP_Damage` handler does (read R19 b, 0x4a56e3,
/// rechecked 11-10): any record of more than 0 to the player, whatever its
/// type, and never a miss. Lying and ducking stay as they are. A player who
/// chose to stay seated when hit (6.453) is left as they are. The handler's
/// early return for a source it does not know is kept; the one for a spell
/// it does not know is not, since the session has no spell table (6.455).
fn stand_when_hit(damage: &Damage, world: &mut World, out: &mut Out<'_, '_>) -> Result<()> {
    let (Some(spawn_id), DamageOutcome::Hit(1..)) = (world.own_spawn, damage.outcome) else {
        return Ok(());
    };
    let known = |id: u16| id == 0 || world.is_player(id) || world.spawns.get(id).is_some();
    if damage.target_id != spawn_id || !known(damage.source_id) {
        return Ok(());
    }
    let stays = world
        .body
        .options()
        .is_some_and(|options| options.posture.stay_seated_when_hit);
    let stands = matches!(
        world.posture.current(),
        Some(PostureState::Sitting | PostureState::Looting)
    ) && out.wire.dialect().lying().hit_stands
        && !stays;
    if stands {
        world.posture.set(spawn_id, Posture::Standing, out)?;
    }
    Ok(())
}

/// The player's stance in this zone admission; unknown counts as standing.
#[derive(Debug, Default)]
pub(super) struct OwnPosture {
    /// The last requested or server-reported stance.
    posture: Option<PostureState>,
    /// A Standing report the server still needs although the player already
    /// stands, which goes out once with the next stand: a Feign Death the
    /// player pressed standing (6.454), whose feign only a posture report
    /// ends, or a camp, whose timer only a Standing report stops.
    stand_owed: bool,
}

impl OwnPosture {
    /// Reports the requested lying posture, without asserting feign success.
    /// The servers accept the stance but exclude the sender from their
    /// broadcast. Both programs' posture setters (Titanium 0x483556 from
    /// read R19 a, `EQMac` 0x50be3c from the feign-death case of its skill
    /// dispatcher 0x4bdf2f) return at once when the player already lies, so
    /// nothing is sent then.
    pub(super) fn lie_for_feign(&mut self, spawn_id: u16, out: &mut Out<'_, '_>) -> Result<()> {
        if self.posture == Some(PostureState::Lying) {
            return Ok(());
        }
        out.request(&Request::FeignDeathPosture)?;
        self.posture = Some(PostureState::Lying);
        self.stand_owed = false;
        out.log.send(ClientEvent::World(WorldEvent::Posture {
            spawn_id,
            posture: PostureState::Lying,
        }))
    }

    /// The last requested or server-reported stance.
    pub(super) fn current(&self) -> Option<PostureState> {
        self.posture
    }
    /// Notes a stance the server reported for the player, which ends a feign
    /// pressed standing as any posture report does.
    pub(super) fn observed(&mut self, posture: PostureState) {
        self.posture = Some(posture);
        self.stand_owed = false;
    }

    /// Notes that the next stand must reach the server even if the player
    /// already stands: a Feign Death pressed standing (6.454) or a camp to
    /// cancel.
    pub(super) fn owe_stand(&mut self) {
        self.stand_owed = true;
    }

    /// Whether the next stand still goes out though the player stands.
    #[cfg(test)]
    pub(super) const fn stand_owed(&self) -> bool {
        self.stand_owed
    }

    /// Forgets an owed stand without a report, as death does.
    pub(super) fn forget_owed_stand(&mut self) {
        self.stand_owed = false;
    }

    /// Sits, stands or crouches the player: sends the stance, notes it and
    /// reports it, since the server will not. Every feature that changes the
    /// player's stance does it here.
    ///
    /// # Errors
    /// Returns an error when the connection or the host's event handler fails.
    pub(super) fn set(
        &mut self,
        spawn_id: u16,
        posture: Posture,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        // Titanium's posture routine changes nothing for the posture the
        // player already has (read R19), but an owed stand still goes.
        let unchanged = self.posture == Some(state(posture))
            && !(posture == Posture::Standing && self.stand_owed);
        if unchanged && out.wire.dialect().skips_unchanged_posture() {
            return Ok(());
        }
        out.request(&Request::Posture(posture))?;
        self.sent(spawn_id, posture, out.log)
    }

    /// Notes a stance this session sent, and reports it since the server will not.
    fn sent(&mut self, spawn_id: u16, posture: Posture, log: &mut Events<'_>) -> Result<()> {
        let posture = state(posture);
        self.posture = Some(posture);
        self.stand_owed = false;
        log.send(ClientEvent::World(WorldEvent::Posture {
            spawn_id,
            posture,
        }))
    }

    /// Whether the character must stand up before it can move. Where the
    /// program holds a lying player still, nothing stands them to move
    /// (read R19 c).
    fn grounded(&self, lying_holds_still: bool) -> bool {
        match self.posture {
            Some(PostureState::Sitting | PostureState::Ducking) => true,
            Some(PostureState::Lying) => !lying_holds_still,
            _ => false,
        }
    }

    /// Stands a seated, crouched or prone character up before it moves.
    pub(super) fn stand_to_move(&mut self, spawn_id: u16, out: &mut Out<'_, '_>) -> Result<()> {
        if !self.grounded(out.wire.dialect().lying().holds_still) {
            return Ok(());
        }
        self.set(spawn_id, Posture::Standing, out)
    }
}

/// The stance a posture request leaves the player in.
const fn state(posture: Posture) -> PostureState {
    match posture {
        Posture::Standing => PostureState::Standing,
        Posture::Sitting => PostureState::Sitting,
        Posture::Ducking => PostureState::Ducking,
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::command;
    use std::time::Instant;
    #[test]
    fn the_player_stance_goes_out_and_is_reported_since_the_server_never_echoes_it() {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        let mut stances = Stances;
        let sit = |spawn_id| ClientCommand::SetPosture {
            session_id: 5,
            spawn_id,
            posture: command::Posture::Sitting,
            created: Instant::now(),
        };
        let outcome = testing::run(|out| stances.handle(&sit(7), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [command::titanium_posture(7, command::Posture::Sitting).unwrap()]
        );
        assert!(matches!(
            outcome.events[..],
            [ClientEvent::World(WorldEvent::Posture {
                spawn_id: 7,
                posture: PostureState::Sitting
            })]
        ));
        let outcome = testing::run(|out| stances.handle(&sit(8), &mut world, out));
        outcome.result.unwrap();
        assert!(
            outcome.sent.is_empty(),
            "expected nothing sent, got {:?}",
            outcome.sent
        );
    }

    /// The lying appearance goes out before a feign unless the player
    /// already lies, when the program's posture setter sends nothing.
    #[test]
    fn the_feign_posture_goes_out_once_until_the_player_stands_again() {
        const EQMAC: &dyn super::super::wire::Wire = &super::super::wire::EqMac;
        let mut posture = OwnPosture::default();
        let outcome = testing::run_on(EQMAC, |out| posture.lie_for_feign(7, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [eq_network_game::abilities::eqmac::feign_posture(7).unwrap()]
        );
        assert_eq!(posture.current(), Some(PostureState::Lying));
        let outcome = testing::run_on(EQMAC, |out| posture.lie_for_feign(7, out));
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty() && outcome.events.is_empty());
        posture.observed(PostureState::Standing);
        let outcome = testing::run_on(EQMAC, |out| posture.lie_for_feign(7, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
    }

    fn hit(target_id: u16, source_id: u16, outcome: DamageOutcome) -> Message {
        Message::Event(WorldEvent::Damage(Damage {
            target_id,
            source_id,
            kind: 231,
            spell_id: Some(5),
            outcome,
            push: None,
        }))
    }

    fn hit_world(posture: PostureState) -> World {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        world.player.admit(testing::player(7));
        world
            .spawns
            .insert(testing::spawn(42, eq_network_game::world::SpawnKind::Npc));
        world.posture.observed(posture);
        world
    }

    /// Titanium's `OP_Damage` handler stands a sitter or a looter on any
    /// record over 0 to them, whatever its type, and leaves a lying or
    /// ducking player as they are (read R19 b and R19b 2).
    #[test]
    fn a_hit_stands_a_sitter_or_a_looter_and_leaves_a_lying_player_lying() {
        for (posture, stands) in [
            (PostureState::Sitting, true),
            (PostureState::Looting, true),
            (PostureState::Lying, false),
            (PostureState::Ducking, false),
            (PostureState::Standing, false),
            (PostureState::Frozen, false),
        ] {
            let mut world = hit_world(posture);
            let outcome = testing::run(|out| {
                Stances.observe(&hit(7, 42, DamageOutcome::Hit(3)), &mut world, out)
            });
            outcome.result.unwrap();
            let expected = if stands {
                vec![command::titanium_posture(7, command::Posture::Standing).unwrap()]
            } else {
                Vec::new()
            };
            assert_eq!(outcome.sent, expected, "{posture:?}");
            let now = if stands {
                PostureState::Standing
            } else {
                posture
            };
            assert_eq!(world.posture.current(), Some(now), "{posture:?}");
        }
    }

    #[test]
    fn misses_hits_on_others_and_unknown_sources_stand_no_one() {
        for message in [
            hit(7, 42, DamageOutcome::Miss),
            hit(7, 42, DamageOutcome::Hit(0)),
            hit(42, 7, DamageOutcome::Hit(3)),
            hit(7, 99, DamageOutcome::Hit(3)),
        ] {
            let mut world = hit_world(PostureState::Sitting);
            let outcome = testing::run(|out| Stances.observe(&message, &mut world, out));
            outcome.result.unwrap();
            assert!(outcome.sent.is_empty(), "{message:?}");
            assert_eq!(world.posture.current(), Some(PostureState::Sitting));
        }
        // A record with no source, such as a fall, still counts.
        let mut world = hit_world(PostureState::Sitting);
        let outcome =
            testing::run(|out| Stances.observe(&hit(7, 0, DamageOutcome::Hit(3)), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
    }

    /// A world whose player chose to stay seated when hit (6.453).
    fn choosing_world(posture: PostureState) -> World {
        let start = Instant::now();
        let mut world = hit_world(posture);
        let mut motion = eq_network_game::movement::MotionSession::new(
            5,
            7,
            eq_network_game::world::Position::default(),
            start,
        )
        .unwrap();
        motion
            .set_options(
                eq_network_game::movement::MovementOptions {
                    posture: eq_network_game::movement::PostureOptions {
                        stay_seated_when_hit: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                start,
            )
            .unwrap();
        world.body.admit(motion);
        world
    }

    #[test]
    fn a_hit_leaves_a_sitter_or_looter_seated_only_when_they_chose_it() {
        for posture in [
            PostureState::Sitting,
            PostureState::Looting,
            PostureState::Lying,
        ] {
            let mut world = choosing_world(posture);
            let outcome = testing::run(|out| {
                Stances.observe(&hit(7, 42, DamageOutcome::Hit(3)), &mut world, out)
            });
            outcome.result.unwrap();
            assert!(outcome.sent.is_empty(), "{posture:?}: {:?}", outcome.sent);
            assert_eq!(world.posture.current(), Some(posture));
        }
    }

    #[test]
    fn the_mac_client_stands_no_one_when_hit() {
        const EQMAC: &dyn super::super::wire::Wire = &super::super::wire::EqMac;
        for posture in [
            PostureState::Sitting,
            PostureState::Looting,
            PostureState::Lying,
        ] {
            let mut world = hit_world(posture);
            let outcome = testing::run_on(EQMAC, |out| {
                Stances.observe(&hit(7, 42, DamageOutcome::Hit(3)), &mut world, out)
            });
            outcome.result.unwrap();
            assert!(outcome.sent.is_empty(), "{posture:?}: {:?}", outcome.sent);
        }
    }

    fn ask(posture: command::Posture) -> ClientCommand {
        ClientCommand::SetPosture {
            session_id: 5,
            spawn_id: 7,
            posture,
            created: Instant::now(),
        }
    }

    /// Titanium's posture routine changes nothing for the posture the player
    /// has (read R19); a feign pressed standing (6.454) is ended by one stand.
    #[test]
    fn a_stand_ends_a_feign_pressed_standing_once() {
        let mut world = hit_world(PostureState::Standing);
        let stand = ask(command::Posture::Standing);
        let outcome = testing::run(|out| Stances.handle(&stand, &mut world, out));
        outcome.result.unwrap();
        assert!(
            outcome.sent.is_empty(),
            "already standing: {:?}",
            outcome.sent
        );
        world.posture.owe_stand();
        let outcome = testing::run(|out| Stances.handle(&stand, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [command::titanium_posture(7, command::Posture::Standing).unwrap()]
        );
        let outcome = testing::run(|out| Stances.handle(&stand, &mut world, out));
        outcome.result.unwrap();
        assert!(
            outcome.sent.is_empty(),
            "a second stand: {:?}",
            outcome.sent
        );
        // A sit ends it too, and a stand after it is an ordinary stand.
        world.posture.owe_stand();
        let outcome =
            testing::run(|out| Stances.handle(&ask(command::Posture::Sitting), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [command::titanium_posture(7, command::Posture::Sitting).unwrap()]
        );
        assert!(!world.posture.stand_owed());
        // A report from the server and the player's death end it as well.
        world.posture.owe_stand();
        world.posture.observed(PostureState::Standing);
        assert!(!world.posture.stand_owed());
        world.posture.owe_stand();
        let death = Message::Event(WorldEvent::Death(eq_network_game::zoning::Death {
            spawn_id: 7,
            killer_id: 42,
            corpse_id: 7,
            bind_zone_id: 1,
            corpse_name: None,
        }));
        testing::run(|out| Stances.observe(&death, &mut world, out))
            .result
            .unwrap();
        assert!(!world.posture.stand_owed());
    }

    /// The Mac-era client's routine is unread, so every request but a second
    /// lie still goes.
    #[test]
    fn the_mac_client_still_sends_an_unchanged_posture() {
        const EQMAC: &dyn super::super::wire::Wire = &super::super::wire::EqMac;
        let mut world = hit_world(PostureState::Standing);
        let outcome = testing::run_on(EQMAC, |out| {
            Stances.handle(&ask(command::Posture::Standing), &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent.len(), 1);
    }

    /// The press lies the player down on Titanium too, before the feign
    /// request (read R19 a).
    #[test]
    fn titanium_lies_down_once_for_a_feign() {
        let mut posture = OwnPosture::default();
        let outcome = testing::run(|out| posture.lie_for_feign(7, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [eq_network_game::feign::titanium_posture(7).unwrap()]
        );
        let outcome = testing::run(|out| posture.lie_for_feign(7, out));
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty(), "{:?}", outcome.sent);
    }

    #[test]
    fn a_server_lying_report_holds_the_player_still_and_keeps_auto_attack() {
        let mut world = hit_world(PostureState::Standing);
        world.attacking = true;
        let outcome = testing::run(|out| {
            Stances.observe(
                &Message::Event(WorldEvent::Posture {
                    spawn_id: 7,
                    posture: PostureState::Lying,
                }),
                &mut world,
                out,
            )
        });
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty(), "{:?}", outcome.sent);
        assert_eq!(world.posture.current(), Some(PostureState::Lying));
        // Only the press turns auto attack off, never a lying report.
        assert!(world.attacking);
    }

    #[test]
    fn only_seated_crouched_or_prone_characters_stand_to_move() {
        let mut posture = OwnPosture::default();
        assert!(!posture.grounded(false));
        for (state, grounded) in [
            (PostureState::Sitting, true),
            (PostureState::Ducking, true),
            (PostureState::Lying, true),
            (PostureState::Standing, false),
            (PostureState::Looting, false),
            (PostureState::Frozen, false),
        ] {
            posture.observed(state);
            assert_eq!(posture.grounded(false), grounded, "{state:?}");
            // Titanium stands no lying player to move (read R19 c).
            let titanium = grounded && state != PostureState::Lying;
            assert_eq!(posture.grounded(true), titanium, "{state:?}");
        }
    }
}
