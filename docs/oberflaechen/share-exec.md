# Oberflächen von Share und Exec

Stand: 2026-10-16. Lesestand des Abschnitts „Bestand vor dem Stapel“: `7e7203c9` (v0.5.181), am Code nachgelesen, Belege als `Datei:Zeile`. Der Abschnitt „Stand nach dem Stapel“ nennt, wo jede Fähigkeit seit dem Stapel „GUI-Parität Share/Exec“ (Plan: `docs/plaene/2026-10-16-luecken-schliessen/plan.md`, Abschnitt I) erreichbar ist. Pflege: jede Änderung an einer Share-/Exec-Einstellung aktualisiert diese Datei im selben Commit (Regel in `docs/oberflaechen/INDEX.md`).

Abkürzungen: `N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`.

## Oberflächen

| Kürzel | Oberfläche | Einstieg |
|---|---|---|
| CLI | `se share …`, dazu `se connections add-peer/add-room/remove-peer/remove-room` | `N/cli/share.rs`, `N/cli/share/*.rs`, `N/cli/connections.rs` |
| Desktop | Fenster „Geräte & Freigaben“ (egui) | `N/app/core/share_window_ui.rs` und die `share_*_ui.rs` daneben; Server und Gerätename zusätzlich in den Einstellungen „Verbindungen“ (`N/app/core/menus_settings.rs`, Bereich des zweiten Programmierers) |
| Android | Tab „Teilen“, Einstellungen „Share-Server“ und „Hintergrund“ | `A/ui/share/ShareScreen.kt` und Nachbarn, `A/ui/settings/ShareServerSettings.kt`, `A/ui/settings/BackgroundSettings.kt` |
| Bridge | JSON-Methoden `share.*` der Android-Bibliothek | `N/mobile/os/shared/domains/mod.rs:160-208`, Domänen `share_*.rs`; Kotlin `A/api/Share*.kt`; Vertrag `docs/superpowers/plans/2026-09-25-android-apk/api.md` §5 |
| IPC | Loopback-IPC GUI/CLI ↔ Hintergrund-Worker | `N/daemon/os/shared/ipc_protocol.rs:262-310` (`MutateExecGrant`, `ExecShare`, `ExecStream`, `ExecJobs`, `CancelExec`, `ShareCommand`), Antworten `:404-416` |
| MCP | `se mcp` (Werkzeuge `status`, `connections`, `exec`, `sessions`, `se`) | `N/cli/mcp/catalog.rs`; `se` führt jeden CLI-Befehl aus (`N/cli/mcp/cli_tool.rs:1-6`), daher gilt für MCP die CLI-Spalte |
| Netz | Exec-Protokoll zwischen Geräten | `N/share/core/exec_protocol.rs:82-94` (`ExecHelloOk`, `ExecWireError`), Ablehnungstexte `N/share/core/exec_policy.rs:171-199`, `N/share/core/exec_program_path.rs:73-78` |

## Bestand vor dem Stapel (Lesestand `7e7203c9`)

„–“ = nicht erreichbar. „Lücke“ nennt, was der Stapel ergänzt; „Ausnahme“ nennt, warum eine Oberfläche die Fähigkeit bewusst nicht hat.

### Exec

