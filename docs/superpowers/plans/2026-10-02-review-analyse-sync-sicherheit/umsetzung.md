# RV1 – Umsetzung

Planbasis: 2026-10-02 (nach damaliger Kritiker-Runde, siehe [review.md](review.md)); Umsetzungsstatus aktualisiert 2026-10-04. Spec: [spec.md](spec.md). Befunde:
[Analyse](review-befunde-analyse.md), [Sync](review-befunde-sync.md), [Sicherheit](review-befunde-sicherheit.md).
Entscheidungen: [recherche.md](recherche.md). Syntax: `docs/refs/` (INDEX).

## Regeln für alle Blöcke

- Dateien gehören genau einem Block (Tabelle „Besitz“); Besitzwechsel nur wie dort vermerkt. Wer eine
  Änderung in einer fremden Datei braucht, schreibt sie nach `anfragen/<block>.md` (Datei, Stelle, Änderung,
  Grund) und arbeitet mit einem lokalen Platzhalter weiter; der Orchestrator leitet weiter.
  `native/Cargo.toml`, `native/Cargo.lock`, `share-server/Cargo.toml`, `share-server/Cargo.lock` ändert nur
  der Orchestrator (Anfrage).
- AGENTS.md gilt: Rust-Dateien < 500 Zeilen (neu oder wesentlich geändert; sonst erst ausgliedern), `core/`
  plattformneutral, OS-Verhalten hinter `os/`, kein `unwrap`/`expect` in Produktivpfaden, Links/Junctions
  bleiben geschützte Auslassungen, Endpunkt-/Verbindungsidentität bleibt erhalten, Rückweg (Versionen,
  Konfliktkopien) für jede Überschreibung/Löschung.
- Keine lokalen Builds/Tests/cargo/gradle/rustfmt, keine Server, keine Installationen, keine Commits/Pushes
  durch Unteragenten, keine weiteren Agenten durch Unteragenten. Lokal nur Text-/Parsing-Prüfungen ohne
  Compiler oder Linker; Formatierung und Kompilierung gehören zur abschließenden Remote-Task-Suite.
- Meilenstein-Tests: Präfix `review_task_`, als Rust-Tests in eigenen oder neuen Testdateien (Kotlin-Unit-
  Tests unter `android/app/src/test`, Gerätetests nur als Beschreibung). Liste mit erwartetem Ergebnis in
  `abnahme/<block>.md`. Tests, die eine Umgebung brauchen (Container, zweites Profil, Windows-Dateisystem),
  sind mit `#[ignore]` + Grund markiert und in `abnahme/<block>.md` als „Suite-Stufe“ beschrieben.
- Kotlin↔Rust-Schnittstellen: Delta nach `api-delta/<block>.md` (Methode, Argumente, Antwort, Fehler);
  AND-SHARE-UI überträgt alle Deltas nach `docs/superpowers/plans/2026-09-25-android-apk/api.md`.
- Eigene Status-Zeile in der Tabelle „Status“ unten pflegen (Stand, Verträge, offene Anfragen).
- Registrierungsdateien gehören niemandem allein: `mod.rs` aller Module (z. B. `share/mod.rs`,
  `bisync/mod.rs`, `analytics/mod.rs`, `vfs/mod.rs`, `daemon/mod.rs`), `native/src/lib.rs`,
  `native/src/mobile/os/shared/dispatch.rs`, `native/src/mobile/os/shared/domains/mod.rs`,
  `native/src/cli/mod.rs`, `native/src/app/core/state.rs`. Jeder Block darf dort ausschließlich eigene
  Einträge hinzufügen (Modul-Zeile, `pub use`, Dispatch-Arm, Feld mit `Default`), nichts Fremdes ändern;
  schlägt ein Edit fehl, weil die Datei inzwischen geändert wurde: neu lesen, eigenen Eintrag erneut setzen.
- Kotlin: neue Datenklassen in die eigene `api/*.kt`-Datei des Blocks, nicht nach `core/**`.

## Verträge

Fundament-Blöcke setzen ihren Vertrag als Erstes um (nur Typen, Signaturen, Standard-Implementierungen und
Stubs, die bestehendes Verhalten nicht ändern), tragen die exakten Signaturen hier unter ihrem Vertrag ein,
setzen ihre Status-Zeile auf „Vertrag fertig“ und beenden ihren Lauf mit dem Bericht „Vertrag fertig“. Der
Orchestrator gibt danach per Nachricht den Rest frei.

### V1 VFS (K1)
- `VfsMeta.special: bool` (FIFO, Socket, Gerät; Windows `FILE_ATTRIBUTE_DEVICE` und AF_UNIX-/LX-Tags). Alle
  Konstruktionen werden zuerst ergänzt; danach Status „V1-VfsMeta fertig“ setzen (erst dann ändern andere
  Blöcke die betroffenen Dateien).
- Zwischendatei mit Größe und Änderungszeit (Zeit beim Hochladen, wo nur so möglich) + „Zeit einer fertigen
  Zwischendatei setzen“ (`Ok(false)` = nicht möglich) + `mtime_precision(root)` → {Nanos, Millis, Seconds,
  TwoSeconds, Minutes, Days, Unknown}.
- `io::ErrorKind::StorageFull`/`ReadOnlyFilesystem`/`QuotaExceeded` für „voll“/„nur lesbar“; Helfer
  `vfs::is_target_refusal(&io::Error)`.
- Optionale Haken (Standard „nicht unterstützt“, weitergereicht von Caching-/Agent-/Unavailable-Backend):
  `find_duplicates`, `hash_walk`, `recycle(path, expected)`, `change_signal(root)` (Abo auf Änderungen der
  Gegenseite: Share `watch_v1`, Nextcloud-ETag, Drive-Feed); Fähigkeitsabfragen dazu.
- Lokal: `syncfs`-Helfer (Linux/Android), sicheres Öffnen fremder Dateien (O_NONBLOCK|O_NOFOLLOW + S_ISREG),
  Dateisystem-UUID/Volume-Seriennummer + Pfad relativ zum Einhängepunkt (`vfs::local_volume_identity`).
- Trait-Datei bleibt < 500 Zeilen (Standard-Implementierungen auslagern).

Signaturen (K1, eingetragen):
- `crate::vfs::VfsMeta` neues Feld `pub special: bool` (nach `is_symlink`; `Default` = `false`). Gesetzt von
  `LocalBackend::{list_dir, stat}` (Linux/Android: weder Datei, Ordner noch Link; Windows:
  `FILE_ATTRIBUTE_DEVICE` oder Reparse-Tag AF_UNIX/`LX_FIFO`/`LX_CHR`/`LX_BLK`) und SFTP (S_IFMT gesetzt,
  weder Ordner, Link noch Datei); alle anderen Backends `false` (FTP/SMB/WebDAV/Drive → V-REMOTE,
  `From<FsMeta>` → K2 mit `FsMeta.special`, Agent `wire_to_vfs` → A-CLIENT). Spezielle Einträge haben
  `is_dir == false`, `is_symlink == false`, `size == 0`.
- Lokale Klassifikation: `crate::local_access::metadata_class(&Path, &fs::Metadata) -> MetadataClass
  { link_like: bool, special: bool }` (Windows mit höchstens einem Reparse-Tag-Lesen); `EntryKind::Other`
  in `local_access::read_directory` heißt jetzt auch unter Windows „speziell“ (AF_UNIX/LX-Tags).

**Erweiterungen statt neuer Trait-Methoden.** Alle übrigen V1-Haken stehen im neuen Trait
`crate::vfs::BackendExtensions: Backend` (`vfs/core/extensions.rs`); `Backend` erhält nur
`fn extensions(&self) -> Option<&dyn BackendExtensions> { None }` (Trait-Datei `core.rs` 497 Zeilen; lange
Standard-Rümpfe nach `core/trait_defaults.rs`). Ein Backend schaltet Erweiterungen frei mit
`impl BackendExtensions for X { … }` + `fn extensions(&self) -> Option<&dyn BackendExtensions> { Some(self) }`
und überschreibt nur, was es kann. **Aufrufer nutzen immer die freien Funktionen `crate::vfs::…`**
(`core/extension_calls.rs`), die ohne Erweiterungen den genannten Rückfall liefern. Hüllen, die Pfade
umschreiben oder Zugriff begrenzen (Export-Wurzel, Mount), reichen `extensions()` des inneren Backends nie
unverändert weiter, sondern setzen eigene Implementierungen mit Übersetzung (oder `None` = nicht unterstützt).
`CachingBackend` reicht alles weiter (Listen/Walks am Cache vorbei, Schreiben/`finish_stage`/`recycle`
invalidieren); `LocalBackend` implementiert die lokalen Teile.

```rust
// Freie Funktionen (B: Backend + ?Sized) – Rückfall ohne Erweiterungen in Klammern
pub fn list_dir_tolerant(b: &B, path: &str) -> VfsResult<VfsListing>;                    // (list_dir, ohne Auslassungen)
pub fn open_read_regular(b: &B, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>>; // (open_read_id)
pub fn open_write_copy_stage_timed(b: &B, path: &str, size: u64, mtime_ms: i64)
    -> VfsResult<Box<dyn Write + Send>>;                                                 // (open_write_copy_stage_sized)
pub fn finish_stage(b: &B, stage: &str, finish: StageFinish) -> VfsResult<StageFinished>; // (StageFinished::default())
pub fn sync_filesystem(b: &B, root: &str) -> VfsResult<bool>;                            // (Ok(false))
pub fn target_limits(b: &B, root: &str) -> TargetLimits;                                 // (TargetLimits::default())
pub fn mtime_precision(b: &B, root: &str) -> MtimePrecision;                             // = target_limits(..).mtime_precision
pub fn unix_mode(b: &B, path: &str) -> VfsResult<Option<u32>>;                           // (Ok(None))
pub fn volume_identity(b: &B, root: &str) -> VfsResult<Option<VolumeIdentity>>;          // (Ok(None) = unbekannt)
pub fn supports_duplicate_search(b: &B, root: &str) -> VfsResult<bool>;                  // (Ok(false))
pub fn find_duplicates(b: &B, root: &str, min_bytes: u64, progress: &crate::analytics::ReclaimProgress)
    -> VfsResult<Option<crate::analytics::DuplicateReport>>;                             // (Ok(None) = nicht unterstützt)
pub fn supports_hash_walk(b: &B, root: &str) -> VfsResult<bool>;                         // (Ok(false))
pub fn hash_walk(b: &B, root: &str, request: HashWalkRequest,
    tx: crossbeam_channel::Sender<HashWalkItem>, cancel: &AtomicBool) -> VfsResult<bool>; // (Ok(false))
pub fn supports_recycle(b: &B, path: &str) -> VfsResult<bool>;                           // (Ok(false))
pub fn recycle(b: &B, path: &str, expected: &RecycleExpectation) -> VfsResult<RecycleOutcome>; // (Err Unsupported)
pub fn change_signal_mode(b: &B, root: &str) -> VfsResult<Option<ChangeSignalMode>>;    // (Ok(None))
pub fn change_signal(b: &B, root: &str, poll_interval: Duration,
    tx: crossbeam_channel::Sender<ChangeNotice>) -> VfsResult<Option<ChangeSubscription>>; // (Ok(None) = selbst abfragen)
// Gleichnamige Methoden mit denselben Parametern (ohne `b`) und denselben Standards im Trait
// BackendExtensions (Standard von list_dir_tolerant/open_read_regular/open_write_copy_stage_timed:
// list_dir / open_read_id / open_write_copy_stage_sized des Backends selbst).

pub struct VfsListing { pub entries: Vec<VfsMeta>, pub omitted: Vec<VfsOmission> } // fn complete(entries) -> Self
pub struct VfsOmission { pub rel: String, pub reason: OmissionReason, pub detail: String }
pub enum OmissionReason { Link, Special, Unreadable, Vanished, Unrepresentable }
pub enum MtimePrecision { Nanos, Millis, TenMillis, Seconds, TwoSeconds, Minutes, Days, Unknown } // Default Unknown;
    // Ord (grob = größer); fn step_ms(self) -> Option<u64>; fn coarser(self, other) -> Self;
    // fn same_instant(self, a_ms: i64, b_ms: i64) -> bool; fn local_time_shifts(self) -> bool (nur TwoSeconds)
pub enum NameLimit { Bytes(usize), Utf16Units(usize) }                    // fn fits(self, name: &str) -> bool
pub enum NameIssue { Windows(crate::types::Win32NameIssue), TooLong }
pub struct TargetLimits { pub windows_names: bool, pub max_name: Option<NameLimit>,
    pub max_file_size: Option<u64>, pub mtime_precision: MtimePrecision } // Default = alles unbekannt;
    // fn name_issue(&self, name: &str) -> Option<NameIssue>; fn fits_size(&self, size: u64) -> bool
pub enum StageDurability { NotRequired /*Default*/, Deferred, Now }
pub struct StageFinish { pub mtime_ms: Option<i64>, pub mode: Option<u32>, pub durability: StageDurability } // Copy, Default
pub struct StageFinished { pub mtime_applied: bool, pub durable: bool }  // Copy, Default
pub struct HashWalkRequest { pub algorithm: Option<crate::analytics::HashAlgorithm>, pub min_bytes: u64 } // Copy
pub enum HashWalkItem { Entry(HashWalkEntry), Omitted(VfsOmission) }
pub struct HashWalkEntry { pub rel: String, pub is_dir: bool, pub size: u64, pub mtime_ms: i64, pub digest: Option<String> }
pub struct RecycleExpectation { pub size: u64, pub sha256: Option<String> }
pub enum RecycleOutcome { Recycled, Changed }
pub enum ChangeSignalMode { Push, Poll }
pub enum ChangeNotice { Ready { generation: Option<u64> }, Changed { generation: Option<u64>, paths: Vec<String> },
    Overflow, Ended(String) }
pub struct ChangeSubscription;                                            // fn new(guard: impl Send + 'static) -> Self; Drop beendet

// Fehlerklassen (core/error_classes.rs)
pub fn is_target_refusal(error: &io::Error) -> bool;          // StorageFull | QuotaExceeded | ReadOnlyFilesystem, auch eingewickelt
pub fn omission_reason(error: &io::Error) -> Option<OmissionReason>; // abgelehnter Link/Spezial → Link/Special,
    // Ordner statt Datei/NotFound → Vanished, PermissionDenied → Unreadable, InvalidFilename → Unrepresentable

// Lokal (vfs + local_access)
pub fn is_staging_name(name: &str) -> bool;                    // alle Zwischen-/Spool-/Quarantäne-Namen der App
pub fn local_volume_identity(path: &str) -> io::Result<Option<VolumeIdentity>>;
pub fn local_mount_boundary(path: &str) -> io::Result<Option<MountKind>>; // Some = Einhängepunkt innerhalb des Baums
pub struct VolumeIdentity { pub volume_id: String, pub relative_path: String, pub fs_type: String }
    // serde; Gleichheit/Hash nur über volume_id + relative_path; fn key(&self) -> String
pub enum MountKind { Local, Network, Fuse, Automount, Pseudo }
pub(crate) fn crate::local_access::open_regular(path: &Path, final_link: FinalLink) -> io::Result<File>;
pub(crate) enum FinalLink { Follow, Refuse }
pub(crate) enum NotRegular { Link, Directory, Special } // steckt im io::Error (InvalidInput); fn of(&io::Error) -> Option<Self>
```

