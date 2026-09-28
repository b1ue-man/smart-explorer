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
| [2026-09-28-share-ipc-agent-path-concurrency.md](2026-09-28-share-ipc-agent-path-concurrency.md) | agent/, agent_proto/, daemon backend_server/transfer/tree_send/request_workers, ipc_client | Desktop-/Android-Share-Pfad GUI→Daemon: Multiplexing, 16 Anfragen je IPC-Verbindung, Roundtrips, GetTree/PutTree sammeln erst alles |
| [2026-09-28-share-host-transport-throughput.md](2026-09-28-share-host-transport-throughput.md) | share/core Server, fs, wire, framing, node(_sessions), keepalive, walk | Host-Seite je FsRequest, 32 blockierende FS-Ops je Prozess, QUIC-Konfiguration, JSON-Draht, Kompatibilität unbekannter Anfragen |
| [2026-09-28-sftp-cache-local-concurrency.md](2026-09-28-sftp-cache-local-concurrency.md) | sftp/, vfs cache*, vfs local/copy_transfer, connect/connector | SFTP eine Sitzung/Kanal, Lesen ohne Vorauslesen, CachingBackend-Weiterleitung, LocalBackend-Primitive |
| [2026-09-28-ftp-webdav-smb-unc-concurrency.md](2026-09-28-ftp-webdav-smb-unc-concurrency.md) | ftp/, webdav/, smb/, net/core/backend.rs | Verbindungsmodell, Lese-/Schreib-Sperren (FTP-Deadlock-Risiko), Spooling, mkdir_all je Ebene |
| [2026-09-28-gdrive-transfer-cost.md](2026-09-28-gdrive-transfer-cost.md) | gdrive/ | API-Aufrufe je Datei/Ordner, globale Sperren, Cache-Schreiben, Rate-Limits |
| [2026-09-28-android-transfer-contract.md](2026-09-28-android-transfer-contract.md) | mobile Fassade (runtime, edits), api.md, Kotlin files UI | `fs.transfer`-Vertrag, In-App-Zwischenablage ohne Vorab-Download, Task-Felder |
