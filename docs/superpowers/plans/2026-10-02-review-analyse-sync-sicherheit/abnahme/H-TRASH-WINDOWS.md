# H-TRASH-WINDOWS – Umsetzung und Abnahme

Stand: 2026-10-03. Begrenzter Anschluss des dokumentierten FA6-/A34-Rests nach H-ANALYSIS
5098ee0 und V-LOCAL cd8632d; kein neues Projekt-Review. Scope:
`scopes/h-trash-windows.json`. Arbeitsweise bleibt verbindlich; ausschließlich statische Arbeit.
Die eine abschließende Remote-Suite und Integration bleiben beim Hauptagenten.

## Stage two vor Quelländerungen

Grundlage der ersten Recherche sind die vorhandenen Quarantäne-/Storage-Verträge und die
FA6-Ergänzung in `umsetzung.md`. Die zweite Recherche ist in
`docs/refs/windows-checked-recycle.md` gesichert: ein freier Shellpfad bietet keinen hier
verlangten bestätigten Handle-/Originalortvertrag. Die vorhandenen Windows-Quellen bestätigen
128-Bit-FileID + Volume, FILE_SHARE_READ, NoReplace und expliziten Restore. Die private
Storage-Fassade bietet exklusive Owner-Dateien, bounded Reads und synchrone Dauerhaftigkeit.

Letzter konkreter Research-Abgleich vor Abschluss: Der Hauptagent bestätigt, dass Windows
`sync_directory` bewusst keine NTFS-Directory-Flush-Garantie liefert. Intent-Persistenz nutzt
deshalb nach exklusiver ID-Reservierung genau `support_dirs::write_private_atomic`
(private Stage + file.sync_all + vorhandene V1-Write-through-Promotion), niemals einen
behaupteten Directory-Flush. Zusätzliche Primärsyntax geprüft am 2026-10-03:
[GetFinalPathNameByHandleW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfinalpathnamebyhandlew),
[CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew) und
[FlushFileBuffers](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers).
Originalpfade werden aus dem geöffneten Handle in exakten UTF-16-Einheiten gespeichert;
die Windows-Sharingreservierung bleibt bis zum Handle-Drop wirksam. Fehlende sichere
Provider-Garantien führen zu einem Fehler vor Capture, ohne unsicheren Pfadrückfall.

| Meilenstein | Zugeordnete Dateien | Konkretes erwartetes Ergebnis für die gemeinsame Remote-Suite |
| --- | --- | --- |
| Typed Record und gewählter Quarantäneslot | `host_trash/core/record.rs`, `local_access/os/windows/quarantine.rs` | Versionierter, validierter Record besitzt exakte UTF-16-Wurzel/Child-Komponenten, Datei-/Rootidentität, Länge/SHA und beide vorher gewählten Held-Namen. Ungültige Slots werden vor Öffnen/Rename abgewiesen. Bestehender Zufallsslot bleibt erhalten. |
| Intent vor erstem Capture | `checked_recycle.rs`, `analytics/os/windows.rs`, `host_trash/os/shared/store.rs`, `host_trash/os/windows.rs` | Quelle wird ordinary/confined gewählt und Länge/SHA geprüft. Privater Record ist exklusiv vollständig geschrieben/gesynct, bevor derselbe bestätigte Guard in den gespeicherten Slot umbenennt. Changed/Link/Intentfehler verschieben nichts. Nach Capture wird Inhalt erneut geprüft. |
| Restart-Katalog und Restore | `host_trash/os/shared/catalog.rs`, `restore.rs`, Windows-Adapter | Begrenzte paginierte Auflistung findet auch unvollständige Intents und unterbrochenes Restore. Root-/Dateiidentität wird frisch geprüft, Kinder werden handlegebunden geöffnet. Restore hat einen schon vorher gespeicherten zweiten Slot und ersetzt keine Original-/Held-Datei. Einzelprobleme verlieren andere Einträge nicht. |
| Sichtbare Host-Bedienung und Wahrheit der Fähigkeit | `app/core/share_host_trash_ui.rs`, `share_window_ui.rs`, additive `share.rs`/`lib.rs`-Registrierung | Im bestehenden Host-Sharefenster ist der Smart-Explorer-Papierkorb mit Originalort, Zustand, Aktualisieren, Pagination und Wiederherstellen sichtbar. I/O läuft im Hintergrund; Fehler sind sichtbar/retrybar. Windows bietet remote_trash_v1 erst mit vollständiger Fassade an; kein nativer Windows-Bin-Claim. |
| Eigener Self-Review und vorbereitete Signale | `host_trash/os/shared/review_task_tests.rs`, eigene drei Berichte | Erwartetes Objekt/Hash, Vor-Capture-Intent, Neustartpositionen, Restore-Konflikt/Linktausch, private Records und sichtbarer UI-Anschluss sind der einzigen Remote-Suite zugeordnet. Keine lokale Ausführung. |

