# Stand SEC (Sicherheitsagent)

Stand: 2026-10-21 (Lauf 4: Punkt 12 E13-Fragen, Punkt 13 Hintergrundsperre; Lauf 3 in 829e7d9a).

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

## Punkt 8: Antworten S1–S8

Grundlage: `festlegungen.md` § U31 (U31-1 bis U31-17, S1–S8), `strang-tresore.md` C3, C4 (K05, K16/K17, K24, K44–K46), C7.1; Code: `core/SecretsKey.kt`, `vault/os/shared/{remember,unlock,session_timer}.rs`, `vault/core/lock_policy.rs`, `cryptomator/core/kdf.rs`, `mobile/os/shared/{runtime,transfer}.rs`, `mobile/core/crumbs.rs`, `service/ServiceNotifications.kt`. Abweichungen vom Entwurf sind mit **Abweichung** markiert; U44 überträgt sie nach `festlegungen.md`, falls nötig.

**S1 (gespeichertes Passwort): Ja, kryptografisch an die Displaysperre gebunden, nicht nur ein Dialog davor. Abweichung von U31-4/U31-17.**
- Befund: Heute legt der Kern das Passwort im Klartext in den eigenen Geheimnisspeicher (`remember::remember`, Konto `smart-explorer:vault:<site-id>`); dessen Datenschlüssel (`SecretsKey.kt`) verlangt bewusst keine Nutzerbestätigung. Ein `BiometricPrompt` nur vor `vault.unlock {saved:true}` wäre eine reine Oberflächensperre: jeder Codepfad im Prozess (Bridge, MCP-/Share-Weg, Instrumentierung) entsperrte ohne Bestätigung.
- Entscheidung: Auf Android speichert der Kern nie das Klartextpasswort, sondern einen in Kotlin erzeugten Umschlag.
  - Keystore-Schlüssel `smart_explorer_vault_saved_v1` (einer für alle Orte): AES-256, `BLOCK_MODE_GCM`, `ENCRYPTION_PADDING_NONE`, `setUserAuthenticationRequired(true)`, `setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL)` (jede Nutzung einzeln bestätigt), `setUnlockedDeviceRequired(true)`, `setInvalidatedByBiometricEnrollment(false)` (wer neue Finger anlegt, kennt die Geräte-PIN schon), zuerst `setIsStrongBoxBacked(true)`, bei `StrongBoxUnavailableException` ohne. Neue Kotlin-Datei `A/ui/vaults/VaultSavedPassword.kt` (≤ 250 Zeilen).
  - Umschlag: Text `android-kw1:` + Base64url ohne Auffüllung von (IV 12 Byte ‖ Chiffrat ‖ Tag); `cipher.updateAAD(site.toByteArray(UTF_8))` bindet ihn an den Ort.
  - Bestätigung: `android.hardware.biometrics.BiometricPrompt` (Framework, keine Gradle-Abhängigkeit; minSdk 30 erlaubt `CryptoObject` mit `BIOMETRIC_STRONG or DEVICE_CREDENTIAL`), Titel **„Tresor entsperren“**, Untertitel **„<Bezeichnung>“**; beim Merken Titel **„Passwort merken“**, Untertitel **„<Bezeichnung>“**. Immer mit `CryptoObject(cipher)`; ohne diesen Weg kein Ver- oder Entschlüsseln.
  - Merken: Kontrollkästchen an → nach erfolgreichem `vault.unlock {site, password, remember:false}` fragt das Blatt per `BiometricPrompt` (ENCRYPT), dann `vault.rememberSaved {site, sealed}`. Der Kern prüft `sealed` beginnt mit `android-kw1:`, der Ort ist entsperrt (das Passwort hat also eben geöffnet), und speichert `sealed` unverändert über `remember::remember`. Bricht der Nutzer die Bestätigung ab, bleibt der Tresor entsperrt, nichts wird gespeichert, Snackbar **„Passwort nicht gespeichert.“**
  - Nutzen: Knopf **„Mit gespeichertem Passwort entsperren“** → `vault.savedSecret {site}` → `{"sealed": string}` → `BiometricPrompt` (DECRYPT) → `vault.unlock {site, password, remember:false}`; die Zeichenkette wird nach dem Aufruf verworfen (wie beim getippten Passwort, U31-14).
  - Kern auf Android: `vault.unlock {saved:true}` und `vault.unlock {remember:true}` → `invalid_argument` mit **„Gespeicherte Passwörter brauchen auf Android die Bestätigung der Displaysperre.“** Die Methodenliste wächst damit auf 22 (`vault.compareItems`, `vault.rememberSaved`, `vault.savedSecret`); `VaultTaskTest` und `api.md` (U43) zählen 22.
  - Fehlerfälle: keine Displaysperre (`KeyguardManager.isDeviceSecure == false`) → Kontrollkästchen deaktiviert, Zusatz **„Nur mit eingerichteter Displaysperre möglich.“**; `KeyPermanentlyInvalidatedException` oder `AEADBadTagException` oder Umschlag ohne Präfix `android-kw1:` → `vault.forgetPassword {site}`, Passwortfeld zeigt **„Das gespeicherte Passwort gilt nicht mehr, weil die Displaysperre geändert wurde. Gib das Passwort ein.“**; `BiometricPrompt` abgebrochen (`BIOMETRIC_ERROR_USER_CANCELED`, `BIOMETRIC_ERROR_CANCELED`, `BIOMETRIC_ERROR_NEGATIVE_BUTTON`) → keine Meldung, Blatt bleibt offen; andere Fehlercodes → **„Bestätigung fehlgeschlagen: <errString>“**.
  - Begründung: Nur die Bindung an einen Keystore-Schlüssel mit Nutzerbestätigung erzwingt die Bestätigung auch gegen Codepfade außerhalb des Blatts; Framework-API statt `androidx.biometric` hält die Regel „keine neue Gradle-Abhängigkeit“ (VLT-M11).

