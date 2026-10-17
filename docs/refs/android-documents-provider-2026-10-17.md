# Android-Dokumentanbieter (DocumentsProvider) und Proxy-Dateideskriptoren

Stand und Abrufdatum 2026-10-17. Reine Quellenrecherche für Strang S2 (`docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md`, Ergebnis E10, Grundlage für E4 „Gerät als Ordner“ und E8 „Tresor als Laufwerk“). Keine Builds, keine Laufzeitversuche. Folgerungen sind als **Folgerung** gekennzeichnet, Annahmen als **Annahme**.

## 0. Quellenstände

| Quelle | Stand | Abruf |
|---|---|---|
| AOSP `frameworks/base` über den GitHub-Spiegel `aosp-mirror/platform_frameworks_base`, Branch `main` | letzter Commit `1cdfff555f4a21f71ccc978290e2e212e2f8b168` (2025-03-27); der Spiegel ist laut GitHub seit 2023-11-08 archiviert, erhielt aber laut Commitliste noch Commits bis 2025-03-27 | 2026-10-17 |
| Dateien daraus: `core/java/android/provider/DocumentsProvider.java`, `core/java/android/provider/DocumentsContract.java`, `core/java/android/os/storage/StorageManager.java`, `core/java/android/os/ProxyFileDescriptorCallback.java`, `core/java/com/android/internal/os/FuseAppLoop.java`, `core/java/android/os/ParcelFileDescriptor.java`, `core/java/android/os/FileUtils.java`, `services/core/java/com/android/server/uri/UriGrantsManagerService.java` | wie oben | 2026-10-17 |
| AOSP DocumentsUI, `android.googlesource.com/platform/packages/apps/DocumentsUI`, `refs/heads/main`: `src/com/android/documentsui/roots/ProvidersCache.java`, `src/com/android/documentsui/DirectoryLoader.java` | Branch-Kopf am Abruftag, Commit-SHA nicht ermittelt | 2026-10-17 |
| developer.android.com „Create a custom document provider“ (`/guide/topics/providers/create-document-provider`) | „Last updated 2025-02-10 UTC“ | 2026-10-17 |
| developer.android.com „Access documents and other files from shared storage“ (`/training/data-storage/shared/documents-files`) | „Last updated 2026-10-16 UTC“ | 2026-10-17 |
| developer.android.com „Processes and app lifecycle“ (`/guide/components/activities/process-lifecycle`) | „Last updated 2025-02-10 UTC“ | 2026-10-17 |
| RSAF (`github.com/chenxiaolong/RSAF`), `app/src/main/java/com/chiller3/rsaf/rclone/RcloneProvider.kt`, README | Branch `master`, Kopf `932a29b0019a99dd0a0ccd24bb8135c98a2ceb7e` (2026-10-15); letzter Release v4.14 („10 Oct“, Jahr auf der Seite nicht angezeigt) | 2026-10-17 |
| Material Files (`github.com/zhanghai/MaterialFiles`), `app/src/main/java/me/zhanghai/android/files/file/FileProvider.kt`, `app/src/main/AndroidManifest.xml` | Branch `master`, Kopf `93c333960aa897836ba3f215a19d0cb0de494db1` (2026-10-15) | 2026-10-17 |
| Nextcloud Android (`github.com/nextcloud/android`), `app/src/main/java/com/owncloud/android/providers/DocumentsStorageProvider.java` | Branch `master`, letzter Commit an der Datei `ca631c7d298787976e799e8246e37a99376ee189` (2026-08-11) | 2026-10-17 |
| Cryptomator Android (`github.com/cryptomator/android`), `presentation/src/main/AndroidManifest.xml` auf `develop`, Issues #35 und #591, Releases | Release 2.0.1 (Latest), Vorabversion 2.1.0-beta1 („09 Oct“) | 2026-10-17 |

Einschränkung der Quellenlage: Die Referenzseiten auf developer.android.com (`/reference/...`) lieferten über WebFetch nur die Navigationsleiste, nicht den Seitentext (drei Versuche mit Versatz). Die API-Verträge unten sind deshalb aus dem Javadoc im AOSP-Quelltext zitiert; dieser Javadoc ist die Quelle der Referenzseiten. Der Quelltextstand ist 2025-03-27 (archivierter Spiegel), nicht der Stand von Android 16 QPR oder Android 17. **Annahme:** Die zitierten öffentlichen Verträge haben sich seither nicht geändert; geprüft ist das nicht.

### Prüfung der Vorbefunde

