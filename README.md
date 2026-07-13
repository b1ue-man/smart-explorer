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
Einseitige Drive-Mirror-Jobs nutzen nach dem ersten Vollabgleich einen lokalen
Sync-Index plus Google-Drive-Changes, damit normale Läufe nur geänderte Pfade
prüfen; unsichere Zustände fallen automatisch auf den bisherigen Vollabgleich
zurück.

**Idle-Verbindungen (ab 0.5.131):** SFTP und der SSH-Agent halten ihre
authentifizierten Sitzungen aktiv; FTP/FTPS prueft unbenutzte Kontrollkanaele
mit `NOOP`; Share ueberwacht Signaling und Iroh/QUIC. WebDAV und Drive ersetzen
stale gepoolte HTTPS-Verbindungen vor sicheren Leseoperationen, waehrend
UNC/SMB-Leases bis zum letzten Benutzer erhalten bleiben. Verbindungsaufbau und
echte Inaktivitaet sind begrenzt, aber eine laufende Datenuebertragung wird
nicht wegen ihrer Gesamtdauer geschlossen. Fehlgeschlagene Schreib-, Loesch-
oder Umbenennungsoperationen werden dabei niemals blind wiederholt.

**Teilen / P2P (ab 0.5.23):** Dateien an gekoppelte Geräte oder in **Räume**
senden. Der aktuelle Iroh/QUIC-Transport ist **Ende-zu-Ende-verschlüsselt** und
versucht zuerst einen direkten Gerätepfad. Falls der nicht erreichbar ist, kann
der mitgelieferte **`se-share-server`** (Linux + Windows, in
[`release-native/share-server/`](release-native/share-server)) verschlüsselte
Transportpakete weiterleiten; er erhält keine Relation-Secrets oder
Dateisystemdaten im Klartext. Toolbar → **📡 Teilen**; Server in Einstellungen →
**TEILEN**. Der frühere Noise/TCP-Entwurf steht historisch in
[`docs/SHARE_PLAN.md`](docs/SHARE_PLAN.md).

**Terminal (ab 0.5.118):** der mitinstallierte Companion **`se`** arbeitet auch
ohne GUI-Sitzung und nutzt dieselben gespeicherten Verbindungen, App-Daten,
Zugangsdaten, Share-Profile und den Daemon wie die GUI. Unter Windows liegen
Secrets im Credential Manager. Unter Linux nutzt Smart Explorer bewusst einen
DBus-freien, benutzereigenen Dateispeicher
(`$XDG_DATA_HOME/smart_explorer/secrets-v1`, Verzeichnis `0700`, Dateien `0600`),
damit SSH, CI und Dienste funktionieren. Dieser Linux-Speicher schützt durch
Unix-Rechte vor anderen normalen Benutzern, verschlüsselt aber nicht gegen
`root`, denselben Benutzer oder Offline-Zugriff auf einen unverschlüsselten
Datenträger. Beispiele: `se doctor --json`, `se connections list`,
`se ls @label:/pfad`,
`se get @sftp:/bericht.pdf .`, `se put ./lokal.txt @webdav:/ziel/`,
`se cp @drive:/a.txt @share:/b.txt`, `se search @label:/ "*.rs"`. Remote
Execution ist ab 0.5.133 als separate, standardmaessig deaktivierte
Geraeteberechtigung verfuegbar. `se share grants exec` zeigt die exakten
Identitaeten, `enable --yes` erlaubt einem Geraet die vollstaendige Shell-Autoritaet
des Smart-Explorer-Benutzers und `disable` entzieht sie wieder und beendet dessen
aktive Prozessbaeume. `se exec -- PROGRAM ARGS...` waehlt den einzigen bereiten
Peer automatisch; alternativ funktionieren sichtbarer Name/Endpunkt und
`--shell COMMAND`. stdin/stdout/stderr werden binaer gestreamt, Exitcodes bleiben
erhalten, `se share exec` zeigt aktive/letzte Jobs und kann sie abbrechen. Windows
kapselt jeden Job in einem Kill-on-close Job Object, Linux in einer transienten
systemd-cgroup; ohne diesen Plattformprovider startet kein Payload. Es gibt
bewusst keine Command-, Pfad- oder Shell-Filter: die Freigabe ist volle
Remote-Codeausfuehrung als der laufende Benutzer.
Setup geht ebenfalls einseitig aus dem Terminal: `se connections add sftp --host
example.com --user alice --root /srv --label prod --password-stdin`,
`se connections add share --root \\server\share --label NAS --password-stdin`
speichert eine UNC-Verbindung, `se connections add-peer --code SE-D3-... --name
Laptop` speichert eine getrackte Freigabeanfrage mit stabiler Request-ID und
reiht sie ueber den Share-Worker ein, und `se connections add-room --code
SE-R3-... --name Team` tritt einem Raum bei. `queued` bedeutet dabei nur lokal
dauerhaft vorgemerkt; auch eine Relay-Meldung `forwarded` bestaetigt noch keinen
Peer-Empfang. Erst der signierte Request-Receipt des Zielgeraets meldet
`received`. Entscheidung (`pending`/`accepted`/`rejected`/`revoked`), aktive
Autorisierung und aktuelle Verbindung werden getrennt angezeigt.

