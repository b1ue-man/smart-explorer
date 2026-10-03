# Anfragen V-LOCAL – konkrete Integration

Stand: 2026-10-03. Native Primitiven und eigene Self-Review-Arbeit sind abgeschlossen.
Die folgenden Consumer-Grenzen sind Teil des bestehenden RV1-Plans. Außer-scope Dateien wurden
hier weder gelesen noch geändert; ältere K1-Anfragen sind ohne Consumer-Codeprüfung keine
Aussage, dass ein anderer Block noch unbearbeitet sei.

## H-ANALYSIS: sichere Aufzählung, lokale Lesekonsente und Recycle

Reexport: `crate::local_access::{DirectoryHandle, QuarantinedChild, LocalEntry, EntryKind}`.
Die Typen bleiben crate-intern. Die vorhandene Root-/Export-Autorisierung muss vor dem
Root-Einstieg stattfinden; ein DirectoryHandle erteilt keine zusätzliche Pfadberechtigung.

```rust
DirectoryHandle::open_root(path: &Path) -> io::Result<DirectoryHandle>
DirectoryHandle::open_root_consented(path: &Path) -> io::Result<DirectoryHandle>
DirectoryHandle::metadata(&self) -> io::Result<std::fs::Metadata>
DirectoryHandle::open_child(&self, name: &OsStr) -> io::Result<DirectoryHandle>
DirectoryHandle::read_directory(&self) -> io::Result<DirectoryEntries>
DirectoryHandle::open_regular_child(&self, name: &OsStr) -> io::Result<File>
DirectoryHandle::create_private_child(&self, name: &OsStr) -> io::Result<DirectoryHandle>
DirectoryHandle::create_file_new(&self, name: &OsStr) -> io::Result<File>
DirectoryHandle::quarantine_regular_child(&self, name: &OsStr, expected: &File)
    -> io::Result<QuarantinedChild>
QuarantinedChild::file(&self) -> &File
QuarantinedChild::retained_location(&self) -> PathBuf
QuarantinedChild::restore(&mut self) -> io::Result<()>
QuarantinedChild::move_to(&mut self, target: &DirectoryHandle, name: &OsStr)
    -> io::Result<()>
```

`DirectoryHandle` ist Clone. `DirectoryEntries` ist ein Iterator über
`io::Result<LocalEntry>` mit frischer Position. Unix öffnet ab der gewählten Wurzel FD-relative,
ohne Child-Linkfolgen; Windows löst die gewählte Wurzel einmal auf und hält physische Vorfahren
und jeden betretenen Ordner ohne Write-/Delete-Sharing fest. Child-Namen sind exakt eine normale
Komponente. Metadaten kommen aus dem geöffneten Objekt, nicht aus einem späteren Pfad-Stat.
Windows-Daten-Reparse bleiben gewöhnliche Analyse-Dateien; echte Redirects und Spezialdateien
sind geschützte Auslassungen. A08/A17 werden mit diesem Vertrag ohne Child-Kanonisierung bedient.

Fremde Host-Aufträge verwenden `open_root`: keine Broker-Abfrage und keine neue Aktivierung von
Backup-Rechten in Root-/Child-/Listing-/Reparse-Probes. Eine konsentierte lokale GUI-Analyse darf
`open_root_consented` verwenden. Diese API startet keinen UAC-Dialog; sie verwendet ausschließlich
bestehende Lesefreigabe oder private Thread-Backup-Rechte. Bei expliziter Impersonation wird kein
GUI-Grant übernommen. Der authentifizierte Helfer hält seinen einmal gewählten Root sitzungsweit;
PinRoot überträgt read-only Vorfahrenhandles, PinChild den neuen Ordner. Alle erhalten nur
FILE_SHARE_READ; normale File-/Listing-Handles bleiben nur lesbar. Keine Schreibrechte/Token
werden vom Broker übertragen. Der neue pin-/lesegebundene Vertrag schließt die A27-Regression
durch die DirectoryHandle-Umstellung, erweitert aber keine Rechte fremder Host-Aufträge.

Für Recycle: reguläre Datei am Parent öffnen → erwartete Größe/SHA-256 prüfen → über denselben
Parent in Quarantäne einfangen → Identität und Inhalt über `file()` erneut prüfen → native
Trash-Policy mit privaten Info-Records reservieren → `move_to` in bereits geöffneten
Same-Filesystem-Zielordner. Die SHA-Revalidierung muss vorher zum Dateianfang seeken; Windows'
`try_clone` kann die Dateiposition teilen. `metadata()` ermöglicht Owner/Mode/Same-FS-Prüfung
der Trash-Ordner am Handle. `create_private_child`/`create_file_new` ersetzen/adoptieren nichts;
Fehler dürfen eine leere eigene Reservierung hinterlassen.

