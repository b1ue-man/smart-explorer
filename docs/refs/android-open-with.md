# Android „Öffnen mit" für Ordner/Dateien — API 30–36 / AOSP `frameworks/base` + `DocumentsUI` (master)

Quelle: https://android.googlesource.com/platform/frameworks/base (`core/java/android/provider/DocumentsContract.java`, `core/java/com/android/internal/content/FileSystemProvider.java`) · https://github.com/aosp-mirror/platform_frameworks_base (`packages/ExternalStorageProvider/.../ExternalStorageProvider.java`, master) · https://android.googlesource.com/platform/packages/apps/DocumentsUI (`AndroidManifest.xml`, refs/heads/main) · https://developer.android.com/training/data-storage/shared/documents-files · https://developer.android.com/training/data-storage/manage-all-files · https://developer.android.com/training/package-visibility · https://developer.android.com/training/package-visibility/automatic · https://developer.android.com/about/versions/12/behavior-changes-12 · https://developer.android.com/reference/android/os/FileUriExposedException · http://www.openintents.org/action/android-intent-action-view/file-directory · https://github.com/openintents/filemanager/issues/87 · Abgerufen: 2026-09-26

## Intent für „Ordner öffnen" (DocumentsUI-Muster)

DocumentsUI selbst deklariert genau diesen Intent-Filter für seine `FilesActivity` (Quelle: `AndroidManifest.xml`, `platform/packages/apps/DocumentsUI`, `refs/heads/main`):

```xml
<activity
    android:name=".files.FilesActivity"
    android:documentLaunchMode="intoExisting"
    android:exported="true"
    android:theme="@style/LauncherTheme">

    <intent-filter>
        <action android:name="android.intent.action.VIEW" />
        <category android:name="android.intent.category.DEFAULT" />
        <data android:mimeType="vnd.android.document/directory" />
    </intent-filter>

</activity>
```

`vnd.android.document/directory` ist der einzige MIME-Typ für Ordner, der sich primärquellig in AOSP nachweisen lässt: `DocumentsContract.Document.MIME_TYPE_DIR`, Konstante in `core/java/android/provider/DocumentsContract.java`:

```java
/**
 * MIME type of a document which is a directory that may contain
 * additional documents.
 *
 * @see #COLUMN_MIME_TYPE
 */
public static final String MIME_TYPE_DIR = "vnd.android.document/directory";
```

Ein sendender Client (Dateimanager, Syncthing, Downloads-Benachrichtigung o. ä.) baut damit typischerweise:

```java
Intent intent = new Intent(Intent.ACTION_VIEW);
intent.setDataAndType(treeOrDocumentUri, DocumentsContract.Document.MIME_TYPE_DIR);
intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
```

**Zu `resource/folder` und `inode/directory` (aus dem Auftrag):** Für beide MIME-Strings ließ sich in dieser Recherche **keine** primäre AOSP-/`developer.android.com`-Quelle finden. `resource/folder` taucht nur in Foren-/Community-Diskussionen ohne Quellenbeleg auf; `inode/directory` ist die klassische Unix/GNOME-Desktop-Konvention, aber keine von Google dokumentierte Android-Konstante. Selbst die OpenIntents-Konvention (`http://www.openintents.org/action/android-intent-action-view/file-directory`, ältere, aber einschlägige Community-Spezifikation für Datei-Intents) nennt für „Ordner ansehen" ausschließlich `ACTION_VIEW` + `vnd.android.document/directory`, optional mit dem Extra `org.openintents.extra.ABSOLUTE_PATH` (String), und explizit **keine** anderen MIME-Typen. Offene Frage, nicht mit Primärquelle klärbar: ob/welche konkreten Closed-Source-Dateimanager `resource/folder` oder `inode/directory` tatsächlich senden.

Alternative/ergänzend zu einem reinen `ACTION_VIEW`+MIME-Intent: Ein Tree-Picker-Aufruf über `ACTION_OPEN_DOCUMENT_TREE` mit `EXTRA_INITIAL_URI` (setzt nur den Startpunkt eines Systempickers, öffnet nicht direkt in einer Fremd-App) – laut `developer.android.com/training/data-storage/shared/documents-files` ist das der dokumentierte Weg, wenn *man selbst* einen Ordner-Picker aufruft, nicht der Weg, wie eine andere App direkt einen Ordner *in* Smart Explorer öffnet.

## Document-ID-Mapping (`content://com.android.externalstorage.documents/...`)

URI-Struktur laut Klassen-Doku von `DocumentsContract` (`core/java/android/provider/DocumentsContract.java`):

```
content://<authority>/tree/<treeDocumentId>/document/<documentId>
```

