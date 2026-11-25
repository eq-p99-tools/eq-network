//! Request-driven installed-file measurements. A cached provider reuses a
//! size and modification-time cache on every server, P99 included; a refusal
//! invalidates the refused operands and the next login rereads them.
use super::{directory_entries, valid_asset_name, Assets, FileChecksum};
use crate::client::CancellationToken;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc, Mutex,
    },
    time::{Instant, UNIX_EPOCH},
};

/// Credential-free progress for a server-requested file check.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct AssetProgress {
    /// Files measured or satisfied by a permitted local cache hit.
    pub completed: usize,
    /// Unique requested operands, including the spell operand when required.
    pub total: usize,
    /// File-content bytes read for CRC computation on this request.
    pub bytes: u64,
    /// Operands answered from the size and modification-time cache.
    pub cache_hits: usize,
    /// Monotonic elapsed milliseconds for this request.
    pub elapsed_ms: u64,
}

#[derive(Default)]
struct Counters {
    completed: AtomicUsize,
    bytes: AtomicU64,
    cache_hits: AtomicUsize,
}

/// A pending file measurement. Dropping it cancels work between read blocks.
pub struct AssetJob {
    received: Receiver<Result<Assets>>,
    cancel: Arc<AtomicBool>,
    counters: Arc<Counters>,
    total: usize,
    began: Instant,
}

impl AssetJob {
    #[cfg(test)]
    pub(crate) fn controlled(total: usize) -> (Self, mpsc::Sender<Result<Assets>>) {
        let (sent, received) = mpsc::channel();
        (
            Self {
                received,
                cancel: Arc::default(),
                counters: Arc::default(),
                total,
                began: Instant::now(),
            },
            sent,
        )
    }
    /// Poll without holding up the network receive loop.
    #[must_use]
    pub fn poll(&self) -> Option<Result<Assets>> {
        match self.received.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(anyhow::anyhow!(
                "Client file measurement ended without a result"
            ))),
        }
    }

    /// Current numeric progress; no filenames, keys or file contents.
    #[must_use]
    pub fn progress(&self) -> AssetProgress {
        AssetProgress {
            completed: self.counters.completed.load(Ordering::Relaxed),
            total: self.total,
            bytes: self.counters.bytes.load(Ordering::Relaxed),
            cache_hits: self.counters.cache_hits.load(Ordering::Relaxed),
            elapsed_ms: u64::try_from(self.began.elapsed().as_millis()).unwrap_or(u64::MAX),
        }
    }
}

impl Drop for AssetJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Supplies only the operands requested by a real integrity challenge.
pub trait AssetSource: Send + Sync {
    /// Starts cancellable measurement. Names must be validated filenames.
    ///
    /// # Errors
    /// Refuses unsafe names or inability to start the measurement worker.
    fn measure(&self, names: &[String], stop: &CancellationToken) -> Result<AssetJob>;

    /// Invalidates the operands of a refused challenge, never unrelated files.
    ///
    /// # Errors
    /// Returns an error if persistent invalidation could not be saved.
    fn refused(&self, _names: &[String]) -> Result<()> {
        Ok(())
    }
}

impl AssetSource for Assets {
    fn measure(&self, names: &[String], stop: &CancellationToken) -> Result<AssetJob> {
        ensure!(!stop.is_cancelled(), "shutdown requested");
        let (sent, received) = mpsc::channel();
        sent.send(Ok(self.clone()))?;
        Ok(AssetJob {
            received,
            cancel: Arc::default(),
            counters: Arc::new(Counters {
                completed: AtomicUsize::new(names.len()),
                ..Counters::default()
            }),
            total: names.len(),
            began: Instant::now(),
        })
    }
}

/// The settings-directory file holding the size and modification-time cache.
/// Deleting it by hand clears the cache; the next check rereads every operand.
pub const CHECKSUM_CACHE_FILE: &str = "asset-checksums-v1.json";

/// An installation whose contents are read only when the server requests them.
/// Construction performs no filesystem access.
///
/// A cached provider answers an operand from its cache when the file's
/// relative path, size and modification time all match the cached entry. It
/// cannot see a same-size edit whose modification time was restored; a server
/// refusal invalidates that challenge's operands, so the next login rereads
/// them. If that invalidation cannot be saved, the provider deletes the cache
/// file (best effort) and stops trusting metadata for the rest of its life.
#[derive(Clone)]
pub struct InstalledAssets {
    directory: PathBuf,
    cache: Option<PathBuf>,
    // Shared by clones, so a failed invalidation disables every later reuse.
    trust_metadata: Arc<AtomicBool>,
    suspect: Arc<Mutex<BTreeSet<String>>>,
}

