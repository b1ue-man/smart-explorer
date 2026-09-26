# Zugriff auf Android/data und Android/obb fremder Apps (MANAGE_EXTERNAL_STORAGE, SAF, Shizuku)

Quelle: https://developer.android.com/training/data-storage/manage-all-files · https://developer.android.com/about/versions/11/privacy/storage · https://developer.android.com/training/data-storage/shared/documents-files · https://www.esper.io/blog/android-dessert-bites-28-file-manager-loophole-closed-73891524 · https://github.com/RikkaApps/Shizuku-API · https://raw.githubusercontent.com/RikkaApps/Shizuku-API/master/README.md · https://github.com/RikkaApps/Shizuku · https://raw.githubusercontent.com/RikkaApps/Shizuku/master/README.md · https://shizuku.rikka.app/ · https://github.com/RikkaApps/Shizuku/blob/master/LICENSE · https://github.com/RikkaApps/Shizuku/issues/1574 · https://github.com/zhanghai/MaterialFiles/issues/1572 · https://github.com/MrHyperIon101/shizuku-apps · https://developer.android.com/studio/debug · Abgerufen: 2026-09-26

## 1. MANAGE_EXTERNAL_STORAGE – offizielle Einschränkung seit Android 11

`developer.android.com/training/data-storage/manage-all-files` (wörtliche Zitate):

> "Write access to all internal storage directories⁠ except `/Android/data/`, `/sdcard/Android`, and most subdirectories of `/sdcard/Android`. This write access includes direct file path access."

> "Apps that are granted this permission still can't access the app-specific directories⁠ that belong to other apps, because these directories appear as subdirectories of `Android/data/` on a storage volume."

Das heißt: `MANAGE_EXTERNAL_STORAGE` gibt einer App zwar lese-/schreibrechte auf praktisch den gesamten gemeinsamen externen Speicher, **explizit ausgenommen** sind `Android/data/` (und `Android/obb/`, da als Unterverzeichnis der gleichen geschützten Struktur behandelt) für die App-spezifischen Verzeichnisse *anderer* Apps.

## 2. SAF: `ACTION_OPEN_DOCUMENT_TREE` kann `Android/data`/`Android/obb` seit Android 11 nicht mehr auswählen; DocumentsUI-Loophole (historisch)

`developer.android.com/training/data-storage/shared/documents-files` (wörtliches Zitat):

> "On Android 11 (API level 30) and higher, you cannot use the `ACTION_OPEN_DOCUMENT` intent action to request that the user select individual files from the following directories: The `Android/data/` directory and all subdirectories. The `Android/obb/` directory and all subdirectories." … "Furthermore, on Android 11 (API level 30) and higher, you cannot use the `ACTION_OPEN_DOCUMENT_TREE` intent action to request that the user select individual files from the following directories: The `Android/data/` directory and all subdirectories. The `Android/obb/` directory and all subdirectories."

`developer.android.com/about/versions/11/privacy/storage` bestätigt dieselbe Einschränkung separat für die Storage-Updates-Übersichtsseite von Android 11 und nennt einen Test-Hinweis (Aktivierung über das `RESTRICT_STORAGE_ACCESS_FRAMEWORK`-Compat-Flag bzw. automatisch bei `targetSdkVersion` 30+).

**Historische Lücke (nur zur Einordnung, keine Option für uns):** Laut Esper ("Android 13 Makes File Managers Less Useful by Fixing a Loophole") und einer ergänzend recherchierten Community-Quelle ließ sich unter Android 11/12(L) über den *initial location*-Parameter von `ACTION_OPEN_DOCUMENT`/`ACTION_OPEN_DOCUMENT_TREE` trotzdem gezielt auf `Android/data`/`Android/obb` zeigen ("apps can set the initial location of the document chooser to be /Android/data or /Android/obb, which is intended functionality of SAF but Google seemingly didn't consider when implementing Android's restrictions on those directories"). Diese Lücke wurde laut derselben Quelle mit Android 13 geschlossen ("The loophole has been closed in Android 13, limiting the ability of third-party file managers to actually do their job"; laut Community-Berichten wurde eine verbleibende Variante zusätzlich per Sicherheits-Patch im September 2023 in einer neuen DocumentsUI-Version geschlossen). Das ist rein historisch und **keine** heute nutzbare Option.

