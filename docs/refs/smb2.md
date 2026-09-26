# smb2 0.26.0

Quelle: unentpackter Crate-Quellcode (`smb2src/smb2-0.26.0`, `src/lib.rs`,
`src/client/{mod,tree,connection,stream,session}.rs`, `src/msg/{create,set_info,close}.rs`,
`src/error.rs`, `src/name.rs`, `Cargo.toml`, `README.md`) und
https://docs.rs/smb2/0.26.0 · Abgerufen: 2026-09-26

Pure-Rust SMB2/3-Client (`#![forbid(unsafe_code)]`), pipelined I/O, MIT OR Apache-2.0.

## Verbindung: `ClientConfig` / `connect` / `SmbClient::connect`

```rust
pub struct ClientConfig {
    pub addr: String,                 // "host:port" — NIE auf ':' splitten wegen IPv6 (crate::client::host_of)
    pub timeout: Duration,             // Default 5s; Whole-Attempt-Budget (Namensauflösung inkl.)
    pub username: String,              // leer oder "Guest" (jede ASCII-Schreibung) = Gast/Anonym
    pub password: String,              // leer für Gast; NICHT verschlüsselt im Speicher
    pub domain: String,
    pub auto_reconnect: bool,          // Default false; siehe „Reconnect“ unten
    pub compression: bool,             // Default true; LZ4, nur wenn es verkleinert
    pub dfs_enabled: bool,             // Default true
    pub dfs_target_overrides: HashMap<String, String>, // Default leer
    pub connect_options: Option<smb2::transport::ConnectOptions>, // Default None
}
impl Default for ClientConfig // Gast, addr="", timeout=5s, compression=true, dfs_enabled=true
```

- **Kein Signing/Encryption-Schalter in `ClientConfig`.** Signing wird beim
  SESSION_SETUP immer mit `SecurityMode::SIGNING_ENABLED` angeboten; ob
  tatsächlich signiert wird, entscheidet der Server (`should_sign` = server
  verlangt es ODER Antwort ist nicht Gast/Null). Verschlüsselung aktiviert sich
  automatisch, wenn die Session es verlangt (`SessionFlags::ENCRYPT_DATA`)
  oder ein Share `SMB2_SHAREFLAG_ENCRYPT_DATA` setzt (`connect_share` prüft
  das je Tree). Cipher-Fallback: AES-128-CCM, wenn der Server im Negotiate
  keinen Encryption-Context sendet.
- `SmbClient::connect(config: ClientConfig) -> Result<Self>` — TCP-Connect +
  NEGOTIATE + SESSION_SETUP (NTLM) in einem Aufruf.
- Kurzform: `smb2::connect(addr: &str, username: &str, password: &str) -> Result<SmbClient>`
  (5 s Timeout, `auto_reconnect: false`, `domain` leer, entspricht obigem Default).
- Kerberos: `Session::setup_kerberos(conn, &KerberosCredentials, server_hostname)` /
  `setup_kerberos_from_ccache(conn, &KerberosCredentials, server_hostname, &CCache)`
  — eigener Pfad, nicht über `ClientConfig`; Re-Export `smb2::{KerberosAuthenticator, KerberosCredentials}`.
- **Fehler:** `Error::Auth` (Login verweigert/Gast-Downgrade erkannt),
  `Error::ConnectFailed { host, attempts }` (jede Adresse fehlgeschlagen),
  `Error::Protocol{status: NEGOTIATE-Ablehnung}`, `Error::InvalidData` (z. B.
  `MaxReadSize`/`MaxWriteSize` < 65536 vom Server).
- **Falle:** Ein Login als `"Guest"` (jede Schreibung) akzeptiert eine
  Gast/Anonym-Antwort als Erfolg; jeder andere Benutzername verweigert eine
  Gast/Anonym-Antwort als `Error::Auth` (Schutz gegen Samba
  `map to guest = bad user`, das ein falsches Passwort so beantwortet).

## `connect_share`

```rust
pub async fn connect_share(&mut self, share_name: &str) -> Result<Tree>
```

