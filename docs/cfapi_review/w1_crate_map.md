# W1 — `cloud-filter` v0.0.6 Crate-Internals Map

Audit reference for Smart Explorer's CfAPI sync root. Every API below is one our
code (`native/src/cfprovider.rs`) actually calls. For each: signature, what it
takes/returns, what happens internally, the ultimate Win32 / WinRT
`StorageProvider` call, and preconditions/safety/lifetime notes. All
`file:line` citations are into the crate source under:

```
/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cloud-filter-0.0.6/src/
```

Severity tags used in the "differences" section: **BUG** (will misbehave),
**RISK** (works today but fragile / depends on undocumented behavior), **OK**
(intentional, safe difference).

---

## 0. Big picture of the data flow

- Registration is done through **WinRT** `Windows.Storage.Provider.StorageProviderSyncRootManager` (UWP API), not the Win32 `CfRegisterSyncRoot`. See `root/sync_root_info.rs` and `root/sync_root_id.rs`.
- Connecting, callbacks, and all per-file operations are done through **Win32 `cfapi.h`** (`CfConnectSyncRoot`, `CfExecute`, `CfCreatePlaceholders`, `CfDisconnectSyncRoot`, `CfReportProviderProgress`).
- The crate is a thin, mostly-`unwrap()` wrapper. Most WinRT setter calls `.unwrap()` internally, so a bad value **panics** rather than returning `Err`. Field emptiness is the only thing validated before `Register`.

---

## 1. `SecurityId::current_user()`

- **Signature** (`root/sync_root_id.rs:256`): `pub fn current_user() -> core::Result<SecurityId>`.
- **Type**: `SecurityId` wraps a `U16String` (`sync_root_id.rs:234`). Returns `windows::core::Result`.
- **Internals** (`sync_root_id.rs:256-295`): all `unsafe`.
  1. `GetTokenInformation(CURRENT_THREAD_EFFECTIVE_TOKEN, TokenUser, None, 0, &size)` to size the buffer; tolerates `ERROR_INSUFFICIENT_BUFFER` (`:269-273`).
  2. Allocates a `Vec<MaybeUninit<u8>>`, calls `GetTokenInformation` again to fill it (`:278-284`).
  3. Casts to `TOKEN_USER`, calls `ConvertSidToStringSidW(token_user.User.Sid, &mut sid)` (`:286-288`).
  4. Copies the PWSTR into an `OsString`, then `LocalFree`s the SID buffer (`:290-291`).
  5. `SecurityId::new(string_sid)` — which **asserts** the string has no `!` (`:247-250`).
