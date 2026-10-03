# V-LOCAL – Fortführung K1

Stand: 2026-10-03. Umsetzung im vorhandenen RV1-Plan; keine neue Review-Kampagne.
Die bestehende Abnahme in [K1.md](K1.md) bleibt Grundlage. Keine lokalen Builds, Compiler,
Formatierer oder Testläufe. Die endgültige Ausführungsabnahme besitzt die eine Remote-Task-Suite.

## Umsetzungsplan und Annahmen

| Meilenstein | Fläche | Erwartetes Ergebnis |
|---|---|---|
| V1-Vertrag und vorhandene lokale Grundlagen schließen | `vfs`, `local_access`, `android_fs`, `types`, lokales `copy` | Die dokumentierten sicheren Rückfälle, Spezialdatei-/Reparse-Klassen, Namen, Veröffentlichungsleiter und Zielgrenzen bleiben erhalten. |
| Sicher öffnen und Zwischendateien nachbearbeiten | lokale OS-Adapter, `local_stage`, `copy`-Staging | Link-/Spezialdateitausch öffnet keinen fremden Inhalt und blockiert nicht; Quellrechte sind vor der Veröffentlichung geschützt. Windows-Daten-Reparse-Dateien bleiben lesbar. |
| Volume- und Mountgrenzen vollständig bestimmen | `mountinfo`, Linux-/Windows-Volume-Adapter | Fehlende Zielwurzeln folgen vorhandenen Vorfahren; Bind-/Pseudo-/Netz-/FUSE-Einhängungen werden als Grenzen erkannt; unbekannte Identitäten bleiben unbekannt. |
| Sichere Host-Aufzählung ohne Kanonisierung pro Unterordner | `local_access`-Verzeichnishandles | Eine gewählte Wurzel bleibt verankert; jedes Kind wird ohne Linkfolgen geöffnet, Aufzählungen behalten die Originalnamen und Fehlerkontexte. |
| Recycle-Einfangen und private Info-Reservierung (Abhängigkeit H-ANALYSIS) | `local_access`-OS-Adapter | Das erwartete Child bleibt handlegebunden; Restore/Trash-Handoff ersetzt nichts. Records entstehen exklusiv mit 0600/0700 oder verifizierter geschützter Windows-Besitzer-DACL. |
| Partielle Watch-Abdeckung (Abhängigkeit T-JOBS/A-CLIENT) | `vfs/core/extension_types.rs` | `ReadyPartial` hält die Abfrage aktiv; nur `Ready` bedeutet vollständige Abdeckung. |
| Self-Review und Remote-Abnahmesignale | eigene Quellen und bestehende K1-Tests | APIs gegen gesicherte Refs/Primärquellen abgleichen, Rust-Quellen statisch parsen, gezielte `review_task_`-Signale an den Hauptagenten übergeben. |

Kompatibilität: Die gewählte Wurzel darf Links/Junctions oberhalb und an der Wurzel benutzen;
Links darunter bleiben geschützte Auslassungen. Explorer-Lesen folgt gewählten Dateilinks,
Walk-/Host-Lesen nicht. OneDrive/WOF/Dedup sind reguläre Dateien, Spezialdateien keine Datenströme.
Lokale/UNC-Identitäten und gespeicherte Endpunktbedeutungen werden nicht verändert.
Abbruch, Quarantäne-Rückkehr und Konfliktverhalten des vorhandenen Copy-Flows bleiben erhalten.

