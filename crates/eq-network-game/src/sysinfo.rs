//! The machine a player enters the world on, told to the world as the
//! official client tells it: one UDP message, opcode `0x1C4C`, a 344-byte
//! record, sent on the world connection just before the client enters the
//! world with its chosen character, and then not again until a week has
//! passed.
//!
//! Why this library bothers: the official client sends it every week on
//! every login, and a client that never sends it looks different on the
//! wire. The world's own answer to it has never been seen (AkkStack's
//! login server and world discard the opcode unread, `loginserver/client.cpp`
//! and `world/client.cpp`), so this is identity traffic, not conversation;
//! it is listed per server type, not for every world.
//!
//! The record's fields were read off the wire twice: on 2026-10-30 under
//! wine, and on 2026-10-31 from the official client on Windows entering P99
//! (the second capture fixed the adapter fields' layout, the send point and
//! which fields belong to the machine). Each constant below says where it
//! came from.
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

/// The first five dwords: the graphics engine's own version and build,
/// `1.06.0007` plus a 10,000 tag, as the shipped `EQGraphicsDX9.dll`
/// reports them. Both captures carry the same words; that every install of
/// the current patch does is inferred (recording checklist item 6.290).
const ENGINE_VERSION: [u32; 5] = [1, 6, 7, 1, 10_000];

/// Five bytes after it: the engine's own feature flags, the same in both
/// captures and labelled with the version (6.290).
const ENGINE_FLAGS: [u8; 5] = [1, 1, 1, 0, 0];

/// The two dwords at offsets 188 and 192: zero in both captures, under wine
/// and on Windows. Nothing here computes them; they go out as observed.
const OBSERVED_188: [u32; 2] = [0, 0];

/// The OS block at offsets 196..212 (platform id, major, minor and
/// service-pack major) and the service-pack string at 216: Windows XP
/// Service Pack 2, the answers the official client got in the Windows
/// capture, which runs it under that compatibility layer. Sent as a
/// constant by Adam's choice (2026-10-31 06:29Z: cap the version at XP SP2
/// so the report looks like a real client's), not this machine's system.
const OS_VERSION: [u32; 4] = [2, 5, 1, 2];

/// The service-pack string that goes with [`OS_VERSION`].
const SERVICE_PACK: &str = "Service Pack 2";

/// What the machine is, as the session reports it to the world.
///
/// The host (the app) fills the fields it can see — the processor's brand
/// string, the display adapter's name, memory and display modes — with
/// this machine's real values, and carries `reported_at`, the saved time
/// of the last report (zero when there is none yet). The library owns
/// when and whether the report goes out; the host owns persisting it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SysInfo {
    /// The processor's brand string, as the CPU reports it. Longer than
    /// 50 characters is cut, as the fixed field forces.
    pub cpu: String,
    /// Physical memory, in megabytes, as the record holds it (the host
    /// applies whatever cap the official record shows).
    pub physical_mb: u32,
    /// Virtual memory, in megabytes, as the record holds it.
    pub virtual_mb: u32,
    /// Colour depth, in bits: the record carries it for the desktop and
    /// again for the game screen.
    pub color_depth: u32,
    /// The desktop's width.
    pub desktop_width: u32,
    /// The desktop's height.
    pub desktop_height: u32,
    /// The width the game screen is in.
    pub screen_width: u32,
    /// The height the game screen is in.
    pub screen_height: u32,
    /// The screen's refresh rate, in hertz.
    pub refresh_hz: u32,
    /// The display adapter's available texture memory, in bytes, at
    /// offset 116 (Direct3D 9's `GetAvailableTextureMem`).
    pub texture_memory: u32,
    /// The three device format checks at offset 120, one byte each, 1 where
    /// the adapter takes the format (A4R4G4B4, DXT1, DXT3).
    pub formats: [u8; 3],
    /// The display adapter's description, in a 32-byte field at offset
    /// 123, cut to 32 bytes with no terminator, as the capture shows.
    pub adapter: String,
    /// The display device's name (such as `\\.\DISPLAY1`), in a 33-byte
    /// field at offset 155, 32 at most and a NUL.
    pub device: String,
    /// When the last report went out, in whole seconds since the epoch;
    /// zero says none ever did. The host keeps it and refreshes it when the
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
    /// [`RECORD_BYTES`] bytes, laid out as the captures show. Everything
    /// not written below goes out zero, as in both captures.
    #[must_use]
    pub fn encode(&self) -> EncodedCommand {
        let mut body = vec![0; RECORD_BYTES];
        body[..20].clone_from_slice(&by_words(&ENGINE_VERSION));
        body[20..25].clone_from_slice(&ENGINE_FLAGS);
        // The CPU's brand, in its 51-byte field: 50 at most, NUL-padded.
        put_cuts(&mut body, 25, 51, &self.cpu);
        body[76..80].clone_from_slice(&self.physical_mb.to_le_bytes());
        body[80..84].clone_from_slice(&self.virtual_mb.to_le_bytes());
        // The desktop, then the game screen, each with its colour depth,
        // and a word between the pairs that reads zero in both captures.
        for (at, value) in [
            (84, self.color_depth),
            (88, self.desktop_width),
            (92, self.desktop_height),
            (96, 0),
            (100, self.color_depth),
            (104, self.screen_width),
            (108, self.screen_height),
            (112, self.refresh_hz),
            (116, self.texture_memory),
        ] {
            body[at..at + 4].clone_from_slice(&value.to_le_bytes());
        }
        body[120..123].clone_from_slice(&self.formats);
        put_cut(&mut body, 123, 32, &self.adapter);
        put_cuts(&mut body, 155, 33, &self.device);
        body[188..196].clone_from_slice(&by_words(&OBSERVED_188));
        body[196..212].clone_from_slice(&by_words(&OS_VERSION));
        // The service-pack string, in its 128-byte field: 127 at most and
        // a NUL.
        put_cuts(&mut body, 216, 128, SERVICE_PACK);
        EncodedCommand {
            opcode: OPCODE,
            body,
        }
    }
}

