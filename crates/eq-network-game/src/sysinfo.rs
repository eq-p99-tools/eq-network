//! The machine a player enters the world on, told to the world as the
//! official client tells it: one UDP message, opcode `0x1C4C`, a 344-byte
//! record, sent once after the player enters the world and then not again
//! until a week has passed.
//!
//! Why this library bothers: the official client sends it every week on
//! every login, and a client that never sends it looks different on the
//! wire. The world's own answer to it has never been seen (AkkStack's
//! login server and world discard the opcode unread, `loginserver/client.cpp`
//! and `world/client.cpp`), so this is identity traffic, not conversation;
//! it is listed per server type, not for every world.
//!
//! The record's fields were read off the wire on 2026-10-30, in a live
//! login to P99 Green with the official client, and matched field by field
//! against the gathering code in the official program (`eqgame.exe`, the
//! one caller of `dsetup.dll!Ordinal_11` inside it). Each constant below
//! says where it came from; the ones labelled *inferred* are the official
//! client's own answer under wine, and the first thing to re-check if a
//! real Windows capture ever disagrees.
use crate::command::EncodedCommand;

/// `OP_SystemInfo` (`0x1C4C`), the client's one-way report. The servers
/// this project has watched answer it with nothing at all.
pub const OPCODE: u16 = 0x1c4c;

/// How long a report stays current: the official client compares its age
/// as strictly greater than 604,799 seconds — a week less a second
/// (`0x93a7f` in the gathering code) — against its own saved send time.
/// Do not round this to 604,800: the comparison is `>`, not `>=`, and the
/// odd second is what the program actually does.
pub const REPORT_SECONDS: u64 = 604_799;

/// The exact size of the record, as sent.
pub const RECORD_BYTES: usize = 344;

/// The first five dwords: the graphics engine's own version and build —
/// `1.06.0007` plus a 10,000 tag — as the shipped `EQGraphicsDX9.dll`
/// reports them to the client that then reports them here. Every install
/// of the current P99 patch carries the same DLL, so this is a constant,
/// not a measurement of this machine.
const ENGINE_VERSION: [u32; 5] = [1, 6, 7, 1, 10_000];

/// Five bytes after it: the engine's own feature flags, same on every
/// install of that DLL.
const ENGINE_FLAGS: [u8; 5] = [1, 1, 1, 0, 0];

/// A dword at record offset 116 the engine carries with the settings; the
/// capture shows one value and no source for it. Send what was seen; do
/// not "fix" it until a second capture says otherwise.
const ENGINE_SETTINGS: u32 = 0xff8d_ce33;

/// Three bytes at offset 120, before the card's name: seen as
/// `01 01 01` on the wire; purpose unknown, likely engine settings.
const ENGINE_MARKS: [u8; 3] = [1, 1, 1];

/// The two dwords at offsets 188 and 192 are everything `dsetup.dll`
/// contributes to the packet: the official program asks its single export
/// for them just before it sends, and the packed stub answers with two
/// values it computed during its load-time probe suite. Under wine the
/// stub takes its no-answer path and writes neither, so the official
/// client itself sent zeros here in the capture; the values on a real
/// Windows machine are *inferred* to be the same until an owner's Windows
/// tcpdump says otherwise. Nothing computes them from this machine — do
/// not invent a source; a live Windows capture is the only way to change
/// this pair.
const DSETUP_ANSWER: [u32; 2] = [0, 0];

/// The OS block at offsets 196..212: platform 2 (Windows NT), version 6.2,
/// build 0, service pack 0. The official program asks the OS the way every
/// unmanifested Titanium-era program does, which pins it to exactly these
/// answers on every Windows the client runs on, and wine answers the same;
/// the captured packet matches. It is deliberately not this machine's
/// operating system: the world has only ever seen this.
const OS_VERSION: [u32; 4] = [2, 6, 2, 0];