- Tree-Connect zum Share; aktiviert Share-Verschlüsselung, wenn nötig.
- **DFS transparent:** Bei `STATUS_BAD_NETWORK_NAME` und
  `SMB2_GLOBAL_CAP_DFS` wird automatisch ein Root-Referral versucht
  (`share_name` kann ein DFS-Namespace `\\domain\namespace` sein). Ein echter
  Tippfehler im Share-Namen bleibt `STATUS_BAD_NETWORK_NAME`
  (`Error::Protocol`); nur `Error::DfsNoReachableTarget`/`DfsTooManyReferrals`
  sind DFS-spezifisch. `Tree.dfs_origin: Option<DfsOrigin>` trägt
  `{requested, target}`, falls die UNC-Anzeige den ursprünglich angefragten
  Namen behalten soll.
- Ein Cluster-Redirect derselben NTSTATUS liefert stattdessen
  `Error::ShareRedirected { share }` (wird nicht als DFS behandelt).
- `Tree` (`#[non_exhaustive]`, `Clone`, `Debug`) trägt u. a. `pub tree_id: TreeId`,
  `pub share_name: String`, `pub server: String`, `pub is_dfs: bool`.

## `list_directory` — `DirectoryEntry`

```rust
pub async fn list_directory(&mut self, tree: &mut Tree, path: &str) -> Result<Vec<DirectoryEntry>>

pub struct DirectoryEntry {
    pub name: String,
    pub size: u64,           // 0 für Verzeichnisse
    pub is_directory: bool,
    pub created: FileTime,   // Windows-FILETIME (100 ns seit 1601), pack::FileTime
    pub modified: FileTime,
}
```

- CREATE (Verzeichnis öffnen) → QUERY_DIRECTORY-Schleife (bis
  `STATUS_NO_MORE_FILES`) → CLOSE. `list_directory_instrumented(conn, path,
  query_buffer_len: Option<u32>) -> Result<(Vec<DirectoryEntry>, ListingTrace)>`
  liefert dieselbe Wire-Sequenz mit Zeitmessung pro Runde.
- **Fehler:** `Error::Protocol` mit `Command::Create` (Pfad fehlt/kein Zugriff)
  oder `Command::QueryDirectory`.

## `stat` — `FileInfo`

```rust
pub async fn stat(&mut self, tree: &mut Tree, path: &str) -> Result<FileInfo>

pub struct FileInfo {
    pub size: u64,
    pub is_directory: bool,
    pub created: FileTime,
    pub modified: FileTime,
    pub accessed: FileTime,
}
```

- Compound CREATE + QUERY_INFO(FileBasicInformation, class 4) +
  QUERY_INFO(FileStandardInformation, class 5) + CLOSE, 1 Round-Trip.
- Batch: `stat_files(&mut self, tree, paths: &[&str]) -> Vec<Result<FileInfo>>`
  — sequenziell, 1 Round-Trip je Pfad, kein DFS-Retry im Batch.
- `resolve(&mut self, tree, path) -> Result<Resolved>` — liefert zusätzlich
  den serverseitigen kanonischen Pfad (Groß/Klein, 8.3-Aliasse aufgelöst) und
  die Dateiidentität; nützlich für Case-/8.3-robuste Vergleiche.
- `try_exists`/`exists` gibt es auf `SmbClient`/`Tree` nicht direkt — Existenz
  wird über `stat` + `ErrorKind::NotFound` geprüft (dieselbe Konvention wie
  `vfs::Backend::try_exists` im eigenen Code).

## Streaming Read

### `open_file_reader` / `FileReader` (positioniert, `pread`-artig)

```rust
pub async fn open_file_reader(&self, tree: &Tree, path: &str) -> Result<stream::FileReader>

impl FileReader {
    pub fn size(&self) -> u64;
    pub fn resolved_path(&self) -> Option<&str>; // None auf SMB2.x/3.0.2 oder Server < Win10/2016
    pub async fn read_at(&self, offset: u64, len: u64) -> Result<Vec<u8>>;
    pub async fn close(self) -> Result<()>;       // konsumiert self; Drop ohne close() leakt das Handle
}
```

- `'static`, hält einen `Arc<Tree>` + geklonte `Connection` — bewegbar über
  Tasks, mehrere `read_at`-Aufrufe pipelinen unabhängig über dieselbe Session.
