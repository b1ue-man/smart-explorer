# Exec-Rechte, se Desktop, Cryptomator-Tresore — Plan (2026-10-17)

Stand: 2026-10-17. Codestand bei Planbeginn: `da2646d2` (nach v0.5.182). Nichts in diesem Plan ist gebaut oder ausgeführt (AGENTS.md: keine lokalen Builds/Tests); „umgesetzt“ heißt Code geschrieben, bis die eine Remote-Task-Suite und das Release es bestätigen.

Ablage dieses Auftrags: dieser Ordner. `plan.md` führt A–D und die Hauptstränge; je Hauptstrang eine Datei `strang-<kürzel>.md` mit Schleifenstand, Recherchebefunden und Strangplan; Inventur der Oberflächen in `docs/oberflaechen/`.

## A — Erstplan aus dem Nutzerauftrag (nur Promptwissen)

Quelle aller Ergebnisse: Nutzernachricht vom 2026-10-17 (Arbeitsweise-Auftrag `98399008…`, Nachricht `msg-2026-10-17-permissions-desktop-vaults`, erneut gesendet als `msg-09d1d9a5…`). Zitate sind wörtlich.

### Beauftragte Ergebnisse

| Nr. | Ergebnis | Wortlaut des Nutzers |
|---|---|---|
| E1 | Exec-Rechtesystem neu: an einen Nutzer gebunden statt jeden erlaubten Befehl einzutippen; für Menschen mit vertretbarem Aufwand einrichtbar | „I want it to be user bound not typing every individual allowed command“; „rn this permission system is not usable for a human since it is taking a above accepatbl effort to set up since i suspect even ls would be blocked if not given permission for specifically“; „design a proper fleshed out permission system“ |
| E2 | Eingehende Terminalverbindungen wahlweise als anderer Betriebssystemnutzer als der, unter dem se läuft, damit das Betriebssystem die Rechte durchsetzt | „i would prefer support of setting a user, a different user then se is running as when wished, for the incoming terminal connections so permissions can be handled on the actual operating system“ |
| E3 | Recherche, was auf Android an Nutzerwechsel möglich ist | „on android i assume user is not that possible, but idk so you will have to research this“ |
| E4 | se Desktop auf Windows, Linux und Android (alle unterstützten Systeme): ein Gerät als Ordner; jede Datei aus jedem Ordner über den se Desktop öffnen | „a se Desktop, across windows, linux and android, all suportet systems“; „1. A device as a folder, I wanna be able to open any file from any folder via this se desktop“ |
| E5 | Auf jedem Gerät arbeiten: zum Bildschirm jedes verbundenen Geräts wechseln, direkt oder in den Gruppen, sehen, was es ausführt (Fenster-Videostream oder auf anderem Weg); sehr schnelle, sehr zuverlässige Stream-Kompression und -Dekompression | „2. operate on any device, i wanna be able to shift to the screen of any other connected device, directly or in the groups, and see aht they are running directly, be it with a window video stream or in other ways. for this we will need to aim for very fast very reliable stream compression and decompression“ |
| E6 | Einführung des se Desktop so nahtlos und kollisionsfrei wie möglich, übernimmt, was übernommen werden kann; Windows-Tiefe (Explorer-Kopplung) recherchieren; Android: vollständiger Launcher mit Widgets und allem, was ein Launcher braucht; Bearbeiten des Standard-Launchers prüfen | „for windows I really do not know how deep we can go with the desktop since it runs over the windows explorer … you will have to research this too like everything else“; „the introduction of the se-Desktop must be as seamless and colission free as possile, taking over what can be taken over“; „for android we are talking about a full on launcher you would be required to build … it still is required to operate as a normal android launcher, with widgets and everything a launcher requires., since android is open source you may be able to edit the default launcher“ |
| E7 | Launcher und Desktop getrennt hinzufügbar, aktivierbar und vollständig deaktivierbar; Open-Source-Bausteine wo anwendbar, Neubau wo schneller (Faustregel: bei substanziellen Änderungen neu bauen) | „you will be required to build these changes in a way where they can be seperatly added, activated and deactivated fully, the launcher and desktop stuff i mean. use open source components where applicable, do not use them when building new would be faster. an estimate is if you must di substential changes, build new“ |
| E8 | Cryptomator im Haus: Tresorsystem, mit dem Tresore geöffnet und bearbeitet werden | „the in house support of cryptomator … i want you to implement a vaults system where i can then open and edit vaults“ |
| E9 | Eigener Tresor-Sync-Job ohne Ordnereinrichtung, synchronisiert zuverlässig die neuesten Änderungen; bei Änderungen auf beiden Seiten keine automatische Konfliktauflösung, sondern ein Tresor-Vergleichsmodus nach dem Öffnen des Tresors | „another idea would be a inividual vault syncjob that does not require folder setup and syncs reliably the latest changes and on changes on both sides does not resolve conflicts automatically, but gives the user a vault comparison mode where he can after opening the vault compare the differences“ |
| E10 | Untersuchen, wie Cryptomator Tresore ins System einhängt, ob davon etwas „donkey“ ersetzen und Zuverlässigkeit bringen kann oder nicht; entscheiden, wie unter Linux Laufwerke eingehängt werden | „you can also be inspired by how cryptomator plugs it's vault into the system and see if any of these could replace donkey and add reliability or not and how you wanna do it for linux with plugging stuff in as drive“ |
| E11 | Zuerst planen, wie viele Hauptstränge nötig sind, jeden einzeln planen; nach der Planung direkt ausführen | „you will need to first plan how many main threads you will need to plan their stuff individually. execute directly once finished with planning“; „dig in and research and research more and plan and reresearch and plan and rerevalidate every component“ |

Eigene Folgerungen (nicht Nutzeraussage):
- „donkey“ bezeichnet nach dem Wortlaut das Windows-Laufwerks-Backend; Annahme bis zur Bestandslektüre: Dokany. (Bestand bestätigt: `native/src/mount/os/windows/dokany_abi.rs`.)
- „in the groups“: Annahme bis zur Bestandslektüre: die Share-Räume (Geräte­gruppen) von se.
- „operate on any device“ schließt nach dem Wort „operate“ Eingaben auf dem entfernten Gerät ein (Fernsteuerung), nicht nur Zuschauen. Ausdrückliche Annahme; Nutzerzitat nennt „operate“ und „see what they are running“.

### Grundfunktion aus dem Zweck

- E1/E2: Ein Mensch erlaubt einem Gerät Befehle/Terminal mit wenigen Entscheidungen (welches Gerät, als welches Konto, ob Rückfrage); ab dann gelten die Rechte dieses Kontos, gewöhnliche Befehle wie `ls` funktionieren ohne Einzelfreigabe. Eine Programmliste ist höchstens eine zusätzliche Einschränkung.
- E4/E5: Ohne se zu öffnen und ohne Verbindung von Hand herzustellen, erscheinen verbundene Geräte dort, wo der Nutzer auf seinem System arbeitet (Desktop/Startbildschirm/Dateimanager), als Ordner; ein Bildschirm eines Geräts ist mit einer Handlung erreichbar und bedienbar.
- E8/E9: Ein Tresor an beliebigem Ort (lokal oder auf jedem Remote-Backend) wird mit Passwort geöffnet, wie ein Ordner gelesen und geschrieben und gesperrt; ein Sync hält eine zweite Kopie aktuell und zeigt Konflikte zum Vergleich statt sie aufzulösen.

### Erweitern, nicht ersetzen

- Bestehende Exec-Grants (Konto, Programmliste, Protokoll), bestehende Laufwerke (Dokany), Sync-Jobs, Share-Geräte/Räume, Explorer-Fenster der App und Android-App-Bildschirme bleiben erhalten. Neue Fähigkeiten kommen hinzu; ein Ersatz (zum Beispiel Dokany → anderes Backend) nur mit belegtem Zuverlässigkeitsgewinn und Migration der gespeicherten Laufwerke.
- Der se Desktop ersetzt die Desktop-Umgebung bzw. den Launcher nur, wenn der Nutzer ihn aktiviert; Deaktivieren stellt den vorherigen Zustand vollständig wieder her.

### Alle Oberflächen

Jede neue oder geänderte Fähigkeit auf CLI (Text, JSON, Vervollständigung), Desktop-GUI, Android-GUI, Android-Bridge, Daemon-IPC, MCP, Netzprotokoll und Doku; Inventur `docs/oberflaechen/` (neues Teildokument je Bereich).

### Grenzen und Autorisierung

- Umsetzung autorisiert („execute directly once finished with planning“); kein Planfreigabe-Haltepunkt.
- AGENTS.md: keine lokalen Builds/Tests; eine Remote-Task-Suite; ein Release am Ende des Stapels; Subagenten höchstens 7 gleichzeitig, keine Unter-Subagenten, Aufträge mit genauer Dateiliste.
- Nutzergeräte-Betrieb nie im Repo (Memory „device-ops-not-in-repo“).

### Fehlende Angaben

- Durch Bestandslektüre klärbar: heutiges Exec-Rechtemodell und seine Standards (blockiert `ls`?); Kontowechsel je OS; Mount-Backend und seine Grenzen; Gerätemodell (Kontakte, Räume, automatisches Verbinden); Android-App-Aufbau; Release-/Feed-Aufbau für zusätzliche Komponenten; Sync-Konfliktbehandlung; vorhandene Cryptomator-Befunde (vorhanden: `docs/CRYPTOMATOR_VAULTS_ERKUNDUNG.md`, `docs/refs/cryptomator-*.md`).
- Nur vom Nutzer klärbar: keine Angabe verhindert den Beginn.

### Tiefe und nächste Handlung

Tiefe 4 (neue Projekte und Datenwege über Modulgrenzen). Wissensgebiete: OS-Kontowechsel (Windows, Linux, Android), Bildschirmaufnahme/Video-Codecs/Transport/Eingabe, Shell-Integration (Explorer, Linux-Desktops, Android-Launcher), Cryptomator-Format 8, Dateisystem-Frontends (Dokany, WinFsp, Cloud Files, FUSE, DocumentsProvider), Sync-Konflikte.

## Hauptstränge (Ergebnis von E11, Begründung in B)

| Kürzel | Strang | Ergebnisse | Datei |
|---|---|---|---|
| S1 | Exec-Rechte nutzergebunden, Kontowechsel je OS | E1, E2, E3 | `strang-rechte.md` |
| S2 | Laufwerke und Einhängen je OS (Windows-Backend-Entscheid, Linux-Laufwerke, Android-Dokumentanbieter) | E10 und Grundlage für E4, E8 | `strang-laufwerke.md` |
| S3 | Cryptomator-Tresore (Format, Tresor-Backend, Verwaltung, Tresor-Sync, Vergleichsmodus) | E8, E9 | `strang-tresore.md` |
| S4 | Bildschirm-Streaming und Fernsteuerung (Aufnahme, Kodierung, Transport, Darstellung, Eingabe) | E5 | `strang-bildschirm.md` |
| S5 | se Desktop (Windows, Linux) und se Launcher (Android), modular aktivierbar | E4, E6, E7 | `strang-desktop.md` |

## Arbeitsweise dieses Stapels (Nutzervorgaben 2026-10-17)

- Kein Workflow-Werkzeug; nur einzelne Agent-Aufrufe, höchstens 7 gleichzeitig (AGENTS.md). Nutzer: „you are also not allowed to use workflows from now on“.
- Modell je Rolle im Modellparameter jedes Aufrufs (Nutzer: „does your arbeitsweise not establish wich models to use for what?“, „and you still ran all as opus …“; Arbeitsweise zentral Rev. 277 und 279; jeder Auftrag an haiku/sonnet verbietet das Opus-Werkzeug `advisor`; Modell wird am Transkript belegt): Bestandslektüre ohne Kreativität `haiku`; reine Faktenrecherche mit Quellen `sonnet`; Entwurf, Bewertung, Planung, Umsetzung, Integration, Autorprüfung Hauptmodell (`opus`).
- Ab 14:28 (Nutzer: „group your further tasks and preferably use for larger research batches then opus medium effort agents“, „opus max, or your current default, is only allowed on implementation“; zentral Rev. 280/282): Aufgaben je Strang gebündelt — ein Bestandsauftrag (haiku) und ein Rechercheauftrag (opus, Effort medium) je Strang; Entwurf und Strangplan je Strang auf opus mit Effort xhigh (Nutzer 14:30: „planning at xhigh, design too.“; zentral Rev. 284), Nachrecherche und Revalidierung als Recherchebündel auf opus mit Effort medium; höchster bzw. Standardaufwand nur für Umsetzungsagenten. Die bis 14:20 gestarteten Einzelrecherchen auf sonnet laufen zu Ende (ihre Arbeit wird weiterverwendet).
- Gemessener Verbrauch bis 14:24 (Transkripte, Listenpreise als Näherung): Sonnet-Rechercheure ≈ 61 %, Hauptsitzung (Opus) ≈ 18 %, gestoppte Opus-Workflow-Agenten ≈ 14 %, Haiku-Leser ≈ 7 %. Ursache: sehr lange Verläufe der Rechercheure (Mittel 200–435 Tausend Token je Anfrage), die bei jeder Anfrage erneut gelesen werden; der Cache-Lesepreis von Sonnet 5.5 entspricht dem von Opus 5.5 (0,20 $ je Million).
- Zwei frühere Planungs-Workflows (S1, S2) wurden am 2026-10-17 um 13:55 durch die Nutzerunterbrechung beendet; ihre vier Leseagenten hatten keine Datei geschrieben. Die Arbeit ist vollständig durch die Agenten unten ersetzt.

## Fortschritt der Agenten

Modellbeleg (Transkripte, 2026-10-17 14:20): alle Bestandsleser antworten ausschließlich mit `claude-haiku-5-5`, alle Faktenrechercheure ausschließlich mit `claude-sonnet-5-5`; Aufrufe des Opus-Werkzeugs `advisor`: 0. Die Anzeige „opus“ im Terminal beim Zurückscrollen ist laut Nutzer eine Fehlanzeige.

| Strang | Rolle | Modell | Ausgabedatei | Stand |
|---|---|---|---|---|
| S1 | Bestand Exec-Host | haiku | `docs/lesungen/2026-10-17-exec-rechtemodell-host.md` | Datei geschrieben (332 Zeilen); danach vom Nutzer im Agentenfenster fortgesetzt |
| S1 | Fakten Rechtemodelle vergleichbarer Produkte | sonnet | `docs/refs/fernzugriff-rechtemodelle-2026-10-17.md` | Datei geschrieben (564 Zeilen) |
| S1 | Bestand Exec-Oberflächen | haiku | `docs/lesungen/2026-10-17-exec-rechte-oberflaechen.md` | Datei geschrieben (620 Zeilen) |
| S1 | Fakten Windows-Kontowechsel | sonnet | `docs/refs/exec-konto-windows-2026-10-17.md` | Datei geschrieben |
| S1 | Fakten Linux-Kontowechsel | sonnet | `docs/refs/exec-konto-linux-2026-10-17.md` | Datei geschrieben (663 Zeilen, 362 Zitate geprüft) |
| S1 | Entwurf und Strangplan | opus, Effort xhigh | `strang-rechte.md` | Entwurf fertig (11 Meilensteine PRM-M1–M11, Nachrecherche N1–N24 + 6 Revalidierungen) |
| S1 | Nachrecherche N1–N24 und Revalidierung | opus, Effort medium | `nachrecherche-rechte.md` | fertig (u. a. Windows-Job-Einhegung, „Nur lesen“, PAMName-Einhegung widerlegt; bestehender Defekt ExitType=main auf systemd 249) |
| S1 | Endfassung Strangplan (Planverfasser fortgesetzt) | opus, Effort xhigh | `strang-rechte.md` | fertig (11 Meilensteine PRM-M1–M11, Schleifenstatus „erreicht (Planungsteil)“) |
| S1 | Fakten Android-Kontexte | sonnet | `docs/refs/exec-konto-android-2026-10-17.md` | Datei geschrieben (480 Zeilen) |
| S2 | Bestand Mount-Kern | haiku | `docs/lesungen/2026-10-17-laufwerke-kern.md` | Datei geschrieben (509 Zeilen) |
| S2 | Fakten Linux-FUSE und Dateimanager-Einbindung | sonnet | `docs/refs/linux-fuse-laufwerke-2026-10-17.md` | Datei geschrieben (503 Zeilen) |
| S2 | Recherchebündel Android-DocumentsProvider + Cryptomator-Einhängen (Revalidierung) | opus, Effort medium | `docs/refs/android-documents-provider-2026-10-17.md`, `docs/refs/cryptomator-einhaengen-revalidierung-2026-10-17.md` | beide Dateien geschrieben |
| S4 | Bestand Transport/Rechte/Darstellung/Plattform | haiku | `docs/lesungen/2026-10-17-bildschirm-bestand.md` | Datei geschrieben (396 Zeilen) |
| S4 | Entwurf und Strangplan | opus, Effort xhigh | `strang-bildschirm.md` | Entwurf fertig (12 Meilensteine SCR-M1–M12, Nachrecherche N1–N25) |
| S4 | Nachrecherche N1–N25 und Revalidierung Abschnitt C | opus, Effort medium | `nachrecherche-bildschirm.md` | fertig (9 widerlegt, 11 nicht belegbar, 14 Entscheidungen mit geänderter Grundlage) |
| S4 | Endfassung Strangplan (Planverfasser fortgesetzt) | opus, Effort xhigh | `strang-bildschirm.md` | fertig (12 Meilensteine SCR-M1–M12, Status offen: K6 Media Foundation/GPU auf GitHub-Runnern nicht prüfbar, K3 KWin kopflos unbelegt) |
| S5 | Bestand Android-App, Gerätemodell, Gerät-als-Ordner-Bausteine | haiku | `docs/lesungen/2026-10-17-android-geraete-bestand.md` | Datei geschrieben (666 Zeilen) |
| S5 | Entwurf und Strangplan | opus, Effort xhigh | `strang-desktop.md` | Entwurf fertig (15 Meilensteine DSK-M1–M15, Nachrecherche DN1–DN30) |
| S5 | Nachrecherche DN1–DN30 und Revalidierung | opus, Effort medium | `nachrecherche-desktop.md` | fertig (R1–R80 geprüft; widerlegt u. a. R6 winit hält die unterste Lage nicht, R35 `set_type` erst ab GNOME 49, R70 Updater beendet keine Prozesse) |
| S5 | Endfassung Strangplan (frischer Planverfasser) | opus, Effort xhigh | `strang-desktop.md` | fertig (15 Meilensteine DSK-M1–M15, Schleifenstatus „erreicht (Planungsteil)“; Modellbeleg: alle Antworten `claude-opus-5-5`, Effort xhigh) |
| alle | Integrationsplan (Abschnitte B, C, D) | opus, Effort xhigh | `plan.md` | fertig (Verträge V1–V14, Widersprüche W1–W14, Pakete U01–U43 und R1–R14 in 15 Phasen, eine Suite `rechte-desktop-tresore-task.yml`); Modellbeleg: Antworten `claude-opus-5-5`, Effort xhigh |
| S5 | Recherchebündel Windows-Tiefe, Linux-Desktop, Android-Launcher, modulare Auslieferung | opus, Effort medium | `docs/refs/se-desktop-launcher-2026-10-17.md` | Datei geschrieben |
| S4 | Recherchebündel Aufnahme, Codecs, Transport, Eingabe, Referenzprojekte | opus, Effort medium | `docs/refs/bildschirm-streaming-2026-10-17.md` | Datei geschrieben (345 Zeilen) |
| S5 | Bestand Desktop-App, Windows-/Linux-Integration, Installer, Release, Feed, Suite-Muster | haiku | `docs/lesungen/2026-10-17-desktop-integration-bestand.md` | Datei geschrieben (513 Zeilen) |
| S3 | Bestand Tresor-Schicht, Klartextgrenze, Geheimnisse, Sync-Konflikte | haiku | `docs/lesungen/2026-10-17-tresore-bestand.md` | Datei geschrieben (532 Zeilen) |
| S3 | Entwurf und Strangplan | opus, Effort xhigh | `strang-tresore.md` | Entwurf fertig (14 Meilensteine VLT-M1–M14, Nachrecherche G1–G26) |
| S3 | Nachrecherche G1–G26 und Revalidierung | opus, Effort medium | `nachrecherche-tresore.md` | fertig (59 Aussagen geprüft; Wortliste Wiederherstellungsschlüssel ohne freie Lizenz) |
| S3 | Endfassung Strangplan (Planverfasser fortgesetzt) | opus, Effort xhigh | `strang-tresore.md` | fertig (14 Meilensteine VLT-M1–M14, P1–P3 ohne Gegengrund bestätigt; Schleife offen nur wegen K13, zwei unbelegte Plattformannahmen zur Bildschirmsperre) |
| S3 | Recherchebündel Format 8, Krypto-Crates, Sync-/Konflikt-Bedienung | opus, Effort medium | `docs/refs/cryptomator-format-sync-2026-10-17.md` | Datei geschrieben (361 Zeilen) |
| S2 | Bestand Windows/Dokany | haiku | `docs/lesungen/2026-10-17-laufwerke-windows-dokany.md` | Datei geschrieben (374 Zeilen) |
| S2 | Fakten Windows-Dateisystem-Frontends (Dokany, WinFsp, Cloud Files, ProjFS, WebDAV) | sonnet | `docs/refs/windows-dateisystem-frontends-2026-10-17.md` | Datei geschrieben (613 Zeilen) |
| S2 | Entwurf und Strangplan | opus, Effort xhigh | `strang-laufwerke.md` | Entwurf fertig (10 Meilensteine DRV-M1–M10, Nachrecherche N1–N27) |
| S2 | Nachrecherche N1–N27 und Revalidierung | opus, Effort medium | `nachrecherche-laufwerke.md` | fertig (16 geänderte Grundlagen; Bestandsfehler ERROR_INVALID_DATA → „Zugriff verweigert“) |
| S2 | Endfassung Strangplan (Planverfasser fortgesetzt) | opus, Effort xhigh | `strang-laufwerke.md` | fertig (10 Meilensteine DRV-M1–M10; K7 vom Hauptagenten entschieden, P1–P3 an S3 übergeben) |
| alle | Umsetzung Phase P1: U01 (S1-Kern), U03 (S5-Fundament), U02 (S2-Kern), dann R1 | opus, Standardaufwand der Sitzung (max) | Code laut D1 | fertig und gepusht: U03 5fc31eeb, U01 888af678, R1 2e3d07b1, U02 f45d9580 (`host_lifecycle.rs` verwaist und entfernt). R1 Teil B2 entfiel: U02 schrieb `drive_task_types_split_serde_unchanged`, der Hauptagent prüfte die `PathDialect`-Namen gegen die R1-Fixtures. Graphify-Aktualisierung gesammelt am Ende der Umsetzung, damit kein halbfertiger Stand laufender Pakete in den Graphen gelangt. |
| S2 | Umsetzung U06 (DRV-M3, dazu Exit 75 in `mount_manager_state.rs` aus U02) | opus, Standardaufwand der Sitzung (max) | Code laut D1 | fertig (Commit a6107a70) |
| alle | Registrierung R2, Teil Laufwerke: U06-Bedarf, `match`-Zweige für `MountedAt`/`Devices`/`Vault`, `ShareSnapshotPeek` | opus, Standardaufwand der Sitzung (max) | Hub-Dateien und vier Bestandsdateien | fertig (Commit 91d70957) |
| alle | Umsetzung U39 (Suite Teil A, vorgezogen) | opus, Standardaufwand der Sitzung (max) | Workflow, `native/rdt_task/**` | fertig (Commit b8816cdb; 15 Jobs, Inventur `expected_signals.txt` mit 189 vorhandenen und 177 kommenden Signalen) |
| S2 | Umsetzung U12 (DRV-M4 Linux-FUSE) | opus, Standardaufwand der Sitzung (max) | Code laut D1 | fertig (Commit fc59b6f3) |
| S3 | Umsetzung U11 (VLT-M4, VLT-M5) | opus, Standardaufwand der Sitzung (max) | Code laut D1 | fertig (Commit ac21926f) |
| alle | Registrierung R4a für U11 (IPC `Vault`, `Err.code`, Fehlerumschlag, Inventur) | opus, Standardaufwand der Sitzung (max) | Hub-Dateien und `Err`-Stellen | fertig (Commit 55d0fb13) |
| S4 | Umsetzung U10 (SCR-M2) | opus, Standardaufwand der Sitzung (max) | Code laut D1 | fertig (Commit c4b2583b; Stand `stand/U10.md`) |
| alle | Registrierung R5x für U10, U12, U24, U30 samt Mitbehebungen (Dokany-Hinweise unter Linux, Registry-Vorbereitung unter Linux, fest gesetzte `graphical_session`) | opus, Standardaufwand der Sitzung (max) | Hub-Dateien und genannte Dateien | fertig (Commit 9456b8ad; doppelter Testname `drive_task_auto_remount_after_daemon_restart` durch `_fuse` aufgelöst, `se drive`-Hilfe nennt Linux) |
| alle | Graphify-Aktualisierung | Hauptagent | `graphify-out/` | fertig (Commit 6370b5c6: 44 952 Knoten, 118 351 Kanten, 1 492 Gemeinschaften) |
| S1 | Umsetzung U24 (PRM-M7 und Konten-Oberflächen, Übergaben aus U04, U07, U18) | opus, Standardaufwand der Sitzung (max) | Code laut D1 | fertig (Commit b7068a61; Stand `stand/U24.md`) |
| S5 | Umsetzung U30 (DSK-M4) | opus, Standardaufwand der Sitzung (max) | Code laut D1 | fertig (Commit 546b5bb7; Wächter-Patch vom Hauptagenten angewendet; Bestandsfehler `xdg-open` mit geerbter Standardein-/ausgabe und Zombie-Prozess behoben in 0a7bb9c0) |
| alle | Vorgezogene Pakete nach Nutzervorgabe 2026-10-17 (Limit erlaubt mehr Parallelität): U04, U09, U15, U33, U18 | opus, Standardaufwand der Sitzung (max) | Code laut D1 | U33 fertig (5367b5ab), U09 fertig (55efff3e), U04 fertig (e2271305), U15 fertig (45919dcf), U18 fertig (2e4c1c00; `exec_job.rs` ruft jetzt `prepare_as` mit dem Konto des Rechts, gesetzt vom Hauptagenten); Dateibereiche ohne Überschneidung geprüft |
| S1 | Umsetzung U07 (PRM-M8, PRM-M9 ohne Konten, Übergaben aus U01/U04, Öffnen-Funktion für Detailseiten) | opus, Standardaufwand der Sitzung (max) | Code laut D1 | fertig (Commit 1a6706b1) |
| S1 | Umsetzung U21 (PRM-M10 ohne Bildschirm-Aktionsziele, Bestandsfehler `share.exec` auf Android) | opus, Standardaufwand der Sitzung (max) | Code laut D1 | fertig (Commit 07914285) |
| alle | Vorgezogene Registrierung für U04, U05, U09, U15, U18 und U33 | opus, Standardaufwand der Sitzung (max) | Hub-Dateien | fertig (Commit 5ec3531b, gepusht) |
| S3 | Umsetzung U05 (Phase P2, vorgezogen; keine Dateiüberschneidung mit U02 und R1) | opus, Standardaufwand der Sitzung (max) | Code laut D1 | fertig (Commit 4e07b200); Quellen von aes-siv 0.8.0, aes-kw 0.3.1, siv-mode-Prüfvektoren (MIT) und zwei CryptoLib-2.3.0-Testdateien aus dem Netz gelesen, vom Hauptagenten als nötig für C1.2 bestätigt |
| S3 | Umsetzung U08 (VLT-M2, VLT-M3) | opus, Standardaufwand der Sitzung (max) | Code laut D1 | fertig (Commit e1f1c819); Abweichung von C2.6: Stufen liegen immer im Inhaltsverzeichnis des Elternordners (deterministische Abbildung), vom Hauptagenten angenommen |

Wochenlimit (Nutzer 2026-10-17, berichtigt 2026-10-18): Vorerst gilt eine Obergrenze für das Wochenlimit. Sie ist eine Startgrenze: Bis zu ihrem Erreichen starten weitere Pakete, ohne Vorabstopp wegen einer Hochrechnung. Ab Erreichen startet nichts Neues; ein dann laufender Agent wird nicht abgebrochen. Vor der harten Grenze des Kontos erhalten laufende Agenten einen Dokumentationsauftrag (`stand/<Paket>.md`), weil ein Abbruch durch das Konto Verlauf kosten kann.

Sitzungslimit (Nutzer 2026-10-17): Ab der Nachricht zum nahenden Sitzungslimit laufen höchstens zwei Agenten gleichzeitig; jeder weitere Schritt startet erst, wenn ein laufender Agent fertig ist. Der Hauptagent fragt den Nutzungsstand alle 5 Minuten ab und setzt nach der Rücksetzung des Fensters einen Weckauftrag, falls die Sitzung am Limit abbricht.

**Stand bei der Pause (2026-10-18 01:05 UTC, Obergrenze des Wochenlimits nach Nutzervorgabe erreicht):**
- **Fertig, committet und gepusht** (bis 6370b5c6):
  - 19 Umsetzungspakete: U01–U12, U15, U18, U21, U24, U30, U33, U39;
  - alle Registrierungen dazu;
  - der Graphify-Graph.
- **Offen, 24 Pakete:**
  - S2: U14, U17, U23;
  - S3: U20, U26, U29, U31;
  - S4: U13, U16, U19, U22, U25, U27, U28;
  - S5: U32, U34, U35, U37, U38, U40;
  - Integration: U36, U41, U42, U43.

  Danach folgen die Registrierungen, die eine Remote-Suite (`rechte-desktop-tresore-task.yml`) mit Fehlerkorrektur und die eine Release-Transaktion.
