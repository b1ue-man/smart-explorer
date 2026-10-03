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
  canceled, not_initialized, internal, weak_pin`. `message` ist deutscher Anzeigetext (wie Desktop).
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
und ohne Volumes ist Home der leere Ordner `<filesDir>/home`, nie das private `filesDir` selbst.
Neue Share-Profile und Räume haben keine Standardfreigabe. Home dient weiterhin als Arbeitsordner
und als genaue Ortsidentität für die einmalige Migration alter automatischer Home-Freigaben (§5).
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
Backend, Verbindung und Präfix sind Teil der Identität: gleiche relative Pfade auf verschiedenen
Konten sind unterschiedliche Orte. Remote-Scanner, Picker und Duplikat-Papierkorb erhalten den
vollständigen Locator einschließlich Share-Direct-/Raumidentität; kein lokaler Ersatzpfad.
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
- `analyze.start`, `reclaim.start` und `reclaim.recycle` melden additiv `remote:Boolean` neben
  `taskId`. Für `remote:true` hält Kotlin die CPU während dieses Tasks wach
  (`TaskForegroundService`, `TaskKeeper.keepCpuAwake`, Partial-Wakelock `SmartExplorer:remote-task`;
  zehn Minuten mit Erneuerung alle 60 Sekunden und Freigabe, sobald kein solcher Task mehr läuft).
  Task-Ende und Ergebnisaufbewahrung sind getrennt: `analyze.release`/`reclaim.release` (§4.9)
  geben aufbewahrte Daten frei, ohne laufende Tasks als erfolgreich auszugeben.

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
{"type":"wake","ms":N}               Kern braucht die CPU N ms wach (Server-Keepalive, eingehende Anfrage
                                     oder Streams im Ruhemodus) → Partial-Wakelock, längere Anforderung gewinnt
```

## 4 Methoden

### 4.1 System (`sys.*`)
- `sys.hostState {powerSave:Boolean, metered:Boolean, wifi:Boolean, charging:Boolean, foreground:Boolean,
  deferScheduling:Boolean}` → `{}` (fehlende Felder = false). `foreground:false` schaltet Share in den
  Ruhemodus (`share::power::set_low_power`); `deferScheduling:true` hält geplante Jobs (Start, Zeitplan,
  Echtzeit, Anschluss) zurück, laufende Jobs und `bg.catchUp` bleiben unberührt (Modus „Periodisch“ bei
  verdeckter App). Der eingebettete Daemon startet zurückgestellt, bis der erste `sys.hostState` kommt.
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
- `analyze.start {location, platform:{volumeUsedBytes:Long?, otherAppsBytes:Long?, apps:[{package, label,
  appBytes:Long, dataBytes:Long, cacheBytes:Long}]?}?}` → `{taskId, remote:Boolean}`
  (Fortschritt: `doneItems` Dateien, `doneBytes`, `message` = „N Ordner · aktueller Ordner“; `platform` nur
  für lokale Pfade: belegter Platz des Volumes der Wurzel per `StatFs`, `otherAppsBytes` =
  `ExternalStorageStats.getAppBytes()` des primären Volumes mit Nutzungszugriff; fehlend/negativ = unbekannt;
  `apps` nur für die ganze Wurzel des primären Volumes mit Nutzungszugriff: je Paket
  `StorageStatsManager.queryStatsForPackage` – `appBytes` = APK/Code inkl. eigenem `Android/obb`, `dataBytes`
  = Daten inkl. eigenem `Android/data`, `cacheBytes` = Cache-Anteil der Daten; Einträge ohne `package` werden
  übergangen, fehlende/negative Zahlen = 0, `cacheBytes` höchstens `dataBytes`, leeres `label` = Paketname,
  doppelte Pakete zählen einmal, Apps ohne Bytes erscheinen nicht; der Kern prüft „ganzes primäres
  Volume“ selbst und ignoriert `apps` sonst); `result = {files, dirs, bytes, issues, protected, notes}`.
  `<Volume>/Android/data|obb` und alles darunter sind geschützt: keine Issues, Status vollständig, eine
  geschützte Wurzel ergibt ein leeres, vollständiges Ergebnis
