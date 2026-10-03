# Anfragen von S-REVOKE

Stand: 2026-10-03. Der freigegebene V5-Vertrag wird fortgeführt. Eigene Implementierung und Self-Review
sind abgeschlossen; die folgenden Anschlüsse gehören den genannten Blöcken. Keine zusätzliche lokale
Abnahme oder gesonderte Prüfung ist angefordert.

## R0/R1 – Übernommener Vertragsstand

Die Ausgliederungen `direct_relation.rs`, `relation_rights.rs`, `signal_commands_local.rs` und die
additiven V5-Felder waren bereits freigegeben. Die Feldzeilen in fremden Konstruktionsstellen wurden im
vorherigen Vertragsabschnitt ergänzt; dieser Block verändert sie nicht erneut. Neue Literale benötigen
weiter `DirectGrant.write`, `DirectContact.relation`, `RoomProfile.policy`, `RoomMember.relation`.
Neue Grants nehmen `write: false`, neue Räume `RoomPolicy::new_room()`; bestehende Profile behalten ihre
früheren Schreibrechte. Der ursprüngliche Formatierhinweis ist Bestandteil der zentralen Remote-Suite,
kein Auftrag zur lokalen Ausführung.

## R2 – S-POLICY / nachfolgende Android-Bedienung

1. `ShareProfiles.direct_request_policy` bleibt mit Serde-Standard `Ask` nötig; neue unbekannte Geräte
   werden nur bei ausdrücklich gewähltem `AutoAccept` automatisch angenommen. Der Direct-Daemon und
   der Legacy-Entscheidungsweg konsumieren das Feld. Ein vorhandenes genaues `Accepted`-Grant oder
   `Reconfirm` mit aktuellem Code wird bestätigt; eine Schlüssel-/Knotensperre hat Vorrang.
2. Sperrende Änderungen in `profiles.rs::set_direct_grant` und `profile_edits` müssen
   `ShareProfiles::withdraw_direct_key(&peer, now)` verwenden. Der Helfer sperrt alle Schlüssel-/Knoten-
   Aliase, schaltet deren Exec monoton ab und markiert betroffene angenommene Legacy-Historie widerrufen.
   Ein frei behaupteter Geräte-ID-Wert entzieht keinem anderen Schlüssel ein Recht.
3. Entfernte Schlüssel werden über den historischen Wert `MAX_REMOVED_DIRECT_PEERS == 64` hinaus
   behalten. Eine alte Validierung darf diesen Wert nicht weiter als Persistenzlimit erzwingen oder
   Datensätze verwerfen. Der bestehende Bytehaushalt des Profilstores darf eine zu große Transaktion
   mit Fehler verweigern; er darf keine früheren Verbote entfernen. Angefragt beim Orchestrator.
4. `merge_contact` übernimmt eine geänderte `relation.share_back` über
   `set_contact_share_back(contact_id, value, now)`; `signed_presence` bleibt vom Worker gelernt und
   darf nie durch einen veralteten GUI-Snapshot gelöscht werden. `merge_room` verwendet
   `set_member_blocked` / `admit_member` und übernimmt bewusst geänderte `RoomPolicy`.
5. Android muss „Wieder erlauben / Bestätigen“ an `allow_direct_peer_again(default_home, device_id)`
   anschließen. Gegenseitig-Wahl und Schreib-/Raumrechte folgen dem V5-Vertrag. Der Orchestrator hat
   diese Übergabe an die zuständigen späteren Blöcke bestätigt. Desktop und `se share grants allow`
   sind in S-REVOKE angeschlossen.

## R3 – S-SIGNAL

1. Die Verbinder-Seite der PIN-Kopplung nimmt `UserPairingOneWay` als Standard und nur mit bewusstem
   Gegenseitig-Opt-in `UserPairing`; die Anbieter-Seite bleibt `UserPairing`. Bestehende Grants werden
   durch einseitiges Koppeln nicht gelöscht. Automatische Reparatur legt nur bei `share_back` oder
   bereits aktivem Grant eine eigene Freigabe an und bestätigt niemals `Reconfirm`.
