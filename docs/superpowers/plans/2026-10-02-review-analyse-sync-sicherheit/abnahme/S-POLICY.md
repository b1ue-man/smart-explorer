# Abnahme S-POLICY

Stand: 2026-10-03. Fortsetzung des vorhandenen FC1-Plans und der freigegebenen V2/V5-Verträge;
kein neues Review. Umsetzung und Remote-Abnahme werden getrennt dokumentiert.

## Konkretisierung des vorhandenen Plans

| Meilenstein | Dateien/Grenze | Erwartetes Signal der gemeinsamen späteren Remote-Suite |
|---|---|---|
| Sichere neue Konfiguration | profiles, profile_persistence, profile_operations | Neuer Profilstore und per Code/PIN angelegte Räume haben keine Freigaben; neue Ordner/Verbindungen sind nur lesbar, neue Grants bleiben ohne Schreiben/Exec. |
| Nachvollziehbare Migration | eigene reine Migrationsdatei, Persistenzadapter | Nur exakt erkanntes altes Auto-Home ohne ausdrückliches access wird einmalig nur lesbar und erhält dauerhaften Hinweis. Alle anderen ausdrücklichen Export-/Kontakt-/Raumrechte und Pfade bleiben erhalten; alte include_connections werden zu aktuellen einzelnen Konten, später gespeicherte Konten werden nicht automatisch freigegeben. |
| Retrybare private Persistenz | profile_store, profile_persistence, profile_transaction | Reale Datei- und kodierte Bytegröße begrenzt; private Regularfile-/Lock-/Stage-API von S-LOCAL. CAS-/Diskfehler behalten den bisherigen Zustand und die Widerrufshistorie, Wiederholen kann erfolgreich persistieren. |
| Feldweise Benutzerrechte | profile_edits, eigene Policy-Aktionen | Concurrent Runtime-/Pin-/Ledger-Fakten bleiben erhalten. Sperren laufen durch withdraw_direct_key; explizite Share-back-/Schreib-/Raumrechte ändern keine fremde Identität und reparieren keine neuen Grants nebenbei. |
| Drei bestehende Bedienwege | Desktop-Freigaben/Räume, CLI-Exports/Grants, Mobile-JSON/Routes | Rechte, Auto-Home-Hinweis/Wiederfreigabe und einzelne Konto-Freigaben sind sichtbar und bewusst änderbar. Mobile-Delta geht an AND-SHARE-UI, keine Kotlin-Erkundung. |

Kompatibilität: root.path, SavedConnection.account(), Profile-/Raum-IDs und Endpoint-Präfixe werden bei
Policyänderungen unverändert erhalten. Legacy fehlendes access/write/RoomPolicy behält die bisherige
Bedeutung; ausdrückliche Entscheidungen werden nicht durch Migration überschrieben. Widerrufs- und
Legacy-Ledger werden nie für Anzahl-/Transport-/Runtimeflags gekürzt.

Recherche/Lückenabgleich aus dem vorhandenen Plan: FC1 in spec.md, V2/V5 in umsetzung.md,
S-REVOKE/S-SIGNAL-Berichte und aktueller Persistenz-/Bediencode. DirectRequestPolicy bleibt im
bestehenden Gerätepräferenzstore mit Ask als Standard; kein zweiter Policy-Store. Saved-Konten kommen
aus creds::load_connections_checked(); Ladefehler sind kein leerer Kontensatz. Private Speichererstellung nutzt ausschließlich die vom Hauptagenten
zugewiesene gemeinsame S-LOCAL-API. Der bestehende CAS-Vertrag bleibt die Commit-Grenze.

## Umsetzung / Fundzuordnung

FC1 ist auf der zugewiesenen nativen Konfigurationsoberfläche umgesetzt. Die folgende Abnahme ist
Quell- und Vertragsabnahme; eine erfolgreiche Laufzeitprüfung wird nicht behauptet. AND-SHARE-UI
übernimmt die Android-Bedienelemente anhand des additiven Deltas.