**S2 (Klartextnamen in Texten): Nein, außer auf geschützten Bildschirmen im Speicher. Bestätigt und erweitert.**
- Klartextnamen und -pfade aus einem Tresor (alles unterhalb `vault://<site>/`) erscheinen nie in: Meldungen der Methoden `vault.*`, Einträgen von `Runtime::record_error`/`log_error` (Fehlerprotokoll), `Log.*` (logcat), Benachrichtigungen, Aufgabentiteln und Aufgabenmeldungen (`TaskCtx::message`; beide zeigt `ServiceNotifications.kt:43` in der Vordergrund-Benachrichtigung), `crash.log` (K24).
- Umsetzung im Kern: Hilfsfunktion `vault_redact(site: &str, text: &str) -> String` in `mobile/os/shared/domains/vaults.rs` ersetzt jeden Klartextpfad durch **„<Element>“**; jede `ApiError`-Meldung aus `vaults.rs` und `vault_compare.rs` geht hindurch. Für Aufgaben mit einer Quelle oder einem Ziel unter `vault://` bildet `crumbs::task_title` (neu neben `crumbs::title`) den Titel **„Tresor <Bezeichnung>“**, und die Aufgabenmeldung ist **„<n> von <m> Elementen“** statt eines Dateinamens; `fs_list.rs` behält `crumbs::title` für die Kopfzeile des Dateien-Reiters.
- Erlaubt bleiben: Bezeichnung und Site-ID, der Speicherort des Tresorordners selbst (Chiffratseite) und Klartextnamen auf den Bildschirmen aus S6, nur im Speicher.
- Snackbars und Fehlerkarten der Tresor-Bildschirme zeigen nur Texte aus `VaultTexts` und die bereinigten Kernmeldungen.
- Begründung: Fehlerprotokoll, logcat und Benachrichtigungen verlassen den geschützten Bildschirm (Benachrichtigungsverlauf, Sperrbildschirm, `adb logcat`); die Aufgabentitel sind der bisher ungeschützte Weg dorthin.

**S3 (Autofill): Ausschluss aktiv setzen, nicht nur `contentType` weglassen. Geändert.**
- Entscheidung: Jeder Bildschirm und jedes Blatt mit Tresorpasswortfeld (Entsperrblatt, Anlegen, Passwort ändern) setzt in einem `DisposableEffect` auf `LocalView.current` `importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS` und stellt beim Verlassen den vorherigen Wert wieder her; kein `contentType`; `KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false, imeAction = ImeAction.Done)`.
- Begründung: Ohne `contentType` darf ein Autofill-Dienst Passwortfelder weiter heuristisch erkennen und das Speichern anbieten; nur der Ausschluss am View-Baum schließt den Autofill-Rahmen sicher aus. Das Tresorpasswort gehört nicht in einen fremden Passwortmanager; „Passwort merken“ (S1) ist der vorgesehene Weg.

**S4 (Abbruch der Schlüsselableitung): Abbrechen beendet den Vorgang ohne Entsperren; die Ableitung selbst läuft zu Ende.**
- Befund: `cryptomator/core/kdf.rs` ruft `scrypt::scrypt` in einem Stück; es gibt keinen Haltepunkt zwischen Blöcken.
- Entscheidung: „Abbrechen“ ruft `task.cancel {id}` und schließt das Blatt sofort. `UnlockOptions` bekommt das Feld `cancel: Option<Arc<AtomicBool>>`; `unlock_entered` prüft es nach `unlock_storage` und vor `register.install`. Ist es gesetzt: Masterkey sofort verwerfen (`Zeroizing` fällt), Ergebnis `UnlockError::Cancelled`, Bridge-Code `cancelled`, kein Merken, kein Sitzungseintrag. Die Aufgabe endet als abgebrochen; das Blatt ist dann schon zu, also keine Meldung.
- Verworfen: entsperren und danach `vault.lock` (kurz installierte Sitzung, die ein anderer Verbraucher schon nutzen könnte).

**S5 (Bildvorschau im Speicher): Ja, unter Bedingungen.**
- Bedingungen: nur auf `VaultCompareDetail` mit `FLAG_SECURE` (S6); Dekodieren aus dem eigenen Provider (`ContentResolver.openInputStream` auf die URI von `SeDocumentsProvider`) mit `BitmapFactory.decodeStream` und `inSampleSize` (lange Kante ≤ Bildschirmbreite; das ist die Speichergrenze des Geräts, keine Produktgrenze); nur in `remember`-Zustand des Detailbildschirms; `bitmap.recycle()` in `onDispose` und beim Ereignis `locked`; kein Coil/Glide, kein `Thumbnails.kt`, kein `cacheDir`, kein Miniaturbild.
- Begründung: Klartext liegt beim Lesen ohnehin im Prozessspeicher (K46 erlaubt Speicher); ohne Vorschau ist der Bildvergleich wertlos.

**S6 (`FLAG_SECURE`): Ja, und weiter als der Entwurf. Erweitert.**
- Geschützt: `VaultUnlockSheet`, `VaultCreateScreen`, Blatt „Passwort ändern“, `VaultCheckScreen`, `VaultCompareScreen`, `VaultCompareDetail`, `MergeScreen` im Tresormodus und der Dateien-Reiter, solange sein Ort unter `vault://` liegt. Ausgenommen: `VaultsScreen` und `VaultSettingsScreen` außer dem Passwortblatt (nur Bezeichnungen).
- Umsetzung: Blätter mit `ModalBottomSheetProperties(securePolicy = SecureFlagPolicy.SecureOn)`, Dialoge mit `DialogProperties(securePolicy = SecureFlagPolicy.SecureOn)`; ganze Bildschirme über `@Composable fun SecureWindow()` in `A/ui/vaults/SecureWindow.kt` (≤ 80 Zeilen) mit Zähler je `Window` (`WeakHashMap<Window, Int>`): der erste Nutzer setzt `FLAG_SECURE`, der letzte löscht es. Ohne Zähler löscht ein verlassener Unterbildschirm den Schutz des noch offenen übergeordneten.
- Begründung: Der Dateien-Reiter zeigt dieselben Klartextnamen wie der Vergleich; Bildschirmfotos und die Vorschau in „Zuletzt verwendet“ landen auf Platte (K45).

