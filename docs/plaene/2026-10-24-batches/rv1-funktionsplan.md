# RV1: Funktionsbefunde aus Lauf 40406246032

Stand: 2026-10-26. Kandidat `1ffbf4b0bc90b2479947680abcd0a5941cce00f8`.
Dies ist die Fortsetzung des beauftragten Gesamtbatches B1–B8/P31/P32,
kein zusätzlicher Funktionsauftrag und keine neue Testkampagne.

## Erster Plan aus dem beobachteten Ergebnis

Windows und Linux kompilieren die Bibliotheks-Prüfbinärdatei, `se` und
`egui_glow` erfolgreich. Ihre Funktionsstufen sind fehlgeschlagen. Die
Vorbereitung ist erfolgreich. Beim Statusabruf 22:40:17 UTC war auch die
Android-Build-/JVM-Stufe beendet. Native Bibliothek und App-APK wurden gebaut;
Instrumentation-Kompilierung und JVM-Prüfung scheiterten. Nur Cache-Nachbereitung
war noch aktiv. Die unten dokumentierte Startgrenze ersetzt das zunächst
vorgesehene Warten auf sämtliche CI-Nachbereitung.

Ziel dieses Fixdurchgangs: die tatsächlich fehlgeschlagenen Funktionen
und fehlerhaften Prüfvoraussetzungen korrigieren, ohne Bestandsschutz,
Rechteprüfung, Wiederaufnahme, Datenhaltbarkeit oder Leistungsanforderungen
zu schwächen. Jeder Fixautor liefert selbst Funktionsliste,
direkte/indirekte Verknüpfungskarte, konkreten Erhaltungsplan, Umsetzung
und anschließende eigene Anti-Regressionsreview. Die bereits abgeschlossenen
unabhängigen Reviews werden nicht zu Fixautoren umfunktioniert.

Der vollständige Desktopbefund umfasst F01–F56 sowie den gesonderten
P27-Profilwechselfehler F57. Mehrere fehlgeschlagene Fälle können dieselbe
Ursache haben. Die Kennungen bezeichnen Beobachtungen, keine bereits
bewiesenen eigenständigen Produktfehler. Compilerwarnungen, erfolgreiche
Prüffälle mit erwarteter Panic und ausdrücklich unverifizierte Hardwarepfade
werden nicht als neue Fehler ausgegeben.

## Bestand und notwendige begrenzte Ergebnisse

