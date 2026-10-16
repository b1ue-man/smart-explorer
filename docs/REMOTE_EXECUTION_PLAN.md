# Secure peer remote execution — implementation and validation record

Status: **implemented for 0.5.134**. Exec remains fail-closed until one exact
device grant is explicitly enabled and the host containment provider is
available.

Research date: **2026-07-12**. Local installed-Linux validation: **2026-07-13**.

This plan enables command execution over Smart Explorer Share. By default an
explicitly authorized peer gets the same practical authority as the OS account
running the Smart Explorer worker (the "Standardnutzer"). Since 2026-10-16 a
grant can narrow that: on a Linux host whose worker runs as root it can name
another local account (`run_as`), and on every host it can list the only
programs the peer may start (`programs`; argv mode only, no shell, no
terminal). There is still no path allowlist, shell grammar filter, restricted
token, seccomp profile, or application sandbox. Plan and evidence of the
2026-10-16 changes: `docs/plaene/2026-10-16-luecken-schliessen/plan.md`.

That trust decision changes where the security boundary belongs. The command is
trusted. The internet, LAN, signaling service, relay, unauthenticated Iroh
endpoints, stale grants, and malformed protocol input are not. Every control
that decides whether a process may start therefore has to complete before any
command-controlled code runs.

## 1. Decision summary

The implementation is accepted only when all of these are true:

1. Exec permission is a separate, default-deny permission on one exact pinned
   device identity. Accepting file access does not grant Exec.
2. Exec uses a new Iroh ALPN and protocol. The existing filesystem v3 ALPN
   remains compatible, but it can never carry an enabled Exec request.
3. A fresh server challenge, Iroh's mutually authenticated endpoint identity,
   the relation proof, the local grant revision, and the current authorization
   epoch all agree before process start.
4. Authorization and process admission have no revoke/start race. A process is
   first created behind a platform launch barrier and is released only while
   its authorization lease is still current.
5. stdout, stderr, stdin, cancellation, and the final status are streamed with
   bounded in-memory queues. Output is not serialized into one JSON value.
6. Disconnect, timeout, explicit cancellation, permission removal, worker
   shutdown, and worker crash terminate the contained process tree.
7. Windows uses Job Objects. Linux uses a transient systemd service/cgroup. No
   weaker process-group-only fallback is permitted.
8. A successful terminal result is sent only after the containment provider has
   confirmed that no contained processes remain.
9. The CLI is self-discovering: missing selectors show the available choices,
   selectors have live completion, and both peers can inspect active and recent
   execution state.
10. Release validation exercises installed Linux and Windows artifacts through
    isolated same-platform native gates, using only values discovered by earlier
    CLI commands in the same test run. Published Windows-to-Linux certification
    remains the separate M11 gate.

## 2. Explicit trust contract

Enabling Exec for a device is equivalent to giving that device an interactive
shell as the Smart Explorer worker's user. It can read and modify everything
that account can, including credentials, startup files, Smart Explorer state,
and unrelated user data. It can install persistence using facilities already
available to that user. Revoking the grant prevents future commands and stops
contained active commands; it cannot undo actions already performed.

For the Standardnutzer there is no privilege transition:

- a normal user worker starts a normal user process;
- an elevated Windows worker starts an elevated process;
- a root Linux worker starts a root process.

