# Smart Explorer für Android (APK) – Spec

Stand: 2026-09-25 · Auftrag: „eine APK-Version des Smart Explorers erstellen – was geht, was geht
nicht, wie; vollständig selbstständig; Funktionsumfang, der geht, wird übernommen; GUI selbst
gestalten; Festplatten-Mounten nicht nötig; Hintergrund-Worker wie unter Windows wären gut."

Faktenbasis: `docs/lesungen/2026-09-25-android-*.md`, `docs/refs/android-*.md`, `docs/refs/rust-jni.md`.
Haltepunkt entfällt auf ausdrücklichen Wunsch des Nutzers („stoppe nicht für Fragen“); Annahmen
stehen unten offen.

## A Definition

Zweck: Eine installierbare Android-App (APK), die den Rust-Kern des Desktop-Smart-Explorers
wiederverwendet (Dateisystem, Filter, Remote-Backends, Sync, Share/P2P, Analyse) und ihn mit einer
für Touch und kleine Bildschirme entworfenen Oberfläche bedient. Für Nutzer des Desktop-Programms,
die dieselben Verbindungen, Sync-Jobs und Share-Geräte auch auf dem Telefon nutzen wollen.

Oberfläche: GUI (Kotlin + Jetpack Compose, Material 3) über dem Rust-Kern (JNI, JSON-Befehle).

### Funktionen

Grundlage
- F1 App-Grundgerüst: installierbare APK (arm64-v8a, x86_64), Android 11+ (minSdk 30, targetSdk 36),
  Einrichtung beim ersten Start (Zugriff auf alle Dateien, Benachrichtigungen), Hell/Dunkel/System,
  randlose Darstellung, Zurück-Geste → Nutzer sieht eine startbereite App mit Dateiansicht.
- F2 Rust-Kern auf Android: die Desktop-Bibliothek kompiliert für Android; Plattformadapter für
  App-Datenverzeichnisse, Netz-, Energie- und Browser-Anbindung; JNI-Brücke mit Panic-Schutz →
  alle folgenden Funktionen laufen über denselben Kern wie am Desktop.

Dateien
- F3 Orte und Navigation: Speicherorte (interner Speicher, SD-Karte/USB), Favoriten, Zuletzt besucht,
  gespeicherte Verbindungen, Google Drive, Share-Geräte und -Räume, Papierkorb; Pfadleiste,
  Zurück-Verlauf, Tabs, zwei Bereiche nebeneinander auf breiten Bildschirmen → Nutzer erreicht jeden
  Ort mit höchstens zwei Tipps.
- F4 Dateiliste: Name, Größe, Datum, Typsymbol, Bildvorschau (lokal), Sortierung, versteckte Dateien,
  problematische Namen markiert, Zähler, Aktualisieren → übersichtliche, schnelle Liste.
- F5 Filter und Suche: Name (Teiltext/Glob/RegEx), Endungen, Größe, Datum, Dateien/Ordner, versteckte,
  „⚠ nur problematische Namen“; rekursiv mit Filter beim Scannen („Treffer · durchsucht“),
  Scan-Grenze, Abbrechen; rekursive Baumansicht mit Ein-/Ausklappen → gefilterte Treffer über die
  ganze Ordnertiefe.
- F6 Ordnersuche: Fuzzy-Suche über einen Ordnerindex der Speicherorte → Sprung in einen Ordner per
  Namensfragment.
- F7 Auswahl und Dateioperationen: Mehrfachauswahl, Kopieren/Ausschneiden/Einfügen zwischen allen
  Orten, Kopieren nach…/Verschieben nach…, Umbenennen, Neuer Ordner/Neue Datei, Löschen in den
  Papierkorb oder endgültig, Pfad kopieren, Eigenschaften, Favorit → gewohnte Dateiverwaltung.
  Gefilterte rekursive Auswahl kopiert nur passende Dateien mit relativen Pfaden (wie Desktop).
  Ausschneiden/Verschieben wie am Desktop nur zwischen lokalen Orten; Remote-Übertragungen
  nummerieren belegte Zielnamen („Name (2)“).
- F8 Übertragungen: bis zu sechs gleichzeitig, weitere in Warteschlange, Fortschritt je Vorgang,
  einzeln/alle abbrechen, Benachrichtigung, läuft beim Verlassen der App weiter → verlässliche
  große Kopien.
- F9 Öffnen, Teilen, Empfangen: Datei in passender App öffnen (remote: vorher laden; geänderte
  Remote-Datei zurückladen mit Konfliktprüfung), an andere Apps teilen, aus anderen Apps geteilte
  Dateien in einen beliebigen Ort (auch remote) speichern.
- F10 ZIP: ZIP-Archiv wie einen Ordner durchsuchen (nur lesen), entpacken hierher oder nach…
- F11 Papierkorb: Löschen verschiebt je Speichervolume in einen App-Papierkorb; Liste,
  Wiederherstellen, endgültig löschen, leeren; Einträge älter als 30 Tage werden automatisch entfernt.
  Remote-Orte mit eigenem Papierkorb (z. B. Google Drive) nutzen diesen; übrige Remote-Orte löschen
  nach Rückfrage endgültig (wie Desktop).

