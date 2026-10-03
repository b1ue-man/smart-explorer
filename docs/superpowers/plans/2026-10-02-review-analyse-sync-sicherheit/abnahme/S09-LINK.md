# S09-LINK – Umsetzung und Abnahme

Stand: 2026-10-03. Begrenzter Anschluss des vorhandenen S09-Befunds nach
`umsetzung.md`; kein neues Projekt-Review. Lokale Ausführung ist ausgeschlossen.

## Endgültiger Meilensteinplan vor Umsetzung

| Meilenstein | Dateien / Grenze | Konkrete erwartete Wirkung |
|---|---|---|
| Pure Status-/Pin-/Interface-Fakten | neue `core/lan_link_facts.rs`, `lan_link_wire.rs` | Vollständige aktuelle Direct-Pins; frische gegenseitige Zufallschallenge; Unknown-Netzstatus ist keine No-Uplink-Auskunft. Exakte lokale IP bestimmt genau ein aktuelles privates Interface, ohne Subnetzschätzung. |
| Begrenzter eigener TLS-Statuskanal | neue `os/shared/lan_link_transport.rs`, `lan_link_exchange.rs`; Parent-Node-/ALPN-Anschluss | Eigenes ALPN, separate begrenzte Zulassung und Probes; kurze Frames, harte Round-/Sessionfrist. Ausschließlich Status, keine Datei-, Schreib-, Exec- oder Rückfreigaberechte. Runtime-Tick liest/aktualisiert nur kurze Snapshots und startet Async-Probes. |
| Gemeinsamer Policy-/Worker-Verbrauch | `lan_runtime*`, `lan_uplink_policy`, `lan_uplink_evidence` | Nur offene aktuelle gepinnte Session mit selektiertem privaten IP-Pfad liefert PeerOnLink. Kurz gültige private Evidence enthält dieselben Pin-/Interface-Fakten; privilegierter Worker prüft aktuelle Profile und Interfaces erneut. |
| Verwerfen / Stop / Anschluss | eigene Änderungen, Parent-only Registrierungen | Close, Pfadwechsel, Entzug, Disable und unbekannte Facts liefern keine Startautorität. Durable Stop bleibt wiederholbar. Alte Peers ohne ALPN bleiben failclosed. |
| Self-Review / gemeinsame Remote-Suite | neue `lan_link_task_tests.rs`, drei Blockberichte | Positive echte private Session, Replay-/Beacon-/Relay-/Mehrdeutigkeits-/Pinwechsel-Ablehnung und Stop-/Entzugssignale. Nur statisches Parsing lokal; Ausführung durch die eine spätere Remote-Suite des Hauptagenten. |

Gesicherte API-Fragen: Iroh 1.0 liefert TLS `remote_id`, `close_reason`, aktuelle
`paths` und selektierte LocalTransportAddr::Ip(Some(IP))/TransportAddr::Ip-Pfade.
Direkte Pins werden über vorhandene DirectPeerIdentity::validate geprüft. Fehlender
OS-Netzverdict bleibt Unknown. Parent besitzt Module-/Node-/ALPN-/IPC-Registrierungen.

## Abschluss

Der begrenzte S09-LINK-Block ist implementiert und am eigenen Diff statisch geprüft.
Der Hauptagent hat die additiven Module-/Node-/ALPN-/Service-/IPC-Anschlüsse bereits
integriert. Eine Ausführungsabnahme steht ausschließlich in der gemeinsamen
Remote-Task-Suite des Hauptagenten an; lokal wurden keine Builds, Tests, Formatter,
Server oder Installationen ausgeführt.

### Zuordnung zum ursprünglichen S09-Befund

| Ursprüngliche Grenze | Umgesetzte Wirkung |
|---|---|
| Beacon-Sender konnte privilegierten Uplink beeinflussen | mDNS liefert nur Dialhinweise. PeerOnLink und private Worker-Evidence entstehen ausschließlich aus einer offenen gepinnten TLS-Session mit eigener frischer bestätigter Challenge. |
| Geschätztes Subnetz ersetzte tatsächliche LAN-Nähe | Nur der selektierte offene IP-Pfad mit bekannter exakter lokaler IP und privater tatsächlicher Remote-IP gilt. Die lokale IP muss genau ein aktuelles aktives OS-Interface bestimmen. Relay, Custom, fehlende IP und Mehrdeutigkeit liefern keine Autorität. |
| Gespeicherte/replaybare oder entzogene Peerdaten | Vollständiger aktueller Accepted-Contact-/Grant-Pin wird gegen TLS remote_id und Statusidentität geprüft; nach Challenge und beim Snapshot erneut. Jede Pfadrevision, Close, Entzug und Disable verwirft den RAM-Nachweis. Schnelles Wiederanschalten reaktiviert keinen alten Cache. |
| Privilegierter Worker prüfte nur Beacon-Evidence | Strikt versionierte private Evidence mit höchstens acht Sekunden Lease, vollständigem Pin und exaktem Interface. Worker lädt aktuelle Profile und Opt-in-Settings und prüft aktuelle InterfaceFacts erneut. |
| Netzstatus durfte fehlende Fakten als keinen Uplink behandeln | Fehlender oder veralteter vorhandener OS-Netzverdict bleibt Unknown. Nur explizite bekannte Statusauskunft wird gecacht; der Tick startet keine zusätzliche Netzwerkprobe. |