2. `signal_publish.rs::send_direct_answer` konsumiert den bereitgestellten Helfer:

   ```rust
   crate::share::signal_presence::build_direct_decision_presence(
       &lookup_id, &requester_device_id, accepted, &identity, &secret, iroh,
   )?
   ```

   Exakte Signatur in `api-delta/S-REVOKE.md`; Integration wurde vom Orchestrator an S-SIGNAL übergeben.
   Der Nonce ist 126 ASCII-Bytes lang, bindet Empfänger und Entscheidungsbit und bleibt innerhalb der
   echten Drahtgrenze von 128 Bytes. Alte unsignierte Antworten bleiben nur im bereits bestehenden
   Pending-/Legacy-Weg kompatibel; sie überschreiben keine getrackte oder abgeschlossene Beziehung.
3. `ipc_host.rs::load_share_server` ruft nach Regularfile-/16-KiB-Prüfung
   `crate::share::migrate_server_file(&path)` auf. Diese zugeteilte Daemon-Integration ist erledigt.
   Der S-SIGNAL-Helfer muss schema-lose gespeicherte Adressen dauerhaft als `tcp://` erhalten.
4. S24 ist im zugeteilten Desktop-Ereignisweg erledigt: Der unbestätigte Kopplungshinweis wird erst
   nach erfolgreicher persistierter Entfernung vergessen. Direct-/Room-Removal geben dafür `bool`
   zurück. Andere Tail-Match-Aufrufer müssen gegebenenfalls die Rückgabe bewusst verwerfen.

## R4 – H-DISPATCH

1. Sitzungsautorisierung bleibt `SessionAuthorization`: Relation-Schreibrecht **und** schreibbarer
   Export sind erforderlich. `is_admitted()` schützt Räume zusätzlich zur Knotenschlüsselbindung.
2. `authorization_restrictions(current, candidate)` ist jetzt die konkrete Reduktionsmenge.
   Reine Erweiterungen und Laufzeitdaten bleiben ohne neue Epoche; ansonsten werden nur passende
   `(Schlüssel oder Knoten, Beziehung)` geschlossen. Ein unbekanntes globales Mapping bleibt
   konservativ. `ShareIrohNode::invalidate_restrictions(&RestrictionSet) -> io::Result<usize>` wird
   von `set_direct_online` konsumiert; H-DISPATCH besitzt Node/Sitzungen/Leases.
3. `exec_grant_runtime::apply_configuration_transition` muss vor der Snapshot-Übernahme weiter
   laufen. Es cancelt nur getroffene Principals, erhöht deren Launch-Barriere und erhält die Tokens
   unbeteiligter Arbeit. Offline/Room-Deaktivierung verändert keine gespeicherte Exec-Policyrevision.
4. Die bestehende Übernahme von `seen_nonces` bei Konfigurationswechseln erhalten. Darin liegt jetzt
   zusätzlich die sofortige, begrenzte Signatur-Erinnerung für neue Principals, bevor der erste
   Daemon-Ereignisstand gespeichert ist. Dauerhafte bekannte Flags bleiben monoton. Unparsebare
   Nonce-Formate anderer Autorisierungspfade werden nicht als abgelaufen behandelt.
5. S66-Ausgangsaufrufer in `node_sessions.rs::repair_direct_reciprocal` ist angeschlossen und für
   H-DISPATCH wieder freigegeben: Kein Transition-Permit während Wire-I/O; erst `persist_initiator`
   nimmt das Gate und wiederholt die aktuelle Autorisierung. Sein Anfangs-Generationstest bleibt.
   Die frühen Daemon-Rückgaben bei `reciprocal_repair_in_flight` wurden in Reload, Configure und
   Neustart entfernt; sie stellten bisher die Konfiguration während des gesamten Peer-I/O zurück.

## Abnahme durch den Orchestrator

Alle neuen Kernsignale heißen `review_task_*`; Transport- und bisherige Legacy-Tests wurden auf die
freigegebenen Schnittstellen bzw. ein ausdrücklich gesetztes `AutoAccept` umgestellt. Sie werden mit
den übrigen Meilensteinen ausschließlich in der einen Remote-Task-Suite ausgewertet. Der Orchestrator
übernimmt Graph-Aktualisierung, Integration, Commit/Push und CI; S-REVOKE führt keine dieser Aktivitäten aus.
