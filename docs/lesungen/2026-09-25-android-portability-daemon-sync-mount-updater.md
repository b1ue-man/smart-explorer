# Android portability audit: daemon / sync / mount / updater / credential modules and the crate root

**Purpose.** Read-only fact-finding for a future Android (APK) port. For every conditional-compilation
site and every Linux/glibc-flavored API in the assigned modules, this records what
`target_os = "android"` (a `unix`-family target that is **not** `target_os = "linux"` and **not**
`windows`) resolves to, whether a referenced symbol goes missing, and candidate remedies. No
architecture verdict is given; facts are `file:line`.

## Files read

```
native/src/daemon/**            native/src/mount/**            native/src/creds/**
native/src/syncjobs/**          native/src/local_access/**     native/src/connect/**
native/src/sync/**              native/src/updater/**          native/src/cli/**
native/src/bisync/**            native/src/autostart/**        native/src/support_dirs.rs
                                 native/src/cloud/**            native/src/lib.rs
                                 native/src/gdrive/**           native/src/main.rs
                                                                 native/src/bin/se.rs
native/Cargo.toml   native/build.rs   native/build_support/private_dokany.rs
native/build-agent-bundles.sh   native/agent-bin/ (listing only)
```
Out-of-scope files that turned out to be tightly coupled (see “Unresolved”): `native/src/share/**`,
`native/src/agent/**`, `native/src/agent_proto/**`, `native/src/app/**`.

---

## 1. Module-selection sites: `target_os = "linux"` vs. `windows`, no `unix`/`not(windows)` fallback

These `#[path = "os/..."]` selections cover only Linux and Windows. `target_os = "android"` matches
neither arm, so the module itself goes **missing**, and every unconditional caller fails to resolve
the symbol (`E0433`-class error) the first time the crate is compiled for an Android target.

| Site (mod declaration) | Module | Android resolution | Unconditional use (file:line) | Remedy candidate |
|---|---|---|---|---|
| `native/src/daemon/mod.rs:44` (`target_os="linux"`) / `:47` (`windows`) | `daemon::ipc_storage` | **missing** | `native/src/daemon/os/shared/mount_client.rs:14`, `ipc_client.rs:10`, `exec_ipc.rs:116-118`, `exec_state.rs:251,257`, `handoff.rs:105`, `ipc_listener.rs:9`, `mount_manager.rs:185`, `mount_probe_client.rs:7`, `exec_grant_journal.rs:120`, `exec_grant_journal_storage.rs:12,19,30,45,54` | Add `#[cfg(any(target_os = "linux", target_os = "android"))]` (or a dedicated `os/android/ipc_storage.rs`) pointing at the same file; body is plain POSIX (`libc::openat/mkdirat/geteuid`, no glibc-only calls) so the existing Linux file is a viable candidate as-is |
| `native/src/daemon/mod.rs:78` / `:81` (windows) | `daemon::mount_process` | **missing** | `mount_client.rs:232-237`, `mount_manager.rs:200` | Same widen-cfg approach; Linux stub already just returns `Unsupported` (mounting is out of scope per task), so an Android stub is a one-line copy |
| `native/src/daemon/mod.rs:101` / `:98` (windows) | `daemon::platform` | **missing** | `mount_registry.rs:106,152,164,193`, `job.rs:168`, `backend_transfer.rs:67`, `handoff.rs:116,118,127,256,266`, `schedule.rs:41,100`, `state.rs:109,230`, and the re-export `pub use platform::DriveInfo` at `mod.rs:154` | Same widen-cfg; API surface to implement for Android: `DriveInfo`, `removable_drives()`, `battery_saver_on()`, `on_metered_network()`, `run_shell_command()`, `normalize_local_backend_path()`, `atomic_replace()`, `restore_control_if_absent()`, `metadata_is_link_like()`, `acquire_daemon_instance_guard()` (Windows surface at `native/src/daemon/os/windows/platform.rs:32-124` is the same 9-function contract) |
| `native/src/syncjobs/mod.rs:19` / `:22` (windows) | `syncjobs::platform` | **missing** | `native/src/syncjobs/os/shared/persistence.rs:198-199`, `migration.rs:60,62,65,74` — i.e. every saved-job read/write (`syncjobs::load/upsert/remove`, used by the daemon and by the GUI) | Widen cfg; body (`std::fs::rename`, `libc::syscall(SYS_renameat2,…)`, `File::sync_all`) is plain POSIX+syscall, portable candidate |
| `native/src/updater/mod.rs:16` / `:13` (windows) | `updater::os` (re-exported as `pub use os::revert_to`) | **missing** | `updater::revert_to` is a `pub` crate API (`mod.rs:36`); callers outside the read surface (likely `app`/`cli`) would fail to resolve it | Widen cfg (body only uses `std::fs::hard_link`, `OpenOptionsExt::mode`, `std::process::Command::spawn` — all portable) |
| `native/src/autostart/mod.rs:4` / `:1` (windows) | `autostart::platform` (`pub use platform::*` at `mod.rs:11`) | **missing** | Whole public API (`is_enabled`, `enable`, `disable`, `spawn_daemon_now`, `spawn_daemon_handoff_checked`) is unresolved | Not a "port the Linux file" case — the *behavior* (`.desktop` autostart entry under `~/.config/autostart`) is itself meaningless on Android. Needs a real Android adapter: no XDG autostart, no `Command::new(exe).spawn()` self-relaunch. The Android equivalent is a `BOOT_COMPLETED` `BroadcastReceiver` + `WorkManager`/foreground-service start, implemented on the Kotlin side and exposed to `core` as a typed capability rather than a spawned process |
| `native/src/cloud/mod.rs:7` (linux) / `:10` (windows), nested `mod os { … }` | `cloud::os::open_url` | **missing** | `native/src/cloud/core/cloud.rs:278` — unconditional call inside the OAuth "open the consent URL in the browser" flow | Linux body (`native/src/cloud/os/linux_os.rs:1-3`) shells out to `xdg-open`, which does not exist on Android and would not work even if the module compiled. Real remedy: JNI callback into Kotlin (`Intent.ACTION_VIEW` / Custom Tabs), not a spawned process |

