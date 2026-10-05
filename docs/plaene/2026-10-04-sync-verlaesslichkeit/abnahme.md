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
| **C10 Echter Google Drive** | `sync_reliability_task_live_drive_*`, nur auf Remote-Runner; reguläre Cloudconfig und Credentialstore mit erforderlichen `SE_DRIVE_TEST_CLIENT_ID`/`SE_DRIVE_TEST_REFRESH_TOKEN` und optionalem `SE_DRIVE_TEST_CLIENT_SECRET` gemäß OAuth-Client. Eigener eindeutig erzeugter Testroot, erzeugte IDs aus API-Antworten; exklusive Mutationen innerhalb dieses Roots. API-Basis und OAuth bleiben unveränderte Produktionsendpunkte. | Realer Google-Notebook-Lauf mit zusätzlicher Schreibweise und echten Datei-/Folderduplikaten; Inhalt/Backup/ID-Wahl, Restart derselben Bindung, Änderungs- und No-op-Lauf. Cleanup ausschließlich eigener erfasster IDs. Fehlt die Autorisierung, wird C10 als Abnahmeblocker ausgegeben; niemals Skip=Pass. |

C03 schließt ausdrücklich die erste Migration mit noch vorhandener globaler
Hintdatei und inzwischen geschriebenem Accountcache ein. Der gemeinsame echte
Dateilader muss die alte Notebook-ID auch dann erhalten, wenn zunächst ein
anderer Job lief. Ein neuer Accountcache ohne historische Evidenz beweist
keine alte mehrdeutige Root-Auswahl. Bereits vor der ersten neuen Registry
gelöschte, verschobene oder umbenannte historische IDs dürfen kein erreichbares
gleichnamiges Ersatzobjekt übernehmen; geprüft wird auch ein Hint ohne MIME.

C02 enthält zusätzlich drei vollständige historische Spellingabläufe unter
`gdrive::sync_reliability_task_spelling_migration_tests`: gleiche tatsächliche
Schreibweise, unterschiedliche Seitenpfade und eine neu auftretende physische
Alias-Zielkollision. Die Fixture erzeugt Signaturen über einen regulären Seed,
materialisiert dessen historische Viermap-/Baseline-/Dirbasis und entfernt
nur die eigene neue Registry. Unveränderter Restart, Gegenänderung, neue
unabhängige Casebäume, tatsächliche Datei-IDs, Side-B-Backup und Restore sowie
No-op bleiben Pflicht. Der Kollisionsfall erhält beide alten Counterparts
und die Baseline, bestätigt eine unabhängige gesunde Datei und wiederholt
den geschützten Teilstatus nach Restart. Nach regulärem Rename ausschließlich
des geprüften neuen Folder-IDs muss derselbe Job vollständig konvergieren.

C05 entdeckt über seinen bestehenden Prefix die beiden Abläufe unter
`bisync::engine_provider_task_tests::spelling_migration_tests`. Der Recorded-
Retry-Ablauf startet mit einer echten gefalteten Viermap-Datei, bestätigt
nichtmutierende Vorschau/Dry-run, unveränderten StateKey und No-op, erzeugt
einen echten Konflikt und verliert genau einen Publish-ACK. Reguläres Resolve
und Recovery müssen den tatsächlichen alten Side-B-Slot benutzen, dessen
Sicherung erhalten und ohne erneute Veröffentlichung abschließen. Ein neues
Kind unter dem alten B-Ordner wird auf seinen belegten A-Präfix übertragen.
Nach bestätigt propagierter Löschung wird eine neue exakte Literaldatei
angelegt; sie muss ohne wiederverwendeten gelöschten Dateialias übertragen
werden und den unveränderten Job nach Restart ohne Mutation abschließen.
Der inkrementelle Mirror-Ablauf verlangt die richtige Zieladresse ohne
Zielvollwalk, eine bestätigte vollständige Indexgeneration und No-op. Eine
anschließend widersprüchliche Spellingdatei darf weder Baseline noch Bytes
oder die vorherige Datei überschreiben; Wiederherstellung der gültigen
Datei ermöglicht demselben Job den erfolgreichen Folgelauf.
Der historische Flow erhält außerdem die alte gemeinsame Ordnerhistorie:
Nach dem Policywechsel muss eine bestätigte einseitige Ordnerlöschung am
richtigen Gegenstück propagiert werden und nach Restart ohne Mutation bleiben.
Ein alter Foldkey wird hierfür nicht als neuer Literalordner interpretiert;
geschützte oder fehlgeschlagene Löschungen geben keine Beziehung frei.

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
Die Anschlusslesung unterscheidet dabei bewiesene falsche Fixtureannahmen
von Produktfehlern: KeepBoth erhält den Verlierer in einer bestätigten
Konfliktkopie; der temporäre Create-Fehler traf zunächst den Replica-Marker;
der Move-Testwrapper bot keinen exklusiven Stage-/Finishvertrag. Diese
Gesamtabläufe werden am tatsächlichen bestehenden Vertrag korrigiert.
Der Incremental-Mirror hingegen fällt beim hashlosen Statvergleich unter
Checksum auf Vollscan zurück und bootstrapped anschließend erneut vollständig;
dieser betroffene Produktanschluss muss frische berührte Signaturen und die
bereits bestätigte Indexbasis verwenden. Im Drive-Roundtrip hatten die zwei
Fixturekonten unterschiedliche Permission-IDs, aber denselben synthetischen
Refresh-Token und deshalb denselben Legacyalias. Jeder Fixtureaccount erhält
nun einen stabil eigenen Token; die tatsächliche Pairalias-Prüfung bleibt
unverändert. Die Provider-Korrektur besitzt einen eigenen DAV-Root mit
bestätigtem Multistatus sowie unabhängig ermittelte Relayports und tatsächlich
verbundene Peers. Diese Änderungen sind noch keine erneute Laufzeitabnahme.

