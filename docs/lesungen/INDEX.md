# Lesungen – Index

Befunde delegierter Codelesungen (Stand je Datei im Kopf). Vor neuen Leseaufträgen hier prüfen.

| Datei | Gegenstand | Zweck |
|---|---|---|
| [2026-09-25-android-portability-share-net-agent.md](2026-09-25-android-portability-share-net-agent.md) | share/, net/, agent/, agent_proto/, quickshare/ | Android-Portabilität (cfg-Auswahlen, Linux-APIs) |
| [2026-09-25-android-portability-daemon-sync-mount-updater.md](2026-09-25-android-portability-daemon-sync-mount-updater.md) | daemon/, syncjobs/, sync/, bisync/, mount/, local_access/, updater/, autostart/, cloud/, gdrive/, creds/, connect/, cli/, support_dirs, build.rs | Android-Portabilität, Datenverzeichnisse, Secrets |
| [2026-09-25-android-core-ops-api-and-portability.md](2026-09-25-android-core-ops-api-and-portability.md) | vfs/, sftp/, ftp/, webdav/, copy/, scanner/, folder_index/, filter/, analytics/, rscan/, zipfs/, linemerge/, types/, icons/, dragout/ | Portabilität + API-Fakten für die Fassade |
| [2026-09-25-android-app-feature-map.md](2026-09-25-android-app-feature-map.md) | app/ (Desktop-GUI) | Funktion → Kern-API, Logik in app/ |
| [2026-09-25-android-background-daemon-model.md](2026-09-25-android-background-daemon-model.md) | daemon/, syncjobs/, autostart/ | Daemon-Modell, Einbettung in einen App-Prozess |
| [2026-09-25-android-ci-release-integration.md](2026-09-25-android-ci-release-integration.md) | .github/workflows, Release-Skripte, RELEASING.md | Task-Suite-Muster, Release-Einbindung der APK |
| [2026-09-25-android-share-facade-map.md](2026-09-25-android-share-facade-map.md) | app/core/share*.rs, share/, daemon IPC-Client, cli/share | Share-Aktionen → Kernaufrufe für die Fassade |
| [2026-09-25-android-sync-facade-map.md](2026-09-25-android-sync-facade-map.md) | Sync-Jobs, bisync, linemerge, Daemon-Steuerung | Sync-/Konflikt-/Hintergrund-Rezepte für die Fassade |
| [2026-09-25-android-files-facade-map.md](2026-09-25-android-files-facade-map.md) | Browsing, Filter, Transfer-Lane, Löschen, Öffnen, ZIP, Index, Cleanup | Datei-Rezepte für die Fassade |
| [2026-09-26-smb-backend-integration.md](2026-09-26-smb-backend-integration.md) | vfs/scheme+dispatch, connect/*, sftp/*, ftp/ftp.rs, mobile/connections+pool, Android ConnectionDraft/Form | Jede Eintragsstelle für ein neues Protokoll (SMB), Zugangsdaten-Speicherung, Backend-aus-gespeicherter-Verbindung, Endpoint-Format-Vorschlag, SFTP-Async-Brücke als Vorbild, Staged-Write-Methoden |
| [2026-09-26-exec-host-flow.md](2026-09-26-exec-host-flow.md) | share/core/exec*.rs, share/os/{linux_os,android}/exec*.rs, daemon exec-Journal/IPC, app/core/share_exec*_ui.rs, mobile share_*.rs | Exec-Host-Datenpfad, Grant-Persistenz, `ContainedExec`-Vertrag für eine Android-Exec-Host-Planung |
