# SSH Remote-Agent — Implementation Plan

A code-level plan for an **opt-in remote helper** that Smart Explorer deploys
over an existing SSH connection so that file *exploration* (listing, the
storage-analysis walk, search/filter) runs **locally on the server** and only
the **results** stream back — instead of the client paying one network
round-trip per directory.

Status: **phases 1–5 implemented & build-verified (host + windows-gnu); the agent
is functional for Linux x86_64/aarch64 servers — only a real-server smoke test
remains.** Researched against how VS Code Remote-SSH, JetBrains Gateway,
`rclone`, and `ansible` deploy and drive a remote side. Date: 2026-06-18.

### Implementation status

- ✅ **Phase 1 — protocol + agent core** (`agent_proto.rs`, `src/bin/se-agent.rs`).
  Framed wire protocol + server-side local fs ops + `serve()`. Unit-tested
  (req/resp roundtrip, framed `serve()` over an in-memory pipe).
- ✅ **Phase 2 — `AgentBackend`** (`agent.rs`) implementing `vfs::Backend`;
  `vfs::Backend::{supports_walk_tree,walk_tree}` hooks; the analytics remote
  scan uses `walk_tree` (one server-side walk) with client-side fallback;
  `analytics::from_wire`. Tested end-to-end against an in-process agent over a
  **TCP socket pair** (`list_dir` + `walk_tree` + `stat`) — no SSH needed.
- ✅ **Phase 3 — SSH transport + deploy logic.** `SftpBackend::{exec_capture,
  open_exec_streams}` (second exec channel on the live russh session + blocking
  read/write adapters over the channel stream); `agent::{deploy_over_sftp,
  remove_from_sftp, artifact_for, sh_quote}` (detect → probe → verified upload →
  `chmod` → launch → handshake). Compiles against the real russh 0.61 API on
  host + windows-gnu; `sh_quote`/`artifact_for` unit-tested. **Not runtime-tested
  (no SSH server in the build env).**
- ✅ **Phase 4 — connect UX.** `use_agent` flag on `SavedConnection` (TSV field 9,
  backward-compatible: old 8-field lines → false) + `ConnectForm`; an opt-in
  checkbox in the connect dialog (SFTP only); the SFTP connect path calls
  `deploy_over_sftp` and falls back to plain SFTP on any error.
- ✅ **Phase 4b — runtime activation (0.5.65).** You don't have to decide at
  config time: an **"⚡ Agent aktivieren"** button on the *live* connection
  indicator deploys the agent on the already-established SFTP session (no
  reconnect — `RemoteState.sftp` keeps the concrete backend; deploy runs
  off-thread, then the backend is swapped to `AgentBackend` in place) and
  persists `use_agent=true` on the matching saved connection so it sticks.
  Falls back silently on any error; the "⚡ Agent" badge then shows it's active.
- ✅ **Phase 5 — cross-compile + bundle.** A standalone minimal crate
  (`/se-agent`, rayon-only — no ring/TLS) cross-compiles to **static musl**
  without a musl C toolchain (rust-lld for aarch64). Built binaries are committed
  under `native/agent-bin/se-agent-{x86_64,aarch64}-linux-musl` (~0.5/0.4 MB,
  static, stripped) and embedded via `include_bytes!` in `artifact_for`. The
  deploy verifies the upload's SHA-256 against the bundled bytes (computed with
  `sha2`, no hardcoded hash). Install path/probe keyed on `PROTO_VERSION`.
- ✅ **Phase 5b — status indicator.** `RemoteState.agent_version` (set from the
  handshake on deploy success) drives a "⚡ Agent" badge next to the connection
  indicator (hover shows the version).
- ⬜ **Remaining: a real-server smoke test** (no SSH server in the build env) and
  Phase 6 polish: server-side search/filter, chunked tree streaming with live
  progress, prefetch, the in-app "remove agent" action.

Regenerating the binaries (e.g. on a `PROTO_VERSION` bump):
`cd se-agent && cargo build --release --target x86_64-unknown-linux-musl` and
`… --target aarch64-unknown-linux-musl` (set `RUSTFLAGS="-C linker=rust-lld"`
for aarch64), then copy the outputs into `native/agent-bin/`.

