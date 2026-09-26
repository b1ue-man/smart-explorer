# Kern-API (Kotlin ↔ Rust) – verbindlicher Vertrag

Gilt für `native/src/mobile/` (Rust-Fassade), `native/android-bridge/` (JNI, Workspace-Mitglied) und
`android/app/src/main/java/app/smartexplorer/android/core/` (Kotlin-Aufrufer). Abweichungen nicht still
einbauen, sondern melden (umsetzung.md, Schnittstellenregel).

## 1 Transport

JNI-Klasse `app.smartexplorer.android.core.NativeBridge` (Kotlin `object`), Bibliothek
`libsmart_explorer_android.so` (`System.loadLibrary("smart_explorer_android")`):

| Kotlin | Rust (Brücke) | Rust (Fassade) |
|---|---|---|
| `external fun init(context: Context, configJson: String): String` | `Java_app_smartexplorer_android_core_NativeBridge_init` | `mobile::init(config: &str) -> String` |
| `external fun call(method: String, argsJson: String): String` | `Java_app_smartexplorer_android_core_NativeBridge_call` | `mobile::call(method: &str, args: &str) -> String` |
| `external fun pollEvents(timeoutMs: Int): String` | `Java_app_smartexplorer_android_core_NativeBridge_pollEvents` | `mobile::poll_events(timeout: Duration) -> String` |

- Alle Strings UTF-8-JSON. `call` ist synchron und darf Sekunden dauern (Kotlin ruft auf
  `Dispatchers.IO`). Alles, was länger als ~2 s dauern kann oder Fortschritt hat, ist ein Task (§3).
- Antwort von `init` und `call`: `{"ok": <Wert>}` oder `{"err": {"kind": "<Art>", "message": "<Text>"}}`.
  `kind` ∈ `not_found, permission, exists, invalid, unsupported, network, auth, conflict, busy,
  canceled, not_initialized, internal`. `message` ist deutscher Anzeigetext (wie Desktop).
- Brücke: jede Funktion fängt Paniken und Fehler und liefert dann
  `{"err":{"kind":"internal","message":"…"}}`; sie wirft nie eine Java-Exception.
- `pollEvents` liefert `{"ok": [Event, …]}` (leeres Array bei Timeout, höchstens 256 Ereignisse).
- `init` darf mehrfach aufgerufen werden (zweiter Aufruf aktualisiert nur Volumes/Host-Zustand).
- `init` blockiert nie auf Netz oder Daemon: Konfiguration, Logger, Laufzeit; der eingebettete Daemon
  wird angestoßen, aber nicht abgewartet (Warten nur in `bg.ensureDaemon`, `share.*`, `bg.catchUp`).
  Kotlin ruft `init` auf einem eigenen Thread (`Core.ready`).
- Keine Umgebungsvariablen als Konfigurationskanal (kein `set_var`): alle Host-Werte gehen über
  typisierte Setter (`support_dirs::set_host`, `tempfile::env::override_temp_dir`).

`init`-Konfiguration:
```json
{"filesDir":"/data/user/0/<pkg>/files","cacheDir":"/data/user/0/<pkg>/cache",
 "noBackupDir":"/data/user/0/<pkg>/no_backup","appVersion":"0.5.163","versionCode":5163,
 "deviceName":"Pixel 8","homeDir":"/storage/emulated/0","bootMarker":"17",
 "updateFeedUrl":null,"startDaemon":true,
 "volumes":[{"path":"/storage/emulated/0","label":"Interner Speicher","primary":true,"removable":false}]}
```
`appVersion` = `PackageInfo.versionName`; `bootMarker` = `Settings.Global.BOOT_COUNT` als Text (für
„Beim Start“-Jobs); `updateFeedUrl` und `startDaemon=false` nur für Tests (Debug-Build liest
`filesDir/test-overrides.json`; sonst eingebauter Feed aus `native/update_source.txt`).
`homeDir` muss, wenn gesetzt, ein absoluter Pfad ohne NUL sein (sonst `invalid`); ohne `homeDir`
und ohne Volumes ist Home und Direct-Standardfreigabe der leere Ordner `<filesDir>/home`, nie das
private `filesDir` selbst.
Cache-Unterordner (auch in `file_paths.xml` deklariert): `open/` (Remote-Kopien zum Öffnen),
`share/` (Kopien zum Teilen), `update/` (APK-Download), `tmp/` (Temp-Wurzel).
Antwort: `{"ok":{"coreVersion":"0.5.163","dataDir":"…/files/smart_explorer"}}`.
Rust-Initialisierung (Reihenfolge verbindlich, erster `init`): `support_dirs::set_host` (Daten-,
Cache-, Home-Verzeichnis, Gerätename, Boot-Marke) + `tempfile::env::override_temp_dir(cache)` →
`ndk_context` → `rustls-platform-verifier` (beides in der Brücke, vor `mobile::init`) → Panik-Hook
(Crash-Log) → Laufzeit → Anstoß des eingebetteten Daemons (ohne Warten, wenn `startDaemon`) →
Share-Poller.

