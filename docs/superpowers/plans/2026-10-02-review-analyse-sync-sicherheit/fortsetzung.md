# RV1 – Fortsetzung der dokumentierten Review-Fixes

Stand: 2026-10-04. Auftrag: die dokumentierten, noch offenen Befunde des abgebrochenen Reviews beheben
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

## Historische Umsetzung und Fixloop

- Abgleich abgeschlossen: vorhandene Vertragsänderungen und Teilimplementierungen, keine vollständige Abnahme.
- Umsetzung: Alle RV1-Produktblöcke und gemeinsamen Consumer-Registrierungen sind quellenfertig committed. Die begonnenen sieben Blöcke, S-POLICY, Windows-FA6, S-LOCAL, V-REMOTE, H-DISPATCH, AND-SHARE-UI, S09-LINK und AND-SYNC sind integriert. Letzte Anschlüsse: E-APPLY (`b9f9b2ae`), H-POLICY-BOUNDARY (`bdec3084`), H-REPLACE (`c785407d`), Desktop/Y156 (`b4fd5851`), E-ENGINE und Merge-/Recovery-Ownergrenzen (`4a8130a2`). Die jeweiligen `abnahme/`-Berichte dokumentieren erwartete Ergebnisse und verbleibende allgemeine Grenzen; Quellenabschluss ersetzt keine Remote-Abnahme.
- Suite: Der eine checked-in Einstieg und seine Linux-/Windows-/Android-Anschlüsse sind geschrieben. Die abschließenden Fixtures decken echte TLS-Statusrunden, verzögerte Worker-Completion, JNI/Persistenz und den Recorded-Merge nach tatsächlichem Android-Prozessende ab. Statisches Parsing ist erlaubt; lokale Ausführung bleibt ausgeschlossen.
- Remote-Abnahme: Derselbe Suiteeintritt scheiterte zuerst an Typ-/Sichtbarkeits-/Fixture-/Größendiagnosen ([37145175629](https://github.com/b1ue-man/smart-explorer/actions/runs/37145175629), `ci-fixes.md`), danach an konkret dokumentierten Verhaltens-/SDK-/Runnerfehlern ([37150409255](https://github.com/b1ue-man/smart-explorer/actions/runs/37150409255), `ci-behavior-fixes.md`). Der dritte Lauf auf `71a8ca45697272453c213c9b0b5412d0cbed0f71` ist vollständig beendet ([37157166735](https://github.com/b1ue-man/smart-explorer/actions/runs/37157166735)); APK/JNI und die wirkliche Gerätestufe wurden erreicht. `ci-closure-fixes.md` und `abnahme/CI-3-*.md` halten seine begrenzten Korrekturen fest.
- Vierter Lauf: `ac3b0c9098963fae386e94558f2f3ca1bb240740` ist vollständig beendet ([37162485159](https://github.com/b1ue-man/smart-explorer/actions/runs/37162485159)). Android-Build erfolgreich, native und Gerätestufe fehlgeschlagen. `ci-fourth-fixes.md` und `abnahme/CI-4-*.md` ordnen ausschließlich diese Fehler zu. Die Korrekturen bewahren Whole-filesystem-Flush, Deferred-Contents, Schutz-Auslassungen und die ursprünglichen Byte-/Retryassertions; unbelegte Copy-/Update-/Lost-ACK-Ursachen erhielten konkrete Statusdiagnosen.
- Quellenkorrekturen des vierten Laufs vollständig integriert: `74a2b67d` bis `c6a923ee`, einzeln zugeordnet unter `ci-fourth-fixes.md`; sämtliche Owner-Handoffs abgeschlossen. Der vollständige Graph und der gepushte Main-Kandidat `1378bc8f` gingen an denselben fünften Suiteeintritt. Keine neue Review oder lokale Ausführung.
- Fünfte tatsächliche Diagnose: Der fünfte Lauf auf dem vollständig gepushten Main-Kandidaten `1378bc8fdb796ac2102ccb1e62e8fb74fe0bd796` ist beendet ([37167542206](https://github.com/b1ue-man/smart-explorer/actions/runs/37167542206)): Android-Build erfolgreich, native/Gerät fehlgeschlagen. Die genau zugeordneten offenen Anschlüsse stehen in `ci-fifth-fixes.md`: falscher SideEmpty-Stopp bei geschütztem Link, konkrete Share-Stagezuordnung, Namespace-/Recoveryentfernung und vollständig bestätigte Mergebasis, tatsächlicher Old→New-FS-Zugriff. Der exakte Formatpatch ist in `36eaafe9`; die verdrehte SavedJob-Fixturezeit in `ef8cb666` korrigiert. E-ENGINE und S-REVOKE sind mit eigenem statischem Handoff abgeschlossen und in `c9cd629a`, `f744c63e`, `1b9eb4fe` beziehungsweise `3e1aea99` committed. Der tatsächliche Altpeer-Zugriff braucht den ausdrücklich gewählten ReadOnly-Export; die Sitzung war bereits zugelassen. A-CLIENT ist in `ef73e71f` mit Writer-ACK-gebundener Stageerstellung und vollständigem Handoff integriert. Normale Servercopy bleibt erhalten; private Stufen verwenden exklusives WriteNew-Streaming. Alle CI-5-Owner sind beendet, der vollständige Rootgraph ist erneuert. Die bestehende Suite enthält den zusätzlich zugeordneten vorhandenen Sized-Writer-Leaf. Nur derselbe Remote-Suiteeintritt bestätigt die vollständige Integration.
- Sechste tatsächliche Evidenz: Der gepushte Kandidat `2035ed266fa201195d39162dfdd7967e31a526d9` ist in [Run 37172335052](https://github.com/b1ue-man/smart-explorer/actions/runs/37172335052) vollständig beendet. Android-Build und echte Gerätestufe erfolgreich. Beide Desktop-Stufen behalten ausschließlich den tatsächlichen Share-Creator-/Freigabe-/Wurzelfehler; zusätzlich ist der exakte Remoteformatpatch in `e27bac22` übernommen. A-CLIENT hat ausschließlich diesen Anschluss gemäß `scopes/ci-6-a-client.json` und `ci-sixth-fixes.md` geschlossen: `a96d39ad` trennt die fallible Dateifähigkeitsabfrage vom Lease-Erwerb, leitet sie durch beide transparenten Wrapper weiter und ersetzt nur die drei belegten Publish-Abfragen. Owner-Handoff und statisches Parsing sind abgeschlossen, ursprüngliche Assertions unverändert. Der vollständige Rootgraph ist erneuert und enthält alle nativen Rustquellen einschließlich der engen Cachelimits-Auslagerung; der danach gepushte Kandidat `87021dcb` ging vor dem terminalen Release an denselben Suiteeintritt. Derselbe vollständige Suiteeintritt bleibt maßgeblich; keine neue Review und keine lokale Ausführung.
- Finale Gesamtabnahme: [Run 37175826251](https://github.com/b1ue-man/smart-explorer/actions/runs/37175826251) auf dem vollständig gepushten exakten Quellenkandidaten `87021dcbb532557bb09478603c8c0c0676710ea4` ist erfolgreich beendet. Beide nativen Stufen, Android-Build und tatsächliche Gerätestufe erfüllen denselben checked-in Einstieg. Der ursprüngliche reale Share-/Auto-Overwrite-/Versions-/Restore-/Local-Roundtrip-Fall ist in beiden Desktoplogs erfolgreich; der kandidatengebundene Remoteformatpatch ist leer und der Größenguard erfolgreich. Die Geräteabnahme bestätigt auch den tatsächlichen Recorded-Merge-Prozessneustart. Keine ursprüngliche Assertion wurde für den Abschluss entfernt.
- Terminaler Release: [Complete-Release 37177564587](https://github.com/b1ue-man/smart-explorer/actions/runs/37177564587) hat den stabilen Top-Level-Wrapper nach erfolgreichem Windows-/WSL-Preflight genau einmal für `v0.5.170` ausgeführt. Der Wrapper hat Version, vollständige Artefakte, Commit/Push/Tag und den einzigen [Publikationsconsumer 37183081190](https://github.com/b1ue-man/smart-explorer/actions/runs/37183081190) koordiniert; beide Workflows sind erfolgreich beendet.

## Aktueller Abschluss

Der implementierte RV1-Batch ist abgenommen und in [v0.5.170](https://github.com/b1ue-man/smart-explorer/releases/tag/v0.5.170) veröffentlicht, sichtbar seit 2026-10-04 06:35:45 UTC. Der immutable Tag zeigt auf den Artefaktcommit `398f0e7fef2e8829521e7f722ad8bf7f12adcb46`, dessen einziger Parent der abgenommene Quellenkandidat `87021dcbb532557bb09478603c8c0c0676710ea4` ist. Cargo und Feed melden `0.5.170`; der passende Installer, Windows-Buildmanifest und signierte Android-APK sind daran gebunden. Alle genau 20 erwarteten Assets sind veröffentlicht: deren GitHub-Digests und Größen stimmen mit den committed Artefakten überein, ebenso alle sechs Desktop- und der Android-Feedhash. Der signierte APK-Handoff hat SHA-256 `ca2ecaa73755692a558abc91bc8a30a9f220fcda137a99c2541be01f9d937281`.

Die im ursprünglichen Spec unter „Nicht in RV1“ ausdrücklich zurückgestellten Themen und die in den Handoffs dokumentierten allgemeinen Provider-/Host-/OS-Grenzen sind weiterhin offen. Sie werden durch diesen Abschluss nicht als gelöst erklärt. Es wurde keine neue Projekt-Review begonnen; abgeschlossen wurden die dokumentierten RV1-Meilensteine und die konkreten Fehler desselben Remote-Fixloops. Der abschließende Dokumentationscommit ändert keine nativen Quellen oder Releasebytes und löst keinen neuen Release aus.
