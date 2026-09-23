# Sync endpoint compatibility repair

Task batch: make every folder-picker filesystem location usable by sync,
including local/UNC, SFTP (with or without the SSH agent), FTP/FTPS, WebDAV,
Google Drive and Direct/Room Share combinations. Preserve existing behavior
and record the compatibility requirement for future agents. Read-only providers
remain read-only: using them as a source does not grant write permission.

Published in [v0.5.161](https://github.com/b1ue-man/smart-explorer/releases/tag/v0.5.161)
on 2026-09-23. The implementation plan below records the investigated batch;
delivery evidence is recorded at the end.

## Stage one: findings and approach

Source inspected on 2026-09-22, starting at `a493d66`, with the root Graphify
query and direct source checks:

- `app/core/sync_core.rs` constructs local destination backends for quick sync,
  bypasses authenticated UNC resolution for local-looking saved jobs, and
  compares split roots without their remote identities.
- `central_tabs.rs` saves backend-relative split roots; `menus_sync.rs` loses
  the active remote prefix when creating a setup.
- `picker_types.rs` hides remote destinations for mirror/bisync. Picker
  connection state duplicates endpoint-prefix construction and drops UNC leases.
- The transfer engines already accept two independent `Backend` objects and
  protect overwrite/delete with backups, revalidation and retryable baselines.
- Connection endpoints are historical application locators with literal path
  strings, not uniformly percent-encoded RFC URLs. Changing their decoding
  would reinterpret existing filenames and saved jobs.

Use the existing resolver and transfer engines, preserve endpoint provenance
through the picker and setup editor, and compare paths within backend namespaces.
Do not replace sync with another engine or migrate persisted baseline identities.

## Research and second gap review

Checked primary sources on 2026-09-22:

- [RFC 3986, sections 2.4 and 3.2](https://www.rfc-editor.org/rfc/rfc3986.html):
  parse components before decoding; avoid double decoding and distinguish the
  authority from the path. Preserve this application's existing literal path
  representation, including percent signs, spaces, Unicode, `#` and `?`.
- [Microsoft path formats](https://learn.microsoft.com/en-us/dotnet/standard/io/file-path-formats):
  drive roots and UNC shares have filesystem semantics; a bare drive designator
  must not accidentally become a drive-relative sync destination.
- [Rclone bisync](https://rclone.org/bisync/): cross-provider sync uses two
  independent endpoints, and provider permissions/filename limitations still
  apply. Keep failed operations out of successful baseline state.

The second review traced saved-connection credentials, IPv6 authorities, cached
browsing backends, local root aliases, Share endpoint identities and both setup
creation paths. Sync needs uncached metadata while retaining live sessions and
their permissions. Network work and filesystem canonicalization belong in the
worker, never in the picker render loop. Invalid URL-like endpoints must produce
an explicit error instead of falling through to a local filesystem operation.

## Final implementation milestones

| Milestone | Surface and dependencies | Expected result / acceptance signal |
| --- | --- | --- |
| Preserve compatibility as a repository rule | `AGENTS.md`; independent | Every future change must identify and preserve affected established behaviors, backend types and persisted formats; agents report unresolved regressions. |
| Resolve endpoint paths consistently | `connect/core`, `connect/os/shared`, `syncjobs/core`, editor validation | Every supported picker locator resolves to its original backend path; literal special characters and legacy locators survive; malformed/unknown schemes never become local writes. Credential matching respects saved roots and IPv6. |
| Carry picker and tab provenance into sync | `app/core/picker*`, new picker-location helper, `sync_core`, menus, `central_tabs` | Quick mirror/bisync accept remote destinations; live connections survive selection; Direct/Room Share and open remote tabs are selectable; both setup creation paths retain endpoint prefixes; saved jobs use the shared worker resolver. |
| Preserve safe sync across namespaces | VFS backend metadata/cache interface, sync pair guard, mirror/bisync launch | Equal path text on different remotes works; identical/nested roots in the same namespace are refused; browsing caches do not hide live changes; existing backup, conflict, cancellation and retry semantics remain intact. |

After all implementation, one checked-in `native/test-sync-paths-task.py`
entrypoint will cover these expectations and directly affected existing safety
integrations through one exact-candidate GitHub Actions dispatch. It reuses a
source-bound library fixture or an incremental host library build, with isolated
app data and local/loopback fixtures on remote Linux and Windows runners.
No build, compiler or test entrypoint runs on the initiating workstation.
Only after the suite is evaluated successfully may the existing `build.yml`
complete-release transaction run for the pushed candidate. No intermediate
version bump, tag, artifact build or publication is part of these milestones.

## Persistence compatibility

Ordinary endpoints keep the existing `source=` and `target=` format. Endpoints
with leading/trailing whitespace, tabs or line breaks use `source_json=` or
`target_json=` to preserve the exact string. The reader continues accepting old
plain fields with their established trimming semantics. There is no lossy plain
fallback alongside an encoded field: an older application must reject an
unrepresentable job instead of syncing a different path. Baseline identities and
literal `%` names retain their existing meaning. Reopening an existing Windows
setup keeps its stored backslashes; separator normalization is used only for
path comparisons, so an existing setup continues using its original baseline.

The existing ZIP backend remains read-only. Its live session can supply a direct
mirror source; it has no persistent connection locator. The setup editor reports
that limitation instead of persisting its internal `/` as a local root.

## Delivery evidence

The final source candidate `f82776164a838e50770142999443eb3c10a5663d` passed the
single task suite on Windows and Linux in
[run 35738067140](https://github.com/b1ue-man/smart-explorer/actions/runs/35738067140).
The acceptance logs cover picker/setup provenance, literal and persisted paths,
old Windows baseline identity, backend pairings, uncached metadata, actual
authenticated loopback Share transfers, and the directly affected existing
backup, conflict, retry, WebDAV and Drive behavior. Provider pairings use the VFS
contract fixtures; the suite does not access users' remote accounts.

The existing complete-release wrapper ran remotely once in
[run 35824655263](https://github.com/b1ue-man/smart-explorer/actions/runs/35824655263).
It advanced the version once, built the complete artifacts, committed and pushed
`d3a057ec8183f17a319728d21a273bb4558dadd5`, and created immutable tag `v0.5.161`.
Its artifact-only [publication run](https://github.com/b1ue-man/smart-explorer/actions/runs/35830620475)
succeeded. On 2026-09-23, Cargo, feed version, installer and tag were checked for
agreement, all feed SHA-256 sidecars matched their payloads, and every required
GitHub Release asset matched the committed size and SHA-256. No build, compiler,
test suite or release entrypoint was invoked on the initiating workstation.
