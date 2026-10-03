# D-SYNCUI – API-Delta

Stand: 2026-10-03. Desktop-Consumer, keine neue Engine-/Draht-API.
Exaktes Dateiinventar, Entscheidungen und Test-/Remote-Signale:
[Abnahme](../abnahme/D-SYNCUI.md). Fremdanschlüsse: [Anfragen](../anfragen/D-SYNCUI.md).

## Eigene additive Registrierungen

- `native/src/app/core/sync_run_state.rs` → `app::sync_run_state`
- `native/src/app/os/shared/sync_manual_run.rs` → `app::sync_manual_run`
- `native/src/app/core/sync_merge_types.rs` → `app::sync_merge_types`
- `native/src/app/os/shared/sync_merge_task.rs` → `app::sync_merge_task`
- `native/src/app/core/sync_preview_types.rs` → `app::sync_preview_types`
- `native/src/app/core/sync_job_state_ui.rs` → `app::sync_job_state_ui`
- `native/src/app/core/sync_versions_ui.rs` → `app::sync_versions_ui`
- `native/src/app/os/shared/sync_versions_task.rs` → `app::sync_versions_task`

## Kleine interne UI-Verträge

Eigene Oberflächen sind `pub(in crate::app)` oder privat:

```rust
App::desktop_sync_active(&self) -> bool;
App::cancel_desktop_sync(&mut self);
App::drain_desktop_sync_workers(&mut self) -> usize; // tatsächlich lebende Worker
App::track_desktop_sync_worker(&mut self, JoinHandle<()>, Arc<AtomicBool>);
App::desktop_job_busy(&self, id: &str) -> bool;
App::start_saved_desktop_run(&mut self, id: &str, Option<JobConfirmation>);
JobConfirmation { kind: BlockKind, source: String, target: String }
App::open_sync_versions(&mut self, id: &str);
App::ui_sync_versions(&mut self, &egui::Context);
App::submit_merge(&mut self, MergeUi, Option<MergeDecision>);
App::cancel_merge(&mut self);
```

RunMailbox hält BisyncCtx, Pendings und Persistenz-/Vorbereitungsfehler. App hält
DesktopRun, SyncWorker-Handles, Mirror-Wake-Hold und SyncVersionsUi.
BisyncCtx ergänzt `state: Option<StateKey>` und `job_id: Option<String>`;
vorhandene Locator-/Backendfelder bleiben erhalten.

MergeUi kommt als Reexport aus dem eigenen Typmodul. MergeSession hält Backends,
Roots, echten Key/Conflict und optionale rohe Originalbytes. MergeDecision hält
Write-Bytes, im Worker zu assemblierende Rows oder KeepBoth-Seite.
MergeLoadRx liefert `Result<MergeUi,String>`; MergeApplyRx liefert
`MergeApplyResult { ui, result: Result<MergeReport,MergeFailure> }`.
Fehler geben dieselbe Originalsession zurück; Restart nutzt RecordedMergeChoice.

apply_one_rx liefert `PreviewApplyResult { preview: Preview, action: Action,
result: Result<String,String> }`: ursprüngliche vollständige Preview wird bewegt
und zurückgegeben. Keine Rekonstruktion von planned/state.

VersionIdentity bindet Quell-/Ziellocators, aufgelöste Roots, Pair-/Lock-ID.
VersionSnapshot trennt Restore-Erfolg vom nachfolgenden Listenfehler.
VersionTask teilt Cancelmarker/JoinHandle mit dem Tracking; explizite Entry-/Seitenwahl.
Restore ausschließlich über die bestehende Engine.

## Konsumierte bestehende Verträge

- `bisync::{RunRequest::new, run_with, RunSettings::for_job, Outcome.state}`.
- `syncjobs::{load_job_states, update_job_state, classify_run, classify_failure,
  record_attempt, confirm_block, block_confirmation}`.
- `daemon::run_job_hook`, `HookPhase::{Before,After,Cleanup}`,
  `keep_awake::hold(Reason::SyncRun)`.
- `bisync::{preview_with, apply_preview_action, resolve_recorded,
  recorded_original_paths_for_key, merge_recorded_for_key,
  pending_merge_for_key, pending_merge_relatives}`.
- `bisync::versions::{list_versions, restore_version, VersionEntry, VersionSide}`,
  `PairLock::acquire` mit echten Endpointidentitäten.
- `vfs::open_read_regular(&dyn Backend, path, None)` für begrenzte Raw-Reads;
  `linemerge::{rows, assemble_rows, TextShape}`.
- `SyncHandle::take_worker` (E-APPLY-Handoff), vorhandene `daemon/autostart`-
  Status-/Kontrollfunktionen.

Picker/Endpointresolver behalten lokal/UNC/SSH/Agent/FTP/FTPS/WebDAV/Drive/Direct/Room.
Kein offener API-/Registrierungsanschluss; Compiler-/Laufbeweise bleiben zentral.

