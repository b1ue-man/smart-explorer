# Android portability audit: Share / net / agent / agent_proto / quickshare

**Purpose.** Read-only source audit for planning an Android (`aarch64-linux-android` /
`x86_64-linux-android`, `target_os="android"`, `target_family="unix"`) build of the
Rust core. Lists every conditional-compilation site in the assigned modules, what
`target_os="android"` resolves to, Linux-only/glibc-only APIs that would or would not
compile, the Share/ runtime pieces and their OS-adapter dependencies, and the
`native/Cargo.toml` per-target dependency gates. No verdict on the overall
architecture is given; findings are facts with `file:line`.

**Files read** (globs):
- `native/src/share/mod.rs`, `native/src/share/core/**`, `native/src/share/os/**`
- `native/src/net/mod.rs`, `native/src/net/core/**`, `native/src/net/os/**`
- `native/src/agent/mod.rs`, `native/src/agent/core/**`
- `native/src/agent_proto/mod.rs`, `native/src/agent_proto/core/**`, `native/src/agent_proto/os/**`
- `native/src/quickshare/mod.rs`, `native/src/quickshare/os/shared/quickshare.rs`
- `native/Cargo.toml`

Out-of-scope but referenced by name (not read, flagged as unresolved dependencies):
`crate::support_dirs`, `crate::creds`, `crate::vfs`.

---

## 1. Conditional-compilation sites (`#[cfg(...)]`, `#[path=...]` module selection)

`target_os="android"` matches `not(windows)`, `unix`, `not(target_os="linux")`,
`not(any(windows, target_os="linux"))`. It does **not** match `windows` or
`target_os="linux"`.

### 1.1 `native/src/share/mod.rs`

| site (file:line) | cfg / selection | Android resolution | consequence | remedy candidates |
|---|---|---|---|---|
| share/mod.rs:158-163 `identity_lock` | `cfg(not(windows))`→`os/linux_os/identity_lock.rs`, `cfg(windows)`→`os/windows/identity_lock.rs` | `os/linux_os/identity_lock.rs` selected | compiles fine (see §2.1) | none needed |
| share/mod.rs:230-235 `platform_exec` | `cfg(target_os="linux")`→`os/linux_os/exec.rs`, `cfg(windows)`→`os/windows/exec.rs` | **no arm matches** → module `platform_exec` is undefined | **missing symbol**: `share/core/exec_platform.rs:27,34,82,86,143` reference `super::platform_exec::*` unconditionally (not behind any `#[cfg]`) → unresolved-module compile error on android | add an `os/android/exec.rs` (or a portable no-op `ContainedExec`) selected by a third arm, e.g. `#[cfg(any(target_os="linux", target_os="android"))]` for a shared "container-exec" path, or `#[cfg(not(any(windows, target_os="linux")))]` android stub returning `ExecProviderStatus{available:false,...}` from `provider_status()`/`Err(Unsupported)` from `ContainedExec::prepare` |
| share/mod.rs:288-293 `system` | `cfg(windows)`→`os/windows/system.rs`, `cfg(not(windows))`→`os/linux_os/system.rs` | `os/linux_os/system.rs` selected | compiles fine (see §2.2) | none needed |

`exec_platform.rs` itself (the OS-neutral wrapper) has no `#[cfg]` guarding its
`super::platform_exec::*` calls, so the missing-module problem above is a hard
build break for every android target, not a degraded feature.

### 1.2 `native/src/net/mod.rs`

