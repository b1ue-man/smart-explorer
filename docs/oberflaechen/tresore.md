# Oberflächen der Tresore

Stand: 2026-10-21 (Lauf 3 des Doku-Laufs U42, VLT-M13). Grundlage: der Plan `docs/plaene/2026-10-17-rechte-desktop-tresore/strang-tresore.md`, Abschnitt C7 (Matrix T1–T22, Öffnen-Regel, CLI, IPC, Bridge), und die Stand-Dateien U05, U08, U11, U14, U20, U26, U29 und U31. Belege als `Datei:Zeile` am Quelltext dieses Stands. Abkürzungen: `N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`. Nicht gebaut und nicht ausgeführt: die Stapel-Suite prüft die Angaben. Pflege: jede Änderung an einer Tresorfähigkeit aktualisiert diese Datei im selben Commit (Regel in `docs/oberflaechen/INDEX.md`).

## 1 Oberflächen

| Kürzel | Oberfläche | Einstieg |
|---|---|---|
| CLI | `se vault list, show, detect, add, create, remove, unlock, lock, password, forget-password, settings, check, sync add/run, compare, resolve` | `N/cli/vault.rs:53-85` (Befehle), `N/cli/vault/manage.rs`, `N/cli/vault/unlock.rs`, `N/cli/vault/compare.rs`, `N/cli/vault/sync.rs`, Ausgabe `N/cli/vault/output.rs`; Vervollständigung `N/cli/vault/complete.rs` |
| Desktop | Seitenleiste „Tresore“ (Kontextmenü), Dialoge Eintragen, Anlegen, Entsperren, Eigenschaften (Seiten Allgemein, Sperren, Passwort, Wiederherstellung, Sync, Laufwerk, Prüfung), Vergleich mit Detailansicht, Zusammenführen | `N/app/core/vault_sidebar_ui.rs`, `vault_add_ui.rs`, `vault_create_ui.rs`, `vault_unlock_ui.rs`, `vault_settings_pages_ui.rs`, `vault_check_ui.rs`, `vault_compare_ui.rs`, `vault_compare_detail_ui.rs`, `vault_sync_editor_ui.rs`, `vault_actions_ui.rs`, Einstellungen `vault_ui_settings.rs` |
| Android | „Mehr → Tresore“ (`A/ui/vaults/VaultsScreen.kt:63`), Entsperrblatt, Anlegen, Einstellungen mit Passwortblatt, Prüfung, Vergleich mit Detail und Zusammenführen, Jobkarte, Benachrichtigung bei Konflikten | `A/ui/vaults/`: `VaultUnlockSheet.kt`, `VaultCreateScreen.kt`, `VaultSettingsScreen.kt`, `VaultCheckScreen.kt`, `VaultCompareScreen.kt`, `VaultCompareDetail.kt`, `VaultMerge.kt`, `VaultSyncSheet.kt`, `VaultSavedPassword.kt`, `SecureWindow.kt` |
| Bridge | JSON-Methoden `vault.*` und `sync.*` der Android-Bibliothek | `N/mobile/os/shared/domains/vaults.rs:24-29`, `vault_keys.rs:34-42`, `vault_compare.rs:30-36`, `mod.rs:152-155`; Kotlin `A/api/VaultApi.kt` |
| IPC | Loopback-IPC GUI/CLI ↔ Hintergrunddienst: `IpcRequest::Vault { token, req: VaultIpcRequest }` | `N/daemon/os/shared/ipc_protocol_vault.rs:78-153` (Varianten), Client `ipc_vault_client.rs`, Host `ipc_vault.rs` |
| MCP | Werkzeug `se` führt jeden `se vault`-Befehl aus; eigenes Werkzeug gibt es nicht | `N/cli/mcp/catalog.rs` (Katalogtext nennt `se vault`) |
| Laufwerk | `se drive mount vault://<SITE>` (S2-Weg, Teildokument `docs/oberflaechen/laufwerke.md`, Abschnitt 6) | `N/cli/drive.rs:363` |

## 2 Fähigkeiten T1 bis T22

