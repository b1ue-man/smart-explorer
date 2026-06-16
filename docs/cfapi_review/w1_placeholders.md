# CfAPI Review W1 — Placeholder Creation & On-Demand Population (FETCH_PLACEHOLDERS)

Authoritative, doc-grounded reference for Smart Explorer's CfAPI sync-root placeholder
creation path. Scope: `native/src/cfprovider.rs::fetch_placeholders` (the
`SyncFilter::fetch_placeholders` callback) and `populate_to`, built on
`cloud-filter` v0.0.6.

Ground-truth sources:

- OUR CODE: `/home/user/smart-explorer/native/src/cfprovider.rs`
  - `fetch_placeholders` lines 92–131
  - `populate_to` lines 142–160
  - `m.name` source: `native/src/vfs.rs` `list_dir` (backend filenames, verbatim from
    remote — e.g. Google Drive titles, SFTP/WebDAV/FTP names) and `download_name`
    (`vfs.rs:72`, appends an export extension for Google-Docs).
- CRATE (v0.0.6) `…/cloud-filter-0.0.6/src/`:
  - `placeholder_file.rs` (PlaceholderFile builder → `CF_PLACEHOLDER_CREATE_INFO`)
  - `metadata.rs` (`Metadata` → `CF_FS_METADATA`/`FILE_BASIC_INFO`)
  - `filter/ticket.rs` (`FetchPlaceholders::pass_with_placeholder`)
  - `filter/info.rs` (`FetchPlaceholders::pattern`)
  - `filter/proxy.rs` (dispatch of `CF_CALLBACK_TYPE_FETCH_PLACEHOLDERS`)
  - `command/commands.rs` (`CreatePlaceholders` → `CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS`)
  - `command/executor.rs` (`CfExecute`)
- MICROSOFT DOCS (URLs cited inline; all fetched 2026-06-16).

> **Headline finding:** the crate does **not** call `CfCreatePlaceholders` from the
> callback. `pass_with_placeholder` runs `CfExecute(CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS)`
> and **hard-codes `…FLAG_DISABLE_ON_DEMAND_POPULATION` with
> `PlaceholderTotalCount = placeholders.len()`** on every call
> (`command/commands.rs:175`, `:177`). This means **one `fetch_placeholders` call must
> return ALL children of the directory** — anything not returned in that single shot is
> silently lost (the platform marks the directory fully populated and never calls back
> again). Our code returns the entire `list_dir` result in one shot, so this is currently
> correct, but it is a latent landmine for large/paged directories. See ISSUE-1.

---

## 1. The documented `CF_PLACEHOLDER_CREATE_INFO` contract (quoted)

From <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_placeholder_create_info>:

```cpp
typedef struct CF_PLACEHOLDER_CREATE_INFO {
  LPCWSTR                     RelativeFileName;
  CF_FS_METADATA              FsMetadata;
  LPCVOID                     FileIdentity;
  DWORD                       FileIdentityLength;
  CF_PLACEHOLDER_CREATE_FLAGS Flags;
  HRESULT                     Result;
  USN                         CreateUsn;
} CF_PLACEHOLDER_CREATE_INFO;
```

| Field | Documented meaning (verbatim) |
|---|---|
| `RelativeFileName` | "The name of the child placeholder file or directory to be created. **It should consist only of the file or directory name.**" The example sets `BaseDirectoryPath = C:\SyncRoot\SubDirectory` and `RelativeFileName = placeholder.txt`. |
| `FsMetadata` | "File system metadata to be created with the placeholder, including all timestamps, file attributes and file size (optional for directories)." |
| `FileIdentity` | "A user mode buffer containing file information supplied by the sync provider. The *FileIdentity* blob **should not exceed `CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH` (defined to 4KB)** in size. *FileIdentity* gets passed back to the sync provider in all callbacks. **This is required for files** (not for directories)." |
| `FileIdentityLength` | "Length, in bytes, of the *FileIdentity*." |
| `Flags` | "Flags for specifying placeholder creation behavior." (see `CF_PLACEHOLDER_CREATE_FLAGS`) |
| `Result` | "The result of placeholder creation. On successful creation, the value is **STATUS_OK**." (per-entry HRESULT — see Q6) |
| `CreateUsn` | "The final USN value after create actions are performed." |