- **Nächste Reihenfolge nach der Wochenrücksetzung:**
  1. U13, U14, U17 und U20 parallel, weil sie ohne gemeinsame Dateien sind;
  2. danach U16, U19, U26, U29, U32 und U34;
  3. dann U22, U23, U25, U27, U28, U31, U35, U37, U38 und U40;
  4. zuletzt U36, U41, U42 und U43;
  5. dann die Suite.

**Wiederaufnahme (2026-10-18 07:33 UTC, nach der Berichtigung der Obergrenze durch den Nutzer):**
- Laufend: U13, U14, U20 und U32 (opus, Standardaufwand, Modell aus den Transkripten belegt). Die Dateibereiche stehen im Arbeitsverzeichnis des Hauptagenten (`running-areas.md`); jeder Agent führt `stand/<Paket>.md` fortlaufend.
- U17 wartet auf U14: gemeinsame Dateien unter `mount/` und `daemon/os/shared/mount_*` sowie Zwangsreihenfolge W3 (`PrepareLock` vor `ChildBackend`/`DeviceWatch`). U32 ersetzt in der Startreihenfolge U17.
- Danach startbar nach den Strangabhängigkeiten: U16 (nach U13), U17 (nach U14), U26 (nach U20), U34 (nach U17 und U32), U27, U35 und U37 (Abhängigkeiten erfüllt; U27 und U35 teilen die Android-Hub-Dateien aus D2.3 und laufen deshalb nicht gleichzeitig).

## Entscheidungen des Hauptagenten zu offenen Strangpunkten

**Statusbefehle gegen Desktop-Ansicht (aus U06):** `se share status`, `se share discoverable` und die Vorprüfung in `se exec` heben die Einschränkung eines Anmelde-Dienstes auf, die passive Ansicht des se Desktop nicht.
- Dafür gibt es die neue IPC-Variante `ShareSnapshotPeek`, die nur `share_worker_peek` sendet.
- `ShareSnapshot` bleibt auf der Leitung unverändert und gilt jetzt als Auslöser.
- Ein älterer Dienst kennt die neue Variante nicht; der se Desktop zeigt dann „unbekannt“. Das ist akzeptiert, weil App und Dienst aus demselben Release stammen.

**S2 K7 (Erreichbarkeit ab Anmeldung):** Ein Daemon, der nur für die Laufwerkswiederherstellung beim Anmelden startet (Startgrund `drive-restore`), startet weder den Share-Host noch den Exec-Host. Beide starten wie bisher beim ersten anderen Auslöser: App, CLI, MCP oder eingeschalteter Autostart. Damit schafft DRV-M6 keine neue Erreichbarkeit ab Anmeldung und hängt nicht an S1. Ob ein eingeschalteter se Desktop (S5) den Share-Host ab Anmeldung startet, legt der Integrationsplan fest.

**S2/S3-Schnittstelle P1–P3:** von S3 ohne Gegengrund bestätigt (`strang-tresore.md`, Abschnitt „Bestätigungen an S2“).
- P1: Die Journal- und Stufen-Pfadfelder des Laufwerkshosts werden mit einem aus dem Spool-Schlüssel abgeleiteten Textschlüssel versiegelt, weil der Laufwerkshost nur den Spool-Schlüssel erhält.
- P2: Unter Windows gibt es keine besondere Laufwerksart gegen Windows Search, weil Netzwerk- und Wechselmodus in S2 C1 verworfen sind. Die Annahme, dass Windows Search zusätzliche Laufwerke nicht von selbst indiziert, bleibt ausdrücklich offen.
- P3: Tresore werden über `se drive mount vault://<SITE>` eingebunden, also über die gemeinsame Orts- und Auflösungsgrenze, nicht über `se drive add --vault`.

**Lizenzhinweise:** `fuser 0.18.0` (MIT) gehört in den Integrationsmeilenstein der Rust-Crate-Lizenzhinweise.

**S5 H1 (Share-Host ab Anmeldung bei eingeschaltetem se Desktop):** Variante V1. Der se Desktop startet den Share-Host nie selbst. Ein anderes Gerät zu bedienen braucht den eigenen Worker nur im Moment der Handlung. So bleibt die Regel aus S2 K7 gewahrt: Kein Anmeldestart schafft neue Erreichbarkeit. Die Punkte H2–H10 entscheidet der Integrationsplan.

**S3 OE2 (Linux-Einhängeort der Tresore):** erledigt durch die S2-Endfassung. Tresor-Laufwerke liegen unter `$XDG_RUNTIME_DIR/smart-explorer/Tresore/<Name>`; ohne tmpfs bricht se ab. Übrige Laufwerke bleiben unter `~/SmartExplorer/`.

**S3 OE3:** Das Feld heißt `MountSource::Vault { site_id }`, weil der Inhalt die S3-Site-ID ist.

**S3 OE4:** Die Anwendungsphasen in `apply_reporting.rs` (`PlanRules::phases`, Standard eine Phase) ändern einen Engine-Pfad aller Jobarten. Das Paket kommt früh in die Integrationsreihenfolge. Die Suite deckt den Ein-Phasen-Standard der bestehenden Jobarten als Erhaltungsfall ab.

**S3 OE5:** Die sieben neuen Pakete des Bildvergleichs (`zune-jpeg`, `zune-core`, `gif`, `weezl`, `color_quant`, `image-webp`, `quick-error`) gehören in den Integrationsmeilenstein der Lizenzhinweise. Dort werden sie mit den vier `image`-Paketen aus der S5-Nachrecherche (DN22) abgeglichen.

**S3 OE1 (Nutzerentscheidung):** Für die Wortliste des Wiederherstellungsschlüssels gibt es drei Wege: Laden zur Laufzeit von Cryptomators GitHub mit festem SHA-256-Pin, Erlaubnis von Skymatic oder dauerhaftes Weglassen. Bis zur Entscheidung entfällt die Funktion im Stapel; alle Oberflächen zeigen einen Informationstext ohne Bedienelement.

- S4 K6 (Windows Media Foundation, GPU-Kodierer): Die eine Suite versucht auf dem Windows-Runner `Server-Media-Foundation` ohne Neustart zu aktivieren; gelingt das nicht, bleibt der Hauptweg als ungeprüft gemeldet (AGENTS.md: reale Tests sind keine Release-Bedingung, ungeprüftes Verhalten wird ehrlich benannt). Kein zusätzlicher Client-Runner.
- S4 K3 (Wayland-Steuern): Die Suite versucht KWin kopflos mit `xdg-desktop-portal-kde`; startet es nicht, wird die Prüfung als nicht ausgeführt gemeldet.
- Lizenzhinweise aller Rust-Crates (S4 `RUST-CRATE-LICENSES`): Das Release verteilt heute keine Lizenztexte der statisch gelinkten Crates. Weil dieser Stapel neue Crates mit Hinweispflicht hinzufügt und der Release-Weg betroffen ist, wird ein Integrationsmeilenstein eingeplant, der die Hinweise aller Crates erzeugt und ausliefert.
- S1 „Nur lesen“ im eigenen Konto: entfällt in diesem Stapel (Landlock/Low lassen den Weg über die se-IPC offen, Nachrecherche S1). Eingeschränkter Zugriff läuft über das Konto `se-remote` oder ein benanntes Konto — das erfüllt den Nutzerwortlaut „permissions can be handled on the actual operating system“; „Nur lesen“ war eine eigene Beispielidee des Hauptagenten, kein Nutzerwortlaut. Bedingungen für später in docs/TODO.md.
- S1 CI-Umgebungen: Suite auf GitHub-Runnern `ubuntu-24.04` (root über sudo), `windows-2025` (Administrator), zusätzlich `ubuntu-22.04` für den `ExitType`-Fix, sofern das Image angeboten wird; Fedora-SELinux bleibt ungeprüft und wird so berichtet.
- S1 Release-Text: nennt, dass Linux-Exec auf Ubuntu 22.04 bis zu diesem Stapel am Bestandsdefekt `ExitType=main` scheiterte, und dass der Linux-root-Worker abgelaufene Konten und erzwungenen Passwortwechsel jetzt ablehnt.
- Eine einzige Task-Suite für den Stapel (AGENTS.md): Die in S2, S4 und S5 geplanten eigenen Suite-Workflows werden im Integrationsplan zu einem Workflow mit Stufen zusammengeführt.

### Übergaben aus fertigen Paketen an spätere Pakete

Die Briefe der genannten Pakete enthalten diese Punkte zusätzlich zu D1.

- **An U04 (aus U01 B und E):**
  - Die Rückfrage- und Zugriffsmeldungen aus `ExecClientSession.notice` (`exec_session.rs`) gehen über `ipc_host_ext.rs` nach außen (W10, `exec_ipc.rs` bleibt unverändert). Sie werden ausgewertet in:
    - `cli/shell_session/transports.rs` und `cli/shell_session/terminal_transports.rs`: Statuszeile und Kopfzeile;
    - `cli/exec.rs`: JSON-Ereignis `approval_pending`;
    - `cli/mcp/exec_share_job.rs`: Zustand `waiting_for_approval`.
  - Texte und Funktionen liegen in `exec_client_messages.rs`.
  - Nach „Immer erlauben“ ruft der Host `start_grant` erneut auf.
  - Das Fenster `--access-consent` ruft `app::update_stop_ui::arm_window()` beim Start und `close_requested(ctx)` in jedem Bild.
- **An U07 (aus U01 Entscheidung 12):** Der C6-Hilfetext von `se exec` ändert sich zusammen mit der Prüfung in `cli/parse_tests.rs:112`.
- **An U21 (aus U01 C, Bestandsfehler):**
  - Die Android-Brücke führt `share.exec` über das alte `Ctrl::Exec`. Der Host lehnt das immer ab (`PreparedExec::run`, `share/core/exec.rs:95-98`). U21 stellt Android auf den Stream-Weg um; damit funktionieren Rückfrage und Kopfzeile.
  - Die `write`-Regel steht doppelt, in `device_view::granted_write` und in `share_policy_status.rs::contact_write` (aus U03). Wo U21 `share_policy_status.rs` ändert, legt es beide zusammen.
- **An die S4-Pakete U09, U13, U16, U19, U25 und U27 (aus U01 D):**
  - `direct_relation.rs:238/308/364` und `room_relation_members.rs:124/145` prüfen `exec.enabled` als „irgendein Recht“; für reine Bildschirmrechte gehört dort `grants_any()` hin.
  - Eine Verengung erreicht die Bildschirm-Registry über `ExecGrant::narrowed_against`.
  - Der zentrale Ablauf-Zeitgeber für Bildschirmsitzungen nutzt `expired_restrictions` und `next_expiry`.
- **An U10 und U25 (aus U03 offener Punkt 6):** Betrachter- und Indikatorfenster rufen `arm_window()` und `close_requested()`.
- **An U17 (aus U03 offener Punkt 3):** `desktop::core::tiles::device_key` bildet `mount::device_key` (V4) nach, unter der Annahme Großbuchstaben-Hex für `%XX`. U17 gleicht das Format an oder ersetzt die Funktion durch den Aufruf.
- **An U25 (aus U03 offener Punkt 4):**
  - Bildschirmziel bei Direktgeräten: `device_id` = `remote_device_id`; fehlt sie, gibt es kein Ziel.
  - Bei Räumen: `relation_id` = lokale Raumprofil-ID wie in `PeerOpenTarget`.
- **An U12 (aus U02 offener Punkt 4):** `daemon/os/shared/rooted_backend*.rs` wendet Windows-Namensregeln auf jede Wurzel und Auflistung an; Linux-Mounts brauchen den Dialekt (`peer_names::*_in` stehen bereit). Die Linux-Module `cache_lease.rs` und `cache_space.rs` stehen vorläufig inline in `mount/os/mod.rs`; U12 verlegt sie nach `linux_os/mod.rs`.
- **An U17 (aus U02 offener Punkt 3):** `mount/os/windows/handle_state.rs` und `handle_reservation.rs` verlangen `&MountEngine`; für den Gerätebaum brauchen sie `&dyn MountFilesystem`. Beide Dateien gehören zu U17.
- **An U14 (aus U02 offener Punkt 5):** Die `detail`-Freitexte im Journal sind nicht versiegelt; DRV-M7 entscheidet.
- **An U39 (aus U02):** Zusätzliche Tests in die erwartete Liste: `drive_task_path_sealer_reaches_every_journal_path_field`, `drive_task_host_session_update_stop_keeps_unpublished_changes`, `drive_task_change_pump_maps_backend_paths_per_dialect`. Die drei `#[ignore]`-Tests mit echtem Dokany laufen in `windows-core: drive-dokany-regression` mit `--include-ignored --exact`.
- **An R2 (aus U05):**
  - `native/src/lib.rs`: nach `pub mod creds;` die Zeile `pub(crate) mod cryptomator;`, ohne `cfg`.
  - `native/Cargo.toml` `[dependencies]`: nach `icu_normalizer` die Zeilen für `aes-siv 0.8.0` (`alloc`, `zeroize`), `aes-kw 0.3.1`, `aes-gcm-011`, `aes-09`, `ctr-010`, `hmac-013`, `sha2-011`, `sha1-011`, `scrypt 0.12.0` und `data-encoding 2.11.0`, jeweils mit `default-features = false`, wörtlich aus dem U05-Bericht.
  - Neu im Lock sind nur `aes-siv` und `aes-kw` (beide MIT/Apache-2.0); die übrigen gibt es schon im Lock.
- **An U20 (aus U05):**
  - `vault_task_sync_phases_content_before_node` steht schon in U05 (`bisync/os/shared/apply_phases_tests.rs`); U20 legt ihn nicht erneut an.
  - Das Einzelanwenden aus der Vorschau (`apply_preview_action`) umgeht die Phasen; U20 sperrt es für Tresor-Jobs oder prüft die Reihenfolge.
  - Der inkrementelle Weg umgeht `adjust`; R4a bleibt nötig.
  - Planregeln wirken nur auf dem Thread, der `bisync::plan_rules::enter` aufgerufen hat.
- **An U39 (aus U05):** Die Testpfade lauten `cryptomator::core::{vectors_tests,config_tests,names_tests,content_tests}::…` und `bisync::apply_reporting::phases_tests::…`. Der Filter `bisync::core::plan_rules` aus VLT-M14 trifft nicht.
- **An U42 (aus U05):** In `docs/GOTCHAS.md` steht als bekannte Grenze: scrypt-Arbeitspuffer, CMAC-Teilschlüssel in aes-siv und entschlüsselte Namen als `String` werden nicht überschrieben.
- **An R11 (aus U33):**
  - `native/src/lib.rs`: `mod licenses;`.
  - `native/src/cli/mod.rs`: `mod licenses;` und der Unterbefehl `Licenses` mit `--json`, der `licenses::run(json)` aufruft.
  - `native/src/mobile/os/shared/domains/mod.rs`: `app_licenses` und das Präfix `"app" => app_licenses::handle(method)`.
  - `native/src/app/mod.rs`: `licenses_ui`.
  - Der Knopf kommt in `native/src/app/core/settings_updates.rs` nach der Versionsanzeige: `super::licenses_ui::licenses_button(ui);`.
  - `show_licenses_window(ctx)` steht in `settings_ui.rs::ui_settings`.
  - `license = "MIT"` und `publish = false` in `native/Cargo.toml`, `native/android-bridge/Cargo.toml`, `native/explorer-command/Cargo.toml` und `share-server/Cargo.toml`.
  - Optional `.gitattributes` mit `eol=lf` für die Lizenzdatei.
- **An U39 (aus U33):**
  - Reihenfolge im Job `prepare`: `lock` → `audit` → `licenses` → `static`.
  - Aufruf: `python native/licenses_task_checks.py --log-root "$RUNNER_TEMP/rdt-prepare/licenses" --place-generated`; installiert cargo-about 0.9.2 mit `--features cli`.
  - Die erzeugte Datei kommt ins Artefakt `rdt-prepare`, und jeder spätere Job legt sie vor dem Bau in seinen Checkout.
  - `native/rdt_task/licenses.py` ruft dieses Skript auf, statt es nachzubauen.
- **An U36 (aus U33):** Der Windows-Installer legt `licenses\Rust-Crates\THIRD-PARTY-LICENSES.txt` ab (`licenses_task_installer_installs_file`).
- **An U07 (aus U04):**
  - CLI `se share exec approvals`, `approve` und `deny`, die Liste im Tab „Befehle“ und die MCP-Sperrliste.
  - Behebung der doppelten Rückfrage bei `se shell -T`, wenn „Befehle“ auf „Fragen“ steht: Probe und Shell fragen heute je einmal. Die Probe löst keine Rückfrage aus; Dateien `cli/shell_session/transports.rs` und `terminal_transports.rs`.
  - Der Aufruf `grant_expired` für laufende Sitzungen in `GrantEndWatch::expire` (`share/core/access_grant_runtime.rs`).
  - Der veraltete Kommentar „does not read it yet“ am Feld `notice` in `exec_client.rs`.
- **An U21 (aus U04):**
  - die Bridge-Methoden `share.accessApprovals`, `share.decideAccessApproval` und `share.reportApprovalUndeliverable`;
  - das UI-Ereignis `ShareEvent::AccessApprovalRequested { approval_id }` in `share/core/types.rs`;
  - der Android-Notifier für Rückfragen; ohne ihn lehnt der Host mit `approval_unavailable` ab.
- **An U24 (aus U18):**
  - `grant_launch` in `exec_admission.rs` setzt `ExecWireError.code` aus `exec_platform::account_error_code(&error)`, mit Rückfall `account_unavailable`.
  - Diese Ablehnungen kommen als `helper_refused` ins Protokoll.
  - `AccessIpcRequest::Accounts` nutzt `crate::share::exec_account_choices()` in `spawn_blocking`.
  - Windows-`prepare` liest `exec_platform::preparing_account()`.
  - `exec_helper_protocol.rs` ist gemeinsam: `Start{pipe_in, pipe_out}` und `Probe{elevated, admin_group}`; eine gescheiterte S4U-Probe braucht eine neue `AccountReason`-Variante.
  - Das `cfg_attr(…allow(dead_code))` in `exec_helper_protocol.rs` fällt weg, sobald Windows alles nutzt.
  - Der Windows-Zweig von `exec_account_choices` in `api_exports.rs` wird ersetzt.
  - CLI `se share exec accounts` mit genau den Wörtern von `cli_words`/`parse_words`; `--yes` gibt es nur bei setup.
  - Die GUI nutzt `ExecAccountsElevation::PkexecGraphical` und zeigt bei `NeedsAdministrator { command, reason }` den Befehl zum Kopieren; die `AccountReason`-Codes bekommen deutsche Texte.
- **An U39 (aus U18):** Die ignorierten Konto-Tests brauchen `--include-ignored`, `--test-threads=1` und `SMART_EXPLORER_PERM_SE_BIN` mit dem absoluten Pfad zum Debug-`se`. Die Schalter dazu:
  - `SMART_EXPLORER_PERM_2204_TASK=1` (Ubuntu 22.04);
  - `SMART_EXPLORER_PERM_ROOT_TASK=1` (mit `sudo -E` als root);
  - `SMART_EXPLORER_PERM_HELPER_TASK=1` (ohne root, mit `sudo -n` und `python3`).
- **An U04-Nachfolge und U24 (aus U04 und U18):** Die Kontoliste gibt es zweimal, als `AccountChoice` in `ipc_protocol_access.rs` (U04) und als `exec_account_choices()`/`AccountChoice` in `exec_helper_protocol.rs` (U18). U24 stellt `AccessIpcReply::AccountList` und `ShareHost::account_list()` auf den U18-Typ um und entfernt den U04-Typ.
- **An U10 (aus U04 und U09):**
  - `impl ConsentBroker for crate::share::access_approval::ApprovalBroker` in `screen/core/host_backend.rs`, wörtlich aus dem U04-Bericht. Den Broker liefert `crate::share::consent_launch::approvals().broker(node.exec_registry().clone())`. Erlaubt sind nur `Once` und `Always`, `ticket.cancel()` bricht ab. Protokolliert wird über `access_audit::record_access` mit `AccessKind::Screen { mode, event }`.
  - Aus U09:
    - `#[allow(dead_code)]` in `screen/mod.rs` entfernen, sobald der Kern genutzt wird.
    - `transport_label` auf `PathKind::{Direct, Relay}` abbilden.
    - Für ein `preview`-Hello bei `ScreenView = Ask` öffnet die Gruppenansicht Vorschauen nur bei `Allow` (Vorschlag U09).
    - `SentFrame.sent_us` und `FrameAck.received_us` jeweils nach dem letzten Satz eines Bildes stempeln.
    - `on_loss_counters` mit kumulativen Zählern aus `Connection::stats()` im Sekundentakt füttern; `Pacer::decide` mit `Connection::rtt()`.
    - Nach jedem Stream-Reset und Sendefehler `Pacer::on_stream_reset` aufrufen.
    - Die Darstellung von `static_psnr_db = ∞` (JSON `null`) in `se screen bench --json` festlegen.
    - `ERROR`-Codes für Aufnahme- und Kodiererfehler festlegen.
    - Die A1-Konstanten nach der ersten Remote-Messung prüfen.
- **An U30 (aus U15):** `--open-page share-device:<id>` und `share-room:<id>` öffnen bisher nur den Reiter. U07 legt in `share_pages_ui.rs` eine Funktion zum Öffnen der Detailseite an; U30 ruft sie in `desktop_settings_ui.rs` auf. `jump_list.active` meldet bis DSK-M5 nur „Wächter läuft“.
- **An U39 und U41 (aus U15):** Die Prozessprüfungen in `desktop/os/shared/process_task_tests.rs` sind `#[ignore]`. Sie brauchen:
  - `SMART_EXPLORER_DESKTOP_TASK=1`, `--include-ignored` und einen Testthread;
  - ein gebautes `smart_explorer` neben `target/debug/deps` oder seinen Pfad in `SE_DESKTOP_TASK_GUI`;
  - `SE_DESKTOP_TEST_HEADLESS=1` für Flächen ohne Bildschirm.

  Die Windows-Steuerungstests arbeiten auf dem echten HKCU des Runners und verweigern den Lauf, wenn dort schon eine se-Desktop-Spur liegt. `desktop_task_peek_does_not_start_share_host` prüft Annahme A41 (Startgrund `drive-restore`).
- **Offen aus der vorgezogenen Registrierung (an die nächsten R-Pakete und U30):**
  - `native/src/desktop/mod.rs`: Der Staged-Kommentar ist veraltet; prüfen, ob `#![allow(dead_code)]` entfallen kann.
  - `start_request.rs`: Es fehlen noch `--screen-indicator` (R4), `--screen-host-worker` (R5) sowie `--screen-view` und `--screen-grid` (R9). Bis dahin enden diese Startarten mit Code 2 ohne Fenster.
  - Das Lizenzfenster wird nur gezeichnet, solange die Einstellungen offen sind (`frame_layout.rs` Z. 67–69). Soll es unabhängig bleiben, gehört der Aufruf nach `frame_layout.rs`.
  - `ipc.rs::require_request_auth` ruft `authorize_extension` erst mit der ersten Variante ohne Daemon-Token auf (R5, `MountHostExt`).
  - Die Beschreibung des MCP-Werkzeugs `status` (`catalog.rs` Z. 68) nennt se Desktop noch nicht.
- **An U12 (aus U06):**
  - den Linux-Startpfad in `mount_manager_start.rs` bauen und dort `allow(dead_code)` an `fail_start` entfernen;
  - in `mount_manager_host.rs` den Zweig `MountedAt` in `validate_host_status` (Pfadlänge ≤ `MAX_PATH_BYTES`) und `| MountStatus::MountedAt { .. }` in Z. 240–243;
  - `pub use types::MountLocation` in `mount/mod.rs`;
  - `--mount-point` auf `with_location_directory` abbilden;
  - das Aufräumen nach FUSE-Abstürzen über `MountManager::tick`, weil der eingeschränkte Dienst `ShareHost::tick` nicht aufruft;
  - den Linux-Anteil des Ende-zu-Ende-Signals liefern.
- **An U14 (aus U06):**
  - den Zweig `MountSource::Vault` in `daemon/os/shared/mount_source.rs`;
  - `DriveIpcRequest::MountPathFor` in `ipc_protocol_drives.rs`, festgelegt für `promotes_daemon()`;
  - entscheiden, ob ein beim Anmelden gesperrter Tresor zurückgestellt wird oder als Fehlstart erscheint;
  - entscheiden, ob Sperren den Registry-Eintrag behält.
- **An U17 (aus U06):**
  - Der Gerätebaum-Eintrag liegt in `registry-2.json` und hält den Anmeldeeintrag aktiv.
  - `DevicesStatus` und `DevicesWatch` gelten in `promotes_daemon()` als nicht auslösend.
  - Kind-Engines rufen `open_share` erst nach `run_start_reason::share_allowed()` auf.
  - `settings_devices_ui.rs` wird in `app/drive_modules.rs` deklariert.
- **An U36 (aus U06, Bestandsfehler):** Der Deinstaller entfernt den Run-Wert `SmartExplorerSync` nicht (schon heute) und ebenso wenig den neuen Wert `SmartExplorerDrives`. U36 entfernt beide bei der Deinstallation.
- **An U39 (aus U06):** Zusatzsignal `drive_task_update_stop_exit_clean_journal_counts_as_unmount` (`daemon::mount_manager::entry_state::update_stop_tests::…`) in die Inventarliste aufnehmen.
- **An das nächste R-Paket (aus U08):**
  - `native/src/lib.rs`: zwischen `pub mod updater;` und `pub mod vfs;` die Zeile `pub(crate) mod vault;`.
  - `ipc_protocol_mount.rs`: `MountBackendScheme::Vault` (serde „vault“) und beide `From`-Zweige.
  - `vfs/mod.rs`: Re-Export `split_stage_name` und `StageParts`.
  - Nötige `match`-Zweige: `connect/core/local_endpoint.rs` (`Vault { .. } | VaultStore { .. } => Ok(None)`) und `mobile/core/crumbs.rs::root_label` (`LocKind::Vault => "Tresor"`).
  - **Fehlweg vorbeugend beheben:** `cli/target.rs:176-187` (`endpoint_backend_keys`) gibt `vault://` den Namensraum `local`. Sonst würde `se mv vault://…/a /home/x` im Tresor umbenennen statt zu kopieren. Nötig ist der Namensraum `vault:<site-id>`; das muss vor U26 erledigt sein.
- **An U29 (aus U08):**
  - `app/core/prefs_tabs.rs:122-145` (`navigate_to_location`) schickt `vault://` in `start_scan`, eine lokale Pfad-API (Fehlweg).
  - `cache_remote` hüllt jedes nicht lokale Backend in `CachingBackend`; für Tresore verhindern oder beim Sperren leeren.
- **An U11 (aus U08):**
  - In `vault/os/shared/open.rs::open_in` den IPC-Weg `daemon::open_vault_backend` für GUI- und CLI-Prozesse ohne eigene Sitzung einsetzen.
  - Gepoolte Backends beim Sperren verwerfen.
  - `VaultLocked` wird auf den Bridge-Code `vault_locked` abgebildet; heute wird es in `mobile/core/error.rs` „permission“.
  - Die Sitzungstabelle (`publish_session`, `withdraw_session`, `unlocked_session`) steht in `open.rs`.
- **An U20 (aus U08):** Tresor-Jobs brauchen einen eigenen Prüfweg, weil `validate_sync_endpoints` alle Tresororte ablehnt (`VAULT_SIDE_REFUSED`).
- **An U43 (aus U08), `docs/TODO.md`:**
  - ein eigenes Feld für Tresororte im `CleanupReport`; heute stehen sie unter `file_errors` und erscheinen als Fehler;
  - Annahme A4: Das ordnerübergreifende `promote_staged` ist für FTP, WebDAV, SMB und Share-Peers ungeprüft.
- **An das nächste R-Paket (aus U21):**
  - `domains/mod.rs`, in `share_method` nach `"share.execLog"`: `share.setAccess`, `share.accessApprovals`, `share.decideAccessApproval` und `share.reportApprovalUndeliverable` auf `share_exec::access::{set_access, approvals, decide, report_undeliverable}`. Keine neue `mod`-Zeile, denn die Untermodule sind schon in `share_exec.rs` und `share_requests.rs` eingebunden.
  - `daemon/mod.rs`: Re-Export um `mutate_access_grant` und `report_approval_undeliverable` erweitern.
  - Danach sind diese `#[allow(dead_code)]` überflüssig und werden entfernt: an `mutate_access_grant` und `report_approval_undeliverable` in `access_ipc.rs` und an `set_notifier` in `access_approval.rs`.