---

## 1. Why

Interactive SFTP browsing is **latency-bound**: every folder is a fresh
`list_dir` round-trip, and a recursive storage-analysis of a big remote tree is
thousands of them. We already softened this with a listing cache (#23.1) and a
parallel level-walk for `parallelism()>1` backends (A5/0.5.62) — but those are
workarounds for *doing the walk from the wrong side of the wire*.

The agent moves the walk to **where the files are**. The server runs Smart
Explorer's own `scanner.rs` / `analytics.rs` locally (native speed, no
per-dir round-trip) and returns a compact result:

- **Storage analysis:** the entire `SizeNode` tree is computed remotely and sent
  back once (millions of files, ~one response).
- **Browse:** a `list_dir` is still one message, but with no per-entry latency
  and optional server-side prefetch of sub-folders.
- **Search / filter:** run server-side over the live tree; stream only matches.

This is exactly Roadmap **21b ("Peer-agent Backend")**, but bootstrapped over
**SSH `exec`** rather than the P2P share transport — the practical route for a
server you already SSH into.

## 2. Scope & non-goals

- **SSH/SFTP only.** The agent needs a shell (`exec`). WebDAV / Google Drive /
  FTP cannot run a remote process → they keep the current cache+parallel path.
  The agent is an SFTP *accelerator*, not a replacement for the VFS layer.
- **Opt-in, per connection.** Never deploy a binary without explicit, visible
  consent (§6). Plain SFTP stays the default until the user enables the agent.
- **Graceful fallback.** Any failure (no exec permission, unknown arch,
  read-only `$HOME`, version/hash mismatch) silently falls back to plain SFTP
  listing — the feature can only make browsing faster, never break it.
- **Not** a general remote shell, not a daemon that outlives the session, not a
  privilege-escalation tool. One unprivileged process, the user's own account,
  lifetime bound to the SSH channel.

## 3. Transport — reuse the russh session

`sftp.rs` already holds a live `russh::client::Handle<Client>` and opens the
SFTP subsystem over one channel (`channel_open_session` →
`SftpSession::new(channel.into_stream())`). The agent rides the **same
authenticated connection**:

1. Open a **second** session channel on the existing `Handle`.
2. `channel.exec(false, "<agent-path> --serve")`.
3. Talk a length-prefixed binary protocol over the channel's **stdin/stdout**
   (stderr → diagnostics/log only).

Benefits: no extra port, no second auth, no firewall change, dies with the
channel. We keep the embedded tokio runtime that already drives russh; the
`AgentBackend` bridges blocking trait calls ↔ the async channel the same way
the SFTP backend does (`rt.block_on` per op, or a dedicated request/response
worker — see §4.3).

Keeping the SFTP subsystem open **alongside** the agent channel lets us use SFTP
for the byte-stream ops (`open_read`/`open_write`) and the agent only for
metadata/walk/search — simplest split, and file transfer is already solved.

## 4. Components

### 4.1 `se-agent` (new headless bin)

A small no-egui binary, like the existing `bench` bin, that **reuses the crate's
own modules** via `#[path]`:

- `scanner.rs` (local recursive walk) — for `list_dir` / browse,
- `analytics.rs` (`SizeNode` tree) — for `walk_tree`,
- `filter.rs` / `types.rs` — for server-side search/filter,
- a new `agent_proto.rs` (shared request/response types).

`se-agent --serve` reads framed requests from stdin, dispatches, writes framed
responses to stdout. `se-agent --version` prints `proto=<n> ver=<semver>` for the
handshake. It links **no** GUI/network crates → tiny, static-musl friendly (§7).

### 4.2 `agent_proto.rs` (shared, compiled into both app and agent)

```rust
// Length-prefixed frames: u32 LE length + body. Body = a compact binary
// encoding of these (manual writer, like analytics' tree serialisation —
// avoid pulling bincode; serde_json is too big for million-node trees).
enum Req {
    Hello { proto: u32 },
    ListDir { path: String },
    WalkTree { root: String },          // → full SizeNode tree (analytics)
    Search { root: String, query: SearchSpec },
    Stat { path: String },
    // open_read/open_write stay on the SFTP subsystem (§3).
}
enum Resp {
    Hello { proto: u32, version: String },
    Dir(Vec<VfsMeta>),
    Tree(SizeNodeWire),                 // streamed in chunks for huge trees
    Matches(Vec<MatchRow>),            // streamed
    Meta(VfsMeta),
    Err(String),
}
```

