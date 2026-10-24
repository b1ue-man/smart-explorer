# Richtung des Projekts (Stand 2026-10-24)

Dieses Dokument hält fest, wohin smart-explorer langfristig gehen soll. Es ist kein Arbeitsauftrag: Offene Arbeit steht nur in `docs/TODO.md`, die nächsten Läufe im vorläufigen Batchplan `docs/plaene/2026-10-24-batches/batches.md`.

Kennzeichnung:
- **Nutzer** = Aussage des Nutzers.
- **Befund** = im Code nachgelesen (Stand `076da6a3`).
- **Einordnung** = eigene Bewertung des Agenten.
- **Annahme** = ungeprüft, ausdrücklich so benannt.

## Herkunft

Nachricht des Nutzers vom 2026-10-24. Sie ist eine Sprachtranskription; der Nutzer schrieb dazu: „das ist eine transkription also rechnen mit fehlern in der aufnahme“. Wörtliche Kernsätze:

- „Der Grund, warum ich … diese Fensterebene will mit dem Streaming, ist wie gesagt, dass ich die Verbindung zwischen Geräten grenzenlos machen will.“
- „Mein langfristiger Plan ist wirklich die Grenzen verschwimmen zu lassen, was auf einem Gerät möglich ist, auf den anderen Geräten, mit denen es verbunden ist, auch möglich zu machen, in jeglicher Denkweise.“
- „Shared Execution, dass Dinge, die keinen großen Datenaustausch benötigen, keinen großen Live-Datenaustausch, auch auf anderen Geräten ausgeführt werden können. … Dass also auf Prozessebene Dinge auf mehrere Geräte verteilt werden können. Über die Netzwerkverbindungen, über LAN, WLAN, über so Geräte und Mac.“ Der letzte Teil ist in der Transkription unklar.
- Korrektur des Nutzers zum Beispiel dafür, ebenfalls am 2026-10-24: „Mein Beispiel war wie zum Beispiel ein Bild von Smart Explorer. Die Ausführung von Smart Explorer selbst ist ein Beispiel zu, was nicht auf einem anderen Gerät laufen kann, weil eben dieser direkte Hardware-Zugriff notwendig ist, um es effizient laufen zu lassen.“
  - Annahme zur Transkription: „Bild“ meint „Build“.
  - Ein Build von Smart Explorer ist dann das Beispiel für geteilte Ausführung.
  - Der Betrieb von Smart Explorer selbst ist das Gegenbeispiel.
- „Der zweite Punkt wäre, dass eben auf Treiber-Ebene geteilt wird. Also zum Beispiel, ich von meinem Laptop aus was drucken kann auf einem Drucker, der mit einem PC in einem anderen Netzwerk verbunden ist. Oder dass ich was programmiere und In TIA-Portal und das dann auf eine SPS in einem anderen Netzwerk hochladen kann.“
- „was die geteilte Hardware angeht, weiß ich … wirklich nicht, wie realistisch das ist. Wahrscheinlich … ziemlich unrealistisch, vielleicht sogar.“
- „als wenn ich jetzt einen Node-Server starte, der über den Localhost erreichbar ist, so was ist natürlich realistisch … Wird der Localhost halt sozusagen geforwardet …, das ist kein Thema“.
- „aber was aktive Prozesse angeht, das zu verteilen. Zum Beispiel nehmen wir jetzt einen Rack-Build, wo … Vektoren für einen Rack berechnet werden. … ob es am Ende schneller oder langsamer ist, wie wenn es auf dem eigenen Gerät läuft, mit den ganzen Daten, die hin und her geschickt werden müssten. Schwer zu evaluieren. Eigentlich unmöglich vorher.“
  - Annahme zur Transkription: „Rack“ meint RAG, also Vektoren für eine Retrieval-Augmented-Generation-Suche.

Grenzen, die der Nutzer gesetzt hat:
- „Dieser Batch jetzt ist erstmal fixe und Verbesserungen der jetzt eingefügten Implementierungen.“
- „ein paar Kleinigkeiten, da kannst du vielleicht in den aktuellen Batch einweben oder berücksichtigen in der Umsetzung, wenn du weißt, was das langfristige Ziel ist. Aber die großen Sachen noch nicht unbedingt.“
- Der Schluss der Nachricht ist abgebrochen: „einzelnen Komponenten richtig funktionieren. Es ist wichtig, dass die Einzelnen.“ Annahme: Gemeint ist, dass zuerst die einzelnen Komponenten richtig funktionieren müssen.

## Grundsätze für jede Ebene

- **Direkt zwischen den Geräten:** gleiches LAN oder WLAN zuerst, dann die übrigen Direktwege, das Relay nur als gekennzeichneter Rückfall. Der Server vermittelt nur. Siehe `docs/ARCHITEKTUR.md`, Abschnitt „Zweck“.
- **Rechte je Gerät an einer Stelle:** Siehe `EINSTELLUNGEN-GERAETE`, P18.
  - Befund: Das Rechtemodell kennt eine feste Liste von Fähigkeiten, nämlich Befehle, Terminal, Bildschirm ansehen und Bildschirm steuern (`native/src/share/core/access_grant.rs:18-37`).
  - Jede neue Ebene kommt dort als eigene Fähigkeit dazu.
- **Erst die einzelnen Komponenten zuverlässig,** dann die nächste Ebene.

## Ebenen und Einordnung

