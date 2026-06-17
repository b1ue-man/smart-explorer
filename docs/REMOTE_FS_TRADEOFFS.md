# Remote file access: on-demand placeholders vs. download-a-real-copy

**Question:** for Smart Explorer — a file explorer that browses remote backends
(SFTP/FTP/WebDAV/Google Drive) and lets you open/edit files — should remote
files be accessed as **(A)** on-demand placeholders in a virtual filesystem
(OneDrive Files-On-Demand / Windows Cloud Files API style), or **(B)** downloaded
to a real local copy that you edit and save back?

This is a well-trodden problem. I researched what the teams who built these at
scale concluded. Four cited deep-dives are in `docs/vfs_research/`
(`rclone.md`, `cfapi_onedrive.md`, `dropbox_fileprovider.md`,
`drive_stream_mirror.md`). This file is the synthesis. Date: 2026-06-17.

---

## TL;DR

**The expert consensus is unanimous: placeholders are for *browsing huge trees*;
a *real local copy* is the right model for *editing a file*. The winning design
is the hybrid — cheap metadata browse, then download-a-real-copy on open — which
is exactly Google Drive for Desktop's own split, what rclone converges on, and
what Smart Explorer already does (Temp/mirror mode + the 0.5.36 conflict guard).**

CfAPI's one real advantage (instant browse of a massive tree without downloading)
Smart Explorer **already gets for free** via per-backend metadata listing — with
none of the cost. So CfAPI buys us essentially nothing for our workload while
demanding everything (an always-running registered provider, full callback
surface, durable machine state, NTFS-only kernel minifilter), and its editing
story is broken (see `docs/CFAPI_REVIEW.md`).

---

## What each source concluded (with the load-bearing quotes)

**rclone** — the closest analog (same many-cloud-backend model). Its
`--vfs-cache-mode` dial *is* the A→B continuum, and only the "real copy" end
works for editing:
- "Many applications won't work with their files on an rclone mount without `--vfs-cache-mode writes` or `--vfs-cache-mode full`." — the mode that "supports all normal file system operations" does so by buffering the **whole file to a real local copy** first.
- Cloud objects can't be patched: a 1-byte edit re-uploads the entire object.
- Verdict: mount/placeholders for browse + read-only; **download-a-real-copy for the open/edit/save workflow.**
- Sources: rclone.org/commands/rclone_mount, /bisync, /overview, forum threads.

**Microsoft CfAPI / OneDrive Files-On-Demand** — the designers' own framing:
- CfAPI is for "a sync engine… a service that syncs files… and present[s] those files… through the Windows file system and File Explorer." It is the **OneDrive architecture**, not a one-file fetch.
- "After a call to `CfDisconnectSyncRoot` returns… the platform will fail any operation that depends on said callbacks." → **no always-running provider, no file.**
- A placeholder is "only available if the sync service is available"; an implicitly-hydrated file "could be dehydrated… if space is needed." Only an explicitly **pinned** file is "guaranteed to be available offline" — and that is just approach (B).
- Desktop-only, NTFS-only (`cldflt.sys`), full callback surface (fetch + notify rename/delete/dehydrate), durable registration that can wedge Explorer if mishandled; the official sample "is not intended to be used as production code."
- Sources: learn.microsoft.com/.../build-a-cloud-file-sync-engine, /cfdisconnectsyncroot, /ne-cfapi-cf_callback_type.

**Dropbox (Project Infinite / Smart Sync) + Apple File Provider** — the hard-won
lessons of actually building virtual filesystems:
- Dropbox went into the **kernel** because FUSE was too slow ("any file operation usually requires an extra user-kernel mode switch"), accepted that "any bug introduced to the kernel can adversely affect the whole machine," then had to **re-platform** onto File Provider — moving the folder to `~/Library/CloudStorage` and breaking apps with hard-coded paths.
- Apple deprecated VFS kexts; even File Provider "is not universally" a replacement. **Partial I/O is unsupported**: "Files can either be in the cloud or downloaded. It is not possible to download/read only a portion of a file."
- **Atomic-save (write-temp-then-rename) breaks placeholder mounts** (documented breaking KeePassXC on a Drive mount; CfApi makes even "a small MS Office document… very slow"). Latency surfaces as hangs / `EDEADLK` (a recent Claude Code bug on dataless files).
- "To make ordinary apps work you must fall back to caching the whole real file" → you converge on (B) anyway. Worth it **only** for an always-running platform with funding for a perpetual per-OS maintenance treadmill.
- Sources: dropbox.tech (Project Infinite, rewriting-sync-engine), developer.apple.com/forums (File Provider), github linuxmint/cinnamon#13555, userfilesystem.com FAQ.