## 2 Gemeinsame Typen (JSON, camelCase)

`location`: undurchsichtige Zeichenkette = Desktop-Endpunkt (`EndpointSpec`): lokaler Pfad
(`/storage/emulated/0/DCIM`), `sftp://user@host:22/pfad`, `ftp://…`, `ftps://…`, `webdav://…`,
`gdrive:///pfad`, `share://…`; zusätzlich App-intern `zip://<lokaler zip-pfad>!/<innen>`,
`trash://` (Papierkorb). Kotlin baut nie selbst Orte zusammen, außer über Felder aus Antworten.
Namen bleiben wörtlich: nur führende Leerzeichen eines Orts werden ignoriert, `…/Bericht ` und
`…/Bericht` sind verschiedene Orte. In `zip://<archiv>!/<innen>` endet das Archiv am ersten `!/`
(oder abschließenden `!`) nach einem Namen auf `.zip`, sodass Ordner mit `!` am Namensende gehen.
App-interne Orte (`zip://`, `trash://`) werden von `loc.toggleFavorite`, „Zuletzt“, `sync.validate`,
`sync.save`, `sync.mirror`, `share.addExport` und dem Zielauswahl-Dialog mit `invalid` abgelehnt;
sie gelangen nie in Desktop-Formate. Favoriten liegen im Desktop-Format `favorites.txt`
(Schlüssel wie `location_key`), damit das Aufräumen beim Entfernen von Verbindungen greift.

```text
Entry     {name, location, isDir, isLink, size:Long, mtimeMs:Long, hidden:Boolean,
           problem:String?  (Warntext für problematische Namen), kind:String
           (dir|image|video|audio|text|archive|document|apk|other), ext:String,
           depth:Int (0 = direkt im Ordner; >0 nur in Scan-Ansichten), hasChildren:Boolean,
           expanded:Boolean}
Crumb     {label, location}
Root      {id, label, subtitle:String?, location, kind:String
           (storage|favorite|recent|connection|gdrive|device|room|trash), removable:Boolean}
Filter    {text, mode:"substring|glob|regex", extensions:String, sizeMin:Long?, sizeMax:Long?,
           mtimeMinMs:Long?, mtimeMaxMs:Long?, files:Boolean, dirs:Boolean, hidden:Boolean,
           problemOnly:Boolean}
Sort      {key:"name|size|mtime|type", desc:Boolean, dirsFirst:Boolean}
Task      {id, kind, title, state:"queued|running|done|failed|canceled", doneBytes:Long,
           totalBytes:Long, doneItems:Long, totalItems:Long, rateBps:Long, message:String?,
           errors:[{path, message}], result:Json?, startedMs:Long, finishedMs:Long?}
```
`Task.kind` ∈ `transfer, delete, scan, properties, open, upload, materialize, extract, index,
sync, mirror, analyze, reclaim, trash, oauth, share, exec, update`.

## 3 Tasks und Ereignisse

- `task.list {}` → `[Task]` (laufende + fertige bis `task.clear`)
- `task.cancel {id}` → `{}` · `task.cancelAll {kind?}` → `{}` · `task.clear {ids:[String]?}` → `{}`
  (entfernt Fertige; mit `ids` nur diese – „Leeren“ der Übertragungen lässt Scans und Analysen stehen)
