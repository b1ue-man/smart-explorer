# W2 — End-to-end DATA-FLOW TRACE ("the web") of the CfAPI remote-file-open feature

Authoritative trace of every CfAPI-related call in Smart Explorer's remote-file-open
feature, from "user double-clicks a remote file" to "file opens + edits save back".
Built by reading OUR code and the `cloud-filter` v0.0.6 crate source, verifying against
Wave-1 docs (`w1_registration.md`, `w1_fetchdata.md`, `w1_placeholders.md`,
`w1_crate_map.md`). Where I found Wave-1 to be incomplete or a new issue to exist, it is
called out in §4.

Conventions: `file:line` cites the exact source. Win32/WinRT lowering is given for every
value that crosses the Rust→FFI→OS boundary. Example remote paths used throughout:
- **A** (normal file): remote `"/Reports/Q3 & Q4.xlsx"`
- **B** (Google Doc, no extension): remote `"/Notes/Plan"` (mimeType `…google-apps.document`)
- **C** (deep path): remote `"/A/B/C/D/E/deep file.txt"`

Connection assumptions for examples: `label = "My Drive"`, `root_path` (connection root,
`self.root_path`) = `"/"` (Drive shows a POSIX-rooted tree), backend = `gdrive.rs`.

---

## 1. The ordered TRACE

Each step: **fn (file:line)** — INPUTS (`name : type : source`) — TRANSFORMATION —
OUTPUTS (`name : type : destination`).

### Phase I — user gesture → open_file

**1. `App::activate_entry` (app.rs:3062)** — double-click / Enter on a row.
- IN: `idx : usize : UI event`; `self.entries[idx] : Entry` (has `path: String` forward-slash
  remote path e.g. A=`"/Reports/Q3 & Q4.xlsx"`, `name: String` = `"Q3 & Q4.xlsx"`,
  `is_dir: bool`).
- TRANSFORM: if `is_dir`, navigate (out of scope). Else clone `(path, name)`.
- OUT: calls `self.open_file(path: String, name: String)` (app.rs:3073).

**2. `App::open_file` (app.rs:3079)** — the dispatcher.
- IN: `path : String : entry.path` (remote, fwd-slash); `name : String : entry.name`;
  `self.remote : Option<RemoteState>` (`rs.backend : BackendHandle = Arc<dyn Backend>`,
  `rs.label : String` = `"My Drive"`); `self.remote_open_mode : RemoteOpenMode`
  (loaded from `%APPDATA%\smart_explorer\remote_open_mode.txt`, app.rs:602–610);
  `self.root_path : String` = connection root `"/"`.
- TRANSFORM: if `self.remote` is `None` → pure-local `open_path(&path)` and return
  (app.rs:3083). Else clone backend+label. On Windows + `RemoteOpenMode::CfApi` → CfApi
  branch (app.rs:3096). Else Temp branch (app.rs:3160).
- OUT: into the CfApi branch.

### Phase II — ensure_mounted (registration + connect), CfApi branch

**3. `cfprovider::ensure_mounted` (cfprovider.rs:178)** — IN: `label:&str="My Drive"`,
`backend:BackendHandle`, `remote_root:&str = self.root_path.trim_end_matches('/')`
(app.rs:3100 — note: `"/"` trimmed → `""`).
- TRANSFORM/OUT below, sub-steps 3a–3i.

**3a. `cfsync::conn_root_dir(label)` (cfsync.rs:40)** — IN: `conn_label:&str="My Drive"`.
- TRANSFORM: `sync_base()` (`%USERPROFILE%\Smart Explorer`, cfsync.rs:22) `.join(san("My Drive"))`.
  `san` (cfsync.rs:30) maps `/\:*?"<>|` and control chars → `_`, then trims spaces and
  leading/trailing `.`. `"My Drive"` has a space (NOT in the replace set) → kept; result
  **`"My Drive"`** (unchanged). local_root = `C:\Users\<u>\Smart Explorer\My Drive`.
- OUT: `local_root : PathBuf` → `key : String` (its lossy string, cfprovider.rs:184).

**3b. registry idempotency (cfprovider.rs:185)** — IN: `key:String`,
`registry() : &'static Mutex<HashMap<String,Connection<RemoteFilter>>>` (process-static,
cfprovider.rs:163). If `contains_key` → return `Ok(local_root)` (already mounted). Else
`create_dir_all(&local_root)` (cfprovider.rs:188) so `with_path` can later resolve it.

**3c. `SecurityId::current_user()` (crate sync_root_id.rs:256)** — IN: none.
- TRANSFORM (all `unsafe`): `GetTokenInformation(CURRENT_THREAD_EFFECTIVE_TOKEN=HANDLE(-6),
  TokenUser,…)` → `TOKEN_USER` → `ConvertSidToStringSidW(User.Sid)` → `PWSTR` UTF-16 →
  `OsString` → `SecurityId(U16String)`.
- OUT: `sid : SecurityId` (wraps a UTF-16 `U16String` like `"S-1-5-21-…"`).
  **Win32:** advapi32 `GetTokenInformation` + `ConvertSidToStringSidW`.

**3d. `provider_id(label)` (cfprovider.rs:168)** — IN: `label:&str="My Drive"`.
- TRANSFORM: map every non-`[A-Za-z0-9]` char → `_`; prefix `"SmartExplorer_"`. Space → `_`,
  so `"My Drive"` → **`"SmartExplorer_My_Drive"`**. (Note: different sanitizer than `san` —
  see §4 NEW-1.)
