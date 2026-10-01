# Android: Share im Hintergrund erreichbar (akkuschonend) und Speicheranalyse – Spezifikation

Stand: 2026-10-01. Auftrag des Nutzers: „Hintergrund auf Android verlässlicher machen; ohne den Akku zu
killen auf eingehende Direct-Share-Verbindungen reagieren, ohne dass die App offen ist; ein
Hintergrundprogramm soll ohne vorheriges Öffnen der App aktiv werden und aktiv bleiben. Speicheranalysen
optimieren; sie stoppt frühzeitig bei `data/` wegen fehlender Rechte, mehr Rechte kann ich nicht geben.“
Der Nutzer verzichtet auf die Freigabe der Spezifikation (direkt umsetzen und veröffentlichen).

Belege: `docs/lesungen/2026-10-01-android-share-host-idle-activity.md`,
`docs/lesungen/2026-10-01-android-storage-analysis-path.md`, `docs/refs/android-background-reachability.md`,
`docs/refs/android-storage-scan.md`.

## Befund (warum es heute nicht trägt)

- Standardmodus „Periodisch“: kein Vordergrunddienst, der Prozess wird eingefroren → Share ist nur bei offener
  App erreichbar. Nur „Dauerbetrieb“ hält den Prozess.
- Auch im Dauerbetrieb ist die Erreichbarkeit im Tiefschlaf nicht verlässlich: alle Timer der App (Rust/tokio,
  `Thread.sleep`) laufen auf `CLOCK_MONOTONIC` und stehen während Suspend. Der Client-Heartbeat (20 s) kommt
  nicht, der Server trennt die Signal-Verbindung nach 60 s Stille; die signierte Presence (300 s) wird nicht
  erneuert und läuft bei den Gegenstellen ab („gespeicherte Presence ist abgelaufen“).
- Gleichzeitig weckt der Prozess das Gerät ständig: Relay-Server-Ping alle 16–20 s, QUIC-Keepalive 5 s je
  offener Peer-Verbindung (Desktop hält Sitzungen nach dem Blättern dauerhaft offen), Multicast-Lock dauerhaft
  gehalten (jedes mDNS/SSDP-Paket im WLAN weckt), Presence-Weiterleitungen jedes Kontakts alle 60 s mit
  Profil-Schreibvorgang. Bei wacher CPU: IPC-Listener alle 100 ms, Offline-Warten alle 25–50 ms, Heartbeat-Datei
  alle 2 s, Profil-Reload alle ~6 s.
- Speicheranalyse: Der Scan bricht bei `Android/data` nicht ab, aber jeder fremde App-Ordner dort (bzw. die
  Ordner selbst) wird als Lesefehler gezählt; `Android/data`/`obb` erscheinen mit 0 B, die Fehlerliste ist voll
  davon, und das Ergebnis heißt „teilweise“. Android 11+ sperrt diese Ordner für jede App (auch mit
  „Zugriff auf alle Dateien“); das ist eine Plattformgrenze, kein Rechteproblem des Nutzers.
- Duplikate prüfen nur die 200 größten Dateien ≥ Mindestgröße (alle kleineren Duplikate bleiben unentdeckt),
  einspurig und mit Software-SHA-256.

## A Hintergrund-Erreichbarkeit

- **A1 Einstellung „Share im Hintergrund erreichbar“** (Standard an). Der Hintergrunddienst (specialUse) läuft,
  wenn Dauerbetrieb gewählt ist oder (Einstellung an und Share eingerichtet = letzter `share.status.running`).
  Er startet bei Geräte-Neustart, App-Update, jedem Prozessstart und durch den Wach-Alarm – ohne die App zu
  öffnen. Grenze (Android): nach der Erstinstallation oder einem „Beenden erzwingen“ muss die App einmal
  geöffnet werden (Stopped-State); das steht im Hilfetext.