- `task.get {id}` → `Task`

Ereignisse (`pollEvents`):
```text
{"type":"task","task":Task}          Fortschritt (gebündelt, höchstens 4/s je Task) und Ende
{"type":"share"}                     Share-Zustand geändert → share.status neu laden
{"type":"shareRequest","count":N}    neue eingehende Anfrage(n) → Badge/Benachrichtigung
{"type":"edits"}                     geöffnete Remote-Kopie wurde lokal geändert → fs.edits
{"type":"jobs"}                      Sync-Jobs/Ergebnisse geändert → sync.jobs neu laden
{"type":"openUrl","url":"https://…"} Kern möchte eine URL im Browser öffnen (OAuth)
{"type":"error","action":"…","message":"…"}   Eintrag für das Fehlerprotokoll
{"type":"volumes"}                   Speicherorte geändert
```

## 4 Methoden

### 4.1 System (`sys.*`)
- `sys.hostState {powerSave:Boolean, metered:Boolean, wifi:Boolean, charging:Boolean, foreground:Boolean}` → `{}`
- `sys.volumes {volumes:[{path,label,primary,removable}]}` → `{}`
- `sys.errors {}` → `[{timeMs, action, message}]` · `sys.clearErrors {}` → `{}`
- `sys.crashLog {}` → `{text}` (leer, wenn keins)
- `sys.info {}` → `{coreVersion, dataDir, cacheDir}`

### 4.2 Orte (`loc.*`)
- `loc.roots {}` → `{storage:[Root], favorites:[Root], recent:[Root], connections:[Root],
  gdrive:Root?, devices:[Root], rooms:[Root], trash:Root}`
- `loc.toggleFavorite {location}` → `{favorite:Boolean}`
- `loc.isFavorite {location}` → `{favorite:Boolean}`

### 4.3 Dateien (`fs.*`)
- `fs.list {location, showHidden:Boolean, filter:Filter?, sort:Sort}` →
  `{location, title, crumbs:[Crumb], parent:String?, backend:"local|sftp|ftp|ftps|webdav|smb|gdrive|share|zip|trash",
    readOnly:Boolean, canTrash:Boolean, entries:[Entry], totalBytes:Long}`
  (nicht rekursiv; Filter mit Desktop-`CompiledFilter`; trägt `location` in „Zuletzt“ ein)
- `fs.stat {location}` → `Entry`
- `fs.checkName {parent, name}` → `{problem:String?, exists:Boolean}`
- `fs.mkdir {parent, name}` → `Entry` · `fs.newFile {parent, name}` → `Entry`
- `fs.rename {location, newName}` → `Entry`
- `fs.conflicts {sources:[location], targetDir}` → `{names:[String], choosable:Boolean}` (Namen, die im
  Ziel existieren; `choosable` nur bei lokal→lokal – Remote-Übertragungen nummerieren belegte Namen
  immer wie am Desktop, „Name (2).ext“)
- `fs.transfer {sources:[location], targetDir, mode:"copy|move", conflict:"skip|replace|keepBoth",
  filter:Filter?, baseDir:String?}` → `{taskId}`
  (mit `filter` + `baseDir`: nur passende Dateien mit Pfaden relativ zu `baseDir`, wie Desktop;
  `move` nur lokal→lokal, sonst Fehler `unsupported` „Verschieben von/zu Remote wird nicht
  unterstützt“ wie am Desktop; `conflict` wirkt nur lokal→lokal)
- `fs.delete {locations:[location], permanent:Boolean}` → `{taskId}` (nicht permanent = Papierkorb;
  Orte ohne Papierkorb → `unsupported`, Kotlin fragt dann „Endgültig löschen?“)
- `fs.properties {locations:[location]}` → `{taskId}`; `result = {items, files, dirs, bytes,
  mtimeMs?, btimeMs?, location?}`
