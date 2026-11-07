# Nutzerentscheidungen 2026-11-07 (E11 bis E20)

Stand: 2026-11-07. Die Entscheidungen stammen aus den Nachrichten des Nutzers vom 2026-11-07
zur Sicherheits- und Direktweg-Prüfung. Die Zuordnung zu den Fragen der Befundlisten steht
in der jeweiligen Zeile „Anlass“. Die Nutzerwörter sind wörtlich zitiert; Rechtschreibung
und Satzzeichen des Nutzers sind übernommen. Umsetzungsberichte je Paket und Testerwartungen
liegen außerhalb des Repositorys; diese Datei hält nur Entscheidung, Wirkung und angenommene Risiken.

## E11 Entsperrte Tresore werden nicht freigegeben, gesperrte schon

- Datum: 2026-11-07 (Nachricht 3). Anlass: Frage F1 zu R-FS-1 (Tresor-Freigabe).
- Nutzerwortlaut: „der entsperrte tresor wird nicht frei gegeben. der gesperrte tresor gerne, also dass
  eine andere maschiene den gesperrten tresor entsperren kann und dann darauf zugreifen, aber der
  entsperrte wird nicht frei gegeben.“
- Wirkung: Einhängepunkte entsperrter Tresor-Laufwerke sind für alle Dateioperationen gekoppelter
  Geräte gesperrt, auch für Beobachtung (Watch) und als Freigabewurzel. Der gesperrte Tresor-Container
  bleibt freigebbar; ein anderes Gerät entsperrt ihn selbst.
- Verworfene Alternative: Option B (Klartext des entsperrten Tresors über die Freigabe).
- Bewusst angenommenes Risiko: keines. Funktionseinbuße: Fernzugriff auf einen entsperrten Tresor über
  eine Freigabe entfällt; der gesperrte Tresor bleibt erreichbar.

## E12 Update-Signatur mit einem Git-Geheimnis

- Datum: 2026-11-07 (Nachricht 3). Anlass: Frage 1 der Rückfragen zu r-lokal-2 (Schlüssel der Update-Prüfung).
- Nutzerwortlaut: „ich dachte gemeint war ein signaturschlüssel von windows als developer oder so, da hab
  ich keinen. wenn was anderes gemeint war wie ein schlüssel den das programm selbst prüft beim updaten,
  gerne als git geheimniss.“ Die Wahl „A“ (eigener Schlüssel als Geheimnis des Release-Workflows) ist
  durch „gerne als git geheimniss“ getroffen.
- Wirkung: Ed25519-Signatur über ein Release-Manifest (Version; Name, Größe und SHA-256 jedes
  Assets). Der öffentliche Schlüssel steht im Programm. Der private Schlüssel ist das
  GitHub-Actions-Geheimnis `SE_UPDATE_SIGNING_KEY`. Der eingebaute Feed stellt ohne gültige Signatur
  nichts bereit; unsignierte Altversionen und `http://`-Feeds installieren nur nach Bestätigung je Update.
- Verworfene Alternativen: Windows-Authenticode (kein Zertifikat vorhanden, schützt nicht den Feed);
  Option B (Schlüssel offline beim Nutzer, Handgriff bei jedem Release).
- Bewusst angenommenes Risiko: Wer den Release-Workflow samt Geheimnis übernimmt (Schreibzugriff auf
  das Repository oder die Actions-Konfiguration), kann gültig signierte Releases erzeugen. Der
  Schutz gilt gegen manipulierte Assets, fremde Feeds und Übertragungsfehler.

## E13 MCP darf Rechte weiter mit `--yes` vergeben

- Datum: 2026-11-07 (Nachricht 3). Anlass: Frage 2 der Rückfragen zu r-lokal-5 (MCP-Rechtevergabe).
- Nutzerwortlaut: „2. B“ (Frage: soll das MCP-Werkzeug `se` das Erteilen und Erweitern von Exec-, Terminal-
  und Bildschirmrechten verweigern; Option B: nein, wie heute; ein Agent kann Rechte mit `--yes` vergeben).
- Wirkung: unverändert. Der Befund r-lokal-5 wird nicht umgesetzt. Entzug bleibt für Agenten erlaubt.
- Verworfene Alternative: Option A (MCP verweigert das Erteilen; nur ein Mensch am Gerät).
- Bewusst angenommenes Risiko: Ein durch Prompt-Injection gesteuerter KI-Agent über `se mcp` kann mit
  `--yes` Exec-, Terminal- und Bildschirmrechte erteilen. Dokumentiert als Restrisiko; die Rechte bleiben
  an die bestehende Freigabe gebunden.

## E14 Maschinenanmeldung nutzt die Relays des konfigurierten Servers

