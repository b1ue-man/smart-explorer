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
  nur nach denselben ausdrücklichen Freigaben wie am Desktop. Ausführung in der App-Sandbox über
  `/system/bin/sh`, eigene Prozessgruppe, Zeitlimit, Ausgabegrenzen, Stopp. Grenzen: läuft als App-Nutzer
  (Dateizugriff der App, Toybox-Befehle), endet mit dem App-Prozess.
- **G5 SMB:** neues Backend „SMB“ (SMB 2/3, Benutzer/Passwort, optional Domäne) über einen reinen
  Rust-Client; Durchsuchen, Kopieren, Sync, Bearbeiten wie bei den anderen Remotes, auf Android und
  Linux-Desktop (Windows behält UNC).
- **G6 „In Smart Explorer öffnen“:** Gegenstück zum Explorer-Kontextmenü: Smart Explorer erscheint bei
  „Öffnen mit“ für Ordner und Dateien anderer Apps und öffnet den Ordner (bei Dateien den Ordner der
  Datei).
- Nicht: Einbinden als Laufwerk; `Android/data`/`Android/obb` bleiben gesperrt (Betriebssystem).

## B Bedienung

- G1–G3: keine neue Bedienung; was bisher mit Fehlermeldung abbrach, läuft durch. Der Dialog
  „Nicht ersetzbar“ bleibt nur für Server ohne sicheres Ersetzen; der Hinweis am Remote-Agent-Schalter
  entfällt.
- G4: Share-Seite → eigenes Gerät: Schalter „Befehle von freigegebenen Geräten erlauben“ mit
  Warnhinweis; je Gerät/Raum die Freigabe wie am Desktop. Laufende Befehle erscheinen als Aufgabe mit
  Stopp.
- G5: Verbindung hinzufügen → Protokoll „SMB“: Host, Freigabe, Benutzer, Passwort, Domäne (optional),
  Startordner; Test/Speichern wie bei SFTP.
- G6: In anderen Apps „Öffnen mit“ → Smart Explorer → Tab „Dateien“ mit dem Ordner.

## C Layout und Bedienkosten

Keine neuen Seiten; SMB ist eine weitere Protokollwahl im bestehenden Formular, die Befehlsfreigabe ein
Schalter auf der Teilen-Seite.
