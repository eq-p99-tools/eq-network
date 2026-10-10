//! What each client generation's servers send, and what the session does
//! with each message: one table per generation, one row per opcode.
//!
//! A row names the message, the server types whose servers send it, and the
//! parts of the session that read it, or why it is left unread: work still
//! to come, an answer to a request eq-network never sends, or nothing in it
//! worth reading. The login and zone sessions tell once of each message
//! they leave unread, but one with nothing worth reading, and of each the
//! table does not list ([`Unheard`]); the world session tells of every
//! message it receives. A test holds each table to its generation's
//! decoders.
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

    /// Whether the session tells of it when it arrives at `stage`: nothing
    /// there reads it, and something in it is worth reading.
    fn told_at(&self, stage: Stage) -> bool {
        !matches!(self.reading, Reading::Unread(Reason::Needless(_)))
            && !self.readers().iter().any(|reader| reader.at(stage))
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
    /// Whether it reads the messages that arrive at `stage`.
    const fn at(self, stage: Stage) -> bool {
        match stage {
            Stage::Login => matches!(self, Self::Login),
            Stage::Zone => !matches!(self, Self::Login | Self::World),
        }
    }

    /// Whether it is one of the generation's zone decoders, which a test
    /// can probe.
    #[cfg(test)]
    pub(super) const fn decodes(self) -> bool {
        matches!(self, Self::Zone | Self::Chat | Self::Answer)
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
    /// Nothing in it needs reading, for this reason, so the zone does not
    /// tell of it.
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
    /// The player's own state: stun, mesmerize and charm, the end of
    /// sneaking and hiding, disciplines and reuse timers, a target the
    /// server sets, rewards, and the confirmations the player answers.
    PlayerState,
    /// Spawns' appearance and effects: the appearance kinds left, illusions,
    /// renames, spell and level effects, sounds, projectiles and spawn
    /// states.
    Appearance,
    /// Social, guild, title and GM messages: inspection, duels, guild
    /// membership, titles and last names, popups and the GM notices.
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
/// What the session leaves unread of an action.
const ACTION: &[Part] = &[Part(
    "actions of a kind other than a spell's",
    Reason::Planned(Step::PlayerState),
)];
/// What the session leaves unread of `EQMac`'s word that the server took or
/// gave back the player's control.
const OWN_NAME: &[Part] = &[Part(
    "the player's own name",
    Reason::Needless("the session has it from its entry"),
)];
/// What the session leaves unread of `EQMac`'s log-server settings.
const LOG_SERVER: &[Part] = &[Part(
    "the error-reporting server's address, port, password and e-mail",
    Reason::Needless("nothing here reports to it"),
)];
/// What the session leaves unread of Titanium's zone header.
const TITANIUM_DESCRIPTION: &[Part] = &[
    Part(
        "the character's name, the zone's number and a second copy of its name",
        Reason::Needless("the session has them from its entry and the player's profile"),
    ),
    Part(
        "the zone's instance",
        Reason::Needless("EQEmu never sets it (zone/zone.cpp Zone::LoadZoneCFG)"),
    ),
    Part(
        "what EQEmu sends past the header's 700 bytes",
        Reason::Needless(
            "it is for later clients; the Titanium client copies only the 700 \
             (common/eq_packet_structs.h NewZone_Struct)",
        ),
    ),
];
/// What the session leaves unread of `EQMac`'s zone header.
const EQMAC_DESCRIPTION: &[Part] = &[
    Part(
        "the character's name",
        Reason::Needless("the session has it from its entry"),
    ),
    Part(
        "special dates and codes, the time zone, and the forage, fishing and script settings",
        Reason::Needless(
            "neither server sets them (common/patches/mac.cpp ENCODE(OP_NewZone), \
             zone/zone.cpp)",
        ),
    ),
];

/// Where in the session a message arrives.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Stage {
    /// At the login server.
    Login,
    /// At a zone, its admission of the player included.
    Zone,
}

impl Stage {
    /// Where else the session reads what it leaves unread here.
    const fn elsewhere(self) -> &'static str {
        match self {
            Self::Login => "after the login",
            Self::Zone => "before the zone",
        }
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Login => "Login",
            Self::Zone => "Zone",
        })
    }
}

/// The messages a login or zone session leaves unread, each told once a
/// session but those with nothing worth reading.
pub(super) struct Unheard {
    stage: Stage,
    told: HashSet<u16>,
}

impl Unheard {
    /// Nothing told yet of what arrives at `stage`.
    pub(super) fn new(stage: Stage) -> Self {
        Self {
            stage,
            told: HashSet::new(),
        }
    }

    /// Tells, the first time one arrives in the session, of a message the
    /// generation lists but this stage leaves unread, unless nothing in it is
    /// worth reading, and of one it does not list.
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
        if message.is_some_and(|message| !message.told_at(self.stage)) || !self.told.insert(opcode)
        {
            return Ok(());
        }
        let why = match message {
            None => "no server is listed as sending it".to_owned(),
            Some(ServerMessage {
                reading: Reading::Unread(reason),
                ..
            }) => reason.to_string(),
            Some(_) => format!("the session reads it only {}", self.stage.elsewhere()),
        };
        let name = message.map_or("an unlisted message", |message| message.name);
        log.diagnostic(format!(
            "{} sent {name} (0x{opcode:04x}, {length} bytes), left unread: {why}",
            self.stage
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
    fn the_login_tells_once_of_each_message_it_leaves_unread() {
        use crate::client::session::wire::EqMac;
        let config = ClientConfig::new("ACCOUNT", "PASSWORD", "Test Server", "Tester");
        let mut said = Vec::new();
        let mut handler = |event| {
            if let ClientEvent::Diagnostic(line) = event {
                said.push(line);
            }
            Ok(())
        };
        let mut log = Events::new(&config, &mut handler);
        let mut unheard = Unheard::new(Stage::Login);
        // The login server's ready message, read there; its banner, which
        // answers what eq-network never asks; a zone's position update; and
        // an opcode no server is listed as sending: each arriving twice.
        for opcode in [
            0x5900, 0x5200, 0xf340, 0x0001, 0x5900, 0x5200, 0xf340, 0x0001,
        ] {
            unheard.tell(&EqMac, opcode, 4, &mut log).unwrap();
        }
        drop(log);
        assert_eq!(
            said,
            [
                "Login sent OP_ClientUpdate (0xf340, 4 bytes), left unread: the session reads \
                 it only after the login",
                "Login sent an unlisted message (0x0001, 4 bytes), left unread: no server is \
                 listed as sending it",
            ]
        );
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
        let mut unheard = Unheard::new(Stage::Zone);
        // A position, read; an adventure merchant's sale, left unread; the
        // character list, read only from the world; an opcode no server is
        // listed as sending; and the empty AA stats, which carry nothing:
        // each arriving twice.
        for opcode in [
            0x14cb, 0x0097, 0x4513, 0x0001, 0x5996, 0x14cb, 0x0097, 0x4513, 0x0001, 0x5996,
        ] {
            unheard.tell(&Titanium, opcode, 8, &mut log).unwrap();
        }
        drop(log);
        assert_eq!(
            said,
            [
                "Zone sent OP_AdventureMerchantSell (0x0097, 8 bytes), left unread: planned \
                 with later and rarely used systems",
                "Zone sent OP_SendCharInfo (0x4513, 8 bytes), left unread: the session reads \
                 it only before the zone",
                "Zone sent an unlisted message (0x0001, 8 bytes), left unread: no server is \
                 listed as sending it",
            ]
        );
    }
}
