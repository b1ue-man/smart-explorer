# Bestand: Gerät verbinden und Berechtigungen (Ist-Stand)

Stand: 2026-10-31. Quellstand: Git-HEAD `015ea1f9` (Commit „docs(plan): section F ...“). Zum Lesezeitpunkt gab `git status --short -- native android docs README.md` keine Ausgabe, also keine Änderungen in diesen Pfaden.

Zweck: Ist-Stand als Grundlage für die Nutzervorgabe vom 2026-10-31 (kompakter Verbindungsablauf für Geräte, Code, Direkt oder Raum; Berechtigungen erst nach Bestätigung; Voreinstellungen auf beiden Seiten). Dieses Dokument ändert kein Verhalten.

Konventionen:
- `N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`.
- Zeilenangaben beziehen sich auf den Quellstand oben.
- „nicht geprüft“ markiert jede Aussage ohne Beleg im Lesebereich.
- Annahmen sind mit „Annahme“ gekennzeichnet.

## 0. Zusammenfassung

1. Jeder Verbindungsweg startet einseitig. Der Anfragende stellt eine Anfrage (Direkt-Code) oder weist sich per PIN aus (Kopplung), und das Ergebnis hängt von der Gegenseite ab. Die PIN-Kopplung legt die Freigabe beim Veröffentlichenden ohne weiteren Schritt an (Abschnitt 2.3).
2. Ohne Bestätigung gesetzt werden heute:
   - (a) Bei jeder angenommenen Direkt-Freigabe Schreibrecht `true` und Befehle aus (`N/share/core/direct_relation.rs:46`, `N/share/core/legacy_direct_request_decision.rs:215`, `N/share/core/direct_ledger_projection.rs:135`).
   - (b) Beim Veröffentlichenden einer PIN-Kopplung sofort eine akzeptierte Freigabe mit Schreibrecht (`N/share/core/discovery_exchange_port_impl.rs:206`, `N/share/core/direct_reciprocal.rs:393-402`).
   - (c) Bei jedem Ordner in der Direkt-Liste Lesen und Schreiben, ohne Bestätigung in CLI und Desktop (`N/share/core/export_config.rs:75-82`, `N/cli/share/exports.rs:142-160`).
   - (d) Raum-Mitgliedschaft ohne Zulassung, solange „Neue Mitglieder bestätigen“ aus ist (`N/share/core/room_relation_members.rs:29-33`, `:198-215`).
   - (e) Bei Auto-Annahme eine Freigabe ohne Nutzerentscheidung (`N/share/core/legacy_direct_request_decision.rs:75-80`).
3. Die Ordnerliste der Direkt-Freigabe ist für alle Direkt-Geräte dieselbe (`N/share/core/access_relation_view.rs:118-124`). Je Gerät gibt es nur Schreibrecht, Befehle, Terminal, Konto, Ende, Programmliste und die Bildschirm- und Zusatzrechte.
4. Laut Code schließt „Bildschirm steuern: Erlauben“ ohne eigene Freigabe ein: Bildschirm ansehen, Programme starten, Maschinenanmeldung, eigene RDP-Dienste des Hosts und freigegebene Loopback-Dienste (`N/share/core/access_steering.rs:1-20`, `:132-150`).
5. Widerspruch: Der Warntext der Auto-Annahme sagt „Schreibrecht und Befehle werden dadurch nicht erlaubt“ (`N/app/core/share_policy_ui.rs:38`). Der Code setzt bei Auto-Annahme `write = true`; Befehle bleiben aus (`N/share/core/legacy_direct_request_decision.rs:215`).
6. Voreinstellungen: drei Befehls-Voreinstellungen (`N/share/core/access_grant.rs:460-491`), zwei Anfrage-Regeln (`N/share/core/direct_relation.rs:154-160`) und die Standardwerte aus Abschnitt 6. Für Ordner, Schreibrecht und Bildschirm gibt es keine benannten Voreinstellungen.
7. Android hat keine Voreinstellungen. Konto nur „App-Konto“ (`N/mobile/os/shared/domains/share_access.rs:313-321`). Die Rechtezeilen enthalten Apps, Anmeldung und TCP nicht (`N/mobile/os/shared/domains/share_access.rs:77-87`).
8. `docs/oberflaechen/share-exec.md` (Stand 2026-10-21): alle 97 darin genannten Quelldateien existieren. Von 127 Zeilenbelegen zeigen mindestens 40 auf eine Zeile mit anderem Inhalt. Mehrere Belege zeigen auf falsche Dateien (Abschnitt 7).

## 1. Begriffe und Bausteine

| Begriff | Form | Stelle | Wirkung |
|---|---|---|---|
| Direkt-Code | `SE-D3-<lookup_id>-<direct_secret, 32 Byte hex>-<fingerprint>-<node_id>` | `N/share/core/identity.rs:186-194` | Enthält das Geheimnis `direct_secret`. Die Auto-Annahme prüft nur den Besitz dieses Geheimnisses (Quelle `AuthenticatedSecretPossession`, `N/share/core/legacy_direct_request_decision.rs:75-80`). |
| Raum-Code | `SE-R3-<room_id>-<secret>` | `N/share/core/profiles.rs:170-176` (Erzeugung), `:266` (Lesen) | Beitrittsgeheimnis des Raums. |
| PIN der Kopplung | sechs Ziffern vorgeschlagen, eigene PIN exakt als UTF-8-Bytes | `N/share/core/discovery_pin.rs:1-5`, `N/share/core/discovery_signal_types.rs:97` | Nachweis im Kopplungsaustausch. Das Angebot endet nach der ersten Kopplung oder nach fünf Fehlversuchen. |
| Angebot (Discovery) | Ziel Direkt oder Raum, Anzeigename, PIN, Dauer | `N/share/core/discovery_signal_types.rs:70-76` (Ziel), `:157-170` (Befehl `Publish`) | Auffindbar für Suchende. Höchstens 30 Minuten (`N/cli/share/discoverable.rs:79-84`). |
| Anfrage (Direkt) | signierter Datensatz: Anfrage-ID, Anfragender, Ziel, Ablauf, Nonce, optionale Nachricht, HMAC, Signatur | `N/share/core/direct_protocol.rs:160-171` | Enthält keine Rechteangabe. Der Empfänger entscheidet. |
| Entscheidung | `Accepted`, `Rejected`, `Revoked` | `N/share/core/direct_protocol.rs:143-147`, `:189-202` | Enthält keine Rechteangabe. |
| Kontakt (ausgehend) | `DirectContact`: Gegenstelle, die dieses Gerät anspricht | `N/share/core/direct_relation.rs:101-135` | `access_state` (Pending, Accepted, Ignored, IdentityConflict; `:14-19`), `auto_connect`, `auto_open`, `relation.share_back`. |
| Freigabe (eingehend) | `DirectGrant`: Gegenstelle darf auf diese Freigaben zugreifen | `N/share/core/direct_relation.rs:63-80` | `state` (Accepted, Ignored, Reconfirm; `:49-61`), `exec` (Befehle), `write`. |
| Zugriffsrecht | `AccessGrant`: Konto, je Fähigkeit Off, Ask oder Allow, Constraints, Ende | `N/share/core/access_grant.rs:230-238` | Die Kernform der Befehls-, Bildschirm- und Zusatzrechte. |
| Raum-Profil, Mitglied, Raumpolicy | `RoomProfile`, `RoomMember`, `RoomPolicy` | `N/share/core/types.rs:108` (`RoomMember`), `:134` (`RoomProfile`); `N/share/core/room_relation_members.rs:14-24` (`RoomPolicy`) | Raumexporte, Mitgliederzulassung, Schreibrecht der Mitglieder. |

Gespeicherte Dateien, je Speicher-Modul:
- `share_profiles.json` (`N/share/os/shared/profile_store.rs:20`): Kontakte, Freigaben, Räume, Exporte.
- `share_identity.json` (`N/share/os/shared/identity_store.rs:10`): Identität und Direkt-Code.
- `share_direct_policy.json` (`N/share/os/shared/direct_policy_store.rs:11`): Anfrage-Regel.
- `lan_settings.json` (`N/share/os/shared/lan_settings_store.rs:6`).
- `exec-audit.log`, `exec-audit-rejected.log` (`N/share/os/shared/exec_audit_files.rs:35-36`).

Der vollständige Speicherpfad je Datei ist nicht geprüft (Ablage über `crate::support_dirs`, Funktion nicht gelesen).

## 2. Punkt 1: Wege, ein Gerät zu verbinden

### 2.1 Übersicht

Vollständige Liste der Wege, die im Lesebereich gefunden wurden:

| Nr | Weg | Auslöser | Ergebnis | Rechte beim Ergebnis (Abschnitt 3) |
|---|---|---|---|---|
| W1 | Direkt-Code, Anfrage | Anfragender fügt den Direkt-Code des anderen hinzu | Kontakt beim Anfragenden, Anfrage bei der Gegenstelle, danach Entscheidung | Bei Annahme Freigabe mit Schreibrecht, Befehle aus |
| W2 | PIN-Kopplung Gerät | Veröffentlichender macht Gerät auffindbar, Verbindender gibt PIN ein | Kontakt und Freigabe je nach Rückfreigabe-Häkchen | Veröffentlichender: Freigabe ohne Bestätigung. Verbindender: nur mit Häkchen |
| W3 | Raum per Code | Raum erstellen (Besitzer) oder beitreten (per Raum-Code) | Raum-Profil, Mitgliedschaft | Mitglied ohne Zulassung, sofern Bestätigung aus; Schreibrecht aus; Befehle aus |
| W4 | Raum per PIN | Raum veröffentlichen, dann PIN-Kopplung | Raum-Profil beim Beitretenden | Wie W3 |
| W5 | LAN-Erkennung | Schalter `se share lan presence` | nicht als Kopplung belegt | Frage, Abschnitt 9 |
| W6 | Wiedereinsetzen entfernter Geräte | „Erneut zulassen“ | Entfernungssperre aufgehoben | Freigabe wie bei W1 bzw. W2 |
| W7 | Hintergrund-Reparatur | automatisch, bei Online-Beziehungen | fehlende Gegenbeziehung wird ergänzt | Nur bestehende Beziehungen, siehe 2.5 |

Abgrenzung: Speicherverbindungen (SFTP, FTP, FTPS, WebDAV, SMB, UNC, Google Drive) sind keine Geräteverbindungen. Sie werden über `se connections add …` angelegt (Befehl `add` in `N/cli/connections.rs`). Quick Share ist laut `docs/oberflaechen/share-exec.md` eine eigene Funktion. `N/app/core/quickshare_ui.rs` wurde nicht gelesen (nicht geprüft).

### 2.2 W1: Direkt-Code (Anfrage)

Desktop („Geräte & Freigaben“):
1. Tab „Geräte“, Schaltfläche „Gerät hinzufügen …“ (`N/app/core/share_device_page_ui.rs:134`).
2. Seite „Gerät hinzufügen“ (`share_device_page_ui.rs:25`), Karte „Per Direkt-Code“ (`:40`).
3. `ui_share_add_device` (`N/app/core/share_direct_ui.rs:7`): Checkbox „Auch meine Freigaben fuer dieses Geraet oeffnen“ (`:14`, Standard `false`, Zeile 8-10), Feld Direkt-Code, Feld Name.
4. Schaltfläche „Hinzufuegen“: `add_direct_from_code_persisted` (`:40`), dann `set_direct_share_back` (`:47`), dann `queue_contact` (`:61`). Die Anfrage wird damit gespeichert und zur Zustellung vorgemerkt (`N/app/core/share_lifecycle_ui.rs:118-140`).
5. Zweiter Weg auf derselben Seite: „Zugriff anfragen“ auf der Gerätseite (`N/app/core/share_device_connection_ui.rs:34`) ruft `queue_contact` auf.