- Datum: 2026-11-07 (Nachricht 4). Anlass: Frage 3 der Rückfragen zu r-lokal-7 (Relay der Maschinenanmeldung, Option A).
- Nutzerwortlaut: „ok dann nimm den server als relay, nicht n0, also den server nicht fest einprogrammiert,
  die konfigurierte verbindung im programm. ich hätte generell ungerne third party server integriert in dem flow.“
- Wirkung: Die Maschinenanmeldung nutzt nur die Relays des im Programm konfigurierten Share-Servers, mit dessen
  Pins und Klartext-Regel. Ein gespeichertes `use_default_relays: true` bedeutet seither „Relays des konfigurierten
  Servers“; Feldname und Dateiformat bleiben. Ohne konfigurierten Server nur Direktwege. Bestehende installierte
  Gateways ohne neue Einrichtung nutzen nur Direktwege, bis sie neu eingerichtet werden.
- Verworfene Alternativen: n0-Relays (Drittanbieter); fest einprogrammierte Relay-Adresse.
- Bewusst angenommene Risiken: Ohne konfigurierten Server ist die Maschinenanmeldung nur über Direktwege
  erreichbar. Bestehende Gateways verlieren den Relay-Rückfall bis zur Neueinrichtung.
- Nachtrag 2026-11-07, Netz-Nachführung ohne neue Identität (`se login provision-network`). Anlass: Lücke L-B2
  der Integrationsprüfung; Umsetzungsplan: `docs/plaene/2026-11-07-sicherheit-direktweg/plan.md`, Abschnitt
  „Ergänzung 1 (2026-11-07)“, E24.
  - Wirkung: Ein installiertes Gateway erhält die Relays des konfigurierten Servers über
    `se login provision-network DIRECTORY [--json]`. Der Befehl schreibt in ein neues privates Bündelverzeichnis
    nur `gateway-network.json` mit den Relays und Pins des konfigurierten Servers, ohne Maschinenidentität,
    Richtlinie oder Ziel. Die Orts- und Größenprüfung entsprechen `se login provision`. Das Installationsskript
    übergibt die Datei als siebtes Argument (`native/install-login-service.sh`) bzw. als `-NetworkFile`
    (`native/install-login-service.ps1`); der vorhandene Maschinenschlüssel bleibt erhalten.
  - Wirkung bis zur Ausführung: Ein bestehendes Gateway nutzt bis zur Ausführung von Befehl und Skript nur Direktwege.
  - Verworfene Alternativen zu diesem Nachtrag: Gateways dauerhaft nur über Direktwege (widerspricht der
    Vorgabe, die Funktionalität nicht einzuschränken); automatische Auflösung im Gateway-Dienst (der Dienst
    liest kein Nutzerprofil).
  - Bewusst angenommene Risiken des Nachtrags: Ein Betreiber, der Befehl und Skript nicht ausführt, behält den
    Zustand „nur Direktwege“. Der Befehl ist nur über die CLI erreichbar; eine MCP- und eine GUI-Aktion fehlen.

## E15 Kein öffentlicher DNS-Rückfall im Verbindungsweg

- Datum: 2026-11-07 (Nachricht 4, Grundsatz „ich hätte generell ungerne third party server integriert in dem flow“).
  Anlass: Bestandsprüfung des Verbindungsablaufs (Plan, Abschnitt 11.2).
- Wirkung: Der Auflöser des Share-Endpunkts fragt nur die Systemkonfiguration (`getaddrinfo` und die
  Systemnameserver); die öffentlichen Rückfall-Resolver (Cloudflare, Google, Quad9) werden nicht befragt.
- Verworfene Alternative: Beibehaltung des iroh-Standards mit öffentlichem Rückfall.
- Bewusst angenommenes Risiko: Scheitern `getaddrinfo` und alle Systemnameserver, löst ein Relay-Name nicht mehr
  auf. Der Verbindungsweg bricht in diesem Fall ab, statt über einen Dritten aufzulösen. Auf Android hängt der
  Pfad an der Liste der Systemnameserver; liefert der Android-Leser eine leere Liste, antwortet der
  iroh-Resolver nicht mehr, `getaddrinfo` bleibt unverändert.

## E16 Abgelehnte Altanfragen verdrängbar

- Datum: 2026-11-07 (Nachricht 3). Anlass: Frage F1 zu r-entdeckung-6 (volle Liste mit 24 Altanfragen).
- Nutzerwortlaut: „F1 ja“ (Option 1: abgelehnte und widerrufene Altanfragen dürfen aus der Liste verschwinden,
  wenn die Liste voll ist; die Sperre des Geräts bleibt).
