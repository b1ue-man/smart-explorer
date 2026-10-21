# S3 — Cryptomator-Tresore

Stand: 2026-10-17, Lesestand `da2646d2`, **Endfassung nach der Nachrecherche**. Teil des Plans `plan.md` (Ergebnisse E8, E9). Diese Datei enthält Schleifenstand, Bestand (B), Entscheidungen (C), Meilensteine (D), Schnittstellen zu S2 und S5 (E), Risiken und unbestätigte Annahmen (F), verbleibende Lücken (G) und am Ende das „Prüfprotokoll Nachrecherche“. Nichts davon ist gebaut oder geprüft (AGENTS.md: keine lokalen Builds/Tests); „umgesetzt“ heißt erst: Code geschrieben und durch die eine Remote-Task-Suite des Stapels und das Release bestätigt. Stellen, die die Nachrecherche geändert hat, tragen den Vermerk „(NR: G<n>)“ bzw. „(NR: Nr. <n>)“ mit der Nummer aus `nachrecherche-tresore.md`.

Quellenkürzel dieser Datei:

| Kürzel | Datei |
|---|---|
| [BEST] | `docs/lesungen/2026-10-17-tresore-bestand.md` (Bestandslesung S3, Tabelle „Klartext-Ablageorte“ K01–K43) |
| [FMT] | `docs/refs/cryptomator-format-sync-2026-10-17.md` (Format 8, Crates, Sync/Konflikte) |
| [ERK] | `docs/CRYPTOMATOR_VAULTS_ERKUNDUNG.md` (Erkundung 2026-10-04) |
| [EIN] | `docs/refs/cryptomator-einhaengen-revalidierung-2026-10-17.md` (Einhängewege) |
| [NR] | `nachrecherche-tresore.md` (Antworten G1–G26, Stände, Revalidierung Nr. 1–59, Folgen) |
| [S2] | `strang-laufwerke.md`, Endfassung im Stand 16:40 (C2 Einhängeort, C4 Traits `SpoolStorage`/`PathSealer`, C6 Klartextgrenze, C7 Oberflächen, DRV-M7, DRV-M8, E1 mit den Bestätigungsfragen P1–P3) |
| [S2-NR] | `nachrecherche-laufwerke.md` (nur gezielt: N5, N8, Miniaturbild-Abschnitt) |
| [S5] | `strang-desktop.md` |
| [ARCH] | `docs/ARCHITEKTUR.md` |
| [OBF] | `docs/oberflaechen/INDEX.md`, `docs/oberflaechen/einstellungen.md` |
| Code | selbst gelesene Stellen, `Datei:Zeile` unter `native/src/`, wenn nicht anders genannt |

## Schleifenstand

**Ziel:** Cryptomator-Tresore in se öffnen und bearbeiten (Tresorsystem), an jedem Ort, den se erreicht; ein eigener Tresor-Sync-Job ohne Ordnereinrichtung, der die neuesten Änderungen zuverlässig abgleicht und bei Änderungen auf beiden Seiten nichts automatisch auflöst, sondern nach dem Öffnen des Tresors einen Vergleichsmodus anbietet. Quelle: E8, E9 in `plan.md` A.

**Kriterien (Status bezogen auf den Planungsteil):**

| Kriterium | Herkunft | Nachweis | Status |
|---|---|---|---|
| K1 Tresore öffnen (entsperren), lesen, bearbeiten, sperren | E8 „open and edit vaults“ | Entscheidung mit Beleg + Abnahmesignal | erfüllt (Planung): C1, C2, C3 mit Belegen [FMT], [NR] G4, G5, G19; Abnahmesignale VLT-M1, VLT-M2, VLT-M4 |
| K2 Kompatibel mit Format 8 (beide Cipher-Kombinationen, lange Namen, Verzeichnis-IDs) | E8 | Testvektoren + Gegentest | erfüllt (Planung): CryptoLib-Vektoren und 640 SIV-Fälle belegt ([NR] Nr. 11, G3); Namen und Verzeichnis-Hash ohne festen Vektor, abgedeckt durch den zweiseitigen CryptoFS-Gegentest (Stufe `vault_interop`) |
| K3 Tresore auf jedem Backend | AGENTS.md, Vorbefund | Backend-Matrix + Test | erfüllt (Planung): Schicht über jedem `vfs::Backend` (C2); Testserver für SFTP, FTP/FTPS, SMB, WebDAV, Agent vorhanden ([NR] G18); Google Drive ohne Testzugang wird als „nicht live geprüft“ berichtet (AGENTS.md: fehlende Zugangsdaten sind kein Planungs- oder Release-Hindernis) |
| K4 Tresor-Sync-Job ohne Ordnereinrichtung, zuverlässig, neueste Änderungen | E9 | Entwurf + Test | erfüllt (Planung): C5 mit Engine-Belegen [NR] G8, G9, G10, G11; Reihenfolge über Anwendungsphasen (C5.4 R4) |
| K5 Keine automatische Auflösung; Vergleichsmodus nach Öffnen | E9 | Bedienweg + Test | erfüllt (Planung): C5 R1–R9, C6 (gültige Chiffratnamen bei „Beide behalten“, [NR] G26) |
| K6 Kein dauerhafter Klartext außerhalb des Tresors | Vorbefund „Klartextgrenze“ | Entwurf + Test | erfüllt (Planung): Jede Ablage von se ist entschieden (C4, K01–K48); S2 hat den Linux-Einhängeort für Tresor-Laufwerke außerhalb `$HOME` übernommen (`$XDG_RUNTIME_DIR/smart-explorer/Tresore/<Name>`, tmpfs geprüft, [S2] C2) und versiegelt Spool und Journal mit den S3-Umsetzungen ([S2] C4, C6). Zwischenspeicher des Betriebssystems liegen außerhalb von se (K45) und sind als Annahmen A3, A6, A7, A14 mit Wirkung benannt |
| K7 Alle Oberflächen | Nutzerregel | Inventur | erfüllt (Planung): C7 (22 Fähigkeiten, Ausnahmen mit Grund); Teildokument in VLT-M13 |
| K8 Lizenz und Sicherheit der Krypto-Bausteine | Vorbefund | Recherche | erfüllt (Planung): Crates und RustSec geprüft ([NR] G6); Wortliste nicht frei lizenziert ([NR] G1) → Wiederherstellungsschlüssel nicht im Stapel (C1.10); CI-Harness ohne Weitergabe ([NR] G21) |
| K9 Bestehendes Verhalten bleibt erhalten; gesperrt/unerreichbar = „nicht verfügbar“, nie leere Quelle mit Löschungen | AGENTS.md | Entwurf + Test | erfüllt (Planung): C2.7, C5 R7, C5.8 (eigener Jobordner schützt ältere Versionen, [NR] G10), C5.4 (Standardphase unverändert) |
| K10 `core/` plattformneutral, Plattformcode unter `os/`, neue Rust-Dateien < 500 Zeilen | AGENTS.md | Dateiliste je Meilenstein | erfüllt (Planung): D |
| K11 Keine willkürlichen Grenzen | Auftrag | Grund je Grenze | erfüllt (Planung): C1.3 (64 KiB als eigene Grenze mit Grund, [NR] G5), C1.6, C5.6 |
| K12 Keine UI-Platzhalter | Auftrag | Bedienwege | erfüllt (Planung): nicht angebotene Funktionen erscheinen nur als Informationstext ohne Bedienelement (C1.10) |
| K13 Plattformprüfung je Mechanismus | Auftrag | Tabelle C8.3 | **offen**: Für jeden Mechanismus ist ein Weg gewählt; unbelegt bleiben (1) die Zustellung von `WM_WTSSESSION_CHANGE` an das verborgene Fenster des Workers (Windows), (2) ob GNOME Shell und KDE Plasma `LockedHint` setzen (Linux) ([NR] G13, G14). Beide betreffen nur „Sperren bei Bildschirmsperre“ auf dem Desktop; Folgen in F |

**Aktiver Stand:** Endfassung von C1–C8, 14 Meilensteine (VLT-M1 bis VLT-M14), Verträge E abgestimmt mit der S2-Endfassung (16:40) einschließlich der Bestätigungen P1–P3 (Abschnitt „Bestätigungen an S2“). Alle in [NR] Abschnitt 4 genannten geänderten Grundlagen sind eingearbeitet (Prüfprotokoll am Ende).

**Nächster Schritt:** Nutzerfrage OE1 und Kenntnisnahme OE4/OE5 durch den Hauptagenten; Umsetzung ab VLT-M1 im Integrationsplan des Stapels. Rückkehrpunkt bei einer späteren Änderung der S2-Endfassung: E (S2-V1 bis S2-V10), C3.6 und C4 (K01–K03, K45).

**Schleifenstatus:** offen – Grund: K13 (zwei unbelegte Plattformannahmen zur Bildschirmsperre: A1 Windows-Zustellung an das verborgene Fenster, A2 `LockedHint` in GNOME/KDE; Nachweis erst in der Suite bzw. nicht automatisierbar). K1 bis K12 sind für den Planungsteil belegt erfüllt.

**Offene Entscheidungen (Hauptagent oder Nutzer):**

| Nr | Frage | Wer | Stand in diesem Plan |
|---|---|---|---|
| OE1 | Wiederherstellungsschlüssel von Cryptomator: Soll se (a) die Wortliste zur Laufzeit von der offiziellen Quelle laden (fester SHA-256-Pin, Tag `1.19.3`), (b) eine schriftliche Erlaubnis von Skymatic einholen und die Liste mitliefern, oder (c) die Funktion dauerhaft weglassen? Die Rechtslage (Herkunft „Oxford Top 5000“, GPLv3, „aggregation“) ist ungeklärt ([NR] G1) | Nutzer | Weg ohne Rechtsrisiko für den Stapel: weglassen mit ehrlicher Anzeige (C1.10); Folgearbeit in `docs/TODO.md` (VLT-M13) |
| OE2 | Linux-Einhängeort der Tresor-Laufwerke außerhalb des Standardumfangs von `localsearch`/`baloo` | Hauptagent mit S2 | erledigt: S2 übernimmt `$XDG_RUNTIME_DIR/smart-explorer/Tresore/<Name>` ([S2] C2, E1 S2-V7) |
| OE3 | Feldname der Laufwerksquelle | Hauptagent mit S2 | erledigt: S2 nennt das Feld `MountSource::Vault { site_id }` ([S2] C4, E1 S2-V1) |
| OE4 | Neue Anwendungsphasen in `bisync/os/shared/apply_reporting.rs` (C5.4 R4) ändern einen Engine-Pfad aller Jobarten (Standard: eine Phase = Bestand) | Hauptagent (Kenntnisnahme, Integrationsreihenfolge) | VLT-M7 |
| OE5 | Sieben neue Pakete für den Bildvergleich auf dem Desktop (`zune-jpeg`, `zune-core`, `gif`, `weezl`, `color_quant`, `image-webp`, `quick-error`, [NR] G20) gehen in den Integrationsmeilenstein „Lizenzhinweise aller Rust-Crates“ | Hauptagent | C6.3, VLT-M10 |

## B — Bestand dieses Strangs (nur Entscheidungsrelevantes)

B1. **Backend-Vertrag.** `Backend` ist blockierend, 74 Methoden, 10 ohne Standardrumpf; `BackendExtensions` 24 Methoden mit konservativem Standard ([BEST] 2.1.1, 2.1.2). Schichten, die Pfade umschreiben, dürfen die Erweiterungen des inneren Backends nicht unverändert herausgeben (`vfs/core/extensions.rs:25-28`); `list_dir_tolerant` liefert `VfsListing` mit Vollständigkeitsangabe (`vfs/core/extensions.rs:73-81`).

B2. **Schichtmuster.** `CachingBackend` übernimmt `scheme`, `root_display`, `state_identity`, `namespace_identity`; `ZipBackend` schreibgeschützt ([BEST] 2.1.3).

B3. **Locator.** `EndpointSpec` mit `Local`, `Saved`, `Drive`, `Peer`; unbekanntes Schema → „Nicht unterstütztes Pfadprotokoll“ (`connect/core/location.rs:51, 57`); Android-Grammatik `mobile/core/location.rs` (`LocKind` mit 10 Varianten, [NR] Nr. 27); CLI über `connect::resolve_endpoint` (`cli/target.rs:99-147`); `vfs::backend_for` behandelt jede Wurzel ohne `sftp://`, `ftp://`, `ftps://`, `smb://` als lokalen Pfad (`vfs/core/dispatch.rs:11-23`).

B4. **Scheme.** Sieben Varianten (`vfs/core/scheme.rs:3-11`); vollständige Zuordnungen `app/core/error_report.rs:55-61`, `daemon/os/shared/ipc_protocol.rs:31-37, 45-51` ([NR] Nr. 26).

B5. **Daemon-IPC und Fehler.** Token je Anfrage; `OpenShare` wird nach `IpcResponse::OpenOk` über `serve_backend` zum Agent-Protokoll-Kanal, der Client baut `AgentBackend::from_streams` — beides nicht Share-spezifisch ([NR] G12). Fehler laufen nur als Text: Agent-Frames `Frame::Err(String)`, IPC `IpcResponse::Err { msg }` (`daemon/os/shared/ipc_protocol.rs:433-435`); der Client erkennt den Umschlag `SE_TRANSPORT_V1 <code> <text>` (`agent_proto/core/transport_error.rs:5-33`, aufgerufen in `agent/core/agent_error.rs:14-21`). Dateigrößen: `ipc_protocol.rs` 475, `ipc_host.rs` 479 Zeilen.

B6. **Geheimnisspeicher.** `creds::set_secret`, `get_secret_checked`, `delete_secret_checked` (B6 alt; [NR] Nr. 29).

B7. **Klartext-Ablageorte.** K01–K43 ([BEST] §3). Zusätzlich: Android `<cache>/open/<editId>/` mit Register `mobile/edits.json` und Ortsliste `mobile/recent.json` (`mobile/os/shared/edits_store.rs:17-21, 68`, `mobile/os/shared/places.rs:248-254`, [NR] G24); Android-Reiterpfade und Android-Fehlerprotokoll nur im Speicher ([NR] G24); Desktop-`recent.txt` hält die letzten 10 Orte (`app/core/prefs_tabs.rs:9-24`); Desktop-App-Fehlerprotokoll nur im Speicher (`app/core/status_errors.rs:36`). Materialisierende Wege in `open-temp` genau drei: Öffnen (`app/os/shared/remote_open.rs:93`), „Für andere Programme bereitstellen“ unter Windows (`app/os/shared/clipboard_provide.rs:43`), Zwischenablage mit langen relativen Pfaden (`app/os/shared/clipboard_materialize.rs:9-21`); Ziehen und virtuelle Zwischenablage streamen ([NR] G25). Share-Freigabewurzeln werden über `share/os/shared/storage_roots.rs:148-161` (`excluded()`, `is_private`) begrenzt.

B8. **Sync.** `FileLevel` (`strict`) Standard; Planung über `plan_pair_spelled` für Vollscan und Vorschau; der inkrementelle Weg (nur Einwegspiegel mit `DeletePolicy::Mirror`) umgeht `plan_pair`; Checkpoints haben keinen eigenen Planungsweg ([NR] G8). Anwendung: erst Ordner-Anlegen (`directory_pass`), dann Dateiaktionen parallel in Gruppen; gruppiert werden nur gleiche oder ineinanderliegende Pfade; getrennte Gruppen laufen gleichzeitig; kein Abhängigkeitsmechanismus (`apply_reporting.rs:120-135`, `apply_groups.rs:1-58`, [NR] G8). `MtimeSize` nutzt angebotene Serverhashes (`snapshot_hash.rs:78-88`, `compare.rs:63-85`); Echtzeit über `remote_watch` mit `connect::resolve_endpoint` und `vfs::change_signal` ([NR] G9). `target_managed` gehört nur zum Einwegspiegel-Index ([NR] G11). Jobs liegen in `<SYNC>/jobs/<id>.conf`; die Desktop-GUI lädt sie streng (`load`, ein ungültiger Job → leere Liste), Daemon, CLI, MCP und Android tolerant (`load_report`) (`syncjobs/os/shared/persistence.rs:196-235`, [NR] G10). Merge arbeitet auf Bytes (`merge_recorded.rs:15-18`), `keep_both` bildet Geschwisternamen aus dem gespeicherten Namen (`merge_keep_both.rs:142`) ([NR] G26).

B9. **Oberflächen.** CLI ohne Sync-Aktionen; Android mit Konflikt-, Varianten-, Merge-, Versionsbildschirmen; MCP-Werkzeug `se` mit Argument `stdin` (`cli/mcp/cli_tool.rs:38-43, 124-128`); Android-Hauptreiter `Files`, `Sync`, `Share`, `More` (`ui/AppNav.kt:14`); `HostMonitor.kt:105-118` registriert `ACTION_SCREEN_ON`/`OFF` dynamisch und ruft `requestPush()`; `HostStateArgs` (`api/SyncApi.kt:265-273`) hat die Felder `powerSave`, `metered`, `wifi`, `charging`, `foreground`, `deferScheduling`, `storageAccess`, aber kein Bildschirmfeld; Rust-Seite `mobile/os/shared/sys.rs:16, 60`.

B10. **Laufwerke (S2-Endfassung 16:40).** Windows: Dokany 2.3.1.1000, Tresore als festes lokales Laufwerk mit Buchstaben (Netzwerk- und Wechselmodus verworfen, [S2] C1, C6). Linux: `fuser =0.18.0`; Laufwerke ohne Tresor unter `~/SmartExplorer/<Name>`, **Tresor-Laufwerke unter `$XDG_RUNTIME_DIR/smart-explorer/Tresore/<Name>`** (Host prüft `TMPFS_MAGIC`, Sticky-Bit, außerhalb eines Mounts `0500`, vor dem Einhängen `0700`; ohne `XDG_RUNTIME_DIR` kein Rückfall auf die Platte; GTK-Lesezeichen und KDE-Ort) ([S2] C2). Android: `DocumentsProvider`. Grenze: Traits `SpoolStorage` (`N/mount/core/spool_storage.rs`, `SpoolFile { read_at, write_at, set_len, len, sync_data }`) und `PathSealer` (`N/mount/core/path_sealer.rs`, `seal`, `unseal`) definiert S2, die versiegelten Umsetzungen liefert S3; der Host erhält nur den Spool-Schlüssel über `Vault::SpoolKey`; Journal und `stages.json` behalten Dateinamen und Format, nur ihre Pfadfelder werden versiegelt; `MountRecovery::RequiresUnlock`, `PrepareLock`, `MountTerminated`; Registry `<AD>/mounts/registry-2.json` mit `MountSource::Vault { site_id }` nach Ausgliederung aus `types.rs` ([S2] C4, C6, DRV-M7, E1). Einbinden über `se drive mount vault://<SITE>` ([S2] C7).

