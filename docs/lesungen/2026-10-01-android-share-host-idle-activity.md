# Android Share host: lifecycle and idle activity - findings

Date: 2026-10-01

## Purpose

Hard facts (file:line) about what starts, stops and restarts the Share host (Iroh endpoint,
signal worker, LAN presence) inside the Android app process, which timers and long-lived
connections an IDLE host keeps (service running, no transfer, no UI interaction), how Iroh 1.0.1
behaves while idle with the options this code sets, and how an incoming Direct request or
connection reaches the Android UI. Gathered to plan a background reaction to incoming Direct
connections without draining the battery. Observed behavior only, no design judgment. Statements
marked "derived" are arithmetic on cited constants; statements marked "not read" name code outside
the allowed reading surface.

Path conventions: Rust paths are relative to `native/src/`; Kotlin paths are relative to
`android/app/src/main/java/app/smartexplorer/android/`; dependency paths are relative to
`/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`.

## Files actually read

Rust, `share/core/` (all read in full unless noted): `node.rs`, `node_accept.rs`,
`node_sessions.rs`, `keepalive.rs`, `service.rs`, `server.rs`, `signal_worker.rs`,
`signal_connection.rs`, `signal_connector.rs`, `signal_presence.rs`, `signal_subscriptions.rs`,
`discovery_signal_maintenance.rs`, `discovery_signal_offline.rs`, `lan_settings.rs`,
`lan_status.rs`, `lan_uplink_policy.rs` (lines 1-200; the rest is tests), `endpoint_routes.rs`,
`peer_endpoint_source.rs`, `direct_reciprocal_worker.rs`, `direct_reciprocal_coordinator.rs`,
`configuration_runtime.rs`.

Rust, `share/os/shared/`: `transport_options.rs`, `lan_presence.rs`, `system.rs`,
`poll_status.rs`, `lan_settings_store.rs`; `share/mod.rs` (module map and re-exports).

Rust, `daemon/os/shared/` and `daemon/os/android/`: `embedded.rs`, `run_loop.rs`, `ipc_host.rs`,
`ipc_host_service.rs`, `ipc_host_stop.rs`, `ipc_host_commands.rs`, `ipc_host_events.rs`,
`ipc_host_ui_events.rs`, `lan_runtime.rs`, `lan_uplink_runtime.rs`, `live.rs`, `host_state.rs`,
`state.rs`, `ipc_share_client.rs`, `android/platform.rs`; partially: `ipc_client.rs` (100-150), `ipc.rs` (30-60,
205-260), `ipc_listener.rs` (1-100),
`handoff.rs` (1-125 and the `stop_requested*` / `stop_control_checked` functions),
`mount_manager.rs` (the `tick` function only). Grep-only for timers: `job_supervisor.rs`,
`schedule.rs`, `catch_up.rs`, `boot_marker.rs`, `ipc_host_direct_event*.rs`, `ipc_host_legacy_events.rs`,
`mount_manager_state.rs` (no Share-idle timer found there; the only 100 ms read loop in `ipc.rs:217-257` (`serve_mount_control`) belongs to the
mount control channel).

Rust, `mobile/os/shared/`: `runtime.rs`, `init.rs`, `sys.rs`, `domains/mod.rs`,
`domains/background.rs`, `domains/share_state.rs`, `domains/share_status.rs`,
`domains/share_settings.rs`, `domains/share_peers.rs`, `domains/share_requests.rs`.

Kotlin: `service/BackgroundService.kt`, `service/BackgroundController.kt`,
`service/BackgroundText.kt`, `service/ServiceNotifications.kt`, `service/TaskKeeper.kt`,
`service/TaskForegroundService.kt`, `service/ExecHostNotifier.kt`, `system/HostMonitor.kt`,
`core/Core.kt`, `api/ShareApi.kt`, `ui/share/ShareViewModel.kt`, `ui/share/ShareScreen.kt`.

Docs: `docs/superpowers/plans/2026-09-25-android-apk/api.md` lines 1-120 and 205-363
(sections 1, 3, 4.1, 4.7, 5).

Dependencies (read-only): `iroh-1.0.1/src/` - `endpoint.rs` (selected ranges), `endpoint/presets.rs`,
`endpoint/quic.rs` (selected), `defaults.rs`, `portmapper.rs`, `lib.rs` (grep), `net_report.rs`
(1-760), `net_report/{defaults,options,probes}.rs`, `net_report/reportgen.rs` (selected),
`socket.rs` (selected), `socket/remote_map/remote_state.rs` (selected),
`socket/transports/relay/actor.rs` (selected); `iroh-relay-1.0.1/src/` - `ping_tracker.rs`,
`defaults.rs`, `protos/relay.rs` (header), `quic.rs` (270-330), `relay_map.rs` (grep),
`client.rs` (grep), `http.rs` (grep); `noq-proto-1.0.1/src/config/transport.rs` (selected);
`mdns-sd-0.11.5/src/service_daemon.rs` (selected), `service_info.rs` (grep), `dns_cache.rs` and
`dns_parser.rs` (grep).

Referenced by the read code but outside the allowed surface (not opened): `session.rs`, `signal_commands.rs`,
`discovery_signal_state.rs`, `tracked_signal_sender.rs`, `tracked_signal_dispatch.rs`, `lan_presence_match.rs`,
`direct_reciprocal_transport.rs`, `connection_events.rs`, `exec_server.rs`, `domains/share_exec.rs`, `native/src/net/`
(see "Offene Punkte").

---

## 1. Android Share host lifecycle

### 1.1 What starts the Share host

- There is no explicit `share.start` method. The facade's Share method table is
  `domains/mod.rs:134-166` (status, watch, setServer, setOnline, setName, discoverable, ...); the
  service is a side effect of the embedded daemon's reload (below).
- Process start chain (Kotlin): `BackgroundController.onAppStart` runs "from
  `SmartExplorerApp.onCreate` (any process start: UI, worker, boot)"
  (`service/BackgroundController.kt:47-59`). It calls `HostMonitor.start`, then
  `SyncApi.ensureDaemon()` (`bg.ensureDaemon`), then `apply(app)`. `SmartExplorerApp` itself was not read.
- `bg.ensureDaemon` -> `background::ensure_daemon` -> `crate::daemon::ensure_embedded_daemon(READY_WAIT)`
  with `READY_WAIT` = 10 s (`mobile/os/shared/domains/background.rs:12,33-37`; `daemon/os/shared/embedded.rs:23-41`).
- `init` also starts it: `if runtime.config().start_daemon { nudge_daemon(runtime) }`
  (`mobile/os/shared/init.rs:34-36`); `nudge_daemon` spawns thread `mobile-daemon-nudge` calling
  `ensure_embedded_daemon(Duration::ZERO)` (`init.rs:85-96`). `startDaemon` is `true` in the
  documented init config; `false` only for tests (`api.md:38,40-43`).