CLI:
- `se connections add-peer --code SE-D3-… [--name NAME] [--no-request] [--json]`. Die Hilfe sagt: die Einrichtung ist einseitig; die Gegenseite bestätigt mit `se share status` und `se share request accept` oder in der GUI (`N/cli/connections.rs:37-40`). Ohne `--no-request` wird die Anfrage gestellt (`N/cli/connections.rs:189-207`, Aufruf `setup::add_peer` in `:269-270`).
- Implementierung von `setup::add_peer`: `N/cli/setup/share_peer.rs:114`. Nicht gelesen, siehe Abschnitt 11.

Android:
1. „Direct-Code hinzufügen“ (`A/ui/share/ShareScreen.kt:379-382`) öffnet `ShareDialog.AddDirect` (`A/ui/share/ShareDialogHost.kt:183-193`).
2. Dialog `CodeDialog` (`A/ui/share/ShareDialogs.kt:86`) mit Checkbox „Auch meine Freigaben für dieses Gerät öffnen“ (`:121-122`, Standard `false`, `:98`).
3. `ShareApi.addDirect(code, name, shareBack)` (`A/api/ShareApi.kt:290`) → Bridge `share.addDirect` (`N/mobile/os/shared/domains/mod.rs:261`) → `add_direct` (`N/mobile/os/shared/domains/share_peers.rs:50`). Die Bridge legt den Kontakt an und setzt Rückfreigabe, sendet aber keine Anfrage.
4. Die Anfrage ist ein separater Schritt: Gerätekarte „Zugriff anfragen …“ (`A/ui/share/DeviceSettingsPage.kt:114`) oder Menü „Zugriff anfragen“ (`A/ui/share/SharePeers.kt:123`) öffnet `ShareDialog.RequestAccess` mit optionaler Nachricht (`A/ui/share/ShareDialogHost.kt:196-205`) → `share.requestAccess` (`mod.rs:282`, `share_requests.rs:29`).

Zustände des Kontakts (`N/share/core/direct_relation.rs:14-29`): „Warte auf Freigabe“ (Pending), „Freigegeben“ (Accepted), „Ignoriert“ (Ignored), „Identitaetskonflikt“ (IdentityConflict). Die Projektion der signierten Entscheidung auf den ausgehenden Kontakt beginnt in `N/share/core/direct_ledger_projection.rs:42` (Funktion `project_outgoing_decision`; der übrige Funktionsrumpf ist nicht im Einzelnen gelesen).

Empfänger-Seite (Entscheidung):
- Desktop, Tab „Anfragen“ (`N/app/core/share_lifecycle_ui.rs:45`): Checkbox „Angezeigten Fingerprint geprueft“ (ab Zeile 203), dann „Akzeptieren“ (`:218`), „Ablehnen“ (`:228`), „Freigaben waehlen“ (`:235`, öffnet den Tab Freigaben, `N/app/core/share_lifecycle_actions_ui.rs:88-92`). Ohne Fingerprint-Häkchen ist Akzeptieren gesperrt (`:217`).
- Android, Abschnitt „Anfragen“ (`A/ui/share/ShareLists.kt:41-62`): „Annehmen“ (`:61`) ruft `actions.decide(request, true)` direkt auf. Im gelesenen Code gibt es dort keinen Fingerprint-Schritt. Desktop und Android verhalten sich damit unterschiedlich (Frage, Abschnitt 9).
- CLI: `se share request accept [SELECTOR] [--fingerprint FP]` und `se share request reject` (`N/cli/share/requests.rs:29-47`). Die Accept-Argumente enthalten keine Rechteoption.
- Bridge: `share.decide` (`mod.rs:283`, `share_requests.rs:47`).

Ergebnis bei Annahme (Empfänger): `DirectGrant` mit `state = Accepted`, `write = NEW_DIRECT_GRANT_WRITE` (= `true`), `exec` = Standard, also Befehle aus (`N/share/core/legacy_direct_request_decision.rs:161-217`, Neuanlage in `:215`; `N/share/core/direct_ledger_projection.rs:72-135`, Neuanlage in `:135`). Keine Ordnerauswahl: Die Freigabe erhält die Direkt-Ordnerliste (Abschnitt 3, R1).

Auto-Annahme (Option „Anfragen mit meinem Direkt-Code automatisch annehmen (unsicherer)“):
- Entscheidungslogik: `N/share/core/legacy_direct_request_decision.rs:75-80`, Quelle `AuthenticatedSecretPossession`, `install_grant: true`.
- Die Freigabe wird ohne Nutzerschritt angelegt (`N/share/core/legacy_direct_request_mutations.rs:84`, `:110`).
- Sperre für Auto-Annahme: ignoriertes Gerät, ignorierter Kontakt, abgelehnte oder widerrufene Anfrage, Sperr-Tombstone (`N/share/core/legacy_direct_request.rs:206-224`).
- Desktop-Schalter: `N/app/core/share_policy_ui.rs:17-33`. Warntext: `:38`.
- CLI: `se share request policy auto-accept` (`N/cli/share/requests_policy.rs:17-27`). Bridge: `share.setPolicy` (`mod.rs:281`, `share_policy.rs:283-292`).

Einseitigkeit: Der Anfragende erhält durch Annahme nur Zugriff auf die Freigaben des Empfängers. Der Empfänger erhält Zugriff auf Freigaben des Anfragenden nur über die Rückfreigabe des Anfragenden (`share_back`, Abschnitt 3, R20) oder über eine eigene Anfrage. Die Hintergrund-Reparatur (2.5) öffnet Exporte nur, wenn der Kontakt Rückfreigabe hat oder die Freigabe bereits akzeptiert ist (`N/share/core/direct_reciprocal.rs:274-281`).

### 2.3 W2: PIN-Kopplung Gerät

Veröffentlichen (Anbieter):
- Desktop: „Gerät hinzufügen“ → Karte „Per PIN“ (`N/app/core/share_device_page_ui.rs:32`) → `ui_share_discovery_direct` (`N/app/core/share_discovery_ui.rs:14`) → Veröffentlichungsbereich: Dauer (`N/app/core/share_discovery_publish_ui.rs:41`), PIN (`:49`), „Neue PIN“ (`:58`), „Unsichere PIN erlauben“ (`:107`). Hinweis „Höchstens 30 Minuten“ (`:48`).
- Android: „Suchbar machen“ (`A/ui/share/ShareDeviceCard.kt:106`, `A/ui/share/ShareScreen.kt:286`) → `ShareDialog.Discoverable` (`A/ui/share/ShareDialogHost.kt:140-150`) → `ShareApi.discoverable` (`A/api/ShareApi.kt:248`) → `share.discoverable` (`mod.rs:256`, `share_settings.rs:215`).
- CLI: `se share discoverable [--minutes N] [--pin PIN | --pin-stdin | --pin-prompt | --random-pin] [--allow-weak-pin] [--room ROOM] [--name NAME] [--json]`. Standard 5 Minuten, Bereich 1 bis 30 (`N/cli/share/discoverable.rs:79-84`). Hilfetext: Konstante `DISCOVERABLE_HELP` in derselben Datei. Die Bestätigung durch den Share-Server wird bis 10 Sekunden erwartet (`:22`). Ausgabe: `prepared` oder `published` (README, Zeilen 506-520).
- Daemon-IPC: `ShareCmd::Discovery(DiscoveryCommand::Publish {target, display_alias, pin, duration_secs, allow_weak_pin})` (`N/share/core/discovery_signal_types.rs:157-170`).

Kein Rechtefeld: Das Angebot enthält Ziel, Anzeigename, PIN, Dauer und die Schwachen-PIN-Option. Rechte werden nicht gewählt (`N/share/core/discovery_signal_types.rs:157-170`).

Verbinden (Suchender):
- CLI: `se share discover list` (Liste) und `se share discover pair OFFER (--pin-stdin | --pin-prompt) [--share-back] [--json]`. Die Hilfe sagt: Die Rückfreigabe ist nur mit `--share-back` (`N/cli/share/discover.rs:35-52`, `:65-95`). Aufruf `crate::daemon::start_discovery_pairing(offer, pin, share_back)` in `pair()` (`:65-95`), weiter über IPC (Abschnitt 11).
- Desktop: Liste (`N/app/core/share_discovery_ui.rs:181`), PIN-Feld (`:241`), Checkbox „Beim Verbinden auch meine Freigaben für das Gerät öffnen“ (`:211-212`), Schaltfläche „Verbinden“ aktiv nur bei kompatiblem Angebot, ohne laufenden Vorgang und mit gültiger PIN (`:255`). Die Checkbox wird nach jedem Verbinden zurückgesetzt (`:267-269`). Abbrechen (`:283`).
- Android: „Gerät verbinden“ (`A/ui/share/ShareScreen.kt:326`) → `ShareDialog.Connect` → `ConnectDeviceDialog` (`A/ui/share/ShareConnectDialog.kt:50`) → `ShareApi.discover()` (`:73`, `A/api/ShareApi.kt:266`) → Angebotsliste (`:264`) → Auswahl → PIN-Feld „PIN des anderen Geräts“ (`:179`) → Checkbox „Auch meine Freigaben für dieses Gerät öffnen“ (`:185-187`, Standard `false`) → „Verbinden“ (`:244`) → `ShareApi.connect(discoveryId, pin, shareBack)` (`:233`, `A/api/ShareApi.kt:272`) → `share.connect` (`mod.rs:259`, `share_settings.rs:296`).
- Android-Ende ohne klares Ergebnis: „Unter Teilen entscheiden“ (`:220`) öffnet die Liste unbestätigter Kopplungen (`A/ui/share/ShareConfirmationDialogs.kt:93`, `PairingDecisionDialog`).

Protokoll (Beleg aus dem Code und aus `docs/SHARE_SERVER.md:112-168`): Der Austausch nutzt OPAQUE-PAKE mit der PIN (`N/share/core/discovery_pake.rs`, nur Namen gelesen). Der Share-Server leitet nur Pakete weiter und erhält PIN, Beziehungsgeheimnis und Schlüssel nicht (`docs/SHARE_SERVER.md:138-142`). Das Paket enthält das Direkt-Material als `PairingBundle` mit `DiscoveryKind::Direct` (`N/share/core/discovery_domain.rs:176-262`, `N/share/core/discovery_signal_types.rs:13-17`).

Zustände des Angebots (`N/share/core/discovery_signal_types.rs:245-258`): Requested, Expired, TargetUnavailable, CapabilityUnavailable, TransportError, WorkerStopped, Paired, TooManyFailedAttempts.

Zustände des Austauschs (`N/share/core/discovery_signal_outcome.rs:27-62`): ExchangeUnconfirmed (Beziehung gespeichert, Gegenbestätigung fehlt), ExchangeCancelled, ExchangeFailed.

Ergebnis beim Veröffentlichenden: `persist_direct(&peer, PairingOrigin::UserPairing)` (`N/share/core/discovery_exchange_port_impl.rs:206`). `UserPairing` heißt: „Deliberate pairing that also opens this device's exports to the peer: the publisher of a PIN offer“ (`N/share/core/removed_direct_peers.rs:60-64`). Folge: Kontakt mit `share_back = true` (`N/share/core/direct_reciprocal.rs:308`, `:364`), Freigabe `Accepted` mit `write = true` und Befehlen aus (`N/share/core/direct_reciprocal.rs:393-402`). Keine Rückfrage an den Veröffentlichenden. Bestätigt durch die Dokumentation in `docs/SHARE_SERVER.md:156`: „Neither flow requires a second click or approval on the publishing side.“

Ergebnis beim Verbindenden: Der Verbindende wählt `UserPairing` nur, wenn die Rückfreigabe gewählt ist. Sonst gilt `UserPairingOneWay` (`N/share/core/discovery_exchange_port_impl.rs:417-421`). `UserPairingOneWay` legt keine Freigabe an oder aktiviert keine (`N/share/core/removed_direct_peers.rs:66-70`, `N/share/core/direct_reciprocal.rs:275-276`). Der Verbindende erhält also nur den Kontakt. Der Veröffentlichende hat in diesem Fall bereits eine Freigabe (siehe oben).

