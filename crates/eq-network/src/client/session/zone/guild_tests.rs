//! Received membership changes through the assembled zone features and sender.
use super::super::feature::testing;
use super::*;
use eq_network_game::{
    command::EncodedCommand,
    guild,
    listing::{GuildRank, Listing, ListingChange},
    world::SpawnKind,
};

struct Visit {
    features: Features,
    world: World,
}

impl Visit {
    fn new() -> Self {
        let mut world = World::new(5);
        world.own_spawn = Some(7);
        let mut player = testing::player(7);
        player.name = "Tester".into();
        player.listing = Listing::default();
        world.player.admit(player);
        world.admitted = Some(Instant::now());
        for spawn_id in [7, 8] {
            world
                .spawns
                .insert(testing::spawn(spawn_id, SpawnKind::Player));
        }
        let server = servers::server_type(crate::client::ServerProtocol::EqEmu);
        let features = Features::new(server, "Tester", eq_network_game::food::AutoEat::default());
        Self { features, world }
    }

    fn listing(&mut self, spawn_id: u16, change: ListingChange) {
        let received = testing::run(|out| {
            hear(
                vec![Message::Event(WorldEvent::Listing { spawn_id, change })],
                &mut self.features,
                &mut self.world,
                out,
                true,
            )
        });
        assert!(received.result.unwrap().is_none());
        assert_eq!(received.sent, Vec::<EncodedCommand>::new());
    }

    fn own_listing(&self) -> Listing {
        self.world.player.as_ref().unwrap().listing
    }

    fn invite(&mut self) -> testing::Outcome<Result<Option<ZoneExit>>> {
        testing::run(|out| {
            apply_commands(
                &[ClientCommand::GuildInvite {
                    session_id: 5,
                    name: "Friend".into(),
                    officer: false,
                }],
                &mut self.features,
                &mut self.world,
                out,
                true,
            )
        })
    }

    fn assert_guildless_invite(&mut self) {
        let refused = self.invite();
        assert!(refused.result.unwrap().is_none());
        assert_eq!(refused.sent, Vec::<EncodedCommand>::new());
        assert!(refused.events.iter().any(|event| matches!(
            event,
            ClientEvent::World(WorldEvent::GuildRefused { reason, .. })
                if reason == "You are not in a guild."
        )));
    }
}

#[test]
fn received_own_guild_join_reaches_normal_invite_serialization() {
    let mut visit = Visit::new();
    visit.assert_guildless_invite();
    visit.listing(7, ListingChange::Guild(Some(3)));
    visit.listing(7, ListingChange::GuildRank(GuildRank::Leader));
    assert_eq!(visit.own_listing().guild, Some(3));
    assert_eq!(visit.own_listing().guild_rank, Some(GuildRank::Leader));

    let invited = visit.invite();
    assert!(invited.result.unwrap().is_none());
    assert_eq!(
        invited.sent,
        [guild::titanium_invite("Friend", "Tester", 3, GuildRank::Member).unwrap()]
    );
    assert_eq!(invited.unreliable, Vec::<EncodedCommand>::new());
    assert!(invited
        .events
        .iter()
        .all(|event| !matches!(event, ClientEvent::World(WorldEvent::GuildRefused { .. }))));
}

#[test]
fn received_own_guild_leave_withdraws_invites_and_clears_rank() {
    let mut visit = Visit::new();
    visit.listing(7, ListingChange::Guild(Some(3)));
    visit.listing(7, ListingChange::GuildRank(GuildRank::Leader));
    visit.listing(7, ListingChange::Guild(None));
    visit.listing(7, ListingChange::GuildRank(GuildRank::Officer));
    assert_eq!(visit.own_listing().guild, None);
    assert_eq!(visit.own_listing().guild_rank, None);
    visit.assert_guildless_invite();
}

#[test]
fn foreign_guild_changes_cannot_grant_or_withdraw_own_membership() {
    let mut visit = Visit::new();
    visit.listing(8, ListingChange::Guild(Some(3)));
    visit.listing(8, ListingChange::GuildRank(GuildRank::Leader));
    assert_eq!(visit.world.spawns.get(8).unwrap().listing.guild, Some(3));
    assert_eq!(visit.own_listing().guild, None);
    assert_eq!(visit.own_listing().guild_rank, None);
    visit.assert_guildless_invite();

    visit.listing(7, ListingChange::Guild(Some(4)));
    visit.listing(7, ListingChange::GuildRank(GuildRank::Officer));
    visit.listing(8, ListingChange::Guild(None));
    assert_eq!(visit.own_listing().guild, Some(4));
    assert_eq!(visit.own_listing().guild_rank, Some(GuildRank::Officer));
    let invited = visit.invite();
    assert!(invited.result.unwrap().is_none());
    assert_eq!(
        invited.sent,
        [guild::titanium_invite("Friend", "Tester", 4, GuildRank::Member).unwrap()]
    );
}

#[test]
fn received_guild_changes_preserve_unknown_and_reset_prior_rank() {
    let mut visit = Visit::new();
    visit.listing(7, ListingChange::GuildRank(GuildRank::Leader));
    assert_eq!(visit.own_listing().guild_rank, None);
    visit.listing(7, ListingChange::Guild(Some(3)));
    assert_eq!(visit.own_listing().guild_rank, None);
    visit.listing(7, ListingChange::GuildRank(GuildRank::Leader));
    visit.listing(7, ListingChange::Guild(Some(4)));
    assert_eq!(visit.own_listing().guild, Some(4));
    assert_eq!(visit.own_listing().guild_rank, None);
    visit.listing(7, ListingChange::GuildRank(GuildRank::Other(42)));
    assert_eq!(visit.own_listing().guild_rank, Some(GuildRank::Other(42)));
}
