# Iroh 1.0: authentifizierter LAN-Pfad für S09

Geprüft am 2026-10-03 gegen die fest verwendete `iroh = 1.0.0` API. Dieser Anschluss schließt den bereits dokumentierten S09-Befund; er ist kein neues Review.

- [Connection](https://docs.rs/iroh/1.0.0/iroh/endpoint/struct.Connection.html): `remote_id()` liefert die TLS-authentifizierte EndpointId. `close_reason()` kennzeichnet geschlossene Verbindungen. `paths()` liefert nur den aktuellen Snapshot offener Pfade, keine früheren Pfade.
- [Path](https://docs.rs/iroh/1.0.0/iroh/endpoint/struct.Path.html): `is_selected()` bezeichnet den für Anwendungsdaten benutzten Pfad; `is_ip()`/`is_relay()` unterscheiden direkte IP und Relay. `local_addr()` und `remote_addr()` liefern die tatsächlichen Transportadressen.
- [LocalTransportAddr](https://docs.rs/iroh/1.0.0/iroh/endpoint/enum.LocalTransportAddr.html): `Ip(Option<IpAddr>)`; fehlt die lokale IP, ist keine Interface-Zuordnung bestätigt. Der Enum ist non-exhaustive. Relay/Custom sind keine private direkte LAN-Bestätigung.

Folgerung für den vorhandenen S09-Vertrag: mDNS bleibt Dialhinweis. Nur ein frisch beantworteter Status über den gepinnten TLS-Kanal eines aktuellen akzeptierten Direct-Peers und ein selektierter direkter IP-Pfad mit bekannter lokaler IP dürfen eine Uplink-Anfrage liefern. Das Interface muss anhand dieser exakten lokalen IP eindeutig in den aktuellen OS-Fakten gefunden werden; annoncierte Remote-Adressen und Subnetzschätzungen sind kein Privilegiennachweis. Ein unbekannter/geschlossener/Relay-Pfad oder eine veraltete Antwort liefert keinen PeerOnLink. Ein eigener kleiner ALPN-Statuskanal gibt keine Datei-, Schreib- oder Exec-Rechte und behält deren bestehenden Autorisierungsgrenzen.

Der Peer meldet das Fehlen eines eigenen Uplinks im TLS-Kanal auf eine frische Challenge. Antwort, aktuelle volle Pins und Pfad werden vor dem privaten, höchstens zehn Sekunden gültigen Worker-Nachweis gemeinsam geprüft. Fehlende Netzauskunft meldet nicht versehentlich „kein Uplink“. Für ältere Peers ohne diesen Statuskanal gibt es keinen automatischen privilegierten Start.
