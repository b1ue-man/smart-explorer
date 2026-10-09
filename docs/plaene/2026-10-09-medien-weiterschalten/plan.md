# Weiterschalten zwischen Medien nach „Öffnen“ aus Smart Explorer (2026-10-09)

Stand: 2026-10-09, Quelle aller Anforderungen: Nutzernachricht vom 2026-10-09
(„schau dir mal an wie programme und apps … das weiterschalten mit links rechts zwischen bildern
handhaben … wenn ich sie via des se öffne funktioniert das wechseln zwischen bildern z.B. nicht.
ich hätte gerne das du das funktionsfähig machst.“). Externe Befunde:
[`docs/refs/medien-weiterschalten-2026-10-09.md`](../../refs/medien-weiterschalten-2026-10-09.md).

## A — Erstplan aus dem Auftrag

| Nr. | Ergebnis | Quelle |
|---|---|---|
| R1 | Untersuchung: Wie handhaben etablierte Programme/Apps auf Windows, Linux und Android das Weiterschalten zwischen Bildern bzw. Medien, wenn ein Dateimanager sie öffnet? | Nachricht |
| R2 | Behebung: Ein über Smart Explorer geöffnetes Bild bzw. Medium lässt sich zum vorherigen/nächsten Medium weiterschalten, auf allen unterstützten Systemen (Windows, Linux, Android). | Nachricht |
| R3 | Eine Remote-Task-Suite, ein Remote-Release, keine lokalen Builds/Tests. | AGENTS.md |

Grundfunktion: Beim Öffnen muss der Betrachter den Kontext „übrige Medien desselben Ordners“
erhalten oder selbst ermitteln können. Der bestehende Weg (Doppelklick/Enter/Tippen → Betrachter)
bleibt; die Nachbarschaft kommt hinzu. Autorisierung: Umsetzung, Suite und Release (AGENTS.md,
Memory „decide-independently“). Nicht aus dem Prompt bekannt: auf welchem System der Nutzer den
Fehler sah (Annahme: Windows mit der Fotos-App; betrifft nur die Reihenfolge der Arbeit).

## B — Bestand (gelesen am 2026-10-09, Stand `4b9889f1`)

1. **Desktop-Öffnen** (`app/os/shared/remote_open.rs::open_file`): lokale Datei →
   `view_selection.rs::open_path` → `open_local_path(path, OpenMode::Default)`. Alle Wege
   (Doppelklick `table.rs`, Enter `frame_keyboard.rs` → `open_selection`, Suche
   `omni_accel.rs`) laufen hier zusammen.
2. **Windows** (`app/os/windows/platform.rs::shell_execute_path`): `ShellExecuteW(path)` ohne
   Nachbarkontext. Die Fotos-App zeigt dann keine Pfeile (R1-Befund Windows). Klassische
   Programme lesen den Ordner selbst und sind nicht betroffen. **Ursache für Windows.**
3. **Linux** (`app/os/linux_os.rs::open_local_path`): `xdg-open <echter pfad>`; Loupe, eog,
   Gwenview laden den Elternordner selbst (R1-Befund Linux). Kein Defekt im lokalen Fall.
4. **Remote am Desktop** (`remote_open.rs`, `transfer/os/shared/temp.rs::allocate_open_temp_path`):
   jede Remote-Datei wird in ein eigenes `e<zufall>/`-Verzeichnis geladen; der Betrachter sieht
   keine Nachbarn – auf Windows und Linux. Save-back, Wiederherstellungsmanifest und die
   50-Einträge-Grenze hängen an diesem Weg (`docs/REMOTE_EDIT.md`).
5. **Android** (`ui/files/FileActions.kt::openFile` → `FilesEffect.Open` →
   `system/Opener.kt::open`): genau eine `content://`-URI per `ACTION_VIEW`; Galerien können
   daraus keine Nachbarn bilden. **Ursache für Android.** Lokal liefert `FilesApi.open` den
   echten Pfad, remote lädt `FilesApi.fetch` (Task, registrierte Kopie unter `cache/open/<id>/`,
   `make_room` verdrängt unveränderte Kopien; die Bearbeitungsleiste zeigt nur geänderte).
   `BrowserTab.shownEntries()` liefert die angezeigte Reihenfolge (flach oder Scanfenster),
   `Entry.kind` = `image|video|audio|…` (`mobile/core/entry.rs::kind_of`).
