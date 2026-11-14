# Recherche: Transport zwischen zwei Programmen (Referenzuntersuchung externer Lösungen)

Stand: 2026-11-14. Gehört zu `plan.md` in diesem Ordner (Abschnitt B, Abschnitt C „offen bis zum Abschluss von recherche.md und bestand.md“).
Dieses Dokument enthält Befunde und Folgerungen. Es enthält keine Entscheidung; die Entscheidung steht in `plan.md` Abschnitt C.

## 0 Lesehinweise und Quellenregeln

Kennzeichen im Text:

- **[M]** veröffentlichter Messwert mit Quelle (Messaufbau steht dabei, soweit die Quelle ihn nennt).
- **[B]** belegte Aussage einer Quelle ohne Zahl.
- **[F]** Folgerung des Verfassers aus belegten Aussagen; die Prämissen stehen im selben Absatz.
- **[A]** Annahme; in keiner gelesenen Quelle belegt.
- **[U]** ungeklärt; die genannte Tatsache wurde nicht gefunden oder die Quelle war nicht lesbar. Alle [U]-Punkte stehen gesammelt in Abschnitt 9.

Quellenregeln:

- Quellen mit Kennung Q01 bis Q58 wurden am **2026-11-14** abgerufen (Verzeichnis in Abschnitt 10 mit URL, Version bzw. Seitendatum, Stelle und Lesestatus). Quellen mit Kennung L1 bis L9 sind lokale Dateien dieses Repositorys.
- Das Abrufwerkzeug lieferte für Webseiten einen maschinell erzeugten Auszug, nicht den Rohtext. Zahlen und Zitate aus Webseiten sind deshalb so zuverlässig wie dieser Auszug. PDF-Quellen Q04 (Seitenbilder gelesen), Q33, Q37, Q42, Q43 (Text mit `pdftotext` extrahiert und gelesen) wurden direkt gelesen.
- Aussagen, die aus `docs/refs/bildschirm-streaming-2026-10-17.md` (L5) oder `docs/refs/uebertragung-tempo-2026-10.md` (L3) übernommen sind, tragen den Vermerk „übernommen aus L3/L5, dort abgerufen 2026-10-17 bzw. 2026-10-21, am 2026-11-14 nicht erneut abgerufen“.
- „Ohne Systemadapter“ bedeutet: ohne virtuelles Netzwerkgerät (TUN/TAP/Wintun/VpnService-Schnittstelle) und ohne Treiberinstallation. „Ohne Adminrechte“ bedeutet: ohne Administrator unter Windows, ohne root oder `CAP_NET_ADMIN` unter Linux, ohne Systemdialog unter Android.

## 1 Ausgangslage in se (aus dem Quelltext, nur gelesen)

Diese Angaben bestimmen die Randbedingungen, unter denen jede Referenz bewertet wird.

- QUIC-Bibliothek: iroh 1.3.0 mit noq 1.3.0 und noq-proto 1.3.0, vendoriert (`native/Cargo.toml:42-44`, L9). TLS-Anbieter: iroh aktiviert Merkmal `tls-ring` (`vendor/iroh-1.3.0/Cargo.toml:72,93-96`); `native/Cargo.toml:115-120` hält aws-lc-rs aus dem Abhängigkeitsbaum. Die Paketverschlüsselung kommt also aus `ring` (L8, L9).
- Paketgröße: `initial_mtu` Vorgabe und Minimum 1200 Byte; MTU-Ermittlung standardmäßig an, Obergrenze 1452 Byte (`vendor/noq-proto-1.3.0/src/config/transport.rs:583-585,872,904`, L6).
- Senden gebündelt: `MAX_TRANSMIT_SEGMENTS = 10` und `MAX_TRANSMIT_DATAGRAMS = 20` je Sendeaufruf (`vendor/noq-1.3.0/src/connection.rs:2123,2130`, L7). Bei 1280 Byte Paketgröße sind das 12,8 KB je `sendmsg` mit `UDP_SEGMENT`; die Linux-Grenze liegt bei 64 Segmenten (Q36).
- Segmentierung: `enable_segmentation_offload` Vorgabe `true` (`transport.rs:601`, L6); noq liest auf Empfängerseite `gro_segments()` aus dem UDP-Socket (`vendor/noq-1.3.0/src/runtime/tokio.rs:104`, L7).
- Staukontrolle: Cubic als Vorgabe (`transport.rs:581`, L6); `NewReno` und `Bbr3` vorhanden (`congestion/`); iroh 1.3.0 überschreibt die Vorgabe nicht (`vendor/iroh-1.3.0/src/endpoint/quic.rs` enthält nur den Durchreicher `congestion_controller_factory`, Zeile 434-438, L8). Die Schnittstellen `Controller`, `ControllerFactory`, `ControllerMetrics` sind aus iroh erreichbar (L4 Abschnitt A.2); damit kann je Verbindung ein eigener Regler eingesetzt werden. `Bbr3Config` ist laut L3 nicht aus iroh exportiert.
- Fenster: Vorgabe `stream_receive_window` 1 250 000 Byte, `send_window` 10 000 000 Byte, `receive_window` unbegrenzt; ausgelegt für 100 Mbit/s bei 100 ms RTT (L4 Abschnitt A.4, gelesen am 2026-09-28 an Version 1.0.1; für 1.3.0 gilt `transport.rs:563-574`, L6, gleiche Konstanten).
- Datagramme: `datagram_receive_buffer_size` Vorgabe 1 250 000 Byte, `datagram_send_buffer_size` 1 048 576 Byte; bei vollem Puffer werden die ältesten Datagramme verworfen (`transport.rs:315-333,594-595`, L6).
- Mehrere Pfade je Verbindung: iroh setzt `max_concurrent_multipath_paths` auf 8 (L4 Abschnitt A.5, geprüft an iroh 1.0.1; an 1.3.0 nicht erneut geprüft). noq-proto sendet Nutzdaten auf jedem validierten Pfad mit Status `Available`; Pfade mit Status `Backup` nur, wenn kein validierter `Available`-Pfad besteht (`vendor/noq-proto-1.3.0/src/connection/mod.rs:1171-1220`, L6). Ob iroh 1.3.0 mehrere Pfade gleichzeitig als `Available` führt, ist nicht geprüft [U].
- Je Gegenstelle genau eine Verbindung für alle Zwecke; Bildschirmstrom auf einem zuverlässigen bidirektionalen Strom (L1 Abschnitt B, L2 Abschnitt 3.1). Der Ein-Kern-Treiber je Verbindung und die Schätzung „2–5 Gbit/s Desktop“ stammen aus `analyse-g2-geraete.md` (nur über L1/L2 bekannt, dort als Schätzung gekennzeichnet).
- Messwerte zu se selbst liegen nicht vor (L1 Abschnitt B, L2 Abschnitt 4).

## 2 VPN im Programm ohne Systemadapter

Leitfrage: Kann ein VPN nur zwischen zwei se-Programmen laufen, ohne Adminrechte, ohne Systemadapter, ohne Nutzerdialog?

**Befund in Kürze.** [B] Ja, technisch ist es für WireGuard-Protokoll plus eingebetteten TCP/IP-Stack belegt (Tailscale `tsnet` mit gVisor-netstack, onetun mit smoltcp und boringtun). [F] Der Preis: Das Programm bringt seinen eigenen TCP/IP-Stack mit; Programme des Betriebssystems (zum Beispiel ein RDP-Client) erreichen diesen Stack nur über einen lokalen Portweiterleiter, also genau über die heutigen TCP-Freigaben. [U] Für den Durchsatz des gVisor-netstack und von smoltcp-Aufbauten auf Windows und Android liegt keine veröffentlichte Messung vor.

### 2.1 boringtun (Cloudflare) und GotaTun (Mullvad)

- Technik: WireGuard-Protokoll als Rust-Bibliothek „without the network or tunnel stacks“; C- und JNI-Anbindung vorhanden (Q07, Abschnitt „Introduction“/Plattformtabelle). Crate `boringtun` 0.7.1 mit Modulen `noise` und `x25519`, beschrieben als „Simple implementation of the client-side of the WireGuard protocol“ (Q08).
- Plattformen laut Q07: Binärprogramm und Bibliothek für x86_64- und aarch64-Linux, armv7-Linux, x86_64-macOS; **nur Bibliothek** für Windows (x86_64 MSVC), iOS, Android (aarch64, arm).
- Randbedingungen: Das Binärprogramm `boringtun-cli` verlangt `sudo` bzw. `CAP_NET_ADMIN` (Q07), weil es ein TUN-Gerät anlegt. Die Bibliothek selbst braucht laut Q07 keinen Adapter (sie liefert nur das Protokoll); [F] ein Aufbau ohne Systemadapter entsteht erst durch Kopplung mit einem Userspace-Stack (Abschnitt 2.3, 2.4).
- Garantien: Noise-basierter Handshake mit einer Rundreise, ChaCha20-Poly1305, Replay-Schutz über Zähler mit Gleitfenster (Q04 §5.4.6 und §6, Seite 13).
- Messwerte: Q09 (Cloudflare, 2019-03-27) und Q07 nennen **keine Zahlen**; Q09 sagt qualitativ, wireguard-go bleibe hinter dem Kernelmodul zurück. [U] Durchsatz von boringtun als Bibliothek.
- Lizenz: BSD-3-Clause (Q07, Q09). Q07 trägt den Hinweis „currently undergoing a restructuring“ und rät von Verknüpfung auf `master` ab.
- GotaTun (Mullvad): Fork von boringtun, Ankündigung 2025-12-19 (Q10); Android ab Version 2025.10, Desktop und iOS für 2026 angekündigt; Sicherheitsprüfung durch Dritte für Anfang 2026 angekündigt, Ergebnis nicht gefunden [U]. Q10 nennt Absturzrate 0,40 % auf 0,01 % **[M]**, aber keinen Durchsatz; Repository `github.com/mullvad/gotatun`; Lizenz in Q10 nicht genannt, ein Suchtreffer nennt BSD-3-Clause (nicht geprüft) [U]. Ob GotaTun als eingebettete Bibliothek ohne TUN nutzbar ist, nennt Q10 nicht [U].
- Suchtreffer ohne Prüfung: NepTUN (NordSecurity, Fork von boringtun) und ein Firezone-Beitrag zu einer Rust-WireGuard-Schicht mit Herstellerangabe „über 2 Gbit/s auf iOS und Android, über 5 Gbit/s auf Windows und macOS“; beide Quellen wurden nicht geöffnet [U].

### 2.2 wireguard-go und Tailscale (tsnet, netstack, gVisor)

- Technik wireguard-go mit Verbesserungen von Tailscale (Q02, Q01, Q03):
  - TUN-Offloads TSO/GRO über `set_offload()` (seit Linux 2.6.27), `sendmmsg`/`recvmmsg` auf dem UDP-Socket (Q02).
  - UDP-GSO (Linux 4.18) und UDP-GRO (Linux 5.0) auf dem Socket, Hardware-UDP-Segmentierung `tx-udp-segmentation`, schnellere Prüfsumme (Q01).
  - TUN-Treiber mit UDP-GSO/GRO ab Linux 6.2 (Q03).
- Messwerte **[M]**; Q01 und Q02: einzelner TCP-Strom mit iperf3 und CUBIC, MSS 1368 Byte, außer wo eine andere MSS genannt ist; Q03: UDP-Last:
  - Q02 (2022-12-13, zwei AWS c6i.8xlarge, Linux 5.15): ohne WireGuard 9,53 Gbit/s; Kernel-WireGuard 2,66; wireguard-go Ausgangsstand 2,42; wireguard-go mit TSO, GRO, mmsg 5,36; wireguard-go mit größerer MTU (8869 Byte MSS) 7,88.
  - Q01 (2023-04-13): i5-12400 mit Mellanox 25 GbE, Linux 5.15: Kernel-WireGuard 11,8 Gbit/s; wireguard-go Ausgangsstand 8,36; wireguard-go mit den Änderungen 13,0. E3-1230-V2 (Baujahr 2012): 3,36 Gbit/s Ausgangsstand, 4,86 ohne und 6,59 mit Hardware-UDP-Segmentierung. c6i.8xlarge: 7,32 Gbit/s; Q01 nennt eine künstliche Netzgrenze der AWS-Instanzen bei etwa 7,3 Gbit/s.
  - Q03 (2023-11-16, UDP-Last mit `secnetperf` von MsQuic, Verschlüsselung ausgeschaltet): i5-12400 Kernel-WireGuard 5,42; wireguard-go Ausgangsstand 2,94; wireguard-go mit UDP-GSO/GRO 12,46 Gbit/s (etwa Faktor 4); c6i.8xlarge 2,20 auf 7,21 Gbit/s.
  - Zitat Q01: „more time is being spent sending UDP packets than encrypting their payloads.“