| Fähigkeit | CLI | Desktop | Android | Lücke / Ausnahme |
|---|---|---|---|---|
| Exec-Grants auflisten mit Konto und Programmliste | `se share grants exec` (`N/cli/share/grants_exec.rs:277-317`, Text und `--json` mit `run_as`, `programs`); `se share grants`/`se share status` nur `exec=`, `exec_revision=` (`N/cli/share/grants.rs:121-125`, `:149-162`) | Karte je Gerät (`N/app/core/share_exec_ui.rs:74-133`) | Liste ohne Konto und Programme (`A/ui/share/ExecHostSection.kt:104-132`, Bridge `N/mobile/os/shared/domains/share_exec.rs:43-59`) | Lücke: `se share grants`/`status`, Vervollständigung (`N/cli/completions.rs:77-126`) und Android zeigen Konto und Programme nicht |
| Exec ein/aus | `se share grants exec enable/disable` (`grants_exec.rs:16-18`) | `share_exec_ui.rs:121`, `:193-263` | „Erlauben…“/„Entziehen“ (`ExecHostSection.kt:124-128`), `share.setExec` (`share_exec.rs:63-105`) | – |
| Ausführungskonto | `--run-as` (`grants_exec.rs:61-65`) | Kontofeld nur auf Hosts mit Kontowechsel (`share_exec_ui.rs:125-127`, `:147-191`) | – | Lücke: Android zeigt kein Konto (Android kann nur den Standardnutzer, `N/share/core/exec_policy.rs:95-106`) |
| Programmliste | `--allow-program`, `--full` (`grants_exec.rs:66-77`), Hilfe `:21-40` | nur Anzeige (`share_exec_ui.rs:107-112`), Verweis auf die CLI (`:224`) | – | Lücke: Desktop und Android können sie nicht bearbeiten |
| Exec-Jobs, Verlauf, Abbruch | `se share exec list/show/cancel/history` (`N/cli/share/exec_status.rs:15-21`) | `N/app/core/share_exec_jobs_ui.rs:24-96` | eingehende Jobs mit „Stopp“ (`ExecHostSection.kt:76-85`), Benachrichtigung `A/service/ExecHostNotifier.kt` | Ausnahme: Android zeigt nur Befehle auf diesem Telefon; ausgehende Befehle laufen im Dialog „Befehl ausführen“ (`A/ui/share/ExecDialog.kt`) |
| Exec-Protokoll | `se share exec log` (`exec_status.rs:22-34`, `:100-120`) | – | – | Lücke: Desktop und Android |
| Befehl auf einem Gerät ausführen | `se exec`, `se shell` | – | „Befehl ausführen“ (`A/ui/share/ExecDialog.kt`) | Ausnahme: Terminal-Verbindung selbst (Nutzerauftrag); Desktop hat keinen Befehlsdialog |

### Server, Identität, Verbindung

| Fähigkeit | CLI | Desktop | Android | Lücke / Ausnahme |
|---|---|---|---|---|
| Share-Server anzeigen/setzen/entfernen | `se share server show/set/clear` (`N/cli/share.rs:104-113`, `:191-248`), `configure` | Menü „Verbindung“ zeigt die Adresse (`N/app/core/share_window_ui.rs:61-75`), Bearbeiten in den Einstellungen „Verbindungen“ | `A/ui/settings/ShareServerSettings.kt:34`, `share.setServer` (`N/mobile/os/shared/domains/share_settings.rs:78-99`) | – |
| Verbinden/Trennen (gespeichertes `auto_connect`) | `se share worker refresh/stop` (`N/cli/share.rs:262-280`) | „Verbinden“/„Trennen“ (`share_window_ui.rs:64-71`, `N/app/core/share.rs:139-178`) | „Online“-Schalter (`A/ui/share/ShareScreen.kt:200`, `share_settings.rs:144-173`) | – |
| Gerätename | nur `se share configure --device-name` mit Pflicht-`--server` (`N/cli/share.rs:82-91`) | „Name aendern“ (`N/app/core/share_direct_ui.rs:73-90`) | „Umbenennen“ (`ShareScreen.kt:199`, `share_settings.rs:175-187`) | Lücke: CLI ohne eigenen Befehl |
| Direkt-Code anzeigen, kopieren | `se share identity` (`N/cli/share/identity_command.rs:17-43`) | `share_direct_ui.rs:35-60` | Gerätekarte (`A/ui/share/ShareDeviceCard.kt`) | – |
| Direkt-Code erreichbar/unerreichbar (Laufzeit, `ShareCmd::SetDirectOnline`, `N/share/core/signal_commands_local.rs:59`) | – | „Online/Offline schalten“ (`share_direct_ui.rs:91-97`) | – | Lücke: CLI, Android |
| Direkt-Code neu erzeugen | nur als Reparatur (`identity_command.rs:60-80`) | „Code neu generieren“ (`share_direct_ui.rs:98-115`, `N/app/core/share_identity_rotation.rs:9-37`) | – | Lücke: CLI, Android |
| Identität reparieren | `se share identity --repair` (`identity_command.rs:9-13`) | – | – | Ausnahme: Reparatur fehlender Geheimnisse ist ein Wartungsweg der CLI (`se doctor` zeigt den Zustand) |
| Diagnose | `se share status`, `se doctor` | Tab „Diagnose“ (`N/app/core/share_diagnostics_ui.rs:4-86`) | Fehlerkarten (`ShareScreen.kt:181-196`) | Ausnahme: Diagnose-Log ist Sitzungszustand der Desktop-GUI |