6. **Abhängigkeiten**: `windows` 0.58 (bereits WinRT `Networking_Connectivity`), `windows-sys`
   0.59 mit `Win32_UI_Shell` (`AssocQueryStringW` ungated). Android minSdk 30, Compose-BOM
   2026.06.01 → foundation 1.11.4 (`HorizontalPager`, `transformable(canPan)`), Symbole
   `ic_play`, `ic_pause`, `ic_arrow_back`, `ic_more_vert` vorhanden.

## C — Entscheidungen

### C1 Windows lokal

Betrachtet (Quellen in der Refs-Datei, Abschnitt Windows):
- *Nachbarabfrage über `Launcher.LaunchFileWithOptionsAsync`* (Microsoft-Vertrag; Total
  Commander „alte Methode“). Wirkt für ältere Fotos-Versionen und andere Store-Apps.
- *`ms-photos:viewer?fileName=<URL-kodiert>`* (Total Commander seit 25.06.24, automatisch nach
  Fotos-Version seit 11.02.25; URL-kodiert wieder ab Fotos 2025.11030.12002.0).
- *Mehrfachauswahl übergeben* (Strg+A/Enter-Umgehung): verworfen, öffnet nur die Auswahl, nicht
  den Ordner.
- *Fotos durch Windows-Fotoanzeige ersetzen* (TC 2016): verworfen, ersetzt die vom Nutzer gewählte
  Standard-App.

Gewählt: Entscheidung nach der tatsächlichen Standard-App der Endung (`AssocQueryStringW`,
`ASSOCSTR_APPID`/`ASSOCSTR_PROGID`, ab Windows 10):
- Fotos (`Microsoft.Windows.Photos_8wekyb3d8bbwe!…`) mit Paketversion ≥ 2024 oder unbekannter
  Version → `ms-photos:viewer?fileName=` mit UTF-8-Prozentkodierung (alles außer
  `A–Z a–z 0–9 - . _ ~`). **Annahme** (Beleg: TC-Changelog): die seit Juni 2024 verteilte Fotos-
  App wertet die Nachbarabfrage von Dritten nicht mehr aus; aktuelle Versionen ≥ 2025.11030
  akzeptieren URL-kodierte Namen.
- Fotos mit Version < 2024 und jede andere Store-App (ProgID `AppX…` oder AUMID mit `!`) →
  `LaunchFileWithOptionsAsync` mit Nachbarabfrage über den Elternordner (flach, nach Name).
- Klassische Programme → unverändert `ShellExecuteW`.
- Nur für Medienendungen (`types::media_kind`), nur `OpenMode::Default`; „Öffnen mit“,
  Ordner und Bearbeitungsstarts (`launch_local_for_edit`) bleiben unverändert. Jeder Fehler
  (Assoziation, WinRT, Protokoll) fällt auf `ShellExecuteW` zurück. Die WinRT-Aufrufe laufen in
  einem eigenen Thread (windows-rs tritt dort implizit dem MTA bei; `IAsyncOperation::get` blockiert
  so nicht die Oberfläche).

### C2 Linux lokal
Keine Codeänderung: `xdg-open` übergibt den echten Pfad, die verbreiteten Betrachter laden den
Ordner selbst – identisch zu Nautilus/Dolphin. Abgesichert durch einen Test, dass der Pfad
unverändert übergeben wird (`linux_os.rs` bleibt unverändert; Test der gemeinsamen Entscheidung).

### C3 Android
Betrachtet: (a) Liste per Intent-Extra anhängen (Material Files) – wirkt nur für den eigenen
Betrachter; (b) `ClipData` mit vielen URIs – kein Galerievertrag, wird ignoriert;
(c) MediaStore-URIs statt eigener URIs – Wischen hängt weiter von der Galerie ab, Remote-Dateien
haben keine MediaStore-ID; (d) **eigener Betrachter** mit Wischen (Files by Google, Material
Files, Solid Explorer). Gewählt: (d).

- Tippen auf `image`/`video`/`audio` öffnet den Betrachter mit allen Einträgen derselben Gruppe
  aus `shownEntries()` in Anzeigereihenfolge: Gruppe „Bild/Video“ (wie Galerien) und Gruppe
  „Audio“ (wie Musikordner).
