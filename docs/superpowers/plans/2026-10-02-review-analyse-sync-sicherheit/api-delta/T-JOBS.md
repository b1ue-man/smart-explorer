# T-JOBS API-Übergabe

Stand 2026-10-03; eigener Scheduler-/Watch-Block statisch abgeschlossen, Remote-Abnahme ausstehend.

- `daemon::run_guardian` erhält den Dienstlebenszyklus; `--sync-guardian` ist ausschließlich als einziges exaktes Argument zulässig. Bestehendes `--sync-daemon` bleibt erhalten.
- `syncjobs::JobState`, `classify_run`, `record_attempt`, `confirm_block` ersetzen Konfigurationszeitstempel als Laufwahrheit. Erfolgreiche Versuche, Abbruch, Fehler und Sicherheitsstopp bleiben getrennt; laufende Konfiguration wird frisch geladen.
- `daemon::set_storage_access`, `set_problem_notifier`, `next_scheduled_run`, `last_catch_up` und `CatchUpStatus.retry_suggested` verbinden Android/GUI. Hook-Aufruf über `run_job_hook` und RAII `keep_awake::hold` bleibt vor/nach der Engine wirksam.
- `watch::watch_confined(&DirectoryHandle, &Path, WatchOptions, WatchFilter, WatchSink)` hält den Pin während der Sitzung. Linux/Android melden direkte Kindhinweise und Teilabdeckung; Windows fehlende sichere Abdeckung bleibt Unsupported. Normale lokale Job-Watches bleiben rekursiv.
- `connect::local_endpoint_path` erhält Locatorsemantik ohne Netzöffnung. `bisync::current_content_signature` bestätigt eigene Schreibereignisse mit sicherem Lesen, Locator, Seite, Generation und erfolgreichem Zustand; ohne Beweis bleibt der Folgelauf offen.
- Persistierte offene Generationen, Entprellung/Höchstwartezeit, Überlauf-/Neustartkontrolle und konservative MediaStore-Cursor erhalten Hybrid-Abfragen.

Konkrete Desktop-/Android-Consumer sind Anschlussblöcke. Details und Fälle: [Abnahme](../abnahme/T-JOBS.md), [Anfragen](../anfragen/T-JOBS.md).