## 3. Shizuku

Was es ist (`raw.githubusercontent.com/RikkaApps/Shizuku-API/master/README.md`, wörtliches Zitat):

> "Shizuku API is the API provided by Shizuku and Sui. With Shizuku API, you can call your Java/JNI code with root/shell (ADB) identity."

Offizielle Website (`shizuku.rikka.app`, sinngemäß/paraphrasiert laut abgerufener Seite): Shizuku ermöglicht Apps, Systemschnittstellen direkt zu nutzen, ohne einen root-Shell-Prozess pro Aufruf starten zu müssen; die App verbindet sich stattdessen über Androids Binder-IPC mit einem einmal gestarteten "Shizuku-Server"-Prozess.

**`UserService` und Identität (Shizuku-API-README, wörtlich):**

> "The service runs in a different process and as the identity (Linux UID) of root (UID 0) or shell (UID 2000)."

D. h. `UserService`-Code einer App läuft – abhängig vom Backend – entweder als `root` (UID 0) oder, wenn Shizuku per ADB (Wireless Debugging) gestartet wurde, als `shell` (UID 2000, `com.android.shell`).

**Aktivierung (Shizuku-README, wörtlich/paraphrasiert je nach Abschnitt):**

> "Android 11 and above have built-in wireless debugging support, user can start Shizuku directly on the device."

> "On non-rooted devices, Shizuku needs to be manually restarted with adb every time on boot." (Rooted-Geräte: dauerhaft aktiv/persistiert über Reboots, laut README-Abschnitt zu Root-Aktivierung.)

**Android-14+-Einschränkungen für Shell-Zugriff auf `Android/data` – nicht abschließend bestätigt:** In der Recherche fanden sich widersprüchliche/nicht-autoritative Hinweise: Community-Quellen behaupten, `shell` (UID 2000) könne auf Android 14/15 weiterhin gezielt in `Android/data`/`Android/obb` lesen/schreiben ("the shell level is exactly what we need to read/write into /Android/data protected areas … it is now likely one of the only reliable non-root methods"), während ein offenes GitHub-Issue im Shizuku-Repository (`RikkaApps/Shizuku#1574`, Oktober 2025) berichtet, dass Shizuku 13.6.0 unter Android 16 (getestet auf einem Galaxy Z Fold 7) über mehrere Dateimanager (MiXplorer, MT Manager, Solid Explorer, Total Commander, X-plore, ZArchiver u. a.) **keinen** Zugriff auf `Android/data/<App>` für App-Backups erhält; eine Maintainer-Antwort oder Ursachenklärung liegt in dem Issue (Stand Abruf) nicht vor. Eine belastbare, primärquellenbasierte Aussage zu einer gezielten Android-14/15/16-Einschränkung für `shell`-Zugriff auf `Android/data` ließ sich daraus **nicht** ableiten – siehe Offene Punkte. Wichtig zur Abgrenzung: Berichte über eine strikte Abschottung von `/data/data` bzw. `/data/user/<n>/<pkg>` ("Google has completely locked down access to /data/**") betreffen den internen App-Speicher unter `/data`, **nicht** den in dieser Recherche relevanten externen Pfad `/storage/emulated/0/Android/data/<pkg>`.

**Lizenz:** `github.com/RikkaApps/Shizuku/blob/master/LICENSE` sowie mehrere unabhängige Quellen (u. a. GitHub-Repo-Metadaten) bestätigen **Apache License 2.0** für den Quellcode; die README ergänzt, dass davon Ausnahmen für Marken/Namen gelten: "the launcher icons are restricted from reuse, and the 'Shizuku' name and specific package identifiers cannot be adopted by derivative works", und dass selbst kompilierte APKs nicht in offiziellen Stores (Google Play, F-Droid, Amazon Appstore etc.) verteilt werden dürfen.