Semantik, auf die sich andere Blöcke verlassen dürfen:
- **Auslassungen** (`VfsListing.omitted`, `HashWalkItem::Omitted`) sind nie „fehlt“: Gegenstück und Basis-
  Einträge bleiben geschützt, auch für alles unterhalb von `rel`. `Ok(VfsListing)` heißt: jedes vorhandene
  Kind steht in `entries` oder `omitted`; bricht die Aufzählung selbst ab, ist es ein Fehler (dann ist der
  ganze Ordner „unlesbar“). Links und Spezialdateien bleiben in Listen als `entries` (Flags), Hash-Walks melden
  sie als Auslassung.
- **Zwischendateien**: schreiben (`open_write_copy_stage_timed` bzw. `…_sized`) → schließen →
  `finish_stage(StageFinish { mtime_ms, mode, durability })` → `promote_*`. `mtime_applied == false` = Ziel
  behält eigene Zeiten (Spiegel vergleicht dann gegen seine Basis); Zeit/Rechte, die das Ziel nicht speichern
  kann, sind kein Fehler; ein misslungenes Flush ist einer. `mode` = Quellrechte & ggf. Rechte der ersetzten
  Zieldatei, auf `0o777` maskiert, nur wo das Ziel Unix-Rechte hat. Durability: `Now` beim Ersetzen,
  `Deferred` für neue Dateien + vor jedem Basis-Zwischenstand `sync_filesystem(root)`; `Ok(false)` dort = keine
  Garantie verfügbar (dann ist `durable` der einzelnen Stufen maßgeblich).
- **Zeitgenauigkeit**: Vergleich über `a.coarser(b).same_instant(x, y)`; bei `local_time_shifts()` (FAT) gilt
  zusätzlich genau 1 h ± ein Schritt als gleich (K3 entscheidet, B23).
- **Hash-Walk**: `min_bytes` lässt kleinere Dateien weg (nur Duplikatsuche); Sync übergibt 0. `Ok(false)` nur
  vor dem ersten Element; jeder spätere Fehler ist ein Fehler.
- **Änderungs-Abo**: Push (Share `watch_v1`) oder Poll (WebDAV-Wurzel-ETag, Drive-Feed, Takt =
  `poll_interval`); `Ready` nach (Wieder-)Anmeldung und `Overflow` heißen „Wurzel einmal prüfen“; `Ended` =
  selbst abfragen und später neu abonnieren (T-JOBS bildet auf `WatchEvent` ab).
- **Fehlerklassen**: Backends bilden „voll/Kontingent/schreibgeschützt“ auf `StorageFull`/`QuotaExceeded`/
  `ReadOnlyFilesystem` ab (FTP 452/552, WebDAV 507/413, SFTP per `statvfs`, Drive `storageQuotaExceeded`,
  SMB DiskFull, Share `FsErrorKind::StorageFull`); Lokal liefert das Betriebssystem sie selbst.
- **`open_read_regular`** (lokal): folgt keinem Link am letzten Element, wartet nie auf FIFOs, liest keine
  Geräte; Windows-Daten-Reparse-Punkte (OneDrive, WOF, Dedup) sind normale Dateien. Fern: wie `open_read_id`.
- **Volume-Identität**: `Ok(None)` heißt „unbekannt“, nie „fremd“.

Stand der Implementierung (Vertrag, ohne Verhaltensänderung bestehender Wege): `VfsMeta.special` gesetzt;
`LocalBackend`: `open_read_regular` (O_NONBLOCK|O_NOFOLLOW|O_NOCTTY + Typprüfung vor/nach dem Öffnen,
Windows Link-Prüfung), `finish_stage` (Zeit, Rechte, Flush; `Deferred` flusht vorerst sofort),
`sync_filesystem` (Linux/Android `syncfs`, Windows `Ok(false)`), `unix_mode`; `is_target_refusal`,
`omission_reason`, `is_staging_name` vollständig. Stubs bis V-LOCAL: `list_dir_tolerant` (strikte Liste),
`target_limits` (unbekannt), `local_volume_identity`/`volume_identity` (`Ok(None)`), `local_mount_boundary`
(`Ok(None)`).

### V2 Share-Draht (K2)
- Fähigkeiten (additiv): `duplicate_search_v1`, `hash_walk_v1`, `list_batches_v1`, `remote_trash_v1`,
  `stage_mtime_v1`, `analysis_deflate_v1`, `analysis_reattach_v1`, `watch_v1`, `export_access_v1`.
- `FsRequest` neu: `DuplicateSearch{path,min_bytes,request_id}`, `HashWalk{path,algo,min_bytes}`,
  `ListDirBatch{path,cursor}`, `Recycle{path,expected_size,expected_sha256}`, `SetStageMtime{staged,mtime_ms}`,
  `WatchExport{path}`; `StorageAnalysis` additiv `request_id`, `node_budget`, `compress`. Die Einordnung
  lesend/schreibend ist ein erschöpfendes `match` ohne Platzhalter.
- `FsMeta.special` (additiv), `FsErrorKind::StorageFull`.
- `AnalysisReport` additiv: `volume`, `platform`, `protected`.
- Freigabe-Typen ziehen in die neue Datei `share/core/export_config.rs` (alte Pfade per `pub use`):
  `SharedRoot{…, access: ExportAccess, allow_system_writes: bool}`, `ExportAccess::default() = ReadOnly`,
  Altdaten über `serde(default = "legacy_read_write")`; `ShareExportConfig` erhält die Verbindungs-Freigabe
  als Liste (`shared_connections: Vec<SharedConnection{name, access}>`, Altfeld `include_connections` wird
  beim Laden migriert). Nach dem Vertrag gehören `fs.rs` H-DISPATCH und `export_config.rs` S-POLICY.
- Host-Einstiege für H-DISPATCH: `serve_duplicate_search`, `serve_hash_walk`, `serve_list_batch`,
  `serve_recycle`, `serve_set_stage_mtime`, `serve_watch` (Signaturen hier nachtragen).

Signaturen (K2, eingetragen). Drahttypen in `share/core/wire.rs` (Anfragen in `share/core/fs_request.rs`, über
`wire` re-exportiert), Freigabe-Typen in `share/core/export_config.rs`, Host-Einstiege in
`share/core/host_requests.rs`, Berichtsfelder in `analytics/core/{analysis_report,host_figures}.rs`.

```rust
// Fähigkeiten: je Pfad in der Capabilities-Antwort (FsResponse::Capabilities { capabilities, .. })
pub(crate) struct FsWriteCapabilities { pub(crate) create: bool, pub(crate) replace: bool,
    pub(crate) namespace_replace: bool, pub(crate) transfer: FsTransferCapabilities,
    pub(crate) features: FsHostFeatures,          // additiv: Host-Fähigkeiten, gleich für jeden Pfad
    pub(crate) access: Option<ExportAccess> }     // additiv: Zugriff der Freigabe des Pfads; None für "/",
                                                  // "/Verbindungen", alte Hosts; verbindlich nur mit export_access_v1
pub(crate) struct FsHostFeatures { pub(crate) duplicate_search_v1: bool, pub(crate) hash_walk_v1: bool,
    pub(crate) list_batches_v1: bool, pub(crate) remote_trash_v1: bool, pub(crate) stage_finish_v1: bool,
    pub(crate) analysis_deflate_v1: bool, pub(crate) analysis_reattach_v1: bool, pub(crate) watch_v1: bool,
    pub(crate) export_access_v1: bool }          // Copy, Default (= alter Host), JSON nur gesetzte Flags
    // fn host() -> Self (was dieser Build anbietet; Vertrag: nichts); fn is_absent(&self) -> bool;
    // fn names(&self) -> Vec<&'static str>

// Anfragen (neue Arten als Newtype mit eigener Nutzlast; JSON wie bisher {"op": "...", feld: ...})
pub(crate) enum FsRequest { /* bisherige Arten unverändert, außer: */
    StorageAnalysis(FsStorageAnalysis),           // war StorageAnalysis { path }, JSON gleich
    DuplicateSearch(FsDuplicateSearch), HashWalk(FsHashWalk), ListDirBatch(FsListBatch),
    Recycle(FsRecycle), FinishStage(FsStageFinish), SyncFilesystem(FsSyncFilesystem), WatchExport(FsWatch) }
pub(crate) struct FsStorageAnalysis { pub(crate) path: String, pub(crate) request_id: Option<String>,
    pub(crate) node_budget: Option<u64>, pub(crate) compress: bool }      // Default
pub(crate) struct FsDuplicateSearch { pub(crate) path: String, pub(crate) min_bytes: u64,
    pub(crate) request_id: Option<String> }                               // Default
pub(crate) struct FsHashWalk { pub(crate) path: String, pub(crate) algo: Option<FsHashAlgo>,
    pub(crate) min_bytes: u64 }                                           // Default; algo None = nur Größe/Zeit
pub(crate) enum FsHashAlgo { Md5, Sha256, Unknown /* serde(other) → Host: Unsupported */ }
pub(crate) struct FsListBatch { pub(crate) path: String, pub(crate) cursor: Option<String> }
pub(crate) struct FsRecycle { pub(crate) path: String, pub(crate) expected_size: u64,
    pub(crate) expected_sha256: Option<String> }                          // hex wie vfs::RecycleExpectation
pub(crate) struct FsStageFinish { pub(crate) staged: String, pub(crate) mtime_ms: Option<i64>,
    pub(crate) mode: Option<u32>, pub(crate) durability: FsStageDurability }   // = vfs::StageFinish
pub(crate) enum FsStageDurability { NotRequired /* Default */, Deferred, Now, Unknown /* serde(other) */ }
pub(crate) struct FsSyncFilesystem { pub(crate) path: String }            // = vfs::sync_filesystem(root)
pub(crate) struct FsWatch { pub(crate) path: String }
impl FsRequest {
    pub(in crate::share) fn mutates_filesystem(&self) -> bool;  // aus erschöpfendem match ohne Platzhalter
    pub(in crate::share) fn is_batch(&self) -> bool;            // unverändert
    pub(in crate::share) fn is_transfer_v1(&self) -> bool;      // unverändert
}

// Antworten (additiv)
pub(crate) enum FsResponse { /* bisherige */, Recycle { moved: bool },     // false = Inhalt geändert, nichts bewegt
    StageFinished { mtime_applied: bool, durable: bool }, Synced { durable: bool } }
pub(crate) struct FsMeta { /* bisherige */, pub(crate) special: bool }    // serde default, nur gesendet wenn true
pub(crate) enum FsErrorKind { NotFound, PermissionDenied, AlreadyExists, Unsupported, Busy,
    StorageFull, QuotaExceeded, ReadOnly, FileTooLarge, InvalidName, Unknown /* serde(other) */ }
    // ↔ io::ErrorKind::{StorageFull, QuotaExceeded, ReadOnlyFilesystem, FileTooLarge, InvalidFilename};
    //   ältere Gegenstellen lesen die neuen Arten als Unknown (Text bleibt)

// Host-Einstiege (share/core/host_requests.rs; Aufruf aus server_fs.rs nach der Zulassung)
pub(in crate::share) async fn serve_duplicate_search(send: SendStream, request: FsDuplicateSearch,
    access: FsAccess, principal: PeerPrincipal) -> io::Result<()>;
pub(in crate::share) async fn serve_hash_walk(send: SendStream, request: FsHashWalk,
    access: FsAccess, principal: PeerPrincipal) -> io::Result<()>;
pub(in crate::share) async fn serve_list_batch(send: SendStream, request: FsListBatch,
    access: FsAccess, principal: PeerPrincipal) -> io::Result<()>;
pub(in crate::share) async fn serve_watch(send: SendStream, request: FsWatch,
    access: FsAccess, principal: PeerPrincipal) -> io::Result<()>;
pub(in crate::share) fn serve_recycle(target: ResolvedTarget, request: FsRecycle) -> io::Result<FsResponse>;
pub(in crate::share) fn serve_finish_stage(target: ResolvedTarget, request: FsStageFinish) -> io::Result<FsResponse>;
pub(in crate::share) fn serve_sync_filesystem(target: ResolvedTarget, request: FsSyncFilesystem)
    -> io::Result<FsResponse>;
// share/core/storage_analysis_server.rs, Signatur geändert:
pub(super) async fn serve(send: SendStream, request: FsStorageAnalysis, access: FsAccess,
    principal: PeerPrincipal) -> io::Result<()>;

// Freigaben (crate::share::{ExportAccess, ShareExportConfig, SharedConnection, SharedRoot};
// alter Pfad share::fs::{ShareExportConfig, SharedRoot} bleibt per pub use)
pub enum ExportAccess { ReadWrite, ReadOnly /* Default; serde(other): unbekannte Werte = ReadOnly */ }
    // Copy; fn allows_write(self) -> bool; JSON "read_write" / "read_only"
pub struct SharedRoot { pub label: String, pub path: String,
    pub access: ExportAccess,                     // serde(default = "legacy_read_write"): Profile vor RV1 = ReadWrite
    pub allow_system_writes: bool }               // serde(default), nur gesendet wenn true
    // fn new(label: impl Into<String>, path: impl Into<String>) -> Self  (ReadOnly, false)
    // fn with_access(self, access: ExportAccess) -> Self
pub struct SharedConnection { pub account: String /* SavedConnection::account() */,
    pub access: ExportAccess /* serde(default) = ReadOnly */ }
pub struct ShareExportConfig { pub roots: Vec<SharedRoot>,
    pub include_connections: bool,                // Altfeld (alle Verbindungen, ReadWrite); wird weiter geschrieben
    pub shared_connections: Vec<SharedConnection> }  // serde(default), nur gesendet wenn nicht leer
    // fn shares_connections(&self) -> bool
    // fn connection_access(&self, account: &str) -> Option<ExportAccess>   (Eintrag vor Altflag)
    // fn migrate_legacy_connections<I>(&mut self, saved_accounts: I) -> bool
    //     where I: IntoIterator, I::Item: Into<String>                    (einmalig; true = geändert)
pub(crate) struct share::fs::ResolvedTarget { /* bisherige */, pub(crate) access: ExportAccess }

// Analysebericht (analytics; AnalysisReport ist der Draht, ScanOutcome das Ergebnis beim Empfänger)
pub(crate) struct AnalysisReport { /* bisherige */, pub(crate) volume: Option<VolumeUsage>,
    pub(crate) platform: Option<PlatformFigures>, pub(crate) protected: Vec<ProtectedOmission> } // serde default
pub struct ScanOutcome { /* bisherige inkl. protected */, pub volume: Option<VolumeUsage>,
    pub platform: Option<PlatformFigures> }
pub struct VolumeUsage { pub total_bytes: u64, pub free_bytes: u64 }   // Copy, serde; fn used_bytes(&self) -> u64
pub struct PlatformFigures { pub app_data: Option<Vec<String>>, pub whole_volume: bool,
    pub volume_used_bytes: Option<u64>, pub other_apps_bytes: Option<u64>, pub apps: Vec<PlatformApp>,
    pub measured_ms: i64 }                        // serde, Default
    // fn new(place: &VolumeRoot, totals: &PlatformTotals, measured_ms: i64) -> Self
    // fn place(&self) -> VolumeRoot; fn totals(&self) -> PlatformTotals
pub struct PlatformApp { pub package: String, pub label: String, pub app_bytes: u64, pub data_bytes: u64,
    pub cache_bytes: u64 }
pub fn remember_platform_totals(volume: &Path, totals: &PlatformTotals);  // Android: Zahlen des Primärvolumes
pub fn remembered_platform_totals() -> Option<(PathBuf, PlatformTotals, i64)>;
// ProtectedOmission jetzt serde. AnalysisReport::validate: ≤ 16 geschützte Bereiche, ≤ 4096 Apps,
// je ≤ 256 KiB Text.
```