- `read_at` clamped an EOF (kein `STATUS_END_OF_FILE`-Fehler bei Range über
  Dateiende); größer als `MaxReadSize` wird intern in mehrere READs gesplittet.
- **Falle:** kein `Drop`-Close (kein async Drop) → `close()` explizit rufen,
  sonst leakt das Handle bis Session-Ende (Debug-Log warnt).

### `download` / `FileDownload` (sequenziell, Chunk-für-Chunk, Read-Ahead)

```rust
pub async fn download<'a>(&'a mut self, tree: &'a Tree, path: &str) -> Result<FileDownload<'a>>

impl<'a> FileDownload<'a> {
    pub fn size(&self) -> u64;
    pub fn bytes_received(&self) -> u64;
    pub fn progress(&self) -> Progress;           // { bytes_transferred, total_bytes: Option<u64> }
    pub fn with_read_ahead(self, ReadAhead) -> Self;   // vor erstem next_chunk()
    pub fn with_chunk_size(self, u32) -> Self;
    pub async fn next_chunk(&mut self) -> Option<Result<Vec<u8>>>; // None = fertig; cancel-safe
    pub async fn collect(self) -> Result<Vec<u8>>;
    pub async fn collect_with_progress<F>(self, F) -> Result<Vec<u8>>
        where F: FnMut(Progress) -> ControlFlow<()>; // Break → Error::Cancelled
}
```

- Hält `&mut Connection` exklusiv für die Lebensdauer (kein Interleaving).
  CLOSE geht beim letzten Chunk raus; die letzte `next_chunk()`-Antwort holt
  nur noch dessen Bestätigung ab. Default `ReadAhead::Adaptive` (max. 4 MiB
  gleichzeitig angefordert).
- **Falle:** Drop vor Fertigstellung leakt das Handle (kein async Drop) —
  Debug-Log warnt.

## Streaming exklusiver Create-Writer

```rust
pub async fn create_file_writer_exclusive(&self, tree: &Tree, path: &str) -> Result<stream::FileWriter>
```

- Gegenstück zu `create_file_writer` (Disposition `FileOverwriteIf`): öffnet
  mit `CreateDisposition::FileCreate` — existiert der Name bereits, schlägt
  das Öffnen mit `ErrorKind::AlreadyExists` fehl statt zu überschreiben.
- `create_file_writer_at(tree, path, offset: u64)` öffnet ohne Truncate
  (`FileOpenIf`); erster geschriebener Byte landet bei `offset` (Anhängen
  nach Server-Side-Copy-Präfix o. Ä.).

### `FileWriter`

```rust
impl FileWriter {
    pub fn resolved_path(&self) -> Option<&str>;
    pub fn with_write_behind(self, WriteBehind) -> Self;   // vor erstem write_chunk
    pub fn with_chunk_size(self, u32) -> Self;              // ≤ MaxWriteSize, sonst geclampt
    pub fn bytes_written(&self) -> u64;                     // vom Server bestätigt
    pub fn progress(&self) -> Progress;                     // total_bytes immer None (push-basiert)
    pub async fn write_chunk(&mut self, data: &[u8]) -> Result<()>; // Backpressure über Fenster
    pub async fn finish(self) -> Result<u64>;   // send_pending + drain + FLUSH + CLOSE; konsumiert self
    pub async fn abort(self) -> Result<u64>;    // verwirft ungesendete Daten, drained Inflight, CLOSE
                                                 // OHNE FLUSH (best-effort), gibt Ok(bytes) selbst bei
                                                 // fehlgeschlagenem CLOSE zurück
}
```

- `finish()` propagiert Flush-/Close-Fehler; `abort()` gibt praktisch nie
  `Err` zurück (Result-Signatur nur zwecks API-Symmetrie mit `finish`).
- **Falle:** Drop ohne `finish()`/`abort()` leakt das Handle (Debug-Log).
  Der aufrufende Code muss eine unerwünschte Teildatei nach `abort()` selbst
  löschen (`remove_file`) — der Server hat 0..N geschriebene Bytes stehen.

## write/overwrite-Varianten