Verbindungen
- F12 Verbindungen verwalten: SFTP (Passwort oder Schlüsseldatei, optional Remote-Agent wie am Desktop), FTP/FTPS,
  WebDAV (HTTPS, wie am Desktop) anlegen, bearbeiten, testen, löschen; Zugangsdaten im geschützten App-Speicher;
  Hostschlüssel nach „Trust on first use“ wie am Desktop; beim Löschen Aufräumen wie am Desktop
  (Favoriten und Tabs werden entfernt, betroffene Sync-Jobs gemeldet).
- F13 Google Drive: eigene OAuth-Client-ID (wie Desktop, `docs/CLOUD_SETUP.md`), Anmeldung im
  Browser, Durchsuchen und Synchronisieren.

Sync und Hintergrund
- F14 Spiegeln: aktuellen Ordner einmalig einseitig in ein Ziel kopieren (lokal oder remote), wie
  „Spiegeln nach…“ am Desktop: Neues und Geändertes wird übertragen, im Ziel wird nichts gelöscht.
- F15 Sync-Jobs: Liste mit Status; anlegen/bearbeiten mit Quelle und Ziel aus allen Orten, Richtung,
  Konflikt- und Löschregel, Versionen, Ignoriermuster, Auslöser (manuell, Intervall, Zeitplan,
  Echtzeit, beim Start); jetzt ausführen, aktivieren/pausieren, löschen; letzter Lauf und Ergebnis.
  Jobs sind dieselben Dateien und dasselbe Format wie am Desktop.
- F16 Konflikte: Konflikte eines Zwei-Wege-Jobs auflisten (aus dem letzten Lauf oder per Probelauf
  „Konflikte prüfen“) und lösen (A behalten, B behalten, überspringen); Textdateien zeilenweise
  zusammenführen oder beide behalten (wie Desktop).
- F17 Hintergrund-Worker: derselbe Desktop-Daemon (Scheduler, Jobs, Share-Host) läuft als Thread im
  App-Prozess, solange der Prozess lebt. Modus Aus (keine geplanten Jobs) / Periodisch (WorkManager
  weckt die App ≥ alle 15 min unter Bedingungen – nur WLAN, nur beim Laden, Akku nicht niedrig – und
  lässt den Daemon nachholen: fällige Intervall-Jobs, Kalender-Termine seit dem letzten Lauf, jeden
  Echtzeit-Job einmal) / Dauerbetrieb (Vordergrunddienst hält den Prozess wach: Echtzeit-Jobs und
  Share wie am Desktop); Pause (1 h, bis morgen, unbegrenzt), automatische Pause bei Energiesparmodus
  oder getaktetem Netz (auf Android echt ausgewertet); „Beim Start“-Jobs einmal je Gerätestart;
  Start nach Neustart; Ausnahme von der Akku-Optimierung; Worker-Protokoll.

Share
- F18 Share/P2P: eigenes Gerät (Name, Status, Share-Server), zeitlich begrenzt suchbar machen mit PIN,
  Geräte und Räume finden und mit PIN verbinden, Direkt-Geräte durchsuchen und entfernen, Räume
  erstellen/beitreten per Code/verlassen, Anfragen annehmen/ablehnen, lokale Ordner freigeben,
  Dateien an ein Gerät senden (Kopieren in dessen Freigabe), einen Befehl auf einem freigegebenen
  Gerät ausführen (Ausgabe anzeigen), LAN-Präsenz im WLAN.

Analyse und mehr
- F19 Speicheranalyse: Ort wählen, Scan mit Fortschritt und Abbrechen, Treemap und Liste der größten
  Einträge, Drilldown, Bericht über Leseprobleme; lokal, remote und Share.
- F20 Duplikate finden: Scan, Gruppen mit gleichen Inhalten, Kopien auswählen, in den Papierkorb.
- F21 Einstellungen: Darstellung, Dateiliste, Hintergrund (F17), Share-Server, Google-Drive-Client-ID,
  Speicherzugriff, Updates, Über/Version.
- F22 Updates: neue Version über den Update-Feed erkennen, APK laden, SHA-256 prüfen, mit dem
  System-Installer installieren.
- F23 Fehlerprotokoll: App-Fehler sammeln, anzeigen, kopieren/teilen; Absturzprotokoll des Kerns.

### Nicht-Ziele (was nicht geht, und warum)

- Remote/Share als Laufwerk einbinden (Dokany) – vom Nutzer ausgeschlossen; Android kennt keine
  Drittanbieter-Dateisysteme ohne Root.
- Explorer-Kontextmenü, Windows-Zwischenablage mit virtuellen Dateien, Drag-and-drop nach außen –
  Windows-Shell-Konzepte; ersetzt durch „Öffnen mit“, „Teilen“ und „Empfangen“ (F9).
- Netzlaufwerke (UNC/SMB) – der Kern hat keinen eigenen SMB-Client (auch Linux-Desktop nicht).
- Internet-Teilen (LAN-Uplink via ICS/NetworkManager) – Android bietet dafür den System-Hotspot.
- Befehle von anderen Geräten auf dem Telefon ausführen (Exec-Host) – die App-Sandbox bietet keine
  Prozess-Container wie Job-Objekte/systemd; das Telefon kann Befehle nur auf anderen Geräten
  ausführen (F18).
- Quick-Share-Interop – Android bringt Quick Share selbst mit.
- Share-Anfragen im Altformat (Geräte mit Versionen vor der aktuellen Anfrage-Protokollversion) –
  werden auf dem Telefon nicht angezeigt; neue Kopplungen nutzen das aktuelle Protokoll.