- `analyze.node {taskId, path:[String]}` → `{name, size, measured, isDir, kind, children:[{name, size, isDir,
  childCount:Int, kind}], location:String?, remote:Boolean, volumeTotal:Long?, volumeFree:Long?}`
  (`remote` Default false; Volumezahlen nur an der Wurzel und nur wenn bekannt;
  Kinder nach Größe absteigend, höchstens 500; `kind` =
  `dir|file|aggregate|protected|rest|apps|app`; Schätzzeilen nur in dieser Sicht: „Weitere App-Daten (laut
  Android, ≈)“ `protected` unter `Android/data` (nur ohne App-Liste), „≈ Nicht einzeln erfasst“ `rest` an einer
  ganzen Volume-Wurzel ohne andere Fehler; `size` der Vorfahren enthält sie, `measured` ist der gemessene Wert).
  Mit App-Liste an der Wurzel: „≈ Apps (laut Android)“ `kind:"apps"`, `isDir:true`, `childCount` = Zahl der
  Apps, `size` = Σ(`appBytes` + `dataBytes`); der Rest heißt dann „≈ System und Sonstiges“ =
  max(0, belegt − (gemessen − doppelt) − Σ Apps), wobei „doppelt“ = gemessene Bytes unter `Android/data` und
  `Android/obb` der Wurzel (höchstens Σ Apps), die auch in den App-Zahlen stecken; die Wurzel-`size` zählt sie
  einmal (= belegt, solange ein Rest bleibt), die Zeilen „Android“ und „≈ Apps“ enthalten sie beide.
  `path:["≈ Apps (laut Android)"]` (der Name dieser Zeile) → die App-Liste `{name, size, measured:0,
  isDir:true, kind:"apps", children, location:null}`; Kinder `kind:"app"`, `name` = App-Name, `isDir:false`,
  `size` = `appBytes` + `dataBytes`, dazu `package`, `appBytes`, `dataBytes`, `cacheBytes` (über 500 Apps:
  die kleinsten in einer `aggregate`-Zeile); tiefere Pfade → `not_found`. Ein echter Ordner gleichen Namens an
  der Wurzel wird dann von der App-Liste verdeckt
- `analyze.issues {taskId}` → `{count:Int, text, notes:[String], protectedCount:Long, protectedText}`
  (`notes` Default leer; `text` enthält nur Leseprobleme, Hinweise stehen getrennt in `notes` und
  erscheinen an der Wurzel auch ohne Leseprobleme; `count` ohne geschützte;
  `protectedText` kann auch bei 0 gefüllt sein, wenn Android fremde App-Ordner nur ausblendet)
- `analyze.release {taskId}` / `reclaim.release {taskId}` → `{released:Boolean}`: gespeichertes Ergebnis
  sofort freigeben, auch ein erst später fertig werdendes. `false` = keines aufbewahrt. Danach sind
  Analyse-Knoten/-Hinweise und Duplikatgruppen/-Summary `not_found`. Kotlin gibt das frühere Ergebnis
  bei neuer Suche/Analyse, „Anderer Ort“ und `onCleared` frei. Der Kern entfernt bei Speicherdruck
  die ältesten fertigen Ergebnisse zuerst; keine feste Vier-Plätze-Kappung.
- `reclaim.start {location, minSize:Long}` → `{taskId, remote:Boolean}` (lokal: jede Datei ≥ `minSize` ist Kandidat, Vergleich
  parallel mit SHA-256; Fortschritt `message` = Phase); `result = {groups, reclaimable, errors, candidates, protected}`
- `reclaim.groups {taskId}` → `[{size, items:[{location, mtimeMs}], contentVerified:Boolean}]`
  (`contentVerified` Default false, vollständiger SHA-256 als Inhaltsevidenz; MD5-/Providergruppen
  bleiben sichtbar, berechtigen jedoch nicht zum inhaltsgebundenen Fern-Papierkorb)