- Randbedingungen: Diese Werte gelten für Linux mit TUN-Gerät (Adapter, root). Unter Windows und Android nennt keine der Tailscale-Quellen Messwerte [U].
- `tsnet` (Q14): bettet einen Tailscale-Knoten in ein Go-Programm; läuft „inside your process using a userspace TCP/IP stack (gVisor)“; ohne Root und ohne Systemdienst; `Listen` und `Dial` liefern `net.Listener` und `net.Conn`; Lizenz BSD-3-Clause; Q14 enthält keine Leistungsangaben.
- Tailscale-Dokumentation (Q13, geprüft 2026-01-05 laut Seite): Der Userspace-Modus mit netstack gilt auf allen Systemen außer Linux im Kernelmodus; „not as optimized as the Linux kernel“; „kernel mode … for heavily used subnet routers“; keine Zahlen.
- gVisor-netstack: Es wurde keine veröffentlichte Durchsatzmessung gefunden; gVisor bringt ein Benchmark-Werkzeug `test/benchmarks/tcp` mit (Suchtreffer, Q-Nummer nicht vergeben, nicht geöffnet) [U].
- Nutzbarkeit aus Rust: wireguard-go und gVisor sind Go; Einbindung in ein Rust-Programm verlangt eine Go-Laufzeit und eine FFI-Grenze; Q10 nennt die FFI-Grenze Rust–Go als Ursache von über 85 % der Android-Absturzberichte bei Mullvad **[M]**.

### 2.3 onetun und smoltcp-basierte Aufbauten

- onetun (Q11): „cross-platform, user-space WireGuard port-forwarder that requires no root-access or system network configurations“; smoltcp erzeugt die IP-Pakete, boringtun verschlüsselt sie. Plattformen: Windows, macOS (Apple Silicon), Linux (amd64, arm64). Lizenz MIT. Einschränkungen laut Q11: UDP „totally experimental“; smoltcp begrenzt die Zahl der Adressen je Schnittstelle zur Übersetzungszeit, standardmäßig sind es 7 Peers je Protokoll (änderbar durch Neuübersetzung); ein Tunnel unterstützt je eine Quell- und eine Ziel-IP-Version; Autor „do[es] not have enough personal time to actively maintain onetun“. Q11 enthält weder Durchsatzangaben noch eine Bibliotheks-Schnittstelle.
- smoltcp (Q12): Lizenz 0BSD; keine Heap-Allokation erforderlich; TCP mit Fenster-Skalierung, Nagle, Keep-Alive, verzögerten ACKs; Staukontrolle „optional, CUBIC and Reno are implemented“; **„Selective acknowledgements are not implemented“**; keine Zeitstempel; keine PLPMTU-Ermittlung; Pufferobergrenzen sind Übersetzungszeit-Einstellungen. Geräte: TUN/TAP, Raw-Socket, Loopback; die Beispiele nennen „Only Linux is supported (right now)“.
- Messwerte smoltcp **[M]** (Q12, eigene Angabe, Intel Core i5-13500H, Linux 6.9.9): TAP-Leser 3,673 Gbit/s, TAP-Schreiber 7,905 Gbit/s, Loopback-Benchmark etwa 15,4 Gbit/s.
- [F] Ein Aufbau „WireGuard-Bibliothek plus smoltcp“ ersetzt für die Daten die Kernel-TCP-Implementierung durch einen Userspace-TCP-Stack ohne SACK; auf Strecken mit Verlust ist das nach der Literatur zu TCP ohne SACK ein Nachteil, eine Messung dazu fehlt [U]. Dieser Befund gilt für Übertragungen über den Userspace-Stack; QUIC bringt Verlustwiederherstellung mit Bestätigungsbereichen bereits mit (RFC 9000/9002, Q29, Q30).

### 2.4 Nebula und ZeroTier (libzt)

- Nebula (Q15): Lizenz MIT; läuft auf Linux, OSX, Windows, iOS, Android; ECDH (Curve25519) und AES-256-GCM in der Vorgabe, Handshake nach Noise; ChaCha20-Poly1305 nennt Q15 nicht. Ob ein TUN-Gerät nötig ist und ob eine Bibliothek ohne Adapter nutzbar ist, steht in Q15 nicht [U]; das Repository enthält ein Verzeichnis `wintun`, was auf den Wintun-Adapter unter Windows hindeutet [F].
- Messwerte Nebula **[M]** (Q16, Herstellerblog von Defined Networking, 2024-02-16; fünf Dell-Rechner mit i7-10700, 10 GbE, `iperf3 -M 1200`, 10 Minuten je Test, Nebula im AES-Modus): Nebula Senden etwa 9 Gbit/s, Empfangen etwa 7,8, beidseitig etwa 9,6; Wirkungsgrad etwa 1500 Mbit/s je CPU-Kern-Prozent im Senden; Speicher etwa 27 MB. Dieselbe Quelle: Tailscale 9 / 8,8 / 9,6 Gbit/s; Netmaker (Kernel-WireGuard) 9 / 8,8 / 13; **ZeroTier etwa 3 / 1,8–2,2 / 3 Gbit/s**, einfädig. Q16 nennt Nebula in AES-Modus auf einem Raspberry Pi 5 mit einem Kern bei 1 Gbit/s ausgelastet. Die Quelle ist Hersteller des Konkurrenzprodukts Nebula; die Messung ist nicht unabhängig.
- ZeroTier libzt (Q17): „P2P cross-platform encrypted sockets library using ZeroTier“; Lizenz **BSL 1.1** (kostenfrei für internen Geschäftsgebrauch, Hochschulen, nichtkommerzielle Zwecke; für geschlossene kommerzielle Anwendungen und Geräte ist eine kommerzielle Lizenz nötig); Ziele C/C++ (Linux, macOS, Windows), C#, Python, Rust (Crate `libzt`), Java, iOS/iPadOS, Android. Ob libzt ohne Systemadapter arbeitet und welchen Netzwerk-Stack es nutzt, nennt Q17 nicht [U]. Durchsatz von libzt: nicht gefunden [U].
- [F] Nebula und ZeroTier sind Overlay-Netze mit eigener Schlüssel- und Adressverwaltung; se hat bereits eine Geräteidentität und Koppelung (iroh-Schlüssel). Beide Referenzen würden eine zweite Identitäts- und Zertifikatsebene einführen. Das ist eine Folgerung aus der Architektur, nicht aus einer Quelle.

## 3 VPN mit Systemadapter

**Befund in Kürze.** [B] Kernel-WireGuard, wireguard-nt und Wintun brauchen Treiber und Administratorrechte; Android erlaubt genau einen aktiven VPN-Dienst je Nutzer bzw. Arbeitsprofil und verlangt einen Systemdialog. [F] Das widerspricht dem Grundsatz „keine Handgriffe, keine Adminrechte“ und würde bei Android das VPN des Nutzers verdrängen.

- Kernel-WireGuard unter Linux (Q04, Q05): GSO/Scatter-Gather/Prüfsummen-Offload des Kernels, Warteschlange mit „super packets“, Verschlüsselung parallel über `padata`, ChaCha20-Poly1305 mit Assembler für Intel-Vektorerweiterungen und ARM/NEON (Q04 §7.1, §7.2, §7.4, Seiten 16–17). Einrichtung über Netlink (Q04 §7.3, Seite 16); [A] das verlangt `CAP_NET_ADMIN`.
- Messwerte Kernel-WireGuard **[M]**:
  - Q04 §8 (Seite 18; Intel Core i7-3820QM und i7-5200U, Gigabit-Ethernet, iperf3 über 30 Minuten gemittelt): WireGuard 1011 Mbit/s, IPsec mit ChaCha20-Poly1305 825, IPsec mit AES-128-GCM 881, OpenVPN 258; Ping 0,403 ms gegen 0,501 / 0,508 / 1,541 ms. Q05 nennt diese Messung selbst „old, crusty, and not super well conducted“ und veröffentlicht keine Zahlen.
  - Q01/Q02/Q03 (siehe 2.2): 2,66–11,8 Gbit/s je Hardware.
- wireguard-nt (Q19, Q20): WireGuard-Implementierung im NT-Kern; `wireguard.dll`; Windows 10 und 11 auf AMD64, x86, ARM64; Quelltext GPLv2, vorgebaute Binärdateien unter einer freizügigeren Lizenz; Entwicklungsaufbau verlangt Testsignierung; Q19 nennt keine Durchsatzzahlen.
  - Messwerte aus der Ankündigung vom 2021-08-02 (Q20) **[M]**: etwa 7,5 Gbit/s Senden auf dem Testgerät des Autors; mit Intel AC9560 WLAN: ohne WireGuard etwa 600 Mbit/s, wireguard-go/Wintun über Ethernet etwa 600 Mbit/s, wireguard-go/Wintun über WLAN etwa 95 Mbit/s, wireguard-nt über WLAN etwa 600 Mbit/s; Q20 und Suchtreffer sprechen von 10 bis 25 % mehr Durchsatz gegenüber wireguard-go mit Wintun (Hersteller-Messung, Hardware nicht vollständig angegeben).
  - Rechte: Der Windows-Client läuft mit einem Tunneldienst als Local System; die Oberfläche startet nur für Administratoren (Q21, `docs/attacksurface.md`); der Tunneldienst behält `SeLoadDriverPrivilege`. Lizenz des Windows-Clients MIT (Q21).
- Wintun (Q18): „very simple and minimal TUN driver for the Windows kernel“, ursprünglich für WireGuard; Quelltext GPL 2.0, vorgebaute signierte DLLs unter freizügigerer Lizenz; Q18 nennt weder Administratorpflicht noch Messwerte. [A] Treiberinstallation verlangt Administrator (nicht in Q18 belegt; Q21 belegt den Administrator-Pfad des WireGuard-Clients).
- Android `VpnService` (Q22, Q23):
  - „User action is required the first time an application creates a VPN connection“; Systemdialog über `prepare()`; Benachrichtigung und Schlüsselsymbol sind Pflicht (Q23).
  - „There can be only one VPN connection running at the same time. The existing interface is deactivated when a new one is created.“ (Q22). „There's only one active service per user or profile. Starting a new service automatically stops an active service.“ (Q23)
  - Der Dienst liest und schreibt IP-Pakete über den Datei-Deskriptor der TUN-Schnittstelle (Q23); Tunnel-Sockets müssen mit `protect()` aus dem VPN genommen werden.
  - Je-App-Listen (`addAllowedApplication`) beschränken, welche Apps das VPN nutzen; sie heben die Grenze „ein Dienst“ nicht auf (Q23).
  - [F] Ein se-eigenes VPN auf Android verdrängt das VPN des Nutzers oder wird von ihm verdrängt und braucht einen Dialog; es erfüllt weder „ohne Nutzerdialog“ noch „ohne Handgriffe“.
- Lizenzfolge für se: GPL-Anteile (Wintun-Quelltext, wireguard-nt-Quelltext, Kernelmodul) wären nur als gebündelte Binärdateien unter deren freizügigerer Binärlizenz nutzbar; der genaue Wortlaut der Binärlizenz wurde nicht gelesen [U].

## 4 Kosten der Verschlüsselung: VPN gegenüber bestehender QUIC/TLS-1.3-Verbindung

**Befund in Kürze.**

1. [F] Ein VPN spart keine Verschlüsselung ein, solange QUIC darunter oder darüber läuft: Läuft QUIC durch einen WireGuard-Tunnel, entsteht eine zweite AEAD-Schicht (WireGuard fügt je Paket 16 Byte Kopf und 16 Byte Poly1305-Tag hinzu, Q04 Seite 13 und Seite 15). Ersetzt WireGuard dagegen QUIC, entfällt die QUIC-Verschlüsselung, aber QUIC-Verlustwiederherstellung, Stromsteuerung und Staukontrolle müssen durch einen Userspace-TCP-Stack ersetzt werden (Abschnitt 2.3).
2. [B] Veröffentlichte Einzelkern-Durchsätze der AEAD-Verfahren liegen über den veröffentlichten Gesamtdurchsätzen der QUIC-Implementierungen; die Paketverarbeitung (UDP, QUIC-Köpfe, ACK) ist die größere Last (Abschnitt 4.2).