| Kennung | Beobachteter Fehler und Quellgrenze | Erforderliches Ergebnis / Erhaltungsvertrag |
|---|---|---|
| F01 | Optischer Index: vorhandener nichtoptischer Eintrag fehlt nach Laden; `app/os/shared/optical_index.rs`, `folder_index` | Optische Beweiswiederverwendung erhält den vollständigen zulässigen Index; bestehende Filter und Tresorausschluss bleiben wirksam. Ursache im Schreiben/Laden/Fixture noch offen. |
| F02–F07 | Linux-Appstart-Fixture scheitert am privaten Journalverzeichnis; `apps/os/linux_os/mod.rs`, `apps/os/batches_launch_task_tests.rs` | Appstarts bleiben dauerhaft und idempotent, Journal tatsächlich privat; eine ungeeignete Fixture darf keine Aufweichung der Eigentums-/Modusprüfung bewirken. |
| F08 | `se apps icon` kollidiert mit automatisch propagierter `version`; `cli/apps.rs`, `cli/mod.rs`, `cli/completions_access.rs` | CLI-Baum und Vervollständigung sind gültig; Icon-Datenversion und Programmversionsausgabe bleiben eindeutig erreichbar. |
| F09 | Codex-Entfernung verliert eine Kopfkommentarzeile; `cli/plugin/batches_codex_task_tests.rs`, `cli/plugin/codex.rs` | Migration und Deinstallation erhalten nicht verwaltete Konfiguration und Kommentare; verwaltete Umgebungswerte bleiben korrekt zugeordnet. |
| F10–F12, F16-Windows, F44, F48–F49, F51–F56 | Windows-Kopieren/Verschieben/Wiederaufnahme scheitert mit Zugriff verweigert bei Haltbarkeitsbestätigung; `local_access/os/windows/publication.rs`, `copy`, `vfs/os/windows`, `transfer` | Erfolgreiche lokale Veröffentlichung bleibt als solche nachweisbar, Recovery bleibt wiederholbar; keine Erfolgsquittung ohne ausreichenden Beleg und kein Löschen unbestätigter Quell-/Quarantänebestände. Gemeinsame Ursache wird je Aufrufpfad belegt. |
| F13 | Windows-Exec: abgeschlossene Registrierung schließt Client, Kontrollleser endet nicht rechtzeitig; `daemon/os/shared/batches_exec_lifetime_task_tests.rs`, Exec-Lebensdauer | Terminaler Abschluss beendet Kontrollbesitz und lässt erfolgreich beendete Befehle ohne neuen Cancel. |
| F14–F15 | Recovery-Registry liest `null` als Sequenz; `daemon/os/shared/mount_inactive_recovery.rs`, Registry-Leser und Eigentümerfixture | Aktive/inaktive Eigentumsübergänge sowie Rücknahme nach Fehler erhalten kompatible lesbare Registry und genau einen Cachebesitzer. |
| F17 | Drive-Seitenfixture erhält HTTP 400; `gdrive/core/gui_task_tests.rs`, Scan-/Listenpfad | Leere Zwischenseiten werden bis zum Ende verfolgt; leere Kindordner bleiben fehlerfrei; Providerfehler werden weiter ehrlich gemeldet. |
| F18–F35 | MCP-Lebensdauer/Deinstallation: Windows-Vorfahren-ACL und Linux nicht lesbare fremde Prozesse; `mcp_launcher/os/windows/security.rs`, `mcp_launcher/os/linux_os/mod.rs`, Lebensdauer-/Deinstallationsfixtures | Eigene Installation bleibt bedienbar und entfernbar, fremde Dateien/Prozesse bleiben unberührt; unbekannter eigener Prozesszustand bleibt sicher behandelt. ACL-/Prozessursache vor jeder Lockerung belegen. |
| F36–F37 | Worker-Budgetdiagnose und Windows-Capture-Resize-Erwartung stimmen nicht; `screen/core/batches_frame_budget_task_tests.rs`, `screen/os/windows/batches_capture_task_tests.rs` | Tatsächliche Grenzen greifen vor Payloadlesen beziehungsweise nativer Allokation; alte Besitzer und neue Generationen sind vollständig bilanziert. Keine bloße Änderung einer erwarteten Zeichenfolge ohne Vertragsnachweis. |
| F38 | Windows-Qualitätsprovenienz findet Frozen-Marker nicht; `screen/os/native_ffi/quality_acceptance_provenance.rs` | Unveränderliche Releasequelle und eingefrorener Vergleich bleiben inhaltlich identisch und auf Windows/Linux auswertbar; keine Hash-/Vergleichsumgehung. Zeilenenden sind eine zu prüfende Hypothese. |
| F39–F40 | X11-Capture-Fixture verliert Verbindung beim Aufbau; `screen/os/linux_os/batches_capture_recovery_task_tests.rs`, `native/review-task-native.py` | Isolierter X-Server bleibt während der gemeinsamen Auswahl verfügbar; Resize-/Captureprüfung bleibt unverändert aussagekräftig. Reset zwischen letzten Clients ist eine Hypothese, kein bewiesener Produktdefekt. |
| F41 | Schreibschutz wird als `ReadOnlyFilesystem` statt `PermissionDenied` gemeldet; `share/core/fs_guard_extensions.rs` | Schreibzugriff bleibt verweigert; Fehlerklassifikation entspricht Backend-/OS-Vertrag ohne falsche Pfadfehlermeldung. |
| F42 | Netzwechsel erreicht den Noq-Epochzähler nicht; `share/core/batches_route_task_tests.rs`, vendorte Iroh-/Noq-Netzwechselgrenze | Reale Netzänderung sperrt alte Grants vor neuem Nutzverkehr; keine synthetische Erfolgsumgehung im Prüffall. |
| F43 | Gehaltene Windows-Root verhindert die in der Fixture versuchte Umbenennung; `source_media/tests.rs`, `local_access/os/windows` | Effektiver physischer Root und optische Abbruch-/Prioritätszuordnung bleiben gebunden; Plattformvertrag entscheidet, ob Umbenennung verhindert oder korrekt verfolgt wird. |
| F16-Linux, F45–F47, F50 | Veröffentlichungsquittung verweigert Zielpfad in Ordner-/Missing-only-Transfers; `transfer/os/shared/engine/file_recovery_receipts.rs`, `file_recovery.rs` | Wörtliche Backendpfade, Remoteidentität, zulässige Ordnerreservierung und fehlende Restarbeit bleiben kompatibel; Quittungen bleiben gegen fremde Ziele und manipulierte Pfade gebunden. |
| F57 | P27 nächstes Netzprofil scheitert an `tc`: `Exclusivity flag on, cannot modify.`; `native/review-task-network.sh` | Alle geplanten Profile laufen nacheinander in derselben isolierten Umgebung; vorhandene Rate-/Latenz-/Queue-Orakel bleiben unverändert. Erster Referenz-/Screenabschluss ist vorhanden, Gesamtabnahme fehlt. |

