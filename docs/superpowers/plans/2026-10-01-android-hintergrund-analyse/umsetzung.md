# Umsetzung – Android-Hintergrund und Speicheranalyse

Batch-Basis: `fe21721031ed4612bb5e214e8915dd6fcf1dc4b7`. Test-Präfix: `android_background_task_`.
Spezifikation: `spec.md` (A1–A7, B1–B5). Keine lokalen Builds/Tests (AGENTS.md); Rust-Dateien vollständig
rustfmt-sauber (stdin-Modus über `sudo -n /root/.cargo/bin/rustfmt --edition 2021`).

## Verträge zwischen den Blöcken

### V1 Signal-Draht (Share-Server ↔ Client), Fähigkeit `idle_keepalive_v1`
```text
Client → Server  {"t":"set_idle","idle":true|false}
Client → Server  {"t":"keepalive_ack"}
Server → Client  {"t":"idle_ack","idle":true|false,"keepalive_secs":180}
Server → Client  {"t":"keepalive"}
```
- Nur nach ausgehandelter Fähigkeit; alte Server/Clients senden/erhalten nichts davon.
- Ruhender Client: Server sendet nach K s ohne eingehende Zeile verschobene Meldungen, dann `keepalive`;
  ohne eingehende Zeile binnen 60 s (`IDLE_REPLY_WINDOW`) schließt er. Nicht ruhend: 60-s-Lesefrist wie bisher.
- `set_idle false` → verschobene Meldungen sofort, Antwort `idle_ack`.
- Verschiebung je Beobachter (ruhend): `direct_available` / `room_joined` nur, wenn für (lookup) bzw.
  (room, device) schon eine Presence an diese Verbindung ging, Route (node_id, relay_url, candidates) gleich
  ist und die zuletzt gesendete Presence nicht vor `now + K + 60 + 30` abläuft. Sonst sofort. Offline/Leave
  verwirft Verschobenes für diesen Schlüssel.

### V2 `crate::share::power` (Rust, prozessweit, `share/core/power.rs`)
```rust
pub fn set_low_power(on: bool);
pub fn low_power() -> bool;
pub fn request_probe(network_changed: bool) -> ProbeTicket; // Wach-Alarm / Netzwechsel; Ticket wartet ≤ 12 s auf {ok, reconnected}
pub fn set_activity_hook(hook: fn(hold_ms: u32)); // Host: Wakelock für hold_ms + Poller wecken (K3)
pub fn request_hold(hold_ms: u32);             // intern; gedrosselt, längere Anforderung gewinnt
pub fn signal_power_status() -> SignalPowerStatus;
pub struct SignalPowerStatus { pub idle_supported: Option<bool>, pub idle_active: bool,
                               pub keepalive_secs: Option<u32>, pub last_server_contact_unix: Option<i64> }
```
Desktop ruft nichts davon → Normalbetrieb.

### V3 Kern-API (api.md)
- `sys.hostState` + `deferScheduling:Boolean` (fehlend = false); `foreground` steuert zusätzlich
  `share::power::set_low_power(!foreground)`.
- `share.wake {networkChanged:Boolean}` → `{ok:Boolean, reconnected:Boolean}` (wartet bis zum Ende der Probe, ≤ 12 s).
- Ereignis `{"type":"wake","ms":N}` → Kotlin hält N ms einen Partial-Wakelock (längere Anforderung verlängert).
- `ShareStatus.power = {idleSupported:Boolean?, idleActive:Boolean, keepaliveSecs:Int?, lastServerContactMs:Long?}`.
- `analyze.start {location, platform:{volumeUsedBytes:Long?, otherAppsBytes:Long?}?}`.
- `analyze.node`-Kinder: `kind:"dir"|"file"|"protected"|"rest"|"aggregate"`.
- `analyze.issues` → `{count, text, protectedCount:Long, protectedText:String}` (geschützt nicht in `count`).
- `reclaim.groups` → alle Gruppen; `reclaim.summary {taskId}` →
  `{files:Long, bytes:Long, candidates:Long, groups:Long, protectedCount:Long, errorCount:Long, errorText, limit:String?}`.
