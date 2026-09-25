# Android background-daemon hosting model — Smart Explorer sync/Share daemon

**Purpose.** Describe how the desktop background daemon (`--sync-daemon`) works today — process model, IPC, scheduling, persistence, what it hosts, stop/cancel, OS-adapter dependencies — as input for planning whether/how to host the same loop inside an Android app process. Read-only research; no verdict on the overall Android architecture is given.

**Files read** (globs):
- `native/src/daemon/mod.rs`
- `native/src/daemon/os/shared/{schedule,job_supervisor,job,state,handoff,ipc,ipc_listener,ipc_protocol,ipc_client,ipc_host,ipc_host_service,mount_manager,mount_client,lan_runtime,lan_uplink_runtime,exec_state,exec_ipc,locks}.rs`
- `native/src/daemon/os/linux_os/{platform,ipc_storage}.rs`
- `native/src/syncjobs/mod.rs`, `native/src/syncjobs/core/{types,schedule}.rs`, `native/src/syncjobs/os/shared/{persistence,results}.rs`, `native/src/syncjobs/os/linux_os.rs`
- `native/src/sync/mod.rs` (re-export surface only — no daemon coupling found)
- `native/src/bisync/mod.rs`, `native/src/bisync/os/shared/orchestration.rs` (entry point `run()`), `native/src/bisync/os/shared/{persistence,state_store}.rs` (state paths only)
- `native/src/share/mod.rs` (module list / doc comment only)
- `native/src/autostart/mod.rs`, `native/src/autostart/os/linux_os.rs`
- `native/src/app/core/settings_background.rs`, `native/src/app/core/menus_sync_jobs.rs`, `native/src/app/core/job_editor_ui.rs`
- `native/src/cli/**` (grepped for `daemon`)

Not read (out of assigned surface, flagged where load-bearing): `native/src/support_dirs.rs` (resolves every data-dir path used below), `native/src/main.rs` (owns `--sync-daemon` CLI parsing and the `run_daemon()` call site), `native/src/connect/*` (endpoint/credential resolution used by `job.rs`), `native/Cargo.toml` (rusqlite feature flags), `native/src/mount/*`, `native/src/net/*`, `native/src/share/**` internals beyond the daemon's call sites.

---

## 1. Process model

- The daemon is **not** a separate binary — it's the same executable (`smart_explorer` or `se`) re-invoked with the flag `--sync-daemon` (doc comment `native/src/daemon/mod.rs:1-7`; spawn code `native/src/autostart/os/linux_os.rs:74,100`). No CLI file under `native/src/cli/**` parses `--sync-daemon` or calls `run_daemon()` (`rg` found nothing there) — the argument parsing and call site live in `native/src/main.rs`, **outside this assignment's read surface**; unresolved for the main agent.
- Public entry point: `pub fn run_daemon()` (`native/src/daemon/os/shared/schedule.rs:134`, re-exported `native/src/daemon/mod.rs:155`). It takes **no parameters** — everything is read from environment variables and global data-dir helpers (see §5).
- Startup sequence (`schedule.rs:134-229`):
  1. Read/clear env vars `SMART_EXPLORER_DAEMON_HANDOFF` / `SMART_EXPLORER_DAEMON_RETIRING_GENERATION` (`autostart/mod.rs:8-9`) to detect a version-upgrade handoff vs. a fresh launch, and derive a random 32-hex-char **generation id** (`new_generation()`, `getrandom`).
  2. If this is a handoff, wait (`wait_for_handoff_activation`, up to 2 s) for the retiring instance to publish the correct `daemon.stop = "handoff:<generation>"` marker (`handoff.rs:32-63`).
  3. Acquire a **single-instance guard**: an `flock(LOCK_EX|LOCK_NB)` on a lock file (Linux: `daemon_lock_directory()` → `$XDG_RUNTIME_DIR`, else `/run/user/<uid>`, else `/tmp/smart-explorer-runtime-<uid>`; file `smart-explorer-sync-daemon.lock`, mode 0600, dir mode 0700 — `daemon/os/linux_os/platform.rs:95-178`). Non-handoff launches try once with zero timeout; handoff launches retry for up to 300 s (`DAEMON_HANDOFF_TIMEOUT`, `schedule.rs:16-17,111-136`).
  4. Claim/consume or restore the `daemon.stop` control file depending on handoff vs. plain start (`claim_handoff_after_singleton` / `discard_stop_after_singleton`, `handoff.rs:67-88`).
  5. Write heartbeat, construct `ShareHost::new(generation)`, start the IPC listener (§2), do one synchronous `share_host.reload_now()`, then `mark_initialized()`.
  6. Enter the tick loop (§3).
