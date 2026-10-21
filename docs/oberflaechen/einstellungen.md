# Oberflächen aller übrigen Einstellungen

Stand: 2026-10-21 (Lauf U42: Ausnahme „Laufwerke“ und Gerätebaum in „Integration“ neu gefasst; übrige Zeilen unverändert). Gilt für alle Einstellungen der App außer Share und Exec (die stehen in [share-exec.md](share-exec.md)). Lesestand des Abschnitts „Bestand vor dem Stapel“: `7e7203c9` (v0.5.181), am Code nachgelesen, Belege als `Datei:Zeile`. Der Abschnitt „Stand nach dem Stapel“ nennt, wo jede Einstellung seit dem Stapel „Parität aller Einstellungen“ (Plan: `docs/plaene/2026-10-16-paritaet-einstellungen/plan.md`) erreichbar ist. Pflege: jede Änderung an einer dieser Einstellungen aktualisiert diese Datei im selben Commit (Regel in [INDEX.md](INDEX.md)).

Abkürzungen: `N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`. „–“ = nicht erreichbar.

## Oberflächen dieses Bereichs

| Kürzel | Oberfläche | Einstieg |
|---|---|---|
| CLI | `se connections`, `se drive`, `se update`, `se plugin`, `se doctor`, `se completions`; neu `se sync`, `se background`, `se connections gdrive`, `se integration` | `N/cli/mod.rs:89-144` (Befehle), Vervollständigung über `clap_complete` (`N/cli/mod.rs:216-218`) |
| Desktop | Fenster „Einstellungen“ mit den Seiten Darstellung, Verbindungen, Updates, Suche & Daten, Integration (`N/app/core/settings_ui.rs:5-13`, `:36-43`); Sidebar „Verbindungen“ (`N/app/core/sidebar.rs:16-90`); Dialog „Neue Verbindung“ (`N/app/core/dialogs.rs:129-326`); Fenster „Sync-Setups“ und Editor (`N/app/core/menus_sync_jobs.rs`, `N/app/core/job_editor_ui.rs`); Laufwerke (`N/app/core/mount_ui.rs`, `mount_ui_draft.rs`) | egui, Windows und Linux |
| Android | „Mehr → Einstellungen“ (`A/ui/settings/SettingsScreen.kt:55-110`), Verbindungen (`A/ui/connections/`), Sync (`A/ui/sync/`) | Compose |
| Bridge | JSON-Methoden `sync.*`, `bg.*`, `conn.*`, `gdrive.*`, `update.*` (`N/mobile/os/shared/domains/mod.rs:104-155`, `:227-228`), Kotlin `A/api/SyncApi.kt`, `A/api/ConnApi.kt`, `A/api/UpdateApi.kt` | Vertrag `docs/superpowers/plans/2026-09-25-android-apk/api.md` |
| IPC | Keine Einstellung dieses Bereichs läuft über die Loopback-IPC: Worker-Steuerung liest Steuerdateien im Sync-Datenordner (`N/daemon/os/shared/state.rs:36-110`), Sync-Setups liegen in `sync/jobs/*.conf` (`N/syncjobs/mod.rs:1-11`), Verbindungen im Verbindungsspeicher (`N/creds/core/types.rs:121-139`) | – |
| MCP | `se mcp`: `status`, `connections`, `se` (`N/cli/mcp/catalog.rs:64-69`, `:176-182`); `se` führt jeden CLI-Befehl aus (`N/cli/mcp/cli_tool.rs:1-6`), daher gilt für MCP die CLI-Spalte | – |
| Doku | `README.md` (Terminal-Abschnitte ab Zeile 601, Updates ab 933), `docs/AI_AGENT_PLUGIN.md`, `docs/CLOUD_SETUP.md`, `docs/SYNC_FEATURES.md` | – |

## Ausnahmen mit Grund (gelten vor und nach dem Stapel)

