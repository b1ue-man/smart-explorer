# Sync-Verlässlichkeit – detaillierter Umsetzungs- und Abnahmeplan

Stand: 2026-10-05. Phase: Remote-Fixloop nach einmaliger Kritik; Gesamtabnahme offen.
Ziel ist die komplette Spec F1–F7, nicht allein das Entfernen einer Meldung.

## Meilensteine

### M1 Vollständige Drive-Kandidatensammlung

- F1/F2. Dateien: `gdrive/core/{resolution,promotion_api,file_list,backend}.rs`,
  neue schmale Module für paged Listings/Namensfilter und Root-Identität,
  Registrierung in `gdrive/mod.rs`, notwendige Felder in `state.rs`.
- Schnittstelle: dieselbe Sammlung von verschiedenen exakten Namen/IDs für
  Pfadauflösung und Mutation; alle Seiten bis zum tatsächlichen Ende. Filter
  andere Literalnamen vor Mehrdeutigkeitsentscheidung. Doppelte IDs werden
  zusammengeführt; widersprüchliche Metadaten lösen frische ID-Prüfung aus.
  Rejected Tokens/incompleteSearch starten einen vollständigen neuen Versuch
  nach bestehender Retry-Regel; keine Teilantwort beweist Abwesenheit.
- Lesen→umsetzen: Drive-Name-/Paging-Ref; ureq-Fehler/Retry-Ref; Metadaten-Ref;
  bestehende `api.rs`-/HTTP-/Promotion-Verträge.
- Erwartet: `Notebook` mit zusätzlichem `notebook`-Treffer funktioniert; echte
  gleichnamige IDs bleiben sichtbar; Mutation betrifft allein das ausgewählte
  Objekt. Realer Root-Parent wird bei Cache/Anlage/Rename korrekt geprüft.
- Abnahme: S1–S3. Keine lokale Ausführung. Abhängigkeit: keine.

### M2 Stabile getrennte Drive-Bäume und logische Namen

- F2/F3/F4. Dateien: `gdrive/core/{sync_listing,extensions,names,duplicates,
  cache,resolution,metadata,state}.rs`, neue kohärente Projektions-/Bindungstypen,
  privat persistierender Adapter unter `gdrive/os/shared/`; VFS
  `core/{extensions,extension_calls,cache_extensions}.rs`, benötigte schmale
  Exporte und Wrapper.
- Schnittstelle: Dateien behalten Literalnamen/-Locators plus erfasste Metadaten-
  IDs. Nur unabhängige Ordner haben unterschiedliche logische Schlüssel und
  exakte Marker-Child-Locators; deren wirklicher Titel wird separat gehalten.
  Optionaler VFS-`sync_stat(path) -> VfsResult<VfsMeta>` hat normalen
  Backend-Stat als Fallback. Die neue Projektion ist kein globaler Browsingmodus.
  Account+Parent-ID+Literalname+Objekt-ID binden eine stabile Zuordnung. Alte
  gültige Pfadbindung wird übernommen. Versionierte private Records je Account/
  Parent werden unter Processmutex und privater Dateisperre neu geladen und
  zusammengeführt, vor Nutzung dauerhaft atomar gespeichert. Fehlende Prüfbarkeit
  darf keine Bindung löschen; vorhandene unbekannte/beschädigte Records bleiben
  erhalten und erlauben keine destruktive Neuzuordnung.
- Doppelte Ordner werden als unabhängige stabile Marker-Bäume synchronisiert.
  Datei-/Ordnergruppen, Markertext als Literal, Präfixkollisionen, Tokenwechsel,
  Neustart, Verschwinden/Neuentstehen eines Geschwisters behalten Identität.
  Echte verschiedene Datei-Inhalte bleiben in der vorhandenen Variantenwahl.
- Lesen→umsetzen: Drive-Identität; private Filecapabilities; vorhandene
  Naming-/Cache-/VFS-/Location-Verträge; Rust/serde gemäß bestehendem Code.
- Erwartet: alle unabhängigen Ordnerinhalte erreichen den Gegenort; alter
  `gdrive://Notebook`-Job bleibt an seinem validierten Baum gebunden. Andere
  Konten und gleich aussehende Locators bleiben getrennt. Literalnamen bleiben
  in Baselines/Versionen wörtlich und in Provider-Locators korrekt kodiert.
- Abnahme: S2/S3/S4/S8. Abhängigkeit: M1.