**S7 (Zwischenablage): Kopieren und Ausschneiden im Tresor-Zusammenführen sperren, Einfügen bleibt. Bestätigt, mit vollständigem Weg.**
- Umsetzung: `MergeScreen` im Tresormodus stellt `LocalClipboard` und `LocalClipboardManager` durch Hüllen bereit, deren Schreibaufrufe (`setClipEntry`, `setText`, `setClip`) nichts tun und deren Lesen weiterreicht; zusätzlich `LocalTextToolbar` mit `onCopyRequested = null` und `onCutRequested = null`. Die Hüllen fangen auch Tastenkürzel (Strg+C, Strg+X, Strg+Einfg, Umschalt+Entf) einer Hardware-Tastatur ab, die am Menü vorbeigehen. Vergleich und Detail zeigen Text ohne `SelectionContainer`.
- Begründung: Die Zwischenablage ist für jede App mit Eingabefokus und für Tastatur-Apps lesbar und landet im Verlauf mancher Tastaturen auf Platte; Einfügen bringt keinen Klartext hinaus.

**S8 (Sperren bei Hintergrund und Bildschirm aus): Kern allein über `HostMonitor` bestätigt; offene Dateien halten nur die Hintergrundsperre auf. Abweichung von der Empfehlung „Frist, dann sperren“.**
- Bestätigt: Gesperrt wird nur im Kern (`sys.hostState` → `SessionRegister::set_host`, `tick`), nie aus dem Lebenszyklus einer Aktivität.
- Bildschirm aus: sperrt sofort, erzwungen; Sperrteilnehmer bekommen die bestehende Frist `participant_budget` (15 s), danach sperrt der Kern auch bei laufendem Lesevorgang.
- Hintergrund: Solange eine Datei des Orts offen ist (dieselbe Prüfung wie beim Leerlauf, `Arc::strong_count(session.keys()) > 1`, also auch ein offener Deskriptor des Providers für eine fremde App), sperrt die Hintergrundzeit diesen Ort nicht. Schließt die letzte Datei und ist die Hintergrundzeit schon abgelaufen, sperrt der nächste `tick` sofort. Umsetzung: `SiteClock` bekommt `open_files: bool`; `lock_policy::due` meldet `Background` nur bei `!open_files`.
- Begründung: Der Provider (U23) ist genau dafür da, dass eine fremde App Tresordateien liest, während se im Hintergrund ist; mit „Frist, dann sperren“ bräche nach einer Minute jedes Lesen ab. Der Schutz des liegengelassenen Geräts kommt von „Bildschirm aus“, der ohne Aufschub bleibt. Tresor-Sync-Jobs übertragen nur Chiffrat und hängen nicht am Entsperren.
- Test (U31): `vault_task_android_background_waits_for_open_file` (Rust, `vault/os/shared/session_timer.rs`-Tests): offene Datei + Hintergrund > Minuten → entsperrt; Datei zu → gesperrt; Bildschirm aus mit offener Datei → gesperrt.

## Punkt 9: restliche Grenzen ohne technische Ursache (R15c), ein Fehler

Stand: im Quelltext fertig, nicht gebaut, nicht getestet, nicht committet. Jede geänderte Rust-Datei mit `rustfmt --check` geprüft (leere Ausgabe), jede unter 500 Zeilen; `screen/core/` ohne `cfg` und ohne OS-Crates.

- **`SessionLimit`:** `RegistryRefusal` ganz entfernt (`screen/core/registry.rs`). `ScreenRegistry::register` gibt `Result<SessionHandle, Infallible>` zurück, damit die `.unwrap()`-Aufrufer in `screen/os/shared/input_keys.rs:205-206` (nicht meine Datei) unverändert bauen. `host_server.rs`: `Start::register` gibt `Option<SessionHandle>`, der Zweig `SessionLimit` ist weg, `None` bleibt `permission_denied`. Drahtcode `ScreenErrorCode::SessionLimit`, `viewer_client.rs::refusal_text` und `app/core/screen_texts.rs` unverändert. Test `screen_task_old_host_session_limit_refusal_stays_readable` in `registry_tests.rs` (kein gleichwertiger vorhanden).
- **`MAX_LIST_ENTRIES` (4096):** entfernt. Ursache der Begrenzung ist allein die Rahmengröße: `MAX_WINDOWS_BYTES` (1 MiB) liest jeder Betrachter der Protokollversion 1 höchstens (`read_frame`), daher bleibt die Byte-Anpassung `fit` in `bounded`; der Schutz gegen übergroße Rahmen eines fremden Peers sitzt unverändert in `read_frame`/`write_frame` (`tag_limit`). `validate` prüft nur noch `MAX_LIMITS`. Aufteilen in mehrere Rahmen braucht eine Protokollerweiterung (offen, siehe unten). Tests in neuer `window_list_tests.rs`.
- **App-Namen:** Kürzung auf 128 Zeichen in `screen_host.rs` entfernt (JNI-String ohne Längengrenze). Hinweis: Auf dem Draht kürzt `window_list::bounded` App-ID und Name weiter auf `MAX_NAME_BYTES` (64 Byte), siehe offene Grenzen.
- **`MAX_ISSUES` (500):** entfernt; Doppelte erkennt jetzt `ScanData::issue_keys` (`HashSet`) statt `Vec::contains` (sonst quadratisch ohne Grenze).
- **`MAX_VIEW_LIMIT` (500):** entfernt. Die Antwort ist ein JNI-String im eigenen Prozess (kein Binder, keine Nachrichtengröße); es gilt die verlangte Menge, ohne `limit` alle Zeilen ab `offset` (`view_limit`). Kotlin: `FilesApi.MAX_WINDOW` entfernt, `scanView` setzt nur `limit ≥ 1`; `ScanWindow.fetchAll` holt ein Fenster von `max + 1` Zeilen statt Schleife über 500er-Fenster.
- **Fehler Verdrängung:** `ScanRegistry::insert` entfernt bei mehr als `MAX_SESSIONS` (6) nur die älteste beendete Sitzung, eine laufende nie; `ScanSession.finished` setzt `FinishOnDrop` beim Ende der Aufgabe (auch bei Fehler oder Abbruch). Das Feld `cancel` und `stop()` sind entfallen, weil nichts mehr eine laufende Sitzung verdrängt (Abbruch über `task.cancel` wie bisher).