### Anfragen und Geräte

| Fähigkeit | CLI | Desktop | Android | Lücke / Ausnahme |
|---|---|---|---|---|
| Anfragen auflisten/annehmen/ablehnen/erneut/löschen | `se share request …` (`N/cli/share/requests.rs:31-41`) | `N/app/core/share_lifecycle_ui.rs:45-110` | `RequestsSection` (`ShareScreen.kt:235-243`) | – |
| Anfrageregel (fragen / automatisch annehmen) | nur Anzeige in `se share export list` (`N/cli/share/exports.rs:83-106`) | nur Anzeige (`N/app/core/share_policy_ui.rs:6-21`) | ändern (`ShareScreen.kt:211-220`, `share.setPolicy` `N/mobile/os/shared/domains/share_policy.rs:283-292`) | Lücke: CLI, Desktop |
| Zugriff anfragen | `se connections add-peer` (Anfrage), `se share request retry` | „Zugriff anfragen“ (`share_direct_ui.rs:209-216`) | `SharePeers.kt:86` | – |
| Unbestätigte Kopplungen | – | `N/app/core/share_discovery_publish_ui.rs:166-196` | `ShareConfirmationDialogs.kt` | Ausnahme: Sitzungszustand der GUI, die gekoppelt hat (`share_settings.rs:322-384`); die CLI meldet ihr Ergebnis sofort im Terminal |
| Gerät per Direkt-Code hinzufügen | `se connections add-peer` (`N/cli/connections.rs:79-80`) | `share_direct_ui.rs:117-182` | `ShareDialogHost.kt:134` | – |
| Rückfreigabe je Kontakt | – | Menü „Verwalten“ (`share_direct_ui.rs:219-228`) | „Eigene Freigaben…“ (`SharePeers.kt:87`) | Lücke: CLI |
| Automatisch verbinden / öffnen je Kontakt | – | `share_direct_ui.rs:229-240` | – | Lücke: CLI, Android |
| Vertrauen zurücksetzen je Kontakt | – | `share_direct_ui.rs:264-270` | – | Lücke: CLI, Android |
| Gerät entfernen | `se connections remove-peer`, `se share grants delete` | `share_direct_ui.rs:271`, `share_lifecycle_ui.rs:319-333` | `SharePeers.kt:88` | – |
| Schreibrecht je Gerät | `se share grants set` (`N/cli/share/grants.rs:31`) | `share_policy_ui.rs:22-44` | `ShareRightsDialogs.kt` | – |
| Autorisierung widerrufen / wieder erlauben | `se share grants revoke/allow` | `share_lifecycle_ui.rs:280-317` | `SharePolicySections.kt:37`, `ShareConfirmationDialogs.kt` | – |
| Entfernte Geräte, erneut zulassen | `se share grants removed` | `N/app/core/share_removed_devices_ui.rs` | `ShareScreen.kt` Block `Removed` | – |

### Räume