B11. **Krypto-Crates.** Neue RustCrypto-Generation im Lock vollständig vorhanden; neu nur `aes-siv 0.8.0`, `aes-kw 0.3.1`; deren Abhängigkeitsbereiche erfüllt der Lock, Standardmerkmale bringen keine neuen Pakete ([NR] G6). `icu_normalizer 2.2` ist **bereits direkte Abhängigkeit** mit `compiled_data` (`native/Cargo.toml:144`), im Bestand genutzt in `bisync/core/keys.rs:168` ([NR] G7). `uuid 1.23.3` mit Merkmal `v4` über `moka` ([NR] Nr. 7). Stabiles Rust 1.99.1, Windows-Release-Image Rust 1.99.0; MSRV aller geplanten Crates ≤ 1.89 ([NR] G6). RustSec ohne Eintrag für die geplanten Versionen ([NR] G6, Abschnitt 2). `transfer::available_memory()` liest `MemAvailable` (Linux, Android) bzw. `GlobalMemoryStatusEx` (Windows) (`transfer/os/unix.rs:28-37`, `transfer/os/windows.rs:38-49`, [NR] G17). `windows-sys`-Merkmale `Win32_System_Power` und `Win32_UI_WindowsAndMessaging` aktiv, `Win32_System_RemoteDesktop` nicht ([NR] G13). Linux-`zbus` mit `Connection::system()` und logind-Inhibitor bereits genutzt in `keep_awake/os/linux_os.rs:16-18, 87-113` ([NR] G14).

B12. **Namensgleichheit „vault“.** Tests `mount_vault_task_*`, Module `vault_task_tests`, `daemon/os/shared/vault_task_tree.rs`; ein libtest-Filter `vault_task_` trifft sie mit.

B13. **Formatfakten** ([FMT] A, [NR] G2–G5, G19, G22): Öffnungsreihenfolge JWT → `kid` → Schlüssel → Signatur → Format; NFC; AES-SIV-Schlüssel = MAC ‖ Enc (bestätigt [NR] G3); CRC-Anteil = `crc32(masterkey).to_le_bytes()[0..2]` ([NR] G2); scrypt-Speicher `128·r·(N + 3)` ([NR] G5); `.bkup`-Suffix Großbuchstaben ([NR] G5); CryptoFS schreibt leere Dateien nur als Header und behält beim Überschreiben den Header ([NR] G4); Dateiinhalt nicht an Pfad oder Namen gebunden; CryptoFS legt beim Öffnen `vault.cryptomator.<HEX>.bkup` an ([NR] G5); se-Stufennamen sind für CryptoFS unsichtbar, die Health-Reparatur `OrphanContentDir` verschiebt sie aber nach `LOST+FOUND` ([NR] G19); 17 Health-Befundtypen mit Reparaturen ([NR] G22); Cryptomator Desktop 1.19.3 enthält die #355-Korrektur nicht ([NR] Abschnitt 2).

B14. **Lizenzen** ([FMT] B, [NR] G1, G21): CryptoLib/CryptoFS AGPLv3; Desktop GPLv3; Wortliste `4096words_en.txt` aus einer „Oxford Top 5000“-Auswahl ohne eigene Lizenzangabe; `siv-mode` MIT; OxCrypt MPL-2.0; cryptomator-rs Apache-2.0.

B15. **Einhängen und Vorschaubilder** ([EIN], [NR] G15, [S2-NR]): `whenTerminated`-Lehre; GIO stuft einen eigenen FUSE-Typ als lokal ein → GNOME Files erzeugt Klartext-Miniaturbilder (`show-image-thumbnails = 'local-only'`); `localsearch` und `baloo` indizieren standardmäßig `$HOME`; Explorer schreibt `Thumbs.db` in Netzwerkordner; Cyberduck blendet `.c9s` und Konfliktkopien mit Sonderzeichen aus ([NR] G23).

## C — Entscheidungen

### C1 Format-Umsetzung

**Betrachtete Lösungen:**

| Weg | Inhalt | Bewertung |
|---|---|---|
| F-a | Eigene Rust-Umsetzung als reines Formatmodul `cryptomator/core/` | K2, K3, K8, K10 erfüllt; Aufwand geschätzt 2 500–3 000 Zeilen in 14 Dateien plus Tests |
| F-b | OxCrypt `oxcrypt-core` übernehmen und anpassen | `std::fs`-E/A muss umgebaut werden; MPL-2.0 je Datei; Mängel (`kid`-Pfad, `rename_file`, keine scrypt-Grenze); dritte Crate-Generation; Edition 2024/MSRV 1.95; Nutzerregel E7 „if you must do substantial changes, build new“ |
| F-c | cryptomator-rs übernehmen und erweitern | nur SIV_GCM/HS256, `libc`-Fehler, fehlerhaftes `rename`; Neubau kaum teurer |
| F-d | CryptoLib/CryptoFS (Java) | AGPLv3 für die ausgelieferte App; JVM; keine se-Backends; automatische Konfliktauflösung ohne Abschaltung ([NR] Nr. 59) |

**Wahl: F-a.** Gründe unverändert; OxCrypt, cryptomator-rs und CryptoFS nur als Lesereferenz, kein Kopieren von MPL-Code.

**C1.1 Bausteine (geändert, NR: G6, G7, Nr. 5, Nr. 7).** Neue RustCrypto-Generation; die ältere brächte **sieben** neue Pakete (`aes-siv 0.7`, `aes-kw 0.2`, `scrypt 0.11`, `cmac 0.7`, `dbl 0.3`, `pbkdf2 0.12`, `salsa20 0.10`), die gewählte zwei. Direkte Abhängigkeiten in `native/Cargo.toml`:

| Abhängigkeit | Paket und Version | Zweck | Im Lock |
|---|---|---|---|
| `aes-siv` | `aes-siv 0.8.0`, `default-features = false`, `features = ["alloc"]` | Namen, Verzeichnis-ID-Hash | neu |
| `aes-kw` | `aes-kw 0.3.1`, `default-features = false` (Merkmal `oid` nicht nötig) | RFC 3394: Masterkey-Wicklung und Wicklung der Spool-Schlüssel (C3.6) | neu |
| `aes-gcm-011` | `aes-gcm 0.11.1` (package-Umbenennung) | SIV_GCM | ja |
| `aes-09` | `aes 0.9` (Lock `0.9.1`) | CTRMAC-Blockchiffre | ja |
| `ctr-010` | `ctr 0.10.1` | SIV_CTRMAC | ja |
| `hmac-013` | `hmac 0.13.0` | CTRMAC-MAC, `versionMac`, JWT HS256/384/512 | ja |
| `sha2-011` | `sha2 0.11.0` | SHA-256/384/512, HKDF | ja |
| `sha1-011` | `sha1 0.11.0` | Verzeichnis-Hash, Kurznamen | ja |
| `hkdf-013` | `hkdf 0.13.0` | Zweckschlüssel (C3.6) | ja |
| `scrypt` | `scrypt 0.12.0` (`Params::new(log_n, r, p)` mit drei Parametern, [NR] Nr. 17) | KEK | ja |
| `data-encoding` | `data-encoding 2.11` | Base32 | ja |
| `subtle` | `subtle 2.6.1` | zeitkonstante Vergleiche | ja |
| `icu_normalizer` | bereits direkt (`native/Cargo.toml:144`) | NFC, Aufruf wie `bisync/core/keys.rs:168` | Wiederverwendung |

`crc32fast` (Prüfsumme des Wiederherstellungsschlüssels) wird im Stapel nicht aufgenommen, weil die Funktion entfällt (C1.10).

Verworfen: ältere Generation (sieben neue Pakete); `jsonwebtoken` (JWT-Prüfung direkt mit `hmac-013`/`sha2-011`); `ring` (kein SIV, kein KW, kein CTR, [NR] Nr. 9); `uuid`: Merkmal `v4` ist über `moka` aktiv ([NR] Nr. 7), aber ob `moka` für jedes Ziel (Android) gebaut wird, ist ungeprüft; die Verzeichnis-ID wird deshalb weiter aus 16 Zufallsbytes (`getrandom 0.2`) als UUIDv4-Text gebildet, damit das Formatmodul nicht vom Merkmal eines fremden Crates abhängt. MSRV: alle Crates ≤ 1.89, CI 1.99.1, Release-Image 1.99.0 ([NR] G6).

**C1.2 Prüfvektoren und Gegenprüfung (geändert, NR: G2, G3).** (1) Die acht CryptoLib-2.3.0-Zeichenketten (sieben Tests, [NR] Nr. 11) als Unit-Tests; der Parser akzeptiert jeden Wert im Feld `version`. (2) AES-SIV-512: aus `cryptomator/siv-mode` `src/test/resources/testcases.txt` (MIT, Commit `b8e0974…`) genau die 640 Fälle mit 512-Bit-Schlüssel und 0 bzw. 1 Zusatzdatenfeld — die beiden Aufrufformen von Cryptomator (Verzeichnis-Hash ohne Zusatzdaten, Name mit Eltern-ID; 160 der 1-Feld-Fälle mit leerem Feld = Wurzel); Datei `native/src/cryptomator/core/testdata/siv_mode_512.txt` mit MIT-Hinweis und Herkunft; Schlüssel `macKey ‖ ctrKey`. (3) Zweiseitiger Gegentest mit CryptoFS 2.11.0 (Stufe `vault_interop`). (4) Wiederherstellungsschlüssel: nicht im Stapel (C1.10); der belegte Vektor aus [NR] G2 wird in `docs/TODO.md` für die Folgearbeit festgehalten.

**C1.3 scrypt-Grenzen (geändert, NR: G5, G17).** (a) N ist Zweierpotenz > 1; (b) CryptoLib 2.3.0: N ≤ 2²⁰, r ≤ 64, **Speicher `128·r·(N + 3)` ≤ 1 GiB + 3 072 Byte**; (c) Laufzeitprüfung mit `transfer::available_memory()` (vorhanden): Desktop „benötigt ≤ verfügbar“; Android „benötigt ≤ `availMem` − `threshold`“ aus `ActivityManager.MemoryInfo`, übergeben als Argumente `memAvail`, `memThreshold` von `vault.unlock`, weil eine Zuweisung bis `MemAvailable` den Low-Memory-Killer auslösen kann ([NR] G17). Lesegrenze der Schlüsseldatei 64 KiB: **eigene Grenze von se** (CryptoLib hat keine, [NR] G5); Grund: jede gültige Datei ist kleiner als 1 KiB, die Grenze verhindert nur unbegrenztes Lesen einer präparierten Datei von einem fremden Speicher; Fehlertext „Schlüsseldatei größer als 64 KiB – kein gültiger Tresor“.

**C1.4 Schlüsselquelle (`kid`).** Nur `masterkeyfile:masterkey.cryptomator`, immer fester Dateiname in der Tresorwurzel (GHSA-5phc-5pfx-hr52 bestätigt, [NR] Nr. 18); `hub+…` → „Cryptomator-Hub-Tresore werden nicht unterstützt“; jeder `kid`-Wert außer diesen beiden → „Nicht unterstützte Schlüsselquelle“.

**C1.5 Formatversionen.** Nur Format 8; Format ≤ 7 → Hinweis, einmal in Cryptomator zu öffnen (Cryptomator migriert).

**C1.6 Anlegen.** Nur `SIV_GCM`, `shorteningThreshold` 36–220, Standard 220 (bestätigt, [NR] Nr. 19); Lesen und Schreiben beider Kombinationen.

**C1.7 Leere Dateien.** Lesen beider Formen, Schreiben nur Header (bestätigt, [NR] G4).

**C1.8 Konfliktkopien beim Auflisten.** Keine automatische Auflösung; fremde Konfliktkopien mit Anzeigename nach CryptoFS-Muster und Zuordnung Anzeigename → tatsächlicher Chiffratname; Bereinigen nur auf Befehl in „Tresor prüfen“ (C6.6); Ordnerkopien nie mit derselben ID umbenennen.

**C1.9 Nicht entschlüsselbare Einträge.** Unvollständige Auflistung über `list_dir_tolerant`; Baumoperationen halten an.

**C1.10 Wiederherstellungsschlüssel (neu entschieden, NR: G1, G2).**

| Weg | Inhalt | Bewertung |
|---|---|---|
| W-a | Weglassen mit ehrlicher Anzeige | kein Rechtsrisiko; Nutzer erzeugen und nutzen den Schlüssel in Cryptomator Desktop |
| W-b | Wortliste zur Laufzeit von `raw.githubusercontent.com/cryptomator/cryptomator/1.19.3/src/main/resources/i18n/4096words_en.txt` laden, SHA-256-Pin, Ablage in `<AD>` | se verteilt die Liste nicht; ob ein MIT-Programm, das eine GPLv3-Datei zur Laufzeit liest, ein „combined work“ bildet, ist laut GPL-FAQ eine Gerichtsfrage; Netz nötig |
| W-c | Liste mit Erlaubnis von Skymatic mitliefern | Erlaubnis liegt nicht vor |
| W-d | Liste mitliefern als GPLv3-Datei („aggregation“) | Rechtsfrage ungeklärt; `include_str!` gilt nach GPL-FAQ als „combined“ |

**Wahl W-a** (Weg ohne Rechtsrisiko für den Stapel). Anzeige (Informationstext, kein Bedienelement): Desktop Eigenschaften → Seite „Wiederherstellung“, Android Tresor-Einstellungen, CLI `se vault show` Feld `recovery_key_supported: false` mit `recovery_key_note`; Text: „Wiederherstellungsschlüssel von Cryptomator werden in se nicht angezeigt und nicht angenommen, weil die Wortliste nicht frei lizenziert ist. Zum Anzeigen oder Zurücksetzen Cryptomator Desktop verwenden; der Tresor bleibt dafür voll kompatibel.“ Annahme mit Wirkung: Die Rechtslage von W-b und W-d ist ungeklärt; würde W-b ohne Klärung umgesetzt, trüge das Projekt ein nicht bewertetes Urheberrechtsrisiko. Nutzerfrage OE1. Folgearbeit: Kodierung (44 Wörter ↔ 66 Byte, CRC Little-Endian) und Vektor [NR] G2 stehen in `docs/TODO.md` (Kennung `VAULT-RECOVERY-KEY`) und werden erst nach der Nutzerentscheidung gebaut.

Quellen C1: [FMT] A, B; [NR] G1–G7, G17, Nr. 1–19; B11, B13, B14.

### C2 Tresor als Schicht, Identität, Locator, Fähigkeiten

**Schicht:** L-a `VaultBackend` über einem inneren `BackendHandle` (gewählt); verworfen L-b (nur im Laufwerks-Engine: kein Durchsuchen in CLI/MCP/Android-App) und L-c (`std::fs` wie OxCrypt: K3 verfehlt).

**Locator:** E-a `vault://<site-id>/<Klartextpfad>` mit Tresorliste (gewählt); verworfen E-b eingebetteter innerer Locator (Maskierung ändert Literalpfade, Vertrag `connect/core/location.rs:1-2`) und E-c neues `creds::Protocol` (Felder passen nicht).

**C2.1 Tresorort (Site) und Tresorliste.** Unverändert: Site = (innerer Endpunkt-Locator, Tresorwurzel), Site-ID 32 Hex; Persistenz `<AD>/vaults/<site-id>.conf` mit `id`, `label`, `location`, `vault_id` (= `jti`), `kind` (`original`/`replica`), `replica_of`, `seeding`, `auto_lock_idle_min`, `lock_on_sleep`, `lock_on_screen_lock`, `lock_on_background_min`, `remember_password`, `created_ms`; doppelter Ort abgewiesen.

**C2.2 Locator-Grammatik.** `vault://<site-id>/<Pfad>` literal; Eingabe auch mit eindeutiger Bezeichnung oder ID-Präfix ≥ 4, gespeichert immer die ID. `vault-store://<site-id>` ohne Pfadteil.

**C2.3 Identität und Chiffratsicht (geändert, NR: G9).** `vault://`: `state_identity()` = `vault:` + innere Identität + `#` + Tresorwurzel; `namespace_identity()` entsprechend; `flow_key()` = innerer. `vault-store://<site>` wird von `connect::resolve_endpoint` **ohne eigene Hülle** aufgelöst: Ergebnis ist das Paar (inneres `BackendHandle`, Tresorwurzel), das die Auflösung des gespeicherten inneren Locators liefert. Damit reichen Serverhashes (`provides_content_hash`), Änderungssignale (`change_signal_mode`, `change_signal`), `change_root_id` und alle Erweiterungen unverändert durch, `remote_watch` arbeitet wie bei jedem Endpunkt, und Identität und Überlappungsprüfung sind die des inneren Orts. Verworfen: eigene `vault-store`-Hülle (müsste jede Methode einzeln durchreichen und dürfte nach B1 Erweiterungen nicht unverändert herausgeben).

**C2.4 Neue `Scheme`-Variante `Vault`.** Anzeige „Cryptomator-Tresor“, Wire-Wert `MountBackendScheme::Vault`, Android `LocKind::Vault` (`vault`); Peer-Sonderwege greifen für Tresore nicht (bestätigt, [NR] Nr. 25).

**C2.5 Fähigkeiten der Klartextsicht `vault://`** (nie großzügiger als der innere Speicher): `staged_write_capabilities`, `probe_staged_write_capabilities`, `mount_path_capabilities`, `concurrent_read_write`, `transfer_ceiling`, `parallelism`, `root_confinement`, `has_duplicate_file_names` = innere Werte; `rename_overwrites` = `false`; `rename_no_replace` für Langnamen nur bei innerem `create = true`, sonst `Unsupported`; `case_sensitive_paths` = `true`; IDs des inneren Chiffratobjekts für `item_id`, `open_read_id`, `remove_file_id`; `open_read_at` über Blockgrenzen; `read_size` aus Chiffratgröße; `server_copy_to_stage` über das innere Backend; `provides_content_hash`, `supports_changes`, `supports_search`, `supports_walk_tree`, `supports_bulk_tree`, `supports_walk_hashed`, `put_batch`, `get_batch` = `false`/`Unsupported`; `delete_disposition` = `Permanent`, `supports_recycle` = `false`; `supports_remove_tree` = `false`; `version_archive_policy` = `Provider`; `volume_identity` = `Ok(None)`; `confirm_namespace`, `sync_filesystem`, `finish_stage`, `replace_staged_reversible`, `discard_copy_stage` an das innere Backend mit Chiffratpfaden; `extensions` = eigene Umsetzung. (Alle Methodennamen bestätigt, [NR] Nr. 28.)