| Methode | Disposition | Round-Trips | Einsatz |
|---|---|---|---|
| `Tree::write_file` / `SmbClient::write_file` | `FileOverwriteIf` | wählt automatisch Compound (≤ `conn.compound_write_limit()`) oder Pipelined | genereller Zweck |
| `write_file_compound` | `FileOverwriteIf` | 1 (CREATE+WRITE+FLUSH+CLOSE; Windows verweigert nicht-letztes FLUSH im Compound → dort 2) | kleine Dateien bis `MaxWriteSize`-Grenze |
| `write_file_compound_exclusive` | `FileCreate` | wie oben | „nur neu anlegen“, Refusal = `Error::Protocol{Command::Create}` mit `ErrorKind::AlreadyExists`, Datei am Ziel bleibt unberührt |
| `write_file_pipelined` | `FileOverwriteIf` | mehrere WRITEs im Sliding-Window | große Dateien |
| `SmbClient::upload` | `FileOverwriteIf` | Compound wenn `data.len() ≤ compound_write_limit()`, sonst `FileUpload` (Pipelined) | „ein API für jede Größe“ |

- Alle Schreibpfade flushen vor dem Schließen (Datensicherheit).

## Rename — hart codiertes `ReplaceIfExists=false` + atomischer Replace-Rename selbst bauen

```rust
pub async fn rename(&mut self, tree: &mut Tree, from: &str, to: &str) -> Result<()>
```

- `Tree::rename` baut intern ein Compound CREATE(`DELETE|FILE_READ_ATTRIBUTES`,
  `FileOpen`) + SET_INFO(`InfoType::File`, `file_info_class=10` =
  `FileRenameInformation`, MS-FSCC 2.4.34.2) + CLOSE. Der private Helper
  `build_rename_info_buffer` schreibt **`ReplaceIfExists = 0` fest** — es gibt
  keinen öffentlichen Parameter dafür. Existiert `to` bereits, schlägt das
  SET_INFO fehl (`Error::Protocol{Command::SetInfo}`, typischerweise
  `ErrorKind::AlreadyExists`), und `rename()` schließt das CREATE-Handle noch
  selbst (Standalone-CLOSE), bevor der Fehler zurückgegeben wird.
- **Falle:** `from` wird mit `self.format_path(from)` normalisiert (bei
  DFS-Shares mit Host\Share-Präfix), `to` dagegen nur mit dem crate-privaten
  `normalize_path` (= `crate::name::encode_path`, **kein** DFS-Präfix). Ein
  eigener Nachbau muss diese Asymmetrie kennen.

### `FileRenameInformation`-Puffer-Layout (MS-FSCC 2.4.34.2)

```
Offset 0      : ReplaceIfExists   u8   (0 = false, 1 = true — hier der Hebel)
Offset 1..8   : Reserved          7 Bytes, 0
Offset 8..16  : RootDirectory     u64 LE (0 = gleiches Verzeichnis)
Offset 16..20 : FileNameLength    u32 LE (Bytelänge von FileName, UTF-16LE)
Offset 20..   : FileName          UTF-16LE, KEIN Null-Terminator
```

### Eigener atomischer Replace-Rename mit den öffentlichen Typen `CompoundOp`/`CreateRequest`/`SetInfoRequest`/`CloseRequest`

Alle beteiligten Typen sind öffentlich (`smb2::client::connection::CompoundOp`,
`smb2::msg::create::{CreateRequest, CreateResponse, CreateDisposition,
ImpersonationLevel, ShareAccess}`, `smb2::msg::set_info::{SetInfoRequest,
InfoType}`, `smb2::msg::close::CloseRequest`, `smb2::types::{FileId, Command,
CreditCharge, flags::FileAccessMask}`). Bauplan (identisch zum internen
`Tree::rename`, nur mit `ReplaceIfExists = 1`):

