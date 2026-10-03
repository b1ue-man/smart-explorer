# H-REPLACE – reversible Ersetzung über Share

Stand: 2026-10-03. Der eng freigegebene Source-Anschluss für Y140/Y142 ist abgeschlossen. Die gemeinsame Remote-Abnahme steht aus; Engine-Intent/-Recovery bleibt beim Hauptagenten/E-ENGINE. Es wurde kein neuer Projekt-Review eröffnet.

## Umsetzung und Fundzuordnung

| Abschnitt | Vorhandene Grenze/Fund | Implementiertes Verhalten und Abnahmesignal |
|---|---|---|
| R1 | Y140/Y142, VFS-Defaultfalse | `reversible_replace_v1` ist eine additive ausgehandelte Fähigkeit. Request `ReplaceStagedReversible` ist schreibend; Antwort `ReversibleReplaced` enthält ein verpflichtendes explizites Boolean. Alt-/No-Feature-Host liefert bei gültigem Vertrag `false` vor Mutation. |
| R2 | PeerBackend-Defaultfalse trotz realem Provider-Hook | Peer-Erweiterung verwendet denselben Backend-/Lease-Kontext, prüft eigene Stage und gleiche virtuelle Parent-Komponenten. Der neue Aufruf sendet den Mutationsframe genau einmal. Zusätzlich erlauben bestehende Kontroll-Attempts dieser neuen Variante keinen Idle-Zusatzretry. |
| R3 | Autorisierte Host-/Providergrenze | Stage, Ziel und Retained werden durch dieselbe aktuelle FsAccess jeweils schreibend aufgelöst; `require_same_backend` prüft beide Paarbindungen. Das bestehende guarded VFS-Provider-Hook erhält die aufgelösten Pfade. Lease-Rezulassung, Fairness und Worker-Wake bleiben am vorhandenen Hostweg. |
| R4 | Unbestätigte Veröffentlichung | Nur eine vollständig erhaltene `replaced: true`-Antwort ruft `release_stage`. `false`, Providerfehler, malformed Antwort, Idle-Close und Antwortverlust lösen weder Stage-Freigabe noch Cleanup, Replay oder Overwrite-Rückfall aus. Der Provider behält das Original gemäß seinem vorhandenen Retained-Vertrag. |
| R5 | Source-Größengrenze | Die vorhandenen `simple/answer/reply_unit/control`-Helfer wurden in die freigegebene `server_fs_control.rs` verschoben; `is_retryable_read/response_matches` unverändert in `peer_request_policy.rs`. Admission-/Antwort-/Retryverhalten bleibt gleich; Dispatcher und Peer-Request-Datei haben Platz für normale Formatierung unter der Dateigrenze. |

Der Anschluss vermittelt den vorhandenen reversiblen Providervertrag. Er implementiert keine neue FTP-/SFTP-Ersetzungsstrategie und behauptet keine atomare Namespace-Ersetzung. Ein Provider ohne Hook bzw. mit mutationsfreiem `false` bleibt unterstützt. Die Engine entscheidet ausschließlich nach ihrem persistierten Intent, wie mit `false`, bestätigtem Erfolg oder unbestätigtem Fehler weitergearbeitet wird.

## Entscheidungen und erhaltenes Verhalten

Die drei Pfade müssen verschieden sein und dieselben virtuellen Parent-Komponenten haben; der Recovery-Sibling heißt exakt `.se-replace-<16lowerhex>` ohne Dateibasename. Stage muss ein vorhandenes Stage-Namensformat haben und beim aufrufenden PeerBackend als eigene Stage registriert sein. Gespeicherte Providerpfade werden weder dekodiert noch umgeschrieben. Kodierte Drive-Parents und Literalnamen bleiben erhalten.

PeerBackend hat als Displayroot den virtuellen Share-Namensraum `/`. Die reale Export-/Verbindungswurzel wird am Host durch FsAccess und gegebenenfalls die bestehende Mount-Lease bestimmt. Es gibt keine Client-Vermutung über einen lokalen Root oder einen anderen Peer. Host-Prüfungen passieren pro Pfad und unmittelbar vor dem bestehenden Guard-Hook; nach dem Worker und vor der Antwort werden aktuelle Rechte erneut geprüft.

Der neue Client nutzt `request_once` direkt und damit keine Attempts-Schleife. Für die neue Wire-Variante allein ist zusätzlich der Idle-Zusatzretry in beiden allgemeinen Kontrollpfaden deaktiviert. Alle bisherigen Lese-/Mutation-Retryverträge anderer Requestarten bleiben erhalten. Die Antwort wird strukturell der Anfrage zugeordnet und wie bisher als Fehler/Ergebnis dekodiert.