Nur im Fehlerfall: „Unbestätigte Kopplung“ mit Schaltflächen „Widerrufen“ und „Behalten“ (`N/app/core/share_discovery_publish_ui.rs:166-196`). Text: „Gekoppelt mit {Name} – Bestätigung fehlt; bei Zweifel widerrufen“ (`N/share/os/shared/discovery_pairing_ui.rs:62-70`). „Widerrufen“ ist nicht möglich für übergebene Raum-Daten (`:78-80`).

### 2.4 W3 und W4: Räume

Raum erstellen:
- Desktop: „Raum erstellen“ (`N/app/core/share_rooms_ui.rs:22-58`), Schaltfläche „Neuen Code“ (`:38`), „Code kopieren“ (`:41`), „Raum erstellen“ (`:44`).
- Android: „Raum erstellen“ (`A/ui/share/ShareScreen.kt:327`) → `ShareDialog.CreateRoom` (`A/ui/share/ShareDialogHost.kt:157`) → `ShareApi.createRoom` (`A/api/ShareApi.kt:306`) → `share.createRoom` (`mod.rs:264`, `share_peers.rs:112`). Die Bridge gibt Profil-ID und Raum-Code zurück (`share_peers.rs:112-119`).
- CLI: `se share room create` (`N/cli/share.rs:88`, Befehlsgruppe „Create, list, join, leave, rename and remove rooms; admit or block members“). Die Argumente in `N/cli/share/rooms.rs` wurden nicht gelesen (nicht geprüft).
- Ergebnis: Raum-Profil mit `RoomPolicy::new_room()`: `members_may_write = false`, `confirm_new_members = false` (`N/share/core/room_relation_members.rs:29-33`). Raum-Exporte gelten als nur lesend (Abschnitt 3, R2).

Raum beitreten per Code:
- Desktop: „Raum per Code beitreten“ (`N/app/core/share_rooms_ui.rs:64`), „Einfuegen“ (`:85`), „Beitreten“ (`:91`).
- Android: „Raum beitreten“ (`A/ui/share/ShareScreen.kt:328`) → `ShareDialog.JoinRoom` (`A/ui/share/ShareDialogHost.kt:172`) → `ShareApi.joinRoom` → `share.joinRoom` (`mod.rs:265`, `share_peers.rs:123`).
- CLI: `se connections add-room --code SE-R3-… [--name NAME]` (`N/cli/connections.rs:91`, Aufruf `setup::add_room` in `:274-275`; Implementierung nicht gelesen).
- Ergebnis: Mitglied wird zugelassen (`RoomMemberAdmission::Admitted`), sofern die Raumpolicy keine Bestätigung verlangt (`N/share/core/room_relation_members.rs:198-215`; Bestätigung verlangt, wenn `confirm_new_members` gesetzt ist oder ein Mitglied gesperrt wurde, `:91-96`). Neue Mitglieder erhalten `ExecGrant::default()` (Befehle aus, `:198-215`).
- Verlassen: `auto_join = false`, dann `ShareCmd::LeaveRoom` (`N/mobile/os/shared/domains/share_peers.rs`, Funktion `leave_room`). Die Raum-Daten bleiben erhalten; „Wieder beitreten“ ist möglich (`N/app/core/share_room_page_ui.rs:101`, `:109`).

Raum per PIN (W4):
- Veröffentlichen: CLI `se share discoverable --room ROOM` (`N/cli/share/discoverable.rs:65-71`). Desktop: Abschnitt „Räume entdecken und per PIN beitreten“ (`N/app/core/share_rooms_ui.rs:17`, Funktion `ui_share_discovery_rooms` in `share_discovery_ui.rs:44`). Android: `ShareDialog.Discoverable` mit Raumziel.
- Beitreten: derselbe Austausch wie W2, mit `DiscoveryKind::Room` und dem Raum-Material (`N/share/core/discovery_domain.rs:264-298` für Rolle und Stufe).
- Android-Text: „Raumbeitritt legt keine eigenen Freigaben an.“ (`A/ui/share/ShareConnectDialog.kt:189`).
- Nicht geprüft: Ergebnis beim Veröffentlichenden (Raum-Mitglied anlegen) im Austauschcode `discovery_exchange_port_impl.rs`, Bereich nach Zeile 220.

### 2.5 W5, W6, W7 und Oberflächen ohne eigene Kopplung

W5 LAN-Erkennung: CLI `se share lan presence` (`N/cli/share/lan.rs:22`). Desktop Tab „Netzwerk“ (`N/app/core/share_lan_ui.rs`, Zeilen nicht im Einzelnen geprüft). Android zeigt den Zustand nur an (`N/mobile/os/shared/domains/share_status.rs:301-313`, `lanPresence`). Das Ereignis `ShareCmd::LanPeerSeen` ist definiert (`N/share/core/types.rs:405`). Ob es Kontakte oder Rechte anlegt, ist nicht belegt (Abschnitt 9).

W6 Entfernte Geräte: Desktop „Entfernte Geräte“ mit dem Hinweis „Kopplung (PIN, Direkt-Code oder Annehmen einer Anfrage) hebt die Sperre auf“ (`N/app/core/share_removed_devices_ui.rs:11`). CLI `se share grants removed [--readmit GERÄT]` (`N/cli/share/grants.rs:45-47`, `:68-75`). Android `ShareDialog.Readmit` (`A/ui/share/ShareDialogHost.kt:62`) → `share.readmit` (`mod.rs:263`, `share_peers.rs`). Ein entferntes Gerät kommt nicht über Hintergrund-Reparatur zurück (`N/share/core/direct_reciprocal.rs:209-225`, Funktion `apply_reciprocal_direct_peer`).

W7 Hintergrund-Reparatur: nur bei gültigem Kontakt, bei Online-Beziehungen. Sie ergänzt die Gegenbeziehung (`N/share/core/direct_reciprocal.rs:265-281`, Fall `AutomaticRepair`). Sie ist in `docs/SHARE_SERVER.md:412-420` dokumentiert.

Oberflächen ohne eigene Kopplungsfunktion:
- MCP-Werkzeug `se`: führt jeden CLI-Befehl aus (`N/cli/mcp/catalog.rs:183-184`, `N/cli/mcp/cli_tool.rs:1-6`). Richtlinie: „Change Share trust (access requests, Exec grants and their program lists, device and room settings, discoverability) only when the user explicitly asks“ (`N/cli/mcp/catalog.rs:39-41`). Gesperrt: nur `se share exec approve|deny` und `se share exec accounts setup|allow|disallow|update|remove` (`N/cli/share/exec_approvals.rs:232-264`). Nicht gesperrt im Code: `se share request accept|reject`, `se connections add-peer`, `se share discoverable`.
- Daemon-IPC: `IpcRequest::ShareCommand { token, cmd: ShareCmd }` (`N/daemon/os/shared/ipc_protocol.rs:128-131`), `IpcRequest::DiscoveryPairing { token, request }` (`:107-110`; Typ `DiscoveryPairingRequest` in `discovery_pairing_protocol.rs`, nicht gelesen), `IpcRequest::MutateExecGrant { token, target, enabled, settings }` (`:132-141`), `IpcRequest::Access { token, request: AccessIpcRequest }` (`:215-218`; Typ in `access_ipc.rs`, nicht gelesen). Antworten: `ShareCommandReply` (`:89-101`), `IpcResponse::DiscoveryPairing` (`:331-334`), `IpcResponse::ShareCommand` (`:365-368`).
- `ShareCmd` (`N/share/core/types.rs:282-357`): Varianten mit Bezug zu diesem Abschnitt: `RequestDirect`, `AnswerLegacyDirectRequest`, `Discovery(DiscoveryCommand)`, `SetDirectOnline`, `EnableExec`, `DisableExec`, `ApplyExecGrant`, `LeaveRoom`, `ConfigureProfiles`, `Configure`, `SyncDirectRequests`, `Refresh`, `Stop`, `UpdateRuntime`. Die Zeilen der einzelnen Varianten sind nicht je Variante notiert; die Liste ist aus den Zeilen 282-357 vollständig übernommen.
- `DiscoveryCommand` (`N/share/core/discovery_signal_types.rs:157-…`): `Publish`, `StopPublishing`, `ListDiscoveries`, `StartDiscoveryExchange`, `StartOwnedDiscoveryExchange` (nur intern, `#[serde(skip)]`), `CancelDiscoveryExchange`. PIN wird im Debug-Ausdruck geschwärzt.
- Bridge (Android): 60 `share.*`-Methoden in `N/mobile/os/shared/domains/mod.rs:245-304` (Zeile 245 bis 304, je eine Methode pro Zeile). Verbindungsrelevant: `discoverable` (256), `stopDiscoverable` (257), `discover` (258), `connect` (259), `cancelConnect` (260), `suggestPin` (251), `unconfirmedPairings` (252), `resolvePairing` (253), `addDirect` (261), `removeDevice` (262), `readmit` (263), `createRoom` (264), `joinRoom` (265), `roomCode` (266), `leaveRoom` (267), `removeRoom` (268), `requestAccess` (282), `decide` (283), `retry` (284), `deleteRequest` (285).

## 3. Punkt 2: Rechte und Datenmodell

Die folgende Liste umfasst alle Rechte und Berechtigungsarten, die in den gelesenen Dateien als Feld, Enum-Variante oder Schalter vorkommen.

