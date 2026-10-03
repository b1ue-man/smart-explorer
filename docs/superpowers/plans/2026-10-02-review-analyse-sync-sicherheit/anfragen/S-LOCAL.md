# S-LOCAL – Anschlüsse und Grenzen

Stand 2026-10-03. Aktueller Block beendet; keine lokale Ausführungsabnahme.

| ID | Owner | Konkreter Anschluss | Stand |
|---|---|---|---|
| A1 / S09-LINK | Hauptagent / präziser Folgeauftrag | Gepinnter Iroh-Statuskanal plus bestätigter selektierter privater IP-Pfad. Signierte mDNS-Snapshotdatei ist nur private Dial-Evidenz, keine Uplink-Autorität. Anschluss an `lan_runtime.rs::paired_sightings/tick_uplink`, `PeerOnLink.authenticated` und `lan_uplink_evidence.rs::authorize`. | S09-LINK `2fbdfddd` und Parent-Registrierungen verbinden realen gepinnten TLS-/Interface-Nachweis. Kein positiver ICS/NAT-Start allein mit Beacon; gemeinsame Laufabnahme bleibt offen. |
| A2 / Deinstallation | Hauptagent | `net::run_uplink_helper_if_requested(&[OsString])` erkennt jetzt auch `--lan-uplink-cleanup` auf Windows/Linux. NSIS ruft es vor CLI-Löschung auf und bricht bei Fehler ab. Bestehenden CLI-Einstieg außerhalb Scope bestätigen; Linux-Uninstall vor Binärlöschung mit ursprünglicher Benutzeridentität/-Appdaten aufrufen, nicht erst als fremder root mit anderem HOME. | Native Hook, CLI-Einstieg und Windows-NSIS sind angeschlossen. README/RELEASING `7cb1f2ef` dokumentieren den vorhandenen Linux-Disable-/Cleanup-Weg vor Binärentfernung; ein Linux-Uninstall-Script existiert nicht. |
| A3 / gemeinsame Suite | Hauptagent | Neue Facility::RepairRequired, LanSettings-/UplinkView-/PeerOnLink-Felder in betroffenen externen Literalen/Matchers berücksichtigen. Über eine Remote-Suite die in abnahme/S-LOCAL.md benannten Signale sammeln. | Nicht lokal ausgeführt. Kein eigener Cargo-/Manifestauftrag. |
| A4 / Exec-Journal | Hauptagent | `daemon/os/windows/ipc_storage.rs::secure_exec_journal_temp` prüft private Exact-Handles. Neuer Erzeugungsweg `support_dirs::write_private_atomic` / `create_private_file` statt permissiver NamedTempFile vor Bytes. | Parent `4f4d75b` verwendet private atomare Erzeugung vor dem Schreiben; Datei nicht vom Subagent geändert. |
| A5 / Profil-CAS | S-POLICY | `ensure_private_dir`, `open_private_file`, `open_private_lock`, `create_private_file` mit bestehender CAS-Promotion. | Owner hat Anschluss bestätigt; keine fremde Profildatei geändert. |

Private Objekte verwenden fertige V1-DACL-Haken; keine eigene ACL-Dopplung im
Profil-/Identity-Store. Windows-Identity-Migration und Linux/Android-Keystore bleiben
laut Plan zurückgestellt. S34-Redaktionen sind Hauptagent-Integrationen fe2ca3d/f7d6af7.

Unsichere Windows-Verzeichnis-DACL wird bei bewusster Reparatur durch eine neue Root
ersetzt. Alte Root bleibt mit Zufallsnamen `.retired-<uuid>` ohne Taskverweis zurück;
unerwartete/reparse/fremde Dateien werden nicht rekursiv entfernt. Dies ist konservative
Quarantäne, keine zweite ausführbare Installation. Die spätere Runner-Abnahme muss
auch die reguläre Off-/Uninstall-Entfernung und retrybaren Fehlerpfad prüfen.
