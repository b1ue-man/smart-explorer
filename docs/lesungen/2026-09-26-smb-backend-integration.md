# SMB-Backend: Integrationspunkte im Code

**Zweck.** Jede Stelle im vorhandenen Code auflisten, an der ein neues
Protokoll/Scheme eingetragen werden muss (mit dem, was dort für
SFTP/FTP/WebDAV bereits existiert), wie Zugangsdaten gespeichert/aufgelöst
werden, wie aus einer gespeicherten Verbindung ein Backend entsteht, ein
Endpoint-String-Format-Vorschlag (`smb://user@host:port/share/path`,
konsistent zu den bestehenden Formaten), wie das SFTP-Backend Async↔Sync
brückt (Vorbild für SMB), und welche `Backend`-Methoden ein Backend mit
vollen Staged-Write-Garantien implementiert.

## Dateien gelesen

- `native/src/vfs/core/scheme.rs`
- `native/src/vfs/core/core.rs` (nur `Backend`-Trait-Signaturen)
- `native/src/vfs/core/dispatch.rs` — Hinweis: der Auftrag nannte
  `native/src/vfs/os/shared/dispatch.rs`; diese Datei existiert nicht. Die
  tatsächliche Dispatch-Datei liegt unter `vfs/core/dispatch.rs` (per `find`
  verifiziert, `vfs/os/shared/` enthält nur `copy_paste_task_tests.rs`,
  `copy_transfer.rs`, `local.rs`, `remote_util.rs`, `sync_roots.rs`).
  Gelesen, weil sie inhaltlich exakt der im Auftrag beschriebene
  Scheme-Dispatch-Ort ist.
- `native/src/connect/core/location.rs`
- `native/src/connect/core/endpoint.rs`
- `native/src/connect/core/types.rs`
- `native/src/connect/os/shared/resolution.rs`
- `native/src/connect/os/shared/connector.rs`
- `native/src/connect/os/shared/persistence.rs`
- `native/src/sftp/mod.rs`
- `native/src/sftp/core/backend.rs`
- `native/src/sftp/core/connection.rs`
- `native/src/sftp/core/url.rs`
- `native/src/ftp/core/ftp.rs` (nur Struktur/Signaturen, Zeilen 1–130)
- `native/Cargo.toml` (nur `[dependencies]`/`[target.*.dependencies]`)
- `native/src/lib.rs` (nur Modulliste)
- `native/src/mobile/os/shared/domains/connections.rs`
- `native/src/mobile/os/shared/pool.rs`
- `android/app/src/main/java/app/smartexplorer/android/ui/connections/ConnectionDraft.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/connections/ConnectionForm.kt`
- Grep `native/src/app` nach dem Protokoll-Enum des Connect-Dialogs →
  Treffer in `native/src/app/core/dialogs.rs` (Zeilen 173–277); diese Datei
  gelesen (nur die zitierten Ausschnitte, per grep-Kontext).

## A — Jede Stelle, an der ein neues Protokoll/Scheme eingetragen werden muss