Semantik, auf die sich andere Blöcke verlassen dürfen:
- **Aushandlung**: Ein Client schickt eine RV1-Anfrage nur, wenn `features` des Pfads sie anbietet; sonst
  bisheriger Weg. Ein Host ohne Angebot antwortet `Unsupported` (wie ein älterer Host, der die Anfrage nicht
  lesen kann). Im Vertrag bietet der Host noch nichts an; jede Fähigkeit wird mit ihrer Umsetzung eingeschaltet.
- **Schreib-Einordnung** (H-DISPATCH-Gatter `mutates_filesystem()`): schreibend `Write`, `WriteNew`,
  `WriteDone`, `MkdirAll`, `CreateDir`, `Rename`, `RenameNoReplace`, `PromoteStaged`, `PromoteNoReplace`,
  `CopyFile`, `RemoveFile`, `RemoveDir`, `DiscardStage`, `PutBatch`, `Recycle`, `FinishStage`, `SyncFilesystem`;
  alles andere lesend (auch Capabilities/Leases, `PutBatchStatus`, `GetBatch`, `ReadAt`, Analysen, Watch).
- **Host-Einstiege**: die vier `async`-Einstiege besitzen den Strom (Antworten, Herzschlag, Abbruch über
  `send.stopped()`), H-DISPATCH ruft sie nach der Zulassung und antwortet selbst nichts mehr. Die drei
  blockierenden Schreib-Einstiege bekommen das von H-DISPATCH aufgelöste und geprüfte Ziel (`request.path`,
  `request.staged` bzw. `request.path`), laufen unter `run_authorized` im Steuer-Pool; H-DISPATCH sendet die
  zurückgegebene Antwort bzw. den Fehler (Hilfsfunktion `answer`, siehe `anfragen/K2.md`). `FinishStage` nimmt
  nur Zwischendateinamen des Clients an. Ablehnung „nur lesbar“ als `io::ErrorKind::ReadOnlyFilesystem`
  (Draht `ReadOnly`, Client stoppt per `vfs::is_target_refusal`).
- **PeerBackend** (K2, H-ANALYSIS) bildet die V1-Erweiterungen 1:1 auf diese Anfragen ab:
  `list_dir_tolerant`→`ListDirBatch`, `find_duplicates`→`DuplicateSearch`, `hash_walk`→`HashWalk`,
  `recycle`→`Recycle`, `finish_stage`→`FinishStage`, `sync_filesystem`→`SyncFilesystem`,
  `change_signal`→`WatchExport` (Push), `supports_*` aus `features`.
- **Freigaben**: Altdaten behalten ihre Bedeutung (Wurzeln ohne `access` = ReadWrite; `include_connections` =
  alle gespeicherten Verbindungen ReadWrite, auch beim Lesen durch alte Versionen). `fs.rs` beachtet schon
  `shared_connections` je Verbindung und füllt `ResolvedTarget.access`. S-POLICY ruft beim Laden
  `migrate_legacy_connections(alle SavedConnection::account())` und speichert; neue Freigaben über
  `SharedRoot::new` (ReadOnly).
- **Bericht**: Der Host sendet geschützte Bereiche weiter auch als Hinweis (ältere Clients); neue Clients
  nehmen `ScanOutcome.protected` und dürfen den gleichlautenden Hinweis (`protected_note`) ausblenden.
  `volume`/`platform` `None` = unbekannt. A-CLIENT ruft `remember_platform_totals(primäres Volume, totals)`,
  wenn Kotlin Plattformzahlen liefert; der Android-Host hängt sie an Analysen eines Peers (nur an das
  Volume, Apps nur an die ganze Primär-Wurzel).

Abweichungen vom Plan (mit Grund): `SetStageMtime`/`stage_mtime_v1` → `FinishStage` + `SyncFilesystem` /
`stage_finish_v1`, weil V1 `finish_stage` (Zeit, Rechte, Dauerhaftigkeit) und `sync_filesystem` festlegt
(Share-Ziele bekommen alle drei, Y131); neue Anfragen als Newtype mit `Fs*`-Nutzlast (JSON identisch, die
Einstiege nehmen die Nutzlast); `FsErrorKind` mit fünf statt einer neuen Art (nur lesbar, Kontingent, zu groß,
Name unmöglich steuern Sync-Abbruch bzw. Auslassungen); `features`/`access` in `FsWriteCapabilities` statt als
neue Felder von `FsResponse::Capabilities` (bricht keine bestehende Zerlegung); `SharedConnection.account`
statt `name` (stabiler Schlüssel, der sichtbare Name entsteht aus dem Label und kann Zusätze bekommen);
`include_connections` bleibt als Altfeld (ältere Versionen verlangen es).

Stand: Vertrag umgesetzt, rustfmt-sauber; ohne Verhaltensänderung bis auf: `access` wird in Capabilities
gemeldet (informativ), `FsMeta.special` und die neuen Fehlerarten reisen mit. Kompiliert erst mit dem Patch aus
`anfragen/K2.md` (server_fs.rs + Struct-Literale fremder Dateien). Antwortformen der Ströme (Duplikate,
Hash-Walk, Listen-Portionen, Watch) legt H-ANALYSIS fest (nur K2 nutzt sie) und trägt sie hier nach.

### V3 Sync-Engine (K3)
- Schnappschuss je Seite: Baum + Ordnermenge + Auslassungen mit Art {Link, Unlesbar, Verschwunden,
  NichtDarstellbar, Speziell, Gefiltert, EigeneDatei (Zwischendatei, `.se-sync-replica`, `.se-versions`,
  App-Daten), Systemordner, Einhängung, ZuGroßFürZiel, NameAufZielUnmöglich}; Auslassungen schützen
  Gegenstück und Basis.
- Planungsschlüssel je Paar: Faltung (wenn eine Seite nicht unterscheidet) + NFC (`icu_normalizer`); jede
  Seite behält ihre Schreibweise für I/O.
- Apply meldet erledigte Aktionen laufend: `CompletedAction{rel, kind, src_sig, dst_sig, durable}`;
  Orchestrierung schreibt Basis-Zwischenstände (alle N Aktionen / T Sekunden, nur `durable`) und am Ende.
- Replika: Markierung `.se-sync-replica` (JSON `{replica_id, created_ms, pair_hint}`) je Wurzel; Basis je
  (Paar-ID, Replika-ID A, Replika-ID B); ohne Markierung: Volume-Identität aus V1; „unbekannt“ ≠ „fremd“.
- Löschschutz: `max_delete_pct` 50 + `max_delete_min` 25, Migration per `config_version` in der Job-Datei.
- `SyncJob` additiv: `rt_max_latency_secs`, `rt_poll_secs` (300), `verify_interval_secs` (3600),
  `verify_target_secs` (86400), `cross_mounts` (neue Jobs aus), `versions_location` {Auto, AppData},
  `config_version`. Laufzeitdaten in einer Zustandsdatei je Job (Modul von T-JOBS, V4).
- Versionen: Modul `bisync/os/shared/versions*.rs` (Stub von K3, Besitz danach E-APPLY): Ordner
  `.se-versions/<lauf>/…` + Verzeichnis, Aufbewahrung je Datei, Bereinigung nach jedem Lauf.
- Paar-Sperre `bisync::PairLock::acquire(pair_id)` (Datei-Lock, geräteweit), genutzt von Daemon, Desktop,
  Android und Konfliktlösung.

Signaturen (K3, eingetragen). Alles unter `crate::bisync::…` (Kern-Typen in `bisync/core/*.rs`,
plattformneutral), Jobs unter `crate::syncjobs::…`.

