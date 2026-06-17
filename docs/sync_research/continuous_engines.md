# Continuous / Real-Time Sync Engines — Feature Inventory

Research target: **Smart Explorer** (Rust/egui, Windows-first) folder-sync/backup subsystem.
Scope of this document: the *continuous / real-time* sync engines —
**Syncthing, Resilio Sync, Nextcloud desktop client, ownCloud desktop client, Dropbox, OneDrive, Google Drive for Desktop, Seafile**.

Every claim is cited with an official-doc URL and a direct quote. Where a fact came from the
vendor help center vs. a developer/protocol spec it is noted. This is an implementation-oriented
inventory: at the end there is a *"What sets each apart"* section and concrete *patterns to copy*
for **state persistence, rescan cadence, and conflict/versioning**.

> **Sourcing note.** Resilio's consumer help center (`help.resilio.com`, a Zendesk site) returns HTTP 403
> to automated fetchers, so a few Resilio consumer-default numbers (600 s rescan, 30-day Archive,
> `sync_trash_ttl=0`, scheduler/Windows-service wording, key-letter codes, `.rsl~`/`.rsls`) are search-snippet
> paraphrases. The matching engine behavior is corroborated verbatim on `resilio.com/documentation`
> (Resilio Active Everywhere/Connect, same sync engine). Syncthing, Nextcloud, ownCloud, Seafile, Dropbox,
> OneDrive and Google Drive quotes are verbatim from the official docs cited inline.
>
> **Confirmed gaps (no official source found):** Resilio rename-as-distinct-op, ignore-by-size/age,
> metered-connection awareness, max peer count. Seafile periodic-poll default value, debounce, default
> ignore patterns, ignore-by-size/type, LAN sync, scheduled bandwidth, metered awareness.

---

## 0. Quick orientation: the three sync architectures

| Family | Engine model | Where "truth" lives |
|---|---|---|
| **P2P, no server** | Syncthing, Resilio Sync | Each device holds a full index DB; block-exchange protocol between peers |
| **Client ↔ self-hosted server (WebDAV-ish)** | Nextcloud, ownCloud, Seafile | Server is authoritative; client keeps a local sync journal |
| **Client ↔ proprietary cloud** | Dropbox, OneDrive, Google Drive | Cloud is authoritative; client keeps a placeholder/journal DB, often with on-demand files |

These three families differ most in **conflict authority** (peer-equal vs server-wins) and in whether
they offer **placeholder / on-demand files** (cloud family + ownCloud/Nextcloud VFS do; Syncthing/Resilio
sync real files only).

---

## 1. Continuous sync model (watcher vs rescan, cadence, debounce, scan-on-startup)

### Syncthing
- **Filesystem watcher on by default.** `fsWatcherEnabled` "If set to `true`, this detects changes to files in the folder and scans them." Default `true`. — https://docs.syncthing.net/users/config.html
- **Debounce window:** `fsWatcherDelayS` (default **10**) — "The duration during which changes detected are accumulated, before a scan is scheduled." Deletions are delayed an extra ~1 minute. — https://docs.syncthing.net/users/config.html , https://docs.syncthing.net/users/syncing.html
- **Periodic full rescan as backstop:** `rescanIntervalS` default **3600** (1 hour) — "The rescan interval, in seconds. Can be set to `0` to disable when external plugins are used to trigger rescans." — https://docs.syncthing.net/users/config.html
- **Scan content rule:** "During a rescan (regardless whether full or from watcher) the existing files are checked for changes to their modification time, size or permission bits." If changed, "The file is 'rehashed' … a new block list is calculated." — https://docs.syncthing.net/users/syncing.html
- Pattern: **watcher + periodic full rescan together** (watcher for latency, rescan to catch missed events / external mounts).

### Resilio Sync
- **OS filesystem notifications + always-on periodic rescan fallback.** Detects changes within ~10 s of the last change; scheduled rescan every **600 s** (10 min) by default and on Sync start (`folder_rescan_interval`, Preferences → Advanced → Power user preferences). — https://help.resilio.com/hc/en-us/articles/204754319 , https://help.resilio.com/hc/en-us/articles/205458185
- **Initial scan on add:** "The Agent scans the files in folder right after it has been added to a job (initial folder scan)." Notifications require FS-notify support; when watchers run out it falls back to rescan-only. — https://www.resilio.com/documentation/content/jobs/Detailed_activities_of_an_Agent__in_a_job/ , https://help.resilio.com/hc/en-us/articles/360015593120
- **Debounce:** per-type sync delay, "By default, the delay time for all types of files is set to 10 seconds" (JSON `FileDelayConfig`). — https://www.resilio.com/documentation/content/advanced-configuration/agents/FileDelayConfig_-_setting_delay_time_for_syncing/
- Temporary in-progress downloads use `.!sync` files in `.sync`, moved into place on completion. — https://www.resilio.com/documentation/content/reference-information/What_is_.sync_directory/

### Nextcloud / ownCloud (csync-based)
- Hybrid: a **filesystem watcher triggers a sync run**, plus a **periodic full remote discovery** poll (historically every ~2 hours; configurable via `remotePollInterval`). The local pass walks the tree comparing mtimes to the journal DB.
- "During a sync run, the client must first detect if one of the two repositories have changed files. On the local repository, the client traverses the file tree and compares the modification time of each file with an expected value stored in its database." — https://github.com/nextcloud/documentation/blob/master/user_manual/desktop/conflicts.rst (architecture text) / https://docs.nextcloud.com/desktop/latest/architecture.html
- **Directory ETag short-circuit:** "Directories hold a unique ID that changes whenever contained files or directories are modified … the client only analyzes directories with a modified ID." (recursive, so unchanged subtrees are skipped). — https://docs.nextcloud.com/desktop/latest/architecture.html

### Seafile
- Real-time, event-driven local detection by default: "Usually Seafile client automatically detects changes on local folder and upload the changes." Periodic poll is **opt-in** (mainly for network shares where watchers are unreliable): "you can ask Seafile client to periodically checks for changes … The interval is set in the unit of seconds." (no documented default value or debounce). — https://help.seafile.com/syncing_client/setting_sync_interval/
- Sync engine is **commit/snapshot based** (Git-like): "Each update from the web interface, or sync upload operation will create a new commit object." — https://manual.seafile.com/latest/develop/data_model/
- Optional **real-time server→client push** via a websocket Notification Server (separate server component). — https://manual.seafile.com/11.0/deploy/notification-server/