/// What the machine is, as the session reports it to the world.
///
/// The host (the app) fills the fields it can see — the processor's brand
/// string, the display adapter's name, memory and display modes — with
/// this machine's real values, and carries `reported_at`, the saved time
/// of the last report (zero when there is none yet). The library owns
/// when and whether the report goes out; the host owns persisting it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SysInfo {
    /// The processor's brand string, as the official client takes it from
    /// the graphics engine, which reads it from the CPU itself. Longer
    /// than 50 characters is cut, as the fixed field forces.
    pub cpu: String,
    /// The display adapter's name, from the engine's device object.
    /// Longer than 64 characters is cut.
    pub gpu: String,
    /// Total physical memory, rounded up to megabytes, as
    /// `GlobalMemoryStatus` gives it: the official record holds
    /// `dwTotalPhys`.
    pub physical_mb: u32,
    /// Total commit, rounded up to megabytes, the same call's
    /// `dwTotalPageFile`. The official client under wine reported the
    /// same 2,048 as memory there; report this machine's real value.
    pub virtual_mb: u32,
    /// Colour depth, in bits: the engine is asked twice, and answers the
    /// same both times, so the record carries it twice.
    pub color_depth: u32,
    /// The desktop's width, as the engine reports it.
    pub desktop_width: u32,
    /// The desktop's height, as the engine reports it.
    pub desktop_height: u32,
    /// The width the game screen is in.
    pub screen_width: u32,
    /// The height the game screen is in.
    pub screen_height: u32,
    /// The screen's refresh rate, in hertz.
    pub refresh_hz: u32,
    /// When the last report went out, in whole seconds since the epoch;
    /// zero says none ever did. The host keeps this in the installed
    /// client's own file (`[Defaults] LastSysInfoDate` of `eqclient.ini`,
    /// the official client's chosen store) and refreshes it when the
    /// session says it reported.
    pub reported_at: u64,
}

impl SysInfo {
    /// Whether the world is due this machine's account of itself: the
    /// official comparison, `REPORT_SECONDS < now - last`, where `last`
    /// is the newer of the saved stamp (`reported_at`, read back from the
    /// host's `eqclient.ini`) and `last_report` — the time this session
    /// itself sent one, which the session keeps so a zone crossing back
    /// through the world inside the week stays quiet even before the
    /// first report's stamp reaches disk. A report dated in the future is
    /// never due.
    #[must_use]
    pub const fn is_due(&self, now: u64, last_report: u64) -> bool {
        let last = if self.reported_at > last_report {
            self.reported_at
        } else {
            last_report
        };
        now.saturating_sub(last) > REPORT_SECONDS
    }

    /// The report as the world receives it: [`OPCODE`] and exactly
    /// [`RECORD_BYTES`] bytes, laid out as the capture shows. Everything
    /// the official gatherer left zeroed — padding, the empty service-pack
    /// string — goes out zeroed here too, so the record starts as zeroes
    /// and only the fields below are written.
    #[must_use]
    pub fn encode(&self) -> EncodedCommand {
        let mut body = vec![0; RECORD_BYTES];
        body[..20].clone_from_slice(&by_words(&ENGINE_VERSION));
        body[20..25].clone_from_slice(&ENGINE_FLAGS);
        // The CPU's brand, in its 51-byte field: 50 at most, NUL-padded.
        put_cuts(&mut body, 25, 51, &self.cpu);
        body[76..80].clone_from_slice(&self.physical_mb.to_le_bytes());
        body[80..84].clone_from_slice(&self.virtual_mb.to_le_bytes());
        // The engine's settings block, offsets 84..116: the two asks the
        // official gatherer makes of its device object — desktop mode,
        // then the game screen — each colour depth, width, height, with a
        // word between the pairs that read zero on the wire.
        for (at, value) in [
            (84, self.color_depth),
            (88, self.desktop_width),
            (92, self.desktop_height),
            (96, 0),
            (100, self.color_depth),
            (104, self.screen_width),
            (108, self.screen_height),
            (112, self.refresh_hz),
        ] {
            body[at..at + 4].clone_from_slice(&value.to_le_bytes());
        }
        body[116..120].clone_from_slice(&ENGINE_SETTINGS.to_le_bytes());
        body[120..123].clone_from_slice(&ENGINE_MARKS);
        // The adapter's name, in its 65-byte field: 64 at most, NUL-padded.
        put_cuts(&mut body, 123, 65, &self.gpu);
        body[188..196].clone_from_slice(&by_words(&DSETUP_ANSWER));
        body[196..212].clone_from_slice(&by_words(&OS_VERSION));
        // The service-pack string, 128 bytes at 216, is left zero, as the
        // capture shows every current Windows leaving it for this client.
        EncodedCommand {
            opcode: OPCODE,
            body,
        }
    }
}

impl Default for SysInfo {
    /// The report an official client could have sent, apart from its
    /// machine's strings and measures: nothing reported yet.
    fn default() -> Self {
        Self {
            cpu: String::new(),
            gpu: String::new(),
            physical_mb: 0,
            virtual_mb: 0,
            color_depth: 0,
            desktop_width: 0,
            desktop_height: 0,
            screen_width: 0,
            screen_height: 0,
            refresh_hz: 0,
            reported_at: 0,
        }
    }
}