- `fs.open {location}` → `{localPath, mime}` (nur lokale Orte; sonst `unsupported`; Kotlin öffnet über
  den eigenen `LocalFileProvider`, Schreiben der Fremd-App trifft das Original)
- `fs.fetch {location}` → `{taskId}`; `result = {localPath, mime, editId}` (Remote in
  `<cache>/open/<editId>/` laden; Register dauerhaft in `<data>/mobile/edits.json`, beim `init` geladen;
  ist es voll, fallen die ältesten unveränderten Kopien heraus, `busy` nur bei 100 geänderten Kopien;
  ein unlesbares Register liefert bei `fs.fetch`/`fs.edits`/`fs.discardEdit` einen Fehler und wird nie
  überschrieben)
- `fs.materialize {locations:[location]}` → `{taskId}`; `result = {paths:[String]}` (Remote-Kopien in
  `<cache>/share/`; lokale Orte liefern ihre Pfade unverändert)
- `fs.edits {}` → `[{editId, name, location, localPath, modified:Boolean}]`
- `fs.uploadEdit {editId, mode:"overwrite|copy", force:Boolean?}` → `{taskId}` (bei `overwrite` und
  seit dem Laden oder letzten Hochladen geänderter Remote-Zeit – auch einer älteren – endet der Task mit
  `failed`, `message`, `result={conflict:true}`; ein erneuter Aufruf mit `force:true` überschreibt nach
  ausdrücklicher Bestätigung – wie erneutes Speichern nach der Konfliktmeldung am Desktop; SFTP ersetzt
  atomar per `posix-rename@openssh.com`, WebDAV per einem `MOVE` mit `Overwrite: T`, FTP per
  `RNFR`/`RNTO`; ein Server ohne sicheres Ersetzen lässt den Task mit dessen Fehler enden; eine Kopie
  aus einer ZIP liefert sofort `permission`)
- `fs.discardEdit {editId}` → `{}`
- `fs.import {files:[{fd:Int, name, size:Long?}], targetDir}` → `{taskId}` (Kotlin übergibt je geteiltem
  Inhalt einen per `ParcelFileDescriptor.detachFd()` gelösten Deskriptor; Rust übernimmt und schließt
  ihn, liest ihn als Task mit Fortschritt direkt ins Ziel – lokal oder remote; belegte Namen →
  „Name (2)“)
- `fs.extract {location, targetDir:String?}` → `{taskId}` (ZIP; `null` = Ordner neben dem Archiv;
  verschlüsselte, nicht unterstützte oder unlesbare Einträge werden mit Grund ausgelassen, der Rest
  wird entpackt)

### 4.4 Rekursiver Scan und Ordnersuche (`scan.*`, `index.*`)
- `scan.validate {filter}` → `{error:String?}`
- `scan.start {location, filter:Filter?, showHidden}` → `{taskId}` (rekursiv, Filter beim Scannen,
  Fortschritt: `doneItems` = durchsucht, `totalItems` = Treffer; `message` bei Grenze/Lesefehlern)
- `scan.view {taskId, sort:Sort, collapsed:[location], offset:Int, limit:Int (≤ 500), sinceRevision:Long?}`
  → `{revision:Long, unchanged:Boolean, entries:[Entry], visibleTotal, matches, scanned,
  truncated:Boolean, issues:Int}` (Baumreihenfolge, `depth`, `hasChildren`, `expanded`; Fenster ab
  `offset`; `unchanged=true` und leere `entries`, wenn `sinceRevision` aktuell ist und dasselbe Fenster
  – Sortierung, `offset`, `limit`, `collapsed` – wie beim vorigen Aufruf angefragt wird; während des Scans
  fragt Kotlin höchstens 1/s)
- `scan.issues {taskId}` → `{text}`
- `index.status {}` → `{state:"none|building|ready", count:Int}` · `index.build {}` → `{taskId}`
- `index.search {query, limit:Int}` → `[{name, path, location, score:Int}]`