`se share request` zeigt ohne weitere Argumente die offene Inbox samt
Request-ID, Geraet, Fingerprint, Empfangs-, Entscheidungs- und
Autorisierungsstatus. Ist genau eine Anfrage offen, akzeptiert oder verwirft
`se share request accept` beziehungsweise `reject` sie ohne versteckte IDs oder
erneute Fingerprint-Eingabe; bei mehreren Anfragen nennt die Inbox direkt die
gueltigen Befehle. `list`, `show`, `retry` und `delete` verwalten den
vollstaendigen Verlauf. `delete` entfernt auch eine offene eingehende oder
ausgehende Anfrage lokal, stoppt ihre Retries und behaelt einen kleinen
dauerhaften Replay-Tombstone; ein spaetes Accept einer geloeschten ausgehenden
Anfrage erzeugt dadurch keine Autoritaet. Das Loeschen abgeschlossener Historie
widerruft keinen unabhaengigen Grant. Eine angenommene eingehende Anfrage bleibt
solange sichtbar, wie sie den aktiven Grant oder dessen noch unbestaetigten
signierten Widerruf traegt; danach kann auch sie geloescht werden. `show`,
`retry` und `delete` waehlen wie die Entscheidungsbefehle den einzigen passenden
Eintrag automatisch. `se share grants` zeigt ohne Unterbefehl
aktive und inaktive Autorisierungen; `se share grants revoke` waehlt die einzige
aktive Freigabe automatisch. In der GUI trennt **Teilen** offene Anfragen vom
eingeklappten Verlauf. Offene Anfragen und sicher abgeschlossene Historie koennen
dort ebenfalls lokal geloescht werden; **Ablehnen** und **Widerrufen** bleiben
die expliziten signierten Peer-Entscheidungen.

`se connections` listet ohne Unterbefehl alle Ziele. Jede Share-Zeile nennt
explizit `selector=<id>`; `remove-peer` akzeptiert genau diese ID, den ebenfalls
ausgegebenen `share://direct/...`-Endpunkt, Name, Geraete-ID oder Fingerprint
und waehlt bei nur einem Peer auch ohne Argument eindeutig aus. Dynamische
Vervollstaendigung fuer Befehle und aktuelle Request-/Grant-/Peer-Selektoren
liefert `se completions bash|zsh|fish|elvish|powershell`, zum Beispiel
`source <(se completions bash)` oder in PowerShell
`se completions powershell | Out-String | Invoke-Expression`.

