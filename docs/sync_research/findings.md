# Sync research — distilled, implementation-relevant findings (cited)

Concrete numbers/behaviors to implement against, distilled from the full
multi-tool sweep (2026-06-17). Grouped by the decision they inform.

## Trigger types seen in the wild (→ Group D)
- **Manual; fixed interval; calendar (daily/weekly/monthly @ time)** — all tools.
- **Real-time watch** with a *minimum pause / idle delay* between runs — Bvckup2
  ("run as soon as there are changes… 'minimum pause between backups'… near
  real-time, delay of several seconds"); SyncBack ("when files change" + "wait
  until idle N seconds"); ownCloud/Nextcloud watcher + 2 h fallback scan.
- **On device/USB connect**, match broad or by label/serial/drive-letter
  (wildcards) — SyncBack `wheninsert`; Bvckup2 ("as soon as source/target device
  is plugged in"); GoodSync on-connect.
- **On startup / logon / logoff / shutdown / screensaver / idle** — SyncBack run-situations.
- **Run as a system service** so backups run with no user logged in — Bvckup2.
- **Pause/resume** durations: OneDrive 2/8/24 h; Dropbox 30 min/1 h/until-tomorrow/indefinitely.
- **Auto-pause on metered network / battery saver** — OneDrive (default ON;
  `DisablePauseOnMeteredNetwork`/`DisablePauseOnBatterySaver`).
- **Rescan/force intervals** — ownCloud/Nextcloud `remotePollInterval` 30 s,
  `forceSyncInterval` 2 h, `fullLocalDiscoveryInterval` 1 h.

## Comparison / change-detection (→ Group C)
- Default = **mtime + size** quick check; opt-in **checksum** for certainty
  (rsync `--checksum`, SyncBack hash, ownCloud `OC-Checksum` SHA1/MD5/Adler32).
- **size-only / ignore-times** modes — rclone.
- **modify-window** tolerance for FAT/exFAT/DST: rsync `--modify-window=1|2`;
  SyncBack "ignore differences ≤ 2 s".
- **Baseline/journal** to know what changed: ownCloud per-dir SQLite journal +
  ETag; SyncBack "Intelligent Sync" history; Dropbox cursor; OneDrive cTag(content)
  vs eTag(whole). We already do this (baseline TSV).
- **Rename/move detection** via stable file-id (ownCloud `oc:id`), atomic moves
  (Dropbox Nucleus), SyncBack "detect renames", rsync `--fuzzy`.
- **Fast scan**: trust last-run DB, skip full dest re-scan — SyncBack "Fast Backup".
- **Differential/block transfer**: Dropbox 4 MiB blocks (SHA-256 blocklist,
  "need blocks"); Bvckup2 64 KB blocks (Blake2b); Drive "differential uploads"
  (v99, Oct 2024); OneDrive differential all types; <8 MB inline / ≥8 MB chunked.
  (For remotes we keep whole-file; deltas are a later optimization.)

## Direction / mode (→ Group B) — the SyncToy taxonomy is the clean reference
- **Synchronize** (two-way; new/updated both ways; renames+deletes both ways).
- **Echo** (one-way L→R; new/updated; renames+deletes propagate L→R = mirror).
- **Contribute** (one-way L→R; new/updated + renames; **no deletions** = additive/safe).
- Plus **Move** (copy then delete source — SyncBack) and **Backup/echo+versioned**.

## Conflict policies (→ Group E)
- **Strict / surface** (never auto-overwrite) — our default; every careful tool.
- **Newer / older / larger / smaller wins; left wins / right wins** — SyncBack Decisions.
- **Keep both** with conflict-copy naming: ownCloud/Nextcloud
  `name (conflicted copy YYYY-MM-DD HHMMSS).ext`; OneDrive appends device name;
  Syncthing `*.sync-conflict-<date>-<time>-<modder>*`; Dropbox
  `<name> (<user>'s conflicted copy <date>).ext` (last save becomes the copy);
  also Dropbox case/whitespace/selective-sync/unicode conflict suffixes.
- **Per-situation decision matrix** (only-on-A / only-on-B / both-changed / same)
  — SyncBack 5 "Decisions" situations.

## Versioning / retention schemes (→ Group F)
- **Keep last N count**: SyncBack default 32; OneDrive personal 25; Drive "30 days
  or 100 newer versions"; "keep forever" pin.
- **Keep N days** (our current model).
- **Staggered / thinning (Time Machine / ownCloud / Syncthing "Staggered")**:
  TM = hourly for 24 h, daily for a month, weekly for all older, oldest deleted
  when full; ownCloud/Nextcloud version schedule = 1/2 s for 10 s, 1/10 s for a
  min, 1/min for an hour, 1/hr for a day, 1/day for a month, 1/week after; cap at
  50 % free space; named versions never expire.
- **GFS** (restic/Borg `--keep-last/hourly/daily/weekly/monthly/yearly`).
- **Recycle/Trash instead of delete** — SyncBack recycle bin; ownCloud `moveToTrash`;
  Syncthing "Trash Can"; trash retention OneDrive 30/93 d, Drive 30 d, Nextcloud 30 d.
- **Versioning store layouts**: SyncBack `$SBV$` hidden dir; ownCloud
  `files_versions`; delta/patch version storage (SyncBack/Bvckup2).
- **Max-delete / mass-change guard**: rclone `--max-delete`; SyncBack ransomware
  detection — abort/prompt if > X files or X% would be deleted (protects against a
  vanished/remounted side). We must add this before enabling aggressive mirror.

## Filters (→ Group G)
- Glob include/exclude (we have); by **size** (rclone `--min/max-size`, Bvckup2),
  by **age/date** (rclone `--max-age`, Bvckup2 "modified between…"), by attribute.
- Default ignore set: `*.tmp`, `~$*`, `desktop.ini`, `.DS_Store`, `Thumbs.db`,
  `System Volume Information`, our own version store (OneDrive skips `.tmp/.ini`;
  Nextcloud default `sync-exclude.lst`).
- Rule files: Dropbox `rules.dropboxignore` (`*.log`, `build/`, `!re-include`, `#comments`).
- "Ask before syncing folders larger than X" — ownCloud/Nextcloud.

## Bandwidth / performance (→ Group H)
- Up/down KB/s limits: OneDrive 50–100000 KB/s; Drive 1–100000 KB/s; SyncBack
  KB/s; ownCloud/Nextcloud; Dropbox custom toggle; Bvckup2 read & write capped
  independently.
- **Auto / % of throughput**: ownCloud 25 %; OneDrive LEDBAT "Adjust automatically"
  = 70 %, or fixed % 10–99.
- Concurrency: SyncBack 3 threads (max 128); robocopy `/MT`; rclone `--transfers`.
- Low I/O priority; LAN-direct transfer (Dropbox LAN sync).

## Reliability (→ Group I)
- **Atomic temp-then-rename / safe copies** — SyncBack "make safe copies"; rsync
  `--partial`. **Verify after copy** (re-read/hash) — SyncBack, Bvckup2 write-verify.
- **Retry** count + delay/backoff — robocopy `/R /W`; SyncBack; Nextcloud 3 retries;
  Bvckup2 auto-retry transient errors.
- **Resume** interrupted transfers — Bvckup2 ("file copying is resumable").
- **VSS** open/locked-file copy (Windows) — SyncBack, Bvckup2.
- **Run program before/after** — SyncBack; **email/notify on result** — SyncBack,
  Bvckup2 (also "missed run"/"cancelled" alerts).
- **Dry-run/preview** (we have).

## State / observability (→ Group J)
- Status set: **up-to-date / scanning(indexing) / syncing / paused / has-conflicts
  / error / offline** — universal (ownCloud/Nextcloud/OneDrive/Dropbox icon legends).
- Overlay + tray icon states; per-file overlay; "Not synced"/activity tab with
  recent activity + errors; yellow "unresolved conflicts" badge.
- Logs: per-job log file, `--logfile/--logdir/--logexpire`, log window.

## Service / lifecycle (→ Group K)
- Start at logon (we have HKCU Run); OneDrive `EnableAutoStart`; Drive
  `AutoStartOnLogin`; "Launch on system startup" toggle everywhere.
- Run as a **true service** (no user logged in) — Bvckup2; (advanced/optional).
- Tray presence + background launch (`--background`) — Nextcloud/ownCloud.
- Restart the background worker after self-update (our K3 gap to verify).

## Config / portability (→ Group L)
- Per-job config files (SyncBack profiles, Bvckup2 INI per job); portable mode
  (`-c <configdir>` / redirect.ini); import/export; global defaults.
</content>
