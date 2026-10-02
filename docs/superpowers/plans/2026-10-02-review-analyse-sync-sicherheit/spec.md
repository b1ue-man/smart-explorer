# Review-Batch RV1: Fern-Analyse, Sync-Zuverlässigkeit, Share-Sicherheit – Spec

Stand: 2026-10-02. Batch-Kürzel **RV1**, Test-Präfix `review_task_`.

## Auftrag (wörtlich, gekürzt um Tippfehler)

> Deine Aufgabe ist extrem wichtig. Führe umfangreiche, saubere Reviews durch, um den gegebenen
> Funktionsumfang auf Bugs, Lücken und anderes zu prüfen. Unter anderem:
> 1. Die Analyse wird, wenn sie von anderen Geräten via Direct Share ausgeführt wird, immer noch
>    anders ausgeführt als wenn das Gerät sie selbst ausführt. Verbinde ich mich mit dem Handy zum PC
>    und lasse die Analyse laufen, läuft sie langsam.
> 2. Die Synchronisierungen: stelle sicher, dass alle Funktionen über alle Betriebssysteme verlässlich
>    sind. Die Echtzeit scheint nicht verlässlich. Überlege dir verbesserte, OS-spezifisch angepasste
>    Arten der Überwachung. Stelle sicher, dass das Synchronisieren wirklich verlässlich funktioniert;
>    es darf nicht scheitern, wenn es für Backups verwendet wird.
> 3. Sicherheit: Datentransfers usw. zwischen Direct Shares und Räumen sollen nicht nur verlässlich,
>    sondern per Standard sicher sein, unsicherer nur per Opt-out.
> Folge /arbeitsweise, arbeite selbstständig; der Plan wird nicht abgenommen.

## Grundlage

Drei Review-Workflows (Finder je Dimension, unabhängige Gegenprüfung, Vollständigkeits-Kritiker):
[Analyse](review-befunde-analyse.md) (A01–A37), [Sync](review-befunde-sync.md) (Y…),
[Sicherheit](review-befunde-sicherheit.md) (S01–S66). Jede Funktion unten nennt die Befunde, die sie
schließt. Widerlegte Befunde sind dort dokumentiert. Was bewusst offen bleibt, steht unter „Nicht in
RV1“ und kommt auf das Board (`docs/TODO.md`).

## Leitentscheidungen (selbst getroffen, mit Grund)

1. **Sicher für Neues, nichts still kaputt für Bestehendes.** Neue Profile, Freigaben, Räume,
   Kopplungen und Server-Adressen starten sicher. Bestehende, gespeicherte Einstellungen behalten ihre
   Wirkung (sonst brechen z. B. Backups auf eine Freigabe), werden aber sichtbar als „unsicher“
   markiert und lassen sich mit einem Klick härten. Ausnahme: Dinge, die nie jemand bewusst so wollte
   und die echten Schaden ermöglichen (eigene App-Daten in einer Freigabe, Kopplung ohne PIN), werden
   für alle sofort abgestellt.
2. **Analyse und Duplikatsuche laufen auf dem Gerät, dem die Daten gehören.** Über die Leitung gehen
   nur Fortschritt und Ergebnis – in jeder Richtung (Handy→PC, PC→PC, PC→Handy), für Direkt-Kontakte
   und Raum-Mitglieder gleich. Ältere Gegenstellen bekommen einen Weg, der nie die ganze Freigabe
   herunterlädt.
3. **Ein Sync-Lauf ist nie „alles oder nichts“.** Was erledigt ist, bleibt erledigt (auch bei Fehlern,
   Abbruch, Absturz); was fehlschlug, wird beim nächsten Lauf wiederholt; was nicht lesbar ist, wird
   ausgelassen und gemeldet, aber nie als gelöscht gedeutet.
4. **Echtzeit = Betriebssystem-Ereignisse + Sicherheitsnetz.** Windows `ReadDirectoryChangesW`,
   Linux/Android `inotify` (eigene, schlanke Adapter statt `notify` 6.1.1, das Überläufe unter Windows
   verschluckt und unter Linux Symlinks folgt, siehe `docs/refs/sync-change-detection.md`), dazu ein
   regelmäßiger Kontroll-Scan und – wo es keine Ereignisse gibt (Netzlaufwerke, Fernziele) – eine
   langsame, sichtbare Abfrage. Ereignisse sind nur Auslöser; was sich geändert hat, entscheidet weiter
   die Sync-Engine.
