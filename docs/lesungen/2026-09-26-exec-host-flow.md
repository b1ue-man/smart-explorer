# Exec-Host Flow (Remote Command Execution)

**Purpose.** Trace the complete data path of Smart Explorer's remote "exec" capability — how a
device becomes an exec **host**, how an incoming peer's exec request is authorized, admitted,
executed under process containment, and reported back — plus the desktop UI/IPC path that
enables/grants/revokes exec, so an Android exec-host facade can be planned against the real
contract instead of assumptions.

## Files read

- `native/src/share/core/exec.rs`, `exec_auth.rs`, `exec_grant_persistence.rs`,
  `exec_grant_runtime.rs`, `exec_job.rs`, `exec_platform.rs`, `exec_policy.rs`, `exec_protocol.rs`,
  `exec_registry.rs`, `exec_registry_view.rs`, `exec_server.rs`, `exec_session.rs`,
  `exec_supervisor_protocol.rs`, `exec_types.rs`, `exec_heartbeat.rs`, `service.rs`,
  `signal_commands.rs`, `types.rs` (`ExecRequest`/`ExecResult`/`ExecGrantTarget`/`ShareCmd`/
  `ShareCmdResult`/`ShareEvent` regions)
- `native/src/share/mod.rs` (module list, `platform_exec` cfg-selection, `exec_*` re-exports)
- `native/src/share/os/linux_os/exec.rs`, `exec_supervisor.rs`, `exec_systemd.rs`
- `native/src/share/os/android/exec.rs`
- `native/src/daemon/os/shared/exec_grant_journal.rs`, `exec_grant_journal_storage.rs`,
  `exec_ipc.rs`, `exec_state.rs`, `ipc_exec_grant_client.rs`, `ipc_host.rs`
- `native/src/daemon/os/shared/ipc_host_commands.rs` (holds `ShareHost::send_command`; declared
  outside `ipc_host.rs`'s own `#[path=...] mod` list, read because the brief's question about
  `send_command` refusing exec commands cannot be answered without it — confirmed via
  `grep -rn send_command native/src/daemon/os/shared/`)
- `native/src/app/core/share_exec_ui.rs`, `share_exec_jobs_ui.rs`
- `native/src/mobile/os/shared/domains/share_settings.rs`, `share_peers.rs`, `share_status.rs`,
  `share_requests.rs`
- `docs/superpowers/plans/2026-09-25-android-apk/api.md` §5 (line 288 onward)

---

## Findings

### 1. Data path: peer exec request → grant checks → `ContainedExec::prepare`/`commit` → back

**Wire handshake and first grant check (connection time).**
`exec_server.rs:26-96 handle_connection` opens a bidirectional QUIC stream, sends a fresh
`ExecServerHello{challenge,...}` (`exec_server.rs:48-60`), receives the peer's `ExecClientHello`
(`61-66`), and calls `exec_auth::authorize_client_hello` (`67-88`).
`exec_auth.rs:54-117 authorize_client_hello_in` is the **first grant check**:
- Direct peers (`relation_kind=="direct"`, `61-81`): finds the exact `DirectGrant` by
  `device_id+public_key+fingerprint+node_id` with `state==Accepted` (`66-76`), then requires
  `grant.exec.enabled && fingerprint_matches(...)` (`77-79`) before even checking the HMAC proof.
- Room members (`82-111`): finds the room by `room_id` with `auto_join`, the member by the same
  four-way identity pin with `!blocked` (`83-98`), then requires `member.exec.enabled &&
  fingerprint_matches(...)` (`99-101`).
- Both branches verify an HMAC proof over the handshake transcript bound to the relation secret
  (`80,106,113`) — the shared secret differs per direct-peer vs. per-room.

`DirectGrant`/`RoomMember` each carry a `.exec: ExecGrant` field (confirmed by field access, not by
reading the struct definitions, which are outside this reading's file list — see
`exec_auth.rs:70-77,91-99`, `exec_grant_persistence.rs:160-184,202-226`,
`exec_grant_runtime.rs:177-199,201-242`). `ExecGrant` itself
(`exec_policy.rs:11-23`) holds `enabled:bool, policy_revision:u64, changed_at:i64,
source_request_id:Option<DirectRequestId>, source_decision_revision:Option<u64>` — a monotonic
revision per exact pinned identity, doc-commented "File access never implies execution access"
(`exec_policy.rs:6-10`).