Entscheidungen: Payload bleibt auf dem ursprünglichen Dateisystem im vorhandenen präzisen
`.held.se-recycle-<16lowerhex>`-Namensraum; unter Appdata liegen ausschließlich private
Recovery-Metadaten. Beide möglichen Held-Positionen werden vor dem ersten Capture gespeichert,
sodass Restore keine unauffindbare Zwischenposition erzeugt. Records bleiben erhalten und der
Katalog leitet den aktuellen Zustand aus den geprüften Positionen ab. Ein Dateihandle mit
exklusiver Windows-Schreibreservierung serialisiert Capture/Restore/Katalog über Prozesse;
bei Busy bleibt eine explizite Retry-Meldung. Linux/Android-Recycle wird durch die
wiederverwendbare Auswahlhilfe semantisch erhalten.

Kompatibilität: Daten-Reparse-Dateien behalten ihre vorhandene reguläre Klassifikation;
redirecting Childlinks/Spezialdateien bleiben Grenzen. Kein broker/elevated Host-Zugriff,
keine Rechteausweitung, kein Permanent-Fallback, keine native Shelloperation auf einem freien
Quarantänepfad, keine Änderungen an bestehenden lokalen Systempapierkorb-Operationen oder an
fremden Share-/Sync-/Android-Flächen. Gemeinsame Stage-/Scan-/Rechtefilter bleiben erhalten.

## Ergebnis und Abnahmesignale

- FA6/A34: Windows-Remote-Recycle hat einen eigenen sichtbaren und reversiblen Host-Papierkorb. Der bestätigte Inhalt bleibt als Datei im ursprünglichen Parent auf demselben Dateisystem. Appdata/host-trash enthält ausschließlich private, immutable JSON-Records und die Prozessreservierung.

- Vor dem ersten Rename sind Originalwurzel und Child-Komponenten als verlustlose UTF-16-Einheiten, Volume + vollständige 128-Bit-Root-/Dateiidentität, Länge, volle SHA-256 sowie beide exakten Held-Namen dauerhaft publiziert. Der Record wird nach EXCL-ID-Reservierung mit der vorhandenen privaten Atomic-Fassade geschrieben. Eine unvollständige Reservierung ist sichtbar, autorisiert aber keinen Capture.

- Ordinary/confined Auswahl, Inhalt vor Capture, bestätigter DELETE-Guard, FILE_SHARE_READ und Inhalt unmittelbar nach Capture binden die Operation an das erwartete Objekt. Chosen-slot ist eine additive opaque, exakt validierte API; der bisherige Zufallsslot und sämtliche V-LOCAL-NoReplace-/Restore-Invarianten bleiben erhalten.

- Restart-Auflistung prüft Wurzel-/Dateiidentität und Länge an beiden dokumentierten Held-Positionen. Restore prüft zusätzlich die volle SHA vor und nach dem erneuten bestätigten Einfang und ersetzt weder Held-Ziel noch Originaldatei. Ein Abbruch im Restore-Hop bleibt durch den schon gespeicherten zweiten Slot auffindbar. Bei Rückstellkonflikt bleiben Inhalt und Record erhalten; Fehler nennt die Record-ID.

- Der lokale Host-Share-Reiter „Papierkorb“ (neuer Index 5; bisherige Reiter unverändert) zeigt Originalort, Zustand, Größe, Datum, Aktualisieren, ältere Seiten und Wiederherstellen. Dateisystemarbeit läuft außerhalb des UI-Threads; Busy-, I/O-, Inhalts- und Konfliktfehler sind sichtbar und wiederholbar. Peer-Clients erhalten keinen Katalogzugriff.

- Der Katalog hält nur die aktuelle Seite plus Nachfolgeanker und begrenzte Eintragsprobleme im Speicher. Alle Records bleiben gespeichert; es gibt keinen neuen Papierkorbeintrag-Deckel oder automatischen Purge. Die Record-Read-Grenze folgt den tatsächlichen 32767 Windows-Pfadeinheiten plus JSON-/Metadaten-Overhead. SHA wird bei schreibenden Aktionen vollständig geprüft; die Auflistung behauptet anhand von Identität/Länge keinen eigenen vollständigen Inhaltsnachweis.