### 4.1 Einzelkern-Durchsatz AES-GCM gegen ChaCha20-Poly1305

Alle Werte **[M]**.

- Q26 (2025-11-07, Rust-Crate `ring`, ein Kern, Eingaben von etwa 100 Byte bis etwa 1 KB; der Abruf nennt als Einheit MB/s und ordnet die Obergrenze des Bereichs den größeren Eingaben zu [A]):

  | Architektur | CPU | ChaCha20 Verschlüsseln MB/s | AES-256-GCM Verschlüsseln MB/s |
  |---|---|---|---|
  | x86_64 | Intel Ice Lake | 396–1158 | 577–2617 |
  | x86_64 | Intel Sapphire Rapids | 393–1195 | 614–2891 |
  | x86_64 | AMD Zen 3 | 436–1425 | 524–2849 |
  | x86_64 | AMD Zen 4 | 456–1410 | 652–3040 |
  | x86_64 | AMD Zen 5 | 467–1580 | 862–3731 |
  | ARM64 | AWS Graviton 2 | 168–503 | 292–1065 |
  | ARM64 | AWS Graviton 3 | 249–778 | 470–1926 |
  | ARM64 | AWS Graviton 4 | 280–895 | 563–2436 |
  | ARM64 | Apple M2 Pro | 327–1088 | 661–3131 |

  [F] Obergrenze AES-256-GCM entspricht 8,5 Gbit/s (Graviton 2) bis 29,8 Gbit/s (Zen 5) je Kern; Obergrenze ChaCha20 entspricht 4,0 Gbit/s (Graviton 2) bis 12,6 Gbit/s (Zen 5). Umrechnung: MB/s mal 8 geteilt durch 1000.
- Q27 (Graviola-Benchmark, Criterion, ein Thread, `aws-lc-rs`, 16-KB-Blöcke; Geräte: Framework 16 mit AMD Ryzen 9 7940HS unter Linux, MacBook Pro M2 von 2022; Messdatum auf der Seite nicht angegeben): AES-256-GCM x86_64 etwa 10,8 GiB/s, aarch64 etwa 6,8 GiB/s; ChaCha20-Poly1305 x86_64 etwa 3,16 GiB/s, aarch64 etwa 1,56 GiB/s. Die Zahlen stammen aus Suchtreffer-Auszügen der Berichtsseiten; die Berichtsseiten selbst wurden nicht geöffnet (Status in Abschnitt 10).
- Q24 (Cloudflare, 2016-04-04, Haswell, AVX2): AES-GCM mit den Befehlen AESENC und CLMULQDQ ist bei Datensätzen über 320 Byte schneller als ChaCha20-Poly1305; ChaCha20-Poly1305 erreicht unter 2 Zyklen je Byte; Prognose mit AVX-512 unter 1 Zyklus je Byte (Prognose, keine Messung).
- Q25 (Cloudflare, 2015-02-23): Galaxy Nexus (OMAP 4460), Entschlüsseln von 1 MB: AES-128-GCM 41,6 ms, ChaCha20-Poly1305 13,2 ms **[M]**; Aussage „three times faster than AES-128-GCM on mobile devices“ ohne Hardware-AES. Für Telefone mit ARMv8-Kryptobefehlen nennt Q25 keine Messwerte; moderne Telefon-SoCs fehlen in allen gelesenen Quellen [U].
- Q04 §8: „As future chips increase the width of vector instructions … ChaCha20Poly1305 will outperform AES-NI“ (Prognose von 2016; Q26 zeigt für 2025 das Gegenteil: AES-256-GCM liegt auf allen gemessenen CPUs vor ChaCha20). Q06 (Known Limitations): ChaCha20-Poly1305 sei „extremely fast in software on virtually all general purpose CPUs“; Hardwareunterstützung sei begrenzt.
- RFC 9001 §6.6 (Q28): Grenze der Vertraulichkeit für AES-128-GCM und AES-256-GCM 2^23 verschlüsselte Pakete je Schlüssel, danach ist ein Schlüsselwechsel Pflicht; für ChaCha20-Poly1305 ist die Grenze „greater than the number of possible packets“. Der Abruf gab die Exponenten ohne Hochstellung wieder; der Wert 2^23 ist aus dem Wortlaut „223“ erschlossen [A]. [F] Bei 1452 Byte je Paket sind 2^23 Pakete rund 12,2 GB; bei 5 Gbit/s folgt ein Schlüsselwechsel etwa alle 19,5 Sekunden. Ob noq-proto 1.3.0 den Wechsel unter dieser Last ohne Durchsatzeinbruch ausführt, ist nicht geprüft [U].
- Welche Suite se im QUIC-Handshake wählt: `noq-proto/src/crypto/rustls.rs` nennt `TLS13_AES_128_GCM_SHA256` für die Initial-Schlüssel (Zeilen 314, 344); die für 1-RTT-Pakete ausgehandelte Suite ist am Quelltext nicht geklärt [U]. [A] Mit `ring` auf x86_64 und ARM64 mit Kryptobefehlen ist es AES-128-GCM.

### 4.2 Anteil der Verschlüsselung am QUIC-Durchsatz

- Q33 (König et al., IFIP Networking 2025; zusammenfassend ihre Arbeit LCN 2023): Bei ausgeschalteter QUIC-Kryptografie in zwei Implementierungen steigt der Durchsatz, aber „cryptographic overhead is not the sole factor driving performance differences“; Durchsatzgrenze sei die Leistung eines einzelnen Kerns auf der Senderseite (Abschnitt VI-F: ein Sender-Kern bei 100 %, Empfänger mit 37 % Leerlauf).
- Q37 (ETH Zürich, Poster, 2025): Das Messen der Kryptolaufzeit mit Uprobes senkt den Durchsatz selbst um 37 % (Messaufwand, kein Kostenwert der Verschlüsselung).
- Q36: noq-Messreihe auf Hetzner (Abschnitt 5.3). Die PR-Beschreibung enthält keine Messung mit ausgeschalteter Verschlüsselung.
- Q32 (Zhang et al., WWW 2024): Ursache der Durchsatzlücke von QUIC gegenüber TCP+TLS+HTTP/2 sei „high receiver-side processing overhead, specifically excessive data packets and QUIC's user-space ACKs“; die Verschlüsselung ist nicht als Ursache genannt.
- [F] Die Annahme aus `plan.md` („Verschlüsselung kostet Zeit und ist ineffizient für die maximale Datenmenge“) ist durch die gelesenen Quellen nicht gestützt: Die Einzelkern-Obergrenzen in 4.1 (mindestens 4 Gbit/s auch für ChaCha20 auf Graviton 2) liegen über den gemessenen QUIC-Durchsätzen (2,4 bis 8,22 Gbit/s in Q33, 3,3 bis 5,9 Gbit/s bei noq in Q36), und die Quellen führen die Grenze auf Paketverarbeitung und ACKs zurück. Eine Messung auf den Zielgeräten (Telefon, Windows) fehlt [U]; `machbarkeit-unverschluesselt.md` (L2 Abschnitt 6) sieht sie vor.
- Sunshine verschlüsselt im lokalen Netz in der Vorgabe nicht: `lan_encryption_mode` Vorgabe 0 („encryption is not used“), `wan_encryption_mode` Vorgabe 1 („used if the client supports it“) (Q47). Q47 nennt keine Begründung.

## 5 Durchsatztechniken von QUIC-Implementierungen

### 5.1 Verfahren und ihre belegte Wirkung

- **UDP-GSO und -GRO.** UDP-GSO (`UDP_SEGMENT`) seit Linux 4.18, bis zu 64 Segmente je Aufruf, alle gleich groß bis auf das letzte kleinere (Q34). Q34 (Cloudflare, 2020-01-08, quiche, Linux 5.3): Systemaufrufe `sendmsg` 904 539 gegenüber 18 824 mit GSO; Durchsatz des Ausgangsstandes 80–90 MB/s; GSO bringe „significant improvement“, Gbit/s-Zahlen nur in Diagrammen. Q33: GSO/GRO hebt quiche von 3,5 auf 4,16 Gbit/s und ngtcp2 von 3,7 auf 4,94 Gbit/s; lsquic ohne GSO-Unterstützung unverändert. Q58 (Kernel-Dokumentation): Gerätemerkmal `NETIF_F_GSO_UDP_L4`; die Abschnitte zu `UDP_SEGMENT` und `UDP_GRO` standen im Abruf nicht.
- **sendmmsg/recvmmsg.** Q34: senkt Systemaufrufe von 904 539 auf 2 437 `sendmsg` plus 15 676 `sendmmsg`, der Durchsatz steigt weniger stark als die Aufrufzahl sinkt. Q01: „sendmmsg() … though the kernel still traverses the stack per packet until GSO/GRO are used“.
- **Paketgröße und MTU.** Q33: MTU 9000 statt 1500: ngtcp2 von 4,94 auf 9,97 Gbit/s (Sättigung von 10 Gbit/s), lsquic von 3,6 auf 9,19, quiche von 4,16 auf 5,84 Gbit/s; „QUIC libraries still create and process headers for each 1500-byte packet“, auch mit GSO. Q02: Erhöhung der Tunnel-MTU allein bringt „more than a 3x improvement“ (2,42 auf 7,88 Gbit/s). [F] Auf dem Internet und im WLAN ist 9000 Byte nicht verfügbar; in se liegt die Obergrenze bei 1452 Byte (Abschnitt 1).
- **Segmentanzahl je Sendeaufruf in noq.** Q37 (quinn): quinn schrieb höchstens 16 KB je Systemaufruf gegenüber 64 KB bei MsQuic, hatte etwa viermal so viele Aufrufe (5277 gegenüber 1377); mit 64 KB je Aufruf stieg der Durchsatz um 14 % und die CPU-Last sank um 15 %. Q36 (noq, 2026-05): Heraufsetzen von `MAX_TRANSMIT_SEGMENTS` von 10 auf 40 und `MAX_TRANSMIT_DATAGRAMS` von 20 auf 80 (Abschnitt 5.3). Die PR wurde am 2026-06-03 ohne Zusammenführung geschlossen zugunsten von PR #645, der die Werte als `TransportConfig`-Setter freigibt (Vorgaben bleiben 10 und 20). Ob #645 in noq 1.3.0 enthalten ist: nein, im vendorierten Quelltext sind die Werte Konstanten (Abschnitt 1) [B]; ob eine spätere Version sie freigibt, ist nicht geprüft [U].
- **Mehrere Verbindungen gegen eine.** Q33: Der Sender ist auf einen Kern begrenzt; die Implementierungen nutzen keine Mehrkernparallelität je Verbindung. Mehrere QUIC-**Ströme** (Q33 Abschnitt V-B, HTTP/3, 8 GB in 100 Teilen zu je 80 MB): Aufteilung der festen Datenmenge in Teile bringt bis zu 11 % mehr Durchsatz; bei quiche fällt der Durchsatz mit nur einem gleichzeitigen Strom von 3,097 Gbit/s auf unter 1,5 Gbit/s und erreicht mit 100 gleichzeitigen Strömen wieder den Wert der Einzeldatei; quic-go gewinnt leicht mit 2 bis 8 Strömen, lsquic bleibt unverändert; Take-away des Abschnitts laut Text: Erhöhen der Zahl gleichzeitiger Ströme habe den Durchsatz „nearly“ verdoppelt (Satz im Text abgeschnitten, Bezug auf quiche aus den davor genannten Zahlen erschlossen [A]). quic-go nutzt mehrere Kerne (bis etwa 400 % CPU ab 32 Strömen), erreicht aber weniger Durchsatz als quiche und lsquic mit höchstens 100 % CPU. Mehrere **Verbindungen** gegen eine wurden in Q33 nicht gemessen [U]. Globus/GridFTP (übernommen aus L3): Parallelität (mehrere TCP-Ströme je Datei) für große Dateien bei hoher Bandbreite und Latenz; ESnet fasterdata: „most data movement applications, such as Globus, would employ 4-8 streams“ (L3, abgerufen 2026-10-21).
- **Staukontrolle.** RFC 9002 (Q29): Vorgabe NewReno-ähnlich, ein Sender dürfe einseitig einen anderen Algorithmus wählen, genannt ist CUBIC; Anfangsfenster 10 mal Datagrammgröße; Pacing „SHOULD“ (§7.7). Q33 verwendet für alle Implementierungen CUBIC. BBR (übernommen aus L3): `draft-ietf-ccwg-bbr-06` vom 2026-07-06, Status Experimental; im Vergleich mit CUBIC „deutlich höherer Durchsatz bei flachen Puffern oder zufälligen Verlusten, deutlich weniger Warteschlangenverzögerung bei tiefen Puffern“; die Messzahlen der BBR-Veröffentlichung (ACM Queue 2016) waren nicht abrufbar (HTTP 403) [U]. Drucker et al. (Q43): „WebRTC BBR“ unterschreite in tiefen Puffern wegen überschätzter Bandbreite („bandwidth overestimation“, auch bei TCP-BBR bekannt), schlage aber GCC im Wettbewerb mit einem TCP-Strom.
- **Multipath.** Q41 (Datentracker, Seite zuletzt 2026-11-07): `draft-ietf-quic-multipath` ist als „RFC 10069, Managing Multiple Paths for a QUIC Connection“ veröffentlicht (Proposed Standard, November 2026, laut Seite); getrennter Staukontrollzustand je Pfad; „The scheduling is a local decision based on the preferences of the application and the implementation“; keine Scheduler-Spezifikation. Q40 (n0, 2026-03-19): iroh macht Relay, direktes IPv4 und IPv6 zu erstklassigen QUIC-Pfaden mit je eigenem Staukontrollzustand; noq „aims for full QUIC Multipath spec coverage, but it is still young“; Q40 nennt keine Messwerte.

