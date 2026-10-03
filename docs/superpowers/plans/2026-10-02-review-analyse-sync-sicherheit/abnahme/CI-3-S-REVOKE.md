# CI-3-S-REVOKE: zugelassener Altpeer ohne Capability-RPC

Stand: 2026-10-03. Der freigegebene Sourceanschluss ist abgeschlossen;
Verhaltens- und Compilerbestätigung durch dieselbe Root-eigene Remote-RV1-Suite
stehen aus. Kein neues Projekt-Review; keine lokalen App- oder Altversionsprogramme ausgeführt.

## Belegter Befund und Plan

Grundlage: Run `37157166735`, Kandidat
`71a8ca45697272453c213c9b0b5412d0cbed0f71`, Formatterpatch `c8c0d2ef`
und [CI-Closure-Plan](../ci-closure-fixes.md), Meilenstein S-REVOKE.
Frisch gelesen: [Share-TLS-/Identitätsrefs](../../../../refs/share-server-tls-auth.md)
und [RV1-Refs](../../../../refs/rv1-remote-suite.md).

`/tmp/rv1-ci-third/linux/mixed-version.log` zeigt den tatsächlichen Fehler
beim Stage `NEW to OLD: explicitly probe the pinned peer after legacy acceptance`.
Der alte Daemon akzeptiert die vollständig gepinnte Iroh-Session, meldet danach
`unknown variant capabilities`. `peer_list_batch::list` fragt über
`peer_stream::features` bereits vor `ListDir` nach modernen Capabilities.
Dadurch erreicht die freigegebene CI-2-Rootprobe ihre eigentliche Leseoperation
nicht; der vorhandene Open-Retry wiederholt denselben nicht unterstützten RPC.
Das gespeicherte Runnerprofil bleibt unter
`/tmp/rv1-ci-third/linux/se-share-mixed-version.tiw1DC` unverändert.

## Umsetzung und Fundzuordnung

1. `legacy_probe::confirm_pending` verwendet nach seinen unveränderten
   Auth-/Identity-/Profilprüfungen eine unmittelbare alte `ListDir`-Rootprobe.
   Die erfolgreiche reale Operation bleibt vor der unveränderten frischen
   Stop-/Entzugs-/Identity-/CAS-Persistenzgrenze zwingend.
2. `backend_capabilities.rs` bündelt die bisherige Mountabfrage und den
   konservativen Vertrag eines tatsächlich zugelassenen Altpeers.
   `peer_endpoint_source` liefert dafür die aktuelle gebundene Relation.
3. `peer_stream::features`, `peer_transfer::probe_transfer_caps`,
   Mount-/StagedWrite-Probes sowie die drei belegten vorgeschalteten Abfragen
   in `peer_storage_snapshot`, `peer_storage_analysis` und
   `peer_extensions::target_limits` nutzen denselben Vertrag.
4. Der vorhandene Mixed-Version-Einstieg erhält nach den erfolgreichen
   CLI-entdeckten Root-/Statoperationen einen additiven Nachweis, dass der
   wirklich zugelassene Altpeer keinen unbekannten `capabilities`-RPC erhält.

## Entscheidungs- und Sicherheitsvertrag

Die Klassifikation erfordert eine aktuelle Directrelation: Accepted mit
Accepted-Key und Zeitpunkt; vollständige übereinstimmende Geräte-, Schlüssel-,
Node- und lokal abgeleitete Fingerprintpins; exakte Lookup-/Scopebindung;
Unsigned ohne bestehenden unmittelbaren oder persistierten Signaturmarker;
genau eine eigene ausgehende LegacyForwarded-Anfrage mit passenden lokalen
und entfernten Identitätspins, Pending/Revision null und ohne moderne
Request-Receipt/Decision/Decision-Receipt. Direct muss online sein.
Raumbeziehungen und ungebundene Kontakte erhalten diesen Pfad nicht.

Diese Fakten allein gelten nicht als neue Peerzulassung. Der Helfer verlangt
eine noch lebende, zuvor tatsächlich authentifizierte gepinnte Session oder
führt nach einem Neustart eine echte alte `ListDir`-Operation aus. Nach I/O
werden Stop, aktuelle Relation und Pins erneut geprüft. Keine Authsperre
bleibt über Peer-I/O gehalten. Request-/Presencefristen der ausdrücklichen
Pending-Bestätigung sowie ihre CAS-Gates bleiben unverändert.

