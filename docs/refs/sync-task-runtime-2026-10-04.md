# Sync-Abnahme: tatsächliche Server und alte Android-Appdaten

Primärquellen geprüft am 2026-10-04; betroffene Quellverträge zuletzt am
2026-10-05. Diese Ref beschreibt die betroffenen Produktionsverträge und
ihre Remote-Abnahme. Produktive TLS- und Signaturregeln bleiben unverändert.

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

Der fünfte Remote-Lauf bestätigt diese daemon-Ownershipgrenze im tatsächlichen
Direct-Transport. Der anschließende Job erzeugt außerdem einen exklusiven
Replica-Stage mit Purpose `sync-replica`. Der Creatorledger benutzt deshalb
die gemeinsame enge `vfs::is_unique_stage`-Grammatik für
`<nonempty file>.se-<purpose [a-z0-9-]+>-<16lowerhex>` statt einer
Purpose-Aufzählung. Das ist nur die Zulassung zur bestehenden Creator-
Erfassung; exklusiver WriteNew, erfolgreicher Writer-ACK, eigener Ledger,
Bindung und frischer Metadatensnapshot bleiben die Ownershipbeweise.
Die breitere `is_staging_name`-Erkennung und der separate Upload-Discardguard
erteilen keine zusätzlichen Lösch- oder Publishrechte. Neue Grenzfälle
benutzen den tatsächlichen Unique-Stage-Generator einschließlich Replica
und unveränderter Literalpräfixe; die Remote-Suite entdeckt sie im selben C08.

## Private Peer-Sicherungen über IPC

Sourceabgleich 2026-10-05: Share hält `.se-versions` ausdrücklich privat
(`fs_policy::private_name`). Der bisherige private Appdata-Fallback in
`version_listing::managed_sync_root` hing an `PermissionDenied`.
`agent_error::kind_from_message` erkennt die
englischen Texte; das deutsche `Pfad ist nicht freigegeben` ohne OS-Code
bleibt `Other`. Der tatsächliche Windows-C04-/C08-Lauf meldet daher nach
erfolgreichen normalen Transfers einen Fehler an der Versionsgrenze.

`vfs::VersionArchivePolicy::{Provider, AppPrivate}` erhält jetzt den Archivvertrag
über reale Peer-, IPC-, Agent- und Cache-Handles. Die reine
`vfs::version_archive_policy(backend) -> VersionArchivePolicy`-Abfrage öffnet
keinen Agent und führt keinen RPC aus. Peer und IPC-Identitätsstub liefern
`AppPrivate`; Agent und Cache delegieren. Ein unbekannter Provider bleibt
beim strikten normalen Archivzugriff. `version_provider_policy::uses_provider_archive`
prüft für die private Wahl Cancellation und den tatsächlichen Root über
`vfs::sync_stat`; damit reicht ein alter warmer Browsingcache nicht aus.
Der Root muss ein gewöhnliches Verzeichnis sein. Verweigerte Providerarchive
und deren Kinder, widerrufene Freigaben und fremde Owner bleiben Fehler.
Backupbytes bleiben über die vorhandene durable private Kopie restorable;
Listing/Retention und Restore behalten dieselbe Pair-/Owneridentität.
Der zusätzliche C05-Ablauf benutzt normale gespeicherte Direct-/Room-Locators
mit eigenem Jobowner und verlangt Save, Listing, abgewiesenen fremden Owner,
Restore bei unveränderter Baseline, Wiederöffnung, Konvergenz und No-op.
Der Laufnachweis gehört ausschließlich zur nächsten vollständigen Remote-Abnahme.

## Alte Spellingpolicy und tatsächliche Seitenpfade

Quellabgleich am 2026-10-05: Die exakte Drivefähigkeit verändert die gemeinsame
`KeyPolicy` nur, wenn beide Seiten exakte Pfade unterstützen. Vorhandene
`StateSpellings` enthalten vier Maps mit den wirklich gespeicherten Datei-
und Ordnerschreibweisen je Seite. `state_spelling_policy::validate` prüft
alle Komponenten mit `SyncRelativePath` und erkennt alte gefaltete Schlüssel.
Die Migration liest denselben StateKey über `checkpoint_journal::Journal`.
Die Baseline hält tatsächliche logische Relativnamen. Dagegen normalisiert
`Checkpoint::planned` die `dirs_add`/`dirs_remove` schon vor Journalreplay
mit der damaligen `KeyPolicy`; die historische `.dirs.json` enthält deshalb
gefaltete Planungsschlüssel, keine zusätzliche Literal-Schreibweise.
Ordnerbelege müssen diese Existenzinformation von den wirklich gespeicherten
Seitenslots und Baseline-Vorfahren unterscheiden. Ein Schlüssel `OLDTREE`
und der belegte Vorfahr `OldTree` sind nicht zwei konkurrierende Literalnamen.
Tatsächlich mehrere Baseline-Anker oder widersprüchliche Seitenslots bleiben
mehrdeutig und geschützt. Dieser Sourcevertrag ist am 2026-10-05 gegen
`checkpoint_run.rs`, Journalreplay und den sechsten Remote-Lauf geprüft.
Die implementierte `state_spelling_history::DirectoryHistory::load(names,
recorded, keys)` bildet ausschließlich diese belegten alten Beziehungen auf
die aktuellen logischen Dirkeys ab. Full-/Incremental-Läufe schreiben ihren
Keydelta über `CheckpointSink::planned(Frame)` unter dem bestehenden Pairlock;
Preview bildet nur dieselbe readonly Sicht. Bei belegtem inkrementellem No-op
benötigt die reine Metadatenmigration keinen Zielwalk oder neue Indexgeneration.
`CheckpointSink::with_path_aliases` überträgt bestätigte DirCreated-/DirRemoved-
Ereignisse anhand ihrer wirklichen Seite auf den gemeinsamen logischen Key.
`StateSpellings::applied_directory_history` retiert den Alias erst, wenn der
vorher bestätigte Key im Completedzustand entfernt ist. Fehlgeschlagene oder
geschützte Removes behalten die Beziehung; aufgezeichnete Datei-Elternslots
bleiben bei fehlgeschlagenen Childaktionen für den Retry verfügbar.
Für eine bereits gespeicherte ungelöste Relation ohne Baseline gelten nur
ihre tatsächlichen Seitenslots als Beleg. Mehrdeutige oder widersprüchliche
Records werden nicht in eine erfundene Zuordnung umgewandelt.