5. **Grenzen nur mit Grund.** Feste Obergrenzen (1 Mio. Einträge, 200 Duplikat-Kandidaten, 16-MiB-
   Listen) werden durch speicherabhängige oder protokollbedingte Grenzen bzw. Streaming ersetzt.
6. **Kompatibilität gemischter Versionen.** Jede Protokollerweiterung (Share-Anfragen, Server-Anmeldung,
   Präsenz-MAC) wird über Fähigkeiten ausgehandelt; alte Gegenstellen funktionieren weiter, bekommen aber
   keine neuen Rechte. Neue Server lassen alte Clients nur dort zu, wo keine authentifizierte Bindung
   verletzt wird.

## Teil A – Definition (was am Ende existiert)

### Fern-Analyse und Duplikate (Punkt 1)

- **FA1 Android analysiert auf dem Host.** Die Speicheranalyse eines Share-Orts (und eines SSH-Agent-
  Orts) läuft über denselben Auswahlweg wie am Desktop (`scan_remote`): Host-Worker zuerst, ältere Pfade
  nur als Rückfall. Android zeigt die Phasen (wartet auf Worker, wird vorbereitet, durchsucht, überträgt,
  prüft), den Übertragungsfortschritt, Hinweise und geschützte Bereiche des Hosts. Verbindung wird vor
  dem Start geprüft (`resolve_live`). Während ein Analyse-/Duplikat-Task gegen ein anderes Gerät läuft,
  hält Android einen Partial-Wakelock (an die Task-Dauer gebunden). Ältere Ergebnisse werden freigegeben,
  sobald ein neues startet. [A02, A15, A06, A11, A23, A33]
- **FA2 Duplikatsuche auf dem Host.** Neue Share-Fähigkeit `duplicate_search_v1`: der Host sucht mit
  seinem lokalen Finder (alle Kandidaten ab Mindestgröße, Stichproben am Dateianfang/-ende, volle Prüfung
  nur bei Gleichstand, parallel, geschützte Bereiche, Freigabe-Begrenzung) und sendet Gruppen +
  Zusammenfassung; Abbruch wirkt sofort. Desktop („Aufräumen“) und Android nutzen sie für Share-Orte.
  Neue Fähigkeit `hash_walk_v1`: der Host berechnet Prüfsummen lokal und streamt sie (für Sync im
  Prüfsummen-Modus gegen Share-Ziele und als Rückfall der Duplikatsuche). Rückfall bei alten Hosts:
  Gruppierung nach Größe aus Listen, Stichproben per `open_read_at` nur für gleich große Kandidaten,
  volle Prüfung nur bei gleichen Stichproben – nie Download aller Dateien. Der bisherige Daemon-Hash-
  Walk (SSH-Agent, Rückfall) überspringt Links, meldet unlesbare Einträge einzeln und reicht Abbrüche
  sofort weiter. [A01, A16, A03, A04, A05, A13]
- **FA3 Andere Kontakte stören laufende Arbeit nicht.** Online/Offline-Wechsel eines Kontakts, ein neues
  Raum-Mitglied oder Präsenz-Daten sind keine Rechteänderung mehr; echte Rechteänderungen schließen nur
  die Verbindungen der betroffenen Beziehung. Laufende Analysen, Übertragungen und Syncs zu anderen
  Geräten laufen weiter. [A30]
- **FA4 Host-Worker so schnell wie lokal.** Die Freigabe-Begrenzung kostet keinen zusätzlichen
  Pfad-Auflösungsaufruf je Ordner mehr (Prüfung nur an Links/Reparse-Punkten und an der Wurzel, Öffnen
  ohne Folgen von Links); Gesamtanalyse „/“ teilt ein Budget über alle Freigaben und läuft verschachtelte
  Freigaben nur einmal; Zähler kommen aus dem Baum (keine Ablehnung wegen Zähler-Abweichung); lange
  Fortschrittspfade werden gekürzt; alle Host-Pfadformen werden auf sichtbare Pfade abgebildet; der
  SSH-Agent-Hinweis stimmt; ältere Clients bekommen ihren v1-Snapshot aus dem schnellen Worker mit
  Teilergebnis statt Abbruch; Analysen werden fair je Gerät zugeteilt; eine Windows-Host-Analyse sagt,
  wenn erhöhte Leserechte lokal mehr sehen würden. [A08, A17, S38, A19, A36, A10, A25, A29, A26, A37,
  A12, A28, A21, S42, A27]
