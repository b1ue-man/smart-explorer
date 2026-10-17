# Tresore: Bestand (Lesung)

Stand 2026-10-17, Lesestand da2646d2

Strang: S3 „Cryptomator-Tresore“ aus `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md`, Abschnitt A ([E8] Tresore öffnen und bearbeiten; [E9] Tresor-Sync-Job ohne Ordnereinrichtung, bei beidseitigen Änderungen keine automatische Auflösung, sondern Vergleichsmodus nach dem Öffnen). Strangdatei `docs/plaene/2026-10-17-rechte-desktop-tresore/strang-tresore.md` nur gelesen.

Rolle: Bestandsleser. Diese Datei enthält belegte Befunde. Sie enthält keine Bewertung, keine Empfehlung und keine Entscheidung. Jeder Befund trägt `Pfad:Zeile` (Zeilen bezogen auf die Arbeitskopie am Lesetag; der Quellcode wurde nicht geändert, siehe Abschnitt 7).

## 1. Ziel und Fragen

Ziel laut `strang-tresore.md` („Schleifenstand“): Cryptomator-Tresore in se öffnen und bearbeiten, an jedem Ort, den se erreicht; ein eigener Tresor-Sync-Job ohne Ordnereinrichtung, der die neuesten Änderungen abgleicht; bei Änderungen auf beiden Seiten keine automatische Auflösung, sondern ein Vergleichsmodus nach dem Öffnen des Tresors.

Fragen dieser Lesung:

1. Backend-Trait: Methodenliste mit Signaturen und Semantik, Fähigkeitsflags, Identität, Erweiterungen; wie eine Schicht ein anderes Backend umhüllt (Beispiel zipfs); wie eine neue Backend-Art aus einem Locator aufgelöst, adressiert und gespeichert wird.
2. Klartextgrenze: jede Stelle, an der Dateiinhalte oder Dateinamen dauerhaft lokal abgelegt werden, mit Pfad, Inhalt und Lebensdauer.
3. Geheimnisspeicher: Signaturen und Schlüsselbenennung für das Ablegen eines Tresor-Passworts.
4. Sync: Job-Modell, Endpunkte, Konflikterkennung und -regeln, Konfliktvarianten, Merge, Versionen; Anzeige und Auflösung auf Desktop, Android und CLI; Hintergrundausführung; Jobs mit automatisch angelegtem Ziel ohne Ordnerwahl.
5. Oberflächen von Sync-Konflikten und Versionen (CLI, Desktop, Android, Bridge, IPC, MCP).

Konventionen: „Lücke“ kennzeichnet eine Tatsache, die die Lesung nicht belegt hat, mit genauer Angabe, was fehlt. „Annahme“ kennzeichnet eine Setzung, die nicht aus dem Code folgt.

## 2. Befunde je Frage

### 2.1 Frage 1: Backend-Trait, Erweiterungen, Schichten, Ortsauflösung

#### 2.1.1 Trait `Backend`

Quelle: `native/src/vfs/core/core.rs:17` (`pub trait Backend: Send + Sync`). Der Trait ist blockierend (`native/src/vfs/mod.rs:7–9`: „The trait is BLOCKING.“). Die Datei deklariert 74 Methoden (`grep '^    fn '`). Zehn davon haben keinen Standardrumpf (Pflicht): `scheme` (18), `root_display` (21), `list_dir` (42), `stat` (51), `open_read` (73), `open_write` (74), `rename` (180), `remove_file` (214), `remove_dir` (215), `mkdir_all` (216).

Die Tabelle nennt jede der 74 Methoden. „Standard“ gibt den Rückfall an, wenn eine Implementierung die Methode nicht überschreibt. „Pflicht“ bedeutet, dass keine Standardimplementierung existiert. Die Semantik stammt aus den Dokumentationskommentaren der Quelle; wo keiner steht, steht „kein Doc-Kommentar“.

| # | Methode | Zeile | Standard | Semantik laut Quelle |
|---|---|---|---|---|
| 1 | `scheme` | 18 | Pflicht | Liefert die `Scheme`-Variante des Backends. |
| 2 | `root_display` | 21 | Pflicht | Anzeigewurzel mit Schrägstrichen; Navigation startet dort. |
| 3 | `state_identity` | 26 | `format!("{:?}:{}", scheme, root_display)` | Stabile, nicht geheime Identität für seitenspezifischen Zustand; Remote-Backends sollen Konto und Host einschließen, nicht nur den Pfad (Doku 23–25). |
| 4 | `namespace_identity` | 30 | = `state_identity` | Dateisystem-Namensraum, unabhängig vom Startordner. |
| 5 | `uncached_backend` | 34 | `None` | Liefert das Live-Backend hinter einem Browsing-Wrapper für die Sync-Revalidierung (Doku 33; genutzt in `native/src/vfs/os/shared/sync_roots.rs:6–8`). |
| 6 | `extensions` | 39 | `None` | Liefert `Option<&dyn BackendExtensions>` (siehe 2.1.2). |
| 7 | `list_dir` | 42 | Pflicht | Einträge eines Verzeichnisses als `VfsMeta`. Kein Doc-Kommentar. |
| 8 | `list_dir_for_sync` | 44 | → `list_dir` | Sync beobachtet tatsächlich vorhandene Namen ohne Browsing-Aliase (Doku 43). |
| 9 | `has_duplicate_file_names` | 48 | `false` | Anbieter mit mehrfachen Namen; vollständige Beobachtung vor jeder Mutation nötig (Doku 47). |
| 10 | `stat` | 51 | Pflicht | Metadaten eines Pfads. Kein Doc-Kommentar. |
| 11 | `try_exists` | 56 | `trait_defaults::try_exists` (`native/src/vfs/core/trait_defaults.rs:7–13`): über `stat`; nur `NotFound` gilt als „nicht vorhanden“ | Zugriffs-, Transport- und Parsefehler dürfen nicht als Abwesenheit gelten (Doku 53–55). |
| 12 | `exists` | 62 | `try_exists(path).unwrap_or(false)` | Nur Hinweis für nicht mutierende UI-Pfade (Doku 60–61). |
| 13 | `item_id` | 68 | `Ok(None)` | Backend-eindeutige ID, falls vorhanden; Drive liefert die Datei-ID (Doku 66–67). |
| 14 | `open_read` | 73 | Pflicht | Lesestrom `Box<dyn Read + Send>`. Kein Doc-Kommentar. |
| 15 | `open_write` | 74 | Pflicht | Schreibstrom `Box<dyn Write + Send>`. Kein Doc-Kommentar; ob ein vorhandener Inhalt ersetzt oder gekürzt wird, steht im Code nicht dokumentiert (Lücke: Semantik nur aus Implementierungen ableitbar). |
| 16 | `open_write_new` | 80 | `unsupported` | Exklusives Neuanlegen ohne Ersetzen, Kürzen oder Übernehmen eines Eintrags; ein Anbieter ohne atomaren Exklusiv-Create lehnt ab (Doku 76–79). |
| 17 | `open_write_copy_stage` | 89 | → `open_write_new` | Private Kopierstufe, die keine bestehende Identität ändert (Doku 85–88). |
| 18 | `promote_copy_stage` | 95 | → `promote_staged_no_replace` | Veröffentlicht eine Kopie, ohne die Identität eines vorhandenen Objekts zu ersetzen (Doku 93–94). |
| 19 | `open_write_copy_stage_sized` | 101 | → `open_write_copy_stage` | Kopierstufe mit bekannter Endlänge; ein Schreiber schlägt bei `flush` fehl, wenn nicht genau `size` Bytes ankamen (Doku 99–100). |
| 20 | `open_write_copy_stage_unsynced` | 114 | → `open_write_copy_stage_sized` | Kopierstufe ohne erzwungenes Flush auf stabilen Speicher; für Paste/Drag/Download (Doku 110–113). Sync, Mounts und Ersetzungen verwenden `…_sized` (Doku 113). |
| 21 | `server_copy_to_stage` | 125 | `Ok(None)` | Serverseitige Kopie von `src` in eine neue private Stufe; `Ok(None)` = Stream; `cancel` beendet eine lange Kopie (Doku 122–124). |
| 22 | `open_write_fresh` | 139 | `Ok(None)` | Neue Datei in einem vom Transfer selbst angelegten Ordner, für Anbieter mit nur vollständig erscheinenden, nie ersetzenden Objekten (Drive); exakt `size` Bytes (Doku 136–138). |
| 23 | `open_read_at` | 146 | `Ok(None)` | Lesen ab Byte `offset`, um einen Download fortzusetzen (Doku 144–145). |
| 24 | `transfer_hint` | 158 | `None` | Fester Grenzwert des Anbieters für die Transferanzeige (Doku 156–157). |
| 25 | `download_name` | 164 | `name` | Lokaler Dateiname beim Download; exportierende Anbieter ergänzen die Endung (Doku 162–163). |
| 26 | `read_size` | 170 | `Some(metadata_size)` | Erwartete Länge des Lesestroms; `None` nur bei Inhalt, der beim Lesen transformiert wird (Doku 168–169). |
| 27 | `copy_file` | 176 | `copy_transfer::copy_file` | Kopie innerhalb desselben Backends (Doku 174–175). |
| 28 | `rename` | 180 | Pflicht | Kein Doc-Kommentar. Ob `rename` eine bestehende Datei atomar ersetzt, beschreibt `rename_overwrites` (268–270). |
| 29 | `rename_no_replace` | 186 | `unsupported` | Verschieben, wenn das Ziel nicht existiert; atomar, eine vorherige Existenzprüfung genügt nicht (Doku 182–185). |
| 30 | `promote_staged` | 195 | `promotion::default_promote_staged` | Commit einer vollständig geschriebenen Stufe, ohne Teilinhalte und ohne vorheriges Löschen des Alten; ID-Anbieter aktualisieren die Ziel-Identität prüfbar (Doku 191–194). |
| 31 | `promote_staged_to_id` | 199 | `promotion::promote_to_id` | Ersetzt das erfasste Objekt, bei Duplikat-Anbietern über die ID (Doku 198). |
| 32 | `promote_staged_no_replace` | 211 | `promotion::default_promote_staged_no_replace` | Commit nur, wenn das Ziel fehlt; eine atomare Operation (Doku 208–210). |
| 33 | `remove_file` | 214 | Pflicht | Kein Doc-Kommentar. Löschsemantik: `delete_disposition` (239). |
| 34 | `remove_dir` | 215 | Pflicht | Kein Doc-Kommentar. |
| 35 | `mkdir_all` | 216 | Pflicht | Kein Doc-Kommentar. |
| 36 | `create_dir` | 220 | → `mkdir_all` | Genau ein Verzeichnis, dessen Elternteil existiert; ein vorhandenes einfaches Verzeichnis ist Erfolg (Doku 218–219). |
| 37 | `create_dir_new` | 226 | `trait_defaults::create_dir_new` (`trait_defaults.rs:16–24`): erst Prüfung, dann Anlage | Neues Verzeichnis; `AlreadyExists` bei vorhandenem Namen; die Standardform ist nicht atomar (Doku 224–225). |
| 38 | `discard_copy_stage` | 232 | `unsupported` | Entfernt eine Kopierstufe, die der Client angelegt und nie veröffentlicht hat (Doku 230–231). |
| 39 | `delete_disposition` | 239 | `Permanent` | Ob `remove_*` endgültig oder papierkorbfähig löscht; `Recycle`, `Permanent` oder `Unsupported` (Doku 237–238; Typ `meta.rs:65–70`). |
| 40 | `parallelism` | 244 | `rayon::current_num_threads()` | Breite des Verzeichnisdurchlaufs (Doku 243). |
| 41 | `flow_key` | 250 | `format!("{:?}@{:p}", scheme, self)` | Schlüssel der Verbindung; Benutzer desselben Schlüssels teilen einen Concurrency-Regler (Doku 248–249). |
| 42 | `transfer_ceiling` | 257 | `None` | Harte Obergrenze gleichzeitiger Transfers, die Protokoll oder Gegenseite vorgibt (Doku 255–256). |
| 43 | `concurrent_read_write` | 264 | `true` | Ob Leser und Schreiber gleichzeitig offen sein dürfen; bei `false` puffert ein Kopiervorgang über einen lokalen Spool (Doku 262–263). |
| 44 | `rename_overwrites` | 271 | `false` | Ob `rename(src, dst)` eine bestehende Datei atomar ersetzt (Doku 268–270). |
| 45 | `staged_write_capabilities` | 277 | `capabilities::staged_write_defaults` (`capabilities.rs:57–65`): `create = false`; `replace` und `namespace_replace` = `rename_overwrites()` | Sichere Stage-Garantien unter `root` (Doku 275–276). |
| 46 | `probe_staged_write_capabilities` | 283 | → `mount_path_capabilities(root)?.staged_write` | Prüfung, die eine aktive Mount-Bindung nicht ersetzt (Doku 281–282). |
| 47 | `case_sensitive_paths` | 291 | `false` | Groß-/Kleinschreibung unterscheidet nur, wenn unter `root` garantiert (Doku 287–290). |
| 48 | `root_confinement` | 297 | `RootConfinement::Unverified` | Ob jede Operation im exakten Wurzelpfad bleibt, auch bei getauschtem Pfad (Doku 295–296). |
| 49 | `mount_path_capabilities` | 303 | `capabilities::mount_path_defaults` (`capabilities.rs:67–75`) | Schnappschuss aus `staged_write` und `root_confinement` (Doku 301–302). |
| 50 | `open_read_id` | 309 | → `open_read(path)`; `id` wird ignoriert | Öffnen über eine backend-eindeutige ID, falls bekannt (Doku 307–308). |
| 51 | `remove_file_id` | 319 | → `remove_file(path)`; `id` wird ignoriert | Löschen über eine ID, falls bekannt (Doku 318). |
| 52 | `plan_dedupe_recursive` | 326 | `Ok(Vec::new())` | Plant ohne Mutation die exakten Duplikate einer Spiegel-Bereinigung (Doku 324–325). |
| 53 | `apply_dedupe_plan` | 337 | `dedupe::apply` | Wendet einen zuvor geprüften Plan an; ID-Anbieter löschen die aufgezeichnete ID (Doku 335–336). |
| 54 | `dedupe_recursive` | 344 | `plan_dedupe_recursive` + `apply_dedupe_plan` | Macht ein Spiegelziel mit Duplikatnamen exakt (Doku 341–343). |
| 55 | `is_local` | 350 | `false` | Lokales Dateisystem: Hashen ist billig (Doku 349). |
| 56 | `provides_content_hash` | 355 | `false` | MD5 in der Listung (Doku 354). |
| 57 | `supports_changes` | 360 | `false` | Unterstützt einen inkrementellen Änderungsfeed (Doku 359). |
| 58 | `change_root_id` | 365 | `Ok(None)` | Stabile Identität einer Sync-Wurzel; ein Cursor gilt nur dort (Doku 364). |
| 59 | `current_change_cursor` | 372 | `Ok(None)` | Cursor vor dem Bootstrap-Durchlauf (Doku 370–371). |
| 60 | `changes_since` | 378 | `VfsChangeBatch { reset: true, .. }` | Änderungen seit Cursor; `reset` = Cursor ungültig, Neuaufbau nötig (Doku 377). |
| 61 | `invalidate_cache` | 387 | keine Aktion | Verwirft einen Listing-Cache; nur `CachingBackend` hat einen (Doku 386). |
| 62 | `scan_storage` | 390 | `Ok(None)` | Optionales vollständiges serverseitiges Analyseergebnis (Doku 389). |
| 63 | `supports_walk_tree` | 399 | `false` | Unterstützt die ältere Baum-Analyse des Agenten (Doku 398). |
| 64 | `walk_tree` | 404 | `Ok(None)` | Serverseitiger Baum mit Fortschritt; `false` bricht ab (Doku 403). |
| 65 | `supports_bulk_tree` | 413 | `false` | Ganzer Teilbaum in einer Sitzung (Doku 412). |
| 66 | `get_tree` | 419 | `unsupported` | Lädt den Inhalt von `root` in einer Sitzung nach `dst` (Doku 417–418). |
| 67 | `put_tree` | 426 | `unsupported` | Lädt den Inhalt von `src` in einer Sitzung nach `root` (Doku 424–425). |
| 68 | `batch_limits` | 433 | `None` | Batch-Grenzen für viele kleine Dateien unter `dir` (Doku 431–432). |
| 69 | `put_batch` | 441 | `unsupported` | Legt jeden Eintrag als neue Datei an; ein `Err` heißt „Ergebnis unbekannt“ und wird nicht blind wiederholt (Doku 438–440). |
| 70 | `get_batch` | 451 | `unsupported` | Liest jedes Element in Anforderungsreihenfolge an `sink` (Doku 450). |
| 71 | `supports_search` | 457 | `false` | Unterstützt die serverseitige Suche (Doku 456). |
| 72 | `search` | 464 | `Ok(false)` | Suche unter `root`, Treffer streamen nach `tx`; `Ok(false)` nur „nicht unterstützt“ vor dem ersten Treffer (Doku 461–463). |
| 73 | `supports_walk_hashed` | 477 | `false` | Signaturen (Größe, mtime, MD5 auf Abruf) in einem serverseitigen Durchlauf (Doku 475–476). |
| 74 | `walk_hashed` | 484 | `Ok(false)` | Serverseitiger Durchlauf mit einem `HashHit` je Eintrag (Doku 481–483). |