EOF, Timeout, Fehlertexte oder ein Server-Advert bestimmen nie das
Protokollalter und begründen nie Accepted. Die alten Antwortgates und das
Verbot ungebundener Unsigned-Entscheidungen bleiben erhalten. Bekannte
Signaturmarker verweigern ausdrücklich die Rückstufung. Es entstehen keine
neuen Optionen, Grants, Schreib-/Exec-Policies oder signierten Entscheidungen.

Der Altvertrag meldet sämtliche modernen Host-/Transferfeatures false,
StagedWrite-Garantien false und `RootConfinement::Unverified`.
Er erzeugt keine Mount-Lease. Die bestehende Lease wird bei jeder neuen
Acquire-Abfrage weiterhin vor einem möglichen Fehlschlag freigegeben;
eine alte Rootbindung bleibt dadurch nicht aktiv. Sichere Mountgarantien
werden ohne Altprotokollbeleg nicht behauptet.

Normale alte `ListDir`-/`Stat`-/`Read`- und vorhandene Dateisystemrequests
bleiben an den bisherigen Session-, Hostrechte- und Requestgrenzen.
Der vorhandene alte `WalkTree`-Fallback bleibt bestehen. Optionale moderne
Analyse meldet Legacy/None; Storage-Limits bleiben Unknown/default.
Dies erklärt fehlende Fähigkeiten und ersetzt keine Dateizugriffszulassung.
Der moderne Peer bleibt am bisherigen Capabilities-/Leasevertrag.
Allgemeine Retries, Streams, Decoder, Stagebesitz, Mutation und Exec wurden
nicht geändert.

## Interne API und Integration

`PeerBackend::probe_legacy_root() -> io::Result<Vec<VfsMeta>>` ist die
alte, ungeleaste Rootoperation für den vorhandenen ausdrücklichen Proof.
`PeerBackend::legacy_capabilities() -> io::Result<bool>` prüft die aktuelle
gebundene Klassifikation und tatsächliche Sessionzulassung.
`PeerEndpointSource::legacy_direct` und `is_legacy_direct` sind rein interne
Anschlüsse. Die Mountmethoden liegen kohäsiv in `backend_capabilities.rs`;
ihre Aufrufer und Rückgabetypen bleiben erhalten.

`share/mod.rs` registriert ausschließlich das eigene neue Modul.
`service.rs`, der Daemon, Wire-DTOs, Capability-Decoder und beide ursprünglichen
Legacy-Admissiongates bleiben unverändert. Kein weiterer Reexport,
Konstruktoranschluss oder Parent-API-Adapter erforderlich.

## Konkrete Abnahmesignale derselben Remote-Suite

Der vorhandene Einstieg `native/test-share-mixed-version-e2e.sh` bleibt
innerhalb der einzigen Root-eigenen RV1-Suite. Exakte Stages/Artefakte:

- `NEW to OLD: prove an explicit probe cannot activate a pending legacy grant`:
  echte Verweigerung vor der Altannahme; Timeout/Kill bleibt ein Fehler.
  `new-to-old-after-pending-probe.json` bleibt Pending/inactive.
- `NEW to OLD: accept using only values emitted by legacy share status`:
  dieselbe tatsächliche Altannahme mit dem vollständigen vorhandenen
  Fingerprintvertrag, ohne direkte Profil- oder Grantmutation des Einstiegs.
- `NEW to OLD: explicitly probe the pinned peer after legacy acceptance`:
  erfolgreicher `se ls`-Open, Ausgabe `new-to-old-explicit-probe.txt`.
- `new-to-old-accepted-lifecycle.json`: unveränderte vollständige Query,
  unter anderem `decision.state=pending`, `effective_state=accepted`,
  `evidence=legacy_relation`, Receipt unconfirmed und aktive Autorisierung
  mit `basis=legacy_contact_projection`.
- `NEW to OLD: prove authorized filesystem access via CLI-discovered target`:
  tatsächliches Root-Listing und Directory-Stat am CLI-entdeckten Export;
  `new-to-old-root-listing.txt` und `new-to-old-remote-stat.txt`.
- `NEW to OLD: keep capability RPCs out of the admitted legacy filesystem path`:
  vorhandenes nichtleeres `old-target/data/smart_explorer/sync/daemon.log`,
  darin tatsächliche `Iroh-Session akzeptiert:` und keine
  `unknown variant capabilities`-Meldung. Das Fehlertextmatching ist nur
  eine Abnahmeassertion; Produktcode verwendet es nie als Altersbeweis.

