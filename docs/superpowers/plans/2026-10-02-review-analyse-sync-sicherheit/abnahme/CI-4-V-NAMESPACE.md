# CI-4-V-NAMESPACE: tatsächliche Bestätigung nach Veröffentlichung

Stand: 2026-10-04. Eigener begrenzter V-NAMESPACE-Abschnitt aus
`ci-fourth-fixes.md`, gemäß dem frisch gelesenen und vor den zusätzlichen
Definitionen von Root erweiterten `scopes/ci-4-v-namespace.json`.
Ausgangspunkt ist der vollständig beendete Run `37162485159`.
Dieser Handoff ersetzt keine Laufzeitbestätigung durch Roots RV1-Suite.

## Zugeordneter Befund und Umsetzung

Die frische vorgegebene Diagnose lokalisiert Androids Post-publish-Ablehnung
in `LocalBackend::sync_filesystem` bei `PerFileOnly`. Die konkrete
Windows-Evidenz in `/tmp/rv1-ci-fourth/e-engine.json` lautet
„namespace was not confirmed; copied file is not recorded“ im bestehenden
Agent-/Daemon-Copy-Fixture. Der Scope verlangt genau diesen Anschluss;
andere Fehlereinträge begründen hier keine zusätzlichen Änderungen.

Eine additive Namespacebestätigung trennt nun den veröffentlichten Parent
von Whole-filesystem-Flush und Deferred-Inhalten. Linux/Android flusht
das tatsächlich gehaltene Verzeichnis erst nach positivem Mountnachweis.
Windows bestätigt den bereits vorhandenen Fileflush-/Write-through-
Veröffentlichungsvertrag. Die Quellen und Syntax wurden vor dem jeweiligen
Edit lokal in [post-publication-namespace.md](../../../../refs/post-publication-namespace.md)
gesichert; positive und negative Qualifier sind dort vollständig beschrieben.

## Exakte API und Owneranschluss

```rust
// BackendExtensions: Default Ok(false)
fn confirm_namespace(&self, parent: &str) -> VfsResult<bool>;
// vfs reexportiert diesen Dispatch; VfsResult = io::Result
pub fn confirm_namespace<B: Backend + ?Sized>(
    backend: &B,
    parent: &str,
) -> VfsResult<bool>;
// Unix DirectoryHandle: geliehener read-only Directorypin
pub(crate) fn directory_file(&self) -> &std::fs::File;
```

`parent` ist das tatsächliche Elternverzeichnis nach bereits erfolgreicher
Veröffentlichung. Ohne Extension oder ohne Nachweis ist das Ergebnis false.
Eine Errorantwort propagiert und darf keinen erfolgreichen Checkpoint
erzeugen. Ein true bestätigt den unterstützten OS-Namespacevertrag;
fehlende Filecontents oder einen Deferred-Endflush ersetzt es nicht.

Definition und Bedeutung wurden vor dem Engineedit direkt mit E-ENGINE
vereinbart und Root gemeldet. Root hat die drei konservativen
Trait-/Dispatch-/Exportdefinitionen während dieses Blocks separat integriert
und besitzt die Cache-/Guard-/Unavailable-/Agent-Produktforwarder.
Root meldet die konkreten Integrationscommits `0703bb5c` (API),
`875c007c` (Produktforwarder) und `ecd67172` (E-Consumer/Fixtureforwarder).
E-ENGINE besitzt `apply_stage` und bestätigt den folgenden Anschluss:

- POST-Publish: lokal `vfs::confirm_namespace(backend, parent_of(path))`,
  mit unverändertem bisherigen Fallback auf `path`, falls es keinen Parent gibt.
- Der bisherige Remotezweig `!backend.is_local() -> Ok(true)` bleibt bestehen.
- PRE-Publish Now-Fallbacks bei nicht geflushtem Stage verwenden weiterhin
  ausschließlich den echten `sync_filesystem`-Vertrag mit true-Erfordernis;
  false bleibt Unsupported. Deferred-Preflight und späterer Endflush bleiben
  ebenfalls am bisherigen Whole-filesystem-Vertrag gebunden.