Andere Verbindungen und Raeume entfernt `se connections remove` beziehungsweise
`remove-room`.
Headless Share laesst sich ueber `se share configure`, `identity`, `status`,
`request`, `grants`, `export`, `room` und `worker` vollstaendig verwalten;
`status` unterscheidet dabei einen laufenden Worker von einer tatsaechlich
verbundenen Signaling-Sitzung. Das vollstaendige Protokoll und die
Statusbedeutungen stehen in [`docs/SHARE_SERVER.md`](docs/SHARE_SERVER.md).
Hat eine alte Linux-Version Identitätsmetadaten ohne dauerhaft gespeicherte
Secrets hinterlassen, nennt der CLI den gezielten Reparaturbefehl
`se share identity --repair`. Er läuft nur bei tatsächlich fehlendem
Secret-Material; eine ersetzte Identität macht alte Einladungen und
Vertrauensbindungen ungültig, sodass die Peers neu gekoppelt werden müssen.

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

**Linux nur Terminal (ohne Desktop-/X11-Abhängigkeiten):**

```bash
curl -fsSL https://raw.githubusercontent.com/b1ue-man/smart-explorer/main/install-linux.sh | sh -s -- --cli-only
```

**Windows:** Kein Admin, kein Setup-Zwang. Zwei Wege:

1. **Installer (empfohlen):** [`Smart Explorer Setup 0.5.133.exe`](release-native/Smart%20Explorer%20Setup%200.5.133.exe)
   (oder unter **[Releases](../../releases/latest)**) herunterladen und ausführen.
   Installiert nach `%LOCALAPPDATA%\Programs\Smart Explorer`, legt Startmenü-/
   Desktop-Verknüpfung an, registriert das Rechtsklick-Menü „In Smart Explorer
   öffnen", installiert `se.exe` fuer Terminal-Operationen und trägt dessen
   Ordner in den Benutzer-`PATH` ein (sichtbar in neu geöffneten Terminals) — **und stellt die
   Update-Prüfung auf den Git-Feed ein. Neue Versionen werden automatisch geprüft
   und sicher bereitgestellt; installiert werden sie erst nach deiner Bestätigung.**
2. **Portable:** [`Smart Explorer.exe`](release-native/Smart%20Explorer.exe)
   herunterladen und direkt starten (keine Installation). Für Auto-Update einmalig
   die Update-Quelle setzen (siehe unten). Das portable Terminal-Binary liegt als
   [`se.exe`](release-native/se.exe) daneben.

## 🔄 Updates bekommen — *das hier eintragen*

Die App prüft bei **jedem Start** automatisch auf eine neuere Version. Sie lädt
und prüft ein gefundenes Update, ändert aber noch keine installierte Datei. Erst
„Jetzt neu starten" startet die Installation; „Später" bewahrt das geprüfte
Staging und fragt beim nächsten Start wieder. Dafür muss **eine Update-Quelle**
gesetzt sein. Der **Installer macht das schon** — bei der portablen EXE trägst
du sie einmal selbst ein:

> **App → linke Sidebar → Abschnitt `UPDATE` → in das Textfeld genau das eintragen:**
>
> ```
> https://github.com/b1ue-man/smart-explorer
> ```
>
> **→ „Speichern" klicken. Fertig.** Beim nächsten Start (oder „Jetzt prüfen")
> stellt die App die neueste Version aus dem Git geprüft bereit und fragt vor
> Installation/Neustart nach.

Das ist alles. (Technisch lädt die App `version.txt` plus das OS-passende Trio
aus App, Update-Helfer und `se` samt SHA-256-Dateien aus
[`release-native/update-feed/`](release-native/update-feed) über
`raw.githubusercontent.com`. Statt des Repo-Links kannst du auch direkt einen
Ordner-Pfad/UNC oder eine `https://…`-URL eintragen.) Die Quelle steht auch in
`%APPDATA%\smart_explorer\update_source.txt` bzw.
`$XDG_DATA_HOME/smart_explorer/update_source.txt`.