```rust
// Kern-Typen
pub enum PairSide { A, B }      // Copy, Ord, Hash; other(), as_str() "a"/"b", parse(&str), label() "Quelle"/"Ziel"
                                // (bisync::sync_flows::PairSide ist jetzt derselbe Typ)
pub enum VersionsLocation { Auto, AppData }       // as_str "auto"/"appdata", parse, label, ALL
pub struct BisyncOptions { /* bisherige Felder */
    pub max_delete_min: u64,        // Prozent-Stopp erst ab so vielen Löschungen; 0 = nur Prozent (Altverhalten)
    pub cross_mounts: bool,         // Default true (Altverhalten); Jobs: SyncJob::cross_mounts
    pub versions: VersionsLocation, // Default AppData (Altverhalten); Jobs: SyncJob::versions_location
    pub verify_target_secs: u64 }   // Ziel ohne Voll-Liste seit so vielen s → vollständig listen; 0 = nie
pub enum OmissionKind { Link, Unreadable, Vanished, NotRepresentable, Special, Filtered, OwnFile,
    SystemFolder, Mount, TooLargeForTarget, NameImpossibleOnTarget }
    // ALL; as_str link|unreadable|vanished|unrepresentable|special|filtered|own|system|mount|too_large|
    // name_impossible; parse; label (deutsch); reported_by_default() (false nur Filtered, OwnFile);
    // impl From<crate::vfs::OmissionReason>
impl SyncOmissions {            // record(rel, report) = record_kind(rel, Link, report); Rest unverändert
    pub(crate) fn record_kind(&mut self, relative: &str, kind: OmissionKind, report: bool);
    pub fn reported(&self) -> impl Iterator<Item = (&str, OmissionKind)>;
    pub fn counts(&self) -> BTreeMap<OmissionKind, u64>; }   // gemeldete je Art
pub type DirSet = BTreeSet<String>;
pub struct SideSnapshot { pub tree: Tree, pub filtered: Tree, pub dirs: DirSet, pub omissions: SyncOmissions }
    // Default; new(fold_case: bool); is_empty() (keine Datei, gefilterte Datei, kein Ordner, keine gemeldete
    // Auslassung – eigene Einträge zählen nicht); entry_count() -> u64
pub struct KeyPolicy { pub fold_case: bool }   // Default; for_pair(a_case_sensitive, b_case_sensitive);
    // key<'a>(&self, rel: &'a str) -> Cow<'a, str> (NFC, dann einfache Großschreibung je Zeichen; ß bleibt)
pub struct Spellings;           // Default; insert(rel, side, spelling) (gleiche Schreibweise wird nicht gespeichert);
    // side_rel<'a>(&'a self, rel: &'a str, side: PairSide) -> &'a str; is_empty()
pub enum CompletedKind { Copied { from: PairSide }, Moved { from: PairSide }, Deleted { side: PairSide },
    DirCreated { side: PairSide }, DirRemoved { side: PairSide } }
pub struct CompletedAction { pub rel: String, pub kind: CompletedKind, pub src_sig: Option<Sig>,
    pub dst_sig: Option<Sig>, pub durable: bool }
    // baseline_entry() -> Option<(Option<Sig>, Option<Sig>)>: (A, B); (None, None) = Eintrag entfernen; None für Ordner
pub enum DirAction { Create { side: PairSide, rel: String }, Remove { side: PairSide, rel: String } } // side(), rel()
pub trait ApplySink: Sync {
    fn completed(&self, action: CompletedAction);
    fn omitted(&self, rel: &str, kind: OmissionKind) {}   // Standard: nichts
    fn deferred(&self, rel: &str, reason: &str) {}
    fn stopped(&self, stop: RunStop) {} }
pub enum RunBlock { MassDelete { side: PairSide, deletes: u64, files: u64 }, DeleteLimit { deletes: u64, limit: u64 },
    SideEmpty { side: PairSide, previous: u64 }, ReplicaMissing { side: PairSide } }
    // code() mass_delete|delete_limit|side_empty|replica_missing; message() -> String (deutsch, was + was tun);
    // confirmation() -> BlockConfirmation
pub enum BlockConfirmation { Deletes { side: Option<PairSide>, max: u64 }, AcceptSide { side: PairSide } }
    // token() "deletes:b:120" | "deletes:*:40" | "accept:a"; parse(&str) -> Option<Self>; covers(&RunBlock) -> bool
pub enum RunStop { TargetFull { side: PairSide }, TargetReadOnly { side: PairSide }, ConnectionLost { side: PairSide } }
    // code() target_full|target_read_only|connection_lost; message()
pub enum ScanDepth { Incremental /* Default */, VerifySources, Full }   // as_str incremental|verify_sources|full, parse
pub enum StateOwner { Job(String), AdHoc /* Default */ }
pub enum ReplicaRef { Marker(String), Volume(String), Unknown }
pub struct StateKey { pub pair_id: String, pub lock_id: String, pub owner: StateOwner,
    pub replica_a: ReplicaRef, pub replica_b: ReplicaRef }    // legacy(pair_id, lock_id); is_legacy(); replica(side)
pub struct RunSettings { pub owner: StateOwner, pub depth: ScanDepth, pub confirmed: Vec<BlockConfirmation>,
    pub lock_wait: Duration }                                 // Default; for_job(job_id)
pub struct SyncLimits { pub walk_entries: u64, pub walk_text_bytes: u64, pub state_entries: u64,
    pub state_text_bytes: u64 }
    // FALLBACK (je Seite 1 Mio./128 MiB, Zustand das Doppelte); for_memory(physical: Option<u64>) (¼ des RAM,
    // nie unter FALLBACK); state_file_bytes()
pub const REPLICA_MARKER_NAME: &str = ".se-sync-replica";
pub const VERSIONS_DIR_NAME: &str = ".se-versions";
pub fn is_engine_name(name: &str) -> bool;   // in jeder Tiefe eigene Einträge (Walk, Überwachung, Duplikate, Share)

// Lauf, Sperre, gespeicherter Zustand
pub struct Outcome { /* bisherige Felder */ pub blocked: Option<RunBlock>, pub stopped: Option<RunStop>,
    pub deferred: Vec<(String, String)>, pub busy: bool, pub canceled: bool, pub state: Option<StateKey>,
    pub run_id: Option<String> }
pub struct RunRequest<'a> { pub a: &'a dyn Backend, pub root_a: &'a str, pub b: &'a dyn Backend,
    pub root_b: &'a str, pub opts: BisyncOptions, pub filter: &'a WalkFilter<'a>, pub cancel: &'a AtomicBool,
    pub settings: RunSettings, pub observer: Option<&'a dyn ApplySink> }
    // RunRequest::new(a, root_a, b, root_b, opts, filter, cancel) = AdHoc, Incremental, keine Freigabe, kein Warten
pub fn run_with(request: RunRequest<'_>) -> Outcome;          // `run(..)` bleibt unverändert (= AdHoc)
pub fn pair_key_policy(a: &dyn Backend, root_a: &str, b: &dyn Backend, root_b: &str) -> KeyPolicy;
pub struct PairLock;            // Debug; Drop gibt frei (auch Prozessende/Absturz)
impl PairLock {
    pub fn acquire(lock_id: &str) -> io::Result<PairLock>;   // sofort; ErrorKind::WouldBlock = läuft bereits
    pub fn acquire_wait(lock_id: &str, wait: Duration, cancel: &AtomicBool) -> io::Result<PairLock>;
                                                              // WouldBlock nach `wait`, Interrupted bei cancel
    pub fn id(&self) -> &str; }
pub fn pair_lock_id(a: &dyn Backend, root_a: &str, b: &dyn Backend, root_b: &str) -> String;  // A↔B = B↔A
pub fn baseline_file(key: &StateKey) -> io::Result<PathBuf>;  // legacy → baseline_path(pair); sonst
    // sync/pairs/<pair_id>/{job-<id>|adhoc}.<replica-token>.sebl
pub fn merge_baseline_entries(lock: &PairLock, key: &StateKey,
    entries: &[(String, (Option<Sig>, Option<Sig>))]) -> io::Result<()>;   // (None, None) entfernt; falsche Sperre = InvalidInput
pub fn forget_job_state(job_id: &str) -> io::Result<()>;      // alle Zustände des Jobs (alle Paare)
pub fn forget_pair_state(pair_id: &str) -> io::Result<()>;    // Paar-Ordner, Altdatei, inkrementeller Index
#[allow(clippy::too_many_arguments)]
pub fn resolve_recorded(a: &dyn Backend, root_a: &str, b: &dyn Backend, root_b: &str, conflict: &Conflict,
    keep_a: bool, variant_id: Option<&str>, state: &StateKey, cancel: &AtomicBool,
    progress: impl FnMut(ResolvePhase)) -> io::Result<(Option<Sig>, Option<Sig>)>;
    // Sperre (sofort, WouldBlock) + Auflösung + Eintrag in `state`; ersetzt eigenes save_baseline der Oberflächen
pub struct Preview { /* bisherige Felder */ pub blocked: Option<RunBlock>, pub state: Option<StateKey>,
    pub planned: Baseline }     // planned: (A, B)-Signaturen jedes Aktions-rel
#[allow(clippy::too_many_arguments)]
pub fn apply_preview_action(a: &dyn Backend, root_a: &str, b: &dyn Backend, root_b: &str, preview: &Preview,
    action: &Action, opts: BisyncOptions, cancel: &AtomicBool) -> io::Result<BisyncStats>;   // Y148

// Versionen: Modul `bisync::versions` (pub mod; Stub K3, danach E-APPLY)
pub struct VersionsContext { pub pair_id: String, pub owner: StateOwner, pub job_name: String, pub run_id: String,
    pub started_ms: i64, pub location: VersionsLocation, pub versioning: Versioning }
    // new(pair_id, owner, location, versioning) (run_id = new_run_id(jetzt))
pub fn new_run_id(started_ms: i64) -> String;                // "20261002T153012Z-1f2e3d4c", sortierbar
pub struct RunVersions;         // Debug; begin(VersionsContext); context(); run_id(); app_data_dir() -> &Path;
                                // finish(&self) -> io::Result<()>
pub enum VersionReason { Replaced, Deleted, Resolved, Restored }
pub enum VersionStore { SyncRoot, AppData }
pub struct VersionEntry { pub side: Option<PairSide>, pub rel: String, pub run_id: String, pub preserved_ms: i64,
    pub reason: Option<VersionReason>, pub size: u64, pub mtime_ms: i64, pub store: VersionStore,
    pub stored_path: String, pub job_id: Option<String> }    // None = Versionen von vor RV1
pub struct VersionSide<'a> { pub side: PairSide, pub backend: &'a dyn Backend, pub root: &'a str }
pub fn prune_after_run(lock: &PairLock, pair_id: &str, sides: &[VersionSide<'_>], versioning: &Versioning,
    cancel: &AtomicBool) -> io::Result<()>;
pub fn list_versions(pair_id: &str, sides: &[VersionSide<'_>], cancel: &AtomicBool) -> io::Result<Vec<VersionEntry>>;
pub fn restore_version(lock: &PairLock, pair_id: &str, entry: &VersionEntry, side: &VersionSide<'_>,
    cancel: &AtomicBool) -> io::Result<()>;
pub fn remove_versions(lock: &PairLock, pair_id: &str, sides: &[VersionSide<'_>], cancel: &AtomicBool)
    -> io::Result<()>;

// Apply- und Walk-Schnittstelle (bisync-intern)
pub(super) struct ApplyScope<'a> { pub(super) sink: &'a dyn ApplySink, pub(super) versions: &'a RunVersions,
    pub(super) spellings: &'a Spellings }                    // checkpoint.rs (K3)
pub(super) struct CollectingSink;   // Default, ApplySink; take() -> Collected { completed, omitted, deferred, stop }
// E-APPLY liefert neu in apply.rs (K3 ruft es aus der Orchestrierung):
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_planned_reporting(actions: &[Action], dirs: &[DirAction], planned_a: &Tree,
    planned_b: &Tree, endpoints: SyncEndpoints<'_>, opts: BisyncOptions, scope: &ApplyScope<'_>,
    errors: &mut Vec<(String, String)>, cancel: &AtomicBool) -> ApplyReport;
// E-APPLY ändert in snapshot_pair.rs (Signatur von read_pair bleibt):
pub(super) struct PairSnapshot { pub a: SideSnapshot, pub b: SideSnapshot, pub repairs: Vec<Conflict>,
    pub conflicts: Vec<Conflict> }  // `plan()` und das zusammengeführte `omissions` entfallen (K3 plant in core)

// Jobs (crate::syncjobs)
pub const CURRENT_CONFIG_VERSION: u32 = 1;
pub struct SyncJob { /* bisherige Felder */ pub rt_max_latency_secs: u64 /* 0 = auto */,
    pub rt_poll_secs: u64 /* 300; 0 = nie */, pub verify_interval_secs: u64 /* 3600; 0 = nie */,
    pub verify_target_secs: u64 /* 86400; 0 = nie */, pub max_delete_min: u64 /* 25 */,
    pub versions_location: VersionsLocation /* Auto */, pub cross_mounts: bool /* neu false */,
    pub config_version: u32 /* neu CURRENT */, pub run_cleanup: String /* leer */ }   // max_delete_pct neu 50
impl SyncJob { pub fn effective_rt_max_latency_secs(&self) -> u64;   // Feld oder max(5 × rt_debounce_secs, 300)
    pub fn checked_glob_set_for(&self, case_insensitive: bool) -> Result<globset::GlobSet, String>; }
pub fn load_report() -> io::Result<JobLoadReport>;           // jede .conf einzeln; load() bleibt
pub struct JobLoadReport { pub jobs: Vec<SyncJob>, pub broken: Vec<BrokenJob> }          // Clone, Debug, Default
pub struct BrokenJob { pub id: String, pub path: PathBuf, pub error: String }          // id "" = Import/Eintrag
pub struct JobEditor { /* bisherige Felder */ pub rt_max_latency: String, pub rt_poll: String,
    pub verify_interval: String, pub verify_target: String, pub max_delete_min: String,
    pub versions_location: VersionsLocation, pub cross_mounts: bool, pub run_cleanup: String }
    // blank() = Standards von SyncJob::new (Prozent-Stopp "50", Mindestzahl "25")
```

Semantik, auf die sich andere Blöcke verlassen dürfen:
- **Job-Datei**: neue Schlüssel nach den alten: `config_version`, `rt_max_latency_secs`, `rt_poll_secs`,
  `verify_interval_secs`, `verify_target_secs`, `max_delete_min`, `versions_location` (`auto|appdata`),
  `cross_mounts` (0/1), `run_cleanup`. Eine Datei ohne `config_version` ist von vor RV1: `config_version` 0,
  `max_delete_pct`/`max_delete_min` wie gespeichert bzw. 0, `cross_mounts` 1. E-PLAN migriert beim Laden
  (`load`, `load_report`; im Speicher, gespeichert beim nächsten Sichern) auf 1: Prozent 0 → 50 % ab 25,
  ein gesetzter Prozentwert bleibt mit Mindestzahl 0 (Wirkung wie bisher); danach ist eine gespeicherte 0
  eine bewusste Wahl. Der Hash des Alt-Imports (`jobs.tsv`) deckt nur die alten Schlüssel ab.
- **Editor-Fehlertexte** neuer Felder (für `field_for_message` von AND-SYNC): „Höchstwartezeit“,
  „Abfrageintervall“, „Kontroll-Lauf“, „Vollprüfung des Ziels“, „Mindestzahl an Löschungen“ (jeweils
  „… enthält keine gültige nichtnegative Zahl.“).
- **Löschschutz**: Stopp bei `deletes > max_delete` (wenn > 0) oder bei `max_delete_pct > 0`,
  `deletes ≥ max_delete_min` und überschrittenem Anteil (Vertrag: so schon wirksam, Alt-Jobs mit 0 wie
  bisher). Ab E-PLAN: Anteil je Seite, verifiziertes Verschieben zählt nicht, Leere-Seite- und Replika-Prüfung
  unabhängig, Ergebnis `Outcome.blocked` statt Fehler „abgebrochen“; „Trotzdem ausführen“ =
  `RunSettings.confirmed = vec![block.confirmation()]` (persistierbar als `token()`).
- **Sperre**: `pair_lock_id` ist ungeordnet. Ab E-PLAN halten Läufe (`run_with`; `RunSettings.lock_wait`,
  sonst `Outcome.busy`), `resolve_recorded`, `apply_preview_action`, `versions::{restore,remove}_*` und
  `merge_baseline_entries` sie; Sperren in derselben App-Instanz schließen sich ebenfalls aus.
- **Zustände (Y150, B01)**: Läufe gespeicherter Jobs übergeben `RunSettings::for_job(job.id)`; ein neuer Job
  mit gleichen Endpunkten erbt nichts. Ad-hoc-Läufe nutzen den paarweiten Zustand. Der erste Joblauf nach RV1
  übernimmt die Altdatei `baseline_<pair>.sebl` (E-PLAN). Oberflächen lösen Konflikte nur noch über
  `resolve_recorded(.., &outcome.state ..)` und speichern keine Basis mehr selbst.
- **Apply (E-APPLY)**: Pfade je Seite `scope.spellings.side_rel(rel, side)`; `planned_a`/`planned_b` sind in der
  Schreibweise der jeweiligen Seite geschlüsselt. Jede fertige Aktion sofort `scope.sink.completed(..)` mit den
  beobachteten Signaturen (Quelle wie per capture/revalidate kopiert, Ziel per stat nach dem Veröffentlichen
  inkl. übertragener Zeit; Hash ≠ 0, wo die Seite gehasht wird); `durable` = Ersetzen mit fsync bzw. Fernziel
  bestätigt, `false` für neue lokale Linux/Android-Dateien mit `StageDurability::Deferred` (der Zwischenstand
  ruft vorher `vfs::sync_filesystem`). Geändert seit der Planung (Drift, nichts übernommen) →
  `sink.deferred`, kein Fehler; Ziel kann den Eintrag nicht halten → `sink.omitted(rel,
  TooLargeForTarget|NameImpossibleOnTarget)`; Ziel voll/nur lesbar/wiederholt nicht erreichbar →
  `sink.stopped(..)` und früh enden. Ordner: `Create` vor den Kopien hinein, `Remove` nach den Löschungen
  (tiefste zuerst, nur wenn leer) mit `CompletedKind::DirCreated/DirRemoved`. Versionen nur über
  `scope.versions`.
- **Walk (E-APPLY)** füllt `SideSnapshot`: `filtered` = alle durch Filter ausgelassenen Dateien (versteckt,
  Muster, Größe, Alter) mit Signatur und Hash 0; gefilterte Ordner werden nicht betreten
  (`OmissionKind::Filtered`, nicht melden); `dirs` = alle Ordner inkl. leerer, ohne Wurzel und ohne
  ausgelassene; Auslassungen: `Link` (melden, außer gefiltert), V1-Auslassungen über
  `OmissionKind::from(reason)`, `OwnFile` (`is_engine_name`, `vfs::is_staging_name`, App-Daten/-Cache/
  -Papierkorb; nicht melden), `SystemFolder` (`lost+found`, `System Volume Information`, `$RECYCLE.BIN` an
  Volume-Wurzeln, `Android/data|obb`), `Mount` (`cross_mounts == false` und `vfs::local_mount_boundary`).
  Grenzen aus `SyncLimits::for_memory(..)` statt `MAX_WALK_*`.
- **Versionen (E-APPLY)**: die Orchestrierung (K3) ruft `RunVersions::begin` vor apply und `finish` +
  `prune_after_run` nach jedem Lauf (auch nach Fehler/Abbruch) unter der Lauf-Sperre. Ändert E-APPLY die
  Sicherungs-Parameter von `copy_replace`/`delete_guarded*`, die `resolve.rs` (K3) nutzt: Anfrage an K3.
- **T-JOBS-Anfragen 1–7 (anfragen/T-JOBS.md)**: erledigt im Vertrag – `load_report` (1), `remove` entfernt
  auch `remove_job_state` (2), `run_cleanup` (3), `RunBlock`/`BlockConfirmation`/`RunSettings.confirmed`
  (4; Abbildung `MassDelete{side, deletions: deletes, total: files}`, `SideEmpty{side}`,
  `ReplicaMissing{side}`, `DeleteLimit` → `Other`), `RunRequest.observer` (5), Stabiles unverändert (6),
  Feldnamen wie angefragt (7): Kontroll-Lauf/Dienststart → `ScanDepth::VerifySources`, sonst
  `Incremental`; `Full` erzwingt E-PLAN selbst über `BisyncOptions::verify_target_secs`.
