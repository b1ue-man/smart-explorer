# Weiterschalten zwischen Medien beim Öffnen aus einem Dateimanager

Stand: 2026-10-09 (alle Quellen an diesem Tag abgerufen). Frage: Wie bekommen Betrachter auf
Windows, Linux und Android die Nachbardateien, damit Links/Rechts zum vorherigen/nächsten Medium
führt, wenn ein anderes Programm als der System-Dateimanager die Datei öffnet?

## Windows

### Woher die Fotos-App ihre Liste nimmt

- Microsoft-Blog „Using Neighbouring File Queries to Power your App“ (Adam Wilson,
  2015-07-28, archiviert auf learn.microsoft.com/archive/blogs/adamdwilson): Der Explorer legt
  einer Dateiaktivierung eine *Neighbouring Files Query* (NFQ) bei; die Fotos-App blättert damit
  durch die übrigen Dateien des Ordners. Quell-Apps setzen sie über
  `LauncherOptions.NeighboringFilesQuery` und `Launcher.LaunchFileAsync(file, options)`.
  Die Abfrage soll breit sein (`FolderDepth.Shallow`, z. B. `kind:picture`); die Ziel-App erhält
  nur Typen, für die sie eine Bibliotheksfähigkeit deklariert.
- `LauncherOptions.NeighboringFilesQuery` (learn.microsoft.com/uwp/api/windows.system.launcheroptions.neighboringfilesquery,
  Seitenstand 2025-11-21): Eigenschaft vom Typ `StorageFileQueryResult`; vorhanden ab
  Windows 10 10240.
- Directory-Opus-Forum (Leo, 2021-11-16 und 2023-08-13, resource.dopus.com/t/39839): Fotos
  liest die Liste aus dem Explorer-Fenster und baut sie nicht selbst; fast alle anderen Betrachter
  lesen den Ordner selbst.

### Wie andere Dateimanager das gelöst haben

Total Commander, `history.txt` (ghisler.com, Stand TC 11.58 vom 2026-07-01):

| Datum | Eintrag (gekürzt) |
|---|---|
| 05.09.16 | Statt der Fotos-App die alte Windows-Fotoanzeige öffnen; „It allows to switch through all images in a folder even when called from other programs than the Explorer“. |
| 21.11.23 | „Open Windows Photo app with current directory parameter so it can switch through all images in the same folder“ (nur Enter/Doppelklick). |
| 22.11.23 | `PhotoAppFilter` in AQS-Syntax, z. B. `System.FileExtension:=(".jpg" OR ".png")` – der Filter der Nachbarabfrage. |
| 19.01.24 | Hilfs-DLL `TCshareWin10.dll` öffnet mehrere Dateien mit der Fotos-App. |
| 25.06.24 | „Windows 11: Open the Photos app with new command (`ms-photos:viewer?fileName=Encoded_filename`) to enable previous/next buttons“. |
| 11.02.25 | `PhotoAppMode`: 0 = automatisch nach Versionsnummer der Fotos-App, 1 = alte Methode (Nachbarabfrage), 2 = `ms-photos:viewer` mit URL-kodiertem Namen, 3 = ohne Kodierung, -1 = nur eine Datei. |
| 24.03.25 | URL-kodierte Namen funktionieren wieder ab Fotos-Version 2025.11030.12002.0. |

- Directory Opus 13.18.8 Beta (2025-10-16): „the Microsoft Photos app when launched via
  double-click now lets you go to the next/previous images in the folder again“; Jon
  (2025-10-15): Microsoft ändert dieses Verhalten regelmäßig.
- xplorer2-Forum (2019-12): keine Lösung, nur Strg+A und Enter im Dateimanager.

**Folgerung:** Neue Fotos-App → `ms-photos:viewer?fileName=<URL-kodierter Pfad>`; alte
Fotos-App und andere Store-Apps → `LaunchFileAsync` mit Nachbarabfrage über den Ordner;
klassische Programme (IrfanView, Windows-Fotoanzeige, VLC, MPC-HC) lesen den Ordner selbst und
brauchen nur den echten Pfad. Grenze: Das `ms-photos`-Protokoll ist nicht offiziell dokumentiert;
die Reihenfolge bestimmt Fotos selbst.

## Linux

- GNOME Loupe (help.gnome.org/loupe/opening-images.html): „Loupe will load other files in the
  parent directory of the image and sort them in alphabetical order“; Vor/Zurück über Buttons und
  Tastenkürzel.
- Gwenview (KDE-Handbuch, lxr.kde.org/source/graphics/gwenview/doc/index.docbook): Vor/Zurück
  innerhalb des Ordners; Eye of GNOME verhält sich gleich (dieselbe Ordnerladung beim Öffnen einer
  einzelnen Datei).
- Nautilus und Dolphin übergeben nur die eine Datei; der Betrachter ermittelt die Nachbarn.

**Folgerung:** Unter Linux genügt die Übergabe des echten Pfads (Smart Explorer:
`xdg-open <pfad>`). Videoplayer laden den Ordner meist nicht automatisch (mpv nur mit
`autoload.lua`, VLC nicht standardmäßig) – das gilt genauso beim Öffnen aus Nautilus/Dolphin.

## Android

- `Intent.ACTION_VIEW` trägt genau eine Daten-URI; seit Android 7 sind es `content://`-URIs
  ohne Ordnerbezug. Galerien zeigen dann nur dieses eine Bild (Total-Commander-Forum,
  ghisler.ch/board/viewtopic.php?p=444590: „only one image gets sent to the gallery, and swiping
  doesn't work“; Wischen hängt von der Galerie ab).
- Material Files (github.com/zhanghai/MaterialFiles, `FileListFragment.kt`, master):
  `maybeAddImageViewerActivityExtras` hängt beim Öffnen eines Bildes alle Bilder der Liste in
  Anzeigereihenfolge und die Position an (`ImageViewerActivity.putExtras`), höchstens 1000 Pfade
  wegen `TransactionTooLargeException`; ausgewertet nur vom eigenen Betrachter.
- Files by Google, Solid Explorer und MiXplorer öffnen Medien in einem eigenen Betrachter mit
  Wischen und bieten „Öffnen mit“ für externe Apps (Beobachtung der Apps; keine Herstellerdoku).

**Folgerung:** Auf Android ist ein eigener Betrachter mit Wischen über die Medien der aktuellen
Liste der etablierte Weg; externe Apps bleiben über „Öffnen mit“ erreichbar.

## Grenzen

- Ob neuere Fotos-Versionen die Nachbarabfrage wieder auswerten, ist nicht dokumentiert.
- Verhalten der neuen Windows-Media-Player-App mit Nachbarabfrage: keine Quelle gefunden.
