# CI-4-V-LOCAL: privater Windows-Versionsbackup-Flush

Stand: 2026-10-04. Begrenzter Abschluss des V-LOCAL-Abschnitts aus
`ci-fourth-fixes.md`, gemäß dem frisch gelesenen `scopes/ci-4-v-local.json`.
Ausgangspunkt: vollständig beendeter Run `37162485159` auf
`ac3b0c9098963fae386e94558f2f3ca1bb240740`; Formatterpatch `74a2b67d` wurde
vom Root bereits übernommen. Dieser Handoff behauptet keine Laufzeitabnahme.

## Zugeordnete Evidenz und Ergebnis

`/tmp/rv1-ci-fourth/v-local.json` enthält genau die beiden zugewiesenen
Windows-AccessDenied-Fehler an den `version_save::save(...).unwrap()`-Aufrufen
in `engine_identity_task_tests.rs:127` und `:339`; der Linux-Block ist leer.
Die aktuelle Sourcekette öffnete das veröffentlichte private `data` über
`support_dirs::open_private_file` mit `writable = false`, härtete den Handle
und rief anschließend `sync_all()` auf. Der Windows-Readhandle besitzt
bewusst kein `GENERIC_WRITE`. Laut frisch geprüftem Microsoft-Vertrag
benötigt `FlushFileBuffers` dieses Recht; Rust 1.99.0 verwendet den Aufruf
in seiner Windows-`fsync`-Implementierung. Das ist die konkrete begrenzte
Zugriffskorrektur dieses Blocks; andere Save-/Journal-/Namespacepfade wurden
nicht pauschal geändert.

