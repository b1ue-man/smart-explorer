# Review-Befunde: Fern-Analyse über Share

Stand: 2026-10-02. Quelle: Review-Workflow (Finder je Dimension, unabhängige Gegenprüfung, Vollständigkeits-Kritiker). Fehlende Dimensionen: keine.

| ID | Schwere | Urteil | Dimension | Titel |
|---|---|---|---|---|
| A01 | high | confirmed | client-routing | Share duplicate search never runs on the exporting host: the client downloads and MD5-hashes every file of the share |
| A02 | high | confirmed | client-routing | Android storage analysis of Share (and SSH-agent) locations walks the tree directory by directory over the network instead of using the host worker |
| A03 | high | confirmed | client-routing | Remote duplicate search aborts completely at the first symlink/junction, unreadable subfolder or unusual file name |
| A04 | medium | confirmed | client-routing | Remote duplicate search groups only the 200 largest candidates; Android tells the user folders were not searched |
| A05 | medium | confirmed | client-routing | Canceling a remote duplicate search is not forwarded until the next hashed file arrives |
| A06 | low | confirmed | client-routing | Android hides host notes and protected areas of remote analyses; the listing path reports Android/data as read errors |
| A07 | medium | confirmed | client-routing | Remote listing walker ignores its retention budget; Android keeps up to four full result trees |
| A08 | medium | confirmed | client-routing | Host-side analysis adds a link check and canonicalize per directory, so it is slower than the host's own local run |
| A09 | low | partially_confirmed | client-routing | Mounted Share drives are analysed by the local scanner through the mount, never by the exporting host |
| A10 | medium | confirmed | client-routing | Client rejects whole host results when host counters and tree disagree (failed remote child, lagging legacy progress, recovered panic) |
| A11 | low | confirmed | client-routing | Android analysis and duplicate search use the pooled Share connection without a liveness check |
| A12 | low | confirmed | client-routing | Hosts serve older clients' StorageSnapshot with the serial walker that aborts on the first unreadable folder |
| A13 | low | confirmed | client-routing | Daemon hash-walk budget fails one entry before the client's budget, turning a graceful limit into a failed search |
| A14 | low | confirmed | client-routing | Analyses of an Android host via Share lack Android's platform figures (apps, uncaptured used space) |
| A15 | high | confirmed | host-execution | Android client never asks the host to analyse: remote analysis is a per-directory ListDir walk instead of the host's local StorageAnalysis |
| A16 | high | confirmed | host-execution | Share duplicate search downloads and MD5-hashes every remote file in the client daemon and fails on the first link or unreadable folder |
| A17 | medium | confirmed | host-execution | Host-side Share analysis adds lstat plus a full canonicalize for every directory that the same host's local analysis does not do |
| A18 | low | partially_confirmed | host-execution | Finished tree is transferred uncompressed and the desktop daemon decodes and re-encodes it before the GUI gets it |
| A19 | medium | partially_confirmed | host-execution | Analysing a host's root '/' gives every export its own 6M-node budget; the merged tree can exceed the transfer limits and fails after the full scan |
| A20 | medium | partially_confirmed | host-execution | A lost connection discards the host's analysis; no host-side retention, re-attach or client retry |
| A21 | medium | confirmed | host-execution | Host admission for analysis work has no per-peer fairness; legacy snapshot/walk streams hold the shared 32-slot control pool for their whole walk |
| A22 | medium | confirmed | host-execution | Remote analysis carries none of the volume and Android platform figures the host's own analysis shows |
| A23 | medium | confirmed | host-execution | Receiver keeps a host-sized tree without regard to its own memory; Android stores up to four results |
| A24 | medium | confirmed | host-execution | Share ListDir replies are one JSON frame; folders above ~95k entries cannot be listed and one bad entry fails the whole listing |
| A25 | low | confirmed | host-execution | Receiver rejects the whole result when the host's byte counter and tree size differ |
| A26 | low | confirmed | host-execution | Host path remapping treats physical paths that start with the export name as already visible; Windows display-form paths stay unmapped |
| A27 | low | confirmed | host-execution | Remote-requested analysis on a Windows host cannot use the consented elevated read access a local analysis uses |
| A28 | low | confirmed | host-execution | Legacy StorageSnapshot/WalkTree host path aborts on the first unreadable folder and at 1M nodes |
| A29 | low | partially_confirmed | host-execution | A host progress path longer than 32 KiB makes the client abort the whole analysis |
| A30 | high | confirmed | critic | Any Direct contact going offline/online, a new Room member or any export edit force-closes every Share connection, aborting running remote analyses on both ends |
| A31 | medium | confirmed | critic | Android host: a peer analysis that started while the app was visible gets no CPU hold once the app goes to background |
| A32 | medium | confirmed | critic | Desktop hosts never hold off system sleep while serving a peer's analysis |
| A33 | medium | confirmed | critic | Android client: remote analysis and duplicate search run without a CPU wake lock, so screen-off turns them into partial or failed results |
| A34 | low | confirmed | critic | Share duplicate and cleanup results cannot be acted on: both clients are display-only although the host has a trash |
| A35 | low | confirmed | critic | Opening a folder from a remote analysis result drops the Share location's endpoint prefix |
| A36 | low | confirmed | critic | Analysing a peer's root '/' counts and walks nested or duplicated exports twice |
| A37 | low | partially_confirmed | critic | Host-side analysis of an exported SSH connection always tells the user to update both devices |

## A01 Share duplicate search never runs on the exporting host: the client downloads and MD5-hashes every file of the share

- Schwere: high · Urteil: confirmed · Kategorie: performance · Plattformen: Android, Windows, Linux
- Stellen: native/src/mobile/os/shared/domains/analyze.rs:158-161; native/src/mobile/os/shared/pool.rs:157-173; native/src/daemon/os/shared/ipc.rs:81-86; native/src/agent/core/search.rs:89-110; native/src/agent_proto/core/features.rs:31-36; native/src/daemon/os/shared/backend_walk.rs:306-395

**Beschreibung.** The Share wire protocol has no duplicate/hash operation (FsRequest offers Capabilities, ListDir, Stat, WalkTree, StorageSnapshot, StorageAnalysis and file I/O only). Every client therefore runs the duplicate search itself. Android reclaim_task resolves a share location to the pooled AgentBackend (bridge to the embedded daemon) and calls scan_reclaim_backend; the desktop 'Find & Reclaim' does the same with CachingBackend(AgentBackend). AgentBackend::supports_walk_hashed() is true, so a WalkHashed frame goes to the client-side daemon, whose handle_walk_hashed_backend walks the raw PeerBackend serially (stat + list_dir per directory, DFS, one request at a time) and, because want_hash=true and the frame carries no size threshold, computes the MD5 of EVERY file through md5_backend -> PeerBackend::open_read, i.e. it streams the complete content of the share to the client (PeerBackend listings never carry content_md5). The host's own search (find_duplicates -> finder_compare) only reads candidates >= minSize that share a size, first/last 64 KiB first, the full content only on equal fingerprints, with a parallel pool; find_duplicates_in already takes a Share-style guard but is only ever called with None. Progress stays in phase Walking without a current folder and bytes only advance after a whole file has been downloaded, so the run looks stalled.

**Fehlerszenario.** Phone: 'Duplikate finden', minimum 1 MiB, location share://direct/<PC>/Daten (200 GB, 150 000 files). The phone pulls all 200 GB - including the files below 1 MiB and every file whose size is unique - over Wi-Fi or the relay, one file at a time, and hashes them on the phone; the search runs for hours, uses mobile data and battery, while the same search run on the PC reads only a few GB and finishes quickly. Desktop PC->PC and PC->phone (the PC daemon then drains the phone) behave the same; Direct peers and Room members alike.

**Richtung.** Add a host-side duplicate operation analogous to StorageAnalysis v2: a capability flag (e.g. duplicate_search_v1) in server_capabilities::describe, FsRequest::DuplicateSearch{path,min_bytes}, served by a storage_analysis_server-style worker (same bounded queue, CancelOnDrop, heartbeat, bounded result stream) that runs find_duplicates_in with ProtectedAreas::for_walk and the same confinement guard as scan_local_target (exported connections: their own walk_hashed/listing on the host); return groups with visible paths plus DuplicateSummary. Add an IPC request like AnalyzeShare and a Backend hook forwarded by CachingBackend/AgentBackend/UnavailableBackend, used first by analyze.rs reclaim_task and reclaim_core. Fallback for old hosts must never download everything: group by size >= minSize from listings, sample first/last bytes via open_read_at only for same-size candidates, read fully only on equal samples (or extend WalkHashed with a size threshold and skip unique sizes).