Die konkreten nativen Korrekturen sind implementiert und durch die jeweiligen
Umsetzer selbst geprüft. Die bisherigen vollständigen Byte-/Backup-/No-op-
Orakel bleiben erhalten; der Indexbeweis fordert zusätzlich eine vollständig
bestätigte, nicht dirty Cachegeneration mit genauen Journal-Signaturen.
Der private Altworkerroot liegt jetzt außerhalb der Uploadgrenze. Seine
Diagnose exportiert nur rekonstruierte Versionen, ausgewählte Worker-/Handoff-
Statusfelder, Command-Größen/Hashes und den Ergebnisbericht. Entfernt wird der
Privatroot erst nach bestätigtem Prozessende und Autostart-Restaurierung;
Cleanupfehler lassen ihn privat erhalten und schlagen die Stufe fehl.
Der [dritte gemeinsame Remote-Aufruf](https://github.com/b1ue-man/smart-explorer/actions/runs/37252322674)
prüft `bf6bd7552fcb5ea67b096953b55b4011ccf8e1e1`. Die Android-Buildstufe ist
erfolgreich. Die ergänzte C09-Diagnose belegt im unveränderten alten APK
`directory ancestor is a link or reparse point: /data/user/0` beim ersten
`CopyAtoB("note.txt")`. Der historische `mkdir_all`-Vertrag prüft sämtliche
Vorfahren. C09 löst deshalb ausschließlich die Basis seines eigenen
Fixture-Roots vor dem alten `sync.save` kanonisch auf; relative Hash-/Baseline-
und Versionsbelege benutzen dieselbe Basis. Die veröffentlichte alte App
erzeugt weiterhin den gesamten Altzustand, und beim Update bleiben ihre
gespeicherten Locators unverändert. Byte-, Baseline-, Backup- und No-op-Orakel
werden nicht abgeschwächt. Der dritte Workflow ist vollständig fehlgeschlagen
ausgewertet; die Android-Korrektur ist noch nicht erneut abgenommen.

Die abgeschlossene Linux-Stufe des dritten Laufs bestätigt zusätzlich die
Unterbrechungs-/Retry-Korrekturen (C06). Die veröffentlichte Altworker-Übernahme
bestätigt erhaltene Job-/Baseline-/Konflikt-/Sicherungsbytes und No-op sowie
Prozessende, Autostart-Restaurierung und Entfernung des Privatroots. Das
Reportarchiv enthält keine privaten Profile oder Altworker-Arbeitsdaten mehr.
Der Literal-Roundtrip scheitert jetzt konkret an `colon:name` in der
Sync-Engine (`relative path contains a drive or stream prefix`); die
Agent-Wire-Pfadgrammatik ist kein allgemeiner Drive-Namensvertrag. C05 nennt
weiter zwei Listings im zuvor bestätigten Mirror-Target. Sein Null-Listing-
und Nested-Byte-/Baseline-Orakel bleibt bestehen. Der Deletepolicy-Ablauf
fordert zudem zwei Löschungen einschließlich einer neu erschienenen
target-only Datei, ruft aber den inkrementellen statt den vollständigen Scan
auf. Er benutzt nun ausdrücklich `ScanDepth::Full`; alle Lösch-/Erhaltungs-
und restaurierbaren Backup-Orakel bleiben unverändert. Die erfolgreiche
Direct-Einrichtung führt in C04 beim Room-Export-Refresh zu `StaleAuthorization`;
dies blockiert auch den Crossremote-Altjob. C10-Autorisierung fehlt weiterhin.
Der geprüfte kandidatgebundene Formatpatch
`177a72fb52b60f62a01677acd94a15b467d8e4015aedb24ede977883e9721f0d`
ist statisch übernommen; sein Report enthält keine Größenverletzung.

Die Windows-Stufe des zweiten Laufs ist ebenfalls vollständig ausgewertet. Notebook,
Bindungsmigration, Schutz und die echte Altworker-Übernahme erfüllen auch dort
ihre Orakel. Zusätzlich scheitert die gesunde `healthy.txt`-Änderung an einem
unrepräsentierbaren Backslash-Geschwister; dessen geschützter Teilstatus darf
unabhängige Dateien nicht verhindern. Die Windows-Provider-Fixture meldet
fehlende Direct-Request-Readiness. Der Share-Server konnte seinen Relay-Port
nicht binden, während der Helper bloße Worker-Erreichbarkeit akzeptiert hatte;
die Korrektur muss Serverlistener und tatsächlich verbundene Peers bestätigen.
Die vollständige gemeinsame Evaluation ist fehlgeschlagen. C10 nennt weiterhin
fehlende Client-ID und Refresh-Token. Keine Releasephase wurde gestartet.

Im dritten Lauf ist die Windows-Server-/Peer-Einrichtung erfolgreich. C07
und die separate tatsächliche veröffentlichte Altworker-Übernahme erfüllen
ihre Orakel; deren Privatroot wurde nach Prozessende und Autostart-Restaurierung
entfernt und liegt nicht im Reportarchiv. Der gemeinsame native Host bricht
im Crossremote-Altjob mit Windows-Status `3221225725` und Stackoverflow ab.
C01–C03 und spätere Drive-/Jobfälle wurden dadurch nicht ausgeführt; sie sind
für diesen Kandidaten nicht als erfolgreich bestätigt. Die vorangegangene
C04-Assertion bleibt in der gepufferten Ausgabe ohne Ursache. Die gemeinsame
Evaluation verweigert den Kandidaten einschließlich der fehlenden Ausführung.
Das kombinierte Reportarchiv ist gegen SHA-256
`6c242df5395ea78411989a2b18c63c2d08778f058ae61add987eeb02f9bc65c5`
verifiziert. Die Suite liefert nun Diagnosen unmittelbar mit serieller
Pretty-Ausgabe; sie verlangt weiterhin jeden ausgewählten vollen Namen genau
einmal und eine passende terminale Summary. Unvollständige/duplizierte Fälle
oder abweichende Ergebniszähler bleiben fehlgeschlagen.

Die Room-Refresh-Korrektur wird im selben C04-Ablauf auf Default-Deny-
Wiederauftauchen, aktiven Widerruf und einen atomar abgewiesenen Batch geprüft.
C07 übernimmt zusätzlich die vorhandenen exakten Direct-/Room-Grant-,
Offline-/Online-Barrieren-, parallelen Revoke-/Launch- und unabhängigen
Principal-Abläufe. Sie beantworten ausschließlich die durch die gemeinsame
Registryänderung betroffenen Rechte-/Cancellationfragen; kein weiterer
Suiteaufruf und keine breite Exec-Matrix werden eingeführt.

Die im vierten Lauf geprüfte Korrektursammlung enthält einen reinen `SyncRelativePath` für
Apply, Baseline, SQL, Checkpoint, Spellings, Versionsmanifest und Replacement-/
Merge-Recovery. `:`, Backslash und `%` bleiben Providerliterale; native Namen,
Root-/Linkschutz und Agent-Wire werden an ihren bestehenden Grenzen geprüft.
Alte physische Archive behalten ihren nativen Pfadvertrag, moderne Versionen
verwenden opake private Datenpfade. Derselbe C02-Roundtrip fordert zusätzlich
Literal-Overwrite, genaue Backupbytes, wiedergeöffnete Endpunkte, gespeicherte
Originalpfade, Restore und abschließendes No-op. Im Nested-Mirror wird gegen
die vollständige bestätigte Sourceindexgeneration verglichen, die auch
implizit erzeugte Eltern enthält; das Null-Listing-Orakel bleibt unverändert.

Der statisch belegte Zyklus `AgentBackend` → `UnavailableBackend` →
`live_backend` → neuer identischer Agent betrifft `sync_child_path`,
`previous_state_identities` und `replace_staged_reversible`. Der Stub verwendet
hier wieder die konservativen VFS-Defaults: Literalpfad, keine unbewiesene
Altidentität und `Ok(false)` ohne Mutation. Der bestehende Bisync-Anschluss
behält geprüfte Sicherung vor Veröffentlichung, sicheres Replace ohne
Delete-Fallback und Recovery-Intents. Die neuen C08-Grenzfälle verwenden den
echten Agent-Dispatch mit bestehendem Hello-Framing für Direct/Room und Cache,
verlangen keine Dateisystem-RPCs und geschlossene Streams. Der tatsächliche
Crossremote-Altjob behält normalen Resolver und Runner; Loader-Restart,
StateKey, Owner, Optionen, Baseline und No-op bleiben überprüfbar. Der
Windows-Abbruch selbst enthält keinen Callframe; seine Behebung und die
vorher unausgeführten Fälle waren vor dem vierten Gesamtlauf noch offen.

Der [vierte gemeinsame Remote-Lauf](https://github.com/b1ue-man/smart-explorer/actions/runs/37261440798)
prüft `7b3cda8a92a957f85e4c65dfe092c8deeb0f1335`. Am 2026-10-05 ist er
vollständig als fehlgeschlagen ausgewertet. Sein kombinierter Report ist
gegen SHA-256 `d72c1025a69f59af914a3ac78fbf4efbd84a24aff102df74e0d25ea2160069de`
verifiziert; die Linux-/Windows-/Gerätereports wurden ebenfalls vor der
gezielten Diagnose gegen ihre API-Digests geprüft. Kein Privatprofil oder
Altworker-Arbeitsverzeichnis liegt in den nativen Reportarchiven.

Linux und Windows bestätigen C01/C03/C05/C06/C07, einschließlich Notebook,
Altbindung, Nested-Mirror, Retry, Schutz und der betroffenen Exec-Rechte- und
Cancellationgrenzen. Beide Hosts führen alle ausgewählten Namen exakt einmal
mit konsistenter terminaler Summary aus. Der Windows-Stackoverflow tritt
nicht mehr auf; die vorher fehlenden Drive-/Jobfälle sind jetzt ausgeführt.
Die separate tatsächliche veröffentlichte Desktop-Workerübernahme erfüllt
auf beiden OS ihren Byte-/Job-/Baseline-/Konflikt-/Retry-/No-op-Vertrag sowie
Prozessende, Autostart-Restaurierung und Entfernung des Privatroots.

C02 nennt auf beiden OS im erweiterten Drive-Roundtrip nur eine geschützte
Auslassung. Ohne konkreten Pfad oder Phase wird daraus kein Produktgrund
behauptet; derselbe Ablauf muss diese Diagnose präzisieren. C04 unter Linux
erreicht den normalen SFTP-Ablauf und meldet dort eine unklare
Replacementdestination für `.obsidian/preferences.json`. Der SFTP-Altjob in
C08 schließt dagegen Seed, Änderung, Reload beider Endpunkte, erhaltenen
StateKey/Baseline und No-op erfolgreich ab. C04 auf Windows scheitert im
normalen gespeicherten UNC-Resolver an der generischen Portprüfung. Der
bestehende `Protocol::Share`-Portwert `0` gehört zum Credentialaccount, wird
aber von WNet nicht als TCP-Port benutzt. Der Connector erhält diesen
Vertrag; andere TCP-Provider behalten die positive Portprüfung.

Die SFTP-Anschlusslesung belegt einen nicht abgeschlossenen Intent nach
verifizierter erfolgreicher Veröffentlichung. Der nächste tatsächliche
Providerlauf enthält bereits eine neue legitime Gegenänderung; Recovery
vergleicht diese gegen den zurückgelassenen alten Intent. Diese Sourcekette
erklärt den Befund, ohne einen SFTP-ID- oder Digestvertrag zu lockern. Der
Fix schließt ausschließlich bestätigten eigenen Publish mit exaktem Cleanup
ab; alle Lost-ACK-/Fremdbyte-/Backup-/Baselineschutz-Orakel bleiben bestehen.

C08 erreicht auf Windows beide normalen Direct-Opens und scheitert danach
beim tatsächlichen `open_write`-/Flush-Abschluss vor dem Jobstart. Die
Anschlusslesung belegt einen fehlenden Creator-Ticket-Purpose für den
exklusiven `*.se-daemon-<16lowerhex>`-Stage in derselben vorhandenen
Peerbackend-Ownershipkette; keine verlorene Backendinstanz ist bewiesen.
Der eigene isolierte reversible-Fallback-Fall verwendet zudem einen
ungültigen Recovery-Sibling. Seine Fixture muss den tatsächlichen generierten
Siblingvertrag benutzen, ohne die produktive Validierung zu lockern.

C09 erzeugt unter der unveränderten veröffentlichten APK den echten Job,
Baseline und Altversionsbytes erfolgreich. Die erste aktuelle `sync.jobs`-
Abfrage führt die dokumentierte RV1-Konfigurationsmigration aus; deshalb
scheitert die bisher pauschale Bytehashprüfung dieser einen Jobdatei.
Die korrigierte Abnahme erfasst alle echten historischen Konfigurationskeys,
einschließlich mehrfacher Ignorewerte und nicht per API exponierter Settings,
und erlaubt ausschließlich die belegten RV1-Zusatzkeys und Deleteguard-Deltas.
Alle anderen alten Persistenzhashes und Backupbytes bleiben strikt. Der
normale erste No-op muss die tatsächliche Job-/Replica-Ownerbaseline mit
denselben bestätigten Records erzeugen; Config-/Ownerbaseline-/Journalhashes
bleiben beim folgenden Force-stop erhalten. Die restlichen Gerätephasen sind
für diesen Kandidaten nicht ausgeführt und nicht als Pass bestätigt.

Der exakt gebundene Remote-Formatpatch
`a63b727bd56856d0912087f510d55203bbb7abb73762cea602c7d5dfeb20cf98`
ist statisch übernommen; sein Report enthält keine Größenverletzung. Die
konkreten Folgekorrekturen werden gemeinsam committed/gepusht und ausschließlich
durch denselben vollständigen Suite-Einstieg bestätigt. C10 nennt weiterhin
fehlende `SE_DRIVE_TEST_CLIENT_ID` und `SE_DRIVE_TEST_REFRESH_TOKEN`; tatsächliche
Google-Abnahme und terminaler Release bleiben offen.

Die Folgekorrekturen des vierten Laufs sind implementiert und gegen die
aktuellen Verträge selbst geprüft. Der erfolgreiche Replacementpublish
schließt nach frischer Byte-/ID- und Namespaceprüfung seine bekannten
Recovery-Slots und zuletzt den privaten Intent ab. C02 behält seine strikte
Auslassungsassertion und nennt zusätzlich die genaue Flowphase und sämtliche
geschützten Pfade samt Grund; seine Laufzeitursache ist weiterhin offen.
Der Peerledger erfasst jetzt auch exklusiv angelegte daemon-Stages.
C08 verbindet erfolgreiche ACKs, fremde Ledger/IDs, verlorene ACKs und
Leasewechsel mit dem tatsächlichen Direct-Ablauf: Das regulär separat
geöffnete Backend darf den Stage nicht beanspruchen; die Bytes bleiben
erhalten und der ursprüngliche Creator veröffentlicht anschließend
erfolgreich. Diese Datei gehört danach zum selben Altjob, seiner Baseline,
dem Restart und No-op. Der ungültige Recovery-Sibling bleibt ausdrücklich
abgewiesen. Dieselbe Case-Discovery nimmt die neuen `old_jobs`-FQNs auf;
es gibt keinen neuen Suite-Einstieg und keine lokale Ausführung.
Die gemeinsame Laufzeit-/Formatbestätigung bleibt ausstehend.

Das Linux-Logartefakt enthielt außerdem das private Arbeitsverzeichnis der
Altworker-Fixture; das Windows-Archiv hatte dieselbe Grenze. Der Upload schließt
deren gesamte generierte Verzeichnisse jetzt wie das native Testprofil aus.
Die beiden eigenen Reportarchive `11319887997` und `11320438715` wurden nach
Sicherung der gezielten Diagnose entfernt (`DELETE` 204, folgende Sichtprüfung
404); kombinierte Summary und Joblogs bleiben erhalten. Ergebnis-/Bytehashbelege
bleiben im kandidatgebundenen Summary. Der Helper trennt zusätzlich seine
privaten Arbeitsdaten von ausgewählten hochladbaren Diagnosen.

Der [fünfte gemeinsame Remote-Lauf](https://github.com/b1ue-man/smart-explorer/actions/runs/37267225051)
prüft exakt `9b4bb4d2d27dc3fcdd14e96ebaf2ea23deb467ce` und endet am
2026-10-05 um 06:09:10 UTC vollständig fehlgeschlagen. Vor der Auswertung
sind die eigenen Reportarchive gegen die GitHub-API-Digests verifiziert:

| Bericht | Artefakt | SHA-256 |
|---|---|---|
| Linux | `11327977763` | `0a36f8e352c3c118f788640d16a880a88e14e18590ce2341efbd4da0c8fb39d5` |
| Windows | `11328427395` | `1b60fceeef260c69e7b2044b3561490f69b46590ec7c0fd7b7cadb1494dad09c` |
| Android-Gerät | `11327427264` | `e4516303b59673da4c12896330f6260f543799f5d789a20505919aa90448230b` |
| Gemeinsame Evaluation | `11327508974` | `46572f68d5f25f37fbb7195a2b960826d624397422f938e98afffb4dded173d2` |

Beide nativen Archive enthalten weder `native-profile/` noch private
`c08-legacy-worker-*/`-Arbeitsdaten. Beide Hosts führen sämtliche ausgewählten
FQNs exakt einmal aus und liefern konsistente terminale Ergebnisse; kein
Stackoverflow oder vorzeitiger Hostabbruch. C01/C03/C05 sind auf beiden
Hosts erfolgreich. Die separate tatsächliche veröffentlichte Desktop-
Workerübernahme schließt auf beiden OS einschließlich Zustands-/Backupbytes,
Wiederanlauf, No-op, Workerende, Autostart-Restaurierung und Privatcleanup ab.

C09 ist vollständig erfolgreich: veröffentlichte 0.5.169-APK, echter alter
Job, tatsächlich importierte Ownerbaseline und alte Sicherungen, normales
Update mit derselben Signatur, erzwungener Prozessneustart, Retry, Konflikt,
Gegenänderung, Versionswiederherstellung und No-op. Alle Phasen behalten
Job-ID `18db8b74fd40356f`. Die ursprüngliche Baseline mit SHA-256
`8bac57738c747440bbbc5e8bccdce42ce1f9f8c6aa81a9d7a8d021d768b2d803`
ist im tatsächlichen Ownerzustand übernommen; Force-stop erhält Konfiguration,
Ownerbaseline und Journal. Die letzten tatsächlichen Quell-/Zielbytes stimmen
gegen SHA-256 `a7a8f6a5e32faba69b9c41a320f1e93e02a3ce39c56ab14f825421574223fa19`
überein. Es bleiben keine unausgeführten Android-Gerätephasen dieses Ablaufs.

C02 nennt nun `Notebook`/`notebook` als Zielnamenskollision bereits beim
Drive↔Drive-Seed mit `KeyPolicy { fold_case: true }`. Der geprüfte Source
dieses fünften Kandidaten erbt noch den konservativen VFS-Default im
Driveadapter. Seine exakte Fähigkeit und die direkt betroffene alte
Spellingpolicy-Migration werden im selben Fixloop gemeinsam korrigiert;
alte Baseline-/Seitenschreibweisen dürfen nicht
verworfen oder neue Namen unbewiesen gepaart werden. Derselbe C02-Roundtrip
behält seine Literal-, Overwrite-, Backup-, Restore-, Reopen- und No-op-Orakel.

C04 unter Linux schließt die vollständigen tatsächlichen SFTP-, Key- und
SSH-Agent-Paare einschließlich Gegenänderung und No-op ab. Der nächste FTP-
Flow meldet `literal%20-file.txt` als `Unreadable`; der echte Server bestätigt
exakten STOR und dessen Bytes. LIST liefert ohne MLSx eine gröbere mtime als
SIZE+MDTM beim Stat; Checksumcapture verlangt zu Recht gleiche frische
Signaturen. Die Korrektur vereinheitlicht reguläre adressierbare Kinder mit
einem gemeinsamen nichtrekursiven Probe. Unsupported/550 und Links behalten
ihren geschützten Eltern-LIST-Vertrag. Windows schließt den UNC-Flow ab;
der folgende mapped-Vergleich scheitert am Textvergleich äquivalenter nativer
Separatorpfade. Nur dieses Fixture-Orakel wird auf native Pfadgleichheit
ausgerichtet. Spätere Providerflows sind noch nicht als erfolgreich bestätigt.

C06/C07 scheitern jeweils an zwei bisherigen Fixtureannahmen, die nach
erfolgreichem Restore/Publish noch einen offenen Intent erwarten. Der nun
korrekt abgeschlossene Intent ist dort nicht mehr vorhanden. Das korrigierte
Orakel prüft tatsächliche Slot-/Intent-Abwesenheit, restaurierbare Backupbytes
und erhaltene Baseline/Checkpoint; echte Fehler-/Lost-ACK-/fremde Creator-
und Ownerorientierungsfälle behalten ihre strikten Recoveryerwartungen.

C08 unter Windows bestätigt exklusive daemon-Stageerstellung mit ACK,
Abweisung des separat geöffneten Backends, Original-Creator-Publish und
reguläres Providerwrite. Der danach gestartete alte Job scheitert an seinem
gültigen `sync-replica`-Unique-Stage als `untracked`. Die Ownershipgrenze muss
die gemeinsame enge Unique-Stage-Grammatik verwenden; Namensform allein
erteilt weiterhin keinen Creatorbeweis. Der tatsächliche alte Job, Baseline,
Restart und No-op bleiben im selben C08-Ablauf verbindlich.

Der kandidatgebundene Remote-Formatpatch mit SHA-256
`08b1ef792d85d79070c2921696a0b0184d16da59eeaed68993d84b8f840307d4`
ist nach exaktem Source-/Pfadabgleich statisch übernommen. Kein Größenverstoß
steht im Formatreport. Alle konkreten Folgekorrekturen werden zusammen
committed/gepusht und durch denselben vollständigen Suite-Einstieg geprüft.
C10 meldet weiterhin fehlende `SE_DRIVE_TEST_CLIENT_ID` und
`SE_DRIVE_TEST_REFRESH_TOKEN`. Echte Google-Abnahme und terminaler Release
bleiben offen; keine lokale Build-/Test-/Releaseausführung wurde gestartet.

Eine erneute GitHub-API-Prüfung am 2026-10-05 während dieses Fixloops nennt
weiterhin genau beide erforderlichen Secret-Namen als fehlend; auch das
optionale Client-Secret ist nicht eingerichtet. Die Abfrage liest nur Namen,
keine Secretwerte. Die bereits angefragte Autorisierung bleibt erforderlich.

Die konkreten Folgekorrekturen sind am 2026-10-05 vollständig umgesetzt und
von ihren jeweiligen Umsetzern selbst geprüft: `b6d6bf7e` vereinheitlicht
FTP-Dateisignaturen und korrigiert ausschließlich das native mapped-Orakel;
`2c8062fe` erhält Creator-/ACK-Belege für alle gültigen Unique-Stages;
`4a40e514` verbindet erfolgreiche Publikation mit tatsächlichem Cleanup und
bewahrt alle realen Fault-/Lost-ACK-Orakel; `f9283c2b` meldet die exakte
Drivefähigkeit und ergänzt historische C02-Gesamtabläufe; `1bd8140c`
integriert die belegte alte Spellingpolicy über Full/Preview/Incremental/
Recorded einschließlich Löschlebenszyklus, Pending-Counterparts und der
bestehenden Merge-Recoveryidentität. Die statische Rust-Parseprüfung des
gesamten neuen Fixkandidaten findet keine Syntaxfehler oder Rohgrößenverstöße.
Format, Kompilation und tatsächliche gemeinsame Konvergenz bleiben durch
denselben Remote-Einstieg zu bestätigen; diese Quellprüfung ist kein
Laufzeitpass und autorisiert noch keinen Release.

Der [sechste gemeinsame Lauf](https://github.com/b1ue-man/smart-explorer/actions/runs/37276864627)
prüft `0f672429d1f6b933ebd36d2a9878576bd8969b6c` und ist am 2026-10-05
vollständig fehlgeschlagen ausgewertet. C01/C03/C06/C07 erfüllen auf beiden
Desktophosts ihre Orakel. Der moderne vollständige Drive-Literal-/Case-/ID-/
Backup-/Restore-/Restart-Roundtrip schließt ab. Linux bestätigt C08 einschließlich
des tatsächlichen normalen Direct-/Room-Altjobs; Android C09 bestätigt den
vollständigen veröffentlichten Altzustand über Update und Force-stop hinweg.
Beide tatsächlichen veröffentlichten Desktopworker werden erfolgreich übernommen.

Konkrete verbleibende Befunde desselben Fixloops:

- C02: Zwei historische Drive-Fixtures setzen einen nicht vorhandenen modernen
  Dirsidecar-Eintrag voraus und behaupten für den Altstand eine Literalschreibweise.
  Tatsächlich speichert der alte Checkpoint gefaltete Dirplanungsschlüssel.
- C05: Die Migration wertet diesen alten Dirkey und einen passenden bestätigten
  Baseline-Vorfahren als mehrere Literalanker. Dieselben beiden vollständigen
  Recorded-/Incremental-Flows verlangen weiterhin Erfolg und alle Erhaltungsorakel.
- Linux C04: Reale SFTP-/Passwort-/Key-/Agent-, FTP-/FTPS- und SMB-Flows schließen
  ab. Der gespeicherte DAV-Childroot bekommt authentifiziert 301; ureq macht daraus
  ein anonymes GET mit 401. Korrektur erfolgt an der Produkt-Metadatengrenze.
- Windows C04/C08: UNC und mapped sowie Direct-Write/Creator/ACK funktionieren.
  Die anschließende Versionsgrenze meldet `Pfad ist nicht freigegeben`.
  Private Sharearchive und tatsächliche Autorisierungsfehler bleiben geschützt;
  der bestehende private Sicherungsfallback muss über die wirklichen Wrapper wirken.
- Windows C04: Die zwei neuen FTP-Metadaten-Fixtures erreichen keinen Login.
  Ihr eigener Listener-/Setupvertrag wird am konkreten Befund korrigiert.
- C10: Die erforderlichen OAuth-Eingänge fehlen weiterhin; kein Google-Lauf
  wird als erfolgreich oder übersprungen ausgegeben.

Linux-, Windows-, Device- und Gesamtreport sind an exakt diesen Kandidaten
gebunden. Ihre Archive stimmen mit den GitHub-API-SHA-256-Digests überein;
private Profile und Altworker-Arbeitsdaten sind nicht enthalten. Beide Hosts
beenden ihre eigenen Provider und Altworker regulär. Der geprüfte Remote-
Formatpatch mit SHA-256
`83ca19b61341ed4c6ec8601f2563c41d0a2ce14b5854221f68ed225ce40c07ea`
ist nach vollständigem Laufende und exaktem Source-/Pfadabgleich statisch
übernommen (`1c0c5e87`). Der Remote-Formatreport nennt keinen Größenverstoß.
Alle Korrekturen werden vor dem nächsten Aufruf gesammelt umgesetzt, committed
und gepusht; bestätigt wird ausschließlich derselbe vollständige Suite-Einstieg.


Die Folgekorrekturen des sechsten Laufs sind quellenfertig und jeweils vom
Umsetzer selbst geprüft: `59d1d380` erhält authentifiziertes PROPFIND bei der
zulässigen Collectioncanonicalisierung und korrigiert nur den kontrollierten
Windows-FTP-Fixturestream; `082e5fab` rekonstruiert die tatsächliche historische
Viermap-/Baseline-/gefaltete Dirbasis. `19134797` erhält den privaten
Sharearchivvertrag typisiert über Peer, IPC, Agent und Cache, mit frischer
Rootprüfung; `4d443a2c` erhält die bewiesene alte Ordnerhistory in Full,
Preview, Incremental und bestätigten Completedereignissen. Fehlgeschlagener
Remove behält seine Basis; erfolgreicher Retry und Neuerstellung ohne alten
Alias bleiben im unveränderten C05-Gesamtablauf verpflichtend.

Die eine Suite ist erst nach Abschluss aller Implementierungen ergänzt:
C04 nimmt die vorhandenen exakten DAV-Mutation-/PUT-No-follow-FQNs auf.
Die neuen metadata-/Private-Policy-/normalen Direct-/Room-Versionen-/IPC-Fälle
werden über dieselben vorhandenen C04-/C05-/C08-Prefixe entdeckt. Der echte
Direct-/Room-Versionsablauf fordert Jobowner, restaurierbare Byte-Sicherung,
abgewiesenen fremden Owner, unveränderte Restorebaseline, Wiederöffnung,
Konvergenz und No-op; bestehende Provider- und Altjoborakel sind unverändert.
Die am 2026-10-05 um 09:09 UTC erneut allein anhand von Namen geprüften
GitHub-Secrets enthalten weiterhin weder Client-ID noch Refresh-Token für
C10. Die Quellenfertigkeit ist kein Laufpass und noch keine Releasefreigabe.

Die statische Rust-AST-Prüfung des gesamten Folgefixkandidaten findet keine
Syntaxfehler; alle betroffenen Quellen liegen roh unter 500 Zeilen/50 KiB.
Der Suite-Einstieg lässt sich als Python-AST vollständig parsen. Diese Checks
rufen weder Compiler noch Tests auf; formatierte Größen, Kompilation und
Verhalten werden ausschließlich im selben Remote-Workflow bestätigt.

Der Rootgraph ist nach Abschluss aller nativen Änderungen vollständig neu
extrahiert und geclustert. Sein Manifest entspricht exakt dem aktuellen
`native/src`-Korpus; es gibt keinen separaten oder partiellen nativen Graph.
Die Änderungen werden als ein Kandidat gepusht und ausschließlich durch den
bestehenden vollständigen Remote-Suite-Einstieg bestätigt.


## Siebte gemeinsame Abnahme und konkrete Restgrenzen

Lauf [37289323834](https://github.com/b1ue-man/smart-explorer/actions/runs/37289323834)
auf `ffba5c54323ef093b63b8d8407ffcc612847ebc8` ist am 2026-10-05 um
10:20:24 UTC vollständig abgeschlossen und ausgewertet. Beide Hostberichte
bestätigen exakte Ausführung und vollständiges Provider-/Workercleanup.
Android-Build und tatsächlicher Gerätelauf bestehen; alle Berichte sind
an denselben Kandidaten und dieselben vier Stufen gebunden.

C01–C03 und C06–C08 bestehen auf beiden Hosts: Notebook, aktuelle sowie
historische Drive-Namen/IDs/Backups/Restore/Restart, geschützte Publish-/Retry-
und Link-/Ownergrenzen, sämtliche alten Desktopjobs und realer alter
v0.5.169-Workerwechsel. C05 bestätigt inkrementelle historische History und
normale Direct-/Room-Privatversionen samt Ownerabwehr, Restore, Reopen und
No-op. C09 bestätigt tatsächliches Altappupdate, Force-stop und Retry mit
beibehaltenem Job-/Ownerzustand, Konflikt, Backuprestore und Gegenänderung.
Diese Erfolge ersetzen keinen vollständigen C04-/C05-/C10-Pass.

Konkrete Restgrenzen: Linux C04 scheitert bei wiederholtem MKCOL einer
vorhandenen `.obsidian`-Collection an HTTP 301; frische Metadaten und erste
Anlage sind bestätigt. Windows C04 schließt seine Providerpaare ab und
scheitert anschließend an der leeren ZIP-Fixture-Scanwurzel. C05 erreicht
auf beiden Hosts den aufgezeichneten Lost-ACK und scheitert am Kindvergleich
`Other` gegen `ConnectionReset`; die vorausgehenden alten Dirhistoryphasen
bestehen. Die nachfolgenden Recorded-Recoverybeweise sind damit noch offen.
C10 nennt unverändert fehlende `SE_DRIVE_TEST_CLIENT_ID` und
`SE_DRIVE_TEST_REFRESH_TOKEN`; die Autorisierungsfrage bleibt offen.

Alle hochgeladenen Evidenzarchive sind gegen GitHubs SHA-256 geprüft und nur
an zulässigen relativen Textpfaden ausgewertet: Linux `11338665176`
(`dd792300d1ee2b3ec0e49530b07f8e7608394e14805faa812ea3ce1bf7c50f26`),
Windows `11339090166`
(`d4617d237da687b8ce23fcf0cfea4ff185be64e6957b9164554115fd4e19728f`),
Gerät `11336278102`
(`f2e1b7d4a5578691995112529e9f20f3edbc8bf97e5e3f108f3e265c966e20a2`),
Gesamtauswertung `11338617424`
(`0762d262a04c35abc8e1dc08326b6a43a6c49ec6855d76876765edef25a33244`).
Es wurden keine Profile, Credentials, APKs oder Developmentbinaries lokal
zur Ausführung übernommen.

Der exakte Remote-Formatpatch
`64ff5adc09922d2bd99b8982a8c2059d8e6a4d5fcedac31db2a0f2bbf33450ea`
ist nach sauberem HEAD-, Report-, Digest- und Sourcepfadabgleich statisch als
`135bf3b6` übernommen. Der Formatreport enthält keine Größenverstöße.
Die zweistufige Anschlussplanung steht bei M3.C04-7 und M2.C05-7;
keine weitere Reviewrunde, Suite oder vorzeitiger Release.


Der bereits veröffentlichte Ausgangsstand bleibt v0.5.171. Am 2026-10-05
ist die lokale Installation statisch gegen den aktuellen
[GitHub Release](https://github.com/b1ue-man/smart-explorer/releases/tag/v0.5.171)
abgeglichen: `se` (`8b18239450e130df5f2ee73f395bf9986786bf883e93b166499805d88e470238`)
und Share-Server (`91e7b177e875784864cacc4e64e89f8987985630c82cbcc2cadae2b1d85eef45`)
stimmen mit dessen tatsächlichen Assets überein. Cargo-/Feedversion, Installer,
Tag `2641fe9ed488754f361c26570ff5fec6aaa6d953`, alle sechs Desktoppayloads
und die Android-APK samt Sidecars passen zu diesem veröffentlichten Stand.
Dies ist kein M6-Nachweis für den noch offenen Kandidaten 0.5.172.
C10-Testsecret-Namen erneut um 10:29 UTC geprüft: erforderliche Client-ID
und Refresh-Token fehlen weiterhin; die bestehende Autorisierungsfrage
bleibt offen. Keine Credentials oder lokalen Binaries wurden ausgeführt.


Die Folgekorrekturen sind quellenfertig und vom jeweiligen Umsetzer selbst
geprüft: `5ad57a22` adressiert MKCOL direkt mit der kodierten Collection-URL
und erhält einzelne Mutation, Pooling und alle tatsächlichen Fehlergrenzen.
Belegte Nicht-Collectionnamen werden nur nach frischem Originalstat als
belegt behandelt; ein fehlgeschlagener Stat behält seinen tatsächlichen
Fehler. ZIP verwendet `/` und verlangt weiterhin read-only-Source, genaue
Bytekopie und No-op einschließlich null Löschungen. `cffaec5d` prüft den
konkreten Recorded-Fehlertext und konsumierte Injektion sowie zusätzliche
Intent-/Pairlock-/B-Literalbindung; alle bisherigen Recoveryorakel bleiben.
Die bestehenden FQNs und Discovery bleiben erhalten.

Erst nach vollständiger Umsetzung ist der eine Suite-Einstieg ergänzt:
C04 enthält zusätzlich die vorhandenen genauen FQNs für einzelne MKCOLs,
Mutationpooling und Congestion/Retry-After. Die neuen Collectionguards
werden durch denselben bestehenden C04-Prefix entdeckt. Die gesamte
Provider- und Altjobabnahme bleibt ein gemeinsamer Remote-Aufruf.


Der vollständige Folgefixkandidat besteht die statische Rust-/Python-AST-
prüfung ohne Compiler-/Testaufruf. Alle geänderten Rustquellen liegen roh
unter 500 Zeilen und 50 KiB; formatierte Größen und tatsächliches Verhalten
bleiben Aufgabe derselben Remote-Abnahme. Der Rootgraph ist nach allen
nativen Änderungen vollständig neu extrahiert und geclustert. Sein Manifest
entspricht exakt dem gesamten aktuellen Nativekorpus, ohne separaten oder
partiellen Graph. Quellen, Suite, Dokumentation und Graph werden gemeinsam
als ein ungetaggter Kandidat gepusht.

## Achter Lauf: vollständige Auswertung und verbleibende Abschlussgrenze

Der [achte Workflow](https://github.com/b1ue-man/smart-explorer/actions/runs/37297823833)
prüft exakt `8a1a36caa1486d8243b40ba827cd8bcc11e312cf` und endet am
2026-10-05 um 11:38:41 UTC vollständig fehlgeschlagen. Sämtliche Jobs sind
abgeschlossen; Windows, Android-Build und echter Android-Geräteablauf sind
erfolgreich. Alle vier Summaryreports besitzen exakt denselben Kandidaten.
Native exakte Ausführung, eigene Providerbereinigung und alter
veröffentlichter Desktopworkerwechsel sind auf beiden Hosts bestätigt.

Linux bestätigt C01/C02/C03 sowie C05/C06/C07/C08, insbesondere den nun
vollständigen historischen Recorded-Lost-ACK-Wiederanlauf mit unveränderten
Baseline-/Intent-/Backup-/Restore-/Restart-/No-op-Orakeln. Die kanonischen
DAV-Collection-, Redirect-, Pooling- und Congestionguards bestehen. Windows
bestätigt zusätzlich seinen vollständigen C04 einschließlich ZIP-Quelle.
Der echte C09-Updateablauf erhält dieselbe alte Job-ID über alte App,
Update und Force-stop/Restart; endgültige Quell-/Zielhashes sind gleich,
der abschließende No-op meldet keine Änderungen oder Fehler.

Linux-C04 erreicht erstmals nach den DAV-Korrekturen FTP→FTPS bei
`pair-051`. STOR von `.obsidian/preferences.json` in seine eigene Stage
liefert 426. Der echte Server meldet um 10:58 UTC fehlenden SSL-Abschluss
und einen fehlgeschlagenen 37-Byte-Upload. Der ganze Providerflow ist
damit offen; vorausgegangene Providerpaare oder spätere unreached Phasen
werden nicht als vollständige Matrixabnahme behauptet. Die aktuelle
zweistufige Primärrecherche und M3.C04-8 behandeln beide FTP-Uploadpfade,
die Datenbesitzgrenze und weiterhin strikte tatsächliche Abschlussantworten.
Der angefragte C10-Zugang fehlt unverändert: Required-Secret-Namen um
11:11:57 UTC gelesen; Client-ID und Refresh-Token sind nicht vorhanden.

Evidenzarchive jeweils gegen GitHubs SHA-256 geprüft und nur als zulässige
relative Textpfade ausgewertet:

- Linux `11342491663`: `7a9a25984299014173b8c182562dda0f27a8ff5c45af72a8a69a6c1beabd178f`.
- Windows `11343305976`: `2e6f6c6f71bef5a39f0505d38d027d14d4a01ce4e07bc9aad8b86056177a1170`.
- Gerät `11340419560`: `88f3d9e439e37745381ed8c302074e6628e8fb30ee558ab4d448324b1978fc2e`.
- Gesamtauswertung `11342702141`: `a7244042806ed74ad68250793c2dd5e3cd3d38ba7fb539bec7e36d8a9007e17b`.

Der exakte Remote-Formatpatch
`8b245bf69799690976beb23c55d975b6817b60037c70926cebdc42163d13f78c`
ist nach sauberem HEAD-, Report-, Digest- und Sourcepfadabgleich als
`91f46ce3` übernommen. Er betrifft ausschließlich die neue
DAV-Collection-Testdatei; sein Report enthält keine Größenverstöße.
FTPS-Korrektur, ihre erneute gemeinsame Laufbestätigung und tatsächlicher
Google-Zugang bleiben vor dem einen terminalen Release verpflichtend.

Die zusammenhängende FTPS-Korrektur ist als `e1e7ae93` quellenfertig.
`UploadData` wird von beiden bisherigen Writern verwendet. Nur der tatsächliche
`DataStream::Ssl` erhält einen zusätzlichen `try_clone`-Besitzhandle bis nach
TLS-Drop und der echten FTP-Abschlussantwort. Plain FTP bleibt bei seinem
gewöhnlichen EOF. Expliziter Flush, fallibler Besitz und terminale Antwort
werden ausgewertet; 451/452/552 haben Vorrang vor einem Datenfehler. Auch
ein positiver FTP-Code heilt keinen belegten Flush-/Besitzfehler. Es gibt
keine neue Wartephase, keinen vorzeitigen Shutdown oder automatischen STOR-Retry.
Der Umsetzer hat die exakten öffentlichen/private API-Grenzen selbst geprüft;
keine Dependency oder öffentliche VFS-/Provider-/Locator-Schnittstelle geändert.

Neue FQNs im bestehenden C04-Präfix:

- `ftp::data_finish::tests::sync_reliability_task_provider_upload_data_error_still_consumes_terminal_refusal`.
- Linux: `ftp::sync_reliability_task_provider_ftps_tests::sync_reliability_task_provider_ftps_both_writers_confirm_bytes_after_tls_close`.

Der erste Fall verlangt trotz Datenfehler die echte terminale Antwort und
korrekte Zielablehnung, der zweite benutzt dieselbe reale gespeicherte
FTPS-Auflösung mit beiden Writern, leeren/kleinen/größeren Bytes, weiterem
Flush, Wiederöffnung und tatsächlichen Zielbytes. Die vollständige Matrix
und sämtliche bestehenden Orakel sind unverändert. Nach allen Quelländerungen
ist ausschließlich der eine Suite-Einstieg um die vorhandenen genauen FQNs
für Uploadlänge, Diskspool, Nichtwiederholung, ungesendeten Writerdrop,
Suspect-Reconnect und FTPS-Datentimeout ergänzt. Keine separaten Läufe.
Required-Secret-Namen erneut um 11:52:50 UTC geprüft: Client-ID und
Refresh-Token fehlen weiterhin; keine Secretwerte gelesen oder ausgegeben.

Alle geänderten Rustquellen bestehen die statische AST- und rohe Größenprüfung;
der eine Python-Einstieg ist statisch geparst. Kein Compiler, Formatter oder
Test ist lokal ausgeführt. Nach sämtlichen nativen Änderungen ist der Rootgraph
vollständig extrahiert und geclustert: Manifest exakt gleich dem gesamten
Nativekorpus (1825 Dateien), 32224 Nodes, 81174 Kanten und 1121 Communities.
Kein Teilgraph oder verschachtelter Nativegraph bleibt. Dies ist der vollständige
Quellkandidat für die erneute gemeinsame Remote-Bestätigung, kein M5-Pass.
