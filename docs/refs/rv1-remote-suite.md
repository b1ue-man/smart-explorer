# Gemeinsame Remote-Task-Suite

## Aktiver Gesamtbatch B1–B8, P31 und P32

Stand und Primärquellen erneut geprüft am 2026-10-26. Der bestehende Einstieg
`.github/workflows/review-task.yml` → `native/test-review-task.sh` wird für den
vollständigen laufenden Gesamtbatch verwendet. Der erste Lauf ist am 2026-10-26 in der Remote-Vorbereitung beendet;
Kompilierung und funktionale Ausführung haben noch nicht begonnen. Maßgeblich sind `docs/plaene/2026-10-24-batches/batches.md`,
`vertraege.md`, `umsetzung.md` und `native/review-task-manifest.json`.
Vor dem ersten Lauf müssen der normale frische Astra/xhigh-Gesamtreview und die
zusätzlichen unabhängigen Astra/max- und Sol-6.1/max-Regressionsreviews erfolgt
sein. Alle Agenten laufen über Codex. Die Regressionen werden gegen den
veröffentlichten Tag v0.5.184 geprüft; Scope, API-Beleg und Fixvertrag stehen in
`docs/plaene/2026-10-24-batches/regressionsreview.md`. Jeder aus diesen beiden
Reviews hervorgehende Fix einschließlich Verknüpfungskarte, Plan und anschließender
Anti-Regressionsprüfung bleibt vollständig bei demselben Sol-6.1/max-Fixagenten.
Die Laufzeitbestätigung nutzt ausschließlich diese eine Remote-Suite.

Das Manifest bindet alle aktiven P-Meilenstein-IDs an die konkrete Auswahl und
deren erwartetes Verhalten. Der native Runner entdeckt die verfügbaren Namen
aus dem tatsächlich gebauten Library-Testhost, verwendet nur `batches_`,
`p18d_` und die ausdrücklich benannten direkt betroffenen Integrationen und
startet diese Auswahl einmal pro Zielplattform. Ein ausschließlich vom
optischen Besitzerfixture gestarteter Kindprozess ist aus dem Hauptlauf
entfernt. Fehlende Namen oder fehlende erfolgreiche Abschlüsse sind Fehler.
Das Manifest und die Ergebnisdateien nennen ausdrücklich die Grenzen ohne
Live-Anbieter, physische Medien, reale sichere Desktops, öffentliche
Netzpfade, OEM-Codecs und vollständige visuelle Abnahme. Ein Fixtureerfolg
bestätigt nur die aufgeführten ausgeführten Fälle.

Der P27-Leistungsnachweis verwendet zusätzlich genau den ignorierten Fall
`screen::core::rate_control::p27_acceptance::p27_network_reference_acceptance`
aus demselben Library-Testhost. Er ist ausdrücklich aus der allgemeinen Auswahl
ausgeschlossen. Der Linuxteil desselben Suiteeinstiegs startet ihn über
`native/review-task-network.sh` in einem eigenen Netzwerk-Namespace. Nur dessen
Loopback wird geformt; der private gepinnte Iroh-Relay erhält synthetische Frames
durch den produktiven Screen-Sendepfad. Die drei Profile sind 10 Mbit/s mit einem
Fenster, 1 Mbit/s mit einem Fenster und 10 Mbit/s mit vier Fenstern. Referenz,
Rampenfrist, Nutzdurchsatz, Steuerlatenz und Queuealter werden nach V7 ausgewertet.
`p27-network/acceptance.json` enthält die Messwerte und Profilbelege; sechs
Rohdateien halten Referenz und Screenmessung getrennt. Der gemeinsame Runner
prüft Kandidat, genaue Profile und erfolgreichen Ergebnisstatus und schreibt
`p27-acceptance.json`. Das ist kein physischer Encoder-, Display- oder
öffentlicher Relaynachweis. Es liegen noch keine ausgeführten Messwerte vor.

Die für diesen privaten Relay erforderlichen Iroh-Devfeatures können den
Lockbestand erweitern. Deshalb beginnt derselbe Remote-Suiteeinstieg mit einer
Abhängigkeitsphase ohne Kompilierung. `cargo metadata --format-version=1` löst
den vorhandenen Manifest-/Lockbestand auf. Veränderte Lockbytes werden als
kandidatengebundene `dependencies.patch` und `Cargo.lock.resolved` ausgegeben;
die Buildjobs starten in diesem Fall noch nicht. Main übernimmt die geprüfte
Auflösung in einen Commit und wiederholt denselben Suiteeinstieg. Alle Builds
verwenden danach weiterhin `--locked`. Diese Phase läuft ausschließlich remote;
sie erzeugt weder einen zweiten Suiteweg noch einen Releasebau.
Die bereits bestehende Format-/Größenauswertung läuft ebenfalls in dieser
Vorbereitungsphase. Dadurch stehen nötige Lock- und Formatkorrekturen gemeinsam
vor dem ersten Build zur Verfügung; der native Ausführungsteil wiederholt die
Formatierung nicht.
Der Cargo-Vertrag wurde am 2026-10-26 gegen
[Cargo metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html)
geprüft. Eine vollständige Neuauflösung aller Pins über `generate-lockfile`
gehört nicht zu diesem Weg.

Die abgeschlossenen Reviewfixes sind in R10 bis R18 mit ihren exakten
Fallnamen registriert: Android-Medienbesitz, Bildqualität, Gerätenavigation,
Windows-/X11-Aufnahmewiederanlauf, X11-Familienfokus, Screen-Sitzungsbudget,
Transferkompatibilität, Vault-Retry und MCP-Lebenszyklus. Die MCP-Fälle trennen
die gemeinsame Auswahl von Linux- und Windows-Dateisperrverträgen. Die
beauftragten Fixautoren haben ihre jeweiligen Kettenkarten und anschließenden
statischen Anti-Regressionsreviews selbst abgeschlossen; diese Registrierung
behauptet noch keine ausgeführte Funktion. Belege stehen in
`docs/plaene/2026-10-24-batches/regressionsreview.md`.