Kein `trash::delete(retained_location())` oder anderer frei pfadbasierter Handoff.
`retained_location` dient nur Diagnose/Recovery. Restore und Move überschreiben keine Ersatzdatei;
bei Fehlern bleibt Inhalt erhalten und muss als nicht erledigt gemeldet werden.
QuarantinedChild::Drop löscht weder Inhalt noch Ersatzobjekte. Linux kann einen während Capture
eingetauschten Namen vorübergehend reversibel verschieben; die anschließende Identitätsprüfung
verweigert ihn und stellt nur ohne Ersetzen zurück. Ein fehlgeschlagener Restore bewahrt den
Quarantänepfad in der Fehlermeldung. Provider ohne echtes atomisches NOREPLACE bzw. sichere
Capture-Identität werden verweigert. Leere markierte Reservierungsordner dürfen bleiben.

Consumer-Fläche laut bestehendem Plan: `native/src/share/core/analysis_*.rs`,
`native/src/share/core/remote_trash*.rs`, `native/src/share/os/shared/storage_analysis_host.rs`
und `native/src/analytics/os/shared/reclaim/{finder*,verify,cleanup}.rs` des H-ANALYSIS-Blocks.
Keine unsichere Pfad-Fallback-Lösung ist durch diesen Hook freigegeben.

## S-LOCAL: private App-/IPC-Objekte und eigene Altobjekte

Zusätzlich bereit:

```rust
crate::local_access::secure_private_handle(file: &File, is_directory: bool)
    -> io::Result<()>
DirectoryHandle::secure_private(&self) -> io::Result<()>
```

Vor dem Lesen/Schreiben privater Bytes verwenden. Der Caller öffnet vorhandene Records
ordinary, no-follow und unter seinem autorisierten/gepinnten Parent. Der Hook ist
Objektprüfung/-Härtung; er ersetzt keine Root-Autorisierung.

Unix: fstat → effektive UID, regulärer Typ bzw. Directory, für Datei genau ein Hardlink →
gegebenenfalls fchmod 0600/0700 → erneutes fstat. Bereits korrekter Modus braucht keine Mutation.

Windows: keinerlei Reparse-/Device-Objekt, korrekte Art, bei Datei genau ein Hardlink →
Owner == effektiver TokenUser → geschützte nicht-null DACL mit genau einer nicht erbenden
Owner-FILE_ALL_ACCESS-ACE verifizieren. Nur eigenes Altobjekt darf über
SetSecurityInfo(handle, SE_FILE_OBJECT, DACL | PROTECTED_DACL) auf diese DACL migriert werden;
Owner und SACL werden nicht geändert. Abschließend Owner/DACL/Art/Links erneut prüfen.
Keine Backup-Aktivierung und kein ACL-Setzen am freien Pfad.

Vorhandenes File benötigt FILE_READ_ATTRIBUTES | READ_CONTROL, bei Migration zusätzlich
WRITE_DAC, OPEN_REPARSE_POINT und FILE_SHARE_READ. Bereits private DACL wird ohne ACL-Mutation
akzeptiert. `DirectoryHandle::secure_private` kapselt den kontrollierten READ_CONTROL/WRITE_DAC-
Reopen am gepinnten physischen Root samt Identitätsvergleich.
Neue Childs/Records aus obigen Create-APIs tragen ihre private DACL bereits vor Bytes.
Provider ohne durchgesetzte private DACL liefern Fehler.

Die von S-LOCAL geplante Façade `crate::support_dirs::{ensure_private_dir, create_private_file,
open_private_file, open_private_lock, write_private_atomic}` kann diese Hooks verwenden.
Die konkreten Consumer unter `support_dirs`/`creds` bleiben in dessen Scope.
Der Hauptagent hat das erforderliche `Win32_Security_Authorization`-Feature in
`native/Cargo.toml` additiv als 779295d eingetragen; Lock/Version bleiben unverändert.
Diese Cargo-Datei wurde von V-LOCAL nur gelesen.

## T-JOBS, A-CLIENT, H-ANALYSIS: partielle Watches

```rust
ChangeNotice::ReadyPartial { generation: Option<u64> }
DirectoryHandle::watch_path(&self) -> Option<PathBuf>
```

