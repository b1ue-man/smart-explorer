# Gotchas & Dead Ends (read before "improving" these)

Hard-won, verified findings. Each cost real debugging. Don't re-tread them.

## AI agent plugin (`se mcp`)

- **Long-lived `se` processes block desktop updates.** The updater helper waits
  at most 30 s for every process running the app or `se` image (Toolhelp on
  Windows, `/proc/<pid>/exe` on Linux) after writing `daemon.stop` = `stop`.
  Anything that keeps an `se` process alive for hours (the MCP server) must
  watch that request and exit (`cli/mcp/update_stop.rs`); a value already
  present at start is stale and ignored.
- **Two MCP lifecycles.** Codex starts servers with `initialize` and protocol
  `2025-06-18`; Claude Code's v2 runtime probes the stateless `2026-07-28`
  `server/discover` first. rmcp 3.5.1 serves both on one connection; a server
  limited to one generation breaks one host. rmcp needs Rust 1.88 and is built
  without its macros (tool schemas are plain JSON in `cli/mcp/catalog.rs`).
- **Never print to stdout from code `se mcp` reaches.** The server writes the
  protocol to a duplicate of stdout and points stdout at stderr, but tests and
  new code should still use stderr for diagnostics.
- **Codex filters MCP server environments** (Unix: `HOME`, `PATH`, `USER`,
  `LANG`, `TERM`, `TMPDIR`, `TZ` and a few more). `se plugin install codex`
  passes `XDG_DATA_HOME`, `XDG_RUNTIME_DIR` and `DBUS_SESSION_BUS_ADDRESS`
  explicitly; without them `se` would read the default app data and could
  start the worker without the user session bus.
- **Claude Code plugins cannot ship permission rules**; users allow
  `mcp__plugin_smart-explorer_se__*` themselves. Local-marketplace plugins are
  loaded from their folder at every session start (no cache copy), which is
  why `se mcp` refreshes the generated files after an update.

## Build / toolchain

- **GNU toolchain only.** Built with `x86_64-pc-windows-gnu` + Strawberry GCC as
  linker (not MSVC). `export PATH="$USERPROFILE/.cargo/bin:/c/Strawberry/c/bin:$PATH"`.