- **Win32 target**: `GetTokenInformation` + `ConvertSidToStringSidW` (advapi32). The token is the constant pseudo-handle `HANDLE(-6)` = current-thread effective token (`:238`).
- **Gotchas**:
  - Uses the **effective token of the calling thread**. If we ever call `ensure_mounted` from a thread that is impersonating, the SID would be the impersonated user's. Our call site is the app's own thread, so OK.
  - `SecurityId::new` **panics** on a SID containing `!` (cannot happen for a real SID string).
  - Returns `Err` if the process has no token user (won't happen for an interactive process).
- **Our usage**: `cfprovider.rs:190` `SecurityId::current_user().map_err(|e| e.to_string())?` — error is propagated, not panicked. **OK.**

---

## 2. `SyncRootIdBuilder` — `new()`, `.user_security_id()`, `.build()`

- **`new(provider_name: impl AsRef<OsStr>) -> Self`** (`sync_root_id.rs:59`):
  - Converts to `U16String`. **Asserts** `name.len() <= CF_MAX_PROVIDER_NAME_LENGTH` (255) and that the name contains no `!` (`0x21`) (`:62-71`). **Panics** otherwise.
  - Default `user_security_id` is empty, default `account_name` empty.
- **`.user_security_id(SecurityId) -> Self`** (`sync_root_id.rs:84`): stores the SID by value, returns `self`. (Docs note: without it, the root registers globally.)
- **`.build(self) -> SyncRootId`** (`sync_root_id.rs:99-109`): joins the three `U16Str` components with the separator `!` (`SEPARATOR = 0x21`, `:126`) and converts to a reference-counted `HSTRING` via `to_hstring()`. Final ID form:

  ```
  provider-name ! security-id ! account-name
  ```

  With our usage `account_name` is never set, so the ID is `provider!SID!` (trailing empty component). `to_components()` (`:215`) still splits into exactly 3 — empty trailing string is fine.
- **Win32 target**: none here — pure string assembly. The HSTRING is later handed to `StorageProviderSyncRootManager`.
- **Our usage**: `cfprovider.rs:191-192` builds `provider_id(label)` = `"SmartExplorer_" + sanitized label`. The sanitizer maps every non-alphanumeric char to `_` (`cfprovider.rs:169-173`), so `!` and length>255 are structurally impossible. **OK.** Note our provider name has no per-process uniqueness beyond the label — two connections with the same label collide on the same sync-root ID (intended: idempotent mount, guarded by the registry map at `cfprovider.rs:185`).

---

## 3. `SyncRootId::is_registered()`, `.register(SyncRootInfo)`, `.unregister()`

- **`is_registered(&self) -> core::Result<bool>`** (`sync_root_id.rs:142-148`):
  - Calls WinRT `StorageProviderSyncRootManager::GetSyncRootInformationForId(&self.0)`.
  - `Ok(_) => true`; `Err` with code `ERROR_NOT_FOUND` => `false`; any other `Err` propagates. So a transient WinRT failure surfaces as `Err`, not `false`.
- **`register(&self, info: SyncRootInfo) -> core::Result<()>`** (`sync_root_id.rs:159-177`):
  - **Field validation** via the `check_field!` macro (`:160-173`): if `display_name`, `icon`, `version`, or `path` equals `""` → returns `Err(ERROR_INVALID_PARAMETER, "<field> cannot be empty")`. **This is the only validation the crate performs.** (`blob` and others are not checked.)
  - `info.0.SetId(&self.0).unwrap()` then `StorageProviderSyncRootManager::Register(&info.0)`.
- **Win32/WinRT target**: WinRT `StorageProviderSyncRootManager::Register`. Internally this is what materializes the sync root that `cfapi` later connects to.
- **`unregister(&self) -> core::Result<()>`** (`sync_root_id.rs:180-182`): WinRT `StorageProviderSyncRootManager::Unregister(&self.0)`. Does **not** delete files; only removes the registration.
- **Gotchas**:
  - `register` will **panic** before it ever validates if any earlier `with_*` setter panicked (those panic at build time, not here). But `SetId(...).unwrap()` (`:175`) can panic if WinRT rejects the id.
  - Registering when already registered is **not** idempotent through this call — `Register` may error or duplicate; we guard with `is_registered()` first (`cfprovider.rs:193`). **OK.**
- **Our usage**: guarded register at `cfprovider.rs:193-211`; on connect failure we `unregister()` to avoid leaving a registered-but-unconnected root (`cfprovider.rs:222`). **OK / good hygiene.**

---

## 4. `SyncRootInfo::default()` and builders

`SyncRootInfo(StorageProviderSyncRootInfo)` (`sync_root_info.rs:29`).

- **`default()`** (`sync_root_info.rs:335-339`): `StorageProviderSyncRootInfo::new().unwrap()` — **panics** if WinRT cannot construct the object (effectively never).

Builder methods we use — note **Result vs Self**:

| Builder | Line | Returns | Internally calls (WinRT setter) | Notes |
|---|---|---|---|---|
| `with_display_name(impl AsRef<OsStr>)` | `:83` | `Self` | `SetDisplayNameResource(HSTRING)` `.unwrap()` | panics on WinRT error |
| `with_icon(impl AsRef<OsStr>)` | `:299` | `Self` | `SetIconResource(HSTRING)` `.unwrap()` | format is `"<module>,<index>"`; empty → caught by `register` validation, not here |
| `with_hydration_type(HydrationType)` | `:254` | `Self` | `SetHydrationPolicy` `.unwrap()` | enum→`StorageProviderHydrationPolicy` (`:392`) |
| `with_population_type(PopulationType)` | `:178` | `Self` | `SetPopulationPolicy` `.unwrap()` | enum→`StorageProviderPopulationPolicy` (`:435`) |
| `with_version(impl AsRef<OsStr>)` | `:196` | `Self` | `SetVersion(HSTRING)` `.unwrap()` | panics on WinRT error |
| `with_path(impl AsRef<Path>)` | `:162` | **`Result<Self>`** | `GetFolderFromPathAsync(...).get()` then `SetPath(folder)` `.unwrap()` | **returns Err if the path is not an existing folder** (the async `get()?` is the fallible part) |

- **Key asymmetry**: `with_path` and `with_recycle_bin_uri` return `Result<Self>`; all the others return `Self`. So in a builder chain `with_path` must be `?`-handled, which our code does (`cfprovider.rs:207-208`). The validation that the *folder exists* happens here — the directory must already exist before `with_path`, which is why we `create_dir_all` first (`cfprovider.rs:188`). **OK.**
- **`HydrationType::Full`** maps to `StorageProviderHydrationPolicy::Full` (`:397`); **`PopulationType::Full`** maps to `StorageProviderPopulationPolicy::Full` (`:438`) → "on-demand population required before completing a user request" (the behavior we rely on for lazy `fetch_placeholders`).
- **register-time required fields**: `display_name`, `icon`, `version`, `path` (see §3). We set all four. We do **not** set `supported_attribute`, `protection_mode`, `allow_pinning`, `hardlinks`, or `blob` — they take WinRT defaults.

---

## 5. `Session::new()`, `Session::connect(path, filter) -> Connection<F>`  — **CRITICAL**

See the dedicated "Connection lifetime & threading" section below; summary here.

- **`Session::new()`** (`session.rs:42`): `Session(CF_CONNECT_FLAG_NONE)`. Wraps connect flags only.
- **`connect<P, F>(self, path, filter) -> core::Result<Connection<F>>`** where `F: SyncFilter + 'static` (`session.rs:58-94`):
  1. `let filter = Arc::new(filter);` (`:63`).
  2. `let callbacks = filter::callbacks::<F>();` — builds the 14-entry `CF_CALLBACK_REGISTRATION` table (`proxy.rs:34`).
  3. `CfConnectSyncRoot(path, callbacks.as_ptr(), Weak::into_raw(Arc::downgrade(&filter)) as context, flags)` (`:65-82`).
     - The callback context is a **`Weak<F>`** raw pointer (`:75`), upgraded per-callback (see §6).
     - Flags are always OR'd with `CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH | CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO` (`:79-80`). The full-path flag is why `Request::path()` and `Request::process()` are always valid.
  4. Spawns a **root-watcher thread** via `spawn_root_watcher(path, filter.clone())` (`:84`) — a `ReadDirectoryChangesW` loop that drives `SyncFilter::state_changed` (`session.rs:118-210`). We don't override `state_changed`, so this thread just watches and discards.
  5. Returns `Connection::new(key, cancel_token, join_handle, callbacks, filter)` (`:87`).
- **Is `connect` blocking?** No — it returns once `CfConnectSyncRoot` succeeds and the watcher thread is spawned. It does **not** block waiting for callbacks.
- **What thread do callbacks run on?** Arbitrary OS threads chosen by the CfAPI runtime (the `extern "system"` proxies in `proxy.rs`). The crate doc on `SyncFilter` says callbacks "could be invoked from an arbitrary thread" — hence the `Send + Sync` bound (`sync_filter.rs:8-12`). `state_changed` runs on the dedicated watcher thread.
- **Win32 target**: `CfConnectSyncRoot` (cfapi).

---

## 6. `SyncFilter` dispatch in `proxy.rs` — routing, panics, required callbacks

- **Registration table** (`proxy.rs:14`, `proxy.rs:34-77`): `Callbacks = [CF_CALLBACK_REGISTRATION; 14]`. The crate **always registers 13 real callbacks + 1 terminator**, regardless of which trait methods you override. The 13:
  `FETCH_DATA`, `VALIDATE_DATA`, `CANCEL_FETCH_DATA`, `FETCH_PLACEHOLDERS`, `CANCEL_FETCH_PLACEHOLDERS`, `NOTIFY_FILE_OPEN_COMPLETION`, `NOTIFY_FILE_CLOSE_COMPLETION`, `NOTIFY_DEHYDRATE`, `NOTIFY_DEHYDRATE_COMPLETION`, `NOTIFY_DELETE`, `NOTIFY_DELETE_COMPLETION`, `NOTIFY_RENAME`, `NOTIFY_RENAME_COMPLETION`, then `CF_CALLBACK_TYPE_NONE` terminator (`:25-28`).
  - So even though we only implement `fetch_data` and `fetch_placeholders`, the OS *will* call into the proxy for delete/rename/dehydrate. The default trait bodies handle them (see below).
- **Routing**: each `extern "system" fn name<T>(info, params)` (e.g. `fetch_data` `:79`) does:
  1. `filter_from_info::<T>(info)` (`:291-315`) — reconstructs the `Weak<T>` from `(*info).CallbackContext`, **upgrades** it to `Arc<T>`; if upgrade fails (filter disconnected) it returns `None` and the callback silently no-ops. On success it re-leaks the weak via `Weak::into_raw` for reuse (`:301`).
  2. Builds `Request::new(*info)` and the typed `ticket::*` / `info::*` from the params union (`:84-92`).
  3. Calls the trait method. If it returns `Err(e)`, the proxy reports failure to the OS via `command::<Op>::fail(connection_key, transfer_key, e).unwrap()` (e.g. `proxy.rs:97` for fetch_data → `command::Write::fail`). That converts `CloudErrorKind` → `NTSTATUS` (`error.rs:74-133`) and `CfExecute`s a failing completion. The `.unwrap()` means **if reporting the failure itself fails, the proxy panics** (see below).
- **What if our method returns `Err`?** Properly handled: the OS is told the operation failed with the mapped `STATUS_CLOUD_FILE_*` code. For `fetch_data` the failing path returns a zero-length transfer with the error status (`commands.rs:87-109`); for `fetch_placeholders` a zero-count transfer (`commands.rs:190-212`).
- **What if our method PANICS?** **RISK — this is undefined behavior.** The proxies are `extern "system"` and contain **no `catch_unwind`**. A panic unwinding out of `fetch_data`/`fetch_placeholders` crosses the FFI boundary back into Windows' CfAPI dispatcher. With the default panic strategy (`unwind`) this is **UB** (typically aborts; can corrupt state). Our handlers must therefore never panic:
  - We `map_err(cerr)` every fallible call and return `Err` (`cfprovider.rs:71,78,87,88,99,129`). Good.
  - But several operations inside our handler **can still panic**: `String::from_utf8_lossy` cannot panic; however `request.path().strip_prefix` is not used in the handler. The main residual panic sources are in the crate itself (e.g. `PlaceholderFile::blob` asserts ≤4 KiB, `Metadata::created/.written` `try_into().unwrap()` on `FileTime`). See §10 and the differences table. **RISK** flagged there.
- **`.unwrap()` in the proxy on the fail path** (`:97`, `:153`, etc.): if `CfExecute` for the failure report errors (e.g. the transfer key is already invalid because the request timed out), the proxy **panics across FFI = UB**. Low probability but present. **RISK.**
- **Required/default-implemented callbacks**: The trait *requires* only `fetch_data` (no default body, `sync_filter.rs:15`). Everything else has a default:
  - `validate_data`, `fetch_placeholders`, `dehydrate`, `delete`, `rename` default to `Err(CloudErrorKind::NotSupported)` (`sync_filter.rs:39,49,75,91,110`). So the OS gets `STATUS_CLOUD_FILE_NOT_SUPPORTED` for delete/rename/dehydrate unless we implement them — meaning **deletes/renames inside the sync root are refused by default**. We only override `fetch_data` and `fetch_placeholders`, so delete/rename/dehydrate of placeholders return NotSupported. (For our browse+hydrate use this is acceptable; save-back is handled out-of-band per the module doc. **OK**, but worth noting for the file-ops audit.)
  - `cancel_fetch_data`, `cancel_fetch_placeholders`, `opened`, `closed`, `deleted`, `dehydrated`, `renamed`, `state_changed` default to no-op (`sync_filter.rs:23,54,58,62,79,95,114,124`).
  - `VALIDATE_DATA` is only actually invoked if `HydrationPolicy::ValidationRequired` is set on the root (`sync_filter.rs:31`); we don't set it, so `validate_data` is never called.

---

## 7. `Request::file_blob()` and `Request::path()`

- **`file_blob(&self) -> &[u8]`** (`request.rs:89-96`): `slice::from_raw_parts(self.0.FileIdentity as *mut u8, self.0.FileIdentityLength)`.
  - Source: the blob we attached to the placeholder via `PlaceholderFile::blob(...)` (`placeholder_file.rs:78`), which `Box::leak`s the bytes into `FileIdentity`/`FileIdentityLength` (`placeholder_file.rs:92-94`). The OS persists it with the placeholder and hands it back here.
  - **Lifetime/safety**: the returned slice borrows `Request`, which owns a copy of `CF_CALLBACK_INFO`. Valid for the duration of the callback. We immediately copy it into a `String` (`cfprovider.rs:68`), so no dangling. If the blob is empty, `FileIdentity` is null and length 0 — `from_raw_parts(null, 0)` is technically UB-adjacent but in practice returns an empty slice; we guard with `blob.is_empty()` (`cfprovider.rs:65`). **OK.**
- **`path(&self) -> PathBuf`** (`request.rs:65-71`): joins `VolumeDosName` (e.g. `C:`) with `NormalizedPath`, both read via `U16CStr::from_ptr_str`. Returns the **full local absolute path** of the placeholder (e.g. `C:\...\SmartExplorer_x\dir\file.txt`). Valid because we pass `CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH` (§5).
  - We map this back to a remote path via `remote_of()` (`cfprovider.rs:41-55`, `98`). **OK.**

---

## 8. `ticket::FetchData::write_at(buf, offset)` and `info::FetchData::required_file_range()`

- **`write_at(&self, buf: &[u8], offset: u64) -> core::Result<()>`** (`ticket.rs:64-78`, via `WriteAt` impl):
  - Builds `command::Write { buffer: buf, position: offset }` and `.execute(connection_key, transfer_key)`.
  - `execute` → `CfExecute` with `CF_OPERATION_TYPE_TRANSFER_DATA` (`commands.rs:66`, `executor.rs:56-82`). `Buffer = buf.as_ptr()`, `Offset = position`, `Length = buf.len()`, `CompletionStatus = STATUS_SUCCESS` (`commands.rs:73-84`).
  - **Win32 target**: `CfExecute(CF_OPERATION_TYPE_TRANSFER_DATA)`.
  - **ALIGNMENT (critical, doc'd at `ticket.rs:66-70`)**: *"The buffer passed must be 4KiB in length or end on the logical file size."* I.e. each `write_at` chunk must be a multiple of **4096 bytes**, UNLESS the chunk ends exactly at the file's logical size (the final chunk). This is an OS restriction on `CF_OPERATION_TYPE_TRANSFER_DATA`. A non-aligned, non-final write fails.
  - **Our usage** (`cfprovider.rs:84-88`): we read `len = range.end - range.start` bytes into one `Vec` and do a **single** `write_at(&buf, range.start)`. For our `HydrationType::Full` root, `required_file_range()` is the whole file `0..size`, so the single write **ends on the logical file size** → alignment satisfied. **OK for Full hydration.** But see RISK below.
- **`required_file_range(&self) -> Range<u64>`** (`info.rs:29-32`): `RequiredFileOffset .. (RequiredFileOffset + RequiredLength)`. The minimum range the OS demands be written. There is also `optional_file_range()` (`info.rs:39`) for larger voluntary chunks; we ignore it.
- **Chunking gotchas**:
  - We do **no chunking** and buffer the entire requested length in memory (`Vec` via `take(len).read_to_end`, `cfprovider.rs:85-87`). For a multi-GB file under `Full` hydration this allocates the whole file. **RISK (memory)**, not a correctness bug for our small-file use.
  - If the root were ever switched to `Partial`/`Progressive` hydration, the OS could request a sub-range not ending on file size; our single non-4KiB-aligned `write_at` would then **fail**. Today we use `HydrationType::Full` (`cfprovider.rs:205`), so this is latent. **RISK (latent).**
  - The skip loop (`cfprovider.rs:75-83`) reads and discards `range.start` bytes to seek; backends with real seek would be more efficient, but functionally fine.

---

## 9. `ticket::FetchPlaceholders::pass_with_placeholder(&mut [PlaceholderFile])`

- **Signature** (`ticket.rs:148-155`): `pub fn pass_with_placeholder(&self, placeholders: &mut [PlaceholderFile]) -> core::Result<()>`.
  - Note it takes **`&mut [PlaceholderFile]`** (a slice). Our code passes `&mut Vec<PlaceholderFile>` which coerces (`cfprovider.rs:129`).
- **Internals**: builds `command::CreatePlaceholders { total: placeholders.len(), placeholders }` and `.execute(...)` → `CfExecute` with `CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS` (`commands.rs:163`, `170-188`).
  - Sets `PlaceholderArray = placeholders.as_ptr()`, `PlaceholderCount = len`, `PlaceholderTotalCount = total`, `CompletionStatus = STATUS_SUCCESS` (`commands.rs:176-185`).
  - **Important flag**: `CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_DISABLE_ON_DEMAND_POPULATION` is always set on the success path (`commands.rs:175`). The crate's own TODO admits this tells the system **"there are no more placeholders in this directory"** even when that could be untrue (no streaming support). So **every `fetch_placeholders` must return the COMPLETE directory listing in one call** — partial/streamed population is not supported by this crate version. We do return the full `list_dir` result in one shot (`cfprovider.rs:99-129`). **OK**, but it means large directories are fully buffered.
- **Does it consume/mutate the vec?** It takes `&mut`. `CfCreatePlaceholders`/`TRANSFER_PLACEHOLDERS` writes back per-item results into each `CF_PLACEHOLDER_CREATE_INFO.Result` and `CreateUsn` (the mutation). It does **not** drain or shrink the Vec; the `PlaceholderFile` elements remain and are dropped (freeing their leaked blob + filename) when our local `placeholders` Vec drops at end of `fetch_placeholders`.
- **Per-item error reporting**: `pass_with_placeholder` returns a single `core::Result<()>` for the whole batch — it does **not** surface per-placeholder failures. Per-item status lives in `PlaceholderFile::result()` (`placeholder_file.rs:99-101`, reads `self.0.Result`), but `pass_with_placeholder` never reads it back to the caller. So **if one placeholder fails to create (e.g. duplicate name, bad metadata) but the `CfExecute` call as a whole succeeds, we get `Ok(())` and silently miss that item.** **RISK** — we don't inspect `.result()` per item.
- **Win32 target**: `CfExecute(CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS)`.

---

## 10. `PlaceholderFile` builder, the struct it lowers to, `Metadata`, `FileTime`

### `PlaceholderFile` (`placeholder_file.rs`)

Wraps **`CF_PLACEHOLDER_CREATE_INFO`** (`placeholder_file.rs:16`).

- **`new(relative_path: impl AsRef<Path>)`** (`:20-31`): sets `RelativeFileName` to a `U16CString` (`.unwrap()` — **panics if the name contains an interior NUL**), `Flags = CF_PLACEHOLDER_CREATE_FLAG_NONE`, `Result = S_FALSE`.
- **`has_no_children()`** (`:37`): OR `CF_PLACEHOLDER_CREATE_FLAG_DISABLE_ON_DEMAND_POPULATION`. "This directory has no children to populate." (We set it on files, see RISK in differences.)
- **`mark_in_sync()`** (`:47`): OR `CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC` → equivalent to `CfSetInSyncState`. Without it the file shows the "sync pending" overlay and may be re-fetched.
- **`overwrite()`** (`:53`): OR `..._SUPERSEDE`. (We don't use it → re-listing a dir whose placeholders already exist may error per-item; see §9 RISK.)
- **`block_dehydration()`** (`:61`): OR `..._ALWAYS_FULL`. (We don't use it.)
- **`metadata(Metadata)`** (`:67`): copies `metadata.0` into `FsMetadata`.
- **`blob(Vec<u8>)`** (`:78-97`): **asserts** `blob.len() <= CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH` (**4096**) — **panics if larger**. Empty blob → null pointer. Otherwise `Box::leak`s the bytes into `FileIdentity`/`FileIdentityLength`. Freed in `Drop` (`:129-144`).
- **Lowers to** `CF_PLACEHOLDER_CREATE_INFO { RelativeFileName, Flags, FsMetadata, FileIdentity, FileIdentityLength, Result, CreateUsn }`.
- **`create()`** (`:115`) calls `CfCreatePlaceholders` directly — but its docstring explicitly says **do not call this from `fetch_placeholders`; use `pass_with_placeholder` instead** (`:111-114`). We correctly use the ticket. **OK.**

### `Metadata` (`metadata.rs`)

Wraps **`CF_FS_METADATA { BasicInfo: FILE_BASIC_INFO, FileSize }`** (`metadata.rs:13`).

- **`file()`** (`:17`): `FileAttributes = FILE_ATTRIBUTE_NORMAL`. **`directory()`** (`:28`): `FILE_ATTRIBUTE_DIRECTORY`.
- **`created/accessed/written/changed(FileTime)`** (`:39-60`): set the corresponding `FILE_BASIC_INFO` time via `time.try_into().unwrap()` — **panics if the `FileTime` is out of `i64` range** (FileTime → i64 NT timestamp). `FileTime::now()` is always in range, so OK in practice.
- **`size(u64)`** (`:63`): sets `FileSize` (cast to `i64`). For directories, `size` is meaningless; the test passes `.size(0)` for dirs but we don't set size on `Metadata::directory()` (we only `.created().written()`), which leaves `FileSize = 0`. **OK.**
- **`attributes(u32)`** (`:69`): OR-adds attribute bits. We don't use it (so no `FILE_ATTRIBUTE_PINNED`/`UNPINNED`/`READONLY` etc. — matches the working example, which also doesn't).

### `nt_time::FileTime`

- Re-exported at `utility.rs:7`. `FileTime::now()` used at `cfprovider.rs:100`. Converts to the NT 64-bit FILETIME (`i64`) consumed by `FILE_BASIC_INFO`. Used identically by the crate's example (`tests/.../sync_filter.rs:57`).

---

## Connection lifetime & threading (CRITICAL)

This is the single most important section for the audit.

### What `Connection<F>` owns (`root/connect.rs:17-25`)

```
connection_key: RawConnectionKey   // i64 from CfConnectSyncRoot
cancel_token:   Sender<()>          // stops the watcher thread
join_handle:    JoinHandle<()>      // the ReadDirectoryChangesW thread
_callbacks:     Callbacks           // the [CF_CALLBACK_REGISTRATION; 14] table, kept ALIVE
filter:         Arc<F>              // the STRONG ref keeping our RemoteFilter alive
```

### What `Drop` does (`connect.rs:57-66`)

1. `CfDisconnectSyncRoot(CF_CONNECTION_KEY(connection_key)).unwrap()` — **disconnects the sync root** (stops callbacks). `.unwrap()` → **panics if disconnect fails.**
2. `cancel_token.send(())` to ask the watcher thread to stop.
3. **Busy-waits** (`thread::sleep(150ms)` loop) until `join_handle.is_finished()`. So dropping a `Connection` **blocks** up to one `ReadDirectoryChangesW` poll cycle.

### MUST the `Connection` be kept alive? — **YES. This is load-bearing.**

- The callback context registered with `CfConnectSyncRoot` is a **`Weak<F>`** (`session.rs:75`). Each callback upgrades it to `Arc<F>` via `filter_from_info` (`proxy.rs:295-304`).
- The **only strong `Arc<F>`** lives inside `Connection.filter` (`connect.rs:24`). The watcher thread holds another `filter.clone()` strong ref (`session.rs:85`), but that thread is itself owned/stopped by the `Connection`.
- **If the `Connection` is dropped**: (a) `CfDisconnectSyncRoot` is called, tearing down the OS registration of callbacks; AND (b) the strong `Arc<F>` is freed, so any in-flight or future callback's `Weak::upgrade()` returns `None` and the callback **silently no-ops** (`proxy.rs:306-313`). Net effect: **no more `fetch_data`/`fetch_placeholders` fire → directories never populate and files never hydrate.**
- Therefore the `Connection` must live as long as the sync root should function. Our code stores it in a process-lifetime `static` registry (`cfprovider.rs:163-166, 226`) keyed by local root path. **This matches the requirement and is correct.** Dropping the process drops the map → disconnect on shutdown. **OK.**
- The crate's **own example drops the connection deliberately at the end** (`tests/.../sync_filter.rs:132 drop(connection)`) only because the test is finished exercising it; while the test runs, `connection` is a live local binding. So "keep alive while in use" holds in both.

### `_callbacks` must also outlive the connection

The `Callbacks` array is passed by pointer to `CfConnectSyncRoot` (`session.rs:66`) and the OS may read it for the connection's lifetime. The crate keeps it in `Connection._callbacks` (`connect.rs:23`) so it isn't freed early. Storing the whole `Connection` preserves this. **OK.**

### Threading model

- **Callbacks** (`fetch_data`, `fetch_placeholders`, etc.): run on **arbitrary CfAPI worker threads**, possibly concurrently → `SyncFilter: Send + Sync` required (`sync_filter.rs:12`). Our `RemoteFilter` holds a `BackendHandle` (an `Arc<dyn Vfs + Send + Sync>` per `vfs.rs`) and only `&self`, so it is `Send + Sync`. Concurrent `fetch_data` calls on the same backend must be thread-safe — the SFTP/FTP backends use internal locks (`ftp.rs` `self.lock()`), so OK, but **concurrent hydrations serialize on those locks** (perf, not correctness). **OK / RISK(perf).**
- **Watcher thread** (`spawn_root_watcher`, `session.rs:118`): one per connection, runs `state_changed` (we no-op). It **`.expect()`s** on opening the sync-root dir and on `ReadDirectoryChangesW` (`session.rs:130,148,160`) — if those fail the **watcher thread panics** (thread-local; aborts only that thread, not the process, but leaves the connection without state-change detection). We don't depend on `state_changed`, so low impact. **RISK (minor).**

---

## How our usage differs from the crate's own working example

Compared against `tests/behavior/sync_filter.rs` (the known-working `MemFilter`). Differences from a known-working baseline are prime suspects.

| # | Aspect | Crate example (`sync_filter.rs`) | Our `cfprovider.rs` | Severity & rationale |
|---|---|---|---|---|
| 1 | **Connection kept alive** | Local `connection` binding, dropped only after the test finishes (`:127,132`) | Stored in a process-lifetime `static` registry map (`:163-166,226`) | **OK** — both keep it alive while in use; ours is stronger (lifetime = process). |
| 2 | **Icon** | `.with_icon("%SystemRoot%\\system32\\charmap.exe,0")` (`:105`) | `.with_icon("<SystemRoot>\\System32\\shell32.dll,4")` (`:198`) | **OK** — both non-empty (register requires non-empty icon). Different resource, cosmetic. |
| 3 | **`with_path` existence** | `ROOT_PATH` created before register (`:123-125`) | `create_dir_all(&local_root)` before register (`:188`) | **OK** — both satisfy `with_path`'s "must be an existing folder" requirement. |
| 4 | **`with_recycle_bin_uri`** | Sets it (`:107`) | Not set | **OK** — optional; not a register-required field. |
| 5 | **Blob content / encoding** | Blob = the **relative** path (`"dir1\\test2.txt"`), used directly as the lookup key in `fetch_data` (`:30,84`) | Blob = the **full remote** path (`base + "/" + name`), `child_remote.into_bytes()` (`:104,123`) | **OK** — semantics differ but consistent within our code; `fetch_data` reads `file_blob` as the remote path (`:64-69`). |
| 6 | **Blob length guard** | Short, constant paths — always < 4 KiB | Remote path can be **arbitrarily long** (deep trees / long names) | **RISK→potential BUG**: `PlaceholderFile::blob` **asserts ≤ 4096 bytes and PANICS** (`placeholder_file.rs:79-84`). A remote path > 4 KiB (rare but possible with deep nesting) panics **inside `fetch_placeholders` → unwinds across FFI = UB** (§6). We never truncate or check blob length. **Recommend a guard.** |
| 7 | **Filename NUL guard** | Constant names, no NUL | `display` derived from backend listing / `download_name` (`:115-119`) | **RISK**: `PlaceholderFile::new` does `U16CString::from_os_str(...).unwrap()` (`placeholder_file.rs:24-26`) — **panics on an interior NUL** in a remote filename. A malicious/odd backend name with `\0` panics across FFI = UB. We don't sanitize names. |
| 8 | **`has_no_children` usage** | Set on **files only**, never on `dir1` (the example dir keeps on-demand population) (`:75-84` vs `:59-63`) | Set on **non-dir** items (`if !m.is_dir { pf = pf.has_no_children() }`) (`:124-126`) | **OK** — same intent (files have no children). Directories correctly omit it so they populate on demand. |
| 9 | **Metadata size vs actual bytes** | `size = path.len()`, and `fetch_data` writes exactly those bytes (`:71,42`) — size **matches** the content written | `Metadata::file().size(m.size)` uses the backend's **reported** size (`:108`), but for Google-Docs export the **downloaded bytes differ** (export to .docx/.pdf is a different length); `download_name` even changes the extension (`:117-119`) | **RISK (real)**: declared placeholder size ≠ bytes delivered in `fetch_data`. Under `HydrationType::Full` the OS expects the transfer to **end on the logical file size** (§8). If the exported content is **shorter** than `m.size`, `write_at` never reaches the declared size → hydration may hang/fail the 4 KiB-alignment rule; if **longer**, a write past declared size is rejected. Standard files are unaffected (size matches). **Flag for the file-ops audit.** |
| 10 | **`fetch_data` range check** | Asserts `required_file_range() == 0..content.len()`, else `InvalidRequest` (`:38`) | No range/size assertion; reads `range.start..range.end` from the backend and writes once (`:70-88`) | **OK→RISK**: more permissive is fine for `Full`. But combined with #9 (size mismatch) there's no guard catching a short/long transfer; the example's assertion would have caught it. |
| 11 | **Error mapping granularity** | Maps specific backend conditions to specific `CloudErrorKind` (e.g. `InvalidRequest`) | `cerr()` collapses **every** error to `CloudErrorKind::Unsuccessful` (`:29-31`) | **OK (minor)** — works, but the user/OS sees a generic `STATUS_CLOUD_FILE_UNSUCCESSFUL` for network-down, not-found, auth-fail alike. Harder to diagnose; no functional break. |
| 12 | **Panic discipline in handlers** | Uses `.unwrap()` on `write_at`/`pass_with_placeholder` (`:42,88`) — example would panic on failure | We `map_err(cerr)?` both (`:88,129`) — no panic | **OK / better than example.** (Note the example's `.unwrap()` is itself an UB risk it gets away with because the test backend never fails.) |
| 13 | **`mark_in_sync`** | Set on every placeholder (`:60,65,76`) | Set on every placeholder (`:121`) | **OK** — identical. |
| 14 | **Population completeness** | Returns the full listing for each dir level in one `pass_with_placeholder` | Same — full `list_dir` in one call (`:99-129`) | **OK** — required, since the crate sets `DISABLE_ON_DEMAND_POPULATION` on success (§9). |
| 15 | **Extra: forced pre-population (`populate_to`)** | Not present (test enumerates via PowerShell) | We pre-walk ancestor dirs with `read_dir` to force population before opening a leaf (`:142-160`) | **OK** — works around CfAPI only populating on enumeration; sound given `PopulationType::Full`. |

### Highest-priority suspects (audit follow-up)

- **#9 (BUG-class for Google-Docs/transform backends)**: placeholder size (original) vs delivered bytes (exported) mismatch under `Full` hydration — most likely real-world hydration failure.
- **#6 / #7 (RISK→UB)**: unchecked blob length (>4 KiB) and unchecked NUL in filenames both panic **inside crate code** during `fetch_placeholders`, and that panic unwinds across the `extern "system"` boundary = **UB** (the crate has no `catch_unwind`). Add guards.
- **#8 latent / §8**: switching off `HydrationType::Full` would break our single non-4 KiB-aligned `write_at`.

---

## Appendix: Win32 / WinRT call map (what each API ultimately invokes)

| Crate API | Ultimate native call | Source |
|---|---|---|
| `SecurityId::current_user` | `GetTokenInformation` + `ConvertSidToStringSidW` | `sync_root_id.rs:261,288` |
| `SyncRootId::is_registered` | WinRT `StorageProviderSyncRootManager::GetSyncRootInformationForId` | `sync_root_id.rs:143` |
| `SyncRootId::register` | WinRT `StorageProviderSyncRootManager::Register` | `sync_root_id.rs:176` |
| `SyncRootId::unregister` | WinRT `StorageProviderSyncRootManager::Unregister` | `sync_root_id.rs:181` |
| `SyncRootInfo::with_path` | WinRT `StorageFolder::GetFolderFromPathAsync` + `SetPath` | `sync_root_info.rs:148-155` |
| `Session::connect` | `CfConnectSyncRoot` (+ spawns `ReadDirectoryChangesW` thread) | `session.rs:66`, `137` |
| `Connection::drop` | `CfDisconnectSyncRoot` | `connect.rs:59` |
| `ticket::FetchData::write_at` | `CfExecute(CF_OPERATION_TYPE_TRANSFER_DATA)` | `commands.rs:66`, `executor.rs:62` |
| `ticket::FetchData::read_at` | `CfExecute(CF_OPERATION_TYPE_RETRIEVE_DATA)` | `commands.rs:34` |
| `ticket::FetchData::report_progress` | `CfReportProviderProgress` | `ticket.rs:37` |
| `ticket::FetchPlaceholders::pass_with_placeholder` | `CfExecute(CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS)` | `commands.rs:163`, `executor.rs:62` |
| proxy `*::fail` (on `Err`) | `CfExecute(<op>)` with mapped `STATUS_CLOUD_FILE_*` | `commands.rs:87+`, `error.rs:74` |
| `PlaceholderFile::create` (unused by us) | `CfCreatePlaceholders` | `placeholder_file.rs:117` |