| # | Datei:Zeile | Was dort für SFTP/FTP/WebDAV existiert | Was für SMB eingetragen werden muss |
|---|---|---|---|
| 1 | `native/src/vfs/core/scheme.rs:3-10` | `pub enum Scheme { Local, Sftp, Ftp, Webdav, GDrive, Peer }` | Variante `Smb` hinzufügen |
| 2 | `native/src/vfs/core/dispatch.rs:10-19` (`backend_for`) | `if lower.starts_with("sftp://") { … } else if lower.starts_with("ftp://") \|\| lower.starts_with("ftps://") { … } else { LocalBackend }` — **WebDAV fehlt hier bewusst nicht als Bug, sondern strukturell**: `backend_from_url` gibt es für WebDAV nicht, WebDAV läuft ausschließlich über den gespeicherten-Verbindung-Pfad (Connect-Dialog/Persistenz), nie über eine reine Root-String-Erkennung | Zweig `else if lower.starts_with("smb://") { Ok(Arc::new(crate::smb::backend_from_url(r)?)) }` — oder, falls SMB wie WebDAV nur über gespeicherte Verbindungen laufen soll, bewusst **nicht** hier eintragen (Konsistenzentscheidung nötig) |
| 3 | `native/src/vfs/core/dispatch.rs:25-28` (`is_remote_root`) | `lower.starts_with("sftp://") \|\| lower.starts_with("ftp://") \|\| lower.starts_with("ftps://")` (auch hier kein `webdav://`) | Analog zu #2: `smb://`-Präfix ergänzen, wenn #2 ergänzt wird |
| 4 | `native/src/connect/core/location.rs:43-48` (`EndpointSpec::parse`) | `"sftp" \| "ftp" \| "ftps" \| "webdav" => { … Ok(Self::Saved(format!("{scheme}://{rest}"))) }` | `"smb"` in dieses Match-Arm aufnehmen |
| 5 | `native/src/connect/core/location.rs:77-98` (`saved_location`) | generisch über `connection.protocol.is_url()` — kein Protokoll-spezifischer Code | Keine Änderung nötig, **sofern** `Protocol::Smb.is_url() == true` (siehe unten, Datei außerhalb des Leseauftrags) |
| 6 | `native/src/connect/core/location.rs:159-176` (`parse_host_port`) | generisch, parametrisiert über `protocol.default_port()` | Keine Änderung nötig, sofern `Protocol::Smb::default_port()` existiert |
| 7 | `native/src/connect/core/endpoint.rs:31-43` (`ep_prefix`) | generisch über `form.protocol.is_url()` | Keine Änderung nötig |
| 8 | `native/src/connect/core/endpoint.rs:58-65` (`is_remote_url`) | feste Liste `["sftp://","ftp://","ftps://","webdav://","gdrive://","share://"]` | `"smb://"` in die Liste aufnehmen |
| 9 | `native/src/connect/core/endpoint.rs:78-100` (`parse_remote_url`) | generisch über `Protocol::parse`/`proto.is_url()` | Keine Änderung nötig |
| 10 | `native/src/connect/os/shared/connector.rs:104-109` (`do_connect_with_agent_fallback`, Haupt-Match) | `match form.protocol { Protocol::Sftp => connect_sftp(...), Protocol::Ftp \| Protocol::Ftps => connect_ftp(...), Protocol::Webdav => connect_webdav(...), Protocol::Share => connect_share(...) }` | Arm `Protocol::Smb => connect_smb(form, secret, port)` ergänzen + Funktion `connect_smb` (Vorbild: `connect_webdav`, Zeilen 239-275 — kein Agent-Konzept, ein `*Config`-Struct, `*Backend::connect`, `persist`, `RemoteState` bauen) |
| 11 | `native/src/connect/os/shared/persistence.rs:14-17` (`build_saved`, `root`-Zweig) | `match form.protocol { Protocol::Share => form.unc…, _ => norm_root(&form.root) }` | Keine Änderung nötig — der `_`-Zweig deckt `Smb` automatisch ab |
| 12 | `native/src/mobile/os/shared/domains/connections.rs:80-83` (`parse_input`, Protokoll-Filter) | `opt_str(input,"protocol").and_then(Protocol::parse)` mit `protocol.is_url()`-Gate | Keine Änderung nötig, sofern `Protocol::parse("smb")` funktioniert |
| 13 | `native/src/mobile/os/shared/domains/connections.rs:33-34` (`connection_json`, `"https"`-Feld) | `"https": connection.protocol == Protocol::Webdav` (SMB hat kein HTTPS-Konzept) | Keine Änderung nötig — bleibt `false` für `Smb` |
| 14 | `native/src/mobile/os/shared/pool.rs:106-114` (`BackendPool::plan`, `LocKind`-Match) | `LocKind::Sftp \| LocKind::Ftp \| LocKind::Ftps \| LocKind::Webdav => { saved_and_path(...) → Plan::Saved }` | `LocKind::Smb` in dieses Arm aufnehmen — **Achtung**: `LocKind` ist in `native/src/mobile/os/shared/location.rs` definiert, das **nicht** im Leseauftrag stand (offene Abhängigkeit, s. u.) |
| 15 | `native/src/app/core/dialogs.rs:173-185` (Connect-Dialog, Protokoll-Anzeigenamen) | `Protocol::Sftp => "SFTP"`, `Protocol::Ftp => "FTP"`, `Protocol::Ftps => "FTPS"`, `Protocol::Webdav => "WebDAV (HTTPS)"`, `Protocol::Share => "Netzlaufwerk (UNC)"` (zwei Stellen: Zeilen 173-177 Anzeigename, 181-185 Auswahlliste) | `Protocol::Smb => "SMB"` an beiden Stellen ergänzen |
| 16 | `native/src/app/core/dialogs.rs:189, 249, 259, 277` (Feld-Sichtbarkeit im Dialog) | `if p != Protocol::Share && f.port…` (Port-Feld), `if f.protocol == Protocol::Sftp` (Key/Agent-Felder), `else if f.protocol != Protocol::Share` (Root-Feld) | Diese Bedingungen sind bereits generisch genug (`!= Share`, `== Sftp`) — für SMB nur nötig, falls SMB eigene Felder braucht (z. B. Domain) |
| 17 | `android/.../ConnectionDraft.kt:73-77` (`defaultPort`) | `"sftp" -> 22; "ftp","ftps" -> 21; else -> if(https) 443 else 80` | `"smb" -> 445` Zweig ergänzen |
| 18 | `android/.../ConnectionDraft.kt:95-101` (`protocolLabel`) | `"sftp"->"SFTP"; "ftp"->"FTP"; "ftps"->"FTPS"; "webdav"->"WebDAV"` | `"smb"->"SMB"` ergänzen |
| 19 | `android/.../ConnectionForm.kt:70` (`ChoiceRow`-Optionsliste) | `options = ConnApi.PROTOCOLS.map { … }` | `ConnApi.PROTOCOLS` (Kotlin, `api`-Paket, **nicht** im Leseauftrag) muss `"smb"` enthalten — offene Abhängigkeit |

