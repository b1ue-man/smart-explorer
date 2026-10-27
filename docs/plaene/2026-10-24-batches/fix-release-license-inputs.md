# R01: Rust-Lizenztexte vor vollständigen Releasebauten

Stand: 2026-10-27. Fixautor: derselbe Codex-Agent für Fehleraufnahme,
Erhaltungsplan, Quelländerung und anschließende statische Anti-Regressionsreview.
Quellbasis: `2081be2e`. Kein lokaler Build, keine lokale Lizenzgenerierung,
keine Suite und keine zusätzliche Releasepipeline.

## Auftrag und Erstplan aus dem Auftrag

Der delegierte R01-Auftrag vom 2026-10-27 verlangt aktuelle vollständige
Rust-Lizenztexte vor neuen Android- und Desktopbauten und deren Aufnahme in den
begrenzten Releasecommit. Der Nutzer verlangt den vollständigen Release ohne
erneuten funktionalen Testlauf. Main besitzt Commit, Push und Remoteausführung.
Die beabsichtigte Version bleibt laut übergebenem Remotezustand 0.5.185; dieser
Bericht behauptet keinen fertigen Release.

Aus dem Auftrag bekannt: Lauf `40425126500` scheiterte im vollständigen Wrapper
nach erfolgreichem Androidjob, vor Desktopbau und Versionswechsel. Die lokale
Korrektur darf ausschließlich die Releasevorbereitung und ihre Anleitung ändern.
Zu klären waren Generatorfehlercodes, Androidmodi, WSL-Toolgrenze, Kandidaten-
wiederverwendung, Source-SHA-Bindung und begrenzte Git-Recovery. Für diese Fragen
ist keine Nutzerangabe erforderlich.

## Fehlerfunktion und belegter Bestand

Die fehlerhafte Fähigkeit ist das Erstellen eines vollständigen installierbaren
Releases aus dem freigegebenen Quellkandidaten, dessen Rust-Abhängigkeiten sich
seit der letzten Generierung geändert haben. `Assert-RustCrateLicenses` prüft
bislang ausschließlich `Check` gegen den eingecheckten Text. Es fehlt die
Herstellung dieser generierten Build-Eingabe im Releaseweg. Der Androidvorbau
bettet denselben Text schon vorher ohne Aktualisierung ein.

Der unveränderte Remote-Logabschnitt in
`/tmp/smartexplorer-release-40425126500-license-failure.log` belegt am
2026-10-27 um 00:55:27 UTC: Die aktuelle Generierung liefert 783 Crates,
329 Lizenztexte und drei NOTICE-Dateien. Eingecheckt waren 767 Crates,
319 Texte und drei NOTICE-Dateien. Der Check scheitert wegen Textabweichung;
die Generierung meldet keinen unaufgelösten Textfehler. Die Warnung zu
`LicenseRef-UFL-1.0` wird im vorhandenen Generator durch die unveränderte
Ergänzung aus der tatsächlichen Crate-Datei behandelt. Es gibt keinen Beleg für
einen Generatorfehler, der eine Ersatzerzeugung rechtfertigen würde.

## Direkte und indirekte Verknüpfungen