- analytics::host_recycle_available bleibt die zentrale OS-Maske: Windows delegiert jetzt an host_trash::available. Die vorhandene FsHostFeatures-Maske konsumiert diese API unverändert. Es wird ein Smart-Explorer-Host-Papierkorb angeboten, keine Integration in den nativen Windows-Systempapierkorb behauptet.

Vorbereitete Signale für **die eine abschließende Remote-Task-Suite**; sie wurden lokal
weder kompiliert noch ausgeführt:

| Erwartetes Verhalten | Exaktes Testsymbol |
| --- | --- |
| Intent vor Capture / Restart | `host_trash::review_task_tests::review_task_host_trash_intent_precedes_capture_and_survives_restart` |
| Restore-Hop nach Restart | `host_trash::review_task_tests::review_task_host_trash_restore_restart_hop_has_durable_mapping` |
| Konflikt, Erhaltung und Retry | `host_trash::review_task_tests::review_task_host_trash_restore_preserves_collision_and_is_retryable` |
| Veränderte Inhalte | `host_trash::review_task_tests::review_task_host_trash_changed_payload_is_never_restored` |
| Fehlgeschlagener Intent / erwartete SHA | `host_trash::review_task_tests::review_task_host_trash_failed_intent_and_expected_hash_leave_source_untouched` |
| Ersetzte Wurzel | `host_trash::review_task_tests::review_task_host_trash_replaced_root_is_not_a_restore_target` |
| Childlink und ungültiger Slot | `host_trash::review_task_tests::review_task_host_trash_link_source_and_invalid_slots_are_refused` |
| Private Records / Teil-Intent / unabhängige Einträge | `host_trash::review_task_tests::review_task_host_trash_private_records_and_partial_intents_keep_other_entries` |
| Pagination ohne Intentverlust | `host_trash::review_task_tests::review_task_host_trash_catalog_pages_without_dropping_intents` |
| Prozessübergreifende Reservierung | `host_trash::review_task_tests::review_task_host_trash_file_reservation_serializes_other_owners` |
| Tatsächlich gerenderter Wiederherstellungsconsumer | `app::share::host_trash_ui::tests::review_task_host_trash_page_exposes_visible_restore_consumer` |

Die Link-Fixture benötigt auf dem Windows-Runner die bereits für V-LOCAL vorgesehenen
Symlink-Rechte/Developer Mode. Ein fehlendes Recht wird als Fixture-Fehler sichtbar;
kein stilles Überspringen. Die Fixtures benutzen isolierte private Stores und Ordinary-Handles.
Der UI-Nachweis rendert den tatsächlichen Consumer mit einem gehaltenen Eintrag; die
gemeinsame Suite verifiziert zusätzlich den vollständigen Host-/Share-/UI-Aufruferweg.