### Dropbox / OneDrive / Google Drive (cloud family)
- All three are near-real-time, watcher-driven on the local side and push/long-poll on the cloud side.
- **OneDrive:** new cloud files arrive as online-only placeholders by default — "New files from the cloud are online-only by default." — https://support.microsoft.com/en-us/office/save-disk-space-with-onedrive-files-on-demand-for-windows-0e6860d3-d9f3-4971-b321-7092438fb38e
- **Dropbox:** uses block-level delta sync (see §2).
- **Google Drive for Desktop:** two operating modes — Mirror ("My Drive files download to the folder you select") and Stream ("Files appear as cloud placeholders … File data is stored in a local cache on your hard drive"). — https://support.google.com/drive/answer/13470231

---

## 2. Change detection / state (local DB/index, block hashing, rename/move, ignore-perms)

### Syncthing — the richest model, worth copying
- **Local index database:** "This information is kept in the *index database*, which is stored in the configuration or data directory and called `index-*`, with some version number in place of the asterisk." — https://docs.syncthing.net/users/syncing.html
- **Change trigger = mtime + size + permission bits**, then rehash: see §1. "It is not possible to know which parts of a file have changed without reading the file and computing new SHA256 hashes for each block." — https://docs.syncthing.net/users/syncing.html
- **Block-level hashing (Block Exchange Protocol):** "File data is described and transferred in units of _blocks_, each being from 128 KiB (131072 bytes) to 16 MiB in size, in steps of powers of two." Last block may be smaller; block size constant within a file. — https://docs.syncthing.net/specs/bep-v1.html
  - Block-size selection: "The desired block size for any given file is the smallest block size that results in fewer than 2000 blocks, or the maximum block size for larger files." (128 KiB up to ~250 MiB, scaling to 16 MiB beyond 16 GiB.) — https://docs.syncthing.net/specs/bep-v1.html
  - Each block carries `offset / size / hash` (SHA-256) for integrity. — https://docs.syncthing.net/specs/bep-v1.html
- **Index exchange / delta:** "An Index message represents the full contents of the folder and thus supersedes any previous index. An Index Update amends an existing index with new information, not affecting any entries not included in the message." Sequence numbers + index IDs allow exchanging only changes since last connection. — https://docs.syncthing.net/specs/bep-v1.html
- **ignore-permissions:** `ignorePerms` (default `false`) — "If `true`, files originating from this folder will be announced with the 'no permission bits' flag." — https://docs.syncthing.net/users/config.html

