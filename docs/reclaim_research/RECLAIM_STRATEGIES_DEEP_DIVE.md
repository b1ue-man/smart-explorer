# Reclaim Strategies Deep Dive

Check date: 2026-06-28. Scope: Smart Explorer `Find & Reclaim` strategy
research for local, remote, and cloud reclaim. This is documentation only; it
does not claim that the implementation already includes the roadmap items below.

Current code reality, checked against the repository on 2026-06-28:

- Local Reclaim lives in `native/src/analytics/os/shared/reclaim.rs`.
- The current local duplicate path groups by file size and then computes MD5 for
  remaining files above `duplicate_min_bytes`.
- The current scan also reports large files, stale files, empty files/dirs, and
  cleanup-name candidates such as `node_modules`, `.git`, caches, logs, and
  `target`.
- Remote/backend interfaces already have useful hooks such as provider content
  hashes and agent `walk_hashed`, but Reclaim does not yet use a full remote
  reclaim strategy.

Supporting files:

- `SEARCH_LOG.md` records the exact search batches and saturation proof.
- `SOURCE_MATRIX.md` records the source catalog and cross-validation table.

## Executive Conclusion

File-by-file hashing is only one layer, not the strategy. The highest-value
Smart Explorer path is:

1. Metadata-first and policy-first scanning to avoid reading file contents where
   possible.
2. Provider/free hashes for remote/cloud backends where the provider already
   exposes a typed content hash.
3. Progressive exact duplicate detection: size grouping, optional partial
   fingerprints, full typed digest only for survivors, then byte-verify or
   revalidate before destructive actions.
4. OS-native acceleration and correctness: file identity, hardlinks,
   reparse-point boundaries, allocated-size accounting, and later USN/MFT-style
   acceleration.
5. Review-only fuzzy modes for images/audio/video and semantic similarity.
6. Chunk/block dedupe as a future backup/version-store direction, not the default
   Reclaim v1 delete workflow.

The biggest immediate product value is not "hash faster"; it is "hash less,
prove more, and explain risk better".

## Strategy Catalog

### 1. Metadata-First Candidate Reduction

How it works:

- Walk the tree and collect name, path, file type, logical size, mtime, extension,
  file identity, link count, hidden/system flags, and directory context.
- Use exact metadata as cheap filters: unique sizes cannot be exact file
  duplicates; zero-byte files can be grouped without content reads; directories
  with known build/cache names can be classified before descending deeply.
- For remote backends, prefer listing metadata and provider-side fields over
  downloading file bodies.

Evaluation:

- Speed: excellent. Directory listing is still I/O, but it avoids content reads.
- Reliability: high for exclusion, not enough for proving identical content.
- CPU/RAM: low if the scanner streams and caps result lists.
- Disk/network I/O: low to medium, depending on directory count.
- Cancelability: good because work is naturally per-entry/per-directory.
- Safety: safe as a filter; not safe as deletion proof.

Smart Explorer fit:

- This should remain the first layer.
- Extend the model with file identity and allocated size.
- Keep result caps and low parallelism; the user's resource concern makes this
  more important than raw throughput.

Cross-validation: `SOURCE_MATRIX.md` S03, S05, S07, S29, S30, S37, S48.

### 2. Progressive Exact Duplicate Detection

How it works:

1. Group files by logical size.
2. Drop singleton size groups.
3. Optionally read a small prefix/suffix or sampled block fingerprint.
4. Hash full content only for remaining candidates.
5. For local destructive actions, byte-compare or re-hash immediately before
   moving anything to trash.

Why it beats file-by-file hashing:

- Unique-size files never need hashing.
- Partial fingerprints reject many same-size non-duplicates with tiny reads.
- Full hash and byte-verify are paid only for plausible duplicates.

Evaluation:

- Speed: high in normal user folders; worst case is many same-size large files.
- Reliability: high only after full hash plus byte verification/revalidation.
- False positives: partial hash can false-positive; cryptographic hashes are very
  low risk but still not a proof under adversarial inputs; byte compare is exact.
