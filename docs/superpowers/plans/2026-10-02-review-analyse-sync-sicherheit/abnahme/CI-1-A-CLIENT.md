# CI-1-A-CLIENT – konkrete Typanschlüsse des RV1-Laufs

Stand: 2026-10-03. Auftrag ausschließlich aus
[Run 37145175629](https://github.com/b1ue-man/smart-explorer/actions/runs/37145175629)
und [ci-1-a-client.json](../scopes/ci-1-a-client.json). Kein neuer Review,
keine lokale Ausführung. Der Root besitzt dieselbe vollständige Remote-RV1-Suite.

## Beleg und enges Vorgehen

Die übernommene Diagnosenliste `/tmp/rv1-ci-first/a-client.json` nennt acht
unterschiedliche fehlerhafte Anschlüsse in sechs Änderungsdateien; mehrere
Linux-/Windows-Builds melden dieselben Stellen. Der Remote-Formatterpatch war
vor Arbeitsbeginn bereits angewandt. Der vorhandene Korrekturplan steht in
[ci-fixes.md](../ci-fixes.md).

Der API-Abgleich verwendet den aktuellen `Backend`-Reexport, die tatsächlichen
IPC-Request-/Hostaufrufe, `Progress::{node_budget,set_node_budget}` und die vom
Compiler genannten Speicher-/Texttypen. Erst danach wurden die sechs Stellen
zusammenhängend geändert. Erwartet wird im selben Remote-Einstieg: kompilierbare
Trait-/Parameter-/Fehlertypanschlüsse sowie erhaltene Budget-, Identitäts-,
Abbruch-, Vollständigkeits- und Retentionsassertions.

## Umsetzung und Entscheidungen

| Datei / Symbol | Diagnose / konkrete Änderung | Erhaltene Grenze |
|---|---|---|
| `native/src/agent/core/extensions.rs`, `AgentBackend::list_dir_tolerant` | E0599: `Backend` über die bestehende VFS-Fassade importiert. | Der alte vollständige Listing-Rückfall ruft weiterhin den Agent-Backendtrait auf; kein Umweg zur inneren SFTP-Verbindung. |
| `native/src/daemon/os/shared/windows_analysis_task_tests.rs`, `bridge` | E0061: `node_budget` aus dem tatsächlichen `AnalyzeShare`-Request entnommen und als dritter Parameter unverändert an `ipc_analysis::serve` weitergegeben. | Tokenprüfung und konkrete Direct-Principal-/Contact-ID-Assertion bleiben vor dem Hostaufruf; Cancellation- und Workerpfad bleiben erhalten. |
| Dieselbe Fixture, `windows_remote_task_analysis_ipc_cancellation_reaches_active_worker` | E0063: vollständiger Request mit `node_budget: Some(progress.node_budget())`. | Kein Ersatz durch `None`; vorhandene Token-/PermissionDenied-Assertions bleiben vollständig. |
| `native/src/daemon/os/shared/ipc_host.rs`, `load_share_server` | E0277: Migrations-`io::Error` mit derselben vorhandenen `error.to_string()`-Strategie in den Host-`String`-Fehlervertrag überführt. | Fehlgeschlagene Migration propagiert weiterhin; kein leerer Server-/Erfolgsfallback und keine Transport-/Persistenzänderung. |
| `native/src/mobile/os/shared/domains/analyze_results.rs`, `tree_bytes` | E0599: `SizeNode.name: Box<str>` über `len()` gezählt. | Ein `Box<str>` hat genau die Textlänge als Speicherfläche; Node-/Allocatorreserve, ungenutzte Kinderkapazität und iterative Traversierung bleiben erhalten. |
| `native/src/share/core/peer_list_batch.rs`, `list` | E0308: retained-memory-Zähler und alle Summanden vor Multiplikation/Addition auf `u64`, saturierend. | Dieselbe Budgetgrenze und OutOfMemory-Meldung; Namensvalidierung, Sortierung, Duplikate, Auslassungen und Endsummen bleiben erhalten. |
| `native/src/share/core/peer_duplicates.rs`, `receive` | Beide E0308: Ergebnis-/Diagnose-/Protected-Speicherkosten gemeinsam als saturierende `u64`-Summen. | Kein verengender `u64`→`usize`-Cast, kein `unwrap`-Konvertierungsweg; derselbe Budgetvergleich, SHA-/Root-/Gruppen-/Summenprüfungen, Reattach und Cancellation bleiben erhalten. |

Keine öffentliche API, Protokollform, Featureverhandlung, Budgethöhe, Principal-
oder Grantentscheidung geändert. Die Speicherformeln behalten ihre bisherigen
Faktoren und Strukturkosten; die Addition kann keine kleinere Breite mehr
überlaufen, bevor sie im Budgetzähler ankommt. Alle sechs Rustdateien bleiben
unter 500 Zeilen und 50 KiB; keine neue Quelldatei oder Registrierung nötig.

## Konkrete Abnahmesignale derselben Remote-Suite

Die betroffenen Linux-/Windows-Library- und `se-dev`-Builds sowie Android-JNI
müssen die zugeordneten E0599/E0061/E0063/E0277/E0308-Stellen akzeptieren. Diese
Quellkorrektur ist kein Kompilierungs- oder Laufzeitnachweis.

Bestehende ausgewählte Symbole und Assertions bleiben unverändert:

- `windows_remote_task_analysis_matches_local_through_gui_worker_and_cache`:
  identischer Baum, echte Datei-/Verzeichnis-/Bytezähler, Hostdauer,
  Fortschrittsdrosselung und null einzelne Metadatenaufrufe.
- `windows_remote_task_analysis_ipc_cancellation_reaches_active_worker`:
  echter Worker endet innerhalb der bestehenden Frist, Ergebnis `Canceled`,
  gleicher Direct-Principal und Token-/PermissionDenied-Grenze.
- `review_task_results_tree_bytes_count_every_node`: jede Node- und
  Box-Textlänge wird gezählt, vorhandene Gleichheitsassertion bleibt gleich.
- `review_task_host_app_figures_participate_in_result_retention`:
  Host-App-/Plattformkosten bleiben Teil der Retention und verdrängen den
  ältesten fertigen Slot.
- `review_task_results_trim_oldest_finished_within_budget` und
  `review_task_results_pending_slot_frees_without_result`: laufende/neuste
  Slots bleiben geschützt, Fehler-/Cancel-Slots werden freigegeben.

Peer-Empfänger behalten ihre bestehenden OutOfMemory-/InvalidData-Pfade;
dieser Auftrag fügt keine unabhängigen Tests oder neue Suiteeintritte hinzu.

## Gelesene Dateien und fehlende gespeicherte Read-Pfade

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-1-a-client.json`.
- `/tmp/rv1-ci-first/a-client.json`.
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-fixes.md`.
- `docs/refs/rv1-remote-suite.md`.
- `docs/refs/share-server-tls-auth.md`.
- `docs/refs/local-fs-identity-durability.md` (gezielte Referenzsuche).
- `native/src/agent/core/extensions.rs`.
- `native/src/agent/core/backend.rs`.
- `native/src/agent/mod.rs` (Trait-/Fassadenabgleich).
- `native/src/agent_proto/mod.rs`.
- `native/src/analytics/core/progress.rs` (Budgetsignaturen).
- `native/src/analytics/mod.rs`.
- `native/src/daemon/mod.rs` (gezielte Symbolsuche).
- `native/src/daemon/os/shared/ipc.rs` (Request→Host-Aufruf).
- `native/src/daemon/os/shared/ipc_analysis.rs` (Request, Empfänger,
  `serve`-Signatur, Budgetsetzen und Cancellation).
- `native/src/daemon/os/shared/ipc_client.rs` (Backend-Analyseanschluss).
- `native/src/daemon/os/shared/ipc_host.rs`.
- `native/src/daemon/os/shared/windows_analysis_task_tests.rs`.
- `native/src/mobile/os/shared/domains/analyze_results.rs`.
- `native/src/mobile/os/shared/runtime.rs` (gezielte Symbolsuche).
- `native/src/share/core/peer_duplicates.rs`.
- `native/src/share/core/peer_list_batch.rs`.
- `native/src/share/core/peer_request.rs` (gezielter Request-/Leaseabgleich).
- `native/src/transfer/mod.rs` (`memory_budget`-Fassade).
- `native/src/vfs/mod.rs` (`Backend`-Fassade).
- Dieser neu erstellte Abnahmebericht für den Self-Review.

Die gespeicherten Read-Pfade `native/src/analytics/core/types.rs`,
`native/src/agent_proto/core/analysis_transfer.rs`,
`native/src/share/core/peer_core.rs` und
`native/src/daemon/os/shared/ipc_server.rs` existieren im aktuellen Worktree
nicht. Sie wurden lediglich als erlaubte Pfade angefragt, ohne Folgeexploration.
Keine Alternativdatei geöffnet. Die aktuelle erlaubte Fassade, der tatsächliche
IPC-Produzent und die konkreten Compiler-Typmeldungen genügen für diese Fixes;
eine zusätzliche Lesefreigabe ist für den abgeschlossenen Block nicht nötig.

## Geändert / erstellt

Geändert ausschließlich:

- `native/src/agent/core/extensions.rs`.
- `native/src/daemon/os/shared/windows_analysis_task_tests.rs`.
- `native/src/daemon/os/shared/ipc_host.rs`.
- `native/src/mobile/os/shared/domains/analyze_results.rs`.
- `native/src/share/core/peer_list_batch.rs`.
- `native/src/share/core/peer_duplicates.rs`.

Erstellt ausschließlich
`docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-1-A-CLIENT.md`.
Keine bestehenden Docs oder fremden Quellen geändert.

## Eigener statischer Self-Review / offene Fremdgrenze

Der Self-Review prüft die sechs Deltas gegen den gespeicherten Ausgangstext,
die konkrete Diagnosezuordnung, identische bestehende Assertiontexte,
Parameterrichtung, breite saturierende Kosten und unveränderte Fehler-/Abbruch-
und Principalgrenzen. Der Textvergleich bestätigt identische bestehende
Assertiontexte in allen Änderungsdateien. Statische Rust-Lexik-/Delimiter-,
Whitespace-/EOF- und Scope-/Größenprüfungen sind abgeschlossen und sauber.
Sie rufen weder Compiler noch Formatter auf und ersetzen keinen Typcheck.

Aktuelle Größen: `extensions.rs` 487 Zeilen / 17.362 Bytes,
`windows_analysis_task_tests.rs` 182 / 10.326,
`ipc_host.rs` 479 / 18.673, `analyze_results.rs` 394 / 13.250,
`peer_list_batch.rs` 97 / 4.287, `peer_duplicates.rs` 197 / 8.168.

Offen bleibt ausschließlich die tatsächliche Compilation/JNI- und
Verhaltensbestätigung im Root-eigenen selben Remote-RV1-Einstieg. Keine lokale
Suite, Compiler, Formatter, Git-, CI-, Graph-, Release- oder Agentoperation
ausgeführt. Keine neue fachliche Scope-Anfrage oder unbehobene zugewiesene
Quellstelle zurückgelassen; alle acht Diagnoseanschlüsse haben eine konkrete
Korrektur im erlaubten Scope. Der Remote-Nachweis steht aus.
