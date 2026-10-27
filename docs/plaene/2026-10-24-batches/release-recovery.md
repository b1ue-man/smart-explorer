# Terminaler Release: begrenzte Wiederherstellung

Stand 2026-10-27: Mit v0.5.185 vollständig veröffentlicht; Abschlussbeleg am
Dokumentende. Die folgenden R01-/R02-Abschnitte dokumentieren die behobenen
Abbrüche vor dem Tag. Der Nutzerauftrag war der sofortige vollständige Release
ohne neuen funktionalen Tasklauf. B1–B8/P31/P32 und M01–M14 sind integriert.

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

## R02: Linux-musl-ABI des vollständigen Releasebuilds

Lauf `40433720045` auf `a94e1ac6546acbb57f37f67212ed53bda82d1e64`
endete am 2026-10-27 um 03:51:28 UTC mit Fehler. R01 ist im echten
Releasebuild bestätigt: Lizenzvorbereitung, Android-APK, Windowsinstaller und
GNU-Linux-Desktopprogramm wurden erfolgreich gebaut. Erst die statischen
Linuxprogramme für `x86_64-unknown-linux-musl` scheitern an sechs Diagnosen
in genau zwei Quelldateien. Es wurde kein Tag veröffentlicht; die beabsichtigte
Version bleibt 0.5.185. Die beibehaltene temporäre Releaseablage wird nicht
manuell übernommen; nach dem Fix läuft derselbe vollständige Wrapper erneut.

Erstplan aus dem konkreten Log: Die fehlende `libc::renameat2`-Bindung in
`mount/os/linux_os/recovery_storage.rs` und die abweichenden Längenfeldtypen
in `apps/os/linux_os/window_correlation.rs` korrigieren. Keine zusätzliche
Funktionssuite, keine Warnungsbereinigung, kein Umbau des Releaseverfahrens.

Erster Quellenpass am 2026-10-27: Die geprüften Quellen der gesperrten
Abhängigkeit `libc 0.2.186` und die tatsächlichen musl-Compilermeldungen zeigen
die ABI-Abweichung. Die bestehende Linux-Dateigrenze verwendet bereits
`libc::syscall(libc::SYS_renameat2, ...)` in
`local_access/os/linux/confined.rs` und `quarantine.rs`.
[rename(2)](https://man7.org/linux/man-pages/man2/rename.2.html) belegt die
Nichtüberschreib- und Parent-FD-Semantik;
[syscall(2)](https://man7.org/linux/man-pages/man2/syscall.2.html) beschreibt
den Aufruf ohne C-Bibliothekswrapper und die errno-Weitergabe.
[cmsg(3)](https://man7.org/linux/man-pages/man3/cmsg.3.html) und
[unix(7)](https://man7.org/linux/man-pages/man7/unix.7.html) beschreiben
Control-Headerlängen, Alignment und die Übergabe von Dateideskriptoren.

Zweiter Lückenpass: Recovery-Publish wird für Identitäten, Paketexport,
Paketimport, Cachewiederherstellung und den Registry-Eigentümercommit benutzt.
Nur die ausdrücklich erlaubte Ersetzung darf überschreiben. Ein Ersatz durch
gewöhnliches `renameat` würde den Nichtüberschreibvertrag verlieren.
Die Fensterzuordnung hält den echten pidfd des gestarteten Kindes, damit
PID-Wiederverwendung keine falsche Zuordnung erzeugt. GNU und musl haben
unterschiedliche Rust-Feldtypen für dieselben Control-Längen; die festen kleinen
Puffer erlauben begründete zielabhängige Konversionen ohne geänderte Nutzdaten.

Finaler begrenzter Plan vor Quellenänderung:

1. Recovery-Publish über die vorhandene Linux-Systemaufrufkonvention verfügbar
   machen. Parentbindung, Nichtüberschreiben, erlaubtes Ersetzen, errno und
   anschließendes Directory-fsync bleiben erhalten. Erwartung: derselbe
   Veröffentlichungsvorgang ist auf GNU, musl und Android darstellbar.
2. Die vier betroffenen Control-Längenausdrücke ABI-gerecht typisieren. Fester
   Puffer, Headerprüfung, pidfd-Besitz, Close-on-exec, Trunkierungsprüfung,
   nichtblockierendes Verhalten und unveränderter Appstart bleiben erhalten.
   Erwartung: GNU und musl bilden dieselbe einzelne SCM_RIGHTS-Nachricht ab.
3. Ein Fixautor besitzt Fehlerfunktionsliste, direkte/indirekte Karte,
   präzisierten Erhaltungsplan, Umsetzung und eigene statische Nachreview in
   `fix-release-linux-abi.md`. Main integriert, aktualisiert den vollständigen
   Quellgraph, committet und pusht. Abnahme der Kompilierbarkeit erfolgt im
   ohnehin erforderlichen vollständigen Remote-Release, nicht lokal.
4. Derselbe stabile Releaseweg veröffentlicht einmal 0.5.185. Ein erfolgreich
   gebautes Teilprogramm ist noch kein vollständiger Release.

Modellwahl nach Bestandsklärung und Plan: Coding, kleiner Umfang, Grad 7 wegen
zweier unsicherer OS-/FFI-Grenzen mit bestehenden Sicherheitsverträgen.
Das Auswahlwerkzeug empfiehlt gpt-6.1-sol/medium; die ausdrückliche
Fixautorenregel bestimmt gpt-6.1-sol/max. Ausführung über Codex-Credits;
Antwortmodelltelemetrie und gemessener Aboverbrauch sind nicht verfügbar.

R02 ist vom selben Autor umgesetzt und anschließend statisch gegen die
verbundenen Aufrufer und libc-Zieltypen geprüft; siehe
[vollständigen Autorenbericht](fix-release-linux-abi.md). Nur musl verwendet
den direkten `SYS_renameat2`-Aufruf. GNU und Android behalten ihren bisherigen
Wrapper. Die vier Control-Längenausdrücke sind zielgerecht typisiert; keine
Nutzdaten, Flags oder Ablehnungsbedingungen wurden geändert. Main hat den
begrenzten Diff integriert und beide Rustdateien ohne Compiler mit dem
vorhandenen Tree-sitter-Rust-Parser gelesen. Das ist kein Kompilier- oder
Laufzeitnachweis. Der nächste solche Build ist der vollständige Remote-Release.


## Abschluss: vollständige Veröffentlichung

R01 und R02 sind im tatsächlichen vollständigen Remote-Release bestätigt.
Lauf `40450627176` für `662aa0e5` endete am 2026-10-27 um 06:54:11 UTC
mit Erfolg. Der stabile Wrapper hat denselben vorgesehenen Versionsstand
0.5.185 veröffentlicht; nach beiden Fehlern vor dem Tag entstand keine
zusätzliche Patchversion. Releasecommit und unveränderter Tag:
`278a827fdae21c25446f717972cd9adc3732c6e0` / `v0.5.185`.
Publikationslauf `40463241931` ist erfolgreich, die
[GitHub Release](https://github.com/b1ue-man/smart-explorer/releases/tag/v0.5.185)
mit allen 26 erwarteten Assets sichtbar. Main hat alle veröffentlichten
Größen und SHA-256-Werte gegen die getaggten Dateien abgeglichen sowie
sämtliche neun Feed-Hashdateien geprüft. Es gab keinen erneuten
funktionalen Tasklauf; diese Abnahmegrenze bleibt ausdrücklich bestehen.
Es gibt keinen offenen Wiederherstellungsschritt dieses Releasebatches.
