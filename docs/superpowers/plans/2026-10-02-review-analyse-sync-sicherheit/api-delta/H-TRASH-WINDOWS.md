# H-TRASH-WINDOWS – API-Delta

Stand: 2026-10-03. Ausschließlich der vorhandene Windows-FA6/A34-Anschluss.
Keine Drahtschema-, Exportrechts-, Lease-, Broker- oder Synchronisationsänderung.

## Neue interne Host-Fassade

Registrierung in `native/src/lib.rs`: `pub(crate) mod host_trash;`.
Die OS-Auswahl und beabsichtigten Reexports liegen in `native/src/host_trash/mod.rs`.

```rust
host_trash::available() -> bool
host_trash::list(cursor: Option<&str>) -> io::Result<CatalogPage>
host_trash::restore(id: &str) -> io::Result<RestoreOutcome>
host_trash::recycle_selected(
    root: &Path, parent: &DirectoryHandle, file: &File,
    expected: &RecycleExpectation,
) -> io::Result<RecycleOutcome>
```

Diese APIs sind `pub(crate)`. Der Aufrufer von `recycle_selected` ist ausschließlich die
ordinary/confined Auswahl in `analytics/os/windows.rs`; Originalpfad/Name werden aus
dem geöffneten Objekt verlustlos ermittelt. Ein frei übergebener Quellpfad autorisiert
keinen Capture. Nicht-Windows-`host_trash::available()` ist false; die übrigen Fassade-
Operationen melden Unsupported. Bestehende Linux/Android-Papierkorb-Adapter bleiben erhalten.

Neue portable Katalogmodelle (`host_trash/core/record.rs`, `pub(crate)`):

- `CatalogPage { entries: Vec<CatalogEntry>, next: Option<String>, issues: Vec<String>, suppressed_issues: u64 }`
- `CatalogEntry { id: String, original: String, size: Option<u64>, created_ms: Option<i64>, state: EntryState, detail: Option<String> }`
- `CatalogEntry::can_restore(&self) -> bool`
- `EntryState::{Held, RestorePending, OriginalPresent, Missing, Problem}`,
  `EntryState::label(self) -> &'static str`
- `RestoreOutcome::{Restored, AlreadyAtOriginal}`

`list` liefert höchstens die aktuelle Seite; `next` ist der Anker für ältere IDs.
„Am Originalort“ bezeichnet die bestätigte Objektposition, keinen unangeforderten
SHA-Nachweis einer nach Wiederherstellung möglicherweise lokal bearbeiteten Datei.
Einzelne defekte/unvollständige Records werden als Problem ohne Restoreaktion gezeigt.
Der Store hält keine Payloadkopien und bietet keinen Permanent-Delete.

Interne Persistenz (`pub(super)`, nicht für fremde Module reexportiert):
`Store::open()/at(&Path)`, `persist(&Record)`, `load(&str)`;
`catalog::list_in(&Store, Option<&str>)` und
`restore::restore_in(&Store, &Record)`.
`Record` V1 speichert rohe UTF-16-Wurzel/relative Komponenten, Root-/Dateiidentität
(Volume + 128 Bit), Länge/SHA, Erstellzeit und zwei exakte Held-Namen.
ID ist 32 lowerhex, Held-Namen sind `.held.se-recycle-<16lowerhex>`.
Records sind private, exklusiv reservierte, immutable `<id>.json` unter
`support_dirs::app_data_dir()/host-trash`; `operation.lock` serialisiert alle Store-
Operationen über GUI/Host-Prozesse desselben Benutzers.

## Additiver Windows-Quarantänevertrag

Nur `native/src/local_access/os/windows/quarantine.rs`:

```rust
DirectoryHandle::checked_quarantine_slot(name: &OsStr)
    -> io::Result<QuarantineSlot>
QuarantineSlot::name(&self) -> &OsStr
DirectoryHandle::quarantine_regular_child_in(
    &self, name: &OsStr, expected: &File, slot: &QuarantineSlot,
) -> io::Result<QuarantinedChild>
```

`QuarantineSlot` ist ein `pub(crate)` opaque Typ im bestehenden Windows-Kindmodul.
Kein neuer globaler Reexport; der Rückgabewert wird über die Methode inferiert und benutzt.
Die Validierung akzeptiert ausschließlich einen einzelnen exakten lowercase-Held-Namen,
keine freien Pfade/Varianten. Der bestehende random-slot-Aufruf bleibt unverändert.
Capture öffnet den bestätigten DELETE-Guard, vergleicht Volume/128-Bit-FileID und benennt
genau dessen Objekt per NoReplace um. FILE_SHARE_READ, Original-ACL, bestehender
`QuarantinedChild::{file,restore,move_to,retained_location}`-Vertrag bleiben erhalten.

