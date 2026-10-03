# S09-TRANSPORT – Fixture und Remote-Abnahmesignale

Stand: 2026-10-03. Begrenzte Transportabnahme des vorhandenen S09-LINK-Vertrags,
kein neues Produkt-Review. Der konkrete API-/Fixture-Plan wurde vor Umsetzung
übergeben; erst nach ausdrücklichem Root-Go nach Abschluss der Produktänderungen
wurden diese Testdateien geschrieben.

## Umsetzung

Der vorhandene ShareIrohNode-Constructor erzeugt zwei echte In-Memory-Identitäten
und Nodes mit deaktiviertem Relay. Die positive Verbindung benutzt den tatsächlichen
IPv4-UDP-Port des Zielnodes und die von der Root-Suite entdeckte private Runner-IP.
Produktdispatch und produktiver Client führen die gegenseitige Drei-Frame-Challenge
über den echten gepinnten TLS-Kanal aus. Es entstehen keine Exports, Exec-Grants,
Dateien, gespeicherten Profile oder privilegierten Uplink-Starts.

Die Fixture benötigt zwingend diese Root-Suite-Werte:

- `SE_REVIEW_LAN_IP`: private oder Link-local IPv4, kein Loopback-/Public-Fallback.
- `SE_REVIEW_LAN_IFINDEX`: positiver aktueller OS-Interfaceindex.
- `SE_REVIEW_LAN_IFNAME`: aktueller OS-Interfacename.

RunnerLan::discover liest frische `crate::net::gather_interface_facts()` und verlangt
genau ein aktives nicht-loopback Interface mit dieser IP, diesem Index und diesem
Namen. Die echte adapter_id stammt aus denselben OS-Fakten. Nach TLS-Verbindung muss
selected_ip_path die exakte lokale Runner-IP und die tatsächliche entfernte
Runner-IP samt Zielport bestätigen; ein anderer Pfad besteht die Fixture nicht.

Eigene Uplink-Auskunft ist ein kontrollierter Fixturewert (Present, Absent beziehungsweise
Unknown), keine Behauptung über den realen Internetstatus des Runners. Die realen
Interface-Fakten werden dabei nicht umgeschrieben. Ohne Presence-/LAN-Kandidaten
startet update_lan_link_host keine zusätzlichen Probes.

## Tatsächliche Testsymbole

Die Root-Suite nimmt alle folgenden Symbole über den Pflichtprefix
`review_task_s09_transport_` auf. Es wurde keine lokale Testausführung vorgenommen.

| Symbol | Konkretes erwartetes Signal |
|---|---|
| `review_task_s09_transport_real_private_tls_round_has_only_status_rights` | Echter TLS-/IP-Kanal und Produkt-ALPN-Dispatch liefern auf beiden Seiten volle aktuelle Contact-/Grant-Pins, eigene verschiedene zufällige Challenges und frische achtsekündige Statusfakten. Actual lokale IP und echte adapter_id stimmen. Dieselbe Connection erhält keine FS-Bindung oder Exec-Jobs. |
| `review_task_s09_transport_rejects_wrong_pin_and_replayed_confirm` | Tatsächlicher TLS-Peer passt nicht zum gespeicherten vollständigen anderen Pin: produktiver Client lehnt ab. Wiederverwendeter Confirm über einen echten TLS-Stream wird verworfen und schließt den Kanal. Eine falsche Geräte-ID mit korrektem TLS-Schlüssel wird ebenfalls abgelehnt. |
| `review_task_s09_transport_path_revisions_reject_returned_path_evidence` | Nach einem echten positiven Austausch verwerfen zwei Revisionsereignisse den Cache. Identische zurückgekehrte Actual-Adressen erlauben keinen Confirm der alten Revision. Erst eine neue produktive Challenge kann Status wiederherstellen. |
| `review_task_s09_transport_close_withdraw_disable_discard_cached_facts` | Echtes Close, Contact-Entzug, Grant-Reconfirm und Disable/Reenable liefern keinen Snapshot und verwerfen alten Confirm. Disable wird auch bei absichtlich belegtem State-Lock geprüft: die Control-Epoche verhindert Reaktivierung. |
| `review_task_s09_transport_expired_or_unknown_facts_fail_closed` | Monotone Cachefrist, abgelaufene Wallclock-Lease und veraltete Hostfakten liefern keine Autorität. Unknown wird durch eine vollständige neue reale Runde transportiert, entfernt alte Autorität und lässt die Statussession weiterlaufen. |
| `review_task_s09_transport_malformed_and_stalled_frames_are_bounded` | Exec-/Write-Payloads und übergroße Frame-Länge werden auf echten TLS-Streams abgelehnt; eine gehaltene unvollständige Länge endet unter der produktiven absoluten Roundfrist. Dabei bleiben FS-/Exec-Autorität und zugehörige Pools unangetastet. |