Beispiel: `content://com.android.externalstorage.documents/tree/primary%3ADownload/document/primary%3ADownload%2Ffoo.txt` (Doc-ID `primary:Download` bzw. `primary:Download/foo.txt`, URL-encodiert).

Auflösung der Doc-ID-Strings selbst passiert nicht im generischen `DocumentsContract`, sondern im jeweiligen Provider. Für `ExternalStorageProvider` (Authority `com.android.externalstorage.documents`) gilt laut Klassen-Javadoc **„Documents ID format: `root:path/to/file`"** (`packages/ExternalStorageProvider/src/com/android/externalstorage/ExternalStorageProvider.java`):

- Root-ID für den internen/emulierten Speicher ist die Konstante `ROOT_ID_PRIMARY_EMULATED = DocumentsContract.EXTERNAL_STORAGE_PRIMARY_EMULATED_ROOT_ID`, deren Wert `"primary"` ist → Doc-IDs wie `primary:Pictures`.
- Root-ID für eine Wechselkarte/USB-Volume ist `volume.getFsUuid()` – bei FAT32 typischerweise im Format `XXXX-XXXX` (Volume-Seriennummer) → Doc-IDs wie `1234-5678:dir`.
- Rückwandlung Doc-ID → `File` (`buildFile`, `getFileForDocId`, `getRootFromDocId`):

```java
private RootInfo getRootFromDocId(String docId) throws FileNotFoundException {
    final int splitIndex = docId.indexOf(':', 1);
    final String tag = docId.substring(0, splitIndex);
    // tag ("primary", "XXXX-XXXX", ...) -> RootInfo aus mRoots-Map
    ...
}

private File buildFile(RootInfo root, String docId, boolean mustExist)
        throws FileNotFoundException {
    final int splitIndex = docId.indexOf(':', 1);
    final String path = docId.substring(splitIndex + 1);
    File target = root.visiblePath != null ? root.visiblePath : root.path;
    ...
    target = new File(target, path).getCanonicalFile();
    ...
}
```

D. h. konkret: Präfix bis zum ersten `:` (Suche beginnt bei Index 1, nicht 0) ist die Root-Kennung, alles danach ist ein Relativpfad unterhalb der Root. `root.path`/`root.visiblePath` ist die tatsächliche Basis auf dem Dateisystem (z. B. `/storage/emulated/0` für `primary`, `/storage/XXXX-XXXX` für eine SD-Karte) – diese Zuordnung Root-ID → Basisverzeichnis kennt **nur** der Systemprozess/`ExternalStorageProvider` selbst; eine fremde App kann sie nicht zuverlässig „raten", sondern muss entweder über `DocumentFile`/`ContentResolver` mit der Content-URI arbeiten oder (nur mit `MANAGE_EXTERNAL_STORAGE`, s.u.) eigene Heuristiken für Standardpfade (`primary` → `Environment.getExternalStorageDirectory()`) verwenden.

`file://`-URIs sind trivial zu mappen (`Uri.getPath()` liefert direkt den Dateisystempfad), aber: seit Android 7.0 (API 24) wirft das Weitergeben einer `file://`-Uri an eine andere App per Intent (aus einer App mit `targetSdkVersion >= 24`) eine `FileUriExposedException` (`developer.android.com/reference/android/os/FileUriExposedException`). Sender müssen dafür `content://` über `FileProvider` verwenden. Für den *Empfänger* (Smart Explorer) heißt das: Ein eingehender `file://`-URI ist zulässig, aber typischerweise nur von Apps mit `MANAGE_EXTERNAL_STORAGE` oder alten `targetSdk<24`-Apps zu erwarten; der Normalfall moderner Sender ist ein `content://`-Tree-/Document-URI.

## `MANAGE_EXTERNAL_STORAGE` – was eine App damit annehmen darf

