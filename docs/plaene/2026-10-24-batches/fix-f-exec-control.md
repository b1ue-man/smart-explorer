# M09/F13: Exec-Kontrollleser nach erfolgreichem Abschluss

Stand: 2026-10-26. Autorbefund, Vorplan und spätere eigene statische
Anti-Regressionsreview desselben Fixautors. Arbeitskopie
`/tmp/smartexplorer-review-fixes-ee75e91b`, Branch
`codex/review-fixes-ee75e91b`; gelesener HEAD `252f3fe8`.
Dieser Bericht behauptet keinen neuen Laufzeitpass und keinen Release.

## Auftrag und belegter Ausgangspunkt

Der Ergebnisauftrag ist: Ein erfolgreich abgeschlossener Exec-Besitzer beendet
seinen Kontrollleser unter Windows und Linux ohne neuen Cancel, Deadlock oder
Beeinflussung eines fremden Besitzers. Der Nutzervertrag vom 2026-10-26 in
`/tmp/smartexplorer-final-review-user-contract.txt` verlangt Fehlerfunktionsliste,
Verknüpfungskarte, konkreten Vorplan, Umsetzung und anschließende Prüfung durch
denselben Autor. Der abgeschlossene Funktionsplan
[`rv1-funktionsplan.md`](rv1-funktionsplan.md), Meilenstein M09, gibt diesen
Umfang frei. Die verbindliche Dateifläche steht in
`/tmp/smartexplorer-f-exec_control-scope.json`.

F13 stammt aus dem abgeschlossenen Remote-Lauf `40406246032`, Quellkandidat
`1ffbf4b0bc90b2479947680abcd0a5941cce00f8`. Das tatsächlich gelesene
`native-failures.json` meldet unter Windows im Fall
`daemon::exec_ipc::lifetime::batches_exec_lifetime_task_tests::batches_task_b5_terminal_registration_closes_control_without_cancelling_finished_owner`
einen Timeout bei `completed.recv_timeout(WAIT)`; die vorherige EOF-Assertion
des Clients ist bereits erreicht. Der Befund belegt einen nicht rechtzeitig
beendeten serverseitigen Kontrollleser. Er belegt keinen erneuten Cancel und
keinen Fehler des Remoteprogramms. Linux ist für diesen Befund nicht als
fehlgeschlagen aufgeführt.

## Erstplan aus dem Ergebnisauftrag

1. Den vorhandenen Aufbau-, Kontroll-, Cancel-, Abschluss- und Besitzervertrag
   an der freigegebenen Exec-Kette belegen; fehlende konkrete Quelldateien an
   Main melden.
2. Die dokumentierten TCP-/Conditionvariable-Verträge gegen geeignete
   Beendigungswege abgleichen. Die fehlende Leserbeendigung korrigieren,
   ohne den geordneten stdin-Pfad oder den Grant-/Journalpfad zu ändern.
3. Bestehende Abnahme erhalten, gezielte Ergänzungen im vorhandenen Testmodul
   anlegen und den vollständigen Diff selbst statisch gegen die Kettenkarte
   und Primärquellen prüfen. Main registriert die Ergänzungen in derselben
   Remote-Suite und besitzt Graphrefresh, Integration, Commit, Push und Release.

Aus dem Prompt bekannt sind Ergebnis, Plattformumfang und abgeschlossener
Fehllauf. Die tatsächliche Fehlerfunktion und der sichere Beendigungsweg
mussten aus dem Bestand und den Plattformverträgen geklärt werden. Die
Planungstiefe ist 2: Kontrollsocket, Ownerzustand, Setupübergabe und
unabhängiges Cancel-Signal sind gekoppelte bestehende Verträge.

## Eigene Fehlerfunktionsliste