| Nr | Recht | Modell (Datei:Zeile) | Standard bei neuer Freigabe | Ohne Bestätigung gesetzt? | Änderung |
|---|---|---|---|---|---|
| R1 | Ordner-Freigabe (Liste) | `SharedRoot {label, path, access, allow_system_writes}`, `N/share/core/export_config.rs:39-48`; Direkt: `ShareProfiles.default_direct_exports` (`N/share/core/profiles.rs:47`); Raum: `RoomProfile.exports` | Eintrag wird angelegt | Ja: CLI (`N/cli/share/exports.rs:142-160`), Desktop (`N/app/core/share_exports_ui.rs:240`, `:248`, „Alle Laufwerke hinzufuegen“ `:235`). Android fragt nach (`A/ui/share/ShareDialogHost.kt:251-277`). | CLI `se share export add/remove/set/move`; Desktop Tab Freigaben (`share_exports_ui.rs:38`); Android „Rechte…“ (`A/ui/share/ShareLists.kt:105`) |
| R2 | Lesen oder Lesen und Schreiben je Ordner | `ExportAccess::ReadOnly` (Standard) und `ReadWrite` (`N/share/core/export_config.rs:11-27`) | Direkt-Scope: `ReadWrite` (`:75-82`). Raum-Scope: `ReadOnly` | Ja für Direkt-Scope | CLI `se share export set ORDNER --write|--read-only` (`N/cli/share/exports_policy.rs:8-29`); Desktop Auswahl „Nur lesen“ / „Lesen und Schreiben“ (`N/app/core/share_exports_ui.rs:117-119`); Android: `ExportAccessDialog` |
| R3 | Systemorte schreiben (Autostart, Anmeldung, Schlüssel) | `SharedRoot.allow_system_writes`, Standard `false` (`N/share/core/export_config.rs:48`) | `false` | Nein | CLI `--allow-system-writes` und `--protect-system-files` (`N/cli/share/exports_policy.rs:18-24`); Desktop Spalte „Systemorte“ (`N/app/core/share_exports_ui.rs:82`) |
| R4 | Schreibrecht je Gerät | `DirectGrant.write: bool` (`N/share/core/direct_relation.rs:78-79`), Konstante `NEW_DIRECT_GRANT_WRITE = true` (`:46`) | `true` (neue Freigaben, Annahme, PIN-Veröffentlichung, Auto-Annahme) | Ja | CLI `se share grants set GERÄT --write|--read-only` (`N/cli/share/grants_write.rs:6-14`); Desktop „Darf in meine Freigaben schreiben“ (`N/app/core/share_device_page_ui.rs:358-363`); Android `AllowGrantDialog` (`A/ui/share/ShareConfirmationDialogs.kt:32`), `ContactWriteDialog` (Datei nicht im Detail geprüft) |
| R5 | Schreibrecht der Raum-Mitglieder | `RoomPolicy.members_may_write` (`N/share/core/room_relation_members.rs:14-24`) | `false` (`:29-33`); alte Räume `true` (`:37-41`) | Nein | CLI `se share export policy --room R --write|--read-only` (`N/cli/share/exports_policy.rs:31-41`); Android „Rechte und Bestätigung…“ (`A/ui/share/SharePeers.kt:176`); Desktop: Schalter nicht gefunden (Abschnitt 9) |
| R6 | Mitglied zugelassen oder gesperrt | `RoomMemberFlags.admission` (`Admitted`, `Pending`) und `blocked` (`N/share/core/room_relation_members.rs:47-62`) | `Admitted` (`:47-52`) | Ja, sofern keine Bestätigung verlangt wird (`:198-215`) | Desktop „Zulassen“, „Wieder zulassen“, „Sperren“ (`N/app/core/share_room_page_ui.rs:247-254`); Android „Zulassen…“, „Wieder zulassen…“ (`A/ui/share/SharePeers.kt:219-220`); CLI `se share room member RAUM GERÄT admit|block|allow` (README, Zeile 795) |
| R7 | Neue Mitglieder bestätigen | `RoomPolicy.confirm_new_members` (`N/share/core/room_relation_members.rs:24`), Erzwingung bei gesperrtem Mitglied (`:91-96`) | `false` (`:33`) | Nein | CLI `se share export policy --confirm-new-members true|false` (`N/cli/share/exports_policy.rs:42-48`) |
| R8 | Befehle (argv und Shell) | `Capability::ExecCommands` (`N/share/core/access_grant.rs:27`), Entscheidung `CapabilityDecision` Off, Ask, Allow (`:100-108`) | Off (`:244`) | Nein | CLI `se share grants access GERÄT --commands off|ask|allow` (`N/cli/share/grants_access.rs:43-84`); Desktop Karte „Rechte“ (`N/app/core/share_access_controls_ui.rs:17`); Android Rechtezeilen (`A/ui/share/AccessDialog.kt:325-330`) |
| R9 | Terminal | `Capability::ExecTerminal` (`N/share/core/access_grant.rs:29`); bei gesetzter Programmliste aus (`:388-393`) | Off | Nein | wie R8 (`--terminal`) |
| R10 | Konto, unter dem Befehle laufen | `AccountProfile::{Own, Remote, Account{name}}` (`N/share/core/access_grant.rs:150-159`); `REMOTE_ACCOUNT_NAME = "se-remote"` (`:17`) | `Own` (`:153`) | Nein | CLI `--as own|remote|account:NAME` (`N/cli/share/grants_access.rs:56-57`); Desktop Konto-Auswahl (`N/app/core/share_access_dialog_ui.rs:60-68`); Android nur `Own` (`N/mobile/os/shared/domains/share_access.rs:313-321`, Meldung `:318`) |
| R11 | Programmliste | `ExecGrantSettings.programs` (`N/share/core/exec_policy.rs:50-60`), `ExecGrant.programs` (`:123`) | leer = volle Shell | Nein | CLI `--allow-program PFAD`, `--full` (`N/cli/share/grants_access.rs:73-78`); Desktop (`N/app/core/share_exec_programs_ui.rs`); Android ⋮ „Programme…“ (`A/ui/share/ExecProgramsDialog.kt:69`) |
| R12 | Bildschirm ansehen | `Capability::ScreenView` (`N/share/core/access_grant.rs:31`) | Off | Nein; bei „Bildschirm steuern: Erlauben“ implizit ja (`N/share/core/access_steering.rs:137-139`) | CLI `--screen-view`, nur in grafischer Sitzung (`N/share/core/access_catalog.rs:64-88`, Zeilen aus `screen`, nicht gelesen); Desktop Tabelle „Rechte“; Android Zeile per `capabilities` |
| R13 | Bildschirm steuern | `Capability::ScreenControl` (`N/share/core/access_grant.rs:33`) | Off | Nein. Erlauben schließt R12, R14, R15, R16 ein (`N/share/core/access_steering.rs:1-20`, `:132-150`) | CLI `--screen-control`; Desktop/Android wie R12 |
| R14 | Apps starten | `Capability::AppLaunch` (`N/share/core/access_grant.rs:35`), `AppLaunchConstraints {app_ids, sessions, users, valid_until}` (`N/share/core/access_constraints.rs:164-170`) | Off; ohne Constraints ungültig (`N/share/core/access_grant.rs:404-413`) | Nein; bei Bildschirm steuern implizit (`access_steering.rs:142-146`) | Desktop „Programme“ auf dem Gerät (`N/app/core/share_device_page_ui.rs:392-393`); CLI nur wenn im Katalog verfügbar (`N/share/core/access_catalog.rs:90-126`); nicht im Android-Katalog (`N/mobile/os/shared/domains/share_access.rs:77-87`) |
| R15 | Anmeldezugang (Maschinenanmeldung) | `Capability::LoginAccess` (`N/share/core/access_grant.rs:37`), `LoginAccessConstraints {sessions, users, operations, valid_until}` (`N/share/core/access_constraints.rs:173-179`), `LoginOperation::{Login, Unlock, SecureDesktop, UserSession}` (`:155-160`) | Off | Nein; bei Bildschirm steuern implizit (`access_steering.rs:142-146`) | Desktop „Maschinenanmeldung“ (`N/app/core/share_device_page_ui.rs:406`); CLI-Hilfe: „Machine-owned login access; ordinary user IPC cannot enable it“ (`N/cli/share/grants_access.rs:110`) |
| R16 | TCP weiterleiten | `Capability::TcpForward` (`N/share/core/access_grant.rs:39`), `TcpForwardRule {initiator, target, direction, host, ports, valid_until}` (`N/share/core/access_constraints.rs:182-189`), `TcpDirection` (`:75-78`), `TcpHost` (`:84`) | Off | Nein. Lokale Dienste sind bei Bildschirm steuern automatisch freigegeben (`N/share/core/access_steering.rs:1-20`, `N/app/core/share_tcp_ui.rs:1-9`) | Desktop „Dienste und TCP“ (`N/app/core/share_device_page_ui.rs:403`) |
| R17 | Remote Desktop (RDP) | kein eigenes Recht im Modell. Bildschirm steuern schließt „eigenen RDP-Dienst des Hosts“ ein (`N/share/core/access_steering.rs:1-7`, `:138-150`). Ablauf: `rdp::open_device_with` (`N/app/core/share_rdp_ui.rs:305-322`) | — | — | Ein Klick auf „Remote Desktop“ (`N/app/core/share_rdp_ui.rs:300-304`); die Host-Einrichtung ist dort als Administratorabfrage beschrieben (`N/app/core/share_rdp_this_device.rs:6-7`); Android hat keinen lokalen RDP-Client (`N/mobile/os/shared/domains/rdp.rs:114`) |
| R18 | Ende (Ablaufzeit) | `AccessGrant.valid_until: Option<i64>` (`N/share/core/access_grant.rs:237`), `Expiry::None` / `At(i64)` (`:220-226`) | ohne Ende (`:247`) | Nein | CLI `--for DAUER`, `--until ZEIT`, `--no-expiry` (`N/cli/share/grants_access.rs:62-72`); Desktop Zeitraum (`N/app/core/share_access_controls_ui.rs:21-22`); Android Zeitraum (`A/ui/share/AccessDialog.kt:337-343`) |
| R19 | Anfrage-Regel (eingehend) | `DirectRequestPolicy::{Ask, AutoAccept}` (`N/share/core/direct_relation.rs:152-160`), Datei `share_direct_policy.json` | `Ask` (`:155-157`) | `AutoAccept`: ja, ohne Nutzerentscheidung | CLI `se share request policy ask|auto-accept` (`N/cli/share/requests_policy.rs:17-27`); Desktop (`N/app/core/share_policy_ui.rs:17-33`); Android `share.setPolicy` (`mod.rs:281`, `share_policy.rs:283-292`) |
| R20 | Rückfreigabe je Kontakt (`share_back`) | `DirectRelationFlags.share_back: bool` (`N/share/core/direct_relation.rs:139-145`), in Kontakt (`:134`) | `false` (serde-Standard) | Beim Veröffentlichenden ja (`N/share/core/discovery_exchange_port_impl.rs:206`); beim Verbindenden nur mit Häkchen (`:417-421`) | CLI `se share device set --share-back` (`N/cli/share/devices.rs:71`); Desktop „Beim Verbinden auch meine Freigaben für dieses Gerät öffnen“ (`N/app/core/share_device_page_ui.rs:264-275`); Android „Eigene Freigaben …“ (`A/ui/share/DeviceSettingsPage.kt:113`) |
| R21 | Kontakt automatisch verbinden und öffnen | `DirectContact.auto_connect`, `auto_open` (`N/share/core/direct_relation.rs:110-111`) | `auto_connect = true`, `auto_open = false` (`N/share/core/profile_persistence.rs:190-191`) | Kein Rechtebezug | Desktop-Karte „Verbindung“ (`N/app/core/share_device_connection_ui.rs:55`); CLI `se share device set --auto-connect|--auto-open`; Android `share.setContactOptions` (`mod.rs:296`) |
| R22 | Zustand der Freigabe (Gesperrt, Bestätigen) | `DirectGrantState::{Accepted, Ignored, Reconfirm}` (`N/share/core/direct_relation.rs:49-61`) | Accepted nach Annahme | Nein | Desktop „Autorisierung widerrufen“ (`N/app/core/share_lifecycle_ui.rs:291`), „Wieder erlauben / Bestaetigen“ (`:319`); CLI `se share grants revoke|allow`; Android `WithdrawGrantDialog` (`A/ui/share/ShareConfirmationDialogs.kt:45`), `AllowGrantDialog` (`:32`) |

Rechte, die in den gelesenen Dateien nicht als Recht modelliert sind:
- Zwischenablage: keine Kennung gefunden (Suche in `N/share/**`, `N/app/core/share*.rs`, `N/cli/share*`, `N/mobile/**` nach Clipboard und Zwischenablage). Die Suche umfasst nicht `N/screen/**`, `N/rdp/**` und `N/cli/setup/**`. Dort nicht geprüft.
- Dateiempfang: keine Rechtekennung in der Suche (Begriffe Empfang, receive, file_receive). Der Empfang von Schnellteilen ist separat (`docs/oberflaechen/share-exec.md`, nicht geprüft).
- Laufwerke und Gerätebaum: nicht geprüft (Mount-Module nicht gelesen).
- Löschen: kein eigenes Recht. Löschen gehört zu `ReadWrite` (`N/share/core/export_config.rs:11-13`, Dokumentationszeile „Additionally create, change, move, recycle and remove files“).
- Papierkorb des Hosts: nicht geprüft (`N/cli/share/host_trash.rs` nicht gelesen; README nennt Schreibrecht als Voraussetzung).
- Auffindbarkeit (Angebot): kein Recht, sondern ein zeitlich begrenzter Zustand (Abschnitt 2.3).

## 4. Punkt 3: Festlegung beim Erzeugen eines Codes oder einer Einladung

| Erzeugung | Was wird festgelegt | Was wird nicht festgelegt | Quelle |
|---|---|---|---|
| Direkt-Code anzeigen (Identität) | Identität, Schlüssel, Fingerprint, Node-ID, Geheimnis | keine Freigabe, keine Ordner, keine Befehle, keine Rechte | `N/share/core/identity.rs:186-194`; CLI `se share identity` (`N/cli/share.rs:69`); Desktop „Dieses Gerät“ (`N/app/core/share_this_device_ui.rs:91-93`) |
| Direkt-Code neu erzeugen | neues Geheimnis; bestehende Freigaben werden auf „Bestätigen“ (Reconfirm) gesetzt | Rechte bleiben als Wert erhalten | `N/app/core/share_identity_rotation.rs:9`; `N/share/core/direct_relation.rs:56-59`; Android `share.renewCode` (`mod.rs:304`, `share_device_settings.rs:144`) |
| Raum erstellen (Raum-Code) | Raum-ID und Geheimnis; Raumpolicy neuer Raum | Ordner, Schreibrecht und Befehle der Mitglieder | `N/share/core/profiles.rs:170-176`; `N/share/core/room_relation_members.rs:29-33` |
| PIN-Angebot veröffentlichen | Ziel (Direkt oder Raum), Anzeigename, PIN, Dauer, Schwachen-PIN-Zustimmung | keine Rechte, keine Ordner | `N/share/core/discovery_signal_types.rs:157-170` |
| Anfrage stellen (Kontakt) | Anfragender, Ziel, Nachricht, Ablauf | keine Rechteforderung | `N/share/core/direct_protocol.rs:160-171` |

