# Remote Layer — Implementation Plan

Verified, code-level plan for the unified network interface + SFTP/FTP/netdrive
backends. Researched and adversarially verified (high confidence) — but **not
yet implemented**. Build the pieces in the order below; de-risk the SFTP
dependency FIRST (see §5).

## 1. Design decision: a SYNC `Backend` trait

The whole codebase is synchronous (rayon + `std::thread` + crossbeam channels).
**Keep the trait blocking.** Remote backends own a private tokio runtime and
`block_on` internally, so `scanner.rs`, `copy.rs`, and the egui drain loop never
see async. (`suppaftp`'s API is already blocking → no runtime needed for FTP;
only the `russh` SFTP backend needs the embedded runtime.)

Tag the **location**, not every entry: add a 1-byte `Copy` `scheme: Scheme`
(`Local|Sftp|Ftp`, default `Local`) to `FileEntry` so the hot local rayon walk
never touches a vtable. The backend handle lives **once per scan** (in `Scanner`)
and **once per copy** (in `CopyOptions`), never per `FileEntry`.

Results stream over the **unchanged** crossbeam `ScanMessage` channel, so
`app.rs`'s drain loop and the entire UI are untouched.

### `vfs.rs` (new) — trait + LocalBackend + dispatch

```rust
use std::io::{Read, Write};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Scheme { #[default] Local, Sftp, Ftp }

#[derive(Clone, Debug)]
pub struct VfsMeta {
    pub name: String, pub is_dir: bool, pub is_symlink: bool,
    pub size: u64, pub mtime_ms: i64, pub btime_ms: i64,
    pub hidden: bool, pub system: bool, // Windows attrs; remote => false
}
pub type VfsResult<T> = std::io::Result<T>;

pub trait Backend: Send + Sync {
    fn scheme(&self) -> Scheme;
    fn root_display(&self) -> String;                  // forward-slash root
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>>;
    fn stat(&self, path: &str) -> VfsResult<VfsMeta>;
    fn exists(&self, path: &str) -> bool { self.stat(path).is_ok() }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>>;
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>>;
    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        let mut r = self.open_read(src)?; let mut w = self.open_write(dst)?;
        Ok(std::io::copy(&mut r, &mut w)?)
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()>;
    fn remove_file(&self, path: &str) -> VfsResult<()>;
    fn remove_dir(&self, path: &str) -> VfsResult<()>;
    fn mkdir_all(&self, path: &str) -> VfsResult<()>;
    fn parallelism(&self) -> usize { rayon::current_num_threads() } // remote: 2-4
}
pub type BackendHandle = Arc<dyn Backend>;

// LocalBackend wraps today's std::fs calls (forward-slash <-> OS separator at
// the boundary; reuse scanner.rs get_attrs/ms_since_unix, hoist them into vfs.rs
// as meta_to_vfs). list_dir = std::fs::read_dir (FindFirstFileW fast path).

pub fn backend_for(root: &str) -> std::io::Result<BackendHandle> {
    let r = root.trim();
    if let Some(rest) = r.strip_prefix("sftp://") {
        Ok(Arc::new(crate::sftp::SftpBackend::connect(rest)?))
    } else if r.starts_with("ftp://") || r.starts_with("ftps://") {
        Ok(Arc::new(crate::ftp::FtpBackend::connect(r)?))
    } else {
        Ok(Arc::new(crate::vfs::LocalBackend::new(r)))  // drive paths + \\server\share UNC
    }
}
```

## 2. Routing (surgical edits)

- **`types.rs`**: `FileEntry` gains `pub scheme: Scheme` (set once per scan from
  `backend.scheme()`, copied per entry). Touches FileEntry construction in 3
  sites in `scanner.rs`, 1 in `copy.rs` (`start_copy_from_paths`), 1 in `app.rs`.
- **`scanner.rs`**: `start_scan` takes `(backend: BackendHandle, root: String, …)`
  instead of `root: PathBuf`. `Scanner` holds the `backend`. The hot inner loops
  in `walk_parallel`, `collect_recursive`/`walk_into_vec` replace
  `std::fs::read_dir`+`metadata` with `backend.list_dir(&dir)`. Paths joined with
  forward slashes (`format!("{dir}/{name}")`). Remote walks throttle width via
  `backend.parallelism()` (remote = single/few SSH channels, not num_cpus).
- **`copy.rs`**: `CopyOptions` gains `src_backend` + `dst_backend: BackendHandle`.
  In `run_copy`: `create_dir_all`→`dst.mkdir_all`, `exists`→`dst.exists`,
  `remove_file`→`dst.remove_file`, Copy→`dst.copy_file` (same Local backend) else
  stream `src.open_read`→`dst.open_write`; Move across different/remote backends
  ALWAYS degrades to copy-then-delete (mirror the existing EXDEV fallback).
  `start_copy_from_paths` stat/expand and `start_copy_pairs` route the same way.
- **`app.rs`**: `start_scan_navigated` builds the backend via `backend_for(root)`
  and passes it to `start_scan`; `confirm_copy`/paste pass backends. The notify
  **watcher must be disabled for remote roots** (no inotify equivalent) — fall
  back to manual rescan / coarse polling.