| Station | Vollständige für R01 betroffene Funktion und Erhaltungsgrenze |
| --- | --- |
| `.github/workflows/build.yml`, Jobs `android-release-apk`, `complete-release` und Exact-Byte-Publikationsguard | Beide Baujobs checken denselben vollständigen `main`-SHA aus, prüfen Input/Event/HEAD/Remote-`main`, verwenden dieselbe Versionsplanung und reichen die signierte APK mit Metadaten weiter. Bestehende Releasekandidaten gehen durch APK-Verify. Die Publikationsstufe führt eine zusätzliche explizite Bash-Dateiliste `allowed_release_change` und prüft den einzigen Quellparent. |
| `native/release-version.ps1` | `Resolve-ReleasePlan` und `Set-NativeVersion` trennen Tagged/Bump/Resume, erhöhen höchstens um die nächste Patchversion und ändern ausschließlich die Rootversion. Diese Logik bleibt bestehen. |
| `android/build-release-apk.sh` | Build führt Cargo-NDK und Gradle aus; Verify prüft vorhandene APK-Bytes; Check-env prüft das bisherige Androidwerkzeugset. Nur Build darf die neue Lizenzvorbereitung erreichen. |
| `native/publish-release-local.ps1` | Preflight und Lock, neue Builds, wiederverwendbare vollständige Kandidaten, getaggte Kandidaten, Windowsdiagnose und begrenzte Arbeitskopie-Recovery müssen getrennt bleiben. `New-CompleteReleaseAndroidApk` verbindet den menschlichen Windows-/Linux-Wrapper mit Bash/WSL. |
| `native/rust-licenses.ps1`, `third-party/rust-crates/about.toml` | Der vorhandene feste Generator cargo-about 0.9.2 verarbeitet die drei versendeten Manifeste mit Workspace, Lockdateien und fünf Zielplattformen. Vollständige Texte und NOTICE-Dateien stammen aus den Abhängigkeiten; Generate/Check scheitern weiterhin bei fehlender Abdeckung. |
| `native/Cargo.toml`, `native/Cargo.lock`, `native/explorer-command/Cargo.toml`, `native/explorer-command/Cargo.lock`, `share-server/Cargo.toml`, `share-server/Cargo.lock` | Dies sind die vom Generator benannten Eingabepfade; weder Manifeste noch Lockdateien werden durch diesen Fix geändert. Metadata-Auflösung darf keine stillen Lockänderungen verursachen. |
| `native/build.rs`, `native/src/lib.rs`, `native/src/licenses/mod.rs` | Build-Cfg und `include_str!` binden den kanonischen Text in die gemeinsame Bibliothek; `text`, `crates`, `sha256` und `summary_json` versorgen CLI, Desktop und Android. Der vorhandene Lizenzgraph bestätigt das Modul und diese Funktionen; Produkt-Rust bleibt unverändert. |
| `native/installer.nsi` | Installation und Deinstallation des kanonischen Texts unter `licenses/Rust-Crates` bleiben bestehen. |
| `native/publish-update.ps1`, `native/publish-feed.sh`, `native/publish-linux-feed-wsl.sh` | Windowsmanifest, Feed-Promotion und Quell-HEAD-Bindung bleiben bestehen; die Vorbereitung muss vor diesen Neubauten abgeschlossen sein. |
| `native/release-publication.ps1` | Die explizite Commitliste wird für Status, begrenztes Unstaging, Staging, Kandidaten-Parentprüfung und Source-SHA-Ermittlung wiederverwendet. Nur der eine kanonische generierte Lizenzpfad kommt hinzu. APK-Metadatenschema, Signerprüfung und Payloadhashes bleiben bestehen. |
| `native/release-lock.ps1` | Der vollständige Wrapper hält weiterhin denselben Cross-Host-Lock. Kein Lock wird im Fix gelöscht oder umgangen. |
| `.github/workflows/review-task.yml`, `README.md`, `docs/RELEASING.md` | Der aktuelle Reviewworkflow enthält keinen behaupteten Lizenz-prepare-Job. Die kanonischen Anleitungen müssen den wirklichen Releaseweg beschreiben; kein Suitelauf wird ergänzt. |

## Stufe eins: begründete Lösungswahl

Der vorhandene Generator ist wiederzuverwenden. Die Remoteevidenz belegt seine
vollständige Erzeugung; `Check` allein kann einen veralteten Text nicht reparieren.
Manuelle Texte, unterdrückte Fehler, Abhängigkeitsänderungen und eine zusätzliche
Pipeline sind keine Bestandteile dieser Korrektur. Neue Builds erhalten deshalb
Generate gefolgt von Check; wiederverwendete und getaggte Kandidaten erhalten
weiterhin ausschließlich Check.

Die Generatorimplementierung verwendet ordinales Sortieren, LF/UTF-8 ohne BOM
und ignoriert im Aktualitätsvergleich ausschließlich Zeilenenden und das
generierte Datum. Die Konfiguration und alle drei Manifestpfade bleiben erhalten.
Die bereits dokumentierte cargo-about-Quelle und die tatsächlich erfolgreiche
Remotegenerierung sind die Grundlage; dieser Fix führt keine neue Generator-API
oder Lizenzbewertung ein.

## Stufe zwei: konkreter Erhaltungs- und Meilensteinplan vor Quelländerung

1. `Assert-RustCrateLicenses` erhält eine ausdrücklich angeforderte Vorbereitung:
   Generate muss erfolgreich sein, danach muss Check erfolgreich sein. Die
   Vorbereitung kommt nur in den tatsächlichen Neubauzweig unter dem
   vollständigen Releaselock beziehungsweise in die Windowsdiagnose vor deren
   Neubau. Check-env, vollständige vorhandene Kandidaten und Tagged bleiben
   schreibfreie Lizenzchecks. Erwartung: kein neuer Payloadbau erreicht einen
   veralteten oder unvollständigen Text; vorhandene Artefakte werden nicht erzeugt.