```rust
use smb2::client::connection::CompoundOp;
use smb2::msg::create::{CreateRequest, CreateResponse, CreateDisposition, ImpersonationLevel, ShareAccess};
use smb2::msg::set_info::{SetInfoRequest, InfoType};
use smb2::msg::close::CloseRequest;
use smb2::types::{Command, CreditCharge, FileId, flags::FileAccessMask};

fn rename_info_buffer_replace(new_name: &str) -> Vec<u8> {
    let name_u16: Vec<u16> = new_name.encode_utf16().collect();
    let mut buf = Vec::with_capacity(20 + name_u16.len() * 2);
    buf.push(1); // ReplaceIfExists = true
    buf.extend_from_slice(&[0u8; 7]);
    buf.extend_from_slice(&0u64.to_le_bytes());
    buf.extend_from_slice(&((name_u16.len() * 2) as u32).to_le_bytes());
    for u in name_u16 { buf.extend_from_slice(&u.to_le_bytes()); }
    buf
}

// from/to bereits mit smb2::encode_path(...) kodiert (siehe „Pfad-Encoding“ unten).
let create_req = CreateRequest {
    requested_oplock_level: smb2::types::OplockLevel::None,
    impersonation_level: ImpersonationLevel::Impersonation,
    desired_access: FileAccessMask::new(FileAccessMask::DELETE | FileAccessMask::FILE_READ_ATTRIBUTES),
    file_attributes: 0,
    share_access: ShareAccess(ShareAccess::FILE_SHARE_READ | ShareAccess::FILE_SHARE_WRITE | ShareAccess::FILE_SHARE_DELETE),
    create_disposition: CreateDisposition::FileOpen,
    create_options: 0,
    name: from_encoded,
    create_contexts: vec![],
};
let set_info_req = SetInfoRequest {
    info_type: InfoType::File,
    file_info_class: 10, // FileRenameInformation
    additional_information: 0,
    file_id: FileId::SENTINEL, // Compound füllt es aus der CREATE-Antwort
    buffer: rename_info_buffer_replace(&to_encoded),
};
let close_req = CloseRequest { flags: 0, file_id: FileId::SENTINEL };

let ops = [
    CompoundOp { command: Command::Create, body: &create_req, tree_id: Some(tree.tree_id), credit_charge: CreditCharge(1) },
    CompoundOp { command: Command::SetInfo, body: &set_info_req, tree_id: Some(tree.tree_id), credit_charge: CreditCharge(1) },
    CompoundOp { command: Command::Close, body: &close_req, tree_id: Some(tree.tree_id), credit_charge: CreditCharge(1) },
];
let responses = client.connection().execute_compound(&ops).await?; // Vec<Result<Frame>>, je 1 pro Op
// responses[i]?.header.status prüfen; bei Fehler nach erfolgreichem CREATE das Handle per
// eigenständigem CLOSE (Tree::close_handle, öffentlich) freigeben — sonst Handle-Leak.
```

- **`tree_id`-Zugriff:** `tree.tree_id` ist ein öffentliches Feld (`pub
  tree_id: TreeId`), direkt lesbar.