- `reclaim.summary {taskId}` → `{files, bytes, candidates, compared, groups, protectedCount, protectedText,
  errorCount, errorText, limit:String?, remote:Boolean, canRecycle:Boolean, recycleNote:String}`
  (`remote`/`canRecycle` Default false, `recycleNote` Default leer; `limit` = erreichte Walk-/Kandidaten-
  Grenze als unverändert sichtbarer Kerntext; `compared` wird angezeigt). Ohne bestätigtes
  `canRecycle` gibt es keine Papierkorbaktion. Lokale aktuelle Antworten liefern weiterhin true;
  bei verlorener Gegenstelle bleibt der Bericht abrufbar, Fähigkeit false samt Hinweis.
- `reclaim.recycle {taskId, locations:[String]}` → `{taskId, remote:true}`: Eingabe-ID = gespeichertes
  Duplikatergebnis, Ausgabe-ID = neue Aktion. Orte bleiben vollständig mit Backend-/Kontoidentität.
  Der Kern reserviert das Ergebnis, verlangt eindeutige bestätigte Pfade und eine verbleibende Kopie
  je Gruppe; jeder Host-Aufruf trägt erwartete Größe und SHA-256. Geänderter Inhalt wird nicht bewegt.
  Task-`result = {moved:Int}`; `reclaim.recycleResult {taskId}` → `{moved:[String]}` (Default leer)
  liefert auch bei Teilfehler/Abbruch exakt die erfolgreich verschobenen vollständigen Orte. Kotlin
  entfernt ausschließlich diese, danach `reclaim.release` für die Aktion. Der Kern aktualisiert
  auch die gespeicherten Gruppen, sodass eine wiederholte Auswahl die letzte Kopie nicht entfernt.
  Lokales Löschen verwendet weiterhin den bestehenden `fs.delete`-Papierkorbpfad.

Entfernte Analyse nutzt `analytics::scan_remote`: Share analysiert auf dem Host, SFTP/SSH-Agent
serverseitig, sonst begrenzte Backendlisten. Der gepoolte Ort wird vor Analyse/Duplikatsuche auf eine
lebende Verbindung aufgelöst. `message` zeigt Phase, Host-Ordner, Worker-Warten, Zusammenstellen,
Ergebnisübertragung/Prüfung und gegebenenfalls „Letzte Meldung … vor N s“ (ab fünf Sekunden).
`doneItems` zählt Dateien; `doneBytes` zählt erfasste Bytes, `totalBytes=0`. Nur während der
Ergebnisübertragung zählen diese Felder übertragene/gesamte Ergebnisbytes; danach wieder erfasste Bytes.
Der begrenzte alte Hostpfad ist ausdrücklich benannt. Ergebnisdetails bleiben durch das gemeinsam
angebotene Knoten-/Speicherbudget begrenzt, Größen und Dateizähler behalten die Gesamtwerte.