Gegenseite bei Annahme einer Anfrage (W1):
- Desktop: Fingerprint-Häkchen, dann „Akzeptieren“. Es gibt keine Rechteauswahl. Die Freigabe erhält `write = true`, Befehle aus, und die Direkt-Ordnerliste (`N/app/core/share_lifecycle_ui.rs:203-218`; `N/share/core/legacy_direct_request_decision.rs:215`).
- Android: „Annehmen“ ohne Fingerprint-Schritt (`A/ui/share/ShareLists.kt:61`). Die Rechte gelten wie auf dem Desktop.
- CLI: `se share request accept` ohne Rechteoption (`N/cli/share/requests.rs:29-47`).

Gegenseite bei PIN-Kopplung (W2):
- Veröffentlichender: sofort Freigabe mit Schreibrecht (Abschnitt 2.3).
- Verbindender: Kontakt; Freigabe nur bei gesetzter Rückfreigabe.

Gegenseite bei Auto-Annahme:
- Freigabe ohne Nutzerentscheidung mit `write = true` und Befehlen aus (`N/share/core/legacy_direct_request_decision.rs:75-80`, `:215`).
- Der Warntext behauptet das Gegenteil für das Schreibrecht (`N/app/core/share_policy_ui.rs:38`). Widerspruch, siehe Abschnitt 0, Punkt 5.

Gegenseite bei Raum-Beitritt: Mitglied wird ohne Zulassung aufgenommen, solange die Raumpolicy das nicht verlangt (`N/share/core/room_relation_members.rs:198-215`). Schreibrecht und Befehle sind aus.

## 5. Punkt 4: Nachträgliche Rechteänderung (Bedienwege)

### 5.1 Desktop („Geräte & Freigaben“)

Tabs und Nummern (`N/app/core/share_pages_ui.rs:11-20`, Anzeigenamen `:40-53`): 0 Geräte, 1 Räume, 2 Freigaben, 3 Diagnose, 4 Netzwerk, 5 Papierkorb (nur Windows), 6 Anfragen, 7 Befehle, 8 Dieses Gerät, 9 Bildschirm. Anzeigereihenfolge `TAB_ORDER` in `:26-36`.

Schritte für ein Gerät (Gerätseite):
1. Tab „Geräte“ → Zeile des Geräts → Schaltfläche „Einstellungen“ (`N/app/core/share_device_page_ui.rs:176`, `:189`, `:219`).
2. Karte „Was dieses Gerät bei mir darf“ (`:264`): Kontrollkästchen „Beim Verbinden auch meine Freigaben für dieses Gerät öffnen“ (Rückfreigabe), Checkbox „Darf in meine Freigaben schreiben“ (`:358-363`, Speicherung `set_direct_peer_write` in `:372`), Hinweis „Freigabe inaktiv; zuerst ausdrücklich wieder erlauben.“ (`:381`).
3. Unter derselben Karte: „Bildschirm und Befehle“ (`:384`), dann die Rechtezeilen (`crate::app::device_rights_ui::ui`, `:385-386`).
4. Karte „Verbindung“ (`share_device_connection_ui.rs:55`): „Automatisch verbinden“, „Automatisch öffnen, sobald es erreichbar ist“, „Diagnose“, „Vertrauen zurücksetzen“ (Hinweis: „Vergisst die gepinnte Identität dieses Geräts und entzieht seine Autorisierung (Befehle aus); der Kontakt bleibt und muss neu bestätigt werden.“).
5. Karte „Was ich auf diesem Gerät nutze“ (`share_device_page_ui.rs:389-406`): Programme, Fenster, Remote Desktop, Dienste und TCP, Maschinenanmeldung.

Schritte für Befehle und Zugriffsrechte (Tab Befehle):
1. Tab „Befehle“ (`N/app/core/share_pages_ui.rs:47`) → Zeile je Gerät, Schaltfläche „Bildschirm- und Befehlsrechte …“ (`N/app/core/share_exec_ui.rs:75`). Funktion `ui_exec_overview` (`:52`).
2. Dialog (`N/app/core/share_access_dialog_ui.rs:1-8`): Karte „Rechte“ (`N/app/core/share_access_controls_ui.rs:17`) mit einer Zeile je Recht und den Auswahlen Aus, Mit Rückfrage, Erlaubt (`rights_table`). Darunter „Gültig“ mit Auswahl „Ende nicht ändern“ (`:21-22`).
3. Karte „Befehle ausführen“ (`:25-31`): Voreinstellungen (`:81-93`, Abschnitt 6), Konto (`run_as`), Programmliste (`:172`).
4. Checkbox „Jeden Start an diesem Gerät bestätigen lassen“ (`:164`).
5. Übernehmen: Schaltfläche „Übernehmen“ (`:148`).
6. Entzug: „Zugriff entziehen“ (`N/app/core/share_exec_ui.rs:142`).
7. Rückfragen (Befehle, die auf Bestätigung warten): Abschnitt „Offene Rückfragen“ auf dem Tab Befehle (`N/app/core/share_exec_ui.rs:53`, `approvals_ui::ui_pending`), mit „Diesmal erlauben“, „Immer erlauben“, „Ablehnen“ (`N/app/core/share_access_approvals_ui.rs`, Schaltflächen nicht im Einzelnen geprüft).

Schritte für Ordner (Tab Freigaben):
1. Tab „Freigaben“ (`N/app/core/share_pages_ui.rs:11-20`, `N/app/core/share_exports_ui.rs:38`).
2. Bereich wählen (Direkt oder Raum). Ordner hinzufügen: Schaltfläche „Hinzufuegen“ (`:248`), „Alle Laufwerke hinzufuegen“ (`:235`, fügt alle Laufwerke ohne Dialog hinzu).
3. Spalten: „Nur lesen“, „Schreiben“, „Systemorte“ (`:82`). Auswahl „Nur lesen“ / „Lesen und Schreiben“ (`:117-119`). „Nach oben“ (`:156`), „Schreiben wieder erlauben“ (`:185`).

Schritte für Räume (Tab Räume):
1. Zeile des Raums → Raumseite (`N/app/core/share_room_page_ui.rs:12-13`, Seite `AddRoom`; Raumseite: Knopf „Raum erstellen oder beitreten …“ `:32`).
2. Schaltflächen auf der Raumseite: „Verlassen“ (`:101`), „Wieder beitreten“ (`:109`), „Umbenennen“ (`:198`), „Zulassen“ (`:247`), „Wieder zulassen“ (`:251`), „Sperren“ (`:254`), „Zugriff …“ (`:262`, Zugriff für das Mitglied).
3. Raumweiter Zugriff: „Zugriff für alle Mitglieder…“ (`N/app/core/share_access_room_ui.rs:83`).
4. Schreibrecht und Bestätigung der Raumpolicy: auf dem Desktop nicht gefunden (Abschnitt 9).

Schritte für Anfragen (Tab Anfragen):
1. Anfrageregel: Radio „Immer fragen“ und „Anfragen mit meinem Direkt-Code automatisch annehmen (unsicherer)“ (`N/app/core/share_policy_ui.rs:8-33`). Speicherung in `share_direct_policy.json`.
2. Schreibrecht je Gerät: Abschnitt „Schreibrecht je Geraet fuer meine Freigaben“ (`:52-…`), je Gerät „Geräteeinstellungen …“ (öffnet die Gerätseite).

### 5.2 Android („Teilen“)

Gerätezeile → Menü (`A/ui/share/SharePeers.kt:123-124`):
- „Zugriff anfragen“ (`:123`) → `ShareDialog.RequestAccess`.
- „Eigene Freigaben…“ (`:124`) → `ShareDialog.ShareBack`.

Geräteeinstellungen (`A/ui/share/DeviceSettingsPage.kt`):
- Abschnitt „Verbindung und Rückfreigabe“ (`:110`): „Eigene Freigaben …“ (`:113`), „Zugriff anfragen …“ (`:114`), „Vertrauen zurücksetzen …“ (`:115`).
- Apps und Programme (`:99-102`).
- Rechteanzeige: Befehle, Terminal, Bildschirm ansehen, Bildschirm steuern (Labels in `:207`); Verbindung (`:242`); Programme (`:257`).

Befehle auf diesem Telefon (`A/ui/share/ExecHostSection.kt:93`):
- Je Gerät „Geräteeinstellungen …“ (`:200`).
- Befehlszeile je Gerät: Zustand, Konto und Programme. Die Zeilenzuordnung ist nicht geprüft (Abschnitt 7).
- „Befehlsprotokoll anzeigen“ (`:121`); Ziel der Aktion `actions.log` nicht geprüft.
- Aktion `onAccess` (`:116`) setzt den Zugriffsschlüssel → `AccessDialog` (`A/ui/share/AccessDialog.kt:285`). Der Schaltflächentext ist nicht geprüft.

`AccessDialog` (`A/ui/share/AccessDialog.kt`):
- Titel „Zugriff für „…““ (`:315`).
- Zeilen je Recht mit Aus, Fragen, Erlauben (`:325-330`). Zeilen: `DEFAULT_ROWS` (`:59-62`) oder die Zeilen aus `share.status` (`N/mobile/os/shared/domains/share_access.rs:77-87`).
- „Bei „Fragen“ bestätigst du jeden Start auf diesem Telefon.“ (`:336`).
- Zeitraum: „Ohne Ende“, „1 Stunde“, „8 Stunden“, „1 Tag“, „1 Woche“, „bis …“ (`:301-303`, `:70-75`, Auswahl `:337-343`).
- Programme: Liste, Hinzufügen, Volle Shell (`:347-378`).
- Hinweis bei Einschränkung: „Laufende Befehle dieses Geräts enden, wenn die Änderung Rechte einschränkt.“ (`:412`).
- Bestätigung: `accessChangeOf(draft, …)` (`:417`).
- Konto: nur „Ausführen als: App-Konto“ (`:383`).

Rückfrage (Befehl wartet auf Entscheidung): `AccessApprovalPrompt` (`A/ui/share/AccessDialog.kt:215-250`) mit „Diesmal erlauben“ (`:246`), „Immer erlauben“ (`:247`), „Ablehnen“ (`:250`). Benachrichtigung mit denselben drei Aktionen (`A/service/AccessApprovalNotifier.kt:118-124`).

Freigaben (Abschnitt „Freigaben“ in `ShareScreen`): „Rechte…“ (`A/ui/share/ShareLists.kt:105`), „Schreiben wieder erlauben…“ (`:135`), „Gespeicherte Verbindungen einzeln…“ (`:141`). Ordner hinzufügen mit Bestätigung: `A/ui/share/ShareDialogHost.kt:261` („Ordner für deine Geräte freigeben?“, Text: „Deine angenommenen Geräte dürfen hier lesen und schreiben.“) und `:270` („Ordner nur lesend freigeben?“).

Freigabe entziehen oder wieder erlauben (`A/ui/share/SharePolicySections.kt:36-37`): „Zugriff entziehen…“, „Wieder zulassen…“. Dialoge: `WithdrawGrantDialog` (`A/ui/share/ShareConfirmationDialogs.kt:45`), `AllowGrantDialog` (`:32`).

Raum (`A/ui/share/SharePeers.kt:174-178`, `:219-224`):
- Menü: „Code anzeigen“ (`:174`), „Rechte und Bestätigung…“ (`:176`, `ShareDialog.RoomPolicy`), „Umbenennen…“ (`:177`), „Verlassen“ oder „Wieder beitreten“ (`:178`).
- Mitglied: „Zulassen…“ (`:219`), „Wieder zulassen…“ (`:220`), „Vertrauen zurücksetzen“ (`:224`).
- Dialog mit Titel „Zulassen“, „Sperren“ oder „Wieder zulassen“: `RoomMemberDialog` (`A/ui/share/ShareConfirmationDialogs.kt:59-64`).

