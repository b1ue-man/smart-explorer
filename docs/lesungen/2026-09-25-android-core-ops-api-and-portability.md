# Android core-ops portability audit + JNI-facade API facts

**Purpose.** Read-only source audit of the file-operation core (`vfs`, `sftp`, `ftp`, `webdav`,
`copy`, `scanner`, `folder_index`, `filter`, `analytics`, `rscan`, `zipfs`, `linemerge`, `types`,
`format`, `icons`, `dragout`, `connect`, `gdrive`, `cloud`) for two purposes: (1) what happens to
every conditional-compilation site and every Linux/glibc-leaning API when compiled for
`target_os = "android"` (unix family, but **not** `target_os = "linux"` and **not** `windows`);
(2) the exact public API surface (`Backend` trait, backend construction, scanner/filter/copy/
analytics/zip/folder_index/rscan/linemerge entry points) a future JNI facade would wrap. All facts
are cited `file:line`. Judgements are limited to fact + candidate remedies, never an overall
verdict.

## Files read

- `native/src/vfs/**` (core, os/shared, os/windows, os/linux_os, mod.rs)
- `native/src/sftp/**`, `native/src/ftp/**`, `native/src/webdav/**`
- `native/src/copy/**` (mod.rs, os/linux_os.rs, os/windows.rs, os/shared/*.rs)
- `native/src/scanner/**`, `native/src/folder_index/**`, `native/src/filter/**`
- `native/src/analytics/**` (incl. `os/shared/reclaim/*.rs`), `native/src/rscan/**`
- `native/src/zipfs/**`, `native/src/linemerge/**`, `native/src/types/**`, `native/src/format/**`
- `native/src/icons/**`, `native/src/dragout/**`
- `native/src/connect/**` (construction/persistence only)
- `native/src/gdrive/mod.rs`, `native/src/gdrive/core/state.rs`, `native/src/gdrive/core/backend.rs` (construction only)
- `native/src/cloud/mod.rs`, `native/src/cloud/core/cloud.rs`, `native/src/cloud/os/*.rs` (construction only)
- `native/Cargo.toml` (read beyond the listed surface — required to answer the explicit
  dependency-gating question; read-only, no edits; see **Decisions** in the returned result)

Out of scope, referenced but **not read** (noted where relevant): `crate::creds` (saved-connection
persistence + credential store), `crate::support_dirs` (app-data-dir resolution), `crate::net`
(UNC/Share), `crate::local_access` (the actual local-FS walk primitives `read_directory`/
`EntryKind`/`LocalEntry` that `analytics/os/mod.rs` re-exports), `crate::agent`/`crate::agent_proto`
(SSH remote agent), `crate::mount` (Dokany — explicitly out of scope for Android per the brief).

---

## A. Conditional-compilation / `#[path]` module-selection sites

Every `mod.rs` in the assigned surface that branches by platform, plus every `cfg!`/`#[cfg(unix)]`
site found. "Android resolution" assumes `target_os="android"`, `target_family="unix"`,
NOT `target_os="linux"`, NOT `windows`.

| Site (file:line) | cfg / selector | Android resolution | Consequence |
|---|---|---|---|
| `vfs/mod.rs:33-38` | `#[cfg(windows)]` → `os/windows/local_platform.rs`; `#[cfg(not(windows))]` → `os/linux_os/local_platform.rs` | takes the `not(windows)` arm | **compiles fine** — despite the `linux_os` directory name, the cfg gate is `not(windows)`, so Android gets this module |
| `vfs/mod.rs:46-48` | `#[cfg(windows)]` → `os/windows/verbatim.rs` | module absent | fine — only used from `os/windows/local_platform.rs:43`, itself Windows-only; no unconditional caller |
| `scanner/mod.rs:9-14` | `#[cfg(windows)]` → `os/windows.rs` **as `platform`**; `#[cfg(target_os = "linux")]` → `os/linux_os.rs` **as `platform`** | **neither arm matches** — `mod platform;` does not exist at all | **MISSING SYMBOL** — see §A.1 |
| `folder_index/mod.rs:17-22` | same pattern: `#[cfg(windows)]` / `#[cfg(target_os = "linux")]` → `platform` | **neither arm matches** | **MISSING SYMBOL** — see §A.1 |
| `cloud/mod.rs:6-16` | inner `mod os { #[cfg(target_os="linux")] mod platform; #[cfg(windows)] mod platform; pub use platform::open_url; }` | **neither arm matches**, `platform` module absent | **MISSING SYMBOL** — see §A.1 |
| `copy/mod.rs:3-8` | `#[cfg(not(windows))]` → `os/linux_os.rs` **as `platform`**; `#[cfg(windows)]` → `os/windows.rs` | takes `not(windows)` arm | **compiles fine** (name is misleading, gate is unix-family-wide); content audited in §B |
| `icons/mod.rs:3-8` | `#[cfg(not(windows))]` → `os/shared.rs`; `#[cfg(windows)]` → `os/windows.rs` | takes `not(windows)` arm | **compiles fine**; `os/shared.rs` is a no-op stub (§D.9) |
| `dragout/mod.rs:16-21` | `#[cfg(windows)]` → `os/windows.rs`, both `mod imp` and `pub use imp::*` | neither line compiles | module exports **nothing** on Android (same as today's Linux desktop build — not an Android regression, but there is zero drag-out capability to reuse; see §D.10) |
| `vfs/os/shared/local.rs:241` | `#[cfg(all(test, unix))]` | test-only | no production effect |
| `copy/os/shared/path_guard.rs:195` | `cfg!(windows)` (runtime branch, not compile-time) | resolves to the `else` (case-sensitive, unmodified) branch | fine — matches ext4/f2fs case-sensitive semantics |
| `copy/os/shared/path_guard.rs:202`, `copy/os/shared/safe_file_tests.rs:117`, `copy/os/shared/relative.rs:91`, `scanner/os/shared_tests.rs:3/42/57`, `folder_index/core/tests.rs:3/101/132/166`, `vfs/core/tests.rs:45`, `vfs/core/delete_tests.rs:194/197`, `sftp/os/shared/known_hosts.rs:167` | `#[cfg(unix)]` / `#[cfg(all(test,unix))]` on test code | test-only, included on Android too (unix family) | fine — these tests exercise `std::os::unix::fs::symlink` etc., all present on Android/bionic |

### A.1 — Missing-symbol findings (compile-blocking on Android as written)

Three modules have an unconditional `use`/`pub use` of a `platform` (sub)module that is only
`#[path]`-selected for `windows` or literal `target_os = "linux"`. On `target_os="android"` the
module simply does not exist → **unresolved module / missing items**, a hard compile error, not a
behavior difference.

| Unconditional use site (file:line) | Missing item(s) | Definition site (only compiled for windows / linux) |
|---|---|---|
| `scanner/os/collect.rs:2` `use super::platform::{get_attrs, is_link_like, path_text};` | `platform` module itself | `scanner/mod.rs:9-14` selects `scanner/os/windows.rs` (defines `get_attrs`/`is_link_like`/`path_text` at `native/src/scanner/os/windows.rs:2,16,24`, all further gated `#[cfg(windows)]`) or `scanner/os/linux_os.rs` (same three fns at `native/src/scanner/os/linux_os.rs:2,7,12`, gated `#[cfg(not(windows))]` — i.e. already Android-safe content, just not selected) |
| `scanner/os/shared.rs:3` (same `use`) | same | same |
| `scanner/os/walk.rs:4` `use super::platform::{get_attrs, path_text};` | same | same |
| `folder_index/os/rank.rs:3` `use super::super::platform::is_plain_directory;` | `platform` module itself | `folder_index/mod.rs:17-22` selects `folder_index/os/windows.rs` (`should_skip_meta`/`is_plain_directory`/`replace_file` at `native/src/folder_index/os/windows.rs:2,12,21`, `#[cfg(windows)]`) or `folder_index/os/linux_os.rs` (same three fns at `native/src/folder_index/os/linux_os.rs:2,7,12`, `#[cfg(not(windows))]`, already Android-safe content) |
| `folder_index/os/walk.rs:8` `use super::super::platform::{is_plain_directory, should_skip_meta};` | same | same |
| `folder_index/os/persistence.rs:8` `use super::super::platform::replace_file;` | same | same |
| `cloud/core/cloud.rs:278` `super::os::open_url(&url);` | `os::open_url` (re-exported at `cloud/mod.rs:15` from the never-selected `platform` submodule) | `cloud/mod.rs:7-9` `linux_os.rs` (`open_url` via `xdg-open`, `native/src/cloud/os/linux_os.rs:1-3`) or `cloud/mod.rs:10-12` `windows.rs` (`ShellExecuteW`, `native/src/cloud/os/windows.rs:1-20`) |

**Remedy candidates (not adopted, candidates only):**
- Scanner + folder_index: change the module-selection cfg from `#[cfg(target_os = "linux")]` to
  `#[cfg(not(windows))]` (matching what `copy/mod.rs:3` and `icons/mod.rs:3` already do). The
  `*_linux_os.rs` content in both modules is *already* written against `#[cfg(not(windows))]`
  internally (`scanner/os/linux_os.rs:1,6,11`; `folder_index/os/linux_os.rs:1,6,11`), so this is a
  pure module-selection fix, no logic change, and it keeps Windows/desktop-Linux behavior
  unchanged. Whether the same body is *correct* on Android (no reparse points, no Win32 attribute
  bits — already the case for desktop Linux too) is unaffected.
- Cloud `open_url`: same cfg widening is **not** sufficient by itself — `xdg-open`
  (`cloud/os/linux_os.rs:2`) is a desktop binary that does not exist on Android; an Android variant
  needs its own adapter (e.g. an `os/android.rs` calling into JNI to start an `Intent.ACTION_VIEW`),
  gated `#[cfg(target_os = "android")]`, in addition to fixing the `mod os { ... }` selection.

---

## B. Linux-only / glibc-leaning APIs actually compiled on Android (unix-family code)

These sites sit inside `#[cfg(not(windows))]` or fully unconditional code, so they **would** be
compiled for Android once/if the §A.1 module-selection gaps are closed (or already are compiled,
for `copy/mod.rs`'s `linux_os.rs`, since its cfg is already `not(windows)`).

| Site (file:line) | API | Android resolution | Consequence | Remedy candidates |
|---|---|---|---|---|
| `copy/os/linux_os.rs:79-91` (`fn rename_no_replace`) | `libc::syscall(libc::SYS_renameat2, ..., libc::RENAME_NOREPLACE)` — raw syscall, not the libc wrapper, because "the libc crate omits the renameat2 wrapper on musl" (comment at line 79-81) | compiles only if the `libc` crate defines `SYS_renameat2`/`RENAME_NOREPLACE` for `target_os="android"` (unverified — external crate, out of the assigned read surface); Android runs the Linux kernel so the syscall itself exists at the kernel level for both aarch64/x86_64 | **unverified — open question**; if the constant is present it should work identically to Linux; if it is *not* exposed by `libc` for android in the pinned `libc = "0.2"` (`native/Cargo.toml:165`), this is a build-blocking missing constant. Separately, some Android versions/OEMs apply a seccomp-bpf filter to app processes that can reject syscalls not on an allow-list, independent of whether it links | verify `libc::SYS_renameat2` is `cfg(target_os="android")`-exposed in `libc 0.2` before relying on it; fallback candidate: a non-atomic `stat`-then-`rename` (weakens the documented no-replace atomicity guarantee, see AGENTS.md "preserve retryability" — would need explicit sign-off) or gate this exact function per-target |
| `vfs/os/linux_os/local_platform.rs:25-58` (`fn rename_no_replace`) | identical `libc::syscall(SYS_renameat2, ...)` pattern, duplicated | same as above | same open question, same remedy | same |
| `copy/os/linux_os.rs:1-97` (whole file) | `std::os::unix::fs::MetadataExt` (`dev()`, `ino()` at lines 4, 17) | **fine** — `MetadataExt` is defined for all unix targets including android | compiles fine | — |
| `copy/os/linux_os.rs:57-59` (`is_cross_device`) | `error.raw_os_error() == Some(18)` (EXDEV) | fine — EXDEV is `18` on Android/arm64 & x86_64 too (shared Linux ABI) | compiles/behaves fine | — |
| `cloud/os/linux_os.rs:2` | `std::process::Command::new("xdg-open")` | `xdg-open` binary does not exist on Android (no XDG desktop, no shell PATH with that tool); `Command::new` itself compiles (portable), the spawn silently fails at runtime (`let _ =` discards the error) | **wrong behaviour** (silently does nothing — no error surfaced) rather than a compile failure | Android needs its own `open_url` via JNI `Intent.ACTION_VIEW` / Custom Tabs, see §A.1 remedy |
| `cloud/core/cloud.rs:271-291` (`authorize`) | `std::net::TcpListener::bind("127.0.0.1:0")` for the OAuth loopback redirect | **compiles and binds fine** on Android (loopback bind needs no special permission) | **architecture risk, not a compile issue**: the system browser opened via `open_url` (blocked per above) is expected to redirect back to `http://127.0.0.1:<port>`; on Android the browser runs as a separate app/process and, depending on OS version and network configuration, reliably reaching the launching app's own loopback listener from the system Chrome/Custom-Tabs process is not guaranteed the way it is on desktop | remedy candidate: switch the Android build to an app-link / custom-URI-scheme OAuth redirect (Custom Tabs + `Intent` deep link) instead of the loopback listener — this is a real protocol change, not just a JNI stub, flagged for the main agent |
| `cloud/core/cloud.rs:67` | `getrandom::getrandom(&mut buf)` (the `getrandom` **crate**, `native/Cargo.toml:76`, unconditional dependency) | cross-platform crate; android support depends on the pinned `getrandom = "0.2"` having an android backend (unverified, external crate, out of scope) | likely fine — noted as an open verification item, not a known break | — |
| `sftp/os/shared/known_hosts.rs:7-12` | `crate::support_dirs::app_data_dir()` (out-of-scope module) → `known_hosts_sftp.txt` | unknown — depends on `support_dirs` implementation, not read | **unresolved dependency**: Android has no `$HOME`/XDG concept; this must resolve to the app's private storage (`Context.getFilesDir()`), which requires an Android-specific `support_dirs` adapter reachable through the JNI boundary | flag `native/src/support_dirs` for the main agent to audit separately |
| `cloud/os/shared.rs:4-8` (`cloud_dir`) | `crate::support_dirs::app_data_dir().join("cloud")` | same unresolved dependency as above | same | same |
| — | `/proc`, `/sys`, `systemd`, `D-Bus`/`zbus`, `polkit`, `NetworkManager`, `landlock`, `prctl`, `HOME` env var reads | **none found** anywhere in the assigned read surface (checked via `rg` across all 18 module roots) | — | — |
| — | `statx`, `memfd_create`, `pidfd_*`, `copy_file_range`, `O_TMPFILE`, `fallocate` | **none found** in the assigned read surface | — | — |
| `vfs/os/shared/copy_transfer.rs:20` | `tempfile::tempfile()` (unnamed spool file for cross-backend copy) | cross-platform crate; whether it uses Linux's `O_TMPFILE` internally and whether that path is also taken for `target_os="android"` is an external-crate detail, not read | likely fine (falls back to a named-then-unlinked temp file where `O_TMPFILE` is unavailable) — flagged as unverified, not a known break | — |

**Dependencies confirmed absent from the assigned modules' own code:** no `Command::new` spawning
a shell, `sh`, `systemd-run`, or any external binary other than `xdg-open` above; no `prctl`, no
`landlock`; the one `Command::new(...)` in the whole read surface is `cloud/os/linux_os.rs:2`.

---

## C. `native/Cargo.toml` per-target dependency gating (read beyond the listed surface; see note above)

| Dependency | Gate (`native/Cargo.toml:line`) | Pulled in for Android? | Note |
|---|---|---|---|
| `libc = "0.2"` | `[target.'cfg(not(windows))'.dependencies]` (:164-165) | **yes** (unix family) | needed for `SYS_renameat2` (§B); comment at :165 documents the reason |
| `rfd` | same block, features `["xdg-portal","tokio"]` (:166) vs. the Windows block's `["common-controls-v6"]` (:112) | **yes**, with the `xdg-portal` feature | `rfd`'s xdg-portal backend talks to the Linux XDG Desktop Portal D-Bus service, which does not exist on Android; not used inside the assigned read surface (`rg 'rfd::'` across all 18 module roots returned nothing), so this is a build-graph/packaging concern for whoever *does* call `rfd`, not a core-ops API concern — flagged for the main agent |
| `zbus = "5.16"` | `[target.'cfg(target_os = "linux")'.dependencies]` (:168-171), features `["tokio","blocking-api"]` | **no** — literal `target_os="linux"`, Android is excluded correctly | comment: "Remote execution uses the systemd Manager D-Bus API ... never shells out to systemd-run" — confirms this is desktop-Linux-only and correctly scoped away from Android already; not used in the assigned modules |
| `keyring = "3"` | `[target.'cfg(windows)'.dependencies]` (:108-111), `windows-native` feature | **no** | comment: "Linux deliberately uses the app's owner-protected, headless file store instead of a DBus/session keyring" (:109-110) — i.e. **non-Windows platforms including Android get no OS keyring at all**; credentials are already routed through a custom file-based store in `crate::creds` (out of scope, not read) |
| `trash = "5"` | `[dependencies]` (:26), unconditional | **yes** | **not referenced anywhere in the assigned read surface** (`rg 'trash::'`/`'use trash'` across `vfs`, `copy`, `analytics` returned nothing) — the recycle/trash call site lives outside this audit's scope; `analytics/os/shared/reclaim/verify.rs:16` only *plans* which paths to delete (`ReclaimTrashPlan`), it never calls the crate itself |
| `getrandom = "0.2"` | `[dependencies]` (:76), unconditional | yes | used at `cloud/core/cloud.rs:67`; android support unverified externally (§B) |
| `windows-sys`, `windows-core`, `winreg`, `windows` | `[target.'cfg(windows)'.dependencies]` (:108-161) | **no** | correctly excluded |
| `russh`, `russh-sftp`, `tokio`, `suppaftp`, `rustls`, `webpki-roots`, `roxmltree`, `base64`, `serde`/`serde_json`, `sha2`, `ureq`, `zip`, `chrono`, `regex`, `globset`, `similar`, `same-file` | `[dependencies]` (unconditional) | yes | all pure-Rust / already documented in `Cargo.toml` comments as avoiding native TLS/crypto (ring-backed rustls, no aws-lc) — no Android-specific red flag found for these within the assigned modules |

---

## D. API facts for the JNI facade

### D.1 — `Backend` trait (`native/src/vfs/core/core.rs:77-477`)

No `egui`/`eframe` dependency anywhere in `vfs/core/core.rs` (confirmed — the only `egui`/`eframe`
use in the whole assigned surface is `icons/core/icons.rs`, see D.9). `VfsResult<T> = io::Result<T>`
(`core.rs:57`); errors are plain `std::io::Error` with `io::ErrorKind` (e.g. `NotFound`,
`Unsupported`, `InvalidData`, `InvalidInput`, `PermissionDenied`) — no custom `VfsError` enum exists.

| Method (all `core.rs`) | Signature | Notes |
|---|---|---|
| `scheme` | `fn scheme(&self) -> Scheme` (:78) | `Scheme` from `core/scheme.rs`, re-exported `core.rs:6` |
| `root_display` | `fn root_display(&self) -> String` (:81) | |
| `state_identity` / `namespace_identity` | `fn(&self) -> String` (:86-92) | default `format!("{:?}:{}", scheme, root_display)` |
| `uncached_backend` | `fn(&self) -> Option<BackendHandle>` (:94-96) | `BackendHandle = Arc<dyn Backend>` (:499) |
| `list_dir` | `fn(&self, path: &str) -> VfsResult<Vec<VfsMeta>>` (:97) | required, no default |
| `stat` | `fn(&self, path: &str) -> VfsResult<VfsMeta>` (:98) | required |
| `try_exists` / `exists` | `fn(&self, path: &str) -> VfsResult<bool>` / `-> bool` (:103-115) | `try_exists` is the safety-critical fallible form |
| `item_id` | `fn(&self, path: &str) -> VfsResult<Option<String>>` (:119-122) | default `None` |
| `open_read` / `open_write` | `-> VfsResult<Box<dyn Read + Send>>` / `Box<dyn Write + Send>` (:124-125) | required |
| `open_write_new` / `open_write_copy_stage` | same return as `open_write`, default `Unsupported` / delegates to `open_write_new` (:131-146) | atomic exclusive-create semantics |
| `promote_copy_stage` | `fn(&self, staged: &str, destination: &str) -> VfsResult<()>` (:151-153) | |
| `download_name` | `fn(&self, path: &str, name: &str) -> String` (:158-160) | default identity |
| `read_size` | `fn(&self, path: &str, metadata_size: u64) -> VfsResult<Option<u64>>` (:165-167) | |
| `copy_file` | `fn(&self, src: &str, dst: &str) -> VfsResult<u64>` (:172-174) | default streams via `copy_transfer::copy_file` |
| `rename` | `fn(&self, src: &str, dst: &str) -> VfsResult<()>` (:176) | required |
| `rename_no_replace` | `fn(&self, src: &str, dst: &str) -> VfsResult<()>` (:182-188) | default `Unsupported` |
| `promote_staged` / `promote_staged_no_replace` | `fn(&self, staged: &str, destination: &str) -> VfsResult<()>` (:195-204) | defaults call `super::promotion::*` |
| `remove_file` / `remove_dir` / `mkdir_all` | `fn(&self, path: &str) -> VfsResult<()>` (:205-207) | required, no default |
| `delete_disposition` | `fn(&self) -> DeleteDisposition` (:212-214) | enum `{Recycle, Permanent, Unsupported}` (:59-64); default `Permanent` |
| `parallelism` | `fn(&self) -> usize` (:218-220) | default `rayon::current_num_threads()` |
| `rename_overwrites` | `fn(&self) -> bool` (:229-231) | default `false`; local filesystem overrides `true` |
| `staged_write_capabilities` | `fn(&self, root: &str) -> StagedWriteCapabilities` (:236-242) | struct from `core/capabilities.rs` (`create`/`replace`/`namespace_replace`) |
| `case_sensitive_paths` | `fn(&self, root: &str) -> bool` (:252-254) | default `false` (conservative) |
| `root_confinement` | `fn(&self, root: &str) -> RootConfinement` (:259-261) | default `Unverified` |
| `mount_path_capabilities` | `fn(&self, root: &str) -> VfsResult<MountPathCapabilities>` (:266-271) | combines the two above |
| `open_read_id` / `remove_file_id` | id-aware variants for duplicate-name backends (:276-290) | default ignores id, falls back to path |
| `plan_dedupe_recursive` / `apply_dedupe_plan` / `dedupe_recursive` | `VfsResult<Vec<DedupeCandidate>>` / `VfsResult<usize>` (:295-332) | `DedupeCandidate { path, id }` (:69-73) |
| `is_local` / `provides_content_hash` / `supports_changes` | `-> bool`, all default `false` (:337-351) | |
| `change_root_id` / `current_change_cursor` / `changes_since` | change-feed API (:355-376) | `VfsChangeBatch { changes: Vec<VfsChange>, new_cursor, reset }` (:50-55) |
| `invalidate_cache` | `fn(&self)` (:380) | no-op unless wrapped in `CachingBackend` |
| `supports_walk_tree` / `walk_tree` | server-side whole-tree walk (:385-399) | returns `crate::agent_proto::WireNode` (out of scope type) — SSH-agent only |
| `supports_bulk_tree` / `get_tree` / `put_tree` | one-session subtree transfer (:404-428) | SSH-agent only |
| `supports_search` / `search` | server-side recursive search (:433-451) | streams `SearchHit { rel, is_dir, size, mtime_ms }` (:481-486) over `crossbeam_channel::Sender` |
| `supports_walk_hashed` / `walk_hashed` | server-side signature walk (:457-476) | streams `HashHit { rel, is_dir, size, mtime_ms, md5 }` (:491-497) |

**`VfsMeta`** (`core.rs:11-29`): `name: String, is_dir: bool, is_symlink: bool, size: u64,
mtime_ms: i64, btime_ms: i64, hidden: bool, system: bool, id: Option<String>,
content_md5: Option<String>`. Fields a remote can't supply default to `0`/`false`.

### D.2 — Backend construction

| Backend | Constructor (file:line) | Config/connection struct (fields) |
|---|---|---|
| Local | not a distinct constructor in scope — `LocalBackend` exported `vfs/mod.rs:64` from `vfs/os/shared/local.rs` (not read in full; out of the enumerated construction focus) | — |
| SFTP | `SftpBackend::connect(cfg: SftpConfig) -> io::Result<SftpBackend>` (`sftp/core/backend.rs:31`) | `SftpConfig { host: String, port: u16, user: String, auth: SftpAuth, root: String }` (`sftp/core/config.rs:12-19`); `SftpAuth::{Password(String), Key{path: String, passphrase: Option<String>}}` (`config.rs:1-10`) |
| FTP/FTPS | `ftp::backend_from_url(url: &str) -> io::Result<FtpBackend>` (`ftp/core/ftp.rs:107`) | no struct — takes one `ftp(s)://user:pass@host:port/root` URL string, built at the call site `connect/os/shared/connector.rs:191-211` |
| WebDAV | `WebdavBackend::connect(cfg: WebdavConfig) -> io::Result<WebdavBackend>` (`webdav/core/webdav.rs:63`) | `WebdavConfig { https: bool, host: String, port: u16, user: String, password: String, root: String }` (`webdav/core/webdav.rs:38-45`) |
| Google Drive | `GDriveBackend::connect(root: &str) -> Result<Self, String>` (`gdrive/core/state.rs:82`) | no config struct — pulls the stored OAuth refresh token via `cloud::refresh_access(Provider::GDrive)` (`state.rs:83`), loads an id/mime cache via `super::cache::load()` (`state.rs:85`) |
| ZIP | `ZipBackend::open(zip_path: &str) -> io::Result<ZipBackend>` (`zipfs/os/shared/zipfs.rs:121`) | no config struct — one local path; parses the whole archive directory map on open |
| Share (Direct/Room) | **not found in the assigned read surface** | construction likely lives in `crate::net`/`crate::share` (out of scope) — flagged unresolved |
| Generic connect dispatch | `connect/os/shared/connector.rs:104-110` (`do_connect_with_agent_fallback`) dispatches on `form.protocol` to `connect_sftp`/`connect_ftp`/`connect_webdav`/`connect_share` (`connector.rs:112-301`) | `ConnectForm` (`connect/core/types.rs:30-45`): `protocol: Protocol, host, port, user, password, use_key, keyfile, passphrase, root, unc, save, label, use_agent` |
| Saved-connection reconnect | `open_saved_at` / `open_saved_at_for_mount` (`connector.rs:318-332`) → `open_saved_at_with_agent_fallback` (`connector.rs:334-377`) | `crate::creds::SavedConnection` (out of scope struct, only field usage visible: `.protocol`, `.account()`) |

**Persistence / credentials (out-of-scope module, facts limited to the call boundary):**
`connect/os/shared/persistence.rs:6-28` builds a `SavedConnection` (protocol, host, port, user,
auth, root, label, use_agent — **no secret**) and calls
`crate::creds::save_connection_with_secret(&saved, secret)` (`persistence.rs:35`); credential lookup
is `crate::creds::get_secret_checked(&c.account())` (`connector.rs:341,360`). The actual file
format/location and the "owner-protected, headless file store" mentioned in
`native/Cargo.toml:109-110` live in `native/src/creds` — **not in the assigned read surface**;
flagged unresolved for the main agent (needed to know whether that store's path logic is portable
to Android's private storage).

### D.3 — Scanner (local) entry points, limits, cancellation, progress

- `start_scan(root: PathBuf, opts: ScanOpts, tx: Sender<ScanMessage>) -> ScanHandle`
  (`scanner/os/shared.rs:58`). `ScanHandle { cancel: Arc<AtomicBool>, truncated: Arc<AtomicBool> }`
  (`shared.rs:23-27`). `ScanOpts { follow_symlinks: bool, max_depth: Option<u32>,
  retention: Option<RetentionHandle> }` (`shared.rs:36-44`).
- `ScanMessage::{Entries(Vec<FileEntry>), Progress(ScanProgress), Error(String),
  FailedPaths(Vec<(String,String)>), Done(ScanProgress)}` (`shared.rs:14-19`), sent over
  `crossbeam_channel::Sender`.
- Limits (`scanner/os/budget.rs:3-5`): `MAX_SCAN_ENTRIES = 1_000_000`,
  `MAX_SCAN_TEXT_BYTES = 128 MiB`, `MAX_SCAN_DEPTH = 512`.
- `ScanRetention` trait (`scanner/core/retention.rs:15-21`): `retain(&self, entry: &FileEntry) -> bool`,
  `descend(&self, directory: &FileEntry) -> bool`. `RetentionHandle = Arc<dyn ScanRetention>` (:23).
  `Lineage` (:27-75) emits not-yet-emitted ancestor directories exactly once so a filtered tree view
  can still place a retained descendant (`Lineage::pending`, `Lineage::emit_pending`).
- Collection-with-cancellation primitive used by copy: `collect_recursive` /
  `collect_recursive_with_access(root: &Path, follow_symlinks: bool, start_depth: u32,
  cancel: &AtomicBool) -> CollectOutcome` (`scanner/os/collect.rs:36-53`); limits
  `MAX_COLLECTED_ENTRIES = 1_000_000`, `MAX_COLLECTED_NAME_BYTES = 128 MiB`,
  `MAX_COLLECTED_DEPTH = 512` (`collect.rs:8-10`); `CollectOutcome { entries, issues,
  suppressed_issues, canceled }` with `is_complete()` (`collect.rs:20-29`).
- Remote/backend-driven recursive scan reuses the same message types:
  `start_scan_backend(backend: BackendHandle, root: String, max_depth: Option<u32>,
  retention: Option<RetentionHandle>, tx: Sender<ScanMessage>) -> ScanHandle`
  (`rscan/os/shared/rscan.rs:53-79`); server-side search:
  `start_search_backend(...)` (`rscan/os/shared/search.rs:20`, re-exported `rscan.rs:14`).
- No `egui`/`eframe` types anywhere in `scanner/**` or `rscan/**`.

### D.4 — Filter model (`native/src/filter/core/filter.rs`, `types/core/types.rs`)

- `FilterDef` (`types/core/types.rs:83-99`): `text: String, text_mode: TextMode, extensions:
  Vec<String>, size: Range<u64>, mtime: Range<i64>, btime: Range<i64>, depth: Range<u32>,
  include_files/include_dirs/include_hidden/include_system: bool, problem_names_only: bool`.
- `TextMode::{Substring, Regex, Glob}` (`types.rs:60-65`); `Range<T> { min: Option<T>, max:
  Option<T> }` (`types.rs:68-80`).
- `CompiledFilter::compile(f: &FilterDef) -> Self` (`filter.rs:59-96`): substring mode builds an
  OR-of-AND term matcher (`text_groups`, `filter.rs:21-31`, split on `;`/`,`); regex mode compiles
  `regex::Regex::new(&format!("(?i){}", text))` (`filter.rs:66`); glob mode compiles
  `globset::GlobBuilder::new(text).case_insensitive(true)` (`filter.rs:77`); extensions normalized
  via `extensions::normalize_extensions` (`filter/core/extensions.rs`). `error() -> Option<&str>`
  surfaces a compile failure without panicking.
- Problematic-name criterion delegates to `crate::types::win32_name_issue` (`types.rs` import at
  `filter.rs:2`), implemented in `types/core/win32_names.rs:35+` — pure string logic (reserved
  device names, trailing dot/space, invalid characters), no platform dependency; kept for
  cross-platform-naming compatibility checks even off Windows.
- No `egui`/`eframe` types in `filter/**`.

### D.5 — Copy/move/delete engine

- Local expand+copy: `start_copy_expanded(seeds: Vec<FileEntry>, filter:
  Option<(FilterDef,String)>, opts: CopyOptions, tx: Sender<CopyMsg>) -> CopyHandle`
  (`copy/os/shared/copy.rs:72-149`); also `start_copy_from_paths` (`copy.rs:160`) and
  `start_copy_pairs` (`copy/os/shared/pairs.rs:20`, explicit source/dest pairs with a `Conflict`
  policy from `crate::types::Conflict`). `CopyMsg::{Progress(CopyProgress), Done{progress,
  errors: Vec<(String,String)>}}` (`copy.rs:40-45`); `CopyHandle { cancel: Arc<AtomicBool> }`
  (`copy.rs:48-50`).
- Cross-backend single-file copy (the one that matters for a JNI facade moving data between two
  arbitrary `Backend`s, e.g. SFTP → local): `copy_between<S: Backend+?Sized, D: Backend+?Sized>
  (source_backend: &S, source: &str, destination_backend: &D, destination: &str) -> io::Result<u64>`
  (`vfs/os/shared/copy_transfer.rs:11-46`). Streams through a `tempfile::tempfile()` disk spool
  (line 20), re-`stat`s the source before/after to detect concurrent mutation (lines 14, 26-32),
  stages via `unique_staging_path` + commits via `promote_staged_replace` (lines 34, 40) — both from
  `vfs::promotion`.
- Recursive delete: `remove_entry(backend: &dyn Backend, target: &DeleteTarget) -> VfsResult<()>`
  and the cancellable/progress form `remove_entry_controlled(backend, target, cancel: &AtomicBool,
  progress: impl FnMut(RecursiveDeleteProgress)) -> Result<RecursiveDeleteReport,
  RecursiveDeleteFailure>` (`vfs/core/delete.rs:64-122`). Two-phase (`Planning` then `Applying`,
  `RecursiveDeletePhase`, :19-22), budget-limited (`MAX_DELETE_ENTRIES = 1_000_000`,
  `MAX_DELETE_TEXT_BYTES = 128 MiB`, `MAX_DELETE_DEPTH = 512`, :6-8), re-verifies each item's
  backend-`id`/type immediately before deleting it (`apply_planned_item`, :258-288) to guard against
  a changed-underneath-us race. Entirely backend-agnostic (works through the `Backend` trait, not
  `std::fs` directly) — this is the one to wrap for arbitrary-backend delete.
- Trash/recycle on Linux: the `trash` crate (`Cargo.toml:26`) is **not called anywhere in the
  assigned read surface**. `Backend::delete_disposition()` (`vfs/core/core.rs:212-214`, default
  `Permanent`) is the only trait-level hook; the actual OS-trash call (if any) happens outside this
  audit's scope. `analytics/os/shared/reclaim/verify.rs:16-` only **plans** a `ReclaimTrashPlan`
  (paths to delete), it performs no deletion itself.
- No `egui`/`eframe` types in `copy/**` or `vfs/core/delete.rs`.

### D.6 — Storage analytics

- `scan(root: &Path, p: &Progress) -> ScanOutcome` (`analytics/os/shared/analytics.rs:62`).
  `Progress { files: Arc<AtomicU64>, dirs: Arc<AtomicU64>, bytes: Arc<AtomicU64>,
  cancel: Arc<AtomicBool> }` (`analytics.rs:44-47`), shared/live, updated during the walk.
  `SizeNode { name: Box<str>, size: u64, is_dir: bool, children: Vec<SizeNode> }` (`analytics.rs:35-39`,
  recursive subtree total for dirs). `ScanOutcome`/`ScanStatus`/`ScanIssue`
  (`analytics/os/shared/analytics_outcome.rs:8,65,70`).
- The actual directory read comes from **out-of-scope** `crate::local_access::{read_directory,
  EntryKind, LocalEntry, parallel_scan_allowed, normalize_scan_root}`, re-exported unchanged by
  `analytics/os/mod.rs:1-3` — this module has no `#[cfg]` of its own, so whatever platform handling
  `local_access` does (or doesn't do) for Android is inherited as-is; **flagged unresolved**, not
  read in this audit (out of the assigned surface).
- Backend-driven (remote) size scan: `scan_backend(...)` (`analytics/os/shared/analytics_backend.rs`,
  exported `analytics.rs:26`), not read in full but present.
- Reclaim/duplicates sub-feature: `scan_reclaim(...)` (`reclaim/local.rs:36`),
  `scan_reclaim_backend(...)` (`reclaim/backend.rs:45`), `ReclaimOptions`/`ReclaimProgress`/
  `ReclaimItem`/`DuplicateGroup`/`ReclaimReport` (`reclaim/types.rs:6,27,112,149,168`); no Linux-only
  API found in `reclaim/**` (checked explicitly, see §B).
- No `egui`/`eframe` types in `analytics/**`.

### D.7 — ZIP browse/extract (`zipfs/os/shared/zipfs.rs`)

`ZipBackend::open` (see D.2) implements `Backend` read-only: `list_dir`/`stat`/`open_read` work off
an in-memory `HashMap<String, Vec<VfsMeta>>` directory map built once at open (:120-155);
`open_write`/`rename`/`remove_file`/`remove_dir`/`mkdir_all` all return
`io::ErrorKind::PermissionDenied` ("ZIP ist schreibgeschützt", :200-224); `delete_disposition() ->
Unsupported` (:216-218); `parallelism() -> 1` (:222-224, single archive file). Separate free
function `extract_all(zip_path: &str, dest: &Path) -> io::Result<usize>` (:245-269), zip-slip-safe
via `enclosed_name()` (:258). Pure `std::fs` + the `zip`/`chrono` crates — no platform-specific code,
no `egui`/`eframe`.

### D.8 — `folder_index` (fuzzy folder search / live index) and `rscan`

- `FolderIndex::build_async(roots: Vec<PathBuf>, persist_path: PathBuf, tx: Sender<IndexMsg>,
  cancel: Arc<AtomicBool>) -> io::Result<()>` (`folder_index/os/shared.rs:21-30`), spawns a detached
  `index-builder` thread. `IndexMsg::{Progress{count,current}, Complete(FolderIndex), Canceled,
  Failed(String)}` (`folder_index/core/model.rs:18-22`).
- `FolderIndex { paths: HashSet<String>, path_text_bytes: usize }` (`model.rs:10-15`); limits
  `MAX_INDEX_PATHS = 1_000_000`, `MAX_INDEX_PATH_TEXT_BYTES = 128 MiB`, `MAX_INDEX_DEPTH = 512`
  (`model.rs:4-6`); `try_insert` enforces them (`model.rs:44-64`).
- CPU-only fuzzy scoring: `FolderIndex::search_scored(&self, query: &str, n: usize) ->
  Vec<(String, i32)>` (`folder_index/core/search.rs:9-40`), rayon-parallel, no filesystem access —
  explicitly documented as safe to call on the UI thread even with a cold disk (comment,
  `search.rs:8-9`).
- Persistence format: plain UTF-8 paths, one per line, in `folder_index.txt` under the app-data
  folder (module doc comment, `folder_index/mod.rs:8-9`); the actual write path again goes through
  the out-of-scope `crate::support_dirs::app_data_dir()` pattern seen elsewhere.
- `rscan` reuses `scanner::{RetentionHandle, ScanHandle, ScanMessage}` for remote/backend-driven
  recursive scans (D.3) — the module has no independent message/limit types of its own.
- No `egui`/`eframe` types in `folder_index/**` or `rscan/**`.

### D.9 — `icons`

`icons/core/icons.rs:18` `use eframe::egui;` — **the one and only** `egui`/`eframe` dependency in
the entire assigned read surface. `IconCache` holds `textures: HashMap<String,
egui::TextureHandle>` (:56) and `drain(&mut self, ctx: &egui::Context) -> bool` (:84) uploads
decoded pixels via `egui::ColorImage`/`egui::TextureOptions` (:96-100) — this is GUI-thread-bound
egui-texture-upload code, not reusable across a JNI boundary as-is; a Compose UI would need its own
icon-cache/texture equivalent. The platform-specific icon *extraction* worker
(`icons/os/shared.rs:1-17`, selected for all non-Windows including Android per `icons/mod.rs:3-8`)
is a complete no-op stub today (`request`/`drain` do nothing) — i.e. file-type icon extraction
already does not exist on desktop Linux and would not exist on Android either unless newly
implemented.

### D.10 — `dragout`

Windows-only OS-drag machinery; `DragOutEffect`/`DragOutOutcome` enums (`dragout/mod.rs:1-14`) are
platform-neutral data types, but the entire implementation module (`os/windows.rs`) and its
`pub use` are both `#[cfg(windows)]` (`mod.rs:16-21`) — nothing is exported on any non-Windows
target, today or on Android. No portability *regression* here; simply zero capability to carry over
without writing a new Android drag-and-drop implementation from scratch.

---

## Summary for the JNI facade

The `Backend` trait (D.1) and the five entry-point families (scanner D.3, filter D.4, copy/delete
D.5, analytics D.6, zipfs D.7, folder_index/rscan D.8) are the clean, `egui`-free surface to wrap.
Three concrete compile-blocking gaps must be closed first (§A.1: scanner, folder_index, cloud
`platform` module selection), one raw-syscall path needs an Android-libc verification (§B:
`SYS_renameat2`), and two out-of-scope modules (`crate::creds`, `crate::support_dirs`) gate whether
credential storage and app-data-directory resolution are Android-portable — both are flagged
unresolved rather than assumed.