Laut `developer.android.com/training/data-storage/manage-all-files`:
- Mit dieser Permission darf die App Dateien/Ordner sowohl über die `MediaStore`-API als auch über **direkte Dateipfade** (`java.io.File`) lesen/schreiben – sie muss also nicht über SAF/Content-URIs gehen.
- Ausgenommen bleiben `Android/data/` und die meisten Unterordner von `Android/obb/` bzw. `sdcard/Android/...` anderer Apps – **App-private Verzeichnisse fremder Apps sind auch mit `MANAGE_EXTERNAL_STORAGE` nicht zulässig**, sie erscheinen zwar als Unterordner, bleiben aber gesperrt.
- Google-Play-Richtlinie (`support.google.com/googleplay/.../10467955`, high-risk permission, „restricted use") empfiehlt, `MANAGE_EXTERNAL_STORAGE` nur zu beantragen, wenn SAF/MediaStore für den Zweck nicht ausreichen.

Praktisch: Eine App mit dieser Permission darf einen erhaltenen Pfad/`file://`-URI direkt per `File`-API öffnen, ohne über `DocumentsContract`/`ContentResolver` zu gehen – **aber nur für Pfade außerhalb der `Android/data`/`Android/obb`-Sperrzonen anderer Apps.**

## Android 11+ Sicherheits-/Sichtbarkeits-Caveats

- **`android:exported` Pflichtangabe (Android 12, API 31+):** Jede Activity/Service/Receiver mit Intent-Filter muss `android:exported` explizit setzen, sonst lässt sich die App auf Android 12+ nicht installieren (`developer.android.com/about/versions/12/behavior-changes-12`). Für eine „Ordner in Smart Explorer öffnen"-Activity mit dem obigen `ACTION_VIEW`/`vnd.android.document/directory`-Filter ist das zwingend `android:exported="true"` (ohne `exported=true` kann keine fremde App die Activity per `startActivity` erreichen).
- **Package Visibility (Android 11, API 30+):** Ab API 30 filtert das System standardmäßig, welche installierten Pakete eine App überhaupt sehen/adressieren kann (`developer.android.com/training/package-visibility`). Für den hier relevanten Fall gilt aber die **„automatische Sichtbarkeit"**-Regel (`developer.android.com/training/package-visibility/automatic`): Jede App, die eine `Intent`-Auflösung macht, sieht automatisch jede andere App, deren `<intent-filter>` zu diesem konkreten Intent passt (Signatur-Match), auch ohne `<queries>`-Deklaration. D. h. Smart Explorer muss selbst **kein** `<queries>`-Element deklarieren, damit *andere* Apps es im „Öffnen mit"-Chooser für `ACTION_VIEW`+`vnd.android.document/directory` finden – die Sichtbarkeit ergibt sich aus dem übereinstimmenden Intent-Filter der sendenden App gegenüber Smart Explorers Manifest-Eintrag.
- **Nie dem Pfad blind vertrauen:** Da jede beliebige App (mit passendem Export/Sichtbarkeit) diesen Intent mit einer selbstgewählten `content://`/`file://`-URI oder Doc-ID senden kann, muss die empfangende Activity die aufgelöste URI/den aufgelösten Pfad validieren (existiert das Ziel wirklich als Verzeichnis? liegt es innerhalb eines für die App zulässigen Bereichs?), bevor sie ihn öffnet. Für Content-URIs ist `DocumentsContract.isDocumentUri(context, uri)`/`isTreeUri(uri)` der dokumentierte Prüfpunkt (`core/java/android/provider/DocumentsContract.java`); ein Aufruf mit einer manipulierten/nicht existierenden Doc-ID scheitert beim Provider mit `FileNotFoundException` statt mit undefiniertem Verhalten (siehe `buildFile`/`getFileForDocId` oben: `mustExist`-Prüfung wirft `FileNotFoundException`, falls die Datei fehlt).
- **App-private Pfade ablehnen:** Weder SAF (`ACTION_OPEN_DOCUMENT_TREE` kann laut `developer.android.com/training/data-storage/shared/documents-files` seit Android 11 nicht mehr auf `Android/data/`, `Android/obb/`, die Wurzel des internen Speichers oder zuverlässige-SD-Karten-Wurzeln zeigen) noch `MANAGE_EXTERNAL_STORAGE` (s.o., App-private Verzeichnisse anderer Apps bleiben gesperrt) erlauben Zugriff auf App-private Verzeichnisse fremder Apps. Eine über einen „Öffnen mit"-Intent eingehende Pfadangabe, die scheinbar in ein `Android/data/<anderes-package>/`-Verzeichnis oder in Smart Explorers eigenes `getFilesDir()`/`getCacheDir()` zeigt, sollte deshalb zurückgewiesen werden statt geöffnet zu werden – das ist konsistent mit dem, was das Betriebssystem selbst an dieser Stelle bereits verweigert.

## Offene Fragen

- Kein Primärquellen-Beleg dafür, dass reale Closed-Source-Dateimanager (z. B. bekannte Play-Store-Apps) tatsächlich `resource/folder` oder `inode/directory` als MIME-Typ senden; nur `vnd.android.document/directory` ist AOSP-belegt.
- Ob ein sendender Dritt-Client bei einem lokalen (nicht-SAF) Pfad eher eine `content://`-Tree-URI (per eigenem `FileProvider`) oder einen rohen `file://`-URI schickt, hängt vom jeweiligen Sender ab und ließ sich nicht anhand eines einzelnen Referenz-Callers in AOSP nachweisen (DocumentsUI selbst ist nur Empfänger/Ziel dieses Intents, kein dokumentierter Standard-Sender im durchsuchten Quellcode).