| Fähigkeit | CLI | Desktop | Android | Lücke / Ausnahme |
|---|---|---|---|---|
| Raum erstellen | `se share room create` (`N/cli/share.rs:124-127`) | `N/app/core/share_rooms_ui.rs:19-58` | `ShareScreen.kt:232` | – |
| Raum per Code beitreten | `se connections add-room` (`N/cli/connections.rs:82`) | `share_rooms_ui.rs:60-105` | `ShareScreen.kt:233` | – |
| Räume mit Mitgliedern auflisten | nur Anzahl in `se share status` (`N/cli/share/status.rs:68-80`) | `share_rooms_ui.rs:117-231` | `SharePeers.kt:96-182` | Lücke: CLI ohne Mitgliederliste |
| Verlassen / wieder beitreten (`auto_join`) | – | „Verlassen“, „Beitreten“, „Auto“ (`share_rooms_ui.rs:133-145`) | nur „Verlassen“ (`SharePeers.kt:137`) | Lücke: CLI; Android ohne Wiederbeitritt |
| Umbenennen | – | „Umbenennen“ nimmt das Namensfeld des Beitreten-Abschnitts (`share_rooms_ui.rs:160-163`) | – | Lücke: CLI, Android; Desktop mit eigenem Feld |
| Raum-Code anzeigen | – | „Code kopieren“ (`share_rooms_ui.rs:151-159`) | „Code anzeigen“ (`SharePeers.kt:135`) | Lücke: CLI |
| Raum entfernen | `se connections remove-room` | `share_rooms_ui.rs:164` | `SharePeers.kt:138` | – |
| Raumrechte (Mitglieder schreiben, neue bestätigen) | `se share export policy` (`N/cli/share/exports_policy.rs:35-133`) | `share_rooms_ui.rs:168-182` | „Rechte und Bestätigung…“ (`SharePeers.kt:136`) | – |
| Mitglied zulassen / sperren / wieder zulassen | – | nur Häkchen „Blockieren“ ohne Zulassen eines wartenden Mitglieds (`share_rooms_ui.rs:222-224`) | `SharePeers.kt:175-179`, `share.setRoomMember` (`share_policy.rs:201-240`) | Lücke: CLI; Desktop ohne „Zulassen“ |
| Mitglied: Vertrauen zurücksetzen | – | `share_rooms_ui.rs:225-229` | – | Lücke: CLI, Android |

### Freigaben, Auffindbarkeit, Netzwerk, Papierkorb

| Fähigkeit | CLI | Desktop | Android | Lücke / Ausnahme |
|---|---|---|---|---|
| Ordner freigeben/entfernen, Zugriff, Systemschreibrechte | `se share export add/remove/set` (`N/cli/share/exports.rs:17-20`) | `N/app/core/share_exports_ui.rs:38-218` | `ShareLists.kt:111`, `ShareRightsDialogs.kt` | – |
| Reihenfolge der Freigaben | – | „Nach oben/unten“ (`share_exports_ui.rs:94-99`) | – | Lücke: CLI, Android |
| Gespeicherte Verbindungen freigeben | `se share export connections` | `N/app/core/share_connections_ui.rs` | `A/ui/share/ShareConnectionDialog.kt` | – |
| Auffindbar machen, Angebote, Stoppen | `se share discoverable` (`N/cli/share/discoverable.rs:49-61`) | `share_discovery_publish_ui.rs` | `ShareDeviceCard.kt:96`, `ShareScreen.kt` | – |
| Geräte/Räume entdecken und per PIN verbinden | `se share discoverable` (Verbinden im Terminal) | `N/app/core/share_discovery_ui.rs` | `A/ui/share/ShareConnectDialog.kt` | – |
| LAN-Erkennung ein/aus | `se share lan presence` (`N/cli/share/lan.rs:22`) | `N/app/core/share_lan_ui.rs:32-45` | nur Anzeige `lanPresence` (`N/mobile/os/shared/domains/share_status.rs:301-313`) | Lücke: Android |
| Internet teilen (ohne Router) | `se share lan uplink` (`lan.rs:24-50`) | `N/app/core/share_lan_uplink_ui.rs` | – | Ausnahme: Android-Apps können keine Internetfreigabe einrichten (kein Root, keine Netzwerkkonfiguration) |
| LAN-Firewall reparieren (Windows) | – | `share_lan_ui.rs:47-57` | – | Ausnahme: nur Windows-Desktop mit UAC-Freigabe; die CLI meldet den Zustand in `se share lan status` |
| Host-Papierkorb (Windows) | – | Tab „Papierkorb“ (`N/app/core/share_host_trash_ui.rs`) | – | Lücke: CLI; Ausnahme Android (kein Windows-Host) |
| Quick Share (LAN, Google-Protokoll) | – | `N/app/core/quickshare_ui.rs` | – | Ausnahme: eigene Funktion des Desktops (`docs/QUICKSHARE.md`), keine Share-Einstellung |
| Share im Hintergrund erreichbar | Worker läuft als Dienst | Hintergrund-Daemon | `A/ui/settings/BackgroundSettings.kt:74-105` | Ausnahme: nur Android braucht einen Vordergrunddienst |