Nach der fachlichen Modulaufteilung ergänzt R19 die bereits bestehenden Fälle
`remote_drive_task_dokany_msi_outcomes_preserve_actionable_exit_codes` und
`remote_drive_task_dokany_library_and_driver_versions_use_separate_domains`.
Sie prüfen unter Linux die unmittelbar verschobenen reinen Runtimetypen;
MSI-Ausführung und Kernel-/FUSEmount sind nicht Gegenstand dieser Registrierung.

Die Ressourcenabschnitte C1–C11, L1–L8, Android-F2–F6, Android-M1–M3/M5–M8, Glow-F3
und Capture-R2–R5/Linux verwenden
zusätzlich explizite Falllisten in `resource_milestones`. Der Runner löst diese
Namen gegen die wirklich ausgewählten Methoden auf und nennt fehlende,
mehrdeutige und nicht ausgeführte Fälle getrennt. C3/C4 benennen insbesondere
Prepare/Activate, Originalcredits, grantlosen Start, Abbruch, EOF, Prozessende
und gepufferte Antworten. C8 bindet den produktiven seriellen Familienplaner;
C11 trennt neue Quellenidentität von weiterhin gemeinsamem W/X-Besitz. Die
Registrierung der Fälle ist kein Beleg ihrer Ausführung. Android-M1–M3 benennt
die Claim-/Alias-, Konstruktor- und Produzentenabschlussfälle aus
`fix-android-capturebesitz.md`; M5–M8 ergänzen Projection-, Screenshot-, AU-
und Hybridinputgrenzen. Echte Android-Allokations- und Codecfehler ohne
Injektion bleiben dort ausdrücklich statisch geprüft. F3-Eingabefreigabe bindet die kontrollierten
Login-/SAS-/Screen-Guardfälle aus `fix-renderer-eingabefreigabe.md`. Die
Ressourcenbesitzer bleiben in diesen Fällen produktiv, die injizierten
Fehlerereignisse sind keine beobachteten Gerätedefekte. Capture-R2–R5 bindet
Parent-/Familienzusatzcredits, den echten Transportbesitz und Worker-Turnover
an die Fälle aus `fix-capture-integration.md`; Capture-Linux ergänzt X11-Cursor
und PipeWire-Wiederzulassung. Kontrollierte Protokoll- und Lebensdauerfälle
ersetzen keinen echten X11-/Wayland-Compositorlauf.

OpenH264 wird für den betroffenen Codecfall ausschließlich aus dem vorhandenen
Cisco-Pin mit festem HTTPS-Ziel, komprimierter Größe und SHA-256 bereitgestellt.
Der Download findet im temporären Remote-Fixtureprofil statt. Fehlendes Netz
wird als nicht verfügbare Voraussetzung protokolliert; falsche Bytes sind ein
Fehler. OpenH264- und Media-Foundation-Fälle können eine ausdrücklich benannte
Nichtausführung melden, die nicht als bestanden zählt.