- **comctl32 `TaskDialogIndirect` = instant crash.** Do NOT use
  `rfd::MessageDialog` — it statically imports `comctl32!TaskDialogIndirect`,
  which only exists in comctl32 v6. Without an embedded v6 manifest (the GNU
  toolchain doesn't add one), the loader binds v5, the import is unresolved, and
  the process dies at load with exit `0xC0000139` (STATUS_ENTRYPOINT_NOT_FOUND)
  **before any Rust runs** (so no crash.log). Use the `confirm_yes_no` helper
  (`MessageBoxW`) instead. `rfd::FileDialog::pick_folder` is fine (no TaskDialog).
  Diagnose load-time "entry point not found" by diffing `objdump -p` imports of
  the broken vs last-working exe.
- **`russh` crypto backend.** Its default is `aws-lc-rs` (needs NASM/CMake, breaks
  on GNU). Use `default-features = false` + `ring`. Still verify it compiles
  before building on top of it — see REMOTE_LAYER_PLAN §5. Since 0.63
  `Handler::check_server_key` receives `PublicKeyOrCertificate`, and russh's
  default host-key list still ends with `ssh-rsa` (SHA-1): `sftp/core/session.rs`
  builds its own `Preferred` without it (2026-10-16).
- **Post-quantum TLS without aws-lc.** rustls' ring provider has no
  `X25519MLKEM768`; `rustls-post-quantum` and iroh's `tls-aws-lc-rs` both pull
  aws-lc-sys. The Share endpoint therefore adds its own hybrid group
  (`share/core/pq_kx.rs`: RustCrypto `ml-kem` 0.3, the API russh 0.64 compiles
  against, plus ring's X25519 via rustls) through `Builder::crypto_provider`.
  The hybrid must stay first in `kx_groups` and keep `hybrid_component`, or
  old peers pay a HelloRetryRequest; ML-KEM first in share and secret
  (RFC 10024, published August 2026, formerly draft-ietf-tls-ecdhe-mlkem; code
  point 0x11EC), unlike the older Kyber draft. The all-zero X25519 secret and a
  malformed ML-KEM encapsulation key (FIPS 203 §7.2 check, done by
  `EncapsulationKey::new_from_slice`) end in `illegal_parameter`.
- **Credential store needs its protector at runtime (Linux/Android).** Sealed
  records open only with the data key named in `secrets-v1/keyring-v2`. With
  `systemd-creds` the store is bound to the host secret
  (`/var/lib/systemd/credential.secret`, user scope also to UID and
  machine-id) like DPAPI on Windows; a reinstalled OS or a changed machine-id
  leaves the records closed. Root uses the system scope only when
  `/var/lib/systemd` is on the store's file system (container volumes).
- **`app-key-file` is the weakest protector.** When neither the Android Keystore
  nor `systemd-creds` can be used, the key file `secrets-v1.kek` lies beside
  `secrets-v1/` in the same app data folder. It only keeps a copy of the store
  directory alone closed; a backup or an offline copy of the whole data folder
  holds both, and only disk encryption protects that. `se doctor` names the
  protector and says so. The protector is chosen once and never changes by
  itself: an Android store created while the Keystore failed stays on
  `app-key-file` even after the Keystore works again (`SecretsKey.kt` retries
  the Keystore three times at 250 ms intervals before giving up).
- **A locked store is not an empty store.** With a descriptor `keyring-v2` that
  names a protector the host cannot serve, sealed records stay closed: no new
  key, no overwritten record, no new Share identity (the identity loader returns
  the error instead of creating one). `se doctor` then reports the store as
  locked (exit code 1) with `plaintext_records` unchanged. Seal-and-port runs at
  worker start, Android `init` and `se doctor` without any secret access
  (`creds::port_secret_store`); only a record an older version wrote later, or one
  with wrong modes or an extra hard link (counted as `unreadable_records`), stays
  plaintext until the next read or start. The doctor line has grown after
  `plaintext_records=`: parse it as `key=value` pairs. The tests override the
  effective UID through a `thread_local` in `linux_file_store_fs.rs` (test builds
  only) instead of needing a second account.
- **`@revoked` outranks an own pin.** `sftp/os/shared/known_hosts.rs` reads the
  OpenSSH files first: a matching `@revoked` line refuses the key even when
  `known_hosts_sftp.txt` pins it (OpenSSH: "never accepted"). Without a
  revocation an existing pin decides alone, as before. Among several OpenSSH
  files the order is revoked, trusted, changed, unknown, because OpenSSH
  accepts when any matching line has the presented key. A revocation behind the
  first 1 MiB of a file is not read. `@revoked` lines are host-bound (`@revoked *`
  covers every host); `@cert-authority` lines are skipped.
- **Host key type and the pin (D9, checked 2026-10-16).** A pin
  (`host:port SHA256:…`) stores the fingerprint of the key the server presented,
  not its type. The client offers Ed25519, ECDSA nistp256/384/521, RSA (SHA-512,
  SHA-256) in that order and takes the first one the server knows; the list is
  the same in russh 0.61.2 (0.5.180) and 0.64.1, so a server with several keys
  presents the same type before and after the update and its 0.5.180 pin keeps
  matching. Guard: unit test `luecken_task_host_key_algorithm_order_matches_0_5_180`.
  `ssh-rsa` with SHA-1 comes only in a second attempt for already pinned hosts.
  Unchanged since 0.5.180: a server that later adds a more preferred key type is
  presented with that type and the pin counts as changed (OpenSSH avoids that by
  preferring the algorithms of known keys; TODO `LUECKEN-SSH-KEYORDER`), and
  `russh` runs with `default-features = false`, so the feature `rsa` is off and
  neither lock contains the package `rsa`; whether RSA host keys can be
  negotiated at all was not examined.
- **A program list is not a sandbox, and the caller's `PATH` never counts.** A
  grant with `--allow-program` allows only argv starts of the listed programs,
  found by absolute path or in the fixed search path of the host (never in the
  `PATH` the caller sends), with only `LANG`, `LC_*` and `TERM` taken from the
  caller. Arguments stay free, so a listed shell, `env`, interpreter or command
  runner (`find`, `git`, `ssh`, `xargs`, `awk`, `sudo`) is a full shell. The same
  file name is required in addition to the same resolved file, because busybox
  and toybox serve many programs from one binary.
- **Older versions read a narrowed grant as switched off.** A grant with a program
  list or another account is stored with `enabled` false plus `restricted_enabled`
  (`share/core/exec_policy_wire.rs`); a version up to 0.5.180 ignores the second
  field and loses the narrowing when it saves the profile, but never turns the
  grant on. Its grant journal refuses an open entry of such a grant (`validate`
  expects `enabled` true for `pending_apply`). Do not "simplify" the encoding to
  plain fields: 0.5.180 would read them as the full shell of the worker's
  account.
- **Audit log classes.** `exec-audit.log` (starts, ends, refused starts) and
  `exec-audit-rejected.log` (refused authentications, penalty refusals) rotate
  separately so that rejections of invented EndpointIds can never push out what
  authorized devices did. A test or script that looks for `auth_rejected` or
  `penalty_refused` in `exec-audit.log` finds nothing; `se share exec log` shows
  both.
- **`smb2` pin and its lock side effect.** The SMB backend pins `smb2 = "=0.26.0"`
  (pure Rust, RustCrypto; the crate ships breaking minors quickly). Its
  `aes-gcm ^0.11.0` requirement lifted the shared `aes-gcm` from `0.11.0-rc.4` to
  `0.11.1`, which russh's `ssh-cipher` also uses (2026-09-26); the SFTP device tests
  cover that path. Resolve new lock entries with `cargo metadata` (no compile) and
  check them per target with `cargo tree --offline --locked --target …`.
- **PowerShell 5.1 + cargo.** cargo writes progress to stderr, which PS 5.1 turns
  into error records → trips `throw` in scripts even on success, and the tool may
  report failure on exit 0. Run cargo via the Bash tool (`2>/dev/null`); do
  file/version/makensis steps as separate simple PS calls. Quote makensis args:
  `& $makensis "/DVERSION=x.y.z" "installer.nsi"`.
- **Release builds must stay inside the memory budget.** The desktop crate once
  exhausted an 8-GiB host with full LTO and displaced unrelated processes.
  Keep ThinLTO, eight codegen units, `CARGO_BUILD_JOBS=1`, and non-incremental
  builds pinned in every canonical release leaf. The top-level wrapper also routes the
  large Linux tree through `native/run-release-memory-bounded.sh`, which uses a
  3-GiB high/4-GiB hard cgroup limit plus at most 1 GiB of swap when systemd
  scopes are available. Do not weaken or make these limits ambient overrides;
  ordinary diagnostic builds may use separate Cargo settings.
- **Embedded SSH-agent binaries retain source paths.** Unmapped Cargo registry
  paths make an otherwise identical static-musl agent differ between a local
  build and GitHub's `/home/runner` build, tripping the intentional exact-byte
  freshness guard. Keep the encoded repository/`CARGO_HOME` path remaps and the
  forced target directory in `native/build-agent-bundles.sh`; it intentionally
  rejects ambient `RUSTFLAGS` variables. Rebuild and commit both
  `native/agent-bin/` payloads instead of weakening the CI comparison.
- **Bundled SQLite for sync state.** `rusqlite` is built with `bundled` so
  incremental mirror state does not depend on a system SQLite DLL. Keep the GNU C
  toolchain available for release builds, and do not drop `bundled` unless the
  Windows and WSL feed build both prove the replacement path.
- **Linux Exec talks to systemd through target-specific `zbus`.** Keep `zbus`
  under the Linux Cargo target section with `default-features = false` and the
  Rust-only Tokio/blocking API features. The Exec provider calls
  `StartTransientUnit` directly; replacing it with `systemd-run` would trust a
  shell/PATH boundary, while moving `zbus` to shared dependencies would burden
  and risk the Windows GNU build.
- **cargo-audit quick-xml advisories.** With iroh 1.3.0 `plist 1.10.1` uses
  `quick-xml 0.42.0`, so plist is no longer the reason (checked in
  `native/Cargo.lock`, 2026-10-16). `quick-xml 0.39.2` is now held by the direct
  dependency `native/Cargo.toml` (`quick-xml = "0.39"`) and by `wayland-scanner`,
  and `quick-xml 0.30.0` by `zbus_xml 4.0.0`. CI explicitly ignores only
  `RUSTSEC-2026-0194` and `RUSTSEC-2026-0195` on the main native lockfile
  (`.github/workflows/build.yml`, whose comment still names the old plist
  path). Remove those ignores as soon as both paths can move to
  `quick-xml >=0.41.0`; do not broaden the audit suppression.
- **suppaftp is pinned to `>= 10.0.2` because of RUSTSEC-2026-0271** (FTP
  command injection through CR/LF in command arguments of `login`, `cwd`,
  `mkdir`, `rmdir`, `rm`, `rename`, `retr`, `put_file`, `site` and
  `custom_command`, fixed in 10.0.2; RustSec entry dated 2026-08-18, read
  2026-10-16). Since 8.0.0 the TLS feature names its provider: use only
  `rustls-ring` with `default-features = false`; `rustls-aws-lc-rs` would pull
  aws-lc back in. Traps of the 6.3 → 10.x step, each handled in `ftp/core`:
  `list::File::from_str` now also tries MLSx, and its MLSx parser accepts any
  text as a file name, so LIST rows go through `ListParser::parse_posix` and
  `parse_dos` only (unreadable rows stay errors and omissions); `abort` reads
  one closing reply and a second only after 426, so after a transfer that had
  already finished the server's reply to ABOR can still be queued, and
  `FtpReader` hands the connection back healthy only after a PWD probe answers
  257; every data stream must end through `finalize_retr_stream`,
  `finalize_put_stream` or `abort`, otherwise the next data command fails with
  `FtpError::DataConnectionAlreadyOpen`. suppaftp checks only CR and LF; NUL
  and the server's own PWD reply (sent back in the restoring CWD) are checked by
  `errors::command_path` / `errors::working_dir`. The lock entry was edited
  without cargo (TODO `LUECKEN-SUPPAFTP`).

## egui / UI

- **Ctrl+C / Ctrl+X / Ctrl+V are NOT delivered as key events.** egui's winit
  backend turns them into semantic `Event::Copy/Cut/Paste` — and for a FILE
  clipboard (CF_HDROP, no text) it emits NEITHER a paste event NOR a key event,
  and when idle triggers no repaint at all. So `consume_key(V)` and in-frame
  polling both fail. Fix (in `app.rs`): a dedicated background thread polls
  `GetAsyncKeyState` ~30×/s, gated to our foreground window, and wakes the UI.
  Clipboard keybindings can't be unit-tested — verify in the running GUI.
- **`ui.columns` does not clip.** The wide table bled into the other split-screen
  pane. Use per-pane `allocate_ui_at_rect` + `ui.set_clip_rect(rect)` + a painted
  divider (see `ui_central`).
- **Tabs use a swap model.** The active tab's state lives in the `App` fields;
  inactive tabs are parked in `TabState` and `mem::swap`'d in/out
  (`swap_with_tab`). Split-screen renders the non-focused pane by swapping its
  tab in around the `ui_table` call, then swapping back. Any new per-view state
  must decide: per-tab (add to `TabState` + `swap_with_tab`) or global.

## Windows remote-drive integration

### Dokany remote-drive boundary

- **Disable IPC batching before every filesystem creation.** The official
  2.3.1.1000 DLL truncates the `0x1000` option to a byte when selecting its
  consumer, while the driver receives batching enabled. Trailing requests in
  a batch are then lost. `DokanOptions::prepare_for_create` clears only that
  option before every foreign call, including reused options; ordinary
  multithreading stays enabled. Do not replace this with single-thread mode or
  clear it after creation. See [evidence and acceptance criteria](MOUNT_BATCHING.md)
  before changing the guard or selecting a replacement runtime.
- **A no-follow open is not evidence of a link.** Windows attribute queries
  use `FILE_OPEN_REPARSE_POINT` even on ordinary files and directory roots.
  Accept that flag and inspect actual metadata; keep rejecting remote symlinks.
  Blanket flag rejection breaks `GetFileAttributesExW`, and .NET Framework's
  fallback can hide the original error behind "file not found". The exact
  callback trace and platform contract are recorded in [the mount evidence](MOUNT_BATCHING.md).
- **This is a real user-mode filesystem, not CfAPI.** The selected backend root
  is exposed as a Windows drive letter through Dokany callbacks. Do not add a
  Cloud Files sync-root registration, placeholders, hydration callbacks, or
  `cldflt.sys` state to this path. Cryptomator is the useful UX analogy; Smart
  Explorer owns the backend proxy and whole-file cache rather than a vault.
- **Dokany is exact-versioned but the reviewed MSI is installer input.** The
  supported runtime is the official Dokany 2.3.1.1000 release. Its library
  header defines `DOKAN_VERSION 231`, while its kernel interface header defines
  the distinct `DOKAN_DRIVER_VERSION 0x0000190` (decimal 400).
  Delay-load only `%WINDIR%\System32\dokan2.dll` with
  `LOAD_LIBRARY_SEARCH_SYSTEM32`, resolve the bounded symbol table, require
  `DokanVersion()` to return DLL API 231, and require `DokanDriverVersion()` to
  return kernel protocol 0x190/400. Never
  search the application directory, current directory, `PATH`, registry, or a
  caller-controlled path, and never silently accept a merely ABI-compatible
  older/newer driver. There is no link-time Dokany import and no DLL or driver
  beside the app. The recommended NSIS installer does, however, embed the
  reviewed official `Dokan_x64.msi` as a standard-selected optional offline
  component; portable/auto-updated users obtain the same pin through the GUI or
  `se drive install-runtime`. Keep the base install per-user and invoke UAC only
  for the system-wide MSI. Plain `/S` must skip it; only
  `/S /INSTALLDOKANY=1` opts a silent setup in. Never uninstall the shared
  Dokany runtime with Smart Explorer. The pin in `native/dokany-runtime.nsh` is
  version `2.3.1.1000`, DLL API 231, driver protocol 0x190/400,
  `https://github.com/dokan-dev/dokany/releases/download/v2.3.1.1000/Dokan_x64.msi`,
  9,269,248 bytes, SHA-256
  `69ff8cb37bfec3a75921c85ffd1c6370b50a9ec4ecef2cf3a009d488dcbf5465`.
  The official project states that signed release drivers are provided; use of
  that official runtime needs neither Developer Mode nor `TESTSIGNING`.
  Primary sources checked 2026-07-22:
  [2.3.1.1000 release](https://github.com/dokan-dev/dokany/releases/tag/v2.3.1.1000),
  [tagged README](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/README.md),
  [tagged API header](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/dokan/dokan.h),
  [tagged driver header](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/sys/public.h),
  [version-query implementation](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/dokan/version.c),
  [Dokan API](https://dokan-dev.github.io/dokany-doc/html/group___dokan.html).
- **Treat the Dokany installer as hostile until every boundary passes.** Rust
  permits HTTPS, at most two redirects, only the exact source URL or GitHub's
  `release-assets.githubusercontent.com` final host, and at most pinned-size +
  one byte. Require exact size/SHA-256; open only a non-reparse regular file
  without write/delete sharing; hash that locked handle and pass the same handle
  to `WinVerifyTrust` with chain revocation. Open each normalized parent below
  the stable volume/share root as a direct non-reparse directory handle without
  share-delete, and hold the file plus parent chain across launch so an attacker
  cannot swap a pathname component before the elevated reopen. Then launch
  System32 `msiexec.exe` through UAC `runas` with `/passive /norestart
  ADDLOCAL=DokanDriverFeature INSTALLDEVFILES=0`. Exit 0 still requires the exact
  DLL-API-231 and driver-protocol-0x190/400 postcheck. The automatic GUI/CLI path
  runs only for an absent runtime or unavailable driver; it must not replace or
  downgrade a genuinely incompatible installed shared runtime. Release fetchers
  consume the same manifest, NSIS
  embeds the verified MSI, and the release wrapper must extract and compare it.
  Keep `third-party/dokany/` notices and GPL/LGPL/MIT texts installed, but do not
  publish the MSI as a separate feed or Release asset.
- **Keep the daemon/host authority split.** The daemon resolves the saved
  connection, credentials, active fallback and exact root. `RootedBackend`
  rejects traversal and link-like ancestors, strips provider object IDs and
  sanitizes errors. The daemon spawns its current executable with the exact
  private `--mount-host <MountId>` argument; that isolated process receives a
  rooted backend proxy over loopback using distinct one-use launch/backend
  capabilities plus a session capability; it must never receive the daemon's
  global token, account, endpoint, credential material, or unrestricted
  backend root. Keep `env_clear`, loopback validation, bounded frames and
  fail-closed EOF behavior. GUI + daemon + one isolated mount host is the
  expected process set for one drive: the daemon owns the shared `Backend` and
  active remote session, so closing a GUI tab must not terminate the mount.
- **Peer roots require a live daemon probe and a principal/root lease.** Offer
  concrete Share roots as `/Label` and `/Verbindungen/<connection>`; the
  aggregate `/` is synthetic and read-only. GUI discovery must ask the daemon's
  active remote `PeerBackend`, not infer guarantees from local labels. Mount
  admission obtains a lease bound to the authenticated device principal, exact
  root, export identity and authorization epoch. Direct/relay route changes and
  physical QUIC reconnects reacquire current Presence routes and preserve that
  lease. Never let a different identity, root or policy epoch inherit the old
  capability; those changes require Retry/remount. Stopping a share or
  revoking/changing its authorization is a synchronous admission barrier: new
  operations fail closed, active leases become invalid and a later re-share
  requires Retry/remount. One already admitted operation may finish;
  multi-stage writes must recheck before flush and promotion.
- **Cache reusable observations, never safety decisions.** Mount metadata
  depth defaults to 2 and is configurable from 0 through 4 in the GUI or with
  `--metadata-depth`. Load only the complete root snapshot synchronously; fill
  deeper complete snapshots breadth-first in bounded 8-target batches and
  rotate at most 16 refresh targets every 20 seconds. Reserve root, oldest-attempt
  fairness and a most-recent demanded directory before the remaining demand
  backlog; servicing a recent directory must not bury it behind an earlier full
  vault scan. Keep the directory cache
  within its 128-MiB retention budget; the five-second point-stat cache has a
  separate 16-MiB bound and the mount daemon ancestor cache has 64 MiB. These
  budgets govern disposable retention, not valid directory or entry counts.
  Share simultaneous loads even when retention fails; do not turn expiry into
  one remote stat per child of a previously observed directory. Wire frames
  still have a 64-MiB safety bound, not an arbitrary 50,000-entry limit. The
  configurable clean content cache remains separate from dirty/recovery bytes.
  Mutations invalidate affected paths. Open/create admission, conflict checks,
  replacement and all write decisions must continue to query live state.
- **Root authority and read/write mode are separate gates.** Strict mode is the
  default even for read-only mounts. The deployed Linux Agent must launch with
  the exact root via `--serve-root`, bind every root component with
  `openat2(RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS)`, and enter Landlock ABI
  3+ before worker threads start; ABI 2 cannot constrain `truncate`/`O_TRUNC`.
  Google Drive's parent-ID hierarchy is also technically confined. Plain SFTP,
  Local/UNC, WebDAV and FTP need explicit trusted-root admission. Peer/Share
  propagates the concrete remote backend/root result: an Agent-confined export
  can remain `Enforced`, while Local/UNC/plain SFTP remains `Unverified` through
  Peer because its check-to-operation race is unchanged. Trusted mode keeps
  serialized validation but cannot close that external symlink/junction race.
  A fallback must be reevaluated for this capability as well as write semantics.
  Sources checked 2026-07-21:
  [Landlock API](https://docs.kernel.org/userspace-api/landlock.html),
  [`openat2(2)`](https://man7.org/linux/man-pages/man2/openat2.2.html).
- **Strict SFTP mounts never inherit the browsing fallback.** A saved SFTP
  connection must have Agent enabled, and deployment plus the protocol-v9
  `--serve-root` handshake must succeed. Surface any failure; never silently
  construct a plain-SFTP mount. Ordinary browsing may retain the established
  SFTP backend only when Agent deployment itself fails; after a successful
  handshake an operation error is surfaced, not replayed blindly. Address
  candidates are deduplicated and interleaved
  IPv6/IPv4 with 250-ms staggering while known-host identity remains the
  original hostname. Keep typed connect/handshake/channel/exec timeouts and
  stdout/stderr capture independently bounded to 64 KiB each. Keep one SFTP
  subsystem for bootstrap, verification and fallback; exec channels run probes
  and the agent.
- **Agent identity is content-addressed, not merely version-addressed.** The
  remote path includes protocol v9 plus the full SHA-256 of the embedded bytes.
  Upload to an exclusive random temporary name, verify its SHA-256 through the
  same SFTP subsystem, then promote/chmod. Re-read and hash the installed file
  before every exec, including reconnect; a matching `--version` string alone
  is insufficient.
- **RW is a three-part contract, not a generic “backend can write” boolean.** A
  mounted root must prove `create`, `replace`, and `namespace_replace` before a
  read-write host starts. `create` means atomic exclusive ownership through
  `open_write_new`, never stat-then-create. Local/UNC and the SSH Agent
  implement all three write primitives, although Local/UNC still require the
  root trust opt-in above.
  Plain SFTP deliberately reports no complete RW staged capability. Its single
  SFTP-v3 subsystem uses standard `SSH_FXP_RENAME`, which is no-replace; do not
  open a second extension subsystem or upgrade a stat+rename race or shell
  command into an atomic guarantee. The aggregate Share root `/` is read-only;
  concrete `/Label` and `/Verbindungen/<connection>` exports delegate capability
  discovery to their exact backend/root. A fallback can therefore change the
  safe answer and must make RW fail conservatively rather than weaken it
  silently; RO remains possible only if the independent root
  gate above also passes or trusted-root mode was explicit. Source checked
  2026-07-21:
  [SFTP-v3 rename](https://datatracker.ietf.org/doc/html/draft-spaghetti-sshm-filexfer#section-6.5),
  [russh-sftp API surface](https://github.com/AspectUnk/russh-sftp).
- **Application writes are whole-file transactions.** Materialize a regular
  file into the mount-specific local spool; persist the dirty record and sync
  it before the first local mutation; on flush upload to a unique staging path;
  then promote with the declared backend primitive. This is what makes repeated
  Obsidian-style truncate/write/flush/close cycles and editor
  temp-file-to-replacing-rename saves viable. A flush after every edit means a
  complete remote upload after every edit. Never stream a partial editor write
  directly onto the destination object.
- **Conflict checks reduce risk; they are not universal CAS.** Compare the
  opening baseline (provider ID, size, mtime, and content MD5 when supplied)
  before staging and again immediately before promotion. Revalidate both
  source and destination for replacing rename. Unless a backend exposes a
  conditional commit, a small TOCTOU window remains between the last stat and
  promotion. Do not document or code this as perfect concurrent-write
  prevention. Never retry an ambiguously dispatched promotion: once a remote
  namespace change may have committed, return filesystem success where Windows
  requires it, surface `Conflict`, and retain the recovery journal/spool.
- **Never unlink an ambiguously owned staging spelling.** After an exclusive
  create crosses Agent/daemon/Peer boundaries, a lost Ready/final ACK or failed
  promotion may leave a hidden stage. A check-then-unlink is still vulnerable
  if another actor moves the owned object and reuses that name. Retain it until
  a future stable-ID/lease garbage collector can prove identity; leaking one
  hidden stage is preferable to deleting foreign content.
- **Recovery state outranks tidy shutdown.** Persisted state is explicitly
  `Clean`, `Required`, or `Unknown`; legacy records without trustworthy status
  map to `Unknown`, never clean. Under the exclusive cache lease, audit local
  journal/spool state at daemon startup and again in the host before opening the
  remote backend. A connection timeout therefore cannot hide local recovery.
  Dirty/conflicting writes and
  quarantined deletes remain under the mount ID in `mount-cache`; startup/Retry
  replays only provably safe work. An eject or host exit must not erase a spool
  merely because the callback returned. Preserve journal rotation/torn-tail
  validation, the exclusive reparse-safe cache lease, and path-wide
  delete-on-last-handle semantics.
- **A host exit code is not an actionable diagnosis.** Preserve the mount
  host's terminal recovery/conflict status and append only bounded stderr/process
  context. Surface DLL API and driver-protocol mismatches independently, name
  strict-root or staged-RW admission failures, map an invalid Peer lease to
  Retry/remount, and say when recovery data must remain. Only a provably clean
  drive-manager entry may be offered for removal.
- **Portable `open-temp` recovery must survive Electron launchers.** Atomically
  write a marker only after a successful download. Delete only complete empty
  legacy manifests with no payload; malformed/truncated state fails closed, and
  genuine sessions are startup notices rather than app errors. Never interpret
  a finished `ShellExecute` launcher as editor completion: Obsidian may forward
  to its existing single-instance process. A declared file remains recoverable
  while temporarily absent during atomic save/delete/rename, and any `NotFound`
  in that mutation path retains the prior marker.
- **Remote latency is application latency.** Reads first materialize an entire
  file, and each changed flush uploads it entirely. Backend operations retain
  their transport deadlines; supervised metadata and whole-file I/O callbacks
  can extend Dokany's timeout while pending. Ambiguous mutations are never replayed.
  The reported free space is the local spool's lower bound, not the remote
  quota. Keep unrelated files parallel but serialize one file/namespace
  mutation.
- **The surface is intentionally narrower than NTFS.** Windows file attributes
  and timestamps cannot be set, ACL/security writes, alternate data streams,
  open-by-ID and actual reparse-point access are unsupported, and remote symlinks are
  hidden/rejected rather than followed. `GetFileSecurity` returns
  `STATUS_NOT_IMPLEMENTED` so Dokany synthesizes a current-user descriptor.
  Never fabricate durable remote semantics for metadata the backend cannot
  preserve.

### Dokany error mapping (0.5.183)

- **Only Dokany's own table reaches Explorer.** Dokany 2.3.1.1000 turns a Win32
  code into an NTSTATUS only for the codes in `dokan/ntstatus.i`; every other code
  becomes `STATUS_ACCESS_DENIED`, which Explorer shows as "Access denied" for an
  error that has nothing to do with rights. `dokany_status` passes only the listed
  codes and returns `STATUS_IO_DEVICE_ERROR` for everything else
  (`native/src/mount/os/windows/callback_status.rs:232-252`,
  `native/src/mount/os/windows/dokany_status_table.rs:1-12`). An error kind without
  any Win32 code also ends as `STATUS_IO_DEVICE_ERROR` (`callback_status.rs:242`).
- **Host and network unreachable have their own codes.** `HostUnreachable` maps to
  `ERROR_HOST_UNREACHABLE` (1232, "The network location cannot be reached."),
  `NetworkUnreachable` to `ERROR_NETWORK_UNREACHABLE`, and a move between two
  shares or devices of the device tree to `ERROR_NOT_SAME_DEVICE`
  (`callback_status.rs:264-285`).

## Linux FUSE drives (0.5.183)

- **No `auto_unmount`.** The host opens the session with `Config::default()`,
  two read loops, owner-only access (`SessionACL::Owner`, so `user_allow_other` is
  not needed) and `nodev,nosuid,noatime`; it does not set `auto_unmount`
  (`native/src/mount/os/linux_os/fuse_host.rs:1-10`). A crashed host leaves a mount
  that the daemon removes (`native/src/daemon/os/linux_os/mount_cleanup.rs`).
- **Three ways a session ends.** An unmount by the host, an unmount outside the host
  (file manager eject, `fusermount3 -u`, `se drive unmount`), and an abort while still
  mounted (kernel request timeout or `fusectl`). Only the last is a failure; its
  reason names the kernel timeout when one is set (`fuse_host.rs:12-18`).
- **The kernel request timeout is a hard limit.** A value above 0 in
  `fs.fuse.default_request_timeout` lets the kernel abort a drive whose request takes
  longer. Se does not work around it; `se drive runtime` reports the value
  (`native/src/mount/os/linux_os/fuse_runtime.rs:6-9`, `:201-203`).
- **Open descriptors can keep a host alive.** After a short grace period the host
  cuts such connections through `fusectl`; their files are then published as after a
  Dokany unmount (`fuse_host.rs:19-21`). Without `fusectl` mounted, only the delayed
  unmount remains (`native/src/mount/os/linux_os/fuse_abort.rs:75`).
- **`setattr` covers only truncate, chmod, chown and utimensat**
  (`native/src/mount/os/linux_os/fuse_ops_attr.rs:1-2`). The filesystem implements
  `lookup`, `getattr`, `mkdir`, `unlink`, `rmdir`, `rename`, `mknod`, `open`, `read`,
  `write`, `flush`, `release`, `fsync`, `opendir`, `readdir`, `releasedir`, `statfs`
  and `create` (`native/src/mount/os/linux_os/fuse_fs.rs:245-476`). Symbolic links,
  hard links, extended attributes and file locks have no handler in that file; what
  the `fuser` library answers for them is not verified here.
- **A FUSE drive needs `fusermount3` with setuid root** and `/dev/fuse` for the
  user (`fuse_runtime.rs:1-12`). The runtime report names the package command per
  distribution; sandboxes (Flatpak, Snap) and WSL are reported separately.

## Exec account service (0.5.183)

- **Windows accounts logged on without a password have no network credentials.**
  The service (LocalSystem with `SeTcbPrivilege`) logs a local account on with
  `MsV1_0S4ULogon` and logon type `Network`
  (`native/src/share/os/windows/exec_account_s4u.rs:1-6`). Network drives and EFS
  files are therefore not available to a command that runs as that account.
- **Windows commands run in session 0 without a window.** The relay runs in session 0
  inside a job object of the service; `PI_NOUI` suppresses message boxes
  (`native/src/share/os/windows/exec_account_spawn.rs:1-4`, `:44-50`). The startup
  info gives the relay an empty desktop name, so it connects to a window station of its
  own logon session (`exec_account_spawn.rs:318-326`).
- **The token probe depends on two policies.** The probe treats an administrator as
  elevated only when `EnableLUA` is on and `LocalAccountTokenFilterPolicy` is not 1
  (`native/src/share/os/windows/exec_account_tests.rs:431-441`).
- **The PAM service has no `pam_systemd`.** Account units use `PAMName=smart-explorer-exec`
  with `account` checks of the distribution, `pam_limits`, `pam_loginuid` and
  `pam_keyinit`, but no `pam_systemd`; otherwise processes of a job would leave their
  unit (`native/src/share/os/linux_os/exec_pam_service.rs:1-10`, `:76-89`).
- **Account units are `Type=notify`, not `Type=exec`.** With `PAMName=` the PAM session
  process that systemd 249 and 255 fork keeps the descriptor that reports the `execve()`
  open (closed only from systemd 256), so a `Type=exec` unit never leaves "activating" and
  is killed after `TimeoutStartUSec` ("timeout, exit status 9"). The supervisor sends
  `READY=1` to `NOTIFY_SOCKET` once it holds its connection; a PAM refusal still ends the
  unit before that with its exit status. The worker's own units stay `Type=exec`
  (`native/src/share/os/linux_os/exec_account_unit.rs`, `exec_supervisor_stdio.rs`).
- **`ExitType=` is not sent.** systemd 249 (Ubuntu 22.04) refuses a unit with an unknown
  property; `ExitType` exists only from systemd 250, where its default `main` is what
  the unit needs (`native/src/share/os/linux_os/exec_systemd.rs:100-103`).
- **The root worker refuses expired accounts and forced password changes.** Before a
  unit starts, the account's shadow entry is read, and an expired account or one that
  waits for a password change is refused (`native/src/share/os/linux_os/exec_account.rs:1-12`).
- **Program copies are not updated by the updater.** The Linux copy lives at
  `/usr/local/sbin/smart-explorer-exec`; the Windows copy at
  `%ProgramFiles%\Smart Explorer\exec-accounts\se.exe`. Run
  `se share exec accounts update` after an update (see `docs/RELEASING.md`).

## Device tree drives (0.5.183)

- **GTK and KDE bookmark files have no lock.** Both files are read once and replaced
  through a temporary file in the same folder. A change another program writes between
  the read and the replace is lost (`docs/oberflaechen/laufwerke.md`, section 4;
  `native/src/mount/os/linux_os/bookmarks_gtk.rs`, `bookmarks_kde.rs`).
- **Device folder names follow fixed rules.** Characters are replaced per dialect,
  the length limit is 255, and an empty name becomes `Gerät`. Two devices whose names
  compare equal (Windows ignores case) both get ` (<6 hex>)` from the SHA-256 of their
  key; a device named `Räume` becomes `Räume (Gerät)` so it cannot collide with the room
  folder (`native/src/mount/core/device_tree_names.rs`).
- **A taken drive letter has no fallback.** On Windows the device tree needs its letter
  free; no other letter is chosen automatically (stated in the DRV-M6 reading; not verified in this run).
- **Child journals survive switching the tree off.** `off` unmounts the tree and keeps
  the caches of the device folders and `children.json`; unsaved changes wait for a
  connection and are published without the user's action when the device returns
  (`docs/oberflaechen/laufwerke.md`, section 5).

## Screen streaming (0.5.183)

- **Pull buffers start with a 16-byte head.** `screenPull` writes the head (time, flags,
  length) at offsets 0 to 15 and the payload from offset 16 (`native/src/screen/os/android/viewer_sink.rs`,
  `HEADER_BYTES`). The decoder's input buffer must leave those 16 bytes free.
- **Return −3 copies nothing.** When the destination is too small, `pull_into` writes only the
  payload length into bytes 12 to 15 and returns −3; the app then pulls again into a larger
  buffer (`viewer_sink.rs`, `pull_into`; `A/ui/screen/ScreenDecoder.kt`, `pullLarger`).
- **`KEY_LOW_LATENCY` is a hint.** The decoder sets it on every configuration; a setter that
  throws `IllegalArgumentException` is ignored (`ScreenDecoder.kt`, `configure`). The app's
  minimum API level is 30 (`android/app/build.gradle.kts`), where the key exists.
- **A stalled viewer queue is emptied, not played back.** A picture older than `STALL_US`
  clears the queue and requests a key frame; until that key frame arrives, the app receives
  nothing (`viewer_sink.rs` module header).
- **Tile pictures are converted from BGRA to RGBA** in the core (`viewer_sink.rs`,
  `bgra_to_rgba`), so the app draws them without a byte-order step.
- **The Android host has no keyboard events.** Its input capabilities are pointer, text,
  touch and seven system actions, and `keyboard` is false (`native/src/screen/os/android/bridge_backend.rs:103-115`).
  Without the accessibility service the host offers no input at all (`bridge_backend.rs:98-100`).
- **Window lists are bounded by bytes, not entries.** `MAX_WINDOWS_BYTES` (1 MiB, protocol
  v1) limits the list; `WindowList::bounded` drops whole entries, never truncates titles
  (`native/src/screen/core/wire.rs:54`, `window_list.rs:171-190`).
- **The relay drops at 512 packets or 1 MiB per client.** The vendored relay's send queue
  is `PER_CLIENT_SEND_QUEUE_DEPTH` = 512 (`vendor/iroh-relay-1.0.0/src/protos/relay.rs:40`)
  with a 1 MiB per-client byte budget (`vendor/iroh-relay-1.0.0/src/server/queue_budget.rs:8`).
  The host keeps a relay path below 768 KiB (`native/src/screen/core/pacing.rs:17`).
- **A viewer reads at most four picture streams at once.** One running and one new picture group
  for each kind (`video`, `preview`); a stream that is reset is stopped (`native/src/screen/core/gop_stream.rs:24-27`).
- **Windows Graphics Capture is asked once per process.** The interop factory check runs once,
  without a version check (`native/src/screen/os/windows/capture_wgc.rs:54-63`).
- **PipeWire is loaded at run time.** `libpipewire-0.3.so.0` is opened with `libloading`
  (`native/src/screen/os/linux_os/pipewire_abi.rs:2`, `:163`); a missing library ends the
  portal path with `pipewire_missing`.
- **OpenH264 is pinned, not verified against Cisco's checksums.** Cisco publishes only MD5 sums,
  so the integrity rests on the pins of version 2.6.0 for win64 and linux64 (size and SHA-256 of the
  compressed file, SHA-256 of the library) in `native/src/screen/os/native_ffi/openh264_pins.rs:1-7`.
  The license is shown before every download.
- **Old peers close with `session_limit`.** The host sends no session-count refusal any more
  (R15c); the viewer still maps `session_limit` for older hosts (`native/src/screen/core/wire_hello.rs:273`).
- **LightDM greeters focus every mapped window.** They run without a window manager and give
  the keyboard to each newly mapped window, except override-redirect windows and windows of type
  NOTIFICATION (unity-greeter 24.04.1 `src/unity-greeter.vala` `focus_upon_map`;
  lightdm-gtk-greeter `wm_window_filter` also skips COMBO and TOOLTIP). `WM_HINTS.input=False`
  does not help. The screen indicator therefore takes the type NOTIFICATION when no EWMH window
  manager announces itself (`_NET_SUPPORTING_WM_CHECK`), otherwise every key of a controlling
  viewer lands in the indicator (`native/src/app/os/linux_os.rs`, `indicator_window_type`).
  Measured 2026-10-28 on a LightDM/unity-greeter X server: an ordinary window and one with the
  input hint off took the focus; a NOTIFICATION window did not.

## Cryptomator vaults (0.5.183)

- **Vault comparison lives in memory, in the service only.** One entry per vault job and
  per checked site; it is never written to disk (`native/src/vault/os/shared/compare_cache.rs:2`).
  A restarted service answers `compare_stale`.
- **Merge limits.** A text merge is offered only up to 512 KiB (`native/src/vault/os/shared/compare_merge.rs:26`);
  one line of the service carries at most 2 MiB after JSON escaping (`compare_merge.rs:23`).
- **Removed cipher files stay in a backup folder.** Repairs and removals first move the
  cipher files to `<app data>/vault-removed/` (`native/src/vault/os/shared/health_fixes.rs:4`,
  `compare_files.rs:6`).
- **Vault folder listings are cached for 20 seconds** (`native/src/vault/os/shared/dir_cache.rs:7`).
  Vault backends are not wrapped in `CachingBackend`: that wrapper appears only for remote backends (`native/src/mobile/os/shared/pool.rs:2`); the vault path uses its own listing cache (Stand U29).
- **The window setting is a separate file.** „Beim Öffnen einhängen“ is stored for all
  sites in `<app data>/vaults/ui.json` as `{"schema":1,"mount_on_open":true}`; a missing or
  unreadable file means `true` (`native/src/app/core/vault_ui_settings.rs:1-4`).
- **Sleep and screen-lock signals depend on the platform.** Linux locks on the screen-lock
  signal only when the desktop environment reports the session lock (`native/src/vault/os/linux_os.rs:38-39`).
  Windows listens with a hidden window; assumption A1 (the message reaches that window)
  is not verified (`native/src/vault/os/windows.rs:9-18`).
- **An open file holds the background lock.** While a vault file is open, the background
  delay does not lock that site; closing the last file lets the next tick lock it
  (`native/src/vault/os/shared/session_timer.rs:59-72`).
- **Freedesktop thumbnails are removed on Linux.** At the end of a drive and on lock, the
  worker removes thumbnails of files in the drive folder (`native/src/vault/os/linux_thumbnails.rs:44`).
  Windows thumbnails can only be switched off globally (Stand U14).
- **The Android saved password is an envelope.** The core stores `android-kw1:` followed by
  Base64url of IV, ciphertext and tag, bound to the site with AAD; a changed lock screen
  invalidates it (`native/src/mobile/os/shared/domains/vault_keys.rs:26`;
  `android/app/src/main/java/app/smartexplorer/android/ui/vaults/VaultSavedPassword.kt:158-161`).
- **Cancelling key derivation discards the key.** The unlock returns `canceled` and does
  not register the vault (`native/src/vault/os/shared/unlock.rs:59`).
- **Android vault screens are protected by a flag and by the input method.** `FLAG_SECURE`
  is set per window through `SecureWindow` and on the sheets with `SecureOn`
  (`android/app/src/main/java/app/smartexplorer/android/ui/vaults/SecureWindow.kt:16`); the password
  fields exclude autofill (`NoAutofill`, `VaultUnlockSheet.kt:163`).
- **Preview images need an open provider path.** The comparison detail decodes with `inSampleSize`
  from the content URI of the own provider (`VaultCompareDetail.kt:66-70`). A preview is offered only
  while the vault is unlocked and shared under „Ordner für Apps bereitstellen“; otherwise it shows
  „Keine Vorschau“ (Abweichung 9 of U31, Stand SEC point 11).
- **Password minimum is 8 characters** (`native/src/vault/os/shared/password.rs:29`).
- **Recovery keys are not supported.** The core shows the note instead of a word list
  (`native/src/vault/os/shared/manage.rs:27`).
- **Vault sync jobs have fixed and selectable fields.** A replica is placed next to the vault
  for a stated reason; the seeding rule is one rule; K20 and K21 stay withheld without a known
  site; a file of the same size in the same second without a server hash is a known gap, covered
  by a test (Stand U20).
- **Fallback copies are sealed.** The recovery manifest of fallback copies seals its field texts
  with the site's log key (`native/src/app/os/shared/remote_helpers/recovery_manifest.rs:49-56`).
- **Carried over from U05 (not re-verified in this run):** the scrypt work buffer, the
  CMAC subkeys in AES-SIV and decrypted names held as `String` are not overwritten after use
  (plan hand-over from U05). `zeroize` is used in `cryptomator/core/keys.rs` and `kdf.rs`.

## Transfer engine tempo (0.5.183)

- **Single files do not stop a run.** A failed file is reported and the run continues; the
  exit code is 1 when any file failed (Stand U49, `native/src/cli/engine_copy.rs`).
- **Numbering at a remote target is an error.** The engine reports it instead of renaming
  (Stand U49).
- **Windows access prompts are not asked by the engine.** A local source for which the
  access gate would ask for read rights stays on the legacy path (`access_prompt_possible`);
  the engine never asks in the terminal or in the MCP service (Stand U49).
- **Sensors fail quietly.** Each read error of a system sensor is `None`; Android does not read
  `/proc/stat` (assumption, Stand U50, `native/src/transfer/os/shared/sensors.rs`).
- **The hints file holds only hashes.** `transfer-hints.json` keeps the SHA-256 of a random salt
  and the connection identity, never a host, account, path or name; it is `0600` in a `0700`
  folder (`native/src/transfer/os/shared/hints.rs:1-12`).
- **Packages are not in the time accounts yet.** Packets do not book their time (Stand U50).
- **The memory budget never revokes.** A buffer held by an operation stays reserved when the
  budget shrinks (`native/src/transfer/os/shared/memory.rs`).
- **`NO_BUFFERING` needs a known memory figure.** Without the available memory the flag is
  never set; with it, a file of at least a quarter of that memory is copied without the system
  cache (`native/src/copy/os/shared/copy_flags.rs:25`, `:47-58`).
- **Windows compressed traffic stays off.** `REQUEST_COMPRESSED_TRAFFIC` is switched off
  (`native/src/copy/os/shared/copy_flags.rs:19`, E13-S11).
- **Parallel ranges need a quiet, known drive.** They run only with permits the flow handed out,
  an empty queue and a drive known not to rotate; an unknown rotation (btrfs, network, `/sys`
  unreadable) never runs them in parallel (Stand U51, `native/src/copy/os/linux_range.rs`).
- **Clipboard copies run in parallel.** Providing remote files for other programs loads folder
  files under the job id `CLIPBOARD_JOB` (`native/src/transfer/os/shared/clipboard_parallel.rs:19`).
- **MD5 runs in its own thread per copy.** The sync hash starts one thread per copy from 1 MiB;
  creation errors stay inline (Stand U52, `StreamHasher`).
- **WebDAV sync uploads save one PROPFIND.** The stage time is not asked again after the stage
  has been checked (`native/src/webdav/core/extensions.rs`, Stand U52).
- **SMB compounds must not follow reparse points.** `read_file_compound*` follows them, so the
  engine sends its own requests; a data reparse point (dedup) is read through the single path
  (Stand U46).
- **FTP has no command pipelining.** STOR and RETR leave before the TCP connection of the data
  channel is set up (suppaftp); the next command waits for the reply to the previous one, as
  RFC 959 requires (`native/src/ftp/core/data_overlap_server.rs`).
- **Packet streams read with their own loop.** `read_exact` retries `Interrupted`; packet
  readers therefore use their own loop (Stand U46).
- **Parts are sparse files.** A part sets the file length first (`set_len`) and seeks to the
  written position on resume (`native/src/transfer/os/shared/engine/download.rs:73`, `:84`).
- **SFTP encryption order.** AES-GCM comes first only when the CPU offers AES instructions
  through `ring`; otherwise ChaCha20 stays first (`native/src/sftp/core/session.rs:341-342`).
- **SFTP range writes need a safe folder chain.** The folder chain from the stage to `/` must
  belong to the account or to root and be closed to group and others (or sticky and root);
  `is_regular()` of `russh-sftp` tests a single bit and takes links for regular files, so the
  engine checks the file type itself (Sicherheitsantwort S7, Stand U48).
- **FTP stage probe is skipped only for random names.** It is skipped only for names
  `.se-upload-<16 hex>` made by `getrandom`; the fallback value of `random_hex` starts with `w`,
  which is not a hex digit and so never looks random (`native/src/transfer/os/shared/engine/util.rs:28-36`).
- **The Share host removes unfinished range stages** at the end of the connection, after 300 s
  without progress and at the next start; an agent range stage stays on the channel that opened it
  (Stand U53).
- **`SE_SHARE_QUIC_CC` and `SE_TEST_NO_LZ4` are measuring switches**, not settings
  (`native/src/share/core/keepalive.rs:105`).
- **SFTP open files per channel are capped at 64.** The window of a batch uses
  `min(open-handles, 64)`; an OPEN with a plain failure status counts as congestion, a specific
  status as a file error (Sicherheitsantwort E13-S13, `native/src/sftp/core/batch_ops.rs:52`).
- **One TCP stream per SSH connection limits long, wide links.** Channels of one SSH connection
  share its TCP stream and kernel buffer, so more channels do not raise the rate there; only extra
  SSH connections do (Sicherheitsantwort E13-S15, Punkt 15; `native/src/sftp/core/channel_pool.rs:20-24`).
- **Extra SSH connections need the stored origin.** They exist only through
  `SftpBackend::connect_with_origin` with a stored password or key; `SftpBackend::connect` assumes
  an interactive login and never opens one (`native/src/sftp/core/backend.rs:38-41`,
  `link_rules.rs:41-55`, `native/src/connect/os/shared/connector.rs:91-107`).
- **The channel limit counts channels in setup.** A refusal learns the limit from the ready and the
  in-setup channels (`native/src/sftp/core/channel_pool.rs:57-59`, `:168-170`, `:355`).
- **`queue/rotational` is not the truth on virtual disks.** Cloud VM disks and loop devices often
  report 1. A buffered copy does not gain from parallel ranges there (writeback sets the depth),
  so only the unbuffered path for very large files probes one thread against two, once per device
  pair, without `fdatasync` (direct requests are done at the device when they return;
  `native/src/copy/os/linux_probe.rs`).
- **`statx` goes through its syscall number.** The `statx` symbol needs glibc 2.28; the release
  baseline is 2.17, so `STATX_DIOALIGN` is read via `libc::syscall` (`native/src/copy/os/linux_direct.rs`).
  `O_DIRECT` through `/proc/self/fd` may be refused on Android (f2fs with fscrypt, restricted /proc):
  every refusal before the first byte falls back to the buffered path.
- **`Detail::Kinds` applies to FTP LIST rows only.** Publishing a stage lists names and kinds; LIST
  rows are then not refined with SIZE and MDTM; the refinement at `metadata.rs:242` is guarded by `!mlsx`, so MLST/MLSD rows never take it
  (`native/src/ftp/core/listing_detail.rs:13-21`, `native/src/ftp/core/metadata.rs:144-146`, `:242`,
  `native/src/ftp/core/staging.rs:96`).
- **Measurement statistics exist only in debug-assertion builds.** `SE_TEST_NO_SHARE_STATS` and
  `share-quic-stats.jsonl` work where debug assertions are on (development builds and the `speed`
  profile), never in release builds or unit tests (`native/src/share/core/transfer_meter.rs:23-27`,
  `native/src/share/core/quic_stats_hub.rs:103`).
- **A netem root rule on `lo` also shapes local IPC.** The test program's TCP hop to the Share
  daemon crosses the same rule; the speed stage keeps the daemon's ports out of it with a two-band
  `prio` root (`native/rdt_task/speed_net.py:41-44`, `:107-124`).
- **`net.core.rmem_max` caps the 7 MiB receive buffer request.** netwatch 0.19.3 (`native/Cargo.lock:4401-4402`)
  asks for `SOCKET_BUFFER_SIZE = 7 << 20` on its UDP socket (crate source on docs.rs, line number not
  determined). Linux limits `SO_RCVBUF` of unprivileged processes to `net.core.rmem_max` (socket(7);
  not verified in this repository). The reporter reads that ceiling (`native/src/share/core/quic_stats_proc.rs:150-168`).

## Share / local network

- Google Drive allows several same-name siblings. `GDriveBackend::list_dir`
  keeps the canonical copy (newest `modifiedTime`, then smaller id) under the
  plain name and renders every further sibling as `Name [drive-id <8-char id
  prefix>]`; `find_child`, cached-id validation, `stat` and rename resolve the
  marker (literal same-named objects win over the marker reading). Do not
  reintroduce a raw duplicate listing: every path-based walker rejects it.
- Removing a Direct contact without a durable denial is not a removal: the
  peer still holds our Direct code, so its reciprocal repair (`apply_reciprocal_direct_peer`)
  or an auto-accepted access request re-creates the contact within seconds.
  `removed_direct_peers` (schema 8) is that denial; only `PairingOrigin::UserPairing`
  may clear it. Keep every new pairing path honest about its origin.
- The worker snapshot replaces the GUI's profile copy on every poll. While a
  reciprocal repair is in flight the worker still reloads the persisted
  profile (without reconfiguring the service) so a removed peer does not
  resurface from a stale in-memory snapshot.
- LAN presence announces a hashed id, never the Iroh key, and matching a
  sighting to a contact is routing evidence only; the session proof and TLS
  node pin remain the authorization. Link-local IPv6 candidates carry the local
  scope (`[fe80::1%3]:port`) and go through `net::parse_candidate`, because
  `SocketAddr::from_str` rejects the `%scope` form.
- Iroh 1.3 (since 0.5.181) resolves relay host names with its own DNS client
  (`n0-dns-resolver`), not with the operating system's resolver, which the
  signaling connection uses. A device whose Iroh lookups fail reaches the
  signaling server, publishes no relay, and is unreachable from outside its
  network. `share/os/shared/relay_dns.rs` therefore installs a resolver that
  asks the operating system first and keeps Iroh's for TXT and as a race. The
  worker logs `Relay nicht erreichbar (…)` after 20 s without a home relay.
- App, drives and `se` open a peer through the worker, which tries for up to
  45 s. Their IPC wait must outlast that (`ipc_open_client.rs`, 75 s): a
  shorter socket timeout reported its own text (Windows: „…Gegenstelle … nicht
  richtig reagiert…“, os error 10060) instead of the worker's cause.
- Uplink sharing is deliberately opt-in and conservative: a spoofed
  announcement could only trigger ICS/NetworkManager sharing on a link that
  the helper itself verifies as router-less, and never on a routed network.
  Every platform facility (ICS installed, `SharedAccess` startable, task
  registered, NetworkManager owning its bus name, `dnsmasq` present, polkit)
  is probed and reported as `nicht verfuegbar: <Grund>`; never assume it.

## Windows shell integration

- **You cannot replace other apps' Open/Save dialogs system-wide.** No registered
  default file picker exists; the dialogs are created in-process per app. Only
  DLL injection could do it (unsupported, breaks on UWP/sandboxed, AV-flagged).
  Directory Opus refuses to. Out of scope, permanently.
- **Registry default-file-manager override does NOT redirect folder double-clicks
  on Win11.** Writing `HKCU\Software\Classes\Directory\shell\open\command` (with
  no DelegateExecute/ddeexec) was supposed to shadow the Folder-class handler.
  **Live test disproved it**: with the exact keys written by hand, ShellExecute
  "open" on a folder still launched Explorer. Win11 routes folder activation
  through the Folder class's `DelegateExecute` COM handler, which wins; in-window
  navigation never consults the verb at all. The toggle shipped in 0.3.4 and was
  REMOVED in 0.3.5 (with a startup self-heal). The ONLY thing that actually
  redirects double-clicks is a background window-hook (FileExplorerInterceptor
  style) — invasive, flashy, declined. `shell_register.rs` keeps the reversible
  context-menu verb + the (proven-correct, reversibility-tested) registry
  helpers, but the default-manager feature is gone.
- **Win11 MAIN/modern context menu needs a SIGNED package.** A legacy registry
  verb (our `OpenInSmartExplorer`) only ever appears under "Show more options".
  Reaching the main menu requires an `IExplorerCommand` COM handler **plus**
  package identity (sparse/MSIX) **signed with a cert the machine trusts**. A
  self-signed cert means asking every user to trust it (non-starter / needs
  admin). Not shippable for an unsigned per-user app without buying a
  code-signing cert (e.g. Azure Trusted Signing). The Rust COM DLL itself is
  feasible; the signing+packaging is the wall. **Update (0.5.2):** the COM half
  is now BUILT — `native/explorer-command/` is a cdylib that implements
  `IExplorerCommand` + `IClassFactory` and exports `DllGetClassObject` /
  `DllCanUnloadNow`; it compiles + links to a real PE DLL on windows-gnu. The
  sparse-package `AppxManifest.xml` is written too. Everything up to **signing**
  is done; signing remains the wall. See `docs/WIN11_CONTEXT_MENU.md`.

## Updater

- **An elevated setup owns what it creates by Administrators** (run as
  administrator, or UAC off: the elevated token's default owner). The
  installation check (`mcp_launcher/os/windows/security.rs::check`) therefore
  trusts the same principals as the ancestor check: the object's owner, the
  running account, Administrators, SYSTEM and TrustedInstaller. Until
  0.5.188 it trusted only owner/SYSTEM/Administrators, so the user's own
  inherited full access on the new root was a "foreign write grant" and every
  elevated install failed (`runtime_acl_foreign_write_grant: …\Programs\Smart
  Explorer`). `.github/workflows/windows-install-check.yml` installs a
  published release elevated on a fresh runner and prints the helper's result.
- **The update helper is a GUI-subsystem program** in release builds; its
  stderr reaches nobody when the setup starts it with `ExecWait`.
  `--install-bundle` failures are written to `%APPDATA%\smart_explorer\updater.log`,
  which the setup's failure message names. PowerShell starts a GUI-subsystem
  program without waiting: use `Start-Process -Wait -PassThru` with
  redirected output to see its result.

- Per-user install under `%LOCALAPPDATA%\Programs\Smart Explorer\`; app data in
  `%APPDATA%\smart_explorer\`. A normal check downloads the OS-specific
  app/updater/`se` trio to app data, verifies every SHA-256, and atomically
  persists a staging manifest. It does **not** replace an installed file or stop
  a process. The update dialog requires explicit consent; **Later** retains the
  same verified staging across restart, while **Discard** removes it.
- Applying is a hash-bound transaction run by the separate updater helper. The
  helper revalidates itself and all staged payloads, waits for the exact parent
  PID to exit, requests a graceful daemon stop, and refuses to continue while a
  matching Smart Explorer binary is still running. Do not reintroduce force-kill
  behavior to make an update appear successful.
- **The downloaded helper must understand the previous app's launch protocol.**
  v0.5.119 installs the new feed helper and invokes it with only the legacy app
  target/staging/status fields plus the staged app hash; the normally installed
  helper path carries no helper-hash argument. Removing the explicit legacy
  parser makes updates fail before replacement with a missing SHA argument. The
  bridge must stay fail-closed: require the staged SHA, never downgrade a request
  containing any modern-only flag, and never UAC-relaunch an updater. This also
  applies to the modern protocol: an elevated helper would otherwise launch the
  GUI with administrator privileges. Require the installer when cleanup or
  replacement needs elevation.
  The first helper must wait for the exact v0.5.119 parent without a fixed
  timeout and serialize duplicate workers for the same installed target. Retire
  an older/equal request only when a target-key/version/app-SHA completion
  receipt proves the winner; rebase a newer request on that verified target and
  attempt it without downgrading. If the launched winner blocks a queued newer
  worker, fail visibly and retain the payload. Before changing the target,
  durably create an exclusive target-keyed intent that binds the requested
  version, baseline SHA, and staged SHA. If a helper dies after replacement but
  before status/receipt publication, a later stale worker must recover that
  exact intent or fail closed; it must never adopt the unproven binary as a new
  baseline. The replacement app syncs a private receipt sibling after its first
  GUI frame, atomically publishes it, and completes a nonce-bound two-way
  loopback handshake. Receipt publication is the irreversible commit point;
  the helper retains rollback state and the intent until then. Do not run
  recovery launch while the exact target process is already running without a
  receipt; wait for its receipt and then fail closed rather than creating a
  duplicate. Do not run abandoned-staging cleanup in that acknowledged launch;
  a queued v0.5.119 worker can still own another verified payload.
  Visible update status/error state must be prepared before launching the new
  or restored app. Preserve the exact-v0.5.119 argv, prelaunch-state,
  durable-intent, receipt, serialization/rebase, rollback, path-alias, and
  target-drift regressions.
- Before replacement, the helper verifies and durably archives the outgoing app
  with a SHA-256 sidecar. It rejects aliases between executable, staging, and
  bookkeeping paths and prepares checked sibling files. Existing targets must
  remain continuously addressable: Windows uses `ReplaceFileW` with a verified
  backup, while Linux creates a verified backup and atomically renames over the
  target. Replace updater, `se`, then app so an interruption always leaves a
  runnable old or new GUI. A failed replace or failed acknowledged launch
  restores the previous targets and status, then attempts to relaunch the
  verified old app.
  Length checks alone are not an acceptable replacement for the immediate
  SHA-256 revalidation at each process/replacement boundary.
- Release publication is also transactional: build into isolated staging,
  verify all six Windows/Linux feed payloads and hashes plus the Windows build
  manifest, promote with rollback backups, and move `version.txt` last. A
  Windows-only partial build must never mutate the shared feed or ancillary
  release files; direct runs require explicit, separate `-Feed` and
  `-ReleaseOutput` paths.
- **Rollback** archives the running version on startup and again as a mandatory
  precondition to replacement, but archives still accumulate only **going
  forward**. Remote rollback discovery uses GitHub Releases first and a legacy
  `release/vX.Y.Z` branch fallback. The atomic `update_pinned.txt` file pauses
  automatic forward checks after manual rollback; applying a newer staged update
  clears the pin only after the new app has launched successfully.

## Windows file names and verbatim paths

- **`std::fs::symlink_metadata(dir.join(name))` is not safe for every listed
  name on Windows.** Win32 name resolution maps `C:\dir\NUL`, `nul.txt`,
  `con.log`, … to the *device*, strips trailing dots and spaces and rejects a
  few characters, so re-opening a child by its joined path returns device
  metadata or fails. The scanner, `collect_recursive` and `LocalBackend::list_dir`
  therefore take `DirEntry::metadata()` (the enumeration record; `lstat` on
  Unix, no extra system call on Windows).
- **Rust's std hands short paths to Win32 unchanged.** `maybe_verbatim` only
  builds a `\\?\` path above 248 UTF-16 units, so `DeleteFileW("C:\dir\nul")`
  hits the device. `vfs/os/windows/verbatim.rs` builds the verbatim form itself
  whenever a component is one `types::win32_name_issue` flags, folding `.`/`..`
  lexically (verbatim paths never do) and keeping every other component
  literal. Ordinary paths keep their plain spelling so shell-facing callers
  never see the prefix.
- **The Recycle Bin parses names like Explorer.** `trash::delete` goes through
  `IFileOperation`, which cannot address a reserved name; the GUI renames such
  an entry to a Win32-safe unique sibling first (`_nul.txt`, `report`) and
  recycles the renamed item. Permanent deletion needs no detour.
- **`RtlIsDosDeviceName_U` semantics** (mirrored by `types/core/win32_names.rs`):
  strip trailing dots and spaces, cut from the first period, trim trailing
  spaces, then compare with `CON`/`PRN`/`AUX`/`NUL` or `COM`/`LPT` plus one
  digit (Microsoft's current guidance also lists `0` and the superscripts
  `¹ ² ³`). `COM10` and `nul_x` are ordinary names.
- **The agent protocol forwards errors as text only.** `Frame::Err(String)` is
  shared with the committed `se-agent` bundles, so it cannot grow a kind field.
  Daemon-proxied Share and SSH backends therefore reached callers as
  `io::ErrorKind::Other`, and the CLI copy planner (`transfer_plan`, tree
  guards) that branches on `NotFound` refused every upload to a new remote
  path. `agent/core/agent_error.rs` rebuilds `NotFound`, `AlreadyExists` and
  `PermissionDenied` from the forwarded `std::io::Error` text (`(os error 2)`,
  `(os error 3)`, `(os error 80|183)` and the English messages); anything else
  stays `Other` with its message. Prefer `try_exists` over stat-and-catch when
  a new call needs existence only.

## Windows removable media (FAT/exFAT)

- **A FAT volume's root directory has file ID 0** (fastfat
  `FatGenerateFileIdFromDirentOffset`: "Our root always has fileid 0"; FAT
  answers `FileIdInfo` with `ERROR_INVALID_PARAMETER`, so the identity comes
  from `GetFileInformationByHandle`). Every pin of a USB stick root therefore
  failed with "provider has no directory identity". `local_access/os/windows/
  directory_identity.rs` accepts ID 0 only for the volume root: its
  `FileNameInfo` is exactly `\`, and its identity is the volume serial
  (`windows-root:<serial>`). Any other object with ID 0 keeps failing.
- **FAT/exFAT folders have no durable identity** (Microsoft: a FAT file ID is
  derived from the directory entry's position and can change). The VFS
  `directory_identity` of a root therefore follows `stable_identity` like its
  children (NTFS/ReFS only), so a folder reservation on a stick creates the
  folder directly instead of through a stage that would need an identity.
- **Measured on Windows Server 2025 (fsutil):** the root of a FAT32 and of an
  exFAT volume both have file ID 0; their subfolders have nonzero IDs.
- **A move renames its source into a quarantine before copying a byte**
  (`copy/os/shared/safe_file.rs`). Without a stage proof (targets without a
  durable identity, such as a stick) the transfer receipt checks that
  quarantined source, not the then empty original path
  (`file_recovery_doubt.rs::quarantined_source_unchanged`); the raw file
  index is not compared after the rename, because FAT derives it from the
  directory entry.