### MCP und Netz

| Fähigkeit | Stand | Lücke / Ausnahme |
|---|---|---|
| Alle CLI-Einstellungen | `se`-Werkzeug (`N/cli/mcp/catalog.rs:176-183`) | gilt wie die CLI-Spalte; Beispiele im Katalog nennen Exec-Einstellungen nicht |
| Exec-Ziele | `connections` nennt `exec: "share"` (`N/cli/mcp/status_tools.rs:66-81`) | – |
| Share-Zusammenfassung | `status` (`status_tools.rs:38-49`) zählt Grants | – |
| Exec-Ablehnung am Gerät | `ServerFrame::Error` mit Text der Programmliste (`exec_policy.rs:171-199`, `exec_program_path.rs:73-78`) | Ausnahme: `ExecHelloOk` trägt keine Einschränkung; der Host verrät seine Liste nicht vorab, die Ablehnung nennt die Regel |

## Stand nach dem Stapel (ausgeliefert in v0.5.182, 2026-10-16)

Jede Lücke des Bestands ist geschlossen oder als Ausnahme begründet. Neue Belege:

### Exec

| Fähigkeit | CLI | Desktop | Android (Bridge → App) |
|---|---|---|---|
| Liste mit Konto und Programmen | `se share grants`/`status`: JSON `exec.run_as`, `exec.programs`, Text `exec_run_as=`, `exec_programs=` (`N/cli/share/grants.rs:121-127`, `:151-168`); Vervollständigung `enabled · as KONTO · programs …` (`N/cli/completions.rs:122`) | Seite **Befehle**: Zeile je Gerät mit Zustand und Konto (`N/app/core/share_exec_ui.rs:42-76`) | `runAs`, `programs`, `execProvider.userLabel` (`N/mobile/os/shared/domains/share_exec.rs`, `target_json`/`provider_json`) → Zeile „Ausführen als: …“, „Programme: …“ (`A/ui/share/ExecHostSection.kt:127`) |
| Programmliste bearbeiten | `--allow-program`, `--full` | **Befehle → Einstellungen → Programmliste bearbeiten** bzw. beim Aktivieren (`N/app/core/share_exec_programs_ui.rs:61`, `:132`; Regeln wie die CLI-Hilfe, Validierung `ExecGrantSettings::validate`/`validate_for_this_host`, Bestätigung je Änderung) über `crate::daemon::mutate_exec_grant` (IPC `MutateExecGrant { settings }`) | `share.setExec {programs}` (`share_exec.rs::program_settings`) → ⋮ **Programme…** (`A/ui/share/ExecProgramsDialog.kt:69`) |
| Konto | `--run-as` | Kontofeld (nur Linux-root-Worker) | Anzeige „Standardnutzer (…, Konto der App)“; Ausnahme: Android kann kein anderes Konto |
| Exec-Protokoll | `se share exec log` | **Befehle → Exec-Protokoll dieses Hosts** (`N/app/core/share_exec_log_ui.rs:21`), Leser `N/share/os/shared/exec_audit_view.rs` | `share.execLog {lines}` → **Befehlsprotokoll anzeigen** (`A/ui/share/ExecAuditDialog.kt:52`) |
| Gerät aus Raum: Befehle | `se share grants exec` | Raumseite → Mitglied **Befehle …** öffnet die Exec-Karte (`N/app/core/share_room_page_ui.rs`, `member_row`) | Zeile des Mitglieds im Abschnitt „Befehle auf diesem Telefon“ |

