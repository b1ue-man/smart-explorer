# Statischer Batchreview: Android-Bildfixtures F2–F6

Stand: 2026-10-26. Review des übergebenen Arbeitsbaumkandidaten innerhalb von
`/tmp/smartexplorer-android-image-fixture-review-scope.json`. Beim Dokumentationskontext-Gate
war `e3e778ea` der letzte Commit; der geprüfte Arbeitsbaum enthält darüber hinaus Änderungen.
Die Fundstellen beziehen sich auf die vor einer möglichen Main-Korrektur gelesenen Dateien.

**Ergebnis: drei funktionelle Fehlerpfade bei Ressourcen- beziehungsweise
Allokationsfehlern.** Ihr Kontrollfluss ist statisch belegt; keiner dieser Fehler wurde
in diesem Review auf einem Gerät ausgelöst. Der Review hat keine Produktdatei verändert
und keine Builds, Compiler, Formatter, Tests, Server, Installationen, Gitmutationen oder
Graphupdates ausgeführt. Er ist der normale F2–F6-Batchreview, kein zusätzlicher
Release-Regressionsreview und keine Bestätigung eines installierten APK.

## Befunde

### A1 — P2: Fehlgeschlagene Kotlin-Lease-Erzeugung lässt native Claims unerreichbar bestehen

**Fundstellen:** `android/app/src/main/java/app/smartexplorer/android/screen/ScreenCaptureBudget.kt:8–11`
und `:47–58`; betroffener F3-Aufrufer
`android/app/src/main/java/app/smartexplorer/android/screen/ScreenEncoder.kt:91–106`.
Die native Besitzerseite steht in `native/src/screen/os/android/capture_arena.rs:15–25,36–38`.

**Auslöser und Ablauf:** `claim()` registriert zuerst den nativen Token. Scheitert
danach die Konstruktion von `ScreenCaptureLease`, dessen `State` oder `AtomicBoolean`
mit `OutOfMemoryError`, erreicht kein Kotlin-Besitzer den Aufrufer und der Token wird
nicht freigegeben. `retain()` hat denselben Übergabefehler für Aliase: Es erhöht zuerst
`state.references` und konstruiert erst danach den Alias. Scheitert dessen Konstruktion,
bleibt eine Referenz ohne Besitzer zurück. `ScreenEncoder.retainSurface()` schützt
diesen Aufruf nicht: `surfaceLease.retain()` liegt vor seinem `try`-Block.

**Folge:** Der native Arena-Eintrag hält den `CaptureSlice` weiter. Beim fehlerhaften
Alias erreicht selbst das Schließen sämtlicher tatsächlich existierender Besitzer
den Zählerstand null nicht mehr. Solange der Prozess weiterläuft, bleiben diese Bytes
belegt und können spätere Capture-Zulassungen verhindern. Ein Prozessende beendet
diese Lebensdauer; der Befund behauptet keinen über einen Prozessneustart bestehenden
Leak. Ein unbehandelter `OutOfMemoryError` kann den Prozess bereits beenden.