`keys::PathAliases` ist eine reine, seitenspezifische Zuordnung zwischen
logischem Schlüssel und tatsächlichem Pfad. Ein belegter Ordnerpräfix gilt
für seine tatsächlichen Kinder; neue ähnliche Namen werden dadurch nicht
allgemein gefaltet. Die private Spellingdatei ergänzt für unterschiedlich
geschriebene Altpaare den logischen Anker. Laden und Vorschau schreiben diese
Datei nicht; Persistenz benutzt weiterhin den bestehenden Pairlock und den
privaten atomaren Store. Ungültiger Altzustand fällt nicht auf leere Maps
mit anschließend behaupteter neuer Basis zurück.

Vollscan, Vorschau und inkrementelle Planung benutzen dieselbe Zuordnung.
Ein optionaler Index ist nur verwendbar, wenn seine tatsächlichen Seiten-
Records nach dieser Zuordnung genau die vorhandene Baseline belegen.
Unsichere oder kollidierende Caches gehen zurück zum vollständigen Scan;
geschützte Teilscans werden weiterhin keine vollständige Indexgeneration.
Gespeicherte Konflikt-/Merge-/Resume-Pfade können ausschließlich eine exakt
aufgezeichnete alte Seitenschreibweise zurück auf ihren logischen Anker
führen. Normale neue Actions verwenden keine allgemeine Foldsuche.
Bezeichnet ein gespeicherter Literaltext zugleich eine neue kanonische Relation
und einen anderen belegten alten Seitenslot, ist diese Auswahl mehrdeutig.
Die Recorded-Grenze wählt dann keinen der beiden Owner; ihre vorhandenen
Bytes und Beziehungen bleiben erhalten. Die reguläre neue Planung verwendet
weiterhin ihre exakten kanonischen Schlüssel.

`apply_actions` übergibt beim Copy den tatsächlichen `target_rel`, beim Delete
den tatsächlichen `source_rel` an `apply_transaction`. Versionsmanifest und
Replacementbinding behalten damit die Literaladresse ihrer jeweiligen Seite;
Restore adressiert genau diesen Pfad. Offene Replacement-Rels werden für den
Schutz auf ihre belegten logischen und tatsächlichen Gegenstücke erweitert.
Eine erfolgreiche Recovery bestätigt Veröffentlichung und Cleanup; die
anschließende reguläre Planung beobachtet und schreibt die Baseline.

Eine bestätigte propagierte Dateilöschung beendet ihre alte Dateirelation;
eine danach neu angelegte Literaldatei erhält keine gelöschte Aliasbindung.
Die vorherige bestätigte Baseline und erfolgreicher Applyabschluss belegen
die Freigabe. Fehlgeschlagene Schritte behalten ihre Zuordnung; bestehende
Ordnerrelationen werden dadurch nicht umgewidmet.

Ein neu auftauchender unabhängiger Baum kann physisch auf den bereits
belegten Counterpart eines Altpaars treffen, etwa A `Notebook` ↔ B `notebook`
und danach zusätzlich A `notebook`. Ohne eigenständig registrierte Zieladresse
darf die Engine den belegten Slot nicht umwidmen. File-/Dirapply schützen
diese Gruppe, erhalten alte Bytes und Baseline und schließen unabhängige
Dateien ab. Nach einer regulären eindeutigen Umbenennung kann derselbe alte
Job vollständig konvergieren. Der Gesamtablauf unterscheidet diesen
Teilstatus ausdrücklich von vollständigem Erfolg.

## Aufgezeichneter Lost-ACK und öffentliche Fehlergrenze

Aktuelle Source und beide Hostberichte von Lauf `37289323834` geprüft am
2026-10-05. `engine_provider_fixture::lost_ack` injiziert beim bestätigten
Publish eine `ConnectionReset`-Fehlerursache. Der öffentliche gespeicherte
Auflösungsweg `resolve_recorded` delegiert an `single_recorded::apply_one`;
dieser rekonstruiert bei `report.stats.errors > 0` den tatsächlichen ersten
Applyfehler als `io::Error::other(String)`. Der belegte öffentliche Kindwert
ist somit `Other`; die rohe Providerursache ist kein API-Kindvertrag.

Das vorhandene historische C05-Orakel muss die konkrete injizierte Ursache
an dieser öffentlichen Grenze erkennen, einschließlich tatsächlich
konsumierter Injektion und veröffentlichter Gewinnerbytes. Alte Baseline,
literal gebundener ReplacementIntent, Schutz des Gegenstücks, Replayfreiheit,
Backupbytes, erfolgreicher Wiederanlauf, Restart und No-op bleiben strikt.
Eine bloße `is_err()`-Prüfung oder eine Änderung der Produktionsfehlerart
allein zur Anpassung an das bisher falsche Fixture-Orakel genügt nicht.