- Versions-Rollback – Android installiert keine ältere Version über eine neuere.
- Echtzeit-Jobs ohne Dauerbetrieb – Android friert Hintergrundprozesse ein; Echtzeit-Jobs laufen im
  Dauerbetrieb oder während die App offen ist, im Modus „Periodisch“ einmal je Worker-Lauf.
- Kalender-Jobs auf die Minute – im Modus „Periodisch“ holt der Worker Termine nach (Abstand ≥ 15 min,
  von Android verschoben); pünktlich nur im Dauerbetrieb.
- Share-Erreichbarkeit ohne Dauerbetrieb – andere Geräte erreichen das Telefon nur, solange die App
  offen ist, eine Übertragung läuft oder der Dauerbetrieb aktiv ist.
- Google-Drive-Anmeldung mit der Desktop-Client-ID – technisch derselbe Loopback-Ablauf; ob Google
  ihn aus dem Android-Browser für jedes Konto annimmt, ist ohne echtes Konto nicht geprüft (offener
  Punkt, in README und „Über“ genannt).
- Auslöser „Bei Geräte-/USB-Anschluss“ – wie unter Linux nicht unterstützt; im Editor ausgeblendet.
- Ordner-Live-Beobachtung für den Ordnerindex – wie unter Linux; Index wird bei Bedarf neu gebaut.
- System-Dateisymbole – Android liefert keine; Typsymbole plus Bildvorschau.
- Google-Play-Veröffentlichung – Verteilung über GitHub-Release und Update-Feed (Sideload).
- Zugriff über Storage Access Framework (`content://`-Bäume) als Browse-Ort – der Kern arbeitet mit
  Dateipfaden; „Zugriff auf alle Dateien“ deckt gemeinsamen Speicher, SD-Karte und USB ab.

### Annahmen und eigene Entscheidungen

- Android 11+ (API 30) als Minimum: ~87 % Geräteabdeckung, einheitlicher Speicherzugriff über
  `MANAGE_EXTERNAL_STORAGE`, `StorageVolume.getDirectory()` verfügbar.
- Sideload-Verteilung: „Zugriff auf alle Dateien“, Akku-Ausnahme und Dauerbetrieb-Dienst (Typ
  `specialUse`) sind für eine Sideload-App zulässig; Play-Richtlinien gelten nicht.
- Stabiler Release-Signaturschlüssel wird einmalig erzeugt und als GitHub-Secret hinterlegt; ohne
  ihn wären Updates über eine installierte Version unmöglich. Sicherungskopie außerhalb des Repos.
- Kein eigener Android-Client-Typ für Google OAuth: die App nutzt wie der Desktop die vom Nutzer
  hinterlegte Desktop-Client-ID mit Loopback-Weiterleitung im Systembrowser. Scheitert das bei
  Google für ein Konto, bleibt Drive am Telefon nicht nutzbar; alle anderen Funktionen sind davon
  unabhängig.
- Die App ist ein eigenständiges Gerät mit eigenen Daten (Verbindungen, Jobs, Share-Identität) im
  App-Speicher, in denselben Formaten wie der Desktop; es gibt keine automatische Übernahme von
  Desktop-Profilen. Diese Daten sind von Android-Sicherung und Geräteumzug ausgeschlossen
  (Zugangsdaten und private Share-Identität verlassen das Gerät nicht).
- Standardfreigabe für gekoppelte Direkt-Geräte ist wie am Desktop das „Home“-Verzeichnis, auf
  Android der interne Speicher; sie steht sichtbar unter „Freigaben“ und lässt sich entfernen.
- Oberflächentexte Deutsch (wie Desktop).
- Der Hintergrund-Default ist „Periodisch, 60 min, nur WLAN“; Dauerbetrieb nur auf Wunsch
  (dauerhafte Benachrichtigung, Akku).

## B Bedienung

Dauerklassen: < 0,1 s nichts · < 1 s sofort quittieren · 1–10 s Indikator, bedienbar · > 10 s
Fortschritt + Abbrechen, im Hintergrund mit Benachrichtigung.

### F1 App-Grundgerüst
- Einstieg: App-Symbol; Freigabe-Aktion „Smart Explorer“ (F9); Benachrichtigung antippen.
- Ablauf: erster Start → Seite „Einrichtung“: Karte „Zugriff auf alle Dateien“ [Erlauben] (öffnet
  System-Einstellung), Karte „Benachrichtigungen“ [Erlauben] (Laufzeitdialog), [Weiter]. Fehlt der
  Dateizugriff später, erscheint in der Dateiansicht ein Hinweisbanner mit [Erlauben].
- Anzeige: danach Hauptansicht „Dateien“ im internen Speicher.
- Warten: Kernstart < 1 s (Ladeindikator im Splash, falls länger).
- Fehler: Kern lässt sich nicht starten → Fehlerseite mit Meldung, [Protokoll teilen], [Erneut].
- Ausstieg: Zurück auf oberster Ebene schließt die App; laufende Übertragungen laufen im Dienst weiter.
- Zustände: ohne Dateizugriff zeigt „Dateien“ nur App-Ordner und das Banner.

### F2 Rust-Kern
- Kein eigener Bedienweg; sichtbar über F23 (Absturzprotokoll) und „Über“ (Kernversion).