Jede Zeile nennt mindestens einen Beleg. „Ausnahme“ steht dort, wo eine Oberfläche die Fähigkeit bewusst nicht hat; der Grund steht in der Zeile oder in Abschnitt 10. Die Zeilen folgen der Matrix C7.1 des Plans.

| Nr | Fähigkeit | CLI / MCP | Desktop | Android | Bridge und IPC | Beleg |
|---|---|---|---|---|---|---|
| T1 | auflisten mit Zustand | `se vault list` | Seitenleiste „Tresore“ | „Mehr → Tresore“ | `vault.list`; IPC `Status` | `N/cli/vault.rs:53`; `N/app/core/vault_sidebar_ui.rs:157`; `A/ui/vaults/VaultsScreen.kt:63`; `N/mobile/os/shared/domains/vaults.rs:24`; `N/daemon/os/shared/ipc_protocol_vault.rs:78` |
| T2 | erkennen | `se vault detect` | Hinweis „Dieser Ordner enthält einen Tresor“ | Ausnahme: kein Hinweis in der Dateiliste belegt (Hinweis nicht geprüft) | `vault.detect` | `N/cli/vault.rs:60`; `N/mobile/os/shared/domains/vault_keys.rs:34`; `N/app/core/vault_add_ui.rs:36` |
| T3 | eintragen | `se vault add` | „Tresor hinzufügen…“ | „Tresor hinzufügen“ | `vault.add` | `N/cli/vault.rs:62`; `N/mobile/os/shared/domains/vault_keys.rs:35`; `N/app/core/vault_add_ui.rs:36` |
| T4 | anlegen | `se vault create` (`--password-stdin`, `--shortening-threshold 36..220`) | „Neuer Tresor…“, Assistent mit Passwort-Mindestlänge 8 | „Neuer Tresor“ (`VaultCreateScreen`) | `vault.create` | `N/cli/vault.rs:64`; `N/mobile/os/shared/domains/vault_keys.rs:38`; `N/app/core/vault_create_ui.rs:66`; `A/ui/vaults/VaultCreateScreen.kt:43`; `N/vault/os/shared/password.rs:29` |
| T5 | austragen | `se vault remove` (die Dateien bleiben) | „Austragen…“ mit Rückfrage | „Austragen“ | `vault.remove`; IPC `Lock` vorher | `N/cli/vault.rs:66`; `N/mobile/os/shared/domains/vault_keys.rs:36`; `N/app/core/vault_actions_ui.rs:35` |
| T6 | entsperren | `se vault unlock` (`--password-stdin` oder `--saved-password`) | Entsperrdialog (ohne Wiederherstellungs-Link) | Entsperrblatt | `vault.unlock`; IPC `Unlock`, `UnlockSaved` | `N/cli/vault.rs:68`; `N/cli/vault/unlock.rs:26`; `N/mobile/os/shared/domains/vault_keys.rs:37`; `N/app/core/vault_unlock_ui.rs:74`; `A/ui/vaults/VaultUnlockSheet.kt:78`; `N/daemon/os/shared/ipc_protocol_vault.rs:80` |
| T7 | sperren | `se vault lock` (`<SITE>` oder `--all`, `--keep-unsaved`) | Kontextmenü „Sperren“, „Alle sperren“ | Schaltfläche, „Alle sperren“ | `vault.lock`, `vault.lockAll`; IPC `Lock` | `N/cli/vault.rs:70`; `N/mobile/os/shared/domains/vaults.rs:26-27`; `N/app/core/vault_sidebar_ui.rs:95`; `N/daemon/os/shared/ipc_protocol_vault.rs:89` |
| T8 | durchsuchen, lesen, schreiben | `vault://`-Ziele in `ls`, `stat`, `cat`, `get`, `put`, `cp`, `mv`, `rm`, `mkdir`, `search` | Reiter des Tresors | Dateien-Reiter (Klartextnamen nur im Speicher, Abschnitt 7) | `fs.*`; IPC `Open` | `N/vault/core/locator.rs:1`; `N/vault/os/shared/backend.rs:45`; `N/daemon/os/shared/ipc_protocol_vault.rs:97` |
| T9 | mit externer App öffnen | Ausnahme: Export über `cat` oder `get` | Öffnen-Regel (Abschnitt 3) | Provider-URI `vault:<site>/<pfad>` (nur entsperrt und freigegeben) | – | `N/app/os/shared/vault_open.rs:1-16`; `A/system/documents/SeDocumentsProvider.kt:1` |
| T10 | Passwort ändern | `se vault password` (`--stdin`: Zeile 1 altes, Zeile 2 neues Passwort) | Eigenschaften → Passwort | „Passwort ändern“ in den Tresor-Einstellungen | `vault.changePassword` | `N/cli/vault.rs:72`; `N/mobile/os/shared/domains/vault_keys.rs:39`; `A/ui/vaults/VaultSettingsScreen.kt:174` |
| T11 | Passwort merken und vergessen | `se vault unlock --remember`; `se vault forget-password` | Kontrollkästchen „Passwort merken“; Eigenschaften | Schalter „Passwort merken“; nur mit Displaysperre (Abschnitt 7) | `vault.unlock {remember}`, `vault.forgetPassword`, `vault.rememberSaved`, `vault.savedSecret` | `N/cli/vault.rs:74`; `N/mobile/os/shared/domains/vault_keys.rs:40-41`; `N/mobile/os/shared/domains/vaults.rs:29`; `A/ui/vaults/VaultSavedPassword.kt:158` |
| T12 | Wiederherstellungsschlüssel anzeigen | nicht angeboten; `se vault show` nennt `recovery_key_supported` (`false`) und `recovery_key_note` | Informationstext unter Eigenschaften → Wiederherstellung | Informationstext | `vault.show` (`recoveryKeySupported`, `recoveryKeyNote`) | `N/cli/vault/manage.rs:247`; `N/vault/os/shared/manage.rs:27`; `N/mobile/os/shared/domains/vaults.rs:25` |
| T13 | mit Wiederherstellungsschlüssel zurücksetzen | Ausnahme: nicht angeboten (Lizenz der Wortliste offen, Nutzerentscheidung OE1) | Ausnahme, wie T12 | Ausnahme, wie T12 | Ausnahme, wie T12 | `N/vault/os/shared/manage.rs:27` (Hinweistext) |
| T14 | Sperr-Einstellungen | `se vault settings` (`--auto-lock-idle-min`, `--lock-on-sleep`, `--lock-on-screen-lock`, `--lock-on-background-min`, `--remember-password`) | Eigenschaften → Sperren | Tresor-Einstellungen | `vault.setSettings` | `N/cli/vault.rs:76`; `N/mobile/os/shared/domains/vaults.rs:28`; `N/app/core/vault_settings_pages_ui.rs:60` |
| T15 | Tresor-Sync einrichten | `se vault sync add` (`--with replica`, `<SITE>` oder `<LOCATION>`) | „Synchronisieren…“ | „Synchronisieren“ (`VaultSyncSheet`) | `vault.syncCreate` | `N/cli/vault.rs:80`; `N/mobile/os/shared/domains/vault_keys.rs:42`; `N/app/core/vault_sync_editor_ui.rs:41`; `A/ui/vaults/VaultSyncSheet.kt:43` |
| T16 | Tresor-Sync ausführen | `se vault sync run` | „Jetzt ausführen“ | Jobkarte | `sync.run` | `N/cli/vault/sync.rs:129`; `N/mobile/os/shared/domains/mod.rs:155` |
| T17 | bearbeiten, aktivieren, entfernen | `se sync edit`, `enable`, `disable`, `remove` | Job-Editor „Tresor-Sync“ | Job-Editor mit Tresor-Abschnitt | `sync.save`, `sync.setEnabled`, `sync.delete` | `N/cli/sync_jobs.rs:56-62`; `N/mobile/os/shared/domains/mod.rs:152-154` |
| T18 | vergleichen | `se vault compare` (`--group`, `--json`) | „Tresor vergleichen“ (Metadaten; Ausnahme: kein Bildvergleich, Abschnitt 10) | `VaultCompareScreen` | `vault.compare`, `vault.compareItems`; IPC `Compare` | `N/cli/vault.rs:82`; `N/mobile/os/shared/domains/vault_compare.rs:30`; `N/app/core/vault_compare_ui.rs:114`; `A/ui/vaults/VaultCompareScreen.kt:53`; `N/daemon/os/shared/ipc_protocol_vault.rs:118` |
| T19 | auflösen | `se vault resolve <JOB> <ITEM> --action …`; `--all` mit `keep-a` oder `keep-b` | Aktionen im Vergleich | Aktionen im Vergleich | `vault.resolve`, `vault.resolveAll`; IPC `Resolve` | `N/cli/vault.rs:84`; `N/mobile/os/shared/domains/vault_compare.rs:34-35`; `N/daemon/os/shared/ipc_protocol_vault.rs:136` |
| T20 | zusammenführen (Text) | `se vault resolve … --action merge --merged-file DATEI` (Plan C7.1: Ausnahme; die Option existiert im Quelltext) | Merge-Ansicht | `VaultMerge.kt` (Zeilen A oder B wählen; kein Textfeld) | `vault.compareText`, `vault.mergeApply`; IPC `CompareText`, `MergeApply` | `N/cli/vault/compare.rs:76-78`; `A/ui/vaults/VaultMerge.kt:36`; `N/mobile/os/shared/domains/vault_compare.rs:33`, `:36`; `N/daemon/os/shared/ipc_protocol_vault.rs:132`, `:147` |
| T21 | Tresor prüfen | `se vault check` | Eigenschaften → Prüfung | `VaultCheckScreen` | `vault.check`; IPC `Check` | `N/cli/vault/compare.rs:91`; `N/app/core/vault_check_ui.rs:33`; `A/ui/vaults/VaultCheckScreen.kt:61`; `N/mobile/os/shared/domains/vault_compare.rs:31`; `N/daemon/os/shared/ipc_protocol_vault.rs:153` |
| T22 | als Laufwerk oder Ordner einhängen | `se drive mount vault://<SITE>` (kein `se vault mount`) | Kontextmenü „Als Laufwerk einhängen“ über den S2-Weg | Ausnahme: kein Laufwerk; Provider-Wurzel je entsperrtem Tresor | `docs.*` (S2); IPC `MountSource::Vault { site_id }` | `N/cli/drive.rs:363`; `N/app/core/vault_actions_ui.rs:46`; `A/system/documents/SeDocumentsProvider.kt:1` |

