//! What each client generation's servers send, and what the session does
//! with each message: one table per generation, one row per opcode.
//!
//! A row names the message, the server types whose servers send it, and the
//! parts of the session that read it, or why it is left unread: work still
//! to come, an answer to a request eq-network never sends, or nothing in it
//! worth reading. The zone session tells once of each message it leaves
//! unread and of each the table does not list ([`Unheard`]), and a test holds
//! each table to its generation's decoders.
//!
//! What the servers send comes from the emulators' sources: `EQEmu`'s for
//! Titanium, TAKP's and Quarm's for `EQMac`. P99's server is closed, so its
//! rows are the few messages only it sends, and what else it sends is not
//! marked; Quarm's login server is not in its source, so the login rows
//! name TAKP's. Opcodes are the values on the wire.

mod eqmac;
mod titanium;

pub(super) use eqmac::eqmac;
pub(super) use titanium::titanium;

use super::{wire::Wire, Events, ServerProtocol};
use anyhow::Result;
use std::{collections::HashSet, fmt};

/// A message a client generation's servers send.
#[derive(Clone, Copy, Debug)]
pub(super) struct ServerMessage {
    /// Its name in the emulators' opcode lists, or for P99's own, what it
    /// is.
    pub(super) name: &'static str,
    /// The server types whose servers send it.
    #[allow(
        dead_code,
        reason = "a record for readers, which the session does not consult"
    )]
    pub(super) senders: &'static [ServerProtocol],
    /// What the session does with it.
    pub(super) reading: Reading,
}

impl ServerMessage {
    /// The parts of the session that read it; none when it is left unread.
    pub(super) const fn readers(&self) -> &'static [Reader] {
        match self.reading {
            Reading::Read(readers) | Reading::Partly(readers, _) => readers,
            Reading::Unread(_) => &[],
        }
    }

    /// Whether the zone session reads it.
    fn read_in_zone(&self) -> bool {
        self.readers().iter().any(|reader| reader.in_zone())
    }
}

/// A row of a generation's table.
const fn sent(
    name: &'static str,
    senders: &'static [ServerProtocol],
    reading: Reading,
) -> ServerMessage {
    ServerMessage {
        name,
        senders,
        reading,
    }
}

/// Left unread until this step reads it.
const fn planned(step: Step) -> Reading {
    Reading::Unread(Reason::Planned(step))
}

/// An answer to a request eq-network never sends, left unread until the
/// step that sends the request.
const fn unasked(step: Step) -> Reading {
    Reading::Unread(Reason::Unasked(step))
}

/// What the session does with a message.
#[derive(Clone, Copy, Debug)]
pub(super) enum Reading {
    /// These parts of the session read it.
    Read(&'static [Reader]),
    /// These parts of the session read it, all but the parts named.
    Partly(
        &'static [Reader],
        #[allow(
            dead_code,
            reason = "a record for readers, which the session does not consult"
        )]
        &'static [Part],
    ),
    /// The session leaves it unread.
    Unread(Reason),
}

/// A part of a message the session leaves unread, and why.
#[derive(Clone, Copy, Debug)]
#[allow(
    dead_code,
    reason = "a record for readers, which the session does not consult"
)]
pub(super) struct Part(pub(super) &'static str, pub(super) Reason);

/// A part of the session that reads the servers' messages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Reader {
    /// The login server's exchange.
    Login,
    /// The world server's: validation, the character list and the handoff
    /// to a zone.
    World,
    /// The zone's admission of the player.
    Admission,
    /// The generation's decoders, whose messages every zone feature hears
    /// ([`Wire::messages`](super::wire::Wire::messages)).
    Zone,
    /// The zone's communication: chat, emotes and the server's own lines
    /// ([`Wire::chat`](super::wire::Wire::chat)).
    Chat,
    /// What the generation's client answers by itself ([`Wire::answer`](super::wire::Wire::answer)).
    Answer,
}

impl Reader {
    /// Whether it reads the zone's messages.
    const fn in_zone(self) -> bool {
        !matches!(self, Self::Login | Self::World)
    }
}