| site (file:line) | cfg / selection | Android resolution | consequence | remedy candidates |
|---|---|---|---|---|
| net/mod.rs:1-6 `interfaces` | `cfg(windows)`→`os/windows/interfaces.rs`, `cfg(target_os="linux")`→`os/linux_os/interfaces.rs` | **no arm matches** → module `interfaces` undefined | **missing symbol**: `net/mod.rs:92` `pub fn gather_interface_facts()` calls `interfaces::gather_interface_facts()` unconditionally → unresolved-module compile error on android | add an `os/android/interfaces.rs` using `if-addrs` only (already a dependency, already used identically in `share/os/shared/system.rs:3`) instead of `/proc/net/route` + sysfs `operstate`, since those Linux paths are unreliable/permission-gated on stock Android |
| net/mod.rs:7-12 `platform` (`connect_impl`/`disconnect_impl`) | `cfg(windows)`→`os/windows.rs`, `cfg(target_os="linux")`→`os/linux_os.rs` | **no arm matches** → module `platform` undefined | **missing symbol**: `net/core/net.rs:19` `use super::platform::{connect_impl, disconnect_impl};` is unconditional → unresolved-import compile error on android | this is UNC/network-drive credential mapping (`WNetAddConnection2W` on Windows, a stub `Err(Unsupported)` on Linux) — per task scope ("mounting Festplatten nicht notwendig") an android arm can reuse the existing Linux stub body (`os/linux_os.rs:3-16`) verbatim under a combined `cfg(not(any(windows, target_os="linux")))` (or an explicit android arm) |
| net/mod.rs:14-19 `ics`/`nm_shared` | `cfg(windows)`→`os/windows/ics.rs`, `cfg(target_os="linux")`→`os/linux_os/nm_shared.rs` | neither compiled | feature absent, no missing-symbol risk (only referenced from within the matching `uplink_adapter` arm, see next row) | none required for a stub build |
| net/mod.rs:20-25 `uplink_adapter` | same windows/linux split | neither compiled | same as above | none required |
| net/mod.rs:26-28 `uplink_helper` | `cfg(windows)` only | not compiled | fine — only referenced inside `#[cfg(windows)]` at net/mod.rs:78-81, with an explicit `#[cfg(not(windows))]` `None` fallback at net/mod.rs:82-86 | none needed, already correctly gated |
| net/mod.rs:29-31 `uplink_polkit` | `cfg(target_os="linux")` only | not compiled | fine — only referenced from `os/linux_os/uplink_adapter.rs`, itself gated `target_os="linux"` | none needed |
| net/mod.rs:56-70 `uplink_adapter()` fn body | `cfg(windows)` / `cfg(target_os="linux")` / `cfg(not(any(windows, target_os="linux")))` | **third arm matches**: `Box::new(UnsupportedAdapter("Internet-Teilen ist auf diesem Betriebssystem nicht implementiert".into()))` | compiles fine, degrades gracefully — this is the pattern the two missing-module cases above should copy | pattern to replicate for `interfaces` and `platform` |
| net/mod.rs:75-87 `run_uplink_helper_if_requested()` | `cfg(windows)` / `cfg(not(windows))` | `not(windows)` arm: returns `None` | compiles fine | none needed |

### 1.3 `native/src/agent_proto/mod.rs`

| site (file:line) | cfg / selection | Android resolution | consequence | remedy candidates |
|---|---|---|---|---|
| agent_proto/mod.rs:17-22 `local_platform` | `cfg(not(windows))`→`os/linux_os/local_platform.rs`, `cfg(windows)`→`os/windows/local_platform.rs` | `os/linux_os/local_platform.rs` selected | compiles fine *if* `libc::SYS_renameat2`/`libc::RENAME_NOREPLACE` are defined for `target_os="android"` in the pinned `libc = "0.2"` — see §2.3 (unverified, flagged) | verify against the exact `libc` version in `Cargo.lock`; if absent, gate `rename_no_replace` to `target_os="linux"` and give android an existence-check-then-`std::fs::rename` fallback (accepts the documented race) |
| agent_proto/mod.rs:34-36 `sandbox` | `cfg(target_os="linux")` only | not compiled | Landlock sandboxing (`os/linux_os/sandbox.rs`) absent on android | no missing-symbol risk: the only consumer, the re-export at line 56, is gated identically |
| agent_proto/mod.rs:55-56 `pub use sandbox::restrict_filesystem` | `cfg(target_os="linux")` | not exported | any *caller* outside this read surface that invokes `agent_proto::restrict_filesystem()` unconditionally would fail to compile on android | **unresolved / out of scope**: the call site is outside `native/src/agent_proto/**`; not found anywhere inside the assigned surface itself |

