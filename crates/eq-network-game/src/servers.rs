//! The worlds a login server lists, which the player chooses among before
//! the chosen world's character list. Each client generation's login server
//! writes its list its own way; both read into these.
use serde::Serialize;

/// Whether a listed world takes players.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerStatus {
    /// It takes players.
    Up,
    /// It is down.
    Down,
    /// It is locked to all but the accounts its operators let in.
    Locked,
}

impl ServerStatus {
    /// The status word of a Titanium login server's list, as `EQEmu`'s login
    /// server writes it: 0 and 2 up, 1 and 3 down, 4 locked, and 5 locked
    /// while down (`WorldServer::SerializeForClientServerList`). Any other
    /// value is taken as down, as no player can be sent there.
    #[must_use]
    pub const fn titanium(value: u32) -> Self {
        match value {
            0 | 2 => Self::Up,
            4 | 5 => Self::Locked,
            _ => Self::Down,
        }
    }

    /// The status an `EQMac` login server's list gives a world in the field
    /// it counts the world's players in, with the count where it is one:
    /// TAKP's login server writes the world's own status there, which the
    /// world reports as -2 while locked, -1 while it has no zones up, and
    /// its player count otherwise (`ServerManager::CreateServerListPacket`
    /// and the world's `LoginServer::SendStatus`). Inferred from that
    /// source until checked against a TAKP login server. Any other
    /// negative value is taken as down.
    #[must_use]
    pub fn eqmac(value: i32) -> (Self, Option<u32>) {
        match value {
            -2 => (Self::Locked, None),
            players if players >= 0 => (Self::Up, u32::try_from(players).ok()),
            _ => (Self::Down, None),
        }
    }

    /// Whether the player may choose the world.
    #[must_use]
    pub const fn open(self) -> bool {
        matches!(self, Self::Up)
    }
}

/// One world a login server lists.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ServerChoice {
    /// The world's name as the list gives it, which a configured world is
    /// found by.
    pub name: String,
    /// Whether it takes players.
    pub status: ServerStatus,
    /// How many players the list says are on it, where it says.
    pub players: Option<u32>,
    /// Whether the list marks it preferred, which the official clients show
    /// apart (inferred from the login servers' sources: `EQEmu` marks its
    /// preferred servers, and TAKP marks none yet).
    pub preferred: bool,
}

/// Why the login server refused the world the player chose. The player
/// may choose again from the same list.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerRefusal {
    /// A string of the installed client's login string table
    /// (`eqlsstr_us.txt`), by its id, as a Titanium login server names it:
    /// for example 326, the world unavailable, in `EQEmu`'s numbering.
    Message(u32),
    /// The login server's own words, as an `EQMac` login server sends them.
    Text(String),
}

impl std::fmt::Display for ServerRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(id) => write!(formatter, "login string {id}"),
            Self::Text(text) => formatter.write_str(text.trim()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titanium_status_words_read_as_eqemu_writes_them() {
        for (value, status) in [
            (0, ServerStatus::Up),
            (1, ServerStatus::Down),
            (2, ServerStatus::Up),
            (3, ServerStatus::Down),
            (4, ServerStatus::Locked),
            (5, ServerStatus::Locked),
            (6, ServerStatus::Down),
            (u32::MAX, ServerStatus::Down),
        ] {
            assert_eq!(ServerStatus::titanium(value), status, "{value}");
        }
        assert!(ServerStatus::Up.open());
        assert!(!ServerStatus::Down.open() && !ServerStatus::Locked.open());
    }

    #[test]
    fn an_eqmac_count_is_the_players_or_the_worlds_status() {
        assert_eq!(ServerStatus::eqmac(42), (ServerStatus::Up, Some(42)));
        assert_eq!(ServerStatus::eqmac(0), (ServerStatus::Up, Some(0)));
        assert_eq!(ServerStatus::eqmac(-1), (ServerStatus::Down, None));
        assert_eq!(ServerStatus::eqmac(-2), (ServerStatus::Locked, None));
        assert_eq!(ServerStatus::eqmac(-7), (ServerStatus::Down, None));
    }
}