- **D-SYNCUI/AND-SYNC**: Läufe über `run_with(RunRequest { settings: RunSettings::for_job(..), .. })`,
  Ignore-Muster `job.checked_glob_set_for(pair_key_policy(..).fold_case)` (Y153), Konflikte
  `resolve_recorded`, „Nur diese Datei“ `apply_preview_action`, Versionen `bisync::versions`, Job löschen:
  `syncjobs::remove` + Angebot `forget_job_state` / `versions::remove_versions`, Blockade mit
  `message()` + „Trotzdem ausführen“.

Stand (Vertrag): Typen und Funktionen vorhanden, bestehende Wege unverändert bis auf: Standards neuer Jobs
(Prozent-Stopp 50 % ab 25 Löschungen; `cross_mounts`/Versionsort wirken erst mit E-APPLY), Prozent-Stopp
beachtet `max_delete_min` (Alt-Jobs 0 = wie bisher), `JobEditor::blank` aus `SyncJob::new`, `syncjobs::remove`
ruft `remove_job_state`. Stubs: `run_with` (= `run` + Altzustand in `Outcome.state`), `apply_preview_action`
(`Unsupported`), `versions` (App-Daten wie bisher; `list/restore/remove` `Unsupported`). Neue Datei außerhalb
der Liste: `bisync/os/shared/orchestration_full.rs` (Auslagerung aus `orchestration.rs`, 500-Zeilen-Grenze
nach rustfmt).

### V4 Jobs, Überwachung, Wachhalten (T-JOBS)
- `native/src/watch/` (core + `os/{windows,linux_os,android,shared}`): Überwachung je Wurzel mit Filter-
  Callback; Ereignisse (Pfad, Art) oder `Overflow`/`Unavailable(Grund)`; eine inotify-Instanz für alle;
  Windows-Handle-Freigabe beim Entfernen. Wird auch von H-ANALYSIS für `watch_v1` genutzt.
- `native/src/keep_awake/` (os-Adapter): `keep_awake::hold(Reason) -> KeepAwake` (RAII, gezählt; Windows
  Power Request System+Execution und Stromdrosselung aus; Linux logind-Sperre über `zbus`; Android über den
  vorhandenen Wakelock-Haken). Genutzt von T-JOBS (Läufe) und H-DISPATCH (fremde Ströme).
- Job-Zustandsdatei (`last_attempt`, `last_success`, `consecutive_failures`, `last_error`, `blocked`,
  `pending_trigger`) mit Sperre gegen verlorene Schreibvorgänge; Problemzustand für Benachrichtigungen.

Signaturen (T-JOBS, eingetragen; die Stubs ändern kein bestehendes Verhalten, Stand unten):

**`crate::watch`** (neu; Nutzer: Hintergrunddienst, H-ANALYSIS `watch_v1`; Android-Host über AND-SYNC)
```rust
pub fn watch(root: &Path, options: WatchOptions, filter: WatchFilter, sink: WatchSink) -> io::Result<WatchHandle>;
pub struct WatchHandle;            // fn id(&self) -> WatchId; fn root(&self) -> &Path; Drop beendet die Überwachung
pub struct WatchId;                // Copy, Eq, Hash, Ord; fn get(self) -> u64
pub struct WatchOptions { pub cross_mounts: bool }                       // Default: false
pub struct WatchEntry<'a> { pub rel: &'a str, pub is_dir: Option<bool> }
pub struct WatchFilter;            // fn new(impl Fn(&WatchEntry<'_>) -> bool + Send + Sync + 'static) -> Self;
                                   // fn all() -> Self (= Default); fn admits(&self, &WatchEntry<'_>) -> bool
pub enum WatchSink { Channel(crossbeam_channel::Sender<WatchMessage>), Callback(Arc<dyn Fn(&WatchMessage) + Send + Sync>) }
                                   // fn callback(impl Fn(&WatchMessage) + Send + Sync + 'static) -> Self; From<Sender<_>>
pub struct WatchMessage { pub id: WatchId, pub event: WatchEvent }
pub enum WatchEvent { Ready(Coverage), Change(Change), Overflow, Unavailable(UnavailableReason) }
pub struct Change { pub rel: String, pub kind: EventKind, pub is_dir: Option<bool> }
pub enum EventKind { Created, Removed, Modified, Metadata, RenamedFrom, RenamedTo, Unknown }
pub enum Coverage { Complete, LocalOnly, SharedStorage }
pub enum UnavailableReason { WatchLimit, Unsupported, RootMissing, DeviceRemoved, AccessDenied, Failed(String) }
pub fn report_host_change(paths: &[PathBuf]);
pub fn set_host_cursor(scope: &Path, cursor: Option<String>);
pub fn host_cursor(path: &Path) -> Option<String>;
```
- `watch` kehrt sofort zurück (Fehler nur `InvalidInput` bei relativer Wurzel); scharf geschaltet wird im
  Dienst-Thread, gemeldet wird `Ready(coverage)` oder `Unavailable(grund)`, danach Änderungen. `Ready` nach
  `Unavailable` und `Overflow` heißen „dazwischen unbekannt → Wurzel neu prüfen“; vor dem ersten `Ready` ist
  nicht alles gemeldet (Start = Kontroll-Lauf). `rel` relativ zur Wurzel mit `/`, `""` nur bei `Unknown` für
  die Wurzel selbst; nicht-UTF-8-Namen verlustbehaftet (Ereignisse sind nur Auslöser).
- Filter und Callback laufen im Dienst-Thread (kurz, keine E/A, darin kein `watch`/Drop); `false` für ein
  Verzeichnis schließt den Teilbaum aus. Kanal-Senke: `try_send`; voll → später genau ein `Overflow`,
  getrennt → Überwachung endet. Nach dem Drop können noch Nachrichten der ID ankommen (ignorieren).
- Fest eingebaut: App-Daten- und Cache-Ordner nie beobachtet/gemeldet; Links/Junctions unter der Wurzel nie
  verfolgt; Linux/Android ohne `cross_mounts` keine fremden Einhängungen. Sync-eigene Namen (`.se-versions`,
  `.se-sync-replica`, Zwischendateien) schließt der Filter des Nutzers aus.
- Wiederanlauf durch den Dienst: `WatchLimit`/`AccessDenied`/`Failed` mit Backoff (1 min, verdoppelt bis
  60 min), `RootMissing`/`DeviceRemoved` bei Wiederkehr; `Unsupported` endgültig. `Coverage::LocalOnly`
  (Linux-Netz-/FUSE-Einhängungen) verlangt zusätzlich Abfrage, `SharedStorage` (Android) Host-Signale und
  Kontroll-Läufe.
- Host-Signale: `report_host_change` meldet jeder Wurzel, die einen Pfad enthält oder darunter liegt,
  `Change { kind: Unknown }` (gefiltert); `set_host_cursor`/`host_cursor` vergleichen Pfade komponentenweise
  (kanonisch übergeben), der längste Bereich gewinnt (Android: `"<MediaStore-Version>:<Generation>"` je Volume).

**`crate::keep_awake`** (neu; Nutzer: Hintergrunddienst, manuelle Läufe D-SYNCUI/AND-SYNC, H-DISPATCH fremde
Ströme, A-CLIENT Fern-Tasks)
```rust
pub enum Reason { SyncRun, PeerService, RemoteTask }   // const ALL: [Reason; 3]; fn label(self) -> &'static str
#[must_use] pub struct KeepAwake;                       // fn reason(&self) -> Reason; Drop gibt frei
pub fn hold(reason: Reason) -> KeepAwake;
pub fn status() -> KeepAwakeStatus;
pub struct KeepAwakeStatus { pub holds: Vec<(Reason, usize)>, pub engaged: bool, pub throttling_off: bool,
                             pub unavailable: Option<String> }
```
- `hold` blockiert und scheitert nie (sicher auch auf Tokio-Threads; die OS-Anforderung stellt ein eigener
  Thread), gezählt je Grund, frei beim Drop des letzten Halters. Windows: Power Request SystemRequired +
  ExecutionRequired je Grund, EcoQoS aus solange ein Halter lebt; Linux: logind `Inhibit("idle:sleep",
  "Smart Explorer", label, "block-weak")` über `zbus` (endet mit dem Prozess); bei älterem logind
  oder fehlender Berechtigung nur `idle`/`block`; Android: `share::power::request_hold`
  (60 s, alle 30 s und sofort beim Wechsel in den Niedrigenergiebetrieb erneuert). Nutzer-Schlaf (Deckel,
  Taste) wird nie verhindert; Probleme stehen in `status().unavailable`.

**Job-Zustand in `crate::syncjobs`** (Datei `<sync_data_dir>/job-state/<id>.json`, Sperrdatei `<id>.lock`;
Nutzer: Hintergrunddienst, D-SYNCUI, AND-SYNC, `app/core/landing.rs`)
```rust
pub struct JobState { pub version: u32, pub last_attempt: Option<i64>, pub last_runner: Option<Runner>,
    pub last_cause: Option<RunCause>, pub last_success: Option<i64>, pub consecutive_failures: u32,
    pub last_error: Option<JobError>, pub blocked: Option<Blocked>, pub pending_trigger: Option<PendingTrigger>,
    pub retry_at: Option<i64>, pub last_result: Option<JobResult>, pub watch: Option<WatchStatus>,
    pub running: Option<RunMark>, pub last_verify: Option<i64>, pub verify_cursor: Option<String>,
    pub last_connect: Option<ConnectMark>, pub notified: Option<Notified>,
    /* nicht gespeichert */ pub load_error: Option<String> }
    // fn running_now(&self, now: i64) -> Option<&RunMark>; fn problem(&self) -> Option<ProblemKind>
pub struct JobError { pub kind: FailureKind, pub message: String }
pub enum FailureKind { Config, Unreachable, Auth, Access, Hook, Run, TargetFull, Internal, Other } // fn needs_user(self) -> bool
pub struct Blocked { pub kind: BlockKind, pub detail: String, pub since: i64, pub confirmed: bool }
pub enum BlockKind { MassDelete { side: JobSide, deletions: u64, total: u64 }, SideEmpty { side: JobSide },
                     ReplicaMissing { side: JobSide }, Other }
pub enum JobSide { A, B }                                        // A = SyncJob::source, B = SyncJob::target
pub struct PendingTrigger { pub kind: PendingKind, pub since: i64, pub volume: Option<String> }
pub enum PendingKind { Change, Connect, Startup, Verify, Confirmed, Other }
pub struct WatchStatus { pub detection: ChangeDetection, pub since: i64, pub note: Option<String> }
pub enum ChangeDetection { Starting, Events, EventsAndPoll { poll_secs: u64 }, Poll { poll_secs: u64 }, Other }
pub struct RunMark { pub runner: Runner, pub cause: RunCause, pub started: i64, pub alive: i64, pub stalled_since: Option<i64> }
pub enum Runner { Daemon, Desktop, Android, Cli, Other }
pub enum RunCause { Manual, Interval, Calendar, Change, Poll, Verify, Startup, Connect, Retry, CatchUp, Confirmed, Other }
pub struct ConnectMark { pub volume: String, pub seen: i64, pub session: Option<String> }   // dienstintern
pub struct Notified { pub key: String, pub at: i64 }                                       // dienstintern
pub struct AttemptReport { pub runner: Runner, pub cause: RunCause, pub started: i64, pub finished: i64,
                           pub outcome: AttemptOutcome, pub result: Option<JobResult> }
pub enum AttemptOutcome { Success, Failed(JobError), Cancelled, Blocked(Blocked) }
pub struct ProblemNotice { pub job_id: String, pub job_name: String, pub kind: ProblemKind, pub title: String, pub text: String }
pub enum ProblemKind { Blocked, NeedsAction, FailureSeries }
pub const JOB_STATE_VERSION: u32 = 1; pub const RUN_MARK_STALE_SECS: i64 = 180; pub const FAILURE_SERIES_MIN: u32 = 3;
pub fn load_job_state(id: &str) -> io::Result<JobState>;
pub fn load_job_states(jobs: &[SyncJob]) -> BTreeMap<String, JobState>;
pub fn update_job_state(id: &str, change: impl FnOnce(&mut JobState)) -> io::Result<JobState>;
pub fn record_attempt(id: &str, report: &AttemptReport) -> io::Result<JobState>;
pub fn confirm_block(id: &str, kind: &BlockKind) -> io::Result<JobState>;
pub fn remove_job_state(id: &str) -> io::Result<()>;
pub fn take_problem_notices(jobs: &[SyncJob], now: i64) -> Vec<ProblemNotice>;
pub fn classify_run(out: &bisync::Outcome, canceled: bool, finished: i64) -> (AttemptOutcome, JobResult);
```
- `classify_run` ist die eine Einordnung für Dienst, Desktop und Android (schon umgesetzt): `canceled` (eigener
  Abbruch) → `Cancelled`; Engine-Stopp (bisher Fehlerart „abgebrochen“) → `Blocked(Other)` statt Abbruch, bis
  E-PLAN den Stopp typisiert; Fehler → `Failed(Run)` mit `stats.errors` (nicht die gekappte Liste); sonst
  `Success`. Das `JobResult` trägt Notiz und Zähler wie bisher.
- `JobResult` erhält zusätzlich `PartialEq, Eq, Serialize, Deserialize` (Felder unverändert); `load_results`,
  `record_result`, `mark_run` bleiben. Zeiten sind Unix-Sekunden; Aufzählungen und Felder neuerer Versionen
  werden toleriert (`Other`, Standardwerte).
- Schreiben nur über `update_job_state`/`record_attempt`/`confirm_block`: exklusive Datei-Sperre je Job
  (prozess- und threadübergreifend, begrenzte Wartezeit → `TimedOut`), dann atomares Ersetzen; Lesen ohne
  Sperre. Fehlt die Datei: Startwert aus `results.tsv` (`last_result`, `last_attempt`) und
  `SyncJob::last_run` (`last_success`); unlesbar: einmal beiseitegelegt, leerer Zustand mit `load_error`.
- Planung zählt ab `last_success`; der Dienst schreibt `last_run` in der `.conf` nicht mehr (`mark_run` bleibt
  Altweg). Nach der Umsetzung trägt `record_result` den Versuch zusätzlich ein (Einordnung aus Notiz/Fehlern)
  und `load_results` liest aus den Zuständen; Anzeigen stellen auf `JobState` um.
- `record_attempt`: immer `last_attempt = started`, `last_runner`, `last_cause`, `last_result = result` (wenn
  `Some`), die Laufmarke dieses Läufers entfällt. `Success`: `last_success = finished`, Fehlerserie 0,
  `last_error`/`blocked`/`retry_at` leer, `pending_trigger` entfällt bei `since <= started`. `Failed(e)`: Serie
  +1, `last_error = e`, `retry_at` = Backoff (nicht bei `e.kind.needs_user()`), Auslöser bleibt. `Cancelled`:
  sonst nichts, Auslöser bleibt. `Blocked(b)`: `blocked = b` (unbestätigt), Auslöser entfällt, keine
  automatische Wiederholung.