- **FA5 Gleiches Ergebnis, robust übertragen.** Der Bericht trägt Volumen-Belegung, Android-Plattformzahlen
  (Apps, nicht einzeln erfasst; vom Android-Host zwischengespeichert) und geschützte Bereiche strukturiert;
  der Ergebnisbaum wird komprimiert übertragen (`analysis_deflate_v1`); ein Verbindungsabbruch verwirft
  die Host-Arbeit nicht (Ergebnis 10 min abrufbar, Client verbindet einmal neu); der Empfänger nennt sein
  Knotenbudget; Host hält Wakelock/Systemwach-Anforderung, solange Analyse-/Übertragungsströme laufen
  (Android beim Wechsel in den Ruhemodus sofort, Desktop über OS-Anforderung). [A14, A22, A18, A20, A23,
  A31, A32]
- **FA6 Bedienlücken.** Analyse eines eingehängten Share-Laufwerks fragt den Host; „Im Explorer öffnen“
  aus einer Fern-Analyse behält den Ort; gefundene Duplikate auf einem anderen Gerät lassen sich in
  dessen Papierkorb verschieben (`remote_trash_v1`, Host prüft Inhalt vorher erneut). [A09, A35, A34]
- **FA7 Große Ordner über Share.** Listen kommen in Portionen (`list_batches_v1`), ohne 16-MiB-/20-s-Grenze
  je Ordner; ein unlesbarer oder nicht darstellbarer Eintrag macht nicht mehr den ganzen Ordner
  unlesbar. [A24, Y126]

### Sync-Zuverlässigkeit (Punkt 2)

- **FS1 Kein Fortschritt geht verloren.** Nach jedem Lauf (auch mit Fehlern oder Abbruch) wird die Basis
  für alle erledigten Aktionen gespeichert, mit den Signaturen, die die Aktion selbst gesehen hat (kein
  zweiter Voll-Scan, keine „Bearbeitung während des Laufs gilt als synchron“). Beidseitig gleiche Dateien
  ohne Basis werden ohne Konflikt zusammengeführt (Größe + Prüfsumme bei Bedarf). [Y29, Y55, Y32, Y33,
  Y58, Y42, Y53]
- **FS2 Änderungszeit bleibt erhalten.** Kopien bekommen die Änderungszeit der Quelle (lokal, SFTP, FTP mit
  MFMT, WebDAV mit `X-OC-Mtime`, Google Drive, Share, SMB soweit möglich). Vergleiche berücksichtigen die
  Zeitauflösung je Ziel (FAT 2 s, FTP-LIST Minuten/Tage, SFTP Sekunden) und die FAT-Sommerzeit-Stunde.
  Spiegel-Jobs gelten als synchron, wenn beide Seiten ihrer gespeicherten Basis entsprechen – auch wo die
  Zeit nicht übertragbar ist. Ergebnis: Ein Backup kopiert nur Geänderte. [Y30, Y57, Y89, Y113, Y80, Y46,
  Y104, Y47, Y129, Y52]
- **FS3 Falsches oder fehlendes Laufwerk löscht nichts.** Jede lokale Wurzel speichert ihre Identität
  (Datenträger + Wurzelordner). Eine leere Seite, die vorher Einträge hatte, oder eine fremde Identität
  stoppt den Lauf mit klarer Meldung; ein anderes, nicht leeres Laufwerk (Rotation) wird ohne
  Löschübernahme abgeglichen. Löschschutz ist für neue und bestehende Jobs standardmäßig an; ein Stopp
  wegen Massenlöschung wird als „blockiert“ gemeldet und nicht minütlich wiederholt. Inkrementelle
  Spiegel prüfen das Ziel regelmäßig vollständig. [Y31, Y62, Y92, Y39, Y69]