- CPU/RAM: bounded if groups are streamed and per-group candidates are limited.
- I/O: proportional to surviving candidates, not all files.
- Cancelability: good if every read loop checks cancellation.

Smart Explorer fit:

- Replace current `size -> MD5` as the product mental model with:
  `Candidate -> PartialMatch -> HashMatch -> VerifiedExact`.
- MD5 can remain useful for Google Drive/Nextcloud compatibility, but the local
  exact path should support a stronger digest such as SHA-256 or BLAKE3 plus
  byte verification before trash.
- Do not auto-select `HashMatch` as if it were `VerifiedExact`.

Cross-validation: S03, S05, S07, S01, S02, S41, S44.

### 3. File Identity, Hardlinks, and "Already Shared" Data

How it works:

- Capture a platform identity: Windows volume serial + file index / `FILE_ID_INFO`
  where available; Unix-like systems use device + inode.
- Capture link count.
- If two paths point to the same file object, they are not duplicate content
  copies in the usual sense; deleting one name may not reclaim the logical file
  size.

Evaluation:

- Speed: cheap for candidate files; doing it for every file may require extra
  stats/handles.
- Reliability: high when the platform identity is available.
- UX risk: high if ignored, because reclaimable bytes are overstated.

Smart Explorer fit:

- Add "hardlink group / already shared" as its own category.
- Do not count same-file identities as duplicate reclaim bytes.
- Before deletion, re-check identity and metadata to avoid races.

Cross-validation: S29, S30, S31, S05, S02.

### 4. Provider / Free Hashes

How it works:

- Some providers expose content hashes in listings or metadata.
- Google Drive exposes checksum fields for binary/blob files.
- Dropbox exposes `content_hash`, a provider-specific block hash.
- Microsoft Graph exposes `quickXorHash` and related hash fields, with caveats.
- S3 exposes checksum fields, while ETag is not reliably a content MD5.
- rclone demonstrates the right model: per-backend hash capabilities.

Evaluation:

- Speed: excellent when hashes are in the listing; no file download.
- Reliability: strong only when algorithm and coverage are known.
- Network I/O: low, but fields may need explicit selection or extra metadata
  calls.
- False positives: depends on algorithm; QuickXor is weaker than cryptographic
  digests; ETag is often not a digest.
- UX risk: medium if different algorithms are mixed.

Smart Explorer fit:

- Add a typed `HashCapability` model:
  `algorithm`, `strength`, `scope`, `available_in_listing`, `requires_download`,
  `coverage`, `provider_notes`.
- Never compare `DropboxContentHash` to MD5 as if they were the same algorithm.
- Treat WebDAV ETag as a change token unless a concrete server exposes a stable
  checksum property.

Cross-validation: S41, S42, S43, S44, S46, S47, S48, S49, S50, S51, S53.

### 5. Remote Agent Hash Index

How it works:

- A trusted remote agent walks the tree near the data and streams signatures:
  path/id, size, mtime, optional content hash.
- A persistent local index stores hash results keyed by remote identity plus
  invalidation metadata such as size, mtime, ETag/version, and provider id.
- When metadata changes, the old hash becomes stale and must be recomputed.

Evaluation:

- Speed: very good for SSH/agent-style remotes after the first scan.
- Reliability: high when invalidation metadata is trustworthy.
- Network I/O: much lower than downloading every file to hash locally.
- Cancelability: good if the agent streams progress and honors cancel.
- Safety: good for candidates; destructive remote operations still need
  revalidation and a review journal.

Smart Explorer fit:

- This is the best remote path for non-cloud servers that do not expose free
  provider hashes.
- The existing `walk_hashed` shape is already aligned with this strategy.
- Avoid client-side remote full-file hashing as a default; it is slow, expensive,
  and exactly the kind of work that makes the PC feel overloaded.

Cross-validation: S22, S48, current Smart Explorer backend model, and the remote
provider docs above.

### 6. Filesystem-Native Enumeration and Delta Acceleration

