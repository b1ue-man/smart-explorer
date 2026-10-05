# Eine Remote-Abnahme: konkrete Eingänge und Ergebnisorakel

Stand 2026-10-05. Die vor Code festgelegten Orakel sind umgesetzt;
der gemeinsame Remote-Fixloop läuft. Die erfolgreiche Gesamtabnahme steht aus.
Ein checked-in Einstieg `native/test-sync-reliability-task.py` besitzt die
Stufen `native`, `android-build`, `device` und `evaluate` desselben kandidatengebundenen
Workflows. Die finale Zuordnung wird im selben Script maschinenlesbar gehalten.
Nur dieser Workflow wird für den Kandidaten gestartet und gegebenenfalls
nach Korrekturen wiederholt. S9 ist kein Suitefall, sondern der terminale Release.

| Fall | Produktionsgrenze / Fixture / zur Laufzeit ermittelte Inputs | Ergebnisorakel |
|---|---|---|
| **C01 Notebook** | Neue cfg(test)-`sync_reliability_task_notebook_*`-Abläufe über GDriveBackend, `task_http::Server`, `task_drive::FakeDrive`, Bisync preview/run. Provider liefert andere Schreibweisen zusätzlich. IDs, Ports und lokale Roots stammen aus der Fixture. | Vollständiger `.obsidian`-Baum mit bekannten Bytes am anderen Ende; exakt richtiger Folder-ID-Baum; keine zusätzliche Folder-POST; zweiter Lauf ohne Transfer und ohne mutierende Requests. |
| **C02 Alle Drive-Namensgruppen** | `sync_reliability_task_names_*`: unterschiedliche Datei-IDs (gleich/verschieden), Ordnerduplikate mit Dateien darunter, Mischtypen, Literalmarker/Prozent/Unicode/Whitespace und Präfixkollisionen. Zwei FakeDrive-Konten mit getrennten Servern/IDs und echte LocalBackends; bestehende `sync_conflict_task_*`-Gesamtabläufe. | Inhaltsauswahl/Backup/Publish/Trash nach exakter ID, vollständige unabhängige Bäume, stabile projizierte Schlüssel. Drive↔Drive-Hin-/Rück-/No-op-Läufe erzeugen keine Zusatzordner und keine neue Escapestufe. Windows-TargetLimits werden ausdrücklich geprüft; geschützte unrepräsentierbare Pfade sind ein sichtbarer Teilstatus, kein voller Pass. |
| **C03 Paging, alte Bindung und Konkurrenz** | `sync_reliability_task_identity_*` über echte HTTP-Aufrufe und private Bindungspersistenz: leere/überlappende Seiten, rejected/cyclic Token, incompleteSearch, reale Root-Parent-ID, konkurrierende Erstbindung, Restart, Cache-Clear und kontrollierte 403/5xx/Verbindungsausfälle. Unbekannter mehrdeutiger Altroot vor und nach einem anderen Parent-Sync; später bewiesener alter ID-Hint und ausdrückliche exakte Pickerauswahl. | Eine persistierte Auswahl bleibt unverändert, während ihre Prüfung vorübergehend unmöglich ist. Vollständiger Retry endet nach Wiederherstellung; kein Ersatzobjekt, keine verlorene Zuordnung, kein Teil-Snapshot. Fehlende bekannte IDs erst nach frischer Bestätigung als fehlend. Eine Parent-Projektion autorisiert keinen unbekannten historischen Plain-Root; validierte alte Auswahl und exakter Picker bleiben wirksam. Herkunftslose Bindungsrecords bewahren IDs, Aliase, Tombstones und ursprüngliche Prüfsumme, erfinden aber keinen Altroot-Beweis. |
| **C04 Provider- und Locator-Matrix** | Native Rust-Fall `sync_reliability_task_provider_matrix`: echte resolvergeöffnete lokale Roots, SFTP mit Passwort bzw. verschlüsselter gespeicherter Schlüsseldatei, ausgeführter SSH-Remote-Agent, FTP/FTPS, WebDAV, Direct/Room und SMB/UNC plus Drive-Vertragsserver und ZIP-Quelle. Linux-Suite besitzt OpenSSH/FTP-/TLS-/DAV-/Samba-/Share-Server und den deployten Remote-Agent; Windows besitzt temporäre eigene SMB-Freigabe und freie gemappte Laufwerkskennung. Config/CA/Ports/Users/Export-/Room-/Peer-IDs werden aus Serverausgaben/Readiness und regulären gespeicherten Verbindungen gewonnen. | Pro unterstütztem schreibbaren Providerpaar einschließlich disjunkter Roots desselben Providers vollständiger Seed→Änderung→Gegenänderung→No-op-Ablauf; tatsächliche Endbytes und gewählte Provideridentität. Key-Authority erlaubt keine Passwortanmeldung; private Keydatei/Passphrase müssen den bestehenden Credential-/Resolververtrag benutzen. ZIP/read-only nur in erlaubter Quellrichtung. Same-path/fremde Authority und Guard-/Cache-/IPC-Wrapper bleiben isoliert. |
| **C05 Optionen und Sync-Modi** | `sync_reliability_task_options_*`, bestehende ausgewählte Engine-/Snapshot-/Apply-/Incremental-Fälle. Produktions-`BisyncOptions`, `RunSettings`, Jobcodec/Editor; lokale/Drive-/Remote-Gegenstellen. Runtime enumeriert die vorhandenen Direction-/Conflict-/Compare-/Delete-/Versioning-Varianten. | Konkrete erwartete Datei-Inhalte pro Modus; Filterausnahmen und geschützte Gegenstücke, wirksame gespeicherte Einstellungen, tatsächlich restaurierbare verdrängte Bytes, richtige Baseline und No-op-Folgelauf. Nicht nur Options-Serialisierung. |
| **C06 Stören und erfolgreich weiterlaufen** | `sync_reliability_task_resume_*` plus ausgewählte tatsächliche `sync_conflict_task_*`-, Safety-, Retry-, Recorded-Merge-/Journal- und Provider-Lost-ACK-Abläufe. Fehlerserver schaltet nach genau erfasstem Scan/Backup/Create/Publish um; Abbruch wird über den normalen Cancelpfad gesetzt. | Vor Bestätigung bleiben alte Bytes/Baseline; nach Aufheben jeder temporären Störung konvergiert derselbe Job. Kein Blind-Replay und keine doppelte Anlage. Zurückgehaltene Recoverybytes sind auf dem normalen Pfad auffindbar; keine unbeaufsichtigten Prozesse. |
| **C07 Geschützte Grenzen und Owner** | Ausgewählte `sync_links_task_*`, snapshot-walk, engine-identity/account/lock, backup-failure und stat/recycle-failure Abläufe; echte lokale Symlink-/Windows-Junction- und Agent/Peer-Fixtures. | Unabhängige Dateien fertig, geschützte Subtree-Gegenstücke und Baseline erhalten, keine unbestätigte komplette Indexgeneration. Neue Fremdinhalte und fremde Accounts unberührt; Lösch-/Backupzulassung unverändert. |
| **C08 Echter Desktop-Altjob und Update** | Der Suite-Einstieg extrahiert veröffentlichte v0.5.169-`se`-Bytes und Sidecar aus dem Tag, isoliert APPDATA/XDG, erstellt normale historische Jobdatei und startet mit `se share status --json` den damaligen Worker. Ein zulässiger OnStartup/Intervall-Trigger führt den Job aus. Script entdeckt Jobstate/Baseline unter dem Profile und wartet auf tatsächlich bestätigten Lauf. Danach `se update --complete-install <aktuelle Runtimeversion>` mit Kandidaten-CLI, Änderung und erneutem Trigger. Zusätzliche cfg(test)-`sync_reliability_task_old_jobs_*` führt tatsächliches `daemon::job::run_one` mit alten Drive-/Crossremote-Locators aus; Provider-Injektion nur cfg(test). | Alter Worker hat real Dateien und Zustand erzeugt. Neue Generation übernimmt exakt dieselben Jobdateien/Endpunkte/Options/StateKey und schließt Änderung/Konflikt/Wiederanlauf ab. Kein Neuanlegen und No-op-Folgelauf. Windows DACL-/OAuth-Konfigkorrektur und alte TSV-/Baselineimporte bleiben wirksam. |
| **C09 Echter Android-Altzustand und Neustart** | Vorhandener Emulator-/JNI-/Instrumentation-Vertrag; neue kohärente Klasse `SyncReliabilityTaskTest`. Alte gehashte veröffentlichte APK v0.5.169 führt Job durch unveränderte API aus. Gleich signierte inkrementelle **Development**-APK übernimmt das bestehende Paket/Appdata. Release-Signing-Secrets sind bereits vorhanden; Debug/Testsignierung wird nur für diese Remote-Abnahme ausgewählt. Native Build nur betroffene Bridge für Emulator-ABI aus Cache, kein vollständiger Releasebuild. | Aus derselben Appdata werden alter Job, Baseline, Optionen und Providerbindung erhalten. `force-stop`/Neustart zwischen Prepare und Retry; neue JNI-/Worker-Läufe konvergieren und behalten private Daten/Versionsorte. Quelle-/Zielbytes werden vom Gerät gelesen. |
| **C10 Echter Google Drive** | `sync_reliability_task_live_drive_*`, nur auf Remote-Runner; reguläre Cloudconfig und Credentialstore mit `SE_DRIVE_TEST_CLIENT_ID`, `SE_DRIVE_TEST_CLIENT_SECRET`, `SE_DRIVE_TEST_REFRESH_TOKEN`. Eigener eindeutig erzeugter Testroot, erzeugte IDs aus API-Antworten; exklusive Mutationen innerhalb dieses Roots. API-Basis und OAuth bleiben unveränderte Produktionsendpunkte. | Realer Google-Notebook-Lauf mit zusätzlicher Schreibweise und echten Datei-/Folderduplikaten; Inhalt/Backup/ID-Wahl, Restart derselben Bindung, Änderungs- und No-op-Lauf. Cleanup ausschließlich eigener erfasster IDs. Fehlt die Autorisierung, wird C10 als Abnahmeblocker ausgegeben; niemals Skip=Pass. |

