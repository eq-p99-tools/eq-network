//! One table of what in-flight actions hold and what new commands need, and
//! one check that a command is still fresh.
//!
//! Controllers still decide when their own action starts and ends (a cast begins
//! and is interrupted, a book edit is answered, a scribe consumes its scroll). This
//! module only turns their current state into held resources and refuses any
//! command that needs one of them, or that was made for an earlier admission or
//! too long ago, so these are decided in one place instead of by per-action guards.
use super::{casting, ClientCommand, ClientEvent, Events};
use anyhow::Result;
use eq_network_game::{
    spells::BookActionStatus,
    world::{CampStatus, WorldEvent},
};
use std::time::{Duration, Instant};

/// How long a command stays fresh after the host made it.
fn freshness(command: &ClientCommand) -> Duration {
    match command {
        // A zone line is crossed where the player stands now, and a movement
        // calibration belongs to the movement that follows it.
        ClientCommand::CrossZoneLine { .. } | ClientCommand::ConfigureMotion { .. } => {
            Duration::from_millis(250)
        }
        _ => Duration::from_secs(1),
    }
}

/// Why a command can no longer be carried out: it names an earlier admission
/// or is no longer fresh. Movement is the movement guard's to judge, since a
/// refused move must also undo the host's prediction.
pub(super) fn stale(
    command: &ClientCommand,
    session_id: u64,
    now: Instant,
) -> Option<&'static str> {
    if matches!(command, ClientCommand::Move(_)) {
        return None;
    }
    if command
        .session_id()
        .is_some_and(|requested| requested != session_id)
    {
        return Some("That was meant for an earlier zone visit");
    }
    let created = command.created()?;
    (created > now || now.duration_since(created) >= freshness(command))
        .then_some("The request expired")
}

/// Game state an in-flight action may hold exclusively.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Resource {
    /// Casting a spell or item click effect.
    Casting,
    /// The spellbook and memorized gems.
    Spellbook,
    /// Inventory contents, including the cursor.
    Inventory,
}

