# S2 — Laufwerke und Einhängen je Betriebssystem

Stand: 2026-10-17, Endfassung nach der Nachrecherche. Lesestand des Codes: `da2646d2`. Teil des Plans `plan.md` (Ergebnis E10; Grundlage für E4 „Gerät als Ordner“ und E8 „Tresor als Laufwerk“). Nichts in dieser Datei ist gebaut oder ausgeführt (AGENTS.md: keine lokalen Builds/Tests); „Abnahmesignal“ heißt: wird in der einen Remote-Task-Suite des Stapels geprüft (Stufen in DRV-M10).

Kennzeichnung: **Befund** = im genannten Code oder Dokument nachgelesen; **Annahme** = ungeprüfte Schlussfolgerung, ausdrücklich so benannt, mit Wirkung und, wo möglich, Abnahmesignal; **Lücke** = genau benannte fehlende Angabe. Abkürzungen: `N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`, `<AD>` = `support_dirs::app_data_dir()` (Linux: `$XDG_DATA_HOME/smart_explorer`, sonst `~/.local/share/smart_explorer`, `N/support_dirs.rs:171-200`). Quellenkürzel: „Kern-Lesung“ = `docs/lesungen/2026-10-17-laufwerke-kern.md`, „Dokany-Lesung“ = `docs/lesungen/2026-10-17-laufwerke-windows-dokany.md`, „Ref-Win“ = `docs/refs/windows-dateisystem-frontends-2026-10-17.md`, „Ref-FUSE“ = `docs/refs/linux-fuse-laufwerke-2026-10-17.md`, „Ref-Android“ = `docs/refs/android-documents-provider-2026-10-17.md`, „Ref-Crypt“ = `docs/refs/cryptomator-einhaengen-revalidierung-2026-10-17.md`, „Tresor-Lesung“ = `docs/lesungen/2026-10-17-tresore-bestand.md`, „NR“ = `nachrecherche-laufwerke.md` (Antworten N1–N27, Revalidierung C1-1 … C8-3), „S3-Plan“ = `strang-tresore.md`, „S5-Plan“ = `strang-desktop.md`.

## Schleifenstand

**Ziel:** Entscheiden und belegen, ob ein Einhängeweg, den Cryptomator nutzt, Dokany ersetzen und Zuverlässigkeit bringen kann; Linux-Laufwerke einführen; Ordner unter Android für fremde Apps bereitstellen; eine gemeinsame Frontend-Grenze schaffen, über die Geräte (S5) und Tresore (S3) als Ordner/Laufwerk erscheinen, ohne dass der Nutzer se öffnet oder Verbindungen von Hand herstellt. Quelle: E10, E4, E8 in `plan.md` A; Nutzer: „it still requires to open se and open the connection. and when i wanna integrate it i have to manually include them as a drive for windows while other systems dont support anything similar yet“.

**Kriterien (Status des Planungsteils):**

| Kriterium | Herkunft | Nachweis | Status |
|---|---|---|---|
| K1 Belegte Entscheidung, ob ein Cryptomator-Einhängeweg Dokany ersetzen kann und Zuverlässigkeit bringt | E10 „see if any of these could replace donkey and add reliability or not“ | Primärquellen + Entscheidung C1 | erfüllt: C1 auf NR N3, N18, C1-1 bis C1-13 gestützt; drei nicht belegbare Nebenangaben (C1-5, C1-14, C1-17) tragen die Entscheidung nicht (Annahmen A1–A3 in F) |
| K2 Linux hängt Laufwerke ein | E10 „how you wanna do it for linux with plugging stuff in as drive“ | Entwurf C2 + Abnahmesignale DRV-M4/M5 | erfüllt (Planungsteil): alle widerlegten Grundlagen korrigiert (C2-7, C2-16, C2-21), fällige Punkte entschieden (`setattr`, Kernel-Zeitlimit, Einhängeort) |
| K3 Android stellt Ordner fremden Apps bereit, soweit die Plattform es erlaubt | Nutzer „other systems dont support anything similar yet“ | Entwurf C3 + DRV-M8 | erfüllt (Planungsteil): N10, N11, N14, N15, N20 eingearbeitet |
| K4 Bestehende Windows-Laufwerke funktionieren weiter; gespeicherte Laufwerke bleiben gültig oder werden migriert | AGENTS.md „preserve existing behavior“ | C4 Persistenz + DRV-M2/M3 | erfüllt (Planungsteil): kein Frontendwechsel; Registry rückrollsicher; Bestandsfehler `ERROR_INVALID_DATA` → „Zugriff verweigert“ wird als Defekt behoben (DRV-M2) |
| K5 Eine Frontend-Grenze für Gerät-als-Ordner (S5) und Tresor-Laufwerk (S3) | E4, E8 | C4 + Verträge E1/E2 | **offen**: S3 muss zwei Präzisierungen bestätigen (E1 Punkte P1 und P3: Schlüssel der versiegelten Journalfelder im Laufwerkshost; CLI-Form `se drive mount vault://…` statt `se drive add --vault`) |
| K6 Kein dauerhafter Klartext eines Tresors in Spool, Cache oder Journal | Erkundung „Klartextgrenze“; S3-Plan K01–K03 | C6 + DRV-M7 | **offen**: Abweichung von S2-V7 (Windows-Laufwerksart für Windows Search, E1 Punkt P2) braucht die Zustimmung von S3; sonst Planungsteil vollständig |
| K7 Verbundene Geräte erscheinen ohne Handarbeit als Ordner | E4 | C5 + DRV-M6 | **offen**: Automatik startet beim Anmelden den Daemon und mit ihm den Share-Host (NR N22: Share-Host startet bei jedem Daemonstart); damit sind Freigaben und Exec-Host dieses Geräts ab Anmeldung erreichbar — Entscheidung des Hauptagenten nötig (betrifft S1) |
| K8 Alle Oberflächen der Laufwerke | Nutzerregel, `docs/oberflaechen/INDEX.md` | C7 + je Meilenstein; Teildokument `docs/oberflaechen/laufwerke.md` | erfüllt (Planungsteil) |
| K9 Bestehendes Verhalten bleibt erhalten (gespeicherte Laufwerke, Buchstaben, Spool-/Recovery-Verträge, Stage-Namen `.se-mount-*`, Remote-Locator und Verbindungsidentität, SMB-Ablehnung am Desktop) | AGENTS.md | C4, DRV-M2, DRV-M3, Windows-Regressionsstufe | erfüllt (Planungsteil); bewusste Änderungen nur: Fehlerabbildung (Defektbehebung) und `HostUnreachable`/`NetworkUnreachable` mit genauerer Meldung |
| K10 `core/` plattformneutral, Plattformcode unter `os/`, neue Rust-Dateien < 500 Zeilen, plattformspezifische Abhängigkeiten in `target`-Abschnitten; vor dem Erweitern fast voller Dateien ausgliedern | AGENTS.md „native Rust architecture“ | Dateilisten in D mit Ausgliederungen | erfüllt (Planungsteil) |
| K11 Keine willkürlichen Grenzen, keine UI-Platzhalter | Auftrag, Memory „decide independently“ | Grund je Grenze in C | erfüllt (Planungsteil): 60-s-Wert (R17 alt) gestrichen; 30-s-Wache aus der IPC-Frist abgeleitet |
| K12 Eine Plattformprüfung je übertragenem Mechanismus | Nutzerregel, Memory „question transferred solutions“ | Tabelle C4 + je Meilenstein | erfüllt (Planungsteil) |
| K13 Lizenz verträglich oder begründet | AGENTS.md, E7 | C8 | erfüllt (Planungsteil): `fuser` MIT; Lizenzhinweise aller Rust-Crates über den Integrationsmeilenstein des Hauptagenten |

**Aktiver Stand:** Windows behält Dokany 2.3.1.1000 als einziges Frontend (WinFsp, Cloud Files API, ProjFS, WebDAV-Miniredirector verworfen; WinFsp mit Wiedervorlage T1–T3). Linux: `fuser =0.18.0` (rein Rust, `fusermount3`), Laufwerke unter `~/SmartExplorer/<Name>`, Tresor-Laufwerke unter `$XDG_RUNTIME_DIR/smart-explorer/Tresore/<Name>` (tmpfs, S2-V7), kein `auto_unmount`, Aufräumen durch den Daemon, Zeit- und Modusänderungen über den bestehenden `finish_stage`-Vertrag. Android: `DocumentsProvider` mit `openProxyFileDescriptor` über dieselbe Engine. Gemeinsame Grenze: Trait `MountFilesystem` mit Pfaddialekt, gemeinsamer Hostablauf, Spool-Speicher-Trait mit Klartext- und versiegelter Umsetzung (versiegelt liefert S3). Gerätebaum automatisch auf allen drei Plattformen. Bestandsfehler der Windows-Fehlerabbildung wird behoben.

**Nächster Schritt:** Bestätigungen von S3 (P1–P3 in E1) und Entscheidung des Hauptagenten (K7) einholen; danach Umsetzung ab DRV-M1. Rückkehrpunkt bei Ablehnung: C6 (P2), C4/E1 (P1), C7 (P3), C5 (K7).

**Schleifenstatus:** offen — Grund: K5 und K6 warten auf drei Bestätigungen von S3 (E1 P1, P2, P3), K7 wartet auf die Entscheidung des Hauptagenten zur Share-Erreichbarkeit ab Anmeldung; alle übrigen Kriterien sind für den Planungsteil belegt erfüllt.

## B — Bestand dieses Strangs (nur Entscheidungsrelevantes)

### B1 Befunddateien

| Datei | Inhalt | Stand |
|---|---|---|
| `docs/lesungen/2026-10-17-laufwerke-kern.md` | `N/mount/core` (51 Dateien, plattformneutral), Engine-Schnittstelle, Spool/Journal/Stage-Ledger, CLI `se drive`, Desktop-Fenster, Android-Provider-Bestand | Lesestand `da2646d2` |
| `docs/lesungen/2026-10-17-laufwerke-windows-dokany.md` | Dokany-Host (47 Dateien), 25 Callbacks, Zeitüberwachung, private DLL, Installer, CfAPI-Altbefund | Lesestand `da2646d2` |
| `docs/refs/windows-dateisystem-frontends-2026-10-17.md` | Dokany, WinFsp, Cloud Files API, ProjFS, WebDAV-Miniredirector | Abruf 2026-10-17 |
| `docs/refs/linux-fuse-laufwerke-2026-10-17.md` | Rust-FUSE-Crates, `fusermount3`, Distributionen, Sandkästen, Dateimanager, Anmeldung | Abruf 2026-10-17 |
| `docs/refs/android-documents-provider-2026-10-17.md` | `DocumentsProvider`, `openProxyFileDescriptor`, Vergleichsprojekte | Abruf 2026-10-17 |
| `docs/refs/cryptomator-einhaengen-revalidierung-2026-10-17.md` | Cryptomator-Mounter, Wechselgründe, `whenTerminated` | Abruf 2026-10-17 |
| `docs/lesungen/2026-10-17-tresore-bestand.md` (Abschnitte 2.3, 3) | Klartext-Ablageorte K01–K43 | Lesestand `da2646d2` |
| `docs/CRYPTOMATOR_VAULTS_ERKUNDUNG.md` | Klartextgrenze der Materialisierung | Stand 2026-10-04 |
| `nachrecherche-laufwerke.md` | Antworten N1–N27, Revalidierung C1–C8, 16 Folgen | Abruf 2026-10-17 |
| `strang-tresore.md` Abschnitte C3.6, C4, C8, E, VLT-M12 | Verträge S2-V1 bis S2-V10 | Stand 15:53 |
| `strang-desktop.md` Abschnitt E1 | Anforderungen S5 → S2 (a)–(d) | Stand 16:02 |

### B2 Entscheidungsrelevante Befunde