**C2.6 Schreiben, Stufen, Atomarität (geändert, NR: G19, Nr. 23).** Jede Schreiboperation erzeugt einen neuen Header und schreibt in eine Stufe des inneren Backends; Veröffentlichung über `promote_staged`, `promote_staged_no_replace`, `promote_copy_stage`. **Stufennamen-Abbildung für alle Formen aus `vfs/core/staging_names.rs:28-140`:** neue Funktion `split_stage_name(name) -> Option<StageParts { lead, base: Option<&str>, tail }>` in `vfs/core/staging_names.rs`; Formen mit Dateiteil (`<file>.se-<zweck>-<16 Hex>`, `.<file>.smart-explorer[-<hex>…].part`, `.<file>.smart-explorer[-<hex>…].move`, `<file>.se-agent-batch-<hex>-<hex>.part`, `<file>.se-upload-<pid>-<hex>.part`) werden auf `lead + S + tail` abgebildet, mit `S` = Chiffratname des Dateiteils (`<b64>.c9r`), wenn der Gesamtname ≤ 255 Byte bleibt, sonst `S` = Base64url(SHA-1(Chiffratname)) (28 Zeichen, wie `.c9s`); Formen ohne Dateiteil (`.se-replace-<16 Hex>`, `.se-agent-<zweck>-<hex>-<hex>-<hex>.spool`, `.se-private-<32 Hex>.tmp`) bleiben wörtlich. Die Abbildung ist nur vorwärts nötig (Auflisten blendet Stufen aus; Stufenbücher rechnen vom Klartextnamen vorwärts). Langnamen-Ersetzen: Stufe `contents.c9r.se-…` im `.c9s`-Ordner. Ordner anlegen: Inhaltsverzeichnis und `dirid.c9r` vor dem Knoten (Abweichung von CryptoFS, das den Knoten zuerst anlegt, [NR] Nr. 21; Grund: Beobachter sehen kein `MissingContentDir`). Bekannte Folge (NR: G19): Liegt eine se-Stufe in einem später verwaisten Inhaltsverzeichnis, verschiebt die Health-Reparatur von CryptoFS sie nach `LOST+FOUND`; „Tresor prüfen“ meldet liegengebliebene se-Stufen und bietet „entfernen“ an (C6.6).

**C2.7 Schutz bestehender Wege.** `vfs::is_remote_root`/`backend_for` lehnen `vault://` und `vault-store://` ab; allgemeine Sync-Jobs lehnen `vault://`-Seiten ab (Grund K6); `vault-store://` nur in Tresor-Jobs; Verbindung bearbeiten/entfernen nennt betroffene Tresororte; Tresor-Symlinks nie verfolgen.

### C3 Schlüssel und Sitzungen

**Wahl H-a (unverändert):** Sitzungsregister im Hintergrund-Worker (Desktop: Daemon-Prozess oder im GUI eingebetteter Worker; Android: App-Prozess); verworfen H-b (Passwort bei jedem CLI-Befehl) und H-c (Masterkey in zwei Prozessen).

**C3.1 Entsperren.** NFC → scrypt mit C1.3 → AES-KW → Signatur → Format. Fehlertexte: „Passwort falsch“, „Schlüsseldatei ungültig“, „Schlüsseldatei größer als 64 KiB – kein gültiger Tresor“, „Schlüsselableitung braucht <x> MiB, verfügbar <y> MiB“, „Tresorformat <n> nicht unterstützt“, „Cryptomator-Hub-Tresore werden nicht unterstützt“, „Nicht unterstützte Schlüsselquelle“, „Speicherort nicht erreichbar: <Ursache>“, „Kein Cryptomator-Tresor (vault.cryptomator fehlt)“.

**C3.2 Hauptschlüssel nur im Speicher; strukturierter Fehler `vault_locked` (geändert, NR: G12).** Sperren vernichtet die Schlüssel (`Zeroizing`); danach liefert jede offene `VaultBackend`-Instanz den Fehler `VaultLocked { site }` (eigener Typ als Nutzlast eines `io::Error` mit Art `PermissionDenied`; `try_exists` wertet nur `NotFound` als Abwesenheit, damit ist „gesperrt“ nie „leer“). Übertragung über Prozessgrenzen:
- Agent-Protokoll (vermitteltes Backend `Vault::Open`): neuer Umschlag nach dem Muster von `SE_TRANSPORT_V1` in der neuen Datei `agent_proto/core/vault_error.rs`: Server schreibt `SE_VAULT_V1 locked <site-id> <Text>`; `agent/core/agent_error.rs::agent_error` prüft ihn vor `parse_transport_error` und baut `VaultLocked` zurück. Ältere Klienten sehen den Text unverändert (alte Bedeutung „Fehler mit Meldung“).
- IPC-Antworten: `IpcResponse::Err { msg, #[serde(default, skip_serializing_if = "Option::is_none")] code: Option<String> }` mit Code `vault_locked` (ältere Stände lesen das Feld nicht).
- Bridge: `ApiError`-Code `vault_locked`.
- CLI: Meldung „Tresor <Bezeichnung> ist gesperrt: se vault unlock <id>“, Exit-Code 1 (wie jeder Befehlsfehler).

**C3.3 Sperren (geändert, NR: G16):**

| Auslöser | Desktop | Android | Standard | Grund |
|---|---|---|---|---|
| ausdrücklich (einzeln, alle) | ja | ja | – | – |
| Leerlauf ohne Tresoroperation, offenes Handle oder Laufwerksaktivität | ja | ja | aus, Dauer 30 min | gleich Cryptomator Desktop (`DEFAULT_AUTOLOCK_WHEN_IDLE = false`, 30 min, [NR] G16) |
| Ruhezustand/Standby | ja | – | an | eigene Entscheidung (Cryptomator Desktop hat diese Einstellung nicht): Speicherabbild und gestohlenes schlafendes Gerät |
| Bildschirmsperre | ja | Bildschirm aus | Desktop aus, Android an | Android gleich Cryptomator (`LOCK_ON_SCREEN_OFF = true`); Desktop: Cryptomator hat keine Entsprechung, Standard aus |
| App im Hintergrund | – | ja | **1 Minute** | gleich Cryptomator Android (`ONE_MINUTE`, [NR] G16); wählbar sofort, 1, 2, 5 oder 10 Minuten oder aus, wie Cryptomator `LockTimeout.kt` (`INSTANT`, 1, 2, 5, 10 Minuten, `NEVER`; [NR] G16) |
| Ende des Workers | ja | ja | immer | Schlüssel leben nur im Prozess |
| Ende eines Tresor-Laufwerks (`MountTerminated`) | ja | ja (Provider-Ende) | sperren nur, wenn außer diesem Laufwerk kein Verbraucher die Sitzung hält | Vorschlag S2 übernommen ([S2] E1 (5)); Verbraucher = lebende `Vault::Open`-Verbindung (GUI-Reiter, CLI-Befehl, Transfer), offene Vergleichssitzung, Rückfallkopie (K05), zweites Laufwerk desselben Orts |

Sperrteilnehmer mit Frist vor dem Vernichten (Laufwerke, Rückfallkopien, Vergleichssitzungen); ausdrückliches Sperren mit ungespeicherten Laufwerksänderungen fragt „Speichern und sperren“, „Sperren, Änderungen verschlüsselt aufbewahren“, „Abbrechen“; erzwungenes Sperren bewahrt sie versiegelt auf ([S2] C6 `RequiresUnlock`).

**C3.4 Plattformsignale (neu entschieden, NR: G13, G14, Nr. 35):**
- **Windows, Ruhezustand:** `PowerRegisterSuspendResumeNotification(DEVICE_NOTIFY_CALLBACK)` im Worker, Ereignis `PBT_APMSUSPEND` sperrt (Merkmale aktiv).
- **Windows, Bildschirmsperre:** Der Worker erzeugt in einem eigenen Thread ein **verborgenes Fenster der obersten Ebene** (`WS_POPUP`, nie angezeigt; kein `HWND_MESSAGE`, weil dessen Empfang unbelegt ist) mit Nachrichtenschleife und registriert es mit `WTSRegisterSessionNotification(NOTIFY_FOR_THIS_SESSION)`; `WM_WTSSESSION_CHANGE`/`WTS_SESSION_LOCK` sperrt. Neues Merkmal `Win32_System_RemoteDesktop` in `windows-sys` (kein neues Crate). Scheitert die Registrierung mit `RPC_S_INVALID_BINDING` (Remote-Desktop-Dienste noch nicht bereit), wiederholt der Thread nach dem Ereignis `Global\TermSrvReadyEvent`. Annahme mit Wirkung (F A1): Ein nie angezeigtes Fenster der obersten Ebene erhält die Benachrichtigung; trifft das nicht zu, sperrt die Bildschirmsperre unter Windows nicht; Ruhezustand, Leerlauf, ausdrückliches Sperren und Worker-Ende wirken weiter. Verworfen: GUI-Weiterleitung als einziger Weg (CLI- oder Daemon-Prozess ohne GUI sperrte nie bei Bildschirmsperre).
- **Linux, Ruhezustand:** Systembus, Signal `PrepareForSleep(true)` am Manager-Objekt; der Worker hält einen `delay`-Inhibitor (`login1.Manager.Inhibit("sleep", "Smart Explorer", "Tresore sperren", "delay")`, Muster `keep_awake/os/linux_os.rs:87-113`), sperrt beim Signal und gibt den Inhibitor danach frei; bei `PrepareForSleep(false)` erneut halten.
- **Linux, Bildschirmsperre:** Sitzungsobjekt auflösen (Eigenschaft `Id` an `/org/freedesktop/login1/session/auto`, dann `login1.Manager.GetSession(<Id>)`), dort `PropertiesChanged` für `LockedHint` (Wechsel auf `true` sperrt) und zusätzlich das Signal `Lock` beobachten; nicht `session/auto` abonnieren (dort kommen keine Signale an). Ohne grafische Sitzung (Worker ohne Anmeldesitzung) zeigt die Einstellung „auf diesem System nicht verfügbar“. Annahme mit Wirkung (F A2): GNOME Shell und KDE Plasma setzen `LockedHint`; sonst wirkt nur `loginctl lock-session`.
- **Android:** `HostMonitor` registriert `ACTION_SCREEN_OFF` bereits; `HostStateArgs` erhält das Feld `screenOn: Boolean` (Standard `true` für ältere Aufrufer), `sys.hostState` (`mobile/os/shared/sys.rs:60`) reicht es an das Register; Vordergrund kommt wie bisher über `foreground`. Keine eigene Methode `vault.hostEvent`.

**C3.5 „Passwort merken“.** Auf Wunsch je Ort, Standard aus (bestätigt, [NR] G16); Konto `smart-explorer:vault:<site-id>`; Nutzung nur beim Öffnen; CLI nur mit `--saved-password`.

**C3.6 Lokale Versiegelung und Spool-Schlüssel (geändert, NR: G26; abgestimmt mit [S2] C4, C6).** Zweckschlüssel per HKDF-SHA256 aus Enc ‖ MAC: `se-vault-local-store-v1` (Ortsdaten im Tresor, K13/K14), `se-vault-log-fields-v1` (Pfadfelder in Protokollen, K20/K21), `se-vault-spool-wrap-v1` (Wickelschlüssel der Spool-Schlüssel). Versiegelt wird immer im Tresor-Inhaltsformat (C1, Header + AES-256-GCM-Blöcke), kurze Texte als Base64url davon. Der Zweck „Merge-Eingaben“ entfällt, weil der Merge nur Chiffrat speichert (C6.4). **Laufwerke:** Je Laufwerk erzeugt S3 im Daemon einen zufälligen 32-Byte-Spool-Schlüssel, wickelt ihn per AES-KW mit dem Zweckschlüssel `spool-wrap` und legt die 40 Byte als `<spool-root>/<mount-id>/spool-key.wrapped` ab; nach einem Absturz entwickelt der Daemon ihn nach dem Entsperren wieder (über Passwortwechsel stabil, weil der Masterkey gleich bleibt). Der Laufwerkshost erhält nur den Spool-Schlüssel (`Vault::SpoolKey`), nie den Masterkey. Aus dem Spool-Schlüssel leitet S3 per HKDF-SHA256 zwei Schlüssel ab: Inhaltsschlüssel `se-mount-spool-content-v1` (für `SealedSpoolFile`) und Textschlüssel `se-mount-journal-v1` (für `JournalPathSealer`, Bestätigung P1); der Spool-Schlüssel selbst verschlüsselt nichts direkt. Android: Engine und Register liegen im App-Prozess, der Schlüssel wird ohne IPC übergeben. Prozesse ohne Schlüssel (GUI, CLI) versiegeln Protokollfelder über IPC `Seal`/`Unseal`.

**C3.7 Passwortwechsel.** Wie bisher, ohne Zeitpunkt ohne gültige Schlüsseldatei; `.bkup`-Suffix Großbuchstaben (bestätigt, [NR] G5); Hinweistext zum unveränderten Schlüssel.

**C3.8 Wiederherstellungsschlüssel.** Entfällt im Stapel (C1.10, OE1).

### C4 Klartextgrenze

**Wahl G-a (unverändert):** je Ablageort verschlüsselt, nur Speicher oder ausgeschlossen. Geänderte Zeilen sind mit „NR“ markiert.

| Nr | Ort | Regel für Tresorinhalte | Umsetzung |
|---|---|---|---|
| K01 | Laufwerks-Spool | verschlüsselt | S3-`SealedSpoolFile` (Tresor-Inhaltsformat, Inhaltsschlüssel aus dem Spool-Schlüssel) hinter dem S2-Trait `SpoolStorage`; S2-V2 |
| K02 | Clean-Cache | verschlüsselt; beim Sperren entfernt | S2-V10 |
| K03 | `journal.jsonl`, `stages.json` | Pfadfelder versiegelt (`remote_path`, `original_path`, `quarantine_path`, Pfade der Namenskonfliktabsichten, Stufenpfade); Dateinamen und Format sonst wie Bestand | S3-`JournalPathSealer` (Textschlüssel aus dem Spool-Schlüssel, P1) hinter dem S2-Trait `PathSealer`; S2-V3 |
| K04 | `private-runtime/` | unverändert | bestätigt: nur Dokany-DLL ([NR] G24) |
| K05 | `open-temp` (NR: G25) | (1) Öffnen: Öffnen-Regel C7.2, Rückfallkopie nur nach Zustimmung; (2) „Für andere Programme bereitstellen“ und (3) Zwischenablage mit langen relativen Pfaden: für Tresorquellen abgelehnt mit „Tresorinhalte werden nicht als Klartextkopie bereitgestellt: als Laufwerk einhängen oder in einen gewählten Ordner kopieren (Export)“; (4) `downloads.rs:138, 202` haben außerhalb von Tests keinen Aufrufer → keine Änderung | Prüfung in `app/os/shared/clipboard_provide.rs` und `clipboard_materialize.rs` vor dem Materialisieren |
| K06 | `session.pid` | unverändert | – |
| K07 | Wiederherstellungsmanifest | nur Rückfallkopien; Pfadfelder versiegelt | – |
| K08 | anonyme Kopierdatei `copy_transfer` | verschlüsselt mit Einmal-Schlüssel | wenn Quelle oder Ziel `VaultBackend` |
| K09 | `analysis-cache/` (NR: G25) | ausgeschlossen | Einhängepunkte von Tresor-Laufwerken in `share/os/shared/storage_roots.rs::excluded()` aufnehmen (Quelle: `list_mounts` mit `MountedAt`, von S2 geliefert, [S2] E1) |
| K10 | `host-trash` | ausgeschlossen | folgt aus K09 und `Permanent` |
| K11, K12 | Ordnerindex | ausgeschlossen | `vault://` und Tresor-Einhängepunkte nie indizieren |
| K13 | `favorites.txt`, `dir_sort.tsv` | global nur `vault://<site-id>/` | innere Pfade versiegelt in `<AD>/vaults/<site-id>.local` |
| K14 | Desktop `recent.txt`; Android `mobile/recent.json` (NR: G24) | global nur `vault://<site-id>/` | wie K13 |
| K15 | `connections.txt` | unverändert | – |
| K16, K17 | Geheimnisspeicher | gemerktes Passwort nur auf Wunsch | – |
| K18, K19, K22, K23, K25, K29, K30 | nicht berührt | keine Tresordaten (K22 Signalserver-Adresse, K23 Uplink-Anfrage, [NR] G24) | – |
| K20 | `transfer-logs/` | Pfadfelder versiegelt | Anzeige entsiegelt nur bei entsperrtem Tresor |
| K21 | `reclaim/actions.jsonl` | Pfadfelder versiegelt | wie K20 |
| K24 | `crash.log` | keine Klartextnamen in Panik-/Fehlertexten | `Result` statt Panik; Site-ID statt Pfad |
| K26 | `<AD>/mobile/` (NR: G24) | `edits.json` (K44) und `recent.json` (K14); Reiterzustände nur im Speicher | – |
| K27 | Mount-Registry | `registry-2.json` mit `MountSource::Vault { site_id }` ([S2] C4) | S2-V1 |
| K28 | Jobdateien (NR: G10) | Tresor-Jobs in eigenem Ordner `<SYNC>/vault-jobs/<id>.conf` mit `vault-store://`-Locatoren; allgemeine Jobs mit `vault://` abgewiesen | C5.8 |
| K31–K35 | Sync-Zustand, Journale, Job-Protokoll | nur Chiffratpfade | Anzeige entschlüsselt nur im Speicher |
| K36, K37 | Versionen | Tresor-Jobs `versions_location = appdata` (Chiffrat) | – |
| K38 | Merge-Eingaben (NR: G26) | nur Chiffrat | Vergleichsmodus übergibt Chiffratbytes an `merge_recorded_for_key`; `merge_inputs.rs` unverändert |
| K39 | Konfliktkopien | keine automatischen; „Beide behalten“ mit gültigem Chiffratnamen | C6.4 |
| K40 | Kopierstufen | Chiffratstufen | C2.6 |
| K41–K43 | `nested-mounts.json`, `catchup.last.json`, `legacy-baseline-jobs/` | nur Chiffratpfade bzw. Locatoren (bestätigt K41, K43; `CatchUpRecord.message` ungelesen, F A9) | – |
| K44 | Android `<cache>/open/<editId>/`, `edits.json` | ausgeschlossen | `fs.fetch`/`fs.open` lehnen `vault://` mit `vault_use_provider` ab; Öffnen über Provider-URI (S2-V5) |
| K45 | Zwischenspeicher fremder Programme/des Systems (NR: G15) | außerhalb von se; belegte Fälle und Gegenmaßnahmen: (a) Linux/GNOME Files erzeugt Klartext-Miniaturbilder unter `$XDG_CACHE_HOME/thumbnails/` für Dateien auf einem FUSE-Laufwerk eigenen Typs; S3 löscht beim Sperren eines Tresors und bei `MountTerminated` eines Tresor-Laufwerks alle Miniaturbilder in `normal/`, `large/`, `x-large/`, `xx-large/` und `fail/*/`, deren PNG-Textfeld `Thumb::URI` mit `file://` + URI-kodiertem Einhängepunkt + `/` beginnt (Annahme A3; Löschen ist kein sicheres Überschreiben); den Hinweis zu Indizierung und Miniaturbildern beim ersten Einhängen zeigt S2 ([S2] C6), unter GNOME mit der angebotenen, nicht automatischen Einstellung `org.gnome.nautilus.preferences show-image-thumbnails = 'never'`; (b) `localsearch`/`baloo`: Tresor-Laufwerke liegen außerhalb `$HOME` ([S2] C2; Annahme A6); (c) Windows: festes lokales Laufwerk (P2); `thumbcache_*.db` und Windows-Suche unbelegt (Annahmen A7, A14); entsteht dennoch `Thumbs.db` im Laufwerk, liegt sie verschlüsselt im Tresor und wird wie jede Datei synchronisiert (R11) | Hinweis in `docs/oberflaechen/tresore.md` |
| K46 | Speicher von se | nur Speicher, beim Sperren verworfen | – |
| K47 | Auslagerungs-/Ruhezustandsdatei | außerhalb von se | Sperren beim Ruhezustand standardmäßig an |
| K48 | IPC-Loopback | nur Speicher/Kern | Token |

