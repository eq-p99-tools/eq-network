//! One pace for all the chat a host asks the session to send: every zone
//! channel, tells, and the chat server's channel commands and messages,
//! whether typed, from a social or a hotbutton, or from a script
//! (recording checklist 6.523).
//!
//! Stock `EQEmu`'s anti-spam (`Client::ChannelMessageReceived`,
//! `zone/client.cpp`, with the `Chat` rules in `common/ruletypes.h`) counts
//! shout, auction, out-of-character and tells: 4 in an interval of 60 s,
//! more with karma up to 12, refused past that with its own line, and the
//! client is kicked at more than 20 attempts in an interval. A script or a
//! social loop could reach that kick, so the session paces chat itself:
//!
//! - any chat: at most [`ANY_CHAT`] in any [`ANY_WINDOW`] (a five-line
//!   social pressed twice goes; a loop does not);
//! - shout, auction, out-of-character, tells and channel messages: at most
//!   [`COUNTED_CHAT`] in any [`COUNTED_WINDOW`]. A sliding window bounds
//!   every span of that length, so however the server's interval falls it
//!   sees no more than 12, `EQEmu`'s most at full karma and below its kick.
//!
//! Both bounds are this library's, inferred until the program's own chat
//! pacing is read (R49-17); the original has no known limiter, so this is
//! a departure that only ever makes the client slower, the same on every
//! server type. A paced send is refused, never kept for later, and a
//! refusal is not counted, so refusals do not lengthen the wait. Only
//! sends a host asked for are counted: the session's own (the chat link's
//! login, the local fixture's depop) are not.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// Chat of any kind allowed in [`ANY_WINDOW`] (inferred, R49-17).
pub(super) const ANY_CHAT: usize = 10;
/// The window [`ANY_CHAT`] counts in.
pub(super) const ANY_WINDOW: Duration = Duration::from_secs(5);
/// Shout, auction, out-of-character, tells and channel messages allowed in
/// [`COUNTED_WINDOW`] (inferred, R49-17; `EQEmu`'s
/// `MaximumMessagesPerInterval`).
pub(super) const COUNTED_CHAT: usize = 12;
/// The window [`COUNTED_CHAT`] counts in (`EQEmu`'s `IntervalDurationMS`).
pub(super) const COUNTED_WINDOW: Duration = Duration::from_secs(60);

/// Which bounds a send counts toward.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// Say, group, guild, raid and the chat server's commands.
    Local,
    /// Shout, auction, out-of-character, a tell, or a channel message:
    /// what `EQEmu` counts, and what reaches people far away.
    Counted,
}

impl Kind {
    /// The kind of a zone chat line.
    pub(super) fn of(chat: &eq_network_game::chat::OutboundChat) -> Self {
        use eq_network_game::chat::OutboundChat;
        match chat {
            OutboundChat::Guild(_)
            | OutboundChat::Group(_)
            | OutboundChat::Say(_)
            | OutboundChat::Raid(_) => Self::Local,
            // Shout, auction, out-of-character and tells; and a channel
            // added later, until it is placed: the slower choice.
            _ => Self::Counted,
        }
    }

    /// The kind of a chat server command: a message to a channel by its
    /// number or its name, or another command.
    pub(super) fn of_channel(text: &str) -> Self {
        if text
            .trim_start()
            .starts_with(|first: char| first.is_ascii_digit() || first == '#')
        {
            Self::Counted
        } else {
            Self::Local
        }
    }
}

/// A send the pace refused, and how long until one would go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Paced {
    /// Whole seconds, rounded up, never zero.
    pub(super) seconds: u64,
}

impl Paced {
    /// The player's line for it, which never repeats what they said.
    pub(super) fn reason(self) -> String {
        let seconds = self.seconds;
        let unit = if seconds == 1 { "second" } else { "seconds" };
        format!("You are sending chat too fast. Wait {seconds} {unit}.")
    }
}

/// The pace a character's zone sessions share, so it spans zones.
pub(in crate::client) type SharedPace = std::sync::Arc<std::sync::Mutex<ChatPace>>;

/// Sends through `send` if the shared pace takes a send of this kind now,
/// and counts it only if it went; otherwise says how long to wait. A send
/// refused for any reason is not counted.
pub(super) fn paced<E>(
    pace: &SharedPace,
    kind: Kind,
    now: Instant,
    send: impl FnOnce() -> Result<(), E>,
) -> Result<Result<(), E>, Paced> {
    let mut held = pace
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    held.check(kind, now)?;
    let outcome = send();
    if outcome.is_ok() {
        held.record(kind, now);
    }
    Ok(outcome)
}

/// When the last sends went, each list no longer than its bound.
#[derive(Debug, Default)]
pub(in crate::client) struct ChatPace {
    any: VecDeque<Instant>,
    counted: VecDeque<Instant>,
}

impl ChatPace {
    /// Takes a send of this kind now, or says how long to wait.
    #[cfg(test)]
    fn admit(&mut self, kind: Kind, now: Instant) -> Result<(), Paced> {
        self.check(kind, now)?;
        self.record(kind, now);
        Ok(())
    }

