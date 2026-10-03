# Abnahme T-JOBS

Stand: 2026-10-03. Umsetzung der freigegebenen FS8–FS10/V4 und
B08/B10/B11/B19/B22/B27/B32, ohne neue Review-Runde. Abnahme ausschließlich
in der einen abschließenden Remote-Task-Suite; lokal keine Builds oder Tests.

## Fortsetzungsplan und erwartetes Verhalten

1. Laufweg und Zustand: aktuelle Jobkonfiguration beim Start; Vorbereitung und
   Vorbefehl vor dem Öffnen; jeder Versuch typisiert gespeichert, Abbruch und
   Sicherheitsstopp getrennt; einmalige Freigabe, jobeigene Basis und gemeinsame
   Paarsperre über `run_with`; Wachhalte-RAII. Fehlversuche verbrauchen keinen
   Zeitplan. Anmeldefehler erfordern Nutzeraktion.
2. Echtzeit: lokale Watch-Abos und entfernte VFS-Signalabos, Filter und
   richtungsabhängige Seiten; persistierte offene Änderungen; Entprellung mit
   Höchstwartezeit; Start/Überlauf/Wiederanmeldung lösen Kontroll-Läufe aus.
   Sichtbarer Abfrage-Rückfall führt pfadbewusste Engine-Läufe aus und belastet
   den Planer nicht mit Walks oder Verbindungsaufbau. Eigene Schreibereignisse
   werden erst nach Vergleich mit dem tatsächlich geschriebenen Zustand verworfen.
3. Planer und Laufaufsicht: Zustandsanker statt Konfigurations-`last_run`,
   unabhängige Paare begrenzt parallel, laufende Jobs mit Lebenszeichen und
   erkennbar fehlendem Fortschritt, begrenztes Warten bei Pause/Stop. Offene
   Startup-/Anschluss-Auslöser überleben Pause und Neustart; kalendarische
   Termine nutzen das vorherige Prüfintervall. Nachholen meldet echte Ergebnisse.
4. OS-Grenzen: USB-/SD-/MMC-Erkennung unter Windows, udev/mountinfo unter Linux,
   Pfadbindung an das gefundene Volume; Autostart-Sperren sichtbar; Wächter
   startet nur einen abgestürzten Dienst erneut, Stop/Handoff bleiben erhalten.
   Probleme als gedrosselte Desktop-/Host-Benachrichtigung, Logrotation erhält
   vorherige Daten.
5. Integration und Self-Review: API-Aufrufe gegen vorhandene Refs (inotify,
   RDCW, CM, IOCTL, logind, StartupApproved) und ergänzende Primärquellen;
   Endpunktidentität und geschützte Link-/Mount-Auslassungen bleiben erhalten.
   Neue Rust-Dateien bleiben unter 500 Zeilen. Konkrete Abnahmesignale und
   offene Fremdänderungen werden hier beziehungsweise in `anfragen/T-JOBS.md`
   festgehalten.

## Rechercheergänzung