2. Der direkte Android-Build führt dieselbe Generate/Check-Folge aus. Beim
   Wrapperaufruf stammen Texte bereits aus dessen erfolgreicher Vorbereitung.
   Der Wrapper übergibt intern ausschließlich deren SHA-256; Bash prüft die
   identischen Checkoutbytes. WSL braucht dafür kein neu vorausgesetztes pwsh.
   Der eigenständige Android-Build ohne vorbereiteten Hash benötigt pwsh.
   Verify und Check-env erreichen keinen dieser Vorbereitungsschritte.
   Erwartung: APK-Bau bindet den vorbereiteten Text; APK-Version, Signer,
   Hashermittlung, Bibliotheksprüfung und das Vierfeld-Metadatenschema bleiben
   unverändert. Der Hash beweist Byteidentität, nicht selbst Aktualität; der
   Aktualitätsbeleg ist die unmittelbar vorgelagerte erfolgreiche Wrapperprüfung.
3. Nur `third-party/rust-crates/THIRD-PARTY-LICENSES.txt` wird als zusätzliche
   Releaseänderung erlaubt und in die gemeinsame explizite Commitliste
   aufgenommen. Die abschließende Statusprüfung umfasst diesen exakten Pfad.
   Dieselbe exakte Ergänzung muss auch im unabhängigen
   `allowed_release_change`-Guard der bestehenden Workflowpublikation stehen.
   Metadata-Aufrufe des Generators verwenden ebenfalls `--locked`.
   Erwartung: Source-SHA und Kandidatenparent dürfen weiterhin nur durch die
   bekannte begrenzte Veröffentlichungskette akzeptiert werden; keine breite
   `third-party`-Freigabe und kein beliebiges Git-Unstaging entstehen.
4. Generatorhinweise und die beiden kanonischen Anleitungen werden angepasst.
   Danach prüft derselbe Autor den gesamten eigenen Diff statisch gegen alle
   Tabellenzeilen und alle Erwartungen. Es wird kein Skript ausgeführt und
   keine funktionale Suite begonnen. Die abschließende Laufzeitabnahme gehört
   ausschließlich zum bereits autorisierten vollständigen Remote-Release.

Zweiter Lückenpass vor Quelländerung: Der Windowswrapper hat pwsh und prüft auf
Windows; sein Androidbau läuft in WSL, dessen bisheriger Vertrag kein pwsh
enthält. Deshalb wird ausschließlich der nach Generate/Check berechnete interne
Hash über den bestehenden WSLENV-Weg mitgeführt und nach dem Aufruf
zurückgesetzt. Der unabhängige CI-Androidjob hat laut delegiertem Umgebungsbeleg
pwsh auf Ubuntu. Das APK-Übergabeverzeichnis bleibt bei exakt APK und Metadaten;
es wird kein neues Metadatenfeld eingeführt. Ein bereits vollständiger Kandidat
wird vor jeder möglichen Generierung erkannt und nur geprüft.

Integrationsbefund vor der zugehörigen Workflowänderung: Main wies auf die
unabhängige Liste `allowed_release_change` in `build.yml` hin. Die anschließende
eigene Quelllektüre bestätigt den Befund: Ohne genau den kanonischen Lizenzpfad
würde die vorhandene Publikationsstufe den neuen Releasecommit nach dem Tag
als Quelländerung ablehnen. Die Karte und Planpunkt drei sind deshalb ergänzt;
die Korrektur benötigt eine Zeile im bestehenden Guard, keine neue Pipeline.

## Umsetzung und eigene Anti-Regressionsreview

Die Umsetzung umfasst `native/publish-release-local.ps1`,
`native/release-publication.ps1`, `native/rust-licenses.ps1`,
`android/build-release-apk.sh` und genau eine zusätzliche Dateizeile im
bestehenden Publikationsguard von `.github/workflows/build.yml`. README und
Releaseanleitung beschreiben jetzt diese tatsächliche Vorbereitung. Die
unbelegte Behauptung eines Suite-prepare-Jobs ist entfernt.

Die eigene anschließende Review wurde am fertigen Diff und an den aufgelisteten
Aufrufern durchgeführt. Der Hinweis des Main zum unabhängigen Workflowguard
wurde vor Abschluss selbst anhand dessen Bash-Code geprüft und behoben.