How it works:

- Windows can use `FindFirstFileExW` with appropriate info/fetch modes for a
  lower-risk fast walk.
- NTFS USN can accelerate changes/deltas and file enumeration on supported
  volumes.
- Raw MFT scanning can be extremely fast, but is NTFS-specific, elevated, and
  easier to get wrong.

Evaluation:

- Speed: high to excellent.
- Reliability: official APIs are safer; raw parsing is fragile and permission
  sensitive.
- Implementation cost: medium for `FindFirstFileExW`, high for robust USN/MFT.
- Cancelability: good for official enumeration loops.
- UX risk: raw MFT requires clear "expert mode" framing.

Smart Explorer fit:

- Tier 1 Windows accelerator: `FindFirstFileExW` plus typed OS metadata adapter.
- Tier 2: USN-backed snapshot/growth acceleration with fallback when the journal
  rotated or is unavailable.
- Tier 3: optional raw MFT scanner only if there is a clear benchmark win and
  safe fallback.

Cross-validation: S26, S27, S28, S37, S69.

### 7. Allocation-Aware Reclaim

How it works:

- Logical size is not always the bytes that will be reclaimed.
- Sparse files, compressed files, deduplicated extents, reflinks, and filesystem
  block allocation can make logical size diverge from allocated size.
- Windows has compressed-size and allocated-range APIs; Linux exposes block
  counts and extent/hole APIs.

Evaluation:

- Speed: medium; allocation detail often requires extra syscalls.
- Reliability: higher reclaim estimates when available.
- UX value: high for accurate "space recovered" claims.
- Implementation cost: medium to high across platforms.

Smart Explorer fit:

- Store both `logical_size` and `allocated_size_estimate`.
- Present reclaim bytes as "estimated" when only logical size is known.
- Do not promise that deleting a sparse or reflinked duplicate frees its full
  logical size.

Cross-validation: S35, S36, S38, S39, S40, S12, S13.

### 8. Extent-Level In-Place Dedupe

How it works:

- On Linux filesystems that support it, `FIDEDUPERANGE` asks the kernel to verify
  that ranges are identical and then share extents.
- This can reclaim physical storage without deleting either file.
- It is not the same as `FICLONE`, which is more about reflink copy semantics.

Evaluation:

- Speed: good after candidates are known; filesystem dependent.
- Reliability: strong when the kernel reports per-range success.
- Safety: safer than replacing files, but still needs review and backup/undo
  thinking.
- Implementation cost: high enough to keep out of first Windows-focused v1.

Smart Explorer fit:

- Offer later as an explicit advanced action on Btrfs/XFS-like targets.
- Run only after exact duplicate candidate discovery.
- Record per-range results in the review journal.

Cross-validation: S11, S12, S13, S14, S15.

### 9. Directory-Level Duplicate Trees

How it works:

- Build file signatures first.
- Build directory signatures bottom-up from sorted child names, child types,
  sizes, and child signatures.
- Equivalent directory signatures identify duplicate directory trees.

Evaluation:

- Speed: good once file signatures exist.
- Reliability: high if file signatures are verified and symlink/reparse behavior
  is explicit.
- UX risk: parent/child double counting and "delete whole tree" anxiety.

Smart Explorer fit:

- Implement after the exact file pipeline and file identity model.
- Show nested duplicate directories carefully; selecting a parent should suppress
  child duplicate reclaim in totals.
- Treat empty directory trees separately from content duplicate trees.

Cross-validation: S01, S02, S17.

### 10. Chunk / Block-Level Dedupe

How it works:

- Fixed-size chunking splits files by offset.
- Content-defined chunking (CDC) chooses boundaries based on the content, often
  using Rabin/Gear-style rolling fingerprints; FastCDC improves performance.
- Chunk identity still requires a strong hash over the chunk content.

Evaluation:

- Speed: fixed chunks are simple; CDC is more CPU-intensive but better after
  insertions/deletions.
- Reliability: exact only when chunks are strongly hashed and verified.
- Storage value: excellent for backups/version stores; awkward for normal file
  deletion reclaim.
