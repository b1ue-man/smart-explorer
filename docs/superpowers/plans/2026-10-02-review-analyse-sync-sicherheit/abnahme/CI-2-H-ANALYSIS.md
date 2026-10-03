# CI-2-H-ANALYSIS – Recycle-Root und leerer Walk-Cancel

Stand: 2026-10-03. Ausschließlich die zwei zugewiesenen Verhaltensbefunde aus
RV1-Run 37150409255, Kandidat ac9b475ff18f6320bedd408c5a03c091710ad01c.
Scope: `../scopes/ci-2-h-analysis.json`. Kein neuer Projektreview oder Testlauf.

## Eigener Plan und zweite konkrete API-Klärung

`with_regular_child` kanonisiert die autorisierte Root, normalisiert aber den
Candidate separat. Windows kann dabei die Root-Schreibweise ändern, insbesondere
bei temporären Kurzpfaden. Der vorhandene OS-Normalisierer behandelt Verbatim-/
UNC-Formen und Literalnamen. Beide nachgewiesenen Root-Schreibweisen werden für
denselben komponentenweisen Child-Bezug benutzt; geöffnet wird weiterhin die
kanonische Root und jeder Child über DirectoryHandle. Kein Child-Kanonisieren,
Case-Folding, String-Präfix oder freier Pfad-Fallback.

Die Walk-Fixture ruft `scan_reclaim_backend` auf. Dessen post-order Reclaim-Modus
nutzt absichtlich keinen Hashwalk; der passende öffentliche Consumer ist
`find_backend_duplicates`. Cancel wird nach einem tatsächlichen Walk-Startsignal
gesetzt. Der Consumer hält den Sender bis zum Abschluss des echten Hashwalk-
Aufrufs, damit ein früh geschlossener Eintragsstream die Cancel-Pumpe nicht beendet.

| Kohäsiver Schritt | Erwartung derselben vollständigen RV1-Suite |
| --- | --- |
| Rootformate auf derselben autorisierten Root binden | Changed-Content erreicht die Verifikation, bleibt unverändert; Publikation/Restore behalten Handle-/Link-/NoReplace-Grenzen. |
| Leeren aktiven Hashwalk abbrechen | Cancel erreicht den laufenden Backendaufruf auch ohne Einträge und nach frühem Sender-Drop; kein Listing-/Downloadrückfall. |
| Bestehende Fixture am echten Consumer ausrichten | Start bestätigt vor Cancel; bisherige Cancel-/Frist-/Root-/Listing-Assertions bleiben unverändert und kein Download wird zusätzlich bestätigt. |
| Eigener statischer Abschluss | Größen-/Parsing-/Assertionserhalt und exaktes Inventar; keine lokale Ausführung. |

V-LOCAL besitzt den parallelen Windows-Handle-/DACL-/Quarantäneanschluss.
Hier werden weder dessen Implementierung noch Sharing-/Watch-/Elevationrechte geändert.

## Abschluss

Die beiden Quellenanschlüsse sind umgesetzt. `with_regular_child` normalisiert
die ursprüngliche und die kanonisierte Schreibweise derselben autorisierten Root.
Der komponentenweise relative Pfad wird ausschließlich aus diesen Formen
abgeleitet; der Veröffentlichungs-/Intentpfad entsteht aus kanonischer Root plus
Child-Komponenten. DirectoryHandle öffnet weiterhin die Root, jeden Unterordner
und das reguläre Child. SHA-/Längenprüfung vor und nach Capture, NoReplace,
Restore und Fehlermeldung mit aufbewahrtem Ort bleiben unverändert.

`scan_backend_hash_walk` hält einen eigenen Eintrags-Sender bis zum Ende des
Backendaufrufs. Die bestehende Cancel-Pumpe erreicht deshalb einen aktiven Walk
auch nach dem frühen Schließen seines Eintrags-Senders. Cancel und Budget-Stopp
behalten das Teilresultat ohne Listing-/Downloadrückfall; echte Gesamtfehler und
fehlende Unterstützung behalten den bisherigen Fallback samt Zählerrücksetzung.

Die vorhandene Cancel-Fixture verwendet jetzt `find_backend_duplicates` statt des
post-order Reclaim-Consumers. Ihr Backend schließt den Eintrags-Sender, bestätigt
den tatsächlichen Walk-Start und wartet anschließend auf Cancel. Der Canceller
wartet auf dieses Startsignal. Alle bisherigen Assertions bleiben wortgleich:
Backend-Cancel erkannt, Abschluss unter fünf Sekunden, kein Rootfehler und null
Listings. Additiv wird `open_reads == 0` geprüft. Keine Fixture entfernt oder neu
angelegt; ausschließlich diese bestehende Testfunktion wurde geändert.

