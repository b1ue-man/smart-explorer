# Reclaim Strategy Source Matrix

Check date: 2026-06-28. Sources are grouped by evidence role. "Primary" means
the source defines the API/tool/algorithm behavior directly. "Secondary" means
it is useful for implementation or expert evaluation, but should not override
primary docs.

## Source Catalog

| ID | Source | Type | Strategy relevance |
|---|---|---|---|
| S01 | [rmlint tutorial](https://rmlint.readthedocs.io/en/master/tutorial.html) | Tool documentation | Duplicate files, duplicate directories, hardlink/rmlint concepts |
| S02 | [rmlint manpage](https://www.mankier.com/1/rmlint) | Tool documentation | Duplicate criteria, lint types, safety modes |
| S03 | [rdfind manpage](https://rdfind.pauldreik.se/rdfind.1.html) | Tool documentation | Size, first bytes, last bytes, checksum elimination pipeline |
| S04 | [rdfind GitHub](https://github.com/pauldreik/rdfind) | Implementation / project | Real duplicate-finder implementation |
| S05 | [jdupes manpage](https://manpages.ubuntu.com/manpages/focal/man1/jdupes.1.html) | Tool documentation | Partial hash modes, byte comparison, hardlinks |
| S06 | [jdupes GitHub](https://github.com/jbruchon/jdupes) | Implementation / project | Real duplicate-finder implementation |
| S07 | [fdupes manpage](https://manpages.debian.org/testing/fdupes/fdupes.1.en.html) | Tool documentation | File size, MD5, byte comparison pipeline |
| S08 | [fdupes GitHub](https://github.com/adrianlopezroche/fdupes) | Implementation / project | Real duplicate-finder implementation |
| S09 | [Czkawka GitHub](https://github.com/qarmin/czkawka) | Implementation / project | Duplicate files, empty dirs, similar images/videos/music |
| S10 | [Czkawka documentation](https://qarmin.github.io/czkawka/) | Tool documentation | GUI/CLI feature coverage and cleanup categories |
| S11 | [duperemove GitHub](https://github.com/markfasheh/duperemove) | Implementation / project | Linux extent dedupe tool |
| S12 | [Btrfs deduplication docs](https://btrfs.readthedocs.io/en/latest/Deduplication.html) | Filesystem documentation | Userspace dedupe and reflink caveats |
| S13 | [ioctl_fideduperange(2)](https://man7.org/linux/man-pages/man2/ioctl_fideduperange.2.html) | OS documentation | Kernel-verified extent dedupe |
| S14 | [ioctl_ficlone(2)](https://man7.org/linux/man-pages/man2/ioctl_ficlone.2.html) | OS documentation | Reflink copy semantics |
| S15 | [bees GitHub](https://github.com/Zygo/bees) | Implementation / project | Btrfs block-level dedupe daemon |
| S16 | [bedup GitHub](https://github.com/g2p/bedup) | Implementation / historical | Btrfs dedupe concept and history |
| S17 | [LBFS paper](https://pdos.csail.mit.edu/papers/lbfs%3Asosp01/lbfs.pdf) | Academic paper | Rabin CDC, network/filesystem dedupe |
| S18 | [FastCDC paper](https://www.usenix.org/system/files/conference/atc16/atc16-paper-xia.pdf) | Academic paper | Fast content-defined chunking |
| S19 | [Sparse Indexing paper](https://www.usenix.org/legacy/event/fast09/tech/full_papers/lillibridge/lillibridge.pdf) | Academic paper | Chunk-index scaling |
| S20 | [Data Domain dedupe paper](https://www.usenix.org/legacy/events/fast08/tech/full_papers/zhu/zhu.pdf) | Academic paper | Chunking throughput and disk bottlenecks |
| S21 | [Restore Fragmentation paper](https://www.usenix.org/system/files/conference/fast13/fast13-final124.pdf) | Academic paper | Restore/read fragmentation from dedupe |
| S22 | [rsync technical report](https://rsync.samba.org/tech_report/) | Algorithm documentation | Rolling checksum and remote delta protocol |
| S23 | [restic CDC writeup](https://restic.net/blog/2015-09-12/restic-foundation1-cdc/) | Engineering blog / project | Practical Rabin CDC backup design |
| S24 | [Borg internals](https://borgbackup.readthedocs.io/en/stable/internals/data-structures.html) | Project documentation | Chunker and repository data structures |
| S25 | [CDC survey/preprint](https://arxiv.org/abs/2409.06066) | Survey / preprint | Chunking algorithm taxonomy |
| S26 | [Microsoft Change Journals](https://learn.microsoft.com/en-us/windows/win32/fileio/change-journals) | OS documentation | NTFS USN delta tracking |
| S27 | [FSCTL_ENUM_USN_DATA](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_enum_usn_data) | OS documentation | Enumerating USN data |
| S28 | [USN_RECORD_V2](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-usn_record_v2) | OS documentation | USN record fields |
| S29 | [FILE_ID_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info) | OS documentation | Windows file identity |
| S30 | [BY_HANDLE_FILE_INFORMATION](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/ns-fileapi-by_handle_file_information) | OS documentation | File index and link count |
| S31 | [FindFirstFileNameW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-findfirstfilenamew) | OS documentation | Hardlink enumeration |
| S32 | [Windows reparse points](https://learn.microsoft.com/en-us/windows/win32/fileio/reparse-points) | OS documentation | Junction/symlink traversal risk |
| S33 | [Symbolic link effects](https://learn.microsoft.com/en-us/windows/win32/fileio/symbolic-link-effects-on-file-systems-functions) | OS documentation | File API behavior around symlinks |
| S34 | [CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew) | OS documentation | Open flags for paths/reparse handling |
| S35 | [GetCompressedFileSizeW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getcompressedfilesizew) | OS documentation | Allocation size for compressed/sparse files |
| S36 | [FSCTL_QUERY_ALLOCATED_RANGES](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_query_allocated_ranges) | OS documentation | Allocated ranges for sparse files |
| S37 | [FindFirstFileExW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-findfirstfileexw) | OS documentation | Windows directory enumeration options |
| S38 | [stat(2)](https://man7.org/linux/man-pages/man2/stat.2.html) | OS documentation | Linux `st_blocks` allocation accounting |
| S39 | [lseek(2)](https://man7.org/linux/man-pages/man2/lseek.2.html) | OS documentation | `SEEK_HOLE` / `SEEK_DATA` |
| S40 | [FIEMAP kernel docs](https://docs.kernel.org/filesystems/fiemap.html) | OS documentation | Extent maps and shared/sparse awareness |
| S41 | [Google Drive files resource](https://developers.google.com/workspace/drive/api/reference/rest/v3/files) | Official API documentation | `md5Checksum`, `sha1Checksum`, `sha256Checksum` fields |
| S42 | [Microsoft Graph file resource](https://learn.microsoft.com/en-us/graph/api/resources/file) | Official API documentation | `file.hashes` on driveItem |
| S43 | [Microsoft Graph hashes resource](https://learn.microsoft.com/en-us/graph/api/resources/hashes) | Official API documentation | QuickXorHash and hash fields |
| S44 | [Dropbox content hash](https://www.dropbox.com/developers/reference/content-hash) | Official API documentation | Dropbox block-hash algorithm |
| S45 | [Dropbox Python FileMetadata](https://dropbox-sdk-python.readthedocs.io/en/latest/api/files.html) | SDK documentation | `content_hash` field exposure |
| S46 | [AWS S3 Object API](https://docs.aws.amazon.com/AmazonS3/latest/API/API_Object.html) | Official API documentation | ETag and checksum fields |
| S47 | [AWS S3 object integrity](https://docs.aws.amazon.com/AmazonS3/latest/userguide/checking-object-integrity.html) | Official API documentation | Multipart/checksum caveats |
| S48 | [rclone overview](https://rclone.org/overview/) | Tool documentation | Backend hash capability table |
| S49 | [rclone check](https://rclone.org/commands/rclone_check/) | Tool documentation | Hash/check behavior |
| S50 | [rclone hashsum](https://rclone.org/commands/rclone_hashsum/) | Tool documentation | Backend hashes and calculated hashes |
| S51 | [RFC 4918 WebDAV](https://datatracker.ietf.org/doc/html/rfc4918) | Protocol specification | ETag, getlastmodified, getcontentlength |
| S52 | [rclone WebDAV backend](https://rclone.org/webdav/) | Tool documentation | WebDAV provider differences |
| S53 | [ownCloud WebDAV checksums](https://doc.owncloud.com/server/next/developer_manual/core/apis/webdav/checksums.html) | Official API documentation | `OC-Checksum` behavior |
| S54 | [Nextcloud WebDAV basics](https://docs.nextcloud.com/server/latest/developer_manual/client_apis/WebDAV/basic.html) | Official API documentation | DAV properties/capabilities |
| S55 | [TLSH](https://tlsh.org/) | Project documentation | Locality-sensitive fuzzy hashing |
| S56 | [TLSH GitHub](https://github.com/trendmicro/tlsh) | Implementation / project | Fuzzy hash implementation |
| S57 | [ssdeep](https://ssdeep-project.github.io/ssdeep/index.html) | Project documentation | Context-triggered piecewise hashing |
| S58 | [pHash docs](https://www.phash.org/docs/howto.html) | Project documentation | Perceptual hashes and distances |
| S59 | [ImageHash GitHub](https://github.com/JohannesBuchner/imagehash) | Implementation / project | aHash/dHash/pHash/wHash practice |
| S60 | [Meta PDQ](https://github.com/facebook/ThreatExchange/tree/main/pdq) | Implementation / project | Image perceptual hash |
| S61 | [Chromaprint](https://acoustid.org/chromaprint) | Project documentation | Audio fingerprinting |
| S62 | [Meta TMK/PDQF](https://github.com/facebook/ThreatExchange/tree/main/tmk) | Implementation / project | Video similarity hashing |
| S63 | [npm cache](https://docs.npmjs.com/cli/v11/commands/npm-cache) | Official tool documentation | npm cache verification/cleaning |
| S64 | [npm ci](https://docs.npmjs.com/cli/v11/commands/npm-ci) | Official tool documentation | Reproducible dependency reinstall |
| S65 | [Cargo clean](https://doc.rust-lang.org/cargo/commands/cargo-clean.html) | Official tool documentation | Rust `target` cleanup |
| S66 | [pip caching](https://pip.pypa.io/en/stable/topics/caching/) | Official tool documentation | pip cache behavior |
| S67 | [Gradle directory layout](https://docs.gradle.org/current/userguide/directory_layout.html) | Official tool documentation | Gradle caches/build output |
| S68 | [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir-spec/latest/) | Specification | Cache semantics on Linux-like systems |
| S69 | [WizTree](https://diskanalyzer.com/) | Product documentation | MFT-based disk analyzer claim |

## Cross-Validated Main Claims

| Main claim | Support | Confidence | Smart Explorer implication |
|---|---|---|---|
| File-by-file full hashing is not the only viable strategy; the normal exact-duplicate pipeline first reduces candidates by metadata and cheap reads, then hashes or compares only survivors. | S03, S05, S07, S01, S09 | Strong | Keep size grouping, add partial fingerprints and late verification instead of eagerly hashing every file. |
| Partial hashes are exclusion filters, not deletion proof. | S03, S05, S07 | Strong | Label partial matches as candidates only; never auto-select from partial hash alone. |
| Full-file hashes are useful but should be algorithm-typed and, for destructive local actions, revalidated or byte-compared immediately before trash/delete. | S05, S07, S41, S44, S47 | Strong | Add `HashMatch` vs `VerifiedExact` status; do not treat all provider hashes as interchangeable. |
| Hardlinks/file identity must be modeled separately from duplicate content, because deleting one name may not reclaim logical duplicate bytes. | S29, S30, S31, S05, S02 | Strong | Add `(volume, file_id, link_count)` or platform equivalents; show hardlinks as already shared, not duplicate reclaim. |
| Directory duplicates are best detected bottom-up with tree signatures/Merkle-like summaries, not by recursively comparing path strings ad hoc. | S01, S02, S17 | Medium-strong | Implement after file hashes exist; avoid double-counting parent and child selections. |
| Raw NTFS MFT scanning can be very fast, but official Win32/USN APIs are a lower-risk first accelerator. | S26, S27, S28, S37, S69 | Strong | Do `FindFirstFileExW`/USN first; leave raw MFT as optional expert mode. |
| Reclaimable bytes are not always logical bytes; sparse, compressed, reflinked, and extent-shared files need allocation/extent awareness. | S35, S36, S38, S39, S40, S12, S13 | Strong | Track logical and allocated size; show estimates when allocation truth is unavailable. |
| In-place Linux extent dedupe is a different reclaim action from deletion and should use kernel-verified `FIDEDUPERANGE` results. | S11, S12, S13, S15 | Strong | Offer explicit "dedupe extents" only after exact candidates and filesystem capability probe. |
| Content-defined chunking is powerful for backup/version repositories, but it is not a safe default "delete duplicate files" strategy. | S17, S18, S19, S20, S21, S23, S24 | Strong | Treat FastCDC/Rabin as future backup/history architecture, not Reclaim v1 default. |
| Provider/free hashes can avoid network downloads, but each backend's hash algorithm and coverage must be represented explicitly. | S41, S42, S43, S44, S46, S48, S49, S50 | Strong | Add backend `hash_capabilities()` with algorithm, strength, coverage, and download cost. |
| WebDAV ETags are change tokens, not portable content hashes. Provider-specific checksums need probing. | S51, S52, S53, S54 | Strong | Use WebDAV ETag for invalidation only unless a server exposes stable checksum properties. |
| Perceptual/fuzzy hashes find similar items, not exact reclaim. They require review UX and conservative thresholds. | S55, S57, S58, S59, S60, S61, S62 | Strong | Make image/audio/video near-duplicates opt-in and never auto-delete. |
| Package/cache cleanup is safest when Smart Explorer delegates or explains native tool semantics instead of blindly deleting every familiar directory name. | S63, S64, S65, S66, S67, S68 | Strong | Upgrade cleanup policy to confidence tiers and tool-aware recommendations; remove `.git` from safe default cleanup. |
| Snapshot/growth analysis catches reclaim opportunities that hashing misses, but it needs a local history database and cannot rely solely on mtime. | S26, S27, S28, S68 | Medium | Add SQLite scan history with file identity where available; use USN as accelerator, not sole truth. |

## Evidence Gaps / Weak Areas

- Public Czkawka docs are strong for feature coverage, but weaker for a precise
  final byte-verification guarantee. Treat Czkawka as evidence for strategy
  breadth, not as proof of a specific safety pipeline.
- Exact FastCDC speedups are workload-dependent. The architectural direction is
  strong; product numbers must come from Smart Explorer benchmarks.
- WebDAV checksum behavior is server-dependent. ownCloud has explicit checksum
  documentation; generic WebDAV and Nextcloud deployments need runtime probing.
- Video near-duplicate detection has many valid methods. TMK/PDQF is a strong
  reference, but a Smart Explorer implementation needs separate codec/decode
  cost benchmarks before it belongs in the main UI.