### 5.2 Vergleich mit TCP und TLS im Kernel

- Q32 (WWW 2024): Über schnelle Netze verliert UDP+QUIC+HTTP/3 bis zu **45,2 %** Datenrate gegenüber TCP+TLS+HTTP/2; die Lücke wächst mit der Bandbreite; Videobitrate bis zu 9,8 % niedriger; Ursache: Empfängerverarbeitung.
- Q33 (IFIP 2025, Intel Xeon W-2145, Intel X550-T2 10 GbE, Linux 5.15.0-56): Einzeldatei über HTTP/3, 8 GB: ngtcp2-ngtcp2 4172 Mbit/s am besten, quic-go–quiche 1220 Mbit/s am schlechtesten; mit QUIC-Lastgeneratoren statt HTTP/3 steigt picoquic von 1,52 auf 3,88 Gbit/s und ngtcp2 von 4,17 auf 4,94 Gbit/s; der frühere Befund derselben Gruppe: TCP und reines UDP füllen 10 Gbit/s, QUIC-Implementierungen erreichen 2,4 bis 8,22 Gbit/s; HTTP/3 statt HTTP/0.9 kostet bis zu 27 %; Hochrechnung „QUIC … requiring twice as much processing power than TCP+TLS“ steht in Q37 als Zitat früherer Arbeiten.
- Q35 (Microsoft, 2021-04-14): MsQuic, Einzelverbindung, Upload von 1,67 Gbit/s (Juli 2020) auf bis zu 7,99 Gbit/s, nur mit Windows-Vorabversionen, im Benutzermodus mit Schannel, auf Serverhardware, mit UDP-Segmentierung (USO) und Empfangs-Zusammenfassung.
- [F] Bei Leitungen bis 1 Gbit/s liegt jeder in Q33 und Q36 gemessene Einzelverbindungswert (kleinster Wert 1220 Mbit/s in Q33; noq 3,3 bis 5,9 Gbit/s in Q36) über der Leitungsrate; die Differenz zwischen QUIC und Kernel-TCP wirkt dort nach diesen Quellen nicht auf die Übertragungsdauer. Bei 2,5 Gbit/s und mehr unterschreiten die langsameren Paare in Q33 die Leitungsrate. Ob das für Telefone und Windows-Geräte gilt, ist ungeklärt [U].

### 5.3 Messungen der genannten Implementierungen

