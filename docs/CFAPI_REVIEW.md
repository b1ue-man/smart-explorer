# CfAPI Implementation — Full Doc-Grounded Review

> **Historical review:** this document audits a CfAPI provider implementation
> that existed during the 2026-06-16 investigation. Current `native/src` no
> longer contains `cfprovider.rs` or `cfsync.rs`; the active remote-open path is
> temp-copy + mtime watch + `Backend::open_write` save-back. Treat the issue
> register below as the evidence for why reviving a native Cloud Files provider
> must be a new, safety-fixed feature, not as current shipped behavior.

Historical scope: every Cloud Files API (CfAPI) call that provider code made,
traced to the letter
(inputs, provenance, data types, byte destinations, outputs), each cross-checked
against the **`cloud-filter` v0.0.6** crate source
(`/root/.cargo/registry/.../cloud-filter-0.0.6/src`) and **Microsoft CfAPI /
StorageProvider docs** (learn.microsoft.com). Produced by eight parallel agent
investigations (raw notes in `docs/cfapi_review/w1_*.md`, `w2_*.md`) plus direct
source verification. Date: 2026-06-16.

At the time of this review, the CfAPI surface lived in:
- `native/src/cfprovider.rs` — the `SyncFilter` provider + `ensure_mounted` + `populate_to` (Windows-only).
- `native/src/cfsync.rs` — local↔remote path mapping (`local_path`, `local_path_named`, `conn_root_dir`, `san`).
- `native/src/app.rs` — the open flow (`open_file` CfApi branch ~3096), `RemoteEdit` save-back watch, mode toggle.
- `native/Cargo.toml` — `cloud-filter = "0.0.6"`, `nt-time`, `windows` features.

---

## 0. Verdict (read this first)

