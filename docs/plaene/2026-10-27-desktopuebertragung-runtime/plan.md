# Desktopübertragung: blockierende Metadaten im asynchronen Host

Stand: 2026-10-27. Ausgangspunkt: veröffentlichtes v0.5.185, Quellstand
`b439d90e`. Dieser Plan enthält keine lokalen Geräte- oder Kontodaten.

## Auftrag und erste Planung

Die nach dem Update gemeldete fehlende Desktopanzeige wiederherstellen.
Ein belegter Installationskontextfehler wird im Betrieb korrigiert. Zusätzlich
zeigt der betroffene Bildschirmweg einen nativen Runtime-Panic; dessen
Korrektur muss einschließlich der betroffenen bestehenden Funktionen ausgeliefert
werden. Keine lokalen Builds oder Tests, keine Änderung von Freigaberechten.

## Bestand und erste Recherche

- `screen/os/linux_os/session_detect.rs` und `host_backend.rs`: X11-Erkennung
  hängt vom Prozesskontext ab, die Monitorliste benötigt Zugriff auf den X-Server.
- `screen/core/icon_host.rs`: Metadatenkanal ruft `list` und `icon` synchron
  innerhalb seines asynchronen Handlers auf. Die Aufrufe halten über
  `share/core/node_screen.rs::NodeAuthority::admit` die gemeinsame Autorisierung.
- `screen/core/host_session_control.rs`: derselbe Fenster-/Icon-Weg besteht
  innerhalb laufender Bildschirmübertragungen; Familienlisten sind verbunden.
- `screen/os/linux_os/host_backend.rs::windows` ergänzt die Liste über
  `app_icons::attach`. `app_icons.rs::current_theme` verwendet `zbus::blocking`.
- Der beobachtete Panic lautet „Cannot start a runtime from within a runtime“;
  anschließend scheitert Share-Presence mit „Share-State gesperrt“. Damit ist
  eine bloße Unterdrückung der Fehlermeldung keine Reparatur.
- Cargo.lock bindet Tokio 1.52.3 und zbus 5.16.0. Die offizielle
  [zbus-Dokumentation](https://docs.rs/zbus/latest/zbus/blocking/index.html)
  verbietet die blockierende API im Async-Kontext. Die online abrufbare
  Dokumentation ist 5.19.0; der versionsgebundene Online-Abruf scheiterte.
  Der gleiche Vertrag wurde deshalb zusätzlich in der tatsächlich lokal
  vorhandenen Registry-Quelle von `zbus-5.16.0/src/blocking/mod.rs:10–16`
  gelesen und bestätigt.

## Zweite Recherche und Entscheidung

Die [Tokio-Dokumentation zu spawn_blocking](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)
beschreibt die geeignete Ausführungsgrenze für endliche blockierende Arbeit.
Begonnene Aufgaben lassen sich damit nicht abbrechen. Ein Fix muss daher neben
der korrekten Ausführungsumgebung auch Rechteentzug, Verbindungsende, begrenzte
Arbeit und die Veröffentlichung verspäteter Ergebnisse berücksichtigen.
Quellen geprüft am 2026-10-27, zusätzlich gegen die tatsächlich vorhandene
Registry-Quelle `tokio-1.52.3/src/task/blocking.rs` und den
[offiziellen Tokio-Tag](https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.52.3/tokio/src/task/blocking.rs).

Nicht akzeptabel sind entfernte Icons/Fensterlisten, abgeschwächte
Autorisierung, erfundene Desktopvariablen zur Umgehung eines Programmfehlers
oder ein Fortsetzen mit vergiftetem Autorisierungszustand. Bestehende synchrone
Aufrufer sowie Linux-, Windows- und Android-Grenzen bleiben erhalten.

## Konkrete Meilensteine

1. **Native Korrektur:** Metadatenabfragen aus dem Async-Host dürfen die
   blockierenden Plattformadapter verwenden, ohne den Executor oder die
   Share-Autorisierung zu beschädigen. Betroffen sind Metadatenkanal,
   laufende Bildschirmübertragung, Fenster-/Icon-/Familienlisten und ihre
   Begrüßungsinformationen. Der implementierende Agent liefert selbst
   Fehlerbild, direkt und indirekt verbundene Funktionen, Fixplan, Fix und
   anschließende statische Regressionsbewertung. Erwartung: identische
   Fenster-/Icon-Fähigkeiten, gültige Rechteprüfung, kein Nested-Runtime-Panic.
2. **Kritische Reviews:** Nach Integration normaler Code-/Quellenreview
   (Astra xhigh) sowie getrennte Regressionsreviews (Astra max und Sol 6.1 max)
   gegen v0.5.185. Reale Befunde gehen an denselben Fixautor zurück.
3. **Eine Remote-Auftragssuite:** Eng begrenzter Einstieg für die neue
   Runtime-Regressionsabdeckung und betroffene bestehende Icon-/Rechteverträge.
   Sie nutzt inkrementelle Entwicklungsartefakte und prüft zusammen
   Metadaten-/Live-Wege, Ergebnisintegrität, Rechteentzug und weiter nutzbaren
   Share-Zustand. Ein Remote-Workflow, mindestens 30 Minuten Timeout;
   keine lokalen Ausführungen und keine breite Matrix.
4. **Terminaler Release und Installation:** Nach Auswertung der Suite den
   bestehenden vollständigen Remote-Release-Weg für den nächsten Patch
   verwenden. Tags bleiben unverändert. Anschließend die vorhandenen lokalen
   Installationen aus den verifizierten veröffentlichten Artefakten erneuern;
   den vollständigen Desktopkontext beim Dienstneustart erhalten. Erwartung:
   veröffentlichte, versionskonsistente Artefakte und verbundener Dienst mit
   erreichbarem Bildschirm. Eine nicht beobachtete Remote-Bildübertragung wird
   nicht als erfolgreich behauptet.

## Stand

- Dienstkontextfehler im Betrieb korrigiert; Monitorzugriff nachgewiesen.
- Nativer Metadatenfehler eingegrenzt; Umsetzung und Remote-Auslieferung offen.
- Nach regulärem Wiederanlauf ist die Share-Verbindung wieder online. Dies ist
  kein Nachweis für die noch ausstehende Runtime-Korrektur.