`CfCreatePlaceholders`
(<https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfcreateplaceholders>)
restates: "**FileIdentity** and **FileIdentityLength** describe a user mode buffer that
contains the opaque file information supplied by the sync provider. The **FileIdentity**
blob should not exceed **CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH** (defined to 4KB) in
size. … This is a mandatory field for files."

**`CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH` = 4 KB (4096 bytes).** (Doc says "defined to
4KB"; the crate names the same constant `CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH` and
asserts against it — `placeholder_file.rs:77,80`.)

---

## 2. What the crate actually sets for each field

`PlaceholderFile::new` (`placeholder_file.rs:20–31`) zero-initializes a
`CF_PLACEHOLDER_CREATE_INFO` (`..Default::default()`), then our builder chain overrides
fields. Net result per child in our `fetch_placeholders`:

| `CF_PLACEHOLDER_CREATE_INFO` field | Set by | Our value |
|---|---|---|
| `RelativeFileName` | `new(&display)` → `U16CString::from_os_str(...).into_raw()` (`placeholder_file.rs:22–25`) | the bare child name `display` (dir: `m.name`; file: `download_name(...)`, possibly with appended `.docx`/`.xlsx`). **Single component, no backslash.** |
| `FsMetadata` (`CF_FS_METADATA`) | `.metadata(md)` → `self.0.FsMetadata = metadata.0` (`placeholder_file.rs:67–70`) | from `Metadata::file().size(m.size)` or `Metadata::directory()`, `.created(now).written(now)`. |
| `FileIdentity` / `FileIdentityLength` | `.blob(child_remote.into_bytes())` (`placeholder_file.rs:78–97`) | leaked box of the **full remote path** UTF-8 bytes; length = its byte-len. (Blob empty → null ptr / len 0.) |
| `Flags` | `.mark_in_sync()` (`:47–50`) + files-only `.has_no_children()` (`:37–40`) | `MARK_IN_SYNC` (0x2) for all; `DISABLE_ON_DEMAND_POPULATION` (0x1) additionally for files. |
| `Result` | `new` sets `S_FALSE` (`:28`); platform overwrites on return | (in-out; see Q6) |
| `CreateUsn` | default 0; platform fills | — |

### `CF_FS_METADATA` / `FILE_BASIC_INFO` details (`metadata.rs`)

`Metadata::file()` (`metadata.rs:17–25`) sets `BasicInfo.FileAttributes =
FILE_ATTRIBUTE_NORMAL` (0x80); `Metadata::directory()` (`:28–36`) sets
`FILE_ATTRIBUTE_DIRECTORY` (0x10). `.size()` (`:63–66`) sets `FileSize` (i64).
`.created()`/`.written()` (`:39–54`) set `CreationTime`/`LastWriteTime`.

Fields we **leave at zero**: `LastAccessTime`, `ChangeTime`, and (for directories)
`FileSize`. We never call `.accessed()`, `.changed()`, or `.attributes()`.

### What actually executes (`pass_with_placeholder`)

`ticket.rs:148–154` → `command::CreatePlaceholders { total: len, placeholders }.execute(...)`.
`command/commands.rs:162–188` builds a `TransferPlaceholders` op and runs it via
`CfExecute` (`executor.rs:56–82`), op type `CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS`
(`:163`), with:

```rust
Flags: CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_DISABLE_ON_DEMAND_POPULATION,  // :175 (hard-coded!)
CompletionStatus: STATUS_SUCCESS,
PlaceholderTotalCount: self.total,        // = placeholders.len()           // :177
PlaceholderArray: <ptr or null if empty>,
PlaceholderCount: self.placeholders.len(),
EntriesProcessed: 0,
```

So this is the **TRANSFER_PLACEHOLDERS path**, not the standalone `CfCreatePlaceholders`
API. The crate's own comment flags the hazard (`commands.rs:173`):
"this flag tells the system there are no more placeholders in this directory (when that can
be untrue) … in the future, implement streaming."

---

## 3. Answers to the six critical questions

### Q1 — `RelativeFileName`: must it be a single path component? Invalid NTFS chars?

