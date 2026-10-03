# H-POLICY-BOUNDARY – Host-Policy hinter der OS-Grenze

Stand: 2026-10-03. Der eng beauftragte Source-Anschluss ist abgeschlossen. Er verschiebt die bestehende H-DISPATCH-Policy; gemeinsame Remote-Abnahme und Veröffentlichung bleiben beim Hauptagenten. Grundlage sind das Scope-Manifest und der dokumentierte Architekturanschluss in `integration.md`, keine neue Review-Kampagne.

## Ergebnis und Zuordnung

| Meilenstein / vorhandene Grenze | Änderung | Konkretes erwartetes Signal |
|---|---|---|
| P1 – H-DISPATCH H4, FC1/S36/S63/S65 | `TargetPolicy`, kanonische Hostfakten, App-/Cache-Roots und `ensure_handle_allowed` liegen in `os/shared/fs_host_policy.rs`. `private_name/private_path/system_write` und reine Textklassifikation bleiben Core. | Private und effektive Aliasziele bleiben gesperrt; Nur-lesen-Schreiben bleibt `ReadOnlyFilesystem`; Systemorte brauchen weiterhin Opt-in. |
| P2 – bestehender Local-/UNC-/Literalvertrag | Hostzielauflösung liegt in `os/shared/fs_local_paths.rs`; Windows-/Unix-Pfadkonvertierung und Containment liegen in ausgewählten OS-Adaptern. `fs::local_paths` bleibt als Alias erhalten. | Bestehende Local-/UNC-/Drive-Root-Schreibweisen und literale Child-Komponenten liefern dieselben Ziele und Fehler wie zuvor. Remote-Locator- und Verbindungsauflösung werden nicht verändert. |
| P2 – H-DISPATCH H4/H6, vorhandener destruktiver Preflight | Der iterative Handle-Walk liegt in `os/shared/fs_host_destructive.rs`, verwendet dieselben `DirectoryHandle`-Operationen und dieselbe `memory_budget()/8`-Grenze. | Private Child-Namen und physische App-/Cache-Aliase verhindern Rename/Delete vor Mutation; Link-Children werden nicht rekursiv geöffnet; Budgetfehler bleibt retrybar. |
| P3 – vorhandene Policy-/Local-Path-Fixtures | Host-Fixtures wurden in `os/shared/fs_host_policy_task_tests.rs` verschoben. Reine Namens-/Systemklassifikation bleibt `core/fs_policy_task_tests.rs`. Die ausgewählte OS-Funktion erstellt die vorhandene Symlink-Fixture. | Bestehende Funktionsselektoren bleiben erhalten und werden in derselben finalen Remote-Suite verwendet. |
| P4 – Scope und Integration | Nur zugewiesene Consumer-Imports und eigene additive Modulregistrierungen geändert; alte Core-Hostdateien nach Referenzmigration gelöscht. | Eigene Core-/Shared-Quellen enthalten keine OS-Selektion oder `MAIN_SEPARATOR`-Abzweige; übrige Consumer verwenden den bisherigen Alias. |

## Erhaltene Entscheidungen

Windows-Policy-Containment benutzt weiterhin die bestehende Textnormalisierung einschließlich Case-, Verbatim- und UNC-Schreibweisen. Unix-Policy-Containment bleibt `Path::starts_with`. Die kanonische Zielauflösung behält auf beiden OS ihren bisherigen komponentenbasierten Vergleich; deshalb gibt es getrennte `policy_contains`- und `canonical_contains`-Adapterfunktionen.

Bare-Drive-Konvertierung (`C:` nach `C:/`), Slash-Ausgabe, private Namensnormalisierung einschließlich Stream-/Punkt-/Leerzeichenformen und sämtliche Systemort-Marker sind erhalten. Effektive neue Ziele werden weiterhin über den nächsten existierenden Parent geprüft. Private Roots werden wie bisher auch beim Erzeugen einer nichtlokalen Policy gesammelt.