Hostzahlen (`ScanOutcome.volume/platform`) werden bei `remote:true` ausschließlich von der Gegenstelle
übernommen. Fehlende `volumeTotal`/`volumeFree`, App- oder Plattformzahlen bleiben unbekannt; kein Ersatz
aus der Platte oder Android-App-Statistik des Empfängers. Entfernte App-Zeilen öffnen keine lokalen
Android-App-Einstellungen. App-/Rest-/geschützte Ansichtszeilen ändern weder Baum noch Dateizähler.
Entfernte Duplikatsuche hat keine feste 200-Dateien-Kappung: Dateien ab `minSize` werden unter dem
Walk-/Kandidatenbudget verglichen; ohne Host-/Provider-Hash werden nur gleich große Kandidaten gelesen
(Anfang/Ende, erst bei Gleichstand vollständiger SHA-256). Teilberichte und Budgetgründe bleiben sichtbar.
Der Windows-Host bezeichnet seinen bestätigten remote-fähigen Speicher als Smart-Explorer-Papierkorb;
die Fähigkeit behauptet keinen nativen Windows-Systempapierkorb. Kein permanenter Löschfallback.

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
aus der Init-Konfiguration (primärer Speicher und genauer Legacy-Home-Migrationsfakt), Gerätename
ebenso. Neue Profile/Räume exportieren nichts automatisch. Die Laufzeit holt Worker-Ereignisse selbst **im Prozess** am
eingebetteten `ShareHost` ab (kein TCP): alle 300 ms, solange die Teilen-Seite sichtbar ist oder ein
Pairing läuft (`share.watch`), sonst alle 5 s bei sichtbarer App und alle 60 s im Hintergrund
(`sys.hostState.foreground`); sie hält den letzten Snapshot und sendet `share`/`shareRequest`.
```text
ShareStatus {running, connected, relayUrl:String?, lastError:String?, server:String?,
  lanPresence:String, identity:{deviceId, deviceName, fingerprint, directCode},
  devices:[{contactId, name, status, statusText, online:Boolean, location, lan:Boolean,
            shareBack:Boolean, write:Boolean?}],
  execProvider:{available:Boolean, provider, detail},
  execTargets:[{targetKey, relation:"direct|room", roomId:String?, roomName:String?, deviceId, name,
                fingerprint, enabled:Boolean, baseAuthorized:Boolean, policyRevision:Long}],
  rooms:[{profileId, roomId, name, status, autoJoin, location:String?,
          policy:{membersMayWrite:Boolean,confirmNewMembers:Boolean},
          members:[{deviceId, name, publicKey, nodeId, fingerprint, status, location, blocked:Boolean,
                    admission:"Admitted|Pending"}]}],
  incoming:[Request], outgoing:[Request],
  exports:{direct:[{label, path, access:"read_only|read_write", allowSystemWrites:Boolean}],
           rooms:{<profileId>:[{label,path,access,allowSystemWrites}]}},
  connectionExports:{direct:[{account,access}],rooms:{<profileId>:[{account,access}]}},
  writeGrants:[{deviceId,name,publicKey,nodeId,fingerprint,state:"Accepted|Ignored|Reconfirm",
                write:Boolean,active:Boolean,canSetWrite:Boolean}],
  autoHomeMigrations:[{scope,path}],
  discovery:{offer:{offerId, target, alias, untilMs}?, advertisements:[{discoveryId, kind:"direct|room",
             alias, expiresMs, compatible:Boolean}], exchange:{exchangeId, state:"running|done|failed|canceled",
             message:String?}?},
  removedDevices:[{deviceId, name}], notices:[String]}
Request {requestId, contactId:String?, name, stateText, canAccept, canReject, canRetry, canDelete,
  message:String?, timeMs:Long}
```
- `share.status {}` → `ShareStatus` (letzter Snapshot des Pollers; billig); dazu
  `power:{idleSupported:Boolean?, idleActive:Boolean, keepaliveSecs:Int?, lastServerContactMs:Long?}`
  (Ruhemodus des Share-Servers, Fähigkeit `idle_keepalive_v1`; `idleSupported:null` = noch nicht verbunden)
- `share.wake {networkChanged:Boolean}` → `{ok:Boolean, reconnected:Boolean}`: Verbindungsprobe (Wach-Alarm,
  Netzwechsel); wartet bis zum Ende der Probe (≤ 12 s); bei `networkChanged` zusätzlich `network_change` am
  Iroh-Endpunkt
- `share.watch {active:Boolean}` → `{}` (Teilen-Seite sichtbar → schneller Takt)
- `share.serverInfo {}` → `{server,security:"encrypted|plaintext|none",summary,plaintext:Boolean,
  ignoredPlaintext:Int,migrated:Boolean}`: kanonische gespeicherte Adresse mit Schema/optionalem
  Zertifikatspin. Nackte alte Adressen werden atomar zu `tcp://` umgeschrieben, ihre Bedeutung bleibt.
  `migrated` beschreibt nur den Aufruf, der tatsächlich umgeschrieben hat; Klartext bleibt sichtbar.