**The open path is now *mostly* correct for plain binary files, but the feature
has three classes of defect — two that crash/corrupt (must-fix safety) and one
that is architectural (cannot be fully fixed with this crate's defaults).**

- **OPEN works only for the happy path:** a plain binary file, ASCII name, path
  under 4 KiB, opened by an app that reads the whole file. Outside that, it
  fails (often silently or, worse, with undefined behavior).
- **SAVE-BACK is fundamentally broken for most editors:** the common
  "write temp → rename over original" atomic-save (Word, Excel, VS Code, Win11
  Notepad) raises `CF_CALLBACK_TYPE_NOTIFY_RENAME`, which the crate defaults to
  `Err(NotSupported)` — the OS blocks/poisons the save. We never implement
  rename/delete callbacks. **This alone makes CfAPI editing unreliable.**
- **Google-Docs are broken** end to end (empty on open, un-importable on save).
- **Lifecycle:** the provider only exists while the GUI runs; placeholders can't
  hydrate when the app is closed, and uninstall orphans the registration.

This validates the recommendation to make the **persistent-mirror/Temp mode the
default** and treat CfAPI as experimental. The mirror approach sidesteps *every*
issue below (no rename callback, no size/alignment contract, no 4 KiB blob, no
NTFS-name panic, works with the app closed, handles Google-Docs export
naturally). See §5.

---

## 1. The "web" — end-to-end data-flow trace

Legend: `name : type — provenance → destination`. File:line are current as of
this commit. Full 20-step trace with the data-type table is in
`docs/cfapi_review/w2_web_trace.md`.

### 1a. Open (user double-clicks a remote file)

1. `App::open_file(path, name)` `app.rs:3079`
   - `path : String` — the remote path of the entry (e.g. `/Reports/Q3.xlsx`), from the file list.
   - `name : String` — display name. `self.remote.backend : BackendHandle`, `self.root_path : String` (connection root).
2. `cfprovider::ensure_mounted(label, backend, root_path)` `app.rs:3097 → cfprovider.rs:178`
   - `label : &str` = connection label; `remote_root : &str` = `root_path.trim_end_matches('/')`.
   - → `local_root : PathBuf` = `cfsync::conn_root_dir(label)` = `%USERPROFILE%\Smart Explorer\<san(label)>`.
   - Registers (once) + connects a CfAPI sync root; stores `Connection<RemoteFilter>` in a process-static registry (kept alive = required, see §3 lifetime).
3. `local_path_named(label, root_path, path, download_name(path,name))` `app.rs:3103 → cfsync.rs:46`
   - → `dest : PathBuf` = `local_root\<san(seg)>\…\<san(leaf)>`. **NOTE:** every segment is `san()`-sanitized here.
4. `populate_to(local_root, dest)` `cfprovider.rs:142`
   - `std::fs::read_dir` each ancestor dir top-down → forces the OS to fire `fetch_placeholders` per level so the leaf placeholder materializes.
5. `open_path(dest)` `app.rs:3120 → ShellExecuteW` — opens the placeholder in its default app.
6. Register a `RemoteEdit { temp: dest, remote_path: path, baseline_mtime: i64::MAX, … }` for mtime-watch save-back.

### 1b. fetch_placeholders (OS → provider, on directory enumeration)

`cfprovider.rs:92`. Input `request : Request` (its `path()` = the local dir being enumerated).
- `remote_dir = remote_of(request.path())` `cfprovider.rs:98` — strips `local_root`, prepends `remote_root`. **No `san()` reversal** (asymmetry with step 3 — see ISSUE-OPEN-2).
- `backend.list_dir(remote_dir)` → `Vec<VfsMeta>`.
- Per child: `PlaceholderFile::new(display)` where `display = download_name(child_remote, name)` (no `san()`); `.mark_in_sync()`; `.metadata(Metadata::file().size(m.size).created(now).written(now))`; `.blob(child_remote.into_bytes())` (full remote path); files also `.has_no_children()`.
- `ticket.pass_with_placeholder(&mut placeholders)` → `CfExecute(TRANSFER_PLACEHOLDERS)`.

### 1c. fetch_data (OS → provider, on first read of a placeholder)

`cfprovider.rs:58`. Inputs `request`, `ticket : FetchData`, `info : FetchData`.
- `blob = request.file_blob()` → remote path bytes we stored; if empty, `remote_of(request.path())`.
- `range = info.required_file_range() : Range<u64>`.
- `backend.open_read(remote)` → `Box<dyn Read>` (stream from offset 0).
- skip `range.start` bytes via 8 KiB sink loop; `r.take(len).read_to_end(&mut buf)`; `ticket.write_at(&buf, range.start)` → `CfExecute(TRANSFER_DATA, Buffer=buf.ptr, Offset=range.start, Length=buf.len())`.

### 1d. Save-back (independent of CfAPI)

`poll_remote_edits` `app.rs:3226` polls the hydrated file's mtime; on a stable change → `upload_file(backend, dest, remote_path)`. No CfAPI callback is involved in detecting the edit — **but the editor's save itself goes through CfAPI rename/delete callbacks we don't implement (see ISSUE-SAVE-1).**

### 1e. Data-type / boundary table (condensed)

| Value | Rust type | Lowers to (Win32) | Encoding / note |
|------|-----------|-------------------|------------------|
| placeholder name | `&str`→`PlaceholderFile::new` | `PCWSTR` via `U16CString::from_os_str().unwrap()` | UTF-16; **panics on interior NUL** |
| blob (remote path) | `Vec<u8>` | `FileIdentity : *const u8` + `FileIdentityLength` | raw bytes; **`assert! ≤ 4096`** |
| size | `u64`→`Metadata::size` | `CF_FS_METADATA.FileSize : i64` | logical placeholder size |
| created/written | `nt_time::FileTime` | `FILE_BASIC_INFO.{CreationTime,LastWriteTime}` | 100-ns NT ticks |
| required range | — | `CF_CALLBACK_PARAMETERS.FetchData.{RequiredFileOffset,RequiredLength}` | `i64`→`u64` (overflow-add in crate) |
| transfer | `&[u8]`,`u64`→`write_at` | `CF_OPERATION_PARAMETERS.TransferData.{Buffer,Offset,Length}` | **Offset must be 4 KiB-aligned; Length 4 KiB-aligned unless ending at EoF** |

---

## 2. Per-call validation (does each call meet the documented contract?)

| Call (file:line) | Contract (crate / MS doc) | Verdict |
|---|---|---|
| `SecurityId::current_user()` `cfprovider.rs:190` | returns current-user SID for the sync-root Id | **OK** |
| `SyncRootIdBuilder::new(pid)` `:192` | asserts `pid.len() ≤ 255` (`CF_MAX_PROVIDER_NAME_LENGTH`) else **panics** (`sync_root_id.rs:62`) | **RISK** (panic escapes `ensure_mounted`'s `Result`; our `pid` is unbounded) |
| assembled `SyncRootId` | MS: total Id ≤ **174** chars or `ERROR_INSUFFICIENT_BUFFER` | **RISK** (long label passes the 255 assert, fails `Register`) |
| `is_registered()` `:193` | reads WinRT registration | **OK** |
| `SyncRootInfo::with_*` `:201-207` | `with_path`/`with_recycle_bin_uri` return `Result`; the rest `.unwrap()` internally | **OK** (current-time/static values don't fail) |
| `register(info)` `:199` | requires non-empty `display_name`, `icon`, `version`, `path` (`sync_root_id.rs:160`) | **OK** (all set since 0.5.33) |
| `with_icon("…shell32.dll,4")` `:203` | "module,resource" string | **OK** (positive index = valid; negative resource-id is version-stabler — FIDELITY) |
| `with_hydration_type(Full)` `:204` | fetch whole file on access via FETCH_DATA | **OK** |
| `with_population_type(Full)` `:205` | MS: *"if not fully populated, the platform will request the provider populate them"* → on-demand FETCH_PLACEHOLDERS | **OK** (this is why `populate_to` works) |
| `unregister()` on connect-fail `:222` | removes the WinRT registration | **RISK** (fires even when the root *pre-existed* — can deregister a prior/parallel mount) |
| `Session::new().connect(local_root, filter)` `:217` | non-blocking; callbacks on arbitrary threads; `Connection` Drop = `CfDisconnectSyncRoot` | **OK** mechanically; lifetime is load-bearing (§3) |
| `PlaceholderFile::new(display)` `:120` | `U16CString::from_os_str().unwrap()` | **BUG/UB** on interior-NUL name (no `catch_unwind` in proxies) |
| `.metadata(.size(m.size))` `:108` | sets logical `FileSize` | **BUG** when `size` ≠ bytes delivered (Google-Docs `size`=0 → empty file) |
| `.created(now).written(now)` `:110` | NT timestamps | **FIDELITY** (should be real `mtime_ms`/`btime_ms`) |
| `.blob(child_remote)` `:123` | `assert! len ≤ 4096` (`placeholder_file.rs:79`) | **BUG/UB** for remote paths > 4 KiB |
| `.has_no_children()` on files `:125` | sets `DISABLE_ON_DEMAND_POPULATION` — "directories only" | **OK/minor** (no-op on files; intent inverted) |
| `pass_with_placeholder` `:129` | hardcodes `DISABLE_ON_DEMAND_POPULATION` + `PlaceholderTotalCount=len`; discards per-item `Result` | **RISK** (one call must return *all* children; per-entry errors swallowed) |
| `fetch_data … write_at(buf, range.start)` `:88` | Offset 4 KiB-aligned always; Length 4 KiB-aligned unless EoF (`CF_OPERATION_PARAMETERS`) | **BUG** (we serve raw `required_file_range`; only full-file/EoF reads happen to satisfy it) |
| `populate_to` read_dir walk `:142` | enumeration triggers FETCH_PLACEHOLDERS synchronously | **OK** (valid technique) but swallows backend errors → silent (§4) |

---

## 3. Connection lifetime & threading (verified vs source)

- The callback context is a `Weak<F>`; the only strong `Arc<F>` is `Connection.filter`. **Dropping the `Connection` calls `CfDisconnectSyncRoot` and all future callbacks silently no-op** (`root/connect.rs`, `root/session.rs`). Our process-static `registry()` (`cfprovider.rs:163`) correctly keeps it alive **for the GUI process lifetime only**.
- `connect` is non-blocking; callbacks run on **arbitrary CfAPI worker threads** → our `Backend` is `Send + Sync` (✓). **But any panic in a callback unwinds across `extern "system"` (no `catch_unwind` in `proxy.rs`) = UB.** This makes the blob/NUL asserts (above) crash-class, not just error-class.
- **Consequence:** hydration only works while Smart Explorer runs. Open a placeholder from Explorer with the app closed → the OS has no provider → operation fails (`CfDisconnectSyncRoot` docs: *"the platform will fail any operation that depends on said callbacks"*). We do **not** reconnect known roots at startup, and `--sync-daemon` does **not** serve CfAPI.

---

## 4. Issue register (ranked)

Severity: **UB** (memory-unsafe / crash) > **BUG** (wrong result) > **RISK** (fragile) > **FIDELITY** (cosmetic).

### Must-fix safety (no-regret regardless of strategy)
- **I1 (UB)** blob > 4 KiB → `assert!` panic across FFI. *Fix:* guard length, empty-blob fallback. `cfprovider.rs:123`.
- **I2 (UB)** interior-NUL name → `new().unwrap()` panic across FFI. *Fix:* strip NUL from display name. `cfprovider.rs:120`.
- **I3 (BUG)** `fetch_data` unaligned transfer → partial/mapped reads rejected `0x8007017C`. *Fix:* align Offset down to 4 KiB, Length up to 4 KiB (EoF-exempt tail). `cfprovider.rs:88`.
- **I4 (BUG)** Google-Docs `size`=0 → empty placeholder → opens blank; export not size-knowable. *Fix:* route transformed files (`download_name != name`) to Temp mode on open. `app.rs:3096`.

### Correctness / robustness
- **I5 (BUG)** open-side `san()` (cfsync) vs callback-side no-sanitization (display + `remote_of`) → any special-char name → `ShellExecute` a leaf the provider never created → silent failure. *Fix:* sanitize the placeholder display name with the same `san()`; rely on the blob (true path) for `fetch_data`. `cfprovider.rs:118` vs `cfsync.rs:46`.
- **I6 (BUG, lifecycle)** atomic-save `NOTIFY_RENAME`/`NOTIFY_DELETE` default to `Err(NotSupported)` → editor saves blocked/poisoned. *Fix:* implement rename/delete callbacks that ack; this is **non-trivial** and is the strongest argument against CfAPI editing. `proxy.rs`/`sync_filter.rs`.
- **I7 (RISK)** `provider_id` not injective + unbounded length → SyncRootId/local_root collisions across connections + `new()` panic + 174-cap. *Fix:* `SmartExplorer_<short-hash(label)>`. `cfprovider.rs:168`.
- **I8 (RISK)** `unregister()` on connect-fail even when root pre-existed. *Fix:* only unregister if *this call* registered it. `cfprovider.rs:222`.
- **I9 (RISK)** uninstall leaves CfAPI roots registered (the `-2145452027` hazard). *Fix:* unregister all `SmartExplorer_*` roots in `--unregister`. `main.rs`/`shell_register.rs`.
- **I10 (RISK)** sentinel `baseline_mtime=i64::MAX` can mistake the hydration write for a user edit and re-upload (no "hydration complete" signal). `app.rs:3243`.
- **I11 (RISK)** `populate_to` swallows backend errors → still `ShellExecute`s → silent failure resurfaces if listing fails. *Fix:* surface a populate error (already partially done via `dest.exists()` check). `cfprovider.rs:142`.

### Fidelity / minor
- **I12 (FIDELITY)** placeholder timestamps = now, not real mtime. *Fix:* use `m.mtime_ms`/`btime_ms`.
- **I13 (FIDELITY)** no `CfSetInSyncState` after save-back → permanent "pending" overlay.
- **I14 (BUG, Google-Docs)** save-back uploads `.docx` to the raw Doc id — no inverse import. (Subsumed by I4: route exports to Temp.)
- **I15 (RISK)** `remote_edits` 50-entry cap silently drops save-back watches; never shrinks.
- **I16 (RISK)** No `longPathAware` manifest (no `.rc`/`build.rs`) → remote trees whose local placeholder path exceeds `MAX_PATH` (260) silently fail population. Separate from the 4 KiB blob limit (I1). *Fix:* ship a long-path-aware manifest, or cap depth.

---

## 5. Strategic conclusion (for the mirror-vs-CfAPI decision)

Every defect above is **absent** in the persistent-mirror/Temp approach:

| Concern | CfAPI | Mirror/Temp |
|---|---|---|
| Open a normal file | ✓ (after I1–I3) | ✓ |
| Partial/mapped reads | needs alignment fix | ✓ (real file) |
| Google-Docs export | broken (I4/I14) | ✓ (download exported bytes) |
| Atomic save (Word/VS Code) | **blocked** (I6) | ✓ (plain file) |
| Deep paths / odd names | UB/silent (I1,I2,I5) | ✓ |
| Works with app closed | ✗ (I-lifetime) | ✓ (file already on disk) |
| Uninstall cleanliness | orphans roots (I9) | ✓ (just files) |
| Implementation/maintenance | deep, untestable here | simple, debuggable |

**Historical recommendation:** make mirror/Temp the default; keep any revived
CfAPI path explicitly experimental and only after the safety fixes (I1–I4,
I7–I9) are applied so it can never crash the app. The current tree follows the
mirror/temp direction and has no active CfAPI toggle. Whole-tree on-demand
placeholders (CfAPI's one genuine advantage) only matter for browsing huge
remote trees of large files — not the edit-a-file workflow driving this.

---

## 6. Source index

Raw, fully-cited investigations:
`docs/cfapi_review/w1_registration.md`, `w1_fetchdata.md`, `w1_placeholders.md`,
`w1_crate_map.md`, `w2_web_trace.md`, `w2_redteam.md`, `w2_lifecycle.md`,
`w2_verify.md`. Crate source: `cloud-filter` v0.0.6. MS docs: `CF_OPERATION_PARAMETERS`,
`CF_PLACEHOLDER_CREATE_INFO`, `CF_CALLBACK_TYPE`, `CfDisconnectSyncRoot`,
`StorageProviderSyncRootInfo`, `StorageProviderSyncRootManager.Register`.