- **FS4 Ein einzelner Problemeintrag stoppt keinen Lauf.** Unlesbare, verschwundene, nicht darstellbare
  Einträge, Pipes/Sockets/Geräte, systemeigene Ordner an Laufwerkswurzeln, eigene Zwischendateien und
  gefilterte Einträge werden ausgelassen, gemeldet und nie als Löschung gedeutet. Groß-/Kleinschreibung
  und Unicode-Normalform werden je Paar richtig behandelt. Größenobergrenzen richten sich nach dem
  Arbeitsspeicher. [Y34, Y88, Y136, Y49, Y87, Y112, Y36, Y68, Y37, Y48, Y102, Y71, Y97, Y125, Y35, Y67,
  Y91, Y120]
- **FS5 Versionen, die wirklich schützen.** Ein Lauf = ein Versionsordner; Aufbewahrung zählt je Datei;
  Bereinigung nach jedem Lauf. Versionen liegen auf dem Datenträger bzw. Server der betroffenen Datei
  (Umbenennen statt Kopieren/Herunterladen), sonst wie bisher in den App-Daten. Kopien sind vor dem
  Veröffentlichen dauerhaft geschrieben; Zwischennamen sind kurz; leere Ordner werden mitgenommen bzw.
  entfernt; bei voller oder schreibgeschützter Gegenseite oder wiederholten Verbindungsfehlern endet der
  Lauf früh (Erledigtes bleibt). [Y38, Y56, Y61, Y105, Y60, Y90, Y131, Y106, Y72, Y96, Y51, Y70, Y108, Y74,
  Y130, Y73, Y133, Y84, Y76, Y75, Y40, Y41, Y63, Y85]
- **FS6 Plattform-Grundlagen.** Linux: Veröffentlichen ohne Ersetzen funktioniert auf NFS/FUSE/ntfs-3g;
  Pipes blockieren nie; Rechte (0600 usw.) bleiben erhalten; Ziele unterhalb von Links/Junctions oberhalb
  der Wurzel funktionieren (Fedora Atomic, verschobene Windows-Ordner, `/sdcard`). Windows: Namen mit `:`
  und anderen verbotenen Zeichen landen nie in Alternate Data Streams; schreibgeschützte Zieldateien
  werden ersetzt; OneDrive-Platzhalter, WOF- und Dedup-Dateien sind normale Dateien. [Y86, Y87, Y94, Y95,
  Y122, Y98, Y100, Y103, Y81, Y99]
- **FS7 Fernziele für Backups.** WebDAV/Nextcloud legt Ordner nur unterhalb der Sync-Wurzel an und liest
  große Ordner; SFTP meldet „nicht gefunden“ richtig, große Listen und Server ohne `posix-rename`
  funktionieren; FTP nutzt MLST/MLSD/MDTM/SIZE statt Voll-Listen je Datei, findet `.local`-Namen,
  bleibt bei langen Übertragungen verbunden und ersetzt auch auf Windows-FTP-Servern; Uploads streamen
  statt komplett zwischenzuspeichern; Drive akzeptiert unter Linux/Android gültige Namen, behält die
  Sync-Identität nach Neuanmeldung, behandelt Google-Dokumente/Verknüpfungen und nutzt den Änderungs-
  Feed; „Ziel voll“ beendet den Lauf mit einer Meldung; Share-Ziele behalten die Sync-Identität nach
  Identitätsreparatur und exportierte Verbindungen werden nicht je Anfrage neu angemeldet. [Y114, Y118,
  Y143, Y115, Y121, Y142, Y116, Y66, Y127, Y128, Y137, Y140, Y117, Y65, Y124, Y132, Y82, Y139, Y43, Y134,
  Y130, Y141, Y123]
- **FS8 Echtzeit-Überwachung.** Ereignisse des Betriebssystems (Windows `ReadDirectoryChangesW`,
  Linux/Android `inotify`) mit Überlauf-Erkennung → Kontroll-Lauf; Watch-Limit/Netzlaufwerk/Fernziel →
  langsame Abfrage (Standard 5 min, sichtbar); Filter des Jobs und eigene Schreibvorgänge lösen nichts
  aus; Entprellung mit Höchstwartezeit; ein Auslöser während eines Laufs startet danach einen weiteren
  Lauf statt verloren zu gehen; offene Auslöser überleben Neustart (Start = Kontroll-Lauf je Echtzeit-
  Job); stündlicher Kontroll-Lauf als Sicherheitsnetz; Android meldet zusätzlich MediaStore-Änderungen.
  Jobs ohne beobachtbare Seite werden abgelehnt bzw. als „Abfrage“ ausgewiesen. [Y02–Y08, Y22, Y93, Y64]