**Gegenprüfung.** Whole chain verified. FsRequest has no hash/duplicate op (wire.rs:302-410). Android: resolve_remote strips the cache (analyze.rs:159-160, sync_roots.rs:7, cache.rs:122-127) leaving the AgentBackend from open_share_backend (pool.rs:157-160, ipc_client.rs:58-75); desktop wraps it in CachingBackend, which forwards supports_walk_hashed/walk_hashed (cache.rs:472-483; share_drain.rs:45 cache_remote). AgentBackend::supports_walk_hashed() is true (agent/core/backend.rs:252-254) and agent_walk_hashed only bails out without link_aware_hash (search.rs:98-100), but the daemon announces sync-links-v1 (backend_server.rs:61-66 -> features.rs:31-36; parsed in transport.rs:281-283). The daemon serves the raw PeerBackend over IPC (ipc.rs:81-86, service.rs:116-131) and handle_walk_hashed_backend does stat+list_dir per directory with a LIFO stack and, because want_hash=true and Frame::WalkHashed carries no threshold (backend_server.rs:270-272), calls md5_backend -> open_read for every file (backend_walk.rs:355-359, 380-395); PeerBackend listings always have content_md5 None (share/core/backend.rs:471). The minimum size is only applied client-side after hashing (reclaim/backend.rs:377). The host-side finder compares only same-size candidates, sampled ends first (finder_compare.rs:51-80), and find_duplicates_in is called with guard None only (finder.rs:152-158; other callers are tests). The progress stage is never begun/entered on this path, so the status stays 'Walking' without a folder and bytes advance per finished file (stage.rs:39-42, 84-91; reclaim/backend.rs:137-151). Severity: the defect is deterministic for every Share duplicate search and transfers the whole share (metered data, relay load), but it causes no data loss or security exposure and is user-initiated and cancelable (see #4 for the delay); high fits better than critical, although critical is defensible if metered-data/relay cost is weighted heavily.

## A02 Android storage analysis of Share (and SSH-agent) locations walks the tree directory by directory over the network instead of using the host worker

- Schwere: high · Urteil: confirmed · Kategorie: performance · Plattformen: Android
- Stellen: native/src/mobile/os/shared/domains/analyze.rs:190-222; native/src/mobile/os/shared/domains/analyze.rs:156-161; native/src/analytics/os/shared/remote.rs:6-40; native/src/app/core/analytics_core.rs:414-427; native/src/daemon/os/shared/ipc_client.rs:54-75; native/src/daemon/os/shared/ipc_client.rs:367-380; native/src/agent/core/backend.rs:194-200; native/src/mobile/os/shared/pool.rs:157-173; native/src/analytics/os/shared/analytics_backend.rs:11-39; native/src/analytics/os/shared/analytics_backend.rs:64-117; native/src/analytics/os/shared/analytics_backend.rs:152-216; native/src/share/core/fs.rs:49-80; native/src/share/core/fs.rs:311-322; native/src/share/core/framing.rs:11; native/src/agent/core/backend.rs:18; native/src/vfs/os/shared/local.rs:80-92; native/src/vfs/core/delete.rs:318-331

**Beschreibung.** analytics::scan_remote is the selection boundary that runs the analysis on the exporting host (Backend::scan_storage -> StorageAnalysis v2, then walk_tree/snapshot, listing walk only last). The desktop GUI uses it (analytics_core.rs:425) and the Android pool's Share backend supports it: pool.rs opens shares via daemon::open_share_backend, which returns AgentBackend whose scan_storage forwards to UnavailableBackend::scan_storage -> ipc_analysis::scan -> IPC AnalyzeShare in the embedded daemon -> PeerBackend::scan_storage. But analysis_task in analyze.rs calls crate::analytics::scan_backend - the generic listing walker - for every non-local location, so scan_storage and walk_tree are never consulted. The listing walker issues one ListDir per directory (phone -> loopback IPC -> embedded daemon -> Iroh -> host), breadth-first with a barrier per level and at most parallelism() requests in flight; on the host every ListDir re-resolves the export (canonicalize of root and target, exists) and builds full metadata. The result also differs from the host's own run: no protected-area handling (Diagnostics::default), a ListDir reply is limited to one 16 MiB frame and the 20 s agent metadata timeout (estimate: ~100k entries), one non-UTF-8/unrepresentable name or a name containing a backslash fails the whole directory (LocalBackend::list_dir / validate_child_name), and the retention budget is not applied (separate finding). The same bug makes Android ignore the SSH agent's server-side walk_tree for SFTP connections. The Android status line (walk_status(dirs,current)) has no mapping for the v2 phases (Queued, Transferring, Verifying, Legacy) or the transfer counters that a fix needs.

**Fehlerszenario.** Phone connects to the PC via Direct Share (or as Room member) and analyses share://direct/<PC>/C (1.5 M files, 200 000 folders): 200 000 network round trips at 30-100 ms each plus host-side path resolution make the run take tens of minutes, while the PC's own analysis of C:\ - and a PC->PC Share analysis, which uses the host worker - finishes in about a minute. A folder holding ~150 000 files cannot be listed in one reply and is reported as unreadable, so the phone shows a partial result where the PC shows a complete one. Phone->phone is affected the same way.

**Richtung.** In analysis_task call crate::analytics::scan_remote(&**backend, root, &scan_progress) instead of scan_backend (keeps the existing fallbacks for old hosts and non-agent remotes, and gives SFTP the agent walk); obtain the backend with resolve_live; map ScanSnapshot.phase, transferred/transfer_total (ctx.progress with a total), directories_unreported and unchanged_ms into the Android task message/progress; surface host notes (see the notes finding). Add an Android acceptance case asserting that no per-directory ListDir reaches the host, like the desktop M10 fixture.

**Gegenprüfung.** analysis_task calls crate::analytics::scan_backend for every non-local location (analyze.rs:190-212); scan_backend never consults scan_storage or walk_tree (analytics_backend.rs:11-39). The desktop uses scan_remote (analytics_core.rs:414-427), which tries Backend::scan_storage first (remote.rs:11). The Android Share backend supports it: AgentBackend::scan_storage forwards to inner (agent/core/backend.rs:194-200) = UnavailableBackend::scan_storage -> ipc_analysis::scan (ipc_client.rs:377-380) -> IPC AnalyzeShare -> scan_remote on PeerBackend in the daemon (ipc.rs:76-79, ipc_analysis.rs:104-107) -> host StorageAnalysis v2 (share/core/backend.rs:227-233, peer_storage_analysis.rs:10-36). The listing walker runs level by level with a barrier (analytics_backend.rs:181-214), one ListDir per directory, with parallelism() threads: AgentBackend::parallelism -> UnavailableBackend default = rayon::current_num_threads() (agent/core/backend.rs:413-415, core.rs:243-245). Each host ListDir re-resolves the export via canonicalize(root) plus exists/canonicalize(target) (fs.rs:74, 311-344). A reply is one JSON control frame capped at 16 MiB (framing.rs:11, 63-70); with about 160 bytes of JSON per FsMeta (wire.rs:229-239) the ~100k-entry estimate is plausible, and the 20 s agent metadata timeout applies (agent/core/backend.rs:18, 128-132). There is no protected-area handling (Diagnostics::default, analytics_backend.rs:21). One unsafe name fails the whole directory (analytics_backend.rs:78; delete.rs:318-331), as does a non-UTF-8 or unrepresentable host name (local.rs:80-92). The SSH-agent remark holds: the agent binaries are bundled on all platforms (deploy.rs:15-27), and AgentBackend::walk_tree is never used by scan_backend. The status line only uses walk_status (analyze.rs:219). This is the direct cause of the reported 'phone -> PC analysis is slow'. Severity high is correct.

## A03 Remote duplicate search aborts completely at the first symlink/junction, unreadable subfolder or unusual file name

- Schwere: high · Urteil: confirmed · Kategorie: bug · Plattformen: Android, Windows, Linux
- Stellen: native/src/daemon/os/shared/backend_walk.rs:321-339; native/src/daemon/os/shared/backend_walk.rs:355-359; native/src/daemon/os/shared/backend_walk.rs:380-388; native/src/analytics/os/shared/reclaim/backend.rs:155-187

**Beschreibung.** handle_walk_hashed_backend (the daemon side of every Share duplicate search) propagates every problem with `?`: a symlink child returns Unsupported(HASH_WALK_LINK_BOUNDARY); a failing stat or list_dir of any subfolder ends the walk; a name rejected by validate_child_name (contains a backslash) ends it; a host listing that fails because of a non-UTF-8/unrepresentable name ends it. LocalBackend reports junctions and symlinks as is_symlink, so they do reach this walk. scan_backend_via_agent maps any such error (when not canceled or budget-stopped) to root_error; Android turns root_error into a failed task with kind 'not_found', the desktop into StorageRunState::Failed without a report. The host's local search skips links and records unreadable folders as bounded errors; bisync treats HASH_WALK_LINK_BOUNDARY as a referral and falls back, reclaim does not. The SSH agent's own hash walk aborts on links the same way.

**Fehlerszenario.** PC exports C:\Users\<u>\Documents, which in a typical Windows profile contains hidden legacy junctions ('My Music', 'My Pictures', 'My Videos' - assumption about the host profile), or exports D:\ with an access-denied 'System Volume Information', or a Linux folder containing one symlink or a file named 'a\b'. A duplicate search from the phone or another PC fails with '<root>: agent hash walk failed: SE_HASH_WALK_LINK_BOUNDARY_V1' or 'Zugriff verweigert' and shows no groups, while the same search on the host completes and only skips or reports those entries.

**Richtung.** In handle_walk_hashed_backend skip symlink children, report per-directory stat/listing failures and invalid names as per-entry errors and continue (a bounded error frame or HashEntry flag); in scan_backend_via_agent treat HASH_WALK_LINK_BOUNDARY like bisync (fallback) and never promote a non-root failure to root_error. The host-side duplicate operation (first finding) removes this path for Share entirely; keep the fix for SSH-agent remotes.

**Gegenprüfung.** Every failure in handle_walk_hashed_backend propagates with '?': the directory stat (backend_walk.rs:321, 397-405), list_dir (323), validate_child_name (324), a symlink child returning Unsupported(HASH_WALK_LINK_BOUNDARY) (336-339), and also, not named in the finding, md5_backend/open_read of any locked or unreadable file (358, 381). Locked files are common on Windows hosts (e.g. NTUSER.DAT or an open .pst), so this trigger is likely frequent. LocalBackend flags junctions/symlinks as is_symlink (local.rs:21, 90-91), and the host's ListDir forwards it unfiltered (fs.rs:74-79, 380-392). A non-UTF-8 or unrepresentable name fails the whole host listing (local.rs:82-87). scan_backend_via_agent turns any non-cancel/non-budget Err into root_error (reclaim/backend.rs:171-186). Android maps root_error to ApiError 'not_found' (analyze.rs:402-404), and the desktop shows Failed without a report (reclaim_core.rs:320-335). The local finder skips links and records failures as bounded errors (finder_walk.rs:114-122, 158-160). bisync treats HASH_WALK_LINK_BOUNDARY as a referral and falls back (snapshot_agent.rs:89-95), reclaim does not. The SSH agent's walk aborts on links the same way (agent_proto/os/shared/hash.rs:26-38). Severity high is correct. Combined with #0, the search may download for a long time and then discard everything.

## A04 Remote duplicate search groups only the 200 largest candidates; Android tells the user folders were not searched

- Schwere: medium · Urteil: confirmed · Kategorie: gap · Plattformen: Android
- Stellen: native/src/mobile/os/shared/domains/analyze.rs:363-367; native/src/analytics/os/shared/reclaim/types.rs:14-23; native/src/analytics/os/shared/reclaim/backend.rs:377-400; native/src/analytics/os/shared/reclaim/backend_duplicates.rs:11-25; native/src/analytics/os/shared/reclaim/finder.rs:1-6; native/src/analytics/os/shared/reclaim/finder.rs:107-142; android/app/src/main/java/app/smartexplorer/android/ui/analytics/DuplicatesScreen.kt:75-76; android/app/src/main/java/app/smartexplorer/android/ui/analytics/DuplicatesScreen.kt:148-159

**Beschreibung.** For remote locations the Android facade builds ReclaimOptions from Default (max_items 200). record_backend_file keeps only the 200 largest MD5 candidates (retain_best) and remote_duplicate_groups groups only those, whereas the Android local finder compares every candidate ('no 200 cap'). from_reclaim turns this into the limit text 'Nur die 200 größten von N Kandidaten verglichen', which DuplicatesScreen renders as 'Suche vorzeitig beendet ... Weitere Ordner wurden nicht durchsucht - das Ergebnis ist unvollständig' (wrong: every folder was walked); searchFacts labels the uncapped candidate count as 'verglichen', and the setup hint promises that all files >= the minimum size are compared.

**Fehlerszenario.** A shared folder holds 3 000 files >= 1 MiB: 250 large videos and 600 duplicated 3 MB photos. The remote search groups only the 200 biggest videos and reports no photo duplicates, while the host's own search lists all photo groups; the user is told that folders were skipped.

**Richtung.** Use the host-side search (first finding). For the fallback, group by (size, md5) under a memory/text budget like MAX_CANDIDATE_TEXT_BYTES instead of a 200-item cap (or pass max_items = usize::MAX on Android); keep walk limits and comparison limits apart in DuplicateSummary and in the Android notice; show `compared` as 'verglichen'.

**Gegenprüfung.** The Android remote path uses ReclaimOptions::default() except for the minimum size (analyze.rs:364-367), and max_items is 200 (types.rs:14-23). record_backend_file keeps only the 200 largest MD5 candidates via retain_best (reclaim/backend.rs:377-400, retention.rs:8-28), and remote_duplicate_groups groups only those (backend_duplicates.rs:11-39). The local finder has no such cap (finder.rs:1-6, 146-159). from_reclaim produces 'Nur die … größten von … Kandidaten verglichen' and, above 200 groups, '… von … Gruppen angezeigt' (finder.rs:112-126). DuplicatesScreen renders any limit as 'Suche vorzeitig beendet … Weitere Ordner wurden nicht durchsucht – das Ergebnis ist unvollständig.' (DuplicatesScreen.kt:156-158), which is wrong here because every folder was walked. searchFacts shows candidates (the uncapped duplicate_candidates) as 'verglichen' (DuplicatesScreen.kt:148-150; the Kotlin ReclaimSummary ignores the view's 'compared' field, AnalyzeApi.kt:144-154). The setup hint promises that all files at or above the minimum are compared (DuplicatesScreen.kt:75-76). The failure scenario (250 large videos crowd out the 3 MB photo duplicates) follows directly from the size-ordered retention. Severity medium is correct.

## A05 Canceling a remote duplicate search is not forwarded until the next hashed file arrives

- Schwere: medium · Urteil: confirmed · Kategorie: bug · Plattformen: Android, Windows, Linux
- Stellen: native/src/analytics/os/shared/reclaim/backend.rs:104-115; native/src/agent/core/search.rs:89-117; native/src/daemon/os/shared/backend_walk.rs:380-395; native/src/mobile/os/shared/domains/analyze.rs:121-145; native/src/app/core/reclaim_core.rs:111-128

**Beschreibung.** scan_backend_via_agent gives walk_hashed a private flag walk_cancel. progress.cancel (set by the Android task or the desktop panel) is copied into walk_cancel only inside `for hit in rx.iter()`, i.e. when the next HashEntry arrives. While the daemon downloads and hashes one large file no entry arrives, so neither agent_walk_hashed (polls walk_cancel every 200 ms and would send Frame::Cancel) nor the daemon (checks its cancel per chunk) learns about the cancel.

**Fehlerszenario.** The daemon is MD5-hashing a 4 GB video from the PC over a 5 MB/s relay and the user taps 'Abbrechen': the Android task keeps running and the download continues for about 13 minutes; on the desktop the panel shows the search as canceled at once while the background daemon keeps transferring.

**Richtung.** Give walk_hashed a flag that observes progress.cancel directly (e.g. pass &progress.cancel and set a separate stop flag for the budget, or loop on rx.recv_timeout and copy progress.cancel into walk_cancel on every timeout).

**Gegenprüfung.** walk_cancel is a private flag (reclaim/backend.rs:105-110). progress.cancel is copied into it only inside 'for hit in rx.iter()' when a hit arrives (111-115). agent_walk_hashed polls only that flag every 200 ms before sending Frame::Cancel (search.rs:110-115), and the daemon checks its own cancel per chunk in md5_backend (backend_walk.rs:384-387), which is set only by a client Cancel. While one large file is being hashed, no HashEntry arrives, so neither side learns of the cancel. On Android, run_watched keeps the task alive until the scan thread ends (analyze.rs:133-139); the UI cancel only flags the task (DuplicatesViewModel.kt:114-123). On the desktop, cancel_reclaim_worker sets the flag and drops the scan handle, so the panel shows Canceled at once (reclaim_core.rs:111-128) while the worker and daemon continue until the next hit. Severity medium is correct.

## A06 Android hides host notes and protected areas of remote analyses; the listing path reports Android/data as read errors

- Schwere: low · Urteil: confirmed · Kategorie: gap · Plattformen: Android
- Stellen: native/src/mobile/os/shared/domains/analyze.rs:318-339; android/app/src/main/java/app/smartexplorer/android/ui/analytics/AnalysisScreen.kt:183-202; native/src/share/os/shared/storage_analysis_host.rs:104-114; native/src/analytics/core/analysis_report.rs:73-84; native/src/analytics/os/shared/analytics_backend.rs:21; native/src/analytics/os/shared/analytics_outcome.rs:197-204; native/src/analytics/os/shared/remote.rs:30-32; native/src/app/core/analytics_core.rs:240-248

**Beschreibung.** A Share host converts protected omissions into a note and clears them (protected_as_note), and AnalysisReport::finish always returns protected: Vec::new(). analyze.issues puts notes only into `text` and derives protectedCount/protectedText from outcome.protected. The Android result page shows the 'Bericht' link only when count > 0 and the protected row only when protectedCount > 0 or protectedText is set, so a remote result's notes (protected areas, 'älterer Analysepfad', 'Detailansicht ... zusammengefasst') are invisible on Android unless read errors exist. With today's listing path the client has no ProtectedAreas at all (Diagnostics::default), so an Android host's Android/data and Android/obb either fail and count as unreadable paths (Partial) or list empty without any protected notice - unlike the host's own result. The desktop shows the notes.

**Fehlerszenario.** Phone A analyses phone B's shared internal storage: A shows '2 Pfade nicht lesbar' and a partial result where B's own analysis shows '2 Bereiche von Android geschützt' (assumption: EACCES vs. empty listing for Android/data depends on the Android version). Once routed through the host worker, A would show neither a protected notice nor the host's notes.

**Richtung.** Add an additive protected-omissions field to AnalysisReport (keep the note for older peers) and return it as protectedCount/protectedText; expose notes separately in analyze.issues and show a notes row on Android regardless of the issue count; route Share analyses through the host worker so protected areas are classified on the host.

**Gegenprüfung.** All statements hold. protected_as_note moves protected omissions into notes[0] and clears them (storage_analysis_host.rs:107-114), and AnalysisReport::finish always returns protected: Vec::new() (analysis_report.rs:80-84). analyze.issues puts notes only into 'text' and derives protectedCount/protectedText from outcome.protected (analyze.rs:324-338). AnalysisScreen shows the 'Bericht' row only when count > 0 (AnalysisScreen.kt:183-191) and the protected row only when protectedCount > 0 or protectedText is set (193-202), so notes of a remote result without read errors are never shown on Android. The listing path has no ProtectedAreas (analytics_backend.rs:21), so a failing Android/data ListDir becomes an issue (Partial, analytics_backend.rs:196-200), while an empty listing gives no notice; which one happens depends on the host's Android version, as the finding says. Severity: the measured tree and sizes are unaffected; only notices and status differ, and the listing-path part disappears once #1 is fixed. Low rather than medium.

## A07 Remote listing walker ignores its retention budget; Android keeps up to four full result trees

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Android, Windows, Linux
- Stellen: native/src/analytics/os/shared/analytics_backend.rs:10; native/src/analytics/os/shared/analytics_backend.rs:101-106; native/src/analytics/os/shared/analytics_backend.rs:152-216; native/src/analytics/os/shared/analytics_backend.rs:218-251; native/src/analytics/os/shared/analytics_budget.rs:11-18; native/src/analytics/os/shared/analytics_budget.rs:75-104; native/src/mobile/os/shared/domains/analyze.rs:27-28; native/src/mobile/os/shared/domains/analyze.rs:59-77; native/src/analytics/core/tree_transfer.rs:7-8

**Beschreibung.** scan_backend is documented as 'bounded retained state', but collect_children discards the Retention returned by budget.claim (`let _ =`) and build_from_listings creates a node for every directory and every non-folded file; only files beyond 4096 per directory are folded. scan_parallel keeps a HashMap of all directory paths -> ChildMeta lists for the whole tree and builds the SizeNode tree while that map is still alive. The note 'Detailansicht ab ... zusammengefasst' is emitted although nothing is aggregated. The local scanner switches to aggregation after 6 M nodes / 768 MiB names. On Android, analyze.rs keeps up to four finished results (MAX_RESULTS) and the Kotlin side has no call to release one; a v2 result may carry up to 12 M nodes.

**Fehlerszenario.** The phone analyses a PC share or an SFTP server with ~5 M entries through the listing walk: the map plus the tree need several hundred MB up to more than 1 GB of native memory (estimate) and Android's low-memory killer ends the app; repeated 'Neu analysieren' of a big remote tree keeps older trees alive until four slots are filled.

**Richtung.** Honour Retention::Aggregate in the listing walker (fold directories beyond the budget into the parent's aggregate as scan_entries does) and assemble the tree bottom-up while dropping consumed listings; on Android release the previous analysis when a new one starts (or add analyze.release) and consider a client-side node budget for received trees.

**Gegenprüfung.** collect_children discards the Retention ('let _ = budget.claim(...)', analytics_backend.rs:101-106), so the only reduction is the per-directory fold above 4096 files (121-150; analytics_budget.rs:18). scan_parallel keeps a HashMap of full directory path -> Vec<ChildMeta> for the whole tree (179, 211) and builds the complete SizeNode tree from it while the map is still borrowed (215, 218-251). The claim still emits the 'Detailansicht ab … zusammengefasst' note when a limit is crossed (analytics_budget.rs:97-102), although nothing is aggregated on this path. The local scanner honours Retention (analytics.rs:262-264, 296-315, 340-346) with 6M nodes / 768 MiB (analytics_budget.rs:11-12). Android keeps up to four results (analyze.rs:28, 59-77), and neither the facade nor AnalyzeApi.kt has a release call. The transfer accepts up to 12,000,002 nodes (tree_transfer.rs:7). The memory estimate is unmeasured but plausible (about 70 bytes per ChildMeta plus about 90 bytes per SizeNode, both alive at the peak). The desktop is affected too via scan_remote's fallback for non-agent remotes (remote.rs:18-19). Severity medium is correct.

## A08 Host-side analysis adds a link check and canonicalize per directory, so it is slower than the host's own local run

- Schwere: medium · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/os/shared/storage_analysis_host.rs:80-98; native/src/analytics/os/shared/analytics.rs:174-189; native/src/share/core/fs.rs:311-322

**Beschreibung.** scan_local_target passes a guard that, for every directory, calls symlink_metadata, metadata_is_link_like and std::fs::canonicalize before the same enumeration the local scan performs. The code comment states 'Same worker ... as the GUI's local drive scan. Only Share confinement is added', but that confinement costs at least two additional metadata/handle operations per directory (Windows canonicalize opens a handle and calls GetFinalPathNameByHandleW; on Linux/Android realpath touches every path component). For UNC/SMB exports each of these is a server round trip.

**Fehlerszenario.** A PC exports \\nas\media or a large NTFS tree and a phone analyses it via Share: for 300 000 folders the host performs about 600 000 extra open/query operations - each one a network round trip on SMB - so the Share analysis takes noticeably longer than the PC's own analysis of the same folder (magnitude not measured; assumption).

**Richtung.** Enforce confinement inside the enumerator without re-resolving paths: handle-relative traversal (openat with O_NOFOLLOW|O_DIRECTORY on Linux/Android; parent-relative open with reparse-point checks from the FILE_ID_EXTD_DIR_INFO batch on Windows), or canonicalize only entries whose enumeration reported a reparse/link attribute; record host-vs-local timing in the acceptance suite.

**Gegenprüfung.** scan_local_target installs a guard that does symlink_metadata + metadata_is_link_like + std::fs::canonicalize for every directory (storage_analysis_host.rs:80-95). scan_dir runs it before each enumeration (analytics.rs:182-187), whereas the host's own GUI scan passes no guard (analytics.rs:58-60; analytics_core.rs:419-421). On Windows, symlink_metadata and canonicalize each open and close a handle (two extra opens per directory besides the enumeration, each passing through filter drivers such as Defender). For UNC/SMB exports, which also resolve to LocalBackend and take this path (fs.rs:169-182, 185-200), each is a network round trip. On Linux/Android this is lstat + realpath, which is cheap with a warm dentry cache but goes through FUSE on /storage/emulated. The comment 'Only Share confinement is added' (96-97) understates this cost. The magnitude is unmeasured, as the finding says. It is secondary to #1 for the phone -> PC symptom, but plausibly significant for directory-heavy trees on Windows and SMB. A fix must keep the TOCTOU confinement, e.g. via handle-relative, no-follow traversal. Medium is acceptable.

## A09 Mounted Share drives are analysed by the local scanner through the mount, never by the exporting host

- Schwere: low · Urteil: partially_confirmed · Kategorie: gap · Plattformen: Windows, Linux
- Stellen: native/src/app/os/windows/platform.rs:41-48; native/src/app/core/analytics_ui.rs:98-112; native/src/app/core/picker_impl.rs:350-378; native/src/app/core/analytics_core.rs:414-422

**Beschreibung.** The analysis panel offers one 'Scannen:' button per logical drive (GetLogicalDrives, unfiltered) and defaults to the drive root of the current folder; both start StorageScanSource::Local and crate::analytics::scan. A Share mounted as a drive letter is therefore walked through the mount host with per-directory Share requests and never reaches StorageAnalysis on the exporting host. Nothing in the analytics path consults the mount registry.

**Fehlerszenario.** The user mounts a PC's Share as Z: on another PC and clicks 'Z:\' in Speicher-Analyse (or opens the analysis while browsing Z:): the scan is as slow as a per-directory network walk, while '📡 Remote-Ordner' on the same Share would use the fast host worker. Assumption: Share mounts appear as drive letters (the mount types use drive letters); the same pattern applies to Linux FUSE mount points picked via 'Ordner…'.

**Richtung.** Before a local scan, check the active Share mounts (daemon::list_mounts) and, for a path inside one, start StorageScanSource::Remote with the share backend and the mapped backend path (or offer that), so the host worker is used.

**Gegenprüfung.** Confirmed: list_drives is the unfiltered GetLogicalDrives (platform.rs:41-48). Each drive button and every non-remote picker result starts StorageScanSource::local -> analytics::scan (analytics_ui.rs:110-111; picker_impl.rs:352-364, 366-377; analytics_core.rs:419-421), and nothing consults the mount registry. Share mounts use drive letters (mount/core/types.rs:215-218, 238-241), so 'Z:\' is walked through the Dokan mount with per-directory Share requests. Overstated: analytics_default_root (analytics_core.rs:106-118) only seeds the Reclaim folder picker (reclaim_ui.rs:389-395), and the panel never auto-scans the current drive, so 'opens the analysis while browsing Z:' does not start a scan by itself. Severity: the user picks this path explicitly, the results are correct (only slower), and the app's own Share browsing offers the fast 'Remote-Ordner' route (analytics_ui.rs:114-130). Low rather than medium.

## A10 Client rejects whole host results when host counters and tree disagree (failed remote child, lagging legacy progress, recovered panic)

- Schwere: medium · Urteil: confirmed · Kategorie: bug · Plattformen: Android, Windows, Linux
- Stellen: native/src/analytics/core/analysis_report.rs:61-72; native/src/share/os/shared/storage_analysis_host.rs:155-189; native/src/analytics/os/shared/remote.rs:21-38; native/src/agent/core/walk.rs:17-30; native/src/agent_proto/core/server.rs:91-111; native/src/agent_proto/os/shared/fs.rs:246-296; native/src/analytics/os/shared/analytics.rs:190-199; native/src/analytics/os/shared/analytics.rs:269-276

**Beschreibung.** AnalysisReport::finish fails the entire result when tree.size != progress.bytes. The host fills progress.bytes from live counters that are never reconciled with the returned tree: scan_remote's legacy branch stores before+last reported bytes and then builds the tree without a final counter update; in scan_container a remote child that fails after partial progress contributes its counted bytes but only an empty node; a panic caught in scan_dir after 128-file batches were already counted returns an empty node. In each case the client discards a result the host considered complete or partial.

**Fehlerszenario.** Analysing the device root '/' of a host that exports 'Verbindungen': one exported SFTP connection drops mid-walk, the host returns Partial with that connection empty but with its partial bytes still in progress.bytes, and the client shows only 'Analyse-Zähler und Baum stimmen nicht überein' instead of the partial result.

**Richtung.** On the host, derive the report's files/bytes from the final tree (or subtract a failed child's partial counters) before send_outcome; on the client, treat a counter/tree difference as a note once shape and SHA-256 are verified instead of failing the analysis.

**Gegenprüfung.** finish rejects the whole result when tree.size != report.progress.bytes (analysis_report.rs:63-72). report.progress is the raw host snapshot taken in send_outcome (analysis_transfer.rs:21-25), never reconciled with the tree. Container: children share the counters (progress.rs:80-87); a failed child contributes an empty node (storage_analysis_host.rs:177-182) but keeps whatever its legacy walk stored (remote.rs:22-26 stores before+bytes absolutely). That walk fails after partial progress whenever the SSH agent meets any unreadable directory, because walk_dir_counted aborts with '?' (agent_proto/os/shared/fs.rs:246, 286-296), so the container scenario is realistic for agent-backed exported SFTP connections. A caught panic leaves counted 128-file batches with an empty node (analytics.rs:190-199, 269-276), even for a single local target. Lagging legacy progress is also real: walk_tree_impl returns the Tree without a final on_progress (walk.rs:30), and the agent's emitter loop 'while !done { sleep; emit }' (server.rs:91-103) can emit nothing when the walk finishes before the emitter thread first checks 'done' (tiny trees). Nuance: for a plain listing-walker connection (SFTP without agent), a mid-walk drop gives a consistent Partial, because counts are added only after a complete listing (analytics_backend.rs:76-116), so the cited 'SFTP connection drops' example needs an agent-backed connection. Severity medium is correct.

## A11 Android analysis and duplicate search use the pooled Share connection without a liveness check

- Schwere: low · Urteil: confirmed · Kategorie: reliability · Plattformen: Android
- Stellen: native/src/mobile/os/shared/domains/analyze.rs:156-161; native/src/mobile/os/shared/runtime.rs:174-186; native/src/mobile/os/shared/runtime.rs:221-231; native/src/daemon/os/shared/ipc_client.rs:54-75; native/src/agent/core/transport.rs:290-350

**Beschreibung.** resolve_remote uses Runtime::resolve, which returns the cached pool entry; unlike with_read and resolve_live it never evicts and reopens a lost connection, and the Share AgentBackend is built with AgentPool::single (no reconnect).

**Fehlerszenario.** After the IPC stream between the pooled AgentBackend and the embedded daemon closed (worker restart, process trimming), every analysis or duplicate search of that Share location fails at the root ('stream closed' / 'Background-Worker-Verbindung geschlossen') until browsing the location happens to evict the pool entry.

**Richtung.** Resolve with rt.resolve_live(&loc) (or evict and retry once on a connection-loss error) in analysis_task and reclaim_task.

**Gegenprüfung.** resolve_remote uses Runtime::resolve, which returns the pooled entry without validation (analyze.rs:159; runtime.rs:176-186; pool.rs:58-62). resolve_live, documented as 'for a write or a task', revalidates with stat_fresh and evicts on connection loss (runtime.rs:206-231). Every other task uses it (scan.rs:137, transfer.rs:154, delete.rs:46, import.rs:88), so analyze/reclaim are the exception. The Share AgentBackend is built with from_streams -> AgentPool::single without reconnect (ipc_client.rs:58; agent/core/backend.rs:29-35, 101-106). Its heartbeat closes the mux after a failed ping (transport.rs:322-349); a frozen or backgrounded app or a restarted embedded worker (embedded.rs:52-73) are plausible triggers. Afterwards analyze/reclaim fail at the root until a with_read call evicts the entry (runtime.rs:190-204). How often the IPC stream dies in practice cannot be decided from the code. Low is correct.

## A12 Hosts serve older clients' StorageSnapshot with the serial walker that aborts on the first unreadable folder

- Schwere: low · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/storage_snapshot.rs:89-130; native/src/share/core/walk.rs:188-209; native/src/share/core/peer_storage_snapshot.rs:25-41

**Beschreibung.** Clients without v2 (older app versions) request StorageSnapshot v1; the current host still builds it with ServerWalker: serial stat + list_dir per directory through Share resolution, and any error (`?`) aborts the whole snapshot. The fast local worker that v2 uses is not reused, so mixed-version pairs keep the slow behaviour.

**Fehlerszenario.** A phone with an older app version analyses a current PC share that contains one access-denied folder: the analysis fails completely; without errors it is still far slower than the PC's own analysis.

**Richtung.** Serve v1 snapshots from scan_with_guard's outcome (convert SizeNode to WireNode within the v1 limits, aggregates as file nodes) so older clients also get the local worker and partial results.

**Gegenprüfung.** build_snapshot uses ServerWalker, which is serial: stat + list_dir per directory through FsAccess (storage_snapshot.rs:94-98; walk.rs:188-209). Every stat/list_dir error propagates with '?' and aborts the snapshot (walk.rs:196, 206; storage_snapshot.rs:98). A current client requests v1 only when the host lacks v2 (peer_storage_analysis.rs:12-21 -> remote.rs:18-27 -> peer_storage_snapshot.rs:25-41), so only mixed-version pairs (older clients against current hosts) hit this. Low is correct.

## A13 Daemon hash-walk budget fails one entry before the client's budget, turning a graceful limit into a failed search

- Schwere: low · Urteil: confirmed · Kategorie: bug · Plattformen: Android, Windows, Linux
- Stellen: native/src/daemon/os/shared/backend_budget.rs:3-28; native/src/daemon/os/shared/backend_walk.rs:314-335; native/src/analytics/os/shared/reclaim/backend.rs:132-136; native/src/analytics/os/shared/reclaim/backend.rs:155-180; native/src/analytics/os/shared/reclaim/budget.rs:5

**Beschreibung.** Both budgets allow 1 000 000 entries, but the daemon also counts the root, so WalkBudget::record fails on the 1 000 000th child while the client has seen 999 999 hits and its ReclaimBudget is not stopped. The server error arrives as Ok(Err) without budget.stopped() and becomes root_error, whereas the local search stops gracefully with 'Suche nach 1.000.000 Einträgen angehalten'.

**Fehlerszenario.** Duplicate search on a share tree with at least 1 000 000 entries (short paths) ends as failed ('backend walk exceeds its bounded collection budget') instead of a partial result with the limit notice.

**Richtung.** Make the server budget strictly larger than any client budget or map the server's budget error to scan_limit instead of root_error (moot for Share once the host-side search exists).

**Gegenprüfung.** The daemon records the root and then every child against MAX_BACKEND_WALK_NODES = 1,000,000 (backend_walk.rs:315, 335; backend_budget.rs:3, 20-24), so child #1,000,000 fails (nodes = 1,000,001). Exactly one HashEntry is emitted per successfully recorded child, so the client has claimed 999,999 entries against MAX_RECLAIM_ENTRIES = 1,000,000 (budget.rs:5; reclaim/backend.rs:132-136) and is not stopped. The Frame::Err arrives as Ok(Err) without budget.stopped() and becomes root_error (reclaim/backend.rs:171-175, 183-186) instead of the graceful limit text. The text budget differs in the other direction (the client counts path+name, the daemon only path), so with long paths the client stops first; hence the 'short paths' condition is correct. Low is correct.

## A14 Analyses of an Android host via Share lack Android's platform figures (apps, uncaptured used space)

- Schwere: low · Urteil: confirmed · Kategorie: gap · Plattformen: Android, Windows, Linux
- Stellen: native/src/mobile/os/shared/domains/analyze.rs:166-173; native/src/analytics/core/storage_view.rs:1-5; native/src/share/os/shared/storage_analysis_host.rs:54-102

**Beschreibung.** Android's local analysis adds rows from Kotlin-supplied figures ('≈ Apps', '≈ Nicht einzeln erfasst', other apps' data). For remote roots analyze.rs uses VolumeRoot::default()/PlatformTotals::default(), and the Share report has no field for platform figures, so a PC or another phone sees only the measured tree. This is a documented design choice (approximate rows exist only in the Android view) but contradicts 'same result in every direction'.

**Fehlerszenario.** A PC analyses the phone's shared internal storage (128 GB used, 40 GB walkable): the PC shows about 40 GB, the phone's own analysis of the same root shows 128 GB with app and system rows.

**Richtung.** Let an Android host attach its last known platform figures (volume used, other apps' bytes, optionally the app list) as an additive report field; Android clients apply Approximations::compute, the desktop shows them as notes.

**Gegenprüfung.** Remote roots get VolumeRoot::default()/PlatformTotals::default() (analyze.rs:168-173). Platform figures come only from the Kotlin 'platform' argument for local roots (analyze_platform.rs:12-58). The Share report has no field for them (analysis_report.rs:7-17), and storage_view.rs:1-5 documents that the approximate rows exist only in the Android view. It is a deliberate design choice, but it contradicts the requirement 'same result in every direction'. Low is correct.

## A15 Android client never asks the host to analyse: remote analysis is a per-directory ListDir walk instead of the host's local StorageAnalysis

- Schwere: high · Urteil: confirmed · Kategorie: performance · Plattformen: Android (client), Windows (host), Linux (host), Android (host)
- Stellen: native/src/mobile/os/shared/domains/analyze.rs:183-212; native/src/mobile/os/shared/domains/analyze.rs:158-161; native/src/analytics/os/shared/remote.rs:6-20; native/src/daemon/os/shared/ipc_client.rs:57; native/src/daemon/os/shared/ipc_client.rs:371-380; native/src/analytics/os/shared/analytics_backend.rs:11-39; native/src/analytics/os/shared/analytics_backend.rs:64-117; native/src/analytics/os/shared/analytics_backend.rs:152-216; native/src/vfs/core/core.rs:243-245; native/src/agent/core/backend.rs:18; native/src/agent/core/backend.rs:128-132; native/src/share/core/server_fs.rs:89-94; native/src/share/core/fs.rs:49-80; native/src/share/core/fs.rs:311-344; native/src/vfs/os/shared/local.rs:75-103

**Beschreibung.** For every non-local location the Android facade's analysis_task calls crate::analytics::scan_backend(&**backend, root, ..) directly (analyze.rs:207-211). It never goes through crate::analytics::scan_remote, the only entry that asks Backend::scan_storage (remote.rs:11). For Share locations that call would reach the AgentBackend's inner UnavailableBackend::scan_storage -> ipc_analysis::scan -> embedded daemon AnalyzeShare -> PeerBackend::scan_storage -> host StorageAnalysis (ipc_client.rs:376-380), i.e. the host's own local walker. Instead the phone walks the host remotely: one ListDir per directory (analytics_backend.rs:76), each hop phone facade -> loopback agent IPC -> embedded daemon -> QUIC -> host fs::list_dir, which re-resolves the export with two canonicalize calls and a stat per request (fs.rs:311-344) and replies with JSON FsMeta (~151 B + name per entry). Concurrency is the phone's rayon::current_num_threads() because UnavailableBackend has no parallelism override (vfs core.rs:243-245), and the walk is a level-synchronous BFS where each depth level waits for its slowest directory (analytics_backend.rs:181-214). Results also differ from the host's own run: (a) Diagnostics::default() has no protected areas (analytics_backend.rs:21), so on phone hosts every Android/data/<pkg> listing failure becomes an issue and the result is Partial instead of Complete with a protected note; (b) host LocalBackend::list_dir fails the whole directory on one failing entry, an unrepresentable or non-UTF-8 name (local.rs:79-102), and validate_child_name/duplicate-name checks fail whole directories (analytics_backend.rs:78-87), while the host walker only skips or records the entry; (c) is_pseudo_dir is evaluated on the virtual share path (analytics_backend.rs:94, 206), so a Linux host exporting / gets /proc (kcore), /sys, /dev walked and counted; (d) the retention budget result is discarded (let _ = budget.claim, analytics_backend.rs:101-106): all listings stay in a HashMap keyed by full path and are copied again into the tree (178-215), so phone memory is unbounded, and the budget still emits the note 'Detailansicht ab ... zusammengefasst' although nothing is aggregated; (e) each ListDir has a 20 s end-to-end limit on the agent hop (agent backend.rs:18, 128-132) and directories above ~95-100k entries exceed the reply frame (separate finding), so large folders drop out as 0 B. The 2026-10-01 lesung (section 2.7) recorded the direct scan_backend call; git history shows it unchanged since the facade was introduced (fc8fbc0).

**Fehlerszenario.** Phone connected by Direct or Room to a PC sharing a user folder with ~250k directories / 1.5M files. The PC's own analysis needs about a minute; from the phone the same analysis issues ~250k ListDir round trips with at most ~8 in flight and a barrier per depth level: estimated several minutes on LAN and 30+ minutes over a relay path, plus hundreds of MB of JSON listings, and the phone holds the whole tree twice in memory. Phone-to-phone analysis of /storage/emulated/0 reports 100+ 'Pfade nicht lesbar' for Android/data packages (Partial) while the host phone's own analysis is Complete with a protected-area note. (Durations are estimates, nothing was run.)

**Richtung.** Route non-local locations in analysis_task through crate::analytics::scan_remote(&**backend, root, &progress) so Share targets use the embedded daemon's AnalyzeShare and the host's StorageAnalysis (other backends keep scan_remote's agent/walk_tree/scan_backend fallbacks), and forward phase, current folder and transfer progress to ctx.progress/message. Independently harden scan_backend for genuinely remote backends: honour the Retention returned by claim, bound depth, replace the level-synchronous BFS with work-stealing DFS, apply pseudo-dir filtering only to local physical paths.

**Gegenprüfung.** The Android facade never reaches the host-side analysis. For non-local locations analysis_task calls `crate::analytics::scan_backend(&**backend, root, ..)` (analyze.rs:207-211). scan_remote has only three callers (app/core/analytics_core.rs:425, daemon ipc_analysis.rs:106, storage_analysis_host.rs:44), and none is on the Android path.

The resolved backend is the daemon AgentBackend: resolve_remote (analyze.rs:158-161) -> pool.rs:157-160 open_share_backend -> ipc_client.rs:57-75 AgentBackend::from_streams with inner UnavailableBackend. The pool wraps it in CachingBackend (pool.rs:169-173), and sync_roots.rs:6-8 unwraps it again. Its scan_storage forwards to the inner backend (agent backend.rs:194-200). That inner backend would go UnavailableBackend::scan_storage -> ipc_analysis::scan (ipc_client.rs:377-380) -> AnalyzeShare (ipc.rs:76-80) -> PeerBackend::scan_storage (share backend.rs:227-233) -> StorageAnalysis. That path exists but is bypassed.

scan_backend itself:
- issues one list_dir per directory (analytics_backend.rs:76);
- is a level-synchronous BFS (181-214);
- gets its width from AgentBackend::parallelism -> inner.parallelism() (agent backend.rs:413-415), which is the default rayon::current_num_threads() (vfs core.rs:243-245). No build_global exists in the crate.

Each request costs the host fs::list_dir -> resolve -> secure_local_target: canonicalize(root), exists(), canonicalize(target) (fs.rs:49-80, 311-344).

Sub-claims:
- (a) Diagnostics::default() has no areas (analytics_backend.rs:21; analytics_outcome.rs:153-164), so failures become record_io issues. The host walk passes ProtectedAreas (analytics.rs:86) and turns them into protected omissions (analytics_outcome.rs:199-204). apptrash/mod.rs:46-50 documents that Android lists the package folders but refuses to open them.
- (b) local.rs:77-102 collects into a Result, so an unreachable or non-UTF-8 name fails the directory. validate_child_name also rejects backslash names, which are legal on Linux (vfs/core/delete.rs:318-331). The host walker skips or lossy-converts instead (analytics.rs:225-239).
- (c) is_pseudo_dir matches only absolute /proc,/sys,/dev,/run (agent_proto/os/shared/fs.rs:61-68), but the walk sees virtual paths of the form /<export>/proc.
- (d) `let _ = budget.claim` (analytics_backend.rs:101-106) keeps every listing. Listings and cloned names coexist in build_from_listings (218-251). The claim still emits the aggregation note (analytics_budget.rs:97-101).
- (e) There is a 20 s metadata timeout with no retry on the reconnect-less agent connection (agent backend.rs:18,128-132; transport.rs:87-104). A failed listing drops the directory as 0 B (analytics_backend.rs:196-201).

Introduced in fc8fbc0 and recorded in lesung §2.7. The durations are estimates. High severity fits the user's report that phone-to-PC analysis is slow and behaves differently.

## A16 Share duplicate search downloads and MD5-hashes every remote file in the client daemon and fails on the first link or unreadable folder

- Schwere: high · Urteil: confirmed · Kategorie: bug · Plattformen: Windows, Linux, Android
- Stellen: native/src/analytics/os/shared/reclaim/backend.rs:45-61; native/src/analytics/os/shared/reclaim/backend.rs:96-187; native/src/daemon/os/shared/backend_walk.rs:306-406; native/src/daemon/os/shared/backend_budget.rs:3-5; native/src/daemon/os/shared/backend_server.rs:61-66; native/src/agent/core/transport.rs:282-283; native/src/agent/core/search.rs:89-108; native/src/share/core/backend.rs:459-472; native/src/analytics/os/shared/reclaim/types.rs:19; native/src/mobile/os/shared/domains/analyze.rs:383-386; native/src/mobile/os/shared/domains/analyze.rs:402-403; native/src/app/core/reclaim_core.rs:47-50; native/src/app/core/reclaim_core.rs:320-334

**Beschreibung.** The Share protocol has no host-side duplicate or hash operation: FsRequest has none (wire.rs:302-410), FsMeta has no hash field (wire.rs:228-239) and PeerBackend sets content_md5: None (backend.rs:459-472). Remote duplicate searches (desktop reclaim_core.rs:47-50 and Android reclaim_task analyze.rs:383-386) call scan_reclaim_backend, which sees supports_walk_hashed()==true on the daemon AgentBackend (the daemon announces the link-aware hash label for every backend, features.rs:33-35; search.rs:98) and requests WalkHashed{want_hash:true} (reclaim/backend.rs:110). The daemon serves it with a generic emulation over the PeerBackend (handle_walk_hashed_backend): a serial depth-first walk with a stat and a list_dir round trip per directory (backend_walk.rs:321-323) and, for every file regardless of the duplicate minimum size (never transmitted), md5_backend opens the file over Share and reads it to the end (355-359, 380-395). Any link-like entry aborts the walk with HASH_WALK_LINK_BOUNDARY (336-339), any unreadable directory or file aborts it via ? (323, 358), and trees beyond 1M nodes / 128 MiB of path text abort (backend_budget.rs:3-5). scan_backend_via_agent turns each abort into root_error (reclaim/backend.rs:171-175, 183-187): Android fails the task (analyze.rs:402-403), the desktop marks the run Failed and drops the report (reclaim_core.rs:325-334). When it does complete, only the 200 largest candidates are grouped (types.rs:19, backend.rs:377-401, backend_duplicates.rs:16-25) by MD5, whereas the phone's local finder compares every file >= minSize with head/tail sampling and reads only same-size candidates (finder.rs). A host-capable finder with a confinement Guard parameter exists (finder.rs:28, 161-167) but has no Share caller. The same daemon handler also serves bisync's walk_hashed_via_agent, so HashMode::Full syncs against Share targets download every file too (sync dimension).

**Fehlerszenario.** (1) Phone runs 'Duplikate' on a PC's shared Documents folder that contains the legacy Windows junctions (listed with is_symlink) -> the walk aborts at the first junction -> task fails with '...agent hash walk failed: SE_HASH_WALK_LINK_BOUNDARY_V1'. (2) PC searches duplicates on a phone's internal storage -> list_dir of Android/data/<pkg> fails with EACCES -> whole search Failed. (3) A 300 GB share without links or denied folders -> the client daemon reads all 300 GB, one file at a time, before any result; over relay or mobile data this takes days and consumes the data volume.

**Richtung.** Add a host-side duplicate search (new capability and FsRequest, e.g. DuplicateSearch{path, min_bytes}) served like StorageAnalysis on a dedicated bounded worker with find_duplicates_in(root, progress, limits, &ProtectedAreas::for_walk(root), Some(&guard)), streaming progress and returning bounded groups plus summary; add a Backend hook forwarded by Caching/Agent/Unavailable backends and an IPC bridge like AnalyzeShare, and route desktop reclaim and Android reclaim.start for Share through it. Make the daemon answer WalkHashed as unsupported for backends without native hashes instead of emulating it by full download; at minimum never hash below the minimum size and treat links and unreadable folders as omissions.

**Gegenprüfung.** Every step was verified in the code.

No host-side hash support:
- FsRequest has no hash or duplicate operation (wire.rs:302-410).
- FsMeta has no hash field (wire.rs:228-239).
- PeerBackend sets content_md5: None (share backend.rs:471).

How the walk is chosen:
- The daemon always announces sync-links-v1: service_version (backend_server.rs:61-66) -> server_version_with (features.rs:31-41).
- The client sets mux.link_aware_hash from it (agent transport.rs:282-283), so agent_walk_hashed sends WalkHashed (search.rs:98-108).
- scan_reclaim_backend picks the agent route because AgentBackend::supports_walk_hashed is true (reclaim/backend.rs:54-56, 110). CachingBackend forwards it on the desktop.

The daemon handler (backend_walk.rs:306-378):
- runs a serial DFS with a stat (require_plain_directory 397-406) and a list_dir per directory (321-323);
- aborts on any link (336-339) and on any list/stat/read error via ?;
- MD5s every file with no size filter, because WalkHashed carries only root and want_hash; md5_backend reads the whole file over Share (380-395);
- enforces 1M nodes / 128 MiB (backend_budget.rs:3-5).

Errors become root_error with no fallback to the listing scan (reclaim/backend.rs:171-175, 183-187). Bisync has such a fallback for the link boundary (snapshot_agent.rs:90-95); reclaim does not. Android fails the task (analyze.rs:402-403). The desktop marks the run Failed and drops the report (reclaim_core.rs:325-334).

Only the max_items = 200 largest candidates are kept (types.rs:19; reclaim/backend.rs:377-401; backend_duplicates.rs:16-25). The guarded finder find_duplicates_in has no Share caller (only finder.rs:152 and tests).

Scenario 1 holds: Windows Documents junctions are NAME_SURROGATE reparse points, so link_like is true (directory.rs:355-358). Scenario 2 depends on Android FUSE behavior as documented in apptrash/mod.rs:46-50.

Severity: this is a broken feature plus a full download of the share, but there is no data loss or security exposure, so high rather than critical.

## A17 Host-side Share analysis adds lstat plus a full canonicalize for every directory that the same host's local analysis does not do

- Schwere: medium · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/os/shared/storage_analysis_host.rs:80-98; native/src/analytics/os/shared/analytics.rs:58-60; native/src/analytics/os/shared/analytics.rs:177-188; native/src/local_access/os/windows/read.rs:46-52; native/src/local_access/os/windows/read.rs:71-79; native/src/local_access/os/windows/directory.rs:109; native/src/local_access/os/windows/directory.rs:363-368; native/src/local_access/os/linux_os.rs:83-89

**Beschreibung.** scan_local_target passes a guard to scan_with_guard and the walker runs it for every directory before listing (analytics.rs:182-187); the GUI's local scan passes None (analytics.rs:58-60). The guard calls local_access::symlink_metadata(path) and std::fs::canonicalize(path) and compares with the canonical root (storage_analysis_host.rs:80-95). On Windows that is two additional CreateFileW/CloseHandle pairs plus GetFinalPathNameByHandleW per directory on top of the single directory open (directory.rs:109); for exported UNC/SMB connections every extra open is a network round trip. On Linux, realpath resolves the path component by component (libc behaviour, not repo-verified); on Android every lookup passes the FUSE layer. The check is not race-free either: read_directory(dir) reopens the path by name afterwards (analytics.rs:188); Linux read_dir follows symlinks and the Windows open avoids following only the final component (read.rs:46-52), so a component swapped between guard and open is still followed.

**Fehlerszenario.** Windows host exports a project drive with ~400k mostly small directories (node_modules). The local analysis does ~400k directory opens; the analysis requested by the phone does ~1.2M opens plus 400k final-path queries, estimated 1.5-2.5x slower on such trees; with an exported NAS share each extra open costs an SMB round trip. (Estimate; not measured.)

**Richtung.** Make confinement a property of how directories are opened: Unix/Android openat(parent_fd, name, O_DIRECTORY|O_NOFOLLOW) with fdopendir and an st_dev/st_ino check against the parent's entry; Windows open each child relative to the parent handle (NtCreateFile with RootDirectory and OBJ_DONT_REPARSE or FILE_FLAG_OPEN_REPARSE_POINT) and compare the FileId already delivered by FileIdExtdDirectoryInfo. That removes the per-directory path resolution and closes the race.

**Gegenprüfung.** scan_local_target builds a guard that calls local_access::symlink_metadata, metadata_is_link_like and std::fs::canonicalize per directory (storage_analysis_host.rs:80-95). It passes the guard to scan_with_guard (98). scan_dir calls the guard before read_directory for every directory, including the root (analytics.rs:182-188). The GUI and Android local scans pass None (analytics.rs:58-60).

On Windows:
- symlink_metadata is std::fs::symlink_metadata, one open/close (read.rs:71-79).
- canonicalize is a second open plus GetFinalPathNameByHandleW.
- The listing itself is one open (directory.rs:109 -> read.rs:20-52).
- reparse_tag adds a third open only for reparse-point directories (directory.rs:363-368).

UNC/SMB connection exports resolve to LocalBackend (fs.rs:186-200), so they take the same guard and pay network round trips.

The race is accurate. read_directory reopens by name afterwards. On Linux std read_dir follows symlinks (linux_os.rs:14). On Windows FILE_FLAG_OPEN_REPARSE_POINT affects only the final component (read.rs:46-52). Exploiting it needs a local actor on the host; Share offers no way to create a symlink.

The speed-up factors are estimates. The Linux realpath cost per component is libc behavior and was not verified in the repo. Medium is appropriate. The overhead remains after the routing fix in #0 and is part of why a remote-triggered analysis stays slower than the host's own.

## A18 Finished tree is transferred uncompressed and the desktop daemon decodes and re-encodes it before the GUI gets it

- Schwere: low · Urteil: partially_confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/analytics/core/tree_transfer.rs:7-12; native/src/analytics/core/tree_transfer.rs:58-80; native/src/analytics/core/analysis_transfer.rs:14-34; native/src/share/core/storage_analysis_server.rs:74-78; native/src/share/core/peer_storage_analysis.rs:49-62; native/src/daemon/os/shared/ipc_analysis.rs:104-117

**Beschreibung.** Each retained node is encoded as 17 bytes plus its name (tree_transfer.rs:155-163) and sent as raw 256 KiB TAG_DATA frames (storage_analysis_server.rs:74-78); up to 12M nodes / 2 GiB are allowed (tree_transfer.rs:95-96). No compression is applied although a deflate implementation is already in the dependency tree (zip with deflate / flate2-miniz, Cargo.toml:117). On the desktop path the daemon receives and fully decodes the tree (peer_storage_analysis -> AnalysisReceiver), keeps it, and only after the complete receipt re-encodes it with send_outcome for the GUI (ipc_analysis.rs:104-117): the GUI transfer cannot start before the peer transfer ends, and the daemon holds a second full copy.

**Fehlerszenario.** PC analyses another device with ~3M retained nodes (average name ~22 B, ~117 MB) over a relay path limited by a 20 Mbit/s home uplink: ~47 s of pure transfer after the host finished, followed by the daemon's decode/re-encode; names and sizes of such trees usually compress several-fold with deflate (estimate).

**Richtung.** Negotiate a compressed tree stream (deflate or zstd per chunk behind an additive capability flag); let the daemon relay the verified data frames to the GUI while hashing instead of decoding and re-encoding (or let only the GUI decode); optionally send completed subtrees while the host is still scanning.

**Gegenprüfung.** Confirmed parts:
- Records are 4-byte length + name + is_dir + u64 size + u32 child count, i.e. 17 B plus the name (tree_transfer.rs:70-75), in chunks of 256 KiB (12, 67-68).
- Limits are 12,000,002 nodes / 2 GiB (7-8).
- The host sends raw TAG_DATA frames (storage_analysis_server.rs:74-78). There is no compression anywhere. zip/deflate is used only for zipfs (Cargo.toml:115-117).
- On the desktop the daemon runs scan_remote -> peer_storage_analysis::receive_result -> AnalysisReceiver, a full decode (peer_storage_analysis.rs:49-62). Only afterwards does it re-encode with send_outcome (ipc_analysis.rs:104-117), so the GUI transfer cannot start before the peer transfer ends.

Incorrect or overstated:
- Every tree_transfer.rs line cited is 88 lines too high: 95-100 should be 7-12, 146-168 should be 58-80, 155-163 should be 67-75.
- "The daemon holds a second full copy" is overstated. encode moves the children out as it goes (tree_transfer.rs:62-77), so the daemon's copy shrinks while the GUI's grows. The bridge hop is loopback, so it adds only seconds even for multi-million-node trees.

The measurable cost is the uncompressed transfer over WAN or relay, bounded by the host's uplink; the project relay allows 64 MiB/s per client (share-server/src/relay.rs:28). This is an optimization opportunity, not a defect, so the severity is lowered.

## A19 Analysing a host's root '/' gives every export its own 6M-node budget; the merged tree can exceed the transfer limits and fails after the full scan

- Schwere: medium · Urteil: partially_confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/os/shared/storage_analysis_host.rs:32-36; native/src/share/os/shared/storage_analysis_host.rs:132-192; native/src/analytics/os/shared/analytics.rs:86-89; native/src/analytics/os/shared/analytics_budget.rs:11-12; native/src/analytics/core/tree_transfer.rs:7-8; native/src/analytics/core/tree_transfer.rs:21-56; native/src/analytics/core/analysis_transfer.rs:20; native/src/share/core/storage_analysis_server.rs:79-81; native/src/app/core/analytics_core.rs:425

**Beschreibung.** For '/' or '/Verbindungen' with dynamic access the host runs scan_container, which calls scan() for every export in sequence; each local export goes through scan_in, which creates a fresh AnalyticsBudget (6M retained nodes, 768 MiB of names). The merged tree is only checked afterwards in send_outcome -> tree_transfer::shape, which fails once nodes exceed 12,000,002 or bytes exceed 2 GiB; the error is sent to the client after the complete scan (storage_analysis_server.rs:79-81). The host meanwhile holds N times the per-scan budget in memory.

**Fehlerszenario.** PC exports C:, D: and E: with ~5M files each; the phone analyses the device root '/' -> after scanning all three drives shape() returns 'Ungültige Größe des Analyse-Ergebnisses' and the user gets no result.

**Richtung.** Share one AnalyticsBudget across all exports of a container scan (pass it into scan_in), sized to the transfer limits, so additional detail is aggregated instead of failing; validate incrementally rather than after the scan.

**Gegenprüfung.** The mechanism is confirmed:
- '/' and '/Verbindungen' with dynamic access go to scan_container (storage_analysis_host.rs:32-36).
- It calls scan() for each export in sequence (155) and keeps every child tree in memory (177-189).
- Each local export reaches scan_in, which creates a fresh AnalyticsBudget::default() (analytics.rs:88) of 6M nodes / 768 MiB (analytics_budget.rs:11-12).
- The merged tree is checked only in send_outcome -> tree_transfer::shape (analysis_transfer.rs:20). Per node, TreeShape::record calls validate, which fails above 12,000,002 nodes or 2 GiB with 'Ungültige Größe des Analyse-Ergebnisses' (tree_transfer.rs:7-8, 21-41, 44-56). The cited 95-96/109-117/132-144 are 88 lines too high.
- The worker error reaches the client only after the full scan (storage_analysis_server.rs:59, 79-81).

The stated failure scenario does not happen today. The phone never sends StorageAnalysis; it walks through ListDir (analyze.rs:207-211, see #0), so it never meets these limits but has unbounded memory instead. Today the defect hits desktop clients (app/core/analytics_core.rs:425 -> daemon -> PeerBackend), and Android only after the routing fix.

Medium is kept: the trigger needs very large multi-export hosts, but the result is a total failure after a long scan, with N full trees held on the host.

## A20 A lost connection discards the host's analysis; no host-side retention, re-attach or client retry

- Schwere: medium · Urteil: partially_confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/storage_analysis_server.rs:18-21; native/src/share/core/storage_analysis_server.rs:28-44; native/src/share/core/storage_analysis_server.rs:63-66; native/src/share/core/peer_storage_analysis.rs:10-36; native/src/share/core/peer_storage_analysis.rs:64-79; native/src/daemon/os/shared/ipc_analysis.rs:13-24; native/src/daemon/os/shared/ipc_analysis.rs:51-58; native/src/app/core/analytics_core.rs:425

**Beschreibung.** The host binds the analysis to the stream: when the client stream stops or the connection drops, serve returns and CancelOnDrop cancels the walker (storage_analysis_server.rs:18-21, 34-44, 63-66). The client makes a single attempt with no reconnect or re-attach, and any 60 s gap between complete frames ends it (peer_storage_analysis.rs:64-79). A finished or nearly finished host scan is thrown away and the next attempt scans from zero.

**Fehlerszenario.** Phone analyses a PC share whose host scan takes 15 minutes; at minute 14 the phone changes network or the app is frozen for more than the 20 s QUIC idle timeout -> connection lost -> error on the phone, the PC stops scanning, a retry starts over.

**Richtung.** Keep the analysis (running or finished) on the host for a grace period keyed by principal, root and a client request id; let the client re-attach after reconnecting and receive Ready/Data again (or from an offset); retry on transport loss in peer_storage_analysis.

**Gegenprüfung.** The code claims hold:
- CancelOnDrop sets cancel when serve returns (storage_analysis_server.rs:18-21, 30).
- serve returns when send.stopped() resolves (41, 65), which happens on STOP_SENDING or connection loss, or when a heartbeat or response write fails (`?` at 42, 66, 72).
- The client opens one stream, calls receive once and never retries (peer_storage_analysis.rs:24-35).
- A 60 s gap between complete frames ends it (64-79).
- scan_remote turns the error into Failed with no retry (remote.rs:11-15).
- The desktop bridge is also a single attempt with a 60 s gap limit (ipc_analysis.rs:13-24, 51-58).
- Nothing on the host keeps a finished outcome.
- The QUIC idle timeout is 20 s (keepalive.rs:7).

The failure scenario names a phone client. The Android facade does not use this path today (analyze.rs:207-211): its per-directory ListDir walk loses only the directories whose requests fail, and later requests reconnect (node_sessions.rs:173-191). So the scenario applies today to desktop clients (analytics_core.rs:425) and to Android only after the #0 fix. Medium is kept.

## A21 Host admission for analysis work has no per-peer fairness; legacy snapshot/walk streams hold the shared 32-slot control pool for their whole walk

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/storage_analysis_server.rs:23-26; native/src/share/core/storage_analysis_server.rs:36-44; native/src/share/core/storage_analysis_server.rs:51-60; native/src/share/core/blocking.rs:9-23; native/src/share/core/blocking.rs:41-55; native/src/share/core/storage_snapshot.rs:26-32; native/src/share/core/walk.rs:29-32; native/src/share/core/server.rs:183-188; native/src/share/core/server_admission.rs:25-31; native/src/share/core/keepalive.rs:8

**Beschreibung.** StorageAnalysis takes one of two process-wide permits (Semaphore::new(2)) with no per-principal share or fair queue; one peer (for example a Room member) can hold both, and every other request waits in Queued indefinitely because heartbeats keep it alive, with no queue position. Legacy StorageSnapshot and WalkTree run their entire walk inside blocking::spawn and hold one of the 32 control permits that ListDir, Stat and Capabilities of all peers also need (blocking.rs describes the pool as short metadata operations). With 64 bidi streams per connection (keepalive.rs:8) one client can occupy all 32 control permits and stall browsing and the Capabilities probe that precedes every analysis. Transfers, by contrast, have per-principal slots (server_admission.rs:25-31).

**Fehlerszenario.** Room member A starts two analyses of a large host; member B's analysis shows 'Wartet auf freien Analyse-Worker' for the whole duration. An older or faulty client that opens many StorageSnapshot streams freezes directory browsing on that host for every peer.

**Richtung.** Add a per-principal limit (for example one running analysis plus a bounded queue) and fair FIFO across principals for StorageAnalysis and report the queue position in progress; move StorageSnapshot and WalkTree to their own bounded pool like StorageAnalysis so long walks cannot starve control operations.

**Gegenprüfung.** Analysis slots:
- StorageAnalysis takes a permit from one process-wide Semaphore::new(2) (storage_analysis_server.rs:23-26).
- It waits in a select loop that sends heartbeats every 250 ms while queued (36-44), so the 60 s client frame timeout (peer_storage_analysis.rs:68) never fires and the wait is unbounded.
- The only feedback is the Queued phase ('Wartet auf freien Analyse-Worker', progress.rs:24); there is no position and no per-principal share.
- Transfers, by contrast, have per-principal slots (server_admission.rs:26-31, 61-77).

Control pool:
- Legacy StorageSnapshot (storage_snapshot.rs:26-32) and WalkTree (walk.rs:29-32) run the whole walk inside blocking::spawn, which holds one of 32 permits until the walk returns (blocking.rs:16, 41-55).
- They also hold it while blocked on network backpressure (blocking_send into a channel of 2).
- ListDir/Stat (server_fs.rs:89-98, 374-380) and Capabilities (server_capabilities.rs:36-39) draw from the same pool.
- One connection may have 64 bidi streams (keepalive.rs:8), and every stream is spawned independently (server.rs:183-188).

One correction: blocking.rs:9-15 explicitly says the control pool also carries the tree walks, so this is a deliberate design whose fairness gap is real, not a misuse. Exhausting the pool needs an old, faulty or deliberately abusive authorized peer. Medium is fair.

## A22 Remote analysis carries none of the volume and Android platform figures the host's own analysis shows

- Schwere: medium · Urteil: confirmed · Kategorie: gap · Plattformen: Android (host), Windows (host), Linux (host)
- Stellen: native/src/analytics/core/analysis_report.rs:7-17; native/src/analytics/core/analysis_report.rs:61-85; native/src/mobile/os/shared/domains/analyze.rs:166-181; native/src/mobile/os/shared/domains/analyze_platform.rs:11-55; native/src/app/core/analytics_core.rs:399-411; native/src/share/os/shared/storage_analysis_host.rs:104-114

**Beschreibung.** The phone's own analysis receives Kotlin-measured platform totals (volume used, other apps' data, installed apps) for local roots and turns them into approximation rows (analyze.rs:166-171, analyze_platform.rs). A Share-hosted analysis on the phone has no source for these figures and the wire report has no field for them (AnalysisReport has status, issues, notes, counters, shape only), so a remote view lacks the apps, other-app-data and not-captured rows. On the desktop the GUI shows drive used/total only for local sources (analytics_core.rs:399-411); hosts never report the capacity of the analysed volume. Protected omissions are flattened into one note string (storage_analysis_host.rs:104-114; analysis_report.rs:81-83 sets protected to empty), so a remote client cannot show the structured protected section either.

**Fehlerszenario.** PC analyses the phone's internal storage: it shows only the walkable files (for example 38 GB) while the phone's own analysis shows 112 GB used including the app and other-app-data rows.

**Richtung.** Extend AnalysisReport additively (serde default) with optional volume totals, Android platform totals and structured protected omissions; the Android host obtains platform figures from the app runtime (Kotlin storage-stats bridge or cached values), desktop hosts via GetDiskFreeSpaceExW/statvfs; clients feed them into Approximations and the drive-usage display.

**Gegenprüfung.** AnalysisReport carries only status, issues, suppressed_issues, permission_denied, notes, aggregated_files, progress and shape (analysis_report.rs:7-17). finish sets protected to Vec::new() (81-83).

The host flattens protected omissions into the first note (storage_analysis_host.rs:104-114). The Android facade gets platform figures only for local roots and passes VolumeRoot::default() / PlatformTotals::default() for remote ones (analyze.rs:168-173; analyze_platform.rs:11-55). Those figures come from Kotlin on the device being analysed.

The desktop shows drive used/total only for StorageScanSource::Local (analytics_core.rs:399-411). No Share message carries volume capacity: a grep of share/ and analytics/core for capacity fields found only the local PlatformTotals.

A remote view therefore lacks the volume, apps, other-apps-data and protected rows the host's own run shows. This directly feeds the user's report that a remote analysis differs from the local one. Medium is fine.

## A23 Receiver keeps a host-sized tree without regard to its own memory; Android stores up to four results

- Schwere: medium · Urteil: confirmed · Kategorie: platform · Plattformen: Android (client)
- Stellen: native/src/analytics/core/tree_transfer.rs:7-8; native/src/analytics/core/tree_transfer.rs:94-138; native/src/analytics/core/analysis_transfer.rs:45-81; native/src/mobile/os/shared/domains/analyze.rs:28; native/src/mobile/os/shared/domains/analyze.rs:58-77; native/src/analytics/os/shared/analytics_backend.rs:101-111; native/src/analytics/os/shared/analytics_backend.rs:178-215

**Beschreibung.** The decoder accepts up to 12,000,002 nodes / 2 GiB from any host and materialises every node as SizeNode plus boxed name. The retention budget is the host's, not the viewer's. Android keeps up to four finished results process-wide (MAX_RESULTS = 4). Assumption: about 90 B per node including allocator overhead, so a 6M-node PC tree costs roughly 0.5 GB native heap per stored result on the phone; on the desktop path the daemon briefly holds an additional copy (see the transfer finding). Today's Android scan_backend path is worse (no retention at all).

**Fehlerszenario.** After the Android routing fix, a phone views the analyses of two large PCs (about 6M retained nodes each) -> more than 1 GB native heap -> the app is killed by the low-memory killer while browsing the result.

**Richtung.** Let the client announce a node budget derived from its memory in the analysis request so the host aggregates beyond it; keep fewer remote results on Android or spill them to disk.

**Gegenprüfung.** The claims hold, but the tree_transfer.rs lines are 88 too high.
- Limits are MAX_NODES = 12,000,002 and MAX_BYTES = 2 GiB (tree_transfer.rs:7-8).
- TreeDecoder::push materialises every record as SizeNode plus a boxed name (94-120, SizeNode at 115). It is bounded only by these host-side limits, not by the viewer's memory.
- AnalysisReceiver feeds every data frame into it (analysis_transfer.rs:72-81).
- Android keeps up to MAX_RESULTS = 4 results and evicts only finished ones (analyze.rs:28, 58-77).
- About 90 B per node is a plausible estimate (56-byte SizeNode plus name allocation); it was not measured.

The finding itself frames the scenario as after the Android routing fix. Today the Android facade's scan_backend path is the real risk. It ignores retention (`let _ = budget.claim`, analytics_backend.rs:101-106), keeps all listings and then a cloned tree (178-215), and folds only files beyond 4096 per directory (121-150). Medium is kept.

## A24 Share ListDir replies are one JSON frame; folders above ~95k entries cannot be listed and one bad entry fails the whole listing

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows (host), Linux (host), Android (host)
- Stellen: native/src/share/core/server_fs.rs:89-94; native/src/share/core/framing.rs:11; native/src/share/core/framing.rs:63-70; native/src/share/core/wire.rs:228-239; native/src/vfs/os/shared/local.rs:75-103; native/src/share/core/peer_request.rs:15-16; native/src/share/core/peer_request.rs:53-61; native/src/agent/core/backend.rs:18

**Beschreibung.** The host answers ListDir with a single FsResponse::Entries control frame; send_tagged refuses frames above 16 MiB. Each FsMeta costs about 151 B plus the name in JSON, so roughly 95-100k entries is the ceiling; above it the host stream ends without a reply, the client sees an EOF/stream error, retries once and fails. LocalBackend::list_dir collects io::Result entries, so a single failing entry (other than NotFound), an unrepresentable or non-Unicode name fails the entire directory. Retryable control requests get 20 s per attempt, the agent hop 20 s end to end. This is the transport of today's Android remote analysis and of the daemon's hash walk, and it also limits browsing.

**Fehlerszenario.** Phone analysis or browsing of a PC folder with 150k files (photo dump, mail store, cache) -> the folder fails ('peer stream closed after 0 bytes') and counts as 0 B; a Linux folder containing one Latin-1-named file fails entirely.

**Richtung.** Stream listings in bounded batches (as WalkBatch already does) or a compact binary encoding with continuation; record failing entries instead of failing the listing; base timeouts on progress instead of a fixed 20 s.

**Gegenprüfung.** The frame cap:
- ListDir replies with one FsResponse::Entries (server_fs.rs:89-94) through send_ctrl -> send_tagged, which refuses payloads above MAX_FRAME = 16 MiB (framing.rs:11, 29-31, 63-70).
- reply's error propagates out of serve with no fallback reply_err. The host only emits a connection error (server.rs:185-187), and the client receives no answer frame.
- FsMeta has nine fields and no skip_serializing (wire.rs:228-239), about 150 B of JSON plus the name. That puts the ceiling near 95-100k entries.
- The client's recv_ctrl uses the same 16 MiB cap. The agent hop allows 64 MiB (agent_proto codec.rs:7), so the Share hop is the binding limit.

Retries:
- ListDir is retryable: 2 attempts of 20 s within a 40 s budget (peer_request.rs:16-17, 56-61, 78-79, 372-380).
- The agent hop has 20 s end-to-end with no retry on the reconnect-less connection (agent backend.rs:18, 128-132; transport.rs:87-104).
- On slow uplinks a frame of several MiB also exceeds the 20 s attempt deadline, so the practical ceiling is lower still.

One bad entry: LocalBackend::list_dir collects into VfsResult<Vec>. Unreachable or non-Unicode names and any entry error other than NotFound fail the whole directory (local.rs:77-102).

This is the transport of the Android analysis, the daemon hash walk (backend_walk.rs:323) and browsing. Medium is fine.

## A25 Receiver rejects the whole result when the host's byte counter and tree size differ

- Schwere: low · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/analytics/core/analysis_report.rs:61-72; native/src/analytics/os/shared/analytics.rs:177-200; native/src/analytics/os/shared/analytics.rs:269-287; native/src/share/os/shared/storage_analysis_host.rs:155-182; native/src/analytics/os/shared/remote.rs:22-26; native/src/analytics/os/shared/remote.rs:37

**Beschreibung.** AnalysisReport::finish fails with 'Analyse-Zähler und Baum stimmen nicht überein' unless tree.size equals progress.bytes. On the host, file bytes are added to the shared counters while a folder is listed (every 128 files and at the end, analytics.rs:269-287); if a panic is then caught for that folder, scan_dir returns an empty 0-byte node (177-200) while the bytes remain counted. In scan_container a child export whose legacy walk_tree reported progress (remote.rs:22-26) and then failed returns ScanOutcome::failed without a tree (remote.rs:37); the container inserts a 0-byte node (storage_analysis_host.rs:177-182) but keeps the counted bytes. Locally such events give a Partial result; remotely the client discards everything.

**Fehlerszenario.** Peer analyses host '/' with shared connections; one exported SFTP connection's agent walk fails after reporting 200k files -> after the complete scan the client rejects the result of all exports.

**Richtung.** Snapshot the counters before each child/folder and roll them back to the tree's real totals on failure or panic (or derive the reported totals from the tree); alternatively accept bytes >= tree.size for Partial results and report the difference.

**Gegenprüfung.** AnalysisReport::finish rejects the result when tree.size != progress.bytes (analysis_report.rs:61-72). The client runs it on Done (analysis_transfer.rs:63-65).

Panic case: scan_entries adds file bytes to the shared counters every 128 files and at the end (analytics.rs:269-287). A panic caught by catch_unwind makes scan_dir return empty_dir, a 0-byte node (analytics.rs:177-200), while those bytes stay counted.

Container case:
- scan_container shares the counters through scoped()/remote_segment(), which clone the Arcs (progress.rs:80-87).
- A connection export whose legacy walk_tree stored progress (remote.rs:22-26) and then failed returns ScanOutcome::failed with no tree (remote.rs:37).
- The container inserts a 0-byte node and keeps going (storage_analysis_host.rs:159-189).
- The host's send_outcome does not check this, so the client rejects the result of every export.

A single non-container export that fails has no tree, so the check is skipped; only the container case or a panic triggers it. Both are rare, so low is right.

## A26 Host path remapping treats physical paths that start with the export name as already visible; Windows display-form paths stay unmapped

- Schwere: low · Urteil: confirmed · Kategorie: bug · Plattformen: Linux (host), Windows (host), Android (host)
- Stellen: native/src/analytics/core/progress.rs:89-102; native/src/share/os/shared/storage_analysis_host.rs:116-130; native/src/analytics/os/shared/analytics_budget.rs:97-101

**Beschreibung.** visible_path returns a path unchanged when it already starts with the visible root plus '/' (progress.rs:92-94). If an export's visible name equals the first physical component (Linux export 'home' for /home/alice, visible root /home), the physical /home/alice/x is returned as is: progress and issue paths are wrong and expose the host's absolute path; remap_issues computes visible = visible_path(physical), which is also unchanged, so details keep the physical path. On Windows remap_issues replaces only the verbatim forms of the root, while notes built from display_path (the budget note 'Detailansicht ab C:/Users/... zusammengefasst', analytics_budget.rs:98-101) keep the host's absolute drive path.

**Fehlerszenario.** Linux host shares /home/alice as 'home'; the peer sees the issue '/home/alice/.cache/x: Permission denied' instead of '/home/.cache/x'. A Windows host's aggregation note reveals the user's profile path.

**Richtung.** Treat all walker paths on the host as physical and strip the physical root prefix (both display and verbatim forms) instead of guessing by prefix; apply the same mapping to notes.

**Gegenprüfung.** visible_path returns any path that equals the visible root or starts with '<visible>/' unchanged (progress.rs:89-94).

Linux/Android collision: scan_local_target scopes (physical display path, visible root) (storage_analysis_host.rs:59-63). For physical /home/alice exported as label 'home' (visible /home):
- every physical path starts with '/home/' and stays unmapped;
- this covers set_phase current (progress.rs:104-105, 120-122) and issue paths (storage_analysis_host.rs:119);
- remap_issues' `visible = visible_path(physical)` (117) is unchanged too, so the detail and note replacements are no-ops.

Windows: target.path is the canonical verbatim form '//?/C:/…' (fs.rs:313-321, 362-364). remap_issues replaces only that form and its backslash variant (120-128). Notes built with display_path, which strips the verbatim prefix (paths.rs:135-144), keep 'C:\Users\…' unmapped, e.g. the budget note at analytics_budget.rs:97-101. Issue paths themselves are mapped correctly through visible_path.

The Linux case needs a label equal to the first physical component. Impact is limited to the host's absolute path being shown to an already-authorized peer, so low is right.

## A27 Remote-requested analysis on a Windows host cannot use the consented elevated read access a local analysis uses

- Schwere: low · Urteil: confirmed · Kategorie: platform · Plattformen: Windows (host)
- Stellen: native/src/local_access/os/windows/read.rs:20-44; native/src/local_access/os/windows/broker.rs:30-33; native/src/share/core/storage_analysis_server.rs:51-60; native/src/share/os/shared/storage_analysis_host.rs:80-98; native/src/autostart/os/windows.rs:2; native/src/app/core/analytics_access.rs:94-115

**Beschreibung.** On access denied, local reads retry with SeBackupPrivilege (only effective in elevated processes) and then with a UAC helper grant kept in a process-global client list (broker.rs:30-33) that the GUI creates after user consent. Share analyses run in the non-elevated background daemon (ipc.rs:76-80 and the share node), which has neither, and the client only offers an access request for local sources (analytics_access.rs:94-110). Folders the local analysis reads after consent are therefore unreadable remotely, with no way to request access and no explanation in the result.

**Fehlerszenario.** User analysed C: locally after granting elevated read (complete); the same C: analysed from the phone shows hundreds of unreadable folders (Partial).

**Richtung.** At least name this cause in the remote result; optionally let the host's user extend a consented read grant to Share analyses (daemon-side broker), never implicitly.

**Gegenprüfung.** On PermissionDenied, local reads first try BackupRead::enable(), which needs SeBackupPrivilege in the token, and then broker::open_granted (read.rs:29-41). Broker grants live in a process-global client list (broker.rs:30-33) that the GUI fills after user consent (analytics_access.rs:102-115). Clients offer the access request only for StorageScanSource::Local (analytics_access.rs:107-109).

The host side of a Share analysis runs in the Share node: storage_analysis_server.rs:51-60 -> storage_analysis_host.rs:98 inside the background worker process. That process is autostarted through the HKCU Run key (autostart/os/windows.rs:2), so it is normally non-elevated and holds no GUI grants.

Correction: ipc.rs:76-80 is the client-side AnalyzeShare bridge, not the host side.

Result: folders that a consented local analysis can read come back as PermissionDenied issues remotely, with nothing in the result hinting that elevated access could be granted on the host. This is a platform gap, so low is right.

## A28 Legacy StorageSnapshot/WalkTree host path aborts on the first unreadable folder and at 1M nodes

- Schwere: low · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/storage_snapshot.rs:89-130; native/src/share/core/walk.rs:188-214; native/src/share/core/walk.rs:283-299; native/src/share/core/peer_storage_snapshot.rs:20-63

**Beschreibung.** Served to clients without storage_analysis_v2 and used by current clients against older hosts: a serial walk that re-resolves the export for each stat and list_dir (fs::resolve with canonicalisation) and propagates any error with ?, so one unreadable folder or more than 1M nodes ends the snapshot with an error instead of a partial tree.

**Fehlerszenario.** An older desktop client analyses an updated phone host whose root contains Android/data -> the snapshot fails at the first package folder.

**Richtung.** Record unreadable folders as empty nodes in the legacy server walk, or answer such clients with an explicit 'please update' error; low priority if all devices update together.

**Gegenprüfung.** ServerWalker::next_event propagates errors with ?: stat (walk.rs:196), list_dir (206) and validate_name (231). reserve fails at MAX_WALK_NODES = 1,000,000 (walk.rs:283-286; walk_assembly.rs:7). build_snapshot propagates walker errors and stops at MAX_SNAPSHOT_NODES and 64 MiB encoded (storage_snapshot.rs:15-16, 94-121). With dynamic access each stat and list_dir re-resolves the export with canonicalisation (fs_access.rs:41-64 -> fs.rs:146-183, 311-344).

The client picks StorageSnapshot or WalkTree by capability (peer_storage_snapshot.rs:20-63), and only when storage_analysis_v2 is absent (peer_storage_analysis.rs:15-20 -> remote.rs:16-27).

Nuance: this repository's walk.rs runs only when an updated host serves an old client. The scenario given (old client analysing an updated phone host) is exactly that case. A current client against an older host executes that host's old code. Low is right.

## A29 A host progress path longer than 32 KiB makes the client abort the whole analysis

- Schwere: low · Urteil: partially_confirmed · Kategorie: bug · Plattformen: Windows (host), Linux (host)
- Stellen: native/src/analytics/core/progress.rs:147-160; native/src/share/core/storage_analysis_server.rs:106-108; native/src/analytics/core/analysis_transfer.rs:50-53; native/src/analytics/os/shared/analytics.rs:177-181; native/src/analytics/os/shared/analytics_backend.rs:72

**Beschreibung.** Progress::receive rejects a snapshot whose current path exceeds 32 KiB with InvalidData; the host sends current untruncated in every heartbeat and in the Ready report, and AnalysisReceiver propagates the error, so the remote analysis fails. A local analysis never applies this limit.

**Fehlerszenario.** Windows host with a very deep tree of long CJK-named folders (verbatim paths up to 32,767 UTF-16 units, up to ~96 KiB in UTF-8): a heartbeat taken while the walker is inside such a folder makes the phone's analysis fail with 'Ungültiger Analyse-Fortschritt'.

**Richtung.** Truncate current on the host before sending (keep the tail with an ellipsis) and clamp instead of failing on the client.

**Gegenprüfung.** Confirmed: Progress::receive rejects any snapshot whose current exceeds 32 KiB (progress.rs:155-159). AnalysisReceiver propagates the error for both Progress and Ready (analysis_transfer.rs:50-53). The host sends progress.snapshot() untruncated in heartbeats (storage_analysis_server.rs:106-108). The local walker sets current before listing (analytics.rs:177-181). Note that Ready carries current = root, set by set_phase(Assembling) at storage_analysis_server.rs:98, so only heartbeats can carry a long path.

Platform correction: Linux and Android local exports cannot trigger it. The walker can only enter directories whose path fits PATH_MAX (4096 B); read_dir fails beyond that (linux_os.rs:14). So current stays at most about 4.3 KiB: parent ≤ 4095 + 1 + 255-byte name.

The trigger needs either a Windows host with very deep non-ASCII verbatim paths (beyond about 10.9k CJK characters) or an exported remote connection with very long paths (e.g. Drive) through scan_backend's enter_directory (analytics_backend.rs:72). Both are rare, so low is right.

## A30 Any Direct contact going offline/online, a new Room member or any export edit force-closes every Share connection, aborting running remote analyses on both ends

- Schwere: high · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/authorization_policy.rs:30; native/src/share/core/authorization_policy.rs:69-80; native/src/share/core/configuration_runtime.rs:70-76; native/src/share/core/configuration_runtime.rs:102-107; native/src/share/core/node.rs:300-329; native/src/daemon/os/shared/ipc_host_events.rs:117-130; native/src/daemon/os/shared/ipc_host_events.rs:367-379; native/src/share/core/mount_lease.rs:88-100; native/src/share/core/peer_request.rs:56-61

**Beschreibung.** The share core forwards presence messages without touching its runtime state (signal_auth.rs:27-40). The daemon writes them into the profiles: DirectAvailable sets contact.presence = Some (ipc_host_events.rs:117), DirectOffline sets it to None (:129), and a newly seen Room member is pushed into room.members (:469). Each of these sets changed = true, persists the profiles (merge_worker_updates copies presence, profile_merge.rs:32) and sends ShareCmd::ConfigureProfiles (ipc_host_service.rs:258-261). RuntimeConfiguration::apply then calls configuration_changed, which counts these as authorization changes: same_contacts compares presence (authorization_policy.rs:30), same_presence returns false for None versus Some (:79), and same_rooms compares the member list and exports (:51-52). When it reports a change, apply calls invalidate_sessions (configuration_runtime.rs:101-106). That drains and closes every incoming and outgoing connection with code 0x5345 and clears all mount leases (node.rs:300-329). A storage analysis is the longest single stream in the protocol. The host loses its work through stopped → canceled, which trips CancelOnDrop. The client sees a stream error. There is no retry (that gap is already listed separately). Presence is not authorization, and a change for one contact or room should not close the sessions of other peers.

**Fehlerszenario.** A phone analyses a PC's share (or the PC analyses the phone) and the scan takes several minutes. Meanwhile a third Direct contact of the PC (a laptop going to sleep, a phone switching networks) disconnects from the signaling server. The PC's daemon receives DirectOffline, sets that contact's presence to None and reconfigures the service. configuration_changed reports true and invalidate_sessions closes all connections. The running analysis fails with a connection error and the host's partial scan is discarded. The peer coming back online (DirectAvailable, None → Some) closes everything a second time. The same happens on every device that has the flapping peer as a contact, and whenever a new member joins any room the device belongs to. On the Android client listing path, the in-flight ListDir requests fail and their folders become unreadable gaps in a 'partial' result.

**Richtung.** Remove runtime presence (and the other runtime-only contact fields) from the authorization comparison: treat None↔Some as unchanged and compare only the pinned identity fields (expected_node_id, keys, fingerprint, access_state). Then scope invalidation to the affected relation. Close only the incoming/outgoing connections and leases of a contact, grant or room whose authorization or exports actually changed, keyed by relation (session_key / PeerPrincipal), not every connection on the node. Adding a member to a room should not close any existing session. Add a task test that a presence flip and a member addition keep an unrelated running analysis stream open.

**Gegenprüfung.** I traced the whole chain. (1) The core only forwards presence. signal_auth.rs:27-40 emits DirectAvailable/DirectOffline, and verify_direct_presence (:226-277) only records a nonce; it never writes contact.presence. (2) The daemon writes presence and sets changed: ipc_host_events.rs:117-118 (Some) and :128-130 (None). A new room member is pushed at :469-482, reached from RoomJoined :242-245 and RoomRoster :220-225. (3) The profiles are persisted with presence: profile_merge.rs:32 copies it and :88-96 adds new members. They are then pushed out: ipc_host_events.rs:367-379 -> configure_service (ipc_host_service.rs:251-263) -> ConfigureProfiles (signal_commands.rs:112-136 connected, :301-308 offline) -> RuntimeConfiguration::apply. (4) configuration_changed counts these: authorization_policy.rs:30 with :69-80 returns false for None vs Some, and same_members compares length (:58). (5) configuration_runtime.rs:70-76 and :102-104 then call invalidate_sessions. node.rs:300-329 drains every outgoing and incoming connection, clears all mount leases (:302) and closes each with 0x5345. Presence is not an authorization input: incoming streams are authorized only from grants, members and secrets (session.rs:185-252). The 2026-09-28 throughput lesson (sections 2 and 6) describes this invalidation as meant for policy edits. A running v2 analysis dies: the host returns canceled at storage_analysis_server.rs:34-35/:63-66, CancelOnDrop (:18-21, :30) stops the worker, and the client side has no retry (peer_storage_analysis.rs:27-35, ipc_analysis.rs:13-24, remote.rs:12-15). Correction: the Android-client sub-claim is overstated. The phone runs a ListDir walk (analyze.rs:207-209), and ListDir is retried once on a fresh connection within a 40 s budget (peer_request.rs:56-61, 78-120, 372-380). A gap therefore needs a second close during the same request, for example Offline followed shortly by Available. The impact also reaches beyond analysis. Leases bound to the old epoch are rejected (mount_lease.rs:88-100), so mounted drives and leased writes are hit, along with every transfer to every peer. High stands.

## A31 Android host: a peer analysis that started while the app was visible gets no CPU hold once the app goes to background

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Android
- Stellen: native/src/share/core/node_idle.rs:257-288; native/src/share/core/power_hub.rs:90-92; native/src/share/core/signal_power.rs:71-73; native/src/mobile/os/shared/domains/share_power.rs:42-46

**Beschreibung.** incoming_stream_started requests the 60-s stream hold once, when the stream starts (node_idle.rs:261). After that, renew_stream_holds sleeps 30 s before each new request (:276, :287). PowerHub::request_hold returns at once unless the device is already in low power (power_hub.rs:91). Low power is switched only by app visibility (share_power.rs:41-47), and set_low_power merely wakes the signal subscribers (power_hub.rs:64-68). Nothing requests a hold for streams that are already running. So when the phone app is visible as a peer's analysis starts, no wake lock is held after the app goes to background until the next renewal tick. That tick is a tokio timer on the monotonic clock, which does not advance while the device is suspended. The host's analysis worker and its 250-ms heartbeat (also a tokio interval) freeze. A local analysis on the phone would simply pause, but the remote client gives up after 60 s without a frame. Incoming peer work is also not a TaskKeeper task, so nothing else keeps the process or CPU awake.

**Fehlerszenario.** The user has the Share screen open on the phone while starting 'Analysieren' for the phone's share on the PC, then locks the phone. Within seconds the phone suspends because no wake lock is held. The renewal timer cannot reach its 30-s tick while suspended. The PC's peer_storage_analysis::next_frame ends with 'Seit 60 Sekunden keine vollständige Analyse-Meldung der Gegenstelle' and the host-side scan is discarded.

**Richtung.** When entering low power, request STREAM_HOLD_MS immediately if StreamHolds.active > 0. For example, give PowerHub a hook that node_idle registers, or let the renewal loop subscribe to low-power transitions instead of polling every 30 s. Make the renewal deadline not depend on a monotonic timer that stops during suspend. Consider treating a running incoming analysis as keep-alive work for TaskKeeper as well.

**Gegenprüfung.** node_idle.rs:257-266 requests STREAM_HOLD_MS once, when the stream starts, and spawns the renewer. The renewer sleeps 30 s before every request (:274-288, STREAM_HOLD_RENEW :27). power_hub.rs:90-92 drops any request while the device is not in low power. Low power is switched only by visibility (share_power.rs:42-46; its only caller is domains/mod.rs:90). set_low_power only wakes subscribers (power_hub.rs:64-68). On entering low power the signal worker only runs an idle sweep (signal_power.rs:71-73): busy connections are kept and no hold is requested. The other holds are 5-15 s and tied to signal activity (signal_session.rs:29, :62; signal_worker.rs:137; tracked_signal_dispatch.rs:75/94/112). So a StorageAnalysis stream that began while the app was visible has no wake lock from the moment the app is hidden until the renewer's next tick. That tick comes after at most 30 s of tokio time, which runs on the monotonic clock and pauses during suspend. During suspend the worker and the 250 ms heartbeat freeze (storage_analysis_server.rs:11, :32-33, :51-60). The desktop client gives up after 60 s without a frame (peer_storage_analysis.rs:64-79). WakeKeeper acts only on `wake` events (Core.kt:192, WakeKeeper.kt:46-63). Caveats: whether the result is a timeout or only a crawl depends on how often incoming QUIC keepalives (keepalive.rs:5) wake the device, which the code cannot decide. BackgroundService (a specialUse foreground service, BackgroundService.kt:30-38) does keep the process alive when background Share is enabled, so only the CPU half of 'nothing keeps the process or CPU awake' holds. The scenario fits a desktop client analysing the phone.

## A32 Desktop hosts never hold off system sleep while serving a peer's analysis

- Schwere: medium · Urteil: confirmed · Kategorie: platform · Plattformen: Windows, Linux
- Stellen: native/src/share/core/power.rs:1-7; native/src/share/core/power_hub.rs:90-92; native/src/mobile/os/shared/domains/analyze.rs:207-209

**Beschreibung.** The only keep-awake mechanism for incoming peer work is PowerHub::request_hold → the Android wake-lock hook. Its own documentation says 'A desktop never calls any of this' (power.rs:6-7). A search of native/src finds no SetThreadExecutionState, PowerCreateRequest/PowerSetRequest, or logind/systemd-inhibit sleep inhibitor. A peer-requested analysis on a Windows or Linux host does not reset the OS idle timer, because disk and network activity is not user input. A laptop host can therefore suspend in the middle of a long analysis whose requesting user is on another device. Neither side resumes or retries afterwards.

**Fehlerszenario.** A laptop runs on battery with the usual 5–15-minute sleep timeout. Its owner walks away and analyses the laptop's 2-million-file share from the phone. The laptop sleeps during the scan. The phone's request fails after 60 s without frames and all host-side scan work is lost; the same applies to a desktop GUI client analysing the laptop.

**Richtung.** While incoming analysis or transfer streams are active, hold a bounded system-required request. On Windows use PowerCreateRequest/PowerSetRequest(PowerRequestSystemRequired) or SetThreadExecutionState(ES_SYSTEM_REQUIRED|ES_CONTINUOUS) on a dedicated thread. On Linux take a logind 'sleep' inhibitor (delay or block mode). Release it when the last stream ends, behind the same per-OS adapter the Android hook uses (os/ layer).

**Gegenprüfung.** There is no sleep inhibition anywhere. Searching native/src, share-server/src and all non-doc repository files for SetThreadExecutionState, ES_SYSTEM_REQUIRED, PowerCreateRequest, PowerSetRequest, inhibit, org.freedesktop.login1 and caffeinate finds nothing. The only Windows power API in use is GetSystemPowerStatus for battery-saver detection (daemon/os/windows/platform.rs:310-316). power.rs:1-7 states that a desktop never uses the hold path, and request_hold is gated on low_power, which only Android sets (share_power.rs:45). A desktop client then fails after 60 s without a frame (peer_storage_analysis.rs:64-79, ipc_analysis.rs:53-57), with no resume (peer_storage_analysis.rs:27-35, remote.rs:12-15). Correction to the scenario: a phone client never sends StorageAnalysis. It walks with ListDir (analyze.rs:207-209). When the laptop sleeps, the pending ListDir calls fail once their retry budget runs out (peer_request.rs:16-17, 56-61). Those folders become unreadable gaps (analytics_backend.rs:196-200), giving a Partial or Failed result rather than a 60 s frame timeout, and that flow has no host-side scan to lose. The gap also covers all other incoming Share work (transfers, mounted drives).

## A33 Android client: remote analysis and duplicate search run without a CPU wake lock, so screen-off turns them into partial or failed results

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Android
- Stellen: android/app/src/main/java/app/smartexplorer/android/service/TaskForegroundService.kt:83-97; native/src/daemon/os/shared/backend_walk.rs:323; native/src/daemon/os/shared/backend_walk.rs:355-358; native/src/daemon/os/shared/backend_walk.rs:380-395; native/src/analytics/os/shared/reclaim/backend.rs:172-187; native/src/mobile/os/shared/domains/analyze.rs:402-404; native/src/share/core/peer_request.rs:56-61

**Beschreibung.** TaskKeeper keeps the process alive for user tasks with a dataSync foreground service. That service takes no wake lock: the only PARTIAL_WAKE_LOCK in the app is WakeKeeper, which fires only on core `wake` events and keep-alive probes. The core requests holds only for incoming streams and signal activity, never for outgoing work such as an `analyze` or `reclaim` task on a Share location. With the screen off, the phone suspends mid-task. The PC side's QUIC idle timeout (20 s) and per-operation deadlines keep running, so after wake-up the in-flight requests fail. In the listing analysis, a failed list_dir becomes an unreadable, zero-size folder in a 'partial' result. In the duplicate search, the daemon's hash walk aborts completely on the first failed open_read (backend_walk.rs `md5_backend(...)?`). A local analysis on the phone only pauses and still returns correct totals. The analysis page even tells the user 'Der Scan läuft weiter, wenn diese Seite verlassen wird'.

**Fehlerszenario.** The user starts 'Speicheranalyse' of the PC's share on the phone and locks the screen. The phone suspends for a minute or more. The PC closes the idle QUIC connection. On wake, the outstanding ListDir requests fail and their folders are recorded as unreadable: the result shows a partial tree with silently missing gigabytes. A duplicate search started the same way ends with 'agent hash walk failed'.

**Richtung.** While a user task that depends on a remote peer (analyze, reclaim, transfers) is running, hold a partial wake lock bounded by the task's lifetime. For example, TaskForegroundService could hold one while Core.tasks has active remote tasks, or the task runtime could emit `wake` holds and renew them. Also make the listing walk retry a directory after a connection loss rather than recording it as unreadable.

**Gegenprüfung.** TaskForegroundService.kt only promotes a dataSync foreground service (:83-97) and never takes a wake lock. The app's only PARTIAL_WAKE_LOCK is in WakeKeeper.kt:87-88, driven by `wake` events (Core.kt:192) and keep-alive probes (KeepAliveAlarm.kt:60, KeepAliveNetwork.kt:66, :81). The core requests holds only for incoming streams (node_idle.rs:261/:287) and signal activity. Nothing requests one for outgoing analyze or reclaim tasks (analyze.rs:121-145, 204-222, 380-398). The QUIC idle timeout is 20 s (keepalive.rs:7). The duplicate-search consequence holds. The phone's embedded daemon hashes every file serially in handle_walk_hashed_backend (routed from backend_server.rs:271). It aborts on the first failed list, open or read via `?` (backend_walk.rs:323, :355-358, :380-395). That failure becomes root_error (reclaim/backend.rs:172-173, :183-187) and the task ends with an error (analyze.rs:402-404). The listing-analysis consequence is overstated. Each ListDir is retried once on a new connection within a 40 s budget (peer_request.rs:56-61, 78-120, 372-380), so a single connection loss after wake-up is normally absorbed. A gap (analytics_backend.rs:196-200) needs both attempts to fail, for example when the phone suspends again during the retry. That is plausible without a wake lock but device-dependent.

## A34 Share duplicate and cleanup results cannot be acted on: both clients are display-only although the host has a trash

- Schwere: low · Urteil: confirmed · Kategorie: gap · Plattformen: Windows, Linux, Android
- Stellen: native/src/app/core/reclaim_core.rs:170-173; native/src/app/core/reclaim_ui.rs:226-238; native/src/analytics/os/shared/reclaim/verify.rs:20-24; native/src/mobile/os/shared/delete.rs:57-63; android/app/src/main/java/app/smartexplorer/android/ui/analytics/DuplicatesViewModel.kt:160-167; native/src/share/core/wire.rs:302-372

**Beschreibung.** After a local duplicate search or 'Find & Reclaim' run, the host lets the user move copies to its recycle bin (desktop) or app trash (Android). For a Share location none of that is possible. On the desktop the 'Papierkorb' button is disabled for `is_remote` reports, and trash_reclaim_selected refuses with 'Remote-Reclaim ist in diesem Release read-only.' prepare_reclaim_trash_plan also skips every remote path. On Android, DuplicatesViewModel calls fs.delete without `permanent`. delete.rs only allows a non-permanent remote delete when backend.delete_disposition() == Recycle, and the Share AgentBackend inherits the default Permanent. The answer is 'unsupported', which shows the 'Nur Anzeige' card. The Share protocol has no host-side trash or move-to-trash request (the FsRequest variants only include the permanent RemoveFile and RemoveDir). The result of a remote duplicate search therefore never matches what the same host offers locally.

**Fehlerszenario.** The phone finds 30 GB of duplicate videos on the PC's share and the user selects 'Kopien automatisch auswählen' → 'In den Papierkorb'. The answer is 'Dieser Ort hat keinen Papierkorb – die Duplikate werden nur angezeigt'. On the desktop GUI the 'Papierkorb' button for a Share reclaim report stays disabled. The user has to go to the PC and repeat the search there.

**Richtung.** Add a Share request that the host executes against its own trash: Windows recycle bin, freedesktop trash, or the Android app trash via apptrash. Have the host re-verify duplicate content (bytes or SHA-256) before moving the copy. Advertise it as a capability and set delete_disposition = Recycle for peers that support it. Both reclaim UIs can then reuse their local trash flow, keeping the plan, confirmation and skipped-path reporting.

**Gegenprüfung.** The facts hold. Desktop: reclaim_core.rs:170-173 refuses remote reports, reclaim_ui.rs:226-238 disables the button for is_remote, and verify.rs:20-24 skips every remote path. Android: DuplicatesViewModel.kt:160-167 calls fs.delete with permanent=false. delete.rs:57-63 answers `unsupported` unless delete_disposition() is Recycle. AgentBackend has no override, CachingBackend forwards (cache.rs:427-429) and the default is Permanent (core.rs:238-240), so the screen shows 'Nur Anzeige' (DuplicatesScreen.kt:221-222). The Share protocol has only the permanent RemoveFile and RemoveDir (wire.rs:302-372). Severity: this is a deliberate limitation stated in both UIs. Desktop says 'Remote-Reclaim ist in diesem Release read-only'; Android shows the hint at DuplicatesScreen.kt:76. On desktop it applies to every remote backend, not just Share. Nothing is lost or misreported, and on Android permanent deletion remains possible from the explorer. Closing the gap needs a protocol extension (a host trash request plus a capability flag), which is feature work. Low, not medium.

## A35 Opening a folder from a remote analysis result drops the Share location's endpoint prefix

- Schwere: low · Urteil: confirmed · Kategorie: bug · Plattformen: Windows, Linux
- Stellen: native/src/app/core/analytics_core.rs:310-326; native/src/app/core/app_models.rs:193-197; native/src/app/core/picker_impl.rs:352-360; native/src/app/core/sync_core.rs:222-239; native/src/app/core/share_drain.rs:45

**Beschreibung.** StorageScanSource::Remote stores only backend, root and label; it keeps no endpoint prefix. navigate_storage_source ('📂 Im Explorer öffnen', treemap file reveal, reclaim reveal) reuses the explorer's RemoteState only if it holds the identical Arc backend. Otherwise it builds a new RemoteState with endpoint_prefix: None, account: None, sftp: None and the analysis label. An analysis started through the folder picker always has a different backend than the explorer, so this happens every time in that flow; it also happens after the explorer has moved to another location. For context-menu analyses the label is the folder path itself. The resulting pane lost its persistent location identity:
- pane_endpoint fails with 'Diese Sitzung hat keine dauerhaft wiederherstellbare Adresse'.
- The folder picker hides the pane as a sync source or target.
- location_key falls back to the local namespace, so per-folder preferences collide with a local path of the same name.
- share_target_is_open no longer recognises the share.
AGENTS.md treats Explorer locations as a contract that must keep each remote path's prefix.

**Fehlerszenario.** On the desktop the user picks 'Ordner…' in the analysis window, chooses a Direct peer's '/Fotos', analyses it, clicks a large folder and presses '📂 Im Explorer öffnen'. The explorer shows the share folder labelled with the picker label. 'Synchronisation einrichten' from that pane then fails, and the pane is missing from the sync folder picker. The sort order saved for it also applies to the local '/Fotos'.

**Richtung.** Carry the endpoint prefix (and the other RemoteState identity fields) in StorageScanSource::Remote, filled from RemoteState or the picker's endpoint_prefix. Reconstruct the full RemoteState in navigate_storage_source, or reuse the explorer's state when the prefixes match, not only when the Arc is identical. Use the connection label, not the path, as the context-menu analysis label.

**Gegenprüfung.** StorageScanSource::Remote stores only backend, root and label (app_models.rs:193-197). navigate_storage_source keeps the explorer's RemoteState only when Arc::ptr_eq holds. Otherwise it builds a new one with endpoint_prefix, account and sftp all None (analytics_core.rs:310-326). The picker flow always differs. A peer picked in the picker gets its own backend. Even the explorer's own tab is handed over through pane_backend -> sync_backend, which unwraps the CachingBackend the explorer stores (share_drain.rs:45, sync_core.rs:222-239, sync_roots.rs:6-8). picker_impl.rs:352-360 passes conn_label but not picker.endpoint_prefix. Context-menu analyses pass the explorer's Arc (remote_context_menu.rs:310-315) and lose the prefix only after the explorer has moved elsewhere; the label is then the bare path. Consequences checked: pane_endpoint errors (picker_locations.rs:21-24), the pane is hidden from the sync picker (:52), location_key has no prefix (prefs_tabs.rs:87-94, removal_scope.rs:200-205), and share_target_is_open returns false (share.rs:303-312). Callers are analytics_ui.rs:391-398 and reclaim_ui.rs:399-406. The collision with a local path is mainly a Linux concern, since Windows local keys carry a drive letter. Low is right.

## A36 Analysing a peer's root '/' counts and walks nested or duplicated exports twice

- Schwere: low · Urteil: confirmed · Kategorie: bug · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/os/shared/storage_analysis_host.rs:32-36; native/src/share/os/shared/storage_analysis_host.rs:132-192; native/src/share/core/fs.rs:248-266; native/src/mobile/os/shared/domains/share_peers.rs:196-200; native/src/cli/share/exports.rs:76-82; native/src/app/core/share_exports_ui.rs:148; native/src/app/core/share_exports_ui.rs:164

**Beschreibung.** Export configuration rejects only exact duplicate path strings (share_peers.rs:198, exports.rs:78). Nested exports such as '/storage/emulated/0' plus '/storage/emulated/0/DCIM' are accepted, and so is the same folder reached through a different spelling. local_mounts maps each root to its own mount without any containment check. The host's synthetic root analysis (scan_container) walks every export independently, each with its own worker pool and budget, and adds every export's size into the root total and the shared counters. The overlapping subtree is therefore read twice and its bytes are counted twice. The host's own local analysis of the same storage counts every byte once.

**Fehlerszenario.** An Android phone shares 'Interner Speicher' (/storage/emulated/0) and 'Kamera' (/storage/emulated/0/DCIM, 40 GB). The PC analyses the phone's root. The scan reads DCIM twice and reports about 40 GB more than the phone actually uses, and the treemap percentages are skewed accordingly.

**Richtung.** In scan_container, resolve every export to its canonical root before walking. Walk a root once and do not re-walk an export contained in another. Either present the nested export as a view of the already-measured subtree, or exclude its bytes from the root total and add a note. Optionally warn about nested exports when one is added.

**Gegenprüfung.** All three add paths reject only an identical path string. Mobile (share_peers.rs:196-200) and CLI (exports.rs:76-82) compare after canonicalize. The desktop GUI does not canonicalize at all (share_exports_ui.rs:148, :164). local_mounts (fs.rs:248-266) gives every root its own mount with no containment check. For '/' with Dynamic access the host runs scan_container (storage_analysis_host.rs:32-36, 132-192). It scans each export independently (:155) and adds every subtree into the root total (:183-188), so a nested export is read and counted twice. The Android listing walk double-counts the same way, because list_dir('/') returns the same export names. The 'different spelling' part applies only to the desktop GUI, since mobile and CLI canonicalize. The cited line numbers are wrong: the file has 219 lines and scan_container is at 132-192.

## A37 Host-side analysis of an exported SSH connection always tells the user to update both devices

- Schwere: low · Urteil: partially_confirmed · Kategorie: bug · Plattformen: Windows, Linux, Android
- Stellen: native/src/analytics/os/shared/remote.rs:18-32; native/src/agent/core/backend.rs:182-200; native/src/vfs/core/core.rs:389-396; native/src/share/os/shared/storage_analysis_host.rs:39-47; native/src/mobile/os/shared/domains/analyze.rs:207-209

**Beschreibung.** For connections exported under 'Verbindungen', the host runs scan_remote on the connection backend (storage_analysis_host.rs:166-170). For an SSH-agent connection, AgentBackend::scan_storage delegates to its inner SFTP backend, which inherits the default Ok(None). scan_remote then always takes the walk_tree path, sets ScanPhase::Legacy and appends the note 'Die Gegenstelle nutzt den älteren Analysepfad; für den lokalen Worker und vollständige Fortschrittsmeldungen beide Geräte aktualisieren.' The host ships this note to the Share client. It blames the 'Gegenstelle' and asks the user to update both Smart Explorer devices, although both are current and no newer path exists for SSH agents. The same note appears on every direct SSH-agent analysis.

**Fehlerszenario.** A PC shares its saved SFTP server under 'Verbindungen'. The phone analyses '/Verbindungen/nas', and the result carries the update note although both devices run v0.5.169. The user updates nothing and keeps seeing it.

**Richtung.** Emit the 'update both devices' note only when a Share peer answered Capabilities without storage_analysis_v2 (the Ok(None) branch of peer_storage_analysis::scan). Give the agent walk path an accurate note, for example that the SSH agent reports no folder counts, or none at all.

**Gegenprüfung.** The code path is real, at different lines. remote.rs has 40 lines, not 107-123. When scan_storage returns Ok(None) and supports_walk_tree() is true, it sets ScanPhase::Legacy (:18-21). After the walk it appends the 'beide Geräte aktualisieren' note if the phase is still Legacy (:27-32). AgentBackend reports supports_walk_tree() true and forwards scan_storage to its inner backend (agent/core/backend.rs:182-200). deploy_over_sftp makes the SFTP backend that inner backend (deploy.rs:106-182), and SFTP keeps the default Ok(None) (core.rs:389-396; CachingBackend forwards at cache.rs:432-447). The host analyses an exported connection through scan_remote (storage_analysis_host.rs:39-47, not 163-171) and ships the notes (MAX_NOTES :8). So a desktop client analysing a peer's /Verbindungen/<ssh-agent> sees the misleading note, and so does every direct desktop SSH-agent analysis (analytics_core.rs:423-426). The stated failure scenario is wrong, though. Android remote analysis runs scan_backend, a ListDir walk (analyze.rs:207-209), and never calls scan_remote or the host's StorageAnalysis. The phone therefore never receives the host's note, and a direct Android SSH analysis never produces it.

## Offene Fragen der Prüfer

- [client-routing] Whether hosts that predate FsRequest::Capabilities can still connect: if they can, PeerBackend::scan_storage and walk_peer both fail on the capability probe instead of falling back to the legacy WalkTree stream (peer_storage_analysis.rs:10-22, peer_storage_snapshot.rs:25-41).
- [client-routing] Exact Android behaviour when a Share host lists <volume>/Android/data|obb (EACCES vs. empty listing) - this decides whether the Android listing path shows read errors or silently missing protected notices.
- [client-routing] Magnitude of the per-directory guard overhead on the host (Windows NTFS, SMB/UNC exports, Android FUSE) was not measured.
- [client-routing] Whether every Windows Share mount gets a drive letter that shows up in the analysis drive row, and how Linux FUSE mount paths are offered.
- [client-routing] The '~100k entries per 16 MiB ListDir reply' threshold is an estimate based on JSON FsMeta size; not measured.
- [client-routing] Android memory figures for large remote trees (listing map plus tree, 4 retained results, up to 12 M nodes via v2) are estimates; not measured on a device.
- [client-routing] Whether daemon request credits or AgentPool::single limit how many ListDir requests the Android listing walk really keeps in flight (affects only how slow the current path is, not the finding).
- [client-routing] Whether OS error texts in host issues/notes can still contain host physical paths that remap_issues does not rewrite (would leak user names to peers); not substantiated from the code read.
- [client-routing] #0 severity (high vs critical) depends on how much weight metered mobile data and the project's own relay bandwidth get; the code itself shows no data loss or security impact.
- [client-routing] #7: the actual per-directory overhead of the Share guard (symlink_metadata + canonicalize) is unmeasured; it needs timing on Windows NTFS with Defender, UNC/SMB exports and Android FUSE before deciding between medium and low.
- [client-routing] #1: the exact entry count at which a host ListDir reply exceeds the 16 MiB control frame (estimated about 100k entries from FsMeta JSON size), and how PeerBackend retries plus the 20 s IPC metadata timeout surface that error, were not traced end to end.
- [client-routing] #5: whether an Android host's Android/data and Android/obb ListDir fails (EACCES) or returns empty depends on the Android version and app permissions; this cannot be decided from code.
- [client-routing] #10: how often the pooled IPC AgentBackend actually dies on Android (heartbeat after the app is frozen, embedded worker restart) is a runtime question.
- [client-routing] #9: the probability of the SSH agent emitter race (no Progress frame for very fast walks, server.rs:91-103) was inferred from thread scheduling, not observed.
- [host-execution] Peer drive mounts (native/src/mount): if a user analyses a mounted peer drive or folder as a local path, the GUI walks it through the mount instead of asking the host; how mounts present and whether the analysis picker offers them was not verified (outside the read scope).
- [host-execution] OS scheduling of the host: no priority or power-throttling settings exist in the code (grep for SetPriorityClass, PROCESS_POWER_THROTTLING, ioprio, nice found nothing). Whether Windows power throttling/EcoQoS slows the window-less daemon, or Android's cpuset placement of a backgrounded host app slows host-side analyses compared with a foreground local run, cannot be determined from the repository.
- [host-execution] Sync dimension, needs routing: bisync's hashed snapshot against Share targets uses the same daemon WalkHashed emulation (daemon/os/shared/backend_walk.rs:306-395 via bisync/os/shared/snapshot.rs:178-183): HashMode::Full downloads every file, and even without hashing the 'fast path' is a serial stat+list per directory instead of the parallel per-directory walk.
- [host-execution] Whether the Android TaskForegroundService/TaskKeeper keeps the phone client process alive and unfrozen for long remote analyses was not checked (out of scope); it affects the disconnect finding.
- [host-execution] All durations, slowdown factors and wire-size ratios are estimates from the code paths; nothing was built, run or measured. The single remote task suite should measure local vs. remote analysis time and bytes on the wire per platform.
- [host-execution] Exact drop semantics of the iroh/noq SendStream when the host fails to send an oversized Entries frame (implicit finish vs. reset) were not verified; either way the client receives an error instead of a listing.
- [host-execution] The 2026-09-28 lesung states QUIC flow-control windows are not set; current keepalive.rs:18-25, 85-117 now sets 16 MiB per stream and up to 64 MiB per connection, so per-stream windows are not a bottleneck for the single result stream (no finding raised).
- [host-execution] What Android's FUSE layer actually returns when another app's Android/data/<pkg> folder is listed or opened cannot be checked from the repository. The code's own comment (apptrash/mod.rs:46-50) says Android lists the package folders but refuses to open them. This decides how many issues #0(a) produces and whether #1 scenario 2 occurs.
- [host-execution] When an oversized ListDir reply fails, the host's SendStream is dropped. Whether iroh/noq then finishes or resets the stream decides the client's exact error text (EOF 'closed after 0 bytes' or a reset). Either way the request fails (#9).
- [host-execution] All durations and throughput figures in #0, #2, #3 and #4 are estimates. Nothing was built, run or measured.
- [host-execution] #12: the daemon is normally non-elevated because it starts via the HKCU Run key. It may run elevated if an elevated GUI spawns it; that case was not traced.
- [host-execution] #1: the daemon's single WalkHashed handler (backend_walk.rs:306-378) also serves bisync. What that means for HashMode::Full syncs against Share targets was not reviewed (sync dimension, out of scope).
- [host-execution] Android reaches a non-Share remote analysis (SFTP etc.) the same way, calling scan_backend directly and bypassing scan_remote's agent walk_tree. This is outside the reviewed Share area and was not assessed.
- [critic] Windows background power throttling (EcoQoS / process power throttling) of the windowless daemon on battery could make the host-run analysis slower than the GUI's foreground local run. The code has no PROCESS_POWER_THROTTLING opt-out (grep for SetProcessInformation/PowerThrottling found nothing), but the effect cannot be confirmed statically; a battery-powered measurement is needed.
- [critic] How often the signaling server emits DirectOffline/DirectAvailable for connected peers in practice (idle keepalive, network handover, laptop sleep) was not traced into share-server/; this sets how often the high-severity session invalidation fires.
- [critic] Not fully traced: whether ipc_host::open_share → reload_now (called for every AnalyzeShare/OpenShare) can apply a pending profile change and so trigger the global invalidation just as a second analysis starts.
- [critic] Android suspend timing after screen-off (whether the device sleeps inside the 30-s renewal window) depends on the device and was not measured.
- [critic] The host user is never shown that a peer is running an analysis or a hash walk on the host (no ShareEvent for incoming operations), including on Android over metered networks. This is a design and consent question, not reported as a defect.
- [critic] The SSH agent tree walk used for exported SSH connections (agent_proto/os/shared/fs.rs walk_dir_counted) aborts on the first unreadable entry. That belongs to the SSH area and was not reported here.
- [critic] Not examined: iroh/quinn behaviour across device suspend (whether the connection survives while only the stream deadline expires) and the share-server presence fan-out rules.
- [critic] #1/#3: whether an Android device actually suspends, and for how long, during the 30 s hold-renewal gap or during a client task is device-dependent. Incoming QUIC keepalives (keepalive.rs:5, every 5 s) may wake the Wi-Fi path, so the code alone cannot decide between a 60 s timeout and a slow crawl.
- [critic] #0: I did not trace whether mounted Share drives and leased sync writes re-acquire their lease automatically after invalidate_sessions clears the host lease table (node.rs:302). mount_manager's remount logic was outside the read scope.
- [critic] Out-of-list, relevant to the user's complaint #1: Android remote analysis calls scan_backend, a ListDir walk (mobile/os/shared/domains/analyze.rs:207-209), instead of scan_remote / StorageAnalysis v2. A phone analysing a PC therefore never uses the host-side worker that desktop clients use. This structural difference, not any of the eight findings, is the main reason phone->PC analysis is slower and behaves differently from a local analysis (also noted in docs/lesungen/2026-10-01-android-storage-analysis-path.md:243).
- [critic] Out-of-list: the Share duplicate search on Android hashes every file regardless of the minimum size (walk_hashed(root, true, ...), reclaim/backend.rs:113). It streams the full content through the phone's embedded daemon in one serial walk (backend_walk.rs:306-376), because peer list entries carry content_md5 None (share/core/backend.rs From<FsMeta>). It also aborts on the first list or read error. This is a performance and reliability concern beyond #3.