| Gegenstand | Nur in | Grund |
|---|---|---|
| Darstellung der GUI (Farbschema, kompakte Zeilen, Zusatzspalten, Filterleiste/Zusammenfassung ein, versteckte Dateien, Ordner zuerst, Bildvorschau, Medienbetrachter) | Desktop-GUI bzw. Android-GUI | Gegenstück zur Terminal-Verbindung: wirkt nur auf die Darstellung der jeweiligen GUI. Die CLI hat keine Dateiliste, deren Darstellung sie einstellen könnte. Desktop und Android haben verschiedene Darstellungsmodelle: Desktop „Ordner zuerst“ je Ort (`N/app/core/state.rs:71-72`), versteckte Dateien als Filter der Filterleiste (`N/app/core/filterbar.rs:238`); Android global (`A/prefs/AppPrefs.kt:25-30`) |
| Terminal-Bedienung (`se completions`, `se shell`, Escape-Zeichen) | CLI | Terminal-Verbindung selbst (Nutzerausnahme) |
| `se mcp` (Server für KI-Agenten) | CLI | Wird vom Agenten als Kindprozess gestartet; Einrichtung über `se plugin`, die nach dem Stapel auch die Desktop-GUI bietet |
| Laufwerke: Windows und Linux; Android: Dokumentanbieter (Plattformgrund) | CLI und Desktop unter Windows und Linux; Android nur über den Dokumentanbieter | Windows-Laufwerksbuchstaben (`--letter`) gibt es nur unter Windows, Einhängeorte (`--mount-point`) nur unter Linux (`N/cli/drive.rs:159-167`); Android hat keine Laufwerke und keine CLI (`N/lib.rs`) und stellt Ordner über den Dokumentanbieter bereit (`docs/oberflaechen/laufwerke.md`, Abschnitt 7) |
| Kontextmenü „In Smart Explorer öffnen“ | CLI und Desktop unter Windows | Registry-Eintrag des Explorers (`N/app/os/windows/platform.rs:185-191`); unter Linux meldet die App „nicht verfügbar“ (`N/app/os/linux_os.rs:74-80`) |
| Speicherzugriff „Alle Dateien“, genaue Alarme, Akku-Optimierung, Benachrichtigungen | Android | Android-Berechtigungen (`A/ui/settings/SettingsScreen.kt:151-175`, `A/ui/settings/BackgroundSettings.kt:217-360`) |
| Android-Hintergrundmodus (Aus/Periodisch/Dauerbetrieb), Abstand, nur WLAN, nur beim Laden, Akku nicht niedrig | Android | WorkManager- und Vordergrunddienst-Bedingungen (`A/prefs/AppPrefs.kt:32-36`); Desktop-Gegenstück ist „Beim Anmelden im Hintergrund synchronisieren“ plus Prüfintervall |
| Auslöser „Bei Anschluss“ (Laufwerk/USB) und sein Muster | Desktop, CLI | Android meldet keine Laufwerksanschlüsse (`N/mobile/os/shared/domains/sync_jobs.rs:55-58`, Hinweis `A/ui/settings/BackgroundSettings.kt:204`) |
| SSH-Agent als Anmeldung | Desktop, CLI | Android hat keinen SSH-Agenten (`N/mobile/os/shared/domains/connections.rs:131-132`) |
| Netzlaufwerk (UNC) | Desktop und CLI unter Windows | WNet-Anmeldung nur unter Windows (`N/net/os/linux_os.rs:3-12`, CLI `N/cli/os/linux_os.rs:18-26`) |
| Update-Rollback auf frühere Versionen | Desktop | Der Rollback tauscht die laufende App und startet sie neu (`N/app/core/settings_updates.rs:112-232`); `se update` stuft nie herunter (`N/cli/update.rs:139-144`) |
| Ordnerindex der Suchleiste, Temp-Wiederherstellung, App-Fehlerprotokoll | Desktop bzw. Android | Daten und Wartung der jeweiligen GUI, keine Einstellung |

## Bestand vor dem Stapel (Lesestand `7e7203c9`)

### Gespeicherte Verbindungen