Eine unbestätigte Veröffentlichung kann physisch bereits erfolgt sein. Daher bleibt die Client-Stage-Registrierung bei Fehler/Antwortverlust bestehen; vorbereiteter Inhalt kann schon am Ziel und das Original am Retained-Pfad liegen. Der Anschluss entfernt keines dieser Objekte und interpretiert Antwortverlust nicht als `false`. E-APPLY setzt inzwischen den bestehenden Cleanupmarker vor einem begonnenen Publish-Versuch und verhindert damit automatisches Stage-Drop-Cleanup nach dessen Fehler. E-ENGINE übernimmt den dauerhaften Intent für diese drei bekannten Pfade; der Cleanupmarker bestätigt keinen erfolgreichen Publish oder eine neue Baseline.

## Exakte Selektoren für die eine Remote-Suite

Die Source-Fixtures sind unter `peer_extensions::reversible_replace::task_tests` registriert. Sie wurden lokal weder kompiliert noch ausgeführt.

| Selektor | Konkretes Signal |
|---|---|
| `review_task_h_replace_no_feature_is_mutation_free` | Kein Mutationsaufruf und keine Stage-Freigabe; altes leeres Featureobjekt meldet `false`. |
| `review_task_h_replace_idle_close_and_lost_ack_never_replay_or_release` | Ein Callbackversuch für Idle-/EOF-Fehler; nach simuliert bereits erfolgter Veröffentlichung bleiben Original am Retained-Pfad und vorbereiteter Inhalt am Ziel erhalten; keine Stage-Freigabe. |
| `review_task_h_replace_only_confirmed_true_releases_own_stage` | Ausschließlich bestätigtes `true` gibt frei; `false`, fremde Stage und falscher Antworttyp nicht. |
| `review_task_h_replace_readonly_host_preserves_all_objects` | Der reale FsAccess-Aufruf auf Nur-lesen-Export meldet `ReadOnlyFilesystem`; Stage/Original unverändert, Retained nicht angelegt. |
| `review_task_h_replace_retained_contract_keeps_literal_parent_and_nonce` | Exakter Nonce-/Sibling-Vertrag; fremder Parent, basenamehaltiger/kurzer/großgeschriebener Nonce und identische Stage/Ziel werden verworfen; kodierter Drive-Parent bleibt gültig. |
| `review_task_h_replace_wire_classifies_mutation_and_requires_explicit_boolean` | Request ist Mutation, JSON-Tags stimmen, fehlendes Antwort-Boolean ist kein implizites `false`. |

Zusätzlich muss die gemeinsame Remote-Suite die tatsächliche Transportschicht und bestehenden E-ENGINE-Consumer abdecken:

| Integration | Erwartetes Ergebnis |
|---|---|
| Direct und Room zu exportiertem SFTP mit realem NoReplace-Hook | Vorbereitetes Ziel wird veröffentlicht, Original bleibt am vorab journaled Retained-Sibling; genau ein Mutationsframe, bestätigtes `true` gibt Clientstage frei. |
| Alter Host ohne Feature und neuer Host/Provider mit `false` | Kein Reversible-Mutationsframe beim alten Host; Provider-`false` verändert Stage/Ziel/Retained nicht. Kein automatischer Legacy-Overwrite-Rückfall. |
| Beziehung ohne Schreibrecht, Nur-lesen-Export, widerrufene oder fremde Lease | Ablehnung vor Provider-Mutation; keine Datenänderung. Capabilities für andere Funktionen bleiben erhalten. |
| Drei Pfade über unterschiedliche Exporte/Verbindungen oder unzulässige Retained-Namen | Ablehnung; kein Cross-Root-/Cross-Connection-Provideraufruf. |
| Idle-Close bzw. Antwortverlust nach möglicherweise erfolgtem Provider-Publish | Keine erneute Mutation, keine Stage-/Retained-Löschung; Engine behält Intent/Recovery-Evidenz und protokolliert keinen bestätigten Erfolg. |
| Fehler im Provider zwischen Capture und Publish | Das bestehende Provider-NoReplace-/Recoveryverhalten bleibt erhalten; Original wird nicht gelöscht. Fehlende Bestätigung wird nicht in Baseline/Intent als erfolgreiche Aktion geschrieben. |

## Statische Evidenz

Scope-/Modul-/Zeilen-/Bytegrenzen, Dateiinventare, Selektoren und lexikalische Rust-Delimiter wurden mit kurzen Text-/Parsing-Prüfungen ohne Befund abgeglichen. Die konkreten Request-/Response-/Effekt-/Retry-Arme und der einzige `request_once`-Aufruf sind statisch angeschlossen. `git diff --check` für eigene bestehende Dateien ist ohne Meldung. Die extrahierten Kontrollhelfer und Wire-Policy-Funktionen stimmen nach Abzug erforderlicher Sichtbarkeit und Imports mit ihrem vorherigen Inhalt überein.

