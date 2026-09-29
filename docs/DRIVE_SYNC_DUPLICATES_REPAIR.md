# Unique file identity during synchronization

Date: 2026-09-29. Extension of the active Direct-opening repair batch.
Source implementation is present; combined remote acceptance and publication
are pending.

## Goal and evidence

The complete active batch includes the Direct-opening/recovery work in
[DIRECT_REMOTE_OPEN_REPAIR.md](DIRECT_REMOTE_OPEN_REPAIR.md) and the reported
sync failure for `Notebook/.obsidian/appearance.json`. Synchronization must
compare one logical file per relative path on each side. When same-name Drive
objects exist, keep the content that unambiguously matches both sides and remove
the redundant objects reversibly. Otherwise expose the competing versions as a
conflict. An explicit choice of A or a specific variant must leave that content
once on both sides, with the discarded versions recoverable.

Current evidence:

- `gdrive/core/backend.rs::list_dir` gives duplicate objects virtual
  `[drive-id ...]` names. The sync snapshot consumes that browsing listing and
  consequently treats the aliases as unrelated files.
- The ordinary snapshot stores signatures by path and drops the provider IDs.
  `apply_transfer.rs` captures an ID for its backup/read, but its publication
  calls the path-only staged promotion.
- `gdrive/core/promotion.rs` then requires exactly one object for the destination
  name, producing the reported error. Simply updating one ID would leave the
  duplicate namespace in place and does not meet the clarified request.
- Mirror orchestration has a generic deduplication hook, but the current Drive
  backend has no implementation of that hook. It cannot be treated as existing
  protection for this case.
- The existing checked resolution, backup, exact-ID trash, staged upload,
  cancellation, deletion limits and baseline-on-success rules are the boundaries
  to retain. This evidence does not establish which client created the existing
  duplicates in the user's account.

## Stage one: implementation approach

Use a separate provider listing contract for sync, preserving the existing
browsing locators. Carry duplicate groups as observations, not as synthetic
independent paths. Compare complete content fingerprints, never names, sizes or
timestamps alone, to choose an automatic survivor. If there is no unique content
choice, retain the group in the conflict model and expose all source choices.

Apply a preflighted group through exact IDs with backups and fresh state checks.
Publish the chosen content before removing other objects. A partially failed
repair must remain unresolved and must not advance the baseline. Preserve the
ordinary uniqueness guard for callers that have no captured destination identity.

## Research and second gap review

Primary sources checked 2026-09-29:

- [Drive file resource](https://developers.google.com/workspace/drive/api/reference/rest/v3/files):
  names need not be unique; IDs identify objects; binary files expose content
  checksums. Modification time is not proof of equal content.
- [Drive files.update](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/update)
  and [resumable uploads](https://developers.google.com/workspace/drive/api/guides/manage-uploads):
  update the captured file ID using the existing verified resumable upload, and
  reconcile uncertain completion instead of allocating another same-name object.
- [Trash and restore](https://developers.google.com/workspace/drive/api/guides/delete):
  trash is addressed by ID and is permission-controlled. Reuse exact-ID trash
  and preserve the local backup; do not use permanent deletion.

The second review checks the existing listing's reversible name encoding and
marker collision rules, cache forwarding, read-by-ID support, provider-native
exports, cancellation and post-commit handling. A duplicate directory must never
be flattened or recursively deleted by a regular-file repair. Literal marker-like
names and stored folder/connection locators retain their meaning. Read-only
sources, failed backups, stale observations and ambiguous IDs cannot authorize
cleanup. Preview is read-only and deletion limits include every automatic removal.
Incremental sync must not bypass unresolved duplicate observations.

Implementation decisions after the gap review: Drive pairs use a full paired
scan because the existing incremental index does not track target-side name
uniqueness. This costs additional listing requests on otherwise idle Drive jobs;
other providers keep their incremental path. `NoDelete` exposes a duplicate
conflict for an explicit user choice instead of automatically removing copies.
Variant transactions share the transfer limiter, keep the configured bandwidth
limit, back up every observed version, and only then publish/trash by exact ID.

## Stage two: final milestones and acceptance

| Milestone | Connected boundary | Expected result in the one combined remote suite |
| --- | --- | --- |
| C1: observe one logical file | VFS sync listing/defaults/cache, Drive listing adapter, bisync snapshots and duplicate model | Duplicate regular files remain a single relative-path group with exact IDs; ordinary providers, literal names, filters and protected omissions keep their contracts; no alias becomes a new copied file. |
| C2: plan unambiguous convergence | Pure group selection, paired snapshots, preview/run and incremental boundary | One common content version chooses one survivor per side; competing content produces an explicit variant conflict; dry run never mutates; automatic cleanup participates in deletion limits. |
| C3: apply the chosen version safely | Checked conflict resolution, variant backups, identity-bound staged publication, exact-ID trash and baseline integration | A/specific variant becomes the sole content on both sides. Every discarded version is backed up first. Failures, drift, denied permissions and cancellation retain recoverability and an unresolved baseline; ordinary no-replace writes never create duplicates. |
| C4: explain and select variants | Desktop conflict UI/worker, mobile conflict JSON, Android conflict models/actions | A unique A remains a direct choice; a side with competing versions offers explicit variant choices with size/date/content identity. No silent newest-file decision; merge/bulk paths cannot guess a variant. |
| C5: integrate and deliver both fixes | Focused HTTP Drive fixture and existing conflict integrations; existing `native/test-direct-open-task.py` and its workflow; root graph and canonical docs | The same task entrypoint covers the Direct-opening milestones and C1–C4 on remote Windows/Linux. Commit and push coherent milestones, evaluate that candidate, then one existing complete release. No separate release or competing suite. |

Remote fixtures exercise the real Drive HTTP adapter against controlled responses.
They do not access or modify the user's real Drive account. Physical Android
radio/Doze transitions and a real-account duplicate cleanup remain validation
limits to report, not claims inferred from a loopback result.

The combined entrypoint additionally invokes only the Android conflict-model
JVM fixture on Linux; Gradle incrementally compiles the affected Kotlin app
sources but does not package an APK or build native Android libraries. The
workflow and entrypoint budgets are 250 and 245 minutes. Native assertions cover
the mobile JSON fields, alongside the desktop/core transactions, and retain the
directly affected ordinary conflict, cancellation, link and snapshot integrations.