| Vorhandener Punkt | Konkrete Änderung |
|---|---|
| FC1: sichere Standards | Fehlender Profilstore erzeugt keine Home-Freigabe. Beide bestehenden Raum-Anlegepfade übernehmen keine Direct-Exports. Vorhandene V2/V5-Defaults bleiben erhalten: neue Ordner/Konten nur Lesen, neue Grants ohne Schreiben/Exec, neue Kontakte ohne Share-back; ältere ausdrückliche Grants/RoomPolicy behalten Schreiben. |
| FC1: alte automatische Home-Freigabe | Nur Label `Home`, exakt unveränderter Home-Ort, fehlendes JSON-`access`, kein Systemwrite-Opt-in und Legacy-Schreibrecht werden auf Lesen umgestellt. `auto_home_migrations` hält `{scope,path}` dauerhaft fest. Explizites `access`, andere Orte und Literalnamen bleiben erhalten. Ohne Home-Fakt scheitert eine unbestimmte alte Home-Konfiguration sichtbar; kein Ersatzprofil wird gespeichert. |
| FC1: einzelne Verbindungen | Altes `include_connections=true` wird mit dem strikten SavedConnection-Loader einmalig in aktuelle einzelne Konten umgewandelt; bisherige RW-Rechte und explizite RO-Einträge bleiben erhalten. Später gespeicherte Konten werden nicht automatisch geteilt. Alle neuen Bedienwege wählen Konten einzeln mit Warnung über die eigenen Zugangsdaten. |
| FC1: bewusstes Schreiben | Exportzugriff und Kontakt-/Raumschreibrecht sind getrennt. Die neue Kontaktaktion vergleicht Device-ID, Key, Node-ID und Fingerprint bei jedem CAS-Versuch; sie erstellt und reaktiviert keinen Grant. Aussetzen/Removed-Denial verhindert Schreibfreigabe. Ausdrückliches Share-back verwendet unverändert den bestehenden S-REVOKE-Vertrag. |
| FC1 / V5: Feldrebase | GUI-Änderungen überschreiben nur geänderte Root-/Account-/Policyfelder. Concurrent Exports, Runtime, gelernte Pins und sämtliche Ledger bleiben bestehen. Ersetzte/entfernte ausgewählte Identitäten führen zu Fehler und vollständigem In-Memory-Rollback. Trust-Reset nutzt `withdraw_direct_key`; Raumblock/Zulassen nutzen die bestehenden Member-Helfer. |
| Persistenzgrenze des vorhandenen Plans | Reale Datei und bereits die JSON-Kodierung sind auf 1 MiB begrenzt; keine 64er-Trunkierung. Laden, Save und CAS prüfen direkte sowie Legacy-Ledger. Änderungen an `auto_connect`/`auto_join`, Runtime oder Transport kürzen keine Widerrufshistorie. Private Datei/Lock/Stage kommen aus S-LOCAL; die bestehende durable CAS-Promotion bleibt bestehen. |
| Retrybarkeit | Migration wird vor Runtime-Übergabe gespeichert; bei CAS-Konflikt lädt der OS-Loader bis zu fünfmal neu. Speicher-/Validierungsfehler erhalten alte Bytes, Revision und Denials. `Untracked` darf nur einen fehlenden Store anlegen; ein Fallback oder stale Snapshot kann keinen bestehenden Store überschreiben. Replacement verwendet die Revision des Ausgangssnapshots. |

## Bedienung

Desktop: Freigaben zeigen Lesen/Schreiben je Root, den optionalen Systemdatei-Schreibzugriff,
Home-Hinweis mit „Schreiben wieder erlauben“, Einzelverbindungen sowie „Darf schreiben“ je Grant.
Die Raumansicht zeigt Schreib-/Bestätigungspolicy. Der wirkungslose Symlink-Schalter und die
pauschalen Verbindungsbuttons entfallen; die V2-Schutzgrenze bleibt verbindlich. Eine fehlende
Raumauswahl bearbeitet keine Direct-Exports. Speicherfehler setzen die Ansicht auf den Ausgangszustand
zurück und bleiben sichtbar; ein erneuter Benutzerbefehl kann speichern.

CLI: bestehendes `export`/`grants` bleibt erhalten; sichtbare Aliase `exports`/`contacts` ergänzen den
geplanten Wortlaut. `exports set`, `exports connections list/set`, `exports policy --room` und
`contacts set NAME --write|--read-only` ändern gezielt bestehende Rechte. Text/JSON zeigt Rechte,
Raumpolicy, alte Home-Migration und die vorhandene Ask-/AutoAccept-Präferenz. Reine Rechteänderungen
normalisieren keine gespeicherten Paths oder Accounts.