### F3 Orte und Navigation
- Einstieg: Menü-Taste (☰) oben links öffnet die Seitenleiste „Orte“; Pfadleiste unter der
  Titelzeile; System-Zurück; Tab-Taste (Zahl) oben rechts.
- Seitenleiste (von oben): Speicher (Intern, SD-Karte, USB …) · Favoriten · Zuletzt (max. 10) ·
  Verbindungen (+ „Verbindung hinzufügen“) · Google Drive · Share (Geräte, Räume) · Papierkorb.
  Langes Drücken auf einen Favoriten → „Entfernen“.
- Pfadleiste: waagerecht scrollbar, jedes Segment antippbar; letztes Segment fett.
- Zurück: geht im Verlauf des Tabs zurück; am Verlaufsanfang ins übergeordnete Verzeichnis; im Root
  schließt es die App (nur auf oberster Ebene). Vorwärts über das Menü „⋮ → Vor“.
- Tabs: Tab-Taste zeigt Liste der Tabs (Ort, Pfad) mit ✕ je Tab und „+ Neuer Tab“; Antippen wechselt.
  Jeder Tab hat eigenen Ort, Verlauf, Filter, Rekursiv- und Klappzustand.
- Zwei Bereiche: ab 840 dp Breite zeigt „Dateien“ zwei Bereiche nebeneinander (je eigener Tab);
  der aktive Bereich ist umrandet; „Kopieren nach…“ schlägt den anderen Bereich als Ziel vor.
- Warten: Remote-Ort öffnen 1–10 s → Ladebalken unter der Titelzeile, Liste bleibt bedienbar.
- Fehler: Verbindung scheitert → Fehlerkarte im Listenbereich mit Meldung, [Erneut], [Bearbeiten].

### F4 Dateiliste
- Zeile: Symbol/Vorschau · Name (problematische Namen in Warnfarbe mit ⚠) · Untertitel „Größe ·
  Datum“ bzw. „Ordner“ · rechts ⋮ (Einzelmenü). Antippen öffnet (Ordner/ZIP → hinein, Datei → F9).
- Menü „⋮“ der Titelzeile → „Ansicht“: Sortieren nach Name/Größe/Datum/Typ, auf-/absteigend, Ordner
  zuerst (ein/aus), versteckte Dateien zeigen, kompakte Zeilen.
- Aktualisieren: nach unten ziehen.
- Fußzeile: „N Elemente · Größe“ bzw. im Filtermodus „Treffer · durchsucht“.
- Leer: „Dieser Ordner ist leer“ mit [Neuer Ordner].

### F5 Filter und Suche
- Einstieg: Lupe in der Titelzeile blendet die Filterzeile ein (Suchfeld mit Fokus, Umschalter
  „Rekursiv“, Taste „Filter“).
- Eingaben: Suchfeld (Teiltext, mehrere Begriffe mit `;`); „Filter“ öffnet ein unteres Blatt mit:
  Modus Teiltext/Glob/RegEx · Endungen (`jpg; *.heic; tar.gz`) · Größe von/bis (KB/MB/GB) ·
  geändert von/bis (Datumsauswahl) · Dateien ✓/Ordner ✓ · versteckte · „⚠ nur problematische Namen“ ·
  [Zurücksetzen] [Fertig]. Ungültiges Glob/RegEx → Fehlertext unter dem Feld, Filter bleibt aus.
- Anzeige: aktive Kriterien als Chips unter der Suche (✕ entfernt einzeln).
- Rekursiv: Scan ab aktuellem Ordner; Zähler „Treffer · durchsucht“; Pfeile vor Ordnern klappen ein/aus.
- Warten: > 1 s Fortschritt „n durchsucht“ + [Stopp]; Grenze erreicht → Hinweis „Scan-Grenze
  erreicht – Ergebnisse unvollständig“; Lesefehler → Hinweis „n Ordner nicht lesbar“ [Details].
- Ausstieg: ✕ im Suchfeld löscht Name; Lupe erneut schließt die Filterzeile und hebt Filter auf;
  anderer Ordner öffnen löscht den Namensfilter (wie Desktop).

### F6 Ordnersuche
- Einstieg: „⋮ → Ordner suchen“ und Seitenleiste oben „Ordner suchen“.
- Ablauf: Suchseite mit Feld; Treffer (Ordnername, Pfad) erscheinen beim Tippen; Antippen öffnet den
  Ordner im aktuellen Tab.
- Warten: erster Aufbau des Index > 10 s → Fortschritt „n Ordner indiziert“, Suche nutzt bereits
  gebaute Teile; [Index neu aufbauen] im Menü der Suchseite.
- Leer: „Kein Ordner gefunden“.

### F7 Auswahl und Dateioperationen
- Einstieg: langes Drücken auf eine Zeile → Auswahlmodus; weitere Zeilen antippen; oder „⋮“ je Zeile.
- Auswahl-Titelzeile: ✕ · „n ausgewählt“ · Kopieren · Ausschneiden · Löschen · Teilen · ⋮
  (Alle auswählen, Auswahl umkehren, Umbenennen [genau 1], Kopieren nach…, Verschieben nach…,
  Entpacken [ZIP], Pfad kopieren, Eigenschaften, Zu Favoriten).