`SizeNodeWire`: the same name+size+is_dir+children shape as `analytics::SizeNode`
(names only, paths rebuilt on descent), serialised depth-first. For very large
trees, stream it in bounded chunks with a live progress counter so the UI fills
as it arrives (ties into the existing analytics progress atomics).

### 4.3 `AgentBackend` (new, implements `vfs::Backend`)

Wraps `(russh Handle, agent channel, SFTP session)`. `list_dir`, `stat` →
request/response over the agent channel; `open_read`/`open_write` → delegate to
an inner `SftpBackend`. Add two **optional** capability methods to the `Backend`
trait (default `None`/unsupported, overridden only by `AgentBackend`):

```rust
fn walk_tree(&self, root: &str, p: &analytics::Progress) -> Option<analytics::SizeNode> { None }
fn search(&self, root: &str, spec: &SearchSpec) -> Option<Receiver<MatchRow>> { None }
```

`analytics::start_analytics_scan_remote` checks `walk_tree` first: present →
one remote walk; absent → the current client-side `scan_backend`. Same for
search. This keeps every other call site untouched (the trait is the seam).

### 4.4 App wiring

- A per-connection toggle "⚡ Remote-Agent verwenden" (saved on the
  `SavedConnection`). When on, `connect` deploys (§5) and, on success, wraps the
  backend as `CachingBackend(AgentBackend{ inner: SftpBackend })`.
- On deploy failure → notice + fall back to the plain `CachingBackend(Sftp)`.
- Status chip shows "● Agent v… aktiv" vs "● SFTP".

## 5. Deploy sequence (over the SSH `exec` channel)

1. **Detect** target: `exec uname -sm` → `(os, arch)` (`Linux x86_64`,
   `Linux aarch64`, `Darwin arm64`, …). Windows servers: `exec cmd /c ver` /
   PowerShell probe — phase 2.
2. **Resolve** the matching bundled agent binary for `(os, arch)` (§7). Unknown
   target → abort to fallback.
3. **Probe** the remote for an existing, matching agent: `exec
   ~/.cache/smart-explorer/se-agent-<ver> --version`. If `proto`+`ver` match →
   skip upload.
4. **Upload** (only if missing/stale) over SFTP to
   `~/.cache/smart-explorer/se-agent-<ver>.tmp`, `fsync`, verify size + **SHA-256**
   against the bundled hash, atomic `rename` into place, `chmod 0700`.
5. **Launch:** open a fresh channel, `exec ~/.cache/.../se-agent-<ver> --serve`,
   `Hello` handshake (proto check). Mismatch → re-upload once, else fallback.

`$HOME` resolution: `exec echo $HOME` (or SFTP realpath ".") once at connect.
Read-only/quota’d home → fallback.

## 6. Security & trust (the load-bearing part)

We are **uploading and executing a binary on the server.** On a host the user
already has SSH to, this is legitimate and routine (it is precisely what VS Code
Remote-SSH does), but it must be **explicit and inspectable**:

- **Opt-in only**, per connection, off by default. First enable shows a clear
  dialog: what gets uploaded, to where, that it runs as the user, and a link to
  this doc.
- **Integrity:** the client ships the SHA-256 of each agent binary; verify after
  upload before `chmod +x`. The agent never auto-updates itself.
- **Least privilege:** unprivileged user process; no setuid; no listening
  socket (stdio over the existing channel only).
- **Provenance:** binaries are built in our normal release pipeline and their
  hashes committed alongside the release.
- **Cleanup:** a "Remote-Agent entfernen" action deletes
  `~/.cache/smart-explorer/` over SFTP. Document the path so users can audit/rm
  it themselves.
- **Known-hosts** is already enforced by `sftp.rs` (`known_hosts_accept`) — the
  channel we exec on is the same verified connection.