**Nicht im Leseauftrag, aber an praktisch jeder Stelle oben referenziert und
daher zwingend mitzuändern (Kernfrage: Datei nicht identifiziert):**
`crate::creds::Protocol` (Enum mit `Sftp, Ftp, Ftps, Webdav, Share` +
Methoden `as_str()`, `default_port()`, `is_url()`, `parse(&str)`) — lebt in
einem `creds`-Modul, das laut `native/src/lib.rs:21` (`pub mod creds;`)
existiert, dessen Dateien aber außerhalb des zugewiesenen Lesebereichs
liegen. Eine `Protocol::Smb`-Variante dort ist Voraussetzung für praktisch
alle Zeilen 5, 6, 7, 9, 11, 12 oben.

## B — Zugangsdaten: Speicherung und Auflösung

- Metadaten (ohne Secret) werden als `crate::creds::SavedConnection { protocol,
  host, port, user, auth: AuthKind, root, label, use_agent }` gehalten.
  `AuthKind::{Password, Key { path }}` — für SMB reicht `Password` (kein
  Key-Login im SMB2-Protokoll über `smb2::ClientConfig`, siehe
  `docs/refs/smb2.md`).
- Das eigentliche Secret (Passwort/Passphrase) geht getrennt in einen
  Plattform-Store: `native/src/connect/os/shared/persistence.rs:35`
  (`crate::creds::save_connection_with_secret(&saved, secret)`), gelesen mit
  `crate::creds::get_secret_checked(&c.account())`
  (`native/src/connect/os/shared/connector.rs:341,360`), entfernt mit
  `crate::creds::delete_secret_checked`/`remove_connection`
  (`native/src/mobile/os/shared/domains/connections.rs:226,232`). Schlüssel
  ist `SavedConnection::account()` (Methode auf dem Typ, nicht im
  Leseauftrag, aber an jeder o. g. Aufrufstelle als Schlüssel benutzt).
