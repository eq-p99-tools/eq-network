//! `RoF2`'s chat as stock `EQEmu`'s encoders write it (6.521). Bodies come
//! from our own writers; names are synthetic.
use super::*;

const RO: GameDialect = GameDialect::Rof2;
const TI: GameDialect = GameDialect::Titanium;

/// A `RoF2` channel message: the sender, the target, the encoder's zero,
/// the language, the channel, a zero word and byte, the skill, the text and
/// the 15-byte tail.
fn channel_body(
    sender: &[u8],
    target: &[u8],
    language: u32,
    channel: u32,
    skill: u32,
    text: &[u8],
) -> Vec<u8> {
    let mut body = [sender, b"\0", target, b"\0"].concat();
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&language.to_le_bytes());
    body.extend_from_slice(&channel.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.push(0);
    body.extend_from_slice(&skill.to_le_bytes());
    body.extend_from_slice(text);
    body.push(0);
    body.extend_from_slice(&[0; 15]);
    body
}

/// A link with `body_size` hex characters naming item 0x1A2B3 and labelled
/// "Short Sword".
fn link(body_size: usize) -> Vec<u8> {
    let mut digits = b"01A2B3".to_vec();
    digits.resize(body_size, b'0');
    [b"\x12".as_slice(), &digits, b"Short Sword\x12"].concat()
}

fn read(opcode: u16, body: &[u8]) -> ChatEvent {
    parse_for(RO, opcode, body, false).unwrap().unwrap()
}

#[test]
fn a_channel_message_reads_each_field_from_its_place() {
    let event = read(
        0x2b2d,
        &channel_body(b"Speaker", b"Hearer", 4, 7, 60, b"well met"),
    );
    assert_eq!(event.channel_name, ChannelName::Tell);
    assert_eq!(event.channel, Some(7));
    assert_eq!(event.sender.as_deref(), Some("Speaker"));
    assert_eq!(event.target.as_deref(), Some("Hearer"));
    assert_eq!((event.language, event.language_skill), (Some(4), Some(60)));
    assert_eq!(event.message.unwrap().text, "well met");
    // No target, the echo of the player's own tell, and the largest names.
    let long = [b'a'; 63];
    let event = read(0x2b2d, &channel_body(&long, b"", 0, 14, 100, b"x"));
    assert_eq!(event.channel_name, ChannelName::TellEcho);
    assert_eq!(event.target, None);
    assert_eq!(event.sender.unwrap().len(), 63);
    let event = read(0x2b2d, &channel_body(b"", &long, 0, 8, 100, b""));
    assert_eq!(event.channel_name, ChannelName::Say);
    assert_eq!(event.sender.as_deref(), Some(""));
    // A language or skill past 255 reads 255, as Titanium's.
    let event = read(0x2b2d, &channel_body(b"S", b"", 300, 8, 70_000, b"x"));
    assert_eq!(
        (event.language, event.language_skill),
        (Some(255), Some(255))
    );
}

#[test]
fn a_channel_message_refuses_what_the_encoder_cannot_write() {
    let body = channel_body(b"Speaker", b"Hearer", 0, 8, 100, b"hello");
    for end in 0..body.len() {
        assert!(
            parse_for(RO, 0x2b2d, &body[..end], false).is_err(),
            "cut at {end}"
        );
    }
    assert!(parse_for(RO, 0x2b2d, &[body.clone(), vec![0]].concat(), false).is_err());
    // A name past the server's 64-byte field.
    let long = [b'a'; 64];
    assert!(parse_for(
        RO,
        0x2b2d,
        &channel_body(&long, b"", 0, 8, 100, b"x"),
        false
    )
    .is_err());
    assert!(parse_for(
        RO,
        0x2b2d,
        &channel_body(b"S", &long, 0, 8, 100, b"x"),
        false
    )
    .is_err());
    // An unterminated text has no tail after it.
    let mut open = body[..body.len() - 16].to_vec();
    open.extend_from_slice(&[b'x'; 16]);
    assert!(parse_for(RO, 0x2b2d, &open, false).is_err());
    // The encoder writes zeros in its tail; a nonzero byte there is ignored.
    let mut tail = body.clone();
    *tail.last_mut().unwrap() = 0xa5;
    assert_eq!(read(0x2b2d, &tail).message.unwrap().text, "hello");
}