| Ebene | Nutzer | Bestand | Einordnung |
|---|---|---|---|
| Dateien und Laufwerke | Grundfunktion | Direktfreigaben, Laufwerke, Sync (Befund) | vorhanden |
| Terminal und Befehle | „Herunter dazu kommen auf Terminal-Ebene“ | `se exec`, `se shell` mit Rechten (Befund) | vorhanden |
| Fenster und Programme | Fensterebene, Programme auf allen Geräten | geplant als P4 und P5 (`DESKTOP-ALLE-PROGRAMME`, `DESKTOP-PROGRAMME-STARTEN`) | groß, schon im vorläufigen Batch 5 |
| Weiterleitung von localhost | „so was ist natürlich realistisch“ | keine Weiterleitung von Ports in `share`, `cli`, `daemon` (Befund: Suche ohne Treffer); für RDP ist eine Weiterleitung durch die Share-Verbindung als neuer Baustein vermerkt (Notiz Fernzugriff, „Abhängigkeiten“) | mittel; als allgemeiner Baustein einwebbar (unten) |
| Geteilte Ausführung | Dinge ohne großen Live-Datenaustausch auf anderen Geräten ausführen. Beispiel: ein Build von Smart Explorer. Gegenbeispiel: der Betrieb von Smart Explorer selbst, „weil eben dieser direkte Hardware-Zugriff notwendig ist“ | `se exec` führt Befehle auf einem anderen Gerät aus | mittel bis groß: Auftrag „X auf Gerät Y mit diesen Dateien“, Ergebnis zurück. Eigene Folgerung: Geeignet ist Arbeit mit großem Rechenanteil und wenig Austausch während des Laufs, etwa ein Build; ungeeignet ist Arbeit, die die Hardware des eigenen Geräts braucht, etwa Dateizugriff mit voller Laufwerksgeschwindigkeit. Ein Teil ist mit P5 (Programmstart) einwebbar |
| Verteilung aktiver Prozesse | „Bin ich mir echt nicht sicher“ | nichts | groß, Forschungsthema. Annahme: lohnt nur für Arbeit, die sich in unabhängige Stücke mit kleinen Ein- und Ausgaben teilen lässt; ein laufender Prozess lässt sich nicht durchsichtig auf andere Geräte verteilen. Nicht jetzt |
| Treiberebene: Drucker im anderen Netz | Drucken vom Laptop auf einem Drucker an einem PC in einem anderen Netz | nichts | Annahme: Ein Netzwerkdrucker oder ein vom PC freigegebener Drucker wird über die Weiterleitung eines Druckports erreichbar und fällt dann unter die Weiterleitung. Ein USB-Gerät selbst zu teilen (USB über IP) braucht Treiber und ist groß. Nicht jetzt |
| Treiberebene: SPS über TIA Portal im anderen Netz | Programm auf eine SPS in einem anderen Netz laden | nichts | Annahme: Die Online-Verbindung zu einer SPS über ihre IP-Adresse könnte über eine TCP-Weiterleitung laufen. Gerätesuche über Layer 2 bräuchte eine virtuelle Netzwerkkarte und ist groß. Lücke: die tatsächlich genutzten Protokolle und Ports |

## Einwebungen in die vorläufigen Batches

Diese Punkte verändern keinen Batch-Inhalt. Sie werden in der Planung des jeweiligen Batches berücksichtigt, damit spätere Ebenen auf den Bausteinen aufsetzen können.

- **Batch 2 (`DIREKT-ZUERST`, P28):** Kennwerte je Gerätepaar festhalten: Weg (LAN, direkt, Relay), Umlaufzeit und erreichte Bandbreite. Sie sind später die Grundlage, um zu entscheiden, ob sich Ausführung auf einem anderen Gerät lohnt. Diese Frage hält der Nutzer heute für „Eigentlich unmöglich vorher“ zu beantworten.
- **Batch 3 (`EINSTELLUNGEN-GERAETE`, P18):** Die Geräteseite zeigt die Rechte als Zeilen aus der Fähigkeitsliste. Eine neue Fähigkeit erscheint dann ohne neuen Umbau, zum Beispiel Weiterleitung, Programmstart oder Anmeldebildschirm.
- **Batch 4 (`SCREEN-RDP`, P1):** Die Weiterleitung für RDP wird als allgemeiner Baustein geplant:
  - TCP-Weiterleitung über die direkte Geräteverbindung, in beide Richtungen;
  - ein eigenes Recht mit Freigabeliste je Gerät und Port;
  - Bedienung im Terminal, in den Apps und über MCP.

  RDP ist dann ein Anwendungsfall. Auf demselben Baustein setzen localhost-Weiterleitung, Druckports und SPS-Verbindungen über IP auf (Annahme für die beiden letzten).
- **Batch 5 (`DESKTOP-PROGRAMME-STARTEN`, P5):** Der Programmstart nimmt das Zielgerät als Parameter. Sein Recht (E4) ist so geschnitten, dass eine spätere geteilte Ausführung es weiterverwenden kann.

## Nicht jetzt (große Zusatzarbeit)

- geteilte Ausführung als eigenes Auftragsmodell mit Dateien und Ergebnissen;
- Verteilung laufender Prozesse oder Rechenlast auf mehrere Geräte;
- Treiberebene: USB-Geräte über das Netz, virtuelle Netzwerkkarte für Layer 2, Druckerfreigabe als eigene Funktion.

## Lücken

- Protokolle und Ports, die TIA Portal für Online-Zugang und Gerätesuche nutzt. Nicht recherchiert.
- Druckwege zwischen Windows, Linux und Android für freigegebene und Netzwerkdrucker. Nicht recherchiert.
- Stand und Treiberbedarf von USB über IP unter Windows und Linux. Nicht recherchiert.
- Transkription: „über so Geräte und Mac“ ist unklar. Offen ist, ob macOS als weitere Plattform gemeint ist; heute unterstützt se Windows, Linux und Android.