Recherche: `docs/refs/local-fs-identity-durability.md` (Stand 2026-10-02),
`spec.md` FS4–FS6 und `umsetzung.md` V1/K1. Für den zusätzlichen verankerten Unix-Iterator
am 2026-10-03 geprüfte Primärquellen: [open(2)](https://man7.org/linux/man-pages/man2/open.2.html),
[fdopendir(3)](https://man7.org/linux/man-pages/man3/fdopendir.3.html),
[readdir(3)](https://man7.org/linux/man-pages/man3/readdir.3.html).
Die ergänzten Recycle-/ACL-Primitiven wurden am selben Tag gegen
[rename(2)](https://man7.org/linux/man-pages/man2/rename.2.html),
[FILE_RENAME_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info),
[SetSecurityDescriptorDacl](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-setsecuritydescriptordacl),
[CreateDirectoryW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createdirectoryw)
und die [windows-sys-0.59-Bindings](https://raw.githubusercontent.com/microsoft/windows-rs/0.59.0/crates/libs/sys/src/Windows/Win32/Security/mod.rs)
abgeglichen. Keine neue Abhängigkeit; die benötigten Win32-Features stehen bereits in `native/Cargo.toml`.

## Ergebnis

Die zugewiesene native Umsetzung ist abgeschlossen und für die Hauptintegration bereit.
Die vorhandenen K1-Grundlagen wurden fortgeführt; ihre Prüf- und Kompatibilitätsverträge bleiben
Bestandteil der einen endgültigen Remote-Task-Suite. Ausführungsabnahme und vollständiger Root-Graph
liegen beim Hauptagenten; sie wurden in diesem Block nicht ausgeführt.

| Befund / Vertrag | Konkrete Umsetzung und Grenze |
|---|---|
| FS6, Y86 | Die vorhandene NOREPLACE-Leiter bleibt erhalten: atomare Umbenennung, Hard-Link-Stufe, geprüfte pfadbasierte letzte Stufe ausschließlich für eigene Stages. Recycle benutzt hingegen nur echtes atomares NOREPLACE; fehlende Provider-Unterstützung gibt `Unsupported`. |
| Y87/B25, FA4 | Regulär-Lesen klassifiziert und prüft das geöffnete Objekt. Unix benutzt NOFOLLOW/NONBLOCK/NOCTTY und fstat; Windows unterscheidet Name-Surrogate-/Spezial-Tags von Daten-Reparse. Move-Snapshots, Copy-Identitätsprüfung und lokaler Einzeldatei-Lister behandeln FIFO/Socket/Device als Auslassung. |
| Y94/Y122 | Gewählte Root-Links oberhalb/an der Wurzel bleiben erlaubt; Childs bleiben Grenzen. Relative mkdir-Ziele werden absolut verankert. Der frühere ungesicherte Pfad-Cache wurde durch erneute Prüfung ersetzt, damit ein Directory→Link-Tausch nicht übernommen wird. |
| Y95, A08/A17 | Neuer DirectoryHandle-Vertrag: Unix FD-relative Childs und eigene Enumerationspositionen; Windows gepinnte physische Vorfahren, keine Kanonisierung pro Child. Für alte pfadbasierte mkdir-/Schreiboperationen bleibt die RootGuard-Integration samt sicherer Cache-/Pin-Lebensdauer eine Consumer-Grenze; Y95 ist dort nicht pauschal als abgeschlossen zu lesen. |
| Y98/B24 | Windows-Neuanlage prüft jeden normalen Pfadteil, besonders ADS-Doppelpunkte. Reservierte/literale bestehende Namen bleiben über Verbatim-Pfade erreichbar; Zielgrenzen melden ihre Übertragbarkeit. Unbekanntes Windows-Dateisystem behält Windows-Namensgrenzen. |
| Y99 | Cloud/WOF/Dedup bleiben reguläre Daten-Dateien in LocalBackend, Copy, Stage-Nachbearbeitung und Broker. Für private App-/IPC-Objekte gilt die strengere Regel: überhaupt kein Reparse-/Device-Objekt. |
| Y100 | Readonly-Ersetzen bleibt erhalten. Readonly-Windows-Stages bekommen für Metadaten einen geprüften Schreib-Handle; das Readonly-Attribut wird dabei sofort zurückgesetzt und auf dem Ergebnis erhalten. |
| Y81/Y103 | Unix-Kopie/Copy-/Server-Copy beginnen mit 0600-Stage; Unix-Endmodus ist Quellrechte ∩ bestehende Zielrechte, begrenzt auf 0777. Keine Set-ID-Übernahme; Windows-Readonly bleibt. Rechte und Stage-Zeit werden am geprüften Handle gesetzt; allgemeine Windows-Copy-ACLs werden durch diesen POSIX-Vertrag nicht neu definiert. |
| FS3, B17/Y152 | Linux-Grenzen kommen aus exaktem mountinfo-Einhängepunkt, auch gleicher Device-Nummer bei Bind-Mounts und autofs; Netz/FUSE/Pseudo bleiben klassifiziert. Fehlende/relative Wurzeln folgen vorhandenen Vorfahren. Windows-Volume-Abfragen unterdrücken Mediendialoge nur im aktuellen Thread und stellen den alten Error-Mode wieder her. |
| FS4/FS5/B18, Y90/Y131 | Vorhandene lokale Stage-/Flush-/syncfs-Verträge bleiben erhalten. Stage-Nachbearbeitung folgt keinem ausgetauschten Link und wartet nicht an Spezialdateien. Es wird keine zusätzliche Namespace-Dauerhaftigkeit für FUSE/CIFS zugesagt. |
| A24-lokal/Y88/Y136 | K1s tolerante Liste bleibt erhalten: unabhängige Einträge weitergeben, originale Namen/EntryError-Kontext behalten, unlesbare oder nicht darstellbare Namen als Auslassung. Große Share-Drahtantworten bleiben H-ANALYSIS/A-CLIENT. |
| A27, lokale Lesekonsente | `open_root_consented` benutzt ausschließlich vorhandene Lesefreigabe oder private Thread-Backup-Rechte. Der Helfer hält den aufgelösten Freigabe-Root sitzungsweit; PinRoot überträgt die read-only Vorfahrenkette, PinChild nur den neuen Ordner. Kein UAC-Start durch diese API; `open_root` und dessen Childs/Tag-Probes bleiben ordinary. Explizite Impersonation darf keinen GUI-Broker benutzen. |
| V2/FA4, erwartetes Recycle-Child | QuarantinedChild bindet den erwarteten regulären Inhalt an den Parent-Handle und bestätigt die Dateiidentität. Restore und Trash-Handoff ersetzen nichts. Linux benutzt eine private 0700-Reservierung; Windows hält DELETE am exakt identifizierten Objekt, erhält seine ACL und sperrt fremdes Schreiben/Löschen. Kein permanentes Löschen, keine Drop-Löschung. |
| S-LOCAL-Abhängigkeit | Private 0700-/0600- bzw. geschützte Owner-DACL-Childs entstehen exklusiv. `secure_private_handle`/`DirectoryHandle::secure_private` prüfen effektiven Owner, Art und Hardlinks, härten nur eigenes Altobjekt und verifizieren am Handle. Windows nutzt SetSecurityInfo; das benötigte Cargo-Feature ergänzte der Hauptagent separat in 779295d. |
| V1/V4, partielle Watch-Abdeckung | Additives `ChangeNotice::ReadyPartial { generation }`; nur Ready bedeutet vollständig. `watch_path` liefert Unix-FD-Anker bei gehaltenem Clone; Windows None. T-JOBS meldet die direkte Root-Watch als LocalOnly und hält Polling für unvollständige Abdeckung aktiv. |
| Eigene private Stages | Der gemeinsame Matcher erkennt ausschließlich `.se-private-<32 lowercase hex>.tmp`; fehlerhafte Präfixe/Längen/Zeichen und gewöhnliche .tmp-Dateien bleiben Nutzerdaten. |

Die frühere K1-Entscheidung „bereits geprüfte Ordner merken“ bezeichnet die damalige Teiländerung.
Ein nackter Pfad-Cache ist in der fortgeführten Umsetzung bewusst entfernt. Es gibt keine Behauptung,
dass alle älteren pfadbasierten VFS-Schreibmethoden damit schon FD-relative Mutationen ausführen.

## Konkrete Abnahmesignale

Alle hier genannten Signale gehören zusammen in die vorhandene endgültige Remote-Task-Suite.
Es wurden keine lokalen Testbefehle ausgeführt.

| Quelle / Signal | Erwartetes Ergebnis |
|---|---|
| `local_access/directory_handle_tests.rs`: `review_task_directory_handles_keep_independent_listing_positions` | Zweite Aufzählung verschiebt die erste nicht; Child-Datei liefert den erwarteten Inhalt. |
| `review_task_directory_handles_refuse_paths_instead_of_child_names` | Leer, . / .., Mehrkomponenten-, Root- und Slash-Namen werden vor Öffnen verweigert. |
| `review_task_directory_handles_create_private_children_without_replacement` | Exklusive Records/Directorys; bestehender Inhalt bleibt; Unix 0700/0600, Windows prüft Owner-DACL bereits vor Bytes. |
| `review_task_private_handle_hardening_refuses_hardlinked_records` | Hardlink-Record wird vor Härten verweigert; danach wird eigenes unaliased Altobjekt privat und erneuter Aufruf gelingt. |
| `review_task_directory_handles_refuse_links_specials_and_keep_raw_names` (Unix) | Verzeichnis-/Dateilink und FIFO verweigert; nicht-UTF8-Name unverändert in Iterator. |
| `review_task_directory_handles_stay_on_root_when_its_path_is_replaced` (Unix) | Root-Rename plus fremder Ersatzlink ändert Lesen, private Child-Erzeugung und Watch-FD-Anker nicht. |
| `review_task_directory_handles_pin_the_entered_windows_directory` | Betretener Ordner kann während der Pins nicht auf eine andere Position umbenannt werden. |
| `review_task_recycle_quarantine_restore_never_replaces_a_new_child`, `...moves_only_to_a_free_anchored_name`, `...refuses_a_replaced_expected_child`, `...keeps_its_parent_anchor_after_root_rename` | Erwartete Datei wird reversibel gehalten; Ersatz/Ziel bleibt unverändert; Root-Rename führt nie in fremde Inhalte. |
| `local_access/core/protocol_tests.rs`: `review_task_broker_child_suffix_stays_under_the_pinned_root` | Kein sibling-/../Drive-/ADS-Suffix; reine, wörtliche Child-Komponenten unter dem festgehaltenen Grant. |
| Bestehende Windows-`search_recursive_access_task_authenticated_helper_reuses_read_handles_in_parent` (ignored, remote-only) | Authentifizierte PinRoot/PinChild-RPCs, ordinary/restricted verweigert fremden GUI-Grant, Inhalt nur lesbar; Pins verhindern Rename auch nach Helfer-Ende. Vorhandene Umgebung `SMART_EXPLORER_ANALYTICS_TASK=1`, realer Windows-Runner mit Backup-Privilege/ACL-Werkzeugen. |
| `vfs/os/linux_os/review_task_local_guard_tests.rs` | Stage-Link/FIFO verweigert; 0400-Stage bleibt nachbearbeitbar; Quell-/Zielrechte ∩ und Set-ID-Maske stimmen; ersetzter mkdir-Ordner wird erneut geprüft. |
| `vfs/os/windows/review_task_stage_tests.rs` | Readonly-Stage behält das Attribut; jeder ungültige Neuanlage-Pfadteil bleibt ohne ADS-/Seiteneffekt. |
| `review_task_mount_boundaries_include_same_device_binds_and_autofs` | Bind-Mount desselben Devices und autofs sowie Netz/FUSE bleiben erkennbare Grenzen. |
| `copy/os/shared/safe_file_tests.rs`: zusätzliche `review_task_`-Signale | Stage vor Inhalt privat, privates bestehendes Ziel bleibt privat, ausgetauschter Stage-Link identifiziert keine fremde Datei, FIFO-Quellsnapshot verweigert. |
| `transfer/os/shared/walk_listers.rs`: `review_task_local_single_special_entry_is_reported_not_transferred` | Auch eine einzelne FIFO-Auswahl wird als Spezialdatei gemeldet und nicht gestreamt. |
| K1s bestehende Vertrags-/Backend-/NFS/FUSE-/FAT/exFAT-Stufen in [K1.md](K1.md) | Keine Regression der vorhandenen V1-Verträge, Leer-/Readonly-Ziele, Zeitgenauigkeit, Volume-UUIDs, Limits oder NOREPLACE-Leiter. |

## Self-Review und Entscheidungen

Statischer Rust-Parser (vorhandener `/root/.local/share/uv/tools/graphifyy/bin/python`,
tree_sitter + tree_sitter_rust) über alle eigenen native Quellen: keine ERROR-/Missing-Knoten.
Eigene neue/substanziell geänderte Rust-Quellen bleiben unter 500 Zeilen und 50 KiB.
`git diff --check` auf der eigenen Fläche liefert keine Whitespace-Fehler. Diese Signale sind
Text-/Syntaxbefunde; sie belegen weder Typprüfung noch OS-/Provider-Verhalten.
Keine Compiler, Linker, Formatter, lokalen Suites, Installationen, Server oder langen Prozesse.
Keine eigenen Commits/Pushes und kein Graph-Neubau; der Hauptagent integriert und aktualisiert den Root-Graph.

Die Owner-DACLs werden vor privatem Inhalt geprüft, nicht nach einem Pfad-Stat. Eine fehlgeschlagene
exklusive Reservierung darf leer erhalten bleiben. Recycle hält bei Fehlern Inhalt für Restore/Recovery;
Restore/Move sind ausdrücklich. Unix-NOREPLACE-Unterstützung und sichere Windows-Dateiidentität sind
Voraussetzungen für Capture; ein Provider ohne diese Garantien wird verweigert. Root-Pins verwenden
128-bit-Identität, wenn vorhanden, dokumentierte 64-bit-Fallback-Identität bei älteren/FAT-Providern;
unbekannte Null-IDs bleiben eine sichtbare Unsupported-Grenze, keine erfundene Gleichheit.

Consumer- und außer-scope Grenzen stehen mit exakten Hooks in [anfragen/V-LOCAL.md](../anfragen/V-LOCAL.md).
Y95-Schreibpfadoptimierung, tatsächliche Recycler-/Watch-/Wire-Integration und die bestehenden
K1-Fremdflächen werden dort ausdrücklich getrennt von der gelieferten nativen Grundlage geführt.

## Erstellte Dateien

Neu in diesem Fortführungsblock:

- `native/src/local_access/directory_handle_tests.rs`
- `native/src/local_access/os/linux/create.rs`
- `native/src/local_access/os/linux/directory_handle.rs`
- `native/src/local_access/os/linux/quarantine.rs`
- `native/src/local_access/os/windows/broker_pins.rs`
- `native/src/local_access/os/windows/create.rs`
- `native/src/local_access/os/windows/directory_handle.rs`
- `native/src/local_access/os/windows/directory_identity.rs`
- `native/src/local_access/os/windows/private_security.rs`
- `native/src/local_access/os/windows/quarantine.rs`
- `native/src/local_access/os/windows/regular.rs`
- `native/src/vfs/os/linux_os/review_task_local_guard_tests.rs`
- `native/src/vfs/os/linux_os/stage.rs`
- `native/src/vfs/os/windows/review_task_stage_tests.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/V-LOCAL.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/V-LOCAL.md`

## Geänderte Dateien

Fortgeführt, einschließlich bereits vorhandener untracked K1-Teilquellen:

- `native/src/copy/os/linux_os.rs`
- `native/src/copy/os/shared/move_guard.rs`
- `native/src/copy/os/shared/safe_file.rs`
- `native/src/copy/os/shared/safe_file_tests.rs`
- `native/src/copy/os/shared/server_copy.rs`
- `native/src/copy/os/shared/staging.rs`
- `native/src/copy/os/windows.rs`
- `native/src/local_access/core/protocol.rs`
- `native/src/local_access/core/protocol_tests.rs`
- `native/src/local_access/mod.rs`
- `native/src/local_access/os/linux_os.rs`
- `native/src/local_access/os/windows/access_task.rs`
- `native/src/local_access/os/windows/broker.rs`
- `native/src/local_access/os/windows/directory.rs`
- `native/src/local_access/os/windows/helper_task.rs`
- `native/src/local_access/os/windows/mod.rs`
- `native/src/local_access/os/windows/read.rs`
- `native/src/transfer/os/shared/walk_listers.rs`
- `native/src/vfs/core/extension_types.rs`
- `native/src/vfs/core/staging_names.rs`
- `native/src/vfs/core/review_task_contract_tests.rs`
- `native/src/vfs/core/promotion.rs`
- `native/src/vfs/mod.rs`
- `native/src/vfs/os/linux_os/local_platform.rs`
- `native/src/vfs/os/linux_os/mountinfo.rs`
- `native/src/vfs/os/linux_os/review_task_mountinfo_tests.rs`
- `native/src/vfs/os/linux_os/volume_id.rs`
- `native/src/vfs/os/shared/local.rs`
- `native/src/vfs/os/shared/local_dirs.rs`
- `native/src/vfs/os/shared/local_extensions.rs`
- `native/src/vfs/os/shared/local_stage.rs`
- `native/src/vfs/os/windows/local_platform.rs`
- `native/src/vfs/os/windows/local_writes.rs`
- `native/src/vfs/os/windows/volume_info.rs`

`vfs/mod.rs` enthält aus diesem Block ausschließlich den eigenen additiven Reexport
`create_local_copy_stage`. Globale Pläne, fremde Registrierungsdateien und Cargo wurden nicht
von V-LOCAL geändert; das erwähnte Authorization-Feature ist der separate Hauptagent-Eintrag.

## Gelesene Dateien

Alle oben erstellten/geänderten Rust-Quellen wurden im eigenen Self-Review gelesen.
Zusätzlich vollständig oder in den konkret betroffenen Abschnitten:

- `native/Cargo.toml`
- `native/src/android_fs/mod.rs`
- `native/src/android_fs/os/rename.rs`
- `native/src/android_fs/os/rename_tests.rs`
- `native/src/copy/mod.rs`
- `native/src/copy/os/shared/copy.rs`
- `native/src/copy/os/shared/durability.rs`
- `native/src/copy/os/shared/path_guard.rs`
- `native/src/copy/os/shared/server_copy_tests.rs`
- `native/src/local_access/core/entry_error.rs`
- `native/src/local_access/core/regular.rs`
- `native/src/local_access/os/windows/directory_records.rs`
- `native/src/local_access/os/windows/directory_tests.rs`
- `native/src/local_access/os/windows/elevation.rs`
- `native/src/local_access/os/windows/paths.rs`
- `native/src/local_access/os/windows/pipe.rs`
- `native/src/local_access/os/windows/privilege.rs`
- `native/src/local_access/os/windows/sync_link_task_tests.rs`
- `native/src/types/core/win32_names.rs`
- `native/src/vfs/core/cache.rs`
- `native/src/vfs/core/cache_extensions.rs`
- `native/src/vfs/core/core.rs`
- `native/src/vfs/core/fs_profile.rs`
- `native/src/vfs/core/promotion_tests.rs`
- `native/src/vfs/core/staging_names.rs`
- `native/src/vfs/core/volume.rs`
- `native/src/vfs/os/shared/review_task_local_tests.rs`

Dokumentationskontext (jeweils konkrete Abschnitte zum zugewiesenen Vertrag):

- `AGENTS.md`, `docs/ARCHITEKTUR.md`
- `docs/lesungen/INDEX.md`, `docs/refs/INDEX.md`, `docs/refs/local-fs-identity-durability.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/v-local.json`
- Im selben Planordner: `umsetzung.md`, `fortsetzung.md`, `recherche.md`, `review.md`, `spec.md`,
  `review-befunde-sync.md`, `review-befunde-analyse.md`, `review-befunde-sicherheit.md`,
  `abnahme/K1.md`, `anfragen/K1.md` sowie diese beiden eigenen Übergabedokumente.
- Skills `/root/.codex/skills/arbeitsweise/SKILL.md` und `/root/.codex/skills/graphify/SKILL.md`.
  Der Hauptagent lieferte die ausgeführte Graph-Abfrage; kein eigener Graph-Neubau.

Zusätzlicher Primärquellen-Abgleich am 2026-10-03 für die zuletzt angeschlossenen Hooks:
[CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
(Sharing bleibt bis Handle-Schließung),
[GetFileInformationByHandle](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfileinformationbyhandle)
(Identitätsfallback),
[GetKernelObjectSecurity](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-getkernelobjectsecurity)
(READ_CONTROL und self-relative Descriptor),
[SetSecurityInfo](https://learn.microsoft.com/en-us/windows/win32/api/aclapi/nf-aclapi-setsecurityinfo)
(handlegebundene DACL-Änderung) und
[Authorization-Bindings 0.59](https://raw.githubusercontent.com/microsoft/windows-rs/0.59.0/crates/libs/sys/src/Windows/Win32/Security/Authorization/mod.rs)
(Feature/Signaturen).
