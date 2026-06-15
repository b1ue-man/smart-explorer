# Smart Explorer

Schneller nativer Datei-Explorer für Windows (Rust + egui). Filtert Dateien/Ordner
über die gesamte Ordnertiefe (Name/Regex/Glob, Größe, Datum via Kalender, Typ),
kopiert gefiltert mit Strukturerhalt (auch über die Windows-Zwischenablage in den
Explorer), Fuzzy-Ordnersuche mit Live-Index, Tabs, Shell-Kontextmenü.

## Struktur

| Pfad | Inhalt |
|---|---|
| `native/` | Rust-Quellcode (das aktuelle Programm) |
| `native/installer.nsi` | NSIS-Installer-Skript |
| `native/publish-update.ps1` | Neue Version bauen + in Update-Feed veröffentlichen |
| `release-native/Smart Explorer Setup X.Y.Z.exe` | Installer (per-User, kein Admin) |
| `release-native/Smart Explorer.exe` | Portable EXE |
| `release-native/update-feed/` | Update-Feed: `version.txt` + `Smart Explorer.exe` |
| `archive/electron-v1-quellcode.zip` | Quellcode der alten Electron-Version (v1) |

## Bauen

```bash
# Git-Bash / PATH: cargo + Strawberry-GCC
export PATH="$USERPROFILE/.cargo/bin:/c/Strawberry/c/bin:$PATH"
cd native && cargo build --release
```

## Release veröffentlichen

1. `version` in `native/Cargo.toml` erhöhen
2. `cd native; .\publish-update.ps1` (PowerShell)

Installierte Instanzen prüfen den Feed bei jedem Start und updaten sich
automatisch (EXE-Tausch + Neustart). Feed-Pfad ist in der App änderbar
(Sidebar → UPDATE) oder in `%APPDATA%\smart_explorer\update_source.txt`.

## Daten der App

- Einstellungen/Index: `%APPDATA%\smart_explorer\` (folder_index.txt, recent.txt, update_source.txt)
- Crash-Log: `%APPDATA%\smart_explorer\crash.log`
- Installation: `%LOCALAPPDATA%\Programs\Smart Explorer\`
