# Google Drive Stream vs. Mirror — and the On-Demand-Placeholder vs. Download-Real-Copy Tradeoff

Research for **Smart Explorer** (Rust remote file explorer: SFTP/FTP/WebDAV/Google Drive).
Decision under study: **(A)** on-demand placeholder / streaming VFS vs **(B)** download-a-real-local-copy (mirror/temp) and save back.

Google ships **both** modes in Drive for Desktop and documents when to use each, so it is the
canonical real-world reference. Everything below is cited with exact URLs and verbatim quotes.

Date compiled: 2026-06-17.

---

## 1. Google's official "Stream vs. Mirror" guidance (verbatim)

**Primary source — Google Drive Help, "Stream & mirror files with Drive for desktop":**
https://support.google.com/drive/answer/13401938?hl=en

Google describes the two modes with these exact lines:

**Stream files**
- Storage: *"Files and folders are stored in the cloud. Local storage is only used when you work on files on your computer, or for recently and frequently used files."*
- Availability: *"Files are only available online unless specifically made available offline through Drive for desktop."*

**Mirror files**
- Storage: *"Files and folders are stored in the cloud and on your local hard drive."*
- Availability: *"Files are available offline and online."*

Google also documents the offline / "app not running" advantage of Mirror verbatim:

> *"Access your cloud files any time, even without an internet connection or when the Drive for desktop app isn't running."*
— https://support.google.com/drive/answer/13401938?hl=en

**Hard constraints on which content can use which mode** (verbatim, same page):
- *"My Drive can be streamed or mirrored."*
- *"Shared Drives can only be streamed."*
- *"Other folders on your device can only be mirrored."*

### Documented downsides, summarized from the verbatim text
- **Downside of Stream:** files are *"only available online"* by default (needs the network); local copy
  exists only when you open a file or explicitly mark it *available offline*; and (see §2) the Drive
  app must be running.
- **Downside of Mirror:** files are *"stored in the cloud and on your local hard drive"* — i.e. it
  **uses local disk** for the full set of mirrored content.