Mobile: vorhandene Statusformen werden additiv erweitert, neue Policy-/Connection-Routen nutzen
dieselbe Persistenz. Optionales `shareBack` bei `share.addDirect` ist für neue Kontakte false; erneutes
Hinzufügen erhält bestehende bewusste Beziehungen und meldet den kanonischen Wert. Scheitert die
separate bewusste Rückfreigabe nach Kontaktanlage, nennt der Fehler den gespeicherten Kontakt und
den retrybaren Folgebefehl. Details stehen in `../api-delta/S-POLICY.md`.

## Self-Review und statische Signale

Die eigenen Änderungen wurden auf Default-/Legacy-Unterscheidung, Pinwechsel, stale GUI-Rebase,
CAS-Fehler, Größenfehler, Missing-Home-Fakten und private I/O-Anbindung geprüft. Beide Ledger-Validatoren
begrenzen aktive Requests bzw. bestehende Replay-Tabellen, keine Removed-Denials. `git diff --check`
für die zugewiesenen Änderungen ist sauber. Ein rein lexikalischer Python-Delimiter-/Dateigrößencheck
fand keine offenen Klammern/Strings oder Größenverstöße in den Featuredateien. Er ist kein Rust-Parser
und kein Compiler. Alle neu/inhaltlich bearbeiteten Featuredateien bleiben unter 500 Zeilen/50 KiB;
das vorhandene größere `share/mod.rs` erhält ausschließlich eigene additive Registrierungen/Reexports.
Keine Builds, Compiler, Formatter, Tests, Server, Installation, CI, Grapharbeit oder Commits ausgeführt.

Für die gemeinsame spätere Remote-Suite sind Quellsignale ergänzt, ohne sie lokal auszuführen:

- `profile_persistence_tests.rs`: `review_task_fc1_implicit_home_and_connections_migrate_once_without_new_grants`,
  `review_task_fc1_failed_migration_is_retryable_and_returns_no_runtime_profile`,
  `review_task_fc1_new_room_has_no_inherited_exports`,
  `review_task_fc1_byte_budget_and_runtime_flags_preserve_more_than_64_denials`.
- `profile_policy_tests.rs`: `review_task_fc1_write_edits_do_not_readmit_withdrawn_or_replaced_keys`,
  `review_task_fc1_same_relative_path_on_two_saved_remotes_is_separate`,
  `review_task_fc1_room_policy_does_not_admit_pending_or_blocked_members`.
- `profile_edits.rs`: `review_task_fc1_gui_rebase_keeps_concurrent_exports_and_all_denials`,
  `review_task_fc1_gui_write_to_a_replaced_key_rolls_back_every_edit` und der bestehende Runtime-Rebase-Fall.
- Zusätzlich als gemeinsame Integration abnehmen: tatsächliche Desktop-/CLI-/Mobile-Kommandos mit
  Neustart, kaputtem Saved-Store, private Regularfile/Lock/Stage und CAS-Konflikt; V5 beendet betroffene
  schreibende Sessions/Jobs nach Einschränkung. Andere Remotes mit identischem relativen Pfad bleiben
  unverändert. CLI `--write` zusammen mit `--read-only` wird abgewiesen. Mobile wartet auf die bestehende
  Worker-Zustellung; `persisted=true` belegt die Dateiänderung, kein synchrones Transportergebnis.

## Gelesene Dateien

Unter `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/`:
`scopes/s-policy.json`, `spec.md`, `umsetzung.md`, `abnahme/S-REVOKE.md`, `anfragen/S-REVOKE.md`,
`api-delta/S-REVOKE.md`, `anfragen/S-SIGNAL.md`, `api-delta/S-SIGNAL.md` sowie die eigenen drei
S-POLICY-Berichte. AGENTS/Architektur und arbeitsweise waren aus dem vorhandenen Auftrag geladen.

Exakte native Read-Pfade (einschließlich der eigenen geschaffenen Dateien aus der nächsten Liste):

