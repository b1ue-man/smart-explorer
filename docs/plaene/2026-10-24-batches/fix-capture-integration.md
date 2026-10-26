# CAP-R1–CAP-R5: Capture-Integrationsfix

Stand: 2026-10-26. Autor dieses Dokuments ist derselbe Agent, der die Fehlerketten,
den Fixplan, die Produktänderungen und die anschließende eigene Review bearbeitet.
Der übernommene Befund steht in [review-capture-integration.md](review-capture-integration.md).
Lokaler HEAD beim Dokumentationsgate: `105bd6a3`. Die parallelen Android- und
Rendereränderungen bleiben außerhalb der Schreibfläche.

## A – Auftrag, Grenzen und Erstplan

Der delegierte Auftrag vom 2026-10-26 verlangt die vollständige Behebung von
CAP-R1 bis CAP-R5. Erhalten bleiben Bildschirm- und Fensterfreigabe, Snapshot,
RAW und kodierte Records, eingeblendeter X11-Cursor, Familienkomposition,
Geometriewechsel, Portal-/PipeWire-Freigabe, Pause, Abbruch, Eingabereferenz und
die tatsächliche Lebensdauer von Bildern, Mappings, Records und Transportbytes.
Die gemeinsame Parent-Bank W/X und die isolierte Workerarchitektur bleiben
maßgeblich. Es wird keine Funktion zur Umgehung eines Fehlers abgeschaltet.

Promptwissen: Die fünf gemeldeten Fehler betreffen Namensauflösung,
Cursorzusatzbesitz, Familienautorität, Ausgabesperre und Formatwechsel. Der
genaue Änderungsweg war vor der Bestandslektüre unbekannt. Die erste Frage war,
wie Zusatzpläne, Bildgrants, Ausgabesperren und Referenzgenerationen tatsächlich
verkettet sind. Die zulässige Fläche steht in
`/tmp/smartexplorer-capture-integration-fix-scope.json`.

Builds, Compiler, Formatter, Tests, Server, Installationen, lange Prozesse,
Commits, Pushes, Graphupdates und Releases führt dieser Unteragent nicht aus.
Main besitzt Suite, Manifest, Git und Remoteorchestrierung. Vorbereitete
Fixturefälle gelten ausdrücklich nicht als ausgeführt oder bestanden.

## B – Bestandsbefund und vollständige betroffene Ketten

Der vorhandene Graph wurde mit `graphify query "CaptureGrant MemberAuthority
X11 cursor worker retirement transport PipeWire"` gelesen. Er lieferte breite
Treffer und eine Versionswarnung; die folgenden Entscheidungen beruhen auf der
konkreten Quelllektüre. Planungstiefe 2: mehrere bestehende gekoppelte Verträge,
keine neue Endnutzerfähigkeit und keine zusätzliche Bibliothek.

| Befund | Direkte und indirekte Funktionskette | Erhaltener Vertrag |
|---|---|---|
| CAP-R1 | Linuxmodul `x11_capture.rs` → Untermodul `x11_capture_shm.rs::Segment` → `CaptureSlice` | SHM-Anlage, Serverattach/-detach, lokales Unmapping und unsichere native Enden bleiben gezählt. |
| CAP-R2 | `X11Source::capture` → `x11_cursor::fetch` → `Gate::cursor/authorize` → `SessionAdmission/DelegatedSession` → `RemoteAuthority::approve` → `GrantRequest::FramePlan` → `CaptureBroker::resource/approve` → Ablösung des Parentgrants → `WorkerReader` → `read_owned/decode_owned` | Cursor-Rawpuffer und ARGB-Vec werden vor Wachstum zugelassen; Bild-/Record-/Parentempfangsbesitz bleibt gültig. |
| CAP-R3 | `PreparedFamily::capture_plan` → `planning::prepare/member_admission` → `Surfaces::activate/grant.for_member` → X11-Cursorpfad → `MemberAuthority::approve` → `changed` → `FamilyScreenSource::capture/readmit/compose` | Echte Bild-/Geometrieänderungen bleiben koordinierte Familienänderungen; bloßer Cursorbedarf löst keine Wiederzulassung aus. |
| CAP-R4 | `host_worker_run::run` → `send_owned` → Transportclaim/-buffer → `CaptureSlice::drop` → `GrantInner::notify_retired` → `RemoteAuthority`-Observer → `output`; parallel `on_param_changed` → `CaptureMemory::format` → `CaptureGrant::retire` → derselbe Observer | Monotone Reduction-Snapshots und tatsächliches Ressourcenende bleiben erhalten; unter `output` wird kein Grant-/Slicehalter zerstört. |
| CAP-R5 | `Worker::produce/raw` → Capture/Skalierung/Encoder/Records → `run`-Referenzhülle → späterer Transportclaim; parallel PipeWire-Format-/Referenzwechsel → Grantretirement; anschließend Parentbroker/Workerreader/`WorkerSource` → Hostpipeline | Ein altes Bild bekommt keine neue Referenz. Erwartbares Retirement erzeugt einen wartenden Zustand; der Worker bleibt ansprechbar. |

