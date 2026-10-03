# S-LOCAL – Umsetzung und Abnahme

Stand: 2026-10-03. Fortsetzung des freigegebenen RV1-Plans für FC7/B14/S60;
kein neues Projekt-Review. Ausführungsabnahme ausschließlich in der einen
Remote-Task-Suite des Hauptagenten.

## Abgegrenzter Meilensteinplan

| Meilenstein | Zugeordnete Grenze | Erwartetes Ergebnis |
|---|---|---|
| Private Objekte ab Erstellung | `support_dirs`, Credential-/Identity-/IPC-Stores; V-LOCAL-ACL-Haken | Verzeichnisse entstehen mit 0700 oder geschützter Besitzer-DACL, Dateien mit 0600 oder derselben DACL. Vor Lesen/Schreiben sind Art, Besitzer und Hardlinks geprüft. Eigene alte Rechte werden ohne fremde Objekte zu ändern eingeengt. |
| Begrenzte Identitätssperren | Linux-/Windows-`identity_lock` | Konkurrenz wartet mit absolutem Zeitlimit; fremde/linkartige Objekte werden abgelehnt. Ein Fehler verhindert nicht den nächsten sicheren Versuch. |
| IPC-Zulassung vor einem Bedienplatz | `ipc_listener` und eigene Zulassungshilfe | Stumme/falsche Nachrichten verbrauchen keinen Bedienplatz; vorhandene Daemon- und MountHost-Capabilities bleiben maßgeblich. Vorprüfung ist begrenzt und blockiert nicht den Accept-Loop. |
| Sichere Uplink-Installation und Reparatur | Windows-Helfer, Linux-polkit, Settings/UI | Keine erhöhte Ausführung aus benutzerschreibbarem Exe-/Script-Pfad. Hash und Administratorrechte werden geprüft. Altinstallation wird erkannt und nicht automatisch benutzt; eine bewusste Reparatur ersetzt sie. Ausschalten/Deinstallation entfernen die Installation. |
| Retrybarer Uplink-Lebenszyklus | freigegebene Setup-/Cleanup-Stellen in `lan_uplink_runtime` | Durable Intent vor Start; fehlgeschlagenes Abschalten behält den Record und die Policy. Reparatur/Entfernung sind wiederholbar und melden Fehler. |
| Private und authentisierte LAN-Präsenz | `lan_presence*`, freigegebene Runtime-Stellen | Ankündigung nur mit akzeptierten Direct-Kontakten, wechselnder geheimnisgebundener ID ohne Rechnernamen; begrenzte Ereignisse/Sichtungen. Uplink verwendet ausschließlich verifizierte frische Tatsachen. Legacy-Dialhinweise bleiben unvertrauenswürdig. Firewall nur UDP/private+domain. |
| Eigener Self-Review und Suite-Signale | nur eigene Änderungen | Kleine statische Parsing-/Whitespace-Prüfung; konkrete Remote-Symbole und Integrationserwartungen im Abschlussbericht. Keine lokale Ausführung. |

## Entscheidungen und Kompatibilität

- Speicherorte, Credentialformat, Profil-CAS und Identity-Repairtransaktion bleiben erhalten.
- Windows-Identitätsmigration und Linux/Android-Keystore bleiben laut Plan zurückgestellt.
- V1/V5 werden über ihre dokumentierten APIs benutzt; V-LOCAL besitzt die ACL-Implementierung,
  S-POLICY die Profildatei-Anbindung. S34-Debug-Redaktion ist Hauptagent-Integration `fe2ca3d`.
- Bestehende Direct-/Room-, UNC- und LAN-Dialwege behalten ihre Identität und ihre Routen.
  Eine LAN-Sichtung allein ist keine Freigabe und keine authentisierte Uplink-Tatsache.

## Ergebnis und Fundzuordnung

Der aktuelle begrenzte S-LOCAL-Block ist implementiert und statisch geprüft.
Die Verhaltensabnahme ist offen. S09 ist bewusst failclosed und geht laut
Hauptagent in den gesonderten S09-LINK-Anschluss; FC7 insgesamt ist erst nach
jenem Anschluss und der gemeinsamen Remote-Suite vollständig abgenommen.

