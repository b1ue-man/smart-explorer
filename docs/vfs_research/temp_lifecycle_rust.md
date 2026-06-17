# Temp-file lifecycle in Rust — safe, unique, reliably-cleaned

Research for Smart Explorer's "edit a remote file in its native app" flow:
download to a **local** temp, launch the OS default editor, watch for the user's
save, upload, then delete. This doc gives the idiomatic Rust crates/APIs and the
cross-platform correctness rules.

**Scope:** the *temp-copy-and-watch* mechanism (the `open_temp_path` /
`RemoteEdit` path described in `docs/REMOTE_EDIT.md` — the universal, any-app
fallback that ships alongside the Windows CfAPI placeholder root). The CfAPI
placeholder root is a separate, non-temp mechanism and is out of scope here.

**Crates:** `tempfile = "3"`, `notify = "8"` (+ `notify-debouncer-full`),
`uuid = "1"` (v4).

---

## 0. TL;DR design

```
<env::temp_dir()>/
  smart_explorer_edit/                 <- app temp ROOT (stable name)
    <session-uuid>/                    <- one per app run; swept on next startup
      <edit-uuid>/                     <- one TempDir per "open for edit", UNIQUE
        report.xlsx                    <- the REAL remote name + extension
```

- **Unique per edit:** one `tempfile::TempDir` per open (`<edit-uuid>`), created
  **atomically** by the OS — never a fixed/guessed path. The real filename lives
  *inside* that dir so editors keep the correct extension, with zero collision
  risk across opens or identically-named remotes.
- **No reuse:** a fresh `TempDir` every open; we never write into a pre-existing
  local path.
- **Save detection:** watch the **edit directory** (not the file) with `notify`
  + `notify-debouncer-full`, so editor "write-temp-then-rename" saves are caught.