**Maven-Koordinaten / Mindest-API (Shizuku-API-README):**
- Gruppe `dev.rikka.shizuku`, Artefakte `api` (Client-API) und `provider` (ContentProvider zum Anfordern der Berechtigung), z. B. `implementation "dev.rikka.shizuku:api:$shizuku_version"` / `implementation "dev.rikka.shizuku:provider:$shizuku_version"`.
- Minimum-API laut README: **Android 6.0 (API 23)+**.

**Welche Dateimanager nutzen Shizuku (nur wo eine Quelle das belegt):**
- **ZArchiver** – bestätigt über zwei unabhängige Quellen: die kuratierte Liste `github.com/MrHyperIon101/shizuku-apps` ("Archive management program. Supports editing files using Root/Shizuku.") und Community-Anleitungen, die den konkreten Menüpfad "Settings > ROOT > Type of root access > Shizuku" nennen.
- **MT Manager** und **FV File Manager** – laut derselben kuratierten Liste (`shizuku-apps`) mit expliziter Nennung von `Android/data`/`Android/obb`-Zugriff über Shizuku.
- **MiXplorer** – laut einem XDA-Forenthread ("MiXplorer, an XDA home grown app, now has Shizuku integration") mit Shizuku-Unterstützung; das ist eine Sekundärquelle (Forum), keine offizielle MiXplorer-Dokumentation.
- **Material Files** – **kein** bestätigter Shizuku-Support: Das GitHub-Issue `zhanghai/MaterialFiles#1572` ("Shizuku support (enhancement)") ist als Duplikat von Issue #1274 geschlossen, d. h. es handelt sich um einen (zum Recherchezeitpunkt) noch offenen/nicht umgesetzten Feature-Wunsch, nicht um eine vorhandene Funktion.

## 4. Andere legitime Nicht-Root-Optionen

- **`adb run-as`:** Laut `developer.android.com/studio/debug` und dem allgemeinen ADB-Verhalten funktioniert `run-as <package>` nur für **debuggable** Pakete (`android:debuggable="true"`), womit es für den generischen Zugriff auf `Android/data` beliebiger, produktiv signierter Drittanbieter-Apps **nicht** einsetzbar ist; es adressiert außerdem primär den internen App-Speicher (`/data/data/<pkg>`), nicht `/sdcard/Android/data`.
- **App-eigener `DocumentsProvider` der besitzenden App:** Eine App kann freiwillig über einen eigenen, `exported`-`DocumentsProvider` Teile ihres `Android/data`-Verzeichnisses für andere Apps via SAF freigeben (offizielles, dokumentiertes Muster für Content-/Documents-Provider). Das ist jedoch **(Schluss)** keine generische Lösung für unser Szenario, weil sie serverseitig die Mitarbeit/Implementierung jeder einzelnen fremden App voraussetzt und in der Praxis kaum eine App so einen Provider für ihr `Android/data`-Verzeichnis anbietet.
- Root (Magisk/KernelSU) wäre die klassische Alternative, ist aber laut Aufgabenstellung kein zu betrachtender Pfad und wird hier nur der Vollständigkeit halber erwähnt, nicht weiter belegt.

## Offene Punkte

- Keine primärquellenbasierte, eindeutige Bestätigung oder Widerlegung einer gezielten Android-14/15/16-Einschränkung für `shell`-UID-Zugriff (UID 2000) auf `/storage/emulated/0/Android/data/<pkg>` gefunden; die Befundlage ist gemischt (Community-Aussagen "funktioniert" vs. ein offenes, ungeklärtes GitHub-Issue mit Fehlbericht auf Android 16).
- Der genaue Sicherheits-Patch-Level/-Monat, in dem die letzte Variante der DocumentsUI-Initial-URI-Lücke (nach dem in Android 13 dokumentierten Fix) endgültig geschlossen wurde, wurde nur über eine Sekundärquelle (Community-Bericht "September 2023") und nicht über einen AOSP-Commit-Diff verifiziert.
- Keine vollständige, autoritative Liste "aller" Shizuku-fähigen Dateimanager gefunden; die Aussagen zu MiXplorer, MT Manager, FV File Manager, ZArchiver und Material Files stützen sich auf die im Abschnitt 3 genannten Einzelquellen, nicht auf eine offizielle RikkaApps-Liste.
