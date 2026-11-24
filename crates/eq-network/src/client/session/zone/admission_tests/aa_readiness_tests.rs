//! Synthetic receive/input ordering through the production Titanium zone loop.
use super::*;
use crate::client::ConnectionStage;
use eq_network_game::alternate_advancement::{self as aa, Action, Refusal, Update};

const FIRST: u32 = 900_100;
const LAST: u32 = 900_102;

fn aa_zone(exact_before_ready: bool, stop: Option<&CancellationToken>) -> Script {
    let mut script = Script::titanium(1.0);
    // A profile names the first rank, even when its third rank is maxed.
    let profile = &mut script.incoming.front_mut().unwrap().body;
    profile[304..308].copy_from_slice(&FIRST.to_le_bytes());
    profile[308..312].copy_from_slice(&3u32.to_le_bytes());
    let mut definition = vec![0; 96];
    for (at, value) in [
        (0, LAST),
        (24, 51),
        (36, 3),
        (52, 900_300),
        (68, 3),
        (72, LAST - 1),
        (76, u32::MAX),
    ] {
        definition[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    script.incoming.insert(
        1,
        Application {
            opcode: aa::DEFINITION_OPCODE,
            body: definition,
        },
    );
    let mut selections = vec![0; aa::SELECTION_SLOTS * 8];
    selections[..4].copy_from_slice(&LAST.to_le_bytes());
    selections[4..8].copy_from_slice(&6u32.to_le_bytes());
    let exact = Application {
        opcode: aa::SELECTIONS_OPCODE,
        body: selections,
    };
    if exact_before_ready {
        script.incoming.insert(2, exact);
    } else {
        script.incoming.push_back(exact);
    }
    script.stop_when_drained = stop.cloned();
    script
}

#[derive(Default)]
struct Witness {
    admissions: Vec<u64>,
    order: Vec<&'static str>,
    refusals: Vec<(Refusal, String)>,
    timers: usize,
}

impl Witness {
    fn hear(
        &mut self,
        event: &ClientEvent,
        sender: &mpsc::Sender<ClientCommand>,
        exact_before_ready: bool,
    ) -> Result<()> {
        let mut ask = false;
        match event {
            ClientEvent::World(WorldEvent::Entered { session_id, .. }) => {
                self.admissions.push(*session_id);
            }
            ClientEvent::Progress(ConnectionStage::Ready) if self.admissions.len() == 2 => {
                self.order.push("ready");
                ask = true;
            }
            ClientEvent::World(WorldEvent::AlternateAdvancement { updates, .. }) => {
                self.timers += updates
                    .iter()
                    .filter(|update| matches!(update, Update::Timer(_)))
                    .count();
                if self.admissions.len() == 2
                    && updates.iter().any(|update| {
                        matches!(
                            update,
                            Update::Selections(Some(selections))
                                if selections.iter().any(|selection| selection.id == LAST)
                        )
                    })
                {
                    self.order.push("exact ownership");
                    ask = !exact_before_ready;
                }
            }
            ClientEvent::World(WorldEvent::AlternateAdvancementRefused {
                session_id,
                refusal,
                reason,
            }) => {
                assert_eq!(Some(session_id), self.admissions.last());
                self.refusals.push((
                    refusal.expect("an ordinary AA ownership refusal"),
                    reason.clone(),
                ));
                self.order.push("refused");
            }
            _ => {}
        }
        if ask {
            sender.send(ClientCommand::ActivateAlternateAdvancement {
                session_id: *self.admissions.last().unwrap(),
                rank_id: LAST,
                target: None,
            })?;
        }
        Ok(())
    }
}

#[test]
fn a_fresh_titanium_zone_is_ready_before_max_rank_ownership_is_known() {
    for exact_before_ready in [false, true] {
        let mut config = config();
        config.local_only = true;
        let identity = ClientIdentity::new("example-host", "example-user");
        let credentials = credentials();
        let stop = CancellationToken::default();
        let (sender, commands) = mpsc::channel();
        let mut context = context(&config, &identity, &credentials, &stop, Some(&commands));
        let mut witness = Witness::default();
        let mut handler = |event| witness.hear(&event, &sender, exact_before_ready);
        let mut log = Events::new(&config, &mut handler);

        // Establish exact ownership in one admission, then really leave it.
        // No activation is requested in this old zone.
        let mut old = aa_zone(true, None);
        let address = old.transfer();
        let exit = run_script(&context, &mut old, &mut log).unwrap();
        assert!(matches!(&exit, ZoneExit::Direct(next) if *next == address));
        assert!(old.closed && old.aa_actions.is_empty());
        context.on_zone_exit(&exit);

        let mut fresh = aa_zone(exact_before_ready, Some(&stop));
        assert!(matches!(
            run_script(&context, &mut fresh, &mut log).unwrap(),
            ZoneExit::Stopped
        ));
        drop(log);
        assert!(fresh.closed && fresh.incoming.is_empty());
        assert_eq!(fresh.aa_actions_at_selections, [0]);
        assert_eq!(witness.admissions.len(), 2);
        assert_ne!(witness.admissions[0], witness.admissions[1]);
        if exact_before_ready {
            assert_eq!(witness.order, ["exact ownership", "ready"]);
            assert_eq!(witness.refusals, Vec::<(Refusal, String)>::new());
        } else {
            assert_eq!(witness.order, ["ready", "refused", "exact ownership"]);
            assert_eq!(
                witness.refusals,
                [(
                    Refusal::NotActivatable,
                    "You don't have that ability to use.".into()
                )]
            );
        }
        assert_eq!(
            fresh.aa_actions,
            [aa::titanium_action(Action::Activate {
                rank_id: LAST,
                target: None,
            })
            .unwrap()
            .to_vec()]
        );
        assert!(commands.try_recv().is_err());
        // One permitted outgoing request is not a completed activation or
        // server reuse timer. Neither is manufactured by Ready or a send.
        assert_eq!(witness.timers, 0);
    }
}