- Seite lokal: `FilesApi.open` (echter Pfad); sonst oder bei `unsupported` (ZIP):
  `FilesApi.fetch`. Bilder der Nachbarseiten werden vorgeladen (Pager compose ±1); Videos/Audio
  laden erst als aktuelle Seite. Verlassen einer Seite bricht ihren Ladetask ab.
- Bild: `ImageDecoder` auf Bildschirmgröße, Zoom per Zwei-Finger/Doppeltipp
  (`transformable(canPan = { scale > 1 })`, Pager-Wischen nur bei Zoom 1).
- Video/Audio: `VideoView` ohne eigene Touch-Bedienung, Steuerung in Compose (Play/Pause,
  Positionsregler, Zeit). Nicht aktuelle Seiten pausieren.
- Leiste: Zurück, Name, „n / m“, Menü „Öffnen mit…“ (bisheriger Chooser) und „In App öffnen“
  (bisheriger Direktweg). Tastatur ←/→ blättert.
- Zustände: „Wird geladen…“, Fehlermeldung mit „Mit App öffnen“, „Keine Vorschau für dieses
  Format“ (z. B. SVG/TIFF) mit „Mit App öffnen“.
- Einstellung „Medien im eigenen Betrachter öffnen“ (Standard an) unter „Dateiliste“; aus →
  bisheriger Direktweg. Langes Drücken → „Öffnen mit“ bleibt unverändert.

Bedienaufwand (geplant, nicht beobachtet): Nächstes Bild vorher = zurück zur Liste + nächste
Zeile tippen (C4, je Bild wiederholt, Kontextwechsel); nachher = eine Wischgeste (C0). Externe
App vorher = Tippen (C1); nachher = Menü → „In App öffnen“ (C2) oder Einstellung aus (C1).

### C4 Remote am Desktop (kreative Schleife, Stand)
- Ziel: R2 für Remote-Dateien auf Windows/Linux. Kriterien: K1 Nachbarn des Remote-Ordners
  erreichbar; K2 kein unbegrenztes Herunterladen ohne Nutzerhandlung (Bandbreite, Drive-Kontingent);
  K3 Save-back/Konflikt/Wiederherstellung bleiben; K4 Standard-App bleibt; K5 Windows und Linux.
- Möglichkeiten: A1 ganzen Ordner vorab laden (K2, K3 verletzt); A2 begrenztes Fenster ±N
  (K1 nur teilweise, willkürliche Grenze; Fotos liest beim Start); A3 über das Dokany-Laufwerk
  öffnen (nur Windows, nur bei vorhandener Einbindung, ändert den Speicherweg); A4 eigener
  Desktop-Betrachter mit Nachladen (K4 verletzt, Video/Audio in egui nicht möglich).
- Status: **Nutzerentscheid nötig** (Bandbreite gegen Bedienweg); kein Ansatz erfüllt alle
  Kriterien ohne Produktentscheidung. Offen in `docs/TODO.md` (`MEDIANAV-REMOTE`). Rückkehrpunkt:
  Antwort des Nutzers.

## D — Arbeitsplan