Diese Engine-/Forwarderdateien wurden von diesem Agent nicht geändert.

## Adapterentscheidungen und erhaltene Grenzen

`LocalBackend::confirm_namespace` konvertiert den Parent mit derselben
vorhandenen OS-Pfadgrenze und ruft ausschließlich den ausgewählten Adapter.
`LocalBackend::sync_filesystem`, `batched_device`, `FsProfile`, Stagefinish,
Root-/Childöffnungen, Link-/Specialschutz und bestehende Assertions
bleiben unverändert.

Linux/Android:

- `DirectoryHandle::open_root` behält seine gewöhnlichen Rechte und
  erlaubten Rootaliase; Child-NoFollow und Identität ändern sich nicht.
  `directory_file` leiht den lebenden `Arc<File>`-Pin ohne neue Pfadrechte.
- `/proc/self/fd/<fd>` wird bei gehaltenem Pin aufgelöst. Der tatsächlich
  geöffnete `st_dev` muss zum vorgefundenen `Mount.device` passen.
  Die verifizierten libc-Helper ersetzen jeden geratenen ABI-Abgleich.
- Native Typen bleiben an ihr bestehendes FlushModel gebunden.
  Generisches Linux-FUSE, fuseblk, Subtypen, fremde Quellen und
  unbekannte/abweichende Mountzuordnungen bleiben false.
- Android-FUSE wird nur für die belegte vold-/MediaProvider-Instanz
  zugelassen: exakt `fuse` und `/dev/fuse`, dieselbe Deviceinstanz an
  einer validierten System-Storage-Root, passende `Mount.root` für
  ganze oder per-user-Views. Die dokumentierten emulated-/UUID-/
  public-ID-Spelling und Bindaliase bleiben unterstützt.
- Erst dann wird `File::sync_all()?` auf genau diesem Directory-FD
  ausgeführt. Ein Öffnungs-, Mount- oder Flushfehler bleibt Err.
  Es gibt keinen Zugriff auf den geschützten unteren Storagepfad und
  kein Permissions-, Fileflush- oder Whole-filesystem-Ersatz.

Windows:

- Der Localadapter prüft das echte Parentverzeichnis und das bestehende
  Filesystemprofil. Die vorhandenen einzeln geflushten Stages und
  erfolgreichen Write-through-Renames bleiben der Bestätigungsvertrag.
- Der Agent prüft ebenfalls einen tatsächlichen Directoryparent und
  antwortet auf seinen vorhandenen SYNC_FILESYSTEM-Vertrag true:
  Required/Deferred-Stages werden bereits einzeln geflusht und beide
  Publicationformen verwenden Write-through. Queryfehler propagieren.
- NoReplace/Replace-Flags, Source-/Stage-/Parent-/Linkprüfungen,
  Read-only, DACL und synchrone Pins bleiben unverändert. Es entsteht
  kein Overlapped-Watchpfad, unprivilegierter Volume-Flush oder zusätzliches
  Daten-/Rootrecht. Der Vertrag gilt nicht für beliebige ungesicherte Writes.

Cancellation, Hash-/Bytebindung, private Backups und Backupfehlerblock
werden durch die additive Namespacequery nicht verändert.

## Eigener statischer Self-Review

Ein statischer Vergleich mit den vor Edit gesicherten SHA-256-Dateifingerprints
bestätigt bei allen acht bestehenden Rustflächen ausschließlich die eigenen
erwarteten Ergänzungen bzw. den begrenzten Windows-Agent-Queryersatz.
Whole-filesystem-, Profile-, Stage- und Publicationbytes sind unverändert.
Die neue Namespacequelle stimmt mit dem gespeicherten Entwurf überein.
Der vorhandene Tree-sitter-Rust-Parser meldet keine ERROR-/Missing-Nodes.
Kein Compiler, Formatter, Test, Server, Installations-, Git-, CI-, Graph-,
Release- oder Unteragentenlauf wurde durch diesen Agent gestartet.