Die Grantunterkette umfasst `capture_grant.rs`, `capture_grant_claims.rs`,
`capture_grant_slices.rs`, `capture_bytes.rs`, `capture_image.rs` und
`capture_records.rs`. Der unveränderte Elternempfang umfasst
`host_worker_reader.rs`, `capture_transport.rs` und `host_worker_source.rs`.
Prepare/Activate und Sessionidentität laufen über `host_worker_prepare.rs`,
`host_worker_prepared.rs`, `capture_broker_imports.rs`, `capture_session_context.rs`
und `capture_authority.rs`. Die Quellen-/Familienpläne werden aus
`capture_plan.rs`, `capture_source_contract.rs`, `host_codec_adapters.rs`,
`x11_capture_memory.rs`, `family_capture_plan.rs` und `pipewire_capture_memory.rs`
übernommen. PipeWire-Imports, ihre Callbackretirementtickets und ihre gemeinsame
Bank werden weiterverwendet, nicht ersetzt.

Zusätzliche betroffene Integrationsgrenze bei CAP-R5: Ein bereits vorbereiteter
alter Transport darf seine Parent-Empfangsberechtigung nicht verlieren, wenn
der asynchrone Kontrollthread den nächsten Bildplan bewilligt bekommt. Der
bisherige Broker retired den alten Parent schon bei dieser Bewilligung. Der
Parent muss deshalb bis zum geordneten Worker-Retirement verfügbar bleiben.
Temporär belegte alte Bildcredits dürfen den nächsten PipeWire-Bildplan
zurückstellen; sie dürfen keinen falschen Erfolg oder vorzeitige Freigabe ergeben.

## C – Recherche und Entscheidungen

Primärquellen erneut geprüft am 2026-10-26:

- [Rust Reference: Item scopes](https://doc.rust-lang.org/reference/names/scopes.html#item-scopes): Das SHM-Untermodul benötigt seinen konkreten Import.
- [Rust std: Mutex](https://doc.rust-lang.org/std/sync/struct.Mutex.html#method.lock): Der Guard hält die Sperre bis zu seinem Drop. Die Lösung verwendet getrennte Besitz- und Schreibscopes.
- [Rust Reference: Destructors und Drop scopes](https://doc.rust-lang.org/reference/destructors.html#drop-scopes): Der Schreibguard endet innerhalb von `PreparedReply::write`; Reply- und Transporthalter liegen beim Aufrufer. Felder und ihre wirklichen Allokationen enden in der ausdrücklich gehaltenen Reihenfolge.
- [PipeWire 0.3.65 thread-loop.h](https://raw.githubusercontent.com/PipeWire/pipewire/0.3.65/src/pipewire/thread-loop.h): Der Loop ist ein eigener Thread; Callbacks laufen unter seinem Lock. Ein Callback kann mit Workerencoding/-versand überlappen.
- Die lokal gepinnten Quellen `x11rb-0.13.2/src/cookie.rs` und `x11rb-protocol-0.13.2/src/protocol/xfixes.rs` definieren die getrennten Raw-/ARGB-Besitzer; Paketwachstum bleibt vor der Headerfreigabe zugelassen.

Gewählte Lösung: ein ausdrücklich zusätzlicher, an den Bildgrant gebundener
Cursorcredit mit ausschließlich Readback-/Metadatabesitz. Er benutzt dieselbe
Sessionbank, verändert weder Broker-`current` noch Parentpartition und wird
auch durch die delegierte Familienautorität unverändert weitergegeben. Der
Parent prüft den Bildgrant und dessen zugelassene Mitglieder selbst; ein
beliebiger Quellname oder eine fremde Generation erhält dadurch kein Recht.

Bildversand wird in Transportvorbereitung und reines Schreiben aufgeteilt.
Der Transportclaim entscheidet unter dem Grantmutex atomar, ob Retirement schon
begonnen hat. Ein bereits erworbener Claim bleibt nach Retirement gültig. Die
Bildhülle bindet die vor der Erzeugung beobachtete Referenz; ein während der
Erzeugung erkannter Wechsel oder ein wegen Retirement abgelehnter Claim wird
als `Unchanged` für Liveframes beziehungsweise `Paused(Starting)` für RAW
beantwortet. Bereits gelieferte konkrete Pausengründe und `NoDisplay`-Ablehnungen
bleiben erhalten. Encoder werden bei einem überholten Bild zurückgesetzt. Auch ein danach eintreffender
Wechsel kann eine alte Antwort nur noch unter ihrer alten Referenz versenden.

Der alte Parentgrant bleibt beim neuen Bildplan empfangsfähig. `RETIRED` entfernt
seinen Brokerregistereintrag in der geordneten Pipe. Bereits empfangene
Parentbilder und Records behalten ihren eigenen Plan für Snapshotkodierung und
Weiterleitung bis zum letzten tatsächlichen Parentbesitzer. Das Workerende
retired diese noch benötigte Parentfähigkeit nicht vorzeitig.

Verworfen: Cursorabschaltung oder Familienumgehung entfernt bestehende
Fähigkeiten. Ein bloßer Brokerfix lässt `MemberAuthority` weiterhin blockieren.
Ein zusätzlicher ungeschützter Generationenread schließt das Claimzeitfenster
nicht. Eine frühzeitige Freigabe des Bildcredits unterschlägt lebende Bytes.
Ein neuer Ausgabethread mit unbeschränkter Nachrichtenqueue würde einen neuen
Besitz-/Abbruchvertrag einführen; getrennte Schreibscopes lösen den belegten
Sperrzyklus innerhalb der bestehenden Architektur.

## D – Meilensteine und vorbereitete Abnahmesignale

| Schritt | Dateien | Abhängigkeit und erwartetes Ergebnis |
|---|---|---|
| 1. Cursorzusatzvertrag | `capture_admission.rs`, `capture_session_context.rs`, `capture_grant.rs`, Worker-Ressourcenwire, `capture_dispatch_requests.rs`, Broker, X11-Gate, Familienautorität | Gebundener Readback-/Metadatacredit erhält Bild-ID und Familienzustand; fremde Quelle, Generation, Import oder Bildowner werden abgewiesen. |
| 2. SHM-Import | `x11_capture_shm.rs` | `CaptureSlice` ist im Modul explizit auflösbar; nativer Abbau bleibt unverändert. |
| 3. Sicherer Bildtransport | `capture_grant_claims.rs`, `capture_transport.rs`, `capture_transport_wire.rs`, `host_worker_run.rs` | Transportclaim ist vor der Allokation vorhanden; Vorbereitung und Zerstörung laufen außerhalb der Ausgabesperre; RAW/Records behalten Originalreferenz. |
| 4. Format-/Parentübergang | Broker, `host_worker_run.rs`, `pipewire_capture_memory.rs`; unveränderter Empfang in `host_worker_source.rs` | Überholte Replies warten ohne Workerabbruch; alte vorbereitete Antworten bleiben bis zu ihrem Ende empfangbar; der nächste Bildplan kann bei temporärer Belegung erneut zugelassen werden. |
| 5. Fokussierte Fixtures und eigene Review | zwei freigegebene neue Fixturemodule, integrierte Modulregistrierung in freigegebenen Quelldateien, dieses Dokument | Zusatzcredit mit Parentbild und Familienmitglied; negative Identitäts-/Ownerfälle; Retirement vor/nach Transportclaim; gleichzeitige Reduction/Schreiben; Formatwechsel während Erzeugung; neuer Bildplan vor altem Transport; Folgeanfrage desselben Workers. |

Die zweite Rechercherunde hat die Mitgliedswhitelist in
`CapturePlan::for_source`, Parent-/Worker-Endbesitz und die bei einem wartenden
RAW-Reply erwartete Parentantwort geklärt: `WorkerSource` versteht bereits
`Paused`; ein vorübergehend nicht speicherbares Bild beendet nach
`host_session.rs::produce` nicht die laufende Sitzung. Eine Erweiterung der
unveränderten RAW-Antwortgrammatik war deshalb nicht erforderlich. Die Remoteausführung
dieser Fälle gehört ausschließlich zur einen von Main verwalteten Tasksuite.

## Fortschritt, eigene Review und Beleggrenzen

Die Produktänderungen und Fixturedeklarationen für die fünf Befunde sind
implementiert. Die eigene anschließende Review ist abgeschlossen; sie wurde
vom selben Fixautor ausgeführt. Der folgende Abschluss ist ein Quellbefund,
kein bestandener Compiler- oder Laufzeitlauf.

| Reviewfrage | Befund und geprüfter Erhalt |
|---|---|
| Endet ein Cursor noch vor seinem Raw-/ARGB-Besitzer? | Das Gate claimt beide Besitzer vor der Headerfreigabe und retired danach nur den separaten Cursorgrant. Raw- und Parsed-Slices reduzieren ihn getrennt; der Bildgrant bleibt aktiv. `Cookie::reply` und `PacketReader::advance` wurden in x11rb 0.13.2 erneut gelesen. |
| Kann der Worker fremde Familienmitglieder über den Cursorweg zulassen? | Der Parent nimmt die bestehende Bild-ID, prüft `current`, die im Bildplan enthaltene Quelle/Generation und ausschließlich Readback-/Metadata-Anforderungen. Cursorcredits haben keine Parentbildpartition und ersetzen `current` nicht. |
| Bleibt eine echte Familienänderung erkennbar? | `MemberAuthority::approve` setzt weiterhin `changed` und liefert `WouldBlock`; nur der ausdrücklich zusätzliche Vertrag delegiert. Die normale Kompositions-/Readmitentscheidung bleibt unverändert. |
| Gibt es weiter `output → notifications → output`? | Die Produktion kann `send_owned` nicht mehr aufrufen; diese Kompatibilitätshilfe ist nur unter `cfg(test)` vorhanden. `PreparedReply::write` leiht fertige Bytes und den Reply, hält nur den Schreibguard und zerstört keinen Grant-/Slicehalter. Erfolg und Schreibfehler verlassen denselben Guard-Scope. Die monotone Notification-Serialisierung bleibt unverändert. |
| Kann ein altes Bild eine neue Eingabereferenz erhalten? | Die Bilderzeugung wird von zwei Referenzbeobachtungen eingerahmt. Bei einem Wechsel wird das Bild verworfen; der atomare Transportclaim entscheidet ein nachfolgendes Retirement ohne Stringvergleich. Ein vorher geclaimter Transport darf mit seiner alten Referenz abschließen. |
| Werden Elternbilder, Snapshot oder Weiterleitung nach Workerende beschädigt? | Der alte Parent bleibt bis zum Empfang zugelassen. Ein empfangenes Bild behält seinen eigenen aktiven Plan auch nach Entfernung des Workerregistereintrags; der neue Brokerfall kodiert dieses RAW-Bild danach mit dem produktiven `AdmittedTilesEncoder`. Der bestehende Prozessende-/Readerendevertrag wurde nicht verkürzt. |
| Bleiben Fensterlayout und echte Pausengründe richtig? | Die neue Referenzhülle nimmt die Layoutrevision nach dem Capture, bindet ihre Generation aber an dessen vorherige Beobachtung. Dadurch geht die erste wieder veröffentlichte Familienrevision nicht verloren. Bestehende `Paused`-Gründe und `NoDisplay`-Ablehnungen werden bei gleichzeitigem Wechsel erhalten. |
| Kann alter Kredit die erneute PipeWire-Zulassung verbrauchen? | `WouldBlock` setzt den beobachteten Revisionsmarker zurück und kehrt ohne neue Bildallokation zum vorhandenen Wait zurück. Harte Ressourcenfehler bleiben Fehler; Importpause und Importretirement wurden nicht verändert. |

Im eigenen Review wurden die Parent-Weiterverwendung nach `RETIRED`, die nach
Capture nötige Familienlayoutrevision sowie der Erhalt konkreter Pause-/Denygründe
zusätzlich abgesichert. Ein eigener Fixture-Namenskonflikt durch ein lokales
`worker`-Binding wurde vor der Übergabe statisch entfernt. Es wurde kein
belegter weiterer Produktfehler in der bearbeiteten Kette offengelassen.

### Genaue vorbereitete Suitefälle

Die nachstehenden Kennungen sind nur deklarierte Fälle für Mains eine entfernte
Tasksuite. Main hat die Gruppenzuordnung `R9-CAP-R2-R5` und `R9-CAP-Linux`
gemeldet; dieser Agent hat das Manifest nicht bearbeitet und keinen Fall gestartet.

```text
batches_task_capture_integration_supplement_keeps_image_and_exact_live_owners
batches_task_capture_integration_delegated_supplement_preserves_member_identity
batches_task_capture_integration_supplement_rejects_foreign_identity_and_image_owners
batches_task_capture_integration_retirement_before_transport_is_a_nonallocating_stale_reply
batches_task_capture_integration_prepared_transport_finishes_under_its_original_reference
batches_task_capture_integration_cursor_credit_does_not_replace_parent_image
batches_task_capture_integration_broker_checks_cursor_member_whitelist_and_generation
batches_task_capture_integration_broker_admits_the_bound_family_member_cursor
batches_task_capture_integration_new_plan_keeps_an_old_prepared_reply_receivable
batches_task_capture_integration_remote_member_supplement_uses_the_image_book
batches_task_capture_integration_family_cursor_preserves_root_and_changed_flag
batches_task_capture_integration_worker_retired_transport_waits_then_answers_the_next_picture
batches_task_capture_integration_worker_turnover_during_capture_preserves_pause_and_liveness
batches_task_capture_integration_worker_turnover_preserves_native_pause_reason
batches_task_capture_integration_worker_prepared_picture_keeps_its_captured_reference
batches_task_capture_integration_worker_write_and_retirement_finish_without_lock_cycle
batches_task_capture_linux_x11_supplement_grants_raw_and_argb_without_retiring_image
batches_task_capture_linux_pipewire_retries_the_same_format_after_live_credit_pressure
```

Die Lockfixture erzwingt die konkrete Reviewreihenfolge: Der Bildwriter hält
`output`, Retirement ist bereits im Observer angekommen, erst dann darf der
Writer fortfahren. Sie prüft Schreiben und Schreibfehler, monotone
Reduction-Snapshots und den letzten tatsächlichen Creditbesitzer. Die
Turnoverfixture retired genau nach der letzten Generationsbeobachtung vor dem
Transportclaim und fordert danach beim selben Worker ein neues Bild an. Diese
gezielten Protokoll-/Lebensdauerfälle ersetzen keinen echten Compositorlauf.

### Gelesene Dateien

Die folgenden Dateien wurden vollständig oder in konkret benötigten Abschnitten
gelesen. Zusätzliche Suchaufrufe ohne vollständige Dateilektüre sind darunter
gesondert bezeichnet. Die beiden neu erstellten Fixturedateien und dieses Dokument wurden
bei der eigenen Review ebenfalls gelesen.

```text
/tmp/smartexplorer-capture-integration-fix-scope.json
/root/.codex/skills/graphify/SKILL.md
docs/ARCHITEKTUR.md
docs/plaene/2026-10-24-batches/review-capture-integration.md
docs/plaene/2026-10-24-batches/pipewire-import.md
native/src/screen/core/capture_admission.rs
native/src/screen/core/capture_authority.rs
native/src/screen/core/capture_session_context.rs
native/src/screen/core/capture_grant.rs
native/src/screen/core/capture_grant_claims.rs
native/src/screen/core/capture_grant_slices.rs
native/src/screen/core/capture_plan.rs
native/src/screen/core/capture_bytes.rs
native/src/screen/core/capture_records.rs
native/src/screen/core/capture_transport.rs
native/src/screen/core/capture_transport_wire.rs
native/src/screen/core/capture_test_support.rs
native/src/screen/core/worker_capture_wire.rs
native/src/screen/core/worker_capture_resource_wire.rs
native/src/screen/core/worker_wire.rs
native/src/screen/core/worker_reply_wire.rs
native/src/screen/core/batches_capture_worker_task_tests.rs
native/src/screen/core/host_backend.rs
native/src/screen/core/host_admission.rs
native/src/screen/core/host_capture.rs
native/src/screen/core/host_pipeline_produce.rs
native/src/screen/core/host_frame_send.rs
native/src/screen/core/host_hybrid.rs
native/src/screen/core/captured_external_hybrid.rs
native/src/screen/core/host_codec_adapters.rs
native/src/screen/core/admitted_tiles_encoder.rs
native/src/screen/core/host_codec_phase_plan.rs
native/src/screen/core/session_state.rs
native/src/screen/os/shared/capture_broker.rs
native/src/screen/os/shared/capture_broker_imports.rs
native/src/screen/os/shared/capture_dispatch.rs
native/src/screen/os/shared/capture_dispatch_requests.rs
native/src/screen/os/shared/host_worker_main.rs
native/src/screen/os/shared/host_worker_run.rs
native/src/screen/os/shared/host_worker_produce.rs
native/src/screen/os/shared/host_worker_prepare.rs
native/src/screen/os/shared/host_worker_prepared.rs
native/src/screen/os/shared/host_worker_reader.rs
native/src/screen/os/shared/host_worker_source.rs
native/src/screen/os/shared/host_worker_client.rs
native/src/screen/os/shared/host_worker_tests.rs
native/src/screen/os/shared/host_worker_main_tests.rs
native/src/screen/os/linux_os/x11_capture.rs
native/src/screen/os/linux_os/x11_capture_shm.rs
native/src/screen/os/linux_os/x11_capture_memory.rs
native/src/screen/os/linux_os/x11_admission_stream.rs
native/src/screen/os/linux_os/x11_cursor.rs
native/src/screen/os/linux_os/batches_capture_x11_task_tests.rs
native/src/screen/os/linux_os/pipewire_capture_memory.rs
native/src/screen/os/linux_os/pipewire_import_control.rs
native/src/screen/os/linux_os/pipewire_stream.rs
native/src/screen/os/linux_os/pipewire_pause.rs
native/src/screen/os/linux_os/portal_session.rs
native/src/screen/os/linux_os/batches_capture_pipewire_task_tests.rs
native/src/remote_window/os/shared/family_source.rs
native/src/remote_window/os/shared/family_capture_plan.rs
native/src/remote_window/os/shared/family_capture_members.rs
```

Zusätzliche gezielte `rg`-Suchfläche; die Liste umfasst auch trefferlose Dateien:

```text
native/Cargo.lock
native/review-task-manifest.json
native/src/screen/core/mod.rs
native/src/screen/core/frame_budget.rs
native/src/screen/core/frame_memory.rs
native/src/screen/core/host_stage.rs
native/src/screen/core/host_session.rs
native/src/screen/core/host_pipeline_actor.rs
native/src/screen/core/host_pipeline.rs
native/src/screen/core/host_server.rs
native/src/screen/core/host_backend_codecs.rs
native/src/screen/core/host_reference.rs
native/src/screen/core/captured_hybrid.rs
native/src/screen/os/mod.rs
native/src/screen/os/windows/host_backend.rs
native/src/screen/os/android/bridge_backend.rs
native/src/screen/os/shared/synthetic_backend.rs
docs/plaene/2026-10-24-batches/aufnahme-speicher.md
```

Die gezielte Suche nach Authorityimplementierungen und Versandaufrufern war auf
die konkreten Lesepfade des Manifests beschränkt. Trefferausgaben des
Graphqueries beruhen auf `graphify-out/graph.json`; keine dort genannte
zusätzliche Quelldatei wurde dadurch außerhalb der Lesefreigabe geöffnet.
Gepinnte Primärquellen aus dem vorhandenen Cratequelltext:
`x11rb-0.13.2/src/cookie.rs`, `x11rb-protocol-0.13.2/src/protocol/xfixes.rs`
und `x11rb-protocol-0.13.2/src/packet_reader.rs`.

### Erstellte und geänderte Dateien

Neu erstellt:

```text
docs/plaene/2026-10-24-batches/fix-capture-integration.md
native/src/screen/core/batches_capture_integration_fix_task_tests.rs
native/src/screen/os/shared/batches_capture_worker_fix_task_tests.rs
```

Geändert:

```text
native/src/screen/core/capture_admission.rs
native/src/screen/core/capture_grant.rs
native/src/screen/core/capture_grant_claims.rs
native/src/screen/core/capture_session_context.rs
native/src/screen/core/capture_transport.rs
native/src/screen/core/capture_transport_wire.rs
native/src/screen/core/worker_capture_resource_wire.rs
native/src/screen/os/shared/capture_broker.rs
native/src/screen/os/shared/capture_broker_imports.rs
native/src/screen/os/shared/capture_dispatch.rs
native/src/screen/os/shared/capture_dispatch_requests.rs
native/src/screen/os/shared/host_worker_run.rs
native/src/screen/os/linux_os/x11_capture_shm.rs
native/src/screen/os/linux_os/x11_admission_stream.rs
native/src/screen/os/linux_os/pipewire_capture_memory.rs
native/src/remote_window/os/shared/family_capture_members.rs
```

### Offene Belege und Übergabe

Der statische Diffcheck hat keinen Whitespacefehler gemeldet. Alle vom Agenten
geänderten und neu erstellten Rustdateien bleiben unter 500 Zeilen und 50 KiB.
Eine lokale Rust-Parserbibliothek war im vorhandenen `python3` nicht verfügbar;
es wurde dafür nichts installiert. Dieser Agent hat keinen Compiler, Formatter,
Build und Test gestartet. Kompilierbarkeit und Fixtureergebnisse bleiben bis zur
einen von Main verwalteten Remoteausführung unbelegt. Main besitzt auch den
erforderlichen abschließenden Rootgraphrefresh, Commit/Push und Release.

Ein X11-Server, eine reale Fensterfamilie und ein Portal-/PipeWire-Stream wurden
in diesem Agentenlauf nicht gestartet. Es fehlen deshalb Livebelege für
Darstellung, Treiber-/Compositorvarianten und die Eintrittshäufigkeit der Races.
Diese fehlenden Livebelege sind nach Projektvorgabe keine Releasevoraussetzung.
Die zwei separaten Release-Regressionsreviews wurden von diesem Fixautor weder
gestartet noch als bereits erledigt bezeichnet.