## 📋 Für neue Entwickler — zuerst lesen

| Doc | Inhalt |
|---|---|
| [`docs/TODO.md`](docs/TODO.md) | Einzige aktuelle Liste für offene bzw. noch real zu validierende Arbeit |
| [`docs/ROADMAP.md`](docs/ROADMAP.md) | Historische Roadmap-/Status-Erzählung; nicht die aktuelle Release- oder TODO-Quelle |
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
| `native/publish-release-local.ps1` | Standard-Release auf Windows: Windows + Linux isoliert bauen, vollständig prüfen, Feed atomar promoten |
| `native/publish-feed.sh` | Vollständiger Release auf Linux/WSL: Windows + Linux, Installer, DLL und Share-Server bauen/pruefen, Feed atomar promoten |
| `native/publish-update.ps1` | Windows-only-Bundle; verlangt `-AllowPartialFeed` sowie getrennte, explizite `-Feed`- und `-ReleaseOutput`-Pfade |
| `native/publish-linux-feed-wsl.sh` | Linux-App/Updater in WSL bauen; Versions-Commit nur mit passendem Windows-Build-Manifest |
| `release-native/Smart Explorer Setup X.Y.Z.exe` | Installer (per-User, kein Admin) |
| `release-native/Smart Explorer.exe` | Portable EXE |
| `release-native/se.exe` | Portable Terminal-Companion |
| `release-native/update-feed/` | Update-Feed: `version.txt` + `smart_explorer.exe` / `smart_explorer` + `se.exe` / `se` |
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
2. Bauen + Artefakte stagen: auf Windows standardmäßig
   `.\native\publish-release-local.ps1`. Der Wrapper baut App/Updater/`se`/Installer,
   baut die Linux/WSL-Payloads im selben isolierten Staging-Baum, prüft alle
   Artefakte und schreibt `version.txt` erst nach der rollback-geschützten
   Gesamt-Promotion. `-SkipLinuxFeed` erzeugt nur ein ausdrücklich nicht
   publizierbares Windows-Prüfbundle; der gemeinsame Feed bleibt unverändert.
3. `release-native/` committen und **nach `main` mergen** (der Feed wird von
   `main` ausgeliefert — erst dann ist das Update live).
4. GitHub-Release veröffentlichen: Tag `vX.Y.Z` pushen (CI `build.yml` released
   auf `v*`). Hängt die verifizierten Windows-/Linux-App-, Updater- und
   `se`-Payloads samt Hashes, Installer, Linux-Installskript, Kontextmenü-DLL,
   beide Share-Server und `version.txt` an.

> **Wichtig:** Damit anonyme Clients aus dem Git updaten können, muss das Repo
> **public** sein (`raw.githubusercontent.com` braucht sonst Auth). Siehe
> RELEASING.md.

**Update-Quelle (Feed)** — einstellbar in der App (Sidebar → UPDATE) oder in
`%APPDATA%\smart_explorer\update_source.txt`. Erlaubt: ein **Ordner**
(lokal/`\\server\share`), eine **https-URL** oder ein **GitHub-Repo-Link**
(`https://github.com/b1ue-man/smart-explorer` → wird auf den `main`-Feed
übersetzt). Installierte Instanzen prüfen den Feed bei jedem Start und stagen
ein neues, hash-verifiziertes App/Updater/`se`-Bundle. Die transaktionale
Installation und der Neustart erfolgen erst nach ausdrücklicher Bestätigung.

## Daten der App

- Windows-Daten: `%APPDATA%\smart_explorer\` (folder_index.txt, recent.txt, update_source.txt, sync/sync_state.sqlite)
- Linux-Daten: `$XDG_DATA_HOME/smart_explorer/` bzw. `~/.local/share/smart_explorer/`
- Windows-Installation: `%LOCALAPPDATA%\Programs\Smart Explorer\`
- Linux-Installation: `~/.local/opt/smart-explorer/`