### Share-Einstellungen

| Fähigkeit | CLI (neu) | Desktop | Android (neu) |
|---|---|---|---|
| Gerätename | `se share identity --set-name NAME` (`N/cli/share/identity_command.rs:20-24`) | **Dieses Gerät** (`N/app/core/share_this_device_ui.rs:16`) | wie bisher |
| Direkt-Code erreichbar | `se share identity --direct-online true|false` | **Dieses Gerät** (`share_this_device_ui.rs:23`) | `share.setDirectOnline` → Gerätekarte „Erreichbar“/„Nicht erreichbar“ (`A/ui/share/ShareDeviceCard.kt:115`) |
| Direkt-Code erneuern | `se share identity --rotate-code [--yes]` | **Dieses Gerät → Code neu generieren** (`share_this_device_ui.rs:32`) | `share.renewCode` → „Code erneuern…“ (`ShareDeviceCard.kt:117`) |
| Anfrageregel | `se share request policy [ask|auto-accept]` (`N/cli/share/requests_policy.rs`) | **Anfragen** (`N/app/core/share_policy_ui.rs:8`) | wie bisher |
| Kontakt: automatisch verbinden/öffnen, Rückfreigabe | `se share device set --auto-connect/--auto-open/--share-back` (`N/cli/share/devices.rs`) | Geräteseite (`N/app/core/share_device_page_ui.rs:160`, `:216`) | `share.setContactOptions` → ⋮ **Verbindung…** (`A/ui/share/SharePeers.kt:95`, `ShareDeviceSettingsDialogs.kt`) |
| Kontakt: Vertrauen zurücksetzen | `se share device reset-trust` | Geräteseite | `share.resetContactTrust` → **Verbindung… → Vertrauen zurücksetzen…** |
| Räume: Liste mit Mitgliedern, verlassen, wieder beitreten, umbenennen, Code, entfernen | `se share room list|join|leave|rejoin|rename|code|remove` (`N/cli/share/rooms.rs`) | Raumseite (`N/app/core/share_room_page_ui.rs:70`, `:159`) | `share.renameRoom`, `share.rejoinRoom` → ⋮ **Umbenennen…**, **Wieder beitreten** (`SharePeers.kt:145-146`) |
| Mitglied zulassen/sperren/wieder zulassen | `se share room member RAUM GERÄT admit|block|allow` | Raumseite, eigene Knöpfe je Zustand (`share_room_page_ui.rs:192`) | wie bisher; ein wartendes Mitglied zeigt jetzt „Zulassen…“ statt „Wieder zulassen…“ |
| Mitglied: Vertrauen zurücksetzen | `se share room member … reset-trust` | Raumseite | `share.resetMemberTrust` → „Vertrauen zurücksetzen“ (`SharePeers.kt:189`) |
| Reihenfolge der Freigaben | `se share export move SEL --up|--down` (`N/cli/share/exports_order.rs`) | wie bisher | `share.moveExport` → „Nach oben/unten“ (`A/ui/share/ShareLists.kt:106-107`) |
| LAN-Erkennung | wie bisher | wie bisher (**Netzwerk**) | `share.setLanPresence`, Status `lanPresenceEnabled` → Schalter in der Gerätekarte (`ShareDeviceCard.kt:88`) |
| Host-Papierkorb | `se share trash list|restore` (`N/cli/share/host_trash.rs`) | wie bisher (**Papierkorb**) | Ausnahme wie oben |

Gemeinsamer Kern aller drei Oberflächen: `N/share/core/profile_settings.rs` (`set_contact_options`, `reset_contact_trust`, `rename_room`, `set_room_auto_join`, `reset_member_trust`, `move_export_root`).