- `ensure_embedded_daemon` -> `start_if_needed` joins a finished handle and spawns thread
  `background-worker` running `run_daemon_with(None)` (`embedded.rs:53-74`). Module doc: starts "on first
  need and is never stopped because of the UI" (`embedded.rs:1-5`).
- `run_daemon_with` (`daemon/os/shared/run_loop.rs:39-231`): instance guard, `start_listener`
  (loopback TCP IPC, `run_loop.rs:87`), `share_host.reload_now()` (`run_loop.rs:95`), `mark_initialized`
  (`:101`), `live::serve` (`:103`), then the 2 s loop (section 2.2).
- `reload_now` -> `reload_now_locked` -> `configure_or_restart_locked`
  (`daemon/os/shared/ipc_host.rs:208-217,219-313`, call at `:306`;
  `ipc_host_service.rs:3-82`). The service runs iff
  `share_service_requested(suspended, server, auto_connect, lan_presence && has_direct_peers)` =
  `!suspended && auto_connect && (!server.trim().is_empty() || lan_only_possible)`
  (`ipc_host_service.rs:14-27,86-93`).
  - `server` = content of `<app data>/share_server.txt`, max 16 KiB (`ipc_host.rs:394-419`).
  - `auto_connect` = persisted profile flag (`state.profiles.auto_connect`; default value not read).
  - `lan_only_possible` = `LanSettings.presence_enabled` (default `true`, `share/core/lan_settings.rs:6-13,24-33`)
    AND at least one contact with `access_state == Accepted` (`ipc_host_service.rs:14-21`).
- The sync "autostart" flag does not gate Share: "A daemon may have been started only for a Share
  session. Scheduled sync work is permitted exclusively after the user enabled background sync."
  (`run_loop.rs:107-112`; sync work gated at `run_loop.rs:143-149`).
- Starting the service: `ShareService::start_with_profile_home` (`ipc_host_service.rs:64-69`;
  `share/core/service.rs:299-404`) creates
  - the Iroh node `ShareIrohNode::start_with_repair_store` (`service.rs:337-343`; `share/core/node.rs:62-141`):
    a Tokio multi-thread runtime named `share-iroh` with Tokio's default worker count
    (`node.rs:86-92`), endpoint bind (`node.rs:108`), `EndpointRoutes::start` (`node.rs:109`) and the accept loop
    (`node.rs:139`, `node_accept.rs:13-61`);
  - the `DirectReciprocalCoordinator` thread `direct-reciprocal` (`service.rs:344`;
    `direct_reciprocal_coordinator.rs:241-246`) and `configuration_runtime::schedule_current` (`service.rs:348`);
  - the signal worker thread `share-signal` (`service.rs:373-389`).
- App-side switches that reach the same gate: `share.setOnline(true)` persists `auto_connect = true`
  and calls `refresh_share_worker_checked` (`share_settings.rs:90-98,102-118`), which sends IPC `RefreshShare` over
  loopback TCP (`ipc_client.rs:113-128`) to `host.refresh_now()` (`ipc.rs:43-46`), which un-suspends and
  reloads (`ipc_host.rs:315-324`); if there is no server and no LAN peer it reports "Share-Worker wurde nicht
  aktiv ..." (`share_settings.rs:112-118`). `share.setServer` writes/removes `share_server.txt` and calls
  `reconfigure` (`share_settings.rs:68-88`, `share_state.rs:399-417`). A changed server, node id, device id/name,
  lookup id or direct secret restarts the service (`ipc_host_service.rs:44-77`).

### 1.2 When it stops

- `share.setOnline(false)`: persists `auto_connect = false`, then sends `ShareCmd::Stop` over the loopback
  IPC (`share_settings.rs:119-128`; `ipc_client.rs:130-132`; `ipc_share_client.rs:14-29`; handled at
  `ipc.rs:47-51`) -> `stop_locked`: `suspended = true`, removes the
  service, `service.cmd(Stop)` (`ipc_host_commands.rs:28-30`; `ipc_host_stop.rs:3-22`). `ShareService::cmd`
  runs `iroh.stop_sharing()` first (`service.rs:37-72`): cancels exec jobs, closes all outgoing/incoming sessions,
  `endpoint.close()` (`node.rs:268-278`).
- `suspended` is in-memory only (`ShareHostState::new` sets `false`, `ipc_host.rs:83-93`); only an explicit
  `RefreshShare` clears it (`ipc_host.rs:61-63,315-324`). The persisted `auto_connect = false` survives a restart.
- A periodic reload whose gate is false (auto_connect off, or no server and no accepted LAN peer, or suspended)
  stops the service (`ipc_host_service.rs:22-36`).
- Daemon loop end: stop control `daemon.stop` or an unreadable stop control (treated as stop) ->
  `stop_daemon` (`share_host.shutdown_lan()`, `stop_mounts()`) and `Serving::drop` -> `stop_service_locked`
  (`run_loop.rs:121-124,151-154,399-409,436-444`; `live.rs:117-137`; `handoff.rs:154-181`).
- Dropping the owning `ShareService` calls `stop_sharing` and `request_stop` (`service.rs:424-432`).
- Process death: no code runs; nothing in the read files writes a "running" marker for Share.

### 1.3 Behavior per background mode

- Modes: `off`, `periodic`, `persistent` (`service/BackgroundController.kt:29-38`). `ensureDaemon` is called at
  every process start regardless of mode (`BackgroundController.kt:48-59`), so the embedded daemon and, via the gate
  in 1.1, the Share host run in every mode while the process lives.
- `off` only switches the sync flag off (`BackgroundController.kt:30-31,78-84`); "disabling only stops scheduling
  (Share stays)" (`background.rs:67-68`; `api.md:256-258`).
- `periodic` (WorkManager `SyncWorker`): `PeriodicWorkRequestBuilder<SyncWorker>(bgIntervalMin, MINUTES)` with
  network/charging/battery constraints (`BackgroundController.kt:114-129`) and `stopBackgroundService` (`:86`).
  `BackgroundService` does not run in this mode; `TaskForegroundService` (dataSync) runs only while user tasks or a
  daemon job run (`TaskKeeper.kt:21-27`, `TaskForegroundService.kt:22-27`). The read Kotlin files contain no `WakeLock`, `WifiLock`, `AlarmManager`
  or `JobScheduler` (grep over `service/*.kt`, `HostMonitor.kt`, `Core.kt`, `ShareApi.kt`, `ShareViewModel.kt`,
  `ShareScreen.kt`: no hit); the only lock is the Wi-Fi `MulticastLock` (section 5).
- `persistent`: `BackgroundService` (foreground service type `specialUse`, `service/BackgroundService.kt:75`) "keeps the
  process and with it the embedded daemon (scheduler, real-time jobs, Share host) awake" (`BackgroundService.kt:25-30`);
  `START_STICKY` (`:63`); started by `apply`, `onBoot` and `onUiVisible(true)` (repair of a failed boot start)
  (`BackgroundController.kt:62-68,71-87,93-98,131-142`). Starting it from the background can be refused on API 31+
  (caught `IllegalStateException`, `BackgroundController.kt:131-141`, `BackgroundService.kt:72-85`).