**C4.1 Absturz-Wiederherstellung ohne Klartext.** Laufwerk: S2 `RequiresUnlock`, nach dem Entsperren meldet das Register an den Laufwerksverwalter, der `retry_pending_changes` ausführt ([S2] C6); Merge: Chiffrat-Eingaben im Bestandsformat, Wiederaufnahme ohne Schlüssel möglich, Anzeige nach Entsperren; Rückfallkopien: bis zum Entsperren behalten und dann angeboten; Sync-Job: nur Chiffrat.

### C5 Tresor-Sync-Job ohne Ordnereinrichtung

**C5.1 Ebene:** Chiffrat-Abgleich ohne Schlüssel (S-a, gewählt); verworfen Klartext-Abgleich (S-b: Schlüssel bei jedem Lauf, Klartextnamen in K31–K35).

**C5.2 Gegenstelle ohne Ordnerwahl:** Standard automatisch angelegte Kopie `<AD>/vault-replicas/<site-id>/` bzw. `<filesDir>/smart_explorer/vault-replicas/<site-id>/` (Z-a); wählbar vorhandener Ort derselben `jti` (Z-b) oder neuer leerer Ordner (Z-c).

**C5.3 Engine:** bisync mit `kind=vault` und Regelstufe nach der Planung; kein eigener Lauf.

**C5.4 Regeln (R4 neu entschieden, NR: G8):**

| Regel | Fall | Behandlung |
|---|---|---|
| R1 | Tresormetadaten | verschiedene `jti` → Lauf bricht ab; `vault.cryptomator`, `masterkey.cryptomator`, `*.bkup` beidseitig verschieden = Konflikt |
| R2 | `dirid.c9r` auf einer Seite gelöscht, Gegenseite ändert im selben Inhaltsverzeichnis | keine Aktion für dieses Inhaltsverzeichnis; alle Knotenlöschungen der löschenden Seite in diesem Lauf zurückgestellt (`deferred`) |
| R3 | `<enc>.c9r/dir.c9r` beidseitig neu, verschiedener Inhalt | Konflikt |
| R4 | Reihenfolge | **zwei Anwendungsphasen**: Phase 1 = Anlegen und Ändern aller Einträge, die keine Knotendatei sind (Einträge unter `d/`, einschließlich `dirid.c9r` und `contents.c9r`/`name.c9s`), plus Löschen von Knotendateien `dir.c9r`/`symlink.c9r`; Phase 2 = Anlegen und Ändern von Knotendateien plus Löschen von Einträgen unter `d/`. Phase 2 beginnt erst, wenn jede Aktion der Phase 1 bestätigt oder gescheitert ist; scheitert eine Aktion der Phase 1, entfallen die Phase-2-Aktionen desselben Ordners (Inhaltsverzeichnis bzw. Knoten) in diesem Lauf und bleiben für den nächsten Lauf geplant. Umsetzung: `PlanRules::phases(&PairPlan) -> Vec<Vec<usize>>` (Standard: eine Phase mit allen Aktionen = Bestand), ausgewertet in `bisync/os/shared/apply_reporting.rs`, das `apply_groups` je Phase nacheinander aufruft. Der vorhandene Ordner-Durchgang (`directory_pass`) legt leere Knotenordner `<enc>.c9r/` vor Phase 1 an; das ist derselbe Zwischenzustand, den CryptoFS beim Anlegen erzeugt ([NR] Nr. 21), und endet mit Phase 2 |
| R4a | Inkrementeller Einwegspiegel | für `kind=vault` gesperrt: `try_incremental_run` gibt für Tresor-Jobs sofort „nicht anwendbar“ zurück; jeder Lauf ist ein Vollscan über `plan_pair_spelled` mit Regelstufe |
| R5 | `.c9s` | Einträge einzeln; beidseitig geändertes `contents.c9r` = Konflikt |
| R6 | Sicherheitsstopp | vorhandene Schwellen auf Chiffratdateien |
| R7 | Seitenprüfung vor der Planung | `vault.cryptomator` mit Job-`jti` und `d/` auf jeder Seite, sonst „nicht verfügbar“; leere Kopie nur bei `seeding=true` |
| R8 | fremde Konfliktkopien | normale Dateien |
| R9 | se-Stufennamen | ausgeschlossen (`is_staging_name`) |
| R10 | Engine-Keep-Both | `KeepBothAtoB`/`KeepBothBtoA` können bei `strict` nicht entstehen; die Regelstufe wandelt sie dennoch in Konflikte um (Schutz gegen ungültige Chiffratnamen, [NR] G26) |
| R11 | `Thumbs.db` unter Windows (NR: G15) | gewöhnliche Datei im Tresor; keine Sonderbehandlung |
| R12 | leere Knotenordner (Suite-Lauf 2, `vault_task_interop_health_after_sync`: CryptoFS meldet `UnknownType`) | die Regelstufe legt keinen Knotenordner (`d/XX/YYYY…/<name>.c9r` oder `.c9s`) leer an: ein Knotenordner entsteht nur durch das Kopieren seiner Dateien (der Ordner-Durchgang der Engine kennt Ordner ohne Verlauf nur einseitig und würde ihn auf der löschenden Seite leer anlegen). Löscht der Lauf Dateien eines Knotenordners (nicht zurückgestellt nach R2), plant die Regelstufe das Entfernen des Ordners auf dieser Seite, außer der Lauf schreibt auf dieser Seite in denselben Knotenordner (Link durch gleichnamigen Ordner ersetzt); die Engine entfernt ihn nur, solange er leer ist (sonst bleibt er für den nächsten Lauf). Umsetzung `vault/core/sync_rules_dirs.rs` |

Verworfen für R4: (a) Kanten zwischen beliebigen Pfaden in `action_groups` (allgemeiner, ändert die Gruppenbildung aller Jobs); (b) R4 weglassen (Health-Reparatur `MissingContentDir` von CryptoFS während eines Laufs erzeugte ein zweites `dirid.c9r` und damit Konflikte).

**C5.5 Änderungserkennung (geändert, NR: G9).** `compare` Standard `MtimeSize`. Auf Drive (`md5Checksum`), WebDAV-Servern mit Hashangabe, Agent- und lokalen Wegen erkennt `MtimeSize` jede Chiffratänderung auch bei gleicher Größe und Sekunde, weil jede Änderung Bytes und damit den Hash ändert (neue Block-Nonce, [NR] G4); Voraussetzung ist die Auflösung C2.3 (innere Fähigkeiten durchgereicht). Auf SFTP v3 ohne Agent und FTP bleibt die Lücke gleicher Größe in derselben Sekunde; dafür bieten Editor und CLI `Checksum` und die vorhandene Vollprüfung des Ziels. Auslöser Standard `RealTime`: Push (Share `watch_v1`) bzw. Poll (WebDAV-ETag, Drive-Feed) über `remote_watch`; ohne Signal Abfrage alle `rt_poll_secs` (bei 0: 300 s) ([NR] G9).

**C5.6 Feste und einstellbare Felder.** Unverändert (Abgleich aller 51 Felder bestätigt, [NR] Nr. 42); zusätzlich fest: `delete_policy` für `direction = both` = `propagate`; für `a2b`/`b2a` wählbar `propagate`, `mirror`, `nodelete` (inkrementeller Weg nach R4a gesperrt).

**C5.7 Umbenennungen und Verzeichnis-ID-Dateien.** Unverändert.

**C5.8 Schutz älterer se-Versionen (neu entschieden, NR: G10).** Tresor-Jobs liegen in **`<SYNC>/vault-jobs/<id>.conf`** (gleiches Dateiformat wie `jobs/`, zusätzliche Schlüssel `kind=vault`, `vault_site_a`, `vault_site_b`), nicht in `<SYNC>/jobs/`. Ältere Versionen lesen diesen Ordner nie: Ihre Desktop-GUI lädt `jobs/` weiter vollständig, kein Tresor-Job läuft, keine Datei wird verändert. Neue Versionen laden `vault-jobs/` in **jedem** Aufrufer tolerant (ungültige Tresor-Jobs erscheinen als „fehlerhaft“ im Tresorbereich), damit ein defekter Tresor-Job die strenge GUI-Liste von `jobs/` nicht leert; `jobs/` behält seine Lade-Semantik je Aufrufer. Job-IDs sind über beide Ordner eindeutig (Prüfung beim Anlegen). `vault-store://` bleibt als Locator der Chiffratsicht (Identität, `remote_watch`). Verworfen: (a) Tresor-Jobs in `jobs/` (ältere GUI verliert die ganze Jobliste); (b) GUI vor dem Release auf `load_report` umstellen (erreicht ältere Versionen nicht).

**C5.9 Erstlauf (geändert, NR: G11).** Ohne `target_managed`. Kopie (`seeding=true`): erster Lauf als Vollscan ohne Baseline überträgt alle Einträge A → B (ohne Baseline gibt es keine Löschungen; Annahme F A10); nach einem Lauf ohne Fehler, ohne Konflikt und ohne Zurückstellung setzt der Läufer `seeding=false`. Zwei vorhandene Orte: Vollscan-Erstlauf der Engine; verschiedene Inhalte unter gleichem Chiffratnamen werden Konflikte.

### C6 Tresor-Vergleichsmodus

**Wahl V-a (unverändert):** Vorschau über das Chiffrat-Paar mit Baseline, Zuordnung zu Klartextpfaden nach dem Entsperren, Aktionen unter der Paarsperre; verworfen V-b (Baumvergleich ohne Baseline) und V-c (nur Konfliktdialog).

**C6.1 Ablauf.** Unverändert (Verzeichniskarten nur im Speicher).

**C6.2 Gruppen (geändert, NR: G22).** `conflict`, `changed_a`, `changed_b`, `new_a`, `new_b`, `deleted_a`, `deleted_b`, `deferred`, `structure`. Gruppe `structure` enthält alle Health-Befunde von CryptoFS 2.11.0 außer den drei gesunden (`HealthyDir`, `ValidShortenedFile`, `KnownType`) sowie vier eigene Befunde:

| Befund | se-Bezeichnung | Aktion in se |
|---|---|---|
| `DirIdCollision` | doppelte Verzeichnis-ID | gleiche Eltern und gleicher Name → „doppelten Eintrag entfernen“ (nur Knoten); sonst „Ordner trennen“ (Inhalt über die Tresorschicht in einen neuen Ordner „<Name> (getrennt)“ mit neuer ID kopieren, danach zweiten Knoten entfernen) |
| `EmptyDirFile`, `ObeseDirFile` | Ordnerknoten beschädigt | „Knoten entfernen“ (das Inhaltsverzeichnis wird dadurch verwaist und ist danach über `OrphanContentDir` einhängbar) |
| `LooseDirFile` | Knotendatei ohne Ordner | „entfernen“ (wie CryptoFS) |
| `MissingContentDir` | Ordner ohne Inhalt | „leeren Inhalt anlegen“ (wie CryptoFS); bei aktivem Tresor-Job mit dem Hinweis „kann ein Zwischenstand eines laufenden Syncs sein“ und nur bei freier Paarsperre |
| `MissingDirIdBackup` | `dirid.c9r` fehlt | „schreiben“ |
| `OrphanContentDir` | verwaistes Inhaltsverzeichnis | „unter LOST+FOUND einhängen“: Knoten `LOST+FOUND` (ID `recovery`, Name wie CryptoFS) in der Wurzel bei Bedarf anlegen, darin Knoten „Wiederhergestellt <erste 8 Zeichen der ID>“ mit der ID aus `dirid.c9r`; keine Einträge verschieben (CryptoFS verschiebt Einträge; der Ort `LOST+FOUND` ist derselbe) |
| `LongShortNamesMismatch` | Kurzname passt nicht | „Ordner umbenennen“ (wie CryptoFS) |
| `MissingLongName`, `NotDecodableLongName`, `ObeseNameFile` | langer Name verloren/beschädigt | nur Anzeige (CryptoFS hat keine Reparatur) |
| `TrailingBytesInNameFile` | überzählige Bytes in `name.c9s` | „`name.c9s` neu schreiben“ |
| `UnknownType` | Knoten ohne Typdatei | „Knoten entfernen“ nur bei freier Paarsperre und nach Bestätigung (kann Zwischenstand eines Syncs sein) |
| `AmbiguousType` | mehrdeutiger Knoten | nur Anzeige |
| eigen | nicht entschlüsselbarer Name | nur Anzeige |
| eigen | fremde Konfliktkopie | „kanonisch benennen“ |
| eigen | liegengebliebene se-Stufe | „entfernen“ |
| eigen | Schlüsseldatei beidseitig geändert (nur Vergleich) | „Fassung A/B behalten“ |

**C6.3 Inhaltsvergleich (geändert, NR: G20).** Text-Diff mit `similar`; Bilder Desktop über `image 0.25` mit `default-features = false, features = ["png", "jpeg", "gif", "webp", "bmp"]` (sieben neue Pakete, OE5, Lizenzhinweise im Integrationsmeilenstein); Android über Provider-Strom; Dateien, die weder gültiges UTF-8 noch eines der fünf Bildformate sind: Metadaten und „Öffnen“ je Fassung.

**C6.4 Aktionen (geändert, NR: G26):**

| Aktion | Wirkung auf Chiffrat |
|---|---|
| „Fassung A behalten“ / „Fassung B behalten“ | `resolve_checked` (Verlierer in Versionen) |
| „Beide behalten“ | eigene Abfolge unter der Paarsperre, ohne Engine-Keep-Both: (1) Klartextname „<Name> (Konflikt <JJJJ-MM-TT HHMM> <Bezeichnung der Seite>).<Erweiterung>“ im selben Ordner bilden und unter dessen Verzeichnis-ID verschlüsseln (bei Überlänge `.c9s`); Chiffratbytes des Verlierers auf seiner Seite unverändert unter diesem Namen anlegen (Stufe + `promote_staged_no_replace`); (2) `resolve_checked` mit der gewählten Fassung für den Originalnamen; (3) neue Vorschau und `apply_preview_action` für den neuen Namen (überträgt auf die Gegenseite und zeichnet die Baseline auf) |
| „Zusammenführen“ (Text) | Zeilenansicht im Speicher aus den entschlüsselten Fassungen; an `merge_recorded_for_key` gehen die **Chiffratbytes** beider Originale (ihre Signaturen sind die Konfliktsignaturen des Chiffrat-Paars) und als `MergeChoice::Write` das **verschlüsselte** Ergebnis (neuer Header); `merge_inputs` speichert damit nur Chiffrat; `RecordedMergeChoice::KeepBoth` wird für Tresor-Jobs nie verwendet |
| „Jetzt übertragen“ | `apply_preview_action`; ein Klartext-Ordnereintrag umfasst mehrere Chiffrataktionen, die in der Reihenfolge R4 nacheinander angewendet werden |
| „Löschung übernehmen“ / „Wiederherstellen“ | wie bisher |
| „Ordner zusammenführen“ / „Beide Ordner behalten“ | wie bisher (Namen neu verschlüsseln, Inhalt unverändert) |
| Strukturaktionen | Tabelle C6.2 |
| „Öffnen (A)“ / „Öffnen (B)“ | Öffnen-Regel C7.2 |
| „Überspringen“ | keine Wirkung |

Sammelaktionen „Alle Konflikte: A“, „Alle Konflikte: B“.

**C6.5 Anknüpfung.** Unverändert (Desktop `bisync_conflicts.rs`, `merge_ui.rs`, `bisync_merge.rs`; Android `ConflictVariantChoice`, `MergeScreen`, `NavRequest`, bestätigt [NR] Nr. 49).

**C6.6 „Tresor prüfen“.** Gruppe `structure` für einen einzelnen entsperrten Ort.

### C7 Oberflächen

**C7.1 Fähigkeiten × Oberflächen (geändert: T12/T13, Sperrsignale).** MCP = CLI-Spalte über `se`.

