# Direkt-Freigaben schreibbar, Laufwerks-Speichern ohne fremde Endungen (2026-10-09)

Stand: 2026-10-09. Quellen: Nutzernachrichten 2–5 vom 2026-10-09 (in derselben Sitzung wie
[Medien weiterschalten](../2026-10-09-medien-weiterschalten/plan.md)).

## A — Auftrag

| Nr. | Ergebnis | Quelle |
|---|---|---|
| R3 → R5 | Meldung „Diese Peer-Wurzel ist nicht sicher schreibbar: neue Dateien, vorhandene Dateien, atomare Editor-Ersetzung“ beim Laufwerk auf das Handy. Ursache laut Nutzer: „auf dem handy war nur lesen aktiv“. Gefordert: Direkt-Shares mit Schreibfreigabe als Standard („dat is der ganze sinn davon“). | Nachrichten 2, 4 |
| R4 | Weitere Regressionspunkte derselben Sicherheitshärtung ebenfalls beheben. | Nachricht 3 |
| R6 | Über das Laufwerk gespeicherte Dateien (Beispiel „Drucken als PDF“) bekommen eine Endung wie „.se-mount-…“ angehängt; das darf nicht passieren, „der explorer hat nichts zu ändern was nicht erwartet wird“. | Nachricht 5 |

## B — Bestand

1. Die Meldung kommt aus dem Laufwerks-Dialog (`app/core/mount_ui_draft.rs::render_write_status`),
   wenn die Fähigkeitsprüfung des Hosts `staged_write` leer liefert. Der Host setzt sie leer, sobald
   die Freigabe nur lesbar ist oder das Gerät kein Schreibrecht hat
   (`share/core/server_capabilities.rs`, `fs_guard_backend.rs::mount_path_capabilities`).
2. RV1/FC1 (Spezifikation `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/spec.md`,
   Commit `0d36bddd` vom 2026-10-03, Vertrag V5) führte ein: neue Freigaben „Nur lesen“
   (`SharedRoot::new`), „Darf schreiben“ je Gerät mit Standard aus (`DirectGrant.write: false`
   an sechs Erzeugungsstellen), neue Räume ohne Schreibrecht, die automatische Home-Freigabe
   einmalig auf „Nur lesen“ (`profile_migration.rs`). Bestehende Grants vor V5 behielten Schreiben.
3. Lokaler Host „ubuntu“ (`/root/.local/share/smart_explorer/share_profiles.json`, gelesen
   2026-10-09): Freigabe „Root“ = `/` mit `read_write`, Grants „Pixel 8 Pro“ und „Silas_Asus“ mit
   `write: true` – also nicht die Ursache; das Handy hatte nach Nutzerangabe nur Lesen.
4. Laufwerks-Speichern (`mount/core/file_commit.rs`): Upload in `<name>.se-mount-<16 hex>`
   (`vfs::unique_staging_path`), dann Umbenennen auf den Zielnamen. Bei Upload-Fehler, nicht
   bestätigter Prüfung, gescheitertem Umbenennen oder Konflikt nach dem Upload blieb der
   Zwischenname absichtlich liegen; jeder Wiederholungsversuch legte einen weiteren an. Ein
   „Ohne-Ersetzen“-Umbenennen, das an einem inzwischen entstandenen Namen scheitert
   (`AlreadyExists`), lief in den Zweig „mehrdeutig“ und ließ den Zwischennamen ebenfalls liegen.
5. Konfliktprüfung: `Baseline`-Gleichheit exakt über Kennung, Größe, Änderungszeit und
   Prüfsumme; `CachingBackend::stat` (`vfs/core/cache.rs:122`) kann einen Listen-Stand liefern,
   dem Kennung/Prüfsumme fehlen oder dessen Zeit auf Sekunden gekürzt ist → falscher Konflikt.
   Welcher der Fälle bei der Nutzermeldung eintrat, ist ohne das Gerät nicht belegt.

## C — Entscheidungen

- **R5:** `DirectGrant.write` neuer/neu zugelassener Geräte = an (`NEW_DIRECT_GRANT_WRITE`);
  neue Freigaben im Bereich „direct“ = „Lesen und Schreiben“ (`SharedRoot::new_in_scope`),
  Raum-Freigaben bleiben „Nur lesen“; ausdrücklich gesetztes „Nur lesen“ bleibt.
  Systemorte (Autostart, Schlüssel) bleiben in jeder Freigabe geschützt. Gespeicherte
  Verbindungen bleiben einzeln und standardmäßig nur lesend (Zugangsdaten Dritter).