Aus der konkreten C03-Ablaufkonstruktion folgt eine zusätzliche Herkunftsgrenze:
Eine kanonische Plain-Bindung aus der vollständigen Projektion eines mehrdeutigen
Elternbaums belegt keine frühere Auswahl eines unbekannten alten Jobroots.
Die dauerhafte Zuordnung muss diese Evidenz unterscheiden. Erwartet ist:
mehrdeutiger Altroot bleibt nach einem anderen Parent-Sync geschützt; seine
bisher validierte alte ID oder eine ausdrückliche exakte Pickerauswahl bleibt
wirksam. Eindeutige Roots und vollständige unabhängige Childbäume bleiben
automatisch synchronisierbar. Dieser zeitliche Cross-Job-Ablauf gehört zu C03
und derselben M2-Implementierung, ohne zusätzlichen Jobcodec oder StateKey.
Nur tatsächlich aus dem alten globalen Cache übernommene ID-Hints werden als
Altevidenz erfasst und frisch geprüft. Ein von einer neuen Parent-Projektion
geschriebener Accountcache erhält diese Herkunft auch nach Neustart nicht.
Die Herkunftsgrenze gilt ebenso für den normalen `ensure_dir`-Writer.
Der alte globale Cache wird deshalb unabhängig von einem inzwischen vorhandenen
Accountcache geladen. Seine IDs bleiben als historische, frisch zu prüfende
Evidenz erreichbar, wenn zuerst ein anderer Job gelaufen ist; die alte Datei
wird dabei nicht überschrieben. Ein bloßer Accountcache darf weder beim
normalen Resolve noch beim Import einer Parentprojektion eine unbekannte
historische Root-Auswahl beweisen. Auch ein alter Root-Hint ohne MIME-Angabe
bleibt nach bestätigtem Löschen, Verschieben oder Umbenennen reserviert,
statt bei der ersten Migration einen neuen gleichnamigen Ordner zu wählen.

Der fünfte Lauf präzisiert C02: `Notebook` und `notebook` werden bei
Drive↔Drive schon im Seed als `NameImpossibleOnTarget` geschützt. Der
Driveadapter erbt noch den konservativen case-insensitive VFS-Default,
obwohl M1/M2 Titel und IDs exakt unterscheiden. M2 meldet deshalb den
belegten exakten Drivepfadvertrag; eine konservative Gegenstelle behält
weiterhin die gefaltete Pairpolicy. Lesen: Drive-Name-Ref, `KeyPolicy`,
`state_spellings`, Journalreplay, Full-/Incrementalplanung und gespeicherte
Conflict-/Restore-/Resume-Pfade. Die zweite Gapprüfung belegt: vorhandene
gefaltete Spellingkeys dürfen beim Policywechsel weder verworfen werden
noch ihre tatsächlich unterschiedlichen Seitenschreibweisen verlieren.
Erwartet ist eine beleggebundene Migration unter unverändertem Pairlock,
StateKey und atomarem privaten Store; Preview bleibt nichtmutierend.
Nur tatsächlich gespeicherte gültige alte Zuordnungen sind Aliasbelege;
neue gleich gefaltet aussehende Namen werden dadurch nicht neu gepaart.
Malformed/widersprüchliche Records bleiben geschützt. Der bestehende
C02-Gesamtablauf erhält historische Spellingbytes aus seinem echten Seed,
öffnet dieselben Endpunkte erneut und verlangt No-op, anschließend neue
unabhängige Casebäume, Gegenänderung, genaue Backup-/Restorebytes, Restart
und No-op. Die Enginegrenze deckt zusätzlich historische unterschiedliche
Seitenschreibweisen und ihre gespeicherten Auflösungswege ab.

Zusätzliche betroffene Grenze: historisch A `Notebook`↔B `notebook`, danach
ein neuer unabhängiger A-Baum `notebook`. Der neue logische Schlüssel darf
den bereits belegten physischen Counterpart nicht übernehmen. Der vorhandene
Destinationvertrag hat für einen solchen belegten Slot keine unabhängige
Folder-ID-/Markerallokation. Ohne tatsächlich registrierte unabhängige
Adresse wird diese konkrete Gruppe daher vor File-/Dirapply geschützt;
beide Altgegenstücke und ihre Baseline bleiben erhalten, gesunde unabhängige
Dateien schließen ab und Partialscan wird kein vollständiger Index.
Die historische Fixture muss ohne moderne Reservierungstombstones diesen
echten Zustand herstellen. Nach einer normalen ausdrücklich eindeutigen
Rename-/Auswahlaktion oder belegten unabhängigen Markeradresse muss derselbe
alte Job vollständig konvergieren und No-op erreichen. Neue Notebook/notebook-
Bäume ohne belegte physische Überschneidung bleiben automatisch unabhängig.

### M3 Gemeinsame sichere Sync-Integration

- F2–F6. Dateien: betroffene Consumer unter `bisync/os/shared/`, insbesondere
  `snapshot_walk`, `snapshot_dir`, `snapshot_duplicates`, `apply_guard`,
  `apply_boundary`, `duplicate_observation`, `duplicate_backup`; tatsächlich
  benötigte Guard-/Wrapper-Weiterleitungen. Bestehende Publisher-/Backup- und
  Journalmodule werden nur bei einem nachgewiesenen Integrationsdefekt geändert.