- Fortschritt `analyze`/`reclaim`: `message` = Phase/aktueller Ordner (≤ 4/s).

## Kritiker-Befunde (eingearbeitet, gehen den Abschnitten oben und unten vor)

K1 **deferScheduling** ist ein eigenes Bit in `HostState`; es sperrt nur das Einreihen geplanter Jobs
(Start-, Zeit-, Echtzeit-, Anschluss-Jobs, `run_loop.rs` ~158-183), nie `permit_mutation`, laufende Jobs oder
den Nachhol-Lauf (`bg.catchUp`). Der eingebettete Daemon startet mit `deferScheduling = true`, bis der Host
den ersten `sys.hostState` meldet (Fassade setzt es vor dem Daemon-Start). `SyncWorker` überspringt weiterhin
nur im Dauerbetrieb bei laufendem Dienst.

K2 **Bündelung:** Der Server führt je ruhendem Client einen eigenen Takt `next_flush_at` (alle K s, unabhängig
von eingehendem Verkehr); bei jedem Takt sendet er Verschobenes und dann `keepalive`. Verschoben wird eine
Auffrischung nur, wenn die zuletzt an diese Verbindung gesendete Presence (Ablauf auf ≤ Empfang + 300 s
begrenzt) nicht vor `next_flush_at + 30 s` abläuft und die Route gleich ist; sonst sofort. K ist auf
30–210 s begrenzt (300 s Lebensdauer − 60 s Desktop-Auffrischung − 30 s Rand); Standard 180. Das Telefon
veröffentlicht seine Presence bei jedem Aufwachen neu, wenn sie (Wanduhr) älter als 120 s ist. Pflichttest:
K = 180, Herausgeber frischt alle 60 s auf → genau eine Zustellung je Takt, Kopie beim Beobachter nie abgelaufen.

K3 **Arbeit nach dem Aufwachen braucht wache CPU:** `share::power` fordert über den Host kurze Wachhaltungen an
(`set_activity_hook(fn(hold_ms: u32))`): 5 s bei jedem Server-Keepalive (Antwort, Presence, Relay-Prüfung),
15 s je Verbindungsversuch, solange Signal- oder Home-Relay-Verbindung im Ruhemodus fehlt (Backoff-Wartezeit
ohne Wachhaltung), 60 s bei eingehenden Streams im Ruhemodus, alle 30 s erneuert solange Streams offen sind,
15 s bei eingehenden Anfragen/Entscheidungen. `share.wake` wartet bis zum Ende der Probe (≤ 12 s, Ergebnis
`{ok, reconnected}`); der Alarm-Empfänger nutzt `goAsync()` und hält einen Wakelock bis dahin. Nach jedem
Keepalive prüft der Worker den Home-Relay-Status und ruft bei fehlender Verbindung `network_change`.

K4 **Relay-Server:** Server-Pings mit fester Pong-Frist 30 s; erster Ping nach 15 s, nach der ersten Antwort
des Clients im Abstand K (+1–5 s Jitter).

K5 **Beide Server-Transporte** (TCP-Zeilen und WebSocket) setzen V1 mit gemeinsamer Zeitlogik um; TCP behält
teilweise gelesene Zeilen über Lese-Timeouts. `set_idle` darf `keepalive_secs` vorschlagen (Server nimmt das
Minimum mit seinem K, ≥ 30). Client: endete eine ruhende Verbindung zweimal hintereinander früher als K nach
der letzten eingehenden Zeile, schlägt er den halben Wert vor (≥ 30), zurück auf Standard nach einer stabilen
Stunde. `docs/SHARE_SERVER.md`: Proxy-Lesefristen ≥ K + 60 s.

K6 **Leerlauf-Verbindungen statt 15-s-Timer:** Aufräumen nur, wenn die CPU ohnehin wach ist (Eintritt in den
Ruhemodus, jeder Server-Keepalive, Alarm-Probe). Geschlossen wird im Ruhemodus eine Verbindung ohne offene
Streams (eingehend im gestarteten Stream-Task gezählt), ohne an sie gebundene Mount-Lease (eingehend) und
deren STREAM-Frame-Zähler (`Connection::stats()`) sich seit einem ≥ 120 s (Wanduhr) alten Schnappschuss nicht
geändert haben – ausgehend wie eingehend. Schließcode `IDLE` (eigener Anwendungscode, Grund `idle`); eingehend
schließt die Annahmeschleife selbst nach einem bevorzugten `accept_bi`-Versuch. Neue Desktops wiederholen eine
Anfrage einmal auf neuer Verbindung, wenn ihr Stream mit `IDLE` endete (nachweislich nicht ausgeführt). Ein am
Desktop eingebundenes Telefon (Lease) bleibt verbunden (dokumentierte Grenze).