## Punkt 9b: Reste aus Punkt 9

Stand: im Quelltext fertig, nicht gebaut, nicht getestet, nicht committet; `rustfmt --check` leer, jede Datei unter 500 Zeilen.

- `wire_tests.rs`: Import von `MAX_LIST_ENTRIES` gestrichen; 4097 Einträge gehen jetzt ganz hinaus (eigener Puffer, damit `sink` leer bleibt); die Escape-Prüfung nutzt die Zahl 4096.
- Kommentare: `wire_hello.rs` (`SessionLimit` nur von Hosts vor R15c), `wire.rs` (`MAX_WINDOWS_BYTES` ohne Eintragszahl).
- `register` → `SessionHandle`: **nicht umgesetzt**. Grund: `screen/core/host_server.rs` (nicht im Dateibereich dieses Laufs) ruft `register` mit `Ok`/`Err(never)` auf und bräche. Nötig sind drei Änderungen zusammen: `registry.rs` Rückgabe `SessionHandle` (und `Ok(...)` am Ende → Wert, `use std::convert::Infallible` streichen); `host_server.rs` Zeilen 124–127 `registered = Some(self.context.registry.register(new));`; `input_keys.rs:205-206` und `registry_tests.rs` die `.unwrap()` streichen.
- `window_list.rs`: `MAX_TITLE_BYTES`, `MAX_NAME_BYTES`, `MAX_LIMITS` und `cut` entfernt. `MAX_LIMITS` hatte keine Ursache außer der früheren 16-KiB-Reserve; `bounded` misst jetzt das Gerüst des Rahmens (JSON ohne Einträge) und passt Grenz-Hinweise, Fenster und Apps ganz nach Bytes ein. `validate` prüft die Fenster-IDs auf die Grammatik der Quell-IDs (Ursache: der Betrachter schickt sie in `SET_MODE` zurück); eine ungültige Monitor-ID wird geleert statt gekürzt. `MAX_ID_BYTES` (64) bleibt: Grammatik von `SourceId`.
- `screen_host.rs`: `MAX_REASON` und die Kürzung des Encoder-Namens entfernt. Technische Ursache für eine Obergrenze: beide reisen in einem Steuerrahmen (`MAX_CONTROL_BYTES`, 64 KiB; Encoder in `STATE`, Grund als `ERROR`-Detail). `MAX_WIRE_TEXT_BYTES = MAX_CONTROL_BYTES / 2` (JSON-Länge); längere Texte werden abgelehnt, nie gekürzt.
- `scanner/os/shared.rs`: `MAX_ERROR_PATHS_TRACKED` entfernt; `distinct_failures` sendet jeden Pfad einmal (Menge, erste Meldung gewinnt). Der Typ `Mutex<Vec<(String, String)>>` bleibt, damit `walk.rs` und `search_recursive_access_task.rs` unverändert bleiben; sonst ändert sich für die Desktop-GUI nur, dass mehr als 500 Pfade ankommen.
- Neu gefunden: `rscan/os/shared/rscan.rs:29` `MAX_ERRORS_TRACKED = 500` (entfernter Scan, gleiche Wirkung) und `rscan/os/shared/walk_state.rs:321` `MAX_DIAGNOSTIC_BYTES = 4096` (kürzt Fehlertexte).

## Punkt 10: U36 (Installer und Linux-Skript)

Stand: im Quelltext fertig, nicht gebaut, nicht ausgeführt, nicht committet. `sh -n install-linux.sh` ohne Befund; die Prüfungen `perm_task_installer_uninstall_removes_service_static` und `licenses_task_installer_installs_file` statisch nachgerechnet (Bedingungen erfüllt).

- `native/installer.nsi`: Lizenzen `licenses\Rust-Crates\THIRD-PARTY-LICENSES.txt` und `licenses\OpenH264\BINARY_LICENSE.txt` mit `Delete`/`RMDir` vor `RMDir licenses`; Ende `SEC_MAIN` `se.exe desktop restart`; `Section /o "se Desktop (optional)" SEC_DESKTOP` (`desktop enable --json`, Fehler: `DetailPrint`, Meldung nur interaktiv); `/DESKTOP=1` in `.onInit` vor dem Dokany-Sprung; Deinstallation: `se.exe desktop remove` nach `taskkill` vor `--unregister`; Kontodienst nach Vorlage U24 nur, wenn `sc.exe query SmartExplorerExecAccounts` ihn findet, Rückfrage `se-remote` Vorauswahl Nein, stille Deinstallation ohne UAC (nur `DetailPrint` mit Befehl); `DeleteRegValue` für `SmartExplorerSync`, `SmartExplorerDrives` und als Rückfall `SmartExplorerDesktop`.
- `install-linux.sh`: `umask 022`; Usage `usage: $0 [--dry-run] [--cli-only] [--with-fuse] [--with-desktop]`, `-h|--help` mit Exit 0; `--with-desktop` mit `--cli-only` → Exit 2 „se Desktop braucht die Desktop-App“; `X-KDE-Wayland-Interfaces=org_kde_plasma_window_management` im Heredoc; Reihenfolge nach H9: Programme und `.desktop` → `check_fuse` (fusermount3, setuid, `/dev/fuse`; `fuse3` nur mit `--with-fuse` über `as_root` aus den Paketquellen; jede Zeile beginnt mit `fuse3:`) → `desktop enable` (nur `--with-desktop`) → `desktop restart` (jede Installation der Desktop-App). `/etc/fuse.conf` bleibt unverändert; es gibt keine Deinstallation im Skript, also auch kein Entfernen von `fuse3`.

## Punkt 11b: Fern-Scanner