- **FS9 Zeitplan und Ergebnisse.** Jeder Versuch wird festgehalten (letzter Versuch, letzter Erfolg,
  Fehlerserie, Grund); fehlgeschlagene geplante Läufe werden mit Backoff wiederholt, Anmeldefehler nicht
  automatisch; Warteschlange hält Job-IDs und lädt die aktuelle Konfiguration beim Start; ein Paar läuft
  nie doppelt (geräteweite Sperre zwischen Desktop-Fenster, Daemon und Android); eine kaputte Job-Datei
  stoppt nur sich; unabhängige Paare laufen begrenzt parallel, hängende Läufe werden erkannt; Kalender
  verpassen keine Termine (Sommerzeit, Monatsende); Laufzeitdaten getrennt von der Konfiguration; „Beim
  Start“ einmal je Anmeldung. [Y09, Y119, Y78, Y16, Y17, Y18, Y79, Y45, Y20, Y21, Y13, Y15, Y26, Y19, Y28,
  Y27]
- **FS10 Anschluss-Auslöser und Energie.** Linux erkennt angeschlossene Datenträger; Windows erkennt
  USB-Festplatten/SSDs (Bustyp statt „Wechseldatenträger“); ein Anschluss bleibt ausstehend, bis der Job
  lief, und der Job prüft, dass sein Pfad auf dem erkannten Datenträger liegt; Linux-Pause bei Akku/
  getakteter Verbindung funktioniert oder wird nicht angeboten; der Desktop hält das System während
  Sync-Läufen wach; abgeschalteter Windows-Autostart wird erkannt und angezeigt. [Y01, Y10, Y14, Y25,
  Y110, Y24]
- **FS11 Android im Hintergrund.** Dauerbetrieb hält einen Wakelock während Läufen und weckt sich zum
  nächsten Termin; im periodischen Modus gehören gestartete Läufe zum Worker-Fenster; vorübergehende
  Fehler führen zu `retry`; große Läufe laufen im Vordergrunddienst weiter. Fehlt der Allzugriff auf den
  gemeinsamen Speicher (widerrufen, Neuinstallation), laufen Jobs mit lokalem Speicher-Endpunkt nicht
  (keine Liste, keine Löschung), sondern melden „Dateizugriff fehlt“ mit Benachrichtigung. [Y11, Y12,
  Y23, Y144]
- **FS12 Eigene Daten, Bedienung, Benachrichtigung.** App-Daten- und Cache-Ordner sind in jedem Walk und
  jeder Überwachung eine geschützte Auslassung (Heartbeat und Versionen lösen nichts aus und werden nicht
  mitgesichert); Linux-Walks bleiben auf dem Dateisystem der Wurzel (andere Einhängepunkte sind
  Auslassungen, Opt-in zum Überqueren); Filtermuster ignorieren Groß-/Kleinschreibung, wo eine Seite
  sie ignoriert; Dateien über der Grenze des Ziel-Dateisystems (FAT32 4 GiB) werden vorab als „zu groß
  für Ziel“ gemeldet; „Überprüfen“ vergleicht Inhalte (Prüfsumme beim Schreiben, erneutes Lesen bzw.
  Server-Prüfsumme); Vor-/Nach-Befehle funktionieren unter Windows mit Anführungszeichen, laufen vor dem
  Verbinden, nach Abbruch gibt es einen Aufräum-Befehl; die Desktop-Zusammenführung erhält Zeilenenden,
  prüft auf zwischenzeitliche Änderungen und sichert beide Originale; „Nur diese Datei jetzt“ nutzt die
  geplanten Zustände und aktualisiert die Basis; Versionen lassen sich je Job ansehen und
  wiederherstellen (Desktop und Android, lesbares Verzeichnis); Löschen/Umziehen eines Jobs bietet an,
  Basis und Versionen zu entfernen, und eine neue gleichartige Job-Basis erbt keine fremde Basis;
  wiederholte Fehlschläge benachrichtigen (Android-Kanal, Desktop-Hinweis beim Öffnen), Protokolle
  rotieren statt gelöscht zu werden; Schließen/Aktualisieren während eines manuellen Laufs fragt nach
  und wartet begrenzt. [Y145, Y152, Y153, Y154, Y155, Y146, Y147, Y148, Y149, Y150, Y151, Y156, Y44, Y54,
  Y59]