| Fund | Konkrete Umsetzung | Abnahmegrenze |
|---|---|---|
| S62 / lokaler Anteil S33 | Private Verzeichnisse und Stage-/Daten-/Lockdateien ab Erzeugung; Unix 0700/0600, Windows V1-Owner-DACL. Art/Owner/Links und eigene alte Rechte werden vor Datenzugriff am Handle geprüft. Identity-Reader und Stage angeschlossen; Profil-CAS über S-POLICY; private atomare TSV/Policy/LAN/Uplink-Stores. | Kein permissives Erstellungsfenster; keine Keystore-Erweiterung. |
| S59 | Windows Token/Journal-/Address-/Generation-Dateien benutzen private Handles; Address/Generation bounded private read/write. Tokens schließen den Erstellungshandle vor geprüfter Wiederlesung. | Fremder Owner, Reparse und Hardlink werden vor Bytes abgelehnt. |
| S35 | Prozessmutex und OS-Sperre teilen eine absolute 5-s-Frist; try_lock/LOCK_NB, Poison-Recovery, keine Änderung der Repairtransaktion. | TimedOut ist retrybar; keine Endlosschleife. |
| S60 | Loopback-, 16-KiB-Capability-Präambel vor jedem der 16 Workerplätze; 256 kurze Pending-Verbindungen, 250-ms-Frist, 64 pro Poll. Peek verbraucht keine Nachrichtenbytes. | Stumme/falsche Clients belegen keinen Worker; Daemon-Token und unabhängige MountHost-Token gelten unverändert. |
| S10 / B14 | Windows: unveränderliches EncodedCommand, absoluter System-PowerShell-Pfad, gepinnte Source durch UAC, SHA-256 nach Elevation und Kopie. Helfer/Manifest unter ProgramFiles mit BA/SYSTEM-Owner und geschützter DACL ab Erstellung; Task mit geschützter DACL direkt bei COM-Registrierung, exakt ein privater SID/Root-Pfad. Jede Ausführung verifiziert Hash, aktuelle Payload, Owner/DACL, Taskprincipal und Aktion. | Alt-/fehlerhafte Installation ist RepairRequired; nur ausdrückliche Reparatur. Eigener alter Task wird entfernt. |
| S11 / B14 | Linux: root-owned single-link 0644-Regel, exakter Text für OS-Benutzer, subject.local && subject.active. Kein user-schreibbarer Zwischenstand bei pkexec; rootseitige private Stage und atomare Promotion. | Alte/unsafe Regel wird nicht benutzt; bewusste Reparatur/Entfernung mit einer Bestätigung. Fehlendes pkexec erlaubt nur bei fehlender Regel die bisherige normale Desktop-Policy. |
| S10/S11-Lebenszyklus | Dauerhafte Startabsicht vor enable; fehlgeschlagener/unklarer Start und fehlgeschlagener Stop behalten den Record. Stop mit Backoff, Neustart reconciled zuerst. Cleanup stoppt die erfasste Sitzung und entfernt Installation; Fehler behalten Retryzustand. NSIS ruft Cleanup vor Löschung auf und bricht bei Fehler ab. | Kein Erfolg bei fehlgeschlagener Record-/Settings-Persistenz; kein erneuter UAC-Loop nach Ablehnung. |
| S14 | HMAC-ID wechselt pro 15-min-Epoche, keine Rechnernamen. Ankündigung nur mit akzeptierten Direct-Kontakten und vorhandenen Credentialmaterialien. Signierte Node-gebundene Adress/Port/Uplink-Tatsachen, begrenzter Eventkanal/Map/Drain und zusammengeführte Routen. | Neue ID ist v2. Alte 16-hex-IDs bleiben Dialhinweise, geben keine Uplink-Rechte. |
| S15 | Nur UDP/private+domain; keine UAC-Anfrage beim automatischen Service-Start. Eigener GUI-Button startet eine ausdrückliche Hintergrundfreigabe für den gebündelten se.exe-Worker. | Öffentliche Netze erhalten keinen automatischen Program-Allow; alte Regel per bewusster Reparatur ersetzen. |
| S34 | Hauptagent: ShareIdentity/DirectCodeRotation fe2ca3d, PeerEndpoint f7d6af7 redigiert. Neue secret-bearing Caches/IPC-Hints haben kein Debug; Peer-Secretpuffer werden gelöscht. | Diese beiden Root-Integrationen sind keine eigenen Dateiänderungen. |
| S09 | Signierte Beacons alleine starten und erhalten keine Freigabe: Produktions-PeerOnLink.authenticated=false, privilegiertes authorize gibt ohne echten Kanal keinen Erfolg zurück. | **Offen für S09-LINK:** gepinnter Iroh-Kanal plus tatsächlich selektierter privater IP-Pfad. |