**Google Drive for Desktop** — ships **both** modes and documents when to use each:
- **Stream** = "stored in the cloud… local storage only used when you work on files"; "only available online unless specifically made available offline."
- **Mirror** = "stored in the cloud and on your local hard drive"; "available offline and online… even without an internet connection or when the Drive for desktop app isn't running."
- Guidance: "Use Streaming when you want to save disk space and mainly browse… use Mirroring when you need reliable offline access or fast access… [for editing/automation]."
- **Editing directly over a network mount is independently dangerous** — SQLite: "some filesystems contain bugs in their locking logic… especially… network filesystems"; WebDAV silently overwrites concurrent edits; SMB metadata caching corrupts shared files. The safe pattern is local edit + atomic rename + single upload.
- Sources: support.google.com/drive/answer/13401938, sqlite.org/howtocorrupt, sabre-io/dav#1294, h2database#1935, MS SMB2 KB.

---

## Tradeoff matrix (condensed; full per-cell cites in `drive_stream_mirror.md` §4)

| Axis | On-demand placeholder (A) | Download real copy (B) |
|------|---------------------------|------------------------|
| Browse latency / huge trees | Excellent (metadata-only, ~1 KB placeholders) | We already get this via metadata listing; copy only on open |
| Offline / app-closed | Online-only unless pinned (= B anyway) | Plain file, always available |
| Edit/save correctness | Write-through invites partial-write/lock/conflict hazards | Edit local file, atomic save, one upload, etag/mtime conflict check |
| App compatibility | Transparent **only** with deep OS integration; reparse/automation quirks | Maximum — an ordinary file, zero hooks |
| Implementation / lock-in | High, OS-specific ×3 (Win NTFS kernel filter, mac File Provider, Linux FUSE/gvfs) | Low, portable, one code path |
| When helper not running | File inaccessible | File is just there |

A wins only for: **huge trees you mostly browse, limited disk, read-mostly, and
you're willing to build+maintain per-OS placeholder integration.** B wins for:
**editing, app compat, offline, simplicity/portability, and a non-daemon app** —
which is Smart Explorer.

---

## Recommendation for Smart Explorer

1. **Keep download-a-real-copy (Temp/mirror) as the default for open/edit.** It is
   what every source endorses for this workload, it's portable, and it's already
   shipped — now with the etag/mtime **conflict guard** (0.5.36) that the experts
   specifically call for. (We can upgrade the guard from mtime to an etag/hash
   compare per backend later for extra precision.)
2. **Keep cheap metadata browsing** (already how every backend lists) — that *is*
   the legitimate, free version of "Stream's" instant-browse advantage, with none
   of the placeholder cost.
3. **Do not invest in CfAPI as the editing path.** Per `docs/CFAPI_REVIEW.md` it's
   architecturally a sync-engine platform; for a non-daemon app it's all cost and
   a broken save story. The safety fixes already shipped (0.5.35) mean the
   experimental toggle can't crash — that's where it should stay, or be removed.
4. **If "remote drives visible in Explorer itself" ever becomes a goal**, that is a
   *separate, deliberate* product direction: an always-running provider built
   properly on the sanctioned per-OS APIs (Windows CfAPI / macOS File Provider) —
   never a hand-rolled FUSE/kext, the exact thing Dropbox and Apple moved away
   from. It is not the way to "edit a remote file."

**Bottom line: the research says we're already on the right architecture.** The
question "should we use a placeholder VFS?" resolves to "only if we decide to
become an always-running cloud sync platform" — which is a different product.
For an explorer that opens and edits remote files, download-a-real-copy is the
expert-endorsed answer.