- Einfügen: nach Kopieren/Ausschneiden erscheint unten die Leiste „n Elemente – [Hier einfügen]
  [✕]“, auch nach Ortswechsel. Namenskonflikt lokal→lokal → Dialog: Überspringen / Ersetzen / Beide
  behalten (gilt für alle); bei Remote-Zielen Hinweis „Vorhandene Namen werden nummeriert“.
  „Ausschneiden“ ist in Remote-Orten ausgegraut; Einfügen eines Ausschnitts in einen Remote-Ort
  meldet „Verschieben zu Remote wird nicht unterstützt – bitte kopieren“.
- Kopieren nach… / Verschieben nach…: Zielauswahl (vollbild) mit Orten, Pfadleiste, Ordnerliste,
  [Neuer Ordner], [Hierhin].
- Umbenennen: Dialog mit Feld (Name vorausgewählt ohne Endung); ungültiger Name → Fehlertext, OK
  gesperrt; vorhandener Name → „Existiert bereits“.
- Neu: Schwebetaste (+) → „Neuer Ordner“ / „Neue Datei“; Dialog mit Vorschlag „Neuer Ordner“.
- Löschen: Dialog „n Elemente in den Papierkorb?“ [Löschen]; Häkchen „Endgültig löschen“; Orte ohne
  Papierkorb (remote) zeigen nur „Endgültig löschen“. Große Löschungen → Fortschritt mit [Stopp].
- Eigenschaften: Blatt mit Name, Ort, Typ, Größe (Ordner: rekursiv, mit Fortschritt), Anzahl,
  geändert, erstellt; [Pfad kopieren].
- Quittung: Snackbar „Kopiert: 3 Elemente“ bzw. Fehler mit [Details].

### F8 Übertragungen
- Einstieg: startet durch Einfügen/Kopieren nach/Verschieben nach/Senden/Empfangen/Entpacken.
- Anzeige: Leiste unten in „Dateien“: „2 Übertragungen · 45 % [Anzeigen]“; Blatt „Übertragungen“:
  je Vorgang Titel (Quelle → Ziel), Balken, „12 von 40 Dateien · 1,2 GB · 8 MB/s“, [✕]; oben
  [Alle abbrechen]; Abschnitt „Fertig“ mit Ergebnis/Fehlern ([Details], [Leeren]).
- Hintergrund: solange Übertragungen laufen, hält ein Vordergrunddienst die App am Leben;
  Benachrichtigung „Smart Explorer überträgt – 45 %“ mit [Abbrechen]. Android 15: nach 6 h im
  Hintergrund bricht das System den Dienst ab → laufende Vorgänge werden sauber abgebrochen und als
  „vom System beendet“ gemeldet.
- Ausstieg: Blatt nach unten wischen; Einzel- oder Gesamtabbruch.

### F9 Öffnen, Teilen, Empfangen
- Öffnen lokal: Antippen einer Datei → Android-App-Auswahl für den Typ (Freigabe über den eigenen
  `LocalFileProvider`; Änderungen der Fremd-App landen direkt im Original).
  Keine App → Snackbar „Keine App für diesen Dateityp“.
- Öffnen remote: Antippen → Fortschritt „Wird geladen…“ (> 1 s mit [Abbrechen]) → App-Auswahl mit
  Schreibrecht. Kehrt der Nutzer zurück und die lokale Kopie wurde geändert (auch nach einem
  Neustart der App – geöffnete Kopien sind dauerhaft registriert): Karte „Geänderte Datei:
  name – [Hochladen] [Verwerfen]“. Remote seit dem Öffnen geändert → Dialog „Konflikt: Remote
  überschreiben / Als Kopie hochladen / Abbrechen“.
- Teilen: Auswahl → „Teilen“ → Android-Teilen-Dialog (lokal direkt; remote vorher laden).
- Empfangen: in anderer App „Teilen → Smart Explorer“ → Zielauswahl (wie „Kopieren nach…“) →
  Übertragung direkt ins Ziel mit Fortschritt (F8, auch remote) → Snackbar „Gespeichert“.

### F10 ZIP
- Antippen einer .zip → Inhalt wie Ordner, Titel „name.zip (nur lesen)“, Schreibaktionen ausgegraut.
- „Entpacken“ (Auswahl-Menü) → „Hierher“ (Ordner neben dem Archiv) oder „Nach…“ (Zielauswahl) →
  Übertragung mit Fortschritt.
- Zurück verlässt das Archiv.

### F11 Papierkorb
- Einstieg: Seitenleiste „Papierkorb“.
- Anzeige: Liste (Name, ursprünglicher Ort, gelöscht am, Größe); Auswahl → [Wiederherstellen]
  [Endgültig löschen]; Titel-Menü [Papierkorb leeren].
- Wiederherstellen: zurück an den ursprünglichen Ort; belegt → „Beide behalten“ (Suffix) nach Dialog.
- Automatik: beim App-Start und im Worker werden Einträge > 30 Tage entfernt.

### F12 Verbindungen
- Einstieg: Seitenleiste „Verbindung hinzufügen“; Mehr → Verbindungen (Liste mit ⋮: Bearbeiten,
  Testen, Löschen).
- Formular: Typ (SFTP, FTP, FTPS, WebDAV) · Name · Host · Port (Vorgabe je Typ) · Benutzer ·
  Passwort oder Schlüsseldatei (Datei aus Speicher wählen) + Passphrase (SFTP) · Startordner · Remote-Agent (SFTP) ·
  [Testen] [Speichern]; WebDAV immer über HTTPS (Desktop-Format).
