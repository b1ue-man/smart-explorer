# E-PLAN – Umsetzung, Fundzuordnung und Abnahme

Stand: 2026-10-03. K3 → E-PLAN ist als eigener Umsetzungsblock abgeschlossen.
Die vorhandenen Teiländerungen und freigegebenen Verträge V1–V5 wurden
weitergeführt; ein neues Review wurde nicht begonnen. Die verbundenen
Apply-/Snapshot-Verträge gehen mit [anfragen/E-PLAN.md](../anfragen/E-PLAN.md)
an E-APPLY. Dessen fehlende Implementierung ist eine Owner-Abhängigkeit;
Kompilierung und Verhaltensabnahme stehen für den gemeinsamen Kandidaten aus.

Lokale Builds, Compiler, Linker, Cargo, Rustformatierer, Tests, Server,
Installationen, Commits, Pushes und Graph-Neubau wurden nicht ausgeführt.
Die registrierten `review_task_`-Fälle sind Quellmaterial für die eine
abschließende Remote-Task-Suite des Hauptagenten.

## Planung und Recherchegrundlage

Ausgangspunkt waren K3-Typen mit noch unverbundener Paarplanung,
`run_with` ohne wirksame Laufsettings, einer nur bei vollständigem Erfolg
gespeicherten Basis und einem vor seinen Einträgen bestätigten SQL-Bootstrap.
Die erste Planung verband Paarplanung, Replika/Basis, Fortschrittssicherung,
inkrementellen Cache und interaktive Einzelaktionen in einem Block.

Der detaillierte Plan folgte FS1–FS4 sowie B01/B09/B18: getrennte
Owner-/Replikazustände; konservativer Leere-/Replikaschutz; gemeinsame Schlüssel
und Filter; wiederaufnehmbare Teilergebnisse; atomarer, optionaler Index;
kompatible Job-Migration und Vorschau. Recherchegrundlage waren V1/V3,
die gesicherten Quellen in `docs/refs/local-fs-identity-durability.md`,
vorhandene SQLite-Transaktionen, Backend-Metadaten und K3-Abnahmefälle.
Die zweite Lückenprüfung präzisierte FAT-Zeitauflösung, fehlende/unbekannte
Identität, Journal-Wiederaufnahme, Alias-Kollisionen und Apply-/Walk-Anschlüsse.
Der Hauptagent hat die benötigten fremden Lesedateien freigegeben.

## Umsetzung und Fundzuordnung

