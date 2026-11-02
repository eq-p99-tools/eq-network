//! Titanium worlds default a just-created session to the tutorial. Its first
//! entry chooses the tutorial for that character, or explicitly clears it.
use anyhow::Result;
use eq_network_transport::Transport;

#[derive(Default)]
pub(in crate::client::session) struct StartChoice(Option<(String, bool)>);

impl StartChoice {
    pub fn created(&mut self, offered: bool, choice: (String, bool)) {
        self.0 = offered.then_some(choice);
    }

    pub fn enter(&mut self, name: &str, session: &mut dyn Transport) -> Result<()> {
        if self
            .0
            .take()
            .is_some_and(|(created, tutorial)| !tutorial || !created.eq_ignore_ascii_case(name))
        {
            // OP_World_Client_CRC1 clears EQEmu's StartInTutorial flag. This
            // policy uses the stock world setting without checksum validation.
            session.send(0x5072, &[0; 2056])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Sent(Vec<(u16, Vec<u8>)>);
    impl Transport for Sent {
        fn send(&mut self, opcode: u16, body: &[u8]) -> Result<()> {
            self.0.push((opcode, body.into()));
            Ok(())
        }
        fn receive(&mut self) -> Result<Option<crate::transport::Application>> {
            Ok(None)
        }
        fn close(&mut self) -> Result<()> {
            Ok(())
        }
        fn last_received_seconds(&self) -> u64 {
            0
        }
    }

    #[test]
    fn tutorial_choice_belongs_only_to_the_created_character_and_first_entry() {
        for (offered, tutorial, selected, packets) in [
            (true, false, "Testhero", 1),
            (true, true, "testhero", 0),
            (true, true, "Anotherhero", 1),
            (false, true, "Testhero", 0),
            (false, false, "Testhero", 0),
        ] {
            let mut start = StartChoice::default();
            let mut wire = Sent::default();
            start.created(offered, ("Testhero".into(), tutorial));
            start.enter(selected, &mut wire).unwrap();
            start.enter(selected, &mut wire).unwrap();
            assert_eq!(wire.0.len(), packets);
            assert!(wire
                .0
                .iter()
                .all(|(op, body)| *op == 0x5072 && body == &[0; 2056]));
        }
    }
}