impl InstalledAssets {
    /// Creates a provider which freshly measures every requested operand.
    #[must_use]
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            cache: None,
            trust_metadata: Arc::new(AtomicBool::new(false)),
            suspect: Arc::default(),
        }
    }

    /// Creates a provider which reuses the size and modification-time cache in
    /// `settings` ([`CHECKSUM_CACHE_FILE`]) on every server. Without a settings
    /// directory nothing persists and every operand is read.
    pub(crate) fn cached(directory: PathBuf, settings: Option<&Path>) -> Self {
        Self {
            cache: settings.map(|path| path.join(CHECKSUM_CACHE_FILE)),
            trust_metadata: Arc::new(AtomicBool::new(true)),
            ..Self::new(directory)
        }
    }

    fn trusts_metadata(&self) -> bool {
        self.trust_metadata.load(Ordering::Acquire)
    }

    #[allow(clippy::too_many_lines)] // Keep one request's measurement and cache settlement ordered.
    fn scan(
        &self,
        names: &[String],
        stop: &CancellationToken,
        cancel: &AtomicBool,
        counters: &Counters,
    ) -> Result<Assets> {
        let check = || -> Result<()> {
            ensure!(
                !stop.is_cancelled() && !cancel.load(Ordering::Relaxed),
                "Client file measurement cancelled"
            );
            Ok(())
        };
        check()?;
        let root = fs::canonicalize(&self.directory).context("read client installation")?;
        let installed = directory_entries(&root)?;
        let resources = if names.iter().any(|name| name == "globalload.txt") {
            installed
                .get("resources")
                .map(|path| directory_entries(path))
                .transpose()?
        } else {
            None
        };
        let mut cache = Cache::load(self.cache.as_deref(), &root);
        let mut files = BTreeMap::new();
        for name in names {
            check()?;
            let path = if name == "globalload.txt" {
                resources.as_ref().and_then(|entries| entries.get(name))
            } else {
                installed.get(name)
            };
            if let Some(path) = path {
                let relative = path.strip_prefix(&root)?.to_path_buf();
                let mut file = fs::File::open(path).context("open requested client file")?;
                let before = Stamp::read(&file.metadata()?)?;
                let suspect = self
                    .suspect
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Checksum cache unavailable"))?
                    .contains(name);
                let hit = self.trusts_metadata()
                    && !suspect
                    && cache.files.get(name).is_some_and(|entry| {
                        entry.relative == relative
                            && entry.stamp == before
                            && entry.checksum.size == before.size
                    });
                let checksum = if hit {
                    counters.cache_hits.fetch_add(1, Ordering::Relaxed);
                    cache.files[name].checksum.clone()
                } else {
                    let mut crc = crc32fast::Hasher::new();
                    let mut buffer = vec![0; 65_536];
                    let mut bytes = 0_u64;
                    loop {
                        check()?;
                        let count = file
                            .read(&mut buffer)
                            .context("read requested client file")?;
                        if count == 0 {
                            break;
                        }
                        crc.update(&buffer[..count]);
                        bytes += u64::try_from(count)?;
                        counters
                            .bytes
                            .fetch_add(u64::try_from(count)?, Ordering::Relaxed);
                    }
                    // No retry loop: changed input is discarded before any answer.
                    ensure!(
                        bytes == before.size
                            && Stamp::read(&file.metadata()?)? == before
                            && Stamp::read(&fs::metadata(path)?)? == before,
                        "Client file changed while measuring; request a fresh login"
                    );
                    FileChecksum {
                        crc32: crc.finalize(),
                        size: bytes,
                    }
                };
                check()?;
                ensure!(
                    Stamp::read(&file.metadata()?)? == before
                        && Stamp::read(&fs::metadata(path)?)? == before,
                    "Client file changed while checking; request a fresh login"
                );
                cache.files.insert(
                    name.clone(),
                    Entry {
                        relative,
                        stamp: before,
                        checksum: checksum.clone(),
                    },
                );
                files.insert(name.clone(), Some(checksum));
            } else {
                cache.files.remove(name);
            }
            counters.completed.fetch_add(1, Ordering::Relaxed);
        }
        check()?;
        // A provider that lost trust never rewrites the store it deleted.
        if self.trusts_metadata() {
            // A cache write failure must not replace a correctly measured answer.
            let _ = cache.save(self.cache.as_deref());
            let mut suspect = self
                .suspect
                .lock()
                .map_err(|_| anyhow::anyhow!("Checksum cache unavailable"))?;
            for name in names {
                suspect.remove(name);
            }
        }
        Ok(Assets {
            client: "Installed Titanium".into(),
            files,
        })
    }
}

