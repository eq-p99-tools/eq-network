//! Who is online: `/who all` asks the world, whose answer reaches the host
//! as news (`WorldEvent::WhoList`). A plain `/who` lists the zone's players
//! from what the host already knows, as the Titanium client does.
use super::{
    feature::{Feature, Out, World},
    ClientCommand,
};
use anyhow::Result;
use eq_network_game::who;

/// Asks the world who is online.
pub(super) struct Who;

impl Feature for Who {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::Who]
    }

    fn owns(&self, command: &ClientCommand) -> bool {
        matches!(command, ClientCommand::WhoAll { .. })
    }

    fn handle(
        &mut self,
        command: &ClientCommand,
        _world: &mut World,
        out: &mut Out<'_, '_>,
    ) -> Result<()> {
        let ClientCommand::WhoAll { filter, .. } = command else {
            return Ok(());
        };
        match who::request(filter) {
            Ok(request) => out.send(&request),
            // The host checks the text first; a request it let through
            // anyway is not worth the session.
            Err(error) => out.log.diagnostic(format!("Who request not sent: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::feature::testing;
    use super::*;

    #[test]
    fn who_all_asks_the_world_with_the_filter() {
        let mut world = World::new(5);
        let ask = |text: &str| ClientCommand::WhoAll {
            session_id: 5,
            filter: who::WhoFilter {
                text: text.into(),
                ..who::WhoFilter::default()
            },
        };
        let outcome = testing::run(|out| Who.handle(&ask("qeynos"), &mut world, out));
        outcome.result.unwrap();
        assert_eq!(
            outcome.sent,
            [who::request(&who::WhoFilter {
                text: "qeynos".into(),
                ..who::WhoFilter::default()
            })
            .unwrap()]
        );
        let outcome = testing::run(|out| Who.handle(&ask(&"x".repeat(64)), &mut world, out));
        outcome.result.unwrap();
        assert!(outcome.sent.is_empty(), "sent {:?}", outcome.sent);
    }
}
