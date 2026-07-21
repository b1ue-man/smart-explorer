# Smart Explorer — Native (Rust + egui)

Schlanke, schnelle native Variante. Die GUI selbst ist ein einzelnes natives
Binary — kein Chromium, Browser oder Node. Sie ist weiterhin portabel startbar;
die reguläre per-user Installation ergänzt den verifizierten Update-Helfer und
den `se`-Terminal-Companion. Updates werden geprüft gestagt und erst nach
expliziter Bestätigung transaktional angewendet.

## Größe & Geschwindigkeit (im Vergleich zur Electron-Variante)

| Metrik | Electron-Version | Native-Version |
|---|---|---|
| Distribution | 79 MB Installer | **~7.7 MB Installer / ~22 MB EXE** |
| Entpackt | ~280 MB | **Native EXE + Updater-Helfer + `se` Companion** |
| Prozesse beim Start | 4 (main+renderer+gpu+util) | **1** |
| Scan `node_modules` (~12k Dateien) | 8.6k/s | **76k/s** |
| Scan `Program Files` (~514k Dateien, 89 GB) | 61.7s | **1.85s (33×)** |

Erreicht durch:
- `std::fs::read_dir` auf Windows nutzt `FindFirstFileW` und liefert alle Metadaten
  in einem Syscall pro Eintrag (statt readdir + stat)
- Rayon-basierte parallele Verzeichnis-Walker (Work-Stealing über alle Cores)
- Channels (crossbeam) streamen Resultate batchweise (1024er-Pakete oder 60ms)
- ThinLTO + strip im Release-Build, panic = abort, codegen-units = 8

## Build

Voraussetzungen:
- Rust GNU-Toolchain (`rustup target add x86_64-pc-windows-gnu` mit `rustup install stable-gnu`)
- Strawberry Perl GCC oder MinGW-w64 als Linker (rustup-Bundle reicht)

```bash
cargo build --release
# → target/release/smart_explorer.exe und target/release/se.exe
```

Release-Artefakte werden nicht per Hand kopiert. Der aktuelle lokale
Release-Flow steht in [`../docs/RELEASING.md`](../docs/RELEASING.md); auf einem
Windows-Rechner ist `..\native\publish-release-local.ps1` der Standard, weil der
Wrapper Windows- und Linux-Feed-Payloads inklusive `se` gemeinsam in einem
isolierten Baum baut, prüft und erst danach rollback-geschützt veröffentlicht.
Die vollständigen Windows- und Linux-Pfade teilen
`release-native/.complete-release.lock` und brechen bei einem zweiten Lauf vor
dem Build ab. Ein fehlgeschlagener vollständiger Lauf behält seinen Stage und
meldet dessen Pfad; dieser ist Diagnosematerial, kein automatisch resumierbarer
oder manuell zu promotender Release-Kandidat.

Bench-Mode:
```bash
cargo build --release --bin bench
target/release/bench.exe "C:/Program Files"
```

## Echte Remote-Laufwerke unter Windows

Der optionale Mount-Pfad projiziert genau eine ausgewählte, bereits
autorisierte Backend-Wurzel als Dokany-Laufwerksbuchstaben. Er verwendet weder
CfAPI noch Placeholder oder eine zweite Protokollimplementierung. Der Daemon
löst gespeicherte Zugangsdaten und den aktiven Backend-/Fallback-Pfad auf,
beschränkt ihn mit `RootedBackend` auf die ausgewählte Wurzel und reicht dem
isolierten, aus `std::env::current_exe()` gestarteten Prozess mit dem privaten
`--mount-host <id>`-Argument nur kurzlebige, getrennte
Loopback-Capabilities. Endpoint, Konto, Credentials, globale Daemon-Autorität
und Backend-IDs verlassen diese Grenze nicht. Pfadtraversal, Symlink-/Reparse-
Durchquerung, Windows-Gerätenamen, ADS-Syntax und case-kollidierende
Verzeichniseinträge werden fail-closed abgelehnt.