| Ergebnis | Umsetzung | Zuordnung |
|---|---|---|
| Erfolgreiche Einträge bleiben bei Fehler, Abbruch und Neustart erhalten. | `CheckpointSink` journalisiert gemeldete Ergebnisse nach 64 Aktionen bzw. alle zwei Sekunden; nicht dauerhafte Ergebnisse synchronisieren vorher die betroffenen Seiten. Basis/Ordnerhistorie werden verdichtet. Unvollständige letzte Frames werden wiederaufgenommen, vollständige beschädigte Frames abgewiesen. | FS1, Y29/Y32/Y33/Y42/Y55/Y58, B18 |
| Unveränderte Spiegel-Paare kopieren bei verschiedenen bestätigten Seitenzeiten nicht erneut. | Gemeinsamer Paarplan vergleicht jede Seite mit ihrer eigenen Basis. Checksum bestätigt keinen fehlenden Hash. Zielzeitübertragung und bytegebundene Ergebnisse sind E-APPLY. | FS1/FS2-Planung, Y30/Y57 |
| Einweg-Jobs reparieren alleinige Zieländerungen; echte gemeinsame Bearbeitung bleibt FileLevel-Konflikt. | Größen-/Zeitkonfliktregeln geben geänderten vorhandenen Dateien Vorrang vor Löschung. Bestätigte Move-Paare ermöglichen `FinalizeMove`. | Y40/Y41/Y59 |
| Schlüssel verändern keine I/O-Schreibweise. | NFC/paarweise Faltung verbinden beide Seiten und Basis; `Spellings` und Seitenschreibweisen behalten Literalnamen. Datei-/Ordner-/Typ-Aliase schützen Nachfahren. Vergleich verwendet Seitenpräzision, FAT-Stundentoleranz und sichere Differenzen. | FS2/FS4-Planung, Y37/Y46/Y47/Y48/Y52/Y102/Y129/Y153 |
| Filter und Auslassungen löschen keine Gegenstücke. | `filtered` bleibt gesondert beobachtet; Quellen-/Zwei-Wege-Filter schützen beide Seiten. Eine eingeschlossene Einweg-Quelldatei darf ihren gefilterten Zielgegenpart aktualisieren. Zielgrenzen lassen nur betroffene Dateien aus. Gemeinsame dynamische Speicherbudgets ersetzen feste Persistenz-/Indexgrenzen. | FS4-Planung, Y35/Y36/Y67/Y68/Y91/Y120; Walk-Anschluss bei E-APPLY |
| Fehlende frühere Marker oder vormals volle leere Seiten stoppen vor Apply. | Beide Marker werden vor Anlage geprüft. Unlesbare/unbekannte Identität beweist keinen Wechsel. Volume → eigener Marker übernimmt den bisherigen Owner-Zustand; echte Rotation erhält unabhängige Basis. Vorherige Eintragszahlen bleiben für Leere-Schutz erhalten. | FS3, Y31/Y62/Y39, B01 |
| Jeder Schutz benötigt die passende Bestätigung. | Absoluter Grenzwert, inklusiver Prozentwert je Seite mit Mindestzahl, Marker- und Leere-Schutz liefern `Outcome.blocked`. Verifizierte Moves zählen nicht. Dedupe zählt physische IDs je Seite genau einmal. | FS3, Y31/Y62/Y69, B09 |
| Ein beschädigter oder gesperrter Index blockiert kein sicher protokollierbares Datei-Apply. | SQL ist optional; dauerhafter Dirty-Marker erzwingt neuen Bootstrap. Zeilen und Pair/Cursor sind eine Transaktion. Tombstones entfernen Zeilen. Partialscan, Filterwechsel, Alias-/Rename-Ambiguität oder Ziel-Drift erzwingen Vollplanung; `verify_target_secs` erzwingt Ziel-Vollprüfung. | Y39/Y44/Y53, FS3; Kontroll-Lauf bei T-JOBS |
| Dedupe-/Reparaturfehler lassen unabhängige Pfade weiterlaufen. | Konflikte/Fehler schützen ihre Gruppen. Eine gesicherte reportingbasierte Kandidatenliste ersetzt direkten Backend-Apply. Eine Extra-ID-Löschung behält den Primärzustand. | Y63, FS1/FS3; Ausführung bei E-APPLY |
| Vorschau und interaktive Aktionen verwenden denselben Zustand. | Vorschau nutzt Paarplan, Schutz, Literalpfade und erfasste Optionen. Einzelaktion revalidiert unter Paarsperre. Reguläres `resolve_recorded` nutzt Reporting/Checkpoint und betreffende Job-Versionierungsoptionen. | FS1/FS3, Y45/Y150; Duplikat-Resolve bei E-APPLY |
| Neue Jobs erben keine fremde alte Basis. | Basis/Index gehören Paar, Owner und beiden Replikas. Altjob-Nachweis bindet ursprüngliche Endpunkte. Migration: alte 0 → 50 % ab 25, gesetzter Prozentwert behält Mindestzahl 0; spätere 0 bleibt 0. Offener TSV-Import behält exakte Konfigurationsbytes/Hash. Bereinigung erfasst passende Indexzustände. | Y150/Y153, B01/B09 |

Kein Ergebnis-Vollscan nach Apply: Nur beobachtete Konvergenz und bestätigte
Apply-Signaturen ändern die Basis. Fehlgeschlagene, ausgelassene, driftende
oder kollidierende Einträge behalten ihren Zustand. `ApplySink::should_stop`
stoppt neue Mutationen bei Checkpointfehler oder typisiertem Laufstopp.

## Konkrete Abnahmesignale für die gemeinsame Suite

Alle Fallnamen tragen zusätzlich das Präfix `review_task_`.
Die Fälle werden ausschließlich in der finalen Remote-Suite ausgeführt;
hier wird kein Lauf als bestanden behauptet.

