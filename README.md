# Smart Explorer

Schneller nativer Datei-Explorer für Windows (Rust + egui). Filtert Dateien/Ordner
über die gesamte Ordnertiefe (Name/Regex/Glob, Größe, Datum via Kalender, Typ),
kopiert gefiltert mit Strukturerhalt (auch über die Windows-Zwischenablage in den
Explorer), Fuzzy-Ordnersuche mit Live-Index, Tabs + Split-Screen, Shell-Kontextmenü.

## 📋 Für neue Entwickler — zuerst lesen

| Doc | Inhalt |
|---|---|
| [`docs/ROADMAP.md`](docs/ROADMAP.md) | Status, To-dos, was als Nächstes kommt |
| [`docs/REMOTE_LAYER_PLAN.md`](docs/REMOTE_LAYER_PLAN.md) | Vollständiger, verifizierter Implementierungsplan für den Netzwerk-Layer (Vfs-Interface + SFTP/FTP/Netzlaufwerke) |
| [`docs/GOTCHAS.md`](docs/GOTCHAS.md) | Verifizierte Sackgassen & Fallen — **vor dem „Verbessern" lesen** |

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
2. Feed bauen + aktualisieren:
   - Windows: `cd native; .\publish-update.ps1` (baut zusätzlich den Installer)
   - Linux/macOS/WSL (Cross-Compile): `native/publish-feed.sh`
3. `release-native/update-feed/` (`version.txt` + `smart_explorer.exe`) committen
   und pushen — damit ist die neue Version über den Git-Feed live.

Installierte Instanzen prüfen den Feed bei jedem Start und updaten sich
automatisch (EXE-Tausch + Neustart).

**Update-Quelle (Feed)** — einstellbar in der App (Sidebar → UPDATE) oder in
`%APPDATA%\smart_explorer\update_source.txt`. Erlaubt ist entweder:

- ein **Ordner** (lokal oder `\\server\share`-Netzlaufwerk), oder
- eine **https-URL** bzw. ein **GitHub-Repo-Link** (z. B.
  `https://github.com/b1ue-man/smart-explorer`) — dann lädt die App
  `version.txt` + `smart_explorer.exe` direkt aus `release-native/update-feed/`
  über `raw.githubusercontent.com`. So lässt sich **das Git als Update-Quelle**
  setzen; jeder Push veröffentlicht ein Update.

## Daten der App

- Einstellungen/Index: `%APPDATA%\smart_explorer\` (folder_index.txt, recent.txt, update_source.txt)
- Crash-Log: `%APPDATA%\smart_explorer\crash.log`
- Installation: `%LOCALAPPDATA%\Programs\Smart Explorer\`
