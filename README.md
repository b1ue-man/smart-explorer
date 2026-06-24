# Smart Explorer

Schneller nativer Datei-Explorer für Windows und Linux (Rust + egui). Filtert Dateien/Ordner
über die gesamte Ordnertiefe (Name/Regex/Glob, Größe, Datum via Kalender, Typ),
kopiert gefiltert mit Strukturerhalt (auch über die Windows-Zwischenablage in den
Explorer), Fuzzy-Ordnersuche mit Live-Index, Tabs + Split-Screen, Shell-Kontextmenü.

**Remote/Cloud (ab 0.4.x):** durchsucht **SFTP**, **FTP/FTPS**, **WebDAV**
(Nextcloud/ownCloud) und authentifizierte **Netzlaufwerke (UNC)** über eine
einheitliche `Backend`-Schnittstelle (Sidebar → **VERBINDEN**); Zugangsdaten im
Windows Credential Manager. **Einseitige Spiegelung** ("⇅ Spiegeln nach…") sichert
den aktuellen (lokalen oder Remote-)Ordner in einen lokalen Zielordner.

**Google Drive (ab 0.5.16):** durchsuchen und **synchronisieren** über denselben
`Backend`-Mechanismus. Smart Explorer ist **kein Cloud-Dienst** — du hinterlegst
einmalig eine eigene **Google OAuth Client-ID** (Anleitung:
[`docs/CLOUD_SETUP.md`](docs/CLOUD_SETUP.md)); Einstellungen → **CLOUD (GOOGLE DRIVE)**.

**Teilen / P2P (ab 0.5.23):** Dateien direkt an gekoppelte Geräte oder in **Räume**
senden — **Ende-zu-Ende-verschlüsselt, direkt zwischen den Geräten**. Der
mitgelieferte **`se-share-server`** (Linux + Windows, in
[`release-native/share-server/`](release-native/share-server)) vermittelt nur die
Verbindung (Discovery), nie die Dateien. Toolbar → **📡 Teilen**; Server in
Einstellungen → **TEILEN**. Plan: [`docs/SHARE_PLAN.md`](docs/SHARE_PLAN.md).

---

## ⚠️ Lizenz & Hinweis

**Dieses Programm wurde vollständig mit [Claude](https://www.anthropic.com)
(einer KI von Anthropic) entwickelt.** Mit der Installation/Nutzung bestätigen
Sie, dass Ihnen dies bewusst ist.

Lizenz: **[MIT](LICENSE)** — frei nutzbar, und wie für freie Software üblich
**„WIE BESEHEN" ("AS IS"), ohne Gewährleistung; eine Haftung ist im gesetzlich
zulässigen Umfang ausgeschlossen.**

**Nutzung auf eigene Gefahr — erstellen Sie Sicherungskopien.** Kurzhinweis:
[`DISCLAIMER.txt`](DISCLAIMER.txt) (wird im Installer und beim ersten Start
angezeigt). Keine Rechtsberatung.

---

## ⬇️ Installieren

**Linux desktop (one line):**

```bash
curl -fsSL https://raw.githubusercontent.com/b1ue-man/smart-explorer/main/install-linux.sh | sh
```

**Windows:** Kein Admin, kein Setup-Zwang. Zwei Wege:

1. **Installer (empfohlen):** [`Smart Explorer Setup 0.5.90.exe`](release-native/Smart%20Explorer%20Setup%200.5.90.exe)
   (oder unter **[Releases](../../releases/latest)**) herunterladen und ausführen.
   Installiert nach `%LOCALAPPDATA%\Programs\Smart Explorer`, legt Startmenü-/
   Desktop-Verknüpfung an, registriert das Rechtsklick-Menü „In Smart Explorer
   öffnen" — **und stellt Auto-Update auf den Git-Feed ein. Danach musst du nichts
   konfigurieren, Updates kommen automatisch.**
2. **Portable:** [`Smart Explorer.exe`](release-native/Smart%20Explorer.exe)
   herunterladen und direkt starten (keine Installation). Für Auto-Update einmalig
   die Update-Quelle setzen (siehe unten).

## 🔄 Updates bekommen — *das hier eintragen*

Die App prüft bei **jedem Start** automatisch auf eine neuere Version und
aktualisiert sich selbst (EXE-Tausch + Neustart). Damit das geht, muss **eine
Update-Quelle** gesetzt sein. Der **Installer macht das schon** — bei der
portablen EXE trägst du sie einmal selbst ein:

> **App → linke Sidebar → Abschnitt `UPDATE` → in das Textfeld genau das eintragen:**
>
> ```
> https://github.com/b1ue-man/smart-explorer
> ```
>
> **→ „Speichern" klicken. Fertig.** Beim nächsten Start (oder „Jetzt prüfen")
> zieht die App die neueste Version aus dem Git.

Das ist alles. (Technisch lädt die App `version.txt` + den OS-passenden Payload
(`smart_explorer.exe` auf Windows, `smart_explorer` auf Linux) aus
[`release-native/update-feed/`](release-native/update-feed) über
`raw.githubusercontent.com`. Statt des Repo-Links kannst du auch direkt einen
Ordner-Pfad/UNC oder eine `https://…`-URL eintragen.) Die Quelle steht auch in
`%APPDATA%\smart_explorer\update_source.txt` bzw.
`$XDG_DATA_HOME/smart_explorer/update_source.txt`.

