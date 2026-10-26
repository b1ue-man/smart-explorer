# Zusätzliche Regressionsreviews vor der Remote-Suite

Stand: 2026-10-26. Dieser Vertrag ergänzt die normale frische Gesamtreview und
die bereits beauftragten Astra/xhigh-Batchreviews. Sämtliche Ausführung erfolgt
über Codex. Der normale Gesamtreview und beide unabhängigen Regressionsreviews
des festen Kandidaten `ee75e91bad6b28ef9206a5b2b2857cbea75c4e5b` sind abgeschlossen.
Die belegten Fehler werden vor der gemeinsamen Remote-Suite behoben.

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

Vor dem Reviewstart wurden der vollständig integrierte Kandidatencommit
`ee75e91bad6b28ef9206a5b2b2857cbea75c4e5b` und seine gesamte geänderte Dateioberfläche festgehalten.
Der Arbeitsbasiscommit `bea312ea` ersetzt diese Releasevergleichsbasis nicht.
Die Batchreviewkorrekturen an Android-Besitzübergaben (`8ac10dc8`),
Renderer-Eingabefreigaben (`43f46c8f`) und Capture-/Workerintegration (`f945ef59`)
sind einschließlich der eigenen statischen Anti-Regressionsreview des jeweils
zuständigen Fixautors integriert. Der Reviewkandidat enthält den gemeinsamen
Suiteanschluss und den vollständig erneuerten nativen Quellgraphen.

## Abgeschlossene Reviews und zugeordnete Fixzyklen

Die getrennten Ergebnisse stehen in [Gesamtreview](review-gesamt.md),
[Astra-Regressionsreview](review-regression-astra.md) und
[Sol-Regressionsreview](review-regression-sol.md). Die nachgeforderte Lektüre
der zunächst ausgelassenen geänderten Capture-/Renderer- und Recoverykörper ist
in denselben Reviewern abgeschlossen. Die Reviewer haben weder die Fixkopie
noch die unabhängigen Berichte gelesen. G5 wurde durch den normalen Reviewer
nach Gegenbeleg zurückgenommen und löst keinen Fix aus.

| Zugeordnete Befunde | Konkreter Funktionsumfang | Einziger Fixautor |
|---|---|---|
| RA-01 | Android-Videowachhaltung und Surfacebesitz | `fix_android_media` |
| RA-02/03/04/09, RSOL-01/02/05 | Gewöhnliches WebDAV-Umbenennen, FTP-/SFTP-Ordnerkopie, fehlende Dateien übertragen, Upload ohne Leserecht, gleichvolumiges Verschieben ohne Zusatzkopie | `fix_transfer_compat` |
| RA-05, RSOL-03/04/10 | Legacy-Screenverbindungen, parallele Android-Vorschauen, Aufnahmefortsetzung und tatsächliches Ressourcenende | `fix_screen_budget` |
| RA-06/08, RSOL-06/07/08 | Legacy-Profilentfernung und Migration, Offline-Terminalweg, Windows-MCP-Deinstallation | `fix_mcp_lifecycle` |
| RA-07 | Wiederholbarer Vault-Laufwerkstart mit erhaltenem Recoveryeintrag | `fix_vault_retry` |
| G1/G2 | X11-Combo- und Untermenübedienung mit Eigentümerfokus | `fix_x11_family` |
| G4, RSOL-09/11 | Ungesendetes Windows-Loginbild sowie wiederholbare Windows-/X11-Aufnahmezulassung | `fix_capture_restart` |
| G7/G8/G9 | Android-Geräteaufträge und Appfensterzugang beider Desktopflächen | `fix_device_navigation` |
| G3 | Isolierte P27-Relayreferenz und V7-Leistungsauswertung | `fix_p27_acceptance` |
| G6 | Versionsgebundener SSIM-/PSNR-Vergleich derselben Captureframes | `fix_quality_acceptance` |

Alle Fixautoren wurden über `collaboration.spawn_agent` mit
`model=gpt-6.1-sol` und `reasoning_effort=max` gestartet. Diese Aufrufparameter
sind belegt; zusätzliche Antwortmodell-Metadaten stellt der Host nicht bereit.
Der gesamte Fünfteilzyklus bleibt je Zeile beim genannten Autor.

Alle zehn statischen Autorzyklen sind abgeschlossen und als kohärente
Meilensteine auf `codex/review-fixes-ee75e91b` committed und gepusht.

| Commit | Vollständiger Bericht des zuständigen Fixautors |
|---|---|
| `caa85706` | [Androidmedienfix](fix-regression-android-media.md) |
| `a3f10e85` | [P27-Abnahmefix](fix-review-p27-abnahme.md) |
| `faf44025` | [Gerätenavigation](fix-device-navigation.md) |
| `4680ecb9` | [Qualitätsabnahme](fix-quality-acceptance.md) |
| `5c7a4c50` | [X11-Familien](fix-x11-family.md) |
| `4bc51e6e` | [Transferkompatibilität](fix-regression-transfer.md) |
| `4abc3b40` | [Screen-Sitzungsbudget](fix-regression-screenbudget.md) |
| `5ed8f8f4` | [Aufnahmewiederanlauf](fix-windows-capture.md) |
| `3705d85e` | [Vault-Retry](fix-vault-retry.md) |
| `c3265aab` | [MCP-Lebenszyklus](fix-regression-mcp.md) |

Die Transfer- und MCP-Autoren haben auch die bei der Integration bestätigten
Folgelücken im jeweils eigenen Zyklus geschlossen: fehlende Dateien nach
Verschieben oder frühem Startfehler erneut übertragen und Legacy-Pluginprofile
bei nur lesbarer CLI-Installation erzeugen. Jeder Bericht enthält die eigene
anschließende Prüfung der ermittelten Funktionskette.

Main hat die Änderungen zusammengeführt, ihre statische Syntax und die
konkreten Schnittstellen abgeglichen sowie die benannten Fälle in dieselbe
Remote-Suite aufgenommen. Die Ressourcenregistrierungen R10 bis R18 nennen
Plattform, genaue Methoden, erwartete Wirkung und verbleibende Grenzen.
Die funktionalen Ergebnisse sind noch nicht remote ausgeführt. Kein Bericht
wird als pauschale Regressionsfreiheit oder ausgeführter Plattformnachweis gewertet.

## Erwartetes Ergebnis und Grenzen

Beide Berichte liegen getrennt vor; jeder Befund erhält seinen tatsächlichen
Status. Jeder daraus entstandene Fix enthält die fünf Ergebnisse seines
zuständigen Sol-6.1/max-Agenten. Die normale Gesamtreview, beide
Regressionsreviews und die Behebung ihrer tatsächlichen Fehler liegen vor dem
ersten beabsichtigten Remote-Suite-Lauf. Ein vollständiger statischer Review
ist keine mathematische Garantie, dass im Laufzeitbetrieb keine unbekannte
Regression existiert. Solche Sicherheit wird weder behauptet noch durch
fiktive Testresultate ersetzt.