- **`folder_index.rs`**: leave as-is (local-only; remote dirs aren't indexed).

## 3. SFTP backend (`sftp.rs`)

Crates (NOTE the crypto-backend trap — §5):
```toml
russh = { version = "0.61", default-features = false, features = ["ring", "flate2", "rsa"] }
russh-sftp = "2.3"
tokio = { version = "1", features = ["rt", "net", "io-util", "time", "macros"] }
```
- Keys are re-exported as `russh::keys` (the old `russh-keys` is folded in).
  Load with `russh::keys::load_secret_key(path, passphrase: Option<&str>)` —
  handles OpenSSH + PEM, encrypted or not.
- **Auth (russh 0.49+ API):**
  - password: `session.authenticate_password(user, pass).await?`
  - keyfile: `let hash = session.best_supported_rsa_hash().await?.flatten();`
    `session.authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash)).await?`
  - both return `AuthResult`; check `.success()`.
- **Host key (trust-on-first-use):** implement `Handler::check_server_key(&mut self, &ssh_key::PublicKey)`
  → compare `server_public_key.fingerprint(..)` against a `known_hosts` file;
  accept-and-persist on first sight, error on mismatch (prompt the user via UI).
- **Open subsystem:** `channel_open_session` → `request_subsystem(true, "sftp")`
  → `SftpSession::new(channel.into_stream()).await`.
- **list_dir:** `sftp.read_dir(path).await?` yields `DirEntry`
  (`file_name()->String`, `metadata()` = `FileAttributes` with `size`, `mtime`,
  `is_dir()`); `btime`/hidden/system unavailable → default 0/false.
- **Sync↔async bridge:** ONE `tokio::runtime::Builder::new_current_thread().enable_all()`
  on a dedicated named `std::thread`, kept alive with `rt.block_on(pending())`;
  expose its `Handle`; Vfs sync methods do `handle.block_on(async {…})`. **Never**
  `block_on` from inside that runtime thread (panics). Do NOT use
  `tokio_util::io::SyncIoBridge` (it block_on's internally and needs
  spawn_blocking — conflicts with the current-thread/Handle model). Instead
  implement `std::io::Read::read` as `handle.block_on(file.read(buf))` in chunks.
- Connection pool keyed by `(host,port,user)` → `Arc<SftpSession>` in a Mutex.
- Set connect/read timeouts so a dead connection surfaces as `Err`
  (→ existing `record_failure` path) instead of freezing a worker.

## 4. FTP backend (`ftp.rs`) + network drives

- **FTP/FTPS:** `suppaftp = { version = "8", features = ["rustls"] }` — the
  default `FtpStream` is **blocking** (no runtime needed). Use `rustls`, NOT
  `native-tls` (avoids schannel/OpenSSL FFI on GNU; consistent with the project's
  no-native-link policy). `connect` → `login(user,pass)` → passive →
  `list`/`mlsd` parse → `retr`/`put` streams. Explicit/implicit FTPS via
  `connect_secure`.
- **Network drives:**
  - `\\server\share` UNC and mapped drive letters **already work** through
    `std::fs::read_dir` → route them to `LocalBackend` (zero new code). Only
    `sftp://`/`ftp://`/`ftps://` prefixes select a remote backend.
  - Authenticated connect by address: `WNetAddConnection2W` (mpr.dll) with a
    `NETRESOURCE` + user/password (no drive-letter mapping needed), then read via
    `std::fs`. windows-sys feature `Win32_NetworkManagement_WNet`.
  - **Local-network DISCOVERY (browsing the neighborhood) is unreliable on
    Win11** — the SMB1 Computer Browser is gone; `WNetOpenEnum`/`WNetEnumResource`
    and `NET VIEW` are widely broken. Connecting to a KNOWN address works;
    auto-discovery does not. Treat discovery as best-effort/optional; the reliable
    UX is "type the address."

## 5. Build de-risk — DO THIS FIRST

`russh`'s **default** crypto backend is `aws-lc-rs`, whose `aws-lc-sys` C build
needs NASM/CMake and breaks on the GNU/MinGW toolchain (and on paths with
spaces). So `default-features = false` + the **`ring`** feature is mandatory.
`ring 0.17.14` builds on `x86_64-pc-windows-gnu` without Perl/NASM — BUT ring has
a history of windows-gnu breakage (a reported ICE on stable-gnu in 2025). So:
**add the deps and run `cargo build` BEFORE writing integration code.** If the
graph doesn't compile, that's a toolchain decision (provision NASM for
aws-lc-rs, or pin a known-good ring) — not something to discover after the
refactor. `suppaftp` (blocking, rustls) and `tokio` are low-risk.

## 6. Credentials & UI

- A "Connect" dialog: protocol (SFTP/FTP/FTPS), host, port, user, auth
  (password | keyfile+passphrase), optional save.
- Store secrets via the **`keyring`** crate (Windows Credential Manager) — never
  plaintext. Saved connections list in the sidebar (like favorites/recent).
- Remote roots are URLs (`sftp://user@host/path`); `start_scan` already takes a
  string root, so navigation/history/breadcrumbs work unchanged once
  `backend_for` recognizes the scheme.

## Open questions to resolve during implementation

- Remote walk parallelism: single locked session vs. a small channel pool —
  spike against a real server (russh-sftp concurrency over one session).
- Whether to buffer whole files on SFTP `open_write` (BufWriter) vs chunked.
- Exact `russh 0.61` handshake/auth API — verify against docs.rs before coding
  (the sketch above matches 0.61.2 / russh-sftp 2.3 at research time).
