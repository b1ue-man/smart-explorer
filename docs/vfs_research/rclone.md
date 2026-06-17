# rclone: VFS-mount vs. sync/copy — lessons for Smart Explorer

> **Why rclone is the relevant analog.** rclone exposes the *same* "one tool, many cloud
> backends" model that Smart Explorer targets (SFTP, FTP, WebDAV, Google Drive, S3, B2, GCS,
> Dropbox, and dozens more), and it has offered both architectures for years:
> - **`rclone mount`** — a FUSE/WinFsp **virtual filesystem** over a remote, backed by a "VFS"
>   layer with selectable cache modes. This is the analog of our **option (A): on-demand /
>   placeholder VFS**.
> - **`rclone sync` / `bisync` / `copy`** — **download a real copy**, operate on it, then push
>   it back. This is the analog of our **option (B): download-edit-upload mirror/temp**.
>
> rclone has wrestled with exactly the open/edit/save question we face, and the docs + maintainer
> guidance are unusually explicit about the tradeoffs. Everything below is quoted verbatim with
> source URLs.

---

## 1. What rclone recommends `mount` FOR vs. `sync`/`copy` FOR

### `mount` is for: filesystem-style *access*, not reliable bulk transfer

The `mount` page itself frames mount as a convenience layer that is **fundamentally less reliable
than sync/copy**, because a filesystem API promises things cloud storage cannot:

> "File systems expect things to be 100% reliable, whereas cloud storage systems are a long way
> from 100% reliable. The rclone sync/copy commands cope with this with lots of retries. However
> rclone mount can't use retries in the same way without making local copies of the uploads."
> — https://rclone.org/commands/rclone_mount/

That sentence is the crux: **sync/copy gets to retry; a live mount does not** (unless a write
cache is buffering the upload locally). So:

- **Use `mount` when** an application needs a *file handle / path* to an object that lives
  remotely — open it in an editor, stream it, browse a tree — and you accept the reliability and
  latency caveats. This is the access pattern Smart Explorer's "browse remote and open a file"
  feature is.
- **Use `sync`/`copy` when** the goal is to move a known set of bytes reliably from A to B
  (backup, migration, archive), where retries and integrity checks matter more than live access.

### `bisync` is for: scheduled, deliberate two-way reconciliation — *not* live editing

rclone's bidirectional sync (`bisync`) is explicitly labeled an **advanced, dangerous** command,
intended for cron-style runs, not interactive file access:

> "bisync is considered an **advanced command**, so use with care. Make sure you have read and
> understood the entire manual (especially the Limitations section) before using, or **data loss
> can result**." — https://rclone.org/bisync/

> "Files that **change during** a bisync run may result in data loss" (mitigated, not eliminated,
> by the v1.66 snapshot model). — https://rclone.org/bisync/

Note also that core rclone historically did **not** have built-in continuous two-way sync; on the
forum, maintainer Nick Craig-Wood (ncw) said *"One day rclone will have bi-directional sync!"* and
users were steered to wrappers like `syncrclone` or to `mount` + a one-way sync.
— https://forum.rclone.org/t/rclone-mount-vs-sync/21975

**Forum consensus on "mount or sync for editing?"** The accepted pattern in
https://forum.rclone.org/t/rclone-mount-vs-sync/21975 is a **hybrid**: mount (with a real local
VFS cache) for day-to-day editing, plus a scheduled one-way `copy`/`sync` for durable archival.
ncw's own recommendation for making a mount usable for editing was to *add a real cache*:

> "You could use the vfs cache `--vfs-cache-mode full`. If you set a high value for
> `--vfs-cache-max-age` then the files will stay on your disk for ages."
> — Nick Craig-Wood, https://forum.rclone.org/t/rclone-mount-vs-sync/21975

And for anything two-way, maintainer `asdffdsa` warns:

