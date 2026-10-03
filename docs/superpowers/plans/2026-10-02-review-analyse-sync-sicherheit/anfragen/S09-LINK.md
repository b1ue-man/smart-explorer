# S09-LINK – Owner-Anschlüsse und Grenzen

Stand: 2026-10-03. Eigener begrenzter Block abgeschlossen; keine lokale Ausführungsabnahme.

| Anschluss | Owner | Stand / konkrete Grenze |
|---|---|---|
| Module, Nodefield, ALPN, Service-Snapshots, IPC-Felder | Hauptagent | Parent bestätigt Registrierung in share/mod.rs, node.rs/node_accept.rs/service.rs und ipc_host.rs: eigener Dispatch vor FS/Exec-App-Admission, Disable vor Close, aktuelle Grants/Snapshots und Hostfacts nach Lan-Lock-Abgabe. Diese Dateien wurden nur gelesen. Exakte APIs in api-delta/S09-LINK.md. |
| Frischer vorhandener OS-Netzcache | Hauptagent | Getter internet_verdict_snapshot ist als 810b0df9 integriert; keine eigene Netzanbieter-Probe. None / fehlende Fakten ergeben Unknown. |
| Pfadereignis-Stream | Hauptagent | futures-util std ohne Defaultfeatures samt vorhandenem Lock-Package ist als ceac4695 integriert. Keine eigene Cargo-/Lock-Auflösung. |
| Ausführungsabnahme / Integration | Hauptagent / eine Remote-Task-Suite | Die in abnahme/S09-LINK.md genannten Rust-Symbole und tatsächlichen TLS-/Pfad-/Worker-Szenarien zusammen abnehmen. Keine Tests/Compiler/Formatter/Server durch diesen Subagenten. |

Keine offene zusätzliche Datei- oder Cargo-Freigabe. Scope hatte zunächst
`native/src/share/core/core.rs` benannt; diese Datei existiert nicht. Parent hat
den tatsächlichen `core/crypto.rs`-Lesepfad freigegeben. Der vorhandene
DirectPeerIdentity::validate-Vertrag wird wiederverwendet.

Praktische Grenzen:

- Ältere Peers ohne paired-link/1-ALPN starten keine automatische Uplink-Freigabe;
  ihre bestehenden Direct-/Room-Datei- und Dialwege bleiben erhalten.
- Nicht verfügbare Internetfakten melden Unknown. Unbekanntes DHCP kann den kleinen
  Statuskanal benutzen, bekommt aber keine neue NAT-/DHCP-Startautorität.
- Der separate privilegierte Prozess erhält keinen Iroh-Handle. Er konsumiert eine
  private höchstens acht Sekunden gültige Lease, prüft aktuelle Opt-in-Settings,
  vollständige aktuelle Profile-Pins und aktuelles OS-Interface erneut. Kanal-/Pfad-
  invalidierung entfernt die RAM-Autorität sofort; der nächste Daemon-Tick publiziert
  den geänderten/leer gewordenen Snapshot. Nach Publikation bleibt die ausdrücklich
  begrenzte Lease-Grenze zwischen Prozessen, keine atomare Close-/OS-Mutationsgarantie.
- Reine Verbindungsunterbrechungen behalten die bestehende Stop-Grace und den
  dauerhaften retrybaren Stop. Ein entzogener letzter Pin auf einer laufend geteilten
  Schnittstelle stößt Stop sofort an; eine andere aktuelle autorisierte Session
  auf derselben Schnittstelle darf die Freigabe weitertragen.
- Probes benötigen aktuelle LAN- oder vorhandene Server-Dialhinweise. Diese Hinweise
  ändern weder Pins noch Grants; eine angenommene Subnetzzuordnung wird nie Autorität.

Keine zusätzliche Review-Runde und keine Änderung fremder Feature-Owner. Graph,
Suite, Commit/Push und Release bleiben beim Hauptagenten.