- RAM/index cost: can be high; smaller chunks increase dedupe and metadata.
- Remote suitability: poor unless the remote side cooperates.

Smart Explorer fit:

- Do not use chunking as the default Reclaim v1 mode.
- Use it later for a backup/version/history repository with packs, manifests,
  chunk indexes, and retryable restore.
- If implemented, prefer a FastCDC-like chunker with conservative chunk sizes and
  a strong chunk ID such as `length + BLAKE3/SHA-256`.

Cross-validation: S17, S18, S19, S20, S21, S22, S23, S24, S25.

### 11. Fuzzy / Semantic Near-Duplicate Detection

How it works:

- Image: aHash/dHash/pHash/PDQ produce perceptual fingerprints compared by
  distance.
- Audio: Chromaprint-style fingerprints identify recordings across encodings.
- Video: frame sampling or TMK/PDQF-like approaches compare visual sequences.
- Generic fuzzy hashing: TLSH/ssdeep can group similar binary/text artifacts.

Evaluation:

- Speed: image hashing is moderate; audio/video decode can be expensive.
- Reliability: good for review candidates, not proof of duplicate content.
- False positives: meaningful and threshold-dependent.
- UX risk: high if presented as "safe to delete".

Smart Explorer fit:

- Add only as opt-in "similar media" mode.
- Require thumbnails/previews and explicit review.
- Never auto-select fuzzy matches.
- Semantics may influence explanation and risk score, not deletion proof.

Cross-validation: S55, S57, S58, S59, S60, S61, S62.

### 12. Policy Cleanup

How it works:

- Detect generated or recoverable directories and files: build outputs, package
  dependency folders, caches, logs, temporary files, and known tool caches.
- Prefer tool-aware cleanup where the ecosystem provides a command.
- Treat project-local and global caches differently.

Evaluation:

- Speed: excellent because names and context are cheap.
- Reliability: high for obvious build outputs, lower for broad names like
  `cache` or `.git`.
- UX risk: medium to high if Smart Explorer deletes developer state blindly.
- Reclaim value: often very high in real machines.

Smart Explorer fit:

- Keep policy cleanup, but refine confidence:
  `safe_auto`, `safe_review`, `risky_review`, `never_auto`.
- `node_modules` should be high-confidence only when a lockfile or package
  manifest makes reinstall plausible.
- `target`, `build`, `dist`, Python caches, old logs, and temp files should be
  context-scored.
- `.git` should be removed from default cleanup selection. Offer Git-specific
  advice such as `git gc` or `git clean` only with clear warnings.

Cross-validation: S63, S64, S65, S66, S67, S68.

### 13. Snapshot / Growth Analysis

How it works:

- Store scan snapshots in a small local database: path, size, mtime, file
  identity, parent, reason, hash status, and scan timestamp.
- Compare 7/30/90 day deltas, rapidly growing folders, newly large files,
  recurring downloads, stale generated outputs, and repeated backup snapshots.
- On Windows, USN can accelerate change discovery, but the app database remains
  the product truth.

Evaluation:

- Speed: first scan same as metadata walk; later scans can be much faster.
- Reliability: good for trends, not proof that content is duplicate.
- UX value: high because it answers "what changed?" and "why is space gone?"
- Implementation cost: medium; needs schema/versioning/privacy thinking.

Smart Explorer fit:

- Add a SQLite history after the exact Reclaim model is safer.
- Use history to prioritize scans and explain recommendations.
- Do not rely on mtime alone; use file identity and provider ids when available.

Cross-validation: S26, S27, S28, S68.

### 14. Stale / Large / Empty Item Heuristics

How it works:

- Large file detection sorts by size.
- Stale detection uses age/mtime thresholds.
- Empty files and directories are cheap to detect.

Evaluation:

- Speed: excellent.
- Reliability: "large" and "empty" are facts; "stale" is a hint.
- UX risk: stale files can be important archives; empty directories can be
  markers.