- OUT: `pid : String`.

**3e. `SyncRootIdBuilder::new(&pid).user_security_id(sid).build()` (cfprovider.rs:192;
crate sync_root_id.rs:59/84/99)** — IN: `pid:&str`, `sid:SecurityId`.
- TRANSFORM: `new` asserts `pid` ≤255 chars and contains no `!` (else **panic**). `build`
  joins three `U16Str` components with separator `!` (0x21): `provider ! SID ! account`;
  account is **empty** (never set). `to_hstring()`.
- OUT: `sync_root_id : SyncRootId` (a refcounted WinRT `HSTRING`), value
  `"SmartExplorer_My_Drive!S-1-5-21-…!"`. **No Win32 yet — pure UTF-16 string assembly.**

**3f. `sync_root_id.is_registered()` (crate sync_root_id.rs:142)** — IN: `&self`.
- TRANSFORM: WinRT `StorageProviderSyncRootManager::GetSyncRootInformationForId(HSTRING)`.
  `Ok→true`, `Err(ERROR_NOT_FOUND)→false`, other `Err` propagates.
- OUT: `bool`. If `true` skip 3g.

**3g. `sync_root_id.register(SyncRootInfo{…})` (cfprovider.rs:199; crate sync_root_id.rs:159)**
— IN: a `SyncRootInfo` built by chained setters (cfprovider.rs:201–208):
  - `.with_display_name(label)` → WinRT `SetDisplayNameResource(HSTRING("My Drive"))`.
  - `.with_icon(icon)` where `icon = format!("{SystemRoot}\\System32\\shell32.dll,4")`
    (cfprovider.rs:197–198) → `SetIconResource(HSTRING)`.
  - `.with_hydration_type(HydrationType::Full)` → `SetHydrationPolicy(StorageProviderHydrationPolicy::Full=2)`.
  - `.with_population_type(PopulationType::Full)` → `SetPopulationPolicy(StorageProviderPopulationPolicy::Full=1)`.
  - `.with_version("1.0.0")` → `SetVersion(HSTRING)`.
  - `.with_path(&local_root)` → **`Result<Self>`**; WinRT `StorageFolder::GetFolderFromPathAsync(local_root).get()?`
    then `SetPath(IStorageFolder)`; errors if folder absent (we `create_dir_all`'d in 3b).
- TRANSFORM: crate `check_field!` rejects empty display_name/icon/version/path
  (`ERROR_INVALID_PARAMETER`); then `info.SetId(&sync_root_id)` and WinRT
  `StorageProviderSyncRootManager::Register(info)`.
- OUT: `()` on success; the OS now has a registered sync root rooted at `local_root`.
  **WinRT:** `Windows.Storage.Provider.StorageProviderSyncRootManager::Register`.

**3h. `RemoteFilter { backend, remote_root, local_root }` (cfprovider.rs:212)** — IN:
`backend:BackendHandle`, `remote_root = remote_root.trim_end_matches('/').to_string()`
(for `"/"` → **`""`**), `local_root.clone()`.
- OUT: `filter : RemoteFilter` (the `SyncFilter` impl; holds the data that every callback
  will read via `&self`). It is `Send+Sync` (backend is `Arc<dyn Backend+Send+Sync>`).

**3i. `Session::new().connect(&local_root, filter)` (cfprovider.rs:217; crate session.rs:58)**
— IN: `local_root:&Path`, `filter:RemoteFilter`.
- TRANSFORM: `Arc::new(filter)`; build the 14-entry `CF_CALLBACK_REGISTRATION` table
  (proxy.rs); `CfConnectSyncRoot(path, callbacks, ctx = Weak::into_raw(Arc::downgrade(&filter)),
  flags | CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH | …REQUIRE_PROCESS_INFO)`; spawn the
  `ReadDirectoryChangesW` root-watcher thread (drives `state_changed`, which we don't override).
- OUT: on `Ok` → `conn : Connection<RemoteFilter>` (owns `connection_key:i64`,
  `Arc<RemoteFilter>` strong ref, the callbacks table, the watcher thread). On `Err` →
  `sync_root_id.unregister()` then return `Err(String)` (cfprovider.rs:219–224).
  **Win32:** `CfConnectSyncRoot`.

**3j. registry insert (cfprovider.rs:226)** — `registry().insert(key, conn)` keeps the
`Connection` (and thus the strong `Arc<RemoteFilter>` and the callbacks table) alive for the
process lifetime. **This is load-bearing**: if dropped, callbacks `Weak::upgrade()→None` and
silently no-op (no population/hydration). OUT: `Ok(local_root : PathBuf)`.

### Phase III — compute the local placeholder path & pre-populate

**4. back in `open_file` (app.rs:3102)** — IN: `local_root : PathBuf` from step 3.
- `local_name = backend.download_name(&path, &name)` (app.rs:3106). For **A** (xlsx, a normal
  binary): `download_name` returns `name` unchanged = `"Q3 & Q4.xlsx"`. For **B** (Google Doc):
  `gdrive::download_name` (gdrive.rs:518) looks up cached `mime_of(path)` →
  `export_ext` = `"docx"` → since `"Plan"` doesn't end `.docx`, returns **`"Plan.docx"`**.
- OUT: `local_name : String`.

**5. `cfsync::local_path_named(label, root_path, path, local_name)` (cfsync.rs:64)** — IN:
`conn_label="My Drive"`, `conn_root=self.root_path="/"`, `remote_path=path`,
`leaf=local_name`.
- TRANSFORM: `parent = remote_path.rsplit_once('/').0` (A → `"/Reports"`; B → `"/Notes"`;
  C → `"/A/B/C/D/E"`). `local_path(label, root, parent)` (cfsync.rs:46) strips `conn_root`
  prefix, splits the remainder on `/`, and **`san()`-sanitizes EACH segment**, joining under
  `sync_base()/san(label)`. Then `.join(san(leaf))`.
- OUT example **A**: `…\Smart Explorer\My Drive\Reports\Q3 & Q4.xlsx` (`&` survives `san`).
  **B**: `…\My Drive\Notes\Plan.docx`. **C**: `…\My Drive\A\B\C\D\E\deep file.txt`.
  `dest : PathBuf` → the file we will `ShellExecute`.

**6. `cfprovider::populate_to(&local_root, &dest)` (cfprovider.rs:142)** — IN: `local_root`,
`target=dest`.
- TRANSFORM: `parent = dest.parent()`. `drain(local_root)` = `std::fs::read_dir` (consumed via
  `.flatten()`), which under the hood is `FindFirstFile/FindNextFile` on a CfAPI placeholder
  dir → **fires FETCH_PLACEHOLDERS** for the root. Then `rel = parent.strip_prefix(local_root)`
  and for each `seg` in `rel.components()`, descend and `drain` that level (fires
  FETCH_PLACEHOLDERS for each ancestor). For **C** this is 5 nested directory enumerations.
- OUT: no return value; **side effect** = each ancestor directory's children become real
  placeholder files on disk (see steps 7–9), so `dest` exists by the time we open it.

### Phase IV — OS-driven FETCH_PLACEHOLDERS (per ancestor enumeration)

**7. OS fires `RemoteFilter::fetch_placeholders` (cfprovider.rs:92)** on an arbitrary CfAPI
worker thread, once per enumerated partial directory.
- IN: `request : Request` (wraps `CF_CALLBACK_INFO`; `request.path()` = full local path of the
  directory being enumerated, e.g. `C:\…\My Drive\Reports`); `ticket : FetchPlaceholders`
  (carries connection_key/transfer_key); `_info` (the glob `Pattern`, ignored).
- `request.path()` (crate request.rs:65): joins `VolumeDosName` (`C:`) + `NormalizedPath`,
  both UTF-16 `U16CStr` → `PathBuf`. (Valid because we set `CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH`.)

**8. `RemoteFilter::remote_of(&request.path())` (cfprovider.rs:41, called :98)** — IN: the
local dir path.
- TRANSFORM: `local.strip_prefix(self.local_root)` → relative; `.to_string_lossy()`;
  `.replace('\\','/')`; trim a trailing `/` off `remote_root` and a leading `/` off rel;
  `format!("{}/{}", root, rel)` (or just `root` if rel empty). **`remote_of` does NOT apply
  `san()`** — it round-trips raw bytes. For `…\My Drive\Reports`, with `remote_root=""`
  (from `"/"`) → `rel="Reports"` → result `"/Reports"`? No: `format!("{}/{}", "", "Reports")`
  = `"/Reports"`. (Empty root + leading separator is how Drive's POSIX root re-emerges.)
- OUT: `remote_dir : String` (e.g. `"/Reports"`).

**9. `self.backend.list_dir(&remote_dir)` (cfprovider.rs:99)** — IN: `remote_dir:&str`.
- TRANSFORM (gdrive.rs:428): `resolve(path)→fileId`, paged Drive `files.list` query
  `'<id>' in parents and trashed=false`, fields `id,name,mimeType,size,modifiedTime,createdTime`.
  Side effect: caches `child_path→mimeType` in `self.mimes` and `child_path→id` in `self.ids`
  (gdrive.rs:453–459) — **this is what later lets `download_name`/`open_read` know B is a Doc.**
- OUT: `metas : Vec<VfsMeta>` (each: `name:String`, `is_dir:bool`, `size:u64` [**0 for Google
  Docs**, gdrive.rs:267], `mtime_ms`, `btime_ms`).

**10. Per-child placeholder build (cfprovider.rs:103–127)** — IN: each `m : VfsMeta`,
`base = remote_dir.trim_end_matches('/')`, `now = FileTime::now()`.
- `child_remote = format!("{}/{}", base, m.name)` (cfprovider.rs:104) — the full remote path,
  e.g. A=`"/Reports/Q3 & Q4.xlsx"`, B=`"/Notes/Plan"`.
- `md : Metadata`: `Metadata::file().size(m.size)` (dir → `Metadata::directory()`),
  `.created(now).written(now)` (cfprovider.rs:105–111). `Metadata` wraps
  `CF_FS_METADATA{ BasicInfo:FILE_BASIC_INFO, FileSize:i64 }`. `file()` →
  `FileAttributes=FILE_ATTRIBUTE_NORMAL(0x80)`; `directory()` → `FILE_ATTRIBUTE_DIRECTORY(0x10)`.
  `FileTime` → 100-ns NT ticks (`i64`).
- `display : String` (cfprovider.rs:115–119): dirs → `m.name`; files → `backend.download_name(&child_remote, &m.name)`.
  For B this becomes **`"Plan.docx"`**; for A stays **`"Q3 & Q4.xlsx"`**. **This is the
  placeholder's on-disk NAME** — and it is **raw, NOT `san()`-ed.**
- `pf = PlaceholderFile::new(&display).mark_in_sync().metadata(md).blob(child_remote.into_bytes())`
  (cfprovider.rs:120–123). `new` → `RelativeFileName = U16CString::from_os_str(display)` (UTF-16,
  `.unwrap()` **panics on interior NUL**). `mark_in_sync` → `CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC(0x2)`.
  `blob(Vec<u8>)` → **asserts len ≤ 4096** (panic if larger), else `Box::leak`s into
  `FileIdentity:*const c_void` + `FileIdentityLength:u32`. Files also get `has_no_children()`
  → `CF_PLACEHOLDER_CREATE_FLAG_DISABLE_ON_DEMAND_POPULATION(0x1)` (cfprovider.rs:124–126;
  no-op on files).
- OUT: `placeholders : Vec<PlaceholderFile>` (each lowering to `CF_PLACEHOLDER_CREATE_INFO`).

**11. `ticket.pass_with_placeholder(&mut placeholders)` (cfprovider.rs:129; crate ticket.rs:148)**
— IN: `&mut [PlaceholderFile]`.
- TRANSFORM: builds a `CF_OPERATION_PARAMETERS` `TransferPlaceholders` op with
  `PlaceholderArray = placeholders.as_ptr()`, `PlaceholderCount = len`,
  `PlaceholderTotalCount = len`, **`Flags = …DISABLE_ON_DEMAND_POPULATION` (hard-coded,
  commands.rs:175)** → marks this directory fully populated (one-shot; must return ALL
  children). `CfExecute(CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS, key)`.
- OUT: `core::Result<()>` (only the top-level HRESULT; per-entry `Result` discarded). On `Err`
  → `map_err(cerr)` → `CloudErrorKind::Unsuccessful` → proxy `CreatePlaceholders::fail`.
  **Win32:** `CfExecute(TRANSFER_PLACEHOLDERS)`. The placeholders now exist as real on-disk
  files under `local_root`.

### Phase V — open the now-existing placeholder

**12. `open_file` existence check (app.rs:3128)** — IN: `dest : PathBuf` (step 5).
- If `dest.exists()` → set a notice and `self.open_path(&dest.to_string_lossy().replace('\\',"/"))`
  (app.rs:3133). Else → user-visible error "Platzhalter … wurde nicht erzeugt" (app.rs:3137).
  Also pushes a `RemoteEdit` watch entry **before** the open (app.rs:3117–3127, see step 14).

**13. `App::open_path` (app.rs:3024, Windows)** — IN: `path:&str` (the `dest` string, fwd-slash).
- TRANSFORM: `path.replace('/', "\\")` → UTF-16 wide + NUL; `ShellExecuteW(NULL, NULL, wide,
  NULL, NULL, SW_SHOWNORMAL=1)`.
- OUT: the associated app opens the file → the app's first read triggers hydration.
  **Win32:** `ShellExecuteW`.

### Phase VI — OS-driven FETCH_DATA (hydration on first read)

**14. (registered earlier, app.rs:3117) `RemoteEdit` entry** — `{ temp: dest, backend,
remote_path: path (RAW remote, e.g. "/Notes/Plan"), name, baseline_mtime: i64::MAX (sentinel),
seen_mtime: 0, uploading: false }`. Note `remote_path` is the RAW remote path, **not**
`child_remote` and **not** `local_name`. Pushed to `self.remote_edits` (cap 50).

**15. OS fires `RemoteFilter::fetch_data` (cfprovider.rs:58)** on a worker thread when the app
reads the dehydrated placeholder.
- IN: `request : Request`; `ticket : FetchData`; `info : FetchData`.
- `blob = request.file_blob()` (crate request.rs:89) = `&[u8]` view of the leaked blob =
  the `child_remote` bytes we stored in step 10 (A=`"/Reports/Q3 & Q4.xlsx"`, B=`"/Notes/Plan"`).
- `remote = if blob.is_empty() { remote_of(request.path()) } else { String::from_utf8_lossy(blob) }`
  (cfprovider.rs:65–69). Normally non-empty → uses the blob's **exact raw remote path**.
  **Key:** the blob path (B=`"/Notes/Plan"`) is the un-exported Drive path — correct for
  download — even though the on-disk name is `Plan.docx`.
- `range = info.required_file_range()` (crate info.rs:29) = `Range<u64>` =
  `RequiredFileOffset .. (RequiredFileOffset + RequiredLength)` (both `i64` fields, **added in
  i64 then cast** — see §4 NEW-4). For a plain full open: `0 .. logical_size`.

**16. `self.backend.open_read(&remote)` (cfprovider.rs:71)** — IN: `remote:&str`.
- TRANSFORM (gdrive.rs:498): `resolve→id`, `mime = mime_of(path)`; if `export_format(mime)`
  is `Some` (B) → `GET …/files/{id}/export?mimeType=<office mime>`; else (A) → `GET …/files/{id}?alt=media`.
- OUT: `r : Box<dyn Read + Send>` (an HTTP response body reader).

**17. skip loop + read (cfprovider.rs:74–87)** — IN: `range`, `r`.
- TRANSFORM: discard `range.start` bytes via an 8 KiB sink (`r.read` in a loop, cfprovider.rs:75–83);
  then `len = range.end - range.start` (`saturating_sub`); `r.take(len).read_to_end(&mut buf)`.
- OUT: `buf : Vec<u8>` holding `[range.start .. range.end)` of the downloaded content (entire
  required range buffered in RAM, no chunking).

**18. `ticket.write_at(&buf, range.start)` (cfprovider.rs:88; crate ticket.rs:64)** — IN:
`buf:&[u8]`, `offset = range.start : u64`.
- TRANSFORM: builds `CF_OPERATION_PARAMETERS` `TransferData{ Buffer: buf.as_ptr(),
  Offset: range.start as i64, Length: buf.len() as i64, CompletionStatus: STATUS_SUCCESS,
  Flags: NONE }`; `CfExecute(CF_OPERATION_TYPE_TRANSFER_DATA, key)`. **No 4 KiB alignment is
  applied by us or the crate** (alignment is the caller's job; OK only because for a full open
  offset=0 and the buffer reaches EoF — see w1_fetchdata.md).
- OUT: `core::Result<()>`; on `Err` → `cerr` → `Unsuccessful` → proxy `Write::fail`. On `Ok`,
  the OS commits the bytes to the placeholder → hydrated → the app's read returns.
  **Win32:** `CfExecute(TRANSFER_DATA)`.

### Phase VII — edit save-back (out-of-band, NOT via CfAPI write callbacks)

**19. `App::poll_remote_edits` (app.rs:3226)** — runs every ~1.5 s.
- IN: `self.remote_edits : Vec<RemoteEdit>`. For each non-uploading entry: `m = file_mtime_ms(e.temp)`
  (the hydrated on-disk file's mtime). The `baseline_mtime == i64::MAX` sentinel arms on first
  sight (records the post-hydration mtime, does **not** upload it). A later mtime change that is
  stable for one poll cycle (`m == seen_mtime`) ⇒ a completed user save.
- OUT: pushes `(temp:PathBuf, backend, remote:String=e.remote_path, name)` to `launch`; sets
  `uploading=true`.

**20. `upload_file(&*be, &temp, &remote)` (app.rs:700, spawned thread)** — IN: `src=temp`
(the hydrated local file, e.g. `…\My Drive\Notes\Plan.docx`), `dest=remote=e.remote_path`
(RAW remote `"/Notes/Plan"`).
- TRANSFORM: `be.mkdir_all(parent)`; `be.open_write(dest)`; `io::copy(file → writer)`; `w.flush()`
  (flush is what triggers the Drive upload). **Note:** save-back writes the edited bytes to the
  RAW remote path; for B this uploads `.docx` bytes to the Drive item that is a native Google
  Doc — see §4 NEW-3.
- OUT: `Result<(),String>` sent back; `drain_edit_saves` (app.rs:3276) clears `uploading`,
  surfaces success/error notice.

> **CfAPI is NOT involved in save-back.** Deletes/renames/uploads inside the sync root are not
> wired through `SyncFilter` (those default to `NotSupported`); save-back is a plain mtime-watch
> + backend `open_write`. The module doc (cfprovider.rs:9–10) states this explicitly.

---

## 2. DATA-TYPE table (values crossing Rust ↔ FFI ↔ OS)

| Value | Rust type | Lowers to (Win32/WinRT) | Conversion / encoding |
|---|---|---|---|
| sync-root Id | `SyncRootId(HSTRING)` | WinRT `HSTRING` | `provider!SID!account` joined with `!`(0x21), UTF-16; built from `U16String`s (sync_root_id.rs:99) |
| SID | `SecurityId(U16String)` | `PWSTR` from `ConvertSidToStringSidW` | UTF-16 `"S-1-5-…"`; copied to `OsString`, `LocalFree`d |
| display name / icon / version | `&str`→`HSTRING` | WinRT `Set*Resource/SetVersion(HSTRING)` | UTF-8 `&str` → UTF-16 `HSTRING` |
| local_root path | `&Path` | `IStorageFolder` via `GetFolderFromPathAsync` | OS-native wide path; folder must pre-exist |
| **placeholder display name** | `String` (`display`) | `RelativeFileName : PCWSTR` | `U16CString::from_os_str` UTF-8→UTF-16, **NUL-terminated, panics on interior NUL** (placeholder_file.rs:24). Single component, no `\`. |
| **blob (remote path)** | `Vec<u8>` (`child_remote.into_bytes()`) | `FileIdentity:*const c_void` + `FileIdentityLength:u32` | raw **UTF-8 bytes** of the full remote path; `Box::leak`ed; **`assert len ≤ 4096`** (placeholder_file.rs:79). Round-trips byte-identical. |
| blob read-back | `&[u8]` (`request.file_blob()`) | `slice::from_raw_parts(FileIdentity, FileIdentityLength)` | same UTF-8 bytes; we `String::from_utf8_lossy` (cfprovider.rs:68) — **lossy substitutes U+FFFD for invalid UTF-8** (lossless here since we wrote UTF-8) |
| `size` | `u64` (`m.size`) | `CF_FS_METADATA.FileSize : i64` | cast `u64→i64` (metadata.rs `size()`). **Google Docs report 0** (gdrive.rs:267) → placeholder logical size 0 (see NEW-3) |
| timestamps | `nt_time::FileTime` (`FileTime::now()`) | `FILE_BASIC_INFO.CreationTime/LastWriteTime : i64` | NT **100-ns ticks since 1601** (`try_into().unwrap()`, panics if out of `i64`); `LastAccessTime`/`ChangeTime` left 0 |
| file attributes | (implicit) | `FILE_BASIC_INFO.FileAttributes : u32` | `FILE_ATTRIBUTE_NORMAL`(0x80) for files / `FILE_ATTRIBUTE_DIRECTORY`(0x10) for dirs |
| `required_file_range` | `Range<u64>` | `RequiredFileOffset:i64`, `RequiredLength:i64` | `start = off as u64`; `end = (off + len) as u64` — **add happens in `i64`** (info.rs:31), overflow/UB risk if OS sent huge values (NEW-4) |
| TRANSFER_DATA buffer | `&[u8]` (`buf`) | `Buffer:*mut c_void`, `Offset:i64`, `Length:i64` | raw bytes; offset/length passed verbatim, **no 4 KiB rounding** |
| `request.path()` | `PathBuf` | `VolumeDosName`(`C:`) + `NormalizedPath` `U16CStr` | UTF-16→`OsString`; full local path |
| error | `CloudErrorKind::Unsuccessful` | `NTSTATUS STATUS_CLOUD_FILE_UNSUCCESSFUL` | every cause collapsed (cfprovider.rs:29) |

### The blob round-trip, end to end (the central data path)
`gdrive.list_dir` → `m.name:String` → `child_remote = base + "/" + m.name : String`
→ `.into_bytes() : Vec<u8>` → `PlaceholderFile::blob` `Box::leak` → `FileIdentity` (OS persists
it with the placeholder on disk) → on read, `Request::file_blob()` `from_raw_parts` → `&[u8]`
→ `String::from_utf8_lossy` → `remote : String` → `backend.open_read(&remote)`. The bytes are
**identical** out and back; the only lossy step (`from_utf8_lossy`) is a no-op because we wrote
valid UTF-8. This is why hydration downloads the correct item even though the on-disk name
differs (B's blob `"/Notes/Plan"` ≠ on-disk `Plan.docx`).

### display-name vs blob divergence (Google-Docs)
- on-disk **name** = `download_name` = `Plan.docx` (so the OS file opens in Word/Excel).
- **blob** = `child_remote` = `/Notes/Plan` (the un-exported Drive path, so `open_read` exports
  the live Doc). The two are intentionally different and both correct.

---

## 3. PATH MAPPING — is local↔remote consistent across the three producers?

Three independent functions compute paths; they MUST agree on the local leaf, or the open
`ShellExecute`s a path the provider never created (silent failure: `dest.exists()==false` →
the app.rs:3137 error, OR worse a stale/empty file).

| Producer | Side | Sanitizes? | Rule |
|---|---|---|---|
| `cfsync::local_path` / `local_path_named` (cfsync.rs:46/64) | OPEN side (what we `ShellExecute`) | **YES — `san()` per path segment AND on the leaf** (cfsync.rs:53,66) | replaces `/\:*?"<>|` + control chars → `_`; trims spaces and `.` ; empty→`_` |
| `cfprovider::remote_of` (cfprovider.rs:41) | CALLBACK side (local→remote for `list_dir`) | **NO** | raw `strip_prefix` + `\`→`/`, trims one leading/trailing `/` |
| placeholder **display name** (cfprovider.rs:115–119) | CALLBACK side (what the provider creates on disk) | **NO** (`m.name` / `download_name`, raw) | bare remote name, only Google-Docs extension appended |

The local leaf is produced by **two different sanitization regimes**: the OPEN side applies
`san()`; the CALLBACK side (the placeholder the OS actually creates) applies **none**. They
agree **only when `san(name) == name`**. Concrete divergences:

**Case 1 — name contains an NTFS-illegal char that `san` rewrites (THE prime silent-failure).**
Remote `"/Reports/Q3:Q4.xlsx"` (a Drive title may contain `:`).
- Provider (`fetch_placeholders`): `display = "Q3:Q4.xlsx"` → `PlaceholderFile::new("Q3:Q4.xlsx")`.
  But `:` is illegal in an NTFS filename, so the platform **rejects this entry**
  (`STATUS_OBJECT_NAME_INVALID`/`0x80070057`); no placeholder is created (w1_placeholders ISSUE-3).
- Open side: `local_path_named` → `san("Q3:Q4.xlsx") = "Q3_Q4.xlsx"`. We `ShellExecute`
  `…\Reports\Q3_Q4.xlsx`.
- Result: provider created **nothing** (or a different name); `dest.exists()` is `false` →
  app.rs:3137 error. **Even if the platform had created a name, it would never be
  `Q3_Q4.xlsx`** because the provider never sanitizes. The two sides can **never** meet for
  such names. **BUG (silent / user-facing failure).**

**Case 2 — name with a `/` (Google Drive allows `/` in titles).**
Remote item titled `"Q3 / Q4"` under `/Reports`.
- `child_remote = "/Reports/Q3 / Q4"` — the `/` in the title is **indistinguishable from a path
  separator** in our string model. `remote_of`/`list_dir` already mis-split this; `local_path`
  splits on `/` and would create nested `Q3 ` / ` Q4` dirs; the provider's `display="Q3 / Q4"`
  hits NTFS-illegal `/`. Total mismatch. **BUG** (pre-existing string-model limitation, but it
  surfaces here as a path-mapping divergence).

**Case 3 — Google Doc extension (verified OK).**
Remote `/Notes/Plan` (Doc).
- Provider: `display = download_name("/Notes/Plan","Plan") = "Plan.docx"` → placeholder
  `…\My Drive\Notes\Plan.docx`.
- Open side: `local_name = download_name(path,name) = "Plan.docx"`; `local_path_named(...,"Plan.docx")`
  → `…\My Drive\Notes\` + `san("Plan.docx")="Plan.docx"`. **MATCH.** ✓
  (Both call the same `download_name`; `mime_of` is cached by the `list_dir` that ran during
  `populate_to`, so both see the same export ext. If the cache were cold on the open side,
  `mime_of` does a stat fallback — gdrive.rs:184 — so still consistent.)

**Case 4 — trailing dot / space (`san` trims, provider doesn't).**
Remote `"/Reports/draft."` or `"/Reports/note "`.
- Provider `display="draft."`: NTFS silently strips the trailing dot → on-disk `draft`; or the
  entry may be rejected.
- Open side: `san("draft.")` trims the `.` → `"draft"` (cfsync.rs:35). These **may** coincide
  (both `draft`) by luck, or may not (if the platform rejects). Fragile. **RISK.**

**Case 5 — connection-root / empty-root handling (verified OK for the common Drive case).**
`root_path = "/"`. `ensure_mounted` passes `remote_root = trim_end_matches('/') = ""`;
`RemoteFilter.remote_root = ""`. In `remote_of`, `root=""`, so a child rel `"Reports"` →
`format!("{}/{}", "", "Reports") = "/Reports"` ✓ (re-prepends the POSIX root). On the open
side `local_path("My Drive","/","/Reports/…")` strips `conn_root.trim_end='' ` (no-op) then
`trim_start_matches('/')` → `"Reports/…"`. Consistent. ✓ But note the **two roots are
configured independently**: `ensure_mounted` uses `self.root_path.trim_end_matches('/')`
(app.rs:3100) while `local_path_named` uses the **untrimmed** `self.root_path` (app.rs:3109).
For `root_path="/home/me"` (SFTP), `remote_of` strips `local_root` and prepends
`remote_root="/home/me"`; `local_path` strips `conn_root="/home/me"`. Consistent for normal
paths, but the trimmed-vs-untrimmed asymmetry is a latent edge (e.g. a `root_path` with a
trailing slash like `"/home/me/"` → `remote_root="/home/me"` but `local_path` strips
`"/home/me"` after its own `trim_end_matches('/')`, so OK; the asymmetry is currently benign
but undocumented). **OK / latent.**

**Case 6 — `label` sanitization mismatch between the two sanitizers (folder identity).**
The connection folder is `san(label)` (cfsync.rs:41) e.g. `"My Drive"`→`"My Drive"`, but the
sync-root **provider_id** is `provider_id(label)` (cfprovider.rs:168) → `"SmartExplorer_My_Drive"`
(space→`_`). These use **different** sanitizers. Both the open side (`conn_root_dir`→`san`) and
the connect side (`with_path(local_root)` where `local_root=conn_root_dir`) use the SAME
`san`-based folder, so the **registered path and the opened path agree** ✓. The divergence is
only in the *identity string*, which doesn't affect the local leaf. **OK** for path-mapping,
but see NEW-1 for the collision angle.

**Net:** the local leaf is consistent **only for names where `san(name)==name` and the name is
NTFS-legal**. For names containing `:* ? " < > |`, `/`, `\`, control chars, or trailing
dot/space, the OPEN side rewrites (via `san`) while the CALLBACK side does not — so we
`ShellExecute` a path the provider never created → `dest.exists()==false` → user error (best
case) or open-the-wrong/empty-file. This is the prime silent-failure suspect and it is **real**.

---

## 4. NEW issues found while tracing (beyond Wave 1)

**[NEW-1 — BUG] `san()` (open side) and `provider_id`/`remote_of` (callback side) use
DIFFERENT, inconsistent sanitization → local leaf mismatch.** Wave 1 noted NTFS-illegal names
fail the *placeholder* (ISSUE-3) and that `provider_id`'s sanitizer is non-injective
(registration §3), but did NOT connect the dots: the path we `ShellExecute`
(`local_path_named`→`san`, cfsync.rs:53,66) is sanitized, while the placeholder the provider
creates (`display`, cfprovider.rs:115) and the local→remote map (`remote_of`, cfprovider.rs:41)
are NOT. So for any name where `san(name)≠name`, the open targets a leaf that does not exist on
disk. `dest.exists()` then fails → app.rs:3137 error path. Fix: the provider's `display` and the
open side's leaf must use one shared, reversible name-mapping (and `remote_of` must invert it),
or restrict to names where `san` is identity. Cite: cfsync.rs:30–37,53,66; cfprovider.rs:41–54,115–119.

**[NEW-2 — RISK] `remote_of` does not reverse `san()`, so even a *created* sanitized placeholder
hydrates the wrong remote.** Suppose we fixed NEW-1 so the provider also sanitized the on-disk
name to `Q3_Q4.xlsx`. On hydration, `fetch_data` uses the **blob** (the raw `child_remote`), so
that path still works — BUT if the blob were ever empty (e.g. a future code path, or a blob we
chose to drop to avoid the 4 KiB panic, w1 ISSUE-2), `fetch_data` falls back to
`remote_of(request.path())` (cfprovider.rs:65–66), which would produce `"/Reports/Q3_Q4.xlsx"`
(the sanitized name) — a remote path that does not exist → download fails. The blob fallback and
the display-name are not reconcilable through `remote_of`. Cite: cfprovider.rs:41–54,65–69.

**[NEW-3 — BUG (Google-Docs)] Placeholder logical `size` = 0 for Google Docs, but hydration
delivers non-zero exported bytes → size/transfer contract violation.** `gdrive::meta_from_json`
sets `size:0` when the Drive API omits `size` (true for all native Docs/Sheets/Slides,
gdrive.rs:267). `fetch_placeholders` sets `Metadata::file().size(0)` (cfprovider.rs:108), so the
placeholder's **logical size is 0**. On open, `required_file_range()` for a 0-byte logical file
is `0..0`, so `len=0`, we download nothing, and `write_at(&[], 0)` transfers an empty buffer —
**the exported `.docx`/`.xlsx`/`.pdf` content is never delivered.** The user opens an empty file.
(Even if the OS requested a non-zero range, declared size 0 vs delivered >0 breaks the
"transfer must end on logical file size" rule, w1_fetchdata §a.) Wave 1's crate-map #9 flagged a
size *mismatch* generically; this confirms it concretely resolves to **0**, which means the Doc
opens **empty**, not merely truncated. Fix: for export-type files, set the placeholder size to a
real (or over-estimated then EoF-trimmed) exported size, or fetch the export size first. Cite:
gdrive.rs:267,518; cfprovider.rs:108; info.rs:29.

**[NEW-3b — BUG] Google-Docs save-back uploads `.docx` bytes onto a native Doc via raw remote
path.** `RemoteEdit.remote_path` is the RAW `path` (`/Notes/Plan`, app.rs:3121), and
`upload_file` does `be.open_write("/Notes/Plan")` (app.rs:706) writing the edited `.docx`. On
Drive, `open_write` to the Doc's path will either create a *new* binary file named `Plan` or
fail/replace unexpectedly — it cannot round-trip an edited export back into the live Google Doc.
The blob/open path correctly *downloads* a Doc but the save-back path has no inverse import.
Cite: app.rs:3121,3254,700–709; gdrive open_write.

**[NEW-4 — RISK] `required_file_range()` computes `RequiredFileOffset + RequiredLength` in
`i64` *before* the `as u64` cast (info.rs:31).** If the OS ever supplies a large offset+length
near `i64::MAX` (e.g. for a multi-exabyte sparse request, or a corrupted/huge logical size we
set), the `i64` addition overflows → in debug builds **panics across FFI = UB**; in release,
wraps to a bogus `Range`. We then `r.take(len)` an absurd length. Our declared sizes are small
so this is latent, but it is a crate-side arithmetic hazard our code inherits and does not guard.
Cite: crate info.rs:29–32; consumed at cfprovider.rs:70.

**[NEW-5 — RISK] `remote_of` mis-roots when `local.strip_prefix(local_root)` fails.**
`remote_of` (cfprovider.rs:42) does `strip_prefix(&self.local_root).unwrap_or(local)` — if the
callback ever delivers a path NOT under `local_root` (it shouldn't, but the crate joins
`VolumeDosName`+`NormalizedPath` and casing/`\\?\` prefixes can differ), `unwrap_or(local)`
falls back to the **full local path** (e.g. `C:\Users\…`), which is then sent verbatim to
`list_dir`/`open_read` as a "remote" path → guaranteed not-found. A defensive log/err would be
safer than silently treating an absolute local path as remote. Cite: cfprovider.rs:42–48.

**[NEW-6 — RISK] `populate_to` cannot create the leaf if any *ancestor* directory name needed
sanitization.** `populate_to` walks `parent.strip_prefix(local_root).components()` and
`read_dir`s each level (cfprovider.rs:153–158). But those component names come from the
**sanitized** `dest` path (built by `local_path_named`→`san`). If an ancestor's real remote name
differs from its `san`-ed local name (NEW-1, applied to a *folder* like `"/2024:Q1/…"`), the
`read_dir` of the sanitized ancestor path enumerates a directory that the provider created under
a *different* name (or didn't create) → the descent dead-ends and the leaf never populates. Same
root cause as NEW-1 but specifically breaks the multi-level pre-population for deep path **C** if
any segment is non-trivial. Cite: cfprovider.rs:142–159; app.rs:3107–3115.

**[NEW-7 — FIDELITY/RISK] The `RemoteEdit` first-sight sentinel can miss a fast edit.**
`baseline_mtime=i64::MAX` is armed only on the first poll where the hydrated file is visible
(app.rs:3243). If hydration + a user save both complete within the first 1.5 s poll window, the
*saved* mtime is taken as the baseline and the edit is never uploaded (treated as the initial
content). Narrow window, but a correctness gap distinct from the temp-mode flow (where the
baseline is set from the known-downloaded content). Cite: app.rs:3243–3247.

---

### Cross-checks against Wave 1 (verified, not re-listed as new)
- Blob >4 KiB panic, NUL-in-name panic, single-shot population, all-errors→`Unsuccessful`,
  no chunking, no progress, timestamps=now: all confirmed present in source, already in W1.
- `HydrationType::Full` + `PopulationType::Full` pairing, `mark_in_sync`, default attributes,
  `populate_to` blocking semantics, connection-kept-alive-in-registry: confirmed correct, per W1.