C03 schließt ausdrücklich die erste Migration mit noch vorhandener globaler
Hintdatei und inzwischen geschriebenem Accountcache ein. Der gemeinsame echte
Dateilader muss die alte Notebook-ID auch dann erhalten, wenn zunächst ein
anderer Job lief. Ein neuer Accountcache ohne historische Evidenz beweist
keine alte mehrdeutige Root-Auswahl. Bereits vor der ersten neuen Registry
gelöschte, verschobene oder umbenannte historische IDs dürfen kein erreichbares
gleichnamiges Ersatzobjekt übernehmen; geprüft wird auch ein Hint ohne MIME.

Für C10 sind Client-ID und Refresh-Token erforderlich; das Client-Secret wird
nur gesetzt, wenn der verwendete OAuth-Client es verlangt. Im Abnahmebericht
stehen ausschließlich Namen fehlender Eingänge, keine Credentialwerte.

## Besitz, Infrastruktur und Fixloop

Der native Testhost erhält vor jeder Provideröffnung ein eigenes APPDATA-/
LOCALAPPDATA-/XDG-Profil. Windows-Credentials und Debug-IPC werden zusätzlich
durch `SMART_EXPLORER_E2E_TEST_NAMESPACE` pro Kandidat, Run-ID und Versuch
getrennt; der vorhandene Parser verlangt 1–48 erlaubte ASCII-Zeichen.
Der separate veröffentlichte Altworker übernimmt sein eigenes Profil auf dem
normalen Windows-Release-IPC-Pfad ohne diesen Debug-Namespace. Private Profile,
Google-Credentials und Fixture-Keys werden nicht als Logartefakte hochgeladen.