- `rscan/os/shared/rscan.rs`: `MAX_ERRORS_TRACKED` (500) entfernt. `walk_state.rs`: `record_failure` liefert jeden Pfad einmal (Menge `failed_seen`, erste Meldung gewinnt).
- `walk_state.rs`: `bounded_text` kürzt nicht mehr (4096 Byte ohne Ursache: die Texte bleiben im Prozess, kein Rahmen). Der Name bleibt wegen der Aufrufer in `search.rs` (nicht meine Datei).
- Tests im Modul `walk_state::tests`: `drive_task_remote_scan_delivers_every_failed_path_once`, `drive_task_remote_scan_keeps_long_diagnostics_whole`.

## Punkt 11: U31 (VLT-M11)

Stand: im Quelltext fertig, nicht gebaut, nicht getestet, nicht committet. Rust mit `rustfmt --check` leer, jede Datei unter 500 Zeilen.

**Rust:**
- `mobile/os/shared/domains/vaults.rs` (list, show, lock, lockAll, setSettings, forgetPassword, Ereignis `vault`, Konfliktmeldung), `vault_keys.rs` (detect, add, remove, unlock, create, changePassword, rememberSaved, savedSecret, syncCreate), `vault_compare.rs` (compare, check, compareItems, compareText, resolve, resolveAll, mergeApply); Registrierung in `domains/mod.rs` (Domäne `vault`, `on_init` → `install_events`).
- `vault/os/shared/unlock.rs`: `unlock_entered_cancellable` mit `UnlockTarget` und `UnlockError::Cancelled` (Code `canceled`); `unlock_entered` ruft sie mit nie gesetztem Flag. Export in `vault/os/mod.rs`, `vault/mod.rs`.
- S2 umgesetzt als „private Aufgabe“ statt `vault_redact`: `TaskRecord::set_private` (`mobile/core/tasks.rs`) unterdrückt Meldungen, ersetzt Fehlerpfade und den Fehlertext; `TaskCtx::keep_names_private`, Fehlerprotokoll mit `PRIVATE_FAILED` (`runtime.rs`); Titel über `crumbs::task_title` („Tresor <Bezeichnung>“) in `transfer.rs`, `import.rs`, `delete.rs`, `scan.rs`; `Loc::is_vault`. Vergleichsaufgaben: voller Text nur im Aufgabenergebnis, öffentlich „Die Aktion im Tresor ist fehlgeschlagen.“ Grund der Abweichung: ein Ersetzen bekannter Namen erfasst nicht jeden Namen.
- `sync_run.rs::run_vault` → `vaults::notify_conflicts`: `syncProblem` mit `jobId` `vault:<job>`, Titel „Tresor-Sync: Konflikte“, Text „<n> Konflikte in <Bezeichnung>.“

**Kotlin:** `api/VaultApi.kt`; `ui/vaults/{VaultsScreen (Host, Liste, Eintragen, Austragen, Frage ungespeicherte Änderungen), VaultUnlockSheet, VaultCreateScreen, VaultSettingsScreen (mit Passwortblatt), VaultCheckScreen, VaultCompareScreen, VaultCompareDetail, VaultMerge, VaultSyncSheet, VaultTexts, VaultViewModel, SecureWindow (+ NoAutofill), VaultSavedPassword}.kt`; geändert `ui/more/MoreScreen.kt` (Zeile „Tresore“), `ui/AppNav.kt` (`OpenVaultCompare`, `openJobConflicts` mit `vault:`), `core/CoreEvents.kt` (`VaultChanged`), `ui/sync/JobCards.kt` („Vergleichen“), `ui/files/FilesScreen.kt` (`SecureWindow` unter `vault://`), `AndroidManifest.xml` (`USE_BIOMETRIC`).

**Abweichungen von § U31 (mit Grund):**
1. 22 Bridge-Methoden: C7.5 (19, mit `vault.show`), `vault.compareItems` (U31-17), `vault.rememberSaved`, `vault.savedSecret` (S1).
2. `vault.compareItems {list, revision, group?, offset?, limit?}` → `{items, nextOffset, total}` statt `{job, group, offset}`: die Revision erkennt einen veralteten Vergleich, `list` deckt auch die Prüfung ab (`check:<site>`); ohne `limit` alle Einträge (keine Grenze ohne Ursache). `vault.check` liefert `list` und `revision`.
3. `vault.compareText` liefert zusätzlich `rows` (aus `crate::linemerge`, mit dessen Frist), `crlf`, `finalNewline`; die App rechnet keinen Unterschied selbst.
4. Zusammenführen nicht über `MergeScreen`, sondern `VaultMerge.kt` (Zeilen A/B wählen): `MergeScreen` arbeitet auf `sync.mergeRows` allgemeiner Jobs und darf den Tresor nicht berühren. S7 ist damit ohne Textfeld erfüllt (kein auswählbarer Klartext).
5. Tresor-Sync anlegen über `VaultSyncSheet` (Kopie, Ordner, anderer Ort desselben Tresors; Richtung) statt einer Ortsart „Tresor“ im Job-Editor: Tresor-Jobs haben feste Werte (C5) und laufen nur über `vault.syncCreate`.
6. `OpenVaultCompare` öffnet im Reiter „Mehr“ (nicht „Sync“): `SyncScreen.kt` liegt nicht in meinem Bereich, die Tresor-Seiten liegen unter „Mehr → Tresore“.
7. Benachrichtigung (U31-13) über den bestehenden Weg `syncProblem` → `Notifications.syncProblem` mit Job-ID `vault:<job>`; `AppNav` leitet sie auf den Vergleich. `Notifications.kt`/`HostMonitor.kt` bleiben unverändert (nicht mein Bereich).
8. Nicht umgesetzt, weil der Kern die Angabe nicht liefert: Prüfsummen-Kürzel im Detail (`SideState` ohne Hash), „<n> zurückgestellt“ auf der Jobkarte (`SyncJob` ohne Zählung), Name des Sicherungsordners (Zeile zeigt „Sicherung: vault-removed“), Sperren der Prüfaktion bei belegter Paarsperre.
9. Bildvorschau und „Öffnen (A)/(B)“ über Provider-URIs `vault:<site>/<pfad>`: nur, wenn der Tresor unter „Ordner für Apps bereitstellen“ freigegeben und entsperrt ist; sonst „Keine Vorschau“ bzw. Hinweis.
10. `VaultsScreen.kt` hat 382 Zeilen (Vorgabe ≤ 350), weil Host, Eintragen, Austragen und die C3.3-Frage dort liegen.
11. S4 ohne Feld `UnlockOptions.cancel`: `daemon/os/shared/ipc_vault.rs` baut `UnlockOptions` mit Feldliteral und liegt nicht in meinem Bereich; daher die zusätzliche Funktion.
12. S8 (offene Datei hält die Hintergrundsperre auf) nicht umgesetzt: braucht `vault/core/lock_policy.rs` (`SiteClock.open_files`, `due`) und `vault/os/shared/session_timer.rs`; mein Bereich in `N/vault/**` umfasst nur den Abbruch.
13. Fehlerart für S1-Ablehnungen `invalid` (statt `invalid_argument`).