| Prüffrage | Konkreter Erhaltungsbeleg am fertigen Quellstand |
| --- | --- |
| Neuer vollständiger Desktopbau | `Assert-RustCrateLicenses -Prepare` liegt im fehlenden-Kandidaten-Zweig nach `Enter-CompleteReleaseLock`, vor `Invoke-LinuxCompleteReleaseBuild` beziehungsweise `Invoke-WindowsReleaseBuild`. Generate-Fehler führen unmittelbar zum Abbruch; der anschließende Check bleibt zwingend. |
| Neuer Androidbau in CI oder direktem Bashaufruf | Die neue Vorbereitung liegt hinter den vollständigen Check-env-/Verify-Zweigen und vor `build_apk`. Ohne internen vorbereiteten Hash muss pwsh Generate und Check erfolgreich ausführen. `set -Eeuo pipefail` verhindert nach einem Fehler den Cargo-NDK-/Gradle-Aufruf. |
| Windowswrapper → WSL und Linuxwrapper → Bash | Der Wrapper initialisiert seinen vorbereiteten Hash leer, berechnet ihn ausschließlich nach erfolgreichem Generate und Check aus der kanonischen Datei und setzt ihn nur um seinen tatsächlichen Android-Bauaufruf. `finally` stellt die vorige Umgebung wieder her; WSLENV übergibt den Hash mit `/u`. Bash verlangt einen gültigen Hash und die identischen nichtleeren Bytes. WSL braucht dafür kein pwsh. |
| Grenze des internen Hashbelegs | Der Hash beweist ausschließlich Byteidentität. Annahme für diese interne Übergabe: Der Aufrufer ist der kanonische Wrapper, der unmittelbar zuvor Generate und Check ausgeführt hat. Ein manuell gesetzter Umgebungswert wäre kein eigenständiger Aktualitätsbeleg. Der CI-Androidjob setzt diesen Wert nicht und verwendet deshalb immer den Generator. Es wird kein neuer öffentlicher Modus eingeführt. |
| Android Verify und Check-env | Beide vorhandenen Zweige verlassen das Skript vor `prepare_rust_licenses`; ihre bisherigen Toolprüfungen sind unverändert. Sie erzeugen keine Lizenztexte und verändern keine vorhandene APK. |
| Wrapper Check-env und Windowsdiagnose | `CheckEnvOnly` ruft ausschließlich den Check auf. Nur die bauende Windowsdiagnose verwendet Prepare; sie bleibt ohne Versionsbump, Commit, Push oder Tag. |
| Vollständiger vorhandener Kandidat | Der Wrapper prüft seine Verfügbarkeit vor Prepare. Der Wiederverwendungszweig ruft ausschließlich den Lizenzcheck und die bestehende APK-Byteprüfung auf. Ein fehlender oder veralteter Lizenztext wird nicht heimlich erneuert. |
| Getaggter Kandidat | `Tagged` erreicht keinen Prepare-Aufruf und keinen Neubau. Die vollständige Kandidatenprüfung und der Vergleich des unveränderlichen Remote-Tags mit dem Kandidaten bleiben bestehen. |
| Versionsplanung und Lockdateien | `native/release-version.ps1` ist unverändert; dieselbe Tagged/Bump/Resume-Planung gilt in beiden Jobs. Der Generator verwendet für seine Metadata-Auflösung jetzt ebenfalls `--locked`; die drei Manifeste, drei Lockdateien, Cargo-about-Pin und Lizenzkonfiguration bleiben unverändert. |
| Source-SHA und Kandidatenparent | Die Workflowbindungen Input/Event/HEAD/Remote-main, Wrapperkontextprüfung, Windowsmanifest und Ein-Parent-Prüfung bleiben bestehen. Die gemeinsame PowerShell-Dateiliste erkennt den generierten Text als begrenzten Releaseinhalt. Die unabhängige Workflow-Bashliste erlaubt denselben exakten Satz; sämtliche übrigen Pfade werden weiterhin abgewiesen. |
| Begrenzte Git-Recovery | `Test-ReleaseMutablePath` erlaubt zusätzlich nur den einen kanonischen Text. Die explizite Commitliste steuert weiterhin Status, ausschließlich begrenztes Unstaging, Staging und Parentvalidierung. Die abschließende Statusabfrage umfasst jetzt denselben Lizenzpfad. Kein Verzeichnispräfix wurde hinzugefügt und keine Lockdatei entfernt. |
| APK-Signaturen, Hashes und Übergabe | `Get-PublicationAndroidReleaseApk`, `verify_apk`, `publish_verified`, APK-Sidecars und Feedpromotion sind unverändert. Die Metadaten enthalten weiterhin genau `version`, `versionCode`, `sha256`, `certSha256`; das Übergabeverzeichnis enthält weiter genau APK und Metadaten. |
| Lizenzanzeige und Installation | `native/build.rs`, `native/src/lib.rs`, `native/src/licenses/mod.rs` und `native/installer.nsi` sind unverändert. Sie konsumieren weiterhin denselben kanonischen Text; vollständige aktuelle Texte werden vor dessen Einbetten hergestellt. Keine Produktfunktion, Oberfläche oder gespeicherte Einstellung ist geändert. |

