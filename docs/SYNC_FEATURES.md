# Smart Explorer — Master Sync Feature List & Build Plan

The complete inventory of folder-sync/backup features across the field, grouped
the way they'll appear in the UI, each marked **[HAVE]** (already in
`bisync.rs`/`syncjobs.rs`/`daemon.rs`), **[EXT]** (partly there, needs
extending), or **[BUILD]** (new). This is the blueprint for "implement them all,
behind grouped options, with readable state and reliability."

Research basis (cited inventories in `docs/sync_research/`): FreeFileSync +
RealTimeSync, GoodSync, **SyncBackPro/SE** (2BrightSparks), Allway Sync, Bvckup2;
**Syncthing**, Resilio, **Nextcloud/ownCloud**, **OneDrive**, Dropbox, Google
Drive; **rsync, rclone (sync/copy/bisync), robocopy, restic/Borg/Kopia, Time
Machine, Windows File History**. Key source URLs are listed per group.

Today: 2026-06-17.

---

## What we already have (the foundation)

- **Baseline 3-way engine** (`bisync.rs`): records each side's last-sync state so
  it knows what *changed*, not just what differs; one side changed → propagate,
  both changed → conflict. Reversible (old bytes copied to a versions store,
  pruned by retention days). Dry-run. Parallel walk. — this is exactly the
  rclone-bisync / SyncBack "Intelligent Sync" model.
- **Persistent jobs** (`syncjobs.rs`): source, target, direction, conflict,
  retain_days, interval_min, include_hidden, ignore globs, last_run, enabled.
- **Headless daemon** (`daemon.rs`) + **logon autostart** (`autostart.rs`):
  `--sync-daemon`, 60 s loop runs due jobs, heartbeat, stop sentinel, capped log.
- **UI**: job list, job editor, conflict resolver.

---

## Group A — Endpoints, pairing & multi-target
- A1 Local↔local, local↔remote (SFTP/FTP/WebDAV/Drive), remote↔remote **[HAVE]** (`resolve_endpoint`)
- A2 **Multi-target fan-out**: one source → many destinations (backup to several places) **[BUILD]** (SyncBack Groups; GoodSync many-to-one)
- A3 **Job groups / batch**: run several jobs sequentially *or in parallel*, ordered **[BUILD]** (SyncBack Group Profiles / Group Queues — sequential default, parallel option)
- A4 Run-before / run-after another job (chaining) **[BUILD]**