Ready bedeutet volle Abdeckung; ReadyPartial bedeutet bekannte partielle Abdeckung.
Generation ersetzt keine Abdeckungszusage. Poll-/Abfragepfad bleibt bei Partial aktiv.
H-ANALYSIS überträgt dies als `FsWatchEvent::Ready { complete: false, ... }`;
unbekannte ältere Wire-Hosts bleiben per Default unvollständig. A-CLIENT muss es als eigenen
WireChange-Kind weiterreichen; T-JOBS behandelt es hybrid. Andere exhaustive Matches müssen
die neue additive Variante sicher aufnehmen.

Unix-FD-Anker: `/proc/self/fd/<owned-fd>/.`. Der Watcher hält einen Clone desselben
DirectoryHandle während Registrierung und bis zur Sink-Zustellung. Childs werden ausschließlich
über open_child betreten; die FD-Schreibweise ist kein frei weitergebbarer Childpfad.
Windows liefert None, solange kein sicherer overlapped Watch-Handle-Vertrag vorhanden ist.
T-JOBS meldete am 2026-10-03 die Integration von `watch::watch_confined`: Root-/Registered-
Clones plus Clone während emit, Unix direkte Root-Watch als LocalOnly, Windows Unsupported/Poll,
kein freier Path-Walk/Extend. Diese Integration wurde hier nicht fremd geprüft.

Gemeinsamer eigener Stage-Matcher erkennt jetzt zusätzlich genau
`.se-private-<32 lowercase hex>.tmp`. Andere .tmp-Namen, falsche Länge/Zeichen und Präfixe
bleiben Nutzernamen. Watch/Walk/Host müssen weiterhin `vfs::is_staging_name` verwenden.

## H-DISPATCH / S-LOCAL: ältere Schreibpfad- und Exportgrenzen

Die alten `LocalBackend::mkdir_all`-/Schreibpfade prüfen nun einen zuvor verwendeten Ordner erneut,
statt einen bloßen Pfad für dauerhaft sicher zu halten. Gewählte Root-Links oberhalb bleiben
zulässig. Diese Pfadmethoden werden damit nicht zu FD-relativen Mutationen.
Y95s weitere Schreibpfad-Beschleunigung darf nur eine verankerte, für die Operation gültige
Guard-/Pin-Lebensdauer wiederverwenden; kein HashSet<PathBuf> ohne Schutz gegen Directory-Tausch.

Die eingegrenzten Hüllen `native/src/daemon/os/shared/rooted_backend*.rs` und
`native/src/share/core/blocking.rs` brauchen eigene Extension-Pfadprüfung/-Übersetzung.
Keine ungeprüfte Übergabe von inner.extensions(). Eigene Stage-/Quarantänennamen sind für
fremde Reads, Mutationen, Analyse und Watch geschützt; besonders
`.held.se-recycle-<16hex>` und `.se-private-<32hex>.tmp`.
Eine partielle Liste darf nicht als vollständig oder als Beweis für Löschungen dienen.
Upload-Bestätigung benötigt den vorhandenen durable Stage-Finish-Vertrag, siehe K1 Nr. 7a.

## Bestehende K1-Fremdflächen zur Integration

Die folgenden konkreten Grenzen bleiben im dort benannten Block zu prüfen, nicht in V-LOCAL:

- A-CLIENT: `agent/core/metadata.rs`, `agent/core/backend.rs`, `agent_proto/**`,
  `daemon/os/shared/backend_server.rs` und `ipc_client.rs`: special und V1-Extensions
  über Hüllen reichen; Linux-Agent-NOREPLACE über `android_fs::rename_no_replace`.
- H-ANALYSIS: `share/core/backend.rs`: FsMeta.special und PeerBackend-Extensions;
  tatsächliche sichere Recycle-Policy/Trashinfo, Abbruch, Fortschritt und omittierte Teilbäume.
- S-LOCAL: `net/core/backend.rs`: UncBackend reicht die lokalen V1-Extensions ohne
  Backend-/Connection-Identitätsverlust weiter.
- Orchestrator: `transfer/os/windows.rs` liegt außerhalb dieser Modify-Fläche.
  Die bereits dokumentierten Y99/Y100-Punkte dort (Daten-Reparse-Upload und Readonly-Ersetzen)
  brauchen `local_access::metadata_is_link_like` bzw. `vfs::replace_local_file`.
  `transfer/os/shared/walk_listers.rs` ist innerhalb Scope fortgeführt.
- E-APPLY/V-REMOTE: Stage-Zeit/-Rechte, tatsächliche Provider-Dauerhaftigkeit, Zielgrenzen,
  Mount-Auslassungen und Baseline-Rückfälle bleiben deren Consumer-Verantwortung nach V1.

Keine fehlende Installation oder weitere lokale Ausführung wird angefragt.
Ergebnis und exakte eigene Dateilisten: [abnahme/V-LOCAL.md](../abnahme/V-LOCAL.md).