Die vorhandenen Suite-Helper liefern Cargo-JSON-Artefaktfindung und gehashte
Development-Binaries. Die konkrete Serversteuerung wird aus vorhandenen
`android/test-servers/`, Remote-/Transfer-Fixtures und aktuellen Primärrefs
abgeleitet; Aufrufsyntax, Banner-/Readiness-Formate und TLS/SSH-Agent-Verträge
werden vor deren Implementierung frisch gelesen und gesichert. Server werden
exklusiv pro Run erzeugt und in `finally` beendet. CA-Vertrauen bleibt im
isolierten Remote-Testprofil. Da FTP/ureq gebündelte CA-Wurzeln verwenden,
bekommt nur das cfg(test)-Transportsetup die echte Fixture-CA als zusätzliche
Vertrauenswurzel; OS-Truststoreänderungen allein reichen nicht. Resolver,
Protokoll und Sync bleiben unverändert; die App bekommt keine gelockerten
TLS-Prüfungen. Syntax/Eingangsbytes sind in
`docs/refs/sync-task-runtime-2026-10-04.md` gesichert.

Die ZIP-Quelle wird über den vorhandenen `ZipBackend` an der gemeinsamen
Enginegrenze geöffnet. `EndpointSpec` kennt keinen gespeicherten ZIP-Joblocator;
die Abnahme erfindet dafür keinen neuen Job-/Resolververtrag. Die bestehende
read-only Quelle wird in ihrer zulässigen Quellrichtung geprüft, während alle
gespeicherten Remote-Provider über ihre tatsächliche Resolvergrenze laufen.