/// Why the session leaves a message, or a part of one, unread.
#[derive(Clone, Copy, Debug)]
pub(super) enum Reason {
    /// Work still to come reads it.
    Planned(Step),
    /// It answers a request eq-network never sends; the work that sends the
    /// request reads it.
    Unasked(Step),
    /// Parked work reads it.
    Parked(&'static str),
    /// Nothing in it needs reading, for this reason.
    Needless(&'static str),
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Planned(step) => write!(f, "planned with {step}"),
            Self::Unasked(step) => write!(
                f,
                "it answers a request eq-network never sends (planned with {step})"
            ),
            Self::Parked(work) => write!(f, "parked in {work}"),
            Self::Needless(why) => f.write_str(why),
        }
    }
}

/// The work still to come that reads what the session leaves unread, one
/// step at a time.
#[derive(Clone, Copy, Debug)]
pub(super) enum Step {
    /// The world and the session: a zone that is down, the character the
    /// world enters, the world's rules and expansions, the chat server's
    /// offer, the logout's first step, the weather, and Quarm's move to the
    /// bind point and its shared bank offer.
    WorldAndSession,
    /// The player's own state: the push of a hit, stun, mesmerize and charm,
    /// the end of sneaking and hiding, disciplines and reuse timers, a
    /// target the server sets, rewards, and the confirmations the player
    /// answers.
    PlayerState,
    /// Spawns' appearance and effects: the appearance kinds left, illusions,
    /// renames, spell and level effects, sounds, projectiles and spawn
    /// states.
    Appearance,
    /// Social, guild, title and GM messages: inspection, duels, guild
    /// membership, titles, popups and the GM notices.
    Social,
    /// `EQMac`'s catch-up on the world: what Titanium's decoders already
    /// read, from doors to the training window.
    EqMacWorld,
    /// `EQMac`'s catch-up on exchanges: trades, loot and resurrection.
    EqMacExchanges,
    /// Alternate advancement and leadership.
    AlternateAdvancement,
    /// The bazaar and traders.
    Bazaar,
    /// Later and rarely used systems: tasks, adventures, tribute, the guild
    /// bank and the like.
    LaterSystems,
    /// Group roles and raids.
    Raids,
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WorldAndSession => "the world and the session",
            Self::PlayerState => "the player's own state",
            Self::Appearance => "spawns' appearance and effects",
            Self::Social => "social, guild, title and GM messages",
            Self::EqMacWorld => "EQMac's catch-up on the world",
            Self::EqMacExchanges => "EQMac's catch-up on exchanges",
            Self::AlternateAdvancement => "alternate advancement and leadership",
            Self::Bazaar => "the bazaar and traders",
            Self::LaterSystems => "later and rarely used systems",
            Self::Raids => "group roles and raids",
        })
    }
}

/// `EQEmu`'s servers, whose sources list what Titanium's servers send.
const EQEMU: &[ServerProtocol] = &[ServerProtocol::EqEmu];
/// P99's, for what only P99 sends.
const P99: &[ServerProtocol] = &[ServerProtocol::Project1999];
/// TAKP's and Quarm's.
const EQMAC: &[ServerProtocol] = &[ServerProtocol::Takp, ServerProtocol::Quarm];
/// TAKP's alone.
const TAKP: &[ServerProtocol] = &[ServerProtocol::Takp];
/// Quarm's alone.
const QUARM: &[ServerProtocol] = &[ServerProtocol::Quarm];