| Einordnung | Funktion | Eigener Beleg und Ursache |
|---|---|---|
| Belegter Produktfehler | `exec_lifetime::serve_control` | Nach ACK werden die Readtimeouts entfernt. Danach wartet `read` unbeschränkt auf ein Kontrollbyte oder EOF und prüft `OwnerState.finished` nie. Die bereits vorhandene `changed`-Benachrichtigung wird von diesem Leser nicht verwendet. Der Windows-Fehllauf zeigt, dass die bisherige Socketbeendigung den Leser nicht rechtzeitig beendet. |
| Beteiligter Abschlussvertrag, kein separat belegter Fehler | `ExecLifetime::finish`, `Registration::drop` | `finish` setzt `finished` vor Socket-Shutdown und benachrichtigt `changed`; `drop` entfernt anschließend genau diesen Owner. Die Zustandsinformation ist vorhanden, wird vom Kontrollleser aber nicht beobachtet. |
| Kein belegter Fixturefehler | `batches_task_b5_terminal_registration_closes_control_without_cancelling_finished_owner` | Der Fall hält den Kontrollclient absichtlich am Leben und verlangt neben Client-EOF auch das tatsächliche Ende des Lesers. Diese unverkleinerte Anforderung entspricht dem Produktvertrag. |
| Ungeprüfte Detailhypothese | konkrete Windows-Kernel-/Handleursache des ausbleibenden Readabschlusses | Das Fehlerartefakt enthält keinen systemnahen Trace des wartenden `recv`. Der Fix darf keinen bestimmten Winsockfehler behaupten oder voraussetzen. |

## Direkte und indirekte Verknüpfungskarte

Die vollständige relevante Karte innerhalb der freigegebenen Grenze lautet:

1. `ipc::handle_client` authentifiziert die IPC-Anfrage und bindet die Runtimelease.
   `ExecStreamControlled` startet `SetupWatch` und `start_remote_cancellable`
   mit unveränderter absoluter 45-Sekunden-Frist. Der klassische
   `ExecStream`-Zweig bleibt erreichbar.
2. `exec_ipc::connect` erhält `ExecControlledReady` und verbindet über
   `connect_control` einen separaten Kontrollsocket; stdin/EOF/Resize verwenden
   weiterhin `ExecIpcInput.stream`, Cancel ausschließlich `ExecIpcInput.control`.
3. `serve_controlled` erzeugt den kryptografisch zufälligen 32-Byte-Token,
   registriert den Owner vor der sichtbaren Ready-Antwort, wartet auf
   `owner.wait_ready`, übergibt `SetupWatch` und ruft `exec_ipc::serve` auf.
4. `ExecControl` wird nach derselben IPC-Authentifizierung an `serve_control`
   geroutet. `ExecState::owner` verlangt Token und Exec-ID. `claim` erlaubt nur
   einen Kontrollleser und verwirft beendete beziehungsweise abgebrochene Owner.
   ACK und `ready` müssen vor dem normalen Kontrolllesen liegen.
5. `serve_control` besitzt den konkreten Owner als `Arc`. Im laufenden Zustand
   führen EOF, Reset oder ein unerwartetes Byte zu `owner.cancel`.
   `cancel` entscheidet unter der Ownermutex und sendet höchstens einmal,
   außerhalb dieser Mutex, über `ShareExecInput::send(Cancel)`.
6. `ShareExecInput::send` verwendet den durch `ShareExecSession::input`
   erhaltenen `ExecInputSender`. Dessen `send(Cancel)` schreibt ausschließlich
   in die Watch; die begrenzte Datenqueue wird nicht erworben. Nicht-Cancel-
   Eingaben warten in einer priorisierten Auswahl zwischen Cancel und Queue-
   Reserve. `forward` erhält diesen unabhängigen Cancelpfad und versendet den
   bestehenden Remote-Cancel. `exec_client::run_connected` beobachtet zusätzlich
   Abbruch während pausierter Routen. `spawn_connected_cancellable` hält den
   Runtime-Retainer bis zum tatsächlichen Ende von `run_connected`.
   `exec_client_active::run_with_notice` beendet den terminalen ResultAck/
   ResultAcknowledged-Austausch vor dem Terminalereignis und Taskerfolg;
   ein Kontrollabschluss darf diese Remotegrenze nicht ersetzen.