### 4.5 Papierkorb (`trash.*`)
- `trash.list {}` → `[{id, name, originalLocation, deletedMs, size, isDir}]`
- `trash.restore {ids:[String]}` → `{restored:Int, renamed:Int}` (belegter Zielname → „Name (2)“)
- `trash.delete {ids:[String]}` → `{taskId}` · `trash.empty {}` → `{taskId}`
- `trash.purge {olderThanDays:Int}` → `{removed:Int}`

### 4.6 Verbindungen (`conn.*`) und Google Drive (`gdrive.*`)
```text
Connection {id, label, protocol:"sftp|ftp|ftps|webdav|smb", host, port:Int, user, root,
            auth:"password|key", keyPath:String?, useAgent:Boolean, https:Boolean, location}
ConnectionInput = Connection ohne id/location, plus password:String?, passphrase:String?, id:String?
```
- SMB (SMB2/3, Standardport 445): `root` beginnt mit der Freigabe (`/<freigabe>/<pfad>`), eine
  Domäne steht als `DOMÄNE\benutzer` im Feld `user`; nur `auth:"password"`; `location` =
  `smb://user@host:port/<freigabe>/<pfad>`; Listing-`backend` = `"smb"`. Ersetzen ist ein Umbenennen
  mit `ReplaceIfExists` auf dem Server, neue Dateien werden exklusiv angelegt.
- `conn.list {}` → `[Connection]`
- `conn.test {input:ConnectionInput}` → `{message}` (Fehler mit `kind` auth/network/…)
- `conn.save {input:ConnectionInput}` → `Connection` (leeres `password` bei Bearbeitung = unverändert)
- `conn.delete {id}` → `{removedFavorites:Int, orphanedJobs:[String]}` (Jobs werden wie am Desktop nur
  gemeldet, nicht gelöscht)
- `conn.forgetHostKey {id}` → `{}`
- `gdrive.status {}` → `{clientConfigured:Boolean, signedIn:Boolean, clientId:String?}`
- `gdrive.configure {clientId, clientSecret:String?}` → `{}`
- `gdrive.signIn {}` → `{taskId}` (sendet `openUrl`; Task endet nach Rückmeldung oder 180 s)
- `gdrive.signOut {}` → `{}`

### 4.7 Sync (`sync.*`) und Hintergrund (`bg.*`)
```text
Job {id, name, source, target, direction, conflict, deletePolicy, compare, versioning,
     retainDays:Int, trigger, intervalMin:Int, calendar:{kind, minuteOfDay:Int, weekday:Int,
     monthday:Int}?, rtDebounceSecs:Int, includeHidden:Boolean, ignore:[String], enabled:Boolean,
     runBefore:String, runAfter:String, lastRun:Long, activeFromMin:Int, activeToMin:Int,
     catchUp:Boolean, moveFiles:Boolean, maxDelete:Int, maxDeletePct:Int, useRecycleBin:Boolean,
     lastResult:{timeMs, aToB:Int, bToA:Int, deleted:Int, conflicts:Int, errors:Int, note}?,
     schedule:String (deutsche Beschreibung des Auslösers, nur Ausgabe),
     runningTask:String?}
```
Aufzählungswerte (`direction`, `conflict`, `deletePolicy`, `compare`, `versioning`, `trigger`,
`calendar.kind`) sind die `as_str()`-Werte der Rust-Enums; `sync.options` liefert sie mit Anzeigetext.
- `sync.options {}` → `{directions:[{value,label}], conflicts:[…], deletePolicies:[…], compares:[…],
  versionings:[…], triggers:[…], calendarKinds:[…], defaults:Job}`
- `sync.jobs {}` → `[Job]`
- `sync.validate {job}` → `{errors:{<feld>:<text>}}` (Desktop-Validierung)
- `sync.save {job}` → `Job` (leere `id` = neu) · `sync.delete {id}` → `{}`
- `sync.setEnabled {id, enabled}` → `Job`
- `sync.run {id}` → `{taskId}` (`bisync::run` meldet keinen Fortschritt → Task ohne Prozent, Text
  „Synchronisiere…“; `result = {summary, aToB:Int, bToA:Int, deleted:Int, conflicts:Int, errors:Int,
  omitted:String?}`; schreibt `last_run`/Ergebnis wie der Desktop; keine Vorher/Nachher-Befehle –
  die laufen wie am Desktop nur im Hintergrundlauf)