/// Resources held right now, each with the reason shown to a refused command.
pub(super) struct Held(Vec<(Resource, &'static str)>);

impl Held {
    /// The holds of every action in flight, the features' first.
    pub(super) fn new(holds: Vec<(Resource, &'static str)>) -> Self {
        Self(holds)
    }

    /// The hold of a cast in flight.
    #[cfg(test)]
    fn casting(cast_guard: &casting::CastGuard) -> Self {
        Self(cast_guard.hold().into_iter().collect())
    }

    /// The reason a command must wait, if it needs anything held.
    pub(super) fn conflict(&self, command: &ClientCommand) -> Option<&'static str> {
        let needs = needs(command);
        self.0
            .iter()
            .find(|(resource, _)| needs.contains(resource))
            .map(|(_, reason)| *reason)
    }
}

/// What a command needs exclusively. Every command is named, so a new one is
/// a decision here.
pub(super) fn needs(command: &ClientCommand) -> &'static [Resource] {
    use Resource::{Casting, Inventory, Spellbook};
    match command {
        ClientCommand::CastSpell { .. } => &[Casting],
        // `EQEmu` ignores a strike during a cast; bards may strike while they
        // sing, but the session cannot tell songs apart, so they wait too.
        ClientCommand::UseAbility { ability, .. } if ability.strikes() => &[Casting],
        ClientCommand::UseItem(_) => &[Casting, Inventory],
        ClientCommand::ScribeSpell { .. } => &[Casting, Spellbook, Inventory],
        ClientCommand::MemorizeSpell { .. }
        | ClientCommand::ForgetSpell { .. }
        | ClientCommand::DeleteSpell { .. }
        | ClientCommand::SwapSpell { .. } => &[Casting, Spellbook],
        // EQEmu kicks a move from outside the cursor range (30-39) during a cast
        // ("Inventory desync"); bard songs are exempt there, but the session
        // cannot tell songs apart, so singing bards wait too.
        ClientCommand::MoveInventory(request) if !(30..=39).contains(&request.from.0) => {
            &[Casting, Inventory]
        }
        // A picked-up item lands on the cursor; Give hands the trade slots over.
        ClientCommand::MoveInventory(_)
        | ClientCommand::PickUp { .. }
        | ClientCommand::Buy { .. }
        | ClientCommand::Sell { .. }
        | ClientCommand::Consume { .. }
        | ClientCommand::AcceptTrade { .. }
        // A combine takes the container's contents and puts what was made
        // on the cursor.
        | ClientCommand::Combine { .. } => &[Inventory],
        ClientCommand::SelectCharacter { .. }
        | ClientCommand::CreateCharacter { .. }
        | ClientCommand::ClickDoor { .. }
        | ClientCommand::CrossZoneLine { .. }
        | ClientCommand::SetPosture { .. }
        | ClientCommand::SendChat(_)
        | ClientCommand::InspectItem { .. }
        | ClientCommand::Consider { .. }
        | ClientCommand::Camp { .. }
        | ClientCommand::Loot { .. }
        | ClientCommand::LootItem { .. }
        | ClientCommand::EndLoot { .. }
        | ClientCommand::Shop { .. }
        // Asking holds nothing, and closing a window must always be possible;
        // coins are not items.
        | ClientCommand::OfferTrade { .. }
        | ClientCommand::CancelTrade { .. }
        | ClientCommand::MoveCoins { .. }
        | ClientCommand::Jump { .. }
        | ClientCommand::AutoAttack { .. }
        | ClientCommand::UseAbility { .. }
        | ClientCommand::SelectTarget { .. }
        | ClientCommand::WhoAll { .. }
        | ClientCommand::Pet { .. }
        | ClientCommand::Training { .. }
        | ClientCommand::AnswerResurrection { .. }
        | ClientCommand::InviteToGroup { .. }
        | ClientCommand::FollowGroup { .. }
        | ClientCommand::DeclineGroup { .. }
        | ClientCommand::Disband { .. }
        | ClientCommand::ToggleAway { .. }
        | ClientCommand::ToggleAnonymous { .. }
        | ClientCommand::ToggleRoleplay { .. }
        | ClientCommand::Random { .. }
        | ClientCommand::Emote { .. }
        | ClientCommand::Assist { .. }
        | ClientCommand::RaidInvite { .. }
        | ClientCommand::RaidAccept { .. }
        | ClientCommand::RaidDecline { .. }
        | ClientCommand::RaidLeave { .. }
        | ClientCommand::ReadItem { .. }
        // Opening a container holds nothing, and closing one must always be
        // possible.
        | ClientCommand::OpenContainer { .. }
        | ClientCommand::CloseContainer { .. }
        | ClientCommand::Consent { .. }
        | ClientCommand::SummonCorpse { .. }
        | ClientCommand::DragCorpse { .. }
        | ClientCommand::DropCorpse { .. }
        | ClientCommand::ConfigureMotion { .. }
        | ClientCommand::AutoEat { .. }
        | ClientCommand::Move(_) => &[],
    }
}

/// Reports a refused command through the result event its caller waits for;
/// a command nobody waits on is only noted.
pub(super) fn refuse(command: &ClientCommand, reason: &str, log: &mut Events<'_>) -> Result<()> {
    refuse_officially(command, (reason, None), log)
}

/// Reports a refused command as [`refuse`] does, naming the official
/// client's string (`eqstr_us.txt`) for it where its result event has room.
pub(super) fn refuse_officially(
    command: &ClientCommand,
    (reason, official): (&str, Option<u32>),
    log: &mut Events<'_>,
) -> Result<()> {
    if let Some(mut event) = refusal(command, reason) {
        if let WorldEvent::ConsumeRefused { string_id, .. }
        | WorldEvent::CorpseRefused { string_id, .. }
        | WorldEvent::PetRefused { string_id, .. }
        | WorldEvent::CombineRefused { string_id, .. }
        | WorldEvent::GroupRefused { string_id, .. }
        | WorldEvent::ListingRefused { string_id, .. }
        | WorldEvent::SocialRefused { string_id, .. }
        | WorldEvent::RaidRefused { string_id, .. }
        | WorldEvent::AbilityRefused { string_id, .. } = &mut event
        {
            *string_id = official;
        }
        log.send(ClientEvent::World(event))?;
    }
    log.diagnostic(reason.into())
}

/// The result event that tells the host a command was refused; None for a
/// command whose caller waits on no result. Every command is named, so a new
/// one is a decision here.
#[allow(
    clippy::too_many_lines,
    reason = "every kind of command named, so that a new one is a decision here"
)]
fn refusal(command: &ClientCommand, reason: &str) -> Option<WorldEvent> {
    let error = Some(reason.to_owned());
    Some(match command {
        ClientCommand::CastSpell { .. } => return casting::rejected(command, reason),
        ClientCommand::UseItem(request) => WorldEvent::ItemUseAction {
            session_id: request.session_id,
            request_id: request.request_id,
            error,
        },
        ClientCommand::MoveInventory(request) => WorldEvent::InventoryAction {
            session_id: request.session_id,
            revision: request.revision,
            error,
        },
        ClientCommand::Buy { session_id, .. }
        | ClientCommand::Sell { session_id, .. }
        | ClientCommand::OfferTrade { session_id, .. }
        | ClientCommand::AcceptTrade { session_id, .. }
        | ClientCommand::MoveCoins { session_id, .. }
        | ClientCommand::Consume { session_id, .. }
        | ClientCommand::UseAbility { session_id, .. }
        | ClientCommand::Pet { session_id, .. }
        | ClientCommand::Training { session_id, .. }
        | ClientCommand::AnswerResurrection { session_id, .. }
        | ClientCommand::InviteToGroup { session_id, .. }
        | ClientCommand::FollowGroup { session_id }
        | ClientCommand::DeclineGroup { session_id }
        | ClientCommand::Disband { session_id }
        | ClientCommand::ToggleAway { session_id }
        | ClientCommand::ToggleAnonymous { session_id }
        | ClientCommand::ToggleRoleplay { session_id }
        | ClientCommand::Random { session_id, .. }
        | ClientCommand::Emote { session_id, .. }
        | ClientCommand::Assist { session_id, .. }
        | ClientCommand::RaidInvite { session_id, .. }
        | ClientCommand::RaidAccept { session_id }
        | ClientCommand::RaidDecline { session_id }
        | ClientCommand::RaidLeave { session_id }
        | ClientCommand::ReadItem { session_id, .. }
        | ClientCommand::Combine { session_id, .. }
        | ClientCommand::Consent { session_id, .. }
        | ClientCommand::SummonCorpse { session_id, .. }
        | ClientCommand::DragCorpse { session_id, .. }
        | ClientCommand::DropCorpse { session_id, .. }
        | ClientCommand::CrossZoneLine { session_id, .. } => {
            return reasoned(command, *session_id, reason.into());
        }
        ClientCommand::PickUp {
            session_id,
            drop_id,
            ..
        }
        | ClientCommand::OpenContainer {
            session_id,
            drop_id,
            ..
        } => WorldEvent::ObjectAction {
            session_id: *session_id,
            drop_id: *drop_id,
            error,
        },
        ClientCommand::ClickDoor {
            session_id,
            door_id,
            ..
        } => WorldEvent::DoorAction {
            session_id: *session_id,
            door_id: *door_id,
            error,
        },
        ClientCommand::Camp { .. } => WorldEvent::Camp(CampStatus::Rejected(reason.into())),
        ClientCommand::SelectTarget {
            session_id,
            spawn_id,
        } => WorldEvent::TargetRejected {
            session_id: *session_id,
            spawn_id: *spawn_id,
            reason: reason.into(),
        },
        ClientCommand::ScribeSpell { .. }
        | ClientCommand::MemorizeSpell { .. }
        | ClientCommand::ForgetSpell { .. }
        | ClientCommand::DeleteSpell { .. }
        | ClientCommand::SwapSpell { .. } => {
            WorldEvent::BookAction(BookActionStatus::Rejected(reason.into()))
        }
        ClientCommand::SelectCharacter { .. }
        | ClientCommand::CreateCharacter { .. }
        | ClientCommand::SetPosture { .. }
        | ClientCommand::SendChat(_)
        | ClientCommand::InspectItem { .. }
        | ClientCommand::Consider { .. }
        | ClientCommand::Loot { .. }
        | ClientCommand::LootItem { .. }
        | ClientCommand::EndLoot { .. }
        | ClientCommand::Shop { .. }
        | ClientCommand::CancelTrade { .. }
        | ClientCommand::Jump { .. }
        | ClientCommand::AutoAttack { .. }
        | ClientCommand::WhoAll { .. }
        | ClientCommand::ConfigureMotion { .. }
        | ClientCommand::AutoEat { .. }
        | ClientCommand::CloseContainer { .. }
        | ClientCommand::Move(_) => return None,
    })
}