- Backend des Secret-Stores selbst laut Kommentar in `native/Cargo.toml`
  (`[target.'cfg(windows)'.dependencies]`, `keyring`-Zeile): „Windows
  secrets use Credential Manager. Linux deliberately uses the app's
  owner-protected, headless file store instead of a DBus/session keyring.“
  → für SMB keine neue Speichertechnik nötig, nur ein weiterer `Protocol`-Wert
  im selben Mechanismus.
- Passwort vs. Passphrase werden nie gemischt: `same_secret_kind`
  (`native/src/mobile/os/shared/domains/connections.rs:153-158`) verhindert,
  dass ein gespeichertes Passwort als Passphrase (oder umgekehrt)
  wiederverwendet wird, wenn sich die `AuthKind`-Art beim Speichern ändert.

## C — Wie aus einer gespeicherten Verbindung ein Backend entsteht

Zwei parallele Aufrufwege, beide enden in derselben
`crate::connect::open_saved_at`-Familie:

1. **Desktop-Reopen** (Sync-Endpoint, Favoriten):
   `native/src/connect/os/shared/resolution.rs:27-33` (`resolve_endpoint`,
   `EndpointSpec::Saved`-Zweig) → `saved_location` (Match aus Host/User/Port
   + `paths_overlap_one_way`-Scoring gegen den gespeicherten `root`, gibt die
   spezifischste passende `SavedConnection` zurück) →
   `connector::open_saved_at(connection, path)`
   (`native/src/connect/os/shared/connector.rs:318-323`).
2. **Android/Mobile-API** (Datei-Browsing):
   `native/src/mobile/os/shared/pool.rs:106-114` (`BackendPool::plan`,
   `LocKind::Sftp|Ftp|Ftps|Webdav`-Arm) →
   `crate::connect::saved_and_path(&location)` (dasselbe
   `saved_location` wie oben, über `connect/core/endpoint.rs:47-51`) →
   `pool.rs:146-175` (`open`, `Plan::Saved`-Zweig) ruft ebenfalls
   `crate::connect::open_saved_at`.

`open_saved_at_with_agent_fallback`
(`native/src/connect/os/shared/connector.rs:334-377`) ist die eigentliche
Fabrik: liest das Secret (`get_secret_checked`), baut ein `ConnectForm` aus
der `SavedConnection` (`ConnectForm::from_saved`,
`native/src/connect/core/types.rs:70-98`), setzt `form.root`/`form.save =
false` und ruft `do_connect_with_agent_fallback` — denselben Pfad wie ein
frischer Connect-Dialog-Submit (`native/src/connect/os/shared/connector.rs:90-110`).
Für einen URL-Typ (alles außer `Share`) liefert das den `remote.backend`
(`BackendHandle`) direkt zurück (Zeilen 369-375). **Ein SMB-Backend braucht
also nur eine `connect_smb`-Funktion im Stil von `connect_webdav`
(Zeilen 239-275) und einen neuen Match-Arm in `do_connect_with_agent_fallback`
(Zeile 104-109) — der Rest der Fabrik (Persistenz, Fehlerformat,
`RemoteState`-Aufbau) ist bereits protokoll-generisch.**

Der Mobile-Pfad kapselt das Ergebnis zusätzlich in `CachingBackend`
(`native/src/mobile/os/shared/pool.rs:169-173`, `open()`), sofern
`backend.is_local()` falsch ist (Default aus dem `Backend`-Trait) — für SMB
automatisch der Fall, keine Änderung nötig.

## D — Endpoint-String-Format: Vorschlag für SMB

Bestehende Formate (alle über `proto://user@host:port` + `/`-Pfad, gebaut in
`native/src/connect/core/endpoint.rs:104-114` `remote_endpoint` und
gespiegelt in `native/src/sftp/core/backend.rs:32`
`format!("sftp://{}@{}:{}{}", user, host, port, root)`):

