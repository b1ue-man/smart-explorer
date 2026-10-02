# Review-Befunde: Sync-Zuverlässigkeit (Windows, Linux, Android; Echtzeit und Backups)

Stand: 2026-10-02. Quelle: Review-Workflow (Finder je Dimension, unabhängige Gegenprüfung, Vollständigkeits-Kritiker). Fehlende Dimensionen: keine.

| ID | Schwere | Urteil | Dimension | Titel |
|---|---|---|---|---|
| Y01 | high | confirmed | triggers-realtime | On-connect trigger can never fire on Linux, yet the desktop editor offers it |
| Y02 | high | confirmed | triggers-realtime | Real-time: a settled change is discarded when the job is running or was admitted less than 60 s ago |
| Y03 | high | confirmed | triggers-realtime | Real-time signature misses renames/moves, folder-only changes and same-size edits |
| Y04 | high | confirmed | triggers-realtime | Real-time debounce restarts on every change with no maximum wait, so continuous writers starve the job forever |
| Y05 | high | confirmed | triggers-realtime | Real-time state is not persisted: edits made while the worker was down, or not yet settled before shutdown/restart, are never synced |
| Y06 | high | confirmed | triggers-realtime | Real-time polling walks every local root on the scheduler thread each tick; silent 1M-entry cap; NAS shares walked over the network |
| Y07 | high | confirmed | triggers-realtime | Real-time remote probe connects every tick before checking change support; remote sides are mostly unwatched; remote-only real-time jobs never run |
| Y08 | high | confirmed | triggers-realtime | Windows: data reparse points (cloud placeholders, WOF, dedup) are skipped by the real-time signature, leaving OneDrive/Files-On-Demand folders effectively unwatched |
| Y09 | high | confirmed | triggers-realtime | Failures before the sync starts are silent: no result, no last run, retried every minute forever |
| Y10 | high | confirmed | triggers-realtime | Windows on-connect only recognises DRIVE_REMOVABLE volumes: USB hard disks/SSDs never trigger, card-reader slots misfire |
| Y11 | high | confirmed | triggers-realtime | Android 'Dauerbetrieb': no wake lock for scheduling or job runs, scheduler time stops in suspend, WorkManager fallback skipped |
| Y12 | high | confirmed | triggers-realtime | Android 'Periodisch' (default mode): jobs the daemon's own schedule starts inside a worker window are not bound to that window |
| Y13 | medium | confirmed | triggers-realtime | Cancelled runs of event-triggered jobs are lost; the startup pass is skipped when the worker starts paused |
| Y14 | medium | confirmed | triggers-realtime | On-connect arrivals are consumed without a run (drive present at start, outside active hours, job busy or cooling down); matched volume is not tied to the synced path |
| Y15 | medium | confirmed | triggers-realtime | Calendar jobs without catch-up fire only if a tick lands within 120 s of the occurrence, but the tick can be up to 3600 s or blocked |
| Y16 | medium | confirmed | triggers-realtime | Failed runs count as completed: no retry until the next interval/occurrence; cancelled manual runs also push the schedule |
| Y17 | medium | confirmed | triggers-realtime | Queued jobs run a stale configuration snapshot (also after deletion, disabling or retargeting) |
| Y18 | medium | confirmed | triggers-realtime | No exclusion between manual runs and background runs of the same job/pair |
| Y19 | low | confirmed | triggers-realtime | Desktop GUI writes back a stale last_run, reverting the daemon's mark_run |
| Y20 | medium | confirmed | triggers-realtime | One invalid job file stops all scheduled jobs and catch-up runs |
| Y21 | medium | partially_confirmed | triggers-realtime | Global one-at-a-time execution with no runtime limit or hang watchdog; stop/pause joins without timeout |
| Y22 | medium | confirmed | triggers-realtime | Real-time signature ignores the job's filters and its own writes: excluded churn and self-writes cause useless full runs |
| Y23 | medium | confirmed | triggers-realtime | Android periodic runs: ~10-minute cap without foreground, success returned on failures, wasted work per window |
| Y24 | medium | confirmed | triggers-realtime | Desktop worker lifecycle: no crash restart, Windows startup-disable not detected, Linux only via XDG autostart, nothing while logged off or asleep |
| Y25 | low | confirmed | triggers-realtime | Linux auto-pause toggles are no-ops |
| Y26 | low | confirmed | triggers-realtime | Calendar edge cases: DST days skipped, day 29-31 skipped in short months, new jobs run immediately, future last_run blocks runs |
| Y27 | low | confirmed | triggers-realtime | 'Letzter Hintergrundlauf' is updated by cancelled, blocked or empty catch-up runs |
| Y28 | low | confirmed | triggers-realtime | Desktop 'Beim Start' jobs run on every worker (re)start, not once per logon or boot |
| Y29 | critical | confirmed | bisync-plan-state | One failed file discards the whole run's baseline update (any error => old baseline kept) |
| Y30 | critical | confirmed | bisync-plan-state | Mirror (backup) full runs re-copy and re-version every file because copies never receive the source mtime |
| Y31 | critical | confirmed | bisync-plan-state | An empty, unmounted or swapped side is taken at face value: mass deletion, all delete guards off by default |
| Y32 | high | confirmed | bisync-plan-state | Cancelled, killed or failed runs turn every completed transfer into a conflict (or duplicate/redundant copy) on the next run |
| Y33 | high | confirmed | bisync-plan-state | Post-apply full re-walk records edits made during the run as already synchronized (silent loss of propagation) |
| Y34 | high | confirmed | bisync-plan-state | A single unreadable, unrepresentable or vanishing entry aborts the whole snapshot and therefore every run |
| Y35 | high | confirmed | bisync-plan-state | Hard 1,000,000-entry / 128 MiB caps make large trees permanently unsyncable |
| Y36 | high | confirmed | bisync-plan-state | Size/age/hidden filters turn 'left the filter on one side' into a deletion of the counterpart |
| Y37 | high | confirmed | bisync-plan-state | Case-insensitive pairs are keyed by exact case: case differences cause permanent drift errors, split baselines or delete/copy churn |
| Y38 | high | confirmed | bisync-plan-state | Count/Staggered/GFS version retention deletes the only recovery copy of most files |
| Y39 | high | confirmed | bisync-plan-state | Incremental mirror never re-verifies the destination and has no destination identity outside Drive |
| Y40 | medium | confirmed | bisync-plan-state | One-way Propagate/NoDelete jobs never repair destination-side changes; a later source edit becomes a blocking conflict |
| Y41 | medium | confirmed | bisync-plan-state | OlderWins/SmallerWins (and LargerWins for empty files) let a deletion win over a modification |
| Y42 | medium | confirmed | bisync-plan-state | Post-apply observation re-walks whole trees; in Checksum mode it re-reads every file |
| Y43 | medium | confirmed | bisync-plan-state | Drive change-feed incremental mirror falls back to a full rebuild for nearly every change |
| Y44 | medium | confirmed | bisync-plan-state | A corrupt or locked sync_state.sqlite blocks every Mirror job |
| Y45 | medium | confirmed | bisync-plan-state | Desktop conflict resolution overwrites a newer on-disk baseline; runs of one pair are not mutually excluded |
| Y46 | medium | confirmed | bisync-plan-state | FAT32 timestamps shift by one hour at DST changes and are treated as modifications |
| Y47 | medium | confirmed | bisync-plan-state | FTP modification times come from LIST rows and change precision as files age |
| Y48 | medium | confirmed | bisync-plan-state | No Unicode normalization of relative paths |
| Y49 | medium | confirmed | bisync-plan-state | Linux FIFOs/sockets/devices are synced as regular files; a FIFO blocks the run indefinitely |
| Y50 | medium | confirmed | bisync-plan-state | No rename/move detection: renamed trees are re-transferred and fully copied into the local versions store |
| Y51 | low | confirmed | bisync-plan-state | Empty directories are never synchronized or removed |
| Y52 | low | confirmed | bisync-plan-state | mtime difference in sig_eq can overflow |
| Y53 | low | confirmed | bisync-plan-state | Incremental bootstrap marks the pair bootstrapped before its items are written |
| Y54 | low | confirmed | bisync-plan-state | Versions store reuses remote names verbatim on the local filesystem (Windows) |
| Y55 | critical | confirmed | apply-and-mirror | One failed file or a cancel discards the whole run's progress; the next run turns every file the run transferred into a spurious conflict (or a full Mirror re-copy) |
| Y56 | high | confirmed | apply-and-mirror | Version pruning (Count, Staggered, GFS) treats each per-second backup folder as a snapshot and deletes most recovery copies right after the run |
| Y57 | critical | confirmed | apply-and-mirror | Copies never preserve the source mtime; the stateless Mirror planner then re-copies and re-backs-up every file on every full run of hashless pairs, and remote sources without a change feed always run full |
| Y58 | high | confirmed | apply-and-mirror | Baseline records the post-run re-walk, so edits made during a run are marked as synced and never propagated |
| Y59 | medium | confirmed | apply-and-mirror | Move jobs: a failed source deletion becomes a permanent conflict under the default compare instead of a FinalizeMove retry |
| Y60 | medium | confirmed | apply-and-mirror | Local staged copies are published by rename without fsync while the baseline is fsynced; after power loss/unplug a truncated destination is recorded as synced and, in two-way mode, propagated back |
| Y61 | high | confirmed | apply-and-mirror | Reversible backups always go to the app-data volume, require full downloads of remote files, and are never pruned on incremental runs, error runs or after a prune error |
| Y62 | high | confirmed | apply-and-mirror | Delete safety guard is off by default and there is no empty/unmounted-root check |
| Y63 | medium | confirmed | apply-and-mirror | Duplicate-name repair or dedupe failure aborts the whole sync before any file is applied |
| Y64 | high | confirmed | apply-and-mirror | Real-time triggers are dropped while the job runs or within 60 s of its last start; all jobs share one global slot |
| Y65 | medium | confirmed | apply-and-mirror | Sync writes use plain open_write: FTP/WebDAV/Drive spool every file completely to a local temp file, FTP uploads serialize on the browsing connection |
| Y66 | high | confirmed | apply-and-mirror | FTP destinations: every copied file lists its parent folder about 6-7 times, making large folders quadratic |
| Y67 | high | confirmed | apply-and-mirror | Hard 1,000,000-entry caps make large backups permanently incomplete (one-way mirror) or impossible (jobs) |
| Y68 | high | confirmed | apply-and-mirror | Size/age filters are evaluated per side: a file leaving the window on one side looks deleted, or its counterpart's absence becomes a permanent conflict/drift error |
| Y69 | medium | confirmed | apply-and-mirror | Delete safety stop is reported with the cancel marker 'abgebrochen': job treated as canceled, last_run not updated, scheduled job re-runs (full walk) every 60 s |
| Y70 | medium | confirmed | apply-and-mirror | Directories are never created or removed by the two-way/mirror engine; folder->file replacement fails forever |
| Y71 | medium | confirmed | apply-and-mirror | Leftover staging files after a crash are treated as user files and synced |
| Y72 | medium | confirmed | apply-and-mirror | Staging and conflict suffixes push long file names over the 255 limit; such files can never be synced |
| Y73 | medium | confirmed | apply-and-mirror | Windows: files in use are never backed up and transient sharing violations are never retried |
| Y74 | medium | confirmed | apply-and-mirror | No run-level breaker or 'target refuses' stop in sync apply; retries sleep while holding flow permits |
| Y75 | low | confirmed | apply-and-mirror | Destination backup is made before staging and repeated on every retry and overload repeat |
| Y76 | medium | confirmed | apply-and-mirror | Move finalization reads both files completely and additionally backs up the verified source |
| Y77 | medium | confirmed | apply-and-mirror | Renames/moves in full runs are executed as delete + full re-transfer (with a full versions copy of each 'deleted' file) |
| Y78 | medium | confirmed | apply-and-mirror | Job results under-report and skip failures: capped error count, unreachable endpoints/invalid config never recorded, failed scheduled runs not retried |
| Y79 | medium | confirmed | apply-and-mirror | No cross-process lock on a sync pair: GUI, Android facade and daemon can run the same job concurrently |
| Y80 | medium | confirmed | apply-and-mirror | One-way mirror decides by 'source newer than destination copy time' across two clocks; same-size updates are skipped |
| Y81 | medium | partially_confirmed | apply-and-mirror | Copies drop source permissions: private files become world-readable at the destination and in the versions dir |
| Y82 | medium | confirmed | apply-and-mirror | Type conflicts and Google-native documents cause permanent errors or endless re-copies in the mirror and move paths |
| Y83 | low | confirmed | apply-and-mirror | Bandwidth limit overshoots and does not cover backups, verification reads and spooled uploads |
| Y84 | low | confirmed | apply-and-mirror | KeepBoth conflict copies are impossible on FTP and a failed keep-both creates an additional conflict copy every run |
| Y85 | low | confirmed | apply-and-mirror | 'Sichere Kopien' (atomic_copy) job option has no effect |
| Y86 | high | confirmed | local-platforms | Linux: no-replace publish uses only renameat2(RENAME_NOREPLACE); every NEW file fails on NFS and FUSE mounts |
| Y87 | high | confirmed | local-platforms | Linux FIFOs, sockets and device nodes look like empty regular files; syncing them hangs forever or streams endlessly |
| Y88 | high | confirmed | local-platforms | One unreadable, vanished or undecodable entry aborts the whole bisync snapshot and disables mirror deletions |
| Y89 | high | confirmed | local-platforms | Copies never carry the source mtime: any failed/cancelled bisync run turns every transferred file into a conflict, and Mirror jobs re-copy everything |
| Y90 | medium | partially_confirmed | local-platforms | Sync replaces destination files with stages that were never flushed to stable storage |
| Y91 | high | confirmed | local-platforms | Hard 1,000,000-entry / 128 MiB limits make large backups fail or stay permanently partial |
| Y92 | high | confirmed | local-platforms | Local roots are identified by path text only: an unmounted mount point or another volume at the same path is synced as the real tree (mass deletion) |
| Y93 | high | confirmed | local-platforms | Real-time trigger is a whole-tree polling signature that misses renames/moves, ignores job filters and can starve |
| Y94 | medium | confirmed | local-platforms | mkdir_all refuses links ABOVE the sync root: every copy fails when an ancestor of the target is a symlink, junction or mounted-folder volume |
| Y95 | medium | confirmed | local-platforms | Per-file ancestor validation costs O(depth) metadata round trips per copy (twice per file in the quick mirror) |
| Y96 | medium | confirmed | local-platforms | Stage names append 25-38 characters to the destination name; long names can never be synced or copied |
| Y97 | medium | confirmed | local-platforms | Stages left by a crash or killed process are walked and synced as user files |
| Y98 | medium | confirmed | local-platforms | Windows: names containing ':' from Linux/Android sources are written into NTFS alternate data streams |
| Y99 | medium | partially_confirmed | local-platforms | Windows copy/transfer engine rejects every reparse point, including OneDrive placeholders, WOF-compressed and deduplicated files |
| Y100 | medium | partially_confirmed | local-platforms | Windows: replacing a read-only destination fails, and the copy module itself makes copies read-only |
| Y101 | medium | confirmed | local-platforms | Windows: files in use are not backed up reliably - no VSS, no retry for sharing violations, listing metadata from the lazily updated NTFS directory index |
| Y102 | medium | confirmed | local-platforms | Case-insensitive targets: quick mirror deletes the only backup copy after a case-only rename; case-variant names break bisync permanently |
| Y103 | medium | confirmed | local-platforms | Linux: synced copies drop POSIX permissions - private files become world-readable on the target |
| Y104 | medium | confirmed | local-platforms | Windows FAT32 targets: DST/time-zone shifts make every file look modified |
| Y105 | medium | confirmed | local-platforms | Forced version backups copy every replaced/deleted file into the app data directory on the system volume |
| Y106 | low | confirmed | local-platforms | Sync stages use non-exclusive File::create instead of the exclusive stage writer |
| Y107 | low | confirmed | local-platforms | Windows EFS-encrypted files are written decrypted to backup targets without the OS encryption-loss check |
| Y108 | low | confirmed | local-platforms | Bisync never creates or deletes directories: empty folders are not backed up, removed/renamed folders leave empty skeletons |
| Y109 | low | partially_confirmed | local-platforms | Recycle-bin deletes pass the raw VFS path and silently delete permanently where no recycle bin exists |
| Y110 | low | confirmed | local-platforms | Long unattended desktop runs do not keep the system awake |
| Y111 | low | confirmed | local-platforms | Backups lose file metadata and miss same-size edits with restored mtimes |
| Y112 | low | partially_confirmed | local-platforms | Android: sync's Android/data/obb exclusion matches only registered volume paths, not /sdcard aliases |
| Y113 | critical | confirmed | remote-targets | Mirror backups re-transfer every file on each full run: no backend preserves mtime and Mirror planning compares source and destination mtimes exactly |
| Y114 | critical | confirmed | remote-targets | WebDAV mkdir_all starts at the server root, so sync uploads to Nextcloud/ownCloud-style DAV roots fail (and cost 2 requests per path level per file elsewhere) |
| Y115 | high | confirmed | remote-targets | Plain SFTP never reports NotFound: every SFTP status error becomes ErrorKind::Other |
| Y116 | high | confirmed | remote-targets | FTP stat lists the whole parent directory; sync issues several such listings per file (quadratic in directory size) |
| Y117 | high | confirmed | remote-targets | Sync uploads to FTP, WebDAV and Google Drive spool the entire file to local temp storage first (bisync uses open_write instead of the streaming sized stage) |
| Y118 | high | confirmed | remote-targets | WebDAV directory listings larger than 10 MiB cannot be read (ureq into_string cap) |
| Y119 | high | confirmed | remote-targets | Scheduled runs whose endpoint cannot be resolved are not recorded and are retried every 60 s indefinitely |
| Y120 | medium | confirmed | remote-targets | Sync trees with more than 1,000,000 entries can never be synced (hard walk budget, also in the agent fast path) |
| Y121 | medium | confirmed | remote-targets | SFTP directory listing must finish within one 20 s absolute deadline |
| Y122 | medium | partially_confirmed | remote-targets | Local writes refuse symlinked or junction ancestors above the chosen root, so local destinations and Share hosts under such paths reject every file |
| Y123 | medium | confirmed | remote-targets | Share host re-opens a saved remote connection for every request to an exported 'Verbindungen' entry |
| Y124 | high | confirmed | remote-targets | Google Drive sync rejects local names that are valid on Linux/Android (Windows naming rules applied everywhere) |
| Y125 | medium | confirmed | remote-targets | Leftover staging files from interrupted runs are synced as user files |
| Y126 | medium | confirmed | remote-targets | Share peer directory listings must arrive within 20 s per attempt and fit one 16 MiB frame |
| Y127 | medium | confirmed | remote-targets | FTP listings use LIST: dotfiles can be missing, timestamps have minute/day precision, the path argument can be globbed by the server |
| Y128 | medium | confirmed | remote-targets | FTP resolves host names with Hickory instead of the operating system resolver |
| Y129 | medium | confirmed | remote-targets | Silent fallback from the SSH agent to plain SFTP changes timestamp precision under the same sync identity |
| Y130 | medium | confirmed | remote-targets | A full or over-quota target does not stop the sync, and several endpoints lose the 'disk full' meaning |
| Y131 | medium | confirmed | remote-targets | Local and Share-host writes are acknowledged without fsync |
| Y132 | medium | confirmed | remote-targets | Google Drive sync identity is derived from the OAuth refresh token |
| Y133 | medium | confirmed | remote-targets | 'Keep both' conflict resolution cannot create the conflict copy on FTP or Google Drive |
| Y134 | medium | confirmed | remote-targets | Drive's change feed is never used by sync (Drive mirrors always do full walks), and its handling would mis-resolve paths if enabled |
| Y135 | medium | confirmed | remote-targets | Sync copies cannot resume: an interrupted large transfer restarts from zero on the next run |
| Y136 | medium | partially_confirmed | remote-targets | One unrepresentable file name aborts the whole sync |
| Y137 | low | confirmed | remote-targets | FTP control connection has no keepalive during long transfers |
| Y138 | low | confirmed | remote-targets | No way to trust a self-signed FTPS/WebDAV certificate, and WebDAV is HTTPS-only |
| Y139 | low | confirmed | remote-targets | Google-native files and shortcuts cause repeated work or errors in every Drive sync |
| Y140 | low | confirmed | remote-targets | FTP replace relies on RNTO overwriting, which Windows-hosted FTP servers refuse |
| Y141 | low | confirmed | remote-targets | Share peer sync identity includes the peer's transport node id |
| Y142 | low | confirmed | remote-targets | SFTP servers without posix-rename@openssh.com cannot receive updates of existing files |
| Y143 | low | confirmed | remote-targets | WebDAV claims free content hashes for every server, so the local side hashes the whole tree |
| Y144 | high | confirmed | critic | Android: syncs run without 'All files access'; the filtered storage view turns other apps' files into deletions on the backup side |
| Y145 | high | confirmed | critic | The app's own state directory is not excluded from sync roots: real-time profile/home backups never fire and the version store re-syncs itself |
| Y146 | medium | confirmed | critic | Pre/post-run commands fail with quotes on Windows, run after endpoints are opened, are skipped on automatic pauses and updates, and never run for manual runs |
| Y147 | medium | confirmed | critic | Desktop line merge and 'Beide als getrennte Dateien' lose line endings, overwrite concurrent edits and keep no backup of the replaced versions |
| Y148 | medium | confirmed | critic | Desktop preview 'Nur diese Datei jetzt synchronisieren' applies stale plan actions without planned-state guards and never updates the baseline |
| Y149 | medium | confirmed | critic | 'Reversible' version backups cannot be restored: no browse or restore path, opaque folders on desktop, unreachable app-private storage on Android |
| Y150 | medium | confirmed | critic | Deleting or retargeting a job leaves its baseline and version store forever; a new job with the same endpoints inherits the stale baseline |
| Y151 | medium | confirmed | critic | Unattended runs never alert the user; the log is wiped at 256 KiB and concurrent writers can overwrite each other's results |
| Y152 | medium | confirmed | critic | Linux: sync walks and the real-time signature cross mount points inside the tree; an unmounted nested mount looks like a mass deletion and a hung network mount hangs the run |
| Y153 | low | confirmed | critic | Sync ignore patterns are case-sensitive even where the file system is case-insensitive |
| Y154 | low | confirmed | critic | Target file-system limits are not checked: files over 4 GiB sent to FAT32 sticks or SD cards fail only after streaming 4 GiB, on every run |
| Y155 | low | confirmed | critic | The 'Überprüfen' option only re-checks the destination size; no content verification exists for backups |
| Y156 | low | confirmed | critic | Closing the desktop window or applying an update during a manual sync, merge or single-file apply kills the worker without warning or waiting |

## Y01 On-connect trigger can never fire on Linux, yet the desktop editor offers it

- Schwere: high · Urteil: confirmed · Kategorie: platform · Plattformen: linux
- Stellen: native/src/daemon/os/linux_os/platform.rs:23-25; native/src/daemon/os/shared/schedule.rs:92-98; native/src/daemon/os/shared/run_loop.rs:307-329; native/src/app/core/job_editor_ui.rs:175-183; native/src/app/core/job_editor_ui.rs:217-225; native/src/app/core/settings_background.rs:192-196; native/src/mobile/os/shared/domains/sync_jobs.rs:53-56

**Beschreibung.** The Linux platform adapter returns no removable drives, so current_drives() is always the empty set and enqueue_connect_jobs never sees an arrival. The desktop job editor lists Trigger::ALL on every OS (only the Android facade filters OnConnect out), so a Linux user can save an 'external drive backup on connect' job that is accepted, displayed as 'bei USB/Gerät' and never runs - no warning, no fallback, no log line.

**Fehlerszenario.** Linux desktop, job /home/u/Documents -> /run/media/u/BACKUP/docs, trigger 'Bei Geräte-/USB-Anschluss', match 'BACKUP*'. The user plugs the drive in every evening; the job never starts, the list keeps showing 'zuletzt: nie', the daemon log is silent.

**Richtung.** Implement Linux volume-arrival detection (UDisks2 D-Bus InterfacesAdded/PropertiesChanged for Filesystem.MountPoints, or poll /proc/self/mountinfo with /sys/block/*/removable and /dev/disk/by-label|by-uuid), expose label/UUID/serial as descriptor and treat USB-attached fixed disks (ID_BUS=usb) as eligible. Until then hide/disable the trigger on Linux like Android and flag existing jobs in the UI.

**Gegenprüfung.** The code shows this as described. daemon/os/linux_os/platform.rs:23-25 returns Vec::new(), and daemon/mod.rs:129-131 selects that adapter for target_os=linux. current_drives() (schedule.rs:93-98) is fed only from it, so run_loop.rs:127 seeds an empty set and enqueue_connect_jobs (run_loop.rs:314-316) never sees a new drive. No other path runs OnConnect jobs: the startup pass filters OnStartup (run_loop.rs:368-371), due() returns false for event triggers (syncjobs/core/schedule.rs:61), and catch-up excludes the trigger (catch_up.rs:97). The desktop editor offers Trigger::ALL without a cfg (job_editor_ui.rs:179-181; types.rs:51-58), and neither build_sync_job nor validate() rejects OnConnect (editor.rs:181-301; validation.rs:69-94). Only the Android facade filters it (mobile sync_jobs.rs:53-56). The settings hint 'Echtzeit & USB-Anschluss brauchen lokale Pfade' (settings_background.rs:192-196) even suggests that USB triggers work. Severity: this is a deterministic silent non-run of one trigger type on one platform. It is visible as 'zuletzt: nie' (menus_sync_jobs.rs:157-161) and causes no data loss or corruption, so high fits better than critical.

## Y02 Real-time: a settled change is discarded when the job is running or was admitted less than 60 s ago

- Schwere: high · Urteil: confirmed · Kategorie: bug · Plattformen: windows, linux, android
- Stellen: native/src/daemon/os/shared/run_loop.rs:284-293; native/src/daemon/os/shared/run_loop.rs:425-436; native/src/daemon/os/shared/job_supervisor.rs:9; native/src/daemon/os/shared/job_supervisor.rs:66-79

**Beschreibung.** When the signature has been stable for the debounce period the loop calls enqueue_job and then unconditionally removes rt_dirty_since. enqueue_job silently ignores EnqueueStatus::AlreadyScheduled (job queued or currently running) and RecentlyAttempted (< MIN_RETRY_INTERVAL = 60 s since the last admission, measured from admission, not completion). The pending change is forgotten, and because rt_sig already holds the post-change signature nothing re-triggers until another change happens.

**Fehlerszenario.** Defaults (tick 15 s, debounce 10 s): save at t=5, detected t=15, admitted t=30, run ends t=32; second save at t=40, detected t=45, settled t=60 -> RecentlyAttempted (30 s < 60 s) -> dirty mark removed -> the second save is never synced until some later edit. Same when a change lands during a long run whose target is remote (AlreadyScheduled).

**Richtung.** Return the enqueue status to the caller and keep the dirty mark until Started/Queued; or give JobSupervisor a per-job 'rerun requested' flag that re-queues once when the active run of that id finishes; measure the cooldown from completion and never let it consume real-time triggers.

**Gegenprüfung.** run_loop.rs:285-291 calls enqueue_job and then removes rt_dirty_since unconditionally. enqueue_job drops AlreadyScheduled and RecentlyAttempted without any action (run_loop.rs:433). The supervisor sets last_admitted at admission (job_supervisor.rs:81), so RecentlyAttempted is returned for 60 s counted from admission, not from completion (job_supervisor.rs:9, 73-79). AlreadyScheduled lasts until poll reaps the finished thread (job_supervisor.rs:67-69, 95-107). rt_sig already holds the post-change signature (run_loop.rs:296-297), so later ticks take the 'unchanged, no dirty mark' branch and nothing re-triggers. The reported timing reproduces: with a local target, the run's own writes and the second save are detected together and settle under 60 s after admission, so the change is dropped. One nuance: AlreadyScheduled for a job that is only queued is harmless, because the queued run will still see the change. The loss is real for RecentlyAttempted and for a job that is already running. Desktop has no recovery path. On Android, periodic catch-up re-admits every RealTime job (catch_up.rs:96), so there the change is picked up in the next worker window.

## Y03 Real-time signature misses renames/moves, folder-only changes and same-size edits

- Schwere: high · Urteil: confirmed · Kategorie: bug · Plattformen: windows, linux, android
- Stellen: native/src/daemon/os/shared/schedule.rs:13-55; native/src/daemon/os/shared/run_loop.rs:273-283

**Beschreibung.** tree_sig reduces a tree to (file count, newest file mtime, sum of file sizes); directories are only pushed on the stack and contribute nothing. A rename or move inside the watched tree keeps all three values (rename does not change a file's mtime on NTFS/ext4); creating/removing empty folders changes nothing; delete+add of an equal-size file with an older mtime (timestamp-preserving copy/restore) changes nothing; one file with a future mtime (camera clock, archive from a skewed host) pins 'newest', after which same-size in-place edits (databases, containers, VM images) are invisible.

**Fehlerszenario.** Real-time mirror C:/Users/u/Documents -> NAS. The user renames folder 'Projekt' to 'Projekt-2026' or moves 500 photos into a subfolder: no run is triggered and the backup keeps only the old names until an unrelated edit happens, possibly days later.

**Richtung.** Use OS change notifications (the notify crate is already a dependency: ReadDirectoryChangesW or USN journal on Windows, inotify/fanotify on Linux/Android) with a periodic verification walk as safety net; if polling stays, add directory mtimes/entry counts (a parent mtime changes on every add/remove/rename) or a rolling hash over (relative path, size, mtime).

**Gegenprüfung.** tree_sig (schedule.rs:15-55) adds up only file count, the newest file mtime and total size. Directories are only pushed onto the stack (schedule.rs:38-39), and names are never hashed. The following therefore leave all three values unchanged:
- a rename or move inside the tree (rename does not update mtime on NTFS, ext4 or f2fs)
- creating or removing empty folders
- deleting a file and adding one of equal size that is not newer than the current maximum
- same-size in-place edits after a file with a future mtime has pinned 'newest'
The doc comment at schedule.rs:13-14 claims more than the code delivers. No data is lost, because the next run applies the rename. However, real-time never fires for these changes until an unrelated content change happens, which can be days later for archive folders. High is defensible for the real-time backup feature.

## Y04 Real-time debounce restarts on every change with no maximum wait, so continuous writers starve the job forever

- Schwere: high · Urteil: confirmed · Kategorie: bug · Plattformen: windows, linux, android
- Stellen: native/src/daemon/os/shared/run_loop.rs:284-298

**Beschreibung.** Any signature change re-arms rt_dirty_since to now; a job only runs after a tick sees no change for rt_debounce_secs. There is no upper bound on the delay since the first unsynced change, so a tree containing any file written more often than once per tick never settles (the job's own writes into a local target also re-arm it during a run).

**Fehlerszenario.** Real-time backup of Documents containing an open Outlook .pst/.ost, a Thunderbird profile, an IDE index or a log written every few seconds (or an excluded cache, see the filter finding): every 15-s tick sees a new signature, the settle timer is reset indefinitely, and none of the other edited documents are synced for the whole working day.

**Richtung.** Keep the first-dirty timestamp and force a run once now - first_dirty exceeds a maximum latency (e.g. max(5 x debounce, 5 min) or a user setting); combine with filter-aware detection so excluded files cannot keep the job unsettled.

**Gegenprüfung.** run_loop.rs:294-298 restarts rt_dirty_since at now on every signature change. The only path to a run is the 'unchanged' branch (run_loop.rs:285-292) after rt_debounce_secs. There is no cap on the time since the first unsynced change. Any entry under either root whose mtime or size changes between consecutive ticks keeps the job from ever running. tree_sig also ignores the job's filters (see #21), so excluded churn counts too, and a long run writing into a local target re-arms the timer as well. This is realistic for folders that contain PST/OST files, IDE metadata, logs or downloads in progress, and the starvation is complete and silent.

## Y05 Real-time state is not persisted: edits made while the worker was down, or not yet settled before shutdown/restart, are never synced

- Schwere: high · Urteil: confirmed · Kategorie: reliability · Plattformen: windows, linux, android
- Stellen: native/src/daemon/os/shared/run_loop.rs:125-126; native/src/daemon/os/shared/run_loop.rs:138-139; native/src/daemon/os/shared/run_loop.rs:299-302; native/src/daemon/os/shared/live.rs:42-49; native/src/mobile/os/shared/domains/background.rs:143-152; android/app/src/main/java/app/smartexplorer/android/work/SyncWorker.kt:41-45

**Beschreibung.** rt_sig/rt_dirty_since exist only in memory. At every worker start (logon, update handoff, GUI-triggered replacement, crash restart, Android process restart) and on every re-enable, the first sighting just stores the current signature as baseline and does not run. Desktop has no catch-up path at all (request_catch_up is called only by the Android facade); in Android persistent mode the periodic worker is skipped while the service runs, so there is no catch-up either.

**Fehlerszenario.** User saves a report and shuts the laptop down 20 s later: the change was detected but debounce + next tick had not elapsed. At the next logon the modified tree becomes the baseline; the report is not backed up until something else in the folder changes. Same for every edit made while the worker was stopped and for changes pending during an auto-update handoff.

**Richtung.** On worker start and on enable, enqueue each enabled real-time job once (cheap with an incremental engine) or persist per-job last-synced signature/cursor and compare (Windows: USN journal cursor gives gap-free change detection across restarts); persist pending dirty marks before stop/handoff.

**Gegenprüfung.** rt_sig and rt_dirty_since are new HashMaps at every worker start (run_loop.rs:125-126) and are cleared on every enable toggle (run_loop.rs:138-139). A first sighting only stores the baseline (run_loop.rs:299-302), and nothing is persisted. request_catch_up has a single caller, the mobile facade (mobile/.../background.rs:151; live.rs:43-49), so desktop has no catch-up. The startup pass enqueues only OnStartup jobs (run_loop.rs:368-371). In Android persistent mode, SyncWorker returns early while the service runs (SyncWorker.kt:45). A pause does not lose state, because rt_sig is kept while may_schedule is false. Pending or unsynced changes are lost until the next change in these cases: logon after a quick shutdown, crash restart, GUI-triggered replacement, update handoff, and disable/enable. Android periodic mode is partly mitigated by catch-up (catch_up.rs:96).

## Y06 Real-time polling walks every local root on the scheduler thread each tick; silent 1M-entry cap; NAS shares walked over the network

- Schwere: high · Urteil: confirmed · Kategorie: performance · Plattformen: windows, linux, android
- Stellen: native/src/daemon/os/shared/schedule.rs:15-55; native/src/daemon/os/shared/schedule.rs:59-69; native/src/daemon/os/shared/run_loop.rs:183-191; native/src/daemon/os/shared/run_loop.rs:265-276; native/src/daemon/os/shared/run_loop.rs:203; native/src/daemon/os/shared/state.rs:224-229; native/src/daemon/os/shared/ipc_host.rs:143-152; native/src/app/core/init.rs:43-44; native/src/daemon/os/shared/ipc_client.rs:207-213; native/src/daemon/os/shared/ipc_client.rs:277-291

**Beschreibung.** Every tick (default 15 s) each enabled real-time job's local source AND target (also for one-way jobs) are walked recursively with one std::fs::symlink_metadata per entry, synchronously in the loop that also enqueues timer jobs, services catch-up, runs share_host.tick (mounts, LAN, Share event drain, reload) and writes the heartbeat. The budget is 1,000,000 entries per root; beyond it the partial signature is returned and, because traversal order is stable, the rest of the tree is never observed (no log). Any existing non-URL path counts as local, so UNC/mapped NAS shares and Linux network/FUSE mounts are walked over the network (no filesystem-boundary check). On Windows std's lstat opens a handle per path (assumption about current std internals) instead of reusing FindNextFileW data via DirEntry::metadata. When a walk exceeds cadence*2+30 s the heartbeat goes stale: the settings show 'Dienst startet beim nächsten Anmelden' and the next GUI start calls request_daemon_replacement(), whose handoff stops the busy worker (cancelling its running backup) and drops real-time state.

**Fehlerszenario.** Real-time job C:/Users/u (about 1.5 M entries incl. AppData) -> UNC share //nas/backup: each tick stats 1 M local entries (changes in the remaining part are never seen) plus the whole NAS tree over SMB (estimate: minutes per tick at 0.5-1 ms per remote stat); scheduling, catch-up and Share maintenance stall meanwhile, disks never idle, and opening the GUI replaces the seemingly dead worker mid-backup.

**Richtung.** Move change detection to a dedicated watcher thread using OS notifications (notify = 6 is already in native/Cargo.toml:23 and used by the GUI in app/os/windows/watchers.rs; Windows ReadDirectoryChangesW or USN journal, Linux/Android inotify with fallback when max_user_watches is exceeded, fanotify where permitted); watch only the side(s) relevant to the job direction; never walk network shares for change detection (SMB2 CHANGE_NOTIFY or slow polling); if polling remains, use DirEntry metadata, stop at mount boundaries, report the budget cut, and decouple heartbeat/Share tick from signature work.

**Gegenprüfung.** Source and target are both walked for every direction (run_loop.rs:265-268). tree_sig runs synchronously on the scheduler thread, before connect jobs, catch-up and the heartbeat (run_loop.rs:184-203).

Walk behavior:
- The 1M budget applies per call (schedule.rs:20) and running out returns the partial signature silently (schedule.rs:27-29). Because the walk is depth-first over read_dir in stable order, the remainder of the tree is never observed.
- local_root accepts any existing path without '://' (schedule.rs:59-69). UNC endpoints are local (resolution.rs:10-20), and so are mapped drives and network/FUSE mounts.
- Each entry costs std::fs::symlink_metadata(e.path()) (schedule.rs:31) instead of the free DirEntry::metadata on Windows.

Heartbeat cascade on desktop:
- Only this thread writes the heartbeat (state.rs:207-209; run_loop.rs:85, 203, 246).
- is_running() requires an age below cadence*2+30 s (state.rs:224-229). Otherwise settings show 'Dienst startet beim nächsten Anmelden' (settings_background.rs:73-77).
- GUI start then calls request_daemon_replacement (init.rs:43-44), which writes handoff:<gen> (ipc_client.rs:277-291).
- The old worker sees stop_requested (handoff.rs:158-164) and stop_daemon cancels the running job (run_loop.rs:438-448).

Additional consequence: the replacement waits only DAEMON_HANDOFF_TIMEOUT = 300 s (run_loop.rs:28; handoff.rs:131-133). If the walk outlasts that, the replacement exits while the handoff control remains, so the old worker exits too. No worker then runs until the next GUI start or logon. Walk durations are estimates, but 100k SMB entries at ~1 ms per open already exceed 60 s.

## Y07 Real-time remote probe connects every tick before checking change support; remote sides are mostly unwatched; remote-only real-time jobs never run

- Schwere: high · Urteil: confirmed · Kategorie: bug · Plattformen: windows, linux, android
- Stellen: native/src/daemon/os/shared/schedule.rs:71-90; native/src/daemon/os/shared/run_loop.rs:269-272; native/src/vfs/core/core.rs:360-375; native/src/gdrive/core/backend.rs:337-347; native/src/connect/os/shared/resolution.rs:7-34; native/src/connect/os/shared/connector.rs:374-420

**Beschreibung.** remote_change_token runs every tick for each real-time Mirror job with a remote data source, calls resolve_endpoint (SFTP/FTP/WebDAV/SMB connect + authentication, UNC connection with stored credentials, Share peer open) on the scheduler thread, and only afterwards checks supports_changes(), which is true only for Google Drive. For Drive the token is the account-wide start page token, so any change anywhere in the Drive fires the job, and transient errors flip the signature ('' vs token) causing extra runs. Non-mirror one-way jobs and two-way jobs never watch their remote side; jobs whose both sides are remote are skipped as 'nothing watchable' forever on desktop and in Android persistent mode (only the Android periodic catch-up runs them).

**Fehlerszenario.** Real-time mirror sftp://nas/photos -> D:/Backup: a new SSH login every 15 s while remote changes still never trigger a run; with the NAS offline every tick blocks the loop for the connect timeout; with a changed password a failed login every 15 s can trip fail2ban or an AD/SMB account-lockout policy. A job sftp://a -> webdav://b set to 'Echtzeit' never runs at all.

**Richtung.** Decide capability from the endpoint scheme before connecting and cache one backend per job; for Drive use changes filtered to the job root instead of the global token; add real remote change sources (SMB2 CHANGE_NOTIFY in the in-house SMB client, change push from Share peers' local watchers) and otherwise a separate, visible low-frequency remote poll; reject or convert real-time jobs without any watchable side (with a clear message) instead of silently never running them.

**Gegenprüfung.** remote_change_token (schedule.rs:73-90) is called every tick for each active RealTime job (run_loop.rs:269). For Mirror jobs with a non-local data source, it calls resolve_endpoint first (schedule.rs:85). That connects and authenticates eagerly (resolution.rs:7-34; connector.rs:374-420, do_connect at 385/413), and SFTP with the agent option even redeploys the agent (connector.rs:158). Only afterwards does it check supports_changes (schedule.rs:86-88), which is true only for Google Drive (vfs core.rs:359-362; gdrive backend.rs:337-339; cache.rs only delegates). The Drive cursor is changes/startPageToken without a driveId (gdrive/core/changes.rs:9-18), so it is account-wide. Errors become None, which formats as '' and flips the signature (schedule.rs:85, 89). Remote sides of non-Mirror and two-way jobs are never watched (schedule.rs:74-81). Jobs with both sides remote hit 'nothing watchable' forever (run_loop.rs:270-272); only Android catch-up runs them (catch_up.rs:96). Nuances: an existing UNC path returns early at schedule.rs:82-84, so the per-tick UNC credential connect happens only while the share is unreachable. The 'local side only' limitation is hinted in the UI (job_editor_ui.rs:213; settings_background.rs:192-196) but is not enforced.

## Y08 Windows: data reparse points (cloud placeholders, WOF, dedup) are skipped by the real-time signature, leaving OneDrive/Files-On-Demand folders effectively unwatched

- Schwere: high · Urteil: confirmed · Kategorie: platform · Plattformen: windows
- Stellen: native/src/daemon/os/windows/platform.rs:129-134; native/src/daemon/os/shared/schedule.rs:31-37; native/src/local_access/os/windows/directory.rs:355-368; native/src/local_access/os/windows/sync_link_task_tests.rs:4-13

**Beschreibung.** The daemon's metadata_is_link_like returns true for every FILE_ATTRIBUTE_REPARSE_POINT entry and tree_sig skips such files and whole subtrees. The sync engine classifies data tags (IO_REPARSE_TAG_CLOUD..CLOUD_F, WOF) as ordinary entries and syncs them, and AGENTS.md requires distinguishing data reparse points from redirecting links in every walk. Files and folders inside a Cloud Files sync root carry cloud reparse tags (assumption: also when hydrated/pinned), so their edits, additions and renames do not change the signature.

**Fehlerszenario.** Windows 11 with OneDrive Known Folder Move: Documents is C:/Users/u/OneDrive/Documents. A real-time job Documents -> E:/Backup never fires for edits or renames of existing documents, while 'Jetzt' would sync them - the gap is only noticed when the backup is needed.

**Richtung.** Reuse local_access's tag-based classification (name-surrogate tags only) in the daemon walk; better, use ReadDirectoryChangesW, which reports placeholder changes without opening files.

**Gegenprüfung.** The code part is certain. daemon/os/windows/platform.rs:129-134 treats every FILE_ATTRIBUTE_REPARSE_POINT entry as link-like, and tree_sig skips such files and whole subtrees (schedule.rs:35-37). The sync engine's local classification treats data tags (CLOUD..CLOUD_F, WOF) as ordinary entries (local_access/os/windows/directory.rs:355-368; tests in sync_link_task_tests.rs:4-13). AGENTS.md requires that same distinction in every walk, so change detection and sync disagree. WOF-compressed and deduplicated files are definitely affected. The OneDrive Known Folder Move scenario additionally depends on whether the Cloud Files filter exposes the reparse attribute of hydrated or pinned placeholders to the unpackaged daemon process (placeholder compatibility mode). To my knowledge unpackaged Win32 processes see it, but this was not verified at runtime and is listed as unresolved.

## Y09 Failures before the sync starts are silent: no result, no last run, retried every minute forever

- Schwere: high · Urteil: confirmed · Kategorie: reliability · Plattformen: windows, linux, android
- Stellen: native/src/daemon/os/shared/job.rs:16-42; native/src/daemon/os/shared/job_supervisor.rs:9; native/src/daemon/os/shared/job_supervisor.rs:70-79; native/src/syncjobs/core/schedule.rs:41-46; native/src/daemon/os/shared/state.rs:247-250; native/src/app/core/menus_sync_jobs.rs:174-188; native/src/mobile/os/shared/domains/job_json.rs:57

**Beschreibung.** If the persisted configuration is invalid or resolve_endpoint fails (NAS/host unreachable, saved connection removed, credential read error, Share peer offline), run_one only logs and returns: no JobResult and no mark_run are persisted. The UI keeps showing the previous result (e.g. green 'ok') and last-run date; the interval job stays due and is re-admitted every 60 s indefinitely, each attempt possibly holding the single job slot for a connect timeout. The log grows by about a line per minute and log() truncates the whole file to empty at 256 KiB, erasing the history that would explain the gap.

**Fehlerszenario.** Laptop away from the home NAS for two weeks: the daily backup never runs, the job list still shows '● ok' from two weeks ago, the worker log consists of 'skip ...: target ...' lines or has been wiped, and the user believes the backup is current.

**Richtung.** Record a failure result ('Quelle/Ziel nicht erreichbar', timestamp, consecutive-failure count) for every failed attempt; exponential backoff per job; notify after N consecutive failures or when the last success is older than the schedule promises; rotate the log instead of truncating it.

**Gegenprüfung.** run_one only logs and returns for an invalid configuration (job.rs:16-25) and for source or target resolution errors (job.rs:26-32, 36-42). persist_attempt (job.rs:185-200) is never reached, so neither mark_run nor record_result runs. The job list keeps the previous result colour and 'zuletzt' date (menus_sync_jobs.rs:157-188; job_json.rs:49, 57). An interval job stays due (syncjobs/core/schedule.rs:41-46). The supervisor re-admits it after MIN_RETRY_INTERVAL = 60 s (job_supervisor.rs:9, 73-79), and each attempt holds the single job slot for the duration of the connect attempt. log() empties the file once it passes 256 KiB (state.rs:11, 247-250); at one line per minute that is roughly every 2-3 days. The only mitigation is for removed connections, which the desktop list marks '⚠ verwaist' (menus_sync_jobs.rs:92-95). On Android, such a job even counts as executed in the catch-up summary (catch_up.rs:399-411).

## Y10 Windows on-connect only recognises DRIVE_REMOVABLE volumes: USB hard disks/SSDs never trigger, card-reader slots misfire

- Schwere: high · Urteil: confirmed · Kategorie: platform · Plattformen: windows
- Stellen: native/src/daemon/os/windows/platform.rs:264-306; native/src/daemon/os/shared/schedule.rs:100-110; native/src/daemon/os/shared/run_loop.rs:307-329

**Beschreibung.** removable() enumerates drive letters and keeps only GetDriveTypeW == DRIVE_REMOVABLE. External USB hard disks and most USB SSDs report DRIVE_FIXED and are never in the set; volumes without a letter are invisible. A card-reader slot without media stays listed (GetVolumeInformationW fails -> empty label, serial '00000000'), so removing the card creates a new descriptor and fires jobs with an empty (any removable) or letter match. GetVolumeInformationW is called on media-less drives without SetThreadErrorMode/SetErrorMode (assumption: some devices can raise the 'no disk' hard-error dialog).

**Fehlerszenario.** Job with match 'BACKUP*' for a USB 3 hard disk labelled BACKUP: plugging it in never starts the job. With the default empty match and a built-in SD reader at E:, pulling the SD card starts the job against a missing path.

**Richtung.** Detect volume arrival via CM_Register_Notification/RegisterDeviceNotification (GUID_DEVINTERFACE_VOLUME) or FindFirstVolumeW + GetVolumePathNamesForVolumeNameW; classify by bus type (IOCTL_STORAGE_QUERY_PROPERTY: USB/SD/1394/Thunderbolt) instead of DRIVE_REMOVABLE; ignore descriptors whose volume information cannot be read; wrap probes in SetThreadErrorMode(SEM_FAILCRITICALERRORS).

**Gegenprüfung.** removable() keeps only drive letters whose GetDriveTypeW result is DRIVE_REMOVABLE (windows/platform.rs:267-280). USB hard disks, most USB SSDs (both DRIVE_FIXED) and volumes without a letter therefore never reach current_drives (schedule.rs:93-98). When GetVolumeInformationW fails, the label falls back to '' and the serial stays 0, formatted as '00000000' (windows/platform.rs:283-304). An empty card-reader slot is therefore listed as 'E:||00000000'. When a card is removed, that descriptor reappears as new in the set difference (run_loop.rs:316) and fires jobs with an empty pattern (schedule.rs:105-107) or a letter match. rg finds no SetErrorMode or SetThreadErrorMode in native/src. Whether the 'no disk' hard-error dialog actually appears on current Windows was not verified, as the finding itself states.

## Y11 Android 'Dauerbetrieb': no wake lock for scheduling or job runs, scheduler time stops in suspend, WorkManager fallback skipped

- Schwere: high · Urteil: confirmed · Kategorie: platform · Plattformen: android
- Stellen: android/app/src/main/java/app/smartexplorer/android/service/BackgroundService.kt:30-38; android/app/src/main/java/app/smartexplorer/android/service/BackgroundService.kt:83-99; android/app/src/main/java/app/smartexplorer/android/system/WakeKeeper.kt:8-16; android/app/src/main/java/app/smartexplorer/android/system/KeepAliveAlarm.kt:16-48; android/app/src/main/java/app/smartexplorer/android/work/SyncWorker.kt:41-45; native/src/daemon/os/shared/run_loop.rs:205-249; native/src/daemon/os/shared/job.rs:10-129; android/app/src/main/java/app/smartexplorer/android/ui/sync/JobEditorScreen.kt:217; android/app/src/main/java/app/smartexplorer/android/ui/settings/BackgroundSettings.kt:209

**Beschreibung.** The specialUse foreground service keeps the process alive but not the CPU; partial wake locks exist only for Share (WakeKeeper) and the keep-alive delivery (at most WAIT_MS 20 s + 5 s). With the screen off the device suspends; std::thread::sleep is nanosleep on CLOCK_MONOTONIC, which does not advance in suspend, so the 15-s tick only consumes awake time and schedules are evaluated in short wake windows (inexact setAndAllowWhileIdle every >= 10 min). Running daemon jobs get no wake lock (also not inside the dataSync task service started by TaskKeeper), so transfers stall while suspended and remote sessions may time out. SyncWorker returns immediately while the service runs, so the only wake-lock-holding path (WorkManager) is unused in this mode. The UI promises 'pünktlich nur im Dauerbetrieb' and that the notification 'hält Jobs ... wach'.

**Fehlerszenario.** Dauerbetrieb, calendar job daily 02:00 without catch-up, phone idle overnight: the first evaluation after 02:00 happens minutes later in a keep-alive window, now - occurrence > 120 s, so the job is skipped every night; with catch-up it starts late and a 20 GB photo backup to a NAS crawls only during wake windows and fails with network timeouts.

**Richtung.** Emit a renewable wake event for the duration of every daemon job so WakeKeeper holds a partial wake lock; schedule an exact/while-idle alarm or a OneTimeWorkRequest at the next due calendar/interval time that requests a catch-up; let SyncWorker also run in persistent mode when work is due; correct the UI texts.

**Gegenprüfung.** BackgroundService only calls startForeground and holds no wake lock (BackgroundService.kt:83-99). WakeKeeper is used only for Share activity holds and keep-alive delivery (WakeKeeper.kt:8-16; Core.kt:192; KeepAliveAlarm.kt:29-33, 41, 60; KeepAliveProbe WAIT_MS = 20 s). TaskKeeper and TaskForegroundService take no wake lock. run_loop, job, job_supervisor, live and catch_up contain no wake handling, and the loop sleeps with std::thread::sleep (run_loop.rs:214), which does not advance during suspend. SyncWorker returns immediately in persistent mode while the service runs (SyncWorker.kt:45), so the WorkManager wake-lock path is unused in this mode. The manifest has no exact-alarm permission. The UI promises that the notification keeps jobs awake (BackgroundSettings.kt:209) and that calendar runs are punctual only in Dauerbetrieb (JobEditorScreen.kt:217). A calendar job without catch-up is skipped when the first evaluation comes more than 120 s after the occurrence (syncjobs/core/schedule.rs:54-56).

## Y12 Android 'Periodisch' (default mode): jobs the daemon's own schedule starts inside a worker window are not bound to that window

- Schwere: high · Urteil: confirmed · Kategorie: reliability · Plattformen: android
- Stellen: native/src/daemon/os/shared/boot_marker.rs:23-41 (marker written at :37 before run_loop.rs:368-376 enqueues the startup jobs); native/src/daemon/os/shared/run_loop.rs:230-235 (defer change breaks the sleep slice before service_catch_up at :239); android/app/src/main/java/app/smartexplorer/android/service/BackgroundController.kt:184-191 (Wi-Fi-only constraint)

**Beschreibung.** During a worker run HostMonitor clears deferScheduling, so the daemon's regular schedule enqueues due interval/calendar jobs first (schedule-owned), plus on-startup jobs and settled real-time jobs. The catch-up run only awaits the due jobs it also selected and never selects OnStartup. When WorkManager stops the worker (constraint lost: 'Nur im WLAN'/'Nur beim Laden', the ~10-min limit, user cancel), SyncWorker cancels the catch-up task, which cancels only run-owned ids; schedule-owned jobs keep running after doWork returned - outside the JobScheduler wake lock and without a foreground service, so the cached-app freezer can freeze them mid-transfer - and in violation of the user's constraints. OnStartup jobs are never awaited, so the worker can finish while they run; their boot marker was claimed when they were enqueued, so a frozen/killed startup job is not repeated in this boot.

**Fehlerszenario.** Default mode (periodic, 60 min) with 'Nur im WLAN': an hourly NAS backup starts in a worker window; the user leaves home, Wi-Fi drops, WorkManager stops the worker, nothing is cancelled, the job continues over mobile data until the process is frozen, then fails with network errors, is recorded as run (see the 'failed runs count as done' finding) and is not retried until the next interval.

**Richtung.** While the host defers scheduling, let only the catch-up run admit work (keep the schedule deferred inside worker windows) or tag everything enqueued during a window as owned by it; include OnStartup and real-time jobs so the worker awaits them; cancel all window-owned jobs when the worker is stopped; claim the boot marker only after the startup jobs completed.

**Gegenprüfung.** During a worker run deferScheduling is false (HostMonitor.kt:139-148, 171-173). The defer change breaks the sleep slice before service_catch_up (run_loop.rs:230-235 vs 239). The daemon's own pass therefore enqueues the deferred startup pass and due timer jobs first (run_loop.rs:171-182), and catch-up gets AlreadyScheduled and only waits for them (catch_up.rs:352-375, comment at 356-358). Catch-up never selects OnStartup (catch_up.rs:97). Its cancel only cancels ids it owns and stops waiting for the schedule's jobs (catch_up.rs:146-153, 296-305). SyncWorker cancels only its own task when WorkManager stops it (SyncWorker.kt:78-87) and then closes the gate (SyncWorker.kt:47-55). With the defaults (periodic, 60 min, Wi-Fi only: AppPrefs.kt:28-30, UNMETERED constraint at BackgroundController.kt:184-191; metered auto-pause off: state.rs:91-96), the job continues over mobile data. Without the Share service the process can also be cached or frozen. The boot marker is written before the startup jobs are enqueued (boot_marker.rs:37 via run_loop.rs:365). The cited boot_marker.rs:127-145 and :141 do not exist, since the file has 57 lines.

## Y13 Cancelled runs of event-triggered jobs are lost; the startup pass is skipped when the worker starts paused

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: windows, linux, android
- Stellen: native/src/daemon/os/shared/job.rs:128; native/src/daemon/os/shared/run_loop.rs:153-159; native/src/daemon/os/shared/run_loop.rs:223-229; native/src/daemon/os/shared/run_loop.rs:284-293; native/src/daemon/os/shared/run_loop.rs:314-328; native/src/daemon/os/shared/run_loop.rs:116-122; native/src/daemon/os/shared/run_loop.rs:333-346; native/src/daemon/os/shared/state.rs:162-170; native/src/syncjobs/core/schedule.rs:53-57

**Beschreibung.** Manual pause, auto-pause (battery saver/metered) and stop/handoff cancel the running job via cancel_and_join. Interval jobs and calendar jobs with catch-up stay due because mark_run is skipped on cancel, but the trigger of real-time jobs (dirty mark removed at enqueue), on-connect jobs (drive already in seen_drives), on-startup jobs (pass consumed) and calendar jobs without catch-up (120-s grace) is gone, so the interrupted run is never repeated. A worker that starts while paused, or while a control file is unreadable, skips the startup pass for its whole lifetime (on desktop: until the next logon).

**Fehlerszenario.** Laptop with auto-pause 'Energiesparmodus': battery saver switches on at 20 % while the on-connect backup to a USB stick runs -> cancelled; after charging nothing restarts it although the stick stays plugged in. Logon during an active 2-h manual pause -> the 'Beim Start' backup is skipped for the whole session.

**Richtung.** Persist a per-job 'pending trigger' that survives cancellation and pause and is re-queued on resume; defer instead of skipping the startup pass while paused or blocked.

**Gegenprüfung.** Pause, auto-pause and stop cancel the active job (run_loop.rs:153-159, 223-229, 438-448), and a cancelled run skips mark_run (job.rs:78-79, 128). Only Interval jobs and Calendar jobs with catch-up therefore stay due. The other triggers lose the run:
- RealTime: the dirty mark was removed at enqueue (run_loop.rs:288-291) and rt_sig stays unchanged.
- OnConnect: seen_drives already holds the drive and is not refreshed while paused (run_loop.rs:193-199, 327).
- OnStartup: the pass is consumed (run_loop.rs:173, 368-376).
- Calendar without catch-up: the 120 s grace has passed (syncjobs/core/schedule.rs:54-56).
A worker that starts while paused, or with an unreadable control file, gets StartupPass::Skip (state.rs:162-170, 175-204), so startup_deferred stays false (run_loop.rs:344). It is not retried until the next worker start or enable toggle (run_loop.rs:141-144).

## Y14 On-connect arrivals are consumed without a run (drive present at start, outside active hours, job busy or cooling down); matched volume is not tied to the synced path

- Schwere: medium · Urteil: confirmed · Kategorie: gap · Plattformen: windows
- Stellen: native/src/daemon/os/shared/run_loop.rs:127; native/src/daemon/os/shared/run_loop.rs:140; native/src/daemon/os/shared/run_loop.rs:307-329; native/src/daemon/os/shared/run_loop.rs:433; native/src/daemon/os/shared/schedule.rs:100-110; native/src/syncjobs/core/types.rs:91-92

**Beschreibung.** seen_drives is initialised with the drives present at worker start, so a backup drive already connected at boot/logon/worker restart never triggers. On arrival the drive is stored in seen_drives regardless of outcome: jobs outside their active window are filtered out and AlreadyScheduled/RecentlyAttempted are ignored, so the arrival is consumed. The descriptor match (label/serial/letter, empty = any removable) only decides whether to run; the job still syncs its fixed letter path, so any stick can start a job aimed at another drive and a drive that received a different letter is not used.

**Fehlerszenario.** Backup disk stays plugged in and the PC is booted in the morning: the on-connect job never runs that day. Rotation: disk B plugged in while disk A's run is still active -> AlreadyScheduled -> B is never backed up until re-plugged. Active hours 08:00-18:00, disk plugged at 07:50 -> no run all day.

**Richtung.** Treat 'matched drive present and job not run since it appeared' as pending (also at start); keep pending arrivals until the job actually ran inside its window; resolve the job endpoint through the matched volume (volume GUID path or label lookup) and verify the target root lies on that volume before running.

**Gegenprüfung.** This applies only on Windows, since the other platforms report no drives. seen_drives is seeded at worker start and on enable toggles (run_loop.rs:127, 140), so a drive that is already connected is never treated as an arrival. Each new descriptor is evaluated once, and seen_drives is then overwritten unconditionally (run_loop.rs:316-327). Jobs outside their active hours are filtered out (run_loop.rs:319), and AlreadyScheduled/RecentlyAttempted are swallowed (run_loop.rs:433), so the arrival is used up without a run. drive_matches only gates the start (schedule.rs:103-110). The job keeps syncing its fixed source and target strings (job.rs:26-42), and an empty pattern matches any removable drive (schedule.rs:105-107).

## Y15 Calendar jobs without catch-up fire only if a tick lands within 120 s of the occurrence, but the tick can be up to 3600 s or blocked

- Schwere: medium · Urteil: confirmed · Kategorie: bug · Plattformen: windows, linux, android
- Stellen: native/src/syncjobs/core/schedule.rs:47-58; native/src/daemon/os/shared/state.rs:44-54; native/src/daemon/os/shared/state.rs:269-275; native/src/app/core/settings_background.rs:85-111; native/src/daemon/os/shared/run_loop.rs:165-182

**Beschreibung.** due() for Calendar with catch_up = false is true only while now - occurrence <= 120. Timer jobs are evaluated once per outer tick; the GUI lets the user set the tick ('Prüfintervall') from 2 to 3600 s, and long real-time walks or remote connects delay ticks further. Missed occurrences are dropped without any log entry.

**Fehlerszenario.** 'Prüfintervall' set to 300 s to save CPU: a daily 09:00 job without catch-up runs only on days where a tick falls between 09:00:00 and 09:02:00 - roughly 60 % of days are skipped silently.

**Richtung.** Fire when the occurrence lies in (previous evaluation time, now] (persist the previous evaluation time) instead of a fixed 120-s window, or sleep until the next occurrence; at minimum make the grace at least one tick plus scheduling latency.

**Gegenprüfung.** A Calendar job with catch_up=false is due only while now-occ <= 120 (syncjobs/core/schedule.rs:54-56). Timer jobs are evaluated once per outer iteration (run_loop.rs:177-182), whose period is the work time plus the tick. The tick can be set to anything from 2 to 3600 s (state.rs:52-54, 269-275; settings_background.rs:93-96) with no warning, and real-time walks or remote probes stretch the loop further (#5, #6). Skipped occurrences are not logged. Only jobs where the user turned off 'Nachholen' are affected (default catch_up=true, types.rs:170), which keeps the severity at medium.

## Y16 Failed runs count as completed: no retry until the next interval/occurrence; cancelled manual runs also push the schedule

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: windows, linux, android
- Stellen: native/src/daemon/os/shared/job.rs:46-63; native/src/daemon/os/shared/job.rs:128; native/src/daemon/os/shared/job.rs:185-193; native/src/syncjobs/core/types.rs:77; native/src/syncjobs/os/shared/results.rs:114-121; native/src/app/core/bisync_ui.rs:204-207; native/src/mobile/os/shared/domains/sync_run.rs:197-200

**Beschreibung.** mark_run (last_run = now) is persisted after every non-cancelled daemon run, including runs with transfer errors and runs aborted by a failing 'Befehl davor', although SyncJob documents last_run as 'the last successful run'. due() measures intervals and calendar catch-up from that value, so a failed backup is retried only a full period later. The desktop GUI and the Android 'Jetzt' run call mark_run unconditionally, also after the user cancelled.

**Fehlerszenario.** Daily 02:00 backup fails halfway because the NAS rebooted (3,000 errors): next attempt 02:00 the next day. A user starts and cancels a manual run of a 24-h interval job at 10:00: the next scheduled run moves to 10:00 tomorrow although the last complete run was yesterday 08:00.

**Richtung.** Store last_attempt and last_success separately; schedule from last_success and retry failed runs with backoff (e.g. 5 min, 15 min, 1 h); do not mark cancelled manual runs.

**Gegenprüfung.** The daemon persists last_run after every non-cancelled run regardless of errors (job.rs:128) and also after a failed before-command (job.rs:46-63, mark_run=true). SyncJob.last_run is documented as 'last successful run' (types.rs:77), and due() measures both intervals and missed calendar occurrences from it (syncjobs/core/schedule.rs:41-58). A failed backup is therefore retried only one full period later. The desktop GUI calls mark_run before even classifying the outcome (bisync_ui.rs:196-207). Android 'Jetzt' calls it unconditionally (sync_run.rs:197-200), in both cases including after a cancel.

## Y17 Queued jobs run a stale configuration snapshot (also after deletion, disabling or retargeting)

- Schwere: medium · Urteil: confirmed · Kategorie: data-safety · Plattformen: windows, linux, android
- Stellen: native/src/daemon/os/shared/job_supervisor.rs:80-89; native/src/daemon/os/shared/job_supervisor.rs:180-192; native/src/daemon/os/shared/job.rs:10-45; native/src/mobile/os/shared/domains/sync_jobs.rs:163-175; native/src/app/core/menus_sync_jobs.rs:214-222

**Beschreibung.** enqueue stores a clone of the SyncJob and start_next runs that clone; run_one never re-reads the job file. Because all daemon jobs are serialised, a job can wait for hours behind a long backup. If the user disables, deletes or edits it meanwhile (new target, new deletion policy), the old settings still run; Android's delete guard checks only facade tasks and the desktop delete cannot see the daemon queue at all.

**Fehlerszenario.** A 4-h backup runs; a real-time job with Mirror deletion is queued. The user notices its target points at the wrong folder and changes it (or deletes the job): when the queue reaches it, the old snapshot still mirrors into the old target, moving that folder's 'extra' files to the version store.

**Richtung.** Queue only the job id; on start reload the job file, skip it when missing/disabled/trigger changed and run the current settings; drop queued entries when a job file changes.

**Gegenprüfung.** enqueue stores job.clone() (job_supervisor.rs:82), and start_next hands that snapshot to run_one (job_supervisor.rs:180-192), which never re-reads the job file (job.rs:10-129). All jobs are serialized globally (job_supervisor.rs:32-36, 83-88), so a snapshot can wait for hours. Delete, disable and edit never touch the daemon queue: Android delete checks only the facade RUNNING map (mobile sync_jobs.rs:163-175), and desktop delete only removes the file (menus_sync_jobs.rs:214-222). For a deleted job, mark_run fails afterwards (results.rs:115-121), but the sync has already run. Versioning and the delete guard bound the damage, so medium.

## Y18 No exclusion between manual runs and background runs of the same job/pair

- Schwere: medium · Urteil: confirmed · Kategorie: data-safety · Plattformen: windows, linux, android
- Stellen: native/src/app/os/shared/sync_jobs.rs:6-13; native/src/mobile/os/shared/domains/sync_run.rs:38-47; native/src/mobile/os/shared/domains/sync_run.rs:166-177; native/src/daemon/os/shared/job_supervisor.rs:32-44; native/src/bisync/os/shared/orchestration.rs:35-49; native/src/bisync/os/shared/orchestration.rs:99-109

**Beschreibung.** The daemon serialises only its own jobs (process-local scheduled set). The desktop GUI runs bisync in its own process and checks only its own flags; on Android the facade's per-job RUNNING slot and the embedded daemon's supervisor do not consult each other. bisync::run takes no pair/baseline lock, so a 'Jetzt' run and a scheduled/real-time/catch-up run of the same job can overlap on the same pair and baseline file.

**Fehlerszenario.** Real-time (or the hourly interval) fires while the user's manual 'Jetzt' run of the same job is still copying: both runs plan from the same baseline, copy the same files, may treat each other's in-flight writes as concurrent changes or conflicts, and the later baseline save overwrites the other (assumption on exact engine effects: duplicate transfers, spurious conflicts, baseline from a stale snapshot).

**Richtung.** Acquire an OS-level per-pair lock (lock file next to the baseline via flock/LockFileEx) inside bisync::run; let the daemon defer and the GUI/facade report 'läuft gerade im Hintergrund' while the lock is held.

**Gegenprüfung.** The desktop GUI checks only its own flags (app/os/shared/sync_jobs.rs:6-13) and runs bisync in the GUI process. The daemon supervisor and live state are per process (job_supervisor.rs:32-44; live.rs:17-34). On Android, claim() checks only the facade RUNNING map (sync_run.rs:38-47), and the embedded supervisor never consults it. bisync::run derives the pair id and baseline path without any lock (orchestration.rs:35-49, 99-109). The only create_new in bisync persistence is a staging temp file for atomic replace (persistence.rs:170-215). rg finds no flock or LockFileEx. The exact outcome of two overlapping runs (apply guards, last-writer-wins baseline) was not traced.

## Y19 Desktop GUI writes back a stale last_run, reverting the daemon's mark_run

- Schwere: low · Urteil: confirmed · Kategorie: bug · Plattformen: windows, linux
- Stellen: native/src/app/core/init.rs:129; native/src/app/core/menus_sync_jobs.rs:202-213; native/src/app/core/job_editor_ui.rs:15-19; native/src/app/core/job_editor_ui.rs:322; native/src/app/core/job_editor_ui.rs:343; native/src/syncjobs/os/shared/editor.rs:250-253; native/src/syncjobs/os/shared/results.rs:115-121

**Beschreibung.** The GUI loads the job list at startup and reloads it only after its own actions, while the daemon updates last_run directly in the job file. 'Ein/Aus' and 'Speichern' upsert the GUI's in-memory copy (editor uses existing.cloned()), restoring the old last_run. Conversely mark_run is an unlocked read-modify-write of the same file, so a GUI save racing with it can be lost (narrow window). Android reloads from disk (find_job) and is not affected by the first part.

**Fehlerszenario.** GUI opened at 08:00; the daemon runs the daily 09:00 calendar backup and stores last_run 09:40; at 11:00 the user switches the job off and on again: last_run reverts to yesterday and the daemon immediately runs today's 09:00 occurrence again (catch-up); an interval job reruns at once.

**Richtung.** Keep runtime fields such as last_run out of the configuration file (separate state file) or merge them from disk inside upsert; reload the job list when the window opens or the jobs directory changes.

**Gegenprüfung.** The GUI loads jobs once at startup (init.rs:129). It reloads them only after its own actions (menus_sync_jobs.rs:206, 216, 233-240; job_editor_ui.rs:343-346; bisync_ui.rs:232), not when the window is opened (menus_sync.rs:139; picker_locations.rs:37). The toggle upserts the cached clone (menus_sync_jobs.rs:203-205), and the editor builds from existing.cloned() of that cache (job_editor_ui.rs:15-19, 322; editor.rs:250-252). A last_run written by the daemon in between (results.rs:115-121) is therefore reverted, which makes the job due again. mark_run is an unlocked read-modify-write, so a racing GUI save can be lost in a very narrow window. Android reloads from disk (mobile sync_jobs.rs:18-23, 177-185). Severity: the usual effect is one extra run plus a stale 'zuletzt', and the lost-update race is rare, so low fits better than medium.

## Y20 One invalid job file stops all scheduled jobs and catch-up runs

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: windows, linux, android
- Stellen: native/src/syncjobs/os/shared/persistence.rs:54-63; native/src/daemon/os/shared/run_loop.rs:166; native/src/daemon/os/shared/run_loop.rs:413-423; native/src/daemon/os/shared/live.rs:157-158; native/src/daemon/os/shared/catch_up.rs:339-346; native/src/daemon/os/shared/run_loop.rs:223-229

**Beschreibung.** load_dir propagates the first parse/validation error, so one unreadable or newly invalid .conf (downgrade after a release added an enum value, stricter validation in a newer version, manual edit, id/filename mismatch) makes syncjobs::load() fail. The loop then treats the job list as empty and logs 'scheduled sync blocked' on every pass - every 2 s while paused, because the paused sleep loop breaks back into the outer loop each slice - and catch-up runs end with an error. All valid jobs stop.

**Fehlerszenario.** After rolling back to an older build, one job uses a trigger/policy value the old build does not know: every other background backup silently stops and the 256-KiB log fills with the same line until it is wiped.

**Richtung.** Load each file independently; quarantine/skip invalid ones with a persistent visible error and keep scheduling the valid jobs; log the error once per change rather than per tick.

**Gegenprüfung.** load_dir returns the first per-file error (persistence.rs:54-63, line 62). A file fails on any unknown enum value (persistence_codec.rs:111-137 via parse_enum at 224-230), on a validation rule (persistence_codec.rs:164-165), or on an id/filename mismatch (persistence.rs:81-92). load_configured_jobs then logs and returns an empty list (run_loop.rs:413-423) on every outer iteration (run_loop.rs:166). While paused, an iteration happens about every 2 s, because the sleep slice breaks on !permit_mutation (run_loop.rs:223-229). Catch-up runs end with the load error (live.rs:157-158; catch_up.rs:339-346), and the startup pass is skipped (run_loop.rs:351-358). The GUI does show the load error at startup (init.rs:129-136), but all background jobs stop.

## Y21 Global one-at-a-time execution with no runtime limit or hang watchdog; stop/pause joins without timeout

- Schwere: medium · Urteil: partially_confirmed · Kategorie: reliability · Plattformen: windows, linux, android
- Stellen: native/src/webdav/core/webdav.rs:44-45 (10 s connect / 60 s IO inactivity timeouts, applied at 94-109); native/src/sftp/core/session.rs:15-16,231-232 (SSH keepalive 15 s x 3)

**Beschreibung.** All jobs share one slot with no priority, per-job deadline or stall detection. A multi-hour first backup delays every other trigger (an on-connect run may start after the drive was unplugged; real-time waits hours). cancel_and_join() joins the active thread without a deadline on the scheduler thread, so a job blocked in network I/O (assumption: some backend calls can block without timeout) blocks pause, stop and handoff indefinitely; a replacement worker gives up after DAEMON_HANDOFF_TIMEOUT (300 s) while the old one keeps the singleton.

**Fehlerszenario.** A WebDAV server stops answering mid-transfer without closing TCP: the job thread hangs, no other job ever runs, the heartbeat stays fresh ('Dienst aktiv'), and an update handoff ends with no working background worker until reboot.

**Richtung.** Run independent pairs in parallel (bounded), prioritise on-connect/real-time over bulk jobs, add progress-based stall detection and a deadline for cancel_and_join (log and detach), and show 'hängt seit' in the UI.

**Gegenprüfung.** These parts are verified:
- Global serialization of all jobs (job_supervisor.rs:32-36, 83-88).
- No per-job deadline or stall detection.
- Joins without timeout in cancel_and_join (job_supervisor.rs:119-135), used by pause, stop and disable (run_loop.rs:148-158, 225-227, 444-446).
- The 300 s handoff limit (run_loop.rs:28; handoff.rs:131-133).
The concrete WebDAV failure scenario is refuted: WebDAV uses a 10 s connect timeout and a 60 s read/write inactivity timeout (webdav.rs:44-45, 94-109). SFTP has 15 s keepalives with 3 misses plus stage deadlines (session.rs:231-232; connection.rs:208, 240). Transfers check cancel per chunk (apply_transfer.rs:388). An indefinite hang remains plausible only for OS-level I/O on hard NFS, FUSE or SMB mounts used as local roots, which was not verified. A blocked join also stops the desktop heartbeat, which leads into the handoff-timeout stranding described under #5.

## Y22 Real-time signature ignores the job's filters and its own writes: excluded churn and self-writes cause useless full runs

- Schwere: medium · Urteil: confirmed · Kategorie: performance · Plattformen: windows, linux, android
- Stellen: native/src/daemon/os/shared/schedule.rs:15; native/src/daemon/os/shared/run_loop.rs:265-276; native/src/daemon/os/shared/job.rs:68-76

**Beschreibung.** tree_sig receives only a root path; ignore globs, include_hidden=false and size/age filters, which the engine applies through WalkFilter, are not applied to change detection. Changes to excluded paths (temp files, caches, node_modules, hidden swap files) trigger runs or keep the debounce from settling. For local targets the job's own writes change the signature, causing one redundant follow-up run (full walk of both sides; for remote targets a full remote listing) whenever the run outlasts the 60-s cooldown.

**Fehlerszenario.** Real-time backup of a project folder with ignore 'node_modules/**' and '**/*.tmp': a running build rewrites thousands of ignored files every few seconds, so the job either never settles or runs repeatedly with nothing to copy, each time rescanning the whole tree and the NAS target.

**Richtung.** Apply the job's glob set and hidden/size/age rules in the watcher/signature; ignore events on the destination side caused by the job's own run (record the post-run state or suppress events while the job runs).

**Gegenprüfung.** tree_sig receives only a root (schedule.rs:15) and counts every entry that is not link-like. The job's WalkFilter (include_hidden, ignore, size and age bounds) is built only for the sync itself (job.rs:68-76). Churn in excluded paths therefore triggers runs or keeps the debounce from settling (#3). For local roots, the job's own writes change the signature. A follow-up run is admitted only if the post-run settle comes at least 60 s after admission; otherwise it is swallowed as RecentlyAttempted (#1). It causes exactly one redundant full walk, because baseline and versions live in app data (bisync persistence.rs:15, 55-61), not inside the roots.

## Y23 Android periodic runs: ~10-minute cap without foreground, success returned on failures, wasted work per window

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: android
- Stellen: android/app/src/main/java/app/smartexplorer/android/work/SyncWorker.kt:58-65; android/app/src/main/java/app/smartexplorer/android/work/SyncWorker.kt:66-77; android/app/src/main/java/app/smartexplorer/android/work/SyncWorker.kt:88-91; android/app/src/main/java/app/smartexplorer/android/work/SyncWorker.kt:95-109; native/src/mobile/os/shared/domains/background.rs:12; native/src/mobile/os/shared/domains/background.rs:22-31; native/src/daemon/os/shared/run_loop.rs:96-107; native/src/daemon/os/shared/run_loop.rs:183-191; native/src/daemon/os/shared/catch_up.rs:96

**Beschreibung.** setForeground from a background-started worker is refused on Android 12+ unless the app has a start exemption (e.g. battery optimisation off); the code only logs, so the run stays a regular job and JobScheduler stops it after about 10 minutes, cancelling the run's own jobs; large first syncs never complete in one window and the next window begins with a full rescan (dataSync foreground work is additionally capped at 6 h per 24 h on Android 15+, assumption). Every failure path (worker not ready within READY_WAIT = 10 s - readiness waits for the synchronous Share initial load before the loop serves; too many open runs; gate closed) returns Result.success(), so WorkManager does not retry until the next period (up to 6 h). In each window the daemon also walks all real-time trees on the scheduler thread before servicing the catch-up, although the catch-up runs every real-time job anyway.

**Fehlerszenario.** Interval '6 h', first backup of 30 GB of photos to a NAS: each window copies for about 10 minutes, is cancelled, and the window 6 h later spends minutes rescanning both trees; on a cold start where Share relay discovery takes 12 s, bg.catchUp ends with 'busy' and the whole 6-h slot is lost.

**Richtung.** Prompt for the battery-optimisation exemption when large jobs exist (or use user-initiated data transfer jobs on Android 14+ for user-started big syncs); return Result.retry() for transient failures; checkpoint progress so a cancelled run resumes without a full rescan; start scheduling independently of the Share initial load; skip tree_sig while scheduling is deferred or inside worker windows.

**Gegenprüfung.** promote() swallows the IllegalStateException that API 31+ raises for a refused foreground start (SyncWorker.kt:95-109), so the run stays a regular job with the roughly 10-minute limit. A stop then cancels the run's own jobs (SyncWorker.kt:78-87; catch_up.rs:146-153). Every outcome other than a stop returns Result.success(): a start failure (SyncWorker.kt:57-65), any task end state including errors (66-77), and a follow failure (88-91). wait_ready is bounded to 10 s (background.rs:12, 22-31). Readiness requires live::serve, which comes after the synchronous Share reload_now (run_loop.rs:96-107; embedded.rs:84-88). Intervals go up to 6 h (BackgroundSettings.kt:63). In each window the defer change leads the loop through enqueue_realtime_jobs, with full tree walks, before service_catch_up (run_loop.rs:183-201), although catch-up runs every RealTime job anyway (catch_up.rs:96). targetSdk 36 (build.gradle.kts) makes the Android 15 dataSync 6 h limit apply. Note: due interval and calendar jobs owned by the schedule are not cancelled by a stop (#11); they continue outside the window instead.

## Y24 Desktop worker lifecycle: no crash restart, Windows startup-disable not detected, Linux only via XDG autostart, nothing while logged off or asleep

- Schwere: medium · Urteil: confirmed · Kategorie: platform · Plattformen: windows, linux
- Stellen: native/src/autostart/os/windows.rs:140-154; native/src/autostart/os/linux_os.rs:64-82; native/src/app/core/init.rs:32-52; native/src/app/core/settings_background.rs:67-83; native/src/daemon/os/shared/run_loop.rs:135-152

**Beschreibung.** Windows: the worker starts once per logon from the HKCU Run value; is_enabled() checks only that value and ignores Explorer/StartupApproved/Run, so an entry disabled in Task Manager or Settings is reported as enabled ('Dienst startet beim nächsten Anmelden') but never starts. Linux: only an XDG autostart .desktop file - no systemd user unit, no restart, nothing in non-XDG or headless sessions; desktop UIs that disable autostart entries keep the file, so is_enabled() stays true. On both, a crashed worker is restarted only when the GUI is opened again. The 'background sync enabled' flag is the autostart entry itself, so a startup-cleanup tool that removes it disables sync and cancels the running job. Nothing runs jobs while the user is logged off or wakes the machine for a scheduled backup.

**Fehlerszenario.** User disables Smart Explorer under Task Manager > Autostart to speed up boot: no worker at logon, the settings still show background sync as enabled, nightly backups silently stop until the GUI is opened.

**Richtung.** Windows: honour StartupApproved or register a Task Scheduler task (logon trigger, restart on failure, optional wake-to-run / run whether logged on); Linux: systemd --user service with Restart=on-failure plus timers (Persistent=true) where available, XDG autostart as fallback; store the enable flag separately from the autostart mechanism; add a watchdog relaunch from the GUI.

**Gegenprüfung.** On Windows, is_enabled reads only the HKCU Run value SmartExplorerSync (autostart/os/windows.rs:141-149). It ignores Explorer\StartupApproved\Run, where Task Manager and Settings record a disabled startup entry. On Linux there is only an XDG autostart file, and is_enabled checks only that it exists (autostart/os/linux_os.rs:64-66, 72-83), ignoring disable markers written by desktop UIs. There is no restart supervisor; the GUI restarts a dead worker only at its own start (init.rs:32-52). The autostart entry itself is the sync-enabled flag, and the loop cancels running work when it disappears (run_loop.rs:135-151). Nothing runs jobs while the user is logged off or wakes the machine. These are design limitations rather than code defects, and medium fits.

## Y25 Linux auto-pause toggles are no-ops

- Schwere: low · Urteil: confirmed · Kategorie: platform · Plattformen: linux
- Stellen: native/src/daemon/os/linux_os/platform.rs:27-33; native/src/app/core/settings_background.rs:163-190

**Beschreibung.** battery_saver_on() and on_metered_network() always return false on Linux, but the GUI shows and persists both auto-pause checkboxes on every desktop OS.

**Fehlerszenario.** Linux laptop tethered to a metered phone hotspot with 'Bei getakteter Verbindung' enabled: background backups upload gigabytes anyway.

**Richtung.** Implement via UPower (OnBattery) / power-profiles-daemon (power-saver) and NetworkManager's Metered property, or hide the options on Linux.

**Gegenprüfung.** linux_os/platform.rs:27-33 returns false for both conditions, and pause_reason relies only on them (state.rs:118-124). settings_background.rs:163-190 shows and saves both checkboxes on every desktop OS. Only the hover texts mention Windows (settings_background.rs:169, 173).

## Y26 Calendar edge cases: DST days skipped, day 29-31 skipped in short months, new jobs run immediately, future last_run blocks runs

- Schwere: low · Urteil: confirmed · Kategorie: bug · Plattformen: windows, linux, android
- Stellen: native/src/syncjobs/core/schedule.rs:41-46; native/src/syncjobs/core/schedule.rs:49; native/src/syncjobs/core/schedule.rs:75-78; native/src/syncjobs/core/schedule.rs:100-106; native/src/syncjobs/core/validation.rs:70-72; native/src/syncjobs/core/validation.rs:79-81; native/src/syncjobs/core/types.rs:160; native/src/syncjobs/core/types.rs:170

**Beschreibung.** last_occurrence skips a day when the local time is nonexistent or ambiguous (`.single()` is None), so 02:00-02:59 jobs are skipped on both European DST change days; cal_monthday 29-31 never matches in shorter months; a new calendar job (last_run 0, catch_up default true) runs immediately because the last past occurrence counts as missed; a last_run in the future (device clock briefly wrong) suppresses interval and calendar runs until real time passes it, as validation only rejects negative values.

**Fehlerszenario.** Daily 02:30 backup skipped on the last Sunday of March and October; 'monatlich am 31.' skips five months a year; a nightly job created at 14:00 starts a full backup at 14:00; a phone whose clock showed 2030 during one run never runs that interval job again until 2030.

**Richtung.** Map nonexistent times to the next valid instant and use earliest() for ambiguous ones; clamp the month day to the month length; seed last_run/created_at when a job is created or its schedule changes; treat last_run > now + slack as unknown.

**Gegenprüfung.** All four edge cases follow from the code:
- DST: last_occurrence skips a day when .single() is None (syncjobs/core/schedule.rs:100-106). That covers both nonexistent (spring) and ambiguous (autumn) local times.
- Days 29-31: day_matches compares the day of month exactly (schedule.rs:76-77), while the editors allow 1-31 (editor.rs:205-208; job_json.rs:289-291).
- New jobs: they start with last_run 0 and catch_up true (types.rs:160, 170), so the latest past occurrence is due at once (schedule.rs:48-52).
- Future last_run: validation rejects only negative values (validation.rs:70-72), so a future last_run blocks interval runs (schedule.rs:45) and calendar runs (schedule.rs:49) until the clock passes it.

## Y27 'Letzter Hintergrundlauf' is updated by cancelled, blocked or empty catch-up runs

- Schwere: low · Urteil: confirmed · Kategorie: gap · Plattformen: android
- Stellen: native/src/daemon/os/shared/live.rs:175-189; native/src/mobile/os/shared/domains/background.rs:62; android/app/src/main/java/app/smartexplorer/android/ui/sync/BackgroundParts.kt:78-79

**Beschreibung.** record_finished stores the finish time for every finished catch-up run, including 'Abgebrochen', gate-closed ('Hintergrund-Sync ist pausiert') and 'Keine fälligen Jobs', and the settings show it as the last background run, suggesting backups ran.

**Fehlerszenario.** Sync is paused for days; every periodic worker run ends immediately with the pause message, yet 'Letzter Hintergrundlauf: vor 5 min' is displayed.

**Richtung.** Persist the outcome with the timestamp (jobs executed/succeeded) and show blocked/failed runs distinctly.

**Gegenprüfung.** record_finished writes catchup.last for every finished entry (live.rs:175-189). That includes a cancel ('Abgebrochen', catch_up.rs:217/304), a closed gate with the pause or disabled text (catch_up.rs:279-283; run_loop.rs:379-389), 'Keine fälligen Jobs' (catch_up.rs:401) and worker shutdown (live.rs:124). bg.status exposes the value (background.rs:62), and the UI shows it as 'Letzter Hintergrundlauf' (BackgroundParts.kt:78-79).

## Y28 Desktop 'Beim Start' jobs run on every worker (re)start, not once per logon or boot

- Schwere: low · Urteil: confirmed · Kategorie: bug · Plattformen: windows, linux
- Stellen: native/src/daemon/os/shared/run_loop.rs:116-122; native/src/daemon/os/shared/run_loop.rs:363-367; native/src/daemon/os/shared/boot_marker.rs:1-5; native/src/app/core/init.rs:32-52

**Beschreibung.** The boot marker is consulted only by the embedded (Android) worker; the desktop worker runs the startup pass at every start, including update handoffs, GUI-triggered replacements after a stale heartbeat and enabling background sync.

**Fehlerszenario.** Every auto-update (handoff to the new executable) or GUI start after a long real-time walk re-runs a heavy 'Beim Start' backup during the working day.

**Richtung.** Use a boot/logon marker on desktop as well (Linux /proc/sys/kernel/random/boot_id or session id; Windows boot time or logon session id) and run the pass once per marker.

**Gegenprüfung.** The boot marker is consulted only for the embedded worker (run_loop.rs:363-367; boot_marker.rs:1-5). Every desktop worker start therefore runs the OnStartup jobs: at logon, at an update handoff (init.rs:33-41), at a replacement after a stale heartbeat (init.rs:43-51), when background sync is enabled (settings_background.rs:23-26), and at the in-loop enable toggle (run_loop.rs:141-144).

## Y29 One failed file discards the whole run's baseline update (any error => old baseline kept)

- Schwere: critical · Urteil: confirmed · Kategorie: data-safety · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/orchestration.rs:312-320; native/src/bisync/os/shared/orchestration.rs:88-95; native/src/bisync/core/plan.rs:245-279; native/src/bisync/os/shared/apply_pool.rs:300-318; native/src/bisync/os/shared/tests/safety.rs:280-305; native/src/agent_proto/core/relative_path.rs:41-43; native/src/bisync/os/shared/persistence.rs:221-228

**Beschreibung.** run_full returns `baseline: base` as soon as `errors` is non-empty, skipping the re-walk, update_baseline, save_baseline and prune_versions, although update_baseline already keeps failed/conflicting rels at their previous entry and records only `report.completed`. All successful copies/deletes of a run with a single per-file error are therefore never recorded, and while the error persists the baseline never advances (versions are never pruned either; Mirror never bootstraps its incremental index). Routine triggers: drift on a file being written during the run (copy/delete guards report it as an error), a locked file (open PST/NTUSER.DAT/SQLite), a name the destination cannot hold (':' '?' etc. on Windows/Android, trailing dot/space), a case collision (see case finding), a transient network error with job retries defaulting to 0, a full local versions store.

**Fehlerszenario.** Two-way job; one file fails every run (e.g. a locked file). Day 1 the user creates report.docx on A -> copied to B, baseline not saved. Day 2 the user deletes report.docx on A -> plan sees a=None, b=Some, base=None => only B 'changed' => CopyBtoA => the deleted file is resurrected on A. Every edit of a file transferred during the frozen period becomes a strict-mode conflict (both sides differ from the stale baseline; copies do not carry the source mtime). With real-time triggers on an active folder almost every run ends with a drift error on the file being written, so the baseline effectively never advances (consistent with the reported 'real-time sync is unreliable').

**Richtung.** On non-cancel errors still observe the post-apply state of the completed rels and save `update_baseline(..., &report.completed, &converged, &conflicts)`; skip only prune_versions and incremental bootstrap. Classify drift on a concurrently changing file as 'skipped, retried next run' instead of a run failure.

**Gegenprüfung.** orchestration.rs:312-320 returns `baseline: base` whenever `errors` is non-empty, before the re-walk (327-380), update_baseline (384), save_baseline (387) and prune_versions (400); run_inner bootstraps only when out.errors.is_empty() (88-95). Recording a partial run would be safe: update_baseline (plan.rs:245-279) records only `applied` and converged rels, and `completed` is filled only on Ok in apply_pool.rs record(). The intended behavior test (safety.rs:307-360) builds its baseline with its own helper run_with_errors (280-305), so the production early return is untested. Resurrection trace holds: base None, an None, bn Some -> a_changed false, b_changed true (plan.rs:143-144) -> CopyBtoA (169-172). Copies made in a failed run also become strict conflicts in the next run: base None, both sides Some, size equal but mtime differs (plan.rs:37, 150, 181-188). Retries default 0 (syncjobs types.rs:188); apply_retry.rs:69 retries only transient pre-commit errors. One more routine trigger the reviewer missed: a name with ':' passes the walk (vfs delete.rs:318-327 does not reject ':') but save_baseline validates every rel with ValidatedRelativePath, which rejects ':' (relative_path.rs:41-43, persistence.rs:228, 296-298). Every run of such a Linux/Android/SFTP pair therefore ends at orchestration.rs:387-398, which also returns `base`. 'Almost every real-time run' overstates it (debounce default 10 s, types.rs:166), but the defect is real. Critical holds because of the default-mode cascade: permanent conflicts that unattended daemon runs never resolve, resurrected deletions, versions that are never pruned, and Mirror never bootstrapping.

## Y30 Mirror (backup) full runs re-copy and re-version every file because copies never receive the source mtime

- Schwere: critical · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/core/plan.rs:59-104; native/src/bisync/core/plan.rs:37; native/src/bisync/os/shared/snapshot_hash.rs:63-72; native/src/bisync/os/shared/apply_transfer.rs:341-353; native/src/vfs/os/shared/local.rs:121-125; native/src/vfs/core/promotion.rs:90-104; native/src/bisync/os/shared/snapshot_dir.rs:112-121; native/src/bisync/os/shared/orchestration.rs:88-95; native/src/bisync/os/shared/incremental.rs:137-138; native/src/bisync/os/shared/incremental.rs:344-353; native/src/local_access/os/windows/directory.rs:355-358

**Beschreibung.** Mirror planning is stateless: a rel is converged only if sig_eq(source, destination). Without a content hash on both sides (local<->local, local<->SFTP/FTP/SMB/peer) that requires equal size and |dmtime| <= modify_window (default 0). stage_source writes the destination via open_write + stream and promotes it; nothing in the sync or VFS layer sets the destination mtime, so every destination carries its write time and never compares equal to its source. Every full Mirror run therefore re-copies every file, and because job runs are always reversible (types.rs:242) copy_replace first copies the existing destination file into the local versions store. Full runs are the norm: on every run when the source is not local and has no change feed (SFTP/FTP/SMB/WebDAV -> local backups, incremental.rs:137-138), whenever the source contains any link/junction/app-trash entry (omissions block bootstrap at orchestration.rs:88-95, and walk_files without an omission sink errors on links so incremental collection always rebuilds, snapshot_dir.rs:112-127), and after any failed or interrupted run.

**Fehlerszenario.** Windows Mirror job C:\Users\X\Documents -> E:\Backup (default MtimeSize). Documents contains the hidden legacy junctions 'My Music/My Pictures/My Videos', so incremental mode never bootstraps and every run is a full run; every file's mtime differs from its copy => each run rewrites the whole tree to E: and first copies each old E: file into %APPDATA%/.../versions_<pair>/<second>/ on C:, retained 30 days => multi-fold disk use until runs fail with disk-full errors. An SFTP-NAS -> local-disk backup re-downloads everything on every run in the same way.

**Richtung.** Preserve the source mtime on the staged file before promotion for every backend that can (std File::set_modified, SFTP setstat, WebDAV X-OC-Mtime/PROPPATCH, Drive modifiedTime, FTP MFMT), record per-backend timestamp granularity and use it for cross-side comparisons; for Mirror also use the stored per-side pair recorded after the last copy instead of a raw cross-side compare; allow incremental bootstrap when the only omissions are protected links.

**Gegenprüfung.** Mirror planning is stateless: a rel converges only when sig_eq(sn, dn) (plan.rs:77-88), otherwise it becomes CopyAtoB (90-94). hash_mode gives HashMode::None to both sides for local<->local, SFTP, FTP, SMB and peer pairs (snapshot_hash.rs:63-72), so the compare falls through to plan.rs:37 with window 0 (types.rs:307, syncjobs types.rs:174). The copy is written through open_write plus stream (apply_transfer.rs:341-353). LocalBackend::open_write is File::create (local.rs:121-125) and promotion is a rename (promotion.rs:90-104). rg finds no set_modified, FileTimes, setstat, MFMT or X-OC-Mtime in any sync or VFS write path, so the destination keeps its write time. Job runs are always reversible (syncjobs types.rs:242; apply.rs:52), so copy_replace first backs up the existing destination (apply_transfer.rs:76-88). Full runs are common for three reasons. (1) A non-local source without a change feed makes try_incremental_mirror return None (incremental.rs:137-138). (2) Any link, junction or app-trash entry is recorded as an omission, even when hidden or ignored, because record(&rel, !excluded) runs before the `excluded` check (snapshot_dir.rs:112-121). Omissions block bootstrap (orchestration.rs:88-95; tests/links.rs:62 and 108-109 rely on this). Windows junctions carry a name-surrogate tag and are therefore link-like (windows/directory.rs:355-358). (3) Every full run resets `bootstrapped` (incremental.rs:344-353). Correction to the affected-backend list: WebDAV/Nextcloud reports provides_content_hash (webdav.rs:470), so files that carry server checksums get hashes on both sides (local side HashMode::Full) and are not re-copied. The re-copy applies to SFTP, FTP, SMB, peers, local-to-local, and WebDAV files without checksums.

## Y31 An empty, unmounted or swapped side is taken at face value: mass deletion, all delete guards off by default

- Schwere: critical · Urteil: confirmed · Kategorie: data-safety · Plattformen: Linux, Windows, Android
- Stellen: native/src/vfs/os/shared/sync_roots.rs:10-35; native/src/bisync/core/plan.rs:95-99; native/src/bisync/core/plan.rs:169-178; native/src/bisync/os/shared/orchestration.rs:209-234; native/src/syncjobs/core/types.rs:178-179; native/src/bisync/os/shared/incremental_collect.rs:247-259; native/src/bisync/os/shared/incremental_changes.rs:155-180; native/src/vfs/core/core.rs:365-368

**Beschreibung.** validate_sync_roots only checks for empty strings and overlap; a root that exists but is empty walks successfully as an empty tree. There is no replica identity (no marker file, no volume/root id for local/SFTP/FTP/SMB: change_root_id defaults to None), no 'side became empty' check, and the guards max_delete/max_delete_pct default to 0 (= unlimited) for new jobs. Disappearance of a whole side is therefore propagated as ordinary deletions, in the full and in the incremental path.

**Fehlerszenario.** (1) Linux two-way job with B = /mnt/nas (fstab CIFS/NFS) while the NAS is down: /mnt/nas is an empty directory => for every baseline entry B 'changed' (deleted), A unchanged => DeleteA for the whole working set on A (each file first copied into the local versions store until it fills). (2) Mirror A->B where A is an unmounted source mount point: full Mirror deletes every 'orphan' on B; incremental mirror turns every stored item into a managed Remove (stat -> NotFound) => the backup is wiped. (3) Two rotating USB drives on the same letter/mount point in a two-way job: files added since the other drive was attached look 'deleted on B' and are deleted on A.

**Richtung.** Store a per-pair replica id (marker file at each root, cf. Syncthing .stfolder / rclone --check-access) or volume+root identity with the baseline and refuse to run when it is missing/different; abort or require confirmation when a side that had N>0 baseline entries is now empty; ship a default percentage guard (e.g. 50%) for new and migrated jobs and enforce it in the incremental path too.

**Gegenprüfung.** validate_sync_roots only rejects empty strings and overlap (sync_roots.rs:16-34). A missing root fails safely: the walk stats the root first (snapshot_walk.rs:74). An existing but empty directory, however, walks as an empty tree. Two-way: every baseline entry with bn None and an unchanged gives (false,true) -> DeleteA (plan.rs:169-178). Full Mirror: (None, Some) -> DeleteB (plan.rs:95-99). The guard applies only when its limits are non-zero (orchestration.rs:210-220), and the defaults are 0 in SyncJob::new (types.rs:178-179), the desktop editor (editor.rs:116-117) and Android (SyncApi.kt:76-77). Incremental mirror: an empty source walk turns every stored rel into stat NotFound -> managed Remove (incremental_collect.rs:247-252). delete_guard_trips is off at zero (incremental_changes.rs:178-179), and the root id is None outside Drive, which matches (core.rs:365-368; incremental.rs:422-426). Mitigations that keep it short of total loss: deleted files are first copied into the versions store (reversible), and volumes whose path disappears entirely (Windows drive letter gone, udisks-removed mount dir, unmounted Android SD card) fail at the root stat. The exposure is persistent empty mount-point directories, emptied remote roots and swapped media on the same path.

## Y32 Cancelled, killed or failed runs turn every completed transfer into a conflict (or duplicate/redundant copy) on the next run

- Schwere: high · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/orchestration.rs:297-309; native/src/bisync/os/shared/apply_transfer.rs:327-371; native/src/bisync/core/plan.rs:31-38; native/src/bisync/core/plan.rs:120-153; native/src/bisync/core/plan.rs:181-233; native/src/syncjobs/core/types.rs:174; native/src/bisync/os/shared/tests/move_retry.rs:131-136

**Beschreibung.** Completed actions are recorded only by the single baseline save at the end of a successful run (cancel returns the old baseline, a crash/kill saves nothing, errors see the first finding). On the next run each transferred rel is 'changed on both sides' relative to the stale baseline and is judged by a direct A-vs-B comparison; since copies carry their write time and modify_window defaults to 0, size-equal copies compare unequal unless both sides have a content hash. Strict mode reports conflicts; KeepBoth creates a '(Konflikt ...)' copy of identical content; NewerWins copies the newer-looking copy back over the original (versioning the original); one-way DestWins never updates the file again. FinalizeMove recovery (plan.rs:132-139) also requires sig_eq(an, bn) and therefore never fires in MtimeSize mode.

**Fehlerszenario.** First two-way sync of 50,000 files local -> USB/NAS is cancelled (or the Android process is killed) after 30,000 copies. Next run: baseline empty, 30,000 files on both sides with equal sizes but A mtime = original, B mtime = copy time => 30,000 conflicts in the default strict mode. A move job whose source delete failed shows a conflict between two identical files instead of finishing the move.

**Richtung.** Preserve source mtimes on copy; checkpoint completed actions durably during the run (append-only journal or periodic baseline saves from targeted re-stats of completed rels) so cancel/crash keep them; when both sides changed to the same size, confirm equality with a hash or the recorded copy pair before declaring a conflict.

**Gegenprüfung.** A cancel returns the old baseline (orchestration.rs:145-146, 270-281, 301-309), errors do the same (#0), and a killed process saves nothing. On the next run every transferred rel is Some on both sides against a None or stale base, so both count as changed (plan.rs:143-144). sig_eq(an,bn) fails because the copy carries its write time (apply_transfer.rs:341-353; local.rs:121-125) and the window is 0 (syncjobs types.rs:174). Outcomes by policy: FileLevel -> conflict (plan.rs:181-188). KeepBoth -> the copy on B is newer -> KeepBothBtoA -> a conflict copy of identical content on A (plan.rs:191-201; apply.rs:193-221). NewerWins -> CopyBtoA of identical content, with A backed up first (plan.rs:207, 224-226). One-way DestWins -> no action, never recorded (plan.rs:224; update_baseline records only applied/converged). FinalizeMove needs sig_eq (plan.rs:132-139), so in MtimeSize without hashes it falls through to a conflict; move_retry.rs:131-136 had to switch to SizeOnly. Exception: pairs where both sides carry content hashes (local<->Drive/Nextcloud) converge (plan.rs:31-33).

## Y33 Post-apply full re-walk records edits made during the run as already synchronized (silent loss of propagation)

- Schwere: high · Urteil: confirmed · Kategorie: data-safety · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/orchestration.rs:327-385; native/src/bisync/core/plan.rs:255-276

**Beschreibung.** After a run with any transfer, each touched side is walked again and update_baseline records `(at2.get(rel), bt2.get(rel))` for every applied and every converged rel, without comparing with the signatures the plan/apply acted on. The re-walk happens after the whole apply phase (minutes to hours), so an edit or deletion made on either side after the first walk is stored as the synchronized state and is never propagated.

**Fehlerszenario.** Two-way job with a long apply phase: report.docx is copied A->B at 10:05; the user saves it again on A at 10:10; the re-walk at 10:30 records A=v2, B=v1-copy. The real-time run triggered by the 10:10 save sees A equal to the baseline => no action; B keeps v1 indefinitely, and a later edit of the file on B overwrites v2 (only recoverable from versions). Same for any converged file (e.g. all identical files of a first run) edited during the run, and for a file deleted on B during the run (recorded as (Some, None), neither restored nor propagated).

**Richtung.** Record for each completed action the signatures the action itself observed (source sig validated by capture/revalidate, destination sig stat'ed right after promotion) and the first-walk signatures for converged rels; if a later observation differs, keep the previous baseline entry so the change is detected next run. This also removes the need for the full re-walk.

**Gegenprüfung.** The touched sides are re-walked only after the whole apply phase (orchestration.rs:335-379). update_baseline(&planning_base, &at2, &bt2, ...) (384) then stores at2/bt2 for every applied and converged rel (plan.rs:255-276) without comparing them to the signatures the plan acted on. check_post_scan only checks duplicates (duplicate_plan.rs:11-17). An edit made after a file's copy (or to a converged file) is therefore recorded as synced; on the next run a_changed is false and nothing is done. A deletion on B during the run is recorded as (Some, None) and stays unrepaired. Nuances: an edit made before that file's own apply is caught by capture() (apply_guard.rs:73-80) and leads to #0's early return instead. One-way jobs without move do not re-walk the source (orchestration.rs:335-336), so source edits there still propagate.

## Y34 A single unreadable, unrepresentable or vanishing entry aborts the whole snapshot and therefore every run

- Schwere: high · Urteil: confirmed · Kategorie: reliability · Plattformen: Linux, Windows, Android
- Stellen: native/src/bisync/os/shared/snapshot_walk.rs:72-81; native/src/bisync/os/shared/snapshot_walk.rs:243-246; native/src/bisync/os/shared/snapshot_dir.rs:68; native/src/bisync/os/shared/snapshot_dir.rs:136; native/src/vfs/os/shared/local.rs:81-87; native/src/vfs/core/delete.rs:318-331; native/src/local_access/os/windows/read.rs:29-41; native/src/agent_proto/core/relative_path.rs:41-43

**Beschreibung.** The walk fails as a whole on the first error so that a partial tree never becomes a baseline, but only links/app-trash are handled as protected omissions. Every other unreadable entry fails the run: a directory without read permission, a non-UTF-8 name (Linux/Android), a name containing '\\' (legal on Linux/Android/SFTP/FTP, rejected by validate_child_name), Windows 'unreachable' names, a subdirectory deleted/renamed between being queued and listed (stat NotFound), and in HashMode::Full/FullFresh (local side of local<->Drive/Nextcloud in MtimeSize mode; all sides in Checksum mode) any file that cannot be opened for hashing or vanished.

**Fehlerszenario.** Sync to/from the root of an ext4 USB drive: lost+found (root, 0700) => EACCES => every run fails. Windows NTFS drive root with hidden files included (job default include_hidden=true): 'System Volume Information' access denied (SYSTEM-only ACL) => every run fails. Local Documents <-> Google Drive with an open, changing Outlook .pst => re-hash => sharing violation => every run fails. Real-time sync of a project folder whose build/temp directory is removed during the walk => the run fails.

**Richtung.** Treat such entries like links: record a protected omission (the entry, or its parent for unrepresentable names), keep counterparts and baseline entries, report them and continue; a directory that vanished mid-walk is omitted for this run (never inferred as deleted); a file that cannot be hashed is omitted for this run. Add default ignores for lost+found, System Volume Information and $RECYCLE.BIN at volume roots.

**Gegenprüfung.** Every error in a stat or listing fails the walk: list_plain_directory does stat + list_dir (snapshot_walk.rs:72-81), the worker calls fail(error) (245), and walk_tree returns the error (53-58). Only links, app trash and Android/data are converted into omissions (snapshot_dir.rs:112-127). The cases cited all fail every run: a backslash in a name (validate_child_name, delete.rs:318-324, called at snapshot_dir.rs:68); unreachable or non-UTF-8 local names (local.rs:81-87); a hashing failure in Full/FullFresh mode (snapshot_dir.rs:136, 174-181); a subdirectory that vanishes before it is listed (stat NotFound at snapshot_walk.rs:74). Windows nuance: read.rs:29-41 retries access-denied with the backup privilege or a broker grant, so 'System Volume Information' fails only without such a grant, which is the typical case for a non-elevated daemon. Another unrepresentable-name case: ':' passes the walk but makes save_baseline fail on every run (relative_path.rs:41-43, persistence.rs:228).

## Y35 Hard 1,000,000-entry / 128 MiB caps make large trees permanently unsyncable

- Schwere: high · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/snapshot.rs:15-16; native/src/bisync/os/shared/snapshot.rs:197-198; native/src/bisync/os/shared/snapshot_dir.rs:96-105; native/src/bisync/os/shared/snapshot_agent.rs:9-10; native/src/bisync/os/shared/snapshot_agent.rs:42-45; native/src/bisync/os/shared/incremental_collect.rs:13-15; native/src/bisync/os/shared/state_validation.rs:8-10; native/src/bisync/os/shared/persistence.rs:10-12; native/src/bisync/os/shared/persistence.rs:221-237

**Beschreibung.** The walk aborts once ~1,000,000 entries (files AND directories AND filtered files of visited folders) were seen or the sum of full absolute paths (root + rel per entry) exceeds 128 MiB; the baseline file and the incremental store also reject more than 1,000,000 entries. There is no configuration or memory-derived limit; the failure repeats every run.

**Fehlerszenario.** Backup of a home directory, mail store or NAS photo archive with 1.2 M entries (or ~850k entries with 150-byte absolute paths) fails every run with 'sync tree exceeds its bounded collection budget'; nothing is synced.

**Richtung.** Derive limits from available memory or make them configurable, keep strict caps only for untrusted remote peers, count rel bytes instead of absolute paths, and reduce per-entry memory (interned path segments, compact signatures).

**Gegenprüfung.** MAX_WALK_NODES is 1,000,000 and MAX_WALK_TEXT_BYTES is 128 MiB (snapshot.rs:15-16), with counters starting at 1 and root.len() (snapshot.rs:197-198). snapshot_dir.rs:96-105 counts every listed entry before any filter (directories, excluded files and links included) and adds the absolute joined path length. The agent walk uses the same caps (snapshot_agent.rs:9-10, 42-45), saving rejects more than 1M baseline entries (persistence.rs:221-224), and the state store has the same budget (state_validation.rs:8-10, 29-36). All are compile-time constants with no configuration, so the error repeats on every run. The ~850k x 150 B example is just under 128 MiB (about 895k entries would trip it); a minor inaccuracy.

## Y36 Size/age/hidden filters turn 'left the filter on one side' into a deletion of the counterpart

- Schwere: high · Urteil: confirmed · Kategorie: data-safety · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/snapshot_dir.rs:107-146; native/src/bisync/core/plan.rs:95-99; native/src/bisync/core/plan.rs:155-180; native/src/bisync/os/shared/incremental_collect.rs:247-259; native/src/bisync/os/shared/snapshot_duplicates.rs:43-55; native/src/bisync/os/shared/duplicate_plan.rs:31-46

**Beschreibung.** In the full walk a file failing the size/age bounds (or excluded as hidden/ignored) is dropped from that side's tree without recording an omission. Size/age/hidden are evaluated on each side's own signature/attributes, so the same rel can be inside the filter on one side and outside on the other; the planner then sees 'deleted on one side' and deletes the counterpart (two-way/one-way Propagate) or removes it as a Mirror orphan. The incremental path treats the same situation as unmanaged (no delete) and the duplicate path records filtered entries as omissions, so behavior depends on the code path.

**Fehlerszenario.** Mirror backup with 'max size 100 MB': a video/log grows from 90 to 120 MB on A => excluded on A while the 90 MB copy on B is still inside => DeleteB. 'Only files modified in the last 30 days': B's copies carry the copy time, so A's original ages out first => B's copy is deleted. 'Only files older than N days': editing a file on A makes it too young => B's previous copy is deleted. include_hidden=false on Windows: setting the hidden attribute on a synced file deletes its counterpart (attributes are not copied).

**Richtung.** Record filter-excluded existing entries as non-reported protected omissions (same mechanism as links) so the counterpart and its baseline entry are preserved; evaluate size/age against the baseline signature as well.

**Gegenprüfung.** Hidden or ignored entries are dropped (snapshot_dir.rs:107-108, 128-130), and size/age failures are dropped without an omission (135). Each side is filtered by its own size, mtime and hidden attribute, so the two sides can disagree. In two-way or one-way Propagate, an None with base Some and bn unchanged gives DeleteB (plan.rs:155-165); Mirror deletes the orphan (95-99). The incremental path treats the same case as unmanaged (incremental_collect.rs:250, 257), and the duplicate path records filtered entries as omissions (snapshot_duplicates.rs:43-53; duplicate_plan.rs:31-46), which confirms the inconsistency. Age bounds are recomputed on every run (daemon job.rs:16; syncjobs types.rs:204-216), and copies carry later mtimes than their originals. So with a 'last N days' filter and scheduled runs, B's copies are deleted systematically in the window between the original aging out and the copy aging out.

## Y37 Case-insensitive pairs are keyed by exact case: case differences cause permanent drift errors, split baselines or delete/copy churn

- Schwere: high · Urteil: confirmed · Kategorie: platform · Plattformen: Windows, Android, Linux
- Stellen: native/src/bisync/core/plan.rs:106-118; native/src/bisync/os/shared/snapshot_pair.rs:62; native/src/bisync/core/omissions.rs:21-27; native/src/bisync/os/shared/apply_groups.rs:1-6; native/src/bisync/os/shared/apply_guard.rs:73-77; native/src/bisync/core/plan.rs:255-276

**Beschreibung.** Trees, plan and baseline use the exact-case relative path; fold_case only feeds omission matching, and apply_groups merely serializes actions whose paths fold equal. On NTFS/exFAT/FAT, SMB and Android shared storage a stat of one spelling finds the other, so ExpectedFile::Missing preconditions fail as drift.

**Fehlerszenario.** (a) First two-way sync of existing folders 'Photos' (A) and 'photos' (B): CopyAtoB('Photos/x.jpg') and CopyBtoA('photos/x.jpg') both expect Missing, both stats resolve the other spelling => drift errors every run (and the first finding freezes the baseline). (b) Case-only folder rename on A ('Photos'->'photos'), both sides case-insensitive: DeleteB then CopyAtoB writes into the still-existing 'Photos' folder; the re-walk records 'Photos/x.jpg': (None, B) and 'photos/x.jpg': (A, None); from then on every edit or delete of these files fails with drift; in Mirror mode each full run deletes and re-copies the folder. (c) 'Makefile' and 'makefile' on a Linux side synced to Windows/Android: the second copy fails with drift every run.

**Richtung.** When either side folds case, plan on a folded (and NFC-normalized) key, keep each side's actual spelling for I/O, map actions onto the existing destination spelling, apply case-only renames as renames, and report true collisions as conflicts/omissions instead of failing every run.

**Gegenprüfung.** Trees, plan and baseline are keyed by the exact-case rel (plan.rs:106-118). fold_case only feeds omissions (omissions.rs:21-27; snapshot_pair.rs:62), and apply_groups only serializes actions whose paths are equal after lowercasing (apply_groups.rs:1-6, 19). capture() with Expected Missing turns into drift when the stat succeeds (apply_guard.rs:73-77). LocalBackend never overrides case_sensitive_paths (default false, core.rs:288-290). (a) and (c) trace as described. For (b), deletions run first in the group, so DeleteB('Photos/x.jpg') succeeds, then CopyAtoB writes into the surviving 'Photos' folder. The re-walk records 'Photos/x.jpg' as (None, Sb) and 'photos/x.jpg' as (Sa, None), as claimed, and Mirror repeats the delete and re-copy on every full run. One nuance: a later A-side delete does not raise a drift error. 'photos/x.jpg' converges as (None,None) and 'Photos/x.jpg' stays unchanged, so the delete is silently not propagated. A-side edits and B-side edits or deletes do fail with drift.

## Y38 Count/Staggered/GFS version retention deletes the only recovery copy of most files

- Schwere: high · Urteil: confirmed · Kategorie: data-safety · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/apply_transfer.rs:281-288; native/src/bisync/os/shared/persistence.rs:344-352; native/src/bisync/os/shared/persistence.rs:377-421; native/src/bisync/core/types.rs:192-205; native/src/bisync/os/shared/tests/extra.rs:6-26

**Beschreibung.** Each backup is written to versions_<pair>/<current unix second>/<rel>, so one run creates one directory per second containing only the files backed up in that second. prune_versions treats each directory as a full snapshot: Count keeps the newest N directories, Staggered/GFS keep one directory per time bucket. The removed directories hold the only versions of other files.

**Fehlerszenario.** GFS: a run overwrites 2,000 files over 20 minutes => ~1,200 per-second directories; at the end of the same run gfs_bucket (age < 1 day => key 'h{hour}') keeps one directory per hour => the backups of ~1,990 files are deleted immediately. Count with retain_count=10 ('letzte N Versionen') keeps only the last 10 seconds of backups. The reversibility promised for every overwrite/delete is gone without any message.

**Richtung.** Apply retention per file path (keep the N newest versions of each rel, thin per rel), or write one directory per run and thin per rel; never remove a directory that holds a file's only version within the retention promise.

**Gegenprüfung.** back_up_captured writes to versions_dir/<now_secs+offset>/<rel> (apply_transfer.rs:281-300), so one run spreads its backups over many per-second directories. prune_versions treats each directory as a full snapshot. Count keeps the newest N directories (persistence.rs:344-351). Staggered keeps one directory per day once it is older than 24 h (397-406). GFS keeps one per hour even for age < 1 day (408-411), applied through keep_per_bucket (377-394). Pruning runs at the end of every successful run (orchestration.rs:400), so GFS deletes most backups of the same run immediately. The test only uses synthetic single-file directories (tests/extra.rs:6-26). The schemes are opt-in; the default is Days (types.rs:248-255).

## Y39 Incremental mirror never re-verifies the destination and has no destination identity outside Drive

- Schwere: high · Urteil: confirmed · Kategorie: data-safety · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/incremental.rs:96-105; native/src/bisync/os/shared/incremental.rs:177-179; native/src/bisync/os/shared/incremental.rs:422-427; native/src/bisync/os/shared/incremental_changes.rs:103-153; native/src/vfs/core/core.rs:365-368; native/src/bisync/os/shared/tests/incremental.rs:182-242

**Beschreibung.** After a clean full run the mirror switches to incremental mode, collecting only source changes; the destination is checked only at paths touched by those changes. root_id_matches returns true when no root id was stored, which is the case for every backend except Drive (default change_root_id = None, LocalBackend does not override it). Files removed, corrupted or never written on the destination are not repaired while the source stays unchanged, and runs report success.

**Fehlerszenario.** Two USB backup drives rotated weekly, both mounted as E:\Backup (or /media/user/BACKUP): after the first bootstrap each run copies only the source changes since the previous run, which went to the other drive => each drive silently misses the other week's new files and changes (a rebuild only happens if a touched path happens to differ). Files someone deletes on the NAS backup share are never restored.

**Richtung.** Store and verify a destination identity (marker file with the pair id, filesystem/volume id + root file id) before trusting the index; schedule periodic full verification of the destination (e.g. daily or every N runs); rebuild when the identity is missing or different.

**Gegenprüfung.** root_id_matches returns true when no id was saved (incremental.rs:422-426). Only Drive implements change_root_id (gdrive backend.rs:341-343; default None at core.rs:365-368), so bootstrap stores None for every other backend (incremental.rs:398-399). The target is checked only at managed changed paths (incremental.rs:177-179; incremental_changes.rs:103-153), and a test asserts that the target is never enumerated (tests/incremental.rs:231-235). Nuance: a touched rel whose target copy differs from the stored signature (a modified pre-existing file on a swapped drive, or a missing target) triggers a full rebuild. The rotation scenario is therefore silent only when the delta consists purely of additions, which is exactly the shape of an append-only photo backup. Destination-side deletions or corruption at untouched paths are never repaired, and the runs report success.

## Y40 One-way Propagate/NoDelete jobs never repair destination-side changes; a later source edit becomes a blocking conflict

- Schwere: medium · Urteil: confirmed · Kategorie: gap · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/core/plan.rs:169-180; native/src/bisync/core/plan.rs:181-188; native/src/bisync/core/plan.rs:224-233; native/src/bisync/core/plan.rs:245-280

**Beschreibung.** In direction A->B a change on B (deleted/modified/corrupted backup file) is 'B changed' and ignored; since the rel is neither applied nor converged its baseline entry stays and it is re-detected and ignored forever. When the source file is edited later both sides differ from the baseline => strict conflict (no backup update until resolved); with DestWins the policy picks B and, being one-way, silently does nothing.

**Fehlerszenario.** One-way backup A->B (Propagate). Someone deletes B/2024/tax.pdf: runs report ok and never restore it. A edits tax.pdf in 2025 => conflict; unattended daemon runs only log it, so the backup keeps lacking the current file.

**Richtung.** For one-way jobs treat destination drift as something to repair from the source (versioning the destination file) or at least report it as destination drift; do not block a source update with a conflict in one-way mode unless explicitly configured.

**Gegenprüfung.** With direction AtoB, a change on B gives (false,true), which acts only when allow_b_to_a (plan.rs:169-180). The rel is neither applied nor converged, so its baseline entry stays (plan.rs:245-279) and the change is re-detected and ignored on every run. A later source edit gives (true,true) -> FileLevel conflict (181-188). With DestWins, a_wins is false and the `else if allow_b_to_a` branch is false, so nothing happens and nothing is recorded (224-233). KeepBoth would resolve it, since a_wins is true for (Some, None).

## Y41 OlderWins/SmallerWins (and LargerWins for empty files) let a deletion win over a modification

- Schwere: medium · Urteil: confirmed · Kategorie: data-safety · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/core/plan.rs:8-13; native/src/bisync/core/plan.rs:203-233

**Beschreibung.** An absent side is mapped to mtime i64::MIN and size 0 before the policy is applied, so for delete-vs-modify conflicts the deleted side wins under OlderWins and SmallerWins.

**Fehlerszenario.** OlderWins: A deletes report.docx while B edits it => i64::MIN <= mtime(B) => A wins => DeleteB of the edited file; B deletes and A edits => DeleteA. SmallerWins: a deletion (size 0) always beats a non-empty modified file.

**Richtung.** Resolve delete-vs-modify independently of the size/mtime policies (keep the modified file or surface a conflict); apply those policies only when both sides exist.

**Gegenprüfung.** An absent side maps to mtime i64::MIN and size 0 (plan.rs:8-13). OlderWins (208) and SmallerWins (210) therefore pick the deleted side, and its None becomes DeleteB or DeleteA of the modified file (213-232). LargerWins only matters when A is the deleting side and B's modified file is empty (0 >= 0). The deleted file is backed up first (reversible), so it is recoverable.

## Y42 Post-apply observation re-walks whole trees; in Checksum mode it re-reads every file

- Schwere: medium · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/orchestration.rs:327-380; native/src/bisync/os/shared/snapshot_dir.rs:178-183; native/src/bisync/os/shared/snapshot_hash.rs:63-72

**Beschreibung.** Any run with a transfer or delete re-walks each touched side completely. With CompareMode::Checksum the re-walk uses HashMode::FullFresh, which never reuses hashes, so all files without a native hash are read (remote: downloaded) a second time.

**Fehlerszenario.** Two-way Checksum job local<->SFTP with 200 GB per side: the first walk reads 400 GB; one changed file triggers another full 400 GB read. On a 500k-file Drive tree every run with one change costs two full listings per side.

**Richtung.** Re-stat (and in Checksum mode re-hash) only the rels of completed actions and reuse first-walk signatures for everything else (also resolves the concurrent-edit finding).

**Gegenprüfung.** Any run with a_to_b, b_to_a or deleted > 0 fully re-walks each touched side (orchestration.rs:327-379). Checksum mode maps to FullFresh (snapshot_hash.rs:66). FullFresh ignores prev and reads every file that has no native hash (snapshot_dir.rs:178-183; content_hash 190-201). Nuance: an agent-backed SFTP side computes hashes server-side (snapshot.rs:178-183 -> snapshot_agent), so 'downloaded' applies to remotes without an agent.

## Y43 Drive change-feed incremental mirror falls back to a full rebuild for nearly every change

- Schwere: medium · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/gdrive/core/changes.rs:20-49; native/src/bisync/os/shared/incremental_collect.rs:156-159; native/src/bisync/os/shared/incremental_collect.rs:293-363; native/src/bisync/os/shared/state_validation.rs:173-181

**Beschreibung.** The feed covers the whole Drive (spaces=drive). A change is resolved via its parent id, which only works when the parent is the sync root because folders are never stored in the index (validate_item rejects is_dir), and id-based resolution is suppressed when parent+name are present. Any unresolved change returns Rebuild, so each run pays the change-feed pagination plus a full walk.

**Fehlerszenario.** Mirror Drive:/Photos -> local: a new photo in /Photos/2026, or editing any unrelated Google Doc elsewhere in the account, makes every run page through the feed and then do a full walk; the incremental path practically never applies.

**Richtung.** Index the folder ids of the synced subtree, ignore changes whose ancestry does not reach the root instead of rebuilding, and rebuild only for folder moves/renames inside the subtree.

**Gegenprüfung.** The feed is account-wide (changes.rs:25 spaces=drive), and every upsert carries parent_id and name (changes.rs:66-71). Because parent_id and name are present, id-based resolution is suppressed (incremental_collect.rs:312-315). The parent is looked up with rel_for_id over stored items (state_store.rs:339-352), and items are files only (state_validation.rs:175-181). So only direct children of the root resolve (incremental_collect.rs:351-352), and any other change, in a subfolder or outside the root, returns Rebuild (158) after the whole feed has been paged (changes.rs:23-48).

## Y44 A corrupt or locked sync_state.sqlite blocks every Mirror job

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/orchestration.rs:72-77; native/src/bisync/os/shared/incremental.rs:339-362; native/src/bisync/os/shared/state_store.rs:22-74; native/src/bisync/os/shared/state_validation.rs:223-230

**Beschreibung.** Before every full Mirror run the index must be invalidated. Open/init/load errors that are not 'untrusted record' conversions (SQLITE_CORRUPT, NOTADB, BUSY beyond the busy timeout, read-only data dir) abort the run, although the full scan itself does not need the index.

**Fehlerszenario.** The state DB is damaged (e.g. storage full or power loss) => every one-way Mirror job fails with 'Vollscan kann nicht sicher beginnen' until the user finds and deletes the file; a long bootstrap write transaction of one pair can make another concurrently starting Mirror job fail the same way.

**Richtung.** On open/integrity failure quarantine (rename) the DB, recreate it and continue with the full scan; wait/retry on BUSY.

**Gegenprüfung.** Every full one-way Mirror run without move first calls invalidate_incremental_state and aborts if it fails (orchestration.rs:72-77). open_store runs Connection::open plus PRAGMA/CREATE (state_store.rs:22-74), and its errors propagate (incremental.rs:348); load errors other than conversion errors also propagate (incremental.rs:360; state_validation.rs:223-230). A corrupt (NOTADB/CORRUPT) or unwritable database therefore blocks every such job until the file is removed. Nuance on the concurrency case: daemon jobs are globally serialized (job_supervisor.rs:32-33), and rusqlite sets a 5 s busy timeout by default (inner_connection.rs:118), so BUSY needs a second process such as the GUI's in-process run.

## Y45 Desktop conflict resolution overwrites a newer on-disk baseline; runs of one pair are not mutually excluded

- Schwere: medium · Urteil: confirmed · Kategorie: data-safety · Plattformen: Windows, Linux
- Stellen: native/src/app/core/bisync_ui.rs:238-240; native/src/app/core/bisync_conflicts.rs:333-353; native/src/app/core/sync_core.rs:192; native/src/daemon/os/shared/job.rs:77; native/src/bisync/os/shared/persistence.rs:171-219; native/src/mobile/os/shared/domains/sync_conflicts.rs:156-185

**Beschreibung.** The GUI keeps the run's Outcome.baseline in memory, applies resolutions to it and finally writes the whole map with save_baseline. Neither bisync::run nor the save takes a per-pair lock, so the daemon (scheduled/real-time) can run the same pair in between. The mobile path merges into the current file instead. Assumption: the daemon may run the same job while the GUI conflict dialog is open (no lock or IPC hand-off found in the code read).

**Fehlerszenario.** Conflict dialog left open; the daemon's real-time trigger syncs the pair (copies new files, propagates deletes) and saves its baseline; the user finishes resolving => the GUI writes the older baseline plus resolutions => entries recorded by the daemon run are lost => later deletions are resurrected and later edits become conflicts.

**Richtung.** Hold a cross-process per-pair lock for a run and for baseline saves; merge resolved entries into the current on-disk baseline (as the mobile path does) instead of writing a stale snapshot.

**Gegenprüfung.** The GUI keeps the run's baseline (bisync_ui.rs:238-240), inserts resolutions into it (bisync_conflicts.rs:292-295; bisync_merge.rs:85-86), and writes the whole map back (bisync_conflicts.rs:349). It runs bisync::run in-process (sync_core.rs:192), while the daemon runs jobs in its own process (job.rs:77). The daemon's serialization is process-local (job_supervisor.rs:32-33), its only lock is single-instance (linux platform.rs:9, 201), and save_baseline has no lock or compare-and-swap (persistence.rs:171-219). The reviewer's assumption therefore holds. The mobile path re-reads the stored baseline before merging (sync_conflicts.rs:168-170), but that is still an unlocked read-modify-write.

## Y46 FAT32 timestamps shift by one hour at DST changes and are treated as modifications

- Schwere: medium · Urteil: confirmed · Kategorie: platform · Plattformen: Windows
- Stellen: native/src/bisync/core/plan.rs:31-38; native/src/bisync/core/plan.rs:143-180

**Beschreibung.** Change detection is pure size+mtime per side with a tolerance that is not hour-aware. FAT stores local time and Windows converts it with the current DST bias (documented OS behavior), so all FAT file times move by 1 h at each DST switch.

**Fehlerszenario.** Two-way job with a FAT32 USB stick: after the DST switch every file on the stick is 'changed on B', A unchanged => CopyBtoA for the whole stick, each A file first copied into the versions store; twice a year.

**Richtung.** Detect exact whole-hour offsets with equal size on FAT volumes (an 'ignore time shift' option) or confirm with a content hash before propagating.

**Gegenprüfung.** Change detection compares size and mtime per side (plan.rs:143-144) with the window at plan.rs:37. The window defaults to 0, and the job field is documented for 1-2 s of FAT granularity (syncjobs types.rs:103-104, 174), not for a 3600 s shift. There is no hour-aware tolerance. FAT's local-time conversion with the current DST bias is documented Windows behavior outside the repository. After a DST switch every FAT file therefore looks changed on B -> CopyBtoA, with each A file backed up first.

## Y47 FTP modification times come from LIST rows and change precision as files age

- Schwere: medium · Urteil: confirmed · Kategorie: platform · Plattformen: Windows, Linux, Android
- Stellen: native/src/ftp/core/ftp.rs:74-95; native/src/bisync/core/plan.rs:143-180

**Beschreibung.** The FTP backend parses ls-style LIST rows. Standard servers print 'Mon DD HH:MM' (year inferred) for files younger than ~6 months and 'Mon DD YYYY' afterwards, and times are server-local without zone, so the parsed mtime of an unchanged file changes when it ages. Assumption: server uses the usual ls -l format; suppaftp's year inference was not verified.

**Fehlerszenario.** Two-way job local<->FTP: each FTP file crossing the 6-month boundary looks modified on B => CopyBtoA overwrites the local file with identical content (versioning it); around New Year an inferred year can be wrong.

**Richtung.** Use MLSD/MLST or MDTM (UTC seconds) when available; with LIST only, treat a coarsened timestamp that still matches at the coarser precision with equal size as unchanged.

**Gegenprüfung.** FTP metadata comes only from LIST rows parsed by suppaftp (ftp.rs:74-88); the module has no MLSD or MDTM (rg). The vendored suppaftp 6.3.0 (list.rs:454-473) appends Utc::now().year() to the 'Mon DD HH:MM' form, with no correction for future dates, and parses 'Mon DD YYYY' as 00:00. So the mtime of an unchanged file changes when the listing switches format at about 6 months. It also jumps by +1 year at every New Year for entries from the previous calendar year that are still shown with HH:MM. The issue is real and slightly worse than stated.

## Y48 No Unicode normalization of relative paths

- Schwere: medium · Urteil: confirmed · Kategorie: platform · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/core/plan.rs:106-118; native/src/bisync/core/omissions.rs:21-27; native/src/bisync/os/shared/apply_guard.rs:73-77

**Beschreibung.** Paths are compared byte-exact and nothing normalizes NFC/NFD. If one endpoint normalizes names (assumption: e.g. a Nextcloud server storing NFC), a decomposed name on the other side is treated as a different file.

**Fehlerszenario.** Linux folder with macOS-made NFD names ('Müller.pdf') two-way with Nextcloud: CopyAtoB stores it as NFC; next run the NFC name is a new B file => CopyBtoA creates a second, visually identical local file; later actions on the NFD rel resolve to the NFC file and fail with drift every run (baseline freeze via the first finding).

**Richtung.** Plan on an NFC (and when needed case-folded) key while keeping each side's spelling for I/O; report normalization collisions.

**Gegenprüfung.** The code part is verified. No Unicode normalization exists anywhere under bisync (rg). Keys are exact (plan.rs:106-118), omissions only lowercase (omissions.rs:21-27), and ValidatedRelativePath does not normalize (relative_path.rs:16-49). Expected Missing turns into drift when the stat succeeds (apply_guard.rs:73-77). The trigger depends on an endpoint that normalizes names (for example Nextcloud storing NFC), which cannot be verified from this repository; given such an endpoint, the trace (a duplicate local file, then drift on every later action) follows.

## Y49 Linux FIFOs/sockets/devices are synced as regular files; a FIFO blocks the run indefinitely

- Schwere: medium · Urteil: confirmed · Kategorie: platform · Plattformen: Linux
- Stellen: native/src/local_access/os/linux_os.rs:25-39; native/src/vfs/os/shared/local.rs:88-91; native/src/vfs/os/shared/local.rs:19-27; native/src/bisync/os/shared/apply_guard.rs:111-118; native/src/local_access/os/linux_os.rs:80-82; native/src/bisync/os/shared/snapshot_hash.rs:33; native/src/daemon/os/shared/job_supervisor.rs:32-33

**Beschreibung.** The Linux reader classifies special files as EntryKind::Other, but LocalBackend::list_dir only maps is_dir/is_link_like, so they become 0-byte regular files. Copy and hashing use a blocking File::open, which on a FIFO waits for a writer and cannot be cancelled; sockets fail with ENXIO every run.

**Fehlerszenario.** Home-folder backup with hidden files included (default) containing ~/.steam/steam.pipe (FIFO) => the copy worker blocks forever; with local<->Drive in MtimeSize mode the walk itself blocks while hashing; scheduled/real-time runs of the job never complete. A socket in the tree produces a permanent per-run error.

**Richtung.** Expose the entry kind in VfsMeta and record non-regular entries as protected omissions; open with O_NONBLOCK and verify S_ISREG via fstat before reading.

**Gegenprüfung.** The Linux reader classifies FIFOs, sockets and devices as EntryKind::Other with size 0 (linux_os.rs:25-38). LocalBackend::list_dir ignores the kind (local.rs:88-91), and stat maps the entry to is_dir=false and is_symlink=false (local.rs:19-27). The regular-file guard only rejects directories and links (apply_guard.rs:111-118). Copy and hashing open the file with a blocking File::open (linux_os.rs:80-82; apply_transfer.rs:343; snapshot_hash.rs:33). Cancellation is polled only between reads (apply_transfer.rs:388), so a blocked open never returns, and sockets fail with ENXIO on every run (#0). Impact is larger than stated: daemon jobs are globally serialized (job_supervisor.rs:32-33), so one hung job stalls every later daemon job, and cancel_and_join waits on its thread. Medium kept only because the trigger is uncommon.

## Y50 No rename/move detection: renamed trees are re-transferred and fully copied into the local versions store

- Schwere: medium · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/core/plan.rs:143-180; native/src/bisync/os/shared/apply.rs:138-162; native/src/bisync/os/shared/apply_delete.rs:84-92; native/src/bisync/os/shared/apply_transfer.rs:271-316

**Beschreibung.** A rename is planned as a copy of every new rel plus a delete of every old rel; each reversible delete first copies the old file into the local versions store (downloading it when that side is remote).

**Fehlerszenario.** Renaming 'Projects' (80 GB) to 'Projekte' on A in a two-way local<->NAS job uploads 80 GB again and downloads 80 GB into the local versions store before the old copies are deleted; with insufficient local space the deletes fail and the first finding freezes the baseline.

**Richtung.** Detect renames (deleted + added rels with equal size and hash/file id on one side) and apply a backend rename on the other side; a pure rename needs no version copy.

**Gegenprüfung.** The full planner never pairs deleted rels with added rels (plan.rs:106-238). Each DeleteA or DeleteB goes through delete_guarded (apply.rs:138-162), which first backs the file up into the local versions dir (apply_delete.rs:84-95; apply_transfer.rs:281-300); jobs are always reversible. A remote old copy is therefore read over the network before deletion. Only the Drive change-feed incremental mirror handles renames by id (incremental_changes.rs:36-40).

## Y51 Empty directories are never synchronized or removed

- Schwere: low · Urteil: confirmed · Kategorie: gap · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/snapshot_dir.rs:131-134; native/src/bisync/core/types.rs:14-15; native/src/bisync/os/shared/apply_delete.rs:114

**Beschreibung.** Snapshots contain files only; directories are just traversed, and deletes remove single files.

**Fehlerszenario.** Deleting a folder tree on A leaves its empty folder skeleton on B forever (also after case-only renames); empty folders are never backed up.

**Richtung.** Track directories in the snapshot, or prune directories emptied by the run when they no longer exist on the other side.

**Gegenprüfung.** A Tree maps rel to a file Sig (types.rs:14-15). Directories are only queued for walking (snapshot_dir.rs:131-134), deletes call remove_file_id (apply_delete.rs:114), and rg finds no remove_dir and no empty-directory creation anywhere in the bisync apply path.

## Y52 mtime difference in sig_eq can overflow

- Schwere: low · Urteil: confirmed · Kategorie: bug · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/core/plan.rs:37

**Beschreibung.** `(a.mtime_ms - b.mtime_ms).abs()` overflows for extreme values; release builds (no overflow-checks) wrap and abs() of i64::MIN stays negative, debug builds panic.

**Fehlerszenario.** A backend or peer reports mtime i64::MIN for a file whose other signature has mtime 0 => difference wraps to i64::MIN => `<= window` is true => a real change is treated as equal.

**Richtung.** Use `a.mtime_ms.abs_diff(b.mtime_ms) <= opts.modify_window_ms.max(0) as u64`.

**Gegenprüfung.** plan.rs:37 computes (a.mtime_ms - b.mtime_ms).abs(). The release profile does not enable overflow-checks (Cargo.toml:213-224), so the result wraps, and i64::MIN.abs() stays i64::MIN, which is <= window and compares as equal; a debug build panics. The mtime has to be extreme, for example a raw i64 decoded from a peer or agent frame (agent_proto codec.rs:125 has no range check).

## Y53 Incremental bootstrap marks the pair bootstrapped before its items are written

- Schwere: low · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/incremental.rs:391-406; native/src/bisync/os/shared/state_store.rs:122-154; native/src/bisync/os/shared/state_store.rs:211-267

**Beschreibung.** save_pair(bootstrapped=true, new cursor) and replace_from_baseline are separate transactions.

**Fehlerszenario.** replace_from_baseline fails after save_pair (disk full, BUSY, item budget) => pair marked bootstrapped with the new cursor but old items; the next incremental run plans against stale items and rewrites the legacy baseline from them (the touched-path drift check catches most target mismatches; delete-guard totals and the baseline file stay stale).

**Richtung.** Write items and the pair record in one transaction and set bootstrapped last.

**Gegenprüfung.** save_pair(bootstrapped=true, new cursor) and replace_from_baseline run in separate transactions (incremental.rs:403-406; state_store.rs:122-154, 211-267), and the result is discarded (orchestration.rs:94). The touched-path drift check (incremental_changes.rs:103-153) catches most of the resulting mismatches, so low is right.

## Y54 Versions store reuses remote names verbatim on the local filesystem (Windows)

- Schwere: low · Urteil: confirmed · Kategorie: platform · Plattformen: Windows
- Stellen: native/src/bisync/os/shared/apply_transfer.rs:281-300; native/src/bisync/os/shared/apply_transfer.rs:74-89; native/src/bisync/os/shared/apply_delete.rs:84-92

**Beschreibung.** Backups are written to versions_dir/<second>/<rel> on the local disk; a rel component that is illegal there cannot be created, and overwrite/delete must not proceed without the backup.

**Fehlerszenario.** Windows desktop syncing Drive <-> SFTP (remote<->remote): a remote file named 'Agenda?.txt' or 'aux.txt' cannot be backed up under %APPDATA% => every overwrite/delete of it fails before commit, every run, freezing the baseline via the first finding.

**Richtung.** Encode rel components for the local versions store (escape illegal characters/reserved names, keep a manifest) independently of the remote names.

**Gegenprüfung.** Backups go to versions_dir/<ts>/<rel> through create_dir_all and create_new on the local filesystem (apply_transfer.rs:286-299). A failure there aborts the overwrite (76-88) and the delete (apply_delete.rs:84-95) as pre-commit errors. Only Windows-hosted remote<->remote pairs are affected: a local Windows side could not hold such a name anyway.

## Y55 One failed file or a cancel discards the whole run's progress; the next run turns every file the run transferred into a spurious conflict (or a full Mirror re-copy)

- Schwere: critical · Urteil: confirmed · Kategorie: data-safety · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/incremental_changes.rs:103-153

**Beschreibung.** run_full collects report.completed (documented in apply_pool.rs:41 as the actions that may enter a new baseline) but returns the OLD, unsaved baseline whenever errors is non-empty or the run was canceled; the incremental mirror does the same. Because copies never preserve the source mtime (see the mtime finding) and hashless pairs compare by mtime, the next run sees both sides changed relative to the stale baseline with different mtimes. The planner then emits a FileLevel conflict (job default) for each such rel regardless of direction; Mirror re-copies instead. Errors that trigger this are routine on live data: a source file that vanished or changed after the walk is a drift error (apply_guard), locked files, transient network errors with the default retries=0.

**Fehlerszenario.** One-way A->B daily backup with defaults (FileLevel, MtimeSize, local -> USB/SFTP/SMB). Run N copies 5,000 changed files; one browser-cache file disappears before its copy (drift error). Nothing is saved. Run N+1: all 5,000 files are 'changed on both sides' with differing mtimes -> 5,000 conflicts, not applied; documents edited again on A are no longer backed up until the user resolves them; on a live home directory almost every run has some drift error, so the conflict set grows with each run. The same happens after Stop, after the daemon stops for an app update (job_supervisor cancel_and_join), and on Android whenever WorkManager stops the SyncWorker (task.cancel). Incremental mirror: error -> previous state -> touched targets drifted -> full rebuild -> full re-copy.

**Richtung.** Always persist update_baseline(base, after-trees, report.completed, converged, conflicts) for the completed actions (failed rels already keep their previous entries), also on cancel (stat only the completed paths instead of a full re-walk). Classify pre-commit drift (source vanished/changed since planning) as 'skipped, retried next run' rather than a run error. Save item updates for completed upserts/deletes in the incremental path. Optionally auto-converge both-changed rels whose content is identical (size + on-demand hash) to heal baselines already damaged.

**Gegenprüfung.** orchestration.rs:301-320 returns `baseline: base` on cancel and on any error; save_baseline runs only at 386-387 after update_baseline(...,&report.completed,...) at 384. Nothing in the sync paths sets a file time (no set_modified/set_times/setstat/MFMT in production code; the Backend trait in vfs/core/core.rs has no such method), and hash_mode gives HashMode::None for MtimeSize unless a side provides_content_hash (snapshot_hash.rs:63-72; only gdrive backend.rs:328 and webdav.rs:470). On the next run a_changed (stale base) and b_changed (new copy) are both true and sig_eq(an,bn) fails on mtime (plan.rs:37; modify_window 0 by default, syncjobs/core/types.rs:174), so plan.rs:181-188 emits a FileLevel conflict for any direction (default conflict FileLevel, types.rs:155). No code auto-resolves identical-content conflicts. Drift is a pre-commit InvalidData error (apply_guard.rs:73-77) that is never transient (apply_retry.rs:78-95), and retries default to 0 (types.rs:188). tests/safety.rs:280-303 and 341-353 assert the intended contract (completed actions enter nb, no retry conflict), which orchestration does not implement. Incremental: incremental.rs:206-213 and 232-238 return previous_baseline. The next run trips target_touched_drifted (incremental_changes.rs:103-153 → incremental.rs:177-178) and falls back to a full Mirror run, which re-copies every file (plan.rs:77-94). Cancel sources are confirmed (job_supervisor.rs:119-135, SyncWorker.kt:78-83). Only pairs with content hashes on both sides (local↔Drive/Nextcloud) are immune. Critical stands: this hits the default config, routine errors and cancels trigger it, and every affected file stops syncing until it is resolved by hand.

## Y56 Version pruning (Count, Staggered, GFS) treats each per-second backup folder as a snapshot and deletes most recovery copies right after the run

- Schwere: high · Urteil: confirmed · Kategorie: data-safety · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/apply_transfer.rs:281-300; native/src/bisync/os/shared/duplicate_backup.rs:13-16; native/src/bisync/os/shared/persistence.rs:302-355; native/src/bisync/os/shared/persistence.rs:377-420; native/src/bisync/os/shared/orchestration.rs:400; native/src/bisync/core/types.rs:196-205; native/src/bisync/core/types.rs:225-231; native/src/app/core/job_editor_ui.rs:154-156

**Beschreibung.** Each backed-up file is stored under versions/<current unix second (+offset)>/<rel>, so one run spreads its backups over many directories, each holding only the files backed up in that second. prune_versions treats every such directory as a complete snapshot: Count keeps the newest N directories overall, GFS keeps one directory per hour for the first day, Staggered one per day after 24 h. All other directories, which hold the only copies of the other overwritten/deleted files, are removed. The UI promises 'Nach Anzahl (letzte N Versionen)' / 'Anzahl der neuesten Versions-Schnappschüsse'.

**Fehlerszenario.** Job with GFS versioning; a run deletes or overwrites 2,000 files over 10 minutes -> about 600 second-directories. prune_versions at the end of the same run keeps only the newest directory of that hour, so the recovery copies of ~99% of the files are deleted immediately. With Count=10 only files backed up in the last 10 seconds survive; with Staggered nearly everything is gone after 24 h. A propagated accidental deletion or a ransomware-encrypted overwrite can then not be restored.

**Richtung.** Make a snapshot equal one run: allocate one run-scoped versions directory (run start + run id) and place every backup of the run in it; or prune per rel (keep the newest N versions of each file, bucket per file). Never prune the current run's directory.

**Gegenprüfung.** back_up_captured takes the current second for each call and writes versions/<ts+offset>/<rel> (apply_transfer.rs:281-296); duplicate_backup.rs:13-16 does the same. prune_versions treats every numeric directory as one snapshot. Count keeps the newest N directories (persistence.rs:344-351). Gfs keeps only the newest directory per h{ts/3600} bucket when younger than 1 day (408-411 via keep_per_bucket 377-395). Staggered keeps one d{day} directory after 24 h (397-405). The prune runs at the end of each successful full run (orchestration.rs:400). So a run whose backups span many seconds loses all but the newest second-directory per hour (GFS), or all but N seconds of backups (Count), right after the run. The only test (tests/extra.rs prune_count_keeps_newest_n) checks directory semantics only. Severity lowered to high: the default scheme Days (SyncJob::new types.rs:175, editor.rs:113) only removes directories older than N days and is unaffected, and the loss hits recovery copies rather than live data.

## Y57 Copies never preserve the source mtime; the stateless Mirror planner then re-copies and re-backs-up every file on every full run of hashless pairs, and remote sources without a change feed always run full

- Schwere: critical · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/snapshot_dir.rs:112-127

**Beschreibung.** Neither the bisync stage nor the one-way mirror sets the destination file time, so the destination mtime is the copy time. DeletePolicy::Mirror planning ignores the baseline and compares source vs destination with sig_eq; without content hashes on both sides (only Drive and Nextcloud-WebDAV provide them) MtimeSize requires equal mtimes, which never holds for files this tool copied. Steady state depends on the incremental index, which is used only when bootstrapped after an error-, conflict- and omission-free full run, and only for local sources or sources with a change feed.

**Fehlerszenario.** (a) Mirror job SFTP/FTP/SMB/WebDAV NAS -> local disk: every run returns None at incremental.rs:138, the full planner emits CopyAtoB for every file, and each existing destination file is first copied into the versions dir and then overwritten: a 500 GB tree is re-downloaded and 500 GB of versions written per run. (b) Local -> USB Mirror of a home dir containing one symlink: omissions are never empty, the index never bootstraps, every run re-copies everything. (c) After any failed incremental run the next run rebuilds and re-copies everything.

**Richtung.** Set the source mtime on the stage before promotion (std set_modified, SFTP setstat, FTP MFMT, WebDAV X-OC-MTime/PROPPATCH, Drive modifiedTime) and record the actual destination signature; in Mirror full runs treat a rel as in sync when both sides equal their baseline signatures instead of requiring src/dst mtime equality; allow incremental bootstrap when omissions exist (they are preserved separately).

**Gegenprüfung.** The copy paths never set a time: stage_source (apply_transfer.rs:337-371), copy_stream (sync_copy.rs:28-106), LocalBackend::open_write = File::create (local.rs:121-125), and promotion.rs has no time handling. The Mirror planner ignores the baseline and copies whenever !sig_eq (plan.rs:59-101). With MtimeSize and no hashes (snapshot_hash.rs:63-72) the copy-time mtime on the destination never equals the source mtime, so every file gets CopyAtoB. Each such copy first backs up the existing destination file into versions (apply.rs:52, apply_transfer.rs:76-88). The incremental index applies only to change-feed sources (only Drive supports_changes) or is_local sources (incremental.rs:116-138), and only after a clean bootstrap (orchestration.rs:72, 88-95). One symlink or the Android app folders create omissions (snapshot_dir.rs:112-127), so the index never bootstraps. tests.rs:355-360 documents the re-transfer, and the links.rs tests deliberately use CompareMode::Checksum. Nuance on (c): the rebuild happens only when a copy in the failed incremental run had already changed a touched target. Critical stands: every run re-downloads the whole tree, and full-size version copies pile up for 30 days (Days) on the app-data volume.

## Y58 Baseline records the post-run re-walk, so edits made during a run are marked as synced and never propagated

- Schwere: high · Urteil: confirmed · Kategorie: data-safety · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/orchestration.rs:321-384; native/src/bisync/core/plan.rs:245-279; native/src/bisync/os/shared/apply_transfer.rs:25-48

**Beschreibung.** For two-way runs both sides are re-walked after apply and update_baseline records (a.get(rel), b.get(rel)) from these new trees for every completed and converged rel. A change made to such a file after it was planned/copied but before the re-walk reaches it becomes the new baseline state although the other side never received it. The same applies to a destination-side edit or deletion during the run. The full second walk of both trees after every changing run is also a large cost for big or remote trees.

**Fehlerszenario.** Real-time two-way job: user saves report.docx (v1), the job copies v1 A->B; 20 s later, while the run still applies other files or re-walks, the user saves v2. The re-walk records A=v2 and B=copy of v1 as the baseline. Following runs see both sides unchanged -> v2 is never synced; a later edit on B is copied B->A and replaces v2 (only a versions copy remains). A deletion on B during the run is absorbed the same way (neither re-copied nor propagated).

**Richtung.** Record for each completed action the source signature that was actually copied (planned sig verified by capture/revalidate) and the destination signature from a stat of the promoted file (return it from copy_replace); record planned sigs for converged rels. If a later observation differs, keep the planned sig so the next run detects the change. This also allows replacing the full re-walk by stats of the touched paths.

**Gegenprüfung.** For Direction::Both, a_touched and b_touched are true (orchestration.rs:335-336). Both sides are re-walked (337-379), and update_baseline records the re-walked (a,b) signatures for every completed or converged rel (plan.rs:255-276; called at orchestration.rs:384). Nothing compares the re-walk with the signature that was planned or copied, and copy_replace returns only the byte count (apply_transfer.rs:25-48). An edit or deletion between the copy and the re-walk becomes the baseline; both sides then look unchanged and the edit is never propagated. One-way runs reuse the planning tree for the untouched source (at reused when !a_touched), so source-side loss affects two-way, move and repair runs. High is right.

## Y59 Move jobs: a failed source deletion becomes a permanent conflict under the default compare instead of a FinalizeMove retry

- Schwere: medium · Urteil: confirmed · Kategorie: bug · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/resolve.rs:123-141

**Beschreibung.** FinalizeMove is planned only when sig_eq(source, destination). Because copies do not preserve mtime and local/SFTP/FTP/SMB have no hashes, sig_eq is false under MtimeSize (job default), so the rel falls through to the normal planner: both sides changed vs the unsaved baseline with different mtimes -> FileLevel conflict (job default). The only test for this path switches to CompareMode::SizeOnly with a comment that the timestamps differ.

**Fehlerszenario.** Move job SD card -> NAS (one-way). One photo's source delete fails (file in use, read-only medium, transient error). Next run: a conflict instead of FinalizeMove; the photo stays on both sides, the move never completes until resolved manually (Mirror+move re-copies it instead).

**Richtung.** For move jobs plan FinalizeMove whenever the destination exists with the same size (or equal hash) and let verify_and_delete_source (which already compares full content) decide; or persist completed copies in the baseline (first finding) and plan FinalizeMove from it.

**Gegenprüfung.** FinalizeMove needs sig_eq(an,bn) (plan.rs:132-139). With hashless MtimeSize the copy-time mtime differs, so the rel falls through to plan.rs:143-188. The failed action never enters completed, so the old baseline entry (or none) remains, and the result is (true,true): a FileLevel conflict. Mirror+move re-copies via plan.rs:77-94. tests/move_retry.rs:131-137 switches to SizeOnly and admits the timestamps differ. The move also never completes after the user resolves the conflict: resolve.rs:123-141 re-copies (new mtime) and the baseline records both new signatures. The next run sees an≠bn by mtime and neither side changed against the baseline, so it hits `continue` and the source is never deleted. Severity lowered to medium: no data is lost (the file stays on both sides) and the trigger is a failed source delete.

## Y60 Local staged copies are published by rename without fsync while the baseline is fsynced; after power loss/unplug a truncated destination is recorded as synced and, in two-way mode, propagated back

- Schwere: medium · Urteil: confirmed · Kategorie: data-safety · Plattformen: Linux, Windows
- Stellen: native/src/bisync/os/shared/apply_transfer.rs:342-353; native/src/bisync/os/shared/apply_transfer.rs:133-137; native/src/sync/os/shared/sync_copy.rs:52-67; native/src/vfs/os/shared/local.rs:121-125; native/src/vfs/core/promotion.rs:111-130; native/src/bisync/os/shared/persistence.rs:191-192; native/src/transfer/os/shared/engine/publish.rs:70-73; native/src/vfs/core/core.rs:107-111

**Beschreibung.** stage_source writes via LocalBackend::open_write (std::fs::File::create), calls flush() (a no-op for File), drops the file and renames it over/into the destination; neither the stage nor the parent directory is ever synced. save_baseline syncs its file before its rename. The transfer engine documents 'sync keeps its durable stages', but the sync paths never use a durable stage writer (only SFTP's writer is durable). Assumption: filesystem behavior (XFS/NTFS/FAT/exFAT do not order data before rename; ext4 auto_da_alloc covers only replace-over-existing).

**Fehlerszenario.** Two-way job laptop <-> USB drive. A run copies X A->B and saves the (synced) baseline; power loss or unplugging within the writeback window leaves B/X zero-length or garbage. Next run: B/X differs from its baseline signature, A/X unchanged -> CopyBtoA writes the truncated content over the good A/X (the original only survives in the versions dir).

**Richtung.** Create local stages exclusively, sync_all() the stage before promotion and fsync the parent directory after the rename (also for version backups); count the action as completed only afterwards. Use a durable sized stage writer (open_write_copy_stage_sized) for all backends.

**Gegenprüfung.** stage_source writes through destination.open_write (apply_transfer.rs:344); File::flush is a no-op, the writer is dropped and the stage is renamed by promote (133-137). Local open_write is File::create (local.rs:121-125). Neither apply_transfer.rs nor sync_copy.rs calls sync_all on the stage or fsyncs the directory; the only sync_all calls are the versions backup (apply_transfer.rs:308) and the baseline (persistence.rs:192). The 'durable stage' comments (publish.rs:70-73, core.rs:107-111) are also false for local: open_write_copy_stage_sized → open_write_new, with no fsync. SFTP open_write is Commit::Durable (sftp backend.rs:250-252). Whether the crash scenario happens depends on filesystem writeback ordering, which the code cannot decide. The overwritten A version survives in versions because reversible is forced on. Medium rather than high.

## Y61 Reversible backups always go to the app-data volume, require full downloads of remote files, and are never pruned on incremental runs, error runs or after a prune error

- Schwere: high · Urteil: confirmed · Kategorie: data-safety · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/persistence.rs:55-61; native/src/support_dirs.rs:99-103; native/src/syncjobs/core/types.rs:242; native/src/bisync/os/shared/apply.rs:52; native/src/bisync/os/shared/apply_delete.rs:84-95; native/src/bisync/os/shared/apply_transfer.rs:271-322; native/src/bisync/os/shared/move_finalize.rs:40-42; native/src/bisync/os/shared/orchestration.rs:386-407; native/src/bisync/os/shared/orchestration.rs:88-95; native/src/bisync/os/shared/incremental.rs:190-330

**Beschreibung.** checked_opts hard-codes reversible: true for every job. Every overwrite, every delete (including Mirror orphans) and every move source is first copied in full into app_data/sync/versions_<pair> (system drive on desktop, internal storage on Android), downloading remote files completely. prune_versions has a single caller at the end of an error-free full run; incremental mirror runs never prune; a prune error is pushed into errors, which in turn blocks incremental bootstrap.

**Fehlerszenario.** Local 256 GB SSD -> 4 TB USB Mirror; the user deletes a 300 GB folder on A. The run copies 300 GB from the USB drive into AppData before deleting each file -> the system disk fills, further backups and other programs fail; versions are never pruned because the run ends with errors. Android: a Mirror job deleting old files on a NAS downloads them into internal storage until it is full. Incremental mirror jobs grow the versions dir without bound.

**Richtung.** Keep versions on the same volume/remote as the affected file (e.g. a hidden per-root versions folder) and move files there with a rename/server-side move instead of downloading; make reversibility configurable per job; prune after every run including incremental and failed runs, report prune problems without blocking bootstrap; enforce a space budget.

**Gegenprüfung.** versions_dir = sync_data_dir()/versions_<pair> (persistence.rs:59-61; support_dirs.rs:99-103 → %APPDATA%, ~/.local/share or the Android files dir). reversible: true is hard-coded (syncjobs/core/types.rs:242). back_up_captured streams the full content via open_read_id into a local file (apply_transfer.rs:292-308), which downloads remote files. Every delete is backed up first (apply_delete.rs:84-95), and so is the move source (move_finalize.rs:40-42). There is no free-space check. prune_versions has a single caller after a successful save (orchestration.rs:400), and the incremental path (incremental.rs:190-335) never prunes. A prune error is pushed (400-407) and blocks bootstrap (88-95). Nuance: a prune error does not stop later attempts; the next clean full run retries. High stands.

## Y62 Delete safety guard is off by default and there is no empty/unmounted-root check

- Schwere: high · Urteil: confirmed · Kategorie: data-safety · Plattformen: Windows, Linux, Android
- Stellen: native/src/syncjobs/core/types.rs:178-179; native/src/syncjobs/os/shared/editor.rs:116-117; native/src/bisync/os/shared/orchestration.rs:182-234; native/src/bisync/core/plan.rs:155-180

**Beschreibung.** max_delete and max_delete_pct default to 0 ('no limit') for new jobs on desktop (SyncJob::new, JobEditor). The guard is the only protection against a side that reads as empty (unmounted mount point that still exists as an empty directory, emptied cloud folder, wrong root after a remount); run_full has no check of side emptiness or root identity against the baseline.

**Fehlerszenario.** Linux /mnt/backup (fstab nofail) is not mounted, the empty directory remains. Two-way job: every file looks deleted on B -> DeleteA for all of A (each copied into the versions dir first, possibly pruned by the pruning defect). Mirror A->B with A unmounted: everything on B is deleted.

**Richtung.** Enable the guard by default for new jobs (percentage and absolute), refuse to apply when a side with baseline entries lists completely empty, and verify a per-root identity (volume id or marker file).

**Gegenprüfung.** The defaults are 0 in SyncJob::new (types.rs:178-179), the desktop editor (editor.rs:116-117) and Android (SyncApi.kt:76-77), and 0 maps to u64::MAX (orchestration.rs:210-219). validate_sync_roots only rejects empty root strings and overlap (vfs/os/shared/sync_roots.rs:10-35). An existing but empty mount point walks as an empty tree. Two-way: an==ba, bn None vs bb Some → (false,true) → DeleteA (plan.rs:169-178); Mirror gives DeleteB (plan.rs:95-99). Each delete is backed up into versions first. High stands.

## Y63 Duplicate-name repair or dedupe failure aborts the whole sync before any file is applied

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/orchestration.rs:149-175; native/src/bisync/os/shared/orchestration.rs:236-269; native/src/bisync/os/shared/duplicate_apply.rs:40-97

**Beschreibung.** run_full first plans/applies the dedupe plan (Mirror) and resolves every duplicate repair; any single error returns immediately with the old baseline. resolve() has no retry and backs up all variants before acting.

**Fehlerszenario.** Drive <-> local job: one duplicate pair lies in a folder shared read-only with the user, remove_file_id fails with permission denied; every run stops at 'Duplikatbereinigung' and none of the other 100,000 files are synced again.

**Richtung.** Handle repair/dedupe failures per rel: record the error, exclude that rel and its variants from the plan, continue the run; keep all-or-nothing only for the deletion-count guard.

**Gegenprüfung.** run_full returns before any apply on a dedupe preflight error (orchestration.rs:162-175), an apply_dedupe_plan error (238-251) or the first failing repair (260-266); apply starts only at 282. Repairs exist only for duplicate-name providers with exactly one shared content (duplicate_plan.rs:55-66). resolve backs up all variants first and has no retry (duplicate_apply.rs:58-64, 80-83). Severity lowered to medium: this is Drive-specific, the error is visible in the result, and an ignore pattern works around it.

## Y64 Real-time triggers are dropped while the job runs or within 60 s of its last start; all jobs share one global slot

- Schwere: high · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/syncjobs/core/schedule.rs:36-63; native/src/daemon/os/shared/state.rs:7

**Beschreibung.** enqueue returns AlreadyScheduled while the job is active/pending and RecentlyAttempted within 60 s of its last admission; the run loop ignores both and clears the job's dirty mark anyway. Nothing remembers that a change arrived during a run. Jobs are serialized globally (one active job, FIFO), so a real-time job waits behind any long backup.

**Fehlerszenario.** Real-time job with 10 s debounce: the user saves twice within a minute. The first save starts a run; the second settles 10 s later -> RecentlyAttempted -> dropped, dirty mark removed -> the second save is not synced until some later change. Combined with the re-walk baseline defect, an edit made during a run is lost entirely. A 3-hour NAS backup delays every real-time job by 3 hours.

**Richtung.** In the supervisor keep a 'run again after completion' flag for an active job and defer RecentlyAttempted triggers to the end of the cooldown instead of dropping them; clear the dirty mark only on Started/Queued; consider per-pair instead of global serialization.

**Gegenprüfung.** enqueue returns AlreadyScheduled, or RecentlyAttempted within 60 s of admission (job_supervisor.rs:9, 66-79). enqueue_job ignores both (run_loop.rs:433), and enqueue_realtime_jobs drops the dirty mark unconditionally (288-291). RealTime jobs have no timer fallback (syncjobs/core/schedule.rs:36-63 returns false). With the default 15 s tick (daemon state.rs:7) and 10 s debounce, a change that settles within about 60 s of the last admission is lost until the next change. The run's own writes to a local side also produce such a dropped cycle. All jobs share one active slot with a FIFO queue (job_supervisor.rs:32-37, 83-88). High stands.

## Y65 Sync writes use plain open_write: FTP/WebDAV/Drive spool every file completely to a local temp file, FTP uploads serialize on the browsing connection

- Schwere: medium · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/apply_transfer.rs:344; native/src/sync/os/shared/sync_copy.rs:52; native/src/ftp/core/ftp.rs:254-259; native/src/ftp/core/writer.rs:15-73; native/src/webdav/core/webdav.rs:291-316; native/src/webdav/core/writer.rs:81-87; native/src/gdrive/core/transfer.rs:306-315; native/src/vfs/core/core.rs:95-111; native/src/bisync/os/shared/duplicate_apply.rs:108

**Beschreibung.** stage_source and copy_stream open the stage with open_write, which on FTP, WebDAV and Drive writes the whole file into tempfile::tempfile() and uploads only in flush(); FTP's writer uploads through pool.primary() under with_stream_mutation, so all parallel apply workers serialize on the browsing connection. The streaming sized-stage writers used by the transfer engine and by duplicate_apply::publish are not used. The bandwidth throttle paces only the spool fill; the upload in flush() is unthrottled. open_write is a non-exclusive create, unlike the exclusive stage writers.

**Fehlerszenario.** Backup of a 40 GB VM image to Nextcloud or an FTP NAS: 40 GB are written to /tmp (often tmpfs/RAM), %TEMP% on C: or the Android app cache before the first byte is uploaded; several parallel large files exhaust temp space and all fail; FTP throughput is one file at a time whatever the flow limit allows.

**Richtung.** Use open_write_copy_stage_sized(&staged, size) (durable variant for sync) in stage_source and copy_stream as the transfer engine does; apply the throttle to the actual network writes.

**Gegenprüfung.** Sync uses open_write (apply_transfer.rs:344, sync_copy.rs:52). On FTP, open_write = FtpWriter on pool.primary() (ftp.rs:254-259): it spools to tempfile::tempfile() and uploads in flush through with_stream_mutation (writer.rs:15-21, 40-73), which holds the primary stream (io_adapters.rs:106-121). WebDAV spools (webdav.rs:291-298, writer.rs:79-131), and so does Drive (transfer.rs:306-315). The streaming sized stages exist but sync does not use them (ftp.rs:276-290, webdav.rs:309-316). The throttle only paces the spool fill (stream()). Severity lowered to medium: correctness is preserved and the cost is throughput, plus failures only when temp space is smaller than the file(s).

## Y66 FTP destinations: every copied file lists its parent folder about 6-7 times, making large folders quadratic

- Schwere: high · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/apply_transfer.rs:63-141; native/src/bisync/os/shared/apply_transfer.rs:338-361; native/src/bisync/os/shared/apply_guard.rs:121-131; native/src/ftp/core/ftp.rs:211-226; native/src/ftp/core/ftp.rs:145-150; native/src/ftp/core/staging.rs:91-104; native/src/vfs/core/promotion.rs:24-36; native/src/sync/os/shared/sync_copy.rs:39-96

**Beschreibung.** On FTP stat = LIST of the parent, try_exists = stat, publish = LIST of the parent. copy_replace per file: capture destination, [backup revalidate], revalidate, unique_staging_path probe, capture of the stage, revalidate again, publish - each a full parent listing - plus mkdir_all of the parent for every file (PWD/MKD/CWD per path level on the primary connection) although prepare_folder already ensured it. The one-way mirror needs about 3 listings per file (+2 source stats on FTP sources). The scan was fixed for this, the apply path was not.

**Fehlerszenario.** 20,000 photos in one FTP folder on a router/NAS: about 140,000 LISTs of a 20,000-entry folder; the backup takes days instead of minutes (timing is an estimate; listing cost depends on the server).

**Richtung.** Keep a per-run listing cache of each destination folder (updated after each publish), trust the stage writer's checked flush instead of re-listing, skip mkdir_all when the FolderRegister already ensured the folder, use MLST/SIZE/MDTM where the server supports them.

**Gegenprüfung.** FTP stat lists the parent (ftp.rs:211-226), try_exists defaults to stat (core.rs:50-56), and publish lists the parent (staging.rs:91-127); all of these go through the primary connection (ftp.rs:198-203, 145-150). copy_replace runs these per file: capture the destination (apply_transfer.rs:63-74), backup revalidate (309), revalidate (100-106), the unique_staging_path probe (promotion.rs:34), capture the stage (358), revalidate (120-125) and publish (133-137). That is 6-7 parent LISTs, plus mkdir_all (339; ftp.rs:313-339 runs PWD/MKD/CWD per level). The one-way mirror needs 3 LISTs (sync_copy.rs:39, 77, 93-97) plus 2 source stats. High stands for FTP destinations.

## Y67 Hard 1,000,000-entry caps make large backups permanently incomplete (one-way mirror) or impossible (jobs)

- Schwere: high · Urteil: confirmed · Kategorie: gap · Plattformen: Windows, Linux, Android
- Stellen: native/src/sync/os/shared/sync.rs:35-60; native/src/sync/os/shared/sync_tasks.rs:217-234; native/src/bisync/os/shared/snapshot.rs:15-16; native/src/bisync/os/shared/snapshot_dir.rs:97-104; native/src/bisync/os/shared/persistence.rs:10-12; native/src/bisync/os/shared/state_validation.rs:8-9

**Beschreibung.** The one-way mirror counts every source entry against 1M nodes / 128 MiB path text; when exceeded, discovery stops (queued folders dropped) and one error is recorded - on every run, although the streaming copy pass never holds the tree in memory. Two-way/one-way jobs fail the whole run at the same limit (walk; the bisync walk belongs to the scan dimension but decides whether apply can run), and the baseline and incremental store are capped at 1M entries.

**Fehlerszenario.** 'Spiegeln nach...' of a 1.3M-file home directory to a USB disk copies the first ~1M entries in listing order and never reaches the rest (one error line). A scheduled job over the same tree fails every run with 'sync tree exceeds its bounded collection budget' and transfers nothing.

**Richtung.** Drop the node cap from the streaming mirror pass (keep depth/loop protection); for jobs, move trees/baseline to the existing disk-backed state store or size limits from available memory.

**Gegenprüfung.** The one-way mirror has a 1M-node / 128 MiB budget (sync.rs:35-60). When it runs out, within_budget sets scan_stopped and clears the queued dirs (sync_tasks.rs:217-234), even though the copy pass streams (FILE_QUEUE_LIMIT note, sync_tasks.rs:31-35). The bisync walk aborts at the same limit (snapshot_dir.rs:97-105, snapshot.rs:15-16). The baseline is capped at 1M (persistence.rs:11, 221-224) and so is the state store (state_validation.rs:8). Minor nuance: each in-flight scanner can add an error line, so it is not exactly one. High stands.

## Y68 Size/age filters are evaluated per side: a file leaving the window on one side looks deleted, or its counterpart's absence becomes a permanent conflict/drift error

- Schwere: high · Urteil: confirmed · Kategorie: data-safety · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/core/omissions.rs:70-76

**Beschreibung.** Cross-dimension (planning + apply guard). Each side's walk drops entries by that side's own size/mtime; the filtered tree becomes the apply expectation. Because copies do not preserve mtime, the two sides' mtimes differ. A source file that grows beyond max_size, or whose source mtime ages out while the destination copy (newer copy time) is still inside the window, is planned as deleted on the source -> DeleteB. A destination copy that is outside the window while the source changed is planned as delete-vs-modify conflict (FileLevel) or as a copy whose guard fails with 'copy destination changed since planning' on every run.

**Fehlerszenario.** Backup job 'only files <= 100 MB': a video grows to 120 MB on A -> its backup on B is deleted (moved to versions, later pruned). New job 'only files changed in the last 30 days' over an existing folder: the next day every file that crossed the 30-day mark on A is deleted from B because B's copy time is still recent.

**Richtung.** Decide filters per rel for the pair (one-way: by the source; two-way: if either side is filtered, exclude the rel on both sides) and never derive a deletion from an entry that was only filtered.

**Gegenprüfung.** Each side filters by its own size and mtime (snapshot_dir.rs:135, size_age_ok snapshot.rs:51-65). Filtered entries are not omissions, so their baseline entries stay in the planning baseline (omissions.rs:70-76). A file on A that grows past max_size or ages out gives an=None, bn=Some, base (Some,Some) → DeleteB (plan.rs:156-166; Mirror plan.rs:95-99). Because mtime is not preserved, B's copy stays inside the age window. A destination outside the window becomes ExpectedFile::Missing (apply_guard.rs:15-19) while the file is present, which is a drift error on every run (73-77) or a FileLevel conflict. The incremental mirror handles a file leaving the window correctly (incremental_collect.rs:248-259 marks it unmanaged), so the defect is in the full planner. High stands.

## Y69 Delete safety stop is reported with the cancel marker 'abgebrochen': job treated as canceled, last_run not updated, scheduled job re-runs (full walk) every 60 s

- Schwere: medium · Urteil: confirmed · Kategorie: bug · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/orchestration.rs:220-234; native/src/bisync/os/shared/incremental.rs:183-189; native/src/daemon/os/shared/job.rs:78-79; native/src/daemon/os/shared/job.rs:114-128; native/src/daemon/os/shared/job_supervisor.rs:73-79; native/src/syncjobs/core/schedule.rs:36-61; native/src/daemon/os/shared/run_loop.rs:177-178; native/src/mobile/os/shared/domains/sync_run.rs:187; native/src/mobile/os/shared/domains/sync_run.rs:222-224

**Beschreibung.** Both safety stops use the error kind 'abgebrochen'. The job runner treats any such error as cancellation: note 'abgebrochen', no after-command, persist_attempt(..., mark_run=false). Interval/Calendar jobs therefore stay due and are re-admitted every 60 s. Android returns the task as canceled.

**Fehlerszenario.** An hourly NAS job would delete 2,000 files with max_delete=1000: every minute the daemon reconnects, walks both trees completely and stops again, indefinitely; the result shows 'abgebrochen', so the user does not learn that a mass deletion was blocked.

**Richtung.** Use a distinct kind/result state for the safety stop, record it as a blocked run with its reason (mark the attempt), and do not auto-repeat until the tree changes or the user confirms.

**Gegenprüfung.** Both safety stops use the error kind "abgebrochen" (orchestration.rs:223, incremental.rs:185). job.rs:78-79 treats that as a cancel: note abgebrochen, no after-command, persist_attempt(...,false) (128), so last_run is not updated. due() (schedule.rs:41-58) keeps Interval and Calendar jobs due; the daemon enqueues them every tick (run_loop.rs:177-178) and the 60 s cooldown (job_supervisor.rs:73-79) lets them through again, each time with a full walk. JobResult stores only counts and the note, so the safety-stop text is lost (job.rs:105-113). The Android facade reports the task as canceled (sync_run.rs:187, 222-224). Medium stands.

## Y70 Directories are never created or removed by the two-way/mirror engine; folder->file replacement fails forever

- Schwere: medium · Urteil: confirmed · Kategorie: gap · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/apply_guard.rs:111-131; native/src/bisync/os/shared/apply_delete.rs:79-83; native/src/bisync/os/shared/apply_groups.rs:1-6; native/src/bisync/core/types.rs:120-155

**Beschreibung.** Trees contain only files; DeleteA/B remove files; nothing removes the emptied folder or creates empty source folders. When a folder is replaced by a file of the same name, the deletions of its files run first (apply_groups) but the now-empty directory stays, and copy_replace's capture rejects a directory at the destination path.

**Fehlerszenario.** User replaces folder notes/ by a file notes on A: every run fails on notes ('copy destination is not a regular file'), B keeps an empty notes/ dir, and the error blocks the baseline (first finding). Mirror jobs ('Ziel exakt angleichen') leave empty skeletons of every deleted folder on the destination; empty folders on A are never backed up.

**Richtung.** Plan directory creation/removal (remove emptied, non-omitted directories bottom-up after file deletions in the same group); in copy_replace remove an empty directory the plan deleted before publishing the file.

**Gegenprüfung.** Trees hold files only (snapshot_dir.rs:131-146), and deletes remove files (apply_delete.rs:106-116). The bisync apply path has no remove_dir or create_dir; mkdir_all creates only file parents (apply_transfer.rs:338-339). apply_groups only orders deletions first (apply_groups.rs:1-6, 52-55). When a folder becomes a file, the emptied directory stays, and the copy's capture → current_metadata → regular() returns InvalidData 'copy destination is not a regular file' on every run (apply_guard.rs:111-131). Medium stands.

## Y71 Leftover staging files after a crash are treated as user files and synced

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/vfs/core/promotion.rs:24-36; native/src/bisync/os/shared/apply_transfer.rs:341; native/src/sync/os/shared/sync_copy.rs:39; native/src/bisync/os/shared/snapshot_dir.rs:106-130; native/src/sync/os/shared/sync_scan.rs:124-162; native/src/gdrive/core/transfer.rs:295

**Beschreibung.** Stages are created next to the target as '<name>.se-bisync-<16 hex>' (mirror '.se-sync-', repairs '.se-bisync-variant-'). After a kill, crash or power loss they remain. Neither the bisync walk nor the mirror scan excludes these names (only the Drive backend recognizes them), and no sweep removes stale stages.

**Fehlerszenario.** The daemon is killed during the upload of a 20 GB file to B. Next two-way run: the partial 'video.mkv.se-bisync-1f..' on B is a new file -> copied to A; both sides now keep a broken junk file. Mirror runs delete it only after first copying it into the versions dir.

**Richtung.** Exclude the engine's stage-name pattern in walks and scans, and remove stale stages (older than the run start) in touched folders.

**Gegenprüfung.** Stage names are built at promotion.rs:30-33. The bisync walk excludes only hidden or ignored entries, links and the app trash (snapshot_dir.rs:106-130); the mirror scan does the same (sync_scan.rs:142-162). Drive's is_internal_staging_path (gdrive/core/transfer.rs:295-303) is used only for upload-ID reservation (transfer.rs:33), not for listing. No stale-stage sweep exists. Nuance: FTP, WebDAV and Drive upload only at flush, so a kill leaves no remote partial there; local and SFTP stages do persist. Medium stands.

## Y72 Staging and conflict suffixes push long file names over the 255 limit; such files can never be synced

- Schwere: medium · Urteil: confirmed · Kategorie: bug · Plattformen: Windows, Linux, Android
- Stellen: native/src/vfs/core/promotion.rs:30-33; native/src/bisync/os/shared/apply_transfer.rs:341; native/src/bisync/os/shared/apply_transfer.rs:441-453; native/src/sync/os/shared/sync_copy.rs:39

**Beschreibung.** The stage name appends 27 characters ('.se-bisync-' + 16 hex; mirror 25) to the full file name; the conflict copy appends ' (Konflikt YYYYMMDD-HHMMSS)' (27+). Names that fit the component limit (255 bytes on ext4, 255 UTF-16 units on NTFS, 143 on eCryptfs) but not with the suffix fail at stage creation on every run and block the baseline.

**Fehlerszenario.** A 240-character PDF title, or an 80-character CJK name (240 bytes on ext4), on A cannot be copied to B ever; a KeepBoth conflict on such a file can never be resolved.

**Richtung.** Use short fixed-length stage names (e.g. '.se-<16 hex>.tmp' in the same folder, mapping kept in memory) and truncate conflict names to fit the component limit.

**Gegenprüfung.** The stage name appends '.se-' + purpose + '-' + 16 hex (promotion.rs:30-33): 27 characters for bisync and 25 for sync. The conflict suffix inserts 27 characters (apply_transfer.rs:441-453). For an over-long candidate, try_exists fails with a non-NotFound error, so unique_staging_path fails and the copy fails pre-commit on every run, which also blocks the baseline (finding 0). Medium stands: such names are rare, but finding 0 amplifies the effect.

## Y73 Windows: files in use are never backed up and transient sharing violations are never retried

- Schwere: medium · Urteil: confirmed · Kategorie: platform · Plattformen: Windows
- Stellen: native/src/bisync/os/shared/apply_retry.rs:34-38; native/src/bisync/os/shared/apply_retry.rs:78-95; native/src/bisync/os/shared/apply_transfer.rs:133-141; native/src/local_access/os/windows/read.rs:54-69; native/src/syncjobs/core/types.rs:188

**Beschreibung.** There is no VSS/shadow-copy reading for locked sources; sharing/lock violations are not among the retried transient kinds; any promote failure is classified commit_attempted and never retried, although a failed local rename (MoveFileExW refused because antivirus, the indexer, OneDrive or an open document holds the target) publishes nothing; retries default to 0.

**Fehlerszenario.** Daily backup of a Windows profile: Outlook .pst, browser profile databases and running VM disks fail on every run (and block the baseline, first finding); Defender briefly scanning a just-written file makes the rename fail -> error instead of a short retry.

**Richtung.** Read locked local sources from a VSS snapshot (helper) or report them as a distinct 'in use, skipped' result; treat sharing/lock violations as transient with short backoff; classify a failed local rename as pre-commit so it is retried.

**Gegenprüfung.** native/src has no VSS code. Windows open_read uses File::open and falls back only on PermissionDenied (local_access/os/windows/read.rs:54-69). is_transient has no sharing or lock violation (apply_retry.rs:78-95), and retryable requires PreCommit (34-38). A failed promote is classified commit_attempted (apply_transfer.rs:138-141) even though a refused local rename publishes nothing. retries default to 0 (types.rs:188). Medium stands.

## Y74 No run-level breaker or 'target refuses' stop in sync apply; retries sleep while holding flow permits

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/apply_pool.rs:187-307; native/src/bisync/os/shared/apply.rs:372-387; native/src/bisync/os/shared/apply_retry.rs:56-114; native/src/sync/os/shared/sync_tasks.rs:42-163; native/src/transfer/core/engine_policy.rs:131-149; native/src/transfer/os/shared/engine/worker.rs:208-210; native/src/transfer/os/shared/engine/worker.rs:267-273

**Beschreibung.** The transfer engine ends a job after consecutive connection failures and when the target refuses files (full/read-only). The sync apply pool and the mirror copy pass have neither, so every remaining action is attempted. run_with_retry waits between retries inside execute, i.e. while the action still holds its flow permits.

**Fehlerszenario.** NAS switched off 10 minutes into a 50,000-file backup: each remaining file goes through its own connection attempts/timeouts and overload backoff (up to 5 minutes patience) - hours of futile work. With a full destination disk, every remaining overwrite still first copies the old file into the versions dir and then fails with ENOSPC.

**Richtung.** Reuse the engine's breaker and target-refusal classification in apply_pool and the mirror pass: end the run (persisting completed work) after N consecutive connection failures or on StorageFull/read-only; release permits before retry delays.

**Gegenprüfung.** The pool's record() only counts errors (apply_pool.rs:287-307); there is no breaker and no target-refuses stop, unlike the transfer engine (engine_policy.rs:131-149, worker.rs:207-210, 263-273). run_with_retry inside execute (apply.rs:372-387) sleeps (apply_retry.rs:56-71) while admit's permits are held (apply_pool.rs:251-258), but only when retries>0. Nuance: overload backoff has a run-level idle limit (sync_overload.rs:77-84), so overload waits stop after 5 minutes without progress, but every remaining action is still attempted. Medium stands.

## Y75 Destination backup is made before staging and repeated on every retry and overload repeat

- Schwere: low · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/apply_transfer.rs:76-119; native/src/bisync/os/shared/apply.rs:373-386; native/src/bisync/os/shared/apply_pool.rs:240-277; native/src/bisync/os/shared/apply_delete.rs:84-116

**Beschreibung.** copy_replace copies the existing destination into versions first and stages the new content afterwards; run_with_retry and the pool's overload loop re-run the whole action, so each attempt makes another full backup (re-downloading a remote destination). A delete whose removal hits overload backs up again on each repeat.

**Fehlerszenario.** WebDAV server answering 503 under load: a 2 GB file being replaced is re-downloaded for backup on every repeat, adding load to the overloaded server and several identical copies to the versions dir.

**Richtung.** Stage first and back up immediately before promotion; reuse a backup already made in this run for the same captured destination identity.

**Gegenprüfung.** The backup is made before staging (apply_transfer.rs:76-119). run_with_retry and the pool's overload loop repeat the whole action (apply.rs:373; apply_pool.rs:245-276; repeatable() at 101-103 allows non-KeepBoth actions even after a commit attempt), and each repeat makes a new backup. Deletes back up before remove (apply_delete.rs:84-116). Severity lowered to low: this only happens with retries or overload and causes extra traffic and duplicate versions, not wrong results.

## Y76 Move finalization reads both files completely and additionally backs up the verified source

- Schwere: medium · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/move_finalize.rs:22-61; native/src/bisync/os/shared/move_finalize.rs:85-111; native/src/bisync/os/shared/apply_transfer.rs:251-268; native/src/syncjobs/core/types.rs:242

**Beschreibung.** verify_and_delete_source compares full contents (full download of a remote destination) and, because jobs force reversible=true, then copies the source into the versions dir before deleting it, although an identical verified copy exists. back_up is called without a cancel flag.

**Fehlerszenario.** Moving 200 GB of camera footage from local storage to a NAS: 200 GB upload + 200 GB re-download for comparison + 200 GB written into AppData/internal storage; Stop does not interrupt the source backup of a large file.

**Richtung.** Skip the source backup once the destination is byte-verified; compare by hash where the destination provides one (or verify the uploaded stage); pass the cancel flag to back_up.

**Gegenprüfung.** content_equal reads both files completely (move_finalize.rs:24-39, 85-111), which downloads a remote destination in full. Because reversible is forced true, the verified source is then backed up too (40-42) via back_up, which passes cancel None (apply_transfer.rs:251-268), so Stop cannot interrupt that backup. Medium stands.

## Y77 Renames/moves in full runs are executed as delete + full re-transfer (with a full versions copy of each 'deleted' file)

- Schwere: medium · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/core/plan.rs:106-238; native/src/bisync/os/shared/apply_groups.rs:12-57; native/src/bisync/os/shared/incremental.rs:240-258

**Beschreibung.** The full planner has no rename detection (same size/hash/file id at a new path); a renamed folder becomes DeleteB for every old path (each backed up into versions) plus CopyAtoB for every new path, run interleaved rather than copy-first. Only the change-feed incremental path handles old_rel.

**Fehlerszenario.** Renaming 'Fotos' to 'Bilder' (150 GB) on A: 150 GB re-uploaded and 150 GB from B copied into the local versions dir; if canceled midway B holds neither tree completely.

**Richtung.** Detect moves (size + hash, or file id/inode where available) and apply them as renames on the destination; otherwise run copies before deletes as the incremental path does.

**Gegenprüfung.** The full planner has no rename detection (plan.rs:106-238). apply_groups only groups identical or nested paths (apply_groups.rs:12-57). old_rel is handled only on the incremental path (incremental.rs:240-258; incremental_changes.rs:36-40). Each DeleteB backs up first. Medium stands.

## Y78 Job results under-report and skip failures: capped error count, unreachable endpoints/invalid config never recorded, failed scheduled runs not retried

- Schwere: medium · Urteil: confirmed · Kategorie: gap · Plattformen: Windows, Linux, Android
- Stellen: native/src/daemon/os/shared/job.rs:16-42; native/src/daemon/os/shared/job.rs:105-128; native/src/bisync/os/shared/apply_pool.rs:25; native/src/bisync/os/shared/apply.rs:395-405; native/src/mobile/os/shared/domains/sync_run.rs:197-208; native/src/syncjobs/core/types.rs:77-78

**Beschreibung.** JobResult.errors is out.errors.len() (at most 101: 100 messages + one summary) instead of stats.errors. Endpoint resolution and configuration failures only write the daemon log and return - no record_result, so the UI keeps showing the previous result. Runs with errors are marked as run (last_run is documented as last successful run), so a failed Calendar backup is not retried before the next occurrence. Android marks canceled runs as run (sync_run.rs:198) while the daemon does not.

**Fehlerszenario.** Nightly backup to a NAS that is switched off: nothing is recorded for weeks and the job list still shows last week's 'ok'. A run with 12,000 failed files shows '101 Fehler'.

**Richtung.** Record a result for every attempt (including 'Ziel nicht erreichbar' and invalid configuration), store stats.errors, keep separate last_attempt/last_success, and retry failed scheduled runs with backoff.

**Gegenprüfung.** Configuration and endpoint failures only log and return (job.rs:16-42). errors is set to out.errors.len() (job.rs:111), which apply caps at 100 plus one summary (apply_pool.rs:25, 300-304; apply.rs:395-405), although out.stats.errors has the real count. Runs with errors are marked as run (job.rs:128) although last_run is documented as the last successful run (types.rs:77). The Android facade marks the run unconditionally (sync_run.rs:198) and also uses errors.len() (207). Additional effect: a job whose endpoint never resolves stays due and is retried about every 60 s. Medium stands.

## Y79 No cross-process lock on a sync pair: GUI, Android facade and daemon can run the same job concurrently

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/app/core/sync_core.rs:118-140; native/src/app/os/shared/sync_jobs.rs:84-94; native/src/mobile/os/shared/domains/sync_run.rs:38-47; native/src/daemon/os/shared/job_supervisor.rs:66-89; native/src/bisync/os/shared/persistence.rs:171-217

**Beschreibung.** The desktop GUI runs saved jobs in-process and only checks its own flags; the Android facade only checks its own RUNNING map; the daemon supervisor only its own queue. Two bisync::run calls on the same pair apply the same plan concurrently and both save the baseline (last writer wins).

**Fehlerszenario.** A real-time daemon run of a job is in progress when the user presses 'Jetzt' for the same job: both copy/delete the same files, the guards turn the collisions into drift errors (so neither saves its baseline, first finding), transfers are duplicated, and the later run may overwrite the baseline of the other.

**Richtung.** Take an exclusive per-pair lock file in sync_data_dir for the whole run (and for conflict resolution) inside bisync::run, and report 'läuft bereits' to the second caller.

**Gegenprüfung.** The GUI checks only its own flags (sync_core.rs:130-143, sync_jobs.rs:6-12) and runs bisync in-process (sync_core.rs:181-194). The Android facade's claim() uses a process-local map (sync_run.rs:26-47). The daemon supervisor only sees its own queue (job_supervisor.rs:66-89). rg finds no pair lock in bisync, and save_baseline replaces the file unconditionally (persistence.rs:171-219). Medium stands.

## Y80 One-way mirror decides by 'source newer than destination copy time' across two clocks; same-size updates are skipped

- Schwere: medium · Urteil: confirmed · Kategorie: bug · Plattformen: Windows, Linux, Android
- Stellen: native/src/sync/os/shared/sync_scan.rs:62-75; native/src/sync/os/shared/sync.rs:5-8; native/src/sync/os/shared/sync_copy.rs:28-100

**Beschreibung.** Copy happens only if sizes differ or src.mtime > dst.mtime. Since the mirror does not preserve mtime, dst.mtime is the destination clock at copy time. A same-size change whose source mtime is not newer (destination/NAS clock ahead, FAT local-time offsets, apps preserving timestamps such as VeraCrypt containers, files restored from archives or downloads with server Last-Modified) is skipped and counted as 'übersprungen'.

**Fehlerszenario.** NAS clock 10 minutes ahead: a fixed-size database or container file modified 5 minutes after a mirror run is skipped by the next run; the 'backup' silently keeps the old content.

**Richtung.** Preserve the source mtime on copy and compare equality with a tolerance window, or keep per-pair state of what was copied as bisync does.

**Gegenprüfung.** decide() copies only when the size differs or src.mtime > dst.mtime (sync_scan.rs:62-75). Because copy_stream never sets times (sync_copy.rs:45-106), dst.mtime is the copy time on the destination's clock. A same-size update with an older or skewed source mtime is therefore skipped. Medium stands.

## Y81 Copies drop source permissions: private files become world-readable at the destination and in the versions dir

- Schwere: medium · Urteil: partially_confirmed · Kategorie: security · Plattformen: Linux, Windows
- Stellen: native/src/bisync/os/shared/apply_transfer.rs:292-296; native/src/bisync/os/shared/apply_transfer.rs:344; native/src/sync/os/shared/sync_copy.rs:52; native/src/vfs/os/shared/local.rs:121-125; native/src/vfs/os/shared/local.rs:134-145

**Beschreibung.** Stages are created with File::create and version backups with OpenOptions::create_new, i.e. mode 0666 & umask (typically 0644); the source's mode bits, ACLs and xattrs are not applied. LocalBackend::copy_file preserves permissions, but the sync paths do not use it. On SFTP destinations the server umask applies.

**Fehlerszenario.** Syncing ~/.ssh or a 0700 private folder to a multi-user Linux machine or a shared NAS via SFTP: the copies (and the copies in ~/.local/share/.../versions_*) are readable by every local user.

**Richtung.** Create stages and version files with 0600 and apply the source permission bits (set_permissions / SFTP setstat) before publication; document ACL/xattr handling.

**Gegenprüfung.** Linux is confirmed. Stages come from File::create (local.rs:121-125) and version files from OpenOptions create_new without a mode (apply_transfer.rs:292-296), so both get 0666&umask; created directories get 0777&umask (local.rs:257-268). LocalBackend::copy_file preserves permissions (local.rs:134-151) but is not on the sync path. The Windows part is not a separate defect: new files inherit the destination folder's ACL, which is normal copy semantics. Whether the versions directory is exposed depends on the home-directory mode.

## Y82 Type conflicts and Google-native documents cause permanent errors or endless re-copies in the mirror and move paths

- Schwere: medium · Urteil: confirmed · Kategorie: bug · Plattformen: Windows, Linux, Android
- Stellen: native/src/gdrive/core/backend.rs:131-160; native/src/sync/os/shared/sync_scan.rs:319-329

**Beschreibung.** One-way mirror: a source file whose destination is a directory (or a source folder whose destination is a file) is a permanent error; the delete pass would be the only remedy, but it runs only after an error-free pass and both production callers pass delete_extra=false. Google-native Docs are listed with size 0 but exported with N bytes: copy_stream fails every run with 'source length changed', move finalization keeps failing on the size check, and the stateless Mirror planner re-copies them on every run.

**Fehlerszenario.** 'Spiegeln nach...' of a Drive folder with Google Docs always ends 'Spiegelung unvollständig'; a move job from Drive never finalizes Docs; a Mirror job re-exports all Docs every run. A renamed-to-folder path in the mirror source errors forever.

**Richtung.** Resolve type conflicts in the mirror (replace the conflicting destination entry after backing it up, or report once as omission); use read_size()/download_name() for exported documents in copy_stream, move finalization and Mirror comparison (as stage_source already does via size_is_authoritative).

**Gegenprüfung.** In the mirror, type mismatches are permanent errors (sync_scan.rs:66-69 and 319-329), the delete pass is gated (sync.rs:271-290) and both callers pass delete_extra:false (sync_core.rs:16-20, sync_run.rs:294-297). Google Docs are listed with size 0 (gdrive/core/metadata.rs:49) and exported on read (backend.rs:131-140, 150-160). copy_stream compares the copied length against the listed size unconditionally (sync_copy.rs:71-76). The bisync copy succeeds because the size is not authoritative (apply_transfer.rs:466-472), but the Mirror sig_eq then fails on 0 vs N (plan.rs:20-22), so the Doc is re-exported every run. The move size check fails forever (move_finalize.rs:24). Medium stands.

## Y83 Bandwidth limit overshoots and does not cover backups, verification reads and spooled uploads

- Schwere: low · Urteil: confirmed · Kategorie: bug · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/core/types.rs:324-360; native/src/bisync/os/shared/apply_transfer.rs:13; native/src/bisync/os/shared/apply_transfer.rs:302-303; native/src/bisync/os/shared/apply_transfer.rs:428-434; native/src/bisync/os/shared/move_finalize.rs:94-110

**Beschreibung.** Throttle::consume adds whole 256 KiB blocks, sleeps to the end of the window when the budget is exceeded and then resets used to 0, discarding the excess; limits below the block size are exceeded by up to ~4x, others by up to one block per second. Backups, verify_expected_content, move comparison and the network upload of spooling writers (flush) are not throttled at all.

**Fehlerszenario.** User sets 64 KiB/s to keep a slow uplink usable: a local -> SFTP run transfers about 256 KiB/s; uploads to WebDAV/FTP/Drive burst at line rate in flush().

**Richtung.** Carry the excess into the next window (token bucket with debt) or consume in sub-block slices; throttle all transfer reads/writes including backups and the real upload.

**Gegenprüfung.** Throttle::consume adds the whole block, sleeps to the end of the window and resets used to 0 (types.rs:333-358). With COPY_BUFFER at 256 KiB (apply_transfer.rs:13), a 64 KiB/s limit runs at about 4x. Backups (303), verify_expected_content (428-434) and the move compare (move_finalize.rs:85-111) are not throttled, and spooled uploads happen in flush outside the throttle. Low stands.

## Y84 KeepBoth conflict copies are impossible on FTP and a failed keep-both creates an additional conflict copy every run

- Schwere: low · Urteil: confirmed · Kategorie: gap · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/apply_transfer.rs:195-236; native/src/bisync/os/shared/apply.rs:164-221; native/src/ftp/core/ftp.rs:291-299; native/src/vfs/core/promotion.rs:54-66

**Beschreibung.** copy_conflict_sibling_at publishes the sibling with rename_no_replace, which FTP deliberately leaves unsupported, although FTP implements promote_staged_no_replace (the documented exception). If the sibling is published but the following copy fails, the conflict remains and the next run publishes another '(Konflikt ...)' copy.

**Fehlerszenario.** Two-way job local <-> FTP with 'Beide behalten': every conflict fails with Unsupported on every run; on other backends a repeatedly failing overwrite accumulates one conflict copy per run.

**Richtung.** Publish conflict copies through promote_staged_create (supports FTP), and record a published sibling so the next run does not create another.

**Gegenprüfung.** The conflict sibling is published with rename_no_replace (apply_transfer.rs:201). FTP keeps the Unsupported default (core.rs:184-191; ftp.rs:291-299) and the collision probe returns false, so after a full stage upload the action fails as commit_attempted on every run. A published sibling followed by a failed copy becomes commit_attempted (apply.rs:182-184); the pool does not repeat it (apply_pool.rs:101-103), but the next run re-plans it with a new timestamp (apply_transfer.rs:154). Low stands.

## Y85 'Sichere Kopien' (atomic_copy) job option has no effect

- Schwere: low · Urteil: confirmed · Kategorie: gap · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/core/types.rs:284-285; native/src/syncjobs/core/types.rs:265; native/src/app/core/job_editor_ui.rs:295

**Beschreibung.** BisyncOptions::atomic is set from the job's atomic_copy checkbox but never read by the apply code; copies are always staged.

**Fehlerszenario.** A user disables 'Sichere Kopien' expecting direct writes (e.g. to save space on a nearly full target); nothing changes.

**Richtung.** Remove the option from the UI or document that copies are always staged.

**Gegenprüfung.** BisyncOptions::atomic (types.rs:284-285) is set from atomic_copy (syncjobs/core/types.rs:265). rg finds no reader in bisync/ or sync/, and stage_source always stages. Low stands.

## Y86 Linux: no-replace publish uses only renameat2(RENAME_NOREPLACE); every NEW file fails on NFS and FUSE mounts

- Schwere: high · Urteil: confirmed · Kategorie: platform · Plattformen: Linux
- Stellen: native/src/vfs/os/linux_os/local_platform.rs:41-75; native/src/copy/os/linux_os.rs:44-56; native/src/copy/os/linux_os.rs:108-140; native/src/vfs/core/promotion.rs:68-76; native/src/vfs/core/promotion.rs:111-119; native/src/bisync/os/shared/persistence.rs:213-214; native/src/copy/os/shared/move_guard.rs:44-49; native/src/copy/os/shared/copy.rs:58-66; native/src/android_fs/os/rename.rs:58-87

**Beschreibung.** LocalBackend::rename_no_replace and the copy module's commit_staged/move_file (non-overwrite) call the raw renameat2 syscall with RENAME_NOREPLACE and deliberately propagate every error. All publications of a new file go through it: promote_staged_create, promote_staged_with when the destination is absent, bisync conflict siblings, copy-dialog no-replace commits, folder moves and move quarantine. Linux NFS rejects any rename flag with EINVAL, and FUSE returns EINVAL when the daemon lacks FUSE_RENAME2 (sshfs rejects flags != 0; libfuse2-based daemons such as ntfs-3g negotiate minor < 23). LocalBackend nevertheless advertises StagedWriteCapabilities::complete(). The project already has the right fallback chain (link+unlink, then documented checked rename) but only compiles it for Android.

**Fehlerszenario.** Linux desktop, bisync or quick mirror of ~/Photos to an NFS-mounted NAS (/mnt/nas/backup), an sshfs mount or an NTFS USB disk mounted through ntfs-3g: every file that does not yet exist on the target fails with 'Invalid argument (os error 22)'; only replacements (std::fs::rename) succeed. A first backup copies nothing, bisync never saves its baseline (see mtime/baseline finding), copy-dialog copies into such mounts fail for every file unless 'overwrite' is chosen.

**Richtung.** Use one shared Linux/Android no-replace adapter: renameat2(RENAME_NOREPLACE); on EINVAL/ENOSYS/EOPNOTSUPP fall back to linkat(src,dst) (atomic create-only on NFS, ntfs-3g, sshfs with hardlink extension) + unlink(src); only as a last resort the documented checked rename (or a clear refusal), like systemd's rename_noreplace(). Report staged-write capability per mount (statfs f_type) instead of complete().

**Gegenprüfung.** Confirmed. vfs/os/linux_os/local_platform.rs:41-75 calls the raw SYS_renameat2 with RENAME_NOREPLACE and returns every error unchanged; lines 57-59 explicitly rule out a fallback. LocalBackend::rename_no_replace forwards to it (vfs/os/shared/local.rs:155-157) and advertises StagedWriteCapabilities::complete() (161-163). LocalBackend does not override promote_staged_no_replace, so promotion.rs:68-76 calls rename_no_replace, and promote_staged_with sends every absent destination there (promotion.rs:117-118). None of the callers handles EINVAL: sync_copy.rs:93-104 (quick-mirror creates), apply_transfer.rs:133-141 (bisync creates), apply_transfer.rs:201-219 (conflict siblings; any error other than AlreadyExists or a collision is final), and copy/os/linux_os.rs:44-56/108-140. The copy-module path is used by safe_file.rs:201 (copies without overwrite), move_guard.rs:49 (the quarantine step of every Move) and copy.rs:63 (folder moves). The link-then-checked-rename chain is compiled only for target_os=android (local_platform.rs:79-85 -> android_fs/os/rename.rs:58-87). The cited kernel behaviour matches upstream: nfs_rename rejects any flag; fuse_rename2 returns EINVAL when no_rename2 is set or the protocol minor is below 23; sshfs rejects flags. Additional impact not in the finding: save_baseline publishes the first baseline via LocalBackend promote_staged_replace -> promote_staged_with -> rename_no_replace (bisync/os/shared/persistence.rs:213-214). The store lives in ~/.local/share/smart_explorer/sync (support_dirs.rs:73-79, 99-103). With an NFS-mounted home, the first baseline can therefore never be written, and every run falls into the conflict cascade of #3. agent_proto/os/linux_os/local_platform.rs:41-60 (writes by Share peers to a Linux host) makes the same raw call. NFS and sshfs are affected unconditionally. NTFS USB disks are affected only where the distro mounts them with ntfs-3g (FUSE) and not with the in-kernel ntfs3 driver, which supports NOREPLACE. Severity high is correct.

## Y87 Linux FIFOs, sockets and device nodes look like empty regular files; syncing them hangs forever or streams endlessly

- Schwere: high · Urteil: confirmed · Kategorie: bug · Plattformen: Linux, Windows
- Stellen: native/src/local_access/os/linux_os.rs:24-46; native/src/vfs/os/shared/local.rs:76-103; native/src/vfs/os/shared/local.rs:19-35; native/src/local_access/os/linux_os.rs:80-82; native/src/sync/os/shared/sync_copy.rs:46-66; native/src/bisync/os/shared/apply_transfer.rs:343-353; native/src/bisync/os/shared/snapshot_dir.rs:131-146; native/src/bisync/os/shared/apply_guard.rs:111-119; native/src/copy/os/shared/staging.rs:115-125; native/src/local_access/os/windows/directory.rs:355-380

**Beschreibung.** read_directory classifies FIFOs/sockets/char/block devices as EntryKind::Other with size 0, but LocalBackend::list_dir builds VfsMeta only from is_dir/is_link_like/size, dropping the kind; stat() (meta_to_vfs) does the same. VfsMeta has no field for special files, so walks and mirrors treat them as empty regular files and later open them with std::fs::File::open (O_RDONLY, blocking). The guard in apply_guard only rejects dirs and links. The copy module, in contrast, refuses non-regular sources.

**Fehlerszenario.** Linux home-directory backup (bisync or quick mirror) containing a named pipe (e.g. Steam's ~/.steam/steam.pipe, assumption: typical Steam install): the copy worker blocks in open() until a writer appears; the run never completes and cancel (checked only between reads) cannot interrupt it. A character device in a backed-up container rootfs/chroot (dev/zero) is read without end until the target disk is full. Unix sockets fail with ENXIO on every run (keeping bisync's baseline unsaved). In checksum mode the hang already happens during the snapshot walk (hash_file).

**Richtung.** Carry the entry kind into VfsMeta (e.g. a 'special' flag) for list_dir and stat; treat special files as protected omissions in bisync walks and the mirror; open local sources with O_NONBLOCK|O_NOFOLLOW and verify S_ISREG with fstat before reading (then clear O_NONBLOCK).

**Gegenprüfung.** Confirmed for Linux. local_access/os/linux_os.rs:25-39 maps FIFOs, sockets and device nodes to EntryKind::Other with size 0. LocalBackend::list_dir never reads entry.kind (local.rs:88-99), and VfsMeta has no kind field (meta.rs:8-26); meta_to_vfs (local.rs:19-35) behaves the same way. Such entries therefore have is_dir=false and is_symlink=false, and snapshot_dir.rs:131-146 records them as regular files. The planner copies them. apply_guard.rs:111-119 rejects only directories and links. stage_source then opens them through local_access::open_read -> std::fs::File::open (apply_transfer.rs:343; linux_os.rs:80-82), which blocks on a FIFO that has no writer. A character device never returns EOF in the stream loop (apply_transfer.rs:377-405). Cancel is checked only between reads (388-390), so a blocked open() cannot be cancelled. The quick mirror takes the same path (sync_scan.rs:171-176 -> sync_copy.rs:48-66). Checksum mode and Full hashing already block during the walk (snapshot_dir.rs:163-183 -> snapshot_hash.rs:30-50). The copy module, by contrast, refuses non-regular sources (staging.rs:115-126). Windows nuance: directory.rs:355-380 does present AF_UNIX and LX_* reparse files as plain files, but opening them fails because no filter handles those tags. On Windows the result is a persistent per-file error (which still blocks the bisync baseline save, orchestration.rs:312-320), not a hang. FILE_ATTRIBUTE_DEVICE entries practically never appear in NTFS listings. Severity high is right because the Linux hang is unbounded and cannot be cancelled.

## Y88 One unreadable, vanished or undecodable entry aborts the whole bisync snapshot and disables mirror deletions

- Schwere: high · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/snapshot_walk.rs:4-6; native/src/bisync/os/shared/snapshot_walk.rs:52-59; native/src/bisync/os/shared/snapshot_walk.rs:72-82; native/src/bisync/os/shared/snapshot_walk.rs:243-246; native/src/bisync/os/shared/snapshot_dir.rs:68; native/src/bisync/os/shared/snapshot_dir.rs:136; native/src/vfs/os/shared/local.rs:37-44; native/src/vfs/os/shared/local.rs:76-103; native/src/vfs/core/delete.rs:318-331; native/src/local_access/os/linux_os.rs:20-35; native/src/local_access/os/windows/read.rs:20-41; native/src/sync/os/shared/sync_scan.rs:137-140; native/src/sync/os/shared/sync_scan.rs:185-213; native/src/sync/os/shared/sync.rs:271-277; native/src/syncjobs/core/types.rs:158

**Beschreibung.** The snapshot walk fails as a whole on its first error, and LocalBackend::list_dir is all-or-nothing per directory. Triggers: stat/list errors of a directory (NotFound because it was deleted during the walk, EACCES/ERROR_ACCESS_DENIED, EIO); a single entry whose name is not valid Unicode (non-UTF-8 on Linux, unpaired surrogates or unrepresentable units on Windows); a per-entry lstat error other than NotFound; a Linux/Android name containing a backslash (validate_child_name rejects it although it is legal there); and content-hash read errors (checksum mode, local side of Drive pairs). Hidden entries are walked by default. The quick mirror records such errors and then skips its whole delete pass.

**Fehlerszenario.** (1) Bisync with an ext4 drive root (/media/user/Backup): lost+found (root, 0700) -> EACCES -> every run fails before planning, nothing is synced. (2) Bisync with an NTFS drive root (D:/) and default include_hidden: 'System Volume Information' -> access denied for the non-elevated user (BackupRead needs SeBackupPrivilege) -> every run fails. (3) Home backup during normal use: a cache/build folder disappears between listing its parent and listing it -> stat NotFound -> intermittent total failure. (4) One Latin-1 filename from an old archive, or a file named 'a\b.txt' on Linux -> job fails permanently. (5) Checksum mode on Windows with an exclusively locked Outlook .pst -> hash read error -> walk fails every run.

**Richtung.** Treat unreadable or vanished subtrees and undecodable/incompatible entries as protected omissions (record, report, preserve counterparts and baseline entries - the rule AGENTS.md already mandates for links) and let the rest of the run proceed; carry non-UTF-8 names losslessly (bytes/escaped form) or omit them per entry; allow backslashes in names when neither side is Windows; exclude System Volume Information, $RECYCLE.BIN and lost+found at volume roots by default.

**Gegenprüfung.** Confirmed. snapshot_walk.rs:243-246 stores the first worker error and 52-59 returns it. A failure on either side ends the pair read (snapshot_pair.rs:101-138), and run_full returns before planning (orchestration.rs:129-132). Each trigger was checked. list_plain_directory runs be.stat(path)? and list_dir? with no tolerance for NotFound or PermissionDenied (snapshot_walk.rs:72-82), so a folder that vanishes during the walk is fatal. LocalBackend::list_dir fails the whole directory on an unreachable or non-UTF-8 name (local.rs:81-87) and filters only per-entry NotFound (101). Linux per-entry lstat errors propagate (local_access/os/linux_os.rs:34). A Windows directory open falls back only to SeBackupPrivilege or a consented broker grant and otherwise returns the denial (local_access/os/windows/read.rs:29-41, broker.rs:105-115); a non-elevated user gets neither for 'System Volume Information'. validate_child_name rejects backslashes (vfs/core/delete.rs:318-331) and is applied with ? (snapshot_dir.rs:68). Hash read errors propagate (snapshot_dir.rs:136, 174-181). include_hidden defaults to true (syncjobs/core/types.rs:158; editor.rs:97). In the quick mirror, errors are recorded per directory (sync_scan.rs:137-140, 200-213) and the entire delete pass is then skipped (sync.rs:271-277). All five scenarios follow directly from this code. Severity high is correct.

## Y89 Copies never carry the source mtime: any failed/cancelled bisync run turns every transferred file into a conflict, and Mirror jobs re-copy everything

- Schwere: high · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/apply_transfer.rs:327-367; native/src/sync/os/shared/sync_copy.rs:45-81; native/src/bisync/os/shared/orchestration.rs:86-95; native/src/bisync/os/shared/orchestration.rs:297-320; native/src/bisync/core/plan.rs:16-41; native/src/bisync/core/plan.rs:59-104; native/src/bisync/core/plan.rs:143-188; native/src/bisync/os/shared/snapshot_hash.rs:63-72; native/src/bisync/os/shared/snapshot_dir.rs:112-121; native/src/bisync/os/shared/apply_transfer.rs:76-88

**Beschreibung.** Neither the bisync stage writer nor the quick-mirror stream copy sets the destination modification time, and the Backend trait has no API for it (no set_times/SetFileTime/utimens in sync or vfs production code). For local-local and other hash-less pairs MtimeSize uses no content hash, so after a copy the two sides never compare equal by signature. The baseline bridges this only if it is saved, but run_full returns without saving it whenever any action failed or the run was cancelled, discarding the record of all completed transfers. Mirror (stateless) compares source vs destination signatures directly and avoids re-copies only through the incremental index, which is bootstrapped only after a run with no errors, no conflicts and no omissions (every link is recorded as an omission, even when excluded). Every run with changes also re-walks the touched sides completely just to learn the new destination mtimes.

**Fehlerszenario.** (1) Default two-way job PC <-> USB disk: run 1 copies 20,000 files, one file fails (locked file, socket, name too long, EINVAL, file changed meanwhile, name invalid on exFAT, >4 GiB on FAT32) or the laptop sleeps / Android kills the process -> baseline not saved. Run 2: for all 20,000 files A and B exist, the baseline has none, sizes match, mtimes differ -> 20,000 FileLevel conflicts; they are no longer backed up until resolved one by one, and the recurring failure keeps the baseline from ever being saved. (2) One-way Mirror of Windows 'Documents' (contains the legacy junctions 'My Music'/'My Pictures'/'My Videos') or of any Linux home with a symlink: omissions never empty -> index never bootstrapped -> each run plans CopyAtoB for every file -> the whole data set is rewritten on every run. (3) Pointing a job at an existing copy made without mtime preservation -> every file is a conflict.

**Richtung.** Set the staged file's mtime to the source mtime before promotion (std::fs::File::set_times locally, protocol equivalents remotely) and record converged signatures from the stage instead of a full re-walk; persist completed actions incrementally (the SQLite state store exists) so one failing file never discards others; bootstrap the incremental Mirror index even when only reported omissions exist; consider size+hash or 'copied by us' markers to converge pre-existing copies.

**Gegenprüfung.** Confirmed. No production code sets destination times: a grep for set_modified, set_times, SetFileTime and utimensat matches only tests. Both stage writers just stream bytes (apply_transfer.rs:343-353, sync_copy.rs:52-67). For local-local MtimeSize, hash_mode returns HashMode::None (snapshot_hash.rs:63-72). sig_eq then compares |mtime_a - mtime_b| against modify_window_ms, which defaults to 0 (plan.rs:31-38; bisync types.rs:306-307; syncjobs types.rs:174). run_full returns the old baseline unsaved after any error or cancel (orchestration.rs:301-320). After a run 1 with one failure, run 2 therefore sees both sides present, no baseline entry, equal sizes and different mtimes. Both sides count as changed, which yields a FileLevel conflict (plan.rs:143-188); FileLevel is the default (types.rs:155). Persistent per-file errors (#1, #10, #12, #15) keep the baseline unsaved permanently. Mirror plans directly on sig_eq(src, dst) (plan.rs:59-104), so without the incremental index every file is copied again on every run. Bootstrapping that index requires errors, conflicts and omissions to all be empty (orchestration.rs:88-95). Every link, including an excluded one, is recorded as an omission root (snapshot_dir.rs:112-121; omissions.rs:29-34, 88-90). So the Windows 'Documents' junctions, any symlink in a home folder, and on Android every job rooted at a volume (Android/data entries become omissions) never bootstrap. Aggravating factor: jobs are always reversible (syncjobs types.rs:242), so each such re-copy first streams the existing destination file into versions_<pair> (apply_transfer.rs:76-88, 285-309). Every unbootstrapped Mirror run thus also duplicates the whole destination into the app-data versions folder. Severity high is justified, arguably critical for Mirror jobs.

## Y90 Sync replaces destination files with stages that were never flushed to stable storage

- Schwere: medium · Urteil: partially_confirmed · Kategorie: data-safety · Plattformen: Windows, Linux, Android
- Stellen: native/src/vfs/os/shared/local.rs:121-133; native/src/sync/os/shared/sync_copy.rs:52-68; native/src/bisync/os/shared/apply_transfer.rs:343-353; native/src/vfs/core/core.rs:83-118

**Beschreibung.** Both sync engines write the stage through Backend::open_write (LocalBackend: plain File::create) and call writer.flush(), a no-op for std::fs::File; there is no sync_data/sync_all and no parent-directory fsync after the rename, so the journaled rename can reach the disk before the data. The trait documents that sync must keep durable stages (open_write_copy_stage_sized) and the copy module syncs replacements, but sync bypasses both. Afterwards the bisync baseline is written with sync_all, so persisted state can claim files whose data is not durable.

**Fehlerszenario.** Backup to a USB disk (Linux page cache, or Windows with write caching): the run reports success, the user unplugs the disk or power fails within the writeback window. On remount the replaced files have the new size but zero/stale content (NTFS valid-data-length zeros, FAT stale clusters); the previous good version was already replaced; because size/mtime match the saved baseline (no hashes in MtimeSize mode) the corruption is never re-detected. The quick mirror keeps no version copy at all.

**Richtung.** For local destinations sync_data the stage before promotion and fsync the parent directory after rename (Linux/Android); on Windows FlushFileBuffers the stage handle before MoveFileEx; route sync through open_write_copy_stage_sized and give LocalBackend a durable stage writer; save the baseline only after durable publication.

**Gegenprüfung.** The cited facts hold. Sync stages are written through open_write = File::create (local.rs:121-125) and finished with writer.flush() (sync_copy.rs:67; apply_transfer.rs:352), which is a no-op for File. Promotion follows with no sync_data or sync_all and no parent-directory fsync (sync_copy.rs:93-97; apply_transfer.rs:133-137). The bisync baseline, by contrast, is fsynced (persistence.rs:192), and the copy module syncs replacements (staging.rs:94-97, durability.rs). Even the trait's 'durable' stage is not durable for LocalBackend: open_write_copy_stage_sized -> open_write_new (core.rs:83-118, local.rs:126-133) has no sync either, so switching sync to that API alone would not help. Overstated parts: before replacing, bisync copies the old version into versions_dir and fsyncs it (apply_transfer.rs:76-88, 301-309), so for bisync the previous version is not lost; only the quick mirror lacks a version. Corruption also requires a crash or unplug inside the writeback window. ext4 (auto_da_alloc) and btrfs flush data on rename-over-existing, which mitigates the replacement case, and Windows removable disks default to quick removal. New files and quick-mirror replacements remain exposed, and MtimeSize mode will not re-detect a zero-filled file of the correct size. Medium fits better than high.

## Y91 Hard 1,000,000-entry / 128 MiB limits make large backups fail or stay permanently partial

- Schwere: high · Urteil: confirmed · Kategorie: gap · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/snapshot.rs:15-16; native/src/bisync/os/shared/snapshot_dir.rs:97-105; native/src/bisync/os/shared/persistence.rs:11-12; native/src/bisync/os/shared/persistence.rs:88; native/src/bisync/os/shared/persistence.rs:222; native/src/sync/os/shared/sync.rs:35-37; native/src/sync/os/shared/sync.rs:46-58; native/src/sync/os/shared/sync_tasks.rs:217-233; native/src/vfs/core/delete.rs:6-8; native/src/daemon/os/shared/schedule.rs:20

**Beschreibung.** Bisync walks fail as a whole beyond 1M entries or 128 MiB of path text; the baseline format refuses more than 1M entries; the quick mirror stops discovering after 1M entries (the same tail of the tree is skipped every run, and the error disables deletions); recursive delete refuses >1M entries; the real-time signature stops at 1M. These are fixed caps, not derived from memory or protocol limits.

**Fehlerszenario.** Bisync of a whole data drive or a developer home with node_modules (about 1.5M files) fails on every run with 'sync tree exceeds its bounded collection budget'; the quick mirror of the same tree leaves the last ~0.5M files unbacked up on every run.

**Richtung.** Stream snapshots/baselines to disk (SQLite state store or external sort-merge) instead of in-memory maps with fixed caps; if a guard is needed, derive it from available memory and report it actionably.

**Gegenprüfung.** Confirmed. The walk budget is MAX_WALK_NODES=1,000,000 and MAX_WALK_TEXT_BYTES=128 MiB (snapshot.rs:15-16). It is checked for every listed entry, before the hidden and ignore filters run, and an overrun returns an error that fails the whole walk (snapshot_dir.rs:97-105). The baseline refuses more than 1M entries on both load and save (persistence.rs:11-12, 88-90, 221-224). In the quick mirror, WalkBudget (sync.rs:35-58) feeds within_budget, which records an error, sets scan_stopped and clears the queued directories (sync_tasks.rs:217-234). The same undiscovered remainder is therefore skipped on every run, and the delete pass is suppressed (sync.rs:271-277). Recursive delete (vfs/core/delete.rs:6-8), the real-time signature (schedule.rs:20) and the incremental collector (incremental_collect.rs:13-15) use the same fixed caps. They are constants, not derived from memory or protocol limits. The failures are reported, not silent, and the only workaround is ignore globs on large subtrees. For backups of whole drives or developer homes, every run fails deterministically, so high stands.

## Y92 Local roots are identified by path text only: an unmounted mount point or another volume at the same path is synced as the real tree (mass deletion)

- Schwere: high · Urteil: confirmed · Kategorie: data-safety · Plattformen: Windows, Linux, Android
- Stellen: native/src/vfs/core/core.rs:25-31; native/src/vfs/core/core.rs:364-368; native/src/vfs/os/shared/local.rs:65-74; native/src/bisync/os/shared/persistence.rs:25-46; native/src/bisync/core/plan.rs:95-99; native/src/bisync/core/plan.rs:169-180; native/src/bisync/os/shared/orchestration.rs:181-232; native/src/syncjobs/core/types.rs:178-179; native/src/sync/os/shared/sync_delete.rs:17-84

**Beschreibung.** LocalBackend keeps the default state_identity (scheme + root text) and change_root_id None; baselines are keyed by root strings. Nothing checks that the root is still the same volume/directory or that a previously populated side suddenly became empty. The mass-delete guard is disabled by default (max_delete 0, max_delete_pct 0). The quick mirror's delete pass has no guard and deletes permanently.

**Fehlerszenario.** (1) Default two-way job with B = /mnt/backup (fstab 'nofail') or an NFS mount that failed at boot: the empty mount-point directory makes every baseline file look deleted on B -> DeleteA for all files on the primary side (each first copied into the versions folder on the system disk until it fills). (2) Mirror job from E:/ when a different USB stick received letter E: -> DeleteB for every file in the backup. (3) The backup disk returning as F: -> user edits the job -> new pair id -> baseline lost -> full conflict set (see mtime finding).

**Richtung.** Store a volume identity (Windows volume GUID/serial via GetVolumeInformationByHandleW, Linux st_dev + filesystem UUID/statfs f_fsid, Android volume UUID) and the root directory's file id with the baseline; refuse to plan deletions when it changed or when a previously non-empty side is empty; enable a default mass-delete threshold; key baselines by volume identity + relative root instead of drive letters.

**Gegenprüfung.** Confirmed. LocalBackend keeps the default state_identity '{Scheme}:{root}' and change_root_id None (vfs/core/core.rs:25-31, 364-368); local.rs:65-74 overrides only namespace_identity. The pair id hashes identity plus root text (persistence.rs:25-46). Nothing compares volume or device identity, and nothing treats a previously populated side that is suddenly empty as suspicious. A missing root is safe because the walk's stat of the root fails (snapshot_walk.rs:74). An existing but empty mount-point directory, or a different volume under the same drive letter, walks successfully. In a two-way job, a file unchanged on A and absent on B with baseline (Some, Some) gives (false, true) and DeleteA (plan.rs:169-180). In Mirror, destination-only files get DeleteB (plan.rs:95-99). The guard fires only when a limit is set (orchestration.rs:210-220), and both limits default to 0 (syncjobs types.rs:178-179, editor.rs:116-117). Each DeleteA first copies the file into the versions folder (apply_delete.rs:86), so files stay recoverable while that copy succeeds. Once the system disk fills, the remaining deletes fail, the run errors, and the old baseline is kept (orchestration.rs:312-320). When B comes back, the files now missing on A are planned as DeleteB (plan.rs:155-166), carrying the loss into the backup as well. The quick mirror's delete pass deletes permanently with no count guard (sync_delete.rs:68-82, 199-202), but it only runs when delete_extra is enabled.

## Y93 Real-time trigger is a whole-tree polling signature that misses renames/moves, ignores job filters and can starve

- Schwere: high · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/daemon/os/shared/schedule.rs:13-55; native/src/daemon/os/shared/schedule.rs:59-68; native/src/daemon/os/shared/run_loop.rs:253-305; native/src/daemon/os/shared/state.rs:7

**Beschreibung.** Every daemon tick (default 15 s) each local root of every real-time job is walked with std::fs and reduced to (file count, newest mtime, total bytes); the job runs only after this triple changed and then stayed unchanged for rt_debounce_secs. The walk applies neither the job's ignore globs nor include_hidden, stops after 1M entries, and treats UNC paths as local.

**Fehlerszenario.** Renaming or moving a file/folder inside the tree changes none of the three values -> no sync. Same-size edits are invisible while any file has a future mtime (newest pinned). A constantly changing ignored path (browser cache, log) changes the triple every tick -> the settle timer restarts forever -> the job never runs. Trees > 1M entries are only partially observed. A UNC root (//nas/share) is walked over the network every 15 s; on laptops/Android the walk keeps disks awake and drains battery.

**Richtung.** Use OS change notifications: Windows ReadDirectoryChangesW (overflow -> rescan) or the USN change journal; Linux inotify per directory with IN_Q_OVERFLOW fallback (fanotify FAN_MARK_FILESYSTEM where privileged); Android inotify/FileObserver on FUSE paths plus MediaStore ContentObserver. Filter events with the job's ignore/hidden rules, make any polling fallback path-aware (hash of path+size+mtime), and debounce with a maximum wait so a busy tree still syncs.

**Gegenprüfung.** Confirmed. The only real-time mechanism for jobs is enqueue_realtime_jobs (run_loop.rs:253-305), polled every tick (default 15 s, state.rs:7). tree_sig (schedule.rs:15-55) receives only the root path, so it applies no ignore globs and no include_hidden. It stops after 1,000,000 entries (schedule.rs:20, 27-29) and reduces the tree to (file count, newest file mtime, total bytes). Directories contribute nothing, so renaming or moving a file or folder changes none of the three values. A job runs only when two consecutive ticks produce the same signature and the change is at least rt_debounce_secs old (run_loop.rs:284-297). A change on every tick, including in paths the job ignores, restarts the settle timer indefinitely. local_root accepts every endpoint without '://' (schedule.rs:59-69), so a UNC root is walked over SMB on every tick. The GUI's notify watcher (app/os/windows/watchers.rs) is not connected to jobs. Further side effect: a run that writes into a local destination changes that root's signature, which re-arms the timer and triggers a second, redundant run.

## Y94 mkdir_all refuses links ABOVE the sync root: every copy fails when an ancestor of the target is a symlink, junction or mounted-folder volume

- Schwere: medium · Urteil: confirmed · Kategorie: platform · Plattformen: Windows, Linux, Android
- Stellen: native/src/vfs/os/shared/local.rs:173-175; native/src/vfs/os/shared/local.rs:227-288; native/src/bisync/os/shared/apply_transfer.rs:337-340; native/src/sync/os/shared/sync_copy.rs:49-51; native/src/sync/os/shared/sync_copy.rs:78-80; native/src/sync/os/shared/sync_copy.rs:110-117; native/src/local_access/os/windows/directory.rs:363-368

**Beschreibung.** mkdir_all_plain walks the absolute destination parent from the filesystem root and rejects every existing component that is link-like. It is called for every copied file (bisync stage_source, quick-mirror plain_parent). The link check is therefore not limited to the subtree below the user-selected root.

**Fehlerszenario.** Windows backup target on a disk mounted into an NTFS folder (C:/Mounts/Backup/..., IO_REPARSE_TAG_MOUNT_POINT is a name surrogate) or below a junction-relocated folder; Linux target addressed through a symlinked ancestor (~/Documents -> /data/Documents, SteamOS /run/media/mmcblk0p1 alias, /home -> var/home on ostree desktops when entered via /home); Android root given as /sdcard/...: every copy fails with 'directory ancestor is a link or reparse point'.

**Richtung.** Validate and pin the sync root once per run (canonicalize, remember its file id) and apply the link check only to components below it; prefer dirfd-relative creation (openat/mkdirat with O_NOFOLLOW, or handle-relative NtCreateFile on Windows).

**Gegenprüfung.** Confirmed. mkdir_all_plain walks absolute.components() from the filesystem root (for UNC, from the share) and calls ensure_plain_component on every Normal component (local.rs:230-255). Any existing link-like component is rejected with PermissionDenied (local.rs:271-279). Bisync calls destination.mkdir_all(parent) for every copy (apply_transfer.rs:337-340). The quick mirror calls it twice per copied file when guard_parent is set (sync_copy.rs:49-51, 78-80, 110-117), and guard_parent = dst.is_local() (sync_pass.rs:174). The function's own doc (local.rs:227-229) says the purpose is to protect the selected root, but the check is not limited to components below the root. IO_REPARSE_TAG_MOUNT_POINT carries the name-surrogate bit and is therefore link-like (directory.rs:355-368). So every copy fails for Windows folder-mounted volumes and junction-relocated folders, Linux paths through a symlinked ancestor, and Android paths spelled via /sdcard. If the root itself is such a link, the walk already fails at the root (snapshot_walk.rs:74-80).

## Y95 Per-file ancestor validation costs O(depth) metadata round trips per copy (twice per file in the quick mirror)

- Schwere: medium · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/vfs/os/shared/local.rs:230-255; native/src/sync/os/shared/sync_copy.rs:49-51; native/src/sync/os/shared/sync_copy.rs:78-80; native/src/sync/os/shared/sync_pass.rs:174; native/src/bisync/os/shared/apply_transfer.rs:337-340; native/src/net/core/backend.rs:127-129

**Beschreibung.** For every file, mkdir_all_plain lstat()s each component from the filesystem root (UNC: from the share root). The quick mirror does it twice per file (before writing the stage and before publishing); bisync once per copy although its FolderRegister already created the folder.

**Fehlerszenario.** Quick mirror of 200,000 files at depth 8 to //nas/backup (UncBackend forwards is_local=true, so guard_parent is on): about 3.2 million extra SMB metadata round trips; at 1 ms LAN latency about 55 minutes, over VPN (30 ms) many hours. Same pattern on NFS/sshfs mounts and Android FUSE.

**Richtung.** Validate each directory once per run (cache validated directories, re-check only the leaf with one stat or an fstat on a held directory handle) and use dirfd-relative operations.

**Gegenprüfung.** Confirmed. This is the same code path as #8: every copied file costs one symlink_metadata per path component, counted from the volume or share root (local.rs:237-252). The quick mirror does this twice per file (sync_copy.rs:49-51 and 78-80). Bisync does it once (apply_transfer.rs:338-339), even after FolderRegister has already prepared the folder (apply.rs:417-438). UncBackend forwards mkdir_all and reports is_local=true (net/core/backend.rs:127-129, 181-183), so guard_parent is on for UNC destinations (sync_pass.rs:174). Plain UNC paths without a saved connection are LocalBackend too (connect/os/shared/resolution.rs:9-20). On local disks the cost is negligible because of the dentry cache. On SMB each check is a CreateFile/query/close sequence, so the scenario's order of magnitude is plausible; the exact time depends on client metadata caching. Only copied files pay the cost, so steady-state runs stay cheap. Medium is fair.

## Y96 Stage names append 25-38 characters to the destination name; long names can never be synced or copied

- Schwere: medium · Urteil: confirmed · Kategorie: bug · Plattformen: Windows, Linux, Android
- Stellen: native/src/vfs/core/promotion.rs:24-42; native/src/sync/os/shared/sync_copy.rs:39; native/src/bisync/os/shared/apply_transfer.rs:341; native/src/vfs/os/shared/local.rs:135; native/src/copy/os/shared/staging.rs:210-223; native/src/copy/os/shared/move_guard.rs:44-48

**Beschreibung.** unique_staging_path builds '<destination>.se-<purpose>-<16 hex>' (+25 chars for 'sync', +27 for 'bisync'); the copy module uses '.<name>.smart-explorer-<16 hex>.part' (+38) and '.move' quarantines. Component limits (255 bytes on ext4/f2fs/Android FUSE, 255 UTF-16 units on NTFS/exFAT, about 143 bytes on eCryptfs) are not considered; the existence probe fails with ENAMETOOLONG/ERROR_INVALID_NAME (not NotFound), so the copy fails before it starts.

**Fehlerszenario.** A PDF with a 240-character title, or an 80-character Japanese filename (240 UTF-8 bytes) on ext4/Android: every sync run reports 'File name too long' for it; in bisync this also keeps the baseline from being saved.

**Richtung.** Bound the stage component length (truncate the stem on a UTF-8/UTF-16 boundary before appending the suffix, or use a short fixed-length '.se-<purpose>-<hex>' name in the same directory) while keeping it recognizable for cleanup.

**Gegenprüfung.** Confirmed. unique_staging_path appends '.se-<purpose>-<16 hex>' to the destination name (promotion.rs:30-33): +25 characters for 'sync', +27 for 'bisync'. The copy module appends '.<name>.smart-explorer-<16 hex>.part' (+38, staging.rs:216-218), and move quarantines use a similar '.move' suffix (move_guard.rs:44-48). The existence probe uses try_exists (promotion.rs:34), which turns every stat error other than NotFound, such as ENAMETOOLONG or ERROR_INVALID_NAME, into a hard error (core.rs:50-56). The copy therefore fails before anything is written. The copy module's create_new fails the same way (staging.rs:222-233), and there is no length-aware fallback. On ext4, f2fs and Android, a name longer than about 228 bytes (about 76 CJK characters) can never be synced. The resulting persistent error keeps the bisync baseline unsaved (#3). On NTFS the limit is 255 UTF-16 units, so bisync fails above about 228 units and the copy dialog above about 217.

## Y97 Stages left by a crash or killed process are walked and synced as user files

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Android, Windows, Linux
- Stellen: native/src/bisync/os/shared/snapshot_dir.rs:106-147; native/src/bisync/os/shared/apply_transfer.rs:341-370; native/src/sync/os/shared/sync_copy.rs:39; native/src/sync/os/shared/sync_copy.rs:83-104; native/src/vfs/core/promotion.rs:30-33; native/src/gdrive/core/transfer.rs:295-304

**Beschreibung.** Stages are created next to the destination with visible names and are removed only on in-process error paths. Nothing recognizes them on the next run: the walk filters only hidden entries, ignore globs, links and the app trash. Only the Drive backend recognizes its own stage names.

**Fehlerszenario.** Android kills the app during a large two-way sync (or power loss on desktop): 'IMG_2041.jpg.se-bisync-3f9c...' (partial content) stays in DCIM. The next run sees a new file on that side and, with the default two-way direction, copies it to the PC; it then exists on both sides permanently (AtoB: lingers as junk; Mirror: deleted and possibly recreated).

**Richtung.** Recognize the engine's own stage pattern in all walks (never sync it), remove stale stages from previous runs at run start (not open, older than the run), or stage inside a per-run private directory on the same volume.

**Gegenprüfung.** Confirmed. Stages are visible siblings named '<file>.se-sync-<hex>' or '<file>.se-bisync-<hex>' (promotion.rs:30-33; sync_copy.rs:39; apply_transfer.rs:341). They are removed only on in-process error paths (sync_copy.rs:83-104; apply_transfer.rs:364-369). The snapshot walk filters only hidden entries, ignore globs, links and the app-trash/Android-data entries (snapshot_dir.rs:107-130), and the quick mirror does the same (sync_scan.rs:154-162). There is no startup sweep: mobile housekeeping (mobile/os/shared/init.rs:103-120) cleans only transfer temp files and Share caches. Only gdrive/core/transfer.rs:295-304 and the agent's DiscardStage whitelist (agent_proto/os/shared/stage_ops.rs:151-195) recognize these names, and neither applies to walks. A stage left on side B of a two-way job is a B-only file with no baseline entry, so it is planned as CopyBtoA (plan.rs:169-173) and copied to A.

## Y98 Windows: names containing ':' from Linux/Android sources are written into NTFS alternate data streams

- Schwere: medium · Urteil: confirmed · Kategorie: platform · Plattformen: Windows
- Stellen: native/src/vfs/os/windows/local_platform.rs:33-47; native/src/vfs/os/windows/verbatim.rs:203-233; native/src/types/core/win32_names.rs:84-90; native/src/vfs/core/promotion.rs:24-42; native/src/vfs/os/shared/local.rs:121-125

**Beschreibung.** to_os converts any path containing a Win32-invalid character into the verbatim path form. Verbatim paths skip Win32 parsing, but NTFS still interprets ':' as the stream separator; other invalid characters (? * " < > |) are rejected by NTFS. Sync never checks destination name compatibility before touching the filesystem.

**Fehlerszenario.** Linux/Android file 'Meeting 10:30.txt' synced to an NTFS folder: the stage 'Meeting 10:30.txt.se-bisync-<hex>' is created as stream '30.txt.se-bisync-<hex>' of a new (or an existing unrelated) file 'Meeting 10'; the no-replace rename of a stream path fails; cleanup deletes only the stream and leaves a 0-byte 'Meeting 10', which the next two-way run copies back to the source. Assumption: an existing unrelated 'Meeting 10' also gets its timestamps touched. Names with ? * " < > | fail on every run and keep bisync's baseline unsaved.

**Richtung.** Before any filesystem call on a Windows local destination, reject or map names containing ':' and NTFS-illegal characters (report as a name-incompatibility omission, or map to a safe name with a persisted mapping); use verbatim paths only for trailing dot/space and device names.

**Gegenprüfung.** Confirmed. ':' is classified as InvalidCharacter (types/core/win32_names.rs:84-89), so to_os switches to the verbatim long-path form (vfs/os/windows/local_platform.rs:42-46; verbatim.rs:19-49). Verbatim paths skip Win32 normalization, but NTFS still parses 'name:stream' in the final component. Sync never checks destination-name compatibility: win32_name_issue is used only by UI, filter and delete code. The stage 'Meeting 10:30.txt.se-bisync-<hex>' (promotion.rs:30-33) therefore becomes a stream of 'Meeting 10', created by File::create (local.rs:121-125). The no-replace MoveFileExW of a stream path (local_platform.rs:128-153) then fails. The cleanup remove_file deletes only the stream (local_platform.rs:118-126), leaving a 0-byte 'Meeting 10' that the next two-way run copies back to the source. NTFS rejects names with ? * " < > | on every run, which keeps the bisync baseline unsaved. The claim that an existing unrelated 'Meeting 10' gets its timestamps changed remains an assumption about OS behaviour.

## Y99 Windows copy/transfer engine rejects every reparse point, including OneDrive placeholders, WOF-compressed and deduplicated files

- Schwere: medium · Urteil: partially_confirmed · Kategorie: platform · Plattformen: Windows
- Stellen: native/src/copy/os/windows.rs:77-80; native/src/copy/os/shared/staging.rs:115-126; native/src/copy/os/shared/safe_file.rs:52-59; native/src/copy/os/shared/safe_file.rs:288-296; native/src/transfer/os/windows.rs:4-9; native/src/local_access/os/windows/directory.rs:355-368

**Beschreibung.** The copy module's Windows metadata_is_link_like treats any FILE_ATTRIBUTE_REPARSE_POINT as a link. Cloud Files placeholders (IO_REPARSE_TAG_CLOUD_*), WOF (CompactOS) and dedup files are data reparse points that local_access lists as ordinary files. The transfer engine (paste, drag & drop, copy dialog via transfer_local; same-backend copies via server_copy_to_stage for local/UNC) therefore refuses them as sources and refuses to overwrite such destinations.

**Fehlerszenario.** Copying a OneDrive (Files On-Demand) folder to a USB disk with the app: every file fails with 'keine regulaere Datei (Links, Reparse-Punkte und Spezialdateien werden nicht uebertragen)'; copying with 'overwrite' into a OneDrive folder fails with 'destination is a link or reparse point' for every existing file.

**Richtung.** Use the tag-aware classification (name-surrogate bit; MOUNT_POINT/SYMLINK/LX_SYMLINK as links, AF_UNIX/LX_FIFO/CHR/BLK as special) in the copy module; decide explicitly whether online-only placeholders (FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) are hydrated or skipped with a report.

**Gegenprüfung.** Confirmed at code level. copy/os/windows.rs:77-80 treats any FILE_ATTRIBUTE_REPARSE_POINT (0x400) as link-like. That rule gates sources in open_source (staging.rs:115-126) and transfer_local (safe_file.rs:52-59), and overwrite destinations in select_initial_target (safe_file.rs:288-296); uploads use the same rule (transfer/os/windows.rs:4-9). This contradicts local_access, which lists data reparse points such as CLOUD_* and WOF as ordinary files (directory.rs:355-368; sync_link_task_tests.rs:4-21). Such files are shown as regular files but refused by paste, drag and the copy dialog. What the code cannot decide is whether OneDrive Files-On-Demand placeholders actually expose the attribute to this process. That depends on the process's placeholder compatibility mode, which the code never sets: there is no RtlSetProcessPlaceholderCompatibilityMode call, and native/app.manifest has no opt-in. If Windows disguises placeholders for this process, the headline OneDrive scenario does not occur, while other data reparse points (dedup, HSM) would still be refused.

## Y100 Windows: replacing a read-only destination fails, and the copy module itself makes copies read-only

- Schwere: medium · Urteil: partially_confirmed · Kategorie: platform · Plattformen: Windows
- Stellen: native/src/copy/os/windows.rs:95-125; native/src/copy/os/shared/staging.rs:87-93; native/src/copy/os/shared/server_copy.rs:75-77; native/src/vfs/os/shared/local.rs:134-154

**Beschreibung.** The copy module replaces with MoveFileExW(MOVEFILE_REPLACE_EXISTING); the VFS replaces with std::fs::rename, whose ACCESS_DENIED fallback uses FileRenameInfoEx with REPLACE_IF_EXISTS|POSIX_SEMANTICS but not FILE_RENAME_FLAG_IGNORE_READONLY_ATTRIBUTE (upstream std source). A read-only destination is refused. The copy module propagates the source's read-only attribute to every copy (set_permissions from the source; CopyFile2 copies attributes), as does LocalBackend::copy_file.

**Fehlerszenario.** Copy files from a DVD/ISO (all read-only) into a backup folder, later copy again with 'Overwrite': every previously copied file fails with 'Access is denied'. A bisync/mirror whose destination contains read-only files (from such copies, robocopy/xcopy, or set by the user) fails to update them on every run.

**Richtung.** On ERROR_ACCESS_DENIED with a read-only existing destination use SetFileInformationByHandle(FileRenameInfoEx) with FILE_RENAME_FLAG_REPLACE_IF_EXISTS|POSIX_SEMANTICS|IGNORE_READONLY_ATTRIBUTE (Windows 10 1809+), or clear the attribute after an identity check and restore it on the published file; decide deliberately whether FILE_ATTRIBUTE_READONLY should be propagated.

**Gegenprüfung.** Copy-module part confirmed. Overwrite commits use MoveFileExW(MOVEFILE_REPLACE_EXISTING|MOVEFILE_WRITE_THROUGH) (copy/os/windows.rs:95-125). Windows refuses to replace a read-only target with ERROR_ACCESS_DENIED, and select_initial_target never clears the attribute (safe_file.rs:268-314), so 'Overwrite' fails for read-only files. Read-only propagation is confirmed: stage_copy applies the source permissions (staging.rs:89-91), server copies do too (server_copy.rs:75-77), CopyFile2 copies attributes (copy/os/windows.rs:150-157), and LocalBackend::copy_file does the same (local.rs:138-143). The sync engines replace through LocalBackend::rename -> std::fs::rename (local.rs:152-154; promotion.rs:90-104). Whether Rust std's ACCESS_DENIED fallback (FileRenameInfoEx) ignores the read-only attribute is internal to std and cannot be checked within the permitted read scope, so the bisync/mirror half of the finding is unproven here.

## Y101 Windows: files in use are not backed up reliably - no VSS, no retry for sharing violations, listing metadata from the lazily updated NTFS directory index

- Schwere: medium · Urteil: confirmed · Kategorie: gap · Plattformen: Windows
- Stellen: native/src/local_access/os/windows/read.rs:54-69; native/src/bisync/os/shared/apply_retry.rs:33-38; native/src/bisync/os/shared/apply_retry.rs:78-94; native/src/bisync/os/shared/apply_transfer.rs:138-141; native/src/local_access/os/windows/directory.rs:164-172; native/src/local_access/os/windows/directory.rs:286-341; native/src/local_access/os/windows/read.rs:71-79; native/src/bisync/os/shared/apply_guard.rs:66-90

**Beschreibung.** Reads use a plain handle; files opened exclusively fail with ERROR_SHARING_VIOLATION, which std maps to Uncategorized. The retry policy treats only network-like kinds as transient, never retries promotion/delete, and defaults to 0 retries. Snapshot metadata comes from FileIdExtdDirectoryInfo (directory index), which Microsoft documents may not be current on NTFS (FindFirstFileW remarks); the apply guard compares it with handle-based stat.

**Fehlerszenario.** Profile backup: NTUSER.DAT, Outlook .pst/.ost, running VM disks, Chrome's Cookies DB fail on every run (bisync then never saves its baseline). An antivirus/indexer/OneDrive briefly opening a fresh stage makes the rename fail once and the whole run counts as failed. A database/log held open and appended keeps its old size/mtime in the directory index until closed, so the change is not planned (backup stays stale) or the guard reports 'changed since planning' (assumption based on documented NTFS behaviour).

**Richtung.** Offer VSS snapshots for backup jobs (IVssBackupComponents, read from the shadow copy); retry rename/delete with backoff on ERROR_SHARING_VIOLATION/LOCK_VIOLATION/ACCESS_DENIED when it is provable the operation did not happen (stage present, destination unchanged); refresh planned metadata from an open handle before deciding instead of treating a listing/stat mismatch as drift.

**Gegenprüfung.** Confirmed. open_read uses a plain File::open and falls back only on PermissionDenied (local_access/os/windows/read.rs:54-69). A sharing violation on an exclusively opened file (NTUSER.DAT, .pst/.ost, VM disks) is therefore a plain error, and there is no VSS path. run_with_retry retries only pre-commit errors of network-like kinds (apply_retry.rs:34-38, 78-95), and the job default is 0 retries (syncjobs types.rs:188). Promotion failures are marked commit_attempted (apply_transfer.rs:138-141) and are never retried. Windows listings come from FileIdExtdDirectoryInfo/FileFullDirectoryInfo, i.e. the directory index (directory.rs:164-172). The quick mirror deliberately re-stats listed entries because NTFS and SMB directory entries can be stale (sync_pass.rs:156-162). Bisync instead plans from the listing, and capture() compares against a handle-based stat (apply_guard.rs:66-90; read.rs:71-79). A file that grows while held open therefore produces 'changed since planning' drift errors. Every such error blocks the baseline save (orchestration.rs:312-320).

## Y102 Case-insensitive targets: quick mirror deletes the only backup copy after a case-only rename; case-variant names break bisync permanently

- Schwere: medium · Urteil: confirmed · Kategorie: data-safety · Plattformen: Linux, Windows, Android
- Stellen: native/src/sync/os/shared/sync_scan.rs:62-75; native/src/sync/os/shared/sync_scan.rs:112-118; native/src/sync/os/shared/sync_scan.rs:270-281; native/src/sync/os/shared/sync_delete.rs:122-141; native/src/bisync/os/shared/apply_guard.rs:73-77; native/src/vfs/core/core.rs:284-290; native/src/bisync/core/omissions.rs:21-27; native/src/bisync/core/omissions.rs:45-68

**Beschreibung.** (a) Quick mirror from a case-sensitive source to a case-insensitive destination (vfat/exFAT/CIFS mounts, Windows, Android): the copy pass resolves the new spelling through a case-insensitive stat to the old entry and skips it (same size, not newer); the delete pass checks the old spelling against the case-sensitive source, finds it absent and deletes it. The reverse direction leaves both spellings forever. (b) Bisync: two source names differing only in case -> the second create finds the first on the case-insensitive side -> drift error every run. (c) LocalBackend never reports case-sensitive paths, so omission folding is always on; on ext4 a real directory 'data' next to an omitted symlink 'Data' is silently excluded.

**Fehlerszenario.** Linux user renames 'Photo.JPG' to 'photo.jpg'; quick mirror with delete_extra to an exFAT USB disk: after the 'successful' run the backup contains no copy of the photo until the next run; if the source disk fails in between, the photo is lost.

**Richtung.** In the quick mirror consult a case-folded index of the source listing before deleting on a case-insensitive destination (and perform a case-only rename instead of copy+delete); detect case collisions at plan time and report them as conflicts/omissions while the rest of the run proceeds; let LocalBackend report real case sensitivity (Windows FILE_CASE_SENSITIVE_INFO per directory, Linux casefold attribute / filesystem type).

**Gegenprüfung.** Confirmed, but the location given for (c) is wrong: omissions.rs has only 131 lines. (a) Quick mirror: a name that exists only in another letter case is 'Unsure' (sync_scan.rs:112-118) and is probed with stat (270-281). A case-insensitive destination returns the old entry, and because copies never carry the source mtime (#3) the destination looks newer, so decide() skips the file (62-75). The delete pass then stats the old spelling on the case-sensitive source, gets NotFound and deletes the destination file permanently (sync_delete.rs:122-141, 199-202). In the reverse direction, stat of the new spelling on a case-sensitive destination is NotFound, so the file is copied; the old spelling still exists on the case-insensitive source, so it is kept, and both spellings remain. (b) Bisync: apply_groups.rs:12-55 only serializes the case variants. The second create captures ExpectedFile::Missing but finds the first file, which is a drift error (apply_guard.rs:73-77) on every run, keeping the baseline unsaved. (c) LocalBackend has no case_sensitive_paths override (core.rs:284-290), so fold_case is always true (snapshot_pair.rs:62). SyncOmissions lowercases its keys (bisync/core/omissions.rs:21-27), and contains/protects match ancestors on that key (45-64). On ext4, a real directory 'data' next to an omitted link 'Data' is therefore removed from the tree by exclude_tree (66-68).

## Y103 Linux: synced copies drop POSIX permissions - private files become world-readable on the target

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: Linux
- Stellen: native/src/vfs/os/shared/local.rs:121-125; native/src/sync/os/shared/sync_copy.rs:52; native/src/bisync/os/shared/apply_transfer.rs:344; native/src/bisync/os/shared/apply_transfer.rs:292-296

**Beschreibung.** Stages are created with File::create (mode 0666 & ~umask) and no mode is applied before or after publication; replacing a destination swaps in a new inode with the default mode. Version backups use the same default mode. Only LocalBackend::copy_file copies permissions.

**Fehlerszenario.** Backing up ~/.ssh/id_ed25519 (0600), ~/.password-store or ~/.gnupg to /srv/backup or an NFS share: copies are 0644 (umask 022) and readable by other local/NFS users; a destination file an admin restricted to 0600 becomes 0644 after the next update.

**Richtung.** Create stages with mode 0600, apply the source mode (policy-masked) via fchmod before publication, keep the stricter of source and existing destination mode on replace; same for version backups.

**Gegenprüfung.** Confirmed. Stages are created with File::create (local.rs:121-125, used by sync_copy.rs:52 and apply_transfer.rs:344), i.e. with mode 0666 & ~umask. Nothing applies the source mode before or after promotion, and the rename swaps in a new inode. Version copies use OpenOptions::create_new with the default mode (apply_transfer.rs:292-296). Created directories get 0777 & ~umask. Only LocalBackend::copy_file (local.rs:138-143) and the copy module (staging.rs:89-91) copy permissions. Under umask 022, a 0600 key becomes 0644, and a 0600 destination becomes 0644 after its next update. This is Linux-only as stated; Android shared storage ignores modes.

## Y104 Windows FAT32 targets: DST/time-zone shifts make every file look modified

- Schwere: medium · Urteil: confirmed · Kategorie: platform · Plattformen: Windows, Linux
- Stellen: native/src/bisync/core/plan.rs:16-41; native/src/bisync/core/plan.rs:169-180; native/src/bisync/core/plan.rs:245-279; native/src/bisync/core/types.rs:306-307; native/src/syncjobs/core/types.rs:174

**Beschreibung.** Signatures are compared with an exact mtime window (default 0) and no exact-hour shift is tolerated. FAT stores local time and Windows converts it with the current DST bias (Microsoft 'File Times'; the reason robocopy has /DST), so every DST change shifts all FAT mtimes by one hour.

**Fehlerszenario.** Two-way PC <-> FAT32 stick: after the DST change every file on the stick looks modified and is copied back to the PC (version copies first). One-way PC -> stick (Propagate): '(false, true)' produces no action and the baseline is not updated, so every file stays 'changed on B'; the next edit of any file on the PC becomes a conflict instead of a backup.

**Richtung.** Detect FAT volumes (GetVolumeInformationW / statfs) and ignore exact +-1 h (and time-zone) shifts with 2 s granularity; for one-way jobs also refresh the destination's baseline signature when only its mtime moved.

**Gegenprüfung.** Confirmed. sig_eq accepts an mtime difference only up to modify_window_ms, which defaults to 0, and has no exact-hour tolerance (plan.rs:16-41; bisync types.rs:306-307; syncjobs types.rs:174). The field's own doc suggests 1-2 s for FAT/DST (syncjobs types.rs:103), which does not cover a 3600 s shift. FAT stores local time and Windows converts it with the current bias, so every FAT mtime shifts by one hour at each DST change. In a two-way job the result is (false, true), i.e. CopyBtoA for every file (plan.rs:169-173), each with a version copy. In a one-way AtoB job, (false, true) produces no action because allow_b_to_a is false, and the file never converges because copies never share mtimes (#3). update_baseline records only applied or converged paths (plan.rs:245-279), so the entry stays 'changed on B', and the next edit on A becomes a FileLevel conflict (plan.rs:181-188).

## Y105 Forced version backups copy every replaced/deleted file into the app data directory on the system volume

- Schwere: medium · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/syncjobs/core/types.rs:242; native/src/bisync/os/shared/apply_transfer.rs:271-317; native/src/bisync/os/shared/apply_delete.rs:84-97; native/src/bisync/os/shared/persistence.rs:59-61; native/src/support_dirs.rs:99-103

**Beschreibung.** Jobs always run with reversible: true; before each replace or delete, back_up_captured streams the full old file from the backend into versions_<pair> under the app data directory and fsyncs it. The copy lands on the machine running the job, not on the destination volume, and uses plain std::fs paths (no Windows verbatim adapter).

**Fehlerszenario.** Nightly backup of a 120 GB VM image to a USB disk: each run reads the old 120 GB from the USB disk into the system drive's app data, then writes the new 120 GB (triple I/O); with less than 120 GB free on C: the replacement fails before copying on every run. On Android the version copies of photos/videos go to app-private internal storage the user cannot see. Mass-deletion cases fill the system disk.

**Richtung.** Keep versions on the destination volume (hidden versions folder under the sync root) and create them by hard link/reflink/rename where possible (O(1)); make the policy configurable and check free space before large replacements.

**Gegenprüfung.** Confirmed. checked_opts always sets reversible: true (syncjobs/core/types.rs:242). Before every replacement (apply_transfer.rs:76-88) and every delete (apply_delete.rs:86), the old file is streamed from the backend into versions_dir/<ts>/<rel> using create_dir_all, create_new and sync_all (apply_transfer.rs:285-309). versions_dir sits under the sync data directory (persistence.rs:59-61): %APPDATA% (Roaming) on Windows, ~/.local/share on Linux, and the app-private files directory on Android (support_dirs.rs:64-80, 99-103). A failed backup, for example on a full disk, is a pre-commit error and blocks the replacement. Retention defaults to 30 days, so a large file replaced nightly accumulates up to about 30 copies. The version path is a plain std::fs join of rel without the verbatim adapter, so hostile Windows names go through Win32 normalization. Interplay with #3: Mirror jobs without the incremental index copy every file each run and therefore back up the entire destination on every run.

## Y106 Sync stages use non-exclusive File::create instead of the exclusive stage writer

- Schwere: low · Urteil: confirmed · Kategorie: security · Plattformen: Linux, Android, Windows
- Stellen: native/src/vfs/os/shared/local.rs:121-133; native/src/sync/os/shared/sync_copy.rs:39-52; native/src/bisync/os/shared/apply_transfer.rs:341-344; native/src/vfs/core/core.rs:74-89

**Beschreibung.** unique_staging_path probes with stat and then open_write (O_CREAT|O_TRUNC, follows symlinks). The trait contract for copy stages is exclusive creation (open_write_new / open_write_copy_stage), which LocalBackend implements but sync does not use.

**Fehlerszenario.** In a writable shared target directory, a symlink placed at the stage name between probe and open would redirect the write and truncate its target (the name is random, so exploitation is unlikely; this is a hardening gap).

**Richtung.** Open sync stages with open_write_copy_stage (O_EXCL|O_NOFOLLOW on Unix, CREATE_NEW on Windows).

**Gegenprüfung.** Confirmed as a hardening gap. unique_staging_path probes with try_exists, and sync then opens the name with open_write = File::create, i.e. O_CREAT|O_TRUNC, which follows symlinks (promotion.rs:29-37; local.rs:121-125; sync_copy.rs:39-52; apply_transfer.rs:341-344). LocalBackend does implement the exclusive open_write_new (local.rs:126-133), which the trait prescribes for copy stages (core.rs:74-89), but sync does not use it. An attack requires predicting a 64-bit randomized name (promotion.rs:143-155), so low severity is right.

## Y107 Windows EFS-encrypted files are written decrypted to backup targets without the OS encryption-loss check

- Schwere: low · Urteil: confirmed · Kategorie: security · Plattformen: Windows
- Stellen: native/src/copy/os/shared/staging.rs:137-177; native/src/sync/os/shared/sync_copy.rs:48-66; native/src/bisync/os/shared/apply_transfer.rs:343-353

**Beschreibung.** Sync streams plaintext through ReadFile/WriteFile. The copy module tries CopyFile2 (which refuses decrypted destinations unless COPY_FILE_ALLOW_DECRYPTED_DESTINATION) but falls back to a handle copy on any error other than AlreadyExists/StorageFull/QuotaExceeded.

**Fehlerszenario.** Backing up EFS-protected documents to a FAT/exFAT stick or a NAS stores them in plaintext without the 'Confirm Encryption Loss' decision Explorer asks for.

**Richtung.** Detect FILE_ATTRIBUTE_ENCRYPTED and require an explicit per-job opt-in (or preserve encryption with the EFS raw APIs when the target supports it).

**Gegenprüfung.** Confirmed. Both sync engines stream plaintext through ReadFile/WriteFile (sync_copy.rs:48-66; apply_transfer.rs:343-353) without any EFS check. In the copy module, when CopyFile2 refuses a decrypted destination, the 'Some(Err(_)) => return Ok(None)' branch falls through to the handle copy (staging.rs:162-173). Durable copies skip CopyFile2 entirely (staging.rs:69-74). Low severity fits.

## Y108 Bisync never creates or deletes directories: empty folders are not backed up, removed/renamed folders leave empty skeletons

- Schwere: low · Urteil: confirmed · Kategorie: gap · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/snapshot_dir.rs:131-146; native/src/bisync/os/shared/apply_delete.rs:67-117; native/src/bisync/core/plan.rs:245-279

**Beschreibung.** Snapshots contain only files; directories are traversed but not recorded, and deletes are file-only (no remove_dir in bisync production code).

**Fehlerszenario.** Renaming 'Projects/2023' to 'Archive/2023': the backup receives the files under the new path while the old tree remains as empty folders forever; intentionally empty folders are missing after a restore.

**Richtung.** Track directories in the snapshot (at least for one-way/Mirror) or prune directories emptied by the run, protected by omissions.

**Gegenprüfung.** Confirmed. scan_listing only queues directories (snapshot_dir.rs:131-134), and the Tree holds files only. plan and update_baseline work on file paths alone (plan.rs:245-279). Deletes are remove_file_id or recycle (apply_delete.rs:111-115). A grep finds no remove_dir or empty-folder handling in bisync production code; folders are created only on demand for file copies (FolderRegister, mkdir_all).

## Y109 Recycle-bin deletes pass the raw VFS path and silently delete permanently where no recycle bin exists

- Schwere: low · Urteil: partially_confirmed · Kategorie: gap · Plattformen: Windows, Linux
- Stellen: native/src/bisync/os/shared/apply_delete.rs:111-123

**Beschreibung.** recycle_local hands the forward-slash VFS path to trash::delete without the platform path adapter; on Windows UNC shares and removable drives the shell has no recycle bin and deletes permanently; hostile names (nul, trailing dot) cannot be recycled.

**Fehlerszenario.** Job with 'use recycle bin' targeting //nas/share or a USB stick: deletions are permanent while the user expects recoverability.

**Richtung.** Check recycle-bin support for the volume (drive type / SHQueryRecycleBin) and report or refuse; pass the native (to_os) path.

**Gegenprüfung.** Code part confirmed. apply_delete.rs:111-113 and 120-123 pass the forward-slash VFS path straight to trash::delete. Unlike every other LocalBackend operation, this skips local_platform::to_os, so hostile Windows names go through Win32 normalization: 'report.' addresses 'report', and a reserved name addresses the device. If a sibling without the trailing dot exists, the wrong file could even be recycled, a risk beyond what the finding states. Whether the shell silently deletes permanently on UNC shares or removable sticks without a recycle bin, rather than failing, depends on the trash crate's IFileOperation flags and shell behaviour, which cannot be verified within scope. On Linux the trash crate uses a $topdir/.Trash-$uid folder, so the 'permanent delete' claim is doubtful there.

## Y110 Long unattended desktop runs do not keep the system awake

- Schwere: low · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux
- Stellen: native/src/daemon/os/shared/job.rs:77

**Beschreibung.** No SetThreadExecutionState/PowerCreateRequest (Windows) or logind/systemd inhibitor (Linux) is taken around a job run (repository grep finds none); Android has its own wake handling.

**Fehlerszenario.** A multi-hour nightly backup to a NAS on a laptop: the idle sleep timer suspends the machine mid-run; on resume network handles are broken, the run ends with errors and (see mtime finding) no baseline is saved.

**Richtung.** Hold a system-required power request (Windows ES_SYSTEM_REQUIRED, Linux logind 'sleep' inhibitor) while a job runs.

**Gegenprüfung.** Confirmed. daemon/os/shared/job.rs:77 runs bisync::run without any power request. A grep of native/src finds no SetThreadExecutionState, PowerCreateRequest/PowerSetRequest or logind inhibitor. Only Android takes a wake lock (android/app/src/main/java/app/smartexplorer/android/system/WakeKeeper.kt:35-88).

## Y111 Backups lose file metadata and miss same-size edits with restored mtimes

- Schwere: low · Urteil: confirmed · Kategorie: gap · Plattformen: Windows, Linux, Android
- Stellen: native/src/vfs/core/meta.rs:8-26; native/src/bisync/core/plan.rs:16-41; native/src/sync/os/shared/sync_copy.rs:52-66; native/src/bisync/os/shared/apply_transfer.rs:377-413

**Beschreibung.** Copies are pure byte streams: no xattrs, ACLs, NTFS alternate data streams, sparse ranges or hard-link structure; change detection uses only size+mtime (no ctime/ChangeTime or file id).

**Fehlerszenario.** A VeraCrypt container (timestamps preserved by default) or a file rewritten with 'touch -r'/'rsync -t' at equal size is never re-synced in MtimeSize mode; a sparse 100 GB VM image with 10 GB data occupies 100 GB on the target.

**Richtung.** Record ctime/ChangeTime and file id for local sides as an additional change signal; optional xattr/ADS copy; sparse-aware copy (SEEK_DATA/SEEK_HOLE, FSCTL_QUERY_ALLOCATED_RANGES).

**Gegenprüfung.** Confirmed. VfsMeta (vfs/core/meta.rs:8-26) has no field for change time, inode, xattrs, ACLs, alternate streams or sparse ranges. sig_eq compares size, mtime and an optional hash (plan.rs:16-41), and the stream loops copy bytes only (sync_copy.rs:52-66; apply_transfer.rs:377-413). Same-size rewrites with a restored mtime are invisible in MtimeSize mode, and sparse files are written out at full size.

## Y112 Android: sync's Android/data|obb exclusion matches only registered volume paths, not /sdcard aliases

- Schwere: low · Urteil: partially_confirmed · Kategorie: platform · Plattformen: Android
- Stellen: native/src/apptrash/mod.rs:51-67; native/src/apptrash/mod.rs:70-120; native/src/bisync/os/shared/snapshot_dir.rs:112-115; native/src/bisync/os/shared/snapshot_walk.rs:72-80

**Beschreibung.** hidden_app_folders_in compares the directory prefix literally with the registered volume roots, while the analytics' ProtectedAreas explicitly handles the aliases /sdcard and /storage/self/primary. A sync root spelled through an alias does not get the protected omission for other apps' folders.

**Fehlerszenario.** Job root '/sdcard' (typed by the user): entries in /sdcard/Android/data are not omitted; if their lstat/open fails (assumption, see unresolved), the walk aborts on every run.

**Richtung.** Canonicalize sync roots (or reuse ProtectedAreas) before the omission check.

**Gegenprüfung.** The alias mismatch is real. in_hidden_app_parent compares the parent prefix literally with the registered volume roots (apptrash/mod.rs:51-67); only ProtectedAreas handles /sdcard and /storage/self/primary (mod.rs:73, 89-120). Sync roots are stored exactly as typed and never canonicalized (connect/core/location.rs:62-70). However, a root spelled exactly '/sdcard' or '/storage/self/primary' never reaches Android/data. LocalBackend::stat is an lstat and the root is a symlink, so the walk already fails at the root with 'sync directory changed into a link' (snapshot_walk.rs:74-80). As a destination, every copy is also refused by the ancestor check (#8). The gap is reachable only with spellings such as '/sdcard/' (the trailing slash makes lstat follow the link) or '/sdcard/Android'. The 'assumption' that Android/data children fail is supported by the code's own comments (apptrash/mod.rs:45-50; local_access/os/linux_os.rs:13-15).

## Y113 Mirror backups re-transfer every file on each full run: no backend preserves mtime and Mirror planning compares source and destination mtimes exactly

- Schwere: critical · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/tests.rs:355-360; native/src/bisync/os/shared/apply.rs:45-55

**Beschreibung.** Copies made by the sync never set the destination modification time: the Backend trait has no mtime setter and no backend sends SETSTAT, MFMT, X-OC-Mtime/PROPPATCH, Drive modifiedTime or File::set_modified, so every copied file carries the copy time. The Mirror branch of the planner is stateless (the baseline is ignored) and treats a file as up to date only if size and mtime match within modify_window_ms (default 0), unless both sides carry a content hash. hash_mode gives no hash to SFTP (with or without the agent), FTP, SMB, Share peers, a local side paired with those, and WebDAV files without oc:checksums (assumption: this includes the app's own plain PUT uploads to Nextcloud, since no OC-Checksum is sent). So after the first run every file the app copied differs in mtime and is copied again whenever the full planner runs. The full planner runs always for remote sources (try_incremental_mirror returns None for a source that is neither local nor change-feed capable), and for local sources whenever the previous run had any error or conflict, the source contains any link/junction/app-trash entry (omission roots are recorded even when hidden or ignored, and bootstrap requires none), or the target drift check fails (see the SFTP NotFound finding). Jobs force reversible=true, so each replacement and each mirror deletion first copies the existing destination file into versions_<pair> under the local app-data directory. Drive binary files are exempt (md5 on both sides).

**Fehlerszenario.** Mirror job 'NAS sftp://.../photos (500 GB) -> external disk', or 'Windows Documents (contains the hidden My Music junction) -> smb://nas/backup'. Run 1 copies 500 GB. Every later run: plan emits a copy for every file (destination mtime = copy time), transfers the 500 GB again and first writes another full copy of the old destination files into the app-data versions folder (Android: internal app storage) with 30-day retention, so the system disk fills and runs no longer finish within their interval.

**Richtung.** Add Backend::set_mtime and apply it to the stage before promotion (SFTP SETSTAT/FSETSTAT, SMB SET_INFO FileBasicInformation, FTP MFMT when FEAT lists it, WebDAV X-OC-Mtime or PROPPATCH where supported, Drive modifiedTime in create/update metadata, a new peer FsRequest, local File::set_modified). Make Mirror baseline-aware (converged when each side still equals its recorded baseline signature). Let protected omissions coexist with an incremental index. For remote destinations keep reversible versions on the destination side (server-side rename into a versions folder or trash) instead of downloading them into app data. Derive the default modify window from the coarser side's timestamp precision.

**Gegenprüfung.** No production code sets a destination mtime: the Backend trait has no setter (vfs/core/core.rs:16-494); grep for set_modified/set_times/setstat/MFMT/X-OC-Mtime/PROPPATCH finds only tests and listing parsers; stage_source writes a fresh stage and promotes it (apply_transfer.rs:327-371); Drive upload metadata is only id/name/parents (gdrive/core/transfer.rs:71-80). russh-sftp even offers set_metadata (session.rs:243) but it is unused. The Mirror branch ignores the baseline (plan.rs:57-104) and sig_eq short-circuits on hashes only when both are non-zero (plan.rs:31-33), otherwise |dmtime| <= modify_window_ms, which jobs default to 0 (syncjobs/core/types.rs:174, 247; bisync types.rs:307). hash_mode returns HashMode::None unless a side provides a content hash (snapshot_hash.rs:63-72; only WebDAV webdav.rs:470-476 and Drive gdrive backend.rs:328-335 do). The project's own test comment admits this exact effect for Drive before hashes were added (bisync tests.rs:355-360). The full planner runs for every non-local, non-change-feed source (incremental.rs:116-139), for every Drive pair (75-79), and for local sources whenever the last full run had errors, conflicts or any omission root (orchestration.rs:88-95; omissions.rs:29-34, 88-90; snapshot_dir.rs:112-121 records links and app-trash entries even when excluded). For example, a Windows Documents folder always contains the hidden My Music, My Pictures and My Videos junctions, so it never becomes incremental. It also runs when the drift probe fails (incremental.rs:177-179, see #2). Jobs force reversible (syncjobs types.rs:242), so every replace first copies the old destination into versions_<pair> in app data (apply.rs:52; apply_transfer.rs:76-88; persistence.rs:14-16, 59-61). Pruning is age-based only (persistence.rs:302-344) and runs only after a successful run (orchestration.rs:400). Critical is justified for backups.

## Y114 WebDAV mkdir_all starts at the server root, so sync uploads to Nextcloud/ownCloud-style DAV roots fail (and cost 2 requests per path level per file elsewhere)

- Schwere: critical · Urteil: confirmed · Kategorie: bug · Plattformen: Windows, Linux, Android
- Stellen: native/src/webdav/core/webdav.rs:396-434; native/src/bisync/os/shared/apply_transfer.rs:338-340; native/src/connect/os/shared/connector.rs:240-276

**Beschreibung.** stage_source calls destination.mkdir_all(parent) for every copied file with the absolute server path; WebDAV backend paths are absolute URL paths such as /remote.php/dav/files/<user>/Backup/... WebdavBackend::mkdir_all sends MKCOL for every prefix starting at the first segment; any answer other than 2xx or 405 is fatal, and on 405 a Depth-0 PROPFIND must succeed and report a collection. Prefixes above the DAV mount point are not DAV resources. Manual copies are unaffected because the transfer engine creates folders with create_dir below the target.

**Fehlerszenario.** Assumption to verify on a live server: Nextcloud's remote.php answers a request with empty path info (/remote.php) with 404 'Path not found'. A job local -> webdav://u@cloud:443/remote.php/dav/files/u/Backup sends MKCOL https://cloud/remote.php for the first file, gets 404, mkdir_all returns NotFound and every copy into Nextcloud fails before commit, on every run. On servers where all ancestors answer 405 and PROPFIND 207, each file still costs MKCOL+PROPFIND for every level (8 levels = 16 extra HTTPS requests per file).

**Richtung.** Never create above the sync root: treat the job root as the known-existing base and create only missing levels below it with create_dir, deepest existing ancestor first; reuse the folder register in apply.rs and skip the per-file mkdir_all when the parent was already ensured in this run.

**Gegenprüfung.** stage_source calls mkdir_all on the absolute parent for every copied file (apply_transfer.rs:338-340). The WebDAV root is the absolute DAV path (connector.rs:240-251; the CLI documents --root /remote.php/dav/files/alice at cli/connections.rs:15). WebdavBackend::mkdir_all sends MKCOL from the first segment (webdav.rs:396-434). Any status other than 2xx or 405 is fatal (415-420, 430), and 404 maps to NotFound (webdav.rs:35). On 405, a Depth-0 PROPFIND must report a collection (421-428), and that PROPFIND would also fail on a non-DAV ancestor. Nextcloud and ownCloud remote.php raise RemoteException('Path not found', 404) for an empty service path; this comes from knowledge of their source and cannot be checked here. So every bisync upload into such a root fails before commit, while the transfer engine's create_dir (transfer_ops.rs:31-42) is unaffected. The cost on servers that answer 405 (MKCOL plus PROPFIND per existing level, per file) is also confirmed.

## Y115 Plain SFTP never reports NotFound: every SFTP status error becomes ErrorKind::Other

- Schwere: high · Urteil: confirmed · Kategorie: bug · Plattformen: Windows, Linux, Android
- Stellen: native/src/sync/os/shared/sync_copy.rs:77; native/src/sync/os/shared/sync_copy.rs:143-149; native/src/sync/os/shared/sync_scan.rs:62-75; native/src/sync/os/shared/sync_tasks.rs:100-112; native/src/sftp/core/connection.rs:280-291; native/src/connect/core/types.rs:62

**Beschreibung.** IntoIoError for russh_sftp::client::error::Error maps everything except Timeout to io::Error::other, including Status(NoSuchFile) and Status(PermissionDenied). SftpBackend::stat and list_dir therefore never return NotFound. try_exists is overridden and correct, and the bisync apply guard falls back to it, but other callers branch on error.kind().

**Fehlerszenario.** (a) Incremental mirror local -> SFTP: any new source file makes target_rel_drifted() stat the absent target path, get Other and report drift, so the run falls back to the full planner (with the mtime finding: the whole backup is re-uploaded whenever a file is added). (b) Quick mirror (sync/) from an SFTP source: for every real extra file on the destination the source stat yields Other instead of NotFound, so 'mirror deletion preflight failed; nothing deleted' on every run and extras are never removed. (c) A Share host exporting an SFTP connection forwards such errors with no kind.

**Richtung.** Map StatusCode::NoSuchFile to NotFound, PermissionDenied to PermissionDenied, OpUnsupported to Unsupported, Eof to UnexpectedEof; add a test that stat of a missing path yields NotFound.

**Gegenprüfung.** IntoIoError maps every russh-sftp error except Timeout to io::Error::other (sftp/core/errors.rs:39-46). safe_metadata returns Status errors through io_err without replay (connection.rs:80-90; classify_sftp_error treats Status as healthy at 280-291), so stat and list_dir never yield NotFound (backend.rs:208-234). try_exists is correct via russh-sftp try_exists (session.rs:161-167). Plain SFTP is the default: use_agent defaults to false (connect/core/types.rs:62). (a) is confirmed: target_rel_drifted treats any non-NotFound error as drift (incremental_changes.rs:135-150), so any added file sends a local-to-SFTP mirror back to the full planner (incremental.rs:177-179), which with #0 re-uploads everything. (b) is unreachable as written: delete_extras runs only with delete_extra (sync.rs:271-289), and every production caller hard-codes delete_extra: false (app/core/sync_core.rs:19-22; mobile sync_run.rs:294-297). A worse reachable effect exists in the same quick mirror: a new file gets Copy(None) (sync_scan.rs:65), and copy_stream then calls validate_destination, whose (None, Err(non-NotFound)) arm fails (sync_copy.rs:77, 143-149). So quick mirror into a plain-SFTP destination fails for every new file. The confirm path has the same problem (sync_tasks.rs:100-112). (c) is confirmed: the Share wire keeps only typed kinds (share/core/fs_error.rs:17-28). Bisync copy capture is protected by the try_exists fallback (apply_guard.rs:121-130).

## Y116 FTP stat lists the whole parent directory; sync issues several such listings per file (quadratic in directory size)

- Schwere: high · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/apply_transfer.rs:63-142; native/src/bisync/os/shared/apply_transfer.rs:327-371; native/src/vfs/core/promotion.rs:24-42; native/src/ftp/core/staging.rs:91-128

**Beschreibung.** FtpBackend::stat is a LIST of the parent plus a linear search; try_exists, unique_staging_path and the publish step all go through listings, and the sync walker stats every directory before listing it. Each LIST opens a new passive data connection (plus a TLS handshake on FTPS) and runs on the single browsing connection.

**Fehlerszenario.** Backing up a camera folder with 20,000 files to FTP: per copied file the apply path does capture, revalidate, a staging-name probe, the staged-size capture, revalidate and the publish listing, about 6 LISTs of the 20,000-entry parent, i.e. ~120,000 listings and billions of parsed lines plus gigabytes of listing traffic, all serialized on one control connection; the run effectively hangs. FTP as a source costs ~3 parent LISTs per file.

**Richtung.** Use MLST for single-entry stat and MLSD for listings when FEAT advertises them (RFC 3659), then SIZE+MDTM, LIST only as last resort; reuse one parent listing per apply step; skip the per-directory stat in the walker when the parent listing already proved a plain directory.

**Gegenprüfung.** FtpBackend::stat lists the parent and searches it (ftp.rs:211-228). list_dir and stat run on the single primary connection (ftp.rs:201-204). One new-file copy into FTP costs: capture of the destination (apply_transfer.rs:68), revalidate (100), the staging-name probe via try_exists then stat (promotion.rs:34), capture of the stage (apply_transfer.rs:358), revalidate (120), and the publish listing (ftp staging.rs:97-98). That is 6 parent LISTs, and replacements add backup revalidation. The walker stats every directory before listing it (snapshot_walk.rs:72-81). Each LIST needs a passive data connection (a TLS handshake on FTPS). Correction: FTP as a source costs 2 parent LISTs per file (capture apply_transfer.rs:63, revalidate 357), not about 3. The quadratic cost on large flat folders stands. 'Hangs' overstates it, but a 20k-file folder means about 10^9 parsed rows and tens of GB of listing traffic.

## Y117 Sync uploads to FTP, WebDAV and Google Drive spool the entire file to local temp storage first (bisync uses open_write instead of the streaming sized stage)

- Schwere: high · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/apply_transfer.rs:341-353; native/src/vfs/core/core.rs:108-118; native/src/ftp/core/ftp.rs:254-259; native/src/ftp/core/writer.rs:16-19; native/src/ftp/core/writer.rs:44; native/src/webdav/core/webdav.rs:291-297; native/src/webdav/core/writer.rs:132; native/src/gdrive/core/transfer.rs:306-310

**Beschreibung.** stage_source writes the stage with destination.open_write(&staged). For FTP, WebDAV and Drive that writer buffers the whole file in tempfile::tempfile() and uploads only at flush, although all three implement the streaming open_write_copy_stage_sized and the trait documents that sync keeps the sized stage. On desktop the spool lives in std::env::temp_dir() (tmpfs, i.e. RAM-limited, on Debian 13, Fedora, Arch); on Android in the app cache on internal storage. FTP's spooled STOR also runs on the browsing connection under its mutex, blocking every stat/list of the run for the whole upload.

**Fehlerszenario.** Backing up a 12 GB video from an Android phone with 8 GB free internal storage to FTP/WebDAV/Drive: ENOSPC while spooling, the copy fails on every run. On a Linux desktop with tmpfs /tmp and 16 GB RAM any file larger than ~8 GB fails. Even with enough space every byte is written and re-read locally before the upload starts.

**Richtung.** In stage_source use open_write_copy_stage_sized(&staged, size) when the source size is authoritative, keeping open_write only for transformed reads (Drive exports); this also moves FTP STOR to a pooled connection and adds length enforcement.

**Gegenprüfung.** stage_source uses destination.open_write(&staged) (apply_transfer.rs:344). Each of the three affected writers spools the whole file before uploading. FTP: FtpWriter uses tempfile::tempfile() (ftp/core/writer.rs:40-47), and STOR runs at flush through with_stream_mutation on pool.primary() (writer.rs:15-21; ftp.rs:254-259), which holds the control mutex for the whole upload (io_adapters.rs:106-125). That blocks every stat and list and serializes all of the run's FTP uploads. WebDAV: WebdavWriter spools (webdav/core/writer.rs:117-134; webdav.rs:291-298). Drive: DriveWriter spools (gdrive/core/transfer.rs:306-315; backend.rs:183-185). Streaming sized stages exist but bisync does not use them (ftp.rs:277-289; webdav.rs:311-317; gdrive backend.rs:201-207; trait docs core.rs:108-111). The spool location is std::env::temp_dir on desktop (support_dirs.rs:38-51) and the app cache on Android (mobile init.rs:77-78). The ENOSPC and tmpfs scenarios follow.

## Y118 WebDAV directory listings larger than 10 MiB cannot be read (ureq into_string cap)

- Schwere: high · Urteil: confirmed · Kategorie: bug · Plattformen: Windows, Linux, Android
- Stellen: native/src/webdav/core/webdav.rs:151-174

**Beschreibung.** propfind() reads the multistatus body with Response::into_string(), which in ureq 2.x (Cargo.lock pins 2.12.1) refuses bodies larger than 10 MB. A Depth-1 PROPFIND of a large collection exceeds this; the error is retried once (downloading the body again) and then propagated, failing the whole walk.

**Fehlerszenario.** A Nextcloud folder with ~25,000 photos (roughly 400-600 bytes of XML per entry with oc:checksums) produces more than 10 MiB, list_dir fails, the snapshot walk fails and every run of that job ends with 'response too big for into_string'.

**Richtung.** Parse the multistatus from into_reader() with a streaming XML parser or an explicit, memory-bounded larger limit; consider server paging (Nextcloud nc:paginate) for huge collections.

**Gegenprüfung.** propfind reads the body with Response::into_string (webdav.rs:164). In the locked ureq 2.12.1 (Cargo.lock), into_string caps bodies at INTO_STRING_LIMIT = 10 MiB and errors with 'response too big for into_string' (ureq response.rs:33, 456-471). The error is retried once (webdav.rs:166) and then propagated. list_dir uses Depth-1 PROPFIND (227-230), so the walk and browsing fail. Nextcloud entries carry about 380-450 bytes of XML each with the requested props, so roughly 23-27k entries per folder exceed the limit.

## Y119 Scheduled runs whose endpoint cannot be resolved are not recorded and are retried every 60 s indefinitely

- Schwere: high · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/daemon/os/shared/job.rs:26-41; native/src/daemon/os/shared/job.rs:128; native/src/daemon/os/shared/job.rs:185-198; native/src/daemon/os/shared/job_supervisor.rs:9; native/src/daemon/os/shared/job_supervisor.rs:66-79; native/src/syncjobs/core/schedule.rs:36-46; native/src/connect/core/location.rs:89-134; native/src/connect/os/shared/resolution.rs:27-33

**Beschreibung.** run_one logs "skip '<job>': source/target ..." and returns before persist_attempt when resolve_endpoint fails (NAS offline, DNS failure, changed password, missing saved connection, peer offline). last_run is not updated, so an interval job stays due(); the supervisor only throttles to MIN_RETRY_INTERVAL = 60 s. No JobResult is written, so the UI keeps showing the last successful result. A job endpoint must match a saved connection by exact protocol/user/host/port, so editing the saved connection (new IP, port or user) silently orphans the job.

**Fehlerszenario.** The NAS password is changed: the daemon attempts one SFTP or SMB login per minute (1,440 per day), fail2ban bans the client or the domain account gets locked out (SMB), while the job list still shows the last 'ok' from weeks ago and no backup happens. A laptop away from home reconnects to an unreachable host every minute.

**Richtung.** Record a failed attempt (result note and error count) and mark the run; back off exponentially on repeated resolution failures and stop automatic retries on authentication failures until the user acts; surface the failure in the job list and notifications.

**Gegenprüfung.** run_one logs and returns before persist_attempt when resolve_endpoint fails (daemon/os/shared/job.rs:26-41; contrast 128 and 185-200), so last_run and the JobResult are not updated. Interval jobs stay due (syncjobs/core/schedule.rs:41-46), and calendar jobs stay due while last_run < occurrence (47-58). They are re-enqueued every tick (run_loop.rs:177-182), throttled only by MIN_RETRY_INTERVAL = 60 s (job_supervisor.rs:9, 73-79). Every attempt performs a full connect and login (connector.rs:374-420). Saved-connection matching requires the exact protocol, user, host and port (connect/core/location.rs:89-134), and nothing rewrites job endpoints when a connection is edited (connect/os/shared/persistence.rs). About 1,440 logins per day and a stale 'ok' status follow.

## Y120 Sync trees with more than 1,000,000 entries can never be synced (hard walk budget, also in the agent fast path)

- Schwere: medium · Urteil: confirmed · Kategorie: gap · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/snapshot.rs:15-16; native/src/bisync/os/shared/snapshot_dir.rs:97-105; native/src/bisync/os/shared/snapshot_agent.rs:9-10; native/src/bisync/os/shared/snapshot_agent.rs:40-45; native/src/bisync/os/shared/persistence.rs:10-12; native/src/bisync/os/shared/incremental_collect.rs:13-15

**Beschreibung.** Every listed entry increments a per-walk counter before filtering (excluded entries count too); at 1,000,000 the walk fails with 'sync tree exceeds its bounded collection budget'. The agent hash walk has the same cap and baselines are capped at 1M entries.

**Fehlerszenario.** A home-directory or NAS photo/mail archive backup with 1.2 million files fails at the walk on every run; nothing is synced.

**Richtung.** Replace the fixed cap by a memory-derived bound or keep trees and baselines in the existing SQLite state store with streamed comparison; count only entries that pass the filter.

**Gegenprüfung.** MAX_WALK_NODES = 1,000,000 (snapshot.rs:15). The counter increments before the exclusion check (snapshot_dir.rs:97-108). The agent fast path has the same cap (snapshot_agent.rs:9-10, 40-45) and counts every streamed hit before filtering (50-58). The agent walks the whole tree server-side, so ignored subtrees such as node_modules still count there. Baselines and change collections are also capped (persistence.rs:11; incremental_collect.rs:13-15). Larger trees fail on every run. This is a deliberate memory bound with an explicit error message, and only very large trees hit it, so medium fits better than high.

## Y121 SFTP directory listing must finish within one 20 s absolute deadline

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/sync_overload.rs:77-88; native/src/transfer/os/shared/flow.rs:392-405

**Beschreibung.** list_dir runs russh-sftp read_dir (OPENDIR, all READDIR batches, CLOSE) inside safe_metadata with SFTP_METADATA_DEADLINE = 20 s for the whole future; a deadline marks the generation suspect and is not replayed.

**Fehlerszenario.** Assumption: OpenSSH's sftp-server returns about 100 names per READDIR. A 40,000-entry directory over a 60 ms VPN/WAN link needs ~400 round trips (~24 s) plus server-side lstat time, the listing times out, the walk fails and the job fails on every run; the SSH connection is also retired.

**Richtung.** Use a progress-based deadline for listings (per READDIR request) instead of one absolute budget, keeping the operation cancellable.

**Gegenprüfung.** list_dir runs the whole russh-sftp read_dir future (OPENDIR, sequential READDIR loop, CLOSE; russh-sftp session.rs:170-198) inside safe_metadata under one 20 s AbsoluteDeadline (sftp/core/connection.rs:17, 74-98; backend.rs:208-215). A timeout retires the generation without replay. The bisync walk does retry TimedOut as overload (transfer flow.rs:392-405; sync_overload.rs:77-88), but each retry restarts the full listing under a new 20 s budget after a reconnect. A deterministically slow listing (OpenSSH sends about 100 names per READDIR, so about 400 round trips for 40k entries) therefore still fails, after up to 5 minutes. This needs a large directory and a high-latency link, so medium.

## Y122 Local writes refuse symlinked or junction ancestors above the chosen root, so local destinations and Share hosts under such paths reject every file

- Schwere: medium · Urteil: partially_confirmed · Kategorie: platform · Plattformen: Linux, Windows
- Stellen: native/src/vfs/os/shared/local.rs:230-288; native/src/bisync/os/shared/apply_transfer.rs:338-340; native/src/sync/os/shared/sync_copy.rs:110-117

**Beschreibung.** LocalBackend::mkdir_all validates every component from the filesystem root and rejects any link-like ancestor, not only components below the sync or export root. bisync calls mkdir_all for every copied file's parent, and a Share host runs the same call for MkdirAll requests from peers.

**Fehlerszenario.** On Fedora Silverblue/Kinoite/Bazzite (/home is a symlink to var/home) a remote -> local backup into /home/user/Backup fails every file with 'directory ancestor is a link or reparse point: /home'; the same for a Share host exporting a folder under /home, on Windows when Documents was relocated with a junction, or when the backup folder is reached through a symlink such as ~/NAS -> /mnt/nas.

**Richtung.** Canonicalize the authorized root once and enforce the no-link rule only for components below it; keep refusing links inside the tree.

**Gegenprüfung.** Confirmed for bisync and quick-mirror local destinations. mkdir_all_plain validates every component from the root and rejects link-like ancestors (vfs/os/shared/local.rs:173-175, 230-288). It is called for every copied file's parent (apply_transfer.rs:338-340) and by the quick mirror's plain_parent (sync_copy.rs:110-117). Link-like on Windows includes junctions and volume mount points (name-surrogate tags, local_access windows/directory.rs:363-368). Refuted for the Share host: secure_local_target canonicalizes the export root before building target paths (share/core/fs.rs:176, 311-322; mount_lease.rs:129-133), so the host's MkdirAll (server_fs.rs:140-149) never sees a symlinked ancestor above the export. The Fedora Atomic scenario is weaker than stated, because $HOME there is likely /var/home/<user>, so picker paths avoid the /home symlink. It still hits paths chosen through /home and user symlinks such as ~/NAS -> /mnt/nas.

## Y123 Share host re-opens a saved remote connection for every request to an exported 'Verbindungen' entry

- Schwere: medium · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/server.rs:295-322; native/src/share/core/backend.rs:450-456

**Beschreibung.** fs::resolve -> resolve_connection -> crate::connect::open_saved_at builds a brand-new SFTP/FTP/WebDAV/SMB backend (TCP, TLS or SSH handshake, login, optional agent deploy, WebDAV probe) for each Stat, ListDir, Read, Write or Rename request; rename and promote resolve twice. Nothing caches the backend.

**Fehlerszenario.** A client syncs a folder that a peer exposes as 'Verbindungen/NAS-SFTP': a walk of 5,000 folders plus per-file stats causes tens of thousands of SSH logins on the host; MaxStartups, fail2ban or FTP per-IP limits start refusing and each request pays seconds of handshake, which also pushes listings past the peer's 20 s budget.

**Richtung.** Keep a per-connection backend cache on the host (keyed by the saved-connection account, idle expiry, invalidated when the connection is edited) and resolve paths against it.

**Gegenprüfung.** Without a mount lease the host uses FsAccess::Dynamic (share/core/server.rs:295-322; fs_access.rs:34-64). resolve then calls resolve_connection, which calls crate::connect::open_saved_at on every call (share/core/fs.rs:146-211, 204): a fresh TCP/SSH/TLS login, an optional agent deploy, and a WebDAV probe (connector.rs:374-420; webdav.rs:129-131). This happens per list or stat (fs.rs:74-75, 105-106) and twice for rename and promote (115-116, 134-135). Sync never acquires a lease; only mount_path_capabilities does (share/core/backend.rs:450-456). The scenario is limited to the opt-in 'Verbindungen' export (include_connections). fail2ban only reacts to failed logins, so that part is overstated. The real impact is extreme slowness and per-request handshakes against the NAS, so medium.

## Y124 Google Drive sync rejects local names that are valid on Linux/Android (Windows naming rules applied everywhere)

- Schwere: high · Urteil: confirmed · Kategorie: platform · Plattformen: Linux, Android
- Stellen: native/src/gdrive/core/names.rs:5-55; native/src/gdrive/core/resolution.rs:76-100; native/src/bisync/os/shared/orchestration.rs:310-320

**Beschreibung.** Drive path segments must be in the canonical names::encode form; decode() fails unless encode(decoded) == segment. encode escapes : " < > | ? * % slash and backslash, control characters, leading whitespace, trailing dots/spaces and the first character of reserved stems (CON, PRN, AUX, NUL, COM1-9, LPT1-9). Local walks pass raw names, so such local names fail in stat, resolve and upload.

**Fehlerszenario.** Two-way or mirror Linux/Android -> Drive: files 'Meeting 10:00.txt', '100%.pdf', 'aux.c', 'con.h', 'notes?.md' fail with 'Ungueltiger Drive-Pfadname' on every run, keeping the job in error (which also blocks incremental bootstrap). Conversely a Drive file 'aux.c' is listed as '%61ux.c', so a two-way pair sees two names for one file and creates '%61ux.c' locally.

**Richtung.** Treat a segment that is not in canonical encoded form as a literal title (decode only canonical escapes), and apply Windows-reserved-name escaping only when the other endpoint cannot store the name.

**Gegenprüfung.** decode() rejects any segment unless encode(decoded) == segment (gdrive/core/names.rs:34-55). encode escapes control characters, / \ % : " < > | ? *, leading whitespace, trailing dots and spaces, and device stems (5-32). Raw local names go through decode in resolution (resolution.rs:76-80, 92-100) and in upload metadata (transfer.rs:74), while listings are encoded (duplicates.rs:56-59). Correction: Windows is affected too, because '%' and leading spaces are valid Windows names. Both '100%.pdf' and 'file%20name.pdf' fail (the second decodes to 'file name.pdf' and re-encodes differently). The 'aux.c' to '%61ux.c' double-name effect is also confirmed. Impact is amplified because any per-file error blocks the baseline save (orchestration.rs:310-320).

## Y125 Leftover staging files from interrupted runs are synced as user files

- Schwere: medium · Urteil: confirmed · Kategorie: data-safety · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/server_transfer.rs:387-400

**Beschreibung.** Stages are named '<file>.se-bisync-<16 hex>' (host side '<file>.se-peer-<16 hex>'). Cleanup after a failure is a best-effort remove_file that silently fails when the connection is gone, the Share host deliberately retains stages after flush or promotion failures, and process death (Android kill, crash, power loss) leaves them too. The bisync walker has no filter for these names and jobs have no default ignore patterns.

**Fehlerszenario.** A two-way local <-> SFTP or peer job is killed during a 4 GB upload. The next run finds the partial 'video.mp4.se-bisync-1a2b...' on side B, absent on A and in the baseline, and copies it into the user's folder on A, from where it propagates to other synced devices; repeated interruptions accumulate stage files on backup targets.

**Richtung.** Recognize the app's own stage patterns (.se-bisync-, .se-peer-, .se-upload-, .se-agent-...) in every walk as protected omissions and remove stale owned stages at the start of the next run.

**Gegenprüfung.** Stages are '<file>.se-bisync-<16 hex>' (promotion.rs:29-36). Cleanup is a best-effort let _ = remove_file (apply_transfer.rs:126, 130, 139, 367), and process death leaves the stage. The Share host retains its '.se-peer-' stage on any failure, including a client that simply disappears: commands None is treated as BrokenPipe and the stage is retained (server_transfer.rs:325-336, 387-400). The walker has no stage-name filter (snapshot_dir.rs:63-131), and jobs ship no default ignores (syncjobs types.rs:159). Only Drive recognizes stage names, and only for upload reservation (gdrive/core/transfer.rs:33, 295-304). Two-way jobs propagate partial stage files as user files, and one-way Propagate jobs accumulate them. Mirror deletes them, with a versions backup first. The effect is junk and wasted space, not loss of user data, so medium.

## Y126 Share peer directory listings must arrive within 20 s per attempt and fit one 16 MiB frame

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/peer_request.rs:16-17; native/src/share/core/peer_request.rs:55-100; native/src/share/core/peer_request.rs:367-380; native/src/share/core/framing.rs:11

**Beschreibung.** ListDir and Stat are retryable reads with a 40 s budget; each attempt's connect, request and complete response must finish before control_attempt_deadline (at most 20 s). The Entries reply is a single JSON control frame limited to 16 MiB.

**Fehlerszenario.** Syncing a phone's DCIM/Camera folder (30-60k files) through the relay: the host's FUSE-backed listing plus a multi-MB JSON transfer over the relay exceeds 20 s twice and the walk fails; folders with more than roughly 90k entries exceed 16 MiB and can never be listed.

**Richtung.** Page ListDir (cursor) or stream entries in data frames with a progress-based deadline.

**Gegenprüfung.** ListDir and Stat are retryable reads with a 40 s budget (peer_request.rs:16-17, 54-62, 372-380). Each attempt's connect, request and full response share connect_deadline = min(overall, now + 20 s) (85-100, 367-370). The reply is one Entries control frame (share/core/server_fs.rs:91), limited to MAX_FRAME 16 MiB on both ends (framing.rs:11, 68-69, 104-106). FsMeta JSON is about 170 bytes per entry (wire.rs:229-239), so roughly 95k entries is the hard ceiling. The walk's overload retry (TimedOut counts as overload) keeps the 20 s per-attempt cap. Medium is appropriate.

## Y127 FTP listings use LIST: dotfiles can be missing, timestamps have minute/day precision, the path argument can be globbed by the server

- Schwere: medium · Urteil: confirmed · Kategorie: gap · Plattformen: Windows, Linux, Android
- Stellen: native/src/ftp/core/ftp.rs:74-95; native/src/ftp/core/ftp.rs:197-209; native/src/ftp/core/staging.rs:73-87

**Beschreibung.** The backend sends 'LIST <path>' and parses ls-style rows; MLSD/MLST are never used. Many servers (for example vsftpd with its default force_dot_files=NO) omit dot entries unless '-a' is given; ls rows give HH:MM only for recent files and only the date for files older than about 6 months; several servers treat LIST arguments as glob patterns.

**Fehlerszenario.** With 'include hidden' enabled, '.config' and other dot entries on an FTP source are silently not backed up; uploading '.bashrc' to such a server succeeds but the stage '.bashrc.se-bisync-...' is invisible to the follow-up LIST, so the copy fails with 'staged copy disappeared' every run. When a file crosses the 6-month boundary its listed mtime loses the time of day and two-way jobs copy it back as 'changed'. Assumption: suppaftp infers the current year for HH:MM rows without correcting future dates, so on 1 January all July-December rows jump a year.

**Richtung.** Prefer MLSD/MLST (UTC modify facts, no hiding, no globbing), fall back to CWD + 'LIST -a'; record listing precision and compare mtimes at that precision.

**Gegenprüfung.** Listings use 'LIST <path>' and ls-row parsing only (ftp.rs:74-95, 197-209; staging.rs:73-79; suppaftp list() at sync_ftp/mod.rs:594-604). MLSD and MLST are never used. suppaftp fills the current year for HH:MM rows with no future-date correction (list.rs:454-479), and date-only rows become 00:00. For a dotfile, the stage '.x.se-bisync-…' is invisible on servers that hide dot entries, so capture of the stage returns None, regular() fails with 'staged copy disappeared' (apply_guard.rs:58-63; apply_transfer.rs:358-359), and publish also cannot find it (staging.rs:107-109).

## Y128 FTP resolves host names with Hickory instead of the operating system resolver

- Schwere: medium · Urteil: confirmed · Kategorie: platform · Plattformen: Windows, Linux, Android
- Stellen: native/src/ftp/core/resolver.rs:33-76; native/src/ftp/core/resolver.rs:97-128; native/src/ftp/core/resolver.rs:130-179; native/src/sftp/core/session.rs:134-138

**Beschreibung.** FTP uses hickory_resolver::Resolver::builder_tokio() (configured DNS servers plus hosts file). Names that only the OS resolver knows - mDNS '.local' names, LLMNR/NetBIOS single-label names on Windows, OS-managed VPN split DNS - fail for FTP, while SFTP (tokio lookup_host, i.e. getaddrinfo) and SMB resolve them. Assumption: the behavior of Hickory's system configuration on Android (no /etc/resolv.conf) is unverified.

**Fehlerszenario.** A home NAS saved as ftp://user@nas.local or ftp://user@DISKSTATION: SFTP to the same name works, but FTP backups fail with an 'FTP DNS ...' error on every run.

**Richtung.** Resolve FTP hosts with the platform resolver on a bounded worker thread with a deadline, or fall back to it when Hickory finds nothing.

**Gegenprüfung.** FTP resolves through hickory_resolver::Resolver::builder_tokio() (ftp/core/resolver.rs:44-47), a deliberate choice (Cargo.toml:103-106). SFTP uses tokio::net::lookup_host, i.e. getaddrinfo (sftp/core/session.rs:134-137). Hickory does unicast DNS plus the hosts file, so mDNS '.local' and LLMNR/NetBIOS names fail for FTP only. The Android assumption is resolved: Hickory reads DNS servers from ConnectivityManager via ndk_context (hickory system_conf/android.rs), and android-bridge initializes that context (android-bridge/src/lib.rs:89-111). So Android works for unicast names but shares the mDNS gap.

## Y129 Silent fallback from the SSH agent to plain SFTP changes timestamp precision under the same sync identity

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/connect/os/shared/connector.rs:155-172; native/src/agent/core/backend.rs:151-156; native/src/agent_proto/os/shared/hash.rs:50; native/src/sftp/core/metadata.rs:18

**Beschreibung.** With use_agent, a failed agent deploy silently yields the plain SFTP backend for sync (AgentFallback::Allow). Both use the same state_identity and baseline, but the agent reports millisecond mtimes while plain SFTP reports whole seconds.

**Fehlerszenario.** Two-way job local <-> SFTP with agent: after a run with the agent, the next deploy fails (remote disk full, noexec home); every remote file now differs from the baseline by its sub-second part, so all files count as changed on the SFTP side and are copied over the local tree (each local file versioned into app data first); when the agent works again the same happens in reverse.

**Richtung.** Normalize SFTP-side mtimes to whole seconds in the agent paths (or store per-side precision and compare at the coarser one), and report or fail an agent fallback for sync jobs instead of switching silently.

**Gegenprüfung.** open_saved_at uses AgentFallback::Allow (connector.rs:358-363), and a failed deploy silently yields plain SFTP (connector.rs:156-163). The agent delegates state_identity to the inner SFTP backend (agent/core/backend.rs:151-153), so the pair id and baseline are shared. The agent reports millisecond mtimes (agent_proto fs.rs:70-75, 96, 116; hash.rs:50), while plain SFTP reports whole seconds (sftp/core/metadata.rs:18). With a 0 ms modify window and no hashes (the agent inherits provides_content_hash=false, agent backend.rs:449-451), a two-way job sees every remote file as changed after each switch and copies it over the local side with versioning. The same flip happens when the user toggles use_agent.

## Y130 A full or over-quota target does not stop the sync, and several endpoints lose the 'disk full' meaning

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/fs_error.rs:17-28; native/src/sftp/core/errors.rs:39-46; native/src/ftp/core/ftp.rs:135-140; native/src/bisync/os/shared/apply_retry.rs:78-95; native/src/transfer/core/engine_policy.rs:88-89

**Beschreibung.** The transfer engine ends a job on StorageFull/QuotaExceeded; bisync never checks these kinds. FTP 452/552 and SFTP's generic SSH_FX_FAILURE become Other; the Share wire carries only NotFound, PermissionDenied, AlreadyExists, Unsupported and Busy, so a host's StorageFull arrives as Other.

**Fehlerszenario.** A NAS backup target fills after 300 GB of a 1 TB run: every remaining file is still streamed until the server refuses it, each failing, for hours; the result lists thousands of generic errors instead of one 'target full'.

**Richtung.** Map ENOSPC equivalents (FTP 452/552; WebDAV 507, SMB DiskFull and agent texts already map), carry StorageFull/QuotaExceeded over the Share wire, and abort the bisync apply phase on the first such error.

**Gegenprüfung.** The bisync pool collects errors and never aborts (apply_pool.rs:66-67). Bisync has no StorageFull/QuotaExceeded handling (grep of bisync and sync finds none), while the transfer engine stops on these kinds (engine_policy.rs:84-92). Several sources lose the kind. SFTP maps every status, including SSH_FX_FAILURE, to Other (errors.rs:39-46). FTP transfer_err only recognizes 421 and 530 refusals (ftp.rs:135-140; connection.rs:42-63). The Share wire has no storage kind (fs_error.rs:21-27). A full target therefore streams every remaining file to failure.

## Y131 Local and Share-host writes are acknowledged without fsync

- Schwere: medium · Urteil: confirmed · Kategorie: data-safety · Plattformen: Windows, Linux, Android
- Stellen: native/src/vfs/os/shared/local.rs:121-133; native/src/share/core/server_transfer.rs:370-386; native/src/vfs/core/core.rs:97-118

**Beschreibung.** LocalBackend::open_write/open_write_new return a bare std::fs::File whose flush() does not sync, there is no durable sized override, and promotion renames without syncing file data or the directory. A Share host acknowledges WriteDone after output.flush() and the promote. SFTP (fsync@openssh.com) and SMB (FLUSH) commit durably, so durability differs per endpoint.

**Fehlerszenario.** A backup to a Share peer (or to a USB disk) reports success; the host loses power or the disk is unplugged minutes later and files are empty or truncated while the baseline records them as synced; in a two-way job the next run treats the truncated host copy as a change and copies it over the intact source (recoverable only from the versions folder).

**Richtung.** Return a durable writer from LocalBackend for sync and peer stages (sync_all on flush, fsync the parent directory after rename) and keep the unsynced variant for plain copies.

**Gegenprüfung.** LocalBackend::open_write and open_write_new return a bare File with no sync on flush (local.rs:121-133). Local has no sized or durable override and promotes by rename (core.rs:99-106, 193-195). The only fsyncs in bisync are for versions backups and baselines (apply_transfer.rs:307-308; persistence.rs:192). The Share host acknowledges after output.flush() and the promote (server_transfer.rs:370-386). The baseline is fsynced and records the page-cache size. After power loss, a truncated new file on ext4 delalloc or exFAT USB becomes a 'changed' side in two-way jobs and overwrites the intact source, recoverable only from versions.

## Y132 Google Drive sync identity is derived from the OAuth refresh token

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/gdrive/core/backend.rs:22-36; native/src/gdrive/core/state.rs:89; native/src/gdrive/core/state.rs:287-310; native/src/bisync/os/shared/persistence.rs:25-34

**Beschreibung.** GDriveBackend::state_identity hashes tokens.refresh_token (or uses 'token-cache-unavailable'), although a stable drive_account_key from about.user.permissionId exists. Re-authorizing the account yields a new refresh token and therefore a new pair id for every Drive job.

**Fehlerszenario.** After the user reconnects Google Drive (token revoked, password change, new login), two-way jobs start without a baseline: files deleted on either side since the last run are copied back, edits on one side become conflicts, and incremental state and versions are orphaned.

**Richtung.** Use drive_account_key in state_identity with a one-time migration from the old pair id.

**Gegenprüfung.** state_identity hashes tokens.refresh_token, or uses 'token-cache-unavailable' (gdrive/core/backend.rs:22-36). The stable drive_account_key exists (state.rs:85-89, 283-307), and its own comment says it, 'Unlike a refresh token, ... remains the same when OAuth credentials are refreshed or re-authorized'. Rotated refresh tokens are persisted (cloud.rs:333-335). The pair id derives from state_identity (persistence.rs:25-34), so re-authorization gives every Drive job a new, empty baseline. Deletions are resurrected, divergent edits become conflicts, and the old versions_<pair> directory is orphaned and never pruned.

## Y133 'Keep both' conflict resolution cannot create the conflict copy on FTP or Google Drive

- Schwere: medium · Urteil: confirmed · Kategorie: gap · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/apply_transfer.rs:168-236; native/src/bisync/os/shared/apply.rs:164-200; native/src/ftp/core/ftp.rs:291-292; native/src/vfs/core/core.rs:184-187

**Beschreibung.** copy_conflict_sibling_at publishes the conflict copy only through rename_no_replace. FTP leaves it unsupported on purpose and GDriveBackend does not override the default (Unsupported); the error is not AlreadyExists and the probe finds no collision, so the stage is discarded and the action fails.

**Fehlerszenario.** A two-way job with conflict mode 'keep both' between a laptop and FTP or Drive: every conflict where the FTP/Drive side loses fails with 'backend has no atomic no-replace rename' on every run; on Drive each attempt also uploads and trashes a staged copy.

**Richtung.** Publish conflict siblings with promote_copy_stage / promote_staged_no_replace (FTP's absence-checked publish, Drive's verified-ID publish) instead of rename_no_replace.

**Gegenprüfung.** copy_conflict_sibling_at publishes only through rename_no_replace (apply_transfer.rs:195-236). FTP keeps the trait default Unsupported on purpose (ftp.rs:291-292; staging.rs:4-6), and GDriveBackend has no override (grep). On a non-AlreadyExists error, the collision probe returns false, the stage is removed, and the action fails as commit_attempted (214-220). KeepBothAtoB/BtoA preserve the losing side's copy on that side's backend (apply.rs:164-221). Every 'keep both' conflict where the FTP or Drive side loses therefore fails. On Drive the stage is uploaded and then trashed each time.

## Y134 Drive's change feed is never used by sync (Drive mirrors always do full walks), and its handling would mis-resolve paths if enabled

- Schwere: medium · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/gdrive/core/backend.rs:100; native/src/gdrive/core/backend.rs:337-351; native/src/bisync/os/shared/incremental.rs:75-79; native/src/gdrive/core/changes.rs:20-90; native/src/bisync/os/shared/incremental_collect.rs:293-334

**Beschreibung.** has_duplicate_file_names() is unconditionally true for Drive, so try_incremental_mirror returns None for any pair containing Drive, while bootstrap still fetches a start page token and collects ids each run. drive_changes_since ignores the root and returns whole-account changes with raw (unencoded) titles; resolve_change returns Rebuild for any change outside the known tree.

**Fehlerszenario.** A Drive -> local mirror of 200k files lists every Drive folder on every run (thousands of API calls) even when nothing changed. If the duplicate guard were lifted, any unrelated edit elsewhere in the account would force a rebuild and names needing encoding would map to wrong relative paths.

**Richtung.** Allow the incremental path when the duplicate observation of the affected folders is clean; filter changes by ancestry to the sync root and encode titles with names::encode before building relative paths.

**Gegenprüfung.** has_duplicate_file_names() is unconditionally true for Drive (gdrive backend.rs:100), so try_incremental_mirror returns None for any Drive pair (incremental.rs:75-79). The full run still asks for a start page token when Drive is the mirror source (orchestration.rs:85-86 -> backend.rs:345-347) and re-bootstraps the store and ids (incremental.rs:373-407). drive_changes_since ignores the root and returns raw titles with rel: None (changes.rs:20-90). resolve_change returning None forces Rebuild (incremental_collect.rs:153-156, 293-334). The latent mis-resolution only matters if the guard is lifted.

## Y135 Sync copies cannot resume: an interrupted large transfer restarts from zero on the next run

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/apply_transfer.rs:342-363; native/src/syncjobs/core/types.rs:188

**Beschreibung.** stage_source opens one reader and one writer and streams the whole file; any read or write error aborts the copy, deletes the stage and, with the default retries = 0, leaves the file for the next run, which starts at byte 0 again. Backends provide open_read_at and resumable uploads that the transfer engine uses elsewhere.

**Fehlerszenario.** A 60 GB VM image over a VPN or mobile link that drops every ~40 minutes never finishes; every scheduled run spends bandwidth for nothing.

**Richtung.** Keep an owned stage across attempts, resume reads with open_read_at and continue writes at the stage offset where the protocol allows (SFTP/SMB offset writes, Drive resumable session, FTP REST/APPE), and retry transient failures within the run.

**Gegenprüfung.** stage_source streams once from one reader to one writer, and any error removes the stage (apply_transfer.rs:342-370). run_with_retry retries only transient pre-commit errors, from byte 0, and jobs default to retries: 0 (syncjobs types.rs:188; apply_retry.rs:60-95). Backends offer open_read_at (core.rs:144-152; sftp backend.rs:243-251; webdav 281-289; gdrive 223-233), but bisync never uses them. The SFTP pool reader retries only before any byte is delivered (pool_reader.rs:95).

## Y136 One unrepresentable file name aborts the whole sync

- Schwere: medium · Urteil: partially_confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/bisync/os/shared/snapshot_dir.rs:68; native/src/vfs/core/delete.rs:318-331; native/src/vfs/os/shared/local.rs:37-44; native/src/webdav/core/multistatus.rs:85-101; native/src/webdav/core/multistatus.rs:135-151; native/src/ftp/core/ftp.rs:74-83

**Beschreibung.** validate_child_name rejects names containing a backslash (legal on Linux, NAS, SFTP, FTP and WebDAV servers); the local listing errors on non-Unicode names; WebDAV errors on non-UTF-8 hrefs; FTP fails the listing on any unparseable LIST row. Each of these fails the directory listing, and the walk fails as a whole. Assumption: russh-sftp and suppaftp also fail on non-UTF-8 names.

**Fehlerszenario.** A single file whose name contains a backslash (typical after unzipping a Windows ZIP on Linux) or a Latin-1 name from an old NAS anywhere in a 500 GB source makes every run of the backup job fail at the walk; nothing else is backed up.

**Richtung.** Treat unrepresentable entries as reported protected omissions (skip, keep counterparts, list them in the result) instead of failing the listing.

**Gegenprüfung.** Confirmed: scan_listing calls validate_child_name on every entry (snapshot_dir.rs:68), and that rejects any name containing a backslash (vfs/core/delete.rs:318-331). One such name aborts the walk for local Linux, SFTP and WebDAV (multistatus.rs:135-151), and FTP rejects it while parsing rows (ftp.rs:83). Local non-Unicode names (local.rs:37-44, 79-87), non-UTF-8 WebDAV hrefs (multistatus.rs:85-101) and unparseable FTP rows (ftp.rs:74-81) also abort the whole walk. Refuted assumption: russh-sftp (buf.rs:24-25) and suppaftp (sync_ftp/mod.rs:771) decode names lossily. Non-UTF-8 names on SFTP or FTP therefore give per-file failures on the U+FFFD name, not a walk abort, unless two names collapse to the same lossy string (duplicate check snapshot_dir.rs:87-95). The 'Latin-1 NAS' example over SFTP thus degrades per file.

## Y137 FTP control connection has no keepalive during long transfers

- Schwere: low · Urteil: confirmed · Kategorie: reliability · Plattformen: Android, Windows, Linux
- Stellen: native/src/ftp/core/connection.rs:291-303; native/src/ftp/core/connection.rs:305-359; native/src/ftp/core/io_adapters.rs:176-219; native/src/ftp/core/io_adapters.rs:271-305

**Beschreibung.** During RETR/STOR the control stream is checked out, so the NOOP keepalive cannot run, and no TCP keepalive is set on control or data sockets. Assumption: NAT and firewall idle timeouts (carrier-grade NAT on mobile, cloud NAT around 350 s) drop the idle control mapping.

**Fehlerszenario.** A 20-minute STOR through mobile CGNAT completes on the data channel but the 226 reply never arrives; the upload is reported as 'Uploadstatus unklar' and fails, repeating for every large file on that network.

**Richtung.** Enable TCP keepalive with a short interval on FTP control and data sockets.

**Gegenprüfung.** checkout takes the control stream out for the transfer (io_adapters.rs:176-219), and keepalive_once skips when the stream is absent (284-286). No SO_KEEPALIVE is set on control or data sockets (ftp/core/connection.rs:291-303, 305-359; no keepalive option in native/src/ftp). The idle-timeout behaviour of NAT and CGNAT is an external assumption. Low fits.

## Y138 No way to trust a self-signed FTPS/WebDAV certificate, and WebDAV is HTTPS-only

- Schwere: low · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/ftp/core/connection.rs:159-170; native/src/connect/os/shared/connector.rs:240-251; native/src/webdav/core/webdav.rs:93-112

**Beschreibung.** FTPS verifies only against webpki roots; WebDAV uses ureq's default rustls roots and the connector hardcodes https: true. There is no certificate pinning or trust-on-first-use (unlike SFTP known_hosts) and no explicit opt-out.

**Fehlerszenario.** A NAS with its default self-signed certificate cannot be used via FTPS or WebDAV at all; users fall back to plain FTP and send credentials and data unencrypted, the opposite of 'secure by default, insecure only by opt-out'.

**Richtung.** Offer certificate pinning (store the fingerprint on first connect after explicit confirmation, like known_hosts) and an explicit, clearly labelled plain-HTTP opt-out for WebDAV.

**Gegenprüfung.** FTPS trusts only webpki_roots (ftp/core/connection.rs:159-170). ureq is built with default-features=false and features 'tls' (rustls with webpki roots; Cargo.toml:35). The connector hardcodes https: true for WebDAV (connector.rs:243-244). There is no pinning, trust-on-first-use or explicit opt-out, so self-signed NAS certificates cannot be used. Low fits.

## Y139 Google-native files and shortcuts cause repeated work or errors in every Drive sync

- Schwere: low · Urteil: confirmed · Kategorie: gap · Plattformen: Windows, Linux, Android
- Stellen: native/src/gdrive/core/api.rs:46-61; native/src/gdrive/core/backend.rs:147-168; native/src/gdrive/core/backend.rs:187-195

**Beschreibung.** Google Docs/Sheets/Slides have no md5 and Drive reports no size, while the local copy is the exported file; every other google-apps type (shortcut, form, site, map) is exported as PDF, which Drive refuses for non-Docs-editor types.

**Fehlerszenario.** A Drive -> local mirror re-exports every Google Doc on every run (size 0 versus exported size never matches); shortcuts and Forms produce an export error on every run and keep the job in error.

**Richtung.** Resolve shortcuts to their targets or omit them as protected entries, skip non-exportable types explicitly, and compare exported Docs by modifiedTime/version recorded in the baseline.

**Gegenprüfung.** Google-native files have no size or md5 in metadata, so the Drive side's Sig is size 0 and hash 0 (gdrive/core/metadata.rs:41-60). The local exported copy has a real size, so the stateless Mirror (plan.rs:57-104) copies it again on every run, with a versions backup. Every other google-apps type, including shortcuts and forms, is exported as PDF (api.rs:46-61), which Drive refuses for non-Docs-editor types, giving an error on every run. read_size returns None for exported files (backend.rs:187-195).

## Y140 FTP replace relies on RNTO overwriting, which Windows-hosted FTP servers refuse

- Schwere: low · Urteil: confirmed · Kategorie: platform · Plattformen: Windows
- Stellen: native/src/ftp/core/staging.rs:116-127

**Beschreibung.** Replacing an existing file is a single RNFR/RNTO pair assuming POSIX rename semantics. Assumption: IIS FTP and other Windows servers answer 550 when the target exists.

**Fehlerszenario.** Backups to an IIS FTP site upload new files but every changed file fails to publish on every run.

**Richtung.** Detect the refusal and fall back to a guarded replace (rename old aside, rename stage, delete old) with recovery on the next run.

**Gegenprüfung.** Replacement is a single RNFR/RNTO pair that assumes POSIX overwrite semantics (ftp/core/staging.rs:1-7, 116-127). IIS and other Windows-hosted servers refusing RNTO onto an existing file is external server behaviour, assumed rather than verifiable here. The affected platform is the server OS, not the client OS. Low fits.

## Y141 Share peer sync identity includes the peer's transport node id

- Schwere: low · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/backend.rs:199-203; native/src/share/core/identity_repair.rs:10-15

**Beschreibung.** state_identity is 'peer:<kind>:<relation>:<node_id>'. If the peer device replaces its Iroh identity (IdentityReplaced) while the contact or room relation stays, every job against it gets a new pair id.

**Fehlerszenario.** After identity repair on the phone, the PC's two-way job runs without a baseline: deletions since the last run are resurrected and edits become conflicts.

**Richtung.** Key the identity on the stable relation and device id used in the endpoint string, not on the transport key.

**Gegenprüfung.** state_identity is 'peer:{kind}:{relation}:{node_id}' taken from the initial endpoint (share/core/backend.rs:199-203; relation_kind_id session.rs:296-301). An IdentityReplaced repair (identity_repair.rs:10-15) gives the device a new node id. If the room or contact relation survives, the pair id changes and the baseline is lost. Low fits.

## Y142 SFTP servers without posix-rename@openssh.com cannot receive updates of existing files

- Schwere: low · Urteil: confirmed · Kategorie: gap · Plattformen: Windows, Linux, Android
- Stellen: native/src/sftp/core/posix_rename.rs:41-51; native/src/sftp/core/backend.rs:312-316

**Beschreibung.** Replacing an existing file goes only through the OpenSSH extension; servers without it get Unsupported for every replace unless the agent is enabled. Assumption: affects some appliance and Java-based SFTP servers.

**Fehlerszenario.** Changed files never reach such a server; the job reports the same error every run.

**Richtung.** Use SFTP v5+ rename flags where offered, otherwise a guarded rename-aside replace with recovery.

**Gegenprüfung.** promote_staged replaces only through posix-rename@openssh.com (sftp/core/backend.rs:310-316). Without the extension it returns Unsupported (posix_rename.rs:41-51), and plain SFTP is the default (connect/core/types.rs:62). How many servers lack the extension is an external assumption. Low fits.

## Y143 WebDAV claims free content hashes for every server, so the local side hashes the whole tree

- Schwere: low · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/webdav/core/webdav.rs:232-269; native/src/webdav/core/webdav.rs:470-476; native/src/bisync/os/shared/snapshot_hash.rs:68-70

**Beschreibung.** provides_content_hash() is always true, so a local side paired with any WebDAV server uses HashMode::Full and reads every new or changed local file for MD5, even when the server never returns oc:checksums (Apache mod_dav, Synology, nginx, and, by assumption, files this app uploaded to Nextcloud without OC-Checksum). stat() never returns the checksum.

**Fehlerszenario.** The first sync of a 2 TB local photo archive to a Synology WebDAV share reads all 2 TB for hashes that are never compared.

**Richtung.** Detect checksum support from the first PROPFIND (any oc:checksums value) and report provides_content_hash accordingly; send OC-Checksum with uploads to Nextcloud/ownCloud.

**Gegenprüfung.** provides_content_hash() is always true for WebDAV (webdav.rs:470-476), so a paired local side gets HashMode::Full (snapshot_hash.rs:68-70) and reads files without a reusable previous hash (snapshot_dir.rs:163-176). stat() never returns a checksum (webdav.rs:267). The cost is mostly one-time per new or changed file, because hashes are reused through the baseline (snapshot_dir.rs:167-171; incremental_collect.rs:213-223). It repeats, however, on every run whose errors prevent a baseline save. Low fits.

## Y144 Android: syncs run without 'All files access'; the filtered storage view turns other apps' files into deletions on the backup side

- Schwere: high · Urteil: confirmed · Kategorie: data-safety · Plattformen: android
- Stellen: android/app/src/main/java/app/smartexplorer/android/ui/onboarding/OnboardingScreen.kt:42-50; android/app/src/main/java/app/smartexplorer/android/ui/onboarding/OnboardingScreen.kt:104; native/src/bisync/os/shared/incremental.rs:116-127; native/src/bisync/os/shared/orchestration.rs:209-234; native/src/bisync/os/shared/apply_delete.rs:226-237

**Beschreibung.** The app's only storage permission is MANAGE_EXTERNAL_STORAGE (there is no READ_MEDIA_* or READ_EXTERNAL_STORAGE). Permissions.hasAllFilesAccess() is used only by the Files-screen banner and the settings. It is not checked by the periodic SyncWorker, the catch-up task, the embedded daemon's own schedule or the manual sync.run. The host state passed to the daemon (power save, metered, defer) carries no storage-access bit. Without the special access, Android's MediaProvider/FUSE layer still lists directories, but only files this app owns, and it reports no error. The bisync walk therefore gets a valid-looking but filtered tree.

**Fehlerszenario.** 1. The user grants access and creates a two-way job /storage/emulated/0/DCIM <-> PC or NAS (or a Mirror job phone -> NAS) and syncs it.
2. Access is later lost: revoked in Settings > Special app access, or not re-granted after a reinstall or device migration.
3. The next periodic, persistent or manual run lists DCIM. Camera photos belong to the camera app, so they are invisible.
4. Two-way: every baseline file looks deleted on side A, so DeleteB is planned for all of them. Mirror ignores the baseline and deletes every B file as an 'extra'.
5. The delete guard is off by default, so the deletions run. Copies back to A then fail with EACCES, so the run ends with 'Fehler' only after the deletions have happened.
6. The reversible copies land in app-private storage (see the versions finding).

**Richtung.** Treat shared-storage access as a hard precondition. Report Environment.isExternalStorageManager() through sys.hostState and also check it before each worker or manual run. The daemon and the facade then refuse jobs with a shared-storage local endpoint (no walk, no deletion), record a visible 'Dateizugriff fehlt' result and notify the user. Also add a plausibility check that aborts when a side suddenly shows only app-owned files compared with the baseline.

**Gegenprüfung.** The core defect is real. The manifest requests only MANAGE_EXTERNAL_STORAGE (AndroidManifest.xml:17-19), and the build targets minSdk 30 / targetSdk 36, so scoped storage applies. Without the special access, MediaProvider's FUSE readdir returns directories plus caller-visible files only, and it raises no error.

No sync path checks the access:
- rg finds hasAllFilesAccess/rememberAllFilesAccess only in AccessBanner.kt:27-39, OnboardingScreen.kt:50, FilesScreen.kt:84 and SettingsScreen.kt:226. Nothing in work/, service/, ui/sync/ or api/SyncApi.kt uses it.
- SyncWorker.doWork (SyncWorker.kt:40-55) goes straight to HostMonitor.beginWorkerRun and SyncApi.catchUp.
- catch_up_task (background.rs:148-166) only calls wait_ready and request_catch_up.
- sync_run::run (sync_run.rs:166-177) checks only reject_app_internal, which covers ZIP and trash (args.rs:77-84), plus the settings.
- The daemon's job.rs:10-77 has no check either.
- HostState carries only power_save, metered and defer_scheduling (host_state.rs:16-23), which matches what HostMonitor.kt:163-171 reports.

The walk takes whatever read_dir lists: local_access linux_os.rs:16-55, snapshot_dir.rs:64-147. Camera files therefore vanish without an error. Mirror then plans DeleteB for every destination rel that is missing from the source (plan.rs:57-104). Two-way with a=None, a_changed and !b_changed plans DeleteB (plan.rs:155-166). The incremental Mirror for a local source, changes_from_source_walk (incremental.rs:116-127), reaches the same deletions.

The delete guard (orchestration.rs:209-234) is inactive by default: max_delete and max_delete_pct are 0 in syncjobs types.rs:178-179, editor.rs:116-117 and SyncApi.kt:76-77. Each delete is first backed up into versions_<pair> under HostConfig.data_home, i.e. app-private filesDir (apply_delete.rs:226-237, persistence.rs:59-61, support_dirs.rs:57-60).

Correction to step 2: 'not re-granted after a reinstall or device migration' does not apply. Jobs, baselines and versions all live in filesDir (syncjobs persistence.rs:13-25, support_dirs.rs:99-103), with allowBackup=false and data-extraction exclusions (AndroidManifest.xml:51-52), so they vanish with the uninstall and are never migrated. The valid triggers are:
- The user revokes the special access; the next periodic, persistent or manual run then proceeds.
- The access was never granted. Onboarding states both permissions are optional (OnboardingScreen.kt:42-46) and 'Weiter' always continues (:104). For example, a new phone→NAS Mirror into an existing backup folder deletes that backup.

The EACCES copy-back step is plausible but secondary: staged names like x.jpg.se-bisync-<hex> are not a media type for DCIM. High severity stands.

## Y145 The app's own state directory is not excluded from sync roots: real-time profile/home backups never fire and the version store re-syncs itself

- Schwere: high · Urteil: confirmed · Kategorie: reliability · Plattformen: windows, linux
- Stellen: native/src/support_dirs.rs:62-103; native/src/daemon/os/shared/state.rs:25-33; native/src/daemon/os/shared/state.rs:206-208; native/src/daemon/os/shared/state.rs:247-258; native/src/daemon/os/shared/run_loop.rs:203; native/src/daemon/os/shared/run_loop.rs:245-247; native/src/daemon/os/shared/run_loop.rs:284-298; native/src/daemon/os/shared/schedule.rs:15-55; native/src/bisync/os/shared/persistence.rs:55-61; native/src/bisync/os/shared/apply_transfer.rs:271-288; native/src/bisync/os/shared/state_store.rs:403; native/src/vfs/os/shared/sync_roots.rs:10-35; native/src/syncjobs/core/types.rs:158

**Beschreibung.** Desktop state lives under %APPDATA%\smart_explorer\sync on Windows and ~/.local/share/smart_explorer/sync on Linux. It contains daemon.heartbeat, daemon.log, results.tsv, jobs/*.conf, baseline_*.sebl, sync_state.sqlite and versions_<pair>/<unix-ts>/<rel>. That is inside the most common backup roots (C:\Users\<name>, ~). Hidden entries are included by default (include_hidden=true). Root validation only checks A against B for overlap. No walker excludes the data directory: not bisync, not the quick mirror, not tree_sig.

**Fehlerszenario.** (1) Real-time job on the profile or home folder:
- The desktop worker rewrites daemon.heartbeat every 2 s and once per tick.
- tree_sig includes every file's mtime, so the signature differs at every tick.
- rt_dirty_since is therefore reset at every tick and the job is never enqueued.
- Even without the heartbeat, each run's own mark_run, record_result and log writes change the signature and re-trigger a full run after every run.
(2) Interval, calendar or Mirror backup of the profile:
- Every overwrite or delete is backed up into versions_<pair> inside the source.
- The next run copies those versions to the target.
- When prune_versions removes them, Mirror or two-way deletes the target copies and backs them up again under versions_<pair>/<new ts>/AppData/.../versions_<pair>/<old ts>/....
- The nesting and storage grow without bound.
(3) The live sync_state.sqlite, the baseline and the log are copied while they are being written, which gives torn copies or per-file errors on every run.

**Richtung.** Exclude the app data and cache directories (canonical paths) from every walk: bisync snapshot, quick mirror, tree_sig and agent walks. Report them as protected omissions, or warn or refuse when a root contains them. Keep high-frequency control files such as the heartbeat out of user-visible trees. Allow the version store to live outside any source root.

**Gegenprüfung.** Where the state lives:
- support_dirs.rs:64-103 places the state in %APPDATA%\smart_explorer\sync or $XDG_DATA_HOME|~/.local/share/smart_explorer/sync.
- That directory holds: heartbeat, log and controls (state.rs:21-42), baseline and versions (persistence.rs:55-61), sqlite (state_store.rs:403), jobs/*.conf (syncjobs persistence.rs:13-25) and results.tsv (results.rs:23-25).

Nothing excludes it:
- snapshot_dir.rs:107-130 skips only hidden entries (when include_hidden is false), ignore globs, symlinks, the app trash and Android/data|obb (apptrash/mod.rs:32-68).
- validate_sync_roots (sync_roots.rs:10-35) and connect::validate_sync_endpoints (location.rs:142-151) compare only A with B.
- include_hidden defaults to true (types.rs:158).
- SYNC_FEATURES.md:103 lists 'our own version store' in the default ignore set, but the editor's defaults (job_editor_ui.rs:251-254) omit it and nothing applies it.

(1) Real-time never fires:
- enqueue_realtime_jobs (run_loop.rs:253-305) calls tree_sig (schedule.rs:15-55). tree_sig applies no hidden or ignore filter and returns the newest mtime.
- write_heartbeat runs after every pass (run_loop.rs:203) and in every 2 s slice of the desktop process (245-247).
- The signature therefore differs at every tick: 294-298 keeps resetting rt_dirty_since and 285-292 is never reached. A job ignore pattern cannot suppress this.
- After a run, mark_run, record_result and log re-dirty the tree anyway.

(2) Versions nest:
- back_up_captured writes versions_dir/<secs>/<rel> (apply_transfer.rs:271-300). These become new source files.
- prune_versions (persistence.rs:302-356) removes them from A. The next run then plans Mirror or two-way DeleteB (plan.rs:95-99, 155-166), and each delete is backed up again one level deeper. The content never leaves.

(3) Deterministic per-run error:
- daemon.heartbeat is planned with its walked signature, but the desktop daemon rewrites it every 2 s.
- capture() then reports drift (apply_guard.rs:78-81). That InvalidData error is not retried (apply_retry.rs:78-95).
- Every daemon run of a home or profile backup therefore ends in 'Fehler'. The baseline is not saved (orchestration.rs:312-320), and Mirror never bootstraps its incremental index (orchestration.rs:88-95).
- Torn copies are mostly turned into drift errors by revalidate (apply_transfer.rs:354-357).

Scope: jobs whose local root contains the data dir. Android is not affected because filesDir is app-private.

## Y146 Pre/post-run commands fail with quotes on Windows, run after endpoints are opened, are skipped on automatic pauses and updates, and never run for manual runs

- Schwere: medium · Urteil: confirmed · Kategorie: bug · Plattformen: windows, linux, android
- Stellen: native/src/daemon/os/windows/platform.rs:44-46; native/src/daemon/os/shared/job.rs:26-64; native/src/daemon/os/shared/job.rs:78-79; native/src/daemon/os/shared/job.rs:114-127; native/src/daemon/os/shared/job.rs:166-176; native/src/daemon/os/shared/run_loop.rs:153-159; native/src/daemon/os/shared/run_loop.rs:223-229; native/src/daemon/os/shared/run_loop.rs:438-448; native/src/app/os/shared/sync_jobs.rs:6-100; native/src/mobile/os/shared/domains/sync_run.rs:1-4; native/src/app/core/job_editor_ui.rs:308-313; native/src/main.rs:1

**Beschreibung.** (a) Command::new("cmd").args(["/C", cmd]) applies MSVC-CRT quoting to the whole command: it wraps the string in quotes and turns every embedded " into \". cmd.exe does not understand backslash escapes. With more than two quotes it strips only the first and the last, so the command it parses starts with \"C:\Program Files\...\". Rust documents CommandExt::raw_arg for exactly this cmd.exe /c case.
(b) Endpoints are resolved, which opens remote connections, before run_before runs.
(c) run_after is skipped whenever the run was cancelled. That includes automatic battery-saver or metered pauses (cancel_and_join when permit_mutation turns false), switching background sync off, and the daemon stop for an update handoff.
(d) The GUI 'Jetzt' run and Android sync.run never execute hooks.
(e) .status() has no timeout and cannot be interrupted by stop or pause; the join waits. The release daemon is a windows_subsystem process, so cmd.exe opens a visible console window, and closing it kills the hook. Hooks receive no information about the run result.

**Fehlerszenario.** 1. On Windows a job has the before-command "C:\Program Files\VeraCrypt\VeraCrypt.exe" /v D:\c.hc /l X /q, or net use X: "\\nas\my share".
2. cmd fails with a filename-syntax error, so every scheduled run ends with 'Befehl davor fehlgeschlagen' and the job never syncs.
3. A before-hook that wakes or VPN-connects the NAS never runs, because resolving the SFTP endpoint fails first. That failure is only logged.
4. A before-command that stops a database is never undone when the laptop's battery saver auto-pauses the run.
5. Clicking 'Jetzt' on a job whose before-command mounts the target syncs into the empty mount point.

**Richtung.** On Windows build the command line with raw_arg as cmd /d /s /c "<command>" and set CREATE_NO_WINDOW. Run run_before before resolving remote endpoints. Add a cleanup hook that also runs after a cancellation, with a bounded timeout. Apply the same hook policy to manual runs, or block manual runs of jobs that have hooks. Kill hooks on timeout or stop. Pass the job id and result to hooks through environment variables.

**Gegenprüfung.** (a) windows/platform.rs:44-46 uses Command::new("cmd").args(["/C", cmd]).
- cmd.exe is not a .bat/.cmd program, so Rust std applies CommandLineToArgvW quoting: the whole command becomes one quoted argument and every embedded " becomes \".
- With more than two quotes, cmd /C strips only the first and the last, leaving \"C:\Program Files\...\". That is invalid, and the 'net use X: "\\nas\my share"' example also fails.
- CommandExt::raw_arg is documented for exactly this cmd.exe /c case.
- Linux and Android use sh -c (linux platform.rs:35-37, android platform.rs:49-51) and are not affected.

(b) job.rs:26-45 resolves both endpoints before run_before (46-64).
- For remote endpoints, resolution opens connections: open_saved_at, open_gdrive, open_share_backend (resolution.rs:7-35).
- A failure only logs and returns (29-30, 39-40). No persist_attempt runs, so no result is recorded.

(c) run_after is skipped when was_canceled (job.rs:117). Cancellation comes from:
- auto-pause (run_loop.rs:155-159, 223-229)
- switching sync off (145-151)
- stop or update handoff (438-448)
This is documented intent (job.rs:114-116; tooltip 'nicht nach Abbruch' at job_editor_ui.rs:312). It is a limitation rather than a bug, but a before-hook's side effect is never undone.

(d) The GUI 'Jetzt' path (app/os/shared/sync_jobs.rs:6-100 → launch_bisync) and Android sync.run (sync_run.rs:1-4) run no hooks. This is documented ('nur Hintergrund-Dienst', job_editor_ui.rs:309).

(e) Blocking and visibility:
- run_cmd calls .status() without a timeout (job.rs:167-176).
- cancel_and_join joins with no timeout on the scheduler thread (job_supervisor.rs:119-135).
- The daemon is the GUI-subsystem exe (main.rs:1, lib.rs:78-80, autostart windows.rs:67-80), so cmd gets its own visible console.
- No run result is passed to hooks.

The factual claims all hold. Medium severity is right.

## Y147 Desktop line merge and 'Beide als getrennte Dateien' lose line endings, overwrite concurrent edits and keep no backup of the replaced versions

- Schwere: medium · Urteil: confirmed · Kategorie: data-safety · Plattformen: windows, linux, android
- Stellen: native/src/app/core/merge_ui.rs:114-122; native/src/app/core/bisync_merge.rs:115-162; native/src/linemerge/core/linemerge.rs:284-297; native/src/linemerge/core/linemerge.rs:299-362; native/src/vfs/os/shared/remote_util.rs:64-82; native/src/mobile/os/shared/domains/sync_merge.rs:47-58; native/src/mobile/os/shared/domains/sync_merge.rs:112-142

**Beschreibung.** Desktop builds the merged text with assemble_rows and the keep-both texts with side_a/side_b. All three rebuild the file from diff rows joined with '\n'. TextShape, which restores CRLF and the final newline, is applied only on Android. The desktop path writes with write_bytes (stage and replace) without first checking that A and B are unchanged since the merge was loaded; Android checks this with ensure_unchanged. Neither platform copies the replaced originals into the versions store, although the A/B conflict resolution promises 'Ersetzte Versionen werden gesichert'.

**Fehlerszenario.** 1. A CRLF Windows script or config conflict is merged on desktop. Both sides become LF without a final newline.
2. With 'Beide als getrennte Dateien', side A, which the user chose to keep unchanged, is rewritten the same way, and B's conflict copy is altered too.
3. If the file is edited on either side while the merge dialog is open, the edit is overwritten without any recovery copy.
4. A wrong row choice cannot be undone, because neither original is saved.

**Richtung.** Desktop: apply TextShape, and keep the raw bytes for keep-both. Add the size/mtime unchanged check before writing. On both platforms, back up both originals into versions_<pair> before replacing them, preferably through the guarded copy_replace path.

**Gegenprüfung.** merge_ui.rs:114-122 builds the merged text with assemble_rows and the keep-both texts with side_a/side_b. All three join with '\n' (linemerge.rs:284-297, 341-362). The rows come from str::lines, which drops \r\n and the final newline (linemerge.rs:138-139, 299-300). TextShape is used only in the Android facade (sync_merge.rs:135-137).

The desktop writes straight through write_bytes (bisync_merge.rs:127-128, 155-158) and does not first compare against the load-time state. write_bytes (remote_util.rs:64-82) stages and replaces, with no precondition and no backup. Android instead has ensure_unchanged (sync_merge.rs:47-58), and its keep-both writes the original texts byte for byte (150-167).

No versions-store copy is made on either platform for merge-apply. The conflict window's 'Ersetzte Versionen werden gesichert' (bisync_conflict_ui.rs:43) holds for the A/B resolution, which uses versions_dir (resolve.rs:34). Nuance: Android keep-both loses nothing, because B's original becomes the copy and A is untouched. Android merge-apply does replace both originals without a backup.

## Y148 Desktop preview 'Nur diese Datei jetzt synchronisieren' applies stale plan actions without planned-state guards and never updates the baseline

- Schwere: medium · Urteil: confirmed · Kategorie: data-safety · Plattformen: windows, linux
- Stellen: native/src/app/core/preview_core.rs:96-145; native/src/app/core/bisync_ui.rs:154-178; native/src/bisync/os/shared/apply.rs:249-297; native/src/bisync/os/shared/apply_guard.rs:15-20; native/src/bisync/os/shared/apply_guard.rs:66-90; native/src/bisync/core/plan.rs:143-184

**Beschreibung.** apply_one_action re-resolves both endpoints and calls the public bisync::apply. That function passes planned=None, so source and destination become ExpectedFile::Unknown, and capture() accepts whatever is currently there. The preview can be minutes or hours old. After the action, nothing writes the baseline.

**Fehlerszenario.** 1. The preview shows CopyAtoB(report.docx).
2. A colleague then edits report.docx on the NAS (side B).
3. Clicking the per-file button overwrites the newer B edit silently: only a versions-store copy remains and no conflict is raised.
4. A previewed DeleteB likewise deletes a B file that was modified after the preview.
5. Even when nothing changed in between, the next full run sees both sides changed relative to the old baseline. Copies have a new mtime and are not hashed by default, so this becomes a FileLevel conflict, or a needless re-copy under NewerWins.

**Richtung.** Keep the preview's planned trees and baseline, and apply through apply_planned_with_results with those expectations. After success, update that rel's baseline entry under the pair's exclusion. Refuse while the daemon is running the same pair.

**Gegenprüfung.** The ▶ button (bisync_ui.rs:154-179) calls apply_one_action (preview_core.rs:96-154).
- That calls crate::bisync::apply (apply.rs:249-272), which calls apply_inner(..., None) (275-297).
- ExpectedFile::from_tree(None) is Unknown (apply_guard.rs:15-20), and capture() accepts any current state (apply_guard.rs:74).
- Preview keeps no trees (preview.rs:24-52), so no planned state can be passed.
- No save_baseline runs (preview_core.rs:115-145).

With reversible=true (syncjobs types.rs:242, apply.rs:52), the overwritten newer B edit survives only in the versions store.

For copies, the next full run also misfires:
- Bisync copies do not preserve the source mtime: stage_source only streams (apply_transfer.rs:327-371), and no set_modified/FileTimes exists in production code.
- local, SFTP and similar sides have no hash under MtimeSize (snapshot_hash.rs:63-72).
- Both sides therefore differ from the baseline and sig_eq fails on mtime. The result is a FileLevel conflict (plan.rs:181-188) or a NewerWins re-copy (204-233).

DeleteB actions converge correctly afterwards, so the conflict claim applies to copy actions.

## Y149 'Reversible' version backups cannot be restored: no browse or restore path, opaque folders on desktop, unreachable app-private storage on Android

- Schwere: medium · Urteil: confirmed · Kategorie: gap · Plattformen: windows, linux, android
- Stellen: native/src/bisync/mod.rs:8-10; native/src/bisync/os/shared/persistence.rs:55-61; native/src/bisync/os/shared/apply_transfer.rs:281-288; native/src/support_dirs.rs:55-61; native/src/support_dirs.rs:89-103; native/src/syncjobs/core/types.rs:242; android/app/src/main/AndroidManifest.xml:51

**Beschreibung.** Every job overwrite or delete is backed up to <app data>/sync/versions_<16-hex pair hash>/<unix seconds>/<rel>, and reversible is always true. Neither the desktop GUI nor the Android UI can list or restore these versions; only bisync internals reference versions_dir. On Android the directory is under the app-private filesDir (HostConfig.data_home). The user and other apps cannot reach it, it is excluded from backup (allowBackup=false), and it is erased on uninstall or clear-data.

**Fehlerszenario.** 1. A Mirror run deletes thousands of photos on the NAS, for example through the Android access finding.
2. The 'reversible' copies exist only under /data/user/0/<pkg>/files/smart_explorer/sync/versions_<hash>/....
3. The user has no way to restore them, and reinstalling the app to fix things destroys them.
4. On desktop, the user would have to guess which hashed folder belongs to which job and decode unix-epoch folder names.

**Richtung.** Add a per-job 'Versionen' view that lists files by path and time and restores them to the original or a chosen location. Store a readable manifest (job name, ISO time, original path). On Android, offer a user-visible or exportable versions location, or keep the versions on the target side.

**Gegenprüfung.** reversible is always true (syncjobs types.rs:242). Backups go to versions_<16-hex pair>/<unix secs>/<rel> (persistence.rs:36-61, apply_transfer.rs:281-288).

rg finds versions_dir in app/ only as the apply argument in preview_core.rs:121, nowhere in mobile/, and in no Kotlin sync screen. There is no list or restore code. Wiederherstellen exists only for the trash.

On Android, data_home is filesDir (support_dirs.rs:8-10, 57-60), allowBackup is false (AndroidManifest.xml:51) and the data-extraction rules exclude it. The store is unreachable to the user and lost on uninstall or clear-data.

On desktop the folders can be browsed by hand under %APPDATA% or ~/.local/share, even with Smart Explorer itself. They are opaque, though: hashed pair names, epoch-second folders and no mapping to jobs. This contradicts the promise in bisync/mod.rs:8-10 that 'any sync action can be undone'. Medium severity is right.

## Y150 Deleting or retargeting a job leaves its baseline and version store forever; a new job with the same endpoints inherits the stale baseline

- Schwere: medium · Urteil: confirmed · Kategorie: data-safety · Plattformen: windows, linux, android
- Stellen: native/src/syncjobs/os/shared/persistence.rs:151-163; native/src/app/core/menus_sync_jobs.rs:214-221; native/src/mobile/os/shared/domains/sync_jobs.rs:162-175; native/src/bisync/os/shared/persistence.rs:21-61; native/src/bisync/os/shared/orchestration.rs:108-111; native/src/bisync/os/shared/orchestration.rs:400-407

**Beschreibung.** State is keyed by pair identity (backend identity plus roots), not by job. Removing a job deletes only <id>.conf. prune_versions runs only for the pair that is being synced, so the versions of deleted or retargeted jobs are never pruned. A later job with the same endpoint strings loads the old baseline_<pair>.sebl; this applies to two-way and Propagate/NoDelete modes, since Mirror ignores the baseline.

**Fehlerszenario.** (a) A backup job is retargeted from D:\Backup to E:\Backup, or deleted. Gigabytes of versions_<old pair> stay on the system drive (on Android, internal storage) indefinitely.
(b) A two-way job Laptop:Docs <-> NAS:Docs is deleted in January. The user then trims files on the laptop, expecting the NAS to keep its archive. In June a new two-way job with the same folders is created, expecting a merge. The January baseline turns every laptop-side deletion since then into DeleteB on the NAS, and the delete guard is off by default.

**Richtung.** When a job is deleted or retargeted, offer to delete or archive its pair state. Bind the baseline to the job (job id or creation stamp) and treat a foreign or stale baseline as absent (fresh union) unless the user explicitly chooses to continue. Add a housekeeping pass that prunes orphaned version stores by retention.

**Gegenprüfung.** Removing a job:
- remove() deletes only <id>.conf (syncjobs persistence.rs:151-163).
- The desktop caller is menus_sync_jobs.rs:214-221. The Android caller (sync_jobs.rs:163-175) also forgets the in-memory conflicts only.
- No code deletes baseline_* or versions_*. forget_pair is called only on the incremental untrusted-record path (incremental.rs:359).
- No baseline handling exists on job save or delete (rg).

State is keyed by pair, not by job:
- pair_id_for hashes only backend state_identity plus the roots (persistence.rs:25-46).
- prune_versions runs only for the pair being synced (orchestration.rs:108-110, 400-407), so versions of deleted or retargeted pairs are never pruned.
- A new job with the same endpoints loads the old baseline (orchestration.rs:111).
- In two-way Propagate, laptop-side deletions since then are planned as DeleteB (plan.rs:155-166), with the delete guard off by default.
- Mirror ignores the baseline and NoDelete never deletes, as the finding says.

## Y151 Unattended runs never alert the user; the log is wiped at 256 KiB and concurrent writers can overwrite each other's results

- Schwere: medium · Urteil: confirmed · Kategorie: gap · Plattformen: windows, linux, android
- Stellen: native/src/daemon/os/shared/job.rs:26-42; native/src/daemon/os/shared/job_supervisor.rs:9; native/src/syncjobs/core/schedule.rs:41-58; native/src/app/core/bisync_ui.rs:205-217

**Beschreibung.** Background results only go to results.tsv and daemon.log. Desktop has no OS notification or tray signal. Android has no notification for failed or conflicted sync runs: the channels are transfers, background, updates, share and exec, and the IDs cover only ongoing work. Results are visible only inside the opened app, and nothing warns when the last successful run is old. log() truncates daemon.log to empty once it exceeds 256 KiB. record_result does an unlocked load-modify-write of one shared results.tsv from the daemon process, the GUI process, the Android facade thread and the embedded daemon thread.

**Fehlerszenario.** 1. A NAS password change or a full target makes the nightly backup fail for weeks. Nobody notices until a restore is needed.
2. A noisy real-time job pushes the log past 256 KiB, which wipes the earlier failure lines.
3. A manual run that finishes at the same moment as a background run of another job overwrites that job's newer 'Fehler' result with the older 'ok'.

**Richtung.** Notify on failed, conflicted or omitted runs and on 'no successful run for N intervals' (an Android channel 'Sync-Probleme'; a desktop toast or tray, also from the daemon). Rotate logs instead of truncating them. Store results per job, or take a file lock around the read-modify-write.

**Gegenprüfung.** No unattended run alerts anyone:
- The Android channels and IDs (Notifications.kt:13-30) cover transfers, background, updates, share and exec, with IDs for ongoing work only.
- SyncWorker only logs the outcome (SyncWorker.kt:74-75, 89-90).
- persistentText shows counts and the active job only (BackgroundText.kt:52-58).
- The desktop has no notification or tray code for sync. mount/os/windows/notifications.rs is Dokany metadata, not user toasts.
- Results surface only in-app: landing.rs:82 and menus_sync_jobs.rs:180-187.

The log is wiped: log() truncates to empty above 256 KiB (state.rs:11, 247-250).

Result writes can be lost:
- record_result_to (results.rs:44-65) is an unlocked read-modify-write followed by an atomic rename.
- Writers are the daemon (job.rs:185-200), the GUI (bisync_ui.rs:205-217), the Android facade (sync_run.rs:198-212) and the embedded daemon in the same Android process.
- A lost update is possible, though the window is narrow.

Amplification beyond the finding: daemon endpoint-resolution failures (NAS password change, host offline) return at job.rs:26-42 without persist_attempt or mark_run.
- results.tsv keeps showing the previous 'ok'.
- The job stays due: Interval at syncjobs schedule.rs:41-46, Calendar with catch_up at 47-58.
- It is re-run every 60 s (job_supervisor.rs:9, 73-79), logging about two lines per attempt, which speeds up the 256 KiB wipe. Scenario 1 is therefore worse than described.

## Y152 Linux: sync walks and the real-time signature cross mount points inside the tree; an unmounted nested mount looks like a mass deletion and a hung network mount hangs the run

- Schwere: medium · Urteil: confirmed · Kategorie: platform · Plattformen: linux
- Stellen: native/src/daemon/os/shared/run_loop.rs:184-191; native/src/daemon/os/shared/run_loop.rs:273-276; native/src/local_access/os/linux_os.rs:16-55; native/src/bisync/os/shared/snapshot_dir.rs:131-134

**Beschreibung.** No walker compares device ids (st_dev) or detects mount points: not the bisync snapshot, not the quick mirror scan, not tree_sig. On Windows, mounted folders are name-surrogate reparse points and are protected as links. On Linux a mount point is an ordinary directory, so the walk descends into FUSE, NFS, SMB or encrypted-vault mounts below the root.

**Fehlerszenario.** 1. A two-way backup of ~ runs while ~/Vault (gocryptfs/Cryptomator) or ~/nas (sshfs/rclone/NFS) is mounted, so the mounted contents are synced.
2. A later run happens while that mount is absent. The empty mount-point directory makes every file below it look deleted on A, and the files are deleted on the target.
3. A stale sshfs or NFS hard mount blocks read_dir/stat indefinitely, which hangs the daemon's single job slot and the real-time signature.

**Richtung.** Record the root's st_dev and treat child directories on other devices as protected omissions, like links, with an explicit opt-in to cross file systems. Use the same rule in tree_sig and the agent walks.

**Gegenprüfung.** No walker notices mount points on Linux:
- The Linux adapter has is_reparse_point()=false (vfs/os/linux_os/local_platform.rs:4-10).
- read_directory uses read_dir plus lstat with no st_dev comparison (local_access/os/linux_os.rs:16-55).
- snapshot_dir.rs:131-134 queues every non-symlink directory.
- tree_sig skips only link-like entries (schedule.rs:31-40).
- The only dev() use is volume_key (local_platform.rs:19-31).
- On Windows, by contrast, mount points are reparse points and become protected omissions.

An absent nested mount leaves an empty directory, so its files are planned as deletions (plan.rs:155-166 two-way, 95-99 Mirror) with the guard off by default.

Hung mounts block the daemon:
- Blocking std::fs calls on a hung hard NFS or sshfs mount cannot be cancelled.
- cancel_and_join waits without a timeout (job_supervisor.rs:119-135).
- tree_sig runs synchronously on the scheduler thread (run_loop.rs:184-191 → 273-276). A hung mount below a real-time root therefore freezes the whole daemon loop (heartbeat, stop handling, catch-up), not just the job slot.

The same failure class also covers a job root that is itself an unmounted fstab mount-point directory.

## Y153 Sync ignore patterns are case-sensitive even where the file system is case-insensitive

- Schwere: low · Urteil: confirmed · Kategorie: platform · Plattformen: windows, android
- Stellen: native/src/app/core/sync_core.rs:148

**Beschreibung.** compile_ignore_patterns uses globset::Glob::new, which is case-sensitive. The explorer filter uses GlobBuilder::case_insensitive(true). Windows and Android shared storage are case-insensitive, and the built-in default exclusions are lowercase literals.

**Fehlerszenario.** 1. **/*.iso does not exclude INSTALL.ISO.
2. The defaults **/desktop.ini, **/Thumbs.db and **/*.tmp miss Desktop.ini, thumbs.db and X.TMP.
3. On a pair whose sides differ only in letter case, a pattern excludes the file on one side only. The result is a one-sided filter with drift errors or one-sided deletions, the same effect as the already-reported size and age filters.

**Richtung.** Compile the ignore set case-insensitively when either backend reports case-insensitive paths (Backend::case_sensitive_paths), at least on Windows and Android.

**Gegenprüfung.** Matching is case-sensitive in all sync paths:
- validation.rs:174 compiles each pattern with globset::Glob::new, which is case-sensitive by default.
- The GUI's launch_bisync also uses Glob::new (sync_core.rs:148).
- The explorer filter, by contrast, uses GlobBuilder::case_insensitive(true) (filter.rs:77).
- WalkFilter::ignored matches the on-disk relative name (snapshot.rs:33-36).
- With fold_case pairs (snapshot_pair.rs:62), the filter is still applied case-sensitively on each side.

Impact is limited:
- The default exclusions are added only via a button (job_editor_ui.rs:250-262).
- They are mixed-case literals ('**/Thumbs.db', '**/desktop.ini', '**/*.tmp'), not all lowercase as stated.
- Windows' usual spellings match them; only differently cased names (X.TMP, Desktop.ini) slip through.

Low severity is right.

## Y154 Target file-system limits are not checked: files over 4 GiB sent to FAT32 sticks or SD cards fail only after streaming 4 GiB, on every run

- Schwere: low · Urteil: confirmed · Kategorie: gap · Plattformen: windows, linux, android
- Stellen: native/src/bisync/os/shared/apply_transfer.rs:327-372; native/src/copy/os/shared/staging.rs:192-203

**Beschreibung.** stage_source streams the whole source into the stage file with no check against the destination file system's maximum file size. The copy engine classifies FileTooLarge as a target failure, but sync has no equivalent; it does not pre-check or mark such files as a permanent skip.

**Fehlerszenario.** 1. A Mirror backup of ~/Videos targets a FAT32 USB stick or a FAT32 SD card on Android, and one video is 6 GB.
2. Every run writes 4 GiB, then fails with EFBIG or a file-system-limit error.
3. The run is recorded as 'Fehler', with the already-reported all-or-nothing baseline consequences.

**Richtung.** Detect the target file system (statvfs/GetVolumeInformation) or classify FileTooLarge once. Then skip such files before transferring and report them as 'zu groß für Ziel-Dateisystem' omissions instead of run errors.

**Gegenprüfung.** stage_source (apply_transfer.rs:327-371) streams the whole source into the stage file until the write fails. There is no size or limit pre-check, and rg finds no FAT32, EFBIG or max-size handling in bisync/ or sync/.

FileTooLarge is not transient (apply_retry.rs:78-95), so there is no retry within a run. The next run repeats the full transfer. Any error keeps the old baseline (orchestration.rs:312-320).

The copy engine's side_of (staging.rs:192-203) also only classifies the error after the fact. Low severity is right.

## Y155 The 'Überprüfen' option only re-checks the destination size; no content verification exists for backups

- Schwere: low · Urteil: confirmed · Kategorie: gap · Plattformen: windows, linux, android
- Stellen: docs/SYNC_FEATURES.md:186

**Beschreibung.** verify_copy compares the promoted file's stat size with the bytes written. stage_source already performs the same size check before promotion, so the option adds no integrity guarantee. SYNC_FEATURES lists I2 'Verify after copy (re-read/hash)' as shipped.

**Fehlerszenario.** A flaky USB stick or NAS write path corrupts bytes. The run with 'Überprüfen' enabled passes, and the corruption is found only at restore time.

**Richtung.** Hash while streaming, then re-read the promoted destination (or use the server's native hash) and compare. Keep a size-only mode as the cheap default.

**Gegenprüfung.** verify_copy (apply_transfer.rs:239-249) is a stat-size comparison of the promoted file. stage_source already performs the same staged-size check before promotion (apply_transfer.rs:358-361). The MD5 in stream() (377-412) only checks the source bytes against a planned nonzero hash; it never re-reads the destination.

The tooltip honestly says 'Größe nach dem Kopieren prüfen' (job_editor_ui.rs:293). The docs disagree: docs/SYNC_FEATURES.md:117 defines I2 as re-read/hash, and :186 marks verify as shipped. Low severity is right.

## Y156 Closing the desktop window or applying an update during a manual sync, merge or single-file apply kills the worker without warning or waiting

- Schwere: low · Urteil: confirmed · Kategorie: reliability · Plattformen: windows, linux
- Stellen: native/src/app/core/sync_core.rs:180-197; native/src/vfs/core/promotion.rs:30-33; native/src/bisync/os/shared/snapshot_dir.rs:107-130

**Beschreibung.** prepare_for_exit only sets the sync, bisync and preview cancel flags and drops the handles. The process then exits without joining the bisync, merge-apply, apply-one or resolution threads. There is no close confirmation while sync work is running, and the update restart uses the same path.

**Fehlerszenario.** 1. The user closes the window during a long manual backup, or clicks 'restart to update'.
2. The worker is killed mid-copy, mid-promotion or between the two write_bytes calls of a merge.
3. Stage leftovers, a missing baseline save and a half-applied merge follow, with the already-reported next-run consequences.

**Richtung.** When sync work is running, ask on close or update. Then cancel and join with a bounded wait, or hand the run to the daemon, before exiting.

**Gegenprüfung.** prepare_for_exit (shutdown.rs:8-77) only sets the sync, bisync and preview cancel flags (54-62) and drops conflict_resolution (63). ConflictResolutionTask's Drop joins only workers that have already finished (bisync_conflicts.rs:76-89).

No thread handles are kept:
- launch_bisync discards its JoinHandle (sync_core.rs:180-197).
- merge-apply and keep-both keep none (bisync_merge.rs:120-133, 147-163).
- apply-one keeps none (preview_core.rs:113-148).

Both exits take the same path: on_exit calls prepare_for_exit(false) (frame_update.rs:5-7), and the update restart calls prepare_for_update_apply (dialogs.rs:103-115 → shutdown.rs:4-6). rg finds no close_requested or CancelClose handling.

The consequences are concrete:
- Stage files '<name>.se-bisync-<hex>' (promotion.rs:30-33) are not excluded by the walk (snapshot_dir.rs:107-130), and no stale-stage cleanup exists. In two-way mode a killed copy's stage file is synced to the other side.
- Because the baseline is not saved and mtimes are not preserved, every file copied before the kill comes back as a both-changed conflict.

Low is defensible, since no data is lost.

## Widerlegt

- (apply-and-mirror) Desktop 'Spiegeln nach...' reads a remote source through the 20-second browsing cache: start_mirror gets its source from pane_backend (sync_core.rs:11), and pane_backend returns `(crate::vfs::sync_backend(backend), root.clone())` (sync_core.rs:239). sync_backend unwraps through uncached_backend (vfs/os/shared/sync_roots.rs:6-8), and CachingBackend::uncached_backend returns the inner backend (vfs/core/cache.rs:123-129). The mirror source is therefore uncached, as on Android.
- (remote-targets) WebDAV remove_dir is a recursive DELETE (Drive trashes the subtree), so quick-mirror directory deletion can remove retained or newly created files: WebDAV remove_dir is indeed a DELETE (webdav.rs:392-394), and Drive trashes folders (gdrive backend.rs:312-314). But the cited quick-mirror deletion path is dead code in production. delete_extras runs only when opts.delete_extra is true (sync/os/shared/sync.rs:271-289), and both production callers hard-code delete_extra: false (app/core/sync_core.rs:19-22; mobile/os/shared/domains/sync_run.rs:294-297). Bisync deletes files only (apply_delete.rs:111-115). The stated scenario cannot occur. It remains a latent hazard if extra-deletion is ever enabled.

## Offene Fragen der Prüfer

- [triggers-realtime] Whether OneDrive/Cloud Files placeholders (files and folders) keep a cloud reparse tag when hydrated or pinned in current OneDrive versions; this determines how much of a OneDrive folder is invisible to the real-time signature (needs a Windows check, e.g. fsutil reparsepoint query).
- [triggers-realtime] Whether the project's Rust toolchain still implements std::fs::symlink_metadata on Windows by opening a handle per path or uses a GetFileInformationByName fast path; affects only the cost estimate of the polling finding.
- [triggers-realtime] bisync behaviour when two runs overlap on the same pair, and whether a cancelled run saves a partial baseline or checkpoint (affects the manual-vs-background exclusion and Android 10-minute-cap findings) - engine reviewer's scope.
- [triggers-realtime] Whether any VFS backend (WebDAV, SFTP, SMB, Share) can block indefinitely without an I/O timeout, which would turn the missing watchdog into a hard hang.
- [triggers-realtime] Whether GetVolumeInformationW on media-less card readers can raise the 'no disk' hard-error dialog for the GUI-subsystem worker on current Windows builds.
- [triggers-realtime] How long the synchronous Share initial load takes on Android cold starts relative to READY_WAIT (10 s).
- [triggers-realtime] Exact behaviour of WorkManager setForeground from periodic workers on OEM Android ROMs and its interplay with the Android 15 dataSync 6-hour limit.
- [triggers-realtime] For the recommended OS-native watchers: reliability of inotify on Android /storage/emulated (FUSE) for changes made by other apps, and inotify watch limits on very large Linux trees, need verification before choosing the design.
- [triggers-realtime] Product intent for desktop 'Beim Start': once per logon/boot or once per worker start.
- [triggers-realtime] #7: Whether the Cloud Files filter shows the reparse-point attribute of hydrated or pinned OneDrive placeholders to the unpackaged daemon process (its placeholder compatibility mode) needs a runtime check on Windows. The impact on WOF-compressed and deduplicated files is certain from the code.
- [triggers-realtime] #9: Whether GetVolumeInformationW on an empty card-reader slot raises the 'no disk' error dialog for the daemon on current Windows was not verified. No SetErrorMode exists in native/src.
- [triggers-realtime] #5: Actual tree_sig durations (NTFS with Defender, SMB round-trip time, 1M-entry trees) were estimated, not measured. Measurement would show how often the 60 s heartbeat threshold and the 300 s handoff timeout are crossed.
- [triggers-realtime] #5/#20 additional observation, not in any finding: if the old worker stays busy longer than DAEMON_HANDOFF_TIMEOUT (300 s), the replacement exits but the handoff control remains, so the old worker exits afterwards too. No background worker then runs until the next GUI start or logon (handoff.rs:131-133, 158-164).
- [triggers-realtime] #17: The exact outcome of two overlapping bisync runs on one pair (apply guards, duplicate transfers, last-writer-wins baseline) was not traced through apply_guard.rs or apply.rs.
- [triggers-realtime] #20: Which backends or OS paths can block without any deadline (hard NFS mounts, FUSE, Share peer streams, FTP) was not traced. WebDAV and SFTP do have timeouts.
- [triggers-realtime] #10/#11/#22: Android runtime behavior needs device verification: CPU suspend under the specialUse foreground service with no wake lock, setForeground refusal from a background worker, and the cached-app freezer on the target OEM builds.
- [bisync-plan-state] Whether the desktop GUI and the daemon can actually run the same job concurrently (IPC hand-off/job locking outside the read surface was not examined); affects the stale-baseline overwrite finding.
- [bisync-plan-state] Exact FAT timestamp behavior on Linux/Android depends on vfat mount options (tz/time_offset); only the Windows behavior is asserted.
- [bisync-plan-state] Whether the WebDAV/Nextcloud servers in use normalize names to NFC (assumption in the Unicode finding).
- [bisync-plan-state] suppaftp's LIST year inference and whether target FTP servers offer MLSD/MDTM were not verified.
- [bisync-plan-state] How the real-time watcher schedules follow-up runs (outside this dimension); determines how often the error-freeze and concurrent-edit findings manifest.
- [bisync-plan-state] Whether Windows cloud placeholders (OneDrive Files On-Demand) are classified as link-like; if not, local Full hashing would hydrate them. Not examined.
- [bisync-plan-state] rusqlite's default busy timeout (believed 5 s) was not confirmed in the vendored crate.
- [bisync-plan-state] Share/peer endpoints (EndpointSpec::Peer) were not examined for timestamp precision or hash support.
- [bisync-plan-state] Memory footprint of ~1M-entry trees on Android was not measured; relevant when raising the walk caps.
- [bisync-plan-state] Defect not in the list (affects #0 and #5): a file name containing ':' passes the snapshot walk, because validate_child_name (vfs/core/delete.rs:318-327) does not reject ':'. save_baseline, however, validates every rel with ValidatedRelativePath, which rejects ':' (agent_proto/core/relative_path.rs:41-43; bisync persistence.rs:228, 296-298). Every run of a Linux/Android/SFTP pair containing such a file therefore ends with 'Synchronisierungsstand konnte nicht gespeichert werden', and the baseline never advances. It should be tracked as its own finding.
- [bisync-plan-state] #5 Windows example: whether 'System Volume Information' can be listed depends on the SeBackupPrivilege or broker-grant fallback in local_access/os/windows/read.rs:29-41. Whether typical users or the daemon hold that privilege was not verified.
- [bisync-plan-state] #19: whether the WebDAV/Nextcloud server normalizes names to NFC is external behavior and cannot be checked from this repository.
- [bisync-plan-state] #1 scope: whether real WebDAV sources supply oc:checksums decides whether WebDAV->local Mirror re-copies everything. Files with checksums converge by hash; files without them re-copy.
- [bisync-plan-state] #20: a FIFO-blocked job stalls the globally serialized daemon queue and its stop (job_supervisor.rs:32-33, 127). Its severity may deserve high if FIFOs in synced home folders (e.g. ~/.steam/steam.pipe with include_hidden=true) are common.
- [apply-and-mirror] Which io::ErrorKind the toolchain maps Windows ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION to was not verified; either way no retry path covers them in apply_retry.rs.
- [apply-and-mirror] Filesystem crash semantics behind the durability finding (XFS/NTFS/FAT/exFAT vs ext4 auto_da_alloc) are platform knowledge, not reproduced.
- [apply-and-mirror] How often WorkManager stops a long SyncWorker run on real devices (foreground promotion refused on Android 12+, dataSync foreground limits on newer Android) was not measured; the code shows that every stop cancels the run and discards its progress.
- [apply-and-mirror] The FTP timing estimate (listing cost per LIST) depends on the server and was not measured.
- [apply-and-mirror] Whether the case-folding walk (fold_case) reports case collisions before apply was not reviewed (scan dimension).
- [apply-and-mirror] The 1M-entry bisync walk cap and per-side filter evaluation sit in the scan/planning dimension; their fixes need coordination with that review.
- [apply-and-mirror] Whether stale NTFS/SMB directory entries for hard-linked files (noted in sync_pass.rs:156-161) make bisync's exact-mtime capture report drift was not verified.
- [apply-and-mirror] Behavior of the trash crate on Windows network/UNC paths when 'Papierkorb' is enabled was not verified (a versions copy is made first in any case).
- [apply-and-mirror] Whether a job whose remote side is a Share peer (agent backend) implements no-replace promotion and durable stages was not checked.
- [apply-and-mirror] #5 impact depends on filesystem and OS writeback ordering (ext4 auto_da_alloc only covers replace-over-existing; XFS/NTFS/exFAT; Windows quick-removal policy for USB), which the code cannot decide; the missing fsync itself is certain.
- [apply-and-mirror] #18 relies on Rust std mapping Windows ERROR_SHARING_VIOLATION/ERROR_LOCK_VIOLATION to an uncategorized ErrorKind (standard-library behavior, not repository code).
- [apply-and-mirror] #10: on Android the location of tempfile::tempfile() depends on the TMPDIR the host sets; not verified.
- [apply-and-mirror] #12: not checked whether the agent's server-side WalkHashed fast path (snapshot.rs:178-184) enforces the same 1M cap.
- [apply-and-mirror] Out of scope but relevant to real-time reliability: real-time detection is a 15 s polling signature (file count, newest file mtime, total bytes) over local roots (daemon/os/shared/schedule.rs:13-55, run_loop.rs:273-283). Renames or moves inside the tree, timestamp-preserving same-size writes, and delete+add pairs with equal size and older mtime leave the signature unchanged and are never detected. Every real-time job also re-stats up to 1M entries per tick.
- [local-platforms] Android 11+ device behaviour when listing <volume>/Android/data|obb through FUSE with all-files access: whether per-entry lstat fails with EACCES (LocalBackend::list_dir would then fail the whole directory and abort the walk before the protected-omission logic runs) or with ENOENT (entries silently dropped). Needs device verification.
- [local-platforms] Which NTFS driver the targeted Linux desktops use for USB disks (ntfs-3g/fuseblk vs in-kernel ntfs3) - decides whether NTFS targets are hit by the RENAME_NOREPLACE finding; NFS and sshfs exposure is confirmed from upstream sources.
- [local-platforms] Whether MediaProvider allows all-files-access apps to create stage names with non-media extensions inside DCIM/Pictures and rename them to media names (assumed yes because Android sync is in use).
- [local-platforms] The ':' -> NTFS alternate-data-stream outcome and timestamp side effects on an existing unrelated file are derived from NTFS semantics, not tested on a device.
- [local-platforms] FAT32 DST behaviour is taken from Microsoft documentation (File Times) and robocopy's /DST rationale, not tested.
- [local-platforms] Read-only replace failure of std::fs::rename is inferred from the upstream std source (fallback lacks FILE_RENAME_FLAG_IGNORE_READONLY_ATTRIBUTE) and the documented FILE_RENAME_INFORMATION semantics; no rust-toolchain pin exists in native/, so the CI's stable std is assumed.
- [local-platforms] Whether the UI offers bulk conflict resolution - determines how severe the post-failure conflict flood is in practice.
- [local-platforms] The real-time trigger lives in daemon/ (outside the assigned read scope); it was included because it consumes local listing/stat primitives and the user reported real-time unreliability. A full watcher design review belongs to the corresponding dimension.
- [local-platforms] Hard-mounted NFS/SMB targets that stop responding block sync threads in uninterruptible kernel waits; whether the daemon supervisor ever times out such a job was not checked.
- [local-platforms] #14: Rust std's Windows fs::rename falls back to FileRenameInfoEx on ERROR_ACCESS_DENIED. Whether that fallback ignores the read-only attribute (FILE_RENAME_FLAG_IGNORE_READONLY_ATTRIBUTE) is internal to std, outside the permitted read scope, and decides whether the bisync/mirror replacement half of the finding is real.
- [local-platforms] #13: Whether OneDrive Files-On-Demand placeholders expose FILE_ATTRIBUTE_REPARSE_POINT to this unpackaged process depends on Windows' default placeholder compatibility mode. The app never sets that mode and native/app.manifest has no opt-in. This needs a test on a real OneDrive folder.
- [local-platforms] #23: Whether trash::delete deletes permanently or fails on volumes without a recycle bin (UNC shares, removable sticks, Linux mounts without a writable topdir trash) depends on the trash crate's flags, which were not checked.
- [local-platforms] #0: For NTFS USB disks, impact depends on whether the target distros mount them with ntfs-3g (FUSE protocol minor below 23, so EINVAL) or with the in-kernel ntfs3 driver (supports NOREPLACE). NFS and sshfs fail regardless.
- [local-platforms] #12: Whether NTFS updates the LastWriteTime of a base file when an alternate data stream is created (the 'unrelated file touched' part) is an OS-behaviour assumption.
- [local-platforms] #7 side question: on Android, run_loop.rs:171 enqueues scheduled and real-time jobs only while the host does not defer scheduling. The Kotlin side that sets deferScheduling was not examined, so whether real-time jobs are evaluated at all in the background on Android is open.
- [local-platforms] #9: The SMB cost estimate assumes about one round trip per component; the real figure depends on the Windows SMB client's metadata and handle caching and was not measured.
- [remote-targets] Confirm on a live Nextcloud/ownCloud that MKCOL and PROPFIND on '/remote.php' fail (expected 404 'Path not found'); this decides whether the WebDAV mkdir_all finding blocks all sync uploads or only adds per-file overhead.
- [remote-targets] Confirm the ureq 2.12.1 Response::into_string 10 MB cap (crate source was outside the read surface).
- [remote-targets] hickory-resolver 0.26 system configuration on Android (FTP host names and Share signaling use builder_tokio()): does it work without /etc/resolv.conf?
- [remote-targets] suppaftp LIST parsing: year inference for HH:MM rows and handling of 'total N' lines; OpenSSH names per READDIR batch (numbers used in the SFTP 20 s finding).
- [remote-targets] Whether Nextcloud stores checksums for plain PUT uploads without an OC-Checksum header (affects how often the mtime/Mirror finding hits WebDAV).
- [remote-targets] Drive's 750 GB/day upload cap: which error reason it returns and whether it is classified as congestion, which would make unattended Drive backups back off for the rest of the day.
- [remote-targets] smb2 request and negotiate timeouts against a blackholed NAS could not be verified without the crate source.
- [remote-targets] Whether the 60 s retry loop for unresolved endpoints also runs on Android, where the host may defer scheduling to WorkManager (run_loop may_schedule).
- [remote-targets] #1 depends on the server answer for MKCOL/PROPFIND on '/remote.php'. The 404 comes from knowledge of Nextcloud and ownCloud remote.php (RemoteException 'Path not found', 404) and was not checked against a live server. #25, #28 and #30 likewise depend on NAT timeouts, IIS RNTO semantics and which SFTP servers implement posix-rename. #9's Fedora Atomic weighting assumes $HOME=/var/home/<user> there.
- [remote-targets] Additional defect outside the list: any per-file error discards the whole new baseline. On errors, orchestration.rs:310-320 returns `baseline: base` before update_baseline and save_baseline (386-399), dropping report.completed. In two-way jobs (default Direction::Both with FileLevel conflicts), every file copied in that run has no baseline entry next run, and its mtimes differ (#0), so plan.rs:143-153 and 181-188 report it as a conflict. One persistent bad file (#11, #21, #24, #27) therefore freezes the baseline permanently. The unit test bisync tests/safety.rs:308-357 uses a helper that does save the partial baseline, unlike production.
- [remote-targets] Additional observation (#2 follow-up): quick mirror ('Spiegeln') into a plain-SFTP destination fails for every new file at sync_copy.rs:143-149, because SFTP stat never returns NotFound and plain SFTP is the default (connect/core/types.rs:62).
- [remote-targets] Real-time triggering (part of this area, not in the list): tree_sig (daemon/os/shared/schedule.rs:15-53) signs only file count, newest mtime and total bytes, so pure renames and moves never trigger. It re-walks each local root (up to 1M entries) every 15 s tick (state.rs:7). Remote sides never trigger except a Drive Mirror source. For a realtime Mirror job with a remote source, remote_change_token calls resolve_endpoint, a full connect and login, on the daemon loop every tick even for backends without a change feed (run_loop.rs:269; schedule.rs:73-90). Drive's start page token is account-wide, so any account change causes a full Drive walk.
- [critic] Android scoped-storage behaviour (MediaProvider/FUSE returning only app-owned files in listings without MANAGE_EXTERNAL_STORAGE) is taken from platform knowledge and the app's own banner text. It was not reproduced on a device. Whether Android kills the process when the access is revoked was also not verified.
- [critic] Rust std's MSVC-CRT quoting of Command::args for cmd.exe could not be read locally because rust-src is not installed. The finding relies on documented std behaviour: CommandExt::raw_arg is documented for 'cmd.exe /c'.
- [critic] The exact Win32 error for writes past 4 GiB on FAT32 was not verified (no local execution allowed).
- [critic] Not examined: sync jobs that target drive letters or mount points served by the app's own daemon mount manager. One open question there is the order at daemon stop, where stop_mounts runs before cancel_and_join for a job writing into such a mount.
- [critic] Not examined: Windows Controlled Folder Access, or antivirus blocking writes into protected folders, as a failure mode of B->A restores. Also not examined: Share-server/iroh internals and the detailed remote backends, which were covered by other dimensions.
- [critic] Linux users with XDG_DATA_HOME outside $HOME avoid the self-inclusion of the app's state directory. How common that configuration is was not assessed.
- [critic] Out of scope, needs confirmation (two-run test): a full-path Mirror may not be idempotent.
- Bisync copies never preserve the source mtime: no set_modified, FileTimes, SetFileTime or utimensat in production code, and stage_source only streams (apply_transfer.rs:327-371).
- The Mirror planner compares A with B directly under MtimeSize with modify_window 0 (plan.rs:57-104, 16-42).
- local, SFTP, FTP, SMB and Share sides carry no hash (snapshot_hash.rs:63-72; only Drive and Nextcloud-WebDAV provide one).
- If that holds, every full-path Mirror run re-copies and re-versions every previously copied file.
- The full path is taken on the first run, after any run with errors, conflicts or omissions (any symlink or junction, e.g. the Windows legacy junctions in Documents) per orchestration.rs:88-95, and always for remote sources without a change feed (incremental.rs:116-138).
- [critic] Combined with finding #1(3): a home or profile Mirror backup errors on every daemon run because of heartbeat drift. It would then never bootstrap the incremental index and would always take the full path, the possibly re-copying one described in the previous item.
- [critic] Out of scope: failed or canceled two-way runs keep the old baseline (orchestration.rs:301-320). Because copies do not preserve mtimes, every file copied in such a run likely reappears next run as a both-changed FileLevel conflict on hash-less backends. This is presumably the 'already-reported all-or-nothing baseline consequences' that #10 and #12 refer to; I did not check it against the other reviewer's report.
- [critic] Out of scope: mark_run (results.rs:115-121) is also an unlocked read-modify-write of the job .conf. It can overwrite a GUI job edit saved concurrently.
- [critic] Not verified from code (Android platform behavior): whether revoking 'All files access' immediately kills the app process. It does not change verdict #0: the next periodic, persistent or manual run proceeds without access either way.