- Wirkung: Bei voller Liste verschwindet der älteste abgelehnte oder widerrufene Eintrag ohne aktive
  Berechtigung, dessen Sperre gilt. Die Sperre bleibt als Eintrag `Ignored` erhalten. Nutzerablehnungen ohne
  Sperre im Eintrag selbst bleiben stehen.
- Verworfene Alternative: Option 2 (heutiges Verhalten; neue Altanfragen scheitern, bis abgelehnte Einträge von Hand gelöscht sind).
- Bewusst angenommenes Risiko: Ein abgelehnter Eintrag ist nicht mehr sichtbar, wenn die Liste voll ist; seine Sperre wirkt weiter.

## E17 Code-Erneuerung ohne Kontaktverlust

- Datum: 2026-11-07 (Nachricht 3 mit der Bedingung, Nachricht 4 mit „alles machen“). Anlass: Frage F2 zu r-entdeckung-8
  (Direkt-Code nach dem Entfernen eines Kontakts).
- Nutzerwortlaut: „F2 wenn das gemacht werden kann ohne andere kontakte die online und offline sind zu dem zeitpunkt zu
  verlieren, ja wenn es wirklich n sicherheitsvorteil bringt“; „alles machen“.
- Wirkung: Beim Entfernen eines Kontakts erneuert das Gerät Lookup-ID und Geheimnis. Die alten Daten bleiben
  für jede Freigabe `Accepted` oder `Reconfirm` gültig, bis das Gegenüber die neuen über eine gepinnte,
  authentifizierte Sitzung übernommen hat. Entfernte Kontakte verlieren den Lookup-Zugang und die Adressliste der
  Server-Presence sofort; im LAN gilt Risiko (1). Höchstens vier Erneuerungen laufen gleichzeitig. Der Server
  bekommt eine zweite, reduzierte Lookup-ID innerhalb der bestehenden Grenze von 64 Lookups je Client;
  keine neue Rolle.
- Verworfene Alternativen: automatische Erneuerung ohne Übergang (verliert Kontakte); Geheimnis je Kontakt
  (größerer Protokollwechsel, eigener Batch).
- Bewusst angenommene Risiken: (1) Während eines Übergangs sieht ein entferntes Gerät die reduzierte Presence
  (Relay-URL, Node-ID) und im LAN die Ankündigung unter der alten ID samt Adressliste (versiegelt mit dem alten
  Geheimnis als `v=3` und im Klartext als `v=2`), bis die Lookup-ID endet; mit Altgegenstellen dauerhaft. (2)
  Altgegenstellen ohne Erneuerungsunterstützung halten den Übergang offen; die Nutzerbedingung „ohne
  Verlust“ hat Vorrang. (3) Geräte, die nur im LAN ohne Server-Presence sind, stellen nicht um.

## E18 Schlüsselanmeldung ist der Standard

- Datum: 2026-11-07 (Nachricht 4). Anlass: Frage F1 zu R-SERVER-05 (Altclients ohne Schlüsselanmeldung).
- Nutzerwortlaut: „alles machen, ich denke es laufen keine mit so alten versionen. keine liste, fang an.“
- Wirkung: Der Server verlangt `key_login_v1` (Standard, `KeyLoginRequired` bei fehlendem Schlüssel). Apps bis
  v0.5.169 erhalten die Meldung „this server requires key login; update Smart Explorer“. Der
  Betreiberschalter `--allow-legacy-clients` (Umgebungsvariable `SE_SHARE_ALLOW_LEGACY_CLIENTS=1`) stellt
  den bisherigen Kompatibilitätsmodus her. `--require-key-login` bleibt gültig und ohne Wirkung; zusammen mit
  `--allow-legacy-clients` ist der Start ein Fehler.
- Verworfene Alternative: Option 1 (Altclients weiter ohne Schlüssel zulassen als Standard).
- Bewusst angenommenes Risiko: Ein Betreiber, dessen Geräte noch mit Apps bis v0.5.169 arbeiten, muss den
  Schalter setzen oder die Apps aktualisieren. Ohne Schalter werden solche Apps abgewiesen.

## E19 Keine Liste zugelassener Schlüssel am Server

- Datum: 2026-11-07 (Nachricht 3 zur Serverrolle, Nachricht 4 „keine liste“). Anlass: Frage F2 zu R-SERVER-09
  (freiwillige Liste zugelassener Geräteschlüssel).
- Nutzerwortlaut (Nachricht 3): „der shareserver muss neue frei aufnehmen können, darf aber natürlich nicht frei
  alles was mit ihm verbunden ist exposen.“ „ich hätte aber eigentlich gerne dass der server seine rolle nicht
  erweitert und rein als resolution server zwischen den mit ihm verbundenen geräten dient, keine zugänge verwaltet
  oder so.“ (Nachricht 4): „keine liste“.