> "I don't recommend doing this.... If you want to try then you want `rclone copy` not `rclone
> sync` to avoid data loss." — https://forum.rclone.org/t/rclone-mount-vs-sync/21975

**Takeaway for us:** rclone's own guidance is that a bare virtual mount is for *access*; the
moment you want to reliably *edit and save*, you end up reintroducing a **real local copy** — either
as a VFS write cache under the mount, or as an explicit sync/copy workflow.

---

## 2. Documented limitations of the mount / VFS (placeholder) approach

### 2.1 Why the VFS layer exists at all — objects aren't files

> "Cloud storage objects have lots of properties which aren't like disk files - you can't extend
> them or write to the middle of them, so the VFS layer has to deal with that."
> — https://rclone.org/commands/rclone_mount/

This is the single most important fact for our decision: **you cannot write to the middle of a
remote object, and you cannot extend it in place.** A placeholder-VFS has to *fake* a normal file
on top of an append-only / replace-only object store.

### 2.2 Without a cache, mounts can't do random writes — and "many applications won't work"

> "Without the use of `--vfs-cache-mode` this can only write files sequentially, it can only seek
> when reading. This means that many applications won't work with their files on an rclone mount
> without `--vfs-cache-mode writes` or `--vfs-cache-mode full`."
> — https://rclone.org/commands/rclone_mount/

In `--vfs-cache-mode off` (the default), the documented restrictions are:

> - Files can't be opened for both read AND write
> - Files opened for write can't be seeked
> - Existing files opened for write must have O_TRUNC set
> - Files open for read with O_TRUNC will be opened write only
> - Files open for write only will behave as if O_TRUNC was supplied
> - Open modes O_APPEND, O_TRUNC are ignored
> - If an upload fails it can't be retried
> — https://rclone.org/commands/rclone_mount/

Most real editors (and almost anything doing in-place saves, lock files, or temp-file-rename saves)
open files `O_RDWR` and seek — i.e. exactly what a cache-less placeholder mount forbids.

### 2.3 The whole-file-rewrite problem: a 1-byte change re-uploads the entire object

Because objects can't be patched in place, **any change re-uploads the whole file.** rclone's FAQ
("Why doesn't rclone support partial transfers / binary diffs like rsync?"):

> "Cloud storage systems (at least none I've come across yet) don't support partially uploading an
> object." — https://rclone.org/faq/

> "There is a 1:1 mapping between files on your hard disk and objects created in the cloud storage
> system." — https://rclone.org/faq/

> "It would be possible to make a sync system which stored binary diffs like rsync does, instead of
> whole objects, but that would break the 1:1 mapping of files on your hard disk to objects in the
> remote cloud storage system." — https://rclone.org/faq/

Maintainer confirmation on the forum feature request:

> "most cloud storages do not provide such functionality - update only part of a stored object"
> — https://forum.rclone.org/t/upload-the-difference-of-changed-files-instead-of-the-whole-thing/44774

Consequence (also documented as a real bug class): even a **metadata/modtime-only** touch can
force a full object rewrite on some backends —

> "When rclone updates modification time in destination (Google Cloud Storage), it rewrites an
> object, even though a metadata update would be enough."
> — https://forum.rclone.org/t/unexpected-rewrite-of-a-file-in-google-cloud-storage/42170

**For Smart Explorer:** whether we choose (A) or (B), saving a 1-byte edit to a 2 GB file means
re-uploading 2 GB. Neither architecture can avoid that on backends without in-place patch APIs.
The difference is *where the pain surfaces* — silently mid-save in a VFS, vs. visibly during an
explicit "upload" step in a mirror approach.

### 2.4 Attribute caching can corrupt files if the remote changes underneath

> "You may see corruption if the remote file changes length during this window. It will show up as
> either a truncated file or a file with garbage on the end. With `--attr-timeout 1s` this is very
> unlikely but not impossible. The higher you set `--attr-timeout` the more likely it is."
> — https://rclone.org/commands/rclone_mount/

This is a multi-writer / TOCTOU hazard inherent to caching attributes about a remote you don't
exclusively own — directly relevant if two clients (or a phone + desktop) touch the same file.

### 2.5 Not all backends support all operations (modtime, hash, empty dirs, etc.)

rclone's unified interface leaks backend differences. From the overview/features matrix:

> "Each cloud storage system is slightly different. Rclone attempts to provide a unified interface
> to them, but some underlying differences show through." — https://rclone.org/overview/

> "Storage systems with a `-` in the ModTime column, means the modification read on objects is not
> the modification time of the file when uploaded." — https://rclone.org/overview/

Backends lacking **ModTime** include Mega, Google Photos, Proton Drive, Seafile, Zoho and others;
backends lacking **Hash** include Zoho, Sia, iCloud Drive, Internxt and others
(https://rclone.org/overview/). And from the mount page itself:

> "Bucket-based remotes - Azure Blob, Swift, S3, Google Cloud Storage and B2 - can't store empty
> directories." — https://rclone.org/commands/rclone_mount/

Modtime/hash matter because they are how a VFS or a sync decides "is the local copy stale?" On a
backend with no real modtime and no hash, **change detection is unreliable** — which is precisely
where bisync's "data loss can result" warning bites.

### 2.6 Network latency surfaces as filesystem stalls; modtime/hash reads cost API calls

> "In particular S3 and Swift benefit hugely from the `--no-modtime` flag ... as each read of the
> modification time takes a transaction." — https://rclone.org/commands/rclone_mount/

> "`hash` is slow with the `local` and `sftp` backends as they have to read the entire file and
> hash it, and `modtime` is slow with the `s3`, `swift`, `ftp` and `qingstor` backends."
> — https://rclone.org/commands/rclone_mount/

A directory listing or a stat that's a microsecond on a local FS becomes a network round-trip
behind a VFS. Under FUSE this manifests as the whole app freezing while a `stat()` blocks — a
real UX trap for a file explorer that lists directories eagerly.

### 2.7 Write-back is deferred, so "saved" ≠ "on the remote"

> "Note that files are written back to the remote only when they are closed and if they haven't
> been accessed for `--vfs-write-back` seconds." (default 5s)
> — https://rclone.org/commands/rclone_mount/

The cache also has eviction policies (`--vfs-cache-max-age`, default 1h; `--vfs-cache-max-size`,
default off — https://rclone.org/commands/rclone_mount/), and the cache directory **must support
sparse files** or performance collapses:

> "IMPORTANT: not all file systems support sparse files. In particular FAT/exFAT do not. Rclone
> will perform very badly if the cache directory is on a filesystem which doesn't support sparse
> files and it will log an ERROR message if one is detected."
> — https://rclone.org/commands/rclone_mount/

---

## 3. The cache-mode spectrum — what it tells us about editing over a VFS

rclone's four cache modes are effectively a **dial from "pure placeholder VFS" (A) toward
"real local copy" (B)**. The intro is blunt about *why* the cache has to exist:

> "These flags control the VFS file caching options. File caching is necessary to make the VFS
> layer appear compatible with a normal file system."
> — https://rclone.org/commands/rclone_mount/

| Mode | Verbatim behavior (rclone.org) | Read+write? | Seek on write? | Retry failed upload? | Maps to our option |
|------|--------------------------------|-------------|----------------|----------------------|--------------------|
| `off` (default) | "read directly from the remote and write directly to the remote without caching anything on disk" | **No** | No (read-seek only) | **No** | Pure placeholder (A), barely usable for editing |
| `minimal` | "very similar to 'off' except that files opened for read AND write will be buffered to disk" | Partial | No (write-only can't seek) | **No** | Mostly (A) |
| `writes` | "files opened for read only are still read directly from the remote, write only and read/write files are buffered to disk first. This mode should support all normal file system operations." | **Yes** | Yes | **Yes** ("retried at exponentially increasing intervals up to 1 minute") | (A)+local write buffer |
| `full` | "all reads and writes are buffered to and from disk ... otherwise identical to `--vfs-cache-mode` writes" | **Yes** | Yes | **Yes** | Closest to (B): a real local copy |

Verbatim restriction lists for the weak modes:

`off`:
> Files can't be opened for both read AND write; Files opened for write can't be seeked; Existing
> files opened for write must have O_TRUNC set; ... If an upload fails it can't be retried.
> — https://rclone.org/commands/rclone_mount/

`minimal`:
> Files opened for write only can't be seeked; Existing files opened for write must have O_TRUNC
> set; Files opened for write only will ignore O_APPEND, O_TRUNC; If an upload fails it can't be
> retried. — https://rclone.org/commands/rclone_mount/

**What the spectrum tells us:**

1. **The default mode (`off`) is essentially unusable for editing.** No read+write, no write-seek,
   no retry. A pure on-demand placeholder, with no local materialization, cannot host a normal
   editor.
2. **The first mode that "should support all normal file system operations" is `writes`** — and
   it works *precisely because it stops being a placeholder and buffers the whole file to disk
   first.* So rclone's own answer to "make the VFS editable" is *"download a real (write) copy."*
3. **Retry — the reliability advantage sync/copy has — only returns at `writes`/`full`**, where a
   local copy exists to retry from. This re-confirms §1's point: editable + reliable ⇒ local copy.
4. **`full` adds buffering reads to disk too**, i.e. on-open hydration that persists — the closest
   the VFS gets to (B). Forum guidance (https://forum.rclone.org/t/vfs-cache-mode-full-vs-writes/34746)
   distinguishes them by read caching: `writes` reads read-only files straight from the remote;
   `full` caches reads on disk. Editors benefit from `full` because re-reads (lock checks, reloads,
   diffing on save) hit local disk instead of the network.

In short: **rclone makes the placeholder VFS editable only by progressively turning it into a
download-a-real-copy system.** The cache-mode dial *is* the (A)→(B) continuum.

---

## 4. Lessons for a multi-backend file explorer (open → edit → save)

**Bottom line: rclone's accumulated experience favors a real local copy for the edit/save
workflow.** A pure on-demand placeholder is fine for *browsing and read-only opening*, but every
limitation rclone documents pushes the *editing* path toward materializing the file on disk.

### Why a pure placeholder VFS (our option A) is risky for edit/save

- **Random writes don't work without a cache.** "many applications won't work ... without
  `--vfs-cache-mode writes` or `--vfs-cache-mode full`" (https://rclone.org/commands/rclone_mount/).
  Real editors do `O_RDWR` + seek + lock-file + atomic-rename-on-save dances that the cache-less
  VFS forbids.
- **No upload retry without a local copy.** "rclone mount can't use retries in the same way without
  making local copies of the uploads" (https://rclone.org/commands/rclone_mount/). A failed save
  to a flaky SFTP/WebDAV link can silently lose the user's edit.
- **Latency becomes app freezes.** Every stat/list/read is a network round-trip; backends charge
  API calls for modtime/hash (https://rclone.org/commands/rclone_mount/). Behind FUSE this looks
  like the editor hanging.
- **Attribute-cache corruption window** if the remote changes length under you
  (https://rclone.org/commands/rclone_mount/) — a real concern for shared/multi-device files.
- **Backend capability gaps** (no modtime / no hash on Mega, Proton, Zoho, Seafile, etc.;
  https://rclone.org/overview/) make "is my cached copy current?" unreliable — the same gap that
  makes `bisync` capable of *"data loss"* (https://rclone.org/bisync/).

### Why "download a real copy" (our option B) is what rclone effectively converges on

- rclone's *own* fix for "make the mount editable" is to **buffer the whole file to disk first**
  (`--vfs-cache-mode writes`/`full`) and only then is it documented to "support all normal file
  system operations" (https://rclone.org/commands/rclone_mount/). That is option (B) wearing a
  mount's clothing.
- A local copy is the only place rclone can **retry a failed upload**
  (https://rclone.org/commands/rclone_mount/), giving save-reliability the placeholder can't.
- The hybrid the forum settles on — *mount-with-cache for daily edits + scheduled one-way
  `copy`/`sync` for durable archive* (https://forum.rclone.org/t/rclone-mount-vs-sync/21975) — is
  exactly an open→edit-locally→upload→(retain backup) pipeline.

### Concrete design recommendations for Smart Explorer

1. **Default the edit/save path to "materialize a real copy," not a bare placeholder.** Mirrors
   rclone's `--vfs-cache-mode full` reality. Use placeholders only for the *listing/browse* tier
   and read-only previews.
2. **Treat "save" as an explicit, retried, whole-object upload.** Expect, and surface to the user,
   that *any* change re-uploads the whole object on most backends — "Cloud storage systems ...
   don't support partially uploading an object" (https://rclone.org/faq/). Show upload progress and
   retry on failure; never let a save fail silently.
3. **Keep the local copy until the upload is confirmed.** rclone defers write-back and keeps cached
   copies precisely so an upload can be retried (https://rclone.org/commands/rclone_mount/). Don't
   discard the user's bytes until the remote has acknowledged them.
4. **Don't rely on modtime/hash for staleness on backends that lack them** (Mega, Proton Drive,
   Seafile, Zoho, Google Photos, etc.; https://rclone.org/overview/). Track your own
   "downloaded-version" token / ETag where the backend offers one, and warn on conflict rather than
   auto-overwriting — bisync's "data loss can result" (https://rclone.org/bisync/) is the cautionary
   tale.
5. **Make latency explicit in the UI.** Stat/list/open are network operations; do them async with
   spinners. rclone's `--no-modtime` advice (https://rclone.org/commands/rclone_mount/) shows even
   listing costs transactions on S3/Swift/FTP — avoid eager per-file modtime/hash on directory
   render.
6. **Per-backend capability table, like rclone's.** Adopt rclone's "unified interface, but
   differences show through" stance (https://rclone.org/overview/): gate features (in-place modtime
   set, hash-based change detection, empty-dir handling — buckets "can't store empty directories",
   https://rclone.org/commands/rclone_mount/) on what each backend actually supports.

**One-line synthesis:** rclone proves you *can* present remote storage as a live filesystem, but
its decade of caveats shows that the **moment you allow editing-and-saving, you must back the file
with a real, retriable local copy** — the placeholder VFS is for *browsing and read-only access*,
and "download a real copy, edit, upload on save" is the architecture that actually survives flaky
networks and partial-update-incapable backends.

---

## Sources

- rclone mount command (Limitations, VFS, VFS File Caching, cache modes, attr-timeout, sparse
  files, write-back, modtime/hash perf, empty dirs): https://rclone.org/commands/rclone_mount/
- rclone bisync (advanced/dangerous, "data loss can result", change-during-run): https://rclone.org/bisync/
- rclone FAQ ("Why doesn't rclone support partial transfers / binary diffs", 1:1 mapping, no
  partial object upload): https://rclone.org/faq/
- rclone overview / features matrix (per-backend ModTime/Hash support, "differences show through"):
  https://rclone.org/overview/
- Forum: "Rclone mount vs Sync" (hybrid consensus, ncw on `--vfs-cache-mode full`, asdffdsa on
  copy-not-sync): https://forum.rclone.org/t/rclone-mount-vs-sync/21975
- Forum: "Vfs-cache-mode full vs writes" (read caching distinction): https://forum.rclone.org/t/vfs-cache-mode-full-vs-writes/34746
- Forum: "Upload the difference of changed files instead of the whole thing" (maintainer: cloud
  storage can't update part of an object): https://forum.rclone.org/t/upload-the-difference-of-changed-files-instead-of-the-whole-thing/44774
- Forum: "Unexpected rewrite of a file in Google Cloud Storage" (modtime update forces object
  rewrite): https://forum.rclone.org/t/unexpected-rewrite-of-a-file-in-google-cloud-storage/42170
