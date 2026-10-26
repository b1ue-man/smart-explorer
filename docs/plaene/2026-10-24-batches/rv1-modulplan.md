# RV1: Modulgrenzen nach Remoteformatierung

Stand: 2026-10-26. Dieser Folgeplan gehört zum selben aktiven Gesamtbatch und
beauftragt keine neue Produktfähigkeit. Beleg ist das `format.json` des
[Vorbereitungslaufs 40332971455](https://github.com/b1ue-man/smart-explorer/actions/runs/40332971455)
für Kandidat `34f3361d`. Die gebundenen Lock-/Formatpatches sind `10988167`.
Compiler-, Funktions- und Gerätejobs wurden in diesem Versuch nicht ausgeführt.

Der erste Befund ist rein strukturell: normal formatierte Verantwortlichkeiten
überschreiten die Projektgrenze. Die konkrete fachliche Aufteilung entsteht
beim jeweils bisherigen Fixautor aus seinen bestehenden Kettenkarten und den
jetzigen Deklarationen; sie wird vor dem Edit in seinem Bericht festgehalten.
Jeder Autor bleibt für Änderung und anschließende Anti-Regressionsreview
zuständig. Neue Pfade und Scopeergänzungen sind ausdrücklich vom Main zugewiesen;
die Schreibflächen überschneiden sich nicht. Höchstens sieben Kindagenten
arbeiten gleichzeitig; die Codec-/Capsgruppe folgt auf den ersten freien Platz.

## Ergebnisvertrag für jede zugewiesene Verantwortung

- Normale Rustformatierung, weniger als 500 Zeilen und weniger als 50 KiB je
  neuem oder wesentlich geändertem Modul; fachliche Trennung statt Zeilenverdichtung.
- Öffentliche Namen, Einstiegspunkte, Plattformfilter, Wire-/Persistenzformate,
  Zugriffsentscheidungen, Ergebniswerte und sichtbare Oberflächen bleiben erhalten.
- Speicher-/Prozessbesitz, Abbruch, Wiederholung, Backups, Konflikte und reale
  Endbelege behalten ihre Reihenfolge und Bedingungen.
- Bestehende Fallnamen und die Abdeckung im einzigen Remoteeinstieg bleiben
  erhalten; ein verschobener Testkörper rechtfertigt keine zusätzliche Testsuite.
- Statische Prüfung im eigenen Autorzyklus; Kompilierung und Verhalten werden
  ausschließlich über denselben später wiederholten Remote-Suiteeinstieg bestätigt.

## Zugeordnete Meilensteine

Jede Zeile bezeichnet eine zusammenhängende Integrationsfläche. Innerhalb
der Fläche gelten die einzelnen Module als kleinere Strukturmeilensteine mit
dem Ergebnisvertrag oben; ihr Abschluss erzeugt keinen separaten Testlauf.

| Verantwortlicher bestehender Autor | Zugewiesene Module | Eigener Plan und Nachreview |
|---|---|---|
| `fix_transfer_compat` | `native/src/agent/core/extensions.rs`<br>`native/src/app/core/picker_impl.rs`<br>`native/src/cli/target.rs`<br>`native/src/copy/os/shared/staging.rs`<br>`native/src/share/core/server_fs.rs`<br>`native/src/transfer/os/shared/engine/file_recovery.rs`<br>`native/src/transfer/os/shared/engine/file_recovery_tests.rs`<br>`native/src/transfer/os/shared/engine/job_store.rs`<br>`native/src/transfer/os/shared/engine/mod.rs`<br>`native/src/transfer/os/shared/engine/test_backend.rs` | [fix-regression-transfer.md](fix-regression-transfer.md) |
| `fix_vault_retry` | `native/src/app/core/drive_snapshot.rs`<br>`native/src/daemon/os/shared/mount_client.rs`<br>`native/src/mount/mod.rs` | [fix-vault-retry.md](fix-vault-retry.md) |
| `fix_mcp_lifecycle` | `native/src/mcp_launcher/os/shared/installer_cli.rs` | [fix-regression-mcp.md](fix-regression-mcp.md) |
| `fix_device_navigation` | `native/src/app/core/analytics_access.rs`<br>`native/src/app/core/share_access_accounts_ui.rs`<br>`native/src/app/core/share_device_page_ui.rs`<br>`native/src/apps/os/android/bridge.rs`<br>`native/src/apps/os/host.rs`<br>`native/src/daemon/os/shared/discovery_pairing_ipc.rs`<br>`native/src/daemon/os/shared/ipc_protocol.rs`<br>`native/src/screen/os/linux_os/app_icons.rs`<br>`native/src/screen/os/windows/app_icons.rs`<br>`native/src/share/core/access_runtime_tests.rs`<br>`native/src/share/core/node_apps.rs` | [fix-device-navigation.md](fix-device-navigation.md) |
| `fix_x11_family` | `native/src/app/core/family_viewer.rs`<br>`native/src/remote_window/core/host.rs`<br>`native/src/remote_window/os/x11.rs`<br>`native/src/remote_window/os/x11/batches_popup_focus_task_tests.rs`<br>`native/src/screen/os/linux_os/host_backend.rs` | [fix-x11-family.md](fix-x11-family.md) |
| `fix_capture_restart` | `native/src/login/os/shared/gateway.rs`<br>`native/src/login/os/shared/viewer.rs`<br>`native/src/rdp/os/shared/session.rs`<br>`native/src/screen/core/batches_capture_admission_task_tests.rs`<br>`native/src/screen/os/linux_os/x11_capture.rs`<br>`native/src/screen/os/mod.rs`<br>`native/src/screen/os/shared/capture_broker.rs` | [fix-windows-capture.md](fix-windows-capture.md) |
| `fix_screen_budget` | `native/src/app/core/screen_decode_thread.rs`<br>`native/src/screen/core/host_session.rs`<br>`native/src/screen/os/android/bridge_channels.rs`<br>`native/src/screen/os/android/bridge_source.rs`<br>`native/src/screen/os/android/viewer_sink.rs`<br>`native/src/screen/os/shared/batches_session_budget_compatibility_task_tests.rs` | [fix-regression-screenbudget.md](fix-regression-screenbudget.md) |
| `fix_quality_acceptance` | `native/src/screen/core/codec_registry.rs`<br>`native/src/screen/core/frame_size_caps.rs`<br>`native/src/screen/core/wire_control.rs` | [fix-quality-acceptance.md](fix-quality-acceptance.md) |

Nach vollständiger Integration aktualisiert Main den Rootgraph, committet und
pusht den nächsten Kandidaten und wiederholt ausschließlich RV1. Eine reine
Format-/Größenkorrektur löst weder eine Versionserhöhung noch einen Release aus.
Der aktive Gesamtbatch endet weiterhin in genau einem vollständigen Remote-Release.

Die statische Gesamtinventur des Quelltexts findet nach den Aufteilungen keine
neue Überschreitung. Die bestehende Registrierungswurzel
`native/src/share/mod.rs` bleibt mit 569 Zeilen als ausdrücklich dokumentierte
Altbestand-Ausnahme bestehen: Sie war bereits an der Batchbasis übergroß;
der Batch ergänzt dort Modulanschlüsse und Reexports, keine Funktionskörper.
Der bestehende Remotegrößenwächter wird dafür nicht geändert. Alle 46
beanstandeten Verantwortlichkeiten und ihre neuen Fachmodule bleiben vom
uneingeschränkten Größenvertrag erfasst; Versuch `40342520266` bestätigt ihre
Größen nach erneuter Remoteformatierung.

## Bestätigte Importkorrektur in der Transferkette

Der Transferautor hat bei seiner eigenen Strukturprüfung in vier Dateien
falsche relative Typverweise ermittelt. `transfer/mod.rs` bindet `engine` direkt
unter `transfer` ein; deshalb führt `super::super::super` in `job_store.rs`,
`folder_alias_store.rs`, `folder_reservation.rs` und `roots.rs` zur Cratewurzel. Die
benötigten Typen liegen in `transfer::job_record` beziehungsweise
`transfer::folder_alias`. Main hat die drei zusätzlich betroffenen
Dateien ausschließlich für diese Typbezüge demselben Autor zugewiesen. Korrektur und Nachreview
bleiben in seiner bestehenden Transferkette; aus gleich aussehenden Präfixen
in tiefer verschachtelten Modulen wird kein weiterer Fehler abgeleitet.

## Integrierter Autorenabschluss

Jeder genannte Autor hat seinen Plan, seine vollständige betroffene Kette,
die Aufteilung und die anschließende eigene statische Anti-Regressionsreview
im verlinkten Bericht abgeschlossen. Main hat die exakten Schreibflächen,
AST-Syntax, Quellgrößen, unveränderten Fallnamen und Modulanschlüsse geprüft
und die Meilensteine auf `codex/review-fixes-ee75e91b` gepusht:

| Verantwortung | Strukturcommit |
|---|---|
| MCP-Installer | `4ac9d8d1` |
| Mount/Snapshot | `55abd87c` |
| X11-Familie | `3d5c8b54` |
| Geräte-/Appnavigation | `bd3e1085` |
| Screen-/Androidbudget | `adefcd5a` |
| Codec/Geometrie/Control | `0e092385` |
| Transfer/Recovery | `86c1bc9c` |
| Login/Capture | `0a8314a1` |

Die fünf korrigierten Transferstellen sind drei Typimporte, der Alias-Lesetyp
in `confirmed_move` und die Alias-Scope-Konstruktion in `roots::plan_roots`.
R19 ergänzt ausschließlich die bereits vorhandenen reinen Vertragsfälle der
verschobenen Mount-Runtimetypen. Die bisherige Fallauswahl bleibt erhalten;
die vier verschobenen Admission-Lebensdauerfälle werden weiterhin über ihre
exakten bestehenden Suffixe aufgelöst. Keine lokale Formatierung, Kompilierung
oder Testausführung fand statt. Versuch `40342520266` hat die Modulgrößen
remote bestätigt und den verbleibenden Layoutpatch geliefert; Main hat ihn
kandidatengebunden übernommen. Typprüfung und Laufzeitbestätigung stehen
weiterhin aus. Die genaue Artefaktbindung steht im [RV1-Beleg](../../refs/rv1-remote-suite.md).