Der bestehende gewöhnliche `DirectoryHandle`-Zugang, dessen Fehler und die physische Ancestry-Prüfung werden nur konsumiert. Consent/Broker, UNC-/Bind-Aliasprüfung, Daten-Reparsepunkte, Provider-Fähigkeiten sowie Cancel-/Backpressure- und Rechteprüfungen der Guards behalten ihre bestehenden Grenzen.

## AcceptanceSelector für die eine bestehende Remote-Suite

Keiner dieser Selektoren wurde lokal ausgeführt oder kompiliert. Die Funktionsnamen wurden erhalten; nur die unten dokumentierten Host-Modulpfade ändern sich.

| AcceptanceSelector | Ort / Signal |
|---|---|
| `review_task_host_private_names_hide_quarantine_and_versions_but_allow_transfer_stages` | Weiterhin `share::fs_policy::task_tests`: private Varianten gesperrt, normale Transfer-Stages und `node_modules` erlaubt. |
| `review_task_host_system_write_classification_covers_startup_shell_and_keys` | Weiterhin `share::fs_policy::task_tests`: reine Klassifikation unverändert. |
| `review_task_host_read_only_is_a_rights_error_for_normal_write_paths` | Jetzt `share::fs_host_policy_task_tests`: normale Writes melden Rechtefehler, private Reads bleiben gesperrt. |
| `review_task_host_system_opt_in_never_opens_app_private_or_versions` | Jetzt `share::fs_host_policy_task_tests`: Opt-in öffnet keine privaten Daten. |
| `local_target_stays_under_root` | Jetzt `share::fs_host_policy_task_tests`: kanonischer neuer Child bleibt unter dem Root. |
| `symlink_escape_is_blocked_when_supported` | Jetzt `share::fs_host_policy_task_tests`: vorhandener auswärts gerichteter Verzeichnislink wird abgewiesen. |

Die vorhandenen H-DISPATCH-Integrationsselektoren `review_task_host_read_only_backend_denies_write_commit_and_recycle_before_provider`, `review_task_host_guard_preserves_literal_provider_hook_and_safe_identity_aliases`, `review_task_host_fast_report_removes_private_sizes_from_every_ancestor` und `review_task_host_fast_report_preserves_provider_stage_and_rejects_path_injection` bleiben direkt betroffene AcceptanceSelector. Ihre außer-scope Quellen wurden nicht neu untersucht oder geändert.

Die bereits dokumentierten echten Windows-/UNC-/Bind-Mount-/Systemopt-in-Fälle gehören in denselben gemeinsamen Suite-Aufruf. Diese Verschiebung beansprucht keine neue Abnahmefläche oder vollständige allgemeine LocalBackend-TOCTOU-/Datei-Hardlink-Härtung.

## Statische Prüfung und eigener Self-Review

Die eigenen geänderten/neu erstellten Rust-Quellen wurden mit dem vorhandenen Tree-sitter-Rust-Parser gelesen: keine `ERROR`-/Missing-Knoten. Eigene Registrierungen wurden separat auf Syntax, exakte Pfade und OS-Auswahl geprüft. `git diff --check` für die zugewiesenen Pfade blieb ohne Befund. Neue Rust-Dateien bleiben deutlich unter 500 Zeilen und 50 KiB; die reinen Import-/Aliasänderungen in den bestehenden Consumer-Dateien vergrößern keine Feature-Verantwortung.

Die bereits größere `share/mod.rs` ist ausschließlich eine Registrierungsausnahme: eigene additive Einträge für Host-Policy, destruktiven Preflight, lokalen Hostadapter, je ausgewählten OS-Pfadadapter und Host-Fixtures. Die bereits vorhandenen parallelen S09-/Root-Änderungen wurden nicht ersetzt und werden diesem Block nicht zugerechnet.

Self-Review: Rechtefehler, Reihenfolge Read vor Write, App-/Cache-Privatheit, physische Root-Prüfung, Link-Omissions, iterative Speichergrenze, Canonicalize-Fehlertext und Alias-Sichtbarkeit wurden gegen die direkt betroffenen bisherigen Quellen verglichen. Der Hauptagent meldete den einzigen außer-scope Typimport in `fs_guard_bulk.rs` und hat ihn selbst auf `fs_host_policy::TargetPolicy` migriert. Diese Datei wurde vom Worker nicht gelesen oder geändert.

