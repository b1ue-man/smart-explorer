# Terminaler Release: begrenzte Wiederherstellung

Stand 2026-10-27. Nutzerauftrag bleibt der sofortige vollständige Release ohne
neuen funktionalen Tasklauf. B1–B8/P31/P32 und M01–M14 sind integriert.

## R01: aktuelle Lizenztexte als vollständige Releaseeingabe

Erstplan: Den konkreten Abbruch von Lauf `40425126500`, Quellkandidat
`2081be2ed568ca4088f1d961f501c76f48236f31`, beheben und denselben stabilen
Releaseweg für dieselbe beabsichtigte Version 0.5.185 erneut ausführen.
Es wurde kein Tag und kein vollständiger Release erzeugt. Keine neue Suite,
kein lokaler Build, kein zusätzlicher Veröffentlichungsweg.

Bestand: Job `android-release-apk` erfolgreich. Job `complete-release`
`121939664735` scheitert am 2026-10-27 um 00:55:27 UTC in
`Assert-RustCrateLicenses`, vor dem Desktopbuild und dem Versionswechsel.
Der eingecheckte Text enthält 767 Crates; die erfolgreiche aktuelle Generierung
im entfernten Check enthält 783. Die Fehlermeldung nennt die fehlenden lokalen
Pfadabhängigkeiten egui_glow, iroh, noq und noq-proto sowie die hinzugekommenen
SVG-Abhängigkeiten. Der Generator hat keinen unaufgelösten Lizenzfehler gemeldet.

Erster Quellenpass: `native/rust-licenses.ps1` besitzt bereits Generate/Check,
den festen Generator 0.9.2, vollständige Manifest-/Zielauflistung, `--locked`,
Fehlerabbruch bei fehlenden Texten und Normalisierung. Die offizielle
[cargo-about-Dokumentation](https://embarkstudios.github.io/cargo-about/) erklärt
die Lizenzgenerierung aus dem Abhängigkeitsgraph; die aktuelle eigene Generierung
ist durch den Remotejob konkret belegt. `docs/RELEASING.md` behauptet bislang
die Erzeugung im Suite-prepare-Job, aber der aktuelle Review-Workflow enthält
keinen solchen Lizenzschritt.

Entscheidung: Die bestehende vollständige Releaseautomatisierung muss ihre
Lizenztexte vor dem Einbetten selbst vollständig erzeugen und prüfen können.
Kein Ausschalten der Lizenzprüfung, keine handgeschriebenen Ersatztexte und kein
zusätzlicher Testlauf zur Herstellung einer Build-Eingabe. Lokale Ausführung des
Generators wird nicht verwendet; dessen Werkzeuginstallation kann kompilieren.

Zweiter Lückenpass: `native/src/licenses/mod.rs` bindet den Text auch in Android
ein. Deshalb muss die Android-Bauvorbereitung mitgezogen werden; nur den
Desktopwrapper zu ändern würde die alte Lizenzansicht in der APK ausliefern.
`android/build-release-apk.sh` trennt Build, Verify und Check-env. Der
Wrapper trennt neue Builds, wiederverwendbare Kandidaten und unveränderlich
getaggte Kandidaten. `Get-PublicationReleaseCommitPaths` und
`Test-ReleaseMutablePath` begrenzen Git-Änderungen; generierte Lizenztexte müssen
in genau dieser begrenzten Releaseeingabe geführt werden. Bestehende APKs und
getaggte Artefakte dürfen nicht neu gebaut oder umgeschrieben werden.

Konkreter Plan, vor Quellenänderung:

1. Bestehenden Generator als begrenzte Releasevorbereitung vor neuen Android-
   und Desktop-Builds einbinden. Erwartung: aktuelle Texte existieren vor
   `include_str!`; fehlende oder unzulässige Lizenzbelege stoppen weiterhin.
2. Genau den kanonischen generierten Lizenzpfad in die notwendige
   Release-/Git-Recoverykette aufnehmen. Sauberer Quellkandidat, SHA-Bindung,
   unveränderliche Tags, Verify/Check-env, vorhandene Kandidaten und explizite
   Veröffentlichungsdateiliste behalten ihre Schutzwirkung.
3. README, Releaseanleitung und Generatorhinweise an den wirklichen Ablauf
   angleichen. Der gleiche Sol-6.1/max-Autor liefert Fehlerfunktion, direkte und
   indirekte Karte, präzisierten Plan, Fix und eigene anschließende statische
   Anti-Regressionsreview. Kein zusätzlicher Reviewagent und kein Funktionstest.
4. Main integriert und pusht nur diese begrenzte Korrektur. Danach derselbe
   komplette Remote-Releaseweg für 0.5.185; keine zusätzliche Patchversion und
   keine parallele Pipeline. Abnahme ist dessen wirklicher veröffentlichter
   Artefaktbestand, nicht die bloße Quelländerung.

Modellwahl nach geklärtem Bestand: Workflow, begrenzte verbundene Releasekette,
Grad 7; Werkzeug empfiehlt gpt-6.1-sol/high. Konfiguriert wird gemäß bestehendem
Fixautorenvertrag gpt-6.1-sol/max über Codex-Credits. Kein lokaler Aboverbrauch
wird als gemessen behauptet.

R01 ist umgesetzt und vom selben Autor anschließend statisch geprüft; siehe
[vollständigen Autorenbericht](fix-release-license-inputs.md). Main hat den
Quell-/Dokumentationsdiff und die direkte Publikationsintegration abgeglichen.
Der zusätzliche Workflowguard ist an dieselbe exakte Dateiliste angepasst.
Neue native Rustdateien oder Produktänderungen sind nicht entstanden; der
Quellgraph bleibt gültig. Nächster Schritt ist ausschließlich der bestehende
vollständige Remote-Release für denselben beabsichtigten Versionsstand.