- `sync.mirror {source, target}` → `{taskId}` (`crate::sync::start_sync` über
  `crate::vfs::sync_backend(…)`-Hüllen wie am Desktop, `delete_extra=false`: einseitig kopieren,
  im Ziel wird nichts gelöscht; Fortschritt; `result = {copied, skipped, errors, omitted:String?}`;
  nichts wird gespeichert)
Konflikte werden am Desktop nicht dauerhaft gespeichert, sie leben im Ergebnis des letzten Laufs.
Die Fassade hält je Job den Kontext des letzten Laufs (Backends, Wurzeln, Pair-ID, Baseline,
Konfliktliste) im Speicher; fehlt er (App neu gestartet, Hintergrundlauf), ermittelt
`sync.checkConflicts` die Konflikte per Probelauf (`dry_run`).
- `sync.conflicts {id}` → `{available:Boolean, items:[{cid, path, a:{exists,size,mtimeMs}?,
  b:{exists,size,mtimeMs}?, text:Boolean}]}` (`available=false` → erst `sync.checkConflicts`)
- `sync.checkConflicts {id}` → `{taskId}` (Probelauf, füllt den Kontext)
- `sync.resolve {id, cid, choice:"a|b"}` → `{taskId}` (`resolve_checked`, Baseline wird gespeichert,
  sobald keine Konflikte mehr offen sind oder `sync.finishConflicts` gerufen wird)
- `sync.skip {id, cid}` → `{}` (nur für diese Sitzung, wie Desktop; war es der letzte offene Konflikt,
  wird die Baseline gespeichert – ein Speicherfehler kommt als `internal`, der Konflikt bleibt
  übersprungen)
- `sync.finishConflicts {id}` → `{}` (speichert eine geänderte Baseline; Fehler → erneut versuchen)
- `sync.mergeRows {id, cid}` → `{rows:[{a:String?, b:String?, equal:Boolean, takeA:Boolean,
  takeB:Boolean}]}` (Textdateien ≤ 16 MiB je Seite, `linemerge::rows`)
- `sync.mergeApply {id, cid, rows:[{takeA, takeB}…]}` → `{taskId}` (schreibt das Ergebnis auf beide Seiten)
- `sync.mergeKeepBoth {id, cid}` → `{taskId}` (A unter Originalname, B als „(Konflikt …)“ auf beiden Seiten)
Hintergrund: genau ein eingebetteter Desktop-Daemon je App-Prozess (Thread), beim ersten Bedarf
gestartet, nie wegen Sichtbarkeit gestoppt. „Aus“ wirkt über das Sync-Flag (Android-Adapter von
`autostart::is_enabled`) und `share.setOnline`.
- `bg.ensureDaemon {}` → `{running:Boolean}` (idempotent; wartet bis zu 10 s auf Bereitschaft)
- `bg.status {}` → `{syncEnabled, daemonRunning, heartbeatAgeSecs:Long?, paused, pausedUntilMs:Long?,
  autopauseBattery, autopauseMetered, cadenceSecs:Int, catchUpRunning:Boolean, lastCatchUpMs:Long?,
  activeJob:String?}`
- `bg.setSyncEnabled {enabled}` → `{}`
- `bg.pause {seconds:Long}` (−1 = unbegrenzt) → `{}` · `bg.resume {}` → `{}`
- `bg.setAutopause {battery, metered}` → `{}`
- `bg.log {maxBytes:Int}` → `{text}`
- `bg.catchUp {}` → `{taskId}` (Task-`kind` = `catchup`; Nachhol-Lauf des Daemons: fällige Intervall-Jobs,
  Kalender-Termine seit dem letzten Lauf – im Hintergrund immer nachgeholt –, aktivierte Echtzeit-Jobs
  einmal; der Task endet, wenn alle für **diesen** Lauf zugelassenen Jobs fertig sind – ein Job, den der
  reguläre Plan schon hält, wird abgewartet und mitgezählt, bleibt aber dessen Job; kürzlich versuchte
  und nicht startbare stehen mit Grund in `result.skipped`; `task.cancel` bricht nur die eigenen Jobs
  dieses Laufs ab; pausiert/Sync aus → Task endet sofort mit `message`)

