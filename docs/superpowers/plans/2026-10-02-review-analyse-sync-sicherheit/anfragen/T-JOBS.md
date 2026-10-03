# Anfragen T-JOBS

Stand: 2026-10-03. T-JOBS ist im Arbeitsbaum umgesetzt; die folgenden
Zuständigkeitsgrenzen dokumentieren die gemeinsame Integration. Quelle für
Umsetzung, Dateien und Abnahmesignale: [abnahme/T-JOBS.md](../abnahme/T-JOBS.md).
Fremde Umsetzung wurde nur an ausdrücklich freigegebenen Grenzen gelesen.

Status: 1–7 sind über V3 integriert; 9 wird über V1 einschließlich ReadyPartial
konsumiert. 17 ist am freigegebenen Host-Caller auf watch_confined umgestellt.
21 ist durch den erweiterten Scope freigegeben. 22 wurde mit dem Hauptagenten
auf den aktuellen logind-Vertrag abgestimmt. 8 bleibt mit konservativer
Textklassifikation überbrückt; 10–16/18–20 gehören zu den jeweiligen
UI-/Host-Blöcken. Keine dieser Grenzen rechtfertigt einen weiteren lokalen
Build, Test, Kritiker oder Graph-Neubau.

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

17. H-ANALYSIS: `watch_v1` über `crate::watch::watch_confined` am bereits
    autorisierten `DirectoryHandle` (siehe 26), mit eigenem Filter für
    `.se-versions`/Zwischendateien. Generation erhöhen bei `Change`, `Overflow`
    und `Ready` nach `Unavailable`; `complete=false` bleibt beim Client hybrid.
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

## Konkrete gemeinsame Integrationen

23. **Guardian-Einstieg – Hauptagent, umgesetzt gemeldet.**
    `lib::run_gui` und `bin/se.rs` erkennen `--sync-guardian` ausschließlich als
    einziges exaktes Argument und rufen `daemon::run_guardian`; `--sync-daemon`
    bleibt erhalten und tritt ohne `SE_SYNC_GUARDIAN_CHILD` in den Guardian ein.
    T-JOBS registriert `run_guardian` additiv in `daemon/mod.rs`. Main/se wurden
    von T-JOBS nicht geändert; deren Abnahme gehört der Gesamtintegration.
24. **Locator – Hauptagent, integriert und konsumiert.**
    `connect::local_endpoint_path(&str) -> Result<Option<String>, String>`
    verwendet exakt `EndpointSpec::parse`, bewahrt gespeicherte Lokalpfade
    einschließlich historischer Drive-Root-Semantik und liefert für Fernorte
    ohne Netzöffnung `None`. T-JOBS nutzt es für Watch- und Volume-Zuordnung.
25. **Gemeinsame Inhalts-Signatur – Hauptagent/E-APPLY, integriert und konsumiert.**
    `bisync::current_content_signature(&dyn Backend, &str, &AtomicBool) ->
    io::Result<u64>` nutzt den vorhandenen Hash-Encoder und `open_read_regular`.
    Nur ein nachweislich passender erfolgreicher Job-/Seiten-/Locator-/Pfad-/
    Generationseintrag darf ein Ereignis unterdrücken. Hash 0, Ordner und
    unbestätigte/abgebrochene Versuche behalten konservativ einen Folgelauf.
26. **Handlegebundene Watch – V-LOCAL/H-ANALYSIS, verbunden.**
    V-LOCAL liefert `DirectoryHandle::watch_path() -> Option<PathBuf>`:
    Linux/Android `/proc/self/fd/<held-fd>/.`, Windows `None`.
    T-JOBS bietet `watch_confined(&DirectoryHandle, &Path, WatchOptions,
    WatchFilter, WatchSink) -> io::Result<WatchHandle>`; RootSpec/Registry und
    Sink-Zustellung halten eigene Handle-Clones. Der Pfad dient ausschließlich
    Anzeige/Filter, wird nicht erneut kanonisiert/geöffnet. Linux/Android
    beobachtet direkte Childnamen, keine pfadbasierte Rekursion/Extend, keine
    freien Host-Pfadsignale; `Ready(LocalOnly)` verlangt periodische Abfragen.
    Windows liefert `Unsupported`; fehlende Childroots werden bei H-ANALYSIS
    bis zu sicherem handlegebundenem Parent/Literal-Wiederanlauf ausdrücklich
    als Unavailable behandelt. Normale lokale Job-Watches bleiben rekursiv.
    Der ausdrücklich freigegebene `share/os/shared/host_watch.rs` konsumiert
    diesen Einstieg und propagiert Teilabdeckung.
27. **Teilabdeckung V1 – V-LOCAL/H-ANALYSIS, konsumiert.**
    `ChangeNotice::ReadyPartial { generation }` startet Kontrolle und nutzt
    Hinweise, behält aber `EventsAndPoll`. `FsWatchEvent::Ready.complete`
    fehlt bei alten Gegenstellen mit Standard `false`. SMB/FUSE/Android dürfen
    durch eine Ready-Meldung keine notwendige Abfrage verlieren.
28. **Backend-Deadlines – jeweiliger Backend-Besitzer.**
    T-JOBS setzt den Abbruch und wartet höchstens zehn Sekunden auf Worker;
    noch blockierende E/A behält ihre Paarsperre. Keine gefährliche Thread-
    Terminierung oder vorzeitige Sperrfreigabe. Falls eine Backend-Operation
    unbeschränkt blockiert, muss die Deadline dort ergänzt werden; fremde
    Backendimplementierungen wurden nicht außerhalb des Scopes untersucht.
    Exakt gleiche konfigurierte Paare werden zusätzlich bereits im Planer
    vor Hooks/Öffnen serialisiert, auch bei vertauschten Seiten. Für physische
    Aliase und fremde Runner gilt weiterhin die gemeinsame Engine-Paarsperre.
29. **B19 MediaStore-Vorfilter – Entscheidung mit Hauptagenten gemeldet.**
    Der Cursor wird nach erfolgreicher Kontrolle gespeichert. Ein unveränderter
    Cursor schließt Änderungen an Dateien außerhalb der Provider-Abdeckung
    nicht aus; nötige Hybrid-Abfragen und Kontrollen bleiben konservativ aktiv.

## Übergabe

Der eigene Implementierungsblock ist abgeschlossen. Aus T-JOBS verbleibt kein
zusätzlicher lokaler Arbeitsschritt. Hauptagent übernimmt gemeinsame Integration,
Meilenstein-Commit/Push, Root-Graph und die eine abschließende Remote-Task-Suite.
Die übrigen Consumer/Plattform-Abnahmesignale bleiben Bestandteil dieser Suite.
