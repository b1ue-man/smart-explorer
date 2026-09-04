# Windows remote-mount request delivery

## Evidence and selected correction

Checked on 2026-09-04 against Dokany `v2.3.1.1000`, upstream commit
`f1d5de68ff459af94e309cfdd171e4b8ca2af4dd`, and the official x64 MSI pinned in
`native/dokany-runtime.nsh`. The extracted `dokan2.dll` SHA-256 is
`75600aba867acbdfdb85fcd142b524da769bdc611b855a760aeb0c6e2eaae17a`, identical
to the user's System32 DLL.

The application enabled `DOKAN_OPTION_ALLOW_IPC_BATCHING` (`0x1000`).
Dokany passes that flag to the kernel, but its library converts the mask to
Windows `BOOLEAN`, an unsigned byte, when selecting its event consumer. The
conversion yields zero. The kernel can return multiple event records while
the selected consumer dispatches only the first and then reuses the buffer.
Subsequent requests in the batch receive no callback/reply. This is supported
by both source and static disassembly of the exact released DLL, not inferred
from a similar application's behavior.

Sources:

- [Library selection and dispatch](https://github.com/dokan-dev/dokany/blob/f1d5de68ff459af94e309cfdd171e4b8ca2af4dd/dokan/dokan.c#L816).
- [Driver event pulling](https://github.com/dokan-dev/dokany/blob/f1d5de68ff459af94e309cfdd171e4b8ca2af4dd/sys/fscontrol.c#L455).
- [Windows data types](https://learn.microsoft.com/en-us/windows/win32/winprog/windows-data-types)
  and [unsigned conversions](https://learn.microsoft.com/en-us/cpp/c-language/conversions-from-unsigned-integral-types).
- [Introducing upstream change](https://github.com/dokan-dev/dokany/commit/aef92bcf23c0dea150e7864a4ef81984325fd6a5).

As of the check date, [2.3.1.1000 remains the newest official release](https://github.com/dokan-dev/dokany/releases/tag/v2.3.1.1000),
and [master at c7a59fc](https://github.com/dokan-dev/dokany/blob/c7a59fc68ddcfeb4474f2fe7f24be4eb264af6a2/dokan/dokan.c#L816)
still contains the defect. No public acknowledgment of this exact truncation
defect was found. The related maintainer discussion of a formerly missing
driver flag is not acknowledgment of this later request-loss defect.

The selected correction clears batching before every `DokanCreateFileSystem`
call and leaves ordinary multithreaded processing enabled. Driver options are
sent before Dokany's worker-count adjustment, so clearing the bit after
creation, or only selecting single-thread mode, is too late. Dokany can mutate
the caller's options; enforcing the invariant at every create boundary protects
reused options too. Read-only, case-sensitivity, session, mount-manager, timeout,
and callback-lifetime behavior must remain unchanged. The official installer
pin and shared system runtime remain unchanged. Existing mounts need a remount
using the corrected application.

Do not re-enable batching merely because a newer version exists. First verify
both producer and consumer behavior against that exact released runtime.
Timeouts can complete individual lost requests while Dokany's keepalive keeps
the mount present; this investigation does not claim that every single request
waits forever or that the third/fourth navigation is a fixed trigger count.

## Two-stage implementation and acceptance plan

Stage one traced the common Windows mount path, checked callback ABI/lifetimes,
locks and remote transports, and compared upgrading Dokany, distributing a
custom DLL, and disabling the optional batching optimization. The last option
directly removes the demonstrated mismatch without a driver replacement.

Stage two resolved initialization order, option mutation/retry handling,
fixture construction, and bounded Windows-process supervision before editing.
The coherent behavioral milestones and their acceptance criteria are:

1. **Safe filesystem creation** (`host.rs`, `dokany_abi.rs`, `runtime.rs`):
   batching is absent on every call, including reused options, and unrelated
   options plus ordinary parallel processing are preserved.
2. **One-command installed-volume check** (`native/verify-mount-windows.ps1`):
   discover managed mounts or accept an explicit drive; exercise bounded
   concurrent directory, metadata, and small-file reads; skip reparse points;
   never write remote data or automatically unmount user drives. Parent-owned
   deadlines cover discovery and workers. Report pass, insufficient coverage,
   error, or timeout without remote paths, contents, account names, or tokens.
3. **Dependency-bound integration evidence** (Windows host test fixture and
   one remote task entrypoint/workflow): run the production options, callbacks,
   engine, real System32 DLL, and installed kernel driver against a deterministic
   virtual backend. Exercise deep navigation and concurrent reads with known
   expected results, then clean teardown. The backend fixture isolates the
   Windows boundary shared by SSH, Share and fallback; it is not a claim of
   end-to-end certification of all transports. Exercise the user checker on
   that same mounted volume. Missing driver/runtime is a failure, not a skip.
4. **Ordinary-node attribute queries** (`callbacks_open.rs` and the same Windows
   fixture): accept no-follow opens on ordinary nodes, while continuing to
   reject actual remote links and open-by-ID. Validate the latter before
   interpreting a possibly binary file ID as a terminated path. The same suite
   must observe successful raw root/file attribute queries, rejection of the
   fixture link, and the unchanged PowerShell navigation/read acceptance.
5. **Distribution**: refresh the root source graph, commit/push the complete
   candidate, evaluate one focused remote Windows suite, then invoke the existing
   complete-release automation once. Verify the published version and assets.

The task suite is created after the behavioral implementation. It incrementally
builds only the affected native development/test targets, reuses those outputs,
and invokes no full workspace, cross-platform or release build. All compilation,
test execution, MSI installation and release work run on the configured remote
runner, never the agent workstation. The terminal release remains the existing
`native/publish-release-local.ps1` transaction; no alternate release path is added.

Windows supervision uses bounded `Process.WaitForExit(Int32)` and drains both
redirected streams concurrently; a parameterless wait or synchronous sequential
pipe reads can introduce a second hang. PowerShell encoded commands use UTF-16LE.
These contracts were checked on 2026-09-04 against [Process.WaitForExit](https://learn.microsoft.com/en-us/dotnet/api/system.diagnostics.process.waitforexit),
[redirected output](https://learn.microsoft.com/en-us/dotnet/api/system.diagnostics.process.standardoutput),
and [Windows PowerShell invocation](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_powershell_exe?view=powershell-5.1).

## Running the bounded checks

The single CI entrypoint is `native/test-mount-batching-task.ps1`, dispatched
through `.github/workflows/mount-batching-task.yml` with `candidate_sha` equal
to the selected pushed commit. Its Windows job has 120 minutes and the entrypoint
has 110 minutes. `-InstallRuntime` installs only the pinned official MSI on that
disposable remote runner if the DLL is absent. Installed DLL and driver SHA-256
must match the audited binaries before the real-volume fixture runs. The script
accepts `-TestBinary` to reuse an existing native library test executable;
otherwise it builds that single incremental target and discovers the executable
from Cargo's JSON output. Only `mount_batching_task` cases execute.

The CI workflow separately retains the compiled fixture with
`-BinaryCacheRoot`, including on checker failure. Before reuse, the task verifies
its binary SHA-256 and a fingerprint of the complete committed Git tree,
excluding only this document, `docs/RELEASING.md`, and the standalone checker.
Changes to Rust, assets, dependencies, the task script, its cache helper, or the
workflow invalidate that fingerprint. A mismatch or corrupt cache falls back to
the same incremental library-target build. Checker-only revisions can therefore
reuse the exact executable while rerunning the same task entrypoint. This is
separate from the dependency cache, which normally removes workspace binaries.
Fixture errors label the failing startup or native I/O operation, including
synthetic fixture paths; bounded phase messages identify mount readiness,
navigation completion, checker launch/results and teardown. These CI-only
diagnostics are separate from the sanitized user checker report.
The failed Windows checker returned a base `0x80070002` before its first
directory lookup completed. That error alone does not identify the failing
kernel request: [.NET Framework's attribute helper](https://github.com/microsoft/referencesource/blob/3b1eaf5203992df69de44c783a3eda37d3d4cd10/mscorlib/system/io/file.cs#L1356)
can retry `GetFileAttributesEx` failures using `FindFirstFile`, trimming a root's
trailing separator. The fixture therefore records bounded, test-only create,
information and enumeration callback statuses/flags, and the raw parent-process
`GetFileAttributesExW` result before selecting a correction.
[Run 33929186454](https://github.com/b1ue-man/smart-explorer/actions/runs/33929186454)
then demonstrated the exact cause: a root create request with access `0x80`,
disposition `FILE_OPEN`, and options `0x00200000` was rejected by the production
callback with `STATUS_NOT_SUPPORTED` (`0xc00000bb`). The raw parent query
returned Win32 error 50. PowerShell processes reached the same mount, succeeded
with ordinary directory opens, and hit the same rejected no-follow request.
Thus this failure was not a missing process/session drive mapping.

The correction removes the blanket rejection of `FILE_OPEN_REPARSE_POINT`:
the [ordinary-file flag contract](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
says this flag has no effect on non-reparse nodes. Existing metadata-based
remote-link refusal remains in place. The raw root and ordinary-file queries
become required assertions alongside the unchanged user checker; the fixture
link must still return error 50. Unsupported open-by-ID is rejected before path
decoding because [its name can be binary and not NUL-terminated](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-zwcreatefile).
No visibility flag, retry or delay is changed. These additional API contracts
and the plan's security/acceptance boundaries were checked on 2026-09-04 before
the correction. The earlier unlabelled native error 2 could not be attributed
retrospectively; subsequent native navigation completed, and exact failure
labels remain in the fixture rather than treating a non-recurrence as proof.
The cache/save-on-failure contracts were checked on 2026-09-04 against the
[Rust cache documentation](https://github.com/Swatinem/rust-cache/blob/v2/README.md)
and [GitHub cache save action](https://github.com/actions/cache/blob/v4/save/README.md).

For an installed Windows mount, the standalone checker requires only Windows
PowerShell 5.1 or later, not a repository build, Rust, or debugger. From a checked-out
repository, its one command is:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\native\verify-mount-windows.ps1
```

It discovers `se.exe` and up to three normal managed mounts. `-Drive M` explicitly
selects a drive and bypasses CLI discovery. `-ReportPath C:\local\new-report.json`
optionally creates a new local JSON report; it never overwrites an existing file.
Automatic CLI startup accepts only a plain system-drive executable with no
observed reparse ancestors: Windows opens executables synchronously, outside
the child timeout. Use explicit `-Drive` for nonstandard CLI locations. This
check does not claim atomic protection against concurrent local path replacement.
Exit codes are 0 (PASS), 2 (INCONCLUSIVE), 3 (ERROR), and 4 (TIMEOUT). The default
overall budget is 120 seconds, with at most five additional seconds for optional
local report writing. A PASS requires four workers each completing three rounds,
at least five distinct directories, and a nonempty small-file read. Insufficient
sample data cannot produce a PASS.

The probe never mounts, retries, unmounts, replaces the runtime, or writes remote
data. Reparse entries are skipped; this is not an atomic namespace-confinement
guarantee against a concurrently changing tree. Files are sampled only when
observed metadata reports at most 64 KiB, since the mount can materialize an
entire file on open; concurrent growth can exceed that size estimate. Reports
omit remote paths, names, file contents, account labels and raw CLI output.
Failures retain only numeric worker-script line/column locations, bounded
exception/error identifiers, and outer/base HRESULTs. Exception messages,
source-line text, target objects and stack traces are not included.

This validates observed behavior, not the identity of an already-running mount
host or every possible remote operation. After updating the application, remount
before using it to check the correction. The real-driver fixture and the broader
SSH/Share/reconnect/write certification tracked in `TODO.md` are distinct.