Die API-Evidenz, Syntax, Fehlerpropagation und gewählte Öffnungsreihenfolge
wurden vor dem Sourceedit in
[private-file-access.md](../../../../refs/private-file-access.md#windows-versionsbackup-flush-am-privaten-datenhandle)
gesichert; die anschließend erlaubte Unix-Bestätigung wurde dort ergänzt.

## Änderung und Entscheidungen

1. `version_save::copy_private` behält den bisherigen privaten Readopen
   und `secure_private_file` bei. Damit wird auch ein über den Unix-Quellmode
   entstandenes read-only Backup weiterhin zuerst am geöffneten Objekt
   auf Owner/Typ/einen Hardlink geprüft und auf 0600 gehärtet.
2. Anschließend wird dieser Readpin ausdrücklich geschlossen. Das erhält
   die gewöhnliche Windows-Readpin-Grenze ohne `FILE_SHARE_WRITE` und
   vermeidet eine Sharekollision beim Öffnen des Flushhandles.
3. Ausschließlich das private, bereits veröffentlichte Backup-`data` wird
   über die vorhandene `creds::private_storage::open_file(path, true)`-API
   wieder geöffnet und mit `sync_all()?` geflusht. Diese API prüft den
   privaten Windows-Parent und den geöffneten Leaf einschließlich
   Owner/DACL, NoFollow und Hardlinkgrenze. Der Consumer schreibt und kürzt
   keine Backupbytes.

Der gezielt erlaubte Unix-Read bestätigt `open_file` →
`open(path, writable, false)` und die handlegebundene Owner-/Mode-/Hardlink-
Prüfung in `secure`. Es wurde kein Unix-Helper oder Plattformmodul geändert.
Die vorhandene RW-API genügt; keine neue Definition oder Rechteausweitung
an globalen Read-, Quell-, Root- oder Watchhandles ist erforderlich.

## Erhaltene Verträge

- Privater Root und Versionsunterordner werden weiterhin vor der
  Stageerstellung gehärtet. Die bestehenden DACL-/Owner-/NoFollow-/
  Hardlinkprüfungen bleiben bestehen; es gibt keinen Schutzfallback.
- Intent wird weiterhin vor der Stage geschrieben; Stageerstellung und
  Veröffentlichung in den fehlenden privaten `data`-Namen bleiben exklusiv.
  Quell-Capture, ExpectedFile, Cancellation, MD5/Bytezahl, Zielsignatur
  und abschließende Quellrevalidierung bleiben unverändert.
- Öffnungs-, Privacy- und Flushfehler propagieren. `entry.json` wird erst
  nach erfolgreichem Flush geschrieben. Ein fehlgeschlagenes Backup
  erlaubt weiterhin keine nachfolgende destruktive Anwendung.
- AppData-/Auto-/Archive-Bedeutung, gespeicherte Verbindungs-/Account-
  Identität, Side-Token, Pairlock, private Versionsdaten und Restore
  bleiben unverändert. Source-Read-only und Source-Metadaten erhalten
  keine neuen Schreibrechte.
- Archive-Rollback, NoReplace, Konflikt-/Retained-/Recoverynamen sowie
  die bestehenden Namespace-/PerFileOnly-Aussagen bleiben unverändert.
  Aus dem Fileflush folgt keine zusätzliche Directory-Durability.
- Sämtliche bestehenden Fixture-Assertions sind unverändert.

## Statischer Self-Review

Der vollständige Sourcevergleich bestätigt genau die Ergänzung im
bestehenden privaten Backup-Consumer: Kommentar, explizites Readpin-Drop
und geprüfter RW-Reopen unmittelbar vor dem bisherigen `sync_all()?`.
Alle anderen Sourcebytes der Datei sind gegenüber dem frisch gelesenen
Ausgangspunkt identisch. Die lokale Ref behält ihren bisherigen Inhalt
und ergänzt nur den datierten CI-4-Abschnitt.

Der bereits vorhandene Tree-sitter-Rust-Parser meldet für die eigene
geänderte Rustdatei keine ERROR-/Missing-Nodes. Die Datei hat 345 Zeilen
und 12 356 Bytes; die längste Zeile ist 100 Zeichen. Es wurden keine
Compiler, Formatter, Tests, Server, Installationen, Git-/CI-/Graph- oder
Releaseaktionen und keine Unteragenten gestartet.

## AcceptanceSelector für dieselbe Root-Suite

- `bisync::engine_provider_task_tests::identity_tests::engine_provider_account_identity_preserves_state_locks_inputs_and_versions`:
  privater Save erreicht das fertige, unveränderliche Versionsmanifest;
  Account-/Baseline-/Lock-/Inputs-Migration und Restore behalten sämtliche
  bereits vorhandenen Assertions.
- `bisync::engine_provider_task_tests::identity_tests::engine_provider_publication_and_lost_ack_use_exactly_one_contract`:
  das private Backup der Originalbytes gelingt vor dem Apply;
  genau ein ausgewählter Publicationvertrag, Lost-ACK-Recovery,
  ForeignCreator-Schutz und erhaltene Versionen behalten sämtliche
  bereits vorhandenen Assertions.

Diese Erwartungen werden ausschließlich über die bestehende vollständige
Root-RV1-Remote-Suite bestätigt.
Es erfolgte keine Laufzeitprüfung.

## Exaktes Dateiinventar

Für diesen CI-4-Block gelesen:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-4-v-local.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-fourth-fixes.md`
- `/tmp/rv1-ci-fourth/v-local.json`
- `native/src/bisync/os/shared/version_save.rs`
- `native/src/bisync/os/shared/version_manifest.rs`
- `native/src/bisync/os/shared/versions.rs`
- `native/src/bisync/os/shared/engine_identity_task_tests.rs`
- `native/src/support_dirs.rs`
- `native/src/creds/os/private_storage_windows.rs`
- `native/src/local_access/os/windows/private_access.rs`
- `native/src/local_access/os/windows/private_security.rs`
- `native/src/vfs/os/windows/local_writes.rs`
- `docs/refs/private-file-access.md`
- `docs/refs/local-fs-identity-durability.md`
- `native/src/bisync/os/shared/apply_stage.rs`
- `native/src/local_access/os/windows/private_ancestors.rs`
- `native/src/creds/os/private_storage_unix.rs` — ausschließlich `open_file` und `secure`, wie freigegeben.
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-4-V-LOCAL.md` — eigener Bericht nach Erstellung zum Self-Review.

Geändert:

- `native/src/bisync/os/shared/version_save.rs` — genau der private Flush-Consumer.
- `docs/refs/private-file-access.md` — datierte API-/Syntax-/Kompatibilitätsreferenz.

Erstellt:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-4-V-LOCAL.md`

Gelöscht: keine. Modulregistrierungen, Signaturen und öffentliche APIs:
keine Änderung. Andere freigegebene Modifydateien wurden nicht verändert.

Frisch gelesene Primärquellen:

- [Microsoft FlushFileBuffers](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)
- [Rust File](https://doc.rust-lang.org/std/fs/struct.File.html)
- [Rust 1.99.0 Windows source](https://github.com/rust-lang/rust/blob/1.99.0/library/std/src/sys/fs/windows.rs)

## Offene Grenzen

Keine neue Implementierungs- oder Scopeabhängigkeit dieses Consumer-Fixes.
Die konkrete Unix-Readfreigabe ist eingearbeitet. Laufzeitbestätigung bleibt
bei Root und derselben Remote-Suite. Apply-/Namespaceänderungen liegen bei
E-ENGINE, Held-Rename bei H-ANALYSIS; dafür wird hier kein Ergebnis behauptet.
Keine Journaldatei und keine Fixture wurde geändert. Der Auftrag endet mit
diesem Handoff.