1. **Engine plattformneutral, aber Windows-Namensdialekt.** `PathProjector::project` verlangt einen Windows-Wurzelpfad und trennt an `\` und `/` (`N/mount/core/path.rs:37-73`); `validate_windows_component` (`path.rs:90-127`); Ausblenden ungültiger Namen (`metadata_loading.rs:348-350`); dieselben Regeln in `engine_recovery.rs:198`, `peer_names.rs:10-98`.
2. **Host-Prozess-Architektur.** Daemon löst die Quelle auf (`N/daemon/os/shared/mount_source.rs:8-52`), umhüllt sie mit `RootedBackend` (`mount_manager.rs:147-156`), startet `current_exe --mount-host <id>` (`N/daemon/os/windows/mount_launch.rs:42`); Host holt Konfiguration und Backend-Proxy über die Loopback-IPC (`mount_client.rs:114`; `ipc_protocol.rs:326-345`). Linux-Start ist ein Stub (`N/daemon/os/linux_os/mount_process.rs:10-20`, für Linux und Android kompiliert, `N/daemon/mod.rs:114-116`); `start_locked` bricht außerhalb Windows ab (`mount_manager.rs:129-140`).
3. **Hostablauf bis auf Dokany generisch** (`N/mount/os/windows/host.rs:47-180`); Lease, Platzabfrage, Buchstabenwahl und Dokany-Teile sind Windows-gebunden; scheitert `retry_pending_changes`, scheitert der ganze Host (`host.rs:105-110`).
4. **Persistenz.** `<AD>/mounts/registry.json`, `{version: 1, mounts: [MountConfig]}`, höchstens 1 MiB und 256 Einträge (`mount_registry.rs:10-23`, `:136-138`); jede Version ungleich 1 und jeder Parse-Fehler macht die ganze Registry ungültig (`:119-134`), danach blockieren Start und Liste (`mount_manager.rs:116`, `:229`, `:488`). Kein `deny_unknown_fields` (`N/mount/core/types.rs:237-252`): unbekannte Felder ignoriert ein älterer Stand, unbekannte Enum-Varianten nicht.
5. **Kein automatisches Wiedereinhängen** nach Daemon-Neustart (`mount_manager.rs:56-100`, Text `:71`).
6. **Buchstabenwahl** `Z`→`D`, nur Laufwerksbuchstaben, Optionen `OPTION_CURRENT_SESSION | OPTION_MOUNT_MANAGER` (Dokany-Lesung 2.2, `host.rs:193-200`, `:294-309`).
7. **Dokany-Zeitlimit** 300 000 ms mit Verlängerung alle 30 s; Dokany hängt bei Ablauf aus (Dokany-Lesung 2.2, Ref-Win 2.1.4).
8. **SMB wird am Desktop nicht eingehängt** (`mount_source.rs:20-24`: „SMB is browsed, copied and synced, but never mounted as a drive, whatever its staged-write guarantees report.“); ein ursächlicher Grund steht weder im Code noch im Commit `7251ab33` noch in `docs/` (NR N19); das SMB-Backend meldet `supports_mounted_writes() == true` (`N/smb/core/backend.rs:410-420`).
9. **Bereichslesen nur bei Nur-Lese-Laufwerken** (`N/mount/core/range_read.rs:7-11`).
10. **Android.** `crate::mount` für Android kompiliert (`N/lib.rs:37`); JNI nur `init`, `call`, `pollEvents` (`native/android-bridge/src/lib.rs:115-163`); kein `DocumentsProvider` (Kern-Lesung F7); `minSdk = 30`, `targetSdk = 36` (`android/app/build.gradle.kts:89-90`); `Application.onCreate` initialisiert `AppPrefs` und `WakeKeeper` (`A/SmartExplorerApp.kt:18-22`).
11. **Präsenz.** `ShareEvent::{DirectAvailable, DirectOffline, RoomRoster, RoomJoined, RoomLeft}` (`N/share/core/types.rs:365-400`); signierte Präsenz 300 s, LAN-Präsenz 150 s; Server trennt nach 60 s ohne Daten bzw. 60 s ohne Keepalive-Antwort (NR N23).
12. **Anmeldung und Sync-Trennung.** Sync-Autostart `~/.config/autostart/smart-explorer-sync-daemon.desktop` bzw. Run-Wert (`N/autostart/os/linux_os.rs:4`, `N/autostart/os/windows.rs:2`); der Daemon plant Sync-Setups nur bei `crate::autostart::is_enabled()` (`N/daemon/os/shared/run_loop.rs:118`); `disabled_by_system()` meldet getrennt, dass der Desktop den Eintrag abgeschaltet hat (`linux_os.rs:70`, `windows.rs:153`); jeder Daemonstart startet den Share-Host (NR N22, `run_loop.rs:86-110`).
13. **Fehlerabbildung Windows mit Bestandsfehler.** `io_status` (`N/mount/os/windows/callback_status.rs:215-248`) übersetzt `ErrorKind` in Win32-Codes und ruft `DokanNtStatusFromWin32`; der Rückfall `_ => ERROR_INVALID_DATA` steht nicht in Dokanys Tabelle `ntstatus.i` und wird zu `STATUS_ACCESS_DENIED` — jede nicht einzeln abgebildete Fehlerart erscheint auf bestehenden Laufwerken als „Zugriff verweigert“ (NR N27 Punkt 3; Dokany `v2.3.1.1000` `dokan/ntstatus.c:25-31`).
14. **Stufen-Abschluss mit Zeit und Modus.** `BackendExtensions::finish_stage(stage, StageFinish { mtime_ms, mode, durability })` gibt einer Stufe vor der Veröffentlichung Zeit und Unix-Modus; Vertrag: „Failing to store the time is no error (`mtime_applied: false`)“, Modus „where the target keeps Unix modes; ignored elsewhere“ (`N/vfs/core/extensions.rs:103-110`, `extension_types.rs:193-212`). Die IPC-Backend-Schicht reicht es weiter (`N/daemon/os/shared/ipc_backend_extensions.rs:47-49`); `RootedBackend` (`rooted_backend.rs`, 422 Zeilen) und `mount_proxy` (217 Zeilen) liefern keine `extensions()`; die Engine ruft `finish_stage` nicht.
15. **Dateigrößen im Bestand** (Grenze 500 Zeilen): `N/mount/core/types.rs` 499, `journal.rs` 499, `N/daemon/os/shared/mount_manager.rs` 498, `run_loop.rs` 488, `ipc_protocol.rs` 475, `N/app/core/mount_ui_draft.rs` 467, `N/cli/drive.rs` 461 — vor jeder Erweiterung ist eine Ausgliederung nötig.
16. **Abhängigkeiten und Toolchain.** `fuser` nicht vorhanden; `nix 0.31.3` im Lock; `quick-xml 0.39`, `libc` (`cfg(not(windows))`), `zbus 5.16` (`cfg(target_os = "linux")`) direkt (`native/Cargo.toml:142`, `:249-260`); CI `dtolnay/rust-toolchain@stable`; Runner `ubuntu-24.04` mit unprivilegiertem FUSE belegt (NR N24, Lauf `37175826251`).
17. **Keine Lizenzübersicht** der Rust-Crates im Repository (NR N25).
18. **Offene Zuverlässigkeitspunkte des Bestands liegen hinter dem Frontend** (`docs/TODO.md:95` D1; `docs/REMOTE_FS_TRADEOFFS.md:278-283`).

## C — Entscheidungen

### C1 — Windows-Frontend (E10)

**Lösungswege:** L1 Dokany behalten; L2 WinFsp ersetzen; L3 Dokany und WinFsp wählbar; L4 Cloud Files API ergänzend; L5 WebDAV-Miniredirector als Rückfall; L6 ProjFS.

**Bewertung (Fakten aus Ref-Win, Ref-Crypt, NR):**

| Merkmal | L1 Dokany | L2/L3 WinFsp | L4 Cloud Files API | L5 WebDAV-Miniredirector | L6 ProjFS |
|---|---|---|---|---|---|
| Belegter Zuverlässigkeitsgewinn für se | Ausgangspunkt | keiner: keine Messung Dokany 2.x gegen WinFsp 2.x (Ref-Win G7); Cryptomators Wechsel betraf Dokany 1.x und war organisatorisch (Ref-Crypt F1); offene se-Probleme liegen hinter dem Frontend (B2 Nr. 18) | nicht vergleichbar | nein | nein |
| Sicherheitslage der stabilen Version | v2.3.1.1000: #1335 (BSOD-Rennen) und #1339 offen; Korrektur `f8a542a1` unveröffentlicht (NR C1-3, C1-4) | v2.1: CVE-2026-3006 (CVSS 7.0) und CVE-2026-7162 (CVSS 7.8), beide lokale Rechteausweitung; CVE-2026-93689 (Dienstverweigerung, CVSS 5.5) bis 2.2.26215; Korrekturen nur in Pre-Releases v2.2B1–B5; kein Termin für stabiles 2.2 (NR N18) | Windows | Windows | Windows |
| Pflege | letzter Release 2025-09-28; 7 unveröffentlichte Commits (NR C1-1, C1-2) | 2026 fünf Betas | Microsoft | Microsoft | Microsoft |
| Lizenz für se (MIT) | LGPL-Laufzeit, dynamisch geladen | GPLv3 mit FLOSS-Ausnahme (Hinweis in Oberfläche und Doku, keine proprietären Teile); Auslegung „link“ für Laufzeitladen offen; Crate `winfsp` GPL-3.0 schiede aus, `winfsp_wrs` MIT (NR C1-9, C1-10) | `cloud-filter` MIT | – | `windows-projfs` GPL-2.0 |
| Installation, Upgrade | MSI, UAC; Upgrade nur nach Deinstallation und Neustart | MSI perMachine, UAC; Upgrade ohne Neustart; Dienst `WinFsp.Launcher` Start „auto“ (NR C1-12) | – | Dienst WebClient | optionale Komponente |
| Laufwerk/Ordner/UNC | Buchstabe mit `OPTION_MOUNT_MANAGER`; Ordner ohne Mount Manager „Access denied“ (#1337) | `DefineDosDevice`-Laufwerk: `GetFinalPathNameByHandleW` scheitert mit 1005 (NR N3); Mount-Manager-Laufwerk nur mit Administratorrechten; Netzwerkmodus mit Office/Dropbox-Fehlern bei Cryptomator | kein Buchstabe | 50 MB Standardgrenze, rund 1 000 Dateien je Ordner | kein Buchstabe |
| Zeitlimit | Ablauf hängt das Volume aus | **Annahme A2** (NR C1-17 nicht belegbar): Ablauf beendet nur die Anfrage | fest 60 s | 30/60 s | – |

**Antwort auf die Nutzerfrage „see if any of these could replace donkey and add reliability or not“:** **Nein, derzeit nicht.** Keiner der Wege, mit denen Cryptomator Tresore einhängt, kann Dokany in se heute mit belegtem Zuverlässigkeitsgewinn ersetzen: (1) Es gibt keine Messung WinFsp 2.x gegen Dokany 2.x; Cryptomators Wechsel betraf Dokany 1.x und war mit Wartungsaufwand begründet. (2) Die einzige stabile WinFsp-Version hat zwei offene Rechteausweitungs-Schwachstellen; die Korrekturen liegen nur in Betas. (3) WinFsp verschiebt Fehlerklassen: Auf dem Laufwerk ohne Administratorrechte scheitert `GetFinalPathNameByHandleW` (belegt, NR N3) — damit scheitern in Node/Electron `fs.promises.realpath` und `fs.realpath(.Sync).native` (belegt); welche dieser Aufrufe Obsidian und VS Code nutzen, ist **Annahme A3**; im Netzwerkmodus meldet Cryptomator Office-/Dropbox-Fehler. (4) Die offenen se-Probleme liegen in Engine und Backends. WebDAV-Miniredirector, Cloud Files API und ProjFS bringen keine Zuverlässigkeit.

**Wahl:** L1 — Dokany 2.3.1.1000 bleibt einziges Windows-Frontend; nur Laufwerksbuchstaben, keine Ordner- und keine UNC-Einhängung. Ob `OPTION_MOUNT_MANAGER` aus dem nicht erhöhten Host ohne Administratorrechte wirkt, bleibt **Annahme A1** (NR C1-14); sie trägt die Entscheidung nicht, weil der Bestand unverändert bleibt. Neu in diesem Stapel und zugleich Messung für T2: Abnahmesignal `drive_task_windows_realpath_native_on_dokany` (Node auf dem Runner ruft `fs.realpathSync.native` auf einer Datei des Dokany-Laufwerks; Ergebnis wird berichtet, ein Fehlschlag wird als Befund zu A3/T2 festgehalten, nicht behoben).

**Wiedervorlage WinFsp (`WINFSP-AB` in `docs/TODO.md`, DRV-M9), wenn alle drei gelten:** T1 stabiles WinFsp-Release mit den Korrekturen von CVE-2026-3006, CVE-2026-7162 und CVE-2026-93689 (Stand 2026-10-17: nicht erfüllt, NR N18); T2 A/B-Lauf derselben Engine mit beiden Frontends (Office-Speichern über Umbenennen, Obsidian-Start, `fs.realpathSync.native`, 25 000 Unterordner im Dialog „Speichern unter“, hängender Backend-Aufruf über 300 s, Auswerfen bei hängendem Fenster) zeigt weniger Fehler mit WinFsp; Voraussetzung: beide Treiber auf `windows-2025` nacheinander installierbar (Lücke, NR N24); T3 Lizenzfrage „link“ und Bedingung 3 geklärt (nur durch den Rechteinhaber). `DOKANY-WATCH`: Release mit Korrektur von #1335; Ablauf des Treiberzertifikats (**Annahme A4**: um 2026-12, NR C1-5 nicht belegbar).

**Bestandsschutz:** Das Geräte-Laufwerk (C5) läuft unter derselben Lebenszyklussperre `MountManager::lifecycle` (`mount_manager.rs:31`, bestätigt NR C1-18), die Ein- und Aushängen serialisiert; der Auslöser von #1335 sind 16 gleichzeitige `DokanRemoveMountPoint`.

**Verworfen:** L2, L3 (Gründe 1–3); L4 (kein Buchstabe, Platzhalter sind Klartext auf der Platte, feste 60-s-Frist, Anbieter muss laufen); L5 (Größen- und Listengrenzen); L6 (nur für schnelle Speicher).

### C2 — Linux-Laufwerke (E10)

**Bibliothek:** L1 `fuser 0.18.0` gewählt; L2 `fuse3 0.9.0`, L3 `polyfuse 0.5.1`, L4 `easy_fuser 0.7.1`, L5 libfuse3 per FFI, L6 GVfs/KIO verworfen (Gründe: L2 MSRV 1.91, `Notify` nur in `poll`, TODO in der Pfadbrücke; L3 Aushängen fest über `/usr/bin/fusermount`, das Fedora/Arch-`fuse3` nicht liefert (NR C2-3); L4 ohne Invalidierung und `auto_unmount`-Steuerung; L5 C-Bibliothek mit SONAME-Wechsel `.so.3`→`.so.4`; L6 erreicht Terminal-Programme nicht). `fuser = { version = "=0.18.0", default-features = false }` in `[target.'cfg(target_os = "linux")'.dependencies]` (Android ist nicht `linux`; `fuser/build.rs` hat für Android keinen Zweig ohne pkg-config, NR C2-6). Pflegerisiko: seit 0.18.0 58 Commits ohne Release, „Pull requests are no longer being accepted“ (NR N26 Punkt 7) → exakte Pin, `fuser`-Typen nur in `N/mount/os/linux_os/fuse_*.rs`, Notfallweg Fork von 0.18.0.

**Threading (korrigiert, NR C2-7):** `let mut config = Config::default(); config.n_threads = Some(2); config.clone_fd = true; config.acl = SessionACL::Owner; config.mount_options = …;` (`Config` ist `#[non_exhaustive]`, `n_threads: Option<usize>`). Zwei Leseschleifen; jede Anfrage geht mit ihrem `Reply` an einen elastischen Arbeiter-Pool (Antwort aus jedem Thread auf den empfangenden Kanal, NR N26 Punkt 3; überzählige Arbeiter beenden sich, sobald die Warteschlange leer ist). Keine feste Obergrenze: Der Kernel begrenzt asynchrone Anfragen über `max_background` (fuser-Standard 16, NR C2-8), synchrone über blockierte Aufrufer.

