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
  `kind` ∈ `invalid, not_found, permission, exists, unsupported, network, auth, conflict, busy,
  canceled, not_initialized, internal, weak_pin, host_key_unknown, sync_failed, hook` sowie die
  Tresor-Codes (unten). Stand 2026-10-21 aus dem Quelltext belegt (`native/src/mobile/core/error.rs`,
  `ApiError::new`/`invalid`/`not_found`/…-Aufrufe in `native/src/mobile/`); `invalid_argument` kommt
  im Code nicht vor. `message` ist deutscher Anzeigetext (wie Desktop).
  - Tresor-Codes (`native/src/vault/core/error_texts.rs`, `vault_use_provider` in
    `native/src/vault/core/klartext_policy.rs`): `vault_locked, vault_wrong_password,
    vault_unsupported_format, vault_hub, vault_memory, vault_no_saved_password, vault_unsaved_changes,
    vault_not_atomic, vault_use_provider, compare_stale, compare_too_large`.
  - `approval_unavailable` ist ein Ergebnis der Rückfrage (Protokoll und Startverweigerung des Hosts),
    kein Fehler der Brücke.
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
`secretsKey` (seit 2026-10-16) ist der 32-Byte-Datenschlüssel des Geheimnisspeichers als Hex-Text,
von `SecretsKey.kt` mit einem nicht exportierbaren AES-Schlüssel des Android-Keystores aus
`noBackupDir/secrets-key-v2.bin` entpackt; `null`, wenn der Keystore versagt (dann bleiben versiegelte
Einträge zu, nichts wird neu erzeugt). Der Kern hält ihn nur im Speicher und nicht in den Einstellungen.
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
           errors:[{path, message}], result:Json?, startedMs:Long, finishedMs:Long?,
           limiter:String, limiterText:String?}
```
`Task.kind` ∈ `transfer, delete, scan, properties, open, upload, materialize, extract, index,
sync, mirror, analyze, reclaim, trash, oauth, share, exec, update`.

`limiter` nennt den Engpass einer Übertragung (Quelle `native/src/transfer/core/limiter.rs`, Feld in
`native/src/mobile/core/tasks.rs`, geschrieben aus `native/src/mobile/os/shared/drive.rs`): `none` (noch kein
Urteil oder nichts gemessen), `ramp` (die Parallelität wächst noch), `idle` (der Bedarf füllt die Grenze nicht: die
Quelle liefert zu langsam), `source` und `target` (lokales Laufwerk), `remote` (Verbindung oder Server der
Gegenseite), `link` (die Leitung ist voll), `se_cpu` (alle Kerne belegt durch Smart Explorer), `se_thread` (ein
Faden rechnet, freie Kerne vorhanden), `memory` (wartet auf das Speicherbudget). Aufgaben ohne Übertragung
bleiben bei `none`. `limiterText` ist der Text dazu; `null`, wenn kein Text vorliegt. Bei einer privaten Aufgabe auf `vault://` (§4.13) ersetzt der Kern den Text: `source` und `target` heißen
„Begrenzt durch das Laufwerk“, `remote` heißt „Begrenzt durch Leitung oder Server“; ohne Namen des Laufwerks oder der
Gegenstelle. Andere Engpässe behalten den Text des Motors (`drive.rs`, `limiter_text_with`). Die Android-App liest
beide Felder in `core/Dtos.kt` (Klasse `TaskInfo`) und zeigt den Text in der Übertragungsleiste (`ui/transfers/TransfersBar.kt`).

Gesetzt wird der Wert bei jedem Fortschrittsbericht einer Übertragung (`drive.rs`, `report_transfer`); der Kern setzt ihn
beim Ende nicht zurück (im Quelltext nicht gefunden: nur `report_transfer` ruft `ctx.limiter` auf).

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
{"type":"syncProblem","jobId":"…","title":"…","text":"…"}   gedrosseltes Jobproblem → eigener Notificationkanal
{"type":"openUrl","url":"https://…"} Kern möchte eine URL im Browser öffnen (OAuth)
{"type":"error","action":"…","message":"…"}   Eintrag für das Fehlerprotokoll
{"type":"volumes"}                   Speicherorte geändert
{"type":"wake","ms":N}               Kern braucht die CPU N ms wach (Server-Keepalive, eingehende Anfrage
                                     oder Streams im Ruhemodus) → Partial-Wakelock, längere Anforderung gewinnt
{"type":"docs.changed","roots":Boolean,"documents":[id…],"commitFailed":Boolean}
                                     Dokumentanbieter: Wurzeln oder Ordner geändert, `commitFailed` bei
                                     fehlgeschlagenem Festschreiben (§4.11)
{"type":"vault","site":"…","state":"unlocked|locked|locking",
 "reason":"screen_off|background|idle|manual|process_end"|null}
                                     Tresor entsperrt oder gesperrt; `reason` nur für die fünf Gründe der
                                     Oberflächen, sonst `null` (§4.13)
{"type":"screenHostStart","session":N,"source":"…","still":Boolean,"maxWidth":N,"maxHeight":N,
 "viewerName":"…","viewerDeviceId":"…"}
                                     Bildschirm-Host: eine Sitzung beginnt (§4.12)
{"type":"screenSession","session":N,"state":"…","reason":String?}
                                     Bildschirm-Host: Zustand einer Sitzung (§4.12)
{"type":"screenView","session":N,"kind":"pending|accepted|state|stream|stats|controlState|windows|error|ended",…}
                                     Bildschirm-Betrachter: Fortschritt einer Ansichtssitzung (§4.12)
