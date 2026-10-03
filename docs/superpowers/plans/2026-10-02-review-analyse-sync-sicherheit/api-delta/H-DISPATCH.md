# H-DISPATCH – API-Delta

Stand: 2026-10-03. Reale im Arbeitsbaum angeschlossene APIs; keine Compiler-/Laufzeitabnahme. Genaues Dateiinventar und Befundbezug: [abnahme/H-DISPATCH.md](../abnahme/H-DISPATCH.md). Restgrenzen/Hauptanschlüsse: [anfragen/H-DISPATCH.md](../anfragen/H-DISPATCH.md).

## V5: eingegrenzte Invalidierung und Principal

```rust
ShareIrohNode::invalidate_restrictions(
    &self, restrictions: &RestrictionSet,
) -> io::Result<usize>

PeerPrincipal::device_identity(&self) -> PeerDeviceKey
PeerPrincipal::from_exec(principal: &ExecPrincipal) -> PeerPrincipal

ShareIrohNode::bind_incoming_principal(
    &self, connection: &iroh::endpoint::Connection, principal: PeerPrincipal,
) -> impl Future<Output = io::Result<()>>
```

Die öffentliche Invalidierung trifft Principal-Key/Knoten und Relation gemäß der tatsächlichen `RestrictionSet`-Semantik: Verbindungen, alte Generationen, registrierte Arbeit, Leases und Exec. `usize` zählt eindeutige geschlossene Verbindungen, nicht Worker oder Leases. Leere Reduktionsmenge verändert keine Epoche und trennt nichts. Unbekannte Zuordnung nutzt die globale Restriktion des vorhandenen V5-Vertrags.

`PeerDeviceKey` ist typisiert, Clone/Eq/Hash und enthält nur `public_key + node_id`; Direct-/Room-/device_id-Aliase erzeugen keine neue Gerätequote. Vollständiger `PeerPrincipal` bleibt Retention-/Berechtigungs-Key. Incoming FS wird nach geprüftem Hello gebunden; Exec wird unmittelbar nach erfolgreichem `authorize_client_hello` gebunden, ohne seinen Job-/Heartbeat-/Providerfluss zu ersetzen.

Bei ConfigureProfiles bleiben Exec-Konfigurationsübergang und Rechtebarrieren vor Veröffentlichung des neuen Auth-Snapshots. Sicherheitsmarker werden OR-monoton bzw. per Set-Übernahme erhalten. Nicht parsebare Sitzungs-Nonceformen bekommen keine erfundene Ablaufzeit.

## V2: FsAccess-Hülle und Retention

```rust
FsAccess::authorized(
    self,
    session: Arc<IncomingSession>,
    auth: Arc<Mutex<ShareAuthState>>,
    node: &Arc<ShareIrohNode>,
) -> io::Result<FsAccess>

FsAccess::policy_key(&self) -> io::Result<String>
FsAccess::retained_snapshot(&self) -> io::Result<FsAccess>
FsAccess::is_dynamic(&self) -> bool
FsAccess::check_read(&self) -> io::Result<()>
FsAccess::check_write(&self) -> io::Result<()>
FsAccess::register_cancel(&self, cancel: &Arc<AtomicBool>) -> io::Result<()>
```

Konstruktion liegt am geprüften Stream-Einstieg. `Authorized { access: Box<FsAccess>, authority: Arc<AccessAuthority> }` erhält Dynamic oder Mounted und ihre Schutzregeln. Dynamic-Exportstand und Generation werden unter derselben Auth-Snapshot-Sperre erfasst; ein alter Root wird nicht an eine neue Grant-Generation gebunden.

`policy_key()` prüft aktuell Rechte und koppelt den festgehaltenen Exportstand bzw. die Lease-Arc-Identität an die Rechtegeneration. Der Schlüssel ist kein Logfeld. `retained_snapshot()` fixiert Dynamic-Exporte bzw. hält dieselbe Mounted-Arc und die Autorität; ausschließlich der Transportmarker entfällt. Aktuelle Rechte und Generation bleiben bei Wiederaufnahme/jedem Frame erforderlich. H-ANALYSIS hält das vollständige FsAccess bis Retentionsende, damit die Arc-Adresse nicht früh wiederverwendet wird.

