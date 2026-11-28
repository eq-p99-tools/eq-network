//! Players' corpses: `/consent` and `/deny`, `/corpse`, `/corpsedrag` and
//! `/corpsedrop`. The host picks a corpse by its spawn; the session names it
//! as the server knows it (`Name's corpse12`) and refuses what the server
//! would drop without a word. The server says the rest in its own string
//! table messages, and a dragged corpse moves as any spawn does.
//!
//! TAKP and Quarm (6.591, from their own handlers at TAKP 44afcbf and
//! Quarm fc75fcf) take the same requests on their own opcodes, with these
//! differences:
//! - A consent toggles: one given again takes it back, and neither server
//!   has a deny. So `/deny` is refused (row 6.591a for the program's).
//! - Only Quarm takes `/corpsedrop`, on an opcode of its own.
//! - `/corpse` goes at most every 250 ms; the session withholds one sooner
//!   without a word (ours, row 6.591c).
//! - The local refusals cite no string numbers there, which are
//!   Titanium's.
use super::{
    actions,
    feature::{Feature, Out, World},
    ClientCommand, ClientEvent,
};
use anyhow::Result;
use eq_network_game::{
    request::Request,
    world::{SpawnKind, WorldEvent},
};
use std::time::{Duration, Instant};

/// Consents, summons and drags the player's and others' corpses.
#[derive(Default)]
pub(super) struct Corpses {
    /// TAKP's and Quarm's rules, where they apply.
    eqmac: Option<EqMacCorpses>,
    auto_consent: Option<[Option<bool>; 3]>,
}

/// What TAKP and Quarm take: whether the server takes `/corpsedrop`, and
/// when the last `/corpse` went.
struct EqMacCorpses {
    drop: bool,
    last_summon: Option<Instant>,
}

/// The least time between two `/corpse` on TAKP and Quarm (ours, 6.591c).
const EQMAC_SUMMON_FLOOR: Duration = Duration::from_millis(250);

impl Corpses {
    /// Stock `EQEmu` implements and persists the Titanium auto-consent flags.
    pub(super) const fn titanium_eqemu() -> Self {
        Self {
            eqmac: None,
            auto_consent: Some([None; 3]),
        }
    }

    /// TAKP's or Quarm's corpses; `drop` where the server takes
    /// `/corpsedrop` (Quarm).
    pub(super) const fn eqmac(drop: bool) -> Self {
        Self {
            eqmac: Some(EqMacCorpses {
                drop,
                last_summon: None,
            }),
            auto_consent: None,
        }
    }
}

/// The official client's string (`eqstr_us.txt`) refusing a consent to no
/// name.
const INVALID_NAME: u32 = 397;
/// Its string refusing a consent to the player themselves.
const YOURSELF: u32 = 399;

/// Why a command is not sent: this library's words, and the official
/// client's string where it refuses in its own.
struct Refused(String, Option<u32>);

impl From<&str> for Refused {
    fn from(reason: &str) -> Self {
        Self(reason.into(), None)
    }
}

/// The request a command makes, or why it is not sent; `official` cites the
/// official client's strings, which are Titanium's.
fn request(command: &ClientCommand, world: &World, official: bool) -> Result<Request, Refused> {
    let cite = |string_id| official.then_some(string_id);
    let player = world.player.as_ref().ok_or("Not in the zone yet")?;
    // Titanium sends the selected corpse regardless of which corpse kind the
    // record names. Permission to move it remains the server's decision.
    let corpse = |spawn_id: u16| -> Result<String, Refused> {
        let spawn = world
            .spawns
            .all()
            .find(|spawn| spawn.spawn_id == spawn_id)
            .ok_or("That corpse is not here")?;
        if spawn.kind == SpawnKind::PlayerCorpse || (official && spawn.kind == SpawnKind::NpcCorpse)
        {
            Ok(spawn.name.clone())
        } else {
            Err("That is not a player's corpse".into())
        }
    };
    Ok(match command {
        // Names the official client will not send, which it refuses in its
        // own words (eqstr 397 and 399).
        ClientCommand::Consent { name, .. } if name.trim().is_empty() => {
            return Err(Refused(
                "That is not a name you can consent".into(),
                cite(INVALID_NAME),
            ));
        }
        ClientCommand::Consent { name, .. } if name.eq_ignore_ascii_case(&player.name) => {
            return Err(Refused(
                "You do not need to consent yourself".into(),
                cite(YOURSELF),
            ));
        }
        ClientCommand::Consent { name, given, .. } => Request::Consent {
            name: name.trim().into(),
            given: *given,
        },
        ClientCommand::SummonCorpse { spawn_id, .. } => Request::SummonCorpse(corpse(*spawn_id)?),
        ClientCommand::DragCorpse { spawn_id, .. } => Request::DragCorpse(corpse(*spawn_id)?),
        ClientCommand::DropCorpse { spawn_id, .. } => Request::DropCorpse {
            corpse: spawn_id.map(corpse).transpose()?,
        },
        _ => return Err("Not a corpse command".into()),
    })
}