Der Hostadapter publiziert den vollständigen Intent **vor** diesem Methodenaufruf mittels
der bestehenden `support_dirs::write_private_atomic`-Fassade. Ein Windows-
`sync_directory`-No-op wird nicht als Namespace-Durability ausgegeben.
Restore benutzt nur den im selben Record bereits gespeicherten zweiten Held-Slot.
Jeder Fehler erhält Inhalt/Record; es gibt keinen zufälligen unprotokollierten Restore-Hop.

## Analytics und Host-UI

`analytics/os/shared/checked_recycle.rs` bietet die schmale `pub(super)` Auswahlhilfe:

```rust
with_regular_child<T>(
    root: &Path, path: &Path,
    selected: impl FnOnce(
        &Path, &Path, &DirectoryHandle, &OsStr, &File,
    ) -> io::Result<T>,
) -> io::Result<T>
```

Das bisherige `recycle(..., publish)` wird über dieselbe Hilfe ausgeführt; sein
Linux/Android-Matches-/Capture-/Publish-/Restore-Verhalten bleibt erhalten.
Windows wählt dort ordinary/confined und ruft die neue Host-Fassade auf.

Unveränderte öffentliche interne Analytics-Verträge:

```rust
analytics::recycle_local(root, path, expected) -> io::Result<RecycleOutcome>
analytics::host_recycle_available() -> bool
```

Windows `host_recycle_available` delegiert an die vollständige Host-Fassade. Die
vorhandene `FsHostFeatures::host().remote_trash_v1`-OS-Maske bleibt unverändert;
relationbezogene Schreibrechte werden weiterhin außerhalb dieses Blocks begrenzt.

Additive Registrierung in `app/core/share.rs`:
`#[path = "share_host_trash_ui.rs"] mod host_trash_ui;`.
`App::ui_share_host_trash(&mut self, &mut egui::Ui)` ist `pub(super)`.
`share_window_ui.rs` erhält ausschließlich den verfügbaren Windows-Host-Reiter
„Papierkorb“ auf Index 5 und den Aufruf dieses Consumers. Kein neuer App-Zustands-/Config-
Persistenzvertrag; die kleine UI-Snapshot-/Busy-Verwaltung liegt in Context-Tempdaten.
OS-Arbeit läuft im benannten Hintergrundworker, Nachrichten/Repaint erhalten Retrybarkeit.

## Exakte Remote-Abnahmesymbole

Alle folgenden Signale gehören in die eine gemeinsame Remote-Suite; keine lokale
Ausführung. Windows-Unitmodul wird aus `host_trash/mod.rs` über
`#[cfg(all(windows, test))]` registriert; der gerenderte Consumer liegt im vorhandenen
App-Modul. Die Windows-Link-Fixture benötigt die etablierten V-LOCAL-Symlink-Rechte.

- `host_trash::review_task_tests::review_task_host_trash_intent_precedes_capture_and_survives_restart`
- `host_trash::review_task_tests::review_task_host_trash_restore_restart_hop_has_durable_mapping`
- `host_trash::review_task_tests::review_task_host_trash_restore_preserves_collision_and_is_retryable`
- `host_trash::review_task_tests::review_task_host_trash_changed_payload_is_never_restored`
- `host_trash::review_task_tests::review_task_host_trash_failed_intent_and_expected_hash_leave_source_untouched`
- `host_trash::review_task_tests::review_task_host_trash_replaced_root_is_not_a_restore_target`
- `host_trash::review_task_tests::review_task_host_trash_link_source_and_invalid_slots_are_refused`
- `host_trash::review_task_tests::review_task_host_trash_private_records_and_partial_intents_keep_other_entries`
- `host_trash::review_task_tests::review_task_host_trash_catalog_pages_without_dropping_intents`
- `host_trash::review_task_tests::review_task_host_trash_file_reservation_serializes_other_owners`
- `app::share::host_trash_ui::tests::review_task_host_trash_page_exposes_visible_restore_consumer`

## Dateibericht und Fremdgrenzen

Die vollständige tatsächliche Lese-/Änderungs-/Erstellungsliste steht in
[abnahme/H-TRASH-WINDOWS.md](../abnahme/H-TRASH-WINDOWS.md#exakter-dateibericht).
Neu benötigt werden genau `host_trash/mod.rs`, `core/record.rs`,
`os/{windows,unsupported}.rs`, `os/shared/{store,catalog,restore,review_task_tests}.rs`
sowie `app/core/share_host_trash_ui.rs`; keine weiteren Registrierungen/Abhängigkeiten.

Offene Fremdaufgaben sind ausschließlich die Integration, globale Statusfortschreibung
und Laufabnahme durch den Hauptagenten:
[anfragen/H-TRASH-WINDOWS.md](../anfragen/H-TRASH-WINDOWS.md).
Keine ungelöste Quarantäne-/Storage-/UI-API innerhalb dieses Outcomes.