## 7. Cross-compilation matrix

The agent must be **statically linked** so it runs on arbitrary server distros
without libc/version surprises:

- `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` (the two that cover
  almost all Linux servers/NAS),
- (phase 2) `x86_64-apple-darwin` / `aarch64-apple-darwin`,
  `x86_64-pc-windows-gnu` (reuse the existing windows-gnu toolchain).

The agent links none of the GUI/TLS/cloud crates, so the build is small and the
musl targets are clean (no ring/aws-lc/NASM concerns — those live in the app,
not the agent). Bundle the per-target binaries (or download-on-demand from the
release feed) + their hashes with the app. Keep each agent **tiny** (strip, LTO)
since it travels over the wire.

## 8. Performance expectations

- **Storage analysis:** from `O(dirs)` network round-trips to **one** remote
  walk + a streamed tree → the headline win (minutes → seconds on big trees).
- **Browse:** one round-trip per folder as today, minus per-entry latency;
  optional server-side prefetch makes drilling feel local.
- **Search/filter:** run over the in-memory remote tree; stream matches — was
  previously impossible without downloading the whole listing.

## 9. Phases (dependency-ordered)

1. **Proto + agent core (local-testable).** `agent_proto.rs`, `se-agent --serve`
   reusing `scanner`/`analytics`, framed stdio. Test the agent end-to-end by
   piping to its stdin/stdout in-process — **no SSH needed**, fully CI-able.
2. **AgentBackend over a local child process.** Drive the agent as a spawned
   local process; implement `Backend` + `walk_tree`. Verify the analytics
   overlay runs through it against a local dir. Still no SSH.
3. **SSH deploy + exec** (russh second channel, detect/upload/verify/launch,
   handshake, fallback). The only part not testable in this sandbox.
4. **App UX:** per-connection toggle, consent dialog, status chip, cleanup.
5. **Cross-compile matrix + release bundling + hashes.**
6. **Server-side search/filter; chunked tree streaming with live progress;
   prefetch.** Polish.

Phases 1–2 deliver the architecture and are **fully testable here**; 3 is the
SSH-specific glue that needs a real server.

## 10. Open questions

- Reuse SFTP for byte streams (simplest, §3) vs. carry them on the agent
  protocol too (one channel, but re-implements transfer)? → start with SFTP.
- Tree encoding: custom binary writer (chosen) vs. a tiny dep. Keep it manual to
  avoid bloat and match `analytics`'s philosophy.
- Multiplex all ops on one agent channel (request ids) vs. one channel per op.
  → one channel + u64 request ids, async-matched by the bridge worker.
- Agent lifetime on flaky links: auto-relaunch on channel drop; idempotent
  re-handshake.
- Windows servers: defer to phase 2 (different exec/quoting + no musl).

---

# Deep-integration roadmap — make the agent the FAST PATH

**Decision (your call):** SFTP stays the baseline for any connection; once the
agent is deployed, route **everything** through it — read/write transfers,
folder-bulk transfers, server-local copy/move, search, and sync — with SFTP as
the automatic per-op fallback. Today the `AgentBackend` only overrides
`list_dir`/`stat`/`walk_tree` and *delegates* byte/mutation ops to SFTP; this
roadmap flips that so the agent implements them natively.

**Honest expectation on "faster/bigger transfers":** the real wins are
**round-trip elimination** (server-local `cp`/`mv`, one-shot recursive transfer),
**server-side parallelism**, optional **compression**, and better **read
pipelining** than russh-sftp's chunked windowing. For a single big file on a
fast link, throughput is still bounded by SSH's encrypted stream — the agent
won't magically beat that. The big wins are many-files, same-server ops, and
enumeration.

Each phase = **(a) a new agent capability (protocol op)** then **(b) the
tool-mapping** that routes the app's operations to it. Shipped together, phase by
phase. Bundled musl binaries get rebuilt whenever `agent_proto` changes.