A grant with `run_as = account:<name>` on a Linux host whose worker runs as
root starts the job's root process as that account: the root supervisor sets
the account's supplementary groups, group and user before `exec`, enters the
working directory with the account's rights, hands a terminal's PTY to the
account (mode 0620) and gives the job a clean environment (HOME, USER,
LOGNAME, SHELL, PATH, locale, plus the caller's values) instead of root's.
Windows hosts (another account needs its password or a SYSTEM service),
Android hosts (one app UID) and Linux hosts with a non-root worker refuse
such a grant. A grant with `programs` admits only argv starts of the listed
programs, found only through a fixed search path and with only locale and
terminal variables from the caller (§6); shell commands and terminals are
refused.

The program list is not a sandbox. It names a program, not what that program
does with its free arguments: a shell, `env`, an interpreter (`python`, `perl`,
`node`) and any program that runs other commands (`find`, `git`, `ssh`,
`xargs`, `awk`, `sudo`) equals a full shell and does not belong in the list.
The CLI help of `se share grants exec enable` says so. What the list closes
is the start itself: the caller can neither smuggle in another program through
`PATH`, `LD_PRELOAD` or a program's own variables (`GIT_PAGER`, ...) nor start
a shell or a terminal.

The grant UI and CLI must say `FULL <user> CODE EXECUTION` and, when applicable,
`REMOTE ADMINISTRATOR SHELL` or `REMOTE ROOT SHELL`. This warning happens once
when changing the permission. Commands are not individually confirmed
afterward; a program list filters them by program only.

The containment contract covers processes normally created by the command and
its descendants. Trusted code can intentionally ask an independent broker such
as Task Scheduler, WMI, a service manager, Docker, or `systemd-run` to start
unrelated work. Preventing that would require a sandbox or reduced account and
would contradict this feature's unrestricted-user-shell contract.

## 3. Pre-implementation audit (resolved in 0.5.134)

The pre-0.5.134 data plane was a sound base:

- Iroh authenticates the remote EndpointId as its TLS public-key identity and
  encrypts traffic end to end, including over a relay.
- Smart Explorer additionally pins device ID, public key, fingerprint, NodeId,
  relation, and an HMAC relation proof in
  `native/src/share/core/session.rs`.
- `IncomingSession::authorize` rechecks the current grant and export policy on
  every accepted stream instead of treating a cached QUIC connection as an
  authorization cache.
- policy changes close cached incoming and outgoing sessions in
  `native/src/share/core/node.rs`.
- direct requests and decisions are signed, revisioned, expiring, and tracked
  durably.
- current Exec submission is deliberately at-most-once; the caller does not
  retry after an ambiguous transport failure.

The following gaps blocked the old batch stub. 0.5.134 resolves them with the
modules and validation gates recorded below rather than enabling that stub:

- `ShareExportConfig.allow_exec` applies to every accepted direct peer, or to
  every member of a room. It is not a per-device permission.
- protocol v3 already deserializes `Ctrl::Exec`, while
  `requested_capabilities` is ignored. There is no secure negotiation boundary.
- authorization is checked, cloned, unlocked, and only then handed to
  `spawn_blocking`; a revoke can race the start.
- session invalidation does not cancel a command already handed to the blocking
  worker.
- the server cannot observe peer disconnect while it synchronously waits for
  `PreparedExec::run`.
- `ExecResult` buffers two byte vectors, encodes them in a JSON control frame,
  and encodes them again through the 2 MiB local IPC line protocol.
- the Iroh accept loop has no application-level pre-auth connection budget, and
  the generic control decoder may allocate a 16 MiB frame before application
  authorization.
- the existing 30-second default and 15-minute clamp describe a batch helper,
  not an unrestricted user shell.

## 4. Authorization model

### 4.1 Profile schema

Replace the inactive global Exec flag with an explicit local policy attached to
the exact principal:

```rust
struct ExecGrant {
    enabled: bool,
    policy_revision: u64,
    changed_at: i64,
    source_request_id: Option<DirectRequestId>,
    source_decision_revision: Option<u64>,
}
```

`DirectGrant` and `RoomMember` each receive an `exec: ExecGrant`. The identity
covered by that grant is the complete tuple `(relation, device_id, public_key,
fingerprint, node_id)`, not a display name or short selector.

Since 2026-10-16 `ExecGrant` also carries `run_as` (`{"kind":"standard"}` or
`{"kind":"account","name":…}`) and `programs` (absent = full shell). Profiles
written before have neither field and read as the Standardnutzer with the
full shell, so every existing grant keeps exactly its rights, its `enabled`
state and its `policy_revision` (no running session is revoked by the
update); the next profile write stores `run_as` explicitly. Changing the
account or the program list is a policy change: it advances the revision and
ends the device's running jobs like any revocation. Set with
`se share grants exec enable [--run-as USER|standard] [--allow-program PATH …|--full]`,
on the desktop under „Geräte & Freigaben → Befehle → Einstellungen“ (account
field on a Linux root worker; „Programmliste bearbeiten“ with the rules of
`--allow-program`, an empty list is the full shell, every change of an enabled
grant needs a confirmation; `app/core/share_exec_programs_ui.rs`) and on Android
under „Teilen → Befehle auf diesem Telefon → ⋮ Programme…“ (`share.setExec`
with `programs`; Android always sends the Standardnutzer). Both send the same
`MutateExecGrant { settings }` through the daemon journal as the CLI. `se share
grants`, `se share status` and the shell completion name the account and the
list (`exec_run_as`, `exec_programs`); the surface inventory is
`docs/oberflaechen/share-exec.md`.

Older versions (E8). Versions up to 0.5.180 read the profile of schema 8 with
the five fields above and ignore unknown ones. A grant that is enabled and
narrowed (program list or another account) is therefore stored with `enabled`
false and the real switch in `restricted_enabled`
(`share/core/exec_policy_wire.rs`), a field only this version reads, and
read back as enabled only next to a real restriction. A version up to 0.5.180
reads such a grant as switched off, never as the full shell of the worker's
account; if it saves the profile, `run_as`, `programs` and `restricted_enabled`
are lost and the grant stays off with the Standardnutzer (the narrowing is
never replaced by wider rights). Unrestricted and switched-off grants are
stored as before and the profile schema stays 8, so Share keeps working after
a downgrade. The grant journal holds the same structure: a version up to
0.5.180 finds an open entry of a restricted grant contradictory (`pending_apply`
with `enabled` false, its `validate`) and refuses the recovery (fail closed);
this only matters for a crash in the middle of a change followed by a
downgrade.

Migration rules are deliberately conservative:

- every migrated device gets `enabled=false`;
- the old `ShareExportConfig.allow_exec=true` value is discarded because no
  released runtime ever honored it safely;
- an identity-key or NodeId change creates a new disabled Exec policy;
- ordinary presence/name refreshes preserve the policy only when the complete
  pinned identity still matches;
- rejection, block, or revocation disables Exec monotonically;
- a lower signed decision revision can never restore an older authorization.

The base device admission remains anchored in the signed direct request and
decision ledger. The Exec elevation is a monotonic local policy owned by the
target. Signaling delivery state is never an authorization source.

### 4.2 One daemon-owned mutation path

CLI and GUI must not independently edit an Exec bit and then hope the worker
reloads it. Add typed daemon commands for `EnableExec`, `DisableExec`, and
`RevokeGrant`:

- enable: persist through the existing profile CAS transaction, apply the new
  authorization epoch in the worker, then report `active`;
- disable/revoke: establish the runtime deny barrier and cancel matching jobs
  first, persist it, and keep the worker fail-closed if persistence needs retry;
- every response reports `persisted`, `applied`, the new policy revision, and
  any safe pending-retry state.

This also removes the current possibility that a persisted revoke is printed as
successful while a stale worker still holds the old runtime state.

### 4.3 Authorization lease and atomic start

An accepted Exec stream receives a typed `ExecAuthorizationLease` containing:

- principal identity and relation;
- grant policy revision;
- global authorization epoch;
- authenticated Iroh session ID;
- Exec request ID and request digest.

The job is registered as `Preparing` while the authorization lock is held. The
platform process/supervisor is then created but remains behind a launch barrier.
The core reacquires authorization, verifies the lease and cancellation state,
and releases that barrier while still excluding a policy mutation. A revoke
therefore either wins before release and no command code runs, or wins after
release and the registered containment is immediately terminated.

`ExecRegistry` indexes preparing and active jobs by principal and request ID.
Policy changes increment the epoch, reject new commits from older leases, and
cancel all matching jobs. The concurrency permit is held until containment is
confirmed empty, not merely until the root process exits.

## 5. Separate Exec transport

### 5.1 ALPN and handshake

Keep the filesystem data plane on `smart-explorer/share-fs/3`. Exec uses the
accepted ALPN `smart-explorer/share-exec/2` only. Filesystem clients keep
working across a rolling upgrade; Exec never falls back to the filesystem ALPN
or the superseded Exec v1 protocol.

The Exec connection uses a fresh challenge-response handshake (as implemented
in `share/core/exec_admission.rs` and `exec_auth.rs`, re-read 2026-10-16):

1. Iroh completes mutually authenticated QUIC and exposes the remote
   EndpointId. The TLS 1.3 key exchange prefers the hybrid post-quantum group
   `X25519MLKEM768` (`share/core/pq_kx.rs`: ML-KEM-768 plus X25519 on rustls'
   ring provider); a peer without it completes plain X25519 from the classical
   share sent alongside, without an extra round trip.
2. The server sends its identity (device id, public key, fingerprint,
   EndpointId) and a 32-byte random challenge. It sends no protocol limits;
   the frame limits are fixed constants of `exec_protocol.rs`.
3. The client returns its relation/device identity, a fresh client nonce,
   `exec_stream_v2`, and an HMAC-SHA256 under the relation secret over the
   transcript `smart-explorer/share-exec/v2|challenge|client nonce|relation
   kind|relation id|client device id|client public key|client fingerprint|
   client EndpointId|server device id|server public key|server fingerprint|
   server EndpointId|client-to-server|exec_stream_v2`. The fixed prefix binds
   the protocol and version (the ALPN `smart-explorer/share-exec/2` itself is
   not repeated), the literal `client-to-server` binds the direction.
4. The server checks, in this order: protocol fields and that the claimed
   EndpointId is the TLS EndpointId; that the claimed identity matches an
   accepted grant (direct) or an admitted member (room); the HMAC; and only
   then the device's Exec permission. A wrong proof is therefore always an
   authentication failure, also for a device whose Exec is off, and only
   authentication failures count towards the penalties of §9.
5. The server returns the current policy revision, the provider status and
   its features. After the client's start, the grant's program list and
   run-as account are applied before any process exists.

The project sends no 0-RTT data for Exec (iroh's TLS layer permits early data,
but no Share client uses it). Captured handshakes cannot be replayed because
the server challenge is fresh. A signaling server can suppress address or
presence data, but it cannot create an authenticated Exec session.

### 5.2 Stream protocol

Use one bidirectional QUIC stream per execution:

```text
client -> Start
server -> Started
client -> StdinChunk* -> StdinEof
client -> Cancel                         (at any time)
client -> Ping*                          (authenticated idle liveness)
server -> Pong*                          (matching Exec ID and sequence)
server -> StdoutChunk* / StderrChunk*
server -> Terminal(Exited | Failed | TimedOut | Cancelled | Revoked | Disconnected)
server -> Error                           (protocol/provider rejection)
client -> ResultAck
server -> ResultAcknowledged
```

Requirements:

- `Start` has a random 128-bit `exec_id` and canonical request digest.
- duplicate ID plus another digest is rejected;
- a live duplicate reports `AlreadyRunning`; a bounded terminal cache may
  return the prior status but never starts the command twice;
- the sender never transparently retries an ambiguous start;
- stdin/stdout/stderr chunks are raw binary, at most 64 KiB, not JSON arrays or
  Base64;
- each direction uses fixed-memory bounded queues and transport backpressure;
- authenticated Ping/Pong runs every 5 seconds, declares a silent peer failed
  after 20 seconds, and bounds every frame write to 10 seconds;
- EOF, Cancel, stream reset, and transport loss are distinct protocol events;
- malformed order, unknown IDs, oversized frames, and extra terminal frames
  close the stream and cancel any admitted job;
- `Started` is sent only after containment and the authorization launch commit;
- a terminal success is sent only after `confirm_empty`;
- neither endpoint treats a terminal result as delivered until the
  `ResultAck`/`ResultAcknowledged` exchange completes.

With the default heartbeat policy, the server allows 35 seconds for ResultAck:
the 20-second peer window, a 10-second bounded write, and one 5-second heartbeat
interval of strict scheduling slack. The client then allows 50 seconds through
ResultAcknowledged, adding the server's bounded confirmation write and another
full interval. Both budgets are derived from the shared policy so they cannot
silently meet at the same timeout boundary.

Handshake and control frames get operation-specific small limits (for example
64 KiB handshake and 128 KiB Start). The existing 16 MiB generic frame limit is
not used as the pre-allocation budget for this protocol.

## 6. Command contract

Two modes deliberately share the same full-code permission:

- `Argv { program, args }` invokes the program directly without shell parsing.
- `Shell { command }` passes the exact command string to the user's shell.

Linux uses the configured user shell (`$SHELL -lc`) with a documented fallback
to `/bin/sh`. Windows uses `%COMSPEC% /D /S /C`. Argument quoting for direct
Windows execution follows `CreateProcessW`/`CommandLineToArgvW` rules and is
covered by round-trip tests; it must not be assembled with ad-hoc quoting.

The default working directory is the worker user's home. An explicit working
directory may be any path accessible to that user; it is not restricted to
Share export roots because arbitrary code could access the same paths anyway.

For the Standardnutzer the child inherits the worker's environment minus every
variable whose name starts with `SMART_EXPLORER_` (`exec_supervisor_protocol.rs::
environment_for`); the daemon IPC token is read from its 0600 file, never from
the environment. A job of another account starts with a clean environment
(HOME, USER, LOGNAME, SHELL from the account, PATH
`/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`, LANG, LC_ALL,
LC_CTYPE and TZ of the worker). Arbitrary caller-supplied environment
overrides are allowed in both cases. Validation rejects only representations the
OS cannot accept (NUL, invalid Windows environment names, arithmetic/encoding
overflow, or protocol-frame overflow).

A grant with a program list tightens both inputs of the start
(`share/core/exec_policy.rs::authorize_start`, `exec_program_path.rs`):

- Environment: the job takes from the caller only `LANG`, `TERM` and
  `LC_<A-Z0-9_>`, each with a value of at most 64 bytes from
  `[A-Za-z0-9._@+-]` (no path, no separator: glibc and ncurses read a name
  with `/` as a path). Any other variable, among them `PATH`, `LD_PRELOAD`,
  `LD_LIBRARY_PATH` and `GIT_PAGER`, refuses the start; the refusal names the
  variable, never its value. The rule follows sshd's `AcceptEnv LANG LC_*`.
  The worker's own environment (minus `SMART_EXPLORER_*`) still reaches the job
  as for the Standardnutzer. Grants without a program list keep the behaviour
  described above.
- Program resolution: a program is found only as an absolute path or in a
  fixed search path of the host, never through the `PATH` the caller sends or
  the worker's own. Linux: `/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`
  (`exec_supervisor_protocol.rs::ACCOUNT_PATH`); Android: `/system/bin`,
  `/system/xbin`, `/vendor/bin`; Windows: `%SystemRoot%\System32`,
  `%SystemRoot%` and `%SystemRoot%\System32\Wbem` with the suffixes `.exe` and
  `.com` (never `.cmd` or `.bat`, which start a shell). A listed entry is
  resolved the same way. A start is allowed when it resolves to the same file as
  a listed entry (links followed) under the same file name, so a listed `ls`
  does not open the other applets of a busybox or toybox binary. A relative
  path with a separator is refused in the list and in the start. The job starts
  with the resolved absolute path (the supervisor searches nothing itself), and
  the audit entry `started` names it as `resolved_program`.

There is no artificial runtime maximum and no default output truncation. A
caller may opt into `--timeout` or `--max-output`; zero/unset means unlimited.
Streaming keeps unlimited output out of receiver memory. If an explicit output
budget is reached, both pipes continue to be drained and discarded so the
process does not deadlock, and the terminal status reports truncation.

The first version supports streamed stdin and normal pipes. PTY/ConPTY terminal
emulation is a separate presentation layer, not a prerequisite for arbitrary
non-interactive shell commands. Adding it later must reuse the same grant,
transport, registry, and containment boundaries.

Update 2026-10-14: the terminal presentation is implemented as the capability
`exec_terminal_v1` over the same grant, transport (ALPN
`smart-explorer/share-exec/2`), registry, and containment. A start with a
terminal request runs the host user's login shell (`ExecCommand::Login`) in a
POSIX PTY on Linux and Android hosts or in a ConPTY on Windows hosts (Windows 10
1809 / Server 2019 or newer); the client sends such a start only to a host that
reports the capability, and window changes travel as the RESIZE frame. Starts
without a terminal are byte-identical to before. See
`docs/plaene/2026-10-14-echtes-terminal/plan.md`; the open parts are tracked as
`ETERM*` and `EXEC-PTY` in `docs/TODO.md`.

## 7. Platform containment boundary

The portable core owns command models, policy, state transitions, protocol, and
deterministic decisions. Host process behavior lives behind a small typed API:

```rust
trait ContainedExec: Send {
    fn write_stdin(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn close_stdin(&mut self) -> io::Result<()>;
    fn next_event(&mut self, deadline: Option<Instant>)
        -> io::Result<PlatformExecEvent>;
    fn terminate_all(&mut self, reason: StopReason) -> io::Result<()>;
    fn confirm_empty(&mut self, deadline: Instant) -> io::Result<()>;
}
```

`terminate_all` is idempotent. `Drop` always attempts it. An implementation
must never report a successful spawn if command-controlled code could have run
outside its containment.

### 7.1 Windows

Create one unnamed Job Object per execution:

1. Create the Job Object and set `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; do not
   enable either breakaway flag.
2. Create only the stdin/stdout/stderr pipes needed by the protocol. Parent
   ends and every unrelated worker handle are non-inheritable.
3. Build `STARTUPINFOEX` with `PROC_THREAD_ATTRIBUTE_JOB_LIST` and
   `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`.
4. Call `CreateProcessW` with `EXTENDED_STARTUPINFO_PRESENT`,
   `CREATE_UNICODE_ENVIRONMENT`, `CREATE_SUSPENDED`, and `CREATE_NO_WINDOW`.
   Atomic job assignment means no command code can run before containment;
   suspended creation is a second launch barrier for the auth-epoch commit.
5. If job assignment or handle setup fails, terminate the suspended process and
   fail closed. Never retry without the Job Object.
6. Resume only after the authorization lease commit.
7. On every stop reason call `TerminateJobObject`, drain/close pipes, and query
   until `ActiveProcesses == 0`.
8. The worker owns the sole non-inherited Job handle. A worker crash closes it,
   and the kernel applies kill-on-close to the complete job tree.

Nested Jobs are supported by the Windows versions Smart Explorer targets. A
restricted outer topology is used only when atomic assignment proves the new
supervisor remains in Smart Explorer's own Job; any topology Windows rejects
remains a diagnosed fail-closed condition.

### 7.2 Linux

Process groups, `setsid`, `PR_SET_PDEATHSIG`, a root pidfd, and `/proc` scans do
not guarantee complete tree teardown. Linux therefore uses one transient
systemd service/cgroup per execution:

1. Probe cgroup v2 and the appropriate systemd manager before advertising
   Exec. A normal desktop/headless user uses its user manager; root may use the
   system manager. `se doctor` reports the exact missing provider or permission.
2. Use systemd's D-Bus `StartTransientUnit` API through a target-specific Rust
   D-Bus adapter (prefer `zbus` without C dependencies). Do not execute a
   `systemd-run` found through `PATH`.
3. Start the running Smart Explorer binary (`/proc/<worker pid>/exe`, the
   same inode the worker executes; no separate hash check) in the hidden
   `--share-exec-supervisor` mode as the unit MainPID. The launch spec travels
   only over a Unix socket (mode 0600) in a 0700 runtime directory after the
   supervisor connected; the worker accepts it only when `SO_PEERCRED` shows
   the worker's own uid/gid and the unit's MainPID equals the peer's pid (no
   random token).
4. Configure `Type=exec`, `KillMode=control-group`, `KillSignal=SIGKILL`,
   `SendSIGKILL=yes`, `Restart=no`, `RemainAfterExit=no`, a short
   `TimeoutStopSec`, and `RuntimeMaxSec` when the caller supplied a timeout.
5. The supervisor starts the payload (as the grant's run-as account where one
   is set, see §2), proxies stdio, watches the worker channel, waits for the
   root's status with `Child::try_wait` (no pidfd), and exits on cancellation
   or HUP.
6. On root exit, systemd stops the unit and kills remaining descendants. On
   supervisor crash, worker crash, cancellation, or timeout, the manager also
   stops the entire control group.
7. Wait for the unit to be inactive and `populated=0` before returning success.

The kernel's cgroup v2 `cgroup.kill` operation covers descendant cgroups,
concurrent forks, and migration races. `KillMode=control-group` is the service
manager contract used to drive that primitive. If the provider is unavailable,
Linux Exec remains unavailable; there is no process-group fallback.

## 8. Lifecycle and observable state

Both endpoints use the same explicit state vocabulary:

```text
queued_local -> connecting -> authenticating -> authorized -> starting
             -> running -> cancelling
             -> exited | failed | timed_out | cancelled | revoked | disconnected
```

Every transition includes `exec_id`, peer identity, policy revision, monotonic
time, and a reason. The target keeps a bounded in-memory history (128 entries,
`exec_registry.rs`) with executable name (`<shell>` for shell and terminal
starts), command digest, start/end time, exit status, transferred byte counts
and truncation flags; host messages are removed from it. It holds no cwd and
no command text. The initiating side's outgoing history (`daemon/os/shared/
exec_state.rs`) keeps the host's messages, because they answer that user's
own commands.

Since 2026-10-16 the target also writes a persistent audit log (JSON lines,
private files, `share/os/shared/exec_audit.rs`) in two classes, each with its
own rotation at 4 MiB to exactly one predecessor file (`.1`):

- `<App-Daten>/exec-audit.log`: what authorized devices did: refused starts,
  starts (device, fingerprint, relation, mode, program or `<shell>`,
  `resolved_program` for a program list, digest of the whole start, cwd,
  timeout, account) and ends (result, exit code, signal, byte counts,
  duration);
- `<App-Daten>/exec-audit-rejected.log`: what strangers and refused devices
  caused: refused authentications (TLS EndpointId, claimed device, reason; also
  a verified device whose Exec is off) and penalty refusals.

Both classes exist because a new EndpointId costs an attacker nothing: ten
thousand rejections of unknown EndpointIds fill and rotate only the second
class and can never push a `started` or `finished` entry out of the first
(unit test `luecken_task_ten_thousand_rejections_never_evict_a_start_or_finish_entry`).
Text supplied by the other side (device names, claimed ids, program names,
working directories, refusal reasons) is cut to 256 characters, and control,
C1 and direction-changing characters are replaced by `?`, so a crafted name
cannot act on the terminal that shows the log. Shell command text is never
logged, like OpenSSH, whose `auth.log` names logins but no commands. A write
error never blocks an execution. `se share exec log [--lines N] [--json]` merges
both classes by time (seconds; within one second the first class first) and
works without the running worker. The desktop („Befehle → Exec-Protokoll dieses
Hosts“) and Android („Befehlsprotokoll anzeigen“, `share.execLog`) read the same
files through `share/os/shared/exec_audit_view.rs`: at most 1000 entries,
newest first, one line per entry, and every field cut and cleaned again with
the writer's rules, because a file can be edited from outside; a line that is
no JSON object shows as „Unlesbare Zeile“.

A device holder can still produce `start_refused` lines of its own class in
bulk and so push older entries of that class out after about 8 MiB; every such
line names the device. There is no per-device throttle.

Updates (D11, unchanged by the 2026-10-16 work). A desktop or terminal update
replaces the worker: the updater helper writes `daemon.stop` and waits for the
processes to end (`bin/smart_explorer_updater/process.rs`), and a CLI or GUI
call that meets a worker of another version replaces it
(`daemon/os/shared/ipc_client.rs`, `worker_version_is_current`). Running Exec
and terminal sessions of the old worker end with it (`se shell` exits with 125).
The update neither ends anything beyond that nor keeps sessions across the
switch: keeping them would need the supervisors handed over to the new worker
(TODO `LUECKEN-UPDATE-SESSIONS`). Grants, `policy_revision`, pins, secrets and
saved connections are not touched by the update itself.

Required management surfaces:

- `se share grants` shows file authorization and Exec authorization separately;
- `se share grants exec enable|disable [SELECTOR]` auto-selects only when the
  choice is unambiguous and provides dynamic completion;
- `se share exec list|show|cancel|history` works entirely from the target CLI;
- `se exec` without enough arguments lists eligible peers and exact examples
  instead of only printing a parser error;
- an omitted target auto-selects only when exactly one Exec-capable target is
  available;
- `se exec TARGET -- PROGRAM ARGS...` is direct argv mode;
- `se exec TARGET --shell 'COMMAND'` is shell mode;
- stdout and stderr stream to their corresponding local descriptors, stdin is
  forwarded, Ctrl+C cancels, and the local exit status reflects the remote
  status without hiding transport errors;
- all target/exec/grant selectors participate in Bash, Zsh, Fish, Elvish, and
  PowerShell completion;
- the GUI has a per-device warning toggle plus active/history rows with Cancel,
  Disable Exec, and Revoke actions.

Running as root/Administrator is always visible in the grant row and before
confirmation. Once a grant exists, no per-command prompt is added.

## 9. Protection against outside attackers

The Exec work includes hardening the entry path, not only process spawning:

- cap concurrent pre-auth Iroh handshakes globally (64 TLS handshakes) and
  share the application handshake slots fairly per remote EndpointId
  (`fair_admission.rs`: unknown endpoints get their own pool of 8 slots and
  3 s, known ones 64 slots and 20 s). There is deliberately no fixed cap per
  EndpointId: the pool is work-conserving, fairness hands slots to other
  waiting devices, and new EndpointIds cost an attacker nothing, so a
  per-identity cap would only waste idle capacity;
- require address validation/retry where supported before expensive work;
- apply handshake, first-stream, and frame-read deadlines;
- reauthorize a cached session before reading a potentially large operation
  frame, so a revoked peer cannot consume the old frame budget first;
- use operation-specific frame sizes and bounded channels/queues;
- cap concurrent authenticated executions by configurable global and principal
  budgets; admission limits are resource controls, not command filters;
- penalize repeated authentication failures per EndpointId without letting an
  attacker fill an unbounded identity map (`auth_penalty.rs`, the defaults of
  OpenSSH `PerSourcePenalties`: 5 s per failed authentication, 1 s per
  connection that never authenticated, refused once 15 s accumulated and
  until they ran out, at most 600 s, at most 65 536 tracked identities with
  permissive overflow). Exec connections of a penalized EndpointId are always
  refused; file sessions only for unknown endpoints, so automatic reconnects
  of paired devices are never throttled. A device whose Exec is merely off is
  not penalized. The ledger does not stop an attacker who mints new
  EndpointIds, because every new one starts without a penalty; against that
  only the fair admission above helps (unknown endpoints share 8 slots and 3 s).
  There is deliberately no global limit for unknown endpoints, since it would
  also lock out the pairing attempts of legitimate new devices;
- bound signaling-server connection workers and writer queues;
- keep WSS as the production signaling recommendation, while treating even a
  compromised signaling service only as an availability/metadata attacker;
- never derive an Exec grant, capability, or policy revision from an unsigned
  signaling event or relay acknowledgement;
- do not expose an extra TCP listener for Exec. Commands travel only inside the
  authenticated Iroh ALPN, and local callers use owner-authenticated daemon IPC.

## 10. Local daemon IPC

The current line-based `ExecShare` request/response is replaced for Exec with a
small authenticated handshake followed by binary framed streaming on the same
loopback connection. The existing owner-protected bearer token remains the
local authentication mechanism.

The daemon monitors both halves of the IPC connection. Local EOF or process
death propagates Cancel to the peer. A GUI cancel button uses the same path.
Backpressure and frame budgets match the QUIC side, so the daemon never buffers
the full output. Local IPC never retries Start after handoff.

## 11. Remote connection keepalive companion requirement

Implementation status: **base backend liveness delivered in 0.5.131; Exec
hardened and enabled in 0.5.134**. Exec reuses the same Iroh keepalive policy,
adds authenticated application Ping/Pong frames with bounded failed-peer
detection, and acknowledges the terminal result before either endpoint closes.

Idle connections must remain usable independently of Exec. The user-visible
contract is: after an arbitrary idle period, the next operation either uses the
healthy session or transparently establishes a fresh session before starting
the operation. An idle stale socket must not consume the user's one operation.

Protocol-specific implementation:

| Backend | 0.5.131 behavior |
|---|---|
| SFTP | russh sends a reply-checked SSH keepalive every 15 seconds and retires the transport after three missed replies. The complete DNS/TCP/SSH/authentication/subsystem setup has a 30-second deadline. A retained generation reconnects before the next operation; a safe read may retry once only before it has exposed bytes, while mutations and started streams are never replayed. |
| SSH agent | The SSH layer has the SFTP keepalive above. In addition, an idle `se-agent --serve` channel exchanges `Hello`/`HelloOk` after 30 seconds and requires activity within another 30 seconds. A blackholed generation and its bounded writer queue are closed visibly, then a fresh authenticated serve channel is opened without copying requests from the old generation. |
| FTP/FTPS | A serialized background keeper sends `NOOP` after 15 seconds idle only while the control stream is checked in. The probe has a 10-second I/O deadline. A failed probe reconnects, repeats TLS/login setup, and restores binary mode. One absolute 10-second budget covers bounded Hickory DNS, at most eight addresses, TCP, greeting, `AUTH TLS`, login, and `TYPE`; control/data inactivity is 60 seconds. No heartbeat can enter an active data transfer. Safe reads may retry after reconnection, but mutations and a failed `STOR` are never replayed. |
| WebDAV | Pooled safe `PROPFIND`/`GET` setup retries once when a stale connection dies. `PROPFIND` also consumes the complete private response body inside that attempt, so headers followed by a partial body or blackhole retry once without exposing bytes; an already returned file reader never restarts after exposure. Connect is bounded to 10 seconds and read/write inactivity to 60 seconds, without a total-duration cap on a progressing transfer. Mutations and `PUT` use an unpooled agent with redirects disabled and strict success-status validation; response loss is terminally ambiguous instead of causing another mutation or another `flush()`. |
| Google Drive | No quota-consuming heartbeat is sent: pooled HTTPS reads transparently replace stale sockets, and complete metadata JSON bodies retry within the bounded read policy before any bytes escape. Metadata and OAuth requests have a 60-second overall deadline; streaming downloads use a 10-second connect and 60-second read/write inactivity deadline without limiting total progressing transfer time. A transport failure or 5xx after a mutation is reconciled against the exact reserved/resource ID and expected postcondition rather than replayed. Folder creation durably journals its pre-generated exact ID under the stable Drive `permissionId` before POST; restarts verify or retry only that same ID. Resumable content retains its server-offset reconciliation. |
| Share signaling | DNS resolution, TCP connect, TLS, WebSocket handshake, and writes are explicitly bounded. The client sends Heartbeat every 20 seconds, requires Pong within 40 seconds, and enters the reconnect/republish loop after a missed deadline. Constants, literal-IP DNS bypass, and thresholds are directly tested. |
| Share Iroh/QUIC | Connection and path keepalive are explicitly configured to five seconds instead of depending on crate defaults. A healthy idle session therefore remains active, while a crashed peer that cannot answer probes reaches the explicit 20-second connection idle timeout. A failed cached `open_bi` is generation-checked, then reconnects and reauthenticates once before any request payload exists. Exec uses a fresh connection and never retries after Start may have been sent. Stream I/O has inactivity deadlines. Exported synchronous backend work runs in a bounded blocking pool, so a slow filesystem cannot starve the QUIC timers. |
| Windows UNC/SMB | The Windows redirector owns SMB ECHO and protocol reconnect. Smart Explorer now retains a reference-counted WNet lease in every saved-UNC backend and cancels it only after the final in-process user releases it. |

Keepalive and reconnect are not mutation replay. A failure after a mutating
operation may have reached the remote service and remains an explicit ambiguous
result unless that protocol operation has an exact idempotency key or a verified
postcondition. The deadlines above are inactivity/setup safety bounds, not idle
disconnect policies: a transfer that continues making progress may run longer.

Deterministic tests blackhole and replace SSH-agent and SFTP generations; run a
real local explicit-FTPS TLS fixture through lost encrypted `NOOP`, re-login,
and protected-data blackhole; force FTP mutation-ACK loss; force WebDAV stale
pooled reads, partial bodies, redirects, and DELETE/PUT response loss; reconcile
Drive metadata-body loss, mutation timeouts, 5xx responses, restart, and
crash-safe exact-ID folder journals; exercise Signal/Pong and cached Iroh
replacement; prove a blocked exported backend cannot starve a single-thread
Tokio timer; and stress UNC generation/lease races. The host suite and Windows
target build use the same portable state machines. Credentialed live-server
tests remain an explicit installed-release gate when external SFTP, FTPS,
WebDAV, Drive, SMB, and two-device Share environments are available; a
cross-build is not recorded as native runtime evidence.

## 12. Module layout

New or extracted Rust files stay behavior-scoped and below the repository's
500-line/50-KiB limits:

- `share/core/exec_types.rs` — portable request/result/state types;
- `share/core/exec_policy.rs` — grants, authorization leases, epochs, admission;
- `share/core/exec_protocol.rs` — bounded streaming state machine;
- `share/core/exec_registry.rs` — preparing/active jobs and cancellation;
- `share/core/exec_server.rs` — authenticated server stream orchestration;
- `share/core/exec_client.rs` — client stream orchestration;
- `share/os/windows/exec.rs` — Job Object and `CreateProcessW` adapter;
- `share/os/linux_os/exec.rs` — systemd transient-unit adapter;
- `share/os/linux_os/exec_supervisor.rs` — hidden supervisor mode;
- 2026-10-16: `share/core/exec_policy_wire.rs` (stored form of a grant, older
  readers see a narrowed grant as off), `share/core/exec_program_path.rs`
  (program list lookup in a fixed search path), `share/os/*/exec*.rs`
  `program_search()` per platform, `share/os/linux_os/exec_account.rs` and
  `exec_supervisor_account.rs` (account resolution in the root worker,
  group/user switch in the supervisor), `share/core/auth_penalty.rs`
  (penalties per EndpointId), `share/os/shared/exec_audit.rs` with
  `exec_audit_files.rs` (two classes, rotation, merged reading) and
  `exec_audit_text.rs` (cutting and cleaning of text from the other side);
- `daemon/os/shared/exec_ipc.rs` — full-duplex local Exec bridge;
- `cli/exec.rs` — discoverable CLI, stdio, completion, and exit mapping;
- focused app UI modules for grant confirmation and active/history rendering;
- backend-owned liveness modules for SFTP/Agent, FTP/FTPS, WebDAV/Drive, Share,
  and UNC rather than one protocol-blind heartbeat thread in `vfs::Backend`.

Implemented source stays within the repository's 500-line/50-KiB limit for
new or substantially edited modules. The daemon uses
`daemon/os/shared/exec_ipc.rs` and a separate crash-safe Exec-grant journal;
the GUI grant/job controls are likewise split into focused modules.

Windows bindings add only the required Job Object/process/thread/pipe features.
The Linux D-Bus dependency is target-specific, uses no C library, and is
documented in `Cargo.toml`/`docs/GOTCHAS.md` with its cross-compile impact.

## 13. Test matrix

### 13.1 Authorization and external attack tests

- unknown, rejected, revoked, blocked, or FS-only devices start no process;
- a known Direct code without an accepted exact identity starts no process;
- permission for peer A never authorizes peer B or a changed key/NodeId;
- a room member needs its own explicit Exec policy;
- replayed challenge/hello/start frames fail or deduplicate without execution;
- filesystem v3, Exec v1, absent `exec_stream_v2`, capability stripping, and
  ALPN downgrade fail;
- revoke in every point of the prepare/start barrier either prevents start or
  kills the registered containment;
- stale decisions and profile-CAS conflicts cannot lower the policy revision;
- malformed/slow/oversized handshakes and output consumers stay within fixed
  memory, task, and queue budgets;
- externally triggered FS/Exec handshake and stream errors use fixed diagnostic
  categories, bounded detail, a bounded service queue, and coalesced summaries;
  attacker-controlled error strings cannot create keys or queue entries without
  bound;
- signaling/relay message injection changes at most availability/status, never
  Exec authorization.

### 13.2 Command semantics tests

- direct argv preserves empty args, spaces, quotes, Unicode, and shell
  metacharacters literally;
- shell mode supports pipelines, redirects, conditionals, quoting, and native
  shell built-ins without filtering;
- arbitrary accessible cwd and environment overrides work;
- stdin and binary stdout/stderr including NUL round-trip unchanged;
- simultaneous stdout/stderr cannot deadlock;
- success, nonzero exit, signal termination, explicit timeout, cancellation,
  and transport failure remain distinct;
- no-timeout and no-output-limit executions run/stream beyond the old limits;
- an explicit output cap drains discarded bytes and reports truncation.

### 13.3 Windows containment tests

Use a committed test helper with `tree`, `root-exits-first`, `ignore-stop`,
`hold-pipes`, `fork-storm`, `echo-stdin`, `dump-handles`, and `print-env` modes:

- every normal descendant reports `IsProcessInJob=true`;
- root exit, timeout, Cancel, QUIC reset, worker Stop, and forced worker death
  leave no helper PIDs;
- an outer compatible Job tests nesting, including inside a runner-owned
  ambient Job; under a UI-restricted outer Job the exact production launcher
  either fails closed before payload start or proves own-Job membership,
  contained execution, breakaway denial, and full drain;
- `CREATE_BREAKAWAY_FROM_JOB` cannot escape;
- the child inherits only its three stdio handles;
- worker crash proves kernel kill-on-close rather than cooperative cleanup.

### 13.4 Linux containment tests

Under cgroup v2/systemd:

- double-fork, `setsid`, root-exits-first, ignore-signal, and concurrent-fork
  helpers all remain in the transient unit cgroup;
- timeout, Cancel, QUIC reset, worker Stop, `SIGKILL` of the worker, and
  `SIGKILL` of the supervisor leave `populated=0`;
- `RuntimeMaxSec` independently cleans up a deliberately hung worker path;
- absence of a user/system manager starts no payload and produces the exact
  `se doctor` containment diagnostic;
- a negative fixture demonstrates why process-group fallback is rejected.

### 13.5 End-to-end and UX tests

- four isolated real worker profiles establish identities, requests, and grants
  using only CLI output from that test run;
- no hidden fixture ID, fingerprint, request ID, or peer ID is injected;
- no-argument commands list/auto-select correctly and completions return live
  grant/peer/exec selectors on every supported shell;
- status is visible and consistent on requester and target for every state;
- installed Linux candidates execute the full multi-profile request
  receive/accept/reject/delete-persistence suite plus argv, shell, binary I/O,
  nonzero exit, healthy idle, Cancel, revoke, and worker-crash coverage;
- native Windows CI runs four isolated Share profiles through the same protocol
  and real Windows provider twice: first with native test binaries, then with
  the exact staged Windows GNU `se.exe` and `se-share-server.exe` bytes that the
  feed/installer ships. Coverage includes remote `cmd.exe`, nonzero exit,
  healthy idle, hard worker crashes, disable, denial, signed revoke, receipts,
  reject, and context-free pending/history deletion;
- published Windows <-> Linux candidates and the old-version update path remain
  the separate M11 interoperability certification rather than being inferred
  from cross-compilation;
- all remote backends pass a real idle-beyond-server-timeout operation test.

## 14. Implementation milestones and validation gates

M0–M9 are implemented in 0.5.134. M10 shipped in 0.5.131. The static Linux
release-feed `se` and release Share server pass locally with four isolated
profiles; the run obtains the invite, request, peer, grant, path, and Exec IDs
only from earlier CLI output. It covers request
receive/accept/reject/delete persistence, binary stdin/stdout, stderr, exit 7,
healthy idle, explicit output truncation, timeout, target-side Cancel, local CLI
death, target-worker `SIGKILL`, cgroup/socket cleanup, permission revoke, and
post-revoke start denial. Native Windows CI must pass the real Job-Object
provider self-test (including root-exits-first and empty containment), the
four-profile Share/Exec lifecycle with remote `cmd.exe`, and a second lifecycle
run using the exact staged Windows GNU release CLI/server. GitHub publication
depends on that release-binary run rather than only on cross-compilation.

| Milestone | Functional result | Primary files | Validation signal |
|---|---|---|---|
| M0 — threat-model fixtures | Fake identities, policy epochs, protocol peers, and process-tree helper exist before runtime code | test support only | Every listed negative case initially fails for the expected missing feature |
| M1 — per-device policy | Profile migration discards global Exec and grants default deny per exact identity | `types.rs`, `profiles.rs`, persistence/ledger/policy modules, CLI/GUI grant views | Migration, stale-decision, identity-change, CAS, and selector tests pass |
| M2 — daemon-owned mutation | Enable/disable/revoke are atomic runtime+durable operations with explicit apply state | daemon IPC/host/profile transaction, CLI/GUI actions | Injected persistence and worker failures remain fail-closed and visible |
| M3 — new ALPN/auth | Challenge-response Exec sessions negotiate only on the new ALPN | `node.rs`, `session.rs`, new exec protocol/client/server | v3 compatibility plus replay/downgrade/wrong-identity tests pass |
| M4 — streaming core | Start/stdin/output/cancel/terminal state machine is bounded and at-most-once | exec types/protocol/registry plus fake adapter | protocol model, fuzz/property, backpressure, duplicate-ID tests pass |
| M5 — local streaming IPC/CLI | CLI is discoverable and carries raw full-duplex streams through the worker | daemon IPC, `cli/exec.rs`, completions | parser/help/completion, stdin/output/exit/Ctrl+C tests pass without a real peer |
| M6 — Windows provider | No Windows payload code runs outside a kill-on-close Job | `share/os/windows/exec.rs`, Cargo features, helper fixture | native Windows process-tree, handle, crash, and nested-Job suite passes |
| M7 — Linux provider | Every payload runs in a transient systemd cgroup with independent cleanup | Linux exec/supervisor, target-specific D-Bus dependency | cgroup tree, crash, unavailable-provider, and `RuntimeMaxSec` suite passes |
| M8 — policy cancellation/UI | Revoke, worker refresh/stop, and GUI/CLI Cancel terminate matching active jobs and show history | registry integration, signal commands, app UI | race stress tests and state parity on both endpoints pass |
| M9 — outside-DoS hardening | Pre-auth work, frames, queues, server writers, idle-peer detection, and terminal-result acknowledgement are bounded | node/framing/server and share-server | Slowloris/connection flood/oversize, heartbeat-timeout, and terminal-handshake tests pass under memory/task assertions |
| M10 — all-backend keepalive ✅ 0.5.131 | Stateful remotes survive idle through protocol heartbeats or safe fresh connections; request-based backends avoid artificial traffic | SFTP, FTP/FTPS, WebDAV/Drive, Share, UNC | deterministic stale-connection/non-replay tests, host suite, Windows cross-check, and full release builds pass; credentialed live-server smokes remain environment-gated |
| M11 — published cross-OS certification | Published candidates work Windows <-> Linux and across an old-version update from a context-free CLI run | release interoperability lab | fresh install and update-path matrix passes with orphan-PID checks; same-OS native gates do not claim this result |

No later milestone may paper over a failed earlier security gate. In particular,
the CLI may be enabled only after both M6 and M7 pass on their native platforms and
the release matrix proves that the corresponding artifacts contain those
providers.

## 15. Release gate

This is a native security feature and follows the complete native release path:

1. `cargo fmt`, focused unit/integration tests, broad host tests, strict clippy,
   and Windows target compilation;
2. native Windows containment tests and Linux systemd/cgroup tests;
3. Share direct and relay E2E, including negative authorization tests;
4. installed Linux multi-profile CLI validation plus native Windows
   multi-profile CLI/Exec and Job-Object gates, including the exact staged
   Windows GNU CLI/server bytes and always using IDs discovered during the run;
5. patch-version bump and full local Windows/Linux feed build;
6. matching commit on `main`, tag, GitHub Release, all expected artifacts and
   hashes, and post-publication Linux CLI reinstall verification. Published
   cross-OS/update-path certification is tracked separately as M11.

The release gate verifies the static feed `se` byte-for-byte before publication,
then reinstalls and rechecks the published Linux payload. The installed CLI must
expose Exec in Help. A platform that cannot provide its containment provider
reports that specific prerequisite and starts no payload.

## 16. Primary references

Checked on 2026-07-13:

- [Iroh 1.0 README: public-key TLS identity, mutual authentication, encrypted relay traffic](https://github.com/n0-computer/iroh/blob/v1.0.1/README.md)
- [Microsoft Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
- [Microsoft process attribute list, including `PROC_THREAD_ATTRIBUTE_JOB_LIST`](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)
- [Microsoft process creation flags](https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags)
- [Linux kernel cgroup v2, including delegation containment and `cgroup.kill`](https://docs.kernel.org/admin-guide/cgroup-v2.html)
- [systemd kill behavior and `KillMode=control-group`](https://www.freedesktop.org/software/systemd/man/latest/systemd.kill.html)
- [systemd D-Bus `StartTransientUnit`](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.systemd1.html)
- [systemd transient-control-group interface](https://systemd.io/CONTROL_GROUP_INTERFACE/)
- [zbus: pure-Rust D-Bus API](https://github.com/z-galaxy/zbus)