| Nr | Fähigkeit | CLI / MCP | Desktop | Android | Bridge | Daemon-IPC |
|---|---|---|---|---|---|---|
| T1 | auflisten mit Zustand | `se vault list` | Seitenleiste „Tresore“ | „Mehr → Tresore“, Dateien-Wurzeln | `vault.list` | `Vault::Status` |
| T2 | erkennen | `se vault detect` | Hinweisleiste | Hinweis in der Dateiliste | `vault.detect` | – |
| T3 | eintragen | `se vault add` | „Tresor hinzufügen…“ | „Tresor hinzufügen“ | `vault.add` | – |
| T4 | anlegen | `se vault create` | „Neuer Tresor…“ | „Neuer Tresor“ | `vault.create` | – |
| T5 | austragen | `se vault remove` | „Austragen…“ | „Austragen“ | `vault.remove` | `Vault::Lock` vorher |
| T6 | entsperren | `se vault unlock` | Entsperrdialog | Entsperrblatt | `vault.unlock` | `Vault::Unlock`, `Vault::UnlockSaved` |
| T7 | sperren | `se vault lock` | Kontextmenü, „Alle sperren“ | Schaltfläche, „Alle sperren“ | `vault.lock`, `vault.lockAll` | `Vault::Lock` |
| T8 | durchsuchen, lesen, schreiben | `vault://` in `ls`, `stat`, `cat`, `get`, `put`, `cp`, `mv`, `rm`, `mkdir`, `search` | Reiter | Dateien-Reiter | `fs.*` | `Vault::Open` |
| T9 | mit externer App öffnen | Ausnahme (Export über `cat`/`get`) | Öffnen-Regel C7.2 | Provider-URI | – | – |
| T10 | Passwort ändern | `se vault password` | Eigenschaften → Passwort | Tresor-Einstellungen | `vault.changePassword` | – |
| T11 | Passwort merken/vergessen | `--remember`, `se vault forget-password` | Kontrollkästchen, Eigenschaften | Schalter | `vault.unlock {remember}`, `vault.forgetPassword` | – |
| T12 | Wiederherstellungsschlüssel anzeigen | nicht angeboten (Lizenz, C1.10): `se vault show` Felder `recovery_key_supported`, `recovery_key_note` | Informationstext Eigenschaften → Wiederherstellung | Informationstext | `vault.show` Felder `recoveryKeySupported`, `recoveryKeyNote` | – |
| T13 | mit Wiederherstellungsschlüssel zurücksetzen | nicht angeboten (Lizenz) | wie T12 | wie T12 | wie T12 | – |
| T14 | Sperr-Einstellungen | `se vault settings` | Eigenschaften → Sperren | Tresor-Einstellungen | `vault.setSettings` | Register lädt neu |
| T15 | Tresor-Sync einrichten | `se vault sync add` | „Synchronisieren…“ | „Synchronisieren“ | `vault.syncCreate` | – |
| T16 | Tresor-Sync ausführen | `se vault sync run` | „Jetzt ausführen“ | Jobkarte | `sync.run` | vorhandene Jobsteuerung |
| T17 | bearbeiten, aktivieren, entfernen | `se sync edit/enable/disable/remove` | Job-Editor „Tresor-Sync“ | Job-Editor mit Tresor-Abschnitt | `sync.save`, `sync.setEnabled`, `sync.delete` | – |
| T18 | vergleichen | `se vault compare` | „Tresor vergleichen“ | `VaultCompareScreen` | `vault.compare` | `Vault::Compare` |
| T19 | auflösen | `se vault resolve` | Aktionen | Aktionen | `vault.resolve`, `vault.resolveAll` | `Vault::Resolve` |
| T20 | zusammenführen (Text) | `--action merge --merged-file DATEI` (Ausnahme) | Merge-Ansicht | Merge-Ansicht | `vault.compareText`, `vault.mergeApply` | `Vault::CompareText`, `Vault::MergeApply` |
| T21 | Tresor prüfen | `se vault check` | Eigenschaften → Prüfung | Tresor-Einstellungen → Prüfung | `vault.check` | `Vault::Check` |
| T22 | als Laufwerk/Ordner einhängen | `se drive mount vault://<SITE>` (S2-Befehl; Ziel über die gemeinsame Orts- und Auflösungsgrenze; kein `se vault mount`, kein `se drive add --vault`) | Kontextmenü „Als Laufwerk einhängen“ ruft den S2-Laufwerksweg mit dem Ort `vault://<site-id>` | Ausnahme: kein Laufwerk; Provider-Wurzel je entsperrtem Tresor, Schalter „Tresore“ unter „Ordner für Apps bereitstellen“ ([S2] C7, S2-V5) | `docs.*` (S2) | vorhandene Mount-Anfragen mit `MountSource::Vault { site_id }` |

Ausnahmen mit Grund: T9 CLI, T20 CLI, T22 Android, T12/T13 auf allen Oberflächen (Lizenz, nur Informationstext), Sperrauslöser „Bildschirm aus/Hintergrund“ nur Android, „Ruhezustand/Bildschirmsperre“ nur Desktop.

**C7.2 Öffnen-Regel (Desktop).** `mount_path_for("vault://<site>/<pfad>")` (S2, `Result<Option<PathBuf>, String>`) liefert einen Pfad → im Laufwerk öffnen; kein Pfad, Laufwerk verfügbar, „Beim Öffnen einhängen“ an (Standard an) → `ensure_mounted_for` (S2) und öffnen; Fehler oder keine Laufwerkslaufzeit → Dialog „Klartextkopie öffnen?“ (K05 (1)). Android: Provider-URI.

**C7.3 CLI `se vault` (geändert: ohne `recovery-key`).**

| Befehl | Optionen | JSON (Schlüssel) |
|---|---|---|
| `se vault list` | `--json` | `vaults[]`: `vault_id`, `sites[]`: `site`, `label`, `location`, `kind`, `state` (`locked`/`unlocked`/`unavailable`), `format`, `cipher`, `mounted`, `remember_password`, `auto_lock_idle_min`, `lock_on_sleep`, `lock_on_screen_lock`, `lock_on_background_min`, `sync_jobs[]` |
| `se vault show <SITE>` | `--json` | wie ein Eintrag `sites[]` plus `shortening_threshold` (nur entsperrt), `recovery_key_supported` (`false`), `recovery_key_note` |
| `se vault detect <LOCATION>` | `--json` | `is_vault`, `format`, `key_source`, `vault_id` |
| `se vault add <LOCATION>` | `--label`, `--json` | `site`, `label`, `vault_id` |
| `se vault create <LOCATION>` | `--label`, `--shortening-threshold 36..220`, `--password-stdin`, `--json` | `site`, `vault_id` |
| `se vault remove <SITE>` | – | – |
| `se vault unlock <SITE>` | `--password-stdin` \| `--saved-password`, `--remember true|false`, `--json` | `site`, `state` |
| `se vault lock` | `<SITE>` \| `--all`, `--keep-unsaved` | – |
| `se vault password <SITE>` | `--stdin` (Zeile 1 alt, Zeile 2 neu), `--json` | `site`, `backup` |
| `se vault forget-password <SITE>` | – | – |
| `se vault settings <SITE>` | `--label`, `--auto-lock-idle-min N|off`, `--lock-on-sleep`, `--lock-on-screen-lock`, `--lock-on-background-min 0|1|2|5|10|off` (0 = sofort), `--remember-password`, `--json` | wie `show` |
| `se vault check <SITE>` | `--json` | `items[]` (Gruppe `structure`) |
| `se vault sync add <SITE>` | `--with replica|<SITE>|<LOCATION>`, `--name`, Zeitplan- und Übertragungsoptionen für die Felder C5.6, `--json` | `job`, `site_a`, `site_b` |
| `se vault sync run <JOB>` | `--json` | `copied_a_to_b`, `copied_b_to_a`, `deleted`, `conflicts`, `deferred`, `errors` |
| `se vault compare <JOB>` | `--group …|all`, `--json` | `job`, `sides`, `items[]` mit `item`, `group`, `kind`, `path`, `a`/`b` (`exists`, `size`, `mtime_ms`, `location`), `actions[]` |
| `se vault resolve <JOB> <ITEM>` | `--action keep-a|keep-b|keep-both|merge|transfer|confirm-delete|restore|merge-folders|keep-folders-both|canonical-name|remove-duplicate|separate-folder|remove-node|create-content|write-dirid|attach-orphan|rename-shortened|rewrite-name-file|remove-stage`, `--merged-file PFAD`, `--json` | `item`, `result` |
| `se vault resolve <JOB> --all` | `--action keep-a|keep-b`, `--json` | `resolved`, `failed[]` |

Einhängen ist kein `se vault`-Befehl: `se drive mount vault://<SITE>` (S2, Bestandsunterbefehl; `<SITE>` als ID, ID-Präfix ≥ 4 oder eindeutige Bezeichnung, gespeichert wird die Site-ID; Status `requires-unlock:<n>` bzw. `failed:vault_locked`, [S2] DRV-M7); `se vault list --json` zeigt im Feld `mounted` den Einhängepfad aus `list_mounts`. Vervollständigung über `clap_complete` (`cli/mod.rs:228`), auch für `vault://`-Ziele von `se drive mount`. MCP: Katalogtext nennt `se vault` und `se drive mount vault://<SITE>`; `status` nennt Anzahl der Tresororte, entsperrte Orte, Tresor-Jobs mit Konflikten; Entsperren nur mit `--password-stdin` (Argument `stdin`) oder `--saved-password`.

**C7.4 Daemon-IPC (geändert, NR: G12).** Eine Variante `IpcRequest::Vault { token, req: VaultIpcRequest }`; Typen in `daemon/os/shared/ipc_protocol_vault.rs`: `Status`, `Unlock { site, password: SecretText, remember: Option<bool> }`, `UnlockSaved { site }`, `Lock { site: Option<String>, keep_unsaved: bool }`, `Open { site }` (Weg `serve_backend` + `AgentBackend::from_streams` wie `OpenShare`), `Seal { site, purpose, items }`, `Unseal { site, purpose, items }`, `SpoolKey { mount_id }` (beantwortet nur dem Laufwerkshost dieser Mount-ID mit seinem Sitzungstoken, Prüfung wie bei den `MountHost*`-Anfragen; Name aus [S2] E1), `Compare { job }`, `CompareText { job, item }`, `Resolve { job, item: Option<String>, action, all: bool }`, `MergeApply { job, item, content }`, `Check { site }`. Ereignisse `VaultEvent { site, state: Locked|Unlocked|Locking, reason }` über die UI-Ereigniszustellung. `IpcResponse::Err` erhält `code: Option<String>` (C3.2). Entfallen: `HostEvent` (C3.4).

**C7.5 Bridge (geändert).** `vault.list`, `vault.show`, `vault.detect`, `vault.add`, `vault.create` (Task), `vault.remove`, `vault.unlock` (Task; Argumente `site`, `password`, `remember`, `memAvail`, `memThreshold`), `vault.lock`, `vault.lockAll`, `vault.changePassword` (Task), `vault.forgetPassword`, `vault.setSettings`, `vault.syncCreate`, `vault.compare` (Task), `vault.compareText`, `vault.resolve` (Task), `vault.resolveAll` (Task), `vault.mergeApply` (Task), `vault.check` (Task) — 19 Methoden; `sys.hostState` erhält `screenOn`. Ereignis `vault` über `pollEvents`. Fehlercodes `vault_locked`, `vault_use_provider`, `vault_wrong_password`, `vault_unsupported_format`, `vault_hub`, `vault_memory`.

**C7.6 Desktop.** Seitenleiste „Tresore“; Kontextmenü Öffnen, Sperren, Als Laufwerk einhängen, Synchronisieren…, Vergleichen, Eigenschaften, Austragen…; Entsperrdialog ohne Wiederherstellungs-Link; Anlage-Assistent mit Passwort-Mindestlänge **8** (wie Cryptomator `DEFAULT_MIN_PW_LENGTH`, [NR] G16) und dem Hinweis „Einen Wiederherstellungsschlüssel erzeugt Cryptomator Desktop für diesen Tresor“; Eigenschaften-Seiten Allgemein, Sperren, Passwort, Wiederherstellung (Informationstext), Sync, Laufwerk (Einhängen/Aushängen über den S2-Laufwerksweg mit `vault://<site-id>`, Anzeige des Einhängepfads und von `requires-unlock`), Prüfung; Hinweisleiste bei `vault.cryptomator`.

**C7.7 Android.** „Mehr → Tresore“, Dateien-Wurzeln, Entsperrblatt, Jobkarte mit „Vergleichen“, Benachrichtigung bei Konflikten, `NavRequest.OpenVaultCompare(jobId)`; Mindestlänge 8 beim Anlegen; Freigabe an fremde Apps nicht als Laufwerk, sondern über die Provider-Wurzel entsperrter Tresore (Schalter „Tresore“ im S2-Abschnitt „Ordner für Apps bereitstellen“).

**C7.8 Doku.** `docs/oberflaechen/tresore.md` (neu) mit Matrix, Ausnahmen, K45-Hinweisen und Interop-Aussagen: Cyberduck/Mountain Duck zeigen `.c9s`-Einträge und Konfliktkopien mit Sonder- oder Leerzeichen nicht an ([NR] G23); Cryptomator Desktop 1.19.3 enthält die CryptoFS-Korrektur #355 nicht ([NR] Abschnitt 2); `INDEX.md`, `README.md`, `ARCHITEKTUR.md`, `api.md`, `docs/TODO.md` (Einträge `VAULT-RECOVERY-KEY` und Restlücken G). Zeile T22 verweist auf `docs/oberflaechen/laufwerke.md` (S2) mit dem Bedienweg `se drive mount vault://<SITE>`.

### C8 Laufwerk und Anzeige im se Desktop/Launcher

**C8.1 Laufwerk (Vertrag an S2, abgestimmt mit der S2-Endfassung).** Tresore sind die Quellart `MountSource::Vault { site_id }` der S2-Frontend-Grenze und werden über `se drive mount vault://<SITE>` bzw. die entsprechenden Desktop-Aktionen eingebunden. S2 definiert die Traits `SpoolStorage` und `PathSealer`; S3 liefert die versiegelten Umsetzungen `SealedSpoolFile` und `JournalPathSealer`, den Spool-Schlüssel (`Vault::SpoolKey`), das Backend, die Sperrereignisse, den Locator und die Sperrregel bei Laufwerksende (C3.3). Einzelverträge und Bestätigungen P1–P3 in E.

**C8.2 Anzeige im se Desktop/Launcher (Vertrag an S5).** Unverändert, Einzelverträge S5-V1 bis S5-V6 in E.

**C8.3 Plattformprüfung je Mechanismus (geändert):**

| Mechanismus | Windows | Linux | Android |
|---|---|---|---|
| AES in Hardware | AES-NI | AES-NI | `getauxval` `HWCAP_AES` (bestätigt, [NR] Nr. 54) |
| Speicherprüfung scrypt | `transfer::available_memory()` (`GlobalMemoryStatusEx`) | `transfer::available_memory()` (`MemAvailable`) | `availMem − threshold` aus `ActivityManager.MemoryInfo` über die Bridge |
| Sitzungshalter | Worker | Worker | App-Prozess |
| Sperren bei Ruhezustand | `PowerRegisterSuspendResumeNotification` | `PrepareForSleep` + `delay`-Inhibitor | entfällt |
| Sperren bei Bildschirmsperre | verborgenes Fenster + `WTSRegisterSessionNotification`, Merkmal `Win32_System_RemoteDesktop` (Annahme A1) | `LockedHint` am konkreten Sitzungsobjekt + Signal `Lock` (Annahme A2) | `ACTION_SCREEN_OFF` über `HostMonitor` → `sys.hostState.screenOn` |
| Sperren im Hintergrund | entfällt | entfällt | `sys.hostState.foreground`, Standard 1 min |
| Passwort merken | Credential Manager | `secrets-v1` | `secrets-v1` mit Keystore |
| inneres lokales Backend | NTFS | ext4/btrfs | `/storage/emulated`, App-Ordner |
| Kopie-Ablage | `<AD>\vault-replicas\` | `<AD>/vault-replicas/` | `<filesDir>/smart_explorer/vault-replicas/` |
| externes Öffnen | Laufwerk (S2) oder Zustimmungskopie | Laufwerk (S2) oder Zustimmungskopie | Provider-URI (S2) |
| Einhängepunkt | festes lokales Laufwerk mit Buchstaben (S2 C1/C6, P2) | `$XDG_RUNTIME_DIR/smart-explorer/Tresore/<Name>` (tmpfs geprüft, Sticky-Bit, `0500` außerhalb eines Mounts; [S2] C2) | Provider-Wurzel nur entsperrt |
| Miniaturbilder/Indexer | `thumbcache`/Windows-Suche unbelegt (A7, A14); Hinweis beim ersten Einhängen (S2) | GNOME-Miniaturbilder → S3 löscht nach `Thumb::URI` beim Sperren und Laufwerksende (A3); Indexer: Ort außerhalb `$HOME` (A6) | kein MediaStore-Scan ([S2] DRV-M8) |
| Bildvergleich | `image` | `image` | `BitmapFactory` aus Provider-Strom |

## D — Meilensteine in Umsetzungsreihenfolge

Gemeinsame Regeln: Testpräfix `vault_task_`; neue Rust-Dateien < 500 Zeilen und < 50 KiB (Schätzwerte in Klammern); `core/` ohne Plattformcode; Graph-Auffrischung nach AGENTS.md; Commits je Meilenstein; kein lokaler Build oder Test. Prüfungen sind **Stufen der einen Task-Suite des Stapels**; einen eigenen Workflow legt S3 nicht an (VLT-M14). Lizenzhinweise aller neuen Crates (`aes-siv`, `aes-kw`, die sieben Bild-Pakete) und der Testdaten (`siv-mode`, MIT) liefert der Integrationsmeilenstein „Lizenzhinweise aller Rust-Crates“ des Hauptagenten (`plan.md`, Abschnitt „Entscheidungen des Hauptagenten“, Kennung aus S4 `RUST-CRATE-LICENSES`).

### VLT-M1 Formatkern `cryptomator`

- **Ergebnis:** Format-8-Bytes erzeugen und prüfen (Schlüsseldatei, Konfiguration, Namen, Verzeichnis-IDs, Inhalte beider Kombinationen, Passwortwechsel-Bytes).
- **Neue Dateien:** `native/src/cryptomator/mod.rs` (40), `cryptomator/core/mod.rs` (40), `core/keys.rs` (130), `core/cipher_combo.rs` (60), `core/masterkey_file.rs` (240; Grenzen C1.3 mit `128·r·(N + 3)`, 64-KiB-Lesegrenze), `core/kdf.rs` (200; NFC über `icu_normalizer::ComposingNormalizerBorrowed::new_nfc()`), `core/vault_config.rs` (280), `core/names.rs` (260), `core/dir_ids.rs` (200), `core/content_header.rs` (220), `core/content_chunks.rs` (260), `core/content_stream.rs` (420), `core/vault_files.rs` (230; `.bkup`-Suffix Großbuchstaben); Tests `core/vectors_tests.rs`, `core/names_tests.rs`, `core/content_tests.rs`, `core/config_tests.rs` (je < 400); Testdaten `core/testdata/siv_mode_512.txt` (640 Fälle mit MIT-Hinweis). Entfällt gegenüber dem Entwurf: `core/recovery_key.rs` (C1.10).
- **Geänderte Dateien:** `native/src/lib.rs`, `native/Cargo.toml` (C1.1).
- **Vertrag:** wie Entwurf ohne `RecoveryKey`: `Masterkey::generate`, `MasterkeyFile::{parse, unlock, create, rewrap}`, `UnverifiedConfig::{parse, key_source, verify}`, `VaultConfig::create`, `encrypt_name`, `decrypt_name`, `content_dir`, `shortened_name`, `ContentCryptor`, `DecryptingReader`, `EncryptingWriter`, `backup_suffix`.
- **Abhängigkeiten:** keine.
- **Plattformprüfung:** Kompilierung Windows x86_64, Linux x86_64, Android arm64-v8a und x86_64 in der Suite.
- **Abnahmesignal (Stufe `vault_unit`):** `vault_task_cryptolib_header_gcm_vector`, `vault_task_cryptolib_header_ctrmac_vector`, `vault_task_cryptolib_chunk_gcm_vector`, `vault_task_cryptolib_chunk_ctrmac_vector`, `vault_task_cryptolib_file_gcm_vector`, `vault_task_cryptolib_masterkey_file_vector`, `vault_task_siv_mode_512_cases` (640 Fälle), `vault_task_scrypt_bounds_reference_formula` (Grenzfälle r = 8, N = 2²⁰ zulässig; r = 64, N = 2¹⁷ unzulässig; N = 3 unzulässig), `vault_task_masterkey_file_over_64k_rejected`, `vault_task_jwt_algs_and_tamper`, `vault_task_kid_only_fixed_masterkeyfile`, `vault_task_name_roundtrip_nfc_nfd`, `vault_task_dir_hash_shape`, `vault_task_shortened_name`, `vault_task_empty_file_both_forms`, `vault_task_size_formulas_boundaries`, `vault_task_backup_suffix_uppercase`.
- **Release/CI:** neue Pakete `aes-siv 0.8.0`, `aes-kw 0.3.1`; `cargo audit` deckt sie ab.

### VLT-M2 Tresorschicht `VaultBackend`

- **Ergebnis:** Entsperrter Tresor auf jedem Backend: durchsuchen, lesen (auch ab Offset), schreiben, umbenennen, löschen, Ordner bearbeiten; Konfliktkopien und nicht entschlüsselbare Einträge nach C1.8/C1.9.
- **Neue Dateien:** `vault/mod.rs` (60), `vault/core/mod.rs` (40), `vault/core/path_map.rs` (320; Stufennamen-Abbildung C2.6), `vault/core/listing.rs` (320), `vault/core/capabilities.rs` (150), `vault/os/mod.rs` (40), `vault/os/shared/backend.rs` (450), `vault/os/shared/backend_write.rs` (420), `vault/os/shared/backend_dirs.rs` (400), `vault/os/shared/backend_extensions.rs` (300), `vault/os/shared/dir_cache.rs` (200); Tests `backend_tests.rs`, `backend_write_tests.rs`, `backend_dirs_tests.rs`.
- **Geänderte Dateien:** `vfs/core/scheme.rs`, `app/core/error_report.rs`, `daemon/os/shared/ipc_protocol.rs` (`MountBackendScheme::Vault`), `vfs/core/staging_names.rs` (`split_stage_name`).
- **Vertrag:** `VaultBackend::new(inner, root, session)`; Fähigkeiten C2.5; Reihenfolgen C2.6; `VaultLocked` nach Sperren.
- **Abhängigkeiten:** VLT-M1.
- **Plattformprüfung:** inneres `LocalBackend` auf NTFS, ext4, Android-Speicher.
- **Abnahmesignal (Stufe `vault_unit`):** `vault_task_backend_read_write_roundtrip_gcm`, `…_ctrmac`, `vault_task_backend_open_read_at_chunk_boundaries`, `vault_task_backend_long_names_create_rename_replace`, `vault_task_backend_mkdir_order_content_before_node`, `vault_task_backend_rmdir_nonempty_refused`, `vault_task_backend_symlink_listed_never_followed`, `vault_task_backend_foreign_conflict_copy_alias_not_renamed`, `vault_task_backend_undecryptable_reported_incomplete`, `vault_task_backend_every_stage_form_mapped_invisible` (alle Formen aus `staging_names.rs:28-140`, Endung nie `.c9r`/`.c9s`/`.c9u`, Länge ≤ 255 Byte), `vault_task_backend_capabilities_never_exceed_inner`, `vault_task_backend_locked_fails_and_publishes_nothing`, `vault_task_backend_identity_distinct_from_inner_and_other_vaults`.
- **Release/CI:** keine.

### VLT-M3 Locator, Tresorliste, Auflösung, Schutz bestehender Wege

- **Ergebnis:** `vault://` und `vault-store://` sind gültige Orte; `vault-store://` löst ohne Hülle auf das innere Backend auf; bestehende Locatoren unverändert; Tresor-Locatoren nie lokal und nie in allgemeinen Jobs.
- **Neue Dateien:** `vault/core/locator.rs` (220), `vault/core/registry_types.rs` (260), `vault/os/shared/registry_store.rs` (300). Entfällt gegenüber dem Entwurf: `vault/os/shared/store_backend.rs` (C2.3).
- **Geänderte Dateien:** `connect/core/location.rs` (`EndpointSpec::Vault`, `EndpointSpec::VaultStore`), `connect/os/shared/resolution.rs` (Desktop: `daemon::open_vault_backend`; Android: Register; `VaultStore` → Auflösung des inneren Locators), `vfs/core/dispatch.rs`, `mobile/core/location.rs`, `mobile/os/shared/pool.rs`, `syncjobs/core/validation.rs`, `connect/os/shared/connection_edit_refs.rs`, `connect/os/shared/cleanup.rs` (Tresororte melden); vorab Bestandsaufnahme der literalen Präfixprüfungen mit Liste in der Commitbeschreibung.
- **Abhängigkeiten:** VLT-M2.
- **Abnahmesignal (Stufe `vault_unit`):** `vault_task_locator_parse_roundtrip`, `vault_task_locator_never_reaches_local_backend`, `vault_task_existing_locators_unchanged`, `vault_task_generic_sync_rejects_vault_endpoint`, `vault_task_vault_store_resolves_to_inner_with_hash_and_signal` (Fake-Backend mit `provides_content_hash = true` und Poll-Signal), `vault_task_vault_store_overlap_with_inner_rejected`, `vault_task_registry_roundtrip_unknown_keys_ignored`, `vault_task_registry_duplicate_location_rejected`, `vault_task_connection_edit_lists_vault_sites`, `vault_task_android_loc_vault_kind`.
- **Release/CI:** keine.