Verbleibende Ausnahmen (unverändert begründet oben): Terminal-Verbindung, Identität reparieren (CLI), Diagnose-Log und unbestätigte Kopplungen (Sitzungszustand der GUI), Internet teilen und LAN-Firewall (Plattform), Host-Papierkorb auf Android, Quick Share (eigene Desktop-Funktion), Share im Hintergrund (nur Android). MCP erreicht jede CLI-Zeile über das `se`-Werkzeug; die Beispiele im Katalog (`N/cli/mcp/catalog.rs`) nennen die neuen Befehle noch nicht (Datei parallel in Arbeit, siehe `docs/TODO.md` SHARE-GUI-PARITY).

### Desktop-Bedienweg vorher → nachher

| Aufgabe | Vorher (eine lange Seite „Geräte“) | Nachher |
|---|---|---|
| Gerät öffnen | Geräte: Zeile → **Öffnen** (C1) | Geräte: Zeile → **Öffnen** (C1, unverändert) |
| Automatisch verbinden/öffnen, Rückfreigabe, Vertrauen zurücksetzen, Diagnose, Fingerprint, Entfernen | Geräte: Zeile → Menü **Verwalten** (C2) | Geräte: Zeile → **Details** → Seite des Geräts (C3; alle Einstellungen des Geräts zusammen, dazu Autorisierung, Schreibrecht, Befehle) |
| Offene Anfrage entscheiden | Geräte: nach unten scrollen bis „Eingehende Anfragen“ (C3 mit Scrollen) | Tab **Anfragen (n)** mit Zählung; Hinweis auf der Geräteseite (C1 + C1) |
| Autorisierung widerrufen / wieder erlauben | Geräte: Abschnitt „Autorisierte Geräte“ (Scrollen) | Geräteseite → Autorisierung (C3) |
| Exec eines Geräts | Geräte: Abschnitt „Remote-Ausführung pro Gerät“ (Scrollen) | Tab **Befehle** → **Einstellungen** (C3) oder Geräteseite → Befehle |
| Exec-Jobs | Geräte: unter den Exec-Karten | Tab **Befehle** |
| Direkt-Code, Name, Online/Offline, Code neu | Geräte: aufklappbar „Dieses Gerät & Direkt-Code“ | Tab **Dieses Gerät** (C1 Tab + direkt) |
| Gerät hinzufügen (PIN oder Code) | Geräte: zwei aufklappbare Abschnitte | Geräte → **Gerät hinzufügen …** (eine Seite für PIN und Code) |
| Entfernte Geräte | Geräte: aufklappbarer Abschnitt | Geräte → **Entfernte Geräte (n) …** |
| Quick Share LAN | Geräte: aufklappbarer Abschnitt | Tab **Netzwerk** (aufklappbar) |
| Raum verwalten | Räume: alle Räume mit allen Knöpfen und Mitgliedern auf einer Seite | Räume: Zeile → **Details** → Raumseite; Erstellen/Beitreten/Entdecken auf eigener Seite |
| Anfrageregel ändern | nicht möglich (nur Anzeige) | Tab **Anfragen** |

Tabnummern 0 bis 5 (Geräte, Räume, Freigaben, Diagnose, Netzwerk, Papierkorb) behalten ihre Bedeutung für Seitenleiste, GUI-Harness und Ereignisse; eingehende Anfragen und Entscheidungen öffnen jetzt **Anfragen** (6). Der Raum-Knopf „Oeffnen“ der alten Liste setzte nur die angezeigte Statusbeschriftung (ohne Speicherung, ohne Verbindung) und entfällt; Mitglieder werden einzeln geöffnet. Das Häkchen „Blockieren“ ist durch Knöpfe je Zustand ersetzt (Zulassen, Sperren, Wieder zulassen) mit derselben Kernfunktion (`set_member_blocked`/`admit_member`) und Identitätsprüfung wie Android.