- `share.setServer {server, allowPlaintext:Boolean=false}` → gleiche Form wie `serverInfo`. Leere Eingabe
  entfernt den Server (nur LAN). Neue Adresse ohne Schema = TLS/WSS auf Port 51820. TCP/WS/HTTP braucht
  ausdrückliches „Unverschlüsselt erlauben“. Neue Eingaben mischen TLS und Klartext nicht, HTTPS/HTTP
  wird als WSS/WS gespeichert. Alte Klartexteinträge einer TLS-Liste werden ignoriert und gezählt;
  kein TLS-Klartext-Fallback. Selbst signierte Zertifikate nur per `#sha256=`-Pin. Status und Einstellungen
  zeigen verschlüsselt beziehungsweise „⚠ unverschlüsselt“; gespeicherte Bedeutung wird nicht repariert
  oder verschlüsselt behauptet, bevor der Nutzer eine TLS-Adresse speichert.
- `share.setOnline {online}` → `{}` (`auto_connect`)
- `share.setName {name}` → `{}`
- `share.suggestPin {}` → `{pin}` (sechs zufällige, nicht triviale ASCII-Ziffern)
- `share.discoverable {target:"direct"|<roomProfileId>, alias, pin, minutes, allowWeakPin:Boolean=false}` → `{}` ·
  `share.stopDiscoverable {offerId}` → `{}`
  (1–30 Minuten; PIN kürzer als sechs Unicode-Zeichen, periodische Wiederholung, einfache Ziffernfolge
  oder häufige PIN liefert ohne Opt-in `weak_pin`. Leere PIN bleibt auch mit Opt-in `invalid`.
  Angebot endet nach erster erfolgreicher Kopplung oder fünf Fehlversuchen; native Regeln sind maßgeblich.)
- `share.discover {}` → `{}` (Ergebnisse erscheinen in `discovery.advertisements`)
- `share.connect {discoveryId, pin, shareBack:Boolean=false}` → `{}` · `share.cancelConnect {exchangeId}` → `{}`
  (Nichtleere fremde PINbytes unverändert, auch alte kurze PIN; nur eine ausdrückliche Rückfreigabeauswahl
  öffnet zusätzlich eigene Freigaben. Ein Raumbeitritt erzeugt keine eigenen Exports.)
- `share.unconfirmedPairings {}` → `{pairings:[{exchangeId,kind:"direct|roomInstalled|roomShared",
  contactId:String?,roomProfileId:String?,label,revocable:Boolean}]}`
- `share.resolvePairing {exchangeId,revoke:Boolean}` → `{}` oder bei Entzug der vorhandene
  Endpoint-Cleanupbericht. `revoke:false` schließt nur den Hinweis; true entfernt den installierten
  Kontakt/Raum. Fehlgeschlagener Entzug erhält den Hinweis; `roomShared` ist nicht zurückholbar,
  deshalb Hinweis und gegebenenfalls neuen Raum anlegen. Unbekannte ID = `not_found`.
- `share.addDirect {code, name, shareBack:Boolean=false}` → `{contactId,shareBack:Boolean}`
  (kanonischer gespeicherter Wert; Wiederholung erhält ältere ausdrückliche Rechte). Bei erfolgreicher
  Kontaktanlage und fehlgeschlagener zusätzlicher Rückfreigabe nennt der Fehler den gespeicherten Kontakt
  und den retrybaren Weg (Hinzufügen wiederholen oder `setShareBack`), statt Erfolg zu behaupten.
- `share.removeDevice {contactId}` → `{removedFavorites:Int, orphanedJobs:[String]}`
- `share.readmit {deviceId}` → `{}`
- `share.createRoom {name}` → `{profileId, code}` · `share.joinRoom {code, name}` → `{profileId}` ·
  `share.roomCode {profileId}` → `{code:String?}` · `share.leaveRoom {profileId}` → `{}` ·
  `share.removeRoom {profileId}` → `{removedFavorites:Int, orphanedJobs:[String]}`