### VLT-M4 Sitzungen, IPC, Sperrauslöser, strukturierte Fehler, Passwort merken

- **Ergebnis:** Ein Entsperren gilt für alle Oberflächen des Geräts; alle Auslöser aus C3.3 sperren; `vault_locked` kommt über Agent-Protokoll, IPC und Bridge als Code an.
- **Neue Dateien:** `vault/core/lock_policy.rs` (200), `vault/core/sealed.rs` (250; Zwecke C3.6), `vault/os/shared/session.rs` (450; Register, Epoche, Verbraucher, Sperrteilnehmer), `vault/os/shared/unlock.rs` (300), `vault/os/shared/remember.rs` (120), `vault/os/windows.rs` (300; Ruhezustand, verborgenes Fenster mit WTS), `vault/os/linux_os.rs` (300; `PrepareForSleep` mit Inhibitor, `LockedHint`), `vault/os/android.rs` (60), `agent_proto/core/vault_error.rs` (80; Umschlag `SE_VAULT_V1`), `daemon/os/shared/ipc_protocol_vault.rs` (260), `daemon/os/shared/ipc_vault.rs` (400), `daemon/os/shared/ipc_vault_client.rs` (350).
- **Geänderte Dateien:** `daemon/os/shared/ipc_protocol.rs` (Variante `Vault`, Feld `code` in `Err`), `daemon/os/shared/ipc_host.rs` (ein Zweig), `daemon/os/shared/ipc_host_ui_events.rs` (`VaultEvent`), `agent/core/agent_error.rs` (Aufruf vor `parse_transport_error`), `mobile/os/shared/sys.rs` (`screenOn`), `mobile/os/shared/domains/mod.rs` (`set_foreground` an das Register), `native/Cargo.toml` (Merkmal `Win32_System_RemoteDesktop`), Kotlin `api/SyncApi.kt` (`HostStateArgs.screenOn`), `system/HostMonitor.kt` (Bildschirmzustand mitsenden).
- **Abhängigkeiten:** VLT-M3.
- **Plattformprüfung:** Windows (Rückruf, verborgenes Fenster, Credential Manager), Linux (zbus, Inhibitor, `LockedHint`), Android (Vordergrund, Bildschirm, Keystore, Speicherschwelle).
- **Abnahmesignal:** Stufe `vault_unit`: `vault_task_session_unlock_once_all_clients`, `vault_task_session_lock_zeroizes_and_fails_open_handles`, `vault_task_lock_policy_triggers_and_defaults` (Leerlauf aus/30 min, Hintergrund 1 min, Bildschirm aus an, Ruhezustand an), `vault_task_lock_participants_called_before_zeroize`, `vault_task_mount_end_locks_only_without_consumers`, `vault_task_vault_error_envelope_roundtrip_and_legacy_text`, `vault_task_ipc_err_code_backward_compatible` (alte Antwort ohne `code` lesbar), `vault_task_remember_password_account_and_forget`, `vault_task_ipc_vault_requests_token_required`, `vault_task_seal_unseal_purposes_distinct`. Stufe `vault_linux`: privater D-Bus-Systembus mit Test-logind-Dienst sendet `PrepareForSleep(true)` und setzt `LockedHint` → Tresor gesperrt, Inhibitor freigegeben. Stufe `vault_windows`: `vault_task_windows_wts_window_dispatch` (Nachricht `WM_WTSSESSION_CHANGE` an das registrierte Fenster gesendet → gesperrt; prüft Verdrahtung, nicht die Zustellung durch das Betriebssystem, Annahme A1).
- **Release/CI:** ein neues `windows-sys`-Merkmal, keine neuen Pakete.

### VLT-M5 Verwaltung: eintragen, erkennen, anlegen, austragen, Passwort, Einstellungen

- **Ergebnis:** Vorhandene Tresore eintragen, neue anlegen (Mindestlänge 8), Passwort ohne Zeitpunkt ohne Schlüsseldatei ändern, Sperrverhalten einstellen; Wiederherstellungsschlüssel als Informationstext (C1.10).
- **Neue Dateien:** `vault/os/shared/manage.rs` (420), `vault/os/shared/password.rs` (220). Entfällt: `vault/os/shared/wordlist.rs`.
- **Abhängigkeiten:** VLT-M4.
- **Abnahmesignal:** Stufe `vault_interop`: `vault_task_create_then_cryptofs_reads`, `vault_task_password_change_then_cryptofs_unlocks_new`; Stufe `vault_unit`: `vault_task_password_change_never_without_masterkey`, `vault_task_detect_hub_and_format7_messages`, `vault_task_remove_keeps_files_and_refuses_with_job`, `vault_task_create_min_password_length_8`, `vault_task_recovery_key_reported_unsupported`.
- **Release/CI:** keine Änderung an Artefakten.

### VLT-M6 Klartextgrenze in den Bestandsmodulen

- **Ergebnis:** Tabelle C4 umgesetzt.
- **Neue Dateien:** `vault/core/klartext_policy.rs` (200), `vault/os/shared/local_store.rs` (250), `vault/os/shared/ephemeral_spool.rs` (300), `app/os/shared/vault_open.rs` (300).
- **Geänderte Dateien:** `vfs/os/shared/copy_transfer.rs` (K08), `transfer/os/shared/engine/issues.rs` (K20), `app/core/reclaim_core.rs` (K21), `folder_index/os/persistence.rs` und `mobile/os/shared/index.rs` (K11, K12), `connect/os/shared/location_prefs.rs`, `app/core/prefs_tabs.rs`, `mobile/os/shared/places.rs` (K13, K14), `app/os/shared/remote_open.rs` (ein Haken), `app/os/shared/clipboard_provide.rs` und `app/os/shared/clipboard_materialize.rs` (K05 (2), (3)), `mobile/os/shared/edits.rs` (K44), `share/os/shared/storage_roots.rs` (`excluded()`, K09), `transfer/os/shared/temp.rs`.
- **Abhängigkeiten:** VLT-M4; S2 `list_mounts` mit `MountedAt` für K09.
- **Abnahmesignal:** Stufe `vault_klartext` (VLT-M14) und Stufe `vault_unit`: `vault_task_policy_every_sink_has_rule`, `vault_task_copy_spool_sealed_when_vault_involved`, `vault_task_transfer_issue_path_sealed`, `vault_task_favorites_and_recent_inside_vault_sealed` (Desktop und Android), `vault_task_folder_index_skips_vault`, `vault_task_android_fetch_rejects_vault`, `vault_task_share_export_inside_vault_mount_rejected`, `vault_task_open_fallback_requires_consent`, `vault_task_provide_and_materialize_reject_vault_source`.
- **Release/CI:** keine.

### VLT-M7 Tresor-Sync-Job

- **Ergebnis:** Aus einem Tresor heraus mit einer Bestätigung ein Sync-Job zu einer Kopie (oder einem gewählten Ort); gesperrter Hintergrundabgleich des Chiffrats; Reihenfolge R4 über Anwendungsphasen; keine automatische Auflösung; ältere se-Versionen unberührt.
- **Neue Dateien:** `bisync/core/plan_rules.rs` (150; Merkmal `PlanRules` mit `adjust` und `phases`, Standard ohne Wirkung), `vault/core/sync_rules.rs` (450; R1–R11 und Phasen), `vault/os/shared/sync_job.rs` (350), `vault/os/shared/replica.rs` (220), `syncjobs/os/shared/vault_jobs_dir.rs` (200; Ordner `vault-jobs/`, tolerantes Laden, ID-Eindeutigkeit über beide Ordner).
- **Geänderte Dateien:** `bisync/os/shared/orchestration_plan.rs` (Regelstufe nach `plan_pair_spelled`, Vollscan und Vorschau), `bisync/os/shared/apply_reporting.rs` (Phasen nacheinander), `bisync/os/shared/incremental.rs` (Sperre für `kind=vault`, R4a), `syncjobs/core/types.rs` (`kind`, `vault_site_a`, `vault_site_b`), `syncjobs/os/shared/persistence_codec.rs`, `syncjobs/os/shared/persistence.rs` (Aufrufer laden beide Ordner; `jobs/` unverändert), `syncjobs/core/validation.rs`, `mobile/os/shared/domains/job_json.rs`.
- **Vertrag:** Jobdatei `<SYNC>/vault-jobs/<id>.conf` mit `kind=vault`, `vault_site_a`, `vault_site_b`, `source=vault-store://<a>`, `target=vault-store://<b>`, feste Werte C5.6; Laufergebnis mit `deferred`.
- **Abhängigkeiten:** VLT-M3, VLT-M5; OE4.
- **Plattformprüfung:** Desktop-Worker, Android-WorkManager (`bg.catchUp`).
- **Abnahmesignal (Stufe `vault_sync`):** `vault_task_sync_seed_replica_then_seeding_false`, `vault_task_sync_one_sided_changes_both_directions`, `vault_task_sync_both_changed_untouched_conflict`, `vault_task_sync_folder_created_both_sides`, `vault_task_sync_delete_vs_change_deferred`, `vault_task_sync_masterkey_both_changed_conflict`, `vault_task_sync_jti_mismatch_aborts`, `vault_task_sync_unreachable_side_no_deletes`, `vault_task_sync_phases_content_before_node` (Fake-Backend zeichnet Zeitpunkte auf: kein Knoten-Anlegen vor Abschluss der Phase 1; kein Löschen unter `d/` vor Abschluss des Knoten-Löschens), `vault_task_sync_phase1_failure_keeps_phase2_planned`, `vault_task_sync_never_incremental`, `vault_task_sync_keepboth_actions_become_conflicts`, `vault_task_plan_rules_default_single_phase` (allgemeine Jobs unverändert), `vault_task_sync_runs_locked`, `vault_task_sync_hash_detects_same_size_same_second` (Fake-Backend mit Hash), `vault_task_sync_log_contains_no_cleartext`, `vault_task_vault_jobs_dir_invisible_to_jobs_loader` (strenges `load()` über `jobs/` unverändert erfolgreich, während `vault-jobs/` einen Job enthält), `vault_task_broken_vault_job_listed_not_fatal`; Stufe `vault_interop`: CryptoFS-Health nach jedem Szenario ohne Befunde außer erwarteten Konfliktfällen.
- **Release/CI:** keine.

### VLT-M8 Vergleichsmodus und Tresorprüfung

- **Ergebnis:** Alle Unterschiede mit Klartextnamen in den Gruppen C6.2; Aktionen C6.4 mit gültigen Chiffratnamen; „Tresor prüfen“ mit allen Health-Befunden.
- **Neue Dateien:** `vault/core/compare_model.rs` (400), `vault/core/conflict_names.rs` (150), `vault/core/health_model.rs` (250; Abbildung der 17 Befunde und vier eigenen), `vault/os/shared/dir_map.rs` (300), `vault/os/shared/compare.rs` (400), `vault/os/shared/compare_actions.rs` (450), `vault/os/shared/compare_keep_both.rs` (250; Abfolge C6.4), `vault/os/shared/compare_merge.rs` (200; Chiffrat-Übergabe an `merge_recorded_for_key`), `vault/os/shared/compare_folders.rs` (350), `vault/os/shared/health.rs` (400), `vault/os/shared/health_fixes.rs` (400). Entfällt: `merge_sealed.rs`.
- **Geänderte Dateien:** keine Bestandsdatei; Nutzung von `preview_with`, `apply_preview_action`, `resolve_checked`, `merge_recorded_for_key` über ihre Schnittstellen.
- **Abhängigkeiten:** VLT-M7.
- **Abnahmesignal (Stufe `vault_compare`):** `vault_task_compare_groups_match_preview`, `vault_task_compare_keep_a_b_with_versions`, `vault_task_compare_keep_both_valid_cipher_name` (neuer Name entschlüsselt zu „<Name> (Konflikt …)“, keine Konfliktkopie im CryptoFS-Sinn), `vault_task_compare_keep_both_long_name_c9s`, `vault_task_compare_keep_both_baseline_recorded` (folgender Lauf ohne Aktion), `vault_task_compare_merge_inputs_contain_only_ciphertext` (Markierungstext nicht in `*.merge-*.inputs/`), `vault_task_compare_folder_merge_reencrypts_names_only`, `vault_task_compare_deferred_confirm_and_restore`, `vault_task_compare_requires_unlock`, `vault_task_check_maps_every_cryptofs_finding` (je Befundtyp ein präparierter Tresor), `vault_task_check_orphan_attached_under_lost_found`, `vault_task_check_stage_leftover_removed`; Stufe `vault_interop`: CryptoFS-Health nach jeder Aktion ohne `DirIdCollision`, `OrphanContentDir`, `MissingContentDir`.
- **Release/CI:** keine.

### VLT-M9 CLI `se vault` und MCP

- **Ergebnis:** C7.3 vollständig; MCP über `se`.
- **Neue Dateien:** `cli/vault.rs` (250), `cli/vault/manage.rs` (400), `cli/vault/unlock.rs` (250), `cli/vault/sync.rs` (350), `cli/vault/compare.rs` (400), `cli/vault/output.rs` (400), `cli/vault/complete.rs` (150).
- **Geänderte Dateien:** `cli/mod.rs`, `cli/sync_jobs.rs`, `cli/sync_jobs/output.rs` (Tresor-Jobs aus `vault-jobs/` anzeigen, feste Felder abweisen), `cli/mcp/catalog.rs`, `cli/mcp/status_tools.rs`.
- **Abhängigkeiten:** VLT-M8.
- **Abnahmesignal (Stufen `vault_cli`, `vault_mcp`):** jeder Befehl der Tabelle C7.3 mit `--json` gegen exakte Schlüsselmengen; Vervollständigung; MCP über beide Protokoll-Lebenszyklen mit `se vault list --json`, `se vault unlock --password-stdin` (Argument `stdin`), `se cat vault://…`, `se vault lock --all`; `status` ohne Pfade; gesperrter Tresor → Meldung mit `se vault unlock <id>`.
- **Release/CI:** keine.