**Primärbeleg:** Die Android-API beschreibt `OutOfMemoryError` beim fehlgeschlagenen
Anlegen eines Objekts. Die zum Java-17-Projektziel passende JLS §15.9.4 beschreibt den
Abbruch einer Objektkonstruktion bei fehlendem Speicher. Damit ist der Übergang
zwischen nativer Registrierung beziehungsweise Zähleränderung und erfolgreicher
Kotlin-Besitzerkonstruktion tatsächlich fehlbar.
([Android OutOfMemoryError](https://developer.android.com/reference/java/lang/OutOfMemoryError),
[JLS 17, §15.9.4](https://docs.oracle.com/javase/specs/jls/se17/html/jls-15.html#jls-15.9.4);
abgerufen 2026-10-26.) Die Arena-Lebensdauer ergibt sich unmittelbar aus dem genannten
Produktcode; `release(token)` entfernt den Eintrag, der Kanalabschluss tut das nicht.

**Korrekturgrenze für Main:** Besitzererzeugung und Claim-Übergabe müssen gemeinsam
zurückgerollt werden. Bei `retain()` darf erst ein erfolgreich erzeugter Alias als
zusätzliche Referenz zählen. Das bereits vorhandene Rollback von `tryBuffer()`
(`ScreenCaptureBudget.kt:13–20`) ist zu erhalten. Ein pauschales Leeren der Arena beim
Kanalabschluss wäre falsch, weil aktive Worker und direkte Puffer weiterhin Besitzer
sein können.

### A2 — P2: Ein fehlgeschlagener Encoderthread-Start hat keinen Ressourcen-Rollback

**Fundstellen:** `android/app/src/main/java/app/smartexplorer/android/screen/ScreenEncoder.kt:71–88`,
insbesondere `:81–82`. Produktiver Aufrufer:
`android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostControl.kt:309–319`.
Fixture-Aufrufer: `android/app/src/androidTest/java/app/smartexplorer/android/task/ScreenTaskTest.kt:241`
und `:541`.

**Auslöser und Ablauf:** `codec.start()` war erfolgreich; danach scheitert die
Erzeugung oder der Start des `screen-encoder`-Threads. Der Konstruktor besitzt zu
diesem Zeitpunkt Codec, gegebenenfalls Input-Surface und Surface-Claim. Der Aufruf
`thread(...)` ist nicht von einem Rollback umgeben. Die vorhandene Freigabe steht
im `finally` des Threadblocks und wird bei nicht gestartetem Thread nicht ausgeführt.
Da der Konstruktor keine Instanz zurückgibt, kann auch der Aufrufer nicht
`encoder.release()` aufrufen. Der Catch um `codec.start()` endet bereits davor.

**Folge:** Die Anwendung führt keine deterministische Freigabe dieser bereits
erworbenen Ressourcen aus. Der native Claim bleibt ohne erreichbaren Kotlin-Besitzer
bestehen, falls der Prozess den Startfehler überlebt. Eine mögliche spätere
plattformseitige Bereinigung eines unerreichbaren Codecobjekts löst den eigenen
Arena-Token nicht. Der Befund behauptet weder einen beobachteten Threadmangel noch
einen dauerhaft lebenden Codec nach Prozessende.

**Primärbeleg:** `kotlin.concurrent.thread` startet bei seinem seit Kotlin 1.0
vorhandenen Standard `start=true` sofort. Im versionsgebundenen AOSP-Stand
`android-16.0.0_r1`, `runtime/thread.cc:911–958`, führt ein fehlgeschlagenes
`JNIEnvExt::Create` oder `pthread_create` zu `ThrowOutOfMemoryError`; der neue Thread
hat dann seinen Block nicht übernommen. Threadressourcen können damit ausfallen,
obwohl der zuvor gestartete Codec bereits existiert.
([Kotlin thread](https://kotlinlang.org/api/core/kotlin-stdlib/kotlin.concurrent/thread.html),
[AOSP Android 16, Thread::CreateNativeThread](https://android.googlesource.com/platform/art/+/refs/tags/android-16.0.0_r1/runtime/thread.cc#911);
abgerufen 2026-10-26. Die AOSP-Datei wurde nach fehlgeschlagenem Browserabruf über
Googlesources `?format=TEXT` gelesen.) Android 16 entspricht dem konfigurierten
`compileSdk`/`targetSdk` 36; dies behauptet keine konkrete Geräteversion.

**Korrekturgrenze für Main:** Die Konstruktion braucht einen durchgehenden Besitzer
bis zur erfolgreichen Übergabe an den gestarteten Drain-Thread, einschließlich
Rollback für `Throwable`. Dabei müssen auch die jetzt vor beziehungsweise zwischen
den Teil-Catches liegenden Konstruktionen nach dem Surface-Claim berücksichtigt
werden (`surfaceOwners`, `surfaceClosers`, `pending`, `EncoderAdmission`, `AccessUnits`).
Ein nie gestarteter Worker benötigt keinen Join; ein tatsächlich gestarteter Worker
darf seine Ressourcen weiterhin erst nach seinem wirklichen Ende verlieren.

### A3 — P2: Die Synthetic-Abschlussmeldung kann an einer neuen Allokation scheitern

**Fundstellen:** `android/app/src/main/java/app/smartexplorer/android/screen/ScreenEncoder.kt:98–103`,
`:122–125` und `:143–168`; Workerabschluss in
`android/app/src/main/java/app/smartexplorer/android/screen/SyntheticSurfaceSource.kt:30–31`.

**Auslöser und Ablauf:** Der Synthetic-Worker erreicht sein `finally` und schließt
den von `retainSurface()` erzeugten Alias. Weil der Abschluss auf dem Worker und
nicht auf dem Erzeugerthread stattfindet, wird zuerst `surfaceClosers.add(endedOn)`
ausgeführt. Die Liste wurde mit `ArrayList<Thread>()` angelegt. Scheitert die hierbei
benötigte Vergrößerung ihres Arrays, werden `surfaceUsers--` und `notifyAll()` nicht
mehr ausgeführt. Der Alias ist bereits geschlossen; ein wiederholtes `close()`
kann diese Abschlussmeldung nicht nachholen.

**Folge:** Obwohl der Zeichenloop beendet ist, bleibt `surfaceUsers` positiv.
Bei weiterlaufendem Prozess wartet `finishResources()` weiterhin auf diesen
Zähler und `release()` verweigert die Freigabe. Codec, Surface und ursprünglicher
Surface-Claim werden deshalb nicht regulär beendet. Ein unbehandelter Fehler auf
dem Worker kann stattdessen den Prozess beenden; ein auf einem Gerät beobachteter
Hänger wird hier ausdrücklich nicht behauptet.

**Primärbeleg:** AOSP `android-16.0.0_r1`,
`ojluni/src/main/java/java/util/ArrayList.java:175–176,238–246,488–503`, verwendet im
leeren Standardkonstruktor ein gemeinsames leeres Array und allokiert beim ersten
`add` ein neues Array. Die Android-API dokumentiert außerdem das automatische
Wachstum. Die Allokation liegt damit gerade im Abschluss des ersten Workers und
ist keine erst nach beliebig vielen Sitzungen angenommene Möglichkeit.
([AOSP Android 16, ArrayList](https://android.googlesource.com/platform/libcore/+/refs/tags/android-16.0.0_r1/ojluni/src/main/java/java/util/ArrayList.java#238),
[Android ArrayList](https://developer.android.com/reference/java/util/ArrayList),
[Android OutOfMemoryError](https://developer.android.com/reference/java/lang/OutOfMemoryError);
abgerufen 2026-10-26. AOSP ebenfalls über `?format=TEXT` gelesen.)

**Korrekturgrenze für Main:** Die Registrierung des später zu joinenden Workers
muss vor der Übergabe des Besitzes fehlbar erledigt sein; der tatsächliche
Workerabschluss darf dafür keinen neuen Speicher benötigen. Nur das Dekrement
in einen weiteren `finally`-Block zu verschieben reicht nicht, wenn dadurch der
Nachweis des echten Threadendes verloren geht. Auch die nachträgliche
`surfaceClosers.toList()`-Allokation vor der Ressourcenfreigabe gehört zu dieser
Abschlussgrenze.

## Geprüfte Abgrenzungen und Entscheidungen

- Für die F2-Capture-Fixture, den nativen AU-Pfad und den festen Pull-Puffer wurde
  kein zusätzlicher funktioneller Fehler nachgewiesen. Die bisherige Bindung an
  die produktive Sessionzulassung, das Halten alter Besitzer über Kanalabschluss
  und Resize sowie die Freigabe erst nach dem letzten tatsächlichen Nutzer sind
  zu erhalten. A1 betrifft die Kotlin-Übergabe, keine Erlaubnis für ein vorzeitiges
  natives Sweep.
- Für F6 wurde kein zusätzlicher funktioneller Fehler der geprüften
  `ViewerFrame`→JNI→NDK→Bitmap→HWUI-Besitzkette nachgewiesen. Ein erfolgreicher
  NDK-Pixellock wird wieder entsperrt; der Renderer-Borrow überlebt bis zur
  Rendererbeendigung. Die NDK-Dokumentation verlangt das passende Unlock;
  AOSP Android 16 bestätigt synchrone Renderthread-Aufrufe für `destroy()` und
  `destroyHardwareResources()`. Das ist ein Quellenbeleg für die geprüfte
  Lebensdauer, kein Nachweis sämtlicher OEM-Implementierungen.
  ([NDK Bitmap](https://developer.android.com/ndk/reference/group/bitmap),
  [AOSP Android 16, RenderProxy](https://raw.githubusercontent.com/aosp-mirror/platform_frameworks_base/android-16.0.0_r1/libs/hwui/renderthread/RenderProxy.cpp),
  [HardwareRenderer](https://developer.android.com/reference/android/graphics/HardwareRenderer);
  abgerufen 2026-10-26.)
- Der Verdacht, die Fixture-JNI-Aufrufe würden durch das Verwerfen von
  `Outcome` Java-Ausnahmen löschen, wurde verworfen. Die tatsächlich vorhandenen
  Quellen von `jni` 0.22.4 zeigen in `env.rs:4738,4793` den Rückgabewert von
  `into_outcome` und die Behandlung durch `with_env`; für die ausdrücklich mit
  `throw_new` gesetzten Java-Ausnahmen wurde kein Löschpfad gefunden. Aus dem
  bloßen Verwerfen des Rust-Ergebnisses wird deshalb kein Befund abgeleitet.
- Für die Main-Anschlüsse F1/F7 wurden die Featureweitergabe Root-Crate→Android-Bridge, die
  Android-/Feature-`cfg`-Grenzen, der Release-`compile_error!`, der
  Debug-Kotlin-Quellpfad, getrennte JNI-Verzeichnisse sowie die DEX-/ELF-Prüfung
  gelesen. Kein zusätzlicher Fehler wurde dabei nachgewiesen. Cargo aktiviert
  optionale Features nicht allein durch ihre Definition; Android-Quellsätze
  werden pro Variante kombiniert, weshalb die explizite Trennung der
  `jniLibs`-Verzeichnisse relevant ist.
  ([Cargo Features](https://doc.rust-lang.org/cargo/reference/features.html),
  [Android Build Variants](https://developer.android.com/build/build-variants);
  abgerufen 2026-10-26.) Das Ergebnis bestätigt keine tatsächlich gebauten DEX-
  oder ELF-Dateien.
- Main meldete zusätzlich eine vollständige Aufrufersuche nach `jniLibs`,
  `assembleDebug`, `assemble*AndroidTest` und `installDebug` in den Shell-,
  PowerShell-, YAML- und Gradle-Kotlin-Dateien unter `android`, `native` und
  `.github`: genau die zwei angepassten Debugfunktionen in
  `android/test-android-task.sh`; Release-JNI schreibt
  `android/build-release-apk.sh`. Diese repoübergreifende Suche ist Main-Evidenz,
  keine von diesem Review selbst ausgeführte Suche. Aus dem zuvor offenen
  Aufruferverdacht wird kein Fehler konstruiert.

## Umfang, Planung und tatsächliche Änderungen

Der Erstplan aus dem Auftrag war die statische Prüfung der zusammenhängenden
Capture-/Login-Besitzübergaben und ihrer APK-Abschottung. Der Zweitplan nach der
Codelektüre konzentrierte sich auf Konstruktor-Rollback, Workerabschluss,
JNI-/NDK-Bitmapbesitz und Feature-/Quellsatzgrenzen. Die zweite Quellenprüfung
klärte ausdrücklich Java-/ART-Allokationsfehler, `ArrayList`-Wachstum und die
tatsächliche `jni`-0.22.4-Semantik. Akzeptanzsignal war ein Bericht mit
nachvollziehbaren Defekten und getrennten Laufzeitlücken. Der Graph wurde nur
mit einer Capture-/Login-bezogenen `graphify query` navigiert; er lieferte keine
Kotlin-Lebensdauerbelege und wurde nicht geändert.

Erstellt wurde ausschließlich
`docs/plaene/2026-10-24-batches/review-android-bildfixtures.md`.
Produktverhalten wurde durch diesen Review nicht geändert. A1, A2 und A3 wurden
Main zur Bearbeitung gemeldet. Es wurden keine zusätzlichen Suite- oder
Release-Gates angelegt; Integration, Korrekturen, Registrierung in der bestehenden
Task-Suite und die eine terminale Remote-Release bleiben bei Main.

### Vollständig gelesene Dateien

Die folgenden Pfade sind jeweils vollständig gelesene Dateien; die Verzeichnisse
selbst wurden nicht als pauschale Lesefläche behandelt.

```text
/tmp/smartexplorer-android-image-fixture-review-scope.json
android/app/src/main/java/app/smartexplorer/android/screen/ScreenEncoder.kt
android/app/src/main/java/app/smartexplorer/android/screen/SyntheticSurfaceSource.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenSyntheticRetirement.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenCaptureBudget.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenCaptureBuffer.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenProjectionFrames.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenStillSource.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenControlService.kt
android/app/src/main/java/app/smartexplorer/android/screen/ScreenHostControl.kt
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginBitmapOwner.kt
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginBitmapRenderer.kt
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginBitmapSurface.kt
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginInput.kt
android/app/src/main/java/app/smartexplorer/android/ui/login/LoginSession.kt
android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenPullBuffers.kt
android/app/src/main/java/app/smartexplorer/android/ui/screen/ScreenDecoder.kt
android/app/src/androidTest/java/app/smartexplorer/android/task/BatchesCaptureTaskTest.kt
android/app/src/androidTest/java/app/smartexplorer/android/task/ScreenCaptureFixture.kt
android/app/src/androidTest/java/app/smartexplorer/android/task/ScreenCaptureQueue.kt
android/app/src/androidTest/java/app/smartexplorer/android/task/LoginImageTaskTest.kt
android/app/src/androidTest/java/app/smartexplorer/android/task/ScreenTaskTest.kt
android/app/src/debug/java/app/smartexplorer/android/core/AndroidImageFixtureBridge.kt
android/app/src/test/java/app/smartexplorer/android/screen/ScreenCaptureBudgetTest.kt
android/check-image-fixture-package.py
native/android-bridge/Cargo.toml
native/android-bridge/src/android_image_fixture_jni.rs
native/android-bridge/src/login_bitmap_jni.rs
native/android-bridge/src/login_bitmap_owner.rs
native/android-bridge/src/login_bitmap_pixels.rs
native/android-bridge/src/screen_capture_jni.rs
native/android-bridge/src/screen_jni.rs
native/src/mobile/os/shared/domains/login_image_fixture.rs
native/src/mobile/os/shared/domains/android_image_fixtures.rs
native/src/mobile/os/shared/domains/login_images.rs
native/src/mobile/os/shared/domains/login_view.rs
native/src/mobile/os/shared/domains/screen_host.rs
native/src/screen/os/android/capture_fixture.rs
native/src/screen/os/android/capture_admission.rs
native/src/screen/os/android/capture_arena.rs
```

### Abschnittsweise gelesene oder gezielt durchsuchte Dateien

| Datei | Gelesener Bezug |
| --- | --- |
| `docs/ARCHITEKTUR.md` | Architektur-/Einstiegsübersicht |
| `docs/plaene/2026-10-24-batches/android-bildfixtures.md` | F2–F6, Besitzkette, Feature-/APK-Anforderungen |
| `docs/plaene/2026-10-24-batches/review-login-bildspeicher.md` | bestehende Login-Befunde und Quellenabgrenzung |
| `/root/.codex/skills/graphify/SKILL.md` | Graphnutzung; keine Installation und kein Rebuild |
| `graphify-out/graph.json` | über die Capture-/Login-bezogene Query ausgegebener Teilgraph |
| `android/app/build.gradle.kts` | SDK-/Java-Ziele, Varianten, Quellsätze und JNI-Verzeichnisse |
| `android/build-release-apk.sh` | APK-Bau-/Prüfpfad und Release-Featureabschottung |
| `android/test-android-task.sh` | beide Debug-Baufunktionen, Fixturefeature, APK-Prüfung |
| `android/app/src/main/java/app/smartexplorer/android/core/Core.kt` | JSON-Konfiguration |
| `android/app/src/androidTest/java/app/smartexplorer/android/task/TaskSupport.kt` | Instrumentation-Kontext, Core-Aufrufe, Bericht- und Fixturehilfen |
| `native/Cargo.toml` | Featuredefinitionen |
| `native/src/lib.rs` | Release-Featureabwehr |
| `native/android-bridge/src/lib.rs` | Fixture-JNI-Modulgrenze |
| `native/src/mobile/mod.rs` | Android-Fixtureexporte |
| `native/src/mobile/os/shared/domains/mod.rs` | Fixturemodule und Exporte |
| `native/src/screen/os/mod.rs` | Android-Fixtureexporte |
| `native/src/screen/os/android/bridge_channels.rs` | Kanal-/Queue-/Generationsbesitz bis Zeile 480 |
| `native/review-task-device.py` | APK-Namen und Android-Instrumentationseinbindung |
| `native/review-task-manifest.json` | Capture-/Login-/F2–F6-Registrierungen |
| `.github/workflows/review-task.yml` | gezielte Suche nach Android-/Fixtureeinbindung |
| `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/jni-0.22.4/src/env.rs` | Zeilen 4520–4850, insbesondere `EnvUnowned`, `with_env`, `into_outcome` |
| `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/jni-0.22.4/src/refs/global.rs` | Zeilen 1–170, globale JNI-Referenzen |

Die im Nutzerkontext gelieferten AGENTS-Anweisungen und die zentralen
Arbeitsweise-Anweisungen wurden berücksichtigt. Die genannten externen
Primärquellen wurden an den jeweiligen API-/Implementierungsstellen gelesen.
Unvollständig ausgegebene Großdateien sind deshalb ausdrücklich in der
Abschnittsliste und nicht als vollständige Dateilektüre geführt.

## Offene Fragen und fehlende Laufzeitbelege

- Für A1–A3 fehlen beobachtete Fehlerauslösungen auf Android. Die Defekte beruhen
  auf den angegebenen fehlbaren Operationen und den unmittelbar folgenden
  nicht ausgeführten Besitzübergaben beziehungsweise Freigaben.
- Dieser Review besitzt keinen Beleg eines erfolgreichen Kotlin-/Rust-Builds,
  der tatsächlich exportierten Symbole eines neuen JNI-Artefakts, des Inhalts
  eines neu gebauten Debug-/Release-APK oder eines erfolgreichen Gerätelaufs
  der F2–F6-Fälle. Diese Aussagen bleiben offen, nicht bestanden.
- Das Verhalten konkreter Hersteller-Codecs, ein tatsächlich blockierendes
  `lockHardwareCanvas`, HWUI-Retirement auf der konkreten Geräteversion und
  sichtbare Barcode-/Snapshot-Ausgaben sind durch Quellenlektüre allein nicht
  beobachtet. Diese Beleggrenzen sind keine zusätzlichen Release-Voraussetzungen.
- Dieser Bericht bestätigt keine nach der jeweiligen Fundstellenlektüre von
  Main eingebrachte Behebung. Eine solche Korrektur benötigt ihre eigene
  Integrationsbewertung durch Main innerhalb des laufenden Batches.