- `confirm_block` setzt `blocked.confirmed` und `pending_trigger = Confirmed`, nur wenn `kind` der gezeigten
  Sperre gleicht (sonst `InvalidInput`); der nächste Lauf darf genau diesen Stopp einmal passieren.
  `running_now`: Marke jünger als `RUN_MARK_STALE_SECS`. `problem()`: `Blocked` vor `NeedsAction`
  (`Config`/`Auth`/`Access`) vor `FailureSeries` (ab `FAILURE_SERIES_MIN`). `take_problem_notices` drosselt
  je Job und Problem (merkt `notified`).

**`crate::daemon`** (Ergänzungen) und **`crate::autostart`**
```rust
pub fn set_problem_notifier(notifier: fn(&ProblemNotice));  // Host-Ziel (Android: Kanal „Sync-Probleme“); ohne → Desktop-Systembenachrichtigung
pub fn next_scheduled_run(now: i64) -> Option<i64>;          // nächster uhrgesteuerter Termin (>= now) für exakte Android-Alarme
pub fn autopause_support() -> AutopauseSupport;              // pub struct AutopauseSupport { pub battery_saver: bool, pub metered: bool }
pub fn set_storage_access(granted: bool);                    // Android-Allzugriff; ohne ihn laufen Jobs mit lokalem
pub fn storage_access() -> Option<bool>;                     //   Speicher-Endpunkt nicht („Dateizugriff fehlt“); None = nicht gemeldet
pub fn last_catch_up() -> Option<CatchUpRecord>;             // pub struct CatchUpRecord { pub finished_ms: i64, pub ran: usize,
                                                             //   pub succeeded: usize, pub failed: usize, pub message: String }
pub enum HookPhase { Before, After, Cleanup }
pub fn run_job_hook(job: &SyncJob, phase: HookPhase, outcome: Option<&AttemptOutcome>, cancel: &AtomicBool) -> Result<(), String>;
// CatchUpStatus additiv: pub failed: usize, pub retry_suggested: bool (vorübergehender Fehler → WorkManager retry)
pub fn autostart::disabled_by_system() -> bool;              // Windows StartupApproved\Run, Linux Hidden=true/X-GNOME-Autostart-enabled=false
```
- `run_job_hook`: ein Weg für Dienst und manuelle Läufe; Windows `cmd /d /s /c "<Befehl>"` ohne Konsole,
  Abbruch über `cancel`, Umgebungsvariablen mit Job und Ergebnis; `Before` vor dem Öffnen der Seiten,
  `After` nach jedem beendeten Lauf außer Abbruch, `Cleanup` nach Abbruch/Startfehler (Feld von K3 angefragt).
- Stand der Stubs: `watch` meldet `Unavailable(Unsupported)`; `keep_awake` zählt nur; Zustandsfunktionen
  lesen leer und schreiben `Unsupported`; `next_scheduled_run`/`last_catch_up` = `None`;
  `CatchUpStatus.failed = 0`, `retry_suggested = false`; `disabled_by_system` = `false`; `run_job_hook` wie
  bisher (`Cleanup` ohne Wirkung); `autopause_support` meldet schon den echten Stand (Windows/Android ja,
  Linux nein); `set_storage_access`/`storage_access`, `set_problem_notifier`, `set_host_cursor`/
  `host_cursor` arbeiten bereits. Linux-Wachhalten über `zbus` gilt laut B10 (spec „Plattform-Prüfung“ und
  recherche E11 nennen noch `systemd-inhibit`, überholt).

### V5 Beziehungen und Rechte (S-REVOKE)
- `types.rs`: Schreibrecht je Direkt-Freigabe (`DirectGrant.write`, neue Grants false, Altdaten true) und je
  Raum; Zustand „neu bestätigen“ (statt „ignoriert“) für Code-Rotation/Identitätsreparatur; Sperr- und
  Entfernungseinträge mit Schlüssel + Knoten; Raum-Merkmal „neue Mitglieder bestätigen“; Wahl
  „Gegenseitig“ beim Code-Hinzufügen/Koppeln (Standard aus).
- Ergebnis der Autorisierung einer Sitzung trägt `may_write`; H-DISPATCH setzt `may_write &&
  root.access == ReadWrite` durch.
- Invalidierung: Ereignis „Recht eingeschränkt für (Schlüssel, Beziehung)“; Präsenz/Laufzeitfelder lösen
  kein `ConfigureProfiles` aus (FA3-Pflichtteil im Daemon-Ereignisweg).

Signaturen (S-REVOKE, eingetragen; die Stubs ändern kein bestehendes Verhalten). Neue Dateien im Besitz von
S-REVOKE: `share/core/direct_relation.rs` (Direkt-Typen aus `types.rs` hierher verlegt; alle bisherigen Pfade
`super::types::…` und `crate::share::…` bleiben per `pub use`), `share/core/relation_rights.rs`,
`share/core/signal_commands_local.rs` (Untermodul von `signal_commands.rs`). Konstruktionsstellen fremder
Dateien: `anfragen/S-REVOKE.md` R1 (erst danach kompiliert der Vertrag).

**Rechte und Zustände** (`crate::share::…`, Felder additiv mit `serde`-Standard)
```rust
pub struct DirectGrant { /* bisherige Felder */
    #[serde(default = "legacy_relation_write")] pub write: bool }  // Altdaten true, jede neue Freigabe false
pub enum DirectGrantState { Accepted, Ignored, Reconfirm }          // Reconfirm = „neu bestätigen“
pub struct DirectContact { /* bisherige Felder */ #[serde(default)] pub relation: DirectRelationFlags }
pub struct DirectRelationFlags { pub share_back: bool, pub signed_presence: bool }      // Default: beide false
pub enum DirectRequestPolicy { #[default] Ask, AutoAccept }
pub struct RoomProfile { /* bisherige Felder */
    #[serde(default = "RoomPolicy::legacy")] pub policy: RoomPolicy }
pub struct RoomPolicy { pub members_may_write: bool, pub confirm_new_members: bool }  // kein Default
    // fn new_room() -> Self (false, false); pub(crate) fn legacy() -> Self (true, false)
pub struct RoomMember { /* bisherige Felder */ #[serde(default)] pub relation: RoomMemberFlags }
pub struct RoomMemberFlags { pub admission: RoomMemberAdmission, pub signed_presence: bool }  // Default
pub enum RoomMemberAdmission { #[default] Admitted, Pending }
impl DirectGrant { pub(crate) fn authorizes_session(&self, device_id: &str, public_key: &str, node_id: &str) -> bool }
impl RoomMember {
    pub(crate) fn is_admitted(&self) -> bool;                       // !blocked && admission == Admitted
    pub(crate) fn authorizes_session(&self, device_id: &str, public_key: &str, node_id: &str) -> bool }
impl RoomProfile {
    pub(crate) fn requires_member_confirmation(&self) -> bool;
    pub fn set_member_blocked(&mut self, device_id: &str, blocked: bool, now: i64) -> bool;
    pub fn admit_member(&mut self, device_id: &str) -> bool }
impl ShareProfiles {
    pub fn set_direct_grant_write(&mut self, device_id: &str, write: bool, now: i64) -> Result<bool, String> }
pub enum PairingOrigin { UserPairing, UserPairingOneWay, AutomaticRepair }  // fn is_user(self) -> bool
```
- `write`: „Darf schreiben“ der Direkt-Freigabe; neue Freigaben (Ledger-Projektion, Legacy-Annahme, Reparatur,
  `ShareProfiles::set_direct_grant`) starten mit `false`. Raum: `RoomPolicy.members_may_write`; jeder neu
  angelegte oder beigetretene Raum (UI, Code, PIN) nimmt `RoomPolicy::new_room()`.
- `Reconfirm` ersetzt `Ignored` nach Code-Rotation/Identitätsreparatur (Umstellung in S-REVOKE): kein
  Nutzerverbot; reaktiviert durch eine mit dem aktuellen Code authentisierte Anfrage derselben Identität
  (Daemon-Entscheidung `Accepted`, schon umgesetzt), eine bewusste Kopplung (`UserPairing`) oder den Nutzer;
  automatische Reparatur nie; Exec bleibt dabei aus. Exhaustive `match` auf `DirectGrantState` gibt es nur in
  S-REVOKE-Dateien.
- `PairingOrigin`: `UserPairing` = bewusste Kopplung, die die eigenen Freigaben öffnet (PIN-Anbieter;
  Verbindender mit „Auch meine Freigaben für dieses Gerät öffnen“); `UserPairingOneWay` = bewusste Kopplung
  ohne eigene Freigabe (Standard des Verbindenden; legt keine Freigabe an, reaktiviert keine);
  `AutomaticRepair` öffnet nur bei `relation.share_back` oder bestehender angenommener Freigabe (Umstellung in
  S-REVOKE). Code-Hinzufügen speichert die Wahl in `DirectContact.relation.share_back` (Standard `false`).
- Raum-Bestätigung (B15): Invariante `admission == Pending ⇒ blocked == true` – jede vorhandene Prüfung von
  `blocked` verweigert damit auch wartende Mitglieder. `set_member_blocked(.., true, ..)` schaltet Exec des
  Mitglieds aus und `confirm_new_members` ein; `set_member_blocked(.., false, ..)` und `admit_member` lassen zu.
  `requires_member_confirmation()` gilt auch für Räume mit einem schon vor V5 gesperrten Mitglied.
- `signed_presence`: B03-Rückstufungsschutz; setzt nur der Daemon-/Worker-Weg (S-REVOKE).
- `DirectRequestPolicy`: Feld `ShareProfiles.direct_request_policy` (`#[serde(default)]`, auch Altdaten `Ask`)
  per Anfrage R2 an S-POLICY; nur der Daemon liest es (automatisches Annehmen nur bei `AutoAccept`).
- Sperr-/Entfernungseinträge mit Schlüssel + Knoten: `RemovedDirectPeer` trägt `public_key`, `node_id`,
  `fingerprint` bereits; `matches` vergleicht in S-REVOKE zusätzlich Schlüssel und Knoten. Raum-Sperren sind
  gesperrte `RoomMember` mit Schlüssel und Knoten; neue Geräte-IDs mit gesperrtem Schlüssel/Knoten werden nicht
  zugelassen (S-REVOKE).

**Autorisierung und Invalidierung** (`share/core/relation_rights.rs`, crate-intern)
```rust
pub(crate) struct SessionAuthorization { pub(crate) exports: ShareExportConfig, pub(crate) may_write: bool }
impl SessionAuthorization {
    pub(crate) fn direct(exports: ShareExportConfig, grant: &DirectGrant) -> Self;  // may_write = grant.write
    pub(crate) fn room(room: &RoomProfile) -> Self }      // exports = room.exports, may_write = members_may_write
pub(crate) enum RelationScope { Direct, Room { room_id: String } }
pub(crate) struct PrincipalKey { pub(crate) public_key: String, pub(crate) node_id: String }
    // fn matches(&self, public_key: &str, node_id: &str) -> bool  (Schlüssel ODER Knoten, leer passt nie)
pub(crate) enum RestrictionReason { Removed, Blocked, Reconfirm, IdentityChanged, WriteRevoked, ExecRevoked,
    ExportsNarrowed, RelationInactive, Unattributed }
pub(crate) struct RightsRestriction { pub(crate) relation: RelationScope,
    pub(crate) principal: Option<PrincipalKey>, pub(crate) reason: RestrictionReason }   // None = alle der Beziehung
    // fn affects(&self, relation_kind: &str, relation_id: &str, public_key: &str, node_id: &str) -> bool
pub(crate) struct RestrictionSet;  // fn everything(RestrictionReason) -> Self; fn push(&mut self, RightsRestriction);
    // fn merge(&mut self, RestrictionSet); fn is_empty(&self) -> bool; fn everything_reason(&self) -> Option<RestrictionReason>;
    // fn items(&self) -> &[RightsRestriction]; fn affects(&self, kind: &str, relation_id: &str, public_key: &str, node_id: &str) -> bool
pub(crate) fn authorization_restrictions(current: &ShareAuthState, candidate: &ShareAuthState) -> RestrictionSet;
```
- H-DISPATCH: `IncomingSession::authorize`/`authorize_state` liefern `SessionAuthorization` statt
  `ShareExportConfig` (Direkt: `DirectGrant::authorizes_session` + Fingerprint + Proof →
  `SessionAuthorization::direct(state.default_direct_exports.clone(), grant)`; Raum: Mitgliedssuche zusätzlich
  `is_admitted()` → `SessionAuthorization::room(room)`); jede schreibende Anfrage nur bei
  `may_write && root.access.allows_write()` (unter `/Verbindungen`: `SharedConnection.access`).
- `authorization_restrictions` ersetzt `configuration_changed`: `relation_kind` `"direct"`/`"room"` wie in
  `PeerHello`, `relation_id` = Raum-ID (bei Direkt ohne Bedeutung, Schlüssel/Knoten genügen in beide
  Richtungen). Leer = nichts schließen, keine neue Epoche (Präsenz, Laufzeitdaten, neue Mitglieder, jede
  Erweiterung gilt sofort für neue Sitzungen); `everything_reason()` = global wie bisher; sonst genau die
  getroffenen Sitzungen, Leases und Übertragungen. Vertragsstand: konservativ (jede autorisierungsrelevante
  Abweichung = alles, Laufzeitfelder nie); die genaue Fassung (Freigabe weg/enger, Schreibrecht weg, Sperre,
  Entfernen, Identitätswechsel, Raum inaktiv/Mitglied gesperrt, Direkt offline) liefert S-REVOKE ohne
  Signaturänderung.
- Exec (B04): H-DISPATCH ruft bei jeder übernommenen Änderung weiter
  `exec_grant_runtime::apply_configuration_transition(&state, &candidate, epoch, registry)` vor dem Ersetzen
  (Signatur bleibt). S-REVOKE stellt um: kein Abbruch aller Exec-Jobs je Epochenwechsel mehr, sondern genau der
  von `authorization_restrictions` getroffenen Prinzipale und abgeschalteter Richtlinien. Reparaturen (S66)
  regelt S-REVOKE in eigenen Dateien (Erlaubnis nur für den Speicherschritt, erneute Prüfung darunter).

