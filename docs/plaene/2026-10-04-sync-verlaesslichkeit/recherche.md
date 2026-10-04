# Sync-Verlässlichkeit – Recherche und Planstufen

Stand: 2026-10-04, vor Umsetzung.

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