Geprüft 2026-10-03: Desktop-Notifications `Notify` hat D-Bus-Signatur
`susssasa{sv}i` und liefert `u32`; `expire_timeout=-1` wählt den Serverstandard.
Quelle: [freedesktop-Protokoll](https://specifications.freedesktop.org/notification/latest/protocol.html).
Windows zeigt Hinweise über eine eigene versteckte Fensterinstanz und
`Shell_NotifyIconW` (`NIM_ADD`, `NIM_MODIFY`, `NIM_DELETE`). Quelle:
[Microsoft Shell_NotifyIconW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyiconw).
Unterprozesse werden über `Child::try_wait` auf Ende/Abbruch geprüft und nach
einem Kill eingesammelt. Quelle: [Rust Child](https://doc.rust-lang.org/std/process/struct.Child.html).

## Status

Der zugewiesene Implementierungsblock ist im Arbeitsbaum abgeschlossen.
Bestehende Teiländerungen wurden weitergeführt; eigenes Quellen-Self-Review ist
abgeschlossen. Verhalten wurde noch nicht durch Remote-CI bestätigt.


## Implementierte Meilensteine und Fundzuordnung

| Meilenstein | Befunde | Umsetzung und erwartetes Abnahmesignal |
|---|---|---|
| Betriebssystem-Watches | FS8, Y02–Y08/Y64/Y93, B08/B27 | Eine inotify-Instanz, Verlust/Overflow baut Kernel-Watches neu auf; Windows-RDCW ≤64 KiB im Netz, Freigabe bei Geräteentfernung. Umbenennungen, Ordner und gleich große Edits lösen aus; Watch-Limit/volle Kanäle führen zur Kontrolle/Abfrage; sicheres Entfernen und Wiederanmelden bleiben möglich. |
| Offene Echtzeit-Arbeit | FS8, Y02/Y04/Y05/Y13/Y22, B19 | Persistenz unter Jobsperre, Ruhezeit plus Höchstwartezeit, spätere Generation während laufender Jobs. Neustart kontrolliert; nach Abbruch oder während des Laufs gesetzte Auslöser bleiben offen. VerifySources prüft Quellen; volle Zielkontrolle gehört dem Engine-Vertrag. |
| Fernseiten und Teilabdeckung | FS8, B07/B19 | VFS-Abos außerhalb des Planers. ReadyPartial, SMB/NAS, FUSE und Android behalten Hybrid-Abfragen. Ready gilt nur bei vollständigem Push als volle Ereignisabdeckung. Zwei Fernseiten starten ohne lokalen Walk; verlorene Abos/Überlauf kontrollieren neu, Auth beendet automatische Wiederanmeldung. |
| Eigene Schreibereignisse | FS8/Y22, B22 | Erfolg, Job, Seite, Locator, Generation, Pfad und zentrale Inhalts-Signatur müssen passen. Größe/mtime werden vor/nach dem Hashen verglichen. Abbruch, falsche Identität/Generation, Hash 0 und eine gleich große Änderung mit wiederhergestelltem mtime behalten den Folgelauf. Ordner werden konservativ weiter geprüft. |
| Laufweg und Zustand | FS9, Y09/Y16/Y18/Y19/Y69/Y78/Y79/Y119 | Vorprüfung vor Hook/Öffnen; Before vor beiden Seiten, run_with mit Job-Basis/Paarsperre/Beobachter, passende einmalige Freigabe, After/Cleanup und RAII. Offline/Auth/Hookfehler vor Start werden gespeichert. Auth/Config/Access wiederholen auch mit altem retry_at nicht automatisch. Defekter Zustand wird gesperrt/quarantänisiert. |
| Zeitplan und Laufaufsicht | FS9, Y15–Y21/Y26 | Anker ab Erfolg/Erstellung, Kalender über das vergangene Tickintervall. Queue lädt IDs erneut. Geänderte/gelöschte/deaktivierte Jobs laufen nicht aus alten Kopien. Unabhängige Paare laufen begrenzt parallel; dieselbe ID/Paar läuft nur einmal. Lebenszeichen/fehlender Fortschritt sind sichtbar; Stop/Pause warten begrenzt. |
| Start-/Anschluss-Auslöser | FS9/FS10, Y01/Y10/Y13/Y14/Y28 | Startup persistiert vor Boot-/Anmeldemarker, auch pausiert/deferiert. Linux mountinfo/udev/sysfs, Windows CM + USB/SD/MMC/1394-Bustyp. Volume und komponententreuer gespeicherter Lokalpfad binden Connect. USB-HDD/SSD/SD werden erkannt, leere Kartenleser und fremde Volume-Orte lösen keinen Sync aus. |
| Catch-up-Ergebnisse | FS9, Y12/Y23/Y27 | Fenster übernimmt persistierte Startup-/Connect-Ursachen. Echte Outcomes steuern failed/retry_suggested; Auth braucht Nutzeraktion. Leere, blockierte, abgebrochene/unterbrochene Fenster behalten den letzten Hintergrundlauf; nur vollständig beendete Fenster mit ausgeführten Versuchen ersetzen ihn. |
| Lebenszyklus/Energie/Diagnose | FS9/FS10, Y24/Y25/Y110/Y146/Y151, B10/B11/B32 | Guardian mit bestehendem Singleton/Handoff, normale Stops ohne Neustart. Gedrosselte Desktop-/Host-Hinweise, gesperrte Logrotation mit drei Archiven, sichtbarer Autostart-Disable. Hooks erhalten Job-/Ergebnis-Umgebung, Windows-Quoting und abbrechbare Prozessgruppe/Job-Object. Letzter RAII-Drop gibt OS-Anfragen frei. |
| Handlegebundene Share-Watch | V4/V1, H-ANALYSIS + V-LOCAL | watch_confined hält den autorisierten DirectoryHandle. Linux/Android beobachten nur den Root-FD direkt und melden LocalOnly; keine Kanonisierung, Childwalk/Extend oder freie Host-Pfadsignale. Root-/Ancestor-Swap darf keine fremden Namen liefern. Überlappende lokale Job-Watches behalten Rekursion. Windows und fehlender Childroot bleiben Unavailable. |

Diese Signale gehören zusammen in die eine abschließende Remote-Task-Suite.
Die vorhandenen Checks für Zustandsablage, Scheduler, Watch-Ereignisse,
Laufaufsicht, Catch-up und RAII wurden weitergeführt/ergänzt. Neue konkrete
Quellenfälle stehen in job_supervisor_tests.rs, catch_up_outcome_tests.rs,
own_writes_tests.rs und watch_tests.rs; keiner wurde lokal ausgeführt.

## Entscheidungen und Integrationsgrenzen

- Exakt gleiche konfigurierte Endpunktpaare werden schon vor Hooks/Öffnen
  serialisiert, auch bei vertauschten Seiten. Andere wartende Paare können
  vorbeiziehen; die Queue wird je Tick nur begrenzt besucht. Physische Aliase
  bleiben über die zentrale Engine-Paarsperre geschützt.
- Gespeicherte Locators werden über connect::local_endpoint_path geprüft,
  ohne eigene Präfixzerlegung oder Netzöffnung. Ursprüngliche Locators gehen
  unverändert an den Resolver; Lauf-Ignore-Muster verwenden pair_key_policy.
- Inhaltsbestätigung verwendet ausschließlich bisync::current_content_signature.
  Ohne Beweis bleibt der konservative Folgelauf erhalten.
- MediaStore-Cursor wird nur nach erfolgreicher Kontrolle gespeichert.
  Unveränderter Cursor allein beweist nicht alle Nicht-Mediendateien und
  schaltet nötige Hybrid-Abfragen/Kontrollen nicht aus.
- Linux: idle:sleep/block-weak, kompatibel idle/block mit sichtbarem Hinweis
  auf begrenzte Wirkung; fehlendes logind bleibt sichtbar. Keine handle-*-
  oder shutdown-Sperren und kein inhibitor-Kindprozess.
- Blockierende Backend-E/A kann ein AtomicBool nicht erzwingen. Nach zehn
  Sekunden bleibt der Planer bedienbar, während der Worker Paarsperre und
  Datenverantwortung behält. Backend-Deadlines liegen beim jeweiligen Backend.
- Linux-Hook-Gruppen beenden gewöhnliche Nachkommen. Ein ausdrücklich
  vertrauenswürdig konfigurierter Hook kann mit eigener Sitzung ausbrechen;
  diese Hooks bilden keine Sandbox.
- Die sichere Share-Watch ist bewusst partiell; H-ANALYSIS nutzt
  watch_confined und complete=false, V-LOCAL liefert watch_path. Windows
  kann aus dem synchronen Read-Pin keine sichere Overlapped-Watch ableiten.
- Guardian-GUI/se-Dispatch außerhalb dieses Schreibscopes integriert der
  Hauptagent. Android-/UI-Consumer stehen in anfragen/T-JOBS.md; deren
  Umsetzung wurde außerhalb des Lesescopes nicht untersucht.

## Ergänzende Primärquellen und Self-Review

Abgeglichen 2026-10-03:
[Windows Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects),
[waitid/WNOWAIT](https://man7.org/linux/man-pages/man2/waitid.2.html),
[systemd INHIBITOR_LOCKS](https://github.com/systemd/systemd/blob/main/docs/INHIBITOR_LOCKS.md)
und [windows-sys 0.58.0](https://github.com/microsoft/windows-rs/tree/0.58.0/crates/libs/sys/src/Windows/Win32).
WNOWAIT hält den Linux-Gruppenführer bis zum abschließenden Kill/Reap gegen
PID-Wiederverwendung fest. block-weak wird für root/Sperreigner ignoriert;
idle bezeichnet automatisches Idle-Verhalten.

Eigenes Quellen-Self-Review hat Run-/Outcome-/Beobachter-Verträge,
Locator-/Hash-/DirectoryHandle-Grenzen, alte Auth-Wiederholtermine,
Triggergeneration und Abbruch-/Speicherfehlerverhalten nachgezogen. Alle neuen
und wesentlich bearbeiteten Feature-Rustdateien liegen im Textstand unter
500 Zeilen und 50 KiB. lib.rs erhielt nur additive eigene Moduleinträge.
Native Remote-Abnahme und Root-Graph-Aktualisierung bleiben beim Hauptagenten.

## Bearbeitete Dateien

Bereits versionierte Dateien mit eigenen Änderungen:
```text
native/src/autostart/os/android.rs
native/src/autostart/os/linux_os.rs
native/src/autostart/os/windows.rs
native/src/daemon/os/android/platform.rs
native/src/daemon/os/linux_os/platform.rs
native/src/daemon/os/windows/platform.rs
native/src/daemon/os/shared/boot_marker.rs
native/src/daemon/os/shared/catch_up.rs
native/src/daemon/os/shared/host_state.rs
native/src/daemon/os/shared/job.rs
native/src/daemon/os/shared/job_supervisor.rs
native/src/daemon/os/shared/live.rs
native/src/daemon/os/shared/run_loop.rs
native/src/daemon/os/shared/schedule.rs
native/src/daemon/os/shared/state.rs
native/src/syncjobs/core/schedule.rs
native/src/syncjobs/os/shared/results.rs
```

Eigene additive Registrierungen in gemeinsam verwendeten Dateien:
```text
native/src/daemon/mod.rs
native/src/syncjobs/mod.rs
native/src/lib.rs
```
Fremde Einträge blieben erhalten; die Guardian-Main/se-Eingangsbindung gehört
zur Integration des Hauptagenten.

Neue Featuredateien im Ergebnis, einschließlich weitergeführter Teiländerungen:
```text
native/src/daemon/os/shared/catch_up_types.rs
native/src/daemon/os/shared/catch_up_outcome_tests.rs
native/src/daemon/os/shared/connect_triggers.rs
native/src/daemon/os/shared/due.rs
native/src/daemon/os/shared/guardian.rs
native/src/daemon/os/shared/hooks.rs
native/src/daemon/os/shared/job_supervisor_tests.rs
native/src/daemon/os/shared/job_triggers.rs
native/src/daemon/os/shared/log_store.rs
native/src/daemon/os/shared/own_writes.rs
native/src/daemon/os/shared/own_writes_tests.rs
native/src/daemon/os/shared/problem_notify.rs
native/src/daemon/os/shared/realtime.rs
native/src/daemon/os/shared/remote_watch.rs
native/src/daemon/os/linux_os/drives.rs
native/src/daemon/os/linux_os/shell.rs
native/src/daemon/os/windows/drives.rs
native/src/daemon/os/windows/session.rs
native/src/daemon/os/windows/shell.rs
native/src/daemon/os/windows/volume_monitor.rs
native/src/keep_awake/mod.rs
native/src/keep_awake/core/types.rs
native/src/keep_awake/os/android.rs
native/src/keep_awake/os/linux_os.rs
native/src/keep_awake/os/windows.rs
native/src/keep_awake/os/shared/holds.rs
native/src/notify_desktop/mod.rs
native/src/notify_desktop/os/android.rs
native/src/notify_desktop/os/linux_os.rs
native/src/notify_desktop/os/windows.rs
native/src/syncjobs/os/linux_os/job_state_lock.rs
native/src/syncjobs/os/windows/job_state_lock.rs
native/src/syncjobs/os/shared/job_state.rs
native/src/syncjobs/os/shared/job_state_classify.rs
native/src/syncjobs/os/shared/job_state_policy.rs
native/src/syncjobs/os/shared/job_state_store.rs
native/src/syncjobs/os/shared/job_state_store_tests.rs
native/src/watch/mod.rs
native/src/watch/core/notify_records.rs
native/src/watch/core/paths.rs
native/src/watch/core/types.rs
native/src/watch/os/shared/host_signal.rs
native/src/watch/os/shared/service.rs
native/src/watch/os/linux_os/backend.rs
native/src/watch/os/linux_os/fs_kind.rs
native/src/watch/os/linux_os/inotify_confined.rs
native/src/watch/os/linux_os/inotify_events.rs
native/src/watch/os/linux_os/inotify_tree.rs
native/src/watch/os/linux_os/watch_tests.rs
native/src/watch/os/windows/backend.rs
native/src/watch/os/windows/device.rs
native/src/watch/os/windows/rdcw_io.rs
```

Eigene Berichte:
```text
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/T-JOBS.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/T-JOBS.md
```

## Gelesene Dateien

Die bearbeiteten Quellen oben und eigenen Berichte wurden gelesen oder
erstellt und anschließend betrachtet. Zusätzlich vollständig/gezielt:
```text
AGENTS.md
docs/ARCHITEKTUR.md
docs/lesungen/INDEX.md
docs/refs/INDEX.md
docs/refs/android-platform.md
docs/refs/android-storage-scan.md
docs/refs/android-sync-triggers.md
docs/refs/local-fs-identity-durability.md
docs/refs/sync-change-detection.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/t-jobs.json
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/umsetzung.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/spec.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/recherche.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-sync.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/fortsetzung.md
graphify-out/.vocab.txt
native/Cargo.toml
native/src/autostart/mod.rs
native/src/bisync/core/completion.rs
native/src/bisync/core/plan_types.rs
native/src/bisync/core/run_types.rs
native/src/bisync/core/types.rs
native/src/bisync/mod.rs
native/src/bisync/os/shared/orchestration.rs
native/src/bisync/os/shared/snapshot.rs
native/src/bisync/os/shared/snapshot_hash.rs
native/src/connect/core/location.rs
native/src/connect/core/local_endpoint.rs
native/src/connect/os/shared/resolution.rs
native/src/daemon/core/tests.rs
native/src/daemon/os/shared/catch_up_tests.rs
native/src/daemon/os/shared/embedded.rs
native/src/daemon/os/shared/handoff.rs
native/src/local_access/mod.rs
native/src/local_access/os/linux/directory_handle.rs
native/src/local_access/os/windows/directory_handle.rs
native/src/share/os/shared/host_watch.rs
native/src/support_dirs.rs
native/src/syncjobs/core/types.rs
native/src/syncjobs/os/shared/persistence.rs
native/src/vfs/core/core.rs
native/src/vfs/core/extension_calls.rs
native/src/vfs/core/extension_types.rs
native/src/vfs/mod.rs
```
Graph: begrenzte vorhandene Abfragen zu watch/job/sync; kein Neubau.
Zusätzlich geladene Skills:
```text
/root/.codex/skills/arbeitsweise/SKILL.md
/root/.codex/skills/graphify/SKILL.md
```
