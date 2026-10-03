# CI-3-S-SIGNAL: aktuelle LAN-Runden ohne Snapshot-/Revisionsabbruch

Stand: 2026-10-03. Beleg: vollständig beendeter
[Run 37157166735](https://github.com/b1ue-man/smart-explorer/actions/runs/37157166735),
Kandidat `71a8ca45697272453c213c9b0b5412d0cbed0f71`.
Die belegten Ursachen sind im Quellcode korrigiert; die gemeinsame Remote-Abnahme
des neuen Kandidaten steht weiterhin aus. Keine lokale Verhaltensausführung.

## Evidenz und begrenzter Plan

`/tmp/rv1-ci-third/s-signal.json` nennt tatsächliche Actorfehler:

- Client-/Server-Confirm bricht mit `LAN-Link-Pfad hat gewechselt` ab.
  Die echte private TLS/IP-Verbindung ist vorhanden, Hostfakten sind erst
  11–25 ms alt, während die Kanalrevision zwischen Erfassung und Confirm steigt.
- Ein Client meldet `WouldBlock: LAN-Link-Zulassung oder Snapshot belegt`.
  Die bisherige Roundbehandlung beendet auch bei kurz konkurrierenden lokalen
  Snapshot-Reads den Actor dauerhaft.
- Die Replay-Fixture erreicht dadurch ihre erste gültige raw Bestätigung nicht.

Stage eins: Die ungültige alte Bestätigung muss verworfen bleiben. Sie ist kein
Beweis, dass ein weiterhin aktueller offener Kanal keine neue Challenge mehr
beantworten kann. Lokale Lockkonkurrenz darf keinen übernommenen Alt-Snapshot
statt einer erneuten Prüfung rechtfertigen.

Stage zwei, vor den Änderungen gegen die Primärrefs geklärt:

1. Nur eine explizite Revisionsabweichung erhält einen privaten typisierten
   Fehler. Fehlender Kanal oder veränderte Disable-Epoche bleiben permanent.
2. Neue Challenges nach einer verworfenen Revision teilen dieselbe absolute
   Dreisekundenfrist. Lokale `WouldBlock`-Reads geben den Task zurück und prüfen
   innerhalb derselben Frist erneut. Es bleiben die vorhandenen 24 Versuche pro
   Peer-Actor und die 45-Sekunden-Sitzungsfrist.
3. Der vorhandene Pfadmonitor invalidiert vor dem nächsten Frame-Actor-Poll und
   gibt nach jedem Event den Poll zurück. Die bestehende Replay-Fixture verwendet
   einen real bestätigten Kanal und verlangt den neuen raw Servernonce ausdrücklich.

Erwartetes Ergebnis aller Meilensteine: dieselben bestehenden Transportfixtures
erreichen aktuelle gegenseitige Bestätigung; alte Revisionsnachweise, Replay,
Entzug, Stop, fehlende Fakten und abgelaufene Fristen bleiben ablehnend.

## Umsetzung und Entscheidungen

`lan_link_transport.rs` ergänzt ausschließlich den privaten Fehlertyp
`PathRevisionChanged: Error` mit bestehendem Meldungstext. `confirm(...) ->
io::Result<()>` prüft weiterhin aktive Freigabe, aktuelle volle Pins,
TLSremote-ID, exakte aktuelle selektierte IP/PathId, eindeutiges privates
OS-Interface und aktuelle Controlepoche. Erst eine danach festgestellte
Revisionsabweichung liefert `Interrupted` mit diesem konkreten Fehlerobjekt.
Dieser Fehler speichert keinen neuen Cacheeintrag. Kein fehlender Kanal oder
Disable-Epochenwechsel wird als retrybare Revision umgedeutet.

`lan_link_exchange.rs` erkennt ausschließlich dieses Fehlerobjekt, nicht
beliebige `Interrupted`-Fehler oder Meldungstexte. Ein verworfener Versuch führt
zu einem neuen Stream und frisch erzeugten Zufallsnonces. Er verwendet dieselbe
absolute `timeout_at`-Frist; die vorhandene Versuchsgrenze zählt auch verworfene
Runden. Nur nach einem tatsächlich erfolgreich abgeschlossenen Round beginnt
die nächste reguläre Frist; der Client behält davor seine Zweissekundenpause.
Falsche Echoes/Frames, geänderte Pins, echte andere selektierte IP-Pfade,
fehlende/veraltete Hostfakten, Disable und Stop bleiben harte Fehler.
Eine gültige Auskunft `OwnUplink::Unknown` bleibt ein abgeschlossener Round,
der keine No-uplink-Autorität speichert und die Statussitzung nutzbar hält.

Der private `snapshot(deadline, read)`-Helfer wiederholt ausschließlich synchrone
lokale Reads mit `WouldBlock`. Er prüft vor jedem Aufruf die absolute Frist;
alle Guards fallen innerhalb des Aufrufs, bevor der Task `yield_now().await`
ausführt. Jede Wiederholung von `confirm` prüft erneut die vollständige
Autorisierung und aktuellen OS-/Kanal-Fakten. IPC und Runtime-Tick erhalten
keine Netzwerkwarteoperation; ihre bisherigen nicht blockierenden Snapshot-APIs
bleiben unverändert.

Der vorhandene `run`-Select pollt den Monitor zuerst. Alle Ereignisse,
einschließlich Lagged, invalidieren weiterhin; keines wird übersprungen.
Nach jedem Event folgt ein Yield, damit eine Ereignisserie nicht unbeschränkt
innerhalb eines einzelnen Monitor-Polls läuft und die Round-Deadline weiter
gepollt wird. Der Frame-Future bleibt erhalten: kein Event startet einen
partiellen `read_exact`-/`write_all`-Vorgang neu. Monitorende und die bestehende
45-Sekundenfrist beenden weiterhin die Sitzung.

`lan_link_transport_task_tests.rs` verwendet im bestehenden Replayabschnitt
`Fixture::positive()` auf der tatsächlichen privaten TLS/IP-Verbindung.
Danach muss der erste raw Confirm einen Fact mit genau `first_nonce` erzeugen.
Die alte Bedingung „Cache nicht leer“ kann den übernommenen Warmstart-Fact nicht
als diese neue Bestätigung akzeptieren. Der anschließende Replay verwendet
weiter den alten Nonce, verlangt Kanalschluss und leeren Servercache.
Keine Schutzassertion wurde entfernt.

## Frisch geprüfte API-/Fehlerverträge

Prüfdatum: 2026-10-03. Die erlaubte Lock-Stanza bestätigt Tokio 1.52.3;
keine Dependencyänderung. Syntax und Grenzen sind vor der Sourceänderung in
[iroh-authenticated-lan-paths.md](../../../../refs/iroh-authenticated-lan-paths.md)
gesichert. Der Dokumentations-Kontextgate wurde von Root erfüllt.

- [Iroh 1.0.0 Connection](https://docs.rs/iroh/1.0.0/iroh/endpoint/struct.Connection.html):
  aktueller Pfadsnapshot und TLSremote-ID; keine Announce-/Subnetzautorität.
- [Iroh 1.0.0 PathWatcher-Quelle](https://docs.rs/iroh/1.0.0/src/iroh/socket/remote_map/remote_state/path_watcher.rs.html):
  `events()` abonniert Broadcast ohne Initialsnapshot. Writer ändern ihren
  Snapshot vor dem Event; aktuelle Pfadprüfung und Revisionsprüfung bleiben beide nötig.
- [Tokio 1.52.3 timeout_at](https://docs.rs/tokio/1.52.3/tokio/time/fn.timeout_at.html):
  absolute Tokio-Instant-Frist; sofort bereite Futures können auch spät `Ok`
  liefern. Deshalb die zusätzliche Fristprüfung vor Read/Confirm.
- [Tokio 1.52.3 yield_now](https://docs.rs/tokio/1.52.3/tokio/task/fn.yield_now.html):
  Rückgabe an Scheduler ohne Annahme einer bestimmten Taskreihenfolge.
- [Tokio 1.52.3 select](https://docs.rs/tokio/1.52.3/tokio/macro.select.html):
  `biased` pollt in Quellreihenfolge; die Monitorserie liefert nach jedem Event
  zurück. Einzelne Events canceln keine Frame-Reads.

Der Web-Reader konnte einige versionierte Seiten nicht laden; die exakt
freigegebenen URLs wurden anschließend direkt per begrenztem HTTPS-Read gelesen.
Keine andere Libraryversion wurde als Syntaxbeleg übernommen.

## Statischer Self-Review und erhaltene Acceptance

Alle drei geänderten Rustdateien sind mit dem vorhandenen Tree-sitter-Rustparser
syntaktisch fehlerfrei gelesen. Das ist keine Rust-Typ-/Laufzeitabnahme.

Statischer Textvergleich gegen den Inhalt vor der Änderung:

- Transport verändert nur Fehlertyp und Revisions-/Controlepochen-Unterscheidung.
  Cache, Snapshot, Disable, aktuelle Pin-/IP-/Interfaceprüfung, Admission,
  Pools und bestehende echte Actordiagnose bleiben exakt erhalten.
- `checked_path`, `send_frame` und `recv_frame` bleiben exakt erhalten;
  Framegröße, Challenge-/Answer-/Confirm-Vertrag und Echo-/Identitätsprüfung
  werden nicht ersetzt. Alle lokalen Await-Wiederholungen liegen unter derselben
  absoluten Roundfrist und halten keinen Mutexguard.
- Die Testdatei verändert nur den Replay-Warmstart und dessen stärkere
  Noncebeobachtung. Alle übrigen Tests/Assertions sind exakt erhalten;
  die Lifecycle-Fixture wurde überhaupt nicht geändert.

Größen mit manueller Formatierungsreserve:

| Datei | Zeilen | Bytes |
| --- | ---: | ---: |
| `native/src/share/os/shared/lan_link_exchange.rs` | 274 | 9223 |
| `native/src/share/os/shared/lan_link_transport.rs` | 463 | 16258 |
| `native/src/share/os/shared/lan_link_transport_task_tests.rs` | 269 | 9771 |

Akzeptanz ausschließlich im selben Root-eigenen RV1-Einstieg, bestehende Symbole:

- `share::lan_link_transport::task_tests::review_task_s09_transport_real_private_tls_round_has_only_status_rights`
- `share::lan_link_transport::task_tests::review_task_s09_transport_rejects_wrong_pin_and_replayed_confirm`
- `share::lan_link_transport::task_tests::review_task_s09_transport_malformed_and_stalled_frames_are_bounded`
- `share::lan_link_transport::task_tests::lifecycle::review_task_s09_transport_path_revisions_reject_returned_path_evidence`
- `share::lan_link_transport::task_tests::lifecycle::review_task_s09_transport_close_withdraw_disable_discard_cached_facts`
- `share::lan_link_transport::task_tests::lifecycle::review_task_s09_transport_expired_or_unknown_facts_fail_closed`

Erhalten bleiben volle Pins/Devicealias/TLS-Identität, reale private Runner-IP
und Adapteridentität, Replayablehnung, malformed/stalled Frist, Away-/Return-
Revisionsablehnung, Close, beidseitiger Entzug, Disable einschließlich belegtem
Cleanup-Lock sowie monotone, Wallclock- und Host-TTL. Unknown über den echten
Kanal erteilt weiterhin keine No-uplink-Autorität. FS-/Exec-Berechtigungen und
Application-/Repair-/Transition-Permits bleiben begrenzt; keine NAT-/ICS-Mutation.

## Exaktes Dateiinventar

Gelesene lokale Plan-/API-/Diagnosebelege:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-3-s-signal.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-closure-fixes.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-behavior-fixes.md` – passender S-SIGNAL-/Remote-Anschluss
- `/tmp/rv1-ci-third/s-signal.json`
- `docs/refs/iroh-authenticated-lan-paths.md`
- `docs/refs/rv1-remote-suite.md`
- `docs/refs/share-server-tls-auth.md` – Iroh-/TLS-/Challenge-Abschnitte
- `native/Cargo.lock` – ausschließlich erlaubte Zeilen 7048–7075
- `/root/.codex/skills/arbeitsweise/SKILL.md` – verpflichtende Arbeitsweise

Gelesene Quellen, teilweise nur zugehörige Definitionen/Ausschnitte:

- `native/src/share/core/lan_link_facts.rs`
- `native/src/share/core/lan_link_wire.rs`
- `native/src/share/core/node.rs` – Active-/Stop-Vertrag
- `native/src/share/core/node_accept.rs` – LAN-ALPN-/Fehlerzweig
- `native/src/share/core/node_restrictions.rs` – zugehörige Active-/Epochverweise
- `native/src/share/core/types.rs` – Autorisierungsepoche
- `native/src/share/os/shared/lan_link_dial.rs`
- `native/src/share/os/shared/lan_link_exchange.rs`
- `native/src/share/os/shared/lan_link_transport.rs`
- `native/src/share/os/shared/lan_link_transport_task_fixture.rs`
- `native/src/share/os/shared/lan_link_transport_task_lifecycle_tests.rs`
- `native/src/share/os/shared/lan_link_transport_task_tests.rs`

Zusätzlich die oben genannten offiziellen versionierten API-/Quelltexte.

Geändert:

- `docs/refs/iroh-authenticated-lan-paths.md`
- `native/src/share/os/shared/lan_link_exchange.rs`
- `native/src/share/os/shared/lan_link_transport.rs`
- `native/src/share/os/shared/lan_link_transport_task_tests.rs`

Erstellt und zum Self-Review gelesen:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-3-S-SIGNAL.md`

## Root-Anschluss und tatsächliche Restgrenze

Keine offene Definition, Dependency, neue Datei oder Parent-Registrierung nötig.
Die bestehenden APIs, ALPN-Dispatchs, Node-Felder und IPC-Snapshots bleiben gleich.
Root besitzt Integration, Commit/Push und Auswertung desselben RV1-Remote-Einstiegs.
Erst diese Auswertung bestätigt das Verhalten auf den tatsächlichen Windows-/Linux-
Runnerverbindungen. Keine neue Suite, keine lokale Compiler-/Formatter-/Test-/Server-
Ausführung, keine Installations-, Git-, CI-, Graph-, Release- oder Kinderaktion.