Der Mainagent wertet jeden Fall einschließlich dessen Laufzeit-Orakel aus.
Fehlende Provider, historische Bytes, Runtimewerte oder tatsächlich aufgerufene
Fälle lassen die eine Abnahme scheitern. Fixes werden gesammelt committed,
gepusht und nur durch denselben Suite-Einstieg bestätigt. Die Suite baut keine
Installer, Feeds oder Releasepayloads und veröffentlicht nichts.

## Laufzeitevidenz und Korrekturen

Der [erste gemeinsame Remote-Lauf](https://github.com/b1ue-man/smart-explorer/actions/runs/37244738623)
prüft Kandidat `1123d125d637b8d6628e729c1f8f4d006c7cf1a7`.
Am 2026-10-05 ist dieser Lauf vollständig als fehlgeschlagen ausgewertet. Die Android-Buildstufe
belegt eine erfolgreich verifizierte alte APK mit der Ausgabe `V2 Signer`,
die der neue Helper zunächst nicht erkannt hat. Der korrigierte Parser übernimmt
die dokumentierten Signerformen des bestehenden Release-Scripts, normalisiert
und dedupliziert vollständige SHA-256-Digests und verlangt genau ein Zertifikat
für alle drei APKs. Die Geräteübergabe benötigt die erfolgreich abgeschlossene
Buildstufe mit finaler Provenienz; ein Zwischenmanifest autorisiert sie nicht.
Die nativen Hosts wurden vor den Sync-Abläufen durch `E0433` beim falschen
`JobEditor`-Modulpfad angehalten; der Pfad ist auf den vorhandenen öffentlichen
`syncjobs::editor`-Vertrag korrigiert. Daraus folgt noch kein Laufzeitnachweis
für C01–C08. Der Linux-Report bestätigt außerdem fehlende C10-Autorisierung.
Der kandidatgebundene Formatpatch mit SHA-256
`29cbe118d34d282718a93611bf27eb4eda2f3e9842b7f3638d1d864331d62bbe`
ist geprüft und übernommen; dessen Kontext wurde ausschließlich für den
bereits korrigierten Editorpfad angepasst. Kein Format-/Größenproblem bleibt
im ersten Formatreport offen. Bestätigung der Korrekturen und eigentliche
Sync-Abnahme erfolgen ausschließlich im selben Remote-Suite-Einstieg.

Der [korrigierte zweite Lauf](https://github.com/b1ue-man/smart-explorer/actions/runs/37247279728)
prüft `13cb308caf0d350b3de32c0be37e250c81ff6e99`. Android-Build und verifizierte
APK-Übergabe sind erfolgreich. C09 scheitert danach im ersten Sync von
`old-prepare` unter der unveränderten veröffentlichten APK; die Antwort nennt
nur einen Fehler ohne dessen Ursache. Die Instrumentation übernimmt deshalb
den vollständigen vorhandenen `task.get`-Snapshot mit Pfad und Fehlermeldung
in ihre Assertion. Sie verlangt weiterhin dieselben erfolgreichen Sync-Zähler;
eine Komponente wird ohne diese Diagnose nicht als Ursache behauptet.

Die Linux-Stufe des zweiten Laufs ist ausgewertet. Notebook, Paging/
Bindungsmigration und geschützte Grenzen erfüllen ihre Orakel. Die tatsächliche
veröffentlichte Desktop-Worker-Übernahme mit erhaltenem Job, Konfliktbytes,
Abbruch/Wiederanlauf und No-op ist ebenfalls erfolgreich; C08 bleibt wegen
der fehlenden Crossremote-Fixture insgesamt offen. Der HTTPS-DAV-Readinessaufruf
erhält HTTP 405; deshalb fehlen die regulären Provider-Fixtures auch dem
Crossremote-Altjob. Weitere konkrete Fehlstellen betreffen KeepBoth-Erhaltung,
RunDepth/Mirror-Erwartungen, einen temporären Stage-Create-Fehler, den
Move-Finalize-Wiederanlauf und die Identitätsprüfung im Drive-Roundtrip.
Diese Ursachen werden vor dem nächsten Aufruf desselben Einstiegs behoben.
Die Windows-Stufe ist ebenfalls vollständig ausgewertet. Notebook,
Bindungsmigration, Schutz und die echte Altworker-Übernahme erfüllen auch dort
ihre Orakel. Zusätzlich scheitert die gesunde `healthy.txt`-Änderung an einem
unrepräsentierbaren Backslash-Geschwister; dessen geschützter Teilstatus darf
unabhängige Dateien nicht verhindern. Die Windows-Provider-Fixture meldet
fehlende Direct-Request-Readiness. Der Share-Server konnte seinen Relay-Port
nicht binden, während der Helper bloße Worker-Erreichbarkeit akzeptiert hatte;
die Korrektur muss Serverlistener und tatsächlich verbundene Peers bestätigen.
Die vollständige gemeinsame Evaluation ist fehlgeschlagen. C10 nennt weiterhin
fehlende Client-ID und Refresh-Token. Keine Releasephase wurde gestartet.

Das Linux-Logartefakt enthielt außerdem das private Arbeitsverzeichnis der
Altworker-Fixture; das Windows-Archiv hatte dieselbe Grenze. Der Upload schließt
deren gesamte generierte Verzeichnisse jetzt wie das native Testprofil aus.
Die beiden eigenen Reportarchive `11319887997` und `11320438715` wurden nach
Sicherung der gezielten Diagnose entfernt (`DELETE` 204, folgende Sichtprüfung
404); kombinierte Summary und Joblogs bleiben erhalten. Ergebnis-/Bytehashbelege
bleiben im kandidatgebundenen Summary. Der Helper trennt zusätzlich seine
privaten Arbeitsdaten von ausgewählten hochladbaren Diagnosen.