K7 **Stopp-Signale:** IPC-Listener auf Linux/Android blockierend mit 5-s-Annahme-Frist (Adapter in
`daemon/os/{linux_os,android}`), prüft die Stopp-Datei bei jedem Aufwachen; Windows unverändert. Offline-Warten
bekommt einen eigenen Stopp-Kanal vom `ShareService` (Drop/Stop) und wartet sonst auf Kommandos,
Reparatur-Abschlüsse und Proben, längstens bis zum nächsten Discovery-Ablauf bzw. 30 s.

K8 **Reload:** nur der periodische Reload in `ShareHost::tick` und nur im eingebetteten Daemon wird durch den
Fingerabdruck gesperrt; er läuft trotzdem, wenn der Dienst laufen soll und nicht läuft, ein Fehler-, Warte- oder
Wiederholzustand besteht, Altanfragen ablaufen, und spätestens alle 60 s. Heartbeat einmal je Takt nur im
eingebetteten Daemon; Desktop unverändert.

K9 `network_change`/Probe nur bei Wechsel des Standardnetzes oder seiner Adressen, 2 s entprellt.

K10 **Duplikate:** eigene Kandidatensammlung (nach Größe, Pfad einmal gespeichert), Speicherbudget für
Kandidatentext 64 MiB (Grenze sichtbar), paralleles Vergleichen; Desktop behält `max_items` und seine
200er-Kandidatengrenze.

K11 `ring` bleibt: `sha2` 0.10.9 nutzt auf aarch64 ohne Feature `asm` nur die Software-Variante (cfg in
`sha256.rs`), `ring` ist über rustls/noq ohnehin im Baum.

K12 **Geschützte Bereiche** über `apptrash`: der Ordner `<Volume>/Android/data|obb` selbst und alles darunter,
auf kanonischen Pfaden (Alias `/sdcard`, `/storage/self/primary`); Eintragsfehler für `data`/`obb` in
`<Volume>/Android`; geschützte Scan-Wurzel → vollständiges, leeres Ergebnis mit Hinweis statt „Failed“.
Share-gehostete Analyse: geschützte Auslassungen reisen als Hinweis (`notes`), nicht als Fehler.

K13 **Synthetische Zeilen nur in der Anzeige** (`analyze.node`), nie im Ergebnisbaum: „Weitere App-Daten
(laut Android, ≈)“ unter `Android/data` nur für das primäre Volume mit Nutzungszugriff und nur wenn > 0;
„≈ Nicht einzeln erfasst“ nur bei ganzer Volume-Wurzel ohne andere Fehler. Plattformwerte gehören zum Volume
der Wurzel (Kotlin ermittelt es).

K14 Analyse-Threads nur auf Android: min(Kerne, 4); Desktop unverändert.

K15 Ruhende Verbindungen zählen in die bestehende Verbindungsgrenze je Quelle; `set_idle` ohne Fähigkeit wird
ignoriert.

K16 Suite zusätzlich: Host-Tests mit einspeisbarer Uhr (Monoton steht, Wanduhr läuft) für Worker und Server,
TCP und WebSocket, Listener-Stopp/Übergabe, deferScheduling-Gate, geschützte Wurzel/Alias/SD, Kandidatenbudget;
Gerät: `SyncWorker` läuft bei laufendem Dienst, Alarm-Probe unter Wakelock, lange eingehende Vorgänge im
Ruhemodus. Echter Akku-Nachweis nur am Gerät: `android/check-background-power.sh` (batterystats-Weckgründe,
manuell über WLAN-adb).

## Blöcke (parallel, getrennte Dateien)

