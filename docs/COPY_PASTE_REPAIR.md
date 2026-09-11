# Local and cross-remote copy/paste repair

Inspection baseline: `4f96ffa`, 2026-09-11. Live status: C1 in [TODO.md](TODO.md).

## Goal and deliverables

Restore file copy/paste from the local machine into Share/Direct exports and
between remote locations. Review the complete clipboard -> destination routing
-> transfer -> acknowledged commit chain, correct demonstrated defects, preserve
source data and concurrent destination data, and deliver one installable release
after the single remote task suite. No local builds, tests, or release execution.
The actual user endpoint/clipboard error is not yet known; code findings must
not be presented as a reproduction of every reported failure.

## Stage one: source-supported approach

The graph query used existing vocabulary `clipboard copy paste peer share
transfer remote backend write`, then source inspection followed the actual
Windows adapters, GUI helpers, VFS promotion and Share server.

- Ordinary app copy calls `OpenClipboard(NULL)`, `EmptyClipboard`, then
  `SetClipboardData`. The documented ownership contract rejects this sequence;
  early returns also bypass closing and leak untransferred allocations.
- Clipboard reads trust offsets, terminators and effect sizes; ANSI bytes are
  incorrectly treated as UTF-8. Busy/error and no-file cases are conflated.
- Remote paste returns from its CF_HDROP-only branch before considering the
  application's own filtered virtual-file payload.
- Background clipboard preparation has no publication sequence guard. Immediate
  paste may consume old contents and later completion may replace newer contents.
- The Windows key poller queues commands while typing/dialogs are active, but
  the GUI only drains them when file shortcuts become enabled again. Consume
  and discard blocked commands each frame rather than replaying them later.
- Cross-remote tab drops compare textual parents before backend identity.
- Upload preflight chooses a free final name but commits with replacement;
  concurrent destination creation (including case aliases) can lose data.
  Bulk upload accepts arbitrary counts and retries after potentially mutating
  failures; post-commit cancellation can hide completed files from accounting.
- Share server copy rejects different exports of one peer; operation paths trim
  significant whitespace; write conflicts lose their error classifications.
- The normal current-version Share write-to-local-export chain does explicitly
  flush and acknowledge promotion. No unconditional transport failure was found.
  Synthetic Share containers are not writable file destinations.

The approach is to repair these boundaries, not weaken authorization, infer
absence from errors, or retry ambiguous mutations. Remote cut/move is not made
destructive merely to match the requested copy behavior: unsupported moves must
be explicit and preserve sources.

## Research pass one (2026-09-11)