7. `exec_ipc::serve` meldet Authorized/Started/Stdout/Stderr/Terminal/Failed,
   aktualisiert `ExecState`, wartet über `ShareExecSession::finish` auf den
   Remote-Task und beendet seinen Eingabethread. Erst die Rückkehr aus diesem
   Scope lässt die `Registration` fallen. Der Fix betrifft die anschließende
   Kontrollbeendigung; Datenframing, Datenmutex und Remote-Task-Retention
   werden nicht geändert.
8. `Registration::drop` → `finish` → `finished = true` → Kontrollshutdown und
   `changed.notify_all`; anschließend `ExecState::remove_owner` mit
   `Arc::ptr_eq`. Ein neuer Owner mit gleicher Exec-ID und eigenem Token
   bleibt unabhängig. `cancel` nach `finished` darf keinen neuen Watchwechsel
   auslösen. `ExecIpcSession::drop` erhält seine Unterscheidung zwischen
   unvollständiger und bereits terminaler Sitzung, auch bei Inputklonen.
9. Der benachbarte `MutateExecGrant`-Zweig ist ein eigener Handler. Er erhält
   keine neue Aufrufkante aus dem Fix. `exec_grant_journal::execute_locked`
   behält Persistenz-CAS, Bestätigung des exakten Principals und der Policy,
   fail-closed Behandlung unbestätigter Freigaben sowie das Maskieren
   ausstehender Rechte. `exec_grant_journal_storage::write_entry` und
   `clear_entry` behalten private dauerhafte Speicherung, Operationsbindung,
   Verifikation und Recovery. Anfrage-/Antworttypen, gespeicherte Rechte,
   Journalphasen und Journalbytes werden nicht verändert.

Der vorhandene Rootgraph wurde ausschließlich lesend innerhalb der
freigegebenen Dateigrenze mit den tatsächlich vorhandenen Vokabeln `exec`,
`lifetime`, `control`, `finish`, `cancel`, `owner`, `registration` traversiert.
Der Graph bestätigt diese Modulverbindungen, enthält aber auch als INFERRED
markierte Namenszuordnungen; die obige Verhaltenskarte stammt aus gelesenem
Quelltext. Es wurde kein Graph geschrieben.

## Recherchepass 1: Beendigungswege

Abrufdatum aller folgenden Primärquellen: 2026-10-26. Die gelesene
Rust-Standardbibliotheksdokumentation weist Version 1.99.1 aus.
`set_nonblocking` ist dort seit 1.9.0 und `wait_timeout` seit 1.5.0 stabil;
das gelesene Projektmanifest verwendet Edition 2021. Die konkrete
Rustversion des nächsten Remote-Runners wurde hier nicht ermittelt.