Keine produktive API oder Registrierung wurde geändert. Die bestehenden APIs
`with_regular_child`, `scan_backend_hash_walk` und `find_backend_duplicates`
werden mit unveränderten Signaturen konsumiert.

## Statischer Self-Review und erwartete Abnahme

Eigener Textvergleich gegen die unmittelbar vor der Änderung gesicherten Quellen:
Recycle-Guard und Verifikation außerhalb der Rootauswahl sind identisch; Cancel-
Schleife, Zähler und Outcome-/Fallback-Entscheidung außerhalb des Workerblocks
sind identisch. Alle alten Assertion-Makros und Testnamen bleiben erhalten.
Kommentar-/String-bereinigte Klammerprüfung ist ausgeglichen; keine nachgestellten
Leerzeichen. Dies ist keine Rust-Kompilierung oder Laufzeitabnahme.

| Geänderte Rust-Datei | Zeilen | Bytes |
| --- | ---: | ---: |
| `native/src/analytics/os/shared/checked_recycle.rs` | 163 | 6197 |
| `native/src/analytics/os/shared/reclaim/backend_agent.rs` | 153 | 6242 |
| `native/src/analytics/os/shared/reclaim/backend_tests.rs` | 483 | 15759 |

Abnahmesymbole in derselben Root-verantworteten vollständigen Remote-RV1-Suite:

- `analytics::os::checked_recycle::tests::review_task_recycle_changed_content_is_not_captured`: erreicht die Inhaltsprüfung; geänderte Datei bleibt vorhanden und wird nicht veröffentlicht.
- `analytics::os::checked_recycle::tests::review_task_recycle_publication_failure_restores_without_replacing`: Publikationsfehler wird gemeldet, Originalinhalt wird ohne Ersetzung wiederhergestellt; Fixture unverändert.
- `analytics::reclaim::backend_tests::review_task_agent_walk_cancel_arrives_without_entries`: nach bestätigtem Start und geschlossenem Eintrags-Sender wird Backend-Cancel innerhalb der bisherigen Frist erkannt; kein Listing und kein Download.

Keine lokale Ausführung, kein Formatter, Git, CI, Graph oder Release. Der
Laufzeitnachweis bleibt ausschließlich der bereits bestehenden Remote-Suite.

## Exaktes Dateiinventar

Gelesen (einschließlich eigener Scope-/Abnahmedatei):

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-2-h-analysis.json`
- `/tmp/rv1-ci-second/h-analysis.json`
- `docs/refs/local-fs-identity-durability.md`
- `docs/refs/rv1-remote-suite.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-behavior-fixes.md`
- `native/src/analytics/mod.rs`
- `native/src/analytics/os/shared/checked_recycle.rs`
- `native/src/analytics/os/shared/reclaim/backend.rs`
- `native/src/analytics/os/shared/reclaim/backend_agent.rs`
- `native/src/analytics/os/shared/reclaim/backend_tests.rs`
- `native/src/analytics/os/shared/reclaim/mod.rs`
- `native/src/analytics/os/shared/reclaim/stage.rs`
- `native/src/analytics/os/shared/reclaim/types.rs`
- `native/src/local_access/mod.rs`
- `native/src/local_access/os/windows/mod.rs`
- `native/src/local_access/os/windows/paths.rs`
- `native/src/vfs/core/core.rs`
- `native/src/vfs/core/extension_calls.rs`
- `native/src/vfs/core/extension_types.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-2-H-ANALYSIS.md`

Geändert:

- `native/src/analytics/os/shared/checked_recycle.rs`
- `native/src/analytics/os/shared/reclaim/backend_agent.rs`
- `native/src/analytics/os/shared/reclaim/backend_tests.rs`

Erstellt:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-2-H-ANALYSIS.md`

## Fremdgrenzen

Keine neue API-/Scope-Anfrage. Die physische Windows-Handle-/DACL-/Quarantänebasis
bleibt beim parallelen V-LOCAL-Block und wird von Root gemeinsam integriert;
`watch_path(None)` und vorhandene Sharing-/Elevationrechte wurden hier nicht
angefasst. Eine erfolgreiche Windows-Laufzeitabnahme wird erst nach diesem
gemeinsamen Anschluss durch die bestehende Remote-Suite behauptet.