**Single component: YES, and we comply.** Doc: "It should consist only of the file or
directory name" with `BaseDirectoryPath` carrying the directory portion
(ns-cfapi-cf_placeholder_create_info). Our `display` is always a bare name — directories
use `m.name`, files use `download_name(...)` (`cfprovider.rs:115–119`), neither contains
`\`. **OK.**

**Invalid NTFS characters: REAL RISK (not handled).** `RelativeFileName` becomes a real
NTFS name, so it must obey NTFS naming rules. The crate does **zero** validation/escaping
(`placeholder_file.rs:20–31`); it would only panic if the name contained an interior NUL
(`U16CString::from_os_str(...).unwrap()` at `:24`). Our `m.name` comes verbatim from the
remote backend (`vfs.rs::list_dir`), and remote systems (Google Drive in particular, also
SFTP/WebDAV/FTP) permit characters NTFS forbids: `:`, `\`, `/`, `*`, `?`, `"`, `<`, `>`,
`|`, trailing dot/space, and reserved device names (CON, PRN, AUX, NUL, COM1…). When such a
name reaches the platform, `TRANSFER_PLACEHOLDERS` returns a per-entry failure such as
`STATUS_OBJECT_NAME_INVALID`/`0x80070057 E_INVALIDARG` for that entry (consistent with the
field contract and community reports —
<https://learn.microsoft.com/en-us/answers/questions/1157727/cfcreateplaceholder-error-0x80070057>).
A Google Drive name containing `/` is the most likely real-world trigger. See ISSUE-3.

### Q2 — `FileIdentity` / blob max length; full remote path risk

**Max = 4096 bytes (`CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH`, "defined to 4KB").** Our
blob = `child_remote.into_bytes()` = full remote path UTF-8 (`cfprovider.rs:104,123`).

**Hard failure mode: PANIC, not a graceful per-entry error.** `placeholder_file.rs:79–84`:

```rust
assert!(
    blob.len() <= CloudFilters::CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH as usize,
    "blob size must not exceed {} bytes, got {} bytes", ...
);
```

This `assert!` runs inside `.blob(...)`, i.e. inside our `fetch_placeholders` callback,
which the crate invokes from an `extern "system"` FFI callback (`proxy.rs:135`). A blob >
4096 bytes therefore **panics across the FFI boundary** — undefined behavior / likely abort,
not a recoverable error and not a skipped entry. Quantify: a path is over budget at >4096
UTF-8 bytes. Ordinary cloud paths are far under this, **but** deeply nested Google Drive
paths (long localized folder titles, emoji/CJK which are 3–4 bytes/char, many levels) can
realistically approach it; a single offending child aborts the whole enumeration (and
possibly the process). The blob is also redundant with the file's path for most backends —
`fetch_data` already falls back to `remote_of(path)` when the blob is empty
(`cfprovider.rs:65–69`). See ISSUE-2.

### Q3 — Timestamps set to NOW (not remote mtime); FileAttributes default

**Timestamps = fidelity bug, not a correctness/sync bug for our design.** We pass
`now = FileTime::now()` for both `created` and `written` (`cfprovider.rs:100,110–111`).
CfAPI does **not** use the placeholder's `LastWriteTime` as the cloud change-detection key;
change detection is the provider's responsibility (the provider decides in-sync state and
calls `CfUpdatePlaceholder`/`CfSetInSyncState`). The platform uses the in-sync bit + USN,
not write-time, to decide staleness. Our provider is browse-and-hydrate only (no upload
reconciliation that keys off mtime), so wrong timestamps don't break browse or hydrate.

Consequences are purely user-visible fidelity:
- Explorer shows the placeholder's "Date modified"/"Date created" as the moment of first
  enumeration, not the true remote times.
- Sort-by-date, "modified since," and incremental-backup tools that trust mtime are misled.
- `LastAccessTime` and `ChangeTime` are left **0** (we never set them). A zero `ChangeTime`
  is benign here (see RestartHydration semantics: 0 = "no change"; for create it just yields
  an epoch/unset value Explorer may render oddly).

If `VfsMeta` carries the real remote mtime, pass it via `.written(real_mtime)` /
`.created(real_ctime)`. See ISSUE-5 (FIDELITY).