**Second grant check (registry seeding + Start frame).** After auth succeeds,
`exec_server.rs:89-96` calls `ExecRegistry::apply_authorization` (`exec_registry.rs:154-190`) to
seed/refresh the in-memory `(policy_revision, enabled)` cache for that principal, cancelling any
running job for a principal whose epoch just advanced or whose policy was disabled
(`165-168,182-188`). The server then reads the first client frame, which must be `Start`
(`exec_server.rs:130-137`), and calls `ExecRegistry::prepare` (`exec_registry.rs:192-266`) — the
**third, independent** check: `policy.1` (enabled) and `policy.0`/epoch must still match the
authorization captured at handshake time (`207-217`), else `StaleAuthorization`. This is what makes
a mid-handshake revoke fail closed even though the HMAC/identity check already passed.

**Fourth check (commit-time barrier).** `serve_job` (`exec_server.rs:194-361`) spawns
`run_contained_job` on a blocking task (`207-209`, `exec_job.rs:43-202`), which calls
`ContainedExec::prepare(&start)` (`exec_job.rs:50`, `exec_platform.rs:32-37`) and then
`ExecRegistry::commit_start` (`exec_job.rs:54-58`, `exec_registry.rs:268-302`) — this **re-checks**
`policy.1`/`policy.0`/epoch a fourth time (`281-284`) right before flipping the job to `Running`
(`300`), specifically so a grant flip that lands while the OS-level `prepare()` was still allocating
resources still prevents the process from ever being started (comment at
`exec_grant_runtime.rs:46-51`: "Registry mutation happens while new handshakes are excluded by the
auth lock... this establishes the deny barrier and cancellation before the authoritative auth
snapshot is changed").

**Back to the peer.** `run_contained_job` forwards `SupervisorEvent::Started/Stdout/Stderr` as
`ServerFrame::Started/Stdout/Stderr` (`exec_job.rs:119-155`), and on exit calls
`ExecRegistry::record_terminal` (`exec_job.rs:279-286`, gated as in §3) to produce
`ServerFrame::Terminal`, sent by `serve_job`'s frame-forwarding loop (`exec_server.rs:319-343`), then
waits for the peer's `ResultAck` before closing (`exec_server.rs:336-343,459-478`).

**Caveat — `core/exec.rs` looks unused.** `exec.rs` defines its own `PreparedExec`/`ExecPermit`
fail-closed stub (global/per-peer semaphores, always denies with `PermissionDenied` unless
`grant.enabled`, and `run()` always fails with `Unsupported`, `exec.rs:66-99`). `mod.rs:114-115`
declares `mod exec;` but re-exports nothing from it, and none of `exec_job.rs`, `exec_server.rs`,
`exec_registry.rs`, or `exec_auth.rs` reference `super::exec::` — the active path uses
`exec_platform::ContainedExec` (§3) instead. This module reads as vestigial/superseded; nothing in
the files read here calls into it.

### 2. Desktop enable/grant/revoke UI, and why `send_command` refuses exec mutations

**UI → command.** `app/core/share_exec_ui.rs:24-62 ui_exec_grants` renders one card per exact
direct-grant/room-member from the already-loaded `app.share_profiles` (`exec_device_views`,
`239-291`). Enabling is a deliberate two-step "arm" flow: click "Remote-Ausführung aktivieren…"
arms a per-view flag (`activation_controls`, `112-132`), which reveals a danger warning
(`exec_warning`, `301-312`) and a mandatory confirmation checkbox; only once `understood &&
provider.available && base_authorized` (`activation_ready`, `176-182`) does "Exec jetzt aktivieren"
enable, setting `action = Some((target, true))` (`167`). Disabling has no confirmation step
(`100-104`). Either action calls `apply_exec_grant(app, target, enabled)` (`210-237`), which calls
`crate::daemon::mutate_exec_grant(target, enabled)` (`211`) — the client side of this is
`daemon/os/shared/ipc_exec_grant_client.rs:9-33`, sending `IpcRequest::MutateExecGrant{token,
target, enabled}` over the daemon's local TCP IPC.

**Daemon: journal-backed CAS mutation.** `ShareHost::mutate_exec_grant`
(`exec_grant_journal.rs:246-296`) is the single entry point:
1. Takes `exec_grant_lock` (`252-255`) so concurrent mutations serialize.
2. `reload_now_locked()` (`256`) refreshes state, then if a journal entry is **already** pending,
   returns that pending retry state instead of starting a second one (`257-265`).
3. `ExecGrantMutation::prepare_persisted` (`exec_grant_persistence.rs:9-31`) computes the new
   `ExecGrant` (advanced `policy_revision`) against the current in-memory profile, capturing
   `expected_revision` for a compare-and-swap.
4. `JournalEntry::new` + `write_entry(&entry)` (`exec_grant_journal.rs:285-286`,
   `exec_grant_journal_storage.rs:5-37`) durably fsyncs a crash-recovery record **before** anything
   is applied to disk-profile or live worker.
5. `execute_locked` → `drive_steps` (`exec_grant_journal.rs:298-338,404-443`) applies the two steps
   in an order that depends on direction: **enable** (`PendingApply`) persists to the on-disk
   profile first, then applies to the live `ExecRegistry` (`417-423`) — so a crash between the two
   recovers as "not yet live" rather than silently active; **disable** (`PendingDeny`) applies to
   the live registry first (installing the deny barrier / cancelling running jobs immediately), then
   persists (`425-431`) — so a crash mid-disable still leaves the runtime already revoked even if the
   disk write hasn't landed yet.
6. `should_fail_closed` (`387-389`): if an **enable** applied live but the journal entry could not be
   durably cleared, the whole Share worker is suspended and stopped (`341-356`) rather than leaving
   an unconfirmed exec grant running.
7. `clear_entry` (`exec_grant_journal_storage.rs:39-64`) removes the journal file and fsyncs its
   parent directory; if that fsync itself fails, `unlink_and_sync_with_recovery` rewrites the
   *identical* entry back and verifies the round trip (`66-102`), so a crash right after still
   recovers the same pending operation instead of losing it.

**Why `send_command` refuses these commands.** `ipc_host_commands.rs:13-22
ShareHost::send_command` explicitly matches `EnableExec | DisableExec | ApplyExecGrant |
ConfigureProfiles` and returns `Err("Dieser Share-Befehl erfordert eine dauerhafte
Daemon-Mutation")`. The reason: `send_command`'s normal path (`23-84`) only forwards a `ShareCmd` to
the running `ShareService` via `service.cmd(cmd)` (`65`), which — for exec — is
`ShareService::enable_exec`/`disable_exec`/`apply_persisted_exec_grant` (`service.rs:74-99`) →
`signal_commands::mutate_exec_grant`/`apply_persisted_exec_grant` (`signal_commands.rs:387-425`) →
only mutates the **live** in-memory `ShareAuthState` + `ExecRegistry`
(`exec_grant_runtime.rs:25-60,82-120`); there is no disk persistence and no crash-recovery journal
on that path. Routing an exec grant change through plain `send_command`/`ShareCmd` would risk a
crash between the live apply and any later persistence attempt silently losing the grant's durable
state (or, for a disable, leaving a stale "enabled" record on disk after restart). `send_command`
therefore force-routes these three mutations to `ShareHost::mutate_exec_grant`'s
journal+CAS path instead, which is the only path in scope that guarantees persisted-and-applied
atomicity across a crash.

### 3. `ContainedExec` contract

`exec_platform.rs:26-79` is the cross-platform wrapper; `mod.rs:238-246` selects the concrete
`platform_exec` module per target (`android → os/android/exec.rs`, `linux → os/linux_os/exec.rs`,
`windows → os/windows/exec.rs` — not read here).

**Methods** (`exec_platform.rs`):
- `prepare(&ExecStart) -> io::Result<Self>` (`32-37`): allocates OS resources before the request is
  committed (Linux: cgroup v2 check + systemd D-Bus connection + Unix socket bind,
  `linux_os/exec.rs:52-167`).
- `commit(&mut self, request: ExecStart) -> io::Result<()>` (`39-55`): re-validates
  `request.validate()`, refuses a double commit (`AlreadyExists`, `41-45`), computes
  `environment_for(request)` (`exec_supervisor_protocol.rs:52-62` — strips inherited
  `SMART_EXPLORER_*` vars before applying caller overrides), then `inner.configure` + `inner.send
  (Start)`. On Android, `configure`/`send` both `match self.never {}` since `ContainedExec` is
  uninhabited (`os/android/exec.rs:18-45`).
- `write_stdin`/`close_stdin` (`57-66`): forward `Stdin`/`StdinEof`, chunked at
  `MAX_EXEC_DATA_BYTES` (64 KiB).
- `next_event(deadline) -> io::Result<SupervisorEvent>` (`68-70`).
- `terminate_all(reason: StopReason) -> io::Result<()>` (`72-74`): on Linux, idempotent
  (`AtomicBool`, `linux_os/exec.rs:225-232`) systemd `StopUnit` with `KillMode=control-group` +
  `SendSIGKILL=true` (`exec_systemd.rs:56-58,75-83`) — kills the **whole cgroup**, not just the
  immediate child.
- `confirm_empty(deadline) -> io::Result<()>` (`76-78`): on Linux, polls every 20 ms until
  `!cgroup_populated(cgroup) && unit_active_state ∈ {inactive, failed}`
  (`linux_os/exec.rs:191-207`, `exec_systemd.rs:130-183`), else `TimedOut`.
- `provider_status()`/`run_supervisor_if_requested` (`81-87`): OS facts, not instance methods.

**Event order guarantee.** `exec_job.rs:43-202 run_contained_job` and the debug self-test
(`exec_platform.rs:89-144`) fix the order: `Started{pid}` first (`119`) → interleaved
`Stdout`/`Stderr` → `RootExited(exit)` when the top-level child exits, opening a
`ROOT_OUTPUT_GRACE=250ms` window (`exec_job.rs:15,84-86,156-164`) to drain output any grandchildren
still have buffered → `Exited(exit)` is the true terminal event; the self-test hard-asserts `Exited`
never overtakes stdout/stderr/`RootExited` (`exec_platform.rs:121-126`, error text "Exited overtook
stdout/stderr or RootExited"). An `Error(message)` can replace any of these and forces an immediate
stop.

**What `terminate_all`/`confirm_empty` together must guarantee.**
`ExecRegistry::record_terminal` (`exec_registry.rs:360-413`) **refuses** to record a terminal result
— and therefore `ServerFrame::Terminal` can never be sent, `exec_job.rs:271-287 finish_job` — unless
`containment_confirmed_empty == true` (`367-369`, else `ContainmentNotConfirmed`). The contract: no
terminal result is ever reported, and no exec slot is ever freed for reuse, until the OS layer has
*positively verified* every descendant process is gone. `exec_job.rs:204-239 stop_and_finish` always
calls `terminate_all` then `confirm_empty` before `finish_job`, on every exit path (normal exit, I/O
error, and even the "commit failed" branch at `exec_job.rs:59-61`).

**`provider_status` feed and reaction to unavailability.** `ExecProviderStatus{available, provider,
detail, elevated, user_label}` (`exec_types.rs:199-206`) is produced fresh per connection in a
blocking task right after hello auth (`exec_server.rs:97-103`), embedded in `ExecHelloOk.provider`
sent to the client (`104-115`), and also cached client-side per UI refresh
(`share_exec_ui.rs:184-193`) to drive the desktop's own "Kapselung: …" label and the enable button's
gate. **Reaction**: immediately after sending `ExecHelloOk` (still carrying the true provider
status), if `!provider.available` the server finishes/flushes the send stream and blocks on
connection close (bounded by `EXEC_HEARTBEAT_POLICY.server_result_ack_timeout()`), then returns
`Err(Unsupported: "{provider}: {detail}")` (`exec_server.rs:117-128`) — it **never reads a Start
frame** in this case, so `run_contained_job`/`ContainedExec::prepare` is never reached when the
provider is unavailable (e.g. today on Android, or on a Linux host lacking cgroup v2/user systemd).

### 4. Linux supervisor: Linux/systemd-specific vs. reusable in-process

**Linux/systemd-specific** (`linux_os/exec.rs` + `exec_systemd.rs`, talks to
`org.freedesktop.systemd1` over D-Bus, requires cgroup v2, re-execs the current binary as a new
OS process supervised by systemd):
- `ContainedExec::prepare` (`linux_os/exec.rs:52-167`): `require_cgroup_v2`
  (`exec_systemd.rs:185-194`), `manager_connection()` (`85-95`), binds a Unix socket under
  `/run/user/<uid>/smart-explorer-exec/` (mode 0700, ownership-checked,
  `linux_os/exec.rs:350-388`), calls `StartTransientUnit` (`exec_systemd.rs:34-73`) to launch
  `/proc/<pid>/exe --share-exec-supervisor <socket>` as a new `.service` unit with
  `KillMode=control-group`, `SendSIGKILL=true`, `RuntimeMaxUSec` bound to the request timeout+30s,
  then `accept()`s that process's connection and verifies its `SO_PEERCRED` uid/gid match this
  process's own euid/egid (`linux_os/exec.rs:78-85,331-348`) before trusting it.
- `terminate()` → systemd `StopUnit` (`exec_systemd.rs:75-83`) and `confirm_empty()`'s
  `cgroup.events`/unit-`ActiveState` polling (`linux_os/exec.rs:191-207`,
  `exec_systemd.rs:130-183,176-183`) — this is exactly the process-tree-containment guarantee
  `core/exec.rs`'s own comment says was previously missing ("Until both supported platforms have a
  typed adapter that guarantees descendant containment and teardown on timeout/disconnect, fail
  closed", `exec.rs:91-98`).
- `provider_status()` availability check (`linux_os/exec.rs:256-275`) and
  `run_supervisor_if_requested` recognizing the internal re-exec mode
  (`linux_os/exec.rs:277-304`, true only when `argv[0]=="--share-exec-supervisor"`).

**Reusable, OS-agnostic** (`exec_supervisor.rs` — the code that *runs inside* the spawned process;
touches only a plain `std::os::unix::net::UnixStream` and `std::process::Command`; no
systemd/D-Bus/cgroup call anywhere in the file):
- `run(stream: UnixStream)` (`exec_supervisor.rs:13-94`): reads exactly one `Start` command
  (`14-17`), `spawn()`s the argv/shell command with piped stdio, `env_clear()` + the supplied
  environment map (`96-131`), sends `Started{pid}`.
- Then loops: a second thread forwards `Stdin`/`StdinEof`/`Cancel`; two more threads drain
  stdout/stderr, each enforcing the shared `max_output_bytes` budget via an atomic counter
  (`reserve_output`, `175-193`) and setting a shared "truncated" flag; on child exit it sends
  `RootExited` then, after joining the output threads, `Exited` with the real truncation flag
  folded in (`63-75`).
- None of this depends on being launched by systemd or living in its own cgroup. This is exactly
  what the brief calls "reusable in-process over a `UnixStream` pair": swapping the systemd-unit
  launch for e.g. `UnixStream::pair()` and running `exec_supervisor::run(one_end)` on a spawned
  thread in the *same* process would work unchanged — at the cost of losing the systemd/cgroup
  containment guarantee (whole-descendant-tree `SIGKILL`, and `confirm_empty`'s OS-verified "empty"
  check) that only the systemd unit + cgroup boundary currently provides. An in-process thread can
  `Command::kill()` the immediate child (SIGKILL to that one PID) but has no OS-verified way to know
  a detached grandchild is actually gone.

### 5. What an Android exec-host facade would need to add

**Documented and coded gap.** `docs/superpowers/plans/2026-09-25-android-apk/api.md:338`: "Nicht
auf Android: Exec-Freigaben für dieses Gerät (Exec-Host), LAN-Uplink, Anfragen im Altformat." —
Android is explicitly scoped as exec-**client**-only today. In code:
`os/android/exec.rs:18-20` makes `ContainedExec` `Infallible`-backed/uninhabited; `prepare()`
always returns `Err(Unsupported)` (`23-25`); `provider_status()` always reports
`available:false, provider:"Android", detail:"Entfernte Ausführung ist unter Android nicht
verfügbar"` (`48-56`). Because `exec_server.rs:117-128` closes the connection right after sending
this provider status and *before* reading a Start frame, an Android exec host today authenticates a
peer, reports "unsupported", and stops — it never reaches grant checks, `ExecRegistry::prepare`, or
`ContainedExec` at all.

**Mobile facade currently has no host-side surface.** `share_settings.rs` exposes
`watch/set_server/set_online/set_name/discoverable/stop_discoverable/discover/connect/
cancel_connect` — no enable/disable-exec method. `share_status.rs:241-308 status_json` builds the
full desktop-parity `ShareStatus` JSON but has no `execGrants`/`execJobs` field, and
`device_json`/`rooms_json` (`84-98,100-133`) carry no per-device `exec` block the way the desktop's
`ExecDeviceView` (`share_exec_ui.rs:5-16`) does. `share_requests.rs:155-225 exec` only exposes the
**client**-side one-shot `share.exec {location,command,shell,timeoutSecs}` → `daemon::exec_share`
(outbound direction only; api.md §5 documents this as `share.exec`, line ~314-317).

**To reach desktop parity, an Android exec-host facade needs, at minimum:**
1. A real Android `ContainedExec` provider (replacing the uninhabited stub in
   `os/android/exec.rs`) giving the same containment guarantee `confirm_empty`/`terminate_all`
   rely on for Linux (§3/§4) — Android has no user systemd/D-Bus, so this needs its own
   OS-appropriate containment primitive before `provider_status().available` could honestly become
   `true`. The generic `exec_supervisor.rs` process-management logic (§4) is reusable as-is once
   paired with such a primitive, e.g. over an in-process `UnixStream::pair()`.
2. New `ShareCmd`/mobile-API methods mirroring the desktop's `enable_exec`/`disable_exec`/
   `apply_persisted_exec_grant` (`service.rs:74-99`), routed through the **same** journal-backed
   `ShareHost::mutate_exec_grant` (`exec_grant_journal.rs:246-296`, §2) — e.g. `share.enableExec
   {target}` / `share.disableExec {target}` calling the equivalent of
   `ipc_exec_grant_client::mutate_exec_grant` (`ipc_exec_grant_client.rs:9-33`), which is already
   transport-agnostic Rust, not desktop-egui code, and needs only a mobile API entry point.
3. New status fields on `ShareStatus`/`share.status`: per-direct-device and per-room-member
   `exec:{enabled, policyRevision, baseAuthorized}` (mirroring `ExecDeviceView`,
   `share_exec_ui.rs:5-16`) plus a `provider:{available, provider, detail, elevated, userLabel}`
   block (`ExecProviderStatus`, `exec_types.rs:199-206`) so the app can warn before an enable
   attempt the way `activation_ready`/`exec_warning` do (`share_exec_ui.rs:176-182,301-312`).
4. A jobs surface mirroring `daemon::exec_jobs()`/`ExecJobsSnapshot`
   (`exec_state.rs:6-12,184-200`) and `daemon::cancel_exec`/`ExecCancelTarget`
   (`exec_state.rs:21-26,202-225`) — i.e. `share.execJobs {}` returning incoming/outgoing
   active+history `ExecJobView`s, and `share.cancelExecJob {direction, execId, peerDeviceId}`,
   matching `share_exec_jobs_ui.rs`'s poll-and-cancel pattern (`24-97,245-292`) but as mobile API
   calls instead of an egui panel.
5. Confirmation-of-danger UX equivalent to the desktop's two-step "arm → explicit checkbox →
   enable" flow (`share_exec_ui.rs activation_controls, 112-174`) — the grant is deliberately an
   irreversible-until-explicit-revoke, full-machine-code-execution authorization
   (`exec_policy.rs:6-10`), and Android has no analogous safeguard yet.

---

## Open questions

- Windows's `ContainedExec` (`os/windows/exec.rs`) was outside the brief's file list; whether it
  offers the same containment guarantee via a Job Object or similar is unconfirmed here.
- `exec_client.rs`/`exec_client_active.rs`/`exec_frame_reader.rs` (the outgoing/client side of the
  exec protocol, used by `exec_session.rs`) were outside the brief's file list; this reading covers
  the host/server side and the desktop grant UI only.
- Whether `core/exec.rs` is truly dead code or still reachable from a file outside this reading's
  scope (e.g. a test harness or CLI path) was not verified beyond grep-free inspection of the files
  read; recommend a repo-wide reference check before deleting it.