- **R4:** Die einmalige FC1-Umstellung der automatischen Home-Freigabe wird für Direkt
  rückgängig gemacht (nur, wo der Migrationshinweis noch steht; danach gilt jede ausdrückliche
  Wahl) und für alte Profile nicht mehr angewendet. Räume behalten FC1 (dort hatte der Nutzer
  am 2026-10-02 ausdrücklich „sicher per Standard“ verlangt; Nachricht 4 nennt nur Direkt-Shares).
  Bereits mit „aus“ angelegte Gerätrechte lassen sich nicht von ausdrücklichen Wahlen
  unterscheiden und bleiben – sie sind je Gerät umschaltbar.
- **R6:** Kein Speichervorgang hinterlässt den Zwischennamen: ohne Wirkung → eigene Stage
  löschen (nur wenn noch reguläre Datei ≤ Spoolgröße), Spool behält den Inhalt für den Retry;
  Konflikt nach dem Upload, `AlreadyExists` beim Ohne-Ersetzen bzw. eine verlorene
  Antwort auf das Umbenennen, nach der die Stage noch besteht → Stage als
  „Name (Konflikt JJJJMMTT-hhmmss)[ (n)].ext“ veröffentlichen (Endung bleibt, nichts wird
  ersetzt). Vergleich von Dateiständen: nur auf einer Seite bekannte Fakten gelten als unbekannt,
  sekundengenau gekürzte Zeit als gleich (`mount/core/baseline_match.rs`).

## D — Arbeitsplan / Abnahme

| M | Inhalt | Dateien | Erwartetes Ergebnis |
|---|---|---|---|
| S1 | Schreibrecht neuer Geräte | `share/core/direct_relation.rs`, `direct_ledger_projection.rs`, `legacy_direct_request_decision.rs`, `profiles.rs`, `direct_reciprocal.rs` | angenommenes Gerät `write == true` (`direct_ledger_tests`, `relation_rights_task_tests`) |
| S2 | Freigabe-Standard je Bereich | `share/core/export_config.rs`, `api_exports.rs`, `app/core/share_exports_ui.rs`, `cli/share/exports.rs`, `mobile/os/shared/domains/share_peers.rs`, Android `ShareDialogHost.kt` | `share_rights_task_new_direct_exports_write_and_room_exports_read` |
| S3 | Home-Freigabe Direkt | `share/core/profile_migration.rs` | `share_rights_task_restricted_direct_home_writes_again_once`, angepasste FC1-Migrations-Tests |
| S4 | Laufwerk ohne Zwischennamen | `mount/core/file_commit.rs`, `file_commit_stage.rs`, `baseline_match.rs`, `commit.rs`, `replace.rs` | `mount_save_task_*` (Upload-Fehler, Konfliktkopie mit Endung, verlorene Umbenennungs-Antwort, Vergleichsregel) |
| S5 | Doku | `README.md`, `docs/TODO.md`, `docs/ARCHITEKTUR.md`, dieser Plan | – |
| S6 | Suite + Release | `native/test-share-write-mount-save-task.py`, `.github/workflows/share-write-mount-save-task.yml` | ein grüner Lauf, dann Release |

## E — Fortschritt
- 2026-10-09: S1–S5 umgesetzt (lokal, rustfmt sauber). Push erst nach Abschluss des laufenden
  Releases der Medien-Navigation (`build.yml` 37931532338), damit dessen Versions-Commit nicht
  abgewiesen wird.
- 2026-10-09: Nachtrag S4: auch nach einer verlorenen Antwort auf das Umbenennen (Stage besteht
  noch, Ziel verändert) wird die Stage als Konfliktkopie veröffentlicht
  (`mount_save_task_lost_promotion_reply_never_leaves_the_stage_name`). Die genaue Ursache auf
  dem Gerät des Nutzers ist ohne dessen Protokolle nicht belegt; abgedeckt sind alle Pfade, auf
  denen `file_commit.rs` die Stage zuvor liegen ließ (Upload-Fehler, nicht bestätigte Prüfung,
  wirkungsloses Umbenennen, Konflikt nach dem Upload, `AlreadyExists`, mehrdeutige Antwort).
  Offen bleibt nur ein Abbruch des Prozesses bzw. der Verbindung mitten im Upload.