**FileAttributes default — required, and the crate supplies it.** We never call
`.attributes()`, but `Metadata::file()` defaults `FileAttributes = FILE_ATTRIBUTE_NORMAL`
(0x80) and `Metadata::directory()` defaults `FILE_ATTRIBUTE_DIRECTORY` (0x10)
(`metadata.rs:21,30`). A `FILE_ATTRIBUTE_*` value **is** required — most importantly the
directory bit, without which the entry would not be a directory and would not populate. The
platform itself adds the placeholder/pinned attributes (`FILE_ATTRIBUTE_PINNED` /
`RECALL_ON_*`). So defaults are correct. **OK.** (Note: `FILE_ATTRIBUTE_NORMAL` is only
valid when used alone; since the platform ORs in its own placeholder attributes afterward,
this is fine for create.)

### Q4 — `mark_in_sync()` on a freshly-created DEHYDRATED placeholder — correct?

**Correct.** "In-sync" describes whether the placeholder's **metadata** matches the cloud,
independent of whether the **data** is hydrated. A dehydrated (online-only) placeholder is
the *normal* in-sync state. Evidence:

- `CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC`: "The newly created placeholder is marked as
  in-sync as part of the TRANSFER_PLACEHOLDERS operation. **This is applicable to both
  placeholder files and directories.**"
  (<https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_placeholder_create_flags>)
- `CfConvertToPlaceholder` exposes `CF_CONVERT_FLAG_MARK_IN_SYNC` and
  `CF_CONVERT_FLAG_DEHYDRATE` as **independent** flags — you can (and routinely do) mark a
  file in-sync *and* dehydrate it. (nf-cfapi-cfconverttoplaceholder)
- Dehydration is only **permitted** on an in-sync placeholder: requesting dehydration on a
  not-in-sync file fails `ERROR_CLOUD_FILE_NOT_IN_SYNC`. So freshly created online-only
  placeholders **must** be in-sync to behave like OneDrive online-only files.
- Crate doc (`placeholder_file.rs:42–50`) links the same concept (SetInSyncState; "What does
  In-Sync Mean?").

If we did **not** set it, every placeholder would show the "sync pending"/not-in-sync
overlay and the OS could try to re-sync them. Setting it on dehydrated placeholders is the
intended pattern. **OK.**

### Q5 — `has_no_children()` on files only — which flag, and is the split correct?

`has_no_children()` sets **`CF_PLACEHOLDER_CREATE_FLAG_DISABLE_ON_DEMAND_POPULATION`**
(0x1) (`placeholder_file.rs:37–40`).

Doc: "This flag is applicable for a child placeholder **directory only**. When the flag is
present, the newly created child placeholder directory is considered to have all of its
children present locally hence accessing it in the future will **not trigger** any
FETCH_PLACEHOLDERS callback on it. When the flag is **absent**, the newly created
placeholder directory is considered partial and **future access will trigger
FETCH_PLACEHOLDERS**." (ne-cfapi-cf_placeholder_create_flags; identical text in
ne-cfapi-cf_placeholder_create_info under `DISABLE_ON_DEMAND_POPULATION`).

**The intent of our split is correct; the per-flag mechanics are a no-op-but-harmless on the
file side:**
- **Directories** (we do NOT set it): correct — they remain *partial*, so enumerating them
  later triggers `FETCH_PLACEHOLDERS` and lazily populates the next level. This is exactly
  what `populate_to` relies on. **Confirmed: directories without the flag WILL trigger
  FETCH_PLACEHOLDERS on enumeration.**
- **Files** (we DO set it): the flag "is applicable for a child placeholder **directory
  only**." On a file placeholder it has no defined effect — files never receive
  FETCH_PLACEHOLDERS regardless (that callback is about *directory* contents). So setting it
  on files is **semantically a no-op**, neither required nor harmful. The naming
  (`has_no_children` / "no child placeholders") is what motivated the file-only call, but it
  buys nothing. **OK** (harmless), flagged as low-severity tidy-up — ISSUE-6.

> Subtlety (interacts with the headline finding): the *operation-level*
> `CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_DISABLE_ON_DEMAND_POPULATION` is **always** set by
> the crate at the batch level (`commands.rs:175`), which marks the **enumerated directory
> itself** fully populated. That is the flag that actually stops repeat callbacks for the
> current directory — and it fires regardless of the per-file `has_no_children()`. The
> per-placeholder file flag in Q5 is a different, directory-only flag and is independent.

