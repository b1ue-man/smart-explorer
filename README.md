# Smart Explorer

Schneller nativer Datei-Explorer für Windows und Linux (Rust + egui). Filtert Dateien/Ordner
über die gesamte Ordnertiefe (Name/Regex/Glob, Größe, Datum via Kalender, Typ),
kopiert gefiltert mit Strukturerhalt (auch über die Windows-Zwischenablage in den
Explorer), Fuzzy-Ordnersuche mit Live-Index, Tabs + Split-Screen, Shell-Kontextmenü.
Breite Datei-/Detailspalten bleiben per horizontalem Scroll erreichbar, auch
wenn ein Detailbereich die verfügbare Tabellenbreite verkleinert.

**Remote/Cloud (ab 0.4.x):** durchsucht **SFTP**, **FTP/FTPS**, **WebDAV**
(Nextcloud/ownCloud) und authentifizierte **Netzlaufwerke (UNC)** über eine
einheitliche `Backend`-Schnittstelle (Sidebar → **VERBINDEN**); Zugangsdaten im
Windows Credential Manager. **Einseitige Spiegelung** ("⇅ Spiegeln nach…") sichert
den aktuellen (lokalen oder Remote-)Ordner in einen lokalen Zielordner.

Das Remote-Rechtsklickmenü unterscheidet Zeile, Mehrfachauswahl und freien
Ordnerhintergrund. Es bietet je nach Ziel unter anderem Öffnen, Herunterladen,
Kopieren/Einfügen, Umbenennen, Löschen, Favoriten, Remote-Pfad kopieren,
Ordneranalyse, Aktualisieren sowie **Neu** mit Ordner und editierbaren
Dateitypen. Unter Windows kann **Öffnen mit…** auch für Remote-Dateien den
nativen App-Auswahldialog verwenden; Smart Explorer materialisiert dafür eine
überwachte lokale Kopie und lädt gespeicherte Änderungen über den bestehenden
Save-back-Pfad zurück.

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

**Remote als echtes Windows-Laufwerk (optional):** Eine gespeicherte Verbindung,
Google Drive oder ein konkretes Share-Ziel kann mit einem Laufwerksbuchstaben in
Explorer und normalen Programmen erscheinen – in derselben Art, wie Cryptomator
einen Tresor als virtuelles Laufwerk bereitstellt. Das ist ein
**Dokany-Dateisystem, ausdrücklich kein CfAPI-Sync-Root und kein
Placeholder-Modell**. Smart Explorer bleibt der einzige Remote-Client: Das
Laufwerk benutzt das bereits gewählte `Backend` einschließlich seiner
Direkt-/Relay-/SSH-Verbindungswege und Fallbacks; Anmeldedaten und das eigentliche
Remote-Ziel bleiben im Daemon und werden nicht an den isolierten
Dateisystem-Host weitergereicht.

Eine laufende Einbindung besitzt deshalb erwartungsgemäß drei Prozesse: GUI,
den langlebigen Smart-Explorer-Daemon und genau einen isolierten Mount-Host.
Der Daemon besitzt das gemeinsame `Backend` und dessen aktive Sitzung; der
Mount-Host spricht ausschließlich über private lokale IPC mit ihm. Das
Schließen eines Remote-Tabs beendet die Laufwerksverbindung nicht und die
beiden Hintergrundprozesse sind keine separat neu verbundenen Remotes.