### Share-Sicherheit (Punkt 3)

- **FC1 Freigaben sicher per Standard.** Jede Freigabe hat „Nur lesen“ oder „Lesen und Schreiben“; neue
  Freigaben sind „Nur lesen“; der Host verweigert jede schreibende Anfrage auf Nur-lesen-Freigaben und
  meldet die Rechte in den Fähigkeiten. Neue Profile haben keine Standardfreigabe (kein ganzer
  Benutzerordner mehr); neue Räume starten ohne Freigaben. Die eigenen App-Daten (Identität, Geheimnisse,
  Zugangsdaten) sind nie über Share erreichbar, egal was freigegeben ist. Gespeicherte Verbindungen werden
  nur einzeln und standardmäßig nur lesend freigegeben, mit Warnhinweis. Bestehende Freigaben behalten
  ihre Rechte, werden aber als „Lesen und Schreiben“ (bei ganzem Benutzerordner mit Warnung) angezeigt.
  Die wirkungslose Option „Symlinks außerhalb blockieren“ entfällt. [S04, S21, S36, S63, S65, S39]
- **FC2 Kopplung per PIN sicher.** Vorgeschlagene PIN: zufällig, 6 Ziffern; kürzere/schwache PINs nur mit
  ausdrücklichem „Unsichere PIN erlauben“; ein Angebot endet nach der ersten erfolgreichen Kopplung oder
  nach 5 Fehlversuchen (mit Warnung) und dauert höchstens 30 min; eine Kopplung ohne Bestätigung der
  Gegenseite heißt „gekoppelt – Bestätigung fehlt“ mit „Widerrufen“ statt „fehlgeschlagen“; der Server
  verteilt Startversuche je Verbinder. [S20, S51, S24, S50]
- **FC3 Verschlüsselte Server-Verbindung per Standard.** Eine Server-Adresse ohne Schema bedeutet TLS
  (`wss://`); `tcp://`, `ws://`, `http://` nur mit ausdrücklichem „Unverschlüsselt erlauben“; bestehende
  Klartext-Adressen behalten diese Erlaubnis (Migration) und werden gewarnt angezeigt; kein Rückfall von
  TLS auf Klartext; Relays nur per HTTPS (Klartext-Relay nur mit derselben Erlaubnis); selbst signierte
  Server per Zertifikats-Fingerabdruck (`…#sha256=`) möglich; Nachrichtengrößen begrenzt; die Anmeldung
  sendet keine lokalen IP-Adressen mehr; der Status zeigt „verschlüsselt“ bzw. „⚠ unverschlüsselt“.
  [S01, S02, S03, S18, S27, S43, S13, S56]
- **FC4 Share-Server als harter Vertrauensanker.** `se-share-server` spricht selbst TLS (Zertifikat/
  Schlüssel aus Dateien, auch für das Relay); Klartext nur mit `--allow-plaintext` oder an Loopback (für
  Reverse-Proxy). Geräte melden sich mit ihrem Schlüssel an (Challenge-Signatur); Geräte-ID, Direkt-
  Lookup und Raum-Mitgliedschaft sind an den beweisenden Schlüssel gebunden; Beobachten/Beitreten braucht
  einen aus dem Beziehungsgeheimnis abgeleiteten Nachweis; nur der Besitzer kann seinen Eintrag ändern
  oder abmelden. Alte Clients funktionieren, können aber gebundene Einträge nicht übernehmen. Grenzen je
  Schlüssel und Adresse, Unangemeldete zuerst verdrängt, Grenzen konfigurierbar, Burst passt zum Client,
  WebSocket-Leichen werden erkannt, Relay nur für angemeldete Geräte. [S43, S44, S45, S47, S49, S25, S08,
  S53, S54, S57, S48]
