# CI-5 ROOT – genaue Formatierung und Job-Zeitvertrag

Stand: 2026-10-04. Nur tatsächliche Diagnose aus Run 37167542206 auf `1378bc8fdb796ac2102ccb1e62e8fb74fe0bd796`; frischer eigener Planabschnitt in `ci-fifth-fixes.md`. Keine neue Review und keine lokale Ausführung.

Der gegen SHA-256 und den unveränderten Checkout geprüfte Remotepatch `081aefbc9c36fc26051dbb2c346aee2159cc8a9d14c7be8d19c51d5e28dde88b` ist in `36eaafe9` übernommen. Headerprüfung beschränkt ihn auf die fünf unten genannten bereits bestehenden Rustquellen; `git apply --check` prüfte lediglich den Textanschluss. Die Remote-Metadaten enthalten keine Größenprobleme.

Der SavedJobfall scheiterte an der umgekehrten Fixtureerwartung. Die frisch gelesene gespeicherte Planzeile 715–716 und die Produzenten definieren `last_attempt = started`, `last_success = finished`; beide erscheinen in Millisekunden im mobilen JSON. Das bestehende JNI-Fixture verlangt jetzt einen positiven Start und Abschluss≥Start mit tatsächlichem Status im Fehlertext. Erfolgreiche Zielbytes sowie alle folgenden Check-, Access-, Problem-, Retry-, Runner- und Successassertions bleiben erhalten. Keine Produktzeit oder Feldbedeutung geändert.

Leseinventar dieses Milestones: `ci-fifth-fixes.md`, `umsetzung.md:715–716`, `native/src/syncjobs/os/shared/job_state.rs` (JobState-Zeitfelder), `job_state_policy.rs` (apply_attempt), `native/src/mobile/os/shared/domains/sync_state_json.rs` (attach/ms), `android/app/src/androidTest/java/app/smartexplorer/android/task/ReviewSyncTaskTest.kt` (bestehender SavedJobfall), `/tmp/rv1-ci-fifth/linux/format.json`, `format.patch` und `/tmp/rv1-ci-fifth/device/sync-savedJobAttemptsKeepChecksAndAccessFailuresDistinct.log`.

Geändert durch den exakten Remotepatch: `native/src/bisync/os/shared/checkpoint_review_tests.rs`, `tests/hash_walk.rs`, `tests/links.rs`, `native/src/local_access/os/windows/directory_rename.rs`, `native/src/vfs/os/linux_os/namespace_flush.rs`. Danach geändert: die obige Android-Fixturedatei. Neu: dieser Bericht. Keine Dateien gelöscht.

Eigener statischer Self-Review: minimale Fixtureänderung gegen die gespeicherten tatsächlichen Produzenten; die ursprünglichen Folgeassertions bleiben. Noch kein Laufzeitnachweis: ausschließlich Roots gleiche vollständige RV1-Suite nach Integration aller zugeordneten Owner. Keine offene Scopeabhängigkeit dieses Milestones.
