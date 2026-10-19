# Stand SEC (Sicherheitsagent), Punkte 1–3

Stand: 2026-10-19. Punkte 1–3 sind im Quelltext fertig. Nichts wurde gebaut, getestet oder committet. Jede geänderte Rust-Datei ist mit `rustfmt --check` geprüft und hat weniger als 500 Zeilen.

## Punkt 1 (R15, E12): Exec ohne Gleichzeitigkeitsgrenze

**Geänderte Dateien** (alle unter `native/src/share/core/`):
- `exec_registry.rs`: entfernt sind `global_active`, `principal_active`, die beiden Prüfungen in `prepare` und `GlobalLimit`/`PrincipalLimit`. `history` und `terminal_cache` bleiben.
- `access_approval.rs`: entfernt sind `DEVICE_RESERVATIONS`, `HOST_RESERVATIONS`, `room_for` und `admits`. Neu ist `open_request`. `request(gauge, …)` gibt es nur noch mit `#[cfg(test)]`. `ApprovalBroker` hat kein Messfeld mehr.
- `access_approval_exec.rs`: entfernt sind `limit_error` und `StartApproval.gauge`.
- `access_approval_answer.rs`: entfernt ist der Arm `DeviceLimit | HostLimit`.
- `exec_admission.rs`: entfernt ist die Vorabprüfung `approvals.admits(...)`.
- `exec_client_messages.rs`: Ein Client übersetzt die Antwort älterer Hosts (`admission_denied` mit `PrincipalLimit` oder `GlobalLimit`) in einen Text mit nächstem Schritt.
- Tests: `exec_registry_tests.rs`, `access_approval_limits_tests.rs`, `access_approval_tests.rs` und `exec_grant_runtime_tests.rs`.

**Bleibt außerhalb des SEC-Bereichs (offen):**
- `ApprovalOutcome::{DeviceLimit, HostLimit}` bleiben mit `#[allow(dead_code)]` und werden nie erzeugt. Grund: `screen/core/authorize.rs:311` nennt sie im Match.
- Der Trait `ReservationGauge` und die Parameter von `broker(gauge)` und `request(gauge, …)` bleiben, werden aber nicht mehr ausgewertet. Grund: Diese Dateien nutzen sie:
  - `screen/core/authorize_tests.rs`
  - `share/core/node_screen.rs:79-80`
  - `share/core/node_screen_task_fixture.rs`
  - `cli/share/exec_approvals_tests.rs`
  - `mobile/os/shared/domains/share_access_tests.rs`
- In `screen/core/wire_hello.rs:264` sagt ein Kommentar noch „or no reservation is free“.

## Punkt 2: Registrierung U13/U14/U20 in den Hub-Dateien

**Gesetzt in:**
- `daemon/drive_modules.rs`
- `daemon/os/shared/ipc_protocol.rs`: `daemon_token()` liefert `None` für `SpoolKey` und `MountHostExt`.
- `daemon/os/shared/ipc.rs`
- `daemon/os/shared/ipc_host_ext.rs`
- `daemon/mod.rs`
- `app/mod.rs`
- `cli/mcp/catalog.rs`

Die `vault`-Moduldeklarationen von U20 waren schon vorhanden.

**Tests:** in `daemon/os/shared/ipc.rs`. Sie prüfen nur die Ablehnungen (Daemon-Token, fremder Token, unbekanntes Laufwerk, ungültige ID). Den Erfolgsfall mit gültigem Sitzungstoken decken nur die End-to-End-Tests aus U14 ab (`vault_task_mount_linux_fuse_e2e`, `drive_task_vault_*`).

## Punkt 3: Tresor-Jobs an den Tresor-Läufer

**Läufer:**
- `daemon/os/shared/job.rs`: neu sind `vault_attempt`, `run_settings` und `finish`. Der gewöhnliche Pfad wurde ohne Verhaltensänderung herausgezogen.
- `app/os/shared/sync_manual_run.rs`: neu ist `run_vault`. Der Lauf setzt keinen Paarkontext und keinen `state`, also bleibt die allgemeine Konfliktlösung zu.
- `mobile/os/shared/domains/sync_run.rs`: `checked_settings` und `run_bisync_with` haben einen Tresorzweig. `state` wird auf `None` gesetzt, also lehnt `sync_conflicts::resolve` die Konflikte ab.
- `app/core/sync_core.rs`: keine Änderung nötig. Gespeicherte Läufe gehen dort über `start_saved_desktop_run`, die Datei enthält keine Ablehnung von Tresor-Jobs.

**`note_vault_backend`** wird jetzt aufgerufen in:
- `vault/os/shared/open.rs::open_with`
- `daemon/os/shared/ipc_vault_client.rs::open_vault_backend`

**K07:** `app/os/shared/remote_helpers/recovery_manifest.rs` versiegelt bei Tresor-Backends die Felder `name`, `local` und `remote`. Ist der Tresor nicht bekannt, bleiben die Felder leer und `withheld` ist gesetzt. Einträge anderer Backends bleiben Byte für Byte gleich (Schema 2).

**Neue Testdateien:** `app/os/shared/sync_manual_run_vault_tests.rs` und `mobile/os/shared/domains/sync_run_vault_tests.rs`.

## Inventurbedarf (`native/rdt_task/expected_signals.txt`, von SEC nicht geändert)

Siehe Abschlussbericht im Agentenlauf vom 2026-10-19.

## Nächster Schritt

Auf Punkt 4 (U27) warten.