| Quelle | Fälle | Erwartetes Signal |
|---|---|---|
| `bisync/core/plan_review_tests.rs` | `mirror_uses_each_recorded_time_without_copy_churn`, `equal_hashes_converge_without_a_previous_basis`, `checksum_never_certifies_a_missing_hash`, `precision_applies_to_each_baseline_side`, `fat_hour_shift_is_separate_from_the_modify_window` | Konvergenz ohne erneute Kopie; fehlende Prüfsummen bleiben ungeprüft; FAT-Toleranz erweitert kein allgemeines Änderungsfenster. |
| dieselbe Datei | `one_way_repairs_destination_drift_and_keeps_real_conflicts`, `modified_file_beats_deletion_under_size_and_time_policies`, `pending_move_uses_its_acknowledged_pair` | Ziel-Drift wird repariert, echte Konflikte bleiben, Löschung verdrängt keine geänderte Datei, Move finalisiert nur bestätigte Quelle. |
| dieselbe Datei | `folded_nfc_keys_keep_both_io_spellings_and_existing_parents`, `collisions_and_omissions_never_forget_prior_spellings`, `directory_alias_collision_protects_descendants`, `filter_transitions_protect_source_and_update_included_destination`, `target_limits_omit_only_the_impossible_files`, `directory_history_distinguishes_union_from_propagated_removal` | Literalpfade/Gegenstücke/Ordnerhistorie bleiben erhalten; unabhängige Dateien bleiben planbar. |
| dieselbe Datei | `percentage_stop_is_per_side_inclusive_and_move_free`, `confirmed_absolute_limit_does_not_confirm_the_percentage_stop`, `silent_filtered_folder_is_not_an_empty_volume` | Schutzbestätigungen sind unabhängig; Filterung erzeugt keinen falschen Leere-Stopp. |
| `bisync/os/shared/checkpoint_review_tests.rs` | `checkpoint_journal_recovers_and_truncates_only_a_torn_tail`, `checkpoint_rejects_a_complete_corrupt_frame`, `checkpoint_keeps_successes_and_old_deferred_entries_on_stop`, `checkpoint_flushes_before_the_end_of_a_long_run`, `checkpoint_timer_saves_while_the_next_transfer_is_slow`, `external_merge_replays_checkpoints_before_updating_one_entry`, `checkpoint_budget_rejects_delta_before_journaling_it` | Wiederaufnahme erhält bestätigte Ergebnisse/alte geschützte Einträge. Zwischenstände existieren vor Laufende; ungültige Deltas ändern die Basis nicht. |
| dieselbe Datei | `corrupt_optional_index_does_not_block_completed_file_work`, `daily_target_verification_finds_untracked_mirror_orphans` | Cachebeschädigung verhindert keine Dateiübertragung; fällige Vollprüfung findet unbekannten Ziel-Orphan. Benötigt fertiges E-APPLY. |
| `bisync/os/shared/replica_review_tests.rs` | `missing_previous_marker_blocks_creation_on_both_sides`, `marker_upgrade_carries_basis_but_rotation_uses_another_state`, `unavailable_identity_preserves_known_replicas_without_rotation` | Markeranlage folgt Schutzentscheid; Upgrade/Rotation haben unterschiedliche Basisregeln; Unlesbarkeit ist kein Wechsel. |
| `bisync/os/shared/index_review_tests.rs` | `index_bootstrap_rolls_back_rows_and_cursor_together`, `index_tombstones_remove_rows_and_owner_cleanup_stays_scoped` | Bootstrap rollt Zeilen und Cursor gemeinsam zurück oder ersetzt beide; Cleanup erhält fremde Jobs. |
| `syncjobs/os/shared/persistence_review_tests.rs` | `delete_guard_migrates_once_and_preserves_explicit_later_zero`, `pending_original_import_keeps_its_exact_configuration_bytes` | Altstandards migrieren einmal; spätere 0 und offene Importbytes bleiben erhalten. K3-Roundtrip-/Locatorfälle bleiben registriert. |

Die übergreifenden FS1–FS4-Abnahmen aus `umsetzung.md` bleiben notwendig:
reale Teilübertragung mit Fehler/Abbruch, wiederholter Vollscan, Links/Mounts/FIFO,
gemischte lokale/Fern-Endpunkte, Dedupe-Backupfehler, ENOSPC/Nur-lesen und
Rotation mit leerem Ziel. Diese Quellenfälle ersetzen keine betroffene
Integrationsabnahme.

## Entscheidungen und Kompatibilität

- FileLevel bleibt bei beiderseitiger echter Bearbeitung streng.
- Journal/Basis sind maßgeblich, SQL ist ein wiederaufbaubarer Cache.
  Fehler des eigenen dauerhaften Zustands stoppen weitere Mutationen.
- Frames speichern ihre Faltungsregel; Replay benutzt diese und baut danach
  den aktuellen Schlüsselindex auf. Budgets werden vor Schreiben geprüft.
- Volume → eigener Marker übernimmt nur denselben bisherigen Owner.
  Neue Jobs und echte Rotation erhalten unabhängige Zustände.
- Endpunktstrings/Backend-/Verbindungsidentität bleiben unverändert.
  Gleiche relative Pfade verschiedener Fernverbindungen bleiben verschieden;
  lokale, UNC/mapped drive, SFTP/Agent, FTP/FTPS, WebDAV, Drive und
  Direct/Room Share-Orte gehen weiter über die Backend-Grenze.
- Apply, Snapshot und `versions.rs` wurden nur gelesen.
  Moduldateien enthalten ausschließlich eigene additive Registrierungen.

## Statische Eigenprüfung

Eigene Änderungen wurden auf Kontrollfluss, Persistenzreihenfolge,
Faltungs-/Literalpfade und Owner-Grenzen geprüft. Ein reiner Textscanner
prüfte UTF-8, Klammern/Kommentare/Zeichenketten und nachlaufenden Whitespace;
`git diff --check` meldete für zugeordnete Änderungen keinen Befund.
Alle bearbeiteten Rust-Dateien bleiben unter 500 Zeilen und 50 KiB.
Diese Kontrollen sind kein Rust-Compiler und keine Verhaltensabnahme.
Root-Graph, Formatierung, Kompilierung, Remote-Suite, Commits und Integration
liegen beim Hauptagenten. Keine weitere E-PLAN-Owner-Lücke bleibt offen.