- Warten: Testen/Verbinden 1–10 s → Indikator; Ergebnis „Verbindung OK“ oder Fehlertext.
- Hostschlüssel: wie am Desktop „Trust on first use“ (erster Schlüssel wird gespeichert). Geänderter
  Schlüssel → Verbindung scheitert mit „Hostschlüssel geändert“; ist die Änderung erwartet, entfernt
  ⋮ → „Hostschlüssel vergessen“ (Bestätigung mit Warnung) den gespeicherten Eintrag.
- Löschen: Dialog nennt, was mit entfernt wird (Favoriten, Jobs) [Löschen].

### F13 Google Drive
- Einstieg: Einstellungen → Google Drive: Feld Client-ID (+ optional Secret), [Anmelden];
  Seitenleiste „Google Drive“ (ohne Anmeldung → Hinweis mit [Einrichten]).
- Ablauf: [Anmelden] → Browser mit Google-Zustimmung → Rückkehr, Status „Angemeldet“.
- Warten: bis zu 3 min auf die Rückmeldung (der Task-Dienst hält die App wach) mit [Abbrechen].
- Fehler: Zeitüberschreitung/Ablehnung → Meldung mit Link auf die Einrichtungsanleitung.

### F14 Spiegeln
- Einstieg: Dateien → ⋮ → „Spiegeln nach…“ → Zielauswahl (alle Orte) → Bestätigung „Neues und
  Geändertes wird nach <Ziel> kopiert; im Ziel wird nichts gelöscht“ → Lauf mit Fortschritt, Ergebnis
  „n kopiert, n übersprungen, n Fehler“; [Als Sync-Job speichern] öffnet den Editor vorbefüllt.

### F15 Sync-Jobs
- Einstieg: untere Leiste „Sync“.
- Liste: Karte je Job: Name, „A ⇄ B“ bzw. „A → B“ mit Ortssymbolen, Auslöser („alle 60 min“),
  letzter Lauf (✓ Zeit / ⚠ n Konflikte / ✕ Fehler), laufend: Balken; Knopf ▶ „Jetzt“; ⋮ (Bearbeiten,
  Aktivieren/Pausieren, Konflikte, Löschen).
- Kopf: Hintergrundstatus („Periodisch · nächster Lauf ~14:30“, „Pausiert bis 18:00“) → tippt in F17.
- Neu: Schwebetaste (+) → Editor (vollbild): Name · Seite A [Wählen] · Seite B [Wählen] · Richtung
  (A→B spiegeln, B→A spiegeln, zwei Wege) · Auslöser (Manuell, Intervall [Minuten], Zeitplan
  [täglich/wöchentlich/monatlich + Uhrzeit], Echtzeit [Wartezeit], Beim Start) · aufklappbar
  „Erweitert“: Konfliktregel, Löschregel, Versionen behalten (Tage), versteckte einschließen,
  Ignoriermuster (eine pro Zeile), Aktivzeit von/bis, verpasste Termine nachholen, Löschschutz
  (Anzahl/Prozent), Papierkorb für lokale Löschungen, Befehl vorher/nachher (nur Hintergrundläufe,
  wie Desktop) · [Speichern]. Prüfung wie Desktop
  (gleiche/verschachtelte Seiten, ungültige Muster) → Fehler am Feld.
- Jetzt: startet sofort (Vordergrunddienst), Karte zeigt Fortschritt, Ergebnis in der Karte.
- Leer: „Noch keine Sync-Jobs“ [Job anlegen] [Ordner spiegeln].

### F16 Konflikte
- Einstieg: Karte „⚠ n Konflikte“ oder ⋮ → Konflikte.
- Liste: Datei, Zeiten/Größen A und B (fehlend = gelöscht); je Eintrag: [A behalten] [B behalten]
  [Überspringen] und bei Text [Zusammenführen]; oben [Alle A] [Alle B]. Ohne gespeicherte Liste
  (App neu gestartet, Hintergrundlauf): Hinweis „Konfliktliste nicht geladen“ [Konflikte prüfen]
  (Probelauf mit Fortschritt).
- Zusammenführen: Zeilenansicht (gleich / nur A / nur B / geändert), je geänderter Zeile A, B oder
  beide wählen, [Alle A] [Alle B], [Übernehmen] (schreibt auf beide Seiten) oder [Beide behalten]
  (B als „(Konflikt …)“-Kopie). Warten > 1 s → Indikator.
- Schließen: ungespeicherte Auflösungen werden gesichert (Baseline); scheitert das → Meldung mit
  [Erneut versuchen].
- Ergebnis: gelöste Einträge verschwinden; Fehler als Snackbar mit [Details].

### F17 Hintergrund-Worker
- Einstieg: Mehr → Einstellungen → Hintergrund; Kopf der Sync-Seite.
- Eingaben: Modus (Aus / Periodisch / Dauerbetrieb) · Intervall (15, 30, 60, 180, 360 min) · nur WLAN
  (ungetaktet) · nur beim Laden · nicht bei niedrigem Akku · automatische Pause bei
  Energiesparmodus · bei getaktetem Netz · Pause [1 h] [bis morgen] [unbegrenzt] / [Fortsetzen] ·
  [Akku-Optimierung ausschalten] (Systemdialog) · [Worker-Protokoll].
- Anzeige: Status (läuft/pausiert/nächster Lauf), letzter Lauf.
- Dauerbetrieb: dauerhafte Benachrichtigung „Smart Explorer im Hintergrund – n Jobs, Share online“
  mit [Pausieren]; startet nach Neustart wieder.