### VLT-M10 Desktop-GUI

- **Ergebnis:** C7.6 und Vergleichsfenster.
- **Neue Dateien:** `app/core/vault_sidebar_ui.rs` (300), `app/core/vault_unlock_ui.rs` (300), `app/core/vault_create_ui.rs` (400), `app/core/vault_settings_ui.rs` (450), `app/core/vault_compare_ui.rs` (450), `app/core/vault_compare_detail_ui.rs` (450), `app/core/vault_check_ui.rs` (350), `app/core/vault_sync_editor_ui.rs` (300), `app/core/vault_models.rs` (300).
- **Geänderte Dateien:** `app/core/sidebar.rs`, `app/core/job_editor_pages.rs`, `app/core/menus_sync_jobs.rs`, `app/mod.rs`, `native/Cargo.toml` (`image` mit fünf Formaten).
- **Abhängigkeiten:** VLT-M8, VLT-M6, S2 (Laufwerk, Öffnen-Stufen 1/2).
- **Abnahmesignal (Stufe `vault_surfaces`):** `vault_task_gui_unlock_states`, `vault_task_gui_compare_rows_and_actions`, `vault_task_gui_sync_editor_fixed_fields`, `vault_task_gui_sidebar_lock_event_clears_tab`, `vault_task_gui_recovery_page_text_only`; Render-Prüfung mit `app/core/gui_design_task_capture.rs`.
- **Release/CI:** sieben neue Pakete (OE5).

### VLT-M11 Android

- **Ergebnis:** C7.7, Bridge C7.5, Sperren bei Bildschirm aus und im Hintergrund (1 min), Öffnen über Provider-URIs.
- **Neue Dateien (Rust):** `mobile/os/shared/domains/vaults.rs` (450), `mobile/os/shared/domains/vault_compare.rs` (400). **(Kotlin):** `api/VaultApi.kt`, `ui/vaults/VaultsScreen.kt`, `ui/vaults/VaultUnlockSheet.kt`, `ui/vaults/VaultCreateScreen.kt`, `ui/vaults/VaultSettingsScreen.kt`, `ui/vaults/VaultCheckScreen.kt`, `ui/vaults/VaultCompareScreen.kt`, `ui/vaults/VaultCompareDetail.kt`. Entfällt: `system/VaultLockObserver.kt` (Bildschirmzustand über `HostMonitor`, VLT-M4), `ui/vaults/VaultRecoveryScreen.kt`.
- **Geänderte Dateien:** `mobile/os/shared/domains/mod.rs`, `mobile/os/shared/places.rs`, `ui/more/MoreScreen.kt`, `ui/AppNav.kt`, `ui/sync/JobCards.kt`, `ui/sync/JobEditorScreen.kt`, `ui/files/` (Öffnen über Provider-URI), `api.md`.
- **Abhängigkeiten:** VLT-M8; S2 DRV-M8 (DocumentsProvider).
- **Abnahmesignal (Stufe `vault_android_device`):** `VaultTaskTest.kt`: anlegen, entsperren mit Speicherschwelle, schreiben/lesen, Bildschirm aus → gesperrt, Hintergrund > 1 min → gesperrt, Tresor-Sync zur Kopie, Konflikt, Vergleich mit Klartextnamen, „Beide behalten“; jede der 19 Bridge-Methoden einmal aufgerufen.
- **Release/CI:** APK mit neuen Bildschirmen; keine Gradle-Abhängigkeit.

### VLT-M12 Laufwerk-Anschluss (S3-Seite der S2-Verträge)

- **Ergebnis:** Ein entsperrter Tresor lässt sich über `se drive mount vault://<SITE>` und die Desktop-Aktion einhängen; auf der Platte liegt kein Klartext (Spool und Journalfelder versiegelt); Sperren hängt aus; Absturzreste werden nach dem Entsperren gespeichert; GNOME-Miniaturbilder des Laufwerks werden beim Sperren und Laufwerksende entfernt. S3 liefert die Umsetzungen der S2-Traits und den Schlüsselweg, S2 das Frontend.
- **Neue Dateien:** `vault/os/shared/sealed_spool.rs` (450; `SealedSpoolFile` als Umsetzung von `SpoolFile`: `read_at`, `write_at`, `set_len`, `len`, `sync_data`; blockweises Neuverschlüsseln mit neuer Block-Nonce bei jedem Schreiben), `vault/os/shared/spool_key.rs` (220; Spool-Schlüssel erzeugen, mit `spool-wrap` wickeln und als `spool-key.wrapped` ablegen, entwickeln, Inhalts- und Textschlüssel ableiten), `vault/os/shared/journal_sealer.rs` (150; `JournalPathSealer` als Umsetzung von `PathSealer` mit `seal_text`/`unseal_text`), `vault/os/shared/mount_bridge.rs` (300; `open_backend_for_mount`, Antwort auf `Vault::SpoolKey`, Abonnement der Sperrereignisse, `MountTerminated` nach C3.3, Anstoß `retry_pending_changes` nach dem Entsperren), `vault/os/linux_thumbnails.rs` (220; Miniaturbild-Bereinigung nach `Thumb::URI`, nur Linux, gewählt in `vault/os/mod.rs`); Tests `sealed_spool_tests.rs`, `spool_key_tests.rs` (je < 400).
- **Geänderte Dateien:** keine Mount-Datei durch S3: Die Traits (`N/mount/core/spool_storage.rs`, `path_sealer.rs`), die Ausgliederung von `MountSource` aus `mount/core/types.rs` (499 Zeilen) nach `mount/core/mount_source.rs` und `daemon/os/shared/mount_source.rs` ändert S2 (DRV-M1, DRV-M2, DRV-M7); damit ist die Forderung „vor der Erweiterung ausgliedern“ erfüllt.
- **Abhängigkeiten:** S2 DRV-M2 (Traits), DRV-M3; VLT-M1 (Inhaltsformat), VLT-M4 (Sitzungen, IPC). S2 DRV-M7 hängt an VLT-M4 und VLT-M12 ([S2] DRV-M7); Reihenfolge: DRV-M2 → VLT-M12 → DRV-M7.
- **Abnahmesignal (Stufe `vault_mount`, zusammen mit den S2-Signalen `drive_task_vault_*`):** `vault_task_sealed_spool_random_access_roundtrip` (Lesen und Schreiben an Blockgrenzen, Kürzen, Verlängern), `vault_task_sealed_spool_detects_block_swap`, `vault_task_spool_key_wrap_unwrap_after_restart`, `vault_task_spool_key_stable_over_password_change`, `vault_task_spool_key_only_unlocked`, `vault_task_journal_text_key_derived_from_spool_key` (Textschlüssel ≠ Inhaltsschlüssel ≠ Spool-Schlüssel), `vault_task_journal_sealer_roundtrip`, `vault_task_mount_requires_unlock`, `vault_task_mount_end_reason_handling`, `vault_task_unlock_triggers_pending_retry`, `vault_task_lock_unmounts_before_zeroize`, `vault_task_linux_thumbnails_removed_by_uri_prefix` (präparierte PNGs mit `Thumb::URI` innerhalb und außerhalb des Einhängepunkts); Linux-E2E über FUSE: schreiben, Host beenden, neu starten, entsperren, Datei vollständig, kein Markierungstext unter `<AD>/mount-cache`; Windows-E2E nur, wenn die Suite Dokany installieren kann, sonst „nicht live geprüft“.
- **Release/CI:** keine neuen Pakete auf S3-Seite.

### VLT-M13 Dokumentation

- **Ergebnis:** C7.8; `docs/TODO.md` mit `VAULT-RECOVERY-KEY` (Nutzerfrage OE1, Vektor [NR] G2, Kodierung C1.10) und den Restlücken aus G.
- **Dateien:** `docs/oberflaechen/tresore.md` (neu), `docs/oberflaechen/INDEX.md`, `README.md`, `docs/ARCHITEKTUR.md`, `docs/TODO.md`, `docs/superpowers/plans/2026-09-25-android-apk/api.md`.
- **Abnahmesignal (Stufe `vault_surfaces`):** jede Zeile T1–T22 in `tresore.md` mit Beleg oder Ausnahme mit Grund.

### VLT-M14 Prüfstufen in der einen Task-Suite des Stapels (definieren, nicht ausführen)

- **Einbindung:** Kein eigener Workflow. S3 liefert die Stufen unten; der Hauptagent führt sie in den einen Stapel-Workflow und dessen Einstiegsskript zusammen (`plan.md`, „Eine einzige Task-Suite“). Zeitlimit der Jobs nach AGENTS.md ≥ 30 min.
- **Testauswahl:** libtest-Filter `cryptomator::`, `vault::`, `bisync::core::plan_rules`, `agent_proto::core::vault_error` mit `--skip mount_vault_task_` (B12); Namensabgleich: jeder ausgeführte Test dieser Stufen beginnt mit `vault_task_`.
- **Stufen (Linux-Job, Windows-Job, Android-Jobs):** `vault_unit` (Linux, Windows), `vault_interop` (Linux; Java-Harness `native/vault-task-interop/pom.xml` und `VaultTaskInterop.java` unter MIT, CryptoFS 2.11.0 unverändert zur Laufzeit aus Maven Central; keine CryptoFS-JARs als Artefakt, Release-Asset oder Cache eines öffentlich lesbaren Speichers hochladen, [NR] G21), `vault_backends` (Linux; Testserver aus `native/sync-reliability-providers.py`: SFTP, Agent, FTP/FTPS, SMB, WebDAV; Share-Paar über den Kandidaten-Server; Google Drive „nicht live geprüft“ ohne Zugangsdaten, [NR] G18), `vault_sync`, `vault_compare`, `vault_cli`, `vault_mcp`, `vault_mount` (Linux FUSE; Windows nach Verfügbarkeit), `vault_linux` (D-Bus-Prüfung VLT-M4), `vault_windows` (WTS-Verdrahtung VLT-M4), `vault_surfaces`, `vault_klartext` (Markierungsnamen und -inhalte durch Durchsuchen, Kopieren, Öffnen-Rückfall, Bereitstellen-Ablehnung, Laufwerk, Tresor-Sync, Vergleich, Merge und einen erzwungenen Prozessabbruch; danach Suche über `<AD>`, Temp-Verzeichnisse, `$XDG_CACHE_HOME/thumbnails`, Android-`filesDir`/`cacheDir` und den inneren Speicher; kein Markierungstext außer in Rückfallkopien mit Zustimmung), `vault_android_device`.
- **Kompatibilität (AGENTS.md):** `vault_task_existing_locators_unchanged`, `vault_task_plan_rules_default_single_phase`, `vault_task_vault_jobs_dir_invisible_to_jobs_loader`, `vault_task_ipc_err_code_backward_compatible`, `vault_task_vault_error_envelope_roundtrip_and_legacy_text`; Laufwerke ohne Tresor unverändert über das S2-Signal `drive_task_plain_spool_bytes_unchanged`.

## E — Schnittstellen zu S2 und S5 (Verträge)

### An S2 (angepasst an [S2] C2, C4, C6, DRV-M7, E1)

| Nr | Vertrag | S3 liefert | S2 liefert bzw. S3 verlangt |
|---|---|---|---|
| S2-V1 | Quelle | Backend einer entsperrten Sitzung je Site-ID (`mount_bridge::open_backend_for_mount`) | übernommen: `MountSource::Vault { site_id }` in `registry-2.json` nach Ausgliederung aus `types.rs`; Start bei gesperrtem Tresor scheitert mit `vault_locked` |
| S2-V2 | Spool | `SealedSpoolFile` (Umsetzung von `SpoolStorage`/`SpoolFile`); Spool-Schlüssel je Laufwerk, gewickelt in `spool-key.wrapped`, an den Host nur über `Vault::SpoolKey`, nie der Masterkey (C3.6) | übernommen: Trait `SpoolStorage` in `N/mount/core/spool_storage.rs`, Klartext-Umsetzung = Bestand byte-gleich |
| S2-V3 | Journal, Stufenbuch | `JournalPathSealer` (Umsetzung von `PathSealer`) mit Textschlüssel `se-mount-journal-v1` aus dem Spool-Schlüssel (P1 bestätigt) | übernommen: Trait `PathSealer` in `N/mount/core/path_sealer.rs`; Pfadfelder in `journal.jsonl` und `stages.json` versiegelt, Dateinamen und Format wie Bestand; Wiederherstellung `RequiresUnlock` |
| S2-V4 | Sperren | Sperrereignisse je Site, Sperrteilnehmer mit Frist | S2 veröffentlicht oder bewahrt versiegelt auf und hängt vor dem Vernichten der Schlüssel aus |
| S2-V5 | Android | Register im App-Prozess | Provider-Wurzel je entsperrtem Tresor, sonst null Wurzeln (`queryRoots`), `notifyChange` bei Sperren/Entsperren; `openProxyFileDescriptor`, keine Cache-Kopie; se öffnet Tresordateien über diese URIs |
| S2-V6 | Laufwerksende | Sperrregel C3.3 (sperren nur ohne Verbraucher außer diesem Laufwerk) | `MountTerminated { site, mount_id, reason: user_unmount|external_unmount|host_exit|lock }` |
| S2-V7 | Einhängeort und Systemzwischenspeicher | Miniaturbild-Bereinigung unter Linux nach `Thumb::URI` beim Sperren und bei `MountTerminated` (VLT-M12, `vault/os/linux_thumbnails.rs`) | übernommen für Linux: `$XDG_RUNTIME_DIR/smart-explorer/Tresore/<Name>` (tmpfs geprüft, Sticky-Bit, `0500`, GTK-Lesezeichen, KDE-Ort; außerhalb des Standardumfangs von `localsearch` und `baloo`); Android: kein MediaStore-Scan; Hinweis zu Indizierung und Miniaturbildern beim ersten Einhängen. **Windows: Abweichung P2 bestätigt**: festes lokales Laufwerk ohne besondere Laufwerksart (Annahme A14) |
| S2-V8 | Öffnen aus se | Öffnen-Regel C7.2 nutzt beide Funktionen | übernommen mit Fehlerkanal: `mount_path_for(location) -> Result<Option<PathBuf>, String>` und `ensure_mounted_for(location) -> Result<PathBuf, String>` |
| S2-V9 | Fähigkeiten | `mount_path_capabilities` der Tresorschicht | Engine nutzt nur diese Werte |
| S2-V10 | Clean-Cache | – | beim Sperren entfernen |

| T22 | Einbinden | – | **P3 bestätigt:** `se drive mount vault://<SITE>` über die gemeinsame Orts- und Auflösungsgrenze; Oberflächen in C7.1 T22, C7.2, C7.3, C7.6, C7.7, C7.8 angepasst |
| K09/K11 | Freigabewurzeln und Ordnerindex | Prüfung in `storage_roots::excluded()` und im Ordnerindex (VLT-M6) | Einhängepfade aller Tresor-Laufwerke über `list_mounts` (`MountedAt`) |

Reihenfolge: S2 DRV-M2 (Traits) → S3 VLT-M12 (Umsetzungen, Schlüsselweg) → S2 DRV-M7; VLT-M12 braucht zusätzlich VLT-M1 und VLT-M4.

### Bestätigungen an S2 (Nachtrag des Hauptagenten, [S2] E1 P1–P3)

