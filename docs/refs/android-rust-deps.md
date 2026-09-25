# Android Cross-Compile Viability of native/ Rust Dependencies

**Purpose.** For a planned Android (APK) build of Smart Explorer — Kotlin + Jetpack Compose UI,
existing `native/` Rust crate cross-compiled for `aarch64-linux-android` /
`x86_64-linux-android` via `cargo-ndk` + NDK clang, called through JNI, no drive mounting —
this file records, per locked dependency, whether it compiles and works on Android, what
breaks at runtime even if it compiles, and the remedy. All web checks dated **2026-09-25**.
`target_os = "android"` has `target_family = "unix"` but is **not** `"linux"`, so any
`#[cfg(target_os = "linux")]` code in this codebase or its dependencies is skipped for Android
builds.

**Files read.**
- `native/Cargo.toml` (full file)
- `native/Cargo.lock` (`rg` queries for exact locked versions and reverse-dependency edges of
  ~90 package names, including duplicate-version disambiguation)
- Web: docs.rs, crates.io (incl. the `crates.io` JSON API), GitHub source/READMEs/issues for the
  crates below (URLs inline per row/section)

---

## 1. Networking / P2P stack (iroh and its tree) — the highest-risk area

| crate | locked | Android compiles? | runtime caveat | remedy / config | source |
|---|---|---|---|---|---|
| `iroh` | 1.0.1 | Yes (pure Rust + `noq`/`noq-udp` QUIC stack, no `quinn`/mio in this version) | Pulls `reqwest` → optionally `rustls-platform-verifier` (see below); pulls `netwatch` (netlink, see §1a) | Build with `cargo-ndk`; verify TLS provider stays `ring` (already pinned project-wide) | [iroh-ffi Kotlin README](https://github.com/n0-computer/iroh-ffi/blob/main/README.kotlin.md) shows a working `aarch64-linux-android`/`x86_64-linux-android` JNI build, min API 29 in its example `.cargo/config.toml` |
| `netwatch` (§1a) | 0.19.1 | Compiles (pure Rust wire code + libc sockets) | **Runtime-restricted**: pulls `netlink-packet-core/-route`, `netlink-proto`, `netlink-sys`, and `netdev` for interface/route-change monitoring. On Android **API ≥ 30** (targetSdk 30+), apps are denied `bind()`/most `RTM_GETLINK` netlink-route traffic by SELinux; only `RTM_GETADDR`-style queries and `SIOCGIF*` ioctls stay permitted | No code fix available from this crate alone; expect iroh's network-change detection to degrade to "no live updates" or errors on modern Android. Must be verified on-device once cross-compiled | [Android netlink `bind()` denial, b/155595000 discussion](https://groups.google.com/g/android-ndk/c/3JIvD0PFaU4); [getifs crate notes on RTM_GETLINK being denied for targetSdk≥30](https://docs.rs/getifs) |
| `netdev` | 0.45.0 (via netwatch) | Compiles; basic interface enumeration works standalone | Deeper metadata (DNS servers, DHCP hints, Wi-Fi link speed) needs Android **JNI context** (`ndk-context`) + app declaring `ACCESS_NETWORK_STATE`/`ACCESS_WIFI_STATE` permissions, or it silently omits that data | If deeper metadata is wanted, call `ndk_context::initialize_android_context(...)` from the JNI `onLoad`/init path (same pattern as netwatch's other Android glue) | [shellrow/netdev](https://github.com/shellrow/netdev) |
| `portmapper` | 0.19.1 | Yes (pure Rust UDP + `igd-next`) | UPnP/IGD port mapping is meaningless on a phone behind carrier/Wi‑Fi NAT with no router UI access; harmless no-op, not a blocker | None needed; iroh already treats it as best-effort | Cargo.lock dependency graph (this session) |
| `iroh-relay` | 1.0.1 | Yes (HTTP(S)/WSS relay client, rustls-backed) | None beyond generic TLS notes below | — | Cargo.lock |
| `iroh-dns` (n0 DNS) | 1.0.1 | Yes | Depends on `hickory-resolver` for resolution (see §2) | — | Cargo.lock (`iroh` → `iroh-dns` edge) |
| `hickory-resolver` | 0.26.1 | Yes, **but see §2 — real crash risk** | — | — | see §2 |
| `rustls-platform-verifier` | 0.7.0 (pulled transitively via `iroh`→`reqwest`, confirmed present and thus feature-enabled in the lock) | Yes; ships a dedicated `rustls-platform-verifier-android` (0.1.1) support crate | **Needs a small Kotlin/Java helper class bundled in the app** to call into Android's own certificate verifier via JNI (`jni` 0.22.4 dep) | Add the documented Android companion module to the Kotlin app project; without it, platform verification on Android will fail/panic at first use | [rustls/rustls-platform-verifier](https://github.com/rustls/rustls-platform-verifier), [crates.io rustls-platform-verifier-android](https://crates.io/crates/rustls-platform-verifier-android) |

### 1a. Known-problematic transitive crates found in `Cargo.lock`

| crate(s) | locked version(s) | reached through | Android relevance |
|---|---|---|---|
| `netlink-packet-route`, `netlink-packet-core`, `netlink-proto`, `netlink-sys` | 0.31.0 / 0.8.1 / 0.12.0 / 0.8.8 | **`iroh` → `netwatch`** (not eframe/rfd/zbus — this is a core, always-needed edge for the planned Android build) | Compiles; runtime netlink-route access is SELinux-restricted on API≥30 (see table above) |
| `nix` 0.31.3 | via **`ctrlc`** (direct dependency, ungated in `native/Cargo.toml` line 13) | `nix` itself supports `target_os = "android"` generally, so this compiles | See §5 — `ctrlc`/SIGINT is not a meaningful concept in the normal Android app lifecycle |
| `nix` 0.29.0 | via `zbus` (both the direct Linux-gated dep and the `rfd`→`ashpd` edge) | Only reached if `zbus` is compiled for Android — see §4, must be avoided | — |
| `x11rb` 0.13.2, `wayland-client` 0.31.14, `wayland-sys` 0.31.11, `wayland-protocols` 0.32.12, `smithay-client-toolkit` 0.19.2 | via **`winit`** (pulled only by `eframe`'s desktop windowing) | Confirmed via reverse-dep grep: `winit` is the sole requirer of all four | **Only reached through eframe** — irrelevant if the Android build excludes eframe/egui (see §6) |
| `smithay-client-toolkit` 0.20.0 | via `smithay-clipboard` | Same eframe/egui desktop-clipboard chain | Irrelevant if eframe excluded |
| `rtnetlink`, `libudev`/`libudev-sys`, `alsa`/`alsa-sys`, plain `x11` | — | **absent from `Cargo.lock` entirely** (checked by exact-name `rg`) | Not a concern at all |

---

## 2. DNS resolution — `hickory-resolver` 0.26.1 (real Android crash risk)

- Default features are `["system-config", "tokio"]` (confirmed in the crate's `Cargo.toml`), and
  `system-config` pulls, per target: `ipconfig`+`resolv-conf` (Windows/Unix), and for
  **`cfg(target_os = "android")` specifically: `jni` 0.22.1 (optional) + `ndk-context` 0.1.1
  (optional)** — i.e. hickory-resolver ships a *real* Android backend
  (`hickory_resolver::system_conf::android::read_system_conf`) that calls into the JVM via
  `ndk-context` to read the system's actual DNS servers, rather than reading
  `/etc/resolv.conf` (which is not a reliable source on Android).
- **Confirmed crash mode**: [hickory-dns/hickory-dns#3625](https://github.com/hickory-dns/hickory-dns/issues/3625)
  — the resolver **panics** inside `ndk_context::android_context()` when the Android context was
  never initialized (reproduced there under Termux, which Rust also reports as
  `target_os = "android"`, but the same panic path applies to *any* process where
  `ndk-context::initialize_android_context()` was not called before the resolver's system-config
  path runs).
- **Remedy (mandatory, not optional)**: the Kotlin/JNI init path must call
  `ndk_context::initialize_android_context(vm, context)` (typically from `JNI_OnLoad` or an
  explicit `Context.init()` call from Kotlin) **before** any code reaches
  `TokioResolver::builder_tokio()`/`from_system_conf()` — used both by `iroh-dns` internally and
  directly by this project's own signaling/FTP host lookup (per the comment at
  `native/Cargo.toml:89-91`). If that init is skipped or races the first resolve, the process
  crashes rather than falling back.
- Secondary fallback, if system-config is deliberately disabled instead: construct the resolver
  explicitly with `ResolverConfig::cloudflare()` or `ResolverConfig::google()`
  ([`ResolverConfig` docs](https://docs.rs/hickory-resolver/latest/hickory_resolver/config/struct.ResolverConfig.html)),
  both plain UDP/TCP on port 53, no extra feature gate required.

Sources: [hickory-resolver docs.rs](https://docs.rs/hickory-resolver/latest/hickory_resolver/),
[hickory-resolver 0.26.1 Cargo.toml source](https://docs.rs/crate/hickory-resolver/0.26.1/source/Cargo.toml),
[issue #3625](https://github.com/hickory-dns/hickory-dns/issues/3625).

---

## 3. TLS / crypto stack

| crate | locked | Android compiles? | runtime caveat | remedy / config | source |
|---|---|---|---|---|---|
| `ring` | 0.17.14 | Yes — builds via the `cc` crate against NDK clang, well-trodden path | Needs versioned NDK clang driver names (e.g. `aarch64-linux-android24-clang`) on recent NDKs; `cargo-ndk` sets `CC_*`/`AR_*` env vars for this automatically | Use `cargo-ndk`; do not hand-roll `CC_aarch64_linux_android` | [briansmith/ring cross-compile issues](https://github.com/briansmith/ring/issues/1050), [cargo-ndk crate](https://crates.io/crates/cargo-ndk/3.5.5) |
| `rustls` | 0.23.40 | Yes (pure Rust, already pinned to the `ring` provider project-wide per `native/Cargo.toml:50`) | None beyond `ring`'s build requirement above | — | crates.io |
| `webpki-roots` | 0.26.11 | Yes (static data, pure Rust) | None | — | crates.io |
| `rustls-platform-verifier` | 0.7.0 | Yes, has Android backend | See §1 table row above | — | see §1 |

No `aws-lc-sys` in the tree (verified: `russh` uses the `ring` feature per `native/Cargo.toml:34`,
matching the project's existing "no aws-lc-rs" policy in `docs/GOTCHAS.md`), so no NASM/CMake
requirement carries over to the NDK build.

---

## 4. Filesystem / desktop-integration crates — need Cargo.toml gating changes for Android

| crate | locked | Android compiles? | runtime caveat | remedy / config | source |
|---|---|---|---|---|---|
| `trash` | 5.2.5 | **No — fails to compile.** Confirmed by reading `src/lib.rs`: the Unix branch is gated `#[cfg(all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android")))]` (freedesktop trash), and there is no Android module at all, so the crate has no `platform` implementation to select for `target_os = "android"` | — | Do not compile `trash` into the Android `cdylib`; implement delete-to-trash via Android's `MediaStore`/`ContentResolver` trash API from Kotlin instead (or the JNI layer), and route the existing `trash::*` call sites behind a feature/OS split | [ArturKovacs/trash `src/lib.rs`](https://github.com/ArturKovacs/trash) |
| `rfd` (non-Windows block, `native/Cargo.toml:164-166`) | 0.15.4 (+ `ashpd` 0.11.1 + `zbus` 5.16.0 + `wayland-client`/`wayland-protocols`) | **No.** Confirmed by reading `src/backend.rs`: the module-selection `cfg`s only enumerate `linux`/`freebsd`/`dragonfly`/`netbsd`/`openbsd` (→ `gtk3` or `xdg_desktop_portal`), `macos`, `windows`, and `wasm32`; `target_os = "android"` matches none of them, so no backend module is compiled in | Its D-Bus/Wayland dependency chain (`ashpd`→`zbus`, `wayland-client`) has no Android equivalent anyway (no session D-Bus, no Wayland compositor) | The current `[target.'cfg(not(windows))'.dependencies]` block in `native/Cargo.toml` includes Android under `not(windows)`. **Must** be narrowed to e.g. `cfg(all(not(windows), not(target_os = "android")))` for `rfd`, and file picking done via Android's Storage Access Framework (`Intent.ACTION_OPEN_DOCUMENT`/SAF) in Kotlin, with the resulting `content://` URI or a `ParcelFileDescriptor` handed to the Rust core | [PolyMeilex/rfd `src/backend.rs`](https://github.com/PolyMeilex/rfd) |
| `zbus` (direct, `native/Cargo.toml:171`) | 5.16.0 | N/A on Android | Already correctly gated `[target.'cfg(target_os = "linux")'.dependencies]` — **not** pulled for `target_os = "android"` at all | No change needed | `native/Cargo.toml:168-171` (read directly) |
| `keyring` (Windows block, `native/Cargo.toml:111`) | 3.6.3 | N/A on Android | Already gated `[target.'cfg(windows)'.dependencies]` — not pulled for Android | Project already plans a headless owner-protected file store for Linux per the existing comment at `native/Cargo.toml:109-110`; reuse/extend that store for Android rather than adding a keyring backend | `native/Cargo.toml:108-111` (read directly) |
| `libc` (non-Windows block, `native/Cargo.toml:165`) | 0.2.186 | Yes, compiles for Android too (stays under the same `not(windows)` block as `rfd`, but this one is fine to keep) | `renameat2`'s `flags` parameter is typed `c_int` on `target_os="linux"` but **`u32` on `target_os="android"`** in the `libc` crate — a bare `libc::RENAME_NOREPLACE` call that compiles on Linux can fail to compile on Android with an `E0308` type mismatch | Cast the flags argument (`RENAME_NOREPLACE as _`) so the call is portable across both signatures; `statx` is also present for Android in recent `libc` (0.2.x) | [libc `renameat2`/Android signature diff](https://docs.rs/libc/latest/aarch64-linux-android/libc/constant.SYS_renameat2.html) |

**`renameat2` on Android (checked 2026-09-25, primary sources):** bionic lists
`renameat2(int, const char*, int, const char*, unsigned) all` in `libc/SYSCALLS.TXT`
([source](https://android.googlesource.com/platform/bionic/+/refs/heads/main/libc/SYSCALLS.TXT)),
added as a new libc function in R / API level 30
([docs/status.md](https://android.googlesource.com/platform/bionic/+/refs/heads/main/docs/status.md)).
`libc/tools/genseccomp.py` builds the app seccomp allowlist as
`(SYSCALLS.TXT names − blocklists) | allowlists`
([source](https://android.googlesource.com/platform/bionic/+/refs/heads/main/libc/tools/genseccomp.py)),
so with minSdk 30 the raw `SYS_renameat2` syscall is permitted (no `SIGSYS`). The locked
`libc` 0.2.186 exports `SYS_renameat2` (aarch64 276, x86_64 316), `RENAME_NOREPLACE: c_int = 1`
and `renameat2(.., flags: c_uint)` for `target_os = "android"`
(`src/unix/linux_like/android/{mod.rs,b64/*/mod.rs}`); the raw `libc::syscall` form used in
`native/src/android_fs/os/rename.rs` avoids the `c_int`/`c_uint` flag mismatch.

---

## 5. Everything else requested

| crate | locked | Android compiles? | runtime caveat | remedy / config | source |
|---|---|---|---|---|---|
| `tokio` | 1.52.3 | Yes — `mio` (tokio's I/O backend) explicitly lists Android as supported, with `cfg(target_os = "android")` paths throughout | — | — | [mio Android support](https://docs.rs/mio) |
| `russh` | 0.61.2 | Yes — built with `ring` (project already sets `default-features=false, features=["ring","flate2"]`, `native/Cargo.toml:34`), avoiding `aws-lc-sys`'s C/CMake/NASM requirement | — | Keep the existing `ring` feature choice; do **not** let Android accidentally pull the default `aws-lc-rs` backend | [russh crypto backend notes](https://github.com/Aloecraft-org/ego-transport/issues/5) |
| `russh-sftp` | 2.3.0 | Yes — pure-Rust SFTP wire implementation, narrow `tokio` feature set (`io-util, rt, sync, time` per its own docs), no OS-specific code | — | — | [russh-sftp on lib.rs](https://lib.rs/crates/russh-sftp) |
| `suppaftp` | 6.3.0 | Yes with the project's existing `rustls` feature (`native/Cargo.toml:42`) — pure Rust, no native TLS | — | — | [suppaftp GitHub](https://github.com/veeso/suppaftp) |
| `ureq` | 2.12.1 | Yes — blocking I/O, no `mio` dependency at all; TLS via `rustls`+`ring` (project's existing `tls` feature, `native/Cargo.toml:31`) | — | — | [ureq docs.rs](https://docs.rs/ureq) |
| `tungstenite` | 0.24.0 | Yes with the project's existing `rustls-tls-webpki-roots` feature (`native/Cargo.toml:95`) — pure Rust WSS, no native TLS/OS cert store dependency | — | — | crates.io feature docs |
| `rusqlite` (`bundled`) / `libsqlite3-sys` | 0.40.1 / 0.38.1 | Yes — `bundled` compiles SQLite from C source via the `cc` crate against NDK clang (same toolchain path as `ring`); this is the standard way SQLite is embedded in Android apps | Static linking recommended (dynamic `-lsqlite3` linking against Android's system SQLite is fragile/version-dependent) | Keep `features = ["bundled"]`, already the case (`native/Cargo.toml:83`) | [rusqlite/libsqlite3-sys](https://github.com/rusqlite/rusqlite/tree/master/libsqlite3-sys) |
| `notify` (`inotify` backend) | 6.1.1 (`inotify` 0.9.6) | Yes — inotify is a Linux kernel facility present in Android's Bionic/kernel; notify's backend selection explicitly covers Linux **and** Android | — | — | crate docs (backend selection) |
| `ctrlc` | 3.5.2 | Yes, compiles (pulls `nix` 0.31.3, which supports Android) | **Functionally close to meaningless on Android**: there is no controlling terminal and the normal app-kill path is Activity/Service lifecycle (Binder) or `SIGKILL`, not `SIGINT`; a registered handler will typically never fire in normal use | Do not rely on `ctrlc` for the Android background worker's shutdown path; drive cancellation from the Kotlin `Service`/`WorkManager` lifecycle into the Rust core via an explicit JNI "stop" call instead | Cargo.lock reverse-dep (`ctrlc` → `nix` 0.31.3, this session); [ctrlc docs](https://docs.rs/ctrlc) |
| `getrandom` (0.2.x) | 0.2.17 | Yes — uses the `getrandom(2)` syscall when available, else falls back through `/dev/urandom`; Android is explicitly listed among the fallback-supported `target_arch`es (aarch64, arm, x86, x86_64 — i.e. every ABI this project would ship) | Needs sufficiently new API level for the syscall path on non-covered archs (not applicable here, since only the 4 listed archs are used) | — | [getrandom docs.rs](https://docs.rs/getrandom/0.2.17/getrandom/) |
| `chrono` (`clock` feature) | 0.4.44 (`iana-time-zone` 0.1.65) | Yes | Chrono moved Android local-time detection to Bionic's `localtime_r`/`mktime` via `libc` directly rather than parsing tzdata or depending on `iana-time-zone`/`android-tzdata` on Android — so no extra JNI plumbing is needed just for `Local::now()` | — | [chronotope/chrono PR #1148](https://github.com/chronotope/chrono/pull/1148) |
| `if-addrs` | 0.13.4 | Yes, with a caveat | Historically broken on Android ≥ 11 in older releases; upstream discussion indicates the custom code path was replaceable by the plain libc `getifaddrs()` implementation once Android's own `getifaddrs()` was fixed — **verify against the exact locked 0.13.4 source**, this project's read surface did not include its `Cargo.toml`/source | If problems appear on-device, the `getifs`/`getifaddrs` crates were mentioned upstream as Android-hardened alternatives | [if-addrs#14 "broken on android >= 11"](https://github.com/messense/if-addrs/issues/14) |
| `socket2` | 0.5.10 (direct) / 0.6.4 (via netwatch/portmapper/noq-udp, duplicate in lock) | Yes — Android is built in socket2's own CI (build-only, not fully runtime-tested upstream) | Some advanced socket options may be unverified on Android specifically | — | [socket2 README](https://github.com/rust-lang/socket2) |
| `mdns-sd` | 0.11.5 | Compiles (pure Rust, `if-addrs`+`socket2`) | Android's Wi‑Fi stack drops inbound multicast unless the app holds a `WifiManager.MulticastLock` — this is JVM-only API, `mdns-sd` itself has no Android/JNI code to acquire it | Kotlin side must `WifiManager.MulticastLock.acquire()` for the duration of discovery/advertisement (mirrors how Flutter mDNS plugins solve this) before the Rust `mdns-sd` calls will actually receive/send multicast traffic | [WifiManager.MulticastLock docs](https://developer.android.com/reference/android/net/wifi/WifiManager.MulticastLock) |
| `zip` (`deflate` feature only) / `flate2` / `miniz_oxide` | 2.4.2 / 1.1.9 / 0.8.9 | Yes — pure Rust `miniz_oxide` backend already selected (no `zlib`/`zlib-ng` C dependency in this feature set per `native/Cargo.toml:103` comment) | — | — | crates.io |
| `clap` / `clap_complete` | 4.6.1 / 4.6.7 | Yes — pure Rust CLI parsing; only matters for the `se` CLI binary, not the Android library target | If the Android build only ships the library (`cdylib`) and not the `se` CLI binary, these are irrelevant to the APK entirely | — | crates.io |
| `same-file` | 1.0.6 | Yes — pure Rust, uses `stat`/inode comparison on Unix (Android included) | — | — | crates.io |
| `tempfile` | 3.27.0 | Yes — standard Unix temp-file APIs work on Android's sandboxed filesystem, but the *location* matters: Android apps can't write to arbitrary paths, only their app-specific dirs | Point `tempfile`'s base dir (via `TMPDIR` or explicit `Builder::tempdir_in`) at the app's `context.getCacheDir()`/`getFilesDir()` path passed in from Kotlin, not a bare `/tmp` | — | general Android sandboxing knowledge |
| `argon2` / `opaque-ke` / `chacha20` (0.9.1 **and** pinned `=0.10.0`, two locked versions) / `chacha20poly1305` / `hkdf` / `zeroize` / `sha2` / `hmac` / `base64` / `roxmltree` / `md5` / `serde` / `serde_json` / `regex` / `globset` / `similar` / `crossbeam-channel` / `rayon` | various | Yes, all | Pure-Rust, no `target_os`-specific code paths in any of these (RustCrypto/serde/regex-ecosystem crates); the duplicate `chacha20` 0.9.1 vs 0.10.0 is an ordinary transitive/direct version split, not an Android-specific issue | — | crates.io (general knowledge; not individually re-verified per-crate given uniform pure-Rust status) |

---

## 6. GUI-stack crates — irrelevant to the Android build if Kotlin/Compose is the UI

| crate | locked | Note |
|---|---|---|
| `eframe` / `egui` / `egui_extras` | 0.29.1 | Only needed for the **desktop** GUI. `eframe`'s `accesskit` feature pulls `accesskit_unix`→`atspi`→`zbus` 4.4.0 (confirmed via reverse-dep grep: `atspi`/`atspi-common`/`atspi-connection`/`atspi-proxies` are the sole requirers of `zbus 4.4.0` in the lock) — a Linux AT‑SPI accessibility bus with no Android equivalent. `eframe` also pulls `winit`→`wgpu`/`glow`, which *does* have an Android backend (`android-activity`/`NativeActivity`) but that entry-point model is a full native `Activity`, incompatible with hosting inside a Compose `Activity`/Service. |
| **Recommendation** | — | Make the desktop-GUI crate group (`eframe`, `egui`, `egui_extras`, and transitively `winit`/`wgpu`/`glow`/`accesskit`) an optional Cargo feature (e.g. `desktop-gui`) so the Android `cdylib` target simply does not enable it, keeping `x11rb`/`wayland-*`/`smithay-client-toolkit`/`atspi`/`zbus 4.4.0` out of the Android dependency graph entirely (they are otherwise unreachable from Android's `target_os` cfg anyway, since nothing in that chain is Android-gated — it would only fail if actually built for the android target because `winit`'s desktop backends themselves don't support it uniformly). |

---

## Cross-cutting notes (not per-crate)

- **`native/Cargo.toml`'s `[target.'cfg(not(windows))'.dependencies]` block currently mixes an
  Android-incompatible crate (`rfd`) with an Android-compatible one (`libc`)** — this block must be
  split for an Android target (see §4).
- No occurrence of `aws-lc-sys`, `alsa`, `alsa-sys`, `libudev`, `libudev-sys`, plain `x11`, or
  `rtnetlink` anywhere in `Cargo.lock` (checked by exact-name `rg`), so none of those need
  consideration at all.
- Two `zbus` versions are locked: 5.16.0 (direct Linux-gated dependency at
  `native/Cargo.toml:171`, **and** via `rfd`→`ashpd` under the too-broad `not(windows)` gate) and
  4.4.0 (via `eframe`→`accesskit_unix`→`atspi`, i.e. only reachable through the desktop GUI
  chain). Neither is reachable on Android once §4's `rfd` gating fix and §6's `eframe`
  feature-gating are applied.
- 16 KB page size: Android 15+ (API 35) requires apps targeting API 35 to support 16 KB memory
  pages as of November 2025; this is a **linker flag** concern (`-Wl,-z,max-page-size=16384`),
  not a per-crate dependency-compatibility concern, and applies uniformly to whatever `cargo-ndk`
  produces — flagged here only so it isn't lost, not analyzed further (out of this task's scope).