- `native/src/share/mod.rs`
- `native/src/share/core/profiles.rs`
- `native/src/share/core/profile_persistence.rs`
- `native/src/share/core/export_config.rs`
- `native/src/share/core/direct_ledger_validation.rs`
- `native/src/share/core/legacy_direct_request_validation.rs`
- `native/src/share/core/direct_relation.rs`
- `native/src/share/core/relation_rights.rs`
- `native/src/share/core/removed_direct_peers.rs`
- `native/src/share/core/room_relation.rs`
- `native/src/share/core/room_relation_members.rs`
- `native/src/share/core/types.rs`
- `native/src/share/os/shared/profile_store.rs`
- `native/src/share/os/shared/profile_transaction.rs`
- `native/src/share/os/shared/profile_operations.rs`
- `native/src/share/os/shared/profile_edits.rs`
- `native/src/share/os/shared/direct_actions.rs`
- `native/src/share/os/shared/direct_policy_store.rs`
- `native/src/share/os/shared/direct_relation_actions.rs`
- `native/src/share/os/shared/direct_repair_store_adapter.rs`
- `native/src/creds/mod.rs`
- `native/src/creds/core/types.rs`
- `native/src/creds/os/shared.rs` (nur der freigegebene Loader-/Persistenzabschnitt)
- `native/src/support_dirs.rs` (Signaturen)
- `native/src/app/core/share.rs`
- `native/src/app/core/share_direct_ui.rs` (Share-back-Aufrufer)
- `native/src/app/core/share_exports_ui.rs`
- `native/src/app/core/share_helpers.rs`
- `native/src/app/core/share_profile_cache.rs`
- `native/src/app/core/share_profile_edits.rs`
- `native/src/app/core/share_rooms_ui.rs`
- `native/src/cli/share.rs` (eigene Registrierungen/Helfer)
- `native/src/cli/share/exports.rs`
- `native/src/cli/share/grants.rs`
- `native/src/mobile/core/config.rs`
- `native/src/mobile/os/shared/domains/mod.rs` (eigene Registrierungen)
- `native/src/mobile/os/shared/domains/share_peers.rs`
- `native/src/mobile/os/shared/domains/share_status.rs`
- `native/src/mobile/os/shared/domains/share_state.rs` (bestehende Commit-/Reconfigure-Grenze)

## Geänderte / geschaffene Dateien

Geändert (vorhandene Teiländerungen fortgeführt, keine fremde Orchestrierung verändert):

- `native/src/share/core/profiles.rs`
- `native/src/share/core/profile_persistence.rs`
- `native/src/share/os/shared/profile_store.rs`
- `native/src/share/os/shared/profile_operations.rs`
- `native/src/share/os/shared/profile_edits.rs`
- `native/src/app/core/share.rs`
- `native/src/app/core/share_exports_ui.rs`
- `native/src/app/core/share_helpers.rs`
- `native/src/app/core/share_rooms_ui.rs`
- `native/src/cli/share/exports.rs`
- `native/src/cli/share/grants.rs`
- `native/src/mobile/core/config.rs` (Home-Kommentar)
- `native/src/mobile/os/shared/domains/share_peers.rs`
- `native/src/mobile/os/shared/domains/share_status.rs`
- `native/src/share/mod.rs`, `native/src/cli/share.rs`,
  `native/src/mobile/os/shared/domains/mod.rs` (nur eigene additive Registrierung/Reexport/Aliase)

Geschaffen, jeweils unmittelbar neben zugeordneten Dateien und kohäsiv nach Verantwortung:

- `native/src/share/core/profile_migration.rs`
- `native/src/share/core/profile_persistence_tests.rs` (bestehende Tests aus der vorher übergroßen Persistenzdatei ausgelagert und FC1-Signale ergänzt)
- `native/src/share/core/profile_policy.rs`
- `native/src/share/core/profile_policy_tests.rs`
- `native/src/share/os/shared/profile_export_edits.rs`
- `native/src/share/os/shared/profile_policy_actions.rs`
- `native/src/app/core/share_connections_ui.rs`
- `native/src/app/core/share_policy_ui.rs`
- `native/src/cli/share/exports_policy.rs`
- `native/src/cli/share/exports_connections.rs`
- `native/src/cli/share/grants_write.rs`
- `native/src/mobile/os/shared/domains/share_policy.rs`
- `native/src/mobile/os/shared/domains/share_policy_status.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/S-POLICY.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/S-POLICY.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/S-POLICY.md`

Offene Owner-Integrationen stehen ausschließlich im eigenen Anfragenbericht; keine globale
Plan-/Boardänderung und keine Änderung am abgeschlossenen A-CLIENT-Delta.