- **Self-replacing update handoff**: a client (GUI/CLI) that finds the running worker stale calls `ipc_client::restart_worker_for_client()` → `launch_replacement()` (`ipc_client.rs:263-277`), which spawns a *new* `--sync-daemon` process with `SMART_EXPLORER_DAEMON_HANDOFF=<new-gen>` / `..._RETIRING_GENERATION=<old-gen>` env vars (`autostart::spawn_daemon_handoff_checked`, `linux_os.rs:158-167`) and then writes the `handoff:<gen>` stop control so the old instance exits once the new one has the singleton. This entire mechanism assumes the OS lets one app process `fork+exec` a second independent OS process of itself — see §8.

## 2. IPC transport & protocol

- **Transport: loopback TCP**, not a Unix domain socket or named pipe. `TcpListener::bind("127.0.0.1:0")` picks an OS-assigned ephemeral port (`daemon/os/shared/ipc_listener.rs:24`). Only loopback peers are accepted (`if !peer.ip().is_loopback() { continue; }`, `ipc_listener.rs:48-50`).
- **Discovery**: the listener publishes its address as plain text to `<app_data_dir>/sync/daemon.ipc` and its generation id to `<app_data_dir>/sync/daemon.generation` (`daemon/os/linux_os/ipc_storage.rs:8-9,18-46`; write call `ipc_listener.rs:27-31`). Clients read those two files plus a shared-secret token to connect (`ipc_client.rs:315-317`, `read_ipc_addr`/`read_token`).
- **Auth**: a 32+ hex-char random token stored at `<app_data_dir>/sync/daemon.token`, file mode 0600, directory mode 0700, created with `O_CREAT|O_EXCL`, re-validated (owner, single-link, mode) on every read (`ipc_storage.rs:13,114-260`). Requests carry the token in-band; comparison is constant-time (`ipc.rs:243-263`). A small subset of requests (`MountHostAttach/Backend/Status`) instead use per-mount launch/session/backend tokens issued out-of-band (`ipc.rs:182-199`).
- **Wire format**: newline-delimited JSON via `serde_json`, tagged enum (`#[serde(tag = "t")]`) — `IpcRequest`/`IpcResponse` in `ipc_protocol.rs:219-388`. Max line size enforced (`MAX_IPC_LINE`); a 5 s pre-auth read deadline and a 16-connection pre-auth limiter guard the listener before the token check (`ipc_listener.rs:16-17,163-207`).
- **Message catalog** (`ipc_protocol.rs:222-387`, dispatch `ipc.rs:33-179`) — no request in this enum ever mentions a *sync job*; job scheduling/listing bypasses IPC entirely (see §4/§9):

| Request | Purpose | Response / notes |
|---|---|---|
| `Ping` | liveness + version + generation + `initialized` | `Pong` — used by every client-side readiness probe |
| `RefreshShare` | clear the "suspended" barrier, reload Share now | `RefreshOk{running}` |
| `ShareCommand{cmd}` | forward a `ShareCmd` (except durable-mutation commands, which are rejected — `ipc_host.rs:315-324`) | `Ok`/`Err` |
| `MutateExecGrant` | enable/disable an exec grant | `ExecGrantMutation` |
| `DrainShareEvents` | pull queued Share UI events + profile snapshot | `ShareEvents{snapshot}` (size-bounded, `bound_snapshot_for_ipc`) |
| `OpenShare{target}` | open a peer backend; **stream handoff**: response then raw backend protocol over the same socket (`serve_backend`) | `OpenOk` then byte stream |
| `ProbeShareMount{target,root}` | capability probe for mounting a peer path | `MountPathCapabilities` |
| `ExecShare{target,req}` | run one remote command, at-most-once | `ExecResult` |
| `ExecStream{target,start}` | long-lived streamed remote exec session | `ExecReady` then framed I/O (`exec_ipc.rs`) |
| `ExecJobs` / `CancelExec` | list / cancel exec sessions | `ExecJobs{snapshot}` / `ExecCancelled` |
| `StartMount/StopMount/ListMounts/RetryMount` | mount lifecycle (drive letter / FUSE-style mount) | `Mount`/`Mounts` |
| `MountHostAttach/MountHostBackend/MountHostStatus` | separate **mount-host process** attach/backend-handoff/status protocol, token-authenticated per mount id | `MountHostReady`/`Ok`/`MountHostStop` |