Sämtliche bestehenden OLD→NEW-/Reject-/RAM-Verlust-/Restart-/Retry-/Widerrufs-
und modernen Sicherheitsassertions bleiben bestehen. Kein zusätzlicher
Suiteeintritt oder unabhängiges Testprogramm.

## Statischer Self-Review und Restgrenze

Statisch abgeglichen: der Legacy-Proof hat ausschließlich den einen
Probeaufruf gewechselt; sein Gate-/Persistenzcode blieb erhalten.
Die drei nachgereichten Consumer haben ausschließlich den freigegebenen
Capability-/Fallbackzweig erhalten. Stream-/Decoder-/Mutationspfade und
die Backend-Traitmethoden bleiben unverändert. Modulregistrierung und
Mixed-Version-Signal sind ausschließlich additiv; nach Entfernen des Signals
ist der gesamte Einstieg bytegleich, einschließlich aller bisherigen Assertions.

Lexikalische Delimiter-/Whitespace- und Scopechecks durchgeführt.
`bash -n` hat nur den Einstieg geparst. Alle bearbeiteten Rustdateien liegen
unter 500 Zeilen und 50 KiB; das neue Modul hat 190 Zeilen/7.798 Bytes,
`backend.rs` nach Extraktion 435 Zeilen. Die engen Consumerergänzungen
erhöhen bestehende Dateien nur um drei oder vier Zeilen.

Keine offenen API-/Consumerabhängigkeiten innerhalb des freigegebenen Blocks.
Die Remote-Suite muss den neuen echten Open-/Lognachweis sowie alle bisherigen
Assertions auf dem integrierten Kandidaten bestätigen. Compiler, Formatter,
Rootgraph, Commit/Push und CI gehören Root. Keine lokalen Builds, Compiler,
Tests, Formatter, Server, Installationen, Git-, CI-, Graph- oder Releaseaktionen;
keine Unteragenten. Die Git-Dokumentationskontextprüfung verbleibt wegen
des ausdrücklich untersagten Worker-Gitzugriffs ebenfalls bei Root.

## Exaktes Dateiinventar

Bei nachgereichten Read-Grenzen wurden nur die Capability-/Fallbackdefinitionen
gelesen und geändert. Keine anderen Verzeichnisse erkundet; keine Token-,
Credential- oder Lockdateien gelesen oder geändert.

### Gelesen

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-3-s-revoke.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-closure-fixes.md`
- `docs/refs/share-server-tls-auth.md`
- `docs/refs/rv1-remote-suite.md`
- `/tmp/rv1-ci-third/linux/mixed-version.log`
- `native/src/share/core/backend.rs`
- `native/src/share/core/peer_endpoint_source.rs`
- `native/src/share/core/wire.rs`
- `native/src/daemon/os/shared/ipc_host.rs`
- `native/src/share/core/service.rs`
- `native/src/share/core/legacy_probe.rs`
- `native/src/share/core/fs_capabilities.rs`
- `native/src/vfs/core/capabilities.rs`
- `native/src/share/core/node_sessions.rs`
- `native/test-share-mixed-version-e2e.sh`
- `native/src/share/core/types.rs`
- `native/src/share/core/direct_ledger.rs`
- `native/src/share/mod.rs`
- `native/src/share/core/peer_list_batch.rs`
- `native/src/share/core/peer_transfer.rs`
- `native/src/daemon/os/shared/ipc_host_mount_probe.rs`
- `native/src/share/core/mount_lease_client.rs`
- `native/src/share/core/peer_request.rs`
- `native/src/share/core/peer_stream.rs`
- `native/src/share/core/wire_capabilities.rs`
- `native/src/share/os/shared/legacy_probe_persist.rs`
- `native/src/share/core/backend_capabilities.rs`
- `native/src/share/core/peer_storage_snapshot.rs`
- `native/src/share/core/peer_storage_analysis.rs`
- `native/src/share/core/peer_extensions.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-3-S-REVOKE.md`

### Geändert

- `native/src/share/core/backend.rs`
- `native/src/share/core/peer_endpoint_source.rs`
- `native/src/share/core/peer_stream.rs`
- `native/src/share/core/peer_transfer.rs`
- `native/src/share/core/legacy_probe.rs`
- `native/src/share/mod.rs`
- `native/test-share-mixed-version-e2e.sh`
- `native/src/share/core/peer_storage_snapshot.rs`
- `native/src/share/core/peer_storage_analysis.rs`
- `native/src/share/core/peer_extensions.rs`

### Erstellt

- `native/src/share/core/backend_capabilities.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-3-S-REVOKE.md`