## Konkrete Signale für die eine Remote-Task-Suite

- `private_storage_tests.rs::review_task_private_objects_are_restrictive_before_data_is_written`
  und `review_task_private_objects_refuse_links_and_tighten_owned_old_modes`: 0700/0600
  schon am leeren Objekt, eigene alte 0644 vor Datenzugriff enger, Symlink/Parentlink,
  Hardlink und FIFO abgelehnt. Windows-Gegenstück über fertige V1-ACL-Prüfung am Objekt.
- `ipc_admission.rs::review_task_ipc_prelude_does_not_consume_the_original_request` und
  `review_task_ipc_prelude_allows_large_authenticated_payload_after_small_prefix`.
  Produktionsintegration: mehr als 16 stumme/falsche Clients + anschließender richtiger
  Daemon-Client und gültige MountHost-Clients bedienen; Grenze/Timeout/Shutdown erhalten.
- `identity_lock.rs::review_task_identity_lock_timeout_is_retryable`; bestehende
  `concurrent_acquirers_are_serialized`, Link-/Mode-Ablehnung und Windows-Sperrkonkurrenz.
  Auch Prozessmutex-Konkurrenz muss dieselbe Gesamtfrist benutzen.
- `lan_privacy.rs::review_task_lan_ids_rotate_and_forged_advisories_are_not_uplink_facts`
  und `lan_uplink_policy.rs::review_task_legacy_lan_hint_cannot_start_or_keep_uplink_sharing`.
  mDNS: keine Kontakte = keine Ankündigung; kein Hostname; wechselnde ID; begrenzte
  Sichtungen; moderne/Legacy-Routen vereinen, Loss-Spoof löscht keine verifizierten Fakten.
- Windows-Runnersignal: alte benutzerschreibbare Taskaktion wird beim Start als Reparatur
  erkannt; keine automatische Nutzung/UAC; ein benutzerschreibbares PSModulePath/Fakemodul darf keinen privilegierten Code laden. Reparatur hält Source unveränderbar durch
  Bestätigung, Task und Dateien sind bereits ab Erstellung geschützt. Geänderte Hashes,
  Aktion, Owner/DACL oder Principal blockieren task start; erneute bewusste Reparatur
  funktioniert. Aktualisierte Desktop-Payload erfordert Reparatur der alten Helferkopie.
- Linux-Runnersignal: alte Regel ohne local/active, falscher Text/Owner/Mode/Link blockiert;
  sichere Reparatur atomar, inaktive oder entfernte Sitzung bekommt keine Passwortlos-Regel.
  Off/Cleanup/Deinstallation entfernen sie; Abbruch/Ablehnung bleibt sichtbar retrybar.
- Durable Runtime: Persistenzfehler vor Start erzeugt keine Netzwerkänderung; enable-Fehler,
  unklarer Workerabbruch, stop-Fehler, cleanup-Fehler und Neustart behalten Autorität zum
  nächsten Stop. Erst bestätigter Stop samt Record-Persistenz konsumiert Stopwunsch.
- S09-LINK ergänzt das positive automatische Startsignal. Der jetzige Stand muss bei einem
  kopierten/korrekt signierten Beacon ohne echte Iroh-Linkbestätigung **inaktiv** bleiben.