| Vorbefund | Ergebnis der Prüfung |
|---|---|
| `docs/lesungen/2026-10-17-laufwerke-kern.md` F7: Smart-Explorer-Manifest hat nur `.system.LocalFileProvider` und `androidx.core.content.FileProvider`, keinen DocumentsProvider, kein `MANAGE_DOCUMENTS` | Gültig (Lesung vom selben Tag, nicht erneut gelesen). |
| `docs/refs/cryptomator-mounting-2026-10-04.md` §6: Cryptomator Android 2.0.1 hat nur einen FileProvider, Issue #35 offen | Gültig: `develop`-Manifest am 2026-10-17 enthält genau einen `<provider>` (`androidx.core.content.FileProvider`, Authority `${applicationId}.fileprovider`, `exported=false`, `grantUriPermissions=true`), keinen `DOCUMENTS_PROVIDER`-Filter, kein `MANAGE_DOCUMENTS`. Issue #35 ist offen. |
| `docs/refs/cryptomator-mounting-2026-10-04.md` §6: `openProxyFileDescriptor` ab API 26 | Nicht erneut am Seitentext geprüft (siehe Einschränkung oben). Für Smart Explorer unerheblich, weil `minSdk = 30` (`docs/refs/android-apis.md`, Kopfzeile). |
| `docs/refs/android-open-with.md`: Android-11-Sperren für `ACTION_OPEN_DOCUMENT_TREE` | Gültig; am 2026-10-17 auf der Seite „documents-files“ (Stand 2026-10-16) erneut bestätigt, siehe A3. |
| `docs/refs/android-platform.md` §1: Hintergrundstart-Ausnahmen für Vordergrunddienste, `dataSync` 6 h je 24 h ab Android 15 | Nicht erneut abgerufen (Stand 2026-09-25); in A4 als Vorbefund zitiert. |

## 1. Ziel und Fragen

Ziel: belegen, was ein Android-Dokumentanbieter leisten muss und kann, damit Smart Explorer Geräte (E4) und entsperrte Tresore (E8) anderen Apps und der System-App „Dateien“ (DocumentsUI) als Ordner bereitstellt; Vergleich mit drei veröffentlichten Implementierungen und mit Cryptomator Android.

- A1: Vertrag von `DocumentsProvider`: Methoden, Root-Flags, Berechtigung, Threading, `CancellationSignal`.
- A2: `StorageManager.openProxyFileDescriptor` mit `ProxyFileDescriptorCallback`: Seek, Schreiben, `onFsync`, `onRelease`, Handler-Thread, Grenzen gegenüber Pipes, Leistung; welche Variante RSAF, Material Files, Nextcloud und Cryptomator nutzen und warum.
- A3: Sichtbarkeit der Roots in DocumentsUI und in `ACTION_OPEN_DOCUMENT`/`ACTION_OPEN_DOCUMENT_TREE`, dynamisches Hinzufügen und Entfernen, Android-11-Einschränkungen.
- A4: Grenzen: Apps nur mit Dateipfaden, Medienscanner, Prozessstart, Hintergrundbeschränkungen bei langen Vorgängen.

## 2. Befunde

### A1 — Vertrag von `DocumentsProvider`

