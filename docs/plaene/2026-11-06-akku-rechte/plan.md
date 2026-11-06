# Akku im Hintergrund und Zugriffe trotz Freigabe (2026-11-06)

Teil des Release-Stapels D13/D12 (`docs/plaene/2026-11-01-idealzustand/uebergabe.md`); dieselbe Suite
`crates-task`, dasselbe Release (v0.5.194).

## Auftrag (Nutzer, wörtlich)

- 2026-11-06: „ich habe festgestellt dass eines der letzten updates bis zur aktuellen version auf dem handy zu
  einem sehr  wesentlich höherem akkuverbrauch geführt hat. die ursache dafür ist zu finden und zu behebn, der
  ist durchaus extrem der akkuverbrauch im hintergrund, so sehr dass ich hibtergrundakkunutzung abschalten
  musste. ich kann weder ursachen noch weitere indizien nennen, es ist so lange so oft wie notwendig die
  änderungen von den letzten paar versionen, der code allgemein, zu checken bis alle möglichen ursachen
  gefunden und behoben sind. kein release zum debuggen und testen, es wird so oft so lang so viel geschaut bis
  kein könnte, kein vielleicht, kein unter diesen umständen mehr besteht.“
- 2026-11-06, zu einem Panic-Log des Handys: „das ist nur ein symptom, eine momentaufnahme.“
- 2026-11-06: „beim austauch zwischen handy und windows gab es auch als permission fehler, mit den zugriffen
  die blockiert waren trotz anderer einstellungen. das ist noch mal intensiv zu prüfen, über alle
  betriebssysteme in allen kombinationen hinweg. auch codeseitig.“
- 2026-11-06: „wobei das auch an der windows maschiene liegen kann. auf der hab ich mit allen zugängen auf die
  maschiene das problem dass kein zugriff funktioniert. auch von windows, nur auf der einen maschiene.“
- 2026-11-06: „dann zeigt der log nicht den eigentlichen fehler weil ich sehe einen ordner, Freigaben... das
  wars, der inhalt leer, kommt nichts.“
- Geltende frühere Regel, 2026-10-28: Koppeln zweier Geräte ist die ganze Einrichtung; jede Funktion richtet
  sich aus der einen Freigabe selbst ein (Gedächtnis `connect-is-the-setup`).

## Methode

Drei Lesungen des gesamten Codes, der auf Android im Hintergrund laufen kann (Kotlin-App; nativer Teil über
`android-bridge` → `mobile` → `engine`; vendored iroh/noq), und aller Rechteprüfungen zwischen Geräten. Je Weg:
Auslöser, Periode, Start/Stopp, Datei:Zeile, Änderung zwischen v0.5.180 und v0.5.193, Urteil. Kein Gerät,
kein Build, kein Test lokal. Ergebnis: die Ursachen unten. Panics aus dem gelieferten Log: beide im heutigen
Code behoben (`node_sessions` seit 3b8f51a2, v0.5.165; mdns-sd 0.11.5 → 0.21.6 seit 8c4b5714, v0.5.187),
keine von beiden erzeugte eine Wiederholschleife.

## Ursachen und Behebung

Spalte „seit“: erste Version mit der Ursache. Spalte „Paket“: Verantwortung in der Umsetzung (unten).

### Akku, nativer Teil