| Einstellung | CLI | Desktop | Android | Bridge / MCP | Lücke |
|---|---|---|---|---|---|
| Anlegen SFTP/FTP/FTPS/WebDAV | `se connections add` (`N/cli/connections.rs:65-95`, `:119-168`) | „Neue Verbindung“ (`N/app/core/dialogs.rs:148-170`) | Formular (`A/ui/connections/ConnectionForm.kt:46-153`) | `conn.save` (`N/mobile/os/shared/domains/connections.rs:207-258`); MCP über `se` | – |
| Anlegen SMB | – (`ConnectionProtocolArg` ohne SMB, `N/cli/connections.rs:98-104`) | nicht angeboten (`N/app/core/dialogs.rs:154`), obwohl der Verbinder SMB auf allen Plattformen kann (`N/connect/os/shared/connector.rs:110`, `:291-327`) | mit Freigabe und Domäne (`A/ui/connections/ConnectionForm.kt:46-48`) | `conn.save` | Lücke: CLI, Desktop |
| Anlegen UNC | `add share`, unter Linux abgelehnt (`N/cli/os/linux_os.rs:18-26`) | angeboten, unter Linux scheitert die Anmeldung (`N/net/os/linux_os.rs:3-12`) | – (Ausnahme) | – | Lücke: Desktop bietet UNC unter Linux an |
| Anmeldung Passwort/Schlüssel+Passphrase/SSH-Agent, Remote-Agent | `--password(-stdin)`, `--key`, `--ssh-agent`, `--agent` (`N/cli/connections.rs:147-168`) | `N/app/core/dialogs.rs:226-286` | Passwort/Schlüssel/Remote-Agent (`N/mobile/os/shared/domains/connections.rs:78-158`) | `conn.save` | – |
| Bearbeiten | – | – | „Bearbeiten“ mit Mitnahme des Geheimnisses (`N/mobile/os/shared/domains/connections.rs:207-258`) | `conn.save` mit `id` | Lücke: CLI, Desktop |
| Entfernen mit Bereinigung (Favoriten, Ordnereinstellungen, Laufwerke; Syncs gemeldet) | `se connections remove` entfernt nur die Zeile (`N/cli/setup.rs:64-79`) | `N/app/core/share_removal_ui.rs:104-121` | `conn.delete` (`N/mobile/os/shared/domains/connections.rs:260-286`) | MCP über `se` | Lücke: CLI ohne Bereinigung |
| Auflisten mit Feldern | `se connections list [--json]` ohne Anmeldeart (`N/cli/connections/output.rs:1-31`, `:66-79`) | Sidebar (`N/app/core/sidebar.rs:67-82`) | Liste (`conn.list`, `N/mobile/os/shared/domains/connections.rs:53-60`) mit `auth`, `keyPath` | MCP `connections` (`N/cli/mcp/status_tools.rs:52-63`) | Lücke: CLI/MCP ohne Anmeldeart |
| SSH-Hostschlüssel vertrauen | `trust-host-key` (`N/cli/connections/host_keys.rs:31-57`) | nach Fehler „Hostschlüssel prüfen …“ (`N/app/core/host_key_ui.rs:35`, `:96`) | `conn.trustHostKey` | MCP über `se` | – |
| SSH-Hostschlüssel vergessen | `forget-host-key` (`host_keys.rs:59-74`) | – | `conn.forgetHostKey` (`A/ui/connections/ConnectionsScreen.kt:116`) | – | Lücke: Desktop |

### Google Drive

| Einstellung | CLI | Desktop | Android | Bridge / MCP | Lücke |
|---|---|---|---|---|---|
| OAuth-Client-ID und Secret | – | `N/app/core/settings_cloud.rs:39-51` | `GoogleDriveSection` (`A/ui/settings/SettingsScreen.kt:96-99`) | `gdrive.configure` (`N/mobile/os/shared/domains/gdrive.rs:25-48`) | Lücke: CLI, MCP |
| Anmelden | – | „Mit Google verbinden“ (`settings_cloud.rs:58-103`) | `gdrive.signIn` | `gdrive.rs:50-96` | Lücke: CLI, MCP |
| Trennen | – | `settings_cloud.rs:24-35`, Sidebar | `gdrive.signOut` | `gdrive.rs:98-103` | Lücke: CLI, MCP |
| Status | – | `settings_cloud.rs:14-17` | `gdrive.status` | `gdrive.rs:15-23` | Lücke: CLI, MCP |

### Sync-Setups (je Feld von `SyncJob`, `N/syncjobs/core/types.rs:66-163`)

