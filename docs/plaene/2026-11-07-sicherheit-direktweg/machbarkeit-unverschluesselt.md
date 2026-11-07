# Machbarkeit: unverschlüsselte Übertragung, je Übertragung während des Laufs umschaltbar (Befund s4)

Stand: 2026-11-07. Ergebnis der Prüfung im Batch „Sicherheit und Direktweg“.
Umsetzung in diesem Batch: keine (Entscheidung E8 im Plan `docs/plaene/2026-11-07-sicherheit-direktweg/plan.md`).

## 1 Anforderung

Nutzer, Nachricht 1 vom 2026-11-07, wörtlich: „Datenübertragung passiert in der Regel aktuell verschlüsselt,
das gefällt mir auch. Ich glaube, sie passieren im Moment verschlüsselt. Aber was ich mein ist, ich will. Will ich
das, dass wenn die Datenübertragung dadurch verlangsamt wird, man die Möglichkeit hat, Daten unverschlüsselt zu
übertragen? Äh. Vielleicht will ich das. Schau mal, ob das umsetzbar ist, dass man das dann nach Bedarf umschalten
kann, während eine Übertragung läuft. Für diese eine Übertragung.“

Geprüft wurde, ob ein Schalter je Übertragung umsetzbar ist, der während des Laufs wirkt und nur die eine Übertragung
betrifft.

## 2 Ergebnis

- (a) Innerhalb der bestehenden QUIC-Verbindung ist ein Klartext-Schalter je Übertragung nicht möglich: Die Schlüssel gelten
  je Verbindung, und mehrere Übertragungen teilen eine Verbindung (Abschnitt 3.1).
- (b) Technisch umsetzbar ist nur ein zweiter Datenkanal je Übertragung: eine eigene Verbindung oder ein eigener Strom,
  der über die verschlüsselte Verbindung ausgehandelt wird, mit Integritätsschutz je Block (Abschnitt 3.2).
- (c) Der erwartete Gewinn ist gering (Annahme, nicht gemessen; Abschnitt 4). Der Sicherheitsverlust ist hoch (Abschnitt 5).
- (d) Empfehlung: zuerst die Kosten der Verschlüsselung auf den Zielgeräten messen (Abschnitt 6). Eine Umsetzung nur nach
  ausdrücklicher Nutzerentscheidung (offene Frage Q1, Abschnitt 8).

## 3 Prüfung der Wege

### 3.1 Innerhalb der bestehenden Verbindung

- Pro Gegenstelle besteht eine QUIC-Verbindung für alle Übertragungen (`native/crates/engine/src/share/core/session.rs:301-304`,
  `native/crates/engine/src/share/core/node_sessions.rs:265-330`). Paketschlüssel gelten je Verbindung, nicht je Stream oder
  Übertragung. Ein Schalter je Übertragung hätte also keine Schlüsselwirkung.
- iroh 1.3.0 und noq 1.3.0 haben keinen Klartextpfad: Die Verschlüsselung ist Teil des Transports (TLS 1.3 im QUIC-Handshake).
- QUIC verlangt AEAD-Verschlüsselung der Nutzdaten und einen aus dem Schlüssel abgeleiteten Header-Schutz (RFC 9001, Abschnitt 5).

### 3.2 Zweiter Datenkanal je Übertragung

Mögliche Ausprägungen, nicht gebaut:

- (i) eine eigene QUIC-Verbindung mit Null-Paketschlüssel, ausgehandelt über die verschlüsselte Verbindung. Dafür wäre ein Vendor-Patch an
  `noq-proto` nach dem abgelaufenen Entwurf `draft-banks-quic-disable-encryption-00` nötig (Transportparameter `disable_1rtt_encryption`,
  ausgehandelt für die ganze Verbindung; Schutz nur über die UDP-Prüfsumme; Zielumgebung „trusted paths“).
- (ii) ein TCP-Strom im LAN mit Integritätsschutz je Block. Der Schlüssel käme aus dem TLS-Exporter der verschlüsselten Verbindung.
  Umschaltung zwischen Blockbereichen über die vorhandenen Bereichsanfragen (`ranges_v1`).

Beide Ausprägungen bräuchten eine eigene Wire-Nachricht, eine Freigabe je Übertragung und eine Prüfung der Gegenstelle.

### 3.3 Der IETF-Entwurf draft-banks

Der Entwurf `draft-banks-quic-disable-encryption-00` (N. Banks, 2020) ist abgelaufen und experimentell. Er schaltet
Verschlüsselung für die ganze Verbindung aus und entfernt Header-Schutz, Payload-Schutz und den 16-Byte-AEAD-Tag je Paket.
Er ist daher für einen Schalter je Übertragung nicht geeignet, auch wenn er technisch die Verbindungsebene beträfe.
Ein Angreifer in der Mitte kann versuchen, die Erweiterung zu erzwingen.

### 3.4 Integrity-only-Suiten in TLS 1.3