| Eigene Rustfläche | Zeilen | Bytes |
| --- | ---: | ---: |
| `native/src/vfs/core/extensions.rs` | 192 | 7950 |
| `native/src/vfs/core/extension_calls.rs` | 284 | 9799 |
| `native/src/vfs/mod.rs` | 145 | 5594 |
| `native/src/local_access/os/linux/directory_handle.rs` | 275 | 9442 |
| `native/src/vfs/os/linux_os/local_platform.rs` | 188 | 7164 |
| `native/src/vfs/os/shared/local_extensions.rs` | 89 | 3609 |
| `native/src/vfs/os/windows/local_platform.rs` | 275 | 9936 |
| `native/src/agent_proto/os/windows/local_platform.rs` | 205 | 7118 |
| `native/src/vfs/os/linux_os/namespace_flush.rs` | 113 | 3881 |

Alle eigenen Rustflächen bleiben unter 500 Zeilen und 50 KiB.

## Abnahmesignale ausschließlich für dieselbe Root-RV1-Suite

- Bestehende tatsächliche Android-checkedDelete-/Versions-/savedSync-/
  Merge-/Private-Fixtures: Einzeln geflushte und exklusive erfolgreiche
  Veröffentlichungen auf dem echten unterstützten Storage erhalten eine
  Namespacebestätigung und ihre bestehenden Folgeassertions. Owner/Mode,
  NoFollow, Hash, NoReplace, Backupfehlerblock und private Daten bleiben
  wirksam. Fehler oder unbekannte Speicher dürfen keinen Erfolg seeden.
- `bisync::tests::links_remote::sync_links_task_agent_and_daemon_streams_fall_back_without_losing_protection`:
  Windows-Agent-Copy einer unabhängigen regulären Datei wird nach Fileflush
  und erfolgreicher Publication bestätigt; geschützte Link-/Specialomissions
  und sämtliche vorhandenen Assertions bleiben erhalten.
- Unbekanntes FUSE oder eine unpassende Device-/Root-/Sourcezuordnung:
  false ohne Ersatzgarantie; Öffnungs-/Flushfehler als Err ohne
  Baselineerfolg. Native Pfade und genuine Android-Storage-/Bindaliase
  behalten den oben beschriebenen positiven Vertrag.

Kein zusätzliches Testentrypoint und keine Fixtureänderung durch diesen
Agent. Runtimebestätigung bleibt bei Root; Tests wurden nicht ausgeführt.

## Exaktes Dateiinventar