| Nr | Ursache (Fundstelle) | seit | Behebung (Ergebnis) | Paket |
|---|---|---|---|---|
| A1 | Routenwächter: Snapshot alle 25 ms je QUIC-Verbindung für deren ganze Lebensdauer (`share/os/shared/route_monitor.rs`) | v0.5.185 | ereignisgesteuert; Takt nur im endlichen Wahlfenster und bei Verkehr. **erledigt** `cc514a6e` | H |
| A2 | Kapazitäts-Worker je Mount-Engine fragt alle 30 s den Anbieter (Desktop-Laufwerke, Kinder des Geräte-Laufwerks, jede Android-Dokumenten-Engine) (`mount/os/shared/capacity_refresh.rs`) | v0.5.185 | nur auf Nachfrage eines Lesers, höchstens alle 30 s; Statusbericht fragt nie. **erledigt** `d2c00459` | H |
| A3 | Geräte-Laufwerk liest alle 20 s Freigabenliste und Ordner jedes einmal geöffneten Geräts neu (`mount/core/device_tree_fs.rs` `refresh_metadata_while`, `mount/os/*/metadata_refresh`) | v0.5.183 | Auffrischen nur nach Zugriff seit der letzten Auffrischung; ein ruhendes Laufwerk weckt kein Gerät | H |
| A4 | Gerätefeed: dauerhafte Screen-Verbindung zu jedem anwesenden Gerät, vom Low-Power-Sweep nicht erfasst (`share/os/shared/peer_feed.rs`, `node.rs:308`, `node_idle.rs:218`) | v0.5.189 | Feeds in Low-Power beenden, bei Sichtbarkeit/Presence/Serververbindung neu starten | A |
| A5 | Feed-Neustart bei jeder Profiländerung irgendeines Geräts, ohne Backoff (`peer_feed.rs:148,193`) | v0.5.189 | Neustart je Gerät bei Änderung dieser Beziehung, Rückkehr in die App oder nach Pause (abgelehnt 1 min bis 30 min, verloren 5 s bis 10 min, Ruhe des Gegenübers 10 min) | A, Review B3/B4 |
| A6 | Feed-Host tastet jede Sekunde ab, auf Android ohne abtastbare Quelle (`screen/core/feed_host.rs:29`) | v0.5.189 | auf Hosts ohne abtastbare Quelle einmal senden, danach nur bei Rechteänderung | A |
| A7 | Alle direkten Kandidatenpfade bleiben offen, je Pfad 5-s-Keepalive (`vendor/iroh-1.3.0/src/socket/remote_map/remote_state.rs:715-722`) | v0.5.185 | nach der Routenentscheidung nicht gewählte IP-Pfade wie upstream zurückbauen | B |
| A8 | mdns-sd 0.21 prüft Schnittstellen alle 5 s statt 30 s (`share/os/shared/lan_presence.rs:47-58`) | v0.5.187 | Intervall 30 s wie zuvor | A2 |
| A9 | Abgelehnte FS-Sitzung: zwei Versuche ohne Pause, je neue QUIC-Verbindung, kein Negativ-Cache (`peer_request.rs:93-101`, `node_sessions.rs:247-287`, `mobile/os/shared/pool.rs:204`) | — | nach Ablehnung kein zweiter Versuch; Ablehnung je Sitzungsschlüssel mit Backoff merken, zurücksetzen bei Änderung der Beziehung | D |
| A10 | Routenhistorie im App-Datenordner ändert den Worker-Fingerprint → Share-Neuladen (`share/os/shared/route_history.rs`, `daemon/os/shared/ipc_host_service.rs:105-114`) | v0.5.185 | Datei außerhalb des Fingerprints, Schreiben gebündelt | A |
| A11 | Exec-Follow-up lädt bei dauerhaftem Fehler alle 10 s alles neu (`daemon/os/shared/exec_grant_journal_follow_ups.rs:32`) | v0.5.190 | Backoff, zurückgesetzt bei Profiländerung | A |
| A12 | Ordner-Watch eines anderen Geräts hält die CPU des Handys für die ganze Watch-Dauer wach (`share/os/shared/host_watch.rs:42`, `keep_awake/os/android.rs`) | vor v0.5.180 | Halten je eingehender Nachricht/Arbeit statt je Stream | A |
| A13 | Hintergrund-Poller meldet `share`-Ereignisse bei jeder Zeitänderung im Statustext (`mobile/os/shared/domains/share_state.rs:343-388`) | vor v0.5.180 | Vergleich ohne zeitabhängige Felder | A2 |
| A14 | `sys.watchHints` macht jeden Cursor einmalig (`mobile/os/shared/sys_platform.rs:81-86`) → jede Sonde gilt als Änderung jeder Echtzeit-Wurzel | vor v0.5.180 | Cursor nur bei echter Änderung | A2 |
| A15 | `next_due` ignoriert Aktivzeitfenster → „fällig jetzt“ außerhalb des Fensters (`daemon/os/shared/due.rs:117-163`) | v0.5.182 | nächster Fensterbeginn | A2 |
| A16 | LAN-Link-Statusrunden alle 2 s auch im Low-Power (`share/os/shared/lan_link_transport.rs:25-30`) | v0.5.170 | im Low-Power keine LAN-Link-Sitzung in beide Richtungen; Fakten behalten ihre 6-s-Gültigkeit (sie autorisieren Uplink-Leases), neue Uplink-Leases warten auf die sichtbare App | A2, Review B11 |

### Akku, Kotlin-App