- `power_save` and `metered` from `sys.hostState` are read only by the daemon's sync auto-pause
  (`daemon/os/shared/state.rs:112-124` <- `daemon/os/android/platform.rs:39-45` <- `host_state.rs`); a grep over
  `daemon/`, `share/` and `mobile/` finds no other consumer. `wifi` and `charging` are not read by Rust
  (`mobile/os/shared/sys.rs:51-61`). `foreground` only sets the poller cadence (section 2.5).

### 1.4 What restarts it after process death

- Any new process runs `onAppStart` -> `ensureDaemon` -> Share per the gate (1.1). Process starts visible in the read
  surface: the UI; WorkManager periodic work (`periodic` and `persistent` modes; in `persistent` it is "only the
  fallback for a service the system ended", `BackgroundController.kt:110-129`); `BOOT_COMPLETED` /
  `MY_PACKAGE_REPLACED` via `onBoot`, which only schedules the periodic work and starts the persistent service
  (`BackgroundController.kt:89-98`; `BootReceiver.kt` not read); the system re-creating `BackgroundService`
  (`START_STICKY`, `BackgroundService.kt:63`).
- Inside Rust: the embedded worker is restarted only when `ensure_embedded_daemon` is called again
  (`embedded.rs:53-74`); the read files contain no watchdog thread.

### 1.5 What the persistent `BackgroundService` does for Share

- Keeps the process in the foreground; notification text "Smart Explorer im Hintergrund - n Jobs, Share online|offline"
  from `HostMonitor.shareOnline` (`ServiceNotifications.kt:82-92`; `BackgroundService.kt:108,130-136`).
- Collects `Core.events`; on `CoreEvent.ShareRequest` posts notification `ID_SHARE_REQUEST` (channel `CHANNEL_SHARE`,
  "Neue Share-Anfrage", content intent `ShowShareRequests`) unless `TaskKeeper.uiVisible`
  (`BackgroundService.kt:99-107,138-142`; `ServiceNotifications.kt:116-124`). Within the read surface this is the
  only collector that posts a notification for `ShareRequest`.
- Every 60 s: `bg.status` and `sync.jobs` (`BackgroundService.kt:94-97,120-128,155`); no Share call.
- It does not start, stop or configure the Share service.

---

## 2. Periodic activity of an idle Share host

"Idle" below: service running, no transfer, UI not interacting. Intervals are the code's constants.

### 2.1 Thread inventory (idle wake pattern)

| Thread | Created at | Idle wake pattern |
|---|---|---|
| `background-worker` (daemon loop) | `embedded.rs:64-66` | every 2 s (2.2) |
| `daemon-ipc` (loopback listener) | `ipc_listener.rs:34-36` | non-blocking `accept`, `sleep(100 ms)` on `WouldBlock`, `stop_requested_for` (reads `daemon.stop`) each pass (`ipc_listener.rs:38-46,74-76`; `handoff.rs:154-181`) |
| `share-signal` | `service.rs:373-376` | connected: at least every 500 ms (2.3); not connected or no server: every 25-50 ms (2.4) |
| `share-signal-connect` | `signal_connector.rs:23-25` | one per connect attempt |
| `direct-reciprocal` | `direct_reciprocal_coordinator.rs:241-246` | Condvar wait until the next due task (`direct_reciprocal_worker.rs:78-111`) |
| `share-iroh` Tokio workers + Iroh tasks | `node.rs:86-92` | event/timer driven (section 3) |
| `se-lan-presence` | `share/os/shared/lan_presence.rs:48-68` | blocks in `browse.recv()` |
| `mDNS_daemon` (mdns-sd) | `mdns-sd service_daemon.rs:184-187` | `poller.wait(timeout = earliest timer)` (section 5) |
| `share-poller` | `domains/share_state.rs:292-297` | 300 ms / 5 s / 60 s (2.5) |
| `core-events` (Kotlin) | `core/Core.kt:158-160` | `pollEvents(1000)` in a loop (2.6) |

### 2.2 Embedded daemon loop (`run_loop.rs`)

- Outer iteration every `tick_secs` = `cadence.txt` (clamped 2..3600) or default 15 s
  (`state.rs:7,43-48,194-200`; `run_loop.rs:119-230,369-397`): `scheduling_controls()`, `load_configured_jobs()`
  (jobs file), `current_drives()` (Android returns an empty list, `android/platform.rs:35-37`), `service_catch_up`,
  `write_heartbeat()` (`run_loop.rs:120-186`).
- Inner slices of 2 s until `tick_secs` is reached (`run_loop.rs:188-229`). Per slice (`:194-228`): two
  `autostart::is_enabled()` calls, up to two `stop_requested` file reads, `scheduling_controls()` (reads
  `cadence.txt`, `pause.until`, `autopause.txt`: `state.rs:43-48,72-94,112-130`), `share_host.tick()` (`:226`) and
  `write_heartbeat()` = `std::fs::write(daemon.heartbeat, now)` (`:227`; `state.rs:132-134`). The slice length is
  fixed at 2 s and independent of `cadence.txt`.
- `ShareHost::tick` (`ipc_host.rs:140-154`): `mounts.tick()` (no work without mounts, `mount_manager.rs:422-451`),
  `lan_tick()`, `drain_events()`, and `reload_now()` when `last_reload.elapsed() >= 5 s` (`:144-153`); derived: every
  third 2 s slice, about every 6 s.
- `reload_now_locked` (`ipc_host.rs:219-313`) per reload: reads `share_server.txt` (`:252-258,394-419`),
  `ShareIdentity::load_or_create` (`:259`), `exec_grant_journal::load_pending` (`:271`),
  `ShareProfiles::load_checked` (`:277`), then `configure_or_restart_locked` which, for an unchanged running service,
  calls `service.cmd(ShareCmd::ConfigureProfiles { profiles })` (`ipc_host_service.rs:78-80,105-117`). `ShareService::cmd`
  waits up to 5 s for the signal worker's acknowledgement (`service.rs:37-72`) while `reload_now_locked` holds the
  host state lock (`ipc_host.rs:221-224`). The worker applies it with `RuntimeConfiguration::apply` (state clone,
  `configuration_changed` comparison, `schedule_snapshot`; `configuration_runtime.rs:52-114`). Whether handling
  `ConfigureProfiles` on a connected worker also publishes presence is in `signal_commands.rs` (not read);
  `signal_worker.rs:206-211` resets the presence timer when a command reports `published`.
- `drain_events` (`ipc_host_events.rs:5-443`) returns early when nothing is queued (`:21-28`). Otherwise events
  that change a contact or room (`DirectAvailable`, `DirectOffline`, `DirectAccessAccepted`, `Room*`,
  `LanPeerSeen`, `LanPeerLost`: `:86-131,152-314`) set `changed`, which triggers
  `ShareProfiles::mutate_persisted` (disk write) and `configure_service` (`:338-380`).

### 2.3 Signal worker (`share/core/signal_worker.rs`) - connected

- Loop body runs at least every 500 ms: sockets have a 500 ms read timeout (`SIGNAL_READ_POLL`,
  `signal_connection.rs:15,72,102,117`); `read_message` returns `WouldBlock`/`TimedOut` which the loop ignores
  (`signal_worker.rs:290-317`). Each pass does `drain_repair_completions`, `commands.try_recv`, the maintenance
  checks and `route_revision()` (`signal_worker.rs:181-267`).
- Heartbeat `ClientMsg::Heartbeat` every 20 s (`keepalive.rs:25`; `signal_worker.rs:245-251`); no `Pong` within 40 s
  -> error event and reconnect (`keepalive.rs:26`; `signal_worker.rs:236-244`).
- Presence refresh every 60 s (`keepalive.rs:27`; `signal_worker.rs:252-267`) and immediately when
  `iroh.route_revision()` changed (`:234-235,252-266`). `publish_all` sends `PublishDirect{presence}` (if
  `direct_online`), `WatchDirect{lookup_id}` per `auto_connect` contact, `RequestDirect` once per pending
  `auto_connect` contact when tracked-direct was not negotiated, `JoinRoom{room_id,presence}` per `auto_join` room
  (`signal_worker.rs:326-379`). A presence
  is HMAC-signed, carries relay URL and all candidate addresses, and expires 300 s after signing
  (`signal_presence.rs:8-47`, `expires_at = now + 300` at `:20`).
- `send_pending_tracked` every 2 s, only if the server negotiated `tracked_direct` (`keepalive.rs:28,63`;
  `signal_worker.rs:268-281`); its cost with an empty outbox is in `tracked_signal_sender.rs` (not read).
- Discovery (only if `discovery_exchange` was negotiated, `signal_worker.rs:282-289`):
  `DiscoverySignalRuntime::maintain` each pass; sends `ListDiscoveries` when `now >= next_list_request_at` and
  re-arms with `DISCOVERY_LIST_REFRESH_INTERVAL`; an unanswered list or publish ends the connection
  (`discovery_signal_maintenance.rs:65-91,173-185`). The interval constants live in `discovery_signal_state.rs`
  (not read). Own offers re-publish when due (`:81-83,134-171`).
- WebSocket transport answers server WebSocket `Ping` with `Pong` inside `read_message`
  (`signal_connection.rs:160-163`).

### 2.4 Signal worker - not connected, and reciprocal repair

- Reconnect backoff: starts 1 s, doubled after each attempt, capped 30 s, reset to 1 s after a successful connect
  (`signal_worker.rs:42,82,92,102-103`). Each attempt: thread `share-signal-connect` (`signal_connector.rs:18-30`),
  endpoint list split at `,` / `;` and tried in order (`signal_connection.rs:31-47,188-195`), DNS (skipped for literal
  IPs, `:233-235`) through a fresh current-thread Tokio runtime and `hickory_resolver` per attempt (`:228-258`), TCP connect timeout 10 s, DNS timeout 10 s
  (`:13-14,220-222,245`), TLS and WebSocket handshake bounded by 10 s socket deadlines (`:99-101`); `lan_ips()` is called
  for the `Hello` (`signal_connector.rs:42`, `share/os/shared/system.rs:1-24`).
- While waiting, the thread wakes every 50 ms (`wait_offline_backoff`, `discovery_signal_offline.rs:49-78`, 50 ms at
  `:65-68`) and every 25 ms while a connect attempt runs (`wait_for_connection`, `:16-47`, 25 ms at `:35`).
- No server configured: `wait_offline_backoff(30 s)` is called in a loop for the life of the service
  (`signal_worker.rs:58-71`).
- Reciprocal Direct repair: scheduled for each contact with `auto_connect`, `Accepted` and a current presence at
  `schedule_current` and on every `apply` (`configuration_runtime.rs:117-182`, filter at `:141-154`); the repair
  dials the peer and caches the connection (`node_sessions.rs:37-127,233-315`). Result handling: complete/already
  complete -> task blocked for this authorization generation; transient -> retry after 2 s, 5 s, 15 s, 60 s, then
  300 s +/- 30 s jitter without a limit; unsupported -> 1800 s +/- 180 s
  (`direct_reciprocal_worker.rs:44-67`; `direct_reciprocal_coordinator.rs:358-381`). Cached outgoing sessions are
  evicted only on authorization change, close detection at next use or explicit invalidation
  (`node_sessions.rs:170-189,277-292`; `node.rs:226-255`); no idle eviction timer in the read files. Whether
  `run_outgoing` closes the connection after the exchange is in `direct_reciprocal_transport.rs` (not read).

### 2.5 LAN presence, uplink runtime, poller

- `lan_tick` runs on every daemon tick, whether or not the service runs (`ipc_host.rs:170-206`).
  `LanRuntime::tick` (`lan_runtime.rs:127-151`): `LanSettings::load()` file read
  on every tick (`:182-190`; `lan_settings_store.rs:16-32`); `crate::net::gather_interface_facts()` at most every 5 s
  (`FACTS_INTERVAL`, `:18,192-208`); mDNS start retry every 60 s while it fails (`START_RETRY`, `:19,210-228`);
  `announce` is a no-op when the announcement is unchanged (`lan_presence.rs:89-99`); the announcement is withdrawn
  when no node id or no bound port is known (`lan_runtime.rs:230-251`; ports come from the running service,
  `ipc_host.rs:171-186`); `reconcile` emits `LanPeerSeen` when candidates changed or `lan_seen_at` is older than
  `LAN_PRESENCE_TTL_SECS / 2` (`lan_runtime.rs:305-328`; the TTL constant is in `lan_presence_match.rs`, not read).
- Uplink runtime: `tick_uplink` runs on every tick (`lan_runtime.rs:149,154-180`); `UplinkRuntime::tick` calls
  `adapter.probe(...)` each tick and `adapter.internet_ifaces(...)` at most every 20 s (`INTERNET_PROBE_INTERVAL`,
  `lan_uplink_runtime.rs:17,86-110`) even with uplink sharing disabled (default `false`, `lan_settings.rs:27-28`).
  Policy timers (5 s start debounce, 90 s stop grace, 15 s uplink loss) apply only when enabled
  (`lan_uplink_policy.rs:6-8,69-78`). `api.md:363` states LAN uplink is not on Android; the Android adapter behind
  `crate::net::uplink_adapter()` was not read.
- `share-poller` (`domains/share_state.rs:288-307`): wait interval 300 ms while the Share page is watched
  (`share.watch`) or a pairing/discovery is in flight, else 5 s while `foreground`, else 60 s
  (`WATCH_INTERVAL` / `FOREGROUND_INTERVAL` / `BACKGROUND_INTERVAL`, `:26-28`; `wait_for_next_poll`, `:259-275`;
  `pairing_in_flight`, `:277-286`). Initial `foreground` is `true` (`:228-233`); `sys.hostState.foreground` sets it
  (`sys.rs:54-61`; `domains/mod.rs:77-81`); `HostMonitor` pushes `foreground = false` at start because its flag
  starts `false` (`HostMonitor.kt:49-50,68,85-103`). Each poll: `drain_share_events_in_process` -> `ShareHost::drain_for_ui`
  -> reload when older than 5 s, `drain_events`, snapshot incl. a clone of all profiles
  (`live.rs:76-88`; `ipc_host_commands.rs:87-115,118-144`), then builds the status JSON and emits `share` when it
  changed (`share_state.rs:309-354`). `wake()` (immediate poll) is called only from command handlers
  (`share_settings.rs`, `share_peers.rs`); nothing in the daemon wakes the poller. `share_exec::watch_host_activity`
  (`share_state.rs:306`) and `share_exec::provider()/host_activity()` per poll (`:316-317`) are not read.

### 2.6 Kotlin side

- `core-events` thread: loop `NativeBridge.pollEvents(1000)` (`Core.kt:40,158-160,177-202`); `Hub::poll` waits on a
  Condvar for at most the timeout (`mobile/os/shared/runtime.rs:59-78`); the empty result of a full-timeout poll does
  not trigger the extra 1 s sleep (`Core.kt:193-200`). Derived: about one JNI return per second while idle. The JNI
  glue was not read.
- `BackgroundService` refresh every 60 s in persistent mode (`BackgroundService.kt:94-97,155`).
- `ExecHostNotifier`: re-check every 5 s only while an exec notification is shown (`ExecHostNotifier.kt:42,63-67`);
  started from `ShareScreen` (`ShareScreen.kt:73`; the only call in the read files).
- `ShareScreen`: 1 s timer only while an own discovery offer is active and the page is composed (`ShareScreen.kt:88-99`).
- `HostMonitor`: event driven - power-save/charging broadcast receiver, default-network callback
  (`onAvailable`/`onLost`/`onCapabilitiesChanged`), `CoreEvent.Share` -> `share.status` (`HostMonitor.kt:55-71,117-143`).
- WorkManager `SyncWorker` every `bgIntervalMin` in `periodic` and `persistent` mode (`BackgroundController.kt:114-129`).

### 2.7 Iroh endpoint options set by this code

`node.rs:93-108`:

- `Endpoint::builder(presets::Minimal)`: the preset only sets the rustls crypto provider; no address lookup, no relay
  (`iroh endpoint/presets.rs:45-79`; `Builder::empty` = "no address lookup services, and `RelayMode::Disabled`",
  `endpoint.rs:190-197`).
- `.secret_key(identity.iroh_secret)`, `.alpns([ALPN, EXEC_ALPN])` (`node.rs:25,100-102`).
- `.relay_mode(...)`: `RelayMode::custom(relay_urls)` when `transport_options::load(server)` yields URLs, otherwise
  `RelayMode::Disabled` (`node.rs:93-99`; `transport_options.rs:7-16`). URLs come from `relay_urls_from_signal(server)`
  (`session.rs`, not read), or from `SE_SHARE_RELAY_URL`.
- `.transport_config(iroh_transport_config())` (`node.rs:104`): `max_idle_timeout` 20 s, `keep_alive_interval` 5 s,
  `default_path_keep_alive_interval` 5 s, 64 bidi / 0 uni streams, flow-control windows
  (`keepalive.rs:5-9,101-118`).
- `.clear_ip_transports()` only when `SE_SHARE_RELAY_ONLY=1` (`node.rs:105-107`; `transport_options.rs:13-15`).
- Not set anywhere in the read Rust surface (grep): `address_lookup`/`clear_address_lookup`, `net_report_config`,
  `portmapper_config`, `dns_resolver`, `proxy_*`, `bind_addr`, `hooks`, `external_addr`, `network_change`,
  `insert_relay`, `home_relay_status`. Sockets are the builder defaults: IPv4 `0.0.0.0` and IPv6 `[::]`, random ports
  (`endpoint.rs:191-197`); the bound ports feed the LAN announcement (`node.rs:151-162`; `service.rs:187-190`).
- Our code uses `endpoint.watch_addr()`, `endpoint.online()` and `endpoint.addr()` only
  (`endpoint_routes.rs:41-72,81-97`): `watch_addr` updates bump `revision`
  (`:41-48`); with a relay configured a task repeats `online()` under `iroh::NET_REPORT_TIMEOUT` (5 s, `iroh lib.rs:292`;
  `net_report/defaults.rs:14`) until a relay handshake completed, then bumps `revision` once (`:50-72`). Each `revision`
  change causes an immediate `publish_all` (`signal_worker.rs:234-266`).
- mDNS is not an Iroh address lookup here: the `iroh-1.0.1/src/address_lookup/` tree has only `dns.rs`, `memory.rs`,
  `pkarr.rs`; LAN discovery is our own `mdns_sd` use (section 5).

---

## 3. Iroh 1.0.1 idle behavior with these options

### 3.1 Relay client (home relay), `socket/transports/relay/actor.rs`

- Ping: `PING_INTERVAL` = 15 s (`:67-71`); `ping_interval = time::interval(PING_INTERVAL)`, first ping immediately,
  `MissedTickBehavior::Delay` (`:532-535`); reset on every received frame (`:640-641`); a ping is also sent on
  `CheckConnection` after network changes (`:586-596`).
- Ping timeout: `PingTracker::default()` with `PING_TIMEOUT` = 5 s (`iroh-relay ping_tracker.rs:5,28-31`); effective
  timeout = `clamp(3 x last_rtt, 500 ms, 5 s)` once an RTT is known (`:8,83-91`); expiry -> `RunError::PingTimeout`, the
  connection is torn down (`actor.rs:561-563`).
- Server pings: protocol doc "server occasionally sends Ping; client responds with Pong"
  (`iroh-relay protos/relay.rs:1-8`); the client answers (`actor.rs:538-541,692`). `PING_INTERVAL` 15 s also exists
  in `protos/relay.rs:31-36` behind feature `server`; server-side jitter and expectations (`server/client.rs:339`) were
  not read.
- Reconnect: exponential backoff 10 ms .. 16 s, jittered, unlimited retries; reset when a pong had arrived
  (`actor.rs:323-357`). Connect timeout 10 s (`:80-84`). The home relay actor never exits; non-home relay actors close
  after `RELAY_INACTIVE_CLEANUP_TIME` = 60 s without writes (`:9,64-65,646`).
- The home relay is chosen from the net report's `preferred_relay` (`actor.rs:1124-1149`; `socket.rs:1964-1975`); a relay
  connection therefore starts only after a net report produced a preferred relay.
- Transport: relay `Client` is a stream/sink over the TCP connection to the relay (`actor.rs:11-12,20-23`), dialed as
  WebSocket, `https` -> `wss`, `http` -> `ws` (`iroh-relay client.rs:258-283,394-413`).

### 3.2 net_report / QAD / periodic re-run

- Periodic timer: `periodic_re_stun_timer`, random 20-26 s (`socket.rs:2004-2019`); created with
  `new_re_stun_timer(false)` at actor start (`:1086`, first tick immediate) and re-armed with
  `new_re_stun_timer(true)` after every report (`:1554-1561`); each tick calls `re_stun(UpdateReason::Periodic)`
  (`:1536-1540,1767-1771`).
- A run is skipped when the relay map is empty (`socket.rs:799-803`, i.e. `RelayMode::Disabled`). Otherwise it calls
  `port_mapper.procure_mapping()` (`:806`) and runs a net report bounded by `NET_REPORT_TIMEOUT` = 10 s
  (`:816-823`; `iroh defaults.rs:129`). Report limits: overall 5 s, probes 3 s, DNS 3 s, captive-portal delay 200 ms and
  timeout 2 s (`net_report/defaults.rs:14,17,23,30,36,38`). `Endpoint` doc: "continuously runs net reports to monitor if
  network conditions have changed" (`endpoint.rs:1402-1403`).
- Full vs incremental: `FULL_REPORT_INTERVAL` = 5 min (`net_report.rs:132`); a report is full on a major change, the
  first report, or more than 5 min after the last full one (`:303-305`).
  - Full: existing QAD connections are closed and re-created (`:455-458`); HTTPS probe plan with 3 attempts per relay at
    200/300/400 ms (`probes.rs:15-18,98-112`) - each probe builds a fresh `reqwest` client, resolves the relay and sends
    `GET <relay>/ping` (`reportgen.rs:815-884`; `iroh-relay http.rs:15`); captive-portal check `GET /generate_204`
    (default on) after 200 ms, cancelled when UDP probes succeeded (`reportgen.rs:253-260,277-344`; `net_report.rs:102-110`).
  - Incremental: reuses a live QAD connection; probe plan is empty when the last report had relay latencies
    (`probes.rs:117-128`); QAD probes (up to 5 relays, each bounded by 3 s) are started again whenever no live QAD
    connection exists (`net_report.rs:475-491,496-519`).
- QAD connection: QUIC to the relay's QUIC port, default UDP 7842 (`iroh-relay defaults.rs:7`; `relay_map.rs:272-283,304-307`;
  `RelayMode::custom` builds `RelayConfig::from(RelayUrl)` per URL, `endpoint.rs:1960-1963`). It is kept with
  `keep_alive_interval` 25 s and `max_idle_timeout` 35 s (`iroh-relay quic.rs:296-300`). QAD runs only when the endpoint
  has IP transports (`socket.rs:1041-1052`).