- **A2 Aktiv bleiben:** `START_STICKY`; Wach-Alarm (`setAndAllowWhileIdle`, `ELAPSED_REALTIME_WAKEUP`, alle
  10 min = 6/h, innerhalb der 7/h-Quote ohne Akku-Ausnahme) an einen Manifest-Empfänger: startet Prozess und
  Dienst neu, falls beendet, und prüft die Verbindung. Ohne Akku-Ausnahme darf Android den Neustart aus dem
  Hintergrund verweigern → Hinweis mit [Akku-Optimierung ausschalten] (bestehende Zeile).
- **A3 Ruhemodus** (App nicht sichtbar; Desktop nie): Share-Server-Fähigkeit `idle_keepalive_v1`.
  - Telefon meldet `set_idle`; der Server pingt das ruhende Telefon nach K Sekunden Stille (`keepalive`,
    Standard K = 180 s: ntfy nutzt 3 min, kleinster gemessener Mobilfunk-TCP-Timeout 255 s, Presence-Lebensdauer
    300 s; Server-Env `SE_SHARE_IDLE_KEEPALIVE_SECS`, 30–1800, für Proxys mit kürzerem Timeout) und erwartet
    binnen 60 s eine Antwort. Das Telefon sendet im Ruhemodus selbst keine Heartbeats.
  - Presence: bei jedem Server-Keepalive neu signiert und veröffentlicht (Wanduhr), bei Routenwechsel sofort.
  - Presence-Bündelung: für ruhende Beobachter verschiebt der Server reine Auffrischungen (gleiche Route) bis
    zum nächsten Keepalive; sofort gehen erste Meldung, Routenwechsel, drohender Ablauf der zuletzt gesendeten
    Presence, Offline, Anfragen, Entscheidungen, Discovery. Gleiches für `room_joined`.
  - Relay: Server-Ping an alle Relay-Clients alle K s (+1–5 s Jitter) statt 15 s (vendored iroh-relay);
    Desktop-Clients pingen weiter selbst alle 15 s, für sie ändert sich nichts.
  - Peer-Verbindungen: im Ruhemodus schließt das Telefon eingehende und eigene zwischengespeicherte
    QUIC-Verbindungen ohne offenen Stream nach 15 s (Gegenstelle baut beim nächsten Zugriff transparent neu auf,
    vorhandener Pfad in `node_sessions`); sofort beim Eintritt in den Ruhemodus für bereits leere Verbindungen.
  - Multicast-Lock nur bei sichtbarer App, während einer Kopplung/Suche oder bei reinem LAN-Betrieb (kein
    Share-Server – dort ist mDNS der einzige Weg).
  - Netzwechsel (Android-Callback) → `endpoint.network_change()` und sofortige Verbindungsprobe.
  - Wach-Alarm → Probe: war der letzte Serverkontakt (Wanduhr) älter als K + 60 s → Heartbeat mit 10-s-Frist,
    sonst neu verbinden.
  - Alter Share-Server ohne Fähigkeit: Verhalten wie bisher; Status zeigt „Share-Server ohne Ruhemodus –
    Server aktualisieren“.
- **A4 Leerlauf ohne Polling:** IPC-Listener blockiert (Wecken beim Stopp), Offline-Warten wartet auf
  Kommandos/Abschlüsse statt 25–50-ms-Schleifen, Daemon-Heartbeat einmal je Takt statt alle 2 s,
  Share-Reload nur bei geänderten Eingabedateien (Sicherheitsnetz 60 s), Hintergrunddienst aktualisiert die
  Benachrichtigung ereignisgesteuert.
- **A5 Eingehende Aktivität:** Anfrage/Entscheidung über den Signal-Kanal oder ein eingehender Stream im
  Ruhemodus meldet „Aktivität“ (gedrosselt ≥ 5 s) → Ereignis `wake` → Kotlin hält 15 s einen Partial-Wakelock
  (FGS-Apps dürfen das im Doze) und der Share-Poller fragt sofort ab → Benachrichtigung ohne 60-s-Verzug.
