# Reclaim Research Search Log

Check date: 2026-06-28. This log records the breadth-first and follow-up
searches used for `RECLAIM_STRATEGIES_DEEP_DIVE.md`. The stop rule was:
continue until at least three consecutive search batches produced no new
strategy family. Batches B10, B11, and B12 only reinforced existing families, so
the search was treated as saturated for the current Smart Explorer decision.

## Search Batches

| Batch | Query / lookup terms | Source type | Result status | New strategy family found |
|---|---|---|---|---|
| B01 | `file reclaim strategies`; `duplicate file detection algorithms`; `storage cleanup duplicate finder strategies`; `disk cleanup stale large duplicate files` | General web search | Usable overview terms only; led to duplicate, stale, cleanup, and policy buckets. | Metadata-first, policy cleanup, stale/large files |
| B02 | `size partial hash full hash byte compare duplicate finder`; `rdfind checksum only if necessary`; `jdupes partial full hash byte compare`; `rmlint paranoid Merkle duplicate directories` | General web, tool docs, manpages | Strong support for progressive exact duplicate detection. | Progressive exact duplicates, byte verification |
| B03 | `rmlint documentation duplicate directories Merkle tree`; `rdfind man page algorithm ranking`; `jdupes manual partial hash`; `fdupes byte by byte comparison`; `Czkawka duplicate similar images empty folders` | Tool docs, manpages, GitHub | Tool behavior cross-check; rmlint/rdfind/jdupes/fdupes/Czkawka agree on staged elimination patterns, with different safety guarantees. | Directory-level duplicate trees, hardlink handling |
| B04 | `duperemove extent dedupe FIDEDUPRANGE`; `Btrfs deduplication duperemove`; `bees btrfs deduplication`; `bedup btrfs deduplication` | Filesystem docs, GitHub, manpages | Strong support for filesystem-native extent dedupe as a separate action from deletion. | Extent-level dedupe |
| B05 | `NTFS MFT disk usage analyzer WizTree`; `USN journal file index duplicate detection`; `Windows file ID hard links GetFileInformationByHandle`; `FindFirstFileEx large fetch enumeration` | Microsoft docs, product docs | Strong support for Windows metadata acceleration, with raw MFT as advanced/high-risk. | OS-native enumeration, file identity, hardlink accounting |
| B06 | `FSCTL_ENUM_USN_DATA USN_RECORD`; `Windows file ID hard links FindFirstFileNameW`; `GetCompressedFileSize sparse FSCTL_QUERY_ALLOCATED_RANGES`; `FIDEDUPERANGE Btrfs XFS`; `FIEMAP SEEK_HOLE st_blocks openat2 no symlinks` | OS docs, Linux manpages, kernel docs | Added allocation-size and reparse/symlink safety dimensions. | Allocated-size accounting, traversal boundaries |
| B07 | `content defined chunking Rabin fingerprint deduplication`; `LBFS Rabin fingerprints`; `FastCDC paper`; `fixed size vs variable size chunking deduplication survey`; `data deduplication techniques survey chunking hashing evaluation metrics` | Academic papers, backup docs | Strong support that chunking is best for backup/version stores, not default file deletion. | Fixed chunking, CDC/Rabin/FastCDC |
| B08 | `sparse indexing chunk index deduplication`; `Data Domain deduplication disk bottleneck`; `restore fragmentation deduplication`; `restic Rabin chunking`; `BorgBackup buzhash chunker`; `rsync rolling checksum algorithm` | Academic papers, backup docs | Reinforced chunk index, fragmentation, and remote-protocol caveats. | Snapshot/backup repository strategy |
| B09 | `Google Drive API md5Checksum`; `Microsoft Graph driveItem hashes quickXorHash`; `Dropbox content_hash algorithm`; `S3 ETag multipart checksum`; `rclone hash remote backends` | Official API docs, rclone docs | Strong support for provider-hash capability matrix. | Provider/free hashes, remote hash capabilities |
| B10 | `WebDAV getetag checksum OC-Checksum Nextcloud ownCloud`; `rclone WebDAV hashes`; `Google Docs md5Checksum unavailable`; `OneDrive quickXorHash not cryptographic` | Official WebDAV/cloud docs, rclone docs | Reinforced provider caveats; no new family. | None |
| B11 | `perceptual hash duplicate image detection`; `pHash dHash aHash false positives`; `TLSH locality sensitive hash`; `ssdeep context triggered piecewise hashing`; `similar video duplicate detection` | Library docs, GitHub, project docs | Strong support for fuzzy/semantic review-only mode; no automatic reclaim proof. | Fuzzy/near-duplicate review |
| B12 | `package manager cache cleanup npm cargo gradle`; `temporary file cleanup heuristics`; `node_modules disk cleanup safety`; `build artifacts reclaim cache invalidation`; `XDG cache semantics`; `Windows USN change journal file system tracking` | Official package/tool docs, OS specs | Reinforced cleanup-policy and snapshot/growth strategy; no new family. | None |