- Schnittstelle: tolerante Liste samt Auslassungen statt globalem Abbruch;
  zielgerichtete frische Metadaten an derselben logischen/exakten ID-Grenze.
  Kein geschützter Teilbaum darf zu Abwesenheit oder neuer vollständiger
  inkrementeller Baseline werden. Erfolg schreibt nur bestätigte Aktionen.
- Lesen→umsetzen: bestehende Engine-/Apply-/Dup-/Journalverträge; Namespace-
  Durabilitätsref, Links-/Literalpfad- und Remote-Metadaten-Verträge.
- Erwartet: unabhängige Dateien schließen trotz geschütztem Kind ab;
  beidseitige Konflikte werden mit erhaltenen Backupbytes aufgelöst; Baseline,
  StateKey, Pairlock, Retrys, Cancel und Recovery bleiben korrekt.
- Abnahme: S4–S7. Abhängigkeit: M2.

Der zweite gemeinsame Remote-Lauf belegt zwei zusätzliche betroffene
Integrationsgrenzen innerhalb M3. `sync_observation::matching/named` muss
denselben Literal-Ein-Komponentenvertrag wie `sync_child_path` verwenden;
ein für ein lokales Windows-Ziel unrepräsentierbares Drive-Geschwister darf
weder eine gesunde unabhängige Datei noch einen zulässigen Drive↔Drive-
Literalnamen sperren. Die strengere rekursive Löschvalidierung bleibt erhalten.
Unter `incremental_changes.rs`, `incremental.rs` und
`incremental_index_commit.rs` benötigt Checksum eine frische Zielsignatur an
berührten Pfaden statt eines hashlosen Statvergleichs. Ein erfolgreich
inkrementeller Lauf aktualisiert den zuvor vollständig bestätigten Index mit
bestätigten Deltas, statt beide Bäume erneut vollständig zu laufen. Fehlt die
belegte alte Cachebasis oder bleibt ein Scan/Apply teilweise bzw. geschützt,
wird daraus keine vollständige Indexgeneration. Erwartet sind tatsächliche
Mirror-Endbytes/No-op ohne Vollwalk unberührter Zielsubtrees sowie weiterhin
erhaltene Baselines und Indexschutz in C02/C05/C06/C07. Die API-/Gapprüfung
verwendet die tatsächlichen Index-/Snapshot-/Bootstrap-/Publishverträge von
`engine_change_feed`; ein zweiter Plan-Kritiker oder eine zweite Suite entsteht
dadurch nicht.

Der dritte Lauf erreicht `colon:name` im Drive↔Drive-Roundtrip und belegt die
falsche Anwendung der Agent-Wire-Pfadgrammatik in `apply_boundary`. Vor der
Korrektur werden die betroffenen Apply-/Baseline-/Checkpoint-/Index-/Versions-
und Recovery-Verbraucher derselben relativen Syncnamen gemeinsam gelesen.
Sie brauchen den providerunabhängigen Literal-Komponentenvertrag mit einer
eigenen relativen Pfadprüfung; Backend-/TargetLimits und effektive Rootgrenzen
bleiben für jede reale Aktion maßgeblich. Keine Lockerung der Agent-Wire-
Grammatik oder Traversal-/Link-/Backup-/Destruktionsregeln. Erwartet sind
vollständiger Literal-Roundtrip einschließlich persistierter Baseline,
Änderung und No-op auf beiden Desktop-OS; unrepräsentierbare Windows-Ziele
bleiben geschützt und unabhängige gültige Dateien schließen ab. Die erneute
API-/Gapprüfung benutzt `vfs::sync_path/sync_child_path`, aktuelle Journal-
und Versionsformate und den getrennten `ValidatedRelativePath`-Wirevertrag.
Das verbleibende Mirror-Nested-Orakel bleibt unverändert. Der Deletepolicy-
Ablauf fordert für die frisch hinzugefügte target-only Datei nun ausdrücklich
den vorhandenen `ScanDepth::Full`-Vertrag statt eines inkrementellen Laufs.

Der vierte Lauf bestätigt C05/C06/C07 auf beiden Desktopplattformen. C02
meldet im erweiterten Literal-/Versionsroundtrip eine geschützte Auslassung,
ohne ihren Pfad oder die Phase zu nennen. Vor einer weiteren Produktänderung
muss derselbe Ablauf diese konkrete Diagnose liefern. C04 erreicht unter
Linux den regulären SFTP-Ersetzungspfad und meldet dort eine unklare
Replacementdestination. Die Anschlusslesung verfolgt `replacement_journal`,
`replacement_publish`, `replacement_recovery` und die tatsächlichen
SFTP-/Agent-Metadaten-/Writerverträge. Erwartet sind erfolgreiche bestätigte
Ersetzung, restaurierbare alte Bytes und No-op; fremde Destinationen, verlorene
ACKs und unbestätigte Baselines bleiben durch die vorhandenen C06/C07-Orakel
geschützt. Keine Fehlerunterdrückung und keine zweite Abnahme-Suite.

