# Anfragen T-JOBS

Stand: 2026-10-02 (Vertrag V4 eingetragen). Je Anfrage: Datei/Stelle, Änderung, Grund. Bis zur Erledigung
arbeitet T-JOBS mit lokalen Platzhaltern weiter.

## An K3 → E-PLAN

1. **Tolerantes Laden** – `syncjobs/os/shared/persistence.rs`: zusätzlich
   `pub fn load_report() -> io::Result<JobLoadReport>` mit `pub struct JobLoadReport { pub jobs: Vec<SyncJob>,
   pub broken: Vec<BrokenJob> }` und `pub struct BrokenJob { pub id: String, pub path: PathBuf,
   pub error: String }`; jede `.conf` einzeln geladen, `load()` bleibt wie es ist. Grund: Y20/FS9 „eine kaputte
   Job-Datei stoppt nur sich“ – der Dienst plant die gültigen Jobs und trägt bei den kaputten
   `FailureKind::Config` in den Job-Zustand ein.
2. **Job löschen** – `persistence::remove(id)`: zusätzlich `super::job_state_store::remove_job_state(id)`
   aufrufen (fehlender Zustand ist kein Fehler). Grund: Laufzeitdaten gelöschter Jobs bleiben sonst liegen
   (Y150); ein einziger Löschweg für Desktop, Android und CLI.
3. **Aufräum-Befehl** – `syncjobs/core/types.rs`, `persistence_codec.rs`, `editor.rs`, `validation.rs`:
   `SyncJob` additiv `pub run_cleanup: String` (Schlüssel `run_cleanup`, Standard leer), behandelt wie
   `run_after`. Grund: FS12/Y146 „nach Abbruch gibt es einen Aufräum-Befehl“;
   `daemon::run_job_hook(.., HookPhase::Cleanup, ..)` liest das Feld.
4. **Sicherheitsstopp typisiert** – `bisync::Outcome` und `BisyncOptions` (V3): Stopp als eigener Wert statt
   Fehler „abgebrochen“ (Y69) mit Art (Massenlöschung mit Seite, Anzahl, Gesamtzahl; Seite leer;
   Replika-Markierung fehlt) und eine einmalige Freigabe genau dieses Stopps in den Optionen. Typnamen bitte
   unter V3 eintragen; T-JOBS bildet sie auf `syncjobs::BlockKind` ab (`JobSide::A` = Quelle, `B` = Ziel) und
   setzt die Freigabe nach `confirm_block`.
5. **Erledigte Aktionen für den Aufrufer** – `bisync::run` (V3 `CompletedAction{rel, kind, src_sig, dst_sig,
   durable}`): ein Weg, auf dem der Aufrufer die laufend gemeldeten Aktionen mitbekommt (Beobachter-Parameter
   oder Kanal in den Optionen). Grund: B22 – eigene Schreibvorgänge lösen keinen Echtzeit-Lauf aus, solange der
   Zustand dem Geschriebenen entspricht (Seite + `rel` + `dst_sig`).
6. **Stabil lassen** – `syncjobs/os/shared/persistence.rs`: `atomic_write`, `read_regular_utf8`, `job_file`,
   `jobs_dir`, `load_job_file`, `san_id` (`pub(super)`) und das Feld `SyncJob::last_run` (Startwert für
   fehlende Zustände) nutzt `job_state_store`.
7. **V3-Felder** – T-JOBS liest `rt_max_latency_secs`, `rt_poll_secs`, `verify_interval_secs`,
   `verify_target_secs`, `cross_mounts`, `config_version` mit genau diesen Namen. Die Tiefe eines Laufs
   (Ziel vollständig listen oder Basis + Replika-Prüfung) entscheidet E-PLAN selbst anhand
   `verify_target_secs`; T-JOBS löst nur Läufe aus. Falls E-PLAN die Lauf-Ursache als Hinweis braucht: bitte
   eine Option nennen (T-JOBS kennt sie als `syncjobs::RunCause`).

## An V-REMOTE

8. **Fehlerart beim Öffnen** – `native/src/connect/**`: Einordnung, warum `resolve_endpoint` scheiterte, z. B.
   `pub fn resolve_endpoint_classified(endpoint: &str) -> Result<(BackendHandle, String), ConnectFailure>` mit
   `ConnectFailure { kind: ConnectFailureKind { Unreachable, Auth, Missing, Config, Other }, message: String }`
   (oder eine Einordnungsfunktion für den bisherigen Fehlertext). Grund: Y119/FS9 – Anmeldefehler werden nicht
   automatisch wiederholt (Kontosperren, fail2ban), Erreichbarkeitsfehler mit Backoff. Bis dahin ordnet T-JOBS
   die Texte heuristisch ein.
