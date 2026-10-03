# S09-LINK – API und Parent-Anschlüsse

Stand: 2026-10-03. Keine eigenen Cargo-/Registrierungsänderungen.

## Bereits integrierte Registrierungen durch Parent

Der Hauptagent hat die folgenden additiven Anschlüsse bestätigt; die zugehörigen
Dateien wurden von diesem Subagenten ausschließlich gelesen.

In `native/src/share/mod.rs` additiv:

```rust
#[path = "core/lan_link_facts.rs"]
pub(crate) mod lan_link_facts;
#[path = "core/lan_link_wire.rs"]
mod lan_link_wire;
#[path = "os/shared/lan_link_transport.rs"]
mod lan_link_transport;
#[path = "os/shared/lan_link_exchange.rs"]
mod lan_link_exchange;
```

`lan_link_task_tests.rs` ist bereits als cfg(test)-Kindmodul von lan_link_facts registriert.

In `native/src/share/core/node.rs`:

```rust
pub(super) lan_links: Arc<super::lan_link_transport::LanLinkTransport>,
// Initialisierung:
lan_links: Arc::new(super::lan_link_transport::LanLinkTransport::default()),
// Zusätzlich in Endpoint::builder(...).alpns(...):
super::lan_link_wire::LAN_LINK_ALPN.to_vec(),
```

Beim Stop zusätzlich `self.lan_links.disable()` vor Endpoint-Schließung aufrufen.
Eigene Node-Methoden stehen bereits im neuen Transport-Modul:

```rust
update_lan_link_host(self: &Arc<Self>, LanLinkHostFacts) -> io::Result<()>
lan_link_snapshot(&self) -> Vec<AuthenticatedLanFact>
accept_lan_link(self: Arc<Self>, Connection) -> impl Future<Output = io::Result<()>>
```

In `native/src/share/core/node_accept.rs` direkt nach erfolgreichem TLS-Handshake
und `drop(permit)`, **vor** `application_handshakes.enqueue`:

```rust
if connection.alpn() == super::lan_link_wire::LAN_LINK_ALPN {
    let _ = node.accept_lan_link(connection).await;
    return;
}
```

Eigener ALPN `smart-explorer/paired-link/1`; 8 eingehende und 4 ausgehende Sessions,
keine FS-/Exec-/Repair-Slots. TLS-Halfopen-Begrenzung bleibt vorhanden. Der Kanal
verarbeitet nur drei Statusframes pro gegenseitiger Challenge, maximal 4096 Bytes
pro Frame, 3 s absolute Roundfrist, 45 s Sessionfrist / höchstens 24 Runden. Aktuelle
Dialhinweise aus LAN oder bestehender Server-Presence; höchstens 32 Peers / 16 Kandidaten.

In `native/src/share/core/service.rs` nur die additiven Wrapper:

```rust
pub(crate) fn update_lan_link_host(&self, facts: super::lan_link_facts::LanLinkHostFacts) -> io::Result<()> {
    self.iroh.update_lan_link_host(facts)
}
pub(crate) fn lan_link_snapshot(&self) -> Vec<super::lan_link_facts::AuthenticatedLanFact> {
    self.iroh.lan_link_snapshot()
}
```

Beide Aufrufe erledigen keine Netzwerk-I/O. Sie benutzen try_lock, kopieren begrenzte
RAM-Fakten und starten gegebenenfalls Async-Probes auf dem vorhandenen Iroh-Runtime.

In `native/src/daemon/os/shared/ipc_host.rs::lan_tick` zusätzlich die aktuellen
`state.profiles.direct_grants.clone()` und `service.lan_link_snapshot()` übernehmen.
Ohne Service ist der Snapshot leer. Neue `LanTickInput`-Felder:

```rust
grants: &[DirectGrant],
paired_links: &[AuthenticatedLanFact],
```

Nach `lan.tick(...)` auch `let host_facts = lan.link_host_facts();` aufnehmen.
Lan-Lock abgeben; am dann aktuellen Service
`service.update_lan_link_host(host_facts)` aufrufen. Keinen Netzwerkaufruf oder
zusätzlichen Netzanbieter-Probe im IPC-Tick durchführen. Ein WouldBlock liefert
keine neue Autorität und wird beim nächsten Tick wieder versucht.

## Gemeinsame Fakten und Worker-Grenze