- `share.requestAccess {contactId, message:String?}` → `{}` · `share.decide {requestId, accept:Boolean}`
  → `{}` · `share.retry {requestId}` → `{}` · `share.deleteRequest {requestId}` → `{}`
- `share.addExport {scope:"direct"|<profileId>, path, label:String?}` → `{}` ·
  `share.removeExport {scope, path}` → `{}` (neue Rootanlage muss ein vorhandenes lokales Verzeichnis sein
  und ist RO; Entfernen nutzt den exakten gespeicherten Pfad)
- `share.setExportAccess {scope,path,access:"read_only|read_write",allowSystemWrites:Boolean?,
  expectedAccess:"read_only|read_write"?}` →
  `{persisted:true,changed:Boolean}`: bestehendes Root ändern; ausgelassenes Systemwrite-Feld erhält
  die bewusste Einstellung. Systemorte werden nur nach zusätzlicher ausdrücklicher Auswahl beschreibbar;
  eigene App-Daten bleiben unzugänglich. `expectedAccess` prüft das aktuelle Root-Recht innerhalb
  derselben Profil-CAS. Android gibt es bei einer reinen Systemwrite-Änderung mit, damit ein
  zwischenzeitlicher Entzug von RW nicht durch das unveränderte alte Recht überschrieben wird.
- `share.connections {scope}` → `{connections:[{account,label,shared:Boolean,access:String?}],
  sharedConnections:[{account,access}],warning}` aus dem strikten Saved-Store. Fehler wird sichtbar,
  keine still leere Liste als neue Freigabegrundlage. Konten sind literal gespeicherte IDs/Präfixe.
- `share.setConnectionExport {scope,account,shared:Boolean,access:"read_only|read_write"?,
  expectedShared:Boolean?}` →
  `{persisted:true,changed:Boolean}`: neue Auswahl RO, fehlendes `access` erhält bestehende Rechte
  auch bei CAS-Retry. `shared:false` entfernt nur dieses Konto, auch wenn nicht mehr gespeichert,
  und darf kein access enthalten. `expectedShared` prüft die aktuelle Auswahl innerhalb derselben
  Profil-CAS. Android sendet bei einer Rechteänderung an einem bereits freigegebenen Konto
  `expectedShared:true`; ein zwischenzeitlicher Entzug führt zum Fehler mit Neuladehinweis statt
  zur erneuten Freigabe. Gespeicherte Zugangsdaten werden nur für die bewusste Auswahl benutzt.
- `share.setContactWrite {deviceId,publicKey,nodeId,fingerprint,write:Boolean,name?}` →
  `{persisted:true,changed:Boolean}`: volle Pins aus `writeGrants`, `nodeId` auch bei Legacy leer mitgeben.
  Schreiben nur bei aktueller aktiver Identität; kein neuer Grant, keine Aufhebung eines Entzugs als Reparatur.
- `share.allowGrantAgain {deviceId,publicKey,nodeId,fingerprint,name?}` → `{persisted:true,changed:Boolean}`:
  bestehende inaktive lokale Grantidentität bewusst wieder zulassen; volle Pins und eindeutige Geräte-ID
  werden erneut geprüft. Kein Exec-Opt-in. `share.readmit` bleibt für gespeicherte entfernte Geräte erhalten.
- `share.withdrawGrant {deviceId,publicKey,nodeId,fingerprint,name?}` → `{persisted:true,changed:true}`:
  expliziter Entzug auch eines rein eingehenden Grants ohne `contactId`. Innerhalb derselben CAS werden
  die eindeutigen vollständigen Pins erneut geprüft und der zentrale Schlüssel-/Knotenentzug samt
  Alias-/Legacy-Historie und Exec-Entzug verwendet. Das Grant bleibt Ignored für bewusste Wiederzulassung;
  kein freies Secretcleanup und kein stilles Löschen der Widerrufshistorie.