`assert_status_only` prüft die konkrete Connection über
`SessionPolicy::is_bound(&Connection)`: keine FS-Bindung, leere FS-Sessiontabelle,
leere aktive Exec-Registry und History sowie unveränderte Application-/Repair-/
Runtime-Transition-Permits. Die zugrunde liegenden Getter wurden nur gelesen.

TLS-Dial, rohe Frame-Runden und Frame-Send haben drei Sekunden Frist;
Beobachtungen sind auf vier Sekunden begrenzt. Produktive Runden/Sessions behalten
ihre bestehenden Fristen. Fixture-Drop bricht den eigenen Clienttask ab und stoppt
beide Nodes außerhalb des Async-Runtimes; aufeinanderfolgende negative Fälle geben
die vorherige Fixture ausdrücklich frei. Es gibt keine absichtlich endlosen Sessions.

## Dateien erstellt

- `native/src/share/os/shared/lan_link_transport_task_tests.rs`
- `native/src/share/os/shared/lan_link_transport_task_fixture.rs`
- `native/src/share/os/shared/lan_link_transport_task_lifecycle_tests.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/S09-TRANSPORT.md`

## Vorhandene Datei geändert

- `native/src/share/os/shared/lan_link_transport.rs`: ausschließlich additive
  `cfg(test)`-/path-/Kindmodulregistrierung. Keine Produktlogik geändert.

Die beiden zusätzlichen kohäsiven CREATE-/READ-Pfade wurden vor dem Anlegen
ausdrücklich vom Root freigegeben und im Scope gespeichert. Fixturehelpers,
Protokollfälle und Lifecycle-Fälle sind getrennt, damit auch für spätere
Remote-Formatierung ausreichend Reserve unter der 500-Zeilen-Grenze bleibt.
Die Testdateien haben vor Remote-Formatierung 161 / 267 / 122 Zeilen und
liegen jeweils unter 11 KiB. Der Transport hat nur die Testregistrierung erhalten.

## Gelesene Dateien

In Plan- und Umsetzungsphase gelesen, teilweise nur gezielte Symbole:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/s09-transport-suite.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/S09-LINK.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/S09-LINK.md`
- `docs/refs/iroh-authenticated-lan-paths.md`
- `native/src/share/os/shared/lan_link_transport.rs`
- `native/src/share/os/shared/lan_link_exchange.rs`
- `native/src/share/core/lan_link_facts.rs`
- `native/src/share/core/lan_link_wire.rs`
- `native/src/share/core/lan_link_task_tests.rs`
- `native/src/share/core/node.rs`
- `native/src/share/core/identity.rs`
- `native/src/share/core/types.rs`
- `native/src/share/core/backend_tests.rs`
- `native/src/share/core/export_config.rs`
- `native/src/share/core/direct_protocol.rs`
- `native/src/net/core/link_facts.rs`
- `native/src/net/mod.rs`: nur InterfaceFacts-Getter/-Reexport.
- `native/src/share/core/node_policy.rs`: nur SessionPolicy-Bindungsgetter.
- `native/src/share/core/exec_registry.rs`: nur Registry-Snapshotgetter.
- Die erstellten eigenen Dateien oben im statischen Self-Review.

Repository-Anweisungen aus AGENTS.md lagen als Nutzernachricht vor. Keine
weitere Surface erkundet, keine anderen Feature-Dateien geändert.

## Self-Review, Entscheidungen und Restgrenzen

Rust-Tree-Sitter-Parsing, eigene Dateigrößen/Whitespace, Modulregistrierungen und
eigener Diff wurden statisch geprüft. Der lokale Produktdatei-Diff enthält nur die
Testregistrierung. Es wurden keine Compiler, Builds, Formatter, Tests, Server,
Installationen, Commits, Pushes, Graph- oder CI-/Release-Aufrufe ausgeführt.

Private Fixture-Zustandsänderungen erlauben die TTL-/Revisionsprüfung ohne lange
Sleeps und ohne neue Produktionshelper. Zwei path_changed-Aufrufe sind die ausdrücklich
freigegebene deterministische Revisionsabnahme auf einer echten offenen TLS/IP-Session;
sie beweisen keine physische NIC-Migration. Schlüssel der In-Memory-Fixtures sind
deterministisch und getrennt; die positive Challenge verwendet Produktzufall.

Kein fehlender API-/Read-Anschluss. Die tatsächliche Ausführungsabnahme samt
Remote-Formatierung gehört ausschließlich zur einen gemeinsamen Root-Suite.
Privilegierte NAT-/ICS-/Worker-Mutation ist kein Bestandteil dieser Transportfixture;
die begrenzte private Worker-Lease und ihre Prozessgrenze bleiben im
[S09-LINK-Bericht](S09-LINK.md) dokumentiert. Root besitzt Suite-Orchestrierung,
Integration, Graph, Commit/Push und Release. Begrenzter Fixtureblock abgeschlossen.