Voraussetzung unter Windows ist die offizielle
[Dokany-2.3.1.1000-Laufzeit](https://github.com/dokan-dev/dokany/releases/tag/v2.3.1.1000)
mit DLL-API **231** und Kernel-Treiberprotokoll **0x190** (dezimal **400**).
Der empfohlene Smart-Explorer-Installer enthält die exakt gepinnte offizielle
`Dokan_x64.msi` offline als standardmäßig ausgewählte optionale Komponente.
Portable und per Auto-Update aktualisierte Installationen können dieselbe
Runtime aus der GUI oder mit `se drive install-runtime` sicher nachinstallieren.
Die Smart-Explorer-Basis bleibt eine per-user-Installation; nur der Windows
Installer (`msiexec`) fordert für den systemweiten Treiber UAC an. Die offizielle
signierte Runtime braucht weder Developer Mode noch `TESTSIGNING`, und Smart
Explorer lädt weiterhin ausschließlich `%WINDIR%\System32\dokan2.dll`.
`DokanVersion()` muss die DLL-API 231 melden; `DokanDriverVersion()` fragt das
getrennte Kernel-Protokoll ab und muss 0x190/400 melden. Eine fehlende Runtime
beziehungsweise ein nicht verfügbarer Treiber kann automatisch installiert
werden. Eine bereits vorhandene, tatsächlich inkompatible gemeinsam genutzte
Runtime wird dagegen nicht automatisch ersetzt oder herabgestuft, sondern mit
dem konkret abweichenden Wert gemeldet. Stand der Primärquellenprüfung:
2026-07-22
([Dokany-Projekt](https://github.com/dokan-dev/dokany),
[API-Header mit `DOKAN_VERSION 231`](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/dokan/dokan.h),
[Treiber-Header mit `DOKAN_DRIVER_VERSION 0x0000190`](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/sys/public.h),
[`DokanVersion`/`DokanDriverVersion`-Implementierung](https://github.com/dokan-dev/dokany/blob/v2.3.1.1000/dokan/version.c)).

In der GUI startet das Laufwerkssymbol neben einer gespeicherten Verbindung,
Google Drive oder einem Share-Gerät den Dialog **Remote als Laufwerk**; das
Toolbar-Laufwerkssymbol öffnet den Manager zum Auswerfen oder erneuten
Verbinden. Der Terminal-Companion bietet dieselbe Steuerung:

```powershell
se drive runtime
se drive install-runtime
se drive mount @prod:/srv --letter M --metadata-depth 2
se drive mount @prod:/notizen --letter N --read-write
se drive mount sftp://host/srv --letter S --trust-remote-root
se drive list
se drive unmount M:
se drive retry <mount-id>
```

Die Metadaten-Tiefe ist in GUI und CLI von 0 bis 4 einstellbar und standardmäßig
2. Vor der Laufwerksbereitschaft lädt Smart Explorer nur ein vollständiges
Root-Snapshot; tiefere vollständige Verzeichnis-Snapshots folgen danach
breitensuchend in kleinen Hintergrund-Batches und werden rotierend alle
20 Sekunden erneuert. Das cached ausschließlich Namen und Dateimetadaten, nie
Inhalte: maximal 4.096 Verzeichnisse, 50.000 Einträge, 32 MiB insgesamt und
4 MiB pro Verzeichnis, ergänzt um einen kurzlebigen 4-MiB-Punktcache.
Öffnen/Erstellen, Konfliktprüfungen und Mutationen fragen weiterhin live ab;
lokale Änderungen invalidieren betroffene Snapshots sofort.

Die Wurzelisolation ist unabhängig vom Schreibmodus standardmäßig strikt. Bei
einem SFTP-Ziel verlangt ein solcher Mount eine gespeicherte Verbindung mit
aktiviertem, erfolgreich gestartetem Linux-SSH-Agent; ein Agent-Fehler wird
angezeigt und darf für den Laufwerksstart nicht still zu plain SFTP werden.
Normales Browsing darf bei einem Agent-Deployment-Fehler weiterhin die bereits
aufgebaute plain-SFTP-Verbindung verwenden. Nach erfolgreichem Agent-Handshake
werden einzelne fehlgeschlagene Agent-Operationen dagegen nicht blind über
SFTP wiederholt.
Der Agent bindet die exakte Wurzel vor seinen
Worker-Threads mit Landlock ABI 3+; Google Drive bleibt durch seine
Parent-ID-Navigation in der ausgewählten Provider-Hierarchie. Plain SFTP,
Local/UNC, WebDAV und FTP können einen Pfad dagegen nicht atomar gegen einen
gleichzeitig ausgetauschten Symlink/Junction absichern. Sie benötigen deshalb
die ausdrücklich zu bestätigende GUI-Option **Remote-Wurzel ohne technische
Sandbox vertrauen** beziehungsweise `--trust-remote-root` – auch read-only.
Smart Explorer validiert und serialisiert Pfade dort weiterhin, vertraut aber
dem Server und anderen Schreibern während eines Zugriffs. Bei einem Share-/Peer-
Ziel wird nicht pauschal diese schwächere Einstufung angenommen: Smart Explorer
prüft den konkret exportierten Unterbaum daemonseitig. Ein Peer-Export eines
Agent-confined Backends kann so `Enforced` und – bei allen drei
Schreibgarantien – RW bleiben; ein exportiertes Local-/UNC-/plain-SFTP-Ziel
bleibt wegen seines Check-to-operation-TOCTOU `Unverified` und braucht die
explizite Vertrauensfreigabe.

Ohne `--read-write` ist das eingebundene Laufwerk absichtlich
schreibgeschützt. Zusätzlich zur Root-Zulassung wird der Schreibmodus schon
beim Einbinden abgelehnt, wenn das aktive Backend nicht alle drei Garantien
`create`, `replace` und `namespace_replace` meldet. Dabei bedeutet `create`
eine atomare exklusive Übernahme des zufälligen Staging-Namens, nicht einen
Stat-dann-Create-Test. Der SSH-Agent sowie Local/UNC besitzen diese
Schreibprimitive. Plain SFTP meldet sie bewusst nicht: Smart Explorer hält nur
ein SFTP-v3-Subsystem offen und nutzt dessen Standard-Rename ohne Replace; eine
zweite Extension-Verbindung oder ein Stat-dann-Rename-Rennen wird nicht als
atomare Garantie ausgegeben. Für Share-Geräte sind `/Label` und
`/Verbindungen/<Verbindung>` konkrete mountbare Wurzeln; das aggregierte `/` ist
synthetisch und bleibt read-only. Vor der Auswahl fragt die GUI über den Daemon
das echte entfernte Peer-Backend ab. Der gestartete Mount erhält anschließend
eine an den authentifizierten Geräte-Prinzipal, die exakte Wurzel sowie Export-
und Autorisierungs-Epoche gebundene Lease. Direkt-/Relay-Wechsel und ein
physischer QUIC-Reconnect sind nur Transportwechsel: Smart Explorer löst dafür
aktuelle Presence-Routen auf und verwendet dieselbe Lease weiter, solange
Identität, Wurzel und Policy unverändert sind. Eine andere Identität, Wurzel
oder Autorisierungs-Epoche scheitert dagegen geschlossen und verlangt `Retry`
beziehungsweise erneutes Einbinden. Ein Fallback wird vor dem Mount mit seinen
tatsächlichen Root- und Schreibgarantien zugelassen und schwächt RW nie still zu
unsicherem Schreiben. Für einen strikten SFTP-Mount ist Agent→SFTP ausdrücklich
kein zulässiger Fallback.

Auch das Stoppen einer Freigabe oder das Entziehen beziehungsweise Ändern ihrer
Autorisierung sperrt neue Operationen synchron und macht aktive Peer-Mount-
Leases ungültig. Nach einer erneuten Freigabe ist deshalb ebenfalls `Retry` oder
ein Remount nötig. Eine bereits zugelassene Einzeloperation darf noch
abschließen; mehrphasige Schreibvorgänge prüfen die Autorisierung vor Flush und
Promotion erneut.

Der SSH-Aufbau staffelt aufgelöste IPv6-/IPv4-Adressen abwechselnd mit 250 ms
Versatz, behält den ursprünglichen Hostnamen für Known-Hosts-Prüfung bei und
liefert typisierte Connect-/Handshake-/Exec-Zeitüberschreitungen. Diagnoseausgabe
von Remote-Kommandos ist begrenzt. Der Agent liegt unter einem Namen mit seinem
vollständigen SHA-256, wird vor jedem Start und nach einem Reconnect erneut über
SFTP gegen die eingebetteten Bytes geprüft und erst dann ausgeführt.

Schreibzugriffe landen zuerst in einem lokalen **Whole-file-Spool**. Dadurch
funktionieren auch wiederholte Editor-Zyklen wie bei Obsidian
`truncate → write → flush → close` sowie `Temp-Datei → atomarer Replace`:
Windows sieht nie eine halb hochgeladene Zieldatei. Vor dem Upload und nochmals
direkt vor der Promotion vergleicht Smart Explorer die beim Öffnen erfasste
Remote-Baseline (ID, Größe, Zeit und – sofern vorhanden – Inhalts-MD5). Nicht
aufgelöste Änderungen und mehrdeutige Commit-Antworten bleiben im dauerhaften
Recovery-Journal und werden als Konflikt angezeigt. Das ist kein serverseitiges
CAS: Zwischen letzter Prüfung und Commit bleibt ohne bedingte Backend-Operation
ein kleines TOCTOU-Fenster. Außerdem kostet jeder Flush einer geänderten Datei
einen vollständigen Upload; Metadaten wie ACLs, Alternate Data Streams,
benutzerdefinierte Zeiten/Attribute und Reparse Points werden nicht emuliert.

Der separate portable Doppelklick-Pfad legt seinen `open-temp`-Recovery-Marker
erst nach einem vollständig erfolgreichen Download atomar an. Eine echte
wiederherstellbare Sitzung erscheint beim Start als Hinweis statt als
App-Fehler. Leere alte Marker werden bereinigt, ungültige Marker bleiben
fail-closed erhalten. Ein kurzlebiger Electron-/Obsidian-Launcher gilt nicht als
Editor-Ende: die Temp-Datei und ihr Marker bleiben für spätere Save-back-Zyklen
erhalten, auch wenn die Datei während eines atomaren Editor-Saves vorübergehend
verschwindet.

Fehlschläge des Laufwerk-Hosts bleiben handlungsorientiert: Smart Explorer
nennt getrennt eine inkompatible DLL-API beziehungsweise ein inkompatibles
Treiberprotokoll, eine abgelehnte Root-/RW-Garantie, erforderliches Remount nach
einer geänderten Peer-Identität/Policy oder erhaltenes Recovery-Material. Zusätzlich
wird die begrenzte Ursache des Host-Prozesses angehängt, statt nur einen
generischen Exit-Code zu zeigen. Ein nachweislich sauberer Eintrag kann im
Laufwerksmanager entfernt werden; für erhaltenes Recovery-Material bleibt
`Retry` der Wiederaufnahmeweg.

**Teilen / P2P (ab 0.5.23):** Dateien an gekoppelte Geräte oder in **Räume**
senden. Der aktuelle Iroh/QUIC-Transport ist **Ende-zu-Ende-verschlüsselt** und
versucht zuerst einen direkten Gerätepfad. Falls der nicht erreichbar ist, kann
der mitgelieferte **`se-share-server`** (Linux + Windows, in
[`release-native/share-server/`](release-native/share-server)) verschlüsselte
Transportpakete weiterleiten; er erhält keine Relation-Secrets oder
Dateisystemdaten im Klartext. Toolbar → **📡 Teilen**; Server in Einstellungen →
**TEILEN**. Der frühere Noise/TCP-Entwurf steht historisch in
[`docs/SHARE_PLAN.md`](docs/SHARE_PLAN.md).

In der Teilen-Ansicht lassen sich das eigene Direct-Gerät und vorhandene Räume
zeitlich begrenzt **suchbar machen**. Fünf Minuten sind voreingestellt; eine
andere positive Dauer ist frei wählbar. Die PIN wird als exakte UTF-8-Bytefolge
verwendet, nicht getrimmt und nicht dauerhaft gespeichert. Es gibt bewusst
keine Mindestlänge: auch eine leere PIN und exakt `0` funktionieren, werden aber
als trivial zu erraten gekennzeichnet. Die getrennten Listen für auffindbare
Direct-Geräte und Räume zeigen den ungeprüften Anzeigenamen, Ablauf und
Kompatibilität. **Connect** plus PIN startet den authentifizierten,
verschlüsselten Schlüsselaustausch vollständig im Hintergrund; die
Freigabeseite des veröffentlichenden Geräts muss nicht noch einmal zustimmen.

Eine erfolgreiche Direct-Kopplung installiert die vollständige Relation auf
beiden Geräten, sodass beide Seiten sofort als Remote-Gerät nutzbar sind.
Bestehende einseitige Direct-Beziehungen werden bei kompatiblen aktuellen
Clients im Hintergrund nachgezogen, sobald beide Geräte online sind. Exakte
Identitäts- oder Relation-Konflikte sowie zuvor ignorierte, abgelehnte,
widerrufene oder gelöschte Beziehungen bleiben dabei fail-closed und werden
nicht automatisch überschrieben.

Die Speicheranalyse eines Direct- oder Raum-Ziels lässt den entfernten
Share-Client den vollständigen, begrenzten logischen Baum aufbauen. Er überträgt
anschließend den fertigen Snapshot samt Fortschritt, Summen und SHA-256-Prüfung;
der anfragende Client validiert es und zeigt daraus direkt die Treemap, statt
den Baum aus einzeln zurückgesendeten Metadatenknoten zusammenzusetzen. Nur bei
einem älteren Peer ohne diese Fähigkeit greift der bisherige Walk-Fallback.

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
Execution ist ab 0.5.134 als separate, standardmaessig deaktivierte
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
reiht sie ueber den Share-Worker ein; ein kompatibles Ziel entscheidet sie nach
der kryptografischen Prüfung automatisch gemäß seiner lokalen Policy. `se
connections add-room --code
SE-R3-... --name Team` tritt einem Raum bei. `queued` bedeutet dabei nur lokal
dauerhaft vorgemerkt; auch eine Relay-Meldung `forwarded` bestaetigt noch keinen
Peer-Empfang. Erst der signierte Request-Receipt des Zielgeraets meldet
`received`. Entscheidung (`pending`/`accepted`/`rejected`/`revoked`), aktive
Autorisierung und aktuelle Verbindung werden getrennt angezeigt.

`se share request` zeigt ohne weitere Argumente die offene Inbox samt
Request-ID, Geraet, Fingerprint, Empfangs-, Entscheidungs- und
Autorisierungsstatus. Eine kryptografisch gültige neue Direct-Anfrage wird vom
Ziel automatisch gemäß seiner dauerhaften lokalen Policy angenommen oder
abgelehnt; im normalen Kopplungsfluss ist daher keine zweite Bestätigung nötig.
Bleibt ein älterer oder importierter Zustand dennoch offen, akzeptiert oder
verwirft `se share request accept` beziehungsweise `reject` bei genau einem
Treffer ohne versteckte IDs oder erneute Fingerprint-Eingabe; bei mehreren
Anfragen nennt die Inbox direkt die gültigen Befehle. `list`, `show`, `retry`
und `delete` verwalten den
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

Meldet sich dieselbe Geraete-ID mit einem anderen Schluessel, Node-ID oder
Fingerprint, bleibt die Anfrage sichtbar, wird aber als Identitaetskonflikt
automatisch fail-closed abgelehnt. Text- und JSON-Ausgabe nennen den
Konflikt sowie direkt nutzbare `revoke`-, `reject`- und `delete`-Befehle;
Ablehnen und lokales Loeschen bleiben auch im Konfliktfall moeglich.

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

Die Desktop-App ist für GNU/Linux mit glibc 2.17+ gebaut und nutzt die üblichen
X11-/Wayland-Clientbibliotheken der Desktop-Distribution. Der Release-Build
startet genau diese Payload vor der Veröffentlichung unter Xvfb.

**Linux nur Terminal (ohne Desktop-/X11-Abhängigkeiten):**

```bash
curl -fsSL https://raw.githubusercontent.com/b1ue-man/smart-explorer/main/install-linux.sh | sh -s -- --cli-only
```

**Windows-Grundinstallation:** Die Smart-Explorer-Basis bleibt ohne Admin und
ohne Setup-Zwang nutzbar. Die optionale systemweite Dokany-Laufzeit für echte
Remote-Laufwerke kann eine Administratorbestätigung anfordern. Zwei Wege:

1. **Installer (empfohlen):** Die aktuelle
   **[`Smart Explorer Setup X.Y.Z.exe`](https://github.com/b1ue-man/smart-explorer/releases/latest)**
   unter **Releases** herunterladen und ausführen.
   Installiert nach `%LOCALAPPDATA%\Programs\Smart Explorer`, legt Startmenü-/
   Desktop-Verknüpfung an, registriert das Rechtsklick-Menü „In Smart Explorer
   öffnen", installiert `se.exe` fuer Terminal-Operationen und trägt dessen
   Ordner in den Benutzer-`PATH` ein (sichtbar in neu geöffneten Terminals) — **und stellt die
   Update-Prüfung auf den Git-Feed ein. Neue Versionen werden automatisch geprüft
   und sicher bereitgestellt; installiert werden sie erst nach deiner Bestätigung.**
   Die offline eingebettete Dokany-2.3.1.1000-Komponente ist im normalen Setup
   standardmäßig ausgewählt; nur dieser MSI-Schritt löst UAC aus. Ein stilles
   Setup installiert Dokany dagegen nur mit `/S /INSTALLDOKANY=1`. Smart Explorer
   entfernt eine installierte Dokany-Runtime beim Uninstall nicht.
2. **Portable:** [`Smart Explorer.exe`](release-native/Smart%20Explorer.exe)
   herunterladen und direkt starten (keine Installation). Für Auto-Update einmalig
   die Update-Quelle setzen (siehe unten). Das portable Terminal-Binary liegt als
   [`se.exe`](release-native/se.exe) daneben. Falls das Laufwerksfeature Dokany
   benötigt: in der GUI **Dokany installieren** wählen oder in PowerShell
   `se drive install-runtime` ausführen. Der Download ist auf die veröffentlichte
   URL, Dateigröße und SHA-256 festgelegt, wird per Authenticode geprüft und
   fordert erst für `msiexec` UAC an. Eine erkannte inkompatible gemeinsam
   genutzte Dokany-Version wird nicht automatisch ersetzt.

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
| `native/publish-release-local.ps1` | Einziger vollständiger Windows-/Linux-Release-Einstieg: Preflight, Patch-Bump, Build, Prüfung, Commit/Push, Tag, GitHub Release und lokale `se`-Aktualisierung |
| `native/publish-feed.sh` | Interner Linux-Cross-Build des Top-Level-Wrappers; ein direkter vollständiger Aufruf ohne geerbtes Release-Lock wird verweigert |
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

1. Den gesamten vorgesehenen Aufgabenblock fertigstellen und über seine eine
   automatisierte Task-Suite validieren. Zwischencommits sind keine Releases.
2. Für den automatisierten Remote-Pfad `build.yml` genau einmal auf `main` mit
   `complete_release_source_sha=<vollständiger aktueller origin/main-SHA>`
   dispatchen; `verify_release_candidate` und `publish_release` bleiben dabei
   aus. Der fest auf `windows-2025` laufende Job bindet Ref, Checkout und
   Remote-`main` an exakt diesen SHA, richtet das gepinnte Windows-/Ubuntu-WSL1-
   Release-Environment ein und ruft ausschließlich den Top-Level-Wrapper auf.
   Das ist der maßgebliche unbeaufsichtigte Pfad: lokal werden nur Commit, Push,
   Dispatch und Monitoring ausgeführt; Build, Tests, Paketierung und
   Veröffentlichung laufen auf GitHub-Runnern.
   Alternativ führt ein menschlicher Release-Operator lokal den nicht bauenden
   Preflight aus:
   `pwsh ./native/publish-release-local.ps1 -CheckEnvOnly`.
3. Beim lokalen Operator-Pfad danach genau einmal
   `pwsh ./native/publish-release-local.ps1` starten. Im Remote-Pfad übernimmt
   das bereits der eine Dispatch. Dieser
   eine Wrapper erhöht die Patch-Version, hält das gemeinsame Cross-Host-Lock,
   baut unter Windows/WSL oder Linux alle Plattformartefakte, prüft die sechs
   Feed-Hashes, die im Installer eingebetteten App-/Updater-/`se`-Bytes, den
   gebundenen Quell-Commit und 18 Release-Assets, committet und pusht den
   exakten Kandidaten nach `main` und
   startet genau eine statische Exact-Byte-Publikation. Normalerweise geschieht
   das über einen unveränderlichen Tag; nur wenn dessen Push technisch abgelehnt
   wird und der Remote-Tag noch fehlt, verwendet derselbe Wrapper einmalig den
   gegenseitig ausschließenden Pfad `release/vX.Y.Z`. CI baut und testet diesen
   Kandidaten nicht erneut, sondern veröffentlicht ausschließlich die bereits
   geprüften Commit-Bytes. Auf Linux aktualisiert der Wrapper danach das lokale
   `se` aus genau diesem Tag und übergibt den Daemon an die neue Version. Unter
   GitHub Actions löst der Wrapper wegen GitHubs Rekursionsschutz mit dem
   Job-Token genau einen internen `publish_release=true`-Dispatch gegen den
   exakten Kandidaten-Ref aus und überwacht dessen zurückgegebene Run-ID; der
   Wrapper wartet dabei ausschließlich für eine frisch zurückgegebene Run-ID
   bis zu zwei Minuten auf GitHubs anfänglich noch unvollständige Ref-/SHA-
   Metadaten. Abgeschlossene Abweichungen sowie spätere Drift brechen sofort ab.
   Der lokale Tag-/Fallback-Pfad bleibt unverändert. Ein
   Fehler vor dem Tag bleibt bei derselben vorgesehenen Version; Tags werden
   niemals verschoben oder überschrieben. Ist nur der erste bereits getaggte
   Workflow fehlgeschlagen, kann derselbe Wrapper diesen exakten Run einmal mit
   unverändertem SHA/Ref fortsetzen, ohne eine zweite Pipeline zu erzeugen.

`verify_release_candidate`/`verify/v*` dienen nur einer ausdrücklich
angeforderten Verifikation **ohne** Release und werden dem normalen Taglauf
nicht vorgeschaltet. `-SkipLinuxFeed` bleibt ein nicht publizierbares
Windows-Diagnosebundle. Der Preflight benötigt neben den Build-Werkzeugen und
7-Zip auch nichtinteraktive Git-Schreibrechte sowie `GH_TOKEN`/`GITHUB_TOKEN`
oder eine gültige `gh auth login`-Sitzung für die authentifizierte Überwachung.

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