| Block | Inhalt | Dateien (Schreibrecht) |
|---|---|---|
| S | Share-Server V1, Relay-Ping = K | `share-server/src/**`, `vendor/iroh-relay-1.0.0/src/server.rs`, `.../server/client.rs`, `docs/SHARE_SERVER.md` |
| C | Share-Client: V1, V2, K3, K5–K7 (Worker/Offline), Leerlauf-Aufräumen K6, `network_change`, Desktop-Wiederholung bei `IDLE` | `native/src/share/core/{power,keepalive,signal_worker,signal_connector,signal_handshake,signal_connection,wire,tracked_signal_dispatch,discovery_signal_offline,discovery_signal_maintenance,server,node,node_accept,node_sessions,peer_request,service,direct_reciprocal_coordinator}.rs` + neue Dateien in `native/src/share/core/`, `native/src/share/mod.rs` |
| D | Daemon/Fassade: K1, K7 (Listener), K8, `share.wake`, `wake`-Ereignis, `power` im Status | `native/src/daemon/os/shared/{ipc_listener,run_loop,state,host_state,ipc_host,ipc_host_service}.rs`, `native/src/daemon/os/{linux_os,android,windows}/platform.rs`, `native/src/daemon/mod.rs`, `native/src/mobile/os/shared/{sys,init,runtime}.rs`, `native/src/mobile/os/shared/domains/{mod,share_state,share_status,share_settings}.rs` + neue Datei `domains/share_power.rs` |
| A | Analyse-Kern B1/B2/B4/B5, K10–K14 | `native/src/analytics/**`, `native/src/local_access/os/linux_os.rs`, `native/src/apptrash/mod.rs`, `native/src/share/os/shared/storage_analysis_host.rs`, `native/src/mobile/os/shared/domains/analyze.rs`, `native/Cargo.toml` (+`ring`), `native/Cargo.lock` |
| K | Kotlin Hintergrund A1/A2/A5/A6/A7, K3 (goAsync-Alarm, Wakelock), K9 | `android/.../{prefs/AppPrefs,service/*,system/HostMonitor,system/BootReceiver,system/Permissions,core/Core,core/CoreEvent,core/CoreEvents,api/SyncApi,api/ShareApi,ui/settings/BackgroundSettings,ui/sync/BackgroundParts}.kt` + neue `system/KeepAlive*.kt`, `system/WakeKeeper.kt`, `work/SyncWorker.kt`, `AndroidManifest.xml`, `android/check-background-power.sh` |
| U | Kotlin Analyse-UI B2/B3/B5, K13 | `android/.../{api/AnalyzeApi,ui/analytics/*}.kt` + neue `system/StorageStatsAccess.kt` (Manifest-Zeilen meldet U, K trägt sie ein) |

Integration, `api.md`, Doku, Graph und die Suite macht der Hauptagent.

## Meilensteine und Abnahme (eine Suite am Ende)

| M | Erwartetes Ergebnis (prüfbar) | Prüfung |
|---|---|---|
| M1 | Server: Ruhe-Client bekommt nach K s `keepalive`, ohne Antwort binnen 60 s Trennung; normale Clients unverändert (60 s) | share-server Unit-Tests |
| M2 | Server: Auffrischungen für ruhende Beobachter werden bis zum Keepalive verschoben, Routenwechsel/erste Meldung/Ablaufnähe/Offline sofort | share-server Unit-Tests |
| M3 | Relay-Server pingt im Abstand K (+Jitter) | share-server Test der Konfiguration |
| M4 | Client: im Ruhemodus kein Heartbeat, `keepalive_ack` + Presence bei jedem Keepalive, Probe bei `request_probe`, alter Server → Heartbeats wie bisher | native Host-Tests mit Test-Server |
| M5 | Offline-Warten ohne 25/50-ms-Takt; Stopp/Kommando weckt sofort | native Host-Tests |
| M6 | Idle-Close: eingehende/ausgehende Verbindungen ohne Stream schließen im Ruhemodus nach 15 s; laufende Streams bleiben; Gegenstelle verbindet neu | native Host-Test (zwei Knoten) |
| M7 | Daemon: Listener ohne 100-ms-Takt, beendet sich beim Stopp; Heartbeat je Takt; Reload nur bei Änderung; deferScheduling stoppt nur geplante Jobs | native Host-Tests |
| M8 | Analyse: geschützte Ordner ohne Issues, Status vollständig, Zähler; synthetische Knoten mit Plattformwerten | native Host-Tests |
| M9 | Duplikate: > 200 Kandidaten, Duplikate unter kleinen Dateien gefunden, Hash = SHA-256, parallel gleiches Ergebnis | native Host-Tests |
| M10 | Gerät: Dienst startet nach Neustart ohne App-Öffnen bei eingerichtetem Share; nach Prozess-Kill durch Alarm wieder da | Emulator |
| M11 | Gerät: App im Hintergrund + Doze erzwungen, Server-K = 20 s: Desktop-`se` erreicht das Telefon nach mehreren Keepalives; Server-Log zeigt Ruhemodus | Emulator + Desktop-Bins |
| M12 | Gerät: Leerlauf-Weckungen der Threads `daemon-ipc`/`share-signal`/`background-work` unter Grenzwert | Emulator `/proc/<pid>/task/*/status` |
| M13 | Gerät: Analyse von `/storage/emulated/0` vollständig, `Android/data` geschützt, keine Issues dafür; Duplikate > 200 Kandidaten | Emulator Instrumentierung |