Checked primary contracts: [OpenClipboard](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-openclipboard),
[SetClipboardData](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setclipboarddata),
[Shell formats](https://learn.microsoft.com/en-us/windows/win32/shell/clipboard),
[GetClipboardData](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getclipboarddata),
and [Rust Write](https://doc.rust-lang.org/std/io/trait.Write.html).
CF_HDROP paths and descriptor/stream payloads are distinct. Clipboard memory is
untrusted, and ownership transfers only after successful publication. A writer's
drop is not a successful commit; explicit flush errors must reach the UI.

## Stage two: final behavioral milestones

| Milestone | Affected boundary | Expected acceptance signal |
| --- | --- | --- |
| M1: Windows clipboard ownership and parsing | `shell_clipboard/os/windows*`, Windows/Linux app adapters | Actual Windows file clipboard round-trip works repeatedly; copy/cut effects and Unicode survive; busy/malformed input is reported distinctly; error paths release clipboard and untransferred memory. |
| M2: clipboard payload and preparation lifecycle | app clipboard, preparation drains/state, filtered upload adapter | Sequence-matched filtered hierarchy pastes to remote; pending/stale preparations never paste or publish older data; unsupported remote moves leave sources untouched and are explicit. |
| M3: cross-namespace routing | app drag/drop, Share copy operation/path/error codec | Same textual directory on different backends transfers; same peer across exports copies; rename remains same-export; path spelling and conflict errors are retained. |
| M4: copy commit and truthful completion | remote upload/copy/download helpers | Concurrent destination creation is never overwritten by copy; explicit edit-save still replaces; bulk mismatch/failure is not replayed into an already-mutated final tree; cancellation records acknowledged commits. |
| M4a: provider-compatible private stages | VFS copy-stage API, WebDAV writer, Drive create-only writer | WebDAV sends conditional creation; Drive only creates its own reserved ID and verifies name/parent/content without updating another ID; ordinary edit-save remains unchanged. |
| M5: one focused integration suite | checked-in task entrypoint and remote CI | Real Windows clipboard feeds actual local -> Share upload and back/cross-export paths; directory/empty/Unicode/filter/collision/error/cancel cases cover M1-M4 with only affected incremental development binaries. |
| M6: terminal delivery | existing complete-release wrapper | One remote release after suite success; installer, feed, tag and visible Release agree. No per-milestone release. |

Dependencies: M2 consumes M1's fallible clipboard read and M4's filtered upload;
M3/M4 must agree on no-replace commit and typed errors. M5 is authored after all
production changes; no milestone authorizes an early execution cycle.

## Research pass two and implementation constraints

Checked [message-only windows](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features#message-only-windows),
[GlobalSize](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-globalsize),
[clipboard lifecycle](https://learn.microsoft.com/en-us/windows/win32/dataxchg/using-the-clipboard),
and [DragQueryFileW sizing](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-dragqueryfilew).
A native owned window must not depend on whichever external application happens
to be foreground. Allocation length bounds parsing; four-byte effect access must
be checked; Windows performs ANSI filename conversion at its adapter boundary.
Use checked-in Windows 0.58 declarations when editing signatures.

Provider compatibility gap checked on 2026-09-11 against
[HTTP conditional creation](https://www.rfc-editor.org/rfc/rfc9110.html#section-13.1.2),
[WebDAV destination overwrite](https://www.rfc-editor.org/rfc/rfc4918.html#section-10.6),
[Drive generated upload IDs](https://developers.google.com/workspace/drive/api/guides/manage-uploads#use_a_pre-generated_id_to_upload_files),
and [Drive file identity/names](https://developers.google.com/workspace/drive/api/reference/rest/v3/files).
WebDAV and Drive advertise staged creation but currently inherit an unsupported
exclusive writer. WebDAV can use `If-None-Match: *` and preserve HTTP 412 as a
conflict. Drive can create a reserved object ID without touching another object,
but cannot atomically reserve a sibling name. Add a copy-specific staging API
whose ID-provider guarantee is explicit; do not weaken mounted exclusive-create
contracts or claim Drive implements an atomic filesystem namespace. Its copy
commit uses existing exact-ID rename, uniqueness verification and rollback.
Both providers publish lazily on flush, so opening a writer proves no remote
ownership. An ambiguous failure must not authorize path-based cleanup.
The cache wrapper must forward both copy-stage methods and invalidate stages
and final names after mutations. Downloads also need an explicit provider read
length: zero bytes is a known empty file, not a generic unknown-size sentinel.
Only transformed provider exports declare an unknown stream length. Acceptance
must cover empty-file growth rejection and transformed reads without comparing
their output length against the provider's stored-document size.

The GUI copy path stops using final-tree bulk upload/download and same-backend `copy_file` where
their interfaces cannot guarantee non-replacement after a name preflight.
The existing temporary-file bridge avoids overlapping read/write sessions;
this sacrifices the SSH bulk/server-side-copy shortcut. This is a deliberate
copy-safety tradeoff, not a performance improvement. Explicit edit-save retains
replacement semantics. Filtered OLE publication has no documented atomic
expected-sequence API: retain a late sequence/generation guard, without claiming
it eliminates an external write racing the OLE call itself.

Keep helpers narrowly scoped and below the native file-size limits. Do not
delete failed Share stages without ownership proof, flatten filtered relative
paths, reinterpret transport/permission errors as absence, or implement
copy-only promotion as a probe followed by replacing rename. Existing synthetic
mount acceptance is not clipboard acceptance. The configured task pipeline must
run Windows coverage; the current Git credential lacks workflow-write scope, so
remote pipeline provisioning needs a supported credential before dispatch.

## Outcome

Implementation and remote acceptance pending. No claim of delivered correction.
