use super::*;
use std::time::Duration;

struct Installation(PathBuf);
impl Installation {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("requested-assets-{:016x}", rand::random::<u64>()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn write(&self, name: &str, bytes: &[u8]) {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fn local(&self) -> InstalledAssets {
        InstalledAssets::cached(self.0.clone(), Some(&self.0.join("settings")))
    }
}
impl Drop for Installation {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn measured(source: &dyn AssetSource, names: &[&str]) -> (Assets, AssetProgress) {
    let job = source
        .measure(
            &names.iter().map(|name| (*name).into()).collect::<Vec<_>>(),
            &CancellationToken::default(),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(result) = job.poll() {
            return (result.unwrap(), job.progress());
        }
        assert!(
            Instant::now() < deadline,
            "offline measurement did not finish"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn construction_defers_every_installation_read_until_a_request() {
    let provider = InstalledAssets::new(PathBuf::from("missing-requested-installation"));
    assert!(!provider.directory.exists());
    let (empty, progress) = measured(&provider, &[]);
    assert!(empty.files.is_empty());
    assert_eq!((progress.bytes, progress.total), (0, 0));
    let job = provider
        .measure(&["spells_us.txt".into()], &CancellationToken::default())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(result) = job.poll() {
            assert!(result.is_err());
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
}

std::thread_local! {
    /// Makes this thread's cache saves fail, as an unwritable store would.
    /// Measurement saves run on their worker thread and are unaffected.
    pub(super) static UNWRITABLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A provider factory: installation directory, then settings directory.
pub(crate) type Provider<'a> = &'a dyn Fn(&Path, &Path) -> Box<dyn AssetSource>;

fn set_modified(path: &Path, modified: std::time::SystemTime) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
}

/// The cold, warm, changed and refused rows, plus the documented limitation
/// (a same-size edit with a restored date is a cache hit until refused). Each
/// call of `provider` is a fresh provider, as a new login builds one.
pub(crate) fn cache_rows(provider: Provider) {
    let install = Installation::new();
    let settings = install.0.join("settings");
    let fresh = || provider(&install.0, &settings);
    install.write("first.s3d", b"123456789");
    install.write("second.eqg", b"second");
    install.write("unrequested.s3d", b"never measured");
    let same = fresh();
    // Cold: only the requested files are read.
    let (cold, progress) = measured(&*same, &["FIRST.S3D", "second.eqg"]);
    assert_eq!(
        cold.files["first.s3d"],
        Some(FileChecksum {
            crc32: 0xcbf4_3926,
            size: 9
        })
    );
    assert_eq!((progress.bytes, progress.cache_hits), (15, 0));
    assert!(!cold.files.contains_key("unrequested.s3d"));
    assert!(settings.join(CHECKSUM_CACHE_FILE).exists());
    // Warm: a new provider answers both from the store.
    let (warm, progress) = measured(&*fresh(), &["first.s3d", "second.eqg"]);
    assert_eq!(warm.files, cold.files);
    assert_eq!((progress.bytes, progress.cache_hits), (0, 2));
    // Changed size, then changed date with the same size: both reread.
    install.write("first.s3d", b"changed size");
    let (_, progress) = measured(&*same, &["first.s3d", "second.eqg"]);
    assert_eq!((progress.bytes, progress.cache_hits), (12, 1));
    let path = install.0.join("first.s3d");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    install.write("first.s3d", b"new contents");
    set_modified(&path, modified + Duration::from_secs(2));
    let (changed, progress) = measured(&*fresh(), &["first.s3d", "second.eqg"]);
    assert_eq!((progress.bytes, progress.cache_hits), (12, 1));
    assert_eq!(
        changed.files["first.s3d"].as_ref().unwrap().crc32,
        crc32fast::hash(b"new contents")
    );
    // Refused: only that challenge's operands are reread by the next login.
    same.refused(&["FIRST.S3D".into()]).unwrap();
    let (_, progress) = measured(&*fresh(), &["first.s3d", "second.eqg"]);
    assert_eq!((progress.bytes, progress.cache_hits), (12, 1));

    // Documented limitation: a same-size edit whose date is restored is
    // answered from the cache; the refusal path is its recovery.
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let (before, _) = measured(&*fresh(), &["first.s3d"]);
    install.write("first.s3d", b"edited bytes");
    set_modified(&path, modified);
    let edited = fresh();
    let (stale, progress) = measured(&*edited, &["first.s3d"]);
    assert_eq!(stale.files, before.files);
    assert_eq!((progress.bytes, progress.cache_hits), (0, 1));
    edited.refused(&["first.s3d".into()]).unwrap();
    let (reread, progress) = measured(&*fresh(), &["first.s3d"]);
    assert_eq!((progress.bytes, progress.cache_hits), (12, 0));
    assert_eq!(
        reread.files["first.s3d"].as_ref().unwrap().crc32,
        crc32fast::hash(b"edited bytes")
    );
}

#[test]
fn cached_cold_warm_changed_and_refused_checks_are_limited_to_requested_files() {
    cache_rows(&|directory, settings| {
        Box::new(InstalledAssets::cached(directory.into(), Some(settings)))
    });
}

#[test]
fn fresh_provider_reads_same_size_edits_even_with_restored_mtime() {
    let install = Installation::new();
    install.write("first.s3d", b"before");
    let (original, _) = measured(&install.local(), &["first.s3d"]);
    let path = install.0.join("first.s3d");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    install.write("first.s3d", b"after!");
    set_modified(&path, modified);
    let verified = InstalledAssets::new(install.0.clone());
    let (current, progress) = measured(&verified, &["first.s3d"]);
    assert_ne!(current.files, original.files);
    assert_eq!(
        current.files["first.s3d"].as_ref().unwrap().crc32,
        crc32fast::hash(b"after!")
    );
    assert_eq!((progress.bytes, progress.cache_hits), (6, 0));
}

#[test]
fn unsaved_invalidation_deletes_the_store_and_ends_metadata_trust() {
    let install = Installation::new();
    install.write("first.s3d", b"bytes");
    let store = install.0.join("settings").join(CHECKSUM_CACHE_FILE);
    let provider = install.local();
    assert_eq!(measured(&provider, &["first.s3d"]).1.bytes, 5);
    assert!(store.exists());
    UNWRITABLE.with(|unwritable| unwritable.set(true));
    let refused = provider.refused(&["first.s3d".into()]);
    UNWRITABLE.with(|unwritable| unwritable.set(false));
    assert!(refused.is_err(), "the diagnostic is kept");
    assert!(!store.exists(), "the store is deleted best effort");
    // The same provider, and its clones, now read the bytes every time and
    // never write the store again.
    for source in [provider.clone(), provider] {
        let (_, progress) = measured(&source, &["first.s3d"]);
        assert_eq!((progress.bytes, progress.cache_hits), (5, 0));
        assert!(source.refused(&["first.s3d".into()]).is_ok());
    }
    assert!(!store.exists());
    // A later provider on the same store finds no entry.
    let (_, progress) = measured(&install.local(), &["first.s3d"]);
    assert_eq!((progress.bytes, progress.cache_hits), (5, 0));
}

#[test]
fn request_response_matches_a_fresh_full_scan_including_spell_operand() {
    let install = Installation::new();
    install.write("Zone.S3D", b"zone bytes");
    install.write("spells_us.txt", b"actual current spells");
    install.write("resources/GlobalLoad.txt", b"global models");
    install.write("GlobalLoad.txt", b"wrong root copy");
    let manifest = b"\x11\x00\x01ZONE.S3D\0\x15\x00\x09GlobalLoad.txt\0\x19\x00\x02skipped.eqg\0\x21\x00\x05absent.eqg\0";
    let fresh = Assets::scan_all(&install.0).unwrap();
    let (actual, progress) = measured(
        &install.local(),
        &["zone.s3d", "globalload.txt", "absent.eqg", "spells_us.txt"],
    );
    assert_eq!(
        actual.file_response(manifest).unwrap(),
        fresh.file_response(manifest).unwrap()
    );
    assert_eq!(actual.spells().unwrap(), fresh.spells().unwrap());
    assert_eq!(progress.total, 4);
    assert!(!actual.files.contains_key("skipped.eqg"));
}

#[test]
fn corrupt_foreign_cache_and_missing_files_are_misses_not_invented_measurements() {
    let install = Installation::new();
    install.write("first.s3d", b"bytes");
    install.write("settings/asset-checksums-v1.json", b"corrupt");
    let (assets, progress) = measured(&install.local(), &["first.s3d", "missing.eqg"]);
    assert_eq!(progress.bytes, 5);
    assert!(!assets.files.contains_key("missing.eqg"));
    let cache_path = install.0.join("settings/asset-checksums-v1.json");
    let mut cache: Cache = serde_json::from_slice(&fs::read(&cache_path).unwrap()).unwrap();
    cache.root = PathBuf::from("another installation");
    fs::write(&cache_path, serde_json::to_vec(&cache).unwrap()).unwrap();
    assert_eq!(measured(&install.local(), &["first.s3d"]).1.bytes, 5);
    assert!(install
        .local()
        .measure(&["../outside".into()], &CancellationToken::default())
        .is_err());
}

#[test]
fn cancellation_does_not_hash_or_create_a_cache_and_drop_cancels_the_job() {
    let install = Installation::new();
    install.write("first.s3d", b"bytes");
    let stop = CancellationToken::default();
    stop.cancel();
    let provider = install.local();
    let counts = Counters::default();
    assert!(provider
        .scan(
            &["first.s3d".into()],
            &stop,
            &AtomicBool::new(false),
            &counts
        )
        .is_err());
    assert_eq!(counts.bytes.load(Ordering::Relaxed), 0);
    assert!(!install.0.join("settings/asset-checksums-v1.json").exists());
    let job = provider.measure(&["first.s3d".into()], &stop).unwrap();
    let cancelled = job.cancel.clone();
    drop(job);
    assert!(cancelled.load(Ordering::Relaxed));
}