## 3 Öffnen-Regel

Ein Tresorpfad `vault://<site>/<pfad>` öffnet sich auf dem Desktop in dieser Reihenfolge (`N/app/os/shared/vault_open.rs:1-16`, `VaultOpenStep`):

1. Ein Ordner oder ein Tresorort öffnet sich als Reiter des Tresors über die Dienst-Sitzung. Ein gesperrter Tresor nennt sich selbst und den Weg zum Entsperren.
2. Eine Tresordatei öffnet sich in ihrem Laufwerk. `mount_path_for` (`N/daemon/os/shared/mount_paths.rs:155`) liefert den Pfad, wenn das Laufwerk eingehängt ist.
3. Ist das Laufwerk nicht eingehängt, Laufzeit vorhanden und „Beim Öffnen einhängen“ an (Standard an, `N/app/core/vault_ui_settings.rs`), bindet `ensure_mounted_for` (`mount_paths.rs:189`) den Tresor auf dem Thread `vault-mount` ein; die Oberfläche bleibt frei.
4. Sonst oder bei einem Fehler fragt der Dialog „Klartextkopie öffnen?“ (`N/app/os/shared/vault_open.rs:55`). Die Kopie entsteht nur nach Zustimmung.

Android kennt keine lokalen Pfade: Die Datei öffnet sich über die Provider-URI, sofern der Tresor entsperrt und für Apps freigegeben ist.