| Nr | Ursache (Fundstelle unter `android/app/src/main/java/app/smartexplorer/android/`) | seit | Behebung | Paket |
|---|---|---|---|---|
| F1 | Weckalarm alle 30 s, solange ein Job überfällig ist, aber nicht laufen darf (`service/BackgroundController.kt:197-227`, `system/SyncScheduleAlarm.kt`) | v0.5.170 | kein Alarm bei überfälligem Job; WorkManager startet bei erfüllten Bedingungen | C |
| F2 | Keep-Alive (10 min) reiht immer einen Nachhol-Lauf mit Vollsync jedes Echtzeit-Jobs ein (`BackgroundController.kt:160`) | v0.5.170 | nur wenn der Dienst nicht läuft oder ein Job fällig ist | C |
| F3 | Jede MediaStore-Änderung jeder App löst einen Nachhol-Lauf aus (`system/MediaStoreSync.kt`) | v0.5.170 | nur bei Änderungen unter Echtzeit-Wurzeln | C |
| F4 | Speicherstatistik aller Pakete alle 5 min im Hintergrund (`system/AndroidHostFigures.kt`, `HostMonitor.kt`) | v0.5.170 | im Hintergrund selten, bei Sichtbarkeit und Datenträgerereignis sofort | C |
| F5 | Bedienungshilfe-Dienst im Hauptprozess hält Rust-Kern, Daemon und Share dauerhaft am Leben (`screen/ScreenControlService.kt`, Manifest) | v0.5.183 | eigener Prozess, Hauptprozess bindet nur bei Bedarf | C |
| F6 | Dauerbetrieb fragt alle 2 s bei überfälligem, pausiertem Job (`service/BackgroundService.kt:164-174`) | vor v0.5.180 | keine Folge-Schleife; Ereignisse lösen aus | C |
| F7 | UI-Schleifen laufen nach Verlassen der App weiter (Sync-Log, Fensterfamilien, Apps, Routendetails, TCP, Verbindungsdialog) | v0.5.173–v0.5.185 | an den Lebenszyklus gebunden | C, Verbindungsdialog D |
| F8 | Bildschirm-Betrachter und Vorschaukacheln halten Sitzung und 100-ms-Decoder im Hintergrund | v0.5.183 | Kacheln schließen bei ON_STOP, Decoder nur ≥ STARTED | C |
| F9 | Dokumenten-Worker bei jedem Prozessstart bis zu 6 Läufe (`system/documents/SeDocumentsProvider.kt:71-81`) | v0.5.183 | nur bei wiederholbaren Einträgen | C |
| F10 | Exec-Benachrichtigung alle 5 s unverändert neu gepostet (`service/ExecHostNotifier.kt:113`) | v0.5.183 | nur bei geändertem Text | C |

### Zugriffe trotz Freigabe

| Nr | Ursache | seit | Behebung | Paket |
|---|---|---|---|---|
| R1 | Einseitige PIN-Kopplung: die eingebende Seite legt keinen Grant an, die andere hält den Kontakt für freigegeben (`share/core/discovery_exchange_port_impl.rs:433-437`, `direct_reciprocal.rs:288-297,404`); erneutes Koppeln aktiviert `Reconfirm` nicht (`:392-395`) | v0.5.170 | Kopplung öffnet beide Seiten (sichtbare Gegenwahl „nur in eine Richtung“); nur die Kopplung in beide Richtungen hebt `Reconfirm` auf; Kontakt zeigt „Gegenseite hat dich noch nicht freigegeben“ | D, Review B5 |
| R2 | Neuer Grant startet ohne Ordner, Code-Rechte gelten nicht für Kopplung/Gegenfreigabe/Reparatur; Host liefert leere Wurzel ohne Fehler (`share/core/connection_rights_apply.rs:139-153`, `export_device_folders.rs:35-60`, `fs.rs:63-72`) | v0.5.190 | eingestellte Rechte gelten für jede neue Verbindung; eine berechtigte leere Wurzel nennt den Grund; das Geräte-Laufwerk öffnet das Gerät, liest leere Wurzeln neu und zeigt Ablehnungen als „Zugriff verweigert“ mit Grund | D, Review B6/B7 |
| R3 | Jede Ablehnung erreicht das Gegenüber nur als „nicht autorisiert“ (`share/core/server.rs:140-158`, Exec/Bildschirm/TCP gleich) | — | Ablehnungsklassen ohne Geheimnisse auf allen Oberflächen; Host-Ereignis mit Aktion „Freigeben“ | D |
| R4 | Grants ohne Knoten-ID: Dateien gehen, Exec/Bildschirm/TCP/Apps nicht, dazu Strafsperre (`relation_proof.rs:163-170`, `access_authorize.rs:25-31`) | v0.5.183 | Migration `node_id = public_key` beim Laden | D |
| R5 | Schreibschalter ohne Ordnerwahl; Ordnerwahl hängt an exakter Pfadschreibweise | v0.5.190 | Schreibschalter zeigt/öffnet Ordnerwahl; Vergleich normalisiert, Umzug bei geänderten Exporten | D |
| R6 | Laufwerks-Lease nach Neustart des Host-Workers unbekannt, Client fordert keine neue an (`share/core/mount_lease_client.rs`) | — | neue Lease anfordern, Anfrage einmal wiederholen | D |
| R7 | Windows: Share startet bei Anmeldung nicht, wenn nur Laufwerke wiederhergestellt werden (`daemon/os/shared/run_start_reason.rs:167-175`) | v0.5.183 | Share startet, wenn angenommene Grants oder Auto-Join-Räume bestehen | D |
| R8 | Windows: fehlende UDP-Firewall-Regel nur im Log; Netzwerktyp „Öffentlich“ unbemerkt (`share/os/windows/system.rs`) | — | Reparatur in der Oberfläche anbieten, Netzwerktyp melden | D |
| R9 | Nach Identitätsreparatur/Code-Erneuerung alle Grants `Reconfirm` ohne Hinweis | — | Hinweis „N Geräte neu bestätigen“ mit einer Aktion | D |
| R10 | TCP-Versionsunterschied zählt als Anmeldefehler; Geräte-Laufwerk verwirft Freigaben mit unter Windows ungültigen Namen; Android-Host ohne „Zugriff auf alle Dateien“ liefert leere Ordner ohne Hinweis | — | eigene Meldung ohne Strafe; Namen ersetzen statt verwerfen; Hinweis auf dem Android-Host | D |