## Recherchefragen vor dem endgültigen Zuschnitt

1. Windows-Veröffentlichung: erforderliche Zugriffs- und Sharingrechte der
   gehaltenen Datei-/Verzeichnisobjekte, tatsächliche Flushgarantie und
   Unterschied zwischen Quittungsdatei und Nutzdatei.
2. Pfadquittungen: normalisierte Zielidentität, wörtliche Backendnamen und
   Beweisbindung in Erstveröffentlichung und Wiederaufnahme.
3. Eigentum: Linux-Modus der Fixture, Windows-ACL-Vererbung, Bedeutung
   unbekannter fremder `/proc`-Einträge und erhaltene fremde TOML-Kommentare.
4. Capture/Transport: aktuelle Budgetrechnung, Zeilenenden des Checkouts,
   X-Server-Lebensdauer sowie tatsächlicher Iroh-Netzwechselpfad.
5. Persistenz: Indexfilter gegenüber Fixturepfad und fehlende Registryfelder
   gegenüber vorhandener `null`-Repräsentation.
6. P27: Austausch bestehender Qdiscs und Portfilter ohne Zustand des
   vorherigen Profils, ausschließlich im eigenen Netzwerk-Namespace.

Die maßgeblichen Artefakte und Hashes stehen in
[rv1-remote-suite.md](../../refs/rv1-remote-suite.md).
Die vollständigen Fallnamen werden aus den tatsächlichen fehlgeschlagenen
Binärfällen übernommen; erfolgreicher Stdout ist keine Fehlerquelle.

## Reihenfolge und Abschlussgrenze

Android wird anhand seines abgeschlossenen Fehlerartefakts ergänzt. Erst nach
Detailmeilensteinen, zweiter Lückenrecherche und vollständigen Schreibflächen
folgt die Modellwahl. Agenten beginnen erst nach Ende der Build-/Testarbeit.
Jeder Quellfix erhält seine Autorenreview. Main integriert Schnittstellen,
registriert nötige Fälle im bestehenden Manifest und aktualisiert den Rootgraph.
Nach Commit und Push läuft ausschließlich derselbe Remote-Einstieg erneut.
Kein lokaler Build oder Test, kein zusätzlicher Testeinstieg, keine vorgezogene
Version und kein Zwischenrelease. Nach erfolgreicher Auswertung folgt der eine
beauftragte vollständige Remote-Release.

## Erster Recherchepass und abgeglichene Verträge

Abgerufen beziehungsweise am Kandidaten gelesen am 2026-10-26:

- Windows [`ReOpenFile`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-reopenfile)
  bindet an das bestehende Objekt; verlangte Zugriffs-/Sharingrechte dürfen
  offenen Handles nicht widersprechen. [`NtFlushBuffersFileEx`](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntflushbuffersfileex)
  dokumentiert Daten-/Metadatensynchronisierung und fehlende Schreibrechte als
  Fehler. Ein generelles Ignorieren von Zugriff verweigert ist damit kein Fix.