- **Entscheidung zu U04/U21:** `ShareEvent::AccessApprovalRequested` entfällt. Android weckt seinen Notifier über die Host-Aktivität; der Desktop öffnet sein Fenster über `consent_launch`. Eine neue `ShareEvent`-Variante würde außerdem ältere GUIs beim Lesen des Snapshots scheitern lassen.
- **An U27 (aus U21):** Aktivitäts-Intents für Bildschirm-Rückfragen in `approvalNotification` (`AccessApprovalNotifier.kt`); den Filter in `share_access.rs::android_rows` aufheben; den Bildschirm-Zweig von `AccessApprovalPrompt` erweitern.
- **An U39/U41 (aus U21):** `PermTaskTest` gehört in die Android-Gerätedurchläufe. Die Tests `perm_task_bridge_*`, auch die drei `perm_task_bridge_exec_stream_*`, gehören in die Linux-Unit-Tests.
- **An U24 (aus U07):**
  - `programs_ui::account_options` liest an `AccountChoice` die Felder profile, account, available, reason und next_step; mit dem Wechsel auf den U18-Typ zieht die Funktion mit.
  - Für „Konten dieses Geräts“ kommt eine Aufrufzeile in `ui_exec_overview` hinter `log_ui::ui_exec_log(ui);`.
  - Die Vervollständigung von `--as` bietet `remote` immer an; sie soll die Kontenliste des Workers lesen.
  - **Alle Oberflächen:** `se share device list` und `se share room list` (`cli/share/devices.rs`, `rooms.rs`) zeigen noch keine Zugriffsfelder. Der Rücktitel „Befehle eines Geraets“ in `share_window_ui.rs` passt nicht mehr zum neuen Seitentitel „ZUGRIFF“.
- **An U30 (aus U07):** Zum Öffnen der Detailseiten gibt es `App::open_share_contact_page(&mut self, contact_id: &str) -> bool` und `App::open_share_room_page(&mut self, room_id: &str) -> bool`; `false` öffnet die Tab-Übersicht. Die beiden `#[allow(dead_code)]` entfallen mit dem Aufruf aus `desktop_settings_ui.rs`.
- **An das nächste Android-Paket (aus U07):** In Android fehlen noch der Satz nach C6 und der Text nach C5 für `ExecProgramsDialog.kt:137`.
- **An U39/U41 (aus U07):**
  - Tests ohne Stapelpräfix in die Namensliste aufnehmen: `shell_session_task_share_session_dialect_is_asked_inside_the_shell`, `shell_session_task_share_session_needs_one_start_line`, `grant_end_watch_logs_grant_expired`, `share_pages_open_detail_pages_from_outside`.
  - Die vier GUI-Tests aus U07 laufen ohne Anzeige und ohne `#[ignore]`.
  - Ungeprüft ist, wie `cmd.exe` die neue Startzeile von `se shell -T` liest. Vorschlag für windows-core: die Zeile mit `cmd /D /S /C` und der Eingabe `echo SE_DIALECT_x_$?` ausführen und `SE_DIALECT_x_True` erwarten.
- **An U41 und den ersten Suite-Lauf (aus U39):**
  - **Dokany:** `SE_TASK_DOKANY_MSI` und `SE_TASK_DOKANY_MSI_SHA256` sind nicht gesetzt, daher melden alle Dokany-Tests „nicht ausgeführt“. Die Suite übernimmt die festgelegte Dokany-Quelle mit Hash aus dem bestehenden Release-Weg (`docs/RELEASING.md`, „Pinned Dokany installer dependency“).
  - Die Testserver-Matrix aus `native/sync-reliability-providers.py` (SFTP, Agent, FTP/FTPS, SMB, WebDAV) für vault-backends ist noch nicht angebunden.
  - Ebenso fehlt das Gerätepaar Emulator ↔ Runner nach `native/review-task-device.py`.
  - Annahme zu `android/test-android-task.sh android-build`: Verlangt das Skript neben x86_64 weitere Ziele, scheitert der Bau.
  - Annahme: Ein Job auf `ubuntu-22.04` scheitert sofort, wenn das Abbild fehlt, statt in der Warteschlange zu hängen.
  - `drive_task_fuse_kernel_request_timeout_reported` meldet „nicht ausgeführt“, weil `fs.fuse.default_request_timeout` erst ab Linux 6.14 existiert.
- **An U11, U12, U14, U20, U26, U36 und U42 (Verträge aus U39):**
  - Umgebungsvariablen: `SE_TASK_SHARE_*`, `SE_TASK_SFTP_*`, `DBUS_SYSTEM_BUS_ADDRESS`, `SE_VAULT_INTEROP_JAVA/JAR`, `SE_VAULT_TASK_KEEP_VAULTS/<test>/<fall>/vault`, `SE_VAULT_TASK_MARKER/ROOT` (mit `input/` und `allowed-marker-paths.txt`).
  - U36: `install-linux.sh --dry-run --with-fuse` endet mit 0, nennt `fuse3` und schreibt nichts nach HOME.
  - Ein neues Abnahmesignal wandert in `native/rdt_task/expected_signals.txt` vom Abschnitt „noch nicht umgesetzt“ nach „umgesetzt“; das erledigt das Paket, das den Test bringt.
- **An U14 (aus U11):**
  - `Vault::SpoolKey { mount_id }` wird mit dem Sitzungstoken geprüft; den Wickelschlüssel liefert `register().with_spool_wrap_key(site, |key| …)`.
  - Je Laufwerk gibt es einen `LockParticipant::prepare_lock` mit `MountHostRequest::PrepareLock { deadline_ms }`. Fristen: Ruhezustand 4 s, manuelles Sperren und Entfernen 60 s, sonst 15 s. Bei ungespeicherten Änderungen liefert er `Err`.
  - Beim Einhängen `consume(site, ConsumerKind::Drive(mount_id))`, beim Aushängen `mount_ended(…)`, außerdem `subscribe` für das Aufräumen der Miniaturbilder (K45).
- **An U20 (aus U11):**
  - Versiegeln läuft im Worker über `register().seal_texts`/`unseal_texts` und außerhalb über `daemon::{seal_vault_items, unseal_vault_items}`.
  - Die Ortsdatei heißt `<VAULTS_DIR>/<site-id>.local`.
  - `jobs_using` erkennt `*.conf` mit `vault_site_a`/`vault_site_b`; andere Formen ergänzt U20.
  - Die Ersatzkopie läuft über `ConsumerKind::FallbackCopy`.
- **An U26 (aus U11):**
  - CLI und MCP nutzen `daemon::{unlock_vault, unlock_vault_saved, lock_vaults, vault_status, seal_vault_items, unseal_vault_items, open_vault_backend}`; die Verwaltung läuft über `vault::{create_site, add_site, detect_location, remove_entered, change_site_password, update_settings, recovery_key_info}`. Danach entfallen die beiden `allow(unused_imports)` in `daemon/mod.rs`.
  - `Compare`, `CompareText`, `Resolve`, `MergeApply` und `Check` kommen in `VaultIpcRequest`/`VaultIpcReply`, in `ipc_vault.rs::serve` und in `promotes_daemon`.
  - MCP verwirft zwischengespeicherte Backends bei `vault::locked_site(&error)`.
  - Die acht Codes brauchen Texte: `vault_locked`, `vault_wrong_password`, `vault_unsupported_format`, `vault_hub`, `vault_memory`, `vault_no_saved_password`, `vault_unsaved_changes` und `vault_not_atomic`.
- **An U29 (aus U11):**
  - Status über `vault_status()` (`event_seq`, `events`).
  - `screen_lock_note` neben dem Schalter für die Bildschirmsperre.
  - Bei `VaultLocked` gepoolte und IPC-Backends verwerfen, ohne Zwischenspeicher über Tresoren.
  - Ein Dialog für `vault_unsaved_changes` (dann `lock_vaults(site, true)`), Prüfung über `check_new_password` und die Texte zum Wiederherstellungsschlüssel.
- **An U31 (aus U11):**
  - Die `vault.*`-Bridge nutzt das Register im Prozess.
  - `vault.unlock` übergibt `memAvail − memThreshold` als `UnlockOptions.available`.
  - Das Bridge-Ereignis `vault` kommt über `register().subscribe`.
- **An U42/U43 (aus U11):** Der Doku-Bedarf steht im U11-Bericht: neue Datei `tresore.md` mit Sperrauslösern, Codes, IPC-Weg und Grenzen; Einträge in `GOTCHAS.md`; TODO-Einträge `VAULT-SCREENLOCK-VERIFY` und `VAULT-RECOVERY-KEY`; in `api.md` `sys.hostState.screenOn`.
- **An das nächste R-Paket (aus R4a):** `daemon/mod.rs` exportiert `vault_status_peek` (U30, `ipc_peek_client.rs`) noch nicht; `desktop/os/shared/feed_sources.rs` und `feed_task_tests.rs` rufen es als `crate::daemon::vault_status_peek` auf.
- **An das nächste R-Paket (aus U12):**
  - `native/Cargo.toml`, Linux-Ziel: `fuser = { version = "=0.18.0", default-features = false }` (MIT); neu im Lock sind `fuser` und `page_size 0.6.0`.
  - `ipc_protocol_mount.rs` (Pflicht): `MountHostConfig.location: Option<crate::mount::MountLocation>` mit `#[serde(default, skip_serializing_if = "Option::is_none")]`, gesetzt in `From<&MountConfig>`.
  - `catalog.rs`: das Linux-Beispiel `drive mount … --mount-point`; die INSTRUCTIONS nennen Laufwerke für Windows und Linux.
  - Die 25 Inventurzeilen aus dem U12-Bericht.
  - **Mitbeheben (Hauptagent):** In `mount_manager.rs::start` gilt `ensure_registry_ready()` nur unter `#[cfg(windows)]` und muss auch für Linux gelten. Außerdem zeigen unter Linux `mount_ui_draft.rs` den Dokany-Hinweis und `mount_ui_cache_settings.rs` die Dokany-DLL-Option, und `mount_ui_helpers.rs` meldet „Dokany fehlt“. Diese Texte und Optionen gehören nur unter Windows.
- **An U14 (aus U12):** Der Zweig `MountSource::Vault` in `mount_source::resolve`; die versiegelte Ablage an der Stelle `MountStorage::plain()` in `host_session.rs`; `MountHostConfig` trägt keine Quelle, ein Tresor wird über `mount_point::place_kind` erkannt; für die Sperrfrist steht `fuse_abort::abort_connection(connection_at(..))` bereit; `MountedAt` für Tresorpfade auswerten.
- **An U17 (aus U12):** Die Haken für GTK-Lesezeichen, KDE-Orte und Öffnen im Dateimanager fehlen in `FuseHost::start_volume`/`shutdown`. `FuseShared` arbeitet über `dyn MountFilesystem` und nimmt den Gerätebaum als Frontend auf; `CrossesDevices` ist bereits auf `EXDEV` abgebildet.
- **An U36 (aus U12):** `install-linux.sh --with-fuse`:
  - prüft `command -v fusermount3`, das setuid-Bit und den Zugriff auf `/dev/fuse`;
  - installiert `fuse3` nur mit dem Schalter, über `sudo` mit `apt-get`, `dnf`, `pacman -S` oder `zypper`;
  - lässt `/etc/fuse.conf` unverändert, und die Deinstallation entfernt `fuse3` nicht;
  - `--dry-run --with-fuse` endet mit 0 und schreibt nichts nach HOME.
- **An das nächste R-Paket (aus U24):**
  - `cli/share.rs`: `mod exec_accounts;`.
  - `cli/share/exec_status.rs`: die Variante `Accounts(super::exec_accounts::AccountsArgs)` und die zwei Zweige, wörtlich aus dem U24-Bericht.
  - `native/Cargo.toml`, windows-sys-Features: `Win32_NetworkManagement_NetManagement`, `Win32_Security_Authentication_Identity`, `Win32_System_Environment`, `Win32_System_Services`.
  - `catalog.rs`: das Beispiel `["share","exec","accounts","--json"]`.
  - Inventur: Die Zeilen von U24 wandern nach „umgesetzt“. Dafür gelten `env=SMART_EXPLORER_PERM_WINDOWS_TASK=1 env=SMART_EXPLORER_PERM_SE_BIN={se}`, `ignored` und `threads=1`.
  - Danach entfallen die `#[allow(dead_code)]` an `preparing_account` und `account_error_code` (`exec_platform.rs`) und an `HelperRefused` (`access_audit.rs`).
- **An U36 (aus U24):** Die Deinstallation entfernt den Kontodienst mit `ExecWait '"$INSTDIR\se.exe" share exec accounts remove…'`, mit Rückfrage zum Löschen von `se-remote` (Vorauswahl Nein). Die NSIS-Vorlage steht in `stand/U24.md`, Abschnitt 4. Bedingungen von `perm_task_installer_uninstall_removes_service_static`:
  - wörtlich `se.exe` in der Aufrufzeile;
  - ein Kommentar mit `UAC` oder `runas` höchstens 12 Zeilen davor;
  - ein `DetailPrint` höchstens 24 Zeilen danach.
- **An das nächste R-Paket (aus U30):**
  - `native/Cargo.toml` (R10, H6): `image = { version = "0.25.10", default-features = false, features = ["png", "jpeg", "gif", "webp", "bmp"] }` nach `trash = "5"`.
  - Re-Exporte in `ipc_client.rs` und `daemon/mod.rs`: `drive_devices_peek`, `drive_devices_watch_peek` und `share_worker_peek` öffentlich, `vault_status_peek` als `pub(crate)`.
  - `cli/desktop.rs:248-249`: `device_tree_mounted` aus `feed_sources::device_tree_now().mounted()`.
  - `desktop/os/shared/status.rs:302-306`: `devices_folder` aus `device_tree_now()`.
  - Die 14 Inventurzeilen aus dem U30-Bericht, mit den Stufen `linux-core:unit` und `linux-desktop:gui-harness`.
  - Der Staged-Kommentar in `desktop/mod.rs:11-13` ist veraltet; das `#![allow(dead_code)]` dort bleibt bis U32/U34.
  - Sobald U10 fertig ist, entfallen die beiden `#[allow(dead_code)]` in `share_pages_ui.rs:89/100`.
- **An U17 (aus U30):** `DevicesStatus` und `DevicesWatch { since: u64 }` in `ipc_protocol_drives.rs` (serde `devices_status`/`devices_watch`, lösen die Einschränkung nicht auf, Antwort ist das V4-Objekt, Geräteschlüssel `%XX` in Großbuchstaben); danach stellt `ipc_peek_client.rs:70-87` auf typisierte Anfragen um.
- **An U32 (aus U30):**
  - `desktop_folders` mit `FOLDERID_Desktop`/`FOLDERID_PublicDesktop`, Versteckt- und System-Attribute in `scan_folders`;
  - Systemhintergrund je Monitor über `FitMode::from_windows`;
  - `EframeHost::work_area` auf `rcWork`;
  - `NativeWindow::Win32` für die Unterklasse, danach entfallen die zwei `allow` in `desktop_surface_host.rs`;
  - Strg+V nach einer Explorer-Kopie;
  - Dateien aus der Fläche herausziehen;
  - Monitornamen.
- **An U34 (aus U30):**
  - `XDG_DESKTOP_DIR` mit Starter-Vertrauen nach C2.7;
  - „Öffnen mit“ über das OpenURI-Portal;
  - Systemhintergrund über `FitMode::from_gnome`;
  - `_NET_WORKAREA` und die X11-Fenster-ID;
  - Ziehen zwischen Monitoren;
  - Datei-Zwischenablage mit anderen Programmen.
- **An U37 (aus U30):** `SurfaceHost` für das Layer-Backend (`NativeWindow::Wayland`), je Ausgabe `show_surface`, eine eigene `Outbox`, Tastatur über `wl_keyboard`. Unter Wayland gibt es kein Ablegen von Dateien und ohne `inner_rect` kein Ziehen zwischen Monitoren.
- **An U13 (aus U10):** `open_encoder` für H.264; `CodecOffer::h264` und `with_max` nutzen; `BenchOptions::decoders()` lehnt H.264 noch ab (`screen/core/bench.rs:66`); `pixel_convert` und `RELAY_QUEUE_BYTES`/`Pacer::frames_in_flight` nutzen; `--screen-host-worker`.
- **An U16 (aus U10):** In `NodeScreen::new` das Plattform-Backend statt `SyntheticBackend`; Monitore, `LIST_WINDOWS`, Standbild; `SourceFrame::Unchanged`/`Paused` sind vorbereitet; MCP-Werkzeug `screen` (R6).
- **An U19 (aus U10):** X11- und Wayland-Backends. Prüfen, ob der Portal-Dialog erst beim Öffnen der Quelle nach der Zustimmung erscheinen darf (`host_server.rs`, `open_source` nach `confirm`). Dazu `LIMIT_CONSENT_EACH_SESSION`; ein root-Worker meldet `no_display`.
- **An U25 (aus U10):** `screen_stream`, `ScreenStream`, `StreamEvent` und `StreamRequest` nutzen (vorbereitet in `screen_ipc_client.rs`), dazu die Eingabehilfen aus `input_model`. Eine Vorschau unter „Fragen“ braucht für das Vollbild eine neue Verbindung im Modus `view`.
- **An U41 (aus U10 und U30):** Die S4- und S5-Inventurzeilen trägt U41 ein. Die Pfade aus U10 und U30 stehen in `stand/U10.md` und `stand/U30.md`, die U09-Pfade im U09-Commit. Stufen:
  - Bildschirm: `linux-core:screen-pair` mit zwei Iroh-Knoten auf Loopback und `linux-core:unit`; `screen_task_relay_only_bench` braucht `SE_TASK_SHARE_SIGNAL` und `SE_SHARE_RELAY_ONLY=1`; `screen_task_netem_recovery` braucht `tc netem` und `SE_SCREEN_TASK_NETEM=1`.
  - Desktop: `linux-core:unit` und `linux-desktop:gui-harness`.
- **An R2 (aus R1 offener Punkt 1):** Die eigentliche IPC-Verteilung steht in `native/src/daemon/os/shared/ipc.rs` (`handle_client`, `match req`), nicht in `ipc_host.rs`; die Annahme in B2 war falsch. `ipc.rs` gehört ab jetzt zu den Hub-Dateien der R-Pakete. R2 setzt dort zwei Zeilen:
  - letzter Zweig in `handle_client`: `req => host.serve_extension(stream, req),`;
  - in `require_request_auth` wird `_ => Err(…)` zu `_ => host.authorize_extension(req),`.
- **An U12 (aus R1 offener Punkt 3):** Der Linux-Host setzt `MountHostConfig.path_dialect = Posix`; `From<&MountConfig>` setzt weiter `Windows`.
- **An U39 und U41 (aus U03 offener Punkt 1):**
  - `desktop_task_updater_helper_unchanged` läuft in `linux-core: unit`, nicht in `prepare`.
  - `desktop_task_online_rule_matches_bridge` und `desktop_task_share_status_device_shape_unchanged` gibt es nur in Linux-Testbauten.
  - Die zwei GUI-Tests brauchen `SMART_EXPLORER_GUI_TASK=1`, isolierte Werte für `XDG_DATA_HOME`, `APPDATA` und `LOCALAPPDATA`, `--include-ignored` und einen Testthread.

## B — Zweitplan aus dem Bestand

Quellen: Abschnitte B und D der fünf Strangpläne (Endfassungen), die dort genannten Lesungsdateien und eigene Zählung am Lesestand `da2646d2` am 2026-10-17 (`wc -l`, `wc -c`, gezieltes `grep`). Abkürzungen wie in den Strangplänen: `N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`.

### B1 Bestandsfakten je Strang

**S1 Exec-Rechte** (Lesungen `docs/lesungen/2026-10-17-exec-rechtemodell-host.md`, `docs/lesungen/2026-10-17-exec-rechte-oberflaechen.md`; Strangplan B1–B28): Ein Exec-Grant liegt je Direktfreigabe und je Raummitglied (`ExecGrant` mit `enabled`, `run_as`, `programs`, Standard aus). Die volle Shell (leere Programmliste) blockiert `ls` nicht; nur eine Programmliste sperrt Shell-Befehle und Terminal. Einen Kontowechsel gibt es heute nur beim Linux-root-Worker (setuid im `pre_exec` ohne PAM); Worker und Installation laufen je Nutzer (Linux XDG-Autostart und `~/.local/opt/smart-explorer`, Windows `HKCU\…\Run` und `%LOCALAPPDATA%\Programs\Smart Explorer`). Die se-IPC läuft über TCP-Loopback mit dem Token `daemon.token` im Datenordner des Worker-Kontos. Bestandsdefekt: Die transiente systemd-Einheit setzt `ExitType=main`, das systemd 249 (Ubuntu 22.04) ablehnt (`N/share/os/linux_os/exec_systemd.rs:54`).

**S2 Laufwerke** (Lesungen `docs/lesungen/2026-10-17-laufwerke-kern.md`, `docs/lesungen/2026-10-17-laufwerke-windows-dokany.md`; Strangplan B2 Nr. 1–18): Die Mount-Engine in `N/mount/core` ist plattformneutral, prüft Namen aber nur im Windows-Dialekt. Der Linux-Start ist ein Stub (`N/daemon/os/linux_os/mount_process.rs:10-20`); der Hostablauf ist bis auf Dokany generisch. `registry.json` (Version 1) wird bei jeder unbekannten Enum-Variante ganz ungültig; nach einem Daemon-Neustart wird nichts wieder eingehängt. Bestandsfehler: Der Rückfall `ERROR_INVALID_DATA` in `N/mount/os/windows/callback_status.rs:215-248` erscheint im Explorer als „Zugriff verweigert“. SMB wird am Desktop nie eingehängt.

**S3 Tresore** (Lesung `docs/lesungen/2026-10-17-tresore-bestand.md`, dazu `docs/CRYPTOMATOR_VAULTS_ERKUNDUNG.md`; Strangplan B1–B15): Der Vertrag `Backend` hat 74 Methoden, `BackendExtensions` 24 Methoden. Orte laufen über `EndpointSpec` (`Local`, `Saved`, `Drive`, `Peer`), und `vfs::backend_for` behandelt jede Wurzel ohne bekanntes Schema als lokalen Pfad. IPC-Fehler werden nur als Text übertragen. Die Klartext-Ablageorte K01–K43 sind in der Lesung aufgelistet. Die Anwendungsphase des Syncs gruppiert nur gleiche oder ineinanderliegende Pfade und kennt keine Abhängigkeiten (`N/bisync/os/shared/apply_reporting.rs:120-135`). Die Desktop-GUI lädt Jobs streng: Ein ungültiger Job ergibt eine leere Liste. Die neue RustCrypto-Generation und `icu_normalizer 2.2` liegen bereits im Lock; neu sind nur `aes-siv 0.8.0` und `aes-kw 0.3.1`.

**S4 Bildschirm** (Lesung `docs/lesungen/2026-10-17-bildschirm-bestand.md`; Strangplan B): Transport über `iroh` 1.3.0 mit den ALPNs `smart-explorer/share-fs/3`, `smart-explorer/share-exec/2` und `smart-explorer/paired-link/1`. Die endpunktweite Konfiguration erlaubt 0 unidirektionale Streams, die Methoden `open_uni`/`accept_uni` gibt es aber je Verbindung. `read_payload`/`write_payload` in `N/share/core/exec_protocol.rs` sind privat. Der Relay-Server verwirft Pakete bei voller Ziel-Warteschlange. Darstellung: eframe 0.29.1 mit `wgpu`. Android: `jni = "=0.22.4"`, `minSdk` 30, `targetSdk` 36. Das Release verteilt Lizenztexte nur für Dokany (`native/installer.nsi:134-139`).

**S5 se Desktop und Launcher** (Lesungen `docs/lesungen/2026-10-17-desktop-integration-bestand.md`, `docs/lesungen/2026-10-17-android-geraete-bestand.md`; Strangplan B): `run_gui` prüft die Startschalter in fester Reihenfolge (`N/lib.rs:65-138`). Bestandsfehler R70: Der Updater-Helfer beendet keine Prozesse, schreibt nur `daemon.stop` und bricht nach 30 s ab; GUI-Fenster beachten den Merker nicht. `share_worker_snapshot` startet den Hintergrunddienst über `ensure_worker_ready`. Die MCP-`INSTRUCTIONS` belegen 1 887 Byte bzw. 1 885 Zeichen von 2 048 Byte (die Prüfung `INSTRUCTIONS.len() <= 2_048` in `N/cli/mcp/catalog.rs:310` zählt Byte; eigene Messung). Alle Shell-Registrierungen liegen unter HKCU. Android hat eine Aktivität ohne HOME-Filter; `SmartExplorerApp.onCreate` führt sieben Startschritte aus; das Navigationstoken von `AppNav` entsteht im lesenden Prozess.

**Release und CI (alle Stränge):** Versionsstand `native/Cargo.toml` = `0.5.182`, `release-native/update-feed/version.txt` = `0.5.182`. `build.yml` läuft bei jedem Push auf jeden Zweig (`push: branches: ["**"]`). Das vollständige Release läuft im Job `complete-release` auf `windows-2025` (Zeitlimit 360 min). Der Job ruft `pwsh ./native/publish-release-local.ps1` mit 330 min Schrittzeit auf, wenn `complete_release_source_sha` gesetzt ist. Diesem Job geht der Job `android-release-apk` voraus. `cargo audit` läuft in `build.yml:454-457` mit zwei `--ignore`. Das Release hat 20 Assets, der Feed 16 Dateien (`native/release-publication.ps1:121-158` laut S5-Bestand). Muster der Task-Suiten: ein Eingang `candidate_sha` (`luecken-task.yml`), ein Python-Einstieg mit `--job`, Prüfung des Kandidaten-SHA im ersten Schritt, Upload der Belege mit `if: always()`. Der KVM-Emulatorlauf läuft über `reactivecircus/android-emulator-runner@v2`, die KVM-Regel wird per `sudo` gesetzt (`android-background-task.yml:453-455`).

### B2 Dateien, die mehr als ein Strang ändert

Grenze nach AGENTS.md: neue oder wesentlich bearbeitete Rust-Dateien unter 500 Zeilen und 50 KiB. Alle Zählungen am Lesestand `da2646d2`. Die Spalte „Regel“ ist verbindlich für die Pakete in D. „R-Paket“ meint die Registrierungspakete aus D1.