Anfrage-Regel: Schalter in `A/ui/share/ShareScreen.kt:311` („Automatisches Annehmen erlauben…“ / „Immer fragen…“). Dialog `RequestPolicyDialog` (`A/ui/share/ShareConfirmationDialogs.kt:78`).

Anfragen: Abschnitt „Anfragen“ (`A/ui/share/ShareLists.kt:41-62`): „Annehmen“ (`:61`), „Ablehnen“ (`:62`).

Bridge-Methoden (Android → Rust, `N/mobile/os/shared/domains/mod.rs:245-304`), Rechte-bezogen: `setExportAccess` (271), `setConnectionExport` (273), `setContactWrite` (274), `allowGrantAgain` (275), `withdrawGrant` (276), `setRoomMember` (277), `setShareBack` (278), `setRoomPolicy` (279), `policy` (280), `setPolicy` (281), `setExec` (287), `execJobs` (288), `cancelExecJob` (289), `execLog` (290), `setAccess` (291), `deviceRights` (292), `accessApprovals` (293), `decideAccessApproval` (294), `reportApprovalUndeliverable` (295), `setContactOptions` (296), `resetContactTrust` (297), `renameRoom` (298), `rejoinRoom` (299), `resetMemberTrust` (300), `moveExport` (301), `setDirectOnline` (302), `setLanPresence` (303), `renewCode` (304).

Implementierung: `setAccess` (`N/mobile/os/shared/domains/share_access.rs:215`), `deviceRights` (`:149`), `approvals` (`:355`), `decide` (`:401`), `access_change` (`:276-…`), `account_of` (`:313-321`).

### 5.3 CLI

Vollständige Befehlsgruppe `se share` (`N/cli/share.rs:64-104`, Dispatch `:158-173`): configure, server, identity, status, request, grants, exec, export, room, device, discoverable, discover, worker, lan, trash.

Rechte-Befehle (vollständig für die Gruppen, die im Lesebereich liegen):
- `se share grants`: list; set GERÄT `--write|--read-only` (`N/cli/share/grants_write.rs:6-14`); revoke; allow; delete; removed `[--readmit GERÄT]`; exec `[enable|disable] [--yes] [--run-as] [--allow-program] [--full]`; access (`N/cli/share/grants_access.rs:43-84`).
- `se share grants access`: Optionen laut `AccessArgs` (`:43-84`): `SELECTOR`, `--device-view`, `--room ROOM --all-members`, `--as own|remote|account:NAME` (`:56-57`), Fähigkeitsoptionen (`--commands`, `--terminal`, `--screen-view`, `--screen-control`, `--app-launch`, `--login-access`, `--tcp-forward` je nach Katalog), `--ask` (`:62`), `--for DAUER` (`:65`), `--until ZEIT` (`:68`), `--no-expiry` (`:70-71`), `--allow-program PFAD` (`:74`), `--full` (`:77`), `--off` (`:80-81`), `--yes` (`:83`).
- Ohne Option: Anzeige. Nur `--yes`: Befehle und Terminal als eigenes Konto (`:207`, `AccessPreset::FullOwn`).
- Feld `constraints: None` (`:214`): die CLI setzt keine Ziel- oder Anwendungsgrenzen.
- `se share export`: list, add `PFAD [--label NAME] [--room ROOM]` (`N/cli/share/exports.rs:45-52`, `:142-160`), remove, set `SELECTOR [--write|--read-only] [--allow-system-writes|--protect-system-files] [--room ROOM]` (`N/cli/share/exports_policy.rs:8-29`), connections, policy `--room ROOM [--write|--read-only] [--confirm-new-members true|false]` (`exports_policy.rs:31-48`), move.
- `se share device`: list, set `[--auto-connect] [--auto-open] [--share-back]` (`N/cli/share/devices.rs:71`), reset-trust (README, nicht im Detail geprüft).
- `se share room`: list, create, join, leave, rejoin, rename, code, remove; member `RAUM GERÄT admit|block|allow|reset-trust` (README, `N/cli/share/rooms.rs` nicht gelesen).
- `se share request`: list, show, accept, reject, retry, delete, policy (`N/cli/share/requests.rs:29-47`, `requests_policy.rs:17-27`).
- `se share exec`: list, show, cancel, history, log `[--lines N] [--json]` (`N/cli/share/exec_status.rs:15-34`), approvals, approve, deny (`N/cli/share/exec_approvals.rs:25-34`), accounts (Unterbefehle laut `N/cli/share/exec_accounts.rs:29-50`: setup, allow, disallow, update, remove).
- `se share discoverable` und `se share discover` (Abschnitt 2.3).
- `se connections add-peer|add-room|remove-peer|remove-room` (`N/cli/connections.rs:89-99`).

### 5.4 MCP

- `se` führt jeden CLI-Befehl aus (`N/cli/mcp/catalog.rs:183-184`). Die Richtlinie in `:39-41` verlangt ausdrückliche Bitte der Person für Änderungen an Share-Vertrauen.
- Gesperrt: `se share exec approve|deny` und `se share exec accounts setup|allow|disallow|update|remove` (`N/cli/share/exec_approvals.rs:232-264`).
- Dedizierte MCP-Werkzeuge im Katalog, die in den gelesenen Zeilen auftreten: `status`, `connections` (`:74-75`), `se` (`:183`), `screen` (`:197`), `screen_input` (`:199`), `tcp_forward` (`:200`), `rdp` (`:202`). Eine vollständige Liste aller Werkzeuge ist nicht geprüft. Ein Werkzeug speziell für Rechte oder Kopplung wurde in den gelesenen Zeilen nicht gefunden.

## 6. Punkt 5: Voreinstellungen und Profile

| Vorlage | Definition | Ort | Wer nutzt sie | Werte und Wirkung |
|---|---|---|---|---|
| Befehls-Voreinstellungen (`AccessPreset`) | drei Varianten plus Schalter „Mit Rückfrage“ | `N/share/core/access_grant.rs:460-491` | Desktop (`N/app/core/share_access_controls_ui.rs:81-93`), CLI (`grants_access.rs:207`) | `FullOwn`: „Volle Rechte als mein Konto“, Konto `Own`, Befehle und Terminal auf Allow (oder Ask), Programmliste leer. `RemoteAccount`: „Eingeschränktes Fernzugriffskonto se-remote“, Konto `Remote`, Befehle und Terminal. Nur wählbar, wenn der Host `se-remote` anbietet (`share_access_controls_ui.rs:95-97`). `Programs`: „Nur bestimmte Programme“, Konto `Own`, Terminal aus, Programmliste gesetzt. |
| Anfrage-Regel | `DirectRequestPolicy` | `N/share/core/direct_relation.rs:152-160` | Desktop, CLI, Android | `Ask` (Standard) oder `AutoAccept`. |
| Zeitraum (Desktop) | Ende nicht ändern (Standard für neue Entwürfe), Unbegrenzt, 1 Stunde, 8 Stunden, 1 Tag, 7 Tage | `N/app/core/share_exec_programs_ui.rs:22-27` (Liste), `N/app/core/share_access_dialog_ui.rs:99` (Standard `Period::Keep`) | Desktop | Festliste |
| Zeitraum (Android) | Ohne Ende (Standard, `A/ui/share/AccessDialog.kt:128`), 1 Stunde, 8 Stunden, 1 Tag, 1 Woche | `A/ui/share/AccessDialog.kt:70-75` | Android | Festliste; bestehendes Ende wird als „bis …“ angeboten (`:301`) |
| Zeitraum (CLI) | frei: `--for 30m|8h|2d|1d12h`, `--until ZEIT` | `N/cli/share/grants_access.rs:62-72` | CLI, MCP | beliebige Dauer im genannten Format |
| Ordner-Voreinstellung | Direkt-Scope: Lesen und Schreiben; Raum-Scope: Nur lesen | `N/share/core/export_config.rs:75-82` | alle Oberflächen | Standard je Scope |
| Raum-Voreinstellung | Schreibrecht der Mitglieder aus, Bestätigung neuer Mitglieder aus | `N/share/core/room_relation_members.rs:29-33` | alle Oberflächen | Bestätigung wird nach der ersten Sperre im Raum erzwungen (`:91-96`) |
| Kontakt-Voreinstellung | `auto_connect = true`, `auto_open = false` | `N/share/core/profile_persistence.rs:190-191` | Desktop, CLI, Android | ohne Rechtebezug |
| Rückfreigabe-Standard | `false` beim Hinzufügen per Code (Desktop `N/app/core/share_direct_ui.rs:8-10`, Android `A/ui/share/ShareDialogs.kt:98`, Bridge `N/mobile/os/shared/domains/share_peers.rs:53-57`), `false` bei PIN-Verbindung (`A/ui/share/ShareConnectDialog.kt`, `shareBack = false` bei Auswahl), zurückgesetzt nach jedem Verbinden (`N/app/core/share_discovery_ui.rs:269`) | je Oberfläche | — | Der Veröffentlichende ist davon nicht betroffen (Abschnitt 2.3). |

Benannte Rechte-Bündel außer den drei Befehls-Voreinstellungen: keine in den gelesenen Dateien gefunden. Der Begriff „Profil“ im Code bezeichnet den Datenspeicher `ShareProfiles` (`N/share/core/profiles.rs:39`) und nicht eine Rechte-Vorlage.

## 7. Punkt 6: docs/oberflaechen/share-exec.md

### 7.1 Prüfmethode und Zahlen

- Geprüft: alle 160 Belegeinträge (`Datei:Zeile` oder `Datei`) im Dokument, durch automatischen Abgleich. Ergebnis: 127 Zeilenbelege, 33 reine Dateibelege, 97 verschiedene Dateien. Fehlende Dateien: 0. Zeilen hinter Dateiende: 0.
- Zeilenbelege: der Inhalt der genannten Zeile wurde angezeigt und gegen die Aussage des Dokuments gehalten. Mindestens 40 Zeilenbelege zeigen nachweislich auf eine Zeile mit anderem Inhalt. Die übrigen Zeilenbelege wurden nicht einzeln mit dem Sinn der Aussage abgeglichen.
- Verschiebung: Der Lesestand des Dokuments ist `7e7203c9` (v0.5.181), die Aussage „Stand 2026-10-21 (Lauf U42, v0.5.183)“ steht in Zeile 3. Seit dem Lesestand liegen Commits vor, die Zeilen verschoben haben.
- Dateien außerhalb des Lesebereichs wurden nur für die Existenzprüfung angefasst, mit Ausnahme der zwei Zeilen aus `N/cli/completions.rs` (Abschnitt 11).

### 7.2 Zuordnungen, die auf falsche Dateien zeigen