Die beiden WGPU-Rendererfälle sind im Manifest ausdrücklich umgebungsabhängig.
Fehlt tatsächlich ein Adapter, liefern sie eine benannte strukturierte
Nichtausführung. Der Runner übernimmt diese Fälle in `not_executed`, entfernt
sie aus `passed_fixtures` und aus den ausgeführten Meilensteinbelegen. Ein
unbekannter oder doppelt gemeldeter Ausnahmename scheitert. `SE_SCREEN_TASK_GPU=1`
macht einen fehlenden Adapter weiterhin zum Fehler. Der kontrollierte
Glow-Besitzernachweis behauptet keinen realen GL-Kontext oder Treiber-Fence.
Der gepinnte `egui_glow`-Library-Testhost gehört zu demselben Suiteeinstieg:
`native.glow_scope` benennt ausschließlich seine Fehlerledgerfälle. Der Runner
verwendet denselben Lock und Buildcache, filtert auf genau deren Präfix und
schreibt `glow-scope-acceptance.json` mit Kandidat, Binärhash und tatsächlichen
Abschlüssen. Kontrollierte GL-Fehlerwerte gelten nicht als physischer Treiberfehler.
Die Library benötigt dafür keine Upstream-Fensterbeispiel-Devabhängigkeiten;
der begrenzte Vendor-Manifestunterschied steht in
`vendor/SMART_EXPLORER_GL_ERRORS.md`.
Der am 2026-10-26 erneut geprüfte
[libtest-Schalter `--show-output`](https://doc.rust-lang.org/rustc/tests/index.html#--show-output)
stellt auch die Ausgaben erfolgreich zurückgekehrter Fälle für diese Auswertung bereit.

Linux und Windows verwenden je einen inkrementellen Library-Testhost und die
Entwicklungs-CLI `se` für den tatsächlich gestarteten Screenworker. Die
Buildfingerprints schließen die gepinnten Iroh-/Noq- und egui_glow-Quellen unter
Root-`vendor/` ein. Quelle, Compilerstand, Buildoptionen und Ausgabehash müssen zum Cache passen.
Der unveränderte Share-Server wird für diese Auswahl nicht neu gebaut.
AppData-/XDG-Verzeichnisse liegen in temporären Fixtureprofilen; HOME und
CODEX_HOME werden nicht umgebogen. Die native Auswahl läuft seriell wegen
prozessweiter Fixturevariablen; Relay-Umgebungsüberschreibungen werden entfernt.

Android baut einmal x86_64-JNI, Debug-APK und Instrumentation-APK und führt die
JVM-Klassen aus dem Manifest aus. `unit-acceptance.json` ordnet die tatsächlich
ausgewerteten Gradle-XML-Ergebnisse den P-Meilensteinen und den expliziten
Ressourcenfällen zu; diese Zuordnung startet keinen zusätzlichen JVM-Lauf.
Der bestehende Quellauswerter berücksichtigt sowohl `@Test fun` auf einer Zeile
als auch getrennte Annotation-/Funktionszeilen. Das standardmäßig abgeschaltete Cargo-Feature
`android-instrumentation-fixtures` ist ausschließlich für diesen Debugbau
aktiv. Gradle verwendet dessen Bibliotheken aus `src/debug/jniLibs`; Release
verwendet weiterhin die getrennten produktiven Bytes aus `src/main/jniLibs`.
`android/check-image-fixture-package.py` liest die tatsächlichen DEX-Klassendefinitionen
und die dynamischen JNI-Exporte mit NDK-`llvm-nm`: Debug braucht die Fixtureklasse
und ihre vollständigen Exporte; die Test-APK enthält keine eigene `.so`.
Der terminale Release prüft an der bereits gebauten APK, dass Fixtureklasse und
JNI-Exporte fehlen. Er erzeugt dafür keinen zusätzlichen Build. Eine aktivierte
Fixturefunktion in einem optimierten Rustbau wird bereits per `compile_error!`
abgewiesen. Diese Quellanschlüsse sind noch kein ausgeführter APK-Nachweis.

Nur diese beiden Debug-/Instrumentation-APKs werden mit Kandidaten-SHA
und SHA-256 an den Emulator übergeben. Der Gerätehelfer entdeckt den installierten
Runner, dessen Zielpaket und das app-private Verzeichnis selbst. Er erzeugt auf
dem Remote-Runner ein zwölf Sekunden langes H.264/AAC-Video und übergibt dessen
wirklichen Pfad als `batchesVideoPath`. Die Instrumentation führt die gesamte
Auswahl in einem Aufruf aus; der bestehende strenge Auswerter akzeptiert keine
übersprungenen oder fehlenden Fälle. Systemlayoutwechsel werden über Androids
Rotationszustand ausgelöst und in den Fixtures zurückgesetzt; das belegt keinen
physischen Rotationssensor.

Die Routineprüfung umfasst echte Dateien und Recovery-Dokumente, atomare
Bildrekonstruktion, gepinnte TCP-Sockets, den realen Iroh-/Noq-Sendegate,
Compose-Darstellung sowie Androids Medienwiedergabe. Mock-Provideridentitäten,
synthetische Ressourcenfakten und simulierte OS-Dispatchs sind in den jeweiligen
Fixtures ausdrücklich gekennzeichnet. Der native Größen-/Formatbericht bleibt
kandidatengebunden; Rustfmt wird ausschließlich remote ausgeführt. Die
STDIN-Übergabe formatiert nur den übergebenen Text: Die am 2026-10-26 gelesene
[Rustfmt-Modulauflösung](https://github.com/rust-lang/rustfmt/blob/master/src/formatting.rs)
schaltet die rekursive Auflösung bei `FileName::Stdin` ausdrücklich ab. Damit
wird aus diesem Bericht kein zusätzlicher Formatlauf über ungeänderte Module. Die nativen
Jobs haben 360 Minuten, Android-Build 240 Minuten und der Gerätejob 360 Minuten.
Eine Fehlerkorrektur verwendet denselben Einstieg und den inkrementellen Cache.
Releasebau und Veröffentlichung gehören nicht in diese Entwicklungssuite.

API-Belege: [libtest-Auswahl](https://doc.rust-lang.org/rustc/tests/index.html),
[Android Instrumentation](https://developer.android.com/studio/test/command-line),
[FFmpeg-Filter](https://ffmpeg.org/ffmpeg-filters.html),
[Python subprocess](https://docs.python.org/3/library/subprocess.html).

API-Abgleich der Paketgrenze, 2026-10-26:
[Android-Quellvarianten](https://developer.android.com/build/build-variants#sourcesets),
[AOSP-DEX-Klassen-, Typ- und Stringtabellen](https://source.android.com/docs/core/runtime/dex-format),
[LLVM-nm für dynamische definierte Exporte](https://llvm.org/docs/CommandGuide/llvm-nm.html).
Die erlaubte gleichzeilige Kotlin-Annotation wurde am 2026-10-26 gegen
[Kotlin: Java annotations](https://kotlinlang.org/docs/annotations.html#java-annotations)
abgeglichen; alle im aktuellen Androidbestand vorhandenen Testdeklarationen
werden durch den statischen Quellabgleich erfasst. Das ist keine Testausführung.

## Historische RV1-Recherche und damalige Runnerfälle

Die folgenden Abschnitte halten frühere RV1-/Sync-Untersuchungen fest. Ihre
FUSE-, Volume-, Altversions- und Share-Serverauswahl gehört nicht automatisch
zum aktuellen Gesamtbatch; die aktive Auswahl steht ausschließlich im Manifest.

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

Am 2026-10-05 für den Sync-Fixloop frisch geprüft:
[`--no-capture`](https://doc.rust-lang.org/rustc/tests/index.html#--no-capture)
liefert stdout/stderr unmittelbar; `--show-output` wartet auf das Ende aller
Fälle. `--format=pretty`, `--color=never` und `--test-threads=1` sind stabile
Optionen. JSON-Ausgabe wäre instabil, `--logfile` ist deprecated; beides wird
nicht eingeführt. Der
[PrettyFormatter](https://github.com/rust-lang/rust/blob/master/library/test/src/formatters/pretty.rs)
schreibt bei serieller Ausführung vor dem Fall dessen Namen und danach das
geflushte Ergebnis mit Newline. Dazwischen dürfen Diagnosen stehen. Der
Suite-Parser ordnet die Abschlusswörter deshalb den Namensabschnitten zu und
verlangt zusätzlich eindeutige vollständige Namen, genau einen Laufheader
und eine terminale Summary mit passenden Passed/Failed/Ignored-Zählern und
ohne gemessene Benchmarks. Ein Prozessabbruch bleibt fehlgeschlagen; bis
dahin ausgegebene Ursachen bleiben im selben Log erhalten.
Die dafür verwendete
[`collections.Counter`](https://docs.python.org/3/library/collections.html#collections.Counter)
zählt die Startnamen und Abschlusszustände; fehlende Schlüssel haben den
Wert null. Die API wurde am selben Tag gegen Python 3 überprüft.

## Subprozesse

Primärquelle: [Python subprocess](https://docs.python.org/3/library/subprocess.html).
Die vorhandene Suite `native/test-sync-paths-task.py` ist das geprüfte Projektmuster.
`Popen` erhält Argumentlisten, `cwd` und eine kopierte `env`; kein Shell-String für dynamische
Werte. POSIX `start_new_session=True` trennt die Kindgruppe. `wait(timeout=...)` wirft
`TimeoutExpired`; danach beendet der Besitzer die Gruppe und wartet auf Completion.
Windows benutzt `CREATE_NEW_PROCESS_GROUP` und das bestehende `taskkill /PID ... /T /F`-
Muster. Logs werden in Dateien geführt, damit keine Pipe durch ungeholte Ausgabe blockiert.

## Eigene CI-Reportarchive mit privaten Fixtureprofilen

Am 2026-10-05 gegen die
[GitHub-Artifacts-API](https://docs.github.com/en/rest/actions/artifacts#delete-an-artifact)
geprüft: `GET /repos/{owner}/{repo}/actions/artifacts/{artifact_id}` liefert
Name und Workflowzuordnung. Vor Entfernen eines eigenen Diagnosearchivs
werden ID, erwarteter Name, abgeschlossener Run und exakter Kandidat geprüft.
`DELETE` an derselben Adresse verlangt Actions-Schreibrecht und bestätigt
mit HTTP 204; die folgende Sichtprüfung muss HTTP 404 liefern.
Im Sync-Fixloop werden nur die zwei identifizierten nativen Reportarchive mit
privaten Altworkerprofilen entfernt, nachdem die kombinierte Summary und
gezielte nicht sensitive Diagnose gesichert sind. Keine Release-/APK-/
Binärartefakte oder fremden Runs werden dadurch geändert.

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

## Isolierter FUSE-/Volume-Anschluss

Am 2026-10-03 geprüft: [SSHFS-Upstream](https://github.com/libfuse/sshfs),
[SSHFS-Manpage](https://raw.githubusercontent.com/libfuse/sshfs/master/sshfs.rst),
[losetup](https://man7.org/linux/man-pages/man8/losetup.8.html),
[mount](https://man7.org/linux/man-pages/man8/mount.8.html). SSHFS unterstützt
`-o directport=PORT` zu einem auf Loopback gebundenen `socat` mit OpenSSH
`sftp-server`; dies bleibt ein echter FUSE/SFTP-Mount auf dem isolierten Runner.
`fusermount3 -u` hängt ihn ab. Der Einstieg besitzt die eigene Socat-Prozessgruppe.
`losetup --find --show` entdeckt die jeweilige Loopdatei; die Suite hängt ihre
FAT32-/exFAT-Mounts vor `losetup --detach` ab. Nicht-interaktives sudo wird zuerst
geprüft. Die Entwicklungssuite verändert keine Produkt-Transportvorgabe.

Der zweite Runnerlauf vom 2026-10-03 meldet fehlende exFAT-Kernelunterstützung.
[exfat-fuse](https://github.com/relan/exfat/blob/master/fuse/mount.exfat-fuse.8)
unterstützt `mount.exfat-fuse -o uid=N,gid=N,umask=077 DEVICE DIR`.
Sein [Mountcode](https://github.com/relan/exfat/blob/master/fuse/main.c)
setzt `blkdev`, `allow_other`, `default_permissions` und den tatsächlichen
Gerätepfad als `fsname`; beliebige Subtypeoptionen werden nicht durchgereicht.
Deshalb erkennt der OS-Adapter den realen `fuseblk`-Quellblocktyp über
`MetadataExt::rdev`, `FileTypeExt::is_block_device` und udev `ID_FS_TYPE`
(Syntax in [local-fs-identity-durability.md](local-fs-identity-durability.md)).
Nur die Speicherlimits stammen vom unteren Typ; FUSE bleibt `PerFileOnly`.
Der Einstieg hängt jeden tatsächlich erzeugten Mount vor seinem Loopgerät ab.

## Öffentlicher Android-AppOps-Anschluss

Am 2026-10-03 gegen [AppOpsManager](https://developer.android.com/reference/android/app/AppOpsManager#permissionToOp(java.lang.String))
und [AOSP](https://android.googlesource.com/platform/frameworks/base/+/master/core/java/android/app/AppOpsManager.java)
geprüft: `permissionToOp(String)` (API 23) liefert den Operationsnamen oder
`null`. AOSP ordnet `MANAGE_EXTERNAL_STORAGE` diesem Namen zu, während seine
direkte `OPSTR_MANAGE_EXTERNAL_STORAGE`-Konstante versteckte System-API ist.
`startWatchingMode(String, String, OnOpChangedListener)` (API 19) beobachtet
nur die eigene UID. Bei fehlender Zuordnung/Runtimefehler bleibt die bestehende
periodische Rechteprüfung aktiv.

## JSON-Capability-Prefix

Am 2026-10-03 gegen den gepinnten
[serde_json 1.0.150 Decoder](https://github.com/serde-rs/json/blob/v1.0.150/src/de.rs)
und die lokale Registryquelle geprüft: `deserialize_map` ruft nach
`Visitor::visit_map` immer `end_map` auf. Frühes `Ok` reicht deshalb bei einem
noch folgenden Payloadfeld nicht. Ein vollständig gelesenes `Hint` wird separat
erfasst und der Visitor gezielt abgebrochen; es gibt kein Matching eines
Fehlertexts und kein Akzeptieren partiell gelesener Authfelder. Der Besitzer
arbeitet nur auf `TcpStream::peek`; die Originalbytes und die abschließende
Nachrichtenvalidierung verbleiben beim vorhandenen Clienthandler.

## Cache und Formatierung des Runners

Runner-APIs am 2026-10-03 gegen [setup-python](https://github.com/actions/setup-python),
[cache restore](https://github.com/actions/cache/blob/main/restore/README.md) und
[cache save](https://github.com/actions/cache/blob/main/save/README.md) abgeglichen.
Die bestehende Projektkonvention `setup-python@v6` mit `python-version: '3.12'`
bleibt verwendbar. Separate Restore-/Save-Schritte erhalten auch fehlgeschlagene
inkrementelle Ausgaben; die Suite validiert jede wiederverwendete ausführbare Datei
selbst gegen alle Buildinputs und SHA-256. Rustfmt wird nur remote über stdin/
stdout verwendet ([Upstream-README](https://github.com/rust-lang/rustfmt/blob/main/README.md)).
Der [ModResolver](https://github.com/rust-lang/rustfmt/blob/master/src/modules.rs)
überspringt externe Kindmodule für stdin; deshalb kann jede geänderte Datei separat
einen präzisen Patch liefern, einschließlich additiver Modulregistrierungen.

## Dritter Lauf: wirkliche FUSE-Zeitauflösung und Watchlimit

Geprüft am 2026-10-03 gegen [exfat-fuse 1.3.0 node.c](https://raw.githubusercontent.com/relan/exfat/v1.3.0/libexfat/node.c) und [exfat.h](https://raw.githubusercontent.com/relan/exfat/v1.3.0/libexfat/exfat.h): `exfat_utimes` übernimmt `tv[1].tv_sec` in `node->mtime`; das Feld ist `time_t`. Subsekunden werden nicht gespeichert. Folgerung: reale FUSE-exFAT-Blockvolumes melden Sekundenauflösung, ohne Kernel-exFAT von 10 ms herabzusetzen oder FUSE einen volumenweiten Flush zuzusichern.

[inotify(7)](https://man7.org/linux/man-pages/man7/inotify.7.html) beschreibt `/proc/sys/fs/inotify/max_user_watches` als echtes Limit pro UID. Die vorhandene Fixture erzeugt 129 Verzeichnisse; das normale Runnerlimit kann deshalb keinen WatchLimit belegen. Die Suite entdeckt ihren vollständigen Namen im vorhandenen Listing und startet genau diesen Fall innerhalb derselben Runtime separat mit einem echten Limit von 16. [sysctl(8)](https://man7.org/linux/man-pages/man8/sysctl.8.html) belegt `-n` und `-w`; `sysctl -n fs.inotify.max_user_watches` liest den ursprünglichen Wert; `sudo -n sysctl -w fs.inotify.max_user_watches=N` setzt und restauriert ihn. Alle anderen Fälle behalten das normale Limit, der eine gemeinsame Log enthält beide Ergebnisse, und EXIT/TERM/INT-Cleanup stellt das ursprüngliche Limit wieder her. Kein Fixturefall wird entfernt oder abgeschwächt.
# Ergänzte Reviewgrenzen vom 2026-10-26

G6 bindet die Qualitätsbelege im selben nativen Durchgang über
`native/review_task_quality.py` an Kandidat, Release-SHA, Codecprofile und
gespeicherte BGRA-Dateien. Der Bericht `quality-acceptance.json` unterscheidet
die tatsächlich ausgeführte OpenH264-Gegenüberstellung, kontrollierten
Referenzverlust und verlustfreie Rekonstruktion. Fehlende Codecbedingungen
bleiben `not_executed`; MF-, Android-Codec- und reale Rendererbelege werden
daraus nicht abgeleitet.

Für RSOL-11 läuft der bestehende Linux-Bibliotheksdurchgang unter einem
eigenen Xvfb mit 1280×720×24 und abgeschaltetem TCP-Listener. Die beiden
explizit registrierten X11-Resizefälle verlangen diesen isolierten Display,
MIT-SHM und `SE_SCREEN_TASK_X11=1`; sie erhalten keine stillschweigende
Erfolgsrückgabe bei fehlender Umgebung. Das vorhandene Auswahlverfahren
nimmt ihre ignorierten Fälle innerhalb desselben Durchgangs auf.
`xvfb-run` besitzt Server und temporäre Autorisierungsdatei und reicht den
Exitcode des Bibliotheksdurchgangs weiter. Quelle, geprüft am 2026-10-26:
[Debian xvfb-run](https://manpages.debian.org/testing/xvfb/xvfb-run.1.en.html).
Das belegt nach erfolgreichem Remote-Lauf die isolierte X11-Schnittstelle,
keine physische Desktop- oder Compositorsitzung.

## Lauf 40332971455: gebundene Vorbereitungskorrekturen

Der [erste Lauf](https://github.com/b1ue-man/smart-explorer/actions/runs/40332971455)
prüfte Kandidat `34f3361de13104e386eba26bf16d6a0160475481`. Sein Job
`dependencies` beendete sich vor den Compilerjobs mit den erwarteten
Lockfile-/Formatkorrekturen und zusätzlich gemeldeten Modulgrößenverletzungen.
Linux, Windows und beide Androidstufen wurden dadurch nicht gestartet.

Die Abhängigkeitsauflösung erhält alle bereits gesperrten Paketversionen und
Checksummen; sie ergänzt die vom vorhandenen Manifest aktivierte Closure.
Original-Lock-SHA256: `17491c4f5c4f68fdb5c51973e25c25701b822ecb95d5f31dd608f4860e943491`.
Aufgelöster Lock-SHA256: `72c269cf0cda3db0b072dcec437d30e2cc5fa342ca6fc47abe6c7c318f5081a4`.
Remote-Formatpatch-SHA256: `57194cbcc7932a64b7816b20b0327cd48986bc5cff3b3ef522496461c87f8a4b`.
Main hat Kandidat, Hashes und Anwendbarkeit geprüft und die gelieferten Patches
übernommen. Die eingefrorenen Releaseblöcke des Bildqualitätsvergleichs bleiben
bytegleich. Weder Cargoauflösung noch Rustformatierung liefen lokal.

Die im Artefakt `format.json` benannten Verantwortlichkeiten wurden vor der
Wiederholung desselben Suiteeinstiegs in passende Module unter der Projektgrenze
aufgeteilt. Bestehende Funktionsnamen, Zugriffsgrenzen, Drahtformate und
Meilensteinauswahl bleiben erhalten. Aus diesem Lauf wird kein bestandener
Verhaltensfall abgeleitet.

## Lauf 40342520266: Größen bestätigt, verbleibendes Layout übernommen

Der [erneute Versuch](https://github.com/b1ue-man/smart-explorer/actions/runs/40342520266)
verwendete `e127c588b5896a7a6d05861dc6667b6e885eea43`. Seine Vorbereitung
meldet keine Größen- oder Parserprobleme. Die Lockdatei blieb bei SHA256
`72c269cf0cda3db0b072dcec437d30e2cc5fa342ca6fc47abe6c7c318f5081a4`
unverändert; `dependencies.patch` ist leer. Verbleibende Rustformatierung
führte erneut zum vorgesehenen Stopp vor allen Compiler- und Gerätejobs.

Main hat das Artefakt mit ZIP-SHA256
`cd857f2690ba505803108aa10c1ea485fe3cc9c453fd2576c3bef8ba17b12faa`
und den an diesen Kandidaten gebundenen Formatpatch mit SHA256
`6543a7ca3711f5ad7f1a66a2b77834b6c65b2c46808dc88182701210e7395a8c`
geprüft und unverändert übernommen. Die betroffenen Quellen parsen statisch
und bleiben unter den Modulgrenzen. Die ursprünglichen Qualitätsfixtures samt
eingefrorenen Releaseblöcken sind bytegleich; weder Paketauflösung noch Formatter
liefen lokal. Die Wiederholung bleibt derselbe Suiteeinstieg. Dieser Versuch
belegt Quellvorbereitung, noch keine Kompilierung oder funktionale Abnahme.

## Lauf 40344852826: gemeinsamer Vendor-Konstruktorfehler

Der [Lauf](https://github.com/b1ue-man/smart-explorer/actions/runs/40344852826)
prüfte `7fb96ef00bfb864509c4b0918aac211f7eafb5b6`. Die Vorbereitung war
erfolgreich: Abhängigkeits- und Formatpatch sind leer; die Lockdatei blieb bei
SHA256 `72c269cf0cda3db0b072dcec437d30e2cc5fa342ca6fc47abe6c7c318f5081a4`.

Linux, Windows und Android scheiterten übereinstimmend an Rustfehler E0451 in
`vendor/noq-proto-1.3.0/src/connection/mod.rs:376`: Der Struct-Updateausdruck
des Verbindungskonstruktors benötigt Zugriff auf drei private Zählerfelder
des Kindmoduls `application_path_policy`. Linux- und Windowsartefakte enthalten
denselben Fehler sowohl für die Bibliothek als auch für den `se`-Worker;
das Android-Buildlog bestätigt dieselbe Ursache. Die Android-Gerätestufe
wurde übersprungen. Die getrennten Glow-Fälle liefen auf Linux und Windows;
daraus folgt keine Abnahme der gesamten Suite oder der Produktbibliothek.

Die Korrektur bleibt im selben Fixloop. Ihr eigener Bericht
[fix-path-policy-construction.md](../plaene/2026-10-24-batches/fix-path-policy-construction.md)
bindet den legalen Konstruktor an erhaltene Anfangswerte, RNG-Reihenfolge,
Opt-in-Semantik und Pfadfreigaben. Der folgende Kandidat wird ausschließlich
über denselben Remote-Suiteeinstieg bewertet. Kein Release ist gestartet.

## Lauf 40354605251: native Integrationsgrenzen

Der [Lauf](https://github.com/b1ue-man/smart-explorer/actions/runs/40354605251)
prüfte `d95dc404e834edfe7be0e1c96be162b36865efa9`. Die Vorbereitung war
erfolgreich. Der Vendor-Konstruktorfehler ist beseitigt: Linux und Windows
melden ein kompiliertes Noq-proto-Artefakt, Android erreicht ebenfalls die
Smart-Explorer-Quellen. Alle drei Compilerstufen scheitern anschließend an
nativen Integrationsfehlern; die Android-Gerätestufe ist übersprungen.

Main hat die vollständigen Desktopdiagnosen und das Androidlog ausgewertet.
Die Diagnosekennungen C01–C58 und A01–A19 sind keine Zahl voneinander unabhängiger
Funktionsfehler. Die Artefaktarchive wurden gegen Kandidat und GitHub-SHA256
geprüft. Die [sechs begrenzten Korrekturgruppen](../plaene/2026-10-24-batches/rv1-compilerplan.md)
sind durch ihre bisherigen Autoren einschließlich eigener Nachreview umgesetzt.
Die gemeinsame Manifestergänzung `R20-Compiler-Integrationen` wählt ausschließlich
direkt betroffene bestehende Fälle aus; vorhandene Auswahl und Aussagen bleiben
erhalten. Die getrennt erfolgreichen Glow-Fälle belegen weiterhin weder die
gesamte Produktbibliothek noch eine erfolgreiche Gesamtabnahme.

Der nächste Kandidat verwendet unverändert `native/test-review-task.sh` im
Workflow `review-task.yml`. Es gibt keinen zusätzlichen Suiteeinstieg und keine
lokale Kompilierung oder Testausführung. Die vollständige funktionale Auswertung
und der eine terminale Release stehen weiterhin aus.

## Lauf 40366764400: abschließende Formatierung der Compilerkorrekturen

Der [Lauf](https://github.com/b1ue-man/smart-explorer/actions/runs/40366764400)
verwendete `531230bc5344494ec01b307f3424e3120ad0c29c`. Die Vorbereitung
meldet ausschließlich drei Formatänderungen. `format.json` enthält keine
Größen- oder Parserprobleme. Die Lockdatei bleibt bei SHA256
`72c269cf0cda3db0b072dcec437d30e2cc5fa342ca6fc47abe6c7c318f5081a4`;
`dependencies.patch` ist leer. Alle nachfolgenden Stufen wurden übersprungen.

Main hat Archiv-SHA256
`5ed2dc46ec098c313728edc8ba4977987542a0c8c1fbfa031f57757d5cd32eb9`
und Formatpatch-SHA256
`7bdcc835b14bd982d8ca69d655778ecda42e38ad5ead3e4bf7d02fe53501f626`
gegen die GitHub-Metadaten beziehungsweise `format.json` geprüft. Der Patch
ist unverändert übernommen. Die syntaktischen Blatttoken bleiben bis auf
ein abschließendes Argumentkomma gleich; Syntax, Aussagen und Modulgrößen
bleiben erhalten. Kein lokaler Formatter oder Compiler wurde aufgerufen.
Dieser Vorbereitungsbefund ist noch keine funktionale Abnahme.

## Lauf 40368746815: Modul-, Plattform- und JNI-Grenzen

Der [Lauf](https://github.com/b1ue-man/smart-explorer/actions/runs/40368746815)
verwendete `b233e5336a0ee177e19382a7adf592d297dcf202`. Die Vorbereitung
ist erfolgreich. Windows und Linux scheitern an nativen Modul-/Typgrenzen;
Android erreicht `smart_explorer_android` und meldet vier Diagnosen für zwei
Booleanübergaben. Die Android-Gerätestufe wurde übersprungen. Main hat die
vollständigen Compilerartefakte und das Android-Buildlog ausgewertet.

Archiv-SHA256 für diesen Kandidaten:

| Stufe | SHA256 |
|---|---|
| Vorbereitung | `3a4bd5a5a44e9e36a3d809cafd4034737fc5f34b1f8469d62f9c413b1b1f43c9` |
| Windows | `b4c703354514aa987297cc28eacb59c858e82ba2eabbaa589b577612a5e55883` |
| Linux | `72c157d2fed58d8e87c02dc2bbeb40223e1dabebf174282dd8c4e45c12b8f94e` |
| Android-Build | `8877cc034ec9594ddbd216e51852f6cf98854b0e51d36dc4a4e842081eb07327` |

Die [begrenzte Fortsetzung](../plaene/2026-10-24-batches/rv1-compilerplan.md)
ordnet D01–D21 und J01–J04 vollständigen Autorenzyklen zu. Der Suiteeinstieg
bleibt unverändert. Aus den Compilerabbrüchen wird keine Funktionsabnahme
abgeleitet; Veröffentlichung bleibt der eine terminale Schritt.

Die daraus folgenden fünf Quellkorrekturen sind separat committed und vom
jeweiligen selben Autor nachgeprüft. `R20-Compiler-Integrationen` enthält nun
auch die vorhandenen Vertragsfälle für Android-Statustexte, Kontenstatus und
Version, Administrator-Fallback, Desktopdateiaktionen und absolute Execfrist.
Keine frühere Auswahl wurde entfernt. Der Eintritt und die Auswertung bleiben
bei `native/test-review-task.sh`; die Ergänzung behauptet keinen bereits
ausgeführten Nachweis.

## Lauf 40379623936: JNI-Formatvorbereitung

Der [Lauf](https://github.com/b1ue-man/smart-explorer/actions/runs/40379623936)
ist für `5609ce9f834e0a8e32b1f076415b586ae427da68` ausschließlich in
der Formatvorbereitung gestoppt. Das gebundene Archiv
`12678821405` hat SHA256
`a1ee8add9280add5132cd6ba29d43dcd3905c93e7045ae9c3cb97db9ef61b29b`;
sein Formatpatch hat SHA256
`a3e1a34e6faacd3d5fe2f01ab63d43d7a5a9bd6b3e6b0e4214e17afa8e98630f`.
Der Patch fasst ausschließlich fünf Argumente in `screen_capture_jni.rs`
zusammen. Er ist unverändert übernommen; statisch bleiben sämtliche
syntaktischen Blatttoken identisch. Lockauflösung unverändert, keine
Größenprobleme. Windows-, Linux-, Android-Build- und Gerätestufen wurden
übersprungen. Kein lokaler Formatter oder Compiler wurde ausgeführt.

## Lauf 40381452096: native Kompilierung und erste Ausführungsbefunde

Der [Lauf](https://github.com/b1ue-man/smart-explorer/actions/runs/40381452096)
für `0425aa4608d7e27eb95ee0a37e3bc13dca2ce8bf` ist vollständig beendet.
Die Vorbereitung ist erfolgreich. Windows und Linux liefern erfolgreich
kompilierte Produktbibliotheks-Prüfbinärdateien, `se` und die betroffene
`egui_glow`-Prüfbinärdatei. Android liefert die native Bibliothek samt JNI und
bestätigt die verlangte LOAD-Ausrichtung, scheitert dann bei Kotlin. Die
Android-Gerätestufe ist übersprungen.

Main hat Kandidatbindung, Archivhash, sämtliche Fehlerdiagnosen und die
betroffenen Quellstellen geprüft:

| Stufe | Artefakt | Archiv-SHA256 |
|---|---|---|
| Vorbereitung | `12679017738` | `e8037c1bda41fa8ef2d2080fd191d31e88a3ed7fde35bf18e37f71e7647e36be` |
| Android-Build | `12678538449` | `115e29d16f796903e46e4d51a7be74c1381dd157b64b46f318f10e4b8bca6408` |
| Linux | `12681652291` | `b3aa1cea1d362661b9914726cb6f7a2962f0ab52212ada59b2812bd2daf08c61` |
| Windows | `12682562310` | `96f22a7e5ab866782c9b72f591c862c106833ab042bfef05c53e94cfd8a337d7` |

K01–K11 betreffen Android-Iconkonsum, Displaynullbarkeit und Budgetaufrufe.
L01/W01 sind Main zugeordnete Auswahlfehler: ein ersetzter Snapshotname und
mobile, nur in Unix-Prüfläufen vorhandene Fälle. Der vollständige statische
Abgleich gegen beide echten Binärlisten ist Grundlage ihrer Zuordnung.
Die kontrollierte GL-Fehlerledgerprüfung liefert ihren vorgesehenen Beleg;
ein physischer Grafiktreiber wurde damit nicht abgenommen.

L02 scheitert am Join der Messaufgaben mit `wire ACK stream closed`.
Die erste encoderfreie Relayreferenz bleibt als Rohdaten erhalten; weder
Bildszenenabschluss noch P27-Leistungsabnahme sind belegt. Die mögliche
Abbruchreihenfolge ist ausdrücklich eine noch zu prüfende Ursache. Der
[Korrekturplan](../plaene/2026-10-24-batches/rv1-compilerplan.md) erhält
sämtliche Profile und Leistungsorakel. Kein lokaler Build-/Testlauf und
kein zusätzlicher Suiteeinstieg werden eingeführt.

Die anschließenden K/L02-Quellkorrekturen sind mit eigener vollständiger
Autorenreview in `00c97f88`, `69f86c46` und `578357a8` integriert. Die
Auswahlkorrektur erhält sämtliche bestehenden Ressourcenfallnamen und
Androidklassen, verschiebt mobile Rustgrenzen zur vorhandenen Linuxstufe und
bindet den tatsächlich vorhandenen Snapshotnachfolger. Die neuen
P27-Abschlussfälle sind als Linuxintegrationen desselben Einstiegs registriert.
Ein statischer Vergleich mit beiden gespeicherten Binärlisten bestätigt die
bestehenden Zuordnungen; neue Abschlussfälle sind dabei ausdrücklich nur durch
ihre Quelldefinitionen belegt. Keine lokale Funktionsausführung und kein
Leistungsnachweis folgen aus diesem Vergleich. P27-Netzprofile, Orakel,
Qualitätsprofile und GL-Ledger bleiben unverändert.

## Lauf 40400605922: Format des P27-Abschlussaufrufs

Der [Lauf](https://github.com/b1ue-man/smart-explorer/actions/runs/40400605922)
für `4a42b936a6be21cf04ee11e786e081282a45949e` ist vollständig beendet.
Ausschließlich die Formatvorbereitung scheitert. Artefakt `12689406223`
hat SHA256 `7c02ee9f95d6bbdafb840f68aca8ab0cd5f653a04ccb024419703fcbd6c2ec79`;
der Formatpatch hat SHA256
`35f142c406b448ef08575485d38bdd879ce2afeb8560681acb1d7cd1c1f6e079`.
Kandidat und beide Hashes sind geprüft. Der Patch bricht nur die Aufrufkette
`finish().await.expect(...)` in der Screenmessfixture um und ist unverändert
übernommen. Der statische AST-Vergleich bestätigt identische Blatttoken.
Lockauflösung unverändert, keine Größenprobleme. Windows-, Linux-, Android-
Build- und Gerätestufen wurden übersprungen. Kein neuer Funktionsnachweis,
kein lokaler Formatter oder Compiler und kein zusätzlicher Suiteeinstieg.

## Lauf 40406246032: Desktop-Funktionsbefunde

[Pipeline](https://github.com/b1ue-man/smart-explorer/actions/runs/40406246032)
für `1ffbf4b0bc90b2479947680abcd0a5941cce00f8`, Statusabruf 2026-10-26
22:10:07 UTC: Vorbereitung erfolgreich; Windows und Linux vollständig beendet,
Funktionsstufen fehlgeschlagen; Android-Checkout noch laufend. Keine Aussage
über einen späteren Androidstatus. Kein Release gestartet.

| Artefakt | ID | SHA256 |
|---|---|---|
| dependencies | 12691462686 | `c43447f59826e1f99bc7d76977f69c357a8144746cf86aa23172b2bf6ae6512a` |
| windows | 12691722778 | `e728f642ab76ba166052db306f3aed4b818dbe56e8db0b17042c01acd7aacf59` |
| linux | 12692376036 | `eea4716b376627e6741333e185e097a17f83acb31a5d2cd4c9ccbbee6f9bf8fc` |

Kandidatenbindung, Archivhash und sichere Archivpfade sind geprüft. Beide
Desktopartefakte bestätigen erfolgreiche Kompilierung der Bibliotheks-
Prüfbinärdatei, `se` und `egui_glow`, ohne Compilerfehler. Die tatsächlichen
fehlgeschlagenen Fallnamen stehen in
[rv1-funktionsbefunde.json](../plaene/2026-10-24-batches/rv1-funktionsbefunde.json),
ihr begrenzter Korrekturplan in
[rv1-funktionsplan.md](../plaene/2026-10-24-batches/rv1-funktionsplan.md).
Die Extraktion verwendet ausschließlich `FAILED`-Identitäten und deren
zugehörige Fehlerabschnitte. Erwartete Panics erfolgreicher Fälle und
`unverified`-Hardwaremeldungen sind keine Fehleridentitäten.

P27 enthält für `10mbit-one-window` sowohl Referenz- als auch Screen-Rohdaten.
Der nächste Profilaufbau scheitert an `tc` mit
`Exclusivity flag on, cannot modify.`. Damit ist der erste Messabschluss
vorhanden; die vollständige Leistungsabnahme ist noch nicht erreicht.
Keine Grenzwerte werden deshalb geändert. Die bereits fertigen Desktoplogs
werden ausgewertet, ohne während des übrigen CI-Laufs Agenten zu aktivieren.