All six sites share one root cause: the module-selection idiom used throughout this codebase is
`#[cfg(target_os = "linux")]` + `#[cfg(windows)]`, written before any third target existed. `unix`/
`not(windows)` is a different, already-used idiom elsewhere (see §2) and does cover Android; these six
do not.

## 2. Module-selection sites using `unix` / `not(windows)` — these DO cover Android

| Site | Module | Android resolution | Notes |
|---|---|---|---|
| `native/src/creds/mod.rs:5` (`not(windows)`) | `creds::secure_store` → `os/linux_os.rs` | **compiles**, Linux file-store code runs as-is | See §7; body is portable POSIX (`libc::openat/flock`, `sha2`, `getrandom` — no glibc-only calls) |
| `native/src/local_access/mod.rs:7` (`not(windows)`) | `local_access::platform` → `os/linux_os.rs` | **compiles** | `native/src/local_access/os/linux_os.rs` is pure `std::fs`/`std::os::unix` — portable. Behavioral gap: `request_access()` (line 58-60) returns a hard-coded German "extra read rights must be granted at the filesystem, on Linux" error — wrong messaging/behavior for Android, which has no such concept and instead needs a real Storage-Access-Framework (SAF) permission round-trip through the Kotlin host |
| `native/src/cli/os/mod.rs:1` (`not(windows)`) | `cli::os::platform` → `os/linux_os.rs` | **compiles** | `native/src/cli/os/linux_os.rs` uses `MetadataExt::dev()/ino()` (line 2,15) — POSIX, works on bionic |
| `native/src/mount/os/mod.rs:1` (`cfg(windows)` only, no fallback module at all) | `mount::os::windows` | **missing, and correctly so** | Every call site in `native/src/mount/mod.rs:264-306` (`drive_runtime_info`, `install_drive_runtime`, `run_host_if_requested`) is itself split `#[cfg(windows)] / #[cfg(not(windows))]` inline, and `drive_mount_supported()` (`mount/mod.rs:259-261`) is `cfg!(windows)`. Mounting already degrades cleanly to `Err("... supported only on Windows")` on any non-Windows target, Android included — consistent with the task's "mounting nicht notwendig" |
| `native/src/bisync/mod.rs:86,89` (`test`-only, `windows`/`unix`) | `link_fixture` (tests only) | n/a | Production bisync code (`os/shared/*`) has **no** platform split at all — confirmed portable (§ below) |