### Q6 — Does `pass_with_placeholder` report per-file errors? One bad child → whole batch?

**The crate surfaces only the operation-level status, NOT per-entry results.**

- The execution path returns `()` on success and discards per-entry data:
  `CreatePlaceholders::result` is `unsafe fn result(_) -> () {}` (`commands.rs:168`), and
  `pass_with_placeholder` returns `core::Result<()>` from the `CfExecute` HRESULT only
  (`ticket.rs:148–154`, `executor.rs:56–82`). The crate never reads back each
  `CF_PLACEHOLDER_CREATE_INFO::Result` HRESULT (the `PlaceholderFile::result()` accessor at
  `placeholder_file.rs:99–101` exists but is only used by the standalone `create()` path,
  not by `pass_with_placeholder`).
- **Batch semantics (platform):** the operation does **not** use STOP_ON_ERROR (the crate
  sets `…FLAG_DISABLE_ON_DEMAND_POPULATION` only; `STOP_ON_ERROR` = 0x1 is absent —
  `commands.rs:175`, ne-cfapi-cf_operation_transfer_placeholders_flags). For the analogous
  `CfCreatePlaceholders`, `CF_CREATE_FLAG_NONE` is "the default mode where the API processes
  **all entries** in the array even when errors are encountered," and "the API returns the
  first failure code encountered, but **continues processing as many entries as possible;
  the caller must then inspect the array** to see which placeholder creation(s) failed"
  (nf-cfapi-cfcreateplaceholders). So at the platform level **one bad child does NOT abort
  the others** — good children are still created; the bad one's per-entry `Result` carries
  its HRESULT.
- **But the crate throws that detail away.** `pass_with_placeholder` returns the single
  top-level HRESULT. So in our code: if child N has a bad name, the other children are still
  created by the platform, but our `map_err(cerr)` (`cfprovider.rs:129`) sees the
  first-failure HRESULT and we report `CloudErrorKind::Unsuccessful` for the *whole*
  callback. We can neither tell *which* child failed nor that the rest succeeded.
- **Worse for too-long blobs:** that case never reaches the platform at all — it panics in
  `.blob()` before `pass_with_placeholder` is even called (Q2). So a too-long blob fails the
  *entire* enumeration hard, not "just that entry."

Net: **per-entry success is invisible to us; a too-long blob is fatal to the batch; a
bad-name entry is skipped by the platform but mis-reported by us as a whole-callback
failure.** See ISSUE-3, ISSUE-4.

---

## 4. On-demand population: is `populate_to` valid?

**Yes — `read_dir` on each ancestor is a valid and idiomatic way to force placeholder
creation, and the enumeration blocks until we return.**

- `CF_CALLBACK_TYPE_FETCH_PLACEHOLDERS`: "This callback is used to ask the sync provider to
  provide information about the contents of a placeholder directory **to satisfy a directory
  query operation or an attempt to open a file underneath the directory**."
  (ne-cfapi-cf_callback_type)
- A partial placeholder directory (created without `DISABLE_ON_DEMAND_POPULATION`) triggers
  the callback on access: "When the flag is absent, the newly created placeholder directory
  is considered partial and **future access will trigger FETCH_PLACEHOLDERS**."
  (ne-cfapi-cf_placeholder_create_flags)
