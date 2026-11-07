//! Drive the provider's real casting owner, inbound decoders and ordered zone stream.
use super::{super::feature::testing, *};
use crate::client::ServerProtocol;
use eq_network_game::{
    food::AutoEat,
    inventory::{ClickEffect, ClickKind, Inventory, InventorySlot, InventoryUpdate, ItemUse},
    spells::{SpellUpdate, EQMAC_BEGIN_OPCODE},
    GameDialect,
};

const REQUESTED: u16 = 2706;
const PORT_BEGIN: u16 = 2935;
const UNRELATED: u16 = 73;
const OWN: u16 = 7;

struct CastStream {
    server: &'static dyn servers::ServerType,
    features: Features,
    world: World,
}

impl CastStream {
    fn new(protocol: ServerProtocol, session_id: u64) -> Self {
        let server = servers::server_type(protocol);
        let setup = servers::Setup::new("Tester", AutoEat::default());
        let features = Features {
            provided: vec![server.casting(&setup).unwrap().feature],
            choices: Vec::new(),
            spell_landing_flag: None,
            answers_list_polls: false,
            gameplay_departures: false,
        };
        let mut world = World::new(session_id);
        world.admitted = Some(Instant::now());
        world.own_spawn = Some(OWN);
        let mut player = testing::player(OWN);
        player.memorized_spells[0] = Some(u32::from(REQUESTED));
        world.player.admit(player);
        Self {
            server,
            features,
            world,
        }
    }

    fn gem(&self) -> ClientCommand {
        ClientCommand::CastSpell {
            session_id: self.world.session_id,
            gem: 0,
            spell_id: u32::from(REQUESTED),
            target_id: OWN,
            created: Instant::now(),
        }
    }

    fn held(&self) -> bool {
        self.features
            .refusal(&self.gem(), &self.world, Instant::now())
            .is_some()
    }

    fn time_out(&mut self, command: ClientCommand, out: &mut Out<'_, '_>) -> Result<()> {
        apply_commands(&[command], &mut self.features, &mut self.world, out, true)?;
        assert!(self.held());
        self.features.tick(
            Instant::now() + Duration::from_secs(5),
            &mut self.world,
            out,
        )?;
        assert!(!self.held());
        Ok(())
    }

    fn begin(&mut self, caster: u16, spell: u16, out: &mut Out<'_, '_>) -> Result<()> {
        let mut body = [0; 8];
        body[..2].copy_from_slice(&caster.to_le_bytes());
        body[2..4].copy_from_slice(&spell.to_le_bytes());
        body[4..6].copy_from_slice(&3000u16.to_le_bytes());
        let opcode = if self.server.wire().dialect() == GameDialect::EqMac {
            EQMAC_BEGIN_OPCODE
        } else {
            0x3990
        };
        self.packet(opcode, &body, out)
    }

    fn mana(&mut self, spell: u16, out: &mut Out<'_, '_>) -> Result<()> {
        if self.server.wire().dialect() == GameDialect::EqMac {
            let mut body = [0; 4];
            body[2..].copy_from_slice(&spell.to_le_bytes());
            self.packet(0x7f41, &body, out)
        } else {
            let mut body = [0; 16];
            body[8..12].copy_from_slice(&u32::from(spell).to_le_bytes());
            self.packet(0x4839, &body, out)
        }
    }

    fn packet(&mut self, opcode: u16, body: &[u8], out: &mut Out<'_, '_>) -> Result<()> {
        let decoded = read_messages(self.server.wire(), opcode, body, self.server.item_rules());
        assert!(!decoded.is_empty());
        self.hear(decoded, out)
    }

    fn hear(&mut self, messages: Vec<Message>, out: &mut Out<'_, '_>) -> Result<()> {
        assert!(hear(messages, &mut self.features, &mut self.world, out, true)?.is_none());
        Ok(())
    }
}

#[test]
fn only_takp_associates_the_stock_luclin_port_alias() {
    for protocol in [
        ServerProtocol::EqEmu,
        ServerProtocol::Project1999,
        ServerProtocol::Quarm,
        ServerProtocol::Takp,
    ] {
        let mut stream = CastStream::new(protocol, 5);
        let outcome = testing::run_on(stream.server.wire(), |out| -> Result<()> {
            stream.time_out(stream.gem(), out)?;
            stream.begin(OWN, PORT_BEGIN, out)?;
            assert!(stream.held());
            stream.mana(REQUESTED, out)?;
            assert_eq!(stream.held(), protocol != ServerProtocol::Takp);
            if stream.held() {
                stream.mana(PORT_BEGIN, out)?;
                assert!(!stream.held());
            }
            Ok(())
        });
        outcome.result.unwrap();
        let associations = outcome
            .events
            .iter()
            .filter(|event| {
                matches!(event,
                    ClientEvent::World(WorldEvent::CastLateBegin { session_id: 5, requested_spell })
                        if *requested_spell == u32::from(REQUESTED)
                )
            })
            .count();
        assert_eq!(
            associations,
            usize::from(protocol == ServerProtocol::Takp),
            "{protocol:?}"
        );
    }
}