| M | Inhalt | Dateien | Erwartetes Ergebnis (Suite) |
|---|---|---|---|
| M1 | Portable Medienklassifikation | `types/core/media_kind.rs`, `types/mod.rs`, `mobile/core/entry.rs` | `kind_of` liefert für jede bisher gelistete Endung dasselbe; `media_kind` erkennt Groß-/Kleinschreibung, Ordner/ohne Endung = keine. |
| M2 | Startentscheidung Windows (portabel) | `app/core/media_launch_plan.rs`, `app/mod.rs` | Fotos ≥ 2024/unbekannt → `ms-photos`-URI; Fotos < 2024 und andere Store-Apps → Nachbarabfrage; klassisch/Nicht-Medium → unverändert; Kodierung von Leerzeichen, Umlauten, `\`, `:`, `#`, `%`, `&`, `+`; Versionsparser für Paketnamen. |
| M3 | Windows-Start mit Nachbarn | `app/os/windows/media_launch.rs`, `app/os/windows.rs`, `app/os/windows/platform.rs`, `native/Cargo.toml` (Features) | Windows-Ziel kompiliert; `open_local_path(Default)` nutzt die Entscheidung, Rückfall `ShellExecuteW`; `With`/Bearbeitung unverändert. |
| M4 | Android-Betrachter | `ui/viewer/*.kt`, `ui/files/FileActions.kt`, `FilesModels.kt`, `FilesScreen.kt`, `FilesViewModel.kt`, `prefs/AppPrefs.kt`, `ui/settings/SettingsScreen.kt` | APK baut; JVM-Test: Gruppenbildung/Startindex/Reihenfolge; Einstellung Standard an. |
| M5 | Doku | `docs/ARCHITEKTUR.md`, `README.md` (Funktionsliste, falls vorhanden), `docs/TODO.md`, dieser Plan | Einträge vorhanden, keine veralteten Aussagen. |
| M6 | Suite | `native/test-media-navigation-task.py`, `.github/workflows/media-navigation-task.yml` | Ein Lauf grün auf Linux, Windows, Android. |
| M7 | Release | `build.yml` `complete-release` | GitHub-Release mit allen Artefakten. |

Fortschritt: siehe Abschnitt E (wird je Meilenstein ergänzt).

## E — Fortschritt
- 2026-10-09: A–D festgehalten; Refs gesichert.
- M1–M3 umgesetzt (`3b0fc624`): `types/core/media_kind.rs`, `app/core/media_launch_plan.rs` mit
  Tests `media_navigation_task_*`, `app/os/windows/media_launch.rs`, `open_local_path(Default)`;
  Cargo-Features `windows` (Foundation, Foundation_Collections, Storage, Storage_Search, System)
  und `windows-sys` (Win32_Storage_Packaging_Appx). rustfmt-Prüfung der Batchdateien sauber.
- M4/M5 umgesetzt (`cf41c335`): `android/…/ui/viewer/` (MediaSet, MediaLoader, ImagePage,
  PlayerPage, MediaViewer), Einstellung `media_viewer`, JVM-Test `MediaSetTest`; README,
  ARCHITEKTUR, TODO (`MEDIANAV`, `MEDIANAV-REMOTE`). Graph aktualisiert (`95d34de9`).
- M6: Suite `native/test-media-navigation-task.py`, Workflow `media-navigation-task.yml`
  (Linux, Windows, Android-Build mit JVM-Tests).
- Lauf 37927265898 (`a723c686`): Android grün (48/48 JVM-Tests inkl. `MediaSetTest`); Linux:
  alle sechs `media_navigation_task_*` grün, aber die acht `app::remote_open`-Tests waren
  `#[ignore]` und liefen ohne `--include-ignored` nicht; Windows: E0277, `StorageFile` →
  `IStorageFile` braucht in windows 0.58 das Feature `Storage_Streams`. Behoben in `68bc9e7a`
  (expliziter `cast::<IStorageFile>()`, Suite mit `--include-ignored` im isolierten Profil).
- Lauf 37928311753 (`68bc9e7a`): Android grün; Windows kompiliert jetzt den WinRT-Adapter, Abbruch bei der Auswahl (`mobile` existiert nur unter Unix/Android); Linux: die acht `remote_open`-Tests verlangen `SMART_EXPLORER_COPY_PASTE_TASK=1` (isolierter Konstruktor). Behoben in `c6dbddec` und dem folgenden Suite-Commit.
- Lauf 37930602918 (`469f2ad3`): Linux, Windows und Android grün.
- Release v0.5.174: Complete-Release `build.yml` 37931532338 (Quelle `469f2ad3`, Release-Commit
  `ed93b619`, Tag `v0.5.174`), Publikation 37945856734 erfolgreich. GitHub-Release mit Installer,
  Windows/Linux app/updater/`se` samt `.sha256`, Android-APK samt `.sha256`, `install-linux.sh`,
  Kontextmenü-DLL, Share-Server (Windows/Linux) und `version.txt`; die `.sha256`-Dateien und
  `version.txt` stimmen mit `release-native/update-feed` am Tag überein (geprüft 2026-10-09).
  Nicht am echten Gerät beobachtet. Offen bleibt `MEDIANAV-REMOTE` (Nutzerentscheid, C4).