`is_dynamic()` reicht die bestehende Routingart durch die Hülle. `register_cancel()` prüft frisch und registriert den Marker atomar zur Generation; nach bereits eingetretener Restriktion wird der Marker gesetzt und der Anschluss abgewiesen. Gewöhnlicher Stream-Drop setzt seinen Transportmarker. Retained-Aufträge übernehmen diesen Marker nicht, bleiben aber durch expliziten Cancel/Rechteentzug abbrechbar.

`resolve_write()` prüft Relation-Schreiben und den konkreten Export-/Zielguard. Jeder Backend-Reader/Writer sowie Providerstream trägt die aktuelle Autorität. Kein rohes uncached Backend wird freigegeben. Local/UNC nutzt `LocalBackend::new` mit dem tatsächlichen autorisierten physischen Root, ohne Parent-Fallback.

## Capabilities und reale Host-Einstiege

`CapabilityQuery` erhält `may_write: bool` und `access: FsAccess`. Antwortzugang und Stage-/Lease-Schreibfähigkeiten verwenden effektive aktuelle Relationrechte und ExportAccess. Providerauflösung, tatsächliche `TargetLimits`, bestehende Akquisition und Legacy-Felder bleiben erhalten.

Legacy verwendet konkret:

```rust
storage_snapshot::serve_snapshot(
    send: SendStream, root: String, access: FsAccess, principal: PeerPrincipal,
) -> impl Future<Output = io::Result<()>>
```

Die V2-Host-Analyse/Duplikate/Hash/List/Watch-/Mutationseinstiege kommen aus dem realen H-ANALYSIS-Arbeitsbaum. Die OS-Wahrheit zu `remote_trash_v1` bleibt zentral in `FsHostFeatures::host()`.

`blocking::run_for(principal, Class::{Control, Background}, operation, work)` nutzt den fairen Pool des authentifizierten Geräts. `run_authorized` führt Fresh-Accessprüfung im Worker aus; Transfer-`spawn_holding` hält Admission und Wake bis zum wirklichen Workerende. Bestehende Host-Pools und Transfer-Sicherheitsgrenzen bleiben erhalten.

## Typed OS-Grenze für Private-Ancestor und Delete

Auf Linux und Windows additiv an dem vorhandenen `DirectoryHandle`:

```rust
DirectoryHandle::is_within_any(&self, roots: &[PathBuf]) -> io::Result<bool>
DirectoryHandle::open_child_for_delete(&self, name: &OsStr) -> io::Result<DirectoryHandle>
DirectoryHandle::remove_child(&self, name: &OsStr) -> io::Result<()>
DirectoryHandle::remove_empty_child(
    &self, name: &OsStr, expected: DirectoryHandle,
) -> io::Result<()>

crate::share::fs::ensure_local_share_handle_allowed(
    handle: &crate::local_access::DirectoryHandle,
) -> io::Result<()>
```

`is_within_any` prüft physische Verzeichnisidentität/-vorfahren, nicht ein frei neu aufgelöstes Anzeigename-Guess. Linux nutzt FD-Ancestry sowie Mount-Root-relativen Vergleich für Bind-Aliase; Windows nutzt vorhandene Volume-/128bit-FileId-Identität der gehaltenen Pins. Unbekannte notwendige Identität scheitert geschlossen. Der Hook verwendet reale App-/Cache-Roots; der Hauptagent schließt ihn im schnellen lokalen H-ANALYSIS-Consumer an.

`remove_child` löscht eine finale nicht-rekursiv behandelte Komponente einschließlich Linkeintrag. `remove_empty_child` konsumiert den erwarteten geöffneten Directory-Pin. Linux verwendet ausschließlich das finale `unlinkat(parent_fd, name, ...)`; Windows verwendet DELETE-Zugriff des ursprünglichen Pins und `SetFileInformationByHandle`, ohne späteres Path-Reopen. Vor jedem Delete prüft der iterative Share-Walker aktuelle Schreibrechte und Private-Ancestry; Exportroot selbst wird nie gelöscht. Die allgemeinen LocalBackend-Methoden werden dadurch nicht pauschal handlegebunden.