| Protokoll | Format | Beispiel |
|---|---|---|
| SFTP | `sftp://user@host:port/root` | `sftp://bob@nas.local:22/home/bob` |
| FTP/FTPS | `ftp(s)://user[:pass]@host:port/root` (Userinfo Percent-encoded via `enc()`, `endpoint.rs:16-27`) | `ftp://alice@ftp.example:21/pub` |
| WebDAV | `webdav://user@host:port/root` (immer HTTPS, `https` ist ein reines Struct-Feld, nicht im URL-String) | `webdav://alice@cloud.example:443/dav` |
| GDrive | `gdrive:///path` (kein Host/User — Token liegt separat) | `gdrive:///Projekte` |
| Share/Peer | `share://…` (eigenes Format, außerhalb dieses Leseauftrags) | — |

**Vorschlag SMB:** `smb://user@host:port/share/path`, exakt dasselbe
`proto://user@host:port` + `/`-Pfad-Schema wie SFTP/FTP/WebDAV. Der
Unterschied zu den anderen Protokollen: SMB hat einen zweistufigen
Namensraum (zuerst der Share-Name über `SmbClient::connect_share`, dann ein
Pfad *innerhalb* dieses Shares über `Tree::list_directory`/`stat`/…, siehe
`docs/refs/smb2.md`). Der gespeicherte `root`-String (`SavedConnection.root`,
z. B. `/Freigabe/Unterordner`) müsste dafür beim Backend-Aufbau am ersten
`/`-Segment gesplittet werden: erstes Segment = Share-Name (Argument zu
`connect_share`), Rest = In-Share-Pfad (Argument zu
`list_directory`/`stat`/…). Das ist konsistent mit der bestehenden
`norm_root`-Konvention (`native/src/connect/core/endpoint.rs:4-13`: leerer
Pfad → `"/"`, sonst führendes `/` garantiert), erfordert aber eine
SMB-eigene Zerlegungsfunktion (`share_name, in_share_path) =
split_first_segment(root)`), die es in den gelesenen Dateien noch nicht gibt
— jedes andere Protokoll hat nur eine Namensraum-Ebene und braucht das nicht.
Default-Port: 445 (Standard-SMB2, siehe `ConnectionDraft.kt`-Tabelle #17).

## E — SFTP-Backend als Async↔Sync-Brücken-Vorbild für SMB

Aus `native/src/sftp/mod.rs:1-13` (Moduldoc) und
`native/src/sftp/core/{backend,connection}.rs`:

1. **Eigene Runtime pro Backend-Instanz.**
   `SftpConnection::connect` (`connection.rs:48-62`) baut
   `tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()`
   und hält sie als `Arc<Runtime>` — nicht die Prozess-Default-Runtime.
   Grund (Moduldoc, `mod.rs:7-11`): ein Worker-Thread treibt `russh`s
   Hintergrund-Verbindungstask kontinuierlich, damit die synchrone
   `Backend`-Brücke (`rt.block_on(...)` je Aufruf) robust bleibt.
2. **`&self`-Methoden, `rt.block_on` pro Aufruf.** Jede `Backend`-Methode in
   `backend.rs` (z. B. `list_dir` Zeile 223-234, `open_read` 248-261) ruft
   synchron `self.rt.block_on(async { … })` bzw. geht über
   `self.connection.safe_metadata(...)` (das intern selbst `block_on`
   nutzt, `connection.rs:78`). Kein `&mut self` nötig — `SftpBackend` ist
   `#[derive(Clone)]` (Zeile 20) und hält nur `Arc`s.
3. **Reconnect-Gate statt Direkt-Retry.** `SftpConnection.reconnect:
   ReconnectGate<SftpTransport>` (`connection.rs:44`,
   `#[path="reconnect_gate.rs"]`, Datei selbst nicht im Leseauftrag).
   `current()`/`current_with_deadline` (`connection.rs:100-126`) fragen
   `self.reconnect.acquire(deadline, |generation| !generation.is_stale() &&
   !generation.value().session.is_closed())`: Ergebnis ist entweder
   `ReconnectAccess::Current(Arc<Generation<T>>)` oder
   `ReconnectAccess::Reconnect(handle)` — im zweiten Fall baut **genau ein**
   Aufrufer die neue Transport-Instanz (`connect_transport`, Zeilen
   165-171), alle anderen warten auf dessen Ergebnis statt eigene parallele
   Reconnects zu starten.
4. **Fehlerklassifikation statt blindem Retry.**
   `classify_sftp_error`/`classify_io_error` (`connection.rs:267-311`) geben
   ein `FailureDisposition { retire: bool, retry_safe: bool }` zurück —
   `dead()` (beides `true`, z. B. `SftpError::IO`, Verbindungsabbruch-Strings
   wie „sender dropped“/„broken pipe“), `suspect()` (`retire=true,
   retry_safe=false`, z. B. Timeout/UnexpectedPacket — Generation wird
   verworfen, aber **ohne** automatischen Replay, weil ein Timeout nicht
   beweist, dass der Request nie angekommen ist), `healthy()` (kein Retire,
   z. B. reine Protokollstatus-Fehler wie „Datei nicht gefunden“).
   `safe_metadata` (Zeilen 71-95) nutzt das für genau **einen** Replay-Versuch
   bei `retry_safe`.
5. **Deadlines absolut, nicht relativ.** `AbsoluteDeadline`
   (`reconnect_gate.rs`, referenziert `connection.rs:5`) wird einmal berechnet
   und durch die ganze Reconnect-plus-Retry-Kette gereicht, statt bei jedem
   Retry-Versuch neu `Duration::from_secs(N)` zu starten (verhindert, dass
   ein Retry-Loop insgesamt länger laufen kann als das nominale Timeout).

**Für SMB direkt übertragbar:** `smb2::SmbClient` bringt mit
`auto_reconnect`/`ReconnectPolicy`/`reconnect()` bereits einen Teil davon
protokoll-nativ mit (siehe `docs/refs/smb2.md`, Abschnitt „disconnect /
reconnect“) — inklusive Serialisierung über eine eigene interne Sperre und
Session-weitem `Connection`-Clone-Zustand. Was **fehlt**, ist genau das
SFTP-Muster aus Punkt 4: `smb2`s eigener Auto-Reconnect wiederholt nur
`list_directory`/`read_file*`/`stat`/`fs_info` automatisch und lässt
`delete`/`rename`/`create`-Fehler unverändert durch — ein SMB-Backend, das
dieselbe „ein Replay-Versuch nur bei bewiesen sicheren Fällen“-Disziplin wie
SFTP auch für seine eigenen zusammengesetzten Operationen (z. B. den
selbstgebauten atomaren Replace-Rename aus `docs/refs/smb2.md`) will, müsste
— wie `SftpConnection` es tut — seine eigene dünne Klassifikationsschicht um
`smb2::Error::kind()`/`is_retryable()` legen, weil `smb2` selbst keine
Rückmeldung gibt, *welche* Fehlerarten sein Auto-Reconnect als "safe to
replay" einstuft.

## F — `Backend`-Methoden für "volle Staged-Write-Garantien"

Aus dem Trait selbst (`native/src/vfs/core/core.rs`):

- Default `staged_write_capabilities` (Zeilen 236-242):
  `StagedWriteCapabilities { create: false, replace: self.rename_overwrites(),
  namespace_replace: self.rename_overwrites() }`. „Voll“ = alle drei `true`.
- `create: true` verlangt ein **Override von `open_write_new`**
  (Default Zeilen 131-137 gibt `io::ErrorKind::Unsupported` zurück — „backend
  has no atomic exclusive-create writer“). Für SMB: der crate-eigene
  `create_file_writer_exclusive`/`open_file_writer_exclusive` (siehe
  `docs/refs/smb2.md`, Abschnitt „Streaming exklusiver Create-Writer“) trägt
  genau diese Garantie (CREATE mit `FileCreate`-Disposition, Fehler bei
  bereits existierendem Namen statt Truncate).
- `replace`/`namespace_replace: true` verlangt entweder
  `rename_overwrites() -> true` (Default `false`, Doku Zeilen 222-231:
  „Local filesystems override this to true“), **oder** ein direktes Override
  von `staged_write_capabilities` selbst. Das SFTP-Backend
  (`sftp/core/backend.rs:378-384`) demonstriert den Mittelweg: es lässt
  `rename_overwrites()` bewusst auf `false` (Kommentar dort: „the extension
  is per server“) und meldet dieselbe `false`/`false` in
  `staged_write_capabilities`, **obwohl** es `promote_staged`
  (Zeilen 315-321) über die separate `posix-rename@openssh.com`-Extension
  atomar überschreibt — es behauptet die Fähigkeit nur dann, wenn sie
  protokollgarantiert ist, nicht bloß serverabhängig verfügbar.
- **Für SMB ist die Lage anders als bei SFTP**: das atomare
  `ReplaceIfExists=1`-Rename (`docs/refs/smb2.md`, Abschnitt „Rename“) ist
  ein Feld im SMB2-SET_INFO-Wire-Format selbst (MS-FSCC 2.4.34.2), keine
  serverabhängige Erweiterung wie `posix-rename@openssh.com`. Ein
  SMB-Backend, das sein eigenes `rename()` mit `ReplaceIfExists=1` statt der
  crate-eigenen, hart auf `false` codierten `Tree::rename` implementiert,
  kann `rename_overwrites() -> true` daher ehrlich zurückgeben; die
  Trait-Defaults für `promote_staged`/`promote_staged_no_replace`/
  `staged_write_capabilities` greifen dann automatisch korrekt (derselbe Weg,
  den vermutlich `LocalBackend` nutzt — nicht Teil dieses Leseauftrags,
  aber laut Trait-Doku der Referenzfall für `rename_overwrites() == true`).
- Weitere im Trait zwingend zu implementierende (nicht default-fähige)
  Methoden für jedes Backend, SMB eingeschlossen: `scheme`, `root_display`,
  `list_dir`, `stat`, `open_read`, `open_write`, `rename`, `remove_file`,
  `remove_dir`, `mkdir_all` (Zeilen 78, 81, 97-98, 124-125, 176, 205-207).

## Offene Fragen / außerhalb dieses Leseauftrags

1. `crate::creds::Protocol` (Enum + `as_str`/`default_port`/`is_url`/`parse`)
   — Datei nicht identifiziert, Voraussetzung für praktisch jeden Punkt in
   Abschnitt A.
2. `native/src/mobile/os/shared/location.rs` (`LocKind`-Enum) — nicht
   gelesen, referenziert in `pool.rs:106`, braucht eine `Smb`-Variante.
3. `android/…/api/ConnApi.kt` (`ConnApi.PROTOCOLS`) — nicht gelesen,
   referenziert in `ConnectionForm.kt:70`.
4. `native/src/vfs/core/capabilities.rs` (Definition von
   `StagedWriteCapabilities`/`RootConfinement`/`MountPathCapabilities`) —
   nicht gelesen; Feldnamen in Abschnitt F stammen aus den
   Struct-Literalen in `core.rs` selbst, nicht aus der Typdefinition.
5. `native/src/vfs/core/promotion.rs` (`default_promote_staged`/
   `default_promote_staged_no_replace`) — nicht gelesen; genauer Ablauf des
   Trait-Default-Pfads für `promote_staged` daher nicht im Detail verifiziert.
6. Kein `native/src/smb/`-Modul existiert bisher (nicht in
   `native/src/lib.rs`s Modulliste) — ein neues SMB-Backend bräuchte einen
   neuen `pub mod smb;`-Eintrag dort, analog zu `pub mod sftp;`/`pub mod ftp;`
   (`native/src/lib.rs:27,40`).