Die anschließende Sourceprüfung belegt einen offenen Intent nach erfolgreichem
`replacement_publish`: Die Datei wurde verifiziert und ihr Namespace bestätigt,
der alte Wiederanlaufauftrag bleibt aber bestehen. Der Providerflow schreibt
danach legitime Gegenbytes, die der nächste Recoverylauf gegen diesen veralteten
Auftrag als fremd behandelt. Der bestätigte eigene Publish muss deshalb seine
bekannten Retained-/Stage-Slots sicher aufräumen und den Intent abschließen.
Unbestätigte oder fehlgeschlagene Veröffentlichung behält dagegen ihre
Recoveryevidenz; kein Digest-/Identitäts-/Ownerguard wird geschwächt.

Der fünfte Lauf bestätigt die vollständigen SFTP-/Key-/SSH-Agent-Flows
mit legitimer Gegenänderung nach erfolgreichem Publish. Zwei bisherige
C06/C07-Fixtureassertions erwarten danach noch einen offenen Intent.
Ihr Orakel muss stattdessen tatsächlichen erfolgreichen Abschluss ohne
Intent, Stage und Retainedslot sowie restaurierbare Backupbytes und
unveränderte Baseline/Checkpoint prüfen. Die fünf real fehlgeschlagenen
oder unklaren Publikationszweige behalten ihre offenen-Intent-, Fremdbyte-,
Ownerorientierungs- und Cleanup-Orakel. Kein Produktguard wird hierfür
geändert; dieselben vorhandenen ausgewählten FQNs bleiben erhalten.

### M4 Bestehende Job-/Updategrenzen vollständig erhalten

- F3/F5/F6. Dateien: `syncjobs`/`daemon`/`connect`/Desktop-/Mobile-Sync-Grenzen
  nur soweit aktueller Source oder die finale Suite eine konkrete Regression
  beweist. Vorhandene Windows-Private-Access-/Workerkorrektur wird erhalten.
- Schnittstelle: unveränderte gespeicherte Endpunkte/Optionen; alte Config- und
  Baseline-Versionen werden nachvollziehbar importiert, nicht neu erzeugt.
  Manuell und automatisch laufen durch denselben zulässigen Jobowner.
- Lesen→umsetzen: Jobs-Persistenz/Codec/Settings, Resolver, Scheduler/`run_one`,
  Catch-up, Worker-Handoff, gespeicherte historische Feedbytes und Sidecars.
- Erwartet: echter alter Job bleibt nach Update und Workerwechsel ausführbar;
  Ergebnisbytes und zweiter No-op-Lauf stimmen. Intervall/Calendar/Realtime/
  Startup/Connect/Catch-up behalten ihre bestehenden Zulassungen und Optionen.
- Abnahme: S5/S6/S8. Abhängigkeit: M3.

Der vierte Lauf führt den Windows-Host ohne Stackoverflow vollständig aus.
Der tatsächliche Direct-Altjob scheitert vor Jobstart beim normalen
Writerabschluss: `backend_stream` erzeugt einen exklusiven
`*.se-daemon-<16lowerhex>`-Stage, dessen Purpose im `PeerBackend`-Creatorledger
noch nicht registriert wird. M4 verfolgt die bestehende Reserve→Writing→ACK→
Ready-Kette über IPC und Peerwriter; derselbe Stage muss diese Ownership-
Prüfung erfolgreich durchlaufen. Fremde und unbestätigte Stages bleiben
abgewiesen. Die isolierte Fallback-Abnahme verwendet einen tatsächlich
gültigen generierten Recovery-Sibling; deren Validierung wird nicht gelockert.

Gespeicherte UNC-Verbindungen behalten `Protocol::Share` und ihren bisherigen
Credentialaccount mit Port `0`. Der Connector darf diesen ungenutzten Wert
beim normalen Wiederöffnen nicht als ungültigen TCP-Port ablehnen. URL-/TCP-
Provider behalten dagegen ihre positive Portprüfung. C04 prüft weiterhin
denselben gespeicherten UNC-/mapped-Resolver und vollständigen Byteroundtrip.

C09 erzeugt den echten Altzustand erfolgreich unter der veröffentlichten
APK. Der dokumentierte RV1-Lader ergänzt Konfigurationskeys und migriert den
fehlenden Deleteguard; die alte Jobdatei ist deshalb kein Byteidentitätsorakel
für diesen einmaligen Schritt. Die Abnahme erfasst ihre tatsächlichen alten
Keys und prüft ausschließlich die exakt belegten Migrationsdeltas. Alle
anderen alten Konfigurations-/Result-/Baselinehashes und Backupbytes bleiben
strikt. Der erste neue No-op entdeckt die reale Job-/Replica-Ownerbaseline,
deren bestätigte Records erhalten bleiben müssen; beim anschließenden
Force-stop gelten wieder unveränderte Config-/Ownerbaseline-/Journalhashes.