## 3. Linux-only / glibc-adjacent APIs actually used, and their Android fate

| Site | API | Android resolution | Consequence | Remedy candidate |
|---|---|---|---|---|
| `native/src/daemon/os/linux_os/platform.rs:59-66` | `libc::syscall(libc::SYS_renameat2, …, libc::RENAME_NOREPLACE)` | Linux kernel syscall number, shared with Android (same kernel ABI); `libc` crate's android arch tables generally mirror the linux_like ones | **likely compiles**, but **not verified in this offline environment** (no vendored `libc` crate source available to confirm `SYS_renameat2`/`RENAME_NOREPLACE` are exported for `target_os="android"`); code already has a graceful fallback to `std::fs::hard_link` + `remove_file` on `ENOSYS`/`EINVAL`/`EOPNOTSUPP` (lines 75-88), so even a missing/older-kernel syscall degrades safely *if it compiles* | Confirm on first Android CI build; if the constant is absent, gate with `target_os` and fall back straight to the hard-link path |
| `native/src/syncjobs/os/linux_os.rs:17-29` | Same `SYS_renameat2` pattern, comment explicitly notes "the libc crate omits the renameat2 wrapper on musl" | Same as above | Same as above — **and** this path has *no* fallback on non-zero result other than returning the raw `io::Error` (line 33), unlike the daemon copy | If the syscall constant is missing at compile time, this needs the same ENOSYS-fallback treatment as `daemon/os/linux_os/platform.rs` before Android use |
| `native/src/daemon/os/linux_os/platform.rs:115-132` | `XDG_RUNTIME_DIR` env var, else `/run/user/{uid}`, else `/tmp/smart-explorer-runtime-{uid}` | `XDG_RUNTIME_DIR` is unset on Android; `/run/user/…` does not exist; falls through to the `/tmp/...` `DirBuilder` branch | **compiles, wrong path** — `std::env::temp_dir()`-equivalent hand-rolled `/tmp/...` is not app-private storage on Android (no writable global `/tmp`) | Feed an app-private runtime dir (Kotlin `context.getCacheDir()` / `context.getNoBackupFilesDir()`) into `daemon::platform` via a typed override instead of deriving it from XDG env vars (mirrors the `support_dirs.rs` gap, §9) |
| `native/src/autostart/os/linux_os.rs:6-16` | `XDG_CONFIG_HOME`/`HOME` + `.desktop` autostart file | No desktop autostart concept on Android at all | **wrong behaviour by design**, not a compile issue (module is already missing per §1) | Real Android autostart is `BOOT_COMPLETED` receiver + `WorkManager`/foreground service, not a file drop |
| `native/src/autostart/os/linux_os.rs:118-126` | `libc::setsid()` via `CommandExt::pre_exec` | `setsid(2)` exists on bionic | Compiles, but the whole self-relaunch-as-daemon design (`Command::new(exe).spawn()`, line 98-127) does not map to Android process model (no long-lived detached child process outside the app's own process tree without a foreground service) | Background execution must become an Android foreground `Service` invoked in-process via JNI, not a forked child executable |
| `native/src/cloud/os/linux_os.rs:1-3` | `std::process::Command::new("xdg-open")` | `xdg-open` binary does not exist on Android | **missing external binary at runtime** (module itself already missing at compile time, §1) | JNI → Kotlin `Intent.ACTION_VIEW` / Custom Tabs, see §1 |
| `native/src/daemon/os/shared/job.rs:167-168` calling `platform::run_shell_command` (`native/src/daemon/os/linux_os/platform.rs:35-37`) | `std::process::Command::new("sh").args(["-c", cmd])` | `/system/bin/sh` exists on stock Android and is on `PATH` for an app process | Compiles once the module is widened (§1); *behaviorally* the user-configurable "run a shell command before/after sync" job feature is only as useful as whatever `sh` can do inside the app's own sandboxed UID (no system-wide effects, no root) | Keep the feature but document/limit scope in the Android UI; no code change required for compilation, only for user expectations |
| `native/src/daemon/os/linux_os/ipc_storage.rs`, `native/src/creds/os/linux_file_store.rs` | `libc::openat/mkdirat/flock/geteuid`, `std::os::unix::fs::{MetadataExt,PermissionsExt,DirBuilderExt,OpenOptionsExt}` | All standard POSIX, present on bionic | **compiles and behaves correctly** once reachable (creds already is reachable, §7; daemon's copy needs §1's cfg widening) | none needed beyond §1 |
| *(not found)* `/proc`, `/sys` (outside `share/`, out of scope), `memfd_create`, `pidfd_*`, `copy_file_range`, `O_TMPFILE`, `prctl`, `landlock`, `NetworkManager`, `polkit` | — | — | **none of these appear anywhere in the assigned read surface** | n/a |
| *(not found in assigned surface)* `zbus`/systemd/D-Bus | — | — | Zero hits inside daemon/syncjobs/sync/bisync/mount/local_access/updater/autostart/cloud/gdrive/creds/connect/cli. All `zbus`/`org.freedesktop.systemd1`/cgroup usage lives in `native/src/share/os/linux_os/exec*.rs` (out of scope, see §10) | n/a for this surface |

## 4. Dependencies used in the assigned modules and their `Cargo.toml` target gates

| Crate | Cargo.toml gate (`native/Cargo.toml:line`) | Used from (assigned modules) | Android pull-in? | Note |
|---|---|---|---|---|
| `libc` | `[target.'cfg(not(windows))'.dependencies]` (:164-165) | `daemon/os/linux_os/*`, `syncjobs/os/linux_os.rs`, `creds/os/linux_file_store.rs`, `cli/os/linux_os.rs` | **yes** (`not(windows)` matches android) | Needed and correct; only open question is the `SYS_renameat2`/`RENAME_NOREPLACE` symbol availability, §3 |
| `rfd` | `[target.'cfg(windows)'.dependencies]` uses `common-controls-v6` (:112); `[target.'cfg(not(windows))'.dependencies]` uses `xdg-portal, tokio` (:166) | not referenced anywhere in the assigned surface (no `rfd::` hit in daemon/syncjobs/sync/bisync/mount/local_access/updater/autostart/cloud/gdrive/creds/connect/cli) | **would be pulled in** for Android via the `not(windows)` arm even though nothing in this surface calls it (folder-picker use is presumably in `app/`, out of scope) | `xdg-portal` implies an XDG Desktop Portal / D-Bus backend that does not exist on Android; if `app/` calls `rfd` unconditionally, the dependency needs a `target_os = "android"` carve-out (native file picker via `Intent.ACTION_OPEN_DOCUMENT_TREE`/SAF from Kotlin instead) |
| `zbus` | `[target.'cfg(target_os = "linux")'.dependencies]` (:168-171) | none in this surface; lives in `native/src/share/os/linux_os/exec*.rs` (out of scope) | **correctly excluded** (`target_os="linux"` does not match `"android"`) | Good gate — this is the one dependency in the file already written the "right" way for a third target |
| `keyring` | `[target.'cfg(windows)'.dependencies]` (:111) | `native/src/creds/os/windows.rs:5` only | **correctly excluded** on Android | Confirms creds already avoids any Secret-Service/D-Bus dependency on non-Windows (§7) |
| `trash` | plain `[dependencies]` (:26), **no target gate at all** | `native/src/bisync/os/shared/apply_delete.rs:112` (`trash::delete(path)`), reached whenever a sync delete uses "recycle" on a local backend, unconditional on every target | **pulled in and called on Android** | **Needs verification** (no vendored crate source available offline): does `trash` 5.x provide a `target_os = "android"` backend at all? If not, this is a hard compile failure for the whole crate, not just a stub. Remedy candidates: confirm crate support; if absent, gate the recycle path behind `#[cfg(not(target_os = "android"))]` with an Android fallback (move into an app-private "trash" folder, or hard delete with a stronger in-app undo window) |
| `tempfile` | plain `[dependencies]` (:46) | `native/src/gdrive/os/shared/copy_writer.rs:30` (`tempfile::tempfile()`), and pervasively elsewhere in the crate | **compiles**, but depends on a writable temp directory | Same root issue as `support_dirs.rs` (§9): `tempfile`/`std::env::temp_dir()` resolve to `/data/local/tmp` on stock Android (Rust std hard-codes this per-target default when `$TMPDIR` is unset), which is not writable by an ordinary app process. An embedding host must set `TMPDIR` (or `tempfile`'s override) to the app's cache dir before first use |
| `getrandom` 0.2 | plain `[dependencies]` (:76) | `daemon/os/linux_os/ipc_storage.rs`, `creds/os/linux_file_store.rs` | **compiles** | `getrandom` 0.2 has first-class Android support (bionic `getrandom(2)`/`/dev/urandom` fallback) — no action needed |
| `eframe`, `egui_extras` | plain `[dependencies]` (:9-10) | **zero hits** anywhere in the 13 assigned modules (only `lib.rs:46-110`, `run_gui()`, out of the per-module surface but present in the crate root read) | n/a to these modules | Confirms daemon/sync/mount/etc. are UI-framework-agnostic already; the planned JNI boundary can call into them without dragging `eframe`/`winit` along, *provided* `run_gui()`/`eframe::run_native` itself is never invoked on Android (it is currently the crate's only GUI entry point, `lib.rs:46`) |
| `notify` | plain `[dependencies]` (:18) | **zero hits** in the assigned surface | n/a | Real-time change watching (`daemon/mod.rs` doc comment line 5 mentions "real-time change" trigger) is presumably wired through a module outside this read surface; unresolved, see §10 |
| `ctrlc` | plain `[dependencies]` (:13) | `native/src/cli/exec.rs` only (grep hit) | **compiles**, POSIX signal handling works on bionic | Only exercised by the desktop CLI (`se`) Ctrl-C handling path; not obviously relevant to an Android JNI host, but not harmful either |
| `iroh`, `hickory-resolver`, `tungstenite`, `mdns-sd`, `if-addrs`, `russh`, `russh-sftp`, `tokio`, `rusqlite` (bundled) | plain `[dependencies]` | `daemon` (iroh, via `ipc_host_direct_event*.rs`), `bisync` (`rusqlite`, via `state_store.rs`/`incremental.rs`), `cli/share/*` (iroh) | **compile on Android** in principle (all are cross-platform pure-Rust or NDK-portable native crates; `rusqlite` `bundled` compiles SQLite from source, which is a normal Android NDK target) | Not verified end-to-end (would need an actual cross-compile, explicitly out of scope for this read-only audit); flagged as the largest "should work but unverified" dependency cluster |

## 5. `build.rs` / agent bundles for `CARGO_CFG_TARGET_OS=android`

- `native/build.rs:14-16`: the entire script (Windows `.rc`/`.manifest` resource compilation, `windres`
  invocation, and the call into `private_dokany::generate()`) is behind one early return:
  `if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") { return; }`. For `android`, `build.rs`
  does **nothing** — no Dokany DLL byte/provenance check, no resource compiler invocation, no
  `OUT_DIR` artifact. Building the library for Android therefore requires **no prebuilt files** from
  `native/build_support/**` or the Dokany-private stage.
- `native/build_support/private_dokany.rs:1-2` documents itself as "build-time byte/provenance checks
  only… Dokany compilation belongs to the remote preparation stage" — confirms it is Windows-only
  tooling, already inert for any other `CARGO_CFG_TARGET_OS`.
- `native/agent-bin/` (directory listing only, per the assigned surface) currently holds exactly two
  prebuilt static binaries: `se-agent-x86_64-linux-musl` (698,888 bytes) and
  `se-agent-aarch64-linux-musl` (620,952 bytes), both owned by `root`. `native/build-agent-bundles.sh`
  (108 lines) is what (re)builds them: it cross-compiles `se-agent` for
  `x86_64-unknown-linux-musl`/`aarch64-unknown-linux-musl` only (lines 19-20, 92-93) and installs the
  results into `native/agent-bin/` (lines 96-103) — **no `*-linux-android` target is built by this
  script today**.
- Neither `native/build.rs` nor any file in this read surface references `agent-bin`, `agent_bin`, or
  `include_bytes!` of an agent binary. `grep` for those needles across `native/src/*.rs` and the 13
  assigned module trees returns nothing; the only consumers are `native/src/agent/**`,
  `native/src/agent_proto/**`, and `native/src/sftp/core/io_adapters.rs` — all **outside** the assigned
  read surface (see §10). So: within the audited surface, an Android build of the daemon/sync/mount/
  updater/creds/cli/crate-root code needs **no** `agent-bin` artifacts; whether the (out-of-scope)
  SSH-agent-forwarding feature needs new Android-target `se-agent` binaries is a separate, unresolved
  question.

## 6. `lib.rs` module list and cross-module dependency references (crate root)

`native/src/lib.rs:1-44` declares the full module list. Relevant to this audit:

- Three modules are Windows-only at the crate root: `shell_clipboard` (`:29-30`), `shell_menu`
  (`:31-32`), `shell_register` (`:33-34`), `virtual_clipboard` (`:41-42`) — all `#[cfg(windows)]`, so
  correctly absent (not missing-but-referenced) on Android; nothing in the assigned surface calls them
  unconditionally.
- `local_access` (`:21`) is a private (`mod`, not `pub mod`) crate-root module — not part of the public
  API, used only internally (by `run_gui()` at `lib.rs:50` and by the mount/local_access boundary).
- `run_gui()` (`lib.rs:46-111`) is the **only** GUI entry point in the crate and calls
  `eframe::run_native` (`:102-110`) unconditionally — this is desktop-only (winit-backed); an Android
  JNI host must not call `run_gui()` and would instead call into `app::App` (out of scope) or the
  individual feature modules directly. `eframe`/`egui_extras` have zero references inside the 13
  assigned modules (§4), so this is structurally already separable.
- `main.rs:5,8` (`native/src/main.rs`) and `bin/se.rs:6,9,12,19-31` (the `se` CLI binary) are both
  desktop process entry points (`--sync-daemon`, `--mount-host`, `--uninstall-cli-path`, etc.) built
  around `std::process::exit`/`std::env::args_os` — neither is a candidate for direct JNI reuse; the
  Kotlin host would call the underlying library functions (`daemon::run_daemon`, `mount::run_host_if_requested`,
  …) directly rather than spawning `se`/`smart_explorer` as a subprocess.

## 7. `creds/` secret storage on Linux, and Android app-private-directory fit

- Store selection: `native/src/creds/mod.rs:5-10` — `#[cfg(not(windows))]` picks
  `os/linux_os.rs` (covers Android, §2).
- Directory: `native/src/creds/os/linux_os.rs:12-14` —
  `crate::support_dirs::app_data_dir().join("secrets-v1")` (one directory for all secrets, mode `0700`
  enforced by `native/src/creds/os/linux_file_store.rs:108-131,180-190`).
- Format: hand-rolled binary record format per secret, one file per `account` — magic `b"SESEC01\0"`
  (`linux_file_store.rs:12`), 1-byte format version, a 32-byte SHA-256 account digest, a 4-byte length
  field, the secret bytes (capped at 64 KiB, `MAX_SECRET_BYTES` `:18`), and a 32-byte checksum
  (`HEADER_BYTES`/`MIN_RECORD_BYTES`/`MAX_RECORD_BYTES`, `:16-19`). No OS keychain/Secret-Service/D-Bus
  dependency by design (doc comment `linux_os.rs:3-5`: "no Secret Service, D-Bus, desktop-session, or
  kernel-keyring dependency… filesystem ownership and mode bits are the security boundary").
- Permissions model: every directory/file is created and *re-validated* at `0700`/`0600`,
  single-hardlink, owned by the calling `euid` (`linux_file_store.rs:147-160` `validate_directory`,
  `224-236` `validate_secure_file`) before every read/write, using `O_NOFOLLOW` throughout
  (`:126,154,175,202,264` etc.) to reject symlink substitution.
- Fit for an Android app-private directory: **structurally yes** — the store's only real precondition
  is "a directory the calling UID exclusively owns, on a filesystem that supports POSIX owner/mode
  bits and hardlinks with `renameat`/`flock`". Android's per-app private storage
  (`context.getFilesDir()`/`context.getNoBackupFilesDir()`, on internal storage, which is `ext4`/`f2fs`
  and per-UID sandboxed) satisfies that. The **only** blocking issue is upstream: `app_data_dir()`
  itself does not resolve to anything writable on stock Android today (§9) — once that is fixed (by
  pointing `support_dirs` at the JNI-provided files dir), `creds/os/linux_os.rs`/`linux_file_store.rs`
  need **no changes** to work correctly on Android. One caveat: mode/owner checks assume a real
  multi-user POSIX filesystem; most Android internal storage (`/data/data/<pkg>/...` on `ext4`) honors
  Unix permission bits normally, so this should hold, but was not independently verified against a real
  device/emulator in this read-only audit.

## 8. `support_dirs.rs` — directory derivation and override surface

`native/src/support_dirs.rs:3-26` (`data_home()`):

| `cfg` branch (line) | Resolution | Android? |
|---|---|---|
| `#[cfg(windows)]` (:4-9) | `%APPDATA%` env var, else `std::env::temp_dir()` | no |
| `#[cfg(target_os = "linux")]` (:11-20) | `$XDG_DATA_HOME`, else `$HOME/.local/share`, else `std::env::temp_dir()` | no (android ≠ linux) |
| `#[cfg(not(any(windows, target_os = "linux")))]` (:22-25) | **unconditionally `std::env::temp_dir()`** | **yes — this is the branch Android falls into** |

- `app_data_dir()` (:28-32) = `data_home().join("smart_explorer")`, then
  `std::fs::create_dir_all(&dir)` — errors are silently discarded (`let _ =`). `app_data_file()` (:34-36)
  and `sync_data_dir()` (:38-42, used by `daemon/os/linux_os/ipc_storage.rs` and everything under it,
  §1/§7) both derive from `app_data_dir()`.
- **Override surface today: none dedicated.** There is no env var, config file, or function parameter
  that lets an embedding host redirect these paths; the only lever is the same one every branch already
  reads indirectly: Rust's `std::env::temp_dir()` checks `$TMPDIR` first
  (`unwrap_or_else` fallback only fires when `$TMPDIR` is unset/empty), and `target_os="android"`'s
  fallback-of-the-fallback is the compiled-in constant `/data/local/tmp` (Rust std's own Android special
  case), which is **not writable by an ordinary (non-shell, non-root) app process**.
- Consequence: as written, `app_data_dir()`/`sync_data_dir()`/`creds` all resolve to a directory that
  either (a) is `/data/local/tmp` and `create_dir_all` fails silently, leaving every dependent store
  broken, or (b) is whatever `$TMPDIR` a JNI host happens to set — repurposing a *temp*-directory
  override to carry *persistent* application data (sync job configs, credentials, IPC token/journal),
  which is semantically wrong (Android can reclaim/clear cache-like paths at OS discretion) even where
  it happens to work.
- Remedy candidate: add a fourth branch, `#[cfg(target_os = "android")]`, that reads from a small
  `once_cell`/`OnceLock<PathBuf>` the JNI host populates once at process/library-load time (e.g. a
  `Java_..._setAppDataDir` JNI export called with `context.getFilesDir().getPath()` before any other
  Rust entry point runs), rather than any environment variable — consistent with this repository's own
  `core`/`os` boundary rule ("Pass OS facts into `core` as typed values… instead of letting `core`
  discover the OS itself", `AGENTS.md` "native Rust architecture"). `support_dirs.rs` is currently
  crate-root code, not under `core/`, so this typed-injection pattern would need to be introduced here
  specifically for Android, alongside (or instead of) a literal env-var read.

## 9. Cross-cutting: what already generalizes cleanly vs. what needs new Android code

**Already portable as written** (compiles today under a hypothetical Android target, modulo the
`support_dirs`/`tempfile` directory issue in §8-9 and the `trash`-crate question in §4):
`connect/**` (zero platform `cfg`s at all), `sync/**`, `bisync/**` (all `os/shared`, only test fixtures
split by `unix`/`windows`), `gdrive/**` (HTTP/JSON only), `cli/os/linux_os.rs`, `local_access/os/linux_os.rs`,
`creds/os/linux_os.rs` + `linux_file_store.rs`, `mount/mod.rs`'s already-correct `cfg(not(windows))`
no-op fallback.

**Needs a genuine new Android adapter, not just a cfg-widening** (existing Linux behavior is
conceptually wrong for a mobile app sandbox, not just uncompiled): `autostart` (§1/§3 — no `.desktop`/
XDG autostart on Android; needs `BOOT_COMPLETED` + foreground service), `cloud::open_url` (§1/§3 — no
`xdg-open`; needs a JNI→Intent callback), the daemon's self-relaunch-as-detached-process pattern
(`autostart/os/linux_os.rs:130-138`, used for the "background worker" the task asks about), and
`support_dirs.rs` (§8 — needs typed host-provided paths instead of env-var/HOME sniffing).

**Needs only a cfg-widening** (Linux code is otherwise directly reusable): `daemon::ipc_storage`,
`daemon::mount_process`, `daemon::platform` (minus `run_shell_command`'s reduced usefulness, §3),
`syncjobs::platform`, `updater::os`.

## Unresolved / out-of-scope dependencies and open questions

- `native/src/share/**` (peer-to-peer Share, remote-exec) is **not** in the assigned read surface but
  is extensively imported from `daemon/os/shared/*.rs` and `cli/share/*.rs` (`grep -rl
  "crate::share::"` returns ~50 files inside the assigned surface). Its Linux remote-exec backend uses
  `zbus`, systemd's Manager D-Bus API, and cgroup v2 paths (`native/src/share/os/linux_os/exec*.rs`,
  confirmed only by grep, not read in depth per scope) — this has no Windows-style fallback documented
  in the assigned surface and its Android story is completely open.
- `native/src/agent/**` / `native/src/agent_proto/**` (SSH-agent forwarding, embeds
  `native/agent-bin/se-agent-*-linux-musl` via `include_bytes!`) is out of scope; whether SFTP/SSH-agent
  forwarding needs new `*-linux-android` agent binaries (today's `build-agent-bundles.sh` only builds
  `x86_64`/`aarch64`-`linux-musl`) is unresolved.
- `native/src/app/**` (GUI glue, likely the actual caller of `rfd` for folder pickers, `notify` for
  real-time change watching, and `eframe`) was not read; whether `rfd`/`notify` are called
  unconditionally there (which would make the Android `not(windows)` → `rfd[xdg-portal]` pull-in in §4
  an actual build blocker rather than a latent one) is unresolved.
- The `libc::SYS_renameat2`/`libc::RENAME_NOREPLACE` availability question (§3) could not be verified
  offline (no vendored crate registry present in this environment) and needs a real Android-target
  compile to settle.
- The `trash` crate's Android platform support (§4) likewise could not be verified offline.
- Whether Android's internal per-app storage genuinely preserves POSIX owner/mode-bit semantics the way
  `creds/os/linux_file_store.rs` assumes (§7) was not verified against a real device/emulator.