fn by_words<const N: usize>(words: &[u32; N]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(words.len() * 4);
    for word in words {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes
}

/// Writes a string into a fixed, zero-filled field, cutting it to the
/// field's length less its terminator and never inside a character, as
/// the official fixed buffers force.
fn put_cuts(body: &mut [u8], at: usize, field: usize, text: &str) {
    let room = field - 1;
    let mut take = text.len().min(room);
    while take > 0 && !text.is_char_boundary(take) {
        take -= 1;
    }
    body[at..at + take].clone_from_slice(&text.as_bytes()[..take]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> SysInfo {
        SysInfo {
            cpu: "Intel(R) Core(TM) Test CPU".into(),
            gpu: "Test Display Adapter".into(),
            physical_mb: 32_768,
            virtual_mb: 40_960,
            color_depth: 32,
            desktop_width: 1920,
            desktop_height: 1080,
            screen_width: 1280,
            screen_height: 720,
            refresh_hz: 60,
            reported_at: 0,
        }
    }

    #[test]
    fn the_record_lays_every_field_where_the_capture_saw_it() {
        let packet = report().encode();
        assert_eq!(packet.opcode, 0x1c4c);
        assert_eq!(packet.body.len(), RECORD_BYTES);
        let word = |at: usize| u32::from_le_bytes(packet.body[at..at + 4].try_into().unwrap());
        // The engine's own account of itself, first and unchanged.
        assert_eq!(
            &packet.body[..20],
            &[1, 0, 0, 0, 6, 0, 0, 0, 7, 0, 0, 0, 1, 0, 0, 0, 0x10, 0x27, 0, 0]
        );
        assert_eq!(&packet.body[20..25], &[1, 1, 1, 0, 0]);
        // The machine's strings, NUL-padded in their fixed fields.
        assert!(packet.body[25..].starts_with(b"Intel(R) Core(TM) Test CPU\0"));
        assert!(packet.body[123..].starts_with(b"Test Display Adapter\0"));
        assert!(packet.body[51..76].iter().all(|&byte| byte == 0));
        assert_eq!(packet.body[76..80], 32_768u32.to_le_bytes());
        assert_eq!(packet.body[80..84], 40_960u32.to_le_bytes());
        // The settings the engine gives: desktop then screen, each with
        // the colour depth it was asked with.
        assert_eq!(word(84), 32);
        assert_eq!(word(88), 1920);
        assert_eq!(word(92), 1080);
        assert_eq!(word(96), 0);
        assert_eq!(word(100), 32);
        assert_eq!(word(104), 1280);
        assert_eq!(word(108), 720);
        assert_eq!(word(112), 60);
        assert_eq!(word(116), 0xff8d_ce33);
        assert_eq!(&packet.body[120..123], &[1, 1, 1]);
        // The two dwords the packed setup DLL answers with, and the OS
        // block no current Windows lets an unmanifested client get any
        // other answer from.
        assert_eq!(word(188), 0);
        assert_eq!(word(192), 0);
        assert_eq!(
            &packet.body[196..212],
            &[2, 0, 0, 0, 6, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]
        );
        // The rest of the record is what the gatherer never filled.
        assert!(packet.body[216..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn a_long_name_is_cut_to_its_field_as_the_official_buffers_force() {
        let long = "x".repeat(200);
        let packet = SysInfo {
            cpu: long.clone(),
            gpu: long.clone(),
            ..report()
        }
        .encode();
        assert_eq!(packet.body[25..75], [b'x'; 50]);
        assert_eq!(packet.body[75], 0, "the CPU field holds 50 and a NUL");
        assert_eq!(packet.body[123..187], [b'x'; 64]);
        assert_eq!(packet.body[187], 0, "the adapter field holds 64 and a NUL");
        assert_eq!(packet.body[188], 0, "the cut does not eat the DLL's dwords");
    }

    #[test]
    fn the_report_is_due_a_week_less_a_second_after_the_last_one() {
        let now = 1_800_000_000;
        let seen = |ago: u64| SysInfo {
            reported_at: now - ago,
            ..report()
        };
        assert!(seen(now).is_due(now, 0), "one never sent is due");
        assert!(
            !seen(REPORT_SECONDS).is_due(now, 0),
            "exactly a week less one keeps quiet"
        );
        assert!(
            seen(REPORT_SECONDS + 1).is_due(now, 0),
            "a second past it speaks"
        );
        assert!(
            !SysInfo {
                reported_at: now + 60,
                ..report()
            }
            .is_due(now, 0),
            "a report from the future is not overdue"
        );
        // The session's own last send counts too, before the host's stamp
        // has caught up: a zone crossing back through the world inside the
        // week must not report twice…
        assert!(
            !seen(now).is_due(now, now - 5),
            "a report this session just sent is not due again"
        );
        // …and the saved stamp holds just as firmly the other way round.
        assert!(
            !seen(60).is_due(now, 0),
            "a fresh stamp on disk keeps a new session quiet"
        );
    }
}