- **FC5 Entziehen wirkt.** Entfernen/Sperren gilt für den Schlüssel (nicht nur die selbst gewählte
  Geräte-ID); Anfragen neuer Geräte mit dem Direkt-Code warten auf Zustimmung (automatisches Annehmen nur
  per Opt-in); Code-Rotation/Identitätsreparatur sperrt keine bestehenden Kontakte mehr aus, sondern
  setzt sie auf „neu bestätigen“; „Wieder erlauben“ für gesperrte Kontakte; Entfernen eines Kontakts
  entzieht auch Freigaben, die über Schlüssel/Knoten zugeordnet sind; alte Entscheidungsnachrichten
  überschreiben keine geprüften Entscheidungen; Entziehen hat Vorrang vor laufenden Reparaturen; Präsenzen
  sind vollständig authentisiert (v2-MAC über alle Felder, Fingerabdruck lokal abgeleitet); Wiederholungen
  werden bis zum Ablauf erkannt; geringe Uhrabweichung führt nicht zu Fehlern. [S19, S22, S05, S23, S29,
  S28, S30, S46, S66, S17, S06, S26, S55, S16, S32]
- **FC6 Host robust gegen Überlastung.** Bekannte Geräte haben reservierte Anmeldeplätze, unbekannte
  kurze Fristen; lokale Steuerverbindungen ebenso; Steueroperationen und Verbindungen werden je Gerät
  begrenzt, lange Läufe in eigenem Pool; rekursives Löschen ist iterativ mit Budget (kein Absturz);
  Lesen prüft Rechte auch während langer Ströme; Ablehnungen verraten keine Details; Wakelocks erst nach
  geprüfter Nachricht. [S07, S60, S37, S41, S42, S64, S40, S12, S52]
- **FC7 Lokal gehärtet.** Linux-App-Daten 0700/Dateien 0600 ab Erstellung; Windows prüft Besitzer/ACL
  der IPC-Dateien; Geheimnisse erscheinen nie in Debug-Ausgaben; Identitätssperre mit Zeitlimit;
  Uplink-Helfer (Windows) läuft nur als geprüfte Kopie an admin-geschütztem Ort und wird beim Abschalten
  entfernt, Linux-polkit-Regel nur für lokale aktive Sitzung und wird entfernt; Uplink startet nur auf
  authentisierte Tatsachen; LAN-Präsenz nur mit Direkt-Kontakten, ohne Rechnernamen, mit wechselnder ID;
  Firewall-Regel nur UDP und private/Domänen-Netze. [S62, S33, S59, S34, S35, S10, S11, S09, S14, S15]

## Teil B – Bedienung (neu oder geändert)

| Ort | Desktop (egui) | Android (Compose) | CLI (`se`) |
|---|---|---|---|
| Freigabe-Rechte | je Freigabe Umschalter „Nur lesen / Lesen und Schreiben“, Warn-Symbol bei ganzem Benutzerordner + Schreiben | gleicher Umschalter in den Share-Freigaben | `se share exports add PATH [--write]`, `… set NAME --read-only/--write` |
| Gespeicherte Verbindungen teilen | Liste mit Häkchen je Verbindung, Standard nur lesen, Warnhinweis | gleich | `se share exports connections add NAME [--write]` |
| PIN-Kopplung | PIN-Feld mit vorgeschlagener Zufalls-PIN, „Neue PIN“, Häkchen „Unsichere PIN erlauben“; Dauer max 30 min | gleich | `--pin` Pflicht oder `--random-pin` (Standard), `--allow-weak-pin` |
| Server-Adresse | Hinweis `wss://server[:port]`, Häkchen „Unverschlüsselt erlauben (unsicher)“, Status „🔒 verschlüsselt“ / „⚠ unverschlüsselt“ | gleich | `se share server set URL [--allow-plaintext]` |
| Anfragen neuer Geräte | Posteingang mit „Annehmen/Ablehnen“ (bestehend); Einstellung „Anfragen mit meinem Code automatisch annehmen (unsicherer)“ | gleich + Benachrichtigung | `se share requests accept/reject` (bestehend), `se share auto-accept on/off` |
| Gesperrte Kontakte | „Wieder erlauben“ | gleich | `se share contacts allow NAME` |
| Sync-Job-Status | Spalte „Letzter Erfolg / Letzter Versuch / Grund“, „blockiert: Massenlöschung (N)“ mit „Prüfen…/Trotzdem ausführen“, Überwachungsart („Ereignisse“, „Abfrage alle 5 min“) | gleiche Angaben auf der Job-Karte | `se sync status` |
| Fern-Analyse Android | Phasen-Text + Übertragungsbalken, Hinweis-Zeile, geschützte Bereiche | – | – |
| Fern-Duplikate | „In Papierkorb des Geräts“ (wenn Host kann) | gleich | – |