### 1.4 `native/src/quickshare/mod.rs`

No `#[cfg]` at all — `os/shared/quickshare.rs` is the only implementation, unconditionally selected for every OS. Compiles fine on android (see §3).

### 1.5 `native/src/agent/mod.rs`

No `#[cfg]` at all. `agent/core/**` is fully platform-neutral (no `std::os::*`, no `libc`, no `Command::new` in any non-test file). Compiles fine on android.

### 1.6 In-body `#[cfg(...)]` (not module selection) inside otherwise-shared files

| site (file:line) | cfg | Android resolution | consequence |
|---|---|---|---|
| share/core/exec_platform.rs:184,189,194,199 | `cfg(all(debug_assertions, target_os="linux"))` | excluded | fine — these are the linux self-test shell command strings; android has no windows counterpart either, so `run_platform_self_test()` (share/mod.rs:409-412, `cfg(debug_assertions)`) would fail to build in debug on android regardless, because it unconditionally calls `provider_status()`/`ContainedExec::prepare` (see §1.1 row 2) |
| share/core/fs.rs:428/430 | `cfg(windows)` / `cfg(not(windows))`, inside a `#[test]` | test-only, `not(windows)` arm (`std::os::unix::fs::symlink_dir`) used | fine |
| net/core/net.rs:350 | `cfg(not(windows))`, inside `#[cfg(test)]` | test-only | fine |
| agent_proto/os/shared/transfer.rs:468; put_tree.rs:419,440 | `cfg(unix)`, inside `#[test]` | test-only, compiled | fine, no android-specific issue |
| agent/core/tests.rs:407 | `cfg(all(target_os="linux", target_arch="x86_64"))` | excluded, test-only | fine |

---

## 2. Linux-only / glibc-only APIs reachable inside `cfg(unix)` / `cfg(not(windows))` / unconditional code

### 2.1 `native/src/share/os/linux_os/identity_lock.rs` (selected for android via `cfg(not(windows))`, §1.1)

Uses `libc::geteuid`, `libc::flock(LOCK_EX)`, `libc::openat` with
`O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC|O_CREAT|O_EXCL|O_NONBLOCK` (identity_lock.rs:45,51-56,63,106,117,130),
plus `std::os::unix::fs::{DirBuilderExt,MetadataExt,OpenOptionsExt,PermissionsExt}`. All of these
are ordinary POSIX/bionic-supported calls (`geteuid`, `flock`, `openat` with those flags are present
in bionic since old API levels) — **compiles and runs fine** on android's `libc` crate target. No
`/proc`, `/sys`, systemd, or D-Bus. The one open question is not the API itself but where
`app_data_dir` (identity_store.rs:303, out-of-scope `crate::support_dirs::app_data_dir()`) resolves
to on android — a `$HOME`-style desktop path would not be writable from the app sandbox.

### 2.2 `native/src/share/os/linux_os/system.rs` (selected for android, §1.1)

Two lines: `pub(crate) use super::shared_system::lan_ips;` and `ensure_firewall_rule()` returning a
canned "not required" string. Trivial, portable, compiles fine.

### 2.3 `native/src/agent_proto/os/linux_os/local_platform.rs` (selected for android via `cfg(not(windows))`, §1.3)