**Abbruch:** `fuser` antwortet `ENOSYS` auf die erste INTERRUPT-Anfrage; danach setzt der Kernel `no_interrupt` (belegt, NR N1). Ein Prozess in einer schon zugestellten Anfrage ist auch mit `SIGKILL` bis zur Antwort nicht beendbar (belegt). Mutationen dürfen ohnehin nicht abgebrochen werden (Dokany-Lesung F3 Nr. 8–9). Ausweg: `se drive unmount --force` schreibt in `/sys/fs/fuse/connections/<n>/abort` (für den einhängenden Nutzer schreibbar, `fusectl` auf Ubuntu 24.04, Debian 13, Arch eingehängt; NR N2) und hängt danach mit `fusermount3 -u -z` aus. `<n>` wird aus Feld 3 („major:minor“) von `/proc/self/mountinfo` gelesen, nicht per `stat` am hängenden Einhängepunkt; `<n>` = minor (**Annahme A5**, NR C2-10; Abnahmesignal `drive_task_fuse_force_abort_frees_blocked_reader`).

**Zeitgrenzen (fällig, NR C2-11):** Die Engine setzt keine eigene Anfragefrist (Fristen liegen in den Backends, wie unter Windows). Kernel ab dem Commit `9b17cb59` (Runner-Kernel 6.17) hat die Sysctls `/proc/sys/fs/fuse/default_request_timeout` und `max_request_timeout` (Standard 0); `fuser 0.18.0` handelt keinen eigenen Wert aus. Entscheidung: `se drive runtime` liest beide Werte und meldet einen Wert > 0 als Grenze („Kernel bricht das Laufwerk ab, wenn eine Anfrage länger als N s dauert; langes Hochladen beim Schließen kann das auslösen“); bricht der Kernel die Verbindung ab, erkennt der Host das Ende der Sitzung (`/dev/fuse` liefert `ENODEV`), meldet `Failed` mit Grund „Kernel-Zeitlimit fs.fuse.default_request_timeout = N s überschritten“, das Journal bleibt, „Erneut versuchen“ veröffentlicht. Kein Umgehungsversuch (Administratorwert hat Vorrang). TTL 1 s für Eintrag und Attribute, negative TTL 0 s (NR C2-12; Nullwert der negativen TTL ist **Annahme A6** ohne Wirkung, weil se den Wert selbst setzt); kein `writeback_cache` („generally not suitable for network filesystems“, NR C2-13); `set_max_write(1 MiB)` (Kernel-Standard 256 Seiten bestätigt, NR N4).

**`setattr`, `chmod`, `chown`, `utimensat` (fällig, NR N6):** `cp -a` und `rsync -a` scheitern an `ENOTSUP` für Zeiten und Modus; GIO, vim, Kate und KIO speichern trotz `ENOTSUP`. Lösungswege: S1 überall `ENOTSUP` (Windows-Gleichstand) — bricht `cp -a`/`rsync -a`; S2 alles annehmen ohne Wirkung — `cp -a` meldet Erfolg, Zeiten gehen aber auch dort verloren, wo das Backend sie speichern kann; S3 über den bestehenden Vertrag `finish_stage` übernehmen, wo möglich, sonst ohne Fehler (B2 Nr. 14). **Wahl S3** mit diesen Regeln:
- Größe → `truncate` (Bestand).
- Änderungszeit und Modus einer Datei mit ausstehendem Inhalt (offenes Schreib-Handle oder schmutziger Eintrag) → am Eintrag gespeichert und bei der Veröffentlichung als `StageFinish { mtime_ms, mode }` an `finish_stage` übergeben; `getattr` zeigt bis dahin die angeforderten Werte. Speichert das Ziel sie nicht (`mtime_applied: false` bzw. Modus ignoriert), ist das nach dem Bestandsvertrag kein Fehler; der Host protokolliert es.
- Zeit- oder Modusänderung an Ordnern und an Dateien ohne ausstehenden Inhalt → Erfolg ohne Wirkung (kein Vertrag zum Setzen von Ordnerzeiten oder zum Nachsetzen ohne neue Veröffentlichung; ein Fehler bräche `cp -a`/`rsync -a` an jedem Ordner). Folge, dokumentiert: `rsync -a` setzt Zeiten erst nach dem Schließen und überträgt deshalb bei jedem Lauf erneut; für wiederholtes Abgleichen gelten die Sync-Setups von se.
- Zugriffszeit → ohne Wirkung (Mount `noatime`; kein Backend und kein `StageFinish`-Feld speichert sie).
- `chown`/`fchown` auf den einhängenden Nutzer und seine Gruppe → Erfolg; jede Kennung außer diesen beiden → `EPERM` (POSIX für Nicht-root; `cp -a` toleriert `EPERM`, NR N6).
- Nur-Lese-Laufwerk → `EROFS` für jede Änderung.
- Windows bleibt unverändert (`set_file_time`, `set_file_attributes` → `ERROR_NOT_SUPPORTED`): Bestandsschutz; Gleichstand wäre eine Verhaltensänderung bestehender Laufwerke.
Voraussetzung: `finish_stage` erreicht den Host über `RootedBackend` und `mount_proxy` (heute ohne `extensions()`, B2 Nr. 14) — beide erhalten eine Erweiterungsumsetzung, die **nur** `finish_stage` weiterreicht und alle übrigen Erweiterungsmethoden auf den Standardwerten lässt.

**Einhängeort (korrigiert NR C2-16, entschieden für Tresore):** Laufwerke ohne Tresor: Standard `~/SmartExplorer/<Name>` (GLib, GVfs, Nautilus, Nemo, Thunar zeigen Mounts unter `$HOME/` ohne „/.“ automatisch, Anzeigename = letzter Pfadteil; NR N8). Tresor-Laufwerke: `$XDG_RUNTIME_DIR/smart-explorer/Tresore/<Name>` (S2-V7: tmpfs, Schreiben nach unbemerktem Ende landet im Arbeitsspeicher); vor dem Einhängen prüft der Host per `statfs`, dass der Ort auf tmpfs liegt (`f_type` = `TMPFS_MAGIC`), sonst Abbruch „Tresor-Laufwerke brauchen einen Arbeitsspeicher-Einhängeort“; fehlt `XDG_RUNTIME_DIR` (keine Anmeldesitzung) → Abbruch mit diesem Grund, kein Rückfall auf die Platte. Regeln am Einhängepunkt: `fusermount3` verlangt kein Symlink, Schreibrecht und — nur wenn der Einhängepunkt selbst das Sticky-Bit trägt — Besitz durch den Nutzer (NR N5, C2-16). se verlangt zusätzlich einen leeren Ordner im Besitz des Nutzers (Gründe: se ändert den Modus des Ordners, was Besitz voraussetzt; ein nicht leerer Ordner verdeckte vorhandene Dateien). Außerhalb eines Mounts steht der Ordner auf `0500` (verriegelt; schreiben scheitert), unmittelbar vor dem Einhängen auf `0700` (bestätigt NR N5). Unter `$XDG_RUNTIME_DIR` trägt der Tresor-Einhängepunkt zusätzlich das Sticky-Bit (Basedir-Spezifikation: schützt vor periodischer Bereinigung; zulässig, weil der Nutzer Besitzer ist). WSL2: Standard bleibt `~/SmartExplorer`; im Windows-Explorer sichtbar sind laut WSL-Mitarbeiter Mounts unter `/mnt/wsl` (NR N16) — `se drive runtime` nennt das unter WSL als Hinweis für `--mount-point`; ob der Nutzer dort ohne `sudo` Ordner anlegen darf, ist **Annahme A7** (Wirkung: Hinweis ohne Nutzen; kein Abnahmesignal möglich, kein WSL im Runner).

**Aufräumen nach Absturz:** kein `auto_unmount` (verlangt in `fuser` `acl != Owner` und damit `user_allow_other`, das auskommentiert ist; mount.fuse3(8) „dangerous“; NR C2-17). Der Daemon überwacht den Host und führt nach dessen Ende `fusermount3 -u -z -- <Pfad>` aus, wenn `/proc/self/mountinfo` dort noch `fuse.smartexplorer` zeigt; vor jedem Einhängen wird ein toter Mount (`ENOTCONN`) zuerst ausgehängt; der Host erhält `PR_SET_PDEATHSIG(SIGTERM)`.

**Sichtbarkeit (korrigiert NR N8):** Typ `fuse.smartexplorer`. GTK-Lesezeichen (`$XDG_CONFIG_HOME/gtk-3.0/bookmarks`) nur für Mounts, die die GLib-Regel nicht automatisch zeigt (Tresor-Laufwerke unter `$XDG_RUNTIME_DIR`, eigene Orte außerhalb `$HOME/` oder mit „/.“), damit Nautilus nicht zwei Einträge zeigt; Ersetzen über Hilfsdatei im selben Ordner (GTK, Nautilus erkennen das sofort, NR N7). KDE: Eintrag in `$XDG_DATA_HOME/user-places.xbel` für jeden se-Mount (Metadaten-Besitzer `urn:smart-explorer:places`, `<id>` = Mount-ID, Bearbeitung mit `quick-xml` unter Erhalt fremder Einträge; KDirWatch erkennt das Ersetzen binnen 500 ms, NR N7); dass Dolphin `fuse.smartexplorer` nicht selbst zeigt, ist **Annahme A8** (NR C2-19; Wirkung bei Irrtum: doppelter Eintrag in Dolphin; kein CI-Abnahmesignal möglich, kein Dolphin im Runner). „Im Dateimanager öffnen“: `org.freedesktop.FileManager1.ShowFolders` über `zbus`, Rückfall `xdg-open`, als `pub(crate)` für S5 (E2).

**Wiederherstellung beim Anmelden (korrigiert NR C2-21):** Eigener XDG-Autostart-Eintrag `~/.config/autostart/smart-explorer-drives.desktop` bzw. Run-Wert `SmartExplorerDrives` mit `<smart_explorer> --drive-restore` (erreicht GNOME, KDE, Xfce, Cinnamon, MATE, NR N21); er besteht genau dann, wenn ein Laufwerk `auto_remount` hat oder der Gerätebaum eingeschaltet ist. Eine eigene Daemon-Rolle entfällt: Sync-Setups plant der Daemon nur bei `autostart::is_enabled()` (B2 Nr. 12). Randfall: Existiert der Sync-Eintrag, ist aber vom Desktop abgeschaltet (`disabled_by_system()`), plant ein durch `--drive-restore` gestarteter Daemon keine Sync-Läufe, bis die GUI oder `--sync-daemon` startet (Startgrund-Merker im Daemon; Ausgliederung aus `run_loop.rs`, B2 Nr. 15). Folge jeder Wiederherstellung: Der Daemon startet den Share-Host (K7, Entscheidung Hauptagent).

**Grenzen, ehrlich angezeigt:** fehlendes oder nicht setuid-`fusermount3` mit Paketbefehl (`apt-get install fuse3`, `dnf install fuse3`, `pacman -S fuse3`, `zypper install fuse3`; auf Desktop-Standardinstallationen von Ubuntu 24.04, Kubuntu 24.04, Debian 13 GNOME/KDE, Fedora Workstation 42 vorhanden, NR N9); `/dev/fuse` fehlt; Flatpak (`/.flatpak-info`) und Snap (`SNAP`) nicht unterstützt (**Annahme A9**, NR C2-24 nicht erneut geprüft; se wird so nicht verteilt; Abnahmesignal `drive_task_runtime_detects_sandbox_markers` prüft nur die Erkennung über eine austauschbare Wurzel); Container ohne `/dev/fuse`/`CAP_SYS_ADMIN`; WSL2-Hinweis; Kernel-Zeitlimit.

### C3 — Android: Dokumentanbieter (K3)