| Datei | Zeilen / Byte | Ändernde Stränge (Meilensteine) | Regel vor und bei der Erweiterung |
|---|---|---|---|
| `N/daemon/os/shared/ipc_protocol.rs` | 475 / 14 790 | S1 PRM-M3 (`Access`), S2 DRV-M1/M3/M4/M6 (`path_dialect`, `Drive`, `location`, `MountHostExt`), S3 VLT-M2/M4 (`MountBackendScheme::Vault`, `Vault`, `Err.code`), S4 SCR-M2 (`Screen`) | Ohne Ausgliederung käme die Datei auf etwa 520 Zeilen. Ausgliederung zuerst (R1): Die Typen des Laufwerkshosts (`MountHost*`, `MountBackendScheme` samt Zuordnungen, heute `:31-51`, `:326-345`) wandern ohne Formänderung nach `N/daemon/os/shared/ipc_protocol_mount.rs`, und `ipc_protocol.rs` re-exportiert sie. Danach erhält `ipc_protocol.rs` je Strang nur das Variantenpaar; die Untertypen liegen in `ipc_protocol_access.rs` (S1), `ipc_protocol_drives.rs`/`ipc_protocol_mount_host.rs` (S2), `ipc_protocol_vault.rs` (S3) und `ipc_protocol_screen.rs` (S4) |
| `N/daemon/os/shared/ipc_host.rs` | 479 / 18 673 | S3 VLT-M4 (ein Zweig); die Verteilung der neuen Varianten von S1, S2 und S4 braucht ebenfalls eine Stelle | R1 legt `N/daemon/os/shared/ipc_host_ext.rs` an (Verteilung aller neuen Varianten, ≤ 200 Zeilen) und ruft es in `ipc_host.rs` an einer Stelle auf (+ ≤ 4 Zeilen). **Annahme:** `ipc_host.rs` verteilt an einer `match`-Stelle, an der ein Rückfallaufruf mit denselben Kontextwerten möglich ist. Ist das falsch, setzt R1 den Aufruf in die Funktion, die die Variante entgegennimmt, und nennt die Stelle im Bericht. `N/daemon/os/shared/exec_ipc.rs` (455) wird dadurch nicht geändert (S1 PRM-M3 hatte dort eine Weiterleitung geplant) |
| `N/daemon/os/shared/ipc_client.rs` | 489 / 18 235 | S5 DSK-M3 (2 Zeilen `#[path]` + `pub use`) | Nur diese 2 Zeilen (→ 491). Die Clients von S3 (`ipc_vault_client.rs`) und S4 (`screen_ipc_client.rs`) werden in `N/daemon/mod.rs` deklariert, nicht in `ipc_client.rs` |
| `N/share/mod.rs` | 488 / 14 872 | S1 (rund 30 neue Module, PRM-M1–M7), S4 SCR-M2 (`node_screen`), S5 DSK-M1 (`device_view`) | S1 deklariert alle neuen Module in der eigenen Sammeldatei `N/share/access_modules.rs` (angelegt von U01). `share/mod.rs` erhält genau `#[path = "access_modules.rs"] mod access_modules;` und `pub(crate) use access_modules::*;` (R1), dazu 2 Zeilen `node_screen` (R4) und 2 Zeilen `device_view` (R1) → 495 Zeilen |
| `N/daemon/mod.rs` | 290 / 9 612 | S1, S2, S3, S4, S5 (rund 25 neue Module) | Die S2-Module stehen in der Sammeldatei `N/daemon/drive_modules.rs` (U06 legt sie an, R2 setzt 3 Zeilen in `daemon/mod.rs`, danach gehört sie S2). Die übrigen Module deklarieren die R-Pakete (Zuwachs ≤ 30 Zeilen) |
| `N/app/mod.rs` | 344 / 9 524 | S1 (5 Dateien), S2 (2), S3 (10 einschließlich `N/app/os/shared/vault_open.rs`), S4 (13), S5 (11), Lizenzen (1) | Sammeldateien je Strang. Das erste Paket des Strangs mit App-Dateien legt sie an; das R-Paket derselben Phase setzt je 3 Zeilen in `app/mod.rs`: `N/app/desktop_modules.rs` (U03/R1), `N/app/access_modules.rs` (U04/R2), `N/app/drive_modules.rs` (U06/R2), `N/app/screen_modules.rs` (U10/R4), `N/app/vault_modules.rs` (U20/R7). Danach deklariert jeder Strang seine Dateien dort selbst. `app/mod.rs` → 361 Zeilen einschließlich 2 Zeilen für das Lizenzmodul (R11) |
| `N/lib.rs` | 185 / 5 385 | S1 (`--access-consent`), S2 (`--drive-restore`), S3 (`mod cryptomator; mod vault;`), S4 (`mod screen;`, `--screen-indicator`, `--screen-host-worker`, `--screen-view`, `--screen-grid`), S5 (`pub mod desktop;`, Wächter/Flächen/`--open-*`), Lizenzen (`mod licenses;`) | Ein gemeinsamer Parser `N/app/core/start_request.rs` (H8, U03). `lib.rs` erhält nur Modulzeilen und je Startart einen Aufruf (R-Pakete); geschätzt ≤ 230 Zeilen |
| `N/main.rs`, `N/bin/se.rs` | 19 / 608; 102 / 3 091 | S2 DRV-M3 (`--drive-restore`), S4 SCR-M3 (`--screen-host-worker` als einziges Argument) | nur R-Pakete; je ≤ 6 Zeilen |
| `N/cli/mod.rs` | 302 / 9 858 | S2 (`drive_set`, `drive_open`, `drive_devices`, `drive_output_linux`), S3 (`vault`), S4 (`screen`), S5 (`desktop`), Lizenzen (`licenses`) | nur R-Pakete; ≤ 340 Zeilen |
| `N/cli/completions.rs` | 413 / 14 233 | S1 (`completions_access.rs`), S4 (`completions_screen.rs`), S3/S5 über clap | neue Vervollständigung in eigenen Dateien; `completions.rs` nur Aufrufzeilen (R-Pakete, ≤ 12 Zeilen) |
| `N/cli/mcp/catalog.rs` | 351 / 23 889 | S1 PRM-M8, S2 DRV-M4/M6/M9, S3 VLT-M9, S4 SCR-M2/M4/M11, S5 DSK-M3 | nur R-Pakete; Werkzeug `screen` in `N/cli/mcp/screen_tool.rs` (S4); `INSTRUCTIONS` nach C1 V12; Ziel ≤ 420 Zeilen |
| `N/cli/mcp/status_tools.rs` | 146 / 5 394 | S3 VLT-M9, S5 DSK-M3 | nur R-Pakete |
| `N/mobile/os/shared/domains/mod.rs` | 250 / 10 490 | S1 PRM-M10, S2 DRV-M8, S3 VLT-M4/M11, S4 SCR-M8/M9, Lizenzen | Neue Präfixe (`vault`, `docs`, `screen`, `app`) und `share`-Methoden verweisen auf Strangdateien; Moduldeklarationen und Verteilzeilen setzen nur R-Pakete (≤ 300 Zeilen) |
| `N/mobile/core/events.rs` | 65 / 2 141 | S3 VLT-M11 (Ereignis `vault`), S4 SCR-M8 | nur R-Pakete |
| `native/android-bridge/src/lib.rs` | 163 / 5 969 | S2 DRV-M8 (`mod io;`), S4 SCR-M8 (`mod screen_jni;`) | nur R-Pakete |
| `native/Cargo.toml` | 282 / 14 691 | S2 (`fuser`), S3 (`aes-siv`, `aes-kw`, `Win32_System_RemoteDesktop`, `image`), S4 (`lz4_flex`, `png`, `openh264`, `bzip2`, `libloading`, `x11rb`, `wayland-*`, `ashpd`, 17 `windows`-Features), S5 (`image`, Wayland-/X11-Pakete, 2 `windows`-Features), S1 (Annahme: zusätzliche `windows-sys`-Features für LSA, Dienste und NetUser in PRM-M7) | nur R-Pakete; eine gemeinsame `image`-Zeile (H6). `native/Cargo.lock` ändert niemand von Hand (C2 Punkt L1) |
| `N/share/core/exec_protocol.rs` | 456 / 15 138 | S1 PRM-M2 (Rahmen 39, `client_features`, `access`), S4 SCR-M1 (zwei Funktionen `pub(crate)`) | S1 legt neue Typen in `exec_access_view.rs` und `exec_client_messages.rs` ab; `exec_protocol.rs` erhält nur Feld- und Konstantenzeilen (≤ 490). Reihenfolge: S1 (P1) vor S4 (P3) |
| `N/share/core/access_catalog.rs` | neu | S1 PRM-M1 legt an, S4 SCR-M2 ergänzt den Aufruf `register_screen_entries` | Reihenfolge U01 (P1) vor U10 (P4) |
| `N/app/core/share_device_page_ui.rs` | 390 / 14 032 | S1 PRM-M9, S4 SCR-M7 | Reihenfolge U07 (P3) vor U25 (P9); Ziel ≤ 420 |
| `N/app/core/share_room_page_ui.rs` | 338 / 11 689 | S1 PRM-M9, S4 SCR-M7 | wie oben |
| `N/app/core/share_pages_ui.rs` | 140 / 4 657 | S1 PRM-M9 (Tab 7 bleibt), S4 SCR-M2 (Tab 9) | Reihenfolge U07 (P3) vor U10 (P4) |
| `N/app/core/settings_ui.rs` | 133 / 5 128 | S2 DRV-M6, S5 DSK-M3 | Reihenfolge U15 (P5, S5-Abschnitt am Ende der Seite „Integration“) vor U17 (P6, S2-Abschnitt vor dem S5-Abschnitt) |
| `N/app/core/sidebar.rs` | 291 / 12 491 | S2 DRV-M4, S3 VLT-M10 | Reihenfolge U12 (P4) vor U29 (P10) |
| `N/app/core/init.rs` | 475 / 18 171 | nur S5 (DSK-M2 ≤ 3, DSK-M3 ≤ 5 Zeilen) | → 483; kein weiterer Strang |
| `A/service/AccessApprovalNotifier.kt` | neu | S1 PRM-M10 legt an, S4 SCR-M8 setzt die Aktivitäts-Intents der Bildschirmfähigkeiten | Reihenfolge U21 (P7) vor U27 (P9) |
| `A/screen/ScreenControlService.kt` | neu | S4 SCR-M8 legt an, S5 ergänzt `performGlobal` (H3; nach C3 W5 in U40 statt in DSK-M12) | Reihenfolge U27 (P9) vor U40 (P14) |
| `android/app/src/main/AndroidManifest.xml` | 154 / 7 827 | S1 PRM-M10, S2 DRV-M8, S4 SCR-M8, S5 DSK-M9/M11/M12/M13 | in jeder Phase nur das eine Android-Paket der Phase (Tabelle D2.3) |
| `A/ui/AppNav.kt` | 139 / 6 135 | S3 VLT-M11, S4 SCR-M9, S5 DSK-M9/M13 | Reihenfolge U28 (P10, S4) → U31 (P11, S3) → U35 (P12, S5 DSK-M9) → U40 (P14, S5 DSK-M13) (H4) |
| `A/core/NativeBridge.kt`, `A/core/CoreEvents.kt`, `A/ui/settings/SettingsScreen.kt`, `android/app/src/main/res/values/strings.xml` | 21; 58; 195; 14 | S2 DRV-M8, S4 SCR-M8, S5 DSK-M9/M13 | in jeder Phase nur das Android-Paket der Phase |
| `native/installer.nsi` | 341 / 15 909 | S1 PRM-M7 (Deinstallation des Kontodienstes), S4 SCR-M10 (OpenH264-Lizenz), S5 DSK-M3 (`SEC_DESKTOP`, `/DESKTOP=1`, `desktop restart`, `desktop remove`), Lizenzen | nur U36 (P12) |
| `install-linux.sh` | 469 / 14 634 | S2 DRV-M9 (`--with-fuse`), S4 SCR-M5 (`X-KDE-Wayland-Interfaces`), S5 DSK-M3 (`--with-desktop`, `desktop restart`) | nur U36 (P12); eine Usage-Zeile (H9) |
| `README.md`, `docs/RELEASING.md`, `docs/GOTCHAS.md`, `docs/ARCHITEKTUR.md`, `docs/TODO.md`, `docs/oberflaechen/INDEX.md`, `docs/oberflaechen/einstellungen.md`, `docs/superpowers/plans/2026-09-25-android-apk/api.md`, `docs/SHARE_SERVER.md` | 1 364; 1 030; 608; 172; 283; 31; 184; 640; nicht gezählt | alle Stränge (Doku-Meilensteine PRM-M11, DRV-M9, VLT-M13, SCR-M11, DSK-M14, dazu die Doku-Zeilen anderer Meilensteine) | nur die Doku-Pakete U42/U43 (P15); die Strangpakete melden ihren Doku-Bedarf im Bericht |

Dateien, die nur ein Strang ändert, aber über 450 Zeilen haben, regeln die Strangpläne selbst. Im Einzelnen: `N/mount/core/types.rs` (499), `journal.rs` (499), `N/daemon/os/shared/mount_manager.rs` (498), `run_loop.rs` (488), `N/app/core/mount_ui_draft.rs` (467) und `N/cli/drive.rs` (461) in S2; `N/app/core/share_exec_ui.rs` (494) und `N/cli/share/grants_exec.rs` (485) in S1, beide schrumpfen durch Verlagerung in den Zugriffsdialog bzw. durch Delegation; `N/share/core/node.rs` (459 → ≈ 470) in S4.

## C — Entscheidungen

Die Strangentscheidungen stehen in den Abschnitten C der Strangpläne und gelten unverändert, soweit dieser Abschnitt nichts anderes festlegt. Hier stehen nur die strangübergreifenden Verträge, die Entscheidungen zu offenen Punkten, die Auflösung von Widersprüchen und die Integrationsmeilensteine.

### C1 Strangübergreifende Verträge

| Nr | Vertrag | Eigentümer | Nutzer | Typen, Funktionen, IPC, Bridge, MCP (verbindlich) | Pakete |
|---|---|---|---|---|---|
| V1 | Spool und Journal von Tresor-Laufwerken | S2 (Traits), S3 (versiegelte Umsetzungen) | S2-Laufwerkshost | `N/mount/core/spool_storage.rs`: `trait SpoolStorage`, `trait SpoolFile { read_at, write_at, set_len, len, sync_data }`, Klartext-Umsetzung byte-gleich mit dem Bestand. `N/mount/core/path_sealer.rs`: `trait PathSealer { seal, unseal }`, Klartext = Identität. S3: `SealedSpoolFile` (`N/vault/os/shared/sealed_spool.rs`), `JournalPathSealer` (`N/vault/os/shared/journal_sealer.rs`). Ein Spool-Schlüssel je Laufwerk, mit dem Zweckschlüssel `spool-wrap` per AES-KW gewickelt in `spool-key.wrapped`. Textschlüssel = HKDF-SHA256(Spool-Schlüssel, info `se-mount-journal-v1`), Inhaltsschlüssel = HKDF-SHA256(Spool-Schlüssel, info `se-mount-spool-content-v1`). Der Host erhält nur den Spool-Schlüssel über `IpcRequest::Vault { req: VaultIpcRequest::SpoolKey { mount_id } }` (Prüfung mit dem Sitzungstoken des Hosts), nie den Masterkey. Versiegelt werden die Pfadfelder in `journal.jsonl` und `stages.json` (`remote_path`, `original_path`, `quarantine_path`, Pfade der Namenskonfliktabsichten, Stage-Pfade) | U02, U14 |
| V2 | Tresor als Laufwerksquelle | S2 | S3 | `MountSource::Vault { site_id }` in `N/mount/core/mount_source.rs`, gespeichert nur in `<AD>/mounts/registry-2.json` (`{version: 1, mounts: […]}`). Start bei gesperrtem Tresor → Code `vault_locked`. Nach Absturz und Sperre `MountRecovery::RequiresUnlock`; CLI-Status `requires-unlock:<n>` bzw. `failed:vault_locked`, JSON `recovery: "requires_unlock"`. `MountHostRequest::PrepareLock { deadline_ms }` in `N/daemon/os/shared/ipc_protocol_mount_host.rs` über `IpcRequest::MountHostExt { id, session_token, req }`. `MountTerminated { site, mount_id, reason: user_unmount \| external_unmount \| host_exit \| lock }`. `DriveIpcRequest::MountPathFor { location }`. In `N/daemon/os/shared/mount_paths.rs`: `pub fn mount_path_for(location: &str) -> Result<Option<PathBuf>, String>` und `pub fn ensure_mounted_for(location: &str) -> Result<PathBuf, String>`. `MountBackendScheme::Vault` in `ipc_protocol_mount.rs`. Einhängepfade aller Tresor-Laufwerke über `list_mounts` mit `MountStatus::MountedAt { path }` (für S3 K09/K11) | U06, U14, U20 |
| V3 | Ort `vault://<SITE>` | S3 | S2, S5 | Locator `EndpointSpec::Vault` und `EndpointSpec::VaultStore` (`vault-store://`) über `connect::resolve_endpoint`. Einbinden mit `se drive mount vault://<SITE>`, wobei `<SITE>` die ID, ein ID-Präfix mit mindestens 4 Zeichen oder eine eindeutige Bezeichnung ist; gespeichert wird die Site-ID. Kein `se vault mount`, kein `se drive add --vault`. Linux-Einhängeort `$XDG_RUNTIME_DIR/smart-explorer/Tresore/<Name>` (tmpfs geprüft; ohne tmpfs Abbruch); Windows festes lokales Laufwerk mit Buchstabe; Android Provider-Wurzel nur bei entsperrtem Tresor. S5 speichert nur den Wurzel-Locator `vault://<site-id>/` (`vault:<site-id>` in den Anordnungsdateien) | U08, U14, U30, U40 |
| V4 | Gerätebaum für se Desktop | S2 | S5 | `pub fn mount::device_key(&PeerOpenTarget) -> String` (Re-Export in `N/mount/mod.rs`; `direct~<contact_id>` bzw. `room~<room_id>~<device_id>`, jedes Byte außerhalb `[A-Za-z0-9._-]` als `%XX`). `IpcRequest::Drive { token, req: DriveIpcRequest::DevicesStatus }` → `{"enabled","state","mounted","root_path","letter","mount_point","revision","devices":[{"key","name","kind","room","online","path"}]}`. `DriveIpcRequest::DevicesWatch { since }` antwortet nach einer Änderung oder spätestens nach 30 s. S5 ruft beide nur über `request_if_running` aus `N/daemon/os/shared/ipc_peek_client.rs` mit 35 s Lesefrist. `pub(crate) fn show_folders` in `N/mount/os/linux_os/file_manager_open.rs`. Android: Authority `${applicationId}.documents`, Root-ID `devices`, Dokument-IDs `devices/<key>/…`, Ereignis `docs.changed`. Zuständigkeit: S2 schaltet den Gerätebaum und besitzt Buchstaben, GTK-Lesezeichen, KDE-Orte und `smart-explorer-drives.desktop`; S5 besitzt nur den Explorer-Navigationseintrag `{DESKTOP_NAV_CLSID}` mit `TargetFolderPath = root_path`, angelegt nur bei `mounted = true` | U17, U12, U23, U30, U32, U34, U40 |
| V5 | Bildschirm öffnen aus se Desktop und Launcher | S4 | S5 | `smart_explorer --screen-view <ziel-json>` mit `{"relation","relation_id","device_id","source","control"}` (`source: null` = ganzer Bildschirm, Annahme A28 aus S5) und `smart_explorer --screen-grid [--room <id>]`; unter Windows ist das Programm `Smart Explorer.exe`. CLI `se screen view` und `se screen grid` mit `--json` → `{"launched":true,"pid"}`. Android `NavRequest.OpenScreen(target)` und `NavRequest.ShowScreenGrid(roomId)` (S4, `A/ui/AppNav.kt`). Externe Navigationsarten `openScreen` (Wert = Ziel-JSON) und `showScreenGrid` (Wert = Raum-ID oder leer) (S5). S5 bettet weder `ScreenViewWidget` noch `ScreenTileGrid` ein und nutzt `crate::screen::device_states` nicht | U25, U28, U30, U40 |
| V6 | Bedienungshilfe für Launcher-Gesten | S4 (Dienst), S5 (Zugriff) | S5 | `A/screen/ScreenControlService.kt` erhält die statische Funktion `performGlobal(action: Int): Boolean` (≤ 15 Zeilen, `false`, wenn der Dienst nicht läuft). Launcher-Protokoll über `A/service/LauncherBridgeService.kt`: `GLOBAL_ACTION {action: "notifications" \| "lockScreen"}` → `RESULT {ok, reason: null \| "accessibility_off" \| "unsupported"}`. Ohne laufenden Dienst bleiben beide Gesten aus und die Einstellungen nennen den Grund | U27, U40 |
| V7 | Rechte für Bildschirm ansehen und steuern | S1 | S4 | `pub enum Capability { ExecCommands, ExecTerminal, ScreenView, ScreenControl }`, `CapabilityDecision { Off, Ask, Allow }` und `CapabilityState { decision, policy_revision: u64, changed_at: i64 }` in `N/share/core/access_grant.rs`. `prove_relation(&RelationServerHello, &RelationClientHello, remote_node: &str, &ShareAuthState) -> Result<ProvenPrincipal, RelationRejection>` mit Transkript-Präfix `smart-explorer/share-screen/v1\|` für S4. `capability_for(&ShareAuthState, &ProvenPrincipal, Capability, now: i64) -> CapabilityState`. `capability_catalog(&HostFacts) -> Vec<CapabilityEntry>` mit genau `HostFacts { graphical_session: bool, account_helper: HelperStatus }`; S4 liefert `register_screen_entries(&mut Vec<CapabilityEntry>, &HostFacts)` in `N/screen/core/catalog_entries.rs`. Entzug über `RestrictionReason::CapabilityNarrowed(Capability)` und `RestrictionReason::GrantExpired`; S4 hängt `screen_registry().restrict_authorization(epoch, restrictions)` ein. Protokoll: `record_access(AccessEntry { kind: AccessKind::Screen { mode, event }, … })` in `N/share/os/shared/access_audit.rs`. Rückfrage: S1 `access_approval` setzt den S4-Trait `ConsentBroker` um; Fenster `smart_explorer --access-consent <ID>`. IPC `IpcRequest::Access { token, request: AccessIpcRequest::{Approvals, Decide { approval_id, decision: once \| always \| deny }} }`. CLI `se share grants access … --screen-view off\|ask\|allow --screen-control off\|ask\|allow`, JSON `screen_view`/`screen_control`. Bridge `share.setAccess` mit `screenView`/`screenControl`, `share.accessApprovals`, `share.decideAccessApproval`. Eine offene Bildschirm-Rückfrage belegt eine Reservierung der Exec-Registry (2 je Gerät, 8 gesamt). Android: Die Aktionen „Diesmal erlauben“ und „Immer erlauben“ für Bildschirmfähigkeiten sind `PendingIntent.getActivity` auf `A/screen/ScreenConsentActivity.kt`, ab API 34 mit `MODE_BACKGROUND_ACTIVITY_START_ALLOWED`. MCP entscheidet keine Rückfrage (Sperrliste in `N/cli/mcp/cli_tool.rs`). Der Kontodienst ist nie Bildschirm-Host | U01, U04, U07, U09, U10, U21, U27 |
| V8 | Gemeinsamer Stoppmerker-Helfer (DSK-M2) | S5 | S1, S2, S4, S5, Bestand `se mcp` | `N/daemon/os/shared/update_stop.rs`: `UpdateStopBaseline::{arm, requested}`, `is_new_request`, `spawn_update_stop_watch(poll, on_request)`, `EXIT_UPDATE_STOP = 75`; Abfrage jede Sekunde; ein beim Start vorhandener Merker wird ignoriert. GUI-Fenster nutzen `N/app/core/update_stop_ui.rs` und schließen über `ViewportCommand::Close` → `on_exit` → `prepare_for_exit(false)`. **Vollständige Liste der Nutzer:** `se mcp` (Bestand, nutzt danach `is_new_request` des Helfers); jedes GUI-Fenster einschließlich der Modi `--access-consent` (S1), `--screen-indicator`, `--screen-view`, `--screen-grid` (S4) und der Fenster aus `--open-location`/`--open-page` (S5); Wächter `--desktop-shell` und Flächen `--desktop-surface` (S5, Exit 75); Hostprozess `--screen-host-worker` (S4, beendet die Sitzung, Exit 75); Laufwerkshost `--mount-host` unter Windows und Linux (S2: veröffentlichen wie beim Aushängen ohne `--force`, Journal bleibt, Exit 75; Haken in `N/mount/os/shared/host_session.rs`); der Daemon (Bestand, unverändert). **Nicht betroffen**, weil der Updater das Abbild nicht ersetzt: `/usr/local/sbin/smart-explorer-exec` (Linux-Kontodienst und Konto-Einheiten) und die Kopie des Windows-Kontodienstes unter `%ProgramFiles%`. Der bestehende Supervisor `--share-exec-supervisor` bleibt unverändert | U03, U02, U12, U04, U10, U13, U15, U25 |
| V9 | Gemeinsamer Startschalter-Parser (H8) | S5 | S1, S2, S4 | `N/app/core/start_request.rs`: `pub(crate) enum StartRequest` mit genau diesen Startarten: `AccessConsent { id }` (`--access-consent <ID>`), `ScreenIndicator` (`--screen-indicator`), `ScreenHostWorker` (`--screen-host-worker`, nur als einziges Argument), `ScreenView { target_json }` (`--screen-view <JSON>`), `ScreenGrid { room: Option<String> }` (`--screen-grid [--room <ID>]`), `DesktopShell` (`--desktop-shell`), `DesktopSurface { monitor \| gnome_monitor \| layer }` (`--desktop-surface --monitor <SCHLÜSSEL> \| --gnome-monitor <INDEX> \| --layer`), `DesktopJumplistClear` (`--desktop-jumplist-clear`), `OpenLocation { locator }` (`--open-location <LOCATOR>`), `OpenPage { page }` (`--open-page <SEITE>`), `Gui { start_path: Option<PathBuf> }` (Bestand). `--drive-restore` ist keine eigene Startart: Es ergänzt `--sync-daemon` (Startgrund, ausgewertet von S2 in `N/daemon/os/shared/run_start_reason.rs`) und gilt nie als Startpfad. Werte von Schaltern mit Wert gelten nie als Startpfad. Reihenfolge in `run_gui`: Bestandsprüfungen bis `--sync-daemon` unverändert; danach die Hintergrundarten `DesktopShell`, `DesktopSurface`, `DesktopJumplistClear`, `ScreenHostWorker`, `ScreenIndicator` vor der Update-Bestätigung; nach den Updater-Schritten die Fensterarten `AccessConsent`, `ScreenView`, `ScreenGrid`, `OpenLocation`, `OpenPage`, `Gui` | U03 (Parser), R2, R4, R5, R9 (Verdrahtung) |
| V10 | Tresore in se Desktop und Launcher | S3 | S5 | S5-V1 bis S5-V6 aus `strang-tresore.md` Abschnitt E mit der Entscheidung H5. Desktop: `IpcRequest::Vault { token, req: VaultIpcRequest::Status }` über `request_if_running` im bestehenden 900-ms-Takt des Wächters. Android: Der Hauptprozess liest `vault.list` und reicht `{"site","label","state"}` über `LauncherBridgeService` weiter. Kein eigenes Passwortfeld in S5; S5 speichert keine Inhalte, Namen oder Vorschauen aus Tresoren | U11, U30, U40 |
| V11 | Erreichbarkeit ab Anmeldung | Hauptagent (S2 K7, S5 H1) | S1, S2, S4, S5 | Der Startgrund `drive-restore` startet weder Share-Host noch Exec-Host. se Desktop und se Launcher starten den Hintergrunddienst nie selbst (V1 aus `strang-desktop.md` C10). `IpcRequest::ShareSnapshotPeek` (nur `share_worker_peek`) startet in einem laufenden Daemon keinen Share-Host (Annahme A41 aus S5, Signal `desktop_task_peek_does_not_start_share_host`); `ShareSnapshot` aus Statusbefehlen hebt die Einschränkung eines Anmelde-Dienstes auf (Entscheidung des Hauptagenten, Signal `drive_task_status_command_promotes_restricted_daemon`). Ein Bildschirm-Host ist nur erreichbar, wenn der Share-Host aus einem bestehenden Anlass läuft | U06, U15, U30, U40 |
| V12 | MCP-`INSTRUCTIONS` (H7) | Hauptagent | S2, S3, S4, S5 | Bestand 1 887 Byte, Grenze 2 048 Byte, erster Absatz unverändert (≤ 512). Genau drei Ergänzungen (+108 Byte → 1 995 Byte, 53 Byte Reserve): (1) Liste „Tools not loaded yet (…)“ um `, screen` vor `, se` (+8, R6 mit SCR-M4); (2) nach `` `gdrive:///path` `` der Text `` , `vault://<site>/path` (unlocked vault) `` (+40, R9 mit VLT-M9); (3) nach `` updates (`update`) `` der Text `` , drives (`drive`), vaults (`vault`), se Desktop (`desktop`) `` (+60; R4 setzt `` , drives (`drive`) ``, R5 setzt `` , se Desktop (`desktop`) ``, R9 setzt `` , vaults (`vault`) ``, jeweils an der Stelle, die die Endreihenfolge drives → vaults → se Desktop ergibt). S1 ändert die `INSTRUCTIONS` nicht | R4, R5, R6, R9 |
| V13 | Lizenzverzeichnis aller Rust-Crates | Hauptagent (C4) | alle | Datei `third-party/rust-crates/THIRD-PARTY-LICENSES.txt`, eingebettet über `N/licenses/mod.rs`; CLI `se licenses [--json]`; Bridge `app.licenses`; Einzelheiten C4 | U33, U36 |
| V14 | Einstellungsseite „Integration“ | S2, S5 | – | Reihenfolge der Abschnitte: Bestand, dann „Verbundene Geräte als Ordner“ (S2, `N/app/core/settings_devices_ui.rs`), dann „se Desktop“ (S5, `N/app/core/desktop_settings_ui.rs`) | U15, U17 |

### C2 Entscheidungen zu offenen Punkten

Bereits entschieden und unverändert übernommen (Abschnitt „Entscheidungen des Hauptagenten zu offenen Strangpunkten“): S2 K7; S2/S3 P1, P2, P3; S3 OE2, OE3, OE4, OE5; S5 H1 (V1); S4 K6; S4 K3; S1 „Nur lesen“ entfällt (S1 Punkt 1); S1 CI-Umgebungen (S1 Punkt 2); S1 Release-Text (S1 Punkt 4); Lizenzhinweise als Integrationsmeilenstein; eine einzige Task-Suite. S3 OE1 bleibt eine Nutzerentscheidung; bis dahin entfällt der Wiederherstellungsschlüssel und alle Oberflächen zeigen einen Informationstext.