- **Pfad-Encoding:** `smb2::encode_path(path: &str) -> String` (Re-Export von
  `crate::name::encode_path`) wandelt `/`-getrennte Pfade in `\`-getrennte
  Wire-Namen und mappt die 8 auf SMB2 illegalen Zeichen (`" * : < > ? \ |`,
  Steuerzeichen, führendes/schließendes Leerzeichen/Punkt je Komponente) in
  den Private-Use-Bereich U+F001–U+F029 (siehe `crate::name`-Moduldoku).
  Für ein DFS-Share fehlt der interne `Tree::format_path`/`host_of`-Helfer
  öffentlich — bei `tree.is_dfs == true` muss der Aufrufer den
  `server\share\`-Präfix selbst voranstellen (Hostname aus dem `host:port`
  ohne einfaches `split(':')`, siehe Falle unten); bei gewöhnlichen Shares
  reicht `encode_path` allein.
- **Falle beim Nachbau:** Schlägt SET_INFO oder CLOSE fehl, NACHDEM CREATE
  erfolgreich war, kaskadiert der Compound-Rest nicht automatisch zu einem
  Server-seitigen Close — das offene Handle muss der Aufrufer selbst per
  `Tree::close_handle(conn, file_id)` (öffentlich, aus der CREATE-Antwort
  extrahiertes `FileId`) schließen, sonst leakt es bis Session-Ende.
- **Host-Herleitung nie mit `split(':')`:** liest `[::1]:445` falsch als
  Klammer und ein IPv6-Literal ohne Klammern falsch am letzten `:`. Der Crate
  hält diese Logik in `crate::client::host_of` (`pub(crate)`, nicht exportiert).

## `delete_file` / `delete_directory` / `create_directory`

```rust
pub async fn delete_file(&mut self, tree: &mut Tree, path: &str) -> Result<()>
pub async fn delete_directory(&mut self, tree: &mut Tree, path: &str) -> Result<()>  // muss leer sein
pub async fn create_directory(&mut self, tree: &mut Tree, path: &str) -> Result<()>
```

- Beide Deletes: Compound CREATE(`DELETE|FILE_READ_ATTRIBUTES`) +
  SET_INFO(`FileDispositionInformation`, class 13, Puffer `[1]` =
  `DeletePending=true`) + CLOSE — **bewusst nicht** `FILE_DELETE_ON_CLOSE` als
  Create-Option: Samba akzeptiert ein solches CREATE gegen ein nicht-leeres
  Verzeichnis, meldet CREATE/CLOSE beide als Erfolg und löscht nichts. Der
  explizite SET_INFO-Weg lässt den Server dagegen mit
  `STATUS_DIRECTORY_NOT_EMPTY` antworten.
- `create_directory`: einzelnes CREATE mit `CreateDisposition::FileCreate` +
  `FILE_DIRECTORY_FILE`, dann sofortiges CLOSE (2 Round-Trips, kein Compound).
- Batch: `delete_files(&mut self, tree, paths: &[&str]) -> Vec<Result<()>>`.

## disconnect / reconnect

```rust
pub async fn disconnect_share(&mut self, tree: &Tree) -> Result<()>   // TREE_DISCONNECT
pub async fn reconnect(&mut self) -> Result<()>                        // erzwungen, auch ohne auto_reconnect
pub fn is_disconnected(&self) -> bool
pub fn on_reconnect(&self, observer: Option<ReconnectObserver>)         // Arc<dyn Fn(ReconnectEvent) + Send + Sync>
pub fn set_reconnect_policy(&self, policy: ReconnectPolicy)
```

- `reconnect()` dialt neu, renegotiiert, re-authentifiziert **in-place** unter
  jedem `Connection`-Clone; jeder vorherige Tree-Connect/Handle wird ungültig
  — der Aufrufer muss `connect_share` je benötigtem Share erneut ausführen.
- `ClientConfig::auto_reconnect: true` armiert einen `SessionReviver`; bei
  Verlust wird nur bei idempotenten Operationen automatisch wiederholt
  (`list_directory`, `read_file*`, `stat`, `fs_info`) — **nicht** bei
  `delete`/`rename`/`create` (könnten bereits gewirkt haben; Fehler wird
  durchgereicht, kein stilles Verschlucken).
- `ReconnectPolicy { max_attempts: u32 (Default 4), initial_backoff: Duration
  (500ms), max_backoff: Duration (8s), total_budget: Duration (60s),
  failure_cooldown: Duration (10s) }`. Fehlschlag → `Error::ReconnectFailed
  { attempts, waited, cause: ErrorKind, reason: String }`.

## Error/ErrorKind-Mapping

`Error` ist `#[non_exhaustive]`, `thiserror`-basiert; `Error::kind() ->
ErrorKind` klassifiziert (`ErrorKind` ebenfalls `#[non_exhaustive]`, `match`
mit `_`-Arm nötig). Relevante Zuordnungen (`classify_status` in `error.rs`):