/// The refusal that is only a reason, for the admission that asked: what a
/// merchant, a trade, coins, food, an ability, a pet, a guildmaster, a
/// corpse or a zone line would not do.
fn reasoned(command: &ClientCommand, session_id: u64, reason: String) -> Option<WorldEvent> {
    Some(match command {
        ClientCommand::Buy { .. } | ClientCommand::Sell { .. } => {
            WorldEvent::MerchantRefused { session_id, reason }
        }
        ClientCommand::OfferTrade { .. } | ClientCommand::AcceptTrade { .. } => {
            WorldEvent::ExchangeRefused { session_id, reason }
        }
        ClientCommand::MoveCoins { .. } => WorldEvent::CoinsRefused { session_id, reason },
        ClientCommand::Consume { .. } => WorldEvent::ConsumeRefused {
            session_id,
            reason,
            string_id: None,
        },
        ClientCommand::UseAbility { .. } => WorldEvent::AbilityRefused {
            session_id,
            reason,
            string_id: None,
            arguments: Vec::new(),
        },
        ClientCommand::Pet { .. } => WorldEvent::PetRefused {
            session_id,
            reason,
            string_id: None,
        },
        ClientCommand::Training { .. } => WorldEvent::TrainingRefused { session_id, reason },
        ClientCommand::AnswerResurrection { .. } => {
            WorldEvent::ResurrectionRefused { session_id, reason }
        }
        ClientCommand::InviteToGroup { .. }
        | ClientCommand::FollowGroup { .. }
        | ClientCommand::DeclineGroup { .. }
        | ClientCommand::Disband { .. } => WorldEvent::GroupRefused {
            session_id,
            reason,
            string_id: None,
        },
        ClientCommand::ToggleAway { .. }
        | ClientCommand::ToggleAnonymous { .. }
        | ClientCommand::ToggleRoleplay { .. } => WorldEvent::ListingRefused {
            session_id,
            reason,
            string_id: None,
        },
        ClientCommand::Random { .. }
        | ClientCommand::Emote { .. }
        | ClientCommand::Assist { .. } => WorldEvent::SocialRefused {
            session_id,
            reason,
            string_id: None,
        },
        ClientCommand::RaidInvite { .. }
        | ClientCommand::RaidAccept { .. }
        | ClientCommand::RaidDecline { .. }
        | ClientCommand::RaidLeave { .. } => WorldEvent::RaidRefused {
            session_id,
            reason,
            string_id: None,
        },
        ClientCommand::ReadItem { .. } => WorldEvent::ReadRefused { session_id, reason },
        ClientCommand::Combine { .. } => WorldEvent::CombineRefused {
            session_id,
            reason,
            string_id: None,
        },
        ClientCommand::Consent { .. }
        | ClientCommand::SummonCorpse { .. }
        | ClientCommand::DragCorpse { .. }
        | ClientCommand::DropCorpse { .. } => WorldEvent::CorpseRefused {
            session_id,
            reason,
            string_id: None,
        },
        ClientCommand::CrossZoneLine { .. } => WorldEvent::ZoneLineRejected { session_id, reason },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use casting::CastGuard;

    #[test]
    fn commands_for_an_earlier_admission_or_made_too_long_ago_are_stale() {
        let now = Instant::now();
        let ago = |millis| now.checked_sub(Duration::from_millis(millis)).unwrap();
        let camp = |session_id, created| ClientCommand::Camp {
            session_id,
            created,
        };
        assert_eq!(stale(&camp(1, now), 1, now), None);
        assert_eq!(stale(&camp(1, ago(999)), 1, now), None);
        assert_eq!(
            stale(&camp(1, ago(1000)), 1, now),
            Some("The request expired")
        );
        assert_eq!(
            stale(&camp(1, now + Duration::from_millis(1)), 1, now),
            Some("The request expired")
        );
        assert_eq!(
            stale(&camp(2, now), 1, now),
            Some("That was meant for an earlier zone visit")
        );
        let cross = |created| ClientCommand::CrossZoneLine {
            session_id: 1,
            destination: eq_network_game::zoning::ZoneLineDestination::Reference(1),
            position: eq_network_game::world::Position::default(),
            created,
        };
        assert_eq!(stale(&cross(ago(249)), 1, now), None);
        assert!(stale(&cross(ago(250)), 1, now).is_some());
        // A command that may wait only has to name this admission.
        let close = |session_id| ClientCommand::EndLoot {
            session_id,
            corpse_id: 7,
        };
        assert_eq!(stale(&close(1), 1, now), None);
        assert!(stale(&close(2), 1, now).is_some());
    }

    #[test]
    fn refusals_answer_through_the_event_each_command_waits_for() {
        let now = Instant::now();
        let door = ClientCommand::ClickDoor {
            session_id: 1,
            door_id: 4,
            created: now,
        };
        assert!(matches!(
            refusal(&door, "No"),
            Some(WorldEvent::DoorAction { door_id: 4, error: Some(error), .. }) if error == "No"
        ));
        assert!(matches!(
            refusal(&memorize(), "No"),
            Some(WorldEvent::BookAction(BookActionStatus::Rejected(_)))
        ));
        let jump = ClientCommand::Jump {
            session_id: 1,
            created: now,
        };
        assert!(refusal(&jump, "No").is_none());
    }

    fn cast() -> ClientCommand {
        ClientCommand::CastSpell {
            session_id: 1,
            gem: 0,
            spell_id: 42,
            target_id: 7,
            created: Instant::now(),
        }
    }

    fn memorize() -> ClientCommand {
        ClientCommand::MemorizeSpell {
            session_id: 1,
            gem: 0,
            spell_id: 42,
            created: Instant::now(),
        }
    }

    #[test]
    fn holds_refuse_only_commands_that_need_them() {
        let idle = Held::casting(&CastGuard::default());
        assert_eq!(idle.conflict(&cast()), None);
        assert_eq!(idle.conflict(&memorize()), None);

        let mut guard = CastGuard::default();
        guard.submitted(42, Instant::now());
        let casting = Held::casting(&guard);
        assert!(casting.conflict(&cast()).is_some());
        assert!(casting.conflict(&memorize()).is_some());
        assert_eq!(
            casting.conflict(&ClientCommand::SelectTarget {
                session_id: 1,
                spawn_id: Some(7)
            }),
            None
        );

        // The first hold a command needs gives the reason.
        let held = Held::new(vec![
            (Resource::Spellbook, "Wait for the current spellbook change"),
            (Resource::Casting, "Wait for the current cast"),
        ]);
        assert_eq!(held.conflict(&cast()), Some("Wait for the current cast"));
        assert_eq!(
            held.conflict(&memorize()),
            Some("Wait for the current spellbook change")
        );
    }

    fn move_from(slot: i32) -> ClientCommand {
        ClientCommand::MoveInventory(eq_network_game::inventory::InventoryMove {
            session_id: 1,
            revision: 1,
            from: eq_network_game::inventory::InventorySlot(slot),
            to: eq_network_game::inventory::InventorySlot(30),
            quantity: eq_network_game::inventory::MoveQuantity::Whole,
            created: Instant::now(),
        })
    }

    #[test]
    fn items_move_during_a_cast_only_from_the_cursor() {
        let mut guard = CastGuard::default();
        guard.submitted(42, Instant::now());
        let casting = Held::casting(&guard);
        assert!(casting.conflict(&move_from(23)).is_some());
        assert!(casting.conflict(&move_from(251)).is_some());
        assert_eq!(casting.conflict(&move_from(30)), None);
    }
}