Smart Explorer fit:

- Keep them as review categories.
- Empty files are generally safer than empty dirs; empty dirs inside known cache
  contexts are safer than arbitrary empty dirs.
- Stale should never be auto-delete by itself.

Cross-validation: S01, S09, S10, S68.

## Comparison Matrix

Scores are product-oriented: 5 is favorable for Smart Explorer, 1 is unfavorable.

| Strategy | Speed | Reliability | Low false positives | Low CPU | Low disk I/O | Low network I/O | Cancelability | Safety | Implementation cost |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Metadata-first | 5 | 3 | 5 as filter | 5 | 4 | 5 | 5 | 5 as filter | 2 |
| Progressive exact duplicates | 4 | 5 when verified | 5 when verified | 3 | 3 | 2 remote / 5 local | 4 | 5 when verified | 3 |
| Provider/free hashes | 5 | 4 typed / 2 opaque | 4 | 5 | 5 | 5 | 5 | 4 with revalidation | 3 |
| Remote agent hash index | 4 | 4 | 4 | 4 local PC | 5 local PC | 4 | 4 | 4 | 4 |
| File identity/hardlinks | 4 | 5 | 5 | 4 | 4 | 5 | 4 | 5 | 3 |
| OS-native enumeration/USN | 5 | 4 | 5 as metadata | 5 | 4 | 5 | 4 | 4 | 4 |
| Allocation-aware reclaim | 3 | 4 | 5 | 3 | 3 | 5 | 4 | 5 | 4 |
| Extent-level dedupe | 3 | 5 on supported FS | 5 | 3 | 3 | 5 | 3 | 4 | 5 |
| Directory-level trees | 4 after file hashes | 4 | 4 | 3 | 3 | 2 remote | 4 | 3 UX risk | 4 |
| Fixed-size chunking | 3 | 3 | 3 | 3 | 2 | 1 remote | 3 | 2 as reclaim | 4 |
| CDC/FastCDC chunking | 3 | 4 in backup repo | 4 | 2 | 2 | 1 remote | 3 | 2 as reclaim | 5 |
| Fuzzy media/hash | 2-4 | 2 exact / 4 similar | 2 | 2-4 | 2-3 | 1 remote | 3 | 2 | 4 |
| Policy cleanup | 5 | 3-5 context-dependent | 3 | 5 | 5 | 5 | 5 | 3-5 | 2 |
| Snapshot/growth | 4 after first scan | 3 | 4 for trend facts | 4 | 4 | 4 | 4 | 4 | 3 |
| Stale/large/empty | 5 | 2-5 category-dependent | 3 | 5 | 5 | 5 | 5 | 3 | 1 |

## Expert Evaluation and Cross-Validation

### What duplicate-finder tools agree on

rmlint, rdfind, jdupes, and fdupes all point toward staged elimination rather
than naive hashing of every file. The exact sequence differs, but the product
lesson is stable: cheap facts first, expensive content reads late, and explicit
safety modes for final actions.

Implication: Smart Explorer should not optimize by simply swapping MD5 for a
faster hash. It should reduce the number of files that need hashing and then
separate candidate status from verified status.

### What filesystem experts imply

Filesystem APIs show that "space recovered" is a filesystem fact, not just a
sum of logical file sizes. Hardlinks, sparse files, compression, reflinks, and
extent sharing all change what deletion or dedupe actually frees.

Implication: Smart Explorer should eventually display estimated vs confirmed
reclaim bytes and avoid overstating hardlink or sparse-file wins.

### What cloud/provider docs imply

Cloud providers are not interchangeable. Google Drive MD5, Dropbox content hash,
OneDrive QuickXorHash, S3 checksums, S3 ETag, WebDAV ETag, and ownCloud checksum
properties each have different semantics.

Implication: Smart Explorer needs typed provider hash capabilities. A single
`md5: Option<String>` field is too narrow for Reclaim vNext.

### What dedupe papers imply