**Manifest und Berechtigung.** Klassen-Javadoc: „When defining your provider, you must protect it with {@link android.Manifest.permission#MANAGE_DOCUMENTS}, which is a permission only the system can obtain.“ Das Beispielmanifest setzt `android:exported="true"`, `android:grantUriPermissions="true"`, `android:permission="android.permission.MANAGE_DOCUMENTS"` und den Intent-Filter `android.content.action.DOCUMENTS_PROVIDER` (= `DocumentsContract.PROVIDER_INTERFACE`). Durchsetzung im Code: `attachInfo` wirft eine `SecurityException`, wenn der Anbieter nicht exportiert ist, keine URI-Grants erlaubt oder nicht mit `MANAGE_DOCUMENTS` geschützt ist. Quelle: `DocumentsProvider.java` (Spiegelstand oben). Der Leitfaden „Create a custom document provider“ (Stand 2025-02-10) nennt dieselben vier Anforderungen und zusätzlich „A target of API level 19 or higher.“

**Methoden (Zitate aus dem Javadoc, `DocumentsProvider.java`):**

| Methode | Vertrag laut Javadoc |
|---|---|
| `queryRoots` | „Return all roots currently provided. To display to users, you must define at least one root.“ — „You should avoid making network requests to keep this request fast.“ — „If this set of roots changes, you must call … {@link DocumentsContract#buildRootsUri(String)} to notify the system.“ |
| `queryDocument` | Metadaten eines Dokuments; im Abruf nicht gesondert zitiert (Lücke L-A1). |
| `queryChildDocuments` (3 Argumente) | „This must only return immediate descendants, as additional queries will be issued to recursively explore the tree.“ — „you may return the local data immediately, setting {@link DocumentsContract#EXTRA_LOADING} on the Cursor“ — „To support change notifications, you must {@link Cursor#setNotificationUri(ContentResolver, Uri)} with a relevant Uri“ und anschließend mit dieser Uri `notifyChange` senden. |
| `openDocument(documentId, mode, CancellationSignal)` | „Mode "r" should always be supported.“ — `@param mode`: „such as 'r', 'w', or 'rw'.“ — „You may return a pipe or socket pair if the mode is exclusively "r" or "w", but complex modes like "rw" imply a normal file on disk that supports seeking.“ — „If you block while downloading content, you should periodically check {@link CancellationSignal#isCanceled()} to abort abandoned open requests.“ — „Your provider should return a reliable {@link ParcelFileDescriptor} to detect when the remote caller“ das Dokument schließt. `@throws AuthenticationRequiredException`. |
| `createDocument` | „Create a new document and return its newly generated {@link Document#COLUMN_DOCUMENT_ID}.“ — „If the MIME type is not supported, the provider must throw.“ — der Anbieter darf den Namen an interne Bedingungen anpassen. |
| `deleteDocument` | „Upon returning, any URI permission grants for the given document will be revoked.“ Für mitgelöschte Dokumente (Inhalt eines Ordners) ist der Implementierer selbst für den Entzug der Grants zuständig (`revokeDocumentPermission`). |
| `renameDocument` | Gibt eine neue `COLUMN_DOCUMENT_ID` zurück, wenn sich die ID ändert; „If the original {@link Document#COLUMN_DOCUMENT_ID} is still valid after the rename, return {@code null}.“ |
| `isChildDocument` | „Test if a document is descendant (child, grandchild, etc) from the given parent.“ — „providers must implement this to support {@link Intent#ACTION_OPEN_DOCUMENT_TREE}.“ — „You should avoid making network requests to keep this request fast.“ Standardimplementierung gibt `false` zurück. |
| `findDocumentPath` | „Finds the canonical path for the requested document.“ — „This API assumes that document ID has enough info to infer the root.“ Ohne Tree-URI prüft der Code `MANAGE_DOCUMENTS` des Aufrufers. |
| `ejectRoot` | „Ejects the root. Throws {@link IllegalStateException} if ejection failed.“ Gehört zu `Root.FLAG_SUPPORTS_EJECT`. |

**Durchsetzung von Tree-URIs.** `enforceTree` ruft bei abweichender Eltern- und Kind-ID `isChildDocument(parent, child)` auf und wirft „Document … is not a descendant of …“, wenn `false` zurückkommt. Der Code prüft dabei `FLAG_SUPPORTS_IS_CHILD` nicht selbst. **Folgerung:** Ohne überschriebenes `isChildDocument` scheitert jeder Zugriff über eine Tree-URI unterhalb der Tree-Wurzel; `isChildDocument` muss ohne Netzaufruf aus der Dokument-ID entscheidbar sein (so lösen es RSAF und Nextcloud, siehe A2).

**Modus-Zeichenketten.** `ParcelFileDescriptor.parseMode` akzeptiert laut Javadoc „"r", "w", "wt", "wa", "rw" or "rwt"“. `FileUtils.translateModeStringToPosix` bildet ab: `rw…` → `O_RDWR | O_CREAT`, `w…` → `O_WRONLY | O_CREAT`, `r` → `O_RDONLY`; `t` ergänzt `O_TRUNC`, `a` ergänzt `O_APPEND`. **Wichtig:** `"w"` enthält kein `O_TRUNC`; nur `"wt"` und `"rwt"` kürzen. **Folgerung:** Ein Anbieter muss `"w"` als Überschreiben ab Offset 0 ohne Kürzen behandeln und darf ein Kürzen nur bei `t` ausführen; ein Client, der mit `"w"` eine kürzere Datei schreibt, hinterlässt sonst Restbytes. Diese Folgerung betrifft das Verhalten des Clients, nicht einen Fehler des Anbieters.

**Root-Flags (`DocumentsContract.Root`, Javadoc-Erstsätze):**

| Flag | Wert | Bedeutung |
|---|---|---|
| `FLAG_SUPPORTS_CREATE` | 1 | „at least one directory under this root supports creating content.“ Root erscheint für `ACTION_CREATE_DOCUMENT`. |
| `FLAG_LOCAL_ONLY` | 1<<1 | „this root offers content that is strictly local on the device.“ Bezug zu `Intent#EXTRA_LOCAL_ONLY`. |
| `FLAG_SUPPORTS_RECENTS` | 1<<2 | über `queryRecentDocuments` abfragbar |
| `FLAG_SUPPORTS_SEARCH` | 1<<3 | `querySearchDocuments` |
| `FLAG_SUPPORTS_IS_CHILD` | 1<<4 | „this root supports testing parent child relationships.“ |
| `FLAG_SUPPORTS_EJECT` | 1<<5 | „this root can be ejected.“ |
| `FLAG_EMPTY` | 1<<6 | Root zurzeit leer; kann beim Öffnen ausgeblendet werden; bei Änderung muss `notifyChange` auf `buildRootsUri` erfolgen |
| `FLAG_ADVANCED`, `FLAG_HAS_SETTINGS`, `FLAG_REMOVABLE_SD`, `FLAG_REMOVABLE_USB` | 1<<16 bis 1<<19 | `@SystemApi` und `{@hide}`: für Drittanbieter nicht nutzbar |

Dokument-Flags (`DocumentsContract.Document`), vollständig laut Quelltext: `FLAG_SUPPORTS_THUMBNAIL`, `FLAG_SUPPORTS_WRITE`, `FLAG_SUPPORTS_DELETE`, `FLAG_DIR_SUPPORTS_CREATE`, `FLAG_DIR_PREFERS_GRID`, `FLAG_DIR_PREFERS_LAST_MODIFIED`, `FLAG_SUPPORTS_RENAME`, `FLAG_SUPPORTS_COPY`, `FLAG_SUPPORTS_MOVE`, `FLAG_VIRTUAL_DOCUMENT`, `FLAG_SUPPORTS_REMOVE`, `FLAG_SUPPORTS_SETTINGS`, `FLAG_WEB_LINKABLE`, `FLAG_PARTIAL` („not complete, likely its contents are being downloaded“; nicht öffnen/kopieren/verschieben, aber löschbar), `FLAG_SUPPORTS_METADATA`, `FLAG_DIR_BLOCKS_OPEN_DOCUMENT_TREE` (sperrt nur diesen Ordner für die Baumauswahl; „To guarantee a directory is never granted“ müssen er und alle Eltern das Flag setzen). Zu `FLAG_SUPPORTS_WRITE`: Dokumente aus `ACTION_OPEN_DOCUMENT` erhalten Lese- und Schreib-Grant, die Beschreibbarkeit kann sich über die Zeit ändern.

**Lade-Extras.** `EXTRA_LOADING`: „Optional boolean flag included in a directory {@link Cursor#getExtras()} indicating that a document provider is still loading data.“ Ebenso `EXTRA_INFO` (Hinweistext) und `EXTRA_ERROR` (Fehlertext) als Cursor-Extras.

**Threading und Abbruch.** Das Klassen-Javadoc enthält keine Threading-Aussage. Belegt sind nur die Methodenhinweise: `queryRoots` und `isChildDocument` ohne Netzaufruf; `openDocument` darf blockieren und soll `CancellationSignal` prüfen. Der Leitfaden: „It's OK to do network operations in this method to download the document, as long as you periodically check the CancellationSignal. If you have an extremely large file to transfer from the network, a better solution may be pipes or sockets“. DocumentsUI (`DirectoryLoader.java`) übergibt beim Verzeichnisabruf ein `CancellationSignal` und ruft `mSignal.cancel()` in `cancelLoadInBackground` auf; ein Abfrage-Timeout setzt `DirectoryLoader` nicht. Nextcloud kommentiert ausdrücklich Clients, die `openDocument` auf dem Hauptthread aufrufen („dirty threading workaround for client apps which call openDocument on the main thread“). **Folgerung:** Ein Anbieter muss jede Methode als von einem Binder-Thread des Systems und mit potenziell wartendem UI-Thread des Clients gerufen behandeln; lange Netzarbeit gehört hinter `EXTRA_LOADING` und `notifyChange`.

**Authentifizierung.** `AuthenticationRequiredException` ist an `createDocument`, `renameDocument`, `deleteDocument`, `findDocumentPath`, `queryChildDocuments` und `openDocument` deklariert. `DirectoryLoader` referenziert sie nicht; ob DocumentsUI sie an anderer Stelle auswertet, ist nicht geprüft (Lücke L-A2).

### A2 — `openProxyFileDescriptor` und `ProxyFileDescriptorCallback`

**Vertrag (Javadoc `StorageManager.java`):** „Opens a seekable {@link ParcelFileDescriptor} that proxies all low-level I/O requests back to the given {@link ProxyFileDescriptorCallback}.“ Gedacht für Inhalte, „that isn't backed by a real file on disk, such as a file on a network share, cloud storage service, etc.“ und für „an encrypted file that you're willing to decrypt on-demand“. Modi: `MODE_READ_ONLY`, `MODE_WRITE_ONLY`, `MODE_READ_WRITE`. `@param handler`: „Handler that invokes callback methods.“ Öffentliche Signatur `openProxyFileDescriptor(int mode, ProxyFileDescriptorCallback callback, Handler handler) throws IOException`; sie prüft `checkNotNull(handler)`. Die Zwei-Argument-Variante ist `{@hide}` und fällt auf `new Handler(Looper.getMainLooper())` zurück.

**Callback-Vertrag (`ProxyFileDescriptorCallback.java`):** „All callback methods except for onRelease should throw {@link android.system.ErrnoException} with proper errno on errors.“ Typische errno: EIO, ENOENT, ENOSPC, EBADF. `onGetSize` liefert die Größe; `onRead(offset, size, data)`: „It needs to return exact requested size of bytes unless it reaches file end.“; `onWrite(offset, size, data)`; `onFsync`: „Ensures all the written data are stored in permanent storage device.“ (Standard wirft EINVAL); `onRelease`: „Invoked after the file is closed.“ (abstrakt, ohne Fehlerkanal). Weitere Callbacks gibt es nicht; insbesondere kein Kürzen (`ftruncate`).

**Mechanik (`FuseAppLoop.java`, `StorageManager.java`):** `openProxyFileDescriptor` ruft `mountProxyFileDescriptorBridge()` und betreibt eine `FuseAppLoop` auf einer AppFuse-Einhängung. Belegt:
- ein einziger nativer Schleifen-Thread je `FuseAppLoop` („native_start(mInstance)“);
- je geöffneter Datei ein eigener `Handler` auf dem Looper des übergebenen Handlers („callback, new Handler(handler.getLooper(), this)“);
- `FUSE_MAX_WRITE = 128 * 1024` (128 KiB je Lese-/Schreibanfrage);
- `FUSE_GETATTR` beantwortet die Größe über `onGetSize()`; `FUSE_FSYNC` ruft `onFsync()`; `FUSE_RELEASE` entfernt den Eintrag;
- Fehlerabbildung: `ErrnoException` → negierte errno; jede andere Ausnahme → `-EBADF`.
- Grenze offener Dateien: nur `Integer.MAX_VALUE - MIN_INODE`; eine kleinere Grenze steht in dieser Datei nicht.

**Grenzen gegenüber Pipes (belegt):** Pipes/Socketpaare sind laut `openDocument`-Javadoc nur für ausschließlich `"r"` oder `"w"` zulässig; `"rw"` verlangt Seek. `createReliablePipe` erlaubt der Schreibseite, einen Fehler über `closeWithError(String)` zu melden, den die Leseseite mit `checkError()` liest. Der Proxy-FD ist seekbar und unterstützt `"rw"`.

**Erfahrungen der Vergleichsimplementierungen:**

| Projekt | Rolle | `openDocument`/`openFile`-Mechanik | Handler-Thread | Begründung im Code |
|---|---|---|---|---|
| RSAF (rclone-SAF-Anbieter) | `DocumentsProvider` | `storageManager.openProxyFileDescriptor(pfdMode, proxyFd, ioHandler)`; Pipe (`createReliablePipe`) nur für Vorschaubilder | je geöffneter Datei ein neuer `HandlerThread` („We spawn a new thread for every opened file to handle I/O.“) | Kommentar: Beim Öffnen sendet `IVold.openAppFuseFile()` ein `FUSE_LOOKUP`, dessen Antwort von `onGetSize()` abhängt; ein gemeinsamer langsamer I/O-Thread blockiert diese Antwort, „Since openAppFuseFile() holds the main vold lock, IVold.monitor() will be blocked … and the watchdog will kill system_server.“ |
| RSAF, Schließen | — | `onRelease()` startet `Thread(::onReleaseBackground)`; dort Upload | — | Kommentar: „Do not block this FUSE operation.“ `IVold.unmountAppFuse()` hält die vold-Hauptsperre; ein langsames Aushängen führt dazu, dass „system_server to be killed, which, to the user, appears as if the system rebooted. This is a pretty nasty Android bug that allows denial of service from any unprivileged app.“ |
| RSAF, Schreiben | — | Upload erst nach dem Schließen („files do not begin to upload until they are closed“) | — | Klassenkommentar: „Because it is impossible to make close() blocking, we can't force the client app to wait for file uploads to complete.“ RSAF blockiert deshalb Umbenennen, Kopieren und Verschieben offener Dateien. README: nur `lseek`, `read`, `pread`, `write`, `pwrite`, `fsync`; „`ftruncate()` is not“ unterstützt; zufällige Schreibzugriffe laden die ganze Datei neu hoch. |
| RSAF, Roots | — | `ROOT_FLAGS = FLAG_SUPPORTS_CREATE or FLAG_SUPPORTS_IS_CHILD`; `FLAG_LOCAL_ONLY` nur mit Einstellung „pretendLocal“ | — | README: kann „pretend to be local storage for apps that block remote SAF roots“. `isChildDocument` vergleicht Pfadkomponenten ohne I/O. `notifyChange` auf Kind-URIs nach Anlegen, Umbenennen, Löschen, Kopieren/Verschieben; auf `buildRootsUri` bei Einstellungsänderung. `EXTRA_LOADING` wird nicht genutzt. |
| Material Files | nur `FileProvider` (`me.zhanghai.android.files.file.FileProvider`, nicht exportiert); **kein** DocumentsProvider (Manifestkommentar: „Unusable until we implement DocumentsProvider.“) | lokale Linux-Pfade: `ParcelFileDescriptor.open(path.toFile(), modeBits)`; alle anderen (SMB, SFTP, FTP, WebDAV): `storageManager.openProxyFileDescriptorCompat(modeBits, ChannelCallback(channel), callbackHandler)` | ein gemeinsamer `HandlerThread("FileProvider.CallbackThread")` für alle Dateien | Anhängen-Modus wird abgelehnt (`require(!hasBits(ParcelFileDescriptor.MODE_APPEND))`). |
| Nextcloud Android | `DocumentsProvider` (`DocumentsStorageProvider`) | lädt die Datei vollständig herunter (eigener Thread, auf den gewartet wird), dann `ParcelFileDescriptor.open(file, accessMode, handler, listener)`; Upload im Schließ-Listener | Listener-`Handler` auf `context.getMainLooper()` | Kommentar: „As we can't upload the file synchronously, let's at least update its metadata already.“ `CancellationSignal` wird nicht ausgewertet. `queryChildDocuments` setzt `EXTRA_LOADING`, wenn der Ordner älter als 1 Minute ist, startet eine Hintergrundaktualisierung und sendet danach `notifyChange`. Bei aktiver App-Sperre liefert `queryRoots` einen leeren Cursor. `isChildDocument` vergleicht Pfade und Konto. |
| Cryptomator Android | nur `androidx.core.content.FileProvider`; kein DocumentsProvider | entschlüsselte Kopie im App-Speicher, Weitergabe per FileProvider-URI, Upload nach Rückkehr in die App bei Änderung | — | Doku (`docs.cryptomator.org/android/access-vault/`): „If the content has changed, the upload process starts.“ Issue #35 („Allow vault access by other apps through Document Provider“) offen seit 2017-01-26; letzter Maintainer-Stand 2024-02-15: Arbeit Aug.–Okt. 2023, dann pausiert, weil die Datenbankverwaltung zuerst umgebaut werden musste; Issue #591 (2025) als Duplikat geschlossen und gesperrt. Keine Maintainer-Aussage zu `openProxyFileDescriptor`. |

**Leistung:** Belegt ist nur die Obergrenze von 128 KiB je FUSE-Anfrage und der Weg Client → Kernel-FUSE → `FuseAppLoop`-Thread → Handler → Callback. Messwerte für Durchsatz oder Latenz liegen in keiner gelesenen Quelle vor (Lücke L-A3).

**Folgerungen für Smart Explorer (als Folgerung):**
1. Für E4/E8 mit Lesen und Schreiben ist `openProxyFileDescriptor` der einzige belegte Weg ohne vollständige Klartextkopie auf dem Gerät; Nextclouds Vollkopie-Weg und Cryptomators FileProvider-Kopie legen Klartext ab, was für Tresore (E8) dem Ziel „kein Klartext auf Platte“ widerspricht.
2. Je geöffneter Datei ein eigener Handler-Thread und ein nicht blockierendes `onRelease` sind nach RSAF nötig, um einen system_server-Watchdog-Neustart zu vermeiden. Die Ursache liegt laut RSAF-Kommentar in vold; eine AOSP-Bestätigung wurde nicht gefunden (Lücke L-A4).
3. Weil `close()` beim Client nicht auf den Upload wartet, muss der Rückschreibvertrag der Mount-Engine (`flush`, `close`, `FlushOutcome`, siehe Lesung `laufwerke-kern.md` §3) auf Android über `onFsync` (blockierend, mit errno) und einen asynchronen Abschluss nach `onRelease` abgebildet werden; ein Fehler nach `onRelease` kann dem Client nicht mehr gemeldet werden.
4. Es gibt kein Kürzen über den Proxy-FD; `"wt"`/`"rwt"` muss der Anbieter beim Öffnen selbst ausführen.

### A3 — Sichtbarkeit und dynamische Roots

**Erkennung durch DocumentsUI (`ProvidersCache.java`):** DocumentsUI ruft je Nutzer `queryIntentContentProviders(new Intent(DocumentsContract.PROVIDER_INTERFACE), 0)` auf und lädt die Roots aller gefundenen Anbieter auf einem Pool mit `NUM_THREADS = 10`. Ein Anbieter wird übersprungen, wenn er nicht exportiert ist, keine URI-Grants erlaubt oder Lese- und Schreibberechtigung nicht beide `MANAGE_DOCUMENTS` sind. Je Authority registriert DocumentsUI einen `RootsChangedObserver` auf `DocumentsContract.buildRootsUri(authority)` (mit Nachkommen); `onChange` lädt die Roots dieser Authority neu. Pakete mit `FLAG_STOPPED` werden im normalen Durchlauf übersprungen und erst bei blockierenden Root-Abfragen nachgeladen. Wartezeiten: `FIRST_LOAD_TIMEOUT_MS = 5000`, danach 15000 ms; bei Überschreitung protokolliert DocumentsUI eine Warnung und fährt fort.

**Dynamisches Hinzufügen und Entfernen:** Leitfaden: „If your document provider connects to a dynamic set of roots—for example, to a USB device that might be disconnected or an account that the user can sign out from—you can update the document UI to stay in sync with those changes using the `ContentResolver.notifyChange()` method“ mit `notifyChange(DocumentsContract.buildRootsUri(AUTHORITY), null)`. Für Sperren: „The solution is to return zero roots in your implementation of `queryRoots()`“ und danach `notifyChange`. Nextcloud setzt das bei aktiver App-Sperre um (leerer Root-Cursor). **Folgerung:** Gerät offline/online (E4) und Tresor gesperrt/entsperrt (E8) lassen sich als Root-Menge abbilden, die bei jeder Zustandsänderung neu gemeldet wird; ein gesperrter Tresor erscheint dann nicht als Root.

**Ordneraktualisierung:** DocumentsUI (`DirectoryLoader.java`) registriert einen `ContentObserver` auf dem Kind-Cursor des Anbieters und lädt bei Änderung neu. Der Anbieter muss dazu `setNotificationUri` setzen (Javadoc `queryChildDocuments`). Wo DocumentsUI `EXTRA_LOADING` auswertet, ist in `DirectoryLoader.java` nicht belegt (keine Referenz in dieser Datei; Lücke L-A2).

**Android-11-Einschränkungen (Seite „documents-files“, Stand 2026-10-16):** „On Android 11 (API level 30) and higher, you cannot use the `ACTION_OPEN_DOCUMENT_TREE` intent action to request access to the following directories: The root directory of the internal storage volume. The root directory of each SD card volume that the device manufacturer considers to be reliable … The `Download` directory.“ und keine Einzeldateiauswahl in `Android/data/` und `Android/obb/`. Die Seite sagt nicht, dass diese Sperren für Roots von Drittanbietern gelten. Für eigene Ordner steht dem Anbieter `Document.FLAG_DIR_BLOCKS_OPEN_DOCUMENT_TREE` zur Verfügung. **Folgerung:** Ein Smart-Explorer-Root (Gerät oder Tresor) ist für `ACTION_OPEN_DOCUMENT_TREE` wählbar, sofern `isChildDocument` implementiert und `FLAG_SUPPORTS_IS_CHILD` gesetzt ist.

**Lokale-nur-Filter:** Apps können mit `Intent#EXTRA_LOCAL_ONLY` nur Roots mit `FLAG_LOCAL_ONLY` anfordern; RSAF bietet deshalb optional „pretendLocal“ an. **Folgerung:** Ein entsperrter Tresor auf lokalem Speicher dürfte `FLAG_LOCAL_ONLY` wahrheitsgemäß setzen; ein entferntes Gerät nicht.

**Dauerhafte Grants:** Clients behalten Zugriff über Neustarts nur mit `takePersistableUriPermission`; „your app doesn't retain access to the URI if the associated document is moved or deleted“. `UriGrantsManagerService`: `MAX_PERSISTED_URI_GRANTS = 512` je UID; beim Überschreiten werden die ältesten dauerhaften Grants entfernt. **Folgerung:** Dokument-IDs müssen über Sperren, Neustarts und Verbindungswechsel stabil bleiben, sonst verlieren Clients ihre gespeicherten Grants.

### A4 — Grenzen

**Apps nur mit Dateipfaden:** Ein Dokumentanbieter liefert `content://`-URIs und `ParcelFileDescriptor`s, keine Dateipfade. Belegt ist das durch den Vertrag selbst (`openDocument` gibt einen `ParcelFileDescriptor` zurück) und durch RSAFs README, das als Ausweg nur „pretend local“ für Apps nennt, die entfernte Roots sperren. **Folgerung:** Apps, die nur `java.io.File`-Pfade verarbeiten, erreichen Inhalte eines Dokumentanbieters nicht; ein systemweiter POSIX-Pfad ist ohne Root-Rechte auf Android nicht herstellbar (die AppFuse-Einhängung des Proxy-FD ist kein für Apps nutzbarer Pfadbaum). Ein Primärbeleg für den zweiten Halbsatz wurde nicht abgerufen (Lücke L-A5).

**Medienscanner:** Keine gelesene Quelle belegt, dass MediaStore Inhalte von Drittanbieter-DocumentsProvidern indiziert. **Annahme:** MediaStore indiziert nur Dateien auf den vom System eingehängten Speichervolumes; Inhalte eines Smart-Explorer-Anbieters erscheinen daher nicht in Galerie-Apps, die nur MediaStore abfragen. Nicht belegt (Lücke L-A6).

**Prozessstart:** Belegt: DocumentsUI findet Anbieter über den Paketmanager und lädt auch gestoppte Pakete bei Bedarf nach (`ProvidersCache`, siehe A3). „Processes and app lifecycle“: Ein Prozess wird erzeugt, „when some of its code needs to run“; und „if process A … is using a `ContentProvider` in process B, then process B's classification is always at least as important as process A's.“ **Folgerung:** Solange ein Client im Vordergrund den Anbieter nutzt, ist der Smart-Explorer-Prozess mindestens gleich wichtig. Nicht belegt ist, ob ein offener Proxy-FD allein (nach Rückkehr des `openDocument`-Aufrufs) die Prozesswichtigkeit weiter anhebt (Lücke L-A7). Die Rust-Laufzeit und die Verbindungsdaten müssten beim Kaltstart aus `ContentProvider.onCreate` heraus bereitstehen, bevor `queryRoots` antwortet; die Wartezeit von DocumentsUI beträgt 5 s beim ersten Laden.

**Hintergrund bei langen Vorgängen:** RSAF startet beim Öffnen eines Proxy-FD einen Vordergrunddienst (`KeepAliveService`), aber nur unter der Bedingung `Permissions.isInhibitingBatteryOpt(context)`; README empfiehlt Benachrichtigungen und deaktivierte Akkuoptimierung, „so RSAF can run reliably after a client app closes a file, which is when uploads begin“, und nutzt `RECEIVE_BOOT_COMPLETED`, um unterbrochene Uploads fortzusetzen. Vorbefund `docs/refs/android-platform.md` §1 (Stand 2026-09-25): Hintergrundstart eines Vordergrunddienstes ist ab Android 12 nur mit einer Ausnahme erlaubt, darunter „User has disabled battery optimization for the app“; `dataSync` ist ab targetSdk 35 auf 6 h je 24 h im Hintergrund begrenzt. **Folgerung:** RSAFs Bedingung entspricht genau dieser Ausnahme (Semantik von `isInhibitingBatteryOpt` nicht gelesen). Für Smart Explorer heißt das: Ein Upload nach `onRelease` ohne Akku-Ausnahme kann nicht verlässlich per Vordergrunddienst abgesichert werden; WorkManager ist die dokumentierte Alternative laut Vorbefund.

## 3. Vergleichstabelle

| Kriterium | RSAF v4.14 | Material Files | Nextcloud Android | Cryptomator Android 2.0.1 / develop |
|---|---|---|---|---|
| DocumentsProvider | ja | nein (nur FileProvider) | ja | nein (nur FileProvider; Issue #35 offen) |
| FD-Mechanik | `openProxyFileDescriptor` | lokal echte Datei, sonst `openProxyFileDescriptorCompat` | Vollkopie + `ParcelFileDescriptor.open` mit Schließ-Listener | entschlüsselte Kopie + FileProvider-URI |
| Handler-Thread | je Datei eigener `HandlerThread` | ein gemeinsamer `HandlerThread` | Hauptlooper für Schließ-Listener | — |
| Klartext/Kopie auf Platte | rclone-VFS-Schreibcache (`vfs_cache_mode=writes`) | nein (Kanal-Streaming) | ja, vollständige Datei | ja, entschlüsselte Kopie im App-Speicher |
| Rückschreiben | asynchron nach `onRelease` | über den Kanal, Zeitpunkt nicht gelesen | Upload nach Schließen | Upload nach Rückkehr in die App |
| `CancellationSignal` | nicht gelesen | — | nicht ausgewertet | — |
| `EXTRA_LOADING` | nein | — | ja (älter als 1 min) | — |
| `isChildDocument` | ja, ohne I/O | — | ja, Pfadvergleich | — |
| Root-Flags | `FLAG_SUPPORTS_CREATE`, `FLAG_SUPPORTS_IS_CHILD`, optional `FLAG_LOCAL_ONLY` | — | nicht gelesen (Datei `RootCursor.java` unter dem vermuteten Pfad nicht gefunden) | — |
| Sperre/Abmeldung | Roots-Notify bei Einstellungsänderung | — | leere Roots bei App-Sperre | Tresor sperrt im Hintergrund/bei Bildschirm aus (Doku) |
| Vordergrunddienst | `KeepAliveService` bei Akku-Ausnahme | — | nicht gelesen | — |

## 4. Widersprüchliche Angaben

- Leitfaden „Create a custom document provider“ empfiehlt für sehr große Netzdateien „pipes or sockets“; das `openDocument`-Javadoc beschränkt Pipes auf reine `"r"`- oder `"w"`-Modi. Der Leitfaden erwähnt `openProxyFileDescriptor` nicht, obwohl `StorageManager` genau diesen Fall (Netzfreigabe, Cloud, Entschlüsselung bei Bedarf) beschreibt. Kein inhaltlicher Widerspruch, aber der Leitfaden ist für `"rw"` unvollständig.
- `StorageManager.java` enthält den Kommentar „Disabled now that FUSE has been replaced by sdcardfs“ in `maybeTranslateEmulatedPathToInternal`, während `openProxyFileDescriptor` im selben Stand AppFuse (FUSE) nutzt. Der Kommentar betrifft die emulierte Speicherpfad-Übersetzung, nicht AppFuse; aufgelöst ist der Widerspruch in der Quelle nicht.
- Die Leitfadenseite nennt als Mindestziel API 19 und ein Beispiel `targetSdkVersion="19"`; das ist veralteter Beispielstand, kein Widerspruch zum Vertrag.

## 5. Offene Lücken

- L-A1: `queryDocument`-Javadoc nicht wörtlich zitiert.
- L-A2: Auswertung von `EXTRA_LOADING` und `AuthenticationRequiredException` in DocumentsUI außerhalb von `DirectoryLoader.java` nicht gelesen.
- L-A3: Keine Messwerte zu Durchsatz und Latenz des Proxy-FD.
- L-A4: Der vold-Sperr-/Watchdog-Mechanismus ist nur durch RSAF-Kommentare belegt, nicht durch AOSP-Quellen (`vold`, `StorageManagerService.AppFuseMountScope`) oder einen Fehlerbericht.
- L-A5: Kein Primärbeleg abgerufen, dass AppFuse-Einhängungen für Drittapps nicht als Pfad nutzbar sind.
- L-A6: Kein Primärbeleg zum Verhalten von MediaStore gegenüber Drittanbieter-Roots.
- L-A7: Nicht belegt, ob ein offener Proxy-FD die Prozesswichtigkeit des Anbieters anhebt.
- L-A8: Referenzseiten auf developer.android.com nicht im Seitentext gelesen; API-Level-Angaben (`openProxyFileDescriptor` ab 26, Flag-Einführungsversionen) nicht erneut verifiziert. Bei `minSdk = 30` betrifft das nur Flags, die nach API 30 eingeführt sein könnten; keines der zitierten Flags ist als nach API 30 eingeführt bekannt (**Annahme**).
- L-A9: Root-Flags und Vordergrunddienst-Nutzung von Nextcloud Android nicht gelesen.
- L-A10: AOSP-Quellstand ist 2025-03-27 (archivierter Spiegel); Änderungen in Android 16 und 17 an `DocumentsProvider`, `FuseAppLoop` und DocumentsUI sind nicht geprüft.