#[test]
fn a_late_item_start_keeps_its_takp_alias_until_spell_bearing_mana() {
    let mut stream = CastStream::new(ServerProtocol::Takp, 5);
    let slot = InventorySlot(22);
    let mut item = testing::item(slot.0);
    item.activation.effect = Some(ClickEffect {
        spell_id: u32::from(REQUESTED),
        kind: ClickKind::Expendable,
        required_level: 0,
        effect_level: 1,
        cast_time_ms: 3000,
        recast_delay_seconds: 0,
        recast_type: 0,
    });
    item.charges = 1;
    let mut inventory = Inventory::default();
    inventory.apply(InventoryUpdate::Snapshot(vec![item]));
    stream.world.inventory = inventory.into();
    let item = ClientCommand::UseItem(ItemUse {
        request_id: 1,
        session_id: 5,
        revision: stream.world.inventory.revision(),
        slot,
        target_id: OWN,
        created: Instant::now(),
    });
    let outcome = testing::run_on(stream.server.wire(), |out| -> Result<()> {
        stream.time_out(item, out)?;
        stream.begin(8, PORT_BEGIN, out)?;
        assert!(!stream.held());
        stream.begin(OWN, UNRELATED, out)?;
        assert!(stream.held());
        stream.mana(UNRELATED, out)?;
        assert!(!stream.held());
        stream.begin(OWN, PORT_BEGIN, out)?;
        stream.begin(OWN, PORT_BEGIN, out)?;
        stream.mana(UNRELATED, out)?;
        assert!(stream.held());
        stream.mana(REQUESTED, out)?;
        assert!(!stream.held());
        Ok(())
    });
    outcome.result.unwrap();
    assert_eq!(outcome.sent.len(), 1);
    assert_eq!(
        outcome.sent[0].opcode,
        eq_network_game::spells::EQMAC_CAST_OPCODE
    );
    let late = outcome
        .events
        .iter()
        .position(|event| {
            matches!(event,
                ClientEvent::World(WorldEvent::CastLateBegin { session_id: 5, requested_spell })
                    if *requested_spell == u32::from(REQUESTED)
            )
        })
        .unwrap();
    assert!(matches!(
        outcome.events[late + 1],
        ClientEvent::World(WorldEvent::Spell(SpellUpdate::Began {
            caster_id: OWN,
            spell_id: PORT_BEGIN,
            duration_ms: 3000,
        }))
    ));
    assert_eq!(
        outcome
            .events
            .iter()
            .filter(|event| matches!(event, ClientEvent::World(WorldEvent::CastLateBegin { .. })))
            .count(),
        1
    );
    assert!(matches!(outcome.events.as_slice(), [..,
        ClientEvent::World(WorldEvent::Spell(SpellUpdate::Mana { spell_id, keep_casting: false })),
        ClientEvent::World(WorldEvent::Mana(0)),
    ] if *spell_id == u32::from(REQUESTED)));
}

#[test]
fn an_old_result_or_lifecycle_change_retires_the_timed_out_context() {
    enum End {
        Mana,
        Death,
        Admission,
    }

    for end in [End::Mana, End::Death, End::Admission] {
        let mut stream = CastStream::new(ServerProtocol::Takp, 5);
        let outcome = testing::run_on(stream.server.wire(), |out| -> Result<()> {
            stream.time_out(stream.gem(), out)?;
            stream.begin(OWN, UNRELATED, out)?;
            match end {
                End::Mana => {
                    stream.mana(REQUESTED, out)?;
                    assert!(stream.held());
                    stream.mana(UNRELATED, out)?;
                }
                End::Death => {
                    stream.hear(
                        vec![Message::Event(WorldEvent::DeathBind { session_id: 5 })],
                        out,
                    )?;
                }
                End::Admission => stream = CastStream::new(ServerProtocol::Takp, 6),
            }
            assert!(!stream.held());
            stream.begin(OWN, PORT_BEGIN, out)?;
            stream.mana(REQUESTED, out)?;
            assert!(stream.held());
            stream.mana(PORT_BEGIN, out)?;
            assert!(!stream.held());
            Ok(())
        });
        outcome.result.unwrap();
        assert!(!outcome
            .events
            .iter()
            .any(|event| matches!(event, ClientEvent::World(WorldEvent::CastLateBegin { .. }))));
    }
}