Der fünfte Windows-Lauf bestätigt den exklusiven daemon-ACK, die Abweisung
des separat geöffneten Backends und den Publish durch den ursprünglichen
Creator. Der folgende reale Altjob erreicht einen weiteren gültigen
`unique_staging_path(..., "sync-replica")`-Stage, den die Purpose-Liste nicht
erfasst. M4 verwendet deshalb die vorhandene enge Unique-Stage-Grammatik
für Creator-Eligibility aller tatsächlich erzeugten Purposes. Die breitere
Quarantäne-/Recovery-/Privatnamenerkennung ist kein Creatorbeweis.
Reserve→WriteNew→ACK→Ready, Ledger/Bindung/Snapshot/Mutation-once und der
separate Upload-Discardvertrag bleiben unverändert. C08 verlangt weiterhin
den vollständigen echten Altjob mit derselben Baseline, Restart und No-op;
ungültige Nonces, fremde Creator, verlorene ACKs und Leasewechsel bleiben
abgewiesen. C09 bestätigt nun das tatsächliche veröffentlichte APK-Update
und den vollständigen Force-stop-/Retry-/Konflikt-/Versions-/Gegenänderungs-
Ablauf mit übernommenem alten Job, Ownerbaseline und alten Sicherungen.

### M5 Eine komplette Remote-Task-Suite

- F1–F7. Erst nach M1–M4 implementieren/aktualisieren.
- Dateien: neuer einzelner checked-in Suite-Einstieg
  `native/test-sync-reliability-task.py`, zugehöriger einzelner Workflow
  `.github/workflows/sync-reliability-task.yml`; schmale cfg(test)-Szenarien
  an den jeweiligen Produktionsgrenzen, vorhandene FakeDrive-/HTTP-Fixtures
  und tatsächliche Job-/Provider-/Share-Fixtures. Kein Produktions-Testbypass.
- Ein Aufruf enthält alle Szenarien in der Tabelle. Linux/Windows benutzen
  nur die inkrementell nötigen nativen Development-Ausgaben, identifizieren
  exakten Commit und Binärinputs, teilen Cache/Binaries im Fixloop. Keine
  all-target/all-feature/Workspace-, Crosscompile- oder Releasebuild-Matrix.
  Jeder Job >=30 Minuten, prozessgebundene Cleanup-Verantwortung im Script.
- Erwartet: Laufzeitergebnisse mit Anfangs-/Endbytes, IDs/Baselines, vollständigem
  Wiederanlauf und restaurierbaren Backups. Fehlende Laufzeitwerte, ungewählte
  Szenarien, übersprungene Gegenstellen oder Cleanupfehler sind kein Pass.
- Abnahme: alle S1–S8/C01–C10 als ein Pipelineergebnis, konkret in `abnahme.md`.
  S9 gehört ausschließlich M6. Abhängigkeit: M1–M4 komplett.

Die fünfte C04-Auswertung erreicht nach vollständigen SFTP-Flows FTP.
Der reale Server bestätigt den unveränderten Literalnamen beim Upload;
LIST ohne MLSx liefert eine gröbere mtime als SIZE+MDTM beim frischen Stat.
Lesen→umsetzen: Remote-Metadaten-/Provider-Fixture-Refs, FTP-Metadatengrenze,
RFC-3659-Vertrag und die unveränderten Checksumcapture-/Applyguards.
Erwartet sind konsistente frische Dateisignaturen für reguläre adressierbare
LIST-Kinder und Stat aus demselben nichtrekursiven Probe. Unsupported/550,
Links und nicht adressierbare Namen behalten vollständige Eltern-LIST- und
geschützte Omissionsentscheidungen; keine Guards werden gelockert.
Der Windows-UNC-Flow schließt vollständig ab. Der folgende mapped-Vergleich
vergleicht dagegen gültige native Pfade mit gemischten/doppelten Separatoren
als Text. Nur dieses native Fixture-Orakel verwendet native Pfadgleichheit;
alle Remote-Locator-/Identitäts-/Byte-/No-op-Verträge bleiben bestehen.
Die beiden konkreten Grenzen werden im selben vollständigen C04-Providerflow
bestätigt, einschließlich aller bislang danach nicht erreichten Provider.

### M6 Ein vollständiger Release

- F7. Bestehender Remote-Workflow `build.yml` ruft ausschließlich den stabilen
  `native/publish-release-local.ps1` auf, >=2 Stunden Timeout. Erst nach
  erfolgreicher Auswertung M5. Ein beabsichtigter Patch 0.5.172; fehlgeschlagene
  ungetaggte Stufen bleiben in diesem Batch und derselben Version.
- Preflight aller Runner-/Tool-/Credential-/Publikationsbedingungen; Lock und
  vorhandene Wrapper-Prozesskoordination erhalten. Ein Build, ein Tag, eine
  Veröffentlichung aus dessen Artefakten. Prüfung der GitHub Release-Assets,
  Versionen und Sidecars; autorisierte lokale Installationen aus veröffentlichten
  Bytes aktualisieren. Keine lokalen Builds oder Veröffentlichung.
