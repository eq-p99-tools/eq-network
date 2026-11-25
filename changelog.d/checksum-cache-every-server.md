- Reuse the installed-file checksum cache on every server, P99 included, not
  only on local-only EQEmu. A requested file whose size and modification time
  match its cached entry is answered from the cache; a refusal invalidates the
  refused files and the next login rereads them. The cache cannot see a
  same-size edit with a restored modification time. Delete
  `asset-checksums-v1.json` (`assets::CHECKSUM_CACHE_FILE`) from the settings
  directory to clear it by hand.
- If a refusal's cache invalidation cannot be saved, delete the cache file
  (best effort) and stop trusting metadata for the rest of that provider's
  life.