Statische Belege: PowerShells `Parser.ParseFile` akzeptiert alle drei geänderten
PowerShelldateien ohne Parsefehler. `bash -n` akzeptiert den Android-Einstieg.
Ein reiner YAML-/Textparser liest den vorhandenen Publikationsschritt, dessen
Bash-Inhalt ebenfalls `bash -n` akzeptiert, und gleicht seine gesamte Dateiliste
mit `Get-PublicationReleaseCommitPaths` ab: beide Mengen stimmen exakt überein;
unter `third-party` ist ausschließlich der kanonische Text erlaubt.
`git diff --check` meldet keinen Whitespacefehler. Diese Aufrufe führen keine
Release-, Build-, Cargo-, Test- oder Formatterbefehle aus.

Die gezielte Dokumentsuche findet in den geänderten Anleitungen und im
Generator keine verbliebene Behauptung, der Task-Suite-prepare-Job stelle diese
Releaseeingabe her. Die Zielversion wurde durch diesen Fix nicht geändert.
Es wurde keine Task-Suite angelegt oder ausgeführt. Eine funktionale
Laufzeitbestätigung ist ausdrücklich nicht behauptet.

## Gelesene beziehungsweise gezielt durchsuchte Dateien

Repositorypfade, vollständig für diesen R01-Arbeitslauf aufgezählt:

- `AGENTS.md`
- `docs/ARCHITEKTUR.md`
- `docs/plaene/2026-10-24-batches/release-recovery.md`
- `.github/workflows/build.yml`
- `.github/workflows/review-task.yml`
- `native/publish-release-local.ps1`
- `native/release-publication.ps1`
- `native/release-version.ps1`
- `native/release-lock.ps1`
- `native/rust-licenses.ps1`
- `native/publish-update.ps1`
- `native/publish-feed.sh`
- `native/publish-linux-feed-wsl.sh`
- `native/build.rs`
- `native/src/lib.rs`
- `native/src/licenses/mod.rs`
- `native/installer.nsi`
- `android/build-release-apk.sh`
- `native/Cargo.toml`
- `native/Cargo.lock`
- `native/explorer-command/Cargo.toml`
- `native/explorer-command/Cargo.lock`
- `share-server/Cargo.toml`
- `share-server/Cargo.lock`
- `third-party/rust-crates/about.toml`
- `third-party/rust-crates/THIRD-PARTY-LICENSES.txt` (nur Bestandslektüre)
- `README.md`
- `docs/RELEASING.md`
- `graphify-out/graph.json` (ausschließlich gezielte Lizenzknotenabfrage)
- `docs/plaene/2026-10-24-batches/fix-release-license-inputs.md`

Zusätzliche ausdrücklich erlaubte Quellen:

- `/root/.codex/AGENTS.md`
- `/tmp/smartexplorer-final-review-user-contract.txt`
- `/tmp/smartexplorer-release-40425126500-license-failure.log`
- `/root/.codex/skills/arbeitsweise/SKILL.md`
- `/root/.agents/skills/arbeitsweise/SKILL.md`
- `/root/.codex/skills/graphify/SKILL.md`
- Die vom Arbeitsweise-MCP gelieferten Anweisungen und ausdrücklichen
  Forschungs-, Planungs- und Reviewreferenzen.

## Geänderte und neue Dateien; Übergabe

Geändert: `.github/workflows/build.yml`, `native/publish-release-local.ps1`,
`native/release-publication.ps1`, `native/rust-licenses.ps1`,
`android/build-release-apk.sh`, `README.md`, `docs/RELEASING.md`.
Neu: dieser Bericht. Der Lizenztext selbst, Produkt-Rust, Manifeste,
Lockdateien und der Quellgraph wurden von diesem Agenten nicht geändert.
Es wurden keine temporären Buildumgebungen, Downloads oder Paketinstallationen
angelegt; die vorhandenen Parser wurden verwendet.

Offene Abhängigkeit: Main muss diese begrenzte Quellkorrektur committen und
pushen und anschließend denselben vollständigen Remote-Release für die
beabsichtigte Version 0.5.185 ausführen. Der neue Generatorpfad sowie
Desktop-/APK-Einbettung sind erst durch diesen tatsächlichen Release praktisch
bestätigt. Es gibt keinen bekannten zusätzlichen technischen Blocker aus der
statischen Review, keine notwendige neue Pipeline und keinen erforderlichen
neuen funktionalen Tasklauf.