## Umsetzungspakete

Je Datei genau ein Schreiber. Gemeinsame Verträge: keiner außer R2/R3 (Wire-Feld additiv, ältere Gegenstellen
ignorieren es) und A9/R3 (Ablehnungsklasse am Client).

| Paket | Inhalt | Dateien (Schreibrecht) |
|---|---|---|
| H | A1, A2, A3; Aufgaben 1–3 und 5 der Übergabe (4-h-Ursache, S09, acht Fehlschläge, Suite) | `route_monitor*`, `route_policy.rs`, `mount/**` außer `mount_lease*`, `native/test-crates-task.py`, `.github/workflows/crates-task.yml`, Pläne |
| A | A4, A5, A6, A10, A11, A12 | `share/os/shared/peer_feed*.rs`, `screen/core/feed_host.rs`, `share/os/shared/route_history.rs`, `daemon/os/shared/ipc_host_service.rs`, `daemon/os/shared/exec_grant_journal_follow_ups.rs`, `share/os/shared/host_watch.rs`, `keep_awake/**`, `share/core/node.rs`, `share/core/node_idle.rs`, `share/core/share_power*.rs` |
| A2 | A8, A13, A14, A15, A16 | `share/os/shared/lan_presence.rs`, `share/os/shared/lan_link_*.rs`, `mobile/**/share_state.rs`, `mobile/**/sys_platform.rs`, `daemon/os/shared/due.rs`, `daemon/os/shared/job_triggers.rs` |
| B | A7 | `vendor/iroh-1.3.0/**`, `vendor/noq*/**`, `vendor/SMART_EXPLORER_ROUTE_POLICY.md` |
| C | F1–F10 (F7 ohne Verbindungsdialog) | `android/app/src/main/**` außer den Dateien von D |
| D | R1–R10, A9 | `share/core/**` außer den Dateien von A, `daemon/os/shared/run_start_reason.rs`, `share/os/windows/**`, `crates/gui/**`, `crates/cli/**`, MCP, `mobile/**` außer A2, Android `ui/share/ShareConnectDialog.kt`, `ShareDialogs.kt`, `ShareRightsDialogs.kt`, `DeviceSettingsPage.kt`, `SharePeers.kt`, `system/Permissions.kt` |
| S | Aufgabe 4 der Übergabe (zwei Tests) per Sicherheitsumsetzung | wie dort |

Abnahme je Punkt: das beschriebene Ergebnis, belegt aus dem Code; die eine Suite `crates-task` kompiliert und
testet alle Crates unter Linux/Windows, die Android-Bibliothek und konfiguriert Gradle. Kotlin-Änderungen
prüft die Android-Kompilierung im Release-Build; eine Gerätemessung gibt es nicht (Regel: kein Release zum
Testen; `android/check-background-power.sh` bleibt das Messwerkzeug für den Nutzer).

## Stand

- Umgesetzt und integriert (2026-11-06): A1–A16, F1–F10, R1–R10, A9, N5, dazu Tresor-Begrenzung ohne Rückfall
  (`eeebd71d`) und der Guardian-Fehler (`2114d411`). Opus-Gesamtreview: 11 Befunde behoben (`1adf6045`).
- Übergabe Aufgaben 1–4 erledigt; Aufgabe 5 (ein `crates-task`-Lauf) folgt.