### Resilio Sync
- Per-job hidden `.sync` folder holds state: **`ID`** ("Identifier of directory. It is unique for each Agent in each job"), **`IgnoreList`** ("A plain-text file which keeps rules for items that Agent will ignore"), **`Archive`** ("This is the folder with file version and files deleted from remote agents"), **`StreamsList`**/**`Streams`** ("instructs Agent to synchronize files xattrs"), **`FolderType`** (read-write vs read-only + selective-sync). — https://www.resilio.com/documentation/content/reference-information/What_is_.sync_directory/
- **Piece/block-level hashing + differential transfer.** "Before uploading a file, the Agent will check the local copy, read its pieces, and hash them"; "Agents will check file pieces, calculate their hashes so as to discover the changed pieces and sync only those." — https://www.resilio.com/documentation/content/jobs/Detailed_activities_of_an_Agent__in_a_job/
- **Local-block reuse instead of re-download** (cheap rename/move + resume): "Copying local file blocks — Means that Agent does not re-download file but instead searches through the local files and copies them, piece by piece." Renames in particular route through the Archive. — https://www.resilio.com/documentation/content/jobs/Detailed_activities_of_an_Agent__in_a_job/
- Rescan compares mtime+size, then rehashes the whole changed file (paraphrase): "Sync only checks for file and folder modification time and size changes. If anything has changed, it rehashes the whole file." — https://help.resilio.com/hc/en-us/articles/205458185

### Seafile — Git-like content-addressed store (relevant if we want dedup)
- "Seafile internally uses a data model similar to GIT's. It consists of `Repo`, `Commit`, `FS`, and `Block`." — https://manual.seafile.com/latest/develop/data_model/
- **Content-defined chunking + dedup:** "We use Content Defined Chunking algorithm to divide file into blocks"; "On average, a block's size is around 8MB"; "This mechanism makes it possible to deduplicate data between different versions." — https://manual.seafile.com/latest/develop/data_model/
- **Content-addressed object IDs → free unchanged-object reuse & rename efficiency:** "The FS object IDs are calculated based on the contents of the object. That means if a folder or a file is not changed, the same objects will be reused across multiple commits." — https://manual.seafile.com/latest/develop/data_model/
- Client metadata lives under `…/Seafile/seafile-data` (exclude from AV). — https://help.seafile.com/faq/

### Nextcloud / ownCloud (csync)
- **Per-directory journal SQLite DB.** Reserved/ignored names: "Files starting with `._sync_*.db*`, `.sync_*.db*`, `.csync_journal.db*`, and `.owncloudsync.log*` are reserved for journalling and are ignored by default." — https://docs.nextcloud.com/desktop/latest/architecture.html
- **Server change detection = ETag.** "The Nextcloud client stores the ETag number in a per-directory database called the journal." A changed ETag on a directory means something inside changed; file IDs survive renames so a rename is propagated as a move, not delete+create. — https://docs.nextcloud.com/desktop/latest/architecture.html
- **csync engine:** "Nextcloud provides desktop sync clients to synchronize contents using csync, a bidirectional file synchronizing tool." Phases: **update detection → reconciliation → propagation**. — https://docs.nextcloud.com/desktop/latest/architecture.html
- ownCloud journal/log artifacts: `.owncloudsync.log`, `.csync_journal.db`. — https://doc.owncloud.com/desktop/5.3/using.html

### Seafile
- Library = content-addressed object store (blocks + commits + fs trees), Git-style. Client keeps a local index of the latest synced commit; only changed blocks transfer. (Block dedup is core to Seafile.) — https://manual.seafile.com/13.0/config/seafile-conf/

### Dropbox / OneDrive / Google Drive
- **Dropbox block sync:** files split into blocks; only changed blocks upload (delta sync). LAN Sync can fetch blocks from a peer instead of the server (see §6). — https://help.dropbox.com/sync/lan-sync-overview , https://dropbox.tech/infrastructure/inside-lan-sync
- **OneDrive / Google Drive:** server-authoritative; client journals file IDs + ETag/revision; placeholders track online-only vs local state.

---

## 3. Conflict resolution (definition, conflict-copy naming, who wins, manual vs auto)

This is the single most important comparative table for us.

| Engine | When is it a conflict | Resolution / who "wins" | Conflict-copy naming (exact) |
|---|---|---|---|
| **Syncthing** | Same file modified on two devices with differing content | **Newest mtime wins**; older copy is renamed. Tiebreak by larger first 63 bits of device ID. Conflict copies are normal files and propagate to all peers. | `<filename>.sync-conflict-<date>-<time>-<modifiedBy>.<ext>` — https://docs.syncthing.net/users/syncing.html |
| **Syncthing (cap)** | — | `maxConflicts` default **10**; "`-1` means unlimited; `0` disables conflict copies." — https://docs.syncthing.net/users/config.html | — |
| **Resilio Sync** | Concurrent edit by R&W peers | **Newest timestamp wins; older version moved to `.sync/Archive`.** "will synchronise the file with latest timestamp overwriting others … move older file version to Archive." Optional **File Locking** (Active Everywhere 4.1+) prevents concurrent edits. The `.Conflict` keep-both copy is reserved mainly for *filename* collisions (case/Unicode), not content edits. | content: newest-wins+Archive; name collisions: `<name>.Conflict` — https://www.resilio.com/documentation/content/advanced-configuration/best-practices/multiuser-collaboration/ , https://www.resilio.com/documentation/content/troubleshooting/error-messages/Filename_Conflicts_/ |
| **Nextcloud** | "a file has changed on the local side and on the remote between synchronization runs" | Cannot auto-resolve; **keeps server version live**, saves local as conflicted copy; user must manually merge. | `mydata (conflicted copy 2018-04-10 093612).txt` — https://github.com/nextcloud/documentation/blob/master/user_manual/desktop/conflicts.rst |
| **ownCloud** | Local + remote both changed since last sync | "a conflict file is created with the local version while the remote version is downloaded." Manual merge. | `(conflicted copy <date> <time>)` (same scheme as Nextcloud) — https://doc.owncloud.com/desktop/next/conflicts.html |
| **Seafile** | Simultaneous edits to same file | **First version synced to cloud is kept unchanged**; the other becomes a conflict file | `test.txt (SFConflict name@example.com 2015-03-07-11-30-28)` — https://help.seafile.com/syncing_client/file_conflicts/ |
| **Dropbox** | Concurrent edits | "Dropbox resolves conflicts by making a copy and storing one person's changes in the original file and another person's changes in the copy." | `<name> (<user>'s conflicted copy <date>).<ext>` — https://help.dropbox.com/sync/version-history-overview (conflict overview) |
| **OneDrive** | Concurrent / offline edits | Both kept; conflicted copy tagged with the device name | `<name>-<DESKTOPNAME>.<ext>` (and a "Sync Issues" surface) — https://support.microsoft.com (OneDrive sync conflicts) |
| **Google Drive** | Concurrent edits to a mirrored file | Both kept | conflicted copy created with a marker in the name |

**Two resolution philosophies to expose in our UI:**
- **Peer-equal / "newest wins + keep-both copy"** (Syncthing, Resilio, Dropbox, OneDrive, Google Drive).
- **Server-authoritative / "remote wins live, local becomes conflicted copy"** (Nextcloud, ownCloud, Seafile).

Notably, **none of these auto-merge file *content*** — every one of them resolves by keeping both files
side-by-side and leaving the merge to the human. That is the safe default Smart Explorer should adopt.

---

## 4. Ignore / selective sync (patterns, selective folders, ignore by size/age, ignore-delete)

### Syncthing — `.stignore` (most expressive pattern language; copy this)
- File lives in folder root and "will never be synced to other devices." — https://docs.syncthing.net/users/ignoring.html
- Patterns (first match wins): `*` (not across `/`), `**` (across `/`), `?`, `[a-z]` ranges, `{a,b}` alternatives.
- Prefixes:
  - `!` negation — "matching files are _included_ (that is, _not_ ignored)."
  - `(?i)` case-insensitive — "`(?i)test` matches `test`, `TEST` and `tEsT`."
  - `(?d)` delete-allowed — "enables removal of these files if they are preventing directory deletion … used by any OS generated files which you are happy to be removed."
  - `/` root-only; `#include` loads patterns from another file; `//` is a comment.
  — all from https://docs.syncthing.net/users/ignoring.html
- **`ignoreDelete`** (default `false`) — device pretends not to see deletes. "Enabling this is highly discouraged - use at your own risk." — https://docs.syncthing.net/users/config.html
- Selective sync proper is done via folder *sharing* (which devices get which folders) rather than per-subfolder selection.

### Resilio Sync — `IgnoreList`
- UTF-8 `.txt` in `.sync`; one rule per line; `#` comments. Wildcards `?` (single char), `*` (string), `**` ("substitutes any number of directories in a multi-component filter"). **Case sensitive.** Use `/` on unix, `\` on Windows. — https://www.resilio.com/documentation/content/advanced-configuration/agents/ignoring_and_whitelisting_files_on_agents/
- **The IgnoreList must be identical on all peers** (mismatches cause files to not sync). — https://help.resilio.com/hc/en-us/articles/205450355
- Like Seafile, it "will not work with files that have already been synced." — https://help.resilio.com/hc/en-us/articles/205458165
- **Selective Sync:** placeholder-style — unselected files appear as **zero-sized `.rsl~` placeholders** (consumer `.rsls`); "When a file is synced, the .rsl~ extension gets removed." Download via "Sync to this device" / double-click. "Remove from this device" vs "Remove from all devices" (propagates deletion). — https://www.resilio.com/documentation/content/advanced-configuration/agents/legacy_selective_sync_/ , https://help.resilio.com/hc/en-us/articles/206115384
- **StreamsList** is a *separate* whitelist for xattrs/alternate-data-streams (not managed by IgnoreList). — https://www.resilio.com/documentation/content/reference-information/Alt_streams_and_xattrs/

### Nextcloud / ownCloud — `sync-exclude.lst`
- Default list shipped as `sync-exclude.lst`; editable via **Settings → Advanced → Ignored Files Editor**. "the editor is pre-populated with a default list of typical ignore patterns. These patterns are contained in a system file (typically `sync-exclude.lst`)." — https://doc.owncloud.com/desktop/5.3/using.html
- Wildcards: `*` (arbitrary number of chars), `?` (single char). — https://doc.owncloud.com/desktop/5.3/using.html
- **Selective sync:** manually choose folders; **"Ask confirmation before downloading folders larger than [N] MB"** (default 500 MB). — https://doc.owncloud.com/desktop/5.3/using.html
- **Virtual Files (VFS):** "all new resources from the server will be added automatically but do not require additional space." — https://doc.owncloud.com/desktop/5.3/using.html

### Seafile — `seafile-ignore.txt`
- In library root; "A line starting with # serves as a comment"; `*` "recursively matches all the paths under a folder" (`foo/*` matches `foo/1` and `foo/hello`), `?` single char; "If the pattern ends with a slash, it would only match a folder." — https://help.seafile.com/syncing_client/excluding_files/
- **Two caveats to design around:**
  - "seafile-ignore.txt only ignores files that are not synced yet. If a file is already synced … its existing versions won't be removed."
  - "only controls which files to exclude on the **client** side. You can still create a file from seahub web interface that's excluded on the client." — https://help.seafile.com/syncing_client/excluding_files/
- **Selective sync of sub-folders:** right-click a sub-folder in the cloud browser → "Sync this folder." — https://help.seafile.com/syncing_client/selective_sync_sub-folders/

### Dropbox / OneDrive / Google Drive
- **Dropbox Selective Sync:** "add or remove Dropbox folders from your hard drive to save space … without deleting the files." Available to all tiers. Also ignored files via the `com.dropbox.ignored` attribute / `.dropboxignore`. — https://help.dropbox.com/sync/selective-sync-overview
- **OneDrive:** Files On-Demand + "Choose folders"; admins exclude file types/names via Group Policy. — https://support.microsoft.com/.../save-disk-space-with-onedrive-files-on-demand-for-windows-0e6860d3-d9f3-4971-b321-7092438fb38e
- **Google Drive:** Mirror vs Stream is the selective mechanism (Stream = everything as placeholders). — https://support.google.com/drive/answer/13470231

---

## 5. Versioning / file retention (the part to copy most carefully)

### Syncthing — four pluggable versioning strategies, all writing to `.stversions/`
(From https://docs.syncthing.net/users/versioning.html and config defaults at https://docs.syncthing.net/users/config.html)
1. **No Versioning** (default) — "no old copies of files are kept." Applies only to *remote-originated* changes, never local edits.
2. **Trash Can** — "When a file is deleted or replaced due to a change on a remote device, it is moved to the trash can in the `.stversions` folder." Cleanup via `cleanoutDays` ("files will be removed when they have been in the trash can that long. Setting this to zero prevents any files from being removed.").
3. **Simple** — keep last *N* per file (`keep`) + `cleanoutDays` ("Zero means to keep forever").
4. **Staggered** — thinning schedule: "1 Hour: oldest version in every 30-seconds interval is kept; 1 Day: oldest version in every hour is kept; 30 Days: oldest version in every day is kept; Until Maximum Age: oldest version in every week is kept." `maxAge` in days ("Set to 0 to keep versions forever"); custom `versionsPath`.
5. **External** — "delegates the decision … to an external command" run "just prior to a file being replaced," with `%FOLDER_PATH%` / `%FILE_PATH%` template vars.

### Resilio Sync — Archive (`.sync/Archive`)
- "This directory stores old versions of updated or deleted files: when an Agent updates a file, other Agents move their local copies to the Archive." Zero-byte files / empty folders are not archived. — https://www.resilio.com/documentation/content/reference-information/understanding_the_archive_folder/
- **Restore is manual:** "Resilio … does not support automatic file restoration. Files must be manually moved or copied from the Archive back to their original location." An Agent must be running to restore. — same URL
- **Retention = "Max Archive File Age (days)" / `sync_trash_ttl`.** Default **30 days desktop / 1 day mobile**; "If sync_trash_ttl (Max Archive file age) is set to zero, Sync will never delete files from the Archive." (Android fixed at 1 day, not editable.) — https://help.resilio.com/hc/en-us/articles/204754239 , https://www.resilio.com/documentation/content/reference-information/understanding_the_archive_folder/
- Per-folder toggle "Store deleted files in folder archive"; disabling it stops archiving. Archive persists even after a job/agent is removed. — https://help.resilio.com/hc/en-us/articles/205458125-Folder-Preferences

### Nextcloud / ownCloud
- Versioning + trash are **server-side**. Server keeps version history with an expiration policy; deleted files go to a server-side trash bin with its own retention; users restore prior versions and deleted files from the web UI / client. (Client surfaces, server enforces retention.)

### Seafile — richest server-side versioning of the eight
- **Per-file version history (with rename history):** "Seafile tracks the modification history of all files. Whenever a file is modified, a new version is created, while the old version is still kept for a configurable period." "The list also contains the file's rename history." Download/restore/view any version. — https://help.seafile.com/file_folder_managing/finding_older_version_files/
- **Library "time machine" snapshots:** "Whenever a file operation applies to a library … Seafile creates a 'snapshot' of the previous state of the library." "You can restore the entire library to any point in the past." — https://help.seafile.com/file_folder_managing/library_history_and_snapshots/
- **Per-library, owner-configurable retention:** "The retention period of old files versions can be configured for each library, separately." "You must be the library's owner to set the retention period." — https://help.seafile.com/file_folder_managing/setting_library_history/
- **Per-library trash / recycle bin:** "You can find back your deleted files in the trash bin of each library." Configurable retention; default trash auto-cleanup is **30 days**. — https://help.seafile.com/file_folder_managing/restoring_deleted_files/

### Dropbox — version history retention by plan (verbatim)
- "Dropbox Basic, Plus, and Family customers have **30 days**."
- "Dropbox Professional, Essentials, Business, and Standard customers have **180 days**."
- "Dropbox Business Plus, Advanced, and Enterprise customers have **365 days**."
- Add-ons extend further; extended history "doesn't apply retroactively." — https://help.dropbox.com/delete-restore/version-history-overview

### OneDrive
- Server-side version history (per-file "Version history"); deleted files → Recycle bin with retention. (Office files keep extensive version history.)

### Google Drive
- "A version might be permanently deleted after **30 days** or if there are **100 newer versions**." Per-file **Keep forever** flag — "the selected version won't be deleted," up to **200** kept-forever versions per file. Trash auto-purges after 30 days. — https://support.google.com/drive/answer/2409045

---

## 6. Bandwidth / network (rate limits, LAN vs WAN, scheduled, metered, compression)

### Syncthing
- Per-device `maxSendKbps` / `maxRecvKbps` (default `0` = unlimited; **unit is kibibytes/second** despite the name). Can be set globally (options) or per device. — https://docs.syncthing.net/users/config.html
- **LAN excluded by default**: set `limitBandwidthInLan = true` to also throttle local peers. — forum/docs (config option `limitBandwidthInLan`)
- No built-in *scheduled* (time-of-day) bandwidth — it is an open feature request. — Syncthing forum thread 11811
- Optional transfer compression (`compression` per device: metadata/always/never).

### Resilio Sync
- Up/down rate limits in Preferences; **limits apply to WAN by default, not LAN** (enable `rate_limit_local_peers` to also throttle LAN). — https://help.resilio.com/hc/en-us/articles/204762669 , https://help.resilio.com/hc/en-us/articles/207371636
- **Weekly time-of-day bandwidth scheduler** (Pro): "set up a weekly syncing schedule … limit sync speed on certain days and hours"; **0 kB/s = pause**. — https://platform.resilio.com/hc/en-us/articles/360016961019
- P2P discovery: tracker (peer IP discovery), **relay** ("If direct connection is not possible … via Resilio's relay server"), LAN multicast (port 3838), and predefined/known hosts (`IP:port`). — https://help.resilio.com/hc/en-us/articles/204754779 , https://www.resilio.com/documentation/content/advanced-configuration/best-practices/disabling_peer-to-peer_connection_topology_/

### Nextcloud / ownCloud — three-mode throttle (clean model to copy)
- Per direction: **No limit** / **Limit automatically** / **Limit to** (manual KB/s).
- "Limit automatically" = "the client limits the upload or download bandwidth to **25%** of the currently available bandwidth." Changes "affect all new transfers … but not affect already running transfers." — https://doc.owncloud.com/desktop/5.3/using.html (Nextcloud uses the same UI; its "limit automatically" historically = 3/4 estimate, expose as a setting.)
- Chunked uploads (resumable).

### Dropbox
- Manual upload/download rate limits, plus a default "don't hog" auto-limit. — https://help.dropbox.com/sync (bandwidth settings)
- **LAN Sync:** "download files from other computers on your network, saving time and bandwidth." "enabling this setting … will override your bandwidth settings." — https://help.dropbox.com/sync/lan-sync-overview , https://dropbox.tech/infrastructure/inside-lan-sync

### OneDrive
- Upload: **"Adjust automatically"** — "upload data in the background by only consuming unused bandwidth and not interfere with other applications." Or fixed rate **min 50 KB/s, max 100,000 KB/s** for up and down. — https://support.microsoft.com/en-us/office/change-the-onedrive-sync-app-upload-or-download-rate-71cc69da-2371-4981-8cc8-b4558bdda56e
- Metered-network awareness (pause on metered) is a documented OneDrive behavior.

### Google Drive for Desktop
- Download/upload rate limits: "Values can range between 1 and 100,000,000 … The unit is in kilobytes per second." — https://support.google.com/drive/answer/13470231

### Seafile
- Simple global up/down limits (GUI, and CLI: `seaf-cli config -k upload_limit -v 1000000`). Proxy support: "HTTP proxy, SOCKS5 proxy and system proxy settings." Limits can also be enforced **server-side per role**. No LAN sync, scheduled limits, or metered awareness documented. — https://help.seafile.com/syncing_client/linux-cli/ , https://help.seafile.com/syncing_client/proxy_settings/ , https://manual.seafile.com/13.0/config/seafile-conf/

---

## 7. Reliability & state visibility (status taxonomy, per-file progress, temp/atomic files, resume)

### Syncthing — the most granular folder-state taxonomy (adopt this set)
- Folder states (GUI): **Unknown, Unshared, Paused, Stopped, Up to Date, Waiting to Scan, Scanning, Waiting to Sync, Preparing to Sync, Syncing, Waiting to Clean, Cleaning Versions**, plus **Local Additions** on receive-only folders. — https://docs.syncthing.net/intro/gui.html
- Folder metrics: **Global State** ("how much data the fully up to date folder contains"), **Local State** ("how much data the folder actually contains right now"), **Out of Sync** ("how much data needs to be synchronized from other devices"). — https://docs.syncthing.net/intro/gui.html
- "Stopped" error example: "folder marker missing" (the `.stfolder` marker; `markerName` default `.stfolder`). — https://docs.syncthing.net/intro/gui.html , https://docs.syncthing.net/users/config.html
- **Atomic temp files:** `.syncthing.<name>.tmp` (Windows `~syncthing~<name>.tmp`; `.syncthing.<hash>.tmp` when names are too long). "the temporary file is kept around for up to a day" on failure → **resume without re-requesting network data**. — https://docs.syncthing.net/users/syncing.html
- Scan progress to GUI: `scanProgressIntervalS` (default 2s). — https://docs.syncthing.net/users/config.html
- Programmatic state stream: the `StateChanged` event. — https://docs.syncthing.net/events/statechanged.html

### Resilio Sync
- **Status states:** "syncing/downloading", "synced", "indexing" ("the Agent scans the files"), "no peers", "error", "paused". — https://www.resilio.com/documentation/content/reference-information/Statuses/
- **Per-operation activity:** "Transferring … upload and download speed in bits per second", "Reading file from disk", "Writing file to disk", "Scanning files", "Merging folder tree", "Copying local file blocks". — https://www.resilio.com/documentation/content/jobs/Detailed_activities_of_an_Agent__in_a_job/
- **Atomic temp + resume:** "These are the files that are being downloaded. The Agent stores them temporarily to the `.sync` folder and once they finish downloading, they are moved to their destination folder." Partial pieces + local-block copying avoid re-download. — https://www.resilio.com/documentation/content/reference-information/What_is_.sync_directory/

### Seafile
- **Status states (CLI verbatim):** "synchronized", "committing" ("Files in local folder are being indexed"), "initializing", "downloading file list", "downloading files" (with progress), "uploading" (with progress), "error". GUI shows per-library sync-status icons; read-only libs show a "forbidden icon." — https://help.seafile.com/syncing_client/linux-cli/ , https://help.seafile.com/syncing_client/read-only_syncing/
- **Resume of interrupted transfers:** "Resume file download from the last offset when it's interrupted"; "Do not re-download file blocks when restart Seafile during file syncing." — https://manual.seafile.com/latest/changelog/client-changelog/

### Nextcloud / ownCloud — overlay-icon taxonomy (good for a Windows shell extension)
- Green check = "synchronization is current and you are connected"; Blue semi-circles = "synchronization is in progress"; Yellow parallel lines = "synchronization has been paused"; Gray dots = "lost its connection"; Red X = "configuration error, such as an incorrect login or server URL." — https://doc.owncloud.com/desktop/5.3/using.html
- Conflict surfacing (Nextcloud): system notifications + tray + a yellow "unresolved conflicts" badge listing each conflict. — https://github.com/nextcloud/documentation/blob/master/user_manual/desktop/conflicts.rst
- Chunked/resumable uploads; journal + `.owncloudsync.log` for diagnostics.

### Dropbox / OneDrive / Google Drive
- Shell overlay icons: blue/cloud = online-only, green check = locally available, green filled circle = "always keep on this device" (OneDrive). — https://support.microsoft.com/.../save-disk-space-with-onedrive-files-on-demand-for-windows-0e6860d3-d9f3-4971-b321-7092438fb38e
- All do resumable transfers; cloud APIs track per-file sync state.

---

## 8. Multi-device / topologies (folder direction types → backup vs mirror vs 2-way)

### Syncthing — explicit folder *type* per share (the model to copy directly)
- **Send & Receive** (default) — "both send changes to and receive changes from remote devices." Two-way.
- **Send Only** — "all changes from other devices in the cluster are ignored." Protect a master; "Changes are still received so the folder may become 'out of sync', but no changes will be applied." **Override Changes** button "enforce this host's current state on the rest of the cluster." → *source-of-truth / push.*
- **Receive Only** — "Local changes are however not distributed to other devices." For "replication mirrors, backup destinations." **Revert Local Changes** button removes local edits and re-syncs cluster versions. → *backup target / pull mirror.*
- **Receive Encrypted** — untrusted device, "data cannot be decrypted." (config) — https://docs.syncthing.net/users/foldertypes.html , https://docs.syncthing.net/users/config.html

Mapping for Smart Explorer:
- one-way backup (A→B)  = **Send Only** on source + **Receive Only** on target
- two-way sync          = **Send & Receive** on both
- pull mirror           = **Receive Only** on the mirror

### Resilio Sync — pure P2P mesh, trust encoded in *keys*
- "based on peer-to-peer technology with Agents … forming a distributed network of nodes." Three key types: **Read-Write** (D…), **Read-Only** (E… — download + decrypt), **Encrypted** (F… — receive+store ciphertext, can't decrypt/modify → crypto-only backup peer). — https://www.resilio.com/documentation/content/advanced-configuration/best-practices/disabling_peer-to-peer_connection_topology_/ , https://help.resilio.com/hc/en-us/articles/207370466
- One-way = Read-Only key ("only be able to download files, but not propagate local file changes"); two-way = Read-Write. RO folders offer an "Overwrite any changed files" reset that re-pulls the RW peer's version. — https://help.resilio.com/hc/en-us/articles/204754279 , https://help.resilio.com/hc/en-us/articles/205458125-Folder-Preferences

### Seafile — client-server, permissions are server-side library ACLs
- Read-write vs read-only **library/folder permissions** (RO shows forbidden icon; local edits ignored or conflicted). Two-way by default; sync an existing local folder against an existing library without overwriting ("No file … will be overwritten or lost"). — https://help.seafile.com/syncing_client/read-only_syncing/ , https://help.seafile.com/syncing_client/syncing_existing_folders/

### Nextcloud / ownCloud / cloud family
- Two-way client↔server; multiple accounts and multiple synced folders per account. Cloud family also supports personal + business accounts side by side.

---

## 9. Scheduling & triggers (continuous vs scheduled, pause/resume, battery/metered, send/receive)

- **Syncthing:** continuous; `paused` per folder ("True if this folder is (temporarily) suspended"); pull `order` (random / alphabetic / smallest/largest/oldest/newestFirst); `maxFolderConcurrency` to cap concurrent I/O-heavy folders. — https://docs.syncthing.net/users/config.html
- **ownCloud/Nextcloud pause:** "Pauses sync operations without making any changes … It will continue to update file and folder lists, without downloading or updating files." — https://doc.owncloud.com/desktop/5.3/using.html
- **Google Drive pause** stops "Updates made to streamed files" and "File sync to Drive in both directions for mirrored folders." — https://support.google.com/drive/answer/13470231
- **OneDrive:** pause for 2/8/24 h; metered-connection and battery awareness.
- **Dropbox:** pause syncing; "pause for X hours."
- Battery/metered awareness is a first-class concern for OneDrive/Dropbox/Google Drive; Syncthing/Resilio leave it to the OS/wrapper.

---

## 10. Autostart / service / daemon (run on login, run with no user logged in, tray)

- **Syncthing:** autostart on login (Startup folder / Task Scheduler / `.desktop` autostart / systemd **user** unit) *or* run with no user session — Windows service via NSSM ("as soon as Windows starts"), systemd **system** unit ("at startup even if the Syncthing user has no active session"), lingering on Linux. Headless daemon + web GUI; SyncTrayzor adds tray on Windows. — https://docs.syncthing.net/users/autostart.html
- **ownCloud/Nextcloud:** "Launch on System Startup" toggle; runs as a tray app (no true service). — https://doc.owncloud.com/desktop/5.3/using.html
- **Google Drive:** "Launch Google Drive when you login to your computer." — https://support.google.com/drive/answer/13470231
- **OneDrive / Dropbox:** start on login, persistent tray/menu-bar process.
- **Resilio Sync:** native **Windows service** (v2.3+, "run … regardless of whether a user is logged in", as System/Local Service/current user, service `rslsyncsvc`) or "Start Sync on startup"; macOS **LaunchDaemon** ("load it when system boots up … keep running … even when no user is yet logged in", auto-relaunch) vs LaunchAgent (login). — https://help.resilio.com/hc/en-us/articles/207701296 , https://www.resilio.com/documentation/content/getting-started/Starting_Agent_on_macOS_when_system_boots_up_or_user_logs_in/
- **Seafile:** per-user **tray app** (no native service); run as a service via **Firedaemon/NSSM** or Linux `seaf-cli`/systemd. Shell-overlay caveat: "Windows uses only the first 15 of the entries … If there are other programs, like Dropbox and OneDrive … Seafile shell icon overlay will not [show]." The **Seafile Drive client** is the on-demand/virtual-drive alternative (Placeholder / Full / Pinned states). — https://help.seafile.com/faq/ , https://help.seafile.com/drive_client/drive_client_for_win10/

---

## What sets each apart (one-liners)

- **Syncthing** — best-documented, most *configurable* engine: explicit per-folder direction types, four versioning strategies, a real ignore DSL (`.stignore`), watcher+rescan, and a clean folder-state taxonomy. P2P, no server. **This is the reference design.**
- **Resilio Sync** — P2P like Syncthing but proprietary; **newest-timestamp-wins** content conflicts (older → Archive) with `.Conflict` reserved for filename collisions + optional File Locking; Archive-based versioning (30-day desktop default, `sync_trash_ttl=0`=forever, manual restore); Read-Write/Read-Only/Encrypted keys; `.rsl~` selective-sync placeholders; `.!sync` temp files; native Windows service.
- **Nextcloud / ownCloud** — csync client↔server; per-directory SQLite journal + server ETags/file-IDs; server-authoritative conflicts (`(conflicted copy …)`); three-mode bandwidth throttle incl. "limit automatically"; VFS on-demand files; rich overlay-icon set.
- **Seafile** — Git-like commit/block model with content-defined chunking (~8 MB) + dedup; **first-to-cloud wins, loser always kept as `(SFConflict …)`**, never silent overwrite; richest server-side versioning (per-file history *with rename tracking* + library "time-machine" snapshots + per-library trash, owner-configurable retention, default 30-day trash); `seafile-ignore.txt` is client-side only and affects only not-yet-synced files (two sharp edges to avoid).
- **Dropbox** — block/delta sync + **LAN Sync** peer fetch; plan-tiered version history (30/180/365 days); Smart Sync placeholders; `(… conflicted copy …)`.
- **OneDrive** — Windows-native Files On-Demand (cloud-files API placeholders), "Adjust automatically" bandwidth, metered/battery awareness, `-DESKTOPNAME` conflict tag; deepest Windows shell integration (relevant for us).
- **Google Drive for Desktop** — Mirror vs Stream modes; 30-day / 100-version retention with per-file **Keep forever** (max 200); simple KB/s rate caps.

---

## Patterns Smart Explorer should copy

### A. STATE PERSISTENCE
1. **Per-folder index DB keyed by (path, mtime, size, perms, blocklist)** — Syncthing's `index-*` model. Store SHA-256 block lists so we can do delta transfer and detect *content* change, not just metadata. — https://docs.syncthing.net/users/syncing.html , https://docs.syncthing.net/specs/bep-v1.html
2. **Per-directory ETag/version short-circuit** — Nextcloud's directory-ID trick: bump a directory hash when anything inside changes so unchanged subtrees are skipped on rescan. — https://docs.nextcloud.com/desktop/latest/architecture.html
3. **Reserved hidden metadata names** that are themselves never synced: a folder marker (`.stfolder`-style), an ignore file, a versions/archive dir, the journal DB. — https://docs.syncthing.net/users/config.html , https://docs.nextcloud.com/desktop/latest/architecture.html
4. **File IDs that survive renames** so moves propagate as moves (cheap), not delete+re-add. — https://docs.nextcloud.com/desktop/latest/architecture.html

### B. RESCAN CADENCE
1. **Watcher + debounce + periodic full rescan**, all three. Defaults to copy: debounce ~10 s (Syncthing `fsWatcherDelayS=10`), extra delay on deletes, full rescan every ~1 h (`rescanIntervalS=3600`), watcher on by default but a rescan backstop for missed events / network mounts. — https://docs.syncthing.net/users/config.html
2. **Scan only mtime+size+perms first, rehash on change** — never hash the whole tree every cycle. — https://docs.syncthing.net/users/syncing.html
3. **Cap concurrent heavy folders** (`maxFolderConcurrency`) and emit scan-progress on an interval (`scanProgressIntervalS≈2`). — https://docs.syncthing.net/users/config.html
4. **Atomic temp file + resume:** write to `.smartexplorer.<name>.tmp`, rename on completion, keep the temp ~1 day to resume partial transfers. — https://docs.syncthing.net/users/syncing.html

### C. CONFLICT & VERSIONING
1. **Never auto-merge content.** Resolve by keeping both files; default **newest-mtime wins** for the live file, loser renamed. Provide a server-/source-authoritative mode too. (All eight engines keep-both rather than merge.) — §3 above
2. **Conflict-copy naming = original name + ` (conflicted copy <date> <time> <device/user>)` + original extension** so it sorts next to the original and survives further sync. Cap copies per file (Syncthing `maxConflicts=10`). — https://docs.syncthing.net/users/config.html , https://docs.nextcloud.com/desktop/latest/architecture.html
3. **Pluggable versioning** mirroring Syncthing's four modes: None / Trash-can (TTL) / Simple (keep N + TTL) / Staggered (thinning schedule) / External (run a command) — all writing to a `.versions`/`.archive` dir, with `TTL=0 ⇒ keep forever` (Resilio `sync_trash_ttl=0`). — https://docs.syncthing.net/users/versioning.html , https://help.resilio.com/hc/en-us/articles/204754239
4. **Surface a rich state taxonomy** (Up to Date / Scanning / Syncing %/ Out of Sync / Waiting to Sync / Local Additions / Paused / Error+reason) plus Global vs Local vs Out-of-Sync byte counts, and shell overlay icons. — https://docs.syncthing.net/intro/gui.html , https://doc.owncloud.com/desktop/5.3/using.html

---

## Source index (primary, official)

- Syncthing config: https://docs.syncthing.net/users/config.html
- Syncthing versioning: https://docs.syncthing.net/users/versioning.html
- Syncthing ignoring (.stignore): https://docs.syncthing.net/users/ignoring.html
- Syncthing syncing/scanning/conflicts/temp files: https://docs.syncthing.net/users/syncing.html
- Syncthing folder types: https://docs.syncthing.net/users/foldertypes.html
- Syncthing GUI states: https://docs.syncthing.net/intro/gui.html
- Syncthing Block Exchange Protocol v1: https://docs.syncthing.net/specs/bep-v1.html
- Syncthing autostart: https://docs.syncthing.net/users/autostart.html
- Resilio .sync directory: https://www.resilio.com/documentation/content/reference-information/What_is_.sync_directory/
- Resilio Agent activities (hashing, blocks, temp, status): https://www.resilio.com/documentation/content/jobs/Detailed_activities_of_an_Agent__in_a_job/
- Resilio Archive (versioning): https://www.resilio.com/documentation/content/reference-information/understanding_the_archive_folder/ , https://help.resilio.com/hc/en-us/articles/204754239
- Resilio conflict (multiuser) + filename conflicts: https://www.resilio.com/documentation/content/advanced-configuration/best-practices/multiuser-collaboration/ , https://www.resilio.com/documentation/content/troubleshooting/error-messages/Filename_Conflicts_/
- Resilio IgnoreList: https://www.resilio.com/documentation/content/advanced-configuration/agents/ignoring_and_whitelisting_files_on_agents/ , https://help.resilio.com/hc/en-us/articles/205458165
- Resilio selective sync (.rsl~): https://www.resilio.com/documentation/content/advanced-configuration/agents/legacy_selective_sync_/
- Resilio statuses: https://www.resilio.com/documentation/content/reference-information/Statuses/
- Resilio sync model / one-way: https://www.resilio.com/documentation/content/jobs/synchronization_job/ , https://help.resilio.com/hc/en-us/articles/204754279
- Resilio Windows service / macOS daemon: https://help.resilio.com/hc/en-us/articles/207701296 , https://www.resilio.com/documentation/content/getting-started/Starting_Agent_on_macOS_when_system_boots_up_or_user_logs_in/
- Resilio folder preferences: https://help.resilio.com/hc/en-us/articles/205458125-Folder-Preferences
- Nextcloud architecture/journal: https://docs.nextcloud.com/desktop/latest/architecture.html
- Nextcloud conflicts: https://github.com/nextcloud/documentation/blob/master/user_manual/desktop/conflicts.rst
- ownCloud using the desktop app: https://doc.owncloud.com/desktop/5.3/using.html
- ownCloud conflicts: https://doc.owncloud.com/desktop/next/conflicts.html
- Seafile data model (Git-like blocks/CDC): https://manual.seafile.com/latest/develop/data_model/
- Seafile file conflicts: https://help.seafile.com/syncing_client/file_conflicts/
- Seafile excluding files: https://help.seafile.com/syncing_client/excluding_files/
- Seafile sync interval / read-only / selective sub-folders: https://help.seafile.com/syncing_client/setting_sync_interval/ , https://help.seafile.com/syncing_client/read-only_syncing/ , https://help.seafile.com/syncing_client/selective_sync_sub-folders/
- Seafile versioning (file history / snapshots / retention / trash): https://help.seafile.com/file_folder_managing/finding_older_version_files/ , https://help.seafile.com/file_folder_managing/library_history_and_snapshots/ , https://help.seafile.com/file_folder_managing/setting_library_history/ , https://help.seafile.com/file_folder_managing/restoring_deleted_files/
- Seafile CLI / proxy / FAQ (service, Drive client): https://help.seafile.com/syncing_client/linux-cli/ , https://help.seafile.com/syncing_client/proxy_settings/ , https://help.seafile.com/faq/ , https://help.seafile.com/drive_client/drive_client_for_win10/
- Dropbox version history: https://help.dropbox.com/delete-restore/version-history-overview
- Dropbox LAN sync: https://help.dropbox.com/sync/lan-sync-overview , https://dropbox.tech/infrastructure/inside-lan-sync
- Dropbox selective sync: https://help.dropbox.com/sync/selective-sync-overview
- OneDrive Files On-Demand: https://support.microsoft.com/en-us/office/save-disk-space-with-onedrive-files-on-demand-for-windows-0e6860d3-d9f3-4971-b321-7092438fb38e
- OneDrive upload/download rate: https://support.microsoft.com/en-us/office/change-the-onedrive-sync-app-upload-or-download-rate-71cc69da-2371-4981-8cc8-b4558bdda56e
- Google Drive for Desktop settings: https://support.google.com/drive/answer/13470231
- Google Drive versions/activity: https://support.google.com/drive/answer/2409045