impl Default for SysInfo {
    /// A report with nothing measured and nothing reported yet.
    fn default() -> Self {
        Self {
            cpu: String::new(),
            physical_mb: 0,
            virtual_mb: 0,
            color_depth: 0,
            desktop_width: 0,
            desktop_height: 0,
            screen_width: 0,
            screen_height: 0,
            refresh_hz: 0,
            texture_memory: 0,
            formats: [0; 3],
            adapter: String::new(),
            device: String::new(),
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
    put_cut(body, at, field - 1, text);
}

/// Writes a string into a fixed, zero-filled field of `room` bytes, cut to
/// fill it with no terminator where it is that long, never inside a
/// character.
fn put_cut(body: &mut [u8], at: usize, room: usize, text: &str) {
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
            cpu: "Test CPU Brand".into(),
            physical_mb: 1024,
            virtual_mb: 2048,
            color_depth: 32,
            desktop_width: 1920,
            desktop_height: 1080,
            screen_width: 1280,
            screen_height: 720,
            refresh_hz: 60,
            texture_memory: 0x3f60_0000,
            formats: [1, 0, 1],
            adapter: "Test Display Adapter".into(),
            device: "\\\\.\\DISPLAY1".into(),
            reported_at: 0,
        }
    }

    #[test]
    fn the_record_lays_every_field_where_the_captures_saw_it() {
        let packet = report().encode();
        assert_eq!(packet.opcode, 0x1c4c);
        assert_eq!(packet.body.len(), RECORD_BYTES);
        let word = |at: usize| u32::from_le_bytes(packet.body[at..at + 4].try_into().unwrap());
        assert_eq!(
            &packet.body[..20],
            &[1, 0, 0, 0, 6, 0, 0, 0, 7, 0, 0, 0, 1, 0, 0, 0, 0x10, 0x27, 0, 0]
        );
        assert_eq!(&packet.body[20..25], &[1, 1, 1, 0, 0]);
        assert!(packet.body[25..].starts_with(b"Test CPU Brand\0"));
        assert!(packet.body[39..76].iter().all(|&byte| byte == 0));
        assert_eq!(word(76), 1024);
        assert_eq!(word(80), 2048);
        assert_eq!(word(84), 32);
        assert_eq!(word(88), 1920);
        assert_eq!(word(92), 1080);
        assert_eq!(word(96), 0);
        assert_eq!(word(100), 32);
        assert_eq!(word(104), 1280);
        assert_eq!(word(108), 720);
        assert_eq!(word(112), 60);
        assert_eq!(word(116), 0x3f60_0000);
        assert_eq!(&packet.body[120..123], &[1, 0, 1]);
        assert!(packet.body[123..].starts_with(b"Test Display Adapter\0"));
        assert!(packet.body[155..].starts_with(b"\\\\.\\DISPLAY1\0"));
        assert_eq!(word(188), 0);
        assert_eq!(word(192), 0);
        assert_eq!(
            &packet.body[196..212],
            &[2, 0, 0, 0, 5, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0]
        );
        assert_eq!(word(212), 0);
        assert!(packet.body[216..].starts_with(b"Service Pack 2\0"));
        assert!(packet.body[230..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn long_strings_are_cut_to_their_fields_as_the_capture_shows() {
        let long = "x".repeat(200);
        let packet = SysInfo {
            cpu: long.clone(),
            adapter: long.clone(),
            device: long,
            ..report()
        }
        .encode();
        assert_eq!(packet.body[25..75], [b'x'; 50]);
        assert_eq!(packet.body[75], 0, "the CPU field holds 50 and a NUL");
        assert_eq!(
            packet.body[123..155],
            [b'x'; 32],
            "the adapter's description fills its 32 bytes with no NUL"
        );
        assert_eq!(packet.body[155..187], [b'x'; 32]);
        assert_eq!(packet.body[187], 0, "the device name holds 32 and a NUL");
        assert_eq!(packet.body[188], 0);
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