- NSIS verweigert Dateilöschung bei fehlgeschlagenem `--lan-uplink-cleanup`; bestehender
  CLI-/Linux-Uninstall-Owner bestätigt den entsprechenden Anschluss außerhalb dieses Scopes.

## Entscheidungen und praktische Grenzen

- `write_private_atomic` promotet Path-native über `vfs::replace_local_file(&stage,path)`;
  keine neue UTF-8-Verengung für gültige Linux-Appdatenpfade. File-sync und Unix-dir-sync;
  Windows V1-write-through statt erfundener Directory-flush-Garantie.
- Unix wählt den physischen Elternpfad einmal und pinnt ihn per openat/O_NOFOLLOW; private
  Endverzeichnisse/Dateien bleiben linkfrei. So bleiben OS-Datenroot-Aliase wie Android filesDir
  nutzbar. Windows verwendet den gepinnten physischen V1-Elternpfad.
- Private Speicherhelfer sind für eigene private App-/Testverzeichnisse gedacht. Sie dürfen
  kein gemeinsames `/tmp` als privaten Elternordner einengen; betroffene TSV-Fixtures verwenden TempDir.
- Neue Sender announcen ausschließlich v2. Neue Empfänger können alte LAN-Hinweise wählen;
  alte Empfänger können neue private Kennungen nicht nach dem alten Stable-ID-Schema zuordnen.
  Persistierte Direct-/Room-/Server-Endpunkte und Node-Pins ändern sich nicht. Diese bewusst
  sicherere Wire-Grenze ist kein Anspruch vollständiger LAN-Mischversionskompatibilität.
- Windows-UAC muss unter demselben SID erfolgen. Alternative Administrator-Credentials werden
  abgelehnt, damit Task/Credentials nicht einem anderen Konto gehören. Beschädigte fremde oder
  verlinkte Installationsobjekte werden nicht rekursiv gelöscht. Eine unsichere eigene Root-DACL
  wird durch ein neues privates Verzeichnis ersetzt; die alte Root wird als `.retired-<uuid>`
  ohne Taskverweis quarantänisiert, statt alte schreibfähige Handles weiterzuverwenden.
- Kein eigener Manifest-/Lockfile-Eingriff; ring, base64, libc, getrandom und die bereits vom
  Hauptagent freigeschalteten Windows-Features genügen. Keine Builds/Tests/Installationen,
  keine Commits/Pushes/Graph-/Release-Arbeit durch diesen Subagenten.
- Eigener Self-Review: nur geänderte APIs/Dateien auf Scope, Handle-/Erstellungsreihenfolge,
  Retry-/Persistenzpfade und Registrierung geprüft. Tree-sitter-Rust, PowerShell
  Parser::ParseFile und ParseInput samt eingebettetem Systemmodul-Präfix ohne Parsefehler;
  Whitespace-Prüfung und git diff --check sauber. Programme nicht ausgeführt. Registry-Ausnahme:
  bestehendes `share/mod.rs` >500 Zeilen, ausschließlich eigene additive Einträge.

## Exakte Dateien

Gelesene Plan-/Quellenbelege (jeweils gezielte Bereiche):

- `AGENTS.md (Anweisungen im Auftrag)`
- `docs/ARCHITEKTUR.md`
- `docs/refs/local-fs-identity-durability.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/s-local.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/spec.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/umsetzung.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/recherche.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-sicherheit.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/integration.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/K1.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/V-LOCAL.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/A-CLIENT.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/S-REVOKE.md`

Gelesene, unveränderte Source-Dateien:

- `native/Cargo.toml`
- `native/src/creds/core/types.rs`
- `native/src/creds/os/linux_os.rs`
- `native/src/creds/os/linux_file_store.rs`
- `native/src/creds/os/windows.rs`
- `native/src/daemon/mod.rs`
- `native/src/daemon/os/shared/locks.rs`
- `native/src/daemon/os/shared/ipc.rs`
- `native/src/daemon/os/shared/ipc_protocol.rs`
- `native/src/share/core/crypto.rs`
- `native/src/share/core/identity.rs`
- `native/src/share/core/types.rs`
- `native/src/share/core/direct_relation.rs`
- `native/src/share/core/relation_rights.rs`
- `native/src/share/os/shared/profile_store.rs`
- `native/src/local_access/os/windows/private_security.rs`
- `native/src/local_access/os/windows/create.rs`
- `native/src/local_access/os/windows/directory_handle.rs`
- `native/src/net/os/windows.rs`
- `native/src/net/os/windows/interfaces.rs`
- `native/src/net/os/linux_os.rs`
- `native/src/net/core/link_facts.rs`
- `native/src/vfs/mod.rs`

Geänderte Source-Dateien (zugleich gelesen; Registrierungen nur eigene additive Einträge):

- `native/installer.nsi`
- `native/src/support_dirs.rs`
- `native/src/creds/mod.rs`
- `native/src/creds/os/shared.rs`
- `native/src/daemon/os/linux_os/ipc_storage.rs`
- `native/src/daemon/os/windows/ipc_storage.rs`
- `native/src/daemon/os/shared/ipc_listener.rs`
- `native/src/daemon/os/shared/lan_runtime.rs`
- `native/src/daemon/os/shared/lan_uplink_runtime.rs`
- `native/src/net/core/backend.rs`
- `native/src/net/core/uplink.rs`
- `native/src/net/mod.rs`
- `native/src/net/os/shared/uplink_state_store.rs`
- `native/src/net/os/windows/uplink_adapter.rs`
- `native/src/net/os/windows/uplink_helper.rs`
- `native/src/net/os/windows/ics.rs`
- `native/src/net/os/linux_os/uplink_adapter.rs`
- `native/src/net/os/linux_os/uplink_polkit.rs`
- `native/src/share/mod.rs`
- `native/src/share/core/lan_presence_match.rs`
- `native/src/share/core/lan_settings.rs`
- `native/src/share/core/lan_status.rs`
- `native/src/share/core/lan_uplink_policy.rs`
- `native/src/share/os/shared/identity_store.rs`
- `native/src/share/os/shared/direct_policy_store.rs`
- `native/src/share/os/shared/lan_presence.rs`
- `native/src/share/os/shared/lan_settings_store.rs`
- `native/src/share/os/linux_os/identity_lock.rs`
- `native/src/share/os/windows/identity_lock.rs`
- `native/src/share/os/windows/system.rs`
- `native/src/app/core/share_lan_ui.rs`
- `native/src/app/core/share_lan_uplink_ui.rs`

Neu erstellte kohäsive Feature-Dateien (anschließend gelesen/geparst):

- `native/src/creds/os/private_storage_unix.rs`
- `native/src/creds/os/private_storage_windows.rs`
- `native/src/creds/os/private_storage_tests.rs`
- `native/src/daemon/os/shared/ipc_admission.rs`
- `native/src/daemon/os/shared/ipc_listener_tests.rs`
- `native/src/daemon/os/shared/lan_runtime_presence.rs`
- `native/src/daemon/os/shared/lan_uplink_operations.rs`
- `native/src/net/os/windows/uplink_install.rs`
- `native/src/net/os/windows/uplink_install.ps1`
- `native/src/share/core/lan_privacy.rs`
- `native/src/share/os/shared/lan_presence_auth.rs`
- `native/src/share/os/shared/lan_uplink_evidence.rs`
- `native/src/share/os/shared/lan_permission_unavailable.rs`
- `native/src/share/os/shared/lan_permission_job.rs`
- `native/src/share/os/windows/lan_permission.rs`

Erstellte/aktualisierte Blockberichte:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/S-LOCAL.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/S-LOCAL.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/S-LOCAL.md`

Skills bereits geladen: `/root/.codex/skills/arbeitsweise/SKILL.md`, `/root/.codex/skills/graphify/SKILL.md`; keine eigene Graph-Abfrage/-Änderung in diesem Auftrag. Nicht vorhandenes `api-delta/V-LOCAL.md` bei früherem Leseversuch war kein Blocker; maßgeblicher ACL-Vertrag kam per Hauptagent/V-LOCAL-Handoff.
