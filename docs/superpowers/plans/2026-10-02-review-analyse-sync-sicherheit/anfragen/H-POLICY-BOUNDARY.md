# H-POLICY-BOUNDARY – Übergabe und Owner-Grenzen

Stand: 2026-10-03. Beauftragte Verschiebung abgeschlossen; keine offene Source-Freigabe und keine neue Review-Liste.

## Aufgelöste Integrationsgrenze

Die freigegebenen direkten Typ-Consumer verwenden `fs_host_policy::TargetPolicy`. Root hat per eigener enger Integrationssuche genau den außer-scope Aufruf in `native/src/share/core/fs_guard_bulk.rs:32` bestätigt und selbst migriert. Der Worker hat die Datei nicht gelesen oder geändert.

`fs::local_paths` bleibt erhalten. Der parallele H-REPLACE-Anschluss an `fs_access/server_fs/Peer/Wire` kann dieselben `secure_local_target/to_os_path`-Pfade verwenden. Eigene Registrierungen wurden nur additiv in `share/mod.rs` eingefügt; parallele S09-/Root-Einträge bleiben dem jeweiligen Owner zugerechnet.

## Gemeinsame Remote-Abnahme

Der Hauptagent übernimmt die bereits geplante eine Suite. Verwendbar bleiben die AcceptanceSelector für private Namen, Nur-lesen, Systemklassifikation und Systemopt-in aus H-DISPATCH sowie `local_target_stays_under_root` und `symlink_escape_is_blocked_when_supported`. Host-Fixtures liegen jetzt in `share::fs_host_policy_task_tests`; exakte vollqualifizierte Pfadänderungen stehen in [api-delta/H-POLICY-BOUNDARY.md](../api-delta/H-POLICY-BOUNDARY.md).

Erwartung: gleichbleibende Berechtigungs-/Privat-/Systemortprüfung bei Local/UNC und bisherigen physischen Aliasfällen; Windows-Case-/Verbatim-/UNC-Verhalten und literale Namen bleiben erhalten. Normale Provider-Stages, Fähigkeiten, Cancel und Backpressure bleiben im vorhandenen Guard-Vertrag. Die statischen Parser-/Textsignale ersetzen keinen Windows-/Unix-Laufzeitnachweis.

## Bekannte unveränderte Restgrenzen

Die bereits in [anfragen/H-DISPATCH.md](H-DISPATCH.md) abgegrenzten allgemeinen pfadbasierten LocalBackend-TOCTOU-Fenster, privaten Datei-Hardlinks außerhalb privater Ancestry und Remote-Listing-Allokationen gehören nicht zu dieser Verschiebung. Der Block erweitert diese Fähigkeiten nicht und meldet sie nicht erledigt.

Commit/Push, Root-Graph-Aktualisierung, gemeinsame Remote-Auswertung und terminaler Release bleiben beim Hauptagenten. Der Worker hat nur kurze statische Text-/Parsing-Arbeit und Self-Review seiner Änderungen ausgeführt.

## Exaktes Inventar

Gelesene, geänderte, neu erstellte und gelöschte Dateien sind vollständig in [abnahme/H-POLICY-BOUNDARY.md](../abnahme/H-POLICY-BOUNDARY.md) aufgeführt. Die gelöschten Core-Hostdateien wurden nach Migration der zugewiesenen Referenzen entfernt. Es gibt keine zusätzliche offene Anfrage außerhalb des bereits vorgesehenen Hauptagent-Anschlusses.