`RootedBackend` akzeptiert standardmäßig nur eine technisch erzwungene
Root-Capability. Der über SSH ausgerollte Linux-Agent startet dazu mit
`--serve-root` und bindet die vollständige, symlinkfreie Root-Auflösung per
`openat2` in eine Landlock-Domain (ABI **3+**, einschließlich Rename und
Truncate), bevor Worker-Threads entstehen. Google Drive ist durch seine
Parent-ID-Hierarchie confined. Plain SFTP, Local/UNC, Peer, WebDAV und FTP
können einen externen Pfadaustausch nicht technisch ausschließen und brauchen
deshalb die explizite GUI-Vertrauensoption oder `--trust-remote-root`. Das gilt
auch für ein read-only Volume: Trusted-Root serialisiert und validiert jeden
Smart-Explorer-Aufruf weiter, setzt aber einen vertrauenswürdigen Server und
keinen gleichzeitig bösartig ausgetauschten Symlink/Junction voraus.

Smart Explorer bindet Dokany nicht statisch. Der empfohlene NSIS-Installer
bettet die exakt gepinnte offizielle `Dokan_x64.msi` als standardmäßig
ausgewählte optionale Offline-Komponente ein; portable oder per Auto-Update
aktualisierte Installationen laden denselben Pin über die GUI oder
`se drive install-runtime`. Die Basis-App bleibt per-user, nur der systemweite
MSI-Schritt läuft nach einer UAC-Bestätigung. Ein stilles Setup installiert die
Runtime ausschließlich mit `/S /INSTALLDOKANY=1`, und der Smart-Explorer-
Uninstaller entfernt Dokany nicht.