- Erwartet: vollständiger passender veröffentlichter Release; lokale `se`-/
  Share-Server-Versionen stimmen. Dokumentation/Graph/TODO sind aktuell,
  Meilenstein- und Abschlusscommits auf `main` gepusht.
- Abnahme: S9. Abhängigkeit: ausgewertete M5-Abnahme.

## Gesamtablauf und vollständige Vertragsmatrix

Die Matrix unterscheidet Provideridentität, Modus und Störung. Nicht jede
Protokollbesonderheit wird mit jeder unabhängigen UI-Option vervielfacht:
jede konkrete Providergrenze wird mit echten vollständigen Sync-Abläufen
geprüft; Optionen/Wiederanlauf werden zusätzlich an der gemeinsamen Engine
und den tatsächlich betroffenen Drive-/Share-/lokalen Grenzen kombiniert.

| Ablauf | Eingang und Ablauf | Verbindliches Ergebnis |
|---|---|---|
| **S1 Eindeutiges Notebook** | Reale HTTP-Calls gegen Drive-v3-Vertragsserver: `Notebook` plus anders geschriebene Kandidaten, Kaltsuche, alter Cache, Dateien darunter; Vorschau → Lauf → Folgelauf | Der exakte Baum ist kopiert, keine falsche Mehrdeutigkeit, keine zweite Ordneranlage; Folgelauf ohne erneuten Transfer |
| **S2 Drive-Namensräume** | Echte verschiedene Datei-IDs (gleicher/verschiedener Inhalt), zwei gleiche Ordner, gemischte File/Folder-Namen, Marker-/Präfixkollisionen, Unicode, führende/trailing Spaces, `%`-/reservierte Namen; beide Richtungen und Inhalt ändern | Jede Ordner-ID bleibt eigener Baum; Dateien gemeinsame Variante oder auflösbarer echter Konflikt; Auswahl, Backup, Rename/Trash wirken auf genau die ID; alle Inhalte bleiben erhalten |
| **S3 Paging/Identität** | Leere Seiten mit Token, wiederholte IDs, zyklische/rejected Tokens, incompleteSearch, reale Root-ID in parents, verschobene/gelöschte IDs, Cache-Clear/Restart/Tokenwechsel, zweites Konto | Vollständiger erfolgreicher Wiederanlauf; kein Teil-Snapshot und kein falsches Missing; alte gültige Bindung bleibt, Konten bleiben getrennt |
| **S4 Gegenstellen/Literalpfade** | Lokal↔jede unterstützte Providerklasse, Drive↔Share/WebDAV/SFTP und zwei verschiedene Remotes; tatsächliche Resolver und Wrapper; ZIP/read-only Quelle, UNC/mapped/SMB, SFTP mit/ohne Agent, FTP/FTPS, WebDAV, Direct/Room; komplette Bäume | Inhalt und Namen konvergieren an autorisierten Orten; keinerlei Umleitung zu lokalen APIs; fremde Konto-/Verbindungszustände unberührt; read-only Quellen lesbar |
| **S5 Modi/Optionen** | AtoB/BtoA/Both, Mirror/Preview, Compare MtimeSize/SizeOnly/Checksum, gespeicherte Konfliktmodi, Hidden/Ignore/Size/Age, Modifywindow/Transfer/Bandbreite, NoDelete/Deletecaps/Move/Recycle, eigene Versionsorte/Aufbewahrung | Gewählte Optionen werden im vollständigen Joblauf sichtbar erfüllt; No-op-Zweitlauf; restaurierbare verdrängte Bytes; unerlaubte Löschungen unterbleiben |
| **S6 Unterbrechen/Fortsetzen** | Cancellation beim Scan/Backup/Upload/Publish, verlorener ACK, Verbindungsverlust, 429/5xx, Quota/Full, zeitweilige Rechte-/Lock-Sperren, Restart und Wiederherstellen derselben Gegenstelle | Alter Inhalt/Baseline bleibt bis Bestätigung; keine doppelte Anlage und kein Fremdoverwrite; derselbe Job endet nach Wiederherstellung korrekt; alle eigenen Worker/Stages abgeschlossen oder nachvollziehbar recovery-gebunden |
| **S7 Schutz/Parallelität** | Geschützte Links/Junctions/native/Shortcut-Kinder, teilweise unlesbarer Teilbaum, unabhängige Dateien, Backupfehler, konkurrierende Objekt-/Zieländerung, gleichzeitige Jobs/Owner, umgedrehte Endpunkte | Unabhängige Dateien fertig; geschützte Gegenstücke/Baseline bleiben; fehlendes Backup blockiert destruktive Aktion; neue Fremdbytes bleiben; Owner-/Replica-/Lockgrenzen erhalten |
| **S8 Alte echte Jobs** | Historische `.conf`/TSV/Baseline und veröffentlichter alter CLI-Worker; aktuelle private Windows-Zugriffe; Update-Handoff, reale gespeicherte Jobauflösung, Desktop-/Mobile-Aufruflogik, manuell und alle Scheduler/Catch-up-Klassen | Gleiche Job-ID/Endpunkte/Optionen/Versionen nach Update; erfolgreiche Änderungs-/Konflikt-/Retry-Läufe ohne Neuanlage; Hintergrund und Vordergrund bleiben konsistent |
| **S9 Lieferung** | Ein geprüfter Commit → ein Remote-Release → eine Publikation; Cargo/feed/Tag/Installer/alle Hashes, Android und Share-Server, lokale veröffentlichte Installation | GitHub Release mit allen erwarteten passenden Assets sichtbar; tatsächliche installierte Programme/Dienste benutzen diese Bytes |