## Saturation Evidence

The strategy families stopped expanding after B09. B10 only refined cloud/WebDAV
hash caveats, B11 mapped fuzzy variants into the already known "review-only"
family, and B12 mapped package-manager details into the already known
policy-cleanup and snapshot/growth families. No search batch after B09 produced a
new implementation family that changes the Smart Explorer architecture decision.

## Exact Search Terms Used

### General

- `file reclaim strategies`
- `duplicate file detection algorithms`
- `storage cleanup duplicate finder strategies`
- `disk cleanup stale large duplicate files`

### Progressive Duplicate Detection

- `size partial hash full hash byte compare duplicate finder`
- `rdfind checksum only if necessary`
- `jdupes partial full hash byte compare`
- `rmlint paranoid Merkle duplicate directories`
- `fdupes byte by byte comparison duplicate files`
- `duplicate finder hard links inode file id`

### Tool-Specific

- `rmlint documentation duplicate directories Merkle tree`
- `rdfind man page algorithm ranking`
- `jdupes manual partial hash`
- `fdupes man page file sizes md5 byte by byte`
- `czkawka duplicate similar images empty folders`
- `duperemove extent dedupe FIDEDUPRANGE`
- `bees btrfs deduplication`
- `bedup btrfs deduplication`

### Filesystem / OS

- `NTFS MFT disk usage analyzer WizTree`
- `USN journal file index duplicate detection`
- `Windows file ID hard links GetFileInformationByHandle`
- `Windows FILE_ID_INFO hard links FindFirstFileNameW`
- `FindFirstFileEx large fetch enumeration`
- `FSCTL_ENUM_USN_DATA USN_RECORD`
- `GetCompressedFileSize sparse FSCTL_QUERY_ALLOCATED_RANGES`
- `Linux FIEMAP SEEK_HOLE st_blocks dedupe`
- `openat2 no symlinks recursive delete safety`

### Chunk / Block Dedupe

- `content defined chunking Rabin fingerprint deduplication`
- `LBFS Rabin fingerprints`
- `FastCDC paper`
- `fixed size vs variable size chunking deduplication survey`
- `data deduplication techniques survey chunking hashing evaluation metrics`
- `sparse indexing chunk index deduplication`
- `Data Domain deduplication disk bottleneck`
- `restore fragmentation deduplication`
- `restic Rabin chunking`
- `BorgBackup buzhash chunker`
- `rsync rolling checksum algorithm`

### Cloud / Remote

- `Google Drive API md5Checksum`
- `Microsoft Graph driveItem hashes quickXorHash`
- `Dropbox content_hash algorithm`
- `S3 ETag multipart checksum`
- `rclone hash remote backends`
- `rclone check hashes remote providers md5 sha1`
- `Nextcloud ownCloud WebDAV oc checksums`
- `WebDAV getetag checksum OC-Checksum Nextcloud ownCloud`

### Similar / Fuzzy

- `perceptual hash duplicate image detection`
- `pHash dHash aHash false positives`
- `TLSH locality sensitive hash`
- `ssdeep context triggered piecewise hashing`
- `similar video duplicate detection`
- `Chromaprint AcoustID audio fingerprinting`
- `PDQ TMK PDQF video perceptual hash`

### Cleanup Heuristics

- `package manager cache cleanup npm cargo gradle`
- `temporary file cleanup heuristics`
- `node_modules disk cleanup safety`
- `build artifacts reclaim cache invalidation`
- `npm cache verify clean npm ci node_modules official`
- `cargo clean target directory official`
- `pip cache purge official`
- `Gradle cache cleanup directory layout official`
- `XDG cache semantics`