#[test]
fn links_are_56_characters_on_rof2_and_45_on_titanium() {
    let rof2 = read(
        0x2b2d,
        &channel_body(
            b"S",
            b"",
            0,
            8,
            100,
            &[b"see ".as_slice(), &link(56)].concat(),
        ),
    );
    let message = rof2.message.unwrap();
    assert_eq!(message.text, "see Short Sword");
    assert_eq!(message.item_links.len(), 1);
    assert_eq!(message.item_links[0].item_id, 0x1a2b3);
    // A Titanium-sized body is not a link to RoF2: its first 56 characters
    // run into the label, which is not hex.
    let titanium_link = read(0x2b2d, &channel_body(b"S", b"", 0, 8, 100, &link(45)));
    assert!(titanium_link.message.unwrap().item_links.is_empty());
    // Titanium's reader, unchanged, takes the first 45 of RoF2's 56 and
    // leaves the other 11 digits in the label, as Titanium's own body
    // check has always done; it never reads RoF2's link.
    let mut titanium = vec![0; 148];
    titanium.extend_from_slice(&link(56));
    titanium.push(0);
    let event = parse_for(TI, 0x1004, &titanium, false).unwrap().unwrap();
    let links = event.message.unwrap().item_links;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].body.len(), 45);
    assert_ne!(links[0].text, "Short Sword");
}

#[test]
fn the_other_lines_read_as_titaniums_under_rof2s_numbers() {
    let text = |bytes: &[u8]| [bytes, b"\0"].concat();
    let mut special = vec![1, 0, 0];
    special.extend_from_slice(&256u32.to_le_bytes());
    special.extend_from_slice(&73u32.to_le_bytes());
    special.extend_from_slice(b"Guard\0");
    special.extend_from_slice(&[0; 12]);
    special.extend_from_slice(b"Move along\0");
    let mut formatted = [0u32, 1032, 15].map(u32::to_le_bytes).concat();
    formatted.extend_from_slice(b"Guard\0ten\0\0");
    let simple = [338u32, 13, 0].map(u32::to_le_bytes).concat();
    let emote = [0u32.to_le_bytes().as_slice(), b"Example waves\0"].concat();
    let colored = [15u32.to_le_bytes().as_slice(), b"Colored\0"].concat();
    let mut guild = vec![0; 136];
    guild[4..9].copy_from_slice(b"Owner");
    guild[68..74].copy_from_slice(b"Setter");
    guild.extend_from_slice(b"Be kind\0");
    for (rof2, titanium, body) in [
        (0x0c22, 0x024d, text(b"Welcome")),
        (0x373b, 0x547a, emote),
        (0x0083, 0x2372, special),
        (0x1024, 0x5a48, formatted),
        (0x213f, 0x673c, simple),
        (0x3e13, 0x475a, guild.clone()),
        (0x4f1f, crate::guild::MOTD_REPLY_OPCODE, guild),
        (0x43af, 0x0b2d, colored),
    ] {
        let ours = parse_for(RO, rof2, &body, true).unwrap().unwrap();
        let theirs = parse_for(TI, titanium, &body, true).unwrap().unwrap();
        // The same event but for its number.
        let mut json = serde_json::to_value(&ours).unwrap();
        json["opcode"] = serde_json::json!(titanium);
        assert_eq!(json, serde_json::to_value(&theirs).unwrap(), "{rof2:#06x}");
        // And the same refusals of a cut body.
        for end in 0..body.len() {
            assert_eq!(
                parse_for(RO, rof2, &body[..end], false).is_err(),
                parse_for(TI, titanium, &body[..end], false).is_err(),
                "{rof2:#06x} cut at {end}"
            );
        }
    }
}

#[test]
fn no_generations_number_reads_under_the_other() {
    for opcode in ROF2_READS {
        assert!(
            parse_for(TI, opcode, &[0; 200], false).unwrap().is_none(),
            "{opcode:#06x}"
        );
        assert!(parse_for(GameDialect::EqMac, opcode, &[0; 200], false)
            .unwrap()
            .is_none());
    }
    for opcode in [
        0x024d,
        0x1004,
        0x547a,
        0x2372,
        0x5a48,
        0x673c,
        0x475a,
        0x0b2d,
        crate::guild::MOTD_REPLY_OPCODE,
    ] {
        assert!(
            parse_for(RO, opcode, &[0; 200], false).unwrap().is_none(),
            "{opcode:#06x}"
        );
    }
    for opcode in 0..=u16::MAX {
        let read = parse_for(RO, opcode, &[], false).map_or(true, |event| event.is_some());
        assert_eq!(read, ROF2_READS.contains(&opcode), "{opcode:#06x}");
    }
}

#[test]
fn every_outbound_form_still_refuses_rof2() {
    for message in [
        OutboundChat::Guild("x".into()),
        OutboundChat::Group("x".into()),
        OutboundChat::Shout("x".into()),
        OutboundChat::Auction("x".into()),
        OutboundChat::Ooc("x".into()),
        OutboundChat::Tell {
            recipient: "Hearer".into(),
            message: "x".into(),
        },
        OutboundChat::Say("x".into()),
        OutboundChat::Raid("x".into()),
    ] {
        assert!(encode_outbound_for(RO, &message, "Speaker").is_err());
        assert!(encode_outbound_in(RO, &message, "Speaker", Tongue::COMMON).is_err());
    }
}
