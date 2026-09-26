# Terminal discoverability and terminal self-update

Status: implemented on branch `cli-discoverable-update` (base `f7a4802`,
v0.5.163). No local builds or test execution. The single task suite
(`native/test-cli-discoverable-update-task.sh`, workflow
`.github/workflows/cli-discoverable-update-task.yml`) and the terminal release
are owned by the manager and have not run yet; the root graphify refresh is
still due (AGENTS.md → graphify).

Implementation record: M1 `5d100e5`, M2 `a96432e`, M3 `578cb57`, M4 `99999b2`,
M5 `5f1c368`, M6 `adce1cc`; review corrections `f3cda60` (discoverability),
`d84feaf` (update) and the suite/documentation commit that follows them.
Formatting was written by hand (no rustfmt on the workstation); the suite's
rustfmt gate on changed lines is the first real check.

## Goal and deliverables

Owner, 2026-09-26: "der cli version fehlt die discoverable until funktion, eine
update funktion auch" — right after asking another session to "make it
discoverable for 5 minutes with PIN 1454". The terminal must do that itself.

1. `se share discoverable` makes this device (Direct) or a Room discoverable for
   N minutes (default 5, as in the desktop UI and Android) under a name with a
   PIN; lists the running own offers (name, target, remaining time / end time,
   prepared or published); stops one or all offers. Text and `--json` like the
   other `se share` commands. The PIN can come from `--pin`, `--pin-stdin` or a
   hidden prompt; `se share discoverable --minutes 5 --pin 1454` works in one
   line. The terminal learns the offer and its end time reliably.
2. `se update` checks the configured update feed, reports current/available
   version (`--check`, `--json`), downloads `se`, verifies SHA-256, replaces the
   actually installed file (symlinks followed) after re-verifying immediately
   before the swap, works for a terminal-only installation
   (`install-linux.sh --cli-only`) and a desktop installation, and moves a
   running worker of the old version to the new version (version-bound
   handoff). Clear messages without a feed or when already current. Windows uses
   the existing hash-bound helper because a running `se.exe` cannot replace
   itself.

## Stage one: repository evidence

Inspected 2026-09-26 at `f7a4802` (reading summary kept in this plan; the
preliminary read-only survey of the same files is the basis).

Discovery:

- The runtime (`share/core/discovery_signal_commands.rs`) already implements
  `DiscoveryCommand::{Publish, StopPublishing}` with duration > 0, PIN ≤ 1024
  bytes, alias 1–256 bytes without control characters, ≤ 8 offers. It emits
  `DiscoveryEvent::OfferPrepared` before acknowledging a publish, then
  `OfferPublished` when the server confirms, `OfferPrepared` again when a lease
  lapses or the connection drops, and `OfferStopped{reason}` on stop, expiry,
  capability loss or rejection. Offers live in the worker thread and vanish
  without an event when the Share service is stopped or restarted.
- Desktop UI and Android dispatch through
  `share/os/shared/discovery_events.rs::dispatch_discovery_ui_action` →
  `daemon::send_share_command`. The daemon (`ipc.rs`) maps every successful
  command to `IpcResponse::Ok`, discarding `ShareCmdResult::DiscoveryOffer`.
- The only way a client learns `offer_id`/`discoverable_until` is the event
  stream. `ShareHost::drain_for_ui` hands out `ui_events` with
  `std::mem::take`: one shared, draining buffer. A running GUI polls it every
  second, so a terminal cannot rely on seeing its own events. `se share status`
  drains the same buffer.
- `ShareWorkerSnapshot` has a persistent `lan` status but nothing for own
  discovery offers.
- Files at the size limit: `ipc_host.rs` 491 lines, `ipc_protocol.rs` 498,
  `ipc_host_events.rs` 484, `ipc_client.rs` 458, `share/mod.rs` 471.
- CLI conventions: tab-separated `key\tvalue` text, `--json` via
  `serde_json::to_string_pretty`, secrets via `--password-stdin`
  (`cli/setup.rs::read_stdin_secret`, trailing CR/LF trimmed), bare commands
  select the single eligible item or print exact selectors.

Update:

- `updater::check_async`/`apply_staged_update` are GUI-only. The feed
  (`update_source.txt` in app data, else beside the executable) is read by
  `feed_files::read_feed_version`; payloads are downloaded, SHA-256 verified
  against required sidecars and staged in app data under hash-bound names.
- `apply_staged_update` binds the helper to `current_exe()` (the GUI), and the
  helper replaces app, helper and `se` transactionally, then relaunches the app
  with `--updated` and waits for its acknowledgement.
