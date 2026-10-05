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
| M1/M2 | Code und eigener Self-Review fertig | vollständige Kandidatensammlung, reale Root-ID, private Folderbindung und stabile Projektion; Herkunftsbeweis getrennt von Parentprojektion und neuem Accountcache, auch am normalen Writer und gemeinsamen Dateilader über wechselnde Job-Reihenfolge; bestätigtes Missing bereits bei erster Migration geschützt; Remote-Abnahme ausstehend |
| M3 | Code und eigener Self-Review fertig | additive Sync-Stat-Grenze, tolerante Beobachtung, Literalpfad-Resolve und erhaltenes Bootstrapgate; Remote-Abnahme ausstehend |
| M4 | Code und eigener Self-Review fertig | fehlende Edit-ID geschützt, wiederanlaufbare Ursachenklassifikation, Loader-Recovery über Workerwechsel; Remote-Abnahme ausstehend |
| M5 Remote-Gesamtablauf | zweiter Workflow vollständig ausgewertet; konkrete Korrekturen im Fixloop | Notebook/Bindungsmigration/Schutz und veröffentlichte Altworker-Übernahme auf beiden Desktop-OS erfolgreich; konkrete Provider-, Optionen-, Retry-, Drive-Roundtrip- und Windows-Teilstatus-Fehlstellen offen. C09 erhält vollständige alte Task-Fehlerdiagnose; native/private Altworkerprofile vom Logupload ausgeschlossen. Keine lokale Ausführung; C10 benötigt die angefragte Drive-Testautorisierung |
| M6 Release | offen | erst nach M5 |

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