- `LanLinkHostFacts`: enabled, interfaces:Vec<InterfaceFacts>, shared_ifaces:Vec<u32>,
  own_uplink:OwnUplink::{Unknown,Present,Absent}. Eigener Empfang von DHCP darf den
  Statuskanal weiter benutzen. Unknown wird niemals Absent.
- `LanPeerPin`: PinOrigin::{Contact{id,lookup_id},Grant{device_id}}, device_id,
  public_key, fingerprint, node_id. Alle Felder werden am aktuellen Accepted-Record
  geprüft; PeerIdentity::validate bindet public_key/Fingerprint/Node, TLS remote_id
  muss exakt passen. Keine Room- oder Rückfreigabe-Autorität.
- `AuthenticatedLanFact`: voller Pin, PrivateInterface{index,adapter_id,name,local_ip},
  actual remote_addr/path_id/connection_id, bestätigte eigene Challenge, peer_uplink,
  confirmed_at/expires_at. Maximal 8 s Wallclock-Lease und 6 s monotone Cachefrist.
- Path-Ereignisse erhöhen eine Kanalrevision und verwerfen die Antwort; auch ein
  Wechsel zurück ist eine neue Revision. Close, geänderte Pins oder Disable erzeugen
  keinen gültigen Snapshot. Disable verändert zusätzlich eine monotone Control-Epoche,
  damit schnelles Wiederanschalten alte Antworten nicht reaktiviert.
- Exakte private lokale IP muss in genau einem aktuellen aktiven nicht-loopback
  Interface vorkommen. Actual remote muss private IP derselben Familie sein;
  fremder IPv6-Scope wird abgelehnt. Relay/Custom/Ip(None) und fehlender Pfad sind ungültig.
- `fact.can_share_on` verlangt bekannte routerlose Facts (kein Gateway, DHCP=false),
  oder das bereits selbst freigegebene Interface zur Fortsetzung. Unbekanntes DHCP
  erteilt keine neue Startautorität; routende/empfangende Interfaces bleiben Statuswege.
- `lan_uplink_evidence::publish(&[AuthenticatedLanFact])->Result<(),String>` ersetzt
  den alten Beacon-Snapshot. Derselbe private Dateiname, nun strikt versionierter
  begrenzter Lease-Envelope. Alte Beacondateien werden abgelehnt.
- `authorize(private_index,&[InterfaceFacts])->Result<(),String>` bleibt unverändert
  für beide bestehenden Worker-Aufrufer. Es prüft aktuelle Opt-in-Settings,
  ShareProfiles::load_checked(Home-Fakt), volle aktuelle Pins, TTL und exaktes aktuelles
  OS-Interface erneut; derselbe can_share_on-Verbrauch wie in PeerOnLink/Policy.

## Bereits integrierte Root-Abhängigkeiten

Root-Getter `UplinkRuntime::internet_verdict_snapshot(&self)->Option<&[u32]>`
(810b0df9): frischer vorhandener Cache, keine Probe. Root-Cargo-Anschluss
`futures-util={version="0.3",default-features=false,features=["std"]}` samt vorhandenem
Lock-Package (ceac4695). Keine eigene Manifest-/Lock-Auflösung.

## Primäre API-Belege

Geprüft am 2026-10-03 gegen Iroh 1.0.0: remote_id stammt aus TLS; paths enthält
aktuelle offene Pfade, path_events meldet Änderungen und endet bei Close.
[Iroh Connection](https://docs.rs/iroh/1.0.0/iroh/endpoint/struct.Connection.html).
is_selected bezeichnet den für Anwendungsdaten verwendeten Pfad; local_addr und
remote_addr sind tatsächliche Transportadressen.
[Iroh Path](https://docs.rs/iroh/1.0.0/iroh/endpoint/struct.Path.html).
Lokale IP kann fehlen; unbekannte Enumvarianten bleiben ablehnend.
[LocalTransportAddr](https://docs.rs/iroh/1.0.0/iroh/endpoint/enum.LocalTransportAddr.html),
[TransportAddr](https://docs.rs/iroh/1.0.0/iroh/enum.TransportAddr.html).

Exakte Dateien und Remote-Signale: [Abnahme](../abnahme/S09-LINK.md).
Owner-Anschlüsse und praktische Grenzen: [Anfragen](../anfragen/S09-LINK.md).