## 4 Sperrauslöser je Plattform

Die Gründe stehen in `N/vault/core/lock_policy.rs:29-47`; die Texte für Ereignisse und Statusantworten sind `explicit`, `idle`, `sleep`, `screen_lock`, `screen_off`, `background`, `mount_end`, `worker_end`, `removed` (`lock_policy.rs`, Funktion `as_str`).

| Auslöser | Desktop Windows | Desktop Linux | Android | Beleg |
|---|---|---|---|---|
| ausdrücklich (`se vault lock`, Knopf, „Alle sperren“) | ja | ja | ja | `lock_policy.rs:29` |
| Leerlauf (Einstellung `auto_lock_idle_min`, aus oder an) | Vorgabe 30 Minuten, wenn eingeschaltet | gleich | gleich; die Berechnung ist für alle Plattformen dieselbe | `N/vault/core/lock_policy.rs:10-13`, `:209`; `N/vault/os/shared/session_timer.rs:60` |
| Ruhezustand | `PBT_APMSUSPEND` (`N/vault/os/windows.rs:1-6`) | `PrepareForSleep(true)` des logind-Managers mit Verzögerungssperre (`N/vault/os/linux_os.rs:1-8`) | Ausnahme | `lock_policy.rs:33-35` |
| Bildschirmsperre | verborgenes Fenster, `WTS_SESSION_LOCK`; Annahme A1 ungeprüft (`windows.rs:9-18`) | `LockedHint` der Sitzung; nur wenn die Desktopumgebung die Sperre meldet (`SCREEN_LOCK_NOTE`, `linux_os.rs:38-39`) | Bildschirm aus, vom Host gemeldet (`N/vault/os/android.rs:20-26`: Bildschirmsperre und Hintergrund) | `windows.rs:9-18`; `linux_os.rs:38-39`; `N/vault/os/android.rs:20-26` |
| Hintergrund | Ausnahme | Ausnahme | Wartezeit 0, 1, 2, 5 oder 10 Minuten (`BACKGROUND_CHOICES`); offene Datei hält sie an (`N/vault/os/shared/session_timer.rs:59-72`) | `lock_policy.rs:14-19`; `session_timer.rs:59` |