9. **`change_signal` (V1)** – T-JOBS nutzt den Haken für Fernseiten von Echtzeit-Jobs (Abfrage alle
   `rt_poll_secs` mit billigem Signal); bitte die Signatur unter V1 eintragen.

## An D-SYNCUI (Welle 3)

10. Job-Liste und Einstellungen aus dem Job-Zustand: letzter Erfolg/Versuch/Grund (`last_success`,
    `last_attempt`, `last_error`, `consecutive_failures`, `JobState::problem()`), Sperre mit „Prüfen…“ und
    „Trotzdem ausführen“ (`confirm_block`), Überwachungsart aus `watch` („Ereignisse“, „Abfrage alle N min“),
    laufend aus `running_now`. `JobState`-Liste zwischenspeichern (nicht je Bild neu laden).
11. Manuelle Läufe: `syncjobs::classify_run` + `record_attempt` (`Runner::Desktop`) statt `mark_run` +
    `record_result` (abgebrochene Läufe zählen dann nicht als Lauf, Fehler mit echter Anzahl);
    `keep_awake::hold(Reason::SyncRun)` für die Laufdauer; Befehle über `daemon::run_job_hook` (`Before` vor
    dem Öffnen der Seiten, `After`/`Cleanup` danach) – den Hinweis „nur Hintergrund-Dienst“ dann anpassen.
12. Einstellungen: `autostart::disabled_by_system()` sichtbar machen („Autostart ist in Windows
    abgeschaltet“), Auto-Pause-Schalter nur laut `daemon::autopause_support()`.

## An AND-SYNC (Welle 3)

13. `sync.run`: `syncjobs::classify_run` + `record_attempt` (`Runner::Android`) statt `mark_run` +
    `record_result`, dazu `keep_awake::hold(Reason::SyncRun)`; Job-Karte aus `JobState` wie D-SYNCUI.
14. `daemon::set_problem_notifier` installieren → Ereignis für den Kanal „Sync-Probleme“ (Titel/Text aus
    `ProblemNotice`, Tippen öffnet den Job).
15. MediaStore-Beobachter → `watch::report_host_change(&[volume_root])` (oder genauere Pfade);
    Generationen → `watch::set_host_cursor(volume_root, Some("<version>:<generation>"))`; Allzugriff →
    `daemon::set_storage_access(granted)` (bei jedem `sys.hostState`).
16. Exakte Alarme aus `daemon::next_scheduled_run(now)`; Worker: `CatchUpStatus.retry_suggested` →
    `Result.retry()`; „Letzter Hintergrundlauf“ aus `daemon::last_catch_up()` (mit Ergebnis statt nur Zeit).

## An H-ANALYSIS, H-DISPATCH, A-CLIENT

17. H-ANALYSIS: `watch_v1` über `crate::watch::watch` je Freigabe (eigener Filter: `.se-versions`,
    Zwischendateien); Generation erhöhen bei `Change`, `Overflow` und `Ready` nach `Unavailable`.
18. H-DISPATCH: `keep_awake::hold(Reason::PeerService)` solange fremde Analyse-/Übertragungsströme laufen
    (Desktop; Android ergänzend zum vorhandenen Stream-Halten).
19. A-CLIENT (optional): `keep_awake::hold(Reason::RemoteTask)` für Analyse-/Duplikat-Tasks gegen ein anderes
    Gerät.

## An den Orchestrator

20. `native/src/app/core/landing.rs` (ohne Besitzer) zeigt `job.last_run` und `load_results()`: auf
    `JobState` umstellen – Vorschlag: D-SYNCUI zuteilen.
21. Neue T-JOBS-Dateien außerhalb der wörtlichen Liste: `syncjobs/os/{linux_os,windows}/job_state_lock.rs`
    (Datei-Sperre: `flock` bzw. `LockFileEx`; `os/shared` bleibt ohne OS-Importe) und im Daemon
    `daemon/os/shared/{problem_notify,hooks}.rs` (angelegt) sowie weitere `daemon/os/shared/*.rs` für die
    Aufteilung von `run_loop.rs` (Echtzeit, Anschluss, Kontroll-Läufe, Wächter; je < 500 Zeilen). Bitte
    bestätigen.
22. Plan-Unstimmigkeit: spec „Plattform-Prüfung“ und recherche E11 nennen für Linux noch `systemd-inhibit` als
    Kindprozess; gültig ist B10/V4 (logind-Inhibit über `zbus`). Bitte bei Gelegenheit angleichen.