Warte- und Fehlerzustände: jeder neue Stopp (Laufwerk unbekannt, Massenlöschung, Ziel voll, Server
unverschlüsselt, PIN zu schwach) hat einen eigenen Klartext, der sagt, was passiert ist und was man tun
kann. Keine neue Seite; alle Einstellungen sitzen dort, wo die bisherigen derselben Sache sitzen.

## Teil C – Layout und Bedienkosten

Keine neuen Fenster oder Navigationsebenen. Sicherheitsrelevante Abweichungen vom sicheren Standard sind
immer sichtbar (Warn-Symbol direkt an der Freigabe bzw. Server-Adresse), Härten kostet einen Klick.
Sync-Fehlerserien stehen auf der Job-Zeile, nicht nur im Protokoll.

## Kompatibilität (AGENTS.md „preserve existing behavior“)

- Endpunkt-Strings, gespeicherte Orte, Sync-Paare (Backend + Verbindungsidentität), Baselines und
  Versionsordner bleiben gültig; neue Felder sind additiv (`serde(default)`), Baselines werden beim ersten
  Lauf migriert (Identität nachgetragen statt Neuaufbau).
- Pair-IDs ändern sich nicht (Drive/Share-Identitätsänderungen mit einmaliger Übernahme der alten ID).
- Links/Junctions bleiben geschützte Auslassungen; Daten-Reparse-Punkte (OneDrive, WOF, Dedup) sind
  Dateien.
- Gemischte Versionen: neue Share-Fähigkeiten nur nach Aushandlung; alte Server/Clients funktionieren.
- Bestehende Freigaben behalten Schreibrecht; bestehende Klartext-Server-Adressen behalten die Erlaubnis.
- Rückweg zum Original (Versionen, Konfliktkopien, Papierkorb) bleibt für jede Überschreibung/Löschung.

## Plattform-Prüfung (übertragene Lösungen)

- Windows-Ereignisse vs. Netzlaufwerke: RDCW auf SMB ist begrenzt (64 KB, Überlauf häufig) → dort
  Abfrage. inotify auf Android-FUSE meldet Schreiben über MediaStore nicht → zusätzlich MediaStore-
  Beobachter + Kontroll-Scan. inotify-Watch-Limit (8192 auf alten Kerneln/Android 12) → Abfrage-Rückfall.
- Versionen „auf demselben Datenträger“: nur wo Umbenennen billig und sicher ist; FAT ohne Hardlinks,
  Fernziele mit Server-Umbenennung; sonst bisheriger Ort.
- Host-Analyse auf Android läuft im eingebetteten Daemon unter Energiegrenzen → Wakelock während
  Strömen, faire Zuteilung.
- Desktop-Systemwach-Anforderung: Windows Power Request, Linux logind-Inhibitor (`systemd-inhibit`
  als Kindprozess, ohne D-Bus-Abhängigkeit); ohne logind keine Inhibition, Hinweis im Protokoll.

## Nicht in RV1 (aufs Board)

Raum-Neuverschlüsselung (Raumgeheimnis rotieren und verteilen) – Sperren per Schlüssel schließt den
Hauptweg, ein gesperrtes Mitglied mit neuer Identität bleibt aber bis zu einem neuen Raum möglich (S05/S22
teilweise); Geheimnisse im OS-Schlüsselspeicher unter Linux/Android (S58, Linux ohne Secret Service oft
nicht verfügbar); Windows-Identität nach `%LOCALAPPDATA%` (S31, Migrationsrisiko); VSS-Schnappschüsse für
geöffnete Windows-Dateien (Y101, „in Benutzung – übersprungen“ wird gemeldet); Umbenennungs-Erkennung im
Voll-Planer (Y50, Y77; Versionen per Umbenennen mindern die Kosten); Fortsetzen abgebrochener großer
Kopien über Läufe hinweg (Y135); Sparse-/xattr-Erhalt (Y111); EFS-Hinweis (Y107); Selbst-signierte
FTPS/WebDAV-Zertifikate mit Fingerabdruck (Y138); Bandbreitenbegrenzung für Nebenpfade (Y83);
Papierkorb auf UNC (Y109); Windows-Aufgabenplanung statt Run-Key (Y24 Rest); Desktop-Systembenachrichtigungen
aus dem fensterlosen Dienst (Y151: Hinweis erscheint beim Öffnen der App, Android benachrichtigt sofort).