impl Feature for Corpses {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        let mut caps = vec![crate::world::Capability::Corpses];
        if self.auto_consent.is_some() {
            caps.push(crate::world::Capability::CorpseAutoConsent);
        }
        caps
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        (self.auto_consent.is_some()
            && matches!(command, ClientCommand::SetCorpseAutoConsent { .. }))
            || matches!(
                command,
                ClientCommand::Consent { .. }
                    | ClientCommand::SummonCorpse { .. }
                    | ClientCommand::DragCorpse { .. }
                    | ClientCommand::DropCorpse { .. }
            )
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        if let ClientCommand::SetCorpseAutoConsent {
            session_id,
            audience,
            enabled,
        } = *command
        {
            let sent = self
                .auto_consent
                .as_mut()
                .filter(|_| session_id == world.session_id && world.player.as_ref().is_some());
            let Some(sent) = sent else {
                return actions::refuse(
                    command,
                    "Auto consent is unavailable on this zone visit",
                    out.log,
                );
            };
            if sent[audience.index()] == Some(enabled) {
                return Ok(());
            }
            out.request(&Request::CorpseAutoConsent { audience, enabled })?;
            sent[audience.index()] = Some(enabled);
            return out
                .log
                .send(ClientEvent::World(WorldEvent::CorpseAutoConsentSent {
                    session_id,
                    audience,
                    enabled,
                }));
        }
        if let Some(eqmac) = &mut self.eqmac {
            let refusal = match command {
                ClientCommand::Consent { given: false, .. } => Some(
                    "This server takes a consent back when you give it again; consent the name again.",
                ),
                ClientCommand::DropCorpse { .. } if !eqmac.drop => {
                    Some("This server has no corpse drop.")
                }
                _ => None,
            };
            if let Some(reason) = refusal {
                return actions::refuse(command, reason, out.log);
            }
            if matches!(command, ClientCommand::SummonCorpse { .. }) {
                if eqmac
                    .last_summon
                    .is_some_and(|last| last.elapsed() < EQMAC_SUMMON_FLOOR)
                {
                    return out.log.diagnostic(
                        "Corpse summon withheld: the last went under 250 ms ago".into(),
                    );
                }
                if send(command, world, false, out)? {
                    eqmac.last_summon = Some(Instant::now());
                }
                return Ok(());
            }
            return send(command, world, false, out).map(|_| ());
        }
        send(command, world, true, out).map(|_| ())
    }
}