- `share.setShareBack {contactId,shareBack:Boolean}` → `{persisted:true,changed:Boolean}`:
  bewusst an kann erneut freigeben, aus entfernt keine frühere ausdrückliche Freigabe. Entzug ist separat.
- `share.setRoomPolicy {profileId,roomId,membersMayWrite:Boolean?,confirmNewMembers:Boolean?}` →
  `{persisted:true,changed:Boolean}`; mindestens ein Feld. Exakte Profil-/Raumidentität erforderlich;
  Android sendet nur die geänderten Felder, damit die Bestätigungswahl kein unverändertes altes
  Schreibrecht erneut setzt. Pending/Blocked werden dadurch nicht zugelassen und Exec wird nicht aktiviert.
- `share.setRoomMember {profileId,roomId,deviceId,publicKey,nodeId,fingerprint,name?,
  action:"admit|block|allow"}` → `{persisted:true,changed:Boolean}`: eindeutige volle Mitgliedspins
  erneut im neuesten Profil prüfen; Pending bewusst zulassen, Identität sperren oder wieder zulassen.
  Namensähnlichkeit genügt nicht. Wiederzulassen aktiviert keine Befehle.
- `share.policy {}` → `{requests:"Ask|AutoAccept",warning:String?}` aus dem vorhandenen privaten
  Gerätepräferenzstore; fehlende/kaputte Datei wirkt als Ask mit Hinweis.
- `share.setPolicy {requests:"Ask|AutoAccept"}` → `{requests,persisted:true}`: ausdrücklicher Opt-in
  für AutoAccept, keine neue Policydatei und keine Löschung der Withdraw-/Legacy-Historie.
- `share.exec {location, command, shell:Boolean, timeoutSecs:Int}` → `{taskId}`;
  `result = {stdout, stderr, exitCode:Int?, timedOut, truncated}` (`daemon::exec_share`, Ausgabe ≤ 1 MiB;
  `shell:false` trennt an Leerraum, Anführungszeichen gruppieren, ein Backslash ist wörtlich außer vor
  Leerraum oder Anführungszeichen; eine verweigerte Freigabe kommt als `permission`)
- Exec-Host (dieses Gerät führt Befehle anderer Geräte aus; Anbieter `android-subreaper`: jeder Befehl
  läuft unter einem eigenen Subreaper-Zwischenprozess mit `/system/bin/sh -c`, Abbruch, Zeitlimit und
  Entzug beenden den ganzen Prozessbaum; Arbeitsordner ohne `cwd` = `homeDir`, `HOME` = `homeDir` und
  `TMPDIR` = `<cache>/tmp`, sofern der Aufrufer sie nicht setzt; Start und Ende eines Befehls lösen
  sofort ein `share`-Ereignis aus). Freigaben gelten wie am
  Desktop je exakter Identität; `execTargets` entsteht wie `exec_device_views` (Direkt-Freigaben und
  Raummitglieder), Schlüssel `direct/<deviceId>/<fingerprint>` bzw. `room/<roomId>/<deviceId>/<fingerprint>`:
  - `share.setExec {targetKey, enabled:Boolean}` → `{revision:Long}` (`daemon::mutate_exec_grant`,
    Journal wie am Desktop; Schlüssel gegen den aktuellen Profilstand aufgelöst, sonst `not_found`;
    nur vollständig gespeichert **und** angewendet gilt als Erfolg, sonst Fehler mit Detail;
    `enabled:true` bei nicht verfügbarem Anbieter → `unsupported` mit Grund, bei nicht aktiver
    Grundbeziehung (`baseAuthorized:false`) → `conflict` wie am Desktop)
  - `share.execJobs {}` → `{active:[ExecJob], history:[ExecJob]}` mit `ExecJob {direction:"incoming|outgoing",
    execId, peerDeviceId, peerName, program, state, startedAt:Long?, finishedAt:Long?, exitCode:Int?,
    message:String?}`; `state` = Lebenszyklus in snake_case (`running`, `exited`, `cancelled`,
    `timed_out`, `revoked`, …), Zeiten in Sekunden seit 1970; eingehende vor ausgehenden, `history`
    unsortiert (die App sortiert)
  - `share.cancelExecJob {direction, execId, peerDeviceId}` → `{}` (nicht mehr aktiv → `not_found`)