```
`syncProblem` mit `jobId` `vault:<job>` (Konflikte eines Tresor-Syncs, Titel „Tresor-Sync: Konflikte“) öffnet
den Vergleich des Tresors statt der Jobkonflikte (`AppNav`, `NavRequest.OpenVaultCompare`).

Navigationsarten (Android-Intent `ACTION_NAVIGATE` mit Extras `kind`, `value`, `token`; kein Aufruf der
Brücke; Kotlin `ui/AppNav.kt`, Funktion `decode`): `openLocation`, `selectTab`, `showTransfers`,
`openJobConflicts` (Wert `vault:<job>` → Tresor-Vergleich), `showShareRequests`, `showUpdate`,
`showBackgroundSettings`, `openScreen`, `showScreenGrid`, `openVaultCompare`, `showLauncherSettings`.
Unbekannte Art oder fehlender Wert, wo einer nötig ist: keine Navigation. Der Intent trägt nur dann eine Anfrage,
wenn sein Token dem der Installation entspricht (`fromIntent`).
- `openLocation` (Wert: ein Ort in der Form von §2 `location`) öffnet den Reiter „Dateien“ an diesem Ort.
- `openScreen` (Wert: Ziel-JSON `{"relation":"direct|room","relation_id","device_id","source","control"}`; bei
  `room` ist `device_id` Pflicht) öffnet den Reiter „Teilen“ mit dem Bildschirm. Das Ziel prüft
  `ScreenTargetJson.parse` (`ui/screen/ScreenTarget.kt`); ein ungültiger Wert wird nicht geöffnet. Der Schlüssel
  heißt `relation_id` und `device_id` (snake_case), nicht camelCase.
- `showScreenGrid` (Wert: Raum-ID oder leer) öffnet den Reiter „Teilen“ mit der Bildschirmübersicht, für einen Raum
  nur mit dessen Kennung.
- `showLauncherSettings` (ohne Wert) öffnet den Reiter „Mehr“ mit dem Abschnitt Launcher (`MoreScreen.kt`,
  `SettingsFocus.Launcher`).

## 4 Methoden

### 4.1 System (`sys.*`)
- `sys.hostState {powerSave:Boolean, metered:Boolean, wifi:Boolean, charging:Boolean, foreground:Boolean,
  deferScheduling:Boolean, screenOn:Boolean, storageAccess:Boolean?}` → `{}` (fehlende Felder = false,
  außer `screenOn`: fehlt es, gilt der Bildschirm als an). `screenOn` reicht an die Tresor-Sitzungen
  weiter (`vault`-Sperre bei aus und im Hintergrund, §4.13); `storageAccess` fehlt → unverändert, sonst
  Alle-Dateien-Zugriff (`Permissions.hasAllFilesAccess()` in `HostMonitor.kt`) erlaubt oder nicht (`native/src/mobile/os/shared/sys.rs`). `foreground:false` schaltet Share in den
  Ruhemodus (`share::power::set_low_power`); `deferScheduling:true` hält geplante Jobs (Start, Zeitplan,
  Echtzeit, Anschluss) zurück, laufende Jobs und `bg.catchUp` bleiben unberührt (Modus „Periodisch“ bei
  verdeckter App). Der eingebettete Daemon startet zurückgestellt, bis der erste `sys.hostState` kommt.
- `sys.volumes {volumes:[{path,label,primary,removable}]}` → `{}`
- `sys.errors {}` → `[{timeMs, action, message}]` · `sys.clearErrors {}` → `{}`
- `sys.crashLog {}` → `{text}` (leer, wenn keins)
- `sys.info {}` → `{coreVersion, dataDir, cacheDir}`
- `sys.platformTotals {volume,platform}` → `{remembered:true}`; ausschließlich das tatsächlich primäre,
  gemeldete lokale Volume. `platform` benutzt dasselbe Schema wie `analyze.start`; fehlende Zahlen
  bleiben unbekannt, erfolgreiche eigene Hostzahlen werden unabhängig von einer Analyse gespeichert.
- `sys.watchHints {volumes:[{path,cursor,changed}]}` → `{accepted,complete:false}`; höchstens 128 bekannte
  absolute Volume-Orte und Cursor bis 2048 Bytes. MediaStore bleibt ein unvollständiger Hinweis;
  Poll und Vollkontrolle bleiben notwendig.

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
- `scan.view {taskId, sort:Sort, collapsed:[location], offset:Int, limit:Int?, sinceRevision:Long?}` (`limit` ohne Angabe: alle Zeilen ab `offset`; mit Angabe mindestens 1, Kotlin `FilesApi.scanView` sendet mindestens 1)
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
- `conn.save {input:ConnectionInput}` → `Connection` mit `warnings:[String]` (leeres `password` bei
  Bearbeitung = unverändert; ändert die Bearbeitung Host, Port, Benutzer oder Startpfad, nennt
  `warnings` die Sync-Setups und Laufwerke, die weiter die alte Adresse nutzen – sie werden wie bei
  `conn.delete` nicht geändert)
- `conn.delete {id}` → `{removedFavorites:Int, orphanedJobs:[String]}` (Jobs werden wie am Desktop nur
  gemeldet, nicht gelöscht)
- `conn.forgetHostKey {id}` → `{}` (der nächste Verbindungsversuch meldet den neuen Schlüssel zur
  Bestätigung)
- `conn.trustHostKey {message}` → `{host, port, algorithm, fingerprint}`: `message` ist der Text eines
  Fehlers mit `kind:"host_key_unknown"` (unbekannter SFTP-Hostschlüssel mit Typ und SHA-256-
  Fingerabdruck); der Kern verbindet neu und speichert den Schlüssel nur, wenn der Server genau diesen
  Fingerabdruck zeigt. `conn.test`, `conn.save` und das Öffnen gespeicherter SFTP-Verbindungen melden
  einen unbekannten Schlüssel mit diesem `kind`.
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
- `sync.run {id}` → `{taskId}`; aktueller gespeicherter Job, jobeigener StateKey,
  `RunSettings::for_job`, gemeinsame Paarsperre und Hook-/Cancellation-Verträge.
  JobState unterscheidet echten Versuch, Erfolg, Sperre, Abbruch und Fehler; Prüfungen erzeugen
  keinen erfundenen Lauf-Erfolg. Ergebnis enthält `summary`, Zähler und reale Auslassungen.
- `sync.confirmBlock {id,kind}` → `{}`; bestätigt genau den angezeigten aktuellen Block einmal
  für den nächsten echten Lauf. Eine geänderte Sperre fordert erneute Prüfung.
- `sync.versions {id}` → `{taskId}`; Ergebnis `{items:[{token,path,side,runId,preservedMs,size,reason,store}]}`.
- `sync.log {id, from?}` → `{text, next, size, restarted, verbose}`: vollständige Zeilen des Live-Protokolls ab Offset `from` (ohne `from`: letzte Seite, höchstens 1 MiB); `restarted` = Datei rotiert/gekürzt.
- `sync.setLogVerbose {id, verbose}` → `{verbose}`: unveränderte Einträge ab dem nächsten Lauf einzeln protokollieren.
  Token ist opaque und an Job sowie unveränderte Endpunkte gebunden.
- `sync.restoreVersion {id,token,side?}` → `{taskId}`; `side` ist bei Legacy-Versionen ohne Seite
  erforderlich. Frische Verbindung, Paarsperre und erneuter Manifestabgleich vor sicherem Restore.
- `sync.mirror {source,target}` → `{taskId}`; einseitiges Kopieren ohne Löschen im Ziel,
  gemeinsame sichere Copy-/Versionsgrenze und Fortschritt; kein gespeicherter Job.

Der Konfliktkontext lebt je Job im Speicher. Nach einem Neustart ermittelt `sync.checkConflicts`
den Kontext per vollständigem Probelauf und ergänzt offene durable Merge-Aufträge aus dem
validierten StateKey. Originale stammen aus persistierten Seitenschreibweisen auf den ursprünglichen
Backends; Originaltexte sind je Seite auf 16 MiB begrenzt.

- `sync.conflicts {id}` → `{available:Boolean,items:[{cid,path,a:{exists,size,mtimeMs}?,
  b:{exists,size,mtimeMs}?,text:Boolean}]}`; ohne Kontext zuerst `sync.checkConflicts`.
- `sync.checkConflicts {id}` → `{taskId}`; reiner Check mit tatsächlichem Block und Recovery-Aufträgen.
- `sync.resolve {id,cid,choice:"a|b"}` → `{taskId}`; aufgezeichnete Auflösung unter PairLock,
  tatsächlicher Ergebnis-Checkpoint im jobeigenen Zustand. Offene Merge-Aufträge sperren A/B.
- `sync.skip {id,cid}` → `{}`; überspringt nur diesen Sitzungseintrag.
- `sync.finishConflicts {id}` → `{}`; erhält den bereits aufgezeichneten Zustand, Speicherfehler bleiben retrybar.
- `sync.mergeRows {id,cid}` → `{rows:[{a:String?,b:String?,equal:Boolean,takeA:Boolean,takeB:Boolean}],
  pending:null|{kind:"write|keep_both",keepA:Boolean?,confirmedA:Boolean,confirmedB:Boolean,
  preview:String?,previewTruncated:Boolean}}`. Bei Pending sind keine Teil-Mergebytes neue Originale;
  die Vorschau ist auf 8192 Unicodezeichen begrenzt und dient ausschließlich der Anzeige.
- `sync.mergeApply {id,cid,rows:[{takeA,takeB}…]}` → `{taskId}`; bytegebundener Recorded-Merge,
  Originale und Wahl werden vor der ersten Mutation privat gespeichert. TextShape erhält Zeilenenden.
- `sync.mergeKeepBoth {id,cid}` → `{taskId}`; aufgezeichnete KeepBoth-Wahl mit demselben exklusiven
  Konflikt-Sibling auf beiden Seiten und erhaltener Wiederaufnahme.
- `sync.mergeRetry {id,cid}` → `{taskId}`; ausdrücklich unveränderten gespeicherten Auftrag mit
  Originalbytes/-Signaturen und ursprünglicher Write-/KeepBoth-Wahl wiederholen.

Merge-Ergebnis und Teilfehler enthalten `confirmedA`, `confirmedB`, `partial`, `baselineRecorded`,
`reload`, `retry`, `preserved:[{path,confirmedA,confirmedB}]`, bei vollständiger Auflösung `remaining`.
Beide Bestätigungen und passender Basis-Eintrag sind nötig; Fehler erhalten Auftrag/Draft.

RV1 ergänzt Job-JSON um `state` (`lastAttemptMs`, `lastSuccessMs`, `lastRunner`, `lastCause`,
`consecutiveFailures`, `lastError`, `retryAtMs`, `blocked`, `problem`, `running`, `loadError`, `watch`,
`pendingTrigger`, `lastVerifyMs`, `verifyCursor`), `brokenConfig` für defekte Jobdateien sowie die
Editoroptionen `rtMaxLatencySecs`, `rtPollSecs`, `verifyIntervalSecs`, `verifyTargetSecs`, `maxDeleteMin`,
`retainCount`, `versionsLocation`, `crossMounts`, `runCleanup`. `sync.options` ergänzt `versionsLocations`.
Seit dem Paritätsstapel vom 2026-10-16 trägt Job jedes Feld des Desktop-Editors (`JobEditor`), Ein- und
Ausgabe: `modifyWindowSec:Long`, `filterMinSizeKb:Long`, `filterMaxSizeKb:Long`, `filterMaxAgeDays:Long`,
`filterMinAgeDays:Long`, `bwlimitKbps:Long`, `maxTransfers:Long`, `atomicCopy:Boolean`,
`verifyCopies:Boolean` (Rust `SyncJob::verify`, Größe nach jeder Kopie prüfen), `retries:Long`,
`retryDelaySecs:Long`, `connectMatch:String` (Muster des Auslösers „Bei Anschluss“; Android bietet den
Auslöser nicht an und erhält den gespeicherten Wert). `sync.validate` nennt deren Fehler unter diesen
Schlüsseln. Übersicht aller Oberflächen: `docs/oberflaechen/einstellungen.md`.
`lastResult` kommt aus JobState; `running` bezeichnet ausschließlich einen lebenden Runmark.

Hintergrund: genau ein eingebetteter Desktop-Daemon je App-Prozess (Thread), beim ersten Bedarf
gestartet, nie wegen Sichtbarkeit gestoppt. „Aus“ wirkt über das Sync-Flag (Android-Adapter von
`autostart::is_enabled`) und `share.setOnline`.
- `bg.ensureDaemon {}` → `{running:Boolean}` (idempotent; wartet bis zu 10 s auf Bereitschaft)
- `bg.status {}` → `{syncEnabled, daemonRunning, heartbeatAgeSecs:Long?, paused, pausedUntilMs:Long?,
  autopauseBattery, autopauseMetered, cadenceSecs:Int, catchUpRunning:Boolean, lastCatchUpMs:Long?,
  activeJob:String?, nextScheduledRunMs:Long?, storageAccess:Boolean?,
  lastCatchUpResult:{finishedMs,ran,succeeded,failed,message}?}`
- `bg.setSyncEnabled {enabled}` → `{}`
- `bg.pause {seconds:Long}` (−1 = unbegrenzt) → `{}` · `bg.resume {}` → `{}`
- `bg.setAutopause {battery, metered}` → `{}`
- `bg.setCadence {seconds:Int}` → `{cadenceSecs}`: Prüfintervall des Workers wie Desktop „Prüfintervall“
  und `se background cadence` (2–3600 s, sonst `invalid`)
- `bg.log {maxBytes:Int}` → `{text}`
- `bg.catchUp {}` → `{taskId}` (Task-`kind` = `catchup`; Nachhol-Lauf des Daemons: fällige Intervall-Jobs,
  Kalender-Termine seit dem letzten Lauf – im Hintergrund immer nachgeholt –, aktivierte Echtzeit-Jobs
  einmal; der Task endet, wenn alle für **diesen** Lauf zugelassenen Jobs fertig sind – ein Job, den der
  reguläre Plan schon hält, wird abgewartet und mitgezählt, bleibt aber dessen Job; kürzlich versuchte
  und nicht startbare stehen mit Grund in `result.skipped`; `task.cancel` bricht nur die eigenen Jobs
  dieses Laufs ab; pausiert/Sync aus → Task endet sofort mit `message`)
  Additives Taskresultat: `failed`, `retrySuggested`, `message`; WorkManager übernimmt die echte
  Retry-Auskunft und cancelt den eigenen nativen Auftrag bei Stop/Timeout. Ein leeres oder
  unterbrochenes Fenster erfindet keinen erfolgreichen Hintergrundlauf.

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

### 4.11 Dokumentanbieter (`docs.*`, Android-Dokumente)
Der Anbieter der Android-Dokumentenoberfläche (Kotlin `SeDocumentsProvider`) fragt über diese Methoden. Nichts ist
standardmäßig freigegeben: Wurzeln existieren nur für die Bereiche, die der Nutzer unter „Ordner für Apps
bereitstellen“ freigegeben hat (`docs.settings.set`); eine nicht freigegebene oder unbekannte ID ist `not_found`.
Eine Tresor-Wurzel existiert nur, solange der Tresor entsperrt ist. `docs.roots` berührt kein Netz.
- `docs.roots {}` → `[{rootId, documentId, title, kind:"devices|conn|gdrive|vault", create:Boolean, localOnly:Boolean}]`
- `docs.document {documentId}` → `Eintrag`; `docs.children {documentId}` → `{loading:Boolean, error:String?, children:[Eintrag]}`:
  ein kalter Ordner antwortet `loading:true` sofort, `docs.changed` folgt, sobald er da ist; ein nicht erreichbares
  Gerät liefert `error` (Gerätetext) und keine Kinder.
- `Eintrag {documentId, name, dir:Boolean, size:Long, mtime:Long (Millisekunden), writable:Boolean}`
- `docs.create {documentId (Elternordner), name, dir:Boolean=false}` → `Eintrag`; ist der Name belegt, zählt der Name
  hoch (`name (2).ext`); in der Geräteliste (ohne Ordner als Elternteil) liefert es `permission`.
- `docs.rename {documentId, name}` → `{documentId}` (neue ID); `docs.delete {documentId}` → `{}`. Wurzeln sind nicht
  umbenenn- oder löschbar (`permission`).
- `docs.settings.get {}` → `{provider, devices, connections:[ID], vaults:[ID], choices:{connections:[{id,label}],
  vaults:[{id,label,unlocked,local}]}}`; `docs.settings.set {provider?, devices?, connections?, vaults?}` → gleiche Form.
- `docs.pending {}` → `{pending:Int}` (Anzahl unveröffentlichter Änderungen aller Engines, geöffnet oder nicht); `docs.retryPending {}` → `{pending:Int, failed:[String]}`.
- Ereignis `docs.changed {roots:Boolean, documents:[documentId], commitFailed:Boolean}` (§3): Wurzeln oder Ordner
  haben sich geändert; `commitFailed:true` heißt, ein Festschreiben ist fehlgeschlagen.

### 4.12 Bildschirm (`screen.*`, Android als Host und als Betrachter)
Quelle: `native/src/mobile/os/shared/domains/screen_host.rs` (Host), `screen_view.rs` (Betrachter); die zwei Binärwege
sind `screenPush`/`screenControl` (Host) und `screenPull` (Betrachter) in `native/android-bridge/src/screen_jni.rs`.

**Host** (dieses Telefon wird gesteuert):
- `screen.hostPrereqs {accessibility?, usageAccess?, appWindowCapture?, display?, encoders?, apps?}` → `{accessibility:"on|off|restricted", usageAccess:Boolean, appWindowCapture:Boolean}`; meldet die App als Host an.
- `screen.hostSessions {}` → `{sessions:[…]}`
- `screen.hostStop {session:Long}` oder `{session:"all"}` bzw. `{all:true}` → `{stopped:Int}`; beendet Sitzungen wie der Host-Nutzer.
- `screen.hostReady {session, width, height, encoder, hardware?, projection?}` → `{}` (nach dem Systemdialog oder für Standbilder)
- `screen.hostEnded {session, reason}` → `{}`

**Betrachter** (dieses Telefon sieht einen anderen Bildschirm):
- `screen.devices {roomId?}` → `{devices:[{relation:"direct|room", relationId, deviceId, name, roomId, roomName, online,
  supported:Boolean|null, view:"off|ask|allow|null", control:"off|ask|allow|null"}]}` (aus Profilen und dem letzten
  `probe`; `null` = unbekannt)
- `screen.probe {relation, relationId, deviceId}` → `{supported, platform, view, control, monitors, input, codecs, limits}`
  (camelCase); fehlende Freigabe ist kein Fehler (`supported:true`, `view:"off"`).
- `screen.windows {relation, relationId, deviceId}` → `{windows, apps, listSource, limits}`
- `screen.viewOpen {relation, relationId, deviceId, mode:"view|control|preview", source?:"monitor:<id>|window:<id>",
  decoders:[{codec:"h264|tiles", profile:"cb|high"|null, hardware:Boolean, maxWidth?, maxHeight?}], displayWidth, displayHeight, maxFps}` → `{session:Long}`;
  der Fortschritt kommt als Ereignis `screenView`.
- `screen.viewSetMode {session, mode:"preview|full", source?, width?, height?, fps?, sharpness?, lossless?}` → `{}`
- `screen.viewInput {session, events:[Eingabe]}` → `{sent:Int}`; höchstens 256 Ereignisse je Aufruf (`invalid`).
  Eine Taste darf statt `hid_usage` mit `key` (Name aus der Tastentabelle) angegeben werden; `hid_usage` bleibt gültig.
- `screen.viewControl {session, action:"take|release"}` → `{}` · `screen.viewListWindows {session}` → `{}` (die Liste kommt als Ereignis `windows`)
- `screen.viewKeyframe {session}` → `{}`: fordert ein Schlüsselbild an (Grund `DecodeError`), wenn die Anforderung fällig ist.
- `screen.viewClose {session}` → `{}` (beendet die Sitzung; wiederholbar)
- Ereignis `{"type":"screenView","session":N,"kind":"pending|accepted|state|stream|stats|controlState|windows|error|ended",…}`:
  `pending {capability:"screenView|screenControl"}`; `accepted` die camelCase-Felder der Antwort des Hosts plus
  `path:"direct|relay"`; `state {state, reason}`; `stream {codec, profile, width, height, gop, encoder, hardware}`
  (bei **jeder** neuen Bildgruppe); `stats` die camelCase-Felder von `HostStats`, `rttMs` ist immer `null` (das Relais
  meldet keine Rundreisezeit); `controlState {holder:"viewer|other|null", blocked}`; `windows` (wie `screen.windows`);
  `error {code, detail}`; `ended {code, detail}`.
- Fehlerarten: `invalid` (ungültige Eingabe, unbekannte Taste), `permission` (Steuern nicht erlaubt), `not_found`, `internal`.
  Der Code kennt `invalid_argument` nicht; `unavailable` ist dort nur ein Zustandswert eines Tresors (§4.13), keine Fehlerart.

**Binärer Weg `screenPull(session: Long, dst: ByteBuffer, timeoutMs: Int): Int`** (Betrachter; `dst` ist direkt):
- Kopf (16 Byte, little-endian) und danach `len` Nutzlastbytes: Bytes 0–7 `pts_us` (Aufnahmezeit des Hosts), 8–11 `flags`,
  12–15 `len`.
- Flags: Bit 0 Schlüsselbild, Bit 1 fertiges Bild `tiles` mit Bytes R,G,B,A (`width × height × 4`), Bit 2 erstes Bild einer
  neuen Bildgruppe (die App konfiguriert den Dekoder neu, nachdem sie das zugehörige `stream`-Ereignis gelesen hat),
  Bit 3 vollständige Zugriffseinheit (immer gesetzt), Bits 16–31 die niedrigen 16 Bit der Bildgruppennummer aus dem
  `stream`-Ereignis.
- Rückgabe: Anzahl geschriebener Bytes (`16 + len`, größer als 0) bei Erfolg; `-1` Zeitüberschreitung (nichts geschrieben);
  `-2` Sitzung beendet; `-3` Puffer zu klein (nur die Bytes 12–15 werden mit der nötigen Länge `len` geschrieben, wenn
  der Puffer mindestens 16 Byte fasst; das Bild bleibt in der Warteschlange); `-4` Brückenfehler (Puffer nicht direkt).
  `timeoutMs` wird auf 1000 begrenzt. Die Funktion wirft nie eine Java-Ausnahme und schreibt nie Bildbytes in ein Protokoll.

**Ereignisse des Hosts** (über `pollEvents`): `screenHostStart {session, source, still, maxWidth, maxHeight, viewerName,
viewerDeviceId}` und `screenSession {session, state, reason}` (§3).

### 4.13 Tresore (`vault.*`, Cryptomator-Tresore)
Quelle: `native/src/mobile/os/shared/domains/vaults.rs`, `vault_keys.rs`, `vault_compare.rs`. Die 22 Methoden stehen in
Rust und in `android/app/src/main/java/app/smartexplorer/android/api/VaultApi.kt`. Fehlerarten: §1, dort die Tresor-Codes.

Methoden mit `{taskId}` laufen als Aufgabe; das Ergebnis steht im Aufgabenergebnis, der Fortschritt im Ereignis `task`.

- `Site {site, label, location, kind, connection, state, replica:Boolean, format, cipher, rememberPassword:Boolean,
  autoLockIdleMin, lockOnScreenLock:Boolean, lockOnBackgroundMin, syncJobs:[String]}`; `state` ist `locked|unlocked|locking|unavailable`
  (`unavailable`: der Eintrag der Tresorliste ist nicht lesbar; das Ereignis `vault` meldet ihn als `locked`).
- `vault.list {}` → `{vaults:[{vaultId, sites:[Site]}]}` · `vault.show {site}` → `Site`
- `vault.lock {site?, keepUnsaved:Boolean=false}` → `{locked}` · `vault.lockAll {keepUnsaved:Boolean=false}` → `{locked}`
- `vault.setSettings {site, label?, autoLockIdleMin?, lockOnScreenLock?, lockOnBackgroundMin?}` → `Site`
- `vault.forgetPassword {site}` → `{}`
- `vault.detect {location}` → `{isVault, format, keySource, vaultId, problem, name}`
- `vault.add {location, label?}` → `{site, label}` · `vault.remove {site}` → `{site}`
- `vault.unlock {site, password}` → `{taskId}`; Ergebnis `{site, already}`. Mit `saved:true` oder `remember:true` →
  `invalid` mit dem Text „Gespeicherte Passwörter brauchen auf Android die Bestätigung der Displaysperre.“
- `vault.create {location, label?, password}` → `{taskId}`
- `vault.changePassword {site, oldPassword, newPassword}` → `{taskId}`; Ergebnis `{backup, rememberCleared}`
- `vault.rememberSaved {sealed}` → `{}` · `vault.savedSecret {site}` → `{sealed}` (versiegelt; `vault_no_saved_password`, wenn nichts gemerkt ist)
- `vault.syncCreate {site, target:"replica|site|location", with?, name?, direction?, deletePolicy?}` → `{job, name, siteB, createdSite}`;
  `with` ist für `site` und `location` Pflicht; ein Ereignis `jobs` folgt.
- `vault.check {site}` → `{taskId}` · `vault.compare {job}` → `{taskId}` (Vergleich eines Tresor-Jobs)
- `vault.compareItems {list, revision:Long, group?, offset?, limit?}` → `{items, nextOffset, total}`; ein veralteter Vergleich → `compare_stale`.
  Ohne `limit` alle Einträge ab `offset`.
- `vault.compareText {job, item}` → `{a, b, rows:[{left, right, equal}], crlf:Boolean, finalNewline:Boolean}`
- `vault.resolve {job, item, action, confirmed?}` → `{taskId}`; `vault.resolveAll {job, action, confirmed?}` → `{taskId}`;
  Ergebnis `{item, resolved, failed:[{item, error}]}`
- `vault.mergeApply {job, item, content}` → `{taskId}`; Ergebnis `{item, resolved:1, failed:[]}`; zu großer Text: `invalid`.

Ereignis `{"type":"vault","site":"…","state":"unlocked|locked|locking","reason":…|null}` (§3). Aufgaben eines Tresor-Ortes
(`vault://<SITE>/…`) bei Übertragung, Scan und Löschen sind privat: Meldungen werden unterdrückt, Fehlerpfade und
Fehlertexte ersetzt; der Titel nennt nur die Bezeichnung („Tresor <Bezeichnung>“). `syncProblem` mit `jobId` `vault:<job>`
meldet Konflikte eines Tresor-Syncs (§3).

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
  lanPresence:String, lanPresenceEnabled:Boolean, identity:{deviceId, deviceName, fingerprint, directCode},
  devices:[{contactId, name, status, statusText, online:Boolean, location, lan:Boolean,
            shareBack:Boolean, write:Boolean?, autoConnect:Boolean, autoOpen:Boolean}],
  execProvider:{available:Boolean, provider, detail, userLabel, runAsSupported:Boolean},
  execTargets:[{targetKey, relation:"direct|room", roomId:String?, roomName:String?, deviceId, name,
                fingerprint, enabled:Boolean, baseAuthorized:Boolean, policyRevision:Long,
                runAs:"standard"|Konto, programs:[String]}],
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
  - `share.setExec {targetKey, enabled:Boolean, programs:[String]?}` → `{revision:Long}` (`daemon::mutate_exec_grant`;
    `programs` ersetzt die Programmliste im selben Schritt (leer = volle Shell), Android sendet immer den
    Standardnutzer; fehlt `programs` oder ist es `null`, bleibt die Liste unverändert; jeder Eintrag wird vor
    der Prüfung um Leerraum gekürzt; ein Eintrag ist ein Programmname oder ein absoluter Pfad, sonst `invalid` vor jeder Änderung;
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
  - `share.execLog {lines:Int?}` → `{entries:[{at, event, title, device, deviceId, detail, kind, account, approval}], max:1000}`
    (neueste zuerst; `lines` wird auf 1 bis 1000 begrenzt, Standard 200; `event`
    `auth_rejected|penalty_refused|start_refused|started|finished|approval_requested|approval_granted|approval_denied|approval_cancelled|approval_unavailable|grant_expired|helper_refused`, sonst ein anderer gespeicherter Name oder `unreadable`;
    `kind` `exec` oder `screen` (Zeilen ohne Art sind Exec-Zeilen); `account` ist das Konto eines Starts, leer wenn
    die Zeile keines nennt; `approval` `not_required|once|always|denied|cancelled|unavailable`, leer wenn keine Angabe)
- Zugriffsrechte je Gerät (Befehle, Terminal, Bildschirm ansehen, Bildschirm steuern). Jede Fähigkeit hat den
  Schlüssel `execCommands`, `execTerminal`, `screenView` oder `screenControl` mit der Entscheidung
  `off|ask|allow`; `targetKey` wie bei `share.setExec`. Nur die Fähigkeiten, die dieses Telefon anbietet (`share.status` → `execProvider.capabilities`):
  - `share.setAccess {targetKey, account?:{kind:"own"}, capabilities?:{<Fähigkeit>:"off|ask|allow"}, programs?:[String], validUntil?:Int|null}`
    → `{revision:Long, access:{account, accountLabel, capabilities, validUntil, expired}}`. Fehlende oder `null`-Felder
    bleiben unverändert; `validUntil:null` entfernt das Ende, eine Zahl (Sekunden seit 1970) muss in der Zukunft liegen;
    ohne jede Änderung `invalid`. `account` ist nur `{"kind":"own"}` (sonst `unsupported`). Eine Fähigkeit, die dieses
    Telefon nicht anbietet, ist `unsupported`; eine Fähigkeit auf `ask` oder `allow` bei fehlendem Anbieter ist
    `unsupported`, bei nicht aktiver Grundfreigabe `conflict`. Unbekanntes Gerät `not_found`. Nur gespeichert und
    angewendet gilt als Erfolg, sonst `internal` mit Angabe, was fehlt.
  - `share.accessApprovals {}` → `{pending:[{id, deviceName, deviceId, fingerprint, relation, room, capability,
    capabilityLabel, text, accountLabel, command, requestedAt, surface, olderClientTerminal}]}`, älteste zuerst; leer,
    solange kein Worker läuft. `capability` ist der Schlüssel der Fähigkeit; `command` ist `{mode:"argv",program,args}`,
    `{mode:"shell",text}`, `{mode:"login"}` oder `null` (Bildschirm).
  - `share.decideAccessApproval {approvalId, decision:"once|always|deny"}` → `{}`. Eine ungültige ID (32 Hexadezimalzeichen)
    ist `invalid`; eine Rückfrage, die nicht mehr offen ist, `not_found`.
  - `share.reportApprovalUndeliverable {approvalId}` → `{}`: Die App kann die Rückfrage weder als Dialog noch als
    Benachrichtigung zeigen. Der Host verweigert den Start dann mit dem Ergebnis `approval_unavailable`; er erlaubt nie.
- Share-Einstellungen je Gerät, Raum und Freigabe (alle → `{persisted:true, changed:Boolean}` außer wo genannt):
  - `share.setContactOptions {contactId, autoConnect:Boolean?, autoOpen:Boolean?}` (mindestens eines)
  - `share.resetContactTrust {contactId}` (vergisst die gepinnte Identität, entzieht die Autorisierung, Exec aus)
  - `share.renameRoom {profileId, name}` (leer = „Raum“, ≤ 256 Byte, keine Steuerzeichen)
  - `share.rejoinRoom {profileId}` (`autoJoin` wieder an)
  - `share.resetMemberTrust {profileId, deviceId}` (Präsenz vergessen)
  - `share.moveExport {scope, path, offset:-1|1}`
  - `share.setDirectOnline {online:Boolean}` → `{online}` (bis zum Neustart des Workers; ohne Worker `network`)
  - `share.setLanPresence {enabled:Boolean}` → `{enabled}`
  - `share.renewCode {}` → `{directCode, warning:String?}` (Worker kurz gestoppt, Direkt-Freigaben pausieren, Exec aus)
Nicht auf Android: LAN-Uplink, Anfragen im Altformat, Exec als anderes Konto.

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

RV1-Sync-Verträge am 2026-10-03 mit aktuellen mobilen Dispatch-/Consumerquellen abgeglichen.
[Detailvertrag und Entscheidungen](../2026-10-02-review-analyse-sync-sicherheit/api-delta/AND-SYNC.md);
Ausführungsabnahme erfolgt durch die eine finale Remote-Task-Suite dieses Batches.