- **Cleanup:** delete after a *successful* upload **and** once the editor no
  longer holds the file (Windows can't delete an open file); plus a **startup
  sweep** of stale session roots, because `TempDir`'s Drop does not run on crash
  / kill / `process::exit`.

---

## 1. `tempfile` — the crate APIs

> docs.rs (crate root): <https://docs.rs/tempfile/latest/tempfile/> ·
> `Builder`: <https://docs.rs/tempfile/latest/tempfile/struct.Builder.html> ·
> `TempDir`: <https://docs.rs/tempfile/latest/tempfile/struct.TempDir.html> ·
> `NamedTempFile`: <https://docs.rs/tempfile/latest/tempfile/struct.NamedTempFile.html>

### 1a. The unique-directory-per-edit scheme (the key recommendation)

We need a **unique directory** so we can keep the file's **real name + extension**
inside it (editors care about the extension; many refuse to associate or syntax-
highlight a random `.tmp` name). `tempfile::Builder::tempdir_in` makes that
directory atomically and uniquely:

> `tempdir_in` — "Attempts to make a temporary directory inside of `dir`. The
> directory and everything inside it will be automatically deleted once the
> returned `TempDir` is destroyed."
> — <https://docs.rs/tempfile/latest/tempfile/struct.Builder.html>

The randomized name (default **6** random bytes, tunable with `rand_bytes`)
guarantees uniqueness:

> `rand_bytes` — "Set the number of random bytes. Default: `6`."
> `prefix` — "Set a custom filename prefix … Default: `.tmp`."
> `suffix` — "Set a custom filename suffix … Default: empty."
> — <https://docs.rs/tempfile/latest/tempfile/struct.Builder.html>

Recommended call:

```rust
use tempfile::Builder;
use std::path::{Path, PathBuf};

/// Create a unique, private temp dir for one edit and place the file inside it
/// under its REAL remote name. Returns (the live TempDir guard, the file path).
fn stage_edit(session_root: &Path, remote_name: &str) -> std::io::Result<(tempfile::TempDir, PathBuf)> {
    // Unique directory, created atomically by the OS (no guessable fixed path).
    let dir = Builder::new()
        .prefix("edit-")     // human-recognizable in the temp tree
        .rand_bytes(12)      // extra collision margin over the default 6
        .tempdir_in(session_root)?;        // <session_root>/edit-<12 random>/

    // Keep the REAL name+extension so the editor opens it correctly.
    let file_path = dir.path().join(remote_name); // .../edit-XXXX/report.xlsx
    std::fs::write(&file_path, /* downloaded bytes */ b"")?;

    Ok((dir, file_path)) // hold `dir` alive for the whole edit session
}
```

Why a **dir** and not `NamedTempFile`/`Builder::tempfile` with a `.suffix`:
`NamedTempFile` would give `edit-<rand>.xlsx`, which preserves the *extension*
but not the *base name* the user recognizes (e.g. "report.xlsx"). The unique-dir
approach preserves the **exact** remote filename **and** extension while keeping
uniqueness in the directory layer. (It also lets a save-via-rename land a
sibling file in the same dir without colliding with anything else.)

> ⚠️ **Don't** keep the `TempDir` guard inside a generic wrapper that might drop
> early: "moving these types into generic APIs can cause premature deletion
> before intended use completes." Hold the guard for the whole edit session.
> — <https://docs.rs/tempfile/latest/tempfile/>

### 1b. Uniqueness / security guarantees (no mktemp race)

`tempfile` creates the entry **atomically** with `O_EXCL`-style semantics, which
is exactly what avoids the classic insecure-`mktemp` TOCTOU/symlink race
(CWE-377): there is no "check then create" window an attacker can win.

> Filename-prediction / DoS mitigation: the crate notes it would take "billions
> of files for collisions with 6 random characters" and that it **re-seeds after
> 3 creation failures** when `getrandom` is enabled.
> — <https://docs.rs/tempfile/latest/tempfile/> (Security)

> ⚠️ **Files are private; directories are not (Unix).** "Temporary _files_
> created with this library are private by default on all operating systems.
> However, temporary _directories_ are created with the default permissions and
> will therefore be world-readable by default unless the user has changed their
> umask."
> — <https://docs.rs/tempfile/latest/tempfile/> (Security → Access Permissions)

Implication for us: the **directory** holding the user's (possibly confidential)
downloaded remote file is world-**readable** by default on Unix `/tmp`. If the
file is sensitive, after creating it tighten perms to `0700`/`0600`, e.g.:

```rust
#[cfg(unix)]
{
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))?;
    std::fs::set_permissions(&file_path, std::fs::Permissions::from_mode(0o600))?;
}
```

On Windows the per-user `%TEMP%` is already user-scoped, so this is a Unix
concern. See §3.

### 1c. Persist / keep / close — the lifecycle verbs

| API | Signature (verbatim) | Use |
|---|---|---|
| `TempDir::close` | `pub fn close(self) -> Result<()>` — "Closes and removes the temporary directory, returning a `Result`. Although `TempDir` removes the directory on drop, in the destructor any errors are ignored." | **Preferred explicit cleanup** — lets us detect/log delete failures. |
| `TempDir::keep` (was `into_path`) | `pub fn keep(self) -> PathBuf` — "Persist the temporary directory to disk … This consumes the `TempDir` without deleting directory on the filesystem." | Hand ownership to our own sweeper (so Drop won't fight us). |
| `TempDir::disable_cleanup` | `pub fn disable_cleanup(&mut self, disable_cleanup: bool)` | Debug only. |
| `NamedTempFile::persist` | `pub fn persist<P>(self, new_path: P) -> Result<F, PersistError<F>>` — "Persist the temporary file at the target path. If a file exists at the target path, persist will atomically replace it." | If you ever stage downloads *to a local path* atomically. |
| `NamedTempFile::keep` | `pub fn keep(self) -> Result<(F, PathBuf), PersistError<F>>` — "turn the temporary file into a non-temporary file without moving it." | (`keep()` on `Builder` is **deprecated**: "Use `Builder::disable_cleanup`".) |
| `NamedTempFile::into_temp_path` | "Closes the file, leaving only the temporary file path. This is useful when another process must be able to open the temporary file." | Hand the path to the editor (release our handle first — see §4 Windows). |
| `NamedTempFile::close` | `pub fn close(self) -> Result<()>` — "Close and remove the temporary file. Use this if you want to detect errors in deleting the file." | Error-checked single-file delete. |

Sources: <https://docs.rs/tempfile/latest/tempfile/struct.TempDir.html>,
<https://docs.rs/tempfile/latest/tempfile/struct.NamedTempFile.html>.

**Recommendation:** prefer **`TempDir::close()`** for our deliberate post-upload
cleanup so deletion errors are surfaced/logged, and **never** rely solely on Drop
(see §5).

---

## 2. Why not a fixed path — TOCTOU / symlink / CWE-377

Writing to a predictable, fixed local path (`%TEMP%\smart_explorer_edit.tmp`) is
the textbook **insecure temporary file** weakness, **CWE-377**:

> "A TOCTOU race condition may occur if an attacker creates a file with the same
> name as the one the application intends to generate before the application
> does." … "On Unix based systems, an attacker can pre-create the file as a link
> to another important file, and if the application truncates or writes data to
> the file, it may unwittingly perform damaging operations for the attacker." …
> "If temporary files have predictable names, attackers can guess the file names
> and either read or modify the file contents."
> — CWE-377: <https://cwe.mitre.org/data/definitions/377.html>

`tempfile`'s atomic, randomized creation defeats all three (predictable name,
pre-create race, symlink-redirect) by **binding to the object at creation** with
no check-then-use gap — the documented mitigation ("bind early to a stable
reference … which prevents later namespace changes from redirecting the
operation"). This is also why we must **not reuse** a prior local file: reuse
reintroduces a known, guessable path and risks acting on attacker-substituted
content.

---

## 3. Cross-platform temp location

We anchor everything under `std::env::temp_dir()`:

- **Windows:** resolves the per-user temp via `GetTempPath` → `%TMP%`, `%TEMP%`,
  `%USERPROFILE%`… i.e. typically `C:\Users\<user>\AppData\Local\Temp`, which is
  **already user-scoped** (not world-readable). Good default.
- **Unix:** `$TMPDIR` if set, else `/tmp`. `/tmp` is **shared and world-readable**;
  a fixed path there is the classic attack surface. We scope a **private
  subdir** (`smart_explorer_edit/<session>/<edit>`) and, for sensitive content,
  `chmod 0700` the dir (§1b). On macOS, GUI apps usually get a per-user
  `$TMPDIR` under `/var/folders/...` which is already private.

`tempfile` honors the same temp dir and exposes
`env::override_temp_dir` to relocate it app-wide if needed:

> "Applications can mitigate the issues described below by using
> `env::override_temp_dir` to change the default temporary directory."
> — <https://docs.rs/tempfile/latest/tempfile/> (Security)

```rust
let root = std::env::temp_dir().join("smart_explorer_edit");
std::fs::create_dir_all(&root)?;            // app temp ROOT (stable name)
let session_root = root.join(session_uuid); // per-run, via uuid::Uuid::new_v4()
std::fs::create_dir_all(&session_root)?;
```

---

## 4. Save detection — `notify`, and **watch the directory**

> notify: <https://docs.rs/notify/latest/notify/> ·
> README: <https://github.com/notify-rs/notify> ·
> `Watcher::watch`: <https://docs.rs/notify/latest/notify/trait.Watcher.html>

**Backends (cross-platform, native, no polling unless asked):**

> "Linux / Android: inotify; macOS: FSEvents or kqueue, see features; Windows:
> ReadDirectoryChangesW; iOS / FreeBSD / NetBSD / OpenBSD / DragonflyBSD:
> kqueue; All platforms: polling"
> — <https://github.com/notify-rs/notify> (README)

Use `notify::recommended_watcher` to pick the right backend automatically:

```rust
let mut watcher = notify::recommended_watcher(tx)?;
watcher.watch(Path::new("."), RecursiveMode::Recursive)?;
```
— <https://docs.rs/notify/latest/notify/>

### The editor "write-temp-then-rename" hazard → watch the DIR, not the file

Many editors save by writing a new temp file and **renaming it over** the
original (atomic save). If you watch the *file path/inode*, the watch can be left
pointing at the now-unlinked old inode and **miss** the save:

> "On some platforms, if the `path` is renamed or removed while being watched,
> behaviour may be unexpected. See discussions in #165 and #166. … If less
> surprising behaviour is wanted one may non-recursively watch the _parent_
> directory as well and manage related events."
> — <https://docs.rs/notify/latest/notify/trait.Watcher.html>

The crate explicitly designs for the atomic-save pattern, and the fix is to
watch the **containing directory**:

> "A practical example would be the safe-saving of a file, where a temporary file
> is created and written to, then only when everything has been written to that
> file is it renamed to overwrite the file that was meant to be saved."
> (notify consolidates these into logical events.)

**Because each edit gets its own dedicated directory (§1a), watching that
directory is both correct *and* tightly scoped** — events can only concern our
one file (and the editor's sibling temp/rename of it). Watch the **edit dir**
non-recursively:

```rust
watcher.watch(edit_dir, RecursiveMode::NonRecursive)?;
// On a relevant Create/Modify/Rename(To) for our filename -> trigger upload.
```

### Debounce (don't upload on every keystroke-driven flush)

> "If you want debounced events (or don't need them in-order), see
> notify-debouncer-mini or notify-debouncer-full."
> — <https://docs.rs/notify/latest/notify/>

Use **`notify-debouncer-full`** (path-aware, coalesces the rename pair and rapid
writes) with a small timeout. The shipped code already uses a ~1.5 s debounce
(`docs/REMOTE_EDIT.md`); keep that ballpark. Debouncing also lets the editor's
multi-step save (`write tmp` → `rename over`) settle into **one** upload.

```rust
use notify_debouncer_full::{new_debouncer, DebounceEventResult};
use std::time::Duration;

let (tx, rx) = std::sync::mpsc::channel();
let mut debouncer = new_debouncer(Duration::from_millis(1500), None, tx)?;
debouncer.watch(edit_dir, RecursiveMode::NonRecursive)?;
for res in rx {
    if let DebounceEventResult::Ok(events) = res {
        if events.iter().any(|e| e.paths.iter().any(|p| p == &file_path)) {
            upload(&file_path)?; // re-upload to remote
        }
    }
}
```

**mtime polling** is the fallback (`notify`'s `PollWatcher`) — simpler, works on
exotic/networked FSes inotify can't watch, but laggy and CPU-costly. Prefer the
native backend; offer polling only as an escape hatch.

---

## 5. Deletion-while-open hazard + reliable cleanup

### 5a. Deleting a file the editor still has open

`std::fs::remove_file`:

> "Note that there is no guarantee that the file is immediately deleted (e.g.,
> depending on platform, other open file descriptors may prevent immediate
> removal)." … "This function currently corresponds to the `unlink` function on
> Unix. On Windows, `DeleteFile` is used or `CreateFileW` and
> `SetInformationByHandle` for readonly files."
> — <https://doc.rust-lang.org/std/fs/fn.remove_file.html>

Platform split that drives our cleanup timing:

- **POSIX (`unlink`):** succeeds even while the editor holds the file open; the
  name disappears immediately, and the inode/space is freed **only when the last
  open handle closes**. "the `unlink()` function is guaranteed to unlink the
  file from the file system hierarchy but keep the file on disk until all open
  instances of the file are closed." So deleting our dir post-upload "works,"
  but disk space lingers until the editor exits — and if the editor later saves
  again it'll be writing to an unlinked file (lost data). **So even on POSIX,
  don't delete while the editor may still save.**
- **Windows (`DeleteFile`):** if the file is open **without** `FILE_SHARE_DELETE`
  (common for editors), delete **fails** with a sharing violation, or is left in
  "delete-pending" so the name lingers; `remove_dir_all` then can't remove the
  parent. (Windows opens default to `FILE_SHARE_READ | WRITE | DELETE`, but the
  *editor's* sharing mode is what governs, and editors frequently lock.)
  — <https://github.com/rust-lang/rust/issues/29497>,
  <https://doc.rust-lang.org/std/os/windows/fs/trait.OpenOptionsExt.html>

**Consequence:** "delete the temp right after upload" is unsafe if the editor may
still be open. Recommended policy:

1. **Release our own handle first.** We only need the bytes to upload; close our
   `File`/`NamedTempFile` (or use `into_temp_path`) before attempting any delete,
   so *we* aren't the locker.
2. **Don't delete on first save.** Keep the edit dir alive while the editor is
   open (the user may save repeatedly). Treat each save as an upload, not an end.
3. **Delete when the edit session ends** — i.e. when the user closes the editor
   / detaches the edit, *and* the last upload succeeded. If we can detect the
   editor process exit (we launched it), delete then. Otherwise defer to (4).
4. **Best-effort + retry + defer.** On Windows, if `TempDir::close()` /
   `remove_dir_all` hits a sharing violation, retry a few times with backoff;
   if still locked, **leave it for the startup sweep** rather than busy-wait.

### 5b. Drop does NOT run on crash → mandatory startup sweep

`TempDir`/`NamedTempFile` clean up via destructors, which **do not run** on
abnormal exit:

> "TempDir and NamedTempFile will fail if their destructors don't run." …
> "If the program exits before the `NamedTempFile` destructor is run, the
> temporary file will not be deleted. This can happen if the process exits using
> `std::process::exit()`, a segfault occurs, receiving an interrupt signal like
> `SIGINT` that is not handled, or by using a statically declared
> `NamedTempFile` instance."
> — <https://docs.rs/tempfile/latest/tempfile/>

> "Note that if the program exits before the `TempDir` destructor is run, such
> as via `std::process::exit()`, by segfaulting, or by receiving a signal like
> `SIGINT`, then the temporary directory will not be deleted."
> — <https://docs.rs/tempfile/latest/tempfile/struct.TempDir.html>

Therefore the **session-root + startup-sweep** pattern is required for reliable
cleanup, not optional:

```
<temp>/smart_explorer_edit/<session-uuid>/<edit-uuid>/report.xlsx
```

- Each app run picks a fresh `<session-uuid>` (`uuid::Uuid::new_v4()`).
- **On startup**, before creating this run's session dir, enumerate sibling
  `<session-uuid>` dirs under `smart_explorer_edit/` and delete any that are
  **not** the current run and not locked. Those are crash/kill leftovers. (We
  can additionally write a lock/pid file per session and only sweep sessions
  whose process is gone, to avoid deleting a *concurrent* instance's dirs.)
- **On normal exit**, explicitly `TempDir::close()` each live edit dir (or
  `remove_dir_all` the session root) and log errors — don't depend on Drop
  order.

```rust
fn sweep_stale_sessions(root: &Path, current: &str) {
    if let Ok(entries) = std::fs::read_dir(root) {
        for e in entries.flatten() {
            let name = e.file_name();
            if name != *current /* && !pid_alive(&e.path()) */ {
                // best-effort; Windows may still hold a lock from a zombie editor
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}
```

This makes cleanup robust to the cases tempfile's Drop can't cover, while
`TempDir::close()` covers the normal path with error visibility.

---

## 6. Putting it together (recommended flow)

1. **Startup:** `root = temp_dir()/smart_explorer_edit`; `sweep_stale_sessions`;
   create `session_root = root/<session-uuid>` (write a pid/lock file).
2. **Open for edit:** `Builder::new().prefix("edit-").rand_bytes(12)
   .tempdir_in(&session_root)` → unique dir; write downloaded bytes to
   `dir.path().join(real_name)`; (Unix) `chmod 0700/0600`; **drop our file
   handle**.
3. **Watch:** `notify-debouncer-full` on the **edit dir**
   (`RecursiveMode::NonRecursive`), ~1.5 s debounce; on an event hitting our
   filename → upload to remote.
4. **End of edit** (editor closed / user detaches) **after last upload ok:**
   `TempDir::close()` the edit dir; on Windows sharing-violation, retry/backoff,
   else leave for the next startup sweep.
5. **Normal exit:** close all live edit dirs / `remove_dir_all` session root +
   pid file. Crash/kill → next run's sweep cleans it.

Invariants satisfied: **unique id per edit** (atomic `tempdir_in`, 12 random
bytes), **no reuse** (fresh dir every open), **real name+extension preserved**
(file lives inside the unique dir), **reliable cleanup** (post-upload close +
startup sweep), **no premature deletion** (delete only after a successful upload
and once the editor isn't holding the file).

---

## Sources

- tempfile (crate root, Security, Resource Leaking): <https://docs.rs/tempfile/latest/tempfile/>
- tempfile `Builder` (prefix/suffix/rand_bytes/tempdir_in): <https://docs.rs/tempfile/latest/tempfile/struct.Builder.html>
- tempfile `TempDir` (close/keep/disable_cleanup, Drop limits): <https://docs.rs/tempfile/latest/tempfile/struct.TempDir.html>
- tempfile `NamedTempFile` (persist/keep/into_temp_path/close): <https://docs.rs/tempfile/latest/tempfile/struct.NamedTempFile.html>
- CWE-377 Insecure Temporary File (TOCTOU/symlink/predictable names): <https://cwe.mitre.org/data/definitions/377.html>
- notify (backends, debouncer pointers): <https://docs.rs/notify/latest/notify/> · <https://github.com/notify-rs/notify>
- notify `Watcher::watch` (watch parent dir vs file; #165/#166): <https://docs.rs/notify/latest/notify/trait.Watcher.html>
- std `remove_file` (no immediate-delete guarantee; unlink vs DeleteFile): <https://doc.rust-lang.org/std/fs/fn.remove_file.html>
- Windows open-file delete / remove_dir_all sharing issues: <https://github.com/rust-lang/rust/issues/29497> · <https://doc.rust-lang.org/std/os/windows/fs/trait.OpenOptionsExt.html>
