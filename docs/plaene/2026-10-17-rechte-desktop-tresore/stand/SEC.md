# Stand SEC (Sicherheitsagent)

Stand: 2026-10-21 (Lauf mit Punkt 8 und 9).

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

## Nächster Schritt

Bericht zu Punkt 8 und 9 an den Hauptagenten; danach Punkt 10 (U36) per Nachricht.