CLI und MCP hatten keinen Befehl für Sync-Setups. Desktop-Editor: `N/app/core/job_editor_ui.rs:41-319`. Android-Editor: `A/ui/sync/JobEditorScreen.kt:144-324`, Bridge `N/mobile/os/shared/domains/job_json.rs:21-66` (Ausgabe) und `:126-235` (Eingabe).

| Feldgruppe | Desktop | Android | Lücke |
|---|---|---|---|
| Name, Seite A/B, Richtung, Konflikte, Löschungen, Verschieben, Aktiv | `job_editor_ui.rs:45-117`, `:317-319` | `JobEditorScreen.kt:144-151`, `:235-288` | CLI, MCP |
| Vergleich, Zeit-Toleranz | `:119-137` | Vergleich ja, Zeit-Toleranz – | CLI, MCP; Android: Zeit-Toleranz |
| Versionen, Tage, Anzahl | `:139-161` | `:240-249` | CLI, MCP |
| Versionsort (`versions_location`) | – | `:251` | Desktop, CLI, MCP |
| Papierkorb, Lösch-Schutz Dateien/% | `:163-174` | `:290-316` | CLI, MCP |
| Lösch-Schutz Mindestzahl (`max_delete_min`) | – | `:309-310` | Desktop, CLI, MCP |
| Auslöser, Intervall, Kalender, Echtzeit-Verzögerung, Gerät/USB, Aktive Zeiten, Nachholen | `:176-242` | `:154-218`, `:274-283` (ohne „Bei Anschluss“, Ausnahme) | CLI, MCP |
| Echtzeit Höchstwartezeit, Abfrageintervall, Kontroll-Lauf, Vollprüfung des Ziels | – | `:255-264` | Desktop, CLI, MCP |
| Versteckte Dateien, Ignoriermuster | `:244-265` | `:252`, `:265-273` | CLI, MCP |
| Eingehängte Dateisysteme (`cross_mounts`) | – | `:253-254` | Desktop, CLI, MCP |
| Größen- und Altersfilter | `:267-283` | – | CLI, MCP, Android |
| Bandbreite, parallel, sichere Kopien, Überprüfen, Wiederholungen, Pause | `:285-308` | – | CLI, MCP, Android |
| Befehl davor/danach | `:310-315` | `:318-324` | CLI, MCP |
| Befehl bei Abbruch (`run_cleanup`) | – | `:324` | Desktop, CLI, MCP |
| Aktivieren/Deaktivieren, Löschen | `N/app/core/menus_sync_jobs.rs:99-107`, `:192-218` | `A/ui/sync/JobCards.kt:149-155`, `sync.setEnabled`/`sync.delete` | CLI, MCP |
| Detailprotokoll (unveränderte Einträge einzeln) | `N/app/core/sync_job_log_ui.rs:111-121` | `sync.setLogVerbose` (`A/ui/sync/SyncLogScreen.kt:94`) | CLI, MCP |
| Protokoll lesen | `sync_job_log_ui.rs` | `sync.log` | CLI, MCP |

### Hintergrunddienst

| Einstellung | CLI | Desktop | Android | Bridge / MCP | Lücke |
|---|---|---|---|---|---|
| Hintergrund-Sync (Autostart) an/aus | – | `N/app/core/settings_background.rs:9-52` | Modus „Aus“ = Sync-Flag (`N/mobile/os/shared/domains/background.rs:91-116`) | `bg.setSyncEnabled` | CLI, MCP |
| Prüfintervall 2–3600 s | – | `settings_background.rs:94-120` | nur gelesen (`bg.status` `cadenceSecs`, `background.rs:62`) | – | CLI, MCP, Android setzen |
| Pause (Dauer, unbegrenzt), Fortsetzen | – | `settings_background.rs:123-171` | `A/ui/settings/BackgroundSettings.kt:272-289` | `bg.pause`, `bg.resume` | CLI, MCP |
| Automatische Pause (Energiesparmodus, getaktetes Netz) | – | `settings_background.rs:174-206` | `BackgroundSettings.kt:172-185` | `bg.setAutopause` | CLI, MCP |
| Status (Dienst aktiv, letzte Meldung) | `se doctor` nur `running`/`heartbeat_age_seconds` (`N/cli/doctor.rs:119-121`) | `settings_background.rs:66-91` | `bg.status` | MCP `status` über `doctor` | CLI ohne Pause/Takt/Autopause |
| Worker-Protokoll | – | `settings_background.rs:53-62` | `BackgroundSettings.kt:196` | `bg.log` | CLI, MCP |