/// Sends the command's request, or refuses it; true when sent.
fn send(
    command: &ClientCommand,
    world: &World,
    official: bool,
    out: &mut Out<'_, '_>,
) -> Result<bool> {
    // A name the packet cannot carry is refused as the feature's own
    // reasons are.
    let packet = request(command, world, official).and_then(|request| {
        out.encode(&request)
            .map_err(|error| Refused(error.to_string(), None))
    });
    match packet {
        Ok(packet) => out.send(&packet).map(|()| true),
        Err(Refused(reason, official)) => {
            actions::refuse_officially(command, (&reason, official), out.log).map(|()| false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;
    use eq_network_game::corpses;
    use eq_network_game::world::WorldEvent;

    #[test]
    fn corpse_auto_consent_sends_each_scope_and_never_acknowledges_it() {
        use corpses::ConsentAudience::{Group, Guild, Raid};
        let mut world = World::new(5);
        world.player.admit(testing::player(7));
        world.own_spawn = Some(7);
        let mut corpse = Corpses::titanium_eqemu();
        for (audience, kind) in [(Group, 40), (Raid, 41), (Guild, 42)] {
            for enabled in [true, false] {
                let command = ClientCommand::SetCorpseAutoConsent {
                    session_id: 5,
                    audience,
                    enabled,
                };
                let outcome = testing::run(|out| corpse.handle(&command, &mut world, out));
                outcome.result.unwrap();
                assert_eq!(outcome.sent[0].opcode, 0x7c32);
                assert_eq!(
                    outcome.sent[0].body,
                    [7, 0, kind, 0, u8::from(enabled), 0, 0, 0]
                );
                let repeated = testing::run(|out| corpse.handle(&command, &mut world, out));
                repeated.result.unwrap();
                assert_eq!(repeated.sent, []);
                assert!(matches!(&outcome.events[..],
                    [ClientEvent::World(WorldEvent::CorpseAutoConsentSent {
                        session_id: 5, audience: scope, enabled: on,
                    })] if *scope == audience && *on == enabled
                ));
            }
        }
        let stale = ClientCommand::SetCorpseAutoConsent {
            session_id: 4,
            audience: Group,
            enabled: true,
        };
        let outcome = testing::run(|out| corpse.handle(&stale, &mut world, out));
        assert_eq!(outcome.sent, []);
        assert_eq!(
            refusals(&outcome.events),
            ["Auto consent is unavailable on this zone visit"]
        );
        assert!(!Corpses::eqmac(false).owns(&stale));
    }

    fn refusals(events: &[super::super::ClientEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                super::super::ClientEvent::World(WorldEvent::CorpseRefused { reason, .. }) => {
                    Some(reason.clone())
                }
                _ => None,
            })
            .collect()
    }

    fn official(events: &[super::super::ClientEvent]) -> Vec<Option<u32>> {
        events
            .iter()
            .filter_map(|event| match event {
                super::super::ClientEvent::World(WorldEvent::CorpseRefused {
                    string_id, ..
                }) => Some(*string_id),
                _ => None,
            })
            .collect()
    }

    /// Player 7, "Tester", with their corpse (4) and an NPC's (5) nearby.
    fn world() -> World {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        player.name = "Tester".into();
        world.player.admit(player);
        let mut corpse = testing::spawn(4, SpawnKind::PlayerCorpse);
        corpse.name = "Tester's corpse4".into();
        world.spawns.insert(corpse);
        world.spawns.insert(testing::spawn(5, SpawnKind::NpcCorpse));
        world
    }

    #[test]
    fn consents_name_another_player() {
        let mut world = world();
        let consent = |name: &str, given| ClientCommand::Consent {
            session_id: 5,
            name: name.into(),
            given,
        };
        let outcome = testing::run(|out| {
            Corpses::default().handle(&consent("Helper", true), &mut world, out)
        });
        outcome.result.unwrap();
        assert_eq!(outcome.sent, [corpses::consent("Helper", true).unwrap()]);
        let outcome = testing::run(|out| {
            Corpses::default().handle(&consent("Helper", false), &mut world, out)
        });
        assert_eq!(outcome.sent, [corpses::consent("Helper", false).unwrap()]);
        for (name, reason, string_id) in [
            ("  ", "That is not a name you can consent", INVALID_NAME),
            ("tester", "You do not need to consent yourself", YOURSELF),
        ] {
            let outcome = testing::run(|out| {
                Corpses::default().handle(&consent(name, true), &mut world, out)
            });
            outcome.result.unwrap();
            assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
            assert_eq!(refusals(&outcome.events), [reason]);
            assert_eq!(official(&outcome.events), [Some(string_id)]);
        }
    }

    #[test]
    fn eqmac_corpses_use_the_mac_packets_and_refuse_what_the_server_lacks() {
        let wire = &super::super::wire::EqMac;
        let mut world = world();
        let run = |feature: &mut Corpses, command: &ClientCommand, world: &mut World| {
            let outcome = testing::run_on(wire, |out| feature.handle(command, world, out));
            assert!(outcome.result.is_ok());
            outcome
        };
        let consent = |name: &str, given| ClientCommand::Consent {
            session_id: 5,
            name: name.into(),
            given,
        };
        let mut takp = Corpses::eqmac(false);
        assert_eq!(
            run(&mut takp, &consent("Helper", true), &mut world).sent,
            [corpses::eqmac_consent("Helper").unwrap()]
        );
        // A consent toggles there, so a deny would grant one: refused.
        let denied = run(&mut takp, &consent("Helper", false), &mut world);
        assert_eq!(denied.sent, []);
        assert_eq!(refusals(&denied.events).len(), 1);
        // Local refusals cite no Titanium string numbers there.
        let own = run(&mut takp, &consent("tester", true), &mut world);
        assert_eq!(own.sent, []);
        assert_eq!(official(&own.events), [None]);
        let drag = ClientCommand::DragCorpse {
            session_id: 5,
            spawn_id: 4,
        };
        assert_eq!(
            run(&mut takp, &drag, &mut world).sent,
            [corpses::eqmac_drag("Tester's corpse4", "Tester").unwrap()]
        );
        let drop = |spawn_id| ClientCommand::DropCorpse {
            session_id: 5,
            spawn_id,
        };
        assert_eq!(run(&mut takp, &drop(None), &mut world).sent, []);
        let mut quarm = Corpses::eqmac(true);
        assert_eq!(
            run(&mut quarm, &drop(Some(4)), &mut world).sent,
            [corpses::quarm_drop(Some("Tester's corpse4"), "Tester").unwrap()]
        );
        assert_eq!(
            run(&mut quarm, &drop(None), &mut world).sent,
            [corpses::quarm_drop(None, "Tester").unwrap()]
        );
    }

    #[test]
    fn an_eqmac_corpse_summon_names_only_a_player_corpse_at_most_every_250_ms() {
        let wire = &super::super::wire::EqMac;
        let mut world = world();
        let mut feature = Corpses::eqmac(false);
        let summon = |spawn_id| ClientCommand::SummonCorpse {
            session_id: 5,
            spawn_id,
        };
        let mut run = |command: &ClientCommand, world: &mut World| {
            let outcome = testing::run_on(wire, |out| feature.handle(command, world, out));
            assert!(outcome.result.is_ok());
            outcome
        };
        // Never an NPC's corpse or a spawn not here, which would name
        // something other than a player's corpse; those refusals start no
        // wait.
        for spawn_id in [5, 6] {
            let refused = run(&summon(spawn_id), &mut world);
            assert_eq!(refused.sent, []);
            assert_eq!(refusals(&refused.events).len(), 1);
        }
        assert_eq!(
            run(&summon(4), &mut world).sent,
            [corpses::eqmac_summon("Tester's corpse4", "Tester").unwrap()]
        );
        // Sooner than 250 ms: withheld without a refusal.
        let held = run(&summon(4), &mut world);
        assert_eq!(held.sent, []);
        assert_eq!(refusals(&held.events), Vec::<String>::new());
        drop(run);
        let last = feature.eqmac.as_mut().unwrap();
        last.last_summon = Instant::now().checked_sub(EQMAC_SUMMON_FLOOR);
        let outcome = testing::run_on(wire, |out| feature.handle(&summon(4), &mut world, out));
        assert_eq!(outcome.sent.len(), 1);
    }

    #[test]
    fn summons_and_drags_name_a_player_corpse() {
        let mut world = world();
        world.spawns.insert(testing::spawn(8, SpawnKind::Npc));
        let summon = ClientCommand::SummonCorpse {
            session_id: 5,
            spawn_id: 4,
        };
        let outcome = testing::run(|out| Corpses::default().handle(&summon, &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [corpses::summon("Tester's corpse4", "Tester").unwrap()]
        );
        let drag = |spawn_id| ClientCommand::DragCorpse {
            session_id: 5,
            spawn_id,
        };
        let outcome = testing::run(|out| Corpses::default().handle(&drag(4), &mut world, out));
        assert_eq!(
            outcome.sent,
            [corpses::drag("Tester's corpse4", "Tester").unwrap()]
        );
        for (spawn_id, reason) in [
            (8, "That is not a player's corpse"),
            (6, "That corpse is not here"),
        ] {
            let outcome =
                testing::run(|out| Corpses::default().handle(&drag(spawn_id), &mut world, out));
            outcome.result.unwrap();
            assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
            assert_eq!(refusals(&outcome.events), [reason]);
        }
        let drop = |spawn_id| ClientCommand::DropCorpse {
            session_id: 5,
            spawn_id,
        };
        let outcome =
            testing::run(|out| Corpses::default().handle(&drop(Some(4)), &mut world, out));
        assert_eq!(
            outcome.sent,
            [corpses::release(Some("Tester's corpse4")).unwrap()]
        );
        let outcome = testing::run(|out| Corpses::default().handle(&drop(None), &mut world, out));
        assert_eq!(outcome.sent, [corpses::release(None).unwrap()]);
    }

    #[test]
    fn titanium_corpse_requests_do_not_require_the_player_corpse_kind() {
        let mut world = world();
        let mut corpse = testing::spawn(4, SpawnKind::NpcCorpse);
        corpse.name = "Tester's corpse4".into();
        world.spawns.insert(corpse);
        for command in [
            ClientCommand::SummonCorpse {
                session_id: 5,
                spawn_id: 4,
            },
            ClientCommand::DragCorpse {
                session_id: 5,
                spawn_id: 4,
            },
        ] {
            let outcome = testing::run(|out| Corpses::default().handle(&command, &mut world, out));
            outcome.result.unwrap();
            assert_eq!(refusals(&outcome.events), [] as [String; 0]);
            assert_eq!(outcome.sent.len(), 1);
            let expected = if matches!(command, ClientCommand::SummonCorpse { .. }) {
                corpses::summon("Tester's corpse4", "Tester").unwrap()
            } else {
                corpses::drag("Tester's corpse4", "Tester").unwrap()
            };
            assert_eq!(outcome.sent, [expected]);
        }
    }
}
