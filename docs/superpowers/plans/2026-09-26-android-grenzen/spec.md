# Android-Grenzen aufheben und Code-Review – Spec

Stand: 2026-09-26. Auftrag: „review den code und mach die fixes … die ‚was nicht geht‘ wirkten wie
Fehler, die auch wenn möglich, außer das Einbinden als Laufwerk.“ Arbeitsweise wie beim ersten
Android-Batch: selbstständig, ohne Zwischenstopp; Annahmen stehen hier.

## A Definition

- **R Code-Review:** Android-App (`android/`), Rust-Fassade (`native/src/mobile`, `native/android-bridge`)
  und die Android-Adapter; jeder bestätigte Fehler wird behoben.
- **G1 Ersetzen auf SFTP:** Sync-Aktualisierungen, hochgeladene Bearbeitungen und Uploads mit „Ersetzen“
  funktionieren auf reinem SFTP (ohne Remote-Agent): atomar über `posix-rename@openssh.com` auf einem
  zweiten SFTP-Kanal derselben SSH-Verbindung, wenn der Server die Erweiterung anbietet (OpenSSH und
  kompatible). Bietet ein Server sie nicht an, bleibt es bei der klaren Fehlermeldung (kein unsicherer
  Zwei-Schritt-Tausch).
- **G2 Ersetzen auf WebDAV und FTP:** je ein einziger serverseitiger Aufruf – WebDAV `MOVE` mit
  `Overwrite: T` (RFC 4918), FTP `RNFR`/`RNTO` (auf POSIX-Servern `rename(2)`, also atomar). Kein
  Zwei-Schritt-Tausch im Client: ein Verbindungsabbruch zwischen zwei Schritten ließe die Datei sonst
  fehlen, und der nächste Sync würde das als Löschung lesen.
- **G3 Hochladen auf FTP:** private Upload-Stufe mit Zufallsnamen nach Abwesenheitsprüfung (FTP kennt
  kein exklusives Anlegen), Veröffentlichen per `RNFR`/`RNTO` nach Abwesenheitsprüfung, Ersetzen über G2.
  Grenze: gegen einen gleichzeitigen Schreiber im selben Moment schützt FTP nicht.
- Unverändert: Laufwerks-Mounts verlangen weiter die vollständigen atomaren Garantien (reines SFTP,
  WebDAV und FTP bleiben dort schreibgeschützt).
- **G4 Befehle anderer Geräte auf dem Telefon:** Das Telefon kann Ziel von „Befehl ausführen“ sein –
  nur nach denselben ausdrücklichen Freigaben wie am Desktop (je exakter Geräte-Identität, Direkt-Gerät
  oder Raummitglied). Ausführung in der App-Sandbox über `/system/bin/sh`; jeder Befehl läuft unter
  einem eigenen Zwischenprozess, der Subreaper ist, sodass Abbruch, Zeitlimit und Entzug den ganzen
  Prozessbaum beenden (auch `setsid`-Kinder und verwaiste Enkel) und Sync-Hooks der App unberührt
  bleiben. Grenzen: läuft als App-Nutzer und liest damit auch App-Daten (gespeicherte
  Verbindungszugänge, Share-Identität, Drive-Tokens); Android kann Kindprozesse im Hintergrund
  beenden oder einfrieren (Ref `android-child-processes.md`); nach einem App-Absturz räumt der nächste
  Start verbliebene Befehle auf.
- **G5 SMB:** neues Backend „SMB“ (SMB 2/3, Benutzer/Passwort, optional Domäne) über einen reinen
  Rust-Client; Durchsuchen, Kopieren, Sync, Bearbeiten wie bei den anderen Remotes – in diesem Batch
  nur in der Android-App (der Desktop behält UNC/Netzlaufwerke; Backend und Schema existieren
  plattformübergreifend, das Einbinden als Laufwerk wird für SMB ausdrücklich abgewiesen).
  Links/Junctions auf dem Server werden als Link erkannt (Reparse-Attribut) und nie rekursiv betreten;
  serverseitig verfolgte Samba-Symlinks sind nicht erkennbar (Grenze). DFS aus, ohne Verschlüsselung,
  sofern der Server sie nicht verlangt (Hinweis im Formular).
- **G6 „In Smart Explorer öffnen“:** Gegenstück zum Explorer-Kontextmenü: Smart Explorer erscheint,
  wenn eine andere App einen Ordner zum Öffnen übergibt, und öffnet ihn. Dateien werden bewusst nicht
  beansprucht (sonst stünde die App bei jedem Datei-Öffnen als Betrachter zur Wahl); für Dateien bleibt
  „Teilen → Smart Explorer“.
- Nicht: Einbinden als Laufwerk; `Android/data`/`Android/obb` anderer Apps (Android 11+ sperrt sie für
  „Alle Dateien“ und SAF; der einzige Weg ohne Root ist Shizuku mit Zusatz-App, Wireless-Debugging-Kopplung
  und Neustart nach jedem Reboot, unter Android 16 laut offenem Shizuku-Issue unzuverlässig – als Option
  in `docs/TODO.md` festgehalten, recherche.md E5).

## B Bedienung

- G1–G3: keine neue Bedienung; was bisher mit Fehlermeldung abbrach, läuft durch. Der Dialog
  „Nicht ersetzbar“ bleibt nur für Server ohne sicheres Ersetzen; der Hinweis am Remote-Agent-Schalter
  entfällt.
- G4: Share-Seite → Abschnitt „Befehle auf diesem Telefon“: je Gerät/Raummitglied (wie am Desktop,
  kein globaler Schalter) „Erlauben…“ → Warnung mit den Android-Risiken + Häkchen „Verstanden“ →
  „Aktivieren“; „Entziehen“ ohne Rückfrage. Liste laufender und letzter Host-Befehle mit Stopp. Solange
  ein fremder Befehl läuft, zeigt eine laufende Benachrichtigung „<Gerät> führt einen Befehl aus“ mit
  „Stopp“. Nicht verfügbarer Anbieter: Hinweis mit Grund, Erlauben gesperrt.
- G5: Verbindung hinzufügen → Protokoll „SMB“: Host, Port (445), Freigabe, Domäne (optional),
  Benutzer, Passwort, Startordner; Test/Speichern wie bei SFTP. Freigabe und Domäne sind nur
  Formularfelder (gespeichert als `root=/<freigabe>/<startordner>`, `user=DOMÄNE\benutzer`); ein
  eingefügtes `\\host\freigabe\pfad` oder `smb://host/freigabe/pfad` im Host-Feld wird zerlegt.
  Eigene Meldungen für „nur SMB1“, „Freigabe nicht gefunden“, Anmeldung.
- G6: In anderen Apps „Öffnen mit“ → Smart Explorer → Tab „Dateien“ mit dem Ordner.

## C Layout und Bedienkosten

Keine neuen Seiten; SMB ist eine weitere Protokollwahl im bestehenden Formular, die Befehlsfreigabe ein
Schalter auf der Teilen-Seite.