### Updates

| Einstellung | CLI | Desktop | Android | Lücke |
|---|---|---|---|---|
| Update-Quelle | `se update --source` (`N/cli/update.rs:66-71`) | `N/app/core/settings_updates.rs:29-56` | fest (Release-Feed, `A/update/UpdateChecker.kt`) | – (Android: Ausnahme, die APK kommt aus dem Release) |
| Prüfen, Installieren | `--check`, ohne Option (`N/cli/update.rs:122-162`) | `settings_updates.rs:28`, `:97-110` | `A/ui/settings/UpdateCard.kt`, `update.check`/`update.download` | – |
| Auto-Update-Pause nach Rollback anzeigen/aufheben | – | `settings_updates.rs:59-82` | – (kein Rollback) | CLI zeigt die Pause nicht |
| Beim Start prüfen (ein/aus) | – | immer an (`N/app/core/init.rs:155`) | Schalter (`A/ui/settings/SettingsScreen.kt:182`) | siehe „Stand nach dem Stapel“ |

### Integration

| Einstellung | CLI | Desktop | Android | Lücke |
|---|---|---|---|---|
| Kontextmenü (Windows) | – | `N/app/core/menus_settings.rs:126-172` | – (Ausnahme) | CLI unter Windows |
| KI-Agenten-Plugin (Claude Code, Codex): Status, Installieren, Entfernen, Konfiguration für andere Clients | `se plugin` (`N/cli/plugin.rs:48-62`) | – | – (kein `se` auf Android) | Desktop |
| Laufwerke: Laufzeit, Einbinden mit Buchstabe, Schreiben, Wurzel vertrauen, Metadaten-Tiefe, Cache, Systemlaufzeit, Name; Liste, Trennen, Wiederholen | `se drive` (`N/cli/drive.rs:25-38`, `N/cli/drive_options.rs:5-46`) | `N/app/core/mount_ui_draft.rs:127-260`, `mount_ui.rs:155-276` | – (Ausnahme) | – |
| Verbundene Geräte als Ordner (Gerätebaum): Schalter, Buchstabe bzw. Einhängeort, Zustand, Öffnen im Dateimanager | `se drive devices status`, `watch`, `on`, `off` (`N/cli/drive_devices.rs`) | Abschnitt „Verbundene Geräte als Ordner“ in Einstellungen → Integration (`N/app/core/settings_devices_ui.rs`) | – (Ausnahme: Dokumentanbieter, `docs/oberflaechen/laufwerke.md` Abschnitt 5 und 7) | – |

## Stand nach dem Stapel

Ausgeliefert in v0.5.182 am 2026-10-16 (Nachweis: Suite `luecken-task.yml`, Lauf 38929646147 grün; siehe Plan). „=“ heißt: unverändert wie im Bestand. Für MCP gilt die CLI-Spalte, weil das Werkzeug `se` jeden CLI-Befehl ausführt; der Katalog nennt die neuen Befehle (`N/cli/mcp/catalog.rs:39-41`, Beispiele im Werkzeug `se`) und `status` zeigt Hintergrundwerte und Zahl der Sync-Setups (`N/cli/mcp/status_tools.rs:10-50`).

### Gespeicherte Verbindungen