## Umsetzung und Status

Drive-Umsetzer besitzt M1/M2 unter `gdrive`; Engine-Umsetzer besitzt M3 und
die additive VFS-Grenze samt Wrappern; Job-Umsetzer besitzt die von M4
betroffenen Job-/Resolver-/Worker-Verträge. Diese Schreibflächen überlappen
nicht. Der VFS-Vertrag `sync_stat` ist das gemeinsame Fundament, dessen
Signatur schon hier feststeht. Der Mainagent orchestriert, integriert und
besitzt Suite, Commits/Push und Release; keine parallelen Builds oder lokalen
Ausführungen. Jeder Umsetzer macht seinen eigenen Self-Review. Genau ein read-only Kritiker
nimmt vor Code den vollständigen Plan auseinander; keine weitere Reviewrunde.

| Meilenstein | Status | Evidenz |
|---|---|---|
| Spec A/B/C | fertig, konkretisiert Nutzerauftrag | `spec.md` |
| Stufe 1 / erste Recherche | fertig | `recherche.md`, gesicherte Refs |
| Stufe 2 / zweite Recherche | fertig, Lücken eingearbeitet | dieser Plan und `abnahme.md` |
| Einmalige Plan-Kritik | abgeschlossen, alle Befunde eingearbeitet | `review.md` |
| M1/M2 | exakte Drivefähigkeit und historische C02-Flows umgesetzt; Engine-Anschluss im Self-Review | C01/C03 auf beiden Hosts bestätigt. `f9283c2b` erhält die exakte Drivefähigkeit und vollständige historische Seeds/Restarts/Casebäume/Backup-/Restore-/Kollisionsabläufe; gemeinsame Remote-Bestätigung offen |
| M3 | bestätigter Publish-Vertrag und konkrete C06/C07-Orakel umgesetzt; gemeinsame Bestätigung offen | C05 und vollständige reale SFTP-Flows bestätigt. `4a40e514` prüft tatsächlichen erfolgreichen Abschluss sowie echte offene Failure-/Lost-ACK-/Fremdbyte-/Baseline-/Backupfälle; keine Guardlockerung |
| M4 | gemeinsame Unique-Stage-Grammatik umgesetzt; C09 vollständig bestätigt | Beide echten Desktop-Workerübernahmen und Android-Update/Force-stop/Retry/Konflikt/Gegenänderung erfolgreich. `2c8062fe` erhält die Creator-/ACK-Kette auch für den gültigen Replica-Purpose; Windows-Altjob gemeinsam remote zu bestätigen |
| M5 Remote-Gesamtablauf | fünfter Workflow vollständig fehlgeschlagen ausgewertet; gleicher Fixloop | Beide Hosts vollständig ohne Abbruch und exakt ausgewertet. C01/C03/C05 und C09 bestätigt; C02/C04/C06/C07 sowie Windows-C08 bleiben offen. `b6d6bf7e` korrigiert FTP-Metadaten und ausschließlich das mapped-Pfad-Orakel. C10 benötigt weiterhin die angefragte echte Drive-Testautorisierung |
| M6 Release | offen | erst nach M5 |