| Punkt | Inhalt | Ergebnis S3 | Begründung |
|---|---|---|---|
| P1 | Journal- und Stufen-Pfadfelder im Laufwerkshost mit einem aus dem Spool-Schlüssel abgeleiteten Textschlüssel versiegeln; S3 liefert die Ableitung neben `SealedSpoolFile` und `seal_text` | **bestätigt**, kein Gegengrund | Der Host erhält nach C3.6 nur den Spool-Schlüssel; eine IPC-Anfrage je Journaleintrag wäre im Wiederherstellungs- und Schreibweg nötig und bei gesperrtem Tresor ohnehin unmöglich. Schlüsseltrennung: Textschlüssel = HKDF-SHA256(Spool-Schlüssel, info `se-mount-journal-v1`), Inhaltsschlüssel = HKDF-SHA256(Spool-Schlüssel, info `se-mount-spool-content-v1`); der Spool-Schlüssel selbst verschlüsselt nichts. Umsetzung `JournalPathSealer` in VLT-M12 |
| P2 | Unter Windows keine besondere Laufwerksart gegen Windows Search; Annahme bleibt offen | **bestätigt**, kein belegter Gegengrund | Netzwerk- und Wechselmodus sind in [S2] C1/C6 mit Belegen verworfen (Dokany #1326, #1305, Konflikt mit dem Network Provider; ungewolltes Aushängen bei `DOKAN_OPTION_REMOVABLE`). Eine Netzwerkart brächte nach [NR] G15 zusätzlich `Thumbs.db` in die Ordner; dass sie Windows Search ausschlösse, ist ebenfalls nicht belegt. Annahme A14 (= S2 A13) bleibt offen; Hinweis beim ersten Einhängen (S2) und in `tresore.md` |
| P3 | Einbinden über `se drive mount vault://<SITE>` statt `se drive add --vault` | **bestätigt**, kein Gegengrund | Ein Ort über die gemeinsame Orts- und Auflösungsgrenze entspricht AGENTS.md („New backends and path representations must work through the shared location/resolution boundary“); kein zweiter Unterbefehl. Angepasst: CLI (C7.3), MCP (C7.3), Desktop (C7.1 T22, C7.2, C7.6), Android (C7.1 T22, C7.7), Doku (C7.8) |

### An S5 (unverändert)

| Nr | Vertrag | S3 liefert | S5 |
|---|---|---|---|
| S5-V1 | Daten | `Vault::Status`/`vault.list` | Kachel/Ordner je Tresorort mit Sperrsymbol |
| S5-V2 | Aktionen | Entsperrdialog der App, Sperren, Öffnen, Vergleichen | kein eigenes Passwortfeld |
| S5-V3 | Ereignisse | `VaultEvent` | Aktualisierung ohne Abfrageschleife |
| S5-V4 | Locator | `vault://<site-id>/` | nur diesen Wurzel-Locator speichern |
| S5-V5 | Miniaturbilder | Inhalte nur über `vault://` oder Provider-URI | keine Miniaturbilder oder Namen von Tresorinhalten auf Platte; gesperrt = keine Inhaltsanzeige |
| S5-V6 | Gerät als Ordner | Tresor eines verbundenen Geräts = Tresorort mit innerem `share://`-Backend | nur eingetragene Tresororte zeigen |

## F — Risiken und unbestätigte Annahmen

**Risiken:**

| Nr | Risiko | Wirkung | Minderung |
|---|---|---|---|
| R-1 | Fehler in der eigenen Formatumsetzung | beschädigte Nutzertresore | Vektoren, Gegentest, Health nach jedem Szenario, Stufe + Veröffentlichung |
| R-2 | `aes-siv` ohne Audit | Implementierungsfehler im SIV | 640 siv-mode-Fälle, Wycheproof im Crate, `cargo audit` |
| R-3 | Wiederherstellungsschlüssel fehlt in se | Nutzer brauchen Cryptomator Desktop dafür | Informationstext; OE1 |
| R-4 | Klartext in Zwischenspeichern fremder Programme (K45, K47) | K6 nur für se-Ablagen und die belegten Gegenmaßnahmen | S2-V7, Sperren beim Ruhezustand, Doku |
| R-5 | unbekannte Chiffrat-Strukturfälle | falsche Struktur | „unbekannt = Konflikt“, Health nach Szenarien |
| R-6 | Rollback auf ≤ 0.5.182 (NR: G10) | Tresor-Jobs unsichtbar und ohne Lauf; allgemeine Jobs unberührt | eigener Ordner `vault-jobs/` |
| R-7 | Schlüssel im Worker-Speicher, Klartext auf Loopback | lokaler Angreifer mit Nutzerrechten | Token, Sperrauslöser |
| R-8 | innere Backends ohne atomare Operationen | Rennen bei mehreren Klienten | Fähigkeiten = Schnittmenge |
| R-9 | Abhängigkeit von S2 | ohne S2: Desktop nur mit Zustimmungskopie, Android ohne externes Öffnen | Reihenfolge D |
| R-10 | Leistung auf Hochlatenz-Backends | langsames Durchsuchen | Verzeichnis-ID-Zwischenspeicher |
| R-11 | Testfilter trifft `mount_vault_task_*` | falsche Zählung | Modulfilter, `--skip` |
| R-12 | Maven Central im CI | `vault_interop` fällt aus | fehlgeschlagener Abruf ist kein Pass |
| R-13 | gemischte GUI/Worker-Versionen | unbekannte IPC-Variante | vorhandene Versionsbehandlung |
| R-14 | Phasenänderung in `apply_reporting.rs` (OE4) | Regression aller Jobs | Standard eine Phase; `vault_task_plan_rules_default_single_phase` |
| R-15 | Health-Reparatur von CryptoFS verschiebt se-Stufen nach `LOST+FOUND` (NR: G19) | Stufen unter fremdem Namen sichtbar (verschlüsselt) | „Tresor prüfen“ meldet und entfernt liegengebliebene Stufen |
| R-16 | Cryptomator Desktop 1.19.3 ohne #355-Korrektur | Cryptomator kann Ordnerkonfliktkopien eines Fremd-Syncs falsch auflösen | se erzeugt keine Ordnerkonfliktkopien; Hinweis in `tresore.md` |
| R-17 | Linux-Tresor-Laufwerk unter `$HOME` | Indexer erfassen Klartext | erledigt: S2 hängt Tresore unter `$XDG_RUNTIME_DIR` ein ([S2] C2); Restannahme A6 |
| R-18 | Passwörter über MCP (`stdin`) im Agentenkontext | Offenlegung an den Agenten-Anbieter | Hinweis im Katalog und in `tresore.md` |

**Unbestätigte Annahmen mit Wirkung:**

| Nr | Annahme | Wirkung, falls falsch |
|---|---|---|
| A1 | Ein nie angezeigtes Fenster der obersten Ebene im Worker erhält `WM_WTSSESSION_CHANGE` nach `WTSRegisterSessionNotification` ([NR] G13) | „Sperren bei Bildschirmsperre“ wirkt unter Windows nicht; Ruhezustand, Leerlauf und ausdrücklich bleiben |
| A2 | GNOME Shell und KDE Plasma setzen `LockedHint` ([NR] G14) | Bildschirmsperre per Tastenkürzel sperrt unter Linux nicht; nur `loginctl lock-session` |
| A3 | Miniaturbilder unter `$XDG_CACHE_HOME/thumbnails/` tragen das PNG-Textfeld `Thumb::URI` (freedesktop-Spezifikation, in [NR] nicht abgerufen) und GNOME Files schreibt es | Bereinigung findet die Dateien nicht; Klartext-Miniaturbilder bleiben bis zur Bereinigung durch den Nutzer |
| A4 | Der se-Worker läuft unter Windows als Benutzerprozess, nicht als Dienst ([NR] G13) | WTS-Benachrichtigung bräuchte `HandlerEx` |
| A5 | `merge_execution` schreibt Zielbytes über Stufe und Veröffentlichung ([NR] G26 Lücke) | ein abgebrochener Merge könnte ein halbes Chiffrat hinterlassen |
| A6 | Distributionen ändern die Standardumfänge von `localsearch`/`baloo` nicht ([NR] G15) | Einhängeort außerhalb `$HOME` würde trotzdem indiziert |
| A7 | Windows-`thumbcache` erfasst Dokany-Tresor-Laufwerke nicht im Klartext (unbelegt) | Klartext-Miniaturbilder im Benutzerprofil |
| A14 | Windows Search indiziert ein zusätzliches festes Dokany-Laufwerk nicht ohne Zutun des Nutzers (= S2 A13; P2) | Tresorinhalte im Suchindex des Benutzerprofils |
| A8 | NFC-Stabilität zwischen ICU4X 2.2 und `java.text.Normalizer` ([NR] G7) | Namen mit neu zugewiesenen Zeichen nicht gegenseitig auffindbar |
| A9 | `CatchUpRecord.message` enthält keine Pfade ([NR] G24) | Tresor-Job-Pfade (nur Chiffrat) im Klartext-Zusammenfassungstext; kein Klartext des Tresors |
| A10 | Vollscan ohne Baseline plant keine Löschungen ([NR] G11 nennt nur die Semantik) | Erstbefüllung einer Kopie könnte löschen; Gegenmaßnahme R7 erlaubt nur leere Kopie bei `seeding=true` |
| A11 | `availMem − threshold` reicht als Android-Grenze für native scrypt-Zuweisungen ([NR] G17) | Abbruch durch den Low-Memory-Killer bei sehr großen N |
| A12 | Die Rechtslage der Wortliste ist ungeklärt (Folgerung [NR] G1) | ohne Klärung bleibt die Funktion weggelassen (OE1) |
| A13 | Ein Actions-Cache des öffentlichen Repositories gilt nicht als Weitergabe ([NR] G21 Lücke) | Harness darf keinen Cache mit CryptoFS-JARs anlegen; VLT-M14 verbietet es vorsorglich |

## G — Verbleibende Lücken nach der Nachrecherche

| Nr | Lücke | Entscheidungsrelevanz | Umgang |
|---|---|---|---|
| L-A | Zustellung WTS an verborgenes Fenster (A1) | K13 | Verdrahtungstest in `vault_windows`; Betriebssystem-Zustellung nicht automatisierbar, im Suite-Bericht als „nicht geprüft“ |
| L-B | `LockedHint` in GNOME/KDE (A2) | K13 | Doku; Prüfung mit Test-logind in `vault_linux` |
| L-C | Pflichtfeld `Thumb::URI` in der freedesktop-Thumbnail-Spezifikation (A3) | K6 Linux | vor VLT-M12 in der Spezifikation nachlesen (Umsetzungsschritt, keine Planänderung) |
| L-J | Windows Search und `thumbcache` auf festen Dokany-Laufwerken (A7, A14) | K6 Windows | Doku als ungeprüft; Hinweis beim ersten Einhängen |
| L-E | `merge_execution`-Atomarität (A5) | K38 | vor VLT-M8 lesen; falls nicht atomar, Tresor-Merge schreibt über die Stufen der Tresorschicht statt über `merge_recorded_for_key` |
| L-F | Wie Vorschau-Gruppen Phasen darstellen (`preview.rs` ab Zeile 190, [NR] G8) | C6.4 „Jetzt übertragen“ | Vergleichsmodus wendet Ordnereinträge selbst in R4-Reihenfolge an; keine Abhängigkeit |
| L-G | WebDAV-Server mit Hashangabe (`webdav.rs:397`, [NR] G9) | C5.5 | Doku nennt „je nach Server“ |
| L-H | Drive-Testzugangsdaten ([NR] G18) | K3 | „nicht live geprüft“ |
| L-I | Mountain Duck ([NR] G23) | Interop-Doku | als ungeprüft benannt |

## Prüfprotokoll Nachrecherche

Grundlage: `nachrecherche-tresore.md` (G1–G26, Abschnitt 2 Stände, Abschnitt 3 Nr. 1–59, Abschnitt 4 Folgen). Je Eintrag: Ergebnis der Nachrecherche → Änderung in diesem Plan.

**Antworten G1–G26:**

| G | Ergebnis | Änderung im Plan |
|---|---|---|
| G1 | Wortliste aus „Oxford Top 5000“, ohne freie Lizenz; Weitergabe rechtlich ungeklärt | C1.10 W-a (weglassen, Informationstext); OE1; VLT-M1/M5 ohne `recovery_key.rs`/`wordlist.rs`; T12/T13; `docs/TODO.md` `VAULT-RECOVERY-KEY` |
| G2 | Referenzschlüssel belegt; CRC = Little-Endian `[0..2]` | C1.2 (4): Vektor für Folgearbeit festgehalten |
| G3 | 512-Bit-SIV-Vektoren (siv-mode MIT, aes-siv); Reihenfolge MAC ‖ Enc bestätigt | C1.2 (2): 640 Fälle, Testdatei mit Lizenzvermerk |
| G4 | leere Datei nur Header; Überschreiben behält Header | C1.7 bestätigt; C5.5 Begründung |
| G5 | keine Lesegrenze in CryptoLib; Formel `128·r·(N+3)`; Suffix Großbuchstaben; `.bkup` beim Öffnen | C1.3 (Formel, 64 KiB als eigene Grenze); C3.7 bestätigt; B13 |
| G6 | Rust 1.99.1/1.99.0; MSRV ≤ 1.89; Bereiche erfüllt; RustSec leer | C1.1 bestätigt; das Entwurfsrisiko „CI-`stable` < 1.89“ entfällt |
| G7 | `icu_normalizer` bereits direkt | C1.1: Wiederverwendung, kein Rückfall |
| G8 | Inkrementell umgeht `plan_pair`; Gruppen parallel; keine Abhängigkeiten | C5.4 R4 als Anwendungsphasen, R4a; VLT-M7 Dateien; OE4; R-14 |
| G9 | `MtimeSize` nutzt Serverhashes; `remote_watch` über `resolve_endpoint` | C2.3 (Auflösung ohne Hülle), C5.5; VLT-M3 Signal |
| G10 | ≤ 0.5.182: GUI lädt keine Jobs bei unbekanntem Schema in `jobs/` | C5.8 eigener Ordner `vault-jobs/`; K28; R-6; VLT-M7 |
| G11 | `target_managed` nur Einwegspiegel-Index | C5.9 ohne `target_managed`; A10 |
| G12 | Vermittlung generisch; Fehler nur als Text | C3.2 Umschlag `SE_VAULT_V1`, IPC-Feld `code`; C7.4; VLT-M4 |
| G13 | Energie-API umsetzbar; WTS braucht `Win32_System_RemoteDesktop`; `HWND_MESSAGE` unbelegt | C3.4 verborgenes Fenster der obersten Ebene; A1, A4; K13 offen |
| G14 | keine Signale an `session/auto`; `LockedHint`; Inhibitor | C3.4 Linux neu; A2; VLT-M4 Stufe `vault_linux` |
| G15 | GNOME-Miniaturbilder für eigenen FUSE-Typ; Indexer in `$HOME`; `Thumbs.db` in Netzwerkordnern | K45 (Bereinigung nach `Thumb::URI` durch S3), S2-V7 (Ort außerhalb `$HOME`, von S2 übernommen), R11, A3, A6, A7, A14; P2 |
| G16 | Leerlauf aus/30 min; Passwort speichern aus; Mindestlänge 8; Android Bildschirm aus an, Hintergrund 1 min | C3.3 (Hintergrund 1 min), C7.6/C7.7 (Mindestlänge 8) |
| G17 | `MemAvailable` Schätzwert; `threshold` über Java-API; Funktionen vorhanden | C1.3 (c) mit `transfer::available_memory()` und Bridge-Argumenten; A11 |
| G18 | Docker-Testserver vorhanden; Drive ohne Zugang | VLT-M14 `vault_backends`; K3; L-H |
| G19 | Stufen unsichtbar; `OrphanContentDir.fix` verschiebt sie | C2.6, R-15, C6.2 „liegengebliebene se-Stufe“ |
| G20 | `image` nur PNG aktiv; sieben neue Pakete | C6.3, OE5, VLT-M10 |
| G21 | AGPL §2: Ausführen ohne Weitergabe bedingungsfrei | VLT-M14 (keine JAR-Artefakte), A13 |
| G22 | 17 Befundtypen mit Reparaturen | C6.2 Tabelle; C6.4; VLT-M8 `health_model.rs`, `health_fixes.rs` |
| G23 | Cyberduck blendet `.c9s` und Konfliktkopien aus | C7.8 Interop-Aussage |
| G24 | Inhalte K04, K22, K23, K26, K41–K43; Reiterpfade nur Speicher | C4 K04, K14, K26, K41–K43; A9 |
| G25 | drei materialisierende Wege; Freigabewurzeln in `storage_roots` | C4 K05, K09; VLT-M6 |
| G26 | Merge auf Bytes; Engine-Keep-Both bildet ungültige Chiffratnamen | C6.4 (Chiffrat-Merge, eigene Keep-Both-Abfolge), K38, R10; VLT-M8 |

**Stände (Abschnitt 2):** CryptoLib 2.3.0, CryptoFS 2.11.0, Desktop 1.19.3 (ohne #355-Korrektur), siv-mode `2.0.0-rc1`, RustSec leer für `aes-siv`/`aes-kw`, Cryptomator Android ohne DocumentsProvider, Rust 1.99.1 — bestätigt; Änderung: R-16, C7.8.

**Revalidierung Nr. 1–59 (Abschnitt 3):**

| Ergebnis | Nummern | Änderung im Plan |
|---|---|---|
| bestätigt ohne Änderung (37) | 1, 2, 4, 8, 9, 10, 11, 13, 15, 18, 19, 20, 21, 24, 25, 26, 27, 28, 29, 30, 31, 36, 37, 38, 39, 40, 42, 47, 49, 51, 52, 53, 54, 55, 56, 58, 59 | keine; Belegvermerke in B und C |
| bestätigt mit Einschränkung oder Ergänzung (6) | 17 (`Params::new` mit drei Parametern), 22 (Stufen nach `LOST+FOUND`), 23 (sechs im Entwurf nicht zitierte Stufenformen), 35 (Empfänger vorhanden, Feld fehlt), 44 (unbekannte Schlüssel gehen bei `upsert` verloren), 50 (Einhängestelle Chiffrat-Merge) | 17 → C1.1; 22 → C2.6, R-15; 23 → C2.6 vollständige Abbildung; 35 → C3.4 `screenOn`; 44 → C5.8 eigener Ordner; 50 → C6.4 |
| teilweise bestätigt (3) | 6 (`jsonwebtoken`-Standardwerte nicht erneut gelesen), 33 (Windows-Signale), 43 (Rückfallschutz) | 6 → keine (Crate verworfen); 33 → C3.4; 43 → C5.8 |
| Lücke geschlossen (3) | 12 (SIV-Vektoren), 41 (Serverhashes), 48 (Bild-Pakete) | 12 → C1.2; 41 → C5.5; 48 → C6.3, OE5 |
| widerlegt (8) | 3 (`icu_normalizer` neu), 5 (vier Pakete), 7 (`uuid` `v4` unbestätigt), 14 (Speicherformel), 16 (Lesegrenze aus Referenz), 32 (Hintergrund 5 min), 34 (`session/auto`), 46 (`target_managed`) | 3 → C1.1; 5 → C1.1 (sieben Pakete); 7 → C1.1 (Begründung geändert, Entscheidung bleibt); 14 → C1.3; 16 → C1.3 (eigene Grenze); 32 → C3.3 (1 min); 34 → C3.4; 46 → C5.9 |
| nicht belegbar (2) | 45 (R4 mit Bestand), 57 (`$XDG_RUNTIME_DIR` im Bestand) | 45 → C5.4 Anwendungsphasen, OE4; 57 → S2-V7, von S2 in der Endfassung entschieden ([S2] C2) |

Summe: 37 + 6 + 3 + 3 + 8 + 2 = 59.

**Folgen (Abschnitt 4), Zuordnung:** C1.10 → C1.10/C3.8/C7.1/VLT-M5; C1.1 → C1.1; C1.2 → C1.2/K2/VLT-M1; C1.3 → C1.3/K11; C3.3 → C3.3 (Standardwerte mit Begründung; das Entwurfsrisiko zu den Standardwerten entfällt); C3.4 → C3.4/C7.4/C8.3/VLT-M4; C5.4 → C5.3/C5.4/VLT-M7; C5.5 → C2.3/C5.5/VLT-M3/VLT-M7; C5.8 → C5.8/R-6/K28; C5.9 → C5.9; C6.2 → C6.2/C6.4/C6.6; C6.3 → C6.3/VLT-M10; C6.4 → C6.4/K38/VLT-M8; C7.4 → C3.2/C7.4/C7.5; C4 K05/K09/K26/K45 → C4/S2-V7; `tresore.md` → C7.8/VLT-M13; VLT-M14 → VLT-M14/R-12/A13. Jede Zeile von Abschnitt 4 ist damit eingearbeitet.

**Nachtrag Abstimmung mit S2 (Endfassung 16:40, Nachricht des Hauptagenten):** Übernommen: Linux-Tresor-Einhängeort unter `$XDG_RUNTIME_DIR` (OE2 erledigt, K6 erfüllt), Feld `site_id` (OE3 erledigt), Traits `SpoolStorage` und `PathSealer` mit S3-Umsetzungen `SealedSpoolFile` und `JournalPathSealer` (C3.6, VLT-M12; der Entwurfsstand „S2 versiegelt mit einem S3-Samen“ ist ersetzt), `Vault::SpoolKey`, `mount_path_for`/`ensure_mounted_for` mit Fehlerkanal, `list_mounts` (`MountedAt`) für K09, Ausgliederung von `types.rs` durch S2. Bestätigt: P1, P2, P3 (Abschnitt „Bestätigungen an S2“). Neu bei S3: Miniaturbild-Bereinigung unter Linux nach `Thumb::URI` (K45, VLT-M12), weil die S2-Endfassung das Löschen nach MD5-Namen aus dem Entwurf nicht übernimmt und S3 die Bereinigung ohne Pfadliste des Hosts leisten kann.
