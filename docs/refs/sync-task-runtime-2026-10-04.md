# Sync-Abnahme: tatsächliche Server und alte Android-Appdaten

Primärquellen und aktuelle Implementierung geprüft am 2026-10-04.
Diese Ref beschreibt ausschließlich die Remote-Task-Abnahme; sie ist kein
lokaler Build-/Testauftrag und keine Änderung produktiver TLS- oder Signaturregeln.

## Android-Update statt Neuinstallation

- Nachprüfung 2026-10-05: [`File.getCanonicalFile()`](https://developer.android.com/reference/java/io/File#getCanonicalFile())
  (API 1) liefert den kanonischen `File` für denselben Ort; unter Unix werden
  dabei symbolische Links aufgelöst. Dateisystemfehler können `IOException`
  auslösen und werden von der Instrumentation als Fehler weitergegeben.
  C09 verwendet den kanonischen bestehenden `Context.filesDir` als Basis für
  seinen eigenen Fixture-Root und relative Persistenzbelege. Die unveränderte
  v0.5.169 prüft in `LocalBackend::mkdir_all` alle Vorfahren strikt; der dritte
  Remote-Lauf belegt ihre Ablehnung des Android-Systemaliases `/data/user/0`.
  Die Fixture wird deshalb vor der normalen alten `sync.save`-API kanonisch
  angelegt. Die gespeicherten Endpunkte werden beim Update nicht umgeschrieben.
  Alte Appdata-Versionen entstehen im Tag über `std::fs::create_dir_all` und
  exklusives `OpenOptions::create_new` unter `smart_explorer/sync/versions_*`;
  ihr tatsächlicher Inhalt und ihre Erhaltung bleiben Pflichtorakel.
- [Android App Signing](https://developer.android.com/studio/publish/app-signing):
  ein APK-Update benötigt einen zur installierten App passenden Signaturschlüssel.
  Der automatisch erzeugte Debugschlüssel erfüllt das für eine veröffentlichte
  Release-APK nicht. App- und Test-APK erhalten für diesen Remote-Task explizit
  die vorhandene Release-Signatur; der normale Debugbuild bleibt unverändert.
- [Build Variants](https://developer.android.com/build/build-variants):
  `android.signingConfigs` definiert den Schlüssel, `buildTypes.debug.signingConfig`
  wählt ihn für den Development-Build aus. Die Auswahl erfolgt nur bei einer
  ausdrücklich gesetzten Task-Property und vollständig vorhandenen Secrets.
- [Advanced Test Setup](https://developer.android.com/studio/test/advanced-test-setup):
  das Instrumentation-APK hat ein eigenes Manifest und einen `<instrumentation>`-
  Eintrag; die Ziel-Package-ID/Runner werden aus installierter Metadatenantwort
  ermittelt. Standard-Testbuild ist debug. Eine Instrumentation kann den alten
  Zielprozess benutzen, sofern Signatur und verwendete öffentliche APIs passen.
- Aktueller Source: `android/app/build.gradle.kts`, `ReviewSyncTaskTest.kt`,
  `native/review-task-device.py`. Die alten APIs müssen zusätzlich gegen den
  Tag geprüft werden. Kein `run-as` für die nicht debuggable alte Release-App
  voraussetzen; Prepare läuft als passend signierte Instrumentation im Ziel.
- Statische Artefaktprüfung: v0.5.169-APK aus dem Tag stimmt mit dessen SHA-256
  überein und enthält `libsmart_explorer_android.so` für arm64-v8a und x86_64.
  Das belegt Eingangsbytes/ABI, noch keinen ausgeführten Updateablauf.

## SFTP und FTP/FTPS

- [atmoz/sftp README](https://raw.githubusercontent.com/atmoz/sftp/master/README.md):
  Usersyntax `user:pass[:e][:uid[:gid[:dir1[,dir2]...]]]`; Homes sind chrooted,
  daher liegt die schreibbare Fixture in einem Unterverzeichnis. Öffentliche
  Schlüssel kommen nach `/home/user/.ssh/keys/`; der Entrypoint erstellt
  `authorized_keys`. Hostkeys dürfen explizit gemountet werden.
- [delfer README](https://raw.githubusercontent.com/delfer/docker-alpine-ftp-server/master/README.md):
  `USERS=name|password|folder|uid|gid`, `ADDRESS`, `MIN_PORT`, `MAX_PORT` sowie
  `TLS_CERT`/`TLS_KEY`. Die Fixture verwendet die vorhandene Imageklasse.
- [delfer Entrypoint](https://raw.githubusercontent.com/delfer/docker-alpine-ftp-server/master/start_vsftpd.sh):
  nach Benutzeranlage wird ein übergebener Command ausgeführt. Für zuverlässigen
  Containerbesitz wird vsftpd explizit im Vordergrund gestartet, wie im
  vorhandenen `android/test-servers/servers.sh`. TLS-Zertifikat-/Keyoptionen
  müssen dabei im übergebenen Command erhalten bleiben.
- [vsftpd-Konfigurationsreferenz](https://manpages.debian.org/testing/vsftpd/vsftpd.conf.5.en.html):
  `ssl_enable`, `force_local_logins_ssl`, `force_local_data_ssl`,
  `rsa_cert_file`, `rsa_private_key_file`, `pasv_min_port`, `pasv_max_port`
  und `pasv_address` konfigurieren die echte Gegenstelle. `require_ssl_reuse`
  ist eine eigenständige Datenkanalbedingung und darf nicht versehentlich
  als Zertifikatsprüfung interpretiert werden.

## Testzertifikat und unveränderte Produktprüfung

Aktueller FTP-Source verwendet `webpki_roots::TLS_SERVER_ROOTS`, WebDAV den
ureq-Rustls-Agenten. Ein Eintrag im Runner-OS-Truststore beweist hier keine
Clientvertrauensstellung. Die isolierte Fixture-CA muss ausschließlich unter
`cfg(test)` in einen echten Rustls-RootCertStore aufgenommen und dem normalen
Transport übergeben werden. Kein unsicherer Zertifikatsverifier, keine
Produkt-Umgebungsvariable und keine gelockerte Produktionsprüfung.

Für selbst signierte FTPS-/HTTPS-Fixtures bleibt deshalb eine explizite
testbezogene Trust-Injektion nötig; die übrige gespeicherte Verbindung,
Resolver-, Auth-, Pfad- und Sync-Engine-Grenze bleibt produktiv. Die Abnahme
unterscheidet diese Vertrauensinjektion vom regulären OAuth-/Drive-Aufruf C10.
CA-/Ports-/Authwerte werden von der Suite erzeugt oder durch Readiness-
Kommandos entdeckt. Sämtliche Server/PIDs gehören genau diesem Remote-Lauf
und werden auch beim Fehler geschlossen.

## Sync-Literale und IPC-Identitätsfallback

Quellabgleich am 2026-10-05: `bisync/core/sync_relative_path.rs` akzeptiert
unveränderte Providerkomponenten einschließlich `:`, Backslash und `%`.
Slash bleibt Trenner; leere Komponenten, `.`/`..` und NUL bleiben verboten.
Das bestehende 32-KiB-Bytebudget des vorher benutzten Agent-Parsers bleibt
erhalten, eine zusätzliche pauschale Tiefenbegrenzung wird nicht eingeführt.
Bestehende konkrete SQL-/Incremental-/Scanbudgets bleiben an ihren Grenzen.
Apply und Persistenz verwenden denselben Literalvertrag; tatsächliche
native Namen werden zusätzlich vor nativen Zugriffen geprüft. Alte physische
Archive benutzen `legacy_backup_path.rs`; moderne Versionsmanifeste speichern
Providerliterale getrennt von opaken privaten Datenpfaden. Die Agent-Wire-
Grammatik in `agent_proto/core/relative_path.rs` bleibt unverändert.

`AgentBackend` delegiert drei reine Fallbackgrenzen an seinen inneren
Share-Identitätsstub. `UnavailableBackend` darf dafür keinen weiteren
identischen Agent öffnen: Die vorigen Overrides erzeugten einen unbedingten
Zyklus. Die vorhandenen VFS-Defaults liefern Literalchild, keine unbewiesenen
vorherigen IDs und `Ok(false)` ohne Mutation für den reversiblen Fallback.
Bisync prüft vorher seine Sicherung und nutzt danach die sichere Backend-
Promotion ohne Remove-Fallback; Recovery-Intent und Ownerbindung bleiben
erhalten. Alle echten IPC-RPC-Methoden und Autorisierungsprüfungen bleiben
unverändert. Die Remote-Fixture verwendet dafür die vorhandenen `Frame`,
`read_frame`/`write_frame` und `PROTO_VERSION` aus `agent_proto/mod.rs`; der
Hello-Versionstoken ohne Build-/Featurelabels aktiviert laut
`ServerFeatures::parse` keine Credit-/Service-/Extensionfeatures.

## Bestätigte Ersetzung und IPC-Stagebesitz

Quellabgleich am 2026-10-05: `replacement_publish` bestätigte bislang die
Veröffentlichung, ließ aber den vorher vorbereiteten privaten Intent offen.
Eine legitime spätere Gegenänderung konnte dadurch beim nächsten Recovery
als fremde Replacementdestination gelten. Der Abschluss benutzt nun dieselbe
enge `replacement_recovery::finish_published`-Grenze wie der bestätigte
Lost-ACK-Wiederanlauf: Destinationbytes und aufgezeichnete ID prüfen,
bekannte Stage-/Retained-Slots frisch prüfen und sicher entfernen,
Destination erneut prüfen und erst zuletzt den privaten Intent löschen.
Unklare oder fehlgeschlagene Veröffentlichung behält die Recoveryevidenz.
Kein Matcher, Backup-, Baseline- oder fremder Creatorvertrag wird gelockert.

`daemon::backend_stream` legt für reguläres IPC-Schreiben exklusiv einen
`*.se-daemon-<16lowerhex>`-Stage an und veröffentlicht ihn nach Writer-ACK.
`share::backend::peer_stages` muss diesen Purpose deshalb in seiner bestehenden
Reserve→Writing→ACK→Ready-Kette registrieren. Jeder normale
`PeerBackend::new_live` erhält über `PeerTransferState::default` einen eigenen
`StageLedger`; Worker desselben Opens klonen denselben Backend-Arc. Ein
separates reguläres Open erhält keinen Creatorbeweis allein durch gleichen
Pfad, Peer oder Lease. Die konkrete C08-Abnahme prüft diese Grenze am realen
Direct-Transport und danach im gespeicherten Altjob über Restart und No-op.
Besitzdiagnosen enthalten ausschließlich escaped Stagepfad und Ledgerphase.