- Link changes (netmon): major change -> rebind, relay connection check, `re_stun(LinkChangeMajor)`; minor ->
  `re_stun(LinkChangeMinor)` (`socket.rs:1661-1696`); QUIC notification waits for a default route with backoff 100 ms ..
  1 s, max 5 s (`:1416-1440,1511-1517,1616-1635`). Docs: some systems "like android do not expose this functionality to
  native code"; `Endpoint::network_change()` exists for the host to call (`endpoint.rs:1629-1647`). The network monitor
  itself comes from `netwatch` (not read).

### 3.3 Path / holepunch timers (per remote with an established connection)

- `HEARTBEAT_INTERVAL` 5 s ("If a path is idle for this long, a PING frame will be sent"), `PATH_MAX_IDLE_TIMEOUT` 15 s,
  `RELAY_PATH_MAX_IDLE_TIMEOUT` 30 s, `MAX_MULTIPATH_PATHS` 8 (`socket.rs:105-137`).
- Per-remote actor: `HOLEPUNCH_ATTEMPTS_INTERVAL` 5 s, `GOOD_ENOUGH_LATENCY` 10 ms, `UPGRADE_INTERVAL` 60 s
  (`check_connections` ticker), `ACTOR_MAX_IDLE_TIMEOUT` 60 s (`remote_state.rs:47-73,249-253,320-322`).
  `check_connections` triggers hole punching only when an existing connection has no IP path with RTT <= 10 ms; with no
  connection it does nothing (`remote_state.rs:751-783`; idle test `:346-350`).