| Aussage im Dokument | Beleg im Dokument | Befund |
|---|---|---|
| „Direkt-Code anzeigen, kopieren“ | `N/app/core/share_direct_ui.rs:35-60` | Die Datei hat 73 Zeilen und enthält nur das Hinzufügen per Code. Die Anzeige steht in `N/app/core/share_this_device_ui.rs:30`, `:91-93`. |
| „Zugriff anfragen“ (Desktop) | `N/app/core/share_device_page_ui.rs:243` | Der Knopf steht in `N/app/core/share_device_connection_ui.rs:34`. |
| „Schreibrecht je Gerät“ (Desktop) | `N/app/core/share_policy_ui.rs:22-44` | Die Liste steht in `share_policy_ui.rs:52-…`; das Kontrollkästchen in `share_device_page_ui.rs:358-363`. |
| Android Direkt-Code hinzufügen | `A/ui/share/ShareDialogHost.kt:134` | Der Dialog steht in `ShareDialogHost.kt:183-193`. |
| Android Raum erstellen und beitreten | `A/ui/share/ShareScreen.kt:232-233` | Die Schaltflächen stehen in `ShareScreen.kt:327-328`. |
| CLI `se share room create` | `N/cli/share.rs:124-127` | Zeile 124 ist `struct ServerArgs`. Der Befehl steht in `N/cli/share.rs:88`. |
| CLI `se connections add-peer` und `add-room` | `N/cli/connections.rs:79-80`, `:82` | Befehle stehen in `N/cli/connections.rs:89` und `:91`. |
| Bridge-Dispatch `share.*` | `N/mobile/os/shared/domains/mod.rs:160-208` | Die Dispatch-Zeilen stehen in `mod.rs:245-304`. `share.setAccess` steht in `mod.rs:291`, nicht in `:54`. |
| IPC: `ShareCommand`-Anfrage | `N/daemon/os/shared/ipc_protocol.rs:190` | Die Anfrage `ShareCommand` steht in `ipc_protocol.rs:128-131`, die Antwort `ShareCommandReply` in `:89-101`. |
| MCP-Sperre | `N/cli/share/exec_approvals.rs:210-238` | Die Funktion `mcp_refusal` steht in `:232-264`. |
| Android: „Auf Android laufen Befehle nur unter dem App-Konto“ | `N/mobile/os/shared/domains/share_access.rs:245-254` | Die Zeilen 245-254 enthalten die Meldung „Die Freigabe für dieses Gerät ist nicht aktiv“. Der zitierte Satz steht in `share_access.rs:318`. Die Aussage stimmt, die Zeile nicht. |
| Desktop Kontakt-Name ändern | `N/app/core/share_direct_ui.rs:73-90` | Die Datei endet bei Zeile 73. |
| Desktop Befehls-Dialog „Zugriff für …“ | `N/app/core/share_access_dialog_ui.rs:353` | Zeile 353 ist eine schließende Klammer. |
| Android Dialog „Zugriff für …“ | `A/ui/share/AccessDialog.kt:306` | Titel steht in `:315`. |
| Android „Zeitraum“ | `A/ui/share/AccessDialog.kt:327` | Abschnitt „Zeitraum“ steht in `:337`. |
| Desktop Raumseite „Zugriff …“ | `N/app/core/share_room_page_ui.rs:248` | Knopf steht in `:262`. |
| Desktop Exec-Protokoll Lesefunktion | `N/app/core/share_exec_log_ui.rs:218` | Zeile 218 startet einen Thread. |
| `REMOTE_ACCOUNT_NAME` | `N/share/core/access_grant.rs:14` | Konstante steht in `:17`. |
| CLI `se share grants access`-Ende (`--for`) | `N/cli/share/grants_access.rs:67-73` | Stimmt im Inhalt. |
| Ausführungsgrenzen | `N/share/core/exec_registry.rs:84-90` | Inhalt stimmt (keine Grenze für parallele Befehle). |

### 7.3 Inhaltliche Befunde

- Das Dokument nennt für das Zugriffsrecht die Teile „Befehle, Terminal, Konto, Ende, Programmliste“ (Abschnitt „Stand nach dem Stapel Rechte“). Der Code kennt sieben Fähigkeiten: `ExecCommands`, `ExecTerminal`, `ScreenView`, `ScreenControl`, `AppLaunch`, `LoginAccess`, `TcpForward` (`N/share/core/access_grant.rs:25-40`). Die Aufzählung ist damit unvollständig. Der Abschnitt „Laufender Gesamtkandidat“ verweist für Maschinenanmeldung, TCP, RDP, Appkatalog und Fensterfamilien nur auf `fernzugriff.md`.
- Die Zeile „Anfrageregel nur Anzeige (share_policy_ui.rs:6-21)“ im Bestandsabschnitt beschreibt den Lesestand. Der Stapel-Abschnitt sagt, die Regel sei änderbar. Das passt zum Code (`share_policy_ui.rs:17-33`).
- Die Aussage „MCP nimmt keine Rückfrage an (approve, deny)“ passt zur Sperrliste (`mcp_refusal`). Die Doku-Zeile zu `mcp_refusal` spricht von „answering a request“ (`N/cli/share/exec_approvals.rs` vor Zeile 232). Die Sperre gilt nur für Befehls-Rückfragen, nicht für `se share request accept` (Abschnitt 2.5).
- Die Aussage zu Rechten, Konto, Programmliste, Ende und Rückfrage passt zum gelesenen Code (`access_grant.rs:150-159`, `:460-491`, `exec_policy.rs:50-60`).
- Die Aussage „PrincipalLimit und GlobalLimit gibt es nicht mehr“ wurde nicht geprüft (Suche außerhalb des Lesebereichs nicht erlaubt).

### 7.4 Weitere Dokumente

- `docs/SHARE_PLAN.md`: als historisch markiert (Kopfzeile). Das Konzept Noise/TCP wird dort beschrieben. Für den Ist-Stand nicht maßgeblich.
- `docs/SHARE_SERVER.md` (Abschnitte 112-168 und 400-420 gelesen): Die Beschreibung der PIN-Kopplung und der Anfrage-Lebensdauer stimmt mit dem Code überein (Ergebnis `UserPairing` beim Veröffentlichenden, Auto-Annahme nur mit Opt-in, Rückfreigabe nur mit Häkchen). Keine Gegenaussage gefunden.
- `README.md` (Share-Abschnitte 485-560 und 728-800 gelesen): „Neue Direkt-Freigaben sind lesbar und beschreibbar, und neu angenommene Geräte haben Schreibrecht“ stimmt mit `NEW_DIRECT_GRANT_WRITE = true` überein. „Neue Raum-Freigaben sind zunächst nur lesbar“ stimmt mit `ExportAccess::ReadOnly` für den Raum-Scope überein.

## 8. Abgleich mit der Nutzervorgabe (Lücken)

Die folgenden Punkte beschreiben nur den Abstand zwischen dem Code und dem Wunschbild. Eine Bewertung oder Entscheidung ist hier nicht getroffen.

1. Rechte nach Bestätigung: Bei der PIN-Kopplung erhält der Veröffentlichende den Verbindenden ohne weiteren Schritt mit Schreibzugriff auf die Direkt-Ordner (`N/share/core/discovery_exchange_port_impl.rs:206`, `N/share/core/direct_reciprocal.rs:400`).
2. Keine Ordnerauswahl pro Gerät: Die Direkt-Ordnerliste gilt für alle Direkt-Geräte (`N/share/core/access_relation_view.rs:118-124`). Ein Gerät bekommt nur Schreibrecht, Befehle, Bildschirm und Zusatzrechte.
3. Desktop sehen und steuern: „Bildschirm steuern“ schließt Bildschirm ansehen, Programme, Anmeldung und eigene RDP-Dienste ein (`N/share/core/access_steering.rs:1-20`).
4. Voreinstellungen beider Seiten: Es gibt drei Befehls-Voreinstellungen. Keine Voreinstellung für Ordner (Lesen, Schreiben, nur lesen) pro Gerät, keine für Bildschirm und Dateien. Android hat keine Voreinstellungen (Abschnitt 6).
5. Abbruch und Entzug: Die Funktion `withdraw_direct_key` setzt passende Freigaben auf Ignored und schaltet deren Befehle aus (`N/share/core/direct_relation.rs:230-246`); der Kontakt wird von dieser Funktion nicht geändert. Welche Oberflächenaktion sie aufruft, ist nicht im Einzelnen geprüft. „Vertrauen zurücksetzen“ entzieht die Autorisierung; der Kontakt bleibt und muss neu bestätigt werden (`N/app/core/share_device_connection_ui.rs`, Hinweistext). „Gerät entfernen“ löscht Kontakt, Autorisierung und Anfragen dieses Geräts (`N/app/core/share_lifecycle_ui.rs:332`).
6. Warntext: Die Auto-Annahme erhält Schreibrecht, der Text sagt das Gegenteil (Abschnitt 0, Punkt 5).
7. Raum: Neue Mitglieder werden ohne Bestätigung aufgenommen, solange die Raumpolicy das nicht verlangt (`N/share/core/room_relation_members.rs:198-215`).
8. Anfrage ohne Fingerprint auf Android (`A/ui/share/ShareLists.kt:61`) und mit Fingerprint auf dem Desktop (`N/app/core/share_lifecycle_ui.rs:203-218`).
9. RDP: ein Klick, ohne Formular und ohne Ablaufzeit (`N/app/core/share_rdp_ui.rs:300-304`). Das entspricht dem Wunsch „rdp setup wenn notwendig“, sofern der Host die Administratorabfrage bestätigt (`N/app/core/share_rdp_this_device.rs:6-7`).
10. Kommentar und Code widersprechen sich bei der Rückfreigabe: Der Kommentar zu `set_contact_share_back` sagt „read-only unless Darf schreiben“ (`N/share/core/direct_relation.rs:323`). Die neu angelegte Freigabe in derselben Funktion setzt `write: NEW_DIRECT_GRANT_WRITE` (`N/share/core/direct_relation.rs:382`), also Schreibrecht `true`.

## 9. Offene Fragen und fehlende Fakten

1. Wie `setup::add_peer` und `setup::add_room` die Anfrage und den Beitritt auslösen (`N/cli/setup/share_peer.rs:114`, nicht gelesen). Fehlender Fakt: Aufruf der Share-Funktionen dort.
2. Fortsetzung des IPC-Pfads `start_discovery_pairing` in `discovery_pairing_ipc.rs` und `discovery_pairing_protocol.rs` (nicht gelesen). Die Dateien `access_ipc.rs`, `devices_ipc.rs`, `exec_ipc.rs`, `apps_ipc.rs` liegen nicht im Glob `ipc*.rs` und wurden deshalb nicht gelesen. Fehlender Fakt: Inhalt dieser Dateien.
3. Auswirkung von `ShareCmd::LanPeerSeen` (`N/share/core/types.rs:405`) auf Kontakte und Rechte. Fehlender Fakt: die Verarbeitungsstelle.
4. Dateiprüfung für Direkt-Freigaben: Bindet jede akzeptierte Direkt-Beziehung dieselbe Liste `default_direct_exports`? Die Ansicht setzt das voraus (`N/share/core/access_relation_view.rs:124`). Fehlender Fakt: die Dateizugriffsstelle (`fs_access.rs`, `server_fs.rs`, nicht gelesen). Annahme: ja.
5. Desktop-Raumseite: Schalter für das Schreibrecht der Mitglieder (`members_may_write`) und für „Neue Mitglieder bestätigen“. Die Suche in `share_room_page_ui.rs` ergab keinen Treffer für „Schreiben“ oder „Bestätigen“. Fehlender Fakt: ob diese Einstellung auf dem Desktop fehlt.
6. Warntext der Auto-Annahme (`N/app/core/share_policy_ui.rs:38`) und tatsächliches Schreibrecht (`write = true`). Frage an den Nutzer: welches Verhalten gewünscht ist.
7. Android: Annahme einer Anfrage ohne Fingerprint-Schritt (`A/ui/share/ShareLists.kt:61`). Frage: gewünschtes Verhalten.
8. RDP-Host-Einrichtung (`rdp::host_setup`, `rdp::open_device_with`; nicht gelesen): welche Rechte sie verlangt und welche Administratorabfrage auf dem Host erscheint.
9. Bildschirm-Katalogzeilen (`N/screen/core/catalog_entries.rs`, nicht gelesen): genaue Bedingung „grafische Sitzung“ und Bezeichnungen.
10. Laufwerke und Gerätebaum als Recht: Mount-Module nicht gelesen. Fehlender Fakt: Rechtekennung für Laufwerksfreigaben.
11. Zwischenablage und Dateiempfang: keine Kennung in den gelesenen Dateien. Fehlender Fakt: Ob sie in `N/screen/**`, `N/rdp/**` oder `N/cli/setup/**` geregelt sind.
12. Papierkorb des Hosts: Recht und Dateien (`N/cli/share/host_trash.rs`) nicht gelesen.
13. MCP: Ob `se share request accept` ausdrücklich erlaubt sein soll. Der Kommentar von `mcp_refusal` spricht von „answering a request“ (`N/cli/share/exec_approvals.rs`, vor Zeile 232). Die Sperrliste nennt nur Befehls-Rückfragen.
14. Verhalten von Dialogen bei Abbrechen: nicht je Dialog geprüft.
15. Quick Share: eigene Funktion laut Dokument; `N/app/core/quickshare_ui.rs` nicht gelesen.
16. Ergebnis beim Veröffentlichenden einer Raum-PIN: Mitgliedsanlage im Austauschcode (`discovery_exchange_port_impl.rs`, Bereich nach Zeile 220) nicht gelesen.
17. Abhängigkeit zur Nutzervorgabe: Das Wunschbild verlangt Rechte erst nach Bestätigung. Der heutige PIN-Pfad gibt dem Veröffentlichenden die Freigabe ohne Bestätigung. Die Entscheidung, ob das für Direkt-Kopplung geändert werden soll, ist offen.