**Wahl L1** (`DocumentsProvider` + `openProxyFileDescriptor` über die Mount-Engine); verworfen: Vollkopie (Nextcloud), Pipes, nur Teilen (Cryptomator), Backend ohne Engine. Festlegungen:
- Manifest: Authority `${applicationId}.documents`, `exported="true"`, `grantUriPermissions="true"`, `permission="android.permission.MANAGE_DOCUMENTS"`, Filter `android.content.action.DOCUMENTS_PROVIDER` (bestätigt NR C3-1). Abschalten über `COMPONENT_ENABLED_STATE_DISABLED`. Die eigene App greift unter derselben UID auf den eigenen Anbieter zu (S2-V5: Bildvergleich, externe Apps).
- Roots: „Smart Explorer – Geräte“ (`devices`); je gespeicherter Verbindung (`conn:<Konto hex>`); Google Drive (`gdrive`); je **entsperrtem** Tresor (`vault:<site_id>`, Titel „<Bezeichnung> (Tresor)“), bei Sperren entfernt und `notifyChange(buildRootsUri)` (bestätigt NR C3-2). SMB-Roots (korrigiert NR C3-4/N19): angeboten, weil das Backend alle Staged-Write-Garantien meldet und Android keinen eigenen SMB-Zugang hat; der Desktop-Ausschluss bleibt unverändert, sein Grund ist im Bestand nicht belegt.
- Root-Flags: `FLAG_SUPPORTS_IS_CHILD` immer, `FLAG_SUPPORTS_CREATE` bei schreibbarem Ordner, `FLAG_LOCAL_ONLY` nur für Tresore auf lokalem Speicher (**Annahme A10**, NR C3-3: Bedeutung nur aus dem Javadoc der Ref-Android; Wirkung bei Irrtum: Apps mit `EXTRA_LOCAL_ONLY` sehen lokale Tresore nicht oder doch; Abnahmesignal prüft nur die se-Seite `drive_task_android_local_only_flag_only_for_local_vaults`).
- Dokument-IDs: `devices/<Geräteschlüssel>/<Pfad>` (Schlüssel aus `mount::device_key`, E2), `conn:<Konto hex>/<Pfad>`, `gdrive/<Pfad>`, `vault:<site_id>/<Pfad>`; `isChildDocument` ohne I/O.
- `queryChildDocuments`: Cache der Engine, sonst `EXTRA_LOADING` + `notifyChange`; Gerät offline → `EXTRA_ERROR` „Gerät nicht erreichbar“ (DocumentsUI zeigt beide, NR N15).
- `openDocument`: „r“, „w“, „wt“, „rw“, „rwt“; „t“ kürzt beim Öffnen; „wa“/„rwa“ dauerhaft abgelehnt, weil `StorageManagerService` `mode &= MODE_READ_WRITE` setzt und AppFuse kein `SETATTR` kennt (NR N20). Je Datei ein eigener `HandlerThread`; `onGetSize` antwortet aus dem Handle ohne Netz (vold-Sperre + system_server-Watchdog 60 s, ab Android 15 + 30 s Pause; NR N11). `onFsync` blockierend mit errno; `onRelease` nie blockierend.
- Veröffentlichung nach `onRelease` (korrigiert NR N14): Hintergrund-Executor sofort; zusätzlich WorkManager-Job `DocsCommitWorker` (expedited, `OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST`) mit `getForegroundInfo()` (Pflicht wegen `minSdk 30`, Android 11), der nach einem Prozessende `docs.retryPending` ausführt; ein Vordergrunddienst aus dem Anbieterprozess nur bei abgeschalteter Akkuoptimierung (Ausnahme 13 der Plattform). Fehler danach → se-Benachrichtigung.
- Bereichslesen: `RangeReadPolicy::ReadOnlyHandles` für Android; Tresor-Roots lesen „r“ gestreamt über `open_read_at` der Tresorschicht (keine Kopie), Schreiben über den versiegelten Spool (S2-V5, C6).
- Kaltstart: `ContentProvider.onCreate` läuft vor `Application.onCreate` (belegt NR N10); der Anbieter ruft die idempotente Initialisierung (`NativeBridge.init`, `AppPrefs`) selbst; `queryRoots` ohne Netz (DocumentsUI wartet 5 s, NR C3-9). Im eingeschränkten Sicherungsmodus installiert Android Anbieter nicht (Befund NR N10, keine Wirkung).
- Grenzen (Einstellungsabschnitt „Ordner für Apps bereitstellen“ und Doku): Apps nur mit Dateipfaden sehen nichts; MediaStore indiziert Anbieterinhalte nicht (**Annahme A11**, NR N12; Abnahmesignal `drive_task_android_mediastore_has_no_provider_files`); kein Kürzen offener Dateien; zufälliges Schreiben lädt die ganze Datei neu; Hochladen nach dem Schließen; Offline-Fehler.

### C4 — Gemeinsame Frontend-Grenze (K5)

**Wahl L1** (Trait `MountFilesystem` in `N/mount/core/`, Adapter unter `N/mount/os/{windows,linux_os,android}`, gemeinsamer Hostablauf `N/mount/os/shared/`); verworfen: Gerätebaum als Verbund-Backend unter einer Engine (ein Journal für alle Geräte → ein offline Gerät blockiert den Hoststart, B2 Nr. 3), je Gerät ein Laufwerk (Buchstabenmangel).

- `MountFilesystem`: `open_file`, `open_metadata_file`, `read`, `write`, `append`, `len`, `truncate`, `flush`, `close`, `stat`, `stat_cached`, `stat_handle`, `list_dir`, `list_dir_cached`, `mkdir`, `rename`, `rename_with_shared_destination`, `flush_path`, `begin_delete`, `cancel_delete`, `commit_delete`, `delete`, `pending_deletes`, `retry_pending_changes`, `maintain_cache`, `evict_clean`, `preload_metadata`, `refresh_metadata`, `drain_metadata_changes`, `dirty_entries`, `set_pending_attributes` (neu, C2), `prepare_lock` (neu, S2-V4/S2-V10), `volume_facts` (neu).
- `PathDialect { Windows, Posix }` (`#[serde(default)]` = `Windows`): Windows unverändert; Posix trennt nur an `/`, Komponente nicht leer, nicht `.`/`..`, ohne NUL, `/`, `\`, höchstens 255 Bytes; Namen mit `\` werden ausgeblendet (einige Backends deuten `\` als Trenner).
- `SpoolStorage` (S2-V2): Trait in `N/mount/core/spool_storage.rs` mit `SpoolFile { read_at, write_at, set_len, len, sync_data }`; Klartext-Umsetzung = Bestand byte-gleich; versiegelte Umsetzung `SealedSpoolFile` liefert S3 (Tresor-Inhaltsformat, S3-Plan C3.6). `PathSealer` (S2-V3): Trait in `N/mount/core/path_sealer.rs` (`seal`, `unseal`), Klartext = Identität; versiegelt liefert S3.
- Lease und Platz Linux/Android: `flock(LOCK_EX | LOCK_NB)` (belegt NR N13, Bestand nutzt es für den Daemon), `statvfs`.

**Plattformprüfung je übertragenem Mechanismus:**

| Mechanismus (Windows-Bestand) | Linux-FUSE | Android-Anbieter |
|---|---|---|
| Ganzdatei-Spool, Veröffentlichung beim Aufräumen | `flush` bei jedem `close(2)`, Fehler an `close`; jeder folgende `flush` ohne Änderung → `NoChanges` | `onFsync` meldet Fehler; nach `onRelease` kein Fehlerkanal → Hintergrund + Benachrichtigung |
| Zeitüberwachung `DokanResetTimeout` | nicht übertragbar; Kernel-Zeitlimit nur per Administrator-Sysctl (C2) | nicht übertragbar; `HandlerThread` je Datei, `onGetSize` ohne Netz |
| Freigabemodus-Nachbildung | nicht portiert (POSIX ohne Freigabemodi) | nicht portiert |
| Windows-Namensregeln | Dialekt `Posix` | Dialekt `Posix` |
| Buchstabenwahl `Z`→`D` | Einhängeorte C2 | Roots |
| `DokanNotify*` | `inval_entry`/`inval_inode` | `notifyChange` |
| `set_file_time`/`set_file_attributes` → `ERROR_NOT_SUPPORTED` | `finish_stage`-Regel (C2) | nicht anwendbar (Proxy-FD ohne `SETATTR`) |
| Freier Platz = lokaler Spool | `statfs` | `COLUMN_AVAILABLE_BYTES` |
| Host-Prozess `--mount-host` | eigener Prozess, Daemon räumt auf | Anbieter im App-Prozess |
| Cache-Lease | `flock` + `O_NOFOLLOW` | `flock` + `O_NOFOLLOW` |
| Bereichslesen nur Nur-Lese-Laufwerk | übernommen | `ReadOnlyHandles` |
| Links/Sonderdateien ausgeblendet | übernommen | übernommen |
| Räume nur lesend | `RO` | ohne Schreib-Flags |
| Stage-Namen `.se-mount-<16 hex>` | unverändert | unverändert |
| Fehlerabbildung | nach `ErrorKind`: `NotFound`→`ENOENT`, `NotADirectory`→`ENOTDIR`, `IsADirectory`→`EISDIR`, `DirectoryNotEmpty`→`ENOTEMPTY`, `PermissionDenied`→`EACCES`, `ReadOnlyFilesystem`→`EROFS`, `AlreadyExists`→`EEXIST`, `InvalidInput`/`InvalidData`→`EINVAL`, `TimedOut`→`ETIMEDOUT`, `StorageFull`/`WriteZero`→`ENOSPC`, `Unsupported`→`ENOTSUP`, `OutOfMemory`→`ENOMEM`, `WouldBlock` (Konflikt)→`EBUSY`, `Interrupted`→`EINTR`, `CrossesDevices`→`EXDEV`, `ConnectionRefused`→`ECONNREFUSED`, `ConnectionReset`→`ECONNRESET`, `NotConnected`→`ENOTCONN`, `NetworkDown`→`ENETDOWN`, `NetworkUnreachable`→`ENETUNREACH`, `HostUnreachable`→`EHOSTUNREACH`, jede übrige Art → `EIO` | dieselbe Tabelle für `ErrnoException` |

**Windows-Fehlerabbildung (Defektbehebung, B2 Nr. 13):** `io_status` übersetzt nur noch in Win32-Codes, die Dokany 2.3.1.1000 abbildet (Liste `DOKANY_MAPPED_WIN32` aus `dokan/ntstatus.i` am Tag `v2.3.1.1000`); der Rückfall `_ => ERROR_INVALID_DATA` wird durch den direkten NTSTATUS `STATUS_IO_DEVICE_ERROR` ersetzt (Anwendung sieht nach **Annahme A12** `ERROR_IO_DEVICE` 1117 „I/O device error“, NR N27 Punkt 4; Abnahmesignal prüft den tatsächlich gesehenen Code); ein Rohcode außerhalb der Liste ebenso. Zusätzlich: `HostUnreachable` → `ERROR_HOST_UNREACHABLE` (1232), `NetworkUnreachable` → `ERROR_NETWORK_UNREACHABLE` (1231) (beide mit Dokany-Abbildung, Meldung „The network location cannot be reached.“, NR N27), `CrossesDevices` → `ERROR_NOT_SAME_DEVICE` (17). Unverändert: alle übrigen Zeilen der Tabelle.

**Persistenz und Migration (K4, K9):** `registry.json` Version 1 nur mit `SavedRemote`, `GoogleDrive`, `Peer`; neue optionale Felder `location` (`Option<MountLocation>`, `MountLocation::Directory { path }`) und `auto_remount` (`bool`) mit `#[serde(default, skip_serializing_if = …)]` (unberührte Einträge byte-gleich; ältere Stände ignorieren sie). `MountSource::Devices` und `MountSource::Vault { site_id }` nur in `<AD>/mounts/registry-2.json` (`{version: 1, mounts: […]}`, gleiche Datei- und Größenregeln), damit ein zurückgerollter Stand `registry.json` weiter lesen kann. Bestandseinträge `auto_remount = false`.

### C5 — Gerät als Ordner automatisch (E4, K7)

**Wahl L1** Gerätebaum. Aufbau: Wurzel mit je Direktkontakt `<Gerätename>`; `Räume/<Raumname>/<Gerätename>`; im Geräteordner die Peer-Wurzeln (`build_peer_roots`: Top-Level-Freigaben, `Verbindungen/<name>`). Je Freigabe eine Kind-Engine (Modus aus `probe_share_mount_capabilities`, Räume `ReadOnly`), Kind-ID = 32 Hex aus SHA-256(„se-devices-child\0“ ‖ Gerätebaum-ID ‖ Geräteschlüssel ‖ Wurzel). Geräteschlüssel (S5-Anforderung a): `mount::device_key(&PeerOpenTarget) -> String` = `direct~<contact_id>` bzw. `room~<room_id>~<device_id>`, jedes Byte außerhalb `[A-Za-z0-9._-]` als `%XX` (sicher für Dokument-IDs mit `/`). Namensregeln: ungültige Zeichen des Dialekts → `_`; gleiche Namen → Zusatz „ (<6 Hex von SHA-256 des Schlüssels>)“; ein Gerät namens „Räume“ → „ (Gerät)“. Virtuelle Ebenen nur lesbar (`PermissionDenied`); Verschieben zwischen Freigaben → `CrossesDevices` (Linux `EXDEV`, Windows `ERROR_NOT_SAME_DEVICE`; keine Mount-Datei erzeugt diese Art heute, vier Fundstellen außerhalb `N/mount/` prüfen nur empfangene Fehler, NR C5-3).

**Online/offline ohne Leerzustand:** Geräteliste aus den Profilen, nicht aus der Präsenz. Zugriff öffnet die Verbindung bei Bedarf (`open_share`, 45-s-Transportfenster bestätigt NR C5-1); Scheitern → `HostUnreachable` (Windows jetzt `ERROR_HOST_UNREACHABLE`, entschieden nach NR N27; Linux `EHOSTUNREACH`; Android `EXTRA_ERROR`). **Schließen der Kind-Engines (korrigiert NR N23):** kein Zeitwert mehr; beim Ereignis `DirectOffline`/`RoomLeft` schließt der Gerätebaum sofort jede Kind-Engine des Geräts ohne offenes Handle und ohne Journaleintrag; die übrigen nach dem letzten Schließen. Präsenz steuert nur Invalidierung, Wiederholen ausstehender Änderungen beim Wiederkommen und dieses Schließen. Ein nicht erreichbares Gerät hält den Hoststart nicht auf.

**Je Plattform:** Windows: ein Dokany-Laufwerk „Smart Explorer Geräte“ (21 Zeichen; Grenze 32 im se-Code, NR C5-5). Linux: `~/SmartExplorer/Geräte` (automatisch sichtbar als „Geräte“, KDE-Ort; kein GTK-Lesezeichen, C2). Android: Root „Smart Explorer – Geräte“.

**Schalter, Voreinstellung:** „Verbundene Geräte als Ordner“, Voreinstellung ein, wenn die Laufzeit bereit ist (Dokany geprüft; `fusermount3` mit Setuid; Android immer); keine automatische UAC-Abfrage; einmaliger Hinweis mit Buchstabe nach dem ersten Einhängen. Ausschalten hängt aus, entfernt Orte und den Autostart-Eintrag, wenn kein Laufwerk mit `auto_remount` ihn braucht; Journale bleiben. **Offen für den Hauptagenten (K7):** die Voreinstellung „ein“ startet beim Anmelden Daemon und Share-Host und damit die Erreichbarkeit dieses Geräts für Freigaben und Exec (S1).