| Einstellung | CLI | Desktop | Android | Bridge / MCP | Ergebnis |
|---|---|---|---|---|---|
| Anlegen SMB | `se connections add smb` (`N/cli/connections.rs:116`, `:127`), Prüfung „Startpfad beginnt mit der Freigabe“ (`N/cli/setup.rs:331-333`) | Protokoll „SMB (Windows-/Samba-Freigabe)“ auf Windows und Linux (`N/app/core/connect_dialog_ui.rs:14-37`, Hinweis `:149-154`) | = | = | geschlossen |
| Anlegen UNC | = (nur Windows) | nur noch unter Windows angeboten (`N/app/core/connect_dialog_ui.rs:22-24`, Adapter `N/app/os/windows/platform.rs`, `N/app/os/linux_os.rs`) | Ausnahme | – | geschlossen (Linux-GUI bot UNC an, das dort scheiterte) |
| Bearbeiten | `se connections edit <label|account>` mit `--host --port --user --root --label --key --ssh-agent --password-auth --agent/--no-agent --password(-stdin)` (`N/cli/connections/edit.rs:85-128`, `N/cli/setup.rs:98-131`), Vervollständigung der gespeicherten Bezeichnungen (`N/cli/connections/edit.rs:130-145`) | Sidebar „⋯ → Verbindung bearbeiten…“ (`N/app/core/sidebar.rs:103-105`), Menü „Weitere“ „✎“ (`N/app/core/menus_sync.rs:72`), Dialog im Bearbeiten-Modus mit „Speichern“ (`N/app/core/connect_dialog_ui.rs:49-53`, `:238-247`, `N/app/core/connection_edit_ui.rs:45-93`) | = | gemeinsamer Speicherweg `connect::save_edited_connection` (`N/connect/os/shared/connection_edit.rs:59-93`) für CLI, Desktop und `conn.save` (`N/mobile/os/shared/domains/connections.rs:208`); nach einem Umzug (Host, Port, Benutzer, Startpfad) nennen alle drei die Sync-Setups und Laufwerke auf der alten Adresse (`N/connect/os/shared/connection_edit_refs.rs`; CLI-Meldung, Desktop-Fehlerzeile, Android-Snackbar aus `warnings` der Antwort von `conn.save`), geändert werden sie wie beim Entfernen nicht | geschlossen |
| Entfernen mit Bereinigung | `se connections remove` wie die GUI: Eintrag, Geheimnis, Favoriten, Ordnereinstellungen, Laufwerke; Syncs genannt (`N/cli/setup.rs:81-94`) | = | = | MCP über `se` | geschlossen |
| Auflisten mit Anmeldeart | `--json` mit `auth`, `key_path`, `edit_command` (`N/cli/connections/output.rs:87-109`) | = | = | MCP `connections` dieselben Zeilen | geschlossen |
| Google Drive in der Liste | Zeile `gdrive`, solange angemeldet (`N/cli/connections/output.rs:72-85`) | Sidebar (=) | Einstellungen (=) | MCP `connections` mit Ziel `gdrive:///` (`N/cli/mcp/status_tools.rs:79-101`) | geschlossen |
| SSH-Hostschlüssel vergessen | = | Sidebar „⋯ → SSH-Hostschlüssel vergessen“ bei SFTP (`N/app/core/sidebar.rs:106-110`, `N/app/core/connection_edit_ui.rs:96-115`) | = | = | geschlossen |
| Verbindung testen | `se stat @label:/` (Ausnahme: kein eigener Befehl) | „Verbinden“ | „Testen“ | – | Ausnahme: Test ist eine Aktion; jede Oberfläche hat einen Weg |

### Google Drive

| Einstellung | CLI | Desktop | Android | Ergebnis |
|---|---|---|---|---|
| Client-ID/Secret | `se connections gdrive configure --client-id … [--client-secret … \| --client-secret-stdin \| --clear-client-secret]` (`N/cli/connections/gdrive.rs:40`, `:158-176`) | = | = | geschlossen |
| Anmelden | `se connections gdrive sign-in [--client-id …]`: Adresse auf stderr, Browser über den Plattformöffner, Rückleitung `127.0.0.1` (`N/cli/connections/gdrive.rs:178-198`, `N/cloud/mod.rs` `open_platform_url`) | = | = | geschlossen |
| Trennen, Status | `sign-out`, `status [--json]` mit den Feldern von `gdrive.status` (`N/cli/connections/gdrive.rs:35-44`, `:141-156`) | = | = | geschlossen |

### Sync-Setups