- A terminal-only installation has only `se` (`~/.local/opt/smart-explorer/se`,
  symlinked from `~/.local/bin/se`); `install-linux.sh --cli-only` writes no
  `update_source.txt`, so such an installation has no feed.
- The daemon is started from whichever binary asks (`autostart::daemon_exe` =
  `current_exe`). A client of a newer version sees the running worker as
  `Stale` and performs the version-bound handoff (`ipc_client.rs`
  `restart_worker_for_client` → `launch_replacement`).
- Defect found: staged payloads are created with `File::create` (mode 0644 on
  Linux). `spawn_update_helper` executes the staged helper directly, so a Linux
  desktop update from an HTTP feed cannot start the helper (EACCES). Local-folder
  feeds keep the source mode through `fs::copy`, which hid it.

### Stage-one plan

- Keep the daemon as the single owner of offer state: fold discovery events
  into a persistent "own offers" book inside the daemon, expose it in the worker
  snapshot like `lan`, return the typed publish result over IPC, and give the
  terminal a snapshot request that does not drain the GUI's events.
- `se share discoverable` on top of that; `se update` on top of the existing
  feed/staging/helper code, with an in-place replacement for terminal-only
  installations.

## Research (both rounds) and resolved questions

Recorded syntax and citations: `docs/refs/cli-terminal-update.md`.

1. Parent arguments beside optional subcommands: clap's documented stash
   pattern (`args_conflicts_with_subcommands = true`, flattened args,
   `Option<Subcommand>`) gives `se share discoverable --minutes 5 --pin 1454`
   plus `list`/`stop`. `--json` is declared per struct (no `global`), because a
   global argument before the subcommand would conflict under that setting.
2. Hidden PIN prompt: POSIX termios (`ECHO` off, `ECHONL` on, `TCSANOW`) and
   Windows `SetConsoleMode` without `ENABLE_ECHO_INPUT`; `ctrlc::set_handler`
   restores the saved mode and exits 130 on Ctrl+C. No new crate: `libc` and
   `windows-sys` are present; only the `Win32_System_Console` feature is added
   (features are not recorded in `Cargo.lock`, `--locked` stays valid). When the
   console API is unavailable (e.g. msys pty) the command fails with the
   `--pin-stdin` hint instead of echoing.
3. Reliability of "publish → offer": crossbeam channels are FIFO; the worker
   sends `OfferPrepared` before acknowledging, so after the acknowledgement the
   daemon drains its own event queue and resolves the offer from the book. If
   the offer already ended in between, the book's bounded "recently ended" list
   yields the reason instead of a false error.