- **quinn** (Q37, ETH Zürich, Poster 2025, eine Verbindung, Download von 25 MB über unbeschränkte Strecke, 50 Läufe): quinn im Mittel 206 Mbit/s, MsQuic 268 Mbit/s (30 % mehr); CPU-Auslastung quinn 81 %, MsQuic 46 %. Der kurze Transfer von 25 MB ist durch die Anlaufphase geprägt [F]. Q38 (TCP-über-QUIC, Quinn, zwei Debian-VMs mit je 4 vCPU, Linux 6.1): „MsQuic consistently outperformed Quinn under loss or reordering“, zitiert aus der Arbeit von Marti.
- **noq** (Q36, PR #636 vom 2026-05-05, Hetzner CCX23 mit 4 dedizierten vCPU, MTU 1280, `fq`, BBR im Betriebssystem; PR-Autor poka-IT, nicht das noq-Team; die Messungen messen die PR-Änderung, nicht den Ausgangsstand allein):
  - Loopback, ein bidirektionaler Strom (CPX32): vorher 4609 Mbit/s bei 99,0 % Client-CPU, nachher 5892 Mbit/s bei 98,4 %.
  - Im Rechenzentrum (fsn1, 0,5 ms RTT, 16 Wiederholungen): Hochladen 3325 auf 3882 Mbit/s, Herunterladen 3473 auf 3943 Mbit/s.
  - fsn1 nach hel1 (26,3 ms RTT): 366 auf 359 Mbit/s hoch, 364 auf 366 Mbit/s herunter (BDP-begrenzt; keine Änderung).
  - Betreuer-Messung Vereinigtes Königreich nach Wien mit dem `perf`-Programm: 128 auf 139 Mbit/s hoch, 130 auf 115 Mbit/s herunter; „quick checks, not rigorous benchmarking“.
- **MsQuic** (Q35 und Q33-Umfeld): siehe 5.2. Das Projekt führt ein Leistungs-Dashboard (`microsoft.github.io/msquic/detailed.html`, Suchtreffer, Zahlen nicht gelesen) [U].
- **quiche** (Q33, Cloudflare-Implementierung, Rust): 3,5 Gbit/s ohne und 4,16 Gbit/s mit GSO/GRO; bei 5 ms Zusatzverzögerung Absturz von 4,16 auf 0,15 Gbit/s (Einzelbefund dieser Arbeit; laut Q33 Abschnitt VI-E); Verlust bis 2,5 % verändert den Durchsatz aller Implementierungen außer quiche kaum.
- **s2n-quic**: keine veröffentlichte Durchsatzmessung gefunden; es wurde keine s2n-quic-Quelle geöffnet [U].
- **Tailscale** (wireguard-go, UDP und TCP, nicht QUIC): Abschnitt 2.2; die Tailscale-Messung Q03 benutzt MsQuic-`secnetperf` als Lastquelle.
- Hinweis zu iroh: Aggregatoren-Seiten (`instagit.com`), die „gigabit-class throughput“ für iroh behaupten, nennen keine Messungen und werden nicht als Quelle verwendet.

## 6 Transport für Bildschirm- und Spielestreaming

**Befund in Kürze.** [B] Alle gelesenen Streaming-Referenzen (Sunshine/Moonlight, Parsec, Microsoft RDP-UDP, WebRTC) übertragen Videobilder auf UDP-Basis mit **ratenbasierter, verzögerungsempfindlicher Regelung und Fehlerkorrektur oder Wiederholung nur nach Bedarf**, nicht auf einem zuverlässigen geordneten Strom. [B] Verlust wird dort je Bild behandelt (Vorwärtsfehlerkorrektur, Referenzinvalidierung, neues Schlüsselbild), und die Senderate folgt dem Laufzeitverhalten, nicht dem Verlust allein. [B] Eine verlustbasierte Staukontrolle in demselben Engpass drückt eine verzögerungsbasierte (GCC) nach unten (Q42, Q43).

### 6.1 WebRTC (SRTP, RTP, NACK, FEC, GCC)

- Pflichtteile (Q44, RFC 8834): SRTP/SRTCP („MUST use SRTP“, §4.2); NACK-Verständnis beim Sender und RTP-Wiederholung beim Empfänger „REQUIRED“ (§6.1); FEC nach RFC 8854 (§6.2); „WebRTC endpoints MUST ensure that the RTP traffic they generate can adapt“ (§7); „no standard congestion control algorithm“ (§7); alle RTP-Ströme über einen Transportfluss (§4.4).
- Google Congestion Control (Q42, Carlucci et al., MMSys 2016; übernommen aus L5 der Entwurf `draft-ietf-rmcat-gcc-02`, abgelaufen 2016-07-08, Auszug in L5 Abschnitt C2): Kalman-Filter über die Laufzeitänderung, Überlastdetektor mit adaptiver Schwelle plus verlustbasierter Regler; Senderate gleich dem Minimum. Q42: Mit **statischer** Schwelle wird ein GCC-Strom von einem langlebigen TCP-Strom „starved“; die adaptive Schwelle verhindert das und erlaubt faire Teilung.
- Q43 (Drucker, Balasubramanian, Gandhi, Stony Brook/Furman, COMSNETS 2025): GCC sei „starved by a loss-based TCP flow using the same bottleneck link“; WebRTC-BBR schlägt GCC im Wettbewerb mit TCP, unterliegt aber bei tiefen Puffern und eingeschränkter Bandbreite wegen Bandbreitenüberschätzung; ohne den „recovery state“ verbessert sich die Videoqualität um 63 %.
- Randbedingungen für se: WebRTC bringt eigenen ICE-, DTLS- und SRTP-Stack mit; neben iroh wären zwei Verbindungsaufbauten und zwei Identitätsprüfungen nötig (Folgerung aus L5 Abschnitt C, Zeile „WebRTC“). Rust-Implementierung `webrtc` 0.13.0 liegt bei RustDesk als optionale Abhängigkeit hinter einem Merkmal und ist dort auf einen eigenen Fork gepinnt, mit dem Kommentar, neuere Versionen verlangten ein neueres `rustc` (Q48, `Cargo.toml`). Lizenz von `webrtc-rs` nicht gelesen [U]; Reife der Rust-Implementierung nicht gemessen [U].
- Leistungsangaben mit Zahlen zu WebRTC-Videoübertragung (Latenz in ms bei Verlust): in den gelesenen Quellen nicht enthalten [U].

### 6.2 Sunshine und Moonlight

- Quellen: Q46 (moonlight-common-c README), Q47 (Sunshine-Konfiguration); Übernahme aus L5 Abschnitt C4: Steuerkanal über gebündeltes, verändertes ENet (UDP), Video als RTP-Pakete (Folgerung aus Quelltextbezeichnern), FEC Reed-Solomon je Bild, nicht wiederherstellbares Bild → Meldung an den Host („Referenzinvalidierung“), sonst IDR-Anforderung; Lizenz beider Projekte GPL-3.0.
- Q47: `fec_percentage` Vorgabe 20 (Bereich 1–255), „Percentage of error correcting packets per data packet in each video frame“; `ping_timeout` Vorgabe 10 000 ms; Verschlüsselung LAN aus, WAN falls der Client es kann (Abschnitt 4.2); Portfamilie ab 47989.
- Q46: gebündelte Untermodule `enet` und `nanors` (Reed-Solomon-Bibliothek, Zweck am README nicht beschrieben).
- Messwerte zu Latenz oder Verlusttoleranz: in Q46 und Q47 nicht enthalten [U].
- Umgang mit gleichzeitiger Massenübertragung: nicht dokumentiert in den gelesenen Quellen [U].

### 6.3 Parsec (BUD)

- Q45 (Parsec-Blog, Seitendatum nicht angezeigt): „BUD“ läuft auf UDP, bietet „basic reliability semantics like TCP“ und ein „highly tuned and custom congestion control algorithm“; verschlüsselt mit DTLS 1.2 über OpenSSL; Entwurfsannahmen: unter 50 ms Ping zum Spiele-PC, 30 Mbit/s; Ziel: Stau erkennen, **bevor** er eintritt, weil Video keine Puffer hat; bei starkem Verlust sinkt die Encoder-Bitrate; Kurzunterbrechungen halten den Strom bis zu 60 Sekunden an; Priorität: Latenz, dann Bildrate, dann Videoqualität; Q45 räumt ein, das Verhalten sei bei höherer Latenz zu konservativ. Zahlen zur Latenz oder zum Durchsatz fehlen [U]; Quelltext nicht öffentlich.

### 6.4 RustDesk

- Q48 (`hbb_common`): Module `tcp`, `udp`, `socket_client`, `stream`, `websocket`, `webrtc` (hinter Merkmal), `config`, `proxy`; vollständige Modulliste im Abruf: compress, protos, proxy, socket_client, tcp, udp, bytes_codec, config, mem, password_security, sh, fingerprint, log_throttle, rx_probe, stream, websocket, webrtc, tls, verifier. `Cargo.toml` enthält `sodiumoxide`, `tokio-rustls`, `webrtc` 0.13.0 (optional), **keine** KCP-Abhängigkeit und kein `quinn`. Die im Auftrag genannte KCP-Nutzung ist in diesen zwei Dateien nicht auffindbar [U]; die Suchanfrage nach einem KCP-Pull-Request in RustDesk lieferte keinen RustDesk-Treffer.
- Übernommen aus L5: Direkt per Hole-Punching (TCP 21116, UDP 21116), sonst Relay `hbbr` über TCP 21117 beziehungsweise WebSocket 21119; Lizenz AGPL-3.0; Verlustbehandlung nicht gelesen [U].

### 6.5 Microsoft RDP (RDP-UDP, Shortpath)

- Q49 (Microsoft Learn, „RDP Shortpath“, Seite aktualisiert 2026-11-04): UDP-basierter Transport „based on the Universal Rate Control Protocol (URCP)“, „enhances UDP with active monitoring of the network conditions and provides fair and full link utilization“; „UDP-based transport offers better connection reliability and more consistent latency“; Verbindung beginnt mit TCP-Reverse-Connect, UDP wird parallel aufgebaut (STUN, bei Bedarf TURN), danach werden alle dynamischen virtuellen Kanäle (Grafik, Eingabe, Geräte) auf den UDP-Transport verschoben; „TLS over reliable UDP“; Port 3390 (verwaltetes Netz) beziehungsweise zufällig im Bereich 49152–65535; ausdrückliche Empfehlung „Avoid using TCP-based VPN solutions“ und Hinweis, TCP-basierte VPN fügten „the inherited overhead of TCP congestion control“ hinzu; Sicherheit laut Seite gleich der des TCP-Transports; QoS über DSCP-Markierung und Ratenbegrenzung je Sitzung in verwalteten Netzen.
- Q50: MS-RDPEUDP2 „is used to exchange data, for example audio and video, … over UDP transport using a URCP based rate control“; Revision 6.0 vom 2024-04-23. Suchtreffer zu MS-RDPEMT/MS-RDPEUDP: zwei UDP-Transporte „reliable“ (TLS) und „lossy“ (DTLS), FEC-Bezeichnung in beiden; Einzelheiten zu FEC und Staukontrolle nicht gelesen [U]. Messwerte: in Q49 und Q50 keine.
- Q49 ist Herstellerdokumentation für einen Cloud-Dienst; Aussagen zur Latenz sind qualitativ.

### 6.6 Steam Remote Play

- Gefunden: Valve nennt UDP-Ports 27031 und 27036 sowie TCP-Ports 27036 und 27037, Remote Play sei in öffentlichen Netzen, Arbeits- oder Schulnetzen, Gastnetzen und Netzen mit Client-Isolation nicht unterstützt (Suchtreffer der Valve-Supportseite, Seite nicht geöffnet, Status in Abschnitt 10). Das Valve-Transportprojekt GameNetworkingSockets beschreibt AES-GCM-256 je Paket, Curve25519 und eine Bestätigungsvektor-Zuverlässigkeitsschicht; ob Remote Play es verwendet, ist nicht belegt; Bandbreitenschätzung sei dort „temporarily broken“ (Suchtreffer-Auszug). Das Streaming-Protokoll von Remote Play (Verlustbehandlung, FEC, Staukontrolle) ist nicht öffentlich beschrieben gefunden [U].

### 6.7 Media over QUIC, RTP über QUIC und QUIC-Datagramme

- RFC 9221 (Q31): DATAGRAM-Frames werden bei Verlust nicht wiederholt, sind aber ack-auslösend (§5.2); keine Flusssteuerung (§5.3); „The sender MUST either delay sending the frame until the controller allows it“ (§5.4) bzw. verwerfen; keine Fragmentierung; kein Strom-Bezug (§5.1), Anwendung muss Datenflüsse selbst kennzeichnen; Priorisierung: Implementierungen sollen eine API anbieten (§5.1); Schutz mit 0-RTT- oder 1-RTT-Schlüsseln (§6). [F] Datagramme laufen unter der Staukontrolle der Verbindung; ein verzögerungsarmer Strom neben einer Cubic-Massenübertragung auf derselben Verbindung teilt sich deren Fenster und Warteschlange.
- RTP über QUIC (Q52, `draft-ietf-avtcore-rtp-over-quic-14`, 2025-03-20): abgelaufener Entwurf in WG Last Call; Abstract: minimale Abbildung von RTP/RTCP auf QUIC, Nutzung von QUIC-Zuständen anstelle von RTCP, Optionen für Staukontrolle und Ratenanpassung ohne RTCP-Rückmeldung; Abschnitte zum Zusammenspiel der Regler, zur Abbildung auf Datagramme oder Ströme und zur Verlustbehandlung waren im Abruf nicht enthalten [U].
- Media over QUIC Transport (Q53, `draft-ietf-moq-transport-23`, 2026-11-02, Standards Track, noch kein RFC): Publish/Subscribe über QUIC und WebTransport; Gruppen werden über Ströme je Untergruppe ausgeliefert; Objekte als Datagramme möglich; Veröffentlicher-Priorität je Objekt; „It MUST reset any open streams associated with the SUBSCRIBE“ bei Ende des Abonnements; Abschnitte 5.1 (Planung) und 5.2 (Zustellfristen) sowie 14.1 (Pufferaufblähung, konstanter Durchsatz) waren im Abruf nicht enthalten [U]. Messwerte: keine in der Seite.
- Eine veröffentlichte Messung von Bildschirm- oder Spielestreaming über **QUIC-Datagramme** (Latenz, Verlust, Konstanz) wurde nicht gefunden [U].
- Cloudflare WARP über MASQUE (Q57, 2024-03-06 und 2024-12-27): QUIC-Pakete werden zusammengefasst („coalesced into the same UDP datagram“), Cloudflare führt MASQUE als bevorzugtes Protokoll vor WireGuard ein; keine Messwerte, keine Angabe zur Staukontrolle der Tunnelverbindung.

### 6.8 Behandlung von Verlust, Stau und gleichzeitiger Massenübertragung je Referenz

| Referenz | Verlust | Stau/Laufzeit | Gleichzeitige Massenübertragung | Quelle |
|---|---|---|---|---|
| WebRTC | NACK/RTX, FEC (RFC 8854), Schlüsselbild-Anforderung (PLI, aus L5) | GCC (verzögerungs- und verlustbasiert, Minimum) | GCC wird von verlustbasiertem TCP im selben Engpass verdrängt (Q42, Q43); adaptive Schwelle mildert | Q42, Q43, Q44, L5 |
| Sunshine/Moonlight | Reed-Solomon-FEC je Bild (20 % Vorgabe), Referenzinvalidierung, IDR | Bitratenwahl des Encoders; Verfahren nicht gelesen | nicht dokumentiert | Q47, L5 |
| Parsec BUD | Zuverlässigkeit „wie TCP“ auf UDP, Bitratensenkung bei Verlust | eigener Regler, Ziel: Stau vor Eintritt erkennen | nicht dokumentiert | Q45 |
| Microsoft RDP-UDP/URCP | FEC-Transporte „reliable“ und „lossy“ | URCP-Ratenregelung; DSCP in verwalteten Netzen | Warnung vor TCP-VPN (TCP-in-TCP) | Q49, Q50 |
| RustDesk | nicht gelesen | nicht gelesen | nicht gelesen | Q48, L5 |
| Steam Remote Play | nicht öffentlich gefunden | nicht öffentlich gefunden | nicht öffentlich gefunden | Abschnitt 6.6 |
| QUIC-Datagramme (RFC 9221) | keine Wiederholung; Anwendung muss behandeln | Staukontrolle der Verbindung (RFC 9221 §5.4) | teilt das Fenster mit Strömen derselben Verbindung | Q31 |
| MoQ-Transport | Ströme je Untergruppe, Rücksetzen veralteter Ströme | QUIC-Staukontrolle | über Priorität je Objekt | Q53 |

## 7 Portweiterleitung und TCP-Tunnel über QUIC oder VPN

**Befund in Kürze.** [B] TCP in einem TCP-Tunnel ist als Fehlerbild („TCP meltdown“) dokumentiert; WireGuard und RDP Shortpath raten davon ab. [B] QUIC-Ströme sind untereinander in der Reihenfolge unabhängig, teilen sich aber Fenster und Staukontrolle der Verbindung. [M] Ein veröffentlichter TCP-über-QUIC-Tunnel mit Quinn war im verlustfreien Fall langsamer als nacktes TCP und bei 20 % Verlust 4,6-mal schneller.

- TCP-in-TCP: OpenVPN-Dokumentation (Q54): „TCP Meltdown“ entstehe, wenn TCP-Verkehr in einem TCP-Tunnel läuft („overcompensating retransmissions“); Abhilfe: UDP als Tunneltransport. WireGuard (Q06): „WireGuard explicitly does not support tunneling over TCP, due to the classically terrible network performance“. Q49 (RDP Shortpath): „Avoid using TCP-based VPN solutions“. Der Originalaufsatz von Titz („Why TCP over TCP is a bad idea“) war nicht abrufbar (Verbindungsfehler `ECONNREFUSED` am 2026-11-14) [U].
- QUIC-Ströme (Q30, RFC 9000): „QUIC does not provide any means of ensuring ordering between bytes on different streams“ (§2); Strom-Flusssteuerung „prevents a single stream from consuming the entire receive buffer for a connection“, Verbindungs-Flusssteuerung begrenzt die Summe aller Ströme (§4.1). Staukontrolle gilt je Verbindung (RFC 9002, Q29); Verlust eines Pakets verzögert nur die Ströme, deren Daten im verlorenen Paket lagen [F aus §2 und RFC 9002].
- TCP über QUIC mit Quinn (Q38, Guo, Bao, Chen, arXiv 2504.10054 v2, 2025-10-07; zwei Debian-12-VMs, 4 vCPU je VM, Linux 6.1, iperf3, fünf Wiederholungen) **[M]**:
  - ohne Verlust: nacktes TCP 824 Mbit/s, Tunnel 132 Mbit/s (Bandbreitentest, Tabelle 1); in der Verlusttabelle 84,9 gegen 72,7 Mbit/s;
  - Verlust 5 %: Tunnel 58,3, TCP 47,5; Verlust 10 %: 32,2 gegen 19,5; Verlust 15 %: 18,1 gegen 5,14; Verlust 20 %: 11,1 gegen 2,41 Mbit/s (4,6-fach);
  - Umordnung 0–20 %: Tunnel 74,8–80,1 Mbit/s, TCP 44,0–52,3 Mbit/s.
  - Q38 nennt für den Tunnel „somewhat higher CPU usage“; die Messwerte der CPU in der Tabelle sind klein und uneinheitlich. Die absoluten Werte (unter 150 Mbit/s) beschreiben einen virtualisierten Aufbau, nicht die Grenzen von Quinn.
- MASQUE: CONNECT-UDP (Q55, RFC 9298) und CONNECT-IP (Q56, RFC 9484) tragen UDP beziehungsweise IP-Pakete in HTTP-Datagrammen, über QUIC-DATAGRAM-Frames bei HTTP/3. Beide Dokumente: „the proxied traffic will incur at least two nested congestion controllers“; die äußere Verbindung „MUST NOT disable congestion control unless it has an out-of-band way of knowing“, dass der innere Verkehr überlastgesteuert ist (RFC 9298 §6); RFC 9484 §10: Verschachtelung von Wiederholungsmechanismen bei TCP als äußerem Transport, daher „IP proxying SHOULD be performed over HTTP/3“; QUIC-Datagramme werden nicht fragmentiert, Pakete über der Pfad-MTU werden verworfen mit ICMP „Packet Too Big“ (RFC 9298 §6.1, RFC 9484 §10.1); IPv6 verlangt mindestens 1280 Byte (RFC 9484 §7.2). [F] Für eine Portweiterleitung auf Verbindungsebene (ein TCP-Anschluss gleich ein QUIC-Strom) entfällt die Verschachtelung der Staukontrolle für TCP; sie bleibt bei der Weiterleitung von UDP in Datagrammen bestehen.
- VPN-Variante der TCP-Freigabe (WireGuard plus Userspace-Stack): kein TCP-in-TCP, weil WireGuard auf UDP läuft; der innere TCP-Strom läuft über smoltcp (ohne SACK, Abschnitt 2.3) oder gVisor (Messwerte fehlen); der Weg verlangt einen vollständigen Userspace-TCP-Stack je Gerät neben dem Kernel-TCP [F].
- Ströme je Verbindung: Vorgabe 64 gleichzeitige bidirektionale Ströme in se (L4 Abschnitt A.1); QUIC-Grenze und Speicherformel: Speicher im ungünstigsten Fall gleich Stromanzahl mal `stream_receive_window`, begrenzt durch `receive_window` (L4 Abschnitt A.4).

## 8 Vergleich der Kandidaten je Komponente

### 8.1 Kandidaten

- **C1** QUIC wie heute: eine noq-Verbindung je Gegenstelle für alle Zwecke.
- **C2** QUIC mit mehreren Verbindungen zwischen denselben Geräten, je Zweck eine eigene Verbindung (eigener Protokollname, eigener Reglerzustand), dieselbe Bibliothek noq.
- **C3** C2 mit QUIC-Datagrammen (RFC 9221) für den Bildschirmstrom plus anwendungseigene Fehlerkorrektur und Ratenregelung (Reglerschnittstelle `ControllerFactory` aus noq).
- **C4** QUIC-Strom je Bild oder Bildgruppe, veraltete Ströme abbrechen (aus L5 Abschnitt C4).
- **C5** WireGuard im Programm (boringtun oder GotaTun) mit Userspace-TCP/IP (smoltcp oder gVisor-netstack), ohne Systemadapter.
- **C6** WireGuard mit Systemadapter (Linux-Kernelmodul, wireguard-nt mit Wintun, Android `VpnService`).
- **C7** Nebula. **C8** ZeroTier (libzt). **C9** Tailscale `tsnet`.
- **C10** WebRTC (SRTP/RTP, NACK, FEC, GCC) neben iroh.
- **C11** eigenes UDP-Protokoll nach dem Vorbild von Sunshine/Moonlight, Parsec oder RDP-UDP (FEC, eigene Ratenwahl, eigener Verbindungsaufbau).
- **C12** TCP mit TLS direkt im lokalen Netz als zusätzlicher Kanal (Kernel-TCP).
- **C13** Media over QUIC. **C14** MASQUE (CONNECT-UDP, CONNECT-IP) über QUIC.

### 8.2 Kriterienmatrix (Bewertungen bezeichnen, was belegt ist; leere Messung steht als „[U]“)

Kriterium „ohne Handgriffe“ gilt je Windows (W), Linux (L), Android (A) ohne Adminrechte, ohne Systemadapter, ohne Nutzerdialog.

| Kandidat | Sicherheit | Verlässlichkeit | Geschwindigkeit | Konstanz | Ohne Handgriffe W / L / A | Lizenz |
|---|---|---|---|---|---|---|
| C1 | TLS 1.3, AEAD, Header-Schutz (RFC 9001) [B] | Verlustwiederherstellung RFC 9002 [B] | noq 3,3–5,9 Gbit/s je Strom in Rechenzentrums-/Loopback-Messung [M Q36]; Windows, Android [U] | ein Fenster und eine Warteschlange für alle Zwecke [F] | ja / ja / ja (heutiger Stand) | Apache-2.0 oder MIT (Q39) |
| C2 | wie C1 | wie C1; Wiederverbindung je Verbindung [F] | Mehrkernnutzung je Verbindung möglich [F aus Q33: Sender auf einem Kern]; Messung mehrerer Verbindungen [U] | getrennte Staukontrollzustände je Zweck; gleicher Engpass bleibt [F] | ja / ja / ja [F: gleiche Bibliothek] | wie C1 |
| C3 | wie C1 (Datagramme unter 1-RTT-Schutz, RFC 9221 §6) | keine Wiederholung; Verlust wird von Anwendung behandelt [B Q31] | Rahmenrate begrenzt durch Datagrammgröße (kein Fragmentieren) [B Q31] | eigener Regler möglich (Reglerschnittstelle) [B L4]; Messung [U] | ja / ja / ja [F] | wie C1 |
| C4 | wie C1 | Wiederholung bis Abbruch je Strom | QUIC-Staukontrolle | Wiederholungen im selben Strom bis zum Abbruch erhöhen Latenz [F] | ja / ja / ja | wie C1 |
| C5 | WireGuard (Noise-basierter Handshake), ChaCha20-Poly1305 [B Q04] | Userspace-TCP ohne SACK (smoltcp) [B Q12]; gVisor [U] | wireguard-go mit TUN: 5,36–13,0 Gbit/s Linux [M Q01, Q02]; ohne Adapter, Windows, Android [U] | WireGuard ohne eigene Staukontrolle [B Q06, Q57]; Konstanz hängt vom inneren TCP ab | ja / ja / ja [F: nur wenn alle Anwendungen den Stack im Programm nutzen] | boringtun BSD-3, smoltcp 0BSD, tsnet BSD-3, wireguard-go [U Lizenz nicht gelesen], onetun MIT |
| C6 | wie C5 | Kernel-TCP | Kernel-WireGuard 2,66–11,8 Gbit/s [M Q01, Q02]; wireguard-nt etwa 7,5 Gbit/s [M Q20] | wie C5 | nein (Admin, Treiber) / nein (CAP_NET_ADMIN [A]) / nein (Dialog, ein VPN gleichzeitig) [B Q21, Q22, Q23] | Kernel GPLv2; wireguard-nt GPLv2 mit freizügigerer Binärlizenz [B Q19]; Wintun GPL 2.0 mit freizügigerer Binärlizenz [B Q18] |
| C7 | Noise, AES-256-GCM [B Q15] | wie C5 | etwa 9 Gbit/s Linux [M Q16, Hersteller] | [U] | Adapter [A]; Windows `wintun` im Quelltext [B Q15] | MIT |
| C8 | [U] nicht gelesen | [U] | etwa 3 Gbit/s Linux, einfädig [M Q16, Konkurrenz-Hersteller] | [U] | libzt: [U] | BSL 1.1 [B Q17] |
| C9 | WireGuard | wie C5 (gVisor) | wie C5 | wie C5 | ja ohne Root [B Q14]; Plattformumfang für Rust-Programm [U] | BSD-3 [B Q14] |
| C10 | SRTP, DTLS-SRTP [B Q44] | NACK/RTX, FEC [B Q44] | Messwerte [U] | GCC; Verdrängung durch verlustbasiertes TCP im selben Engpass [B Q42, Q43] | eigener ICE/DTLS-Stack [F]; Rust-Reife [U] | webrtc-rs [U] |
| C11 | Parsec DTLS 1.2 [B Q45]; Sunshine AES-GCM optional [B Q47] | FEC und Referenzinvalidierung [B L5, Q47] | Messwerte [U] | Ziel „Stau vor Eintritt erkennen“ [B Q45]; Messwerte [U] | ja / ja / ja [F: eigener Code] | Sunshine/Moonlight GPL-3.0, kein Code übernehmbar [B L5] |
| C12 | TLS 1.3 über rustls/ring [F] | Kernel-TCP-Verlustwiederherstellung | TCP füllt 10 Gbit/s, QUIC 2,4–8,22 [M Q33]; Android, Windows [U] | verlustbasierte Staukontrolle im Kernel [F] | ja / ja / ja [F: nur lokales Netz] | rustls Apache-2.0/MIT/ISC [A] |
| C13 | QUIC/TLS | Ströme je Untergruppe, Rücksetzen [B Q53] | [U] | Entwurf, kein RFC; keine Rust-Anbindung in se geprüft [U] | [U] | [U] |
| C14 | QUIC/TLS | verschachtelte Staukontrolle [B Q55, Q56] | [U] | [U] | ja [F] | RFC; Implementierung [U] |

### 8.3 Begründeter Vergleich je Komponente

Jede Zeile ist eine Folgerung [F] aus den in Abschnitt 2 bis 7 genannten Befunden. Messwerte für se selbst gibt es nicht (Abschnitt 1); eine Rangfolge bedeutet „beste belegte Lösung“, nicht „gemessen am se-Prototyp“.

**Dateitransfer groß.**

- Beste belegte Lösung: **C2** mit mehreren noq-Strömen und, wo die Bibliothek es freigibt, größeren Sendebündeln (`MAX_TRANSMIT_SEGMENTS`). Belege: Sender ist auf einen Kern begrenzt (Q33); größere Bündel heben Durchsatz um 14–28 % (Q37, Q36); Paketverarbeitung und ACKs, nicht die Verschlüsselung, begrenzen (Abschnitt 4.2); noq-Einzelstrom im Rechenzentrum 3,3–5,9 Gbit/s (Q36).
- Bedingt möglich: **C12** (Kernel-TCP mit TLS) als zusätzlicher Kanal im lokalen Netz oberhalb von etwa 5 Gbit/s Leitungsrate, weil TCP in Q33 10 Gbit/s füllt und QUIC höchstens 8,22 Gbit/s; unterhalb davon ohne belegten Gewinn; Voraussetzung ist eine Messung auf den Zielgeräten [U].
- Ausschlussgründe: C5, C7, C8, C9 (ersetzen Kernel-TCP durch Userspace-TCP oder fügen eine zweite AEAD-Schicht hinzu; Q02/Q01 zeigen, dass UDP-Paketkosten dominieren, nicht die Verschlüsselung; smoltcp ohne SACK; gVisor-Messwerte fehlen); C6 (Adminrechte, Treiber, Android „ein VPN gleichzeitig“); C10 (kein Massentransportprotokoll; Daten-Kanal-Eigenschaften von SCTP nicht untersucht [U]); C11 (neues zuverlässiges Protokoll ohne Belegvorsprung gegenüber QUIC); C13 (Medienprotokoll, Entwurf); C14 (verschachtelte Staukontrolle ohne Nutzen für Dateidaten).

**Viele kleine Dateien.**

- Beste belegte Lösung: **C1 oder C2** mit vielen gleichzeitigen Strömen und Pipelining der Anfragen. Belege: Ströme sind in der Reihenfolge unabhängig (Q30 §2); mehrere Ströme heben den Durchsatz mehrdateiiger Übertragungen teilweise (Q33); Pipelining und Parallelität sind die belegten Hebel für kleine Dateien (Globus/GridFTP, L3). Die Kosten liegen laut L3 im Namensraum des Dateisystems, nicht im Transport.
- Ausschlussgründe: Alle VPN-Kandidaten (C5 bis C9) ändern nichts am Namensraum und fügen Verarbeitungsschichten hinzu; C10, C11, C13, C14 haben keinen Bezug zu Dateitransfer; C12 hat keinen Strommultiplex ohne eigenes Protokoll.
- Trennung von großen Übertragungen (C2) ist durch Messung noch nicht belegt [U]; belegt ist nur, dass je Verbindung ein Fenster und ein Reglerzustand gelten (Q29).

**Bildschirmstrom.**

- Belegtes Muster der Referenzen (Sunshine/Moonlight, Parsec, RDP-UDP, WebRTC): UDP-basierter Transport, Bild-weise Fehlerbehandlung (FEC, Referenzinvalidierung, Schlüsselbild), verzögerungsempfindliche Ratenregelung, Bitratensenkung bei Verlust (Abschnitt 6.8). Kein Referenzprojekt überträgt Videobilder über einen zuverlässigen geordneten Strom als Regelweg.
- Beste belegte Lösung innerhalb der vorhandenen Bibliothek: **C3**: eigene Verbindung für den Bildschirm (getrennter Reglerzustand), QUIC-Datagramme für Bildpakete, eigene FEC und Referenzinvalidierung nach Moonlight-Vorbild, **eigener verzögerungsempfindlicher Regler** über `ControllerFactory`; zweite Wahl **C4** (ein Strom je Bild, Abbruch veralteter Bilder).
- Belege und Grenzen: Datagramme unterliegen der Staukontrolle der Verbindung (RFC 9221 §5.4); Cubic als Vorgabe ist verlustbasiert; ein verlustbasierter Regler in demselben Engpass verdrängt verzögerungsbasierte Regler (Q42, Q43); deshalb muss für „konstante Latenz“ auch der Dateitransfer-Verkehr bei gleichzeitigem Bildschirmstrom verzögerungsempfindlich geregelt oder begrenzt werden. Eine eigene Verbindung allein beseitigt die gemeinsame Warteschlange im Engpass nicht [F].
- Offen: Messung von QUIC-Datagrammen für Bildschirmstreaming [U]; Messung von noq-`Bbr3` [U] (L3: BBRv3 verlangt direkte Abhängigkeit auf `noq-proto`).
- Ausschlussgründe: C5, C6, C7, C8, C9 (WireGuard hat weder Verlustbehandlung noch Staukontrolle, Q06; der innere Verkehr trägt beides; kein Gewinn gegenüber C3); C10 (vollständig und belegt, aber zweiter ICE/DTLS-Stack neben iroh, Rust-Reife [U]; GCC unterliegt ohnehin demselben Wettbewerbsproblem, Q42, Q43); C11 (belegt als Muster, aber Code GPL-3.0, Wiederverwendung nur als Konzept); C12 (verlustbasierte Staukontrolle im Kernel, Verlustwiederholung erzeugt Latenzspitzen; Parsec begründet damit den Verzicht auf TCP, Q45); C13 (Entwurf noch nicht RFC, Abschnitte zu Zustellfristen nicht gelesen).

**TCP-Freigaben.**

- Beste belegte Lösung: **C1 oder C2**: ein QUIC-Strom je weitergeleiteter TCP-Verbindung, Bytes unverändert; belegt als Verfahren durch den TCP-über-QUIC-Tunnel mit Quinn (Q38: bei 5 bis 20 % Verlust schneller als TCP, bei verlustfreier Strecke langsamer als nacktes TCP) und durch die Vermeidung von TCP-in-TCP (Q54, Q06, Q49). UDP-Weiterleitung in Datagrammen nach dem Muster von RFC 9298 mit der dort genannten Pflicht, die Staukontrolle der äußeren Verbindung nicht abzuschalten.
- Ausschlussgründe: C5 (Userspace-TCP-Stack je Gerät nötig; smoltcp ohne SACK), C6 (Systemadapter: Dialog, Adminrechte, Android ein VPN gleichzeitig), C7 bis C9 (zweite Identitätsebene), C14 (verschachtelte Staukontrolle für UDP-Weiterleitung nach RFC-Wortlaut zulässig nur bei bekannter innerer Staukontrolle), C10, C11, C13 (kein Bezug).
- Unbelegt: Verhalten bei vielen gleichzeitigen Strömen und einem Massentransfer in derselben Verbindung (Fensterverteilung, `send_fairness`) [U].

**Interaktive Befehle und Terminal.**

- Beste belegte Lösung: **C2**: eigene, dauerhaft offene Verbindung mit kleinen Strömen; Begründung: Die Latenz kleiner Nachrichten hängt vom Reglerzustand und der Warteschlange der Verbindung ab (RFC 9002 Pacing und Fenster, Q29); eine eigene Verbindung entkoppelt vom Fenster einer laufenden Massenübertragung, nicht von der Warteschlange im Engpass [F].
- Belege sind schwach: Es wurde keine veröffentlichte Messung eines Terminals oder von Befehlsabläufen über QUIC gefunden [U]; die Folgerung beruht auf den allgemeinen Eigenschaften in 7 und 5.1.
- Ausschlussgründe: Alle VPN-Kandidaten (kein Latenzgewinn belegt; WireGuard misst 0,403 ms Ping gegen 0,501 / 0,508 / 1,541 ms bei IPsec/OpenVPN auf Gigabit-Ethernet im LAN, Q04 §8, Messung zwischen Rechnern ohne Last und ohne QUIC-Vergleich), C10, C11, C13, C14 (kein Bezug), C12 (Kernel-TCP bei Verlust).

### 8.4 Antwort auf die Nutzerfrage „ein VPN für alles oder mehrere“

[F] Nach den Quellen ist **kein** VPN-Kandidat (C5 bis C9) für irgendeine der fünf Komponenten die beste belegte Lösung:

- Ohne Systemadapter (C5, C9) sind die Messwerte für Windows und Android nicht veröffentlicht, und der Preis ist ein Userspace-TCP-Stack je Gerät.
- Mit Systemadapter (C6) entfällt „ohne Handgriffe“ auf allen drei Systemen.
- WireGuard bringt weder Staukontrolle noch Fehlerkorrektur mit (Q06); damit fehlen genau die Eigenschaften, die die Streaming-Referenzen als entscheidend benennen.

[F] Das aus den Quellen ableitbare Muster ist: **mehrere getrennte Verbindungen zwischen denselben Geräten je Zweck** (C2), mit je Zweck passender Regelung (verzögerungsempfindlich für den Bildschirm, durchsatzorientiert für Dateien), auf der vorhandenen Bibliothek. Dass getrennte Verbindungen je Zweck die Konstanz eines Bildschirmstroms neben einer Massenübertragung verbessern, ist in keiner gelesenen Quelle gemessen [U]; belegt sind nur die Bausteine: getrennter Reglerzustand je Verbindung (Q29), die Verdrängung verzögerungsbasierter durch verlustbasierte Regler im selben Engpass (Q42, Q43) und die Möglichkeit eines eigenen Reglers je Verbindung (Abschnitt 1).

## 9 Ungeklärte Angaben (vollständige Liste)

1. Durchsatz von boringtun als Bibliothek, von GotaTun und von onetun (Abschnitt 2.1, 2.3).
2. Durchsatz des gVisor-netstack und von smoltcp-Aufbauten auf Windows und Android; Durchsatz von libzt (Abschnitt 2.2, 2.3, 2.4).
3. Ob libzt, Nebula und GotaTun ohne Systemadapter einbettbar sind (Abschnitt 2.1, 2.4).
4. Binärlizenz-Wortlaut von Wintun und wireguard-nt; Lizenz von wireguard-go und `webrtc-rs` (Abschnitt 3, 6.1).
5. Messwerte für Verschlüsselung auf aktuellen Telefon-SoCs (ARMv8-Kryptobefehle) und auf Windows-Geräten (Abschnitt 4.1, 4.2).
6. Welche TLS-1.3-Suite noq 1.3.0 für 1-RTT-Pakete wählt; Verhalten des Schlüsselwechsels bei hoher Rate (Abschnitt 4.1).
7. Ob UDP-GSO/GRO beziehungsweise Windows-USO in der von noq 1.3.0 genutzten UDP-Schicht auf Windows und auf den Android-Kerneln der Zielgeräte tatsächlich wirken (Abschnitt 5.1; `noq-udp` ist nicht vendoriert und wurde nicht gelesen).
8. Ob eine noq-Version `MAX_TRANSMIT_SEGMENTS` als Einstellung freigibt (PR #645, Stand der Veröffentlichung nicht geprüft) (Abschnitt 5.1).
9. Messung mehrerer QUIC-**Verbindungen** gegen eine Verbindung (gleicher Engpass, Kernauslastung) (Abschnitt 5.1).
10. BBR-Messwerte von 2016 (Abruf HTTP 403); noq-`Bbr3` unter Verlust und mit Datagrammen (Abschnitt 5.1, 8.3).
11. s2n-quic: veröffentlichte Messung; MsQuic-Dashboard-Zahlen; quinn/noq-Messung über 10 Gbit/s (Abschnitt 5.3).
12. Ob iroh 1.3.0 mehrere Pfade gleichzeitig als `Available` führt und wie noq über mehrere Pfade verteilt (Abschnitt 1).
13. Messung von Bildschirm- oder Spielestreaming über QUIC-Datagramme; WebRTC-Latenz bei Verlust; Sunshine/Moonlight/Parsec/RustDesk-Latenzwerte (Abschnitt 6).
14. Nutzung von KCP in RustDesk; RustDesk-Verlustbehandlung (Abschnitt 6.4).
15. Steam Remote Play: Protokoll, FEC, Staukontrolle, Verhalten bei gleichzeitigem Massenverkehr (Abschnitt 6.6).
16. Details von RDP-UDP zu FEC und Staukontrolle (nur URCP-Name belegt) (Abschnitt 6.5).
17. RTP über QUIC und MoQ: Abschnitte zu Datagramm-/Strom-Abbildung, Zustellfristen, Pufferaufblähung nicht gelesen (Abschnitt 6.7).
18. Der Aufsatz von Titz zu TCP-über-TCP (Abruf fehlgeschlagen) (Abschnitt 7).
19. Verhalten vieler gleichzeitiger QUIC-Ströme neben einer Massenübertragung (Fensterverteilung, `send_fairness`) (Abschnitt 7).
20. Messung eines Terminals oder interaktiver Befehle über QUIC (Abschnitt 8.3).
21. Wirkung der Verschlüsselung auf se selbst: Messung `s4_messung_task_*` ist geschrieben, aber nie gelaufen (L1 Abschnitt B).
22. Suchtreffer ohne Prüfung: NepTUN, Firezone-Rust-WireGuard-Schicht, GitHub-Diskussion „Nebula benchmarks need to be published“ (#911) (Abschnitt 2.1, 2.4).

## 10 Quellenverzeichnis

Abrufdatum aller Q-Quellen: 2026-11-14. „Auszug“ bedeutet: lesbarer Auszug des Abrufwerkzeugs; „PDF gelesen“: Text oder Seitenbilder selbst gelesen; „Suchtreffer“: nur der Ausschnitt einer Websuche, Seite nicht geöffnet.

| Kennung | Quelle | URL | Version/Seitendatum | Gelesene Stelle | Status |
|---|---|---|---|---|---|
| Q01 | Tailscale, „Tailscale joins the 10Gb/s club on bare metal Linux“ | https://tailscale.com/blog/more-throughput | 2023-04-13 | Tabellen, Abschnitte zu GSO/GRO, Prüfsumme | Auszug |
| Q02 | Tailscale, „Surpassing 10Gb/s over Tailscale“ (Throughput improvements) | https://tailscale.com/blog/throughput-improvements | 2022-12-13 | Tabelle, TSO/GRO/mmsg | Auszug |
| Q03 | Tailscale, UDP-Durchsatz mit GSO/GRO | https://tailscale.com/blog/quic-udp-throughput | 2023-11-16 (Tailscale 1.54) | Tabelle secnetperf | Auszug |
| Q04 | WireGuard-Whitepaper | https://www.wireguard.com/papers/wireguard.pdf | Datum auf gelesenen Seiten nicht erkennbar | §5.4.6, §6.5, §7.1–7.4, §7.6, §8 (Seiten 13–18) | PDF gelesen (Seitenbilder) |
| Q05 | WireGuard, Performance | https://www.wireguard.com/performance/ | Seite nennt Linux 4.6.1 | gesamte Seite | Auszug |
| Q06 | WireGuard, Known Limitations | https://www.wireguard.com/known-limitations/ | ohne Datum | Punkte „TCP Mode“, „Hardware Crypto“ | Auszug |
| Q07 | Cloudflare boringtun, README | https://github.com/cloudflare/boringtun | ohne Version im Abruf | Lizenz, Plattformtabelle, Hinweis „restructuring“ | Auszug |
| Q08 | docs.rs boringtun | https://docs.rs/boringtun/latest/boringtun/ | 0.7.1 | Kopf der Seite | Auszug |
| Q09 | Cloudflare-Blog zu BoringTun | https://blog.cloudflare.com/boringtun-userspace-wireguard-rust | 2019-03-27 | gesamter Beitrag | Auszug |
| Q10 | Mullvad, „Announcing GotaTun“ | https://mullvad.net/en/blog/announcing-gotatun-the-future-of-wireguard-at-mullvad-vpn | 2025-12-19 | gesamter Beitrag | Auszug |
| Q11 | onetun, README | https://github.com/aramperes/onetun | Copyright 2025 | Architektur, Einschränkungen | Auszug |
| Q12 | smoltcp, README | https://github.com/smoltcp-rs/smoltcp | README nennt Rust 1.91 | Abschnitte Features, TCP, Leistung | Auszug |
| Q13 | Tailscale, „Kernel vs. netstack subnet routing“ | https://tailscale.com/kb/1177/kernel-vs-userspace-routers | validiert 2026-01-05 laut Seite | Abschnitte Performance, Plattformen | Auszug |
| Q14 | Go-Paket `tailscale.com/tsnet` | https://pkg.go.dev/tailscale.com/tsnet | Seite ohne Version im Auszug | Paketbeschreibung | Auszug |
| Q15 | Nebula, README | https://github.com/slackhq/nebula | ohne Version im Abruf | Plattformen, Verschlüsselung | Auszug |
| Q16 | Defined Networking, „Nebula is not the fastest mesh VPN“ | https://www.defined.net/blog/nebula-is-not-the-fastest-mesh-vpn/ | 2024-02-16 | Testaufbau, Tabellen | Auszug |
| Q17 | ZeroTier libzt, README | https://github.com/zerotier/libzt | ohne Version im Abruf | Lizenz, Plattformen | Auszug |
| Q18 | Wintun | https://www.wintun.net/ | ohne Datum | gesamte Seite | Auszug |
| Q19 | WireGuardNT | https://git.zx2c4.com/wireguard-nt/about/ | Copyright bis 2026 | Beschreibung, Lizenz, API | Auszug |
| Q20 | WireGuardNT-Ankündigung | https://lists.zx2c4.com/pipermail/wireguard/2021-August/006887.html | 2021-08-02 | Messwerte | Auszug |
| Q21 | WireGuard für Windows (Beschreibung und `docs/attacksurface.md`) | https://git.zx2c4.com/wireguard-windows/about/ und https://git.zx2c4.com/wireguard-windows/about/docs/attacksurface.md | Copyright bis 2026 | Abschnitte zu Diensten und Rechten | Auszug |
| Q22 | Android `VpnService` | https://developer.android.com/reference/android/net/VpnService | Seite ohne Datum; Auszug deckte die ersten 100 000 von 221 740 Zeichen ab | Klassenbeschreibung, `prepare`, `protect` | Auszug |
| Q23 | Android, „Build VPN solutions“ | https://developer.android.com/develop/connectivity/vpn | ohne Datum | Abschnitte VPN service, Always-on, Per-app | Text vollständig im Auszug |
| Q24 | Cloudflare, „It takes two to ChaCha (Poly)“ | https://blog.cloudflare.com/it-takes-two-to-chacha-poly/ | 2016-04-04 | Abschnitte zu AVX2 und AES-GCM | Auszug |
| Q25 | Cloudflare, „Do the ChaCha“ | https://blog.cloudflare.com/do-the-chacha-better-mobile-performance-with-cryptography/ | 2015-02-23 | Abschnitt zu Galaxy Nexus | Auszug |
| Q26 | A. Vardanian, „ChaCha vs AES 2025“ | https://ashvardanian.com/posts/chacha-vs-aes-2025/ | 2025-11-07 | Tabelle | Auszug |
| Q27 | Graviola-Benchmarks | https://jbp.io/graviola/ und https://jbp.io/graviola/reports/x86_64/report/index.html | Graviola 0.4.4; Messdatum nicht angegeben | Hardware- und Bibliotheksliste; Durchsatzzahlen nur aus Suchtreffern der vier Berichtsseiten `https://jbp.io/graviola/reports/{x86_64,aarch64}/{aes256-gcm,chacha20poly1305}/aws-lc-rs/16KB/report/index.html` | Auszug / Suchtreffer |
| Q28 | RFC 9001 | https://www.rfc-editor.org/rfc/rfc9001.html | RFC | §5.3, §5.4, §6, §6.6 (erste 100 000 von 143 048 Zeichen) | Auszug |
| Q29 | RFC 9002 | https://www.rfc-editor.org/rfc/rfc9002.html | RFC | §7, §7.2, §7.7 | Auszug |
| Q30 | RFC 9000 | https://www.rfc-editor.org/rfc/rfc9000.html | RFC | §1, §2, §4.1, §5.1.2 (erste 100 000 von 447 469 Zeichen) | Auszug |
| Q31 | RFC 9221 | https://www.rfc-editor.org/rfc/rfc9221.html | RFC | §3, §5.1–5.4, §6 | Auszug |
| Q32 | Zhang et al., „QUIC is not Quick Enough over Fast Internet“ | https://arxiv.org/abs/2310.09423 | WWW ’24, arXiv Oktober 2023 | Zusammenfassung | Auszug |
| Q33 | König, Rust, Zitterbart, Scheuermann, „Examining the Heterogeneous Throughput Performance Landscape of QUIC Implementations“ | https://doc.tm.kit.edu/2025-Examining-the-Heterogeneous-Throughput-Performance-Landscape-of-QUIC-Implementations-Koenig-et-al.pdf | IFIP Networking 2025 | Abschnitte I–VII | PDF gelesen (Text) |
| Q34 | Cloudflare, „Accelerating UDP packet transmission for QUIC“ | https://blog.cloudflare.com/accelerating-udp-packet-transmission-for-quic/ | 2020-01-08 | gesamter Beitrag | Auszug |
| Q35 | Microsoft, „Making MsQuic Blazing Fast“ | https://techcommunity.microsoft.com/blog/networkingblog/making-msquic-blazing-fast/2268963 | 2021-04-14, Update 2021-04-15 | Abschnitt Upload-Geschwindigkeit | Auszug |
| Q36 | noq, Pull Request #636 (und Verweis auf #645) | https://github.com/n0-computer/noq/pull/636 | eröffnet 2026-05-05, geschlossen 2026-06-03 | Beschreibung, Messtabellen | Auszug |
| Q37 | Brandner et al., „Better QUIC implementations with Nesquic“ (Poster) | https://www.research-collection.ethz.ch/server/api/core/bitstreams/03d541ee-30b2-40bb-bd0b-fa3a0061adce/content (DOI 10.3929/ethz-c-000783476) | 2025 | Abschnitte 2–3 | PDF gelesen (Text) |
| Q38 | Guo, Bao, Chen, „Implementation and Performance Evaluation of TCP over QUIC Tunnels“ | https://arxiv.org/abs/2504.10054 und https://arxiv.org/html/2504.10054v2 | arXiv 2025-04-14, v2 2025-10-07 | Tabellen 1, 2, 4 | Auszug |
| Q39 | noq, README | https://github.com/n0-computer/noq | ohne Version im Abruf | Beschreibung, Lizenz, Standards | Auszug |
| Q40 | n0, „noq, noq, who's there?“ | https://iroh.computer/blog/noq-announcement | 2026-03-19 | gesamter Beitrag | Auszug |
| Q41 | IETF Datentracker, Multipath QUIC (RFC 10069) | https://datatracker.ietf.org/doc/draft-ietf-quic-multipath/ | Seite zuletzt 2026-11-07 | Abstract, Staukontrolle, Planung | Auszug |
| Q42 | Carlucci, De Cicco, Holmer, Mascolo, GCC | https://cs538.github.io/readings/carlucci16.pdf | MMSys 2016 | Abschnitte zu TCP-Wettbewerb und adaptiver Schwelle | PDF gelesen (Text, Suche nach „starv“) |
| Q43 | Drucker, Balasubramanian, Gandhi, „Investigating WebRTC BBR as an alternative to GCC“ | https://www3.cs.stonybrook.edu/~anshul/comsnets25_webrtcbbr.pdf | COMSNETS 2025 | Abstract, Einleitung | PDF gelesen (Text) |
| Q44 | RFC 8834 | https://www.rfc-editor.org/rfc/rfc8834.html | RFC | §4.2, §4.4, §4.5, §6.1, §6.2, §7, §7.1 (erste 100 000 von 130 779 Zeichen) | Auszug |
| Q45 | Parsec, Blog zu BUD | https://parsec.app/blog/a-networking-protocol-built-for-the-lowest-latency-interactive-game-streaming-1fd5a03a6007 | Datum nicht angezeigt | gesamter Beitrag | Auszug |
| Q46 | moonlight-common-c, README | https://github.com/moonlight-stream/moonlight-common-c | ohne Version | Untermodule, Lizenz | Auszug |
| Q47 | Sunshine, Konfiguration | https://docs.lizardbyte.dev/projects/sunshine/latest/md_docs_2configuration.html | „latest“ am Abrufdatum | Einträge `fec_percentage`, `lan_encryption_mode`, `wan_encryption_mode`, `port`, `ping_timeout` | Auszug |
| Q48 | RustDesk `hbb_common` | https://raw.githubusercontent.com/rustdesk/hbb_common/main/Cargo.toml und https://raw.githubusercontent.com/rustdesk/hbb_common/master/src/lib.rs | `hbb_common` 0.1.0 | Abhängigkeiten, Module | Auszug |
| Q49 | Microsoft Learn, „RDP Shortpath“ | https://learn.microsoft.com/en-us/azure/virtual-desktop/rdp-shortpath | `ms.date` 2026-10-23, `updated_at` 2026-11-04 | gesamte Seite | Text vollständig |
| Q50 | Microsoft, MS-RDPEUDP2 (Seite) und Suchtreffer zu MS-RDPEUDP/MS-RDPEMT | https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp2/9db34630-e880-4bfd-9d8d-50bc044c3288 | Revision 6.0 vom 2024-04-23 | Einleitungssatz; MS-RDPEUDP nur Suchtreffer | Auszug / Suchtreffer |
| Q51 | Valve, Remote-Play-Support und GameNetworkingSockets | https://help.steampowered.com/en/faqs/view/3E3D-BE6B-787D-A5D2 und https://github.com/ValveSoftware/GameNetworkingSockets | nicht abgerufen | nur aus Suchtreffer-Zusammenfassung | Suchtreffer |
| Q52 | IETF, `draft-ietf-avtcore-rtp-over-quic` | https://datatracker.ietf.org/doc/draft-ietf-avtcore-rtp-over-quic/ | Revision 14, 2025-03-20; Seite 2026-05-25 | Abstract, Status | Auszug |
| Q53 | IETF, `draft-ietf-moq-transport` | https://datatracker.ietf.org/doc/draft-ietf-moq-transport/ | Revision 23, 2026-11-02 | Abstract, Lieferung, Rücksetzen (erste 100 000 von 402 739 Zeichen) | Auszug |
| Q54 | OpenVPN Access Server, FAQ TCP Meltdown | https://openvpn.net/as-docs/faq-tcp-meltdown.md | ohne Datum | gesamte Seite | Auszug |
| Q55 | RFC 9298 (CONNECT-UDP) | https://www.rfc-editor.org/rfc/rfc9298.html | RFC | §1, §3.1, §5, §6, §6.1 | Auszug |
| Q56 | RFC 9484 (CONNECT-IP) | https://www.rfc-editor.org/rfc/rfc9484.html | RFC | §1, §6, §7.1, §7.2, §10, §10.1 | Auszug |
| Q57 | Cloudflare, MASQUE-Beiträge | https://blog.cloudflare.com/zero-trust-warp-with-a-masque/ (2024-03-06) und https://blog.cloudflare.com/masque-now-powers-1-1-1-1-and-warp-apps-dex-available-with-remote-captures/ (2024-12-27) | siehe Spalte URL | gesamte Beiträge | Auszug |
| Q58 | Linux-Kernel-Dokumentation, Segmentation Offloads | https://www.kernel.org/doc/html/latest/networking/segmentation-offloads.html | „latest“ am Abrufdatum | Abschnitt UDP Segmentation Offload | Auszug |

Nicht abrufbar am 2026-11-14: Titz-Aufsatz (`http://sites.inka.de/bigred/devel/tcp-tcp.html`, `ECONNREFUSED`), `web.archive.org` (vom Werkzeug gesperrt), ACM Queue zu BBR (`https://queue.acm.org/detail.cfm?id=3022184`, HTTP 403), netdev-Aufsatz zu WireGuard im Kernel (`https://iris.polito.it/retrieve/4cdd2a3e-6718-4276-8d85-751eaf2e4551/netdev-0x18-paper23-talk-paper.pdf`, HTTP 403), `https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp/1ccb7a7b-1b0f-4e1a-8c3d-3b0b4b0c5c3a` (HTTP 404, geratene Adresse), ZeroTier-Forum-Artikel (Weiterleitung auf die Startseite).

Lokale Quellen (nur gelesen):

- L1 `docs/plaene/2026-11-14-verbindungen-je-zweck/plan.md`
- L2 `docs/plaene/2026-11-07-sicherheit-direktweg/machbarkeit-unverschluesselt.md`
- L3 `docs/refs/uebertragung-tempo-2026-10.md` (Abschnitte zu QUIC in noq/iroh, BBR, Globus/GridFTP, Kompression)
- L4 `docs/refs/quic-sftp-throughput.md` (Abschnitt A, QUIC-Fenster; geprüft 2026-09-28 an iroh 1.0.1)
- L5 `docs/refs/bildschirm-streaming-2026-10-17.md` (Abschnitte C1 bis C4, Tabellen zu Sunshine, Moonlight, RustDesk, Parsec, Chrome Remote Desktop)
- L6 `vendor/noq-proto-1.3.0/src/config/transport.rs`, `src/connection/mod.rs`, `src/crypto/rustls.rs`
- L7 `vendor/noq-1.3.0/src/connection.rs`, `src/runtime/tokio.rs`
- L8 `vendor/iroh-1.3.0/Cargo.toml`, `src/endpoint/quic.rs`
- L9 `native/Cargo.toml`
- `docs/refs/INDEX.md` wurde nicht inhaltlich ausgewertet (nur Größenprüfung); eine Zeile für dieses Dokument wurde dort nicht ergänzt, weil der Auftrag nur die Änderung von `recherche.md` erlaubt.