Der Dienst ist Sperrteilnehmer jedes Tresor-Laufwerks (Frist: Ruhezustand 4 Sekunden, ausdrücklich und Entfernen 60 Sekunden, sonst 15 Sekunden; `docs/oberflaechen/laufwerke.md`, Abschnitt 6).

## 5 Codes

Die strukturierten Codes eines Tresor-Auftrags (`N/vault/core/error_texts.rs:7-18`, Anzeigetexte `:28-45`):

| Code | Bedeutung |
|---|---|
| `vault_locked` | Der Tresor ist gesperrt; Entsperren mit `se vault unlock <Tresor>` |
| `vault_wrong_password` | Das Passwort ist falsch |
| `vault_unsupported_format` | Das Tresorformat wird nicht unterstützt |
| `vault_hub` | Der Tresor gehört zu einem Cryptomator Hub und lässt sich nicht mit Passwort entsperren |
| `vault_memory` | Der Arbeitsspeicher reicht zum Entsperren nicht |
| `vault_no_saved_password` | Für diesen Tresor ist kein Passwort gespeichert |
| `vault_unsaved_changes` | Es gibt noch nicht gespeicherte Änderungen |
| `vault_not_atomic` | Der Speicherort kann Dateien nicht atomar ersetzen; ein Passwortwechsel ist dort nicht sicher |
| `compare_stale` | Der Vergleich ist veraltet |
| `compare_too_large` | Die Datei ist zu groß für die Zusammenführung |

Der Entsperrvorgang kennt außerdem den Abbruch `canceled` (`N/vault/os/shared/unlock.rs:59`).

## 6 IPC-Weg

Die Oberflächen sprechen den Hintergrunddienst über `IpcRequest::Vault { token, req }` an (`N/daemon/os/shared/ipc_protocol_vault.rs`). Varianten: `Status`, `Unlock`, `UnlockSaved`, `Lock`, `Open`, `Seal`, `Unseal`, `SpoolKey`, `Compare`, `CompareText`, `Resolve`, `MergeApply`, `Check` (Zeilen 78 bis 153). `SpoolKey` antwortet nur dem Laufwerkshost der Mount-ID mit dessen Sitzungstoken (Abschnitt 4 von `laufwerke.md`). Die Android-Brücke ruft dieselben Kernfunktionen über `vault.*` und `sync.*` auf (Abschnitt 1).