Der [fünfte Workflow](https://github.com/b1ue-man/smart-explorer/actions/runs/37267225051)
prüft `9b4bb4d2d27dc3fcdd14e96ebaf2ea23deb467ce` und ist vollständig
fehlgeschlagen ausgewertet. Beide Hosts führen alle gewählten Fälle ohne
Prozessabbruch aus; unmittelbare Diagnosen und passende Ergebniszähler
sind bestätigt. Der tatsächliche Android-Altjob schließt alle Update-/
Neustartphasen erfolgreich ab. Die konkreten verbleibenden
Befunde stehen bei M3/M4 und in `abnahme.md`; sie werden gemeinsam im selben
Suite-Einstieg bestätigt. Private Uploadgrenzen und alle bestehenden
Erhaltungs-/No-op-Orakel bleiben verbindlich. Kein Release vor erfolgreichem M5.

Pro Meilenstein: gegen Refs und Aufrufer selbst prüfen, kohärent committen;
Abnahme erst gesammelt nach kompletter Umsetzung. Nach nativen Änderungen
Root-Graph vollständig neu extrahieren/clustern. Kandidat committen/pushen,
genau eine Remote-Suite starten; relevante Befunde beheben und ausschließlich
dieselbe Suite wiederholen. Keine vorgezogenen Patch-Releases.

## Verbindliche Migration und gemeinsame Providergrenze

| Zustand eines alten Drive-Pfads | Entscheidung | Bewahrte Identität |
|---|---|---|
| Vorhandene gespeicherte ID lässt sich frisch gegen Account, Parent, Titel und Typ prüfen | Diese ID wird vor jeder neuen Auswahl dauerhaft gebunden; keine mtime-Neuwahl | Alter Locator, Account-StateKey, Pair-/Jobowner und Baseline bleiben unverändert |
| Kein ID-Beleg, genau ein passender Ordner | Eindeutigen Ordner frisch bestätigen und binden | Unveränderte alte Jobdatei und Pfadbedeutung |
| Mehrere passende Ordner, kein historischer ID-Beleg | Für einen als Jobroot genutzten Altpfad genaue Auswahl über bestehenden Ordnerpicker; kein behaupteter automatischer Altidentitätsbeweis. Bei einer neu erstellten vollständigen Elternbaum-Synchronisation wird der bestehende deterministische Erstwahlvertrag verwendet und sofort gebunden; alle weiteren IDs sind eigene Bäume. | Keine stillschweigende Übernahme einer unbekannten historischen Root-Auswahl |
| Bekannte ID ist vorübergehend nicht prüfbar | Bindung und sämtliche Zustände unverändert halten; derselbe Job setzt nach Wiederherstellung fort | Kein Wechsel auf erreichbares Geschwister, keine Ersatzanlage |
| Bekannte ID bestätigt gelöscht oder aus dem erwarteten Parent entfernt | Alias bleibt reserviert; Jobroot bleibt geschützt/fehlend. Ein untergeordnetes wirklich gelöschtes Objekt darf nur durch die bestehenden bestätigten Löschregeln verarbeitet werden. | Ein neues gleichnamiges Objekt übernimmt keinen bekannten Root |
| Bekannter Root wurde umbenannt | Alter Root wird nicht auf ein gleichnamiges Geschwister umgeleitet. Explizite neue Auswahl des gleichen Objekts benutzt den vorhandenen Picker-/Jobeditpfad; die alte Baseline bleibt bis zur bewiesenen Übernahme erhalten. | Objekt-ID-Evidenz statt zufälliger neuer Namenstreffer |
| Neue Bindungsdatei existiert, ist aber beschädigt oder hat ein unbekanntes Format | Vorherige Datei bewahren, Mutation dieses Baums sperren; kein Fallback auf leere Zuordnung | Kein neuer Baseline-/Identitätserfolg aus unprüfbarem Zustand |

`BackendExtensions::sync_stat(&self, path: &str) -> VfsResult<VfsMeta>` und
`vfs::sync_stat<B: Backend + ?Sized>(&B, &str)` sind additiv; ohne Extension
entspricht das Ergebnis `Backend::stat`. Cache, Guard und live IPC leiten
mit ihren bestehenden Rechte-/Pfadgrenzen weiter. Bei Drive-Files ist
`meta.name` der tatsächliche Literalname; bei projizierten Ordnern ist es
der registrierte logische Sync-Schlüssel. Der echte Titel/ID/Parent bleibt
in der Drive-Projektion getrennt. `sync_child_path` interpretiert einen
Markertext nur mit registrierter Herkunft; sonst ist er ein Literalname.

Für einen Gegenort mit anderer Folder-ID gilt derselbe logische Name als
gemeinsamer Sync-Schlüssel. Neu erzeugte Markertext-Ordner auf diesem Gegenort
sind dessen Literalnamen. Vorhandene Bindungen und Kurzmarker bleiben stabil;
neue Kollisionen erhalten einen noch nicht belegten vollständigen ID-Marker,
ohne bestehende Aliase zu verlängern oder mehrfach zu escapen. Tombstones
reservieren vorige Plain-/Markerzuordnungen. Namen, die der konkrete Ziel-
Provider physisch nicht abbilden kann, bleiben nach dessen TargetLimits
geschützte Auslassungen mit erhaltenem Gegenstück; die Suite muss diesen
Teilstatus feststellen und darf dafür keine volle Konvergenz behaupten.

Paging-Retries beginnen bei leerer Sammlung. `incompleteSearch` veranlasst
geeigneten User-/Drive-Corpus statt blind gleicher Anfrage; ein verworfener
Token oder Tokenzyklus wird unter bestehender begrenzter Retry-/Backoff-
Politik neu gestartet. Fehlende bekannte IDs werden frisch überprüft; wenn
das nicht möglich ist, wird keine destruktive Abwesenheit abgeleitet. Der
vorhandene Cancel-/Requesttimeout-Vertrag und strikte Mutation-once-ACK-
Reconciliation bleiben erhalten.