## 10. Gelesene Dateien

Dokumente:
- `docs/oberflaechen/share-exec.md` (vollständig, 231 Zeilen)
- `docs/SHARE_PLAN.md` (vollständig)
- `docs/SHARE_SERVER.md` (Gliederung; Zeilen 112-168, 400-420)
- `README.md` (Zeilen 485-560, 728-800; Treffer der Suche nach Share-Begriffen)

Rust, `N/share/core`:
- `access_grant.rs` (vollständig), `access_catalog.rs` (vollständig), `access_steering.rs` (1-30, 120-150), `access_constraints.rs` (75-90, 155-240), `access_relation_view.rs` (112-130), `access_grant_wire.rs` (Treffer), `export_config.rs` (1-80, Treffer), `exec_policy.rs` (20-60, 118-160), `exec_registry.rs` (84-90, Beleg), `exec_protocol.rs` (Beleg), `exec_program_path.rs` (Beleg)
- `direct_relation.rs` (1-400, Ausschnitte; Treffer bis 441), `direct_reciprocal.rs` (205-240, 255-330, 352-410), `direct_ledger_projection.rs` (66-140), `direct_protocol.rs` (160-206), `direct_lifecycle.rs` (Treffer)
- `legacy_direct_request_decision.rs` (36-118, 155-220), `legacy_direct_request.rs` (200-225), `legacy_direct_request_mutations.rs` (Treffer 84, 110)
- `discovery_signal_types.rs` (10-30, 66-100, 155-260), `discovery_domain.rs` (176-262), `discovery_pin.rs` (Modul-Doku), `discovery_signal_outcome.rs` (27-62), `discovery_offer_book.rs` (Modul-Doku), `discovery_exchange_port_impl.rs` (196-212, 405-425), `discovery_relation_store.rs` (Signaturen und Treffer)
- `profiles.rs` (106-170, 206-232, Treffer), `profile_persistence.rs` (180-194), `profile_migration.rs` (Treffer), `identity.rs` (180-200), `removed_direct_peers.rs` (55-75), `room_relation_members.rs` (1-70, 86-96, 198-218), `types.rs` (108-140 und Treffer; 282-357 Varianten), `node_accept.rs` (vollständig), `einrichtung_modules.rs` (vollständig)
- Treffer ohne vollständiges Lesen: `profile_operations.rs`, `profile_policy.rs`, `access_relation_view.rs`, `signal_commands_local.rs` (Zeile 59), `exec_auth.rs` (Treffer 252, 263, 304, 315; Testzuordnung nicht geprüft)

Rust, `N/share/os`:
- `os/shared/discovery_pairing_ui.rs` (62-85), `os/shared/profile_store.rs` (Treffer), `os/shared/identity_store.rs` (Treffer), `os/shared/direct_policy_store.rs` (Treffer), `os/shared/lan_settings_store.rs` (Treffer), `os/shared/exec_audit_files.rs` (Treffer), `os/shared/exec_audit_view.rs` (nur Existenz)
- `os/linux_os/exec_*.rs` (Treffer je Konstante), `os/windows/exec_account_*.rs` (Treffer je Modul-Doku)

Rust, `N/app/core`:
- `share_pages_ui.rs` (1-80, Treffer), `share_device_page_ui.rs` (20-60, 240-300, 330-411, Treffer), `share_device_connection_ui.rs` (1-140), `share_direct_ui.rs` (vollständig), `share_policy_ui.rs` (vollständig), `share_discovery_ui.rs` (1-30, Treffer 181-300), `share_discovery_publish_ui.rs` (150-196, Treffer), `share_rooms_ui.rs` (Treffer), `share_room_page_ui.rs` (Treffer), `share_access_dialog_ui.rs` (1-16, Treffer), `share_access_controls_ui.rs` (1-44, 78-94, Treffer), `share_access_room_ui.rs` (Treffer), `share_access_accounts_ui.rs` (Treffer), `share_access_approvals_ui.rs` (Treffer), `share_exec_ui.rs` (40-80, Treffer), `share_exec_programs_ui.rs` (22-31, Treffer), `share_exec_jobs_ui.rs` (Treffer), `share_exec_log_ui.rs` (Treffer), `share_lifecycle_ui.rs` (118-140, 200-246, Treffer), `share_lifecycle_actions_ui.rs` (88-92), `share_exports_ui.rs` (Treffer), `share_this_device_ui.rs` (Treffer), `share_identity_rotation.rs` (Treffer), `share_removed_devices_ui.rs` (Treffer), `share_rdp_ui.rs` (1-12, 296-322), `share_rdp_this_device.rs` (1-10), `share_tcp_ui.rs` (1-12), `share_window_ui.rs` (Treffer), `share_lan_ui.rs` (Treffer), `device_rights_model.rs` (1-60), `menus_settings.rs` und `quickshare_ui.rs` (nur Existenz)

Rust, `N/cli`:
- `share.rs` (58-173, Treffer), `share/discover.rs` (vollständig), `share/discoverable.rs` (20-112), `share/grants.rs` (20-60, Treffer 121-127), `share/grants_access.rs` (1-100, Treffer 106-110, 207, 214), `share/grants_write.rs` (Treffer), `share/exports.rs` (17-60, 140-160), `share/exports_policy.rs` (8-50), `share/requests.rs` (25-60), `share/requests_inbox.rs` (1-40, Testfixture 330-366), `share/requests_policy.rs` (Treffer 17-40), `share/exec_approvals.rs` (25-34, 200-266), `share/exec_accounts.rs` (Treffer), `share/exec_status.rs` (Treffer), `share/devices.rs` (Treffer 71; Testfixture 395-415), `share/lan.rs` (Treffer)
- `connections.rs` (1-110, 185-207, 265-295, Treffer)
- `mcp/catalog.rs` (20-44, 71-75, 176-202, Treffer), `mcp/cli_tool.rs` (1-6), `mcp/status_tools.rs` (Treffer)

Rust, `N/mobile` und `N/daemon`:
- `mobile/os/shared/domains/mod.rs` (150-215, 245-304)
- `mobile/os/shared/domains/share_peers.rs` (1-200)
- `mobile/os/shared/domains/share_access.rs` (77-92, 149, 215, 245-330, 355, 401; Treffer)
- `mobile/os/shared/domains/share_requests.rs` (29-60, 91; Treffer)
- `mobile/os/shared/domains/share_settings.rs` (Funktionszeilen; `set_server` 78-99)
- `mobile/os/shared/domains/share_policy.rs` (Funktionszeilen 49-283)
- `mobile/os/shared/domains/share_device_settings.rs` (Funktionszeilen 67-144)
- `mobile/os/shared/domains/share_exec.rs` (26-27, 101, 189, 194; Treffer)
- `mobile/os/shared/domains/rdp.rs` (Treffer 23, 114, 117)
- `mobile/os/shared/domains/share_status.rs` (Treffer 301-313)
- `daemon/os/shared/ipc_protocol.rs` (86-142, 150-250, 330-370, Treffer)

Android, `A/ui/share`, `A/api`, `A/service`, `A/screen`:
- `ui/share/ShareDialogHost.kt` (21-78, 90-205, 236-300), `ui/share/ShareConnectDialog.kt` (50-70, 150-200, 220-265, Treffer), `ui/share/ShareConnectionDialog.kt` (Treffer), `ui/share/ShareConfirmationDialogs.kt` (Treffer), `ui/share/ShareDialogs.kt` (86-125, Treffer), `ui/share/AccessDialog.kt` (55-76, 121-150, 152-250, 285-430, Treffer), `ui/share/ExecHostSection.kt` (Treffer 49, 93, 116, 121, 127, 200), `ui/share/ExecProgramsDialog.kt` (Treffer 69), `ui/share/ShareDeviceCard.kt` (Treffer 91, 106, 138-139), `ui/share/ShareLists.kt` (41-62, 80-141), `ui/share/ShareScreen.kt` (Treffer 70, 286, 311, 326-328, 348, 376-384), `ui/share/SharePeers.kt` (Treffer 95, 123-124, 154, 174-178, 219-224), `ui/share/SharePolicySections.kt` (Treffer 23, 36-37), `ui/share/DeviceSettingsPage.kt` (Treffer 99-115, 164-165, 207, 242, 257)
- `api/ShareApi.kt` (Treffer 207, 245-306), `api/ShareAccessApi.kt` (Treffer 171, 203, 210)
- `service/AccessApprovalNotifier.kt` (Treffer 118-124), `screen/ScreenConsentActivity.kt` (Treffer 74-89)

Zusätzlich gelesen, außerhalb des Lesebereichs (Abschnitt 11): `N/cli/completions.rs` (Zeilen 77 und 122, Belegprüfung), `A/launcher/se/DeviceKey.kt` (Kopfkommentar), `A/launcher/pin/PinRequestActivity.kt` (Treffer 130-144).

Gesucht, nicht gelesen: `N/share/core/discovery_pake.rs` (nur Namen), `N/share/core/discovery_exchange.rs` (nur Signaturen), `N/share/os/shared/discovery_relation_store_adapter.rs` (Treffer).

## 11. Nicht gelesen oder außerhalb des Lesebereichs

- Nicht gelesen, weil im Lesebereich nicht enthalten: `N/cli/setup/share_peer.rs` (Implementierung von `add_peer`, `add_room`), `N/daemon/os/shared/discovery_pairing_ipc.rs`, `discovery_pairing_protocol.rs`, `access_ipc.rs`, `devices_ipc.rs`, `exec_ipc.rs`, `apps_ipc.rs`, `N/screen/**`, `N/rdp/**`, `N/cli/completions.rs` (außer Zeilen 77 und 122).
- Außerhalb des wörtlichen Glob `ipc*.rs`: die Dateien `*_ipc.rs` im Verzeichnis `N/daemon/os/shared/`.
- Außerhalb des Lesebereichs gelesen: `N/cli/completions.rs` (Zeilen 77 und 122, nur für die Belegprüfung von `share-exec.md`; keine Aussage dieses Dokuments stützt sich darauf), `A/launcher/se/DeviceKey.kt` (Kopfkommentar, nicht verwendet), `A/launcher/pin/PinRequestActivity.kt` (Treffer; dies ist ein Start-Dialog für Verknüpfungen und Widgets, keine Gerätekopplung).
- Graphify: nicht verwendet. Die Abfrage liest `graphify-out/graph.json`, das außerhalb des Lesebereichs liegt. AGENTS.md verlangt die Abfrage vor Codefragen, diese Vorgabe konnte wegen des Lesebereichs nicht erfüllt werden.
- Build, Test und Ausführung: nicht ausgeführt.

Annahmen (explizit):
- A1: Die Standardwerte im Code gelten zur Laufzeit. Es gab keinen Laufzeittest.
- A2: Die Zeilenbelege aus dem Code gelten für den HEAD-Stand `015ea1f9`.
- A3: Die CLI-Befehle entsprechen den clap-Definitionen in den gelesenen Dateien; Zusatzbefehle in nicht gelesenen Dateien sind möglich.
- A4: Die Ordnerliste der Direkt-Freigabe gilt für alle Direkt-Geräte (siehe Abschnitt 9, Punkt 4).