## Punkt 12: Antworten E13-S1 bis S13

Grundlage: `festlegungen.md` § U44 (U44-4, U44-5, U44-10, U44-11, U44-13, Fragen S1–S13), § U48 (U48-4), § U49, § U51, § U53; Code gelesen: `share/core/{fs_request.rs:240, batch_wire.rs:20–62, keepalive.rs:8–24}`, `agent_proto/core/{credit.rs:33, features.rs}`, `agent_proto/os/shared/write_new.rs:14–60`, `ftp/core/staging.rs`, `ftp/core/ftp.rs:289–310`, `vfs/core/promotion.rs:34–60, 158–170`, `transfer/os/shared/engine/{publish.rs:121, util.rs:26–38}`. Gilt für U53 (E13-S1 bis S5), U48 (S7, S12, S13), U46 (S6), U49 (S8), U50 (S9, S10), U51 (S11). Wo die Antwort von § U53/§ U48 abweicht, gilt die Antwort.

**E13-S1 (Share `ReadAt` mit `len`): übernommen, mit Bedingungen.**
- Keine neue Berechtigung; dieselbe Prüfung wie `ReadAt` vor dem Öffnen (`fs_guard_backend.rs`), auch für `open_read_range`.
- Rechnung nur mit geprüfter Arithmetik: `offset > size` → bestehender Fehler `InvalidInput`; `remaining = min(size − offset, len)` (bei `len = 0`: `size − offset`); `offset + len` wird nie ungeprüft addiert.
- Länge über das Dateiende: auf das Ende gekürzt; die angekündigte Länge ist `remaining`.
- Schrumpft die Datei während des Lesens, endet der Strom vor `remaining`; der Client muss das als Fehler „Quelle wurde während der Übertragung geändert“ werten (U44-4 Punkt 7), nie als vollständigen Bereich.
- Grenze: keine neue Zahl. Technische Ursache der bestehenden Grenze: QUIC lässt je Verbindung 64 bidirektionale Ströme zu (`IROH_MAX_CONCURRENT_BIDI_STREAMS`), 4 bleiben für Steuerung und Blättern (`CONTROL_STREAM_RESERVE`); Bereichsströme zählen gegen `admission` (60) wie jede Übertragung, gleich ob sie dieselbe Datei betreffen.

**E13-S2 (Bereichsschreiben Share und Agent): übernommen, mit geänderten Bedingungen.**
- Den Stufennamen wählt allein der Wirt (`<Ziel>.se-upload-<16 Hex aus getrandom>`); der Client nennt nur das Ziel. Erzeugen ausschließlich (`create_new`), Elternordner geprüft wie bei `WriteNew` (Share: `fs_guard_*`; Agent: `ensure_destination_parent_plain` und `secure_staging_file`), Recht „neue Datei“, nicht in schreibgeschützten Räumen.
- **Geändert:** Der Wirt behält das beim Erzeugen geöffnete Datei-Handle in der Stufentabelle und schreibt jeden Bereich über **dieses** Handle (`write_all_at`/`seek_write`); er öffnet die Stufe nie erneut per Pfad. `Finish` prüft Länge und Abdeckung über dasselbe Handle.
- **Geändert:** Ein Bereich wird beim Annehmen der Anfrage als „belegt“ eingetragen (`pending`); überlappt er `covered ∪ pending`, lehnt der Wirt ab. Nach vollständigem Empfang wechselt er nach `covered`; scheitert er, wird er aus `pending` entfernt und darf neu geschrieben werden. Ohne `pending` könnten zwei gleichzeitige, überlappende Anfragen beide angenommen werden.
- `offset + len` mit Überlaufprüfung, außerhalb `[0, size)` abgelehnt; mehr oder weniger als `len` Byte → Fehler, Bereich nicht abgedeckt.
- Jede `StageWriteAt`/`WriteAt` und `Finish` prüft das Schreibrecht erneut (wie jede `WriteNew`); ist es entzogen oder der Raum schreibgeschützt geworden, wird abgelehnt und die Stufe entfernt.
- Stufe gehört der öffnenden Verbindung und deren Prinzipal; fremde Verbindung → „unbekannte Stufe“.
- Platz: **Geändert:** Der Wirt führt die Summe der noch nicht geschriebenen Bytes aller offenen Bereichsstufen (Reservierung) und lehnt `Open` mit `StorageFull` ab, wenn der freie Platz kleiner als `Reservierung + size` ist. Ursache: dünn besetzte Dateien belegen den Platz erst beim Schreiben; ohne Summe können mehrere Stufen den Datenträger gemeinsam überbelegen.
- Zahl offener Stufen je Verbindung: Share `admission` (60, Ursache wie E13-S1), Agent `CREDIT_REQUEST_LIMIT` (Ursache: Kreditbudget der Verbindung). Darüber `Busy`.
- Aufräumen bei Verbindungsende, `DiscardStage`, Zeitablauf ohne Fortschritt und beim Start des Wirts wie die vorhandenen `.se-upload-`-Stufen.