### 4.8 Share (`share.*`)
Siehe §5.

### 4.9 Analyse (`analyze.*`, `reclaim.*`)
- `analyze.start {location}` → `{taskId}` (Fortschritt: `doneItems` Dateien, `doneBytes`)
- `analyze.node {taskId, path:[String]}` → `{name, size, isDir, children:[{name, size, isDir,
  childCount:Int}], location:String?}` (Kinder nach Größe absteigend, höchstens 500)
- `analyze.issues {taskId}` → `{count:Int, text}`
- `reclaim.start {location, minSize:Long}` → `{taskId}`
- `reclaim.groups {taskId}` → `[{size, items:[{location, mtimeMs}]}]`
- Löschen der gewählten Kopien über `fs.delete` (Papierkorb).

### 4.10 Update (`update.*`)
- `update.check {}` → `{current, latest, available:Boolean, notes:String?}` (`current` =
  `appVersion` aus der Init-Konfiguration, also `PackageInfo.versionName`)
- `update.download {}` → `{taskId}`; `result = {path, version}` (nach `<cache>/update/`, SHA-256 geprüft;
  `busy`, solange ein früherer Download-Task noch lebt)

## 5 Share (`share.*`)
Grundlage: `docs/lesungen/2026-09-25-android-share-facade-map.md` (Rezepte 1–9). Der Share-Dienst läuft
im eingebetteten Daemon; die Fassade nutzt dieselben IPC-Client-Funktionen wie die Desktop-GUI
(`daemon::drain_share_worker_events`, `send_share_command`, `refresh_share_worker_checked`,
`open_share_backend`, `exec_share`) und dieselben `ShareProfiles::*_persisted`-Funktionen; freie
Profiländerungen laufen über `mutate_persisted` + `merge_user_edits` (egui-freie Logik aus
`app/core/share_profile_edits.rs`, nach `share/os/shared/` verschoben). `default_home` = `homeDir`
aus der Init-Konfiguration (primärer Speicher, Desktop-Parität „Home“ → Standardfreigabe für
Direkt-Geräte), Gerätename ebenso. Die Laufzeit holt Worker-Ereignisse selbst **im Prozess** am
eingebetteten `ShareHost` ab (kein TCP): alle 300 ms, solange die Teilen-Seite sichtbar ist oder ein
Pairing läuft (`share.watch`), sonst alle 5 s bei sichtbarer App und alle 60 s im Hintergrund
(`sys.hostState.foreground`); sie hält den letzten Snapshot und sendet `share`/`shareRequest`.
```text
ShareStatus {running, connected, relayUrl:String?, lastError:String?, server:String?,
  lanPresence:String, identity:{deviceId, deviceName, fingerprint, directCode},
  devices:[{contactId, name, status, statusText, online:Boolean, location, lan:Boolean}],
  execProvider:{available:Boolean, provider, detail},
  execTargets:[{targetKey, relation:"direct|room", roomId:String?, roomName:String?, deviceId, name,
                fingerprint, enabled:Boolean, baseAuthorized:Boolean, policyRevision:Long}],
  rooms:[{profileId, roomId, name, status, autoJoin, location:String?,
          members:[{deviceId, name, status, location, blocked:Boolean}]}],
  incoming:[Request], outgoing:[Request],
  exports:{direct:[{label, path}], rooms:{<profileId>:[{label, path}]}},
  discovery:{offer:{offerId, target, alias, untilMs}?, advertisements:[{discoveryId, kind:"direct|room",
             alias, expiresMs, compatible:Boolean}], exchange:{exchangeId, state:"running|done|failed|canceled",
             message:String?}?},
  removedDevices:[{deviceId, name}], notices:[String]}
Request {requestId, contactId:String?, name, stateText, canAccept, canReject, canRetry, canDelete,
  message:String?, timeMs:Long}
```
- `share.status {}` → `ShareStatus` (letzter Snapshot des Pollers; billig)
- `share.watch {active:Boolean}` → `{}` (Teilen-Seite sichtbar → schneller Takt)
- `share.setServer {server}` → `{}` (Desktop-Validierung; leer = Share-Server entfernen, dann nur LAN)
- `share.setOnline {online}` → `{}` (`auto_connect`)
- `share.setName {name}` → `{}`
- `share.discoverable {target:"direct"|<roomProfileId>, alias, pin, minutes}` → `{}` ·
  `share.stopDiscoverable {offerId}` → `{}`
