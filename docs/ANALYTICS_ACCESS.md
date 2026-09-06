# Storage-analysis access correction

## Goal and release boundary

User request on 2026-09-06: make local storage analysis read protected Windows
directories with the necessary, explicitly authorized privileges; request those
privileges when unavailable; expose the affected paths instead of only the first
error. Keep the already running mount release untouched, then publish this fix
as a subsequent release. Agent release checks are at least 30 minutes apart.

The independent working branch is `work/analytics-access`. Do not advance the
mount release's bound `main` until that transaction completes. No local builds,
tests, or release execution are allowed. The mount task has already passed its
remote suite; its distribution transaction remains separate from this task.

## Stage one: source evidence and approach

- `analytics/os/shared/analytics.rs` uses `std::fs::read_dir`, `DirEntry::file_type`
  and per-file `metadata`. It neither requests backup access nor enables a
  Windows backup privilege. Running the same code elevated is not equivalent to
  opening directories with an enabled backup privilege.
- `analytics_outcome.rs` distinguishes a failed root from a partial child scan.
  It retains up to 64 issues, but has no typed access-denied count.
- `app/core/analytics_core.rs` logs partial results through `push_app_error` and
  includes only the first path. `analytics_ui.rs` shows a partial-result count
  but no full retained issue list or rights-request action.
- The latest history touching these scanner/reporting paths predates the mount
  follow-up. This proves a missing access path and incomplete diagnostics, not
  when the user's particular Windows installation first encountered the issue.
- Keep logical file-size accounting, cancellation, scan budgets and remote
  provider authorization. Do not change ACLs, ownership, restore/write privileges,
  or the separately deferred MFT scanner.

Stage-one implementation: introduce a typed local-directory adapter, use a
Windows handle-based metadata enumerator with a narrowly scoped backup-read
retry, retain access-error identity through the outcome, and offer an explicit
UAC restart of this exact analysis in a separate application window. The original
window, mounts and current partial result stay intact. No privileged IPC service,
arbitrary helper command execution or staged result-file protocol is needed.

## Primary-source research

Checked 2026-09-06:

