//! Reliable UDP sessions and wire helpers used by EverQuest-compatible servers.

/// Encoding and decoding for SOE combined transport packets.
pub mod combined;
/// SOE keyed CRC helpers.
pub mod crc;
/// Transport codec errors.
pub mod error;
/// SOE application-fragment assembly and encoding.
pub mod fragment;
/// Reliable UDP used by the `EQMac` client generation.
pub mod legacy;
/// Reliable UDP used by later SOE client generations.
pub mod modern;
/// Primitive SOE session and transport packet codecs.
pub mod soe;

pub use combined::{build_combined, CombinedPacket, SubPacket};

/// What a game session needs from its connection to a server, whichever
/// client generation's transport carries it.
pub trait Transport {
    /// Sends one application packet the server must receive.
    ///
    /// # Errors
    /// Returns an error when the session is closed, the packet is too large
    /// or the socket fails.
    fn send(&mut self, opcode: u16, body: &[u8]) -> anyhow::Result<()>;

    /// Sends a packet the next one replaces, such as a position, without
    /// asking for an acknowledgement. A generation without unreliable
    /// delivery sends it reliably.
    ///
    /// # Errors
    /// Returns an error when the packet cannot be sent.
    fn send_unreliable(&mut self, opcode: u16, body: &[u8]) -> anyhow::Result<()> {
        self.send(opcode, body)
    }

    /// The next application packet from the server, if one is ready.
    ///
    /// # Errors
    /// Returns an error when the connection fails or a datagram is malformed.
    fn receive(&mut self) -> anyhow::Result<Option<Application>>;

    /// Closes the session, telling the server.
    ///
    /// # Errors
    /// Returns an error when the closing packet cannot be sent.
    fn close(&mut self) -> anyhow::Result<()>;

    /// Whole seconds since the server last sent anything.
    fn last_received_seconds(&self) -> u64;
}

impl Transport for modern::Session {
    fn send(&mut self, opcode: u16, body: &[u8]) -> anyhow::Result<()> {
        Self::send(self, opcode, body)
    }

    fn send_unreliable(&mut self, opcode: u16, body: &[u8]) -> anyhow::Result<()> {
        Self::send_unreliable(self, opcode, body)
    }

    fn receive(&mut self) -> anyhow::Result<Option<Application>> {
        Self::receive(self)
    }

    fn close(&mut self) -> anyhow::Result<()> {
        Self::close(self)
    }

    fn last_received_seconds(&self) -> u64 {
        Self::last_received_seconds(self)
    }
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn send(&mut self, opcode: u16, body: &[u8]) -> anyhow::Result<()> {
        (**self).send(opcode, body)
    }

    fn send_unreliable(&mut self, opcode: u16, body: &[u8]) -> anyhow::Result<()> {
        (**self).send_unreliable(opcode, body)
    }

    fn receive(&mut self) -> anyhow::Result<Option<Application>> {
        (**self).receive()
    }

    fn close(&mut self) -> anyhow::Result<()> {
        (**self).close()
    }

    fn last_received_seconds(&self) -> u64 {
        (**self).last_received_seconds()
    }
}

/// `EQMac` has no unreliable channel, so positions travel reliably.
impl Transport for legacy::OldSession {
    fn send(&mut self, opcode: u16, body: &[u8]) -> anyhow::Result<()> {
        Self::send(self, opcode, body)
    }

    fn receive(&mut self) -> anyhow::Result<Option<Application>> {
        Self::receive(self)
    }

    fn close(&mut self) -> anyhow::Result<()> {
        Self::close(self)
    }

    fn last_received_seconds(&self) -> u64 {
        Self::last_received_seconds(self)
    }
}
pub use fragment::FragmentAssembler;
pub use modern::Application;
pub use soe::{
    build_ack, build_disconnect, build_keepalive, build_session_request, build_session_response,
    get_sequence, set_sequence, transport_opcode, wrap_app_packet, SessionResponse, TransportOp,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// A transport that remembers what it was asked to send.
    #[derive(Default)]
    struct Recorder(Vec<u16>);

    impl Transport for Recorder {
        fn send(&mut self, opcode: u16, _body: &[u8]) -> anyhow::Result<()> {
            self.0.push(opcode);
            Ok(())
        }

        fn receive(&mut self) -> anyhow::Result<Option<Application>> {
            Ok(None)
        }

        fn close(&mut self) -> anyhow::Result<()> {
            Ok(())
        }

        fn last_received_seconds(&self) -> u64 {
            0
        }
    }

    #[test]
    fn a_generation_without_unreliable_delivery_sends_reliably_and_boxes_pass_through() {
        let mut boxed: Box<dyn Transport> = Box::new(Recorder::default());
        boxed.send_unreliable(0x14cb, &[]).unwrap();
        boxed.send(0x7752, &[]).unwrap();
        assert!(boxed.receive().unwrap().is_none());
        let mut recorder = Recorder::default();
        recorder.send_unreliable(0x14cb, &[]).unwrap();
        assert_eq!(recorder.0, [0x14cb]);
    }
}