- Die lokal vorhandene, durch `Cargo.lock` gepinnte Primärquelle
  `clap_builder 4.6.0`, `src/builder/command.rs:1493/1527`, bestätigt
  `disable_version_flag` und die vererbte `propagate_version`-Semantik;
  `debug_asserts.rs:100` bestätigt die Eindeutigkeit von Argumentnamen.
  Die versionsspezifische Webseite war nicht abrufbar; die lokale
  Paketquelle ist der Versionsbeleg, nicht eine behauptete Webprüfung.
- [`tempfile 3.27.0`](https://docs.rs/tempfile/3.27.0/tempfile/struct.Builder.html#method.permissions)
  dokumentiert Verzeichnisrechte `0777` vor Anwendung der Umask. Der Appfall
  übergibt genau dieses bereits bestehende Tempverzeichnis an die
  Journalprüfung. Der Schutz des privaten Journals bleibt bestehen.
- [`/proc/pid/exe`](https://man7.org/linux/man-pages/man5/proc_pid_exe.5.html)
  unterliegt einer Ptrace-Zugriffsprüfung. Nicht lesbare Prozessbilder
  belegen daher nicht, dass eine eigene Generation benutzt wird; sie dürfen
  zugleich nicht pauschal als unbenutzt umgedeutet werden. Die relevante
  eigene Besitz-/Zugriffsgrenze muss der Fixautor vollständig herleiten.
- [`toml_edit 0.25.11+spec-1.1.0`](https://docs.rs/toml_edit/0.25.11+spec-1.1.0/toml_edit/struct.Table.html)
  wurde zusätzlich aus der gepinnten lokalen Paketquelle gelesen:
  `Table::remove` entfernt das gesamte Item samt dessen Dekoration.
  Die Kommentare einer entfernten eigenen Tabelle müssen von fremden
  Dokumentkommentaren unterschieden werden.
- [Git-Zeilenenden](https://git-scm.com/docs/gitattributes) sind im Checkout
  konfigurationsabhängig, solange kein `eol` gesetzt ist. Die aktuelle
  `.gitattributes` setzt kein allgemeines Rust-LF. Die Windows-Provenienz
  scheitert beim LF-abhängigen Frozen-Marker, Linux besteht denselben
  Vergleich. Die Fixgrenze muss Rohhash und Inhaltsgleichheit erhalten.
- Das [X.Org-Serverhandbuch](https://www.x.org/archive/current/doc/man/man1/Xserver.1.xhtml)
  dokumentiert `-noreset` gegen den Reset nach Schließen des letzten Clients.
  Die Primärseite wurde nach einem XHTML-Abruffehler direkt gelesen.
  Die aktuelle Suite startet Xvfb ohne dieses Argument. Die Logs belegen
  Verbindungsreset beim Aufbau, jedoch keinen behaupteten Xvfb-Absturz.
- Die [Drive-API](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/list)
  verlangt das Folgen von `nextPageToken` bis zum Ende. Die Fixture stellt
  eine feste Requestfolge bereit und antwortet bei abweichender Folge mit
  HTTP 400. Der konkrete fehlende oder falsche Aufruf ist noch zu belegen.
- Der lokale Iroh-1.3.0-Vertrag `Endpoint::network_change` meldet eine
  *potenzielle* Änderung an den Monitor. `socket.rs` unterscheidet diese
  Nachricht vom tatsächlich erkannten Wechsel und dessen Übergabe an Noq.
  Ein Aufruf ohne Netzänderung beweist daher allein keinen Epochwechsel.
- [Winsock-Abschluss](https://learn.microsoft.com/en-us/windows/win32/winsock/graceful-shutdown-linger-options-and-socket-closure-2)
  unterscheidet Protokoll-Shutdown und Freigabe des Sockets. Der konkrete
  Kontrollleser muss aus seinem blockierten Read freikommen, ohne den
  abgeschlossenen Exec erneut abzubrechen.
- Der [Linux-Qdisc-Code](https://github.com/torvalds/linux/blob/v6.17/net/sched/sch_api.c#L1501-L1624)
  zeigt bei identischem Handle den Änderungsweg; exklusives erneutes Add
  meldet genau den beobachteten Fehler. Das
  [`tc`-Handbuch](https://man7.org/linux/man-pages/man8/tc.8.html)
  beschreibt das Löschen einschließlich Kind-Qdiscs und Filter. Der eigene
  Namespace muss vor jedem frischen Profil vollständig passend sein.

## Zweiter Lückenabgleich vor Bearbeitung

Der Quellabgleich präzisiert die Erhaltungsgrenzen:

- F14/F15: `write_document` verwendet mutable JSON-Indizierung auf einem
  optionalen Feld. Dadurch entsteht bei einem Dokument ohne Recoveryfeld
  ein neues Objekt mit `records: null`; `book` verlangt eine Sequenz.
  Ein unverändertes aktives Dokument darf dieses Feld nicht erhalten.
- F01: `FolderIndex::load` filtert ausdrücklich versteckte Pfadsegmente
  und Windows-`AppData`. Der voreingestellte Temppräfix ist `.tmp`.
  Die Fixture muss ein zulässiges Indexobjekt nachweisen; die etablierten
  Filter dürfen wegen ihres Speicherorts nicht aufgehoben werden.
- F43: Windows hält den Root ohne `FILE_SHARE_DELETE` offen.
  Die Fixture verlangt dennoch erfolgreiches Umbenennen dieses Roots.
  Der Erhaltungsvertrag ist der physisch gebundene Reader einschließlich
  Abbruch; er verlangt keine Aufhebung des Windows-Pins.
- F36: `read_message_with_limits` lehnt unbesessene RAW-/ENCODED-Pakete
  inzwischen bereits vor der alten Budgetdiagnose ab. Der Schutzfall
  muss die tatsächliche Ticket-/Payloadgrenze ausüben, nicht nur einen
  neuen erwarteten Meldungstext erhalten.
- F37: Die Fixture hält genau einen Bytebesitz und nimmt an, dass damit
  der gesamte alte Plan reserviert bleibt. Ob dieser Annahme der aktuelle
  Retirementvertrag entspricht, ist vom Fixautor mit der tatsächlichen
  Livebilanz zu klären. Ein funktionierender Resize darf nicht künstlich
  verboten werden; reale Überbelegung bleibt verboten.
- F16/F45–F50: `start_file` bindet Beweispfade an das normalisierte Ziel;
  `finish` setzt später einen neu beobachteten Beweis. Jeder neue und
  wiederaufgenommene Pfad muss dieselbe Namens-/Identitätsdarstellung
  verwenden. Die direkte Pfadursache bleibt bis zur Autorenanalyse offen.
- F38: Ein möglicher LF/CRLF-Unterschied darf ausschließlich an der
  Darstellung des eingebetteten Vergleichsblocks behandelt werden;
  der Hash der unveränderlichen Gitquelle darf nicht ersetzt werden.
- F57: Ein bloßes Ersetzen der beiden Netem-Kinder reicht als Plan nicht,
  solange Portfilter des vorherigen Relays verbleiben könnten. Ein Profil
  muss Rate, Verzögerung und Filter gemeinsam frisch binden.

Damit sind die begrenzten Desktop-Ergebnisse und ihre kritischen APIfragen
für den Zuschnitt geklärt. Nicht belegte Detailursachen werden ausdrücklich
vom zuständigen Fixautor innerhalb seiner zugewiesenen Kette geklärt.
Die nachstehend ausgewerteten Androidbefunde ergänzen den Zuschnitt um M14.

## Detaillierte Desktopmeilensteine

Alle Meilensteine umfassen den vollständigen eigenen Fixzyklus. Die oben
aufgeführten Erhaltungsverträge sind ihre erwarteten Ergebnisse. Die vorhandenen
Fälle bleiben in derselben Suite; notwendige Ergänzungen werden ausschließlich
in deren Manifest registriert. Dateilisten werden vor Aktivierung vollständig
im jeweiligen begrenzten Scope festgehalten.

| Meilenstein | Befunde | Konkrete Änderungsgrenze | Abhängigkeit und Abnahmesignal |
|---|---|---|---|
| M01 Windows-Veröffentlichung | F10–F12, F16-Windows, F44, F48–F49, F51–F56 | Windows-Publikationsadapter, Kopierbeweis und deren gezielte Fälle | Unabhängig editierbar; lokale Kopie/Move/Recovery bestätigt Ziel und erhält Daten nach injiziertem Fehler. Integration mit M02. |
| M02 Transferquittungen | F16-Linux, F41, F45–F47, F50 | Recoveryquittung, Missing-only und Share-Schreibschutzklassifikation | Unabhängige Schreibdateien zu M01; Remoteordner, leere Wurzeln und Konflikte bleiben wiederaufnehmbar. |
| M03 MCP-Lebensdauer | F09, F18–F35 | Installationsbesitz, ACL-/Prozessprüfung und Codex-Dokumentgrenze | Keine Schreibüberschneidung; Lifecycle/Entfernung erhält fremde Bestände und echte Aktivitätsgrenzen. |
| M04 Appstart/CLI | F02–F08 | Journalfixture und Icon-CLI | Unabhängig; dauerhafte Idempotenz und kompletter CLI-Baum einschließlich Versionsoptionen gültig. |
| M05 Registry | F14–F15 | Optionales Eigentumsfeld und bestehende Rollbackfälle | Unabhängig; unveränderte aktive Dokumente sowie inaktive Übergänge bleiben lesbar und atomar. |
| M06 Capturebudgets | F36–F37 | Worker-Besitzweg und Windows-Resizebilanz | Unabhängig; Payload/Nativallokation bleibt unter tatsächlich reserviertem Limit. |
| M07 Netzprofile | F57 | P27-Netzwerkhelper | Unabhängig; jede geplante Profilfolge besitzt passende frische Qdiscs und Filter bei unveränderten Messkriterien. |
| M08 Optischer Bestand | F01, F43 | Indexfixture und physisch gebundener optischer Reader | Keine Windows-Adapteränderung neben M01; vollständiger zulässiger Index, Priorität und Abbruch bleiben nachgewiesen. |
| M09 Exec-Kontrollabschluss | F13 | Exec-Lebensdauer und Kontrollsocketfixture | Unabhängig; abgeschlossener Besitzer gibt Kontrollleser frei, ohne neuen Cancel. |
| M10 Drive-Seitenfolge | F17 | Drive-Scan-/Listenintegration und HTTP-Fixture | Unabhängig; Tokenfortsetzung und leere Kinder ohne falschen Fehler. |
| M11 Qualitätsprovenienz | F38 | Frozen-Blockextraktion beziehungsweise eng begrenzte Attribute | Unabhängig; Windows/Linux belegen dieselbe unveränderliche Quelle und denselben Codec. |
| M12 X11-Fixture | F39–F40 | Ausschließlich Xvfb-Lebensdauer im gemeinsamen Runner und zugehörige Capturefixture | Main behält Suiteausführung; Capture-/Resizeorakel bleiben erhalten. |
| M13 Netzwechselepoche | F42 | Iroh-/Noq-Grenze und tatsächlicher Netzwechselfall | Unabhängig; alte Tokens verlieren ihre Freigabe vor Nutzverkehr der nächsten Generation. |

M01–M14 erhalten getrennte Schreibflächen; Main integriert die gemeinsamen
Verhaltensgrenzen. Höchstens sieben Fixautoren gleichzeitig zusätzlich zu Main.
Die Modellwahl folgt erst nach dem hier abgeschlossenen Androidabgleich.


## Android: Erstbefund, Quellenabgleich und Detailmeilenstein

Der vollständig ausgewertete Android-Fehlerbestand desselben Kandidaten:

| Kennung | Beobachtung | Erhaltungsgrenze |
|---|---|---|
| A01 | `BatchesDeviceNavigationTaskTest.kt:5` importiert `assertDoesNotExist` als nicht vorhandene Top-Level-Funktion. | Die vorhandenen Member-Assertions sowie echte Activity-/Intent-/Navigationswege bleiben erhalten. |
| A02 | Sämtliche Fälle in `LoginBitmapOwnerTest` scheitern vor ihrem Prüfkörper: `SmartExplorerApp.onCreate` startet `BackgroundController`, dessen WorkManager in Robolectric nicht initialisiert ist. | Native Bitmap-/Canvasprüfung, Retain/Retire und verspätete Präsentationsnachweise bleiben erhalten; keine Änderung am produktiven Appstart für eine isolierte Besitzerfixture. |
| A03 | Zwei `BatchesMediaStateTest`-Vergleiche melden identisch dargestellte `Offset`-Werte als ungleich. | Bildfokus, begrenztes Panning und Nullversatz bei eingepasstem Bild bleiben quantitativ geprüft; Android-Systemrotation bleibt allein maßgeblich. |

Erstplan: Fixtureumgebung, tatsächliche Compose-API und geometrische Gleichheit
an ihren vorhandenen Aufrufern klären. Eine fehlschlagende Fixture belegt noch
keinen Produktdefekt. Die Instrumentation-APK fehlt, daher ist die Geräteabnahme
nicht erfolgt. Das erfolgreich gebaute App-APK ersetzt diese Abnahme nicht.

Erster Quellenpass, Abruf 2026-10-26:

- Das [Compose-BOM 2026.06.01](https://dl.google.com/dl/android/maven2/androidx/compose/compose-bom/2026.06.01/compose-bom-2026.06.01.pom)
  bindet `ui-test`, `ui-geometry` und `ui-util` an 1.11.4. Die direkt abgerufenen
  offiziellen Source-JARs bestätigen `SemanticsNodeInteraction.assertDoesNotExist`
  als Member. Die Android-Referenzseite scheiterte am Abrufgrößenlimit;
  die versionsgebundene Paketquelle ist der maßgebliche Beleg.
- [Robolectric 4.14.1 Config.java](https://github.com/robolectric/robolectric/blob/robolectric-4.14.1/annotations/src/main/java/org/robolectric/annotation/Config.java)
  und die [Konfigurationsdokumentation](https://robolectric.org/configuring/)
  bestätigen den klassenbezogenen Ersatz der Manifest-Application.
  `DriveTaskProviderTest` und `BatchesScreenStateTest` benutzen im Projekt
  bereits eine einfache `Application` für isolierte Android-Prüfungen.
- [Compose ui-geometry 1.11.4](https://dl.google.com/dl/android/maven2/androidx/compose/ui/ui-geometry/1.11.4/ui-geometry-1.11.4-sources.jar)
  definiert `Offset` als Value-Class über einem gepackten Long.
  [ui-util 1.11.4](https://dl.google.com/dl/android/maven2/androidx/compose/ui/ui-util/1.11.4/ui-util-1.11.4-sources.jar)
  packt beide `Float.toRawBits`. Die Darstellung rundet beide Komponenten.

Zweiter Lückenabgleich: `MediaGeometry.clamp` kann bei einer Achsgrenze von
null durch `-x` beziehungsweise `-y` negatives Null liefern. Damit ist ein
bitweiser Offsetvergleich für geometrische Gleichheit ungeeignet; die Aussage
über gültige Koordinaten muss bestehen bleiben. Die Loginfälle benötigen
Android-Rasterung, aber keinen WorkManager, Corestart oder Hintergrunddienst.
Der Fixautor belegt diese vollständigen Aufrufergrenzen selbst und behält die
vorhandenen Funktionsassertions bei. Kein Prüffall wird entfernt.

| Meilenstein | Befunde | Änderungsgrenze | Erwartetes Ergebnis |
|---|---|---|---|
| M14 Android-UI-Fixtures | A01–A03 | Genau die Navigations-Instrumentation sowie Loginbitmap- und Mediengeometrie-JVM-Fixtures | Bestehende Benutzerverträge werden unter passender isolierter Umgebung und korrekter API-/Zahlensemantik erreicht. Nachweis über denselben Remote-Einstieg; kein lokaler Lauf. |

## Präzisierte Agentenstartgrenze

Die Projektregel verbietet Agentenstarts während laufender Suite-, Build-,
Paket- oder Releaseprozesse. Beim Abruf 22:40:17 UTC sind sämtliche solchen
Arbeitsstufen dieses Laufs beendet. Die Android-Stufe wartet in
`review-task-native.py::run` synchron auf `android/test-android-task.sh`;
dessen Gradleaufruf verwendet `--no-daemon`. Das abgeschlossene Handoff
lieferte kein `ready=true`, der APK-Upload wurde deshalb übersprungen.
Der nachgelagerte Gerätejob benötigt ausdrücklich dieses `handoff_ready=true`
und kann für diesen fehlgeschlagenen Kandidaten nicht mehr starten.
Nur Rust-/Java-/Python-/Checkout-Nachbereitung verblieb im Jobstatus.

Daher entfällt das selbst gesetzte Warten auf das gesamte Cache-Postprocessing.
Nach Abschluss dieses Plans dürfen Fixautoren arbeiten. Das ist keine Aussage,
dass der gesamte CI-Lauf bereits abgeschlossen oder erfolgreich wäre. Der
nächste Statusabruf bleibt frühestens 23:10:17 UTC; kein zusätzlicher Lauf,
kein Abbruch und kein Release werden dadurch ausgelöst.


## Aktivierung und erste Integration

Nach abgeschlossener Planung hat die Arbeitsweise für alle M01–M14
`gpt-6.1-sol` gewählt. Die Einstufungen liegen bei 6–8; der Selektor schlug
je nach Grenze low, medium oder high vor. Die ausdrückliche Nutzervorgabe
für den vollständigen eigenen Fixzyklus setzt den tatsächlich konfigurierten
Effort auf `max`. Ausführungsweg ist das vorhandene Codex-Agentenwerkzeug
mit Credits. Die Startparameter sind belegt; Antwortmodellmetadaten und
gemessener Creditsverbrauch sind vom Host nicht verfügbar.

M14 ist durch denselben Autor analysiert, geplant, korrigiert und anschließend
statisch gegen Quellen geprüft. Main hat den vollständigen begrenzten Diff
und die Einbindung kontrolliert. Die drei Fixturekorrekturen erhalten ihre
Prüfkörper und Fallnamen; keine neue Registrierung ist nötig. Der
[Autorenbericht](fix-f-android-ui-fixtures.md) benennt alle gelesenen/geänderten
Dateien und die ausstehende Remote-Bestätigung. Kein Laufzeitpass behauptet.

Der finale API-Abruf um 23:10:44 UTC bestätigt den vollständig beendeten
Lauf mit `failure`; der Gerätejob ist `skipped`. Es läuft keine Build-,
Test- oder Releasestufe dieses Kandidaten mehr.

M05 ist als `515e8286`, M07 als `da059df3` und M02 als `4d931637` integriert
und gemeinsam mit M14 auf `main` und `codex/review-fixes-ee75e91b` gepusht.
Jeder Autor hat seinen vollständigen eigenen Fixzyklus abgegeben; Main hat
Diff, statische Syntax, Größen, Grenzen und Integration kontrolliert.
Registryfelder bleiben unverändert, fremde Netzprofile werden nicht entfernt,
absolute Backendpfade und strenge Quittungsprüfungen bleiben erhalten.
Die erforderlichen Fallregistrierungen werden für dieselbe gemeinsame Suite
gesammelt; es wurde kein lokaler Funktionslauf gestartet.

Aktiv sind M01, M03, M06, M08, M11, M12 und M13; M04, M09 und M10 bleiben
eingeplant. Der Scope wurde jeweils nur für konkret gemeldete
Aufrufer erweitert. M02 umfasst nun die falschen relativen Joins in
Ordnerreservierung und Ordner-Move. M06 umfasst den tatsächlichen
Worker-/Dispatcherleser vor unbesessenem Payload. M13 umfasst die synchrone
Entziehung alter Freigaben vor Rebind, ohne die bestehende erfolgreiche
Annahme einer neuen Generation im Routemonitor zu verlieren. M03 untersucht
den gebundenen Linux-Selbstübergang vor dem Inaktivitätsbeweis der eigenen
Deinstallation. Jeder dieser Autoren aktualisiert seinen eigenen endgültigen
Plan und Quellenabgleich vor der betroffenen Änderung. Bei M13 umfasst das
auch einen sichtbaren Wartezustand während des Netzumbaus und eine frische
Auswahlfrist nach dessen Ende, ohne absolute Operationdeadlines zu verlängern.
Bei M01 wird die geänderte Namespacebestätigung durch sämtliche direkt
angeschlossenen Sync-, Backup- und Recoveryverbraucher verfolgt; ein ehrliches
unbestätigtes Ergebnis darf nicht still unterstützte Dateisysteme aussperren.