    /// Whether a send of this kind may go now, or how long to wait.
    fn check(&self, kind: Kind, now: Instant) -> Result<(), Paced> {
        let wait = [
            Some(Self::wait(&self.any, ANY_CHAT, ANY_WINDOW, now)),
            (kind == Kind::Counted)
                .then(|| Self::wait(&self.counted, COUNTED_CHAT, COUNTED_WINDOW, now)),
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or_default();
        if !wait.is_zero() {
            // Whole seconds, rounded up: a wait of 1 ns still says 1.
            let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
            return Err(Paced { seconds });
        }
        Ok(())
    }

    /// Counts a send of this kind that went now.
    fn record(&mut self, kind: Kind, now: Instant) {
        Self::push(&mut self.any, ANY_CHAT, now);
        if kind == Kind::Counted {
            Self::push(&mut self.counted, COUNTED_CHAT, now);
        }
    }

    /// How long until a list with `bound` sends in `window` takes another:
    /// zero when it is under its bound within the window now.
    fn wait(sent: &VecDeque<Instant>, bound: usize, window: Duration, now: Instant) -> Duration {
        if sent.len() < bound {
            return Duration::ZERO;
        }
        // The oldest of the last `bound` sends leaves the window at
        // `oldest + window`; the list holds only those.
        sent.front().map_or(Duration::ZERO, |oldest| {
            (*oldest + window).saturating_duration_since(now)
        })
    }

    fn push(sent: &mut VecDeque<Instant>, bound: usize, now: Instant) {
        if sent.len() == bound {
            sent.pop_front();
        }
        sent.push_back(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_lines_go_in_five_seconds_and_the_eleventh_waits_until_the_first_leaves() {
        let start = Instant::now();
        let mut pace = ChatPace::default();
        for line in 0..ANY_CHAT {
            let at = start + Duration::from_millis(100 * line as u64);
            assert_eq!(pace.admit(Kind::Local, at), Ok(()), "{line}");
        }
        // Just before the first leaves the window: wait the rest, rounded
        // up to whole seconds.
        let early = (start + ANY_WINDOW)
            .checked_sub(Duration::from_nanos(1))
            .unwrap();
        assert_eq!(pace.admit(Kind::Local, early), Err(Paced { seconds: 1 }));
        let mid = start + Duration::from_millis(1500);
        assert_eq!(pace.admit(Kind::Local, mid), Err(Paced { seconds: 4 }));
        // At the instant it leaves, the next goes.
        assert_eq!(pace.admit(Kind::Local, start + ANY_WINDOW), Ok(()));
        // The refusals counted for nothing: the next waits only for the
        // second line to leave.
        let next = start + ANY_WINDOW;
        assert_eq!(
            pace.admit(Kind::Local, next),
            Err(Paced { seconds: 1 }),
            "the second line left at 5.1 s"
        );
        assert_eq!(
            pace.admit(Kind::Local, start + ANY_WINDOW + Duration::from_millis(100)),
            Ok(())
        );
    }

    #[test]
    fn twelve_far_reaching_lines_go_in_a_minute_and_the_thirteenth_waits() {
        let start = Instant::now();
        let mut pace = ChatPace::default();
        // One every 2 s stays under the 5 s bound.
        let at = |line: u64| start + Duration::from_secs(2 * line);
        for line in 0..12 {
            assert_eq!(pace.admit(Kind::Counted, at(line)), Ok(()), "{line}");
        }
        // The 13th at 24 s waits until 60 s, when the first leaves.
        assert_eq!(
            pace.admit(Kind::Counted, at(12)),
            Err(Paced { seconds: 36 })
        );
        // Say, group and the like still go: only the 5 s bound holds them.
        assert_eq!(pace.admit(Kind::Local, at(12)), Ok(()));
        let edge = (start + COUNTED_WINDOW)
            .checked_sub(Duration::from_millis(1))
            .unwrap();
        assert_eq!(pace.admit(Kind::Counted, edge), Err(Paced { seconds: 1 }));
        assert_eq!(pace.admit(Kind::Counted, start + COUNTED_WINDOW), Ok(()));
        // Never more than 12 in any 60 s span: the next waits for the
        // second line's turn.
        assert_eq!(
            pace.admit(Kind::Counted, start + COUNTED_WINDOW),
            Err(Paced { seconds: 2 })
        );
    }

    #[test]
    fn a_flood_of_refused_lines_keeps_the_lists_at_their_bounds() {
        let start = Instant::now();
        let mut pace = ChatPace::default();
        for line in 0..10_000u64 {
            let _ = pace.admit(Kind::Counted, start + Duration::from_millis(line));
        }
        assert!(pace.any.len() <= ANY_CHAT);
        assert!(pace.counted.len() <= COUNTED_CHAT);
    }

    #[test]
    fn kinds_follow_what_the_server_counts() {
        use eq_network_game::chat::OutboundChat;
        for chat in [
            OutboundChat::Shout("a".into()),
            OutboundChat::Auction("a".into()),
            OutboundChat::Ooc("a".into()),
            OutboundChat::Tell {
                recipient: "Someone".into(),
                message: "a".into(),
            },
        ] {
            assert_eq!(Kind::of(&chat), Kind::Counted, "{chat:?}");
        }
        for chat in [
            OutboundChat::Say("a".into()),
            OutboundChat::Group("a".into()),
            OutboundChat::Guild("a".into()),
            OutboundChat::Raid("a".into()),
        ] {
            assert_eq!(Kind::of(&chat), Kind::Local, "{chat:?}");
        }
        assert_eq!(Kind::of_channel("2 hello"), Kind::Counted);
        assert_eq!(Kind::of_channel("#name hello"), Kind::Counted);
        assert_eq!(Kind::of_channel("join a, b"), Kind::Local);
        assert_eq!(Kind::of_channel("list"), Kind::Local);
    }

    #[test]
    fn the_line_says_how_long_and_never_what_was_said() {
        assert_eq!(
            Paced { seconds: 1 }.reason(),
            "You are sending chat too fast. Wait 1 second."
        );
        assert_eq!(
            Paced { seconds: 36 }.reason(),
            "You are sending chat too fast. Wait 36 seconds."
        );
    }
}