## 7 Klartextgrenze und Hinweise

- **Klartextnamen in Texten.** Klartextnamen und -pfade aus einem Tresor erscheinen nicht in Meldungen der Methoden `vault.*`, im Fehlerprotokoll, in `logcat`, in Benachrichtigungen oder in Aufgabentiteln; Aufgaben tragen den Titel „Tresor <Bezeichnung>“ (`N/mobile/core/crumbs.rs`, `TaskRecord::set_private` in `N/mobile/core/tasks.rs`; Plan S2).
- **Android-Bildschirme.** Die Tresorbildschirme setzen `FLAG_SECURE` über `SecureWindow` und `SecureOn` bei Blättern und Dialogen (`A/ui/vaults/SecureWindow.kt:16`, `VaultSettingsScreen.kt:188`). Die Passwortfelder schließen Autofill aus (`NoAutofill`, `VaultUnlockSheet.kt:163`; `SecureWindow.kt:66`).
- **Gespeichertes Passwort (Android).** Der Kern speichert nie das Klartextpasswort, sondern einen Umschlag mit dem Präfix `android-kw1:` (`N/mobile/os/shared/domains/vault_keys.rs:26`). Er entsteht mit einem Keystore-Schlüssel, der bei jeder Nutzung eine Bestätigung verlangt (`A/ui/vaults/VaultSavedPassword.kt:158-161`, `setUserAuthenticationRequired` mit biometrischer oder Geräte-Bestätigung). Ohne Displaysperre ist „Passwort merken“ deaktiviert.
- **Abbrechen.** „Abbrechen“ während der Schlüsselableitung bricht den Auftrag ab; der Kern verwirft den Schlüssel und entsperrt nicht (`N/vault/os/shared/unlock.rs:59`; `A/ui/vaults/VaultUnlockSheet.kt:117`).
- **Bildvorschau (Android).** Eine Bildvorschau im Vergleich dekodiert mit `inSampleSize` aus dem eigenen Provider und nur im Vergleichsdetail (`A/ui/vaults/VaultCompareDetail.kt:66-70`). Sie ist nur möglich, wenn der Tresor entsperrt und unter „Ordner für Apps bereitstellen“ freigegeben ist; sonst „Keine Vorschau“ (Abweichung 9 der Stand-Datei U31).
- **Windows-Miniaturbilder und Windows Search** sind nur global steuerbar; Stand U14, am Code nicht erneut geprüft.
- **Linux-Miniaturbilder.** Beim Laufwerksende und beim Sperren entfernt der Dienst die freedesktop-Miniaturbilder im Laufwerksordner (`N/vault/os/linux_thumbnails.rs:44`).
- **Versiegelter Spool.** Spool-Dateien liegen im Cryptomator-Inhaltsformat unter dem Spool-Schlüssel; ein Kürzen an einer Blockgrenze ist von außen nicht erkennbar (Stand U14).
- **Vergleichsstand** liegt nur im Speicher des Dienstes (`N/vault/os/shared/compare_cache.rs:2`). Zusammenführen ist nur bis 512 KiB möglich (`N/vault/os/shared/compare_merge.rs:26`); die Zeilenlänge des Dienstes ist 2 MiB mit Escaping (`compare_merge.rs:23`).
- **Kryptografie.** Format 8 (`N/cryptomator/core/names.rs:1`, `cipher_combo.rs:1`). Der Passwortbyte-Puffer liegt in `Zeroizing` (`N/cryptomator/core/kdf.rs:74`). Die Passwortmindestlänge ist 8 Zeichen (`N/vault/os/shared/password.rs:29`).

## 8 Interop-Aussagen

Die Aussagen stammen aus der Nachrecherche des Stranges (`docs/plaene/2026-10-17-rechte-desktop-tresore/nachrecherche-tresore.md`, Abruf 2026-10-17); am 2026-10-21 nicht erneut geprüft.