- [File security and backup access](https://learn.microsoft.com/en-us/windows/win32/fileio/file-security-and-access-rights)
  and [CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew):
  backup privileges must be granted and enabled, and the open must use backup
  semantics. Administrator membership alone is insufficient. Request read/list
  access only and preserve sharing with existing users of the directory.
- [AdjustTokenPrivileges](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-adjusttokenprivileges):
  nonzero return does not establish success; check `ERROR_NOT_ALL_ASSIGNED`.
  This API cannot grant a missing privilege. Use an impersonation-token copy,
  not a process-wide privilege change shared with other app functionality.
- [DuplicateTokenEx](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-duplicatetokenex)
  and [SetThreadToken](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setthreadtoken):
  scope the token to the calling scan worker and restore its previous identity.
- [GetFileInformationByHandleEx](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getfileinformationbyhandleex)
  and [FILE_ID_EXTD_DIR_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_extd_dir_info):
  directory records already contain filenames, logical sizes, attributes and
  reparse tags. File contents and per-file data handles are unnecessary.
- [Reparse tags](https://learn.microsoft.com/en-us/windows/win32/fileio/reparse-point-tags):
  name-surrogate links/mount points are traversal boundaries. Do not confuse
  every cloud/storage reparse tag with a directory redirect.
- [ShellExecuteExW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shellexecuteexw)
  and [SHELLEXECUTEINFOW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-shellexecuteinfow):
  use the documented `runas` consent path, initialize COM, distinguish canceled
  consent from a launch failure, and close the returned process handle. No shell
  interpreter or environment expansion is needed for the exact application path.

## Stage two: behavioral milestones

1. **Protected metadata enumeration.** `analytics/mod.rs`, `analytics/os/mod.rs`,
   focused Windows enumeration/decoder/privilege files, a portable filesystem
   adapter, and the existing local scanner. Ordinary opens remain first choice;
   denied opens retry with only `SeBackupPrivilege` on a temporary thread token.
   Use directory-record sizes, preserve native filenames, never read contents,
   and do not recurse through name-surrogate reparse children. Use documented
   compatibility fallback when an information class is unsupported; never turn
   permission denial into an empty successful directory.
   Acceptance: a real deny-ACL Windows fixture fails ordinary enumeration but
   scans correctly with available backup privilege; files locked against data
   reads still contribute their directory-record sizes; thread/process privilege
   state is unchanged afterward; redirect children do not lead out of the tree.

2. **Complete, honest access diagnostics.** `analytics_outcome`, local/backend
   error recording, a focused app issue-list/reporting module, `analytics_core`
   and `analytics_ui`. Track permission denial separately from unrelated errors;
   expose every retained path/detail, copying of the report, and an explicit
   omitted count. Keep useful partial trees visible without a generic app-error
   popup presenting a child denial as total failure.
   Acceptance: multiple child denials retain readable siblings and distinguish
   partial/failed/canceled outcomes; all retained paths appear in the report;
   omitted diagnostics remain explicit; a missing root still fails honestly.

3. **User-approved rights request.** Focused analytics access/startup APIs, a
   Windows launch adapter, app access-prompt state/controller and GUI bootstrap.
   A completed local scan with missing rights offers a consent prompt for this
   exact root in a new elevated analysis window. No automatic retry loop; keep
   the original window/result when consent is declined or launching fails.
   Remote provider roots do not request local Windows privileges. Verify and
   SHA-bind the current executable immediately before elevated launch, hold its
   file/path identity through launch, encode arguments without a shell, and
   validate the startup request before beginning the new scan.
   Acceptance: exact-root startup and argument quoting survive spaces/trailing
   separators; malformed paths/hash/arguments fail closed; canceled consent is
   distinct from error; no arbitrary executable or command can enter this route;
   existing app startup and remote scans retain their established behavior.

4. **One focused remote acceptance, then one subsequent release.** After all
   implementation, add one checked-in `analytics_access_task` suite/entrypoint
   and exact-candidate Windows CI dispatch with at least 30 minutes of runtime.
   Reuse a development library fixture and only the affected incremental build.
   Include the protected-directory, decoder, privilege-restoration, reporting,
   startup/launch-policy, cancellation and directly affected remote boundaries
   above. Do not launch broad workspace checks or another mount suite. Integrate
   the completed prior release before binding/pushing this candidate to `main`.
   Release only after this suite passes and the prior release is complete, using
   the same existing remote top-level release wrapper, not a new release path.

The legacy central `App` state/init files may receive only one typed access-state
field and its initializer; behavior lives in the extracted module. This is an
explicit integration exception, not an expansion of those oversized files.

## Final decisions after the second API review

Use the SDK's `offset_of!(..., FileName)` for variable records rather than an
assumed packed layout. Try extended directory records, then full records only
when the first query reports an unsupported information class; a final ordinary
enumerator fallback must preserve errors. End enumeration only on
`ERROR_NO_MORE_FILES`. Full records obtain a reparse tag only for reparse entries.
Check an opened directory's own tag before traversal. Restore the exact previous
thread token; failure to restore is a fail-closed process termination, as required
by Microsoft's impersonation guidance. Never change the process token or ACLs.

The elevation route admits exactly a dedicated startup switch, one absolute local
drive path and its current executable SHA-256. No additional switches, device/UNC
paths or relative path components are accepted. The executable and its non-root
parent directories remain locked against replacement through `ShellExecuteExW`.
The existing GUI gets one typed access controller; issue rendering is extracted
to avoid growing the already large treemap module. Consent is an explicit action,
not an automatic launch or repeated prompt. Declining leaves the result intact.

Inspection of `App::new` also found automatic daemon/update initialization. The
elevated window therefore uses a dedicated read-only analytics `eframe::App`,
dispatched before normal GUI bootstrap, with progress, cancellation, drilldown,
the existing treemap layout and the shared diagnostics view. It does not start
the ordinary app's background services, updater, reclaim or file-write actions.

Windows may still
refuse access because of filesystem/provider policy or unavailable privileges;
report that accurately rather than claiming every possible path is readable.
Logical file totals are not a claim to include unexposed filesystem bookkeeping,
snapshots or all allocated disk space.

## Candidate implementation and acceptance entrypoint

Implementation checkpoints `fdd46d4` and `fe92c36` are pushed on the isolated
branch. The candidate includes the typed directory adapter, temporary backup
token, directory-record decoder, exact-purpose elevated startup and retained-path
report. A startup-validation failure opens an error-only analysis window; it
never falls through into the ordinary app or starts a scan.

The single entrypoint is `native/test-analytics-access-task.ps1`, dispatched by
`.github/workflows/analytics-access-task.yml` on Windows 2025 with an exact source
SHA. It preflights administrator/backup-token availability before compiling,
reuses the existing source-bound library binary-cache helper, builds only the
incremental native library fixture if needed, and runs only
`analytics_access_task`. The workflow has 120 minutes; its task step has 110.
No mount runtime installation, broad test matrix or release build is part of it.

Coverage maps to milestones 1–3: real denied directory and locked-file metadata,
ACL/process-token invariance, preservation of an existing or restricted thread
identity, junction boundaries, full-record fallback, malformed record admission,
native UTF-16 names, partial/root/canceled outcomes, reporting omissions, remote
authorization separation, exact startup/hash binding and Windows argument parsing.
Actual interactive UAC consent and GUI rendering are not claimed as automated
headless desktop certification. The API result/quoting path and privileged
filesystem operations are exercised separately by this same suite.

Status: implementation complete; remote acceptance has not started. The previous
mount release remains a separate, unattended transaction. No local builds or
tests were run; source formatting and static parser checks do not execute Rust
compilation/linking or the suite.