**E13-S3 (LZ4): übernommen, mit Bedingungen.**
- Nur Dateiinhalt; nie Steuerrahmen, Pfade, Namen, Fehlertexte, Listings, Hashes.
- Jeder Block unabhängig, kein Wörterbuch, kein Zustand zwischen Blöcken oder Dateien. Damit gibt es kein Kompressionsorakel nach CRIME-Art: Dort mischt der Angreifer eigene Bytes mit einem Geheimnis im selben Kompressionskontext; hier stammt jeder Block aus genau einer Datei. Was ein Beobachter der verschlüsselten Verbindung lernt, ist höchstens die Komprimierbarkeit einzelner 256-KiB-Blöcke; das gilt für Direct und Räume gleich und ist hinnehmbar.
- Harte Grenze mit Ursache: Rohlänge ≤ 262 144 (= `CHUNK`, der Empfangspuffer); größer → `InvalidData` ohne Entpacken. Entpacken mit `decompress_into` in einen Puffer genau der angesagten Rohlänge; abweichende Länge oder unvollständiger Block → `InvalidData`.
- CPU-Last eines bösartigen Senders: LZ4-Entpacken ist linear in der Rohlänge; da der Kredit (Agent) bzw. das Stromfenster (Share) die Rohbytes zählt, ist der Aufwand je Zeit durch dieselbe Grenze gedeckelt wie bei rohen Daten. Kredit mit Rohlänge belasten (wie entworfen).
- Aushandlung nur über die authentisierten Fähigkeiten (`fs_lz4_v1` im Hello der QUIC/TLS-Sitzung, `lz4-v1` im Versionstext im SSH-Kanal); ein Mittelsmann kann weder aufwerten noch herabstufen. Ohne Merkmal beider Seiten nie komprimiert.

**E13-S4 (Agent-Labels, alte Gegenstellen): übernommen, mit Bedingungen.**
- `SegStageOpen` nutzt dieselben Wächter wie `WriteNew` (`ensure_destination_parent_plain`, `create_new`, `secure_staging_file`); `WriteAt` schreibt über das Handle aus `SegStageOpen` (E13-S2), es gibt keinen eigenen Pfadweg. Der Umsetzer nennt die Prüffunktionen im Bericht.
- Ohne Label `range-v1`/`lz4-v1` sendet der Client keinen neuen Rahmen; er nutzt den Bestandsweg ohne Fehler. Ein Server, der einen unbekannten Tag erhält, antwortet wie heute mit Fehler (kein stilles Ignorieren).
- Der Agent läuft unter dem Konto des Dienstes (`run_as`); keine zusätzlichen Rechte.

**E13-S5 (QUIC-Regler, Fenster, Speicher): übernommen, mit Bedingungen.**
- `SE_SHARE_QUIC_CC` bleibt Messschalter: nur `bbr3` wählt BBRv3, jeder andere Wert und Fehlen `cubic`; keine Oberfläche, keine Einstellung. Wer die Umgebung des Prozesses setzt, steuert ihn ohnehin.
- Vorgabe bleibt `cubic`, bis die Entscheidungsregel U44-11 (b) erfüllt ist (BBRv3 ist ein experimenteller Entwurf).
- Fenster unverändert (P3). Speicher des Wirts: Die Verbindungsgrenze `CONNECTION_RECEIVE_WINDOW` (64 MiB) begrenzt die nicht bestätigten Bytes aller Ströme; die 16 MiB je Strom sind Obergrenzen, keine Zuteilung. Bedingung: das Budget von U50 darf `STREAM_RECEIVE_WINDOW` und `CONNECTION_RECEIVE_WINDOW` nicht erhöhen, nur senken; eine Erhöhung braucht eine neue Sicherheitsantwort.

**E13-S6 (FTP-Proben): (a) geändert: darf entfallen, mit Bedingung; (b) übernommen: bleibt.**
- (a) `FTP_PROBE_STAGE_NAME` darf `false` werden, **nur** für Stufennamen mit 64 Bit aus `getrandom` (`engine::util::random_hex` ohne Rückfallzweig, oder `vfs::unique_staging_path`). Ursache: Ein fremdes Objekt mit diesem Namen müsste vor `STOR` existieren; der Name ist vorher niemandem bekannt (Zufall 2⁻⁶⁴). Liefert `random_hex` den Rückfallwert (Zeit/Zähler, vorhersagbar), muss die Probe für diesen Namen laufen (oder `random_hex` meldet Fehler); der Umsetzer kennzeichnet den Rückfall dafür.
- (b) `FTP_VERIFY_STAGE_BEFORE_RENAME` bleibt `true`. Ohne die Prüfung veröffentlicht `RNFR`/`RNTO` ein vertauschtes Objekt (Ordner, Link) unter dem Zielnamen, und se meldet Erfolg für Daten, die es nicht geschrieben hat. Die Probe des Ziels bleibt ohnehin.

**E13-S7 (SFTP-Segmentstufe, Wiederöffnen per Pfad): geändert: Schreiber-Zweig nur mit Prüfung der Ordnerkette.**
- Befund: SFTP v3 kennt kein `O_NOFOLLOW` und keine Inode in den Attributen; `fstat` kann einen Tausch durch einen Link nicht sicher erkennen, und ein vertauschter Pfad lässt die Bereiche in eine **fremde Datei** schreiben, die das se-Konto schreiben darf. Eine Bindung an `fsync@openssh.com`/`posix-rename` ändert daran nichts.
- Entscheidung: `SFTP_SEGMENTED_WRITE = true` ist erlaubt, `open_segmented_stage` liefert aber nur dann `Some`, wenn **jeder** Ordner vom Elternordner der Stufe bis zur Wurzel (`/`) diese Regel erfüllt: Eigentümer ist das se-Konto (`uid` der frisch erzeugten Stufe) oder `uid 0`, **und** der Ordner ist für Gruppe und andere nicht beschreibbar (`mode & 0o022 == 0`) **oder** trägt das Sticky-Bit (`mode & 0o1000`) bei Eigentümer `uid 0`. Dann kann kein anderes Konto außer root einen Eintrag dieser Kette umbenennen oder ersetzen.
- Liefert der Server für einen Ordner keine `uid` oder keine Rechte (z. B. Windows-Server), gilt die Regel als nicht erfüllt → ein Strom, ohne Fehler (U44-4 Punkt 11).
- Die `stat`-Ergebnisse der Kette werden je Auftrag und Ordner einmal ermittelt (Kosten: eine Rundreise je Ordnerebene je Auftrag).
- Zusätzlich bleibt die `fstat`-Prüfung jedes Bereichs-Handles vor dem ersten Byte (Größe = gesetzte Länge, Eigentümer = se-Konto) als zweite Linie.