### Entscheidungen und bewahrtes Verhalten

- Eigenes ALPN `smart-explorer/paired-link/1` und eigene Zulassung mit acht eingehenden
  und vier ausgehenden Sessions. Kein Zugriff auf Datei-, Schreib-, Exec-, Repair-
  oder Rückfreigabe-Rechte. TLS-Halfopen-Zulassung bleibt bestehen.
- Gegenseitige Challenge in drei Statusframes, höchstens 4096 Bytes pro Frame,
  drei Sekunden absolute Roundfrist, 45 Sekunden Sessionfrist und höchstens 24 Runden.
  Probes sind auf 32 Peers, 16 private Adresskandidaten und vier gleichzeitige
  ausgehende Sessions begrenzt. Sie laufen asynchron außerhalb des IPC-Ticks.
- Vollständige Pin-, Challenge-, Pfad- und Interface-Fakten werden in Pure-Core-Typen
  validiert. Iroh-/RAM-Orchestrierung bleibt im portablen OS/shared-Adapter;
  der Core entdeckt keine OS-Fakten selbst.
- Ein empfangender Peer darf den Statuskanal auch nach DHCP-Konfiguration benutzen.
  Neue NAT-/DHCP-Startautorität verlangt bekannte routerlose Fakten mit DHCP=false;
  unbekanntes DHCP erteilt keine solche Autorität. Ein bereits selbst geteiltes
  Interface darf die vorhandene Freigabe weitertragen.
- Bestehende fünf Sekunden Start-Debounce, 90 Sekunden Stop-Grace bei bloßem
  Verbindungsverlust und dauerhafter retrybarer Stop bleiben erhalten. Entzug des
  letzten Pins auf einem aktiv geteilten Interface und Disable stoßen Stop an.
  Eine andere aktuelle autorisierte Session auf demselben Interface bleibt zulässig.
- Direct-/Room-Dateipfade, gespeicherte Endpunkte und vorhandene Server-Dialhinweise
  bleiben erhalten. Ältere Peers ohne Status-ALPN erhalten keine automatische
  Uplink-Startautorität.
- Private Evidence benutzt den vorhandenen sicheren Storagehelper und denselben
  Dateinamen. Alte Beacon-Envelopes werden abgelehnt. Der separate Worker besitzt
  keinen Iroh-Handle: nach Veröffentlichung bleibt die ausdrücklich begrenzte
  achtsekündige Lease-Grenze; Close und OS-Mutation sind nicht atomar gekoppelt.

### Exakte erstellte Dateien

- `native/src/share/core/lan_link_facts.rs`
- `native/src/share/core/lan_link_wire.rs`
- `native/src/share/core/lan_link_task_tests.rs`
- `native/src/share/os/shared/lan_link_transport.rs`
- `native/src/share/os/shared/lan_link_exchange.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/S09-LINK.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/S09-LINK.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/S09-LINK.md`

### Exakte geänderte vorhandene Dateien

- `native/src/share/os/shared/lan_uplink_evidence.rs`
- `native/src/daemon/os/shared/lan_runtime.rs`
- `native/src/daemon/os/shared/lan_runtime_presence.rs`
- `native/src/share/core/lan_uplink_policy.rs` (präzisierte Fakt-Semantik in Kommentaren;
  bestehende Policy-Fristen bleiben erhalten)

Parent-only Registrierungen, Cargo/Lock, Graph, Suite, Commits, Push und Release wurden
von diesem Subagenten nicht geändert oder ausgeführt.

### Exakte gelesene Dateien