Typen der Schnittstelle (alle in `native/src/vfs/core/meta.rs` und `capabilities.rs`, Re-Exports in `native/src/vfs/mod.rs:107–116`):

- `VfsResult<T> = io::Result<T>` (`meta.rs:58`).
- `VfsMeta` (`meta.rs:8–29`): `name`, `is_dir`, `is_symlink`, `special`, `size`, `mtime_ms`, `btime_ms`, `hidden`, `system`, `id: Option<String>`, `content_md5: Option<String>`.
- `VfsChange` (`meta.rs:42–49`) mit `kind: ChangeKind` (`Upsert` | `Remove`, `meta.rs:33–36`), `rel`, `id`, `parent_id`, `name`, `meta`; `VfsChangeBatch` (`meta.rs:52–56`) mit `changes`, `new_cursor`, `reset`.
- `DeleteDisposition` (`meta.rs:66–70`): `Recycle` | `Permanent` | `Unsupported`.
- `DedupeCandidate` (`meta.rs:76–79`): `path`, `id`.
- `SearchHit` (`meta.rs:83–88`): `rel`, `is_dir`, `size`, `mtime_ms`. `HashHit` (`meta.rs:93–98`): `rel`, `is_dir`, `size`, `mtime_ms`, `md5`.
- `StagedWriteCapabilities` (`capabilities.rs:5–9`): `create`, `replace`, `namespace_replace`. `complete()` setzt alle drei auf `true` (21–28); `supports_mounted_writes()` verlangt alle drei (30–32); `intersect` (34–38).
- `MountPathCapabilities` (`capabilities.rs:16–19`): `staged_write`, `root_confinement`.
- `RootConfinement` (`capabilities.rs:45–49`): `Unverified` (Standard) | `Enforced`. Doku: `Enforced` ist für Kernel-Sandbox oder einen Namensraum reserviert, dessen Objektsuche den Wurzelpfad nicht verlassen kann (Doku 41–43).
- `Scheme` (`native/src/vfs/core/scheme.rs:3–11`): `Local` (Standard) | `Sftp` | `Ftp` | `Webdav` | `Smb` | `GDrive` | `Peer`. Eine Tresor-Variante existiert nicht.
- `VolumeIdentity` (`native/src/vfs/core/volume.rs:7–18`): `volume_id`, `relative_path`, `fs_type` (nur Anzeige). `key()` = `"{volume_id}:{relative_path}"` (37–39); `PartialEq` und `Hash` vergleichen nur `volume_id` und `relative_path` (42–54).
- `MountKind` (`volume.rs:22–33`): `Local` | `Network` | `Fuse` | `Automount` | `Pseudo`.

#### 2.1.2 Erweiterungen `BackendExtensions`

Quelle: `native/src/vfs/core/extensions.rs:29` (`pub trait BackendExtensions: Backend`). Doku 25–28: „Every method has a conservative default, so an implementation names only what its protocol or host can do. Wrappers that rewrite paths or guard access (export roots, mounts) must not hand out the extensions of the backend they wrap unchanged.“ Alle 24 Methoden haben einen Standardrumpf; es gibt keine Pflichtmethode. Die Methoden: `version_archive_policy` (32), `previous_state_identities` (38), `sync_child_path` (45), `sync_stat` (52), `replace_staged_reversible` (63), `list_dir_tolerant` (74), `open_read_regular` (87), `open_write_copy_stage_timed` (94), `finish_stage` (107), `sync_filesystem` (115), `confirm_namespace` (123), `target_limits` (130), `unix_mode` (137), `volume_identity` (145), `supports_duplicate_search` (151), `find_duplicates` (159), `supports_hash_walk` (170), `hash_walk` (179), `supports_recycle` (191), `recycle` (198), `supports_remove_tree` (206), `remove_tree` (217), `change_signal_mode` (228), `change_signal` (236).

Aufrufer gehen über freie Funktionen in `native/src/vfs/core/extension_calls.rs` mit Rückfällen (Doku `extensions.rs:1–4`). Die Funktionsnamen sind in `native/src/vfs/mod.rs:86–94` exportiert.

Die für Tresore relevanten Erweiterungen mit Standardwert:

- `version_archive_policy` (32–34): `Provider` (Standard) | `AppPrivate` (`extensions.rs:17–23`).
- `volume_identity` (145–148): Standard `Ok(None)`; Doku: „`Ok(None)` = not determinable, which means "unknown", never "another volume"“ (142–144).
- `replace_staged_reversible` (63–71): Standard `Ok(false)` ohne Mutation; die Doku verlangt, dass der Aufrufer die Sicherungsstelle `retained` vorher dauerhaft journalisiert (57–62).
- `remove_tree` (217–225): Standard `Err(tree_removal_unsupported())`; Doku: Links werden als Einträge entfernt und nie verfolgt (211–213).

#### 2.1.3 Wie eine Schicht ein anderes Backend umhüllt

Beispiel 1, `CachingBackend` (`native/src/vfs/core/cache.rs`):

- Struktur (`cache.rs:65–70`): `inner: BackendHandle` (= `Arc<dyn Backend>`, `core.rs:496`), `cache: Arc<Mutex<CacheState>>`, `child_key`, `limits: CacheLimits`.
- Konstruktoren: `CachingBackend::new(inner)` (73–75) und `for_mount(inner, child_key)` (86–92), das `CacheLimits::MOUNT` setzt.
- Delegation: fast jede Methode ruft `self.inner.<methode>` auf (zum Beispiel `scheme` 95–97, `open_read` 139–141, `rename` 224). `scheme`, `root_display`, `state_identity` und `namespace_identity` liefern die Werte des inneren Backends (95–105). `extensions` (`cache.rs:114–116`) liefert `Some(self)`. Die Implementierung `impl BackendExtensions for CachingBackend` (`native/src/vfs/core/cache_extensions.rs:16`) überschreibt alle 24 Methoden des Traits und ruft jede über die freien Funktionen aus `extension_calls` mit `&*self.inner` auf (`cache_extensions.rs:17–154`). Vier Methoden invalidieren den Cache nach dem Aufruf: `replace_staged_reversible` (`cache_extensions.rs:39–43`, Pfade `staged`, `destination`, `retained`), `finish_stage` (66–68, Pfad `stage`), `recycle` (123–125, Pfad `path`) und `remove_tree` (138–141, `invalidate_prefix(path)`). Doku `cache_extensions.rs:1–2`: „writes and trash moves invalidate it“.
- Schreiben: `open_write` (`native/src/vfs/core/cache.rs:145–160`) und `open_write_new` (161–) entfernen den Listing-Eintrag vor dem Öffnen und umschließen den inneren Schreiber mit `InvalidatingWriter` (`native/src/vfs/core/cache_writer.rs`). Die Kopierstufen `open_write_copy_stage` (`cache.rs:183`), `open_write_copy_stage_sized` (`cache.rs:196`) und `open_write_copy_stage_unsynced` (`cache.rs:203–205`) laufen über `invalidating_writer` (`native/src/vfs/core/cache_paths.rs:68–77`); `open_write_fresh` (`cache.rs:305`) über `wrap_writer` (`cache_paths.rs:79–89`).
- Cache-Inhalt: Verzeichnislisten im Speicher. Die Datei `native/src/vfs/core/cache*.rs` enthält keinen Aufruf von `std::fs`, `File::`, `OpenOptions` oder `tempfile` (per grep geprüft). Der Cache hält `CacheState` (`cache.rs:51–63`) im Speicher.
- Grenzen: TTL `CACHE_TTL` = 20 Sekunden (`cache.rs:42`). `CacheLimits::BROWSING` = 4 096 Verzeichnisse, 50 000 Einträge, 32 MiB (`cache_limits.rs:9–13`); `CacheLimits::MOUNT` = unbegrenzte Verzeichnis- und Eintragszahl, 64 MiB (`cache_limits.rs:15–19`).
- Sync verwendet ihn nicht: Doku `cache.rs:39–41` („NOT used by sync - sync re-opens a fresh backend per run“). Sync-Pfade lösen den Wrapper über `uncached_backend()` auf (`sync_roots.rs:6–8`).

Beispiel 2, `ZipBackend` (`native/src/zipfs/os/shared/zipfs.rs`, Modul `native/src/zipfs/mod.rs`):

- Doku (`zipfs.rs:1–9`): ein schreibgeschütztes ZIP-Archiv als `vfs::Backend`; „Mutations are unsupported (read-only) — extract to edit.“ (`zipfs.rs:9`).
- Struktur (`zipfs.rs:53–61`): `archive: Archive`, `display: String` (der Archivpfad), `dirs: HashMap<String, Vec<VfsMeta>>`. Ein Wrapper über einem inneren `BackendHandle` ist es nicht: `ZipBackend` liest die Datei direkt über `std::fs::File::open(native(zip_path))` (`zipfs.rs:125`).
- Öffnen: `ZipBackend::open(zip_path)` (`zipfs.rs:124–158`) parst das zentrale Verzeichnis einmal; Einträge werden über `enclosed_name()` gefiltert (`zipfs.rs:132–134`); Dateibytes werden beim Lesen dekomprimiert (`native/src/zipfs/os/shared/entry.rs`; Doku `zipfs.rs:7–9`).
- Schema: `scheme()` liefert `Scheme::Local` (`zipfs.rs:166–168`). Es gibt keine `state_identity`-Überschreibung (per grep), also gilt der Standard aus `core.rs:26–28`: `"Local:<Archivpfad>"`.
- Mutationen: `open_write` (`zipfs.rs:218`), `rename` (221), `remove_file` (224), `remove_dir` (227) und `mkdir_all` (233) liefern `Err(readonly())`; `delete_disposition` liefert `Unsupported` (`zipfs.rs:229–231`). Der Test `zipfs.rs:338` prüft `open_write`.
- Aufrufer: `native/src/app/os/shared/remote_zip.rs:9` (`ZipBackend::open`); `native/src/mobile/os/shared/pool.rs:163` (Android-Pool; Doku `pool.rs:17`: Archivgröße und Änderungszeit, mit denen ein `ZipBackend` geöffnet wurde); `native/src/mobile/os/shared/import.rs:255–257` (`extract_all_controlled`).

Beobachtung am Bestand (keine Empfehlung): `CachingBackend` übernimmt `scheme()`, `root_display()`, `state_identity()` und `namespace_identity()` vom inneren Backend (`cache.rs:95–105`). `ZipBackend` meldet `Scheme::Local` (`zipfs.rs:166–168`) und überschreibt `state_identity()` nicht; sein Standardwert ist `"Local:<Archivpfad>"` (`core.rs:26–28`). Eine Schicht, die `scheme()` und `root_display()` vom inneren Backend übernimmt und `state_identity()` nicht überschreibt, meldet deshalb dieselbe Identität wie das innere Backend (Ableitung aus `core.rs:26–28` und `cache.rs:101–103`).

#### 2.1.4 Auswahl eines Backends aus einer Wurzel

`native/src/vfs/core/dispatch.rs:11–23` (`backend_for(root)`): Präfixe `sftp://` → `crate::sftp::backend_from_url`; `ftp://` und `ftps://` → `crate::ftp::backend_from_url`; `smb://` → `crate::smb::backend_from_url`; jede Wurzel ohne diese vier Präfixe → `LocalBackend::new(r)`. Das Präfix `webdav`, `gdrive` und `share` wird hier nicht ausgewertet. `is_remote_root` (`dispatch.rs:28–34`) kennt dieselben vier Präfixe.

`LocalBackend::new(root: &str)` (`native/src/vfs/os/shared/local.rs:89`).

#### 2.1.5 Auflösung eines Locators (Endpunkt)