**E13-S8 (`se cp`/`get`/`put` auf der Engine): übernommen, mit Bedingungen.**
- „Ziel fehlt“ wird mit `symlink_metadata`/`lstat` geprüft (nicht `exists`, das einen hängenden Link als fehlend sähe); ein Link als Wurzel bleibt im Bestandsweg.
- Die Engine veröffentlicht ohne Ersetzen; ein Wettlauf endet als Fehler „existiert bereits“, nie als Überschreiben. `--force` und vorhandene Ziele bleiben im Bestandsweg.
- Links und Spezialdateien im Baum: gemeldete Auslassungen wie im Bestand (`print_notices`), Exit-Code wie im Bestand.

**E13-S9 (`transfer-hints.json`): geändert: übernommen mit Salz und Prüfung beim Lesen.**
- Schlüssel = SHA-256 über ein zufälliges 32-Byte-Salz der Installation (in derselben Datei, beim ersten Schreiben aus `getrandom`) und die Identität. Ursache: ohne Salz lassen sich Schlüssel installationsübergreifend vorausberechnen und Nutzer über gleiche Server verknüpfen.
- Rechte: Unix Datei `0600`, Ordner `0700`; Windows Standard-ACL des Profils. Schreiben atomar (Stufe + Umbenennen).
- Beim Lesen: unbekannte oder unlesbare Einträge verwerfen; Grenzwerte außerhalb des gültigen Bereichs (0 oder über der Protokollgrenze des Flows) und Zeitstempel in der Zukunft verwerfen; nie Pfade oder Namen speichern, auch keine Tresorpfade.

**E13-S10 (Sensoren, Priorität, Anzeige): übernommen, mit Bedingungen.**
- Die genannten Lesezugriffe brauchen keine Rechte. Fehlt eine Quelle oder ist der Zugriff verboten (Android verweigert Apps `/proc/stat` seit Android 8; `/sys/class/net/*/speed` meldet `EINVAL` bei getrennter Leitung), gilt der Wert als unbekannt, nie als Fehler.
- Priorität nur senken oder auf den Startwert zurücksetzen, nie über den Startwert anheben (keine negative `nice`, kein `THREAD_PRIORITY_ABOVE_NORMAL`).
- `limiter_text` im MCP-Fortschritt nennt nur Laufwerksbezeichnung/Gerät und Leitungsrate, nie Datei- oder Ordnernamen; bei Tresororten „Tresor <Bezeichnung>“ (S2 aus U31).

**E13-S11 (`COPY_FILE_REQUEST_COMPRESSED_TRAFFIC`): übernommen: nicht setzen.** `WINDOWS_REQUEST_COMPRESSED_TRAFFIC` bleibt `false`, ohne Schalter in einer Oberfläche. Ursache: `CopyFile2` meldet nicht, ob die SMB-Verbindung verschlüsselt ist, und Kompression vor Verschlüsselung ist der Fall der Frage.

**E13-S12 (FTP-Bereiche, Pool-Anmeldungen): übernommen, mit Bedingungen.**
- Bereiche nur auf bereits angemeldeten, freien Pool-Verbindungen (mindestens zwei frei); für Bereiche wird nie eine neue Anmeldung geöffnet.
- Antwortet der Server mit 421 oder 530, endet die Datei als ein Strom ohne Fehler, die Verbindung merkt „keine Bereiche“ 15 Minuten und die gelernte Grenze sinkt; keine Wiederholungsschleife neuer Anmeldungen.

**E13-S13 (SFTP-Pakete, offene Handles): übernommen, mit Bedingungen.**
- Je Kanal höchstens `min(open-handles des Servers, 64)` gleichzeitige Handles (Ursache wie U44-5: Grenze des Servers bzw. Ulimit 1024 durch `MaxSessions` 10).
- `SSH_FX_FAILURE` oder Kanalfehler beim `OPEN` gelten als Überlast (halbieren), nicht als Dateifehler; jedes Handle wird bei Abbruch und Fehler geschlossen.

## Punkt 13: Hintergrundsperre bei offener Datei (S8 aus U31)

Stand: im Quelltext fertig, nicht gebaut, nicht getestet, nicht committet; `rustfmt --check` leer.

- `vault/os/shared/session_timer.rs::tick`: `open_files = Arc::strong_count(entry.session.keys()) > 1`; bei offenen Dateien geht der Ort ohne Hintergrundzeit in `SiteClock` (`background: None`). Damit melden `lock_policy::due` und `wake_in` für diesen Ort kein `Background`; schließt die letzte Datei, sperrt der nächste `tick` sofort, wenn die Zeit abgelaufen ist.
- **Abweichung vom Auftrag:** kein neues Feld `SiteClock.open_files`. Grund: `vault/os/shared/session_lock_tests.rs` (nicht in meinem Bereich) baut `SiteClock` mit Feldliteral und bräche. Die Wirkung ist dieselbe; `lock_policy.rs` dokumentiert sie am Feld `background`.
- Bildschirm aus: unverändert ein eigenes Signal in `set_host`, sperrt mit offener Datei sofort (Frist der Sperrteilnehmer 15 s).
- Desktop und Daemon: keine Wirkung. `due` zählt die Hintergrundzeit nur auf `LockPlatform::Android`; die Leerlaufzeit war schon vorher null, solange eine Datei offen ist; Ruhezustand und Bildschirmsperre des Desktops sind Signale und bleiben unverändert.
- Test `vault_task_android_background_waits_for_open_file` (Modul `vault::os::session::timer::open_file_tests`).

## Nächster Schritt

Bericht zu Punkt 12 und 13; danach auf den nächsten Auftrag warten.