- **Client side** (`ipc_client.rs`) implements: `ensure_worker_ready()` (probe → wait-for-starting/retiring → `restart_worker_for_client()`), `open_share_backend`, `exec_share`, `refresh_share_worker_checked`, `send_share_command`, `drain_share_worker_events`, `request_daemon_replacement`. All are per-call TCP connects with short timeouts (1–8 s, or up to `req.timeout_ms` for exec) — no persistent client connection is kept open except for the long-lived `OpenShare`/`ExecStream`/mount-host streams.

## 3. Scheduling

- Tick length: `cadence_secs()`, default 15 s (`DEFAULT_TICK_SECS`, `state.rs:7`), user-editable via `<sync_data_dir>/cadence.txt`, clamped 2–3600 s (`state.rs:32-52`, GUI control `settings_background.rs:90-104`).
- Each tick (`schedule.rs:229-374`): re-read `scheduling_controls()` (cadence + pause), check stop, react to the "background sync enabled" toggle flipping (cancels all jobs when turned off, `schedule.rs:235-252`), call `share_host.tick()` (mounts/LAN/Share reload, §6), then — only if `sync_enabled && permit_mutation` — evaluate jobs:
  1. **Timer jobs** (`Interval`, `Calendar`): `job.due(now)` (`syncjobs/core/schedule.rs:36-63`) is evaluated **once per tick**, no separate timer thread.
  2. **RealTime jobs**: **not** an OS filesystem watch (no `notify` crate anywhere in this surface). It's a poll-based "tree signature" — `(file_count, newest_mtime_ms, total_bytes)` computed by walking the local root every tick (`tree_sig()`, `schedule.rs:19-61`, budget-capped at 1,000,000 entries), compared to the previous tick's signature; a changed signature (re)starts a per-job debounce timer (`rt_debounce_secs`, default 10 s) and the job runs once the signature has been stable for that long (`schedule.rs:290-321`). For a remote endpoint with `delete_policy == Mirror`, a lightweight `remote_change_token()` (`backend.current_change_cursor()`, `schedule.rs:79-96`) is folded into the same signature string instead of a filesystem walk.
  3. **OnConnect jobs**: diff of `current_drives()` (`platform::removable_drives()`) against the previous tick's set (`schedule.rs:99-104,323-337`). **On Linux this trigger never fires**: `removable_drives()` is a stub returning `Vec::new()` (`daemon/os/linux_os/platform.rs:23-25}`).
  4. **OnStartup jobs**: enqueued once right after daemon start, and again every time background sync is toggled back on (`enqueue_startup_jobs`, `schedule.rs:220-222,241-244,387-397`).
- Sleep between ticks happens in 2 s slices so `stop_requested`/cadence/pause changes are honoured promptly, while still polling `poll_jobs()` and `share_host.tick()` inside the sleep (`schedule.rs:340-373`).
- **Job kinds**: there is no separate "Drive changes" job kind — `SyncJob`/`Trigger` (`syncjobs/core/types.rs:6-59`) only has `Manual, Interval, Calendar, RealTime, OnStartup, OnConnect`; a Google-Drive-style change feed is just one more `remote_change_token()` source feeding the RealTime path above (`schedule.rs:79-96`), gated on `backend.supports_changes()`.

## 4. `JobSupervisor` concurrency

