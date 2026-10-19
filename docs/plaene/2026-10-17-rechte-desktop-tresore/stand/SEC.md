# Stand SEC (Sicherheitsagent)

Stand: 2026-10-19.

- Punkte 1–3 sind committet: df15feb5, e6b4c24a, 25e08110.
- Punkt 1b und Punkt 4 (U27) sind im Quelltext fertig, aber nicht gebaut, nicht getestet und nicht committet.
- Jede geänderte Rust-Datei ist mit `rustfmt --check` geprüft und hat weniger als 500 Zeilen.

## Punkt 1b: R15 restlos

**Entfernt** (alles außerhalb von `exec_client_messages.rs` und seinem Test):

- `ReservationGauge` mit allen Implementierungen;
- die Parameter von `broker(gauge)` und `request(gauge, …)`; `request(principal, capability, summary)` ist jetzt der einzige Weg;
- `ApprovalOutcome::{DeviceLimit, HostLimit}` und der Arm in `screen/core/authorize.rs`;
- der Kommentar in `wire_hello.rs`;
- der Parameter von `NodeScreen::new`.

**Bleibt mit Absicht:** `PrincipalLimit` und `GlobalLimit` nur noch in der Übersetzung für alte Hosts (`exec_client_messages.rs`) und deren Test (`access_approval_limits_tests.rs`).

**Offen:**

- `share/core/node.rs:173` muss `NodeScreen::new()` statt `NodeScreen::new(&exec_registry)` aufrufen. Die Datei liegt außerhalb des SEC-Bereichs.
- Die Doku von `ConsentBroker::request` in `screen/core/host_backend.rs:132–133` nennt noch „reservation“. Die Datei gehört U16.

## Punkt 4: U27 (Android als Bildschirm-Host)

**Neue Rust-Dateien:**

- `screen/os/android/bridge_channels.rs`
- `screen/os/android/bridge_backend.rs`
- `screen/os/android/bridge_source.rs`
- `screen/os/android/bridge_tests.rs`
- `mobile/os/shared/domains/screen_host.rs`
- `mobile/os/shared/domains/screen_host_tests.rs`
- `android-bridge/src/screen_jni.rs`

**Neue Kotlin-Dateien:**

- unter `A/screen/`: ScreenHostControl, ScreenHostService, ScreenConsentActivity, ScreenEncoder, ScreenControlService, GestureMapper, RunningApps, ScreenStillSource, SyntheticSurfaceSource, ScreenHostNotifier;
- `A/ui/screen/ScreenHostSection.kt`;
- `A/api/ScreenApi.kt`;
- `res/xml/screen_control_service.xml`;
- `androidTest/.../ScreenTaskTest.kt`.

**Geänderte Dateien:**

- `NativeBridge.kt`, `CoreEvents.kt`, `AndroidManifest.xml`, `strings.xml`, `ShareScreen.kt`;
- `AccessApprovalNotifier.kt` (Aktionsziele aus W1);
- `AccessDialog.kt` (Übergabe U21: Rechtezeilen und Bildschirm-Zweig der Rückfrage);
- `mobile/.../share_access.rs` mit Tests (Filter der Bildschirmzeilen aufgehoben);
- `share/core/node_screen.rs` (`host_indicator`).

**Registrierungsbedarf:** siehe Abschlussbericht vom 2026-10-19 (Lauf Punkt 1b/4). Betroffen sind `screen/os/mod.rs` (U16), `domains/mod.rs` und `mobile/mod.rs` (R9) sowie `android-bridge/src/lib.rs`.

**Offen:**

- `PermTaskTest.kt` (U21) erwartet für Bildschirm-Rückfragen nur „Ablehnen“. Seit U27 gibt es dort drei Aktionen; die Zeilen sind anzupassen.

## Nächster Schritt

Auf Punkt 5 warten.