## Status

| Block | Stand |
|---|---|
| S | umgesetzt (Bericht: neue Dateien `share-server/src/{idle,idle_outbox,signal_session,transport_serve,writer_idle}.rs`; WebSocket ohne Ruhemodus behält „keine Eingangsfrist“) |
| C | umgesetzt (`power*`, `signal_*`, `node_idle`, `node_wake`; Worker ereignisgesteuert mit Wächter-Thread; Alarm-Probe: > K+90 s still → neu verbinden, Netzwechsel → Heartbeat mit 10-s-Frist) |
| D | umgesetzt (`share_power.rs`, Listener `poll(2)` 5 s, Reload-Fingerabdruck nur eingebettet, `deferScheduling`) |
| A | umgesetzt (`ProtectedAreas`, `storage_view`, `reclaim/finder*`, `ring`) |
| K | umgesetzt (`KeepAlive*`, `WakeKeeper`, Worker-Klammer für „Beim Start“-Jobs) |
| U | umgesetzt (`StorageStatsAccess`, `AnalysisParts`) |
| Integration | `reclaim.summary` geroutet, api.md §3/§4.1/§4.9/§5, README, TODO (AND5/AND6/DUP1), ARCHITEKTUR |
| B6 | umgesetzt (`storage_view` Apps-Zeilen, `analyze_platform.rs`, `AppDetailDialog.kt`; App-Liste wird vor dem Scan ermittelt, nicht parallel) |
| Suite | `native/test-android-background-task.sh`, `.github/workflows/android-background-task.yml`, Gerätestufen `reach_check`/`reach_boot_check` + `BackgroundReachTaskTest`, `AnalysisProtectedTaskTest`; Läufe 36940992431, 36955746267 (Befunde behoben), 36963157669 grün |
| Release | v0.5.169: complete release 36965674753, Veröffentlichung 36973655500, 20 Assets und Feed-Hashes geprüft |

## Nachtrag B6 – Apps in der Analyse

Vertrag: `analyze.start.platform.apps = [{package, label, appBytes, dataBytes, cacheBytes}]` (nur
Volume-Wurzel des primären Volumes mit Nutzungszugriff). `analyze.node` an dieser Wurzel: Kind
„≈ Apps (laut Android)“ `kind:"apps"`, Größe Σ(appBytes + dataBytes); Pfad hinein → Kinder `kind:"app"`
mit `name` = Label, `package`, `size` = appBytes + dataBytes, `appBytes`, `dataBytes`, `cacheBytes`.
Mit App-Liste kein „Weitere App-Daten“-Eintrag; Rest „≈ System und Sonstiges“ = max(0, belegt − gemessen −
Σ Apps). Manifest `QUERY_ALL_PACKAGES`. Abnahme (M14): Gerät mit `appops … GET_USAGE_STATS allow` →
Wurzel hat `kind:"apps"`, darin die eigene App mit Größe > 0.