- [Microsoft: Shutdown und Socketfreigabe](https://learn.microsoft.com/en-us/windows/win32/winsock/graceful-shutdown-linger-options-and-socket-closure-2)
  trennt die beiden Operationen. Der Leserabschluss muss als eigener Zustand
  beobachtbar sein; Client-EOF allein reicht für das F13-Orakel nicht.
- [Microsoft: `shutdown`](https://learn.microsoft.com/en-us/windows/win32/api/winsock/nf-winsock-shutdown)
  beschreibt das Deaktivieren von Senden/Empfangen, aber keine Freigabe der
  Socketressourcen. Der bestehende explizite Shutdown bleibt erhalten.
- [Rust: `TcpStream`](https://doc.rust-lang.org/std/net/struct.TcpStream.html#method.set_nonblocking)
  dokumentiert sofortige Rückkehr nichtblockierender Reads und `WouldBlock`
  bei fehlender Bereitschaft. Der Moduswechsel kann nach der letzten
  ACK-Schreiboperation ausschließlich auf dem separaten Kontrollsocket erfolgen.

Vorläufige Optionen waren wiederholte Readtimeouts oder nichtblockierendes
Lesen mit Zustandsbenachrichtigung. Eine neue OS-FFI, ein neuer Worker oder
ein neues Protokollbyte sind für den vorhandenen Ownerzustand nicht nötig.

## Recherchepass 2 und endgültiger Vorplan vor Quelledits

[Microsoft: `SO_RCVTIMEO`](https://learn.microsoft.com/th-th/windows/win32/winsock/sol-socket-socket-options)
empfiehlt nach einem abgelaufenen blockierenden Read das Schließen der
Verbindung. Der konkret gelesene Tabellentext dieser Microsoft-Lokalisierung
ist Englisch und enthält den Warnhinweis. Deshalb wird wiederholtes
Weiterlesen nach einem Sockettimeout verworfen. Die bestehende Handshakefrist
bleibt bestehen.
[Rust: `Condvar::wait_timeout`](https://doc.rust-lang.org/std/sync/struct.Condvar.html#method.wait_timeout)
gibt die Mutex atomar frei und verlangt die erneute Prüfung der Bedingung nach
dem Aufwachen. Der bereits vorhandene 25-ms-Takt in `wait_ready` und
`SetupWatch` wird für die Erkennung eines Peerabbruchs wiederverwendet;
Ownerabschluss weckt unmittelbar über `changed`.

| Teil | Geplanter Quellumfang | Abhängigkeit | Konkretes erwartetes Ergebnis |
|---|---|---|---|
| E1 Kontrollabschluss | `exec_lifetime.rs`: benannter vorhandener 25-ms-Takt; privater nichtblockierender Kontrollleser mit `finished`-Prädikat und `changed.wait_timeout`; Moduswechsel erst nach ACK | vorhandene Ready-/Ownerprüfung | Kein blockierender Socketread nach ACK; Abschluss wird vor dem nächsten Read unter derselben Ownermutex erkannt. `WouldBlock` und `Interrupted` führen zum Warten, EOF/Byte/echter Fehler erhalten Cancel des laufenden Besitzers. |
| E2 Abschluss ohne Shutdownwirkung | `batches_exec_lifetime_task_tests.rs`: neuer realer Socketfall mit zurückgehaltenem Owner-Shutdownhandle | E1 | Der Leser endet über den Ownerzustand selbst dann, wenn kein Socketshutdown den Read beenden kann; kein neuer Cancel, Owner wird entfernt. Bestehender F13-Fall bleibt mit Client-EOF und Leserjoin unverändert erhalten. |
| E3 Besitz und laufender Abbruch | dasselbe Testmodul: idle Kontrollverbindung und zwei Owner mit gleicher Exec-ID, getrennten Tokens und getrennten Cancel-Watches | E1 | Idle löst keinen Cancel aus. Abschluss des ersten Owners lässt den zweiten Owner registriert und laufend; falscher/alter Kontrollbesitz bleibt verweigert. Peer-EOF beziehungsweise unerwartetes Kontrollbyte brechen den gebundenen laufenden Owner weiter ab. |
| E4 Autorprüfung | derselbe Bericht und ausschließlich statischer Rustparser/Textdiff | E1–E3 und vervollständigte Kette | Kein OS-spezifischer Import, keine Protokoll-/Persistenzänderung, keine neue Datenmutex-Abhängigkeit; Sperrfolge und Wake-Rennen gegen Quellverträge geprüft. Die lokale Abgabe ist keine Laufzeitabnahme. |

Der Erhaltungsvertrag umfasst genau: absolute Setupfrist, ACK vor Handoff,
Single-Claim, zufällige Capability mit Exec-ID-Abgleich, unabhängiger Cancel
bei voller stdin-Queue und gehaltener stdin-Mutex, höchstens ein laufender
Cancel, kein neuer Cancel nach Abschluss, unveränderte Terminal-/Failed-
Weiterleitung, Remote-Task-/Runtimelease-Retention, genaue Ownerentfernung,
klassisches ExecStream, Grant-/Journalpersistenz sowie erfolgreich beendete
Sitzungen mit noch gehaltenen Inputklonen.

## Umsetzung und eigene anschließende Anti-Regressionsreview

E1–E4 sind durch denselben Autor umgesetzt und statisch geprüft:

- `exec_lifetime.rs` benennt den bereits verwendeten 25-ms-Takt und verwendet
  ihn unverändert in `wait_ready`. `serve_control` schaltet ausschließlich
  seinen Kontrollsocket nach vollständig geschriebenem ACK und vor `ready`
  nichtblockierend. Der private `wait_control` hält das Endprädikat und den
  jeweils sofort zurückkehrenden Read unter derselben Ownermutex. Bei
  `WouldBlock` und `Interrupted` gibt `changed.wait_timeout` diese Mutex frei.
  Nach jedem Aufwachen wird `finished` vor dem nächsten Read geprüft.
- `finish`, `cancel`, `claim`, `ready`, `Registration::drop`,
  `serve_controlled` und `connect_control` sind textgleich zum gelesenen
  Ausgangsstand. Das gilt auch für sämtliche bisherigen Funktionen im
  zugewiesenen Testmodul; kein bestehender Prüfkörper wurde verkleinert.
- Die Ergänzungen verwenden die vorhandenen echten Loopback-Kontrollsockets,
  die echte `ShareExecInput`-Watch und den vorhandenen Registrycode. Für den
  ausbleibenden Shutdown wird nur in der Fixture das gespeicherte Handle
  zurückgehalten; beide Socketendpunkte bleiben während des erwarteten
  Leserabschlusses offen. Zwei Owner mit identischer Exec-ID verwenden
  getrennte reale Input-Fixtures und getrennte Tokens.

Die anschließende eigene Anti-Regressionsreview prüfte folgende vollständige
Menge der betroffenen Grenzen:

| Prüffrage | Statischer Befund nach Umsetzung | Verbleibender praktischer Nachweis |
|---|---|---|
| Abschluss vor erstem Read oder während des Wartens | Derselbe Mutex schützt `finished` und den Read. `finish` kann während `wait_timeout` eintreten; Benachrichtigung plus erneute Prädikatsprüfung verhindert den verlorenen Wake. Auch ein bereits vorher gesetztes `finished` wird vor dem ersten Read erkannt. | vorhandener F13-Fall und neuer Fall ohne Shutdownhandle |
| Socketfehler infolge Abschluss | `finish` kann seinen Shutdown erst nach Erwerb derselben Mutex ausführen. Der Leser sieht danach `finished` vor einem weiteren Read und beendet sich erfolgreich; `owner.cancel` erzeugt wegen dieses Zustands kein neues Signal. | dieselben Abschlussfälle auf beiden Plattformen |
| Laufender Peerabbruch | EOF und unerwartetes Byte bleiben `Ok`-Rückkehr aus dem Leser mit anschließendem bestehendem Cancel; echte Readfehler bleiben Fehler mit Cancel. `WouldBlock` und `Interrupted` erzeugen keinen Cancel. | idle/EOF-Ergänzung, Byte-/Same-ID-Ergänzung und bestehender Fall mit voller stdin-Queue |
| Externer Job-Cancel bei offenem Kontrollpeer | `ExecLifetime::cancel` ist privat und wird im Produktionsmodul ausschließlich nach Rückkehr aus dem Kontrollleser in `serve_control` aufgerufen. Seine `cancelled`-Marke und Benachrichtigung erreichen damit insbesondere den noch wartenden `wait_ready` bei gescheitertem Attachment. Externer Cancel läuft dagegen über `ExecState::cancel` direkt an `ShareExecInput`, setzt keinen `OwnerState` und muss den Remoteabschluss samt Retainer weiter erhalten. Der Kontrollleser endet deshalb über `Registration::drop`/`finish`; eine zusätzliche `cancelled`-Prüfung hätte für diese tatsächlichen Aufrufer keinen Effekt. Kein zusätzliches Fehlverhalten belegt und kein weiterer Quelledit daraus abgeleitet. | unveränderte Cancel-/Terminal-/Retainerfälle; lokaler Nachreview ist der Aufruferbeleg |
| Sperren und Rückstau | Der neue Socketread ist nichtblockierend. Das Warten gibt die Ownermutex frei; der anschließende Cancel sendet unverändert außerhalb dieser Mutex. Datenmutex und begrenzte stdin-Queue werden im neuen Leser nicht erworben. | bestehender Fall mit gehaltener stdin-Mutex und blockiertem Produzenten |
| ACK, Claim und Fristen | Der Moduswechsel liegt nach dem letzten ACK-Write und vor `ready`; keine ACK-Schreiboperation wird nichtblockierend. Single-Claim, Exec-ID/Token-Abgleich und absolute Attach-/Setupfristen bleiben unverändert. Der 25-ms-Takt ersetzt nur denselben bisherigen Literalwert. | bestehender Owner-/Duplicate-Fall und gemeinsame unveränderte Setupfälle |
| Besitzerisolation | Jeder Leser wartet auf genau seine `ExecLifetime`; `remove_owner` prüft weiterhin Token und `Arc::ptr_eq`. Ein fertiger Token ist nicht mehr auflösbar und kann den zweiten Owner gleicher Exec-ID nicht abbrechen. | Same-ID-Ergänzung mit zwei Cancel-Watches |
| Erfolg, Runtimelease und Retainer | Der Remote-ResultAck/ResultAcknowledged-Austausch, das Taskende und `ShareExecSession::finish` liegen weiterhin vor dem Abbau der Registrierung. Der Kontrollleser ersetzt keinen dieser Schritte. Terminal/Failed, klassische ExecStream-Anfragen und fertige `ExecIpcSession`-Inputklone bleiben unverändert. | vorhandene Terminal-, Session-Drop- und Retainerfälle derselben Suite |
| Grant-/Journalvertrag | Keine neue Kante vom Kontrollleser zur Grantmutation; Persistenz-CAS, exakte Policy-/Principalbestätigung, Fail-closed- und Recoverypfade sind unverändert. | bestehende Grant-/Journalfälle derselben Suite |
| Plattform und Ressourcen | Nur portable `std`-APIs, keine neue Abhängigkeit, OS-FFI, Queue oder Worker. Ein Byte wird wiederverwendet. Idle Kontrollleser warten im vorhandenen 25-ms-Takt; terminale Owner wecken direkt. Dieser periodische Readversuch betrifft ausschließlich den separaten Kontrollthread. | Windows-/Linux-Ausführung; eine Laufzeit- oder Lastmessung wurde lokal nicht durchgeführt |

Die APIprüfung wurde nach dem Quelledit gegen die oben genannten aktuellen
Rust-/Microsoft-Primärquellen wiederholt: `set_nonblocking(true)` liefert bei
fehlender Bereitschaft `WouldBlock`; `wait_timeout` liefert den wieder
erworbenen Guard und kann vorzeitig aufwachen. Der Code prüft das Prädikat
deshalb in einer Schleife. Die verworfene wiederholte Winsock-Timeoutstrategie
wurde nicht eingebaut.

Rein statischer Befund: Der vorhandene Tree-sitter-Rustparser meldet in beiden
geänderten Rustdateien keine Syntaxfehler. `git diff --check` für die beiden
Quellen und diesen Bericht meldet keinen Whitespacefehler. Der funktionsweise
Textvergleich mit `252f3fe8` bestätigt die oben benannten unveränderten
Bestandsfunktionen und Prüfkörper. Die Dateien liegen mit 286 Zeilen/9232 Byte
und 329 Zeilen/12770 Byte unter den Projektgrenzen. Diese Angaben sind keine
Rust-Typprüfung, keine Laufzeitabnahme und kein neuer Testpass.

## Erforderliche Fallregistrierung in derselben Remote-Suite

Main ergänzt die folgenden vollständigen Namen für Linux und Windows im
bestehenden Manifest und bestehendem Einstieg `native/test-review-task.sh`;
kein zusätzlicher Suiteeinstieg ist nötig:

| Ergänzender Fallname | Bindung an Vorplan und Ergebnis |
|---|---|
| `daemon::exec_ipc::lifetime::batches_exec_lifetime_task_tests::batches_task_b5_finished_owner_releases_control_reader_without_socket_shutdown` | E1/E2: echter Leser endet über Ownerzustand trotz fehlender Shutdownwirkung, ohne Cancel |
| `daemon::exec_ipc::lifetime::batches_exec_lifetime_task_tests::batches_task_b5_idle_control_owner_stays_live_until_peer_cancels` | E1/E3: Idle bleibt laufend, echter Peer-EOF bricht weiterhin ab |
| `daemon::exec_ipc::lifetime::batches_exec_lifetime_task_tests::batches_task_b5_finished_control_cannot_cancel_same_id_live_owner` | E1/E3: Abschluss und alter Token beeinflussen den zweiten Owner gleicher Exec-ID nicht; unerwartetes Kontrollbyte bricht ausschließlich den gebundenen laufenden Owner ab |

Die unverändert zu erhaltenden bestehenden Fälle dieses Moduls heißen vollständig:

- `daemon::exec_ipc::lifetime::batches_exec_lifetime_task_tests::batches_task_b5_terminal_registration_closes_control_without_cancelling_finished_owner`
- `daemon::exec_ipc::lifetime::batches_exec_lifetime_task_tests::batches_task_b5_control_owner_cancels_full_stdin_without_acquiring_stdin_mutex`
- `daemon::exec_ipc::lifetime::batches_exec_lifetime_task_tests::batches_task_b5_session_drop_releases_control_and_data_despite_input_clone_and_lock`

## Dateifläche, Entscheidungen und offene Abhängigkeiten

Vollständige Liste der tatsächlich gelesenen oder gezielt durchsuchten
Projektdateien; eine gezielte Suche beziehungsweise ein Auszug behauptet
keine vollständige Inhaltslektüre:

- `AGENTS.md` – Projektregeln; vollständig auch in der zugewiesenen Nutzernachricht.
- `docs/ARCHITEKTUR.md` – Architektur und bestehende Remote-Suite.
- `docs/plaene/2026-10-24-batches/rv1-funktionsplan.md` – freigegebener Fixplan.
- `docs/plaene/2026-10-24-batches/rv1-funktionsbefunde.json` – gezielt F13.
- `docs/plaene/2026-10-24-batches/fix-f-exec-control.md` – eigener Bericht.
- `graphify-out/graph.json` – lesende, auf die erlaubten Quellen eingeschränkte Traversierung.
- `native/review-task-manifest.json` – gezielte Suche und lesende JSON-Auswertung.
- `native/Cargo.toml` – gezielte Abfrage von Edition und vorhandenen Abhängigkeiten.
- `native/src/daemon/os/shared/exec_lifetime.rs`.
- `native/src/daemon/os/shared/batches_exec_lifetime_task_tests.rs`.
- `native/src/daemon/os/shared/exec_connect.rs`.
- `native/src/daemon/os/windows/exec_connect.rs`.
- `native/src/daemon/os/linux_os/exec_connect.rs`.
- `native/src/daemon/os/shared/exec_ipc.rs`.
- `native/src/daemon/os/shared/exec_ipc_tests.rs`.
- `native/src/daemon/os/shared/exec_state.rs`.
- `native/src/daemon/os/shared/ipc.rs` – Aufbau, Exec-/Grantzweige und benachbarter Kontrollleser.
- `native/src/daemon/os/shared/ipc_protocol.rs` – Exectypen, Tokenzuordnung und I/O-Reexports.
- `native/src/daemon/os/shared/ipc_protocol_io.rs`.
- `native/src/daemon/os/shared/ipc_protocol_bounds.rs` – gezielte Exec-/Timeoutsuche.
- `native/src/daemon/os/shared/exec_grant_journal.rs` – gezielte Suche und `execute_locked`.
- `native/src/daemon/os/shared/exec_grant_journal_storage.rs`.
- `native/src/daemon/os/shared/exec_grant_journal_tests.rs` – gezielte Execsuche.
- `native/src/cli/shell_session/registry.rs` – gezielte Execsuche.
- `native/src/share/core/exec_session.rs`.
- `native/src/share/core/batches_exec_fixture.rs`.
- `native/src/share/core/exec_client.rs` – Sender-/Spawnreexports, Session und verbundener Client.
- `native/src/share/core/exec_client_input.rs`.
- `native/src/share/core/exec_client_spawn.rs`.
- `native/src/share/core/exec_client_active.rs`.
- `native/src/share/core/exec_client_messages.rs`.

Vollständige Liste der gelesenen externen lokalen Auftrags-/Anweisungsdateien:

- `/tmp/smartexplorer-f-exec_control-scope.json`.
- `/tmp/smartexplorer-final-review-user-contract.txt`.
- `/tmp/smartexplorer-rv1-40406246032/native-failures.json` – gezielt F13.
- `/root/.codex/AGENTS.md`.
- `/root/.codex/skills/arbeitsweise/SKILL.md`.
- `/root/.agents/skills/arbeitsweise/SKILL.md`.
- `/root/.codex/skills/graphify/SKILL.md`.
- `/root/.codex/skills/graphify/references/query.md`.

Vollständige Liste der erstellten oder geänderten Dateien:

- Geändert: `native/src/daemon/os/shared/exec_lifetime.rs`.
- Geändert: `native/src/daemon/os/shared/batches_exec_lifetime_task_tests.rs`.
- Erstellt: `docs/plaene/2026-10-24-batches/fix-f-exec-control.md`.

Die sechs nachgemeldeten Lesedateien wurden von Main vor ihrer Lektüre
freigegeben: beide plattformspezifischen `exec_connect.rs` sowie
`exec_client.rs`, `exec_client_input.rs`, `exec_client_spawn.rs` und
`exec_client_active.rs`. Die Schreibfläche wurde nicht erweitert. Alle für
E1–E4 benötigten Aufrufergrenzen wurden vor dem Quelledit geklärt. Es besteht
keine offene Scopeabhängigkeit und keine zusätzliche Schreibanforderung.

Entscheidungen: vorhandenen Ownerzustand und Conditionvariable wiederverwenden;
Sockethandshake und Capability unverändert erhalten; keinen zusätzlichen
Protokollrahmen, keine Plattform-FFI und keinen neuen Worker einführen;
wiederholte Sockettimeouts aufgrund des Winsockvertrags verwerfen. Der
Graphrefresh und die Manifestregistrierung bleiben bei Main. Lokal wurden
keine Builds, Compiler, Formatter, Tests, Server, Installationen, Commits,
Pushes oder Releasebefehle ausgeführt und kein Unteragent gestartet.

Laufzeitfrage für denselben Remote-Einstieg: Beenden Windows und Linux den
realen Kontrollleser nach terminalem Ownerabschluss, während aktive EOF-/Byte-
Abbrüche, volle stdin-Queue und gleiche Exec-ID mit unabhängigem Besitzer
unverändert funktionieren? Registrierung und Ausführung besitzt Main. Der
Autorenzyklus endet mit dieser statischen Abgabe; der Windows-/Linux-
Laufzeitnachweis ist ausdrücklich noch unbestätigt.