- Other `Actor` timers: `remote_map.cleanup()` branch (`socket.rs:1615`); no further fixed-interval timers in the
  read parts of the main `select!` (`:1519-1639`).

### 3.4 QUIC defaults (noq-proto) and iroh overrides

- noq-proto `TransportConfig::default()`: `max_idle_timeout` 30 s ("Defaults to 30 seconds"), `keep_alive_interval` `None`
  ("`None` to disable, which is the default. Only one side of any given connection needs keep-alive enabled"),
  `default_path_max_idle_timeout` / `default_path_keep_alive_interval` `None`, `initial_rtt` 333 ms
  (`noq-proto config/transport.rs:96-97,279-281,556,564,573,589-590`).
- iroh builder defaults: `keep_alive_interval` 5 s, `default_path_keep_alive_interval` 5 s,
  `default_path_max_idle_timeout` 15 s, 8 multipath paths (`iroh endpoint/quic.rs:151-163`). Setter limits:
  `default_path_keep_alive_interval` ignores values above 5 s (`:505-523`), `default_path_max_idle_timeout` clamps to 15 s
  (`:484-503`), `keep_alive_interval` has no clamp (`:358-368`).
- This code overrides `max_idle_timeout` to 20 s and sets both keep-alive intervals to 5 s (`keepalive.rs:101-118`).