- Cyberduck und Mountain Duck blenden `.c9s`-Einträge und Konfliktkopien mit Sonder- oder Leerzeichen aus (G23).
- Cryptomator Desktop 1.19.3 enthält die CryptoFS-Korrektur #355 nicht (Abschnitt 2 der Nachrecherche).

## 9 Wiederherstellungsschlüssel

Se zeigt und akzeptiert keinen Wiederherstellungsschlüssel. Der Hinweis lautet: „Wiederherstellungsschlüssel von Cryptomator werden in se nicht angezeigt und nicht angenommen, weil die Wortliste nicht frei lizenziert ist. Zum Anzeigen oder Zurücksetzen Cryptomator Desktop verwenden; der Tresor bleibt dafür voll kompatibel.“ (`N/vault/os/shared/manage.rs:27-29`). Die Entscheidung über den Weg (Laden von Cryptomators Quelle mit Pin, schriftliche Erlaubnis oder Weglassen) ist Nutzerfrage OE1 (Lesestand U11).

## 10 Ausnahmen mit Grund

| Oberfläche | Fehlt | Grund |
|---|---|---|
| CLI | „Mit externer App öffnen“ (T9) | Plan C7.1: Ausnahme; der Weg ist der Export über `cat` oder `get` |
| CLI | Merge als eigener Befehl (T20) | Plan C7.1 führt ihn als Ausnahme; die Option `--action merge --merged-file` ist im Quelltext vorhanden (`N/cli/vault/compare.rs:76-78`) |
| Android | Laufwerk (T22) | Android hat keine Laufwerke; Ordner stellt der Dokumentanbieter bereit (`docs/oberflaechen/laufwerke.md`, Abschnitt 7) |
| Desktop | Bildvergleich (T18) | Der Vergleich zeigt nur Metadaten; Bildvergleich ist offen (TODO `VAULT-COMPARE-IMAGE`, Lesestand U29) |
| alle | Wiederherstellungsschlüssel (T12, T13) | Wortliste nicht frei lizenziert; Nutzerentscheidung OE1 |
| Android | Sperren bei Ruhezustand | Android meldet keinen Ruhezustand (`N/vault/os/android.rs:20-26`, `sleep: false`) |
| Desktop | Sperren bei Bildschirm aus und Hintergrund | Desktop meldet Ruhezustand, Bildschirmsperre und Leerlauf |

## 11 Offene Punkte

- Annahme A1 (Windows): ob Windows die Sperrnachricht an das verborgene Fenster liefert, ist ohne Handtest offen (Kennung `VAULT-SCREENLOCK-VERIFY` laut Plan; in `docs/TODO.md` noch nicht eingetragen).
- Annahme A2 (GNOME Shell und KDE Plasma setzen `LockedHint`): ohne Handtest offen (`VAULT-SCREENLOCK-VERIFY`).
- Wiederherstellungsschlüssel: Nutzerentscheidung OE1 offen (`VAULT-RECOVERY-KEY`; in `docs/TODO.md` noch nicht eingetragen).
- Bildvergleich (`VAULT-COMPARE-IMAGE`): nicht umgesetzt; in `docs/TODO.md` noch nicht eingetragen. `VAULT-COMPLETE-DRIVE-TARGET` (Lesestand U26): Inhalt im Stand nicht beschrieben; in `docs/TODO.md` ebenfalls nicht eingetragen.
- Restlücken aus der Nachrecherche (Plan G) bleiben offen; Tresor-Jobs laufen auf Android nur über `vault.syncCreate`.
- Tresor-Sync (Lesestand U20): Replikat-Ort mit Grund, feste und wählbare Felder der Jobs, Seeding-Regel, R2-Zurückstellung; K20 und K21 bleiben ohne bekannte Site zurückgehalten; eine Datei gleicher Größe in derselben Sekunde ohne Serverhash ist eine bekannte Lücke (Test belegt). Tresor-Jobs laufen über `vault::run_vault_job`.