LBFS, FastCDC, Sparse Indexing, Data Domain, and restore-fragmentation work all
support chunk dedupe as a powerful storage-system technique. They also show why
it needs indexes, chunk-size tuning, restore considerations, and cooperative
protocols.

Implication: CDC belongs in a future backup/version repository or remote-agent
workflow, not as a first-pass file cleanup button.

### What fuzzy-hash/media tools imply

Perceptual and fuzzy hashes are useful discovery tools. They do not establish
identity. Their thresholds are domain and corpus dependent.

Implication: "Similar" must be a review UI, not an automatic reclaim action.

## Smart Explorer Roadmap

### Tier 1: Highest Value, Low Risk

1. Add confidence/status model:
   `Candidate`, `PartialMatch`, `HashMatch`, `VerifiedExact`, `PolicySafe`,
   `PolicyReview`, `RiskyReview`, `NeverAuto`.
2. Refine cleanup policy:
   remove `.git` from default cleanup; mark as `NeverAuto` or Git-tool advice.
   Require context for `node_modules`, `target`, `build`, `dist`, caches, and
   global package caches.
3. Add typed provider hash capabilities:
   Drive MD5/SHA fields, Dropbox content hash, OneDrive QuickXorHash as weak,
   S3 checksum vs ETag caveat, WebDAV checksum probing.
4. Add local pre-delete revalidation:
   re-stat selected duplicate files and byte-verify or strong-hash them before
   sending to trash.

Why first: this directly improves safety and speed without a large new storage
engine.

### Tier 2: Faster and More Exact Local Reclaim

1. Add partial fingerprint stage for same-size groups.
2. Add file identity and link-count metadata.
3. Track logical and allocated size where cheap.
4. Add duplicate-directory signatures after exact file status exists.
5. Store a review journal for planned actions and outcomes.

Why second: it reduces full reads and fixes incorrect reclaim accounting.

### Tier 3: Remote and OS Acceleration

1. Use provider hashes in remote/cloud Reclaim.
2. Use agent `walk_hashed` and persistent hash index for non-cloud remotes.
3. Add Windows `FindFirstFileExW`/metadata adapter.
4. Add USN-backed incremental snapshots with full-walk fallback.
5. Keep raw NTFS MFT as optional expert acceleration only after benchmarking.

Why third: it unlocks remote value while avoiding network-heavy hashing.

### Tier 4: Advanced / Optional Modes

1. Similar image review with dHash/aHash first, pHash/PDQ for candidates.
2. Audio review through Chromaprint/fpcalc-style fingerprinting.
3. Video review only after decode-cost benchmarks.
4. Linux extent dedupe via `FIDEDUPERANGE` as an explicit advanced action.
5. FastCDC/Rabin chunking as part of a backup/version repository, not Reclaim v1.

Why fourth: powerful, but easy to over-promise or overload the machine.

## What Should Replace File-by-File Hashing Now

- Metadata-first elimination.
- Context-aware policy cleanup.
- Provider/free hashes where available.
- Partial fingerprints before full hash.
- Verification status before destructive actions.

The practical replacement is not one algorithm. It is a staged pipeline that
does less work and produces a more honest confidence level.

## What Should Not Be in Reclaim v1

- Default raw NTFS MFT parsing. Use official APIs first.
- Default chunk-level dedupe. It is a backup/storage-engine feature.
- Automatic fuzzy-media deletion. It is similarity, not identity.
- Blind deletion of `.git` or global package caches.
- Treating S3 ETag, WebDAV ETag, Dropbox content hash, QuickXorHash, and MD5 as
  the same "hash".

## Product Decision

The next implementation slice should be:

1. `ReclaimEvidence` / `ReclaimConfidence` data model.
2. Typed hash capabilities in backend metadata.
3. Progressive duplicate pipeline with optional partial fingerprint and final
   local verification.
4. Policy cleanup confidence rewrite.
5. Review journal for all destructive actions.

That gives Smart Explorer the biggest reclaim value with the least machine load:
fewer full reads, fewer network downloads, clearer risk labels, and safer
deletion behavior.