`daemon/os/shared/job_supervisor.rs`:
- One active job thread at a time, globally serialized (`active: Option<ActiveJob>`, rest queued in a `VecDeque<SyncJob>`) — confirmed by test `serializes_all_daemon_jobs_globally` (max observed concurrency = 1, `job_supervisor.rs:196-226`).
- `enqueue()` (`job_supervisor.rs:61-84`) dedupes by job id (`scheduled: HashSet<String>` → `AlreadyScheduled`) and enforces a 60 s minimum-retry cooldown per id (`MIN_RETRY_INTERVAL`, `last_admitted: HashMap`, GC'd after 24 h) → `RecentlyAttempted`.
- `poll()` reaps a finished thread (`JoinHandle::is_finished()`/`join()`) and starts the next queued job (`job_supervisor.rs:88-109`); called every tick and every 2 s sleep slice (`schedule.rs:253-254,367-369`).
- `cancel_and_join()` clears the queue, flips the active job's `Arc<AtomicBool>` cancel flag, and **blocks until the thread returns** — guarantees no job thread outlives daemon stop or the sync-disabled transition (`job_supervisor.rs:113-127`, also `impl Drop`).
- Actual work: `runner: Arc<dyn Fn(&SyncJob, &AtomicBool)>` defaults to `job::run_one` (`job_supervisor.rs:45`), which validates the job, resolves both endpoints (`crate::connect::resolve_endpoint` — credentials from the OS keyring, out of this surface), runs an optional `run_before` shell hook, calls `bisync::run(a, root_a, b, root_b, opts, cancel, &filter) -> Outcome` (`bisync/os/shared/orchestration.rs:35-49`, called from `job.rs:77`), then an optional `run_after` hook, and persists the result (`job.rs:10-200`).

## 5. Persistence — flat files + one SQLite DB, no config/state daemon-specific database beyond that

All paths are rooted at `crate::support_dirs::sync_data_dir()` / `app_data_dir()` (**not in this assignment's read surface** — its Android resolution is unresolved and load-bearing, see §8).

| Path (relative to its root) | Contents | Source |
|---|---|---|
| `<sync_data_dir>/jobs/<id>.conf` | one `key=value` file per sync job (forward-compatible: unknown keys ignored) | `syncjobs/mod.rs:1-11`, `syncjobs/os/shared/persistence.rs:13-25,43-45,96-106` |
| `<sync_data_dir>/jobs.tsv` | legacy single-file store, imported once then unused | `persistence.rs:18-20` |
| `<sync_data_dir>/results.tsv` | last-run result per job id (TSV) | `syncjobs/os/shared/results.rs:23-25` |
| `<sync_data_dir>/sync_state.sqlite` | **SQLite** (rusqlite, `PRAGMA journal_mode = WAL`) — `pairs` (per sync-pair cursor/bootstrapped/managed state) and `items` (per-side incremental item cache) tables, used by the incremental-mirror fast path | `bisync/os/shared/state_store.rs:4,32-74,394` |
| `<sync_data_dir>/baseline_<pair>.sebl` | full-scan bisync baseline (custom binary format, magic `SEBL\x02`) | `bisync/os/shared/persistence.rs:9,14-16,55-57` |
| `<sync_data_dir>/versions_<pair>/` | reversible-overwrite version store, pruned by retention | `persistence.rs:59-61` |
| `<sync_data_dir>/daemon.heartbeat` | unix-seconds heartbeat, used by `is_running()`/`last_heartbeat_age()` | `daemon/os/shared/state.rs:23-25,112-134` |
| `<sync_data_dir>/daemon.stop` | stop / handoff control (`"stop"` or `"handoff:<gen>"`) | `state.rs:26-28`; parsed `handoff.rs:181-195` |
| `<sync_data_dir>/daemon.log` | plain-text log, capped at 256 KiB, tailed for the GUI log viewer | `state.rs:9,29-31,152-164`; GUI `app/core/menus_sync_jobs.rs:26` |
| `<sync_data_dir>/cadence.txt` | tick length override (2–3600 s) | `state.rs:32-52` |
| `<sync_data_dir>/pause.until` | manual pause deadline (unix secs; `i64::MAX` = forever) | `state.rs:35-37,56-86` |
| `<sync_data_dir>/autopause.txt` | `"b,m"` battery/metered 0/1 flags | `state.rs:38-40,88-109` |
| `<app_data_dir>/sync/daemon.ipc` | `"ip:port"` of the live IPC listener | `daemon/os/linux_os/ipc_storage.rs:8,18-20,38-41` |
| `<app_data_dir>/sync/daemon.generation` | 32-hex generation id of the current singleton owner | `ipc_storage.rs:9,22-24,43-46` |
| `<app_data_dir>/sync/daemon.token` | mode-0600 shared-secret IPC token | `ipc_storage.rs:13,114-260` |
| `<app_data_dir>/sync/exec-grants.journal` | pending exec-grant recovery journal, mode 0600, `openat`+`O_NOFOLLOW` hardened | `ipc_storage.rs:10-11,63-97` |
| `<app_data_dir>/share_server.txt` | configured Share server address, capped 16 KiB | `daemon/os/shared/ipc_host.rs:440-465` |
| `/run/user/<uid>/smart-explorer-sync-daemon.lock` (or `$XDG_RUNTIME_DIR`, else `/tmp/smart-explorer-runtime-<uid>`) | single-instance `flock` guard | `daemon/os/linux_os/platform.rs:95-178` |

- Pause/autopause/cadence/heartbeat/stop are all read/written via an atomic temp-file+rename convention (`state.rs:207-236`) that is transport-agnostic — it works the same whether the writer is another OS process or another thread inside the same process, so these controls need **no IPC redesign** for in-process embedding.
- **Autopause reacts to**: only `battery` and `metered` flags (no "idle" condition exists in this surface) (`state.rs:88-109`, GUI copy at `settings_background.rs:164-190` literally says "Windows-Energiesparmodus" / "(Windows)"). On Linux both underlying checks are **stubs returning `false`** (`daemon/os/linux_os/platform.rs:27-33`), so auto-pause is already a no-op on desktop Linux today — an Android port inherits the same "safe no-op" unless real `BatteryManager`/`ConnectivityManager` checks are added.

## 6. What else the daemon hosts (`ShareHost`, `daemon/os/shared/ipc_host.rs`)

`ShareHost::tick()` (`ipc_host.rs:129-143`), called every scheduling tick and every 2 s sleep slice, drives four sub-systems together with job scheduling:

| Sub-system | Role | Key files (this surface) |
|---|---|---|
| Share service | Iroh/QUIC P2P file-sharing engine + rendezvous-server client; started/stopped by `configure_or_restart_locked`/`stop_service_locked` based on `ShareProfiles.auto_connect`, configured server, and LAN presence + accepted direct peers | `ipc_host_service.rs:1-148`, doc comment `share/mod.rs:1-5` |
| LAN presence | mDNS-style announcer, converts sightings of paired peers into `ShareEvent`s, renders `LanStatus` for GUI/CLI | `daemon/os/shared/lan_runtime.rs` |
| LAN uplink | shares *this host's own* internet connection with other paired devices (Windows-ICS-style); probes a platform `UplinkAdapter`, applies start/stop off the tick thread | `daemon/os/shared/lan_uplink_runtime.rs` |
| Exec grants/sessions | remote command execution over Share, at-most-once semantics, persisted recovery journal | `daemon/os/shared/exec_state.rs`, `exec_ipc.rs`, `exec_grant_journal.rs` (referenced) |
| Mount supervision | starts/stops/lists FS mounts; explicitly Dokany-oriented (`STOP_GRACE` comment cites "Dokany callback", `mount_client.rs:18-19`); **spawns a separate mount-host OS process** that re-attaches via its own token-authenticated IPC handshake (`MountHostAttach/Backend/Status`) | `daemon/os/shared/mount_manager.rs`, `mount_client.rs` |

`ShareHost::tick()` also does a periodic (every 5 s) `reload_now()` of Share identity/profiles independent of the job-scheduling gate — Share/LAN/exec/mount all keep running even when "background sync" (job scheduling) is toggled off (`schedule.rs:253-259`; only the sync-jobs are canceled, `share_host.tick()` is still called unconditionally).

The `daemon/os/shared/mount_*` family is compiled **unconditionally** by `daemon/mod.rs` (not gated behind `windows`/`linux`) and `ipc.rs:114-151` unconditionally dispatches every `Mount*`/`MountHost*` request — an Android host that wants to skip drive mounting per the task's scope would need to either stub these IPC branches or accept the dead code compiling in.

## 7. Stop / cancel API

- `daemon::request_stop()` (pub, `daemon/mod.rs:159`, impl `state.rs:148-150`) writes `daemon.stop = "stop"`. It is **transport-agnostic**: the running `run_daemon()` loop polls this file at least every 2 s (`schedule.rs:344-354`) regardless of whether the caller is a separate OS process or another thread of the same process — directly reusable for in-process embedding as-is.
- On a positive stop check, `stop_daemon()` runs (`schedule.rs:471-479`): `share_host.shutdown_lan()` → `share_host.stop_mounts()` → `job_supervisor.cancel_and_join()` (blocks until the one active job thread observes cancellation and returns) → `clear_heartbeat()`.
- **No per-job-id cancel exists** in this surface — `JobSupervisor::cancel_and_join()` cancels the single active job and drops everything queued; there is no "cancel just job X, keep the daemon running" call.
- The GUI's own "▶ Jetzt" (Run now) button (`app/core/menus_sync_jobs.rs:108-110,223-224`) calls `self.run_job(&id)` — a method **not present anywhere in this assignment's read surface** (defined elsewhere under `app/core`). Since the `IpcRequest` enum has no job-run variant (§2 table) and `JobSupervisor`/`job::run_one` are private to the `daemon` module tree, the natural reading is that "Run now" runs `bisync::run()` directly inside the GUI process, bypassing the daemon/`JobSupervisor` entirely — **unconfirmed, flagged for the main agent** since `self.run_job` is out of scope here.

## 8. OS-adapter dependencies and the Android gap

Every `#[cfg(...)]` gate touching the daemon/syncjobs/autostart tree in this surface only matches `windows` or `target_os = "linux"` — **never `target_os = "android"`**, and per the task's own framing `target_os = "android"` ≠ `target_os = "linux"` even though `target_family = "unix"` holds on both. Concretely:

| Module gated | `#[cfg(...)]` arms present | File |
|---|---|---|
| `daemon::ipc_storage` | `target_os = "linux"`, `windows` | `daemon/mod.rs:44-49` |
| `daemon::mount_process` | `target_os = "linux"`, `windows` | `daemon/mod.rs:78-83` |
| `daemon::platform` | `windows`, `target_os = "linux"` | `daemon/mod.rs:98-103` |
| `daemon::mount_job` / `mount_launch` / `mount_process_environment` | `windows` only (no Linux arm at all — Dokany-specific) | `daemon/mod.rs:68-73,84-89` |
| `syncjobs::platform` (`atomic_replace`/`rename_no_replace`/`sync_parent`) | `target_os = "linux"`, `windows` | `syncjobs/mod.rs:19-24` |
| `autostart::platform` (**the whole module**) | `windows`, `target_os = "linux"` | `autostart/mod.rs:1-6` |
| `bisync::link_fixture` (test-only) | `cfg(unix)` / `cfg(windows)` | `bisync/mod.rs:86-91` — **this one already matches Android** since it uses `cfg(unix)`, not `target_os = "linux"` |

**Consequence**: today, compiling for `aarch64-linux-android`/`x86_64-linux-android` fails outright wherever a `target_os = "linux"`-gated module is referenced unconditionally — e.g. `autostart/mod.rs:11` does `pub use platform::*;` but no `mod platform` block matches on Android, so the whole `crate::autostart` API (`is_enabled`, `enable`, `disable`, `spawn_daemon_now`, `spawn_daemon_handoff_checked`) is undefined; that API is called from `schedule.rs:135,215,235,244`, `ipc_client.rs:269`, and `settings_background.rs:13,24,33,45` — i.e. from the daemon's own startup path, not just optional UI. Same absence pattern applies to `daemon::ipc_storage`, `daemon::mount_process`, `daemon::platform` and `syncjobs::platform`.

**How portable is the underlying code, if the gate were simply widened to `any(target_os = "linux", target_os = "android")`?**

| Piece | Portability if cfg widened | Evidence |
|---|---|---|
| `ipc_storage.rs` (token/addr/generation files, `openat`+`O_NOFOLLOW`, `flock`-style hardening) | Plain libc calls a Linux kernel honours the same way on Android | `daemon/os/linux_os/ipc_storage.rs` (whole file) |
| `syncjobs`/daemon `atomic_replace` (`rename`, `SYS_renameat2` with `RENAME_NOREPLACE`, ENOSYS fallback to `hard_link`+`remove_file`) | Same — a raw syscall, works on any Linux kernel including Android's | `daemon/os/linux_os/platform.rs:43-89`, `syncjobs/os/linux_os.rs:4-35` |
| `metadata_is_link_like` (symlink check) | Portable | `platform.rs:91-93` |
| **`daemon_lock_directory()`** (`$XDG_RUNTIME_DIR` / `/run/user/<uid>` / `/tmp/...` fallback) | **Not portable as-is** — Android apps have no XDG runtime dir and (mostly) no writable `/run` or unrestricted `/tmp`; needs an Android-specific runtime-dir choice (app's own files dir) | `platform.rs:114-139` |
| **`autostart` XDG `.desktop` entry + `Command::new(exe).arg("--sync-daemon")` respawn** | **Not portable** — Android has no autostart desktop-entry mechanism and does not let an app fork/exec a second independent OS process of its own APK's native code the way desktop Linux does; the entire "daemon = second process of the same executable, restarted via autostart/handoff" architecture documented at `daemon/mod.rs:1-7` has no direct Android analogue | `autostart/os/linux_os.rs:6-20,56-167` |
| `run_shell_command()` (`sh -c`, used for job `run_before`/`run_after` hooks) | Unresolved — whether Android's runtime exposes a usable `/bin/sh` for arbitrary user shell hooks is outside this surface | `platform.rs:35-37`; caller `job.rs:167-168` |
| `removable_drives()` / `battery_saver_on()` / `on_metered_network()` | Already stubs (`false`/empty) on Linux — Android would need real `StorageManager`/USB broadcast + `BatteryManager` + `ConnectivityManager` implementations to make OnConnect triggers and auto-pause actually do anything, or it inherits the existing no-op | `platform.rs:23-33` |
| `sync_state.sqlite` (rusqlite) | Needs `native/Cargo.toml`'s rusqlite feature flags checked (bundled vs. system SQLite) for Android cross-compile — **not in this read surface, unresolved** | `bisync/os/shared/state_store.rs:4,394` |

## 9. Minimal embedding API — what exists vs. what's missing

| Need | Existing function | Gap |
|---|---|---|
| (a) Start the loop on a background thread with **given data dirs** | `daemon::run_daemon()` (pub, `daemon/mod.rs:155`) — closest match, but takes **no parameters** | Reads all paths from `crate::support_dirs::*` (global, not injectable), reads handoff state from **process env vars**, and takes a process-wide `flock` singleton — none of which fit "run on a thread inside an already-running app process with an explicit data-dir". A new, smaller entry point is needed that (1) accepts explicit paths, (2) skips the env-var handoff dance and OS-process instance guard (an Android foreground service is already a natural singleton), (3) still drives the same `JobSupervisor`/`ShareHost` tick loop |
| (b) Stop it | `daemon::request_stop()` (pub, `daemon/mod.rs:159`, `state.rs:148-150`) | **None — directly reusable as-is.** File-based, polled by the loop every ≤2 s, indifferent to caller process/thread identity |
| (c) Trigger one job now | **No public function exists.** `job::run_one(job, cancel)` and `JobSupervisor::enqueue()` are `pub(crate)`/`pub(super)`; `IpcRequest` has no run-job variant (§2) | Needs either (i) a new pub wrapper calling `job::run_one` (or `JobSupervisor::enqueue`) directly, or (ii) an independent one-shot `bisync::run()` call exactly like the GUI's own "▶ Jetzt" button appears to do (`self.run_job`, out of scope, §7) |
| (d) List jobs and their last result | `syncjobs::load() -> io::Result<Vec<SyncJob>>` (`syncjobs/mod.rs:35`) + `syncjobs::load_results() -> BTreeMap<String, JobResult>` (`syncjobs/mod.rs:36`) | **None — both already daemon-independent.** They read the flat files directly, need no running daemon/IPC call, and are already used this way by the GUI (`menus_sync_jobs.rs:56,234`) |

---

**Summary for the main agent**: the daemon's *scheduling/job/persistence* core (`schedule.rs`, `job_supervisor.rs`, `job.rs`, `syncjobs::*`, `bisync::run`) is architecturally close to embeddable — its only genuine blockers are (1) the missing `target_os = "android"` cfg arms (likely a small, mechanical widening for the Linux-syscall-based pieces), (2) `support_dirs`/`connect` (unread, likely also need Android-aware paths/credential storage), and (3) the fact that `run_daemon()`/`autostart` today assume "daemon = separate OS process, started by re-exec with a CLI flag" — that whole model, not just a cfg gate, needs to become "daemon = loop on a background thread inside the single Android app process" for a foreground-service/WorkManager host. The IPC layer (loopback TCP + JSON) is protocol-portable to Android but becomes optional/simplifiable for an in-process host, since Rust↔Rust intra-process calls need no socket at all — only cross-process desktop GUI↔daemon needs it.