Neue Module sind von Hand normal gegliedert und liegen deutlich unter 500 Zeilen/50 KiB; `peer_request.rs` liegt nach der kohäsiven Extraktion bei 443 Zeilen, sein neues Policy-Modul bei 58 Zeilen. Es wurde kein Formatter aufgerufen. Die Remote-Suite liefert erst den Compiler-/Laufzeitnachweis.

## Dateien gelesen

In diesem Anschluss wurden diese Dateien bzw. gezielte Definitionsstellen gelesen; eigene neue Quellen und Berichte werden beim Self-Review mitgelesen:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/h-replace.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/umsetzung.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/integration.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-sync.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/V-REMOTE.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/E-APPLY.md`
- `docs/refs/local-fs-identity-durability.md`
- `docs/refs/sync-remote-metadata.md`
- `native/src/vfs/core/core.rs`
- `native/src/vfs/core/extensions.rs`
- `native/src/vfs/core/extension_calls.rs`
- `native/src/vfs/core/staging_names.rs`
- `native/src/share/core/peer_extensions.rs`
- `native/src/share/core/peer_stream.rs`
- `native/src/share/core/peer_transfer.rs`
- `native/src/share/core/peer_request.rs`
- `native/src/share/core/peer_fs_logging.rs`
- `native/src/share/core/fs_request.rs`
- `native/src/share/core/fs_response.rs`
- `native/src/share/core/fs_access.rs`
- `native/src/share/core/fs.rs`
- `native/src/share/core/fs_guard_extensions.rs`
- `native/src/share/core/fs_guard_backend.rs`
- `native/src/share/core/fs_paths.rs`
- `native/src/share/core/server_fs.rs`
- `native/src/share/core/mount_lease.rs`
- `native/src/share/core/wire_capabilities.rs`
- `native/src/share/core/peer_literal_paths.rs`
- `native/src/share/mod.rs`
- `native/src/share/core/backend.rs`
- `native/src/sftp/core/reversible_replace.rs`
- `native/src/bisync/os/shared/apply_stage.rs`
- `native/src/share/core/peer_reversible_replace.rs`
- `native/src/share/core/fs_reversible_replace.rs`
- `native/src/share/core/reversible_replace_task_tests.rs`
- `native/src/share/core/server_fs_control.rs`
- `native/src/share/core/peer_request_policy.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/H-REPLACE.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/H-REPLACE.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/H-REPLACE.md`

Bereits gelesene und wiederverwendete Arbeitsvorgaben: `AGENTS.md`, `docs/ARCHITEKTUR.md`, `/root/.codex/skills/arbeitsweise/SKILL.md`, `/root/.codex/skills/graphify/SKILL.md` und der abgeschlossene H-DISPATCH-Vertrag. Kein neuer Graph-/Reviewlauf.

Beim freigegebenen Backend-Definitionsread wurde einmal ein zu großes Fenster (Zeilen 1–158) ausgegeben, das auch benachbarten Exec-/Probe-Text zeigte. Das wurde dem Hauptagenten gemeldet; diese zusätzlichen Abschnitte wurden weder geändert noch weiter untersucht. Alle weiteren Reads blieben bei den freigegebenen Definitionen.

## Bestehende Dateien geändert

- `native/src/share/core/peer_extensions.rs`
- `native/src/share/core/peer_request.rs`
- `native/src/share/core/peer_fs_logging.rs`
- `native/src/share/core/fs_request.rs`
- `native/src/share/core/fs_response.rs`
- `native/src/share/core/fs_access.rs`
- `native/src/share/core/server_fs.rs`
- `native/src/share/core/wire_capabilities.rs`

## Dateien erstellt

- `native/src/share/core/peer_reversible_replace.rs`
- `native/src/share/core/fs_reversible_replace.rs`
- `native/src/share/core/reversible_replace_task_tests.rs`
- `native/src/share/core/server_fs_control.rs`
- `native/src/share/core/peer_request_policy.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/H-REPLACE.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/H-REPLACE.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/H-REPLACE.md`

Registrierungen liegen ausschließlich bei den eigenen Elternmodulen `peer_extensions.rs`, `fs_access.rs`, `server_fs.rs` und `peer_request.rs`; der neue Fixture-Submodulpfad liegt in `peer_reversible_replace.rs`. `share/mod.rs` wurde nicht geändert.

## Hauptanschlüsse

Exakte Signaturen stehen in [api-delta/H-REPLACE.md](../api-delta/H-REPLACE.md). Der E-APPLY-Cleanup-Anschluss ist im Arbeitsbaum vorhanden. Dauerhafter Engine-Intent sowie der vereinbarte additive Wire-DTO-Reexport und repräsentative Roundtrip-Fixtures bleiben bei E-ENGINE beziehungsweise beim Hauptagenten; Details stehen in [anfragen/H-REPLACE.md](../anfragen/H-REPLACE.md). Nach der Übergabe stoppt der Worker. Commit/Push, Root-Graph, die eine Remote-Suite und Release bleiben beim Hauptagenten.