| NTSTATUS | `ErrorKind` |
|---|---|
| `OBJECT_NAME_NOT_FOUND`, `OBJECT_PATH_NOT_FOUND`, `NO_SUCH_FILE`, `BAD_NETWORK_NAME` | `NotFound` |
| `OBJECT_NAME_COLLISION` | `AlreadyExists` |
| `ACCESS_DENIED` | `AccessDenied` |
| `LOGON_FAILURE`, `ACCOUNT_DISABLED/EXPIRED/LOCKED_OUT`, `PASSWORD_EXPIRED/MUST_CHANGE`, `ACCOUNT_RESTRICTION`, `INVALID_LOGON_HOURS`, `INVALID_WORKSTATION` | `AuthRequired` |
| `Error::Disconnected`, `Error::ServerUnresponsive`, `NETWORK_NAME_DELETED`, `USER_SESSION_DELETED`, `Error::ReconnectFailed`, `Error::DurableHandleLost`, `Error::DfsNoReachableTarget` | `ConnectionLost` |
| `Error::CreditStarvation`, `Error::SendTimeout`, `Error::Timeout` | `TimedOut` (Credit-/Send-Timeout ebenfalls `TimedOut`, nicht eigener Kind) |
| `OBJECT_NAME_INVALID` | `InvalidName` (≠ `NotFound`: Server hat gar nicht gesucht) |
| `SHARING_VIOLATION`, `FILE_LOCK_CONFLICT` | `SharingViolation` |
| `FILE_IS_A_DIRECTORY` / `NOT_A_DIRECTORY` | `IsADirectory` / `NotADirectory` |
| `DISK_FULL` | `DiskFull` |
| `NOT_SUPPORTED`, `INVALID_DEVICE_REQUEST`, `NOT_IMPLEMENTED` | `Unsupported` |
| `PATH_NOT_COVERED` | `DfsReferral` |
| alles andere | `Other` |

- `Error::is_retryable() -> bool`: `Timeout`, `Disconnected`,
  `CreditStarvation`, `SendTimeout`, `ServerUnresponsive`, `ReconnectFailed`,
  `DurableHandleLost`, `DfsNoReachableTarget`, `ConnectFailed`,
  `Protocol{INSUFFICIENT_RESOURCES|INSUFF_SERVER_RESOURCES}` — **nicht**
  `FileTooLargeForSingleRead` (kein Neuversuch sinnvoll ohne Änderung).
- `Error::status() -> Option<NtStatus>` nur für `Error::Protocol`.

## Laufzeit-Anforderungen

- Feature `tokio` (Default an) zieht `tokio/{net,io-util,rt,time}`; Feature
  `smol` als Alternative (`default-features = false, features = ["smol"]`).
  Mit beiden an läuft jeder Call auf tokio, wenn er innerhalb eines
  Tokio-Runtimes ausgeführt wird, sonst auf smol.
- `Connection` ist intern `Arc<Inner>`-basiert und `Clone`; mehrere Klone
  dürfen aus verschiedenen Tasks gleichzeitig `execute`/`execute_compound`
  aufrufen (`&self`, kein `&mut` nötig) — dafür ausgelegt
  (`execute_with_credits`-Doku: „safe to call from multiple tasks on clones
  of the same Connection“). `SmbClient` selbst braucht dagegen für die
  meisten High-Level-Methoden `&mut self` (z. B. `list_directory`, `stat`,
  `rename`) wegen des DFS-/Reconnect-Bridging-Zustands; `Tree`-Methoden
  brauchen nur `&self` + `&mut Connection` (Ausnahme: Streaming-Typen wie
  `FileWriter`/`FileReader`/`FileDownload` halten die Connection selbst).
- Keine expliziten `unsafe impl Send/Sync` nötig — `#![forbid(unsafe_code)]`
  im ganzen Crate; Thread-Sicherheit ergibt sich aus `Arc`+`Mutex`/Channels.

## MSRV & Abhängigkeiten

- `rust-version = "1.85"`, `edition = "2021"`.
- Pure Rust, keine C-FFI/`-sys`-Crates. RustCrypto-Versionen (Cargo.toml):
  `aes >=0.9.1,<0.9.3`, `aes-gcm 0.11.0`, `ccm 0.6.1`, `cmac 0.8.0`,
  `digest 0.11`, `hmac 0.13`, `md-5 0.11`, `md4 0.11`, `sha1 0.11`,
  `sha2 0.11`, `pbkdf2 0.13.0`. Weitere: `thiserror 2`, `log 0.4`,
  `async-trait 0.1`, `num_enum 0.7`, `lz4_flex 0.14` (Kompression),
  `getrandom 0.4`, `futures-util 0.3` (`default-features = false`, Features
  `std`, `async-await`), `ctutils 0.4.2`, `tokio 1` (Basis-Feature `sync`,
  Zusatz-Features nur über das `tokio`-Crate-Feature).
- Optional: `serde 1` (Feature `serde`, Diagnostics-Serialisierung),
  `smol 2.0.2` (Feature `smol`).