- Wirkung: Der Server nimmt neue Geräte ohne Zutun auf, führt keine Liste, verwaltet keine Zugänge und keine
  Konten. Die Daten, die er sieht, bleiben auf Anwesenheit und Adressen begrenzt. Kein Paket gibt dem Server eine
  neue Rolle.
- Verworfene Alternative: Option 2 (freiwillige Liste; neue Geräte müssten vor dem Koppeln eingetragen werden).
- Bewusst angenommenes Risiko R-SERVER-09: Jeder, der sich am Server registriert, darf Relay und QAD nutzen und
  Entdeckungsangebote sehen. Die Kopplung mit beliebigen Geräten bleibt ohne Einrichtung möglich. Die Relay-Grenzen
  begrenzen Last, nicht den Zugang.

## E20 Signierte Release-Assets im Release-Weg

- Datum: 2026-11-07. Anlass: Umsetzung von E12 im Release-Workflow.
- Wirkung: `.github/workflows/release.yml` bricht vor jedem Build ab, wenn das Geheimnis `SE_UPDATE_SIGNING_KEY`
  fehlt. Der Publish-Schritt erzeugt `release-manifest` und `release-manifest.sig`, prüft die Signatur gegen den
  öffentlichen Schlüssel vor Commit, Tag und Upload und veröffentlicht 28 Assets (26 Build-Assets plus das Paar).
  Das Geheimnis liegt nur in der Umgebung dieses Schritts.
- Verworfene Alternative: Signatur außerhalb des Workflows (Handgriff bei jedem Release).
- Bewusst angenommenes Risiko: Übernahme des Workflows samt Geheimnis (siehe E12). Ein Schlüsselwechsel macht
  neue Releases für installierte Versionen mit dem alten öffentlichen Schlüssel unprüfbar; sie verweigern das
  Update (fail closed).

## G1 Budget automatisch gebundener Dienste aus dem Betriebssystem

- Datum: 2026-11-07. Anlass: Befund r-entdeckung-9 (Gesamtobergrenze automatischer Listener) und Lücke L-B3
  der Integrationsprüfung (Desktop: Dienste oberhalb einer festen Grenze von 256 nicht mehr nutzbar). Umsetzungsplan:
  `docs/plaene/2026-11-07-sicherheit-direktweg/plan.md`, Abschnitt „Ergänzung 1 (2026-11-07)“, E25.
- Nutzerwortlaut: keine eigene Äußerung des Nutzers zu diesem Punkt. Grundlage ist die Vorgabe „Ohne dass du jetzt
  die Funktionalität einschränkst.“ (Nachricht 1). Diese Entscheidung ist daher kein gesonderter Nutzerentscheid.
- Wirkung: Die feste Grenze von 256 automatischer Listener entfällt. Das Budget kommt aus dem Betriebssystem.
  Linux und Android heben das weiche Limit für Dateideskriptoren einmal je Prozess auf das harte Limit an; das
  Budget beträgt ein Viertel des danach geltenden weichen Limits, mindestens 256. Windows kennt keine solche
  Grenze; dort gibt es keine Obergrenze automatischer Listener außer der Grenze je Gerät. Die Grenze von 512
  Diensten je Gerät (`MAX_SERVICES`) bleibt auf allen Plattformen.
- Verworfene Alternativen: Aktion „Öffnen“ auf dem Desktop (neuer IPC-Befehl zwischen Oberfläche und Worker sowie
  Einträge in CLI, MCP und GUI); Rückfrage an den Nutzer (nicht nötig, weil die Abhilfe ohne Funktionsverlust
  möglich ist).
- Bewusst angenommene Risiken: (1) Bei hartem Limit von 1024 Deskriptoren oder weniger beträgt das Budget 256;
  Dienste oberhalb von 256 werden dann abgewiesen. Auf solchen Systemen ist das gegenüber dem Stand HEAD eine
  Einschränkung; nicht gemessen. (2) Windows: Annahme, dass Windows keine Deskriptorgrenze dieser Art hat; nicht
  nachgewiesen.

## Offene Entscheidung

- Q1 (s4, Machbarkeit unverschlüsselter Übertragung): offen. Die Frage, ob eine je Übertragung zuschaltbare
  unverschlüsselte Übertragung gebaut werden soll oder zuerst nur die Messung der Verschlüsselungskosten
  erfolgt, steht in `docs/plaene/2026-11-07-sicherheit-direktweg/machbarkeit-unverschluesselt.md`. Ohne Antwort
  wird nichts davon gebaut.