4. Installed file: `current_exe` may return a symlink path on some platforms;
   Linux resolves explicitly with `canonicalize`. Windows keeps `current_exe`
   because canonical `\\?\` paths are not safe to pass to the helper.
5. Atomic swap: POSIX `rename` keeps the name visible and refers to old or new
   throughout; the pending copy is created beside the target (same file system).
   A verified backup copy allows rollback when the new `se` does not start.
6. Handoff: the new binary performs it (only it knows its own version). The old
   process runs the replaced file as `se update --complete-install`, which
   probes the worker without starting one: missing → nothing; current →
   nothing; stale → existing `restart_worker_for_client`.
7. Desktop installations: replacing only `se` would leave GUI and `se` on
   different versions that keep handing the worker back and forth. `se update`
   therefore stages app, helper and `se` (the GUI's `stage_from_feed`) and runs
   the same hash-bound helper, bound to the app beside `se`. The helper waits
   for `se` to exit, stops the worker (stop marker), waits for other app/`se`
   processes, replaces all three with rollback and restarts Smart Explorer
   (established "Restart now" semantics; needs a graphical session). Windows
   always takes this path.
8. Feed for terminal-only installations: `install-linux.sh --cli-only` writes
   `update_source.txt` only when none exists (an existing desktop source stays
   unchanged, as the release wrapper requires); `se update --source` sets the
   same app-data override as the app's UPDATE field for older installations.

## Decisions

- New stop reason `DiscoveryOfferStopReason::WorkerStopped`: when the daemon
  stops or replaces the Share service it ends every tracked offer and emits
  `OfferStopped{WorkerStopped}` into the UI events, so GUI and Android stop
  showing offers that no longer exist (previously they stayed visible).
- The daemon refuses a second offer for the same target (same rule as the
  desktop UI and Android, now also across clients), serialized by a publish lock.
- `se update --reinstall` (terminal-only) reinstalls the feed's version even if
  it is not newer; desktop installations use the app updater semantics only.
- The replaced `se` must report exactly the feed's `version.txt`; otherwise the
  previous file is restored. `se update --complete-install <version>` is a
  hidden contract every later `se` keeps; the new binary hands the worker over
  only when its version is the promised one.
- Test prefix for this batch: `cli_task_`.

## Review corrections (Opus review, SHIP AFTER FIXES)

- Clippy: the two `format!("…{}", e)` calls in the updater adapters use inline
  arguments; no other changed line passes a bare identifier positionally.
- Helper from the terminal (FIX 1): `spawn_update_helper(…, from_terminal)`
  gives the helper null standard handles, on Linux in its own session
  (`setsid`), so `se update --json | jq` and a closed terminal do not hold or
  hang up the helper and the restarted app.
- No graphical session (FIX 2): Linux refuses the desktop path before staging
  when `DISPLAY` and `WAYLAND_DISPLAY` are empty.
- Completion (FIX 3, 6): `--complete-install <version>`; the installed file is
  compared with the payload hash right before it starts.
- Swap safety (FIX 4, 5): `updater/os/shared/cli_swap.rs` holds the lock
  (`<se>.update-lock` with the owner PID, stale locks taken over), removes
  leftovers of ended update processes, keeps a hash-verified backup, checks it
  again before a rollback (never deletes it on a mismatch), and restores it on
  Ctrl+C through a handler that shares a mutex with the swap and the commit.
  SIGHUP/SIGTERM are not handled (ctrlc without its `termination` feature);
  their leftovers are removed by the next `se update`.
- Publish failure (FIX 7): an offer the worker kept for the target is stopped
  and the error says so.
- Tests (FIX 8, 9, 10): the daemon refusal and the worker snapshot are pure
  functions with tests; `ShareHostState::new()` keeps tests out of the app data
  folder; the GUI state test covers `WorkerStopped` and offers that vanish; the
  E2E adds `current` handoff, a new `se` that does not start, lock, leftovers,
  unusable source, headless refusal and the PIN leak check. The `replaced`
  handoff needs two builds of different versions and stays covered by the
  decision test.
- Terminal readers (`share status`, `lan`, identity repair, `exec`) use the
  non-draining snapshot, which returns a copy of the UI events; `status` text
  prints the newest 20.
- Handoff: GUI and Android drop offers the daemon no longer lists
  (`DiscoveryUiState::retain_live_offers`), so a handoff to a new daemon ends
  them visibly.
- Deliberately unchanged: daemon messages shared with the German desktop UI and
  Android (second-offer refusal, missing state event) stay German like the
  daemon's other errors; the shared updater errors (hash mismatch, copy) stay
  German as elsewhere in the updater; the desktop rollback archive label uses
  `se`'s version (the app's version cannot be read without starting it; both
  match in installer- and updater-made installations); the IPC request that
  carries the PIN to the local daemon is a transient JSON line as for the
  desktop UI.

## Milestones

### M1 — Daemon owns the terminal-visible offer state

Files: `share/core/discovery_offer_book.rs` (new), `share/mod.rs`,
`share/core/discovery_signal_types.rs` (reason), `share/os/shared/discovery_events.rs`
(label), `daemon/os/shared/ipc_host.rs`, `ipc_host_commands.rs` (new; moved
`send_command`/`drain_for_ui`), `ipc_host_events.rs`, `ipc_host_service.rs`,
`ipc_host_stop.rs`, `ipc_protocol.rs`, `ipc_protocol_bounds.rs` (new; moved
snapshot bounding), `ipc.rs`, `ipc_client.rs`, `ipc_share_client.rs` (new),
`daemon/mod.rs`.

Expected result: publishing returns the offer with id, target, name,
`discoverable_until` and state over IPC; `ShareWorkerSnapshot.discovery_offers`
lists own unexpired offers and survives any number of event drains; a
non-draining snapshot request exists; a stopped/replaced service ends tracked
offers with `WorkerStopped` events; a second offer for the same target is
refused by the daemon.

Acceptance: `cli_task_offer_book_*`, `cli_task_share_command_reply_*`,
`cli_task_snapshot_*`, `cli_task_worker_stop_ends_tracked_offers` plus the
E2E "publish, drain twice, list still shows the offer".

### M2 — `se share discoverable`

Files: `cli/share/discoverable.rs`, `cli/share/discoverable_output.rs`,
`cli/share/discoverable_input.rs` (new), `cli/share.rs`, `cli/share/status.rs`,
`cli/os/{mod,linux_os,windows}.rs`, `native/Cargo.toml` (console feature).

Syntax:

```
se share discoverable [--room ROOM] [--name NAME] [--minutes N] [--pin PIN | --pin-stdin] [--json]
se share discoverable list [--json]
se share discoverable stop [OFFER] [--all] [--json]
```

Expected result: one-line publish prints the offer (state, target, name, end
time, remaining); prepared offers are awaited up to 10 s while connected;
busy target, missing Share service, invalid name/PIN and ambiguous stop
selectors give exact messages; `se share status` lists offers too.

Acceptance: `cli_task_discoverable_*` parser/selection/format tests and the
E2E against a local Share server (Direct and Room publish, list, conflict,
stop one/all, worker stop clears).

### M3 — Updater terminal API

Files: `updater/os/shared/terminal.rs` (new), `cli_swap.rs` (new, lock/swap/undo), `apply.rs`, `config.rs`,
`feed.rs`, `staging.rs`, `core/core.rs`, `os/linux_os.rs`, `os/windows.rs`,
`updater/mod.rs`, `daemon` handoff (`ipc_share_client.rs`).

Expected result: feed check with plausible version; installation detection
(terminal-only vs desktop by the app beside the canonical `se`); staged
payloads executable on Linux; in-place replacement with re-verification right
before an atomic rename, preserved mode, verified backup, commit/rollback;
desktop apply bound to the installed app; the handoff probe never starts a
missing worker.

Acceptance: `cli_task_install_*`, `cli_task_installation_*`,
`cli_task_staged_payload_is_executable`, `cli_task_handoff_*`.

### M4 — `se update`

Files: `cli/update.rs` (new), `cli/mod.rs`, `install-linux.sh`,
`native/test-share-lifecycle-e2e.sh` (installer assertion).

Syntax:

```
se update [--check] [--reinstall] [--source URL|FOLDER] [--json]
```

Expected result: no feed → exit 1 with the `--source` hint; current → status
`up_to_date`; terminal-only → `updated` with the canonical target, SHA-256 and
worker result (`not_running`/`current`/`replaced`); version mismatch or a
replaced file that does not start → previous file restored, exit 1; desktop →
bundle staged and helper started; `--check` changes nothing.

Acceptance: `cli_task_update_*` parser tests and the E2E (no feed, up to date,
reinstall through a symlink from a 0644 feed file, hash mismatch leaves the
file unchanged, version mismatch rolls back), installer dry-run with and
without an existing `update_source.txt`.

### M5 — Single task suite

`native/test-cli-discoverable-update-task.sh` (milestone tests, directly
affected integrations, CLI help, E2E `native/test-cli-discoverable-update-e2e.sh`
with the debug `se` and share-server builds, installer dry-run, per-file
rustfmt, clippy host + `x86_64-pc-windows-gnu` on changed lines) and
`.github/workflows/cli-discoverable-update-task.yml` (`workflow_dispatch`,
required `candidate_sha`).

### M6 — Documentation

README (terminal section, updates), `docs/RELEASING.md` (terminal update,
CLI-only feed), `docs/TODO.md` (item CLI2), `docs/ARCHITEKTUR.md`, CLI help.

## Compatibility expectations (covered by the suite)

- Desktop UI and Android keep publishing/stopping through
  `dispatch_discovery_ui_action`; `send_share_command` still returns `()` and
  accepts the new typed reply; events still reach the GUI unchanged, plus
  `WorkerStopped` when a service ends (`share_remote_task_discovery_ui_*`,
  `android_task_share_status_maps_a_worker_snapshot`).
- `ShareWorkerSnapshot` stays wire-compatible (`#[serde(default)]` field);
  IPC bounding unchanged (`maximum_profile_and_event_backlog_fit_one_ipc_response`
  and the other protocol tests).
- Existing `se share status/lan/worker` output unchanged except the added
  `discoverable` lines/array; the stop barrier and pending-commit refusal keep
  working (`explicit_stop_barrier_blocks_periodic_auto_connect_reload`,
  `stop_refuses_to_strand_a_pending_profile_commit`).
- GUI update flow: `apply_staged_update` keeps the same helper arguments bound
  to the running app; staging and manifest unchanged except the Linux mode fix.
- `install-linux.sh`: desktop mode unchanged; `--cli-only` never overwrites an
  existing `update_source.txt`.