Eigener Self-Review: Vorher-/Nachher-Inhaltsbindung, Intent-vor-Capture-Reihenfolge,
Abbruchpositionen, Record-Isolation, frische Root-/Dateiidentität, NoReplace und additive
Registrierungen wurden am eigenen Diff abgeglichen. `git diff --check` sowie eine
statische Rust-Kommentar-/String-/Delimiterprüfung ergeben keine Text-/Klammerfehler.
Alle neuen und betroffenen Rust-Dateien bleiben unter 500 Zeilen/50 KiB; Core und Shared
besitzen keine direkten OS-Imports/FFI. Das ist **keine** Compiler-, Laufzeit- oder
Power-loss-Abnahme. Sämtliche Laufbeweise bleiben beim Hauptagenten in der einen Remote-Suite.
Für den gerenderten Egui-Nachweis wurde die 0.29.1-API `Galley::text()` in der
[offiziellen Quelle](https://github.com/emilk/egui/blob/0.29.1/crates/epaint/src/text/text_layout_types.rs)
am 2026-10-03 abgeglichen.

Grenzen: Eine extern verschobene/ersetzte Wurzel, fehlende Provider-Identität, entfallene
Ordinary-Rechte oder manipulierte Metadaten werden sichtbar abgewiesen, ohne anderes Objekt
zu verändern. Inhalte behalten ihre ursprüngliche ACL; Records gehören dem effektiven
Hostbenutzer/Datenprofil. Es gibt keinen neuen Broker, keine fremde Account-Migration und
keine automatische Suche nach extern verschobenen Wurzeln. Ein inhaltlich veränderter Held
bleibt erhalten und wird beim Restore abgewiesen. Erfolgreich wiederhergestellte Records
bleiben als „Am Originalort“ nachvollziehbar; kein nicht angeforderter Permanent-Delete.

## Exakter Dateibericht

Gelesen (teilweise gezielte Symbol-/Abschnittslesung); AGENTS.md wurde zusätzlich als
Nutzeranweisung übernommen, Architektur/Skills stammen aus dem vorausgehenden H-ANALYSIS-
Kontext und wurden nicht erneut breit erkundet:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/h-trash-windows.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/umsetzung.md`
- `docs/refs/windows-checked-recycle.md`
- `native/src/local_access/os/windows/quarantine.rs`
- `native/src/local_access/os/windows/directory_handle.rs`
- `native/src/local_access/os/windows/create.rs`
- `native/src/local_access/os/windows/private_security.rs`
- `native/src/analytics/os/windows.rs`
- `native/src/analytics/os/shared/checked_recycle.rs`
- `native/src/analytics/os/mod.rs`
- `native/src/app/core/share.rs`
- `native/src/app/core/share_window_ui.rs`
- `native/src/app/core/share_exports_ui.rs`
- `native/src/creds/mod.rs`
- `native/src/support_dirs.rs`
- `native/src/lib.rs`
- `native/src/app/mod.rs`
- `native/Cargo.toml`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/V-LOCAL.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/V-LOCAL.md`
- `native/src/host_trash/mod.rs`
- `native/src/host_trash/core/record.rs`
- `native/src/host_trash/os/shared/store.rs`
- `native/src/host_trash/os/shared/restore.rs`
- `native/src/host_trash/os/shared/catalog.rs`
- `native/src/host_trash/os/windows.rs`
- `native/src/host_trash/os/unsupported.rs`
- `native/src/host_trash/os/shared/review_task_tests.rs`
- `native/src/app/core/share_host_trash_ui.rs`
- `native/src/analytics/mod.rs`
- `native/src/local_access/mod.rs`
- `native/src/vfs/core/staging_names.rs`
- `native/src/share/core/wire_capabilities.rs`
- `native/src/vfs/core/extension_types.rs`
- `native/src/vfs/mod.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/H-TRASH-WINDOWS.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/H-TRASH-WINDOWS.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/H-TRASH-WINDOWS.md`

Erstellt:

- `native/src/host_trash/mod.rs`
- `native/src/host_trash/core/record.rs`
- `native/src/host_trash/os/shared/store.rs`
- `native/src/host_trash/os/shared/restore.rs`
- `native/src/host_trash/os/shared/catalog.rs`
- `native/src/host_trash/os/windows.rs`
- `native/src/host_trash/os/unsupported.rs`
- `native/src/host_trash/os/shared/review_task_tests.rs`
- `native/src/app/core/share_host_trash_ui.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/H-TRASH-WINDOWS.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/H-TRASH-WINDOWS.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/H-TRASH-WINDOWS.md`

Geändert:

- `native/src/local_access/os/windows/quarantine.rs`
- `native/src/analytics/os/windows.rs`
- `native/src/analytics/os/shared/checked_recycle.rs`
- `native/src/app/core/share_window_ui.rs`
- `native/src/lib.rs`
- `native/src/app/core/share.rs`

`lib.rs` enthält ausschließlich meine additive `host_trash`-Registrierung;
`share.rs` ausschließlich meine additive `host_trash_ui`-Registrierung. Gleichzeitig
sichtbare fremde Änderungen wurden nicht bearbeitet. Die aufgeführte
`api-delta/S-LOCAL.md` war beim gezielten Zugriff nicht vorhanden; der Hauptagent hat den
konkreten privaten Atomic-/Write-through-Vertrag schriftlich bestätigt. Keine zusätzliche
Scopefläche wurde erkundet.

Fremde Integrations-/Abnahmegrenzen sind in [anfragen/H-TRASH-WINDOWS.md](../anfragen/H-TRASH-WINDOWS.md)
aufgeführt; exakte APIs in [api-delta/H-TRASH-WINDOWS.md](../api-delta/H-TRASH-WINDOWS.md).
Der eigene Umsetzungsblock ist abgeschlossen. Integration, Commit/Push, Graph und
Remote-Suite werden ausschließlich vom Hauptagenten übernommen.
