# Stand SEC (Sicherheitsagent)

Stand: 2026-10-19.

- Committet sind Punkt 1–3, 1b, 4, 4b und 5 (df15feb5, e6b4c24a, 25e08110, e2233d85, c87bd083, 33744c73, 99cf1d63).
- Punkt 6 (R15c) und Punkt 7 (U23) sind im Quelltext fertig, aber nicht gebaut, nicht getestet und nicht committet.
- Jede geänderte Rust-Datei ist mit `rustfmt --check` geprüft und hat weniger als 500 Zeilen.

## Punkt 4b

- `screen/os/mod.rs`: Android-Module eingetragen; `platform_backend()` hat je einen Arm für Windows, Linux, Android und alle übrigen Plattformen; `host_backend()` nutzt auf Android `BridgeBackend` ohne `WorkerBackend`; `host_indicator()` ist neu.
- R9-Zeilen gesetzt in `mobile/os/shared/domains/mod.rs`, `mobile/mod.rs` und `android-bridge/src/lib.rs`.
- `PermTaskTest.kt` prüft die drei Aktionen der Bildschirm-Rückfrage.
- In `screen/core/host_backend.rs` ist der Kommentar zu den Reservierungen gestrichen.

## Punkt 5: U22

**Entscheidung zum Ort der Eingabe (Frage aus U16-4):** Die Eingabe läuft im Hostprozess über `worker_wire` `INPUT` → `INPUT_DONE`. Der Daemon-`WorkerBackend` leitet sie an den jüngsten Hostprozess einer Sitzung im Modus `control` weiter. Ob eine Sitzung steuern darf, liest `WorkerBackend` beim Öffnen aus dem Register. Das Register bekommt er über `host_indicator`.

**Umgesetzt:**
- Windows: `input_send.rs`, `input_integrity.rs` und Tests.
- Linux: `x11_input.rs`, `x11_keymap.rs` und `portal_input.rs` (RemoteDesktop-Sitzung mit ScreenCast).
- Gemeinsam: `os/shared/input_keys.rs`.
- CLI: `cli/screen/input.rs`.
- MCP: eigenes Werkzeug `screen_input` in `cli/mcp/screen_tool.rs`.
- Lizenz: `third-party/openh264/BINARY_LICENSE.txt` und `screen/core/licenses.rs`.
- `se screen codec licenses` und das Fenster „Lizenzen“ in der Desktop-App.

**Registrierungsbedarf:** siehe Abschlussbericht vom 2026-10-19 (Lauf 4b/5). Betroffen sind `screen/core/mod.rs`, `cli/screen.rs`, `daemon/mod.rs`, `cli/mcp/catalog.rs`, `cli/mcp/server.rs`, `.gitattributes` und `installer.nsi` (U36).

## Punkt 6: R15c

- `screen/core/registry.rs`: `MAX_SESSIONS` (8) und `MAX_DEVICE_SESSIONS` (4) samt Prüfung entfernt. Die exklusive Steuerung bleibt. `RegistryRefusal::SessionLimit` bleibt mit `#[allow(dead_code)]`, weil `screen/os/shared/host_server.rs` die Variante noch nennt.
- `share/os/windows/exec_account_connection.rs` und `share/os/linux_os/exec_helper_server.rs`: `MAX_SESSIONS` (8 je Verbindung) samt Fehlertext entfernt.
- `mobile/os/shared/domains/screen_host.rs`: `MAX_APPS` (256) ganz entfernt. Begründung: Die Liste läuft als JNI-String, nicht über Binder; Annahme A50 trifft nicht zu.
- `mount/os/linux_os/mount_point.rs`: `MAX_SUFFIX` (999) entfernt. Die Suche läuft, bis ein freier Name gefunden ist; nur ein Ein-/Ausgabefehler außer `NotFound` oder ein `u64`-Überlauf bricht ab.
- `mobile/os/shared/scan.rs` geprüft: `MAX_SESSIONS` (6) ist eine Aufbewahrungsgrenze und blockiert keinen neuen Scan; bleibt.

## Punkt 7: U23 (DRV-M8, Android-Dokumentanbieter)

**Rust:** `mobile/os/shared/domains/docs.rs` (Methoden `docs.*`, Listen mit `loading`, Ereignis `docs.changed`), `docs_engines.rs` (eine Mount-Engine je Root bzw. Gerät, Cache-Lease, Posix, `ReadOnlyHandles`, `docs.pending`, `docs.retryPending`), `docs_io.rs` (Deskriptoren, negiertes errno, Widerruf), `docs_ids.rs` (Dokument-IDs, `is_child` ohne I/O), `docs_vault.rs` (Freigabe-Einstellungen, Tresor-Roots, Sperrteilnehmer), `native/android-bridge/src/io.rs` (JNI `doc*`), `mount/core/range_read.rs` (`RangeReadPolicy`, `MountEngine::open_rooted`), `mount/core/types.rs` (Feld `range_reads`).

**Kotlin:** `A/system/documents/{SeDocumentsProvider,DocumentIds,ProxyFileCallback,DocumentsCursors,DocsCommitWorker}.kt`, `A/api/DocsApi.kt`, `A/ui/settings/DocumentsProviderSection.kt`; geändert `NativeBridge.kt`, `CoreEvents.kt`, `SettingsScreen.kt`, `AndroidManifest.xml`, `strings.xml`, `build.gradle.kts` (Robolectric 4.14.1).

**Tests:** `android/app/src/test/.../system/documents/DriveTaskProviderTest.kt` (12 Robolectric-Signale), `android/app/src/androidTest/.../task/DriveTaskProviderDeviceTest.kt` (7 Emulator-Signale), Rust-Hilfstests `drive_task_docs_*` in `docs_ids.rs`, `docs_io.rs`, `docs_vault.rs`.

**Offen:** R8-Zeilen (`domains/mod.rs`, `mobile/mod.rs`, `android-bridge/src/lib.rs`), `api.md` (U43); siehe Bericht vom 2026-10-20.

## Nächster Schritt

Auf den nächsten Punkt warten.
