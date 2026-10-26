# Zusätzliche Regressionsreviews vor der Remote-Suite

Stand: 2026-10-26. Dieser Vertrag ergänzt die normale frische Gesamtreview und
die bereits beauftragten Astra/xhigh-Batchreviews. Sämtliche Ausführung erfolgt
über Codex. Die beiden zusätzlichen Reviewer sind noch nicht gestartet.

## Auftrag und verbindliche Zuständigkeit

Nutzervorgabe: Seit dem letzten veröffentlichten Release darf keine bestehende
Endnutzerfunktion durch die Änderungen unbeabsichtigt verloren gehen, kaputtgehen
oder negativ beeinflusst werden. Der interne Weg darf sich ändern, die Funktion
muss weiterhin funktionieren. Der Vergleich umfasst den vollständigen Diff des
fertigen Kandidaten seit diesem Release einschließlich vor dem aktuellen
Auftragsbasiscommit entstandener Änderungen und direkt oder indirekt betroffener
Integrationen.

Nach vollständiger Integration arbeiten zwei frische, voneinander unabhängige
Reviewer mit GPT-6-Astra/max und GPT-6.1-Sol/max. Ihr alleiniger Ergebnisauftrag
ist das Erkennen funktioneller Regressionen. Die normale frische
Astra/xhigh-Gesamtreview bleibt ein eigener Auftrag. Die beiden Reviewer liefern
Fehler mit konkretem Auslöser, beobachtbarer Endnutzerwirkung und Codebeleg. Auch
kleine tatsächliche Probleme werden genannt. Es gibt keine Mindestfehlerzahl,
keine erfundenen Fälle und keine als gesichert dargestellten Vermutungen.
Unsicherheit und fehlender Laufzeitnachweis müssen ausdrücklich erkennbar sein.
Stilpräferenzen ohne funktionelle Auswirkung sind keine Regression.

Für jeden aus diesen beiden Reviews hervorgehenden Fix wird ein eigener
Sol-6.1/max-Fixauftrag vergeben. Derselbe beauftragte Agent verantwortet alle
folgenden Ergebnisse vollständig:

1. Auflistung der fehlerhaften Endnutzerfunktion.
2. Vollständiges Abbild aller direkt und indirekt verketteten Programmstellen,
   soweit sie diese Funktion und die Auswirkungen des Fixes bestimmen.
3. Konkreter Fixplan, der die Funktion repariert und das Verhalten jeder
   betroffenen Verknüpfung erhält.
4. Umsetzung des Fixes.
5. Anschließende Anti-Regressionsprüfung der eigenen Änderung gegen die gesamte
   ermittelte Funktionskette mit Belegen und ausdrücklich offenen Grenzen.

Diese Kette wird weder zwischen Agenten geteilt noch von einem der beiden
Regressionsreviewer übernommen. Reviewer liefern Fehlerbefunde; die vollständige
Funktionskarte und der Fixplan entstehen beim jeweiligen Fixagenten. Zusammen
gehörende Befunde derselben Fehlerursache können einen kohärenten Fixauftrag
bilden. Diese Bündelung ist eine organisatorische Folgerung des Main und ändert
keine der fünf geforderten Pflichten.

Die lokale Ausführung von Builds und Tests bleibt verboten. Die anschließende
Prüfung durch den Fixagenten ist vor dem Suitebeginn eine statische Code- und
Quellenprüfung; die konkrete funktionale Bestätigung wird im selben einzigen
Remote-Task-Suite-Einstieg beziehungsweise dessen notwendigem Fixloop geführt.
Ein statischer Befund wird nicht als ausgeführter Laufzeitnachweis bezeichnet.
Die Hauptinstanz behält Commits, Push, Suiteorchestrierung und den einen
terminalen Remote-Release.

## Belegte Vergleichsbasis

Am 2026-10-26 lieferte die GitHub-REST-API
[`releases/latest`](https://api.github.com/repos/b1ue-man/smart-explorer/releases/latest)
den veröffentlichten, nicht als Entwurf oder Vorabversion markierten
[Release v0.5.184](https://github.com/b1ue-man/smart-explorer/releases/tag/v0.5.184),
veröffentlicht am 2026-10-22 um 10:04:37 UTC. Der separat abgerufene
[`git/ref/tags/v0.5.184`](https://api.github.com/repos/b1ue-man/smart-explorer/git/ref/tags/v0.5.184)
und der lokale aufgelöste Tag zeigen übereinstimmend auf
`a814f263096fe69d3746e840b5f3fad3fd7e88a0`.

`native/Cargo.toml`, `release-native/update-feed/version.txt` und der vorhandene
Installer `release-native/Smart Explorer Setup 0.5.184.exe` tragen dieselbe
Version. Alle sieben vorhandenen Desktop-/Android-Payload-Hashdateien wurden
gegen ihre tatsächlichen lokalen Payloadbytes abgeglichen und stimmen. Die
GitHub-Releaseantwort enthält den passenden Installer, die sechs
Desktoppayloads, Android-APK, sieben Payloadhashes, Linuxinstallationsskript,
Kontextmenü-DLL, beide Shareserverpayloads und `version.txt`.

Vor dem Reviewstart werden der dann aktuelle vollständig integrierte
Kandidatencommit und seine gesamte geänderte Dateioberfläche festgehalten.
Der Arbeitsbasiscommit `bea312ea` ersetzt diese Releasevergleichsbasis nicht.
Der genaue Reviewkandidatencommit steht noch nicht fest, weil Coreintegration,
Android-Fixtures und der Glow-Batchreviewfix noch bearbeitet werden.

## Erwartetes Ergebnis und Grenzen

Beide Berichte liegen getrennt vor; jeder Befund erhält seinen tatsächlichen
Status. Jeder daraus entstandene Fix enthält die fünf Ergebnisse seines
zuständigen Sol-6.1/max-Agenten. Die normale Gesamtreview, beide
Regressionsreviews und die Behebung ihrer tatsächlichen Fehler liegen vor dem
ersten beabsichtigten Remote-Suite-Lauf. Ein vollständiger statischer Review
ist keine mathematische Garantie, dass im Laufzeitbetrieb keine unbekannte
Regression existiert. Solche Sicherheit wird weder behauptet noch durch
fiktive Testresultate ersetzt.