| Punkt | Entscheidung | Begründung |
|---|---|---|
| S5 H2 (Stoppmerker in anderen Strängen) | Jeder Prozess der Liste V8 nutzt den Helfer; DSK-M2 kommt in Phase P1 (U03). Laufwerkshosts beachten den Merker in `host_session.rs`: ausstehende Änderungen veröffentlichen wie beim Aushängen ohne `--force`, Journal behalten, Exit 75. Neue Signale `drive_task_host_session_update_stop_unmounts_and_exits` (Windows in U02, Linux in U12) und `screen_task_host_worker_exits_on_update_stop` (U13) | Jeder langlebige Prozess desselben Abbilds lässt sonst das Update nach 30 s scheitern (gleiche Ursache wie R70). |
| S5 H3 (globale Aktionen der Bedienungshilfe) | Angenommen. Ort und Zeitpunkt: U40 (P14), nicht DSK-M12 (Widerspruch W5) | Eine zweite Bedienungshilfe bräuchte eine zweite Freigabe durch den Nutzer; die vorhandene S4-Bedienungshilfe kann die Aktionen bereits ausführen. |
| S5 H4 (`AppNav.kt`) | Reihenfolge U28 (P10, SCR-M9: `NavRequest.OpenScreen`, `ShowScreenGrid`) → U31 (P11, VLT-M11: `NavRequest.OpenVaultCompare(jobId)`) → U35 (P12, DSK-M9: `NavRequest.ShowLauncherSettings`, Art `showLauncherSettings`, Überladung `intentFor(context, request, token)`) → U40 (P14, DSK-M13: externe Arten `openScreen`, `showScreenGrid`). In jeder dieser Phasen gehört die Datei dem genannten Paket allein | Die externen Arten aus DSK-M13 bilden auf die `NavRequest`-Typen aus SCR-M9 ab, also muss SCR-M9 vorher fertig sein. |
| S5 H5 (Tresor-Aktualisierung ohne Push-Kanal) | Voreinstellung angenommen: Der Wächter fragt `Vault::Status` im bestehenden 900-ms-Takt über `request_if_running` ab; S3 baut keine `VaultIpcRequest::Watch` | Die Abfrage läuft über Loopback in einem ohnehin laufenden Takt; eine Wache würde den S3-Vertrag erweitern, ohne eine Funktion hinzuzufügen. |
| S5 H6 (`image`) | Eine Zeile in `[target.'cfg(not(target_os = "android"))'.dependencies]`: `image = { version = "0.25.10", default-features = false, features = ["png", "jpeg", "gif", "webp", "bmp"] }` (Vereinigung aus S3 C6.3 und S5 C8); eingetragen von R10 | Zwei Zeilen für dasselbe Paket sind in Cargo nicht möglich, und die Vereinigung bringt genau die sieben Pakete aus OE5. |
| S5 H7 (MCP-`INSTRUCTIONS`) | Budget nach V12: genau drei Ergänzungen, 1 995 von 2 048 Byte | Jeder Strang bekommt einen Hinweis auf seinen Befehl, und die Hostgrenze bleibt mit 53 Byte Reserve eingehalten. |
| S5 H8 (Startschalter in `N/lib.rs`) | Ein Parser nach V9 in `N/app/core/start_request.rs`, gebaut in U03 (P1) mit allen Startarten aller Stränge; die Verdrahtung in `lib.rs` setzt das R-Paket der Phase, in der die Zielfunktion entsteht (R2 `--access-consent`, R5 `--screen-host-worker` und Desktop-Arten, R4 `--screen-indicator`, R9 `--screen-view`/`--screen-grid`) | Eine Entscheidungsfunktion mit einer Tabelle aller Aufrufformen macht die Erhaltungsprüfung `desktop_task_gui_start_order_unchanged` für alle Stränge auf einmal möglich. |
| S5 H9 (`install-linux.sh`) | Usage-Zeile `usage: $0 [--dry-run] [--cli-only] [--with-fuse] [--with-desktop]`. Reihenfolge: Programme installieren → `smart-explorer.desktop` mit der S4-Zeile `X-KDE-Wayland-Interfaces=org_kde_plasma_window_management` schreiben → S2-Laufzeitprüfung von `fusermount3` (Paketbefehl ausgeben; `fuse3` nur mit `--with-fuse` installieren) → bei `--with-desktop` `"$CLI_BIN" desktop enable` → nach jeder Installation der Desktop-App `"$CLI_BIN" desktop restart`. `--with-desktop` zusammen mit `--cli-only` → Exit 2 mit „se Desktop braucht die Desktop-App“; `--with-fuse` ist mit `--cli-only` erlaubt. Einziger Bearbeiter: U36 | `se drive mount` ist auch ohne GUI nutzbar, se Desktop nicht; die Reihenfolge stellt sicher, dass `desktop restart` die frisch installierten Programme startet. |
| S5 H10 (Suite und gemeinsame Umgebungen) | Angenommen: S5 liefert nur Stufen; die KWin-Umgebung teilen sich S4 K3 und S5 im Job `linux-wayland`; den Helfer `N/child_lifetime/` gibt es nicht (S4 behält `worker_job.rs`/`worker_death.rs`) | Eine Umgebung je Werkzeug spart Runner-Zeit, und keine Entscheidung hängt an einem gemeinsamen Lebenszeit-Helfer. |
| S3 K13 (zwei unbelegte Annahmen zur Bildschirmsperre) | Der Plan bleibt: Windows über ein verborgenes Fenster mit `WTSRegisterSessionNotification`, Linux über `LockedHint` und das Signal `Lock`. Die Suite prüft die Verdrahtung (`vault_task_windows_wts_window_dispatch`, Stufe `vault_linux` mit Test-logind) und meldet die Zustellung durch das Betriebssystem als „nicht ausgeführt“ mit Grund. Die Oberflächen nennen beim Schalter „Bei Bildschirmsperre sperren“ den Satz „Unter Linux nur, wenn die Desktopumgebung die Sitzungssperre meldet.“. `docs/TODO.md` erhält `VAULT-SCREENLOCK-VERIFY` mit den zwei Fakten A1 und A2 aus `strang-tresore.md` F als Handtest | Beide Annahmen betreffen nur einen von vier Sperrauslösern; Leerlauf, Ruhezustand und ausdrückliches Sperren wirken unabhängig davon, und ein ungeprüftes Verhalten wird nach AGENTS.md ehrlich benannt. |
| S1 Punkt 3 (Reihenfolge an `ipc_protocol.rs`) | Zuerst die Ausgliederung der Laufwerkshost-Typen nach `ipc_protocol_mount.rs` (R1, P1). Danach `Access` (S1) und `Drive` (S2) in R2 (P2), `MountBackendScheme::Vault` in R3 (P3), `Vault` mit `Err.code` (S3), `Screen` (S4) und das Feld `location` (S2) in R4 (P4), `MountHostExt` (S2) in R5 (P5). Jeder Strang bekommt genau ein Variantenpaar, die Untertypen liegen in Strangdateien | So bleibt die Datei unter 500 Zeilen, und S1 kommt vor S4, wie SCR-M2 es braucht. |
| S1 A10 (Runner `ubuntu-22.04`) | Job `linux-2204` mit `runs-on: ubuntu-22.04`. **Annahme:** Das Abbild ist am Tag des Suite-Laufs verfügbar. Wirkung, falls falsch: Der Job bleibt in der Warteschlange; der Hauptagent bricht ihn nach 30 min ab und meldet `perm_task_exec_linux_ubuntu2204_starts` als „nicht ausgeführt“. Die statische Prüfung `perm_task_systemd_unit_props_v249_compatible` läuft unabhängig davon in `linux-core` | Der Bestandsdefekt `ExitType=main` betrifft genau systemd 249; nur dieses Abbild prüft ihn real. |
| S2 DRV-M9 (`INSTRUCTIONS`) | durch V12 erledigt | gleiche Längengrenze für alle Stränge |
| S4 SCR-M12 (eigener Workflow `screen-task.yml`) | entfällt; Stufen in `rechte-desktop-tresore-task.yml` (D3) | Entscheidung „Eine einzige Task-Suite“ |
| S4 SCR-M10 „Nicht in S4: RUST-CRATE-LICENSES“ | im Integrationsmeilenstein C4 (U33); SCR-M10 wird verkleinert (Widerspruch W6) | Die Lücke betrifft alle Crates aller Stränge. |
| L1 (neu: `native/Cargo.lock`) | Kein Paket ändert `Cargo.lock` von Hand; die R-Pakete ändern nur `native/Cargo.toml`. Der Job `prepare` der Suite löst die Abhängigkeiten ohne `--locked` auf (bestehende Pins bleiben), prüft mit `cargo audit` und lädt die aufgelöste `Cargo.lock` als Artefakt hoch; alle übrigen Jobs übernehmen sie vor dem Bau. Der Hauptagent committet sie nach dem ersten Suite-Lauf (`chore(lock): …`). **Annahme:** Ein Bau in `build.yml` mit veraltetem Lock bei einem Zwischen-Push ist ein Entwicklungsstand und kein Gate | Ohne lokalen Cargo-Aufruf (AGENTS.md) entsteht ein gültiger Lock nur im Remote-Lauf. |
| L2 (neu: erste Lizenzdatei) | Wie L1: Der Job `prepare` erzeugt `THIRD-PARTY-LICENSES.txt`, vergleicht sie mit dem eingecheckten Stand (`licenses_task_generated_file_current`) und lädt sie hoch; der Hauptagent committet sie zusammen mit dem Lock. Bis dahin baut `native/build.rs` ohne die Datei ein ausgewiesenes Fehlen (C4) | Die Erzeugung braucht `cargo-about` und `cargo metadata`, die lokal nicht laufen dürfen. |
| Z1 (neu: Teilung von PRM-M8/PRM-M9) | U07 (P3) baut PRM-M8 und PRM-M9 ohne die Konten-Oberflächen. Die Konten-Oberflächen gehen nach U24 (P8, zusammen mit PRM-M7): `se share exec accounts*` (`N/cli/share/exec_accounts.rs`), „Konten dieses Geräts“ (`N/app/core/share_access_accounts_ui.rs`) und die Signale `perm_task_cli_accounts_json`, `perm_task_gui_accounts_page_states`, `perm_task_gui_setup_without_pkexec_shows_sudo_command`. Bis U18/U24 liefert `AccessIpcRequest::Accounts` die Kontoprofile `own` und, beim Linux-root-Worker, die Bestandskonten von `run_as` | SCR-M2 braucht nur Rechtezeilen in CLI und Dialog; mit vollem PRM-M8/M9 nach PRM-M7 käme S4 vier Phasen später. |
| Z2 (neu: Sammeldateien und `ipc_host_ext.rs`) | wie B2 | Sie halten die Hub-Dateien unter 500 Zeilen und erlauben drei Pakete je Phase ohne Dateikonflikt. |
| Z3 (neu: Hub-Dateien) | Die Dateien der Liste D2.2 ändern nur R-Pakete bzw. das Android-Paket der Phase (D2.3); jedes Umsetzungspaket meldet seinen Registrierungsbedarf als genaue Zeilen im Bericht | Jede Datei gehört je Phase genau einem Paket. |
| Z4 (neu: Inventurdateien) | Die Inventurdatei eines Strangs legt sein erstes Paket mit Oberflächen an und pflegt sie im selben Commit (C6). Laufen zwei Pakete desselben Strangs in einer Phase (P9: U25/U27; P12: U34/U35; P13: U37/U38), pflegt nur das jeweils erste die Datei; das zweite meldet seine Zeilen, und das Doku-Paket trägt sie ein | Eine Datei, ein Eigentümer je Phase. |

### C3 Widersprüche zwischen Strangplänen und ihre Auflösung

| Nr | Widerspruch | Auflösung |
|---|---|---|
| W1 | S1 PRM-M10 nennt S4 SCR-M8 als Abhängigkeit (`ScreenConsentActivity`); S4 SCR-M8 nennt S1 PRM-M10 als Abhängigkeit | U21 (P7) baut PRM-M10 ohne Aktionsziele für Bildschirmfähigkeiten; der Katalog zeigt vor SCR-M8 auf Android keine Bildschirmzeilen. U27 (P9) setzt in `A/service/AccessApprovalNotifier.kt` die Aktivitäts-Intents. Das Signal `perm_task_android_screen_approval_actions_are_activity_intents` läuft in der Suite nach beiden |
| W2 | S4 nennt die IPC-Formen `AccessApprovals`/`DecideAccessApproval` und „S1 PRM-M3 fügt vier Varianten hinzu“; S1 legt eine Sammelvariante `IpcRequest::Access` mit `AccessIpcRequest::{MutateGrant, Approvals, Decide, ReportUndeliverable, Accounts}` und eine Antwortvariante fest | Es gilt die S1-Form (V7). S4 ruft `IpcRequest::Access { request: AccessIpcRequest::Decide {…} }` |
| W3 | S2 DRV-M7 braucht `MountHostRequest::PrepareLock` und den Kanal `IpcRequest::MountHostExt`; beide legt der Strangplan erst in DRV-M6 an, das nach DRV-M7 kommen müsste | U14 (DRV-M7, P5) legt `N/daemon/os/shared/ipc_protocol_mount_host.rs` mit `MountHostRequest::PrepareLock { deadline_ms }` an, R5 setzt `IpcRequest::MountHostExt { id, session_token, req }`. U17 (DRV-M6, P6) ergänzt `ChildBackend` und `DeviceWatch` |
| W4 | S2 DRV-M9 legt `docs/oberflaechen/laufwerke.md` „neu“ an, und S5 DSK-M14 legt `desktop-launcher.md` „neu“ an; beide Pläne verlangen aber, dass jede frühere Oberflächenänderung die Datei im selben Commit pflegt | Angelegt wird die Datei vom ersten Paket mit Oberflächen (laufwerke.md: U06; desktop-launcher.md: U15). DRV-M9 und DSK-M14 prüfen und vervollständigen sie (C6) |
| W5 | S5 DSK-M12 plant die Gesten „Benachrichtigungen“ und „Sperren“ über `GLOBAL_ACTION` in `A/service/LauncherBridgeService.kt`; diese Datei und `BridgeProtocol.kt` entstehen erst in DSK-M13 | Beide Gesten, `performGlobal` (H3) und die Signale `launcher_task_gestures_off_without_accessibility` und `launcher_task_gestures_with_accessibility` wandern nach U40 (DSK-M13). U38 baut DSK-M12 ohne diese zwei Gesten; „Wisch nach oben“ bleibt in U38 |
| W6 | S4 SCR-M10 liefert eigene Lizenzdateien für `openh264`/`openh264-sys2`, `ashpd`, `wide` und `bzip2` und das Signal `screen_task_license_notices_complete`; der Integrationsmeilenstein C4 erzeugt die Texte aller Crates | Die Crate-Texte kommen allein aus C4. SCR-M10 behält: `third-party/openh264/BINARY_LICENSE.txt` (Cisco v1.0 wörtlich), `N/screen/core/licenses.rs` nur für den Cisco-Text, `se screen codec licenses` und die Ansicht **Bildschirm → Kodierung → Lizenzen** (Cisco-Text plus die Einträge `openh264` und `openh264-sys2` aus dem Lizenzverzeichnis), außerdem `screen_task_cisco_license_verbatim` und `screen_task_android_has_no_openh264`. Entfallen: `third-party/openh264-rs/LICENSE`, `third-party/ashpd/LICENSE`, `third-party/wide/LICENSE`, `third-party/bzip2-rs/LICENSE` und deren Installer-Zeilen. `screen_task_license_notices_complete` wird durch `licenses_task_every_shipped_crate_has_text` ersetzt; `screen_task_installer_lists_screen_licenses` prüft die Cisco-Datei und das Lizenzverzeichnis |
| W7 | S2 nennt `N/main.rs` für `--drive-restore`, S4 und S5 nennen `N/lib.rs`; der Windows-Autostart startet das GUI-Programm mit `--sync-daemon`, der Linux-Autostart das aufrufende Programm (`se` oder `smart_explorer`) | `--drive-restore` wirkt überall dort, wo `--sync-daemon` ausgewertet wird. **Annahme:** Das ist `run_gui` in `N/lib.rs` und `N/bin/se.rs`. R2 setzt den Startgrund in beiden und nur dann in `N/main.rs`, wenn diese Datei die Argumente selbst auswertet |
| W8 | S3-V3 verlangt „Aktualisierung ohne Abfrageschleife“; S5 fragt im 900-ms-Takt ab | Entscheidung H5 |
| W9 | S4 E2 bietet `ScreenViewWidget`/`ScreenTileGrid` und verlangt `desired_maximum_frame_latency = Some(1)` in den `NativeOptions` von S5; S5 E2 bettet nichts ein | Es gilt S5: keine Einbettung, keine Vorgabe an die `NativeOptions` von S5. S4 behält die Widgets für den eigenen Betrachter |
| W10 | S1 PRM-M3 ändert `N/daemon/os/shared/exec_ipc.rs` (455 Zeilen) für die Weiterleitung; S3 VLT-M4 ändert `ipc_host.rs` (479 Zeilen) | Beide Weiterleitungen laufen über `ipc_host_ext.rs` (B2); `exec_ipc.rs` bleibt unverändert |
| W11 | S1 PRM-M8 hängt laut Strangplan von PRM-M7 ab; S4 SCR-M2 braucht PRM-M8 | Entscheidung Z1 |
| W12 | S4 SCR-M5 und S5 C8 ändern beide `install-linux.sh`, S2 DRV-M9 ebenfalls, und S5 DSK-M3 ändert `native/installer.nsi`, ebenso S1 PRM-M7 und S4 SCR-M10 | Ein Paket U36 (P12) für beide Dateien (H9, C5) |
| W13 | S5 C7 ordnet `desktop_task_installer_static` und `desktop_task_install_linux_static` DSK-M3 zu; die Dateien ändert erst U36 | Beide Signale gehören zu U36; DSK-M3 (U15) liefert nur die Befehle `se desktop enable/restart/remove`, die der Installer aufruft |
| W14 | S3 B12: Der libtest-Filter `vault_task_` trifft auch `mount_vault_task_*` | Die Suite wählt Tests über genaue Namenslisten je Stufe (D3) und schließt `mount_vault_task_` ausdrücklich aus |

### C4 Integrationsmeilenstein „Lizenzhinweise aller Rust-Crates“ (INT-LIC, Paket U33, Installer-Teil U36)

**Bestandsfehler:** Das Release verteilt Lizenztexte nur für Dokany (`native/installer.nsi:134-139`) und die eigene MIT-Lizenz; die Texte der statisch gelinkten Crates fehlen in Installer, Linux-Payload, Update-Feed und APK (S4 NR N25, S2 NR N25).

**Werkzeug:** `cargo-about` (Lizenz MIT OR Apache-2.0; nur CI-Werkzeug, wird nicht ausgeliefert), installiert mit `cargo install cargo-about --locked --version <X.Y.Z>`. `<X.Y.Z>` ist die neueste Version, die am Tag der Umsetzung mindestens 14 Tage alt ist; U33 trägt sie fest in `native/rust-licenses.ps1` ein. **Annahme:** Diese Version unterstützt in `about.toml` die Schlüssel `targets`, `ignore-build-dependencies`, `ignore-dev-dependencies` und `accepted` sowie Handlebars-Vorlagen mit Textausgabe. Wirkung, falls falsch: U33 nimmt die nächstältere Version mit diesen Schlüsseln und nennt sie im Bericht. Begründung: Auswertung der SPDX-Ausdrücke und Zuordnung der Lizenzdateien selbst zu bauen ist nicht schneller (E7).

**Konfiguration und Ausgabe:**
- `third-party/rust-crates/about.toml`: Ziele `x86_64-pc-windows-gnu`, `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, `aarch64-linux-android`, `x86_64-linux-android`; Build- und Entwicklungsabhängigkeiten ausgenommen; `accepted` = Liste der im Lock vorkommenden zulässigen Lizenzen. Eine Lizenz außerhalb der Liste bricht die Erzeugung ab.
- `third-party/rust-crates/about.hbs`: Textvorlage. Der Kopf enthält Werkzeug und Version, das Datum der Erzeugung und je Manifest die Liste `Name Version Lizenzausdruck`. Danach folgt jeder verschiedene Lizenztext einmal, mit der Liste der Crates, für die er gilt, und den NOTICE-Dateien der Crates, die eine solche mitbringen.
- Ausgabedatei (eingecheckt): `third-party/rust-crates/THIRD-PARTY-LICENSES.txt`.
- Erfasste Manifeste: `native/Cargo.toml` mit dem Workspace-Mitglied `native/android-bridge`, dazu `native/explorer-command/Cargo.toml` (Kontextmenü-DLL) und `share-server/Cargo.toml` (Share-Server-Payloads). Die Datei enthält die Vereinigung aller Crates aller Ziele.
- Skript `native/rust-licenses.ps1 -Mode Generate|Check [-Out PFAD]` (PowerShell 7, läuft auf `windows-2025` und auf Ubuntu-Runnern; **Annahme:** `pwsh` ist auf `ubuntu-24.04` vorinstalliert, sonst installiert der Job `prepare` es mit `sudo apt-get install -y powershell` aus der Microsoft-Paketquelle des Runners): `Generate` schreibt die Datei; `Check` erzeugt in ein Temp-Verzeichnis und bricht mit Exitcode 1 und der Liste der Unterschiede ab, wenn die eingecheckte Datei fehlt oder abweicht.

**Ort im bestehenden Release-Ablauf (kein neuer Release-Weg):** `native/publish-release-local.ps1` ruft in seiner Vorprüfung vor dem ersten Bau `native/rust-licenses.ps1 -Mode Check` auf und installiert `cargo-about` in der festen Version, falls es fehlt. **Annahme:** Das Skript hat eine Vorprüfung vor dem Bau, in die der Aufruf passt. Wirkung, falls falsch: U33 legt den Aufruf unmittelbar vor den ersten Bauschritt und nennt die Zeile im Bericht. `build.yml` ändert sich nicht. Eine veraltete Datei bricht das Release also vor dem Bau ab, statt veraltete Texte zu verteilen. Erzeugt wird die Datei nur in der Suite (L2) und eingecheckt vom Hauptagenten.

**Einbettung und Oberflächen:**
- `native/build.rs` (Bestand; U33 ergänzt): Liegt die Datei vor, gibt das Skript `cargo:rustc-cfg=se_crate_licenses` und `cargo:rerun-if-changed=../third-party/rust-crates/THIRD-PARTY-LICENSES.txt` aus. **Annahme:** `native/build.rs` existiert, weil `native/Cargo.toml:262` einen Abschnitt `[build-dependencies]` hat. Wirkung, falls falsch: U33 legt `native/build.rs` neu an und setzt `build = "build.rs"` über R11.
- `N/licenses/mod.rs` (neu, ≤ 200 Zeilen): `pub(crate) fn text() -> Option<&'static str>` (`include_str!` nur mit `se_crate_licenses`), `pub(crate) fn crates() -> Vec<CrateLicense { name, version, license }>` (aus dem Kopf der Datei), `pub(crate) fn sha256() -> Option<String>`. Fehlt die Datei in einem Entwicklungsbau, melden alle Oberflächen „Lizenzverzeichnis fehlt in diesem Entwicklungsbau“. Ein Release ohne Datei verhindert `-Mode Check`.
- CLI `se licenses [--json]` (`N/cli/licenses.rs`, neu): Text = Inhalt der Datei; JSON = `{"embedded":bool,"sha256":"…","crates":[{"name","version","license"}]}`; Exit 1 mit Grund, wenn nicht eingebettet. MCP über das Werkzeug `se`.
- Desktop: Einstellungen → Seite „Updates“ unter der Versionsanzeige erhält den Knopf „Lizenzen von Drittanbietern…“, der ein Fenster mit dem Text und einer Suche öffnet (`N/app/core/licenses_ui.rs`, neu). **Annahme:** Die Seite „Updates“ liegt in einer Datei, die in P11 kein anderes Paket ändert; U33 nennt die Datei im Bericht.
- Android: Bridge `app.licenses {}` → `{"embedded","sha256","crates":[…],"text"}` (`N/mobile/os/shared/domains/app_licenses.rs`, neu; Präfix `app` in `domains/mod.rs` über R11). `A/ui/settings/AboutScreen.kt` (Lizenzstelle der App) erhält den Eintrag „Lizenzen der Rust-Bibliotheken“; dazu `A/api/LicensesApi.kt` (neu).
- Installer (U36): `SetOutPath "$INSTDIR\licenses\Rust-Crates"` und `File "${__FILEDIR__}\..\third-party\rust-crates\THIRD-PARTY-LICENSES.txt"` nach den Dokany-Zeilen; im Deinstallationsteil die passenden `Delete` und `RMDir "$INSTDIR\licenses\Rust-Crates"` vor `RMDir "$INSTDIR\licenses"`.

**Abdeckung der Artefakte:**