### Google's own "which to choose" framing
Google frames Stream as the disk-saving / cloud-first default (*"Minimize hard drive usage and safely
store content in the cloud"*) and Mirror as the always-available-offline option
(https://support.google.com/drive/answer/13401938?hl=en). Practitioner distillations agree:

> *"Use Streaming when you want to save local disk space and mainly browse or occasionally open files;
> use Mirroring when you need reliable offline access or frequent, fast access to large files and
> scheduled batch processing."*
— buralog, "Streaming vs Mirroring: Practical Decision Criteria for Choosing Drive for Desktop",
https://buralog.jp/en/streaming-vs-mirroring-en/

A second independent comparison states the same disk/offline split:

> *"all your Google Drive contents are stored in the cloud. And files will not take up space on your
> computer until you open them"* (Stream) vs *"all the Drive documents are saved on both your computer
> and Google Drive. And the data will take up space on your computer"* (Mirror), with the recommendation:
> *"If you don't have much space on the local hard drive, it is recommended to pick streaming files,
> which will help to save the local space on your computer drive."*
— CBackup, "Stream Files vs. Mirror Files",
https://www.cbackup.com/articles/stream-files-vs-mirror-files.html

---

## 2. The "app not running" axis — Stream's most load-bearing downside

This is the single biggest argument against a pure on-demand VFS for an *editor*, and it is documented
by both Google and Microsoft.

**Google (Stream requires the running helper):** Mirror's selling point is explicitly that it works
*"even without an internet connection or when the Drive for desktop app isn't running"*
(https://support.google.com/drive/answer/13401938?hl=en). The implication, stated plainly by
practitioners, is that **Stream-only files are not present when the helper isn't running**.

**Microsoft Cloud Files API (cfapi) — the canonical placeholder model**, "Build a Cloud Sync Engine
that Supports Placeholder Files":
https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine

Microsoft defines the three file states verbatim:
> - **Placeholder file**: *"An empty representation of the file and only available if the sync service is available."*
> - **Full file**: *"The file has been hydrated implicitly and could be dehydrated by the system if space is needed."*
> - **Pinned full file**: *"The file has been hydrated explicitly by the user through File Explorer and is guaranteed to be available offline."*

So even on the most polished OS-integrated placeholder platform, a placeholder is *"only available if
the sync service is available"*, and an implicitly-hydrated full file *"could be dehydrated by the
system if space is needed"* — i.e. it can silently revert to needing the network. Only a **pinned**
(explicitly downloaded) file is *"guaranteed to be available offline"* — which is exactly approach (B).

**Apple File Provider (macOS)** — same model, same constraint. Apple's term for downloading content
on demand is *"materialising"* a *"dataless"* file, and a non-functioning provider extension yields
`cannotMaterialize` errors when an app tries to open a placeholder
(Apple developer forum thread "Error when materializing files",
https://developer.apple.com/forums/thread/802063). Apple also forced all third-party providers into
`~/Library/CloudStorage`, breaking apps that hard-coded the old paths (TidBITS, "Apple's File Provider
Forces Mac Cloud Storage Changes", https://tidbits.com/2023/03/10/apples-file-provider-forces-mac-cloud-storage-changes/).

---

## 3. App compatibility — does the app see a "real" file?

On-demand placeholders *can* be made app-transparent, but **only via deep OS integration**, not by a
naive userspace VFS.

**Microsoft cfapi (best case for on-demand):**
> *"Placeholder files present as typical files to apps and to end users in the Windows Shell."*
>
> *"Placeholder files are vertically integrated from the Windows kernel up to the Windows Shell, and
> app compatibility with placeholder files is generally a non-issue. Whether you use file system APIs,
> the Command Prompt, or a desktop or a UWP app to access a placeholder file, the file will hydrate
> without additional code changes and that app can use the file normally."*
> — https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine

But Microsoft also documents a real compatibility *gotcha* with the placeholder mechanism:
> *"The cloud files API implements the placeholder system using reparse points... many existing
> applications hit errors when encountering any reparse point. To mitigate this compatibility issue,
> the cloud files API always hides its reparse points from all applications except for sync engines..."*
> — same page.

**Google Stream — the practitioner warning** is that some apps/automation do *not* gracefully trigger
hydration and break on "online-only" files:
> *"Excel external references or batch processes may fail when such files are needed"* because files
> remain *"online-only"* without local copies; the fix is to *"point Power Query to local mirrored
> folders when data refresh is automated"* and to ensure *"all referenced files are present locally.
> Streaming may leave files 'online-only' which causes failures."*
> — buralog, "Google Drive for Desktop: Streaming vs Mirroring — Operational Differences for Business Use",
> https://buralog.jp/en/drive-streaming-vs-mirroring-en/

**Takeaway for Smart Explorer:** a *real local copy* (B) is seen by every app as an ordinary file with
zero OS hooks. A placeholder (A) is only app-transparent if you implement cfapi / File Provider /
gvfs-style integration; a hand-rolled FUSE/virtual mount risks the same "app didn't expect this" class
of failures, plus all the network-FS editing problems in §5.

---

## 4. Tradeoff matrix — On-demand placeholder (A) vs. Download real copy (B)

| # | Axis | On-demand placeholder / streaming VFS (A) | Download real local copy (B) | Source(s) |
|---|------|-------------------------------------------|------------------------------|-----------|
| 1 | **Time-to-first-byte / browse latency** | Excellent: listing a huge tree creates 1 KB placeholders, no download. cfapi placeholders *"consume only 1 KB of storage for the filesystem header"*. Browsing is metadata-only. | Poor if you mirror eagerly (must download to populate); fine if you download lazily per-file but then it's just (A) for browse + (B) for open. | MS cfapi (1 KB placeholders) — learn.microsoft.com/.../build-a-cloud-file-sync-engine |
| 2 | **Full vs partial download cost** | Wins for "browse, rarely open": *"Local storage is only used when you work on files... or for recently and frequently used files."* Supports progressive/partial hydration policies (`PARTIAL`/`PROGRESSIVE`/`FULL`). | Pays full-file cost up front (mirror) or on open (temp). No partial-read benefit unless you also implement range reads. | Google (support.google.com/drive/answer/13401938); MS hydration policies (same MS doc) |
| 3 | **Offline availability** | Default = online-only. Only *pinned* files are *"guaranteed to be available offline"*; implicitly-hydrated files *"could be dehydrated... if space is needed."* | Strong: the copy is just a local file. Equivalent to Google Mirror: *"available offline and online."* | MS cfapi; Google (answer/13401938) |
| 4 | **Edit/save correctness & conflicts** | Risky: write-through to remote on save invites partial-write/lock problems (see §5). Conflict handling is the *provider's* job — MS notes the engine *"must handle merges according to their own specifications"* (the Cloud Mirror sample does **not** implement it). | Cleaner: edit a normal local file with atomic save (write-temp + rename), then upload as one PUT/STOR. Conflict detection = compare remote mtime/etag before upload. | MS cfapi (merge is provider's responsibility); POSIX atomic `rename` — en.wikipedia.org/wiki/Rename_(computing) |
| 5 | **App compatibility (real file?)** | Transparent **only** with deep OS integration (cfapi/File Provider). Even then, reparse-point quirks and "online-only" automation failures occur. A naive VFS is the weakest here. | Maximum: an ordinary file on local disk; every app, CLI tool, and script works with no hooks. | MS cfapi ("non-issue" *with* integration; reparse-point caveat); buralog (Stream automation failures) |
| 6 | **Implementation complexity & OS lock-in** | High and OS-specific: Windows cfapi (`cldflt.sys`, **NTFS-only**), macOS File Provider extension (`~/Library/CloudStorage`), Linux gvfs/FUSE. Three separate platform integrations. | Low and portable: download to temp, edit, upload. No kernel drivers, no per-OS extension, works the same on Win/mac/Linux. | MS cfapi (*"Cldflt.sys currently only supports NTFS volumes"*); Apple File Provider (TidBITS) |
| 7 | **What happens when the helper isn't running** | Files vanish / become inaccessible: placeholder is *"only available if the sync service is available"*; Mirror's advantage is working *"when the Drive for desktop app isn't running."* | Copy is a plain file: available regardless of whether Smart Explorer is running. | MS cfapi; Google (answer/13401938) |

---

## 5. Why editing **directly** over network mounts (SMB/WebDAV/FUSE) is flaky

This is the strongest technical argument for approach (B) (download → edit local → upload) over a
write-through VFS, and it is well documented:

**SQLite — canonical authority on network-FS locking** ("How To Corrupt An SQLite Database File", §2.1):
> *"SQLite depends on the underlying filesystem to do locking as the documentation says it will. But
> some filesystems contain bugs in their locking logic such that the locks do not always behave as
> advertised. This is especially true of network filesystems and NFS in particular."*
>
> *"If SQLite is used on a filesystem where the locking primitives contain bugs, and if two or more
> threads or processes try to access the same database at the same time, then database corruption
> might result."*
> — https://sqlite.org/howtocorrupt.html

**WebDAV editors clobber data on the save/rename step.** Office and many editors save by writing a new
temp file then *moving* it over the target; over WebDAV this loses history and can overwrite a
concurrent edit with no warning:
> *"The changes that User2 made overwrites the changes that User1 made... without any notice/message/
> warning... Neither are even aware that changes were overwritten"* — sabre/dav issue #1294,
> https://github.com/sabre-io/dav/issues/1294

**WebDAV temp-file / lock churn breaks PUTs** and leaves stale `webdavLock`s mid-edit:
- davfs2 discussion: tools like vim/sed *"create a lock on the file... but never actually completes the changes."* — https://sourceforge.net/p/dav/discussion/82589/thread/92170b7c/
- Mountain Duck file-locking docs note *"Failed PUT-requests are almost ever caused by temporary file locks."* — https://docs.mountainduck.io/mountainduck/locking/

**SMB/CIFS caching + weak locking → corruption** for in-place edits of stateful files:
- H2 database, issue #1935: file-lock logic *"fails on SMB drives where there is caching of metadata...
  Windows caches metadata... for about 5-10 seconds"* so another process *"does not understand it is
  modified until after 5-10 seconds."* — https://github.com/h2database/h2database/issues/1935
- Microsoft KB: *"Data corruption when multiple users perform read and write operations to a shared
  file in the SMB2 environment."* — https://support.microsoft.com/en-us/topic/data-corruption-when-multiple-users-perform-read-and-write-operations-to-a-shared-file-in-the-smb2-environment-4bb67519-71b1-4588-c380-e4ceaa695418

**Recommended pattern** (the (B) approach): copy locally, edit, write back atomically. POSIX `rename`
*"is guaranteed to have been atomic... another program would only see the file with the old name or the
file with the new name, not both or neither... used during a file save operation to avoid any possibility
of the file contents being lost if the save operation is interrupted."*
— https://en.wikipedia.org/wiki/Rename_(computing)

---

## 6. When each approach wins (concrete)

### On-demand placeholder / streaming VFS (A) wins when:
- **Huge remote trees you mostly browse, rarely open.** Metadata-only listing + 1 KB placeholders;
  *"Local storage is only used when you work on files... or for recently and frequently used files."*
  (Google; MS cfapi). This is exactly Google's pitch for Stream.
- **Limited local disk.** Don't materialize the whole tree. *"If you don't have much space on the
  local hard drive, it is recommended to pick streaming files"* (CBackup).
- **Browse-not-edit / read-mostly workflows**, where being online is acceptable and you never need the
  file when offline.
- **You are willing to pay for deep OS integration** (cfapi on NTFS-only Windows, File Provider on
  macOS, gvfs/FUSE on Linux) to get true app-transparency — otherwise app-compat suffers (§3).

### Download-a-real-local-copy / mirror or temp (B) wins when:
- **You want to edit one (or a few) files and save back.** Edit a normal local file, atomic
  write-temp+rename, single upload; conflict = compare remote etag/mtime first. Avoids every network-FS
  locking/partial-write hazard in §5.
- **App compatibility matters.** The copy is an ordinary file every tool/app understands with zero OS
  hooks — no reparse-point quirks, no "online-only" automation failures (§3).
- **Offline access is required.** A downloaded copy behaves like Google Mirror: *"available offline and
  online"* and works *"when the Drive for desktop app isn't running."* A placeholder is only guaranteed
  offline if explicitly **pinned/hydrated** — which is just (B) under the hood.
- **Simplicity / portability / avoiding OS lock-in.** No kernel minifilter (cfapi is *"NTFS only"*),
  no per-OS extension. One code path on Windows/macOS/Linux.
- **The helper app isn't always running.** Plain files survive Smart Explorer being closed; placeholders
  are *"only available if the sync service is available."*

### Practical synthesis for Smart Explorer
Mirror Google's own design: **browse via on-demand metadata + placeholders (A) for the tree, but the
moment the user opens a file to edit, switch to download-real-copy (B)** — fetch to a temp/cache file,
let the external editor work on a true local file, then upload atomically on save with an etag/mtime
conflict check. This captures (A)'s cheap browse / low disk and (B)'s edit correctness, app-compat,
offline robustness, and simplicity — and crucially sidesteps the network-FS editing hazards in §5.
A pure write-through VFS only pays off if you also commit to per-OS placeholder-API integration.

---

## 7. Source list (URLs)

1. Google Drive Help — "Stream & mirror files with Drive for desktop": https://support.google.com/drive/answer/13401938?hl=en
2. Microsoft Learn — "Build a Cloud Sync Engine that Supports Placeholder Files" (cfapi): https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine
3. buralog — "Streaming vs Mirroring: Practical Decision Criteria": https://buralog.jp/en/streaming-vs-mirroring-en/
4. buralog — "Google Drive for Desktop: Streaming vs Mirroring — Operational Differences for Business Use": https://buralog.jp/en/drive-streaming-vs-mirroring-en/
5. CBackup — "Stream Files vs. Mirror Files": https://www.cbackup.com/articles/stream-files-vs-mirror-files.html
6. SQLite — "How To Corrupt An SQLite Database File": https://sqlite.org/howtocorrupt.html
7. sabre/dav issue #1294 — direct WebDAV editing overwrites: https://github.com/sabre-io/dav/issues/1294
8. H2 database issue #1935 — SMB metadata caching / locking corruption: https://github.com/h2database/h2database/issues/1935
9. Microsoft Support — SMB2 multi-user read/write data corruption: https://support.microsoft.com/en-us/topic/data-corruption-when-multiple-users-perform-read-and-write-operations-to-a-shared-file-in-the-smb2-environment-4bb67519-71b1-4588-c380-e4ceaa695418
10. Mountain Duck — File Locking docs (failed PUTs = temp locks): https://docs.mountainduck.io/mountainduck/locking/
11. davfs2 discussion — incomplete writes / lingering locks: https://sourceforge.net/p/dav/discussion/82589/thread/92170b7c/
12. Wikipedia — "Rename (computing)" (POSIX atomic rename for safe saves): https://en.wikipedia.org/wiki/Rename_(computing)
13. TidBITS — "Apple's File Provider Forces Mac Cloud Storage Changes": https://tidbits.com/2023/03/10/apples-file-provider-forces-mac-cloud-storage-changes/
14. Apple Developer Forums — "Error when materializing files" (cannotMaterialize): https://developer.apple.com/forums/thread/802063
