# Secure peer remote execution — implementation and validation record

Status: **implemented for 0.5.133**. Exec remains fail-closed until one exact
device grant is explicitly enabled and the host containment provider is
available.

Research date: **2026-07-12**. Local installed-Linux validation: **2026-07-13**.

This plan enables unrestricted command execution over Smart Explorer Share. An
explicitly authorized peer gets the same practical authority as the OS account
running the Smart Explorer worker. There is deliberately no command allowlist,
path allowlist, shell grammar filter, restricted token, seccomp profile, or
application sandbox.

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
10. Release validation exercises the installed Windows and Linux artifacts in
    both directions, using only values discovered by earlier CLI commands in
    the same test run.

## 2. Explicit trust contract

Enabling Exec for a device is equivalent to giving that device an interactive
shell as the Smart Explorer worker's user. It can read and modify everything
that account can, including credentials, startup files, Smart Explorer state,
and unrelated user data. It can install persistence using facilities already
available to that user. Revoking the grant prevents future commands and stops
contained active commands; it cannot undo actions already performed.

There is no privilege transition:

- a normal user worker starts a normal user process;
- an elevated Windows worker starts an elevated process;
- a root Linux worker starts a root process.

The grant UI and CLI must say `FULL <user> CODE EXECUTION` and, when applicable,
`REMOTE ADMINISTRATOR SHELL` or `REMOTE ROOT SHELL`. This warning happens once
when changing the permission. Commands are not individually confirmed or
filtered afterward.

The containment contract covers processes normally created by the command and
its descendants. Trusted code can intentionally ask an independent broker such
as Task Scheduler, WMI, a service manager, Docker, or `systemd-run` to start
unrelated work. Preventing that would require a sandbox or reduced account and
would contradict this feature's unrestricted-user-shell contract.

## 3. Pre-implementation audit (resolved in 0.5.133)

The pre-0.5.133 data plane was a sound base:

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

The following gaps blocked the old batch stub. 0.5.133 resolves them with the
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

Keep the filesystem data plane on `smart-explorer/share-fs/3`. Add an accepted
ALPN such as `smart-explorer/share-exec/1` for Exec only. Filesystem clients keep
working across a rolling upgrade; Exec never falls back to v3.

The Exec connection uses a fresh challenge-response handshake:

1. Iroh completes mutually authenticated QUIC and exposes the remote
   EndpointId.
2. The server sends a cryptographically random challenge and protocol limits.
3. The client returns its relation/device identity, a fresh client nonce,
   `exec_stream_v1`, and an HMAC over the complete transcript: ALPN, both
   nonces, relation, both device IDs, both NodeIds, and both endpoint roles.
4. The server checks the TLS EndpointId, identity pins, relation proof, current
   base grant, and per-device Exec policy.
5. The server returns the negotiated capability and current policy revision.

No 0-RTT data is accepted for Exec. Captured handshakes cannot be replayed
because the server challenge is fresh. A signaling server can suppress address
or presence data, but it cannot create an authenticated Exec session.

### 5.2 Stream protocol

Use one bidirectional QUIC stream per execution:

