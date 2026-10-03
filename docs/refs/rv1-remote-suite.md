# RV1: schmale Remote-Suite-APIs

Geprüft am 2026-10-03. Diese API-Lückenprüfung betrifft ausschließlich die gemeinsame
Abnahme des vorhandenen RV1-Plans, keinen zusätzlichen Projekt-Review.

## Rust/Cargo

Primärquellen: [cargo test](https://doc.rust-lang.org/cargo/commands/cargo-test.html),
[libtest](https://doc.rust-lang.org/rustc/tests/index.html),
[Cargo-JSON](https://doc.rust-lang.org/cargo/reference/external-tools.html#json-messages).

`cargo test --locked -p smart_explorer --lib --no-run --message-format=json` baut nur den
gewählten Library-Testhost, ohne ihn auszuführen. `compiler-artifact` mit
`profile.test == true`, dem passenden Targetnamen und `executable` entdeckt die Ausgabe.
Der Share-Server hat einen eigenen Bin-Testhost; sein `--bin` wählt nur diesen.
Cargo-Ausgaben bleiben in ihrem inkrementellen Cache; kein Workspace-/All-target-Build.

Die direkte libtest-Datei akzeptiert mehrere Filter als OR von Teilstrings.
`--list --format terse` entdeckt tatsächliche Namen, `--include-ignored` nimmt die
explizit bereitgestellten Umgebungsfixtures dazu, `--test-threads=1` schützt gemeinsamen
Prozesszustand. `--exact` gilt für sämtliche Filter und eignet sich nur für volle Namen.
Windows darf statt einer überlangen Liste voller Namen kurze Filter verwenden, sofern
entdeckte Auswahl und tatsächliche erfolgreiche Ergebnisse vollständig abgeglichen werden.

## Subprozesse

Primärquelle: [Python subprocess](https://docs.python.org/3/library/subprocess.html).
Die vorhandene Suite `native/test-sync-paths-task.py` ist das geprüfte Projektmuster.
`Popen` erhält Argumentlisten, `cwd` und eine kopierte `env`; kein Shell-String für dynamische
Werte. POSIX `start_new_session=True` trennt die Kindgruppe. `wait(timeout=...)` wirft
`TimeoutExpired`; danach beendet der Besitzer die Gruppe und wartet auf Completion.
Windows benutzt `CREATE_NEW_PROCESS_GROUP` und das bestehende `taskkill /PID ... /T /F`-
Muster. Logs werden in Dateien geführt, damit keine Pipe durch ungeholte Ausgabe blockiert.

## Android und OS-Fixtures

Die etablierten, versionsgebundenen Android-Aufrufe bleiben in
[android-ci.md](android-ci.md), [android-toolchain.md](android-toolchain.md) und
[android-gradle-build.md](android-gradle-build.md). Der vorhandene Gerätehelfer liefert
`coreTest`, `Api`, `Fixture`, `SyncJobs`, `Share` und JNI-Calls; keine Mock-Erfolge ersetzen sie.
Die Suite baut nur x86_64-JNI/Debug/Test-APK und übergibt Entwicklungs-CLI/Server aus Linux.

Reales privates IPv4, ifindex und Interfacename kommen aus `ip -j` beziehungsweise
`Get-NetIPAddress` und werden vom Statusfixture gegen OS-Fakten erneut geprüft. FUSE,
FAT32 und exFAT werden auf dem isolierten Linux-Runner eingerichtet und in `finally`/Trap
abgehängt; alle Laufzeitpfade/Ports/Releaseassets stammen aus den Einrichtungskommandos.