**Laufzeitdaten ohne Konfigurationsübergang (FA3-Pflichtteil)**
```rust
pub enum ShareCmd { /* bisherige */ UpdateRuntime { runtime: Box<RelationRuntime> } }   // daemon-intern
pub struct RelationRuntime { pub contacts: Vec<DirectContactRuntime>, pub rooms: Vec<RoomRuntime> }
    // pub fn from_profiles(profiles: &ShareProfiles) -> Self
pub struct DirectContactRuntime { pub contact_id: String, pub status: ShareStatus, pub last_seen: Option<i64>,
    pub last_error: Option<String>, pub presence: Option<PeerPresence>, pub lan_candidates: Vec<String>,
    pub lan_seen_at: Option<i64>, pub lan_uplink: Option<bool> }
pub struct RoomRuntime { pub room_id: String, pub status: ShareStatus, pub last_seen: Option<i64>,
    pub members: Vec<RoomMember> }
impl ShareAuthState { pub(crate) fn apply_runtime(&mut self, runtime: &RelationRuntime) -> bool }
```
- Der Daemon schickt für reine Laufzeitänderungen (Präsenz, Status, `last_seen`, LAN-Routen, Mitgliedsrouten
  und -namen, neu gesehene Raum-Mitglieder) `UpdateRuntime` statt `ConfigureProfiles`; der Worker übernimmt sie
  ohne `begin_runtime_transition`, ohne Epoche und ohne Invalidierung und plant danach Reparaturen
  (`schedule_current`). Bekannte Mitglieder (gleiche Geräte-ID, Schlüssel, Knoten) erhalten nur Laufzeitfelder,
  unbekannte werden mit Exec aus ergänzt; Rechte, Pins, Freigaben und Raum-Richtlinie ändert der Weg nie.
  IPC-Clients dürfen `UpdateRuntime` nicht senden (abgewiesen wie `ConfigureProfiles`).
- `ConfigureProfiles` bleibt für alles Übrige und trägt weiter Laufzeitfelder; deshalb ignoriert
  `authorization_restrictions` sie.

**Signierte Präsenzen (B03, Draht unverändert)**: die Signatur reist im Feld `nonce`
(`"<Zufall>.ps1.<Ed25519-Signatur, base64url>"`, ≤ 128 Byte, Zeichen `[A-Za-z0-9_.-]`), weil Server jedes
Präsenz-Objekt typisiert neu schreiben und unbekannte Felder verwerfen. Der HMAC deckt wie bisher den ganzen
Nonce (alte Empfänger prüfen unverändert), die Signatur eine längenpräfixierte Nutzlast inkl. `device_name` und
`fingerprint`. Server und Clients behandeln den Nonce weiter undurchsichtig; die Server-Grenze 256 Byte genügt.

## Blöcke und Besitz

| Block | Inhalt | Besitz (exklusiv) | Start | Fertig, wenn |
|---|---|---|---|---|
| K1 → V-LOCAL | V1; FS6, lokale Teile FS4/FS5/FA7 (tolerante Listen, Dauerhaftigkeit, NOREPLACE-Leiter, mkdir_all-Wurzel, Windows-Namen inkl. reservierte, Nur-lesen-Ersetzen, Rechte, Reparse-Klassen, sicheres Öffnen) | `native/src/{vfs,local_access,copy,android_fs,types}/**`, `native/src/transfer/os/shared/walk_listers.rs`; VfsMeta-Zeilen überall (nur bis „V1-VfsMeta fertig“) | Welle 1 | V1 eingetragen; Y86/Y87/Y94/Y95/Y98/Y100/Y103/Y122/Y99/Y81, A24-lokal, B24/B25 umgesetzt; Tests je Plattform |
| K2 → H-ANALYSIS | V2; FA2/FA4/FA5/FA6/FA7 Host- und Peer-Client-Seite, `watch_v1`-Host | `native/src/share/core/{wire,fs_request,fs_response,fs_error,server_capabilities,storage_*,peer_*,walk_assembly,framing,backend,backend_tests,host_requests,export_config}.rs` (export_config nur bis Vertrag), neue `share/core/{duplicate_*,hash_walk*,list_batch*,remote_trash*,analysis_*,watch_*}.rs`, `native/src/share/os/shared/storage_analysis_host.rs`, `native/src/analytics/core/**`, `native/src/analytics/os/shared/{analytics,analytics_budget,analytics_outcome}.rs`, `native/src/analytics/os/shared/reclaim/{finder*,verify,duplicates,local,stage,types,util,retention,budget,cleanup,mod}.rs` | Welle 1 (fremde VfsMeta-Dateien erst nach „V1-VfsMeta fertig“) | V2 eingetragen; Host-Duplikate/Hash-Walk/Listen-Portionen/Papierkorb/Watch/Kompression/Wiederanbindung/Berichtsfelder; FA4-Punkte; Tests |
| K3 → E-PLAN | V3; Paarplanung, Replika-/Owner-Basis, Checkpoints, Index und Migration | Exakte Quellen in `scopes/e-plan.json`; Apply/Snapshots gehen danach an E-APPLY | Welle 1, Quellen `dd0dccf` abgeschlossen | Planung und Baseline schützen Teilscans, fremde Replika und fehlgeschlagene Aktionen; Bestätigung in der einzigen Remote-Suite |
| A-CLIENT | FA1 sofort (scan_remote, resolve_live, Phasen, Hinweise, Ergebnisse freigeben, Wakelock), danach FA2/FA6-Clients, Daemon-Hash-Walk-Korrekturen, A07/B26, Agent-/IPC-Weiterreichung neuer Haken | `native/src/mobile/os/shared/domains/{analyze,analyze_platform}.rs`, `native/src/analytics/os/shared/{remote,analytics_backend}.rs`, `native/src/analytics/os/shared/reclaim/{backend,backend_duplicates}.rs`, `native/src/app/core/{analytics_*,reclaim_*}.rs`, `native/src/daemon/os/shared/{ipc,ipc_protocol,ipc_protocol_bounds,ipc_client,ipc_analysis,backend_server,backend_walk,backend_budget,backend_batch,backend_stream,backend_transfer,backend_tree_send,request_workers}.rs`, `native/src/agent/**`, `native/src/agent_proto/**`, Kotlin `ui/analytics/**`, `api/AnalyzeApi.kt`, `service/{TaskForegroundService,TaskKeeper}.kt` | Welle 1 (FA1 zuerst; Rest nach V2) | Android analysiert Share-Orte auf dem Host; Duplikate host-seitig mit sicherem Rückfall; Tests über `mobile::call` |
| T-JOBS | V4; FS8, FS9, FS10 (Daemon), Y146, Y151 (Rotation, Ergebnisse, Benachrichtigung Desktop), B08/B10/B11/B19/B22/B27/B32 | `native/src/daemon/os/shared/{run_loop,schedule,state,catch_up,job,job_supervisor,host_state,live,boot_marker,handoff,embedded}.rs`, `native/src/daemon/os/{windows,linux_os,android}/platform.rs`, `native/src/daemon/mod.rs`, neue `native/src/{watch,keep_awake,notify_desktop}/**`, `native/src/syncjobs/os/shared/results.rs`, neue `syncjobs/os/shared/job_state*.rs` und `syncjobs/os/{linux_os,windows,android}/job_state_lock.rs`, neue Ausgliederungen von `run_loop.rs` unter `daemon/os/shared/`, `native/src/syncjobs/core/schedule.rs`, `native/src/autostart/**`, `native/src/lib.rs`/`main.rs` (nur Modul-Einträge und Wächter-Einstieg) | Welle 1 (V4 zuerst) | Echtzeit per Ereignissen + Abfrage + Kontroll-Läufe; Wächter; Benachrichtigung; Anschluss-Erkennung Linux/Windows; Tests |
| S-SIGNAL | FC2 + FC3 + FC4 (Client und Server), B20, B21 | `native/src/share/core/{discovery_*,signal_connection,signal_connector,signal_handshake,signal_session,signal_connected,signal_worker*,signal_schedule,signal_publish,signal_subscriptions,signal_readiness,signal_idle,signal_power,endpoint_routes,service}.rs`, `native/src/share/os/shared/{discovery_*,transport_options}.rs`, `share-server/**`, `vendor/iroh-relay-1.0.0/**` (nur falls nötig), `native/src/app/core/{share_discovery_*,menus_settings}.rs`, `native/src/cli/share.rs`, `native/src/cli/share/discoverable*.rs`, `native/src/mobile/os/shared/domains/share_settings.rs`, `docs/SHARE_SERVER.md` | Welle 1 | TLS-Standard Client/Server, Opt-in, Migration, Anmeldung mit Schlüssel, Bindungen, DoS-Grenzen, PIN-Regeln; Tests inkl. gemischter Versionen |
| S-REVOKE | V5; FC5, FA3-Pflichtteil im Daemon-Ereignisweg, Exec-Invarianten (B04), Raum-Bestätigung (B15), signierte Präsenzen/Entscheidungen (B03) | `native/src/share/core/{direct_*,legacy_direct_*,removed_direct_peers,identity,identity_repair,crypto,signal_auth,signal_presence,signal_commands,signal_commands_local,relation_rights,tracked_signal_*,types,exec*,room_relation}.rs`, `native/src/share/os/shared/{direct_*,legacy_direct_actions,removal,identity_store,lifecycle_view}.rs`, `native/src/share/os/*/exec*.rs`, `native/src/daemon/os/shared/{ipc_host,ipc_host_*,exec_*}.rs`, `native/src/app/core/{share_direct_ui,share_lifecycle_*,share_legacy_lifecycle_ui,share_removal_ui,share_removed_devices_ui,share_identity_rotation,share_exec*}.rs`, `native/src/cli/share/{requests*,grants*,request_selection,lifecycle_output,exec_status}.rs`, `native/src/cli/exec.rs`, `native/src/mobile/os/shared/domains/{share_requests,share_exec}.rs` | Welle 1 (V5 zuerst) | Entziehen per Schlüssel, Bestätigung neuer Geräte, Rotation ohne Aussperren, signierte Präsenzen, Exec-Invarianten; Tests |
| V-REMOTE | FS7, V1 für SFTP/FTP/WebDAV/SMB/Drive inkl. `change_signal` (Nextcloud-ETag, Drive-Feed) | `native/src/{sftp,ftp,webdav,smb,gdrive,connect}/**` | Welle 2 (nach V1) | Y114…Y143 (Fernziel-Teil) umgesetzt; Tests (Container-Stufen als Suite-Stufe) |
| E-APPLY | FS4-Walk, FS5, FS2-Apply, Schnellspiegel, Y145/Y152/B17/Y154/Y155/Y54/Y149-Kern, Versionsordner (B31), Dauerhaftigkeit (B18) | `native/src/bisync/os/shared/{snapshot*,apply*,move_finalize,duplicate_apply,duplicate_backup,versions*}.rs`, `native/src/sync/**` | Welle 2 (nach V3, V1) | Walks tolerant, Versionen je Lauf/Datei auf dem Ziel, Zeiten übertragen, dauerhaft, Ordner; Tests |
| H-DISPATCH | FA3 (Eingrenzen), FC1-Durchsetzung (Nur-lesen, `may_write`, Systemorte, App-Daten, `.se-versions` ausblenden), FC6, Wachhalten fremder Ströme | `native/src/share/core/{server,server_fs,server_admission,server_transfer,server_batch_get,server_batch_put,blocking,authorization_policy,configuration_runtime,node,node_accept,node_sessions,node_idle,node_wake,handshake_limits,fs,fs_access,fs_paths,fs_copy,walk,session,mount_lease*,power*,keepalive,io_deadline}.rs`, `native/src/daemon/os/shared/rooted_backend*.rs` | Welle 2 (nach V2, V5) | jede schreibende Anfrage geprüft, eingegrenzte Invalidierung, faire Grenzen, iteratives Löschen; Tests |
| S-POLICY | FC1-Konfiguration: Standards, Auto-Home-Migration, Räume ohne Freigaben, Verbindungs-Freigabe einzeln, Schreibrecht je Kontakt (UI/CLI), Desktop-Freigaben-UI, CLI | `native/src/share/core/{profiles,profile_persistence,export_config}.rs` (export_config nach V2), `native/src/share/os/shared/profile_*.rs`, `native/src/app/core/{share_exports_ui,share_helpers,share_rooms_ui,share_profile_*,share}.rs`, `native/src/cli/share/exports.rs`, `native/src/mobile/os/shared/domains/share_peers.rs`, `native/src/mobile/core/config.rs` | Welle 2 (nach V2, V5) | neue Profile/Räume ohne Freigaben, Migration, Rechte sichtbar; Tests |
| S-LOCAL | FC7 inkl. B14, IPC-Vorab-Plätze (S60) | `native/src/support_dirs.rs`, `native/src/creds/**`, `native/src/daemon/os/shared/{ipc_listener,locks}.rs`, `native/src/daemon/os/{windows,linux_os}/ipc_storage.rs`, `native/src/share/os/{windows,linux_os}/identity_lock.rs`, `native/src/net/**`, `native/src/share/core/lan_*.rs`, `native/src/share/os/shared/lan_*.rs`, `native/src/app/core/share_lan*_ui.rs`, `native/installer.nsi` | Welle 2 | Rechte ab Erstellung, Windows-ACL-Prüfung, Uplink-Reparatur, LAN-Privatsphäre; Tests |
| D-SYNCUI | Desktop-Bedienung FS3/FS9/FS10/FS12, Y147/Y148/Y156/Y19, Versionen ansehen/wiederherstellen | `native/src/app/core/{job_editor*,menus_sync_jobs,settings_background,bisync_ui,bisync_conflict*,bisync_merge,merge_ui,preview_core,sync_core,landing}.rs`, `native/src/app/os/shared/sync_jobs.rs`, neue `app/core/sync_versions_ui*.rs` | Welle 3 | Desktop zeigt/bedient alles Neue; Tests der Logik |
| AND-SYNC | FS11, Y144, Android-Teile FS8/FS9/FS12 | `native/src/mobile/os/shared/domains/{background,sync_run,sync_jobs,job_json,sync_conflicts,sync_merge}.rs`, `native/src/mobile/os/shared/sys.rs`, Kotlin `work/**`, `service/{BackgroundService,BackgroundController,BackgroundText}.kt`, `system/{BootReceiver,HostMonitor,KeepAlive*,WakeKeeper,Permissions,Notifications}.kt`, `ui/sync/**`, `ui/settings/BackgroundSettings.kt`, `api/SyncApi.kt`, `android/app/src/main/AndroidManifest.xml` | Welle 3 | Wakelock, Alarme, Content-Trigger, Allzugriff-Vorbedingung, Jobstatus/Versionen; Tests |
| AND-SHARE-UI | Android-Bedienung FC1–FC5 | Kotlin `ui/share/**`, `ui/settings/SettingsScreen.kt`, `api/ShareApi.kt`, `docs/superpowers/plans/2026-09-25-android-apk/api.md` | Welle 3 | alle Deltas in api.md, UI vollständig; Tests |
| SUITE | eine Task-Suite | `native/test-review-task.sh`, `.github/workflows/review-task.yml`, neue Suite-Hilfsdateien | nach Welle 3 | jede Abnahme-Zeile unten automatisiert |

Nicht aufgeführte Dateien ändert nur, wer sie per Anfrage zugeteilt bekommt.

## Agentenplan