| Feldgruppe | CLI | Desktop | Android | Ergebnis |
|---|---|---|---|---|
| Alle Felder des `JobEditor` | `se sync add/edit` mit einer Option je Feld, Namen = Schlüssel der Jobdatei, Auswahlwerte = gespeicherte Werte (`N/cli/sync_jobs/options.rs:34-259`, Übernahme `:269-360`); `@label:/pfad` wird zum Endpunkt, lokale Seiten müssen absolut sein (`:397-417`) | Editor auf fünf Seiten statt einer langen Liste (`N/app/core/job_editor_pages.rs:9-87`, Seiten `:95-455`, Fenster `N/app/core/job_editor_ui.rs:34-89`, Sprung zur Seite eines Fehlers `:59-71`) | „Erweitert“ plus neue Abschnitte „Filter“ und „Übertragung“ (`A/ui/sync/JobEditorScreen.kt:241-251`, `:328`, `:338-371`), Entwurf `A/ui/sync/JobDraft.kt:61-72` | geschlossen |
| Höchstwartezeit, Abfrageintervall, Kontroll-Lauf, Vollprüfung des Ziels | `--rt-max-latency-secs`, `--rt-poll-secs`, `--verify-interval-secs`, `--verify-target-secs` | Seite „Zeitplan“ (`N/app/core/job_editor_pages.rs:242-258`) | = | geschlossen (Desktop fehlte) |
| Versionsort, Löschschutz-Mindestzahl | `--versions-location`, `--max-delete-min` | Seite „Vergleich & Versionen“ (`N/app/core/job_editor_pages.rs:320-346`) | = | geschlossen (Desktop fehlte) |
| Eingehängte Dateisysteme | `--cross-mounts` | Seite „Filter“ (`N/app/core/job_editor_pages.rs:412-414`) | = | geschlossen (Desktop fehlte) |
| Befehl bei Abbruch | `--run-cleanup` | Seite „Übertragung & Befehle“ (`N/app/core/job_editor_pages.rs:449`) | = | geschlossen (Desktop fehlte) |
| Zeit-Toleranz, Größen-/Altersfilter, Bandbreite, parallel, sichere Kopien, Überprüfen, Wiederholungen, Pause | `--modify-window-sec`, `--filter-*`, `--bwlimit-kbps`, `--max-transfers`, `--atomic-copy`, `--verify`, `--retries`, `--retry-delay-secs` | = | neu (`A/ui/sync/JobEditorScreen.kt:241-251`, `:338-371`); Bridge Ein-/Ausgabe `modifyWindowSec`, `filterMinSizeKb`, `filterMaxSizeKb`, `filterMaxAgeDays`, `filterMinAgeDays`, `bwlimitKbps`, `maxTransfers`, `atomicCopy`, `verifyCopies`, `retries`, `retryDelaySecs`, `connectMatch` (`N/mobile/os/shared/domains/job_json.rs:70-81`, `:144-271`), Feldfehler `:351-389`; Vertrag `docs/superpowers/plans/2026-09-25-android-apk/api.md` §4.7 | geschlossen (Android fehlte) |
| Muster „Bei Anschluss“ | `--connect-match` | = | Ausnahme (Android meldet keine Laufwerksanschlüsse); Bridge trägt den Wert unverändert | Ausnahme |
| Auflisten, Anzeigen | `se sync [list] [--json]`, `se sync show JOB [--json]` mit Zustand (`N/cli/sync_jobs.rs:163-210`, JSON `N/cli/sync_jobs/output.rs:11-69`) | = | = | geschlossen |
| Aktivieren/Deaktivieren, Entfernen | `se sync enable/disable/remove JOB`; Entfernen und Bearbeiten lehnen einen laufenden Job ab wie Android (`N/cli/sync_jobs.rs:149-158`, `:229-240`, `:279-289`) | = | = | geschlossen |
| Protokoll und Detailprotokoll | `se sync log JOB [--from N] [--verbose true\|false] [--json]` (`N/cli/sync_jobs.rs:242-275`) | = | = | geschlossen |
| Auswahl eines Jobs | Id, exakter Name, Id-Präfix ≥ 4; Vervollständigung (`N/cli/sync_jobs.rs:300-343`) | – | – | geschlossen |
| Jetzt ausführen, Vergleichen, Versionen wiederherstellen, Konflikte lösen, Sicherheitsstopp bestätigen | – | = | = | offen: Aktionen, keine Einstellungen; ein CLI-Lauf braucht Abbruch mit Strg+C, Fortschritt und Konfliktdialog (eigener Auftrag, in `status.txt` des Stapels genannt) |

### Hintergrunddienst