### 3.5 Builder APIs that change these

- `Builder::net_report_config(NetReportConfig::minimal())` disables HTTPS probes and the captive-portal check
  (`endpoint.rs:791-801`; `net_report.rs:82-130`); default is both enabled. Doc: "Disabling them ... will completely
  prevent finding the home relay on networks that do block QUIC" (`net_report.rs:97-99`).
- `Builder::portmapper_config(PortmapperConfig::Disabled)` (`endpoint.rs:781-789`; `portmapper.rs:10-39`); default
  `Enabled`; the real portmapper exists only if iroh's `portmapper` feature is compiled (`portmapper.rs:41-50`; feature
  state not read).
- `Builder::relay_mode(RelayMode::Disabled)` / `clear_relay_transports()` (`endpoint.rs:509-514,542-577`): empty relay map
  -> no net report runs (`socket.rs:799-803`).
- `Builder::clear_ip_transports()` (`endpoint.rs:501-507`): no QAD (`socket.rs:1041-1052`).
- `Endpoint::network_change()` (`endpoint.rs:1629-1647`), `insert_relay` / `remove_relay` (`:977-1001`).
- No public setter for the relay ping interval (`actor.rs:71` private), the re-stun interval (`socket.rs:2004-2019`) or
  `FULL_REPORT_INTERVAL` (`net_report.rs:132` private).

---

## 4. Remote servers and long-lived connections of an idle host

