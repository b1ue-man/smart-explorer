# Sync-Verlässlichkeit – Recherche und Planstufen

## Anschlussrecherche nach dem achten Remote-Lauf

Am 2026-10-05 ist `37297823833` auf `8a1a36ca` vollständig ausgewertet.
Windows, Android-Build und der tatsächliche Android-Altjobupdateablauf
bestehen. Linux bestätigt jetzt auch den vollständigen Recorded-Recovery-
und die DAV-Collectionflows. Seine Provider-Matrix erreicht FTP→FTPS und
scheitert beim echten STOR mit 426; der Server bestätigt fehlenden
SSL-Datenabschluss. C10 fehlt weiterhin der angefragte autorisierte Zugang.

**Stufe 1, aktuelle Quellen:** Die zusammenhängende Grenze liegt bei
`ftp/core/streams.rs::FtpStoreWriter::finish` und
`ftp/core/writer.rs::FtpUpload::upload`: beide lassen suppaftp beim
Finalisieren den Datenstrom schließen und anschließend die Steuerantwort
lesen. Verbindung, Timeouts, Fehlerklassen und Poolgesundheit bleiben
bestehende Verträge. Das erfolgreiche Windows-Ergebnis ersetzt keinen
Beweis dieser tatsächlichen Linux-TLS-Providerstrecke.

**Erste Primärrecherche:** RFC 4217 und TLS-Abschlussregeln sowie die exakt
gepinnten suppaftp-/rustls-Quellen bestätigen, dass TLS-Abschluss und
positive FTP-Antwort gemeinsam maßgeblich sind. Der Bibliotheks-Drop
liefert keinen falliblen vollständigen TLS-Abschluss. Stufe-1-Ansatz:
eine zusammenhängende FTP-Datenabschlusskorrektur für beide Writer,
keine Wiederholung eines mehrdeutigen STOR und keine gelockerte Fixture.

