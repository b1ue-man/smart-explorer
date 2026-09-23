# Sync link and reparse-point repair

Task batch: fix the reported whole-job abort at a nested `node_modules` entry,
retain cross-backend synchronization, and preserve existing files, baselines,
permissions and root boundaries. No local builds, compilers or tests are allowed.

## Stage one: code findings and initial approach

Inspected on 2026-09-23 at `0180b7b`, using the root Graphify query and current
source:

- `bisync/os/shared/snapshot.rs` returns a fatal error for one link-like entry
  before applying hidden-file and ignore filters. The orchestration then aborts
  the whole job before planning ordinary files.
- Windows directory enumeration already distinguishes name-surrogate reparse
  tags for Explorer rendering, but the local VFS marks every reparse point as a
  symlink. Classification therefore needs review for cloud placeholders.
- Simply dropping links from a snapshot is unsafe: a missing baseline path can
  mean deletion, and a matching target subtree could be overwritten or removed.
- Preview, full sync, incremental mirror, quick mirror and saved-job results
  consume related snapshots/results. Omitted subtrees must remain protected at
  those boundaries and omissions must be visible without reporting full success.

Initial approach: distinguish ordinary Windows data reparse points from actual
path-redirection links; represent unsupported child links as protected scan
boundaries; synchronize independent regular entries; preserve both sides and
prior baseline entries under each omitted boundary. Keep invalid metadata,
unreadable directories, cancellation and mutation races fail-closed.

The user has been asked whether true links should be omitted with a visible
notice or whether their target contents are intended to be synchronized. Work
on classification and the safety boundary does not depend on that preference.

## Primary-source research

Checked on 2026-09-23:

- [Microsoft reparse tags](https://learn.microsoft.com/en-us/windows/win32/fileio/reparse-point-tags)
  and [name-surrogate semantics](https://learn.microsoft.com/en-us/windows/win32/api/winnt/nf-winnt-isreparsetagnamesurrogate):
  reparse metadata does not itself imply a filesystem link. Name-surrogate tags
  identify redirection to another named entity; cloud data tags are distinct.
- [FILE_ATTRIBUTE_TAG_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_attribute_tag_info)
  and [CreateFile](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilea):
  query attributes/tags through a metadata handle opened without following the
  reparse point. Unknown or unreadable classification remains a protected boundary.
- [Rclone local link behavior](https://rclone.org/local/#symlinks-junction-points):
  independent regular files can be synchronized while reporting omitted links;
  following targets and copying link objects are separate, explicit policies.

## Stage two: detailed milestone plan

| Milestone | Affected boundary and dependencies | Expected acceptance signal |
| --- | --- | --- |
| Correct Windows reparse classification | `local_access/os/windows`, local VFS metadata and directory-creation adapters; independent | Listing and fresh metadata agree for files, directories, junctions, symlinks and cloud/data reparse tags. Unknown tags stay conservative; link ancestors cannot redirect writes. |
| Collect protected scan boundaries | `bisync` snapshot/filter layer and agent scan boundary; depends on classification | One nested link cannot abort independent regular files. Filters apply without entering excluded paths. An ordinary directory named `node_modules` still synchronizes. No link traversal or loops. |
| Preserve both sides and prior state | `bisync` preview/full planning, baseline updates, incremental mirror and quick mirror; depends on scan boundaries | An omitted file/subtree causes no transfer, deletion or conflict at that relative location on either side; its prior baseline remains unchanged. Other changes proceed. Mutation failures/cancellation still fail closed. |
| Make omissions visible | GUI preview/results, quick mirror results and scheduled-job notes/logs; depends on result model | The user sees omitted paths and a partial-result notice; no misleading “both sides equal” or unconditional success. Actual I/O failures remain errors. |
| Verify the reported failure and affected integrations | Extend the existing single `native/test-sync-paths-task.py` entrypoint and `sync-paths-task.yml` coverage after implementation | Remote Windows/Linux acceptance includes a real Windows junction, Unix links, the reported nested path shape, existing target/baseline data, ignored links, cloud-tag metadata fixtures, and the preceding cross-remote compatibility and safety boundaries. No local test invocation. |

## Second gap review and final decisions

The metadata query uses the existing read-only `ReadKind::Metadata` handle;
classification never opens a link target. Rust's Windows implementation also
[uses the name-surrogate bit](https://raw.githubusercontent.com/rust-lang/rust/1.98.0/library/std/src/sys/fs/windows.rs)
for `FileType::is_symlink`; ordinary data reparse points are not links. Retain a
conservative result when the explicit tag query fails or returns no usable tag.

Snapshots will retain protected relative-path prefixes separately from regular
file signatures. Union those prefixes across both endpoints before planning;
exclude their baseline entries from decisions and restore them unchanged after
updating independent paths. Revalidate the same protection after mutations.
Path matching respects component boundaries. The existing
`Backend::case_sensitive_paths` capability controls case aliases; unknown remote
filesystems retain conservative protection rather than assuming Unix semantics.

The existing agent hash stream has no omission field and currently hides links.
Keep its wire layout and ordinary-file fast path: an explicit, reserved hash-walk
error marker requests a fresh metadata walk when a link is encountered. Only
that marker permits fallback; cancellation and arbitrary partial-stream failures
remain fatal. Both the deployed SSH agent and daemon-backed hash handler need
the same behavior. SSH deployment already binds the agent executable to its exact
bundled hash, including reconnects. No protocol version or frame layout changes.

The legacy strict walk remains strict for incremental collection. A link makes
that path fall back to the full protected snapshot, and an incomplete full result
must not seed a supposedly complete incremental index. Quick mirror must protect
matching destination subtrees during deletion and report links separately from
I/O errors. GUI summaries, previews and job notes must retain the distinction.
Reports show bounded path samples; collection retains the existing walk budgets.

Retire a previous incremental index before a full mirror pass and bootstrap it
again only after a complete success. Touched incremental target ancestors are
checked before leaf metadata so an intervening junction selects the full scan.
Agent filesystem metadata and mutation guards use the same name-surrogate/data
reparse distinction as local VFS operations.

Absent a different user preference, true links are omitted with a visible
notice. No `node_modules` name-based exclusion is added. Verified data reparse
points remain ordinary filesystem entries.

The existing remote suite is the only task verification entrypoint; the complete
release wrapper is invoked remotely only after the final pushed candidate passes
that suite. The regression fixture must reproduce the reported nested path and
real link type, rather than relying only on ordinary backend fixtures.