- Fehler: Benachrichtigungen verboten → Hinweis mit [Erlauben].

### F18 Share/P2P
- Einstieg: untere Leiste „Teilen“.
- Seite (von oben): Karte „Dieses Gerät“ (Name ✎, Status Online/Offline/Relay, [Suchbar machen]);
  Abschnitt „Geräte“ (Direkt-Geräte: Name, Status; Antippen → Dateien des Geräts; ⋮: Datei senden,
  Befehl ausführen, Entfernen); Abschnitt „Räume“ (Name, Mitglieder; Antippen → Raum; ⋮: Code
  anzeigen, Verlassen); Knöpfe [Gerät verbinden] [Raum erstellen] [Raum beitreten];
  Abschnitt „Anfragen“ (nur wenn vorhanden: [Annehmen] [Ablehnen] [Erneut senden] [Löschen]);
  Abschnitt „Freigaben“ (lokale Ordner, die dieses Gerät anbietet, für Direkt-Geräte und je Raum:
  [+ Ordner freigeben], ⋮ Entfernen); Abschnitt „Entfernte Geräte“ (nur wenn vorhanden:
  [Wieder zulassen]); [Direct-Code hinzufügen] als Alternative zur PIN-Suche.
- Suchbar machen: Dialog Dauer (5 min vorgegeben) + PIN (Hinweis bei trivialer PIN) → Countdown.
- Gerät verbinden: Liste gefundener Geräte/Räume (Name, Ablauf) → Antippen → PIN → „Verbinde…“
  (1–10 s) → Ergebnis.
- Raum beitreten: Code eingeben → Ergebnis.
- Datei senden: Auswahl in „Dateien“ → „Kopieren nach…“ mit Gerät als Ziel (Übertragung F8).
- Befehl ausführen: Dialog Befehl → Ausgabe (stdout/stderr, Exitcode), [Abbrechen]; fehlende
  Freigabe → Meldung „Gerät erlaubt keine Befehle von diesem Telefon“.
- Share-Server: Einstellungen → Share-Server (Adresse).
- Offline: Karte zeigt „Offline – [Erneut verbinden]“; ohne Server „nur LAN“.

### F19 Speicheranalyse
- Einstieg: Mehr → Speicheranalyse (aktueller Ort vorgewählt) oder Dateien → ⋮ → „Analysieren“.
- Ablauf: Scan mit Fortschritt (Dateien, Ordner, Bytes) und [Abbrechen] → Ergebnis: Treemap oben
  (Rechtecke nach Größe, Farbe nach Typ), darunter Liste der größten Kinder mit Balken; Antippen →
  hinein; Zurück → hinauf; ⋮ „In Dateien öffnen“.
- Leseprobleme: Hinweis „n Pfade nicht lesbar“ [Bericht] (kopierbar).

### F20 Duplikate
- Einstieg: Mehr → Duplikate finden → Ort wählen, Mindestgröße → Scan (Fortschritt, [Abbrechen]) →
  Gruppen (Größe, Anzahl); [Kopien automatisch auswählen] (behält jeweils die älteste) →
  [In den Papierkorb]. Remote: nur Anzeige, wenn der Ort keinen Papierkorb hat.

### F21 Einstellungen
- Abschnitte: Darstellung (Hell/Dunkel/System) · Dateiliste (versteckte, Ordner zuerst, kompakt,
  Bildvorschau) · Hintergrund (F17) · Share-Server · Google Drive (F13) · Speicherzugriff (Status +
  [Erlauben]) · Updates (F22) · Fehlerprotokoll (F23) · Über (App-/Kernversion, Lizenz, Hinweis).

### F22 Updates
- Einstieg: Einstellungen → Updates [Jetzt prüfen]; automatische Prüfung beim Start (höchstens 1×/Tag,
  abschaltbar).
- Ablauf: neue Version → Karte „Version x.y.z verfügbar [Installieren]“ → Download mit Fortschritt →
  SHA-256-Prüfung → System-Installer. Fehlende Erlaubnis „Unbekannte Apps installieren“ → Hinweis mit
  [Erlauben].
- Fehler: Prüfsumme falsch → „Download beschädigt – nicht installiert“.

### F23 Fehlerprotokoll
- Einstieg: Einstellungen → Fehlerprotokoll; Snackbar-[Details].
- Anzeige: Liste (Zeit, Aktion, Meldung); [Kopieren] [Teilen] [Leeren]; Absturzprotokoll des Kerns
  als eigener Eintrag.

### Vollständige Elementliste (muss existieren)
Aktivität `MainActivity` (Einrichtung, Dateien, Sync, Teilen, Mehr + Unterseiten), Seitenleiste
„Orte“, Tab-Umschalter, Filterzeile + Filterblatt, Ordnersuche, Zielauswahl, Übertragungen-Blatt,
Einfüge-Leiste, Dialoge (Umbenennen, Neu, Löschen, Konflikt, Eigenschaften, Hostschlüssel,
Suchbar machen, PIN, Raum-Code, Befehl), Sync-Editor, Konfliktliste, Zusammenführen-Ansicht,
Verbindungsformular, Speicheranalyse, Duplikate, Papierkorb, Einstellungen, Fehlerprotokoll,
Update-Karte; Dienste `TaskForegroundService` (dataSync: Übertragungen, „Jetzt“-Läufe,
Google-Anmeldung), `BackgroundService` (specialUse), `SyncWorker` (WorkManager), `BootReceiver`,
`LocalFileProvider` (lokale Dateien, Schreiben aufs Original), FileProvider (nur Cache/Remote-Kopien),
Teilen-Empfänger (SEND/SEND_MULTIPLE); Benachrichtigungskanäle „Übertragungen“, „Hintergrund“,
„Updates“.