**Stufe 2 und zweite Lückenrecherche:** Öffentliche/private Crate-APIs,
`TcpStream`-Besitz und die Linux-Resetgrenze sind in
[ftp-pool.md](../../refs/ftp-pool.md#5-ftps-datenabschluss-zweite-recherche-am-2026-10-05)
mit exakter Syntax gesichert. Private Rustls-Felder sind kein verfügbarer
Appadapter; Transport-EOF ist kein TLS-Erfolgsbeweis. Der konkrete Reset
bleibt eine aus Plattformquellen abgeleitete mögliche Ursache, während
426 und der fehlende SSL-Abschluss unmittelbar belegt sind. Die finale
Abnahme bleibt der echte strenge Providerflow plus vorhandene schmale
Längen-/STOR-/Poolguards in derselben Suite. M3.C04-8 beschreibt den
vollständigen Outcome, seine Abhängigkeiten und das erwartete Signal.
Keine neue Plan-Kritik, Suite oder vorgezogene Version entsteht.

## Ursprüngliche Planung vom 2026-10-04

Die folgenden Ausgangsbefunde stammen aus der Planung vor Umsetzung;
der aktuelle Anschlussstatus steht oben und in `umsetzung.md`.

## Stufe 1: Code, Dokumentation und etablierte Verträge

Architektureinstieg: `docs/ARCHITEKTUR.md`; Graph-Abfragen zu Drive-Auflösung und
Sync/Baseline/Identity/Job/Retry. Live Ausgangspunkt ist `main` bei `3d70c5df`,
Cargo/Feed 0.5.171. Historische Pläne zur Drive-Dateivariantenwahl und zu
Remote-Literalpfaden dienen nur zusammen mit aktuellem Source als Evidenz.

Die aktuelle Drive-Namenssuche in `gdrive/core/resolution.rs` wertet einen
nicht exakt gleich geschriebenen Namen oder eine erneut gelieferte ID als
inkonsistent. `promotion_api.rs` hat denselben exakten-Namen-Vorbehalt schon vor
der Sammlung verschiedener Identitäten. Der Provider-Testserver filtert bisher
selbst case-sensitive; damit bildet er den betroffenen realen Vertrag nicht ab.

`sync_listing.rs` schützt alle gleichnamigen Ordner durch Auslassung. Im
Duplicate-Pfad fordert `snapshot_walk.rs` zusätzlich eine vollständige Liste;
dadurch bricht eine geschützte Auslassung auch unabhängige Dateien ab. Die
Dateivariantenwahl benötigt dagegen reale Namen und exakte Objekt-IDs.
Browser-Marker und Sync-Literalnamen sind bisher unterschiedlich behandelt;
der Apply-Guard vergleicht teilweise Browser-Statnamen mit Sync-Listennamen.

Alte Jobs werden von `syncjobs` aus `.conf` und alten TSVs importiert;
`daemon::job::run_one` löst dieselben gespeicherten Endpunkte auf und ruft
Bisync auf. Account-Identität, Pair-/Jobowner, Versionsorte, Replacementjournal,
Schutz vor Löschlawinen und geschützte Links sind vorhandene Verträge, keine
neuen Funktionen. Vorhandene Snapshot-/Apply-/Provider-/Pfad-/Job-Tests liefern
gezielte Bausteine für die eine finale Suite.

Stufe-1-Plan: (1) Suchergebnisse korrekt sammeln, (2) stabile, getrennte
Drive-Ordneridentitäten und Literal-Metadaten an der VFS-Grenze, (3) alle
betroffenen Sync-Consumer gemeinsam integrieren, (4) alte Jobs über die reale
Start-/Updategrenze prüfen, (5) sämtliche unterstützten Sync-Vertragsklassen in
einem kompletten Remote-Gesamtablauf abnehmen, (6) einmal vollständig releasen.

## Erste Recherche: etablierte Protokolle und Alternativen

Neue Primärrecherche ist in
[`drive-name-identity-2026-10-04.md`](../../refs/drive-name-identity-2026-10-04.md)
gesichert. Vorhandene passende Refs bleiben verbindlich: Drive/ureq,
Remote-Metadaten, lokale Identität/Durabilität, private Dateicapabilities,
Windows-Vererbung und Remote-Suite. Keine neue Runtime oder Bibliothek ist
erforderlich. Rust und die vorhandenen gepoolten HTTP-Agenten bleiben geeignet.

Die Suchabfrage ist ein Kandidatenfilter. Der Client filtert Literalnamen und
sammelt verschiedene IDs über alle Seiten. Ausgewählte Objekte werden danach
frisch und streng validiert. Das entspricht dem vergleichbaren rclone-Pfad,
ohne dessen Dateiduplikate beliebig zu verwerfen.

Ordner werden als getrennte Bäume projiziert, nicht vereinigt. Eine bereits
gültige Zuordnung wird behalten; neue Bindungen werden vor Nutzung
accountgebunden gespeichert. Zusätzliche Ordner bekommen die vorhandene
Drive-ID-Markerdarstellung. Exakte Backend-Locators und logische Sync-Namen
bekommen eine kleine optionale VFS-Grenze; andere Backends behalten ihren
Fallback. Der Explorer erhält keinen globalen Sync-Modus.

Verworfen: alle Namensabweichungen akzeptieren und den ersten Treffer nehmen
(falsche Objekte); IDs nur nach neuester mtime wählen (alte Jobs driften);
Ordner vereinigen (Daten-/Konfliktverlust); alle Duplikate auslassen (keine
funktionierende Synchronisation); Locator global dekodieren (Regression bei
Literalnamen); Fehler schlucken und Baseline schreiben (falscher Erfolg).

## Zweite Recherche: offene Integrationsfragen

1. **Root und Namespace:** `root` ist Alias, Eltern-Metadaten enthalten IDs.
   Ein zentraler lazily gecachter Aliasvergleich gilt auch für Ordneranlage,
   Stages und Rename-Prüfung. Vorhandene Tests müssen reale Root-IDs liefern.
2. **Persistenz:** die vorhandene Pfadcachedatei ist nur ein Hint. Neue stabile
   Ordnerbindungen brauchen eigenen accountgebundenen, privat/atomar
   geschriebenen Zustand; vor dem ersten Lauf wird eine gültige alte Bindung
   übernommen. Ein Cache-Clear darf die stabile Sync-Zuordnung nicht ändern.
3. **Metadaten:** ein exakt adressierbarer Locator darf nicht als wörtlicher
   Dateiname in der Baseline landen. Der optionale Sync-Stat-/Childvertrag
   muss durch Cache-/Share-Wrapper delegiert werden und in Guards, Snapshots
   und Recovery benutzt werden, soweit diese Namen vergleichen.
4. **Auslassungen:** tolerante Listings werden immer ausgewertet. Fehlende
   Inhalte eines geschützten Teilbaums bedeuten keine Löschung. Eine frische
   Einzelobjektprüfung berücksichtigt dessen eigene Auslassung, statt an
   einem anderen geschützten Kind zu scheitern.
5. **Wiederanlauf:** die vorhandene bestätigte Publication-/Journalgrenze
   bleibt maßgeblich. Verlorene Antworten veranlassen Identitätsprüfung,
   nicht einen unkontrollierten zweiten Create/Overwrite.
6. **Abnahmeinfrastruktur:** ausschließlich Remote-CI; vorhandene inkrementelle
   Development-Ausgaben und `review-task-native.py::artifact` werden genutzt.
   Libtest-Namen werden zur Laufzeit entdeckt; ein neues Szenario ohne
   passende Laufzeit-Evidenz lässt die Suite scheitern. Provider-Server und
   Alte-Version-Bytes werden selbst entdeckt/gestartet/gehasht und wieder
   beendet. Die Suite darf keine Releasebytes erzeugen.

Der tatsächliche Befehlsbaum in `cli/mod.rs` enthält keinen Sync-Befehl.
End-to-end Jobtests benutzen `run_one`/die bestehenden gemeinsamen Einstiegspfade;
ein test-only Provider-Injektor darf keine Produktions-URL-/Auth-Hintertür
einführen. Mobile JSON-Verträge werden im gemeinsamen Kern geprüft; die
Release-APK wird durch das etablierte komplette Release erstellt.

Die detaillierten Szenarien und Acceptance-Signale stehen in `umsetzung.md`.
Vor Code entsteht genau eine Plan-Kritik; deren Entscheidungen werden in
`review.md` festgehalten.

## Konkretisierte Laufzeitgrenzen vor M5

`docs/refs/sync-task-runtime-2026-10-04.md` hält die tatsächlichen Server-
Entrypoints, TLS-Vertrauensinjektion ausschließlich für cfg(test) sowie den
signierten Android-Altapp-Updatevertrag fest. Die veröffentlichte v0.5.169-APK
ist statisch gegen ihre Sidecar geprüft und enthält die benötigte x86_64-ABI.
Der spätere Ablauf muss trotzdem die alte App real starten, den Job ausführen
und ihre Daten durch ein passendes APK-Update erhalten.

Für C10 fehlen die erforderlichen Client-ID-/Refresh-Token-Testsecrets auf
GitHub Actions und eine lokale Testanmeldung. Das Client-Secret ist optional
und nur bei einem entsprechend konfigurierten OAuth-Client erforderlich. Der Nutzer wurde nach dem autorisierten Testzugang
gefragt; Umsetzung und übrige Abnahme werden unabhängig davon vorbereitet.
Das ist ein offener Runtimezugang, kein ausgeführter oder erfolgreicher Fall.

## Zweite Gapprüfung des sechsten gemeinsamen Fixloops

Aktualisiert 2026-10-05 vor Anschlussänderungen. Der vollständige sechste Lauf
belegt historische Dirkeys, DAV-Collectionredirects, den Windows-Direct-
Versionszugriff und zwei Windows-FTP-Fixture-Logins als konkrete Grenzen.
Die aktualisierten Runtime-/Providerrefs sichern den tatsächlichen Checkpoint-
Schreibvertrag, RFC 4918 §5.2 und die gepinnte ureq-2.12.1-API/Implementierung.
Der vorhandene private Versionsfallback wird zusammen mit Peer-/IPC-Scheme
und Authorization verfolgt; Share-Private-/Ownerguards bleiben maßgeblich.
Die konkreten erwarteten Ergebnisse stehen bei M2–M5 und in `abnahme.md`.
Keine neue allgemeine Reviewrunde oder zusätzliche Suite. C10 benötigt zwei
erforderliche OAuth-Eingänge; das Client-Secret hängt vom verwendeten Client ab.

## Siebter Fixloop: zweistufige Anschlussplanung

Am 2026-10-05 ist Lauf `37289323834` auf `ffba5c54` einschließlich aller
vier Stufen ausgewertet. Stufe 1 lokalisiert die konkreten Restfehler anhand
der echten Serverlogs, Hostberichte und aktuellen Source: wiederholtes DAV-
MKCOL ohne Slash, leere ZIP-Scanwurzel der Fixture und eine rohe Provider-
Fehlerart als unzutreffende Erwartung am öffentlichen Recorded-API.
Die primäre Protokollrecherche prüft RFC 4918 §5.2/§9.3 und Apache
DirectorySlash; die Syntax und aktuelle API stehen in der Provider-Ref.

Stufe 2 konkretisiert M3.C04-7 und M2.C05-7 in `umsetzung.md`. Die zweite
Gapprüfung verfolgt ZIP durch `Backend::is_local`, Namespaceidentität und
`validate_sync_roots`: die bestehende logische Archivwurzel `/` wird nicht
als natives Root behandelt. Recorded-Apply rekonstruiert den konkreten
Fehlertext als `Other`; Injektion, Slotbytes, Intent, Baseline und Replay
bleiben eigenständige strikte Beweise. Für MKCOL werden belegte Dateinamen,
fehlende Parents, Rechte, Transport, Pooling, Redirects und literal kodierte
URLs ausdrücklich erhalten. Nur eine nachgewiesene vorhandene Collection
erlaubt idempotenten Erfolg. Kein Serverworkaround und kein Fehler-Skip.

Der bereits einmal vollständig kritisierte Plan bleibt derselbe. Es gibt
keine neue Reviewrunde, zweite Suite oder Zwischenveröffentlichung. C10
bleibt wegen fehlender tatsächlicher Google-OAuth-Eingänge offen.