### C6 — Klartextgrenze für Tresor-Laufwerke (K6, S3-Verträge)

Die frühere eigene Spool-Verschlüsselung (XChaCha20-Poly1305, HKDF aus einem Samen) ist ersetzt durch den S3-Entwurf (S3-Plan C3.6): ein Format für alle Tresor-Ablagen, Kryptografie bei S3.
- Spool: `SpoolStorage` mit S3-`SealedSpoolFile` (Tresor-Inhaltsformat); je Laufwerk ein zufälliger Spool-Schlüssel, mit dem Zweckschlüssel `spool-wrap` per AES-KW gewickelt im Spool-Ordner; der Host erhält nur den Spool-Schlüssel über `Vault::SpoolKey` (S2-V2), nie den Masterkey.
- Journal und `stages.json`: Pfadfelder (`remote_path`, `original_path`, `quarantine_path`, Pfade der Namenskonfliktabsichten, Stage-Pfade) über `PathSealer` mit S3-`seal_text`/`unseal_text` versiegelt (S2-V3); Dateinamen und Format sonst wie Bestand. **P1 (Bestätigung S3):** Im Host ist nur der Spool-Schlüssel vorhanden; S2 verlangt, dass `seal_text`/`unseal_text` mit einem aus dem Spool-Schlüssel abgeleiteten Textschlüssel arbeiten (Zweck `se-mount-journal-v1`), damit der Host ohne IPC je Journaleintrag versiegeln kann.
- Absturz: ohne Spool-Schlüssel (gesperrt) meldet der Host `MountRecovery::RequiresUnlock` mit Zahl ausstehender Einträge; nach dem Entsperren läuft die Bestandswiederherstellung; nichts wird verworfen.
- Sperren (S2-V4, S2-V10): Der Daemon ist Sperrteilnehmer für jeden Tresor-Laufwerkshost; auf die Sperranfrage mit Frist (S3-Plan C3.3) veröffentlicht der Host ausstehende Änderungen, entfernt die Clean-Einträge, hängt aus und bestätigt; reicht die Frist nicht, bleiben Änderungen versiegelt, der Daemon bricht die Verbindung ab (Linux `fusectl`-`abort`, Windows Beenden des Hosts im Job-Objekt) und bestätigt erst danach; nach dem Sperren beantwortet kein Laufwerk mehr Lese- oder Schreibanfragen.
- Unerwartetes Ende (S2-V6, Lehre `whenTerminated`): Ende der FUSE-Sitzung, Dokany-Ende oder Prozessende des Hosts bei entsperrtem Tresor → Daemon meldet dem Sitzungsregister „Laufwerk beendet“ und den Oberflächen (`MountStatus` mit Grund `external_unmount` bzw. `host_exit`).
- Einhängeort (S2-V7): Windows nur Laufwerksbuchstabe; Linux tmpfs unter `$XDG_RUNTIME_DIR` (C2); Android Root ohne MediaStore-Eintrag. **P2 (Abweichung, Bestätigung S3):** S2-V7 verlangt unter Windows eine „Laufwerksart, die Windows Search nicht indiziert“. Dokanys Netzwerkmodus ist in C1 verworfen (#1326, #1305, Konflikt mit dem Network Provider) und `DOKAN_OPTION_REMOVABLE` warnt vor ungewolltem Aushängen (Ref-Win 2.1.4); S2 hängt Tresore deshalb als festes lokales Laufwerk ein und zeigt beim ersten Einhängen den Hinweis zu Indizierung und Miniaturbildern. Dass Windows Search zusätzliche Laufwerke ohne Zutun des Nutzers nicht indiziert, ist **Annahme A13** (S3-Plan G15); Miniaturbilder lassen sich nur global abschalten (NR N17).
- Grenzen außerhalb von se: Miniaturbild-Caches (GNOME stuft `fuse.smartexplorer` als lokal ein, nur globale Schalter; Windows nur globale Richtlinien; NR N17), Auslagerung, Sicherungsdateien fremder Apps.

### C7 — Oberflächen (K8)

| Oberfläche | Windows | Linux | Android |
|---|---|---|---|
| CLI `se drive` | Bestand + `--auto-remount`, `set`, `open`, `devices status\|on\|off\|watch`, Ziel `vault://<SITE>` | alle Unterbefehle; `--mount-point`, `--force`; `runtime`/`install-runtime` melden FUSE und Kernel-Zeitlimit | – (keine CLI, `N/lib.rs:15-16`) |
| Desktop-GUI | Bestand + „Beim Anmelden automatisch einbinden“, Einstellungen → Integration „Verbundene Geräte als Ordner“ (vor „se Desktop“, S5-Plan E1 Punkt 4) | dieselben Fenster; „Einhängeort“; Laufzeithilfe; „Im Dateimanager öffnen“ | – |
| Android-GUI | – | – | „Ordner für Apps bereitstellen“: Schalter Anbieter, Geräte, Verbindungen, Tresore; Grenzentext; ausstehende Speicherungen mit „Erneut versuchen“ |
| Android-Bridge | – | – | `docs.*` + Binär-JNI |
| Daemon-IPC | `Drive { token, req: DriveIpcRequest }` (Optionen, Gerätebaum, Wache, Wiederherstellung, `MountPathFor`) und `MountHostExt { id, session_token, req }` (Kind-Backends, Gerätewache, Sperr-/Befehle) | wie Windows | – |
| MCP | über das Werkzeug `se`; Beispiele im Katalog | wie Windows | – |
| Doku | `docs/oberflaechen/laufwerke.md` und Folgeänderungen | wie Windows | wie Windows |

**P3 (Bestätigung S3):** S3-Plan T22 nennt `se drive add --vault <SITE>`; der Bestand hat `se drive mount <TARGET>`, daher Tresore als Ziel `se drive mount vault://<SITE>` (Bezeichnung oder ID-Präfix wie im S3-Locator, gespeichert wird die Site-ID); kein zweiter Unterbefehl.

### C8 — Installer, Release, Lizenzen

- Windows unverändert (Dokany-MSI 2.3.1.1000, private DLL, Lizenzdateien; NR C8-1). Kein WinFsp-MSI.
- Linux: `fuser` rein Rust; `install-linux.sh` prüft `fusermount3` (Pfad, Setuid) und meldet den Paketbefehl; installiert `fuse3` nur mit `--with-fuse`. Kein `user_allow_other` nötig (NR C8-2).
- Android: neuer `<provider>`; keine neue Berechtigung.
- Update-Feed: keine neue Datei.
- Lizenzhinweise (korrigiert NR C8-3/N25): Es gibt keine Lizenzübersicht; die MIT-Hinweispflicht für `fuser` erfüllt der gemeinsame Integrationsmeilenstein des Hauptagenten für die Lizenzhinweise aller Rust-Crates; S2 liefert dafür nur die Angabe „`fuser 0.18.0`, MIT, Linux-Binärdateien“ und keine eigene Lösung.

## D — Meilensteine (Umsetzungsreihenfolge)

Gemeinsam: neue Rust-Dateien < 500 Zeilen (Schätzung in Klammern); vor dem Erweitern einer Datei mit mehr als 450 Zeilen wird ein zusammenhängender Teil ausgegliedert (B2 Nr. 15); `N/mount/core/` ohne `cfg`, ohne OS-Crates, ohne FFI; Plattformcode unter `N/mount/os/{windows,linux_os,android,shared}`; jede Oberflächenänderung aktualisiert `docs/oberflaechen/laufwerke.md` im selben Commit; Graphify nach nativen Änderungen. Abnahmesignale tragen das Präfix `drive_task_` und laufen in den Stufen aus DRV-M10 der einen Stapel-Suite.

### DRV-M1 — Pfaddialekt in der Engine

- **Ergebnis:** kein Bedienweg; Engine verarbeitet Linux-/Android-Namen; Windows unverändert.
- **Neue Dateien:** `N/mount/core/path_dialect.rs` (250), `path_dialect_tests.rs` (300).
- **Geänderte Dateien:** `N/mount/core/path.rs`, `metadata_loading.rs`, `engine_recovery.rs`, `peer_names.rs`; `types.rs` erst nach Ausgliederung von `MountSource`/`PeerMountTarget` in `N/mount/core/mount_source.rs` (90, zugleich S3-Plan VLT-M12) und von `MountStatus` und den Ergebnis-Enums in `mount_status.rs` (150), dann Feld `path_dialect` in `MountRuntimeConfig`; `N/daemon/os/shared/ipc_protocol.rs` (`MountHostConfig.path_dialect`, `#[serde(default)]`); `N/mount/mod.rs`.
- **Verträge:** `PathDialect` (`windows`/`posix`); erhalten: Windows-Prüfungen, Journal-Format, serde-Form von `MountSource`/`MountStatus` (Ausgliederung ohne Formänderung).
- **Plattformprüfung:** `NAME_MAX` 255 Bytes gegen 255 UTF-16-Einheiten.
- **Abnahmesignal:** `drive_task_dialect_windows_table_unchanged`, `drive_task_dialect_posix_accepts_colon_con_trailing_dot`, `drive_task_dialect_posix_rejects_nul_slash_backslash_dotdot`, `drive_task_dialect_posix_name_max_bytes`, `drive_task_posix_listing_shows_windows_invalid_names`, `drive_task_posix_listing_hides_backslash_names`, `drive_task_types_split_serde_unchanged`.

### DRV-M2 — Frontend-Grenze, gemeinsamer Hostablauf, Windows-Fehlerabbildung

- **Ergebnis:** Windows-Laufwerke laufen über das Trait; ein nicht einzeln abgebildeter Fehler erscheint im Explorer nicht mehr als „Zugriff verweigert“, sondern als E/A-Gerätefehler; Offline-Netzfehler als „Netzwerkort nicht erreichbar“.
- **Neue Dateien:** `N/mount/core/frontend.rs` (300), `frontend_engine.rs` (250), `spool_storage.rs` (200, Trait + Klartext-Umsetzung), `path_sealer.rs` (80), `N/mount/os/shared/host_session.rs` (400), `host_platform.rs` (120), `change_pump.rs` (250), `N/mount/os/linux_os/cache_lease.rs` (250), `cache_space.rs` (80), `N/mount/os/windows/dokany_status_table.rs` (150: `DOKANY_MAPPED_WIN32` aus `ntstatus.i` Tag `v2.3.1.1000`), Tests `host_session_tests.rs` (300), `spool_storage_tests.rs` (250).
- **Geänderte Dateien:** `N/mount/core/spool.rs`, `stage_ledger.rs`, `file_io.rs`, `materialization.rs`, `clean_cache.rs` (über `SpoolStorage`); `journal.rs` erst nach Ausgliederung der Satztypen in `journal_record.rs` (200), dann `PathSealer`-Haken; `N/mount/os/windows/host.rs`, `metadata_refresh.rs`, `notifications.rs`, `callback_context.rs`, `callbacks_open.rs`, `callbacks_io.rs`, `callbacks_mutation.rs`, `callbacks_metadata.rs`, `callback_status.rs` (Tabellenprüfung, Rückfall `STATUS_IO_DEVICE_ERROR`, drei neue Zeilen C4); `N/mount/os/mod.rs`; `N/mount/mod.rs`; Windows-Harnesses, die das alte „Zugriff verweigert“ für nicht abgebildete Arten erwarten, werden mitgeändert (Prüfpunkt: `vault_volume_errors.rs`, `windows_remote_volume_task_tests.rs`).
- **Verträge:** `MountFilesystem`, `SpoolStorage`, `PathSealer`; Fehlerabbildung C4; erhalten: Hostprotokoll, Recovery-Reihenfolge, Spool-/Journal-Bytes im Klartextfall.
- **Abhängigkeiten:** DRV-M1.
- **Plattformprüfung:** `flock` auf Android `filesDir` (NR N13).
- **Abnahmesignal:** Windows-Bestandsharnesses `mounted_volume_task_tests`, `optimization_volume_tests`, `windows_remote_volume_task_tests`, `recovery_cache_task_tests`, `mount_options_task_tests` grün; `drive_task_windows_unmapped_error_is_io_device_not_access_denied` (Fixture-Backend liefert `ErrorKind::Other`; Anwendung sieht nicht 5; gesehener Code wird berichtet, Erwartung 1117), `drive_task_windows_status_table_covers_emitted_codes`, `drive_task_windows_host_unreachable_maps_1232`, `drive_task_boundary_delegates_every_operation`, `drive_task_host_session_order_matches_windows_host`, `drive_task_plain_spool_bytes_unchanged`, `drive_task_unix_lease_excludes_second_host`, `drive_task_unix_lease_rejects_symlinked_cache`.

### DRV-M3 — Persistenz, Optionen, Wiederherstellung beim Anmelden

- **Ergebnis:** „Beim Anmelden automatisch einbinden“ je Laufwerk (Desktop, `se drive mount … --auto-remount`, `se drive set <SELEKTOR> --auto-remount on|off`); nach Neuanmeldung sind diese Laufwerke ohne Öffnen von se da.
- **Neue Dateien:** `N/daemon/os/shared/mount_registry_ext.rs` (250: `registry-2.json`), `mount_restore.rs` (250), `mount_manager_start.rs` (300: Startpfad aus `mount_manager.rs` ausgegliedert), `drive_autostart.rs` (150), `run_start_reason.rs` (80: Startgrund-Merker, Randfall `disabled_by_system`), `ipc_protocol_drives.rs` (300: `DriveIpcRequest`), `N/mount/core/mount_location.rs` (120), `N/cli/drive_set.rs` (150), `N/app/core/mount_ui_location.rs` (200: Einhängeort-Feld und Option aus `mount_ui_draft.rs` ausgegliedert), Test `registry_compat_tests.rs` (300).
- **Geänderte Dateien:** `N/mount/core/types.rs` (Felder `location`, `auto_remount`), `mount_source.rs` (Varianten `Devices`, `Vault { site_id }`), `mount_status.rs` (`MountedAt { path }`), `N/daemon/os/shared/mount_registry.rs`, `mount_manager.rs`, `run_loop.rs` (eine Zeile: Bedingung aus `run_start_reason`), `ipc_protocol.rs` (eine Variante `Drive { token, req }`), `N/autostart/os/linux_os.rs`, `windows.rs`, `N/lib.rs`, `N/main.rs` (`--drive-restore`), `N/cli/drive.rs` (Aufruf der ausgegliederten Unterbefehle), `drive_options.rs`, `N/app/core/mount_ui.rs`, `mount_ui_draft.rs`.
- **Verträge:** Persistenz C4; `se drive set <SELEKTOR> --auto-remount on|off [--json]` → Text `drive\t<id>\tauto-remount=<on|off>`, JSON `MountSnapshot`; `list --json` um `config.auto_remount`/`config.location` (nur wenn gesetzt); IPC `DriveIpcRequest::{SetOptions { id, auto_remount }, Restore}`; Exitcodes unverändert.
- **Oberflächen:** CLI (Text, JSON, Vervollständigung des Selektors), Desktop, IPC, MCP über `se`, Doku.
- **Abhängigkeiten:** DRV-M2.
- **Abnahmesignal:** `drive_task_registry_v0_5_182_fixture_roundtrip_identical`, `drive_task_registry_old_parser_ignores_new_fields`, `drive_task_registry_ext_kinds_never_in_v1_file`, `drive_task_existing_drive_after_restart_unchanged`, `drive_task_auto_remount_after_daemon_restart` (Windows Dokany, Linux FUSE), `drive_task_restore_runs_after_recovery_audit`, `drive_task_drive_restore_no_jobs_without_sync_autostart`, `drive_task_drive_restore_no_jobs_when_sync_entry_disabled_by_system`, `drive_task_autostart_entry_follows_need`, `drive_task_cli_set_text_and_json`.

### DRV-M4 — Linux-FUSE-Host

- **Ergebnis:** Unter Linux bindet „Als Laufwerk einbinden“ bzw. `se drive mount @prod:/srv` unter `~/SmartExplorer/<Name>` ein; Lesen, Schreiben, Editor-Speichern über Umbenennen, `cp -a` und `rsync -a` ohne Fehlercode, Löschen, Ordner anlegen; Auswerfen, „Erneut versuchen“, `--force`.
- **Neue Dateien:** `N/mount/os/linux_os/mod.rs` (60), `fuse_host.rs` (400), `fuse_fs.rs` (450), `fuse_ops_meta.rs` (400), `fuse_ops_io.rs` (400), `fuse_ops_attr.rs` (250: `setattr`-Regeln C2), `fuse_ops_ns.rs` (300), `fuse_inodes.rs` (400), `fuse_errno.rs` (120), `fuse_workers.rs` (250), `fuse_notify.rs` (150), `mount_point.rs` (350: Orte, Prüfung, `0500`/`0700`, Sticky-Bit unter `$XDG_RUNTIME_DIR`, tmpfs-Prüfung, toter Mount), `fuse_runtime.rs` (300: Bereitschaft, Kernel-Zeitlimit-Sysctls, Sandkasten/WSL, Paketbefehl), `fuse_abort.rs` (120: `mountinfo` → `fusectl`-`abort`), `N/mount/core/pending_attributes.rs` (200: Zeit/Modus am Eintrag, Übergabe an `finish_stage`), `N/daemon/os/shared/rooted_backend_extensions.rs` (150: nur `finish_stage` mit Wurzelpfad), `mount_proxy_extensions.rs` (100: nur `finish_stage`), `N/daemon/os/linux_os/mount_cleanup.rs` (150), `N/cli/drive_output_linux.rs` (200), Tests `fuse_inodes_tests.rs`, `mount_point_tests.rs`, `pending_attributes_tests.rs` (je < 500).
- **Geänderte Dateien:** `N/mount/core/file_commit.rs` (Aufruf `finish_stage` nur bei ausstehenden Attributen), `N/daemon/os/shared/rooted_backend.rs` und `mount_proxy.rs` (je `fn extensions()` auf die neuen Dateien), `N/daemon/os/linux_os/mount_process.rs`, `N/mount/mod.rs` (`drive_mount_supported()` = `cfg!(any(windows, target_os = "linux"))`, `drive_runtime_info`/`install_drive_runtime`/`run_host_if_requested` je OS), `N/mount/os/mod.rs`, `mount_manager_start.rs` (Linux-Startpfad; „nicht unterstützt“ nur `target_os = "android"`), `mount_manager_start_cache.rs`, `native/Cargo.toml` (`fuser`), `N/cli/drive.rs`, `drive_options.rs`, `N/app/core/mount_ui.rs`, `mount_runtime_ui.rs`, `shell_commands.rs`, `sidebar.rs`, `N/cli/mcp/catalog.rs` (Beispiel `["drive","mount","@prod:/srv","--mount-point","/home/u/SmartExplorer/prod"]` (Linux)).
- **Verträge:** `se drive runtime [--json]` → Text `fuse\tready\tfusermount3=<Pfad>\tversion=<v>\tsetuid=yes\tdevice=/dev/fuse\tkernel_request_timeout=<s>`; JSON `{"platform":"linux","ready","device":{"path","accessible"},"fusermount3":{"path","setuid","version"},"kernel_request_timeout_s","kernel_max_request_timeout_s","sandbox":"none|flatpak|snap|container","wsl","problems":[…]}`; `install-runtime` → `{"status":"manual_required","command","reason"}` (Exit 2) bzw. `already_ready` (Exit 0); `mount <TARGET> [--mount-point PFAD] [--read-write] [--trust-remote-root] [--metadata-depth 0..=4] [--cache-mib 0..=65536] [--label TEXT] [--auto-remount] [--json]`; `--letter`/`--system-runtime` unter Linux → Fehler „Laufwerksbuchstaben gibt es nur unter Windows; nutze --mount-point“ (Exit 2); `unmount <SELEKTOR> [--force] [--json]`; `list` Spalte 3 = Einhängepfad, Status `mounted:<Pfad>`; Windows-Ausgaben unverändert. IPC: `MountHostConfig.location`; Backend-Erweiterung `finish_stage` über den Mount-Proxy.
- **Oberflächen:** CLI (inkl. Pfadvervollständigung), Desktop Linux, IPC, MCP über `se`, Doku.
- **Abhängigkeiten:** DRV-M1–M3.
- **Plattformprüfung:** `ubuntu-24.04`-Runner mit `fuse3` per apt und unprivilegiertem FUSE (NR N24); Kernel ≥ 4.5 für `clone_fd`.
- **Abnahmesignal:** `drive_task_fuse_mount_local_fixture_rw`, `drive_task_fuse_sftp_container_rw`, `drive_task_fuse_editor_save_rename_over`, `drive_task_fuse_gio_replace_stays_atomic` (Modus gleich → kein Rückfall), `drive_task_fuse_cp_a_exit_zero_and_times_applied_on_sftp`, `drive_task_fuse_rsync_a_exit_zero`, `drive_task_fuse_chown_other_uid_eperm`, `drive_task_fuse_conflict_returns_ebusy_and_copy`, `drive_task_fuse_large_readonly_range_read`, `drive_task_fuse_names_colon_con_listed`, `drive_task_fuse_links_hidden`, `drive_task_fuse_unmount_busy_then_force`, `drive_task_fuse_force_abort_frees_blocked_reader`, `drive_task_fuse_kernel_request_timeout_reported` (Runner setzt `fs.fuse.default_request_timeout` per `sudo`; langsames Fixture-Backend; Status-Grund, Journal bleibt, Wiederholung veröffentlicht), `drive_task_fuse_host_kill_daemon_cleans_mount`, `drive_task_fuse_stale_mount_cleaned_before_remount`, `drive_task_fuse_mountpoint_locked_after_unmount`, `drive_task_fuse_recovery_retries_pending_after_restart`, `drive_task_fuse_runtime_report_missing_fusermount`, `drive_task_runtime_detects_sandbox_markers`, `drive_task_cli_linux_shapes`, `drive_task_smb_mount_still_refused`, `drive_task_windows_drive_ignores_pending_attributes` (Windows: kein `finish_stage`-Aufruf).

### DRV-M5 — Linux-Dateimanager-Einbindung

- **Ergebnis:** se-Laufwerke erscheinen in Nautilus, Nemo, Thunar (automatisch bzw. per Lesezeichen für Orte außerhalb der GLib-Regel) und Dolphin (Orte); „Im Dateimanager öffnen“ und `se drive open <SELEKTOR>`.
- **Neue Dateien:** `N/mount/os/linux_os/bookmarks_gtk.rs` (250), `bookmarks_kde.rs` (350), `file_manager_open.rs` (120: `pub(crate) fn show_folders`, für S5), `bookmarks_tests.rs` (350), `N/cli/drive_open.rs` (120).
- **Geänderte Dateien:** `fuse_host.rs`, `N/daemon/os/linux_os/mount_cleanup.rs`, `N/cli/drive.rs`, `N/app/core/mount_ui.rs`.
- **Verträge:** GTK-Zeile nur für Orte außerhalb der GLib-Regel; XBEL `urn:smart-explorer:places`/`<id>`; `se drive open <SELEKTOR> [--json]` → `{"opened":"<Pfad>","method":"filemanager1|xdg-open|explorer"}` (Windows öffnet den Buchstaben im Explorer).
- **Abnahmesignal:** `drive_task_gtk_bookmark_only_outside_glib_rule`, `drive_task_gtk_bookmark_removed_exactly`, `drive_task_kde_places_preserves_foreign_entries`, `drive_task_bookmarks_cleaned_after_host_crash`, `drive_task_glib_display_rule_path`, `drive_task_cli_open_json`.

### DRV-M6 — Gerätebaum und Automatik

- **Ergebnis:** Ohne se zu öffnen erscheinen gepaarte Geräte und Raumgeräte als Ordner (Windows Laufwerk „Smart Explorer Geräte“, Linux `~/SmartExplorer/Geräte`); offline bleiben sie gelistet und melden „nicht erreichbar“; Schalter in Einstellungen → Integration und `se drive devices on|off`.
- **Neue Dateien:** `N/mount/core/device_tree.rs` (450), `device_tree_names.rs` (250: Namensregeln, `device_key`), `device_tree_children.rs` (350), `peer_roots.rs` (150, aus `N/app/core/mount_peer_roots.rs`), `N/daemon/os/shared/devices_mount.rs` (400), `devices_ipc.rs` (250: Status, Wache), `ipc_protocol_mount_host.rs` (250: `MountHostRequest::{ChildBackend, DeviceWatch, PrepareLock}`), `N/cli/drive_devices.rs` (250), `N/app/core/settings_devices_ui.rs` (250), Tests `device_tree_tests.rs`, `device_tree_names_tests.rs`.
- **Geänderte Dateien:** `ipc_protocol.rs` (eine Variante `MountHostExt { id, session_token, req }`), `ipc_protocol_drives.rs` (`DevicesStatus`, `DevicesEnable { letter, mount_point }`, `DevicesDisable`, `DevicesWatch { since }`), `mount_client.rs`, `mount_source.rs` (Daemon), `mount_manager_start.rs` (Singleton), `N/mount/os/windows/host.rs`, `N/mount/os/linux_os/fuse_host.rs`, `N/app/core/mount_peer_roots.rs`, `settings_ui.rs`, `mount_ui.rs`, `N/cli/mod.rs`, `N/cli/mcp/catalog.rs`, `N/mount/mod.rs` (Re-Export `device_key`).
- **Verträge:** `se drive devices status [--json]` → Text `devices\t<on|off>\t<state>\t<mounted|unmounted>\t<Wurzelpfad|->` und je Gerät `device\t<key>\t<Name>\t<direct|room:<Raum>>\t<online|offline>\t<Ordnerpfad>`; JSON `{"enabled","state","mounted","root_path","letter","mount_point","revision","devices":[{"key","name","kind","room","online","path"}]}`; `se drive devices watch [--since N] [--json]` (eine Antwort nach Änderung oder nach 30 s); `on [--letter auto|X] [--mount-point PFAD]`, `off`; IPC wie oben; Wache antwortet nach Änderung oder spätestens 30 s (unter der 50-s-IPC-Frist des Bestands, `mount_probe_client.rs:23`); Persistenz `registry-2.json`; Offline-Fehler `HostUnreachable`.
- **Oberflächen:** CLI, Desktop, IPC, MCP über `se`, Doku; Android in DRV-M8.
- **Abhängigkeiten:** DRV-M2–M5; S5 nutzt E2.
- **Plattformprüfung:** Windows ohne `OPTION_CASE_SENSITIVE` für die virtuellen Ebenen; Linux virtuelle Ordner `0555`.
- **Abnahmesignal:** `drive_task_devices_folder_appears_without_manual_steps`, `drive_task_devices_offline_error_not_empty` (Linux `EHOSTUNREACH`, Windows 1232), `drive_task_devices_online_retries_pending_upload`, `drive_task_devices_room_readonly`, `drive_task_devices_cross_export_move_exdev`, `drive_task_devices_names_collision_deterministic`, `drive_task_devices_key_matches_s5_rule`, `drive_task_devices_offline_closes_idle_children_only`, `drive_task_devices_offline_peer_does_not_block_host_start`, `drive_task_devices_switch_off_unmounts_and_removes_place`, `drive_task_devices_watch_returns_within_30s`, `drive_task_devices_windows_drive_lists_peer`, `drive_task_devices_single_peer_mounts_unchanged`, `drive_task_cli_devices_shapes`.

### DRV-M7 — Tresor-Laufwerke (S3-Verträge S2-V1 bis S2-V10)

- **Ergebnis:** Ein entsperrter Tresor lässt sich unter Windows als Buchstabe und unter Linux unter `$XDG_RUNTIME_DIR/smart-explorer/Tresore/<Name>` einhängen (S3-Kontextmenü „Als Laufwerk einhängen“, `se drive mount vault://<SITE>`); ohne Klartext auf der Platte; Sperren hängt aus; Absturzreste werden nach dem Entsperren gespeichert; `mount_path_for` öffnet Tresordateien im Laufwerk.
- **Neue Dateien:** `N/daemon/os/shared/mount_paths.rs` (250: `pub fn mount_path_for(location: &str) -> Result<Option<PathBuf>, String>`, `pub fn ensure_mounted_for(location: &str) -> Result<PathBuf, String>`), `mount_lock_participant.rs` (250), `mount_termination.rs` (200), Test `mount_paths_tests.rs` (300).
- **Geänderte Dateien:** `N/daemon/os/shared/mount_source.rs` (`Vault { site_id }` → Backend aus dem Sitzungsregister; gesperrt → `vault_locked`), `mount_manager_start.rs`, `N/mount/os/shared/host_session.rs` (Spool-Schlüssel über `Vault::SpoolKey`, `SealedSpoolFile`, `PathSealer`), `recovery_state.rs` (`MountRecovery::RequiresUnlock`), `N/mount/os/linux_os/mount_point.rs` (Tresor-Ort), `N/app/core/mount_ui_helpers.rs`, `N/cli/drive.rs` (Status `requires-unlock:<n>`, `failed:vault_locked`).
- **Verträge:** E1; CLI-Status `requires-unlock:<n>`, JSON `recovery: "requires_unlock"`; IPC `DriveIpcRequest::MountPathFor { location }`, `MountHostRequest::PrepareLock { deadline_ms }`.
- **Abhängigkeiten:** S3-Plan VLT-M4 (Sitzungen, IPC) und VLT-M12 (`SealedSpoolFile`, Spool-Schlüssel, `vault::sealed`); DRV-M2–M4.
- **Abnahmesignal (S2-Seite; die Kryptografie prüft S3 mit `vault_task_*`):** `drive_task_vault_spool_uses_sealed_storage` (Test-Versiegler), `drive_task_vault_journal_path_fields_sealed`, `drive_task_vault_locked_start_fails_vault_locked`, `drive_task_vault_requires_unlock_after_crash`, `drive_task_vault_prepare_lock_flushes_evicts_unmounts`, `drive_task_vault_lock_deadline_keeps_sealed_and_aborts`, `drive_task_vault_unexpected_end_reported`, `drive_task_vault_linux_mountpoint_tmpfs_sticky_locked`, `drive_task_vault_linux_refuses_non_tmpfs`, `drive_task_mount_path_for_maps_drive_paths`, `drive_task_vault_drive_letter_rw` (Windows).

### DRV-M8 — Android-Dokumentanbieter

- **Ergebnis:** In der Systemauswahl und der App „Dateien“ erscheinen „Smart Explorer – Geräte“, jede gespeicherte Verbindung (einschließlich SMB), Google Drive und jeder entsperrte Tresor; öffnen, bearbeiten, anlegen, umbenennen, löschen, Ordnerfreigabe; Einstellungen „Ordner für Apps bereitstellen“.
- **Neue Dateien (Kotlin):** `A/system/documents/SeDocumentsProvider.kt`, `DocumentIds.kt`, `ProxyFileCallback.kt`, `DocumentsCursors.kt`, `DocsCommitWorker.kt` (mit `getForegroundInfo()`), `A/api/DocsApi.kt`, `A/ui/settings/DocumentsProviderSection.kt`; Tests `DriveTaskProviderTest.kt` (Robolectric), `DriveTaskProviderDeviceTest.kt` (Instrumentierung).
- **Neue Dateien (Rust):** `N/mobile/os/shared/domains/docs.rs` (400), `docs_engines.rs` (350), `docs_io.rs` (300), `docs_ids.rs` (200), `docs_vault.rs` (200: Tresor-Roots, Sperrteilnehmer im Prozess), `native/android-bridge/src/io.rs` (250: `docOpen`, `docRead`, `docWrite`, `docSize`, `docFsync`, `docRelease`).
- **Geänderte Dateien:** `android/app/src/main/AndroidManifest.xml`, `A/core/NativeBridge.kt`, `A/ui/settings/SettingsScreen.kt`, `N/mobile/os/shared/domains/mod.rs`, `native/android-bridge/src/lib.rs`, `N/mount/core/range_read.rs`, `types.rs` (`RangeReadPolicy`), `docs/superpowers/plans/2026-09-25-android-apk/api.md`.
- **Verträge:** Bridge `docs.roots`, `docs.document`, `docs.children`, `docs.create`, `docs.rename`, `docs.delete`, `docs.settings.get/set {provider, devices, connections, vaults}`, `docs.pending`, `docs.retryPending`; Ereignis `docs.changed`; JNI mit negiertem errno; Dokument-IDs C3.
- **Abhängigkeiten:** DRV-M1, DRV-M2, DRV-M6; Tresor-Roots S3 VLT-M4/VLT-M12 und DRV-M7.
- **Abnahmesignal:** Robolectric `drive_task_android_roots_follow_settings`, `drive_task_android_locked_vault_has_no_root`, `drive_task_android_is_child_without_io`, `drive_task_android_children_loading_then_notify`, `drive_task_android_offline_device_extra_error`, `drive_task_android_mode_wt_truncates_w_not`, `drive_task_android_append_rejected`, `drive_task_android_ids_stable_after_device_rename`, `drive_task_android_reader_sees_dirty_entry`, `drive_task_android_local_only_flag_only_for_local_vaults`, `drive_task_android_provider_init_before_application`, `drive_task_android_commit_worker_has_foreground_info`; Emulator `drive_task_android_open_read_seek_large_file_range`, `drive_task_android_write_close_uploads`, `drive_task_android_process_kill_retries_pending`, `drive_task_android_tree_grant_persists_restart`, `drive_task_android_handler_thread_per_file`, `drive_task_android_smb_root_rw`, `drive_task_android_mediastore_has_no_provider_files`.

### DRV-M9 — Inventur, Doku, Installer

- **Neue Dateien:** `docs/oberflaechen/laufwerke.md`.
- **Geänderte Dateien:** `docs/oberflaechen/INDEX.md`, `docs/oberflaechen/einstellungen.md` (Ausnahme ersetzt durch „Laufwerke: Windows und Linux; Android: Dokumentanbieter (Plattformgrund)“), `README.md`, `docs/RELEASING.md` (fuse3 als Laufzeitvoraussetzung), `docs/GOTCHAS.md` (FUSE-Grenze: kein `auto_unmount`, Verriegelung, Interrupt, `setattr`-Regeln, Kernel-Zeitlimit, Dialekt; Windows-Fehlerabbildung), `docs/ARCHITEKTUR.md`, `docs/TODO.md` (`WINFSP-AB`, `DOKANY-WATCH`), `install-linux.sh` (`--with-fuse`), `N/cli/mcp/catalog.rs` (Beispiele; `INSTRUCTIONS` nur in Abstimmung mit dem Hauptagenten wegen der gemeinsamen Längengrenze 2 048, S5-Plan).
- **Abnahmesignal:** `drive_task_install_linux_dry_run_with_fuse`, `drive_task_docs_surfaces_complete`, Veraltetsuche „nur unter Windows“.

### DRV-M10 — Stufen in der einen Stapel-Suite

Kein eigener Workflow: Der Hauptagent führt die Suite des Stapels zusammen. S2 liefert diese Stufen (Testpräfix `drive_task_`, Zeitlimit je Job ≥ 30 min, Runner fest `ubuntu-24.04` und `windows-2025`): `drive-unit` (Linux, Windows), `drive-fuse` (Linux; `apt-get install fuse3`; `sudo` nur für das Kernel-Zeitlimit-Szenario), `drive-devices` (Linux- und Windows-Share-Paar über den Kandidaten-Server), `drive-desktop` (Linux, temporäres `HOME`), `drive-cli` (Linux, Windows), `drive-dokany-regression` (Windows, Bestandsharnesses namentlich und `drive_task_windows_*`), `drive-vault` (Linux, Windows; nach S3), `drive-android` (Robolectric), `drive-android-device` (Emulator, SFTP-Container über `10.0.2.2`), `drive-install` (Linux). Jedes Abnahmesignal aus DRV-M1 bis DRV-M9 genau einmal; Bestandsverträge zusätzlich: Stage-Namen `.se-mount-*`, SMB-Ablehnung am Desktop, `MountSnapshot`-JSON, CLI-Fehlertexte (Kern-Lesung Abschnitt 5). Überschneidung mit S3: Kryptografie, Spool-Schlüssel und Absturz nach Entsperren prüft S3 (`vault_task_*`), die Frontend-Seite prüft S2.

## E — Schnittstellen als Verträge

### E1 — S2 ↔ S3 (Übernahme S2-V1 bis S2-V10)

| Nr | Vertrag S3 | Umsetzung S2 | Meilenstein |
|---|---|---|---|
| S2-V1 | `MountSource::Vault { site_id }`, Wurzel `/`, Start bei gesperrtem Tresor scheitert mit `vault_locked`, Registry nur Site-ID | übernommen; Variante in `N/mount/core/mount_source.rs` nach Ausgliederung aus `types.rs`; gespeichert nur in `registry-2.json` | DRV-M1, DRV-M3, DRV-M7 |
| S2-V2 | `SealedSpoolFile`, Spool-Schlüssel über `Vault::SpoolKey`, nie Masterkey | übernommen; Trait `SpoolStorage` (S2), Umsetzung `SealedSpoolFile` (S3) | DRV-M2, DRV-M7 |
| S2-V3 | Pfadfelder in `journal.jsonl` und `stages.json` über `vault::sealed` | übernommen über `PathSealer`; **P1:** Textschlüssel aus dem Spool-Schlüssel (Zweck `se-mount-journal-v1`), weil der Host außer dem Spool-Schlüssel keinen Schlüssel erhält | DRV-M2, DRV-M7 |
| S2-V4 | Sperrteilnehmer mit Frist; keine Antwort nach dem Sperren | übernommen (`PrepareLock`, Abbruch bei Fristende, Bestätigung erst danach) | DRV-M7 |
| S2-V5 | Android: Root je entsperrtem Tresor, gestreamt, keine Cache-Kopie, se öffnet über diese URIs | übernommen; Lesen gestreamt über `open_read_at`, Schreiben nur über den versiegelten Spool | DRV-M8 |
| S2-V6 | Erkennen eines unerwarteten Endes, Meldung an Worker und Oberflächen | übernommen | DRV-M7 |
| S2-V7 | Windows nur Buchstabe, Laufwerksart ohne Indizierung; Linux unter `$XDG_RUNTIME_DIR` plus Lesezeichen; Android kein MediaStore | Linux und Android übernommen (tmpfs geprüft, Sticky-Bit, GTK-Lesezeichen und KDE-Ort); **P2 Abweichung Windows:** festes lokales Laufwerk, keine besondere Laufwerksart (C6), Annahme A13 | DRV-M5, DRV-M7, DRV-M8 |
| S2-V8 | `mount_path_for(location) -> Option<PathBuf>`, Einhängen auf Anforderung | übernommen mit Fehlerkanal: `Result<Option<PathBuf>, String>` (IPC kann scheitern) und `ensure_mounted_for` | DRV-M7 |
| S2-V9 | Engine verlässt sich nur auf `mount_path_capabilities` der Tresorschicht | übernommen (Bestand: Fähigkeiten aus dem Backend) | DRV-M7 |
| S2-V10 | Clean-Einträge beim Sperren entfernt | übernommen (Teil von `PrepareLock`) | DRV-M7 |
| T22 (S3-Plan C7) | `se drive add --vault <SITE>` | **P3:** `se drive mount vault://<SITE>` (Bestandsunterbefehl) | DRV-M7 |
| K09/K11 (S3-Plan C4) | Freigabewurzeln in Tresor-Einhängepunkten ablehnen, Ordnerindex ohne Tresorpfade | S2 liefert die Einhängepfade aller Tresor-Laufwerke über `list_mounts` (`MountedAt`); Prüfung bei S3 | DRV-M7 |

### E2 — S2 → S5 (Übernahme S5-Plan E1)

1. Status über IPC `DriveIpcRequest::DevicesStatus`, CLI `se drive devices status --json`, Android `docs.roots`/`docs.children`; Authority `${applicationId}.documents`, Root-ID `devices`, Dokument-IDs `devices/<key>/…`; Ereignis `docs.changed`.
2. Zusatzanforderungen S5, übernommen: (a) `mount::device_key(&PeerOpenTarget) -> String` (Regel C5); (b) Status-Merkmal `"mounted"` und `"root_path"`; (c) IPC-Wache `DevicesWatch { since }` für Nicht-Host-Leser mit Antwort nach Änderung oder spätestens 30 s (S5 braucht keine 5-s-Abfrage); (d) `pub(crate) fn show_folders` in `N/mount/os/linux_os/file_manager_open.rs`.
3. Zuständigkeit wie S5-Plan E1 Punkt 3: S2 hängt ein, besitzt Buchstaben, GTK-Lesezeichen, KDE-Orte, `smart-explorer-drives.desktop` und den Schalter; S5 besitzt nur den Explorer-Navigationseintrag und schaltet den Gerätebaum nie.
4. Einstellungen → Integration: „Verbundene Geräte als Ordner“ (S2) vor „se Desktop“ (S5).

### E3 — S1 und S4

S4: keine Abhängigkeit. S1: Die Automatik (K7) startet den Daemon beim Anmelden und damit den Share-Host; ob Exec-Freigaben dann ab Anmeldung erreichbar sein dürfen, entscheidet der Hauptagent mit S1.

## F — Risiken und unbestätigte Annahmen

| Nr. | Risiko oder Annahme | Wirkung, wenn falsch | Gegenmaßnahme / Abnahmesignal |
|---|---|---|---|
| A1 | `OPTION_MOUNT_MANAGER` wirkt ohne Administratorrechte (NR C1-14) | keine für diesen Stapel (Bestand unverändert) | – |
| A2 | WinFsp-Zeitablauf beendet nur die Anfrage (NR C1-17) | nur Vergleichstabelle C1 | Teil von T2 |
| A3 | Obsidian/VS Code nutzen `realpath.native`/`promises.realpath` (NR N3) | Gewicht von Grund 3 in C1 | `drive_task_windows_realpath_native_on_dokany` (Messung auf Dokany) |
| A4 | Dokany-Treiberzertifikat läuft um 2026-12 ab (NR C1-5) | kein signiertes Dokany-Release mit Korrekturen | `DOKANY-WATCH` |
| A5 | `fusectl`-Verzeichnis = minor aus `mountinfo` (NR C2-10) | `--force` findet die Verbindung nicht | `drive_task_fuse_force_abort_frees_blocked_reader` |
| A6 | Nullwert der negativen TTL in libfuse (NR C2-12) | keine (se setzt den Wert) | – |
| A7 | `/mnt/wsl` für Nutzer beschreibbar (NR N16) | WSL-Hinweis ohne Nutzen | kein WSL-Runner; nur Doku |
| A8 | Dolphin zeigt `fuse.smartexplorer` nicht selbst (NR C2-19) | doppelter Eintrag in Dolphin | kein Dolphin im Runner; Doku |
| A9 | Flatpak/Snap-Grenzen (NR C2-24) | Meldungstext | `drive_task_runtime_detects_sandbox_markers` (nur Erkennung) |
| A10 | Bedeutung `FLAG_LOCAL_ONLY` (NR C3-3) | Sichtbarkeit lokaler Tresore bei `EXTRA_LOCAL_ONLY` | `drive_task_android_local_only_flag_only_for_local_vaults` (se-Seite) |
| A11 | MediaStore indiziert Anbieterinhalte nicht (NR N12) | Tresornamen in Galerien | `drive_task_android_mediastore_has_no_provider_files` |
| A12 | `STATUS_IO_DEVICE_ERROR` erscheint als `ERROR_IO_DEVICE` 1117 (NR N27 Punkt 4) | Meldungstext weicht von 1117 ab; Kern der Behebung (nicht 5) bleibt | `drive_task_windows_unmapped_error_is_io_device_not_access_denied` berichtet den Code |
| A13 | Windows Search indiziert zusätzliche Laufwerke nicht ohne Zutun (S3-Plan G15) | Tresorinhalte im Suchindex | P2 an S3; Hinweis beim ersten Einhängen |
| R1 | `fuser`-Pflege (keine Pull Requests, 58 unveröffentlichte Commits) | Korrekturen fehlen | Pin, Kapselung, Fork-Notfallweg |
| R2 | Prozesse in zugestellten FUSE-Anfragen nicht beendbar (belegt NR N1) | hängende Programme bei hängendem Backend | `--force`; Doku |
| R3 | Administrator setzt `fs.fuse.default_request_timeout` | Laufwerk bricht bei langem Hochladen ab | Anzeige, Journal bleibt, Wiederholen |
| R4 | `rsync -a` setzt Zeiten nach dem Schließen | Wiederübertragung bei jedem Lauf | Doku: Sync-Setups verwenden |
| R5 | Erweiterungsumsetzung in `RootedBackend`/`mount_proxy` ändert Standardpfade für Windows-Laufwerke | Verhaltensänderung bestehender Laufwerke | nur `finish_stage` umgesetzt; Bestandsharnesses; `drive_task_windows_drive_ignores_pending_attributes` |
| R6 | vold-/Watchdog bei blockiertem `onGetSize` (belegt NR N11) | system_server-Neustart | `HandlerThread` je Datei, `onGetSize` ohne Netz |
| R7 | Veröffentlichung nach `onRelease` wird beendet (NR N14) | späte Veröffentlichung | Journal, `DocsCommitWorker`, Liste ausstehender Speicherungen |
| R8 | Gleichzeitiges Lesen und Schreiben derselben Datei auf Android | Leser sieht Stand vor Veröffentlichung | `drive_task_android_reader_sees_dirty_entry` |
| R9 | Geräte-Laufwerk belegt einen Buchstaben, den der Nutzer später braucht | `net use` scheitert | Hinweis, Buchstabe wählbar, Schalter |
| R10 | Share-/Exec-Erreichbarkeit ab Anmeldung (K7) | ungewollte Erreichbarkeit | Entscheidung Hauptagent vor DRV-M6 |
| R11 | Fehlerbehebung DRV-M2 ändert Erwartungen bestehender Windows-Harnesses | Suite rot | betroffene Harnesses im selben Meilenstein angepasst |

## G — Verbleibende Lücken (keine Entscheidung hängt allein daran)

| Nr. | Lücke | Wirkung | Quellenhinweis |
|---|---|---|---|
| G1 | Fedora-42-Dateiliste von `systemd` (`fusectl`-Unit) | `--force` auf Fedora | `src.fedoraproject.org/rpms/systemd` |
| G2 | Aufrufstellen von `realpath` in VS Code; Obsidian nicht quelloffen | A3 | VS Code `src/vs/base/node/extpath.ts` |
| G3 | LibreOffice-Speicherweg | `setattr`-Regeln für LibreOffice | `sal/osl/unx/file.cxx` |
| G4 | rsync-Exitcode bei `FERROR_XFER` | nur Doku | `rsync/errcode.h` |
| G5 | Dokany und WinFsp nebeneinander auf `windows-2025` | Voraussetzung T2 | Runner-Lauf |
| G6 | Vollständige Liste der Win32-Codes in `dokan/ntstatus.i` (`v2.3.1.1000`) | Inhalt von `DOKANY_MAPPED_WIN32` | Tag `v2.3.1.1000`, Umsetzung liest sie aus |
| G7 | Fristen für Raumgeräte (`RoomRoster`, `RoomLeft`) | nur Anzeigegeschwindigkeit | `N/share/core/signal_*.rs` |
| G8 | Explorer-Dialogtext für 1117 und 1232; deutsche Systemtexte | Meldungstext | Windows-Laufzeitprobe in `drive-dokany-regression` |

## Prüfprotokoll Nachrecherche

Je Nachrecherche-Antwort und je Revalidierungszeile: Ergebnis laut NR und die Änderung in dieser Fassung.

| Punkt | Ergebnis NR | Änderung im Plan |
|---|---|---|
| N1 | belegt: `no_interrupt` nach `ENOSYS`; zugestellte Anfrage auch mit `SIGKILL` nicht beendbar | C2 „Abbruch“ als Befund statt Annahme; R2 belegt |
| N2 | `abort` für Nutzer schreibbar; `fusectl` auf Ubuntu/Debian/Arch eingehängt | C2 `--force` bestätigt; Verbindungsnummer aus `mountinfo` (A5); G1 Fedora |
| N3 | `GetFinalPathNameByHandleW` scheitert auf WinFsp-`DefineDosDevice`; Node nur `.native`/`promises` | C1 Grund 3 präzisiert; A3; neues Messsignal auf Dokany |
| N4 | `max_pages_limit` 256 bestätigt | C2 ohne Annahme |
| N5 | `fusermount3` verlangt Schreibrecht | C2 `0500`/`0700` bestätigt |
| N6 | `cp -a`/`rsync -a` scheitern an `ENOTSUP` | C2 `setattr`-Regeln über `finish_stage` entschieden; DRV-M4 Dateien und Signale; R4 |
| N7 | GTK, Nautilus, KDE erkennen das Ersetzen | DRV-M5 bestätigt; Hilfsdatei im selben Ordner |
| N8 | Mounts unter `$HOME/` sichtbar, Name = letzter Pfadteil; Doppelanzeige mit Lesezeichen | C2 GTK-Lesezeichen nur außerhalb der GLib-Regel |
| N9 | fuse3 auf Desktop-Standardinstallationen vorhanden | C5 Voreinstellung „ein“ gestützt |
| N10 | Anbieter vor `Application.onCreate` belegt | C3 Kaltstart; Signal `drive_task_android_provider_init_before_application` |
| N11 | vold-/Watchdog-Mechanismus belegt | C3, R6 belegt |
| N12 | keine Negativaussage | A11 mit Signal |
| N13 | `flock` auf Android mit Linux-Semantik | C4/DRV-M2 bestätigt |
| N14 | Anbieterprozess keine FGS-Ausnahme; `getForegroundInfo()` nötig | C3 Veröffentlichung korrigiert; Signal `drive_task_android_commit_worker_has_foreground_info` |
| N15 | DocumentsUI zeigt `EXTRA_ERROR`/`EXTRA_LOADING` | C3 bestätigt |
| N16 | Explorer sieht WSL-FUSE nur unter `/mnt/wsl` | C2 WSL-Hinweis; A7 |
| N17 | Miniaturbilder nur global abschaltbar | C6 Grenzen bestätigt |
| N18 | kein stabiles WinFsp 2.2; CVE-2026-93689 = Dienstverweigerung | C1 Tabelle und T1 präzisiert |
| N19 | SMB meldet alle Garantien; Ausschlussgrund unbelegt | C3 SMB-Begründung korrigiert; B2 Nr. 8 |
| N20 | `mode &= MODE_READ_WRITE`; kein `SETATTR` | C3 „wa“ dauerhaft abgelehnt |
| N21 | XDG-Autostart in GNOME, KDE, Xfce, Cinnamon, MATE | C2 bestätigt |
| N22 | Sync-Planung hängt an `autostart::is_enabled()`; Share-Host startet immer | C2 Daemon-Rolle gestrichen; Randfall `disabled_by_system` mit Startgrund-Merker; K7 offen |
| N23 | Präsenz 300 s (LAN 150 s), Server-Fenster 60 s | C5 Zeitwert gestrichen, ereignisgesteuertes Schließen |
| N24 | `ubuntu-24.04` mit unprivilegiertem FUSE; Dokany-MSI auf `windows-2025` | DRV-M10 Runner fest; G5 |
| N25 | keine Lizenzübersicht | C8 Verweis auf Integrationsmeilenstein |
| N26 | `Config` nicht erschöpfend, `n_threads: Option<usize>` | C2 Schreibweise korrigiert |
| N27 | `ERROR_HOST_UNREACHABLE` abgebildet; Bestandsfehler `ERROR_INVALID_DATA` → `STATUS_ACCESS_DENIED` | C4/C5 Fehlerabbildung; Defektbehebung in DRV-M2; A12; G6, G8 |
| C1-1 bis C1-4, C1-6 bis C1-13, C1-15, C1-16, C1-18 | bestätigt (C1-7 mit Ergänzung „Dienstverweigerung“) | C1 unverändert bzw. präzisiert |
| C1-5 | nicht belegbar (Ablaufdatum) | A4 |
| C1-14 | nicht belegbar | A1 |
| C1-17 | nicht belegbar | A2 |
| C1-19 | teilweise bestätigt | A3, Messsignal |
| C2-1 bis C2-6, C2-8, C2-9, C2-12 bis C2-15, C2-17, C2-18, C2-20, C2-22, C2-23 | bestätigt (C2-1: `init` ist `&mut self`) | C2 unverändert bzw. präzisiert |
| C2-7 | widerlegt (Schreibweise) | C2 Threading korrigiert |
| C2-10 | bestätigt mit Folgerung zu `<n>` | A5 |
| C2-11 | bestätigt mit Einschränkung (Admin-Sysctls) | C2 Zeitgrenzen entschieden; R3; Signal |
| C2-16 | teilweise widerlegt | C2 Einhängeort-Regeln korrigiert |
| C2-19 | nicht belegbar | A8 |
| C2-21 | widerlegt | C2 Wiederherstellung korrigiert |
| C2-24 | nicht belegbar | A9 |
| C3-1, C3-2, C3-5 bis C3-10, C3-12 | bestätigt (C3-6 Grund präzisiert, C3-12 mit Auflage) | C3 präzisiert |
| C3-3 | nicht belegbar | A10 |
| C3-4 | nicht belegbar (Begründung) | C3 SMB-Begründung ersetzt |
| C3-11 | nicht belegbar | A11 |
| C3-13 | nicht belegbar | gestrichen: Pipes werden nicht genutzt, die Aussage trägt nichts |
| C4-1 | bestätigt, Zeilen `:114-116` | B2 Nr. 2 korrigiert |
| C4-2 | Bestandsfehler | DRV-M2 |
| C4-3 | Grundlage geändert | C2 `setattr` |
| C5-1, C5-2, C5-5, C5-6 | bestätigt | C5 präzisiert (Grenze 32 ist se-Code) |
| C5-3 | Aufzählung widerlegt, Kernaussage bestätigt | C5 korrigiert |
| C5-4 | widerlegt | C5 Schließen ohne Zeitwert |
| C6-1, C6-2 | bestätigt | gegenstandslos: eigene Spool-Kryptografie ersetzt durch S3-Format (C6) |
| C6-3 | bestätigt | C6 Grenzen |
| C8-1, C8-2 | bestätigt | C8 unverändert |
| C8-3 | widerlegt (Voraussetzung) | C8 Lizenzhinweise über Integrationsmeilenstein |
| NR Abschnitt 3, Punkte 1–16 | Folgen | 1 → C2 `setattr`; 2 → C2 Zeitgrenzen; 3 → C2 Threading; 4 → C2 Einhängeort; 5 → C2 Wiederherstellung, K7; 6 → C2 WSL; 7 → C2 Sichtbarkeit; 8 → C3 SMB; 9 → C3 Modi; 10 → C3 Veröffentlichung; 11 → R6; 12 → C5 Schließen; 13 → C4/C5 Fehlerabbildung, DRV-M2; 14 → C8; 15 → C1 T1; 16 → DRV-M10 |