## 📋 Für neue Entwickler — zuerst lesen

| Doc | Inhalt |
|---|---|
| [`docs/ROADMAP.md`](docs/ROADMAP.md) | Status — **Roadmap vollständig** (Remote-Layer, Cloud/WebDAV, Sync, Win11-Menü) |
| [`docs/REMOTE_LAYER_PLAN.md`](docs/REMOTE_LAYER_PLAN.md) | Verifizierter Implementierungsplan für den Netzwerk-Layer (umgesetzt: `vfs.rs` + `sftp.rs`/`ftp.rs`/`webdav.rs`/`net.rs`/`rscan.rs`/`connect.rs`/`creds.rs`/`sync.rs`) |
| [`docs/RELEASING.md`](docs/RELEASING.md) | **Release- & Update-Flow von A bis Z** (bauen → Feed → GitHub-Release → Selbst-Update); inkl. „Repo muss public sein" |
| [`docs/CLOUD_SETUP.md`](docs/CLOUD_SETUP.md) | **Google Drive einrichten** mit deinem eigenen Google-Projekt (OAuth Client-ID) — die App ist kein Dienst |
| [`docs/WIN11_CONTEXT_MENU.md`](docs/WIN11_CONTEXT_MENU.md) | Win11-Modern-Kontextmenü: COM-DLL (`explorer-command/`) gebaut; offen ist nur die Signierung |
| [`docs/GOTCHAS.md`](docs/GOTCHAS.md) | Verifizierte Sackgassen & Fallen — **vor dem „Verbessern" lesen** |

## Struktur

| Pfad | Inhalt |
|---|---|
| `native/` | Rust-Quellcode (das aktuelle Programm) |
| `native/explorer-command/` | Separate COM-DLL (`IExplorerCommand`) für das Win11-Modern-Kontextmenü |
| `native/installer.nsi` | NSIS-Installer-Skript |
| `native/publish-update.ps1` | Neue Version bauen + in Update-Feed veröffentlichen |
| `native/publish-feed.sh` | Cross-Compile-Build + Git-Feed aktualisieren (Linux/macOS/WSL) |
| `release-native/Smart Explorer Setup X.Y.Z.exe` | Installer (per-User, kein Admin) |
| `release-native/Smart Explorer.exe` | Portable EXE |
| `release-native/update-feed/` | Update-Feed: `version.txt` + `Smart Explorer.exe` |
| `LICENSE` | MIT-Lizenz (frei, AS-IS, ohne Gewähr/Haftung) |
| `DISCLAIMER.txt` | Kurzhinweis (KI-Bau + Lizenzverweis), im Installer/ersten Start |
| `archive/electron-v1-quellcode.zip` | Quellcode der alten Electron-Version (v1) |

## Bauen

```bash
# Git-Bash / PATH: cargo + Strawberry-GCC
export PATH="$USERPROFILE/.cargo/bin:/c/Strawberry/c/bin:$PATH"
cd native && cargo build --release
```

## Release veröffentlichen

Der vollständige Flow (bauen → Feed → GitHub-Release → Selbst-Update) steht in
**[`docs/RELEASING.md`](docs/RELEASING.md)**. Kurz:

1. `version` in `native/Cargo.toml` erhöhen, committen.
2. Bauen + Artefakte stagen: `native/publish-feed.sh` (Linux/WSL/macOS) bzw.
   `cd native; .\publish-update.ps1` (Windows) — erzeugt die Windows-Artefakte.
   Für einen vollständigen Windows+Linux-Feed `native/publish-feed.sh` auf einem
   Linux/WSL-Host ausführen.
3. `release-native/` committen und **nach `main` mergen** (der Feed wird von
   `main` ausgeliefert — erst dann ist das Update live).
4. GitHub-Release veröffentlichen: Tag `vX.Y.Z` pushen (CI `build.yml` released
   auf `v*`). Hängt Windows- und Linux-Binaries, Installer, Script, DLL und
   `version.txt` an.

> **Wichtig:** Damit anonyme Clients aus dem Git updaten können, muss das Repo
> **public** sein (`raw.githubusercontent.com` braucht sonst Auth). Siehe
> RELEASING.md.

**Update-Quelle (Feed)** — einstellbar in der App (Sidebar → UPDATE) oder in
`%APPDATA%\smart_explorer\update_source.txt`. Erlaubt: ein **Ordner**
(lokal/`\\server\share`), eine **https-URL** oder ein **GitHub-Repo-Link**
(`https://github.com/b1ue-man/smart-explorer` → wird auf den `main`-Feed
übersetzt). Installierte Instanzen prüfen den Feed bei jedem Start und updaten
sich automatisch (EXE-Tausch + Neustart).

## Daten der App

- Windows-Daten: `%APPDATA%\smart_explorer\` (folder_index.txt, recent.txt, update_source.txt)
- Linux-Daten: `$XDG_DATA_HOME/smart_explorer/` bzw. `~/.local/share/smart_explorer/`
- Windows-Installation: `%LOCALAPPDATA%\Programs\Smart Explorer\`
- Linux-Installation: `~/.local/opt/smart-explorer/`