`rename_no_replace()` (local_platform.rs:40-65) calls `libc::syscall(libc::SYS_renameat2, libc::AT_FDCWD, ..., libc::RENAME_NOREPLACE)` directly (bypassing any bionic wrapper, per the file's own comment that "`libc` does not expose the renameat2 wrapper on musl targets"). The rest of the file (`file_identity` via `MetadataExt::dev()/ino()`, `secure_staging_directory`/`secure_staging_file` via `PermissionsExt::from_mode`, `replace_file_atomic` via `std::fs::rename`) is plain POSIX, compiles fine.
`SYS_renameat2`/`RENAME_NOREPLACE`: the Linux kernel syscall itself is present in Android kernels (kernel ≥3.15 for years); whether the pinned `libc = "0.2"` crate exposes the `SYS_renameat2`/`RENAME_NOREPLACE` *constants* specifically for `target_os="android"` (as opposed to only `target_os="linux"`) is **not verifiable by source inspection alone** (would need the resolved `libc` crate version/docs, i.e. a remote build). Flagged, not asserted either way.

### 2.4 `native/src/share/os/linux_os/exec.rs`, `exec_systemd.rs`, `exec_self_test.rs`, `exec_supervisor.rs` (gated `target_os="linux"` only, §1.1 — **not compiled** on android)

Listed for completeness since these are the Linux-only APIs the task explicitly asks about; they never reach an android build because the whole `platform_exec` module for Linux requires `target_os="linux"`:
- systemd Manager D-Bus over `zbus` (`exec.rs:18-19`, `exec_systemd.rs`), transient cgroup-backed units, `/sys/fs/cgroup/cgroup.controllers` (exec_systemd.rs:186), `/sys/fs/cgroup` (exec_systemd.rs:235), `/proc/{pid}/exe` (exec_systemd.rs:41), session-bus fallback `unix:path=/run/user/{uid}/bus` (exec_systemd.rs:89).
- `libc::fork`, `libc::waitpid`, `libc::setsid`, `libc::signal(SIGTERM/SIGHUP, SIG_IGN)`, `libc::pause`, `libc::kill` (exec_self_test.rs:47,161,172-199).
- `SO_PEERCRED` credential lookup on a `UnixStream` (`exec.rs:314-351`), `getpwuid_r` (exec.rs:392-396).
- `/run/user/{uid}` / `/tmp/smart-explorer-runtime-{uid}` socket directory (exec.rs:352-356).
- `Command::new(program)` re-exec of the process itself as `--share-exec-supervisor` (exec_supervisor.rs:99-115).

None of this is reachable on android — but see §1.1 row 2: the portable wrapper around it (`exec_platform.rs`) does not compile on android at all today, independent of whether this Linux-only body would have been portable.

### 2.5 `native/src/agent_proto/os/linux_os/sandbox.rs` (gated `target_os="linux"` only, §1.3 — **not compiled** on android)

Raw `libc::syscall(SYS_landlock_create_ruleset)`, `SYS_openat2`, `SYS_landlock_add_rule`,
`SYS_landlock_restrict_self` (sandbox.rs:67-142), plus `libc::prctl(PR_SET_NO_NEW_PRIVS,...)`
(sandbox.rs:138). Landlock is a Linux-kernel-only LSM; even where present in an Android kernel it is
not a bionic/AOSP-exposed sandboxing primitive the same way, and this file is already excluded from
android by its `target_os="linux"` gate, so it has no build-time effect.

### 2.6 `native/src/net/os/linux_os/interfaces.rs` (gated `target_os="linux"` only, §1.2 — **not compiled** on android)

`/sys/class/net/{name}/operstate`, `/sys/class/net/{name}/carrier` (interfaces.rs:39,45),
`/proc/net/route`, `/proc/net/ipv6_route` (interfaces.rs:57,60), DHCP-lease heuristics reading
`/run/systemd/netif/leases/{index}`, `/var/lib/dhcpcd/*`, `/var/lib/dhcp/*`, `/var/lib/dhclient/*`
(interfaces.rs:110-114). None of this is android-appropriate even if it were compiled (SELinux blocks
most of `/proc/net/*` for unprivileged apps on modern Android, and none of the listed lease-file
paths exist there) — but the file is already excluded by the `target_os="linux"` gate. The
consequence is that android instead hits the **missing-module** problem in §1.2 row 1, not a wrong
answer from this file.

### 2.7 `native/src/net/os/linux_os/nm_shared.rs`, `uplink_adapter.rs`, `uplink_polkit.rs` (gated `target_os="linux"` only — **not compiled** on android)

NetworkManager over system D-Bus (`zbus::blocking::{Connection,Proxy}`, nm_shared.rs:7-8,25-34),
`dnsmasq` PATH probing (uplink_adapter.rs:45-52), polkit rule file
`/etc/polkit-1/rules.d/49-smart-explorer-lan-uplink.rules` (uplink_polkit.rs:7) and
`Command::new("pkexec")` (uplink_polkit.rs:59). Correctly excluded from android by the
`target_os="linux"` gate; the crate-level `uplink_adapter()` factory (net/mod.rs:56-70) already
degrades to `UnsupportedAdapter` for android, so this is a clean "feature absent" rather than a
build or missing-symbol problem.

### 2.8 Other categories asked about, not found in this read surface

`std::os::linux::*`, `memfd_create`, `statx`, `copy_file_range`, `pidfd_*`, `O_TMPFILE`,
`fallocate64`, `landlock` outside §2.5, `prctl` outside §2.5, `xdg` paths, D-Bus/zbus outside
§2.4/§2.7, polkit outside §2.7 — none found anywhere else in `share/**`, `net/**`, `agent/**`,
`agent_proto/**`, `quickshare/**`. `getrandom` **is** used (crypto.rs:37, discovery_pake.rs:114,
nm_shared.rs:122, deploy.rs:78) but via the `getrandom` crate (not a raw libc call), which lists
android as a first-class supported target — flagged only for completeness, not a concern.

---

## 3. Dependencies (`native/Cargo.toml`) and their per-target gating

| dependency | Cargo.toml line(s) | gate | android pulls it in? | used in assigned surface? | note |
|---|---|---|---|---|---|
| `iroh` | 87 | none (`[dependencies]`) | yes | share/core/node.rs, crypto.rs, direct_protocol.rs, fs_copy.rs, framing.rs, keepalive.rs, identity.rs, endpoint_routes.rs | pure-Rust QUIC/relay stack, no OS-conditional code observed at any call site in this surface (§4.1) |
| `tungstenite` | 95 | none | yes | share/core/signal_connection.rs | rustls/webpki TLS feature set only, no native TLS |
| `mdns-sd` | 99 | none | yes | share/os/shared/lan_presence.rs, quickshare/os/shared/quickshare.rs | needs a runtime multicast/UDP capability; on android this is an OS **permission** concern (`CHANGE_WIFI_MULTICAST_STATE` / `WifiManager.MulticastLock`), not a compile concern — outside this crate |
| `if-addrs` | 100 | none | yes | share/os/shared/system.rs, net/os/linux_os/interfaces.rs (excluded on android, §2.6) | already the portable choice used elsewhere in this surface; candidate replacement for the missing android `interfaces` module (§1.2) |
| `opaque-ke`, `argon2`, `chacha20`, `chacha20poly1305`, `hkdf`, `zeroize` | 67-72 | none | yes | share/core/discovery_pake.rs and related discovery_signal_* wire/crypto files | pure Rust, no OS dependency observed |
| `hickory-resolver` | 92 | none | yes | share/core/signal_connection.rs | pure-Rust async DNS, used with the `tokio` runtime already a dependency |
| `libc` | 165 | `[target.'cfg(not(windows))'.dependencies]` | **yes** (android matches `not(windows)`) | share/os/linux_os/identity_lock.rs (§2.1), agent_proto/os/linux_os/local_platform.rs (§2.3), and the linux-only files in §2.4/§2.5 (those specific files are additionally gated `target_os="linux"` at the `mod` level, so the crate dependency is present but those particular call sites are not compiled) | see §2.3 for the one unverified `SYS_renameat2`/`RENAME_NOREPLACE` question |
| `rfd` (`xdg-portal`,`tokio` features) | 166 | `[target.'cfg(not(windows))'.dependencies]` | **yes** (android matches `not(windows)`) | **no call site found anywhere in `share/**`, `net/**`, `agent/**`, `agent_proto/**`, `quickshare/**`** | flagged because Cargo.toml was in the read surface: this pulls rfd's `xdg-portal` backend (→ `ashpd` → D-Bus to a desktop portal) into every android build target regardless of whether this assigned surface uses it; the desktop-portal protocol has no android counterpart. Whether this breaks the android build depends entirely on code outside this read surface (the GUI file-picker call site) — **unresolved / out of scope** for this audit |
| `zbus` | 171 | `[target.'cfg(target_os = "linux")'.dependencies]` | **no** (android is `target_os="android"`, not `"linux"`) | net/os/linux_os/nm_shared.rs, share/os/linux_os/exec.rs, exec_systemd.rs, exec_self_test.rs — all already gated `target_os="linux"` at the module level (§2.4, §2.7), so the dependency drop is consistent with the source gating; no orphaned unconditional use of `zbus` was found | correctly scoped already |
| `keyring` (`windows-native`) | 111 | `[target.'cfg(windows)'.dependencies]` | no | not referenced in this surface (only `crate::creds`, out of scope, is) | consistent with the Cargo.toml comment (line 109-110) that non-Windows uses a file store instead |
| `windows-sys`, `windows-core`, `winreg`, `windows` | 108-161 | `[target.'cfg(windows)'.dependencies]` | no | net/os/windows/*, share/os/windows/* only | correctly excluded |

---

## 4. Share/ runtime pieces: entry points and OS-adapter dependence

| piece | entry point (file:line, signature) | OS-adapter dependent? | notes |
|---|---|---|---|
| Iroh endpoint | `ShareIrohNode::start_with_repair_store(server:&str, identity:&ShareIdentity, auth:Arc<Mutex<ShareAuthState>>, ev:crossbeam_channel::Sender<ShareEvent>, direct_repair_store:SharedDirectRepairStore) -> io::Result<Arc<Self>>` — share/core/node.rs:71; `Endpoint::builder(presets::Minimal)...bind()` at node.rs:92-100 | no | no `#[cfg]` in node.rs; relies only on `iroh`/`tokio` |
| Signaling over tungstenite WSS | `SignalConnection::connect(config:&str) -> io::Result<Self>` — share/core/signal_connection.rs:30 (dispatches to `connect_tcp`/`connect_ws` at lines 62,80) | no | plain `std::net::TcpStream` + `tungstenite`/`hickory-resolver`, no `#[cfg]` in the file |
| Background worker (signal + direct + exec-grant loop) | `pub(super) fn worker(server:String, identity:ShareIdentity, iroh:Arc<ShareIrohNode>, auth:Arc<Mutex<ShareAuthState>>, commands:Receiver<PendingShareCmd>, events:Sender<ShareEvent>, stopped_flag:Arc<AtomicBool>, discovery_port:Box<dyn DiscoveryExchangePort>, reciprocal:Arc<DirectReciprocalCoordinator>, repair_completions:DirectRepairCompletionReceiver)` — share/core/signal_worker.rs:29; spawned via `std::thread::Builder::new().name("share-signal").spawn(...)` at share/core/service.rs:372-386, inside `ShareService::start`/`start_with_profile_home` (service.rs:291,299) | no (thread itself), yes at the host-process level | this OS thread is the natural mapping target for an android foreground service / `WorkManager` job — nothing in the thread body is android-incompatible by itself, but its process lifetime today is "as long as the desktop app process runs", which does not exist as a concept on android without an explicit foreground service wrapping it from the JNI/Kotlin side |
| LAN presence (mDNS) | `LanPresence::start() -> Result<Self, String>` — share/os/shared/lan_presence.rs:38; browser thread `std::thread::Builder::new().name("se-lan-presence").spawn(...)` at lines 48-58 | no compile-time OS-adapter, yes at the runtime-permission level | `mdns_sd::ServiceDaemon` is pure Rust; android needs a multicast lock acquired at the app layer for this to actually receive mDNS packets — not present anywhere in this crate |
| Pairing / OPAQUE | `DiscoveryPakeRegistration::register(...)` — share/core/discovery_pake.rs:161; `start_exchange` (207); `DiscoveryPakeExchange::start` (270); `finish` (290, and a second `finish` at 364) | no | pure `opaque-ke`/`argon2`/Sha512, no `#[cfg]` in the file |
| Exec server (remote command execution, server side) | `pub(super) async fn handle_connection(...)` — share/core/exec_server.rs:26 | **yes, indirectly**: the server protocol itself is portable async code, but every actual process is started through `super::exec_platform::ContainedExec` (exec_platform.rs:26-79), which requires the missing `platform_exec` module on android (§1.1 row 2) | this is the one piece in this list that is a genuine build blocker today, not just a runtime-permission gap |
| Exec client (issuing a remote command) | `pub(crate) fn spawn_connected(...)` — share/core/exec_client.rs:81 | no | client side only sends/receives wire frames over the Iroh stream; no local process is started here |
| LAN uplink / ICS / NetworkManager | `pub fn uplink_adapter() -> Box<dyn UplinkAdapter>` — net/mod.rs:56 | yes, and already correctly degraded: `#[cfg(not(any(windows, target_os="linux")))]` arm at net/mod.rs:65-69 returns `UnsupportedAdapter` | no missing-symbol risk; the feature is simply unavailable on android, consistent with "drive mounting / uplink sharing not required" |
| Storage snapshot (recursive tree walk over a share) | `pub(super) async fn serve_snapshot(mut send:SendStream, root:String, access:FsAccess) -> io::Result<()>` — share/core/storage_snapshot.rs:20 | no | walks via `super::walk`/`fs_access` (std::fs-based), no OS-conditional code in the file itself |

### 4.2 Does share code spawn processes or use local IPC that assumes a desktop?

- Yes, but only inside code already excluded from android by `target_os="linux"` gating (§2.4,
  §2.7): `Command::new(program)`/`Command::new(shell)` self-re-exec as the exec supervisor
  (share/os/linux_os/exec_supervisor.rs:99,110), systemd Manager D-Bus IPC
  (share/os/linux_os/exec_systemd.rs, exec.rs:18-19), `Command::new("pkexec")`
  (net/os/linux_os/uplink_polkit.rs:59).
- The one android-reachable process-adjacent code (identity_lock.rs, local_platform.rs) does not
  spawn processes; it only opens/locks files and does `flock`/`openat`/`rename` — no `Command::new`,
  no IPC socket beyond the `UnixStream` used for exec peer-credential checks, which is itself inside
  the excluded `exec.rs`.
- Net's windows-only `Command::new("powershell"/"netsh")` sites (net/os/windows/uplink_helper.rs:60,112;
  net/os/windows/ics.rs:14; share/os/windows/system.rs:19,31,78) are irrelevant to android (excluded
  by `cfg(windows)`).

---

## Summary of the two build-breaking findings

1. **share/mod.rs:230-235 + share/core/exec_platform.rs:27,34,82,86,143** — `platform_exec` module
   is only defined for `windows`/`target_os="linux"`; `exec_platform.rs` references it
   unconditionally. Android build fails with an unresolved-module error unless a third arm is added
   (or the whole exec feature is compiled out with its own android-aware guard inside
   `exec_platform.rs`).
2. **net/mod.rs:1-6 (`interfaces`) and net/mod.rs:7-12 (`platform`) + net/mod.rs:92, net/core/net.rs:19**
   — both modules are only defined for `windows`/`target_os="linux"`; both are referenced
   unconditionally by `gather_interface_facts()` and by the `use super::platform::{...}` import.
   Android build fails the same way, twice, unless android arms are added (an `if-addrs`-based
   `interfaces` arm, and a stub `platform` arm reusing the existing Linux no-op body).

Everything else discovered either already degrades cleanly on android (`uplink_adapter()`,
`run_uplink_helper_if_requested()`, `identity_lock`, `system`) or is correctly excluded from the
android build by existing `target_os="linux"` gates (systemd/D-Bus exec, NetworkManager/polkit
uplink, Landlock sandbox, `/proc`+`/sys` interface facts).