- Welle 1 (7 gleichzeitig): K1, K2, K3, A-CLIENT, T-JOBS, S-SIGNAL, S-REVOKE.
- Welle 2: V-REMOTE, E-APPLY, H-DISPATCH, S-POLICY, S-LOCAL – sobald Plätze frei und Verträge eingetragen.
- Welle 3: D-SYNCUI, AND-SYNC, AND-SHARE-UI; danach SUITE.
- Nach allen Implementierungswellen genau eine vollständige Remote-Task-Suite (`review-task.yml`, Modus
  `suite`); keine Zwischen-Checks. Während des Remote-Laufs werden keine neuen Agenten gestartet (AGENTS.md).
  Fehler gehen per Nachricht an den zuständigen Agenten; nur dieselbe Suite wird für erforderliche Fixes wiederholt.
- Commits je fertigem Block (Orchestrator), graphify-Auffrischung nach nativen Änderungen.

## Gesamtablauf (Abnahme in der einen Suite)

| Ablauf | Spec | Wie | Erfolg, wenn |
|---|---|---|---|
| Meilensteine | alle | `cargo test review_task_` auf Linux, Windows (windows-2025-Job) und Android-Host | alle grün, Quell- und Laufliste stimmen überein |
| Fern-Analyse | FA1–FA7 | Rust-Test über `mobile::call("analyze.start"/"reclaim.start")` gegen einen Share-Peer im selben Prozess, Host-Baum mit vielen Ordnern; Zähler am Host | 0 ListDir/Read für Analyse und Duplikatsuche, Ergebnis = lokale Analyse des Hosts, Abbruch sofort |
| Präsenzwechsel | FA3 | laufende Analyse/Übertragung, dann Präsenz eines dritten Kontakts und neues Raum-Mitglied | Strom bleibt offen; Sperre eines Schlüssels schließt alle seine Sitzungen inkl. Exec |
| Sync-Backup | FS1–FS7 | Spiegel- und Zwei-Wege-Jobs lokal→lokal, →SFTP/WebDAV/FTP (Container), Abbruch, Fehlerdatei, leeres Ziel, Rotation zweier Loop-Abbilder (FAT/exFAT), FIFO, langer Name, Windows-Namen | zweiter Lauf kopiert nichts; Abbruch verliert nichts; leeres/fremdes Ziel stoppt; Rotation spiegelt je Laufwerk; FIFO blockiert nicht |
| Echtzeit | FS8 | Daemon-Echtzeit-Job: Umbenennen, Verschieben, gleich große Änderung, Dauer-Schreiber, Überlauf (viele Dateien), Watch-Limit (gesenktes Limit), Neustart mit offener Änderung, Share-Gegenseite (`watch_v1`) | jeder Fall startet einen Lauf innerhalb Entprellung+Höchstwartezeit; Limit → sichtbare Abfrage |
| Sicherheit | FC1–FC7 | Profile ohne Standardfreigabe, Schreiben/Recycle/SetStageMtime auf Nur-lesen, Systemorte, App-Daten, leere PIN, Klartext-Server ohne Opt-in, Lookup-Übernahme am Server, gefälschte Präsenz eines zweiten Kontakts, entfernter Peer mit neuer Geräte-ID, Exec nach „Wieder erlauben“, keine Gegenseitigkeit ohne Wahl | alles abgelehnt; Opt-ins wirken |
| Gemischte Versionen | FC3/FC4/FA2 | `native/test-share-mixed-version-e2e.sh` (veröffentlichte 0.5.126- bzw. 0.5.169-CLI gegen neuen Server/Client) | Grundfunktionen gehen, neue Rechte nur mit neuer Seite |
| Android-Gerät | FA1, FS11, FC-UI | Emulator: Fern-Analyse gegen Desktop-Host, Dauerbetrieb-Lauf mit Bildschirm aus, Allzugriff entzogen, Share-Einstellungen | wie Spec |

## Status

Stand 2026-10-04: Alle unten aufgeführten RV1-Quellenblöcke und Consumer sind gemeinsam durch [Run 37175826251](https://github.com/b1ue-man/smart-explorer/actions/runs/37175826251) auf `87021dcb` abgenommen und in [v0.5.170](https://github.com/b1ue-man/smart-explorer/releases/tag/v0.5.170) am Artefaktcommit `398f0e7f` veröffentlicht. Der vollständige Remote-Release und sein einziger Publikationsconsumer sind erfolgreich; Version, Tag, Installer und alle erwarteten Asset-/Feedhashes stimmen überein. Die Commitangaben dokumentieren die einzelnen Quellenmeilensteine. Ausdrücklich zurückgestellte Spec-Themen und allgemeine Handoff-Grenzen bleiben offen.

| Block | Status | Notiz |
|---|---|---|
| K1 → V-LOCAL | Abgenommen in v0.5.170; Quellen `cd8632d` | V1, relative DirectoryHandles, sichere private Erstellung/Quarantäne, Stages und lokale Pfad-/Mountregeln umgesetzt. Consumer und gemeinsame Remote-Abnahme sind abgeschlossen. |
| K2 → H-ANALYSIS | Abgenommen in v0.5.170; Quellen `5098ee0` | Host-Analyse, Duplikate, Hash/List/Watch, Deflate und Wiederanbindung umgesetzt. Windows-FA6 und Android-Host-Produzent sind ausdrücklich getrennte Anschlussblöcke. |
| K3 → E-PLAN | Abgenommen in v0.5.170; Quellen `dd0dccf` | Planung, Owner-/Replika-Basis, Checkpoints und Index umgesetzt; Apply-/Provider-Integration und gemeinsame Remote-Abnahme abgeschlossen. |
| A-CLIENT | Abgenommen in v0.5.170; Quellen `594ed68` | FA1/FA2/FA6, Budgets, Host-Retention, Ortsidentität und Agent-Protokoll 11 verbunden; konkrete Signale in `abnahme/A-CLIENT.md`. Optionale Range-Lesung bleibt Erweiterung, Agent-Nutzlasten wurden im terminalen Remote-Release neu gebaut. Gemeinsame Registrierungen sind integriert; der konkrete CI-6-Share-Anschluss ist in `a96d39ad` committed (eigener Handoff `abnahme/CI-6-A-CLIENT.md`). Die gemeinsame Remote-Abnahme ist abgeschlossen. |
| T-JOBS | Abgenommen in v0.5.170; Quellen `7806d24` | Persistierte Echtzeit-Arbeit, Scheduler, Watch, Guardian, Hooks und Ergebnisse umgesetzt; Desktop-/Android-Consumer sind integriert und abgenommen. |
| S-SIGNAL | Abgenommen in v0.5.170; Quellen `af77ead` | FC2–FC4/B20/B21 fertig; gepinnter Relay-Builder und Peer-URL-Filter bei H-DISPATCH. Gemeinsame Remote-Abnahme abgeschlossen. |
| S-REVOKE | Abgenommen in v0.5.170 | `0d36bdd`: Beziehungen, gezielter Exec-Entzug, Persist-Gates/S66, retrybarer Widerruf/S24, Zeitversatz/S32; eigener Block nach statischem Self-Review beendet. V5-Durchsetzung und UI-Anschlüsse sind durch H-DISPATCH/S-POLICY/Android integriert; gemeinsame Remote-Abnahme abgeschlossen. |
| V-REMOTE | Abgenommen in v0.5.170; Quellen `9b5bd1cb` | Provider-Erweiterungen, Literalnamen und echte Pollsignale; Engine-Verbrauch Y124/Y132/Y134/Y140/Y142 ist mit E-ENGINE integriert und abgenommen. Keine Umdeutung gespeicherter Locators. |
| E-APPLY | Abgenommen in v0.5.170; Quellen `b9f9b2ae`, `4a8130a2` | V3-Reporting/Snapshots, reversible Versionen und bestätigte Signaturen; die begrenzten Restdiagnosen des fünften Laufs sind quellenfertig committed (`c9cd629a`, `f744c63e`, `1b9eb4fe`); der gleiche vollständige Remoteeintritt hat auch diese Anschlüsse erfolgreich abgenommen. |
| H-DISPATCH | Abgenommen in v0.5.170; Quellen `74af4e1a` | V2-Hostoperationen, gezielter Rechteentzug, faire Zulassung und private Pfade. H-REPLACE/OS-Policy-Grenze sind integriert und abgenommen; verbleibende allgemeine LocalBackend-Grenzen bleiben ehrlich dokumentiert. |
| H-TRASH-WINDOWS | Abgenommen in v0.5.170; Quellen `708b6f1` | Windows-Fern-Papierkorb mit Record vor Capture, Handle-/Inhaltsbindung und sichtbarer Host-Wiederherstellung vollständig angeschlossen; kein neues Review. |
| S-POLICY | Abgenommen in v0.5.170; Quellen `d3cced7` | FC1-Konfiguration, Migration und Bedienwege, mit fertiger V5-Widerrufshistorie. Native Android-Rechtefacaden ergänzt. |
| S-LOCAL | Abgenommen in v0.5.170; Quellen `0a2a39e2`, `2fbdfddd` | FC7/B14/S60 private Erstellung/IPC/Helper/Privatsphäre; gepinnter aktueller privater TLS-LAN-Pfad statt Beacon-Autorität. Gemeinsame Remote-Abnahme abgeschlossen. |
| D-SYNCUI | Abgenommen in v0.5.170; Quellen `b4fd5851` | Desktop-Consumer für JobState, Versionen, Recorded-Merge/KeepBoth und Hintergrundzustände; erwartete Abschlussgrenzen sind Teil derselben vollständigen Remote-Suite. |
| AND-SYNC | Abgenommen in v0.5.170; Quellen `7321668e` | Echte Jobzustände, Storageverlust-Cancel, Versionen, Probleme, Alarm/Worker, primäre Hostzahlen und ausdrücklicher Recorded-Merge-Retry. Regulärer Recovery-Pfadschutz ist mit E-ENGINE integriert und abgenommen. |
| AND-SHARE-UI | Abgenommen in v0.5.170; Quellen `a70cb532` | Voll gepinnte Rechtefacaden, explizite wiederholbare Dialoge und zentrale API-Dokumentation. Gemeinsame Remote-Abnahme abgeschlossen. |
| SUITE | Abgenommen in v0.5.170 | `review-task.yml` → `native/test-review-task.sh`; genaue Kandidaten-/Diagnosezuordnung in `fortsetzung.md`, CI-5-Quellenkorrekturen committed; die konkrete verbliebene Share-Bindung aus dem sechsten Lauf ist gemäß `ci-sixth-fixes.md` in `a96d39ad` integriert. Derselbe vollständige Remoteeintritt hat den exakten gepushten Kandidaten `87021dcb` erfolgreich ausgewertet. Kein neuer Suiteeintritt und keine lokale Ausführung. |

## Konkreter FA6-Anschluss Windows

Die gemeldete Windows-Unsupported-Grenze wird als vorhandener FA6-Rest geschlossen.
Recherche eins: die gespeicherten Handle-/Quarantäne- und Papierkorbverträge; Recherche zwei:
Microsoft-DeleteItem/OperationFlags und aktuelle `QuarantinedChild`-API, gesichert in
`docs/refs/windows-checked-recycle.md`. Kein zusätzlicher Projekt-Review.

Meilenstein H-TRASH-WINDOWS verbindet (1) einen privat und dauerhaft geschriebenen Intent
vor Capture, (2) exakte Handle-/SHA-256-Bindung und retrybaren Restore ohne Überschreiben,
(3) begrenzte Neustart-Auflistung und sichtbare Host-Bedienung. Betroffene konkrete
Dateien und die erlaubte kleine Quarantäne-API-Erweiterung stehen im Scope-Manifest.
Erwartetes Ergebnis: Fern-Recycle auf Windows ist in der tatsächlichen Capability
angeboten, verändert keine inzwischen andere Kopie, bleibt nach Teilfehler/Neustart
auffindbar und lässt sich am Host wiederherstellen. Diese Signale gehen zusammen mit
Linux/Android-Papierkorb und der letzten verbleibenden Kopie in die eine Remote-Suite.

### Provider-/Engine-Anschluss der dokumentierten Restbefunde

Y124/Y132/Y134/Y140/Y142 benötigen gemeinsam die Grenze zwischen Provider und Engine. V-REMOTE liefert Literalname-Kodierung, stabilen Drive-Accountkey, ehrliche paginierte Feed-Signale und reversible Ersetzung ohne Atomicity-Claim. Der Engine-Anschluss konsumiert `sync_path` für alle betroffenen Kontroll-/Apply-/Version-/Indexpfade, schützt alte kodierte Locators und migriert vorhandene Baseline-Identitäten einmalig unter der bestehenden Paarsperre. Feed-IDs werden nur nach belegter Root-Ancestry in Pfade übersetzt; entfernte/mehrdeutige IDs oder unvollständige Beobachtung erzwingen Kontrolle.

Vor einer Rename-aside-Ersetzung steht ein privater dauerhafter Intent mit Backend-/Verbindungs-/Rootbindung, Stage, Original und exakt gewähltem Recovery-Sibling. Jeder Fehler erhält beide Inhalte und den Intent; Wiederanlauf überschreibt kein inzwischen fremdes Original. Backup-/Versionspflicht bleibt vor der Mutation. Erwartetes Ergebnis: literal `aux.c`, `%61ux.c`, `100%.pdf` und andere zulässige Linux-/Android-Namen bleiben verschiedene Dateien auf Drive; Refresh-Tokenwechsel setzt eine bestehende passende Basis nicht still zurück; accountfremde Feed-Einträge löschen nichts; FTP/IIS und SFTP ohne posix-rename aktualisieren mit wiederherstellbarem Original. Diese Fälle gehen gesammelt in die eine Remote-Task-Suite.

## S09-Anschluss: tatsächlicher gepinnter LAN-Kanal

Der bereits dokumentierte S09-Befund verlangt mehr als signierte mDNS-Dialhinweise: ein bounded eigener Iroh-Statuskanal eines aktuell akzeptierten Direct-Peers antwortet auf eine frische Challenge mit der eigenen Uplink-Auskunft. Nur ein offener selektierter IP-Pfad mit bekannter, eindeutig einem aktuellen privaten Interface zugeordneter lokaler IP erzeugt einen kurzen privaten Worker-Nachweis. Fehlende Facts, Relay, veraltete Antwort, geänderte Pins und Alt-Peers ohne Kanal starten keine ICS/NAT-Sitzung. Datei-/Exec-/Rückfreigabe-Rechte bleiben an ihren vorhandenen Grenzen. Parent integriert nur Node-Felder, ALPN-Dispatch und IPC-Snapshot; S09-LINK besitzt den zusammenhängenden Kanal/Policy/Evidence-Verbrauch laut exaktem Scope. Erwartung für die eine Remote-Suite: eine frische echte gepinnte private Session kann opt-in starten, ein alleiniger Beacon, Replay, fremde/mehrdeutige Interface-IP, Relay oder widerrufener Peer nicht; Stop/Entzug bleiben wirksam.
