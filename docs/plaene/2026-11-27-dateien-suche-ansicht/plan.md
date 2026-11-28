# Dateioperationen: Wechseldatenträger, Volltextsuche, Arbeitsziel für Agenten, Textansicht (2026-11-27)

Stand: ausgeliefert mit v0.5.208 (2026-11-28; Suite `crates-task` Lauf 46095407151 grün, Release 46101256778). Offene Reste: `docs/TODO.md` SUCHE-EXEC-SHARE-CWD, SUCHE-ERSETZEN, WECHSELMEDIUM-REST.

Auftrag des Nutzers (2026-11-27, nach v0.5.207):

1. „reviewe die cli befehele für file operations in verbindung mit usb sticks und co, da ist wohl als n fehler.“
2. „file operationen selbst sollen ausgebaut werden. notepad++ hat eine sehr schnelle full text search die auch regex nutzen kann, über files und ordner hinweg. sowas möchte ich, auch so rasend schnell. des weiteren entsprechende read und edit operationen falls es die noch nicht gibt“
3. „ich will dass n agent basically n remote setzen kann als ziel einmal und dann read edit write operationen sowie ls und andere file system operationen dort direkt ausführen kann, ohne den umweg über exec, wie wenn der nutzer es offen hätte in nem tab defakto“ — präzisiert: „for connected remotes, via share direct, groups or also for servers like sftp n stuff if possible there“.
4. „für den computer client und auch die app selbst … die suchfunktionen für fulltext, mit und ohne regex, in files, folders etc. wobei wie in notepad++ auch ordnerpfad mit regex angegeben werden können soll“
5. „eine file ansicht für textbasierte files aller art, also txt, md, aber auch programmiersprachen wie html c# … ide mäßig farblich markiert … die ansicht so zu gestalten dass sie getrent von dem farbrendering funzt und dies dann wie üblich für solche sachen in seperaten language packets verwaltet wird“; „md natürlich mit md rendering“.
6. Ausgenommen: Volltextsuche und Vorschau für Binär- und Office-Dateien („bleibt erstmal aus“).

Geräte-Installationen auf den verbundenen Geräten verwaltet der Agent nicht (Nutzer 2026-11-27).

## A. Befund Wechseldatenträger (Codeprüfung 2026-11-27)

Zwei Prüfungen (Linux, Windows) haben die Wege `se ls|stat|cat|get|put|cp|mv|rm|mkdir|search` bis in
`vfs::LocalBackend`, `local_access` und `transfer::copy` gelesen. Belegte Fehler:

| Kennung | Plattform | Befehl | Fehler | Korrektur |
|---|---|---|---|---|
| U1 | Linux | cp/get/put/mv | `create_private_child`/`verify_private` (`core/src/local_access/os/linux/create.rs`) verlangen Modus `& 0o077 == 0`; vfat/exFAT/ntfs3/ntfs-3g/exfat-fuse zeigen `0777 & ~dmask`: jedes Aufräumen einer eigenen Stufe oder Quarantäne scheitert („created entry is not owner-private“); `mv` lässt die Quelle als `.….move`, ein Abbruch lässt `.….part` bis 4 GiB liegen | Dateisysteme ohne speicherbare Modi erkennen (fstatfs-Magic, wie `confined_local.rs`); dort die eigene Datei nach dev/ino-Abgleich des gehaltenen Handles direkt mit `unlinkat` löschen; einen gerade angelegten `.held`-Ordner bei Fehlschlag entfernen |
| U2 | Linux | cp/mv in vorhandenen Ordner | `reported_name` liefert den eingetippten Namen; auf vfat/exFAT findet der Kernel „Fotos“ für „fotos“, das Listing meldet „Fotos“: „destination child disappeared after preflight“ | auf vfat/msdos/exfat den gespeicherten Namen aus dem Elternlisting melden (gleiche Faltung wie Windows' `reported_name`) |
| U3 | Linux | mv (Abbruch) | Rückbenennen aus der Quarantäne (`linux_recovery.rs`, `quarantine.rs`) nutzt `renameat2(RENAME_NOREPLACE)` ohne Ersatzweg; FUSE ohne RENAME2 (ntfs-3g) liefert EINVAL | derselbe Ersatzweg wie `publish_no_replace` (`linux_publish.rs`) |
| U4 | beide | `--resume-job` | `stable_identity` ist auf FAT/exFAT/FUSE `None`; Fortsetzen lehnt „Gespeicherter Zielroot/Quellordner ist geändert oder unbestätigt“ ab | schwächere, benannte Identität: Volume-Kennung (UUID/Seriennummer) + volume-relativer Pfad (+ Erstellzeit unter Windows); der Inhaltsvergleich des Fortsetzens bleibt |
| U5 | Linux | rm | Einhängepunkte (`/media/<u>/<STICK>`) sind keine geschützten Wurzeln; `rm -r --force` leert den Stick, danach EBUSY | Einhängepunkte (mountinfo) als geschützte Wurzeln wie `rm --preserve-root=all` |
| U6 | Linux | cp/mv große Dateien | `set_len(size)` vorab („sparse“) füllt auf vfat (exFAT vor valid_size) die ganze Datei mit Nullen: doppeltes Schreibvolumen | ohne Sparse-Unterstützung laut `fs_profile` nicht vergrößern |
| U7 | beide | cp/get/put/mv | keine Vorprüfung von Namen (FAT/exFAT/NTFS-Zeichen, Punkt/Leerzeichen am Ende) und Dateigröße (FAT32 4 GiB − 1); Fehler erst nach bis zu 4 GiB („File too large“/„Invalid argument“, Windows 223/112), Windows bricht den ganzen Auftrag ab | Prüfung im Plan gegen `fs_profile`-Grenzen (`target_limits`), je Datei mit Grund melden und weitermachen (robocopy/rsync); Windows: `ERROR_FILE_TOO_LARGE` als endgültiger Stufenfehler |
| U8 | Linux | Veröffentlichungsfehler | ntfs3-Magic 0x7366746e fehlt in der Nicht-Veröffentlichungsprüfung (`transfer/src/copy/os/linux_os.rs`) | ergänzen |
| U9 | Linux | ls/search/cp | ein Nicht-UTF-8-Name bricht die ganze Auflistung ab (`local.rs collect`) | tolerante Auflistung, ausgelassene Einträge melden |
| U10 | Linux | mv | reine Umbenennung der Schreibweise auf vfat/exFAT abgelehnt („same path or file“) | über eindeutigen Zwischennamen umbenennen |
| U11 | Linux | cp | Namenskollision nach Faltung („a.txt“/„A.txt“ nach vfat) meldet „destination changed during copy“ | Kollision im Plan erkennen, beide Namen nennen |
| U12 | Linux | mv | `raw_os_error() == Some(17)` ist unter Linux EEXIST, nicht ERROR_NOT_SAME_DEVICE | Windows 17, Linux EXDEV 18 |
| U13 | Windows | mv | Dateikennung auf FAT hängt an der Lage des Verzeichniseintrags; das Umbenennen in den längeren Quarantänenamen ändert sie; `move_guard.rs` vergleicht danach mit dem alten Snapshot: jede Datei „source changed during transfer“ | Snapshot nach dem Umbenennen aus dem gehaltenen Handle; Gleichheit über zwei lebende Handles (`same_file_object`); auf FAT/exFAT Länge + Änderungszeit; dasselbe in `local_publish.rs`, `restore_quarantine`, `remove_quarantine` |
| U14 | Windows | cp/get/put/mv (nicht erhöht) | `can_request_access` ist für jedes Laufwerk wahr: der alte Spool-Weg verschlüsselt den ganzen Baum vorab nach %TEMP% (Platz in Höhe der Quelle) und schreibt ohne Zeiten/Attribute | Engine-Weg ohne vorgezogene Zugriffsabfrage; nur eine tatsächlich verweigerte Datei fragt nach Zugriff; im alten Weg Zeiten und Attribute setzen |
| U15 | Windows | cp/mv | `\\?\Volume{GUID}\…`/`\\?\E:\…`: Elternpfad wird zum Volume-Gerät ohne `\` | Verbatim-Wurzeln als Wurzel mit `\` |
| U16 | Windows | ls/search/cp/mkdir | in einen NTFS-Ordner eingehängtes Volume: Auflistung mit `FILE_FLAG_OPEN_REPARSE_POINT` zeigt den leeren Einhängeordner; Schreiben lehnt ihn als Link ab | Mount-Point auf ein Volume als Volume-Grenze, nicht als Link |
| U17 | Windows | rm/mv | Rückfall von `FileDispositionInfoEx` löscht schreibgeschützte Dateien auf FAT nicht (Schreibschutz bleibt) | READONLY auf dem gehaltenen Handle entfernen, dann löschen (wie `delete_own`) |
| U18 | Windows | ls (leerer Kartenleser) | ohne prozessweites `SEM_FAILCRITICALERRORS` kann der Systemdialog „kein Datenträger“ blockieren; ein entferntes Medium liefert eine stille, unvollständige Liste | Fehlermodus prozessweit; Fehler am Verzeichnis-Handle weitergeben |

## B. Volltextsuche

### B1. Vorbild Notepad++ „Find in Files“

(Recherche 2026-11-27 aus Handbuch npp-user-manual.org/docs/searching und Quelltext `FindReplaceDlg.cpp`,
`Notepad_plus.cpp`, `Common.cpp`, `BoostRegExSearch.cxx`, Zweig master; neueste Version v8.9.9.1.)

- **Felder:** Find what, Replace with, Filters, Directory (genau ein Ordner; „<<“ übernimmt den Ordner des aktiven
  Dokuments), In all sub-folders (Standard an), In hidden folders (Standard aus; betrifft nur Ordner, versteckte
  Dateien werden immer durchsucht), Match whole word only (im Regex-Modus gesperrt), Match case, Search Mode
  Normal / Extended (`\n \r \t \0 \\ \b \o \d \x \u` werden zu Zeichen, dann wörtlich) / Regular expression mit
  „. matches newline“, Find All, Replace in Files.
- **Filters:** Muster durch Leerraum getrennt, keine Anführungszeichen; `*` und `?` (Windows `PathMatchSpec`) nur
  gegen den Dateinamen; `!muster` schließt Dateien aus und gewinnt unabhängig von der Reihenfolge; `!\muster` schließt
  Ordner der ersten Ebene nach ihrem Namen aus, `!+\muster` Ordner jeder Ebene; leer = `*.*`, nur Ausschlüsse =
  `*.*` plus Ausschlüsse. Beispiele: `*.cpp *.cxx *.h *.hxx *.hpp`, `*.* !*.exe !*.obj !*.log`, `*.* !\tests !\bin*`,
  `*.* !+\log*`.
- **Kein Regex auf Ordnerpfade:** Notepad++ prüft Ordner nur über den Namen des einzelnen Ordners mit Wildcards und
  kennt keinen Einschluss von Ordnern. Der Ordnerpfad-Regex ist ein ausdrücklicher Wunsch des Nutzers
  („ordnerpfad mit regex“) und kommt **zusätzlich** zur Notepad++-Filtersyntax (Felder „Ordnerpfad (Regex)“ und
  „Ordnerpfad ausschließen (Regex)“, gegen den relativen Pfad mit `/`).
- **Regex:** Boost.Regex (Perl/ECMAScript), immer über das ganze Dokument: `^`/`$` an Zeilen, Treffer über Zeilen
  möglich, `\R`, Lookahead, Lookbehind fester Länge, `\K`, atomare Gruppen, Rückverweise.
- **Ergebnisfenster:** unten angedockt; Kopf `Search "foo" (1 hit in 1 file of 1 searched) [Normal: Case/Word]`, je
  Datei eine Zeile mit Trefferzahl, je Treffer `Line 12: <Zeile>` mit hervorgehobenem Treffer, Zeilen über 1024
  Zeichen gekürzt; Doppelklick/Enter öffnet die Datei am Treffer, F4/Shift+F4 nächster/voriger Treffer;
  Kontextmenü: alle auf-/zuklappen, Kopieren, ausgewählte Zeilen kopieren, Pfade kopieren, Pfade öffnen, alles
  auswählen, leeren, „in diesen Ergebnissen suchen“, Zeilenumbruch, „bei jeder Suche leeren“.
- **Kodierung:** BOM (UTF-8, UTF-16 LE/BE), sonst UTF-8 wenn gültig, sonst ANSI. Binärdateien werden nicht
  übersprungen (hier schon: Binärdateien sind ausgenommen).
- **Geschwindigkeit:** Notepad++ durchsucht die Dateien nacheinander. „Rasend schnell“ übernimmt deshalb den Weg von
  ripgrep: paralleler Durchlauf mit Work-Stealing, jeder Thread durchsucht die Dateien, die er findet; Literal-
  Vorfilter (memchr, Teddy SIMD), Suche über den Puffer statt Zeile für Zeile, Zeilenzählung nur bis zum Treffer,
  Abbruch beim ersten NUL-Byte (Binär), BOM-Sniffing.

### B2. Entwurf

- **Ein Suchkern** in `storage/src/agent_proto` (wird per `#[path]` auch in `se-agent` eingebunden): Abfrage
  (`ContentQuery`), Pfadfilter (Notepad++-Filterfeld, Ordner-Regex einschließen/ausschließen), Treffer
  (`ContentEvent`), Abgleich über die ripgrep-Bibliotheken `grep-searcher`/`grep-regex`/`grep-matcher`
  (Literal-Optimierungen, SIMD, Binärerkennung, BOM-Erkennung UTF-16) und ein paralleler Durchlauf
  (rayon, Links werden nie betreten).
- **Wo gesucht wird:** immer dort, wo die Dateien liegen.
  - lokal: der Kern auf den eigenen Kernen;
  - SSH-Server: der `se-agent` (neuer Rahmen, Fähigkeit `content_search_v1`, `PROTO_VERSION` + 1);
  - Share Direct und Gruppen: der Host (neue Anfrage `FsRequest::ContentSearch`, Fähigkeit
    `content_search_v1`), innerhalb der Rechte der Freigabe (`FsAccess`), lokale Wurzeln mit der
    handle-gebundenen Traversierung der Hash-Wanderung;
  - FTP/FTPS, WebDAV, SMB, Google Drive, SFTP ohne Agent und ältere Gegenstellen: der Kern liest die
    Dateien über das Backend (`list_dir` + `open_read`), mehrere Dateien gleichzeitig.
- **Backend-Schnittstelle:** `Backend::content_search(root, &ContentQuery, Sender<ContentEvent>, &AtomicBool)
  -> VfsResult<ContentSummary>`; Standard = Lesen über das Backend; Agent-, Peer-, Hüllen-Backends reichen weiter.

## C. Arbeitsziel für Agenten (MCP)

- Werkzeug `root`: `set` (ein Ziel jeder Art: lokal, `@label:/pfad`, gespeicherte Endpunkte, `share://direct/…`,
  `share://room/…`, `gdrive:///…`, `vault://…`), `show`, `clear`. Danach nehmen alle Dateiwerkzeuge (`list`, `stat`,
  `read`, `write`, `edit`, `mkdir`, `remove`, `copy`, `move`, `search`, `grep`) relative Pfade unter dem Arbeitsziel;
  `..` verlässt es nicht. Absolute Ziele bleiben gültig. `exec` ohne `target` läuft auf dem Gerät/Server des
  Arbeitsziels mit ihm als Arbeitsordner.
- Die Verbindung zum Arbeitsziel bleibt für die Sitzung offen wie ein Tab (auch Share-Peers, SMB, Google Drive);
  nach einem Verbindungsfehler baut der nächste Aufruf sie einmal neu auf.
- Neues Werkzeug `grep` (Inhaltssuche, Ausgabearten Treffer mit Zeilen, nur Dateien, Anzahl je Datei; Kontextzeilen);
  `read` nach Zeilen (`line`, `lines`, nummerierte Ausgabe).

## D. Textansicht und Sprachpakete

- **Kern** `engine/src/textview/`: Dokument über eine Bereichsquelle (lokal und entfernt über
  `Backend::open_read_range`), Kodierung wie Notepad++ (BOM, UTF-8, sonst Windows-1252), Zeilenindex mit dünnen
  Stützpunkten (beliebig große Dateien ohne Vollspeicher), Fenster-Lesen.
- **Ansicht getrennt vom Farbrendering:** die Ansicht erhält `StyledLine { text, spans: [StyledSpan { start, end,
  style }] }` und kennt keine Sprachen; der `Highlighter` (syntect mit reiner Rust-Regex-Engine) wählt die Sprache
  nach Endung, Dateiname und erster Zeile; Themen hell/dunkel; Parserzustand an Stützpunkten, sichtbare Zeilen vom
  nächsten Stützpunkt; sehr lange Zeilen ohne Hervorhebung.
- **Sprachpakete:** Sublime-Syntax (`.sublime-syntax`), das Format von Sublime Text, bat und syntect; ein Paket ist
  eine Datei oder ein Ordner davon. Eingebaut: bats Syntaxsatz (two-face), jede Sprache als eigener Eintrag mit
  Herkunft und Lizenz; eigene Pakete im App-Datenordner `languages/`, verwaltet über `se languages`, die
  Einstellungen der Desktop-App und der Android-App; ein eigenes Paket geht dem eingebauten vor.
- **Markdown:** pulldown-cmark → gemeinsames Darstellungsmodell (Blöcke, Inline-Läufe, Codeblöcke bereits
  hervorgehoben), gerendert von egui (Desktop) und Compose (Android); Umschalter Darstellung/Quelltext.

## E. Oberflächen

| Oberfläche | Suche in Dateien | Textansicht |
|---|---|---|
| CLI | `se grep <ziel> <muster>` mit den Notepad++-Optionen, Ordner-Regex, Kontext, `--files-only`, `--count`, `--json` | `se languages list|add|remove|folder`; `se cat` unverändert |
| MCP | Werkzeug `grep`; Werkzeug `root` (Arbeitsziel); `read` nach Zeilen | – |
| Desktop-GUI | Dialog „In Dateien suchen“ (Strg+Umschalt+F, Kontextmenü eines Ordners) mit den Notepad++-Feldern und den zwei Ordner-Regex-Feldern; Ergebnisbereich unten wie Notepad++ (Kopf, Dateien, Zeilen, F4/Umschalt+F4, Kontextmenü) | Fenster „Ansicht“ (Kontextmenü „Ansehen“, F3, Treffer der Suche): Zeilennummern, Hervorhebung, Sprache wählbar, Umbruch, Suchen in der Datei (Strg+F), Gehe zu Zeile (Strg+G), Markdown-Darstellung; Einstellungsseite „Sprachpakete“ |
| Android-App | Suche im Ordnermenü mit denselben Feldern, Ergebnisliste nach Dateien | Textansicht mit Hervorhebung, Markdown-Darstellung, Suchen in der Datei; Einstellungen „Sprachpakete“ (Paket über die Dateiauswahl hinzufügen) |

Doppelklick und Öffnen bleiben wie heute (zugehörige App); die Ansicht ist ein zusätzlicher Weg.

## F. Meilensteine und Abnahme

| M | Ergebnis | Abnahme (eine Suite `crates-task`) |
|---|---|---|
| M1 | U1–U18 behoben | Unit-Tests je Befund; Integrationstests auf echtem vfat/exFAT (Linux, Loop-Image) und FAT32/exFAT (Windows, VHD) für mv, Abbruch, Fortsetzen, > 4 GiB, Schreibweise |
| M2 | Suchkern mit Notepad++-Filtern, Ordner-Regex, drei Modi, Mehrzeile, Binär/UTF-16/ANSI | Kern-Tests mit allen Notepad++-Beispielen |
| M3 | Suche dort, wo die Dateien liegen: lokal, SSH-Agent (`content_search_v1`), Share-Host (`content_search_v1`), Rückfall durch Lesen | Codec-/Wire-Rundreisen, Host-Suche mit Rechten, Agent-Server, Rückfall-Backend |
| M4 | MCP `root`, `grep`, `read` nach Zeilen, Verbindung wie ein Tab; `se grep` | Auflösung unter dem Arbeitsziel, Neuaufbau nach Fehler, Katalogtests, CLI-Exit-Codes |
| M5 | Textansicht-Kern, Sprachpakete, Markdown-Modell, `se languages` | Kodierung, Index, Hervorhebung, Pakete, Markdown |
| M6 | Desktop-GUI: Suche + Ergebnisbereich + Ansicht + Sprachpakete | GUI-Kern-Tests der Zustände (Ergebnisbaum, Navigation, Ansicht-Fenster) |
| M7 | Android: Suche + Ansicht + Sprachpakete über die Brücke | Rust-Tests der Brückenmethoden, Kotlin-Tests der Zustände |
| M8 | Doku: `docs/oberflaechen/dateisuche.md`, `textansicht.md`, INDEX, AI_AGENT_PLUGIN.md, SKILL.md, ARCHITEKTUR.md, GOTCHAS.md (FAT/exFAT) | Doku-Kontextprüfung |

## G. Nicht Teil dieses Auftrags

- „Replace in Files“ (Notepad++ hat es; verlangt sind Suche und für Agenten read/edit).
- Volltextsuche und Vorschau für Binär- und Office-Dateien (Nutzer: „bleibt erstmal aus“).

## H. Quellen

- Notepad++: https://npp-user-manual.org/docs/searching/ ; github.com/notepad-plus-plus/notepad-plus-plus (master,
  `PowerEditor/src/ScintillaComponent/FindReplaceDlg.cpp|.h|.rc`, `PowerEditor/src/Notepad_plus.cpp`,
  `PowerEditor/src/MISC/Common/Common.cpp`, `boostregex/BoostRegExSearch.cxx`), gelesen 2026-11-27.
- ripgrep: https://burntsushi.net/ripgrep/ ; github.com/BurntSushi/ripgrep (`crates/ignore/src/walk.rs`,
  `crates/searcher`, `crates/regex/src/literal.rs`, `GUIDE.md`); https://burntsushi.net/regex-internals/.
- crates.io 2026-11-27: grep-searcher 0.1.18, grep-regex 0.1.15, grep-matcher 0.1.10 (MSRV 1.96), encoding_rs_io
  0.1.8, syntect 5.3.0, two-face 0.5.2+bat-0.26.1, pulldown-cmark 0.13.6, fancy-regex 0.16 (syntects Version).
