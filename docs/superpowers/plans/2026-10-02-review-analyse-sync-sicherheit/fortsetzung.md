# RV1 – Fortsetzung der dokumentierten Review-Fixes

Stand: 2026-10-03. Auftrag: die dokumentierten, noch offenen Befunde des abgebrochenen Reviews beheben
und anschließend den vollständigen Release durchführen. Kein neues Review und keine neue Kritiker-Runde.

## Grundlage und Abgrenzung

Stage eins bleibt die vorhandene Spec mit den drei Befunddateien; Stage zwei bleibt der detaillierte
Meilensteinplan in `umsetzung.md`. Die vorhandene Recherche und die unter `docs/refs/INDEX.md` gesicherten
APIs decken den geplanten Ansatz ab. Die einmalige Plan-Kritik ist in `review.md` abgeschlossen.
Vorhandene Änderungen werden weitergeführt und nicht verworfen. Als erledigt gilt ein Punkt erst nach
Abgleich mit seinem implementierten Verhalten; Vertrags-Stubs sind keine abgeschlossenen Fixes.

Die in der Spec unter „Nicht in RV1“ ausdrücklich zurückgestellten Befunde bleiben sichtbar und werden
am Ende gegen den aktuellen Nutzerauftrag eingeordnet; sie dürfen nicht als behoben gemeldet werden.

## Reihenfolge und Abnahme

1. Die sieben begonnenen Blöcke K1/V-LOCAL, K2/H-ANALYSIS, K3/E-PLAN, A-CLIENT, T-JOBS, S-SIGNAL und
   S-REVOKE anhand ihrer vorhandenen Verträge fertigstellen. Erwartete Ergebnisse stehen unverändert
   in `umsetzung.md` und werden je Block nach `abnahme/<block>.md` übertragen.
2. V-REMOTE, E-APPLY, H-DISPATCH, S-POLICY und S-LOCAL integrieren; anschließend Desktop-/Android-
   Bedienung schließen. Kompatibilität: gespeicherte Endpunkte und Verbindungsidentitäten, geschützte
   Link-Auslassungen, bestehende Freigaben und Klartext-Opt-ins, reversible Überschreibungen und
   wiederholbare fehlgeschlagene Aktionen erhalten.
3. Die eine bestehende Task-Suite auf alle Meilensteine und direkt betroffenen Integrationen ergänzen.
   Keine lokalen Builds/Tests und keine Zwischen-Kompilierläufe. Meilensteine einzeln committen,
   Kandidat pushen; die vollständige Suite einmal remote auslösen und ihre Befunde beheben.
4. Nach erfolgreicher Abnahme einmal `build.yml` mit `complete_release_source_sha` auf dem exakten
   Main-Kandidaten auslösen. Der konfigurierte Windows/WSL-Runner führt den stabilen Top-Level-Wrapper
   aus; dessen Preflight, Versionierung, Build, Feed-/Hash-Prüfung, Commit/Tag und Veröffentlichung bleiben
   ein zusammenhängender Release. Release-Status höchstens alle 30 Minuten abfragen.

## Übernommene Ausführungsevidenz

Der letzte vorhandene Remote-Check auf `rv1-wip` (Run `37061450335`) scheiterte an zwei konkreten
Integrationsfehlern: `WireMeta.special` fehlt in `daemon/os/shared/backend_server.rs`, und der Match in
`share/core/legacy_direct_request_mutations.rs` behandelt `DirectGrantState::Reconfirm` nicht.
Die zugehörigen Blöcke übernehmen diese Fixes. Dies ist vorhandene CI-Evidenz, kein neuer Prüflauf.
Die GitHub-REST-Zugangsdaten und die konfigurierten Workflows sind erreichbar; beim Einstieg läuft
keine GitHub-Actions-Ausführung.

## Status

- Abgleich abgeschlossen: vorhandene Vertragsänderungen und Teilimplementierungen, keine vollständige Abnahme.
- Umsetzung: Alle RV1-Produktblöcke und gemeinsamen Consumer-Registrierungen sind quellenfertig committed. Die begonnenen sieben Blöcke, S-POLICY, Windows-FA6, S-LOCAL, V-REMOTE, H-DISPATCH, AND-SHARE-UI, S09-LINK und AND-SYNC sind integriert. Letzte Anschlüsse: E-APPLY (`b9f9b2ae`), H-POLICY-BOUNDARY (`bdec3084`), H-REPLACE (`c785407d`), Desktop/Y156 (`b4fd5851`), E-ENGINE und Merge-/Recovery-Ownergrenzen (`4a8130a2`). Die jeweiligen `abnahme/`-Berichte dokumentieren erwartete Ergebnisse und verbleibende allgemeine Grenzen; Quellenabschluss ersetzt keine Remote-Abnahme.
- Suite: Der eine checked-in Einstieg und seine Linux-/Windows-/Android-Anschlüsse sind geschrieben. Die abschließenden Fixtures decken echte TLS-Statusrunden, verzögerte Worker-Completion, JNI/Persistenz und den Recorded-Merge nach tatsächlichem Android-Prozessende ab. Statisches Parsing ist erlaubt; lokale Ausführung bleibt ausgeschlossen.
- Remote-Abnahme: Der exakte Kandidat `395f912a30455ebd96799f61fdbe1fec2e1c7998` ist gepusht. Der eine [RV1-Lauf 37145175629](https://github.com/b1ue-man/smart-explorer/actions/runs/37145175629) ist beendet und fehlgeschlagen. Seine kandidatgeprüfte Formatierung ist übernommen; die konkreten Typ-/Sichtbarkeits-/Fixture- und Größendiagnosen werden gemäß `ci-fixes.md` geschlossen. Die Geräteabnahme konnte noch nicht starten. Bestätigung erfolgt ausschließlich durch denselben vollständigen Remote-Einstieg auf dem korrigierten Kandidaten.
- Release: noch nicht ausgelöst.