## Negotiate Literalchildren: Wire und Provider

Additive read-only Drahtvarianten:

```rust
FsRequest::SyncChildPath { parent: String, literal_name: String }
FsResponse::ChildPath { path: String }
FsHostFeatures { literal_children_v1: bool, /* vorhandene Felder */ }
```

JSON-Request-Tag ist `sync_child_path`, Antwort-Tag `child_path`. Feature ist serde-default `false` für alte Hosts und in `FsHostFeatures::host()` `true`. Die Anfrage ist in `mutates_filesystem()` lesend und im ausdrücklich freigegebenen Peer-Retry-Read-Match aufgeführt. Logging meldet nur die generische Operation/Antwortart.

```rust
peer_extensions::literal_paths::client(
    backend: &PeerBackend, parent: &str, literal_name: &str,
) -> io::Result<String>
peer_extensions::literal_paths::host(
    access: &FsAccess, parent: &str, literal_name: &str,
) -> io::Result<String>
```

Client fragt das Feature ab. Bei dessen Fehlen gilt die alte Parent/Literal-Join-Semantik. Bei ausgehandeltem Feature gelten alle Fehler ohne Legacy-Rückfall. Host prüft frisch, löst Parent auf, nutzt den realen `vfs::sync_child_path`-Hook und bildet ausschließlich einen einzelnen gültigen Child-Suffix unter demselben VirtualParent zurück. Er löst Child erneut und prüft gleiche Lease/Mount, Namespace und genauen Providerpfad. Gespeicherter Provider-Parent bleibt unverändert; nur der neue Literalname wird providerseitig kodiert. `split_clean`-Syntax wird nicht gelockert.

GuardedBackend delegiert mit aktueller Pfad-/Read-/Write-Grenze:

```rust
BackendExtensions::sync_child_path(&self, parent: &str, literal_name: &str) -> VfsResult<String>
BackendExtensions::replace_staged_reversible(
    &self, staged: &str, destination: &str, retained: &str,
) -> VfsResult<bool>
BackendExtensions::previous_state_identities(&self) -> VfsResult<Vec<String>>
```

Beim Reversible-Hook werden Stage, Destination und Retained geprüft; der freie VFS-Dispatcher validiert den vorab journaled Sibling `.se-replace-<16lowerhex>` im selben Zielparent. Default `false` mutiert nichts, `true` erhält das alte Original. Provider-Identity-Aliase werden unverändert delegiert, sofern aktuell autorisiert; keine Token-/Secret-Daten werden geloggt.

Privatnamenprüfung enthält neben Literalnamen eine komponentenweise einmalige Percent-Auslegung ausschließlich für `Scheme::GDrive`. Sie ändert keinen gespeicherten Locator und keine I/O-Schreibweise: `%2Ese-versions` bezeichnet privat, `%252Ese-versions` bleibt der andere Literalname `%2Ese-versions`. Normale SFTP-/FTP-/WebDAV-/SMB-Literalnamen erhalten keine pauschale Percent-Dekodierung.

## Reale Registrierungen und verbleibende Grenzen

Neue Module sind eigene additive Einträge oder kohäsive Submodule bei den zugewiesenen Dateien. Die gemeinsame `share/mod.rs` registriert nur eigene H-DISPATCH-Module. Endpoint-Transportoptionen verwenden das vorhandene `ca_tls_config()`; Sessions filtern Relay-URLs mit `accepts_relay_url()`. S66-Persist-Gate und B21-Dateimigration bleiben erhalten. S09-ALPN-/Node-Fakten werden vom Hauptagenten anschließend additiv integriert.

Nicht als fertig gehärtet behauptet werden die allgemeine pfadbasierte LocalBackend-TOCTOU-Grenze, Datei-Hardlinks außerhalb privater Verzeichnis-Ancestry oder eine Vorabgrenze der von bestehenden Remote-Providern komplett allokierten `list_dir`-Vec. Exakter Status und Hauptanschlüsse stehen in [anfragen/H-DISPATCH.md](../anfragen/H-DISPATCH.md).