```text
client -> Start
server -> Started
client -> StdinChunk* -> StdinEof
client -> Cancel                         (at any time)
server -> StdoutChunk* / StderrChunk*
server -> Finished | Cancelled | TimedOut | Revoked | Error
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
- EOF, Cancel, stream reset, and transport loss are distinct protocol events;
- malformed order, unknown IDs, oversized frames, and extra terminal frames
  close the stream and cancel any admitted job;
- `Started` is sent only after containment and the authorization launch commit;
- a terminal success is sent only after `confirm_empty`.

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

The child inherits the user's normal worker environment. Process-private Smart
Explorer handoff values, IPC bearer tokens, and temporary bootstrap secrets are
removed to avoid accidental disclosure, while arbitrary caller-supplied
environment overrides are allowed. Validation rejects only representations the
OS cannot accept (NUL, invalid Windows environment names, arithmetic/encoding
overflow, or protocol-frame overflow).

There is no artificial runtime maximum and no default output truncation. A
caller may opt into `--timeout` or `--max-output`; zero/unset means unlimited.
Streaming keeps unlimited output out of receiver memory. If an explicit output
budget is reached, both pipes continue to be drained and discarded so the
process does not deadlock, and the terminal status reports truncation.

The first version supports streamed stdin and normal pipes. PTY/ConPTY terminal
emulation is a separate presentation layer, not a prerequisite for arbitrary
non-interactive shell commands. Adding it later must reuse the same grant,
transport, registry, and containment boundaries.

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

Nested Jobs are supported by the Windows versions Smart Explorer targets. An
incompatible outer Job remains a diagnosed fail-closed condition.

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
3. Start the current verified Smart Explorer binary in a hidden Exec-supervisor
   mode as the unit MainPID. Pass the launch spec only over an owner-only,
   randomly authenticated local channel after the supervisor is live.
4. Configure `Type=exec`, `KillMode=control-group`, `KillSignal=SIGKILL`,
   `SendSIGKILL=yes`, `Restart=no`, `RemainAfterExit=no`, a short
   `TimeoutStopSec`, and `RuntimeMaxSec` when the caller supplied a timeout.
5. The supervisor starts the payload, proxies stdio, watches the worker channel,
   waits for the root pidfd/wait status, and exits on cancellation or HUP.
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
time, and a reason. The target keeps a bounded history with command text
redacted by default because shell strings commonly contain secrets. It may
store executable name, command digest, cwd, start/end time, exit status,
transferred byte counts, and truncation flags. Raw commands are shown only for
active executions and are not persisted unless the user explicitly enables
that audit option.

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

- cap concurrent pre-auth Iroh handshakes globally and per remote EndpointId;
- require address validation/retry where supported before expensive work;
- apply handshake, first-stream, and frame-read deadlines;
- reauthorize a cached session before reading a potentially large operation
  frame, so a revoked peer cannot consume the old frame budget first;
- use operation-specific frame sizes and bounded channels/queues;
- cap concurrent authenticated executions by configurable global and principal
  budgets; admission limits are resource controls, not command filters;
- rate-limit repeated authentication failures without letting an attacker fill
  an unbounded identity map;
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
enabled in 0.5.133**. Exec reuses the same Iroh keepalive policy and adds
bounded failed-peer detection without closing a healthy idle session.

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
- v3, absent `exec_stream_v1`, capability stripping, and ALPN downgrade fail;
- revoke in every point of the prepare/start barrier either prevents start or
  kills the registered containment;
- stale decisions and profile-CAS conflicts cannot lower the policy revision;
- malformed/slow/oversized handshakes and output consumers stay within fixed
  memory, task, and queue budgets;
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
- an outer compatible Job tests nesting; an incompatible Job fails before user
  code;
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

- two isolated real workers establish identities and grants using only CLI
  output from that test run;
- no hidden fixture ID, fingerprint, request ID, or peer ID is injected;
- no-argument commands list/auto-select correctly and completions return live
  grant/peer/exec selectors on every supported shell;
- status is visible and consistent on requester and target for every state;
- installed Linux candidates execute the full two-profile argv, shell, binary
  I/O, nonzero exit, long-running, Cancel, revoke, and worker-crash suite;
- native Windows CI runs two isolated Share endpoints through the same protocol
  and real Windows provider, including remote `cmd.exe`, nonzero exit, disable,
  denial, signed revoke, receipt, and context-free history deletion;
- published Windows <-> Linux candidates and the old-version update path remain
  the separate M11 interoperability certification rather than being inferred
  from cross-compilation;
- all remote backends pass a real idle-beyond-server-timeout operation test.

## 14. Implementation milestones and validation gates

M0–M9 are implemented in 0.5.133. M10 shipped in 0.5.131. The static Linux
release-feed `se` and release Share server are locally green with two isolated
profiles; the run obtains the invite, request, peer, grant, and Exec IDs only
from earlier CLI output. It covers binary stdin/stdout, stderr, exit 7, explicit
output truncation, timeout, target-side Cancel, local CLI death, target-worker
`SIGKILL`, cgroup/socket cleanup, permission revoke, and post-revoke start
denial. Native Windows CI runs both the real Job-Object
provider self-test (including root-exits-first and empty containment) and a
two-isolated-profile Share/Exec lifecycle with remote `cmd.exe`; the GNU target
compile additionally verifies the full Windows CLI/provider surface.

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
| M9 — outside-DoS hardening | Pre-auth work, frames, queues, and server writers are bounded | node/framing/server and share-server | Slowloris/connection flood/oversize tests pass under memory/task assertions |
| M10 — all-backend keepalive ✅ 0.5.131 | Stateful remotes survive idle through protocol heartbeats or safe fresh connections; request-based backends avoid artificial traffic | SFTP, FTP/FTPS, WebDAV/Drive, Share, UNC | deterministic stale-connection/non-replay tests, host suite, Windows cross-check, and full release builds pass; credentialed live-server smokes remain environment-gated |
| M11 — published cross-OS certification | Published candidates work Windows <-> Linux and across an old-version update from a context-free CLI run | release interoperability lab | fresh install and update-path matrix passes with orphan-PID checks; same-OS native gates do not claim this result |

No later milestone may paper over a failed earlier security gate. In particular,
the CLI stays disabled until both M6 and M7 pass on their native platforms and
the release matrix proves that the corresponding artifacts contain those
providers.

## 15. Release gate

This is a native security feature and follows the complete native release path:

1. `cargo fmt`, focused unit/integration tests, broad host tests, strict clippy,
   and Windows target compilation;
2. native Windows containment tests and Linux systemd/cgroup tests;
3. Share direct and relay E2E, including negative authorization tests;
4. installed Linux two-profile CLI validation plus native Windows two-profile
   CLI/Exec and Job-Object gates, always using IDs discovered during the run;
5. patch-version bump and full local Windows/Linux feed build;
6. matching commit on `main`, tag, GitHub Release, all expected artifacts and
   hashes, and post-publication Linux CLI reinstall verification. Published
   cross-OS/update-path certification is tracked separately as M11.

The locally installed `se 0.5.133` is byte-identical to the verified static feed
payload (`SHA-256 badfa1f0f247bb14c0d17deace975bcdadba6bedc7823303ec36ec9d1af70e17`),
completed the two-profile Linux gate with the release Share server, and exposes
Exec in Help. A platform that cannot provide its containment provider reports
that specific prerequisite and starts no payload.

## 16. Primary references

Checked on 2026-07-12:

- [Iroh 1.0 README: public-key TLS identity, mutual authentication, encrypted relay traffic](https://github.com/n0-computer/iroh/blob/v1.0.1/README.md)
- [Microsoft Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
- [Microsoft process attribute list, including `PROC_THREAD_ATTRIBUTE_JOB_LIST`](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)
- [Microsoft process creation flags](https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags)
- [Linux kernel cgroup v2, including delegation containment and `cgroup.kill`](https://docs.kernel.org/admin-guide/cgroup-v2.html)
- [systemd kill behavior and `KillMode=control-group`](https://www.freedesktop.org/software/systemd/man/latest/systemd.kill.html)
- [systemd D-Bus `StartTransientUnit`](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.systemd1.html)
- [systemd transient-control-group interface](https://systemd.io/CONTROL_GROUP_INTERFACE/)
- [zbus: pure-Rust D-Bus API](https://github.com/z-galaxy/zbus)