Zur Laufzeit wird `dokan2.dll` mit
`LoadLibraryExW(..., LOAD_LIBRARY_SEARCH_SYSTEM32)` nur aus
`%WINDIR%\System32` geladen; sowohl `DokanVersion()` als auch
`DokanDriverVersion()` müssen exakt **231** melden. Unterstützt ist die
offizielle [Dokany-Version
2.3.1](https://github.com/dokan-dev/dokany/releases/tag/v2.3.1.1000). Deren
offizielle Builds enthalten signierte Treiber, weshalb weder Windows Developer
Mode noch `TESTSIGNING` nötig ist; die systemweite Runtime-Installation kann
Adminrechte verlangen. Primärquellen geprüft am 2026-07-21:
[Dokany README](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/README.md),
[2.3.1-Header mit `DOKAN_VERSION 231`](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/dokan/dokan.h).

Bedienung:

```powershell
se drive runtime
se drive install-runtime
se drive mount @prod:/srv --letter M
se drive mount @prod:/notizen --letter N --read-write --label "Notizen"
se drive mount sftp://host/srv --letter S --trust-remote-root
se drive list --json
se drive unmount M:
se drive retry <mount-id>
```

`--letter` akzeptiert `auto` (Standard) oder einen Buchstaben. Als Ziel gelten
`@label:/pfad`, ein exakt zu einer gespeicherten Verbindung passender Remote-
URL-/UNC-Pfad, `gdrive://...` oder ein `share://...`-Endpunkt. Ohne
`--read-write` ist das Volume read-only. Die GUI bietet dieselbe Auswahl über
das Laufwerkssymbol an gespeicherten Verbindungen, Google Drive und
Share-Geräten; der globale Laufwerksmanager kann Mounts auswerfen oder nach
Fehlern erneut verbinden. Nicht-Windows-Plattformen liefern für den Mount-Pfad
bewusst `Unsupported`.

Ein strikter SFTP-Mount wird nur aus einer gespeicherten Verbindung mit
aktiviertem und erfolgreich gestartetem Agent zugelassen. Deployment- oder
Handshake-Fehler werden für den Mount sichtbar zurückgegeben und dürfen nicht
still zu plain SFTP wechseln; normales Browsing darf nur beim Agent-Deployment
auf die bereits aufgebaute SFTP-Verbindung zurückfallen. Nach erfolgreichem
Handshake werden fehlgeschlagene Agent-Operationen nicht blind über SFTP
wiederholt. Der SSH-Verbindungsaufbau
staffelt IPv6-/IPv4-Versuche mit 250 ms Versatz, behält den ursprünglichen Hostnamen
für Known-Hosts und liefert typisierte Connect-/Handshake-/Exec-Timeouts.
Remote-Ausgabe wird begrenzt. Das Agent-Artefakt wird nach seinem vollständigen
SHA-256 benannt, vor dem Start und erneut nach einem Reconnect über SFTP gegen
die eingebetteten Bytes verifiziert.

### Schreib- und Recovery-Modell

Ein Backend wird nur dann read-write gemountet, wenn es für die exakte Wurzel
alle statischen `StagedWriteCapabilities` garantiert:

| Garantie | Bedeutung |
|---|---|
| `create` | der zufällige Staging-Name wird atomar exklusiv geöffnet und kann als noch fehlendes Ziel veröffentlicht werden; kein Stat/Create-Rennen |
| `replace` | eine vorhandene Datei kann durch eine vollständige Staging-Datei ersetzt werden |
| `namespace_replace` | `Temp-Datei → vorhandener Name` ist ein atomarer old-or-new Namespace-Replace |

Diese Schreibgarantien sind von der Root-Isolation getrennt. Local/UNC und der
Smart-Explorer-SSH-Agent melden alle drei Schreibprimitive. Plain SFTP meldet
bewusst keine vollständige RW-Capability: ein einziges SFTP-v3-Subsystem nutzt
Standard-`SSH_FXP_RENAME`, das ein vorhandenes Ziel nicht ersetzen darf. Eine
zweite Extension-Session oder ein Stat-dann-Rename-Rennen wird nicht zur
atomaren Garantie hochgestuft. Synthetische Peer-Container (`/`,
`/Verbindungen`) melden keine
Schreibgarantien; ein konkreter Export-Unterbaum fragt die Capability des dort
aufgelösten Backends ab. Google Drive kann create/replace, aber keinen atomaren
Namespace-Replace garantieren; WebDAV sowie FTP/FTPS garantieren derzeit nicht
das vollständige Set. Ob ein Ziel read-only überhaupt startet, entscheidet
zusätzlich die obige Strict-/Trusted-Root-Zulassung. Weil beide Capabilities am
aktiven Backend hinter dem vorhandenen Verbindungsweg hängen, kann ein Fallback
den Strict- oder RW-Mount konservativ ablehnen, ohne den Mount-Kern oder die
Credentials zu duplizieren. Für SFTP verlangt der strikte Modus den Agent und
akzeptiert deshalb keinen stillen Agent→SFTP-Fallback. SFTP-v3-Semantik geprüft
am 2026-07-21: [Abschnitt
6.5](https://datatracker.ietf.org/doc/html/draft-spaghetti-sshm-filexfer#section-6.5).

Dateien werden beim ersten Öffnen vollständig in einen lokalen,
mount-spezifischen Spool materialisiert. Der erste lokale Änderungsbyte wird vor
dem Schreiben in einem synchronisierten, dauerhaften Recovery-Journal als dirty
festgehalten. `FlushFileBuffers` beziehungsweise Cleanup/Close synchronisiert
den Spool, lädt eine vollständige Staging-Datei hoch, prüft die beim Öffnen
erfasste Baseline (Objekt-ID, Größe, mtime, optional content-MD5) vor dem Upload
und direkt vor der Promotion erneut und veröffentlicht erst dann atomar. Das
deckt Obsidian-artige Folgen aus wiederholtem Truncate/Write/Flush/Close und den
üblichen Editor-Save per temporärer Datei plus Replace ab. Ein abgebrochener
Upload berührt das Ziel nicht; dirty, mehrdeutige oder konfliktbehaftete Zustände
bleiben für `Retry` im Cache erhalten und verhindern einen scheinbar sauberen
Abschluss.

Vor einem Start prüft der Daemon den lokalen Mount-Cache unter einer exklusiven
Lease, bevor er den Remote-Backendaufbau versucht. Persistierter Zustand ist
explizit `Clean`, `Required` oder `Unknown`; alte Datensätze ohne beweisbaren
Status werden als `Unknown` fail-closed behandelt. Nur `Clean` darf nach einem
reinen Vor-Mount-Fehler als sauber entfernt werden. Dadurch verschleiert etwa
ein SSH-Timeout kein lokales Recovery-Material.

Auch der portable Doppelklick-Pfad ist crash-sicherer: der atomare
`open-temp`-Marker entsteht erst nach erfolgreichem Download. Leere Legacy-
Marker werden entfernt, ungültige Marker bleiben fail-closed, und echte
wiederherstellbare Sitzungen sind Hinweise statt App-Fehler. Ein bereits
beendeter Electron-/Obsidian-Launcher beweist keinen Editor-Abschluss; Temp-Datei
und Marker bleiben über Single-Instance-Weiterleitung und ein vorübergehendes
Verschwinden während atomarer Save-back-Zyklen erhalten.

Die Baseline-Prüfung ist bewusst kein universelles serverseitiges Compare-and-
Swap. Ohne conditional commit des konkreten Backends bleibt zwischen der
letzten Prüfung und der atomaren Promotion ein kleines TOCTOU-Fenster. Jeder
geänderte Flush überträgt die ganze Datei, sodass große Dateien und hohe Latenz
direkt als Anwendungspause sichtbar werden; lange Callbacks verlängern ihren
Dokany-Timeout alle 30 Sekunden auf fünf Minuten. Die angezeigte freie Kapazität
ist die lokale Spool-Kapazität, Remote-Quoten melden sich erst beim Commit.
Setzen von ACLs/Security Descriptors, Creation/Access/Write-Time, beliebigen
Dateiattributen, Alternate Data Streams und Reparse Points ist nicht
implementiert. Security-Abfragen lässt Dokany mit einem synthetischen
Current-User-Descriptor beantworten; Remote-Symlinks werden nicht durchquert.

## Stack

- `eframe 0.29` + `egui_extras` — immediate-mode GUI, virtualisierte Tabelle;
  ein äußerer horizontaler Scrollbereich hält breite Detailspalten auch bei
  eingeblendetem Seitenpanel erreichbar
- `rayon` — parallele Walker
- `crossbeam-channel` — Lock-free Channels für Stream-Updates
- `regex`, `globset` — Filter-Muster
- `chrono` — Datumshandling
- `rfd` — natives File-Dialog (Win32 Common Controls)
- `trash` — Papierkorb
- `windows-sys` — `GetLogicalDrives` für Laufwerksliste

## Limitierungen vs. Electron-Variante

- **Web-/Electron-Ökosystem:** bewusst nicht enthalten; Erweiterungen müssen in
  Rust/native umgesetzt werden.
- **Windows 11 modernes Kontextmenü:** COM-DLL und Sparse-Package-Manifest sind
  gebaut, aber die Aktivierung braucht ein vertrauenswürdiges Codesigning-Zertifikat
  (siehe [`../docs/WIN11_CONTEXT_MENU.md`](../docs/WIN11_CONTEXT_MENU.md)).
- **NTFS-MFT-Scan:** als spätere, erhöhte Windows-Option geplant; der normale
  parallele Walker bleibt der universelle Pfad.

## Zur weiteren Beschleunigung Richtung WizTree

Die schnelle Standardanalyse nutzt den parallelen Walker und die eigene
Storage-Analytics-Pipeline. WizTree liegt bei 1-3 Mio/s durch direktes
NTFS-MFT-Lesen. Mögliche Ergänzungen für Folge-Versionen:

1. `FindFirstFileExW` mit `FIND_FIRST_EX_LARGE_FETCH` und `FindExInfoBasic`
   (überspringt 8.3-Aliase, batcht Kernel-Calls) → ~2× zusätzlich
2. NTFS-MFT-Reader via `\\.\C:` mit `FSCTL_ENUM_USN_DATA` (braucht Admin) →
   1-3 Mio/s, ähnlich WizTree