| Einstellung | CLI | Desktop | Android | Ergebnis |
|---|---|---|---|---|
| Hintergrund-Sync beim Anmelden | `se background enable/disable` mit derselben Rücknahme wie die GUI (`N/cli/background.rs:45-48`, `:134-148`); der Anmeldeeintrag startet `se --sync-daemon` bzw. nach dem nächsten GUI-Start die App | = | = (Modus „Aus“) | geschlossen |
| Prüfintervall | `se background cadence 2..3600` (`N/cli/background.rs:51-58`) | = | neu: „Prüfintervall“ (`A/ui/settings/BackgroundSettings.kt:192-193`, `:219-245`), `bg.setCadence` (`N/mobile/os/shared/domains/background.rs:148-157`, `N/mobile/os/shared/domains/mod.rs:146`), Kotlin `A/api/SyncApi.kt:394-396` | geschlossen |
| Pause, Fortsetzen | `se background pause 2h\|90m\|1d\|SEKUNDEN\|forever`, `resume` (`N/cli/background.rs:59-68`, Dauer `:313-340`) | = (2/8/24 h, ∞) | = (1 h, bis morgen, unbegrenzt) | geschlossen; die Schnellwahl unterscheidet sich je GUI, der gespeicherte Wert ist derselbe |
| Automatische Pause | `se background autopause [--battery true\|false] [--metered true\|false]`; eine auf der Plattform nicht beobachtbare Bedingung bleibt wie gespeichert (wie die gesperrte GUI-Checkbox) (`N/cli/background.rs:69`, `:152-184`) | = | = | geschlossen |
| Status | `se background [status] [--json]` (`N/cli/background.rs:186-253`), MCP `status.background` | = | = | geschlossen |
| Worker-Protokoll | `se background log [--lines N]` | = | = | geschlossen |

### Updates

| Einstellung | CLI | Desktop | Android | Ergebnis |
|---|---|---|---|---|
| Auto-Update-Pause nach Rollback | `se update --check` zeigt `auto_update\tpaused after a rollback to …` bzw. JSON `auto_update_paused_at` (`N/cli/update.rs:315`, `:355-373`) | = | – (kein Rollback) | geschlossen |
| Beim Start prüfen | `se update --startup-check true\|false` (`N/cli/update.rs:72-77`, JSON `startup_check` `:316`) | neu: „Beim Start nach Updates suchen“ (`N/app/core/settings_updates.rs:27-39`), wirksam beim Start (`N/app/core/init.rs:148`), gespeichert in `update_startup_check.txt` (`N/updater/os/shared/config.rs:79-103`) | = („Beim Start prüfen (höchstens 1× am Tag)“, eigene App-Einstellung) | geschlossen; Android behält seinen eigenen Wert, weil das Telefon keine Desktop-Updates bezieht |
| Rollback | – | = | – | Ausnahme (siehe oben) |

### Integration

| Einstellung | CLI | Desktop | Android | Ergebnis |
|---|---|---|---|---|
| Kontextmenü (Windows) | `se integration context-menu [on\|off] [--json]`; registriert die App neben `se`, nie `se` selbst (`N/cli/integration.rs:53-75`, `N/shell_register/os/windows.rs` `set_context_menu_for`, Adapter `N/cli/os/{windows,linux_os}.rs`) | = | Ausnahme | geschlossen |
| KI-Agenten-Plugin | = (`se plugin`) | neu: „Einstellungen → Integration → KI-Agenten“ mit Status je Host, Installieren (alle/Claude Code/Codex), Entfernen, Konfiguration für andere Clients; führt das installierte `se` aus (`N/app/core/settings_ai_plugin.rs:93-113`, `:189-266`, Pfad `N/updater/mod.rs` `installed_cli_path`) | Ausnahme (kein `se` auf Android) | geschlossen |

### Offene Punkte nach dem Stapel

- Sync-Aktionen in der CLI (siehe Tabelle Sync-Setups, letzte Zeile).
- Die Desktop-GUI liest Sync-Setups, die `se sync` ändert, beim nächsten Neuladen der Liste (Öffnen des Fensters „Sync-Setups“, Speichern im Editor); eine offene Liste zeigt bis dahin den alten Stand. Gleiches galt schon für Android-Änderungen.