- `share.discover {}` → `{}` (Ergebnisse erscheinen in `discovery.advertisements`)
- `share.connect {discoveryId, pin}` → `{}` · `share.cancelConnect {exchangeId}` → `{}`
- `share.addDirect {code, name}` → `{contactId}` (Direct-Code wie am Desktop)
- `share.removeDevice {contactId}` → `{removedFavorites:Int, orphanedJobs:[String]}`
- `share.readmit {deviceId}` → `{}`
- `share.createRoom {name}` → `{profileId, code}` · `share.joinRoom {code, name}` → `{profileId}` ·
  `share.roomCode {profileId}` → `{code:String?}` · `share.leaveRoom {profileId}` → `{}` ·
  `share.removeRoom {profileId}` → `{removedFavorites:Int, orphanedJobs:[String]}`
- `share.requestAccess {contactId, message:String?}` → `{}` · `share.decide {requestId, accept:Boolean}`
  → `{}` · `share.retry {requestId}` → `{}` · `share.deleteRequest {requestId}` → `{}`
- `share.addExport {scope:"direct"|<profileId>, path, label:String?}` → `{}` ·
  `share.removeExport {scope, path}` → `{}` (Pfad muss existierendes Verzeichnis sein)
- `share.exec {location, command, shell:Boolean, timeoutSecs:Int}` → `{taskId}`;
  `result = {stdout, stderr, exitCode:Int?, timedOut, truncated}` (`daemon::exec_share`, Ausgabe ≤ 1 MiB;
  `shell:false` trennt an Leerraum, Anführungszeichen gruppieren, ein Backslash ist wörtlich außer vor
  Leerraum oder Anführungszeichen; eine verweigerte Freigabe kommt als `permission`)
- Exec-Host (dieses Gerät führt Befehle anderer Geräte aus; Anbieter `android-subreaper`: jeder Befehl
  läuft unter einem eigenen Subreaper-Zwischenprozess mit `/system/bin/sh`, Abbruch, Zeitlimit und
  Entzug beenden den ganzen Prozessbaum; Arbeitsordner ohne `cwd` = `homeDir`). Freigaben gelten wie am
  Desktop je exakter Identität; `execTargets` entsteht wie `exec_device_views` (Direkt-Freigaben und
  Raummitglieder), Schlüssel `direct/<deviceId>/<fingerprint>` bzw. `room/<roomId>/<deviceId>/<fingerprint>`:
  - `share.setExec {targetKey, enabled:Boolean}` → `{revision:Long}` (`daemon::mutate_exec_grant`,
    Journal wie am Desktop; Schlüssel gegen den aktuellen Profilstand aufgelöst, sonst `not_found`;
    nur vollständig gespeichert **und** angewendet gilt als Erfolg, sonst Fehler mit Detail;
    `enabled:true` bei nicht verfügbarem Anbieter → `unsupported` mit Grund)
  - `share.execJobs {}` → `{active:[ExecJob], history:[ExecJob]}` mit `ExecJob {direction:"incoming|outgoing",
    execId, peerDeviceId, peerName, program, state, startedAt:Long?, finishedAt:Long?, exitCode:Int?,
    message:String?}`; `state` = Lebenszyklus in snake_case (`running`, `exited`, `cancelled`,
    `timed_out`, `revoked`, …), Zeiten in Sekunden seit 1970
  - `share.cancelExecJob {direction, execId, peerDeviceId}` → `{}` (nicht mehr aktiv → `not_found`)
Nicht auf Android: LAN-Uplink, Anfragen im Altformat.