Nicht auf Android: LAN-Uplink, Anfragen im Altformat.

FC1-Kompatibilität: Persistente Legacy-Roots ohne `access` behalten RW, alte Grants ohne `write` true,
alte Raumpolicy ihr bisheriges Schreibrecht. Neue Roots/Konten sind RO, neue Grants/Räume ohne Schreiben,
neue Kontakte ohne Share-back. Kotlin zeigt fehlende neue Rechtefelder als unbekannt statt ein Opt-in zu
erfinden. Nur exakt identifizierte alte automatische Home-Roots werden einmalig RO; der dauerhafte
`autoHomeMigrations`-Hinweis bietet „Schreiben wieder erlauben“. Dies ändert nur das Root-Recht und
öffnet weder Kontakt-, Raum- noch Exec-Rechte. Alle bestehenden IDs/Locators bleiben erhalten.
`devices.write:null` bedeutet keine eindeutig verknüpfte lokale Freigabe; `writeGrants.write` ist die
gespeicherte Einstellung, `active` die aktuelle Grant-/Removed-Prüfung, `canSetWrite` erlaubt auch den
Entzug eines alten inaktiven Schreibflags. Der Core speichert Migration vor Rückgabe, begrenzt reale
Profilbytes/Encoding auf 1 MiB und behält beide Widerrufs-Ledger ohne 64er-Eviction.

Die Policy-Schreibantwort bestätigt die persistierte Datei, nicht den synchronen Abschluss aller
Transportsitzungen; Workerfehler bleiben im Log/Status. Android zeigt neue Rechte erst nach Erfolg und
Status-Neuladung. Unveränderte Rechte werden nicht erneut gespeichert. Dialogeingaben bleiben bei Fehlern erhalten, Aktionsfehler besitzen einen expliziten
Retry und werden durch erfolgreiche Statuspolls nicht gelöscht. Ein fehlgeschlagener Entzug oder eine
fehlende Gegenbestätigung wird nicht als abgeschlossener Erfolg dargestellt. Registrierung/Transport-
Status oder Offline-Flags löschen keine Withdraw-/Legacy-Historie. Neue Share-Server-Schlüsselanmeldung
(`key_login_v1`) und gebundene Direct-/Raumnachweise sind interne Protokolldeltas, keine Kotlin-Methoden;
Legacy-Server-Kompatibilität hat keinen automatischen Grant-/Exec-Opt-in zur Folge.
Die private Dateifassade aus S-LOCAL stellt für vorhandene Gerätepräferenzen/Profile ownergebundene
No-follow-/Hardlinkprüfungen, exklusive private Stufen, Datei-Sync und atomare Ersatzspeicherung bereit;
I/O-Fehler werden weitergereicht. Dies ergänzt keine Kotlin-Route, keine pauschale Freigabe und keine
neue LAN-Autorität. LAN-Uplink/S09-Linkauthentisierung bleiben andere Owner-Grenzen.

Interne Fernanalyse-Deltas: IPC `AnalyzeShare.node_budget` ist optional mit Default `None`; Clients
binden Empfang und ältere Listing-/Baumpfade vor Allokation an dasselbe Progress-Budget. Agentprotokoll
11 ergänzt `WireMeta.special` (Flagbit 1 neben Linkbit 0) und Watch-Kind 4 (`ReadyPartial`); Kind 0 bleibt
vollständige Bereitschaft. Share-Watch `complete:false` und Duplicate-Teil `more:false` sind additive
Legacy-Defaults; unvollständige Watchabdeckung behält periodische Abfragen. Diese Drahtfelder ändern keine
Kotlin-Ortsidentität und gemischte Agent-Binärversionen werden nicht stillschweigend dekodiert.