| Artefakt | Wie die Texte enthalten sind |
|---|---|
| Windows-Installer | Datei unter `licenses\Rust-Crates\` und eingebettet in `Smart Explorer.exe` und `se.exe` |
| Linux-Payload (`install-linux.sh` und Feed-Dateien) | eingebettet in `smart_explorer` und `se` (`se licenses`) |
| Update-Feed (16 Dateien unverändert) | eingebettet in die ausgelieferten Programme; ein Update ersetzt nicht die Installer-Datei, wohl aber den eingebetteten Text |
| Android-APK | eingebettet in `libsmart_explorer_android.so`, angezeigt über `app.licenses` in der Lizenzstelle |

Keine neue Feed-Datei, kein neues Release-Asset (Erhaltungsfall F6 aus S5).

**Neue Crates dieses Stapels (Prüfliste für `licenses_task_new_batch_crates_listed`):**

| Strang | Crate (Version) | Lizenz laut Strangplan | Ziel |
|---|---|---|---|
| S2 | `fuser` 0.18.0 | MIT | Linux |
| S3 | `aes-siv` 0.8.0, `aes-kw` 0.3.1 | **Annahme:** Apache-2.0 OR MIT wie die übrigen RustCrypto-Crates; der Test liest das Lizenzfeld aus `cargo metadata` | alle |
| S3/S5 | `zune-jpeg`, `zune-core` | MIT OR Apache-2.0 OR Zlib | Windows, Linux |
| S3/S5 | `image-webp` | MIT OR Apache-2.0 | Windows, Linux |
| S3/S5 | `quick-error` | MIT/Apache-2.0 | Windows, Linux |
| S3 | `gif`, `weezl`, `color_quant` | **Annahme:** `gif` und `weezl` MIT OR Apache-2.0, `color_quant` MIT (in den Strangplänen ohne Angabe); geprüft durch den Test | Windows, Linux |
| S5 (neu direkt, bereits im Lock) | `image` 0.25.10 (MIT OR Apache-2.0), `smithay-client-toolkit` 0.19.2, `wayland-client` 0.31.14, `wayland-protocols-wlr` 0.3.12, `calloop` 0.13.0, `xkbcommon-dl` 0.4.2, `x11rb` 0.13.2, `egui-wgpu` 0.29.1, `raw-window-handle` 0.6.2 | Lizenzfeld aus `cargo metadata` | Linux bzw. Windows und Linux |
| S5 (besondere Pflicht) | `winit` 0.30.13 | Apache-2.0; eine NOTICE-Datei des Crates wird mit ausgegeben (`licenses_task_winit_notice_included`; hat das Crate keine, meldet der Test das als Befund) | Windows, Linux |
| S4 | `ashpd` 0.13.13 | MIT | Linux |
| S4 | `openh264` 0.9.9, `openh264-sys2` 0.9.9 | BSD-2-Clause | Windows, Linux |
| S4 | `wide` | Annahme A16 aus S4: Zlib OR Apache-2.0 OR MIT | Windows, Linux |
| S4 | `bzip2` 0.6.1 | MIT OR Apache-2.0 | Windows, Linux |
| S4 | `libbz2-rs-sys` | bzip2-1.0.6 | Windows, Linux |
| S4 (neu direkt, bereits im Lock) | `lz4_flex` 0.14.0 (MIT), `libloading` 0.8.9 (ISC), `png` 0.18.1 (MIT OR Apache-2.0), `wayland-protocols` 0.32.12, `wayland-protocols-plasma` 0.3.12 | Lizenzfeld aus `cargo metadata` | wie S4 C8 |
| S1 | keine neue Crate. **Annahme:** PRM-M7 braucht nur zusätzliche `windows-sys`-Features (LSA, Dienststeuerung, NetUser, Profile) | – | Windows |

Die Cisco-Binärlizenz von OpenH264 ist keine Crate-Lizenz und bleibt bei SCR-M10 (W6). Abnahmesignale: `licenses_task_generated_file_current`, `licenses_task_every_shipped_crate_has_text`, `licenses_task_new_batch_crates_listed`, `licenses_task_winit_notice_included`, `licenses_task_embedded_text_matches_file`, `licenses_task_cli_json_shape`, `licenses_task_bridge_app_licenses`, `licenses_task_installer_installs_file` (U36), `licenses_task_release_wrapper_checks_licenses` (statisch: `rust-licenses.ps1 -Mode Check` steht in `publish-release-local.ps1` vor dem ersten Bauaufruf).

**Danach offen (Eintrag `RUST-CRATE-LICENSES` in `docs/TODO.md`):** Das eigenständige Share-Server-Payload (`se-share-server`) und die Kontextmenü-DLL tragen den Text nicht selbst. Ihre Crates stehen in der Datei, die mit der App und dem Installer verteilt wird; ein eigener Ausgabeweg (`se-share-server --licenses`) fehlt. Ob die APK die Hinweise ihrer Gradle-Abhängigkeiten zeigt, ist hier nicht geprüft (Lücke, ebenfalls im Eintrag).

### C5 Release, Installer, Doku und TODO

**Installer und Linux-Skript (einziger Bearbeiter U36, P12):**

| Datei | Änderung | Herkunft | Abnahmesignal |
|---|---|---|---|
| `native/installer.nsi` | `Section /o "se Desktop (optional)" SEC_DESKTOP` nach `SEC_MAIN` mit `ExecWait '"$INSTDIR\se.exe" desktop enable --json' $0` (Exitcode ≠ 0 → Meldung nur interaktiv, dann weiter); stiller Schalter `/DESKTOP=1` in `.onInit` nach dem Muster `/INSTALLDOKANY=1`; am Ende von `SEC_MAIN` `ExecWait '"$INSTDIR\se.exe" desktop restart'` | S5 C8 | `desktop_task_installer_static` |
| `native/installer.nsi` | Deinstallation: nach dem bestehenden `taskkill` und vor `--unregister` `ExecWait '"$INSTDIR\se.exe" desktop remove' $1` (Exitcode ≠ 0 → `DetailPrint` und weiter) | S5 C8 | `desktop_task_installer_static` |
| `native/installer.nsi` | Deinstallation: Entfernen des Windows-Kontodienstes und seiner Kopie unter `%ProgramFiles%` mit einer UAC-Abfrage (`se.exe share exec accounts remove` erhöht); bei Ablehnung `DetailPrint` mit Dienstname und Ordner; `se-remote` nur auf ausdrückliche Wahl | S1 PRM-M7, D.5 | `perm_task_installer_uninstall_removes_service_static` (neu, statisch) |
| `native/installer.nsi` | `licenses\OpenH264\BINARY_LICENSE.txt` und `licenses\Rust-Crates\THIRD-PARTY-LICENSES.txt` mit `Delete`/`RMDir` im Deinstallationsteil | S4 SCR-M10 (verkleinert, W6), C4 | `screen_task_installer_lists_screen_licenses`, `licenses_task_installer_installs_file` |
| `install-linux.sh` | Optionen `--with-fuse` und `--with-desktop`, Usage-Zeile und Reihenfolge nach H9, Zeile `X-KDE-Wayland-Interfaces=org_kde_plasma_window_management` im Heredoc von `smart-explorer.desktop` (`install-linux.sh:392-403`) | S2 DRV-M9, S4 SCR-M5, S5 C8 | `drive_task_install_linux_dry_run_with_fuse`, `desktop_task_install_linux_static`, `screen_task_install_linux_kde_interfaces_line` (neu, statisch) |

**Release-Text v0.5.183 (Hauptagent, Release-Notiz):** (1) Linux-Exec scheiterte bis v0.5.182 auf Ubuntu 22.04 (systemd 249) am Bestandsdefekt `ExitType=main`; behoben. (2) Der Linux-root-Worker lehnt abgelaufene Konten und erzwungenen Passwortwechsel jetzt über PAM ab. (3) Auf Windows-Laufwerken erscheint ein nicht einzeln abgebildeter Fehler nicht mehr als „Zugriff verweigert“, sondern als E/A-Gerätefehler; Offline-Netzfehler erscheinen als „Netzwerkort nicht erreichbar“. (4) Neu: Zugriffsrechte mit Konto, Rückfrage und Ablauf; Kontodienst für Linux und Windows; Linux-Laufwerke über FUSE; Gerätebaum; Android-Dokumentanbieter; Cryptomator-Tresore mit Tresor-Sync und Vergleich; Bildschirm ansehen und steuern; se Desktop (abschaltbar) und se Launcher (standardmäßig aus); Lizenzverzeichnis aller Rust-Crates. (5) Ungeprüft und so benannt: Media-Foundation- und GPU-Kodierer (K6), KDE-Steuern, falls KWin im Runner nicht startet (K3), Fedora-SELinux (S1 A5), Windows 11 Home und Microsoft-/Entra-Konten (S1 A6, A7), die Zustellung der Bildschirmsperre (S3 K13) und jede Prüfung, die die Suite als „nicht ausgeführt“ meldet.

**Doku (einzige Bearbeiter U42 und U43, P15):**

| Datei | Inhalt | Herkunft | Paket |
|---|---|---|---|
| `README.md` | Neue Fettabsätze im Stil „**Thema (ab 0.5.183):**“: Zugriffsrechte und Kontodienst mit Plattformgrenzen und der Verhaltensänderung des root-Workers (S1); Laufwerke unter Linux, Gerätebaum, Android-Dokumentanbieter und die Windows-Entscheidung „Dokany bleibt“ samt Begründung (S2, E10); Tresore mit Interop-Aussagen (S3); Bildschirm mit dem Abschnitt OpenH264 und dem Verweis auf `third-party/openh264/BINARY_LICENSE.txt` (S4); se Desktop und se Launcher mit der Tabelle „Desktop anzeigen“ je Fenstermanager (S5). Abschnitt „## ⬇️ Installieren“: `--with-fuse`, `--with-desktop`, Installer-Abschnitt „se Desktop (optional)“, `/DESKTOP=1`. Abschnitt „## ⚠️ Lizenz & Hinweis“: Verweis auf `se licenses`, die Installer-Datei und die Lizenzstelle der App | PRM-M11, DRV-M9, VLT-M13, SCR-M10/M11, DSK-M14, C4 | U42 |
| `docs/RELEASING.md` | Neben „Pinned Dokany installer dependency“ ein Abschnitt „Lizenztexte der Rust-Crates“ (Skript, Prüfung im Wrapper, Erzeugung nur in der Suite); Satz zu `fuse3` als Laufzeitvoraussetzung; der Kontodienst ist nicht Teil des Feeds; Installer-Abschnitt `/o` mit `/DESKTOP=1`, `desktop restart`, `desktop remove`; Wiederanlauf von se Desktop nach dem Update; keine neuen Assets (20) und keine neuen Feed-Dateien (16) | PRM-M6, DRV-M9, SCR-M10, DSK-M14, C4 | U42 |
| `docs/GOTCHAS.md` | S4U ohne Netzanmeldedaten, Sitzung 0, PAM-Dienst ohne `pam_systemd`, `ExitType` auf v249, Programmkopie (S1); FUSE-Grenzen und Windows-Fehlerabbildung (S2); PipeWire-Laden, OpenH264-Pins, WGC-Interop-Fabriken, Uni-Stream-Zählplätze, Relay-Grenze (S4); 24H2-Desktophierarchie, `HWND_BOTTOM`-Unterklasse, GNOME-Varianten, KWin-Namensraum, Layer-Tastatur, Lesen ohne Dienststart, Stoppmerker für jeden langlebigen Prozess (S5) | PRM-M11, DRV-M9, SCR-M11, DSK-M14 | U42 |
| `docs/ARCHITEKTUR.md` | Zeile „Exec-Host“ (S1), Einhängen je Betriebssystem (S2), Module `cryptomator/` und `vault/` (S3), `screen/` (S4), `desktop/` und Launcher-Prozess (S5), `licenses/` (C4) | alle Doku-Meilensteine | U42 |
| `docs/SHARE_SERVER.md` | Relay verwirft bei voller Ziel-Warteschlange; Video-Wächter | SCR-M11 | U42 |
| `docs/oberflaechen/einstellungen.md` | Ausnahme „Laufwerke“ ersetzt durch „Laufwerke: Windows und Linux; Android: Dokumentanbieter (Plattformgrund)“; Abschnitt „Integration“ um „Verbundene Geräte als Ordner“ und „se Desktop“ | DRV-M9, DSK-M14 | U42 |
| `docs/superpowers/plans/2026-09-25-android-apk/api.md` | Bridge-Methoden `share.setAccess`, `share.accessApprovals`, `share.decideAccessApproval`, `share.reportApprovalUndeliverable`, Felder in `share.status`, `share.execLog` (S1); `docs.*`, Ereignis `docs.changed` (S2); 19 Methoden `vault.*`, `sys.hostState.screenOn`, Ereignis `vault` (S3); `screen.*`, Ereignisse `screenHostStart`, `screenSession` (S4); Navigationsarten `showLauncherSettings`, `openScreen`, `showScreenGrid` (S5); `app.licenses` (C4) | PRM-M10, DRV-M8, VLT-M11, SCR-M8/M9, DSK-M13, C4 | U43 |
| `docs/TODO.md` | Einträge im Format der Tabelle „Still open“ (`\| ID \| **Titel** … \| Zustand \| Notizen \|`); Liste unten | alle | U43 |

**Einträge in `docs/TODO.md` (U43):**

| ID | Zustand | Inhalt |
|---|---|---|
| `EXEC-READONLY` („Nur lesen“) | later | „Nur lesen“ als Sandbox-Schalter entfällt in v0.5.183; eingeschränkter Zugriff läuft über `se-remote` oder ein benanntes Konto. Belegte Bedingungen für einen späteren Stapel: Linux Landlock ab ABI 6 plus seccomp-Verbot von `socket()` für alle Adressfamilien; Windows Integritätsstufe Low plus „No-Read-Up“ auf `daemon.token` und eine Recherche der RPC-Wege; Android 17 mit GKI ab `android16-6.12`. Quelle `strang-rechte.md` C8 |
| `SCREEN-VAAPI` | later | VA-API-Kodierung unter Linux ist nicht in v0.5.183; offene Punkte aus `nachrecherche-bildschirm.md` Antwort N14 |
| `RUST-CRATE-LICENSES` | ⬜ | nur der Rest nach C4: eigenständiges Share-Server-Payload und Kontextmenü-DLL ohne eigenen Ausgabeweg; Gradle-Abhängigkeiten der APK nicht geprüft |
| `VAULT-RECOVERY-KEY` | ⬜ (Nutzerentscheidung OE1) | Wortliste des Wiederherstellungsschlüssels. Drei Wege: (a) Laden zur Laufzeit von Cryptomators GitHub mit festem SHA-256-Pin (Tag `1.19.3`), (b) schriftliche Erlaubnis von Skymatic und Mitliefern, (c) dauerhaftes Weglassen. Bis zur Entscheidung zeigen alle Oberflächen einen Informationstext; Vektor `nachrecherche-tresore.md` G2, Kodierung `strang-tresore.md` C1.10 |
| `VAULT-SCREENLOCK-VERIFY` | ⬜ | Handtest der Annahmen A1 (Windows: `WM_WTSSESSION_CHANGE` erreicht das verborgene Fenster des Workers) und A2 (GNOME Shell und KDE Plasma setzen `LockedHint`) aus `strang-tresore.md` F |
| `LUECKEN-WINDOWS-RUNAS` (Bestand, `later`) | ✅ nach dem Release | erledigt durch den Windows-Kontodienst mit S4U (PRM-M7); U43 setzt die Notiz auf „umgesetzt, ausgeliefert nach Release“, den Haken setzt der Hauptagent nach dem Release |
| aus den Strängen, unverändert übernommen | later bzw. ⬜ | S1: Android-Kontexte root/Shizuku mit Einhegungsfrage. S2: `WINFSP-AB`, `DOKANY-WATCH`. S3: Restlücken L-C, L-E, L-G, L-H, L-I aus `strang-tresore.md` G. S4: SYSTEM-Hilfsdienst mit SAS-Richtlinie, UIAccess, Shizuku-`UserService`, AV1/HEVC, Zwischenablage, Ton, libei, NVENC unter Linux. S5: `PIN-TASKBAR`, `DESKTOP-EMBED`, `DESKTOP-LAYER-IME`, `DESKTOP-WAYLAND-DND`, `DESKTOP-LINUX-DRAGOUT`, `DESKTOP-GNOME-LEGACY`, `DESKTOP-GNOME-XML-WALLPAPER`, `LINUX-UNINSTALL` |

### C6 Oberflächeninventare

Format wie `docs/oberflaechen/share-exec.md` und `einstellungen.md`: Kopf mit Stand und Lesestand; Abschnitt „Oberflächen“ (Kürzel, Oberfläche, Einstieg); Fähigkeitstabellen mit den Spalten CLI (Text/JSON/Vervollständigung), Desktop, Android (Bridge → App), IPC und MCP, Belege als `Datei:Zeile`; „Ausnahmen mit Grund“; „Offene Punkte nach dem Stapel“. Jede Zeile nennt einen Beleg oder eine Ausnahme mit Grund.

| Datei | Gliederung | Anlegen | Pflege | Eigentümer-Meilenstein (Vollständigkeit) |
|---|---|---|---|---|
| `docs/oberflaechen/laufwerke.md` (neu) | 1 Oberflächen; 2 Laufwerk einhängen, aushängen, Status (Windows, Linux, Android-Ausnahme); 3 Optionen und „Beim Anmelden automatisch einbinden“; 4 Einhängeort und Dateimanager-Einbindung (GTK, KDE, Explorer); 5 Gerätebaum „Verbundene Geräte als Ordner“ (`se drive devices …`, Einstellungen, IPC-Wache); 6 Tresor-Laufwerke (`se drive mount vault://<SITE>`, `requires-unlock`, Sperren); 7 Android-Dokumentanbieter (`docs.*`, Einstellungen „Ordner für Apps bereitstellen“); 8 Laufzeit (`se drive runtime`, `install-runtime`); 9 Ausnahmen mit Grund; 10 Offene Punkte | U06 (P2) | U12, U14, U17, U23 | DRV-M9 (U42) |
| `docs/oberflaechen/tresore.md` (neu) | 1 Oberflächen; 2 Fähigkeiten T1–T22 (Matrix aus `strang-tresore.md` C7.1); 3 Öffnen-Regel (C7.2); 4 Sperrauslöser je Plattform mit dem Satz zu K13; 5 Tresor-Sync und Vergleichsmodus; 6 Klartextgrenze und Hinweise K45 (Miniaturbilder, Indexer, Auslagerung); 7 Interop-Aussagen (Cyberduck/Mountain Duck, Cryptomator Desktop 1.19.3 ohne #355); 8 Wiederherstellungsschlüssel (OE1, nur Informationstext); 9 Ausnahmen mit Grund; 10 Offene Punkte | U42 (P15) | – | VLT-M13 (U42) |
| `docs/oberflaechen/desktop-launcher.md` (neu) | 1 Oberflächen; 2 Fähigkeiten O1–O19 (Tabelle aus `strang-desktop.md` C7); 3 Plattformgrenzen Windows, Linux (je Sitzungsart), Android; 4 feste Namen und Pfade (`SmartExplorerDesktop`, `{DESKTOP_NAV_CLSID}`, `state.json`, `config.json`, `layout/*.json`, Erweiterungs-UUID, Launcher-Komponenten); 5 Modularität (Zustände, Restspuren, `residue`); 6 Erreichbarkeit ab Anmeldung (V11); 7 Ausnahmen mit Grund; 8 Offene Punkte | U15 (P5) | U30, U32, U34, U37, U40 | DSK-M14 (U43) |
| `docs/oberflaechen/bildschirm.md` (neu, aus S4) | 1 Oberflächen; 2 Rechte (Verweis auf `share-exec.md`); 3 Geräte, Probe, Messung (`se screen devices/probe/bench`); 4 Sitzungen und Anzeige; 5 Kodierung und OpenH264 mit Lizenzansicht; 6 Fenster und Standbild; 7 Eingabe; 8 Betrachter und Gruppenansicht (Desktop, Android); 9 Android als Host; 10 Ausnahmen mit Grund (Live-Video nicht über MCP, Rückfrage nicht über MCP, Wayland-Fensterwahl am Host, wlroots ohne Steuern, Android ohne Tastenereignisse, Windows ohne UAC/Strg+Alt+Entf); 11 Offene Punkte | U10 (P4) | U13, U16, U19, U22, U25, U28 | SCR-M11 (U42) |
| `docs/oberflaechen/share-exec.md` (Bestand) | neuer Abschnitt „Stand nach dem Stapel Rechte (v0.5.183)“: Zugriffsrecht anzeigen und ändern, Ausführen als, Rückfrage entscheiden, Gültig bis, Konten dieses Geräts, Raum-Sammelaktion, Protokoll mit Art; Ausnahmen MCP (Rückfragen entscheiden, Kontodienst einrichten) und Android (nur App-Konto); Abweichungen A1–A10 der Inventur behoben; Bildschirmzeilen mit Verweis auf `bildschirm.md` | – | – | PRM-M11 (U42) |
| `docs/oberflaechen/INDEX.md` (Bestand) | Teildokumente um `laufwerke.md`, `tresore.md`, `desktop-launcher.md`, `bildschirm.md`; Kostenklassen C1–C3 definiert (S1 B23) | – | – | PRM-M11, DRV-M9, VLT-M13, SCR-M11, DSK-M14 (U42) |

## D — Arbeitsplan

### D0 Regeln für jedes Paket

- **Dateien eines Pakets:** genau die Dateien, die die Strangpläne unter „Neue Dateien“ und „Geänderte Dateien“ der enthaltenen Meilensteine nennen (Fundstellen in D1), abzüglich der Hub-Dateien aus D2.2 und der Android-Hub-Dateien, die D2.3 einem anderen Paket zuweist, zuzüglich der in D1 ausdrücklich genannten Ergänzungen. Testdateien, die ein Meilenstein nennt, gehören dazu. Abnahmesignale der Pakete werden als Tests im Code geschrieben und nur in der einen Suite ausgeführt.
- **Nicht berühren:** jede Hub-Datei (D2.2), jede Datei der anderen Pakete derselben Phase und jede Datei, die D1 für das Paket nicht nennt. Jedes Paket nennt in seinem Bericht unter „Registrierungsbedarf“ die genauen Zeilen, die das R-Paket der Phase in Hub-Dateien setzen soll (Datei, Stelle, Zeilen). Unter „Doku-Bedarf“ nennt es die Sätze für die Doku-Dateien aus C5.
- **Keine Builds, Tests, Paketinstallationen, Commits oder Pushes** durch Pakete (AGENTS.md). Neue oder wesentlich bearbeitete Rust-Dateien bleiben unter 500 Zeilen und 50 KiB; `core/`-Dateien ohne `cfg`, OS-Crates und FFI.
- **Bericht** (deutsch): gelesene Dateien; angelegte und geänderte Dateien mit Zeilenzahl; Abnahmesignale mit Testdatei; Registrierungsbedarf; Doku-Bedarf; Entscheidungen; offene Punkte.
- **R-Pakete** (R1–R14): je Phase eines, nach den Umsetzungspaketen der Phase. Sie setzen den gemeldeten Registrierungsbedarf und die in D1 genannten Hub-Aufgaben um und ändern nur Hub-Dateien.
- Die Phasenzuordnung legt fest, welche Pakete ohne Dateikonflikt gleichzeitig laufen können (höchstens drei). Wie viele davon tatsächlich gleichzeitig laufen, entscheidet der Hauptagent nach dem Nutzungslimit.

### D1 Umsetzungspakete

Fundstellen: „R“ = `strang-rechte.md`, „L“ = `strang-laufwerke.md`, „T“ = `strang-tresore.md`, „B“ = `strang-bildschirm.md`, „D“ = `strang-desktop.md`, jeweils mit den Zeilennummern der Dateilisten und der Abnahmesignale im Stand vom 2026-10-17.

| Paket | Phase | Strang | Meilensteine | Größe (Zeilen, grob) | hängt ab von |
|---|---|---|---|---|---|
| U01 | P1 | S1 | PRM-M1, PRM-M2, PRM-M5 Teil (a) | 3 300 | – |
| U02 | P1 | S2 | DRV-M1, DRV-M2 | 4 000 | – (Helfer-API aus V8 als Vertrag) |
| U03 | P1 | S5 | DSK-M2, DSK-M1, Parser V9 | 3 800 | – |
| R1 | P1 | alle | Ausgliederungen und Registrierung P1 | 400 | U01, U02, U03 |
| U04 | P2 | S1 | PRM-M3, PRM-M4 | 2 400 | R1 |
| U05 | P2 | S3 | VLT-M1, PlanRules-Haken aus VLT-M7 | 3 350 | R1 |
| U06 | P2 | S2 | DRV-M3 | 2 600 | R1 |
| R2 | P2 | alle | Registrierung P2 | 150 | U04, U05, U06 |
| U07 | P3 | S1 | PRM-M8 und PRM-M9 ohne Konten (Z1) | 2 500 | R2 |
| U08 | P3 | S3 | VLT-M2, VLT-M3 | 4 000 | R2 |
| U09 | P3 | S4 | SCR-M1 | 4 300 | R2 |
| R3 | P3 | alle | Registrierung P3 | 100 | U07, U08, U09 |
| U10 | P4 | S4 | SCR-M2 | 5 000 | R3 |
| U11 | P4 | S3 | VLT-M4, VLT-M5 | 3 850 | R3 |
| U12 | P4 | S2 | DRV-M4 | 5 600 | R3 |
| R4 | P4 | alle | Registrierung P4 | 150 | U10, U11, U12 |
| U13 | P5 | S4 | SCR-M3 | 3 700 | R4 |
| U14 | P5 | S3+S2 | VLT-M12, danach DRV-M7 | 3 350 | R4 |
| U15 | P5 | S5 | DSK-M3 ohne Installer-Teil | 4 300 | R4 |
| R5 | P5 | alle | Registrierung P5 | 150 | U13, U14, U15 |
| U16 | P6 | S4 | SCR-M4 | 3 100 | R5 |
| U17 | P6 | S2 | DRV-M5, DRV-M6 | 4 400 | R5 |
| U18 | P6 | S1 | PRM-M5 Teile (b), (c), PRM-M6 | 3 000 | R5 |
| R6 | P6 | alle | Registrierung P6 | 120 | U16, U17, U18 |
| U19 | P7 | S4 | SCR-M5 | 4 300 | R6 |
| U20 | P7 | S3 | VLT-M6, VLT-M7 ohne PlanRules-Haken | 3 150 | R6 |
| U21 | P7 | S1 | PRM-M10 ohne Bildschirm-Aktionsziele (W1) | 1 600 | R6 |
| R7 | P7 | alle | Registrierung P7 | 100 | U19, U20, U21 |
| U22 | P8 | S4 | SCR-M6, SCR-M10 verkleinert (W6) | 2 200 | R7 |
| U23 | P8 | S2 | DRV-M8 | 3 300 | R7 |
| U24 | P8 | S1 | PRM-M7 und Konten-Oberflächen (Z1) | 3 050 | R7 |
| R8 | P8 | alle | Registrierung P8 | 100 | U22, U23, U24 |
| U25 | P9 | S4 | SCR-M7 | 2 950 | R8 |
| U26 | P9 | S3 | VLT-M8, VLT-M9 | 5 800 | R8 |
| U27 | P9 | S4 | SCR-M8 einschließlich der Aktionsziele aus W1 | 3 600 | R8 |
| R9 | P9 | alle | Registrierung P9 | 150 | U25, U26, U27 |
| U28 | P10 | S4 | SCR-M9 | 2 100 | R9 |
| U29 | P10 | S3 | VLT-M10 | 3 300 | R9 |
| U30 | P10 | S5 | DSK-M4 | 3 200 | R9 |
| R10 | P10 | alle | Registrierung P10 | 100 | U28, U29, U30 |
| U31 | P11 | S3 | VLT-M11 | 3 350 | R10 |
| U32 | P11 | S5 | DSK-M5 | 2 000 | R10 |
| U33 | P11 | alle | INT-LIC ohne Installer-Teil (C4) | 900 | R10 |
| R11 | P11 | alle | Registrierung P11 | 80 | U31, U32, U33 |
| U34 | P12 | S5 | DSK-M6, DSK-M8 | 3 200 | R11 |
| U35 | P12 | S5 | DSK-M9, DSK-M10 | 3 500 | R11 |
| U36 | P12 | alle | Installer und Linux-Skript (C5) | 250 | R11 |
| R12 | P12 | alle | Registrierung P12 | 60 | U34, U35, U36 |
| U37 | P13 | S5 | DSK-M7 | 2 650 | R12 |
| U38 | P13 | S5 | DSK-M11, DSK-M12 ohne die zwei Gesten aus W5 | 2 300 | R12 |
| U39 | P13 | alle | Suite Teil A: Workflow, Einstieg, Stufen S1, S2, S3, Lizenzen, `prepare` | 2 800 | R12 |
| R13 | P13 | alle | Registrierung P13 | 60 | U37, U38, U39 |
| U40 | P14 | S5 | DSK-M13, Gesten und `performGlobal` (W5, H3) | 2 200 | R13 |
| U41 | P14 | alle | Suite Teil B: Stufen S4, S5, Launcher, Android-Geräteläufe | 2 500 | R13 |
| R14 | P14 | alle | Registrierung P14 | 60 | U40, U41 |
| U42 | P15 | alle | Doku A: PRM-M11, DRV-M9 (Doku), VLT-M13, SCR-M11 | 1 200 | R14 |
| U43 | P15 | alle | Doku B: DSK-M14, `docs/TODO.md`, `api.md` | 500 | R14 |

Die Ausgliederungen vor der Erweiterung stehen bei dem Paket, das die Datei zuerst erweitert: `types.rs` → `mount_source.rs`/`mount_status.rs` (U02), `journal.rs` → `journal_record.rs` (U02), `mount_manager.rs` → `mount_manager_start.rs` und `mount_ui_draft.rs` → `mount_ui_location.rs` (U06), `ipc_protocol.rs` → `ipc_protocol_mount.rs` und `ipc_host.rs` → `ipc_host_ext.rs` (R1), `N/app/core/mount_peer_roots.rs` → `N/mount/core/peer_roots.rs` (U17).

**Einzelheiten je Paket** (Dateien = Regel D0 mit den genannten Fundstellen; „+“ = Ergänzung, „−“ = Ausnahme; Abnahmesignale = alle Namen der genannten Fundstellen plus „+“):

- **U01** (S1-Kern). Dateien: R 296–297, 308–309; aus PRM-M5 nur `N/share/os/linux_os/exec_systemd.rs` (Eigenschaft `ExitType` entfernt). + `N/share/access_modules.rs` (neu; Deklaration aller neuen S1-Module unter `N/share/`). Nicht berühren: `N/mount/**`, `N/desktop/**`, `N/app/core/init.rs`, `N/app/core/frame_update.rs` (U02, U03). Abnahmesignale: R 302, R 314, + `perm_task_systemd_unit_props_v249_compatible`.
- **U02** (S2-Kern). Dateien: L 229–230, 238–239, ohne `N/daemon/os/shared/ipc_protocol.rs`; das Feld `MountHostConfig.path_dialect` setzt R1. + Haken für den Stoppmerker (V8) in `N/mount/os/shared/host_session.rs`. Nicht berühren: `N/share/**`, `N/cli/**`, `N/desktop/**`, `N/app/**` (U01, U03). Abnahmesignale: L 233, L 243, + `drive_task_host_session_update_stop_unmounts_and_exits` (Windows).
- **U03** (S5-Fundament). Dateien: D 526–527, 538–539 ohne `N/lib.rs`, ohne die Moduldeklaration von `device_view` (der Strangplan nennt `N/share/core/mod.rs`; im Bestand heißt die Datei `N/share/mod.rs`) und ohne die „Exportstelle im Modul daemon“ (alle drei R1). + `N/app/core/start_request.rs` (Parser V9 mit allen Startarten und der Tabelle aller bisherigen Aufrufformen) und `N/app/desktop_modules.rs` (neu). Nicht berühren: `N/share/core/**` außer der neuen Datei `device_view.rs`, `N/mount/**`. Abnahmesignale: D 532, D 544, + `desktop_task_gui_start_order_unchanged`, `desktop_task_open_location_args` (beide gegen den Parser; Verdrahtung in R5).
- **R1**. Hub-Aufgaben: Ausgliederung `ipc_protocol.rs` → `N/daemon/os/shared/ipc_protocol_mount.rs` (B2, serde-Form unverändert, Re-Export); `N/daemon/os/shared/ipc_host_ext.rs` mit Aufrufstelle in `ipc_host.rs`; `MountHostConfig.path_dialect` (`#[serde(default)]`); `share/mod.rs` (`access_modules`, `device_view`); `lib.rs` (`pub mod desktop;` nur für Nicht-Android); `daemon/mod.rs` (`update_stop`, `access_grant_journal`, Re-Exporte von `update_stop_marker` und den neuen Namen); `app/mod.rs` (`desktop_modules`). Abnahmesignale: + `drive_task_ipc_mount_types_serde_unchanged` (Rundlauf der ausgegliederten Typen gegen JSON-Fixtures aus `da2646d2`), `drive_task_types_split_serde_unchanged`.
- **U04** (Rückfrage, Protokoll). Dateien: R 320–321 ohne `ipc_protocol.rs` (R2) und ohne `N/daemon/os/shared/exec_ipc.rs` (W10), R 332–333. + `N/app/access_modules.rs` (neu). Das Fenster `--access-consent` nutzt `update_stop_ui.rs` (V8). Nicht berühren: `N/cryptomator/**`, `N/bisync/**`, `N/mount/**`, `N/daemon/os/shared/mount_*`, `N/autostart/**` (U05, U06). Abnahmesignale: R 326, R 336.
- **U05** (Tresor-Formatkern, PlanRules). Dateien: T 443–444 ohne `native/src/lib.rs` und `native/Cargo.toml` (R2); aus VLT-M7 nur `N/bisync/core/plan_rules.rs`, `N/bisync/os/shared/orchestration_plan.rs`, `N/bisync/os/shared/apply_reporting.rs` und die Moduldeklaration für `plan_rules` im Bisync-Modul. Nicht berühren: `N/share/**`, `N/mount/**`, `N/daemon/**`. Abnahmesignale: T 448, + `vault_task_plan_rules_default_single_phase`, `vault_task_sync_phases_content_before_node` (mit Fake-Regeln, ohne Tresor).
- **U06** (Persistenz, Wiederherstellung). Dateien: L 248–249 ohne `ipc_protocol.rs`, `N/lib.rs`, `N/main.rs` (R2). + `N/daemon/drive_modules.rs` (neu), `N/app/drive_modules.rs` (neu), `docs/oberflaechen/laufwerke.md` (neu, Gliederung C6). Nicht berühren: `N/share/**`, `N/notify_desktop/**`, `N/cryptomator/**`, `N/bisync/**`. Abnahmesignale: L 253.
- **U07** (Rechte-Oberflächen). Dateien: R 376–377 ohne `N/cli/share/exec_accounts.rs`, ohne `N/cli/completions.rs` und `N/cli/mcp/catalog.rs` (R3); R 387–388 ohne `share_access_accounts_ui.rs`. Nicht berühren: `N/vault/**`, `N/connect/**`, `N/vfs/**`, `N/screen/**`, `N/share/core/exec_protocol.rs` (U09). Abnahmesignale: R 381 ohne `perm_task_cli_accounts_json`; R 391 ohne `perm_task_gui_accounts_page_states` und `perm_task_gui_setup_without_pkexec_shows_sudo_command`.
- **U08** (Tresorschicht, Locator). Dateien: T 454–455 ohne `ipc_protocol.rs` (`MountBackendScheme::Vault` setzt R3 in `ipc_protocol_mount.rs`); T 465–466. Nicht berühren: `N/share/**`, `N/cli/share/**`, `N/app/core/share_*`, `N/screen/**`. Abnahmesignale: T 459, T 468.
- **U09** (Bildschirm-Kernformate). Dateien: B 301–302 ohne `N/lib.rs` und `native/Cargo.toml` (R3). Nicht berühren: `N/vault/**`, `N/cli/**`, `N/app/**`. Abnahmesignale: B 306.
- **U10** (Bildschirmverbindung). Dateien: B 312–313 ohne `ipc_protocol.rs`, `N/cli/mod.rs`, `N/cli/completions.rs`, `N/lib.rs`, `N/cli/mcp/catalog.rs`, `docs/oberflaechen/INDEX.md` (R4 bzw. U42). + `N/app/screen_modules.rs` (neu). `node_screen` deklariert R4. Die Prozesse `--screen-indicator` nutzen V8. Nicht berühren: `N/vault/**`, `N/agent/**`, `N/agent_proto/**`, `N/mobile/**`, `N/mount/**`, `N/daemon/os/shared/rooted_backend*`, `N/daemon/os/shared/mount_proxy*`. Abnahmesignale: B 318.
- **U11** (Sitzungen, Verwaltung). Dateien: T 474–475 ohne `ipc_protocol.rs`, `ipc_host.rs`, `domains/mod.rs`, `native/Cargo.toml` (R4); T 484. Nicht berühren: `N/screen/**`, `N/share/core/node*.rs`, `N/mount/**`, `N/app/core/sidebar.rs`. Abnahmesignale: T 478, T 486.
- **U12** (Linux-FUSE-Host). Dateien: L 258–259 ohne `native/Cargo.toml` und `N/cli/mcp/catalog.rs` (R4). Der Linux-Host übernimmt den Stoppmerker-Haken aus U02. Nicht berühren: `N/vault/**`, `N/agent*/**`, `N/screen/**`, `N/share/**`. Abnahmesignale: L 264, + `drive_task_host_session_update_stop_unmounts_and_exits` (Linux).
- **U13** (H.264, Hostprozess). Dateien: B 324–325 ohne `native/Cargo.toml`, `N/bin/se.rs`, `N/lib.rs` (R5). Der Hostprozess nutzt V8. Nicht berühren: `N/vault/**`, `N/mount/**`, `N/desktop/**`, `N/daemon/os/shared/mount_*`, `N/cli/drive*.rs`. Abnahmesignale: B 330, + `screen_task_host_worker_exits_on_update_stop`.
- **U14** (Tresor-Laufwerke beider Seiten; Commit-Reihenfolge VLT-M12, dann DRV-M7). Dateien: T 548; L 288–289. + `N/daemon/os/shared/ipc_protocol_mount_host.rs` (neu, nur `MountHostRequest::PrepareLock { deadline_ms }`, W3), Änderung `N/daemon/os/shared/ipc_protocol_drives.rs` (`MountPathFor`). Pflege `laufwerke.md`. Nicht berühren: `N/screen/**`, `N/desktop/**`, `N/app/core/settings_ui.rs`, `N/app/core/init.rs`, `N/app/core/frame_update.rs`, `N/daemon/os/shared/ipc_client.rs`. Abnahmesignale: T 551, L 292.
- **U15** (Modulrahmen Desktop). Dateien: D 550–551 ohne `N/lib.rs`, `N/cli/mod.rs`, `N/cli/mcp/catalog.rs`, `N/cli/mcp/status_tools.rs` (R5) und ohne `native/installer.nsi`, `install-linux.sh` (U36). + `docs/oberflaechen/desktop-launcher.md` (neu, C6). Nicht berühren: `N/screen/**`, `N/vault/**`, `N/mount/**`. Abnahmesignale: D 562 ohne `desktop_task_installer_static` und `desktop_task_install_linux_static` (W13).
- **U16** (Windows als Bildschirm-Host). Dateien: B 336–337 ohne `native/Cargo.toml` und `N/cli/mcp/catalog.rs` (R6: Werkzeug `screen`, Testliste in `mcp_task_catalog_is_complete_and_within_host_limits`). Nicht berühren: `N/mount/**`, `N/share/os/linux_os/**`, `N/share/core/exec_*`. Abnahmesignale: B 342.
- **U17** (Dateimanager, Gerätebaum). Dateien: L 269–270; L 277–278 ohne `ipc_protocol.rs` (die Variante `MountHostExt` steht seit R5), ohne `N/cli/mod.rs` und `N/cli/mcp/catalog.rs` (R6); `N/mount/mod.rs` (Re-Export `device_key`) ändert U17 selbst. Erweiterung `ipc_protocol_mount_host.rs` um `ChildBackend`, `DeviceWatch` (W3). Abschnitt in `settings_ui.rs` vor dem S5-Abschnitt (V14). Pflege `laufwerke.md`. Nicht berühren: `N/screen/**`, `N/share/os/linux_os/**`. Abnahmesignale: L 272, L 283.
- **U18** (Linux-Konten). Dateien: R 342–343 (in `exec_systemd.rs` nur noch die Kontoeigenschaften; `ExitType` hat U01 entfernt); R 354–355. Nicht berühren: `N/screen/**`, `N/mount/**`, `N/app/**`. Abnahmesignale: R 348 ohne `perm_task_systemd_unit_props_v249_compatible` (U01), R 359.
- **U19** (Linux als Bildschirm-Host). Dateien: B 348–349 ohne `native/Cargo.toml` (R7) und ohne `install-linux.sh` (U36). Nicht berühren: `N/vault/**`, `N/syncjobs/**`, `N/bisync/**`, `N/mobile/**`, `A/**`. Abnahmesignale: B 354.
- **U20** (Klartextgrenze, Tresor-Sync). Dateien: T 492–493; T 501–502 ohne die drei PlanRules-Dateien aus U05. + `N/app/vault_modules.rs` (neu; deklariert `N/app/os/shared/vault_open.rs`). Nicht berühren: `N/screen/**`, `A/**`, `N/mobile/os/shared/domains/share_*.rs`. Abnahmesignale: T 495, T 506 ohne `vault_task_plan_rules_default_single_phase` (U05).
- **U21** (Android-Rechte). Dateien: R 397–398 ohne `docs/superpowers/plans/2026-09-25-android-apk/api.md` (U43); Android-Hub-Dateien nach D2.3. Kein Aktionsziel für Bildschirmfähigkeiten (W1). + `android/app/src/androidTest/java/app/smartexplorer/android/task/PermTaskTest.kt` (neu). Nicht berühren: `N/vault/**`, `N/syncjobs/**`, `N/screen/**`, `N/mobile/os/shared/places.rs`, `N/mobile/os/shared/edits.rs`, `N/mobile/os/shared/domains/job_json.rs`. Abnahmesignale: R 401 ohne `perm_task_android_screen_approval_actions_are_activity_intents` (läuft nach U27).
- **U22** (Eingabe, OpenH264-Lizenz). Dateien: B 360–361; aus B 410–411 nur `third-party/openh264/BINARY_LICENSE.txt`, `N/screen/core/licenses.rs`, `N/cli/screen/codec.rs`, `N/app/core/share_screen_codec_ui.rs` (W6). Nicht berühren: `A/**`, `N/mobile/**`, `N/share/os/windows/**`, `native/installer.nsi`. Abnahmesignale: B 366; aus B 415 `screen_task_cisco_license_verbatim`, `screen_task_android_has_no_openh264`.
- **U23** (Android-Dokumentanbieter). Dateien: L 297–299 ohne `native/android-bridge/src/lib.rs` und `domains/mod.rs` (R8), ohne `api.md` (U43); Android-Hub-Dateien nach D2.3. + `android/app/build.gradle.kts` (Robolectric als `testImplementation`, falls nicht vorhanden; **Annahme:** Robolectric fehlt im Bestand). Pflege `laufwerke.md`. Nicht berühren: `N/share/os/windows/**`, `N/screen/**`. Abnahmesignale: L 302.
- **U24** (Windows-Kontodienst, Konten-Oberflächen). Dateien: R 365–366 ohne `native/installer.nsi` (U36); aus R 376–377 `N/cli/share/exec_accounts.rs` und die Aufrufzeile im Share-CLI-Modul; aus R 387–388 `N/app/core/share_access_accounts_ui.rs` und die Aufrufzeile in `share_exec_ui.rs`. Nicht berühren: `N/screen/**`, `N/mount/**`, `A/**`. Abnahmesignale: R 370; `perm_task_cli_accounts_json`, `perm_task_gui_accounts_page_states`, `perm_task_gui_setup_without_pkexec_shows_sudo_command`.
- **U25** (Desktop-Betrachter). Dateien: B 372–373 ohne `N/lib.rs` (R9). Betrachterfenster nutzen V8. Nicht berühren: `A/**`, `native/android-bridge/**`, `N/screen/os/android/**`, `N/screen/os/mod.rs`, `N/vault/**`, `N/cli/vault*`. Abnahmesignale: B 378.
- **U26** (Vergleich, Prüfung, CLI). Dateien: T 512; T 521–522 ohne `N/cli/mod.rs`, `N/cli/mcp/catalog.rs`, `N/cli/mcp/status_tools.rs` (R9). Nicht berühren: `N/app/core/share_*`, `N/screen/**`, `A/**`. Abnahmesignale: T 515, T 524.
- **U27** (Android als Bildschirm-Host). Dateien: B 384–386 ohne `native/android-bridge/src/lib.rs`, `N/mobile/core/events.rs`, `domains/mod.rs` (R9) und ohne `api.md` (U43); Android-Hub-Dateien nach D2.3; `A/service/AccessApprovalNotifier.kt` (W1). + Instrumentierung `android/app/src/androidTest/java/app/smartexplorer/android/task/ScreenTaskTest.kt` (neu). Inventarzeilen nur im Bericht (Z4). Nicht berühren: `N/app/**`, `N/vault/**`, `N/cli/**`. Abnahmesignale: B 391, + `perm_task_android_screen_approval_actions_are_activity_intents`.
- **U28** (Android als Betrachter). Dateien: B 397–398 ohne `domains/mod.rs` (R10) und ohne `api.md` (U43); `A/ui/AppNav.kt` nach D2.3. Nicht berühren: `N/app/**`, `N/desktop/**`, `N/vault/**`. Abnahmesignale: B 403.
- **U29** (Tresor-GUI). Dateien: T 530–531 ohne `N/app/mod.rs` (Deklaration in `N/app/vault_modules.rs`) und ohne `native/Cargo.toml` (R10, H6). Nicht berühren: `N/desktop/**`, `N/daemon/os/shared/ipc_peek_client.rs`, `N/app/core/desktop_*`, `A/**`. Abnahmesignale: T 533.
- **U30** (Desktopfläche). Dateien: D 568–569 ohne `native/Cargo.toml` (R10, H6). Nicht berühren: `N/app/core/vault_*`, `N/app/core/sidebar.rs`, `A/**`, `N/screen/**`. Abnahmesignale: D 574.
- **U31** (Tresore auf Android). Dateien: T 539–540 ohne `domains/mod.rs` (R11) und ohne `api.md` (U43); `A/ui/AppNav.kt` nach D2.3 (`NavRequest.OpenVaultCompare(jobId)`). + `android/app/src/androidTest/java/app/smartexplorer/android/task/VaultTaskTest.kt` (neu). Nicht berühren: `A/ui/settings/AboutScreen.kt`, `N/desktop/**`, `N/licenses/**`. Abnahmesignale: T 542.
- **U32** (Windows-Adapter Desktop). Dateien: D 580–581 ohne `native/Cargo.toml` (R11). Nicht berühren: `A/**`, `N/mobile/**`, `N/licenses/**`, `native/build.rs`. Abnahmesignale: D 586.
- **U33** (INT-LIC). Dateien nach C4: `native/rust-licenses.ps1`, `third-party/rust-crates/about.toml`, `third-party/rust-crates/about.hbs`, `native/build.rs`, `N/licenses/mod.rs`, `N/cli/licenses.rs`, `N/app/core/licenses_ui.rs`, die Datei der Einstellungsseite „Updates“ (Annahme C4), `N/mobile/os/shared/domains/app_licenses.rs`, `A/ui/settings/AboutScreen.kt`, `A/api/LicensesApi.kt`, `native/publish-release-local.ps1` (nur der `Check`-Aufruf und die Werkzeugprüfung). Nicht berühren: `native/installer.nsi`, `third-party/rust-crates/THIRD-PARTY-LICENSES.txt` (entsteht in der Suite, L2), `A/ui/AppNav.kt`, `android/app/src/main/res/values/strings.xml`, `N/desktop/**`, `N/vault/**`. Abnahmesignale: C4 ohne `licenses_task_installer_installs_file`.
- **U34** (Linux-X11, GNOME). Dateien: D 592–593, D 616–617. Nicht berühren: `A/**`, `android/**`, `native/installer.nsi`, `install-linux.sh`. Abnahmesignale: D 598, D 622.
- **U35** (Launcher-Rahmen und Grundfunktionen). Dateien: D 628–629, D 640–641; Android-Hub-Dateien nach D2.3. + `android/app/src/androidTest/java/app/smartexplorer/android/task/LauncherTaskTest.kt` (neu) und der Bau des Test-Launchers für `launcher_task_disable_with_two_launchers_shows_chooser_hint`. **Annahme:** Dafür reicht eine zweite Aktivität mit HOME-Filter im Test-APK; braucht es ein eigenes Gradle-Modul, ändert U35 `android/settings.gradle.kts` und nennt das im Bericht. Inventarzeilen nur im Bericht (Z4). Nicht berühren: `N/**`, `native/**`, `install-linux.sh`. Abnahmesignale: D 634, D 646.
- **U36** (Installer, Linux-Skript). Dateien: `native/installer.nsi`, `install-linux.sh` mit allen Zeilen aus C5. Nicht berühren: alle übrigen Dateien. Abnahmesignale: `desktop_task_installer_static`, `desktop_task_install_linux_static`, `drive_task_install_linux_dry_run_with_fuse`, `screen_task_installer_lists_screen_licenses`, `screen_task_install_linux_kde_interfaces_line`, `licenses_task_installer_installs_file`, `perm_task_installer_uninstall_removes_service_static` (alle statisch, ausgeführt im Job `prepare`).
- **U37** (Wayland-Layer). Dateien: D 604–605 ohne `native/Cargo.toml` (R13). Pflege `desktop-launcher.md`. Nicht berühren: `A/**`, `android/**`, `native/rdt_task/**`, `.github/**`. Abnahmesignale: D 610.
- **U38** (Widgets, Badges, Profile). Dateien: D 652–653; D 664–665 ohne `A/screen/ScreenControlService.kt` und `A/service/LauncherBridgeService.kt` (W5); Android-Hub-Dateien nach D2.3; `AT/task/TestWidgetProvider.kt`, `android/app/src/androidTest/AndroidManifest.xml`, `LauncherTaskTest.kt`. Nicht berühren: `N/**`, `native/**`, `.github/**`. Abnahmesignale: D 658; D 670 ohne `launcher_task_gestures_off_without_accessibility`, `launcher_task_gestures_with_accessibility`.
- **U39** (Suite A). Dateien: `.github/workflows/rechte-desktop-tresore-task.yml`, `native/test-rechte-desktop-tresore-task.py`, `native/rdt_task/__init__.py`, `native/rdt_task/common.py`, `native/rdt_task/prepare.py`, `native/rdt_task/perm.py`, `native/rdt_task/drive.py`, `native/rdt_task/vault.py`, `native/rdt_task/licenses.py`, `native/vault-task-interop/pom.xml`, `native/vault-task-interop/VaultTaskInterop.java` (MIT, D3). Nicht berühren: alle Rust-, Kotlin- und Doku-Dateien, `native/rdt_task/screen.py`, `desktop.py`, `launcher.py`. Abnahmesignale: `rdt_suite_workflow_static` (jeder Job mit `timeout-minutes` ≥ 30, ein Eingang `candidate_sha`, Bindungsschritt vorhanden), `rdt_suite_lock_committed_current`, `rdt_suite_cargo_audit`, `rdt_suite_inventory_complete` (D3).
- **U40** (se-Inhalte im Launcher). Dateien: D 676–677; + die zwei Gesten aus D 664–665 (`A/launcher/gestures/HomeGestures.kt`) und `A/screen/ScreenControlService.kt` (`performGlobal`, V6); Android-Hub-Dateien nach D2.3; `LauncherTaskTest.kt`. Pflege `desktop-launcher.md`. Nicht berühren: `native/rdt_task/**`, `.github/**`. Abnahmesignale: D 682, + `launcher_task_gestures_off_without_accessibility`, `launcher_task_gestures_with_accessibility`.
- **U41** (Suite B). Dateien: `native/rdt_task/screen.py`, `native/rdt_task/desktop.py`, `native/rdt_task/launcher.py`, `native/screen-task-linux.py`, `native/screen-task-windows.py`, `native/screen-task-android.py`, `native/desktop-task-windows.py`, `native/desktop-task-linux.py`, `native/launcher-task-android.py`. Nicht berühren: `.github/**`, `native/test-rechte-desktop-tresore-task.py`, die U39-Module, `A/**`. Abnahmesignale: `rdt_suite_inventory_complete` für die Jobs aus D3 mit S4-, S5- und Launcher-Stufen.
- **U42** (Doku A). Dateien: `README.md`, `docs/RELEASING.md`, `docs/GOTCHAS.md`, `docs/ARCHITEKTUR.md`, `docs/SHARE_SERVER.md`, `docs/oberflaechen/INDEX.md`, `docs/oberflaechen/einstellungen.md`, `docs/oberflaechen/share-exec.md`, `docs/oberflaechen/laufwerke.md`, `docs/oberflaechen/tresore.md` (neu), `docs/oberflaechen/bildschirm.md`, jeweils nach C5/C6 und dem gemeldeten Doku-Bedarf. Nicht berühren: Code, `docs/TODO.md`, `api.md`, `desktop-launcher.md`. Abnahmesignale: `perm_task_inventory_paths_exist`, `drive_task_docs_surfaces_complete`, `screen_task_docs_inventory_lists_every_cli_command`, `vault_task_docs_rows_t1_t22` (neu: jede Zeile T1–T22 in `tresore.md` mit Beleg oder Ausnahme), Stale-Suche nach AGENTS.md.
- **U43** (Doku B). Dateien: `docs/TODO.md`, `docs/superpowers/plans/2026-09-25-android-apk/api.md`, `docs/oberflaechen/desktop-launcher.md` (Vervollständigung DSK-M14). Nicht berühren: alle U42-Dateien, Code. Abnahmesignale: D 690.
- **R2–R14** setzen den Registrierungsbedarf ihrer Phase um. Ausdrückliche Hub-Aufgaben: R2 `Access`- und `Drive`-Variantenpaar, `--access-consent`-Verdrahtung, `--drive-restore` (W7), `cryptomator`, `aes-siv`/`aes-kw`, Sammeldateien `access_modules`/`drive_modules`; R3 `MountBackendScheme::Vault`, `mod screen;`, `lz4_flex`, Beispiele `["share","exec","approvals","--json"]`; R4 `Vault` mit `Err.code`, `Screen`, `location`, `node_screen`, `screen_modules`, `fuser`, `Win32_System_RemoteDesktop`, `--screen-indicator`, `INSTRUCTIONS` „drives“; R5 `MountHostExt`, `--screen-host-worker`, Desktop-Startarten und `--open-*`, `se desktop` in CLI, Katalog und `status`, `INSTRUCTIONS` „se Desktop“, 7 `windows`-Features und `openh264`/`bzip2`/`libloading` aus SCR-M3; R6 Werkzeug `screen` im Katalog samt Testlisten, `INSTRUCTIONS` „screen“, 10 `windows`-Features und `png` aus SCR-M4; R7 Linux-Abhängigkeiten aus SCR-M5, `vault_modules`; R8 `mod io;`, `docs`-Präfix, `windows-sys`-Features aus PRM-M7; R9 `--screen-view`/`--screen-grid`, `mod screen_jni;`, `screen`-Präfix, Ereignisse, `se vault` in CLI, Katalog und `status`, `INSTRUCTIONS` „vaults“ und Ziel `vault://`; R10 `image`-Zeile (H6); R11 `mod licenses;`, `se licenses`, Präfix `app`, Ereignis `vault`, die 2 `windows`-Features aus DSK-M5; R13 Linux-Abhängigkeiten aus DSK-M7 (S5 C8); R12 und R14 nur gemeldeten Bedarf. Abnahmesignale der R-Pakete: `mcp_task_catalog_is_complete_and_within_host_limits`, `desktop_task_mcp_instructions_within_limit`, `desktop_task_lock_only_expected_additions`.

### D2 Phasen- und Reihenfolgeplan

**D2.1 Phasen.** Eine Phase beginnt erst, wenn das R-Paket der Vorphase fertig ist und der Hauptagent ihre Commits gepusht hat. Innerhalb einer Phase laufen die Umsetzungspakete ohne Dateikonflikt nebeneinander, das R-Paket danach.

| Phase | Gleichzeitig möglich | danach |
|---|---|---|
| P1 | U01, U02, U03 | R1 |
| P2 | U04, U05, U06 | R2 |
| P3 | U07, U08, U09 | R3 |
| P4 | U10, U11, U12 | R4 |
| P5 | U13, U14, U15 | R5 |
| P6 | U16, U17, U18 | R6 |
| P7 | U19, U20, U21 | R7 |
| P8 | U22, U23, U24 | R8 |
| P9 | U25, U26, U27 | R9 |
| P10 | U28, U29, U30 | R10 |
| P11 | U31, U32, U33 | R11 |
| P12 | U34, U35, U36 | R12 |
| P13 | U37, U38, U39 | R13 |
| P14 | U40, U41 | R14 |
| P15 | U42, U43 | – |

**D2.2 Hub-Dateien (nur R-Pakete):** `native/src/lib.rs`, `native/src/main.rs`, `native/src/bin/se.rs`, `native/src/share/mod.rs`, `native/src/daemon/mod.rs`, `native/src/app/mod.rs`, `native/src/cli/mod.rs`, `native/src/cli/completions.rs`, `native/src/cli/mcp/catalog.rs`, `native/src/cli/mcp/status_tools.rs`, `native/src/daemon/os/shared/ipc_protocol.rs`, `native/src/daemon/os/shared/ipc_protocol_mount.rs`, `native/src/daemon/os/shared/ipc_host.rs`, `native/src/daemon/os/shared/ipc_host_ext.rs`, `native/src/mobile/os/shared/domains/mod.rs`, `native/src/mobile/core/events.rs`, `native/android-bridge/src/lib.rs`, `native/Cargo.toml`. `native/Cargo.lock` ändert nur der Hauptagent aus dem Suite-Artefakt (L1).

**D2.3 Android-Hub-Dateien** (`android/app/src/main/AndroidManifest.xml`, `A/ui/AppNav.kt`, `A/core/NativeBridge.kt`, `A/core/CoreEvents.kt`, `A/ui/settings/SettingsScreen.kt`, `android/app/src/main/res/values/strings.xml`, `android/app/build.gradle.kts`): In jeder Phase gehören sie dem einzigen Android-Paket der Phase: P7 U21, P8 U23, P9 U27, P10 U28, P11 U31, P12 U35, P13 U38, P14 U40. Ausnahmen: U33 (P11) ändert nur `A/ui/settings/AboutScreen.kt` und `A/api/LicensesApi.kt`; U04 (P2) nur `A/ui/share/ExecAuditDialog.kt` und `A/api/ShareExecApi.kt`; U01 (P1) nur `A/ui/share/ExecDialog.kt`; U11 (P4) nur `A/api/SyncApi.kt` und `A/system/HostMonitor.kt`.

**D2.4 Zwangsreihenfolgen** (jede ist durch die Phasen erfüllt):
1. DRV-M2 (U02, P1) → VLT-M12 (U14, P5, erster Commit) → DRV-M7 (U14, P5, zweiter Commit); VLT-M12 braucht außerdem VLT-M1 (U05, P2) und VLT-M4 (U11, P4); DRV-M7 braucht DRV-M3 (U06, P2) und DRV-M4 (U12, P4).
2. Ausgliederungen vor jeder Erweiterung: `types.rs` und `journal.rs` in U02 vor allen späteren S2-/S3-Änderungen; `ipc_protocol.rs` und `ipc_host.rs` in R1 vor allen neuen IPC-Varianten (R2–R5); `mount_manager.rs` und `mount_ui_draft.rs` in U06 vor U12, U14, U17; `mount_peer_roots.rs` in U17.
3. S1 und S4 an `ipc_protocol.rs`: `Access` in R2 vor `Screen` in R4 (C2, S1 Punkt 3).
4. S4 und S5 an `AppNav.kt`: U28 (P10) → U31 (P11) → U35 (P12) → U40 (P14) (H4).
5. Anwendungsphasen in `apply_reporting.rs` (OE4) früh: U05 in P2; der Erhaltungsfall `vault_task_plan_rules_default_single_phase` steht in der Suite.
6. Stoppmerker-Helfer (DSK-M2) in P1 vor jedem langlebigen Prozess der Liste V8 (U04 ab P2, U10 P4, U12 P4, U13 P5, U15 P5, U25 P9).
7. PRM-M5 Teil (a) (`ExitType`) als erste S1-Änderung in U01.
8. Rechte vor Bildschirm: PRM-M1 (U01) → SCR-M1 (U09); PRM-M3/M4 (U04) und PRM-M8/M9 ohne Konten (U07) → SCR-M2 (U10).
9. W1: PRM-M10 (U21, P7) → SCR-M8 (U27, P9).
10. W3: `PrepareLock` in U14 (P5) → `ChildBackend`/`DeviceWatch` in U17 (P6).
11. S5-Inhalte nach ihren Quellen: DRV-M6 (U17, P6), VLT-M4 (U11, P4) und SCR-M7 (U25, P9) → DSK-M4 (U30, P10) → DSK-M5 (U32, P11), DSK-M6/M8 (U34, P12), DSK-M7 (U37, P13); DRV-M5 (U17) → DSK-M6 (U34); SCR-M9 (U28), VLT-M11 (U31), DRV-M8 (U23), DSK-M9/M10 (U35) → DSK-M13 (U40); SCR-M8 (U27) → `performGlobal` (U40).
12. Einstellungsseite „Integration“: U15 (P5) vor U17 (P6) (V14).
13. Lizenzen: U33 (P11) vor U36 (P12, Installer-Zeile); die Datei selbst nach dem ersten Suite-Lauf (L2).
14. Reihenfolge innerhalb S2 geändert: DRV-M7 (U14, P5) vor DRV-M5 (U17, P6). Zulässig, weil DRV-M7 keinen Code aus DRV-M5 aufruft; die GTK- und KDE-Einträge für Tresor-Laufwerke entstehen mit DRV-M5 über die allgemeine Lesezeichenlogik, und die Suite prüft sie danach (`drive_task_gtk_bookmark_only_outside_glib_rule`).

### D3 Die eine Stapel-Suite

**Workflow** `.github/workflows/rechte-desktop-tresore-task.yml` (U39), Name `rechte-desktop-tresore-task`. Auslöser nur `workflow_dispatch` mit genau einem Eingang `candidate_sha` (volle 40-Zeichen-SHA, muss dem gewählten Ref entsprechen). `permissions: contents: read`; `concurrency` mit der Gruppe `rechte-desktop-tresore-task-${{ github.repository }}` und `cancel-in-progress: false`. Jeder Job folgt dem Muster aus `luecken-task.yml`:
- Checkout mit `ref: ${{ github.sha }}` und `persist-credentials: false`;
- Bindungsschritt (SHA-Format, `CANDIDATE_SHA` = `GITHUB_SHA` = `HEAD`);
- `dtolnay/rust-toolchain@stable`, `actions/setup-python@v6` mit 3.12, `Swatinem/rust-cache@v2` mit `shared-key: rechte-desktop-tresore`;
- ein Aufruf `python native/test-rechte-desktop-tresore-task.py --job <JOB> --log-root "$RUNNER_TEMP/rdt-<JOB>"`;
- Belege mit `if: always()` über `actions/upload-artifact@v4`, 14 Tage Aufbewahrung.

Jeder Job außer `prepare` hat `needs: prepare` und legt vor dem Bau `native/Cargo.lock` und `third-party/rust-crates/THIRD-PARTY-LICENSES.txt` aus dem Artefakt `rdt-prepare` über den Checkout (L1, L2).

**Bauregel:** nur das Entwicklungsprofil. Gebaut werden genau die Bibliothekstests des Crates `smart_explorer` mit genauen Namenslisten (`cargo test --lib --manifest-path native/Cargo.toml -- <NAMEN> --exact`), die Programme `se` und `smart_explorer` (`cargo build --bin se --bin smart_explorer`) für Ende-zu-Ende-Stufen, das Share-Server-Programm nur in Jobs mit Gerätepaar sowie die Android-Bibliothek nur für `x86_64-linux-android` (Emulator). Nicht gebaut werden `--workspace`, `--all-targets`, `--release`, Installer (`makensis` läuft nicht; der Installer wird statisch geprüft), Feed und weitere Zielplattformen. Der Prüfpunkt von VLT-M1 „Kompilierung Android arm64-v8a“ wird deshalb als „nicht ausgeführt“ gemeldet, mit dem Grund „kein plattformübergreifender Bau in der Suite (AGENTS.md); arm64 baut das Release“.

**Bericht:** Jeder Job schreibt `report.json` = `{"job","candidate_sha","tests":[{"name","stage","status":"passed"|"failed"|"not_run","reason","duration_s"}]}`. Er endet mit Exitcode 1, wenn ein Test `failed` ist oder die Prüfung `rdt_suite_inventory_complete` scheitert. Diese Prüfung verlangt, dass jeder erwartete Name des Jobs genau einmal vorkommt und dass kein unerwarteter Name mit den Präfixen dieses Stapels auftaucht. `not_run` zählt nie als bestanden; die Zusammenfassung des Jobs listet jeden `not_run`-Test mit Grund. Die Testauswahl läuft über genaue Namenslisten je Stufe, nie nur über Präfixfilter; `mount_vault_task_*` wird ausdrücklich ausgeschlossen (W14).

**Präfixe:** `perm_task_` (S1), `drive_task_` (S2), `vault_task_` (S3), `screen_task_` (S4), `desktop_task_` (S5 Rust), `launcher_task_` (S5 Android), `licenses_task_` (C4), `rdt_suite_` (Prüfungen der Suite selbst). Bestandstests behalten ihre Namen.

**Jobs:**

| Job | Runner | Zeitlimit (min) | needs | `sudo` bzw. Administrator | Stufen |
|---|---|---|---|---|---|
| `prepare` | `ubuntu-24.04` | 60 | – | `sudo` nur, falls `pwsh` fehlt (C4) | `lock` (Auflösung ohne `--locked`, `rdt_suite_lock_committed_current`), `audit` (`cargo audit --file native/Cargo.lock --ignore RUSTSEC-2026-0194 --ignore RUSTSEC-2026-0195`, Test `rdt_suite_cargo_audit`), `licenses` (`licenses_task_generated_file_current`, `licenses_task_every_shipped_crate_has_text`, `licenses_task_new_batch_crates_listed`, `licenses_task_winit_notice_included`, `licenses_task_release_wrapper_checks_licenses`), `static` (die sieben Signale von U36; `desktop_task_release_assets_unchanged`, `desktop_task_updater_helper_unchanged`, `desktop_task_gnome_extension_static`, `desktop_task_lock_only_expected_additions`, `desktop_task_docs_todo_entries_present`, `screen_task_windows_interop_factories_only`, `screen_task_android_has_no_openh264`, `perm_task_inventory_paths_exist`, `drive_task_docs_surfaces_complete`, `vault_task_docs_rows_t1_t22`, `rdt_suite_workflow_static`, Stale-Suche nach AGENTS.md); Artefakt `rdt-prepare` |
| `linux-core` | `ubuntu-24.04` | 360 | prepare | `sudo` für `modprobe sch_netem` und `tc` (netem); Docker für Testserver | `unit` (alle Kern- und Einheitstests ohne Sonderumgebung aus PRM-M1, M2, M4, M8, M10 (Bridge-Seite in Rust), DRV-M1, M2, M3, M6, VLT-M1 bis M9 und M12 (Stufe `vault_unit`), SCR-M1, M3, M7, DSK-M1, M2, M3, M4, M7, M13 sowie INT-LIC `licenses_task_embedded_text_matches_file`, `licenses_task_cli_json_shape`, `licenses_task_bridge_app_licenses`), `perm-pair` (Gerätepaar im eigenen Konto: PRM-M2, M3, M4, M8), `drive-cli`, `drive-devices` (Share-Paar über den Kandidaten-Server), `vault-backends` (SFTP, Agent, FTP/FTPS, SMB, WebDAV aus `native/sync-reliability-providers.py`; Google Drive `not_run` ohne Zugangsdaten), `vault-sync`, `vault-compare`, `vault-cli`, `vault-mcp`, `vault-interop` (Java 17, CryptoFS 2.11.0 zur Laufzeit aus Maven Central; nichts davon als Artefakt oder Cache hochladen), `vault-klartext` (ohne Laufwerk), `screen-pair` (SCR-M2; netem `not_run`, wenn `sch_netem` fehlt), `docs-cli` (`screen_task_docs_inventory_lists_every_cli_command`, `desktop_task_docs_inventory_lists_every_cli_command`) |
| `linux-root` | `ubuntu-24.04` | 240 | prepare | `sudo` für den Worker als root, `useradd`, `/etc/pam.d/smart-explorer-exec`, `/usr/local/sbin/smart-explorer-exec`, `/etc/security/limits.d/perm-task.conf`, die Einrichtung des Kontodienstes (statt `pkexec`), `apt-get install -y fuse3` und `sysctl fs.fuse.default_request_timeout` | `perm-linux-root` (PRM-M5), `perm-linux-helper` (PRM-M6; Fedora-Fall `not_run`, kein Runner, Annahme A5 aus S1), `drive-fuse` (DRV-M3 Linux-Teil, DRV-M4, `drive_task_host_session_update_stop_unmounts_and_exits` Linux), `drive-vault` (DRV-M7 Linux), `vault-mount` (VLT-M12 Linux-Ende-zu-Ende), `vault-linux` (privater Systembus mit Test-logind, VLT-M4), `vault-klartext-drive` |
| `linux-2204` | `ubuntu-22.04` | 90 | prepare | `sudo` nur für `apt` | `perm-2204`: `perm_task_exec_linux_ubuntu2204_starts` (Annahme zu A10 in C2) |
| `linux-desktop` | `ubuntu-24.04` | 300 | prepare | `sudo apt-get install -y xvfb xauth x11-utils openbox xfwm4 xfdesktop4 x11-xserver-utils xdotool ffmpeg fuse3` | `gui-harness` (`SMART_EXPLORER_GUI_TASK`: Rückfragefenster PRM-M3, PRM-M9, Stufe `vault_surfaces` aus VLT-M10, Prüfung der GUI-Knöpfe aus SCR-M7, die Stufe `desktop-gui-harness` aus DSK-M15), `screen-x11` (SCR-M3 OpenH264 unter Linux mit der `ffmpeg`-Testdatei, SCR-M5 X11, SCR-M6 X11), `desktop-linux-x11` (DSK-M3-Prozessprüfungen unter Linux, DSK-M6, zuletzt `desktop_task_no_hang_restart_during_suite`), `drive-desktop` (DRV-M5 mit temporärem `HOME`, `desktop_task_disable_keeps_device_tree`) |
| `linux-wayland` | `ubuntu-24.04` | 240 | prepare | `sudo apt-get install -y sway grim pipewire wireplumber xdg-desktop-portal xdg-desktop-portal-wlr`; Versuch `kwin-wayland xdg-desktop-portal-kde` | `screen-wayland` (SCR-M5 Wayland, SCR-M6 Sway), `desktop-linux-wayland` (DSK-M7), `kde` (`screen_task_kde_window_list`, `screen_task_kde_remote_desktop_input`, `desktop_task_kwin_layer_namespace_desktop`; startet KWin kopflos nicht, alle drei `not_run`, S4 K3) |
| `linux-gnome` | `ubuntu-24.04` | 180 | prepare | `sudo apt-get install -y gnome-shell gnome-shell-extension-desktop-icons-ng`; Docker-Container `fedora:43` | `desktop-gnome` (`desktop_task_gnome_headless_variant_a`, `_b`; `not_run`, wenn die Shell kopflos nicht startet, Annahme A19 aus S5) |
| `windows-core` | `windows-2025` | 360 | prepare | Administrator (Dienst, `NetUserAdd`, Dokany-MSI) | `unit` (Windows), `perm-windows` (PRM-M7 und die Konten-Oberflächen aus U24), `drive-dokany-regression` (Bestandsharnesses `mounted_volume_task_tests`, `optimization_volume_tests`, `windows_remote_volume_task_tests`, `recovery_cache_task_tests`, `mount_options_task_tests` und alle `drive_task_windows_*`), `drive-cli` (Windows), `drive-vault` (Tresor mit Buchstaben), `vault-unit`, `vault-windows` (WTS-Verdrahtung), `vault-mount` (Windows; `not_run`, wenn Dokany sich nicht installieren lässt) |
| `windows-screen` | `windows-2025` | 240 | prepare | Administrator (`Install-WindowsFeature Server-Media-Foundation` ohne Neustart, S4 K6) | `screen-windows` (SCR-M3 Windows; `screen_task_mf_encode_decode` `not_run`, wenn Media Foundation fehlt; SCR-M4; SCR-M6 Windows; als letzter Schritt `screen_task_windows_lock_reports_secure_desktop`) |
| `windows-desktop` | `windows-2025` | 240 | prepare | kein zusätzliches Recht | `desktop-gui-harness` (Windows), `desktop-windows` (Vorprüfung `FindWindowW("Progman")` mit berichtetem Ausgang, DSK-M3-Prozessprüfungen, DSK-M5, zuletzt `desktop_task_no_hang_restart_during_suite`) |
| `desktop-bins` | `ubuntu-24.04` | 90 | prepare | – | Linux-`se` und Share-Server im Entwicklungsprofil für die Gerätepaare der Android-Jobs, Artefakt `rdt-desktop-bins` |
| `android-build` | `ubuntu-24.04` | 180 | prepare | – | Debug-APK, Test-APK mit Test-Launcher, NDK nur `x86_64-linux-android` über `cargo-ndk`; JVM- und Robolectric-Tests: Robolectric-Teil von DRV-M8, `LayoutModelTest`, `GridGeometryTest`, `LauncherSearchTest`, `WidgetPreviewTest`, `launcher_task_widget_picker_preview_order`, `launcher_task_private_space_settings_entry_only_api36`, `launcher_task_existing_nav_kinds_unchanged`, `launcher_task_main_activity_filters_unchanged`; Artefakt `rdt-apks` |
| `android-device` | `ubuntu-24.04` mit KVM | 240 | android-build, desktop-bins | `sudo` für die KVM-Regel | Emulator API 36 (`google_apis`, `x86_64`) über `reactivecircus/android-emulator-runner@v2`: `PermTaskTest`, `DriveTaskProviderDeviceTest`, `VaultTaskTest`, `ScreenTaskTest`, `LauncherTaskTest`, Bestand `UiTaskTest` und `IntentsTaskTest`; Gerätepaar Emulator ↔ Runner nach `native/review-task-device.py`; `appops … PROJECT_MEDIA allow`, `settings put secure enabled_accessibility_services`, `cmd role add-role-holder`, `pm create-user` (privates Profil und Arbeitsprofil), `appwidget grantbind`. **Annahme:** Das Systemabbild API 36 ist für den Emulator-Runner verfügbar; sonst laufen die Instrumentierungen auf API 35, und der Bericht nennt das |
| `android-device-api30` | `ubuntu-24.04` mit KVM | 150 | android-build, desktop-bins | `sudo` für die KVM-Regel | Emulator API 30: `launcher_task_widget_bind_config_resize`, `launcher_task_devices_widget_only_when_enabled`, `launcher_task_devices_widget_click_opens_device`, `perm_task_android_screen_approval_actions_are_activity_intents`, `screen_task_android_consent_action_is_activity_intent`, `launcher_task_gestures_with_accessibility` (Annahme A38 aus S5) |

**Meilenstein → Job, Stufe, Abnahmesignale** (Signale = Fundstelle aus D1 plus die dort genannten Ergänzungen):

| Meilenstein | Paket | Job: Stufe | Signale |
|---|---|---|---|
| PRM-M1 | U01 | linux-core: unit; windows-core: unit | R 302, `perm_task_systemd_unit_props_v249_compatible` |
| PRM-M2 | U01 | linux-core: perm-pair | R 314 |
| PRM-M3 | U04 | linux-core: perm-pair (`perm_task_pending_surface_cli_only_without_display` ohne `DISPLAY`); linux-desktop: gui-harness | R 326 |
| PRM-M4 | U04 | linux-core: perm-pair | R 336 |
| PRM-M5 | U01, U18 | linux-root: perm-linux-root; linux-2204: perm-2204 | R 348 |
| PRM-M6 | U18 | linux-root: perm-linux-helper | R 359 |
| PRM-M7 | U24 | windows-core: perm-windows | R 370 |
| PRM-M8 | U07, U24 | linux-core: unit, perm-pair; windows-core: perm-windows (Konten) | R 381 |
| PRM-M9 | U07, U24 | linux-desktop: gui-harness | R 391 |
| PRM-M10 | U21, U27 | linux-core: unit (Bridge); android-device; android-device-api30 | R 401 |
| PRM-M11 | U42 | prepare: static | R 409 |
| DRV-M1 | U02 | linux-core: unit; windows-core: unit | L 233 |
| DRV-M2 | U02 | windows-core: drive-dokany-regression; linux-core: unit | L 243, `drive_task_host_session_update_stop_unmounts_and_exits` |
| DRV-M3 | U06 | linux-core: unit; windows-core: drive-cli; linux-root: drive-fuse | L 253 |
| DRV-M4 | U12 | linux-root: drive-fuse | L 264 |
| DRV-M5 | U17 | linux-desktop: drive-desktop | L 272 |
| DRV-M6 | U17 | linux-core: drive-devices, unit; windows-core: drive-cli | L 283 |
| DRV-M7 | U14 | linux-root: drive-vault; windows-core: drive-vault | L 292 |
| DRV-M8 | U23 | android-build (Robolectric); android-device | L 302 |
| DRV-M9 | U36, U42 | prepare: static | L 308 |
| DRV-M10 | U39 | Stufen dieser Suite | – |
| VLT-M1 | U05 | linux-core: unit; windows-core: vault-unit | T 448 (arm64 `not_run`, siehe Bauregel) |
| VLT-M2 | U08 | linux-core: unit | T 459 |
| VLT-M3 | U08 | linux-core: unit | T 468 |
| VLT-M4 | U11 | linux-core: unit; linux-root: vault-linux; windows-core: vault-windows | T 478 (Zustellung durch das Betriebssystem `not_run`, K13) |
| VLT-M5 | U11 | linux-core: unit, vault-interop | T 486 |
| VLT-M6 | U20 | linux-core: unit, vault-klartext; linux-root: vault-klartext-drive | T 495 |
| VLT-M7 | U05, U20 | linux-core: vault-sync, vault-interop | T 506 |
| VLT-M8 | U26 | linux-core: vault-compare, vault-interop | T 515 |
| VLT-M9 | U26 | linux-core: vault-cli, vault-mcp | T 524 |
| VLT-M10 | U29 | linux-desktop: gui-harness | T 533 |
| VLT-M11 | U31 | android-device | T 542 |
| VLT-M12 | U14 | linux-core: unit; linux-root: vault-mount; windows-core: vault-mount | T 551 |
| VLT-M13 | U42 | prepare: static | T 558, `vault_task_docs_rows_t1_t22` |
| VLT-M14 | U39 | Stufen dieser Suite | – |
| SCR-M1 | U09 | linux-core: unit; windows-core: unit | B 306 |
| SCR-M2 | U10 | linux-core: screen-pair; linux-desktop: gui-harness | B 318 |
| SCR-M3 | U13 | linux-desktop: screen-x11; windows-screen; linux-core: unit | B 330, `screen_task_host_worker_exits_on_update_stop` |
| SCR-M4 | U16 | windows-screen | B 342 |
| SCR-M5 | U19 | linux-desktop: screen-x11; linux-wayland: screen-wayland, kde | B 354 |
| SCR-M6 | U22 | windows-screen; linux-desktop: screen-x11; linux-wayland: screen-wayland, kde | B 366 |
| SCR-M7 | U25 | linux-desktop: gui-harness; linux-core: unit | B 378 |
| SCR-M8 | U27 | android-device; android-device-api30 | B 391 |
| SCR-M9 | U28 | android-device | B 403 |
| SCR-M10 | U22, U36 | linux-core: unit; prepare: static | `screen_task_cisco_license_verbatim`, `screen_task_installer_lists_screen_licenses`, `screen_task_android_has_no_openh264` (W6) |
| SCR-M11 | U42 | linux-core: docs-cli; prepare: static | B 422 |
| SCR-M12 | U39, U41 | Stufen dieser Suite | – |
| DSK-M1 | U03 | linux-core: unit; windows-core: unit | D 532 |
| DSK-M2 | U03 | linux-core: unit; linux-desktop: gui-harness; windows-desktop: desktop-gui-harness; prepare: static | D 544 |
| DSK-M3 | U15, U36 | linux-core: unit; linux-desktop: desktop-linux-x11; windows-desktop: desktop-windows; prepare: static | D 562 |
| DSK-M4 | U30 | linux-core: unit; linux-desktop: gui-harness | D 574 |
| DSK-M5 | U32 | windows-desktop: desktop-windows | D 586 (Prüfungen mit Explorer-Bezug `not_run`, wenn `Progman` fehlt) |
| DSK-M6 | U34 | linux-desktop: desktop-linux-x11 | D 598 |
| DSK-M7 | U37 | linux-wayland: desktop-linux-wayland, kde; linux-core: unit; prepare: static | D 610 |
| DSK-M8 | U34 | linux-gnome; prepare: static | D 622 |
| DSK-M9 | U35 | android-device | D 634 |
| DSK-M10 | U35 | android-build; android-device | D 646 |
| DSK-M11 | U38 | android-build; android-device; android-device-api30 | D 658 |
| DSK-M12 | U38, U40 | android-build; android-device | D 670 (privater Bereich `not_run`, wenn `pm create-user` scheitert, Annahme A34) |
| DSK-M13 | U40 | android-device; android-device-api30; linux-core: unit | D 682 |
| DSK-M14 | U43 | prepare: static; linux-core: docs-cli | D 690 |
| DSK-M15 | U39, U41 | Stufen dieser Suite | – |
| INT-LIC | U33, U36 | prepare: licenses, static; linux-core: unit | C4 |

**Erhaltungsfälle für bestehendes Verhalten** (laufen in den genannten Stufen mit):
- S1 (linux-core, windows-core): `perm_task_port_full_shell_0_5_180`, `perm_task_port_program_list_0_5_182`, `perm_task_port_run_as_root_worker`, `perm_task_port_disabled_keeps_settings`, `perm_task_write_legacy_form_bytes_unchanged`, `perm_task_old_reader_sees_new_grant_off`, `perm_task_relation_proof_matches_exec_auth`, `perm_task_old_client_no_pending_frame`, `perm_task_old_client_survives_long_approval`, `perm_task_new_client_old_host`, `perm_task_cli_enable_alias_unchanged`, `perm_task_audit_rotation_unchanged`, `perm_task_own_account_path_unchanged`, `perm_task_windows_own_account_path_unchanged`.
- S2: die fünf Windows-Bestandsharnesses aus DRV-M2, `drive_task_dialect_windows_table_unchanged`, `drive_task_types_split_serde_unchanged`, `drive_task_ipc_mount_types_serde_unchanged`, `drive_task_plain_spool_bytes_unchanged`, `drive_task_registry_v0_5_182_fixture_roundtrip_identical`, `drive_task_registry_old_parser_ignores_new_fields`, `drive_task_existing_drive_after_restart_unchanged`, `drive_task_smb_mount_still_refused`, `drive_task_devices_single_peer_mounts_unchanged`, `drive_task_windows_drive_ignores_pending_attributes`; dazu die Bestandsverträge aus DRV-M10 (Stage-Namen `.se-mount-*`, SMB-Ablehnung, JSON-Form von `MountSnapshot`, CLI-Fehlertexte aus der Kern-Lesung Abschnitt 5).
- S3: `vault_task_existing_locators_unchanged`, `vault_task_generic_sync_rejects_vault_endpoint`, `vault_task_plan_rules_default_single_phase` (OE4: Ein-Phasen-Standard aller bestehenden Jobarten), `vault_task_vault_jobs_dir_invisible_to_jobs_loader`, `vault_task_ipc_err_code_backward_compatible`, `vault_task_vault_error_envelope_roundtrip_and_legacy_text`.
- S4: `screen_task_fs_exec_unchanged` in jeder Paar-Stufe (linux-core: screen-pair, windows-screen, android-device), `screen_task_old_peer_unsupported`.
- S5: alle Signale der Erhaltungsfälle F1–F20 aus `strang-desktop.md` Abschnitt F.
- Bestand: `mcp_task_catalog_is_complete_and_within_host_limits` und `mcp_task_only_new_update_stop_requests_end_the_server` (linux-core: unit), `UiTaskTest` und `IntentsTaskTest` (android-device).

**Nie als bestanden gemeldet** (Grund steht im Bericht): Media-Foundation- und GPU-Kodierer ohne Media Foundation auf dem Runner (K6); KDE-Prüfungen ohne startendes KWin (K3); netem ohne `sch_netem` (S4 A17); GNOME kopflos ohne startende Shell (S5 A19); Fedora-SELinux (S1 A5); Windows 11 Home und Konten mit Microsoft- oder Entra-Anmeldung (S1 A6, A7); Zustellung der Bildschirmsperre (S3 K13); Prüfungen mit Explorer-Bezug ohne `Progman` (S5 A7); privater Bereich (S5 A34) und Rollenwechsel (S5 A32), wenn der Emulator sie nicht erlaubt; Google-Drive-Tresor ohne Zugangsdaten; Windows-Tresor-Laufwerk ohne Dokany; arm64-Kompilierung (Bauregel); alle Handtests aus den Annahmen der Strangpläne.

### D4 Commit-Plan, Graphify, Release

1. **Commits** (nur der Hauptagent): Je zusammenhängendem Meilenstein ein Commit nach dem Bericht des Pakets; Pakete mit zwei Meilensteinen ergeben zwei Commits, U01 drei (zuerst `ExitType`, dann PRM-M1, dann PRM-M2), U14 zwei (VLT-M12, dann DRV-M7). Jedes R-Paket ergibt einen Commit „Registrierungen Phase N“. Betreffzeile im Stil des Bestands (`feat(…)`, `fix(…)`, `chore(…)`, `docs(…)`) mit der Meilenstein-ID. Die Commit-Nachricht endet mit der Zeile `[task candidate]`; Attributionszeilen der Sitzung stehen davor.
2. **Graphify:** Nach dem R-Paket jeder Phase mit nativen Änderungen (P1–P14) frischt der Hauptagent den Root-Graphen nach AGENTS.md auf. Erst prüft er, dass die Ziele im Root-Ordner `graphify-out/` liegen; dann entfernt er nur `graphify-out/manifest.json`, `graphify-out/graph.json` und `graphify-out/GRAPH_REPORT.md` und führt `graphify extract native/src --out . --no-cluster` und `graphify cluster-only . --no-viz --no-label` aus. Das Ergebnis ist ein eigener Commit `chore(graphify): …` mit `[task candidate]`.
3. **Push** am Ende jeder Phase (Umsetzungs-, R- und Graphify-Commits zusammen). Zwischenläufe von `build.yml` sind Entwicklungsstände (L1).
4. **Suite:** Nach dem Push von P15 ein Dispatch von `rechte-desktop-tresore-task.yml` mit der Kandidaten-SHA. Danach wertet der Hauptagent die Berichte aus und behebt jeden Befund mit einem Commit. Bekannte Folge des ersten Laufs: Er committet `native/Cargo.lock` und `third-party/rust-crates/THIRD-PARTY-LICENSES.txt` aus dem Artefakt `rdt-prepare` (L1, L2). Danach dispatcht er dieselbe Suite erneut, nur wenn eine Bestätigung nötig ist.
5. **Release v0.5.183:** genau eine Transaktion, wenn die Suite bis auf ausgewiesene `not_run` grün ist. Zuerst die Vorprüfung nach AGENTS.md, einschließlich `cargo-about` in der festen Version (C4). Dann ein Dispatch von `build.yml` mit `complete_release_source_sha` = finale `main`-SHA. Das ist der bestehende Weg: Job `complete-release` mit 360 min, Wrapper `native/publish-release-local.ps1` mit 330 min, also mindestens 2 h. Der Wrapper hebt `native/Cargo.toml` und `release-native/update-feed/version.txt` auf 0.5.183, baut, prüft die Lizenzdatei (`-Mode Check`), committet, setzt den Tag `v0.5.183` und veröffentlicht. Der Hauptagent prüft den Stand höchstens alle 30 min. Danach prüft er das GitHub-Release v0.5.183 mit den Assets aus AGENTS.md: Windows-Installer; Windows-Programm, Updater und `se` samt Hashes; Linux-Programm, Updater und `se` samt Hashes; `smart-explorer-android.apk` samt Hash; `install-linux.sh`; Kontextmenü-DLL; Share-Server-Payloads; `version.txt`. Release-Text nach C5.
6. **Nach dem Release:** ein Doku-Commit ohne neues Release. Er setzt in `docs/TODO.md` die Einträge dieses Stapels auf ✅ mit Link auf v0.5.183 (darunter `LUECKEN-WINDOWS-RUNAS`) und trägt in `plan.md` den Abschluss ein.

### D5 Abnahme: Nutzeranforderungen → Pakete und Tests

| Anforderung | Pakete | Abnahmesignale (Auswahl, vollständig in D3) |
|---|---|---|
| E1 Rechte an einen Nutzer gebunden, mit vertretbarem Aufwand einrichtbar, `ls` ohne Einzelfreigabe | U01, U04, U07, U21, U24 | `perm_task_allow_own_ls_runs`, `perm_task_gui_access_dialog_one_confirm`, `perm_task_android_access_dialog_two_actions`, `perm_task_cli_grants_access_json_shape`, `perm_task_cli_room_all_members_snapshot`, `perm_task_ask_once_starts_and_logs`, `perm_task_expiry_ends_sessions` |
| E2 Eingehende Terminals als anderes OS-Konto, Rechte durch das OS | U01 (Behebung `ExitType`), U18, U24 | `perm_task_remote_account_ls_runs_linux_root`, `perm_task_linux_root_pam_session`, `perm_task_linux_helper_setup_and_remote_account`, `perm_task_windows_helper_setup_remote_account`, `perm_task_windows_terminal_as_remote_account`, `perm_task_remote_account_cannot_read_ipc_token_linux`, `perm_task_remote_account_cannot_read_ipc_token_windows`, `perm_task_exec_linux_ubuntu2204_starts` |
| E3 Recherche Nutzerwechsel auf Android, ehrlich angezeigt | U21, U42, U43 (Recherche in `docs/refs/exec-konto-android-2026-10-17.md`) | `perm_task_bridge_status_account_choices_android`, `perm_task_android_access_dialog_two_actions`; TODO-Eintrag Android-Kontexte root/Shizuku (C5) |
| E4 se Desktop auf Windows, Linux, Android; Gerät als Ordner; jede Datei öffnen | U12, U17, U23, U15, U30, U32, U34, U37, U40 | `drive_task_devices_folder_appears_without_manual_steps`, `drive_task_devices_offline_error_not_empty`, `drive_task_android_roots_follow_settings`, `desktop_task_open_device_spawns_gui_location`, `desktop_task_open_device_file_manager_path`, `launcher_task_tile_opens_device_location`, `desktop_task_windows_nav_pane_follows_device_root` |
| E5 Zum Bildschirm jedes Geräts wechseln, direkt und in Gruppen, sehen und bedienen; schnelle, zuverlässige Kompression | U09, U10, U13, U16, U19, U22, U25, U27, U28 | `screen_task_grid_switch_reuses_connection`, `screen_task_android_grid_switch`, `screen_task_pair_bench_h264`, `screen_task_relay_only_bench`, `screen_task_recovery_after_injected_reset`, `screen_task_windows_sendinput_text_reaches_window`, `screen_task_x11_xtest_text_and_click`, `screen_task_android_gesture_tap_lands`, `desktop_task_screen_view_args_parse_in_s4`, `launcher_task_screen_nav_kinds` |
| E6 Einführung nahtlos und kollisionsfrei, Windows-Tiefe recherchiert, Android als vollständiger Launcher mit Widgets | U15, U32, U34, U37, U35, U38, U40, U42 (Windows-Tiefe und No-Go „Einbetten in den Explorer-Desktop“ in README und GOTCHAS) | `desktop_task_windows_surface_bottom_toolwindow`, `desktop_task_windows_surface_stays_bottom_after_click`, `desktop_task_x11_below_with_foreign_desktop`, `desktop_task_layer_shell_bottom_surface`, `desktop_task_gnome_ding_takeover_and_restore`, `launcher_task_enable_requests_role`, `launcher_task_widget_bind_config_resize`, `launcher_task_pin_widget_request`, `launcher_task_private_space_hidden_when_locked`, `launcher_task_badge_from_test_notification` |
| E7 Launcher und Desktop getrennt hinzufügbar, aktivierbar, vollständig deaktivierbar; Open Source wo sinnvoll | U15, U35, U33 | `desktop_task_enable_disable_leaves_no_trace`, `desktop_task_remove_without_ledger`, `desktop_task_absent_starts_nothing`, `launcher_task_components_disabled_by_default`, `launcher_task_disable_returns_home`, `launcher_task_remove_deletes_files`, `licenses_task_new_batch_crates_listed` |
| E8 Cryptomator-Tresore öffnen und bearbeiten | U05, U08, U11, U14, U20, U26, U29, U31 | `vault_task_cryptolib_file_gcm_vector`, `vault_task_siv_mode_512_cases`, `vault_task_backend_read_write_roundtrip_gcm`, `vault_task_session_unlock_once_all_clients`, `vault_task_create_then_cryptofs_reads`, `vault_task_password_change_then_cryptofs_unlocks_new`, `vault_task_gui_unlock_states`, `drive_task_vault_drive_letter_rw`, `VaultTaskTest` |
| E9 Tresor-Sync ohne Ordnereinrichtung, keine automatische Konfliktauflösung, Vergleichsmodus | U05, U20, U26, U29, U31 | `vault_task_sync_seed_replica_then_seeding_false`, `vault_task_sync_one_sided_changes_both_directions`, `vault_task_sync_both_changed_untouched_conflict`, `vault_task_sync_unreachable_side_no_deletes`, `vault_task_compare_groups_match_preview`, `vault_task_compare_keep_both_valid_cipher_name`, `vault_task_gui_compare_rows_and_actions`, `vault_task_plan_rules_default_single_phase` |
| E10 Cryptomator-Einhängewege gegen Dokany geprüft, Linux-Laufwerke | U02 (Dokany bleibt, Fehlerabbildung behoben), U12, U14, U17, U42 (Entscheidung und Begründung in README, GOTCHAS; TODO `WINFSP-AB`, `DOKANY-WATCH`) | `drive_task_windows_unmapped_error_is_io_device_not_access_denied`, `drive_task_windows_host_unreachable_maps_1232`, `drive_task_fuse_mount_local_fixture_rw`, `drive_task_fuse_sftp_container_rw`, `drive_task_fuse_editor_save_rename_over`, `drive_task_vault_linux_mountpoint_tmpfs_sticky_locked`, `drive_task_gtk_bookmark_only_outside_glib_rule` |

E11 (Planen in Hauptsträngen, danach direkt ausführen) ist mit diesem Plan erfüllt; die Ausführung beginnt mit P1.