/// What the session leaves unread of the player's profile.
const PROFILE: &[Part] = &[
    Part(
        "languages and disciplines",
        Reason::Planned(Step::PlayerState),
    ),
    Part(
        "alternate advancement and leadership",
        Reason::Planned(Step::AlternateAdvancement),
    ),
    Part("tribute", Reason::Planned(Step::LaterSystems)),
];
/// What the session leaves unread of a spawn's appearance.
const APPEARANCE: &[Part] = &[Part(
    "the other appearance kinds",
    Reason::Planned(Step::Appearance),
)];
/// What the session leaves unread of a hit.
const PUSH: &[Part] = &[Part("the push", Reason::Planned(Step::PlayerState))];
/// What the session leaves unread of an action.
const ACTION: &[Part] = &[Part(
    "melee and skill actions, and the push",
    Reason::Planned(Step::PlayerState),
)];
/// What the session leaves unread of the weather.
const WEATHER: &[Part] = &[Part(
    "its kind and intensity",
    Reason::Planned(Step::WorldAndSession),
)];
/// What the session leaves unread of the zone's description.
const DESCRIPTION: &[Part] = &[Part(
    "fog, gravity, the safe point and the rest",
    Reason::Planned(Step::WorldAndSession),
)];

/// The messages a zone session leaves unread, each told once a session.
#[derive(Default)]
pub(super) struct Unheard(HashSet<u16>);

impl Unheard {
    /// Tells, the first time one arrives in the session, of a message the
    /// generation lists but the zone leaves unread, and of one it does not
    /// list.
    ///
    /// # Errors
    /// Returns an error when the host's event handler fails.
    pub(super) fn tell(
        &mut self,
        wire: &dyn Wire,
        opcode: u16,
        length: usize,
        log: &mut Events<'_>,
    ) -> Result<()> {
        let message = wire.coverage(opcode);
        if message.is_some_and(|message| message.read_in_zone()) || !self.0.insert(opcode) {
            return Ok(());
        }
        let why = match message {
            None => "no server is listed as sending it".to_owned(),
            Some(ServerMessage {
                reading: Reading::Unread(reason),
                ..
            }) => reason.to_string(),
            Some(_) => "the session reads it only before the zone".to_owned(),
        };
        let name = message.map_or("an unlisted message", |message| message.name);
        log.diagnostic(format!(
            "Zone sent {name} (0x{opcode:04x}, {length} bytes), left unread: {why}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{session::wire::Titanium, ClientConfig, ClientEvent};

    #[test]
    fn every_listed_message_is_named_and_says_what_reads_it() {
        for (generation, table) in [
            ("Titanium", titanium as fn(u16) -> Option<ServerMessage>),
            ("EQMac", eqmac),
        ] {
            for opcode in 0..=u16::MAX {
                let Some(message) = table(opcode) else {
                    continue;
                };
                let row = format!("{generation} 0x{opcode:04x}");
                assert!(!message.name.is_empty(), "{row}");
                match message.reading {
                    Reading::Read(readers) => assert!(!readers.is_empty(), "{row}"),
                    Reading::Partly(readers, parts) => {
                        assert!(!readers.is_empty() && !parts.is_empty(), "{row}");
                    }
                    Reading::Unread(_) => (),
                }
            }
        }
    }

    #[test]
    fn the_zone_tells_once_of_each_message_it_leaves_unread() {
        let config = ClientConfig::new("ACCOUNT", "PASSWORD", "Test Server", "Tester");
        let mut said = Vec::new();
        let mut handler = |event| {
            if let ClientEvent::Diagnostic(line) = event {
                said.push(line);
            }
            Ok(())
        };
        let mut log = Events::new(&config, &mut handler);
        let mut unheard = Unheard::default();
        // A position, read; a stun, left unread; the character list, read
        // only from the world; and an opcode no server is listed as sending,
        // each arriving twice.
        for opcode in [
            0x14cb, 0x1e51, 0x4513, 0x0001, 0x14cb, 0x1e51, 0x4513, 0x0001,
        ] {
            unheard.tell(&Titanium, opcode, 8, &mut log).unwrap();
        }
        drop(log);
        assert_eq!(
            said,
            [
                "Zone sent OP_Stun (0x1e51, 8 bytes), left unread: planned with the \
                 player's own state",
                "Zone sent OP_SendCharInfo (0x4513, 8 bytes), left unread: the session reads \
                 it only before the zone",
                "Zone sent an unlisted message (0x0001, 8 bytes), left unread: no server is \
                 listed as sending it",
            ]
        );
    }
}