impl AssetSource for InstalledAssets {
    fn measure(&self, names: &[String], stop: &CancellationToken) -> Result<AssetJob> {
        ensure!(
            names.iter().all(|name| valid_asset_name(name)),
            "asset inventory entries must be filenames"
        );
        let names: Vec<_> = names
            .iter()
            .map(|name| name.to_ascii_lowercase())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if names.is_empty() {
            return Assets {
                client: "Installed Titanium".into(),
                files: BTreeMap::new(),
            }
            .measure(&[], stop);
        }
        let (sent, received) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let counters = Arc::new(Counters::default());
        let provider = self.clone();
        let stop = stop.clone();
        let worker_cancel = cancel.clone();
        let worker_counts = counters.clone();
        let total = names.len();
        std::thread::Builder::new()
            .name("client file checks".into())
            .spawn(move || {
                let result = provider.scan(&names, &stop, &worker_cancel, &worker_counts);
                let _ = sent.send(result);
            })?;
        Ok(AssetJob {
            received,
            cancel,
            counters,
            total,
            began: Instant::now(),
        })
    }

    fn refused(&self, names: &[String]) -> Result<()> {
        if !self.trusts_metadata() {
            return Ok(());
        }
        let names: BTreeSet<_> = names.iter().map(|name| name.to_ascii_lowercase()).collect();
        let saved = (|| {
            self.suspect
                .lock()
                .map_err(|_| anyhow::anyhow!("Checksum cache unavailable"))?
                .extend(names.iter().cloned());
            let root = fs::canonicalize(&self.directory)?;
            let mut cache = Cache::load(self.cache.as_deref(), &root);
            for name in &names {
                cache.files.remove(name);
            }
            cache.save(self.cache.as_deref())
        })();
        if saved.is_err() {
            // An unsaved invalidation must not leave a stale entry trusted:
            // stop reusing metadata here and drop the store for later providers.
            self.trust_metadata.store(false, Ordering::Release);
            if let Some(path) = &self.cache {
                let _ = fs::remove_file(path);
            }
        }
        saved
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Stamp {
    size: u64,
    seconds: u64,
    nanos: u32,
}

impl Stamp {
    fn read(metadata: &fs::Metadata) -> Result<Self> {
        let modified = metadata.modified()?.duration_since(UNIX_EPOCH)?;
        ensure!(metadata.is_file(), "Requested client asset is not a file");
        Ok(Self {
            size: metadata.len(),
            seconds: modified.as_secs(),
            nanos: modified.subsec_nanos(),
        })
    }
}

#[derive(Serialize, Deserialize)]
struct Entry {
    relative: PathBuf,
    stamp: Stamp,
    checksum: FileChecksum,
}

#[derive(Serialize, Deserialize)]
struct Cache {
    version: u8,
    root: PathBuf,
    files: BTreeMap<String, Entry>,
}

impl Cache {
    fn load(path: Option<&Path>, root: &Path) -> Self {
        path.and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .filter(|cache| cache.version == 1 && cache.root == root)
            .unwrap_or_else(|| Self {
                version: 1,
                root: root.to_path_buf(),
                files: BTreeMap::new(),
            })
    }

    fn save(&self, path: Option<&Path>) -> Result<()> {
        let Some(path) = path else {
            return Ok(());
        };
        let parent = path
            .parent()
            .context("Checksum cache directory is unavailable")?;
        #[cfg(test)]
        ensure!(
            !tests::UNWRITABLE.with(std::cell::Cell::get),
            "Checksum cache store is unwritable"
        );
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(
            ".asset-checksums-{:016x}.tmp",
            rand::random::<u64>()
        ));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&serde_json::to_vec(self)?)?;
            file.sync_all()?;
            fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

#[cfg(test)]
pub(crate) mod tests;