## Group B — Direction & mode
- B1 Two-way (bidirectional) **[HAVE]** (`Both`)
- B2 One-way **mirror** (make dest identical, delete extras) **[EXT]** (have `AtoB`; need explicit delete-extras vs keep)
- B3 One-way **update/contribute** (copy new/changed, never delete) **[BUILD]** (rsync default; Syncthing send-only)
- B4 **Move** (copy then delete from source) **[BUILD]** (SyncBack "Move Files")
- B5 **Echo/backup** (one-way + versioned destination) **[BUILD]**
- B6 **Detect renames/moves** (don't re-transfer a renamed file) **[BUILD]** (SyncBack, ownCloud file-id, rsync `--fuzzy`)

## Group C — Change detection & comparison
- C1 Baseline-driven "what changed" **[HAVE]**
- C2 Compare by **mtime + size** (default) **[HAVE]**
- C3 Compare by **content hash/checksum** (certainty over speed) **[BUILD]** (SyncBack hash; rsync `--checksum`; ownCloud checksums)
- C4 **Size-only** / **ignore-mtime** modes **[BUILD]** (rclone `--size-only`, `--ignore-times`)
- C5 **mtime tolerance / modify-window** (FAT 2 s, DST/timezone) **[BUILD]** (rsync `--modify-window`; SyncBack "ignore ≤2 s")
- C6 Attribute compare (hidden/read-only) **[BUILD-opt]** (SyncBack)
- C7 **Fast scan** (trust baseline, skip full dest re-scan) **[EXT]** (SyncBack "Fast Backup")

## Group D — Triggers & scheduling  ← the heart of the request
- D1 Manual "Run now" **[HAVE]**
- D2 Fixed **interval** (every N min) **[HAVE]**
- D3 **Calendar schedule**: daily/weekly/monthly at specific time(s) **[BUILD]** (FreeFileSync via Task Scheduler; SyncBack scheduled)
- D4 **On app/system startup** (run on launch / at logon) **[BUILD]** (SyncBack "on Windows startup")
- D5 **On logon daemon** (background service started at login) **[HAVE]** (`autostart`)
- D6 **Real-time** filesystem watch, debounced + idle-delay before run **[BUILD]** (FreeFileSync RealTimeSync; SyncBack "when files change"; ownCloud watcher; Syncthing fsWatcher)
- D7 **On device/USB connect**, matched by volume label / serial / drive letter (wildcards) **[BUILD]** (SyncBack "device insertion"; GoodSync "on connect")
- D8 **On idle** (after N s of no activity) **[BUILD]** (SyncBack idle)
- D9 **On logoff/shutdown / screensaver / display-off** **[BUILD]** (SyncBack)
- D10 **Pause/resume**, with quick durations (2 / 8 / 24 h) **[BUILD]** (OneDrive)
- D11 **Auto-pause on metered network / battery saver** **[BUILD]** (OneDrive `DisablePauseOnMeteredNetwork`/`…BatterySaver`)
- D12 **Active hours / blackout windows** (only run between X–Y; or never during Z) **[BUILD]**
- D13 **Catch-up** a missed scheduled run on next start **[BUILD]**
- D14 Periodic **force-resync / full rescan interval** **[BUILD]** (ownCloud `forceSyncInterval` 2 h, `fullLocalDiscoveryInterval` 1 h)
- D15 Daemon **wake cadence** (how often it checks for due jobs) editable **[EXT]** (currently fixed 60 s)

## Group E — Conflict handling
- E1 **Strict** (both-changed = conflict, never auto-overwrite) **[HAVE]** (`FileLevel`)
- E2 **Newer wins** **[HAVE]**
- E3 Older / Larger / Smaller wins **[BUILD]** (SyncBack decisions)
- E4 **Source(left) wins / Dest(right) wins** **[BUILD]**
- E5 **Keep both** (rename loser with conflict suffix) **[BUILD]** (ownCloud `(conflicted copy <ts>)`, OneDrive device-name suffix, Syncthing `*.sync-conflict-*`)
- E6 **Prompt / interactive resolve** **[HAVE]** (conflict resolver UI)
- E7 **Per-situation decision matrix** (file only on A / only on B / both changed / same) **[BUILD]** (SyncBack "Decisions")
- E8 Conflict-copy naming scheme (timestamp + device) **[BUILD]**

## Group F — Versioning, retention & deletion safety
- F1 Reversible versions store **[HAVE]**
- F2 Retain by **days** **[HAVE]**
- F3 Retain by **count** (keep last N versions) **[BUILD]** (SyncBack default 32; OneDrive 25)
- F4 **Versioning schemes** **[BUILD]**:
  - Recycle/Trash-can (latest deleted only) — Syncthing "Trash Can"
  - Simple (keep N) — Syncthing "Simple"
  - **Staggered / thinning** (1/hr for a day, 1/day for a month, 1/week after) — Syncthing "Staggered", ownCloud version schedule, Time Machine thinning
  - **GFS** (keep-last/hourly/daily/weekly/monthly/yearly) — restic/Borg `--keep-*`
- F5 **Recycle Bin instead of hard delete** (local) **[BUILD]** (SyncBack; ownCloud `moveToTrash`)
- F6 **Max-delete / max-change safety limit** (abort or prompt if > X files or X% would be deleted) **[BUILD]** (rclone `--max-delete`; protects against a vanished/remounted side)
- F7 Versions space cap (never exceed X% free) **[BUILD-opt]** (ownCloud 50%)
- F8 Delta/patch version storage **[NICE]** (SyncBack delta)

## Group G — Filters & selection
- G1 Include/exclude **globs** **[HAVE]** (`ignore`)
- G2 Include hidden toggle **[HAVE]**
- G3 By **size** (min/max) **[BUILD]** (rclone `--min-size/--max-size`)
- G4 By **age/date** (modified after/before, max-age) **[BUILD]** (rclone `--max-age`)
- G5 **Default ignore set** (`*.tmp`, `~$*`, `desktop.ini`, `.DS_Store`, `Thumbs.db`, our own version store) **[BUILD]** (OneDrive skips `.tmp/.ini`)
- G6 Exclude-if-present (skip dir containing a marker file) **[NICE]** (rclone `--exclude-if-present`)
- G7 Selective subfolders (pick which subtrees) **[BUILD]** (selective sync)
- G8 Reserved/invalid-name & path-length guarding for cross-fs targets **[BUILD-opt]** (OneDrive restricted chars / 400-char path)

## Group H — Bandwidth & performance
- H1 Parallel walk/transfer per backend **[HAVE]** (`parallelism()`)
- H2 **Bandwidth limit** KB/s (and separate up/down) **[BUILD]** (rclone `--bwlimit`; SyncBack; ownCloud)
- H3 Auto-limit (% of available) **[NICE]** (ownCloud 25%)
- H4 Time-scheduled bandwidth **[NICE]** (rclone `--bwlimit` timetable)
- H5 Configurable transfer concurrency **[BUILD]**

## Group I — Reliability
- I1 **Atomic temp-then-rename** writes **[EXT/verify]** (rsync `--partial`; SyncBack "safe copies")
- I2 **Verify after copy** (re-read/hash) **[BUILD]** (SyncBack verify; ownCloud `OC-Checksum`)
- I3 **Retry** on failure (count + backoff + delay) **[BUILD]** (robocopy `/R /W`; SyncBack)
- I4 **Resume** partial transfers **[NICE]**
- I5 **Dry-run / preview** "what would change" **[HAVE]**
- I6 Run **program before/after** a job **[BUILD]** (SyncBack)
- I7 Locked/open-file copy (Windows **VSS**) **[NICE-adv]** (SyncBack VSS)
- I8 Ransomware/mass-change guard (ties to F6) **[BUILD]** (SyncBack ransomware detection)

## Group J — State, observability & notifications  ← "readability of states"
- J1 **Per-job status**: idle / scanning / syncing / up-to-date / has-conflicts / paused / error **[BUILD]** (every engine surfaces this)
- J2 Last-run time + **result stats** (→/←/del/conflict/err/bytes) **[HAVE]**
- J3 **Live progress** (current file, files done/total, bytes, %) **[BUILD]**
- J4 **Readable per-job history log** in the GUI **[EXT]** (daemon log exists; surface it)
- J5 Daemon **alive/heartbeat** indicator **[HAVE]**
- J6 **Notifications** on completion/conflict/error (in-app toast; optional email) **[BUILD]** (SyncBack email; OneDrive activity center)
- J7 Tray icon + overlay status **[NICE]** (ownCloud/OneDrive)
- J8 Conflict list + resolver **[HAVE]**

## Group K — Service & lifecycle  ← "autostart after startup and update"
- K1 Headless daemon (same exe) **[HAVE]**
- K2 Autostart at logon (HKCU Run, no admin, reversible) **[HAVE]**
- K3 **Restart daemon after self-update** (so a new version keeps syncing) **[BUILD]** — verify the update flow re-launches `--sync-daemon`
- K4 Start-daemon-on-GUI-launch option **[EXT]**
- K5 Single-instance guard **[HAVE]**
- K6 Run when no user logged in (true Windows **service**) **[NICE-adv]**

## Group L — Config & portability
- L1 Per-job persistent config **[HAVE→migrate]** (TSV → structured/JSON for the larger option set)
- L2 **Grouped, collapsible option UI** with safe defaults **[BUILD]** ← the organizing principle
- L3 Global defaults applied to new jobs **[BUILD]**
- L4 Import/export job config **[NICE]** (SyncBack portable profiles)
- L5 Per-job enable/disable **[HAVE]**

---

## Build phases (each phase = a shippable, tested release)

1. **Model & persistence migration** — extend the job model for every option
   below as typed, grouped structs; migrate persistence to a forward-compatible
   structured format (keep reading old TSV); regroup the editor into collapsible
   sections (Group L2). No behavior change yet — pure scaffolding the rest sits on.
2. **Triggers & scheduling** (Group D) — calendar schedule, on-startup,
   real-time watch (debounced), USB/device-connect, idle, active-hours/blackout,
   pause + auto-pause (metered/battery), catch-up, editable daemon cadence.
3. **Direction & comparison** (B + C) — mirror/update/move/echo, rename detect,
   checksum/size-only/ignore-mtime, modify-window, fast-scan.
4. **Conflict policies** (E) — full decision set + keep-both + per-situation matrix.
5. **Versioning & safety** (F) — count retention, staggered/GFS schemes, recycle
   bin, max-delete guard.
6. **Filters** (G) — size/age/default-ignore/selective-subfolders.
7. **Reliability + bandwidth** (I + H) — verify, retry, run-before/after,
   bwlimit, concurrency.
8. **Observability + service polish** (J + K) — per-job live status/progress,
   history view, notifications, post-update daemon restart.

Each phase lands behind grouped options, defaults stay safe (two-way, strict
conflicts, reversible), and every new control is persisted and shown in a
readable state line.

### Key research URLs
- SyncBack run-situations: https://help.2brightsparks.com/support/solutions/articles/43000335862 · device-insert: https://www.2brightsparks.com/syncback/help/wheninsert.htm · decisions: https://www.2brightsparks.com/syncback/help/decisionsfiles.htm · versioning: https://www.2brightsparks.com/syncback/help/copydeleteversioning.htm
- ownCloud architecture/intervals: https://doc.owncloud.com/desktop/5.3/appendices/architecture.html · config: https://doc.owncloud.com/desktop/5.3/advanced_usage/configuration_file.html · versions: https://doc.owncloud.com/server/next/classic_ui/files/version_control.html
- OneDrive sync model: https://learn.microsoft.com/en-us/SharePoint/sync-process · pause: https://support.microsoft.com/en-us/office/how-to-pause-and-resume-sync-in-onedrive-2152bfa4-a2a5-4d3a-ace8-92912fb4421e · policies: https://learn.microsoft.com/en-us/sharepoint/use-group-policy
- Syncthing versioning: https://docs.syncthing.net/users/versioning.html · rclone flags: https://rclone.org/docs/ · rsync man: https://download.samba.org/pub/rsync/rsync.1 · restic forget/GFS: https://restic.readthedocs.io/en/stable/060_forget.html · robocopy: https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/robocopy