TLS 1.3 kennt Integrity-only-Suiten (RFC 9150, `TLS_SHA256_SHA256`). QUIC verlangt jedoch AEAD und Header-Schutz (Abschnitt 3.1).
Ob rustls RFC 9150 anbietet, ist nicht am rustls-Quelltext geprüft (Annahme). Folgerung: kein Weg innerhalb der bestehenden QUIC-Verbindung.

## 4 Leistungsbezug (Annahme, nicht gemessen)

- Bestandsschätzung aus `docs/plaene/2026-10-28-verbinden-ist-einrichtung/analyse-g2-geraete.md`, Zeilen 47–53: je Gegenstelle ein
  Verbindungs-Treiber-Task „ein Kern für Paketbau, AES-GCM und Senden“. Die Autoren schreiben: „Auf 1 Gbit reicht das; auf 10 Gbit ist das
  die erwartete Decke (Schätzung: 2–5 Gbit/s Desktop, deutlich weniger auf Telefonen)“. Dies ist eine Schätzung, keine Messung.
- Im Repository gibt es keine Messung mit und ohne Verschlüsselung (Bestandsprüfung, Plan Abschnitt 4).
- Annahme: Mit AES-NI (x86) bzw. ARMv8-Kryptobefehlen kostet AES-GCM nur einen Bruchteil der Paketverarbeitung. Bis etwa 1 Gbit/s
  ist der Gewinn eines Klartextwegs deshalb nach dieser Annahme nicht messbar; darüber begrenzt die Paketverarbeitung je Verbindung.
  Diese Annahme ist durch die Messung in Abschnitt 6 zu belegen oder zu widerlegen.

## 5 Sicherheitsbewertung

- Vertrauensgrenze: jedes Netzglied zwischen den Geräten (WLAN-Mithörer, Router, Provider-Netz bei Mobilfunk) sieht den Inhalt.
- Das Relay darf keinen Klartext sehen. Klartext über das Relay widerspräche `docs/SHARE_SERVER.md` (Einleitung: das Relay
  sieht Routing-Metadaten und Chiffrat) und der Grundregel „Relay nur als gekennzeichneter Rückfall“.
- Klartext erreicht eine Gegenstelle nur mit Zustimmung beider Geräte.
- Ein Angriff, der Klartext erzwingt (Herabstufung), ist nicht ausgeschlossen, solange der Schalter nicht an die Entscheidung je Übertragung
  gebunden ist.

## 6 Messplan (vor jeder Umsetzung, nicht Teil dieses Batches)

- Messgrößen je Gerät: Durchsatz einer Übertragung im LAN mit dem Standardweg (verschlüsselt) und mit einem Messaufbau ohne Paketschutz,
  ausschließlich zum Messen und nicht als Produktweg; CPU-Zeit je übertragenem Gigabyte.
- Messgeräte: die Zielgeräte des Nutzers (Desktop Linux, Desktop Windows, Android-Telefon). Die Geräte werden in diesem Dokument nicht
  namentlich genannt.
- Entscheidungsschwelle: vom Nutzer festzulegen (offen, Abschnitt 8). Ohne Schwelle wird keine Umsetzung begonnen.
- Der Messaufbau ist kein Bestandteil des Produkts und kein Release-Artefakt.

## 7 Quellen

Abrufdatum aller Quellen: 2026-11-07 (Planstand). In dieser Dokumentationssitzung wurden die externen Quellen nicht erneut abgerufen;
die Angaben stammen aus dem Plan `docs/plaene/2026-11-07-sicherheit-direktweg/plan.md` (Abschnitt 4) und aus dem Quelltext der
gesperrten Crates im Repository.

- iroh-Blog „noq, noq, who's there?“, https://iroh.computer/blog/noq-announcement (QAD ersetzt STUN ab iroh 0.32).
- docs.rs, iroh 0.95.1 (Angabe „relay server provides QUIC Address Discovery (QAD) and hole-punching assistance“),
  https://docs.rs/crate/iroh/=0.95.
- IETF-Entwurf draft-banks-quic-disable-encryption-00 (abgelaufen, experimentell), https://datatracker.ietf.org/doc/html/draft-banks-quic-disable-encryption-00.
- RFC 9001, Abschnitt 5 (Header-Schutz und AEAD in QUIC); RFC 9150 (TLS-Integrity-only-Suiten).
- Quelltext der vendorierten Crates im Repository (Pfade im Plan, Abschnitt 3).

## 8 Offene Punkte

- Q1 (Nutzerfrage): Soll nach diesem Bericht eine je Übertragung zuschaltbare unverschlüsselte Übertragung gebaut werden (nur Direktweg,
  nie Relay, mit Integritätsschutz, Zustimmung beider Geräte), oder zuerst nur die Messung aus Abschnitt 6? Ohne Antwort wird nichts davon
  gebaut.
- Entscheidungsschwelle für die Messung (Abschnitt 6): vom Nutzer festzulegen.
- Unbelegt: ob rustls RFC 9150 anbietet (Abschnitt 3.4); Messwerte (Abschnitt 4).