| Peer | Transport | Kept alive by | Source |
|---|---|---|---|
| Share server (signal channel) | TCP JSON lines (`tcp://host[:51820]` or bare `host[:port]`) or WebSocket `ws://` / `wss://` (`http(s)://` rewritten to `ws(s)://`), TLS through `tungstenite::client_tls` | app-level `Heartbeat` 20 s, presence 60 s, 500 ms read poll | `signal_connection.rs:49-107,188-215`; `signal_worker.rs:245-267` |
| Iroh home relay | WebSocket over TCP (TLS for https) to the relay URL | relay Ping/Pong 15 s | `iroh relay/actor.rs:20-23,532-568`; `iroh-relay client.rs:258-283` |
| Relay QAD endpoint | QUIC over UDP, default port 7842 | QUIC keep-alive 25 s, idle 35 s | `iroh-relay quic.rs:296-300`; `net_report.rs:451-491` |
| Paired peers (after a repair or a UI action) | QUIC over UDP (random local ports) and/or relay | QUIC keep-alive 5 s per connection and path, idle 20 s | `keepalive.rs:101-118`; `node_sessions.rs:233-315` |
| LAN | mDNS multicast UDP 5353 (224.0.0.251, ff02::fb) | mdns-sd timers (section 5) | `mdns-sd service_daemon.rs:72-74` |

- The Share server and the relay are separate connections. One signal endpoint is active at a time (first that connects);
  the relay URL(s) are derived from the same server string (`transport_options.rs:7-16`; `relay_urls_from_signal` not
  read). Without a server string: `RelayMode::Disabled`, no relay, no QAD, no net reports (`node.rs:93-99`;
  `socket.rs:799-803`); the signal worker sits in the 30 s offline loop (`signal_worker.rs:58-71`).