Gelesen für diesen Block, Sourceinhalte gemäß den festgelegten Symbol-
bzw. Bereichsgrenzen; eigene neue Dateien zusätzlich zum Self-Review:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-4-v-namespace.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-fourth-fixes.md` — own V-NAMESPACE section.
- `docs/refs/local-fs-identity-durability.md` — local namespace/directory-fsync, Android storage and Windows publication sections only.
- `docs/refs/private-file-access.md` — existing pinned directory access needed for flushing only.
- `/tmp/rv1-ci-fourth/e-engine.json` — actual namespace failure evidence only.
- `native/src/vfs/core/extensions.rs` — BackendExtensions sync_filesystem and stage APIs only.
- `native/src/vfs/core/extension_calls.rs` — sync_filesystem and extension dispatch only.
- `native/src/vfs/os/shared/local_extensions.rs` — sync_filesystem, finish_stage and local path adapter calls only.
- `native/src/vfs/os/linux_os/local_platform.rs` — filesystem_profile, flush_filesystem and selected adapter registration only.
- `native/src/vfs/os/windows/local_platform.rs` — flush_filesystem and filesystem_profile only.
- `native/src/agent_proto/os/windows/local_platform.rs` — sync_filesystem, stage opening and both publishing rename APIs only.
- `native/src/local_access/mod.rs` — pinned parent/dir file opening API only.
- `native/src/vfs/os/linux_os/mountinfo.rs` — Mount fields, resolve_existing, containing and read only.
- `native/src/vfs/core/fs_profile.rs` — FlushModel and actual filesystem profiles only.
- `native/src/vfs/mod.rs` — local adapter and extension registrations/exports only.
- `native/src/agent_proto/mod.rs` — selected local adapter registration only.
- `native/src/agent_proto/os/shared/ext_ops.rs` — finish_stage and answer(SYNC_FILESYSTEM) only.
- `native/src/agent_proto/os/shared/promotion.rs` — publish stage replace/create only.
- `native/src/agent_proto/os/linux_os/local_platform.rs` — sync_filesystem and parent directory flush contract only.
- `native/src/agent/core/extensions.rs` — sync_filesystem dispatch only.
- `native/src/vfs/os/shared/local.rs` — batched_device and staged/rename publication only.
- `native/src/vfs/os/shared/local_stage.rs` — stage durability only.
- `native/src/local_access/os/linux/directory_handle.rs` — DirectoryHandle fields, root/child opens, pinned File/FD metadata access and related sync methods only.
- `native/src/vfs/os/windows/local_writes.rs` — write-through publication and no-replace only.
- `native/src/bisync/os/shared/apply_stage.rs` — read-only namespace and stage consumer only.
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/libc-0.2.186/src/unix/linux_like/android/mod.rs` — safe const major(dev_t)->c_int and minor(dev_t)->c_int definitions, lines 3392-3405 only.
- `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/libc-0.2.186/src/unix/linux_like/linux_l4re_shared.rs` — major/minor definitions lines 1623-1637 only.
- `native/src/vfs/os/linux_os/namespace_flush.rs`
- `docs/refs/post-publication-namespace.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-4-V-NAMESPACE.md` — eigener Bericht nach Erstellung.

Geändert:

- `native/src/vfs/core/extensions.rs`
- `native/src/vfs/core/extension_calls.rs`
- `native/src/vfs/mod.rs`
- `native/src/local_access/os/linux/directory_handle.rs`
- `native/src/vfs/os/linux_os/local_platform.rs`
- `native/src/vfs/os/shared/local_extensions.rs`
- `native/src/vfs/os/windows/local_platform.rs`
- `native/src/agent_proto/os/windows/local_platform.rs`

Erstellt:

- `docs/refs/post-publication-namespace.md`
- `native/src/vfs/os/linux_os/namespace_flush.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-4-V-NAMESPACE.md`

Gelöscht: keine. Eigene additive Registrierungen: `vfs::confirm_namespace`
und `local_platform::namespace_flush` mit beabsichtigtem Reexport.
Kein vorhandener öffentlicher oder Wirevertrag wird entfernt.

Frisch gelesene Primärlinks: exakt die im Scope erlaubten fsync-/syncfs-/
MoveFileExW-, MediaProvider-main- und drei vold-Mountquellen; die datierten
Syntaxbelege und Blob-IDs stehen in der oben verlinkten eigenen Ref.
Die libc-Cratequellen wurden ausschließlich in den freigegebenen Bereichen
`android/mod.rs:3392–3405` und `linux_l4re_shared.rs:1623–1637` gelesen.

## Verbleibende Grenzen

Die zusätzlich nötigen DirectoryHandle-/libc-/vold-Definitionsgrants sind
von Root erteilt und eingearbeitet; keine offene Implementierungs- oder
Scopeabhängigkeit dieses Produzentenanschlusses.
Der android16-release-Primärlink war zweimal nicht abrufbar; dafür wird
kein Quellenbeleg behauptet. Fresh main plus konkreter vold-Mountaufbau
tragen den gespeicherten begrenzten Adaptervertrag.

Root besitzt Produktforwarder und Integration, E-ENGINE den Consumer und
die Contents-/Deferred-Grenzen. Deren Änderungen und die Laufzeitabnahme
gehören nicht zu diesem Agentbericht. Der Block ist abgeschlossen; dieser
Agent stoppt.
