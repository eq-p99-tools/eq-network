//! Wait for requested file measurements while continuing transport service.
use crate::{
    assets::{parse_manifest, AssetSource, Assets},
    client::{CancellationToken, ClientEvent, Events},
};
use anyhow::{ensure, Result};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

pub(super) fn service(
    session: &mut dyn eq_network_transport::Transport,
    packets: &mut std::collections::VecDeque<crate::transport::Application>,
    deadline: Instant,
) -> Result<()> {
    ensure!(Instant::now() < deadline, "world handshake timed out");
    if let Some(packet) = session.receive()? {
        ensure!(
            packets.len() < 1024
                && packets
                    .iter()
                    .map(|packet| packet.body.len())
                    .sum::<usize>()
                    + packet.body.len()
                    <= 8 * 1024 * 1024,
            "Too much world data arrived during client file checking"
        );
        packets.push_back(packet);
    }
    Ok(())
}

pub(super) fn operands(manifest: &[u8], spells: bool) -> Result<Vec<String>> {
    let mut names: BTreeSet<_> = parse_manifest(manifest)?
        .into_iter()
        .filter(|entry| !entry.is_skipped())
        .map(|entry| entry.name.to_ascii_lowercase())
        .collect();
    if spells {
        names.insert("spells_us.txt".into());
    }
    Ok(names.into_iter().collect())
}

pub(super) fn measure(
    source: &dyn AssetSource,
    manifest: &[u8],
    spells: bool,
    stop: &CancellationToken,
    mut service: impl FnMut() -> Result<()>,
    log: &mut Events<'_>,
) -> Result<(Assets, Vec<String>)> {
    let names = operands(manifest, spells)?;
    let job = source.measure(&names, stop)?;
    let result = (|| {
        let mut next_progress = Instant::now();
        loop {
            ensure!(!stop.is_cancelled(), "shutdown requested");
            if let Some(result) = job.poll() {
                log.send(ClientEvent::FilesChecking(Some(job.progress())))?;
                return result;
            }
            if Instant::now() >= next_progress {
                log.send(ClientEvent::FilesChecking(Some(job.progress())))?;
                next_progress = Instant::now() + Duration::from_millis(250);
            }
            service()?;
        }
    })();
    // Preserve the measurement/transport error if presentation also fails.
    let cleared = log.send(ClientEvent::FilesChecking(None));
    let assets = result?;
    cleared?;
    Ok((assets, names))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{assets::AssetJob, client::ClientConfig};
    use std::sync::{mpsc::Sender, Mutex};

    struct Delayed(Mutex<Option<Sender<Result<Assets>>>>);
    impl AssetSource for Delayed {
        fn measure(&self, names: &[String], _stop: &CancellationToken) -> Result<AssetJob> {
            let (job, sent) = AssetJob::controlled(names.len());
            *self.0.lock().unwrap() = Some(sent);
            Ok(job)
        }
    }

    #[test]
    fn a_pending_hash_keeps_servicing_transport_and_clears_progress() {
        let source = Delayed(Mutex::default());
        let config = ClientConfig::new("SYNTHETIC", "SYNTHETIC", "", "");
        let mut events = Vec::new();
        let mut handler = |event| {
            events.push(event);
            Ok(())
        };
        let mut log = Events::new(&config, &mut handler);
        let mut serviced = 0;
        let (_, names) = measure(
            &source,
            b"\x01\x00\x01one.s3d\0\x02\x00\x02skip.eqg\0",
            true,
            &CancellationToken::default(),
            || {
                serviced += 1;
                source.0.lock().unwrap().take().unwrap().send(Ok(Assets {
                    client: "test".into(),
                    files: std::collections::BTreeMap::new(),
                }))?;
                Ok(())
            },
            &mut log,
        )
        .unwrap();
        assert_eq!(serviced, 1);
        assert_eq!(names, ["one.s3d", "spells_us.txt"]);
        assert!(matches!(
            events.first(),
            Some(ClientEvent::FilesChecking(Some(_)))
        ));
        assert!(matches!(
            events.last(),
            Some(ClientEvent::FilesChecking(None))
        ));
    }
}