## Dateien

### Neu erstellt

```text
native/src/bisync/core/baseline_records.rs
native/src/bisync/core/plan_review_tests.rs
native/src/bisync/os/shared/checkpoint_journal.rs
native/src/bisync/os/shared/checkpoint_run.rs
native/src/bisync/os/shared/checkpoint_review_tests.rs
native/src/bisync/os/shared/index_review_tests.rs
native/src/bisync/os/shared/orchestration_plan.rs
native/src/bisync/os/shared/persistence_versions.rs
native/src/bisync/os/shared/replica.rs
native/src/bisync/os/shared/replica_review_tests.rs
native/src/bisync/os/shared/single_recorded.rs
native/src/bisync/os/shared/state_bootstrap.rs
native/src/bisync/os/shared/state_metadata.rs
native/src/bisync/os/shared/state_spellings.rs
native/src/syncjobs/os/shared/baseline_migration.rs
native/src/syncjobs/os/shared/recorded_options.rs
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/E-PLAN.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/E-PLAN.md
```

### Bestehende Dateien/Teiländerungen weitergeführt

```text
native/src/bisync/core/compare.rs
native/src/bisync/core/completion.rs
native/src/bisync/core/plan.rs
native/src/bisync/core/plan_decide.rs
native/src/bisync/core/plan_pair.rs
native/src/bisync/core/plan_filter.rs
native/src/bisync/core/snapshot_types.rs
native/src/bisync/core/types.rs
native/src/bisync/core/run_types.rs
native/src/bisync/mod.rs
native/src/bisync/os/shared/persistence.rs
native/src/bisync/os/shared/persistence_tests.rs
native/src/bisync/os/shared/replica_state.rs
native/src/bisync/os/shared/replica_state_tests.rs
native/src/bisync/os/shared/orchestration.rs
native/src/bisync/os/shared/orchestration_full.rs
native/src/bisync/os/shared/preview.rs
native/src/bisync/os/shared/resolve.rs
native/src/bisync/os/shared/pair_lock.rs
native/src/bisync/os/shared/incremental.rs
native/src/bisync/os/shared/incremental_collect.rs
native/src/bisync/os/shared/incremental_changes.rs
native/src/bisync/os/shared/state_store.rs
native/src/bisync/os/shared/state_validation.rs
native/src/syncjobs/mod.rs
native/src/syncjobs/os/shared/persistence.rs
native/src/syncjobs/os/shared/persistence_review_tests.rs
```

### Gelesen bzw. gezielt eingesehen

Alle oben genannten Rust-Dateien und die beiden eigenen Berichte; außerdem:

```text
AGENTS.md
docs/ARCHITEKTUR.md
docs/lesungen/INDEX.md
docs/refs/INDEX.md
docs/refs/local-fs-identity-durability.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/e-plan.json
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/umsetzung.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/spec.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/recherche.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-sync.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/K3.md
docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/K3.md
native/src/bisync/core/contract_tests.rs
native/src/bisync/core/duplicate_types.rs
native/src/bisync/core/guards.rs
native/src/bisync/core/keys.rs
native/src/bisync/core/limits.rs
native/src/bisync/core/omissions.rs
native/src/bisync/core/paths.rs
native/src/bisync/core/plan_dirs.rs
native/src/bisync/core/plan_index.rs
native/src/bisync/core/plan_types.rs
native/src/bisync/os/shared/apply.rs
native/src/bisync/os/shared/checkpoint.rs
native/src/bisync/os/shared/duplicate_apply.rs
native/src/bisync/os/shared/duplicate_plan.rs
native/src/bisync/os/shared/snapshot.rs
native/src/bisync/os/shared/snapshot_hash.rs
native/src/bisync/os/shared/snapshot_pair.rs
native/src/bisync/os/shared/state_types.rs
native/src/bisync/os/shared/versions.rs
native/src/syncjobs/core/types.rs
native/src/syncjobs/core/validation.rs
native/src/syncjobs/os/shared/migration.rs
native/src/syncjobs/os/shared/persistence_codec.rs
native/src/vfs/core/core.rs
native/src/vfs/core/extension_calls.rs
native/src/vfs/core/extension_types.rs
native/src/vfs/mod.rs
/root/.codex/skills/arbeitsweise/SKILL.md
/root/.codex/skills/graphify/SKILL.md
```

Die zugeordneten Skill-Arbeitsreferenzen wurden verwendet.
Die initiale Graph-Abfrage lieferte die V3-Symbole; der Hauptagent
aktualisiert den Root-Graph nach Integration.