Die erstellten und geänderten Dateien oben wurden gelesen und statisch geprüft.
Zusätzlich gelesen, jeweils nur für diesen Anschluss oder gezielte Symbole:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/s09-link.json`
- `docs/refs/iroh-authenticated-lan-paths.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/umsetzung.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-sicherheit.md`
- `native/Cargo.toml`
- `native/src/share/mod.rs`
- `native/src/share/core/node.rs`
- `native/src/share/core/node_accept.rs`
- `native/src/share/core/service.rs`
- `native/src/share/core/node_policy.rs`
- `native/src/share/core/node_restrictions.rs`
- `native/src/share/core/endpoint_routes.rs`
- `native/src/share/core/direct_relation.rs`
- `native/src/share/core/direct_protocol.rs`
- `native/src/share/core/types.rs`
- `native/src/share/core/session.rs`
- `native/src/share/core/framing.rs`
- `native/src/share/core/io_deadline.rs`
- `native/src/share/core/crypto.rs`
- `native/src/share/core/lan_presence_match.rs`
- `native/src/share/core/relation_rights.rs`
- `native/src/net/core/link_facts.rs`
- `native/src/net/mod.rs`
- `native/src/daemon/os/shared/lan_uplink_runtime.rs`
- `native/src/daemon/os/shared/ipc_host.rs`

Die Repository-Anweisungen aus AGENTS.md lagen als Nutzernachricht vor. Ein erster
Lesefehler für den im Scope genannten nicht existierenden `native/src/share/core/core.rs`
wurde durch die explizite Parent-Freigabe von `core/crypto.rs` behoben; keine eigene
Scope-Erweiterung. Primäre Iroh-1.0-API-Belege und endgültige Signaturen stehen in
[API-Delta](../api-delta/S09-LINK.md).

### Konkrete Signale für die eine spätere Remote-Suite

Die neuen reinen Grenzprüfungen stehen in `core/lan_link_task_tests.rs`:

- `review_task_s09_full_current_pins_are_required_for_channel_and_worker`
- `review_task_s09_exact_ip_rejects_ambiguous_interfaces_and_unknown_dhcp`
- `review_task_s09_ipv6_uses_exact_local_ip_and_rejects_wrong_scope`
- `review_task_s09_old_facts_and_changed_adapters_cannot_authorize`
- `review_task_s09_missing_net_verdict_is_unknown_not_no_uplink`
- `review_task_s09_challenge_replay_and_non_status_frames_are_rejected`

Für die tatsächliche integrierte Grenze muss dieselbe Remote-Suite außerdem beobachten:

| Szenario | Erwartetes Abnahmesignal |
|---|---|
| Zwei aktuell gepinnte Peers, echter selektierter privater Iroh-IP-Pfad, eindeutige OS-Fakten, bekannter Peer ohne eigenen Uplink, lokales Opt-in | Challenge bestätigt; beide Seiten erhalten frische Statusfakten. Nach vorhandener Debounce kann der Worker starten und dieselben aktuellen Pins/Interfaces bestätigen. |
| Ausschließlich korrekter/frischer mDNS-Beacon oder alter Peer ohne Status-ALPN | Dialhint ist sichtbar; keine PeerOnLink-Startautorität, keine verwendbare Worker-Evidence, kein NAT-/DHCP-Start. |
| Falscher TLS-Pin, geänderter Record/Node/Fingerprint, entzogenes Contact/Grant, Replay oder falsches Echo | Runde beziehungsweise Snapshot wird abgelehnt; kein privilegierter Start. Entzug des letzten tragenden Pins löst retrybaren Stop aus. |
| Relay, Custom, Ip(None), fremde/mehrdeutige lokale IP, falsches IPv6-Interface, geänderter Adapter oder unbekannte DHCP-/Internetfakten | Kein neuer Start. Unknown bleibt sichtbar; der empfangende DHCP-Peer kann Status austauschen, ohne Startautorität zu erhalten. |
| Close, Pfadwechsel und kurzer Wechsel zurück während einer Runde; Disable und sofortiges Reenable | Cache und Revision verhindern Wiederverwendung. Erst eine neue aktuelle Challenge auf gültigem Pfad kann wieder Autorität liefern. |
| Private Evidence abgelaufen, altes Beaconformat, aktuelle Profile/Opt-in/Interface vor Helperstart geändert | Helper verweigert Start. Gültige Lease ist höchstens acht Sekunden alt; die begrenzte Publikations-/Prozessgrenze wird als solche geprüft. |
| Viele Peers, blockierter Partner, übergroße Frames oder gehaltene Streamrunde | Grenzen und harte Fristen greifen; IPC-Tick bleibt bei RAM-Snapshots, FS-/Exec-Pools bleiben verfügbar. Stop bleibt nach Teilfehler wiederholbar. |

Statischer Self-Review umfasst Rust-Parsing, Dateigrößen, Whitespace und den eigenen
Diff; er ist keine Aussage über ausgeführte TLS-/OS-/Worker-Szenarien. Keine offene
zusätzliche Datei- oder Abhängigkeitsfreigabe. Owner-Grenzen und praktische Lease-
Beschränkung stehen in [Anfragen](../anfragen/S09-LINK.md).
