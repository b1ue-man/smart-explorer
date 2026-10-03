# CI-2-S-SIGNAL: reale LAN-Rundendiagnostik

Stand: 2026-10-03. Bezug: [Run 37150409255](https://github.com/b1ue-man/smart-explorer/actions/runs/37150409255),
Kandidat `ac9b475ff18f6320bedd408c5a03c091710ad01c`.
Die drei Verhaltensfehler sind noch nicht als behoben oder abgenommen gemeldet.
Ihr konkreter Round-Ursachennachweis bleibt offen. Gemäß enger Root-Nachsteuerung
wurde die vorhandene reale Fixturediagnostik geschlossen, damit derselbe Remote-Einstieg
den tatsächlichen Actorfehler statt nur des Beobachtungs-Timeouts liefert.

## Belegte Grenze und Entscheidungen

`/tmp/rv1-ci-second/s-signal.json` enthält für diese drei Symbole ausschließlich
`bounded observation failed: mutually confirmed status`:

- `share::lan_link_transport::task_tests::review_task_s09_transport_real_private_tls_round_has_only_status_rights`
- `share::lan_link_transport::task_tests::lifecycle::review_task_s09_transport_close_withdraw_disable_discard_cached_facts`
- `share::lan_link_transport::task_tests::lifecycle::review_task_s09_transport_expired_or_unknown_facts_fail_closed`

Die bisherige Fixture verwirft beide Eventempfänger und beobachtet das
Client-JoinResult nicht im Statuswaiter. Der reale LAN-ALPN-Acceptzweig verwirft
`accept_lan_link(...).await` mit `let _`. Damit ist ein früher Admission-/Roundfehler
aus diesen drei Fehlermeldungen nicht erkennbar. Das ist der belegte Diagnoseverlust.
Mutex-Contention oder eine Pfadrevision während der Runde sind aus den Quellen
mögliche Fehlerwege, aber durch diesen Lauf nicht als Ursache bewiesen.

Änderungen:

- `lan_link_transport.rs`: ausschließlich `cfg(test)`-Haken melden das reale
  `io::Error` für `inbound_slots`, `admission` oder `round` über den vorhandenen
  `ShareEvent::Error`-Sender. Enthalten sind Kanal-ID, selektierter tatsächlicher
  IP-Pfad, ErrorKind und höchstens 256 Zeichen Fehlertext. Der Roundfehler wird vor
  dem bestehenden Kanalcleanup beobachtet. Keine neuen Produktionsereignisse.
- `lan_link_transport_task_fixture.rs`: beide vorhandenen Empfänger bleiben erhalten.
  `wait_status(&mut self)` wertet den tatsächlichen beendeten Client-Join und
  die Diagnoseereignisse aus. Pro Empfänger/Poll werden höchstens 16 Ereignisse
  konsumiert; nur der konkrete LAN-Diagnosepräfix beendet die Beobachtung.
- Der Waiter akzeptiert weiterhin ausschließlich zwei aktuelle Snapshots.
  Bei Fehler oder Timeout nennt er aktuelle akzeptierte Pin-Node-IDs,
  Autorisierungsepoche, Hostfaktenalter/Uplinkzustand, Cachegröße,
  Kanalrevision/Controlepoche, selektierten IP-Pfad/Closegrund und die tatsächlich
  entdeckten Runner-Interface-Fakten. Diagnose-Reads verwenden `try_lock`.
  Lookup-/Pairingcodes, private Schlüssel und Challengebytes werden nicht ausgegeben.

`OBSERVE = 4 s`, die Pollpause von 10 ms, `ROUND_DEADLINE = 3 s`,
`SESSION_DEADLINE = 45 s` und alle TTL-/Refreshgrenzen bleiben unverändert.
Es wurden keine Busy-Retries, PathEvent-Ausnahmen, zusätzlichen Frames oder
Interface-/Bind-Verengungen ohne belegte Ursache eingeführt.

## Frisch geprüfte Iroh-1.0.0-Primärquellen

Prüfdatum: 2026-10-03; ausschließlich die fest verwendete Version 1.0.0 als Beleg.

- `path_events()` liefert Lifecycleereignisse; `paths_stream()` liefert beim
  ersten Poll einen Snapshot. Das sind unterschiedliche Beobachtungsverträge.
  [Connection-API](https://docs.rs/iroh/1.0.0/iroh/endpoint/struct.Connection.html#method.path_events).
- `PathStateReceiver::events()` erzeugt `sender.subscribe()` und einen
  `BroadcastStream`; es baut keinen Initialsnapshot auf. Ein pauschales
  Überspringen des ersten Ereignisses ist daraus nicht gerechtfertigt.
  [Versionierter PathWatcher-Quelltext, Definition ab Quellzeile 317](https://docs.rs/iroh/1.0.0/src/iroh/socket/remote_map/remote_state/path_watcher.rs.html#317-330).
- `Incoming::local_addr()` benutzt die vom QUIC-Stack berichtete lokale IP und
  die tatsächliche Remote-Adresse zur Transportadressenzuordnung.
  [Versionierter Connection-Quelltext, Quellzeilen 195–200](https://docs.rs/iroh/1.0.0/src/iroh/endpoint/connection.rs.html#195-200).
- Der Builder unterstützt die vorhandenen Wildcard-Sockets für IPv4 und IPv6;
  `bind_addr` ersetzt den Default der jeweiligen Adressfamilie. Das belegt keine
  Notwendigkeit, den Node auf ein erratenes Interface zu verengen.
  [Versionierter Endpoint-Quelltext, Abschnitt `bind_addr`](https://docs.rs/crate/iroh/1.0.0/source/src/endpoint.rs).

Folgerung für diesen Abschluss: Die bestehenden strengen IP-/Revisionsprüfungen
bleiben erhalten. Die Quellen beweisen den Initialsnapshot-Verdacht nicht als
Ursache der drei CI-Fehler. Aktuelle API-Seiten einer anderen Iroh-Version wurden
nicht als Beleg für 1.0.0 übernommen.

## Erhaltene Assertions und statischer Self-Review

Die sechs bestehenden `review_task_s09_transport_`-Tests und alle ihre Assertions
wurden nicht geändert. Die reale Verbindung muss weiter die TLSremote-ID,
gespeicherten vollen Kontakt-/Grant-Pins, die exakte private lokale IP, Remote-IP
und echte OS-Interfaceidentität erfüllen. Replay, Aliaswechsel, malformed/stalled
Frames, Pfadrevision einschließlich Rückkehr, Close, Withdrawal, Disable,
Unknown und monotone/Wallclock-/Host-TTL bleiben failclosed. Die Fixture prüft
weiter fehlende FS-/Exec-Autorität sowie unveränderte Application-/Repair-/Transition-Permits;
sie startet keine NAT-/ICS-Mutation.

Statischer Vergleich gegen den Inhalt vor der Änderung: Nach Entfernen der neuen
`cfg(test)`-Diagnosehaken entspricht der gesamte Transportquelltext exakt dem
Original. In der Fixture bleiben OS-Discovery, Pins, tatsächliche Connectionprüfungen,
Privilegienassertions, Freeze und Cleanup exakt erhalten. Der private Waiter benötigt
nun `&mut self`, um ein bereits beendetes echtes JoinResult zu übernehmen;
alle vorhandenen Aufrufer besitzen bereits eine mutable Fixture.

Beide geänderten Dateien wurden mit dem vorhandenen Tree-sitter-Rustparser ohne
Syntaxfehler statisch gelesen. Größen mit manueller Formatierungsreserve:

| Datei | Zeilen | Bytes |
|---|---:|---:|
| `native/src/share/os/shared/lan_link_transport.rs` | 454 | 15910 |
| `native/src/share/os/shared/lan_link_transport_task_fixture.rs` | 414 | 14446 |

Keine lokale Ausführung, Compiler-/Formatterprüfung, Test-/Server-/Installations-
oder Git-/CI-/Graph-/Releaseaktion. Das Parsing ist keine Rust-Typ- oder Laufzeitabnahme.

## Exaktes Dateiinventar

Gelesene Belege:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-2-s-signal.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-behavior-fixes.md`
- `/tmp/rv1-ci-second/s-signal.json`
- `docs/refs/rv1-remote-suite.md`
- `docs/refs/share-server-tls-auth.md` – zugehörige TLS-/Iroh-Abschnitte
- `docs/refs/iroh-authenticated-lan-paths.md` – enge Root-Read-Nachfreigabe

Gelesene vorhandene Quellen, teilweise nur passende Definitionen/Ausschnitte:

- `native/src/net/core/link_facts.rs`
- `native/src/share/core/lan_link_facts.rs`
- `native/src/share/core/lan_link_wire.rs`
- `native/src/share/core/node.rs`
- `native/src/share/core/node_accept.rs` – nur LAN-ALPN-Zweig/Fehlerweitergabe, Root-Nachfreigabe
- `native/src/share/core/node_restrictions.rs` – betroffene Active-/Node-Verweise
- `native/src/share/core/types.rs` – `ShareEvent`/Node-Eventvertrag
- `native/src/share/mod.rs` – zugehörige Modulregistrierungen
- `native/src/share/os/shared/lan_link_dial.rs`
- `native/src/share/os/shared/lan_link_exchange.rs`
- `native/src/share/os/shared/lan_link_transport.rs`
- `native/src/share/os/shared/lan_link_transport_task_fixture.rs`
- `native/src/share/os/shared/lan_link_transport_task_lifecycle_tests.rs`
- `native/src/share/os/shared/lan_link_transport_task_tests.rs`

Zusätzlich wurden die oben verlinkten offiziellen versionierten API-/Quelltexte
sowie die zugehörigen offiziellen `PathEvent`-/`PathEventStream`-APIseiten gelesen.

Geändert:

- `native/src/share/os/shared/lan_link_transport.rs` – ausschließlich Testdiagnostik
- `native/src/share/os/shared/lan_link_transport_task_fixture.rs`

Erstellt und für den Self-Review gelesen:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-2-S-SIGNAL.md`

## Offener Root-Anschluss

Keine fehlende Definition, Dependency oder weitere Scope-Anfrage. Der verbleibende
Ursachennachweis benötigt die reale Actordiagnose aus demselben Root-eigenen
Remote-Einstieg. Die drei Verhaltensbefunde bleiben bis zur belegten Korrektur und
erfolgreichen gemeinsamen Auswertung offen. Es wird keine neue Suite oder einzelne
lokale/remote Testausführung angefordert; Root besitzt Integration und denselben
RV1-Wiederholungsanschluss.