Der Locator-Typ `EndpointSpec` (`native/src/connect/core/location.rs:7–12`) hat vier Varianten: `Local(String)`, `Saved(String)`, `Drive(String)`, `Peer(crate::share::PeerOpenTarget, String)`.

`EndpointSpec::parse(endpoint)` (`location.rs:15–59`):

1. Leer oder mit NUL-Zeichen: Fehler „Leerer oder ungültiger Pfad“ (16–18).
2. Absoluter Pfad (`/`, `\`, Laufwerksbuchstabe mit `:`): `Local` (19–25).
3. Ohne `://`: `Local`; ein Präfix mit Schema-Zeichen vor einem einzelnen `:` ergibt den Fehler „benötigt ://“ (26–36).
4. Schema-Zeichen außerhalb `[A-Za-z0-9+.-]`: `Local` (39–44).
5. Schema, klein geschrieben: `gdrive` → `Drive("/" + rest)` (47); `share` → `Peer` über `PeerOpenTarget::from_endpoint` (48–50); `sftp`, `ftp`, `ftps`, `webdav`, `smb` → `Saved("{schema}://{rest}")`, Fehler bei leerem Rest (51–56); alle anderen Schemata → Fehler „Nicht unterstütztes Pfadprotokoll“ (57).

Die Liste der gespeicherten Protokolle ist literal in `location.rs:51` festgeschrieben. Eine neue Protokollart braucht dort einen neuen Arm.

`resolve_endpoint(endpoint)` (`native/src/connect/os/shared/resolution.rs:7–44`) liefert `(BackendHandle, String)`:

- `Local(path)`: UNC-Pfade (`crate::net::is_unc`) suchen zuerst eine gespeicherte Verbindung mit längstem passendem Wurzelpfad (`unc_is_same_or_below`, 46–59) und öffnen sie über `connector::open_saved_at`; sonst `LocalBackend::new(&path)` (14–30).
- `Drive(path)` → `connector::open_gdrive(&path)` (31).
- `Peer(target, path)` → `crate::daemon::open_share_backend(target)` (32–35).
- `Saved(endpoint)`: `creds::load_connections_checked()` (37), dann `saved_location(&connections, &endpoint)` (`location.rs:89–134`: genaue Präfixübereinstimmung oder Schema/Benutzer/Host/Port-Vergleich; die Wurzel mit der größten Überlappung gewinnt) und `connector::open_saved_at` (38–41).

`connector::open_gdrive(path)` (`native/src/connect/os/shared/connector.rs:356–364`) ruft `crate::gdrive::GDriveBackend::connect(path)` auf.

`connector::open_saved_at(c, path)` (`connector.rs:369–374`) und `open_saved_at_for_mount` (376–383) rufen `open_saved_at_with_agent_fallback` auf (385–389). Die Protokollauswahl steht in `connector.rs:106–112`: `Sftp` → `connect_sftp`; `Ftp` | `Ftps` → `connect_ftp`; `Webdav` → `connect_webdav`; `Smb` → `connect_smb`; `Share` → `connect_share`.

Rückgaben von `scheme()` (je Backend belegt): SFTP `Scheme::Sftp` (`native/src/sftp/core/backend.rs:199–201`); FTP `Scheme::Ftp` (`native/src/ftp/core/ftp.rs:205–207`); SMB `Scheme::Smb` (`native/src/smb/core/backend.rs:218–220`); WebDAV `Scheme::Webdav` (`native/src/webdav/core/webdav.rs:211–213`); Google Drive `Scheme::GDrive` (`native/src/gdrive/core/backend.rs:10–12`). Zu `Scheme::Peer` ist belegt: `native/src/app/os/shared/remote_open.rs:24` vergleicht den Schemawert mit `Scheme::Peer`; die Zeile `native/src/bisync/os/shared/version_provider_policy.rs:92` enthält `Scheme::Peer`, ihre Bedeutung wurde nicht gelesen. Die Anzeigetexte je Variante stehen in `native/src/app/core/error_report.rs:55–61` (sieben Zweige, kein `_`-Zweig).

#### 2.1.6 Protokolltyp, Konto und Verbindungsmetadaten

`creds::Protocol` (`native/src/creds/core/types.rs:1–10`): `Sftp`, `Ftp`, `Ftps`, `Webdav`, `Smb` (Doku: „SMB 2/3; the root starts with the share“), `Share`.

- `as_str` (13–22) und `parse` (24–34) übersetzen je Variante in `sftp`, `ftp`, `ftps`, `webdav`, `smb`, `share`.
- `default_port` (36–44): `Sftp` 22; `Ftp` und `Ftps` 21; `Webdav` 443; `Smb` 445; `Share` 0.
- `is_url` (46–48): `!matches!(self, Protocol::Share)`.

`AuthKind` (`types.rs:51–59`): `Password` | `Key { path }` | `SshAgent` (Doku: SFTP only, „no stored secret“).

`SavedConnection` (`types.rs:61–75`, `#[derive(Clone, Debug)]`) hat acht Felder: `protocol: Protocol`, `host`, `port: u16`, `user`, `auth: AuthKind`, `root` (Doku 68–69: Startpfad bei sftp/ftp, bei smb `/<share>/<pfad>`, bei share die UNC), `label`, `use_agent: bool` (Doku 72–73: nur SFTP; Standard `false`).

- `account()` (`types.rs:79–88`): `"{proto}://{user}@{host}:{port}{root}"`. Doku: „Stable, unique keyring account key for this connection's secret.“
- `to_target()` (92–105): URL für `sftp`, `ftp`, `ftps`, `webdav` und `smb` (jeweils `is_url()` wahr); für `share` die UNC-Wurzel.
- `display()` (107–113): Label oder `account()`.

Speicherung der Metadaten: `native/src/creds/os/shared.rs:34–36`: `connections.txt` im App-Datenordner, TSV-Format (`serialize`/`parse` in `creds/core/types.rs:120–140`, Feldtrennung `\t`). Sperrdatei `connections.lock` (`shared.rs:23`). Obergrenze 1 MiB (`shared.rs:22`). Die Zeilenformatierung (`sanitize`, `types.rs:116–118`) ersetzt Tabulator und Zeilenumbruch durch Leerzeichen.

Gesamter Ablauf einer neuen Backend-Art (nur Bestand, belegt):

1. Schema im Locator: literale Liste in `location.rs:51`; unbekannte Schemata → Fehler (`location.rs:57`).
2. Protokollvariante: `Protocol` (`types.rs:2–10`), Parsing (`types.rs:24–34`), Port (36–44), URL-Eigenschaft (46–48).
3. Verbindungsaufbau: neuer Arm in `connector.rs:106–112` (heute: fünf Arme).
4. Auflösung eines gespeicherten Locators: `resolution.rs:36–41` (über `saved_location` und `open_saved_at`).
5. Schemawert des Backends: `Scheme`-Variante (`scheme.rs:3–11`). Die Anzeige-Zuordnung in `error_report.rs:55–61` enthält sieben Zweige für alle Varianten und keinen `_`-Zweig; eine neue Variante bräuchte dort also einen Zweig (Rust-Exhaustivität, aus dem Zeilenbefund abgeleitet).
6. Speicherung: `SavedConnection` in `connections.txt`; das Geheimnis unter `SavedConnection::account()` (siehe 2.2).
7. Ortsidentität im Sync: `validate_sync_roots` (`native/src/vfs/os/shared/sync_roots.rs:10–35`) vergleicht bei zwei lokalen Seiten kanonische Pfade und prüft `connect::validate_sync_endpoints`; sonst `namespace_identity()` und `location_paths_overlap` (`location.rs:142–151`, `183–`). Die Lesung hat die Rumpfe der Funktionen `validate_sync_endpoints` und `paths_overlap` nicht vollständig gelesen (Lücke).

Verbindung zum Tresor-Vorbefund: Der Vorbefund in `strang-tresore.md` („Ortsidentität = innerer Endpunkt + Tresorpfad“) ist im Bestand kein vorhandener Zustand. Der Bestand kennt heute keine verschachtelte Endpunktform, sondern eine Zeichenkette je Ort (`EndpointSpec`).

### 2.2 Frage 3: Geheimnisspeicher

Öffentliche Schnittstelle (`native/src/creds/mod.rs` re-exportiert `core` und `os`; Implementierung `native/src/creds/os/shared.rs`):

| Funktion | Zeile | Signatur | Verhalten laut Quelle |
|---|---|---|---|
| `set_secret` | 40 | `pub fn set_secret(account: &str, secret: &str) -> Result<(), String>` | Schreibt über die Plattformschicht. |
| `get_secret_checked` | 69 | `pub fn get_secret_checked(account: &str) -> Result<Option<String>, String>` | `Ok(None)` = kein Eintrag; `Err` bei Sperre oder Lesefehler. |
| `get_secret` | 75 | `pub fn get_secret(account: &str) -> Option<String>` | `get_secret_checked(..).ok().flatten()`; Fehler gehen verloren. |
| `delete_secret_checked` | 79 | `pub fn delete_secret_checked(account: &str) -> Result<(), String>` | Löscht über die Plattformschicht. |
| `delete_secret` | 85 | `pub fn delete_secret(account: &str)` | Ignoriert Fehler; Doku 83–84: Abläufe verwenden `delete_secret_checked`, damit kein falscher Erfolg gemeldet wird. |
| `secret_store_description` | 91 | `pub fn secret_store_description() -> &'static str` | Beschreibung der Plattformschicht. |
| `secret_store_status` | 97 | `pub fn secret_store_status() -> Option<Result<String, String>>` | Statuszeile. |
| `probe_secret_store` | 124 | `pub fn probe_secret_store() -> Result<(), String>` | Liest den Schlüssel `"smart-explorer:credential-store-probe"` (125). |
| `save_connection_with_secret` | 266 | `pub fn save_connection_with_secret(c: &SavedConnection, secret: Option<&str>) -> Result<(), String>` | Ersetzt den Eintrag mit gleichem Konto in `connections.txt` (276–277). Ohne nichtleeres Geheimnis werden nur die Metadaten gespeichert (279–281). Mit Geheimnis: liest das vorherige Geheimnis (283–284) und übergibt Setzen, Metadatenspeichern und Rückbau an `transaction::commit_secret_and_metadata` (286–294). |
| `remove_connection` | 318 | `pub fn remove_connection(account: &str) -> Result<(), String>` | Entfernt den Metadateneintrag (`conns.retain`, 324) und das Geheimnis; liest das vorherige Geheimnis (325–326); `commit_secret_and_metadata` mit `delete_secret_checked` (330) und Rückbau über `restore_secret` (335). |

Die Transaktion selbst (`native/src/creds/core/transaction.rs`, 105 Zeilen) wurde nicht gelesen; belegt ist nur ihr Aufruf in `shared.rs:286–294` und `shared.rs:328–336`.

Plattformschicht, Linux und Android (`native/src/creds/os/linux_os.rs`):

- Speicherort: `FileStore::new(crate::support_dirs::app_data_dir().join("secrets-v1"))` (`linux_os.rs:12–14`). `app_data_dir()` = `data_home().join("smart_explorer")` (`native/src/support_dirs.rs:196–200`); der Basisordner `data_home()` ist plattformabhängig (`support_dirs.rs:160–175`; die Auflösung unter Linux wurde nicht vollständig gelesen, Lücke).
- Operationen: `set` (48–50), `get` (52–54), `delete` (56–58).
- Konto-Prüfung: nur `account.is_empty()` führt zu einem Fehler (`native/src/creds/os/linux_file_store.rs:225–231`).
- Dateiname: `<64 Hex-Zeichen>.secret`; die Zeichen ergeben sich aus `SHA-256("smart_explorer\0" + account)` (`native/src/creds/core/sealed_record.rs:111–120`). Das Konto selbst steht nicht als Dateiname auf der Platte.
- Verschlüsselung: Format `SESEC02` mit ChaCha20-Poly1305 unter einem 32-Byte-Datenschlüssel; die zugehörigen Daten sind Magic, Version, Konto-Digest und Schlüssel-ID (`sealed_record.rs:1–7`, `24–30`). Format `SESEC01` liegt nur im Lesepfad zur Migration (`sealed_record.rs:3–5`, `19–22`).
- Höchstgröße eines Geheimnisses: 64 KiB (`sealed_record.rs:15`).
- Datenschlüssel: Schutzmechanismus in `native/src/creds/os/key_protector.rs` (Datei `APP_KEY_FILE`, `linux_os.rs:32–35`: „the key file lies beside the store“). Der Aufbau des Schutzes wurde nicht gelesen (Lücke).

Plattformschicht, Windows (`native/src/creds/os/windows.rs`):

- `entry(account)` (33–36): `keyring::Entry::new(service, account)`.
- Dienstname: `KEYRING_SERVICE` (`windows.rs:40–41`); der Test in `windows.rs:82` erwartet `"smart_explorer"`. In Debug-Builds verändert ein Testnamensraum den Dienstnamen (`windows.rs:44–52`); in Release-Builds nicht.
- Operationen: `set_secret` (55), `get_secret` (61), `delete_secret` (69).

Schlüsselbenennung:

- Die Schnittstelle nimmt jede nichtleere Zeichenkette als `account`. Es gibt keine Präfix- oder Namensraumregel im Geheimnisspeicher selbst (Linux: `linux_file_store.rs:225–231`; Windows: `windows.rs:33–36`).
- Bestehende Konvention: Verbindungsgeheimnisse nutzen `SavedConnection::account()` (`types.rs:79–88`). Der Probe-Schlüssel `"smart-explorer:credential-store-probe"` (`shared.rs:125`) zeigt, dass Doppelpunkte in Schlüsseln vorkommen.
- Für Tresore existiert im Bestand kein Schlüsselschema. Lücke: ob Windows-Credential-Manager eine Längen- oder Zeichenbeschränkung für `account` hat, wurde nicht geprüft (keine Netz- oder Dokumentationsrecherche in dieser Lesung).
- Weitere Geheimnisse außerhalb des Speichers (Klartext-Datei oder Schlüsselbund, Details in Abschnitt 2.3): Cloud-Refresh-Token im Schlüsselbund (`native/src/cloud/os/shared.rs:61`); Attach-Token einer Shell-Sitzung in einer JSON-Datei unter `<app data>/shell-sessions/<id>.json` (`native/src/cli/shell_session/registry.rs:1–7`).

Beim Öffnen einer gespeicherten Verbindung setzt `connector.rs:394–395` `form.save = false`. Lücke: die Oberflächenoption zum Speichern eines Passworts bei der Neuanlage wurde nicht gelesen.

### 2.3 Frage 2: Klartextgrenze (Überblick; vollständige Tabelle in Abschnitt 3)

Bezugsgrößen (Annahme: aus dem Code abgeleitet, Pfad-Auflösung auf Linux nicht vollständig gelesen):

- `<AD>` ist `crate::support_dirs::app_data_dir()` = `data_home().join("smart_explorer")` (`native/src/support_dirs.rs:196–200`). `data_home()` ist plattformabhängig (`support_dirs.rs:160–175`); `platform_data_home()` wurde nur teilweise gelesen (Lücke: Linux-Ergebnis).
- `<SYNC>` ist `<AD>/sync` (`support_dirs.rs:206–210`).

Befunde:

1. Dateiinhalte liegen dauerhaft oder bis zum Ende einer Sitzung auf dem Datenträger an diesen belegten Stellen:
   - Windows-Laufwerk: Ganzdatei-Spool unter `<AD>/mount-cache/<mount_id>/files/` (`native/src/mount/core/spool.rs:80–88, 127–146`); Schreiben in den Spool (`native/src/mount/core/file_io.rs:138–146`); Lesen des Remote-Inhalts in den Spool mit `open_read_id` und `sync_data` (`native/src/mount/core/materialization.rs:163–178`). Der Mount-Pfad existiert nur unter Windows (`native/src/mount/mod.rs:279–281`: `drive_mount_supported` = `cfg!(windows)`; `native/src/mount/os/` enthält nur `mod.rs` und `windows/`).
   - Geöffnete oder bearbeitete Remote-Dateien, Zwischenablage-Materialisierungen und Remote-Kopie-Brücken: vollständige Kopie mit Originalnamen unter `<AD>/open-temp/` (`native/src/transfer/os/shared/temp.rs:1–11`).
   - Kopie zwischen zwei Backends, wenn Leser und Schreiber nicht gleichzeitig offen sein dürfen: anonyme Datei `tempfile::tempfile()` (`native/src/vfs/os/shared/copy_transfer.rs:19–37`).
   - Analyse-Spool des Share-Hosts: anonyme Datei in `<AD>/analysis-cache/` (`native/src/share/os/shared/analysis_spool.rs:26–30`).
   - Sync-Versionen mit vollständigen Kopien: `.se-versions` auf jeder Seite (`native/src/bisync/core/paths.rs:5`) und `<SYNC>/versions_<pair_id>` (`native/src/bisync/os/shared/persistence.rs:59–61`).
   - Merge-Eingaben: vollständige Fassungen beider Seiten und des Ergebnisses als Dateien `original-a`, `original-b`, `merged` (`native/src/bisync/os/shared/merge_inputs.rs:36–43, 62–85, 175–189`).
   - Konfliktkopien: vollständige verlorene Fassung im Synchronisationsordner, Name mit Suffix ` (Konflikt <Zeitstempel>)` (`native/src/bisync/os/shared/apply_transfer.rs:217, 240–242`).
   - Kopierstufen: vollständige neue Fassung unter einem Stufennamen neben der Nutzerdatei (`native/src/vfs/core/staging_names.rs:1–2`; `native/src/vfs/os/shared/local_stage.rs:1–2`).
   - Host-Papierkorb (Windows): JSON-Intent mit Pfad als UTF-16-Einheiten und SHA-256 (`native/src/host_trash/core/record.rs:19–31`; `native/src/host_trash/os/shared/store.rs:21–39`).
2. Dateinamen und Pfade liegen dauerhaft im Klartext vor in: Baselines (`native/src/bisync/core/types.rs:8–18`), dem SQLite-Index (`native/src/bisync/os/shared/state_store.rs:55–67`), Jobdateien (`native/src/syncjobs/mod.rs:1–6`), Job-Logs (`native/src/bisync/os/shared/run_log_lines.rs:80–81`), dem Transfer-Fehlerprotokoll (`native/src/transfer/os/shared/engine/issues.rs:126`), dem Aufräum-Protokoll (`native/src/app/core/reclaim_core.rs:370–380`), dem Ordnerindex (`native/src/folder_index/os/persistence.rs:143–144`), den Ortsvorgaben (`native/src/connect/os/shared/location_prefs.rs:1–3`), den Verbindungsmetadaten (`native/src/creds/os/shared.rs:8–10`) und dem Mount-Journal (`native/src/mount/core/journal.rs:19–27`).
3. Geheimnisse: Passwörter und Passphrasen liegen verschlüsselt in `secrets-v1` (Linux/Android, `native/src/creds/core/sealed_record.rs:1–7`) oder im Windows-Tresor (`native/src/creds/os/windows.rs:33–36`). Ein Attach-Token steht im Klartext in `<AD>/shell-sessions/<id>.json` (`native/src/cli/shell_session/registry.rs:1–7`). Die Befehlsfelder `run_before`, `run_after`, `run_cleanup` der Jobdatei stehen im Klartext (`native/src/syncjobs/core/types.rs:160–165`; Annahme: Befehlszeilen können Geheimnisse enthalten; das wurde nicht geprüft).
4. Ohne dauerhafte Ablage gefunden: der VFS-Listencache (`native/src/vfs/core/cache*.rs` enthält keinen Aufruf von `std::fs`, `File::`, `OpenOptions` oder `tempfile`; Daten im Speicher, `native/src/vfs/core/cache.rs:51–63`). Vorschaubilder: die einzige Fundstelle des Suchbegriffs „thumb“ ist das Ignoriermuster `**/Thumbs.db` (`native/src/app/core/job_editor_pages.rs:372`).
5. Die vollständige Liste der Ablageorte, mit Pfad, Inhalt und Lebensdauer und mit Kennzeichnung „Inhalt nicht gelesen“, steht in Abschnitt 3.

### 2.4 Frage 4: Sync (Job-Modell, Endpunkte, Konflikte, Versionen, Hintergrund)

**Job-Modell.** `SyncJob` (`native/src/syncjobs/core/types.rs:68–166`) hat 51 öffentliche Felder: `id`, `name`, `source`, `target`, `direction`, `conflict`, `retain_days`, `interval_min`, `include_hidden`, `ignore`, `last_run`, `enabled`, `trigger`, `cal_time_min`, `cal_weekdays`, `cal_monthday`, `rt_debounce_secs`, `rt_max_latency_secs`, `rt_poll_secs`, `verify_interval_secs`, `verify_target_secs`, `connect_match`, `active_from_min`, `active_to_min`, `catch_up`, `delete_policy`, `move_files`, `compare`, `modify_window_sec`, `versioning_scheme`, `retain_count`, `use_recycle_bin`, `max_delete`, `max_delete_pct`, `max_delete_min`, `versions_location`, `cross_mounts`, `config_version`, `filter_min_size_kb`, `filter_max_size_kb`, `filter_max_age_days`, `filter_min_age_days`, `bwlimit_kbps`, `max_transfers`, `atomic_copy`, `verify`, `retries`, `retry_delay_secs`, `run_before`, `run_after`, `run_cleanup`.

Enumerationen (vollständig):

- `Direction`: `AtoB` („a2b“), `BtoA` („b2a“), `Both` („both“) (`native/src/bisync/core/types.rs:57–62`).
- `ConflictMode`: `FileLevel` („strict“), `NewerWins` („newer“), `OlderWins` („older“), `LargerWins` („larger“), `SmallerWins` („smaller“), `SourceWins` („source“), `DestWins` („dest“), `KeepBoth` („keepboth“) (`native/src/bisync/core/types.rs:90–105`; Text-Zuordnung 107–131; `ALL` mit acht Einträgen ab Zeile 145).
- `DeletePolicy`: `Propagate`, `Mirror`, `NoDelete` (`native/src/bisync/core/types.rs:163–167`).
- `CompareMode`: `MtimeSize`, `SizeOnly`, `Checksum` (`native/src/bisync/core/types.rs:198–202`).
- `VersioningScheme`: `Days`, `Count`, `Staggered`, `Gfs` (`native/src/bisync/core/types.rs:233–242`).
- `VersionsLocation`: `Auto`, `AppData` (`native/src/bisync/core/types.rs:280–287`).
- `Trigger`: `Manual`, `Interval`, `Calendar`, `RealTime`, `OnStartup`, `OnConnect` (`native/src/syncjobs/core/types.rs:13–21`).

**Standardwerte bei Neuanlage** (`SyncJob::new`, `native/src/syncjobs/core/types.rs:181–`): `direction` = `Both` (187), `conflict` = `FileLevel` (188), `retain_days` = 30 (189), `include_hidden` = true (191), `enabled` = true (194), `trigger` = `Manual` (195), `cal_time_min` = 540 (196), `rt_debounce_secs` = 10 (199). Die Doku (177–180) nennt zusätzlich einen Löschstopp bei 25 Dateien und 50 %; die Feldwerte `max_delete` und `max_delete_pct` wurden nicht gelesen (Lücke). Die Engine-Standardwerte (`BisyncOptions::default`, `native/src/bisync/core/types.rs:378–390`) sind `Both`, `FileLevel`, `reversible` = true, `Propagate`, `MtimeSize`.

**Endpunkte.** `source` und `target` sind Zeichenketten. Doku (`types.rs:72–75`): „Side A: a local path or a remote target (e.g. sftp://user@host:port/p)“. Prüfung: nichtleer, höchstens `MAX_ENDPOINT_LEN` Bytes, ohne NUL (`native/src/syncjobs/core/validation.rs:53–57`), danach `connect::validate_sync_endpoints` (`validation.rs:63`; `native/src/connect/core/location.rs:142–151`). Auflösung: `EndpointSpec` und `resolve_endpoint` (Abschnitt 2.1.5).

**Konflikterkennung und -regeln.**

- Der Plan entscheidet je Datei: `one_way_both_changed` (`native/src/bisync/core/plan_decide.rs:157–176`, einseitige Jobs) und `two_way_both_changed` (178–193, beidseitige Jobs). `Decision::Conflict` (`plan_decide.rs:24`) wird in `plan_pair.rs:211` als `Conflict { rel, a, b, duplicates }` eingeplant (`native/src/bisync/core/types.rs:468–473`).
- `FileLevel` (Standard): beidseitige Änderung ergibt `Decision::Conflict` (`plan_decide.rs:162` und `180`); die Textfassung lautet „Streng: beidseitige Änderung = Konflikt (sicher)“ (`types.rs:135`).
- `NewerWins`, `OlderWins`, `LargerWins`, `SmallerWins`: Entscheidung über `winner_under` nach mtime (`plan_decide.rs:204–205`) bzw. Größe (206–207). `SourceWins` und `DestWins` entscheiden über die Seite (200–201). Ist nur eine Seite vorhanden, gewinnt diese (202–203).
- `KeepBoth`: einseitig Quelle als Gewinner (`plan_decide.rs:163–164`); beidseitig Gewinner nach mtime (182–189); der Verlierer bleibt als Konfliktkopie erhalten.
- Ergebnis: Im Modus `FileLevel` entscheidet der Plan beidseitige Änderungen nicht; die sieben anderen Modi entscheiden sie automatisch, `KeepBoth` behält beide Fassungen.

**Konfliktvarianten.**

- Gleichnamige Duplikate: `DuplicateConflict { a: Vec<FileVariant>, b: Vec<FileVariant> }` (`native/src/bisync/core/duplicate_types.rs:19–23`); `FileVariant { id: Option<String>, signature: Sig, content_size: u64, content_md5: String }` (`duplicate_types.rs:5–11`). `needs_variant_choice` ist wahr, wenn Inhalte abweichen (31–34); `common_choice` liefert genau einen gemeinsamen Inhalt (37–44).
- Konfliktkopie-Benennung: ` (Konflikt {stamp})`, bei Mehrfachen mit laufender Nummer ` (Konflikt {stamp} {n})` (`apply_transfer.rs:217, 240–242`).

**Auflösung (Kern).** `resolve` (`native/src/bisync/os/shared/resolve.rs:27`), `resolve_checked` (78; prüft die beim Nutzer angezeigten exakten Signaturen, Doku 72–77), `resolve_recorded` (99; unter der Paarsperre, `WouldBlock` während eines laufenden Laufs, Doku 94–98) und `resolve_variant_checked` (257; Parameter `keep_a: bool`, `variant_id: Option<&str>`). Doku `resolve.rs:23–25`: „Resolve one conflict by copying the chosen side over the other with a reversible backup of the loser.“ Phasen: `ResolvePhase` (`resolve.rs:15–21`: `Preparing`, `BackingUp`, `Copying`, `Deleting`, `ReadingSignatures`). Die Paarsperre gilt für Lauf, Konfliktlösung und Versions-Wiederherstellung (`native/src/bisync/os/shared/pair_lock.rs:1–3`).

**Merge.** `merge_recorded_for_key` (`native/src/bisync/os/shared/merge_recorded.rs:56`), `MergeChoice<'a> { Write(&'a [u8]), KeepBoth { keep_a: bool } }` (`merge_recorded.rs:19–23`), `RecordedMergeChoice { Write, KeepBoth { keep_a: bool } }` (`native/src/bisync/os/shared/merge_inputs.rs:9–13`). Doku `merge_recorded.rs:1`: „Recorded line merge and keep-both without replacing changed originals.“ Ob der Merge nur Textdateien zulässt, wurde nicht geprüft (Lücke).

**Versionen.**

- `VersionReason`: `Replaced`, `Deleted`, `Resolved`, `Restored` (`native/src/bisync/os/shared/versions.rs:134–143`).
- `VersionStore`: `SyncRoot` (`.se-versions` am Wurzelpfad der Seite) und `AppData` (`versions_<pair>`, `versions.rs:147–152`).
- `VersionEntry` (`versions.rs:156–173`): `side`, `rel` (Originalpfad relativ zur Wurzel), `run_id`, `preserved_ms`, `reason`, `size`, `mtime_ms`, `store`, `stored_path`, `job_id`.
- Funktionen: `prune_after_run` (`versions.rs:184`), `list_versions` (196), `restore_version` (207; die ersetzte Datei wird als Version gehalten, Doku 204–206), `remove_versions` (220).
- Aufbewahrung: `retain_days` (Standard 30), `retain_count`, `versioning_scheme` und `versions_location` (Felder siehe oben). CLI-Hilfetext für `--versions-location`: „auto = .se-versions at the synced folder, else app data“ (`native/src/cli/sync_jobs/options.rs:81`).

**Hintergrundausführung.**

- Alle Läufe gehen über dieselbe Engine: „Every runner (background service, desktop window, Android, terminal) writes through the same engine“ (`native/src/bisync/os/shared/run_log.rs:3–4`).
- Warteschlange des Dienstes: `job_supervisor.rs` (`enqueue` 98, `enqueue_cause` 102, `poll` 130).
- Konflikte in Hintergrundläufen des Dienstes: die Konfliktzahl steht in der Protokollzeile „ran … conf …“ (`native/src/daemon/os/shared/job.rs:255–264`); die Zustandsklassifikation nennt einen Lauf mit Konflikten „Konflikte“ (`native/src/syncjobs/os/shared/job_state_classify.rs:71`); das Protokoll schreibt „Konflikt: beide Seiten geändert, nichts überschrieben“ (`native/src/bisync/os/shared/run_log_lines.rs:119`).
- Auslöser: `Trigger` (oben); `connect_match` (`types.rs:111`) für „Bei Anschluss“; Doku `native/src/daemon/os/shared/connect_triggers.rs:1–2`: „Volume arrivals are bound to a configured local endpoint and volume identity.“
- Catch-up im laufenden Prozess: `native/src/daemon/os/shared/live.rs:42` („Ask the worker loop in this process for one catch-up run.“).
- Android: Läufe mit Methoden `sync.*` halten die CPU wach (`android/app/src/main/java/app/smartexplorer/android/api/SyncApi.kt:449`).

**Jobs mit automatisch angelegtem Ziel ohne Ordnerwahl.**

- Befund: Das Modell kennt kein Kennzeichen dafür. Die Validierung verlangt für `source` und `target` jeweils einen nichtleeren Pfad (`native/src/syncjobs/core/validation.rs:53–57`).
- Suche: Die Muster `anlegen`, `neu anlegen`, `create.*target`, `Ziel fehlt`, `Ziel.*leer`, `target.*empty`, `target.*required` und `missing` (ohne Groß-/Kleinschreibung) ergaben in `native/src/syncjobs/core/validation.rs`, `native/src/syncjobs/os/shared/editor.rs`, `native/src/mobile/os/shared/domains/job_json.rs`, `native/src/mobile/os/shared/domains/sync_jobs.rs` und `native/src/app/core/job_editor_pages.rs` keinen Treffer, der ein Anlegen des Ziels beschreibt.
- Lücke: Die Anlagewege in CLI (`se sync add`), Android-Editor und Bridge-Eingabe wurden nicht vollständig gelesen.
- Lücke: Das Zustandsfeld `target_managed` (`native/src/bisync/os/shared/state_types.rs:29`; gelesen in `incremental.rs:446–452`, gesetzt in `incremental_index_commit.rs:335`) ist im gelesenen Code nicht erklärt.

### 2.5 Frage 5: Oberflächen für Sync-Konflikte und Versionen

| Oberfläche | Konflikte | Versionen | Beleg |
|---|---|---|---|
| Desktop-GUI | Konfliktmodus im Job-Editor (Auswahl „Konflikte“, `ConflictMode::ALL`); Konfliktdialog mit Fortschritt und Variantenwahl; Auflösung im Worker | Ansicht „Versionen“ mit Öffnen (`open_sync_versions`) und Anzeige (`ui_sync_versions`); Seite „Vergleich & Versionen“ laut Doku | `native/src/app/core/job_editor_pages.rs:150–156`; `native/src/app/core/menus_sync_jobs.rs:165`; `native/src/app/core/bisync_conflict_ui.rs:9, 165, 220`; `native/src/app/core/bisync_conflicts.rs:98, 107, 224, 293`; `native/src/app/core/merge_ui.rs:6`; `native/src/app/core/bisync_merge.rs:7, 132`; `native/src/app/core/sync_versions_ui.rs:54, 78`; `docs/oberflaechen/einstellungen.md:144` |
| Android-GUI | `ConflictsScreen` (Liste, „Alle A“, „Alle B“), `ConflictSession` (Sitzung), `ConflictVariantChoice` (Versionsauswahl), `MergeScreen` | `VersionsScreen` (Liste, Wiederherstellung über `SyncApi`); Editor `JobDraft` mit Feld `conflict` | `android/app/src/main/java/app/smartexplorer/android/ui/sync/ConflictsScreen.kt:49, 120, 129–130`; `ConflictSession.kt:26`; `ConflictVariantChoice.kt:19`; `MergeScreen.kt:164`; `VersionsScreen.kt:36`; `JobDraft.kt:39`; `SyncApi.kt:335–373` (Konflikte), `SyncApi.kt:307, 322` (Versionen) |
| Bridge (Android, JNI) | Methoden `sync.conflicts`, `sync.checkConflicts`, `sync.resolve`, `sync.skip`, `sync.finishConflicts`, `sync.mergeRows`, `sync.mergeApply`, `sync.mergeKeepBoth`, `sync.mergeRetry` | `sync.versions`, `sync.restoreVersion` | `native/android-bridge/src/lib.rs:139–148` (`Java_app_smartexplorer_android_core_NativeBridge_call` ruft `smart_explorer::mobile::call`); `native/src/mobile/os/shared/domains/mod.rs:110–133` |
| Mobile-Implementierung | `sync_conflicts::conflicts` (275), `check` (305; Zählung `conflicts`/`errors` in den Zeilen 356–360), `resolve` (375; Wahl `"a"` oder `"b"` in den Zeilen 379–381), `skip` (421), `finish` (439); `save_resolved` (205) wird aus `settle_reserved` (248–251, Zeile 249) aufgerufen | `sync_versions::list` (21), `restore` (68) | `native/src/mobile/os/shared/domains/sync_conflicts.rs`; `native/src/mobile/os/shared/domains/sync_versions.rs` |
| CLI | Die Unterbefehle enthalten keine Konflikt- oder Versionsfunktion (keine Konfliktliste, keine Konfliktlösung, keine Versionsliste, keine Wiederherstellung). Einstellbar: `--conflict` (Werte `strict`, `newer`, `older`, `larger`, `smaller`, `source`, `dest`, `keepboth`), `--versioning-scheme` (`days`, `count`, `staggered`, `gfs`), `--retain-days`, `--retain-count`, `--versions-location` (`auto`, `appdata`); Unterbefehle `list`, `show`, `add`, `edit`, `remove`, `enable`, `disable`, `log` | Ausgabe-JSON enthält `conflict`, `versioning_scheme`, `retain_days`, `retain_count`, `versions_location` und `state` | `native/src/cli/sync_jobs.rs:45–64` (Unterbefehle), `:159` (Auswahl), `:242` (`log`); `native/src/cli/sync_jobs/options.rs:15–21, 50–51, 67–68, 72–74, 78–82`; `native/src/cli/sync_jobs/output.rs:20–30, 70` |
| Daemon-IPC | Keine Methode für Sync-Konflikte, Versionen oder Merge gefunden. Treffer der Suche nach „conflict“, „versions“, „merge“ und „sync.“ betreffen Identitäts-, Anwesenheits- und Anfrageselektor-Konflikte (`ipc_host_direct_events.rs:259`, `ipc_host_relation_events.rs:228`, `ipc_host_legacy_events.rs:167`) | keine Treffer | `native/src/daemon/os/shared/ipc*.rs` (Suche per grep; Dispatch nicht vollständig gelesen) |
| MCP | Kein eigenes Werkzeug für Sync-Konflikte gefunden. Der Katalog nennt „sync setups (`sync`)“ und das allgemeine Werkzeug `se <args>` | kein eigenes Werkzeug gefunden | `native/src/cli/mcp/catalog.rs:40–41, 180`; `native/src/cli/mcp/status_tools.rs:35` (`sync_setups`) |
| Fremdbegriff | `fs.conflicts` (Aufruf `fs_edit::conflicts`; die Zuordnung zur Dateibearbeitung folgt aus dem Modulnamen, die Funktion wurde nicht gelesen) | – | `native/src/mobile/os/shared/dispatch.rs:33` |

Vorschau und Vergleich (Desktop): `Preview` (`native/src/bisync/os/shared/preview.rs:17–31`) ist ein schreibgeschützter Vergleich mit demselben Planer wie ein Lauf (`preview`, Zeilen 33–40; `preview_with`, Zeile 56). `apply_preview_action` (Zeilen 237–242) wendet genau eine angezeigte Aktion unter der Paarsperre an. Aufrufer per Suche: `native/src/app/core/preview_core.rs`, `native/src/app/core/state.rs`, `native/src/app/mod.rs`. Android-, CLI-, Daemon- und MCP-Aufrufe wurden per Suche nach `preview::`, `preview_with` und `sync_preview` nicht gefunden.

Zusätzlich: In der Tabelle `docs/oberflaechen/einstellungen.md:153` steht für „Jetzt ausführen, Vergleichen, Versionen wiederherstellen, Konflikte lösen, Sicherheitsstopp bestätigen“ bei der CLI der Eintrag „–“ und als Ergebnis „offen“. Die Strategie-Doku `docs/SYNC_FEATURES.md:74–82` (Gruppe E) nennt Statuswerte „[HAVE]“ und „[BUILD]“; diese Statuswerte wurden nicht gegen den Code geprüft.

### 2.6 Vorbefunde aus den Cryptomator-Dokumenten, gegen den Bestand geprüft

Gelesen: `docs/CRYPTOMATOR_VAULTS_ERKUNDUNG.md`, Abschnitte „Anschluss an die bestehende Smart-Explorer-Architektur“ (Zeilen 198–233) und „Entfernte Vaults und Synchronisation“ (234–273); `docs/refs/cryptomator-vault-api-2026-10-04.md` (83 Zeilen, gesamt gelesen).

Am Quelltext bestätigt:

- „Aktuell kein Vault-Endpunkttyp“: `EndpointSpec` hat die Varianten `Local`, `Saved`, `Drive`, `Peer` (`native/src/connect/core/location.rs:7–12`).
- „Materialization kopiert den Stream aus `backend.open_read_id`“ in eine `.spool`-Datei mit `sync_data`: `native/src/mount/core/materialization.rs:163, 171, 178`.
- „`file_io::write` schreibt Eingabebytes in dieselbe lokale Arbeitsablage“: `native/src/mount/core/file_io.rs:138–146`.
- „Betriebssystemlaufwerke werden nur auf Windows unterstützt“: `native/src/mount/mod.rs:279–281`.
- „Keine Cryptomator-Ent- oder Verschlüsselung“: die Suche nach `masterkey`, `.c9r`, `c9s`, `vault.cryptomator`, `dirid`, `aes-siv`, `scrypt`, `xchacha` und `SIV` in den Dateitypen `.rs`, `.kt`, `.java` und `.toml` unter `native/` und `android/` ergab keinen Formattreffer; der einzige Treffer des Musters `SIV` ist `native/src/syncjobs/os/windows/job_state_lock.rs` (Teil von `LOCKFILE_EXCLUSIVE_LOCK`, kein Bezug).

Abweichung: Der Bericht nennt im Mount-Bestand „MountSource umfasst SavedRemote/Drive/Peer“. Der Code nennt die Varianten `SavedRemote`, `GoogleDrive` und `Peer` (`native/src/mount/core/types.rs:135–`; die Variantenliste wurde nur bis `Peer` gelesen).

Nicht am Bestand prüfbar (externe Quellen, keine Netzrecherche in dieser Lesung): die Aussagen zu Cyberduck/Mountain Duck, zu den Formaten 7 und 8 als Produktstand, zur Cryptomator-Konfliktdokumentation und die Versions-, Signatur- und Dateiangaben der Referenznotiz (CryptoFS 2.10.0, CryptoLib 2.2.2, CLI 0.6.2). Die Referenznotiz nennt für `MasterkeyFileAccess.persist` ein `.tmp`-Schreiben mit `Files.move(..., REPLACE_EXISTING)` ohne `ATOMIC_MOVE` und ohne fsync; dies ist eine Aussage über externen Code und wurde hier nicht nachvollzogen.

## 3. Tabelle „Klartext-Ablageorte“

Konventionen: `<AD>` = `<app data>` = `support_dirs::app_data_dir()` (`native/src/support_dirs.rs:196–200`); `<SYNC>` = `<AD>/sync` (`support_dirs.rs:206–210`). Aufgeführt sind alle Ablageorte, die die Suche nach App-Datenpfaden und nach Sitzungsordnern ergab, sowie die Stellen aus Abschnitt 2.3. Ausgeschlossen sind Orte ohne Nutzerinhalt und ohne Dateinamen des Nutzers (Sperrdateien ohne Textinhalt wie `connections.lock` in `creds/os/shared.rs:23`, `<AD>/mount-cache/.<id>.host.lock` in `cache_lease.rs:38` und `<SYNC>/locks/pair-<id>.lock` in `pair_lock.rs:125–127`, Zähler `creds-revision` in `creds/os/shared.rs:47`). Spalte „Inhalt“ mit „nicht gelesen“ bedeutet: Der Ort ist belegt, sein Inhalt wurde nicht gelesen.

| Nr | Ort | Inhalt | Lebensdauer | Beleg |
|---|---|---|---|---|
| K01 | `<AD>/mount-cache/<mount_id>/files/<32 Hex-Zeichen>.spool` (nur Windows) | vollständiger Inhalt einer geöffneten oder geschriebenen Laufwerksdatei; Name aus 16 Zufallsbytes als Hex | bis Commit oder Verwerfen; verwaiste Spool-Dateien werden beim Öffnen und im `Drop` entfernt | `native/src/mount/core/spool.rs:80–88, 127–146, 117–119, 225–245, 262–268, 312–316`; `native/src/daemon/os/shared/mount_manager_start_cache.rs:13–14, 79–80` |
| K02 | derselbe Ordner, Clean-Cache-Einträge (`IdleClean`) | unveränderter Inhalt nach dem Schließen als Spool-Datei | höchstens 300 s (`MAX_CONTENT_AGE`); Gesamtbudget `retained_mib` (Standard 500, Maximum 65 536 MiB); höchstens 10 000 Einträge | `native/src/mount/core/clean_cache.rs:10–11, 13–22, 79–83`; `native/src/mount/core/cache_policy.rs:6–7, 19, 46`; `native/src/mount/core/materialization.rs:154–159` |
| K03 | `<AD>/mount-cache/<mount_id>/journal.jsonl` (nur Windows) | JSON-Zeilen mit `remote_path` (Klartextpfad), `spool_name`, Baseline, Zustand, Löschvorgängen (`original_path`, `quarantine_path`) und Namenskonfliktabsichten (`namespace_conflicts`, Felder nicht gelesen) | bis Vergessen oder Kompaktierung (Schwelle 1 MiB, Obergrenze 64 MiB) | `native/src/mount/core/journal.rs:8–11, 19–47, 162`; `native/src/mount/core/spool.rs:87` |
| K04 | `<AD>/mount-cache/private-runtime/` (nur Windows) | Laufzeitdateien des Dokany-Helfers; Inhalt nicht gelesen | nicht gelesen | `native/src/mount/os/windows/private_payload.rs:50` |
| K05 | `<AD>/open-temp/s<pid>_<nanos>/e<16 Hex>/<Originalname>` | vollständige Kopie geöffneter oder bearbeiteter Remote-Dateien, Zwischenablage-Materialisierungen und Remote-Kopie-Brücken; Originalname, nur `/`, `\` und `:` werden ersetzt | bis zum sauberen Beenden (`cleanup_session_temp`); Reste früherer Sitzungen werden beim Start entfernt | `native/src/transfer/os/shared/temp.rs:1–11, 26–28, 52–66, 99–102`; `native/src/app/os/shared/remote_helpers/temp.rs:39` |
| K06 | `<AD>/open-temp/s<pid>_<nanos>/session.pid` | Prozess-ID, Sitzungs-Tag und Startzeit; keine Nutzerdaten | wie K05 | `native/src/transfer/os/shared/temp.rs:5–6, 30–50` |
| K07 | Wiederherstellungsmanifest der Fernbearbeitung im Sitzungsordner | Felder `RecoveryManifest` und `RecoveryManifestEntry` nicht gelesen; Pfade vermutlich enthalten (Annahme) | bis zur Sitzungsbereinigung (Annahme) | `native/src/app/os/shared/remote_helpers/recovery_manifest.rs:26–32, 194–225` |
| K08 | anonyme Datei beim Kopieren zwischen Backends, wenn Leser und Schreiber nicht gleichzeitig offen sein dürfen | vollständiger Dateiinhalt; kein Name im Dateisystem | bis zum Ende der Kopierfunktion | `native/src/vfs/os/shared/copy_transfer.rs:19–37`; Ort: Annahme, das Systemtemp-Verzeichnis des Crates `tempfile` (nicht im Repository belegt) |
| K09 | `<AD>/analysis-cache/` (anonyme Datei, Share-Host) | gerahmte Antworten; `FsResponse::Entries` mit `FsMeta`-Einträgen (Namen und Metadaten; Felder von `FsMeta` nicht gelesen) | solange der Dateideskriptor offen ist; danach ohne Namen | `native/src/share/os/shared/analysis_spool.rs:25–31, 41–52`; `native/src/share/core/fs_response.rs:47–49` |
| K10 | `<AD>/host-trash/<id>.json` (Windows) | JSON-Intent mit Wurzel und relativem Pfad als UTF-16-Einheiten, SHA-256, Größe, Identitäten und Haltenamen; Ort des gehaltenen Objekts nicht gelesen | bis Wiederherstellung oder Abschluss (nicht gelesen) | `native/src/host_trash/core/record.rs:19–31`; `native/src/host_trash/os/shared/store.rs:14–39`; `native/src/host_trash/mod.rs:1` |
| K11 | `<AD>/folder_index.txt` (Desktop) | ein Pfad je Zeile, höchstens 1 000 000 Pfade | bis zum nächsten Aufbau oder Überschreiben | `native/src/app/os/shared/platform_helpers.rs:118–120`; `native/src/folder_index/os/persistence.rs:143–144`; `native/src/folder_index/core/model.rs:4` |
| K12 | `rt.config().data_dir()/folder_index.txt` (Mobil) | wie K11 | wie K11 | `native/src/mobile/os/shared/index.rs:1–2, 38–39` |
| K13 | `<AD>/favorites.txt` und `<AD>/dir_sort.tsv` | Ortsschlüssel je Zeile (`favorites.txt`); Ortsschlüssel mit Tabulator und Flag (`dir_sort.tsv`) | dauerhaft | `native/src/connect/os/shared/location_prefs.rs:1–3, 8, 29` |
| K14 | `<AD>/recent.txt` | Inhalt nicht gelesen; Zuordnung über `settings_path` | nicht gelesen | `native/src/app/os/shared/platform_helpers.rs:114–116` |
| K15 | `<AD>/connections.txt` | TSV mit Protokoll, Host, Port, Benutzer, Auth-Art, Schlüsselpfad, Wurzel und Bezeichnung; kein Geheimnis | dauerhaft bis Entfernen | `native/src/creds/os/shared.rs:8–10, 34–36`; `native/src/creds/core/types.rs:61–75, 120–140` |
| K16 | `<AD>/secrets-v1/<64 Hex-Zeichen>.secret` (Linux und Android) | ChaCha20-Poly1305-verschlüsseltes Geheimnis (Passwort, Passphrase), Höchstgröße 64 KiB; Dateiname = SHA-256 über `smart_explorer\0` und Konto | bis Entfernen | `native/src/creds/os/linux_os.rs:12–14`; `native/src/creds/core/sealed_record.rs:1–7, 15, 111–120` |
| K17 | Windows-Anmeldeinformationsverwaltung, Dienst `smart_explorer` | Geheimnis im Betriebssystem-Tresor, nicht als Datei | bis Entfernen | `native/src/creds/os/windows.rs:33–36, 40–41` |
| K18 | `<AD>/shell-sessions/<id>.json` und `<id>.lock` | Attach-Port, Attach-Token (Klartext in JSON), Name, Eigentümer, Ziel, Art, Dialekt; Rechte 0700 und 0600 (Unix) | solange die Besitzersperre gehalten wird; sonst beim nächsten Auflisten entfernt | `native/src/cli/shell_session/registry.rs:1–7, 73` |
| K19 | `<AD>/cloud/<provider>.cfg` | Konfiguration des Cloud-Clients (`ClientConfig`); Inhalt nicht gelesen. Refresh-Token im Schlüsselbund | dauerhaft | `native/src/cloud/os/shared.rs:4–12, 61` |
| K20 | `<AD>/transfer-logs/` (Dateiname nicht gelesen) | JSON-Zeile je nicht übertragenem Eintrag mit `path` und `message` | dauerhaft; Rotation nicht gelesen | `native/src/transfer/os/shared/engine/issues.rs:1–4, 41, 126` |
| K21 | `<AD>/reclaim/actions.jsonl` | Wurzel, Auswahl, zu löschende Pfade, verifizierte Duplikatpfade, übersprungene Pfade, Zeitstempel | fortlaufend angehängt; Löschung nicht gelesen | `native/src/app/core/reclaim_core.rs:364–380` |
| K22 | `<AD>/share_server.txt` | Inhalt nicht gelesen | nicht gelesen | `native/src/app/core/support_paths.rs:51`; `native/src/cli/share.rs:189` |
| K23 | `<AD>/lan_uplink/` (Windows) | Inhalt nicht gelesen | nicht gelesen | `native/src/net/os/windows/uplink_install.rs:88`; `native/src/net/os/windows/uplink_helper.rs:45` |
| K24 | `<AD>/crash.log` (Desktop); `<data>/crash.log` (Mobil) | Inhalt nicht gelesen | nicht gelesen | `native/src/lib.rs:160`; `native/src/mobile/os/shared/sys.rs:95` |
| K25 | `<AD>/update_*.txt`, `<AD>/staged_update.json` | Update-Zustand; Inhalt nicht gelesen | nicht gelesen | `native/src/updater/os/shared/config.rs:8, 20` |
| K26 | `<AD>/mobile/` (Mobil) | Inhalt nicht gelesen | nicht gelesen | `native/src/mobile/core/config.rs:76` |
| K27 | `<AD>/mounts/` | Inhalt nicht gelesen; Zweck nur aus dem Modulnamen `mount_registry` abgeleitet (Annahme) | nicht gelesen | `native/src/daemon/os/shared/mount_registry.rs:137` |
| K28 | `<SYNC>/jobs/<id>.conf` | `key=value`-Zeilen: Quelle, Ziel, Richtung, Konfliktmodus, Ignoriermuster, Filter, Befehle `run_before`, `run_after`, `run_cleanup` | dauerhaft bis Entfernen | `native/src/syncjobs/mod.rs:1–11`; `native/src/syncjobs/os/shared/persistence.rs:22–25`; `native/src/syncjobs/core/types.rs:69–166` |
| K29 | `<SYNC>/jobs.tsv` (Altformat) | positionsbasierte TSV; einmaliger Import | bis zum Import | `native/src/syncjobs/os/shared/persistence.rs:17–20`; `native/src/syncjobs/os/shared/migration.rs:270–289` |
| K30 | `<SYNC>/results.tsv` (Altformat) | letzte Ergebnisse; nur zum Befüllen fehlender Zustände gelesen; Inhalt nicht gelesen | nicht gelesen | `native/src/syncjobs/os/shared/results.rs:1–6, 33` |
| K31 | `<SYNC>/sync_state.sqlite` | Tabelle `pairs` (`root_a`, `root_b`, Modus, Quellseite, Cursor, Wurzel-IDs, `bootstrapped`, `target_managed`) und Tabelle `items` (`pair`, `side`, `rel`, `id`, `parent_id`, `name`, `size`, `mtime_ms`, `hash`, `is_dir`, `deleted`, `updated_ms`); Pfade und Namen im Klartext | dauerhaft; Ende der Lebensdauer nicht gelesen (Lücke) | `native/src/bisync/os/shared/state_store.rs:37–67, 360` |
| K32 | `<SYNC>/pairs/<pair_id>/<Replik>.<Kennung>.sebl` (Baseline) | Abbildung von relativem Pfad auf (Signatur Seite A, Signatur Seite B); Signatur = Größe, mtime, 64-Bit-Hash | bis Zurücksetzen oder Entfernen (Lücke: genaue Lebensdauer) | `native/src/bisync/os/shared/replica_state.rs:17–27, 140–144`; `native/src/bisync/core/types.rs:8–18` |
| K33 | `<SYNC>/pairs/<pair_id>/<Besitzer>.replace-<nonce>.json` (Ersetzungsjournal) | Absicht mit relativem Pfad (`binding.rel`) und Paar-ID | bis Veröffentlichung oder Wiederherstellung | `native/src/bisync/os/shared/replacement_journal.rs:1, 76–84` |
| K34 | Checkpoint-Journal (Speicherort nicht ermittelt, Lücke) | angehängte Rahmen mit Länge, SHA-256 und Byte-Inhalt; Inhalt nicht gelesen | nicht gelesen | `native/src/bisync/os/shared/checkpoint_journal.rs:1–3, 163–165`; `native/src/bisync/os/shared/checkpoint_run.rs:8, 74` |
| K35 | `<SYNC>/job-logs/<id>.log` (und `<id>.log.1` nach Rotation) | Zeilen `<rel>: <Aktion>` und Vergleichszeilen mit relativen Pfaden, Konfliktzeilen, Zusammenfassungen | Rotation bei `JOB_LOG_ROTATE_BYTES` (Wert nicht gelesen) | `native/src/bisync/os/shared/run_log.rs:1–8, 82`; `native/src/bisync/os/shared/run_log_lines.rs:80–81, 111, 119, 172` |
| K36 | `<SYNC>/versions_<pair_id>/` (Versionen in den App-Daten) | ersetzte, gelöschte, verlorene und wiederhergestellte Dateien als Kopie mit Originalpfad | nach Aufbewahrungsregel; Löschung über `remove_versions` | `native/src/bisync/os/shared/persistence.rs:59–61`; `native/src/bisync/os/shared/versions.rs:145–152, 154–173, 220` |
| K37 | `.se-versions/` im Synchronisationsordner jeder Seite | Kopien mit relativem Originalpfad | wie K36 | `native/src/bisync/core/paths.rs:5`; `native/src/bisync/os/shared/version_listing.rs:129`; `native/src/bisync/os/shared/version_ops.rs:253` |
| K38 | Merge-Verzeichnis `<Baselinestamm>.merge-<token(rel)>.inputs/` mit `original-a`, `original-b`, `merged`, `inputs.json` | vollständige Fassungen beider Seiten und des Ergebnisses im Klartext | bis `remove` nach Abschluss; beim Neustart wiederverwendet | `native/src/bisync/os/shared/merge_inputs.rs:36–43, 62–85, 110–148, 175–189` |
| K39 | Konfliktkopie im Synchronisationsordner (beide Seiten) | vollständige verlorene Fassung mit Suffix ` (Konflikt …)` | wie normale Nutzerdateien | `native/src/bisync/os/shared/apply_transfer.rs:217, 240–242`; `native/src/bisync/core/types.rs:102–104` |
| K40 | Kopierstufen neben Nutzerdateien | vollständige neue Fassung unter einem Stufennamen (Präfix, Name, Suffix; höchstens 255 Bytes und 255 UTF-16-Einheiten) | bis Veröffentlichung oder `discard_copy_stage` | `native/src/vfs/core/staging_names.rs:1–12`; `native/src/vfs/os/shared/local_stage.rs:1–2`; `native/src/vfs/core/core.rs:232–235` |
| K41 | `<SYNC>/nested-mounts.json` | Inhalt nicht gelesen | nicht gelesen | `native/src/bisync/os/shared/snapshot_mounts.rs:17` |
| K42 | `<SYNC>/catchup.last.json` | Inhalt nicht gelesen | nicht gelesen | `native/src/daemon/os/shared/live.rs:79, 199` |
| K43 | `<SYNC>/legacy-baseline-jobs/` | Altbestand der Baseline-Migration; Inhalt nicht gelesen | nicht gelesen | `native/src/syncjobs/os/shared/baseline_migration.rs:19, 99` |

## 4. Wiederverwendbare Bausteine (Bestand; keine Empfehlung)

- Backend-Schnittstelle mit Standardrümpfen: 74 Methoden in `Backend` (`native/src/vfs/core/core.rs:17–494`) und 24 Methoden in `BackendExtensions` (`native/src/vfs/core/extensions.rs:29–245`). Ein Backend überschreibt nur, was es kann; die Fähigkeiten melden `supports_changes`, `supports_bulk_tree`, `supports_walk_tree`, `supports_search`, `supports_walk_hashed` (Trait `Backend`), `supports_duplicate_search`, `supports_hash_walk`, `supports_recycle`, `supports_remove_tree` (Trait `BackendExtensions`) sowie `is_local`, `provides_content_hash`, `staged_write_capabilities` und `root_confinement` (Abschnitt 2.1.1).
- Umhüllungsmuster mit Weiterleitung: `CachingBackend` (`native/src/vfs/core/cache.rs:65–93`) leitet alle Methoden an `inner: BackendHandle` weiter, ruft nach Mutationen `invalidate` auf (`cache_paths.rs:68–101`) und liefert Erweiterungen über `cache_extensions.rs:16–154`.
- Schreibgeschützte Schicht: `ZipBackend` (`native/src/zipfs/os/shared/zipfs.rs:53–61, 124–158, 217–233`) mit Verzeichnisbaum im Speicher.
- Locator-Auflösung: `EndpointSpec::parse` (`native/src/connect/core/location.rs:15–59`), `saved_location` (89–134), `resolve_endpoint` (`native/src/connect/os/shared/resolution.rs:7–44`), `connector::open_saved_at` (`native/src/connect/os/shared/connector.rs:369–374`).
- Metadaten und Geheimnis gemeinsam sichern: `save_connection_with_secret` und `remove_connection` mit Rückbau über `transaction::commit_secret_and_metadata` (`native/src/creds/os/shared.rs:266–336`; Transaktionsmodul `native/src/creds/core/transaction.rs`, nicht gelesen).
- Verschlüsselter Geheimnisspeicher: `sealed_record.rs` (ChaCha20-Poly1305, Konto-Digest als zugehörige Daten, Migrationspfad aus Format 1; `native/src/creds/core/sealed_record.rs:1–30, 111–120`).
- Konfliktmodell: `ConflictMode` (`native/src/bisync/core/types.rs:90–105`), `winner_under` (`native/src/bisync/core/plan_decide.rs:198–215`), `Conflict` (`native/src/bisync/core/types.rs:468–473`), `DuplicateConflict` und `FileVariant` (`native/src/bisync/core/duplicate_types.rs:5–47`).
- Auflösung mit Sicherung des Verlierers und exakten Signaturen: `resolve_checked` (`native/src/bisync/os/shared/resolve.rs:72–93`), `resolve_variant_checked` (257–), Paarsperre (`native/src/bisync/os/shared/pair_lock.rs:1–3`).
- Vergleich mit Einzelaktion: `Preview` (`native/src/bisync/os/shared/preview.rs:17–31`) mit schreibgeschütztem Planlauf (`preview`, `preview_with`) und `apply_preview_action` (237–242), das genau eine angezeigte Aktion unter der Paarsperre anwendet.
- Versions-API: `VersionStore`, `VersionReason`, `VersionEntry`, `list_versions`, `restore_version`, `remove_versions` (`native/src/bisync/os/shared/versions.rs:132–229`).
- Zusammenführung mit dauerhaften Eingaben: `merge_inputs.rs` (Wiederaufnahme nach Neustart, `save`, `load`, `remove`) und `merge_recorded.rs` (`MergeChoice`, `merge_recorded_for_key`).
- Dauerhafte Journale mit Prüfsumme: `checkpoint_journal.rs` (Länge, SHA-256, abgeschnittener Endrahmen wird ignoriert, Doku 1–3) und `replacement_journal.rs` (Absicht vor dem Veröffentlichen).
- Mount-Spool und Journal (nur Windows): `spool.rs` (Zufallsnamen, verwaiste Dateien entfernen), `journal.rs` (JSON-Zeilen mit `sync_data`, Kompaktierung), `clean_cache.rs` (Ablauf 300 s, Budget).
- Anonyme Spools: `analysis_spool.rs` (`tempfile::tempfile_in`), `copy_transfer.rs` (`tempfile::tempfile()`).
- Namensbegrenzung für Stufen: `staging_names.rs` (`fit_stage_name`, `is_staging_name`).

## 5. Betroffene Verträge

Festgestellt: Die folgenden Verträge sind im Bestand belegt. Ob und wie Tresore sie berühren, ist keine Aussage dieser Lesung.

- Persistenz (Dateien): `connections.txt` (TSV, Feldreihenfolge in `native/src/creds/core/types.rs:120–140`); `secrets-v1`-Datensätze (Format `SESEC02`, Format `SESEC01` nur lesend für die Migration; `sealed_record.rs:1–7, 19–22`); `<SYNC>/jobs/<id>.conf` (unbekannte Schlüssel werden laut Doku ignoriert, `native/src/syncjobs/mod.rs:8–11`); `sync_state.sqlite` (Schema-Tabelle `schema_version`, `state_store.rs:37–39`); Baseline-Dateien (`.sebl`, `replica_state.rs:18`); Mount-Journal (`JOURNAL_VERSION = 1`, `journal.rs:8`); Versionsordner (`.se-versions`, `bisync/core/paths.rs:5`); Merge-Verzeichnisse (`merge_inputs.rs:36–43`).
- Locator: `EndpointSpec` mit den Schemata `gdrive`, `share`, `sftp`, `ftp`, `ftps`, `webdav`, `smb` (`native/src/connect/core/location.rs:45–57`); gespeicherte Adressen im Format `proto://user@host:port/wurzel` (`native/src/creds/core/types.rs:79–88`); Doku `location.rs:1–2`: „Application locators keep their historical literal backend paths.“ Persistierte Adressen sind damit ein Vertrag; die Doku nennt als Beispiel, dass `%20` beim Wiederöffnen eines Syncs nicht in Leerzeichen dekodiert wird (`location.rs:1–2`).
- Scheme: `Scheme` mit sieben Varianten (`native/src/vfs/core/scheme.rs:3–11`); die Anzeige in `native/src/app/core/error_report.rs:55–61` erfasst alle Varianten ohne `_`-Zweig.
- Sync-Konfigurationsschlüssel: `ConflictMode::parse` und `as_str` (`bisync/core/types.rs:107–131`) sind die gespeicherten Textwerte; die CLI übernimmt sie als Werte (`cli/sync_jobs/options.rs:15–17`).
- IPC (Daemon): keine Sync-Konflikt-, Versions- oder Merge-Methode gefunden (Abschnitt 2.5).
- Bridge (Android, JNI): `Java_app_smartexplorer_android_core_NativeBridge_call` mit Methodennamen-Strings `sync.*` (22 Einträge in `native/src/mobile/os/shared/domains/mod.rs:112–133`; Aufrufe in `SyncApi.kt:279–373`). Die Bridge reicht Methode und Argumente unverändert an `smart_explorer::mobile::call` weiter (`native/android-bridge/src/lib.rs:139–148`).
- CLI: Unterbefehle `list`, `show`, `add`, `edit`, `remove`, `enable`, `disable`, `log` (`native/src/cli/sync_jobs.rs:45–64`); Optionen `--conflict`, `--versioning-scheme`, `--retain-days`, `--retain-count`, `--versions-location` (`native/src/cli/sync_jobs/options.rs:50–82`); JSON-Schlüssel der Ausgabe (`native/src/cli/sync_jobs/output.rs:20–30, 70`).
- GUI Desktop: Job-Editor-Seiten (`native/src/app/core/job_editor_pages.rs:150–156, 242–258, 320–346, 412–414, 449`) und Konfliktdialog (`native/src/app/core/bisync_conflict_ui.rs`).
- GUI Android: `JobDraft.kt` mit Feld `conflict` (`JobDraft.kt:39, 102, 144, 241`) und die Sync-Bildschirme (Abschnitt 2.5).
- Locatoren auf dem Desktop-Editor: `SyncJob.source` und `SyncJob.target` als Zeichenketten (`native/src/syncjobs/core/types.rs:72–75`); `connect_match` (`types.rs:111`).

## 6. Lücken, Unsicherheiten und Annahmen

Lücken (genau benannt):

1. Cryptomator-Format 8 ist im Bestand nicht implementiert. Es gibt keinen Code für `masterkey.cryptomator`, `vault.cryptomator`, `.c9r`, Verzeichnis-IDs, AES-SIV oder die scrypt-Parameter. Die Lock-Datei enthält kein `aes-siv` und kein `aes-kw`.
2. Krypto-Primitive in der Lock-Datei (statische Auswertung der Einträge, nicht per Build geprüft): `aes-gcm` 0.10.3 und 0.11.1 (Abhängige: `noq-proto`, `smb2`, `ssh-cipher`), `aes` 0.8.4 und 0.9.1 (Abhängige: `aes-gcm`, `pkcs5`, `russh`), `scrypt` 0.12.0 (Abhängige: `pkcs5`, `russh`), `hkdf` 0.12.4 und 0.13.0 (`elliptic-curve`, `opaque-ke`, `smart_explorer`), `chacha20poly1305` 0.10.1 (direkt in `smart_explorer`), `argon2` 0.5.3 (direkt in `smart_explorer`, `ssh-key`), `zeroize` 1.9.0 (`Cargo.toml` verlangt `1.8`). Eine Freigabe, Lizenz- oder Sicherheitsaussage dazu ist nicht Teil dieser Lesung.
3. Linux-Laufwerk: Im Mount-Modul fehlt ein Linux-Adapter; `native/src/mount/os/` enthält nur `mod.rs` und `windows/`. Die Frage E10, wie Linux Laufwerke einhängt, ist im Bestand nicht beantwortet.
4. Der Schemawert `Scheme::Webdav` wird vom WebDAV-Backend gemeldet (`native/src/webdav/core/webdav.rs:211–213`); die Sicht auf WebDAV-Identität (Konto und Host) wurde nicht gelesen.
5. Externe Aussagen in den Cryptomator-Dokumenten (Abschnitt 2.6) sind nicht am Code prüfbar und wurden nicht extern nachgeprüft.
6. `target_managed` (Zustandsfeld): Bedeutung im Code nicht erklärt (`state_types.rs:29`).
7. Anlagewege für Sync-Jobs: die CLI-Anlage (`se sync add`), der Android-Editor und die Bridge-Eingabe wurden nur über Zeilen der Oberfläche und der Validierung belegt, nicht vollständig gelesen.
8. Vergleich: Die Aktion „Vergleichen“ in der Oberflächen-Doku (`docs/oberflaechen/einstellungen.md:153`) ist im Code nicht gelesen. Für Sync-Setups existieren `native/src/bisync/os/shared/preview.rs` (292 Zeilen; Vergleich, Abschnitt 2.5) und `native/src/app/core/sync_preview_types.rs`; deren Inhalt wurde nur an den Stellen gelesen, die Abschnitt 2.5 nennt. Ob der Vorschau-Vergleich als Vergleichsmodus für Tresore geeignet ist, ist nicht Teil dieser Lesung.
9. Merge: ob nur Textdateien zugelassen sind, wurde nicht geprüft (`merge_recorded.rs`, Rumpf nicht gelesen).
10. Android-Konfliktsitzung und Entwurf: `ConflictSession.kt` (241 Zeilen) und `JobDraft.kt` (283 Zeilen) wurden nur über Deklarationen und Zeilen gelesen.
11. Lebensdauer der Baselines und des SQLite-Index: Die Löschung beim Entfernen eines Jobs ist nicht gelesen (`syncjobs/os/shared/job_state_store.rs`, `persistence.rs` nur teilweise).
12. Inhalte nicht gelesen (Tabelle Abschnitt 3): `recent.txt` (K14), `share_server.txt` (K22), `lan_uplink` (K23), `crash.log` (K24), Update-Zustand (K25), `mobile/` (K26), `mounts/` (K27), `results.tsv` (K30), Checkpoint-Journal (K34, Speicherort Lücke), `nested-mounts.json` (K41), `catchup.last.json` (K42), `legacy-baseline-jobs/` (K43), `private-runtime/` (K04), Cloud-Konfiguration (K19), Wiederherstellungsmanifest (K07), Felder von `FsMeta` (K09), Ort des Host-Papierkorb-Inhalts (K10), Rotationsgröße `JOB_LOG_ROTATE_BYTES` (K35), Rotation der Transfer-Logs (K20).
13. Linux-Ergebnis von `platform_data_home()` (`native/src/support_dirs.rs:171–`) nicht vollständig gelesen; damit ist der genaue Linux-Pfad von `<AD>` nicht vollständig belegt.
14. Die Schlüssel-Schutzdatei für den Linux-Datenschlüssel (`native/src/creds/os/key_protector.rs`, Datei `APP_KEY_FILE`) wurde nicht gelesen.
15. Windows-Credential-Manager: Längen- und Zeichenbeschränkungen für das Konto-Feld sind nicht belegt (keine Dokumentationsrecherche).
16. Unbelegt bleibt, ob `tempfile::tempfile()` im Systemtemp-Verzeichnis anlegt (Crate-Verhalten, nicht im Repository belegt).

Annahmen (ausdrücklich, nicht aus dem Code bewiesen):

- A1: `<AD>` und `<SYNC>` sind die Ablagepfade laut `support_dirs.rs:196–210`; ihr Linux-Wert ist nicht vollständig belegt (Lücke 13).
- A2: Die Lebensdauer von Sitzungsordnern in `open-temp` endet mit der Sitzungsbereinigung beim Start (`app/os/shared/remote_helpers/temp.rs:39`); ob sie Reste nach Absturz vollständig entfernt, wurde nicht geprüft.
- A3: Eine Schicht, die `scheme()` und `root_display()` übernimmt und `state_identity()` nicht überschreibt, erhält die Identität des inneren Backends (abgeleitet aus `core.rs:26–28` und `cache.rs:101–103`).
- A4: Befehlszeilen in `run_before`, `run_after` und `run_cleanup` können Geheimnisse enthalten (nicht geprüft; die Felder sind Klartext in `<SYNC>/jobs/<id>.conf`).

## 7. Grenzen der Lektüre und Lesemethode

- Lesestand: Commit `da2646d2`. Zum Lesezeitpunkt zeigte `git status --short -- native android` nur den unversionierten Ordner `native/__pycache__/`, der schon im Start-Status stand. Quelltext wurde nicht geändert.
- Methode: Suche mit `grep -rn` (Zeilennummern); Abschnittslesung mit `awk`, `sed` und `cut` auf Zeilenbereichen; Zählung mit `grep -c` (Trait-Methoden 74, Erweiterungs-Methoden 24, Pflichtmethoden 10, `SyncJob`-Felder 51); statische Auswertung von `native/Cargo.lock` mit `awk`; Vollständigkeitsprüfung der Methoden- und Feldlisten per Skript gegen diese Datei.
- Nicht ausgeführt: keine Builds, kein `cargo`, keine Tests, keine Paketinstallation, keine Server, keine Commits oder Pushes, keine Netzrecherche, keine Subagenten. `graphify` wurde nicht verwendet; alle Befunde stammen aus dem Quelltext und den genannten Dokumenten.

Gelesen, vollständig:

- Dokumente: `docs/plaene/2026-10-17-rechte-desktop-tresore/strang-tresore.md`; `docs/refs/cryptomator-vault-api-2026-10-04.md`.
- Rust: `native/src/vfs/mod.rs`; `native/src/vfs/core/core.rs`; `native/src/vfs/core/scheme.rs`; `native/src/vfs/core/volume.rs`; `native/src/vfs/core/trait_defaults.rs`; `native/src/vfs/core/capabilities.rs`; `native/src/vfs/core/meta.rs`; `native/src/vfs/core/extensions.rs`; `native/src/vfs/core/dispatch.rs`; `native/src/vfs/core/cache_limits.rs`; `native/src/vfs/core/cache_paths.rs`; `native/src/vfs/os/shared/sync_roots.rs`; `native/src/zipfs/mod.rs`; `native/src/connect/os/shared/resolution.rs`; `native/src/creds/mod.rs`; `native/src/bisync/core/duplicate_types.rs`.

Gelesen, Auszüge (Zeilenbereiche; Zeilen, die in Abschnitt 2 oder 3 zitiert sind):

- Dokumente: `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md` (Zeilen 1–61); `docs/CRYPTOMATOR_VAULTS_ERKUNDUNG.md` (Zeilen 198–273); `docs/oberflaechen/einstellungen.md` (Gliederung, Zeilen 36–72, 138–155).
- Rust vfs und zipfs: `native/src/vfs/core/cache.rs` (Zeilen 1–125, 145–170, 200–210); `native/src/vfs/core/cache_extensions.rs` (Zeilen 16–156, gefiltert); `native/src/zipfs/os/shared/zipfs.rs` (Zeilen 1–14, 50–62, 123–160, 217–233); `native/src/webdav/core/webdav.rs` (Zeilen 211–213); `native/src/sftp/core/backend.rs` (Zeilen 199–201); `native/src/ftp/core/ftp.rs` (Zeilen 205–207); `native/src/smb/core/backend.rs` (Zeilen 218–220); `native/src/gdrive/core/backend.rs` (Zeilen 10–12).
- Rust connect und creds: `native/src/connect/core/location.rs` (Zeilen 1–61, 86–135); `native/src/connect/os/shared/connector.rs` (Zeilen 96–118, 356–400); `native/src/creds/core/types.rs` (Zeilen 1–120); `native/src/creds/core/sealed_record.rs` (Zeilen 1–30, 105–125); `native/src/creds/os/shared.rs` (Zeilen 1–40, 266–336); `native/src/creds/os/linux_os.rs` (Zeilen 1–60); `native/src/creds/os/linux_file_store.rs` (Zeilen 225–235); `native/src/creds/os/windows.rs` (Zeilen 40–52, 82–91).
- Rust mount, daemon, transfer, share, host_trash: `native/src/mount/core/spool.rs` (Zeilen 1–180, 262–270); `native/src/mount/core/journal.rs` (Zeilen 1–60); `native/src/mount/core/clean_cache.rs` (Zeilen 1–24); `native/src/mount/core/materialization.rs` (Zeilen 145–162); `native/src/mount/core/file_io.rs` (Zeilen 127–150); `native/src/mount/core/types.rs` (Zeilen 135–160); `native/src/mount/mod.rs` (Zeilen 279–281); `native/src/daemon/os/shared/mount_manager_start_cache.rs` (Zeilen 1–30, 70–90); `native/src/daemon/os/shared/job.rs` (Zeilen 252–264); `native/src/daemon/os/shared/live.rs` (Kopf sowie Zeilen 42–56, 79, 199 per Suche); `native/src/daemon/os/shared/job_supervisor.rs` (Zeilen 1–12); `native/src/transfer/os/shared/temp.rs` (Zeilen 1–110); `native/src/share/core/fs_response.rs` (Zeilen 29–52); `native/src/share/os/shared/analysis_spool.rs` (Zeilen 1–52 und Auszüge bis 92); `native/src/host_trash/os/shared/store.rs` (Zeilen 1–40); `native/src/host_trash/core/record.rs` (Zeilen 1, 17–34).
- Rust Ablage und Index: `native/src/support_dirs.rs` (Zeilen 160–175, 196–225); `native/src/app/os/shared/platform_helpers.rs` (Zeilen 108–130); `native/src/app/core/reclaim_core.rs` (Zeilen 362–380); `native/src/folder_index/os/persistence.rs` (Zeilen 1–16, 17–56 per Suche); `native/src/mobile/os/shared/index.rs` (Kopf per Suche, Zeilen 29–39); `native/src/cloud/os/shared.rs` (Zeilen 1–12, 18, 61 per Suche); `native/src/cli/shell_session/registry.rs` (Kopf 1–8, Zeile 73 per Suche).
- Rust Sync: `native/src/syncjobs/mod.rs` (Zeilen 1–11); `native/src/syncjobs/core/types.rs` (Zeilen 13–22, 66–90, 160–200); `native/src/syncjobs/core/validation.rs` (Zeilen 18–64); `native/src/syncjobs/os/shared/persistence.rs` (Zeilen 13–30); `native/src/bisync/core/types.rs` (Zeilen 8–18, 56–150, 372–390, 468–483 per Suche); `native/src/bisync/core/plan_decide.rs` (Zeilen 150–215); `native/src/bisync/os/shared/resolve.rs` (Zeilen 1–27, 72–100, 240–276); `native/src/bisync/os/shared/merge_inputs.rs` (Zeilen 8–14, 36–43, 110–112, 170–189 per Suche); `native/src/bisync/os/shared/merge_recorded.rs` (Zeilen 1, 15–24); `native/src/bisync/os/shared/versions.rs` (Zeilen 132–175, 196–230 per Suche); `native/src/bisync/os/shared/persistence.rs` (Zeilen 14–16, 59–63); `native/src/bisync/os/shared/replica_state.rs` (Zeilen 17–27, 140–144); `native/src/bisync/os/shared/state_types.rs` (Zeilen 20–30); `native/src/bisync/os/shared/preview.rs` (Zeilen 12–40, 233–242); `native/src/bisync/os/shared/incremental.rs` (Zeilen 444–455); `native/src/bisync/os/shared/checkpoint_journal.rs` (Kopf 1–3 und Zeilen 163–165 per Suche).
- Rust Konsum (Oberflächen, Bridge, CLI, Fremdsystem): `native/src/app/core/error_report.rs` (Zeilen 55–62); `native/src/app/core/sync_preview_types.rs` (Zeile 1); `native/src/mobile/os/shared/domains/mod.rs` (Zeilen 110–136); `native/android-bridge/src/lib.rs` (Zeilen 115–156 per Suche, 139–152); `native/src/cli/sync_jobs.rs` (Zeilen 44–64, 159, 242 per Suche); `native/src/cli/sync_jobs/options.rs` (Zeilen 15–21, 50–82 per Suche); `native/src/cli/sync_jobs/output.rs` (Zeilen 11–30, 70 per Suche).
- Android: `android/app/src/main/java/app/smartexplorer/android/api/SyncApi.kt` (Zeilen 279–373, 449 per Suche); `android/app/src/main/java/app/smartexplorer/android/ui/sync/ConflictVariantChoice.kt` (Zeilen 19–30).
- Abhängigkeiten: `native/Cargo.lock` (statische Auswertung mit `awk`); `native/Cargo.toml` nur per Suche (Krypto-Zeilen).

Nur per Suche (Trefferzeilen mit Zeilennummern, Zusammenhang nicht gelesen):

- Dokumente: `docs/SYNC_FEATURES.md` (Gruppe E).
- Rust app: `native/src/app/core/bisync_conflict_ui.rs`, `native/src/app/core/bisync_conflicts.rs`, `native/src/app/core/bisync_merge.rs`, `native/src/app/core/job_editor_pages.rs`, `native/src/app/core/job_editor.rs`, `native/src/app/core/menus_sync_jobs.rs`, `native/src/app/core/merge_ui.rs`, `native/src/app/core/preview_core.rs`, `native/src/app/core/state.rs`, `native/src/app/core/support_paths.rs`, `native/src/app/core/sync_versions_ui.rs`, `native/src/app/mod.rs`, `native/src/app/os/shared/remote_helpers/recovery_manifest.rs`, `native/src/app/os/shared/remote_helpers/temp.rs`, `native/src/app/os/shared/remote_open.rs`, `native/src/app/os/shared/remote_zip.rs`.
- Rust bisync: `native/src/bisync/core/paths.rs`, `native/src/bisync/core/plan_pair.rs` (Zeile 211), `native/src/bisync/os/shared/apply_transfer.rs`, `native/src/bisync/os/shared/checkpoint_run.rs`, `native/src/bisync/os/shared/duplicate_backup.rs`, `native/src/bisync/os/shared/engine_change_feed.rs`, `native/src/bisync/os/shared/incremental_index_commit.rs`, `native/src/bisync/os/shared/pair_lock.rs` (Kopf und Zeilen 125–127), `native/src/bisync/os/shared/replacement_journal.rs` (Zeilen 1, 76–84), `native/src/bisync/os/shared/run_log.rs` (Kopf 1–8), `native/src/bisync/os/shared/run_log_lines.rs`, `native/src/bisync/os/shared/snapshot_mounts.rs`, `native/src/bisync/os/shared/state_store.rs` (Zeilen 37–67, 360), `native/src/bisync/os/shared/version_listing.rs`, `native/src/bisync/os/shared/version_ops.rs`, `native/src/bisync/os/shared/version_provider_policy.rs`.
- Rust cli, connect, daemon, folder_index, host_trash, lib, mobile, mount, net, syncjobs, transfer, updater, vfs: `native/src/cli/share.rs`, `native/src/cli/mcp/catalog.rs`, `native/src/cli/mcp/status_tools.rs`, `native/src/cli/mcp/wait.rs`, `native/src/connect/os/shared/location_prefs.rs` (Kopf und Zeilen 1–3, 8, 29), `native/src/daemon/os/shared/connect_triggers.rs` (Kopf 1–2), `native/src/daemon/os/shared/mount_registry.rs` (Zeile 137), `native/src/folder_index/core/model.rs` (Zeilen 4–7), `native/src/host_trash/mod.rs` (Zeile 1), `native/src/lib.rs` (Zeile 160), `native/src/mobile/core/config.rs` (Zeile 76), `native/src/mobile/os/shared/dispatch.rs` (Zeile 33), `native/src/mobile/os/shared/domains/sync_conflicts.rs` (Funktionsdeklarationen und einzelne Zeilen per Suche), `native/src/mobile/os/shared/domains/sync_versions.rs` (Zeilen 21, 68), `native/src/mobile/os/shared/domains/sync_merge.rs` (Zeile 21), `native/src/mobile/os/shared/domains/job_json.rs`, `native/src/mobile/os/shared/domains/sync_jobs.rs`, `native/src/mobile/os/shared/import.rs` (Zeilen 255–257), `native/src/mobile/os/shared/pool.rs` (Zeilen 17, 163), `native/src/mobile/os/shared/sys.rs` (Zeile 95), `native/src/mount/core/cache_policy.rs` (Zeilen 6–7, 19, 33–36, 46), `native/src/mount/os/windows/private_payload.rs` (Zeilen 28–50), `native/src/mount/os/windows/cache_lease.rs` (Zeilen 33–38, 60–71), `native/src/net/os/windows/uplink_helper.rs` (Zeile 45), `native/src/net/os/windows/uplink_install.rs` (Zeile 88), `native/src/syncjobs/os/shared/baseline_migration.rs` (Zeilen 19, 99), `native/src/syncjobs/os/shared/editor.rs` (Zeilen 16, 111, 251), `native/src/syncjobs/os/shared/job_state_classify.rs` (Zeilen 71, 79), `native/src/syncjobs/os/shared/job_state_store.rs` (Zeile 156), `native/src/syncjobs/os/shared/migration.rs` (Zeilen 270–289), `native/src/syncjobs/os/shared/results.rs` (Kopf, Zeile 33), `native/src/syncjobs/os/windows/job_state_lock.rs` (Treffer „SIV“ als Teil von `LOCKFILE_EXCLUSIVE_LOCK`), `native/src/transfer/os/shared/engine/issues.rs` (Kopf, Zeilen 41, 126), `native/src/updater/os/shared/config.rs` (Zeilen 8, 20, 57), `native/src/vfs/core/cache_writer.rs` (Zeile 38), `native/src/vfs/core/extension_types.rs` (Deklarationen), `native/src/vfs/core/staging_names.rs` (Kopf, Zeilen 6–12, 28), `native/src/vfs/os/shared/copy_transfer.rs` (Zeilen 1, 19–37), `native/src/vfs/os/shared/local.rs` (Zeile 89), `native/src/vfs/os/shared/local_stage.rs` (Kopf, Zeilen 10–14, 28–30), `native/src/daemon/os/shared/ipc*.rs` (Suche nach Begriffen).
- Android: `android/app/src/main/java/app/smartexplorer/android/ui/sync/ConflictsScreen.kt` (Deklarationen und einzelne Zeilen per Suche), `ConflictSession.kt` (Deklaration), `MergeScreen.kt` (Deklarationen), `VersionsScreen.kt` (Deklaration, Aufrufe), `JobDraft.kt` (Deklaration, Zeilen 39, 102, 144, 241).

Muster (nicht als Dateiliste gelesen; nur Suche nach Begriffen in diesen Verzeichnissen): `native/src/app/core/*`, `native/src/bisync/os/shared/apply_*.rs`, `native/src/cli/mcp/*`, `native/src/daemon/os/shared/ipc*.rs`, `native/src/vfs/core/cache*.rs`, `native/src/mount/os/` (nur Verzeichnisinhalt per `ls`), `native/src/share/`, `native/src/gdrive/`, `native/src/webdav/` (nur die genannten Zeilen).

Nicht gelesen, aber im Bericht benannt: `native/src/vfs/core/extension_calls.rs`, `native/src/zipfs/os/shared/entry.rs`, `native/src/creds/core/transaction.rs` (nur der Aufruf in `shared.rs` gelesen), `native/src/creds/os/key_protector.rs`, `native/src/creds/os/linux_file_store*.rs` (außer den genannten Zeilen), `native/src/connect/os/shared/connector.rs` (außer den genannten Zeilen), `docs/refs/cryptomator-format-sync-2026-10-17.md` und weitere unversionierte Dateien in `docs/refs/`, `docs/refs/cryptomator-mounting-2026-10-04.md` (nicht Teil des Auftrags).

Testdateien (`*tests*.rs`, `*_task_tests.rs`): nicht gelesen. Sie wurden nur als Treffer der Suche erwähnt, wo sie in einer Fundstelle vorkamen.

- Keine Daten der Geräte des Nutzers: Die Lesung hat keine Gerätenamen, Kennungen, Freigaben, PINs oder lokale Installationspfade übernommen. Der Test-Dienstname für Windows wurde nur als Konstante `smart_explorer` benannt.