- **Blocking:** the request that triggers the callback is *pended* and the directory query
  cannot complete until the provider transfers placeholders. The platform docs state "All
  callbacks are asynchronous. Asynchronous user requests that trigger the callbacks are
  pended" with "a fixed 60 second timeout" (ne-cfapi-cf_callback_type). For a synchronous
  `FindFirstFile`/`std::fs::read_dir` the calling thread blocks on that pended I/O until the
  provider's `TRANSFER_PLACEHOLDERS` (our `pass_with_placeholder`) completes or the 60s
  timeout fires. The `Pattern` field is "a standard Windows file pattern … Often the pattern
  will be `*`"; the provider "is expected to begin transferring placeholder information for
  all files in the directory" (ns-cfapi-cf_callback_parameters). We ignore `Pattern`
  (`info.rs:129–146` exposes it; `cfprovider.rs` doesn't read it) and return everything,
  which is explicitly allowed ("may additionally choose to transfer placeholders not
  matching the pattern").

So `populate_to` (`cfprovider.rs:142–160`) draining `read_dir` from the sync root down to
the target's parent forces each level's `fetch_placeholders` synchronously, so by the time
it returns the leaf placeholder exists on disk and can be opened/hydrated. **Confirmed
valid.** (Caveat: each level must complete within the 60s callback timeout — fine for
normal directories; a slow backend listing a huge directory could time out. And because of
the headline finding, each level is populated in a single shot and then marked complete.)

---

## 5. ISSUES (severity-tagged)

### ISSUE-1 — RISK — `fetch_placeholders` must return ALL children in one shot
The crate hard-codes `CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_DISABLE_ON_DEMAND_POPULATION`
with `PlaceholderTotalCount = placeholders.len()` on every `pass_with_placeholder`
(`command/commands.rs:175,177`). Per
ne-cfapi-cf_operation_transfer_placeholders_flags, this "Disables on-demand population for
the directory, preventing further CF_CALLBACK_TYPE_FETCH_PLACEHOLDERS callbacks" and marks
the directory fully populated. **Consequence:** if `list_dir` is ever paged, filtered, or
errors mid-stream and we return a partial set, the remaining children are **permanently
invisible** (no further callback). Today `cfprovider.rs:99,103–128` returns the full
`list_dir` result in one shot, so behavior is correct — but it is fragile: any future
streaming/pagination/early-return breaks silently. The crate even documents this as a known
limitation (`commands.rs:173` "when that can be untrue … in the future, implement
streaming"). Mitigation: keep returning the complete listing in one call; if a directory can
be huge or paged, this crate version cannot express "more to come" and would need a patch.

### ISSUE-2 — RISK (BUG under adversarial input) — too-long blob panics across FFI
`.blob()` `assert!`s `blob.len() <= 4096` (`placeholder_file.rs:79–84`) **inside** the
`fetch_placeholders` callback, which runs from an `extern "system"` FFI trampoline
(`proxy.rs:135`). A remote path > 4096 UTF-8 bytes (plausible for deep Google Drive trees
with long/CJK/emoji folder names) **panics across the FFI boundary** → abort/UB, taking down
the enumeration and possibly the process. Max is `CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH`
= 4096 bytes (doc + `placeholder_file.rs:80`). Mitigation: bound/guard the blob before
calling `.blob()` (e.g. skip the child, or store a short key/ID instead of the full path and
resolve it in `fetch_data`). Note `fetch_data` already tolerates an empty blob by falling
back to `remote_of(path)` (`cfprovider.rs:65–69`), so for path-addressable backends the blob
is largely redundant and could be dropped or shortened.

### ISSUE-3 — RISK — names with NTFS-invalid characters fail the entry (and we mis-report)
`RelativeFileName = display` is the raw remote name (`cfprovider.rs:115–119`,
`vfs.rs::list_dir`). Remote names can contain `: \ / * ? " < > |`, trailing dot/space, or
reserved device names — all illegal on NTFS — with Google Drive `/` in titles the most
likely trigger. The crate does no validation/escaping (`placeholder_file.rs:20–31`); the
platform fails that entry (e.g. `STATUS_OBJECT_NAME_INVALID` / `0x80070057`,
<https://learn.microsoft.com/en-us/answers/questions/1157727/cfcreateplaceholder-error-0x80070057>).
Because of ISSUE-4 we then report the whole callback as failed. (Also: an interior NUL in a
name would panic at `placeholder_file.rs:24` `.unwrap()`.) Mitigation: sanitize/escape names
to a reversible NTFS-safe form for the placeholder display name while keeping the true name
in the blob/mapping for `fetch_data`.

### ISSUE-4 — RISK — per-entry results discarded; whole callback fails on any one bad child
`pass_with_placeholder` returns only the top-level `CfExecute` HRESULT and ignores each
entry's `Result` HRESULT (`ticket.rs:148–154`; `CreatePlaceholders::result` is a no-op
`commands.rs:168`). The platform actually creates the good children and reports first-failure
while continuing (`CF_CREATE_FLAG_NONE` semantics, nf-cfapi-cfcreateplaceholders), but our
`map_err(cerr)` (`cfprovider.rs:129`) collapses that into a single
`CloudErrorKind::Unsuccessful` for the entire enumeration — we lose which child failed and
the fact that others succeeded. Mitigation: after `pass_with_placeholder`, inspect each
`PlaceholderFile::result()` (the accessor exists at `placeholder_file.rs:99`) — but the crate
doesn't expose it on this path, so this likely needs a crate patch or pre-validation of names
(ISSUE-3) and blob sizes (ISSUE-2) before calling.

### ISSUE-5 — FIDELITY — timestamps are NOW, not the real remote mtime; access/change unset
`created`/`written` = `FileTime::now()` (`cfprovider.rs:100,110–111`); `LastAccessTime` and
`ChangeTime` left 0. Not a sync-correctness bug (CfAPI keys staleness off the in-sync bit +
USN, not write-time, and our provider doesn't reconcile uploads by mtime), but Explorer
shows wrong dates and date-sort / "modified since" / mtime-based backup tools are misled.
Mitigation: thread the real remote times from `VfsMeta` into
`.created(real_ctime).written(real_mtime)` (and optionally `.accessed()`/`.changed()`).

### ISSUE-6 — OK / minor cleanup — `has_no_children()` on files is a no-op
`DISABLE_ON_DEMAND_POPULATION` is "applicable for a child placeholder **directory only**"
(ne-cfapi-cf_placeholder_create_flags). Setting it on file placeholders
(`cfprovider.rs:124–126`) has no defined effect and can be dropped for clarity. Harmless
today. (The directory side correctly omits it so directories stay partial and populate on
demand — which is the behavior `populate_to` depends on.)

### OK — confirmed correct
- `RelativeFileName` is a single component, no backslash (Q1). **OK.**
- `mark_in_sync()` on dehydrated placeholders is the intended pattern; in-sync = metadata
  matches cloud, independent of hydration (Q4). **OK.**
- Default `FileAttributes` (`FILE_ATTRIBUTE_NORMAL` / `FILE_ATTRIBUTE_DIRECTORY`) are
  supplied by the crate and are sufficient/required; the platform adds placeholder
  attributes (Q3). **OK.**
- Directories created **without** `DISABLE_ON_DEMAND_POPULATION` correctly trigger
  `FETCH_PLACEHOLDERS` on enumeration; `populate_to` via `read_dir` is a valid, blocking
  trigger (Q5, §4). **OK.**

---

## 6. Citations index

- CF_PLACEHOLDER_CREATE_INFO — <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_placeholder_create_info>
- CfCreatePlaceholders — <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfcreateplaceholders>
- CF_PLACEHOLDER_CREATE_FLAGS — <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_placeholder_create_flags>
- CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAGS — <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_operation_transfer_placeholders_flags>
- CF_OPERATION_PARAMETERS (TransferPlaceholders) — <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_operation_parameters>
- CF_CALLBACK_TYPE (FETCH_PLACEHOLDERS) — <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_callback_type>
- CF_CALLBACK_PARAMETERS (FetchPlaceholders.Pattern) — <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_callback_parameters>
- CfConvertToPlaceholder (MARK_IN_SYNC vs DEHYDRATE) — <https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfconverttoplaceholder>
- 0x80070057 on CfCreatePlaceholders (bad name) — <https://learn.microsoft.com/en-us/answers/questions/1157727/cfcreateplaceholder-error-0x80070057>
- Repeated FETCH_PLACEHOLDERS w/o DISABLE flag — <https://learn.microsoft.com/en-us/answers/questions/67054/cfapi-cf-callback-type-fetch-placeholders-calls-ba>

Crate v0.0.6 line refs: `placeholder_file.rs:20–31,37–40,47–50,67–70,78–97,99–101`;
`metadata.rs:17–36,39–66`; `filter/ticket.rs:129–155`; `filter/info.rs:129–146`;
`filter/proxy.rs:135–155`; `command/commands.rs:153–212` (esp. `:163,168,175,177`);
`command/executor.rs:56–82`.