Keine Builds, Tests, Compiler, Formatter, Server, Installationen, langen Prozesse, Git-Mutationen, CI, Releases oder Graph-Neubauten ausgeführt. Parsing ist kein Laufzeit-/Typprüfnachweis.

## Gelesen / gezielt abgefragt

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/h-policy-boundary.json`
- `native/src/share/core/fs_policy.rs`
- `native/src/share/core/fs_policy_destructive.rs`
- `native/src/share/core/fs_local_paths.rs`
- `native/src/share/core/fs_policy_task_tests.rs`
- `native/src/share/core/fs.rs`
- `native/src/share/mod.rs`
- `native/src/share/core/fs_guard_backend.rs`
- `native/src/share/core/fs_guard_reports.rs`
- `native/src/share/core/fs_guard_stream.rs`
- `native/src/share/core/fs_paths.rs`
- `native/src/share/core/export_config.rs`
- `native/src/local_access/mod.rs`
- `docs/ARCHITEKTUR.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/H-DISPATCH.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/H-DISPATCH.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/integration.md`
- `AGENTS.md`
- `native/src/local_access/os/linux/directory_handle.rs`
- `native/src/local_access/os/linux/private_ancestors.rs`
- `native/src/local_access/os/windows/directory_handle.rs`
- `native/src/local_access/os/windows/private_ancestors.rs`
- `native/src/support_dirs.rs`
- `native/src/transfer/os/shared/memory.rs`
- `native/src/share/os/shared/fs_host_policy.rs`
- `native/src/share/os/shared/fs_host_destructive.rs`
- `native/src/share/os/shared/fs_host_policy_task_tests.rs`
- `native/src/share/os/shared/fs_local_paths.rs`
- `native/src/share/os/windows/fs_path_adapter.rs`
- `native/src/share/os/linux_os/fs_path_adapter.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/H-POLICY-BOUNDARY.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/H-POLICY-BOUNDARY.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/H-POLICY-BOUNDARY.md`

Bereits geladene Skills `arbeitsweise` und `graphify` wurden weiter angewendet; Root lieferte die gezielte Graph-Abfrage. Der vorhandene Parser-Interpreter wurde nur für kurze statische Syntaxarbeit an eigenen Quellen verwendet.

## Bestehende Dateien geändert

- `native/src/share/core/fs_policy.rs`
- `native/src/share/core/fs_policy_task_tests.rs`
- `native/src/share/core/fs.rs`
- `native/src/share/core/fs_guard_backend.rs`
- `native/src/share/core/fs_guard_reports.rs`
- `native/src/share/core/fs_guard_stream.rs`
- `native/src/share/mod.rs`

## Dateien erstellt

- `native/src/share/os/shared/fs_host_policy.rs`
- `native/src/share/os/shared/fs_host_destructive.rs`
- `native/src/share/os/shared/fs_host_policy_task_tests.rs`
- `native/src/share/os/shared/fs_local_paths.rs`
- `native/src/share/os/windows/fs_path_adapter.rs`
- `native/src/share/os/linux_os/fs_path_adapter.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/H-POLICY-BOUNDARY.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/H-POLICY-BOUNDARY.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/H-POLICY-BOUNDARY.md`

## Dateien gelöscht

- `native/src/share/core/fs_policy_destructive.rs`
- `native/src/share/core/fs_local_paths.rs`

Die Löschungen erfolgten nach Migration aller Referenzen in der zugewiesenen Fläche. Keine weiteren Dateilöschungen.

## Rest / Owner-Grenzen

Es bleibt die gemeinsame Remote-Auswertung samt Commit/Push, Root-Graph-Aktualisierung und abschließender Veröffentlichung beim Hauptagenten. Die bekannten H-DISPATCH-Restgrenzen bleiben dort dokumentiert. Exakte API- und Selector-Pfadänderungen stehen in [api-delta/H-POLICY-BOUNDARY.md](../api-delta/H-POLICY-BOUNDARY.md), die Owner-Übergabe in [anfragen/H-POLICY-BOUNDARY.md](../anfragen/H-POLICY-BOUNDARY.md).