- **A6 Periodisch bleibt periodisch:** hält der Erreichbarkeitsdienst den Prozess im Modus „Periodisch“ wach,
  laufen geplante Jobs im Hintergrund weiterhin nur beim Worker-Lauf (`sys.hostState.deferScheduling`);
  bei sichtbarer App wie bisher.
- **A7 Anzeige:** Benachrichtigung „Share erreichbar“ (Kanal „Hintergrund“, niedrig) bzw. Dauerbetrieb wie
  bisher; Einstellungen → Hintergrund: Schalter A1 mit Hinweis; Share-Seite/Einstellungen: Ruhemodus-Status des
  Servers.

## B Speicheranalyse

- **B1 Geschützte Bereiche:** `<Volume>/Android/data` und `<Volume>/Android/obb` (fremde App-Ordner) sind
  Plattform-Grenzen: keine Lesefehler mehr, sondern gezählte „geschützte“ Auslassungen mit einem Hinweis; das
  Ergebnis ist „vollständig“, wenn sonst nichts fehlte. Gilt für lokale Analyse, Share-gehostete Analyse
  (Desktop analysiert das Telefon) und Duplikatsuche.
- **B2 Größen trotzdem zeigen:** Kotlin liefert `platform {volumeUsedBytes, otherAppsBytes?}`:
  `StatFs` (ohne Recht) und `ExternalStorageStats.getAppBytes()` (Nutzungszugriff, optional). Unter
  `Android/data` erscheint „Andere Apps (geschützt)“ = otherAppsBytes − eigener erfasster Teil; bei einer
  Volume-Wurzel zusätzlich „Nicht einzeln erfasst (Apps, System)“ = belegt − erfasst − andere Apps (≥ 0).
  Ohne Nutzungszugriff fällt „Andere Apps“ in den Rest. `obb` hat keine eigene API-Größe (nur im Rest).
- **B3 Nutzungszugriff** optional: Karte im Analyse-Ergebnis „App-Ordner-Größen anzeigen“ → Einstellungen;
  ab Android 15 vorher „Eingeschränkte Einstellungen zulassen“ (App-Info ⋮) für seitlich geladene Apps.
- **B4 Tempo:** Analyse-Threads auf Android = min(Kerne, 8) statt 2 (MediaProvider-FUSE: libfuse 3.10
  unbegrenzte bzw. 16: 10 Worker; readdirplus füllt den Attribut-Cache, das stat danach kostet keinen
  Aufruf); Fortschritt mit Ordnerzahl und aktuellem Ordner.
- **B5 Duplikate:** alle Dateien ≥ Mindestgröße sind Kandidaten (nur Größen mit ≥ 2 Dateien werden geprüft;
  Grenze ist das bestehende Walk-Budget), Anfang/Ende- und Volltext-Vergleich parallel mit Hardware-SHA-256
  (`ring`), Fortschritt in Phasen, alle Gruppen, Fehler und Walk-Grenze sichtbar.

## Nicht-Ziele / Grenzen

- Kein FCM/UnifiedPush (Google-Dienste bzw. Fremd-App nötig; eigener Server hält die Verbindung ohnehin).
- Fremde `Android/data`-Inhalte bleiben unlesbar (Shizuku: TODO AND3).
- Echter Akkuverbrauch lässt sich nur am Gerät messen; die Suite prüft Weckhäufigkeit und Verhalten.
- Desktop-Aufräumen behält vorerst die 200er-Kandidatengrenze (TODO).

## Erhaltenes Verhalten (Abnahme)

Desktop-Share (Signal, Relay, Sitzungen) unverändert; Daemon-Stopp/Übergabe auf Desktop; Sync-Modi Aus/
Periodisch/Dauerbetrieb inkl. Boot-Start; Pairing/Anfragen/Exec; Analyse-Ergebnisse außerhalb geschützter
Bereiche identisch; Duplikat-Hashes identisch (SHA-256); Share-gehostete Analyse für Desktop.