- No DNS/pkarr address lookup runs (`presets::Minimal`, section 2.7); peers are dialed only with the addresses in
  their signed presence or LAN evidence (`service.rs:212-251`; `endpoint.rs:601-602`: "If no Address Lookup is set,
  connecting to an endpoint without providing its direct addresses or relay URLs will fail").
- Is an incoming Direct connection possible with only the relay connection alive? Mechanically yes: a peer that holds our
  presence dials with `relay_url` from it; `online()` means "at least one relay server has completed its connection
  handshake (i.e. the endpoint is registered and reachable via that relay)" (`endpoint.rs:1308-1312`); the relay
  actor reads from the relay TCP stream and hands datagrams to the socket (`actor.rs:20-23`); the accept loop then
  receives the QUIC handshake (`node_accept.rs:16`). Conditions visible in code: (a) a relay must be configured and its
  handshake complete (home relay needs a net report with `preferred_relay`, 3.1); (b) the peer must consider our
  presence current: presence is signed for 300 s (`signal_presence.rs:20`) and refreshed over the signal channel every
  60 s (`signal_worker.rs:252-267`); an expired presence makes the dialing side report "Direktgeraet ist nicht online
  (gespeicherte Presence ist abgelaufen)" (`service.rs:227-235`), unless fresh LAN evidence exists
  (`lan_presence_match::effective_presence`, not read). Dial-side address validation is in `session.rs::endpoint_addr`
  (not read; `peer_endpoint_source.rs:57-62`).
- LAN-only (no server): reachable only through LAN candidates (direct UDP) and mDNS evidence at the peer.

---

## 5. mdns-sd 0.11.5 and the LAN presence

Our use (`share/os/shared/lan_presence.rs`): `ServiceDaemon::new()` + `browse("_se-share._udp.local.")` + thread
`se-lan-presence` (`:15,41-74`); `announce` registers instance `se-<hashed id>` with TXT `v`, `id`, `p4`, `p6`, `up`, host
`<device name>.local.`, port = IPv4 port or IPv6 port, `enable_addr_auto()` (`:89-126`); `Drop` -> `withdraw` + `shutdown`
(`:137-142`). The owner is `LanRuntime` (2.5); `presence_enabled` defaults to `true` (`lan_settings.rs:8-9,27`).

mdns-sd timers:

- Daemon thread `mDNS_daemon`: `poller.wait(events, timeout)` where `timeout` = time to the earliest timer, `None` if no
  timer (`service_daemon.rs:497-513`); it wakes for every received mDNS packet on any bound interface
  (`:426-460`) and for commands via a loopback signal socket (`:140-193`).
- Interface re-enumeration every 30 s (`IP_CHECK_INTERVAL_MILLIS = 30_000`, `:491-493,603-606`; `check_ip_changes`
  `:1122-1157`: adds/removes per-interface sockets and updates `addr_auto` services).
- Announcement: on register, one unsolicited response, a second one 1 s later (RFC 6762 section 8.3,
  `RegisterResend`) (`:1193-1219,2236-2250`). No other periodic re-announcement in the read code. Record TTLs: host
  records 120 s, PTR/TXT 4500 s (`service_info.rs:15-17`).
- Query (browse): first query immediately; retransmission delay starts at 1 s and doubles each time up to 3600 s, i.e. a
  steady state of one PTR query per hour (`:238,2031-2068`).
- Cache refresh for browsed services: PTR/SRV/A/AAAA records are re-queried when they pass 80% of their TTL, then at 85,
  90, 95% (`refresh_active_services` `:2253-2291`; `dns_parser.rs:128`, refresh progression `:187-196`).
- Our own code does not poll the daemon thread; it drains the event channel with `try_iter` once per daemon tick (2 s)
  (`lan_runtime.rs:253-275`).

Android Wi-Fi `MulticastLock` (`system/HostMonitor.kt`):

- Created lazily by `WifiManager.createMulticastLock("smart-explorer-share")`, `setReferenceCounted(false)`
  (`HostMonitor.kt:40,145-150`).
- Acquired when `share.status.running` is `true`, released when it is `false` (`:134-158`); `running` = the service exists
  (`share_status.rs:28-36`; `ipc_host_commands.rs:118-119`).
- Re-evaluated at `HostMonitor.start` (`:70`) and on every `CoreEvent.Share` (`:69`); `HostMonitor.start` is called from
  `BackgroundController.onAppStart` (`BackgroundController.kt:50`), i.e. every process start, in all background modes. The
  only explicit release is the `running == false` path; there is no release on process end other than process death.
- Doc comment: "holds a Wi-Fi multicast lock while Share is online (LAN presence/discovery needs multicast)"
  (`HostMonitor.kt:32-36`).

---

## 6. How incoming Direct requests and connections reach the Android UI

### 6.1 Incoming Direct access request (pairing, via the signal channel)

1. Server -> host over the signal connection; the worker dispatches lines in `dispatch_server_line`
   (`signal_worker.rs:290-317`; implementation in `tracked_signal_dispatch.rs`, not read). The events the daemon consumes
   for requests are `ShareEvent::DirectSignal` (tracked) and `DirectAccessRequest` (legacy)
   (`ipc_host_events.rs:62-85,133-151`); that the dispatcher emits them is inferred from those handlers.
2. Daemon `drain_events` (every 2 s tick, also on every poller snapshot) queues them: `DirectSignal` ->
   `direct_event_queue::enqueue` and, in the same drain, `direct_event_schedule::process_tick` with `persist_all`;
   `DirectAccessRequest` -> `legacy_events::enqueue` and `persist_all` (`ipc_host_events.rs:62-85,133-151,381-439`;
   `ipc_host_commands.rs:97-115`). The durable result is the profile's request ledger.
3. `share-poller` takes `snapshot.profiles` (unless a recent local commit outranks it, `share_state.rs:136-147`), computes
   `open_incoming(profiles, now)` = incoming requests with `can_decide` (`share_status.rs:316-323`), compares with
   `seen_requests`, and emits `{"type":"shareRequest","count":<number of all open requests>}` when at least one id is new
   (`share_state.rs:330-336,351-353`). `seen_requests` is empty at process start, so open requests re-emit once per process.
   It also emits `{"type":"share"}` when the status JSON changed (`:337-349`).
4. Kotlin: `Core.events` (1 s pump) delivers `CoreEvent.ShareRequest` / `CoreEvent.Share` (`Core.kt:177-202`;
   `api.md:97-107`). Consumers in the read surface: `BackgroundService` -> notification (1.5, persistent mode only);
   `ShareScreen` -> `vm.reload()` (`ShareScreen.kt:83-87`); `HostMonitor` and `ExecHostNotifier` on `Share` events
   (`HostMonitor.kt:69`, `ExecHostNotifier.kt:60`). "While the UI is visible the tab badge shows new requests instead"
   (`BackgroundService.kt:138`; the badge code is not read).
5. Latency pieces visible in code: signal read poll 500 ms, daemon drain <= 2 s, then the poller interval: 300 ms (watch),
   5 s (foreground), 60 s (background). Nothing wakes the poller on arrival of a request (2.5).
6. Decision: `share.decide` -> `decide_direct_request`, then `reconfigure` (`share_requests.rs:43-85`).

### 6.2 Incoming QUIC connection from a paired peer

- Accept path: `endpoint.accept()` loop; refused when sharing is stopped; global limit 64 pending application handshakes,
  4 per remote endpoint; ALPN `smart-explorer/share-fs/3` or `EXEC_ALPN` (`node_accept.rs:13-74`; `node.rs:25-29`).
- `server.rs::handle_connection` (`server.rs:80-172`): first bidi stream within 20 s (`HANDSHAKE_TIMEOUT`, `:34,89-92`), a
  `PeerHello` with `protocol_version == 3` (`:93-116`), `authenticate_incoming_session(&hello, &remote_node, &node.auth)`
  (`:117-133`), `PeerHelloOk` (`:134-139`), event `ShareEvent::Status("Iroh-Session akzeptiert: <device_id> (<node>)")`
  (`:141-144`), then one task per bidi stream with `session.authorize(&auth)` per request (`:153-171,203`).
- No user prompt or Android callback exists on this path: the code contains no interaction step between the hello and
  `PeerHelloOk`; acceptance depends only on `authenticate_incoming_session` (`session.rs`, not read), i.e. on the hello's
  relation proof. The outgoing counterpart requires `access_state == Accepted` (`peer_endpoint_source.rs:88-90`).
- Visible effects of an accepted incoming session: the `Status` event is logged by the daemon (`ipc_host_events.rs:43`),
  forwarded as a UI event (`:316-318`), folded into `ShareStatus.notices` (max 20, `share_state.rs:82-91,119`), shown as
  hint lines on the Share page (`ShareScreen.kt:175`); it can flip the status JSON and thereby emit one `share` event. No
  `shareRequest` event and no notification is produced by an accepted file-system session.
- Exec connections (ALPN `EXEC_ALPN`) dispatch to `exec_server::handle_connection` (`node_accept.rs:71`; not read). The
  ongoing "<device> fuehrt einen Befehl aus" notification is driven by `share` events plus `share.execJobs`
  (`ExecHostNotifier.kt:1-69`); it is started only from `ShareScreen` in the read files (`ShareScreen.kt:73`).
- Connection errors are reported through `emit_connection_error` (`node_accept.rs:49-57`; `connection_events.rs` not read).

---

## Offene Punkte

Not determinable inside the allowed surface:

1. `ShareCmd::ConfigureProfiles` handling on a connected worker (`signal_commands.rs::run_connected_command`): whether the
   6 s reload leads to signal-channel traffic (presence publish) or only local work. `signal_worker.rs:209-211` resets the
   presence timer if a command reports `published`.
2. `DISCOVERY_LIST_REFRESH_INTERVAL` and `DISCOVERY_PUBLISH_ACK_TIMEOUT` values (`discovery_signal_state.rs`): interval of
   the idle `ListDiscoveries` message.
3. `tracked_signal_sender::send_pending_tracked` and `tracked_signal_dispatch::dispatch_server_line`: cost and traffic with
   an empty outbox every 2 s.
4. `session.rs`: `relay_urls_from_signal` (how relay URLs derive from the server string, whether the relay serves QAD on UDP
   7842), `authenticate_incoming_session` (exact authorization rule), `endpoint_addr` (presence expiry check on dialing).
5. `lan_presence_match.rs`: `LAN_PRESENCE_TTL_SECS` and `effective_presence` (LAN evidence lifetime, persistence cadence of
   `lan_seen_at` = TTL/2).
6. `direct_reciprocal_transport.rs::run_outgoing`: whether the reciprocal-repair connection is closed after the exchange (if
   not, each online accepted contact keeps a cached QUIC connection with 5 s keep-alives).
7. Server behavior: how often the signal server forwards `DirectAvailable` (each peer presence refresh or only changes);
   each one marks `changed` and writes the profile (`ipc_host_events.rs:86-119,338-366`); relay-server ping jitter
   (`iroh-relay server/client.rs:339`).
8. Cost per call on Android of `ShareIdentity::load_or_create`, `ShareProfiles::load_checked`,
   `exec_grant_journal::load_pending` (6 s reload), `autostart::is_enabled()` (several calls per 2 s slice),
   `crate::net::gather_interface_facts()` (5 s) and `uplink_adapter().probe()` (every 2 s tick).
9. Android build features of iroh: whether the `portmapper` feature is compiled (`native/Cargo.toml`), and behavior of the
   `netwatch` network monitor and of the `portmapper` crate on Android.
10. Defaults and callers not read: `AppPrefs.bgMode` default, `ShareProfiles.auto_connect` default, `InitConfig` `startDaemon`,
    `SmartExplorerApp`, `BootReceiver`, `SyncWorker`, `CoreEvent.kt`, the tab-badge code.
11. JNI glue (`native/android-bridge`, `mobile/mod.rs`): the exact mapping of `pollEvents` to `Hub::poll`.
12. `domains/share_exec.rs::watch_host_activity`, `provider()`, `host_activity()` (called on poller start and every poll);
    `exec_server`, `exec_registry` behavior of incoming exec connections.
13. Android operating-system behavior (Doze, App Standby buckets, background network restrictions, foreground-service type
    limits, Wi-Fi multicast filtering with the screen off, battery cost of a held `MulticastLock`) is not determinable from
    the code.