## C Layout und Bedienkosten

Häufigkeit: ständig = Ordner öffnen, zurück, scrollen, Datei öffnen, auswählen, kopieren/einfügen,
filtern · regelmäßig = Ort wechseln, löschen, umbenennen, neuer Ordner, Übertragungen ansehen,
Job jetzt ausführen, Gerät öffnen · selten = Verbindungen anlegen, Jobs bearbeiten, Konflikte,
Einstellungen, Analyse, Duplikate, Updates, Papierkorb.

Hauptansicht „Dateien“ (Telefon, hochkant):
```
┌─────────────────────────────────────────┐
│ ☰  Interner Speicher        🔍  [2]  ⋮  │  Titel = Ort; 🔍 Filterzeile; [2] Tabs; ⋮ Menü
│ › Intern › DCIM › Camera                │  Pfadleiste (scrollbar, Segmente antippbar)
│ [ name…            ✕ ] (Rekursiv) [⚙]   │  nur bei 🔍: Suchfeld, Umschalter, Filterblatt
│ ( Bilder ✕ ) ( > 10 MB ✕ )              │  aktive Filter-Chips
├─────────────────────────────────────────┤
│ ▸ 📁 Camera                    Ordner ⋮ │  Ordner (Pfeil nur rekursiv)
│   🖼 IMG_0001.jpg     3,2 MB · 12.09. ⋮ │  Datei mit Vorschau
│ ⚠  nul.txt              0 B · 01.01. ⋮ │  problematischer Name
│  …                                      │
│ 124 Elemente · 1,8 GB                   │  Fußzeile
├─────────────────────────────────────────┤
│ ⇅ 2 Übertragungen · 45 %      [Anzeigen]│  nur bei laufenden Übertragungen
│ 3 Elemente kopiert  [Hier einfügen] [✕] │  nur mit gefüllter Zwischenablage
│                                    (+)  │  Neu (Ordner/Datei)
├─────────────────────────────────────────┤
│  📁 Dateien   ⇄ Sync   ⤴ Teilen   ⋯ Mehr │  untere Navigation
└─────────────────────────────────────────┘
```
Auswahlmodus ersetzt die Titelzeile: `✕  3 ausgewählt   ⧉  ✂  🗑  ⤴  ⋮`.

Menü „⋮“ Dateien (oberste Ebene, gruppiert): Ansicht… · Ordner suchen · ── · Neuer Tab ·
Vor · ── · Spiegeln nach… · Analysieren · ── · Zu Favoriten · Pfad kopieren · Eigenschaften.
(9 Einträge, ein Untermenü „Ansicht“ nur als Blatt für die zusammengehörigen Sortier-/Anzeigeoptionen.)

Zeilenmenü „⋮“: Öffnen mit · Teilen · ── · Kopieren · Ausschneiden · Kopieren nach… ·
Verschieben nach… · ── · Umbenennen · Löschen · Eigenschaften (+ Entpacken bei ZIP).

Breite Bildschirme (≥ 840 dp): Navigationsschiene links statt unterer Leiste, Seitenleiste „Orte“
dauerhaft sichtbar, zwei Dateibereiche nebeneinander.

Seite „Sync“: Kopfkarte Hintergrundstatus → Jobkarten (▶ direkt auf der Karte, weil „Jetzt“
regelmäßig ist) → (+). Seite „Teilen“: Karte dieses Gerät → Geräte → Räume → Anfragen (nur wenn
vorhanden) → Freigaben. Seite „Mehr“: Liste Speicheranalyse · Duplikate finden · Verbindungen ·
Papierkorb · Übertragungen · Einstellungen · Fehlerprotokoll · Über.

Kosten-Abwägung:
- Ordner öffnen, zurück, Datei öffnen: 1 Tipp/Geste, kein Dialog.
- Kopieren → Einfügen: 3 Tipps (lange drücken, ⧉, „Hier einfügen“) + Ortswechsel; „Kopieren nach…“
  als Alternative ohne Ortswechsel im aktuellen Tab.
- Filter: 🔍 + Tippen; weitere Kriterien 1 Blatt tief; aktive Kriterien immer sichtbar als Chips.
- Ort wechseln: ☰ + 1 Tipp; auf breiten Bildschirmen 1 Tipp.
- Job jetzt ausführen: 1 Tipp auf der Karte.
- Selten Genutztes liegt in „Mehr“ oder in „⋮“, nie auf der obersten Ebene.
- Texte: kurze Beschriftungen; Erklärungen nur in Hinweiszeilen unter Feldern und in Fehlerkarten;
  Fehlermeldungen nennen Ursache und nächsten Schritt.
- Rückmeldung: jede Aktion endet in Snackbar, Fortschritt oder sichtbarer Listenänderung.

Hilfe: jede Seite mit Fehlern/leeren Zuständen erklärt dort den nächsten Schritt; „Über“ verlinkt
README/CLOUD_SETUP auf GitHub.