## Phase 0 — protocol v2 foundation (prerequisite for transfers)
- **Capability:** framed, multiplexed, streaming protocol. Every frame carries a
  `req_id: u64`; responses and stream chunks are tagged with it. New frames:
  `DataChunk{req_id, bytes}`, `End{req_id}`, `Err{req_id, msg}`, `Cancel{req_id}`,
  `Progress{req_id, done, total}`. A bridge worker thread in `AgentBackend` pumps
  the channel and routes frames to the waiting op by id (blocking ops `recv` on
  their id); enables concurrent ops + progress + cancel on one channel.
- **Tool-mapping:** none yet (foundation). Bump `PROTO_VERSION` → agents
  re-deploy on mismatch (already handled).
- **Testable:** fully (in-process / socket).

## Phase 1 — Read transfer (open / download via agent)
- **Capability:** `Read{path}` → streamed `DataChunk`s (large buffer, pipelined);
  optional gzip for compressible types; `Progress`.
- **Mapping:** `AgentBackend::open_read` / `open_read_id` → agent stream (was
  SFTP delegation). Speeds up: **opening a remote file** (download-to-temp),
  **Ctrl+C remote→local**, **drag remote→local**, **"Herunterladen nach…"**.

## Phase 2 — Write transfer (upload / save via agent)
- **Capability:** `Write{path}` ← streamed chunks; server writes to a temp,
  `fsync`, atomic rename; `Progress`.
- **Mapping:** `AgentBackend::open_write` → agent stream. Affects: **save-back of
  an edited remote file**, **paste/drop upload into a remote folder**, **drag
  local→remote**, **"Neu" file creation** on a remote.

## Phase 3 — Server-local mutations (instant)
- **Capability:** `Copy{src,dst}`, `Rename{src,dst}`, `Remove{path,recursive}`,
  `Mkdir{path}` — executed natively on the server.
- **Mapping:** `AgentBackend::{copy_file, rename, remove_file, remove_dir,
  mkdir_all}` → agent. Biggest single-op win: **remote→remote copy/move on the
  SAME server** becomes instant (today it streams down then back up through a
  temp). Also delete / rename / new-folder in one op.

## Phase 4 — Recursive bulk transfer (folders, one session)
- **Capability:** `GetTree{root}` / `PutTree{root}` — stream an entire subtree
  (entries + bytes) in one framed session; the server reads files in parallel
  (rayon) and pipelines; `Progress` over the whole job.
- **Mapping:** the copy engine (`copy.rs`) for remote **folders**; drag/paste of
  folders; "Herunterladen nach…" on a folder. This is the "größere Transfers"
  win — no per-file round-trip.

## Phase 5 — Server-side search / filter
- **Capability:** `Search{root, spec}` (name / glob / regex + size/date) →
  streamed matches.
- **Mapping:** the omnibox folder-search and the active filter, when on a remote,
  run **server-side**; matches stream into the list. Find on huge remote trees
  without client-side enumeration.

## Phase 6 — Hashing & sync via the agent
- **Capability:** `WalkHashed{root, mode}` — the sync signature (size+mtime, and
  md5/native hash on demand) in one server walk; reuse Phase 1–4 ops to apply.
- **Mapping:** `bisync::{walk_files, apply}` use the agent when the endpoint's
  connection has it — teach `connect::resolve_endpoint` / the sync runner to
  deploy + use the agent (sync currently builds a *fresh, plain* SFTP backend).
  Also unlocks the **duplicate finder (A2)** for remotes (hash without download).

## Cross-cutting (all phases)
- **Fallback:** any agent op error → fall back to the inner SFTP op (per-op), as
  today. The feature can only speed things up, never break them.
- **Progress + cancel:** stream byte/file counts into the existing copy/transfer
  progress UI; `Cancel{req_id}` wired to the cancel buttons.
- **Security/versioning:** unchanged — opt-in deploy, SHA-256 integrity,
  `PROTO_VERSION` re-deploy, `release/v*` bundled musl binaries.
- **Testability:** phases 0–5 are locally testable (in-process child / socket);
  phase 6's real sync needs a live server.

**Sequencing:** 0 → 1 → 2 → 3 give "everything interactive goes through the
agent" (open/save/copy/move/delete). 4 adds bulk folders, 5 search, 6 sync. Each
is independently shippable and falls back to SFTP until done.
