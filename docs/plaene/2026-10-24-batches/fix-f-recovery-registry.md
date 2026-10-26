# M05: optionale Recoveryfelder und atomarer Registrybesitz

Stand: 2026-10-26. Eigener statischer Autorzyklus zu F14/F15 abgeschlossen.
Arbeitsbaum `/tmp/smartexplorer-review-fixes-ee75e91b`, Branch
`codex/review-fixes-ee75e91b`; Dokumentationsgate: `33257f82`.
Die Rohfehler beziehen sich auf `1ffbf4b0bc90b2479947680abcd0a5941cce00f8`;
laut delegiertem Auftrag ist dessen Quellstand identisch. Ein lokaler
Compiler-, Test- oder Laufzeitpass wird nicht behauptet.

## 1. Auftrag und Erstplan

Der eigene vollständige Fixauftrag M05 von Main verlangt lesbare kompatible
optionale Recovery-Registryfelder, atomare aktive/inaktive Eigentumsübergänge
und Rücknahme nach Fehlern. Fehlerliste, direkte/indirekte Funktionskarte,
Erhaltungsplan, Fix und eigene anschließende Anti-Regressionsreview bleiben
bei diesem Autor; maßgeblich ist der vollständige Nutzervertrag in
`/tmp/smartexplorer-final-review-user-contract.txt`. Die aktuelle Fläche ist
ausschließlich `/tmp/smartexplorer-f-recovery_registry-scope.json`.

Promptwissen: F14/F15 sind zugewiesen; mutable JSON-Indizierung ist ein
belegter Ansatzpunkt und ersetzt nicht die eigene Kettenanalyse. Der
abgeschlossene gemeinsame Plan ist `rv1-funktionsplan.md` im selben Ordner.
Eigene Schreibflächen sind die vier konkret freigegebenen Registry-/Fixture-
Dateien und dieser Bericht. Main besitzt Integration, Git, Rootgraph,
gemeinsame Remote-Suite und Release. Lokale Builds, Compiler, Tests,
Formatter, Server, Installationen, Unteragenten und Gitmutationen entfallen
gemäß Auftrag. Die Quellen-/Dokumentänderung ist autorisiert.

Erstplan vor dem Quellfix: Rohdiagnosen F14/F15 gegen ihre tatsächlichen
Fixturezeilen prüfen; gewöhnliche und Recoverywriter, beide Registryleser,
Besitzergates, Paket-/Cacheidentität und Rücknahme verfolgen; das optionale
Feld und seine fehlenden/ungültigen Zustände von gültigen Eigentümerbüchern
trennen. Danach die Import-/Indexierungsverträge anhand der tatsächlich
gelockten serde_json-Version belegen, Alternativen abwägen und den konkreten
Erhaltungsplan vervollständigen. Die anschließende eigene Review prüft den
gesamten ermittelten betroffenen Weg gegen den fertigen Diff. Unbekannt sind
zu diesem Zeitpunkt die vollständigen noch zu lesenden Aufrufer-/OSgrenzen;
eine notwendige nur vom Nutzer lieferbare Angabe ist nicht benannt.

## 2. Belegte Fehlerfunktion und Ursache

Die Rohquelle `/tmp/smartexplorer-rv1-40406246032/native-failures.json` wurde
gezielt vollständig für F14 und F15 ausgewertet und mit
`rv1-funktionsbefunde.json` abgeglichen. Beide Diagnosen treten auf Linux und
Windows auf:

| Befund | Bestehender Fall | Tatsächlich beobachteter Fehler |
|---|---|---|
| F14 | `batches_task_recovery_owner_document_commit_keeps_registry_readers_consistent` | Zeile 70: Nach dem ersten aktiven Dokument scheitert `mount_registry::load_in` mit `invalid type: null, expected a sequence`. |
| F15 | `batches_task_recovery_owner_injected_commit_failures_restore_active_or_inactive_origin` | Zeile 166: Der Leser scheitert nach dem injizierten Fehler mit derselben Meldung; dadurch ist die vorhandene Rücknahme-/Wiederholungsabnahme noch nicht bestätigt. |

Die genannten Zeilennummern gehören zum Rohfehlerkandidaten vor diesem Fix.

| Funktion / Fixture | Eigene Fehlerklassifikation |
|---|---|
| `write_document` | Einzige belegte fehlerhafte Produktfunktion dieses Befundpaares: eine zur optionalen Kürzung gedachte Abfrage verändert die Dokumentform. |
| `book`, `list_in`, `effective`, `load_in` | Korrekte abhängige Leser: Ihr Fehler ist die sichtbare Folge des ungültig erzeugten Felds; ihre strenge Validierung bleibt erhalten. |
| `replace_private`, Rücknahmezweig von `write_document` | Direkt betroffene Commit-/Fehlerkette; F15 muss nach der Formkorrektur ihre bestehende aktive/inaktive Rücknahme und Wiederholung bestätigen. Kein eigenständiger Fehler dieser Mechanik ist durch F14/F15 belegt. |
| `OwnerFixture::{active_document,inactive_document}` | Zulässige Dokumente des vorhandenen Produktschemas. Kein Fixturefehler und keine Anpassung der aktiven Datenform nötig. |

Eigener Quellbefund am Fehlerkandidaten: `mount_inactive_recovery.rs::write_document` klont ein
Registrydokument und verwendet zum optionalen Kürzen der Lückenliste
`stored[FIELD]["records"].as_array_mut()`. Schon die mutable Indizierung
fügt fehlende Schlüssel hinzu, bevor `as_array_mut` überhaupt entscheidet.
Ein gültiges aktives Dokument ohne `inactive_recoveries` wird so um
`"inactive_recoveries":{"records":null}` ergänzt. `book` akzeptiert ein
fehlendes Feld als leeres Buch, verlangt bei einem vorhandenen Feld aber
`RecoveryBook.records: Vec<InactiveMountRecovery>`. Der erzeugte Nullwert
ist kein zulässiges Buch. `Stored::read` kann die Mountliste lesen;
`load_in → effective → list_in → book` lehnt das zusätzlich erzeugte Feld ab.

Dieser Befund ist ein Produktfehler der gemeinsamen Schreibfunktion. Die
bestehenden Fixtures verwenden ein zulässiges aktives Dokument ohne
optionales Feld und werden nicht als Ursache umgedeutet. Ein durch die
Rohdiagnose separat belegter Fehler der eigentlichen atomaren Ersetzung
oder einer Sicherheitsentscheidung liegt hier nicht vor. Ihre konkrete
Kette wurde vor dem Fix untersucht und ist nachstehend dokumentiert.

## 3. Direkte und indirekte Verknüpfungskarte

Die folgende eigene Karte beschreibt die vollständige durch diesen Writerfix
betroffene Kette innerhalb der ausdrücklich freigegebenen Grenze. Sie endet
an den öffentlichen Recoveryclientaufrufen beziehungsweise den konkreten
unveränderten OS-/VFS-/Vaultverträgen; sie behauptet keine erneute Review der
vollständigen Provider-, GUI- oder Kernelimplementierung.

| Direkte oder indirekte Kante | Konkrete Quellen und Funktionen | Zu erhaltender Vertrag |
|---|---|---|
| Öffentliche Recoveryaktionen | `daemon/mod.rs` reexportiert `mount_client::{save_remove_mount,list_mount_recoveries,import_mount_recovery,reactivate_mount_recovery}` | Dieselben vier Aktionen und Antworttypen; kein Verlust des Desktop-/CLI-/MCP-Vertrags an dieser gemeinsamen APIgrenze. |
| Transport und Berechtigung | `mount_client::recovery_request → authenticated_stream → IpcRequest::Drive`; `ipc::handle_client → ShareHost::serve_extension` in `ipc_host_ext.rs → ipc_protocol_drives::serve` | Bestehender Daemontoken, authentifizierte Anfrage und dieselben vier `DriveIpcRequest::Recovery*`-Varianten; kein neuer Sicherheits-/Timeoutpfad. |
| Lebenszyklus und Fehlergrenze | `MountManager::{save_remove,recovery_list,recovery_import,recovery_reactivate}` in `mount_recovery_transaction.rs`; `lifecycle_guard`, `state_guard`, `ensure_registry_ready` in `mount_manager.rs` | Serialisierung der Aktionen; unlesbare Registry wird als Fehler behalten. Eigentumscommit findet statt, bevor die In-Memory-Tabelle ihren aktiven Eintrag verliert oder erhält. |
| Sichern und Entfernen | `save_remove → recovery_cache_ids → lease_existing → export_mount_recovery → inactive::detach → write_document` | Laufender Host/Backendstream wird abgelehnt; vorhandene Cacheleases bleiben bis nach dem Commit gehalten; Snapshot und Manifest liegen vor der Ablösung vor. Erst nach Erfolg: Stateeintrag und Vaultbindung entfernen. |
| Inaktives Dokument | `inactive::{detach,file_holding,file_for,read_document,book,write_document}` | Aktive Mountkonfiguration und owning Recoveryrecord wechseln in demselben Dokument. Konfigurationsabweichungen und doppelte Recovery-/Mountbesitzer bleiben Fehler; handeditierter Speicherort wird berücksichtigt. |
| Import | `recovery_import → ensure_recovery_directory → copy_recovery_package → inactive::import` | Paket wird privat kopiert und verifiziert. Ein importierter Record besitzt zunächst keinen Original-Mountcache (`owns_mount_cache=false`); Originalpaket und fremde aktive Mount-ID werden nicht still übernommen. |
| Reaktivierung | `recovery_reactivate → read_recovery_manifest → validate_vault_identity → restore_recovery_cache → inactive::{reactivate,file_for_reactivation} → write_document → registered_entry` | Manifesthash und historische Konto-/Vault-/Rootidentität bleiben erforderlich. Vorhandene Caches müssen exakt passen; kein Überschreiben. Im gleichen Dokument owning Flag löschen und aktive Konfiguration hinzufügen; anschließend gestoppter Eintrag mit Unknown-Recoverystatus und Auto-Remount aus. |
| Lesen des Eigentümerbuchs | `inactive::{list,list_in,book,is_inactive,is_inactive_in,effective}` | Fehlendes Feld bedeutet leeres Buch. Vorhandenes Buch verlangt Version 1 und Records als Sequenz; absolute Pfade, gültige Konfigurationen sowie eindeutige Recovery-IDs und owning Mount-IDs bleiben geprüft. `list` hydriert Lücken aus dem hashgebundenen Manifest oder meldet Paketfehler. |
| Aktive Registryleser | `mount_registry::{load,load_in,registered_for_completion}`, `Stored::{read,configs}`, `parse_first`, `ext::read`; `load_in → inactive::effective` | Beide Registrydateien und ältere Datenformen bleiben lesbar. Gleichzeitig aktive und inaktive owning Mount-ID bleibt ein Fehler; ein unlesbarer Bestand wird nicht als erlaubte leere Tabelle ausgegeben. Completion erhält bei Lesefehlern weiterhin keine Kandidaten. |
| Gewöhnliche Writer | `upsert/upsert_in`, `remove/remove_in → Stored::write → write_first` beziehungsweise `ext::{write,encode} → write_file → inactive::preserve_field` | Neues Buch wird nicht erfunden; vorhandenes gültiges Buch bleibt bei normalen Konfigurationsänderungen erhalten. v0.5.182-kompatible Arten bleiben in `registry.json`, Devices/Vault in `registry-2.json`; unbekannte spätere Arten bleiben als JSON erhalten. 1 MiB und 256 Einträge bleiben Grenzen. |
| Privater Recoverycommit | `write_document → replace_private → DirectoryHandle::{open_root,secure_private,create_file_new} → File::{write_all,sync_all} → recovery_checkpoint(OwnershipSynced) → publish_recovery_object(replace=true) → recovery_checkpoint(OwnershipPublished)` | Genau ein vollständiges Registrydokument wird ersetzt; Kürzung betrifft nur die Shadowkopie von `gaps` und `package_error`. Gapanzahl, Manifesthash, Paket-/Cachepfade und Eigentümerflags bleiben gespeichert. |
| Fehlerrücknahme | `write_document`: vorherige Bytes lesen, fehlgeschlagene Veröffentlichung mit aktuellen Bytes vergleichen, `replace_private(previous)` beziehungsweise neu angelegte Datei entfernen und Namespace bestätigen | Fehler vor Publish behält den alten Stand. Nach erkanntem Publish wird das vollständige vorige Dokument wiederhergestellt. Rollbackfehler bleiben ausdrücklich unbestätigte Commits; Paket und Originalcache werden nicht gelöscht. |
| Konkrete OSveröffentlichung | `mount/mod.rs::recovery_storage`; Windows `mount/os/windows/recovery_storage.rs::{publish,confirm_namespace}`, Linux `mount/os/linux_os/recovery_storage.rs::{publish,confirm_namespace}` | Gleicher Parent; vorhandene private/pinnede Parentgrenze. Windows verwendet `MoveFileExW`, Linux denselben gehaltenen Parent-FD mit `renameat2` und danach `sync_all`. `replace=false` bleibt für Pakete/Caches ein Nichtüberschreibvertrag, `replace=true` nur für das Eigentümerdokument. |
| Gewöhnliche atomare Ersetzung | `mount_registry::write_file → daemon::platform::atomic_replace`; Auswahl in `daemon/mod.rs`, Implementierungen `daemon/os/{windows,linux_os}/platform.rs` | Gesyncte temporäre Datei, bisherige Zielprüfung und unveränderte Plattformersetzung; optionaler Eigentümerbestand wird vor diesem Writer erhalten. |
| Paket-/Cacheinhalt | Shared `recovery_export::{export_mount_recovery,recovery_cache_ids}` und `recovery_import::{copy_recovery_package,read_recovery_manifest,verify_recovery_package,restore_recovery_cache}`; reine Kernvalidierung in `mount/core/{recovery_export,recovery_import,recovery_identity,recovery_manifest}.rs` | Rohe Nutzdaten, versiegelte Inhalte, Journal-/Stageinventare, SHA-256 und geschützte Links bleiben beim vorhandenen Export-/Importvertrag. Registrykürzung ersetzt keine Manifestdaten und keine Freigabe. |
| Start, Retry und Host | `mount_manager::default/from_registry → start_cache::audit_registered → registered_entry`; `start_locked_retained → inactive::is_inactive`; `ensure_retryable`; Host-/Exitstate in `mount_manager_{host,state}.rs` | Inaktiver owning Bestand verhindert ungewollten Start; Retry wartet auf Host-/Streamende. Clean, Required, Unknown und RequiresUnlock behalten ihre Bedeutung; Registryfehler werden nicht als erfolgreicher Start maskiert. |
| Login und Bestandstypen | `drive_autostart::{reconcile,restore_needed,reconcile_registered}`; `MountConfig::{validate,restores_at_login}`; `InactiveMountRecovery`, `RecoveryBook`, `RecoveryCommitPoint` | Autostart folgt nur aktiven Konfigurationen und wird nach einem Commit abgeglichen. Fehler dort werden geloggt und löschen keine Recovery. Optionale Konfigurationsfelder und Wire-/Recordtypen ändern sich nicht. |

Die lesende Graphabfrage
`graphify query 'mount_registry preserve_field inactive_recovery activate reactivate rollback' --budget 900`
erreichte die relevanten Datei-/Typknoten; allgemeine Funktionsnamen erzeugten
zusätzliche gekürzte Treffer. Eine gezielte JSON-Lektüre desselben Rootgraphs
ordnete `ShareHost::serve_extension` der tatsächlichen Datei
`daemon/os/shared/ipc_host_ext.rs` zu. Die vorstehende Aufrufkarte wurde an
den konkret freigegebenen Quellen nachgelesen. Die CLI meldete Skill 0.9.11
gegenüber Paket 0.9.17; es erfolgten weder Installation noch Graphupdate.

## 4. Vorheriger Fixplan und Erhaltungsvertrag

Die zusammenhängende Planungstiefe folgt aus dem belegten Zusammenhang
zwischen optionaler Serialisierung, mehreren Registrylesern, Eigentumscommit
und Fehlerrücknahme. Es entsteht keine neue Fähigkeit oder Strukturgrenze.

Verifizierter Bestand für den Zweitplan: Beide Rohfehler sind Folgen des
gleichen Writers. Die vorhandenen Eigentümerfälle verwenden zulässige
aktive/inaktive Dokumente, echte private Veröffentlichung und beide
Einmalfehlerpunkte. Die Rücknahmelogik stellt vollständige vorige Bytes
wieder her. Gewöhnliche Writer besitzen bereits eine validierende
`preserve_field`-Grenze. Folglich sind kein neues Registryformat, keine
eigene Persistenzschicht und keine Änderung der Commit-/Leserlogik nötig.

### Entscheidungsrecherche

`native/Cargo.lock` löst `serde_json` auf **1.0.150** und serde auf **1.0.228**
auf. Die offizielle versionsgebundene
[Indeximplementierung von serde_json 1.0.150](https://raw.githubusercontent.com/serde-rs/json/v1.0.150/src/value/index.rs)
belegt den Unterschied: mutable Indexierung führt bei fehlenden Objektschlüsseln
über den Einfügepfad, während der mutable Getter vorhandene Werte optional
zurückgibt. Die [Value-Dokumentation derselben Version](https://docs.rs/serde_json/1.0.150/serde_json/value/enum.Value.html#method.get_mut)
beschreibt den benötigten Getterzugang. Die Indexquelle bestätigt ausdrücklich
auch die Umwandlung eines Nullwerts in ein leeres Objekt beim einfügenden Pfad.
Abrufdatum: 2026-10-26. Der APIvertrag erklärt damit exakt die beobachtete
Form `{"inactive_recoveries":{"records":null}}`.

Gewählt: `stored.get_mut(FIELD).and_then(|book| book.get_mut("records"))`
und danach `Value::as_array_mut`. Diese Kette erreicht ausschließlich eine
bereits vorhandene Arrayliste; fehlende oder typfremde Werte werden dabei
nicht erzeugt oder umgeschrieben. Vorhandene gültige Records werden weiter
genau wie bisher gekürzt. Der weiterhin unveränderte `book`-Leser lehnt
vorhandene Nullwerte, fehlende Pflichtsequenzen und unbekannte Buchversionen
weiter ab.

Verworfen: Ein leeres Buch immer anzulegen würde einen optionalen Vertrag
unnötig verändern. `#[serde(default)]` auf Pflichtrecords oder ein generelles
Null-als-leer würde fehlerhaften Eigentümerbestand verschweigen. Ein JSON-
Pointer wäre eine zusätzliche feste Pfadschreibweise ohne Vorteil gegenüber
der vorhandenen Feldkonstante. Ein eigener Writer oder eine Änderung der
Registryversion würde den korrekt vorhandenen Atomaritäts-/Rollbackvertrag
unnötig vergrößern.

Für die unveränderte OSgrenze wurden die Primärquellen
[Linux rename/renameat2](https://man7.org/linux/man-pages/man2/rename.2.html)
und [Microsoft MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)
am 2026-10-26 gelesen. Sie begründen die Trennung von Ersetzung und
Nichtüberschreiben sowie die bestehenden Rückgabefehler. Die statische
Lektüre ist kein Beleg einer unter Stromausfall getesteten Haltbarkeit.
Diese Plattformverträge werden durch den M05-Getterfix nicht geändert.

### Konkrete Arbeitspunkte vor der Produktänderung

| Arbeitspunkt | Eigene Änderung und Abhängigkeit | Erwartetes Ergebnis / unverkleinerte Abnahme |
|---|---|---|
| Optionalen Writerzugriff korrigieren | Nur `mount_inactive_recovery.rs::write_document`; vorhandene serde_json-1.0.150-Getter | Aktives Dokument ohne Feld bleibt ohne Feld; existierende Records werden unverändert gekürzt. Alle Bytes außerhalb dieses optionalen Zugriffs und eines erklärenden Kommentars bleiben im Produktquelltext identisch. |
| Bestehenden F14-Fall präzisieren | `batches_recovery_owner_task_tests.rs`, bestehender Konsistenzfall | Nach Schreiben des aktiven Dokuments zusätzlich vollständige semantische Dokumentgleichheit prüfen; vorhandene Leser-, Ausschluss- und Payloadassertions erhalten. Kein neuer Testname und keine künstliche Erfolgserwartung. |
| Eigene anschließende Review | Fertiger Diff gegen `1ffbf4b0`, obige eigene Karte, Fehler-/Grenzfälle und Typverträge | Abwesendes/ungültiges/gültiges Feld unterscheiden; aktive/inaktive Richtung, beide Fehlerpunkte, Rücknahme und Wiederholung nachvollziehen. Quellvergleich, Whitespace und normale native Größen statisch kontrollieren; kein lokaler Compiler-/Testlauf. |
| Bestehende Fälle übergeben | Main besitzt `native/review-task-manifest.json` und denselben Remote-Einstieg | Vier vorhandene `batches_task_recovery_owner_`-Fälle beibehalten; drei bestehende Registrykompatibilitätsfälle exakt ergänzend registrieren. Kein zusätzlicher Suiteeinstieg. |

Zweite Lückenprüfung vor Umsetzung: Tatsächlicher Werttyp, versionsgebundene
Gettersemantik, Client-/IPCroute, Zustandslocks, beide Registryformen,
Publikationsadapter und bestehende Fehlerrücknahme sind an Quellen belegt.
Kein offener APIparameter und keine zusätzliche Schreibdatei sind nötig.
Die Korrektur migriert keinen bereits ungültigen Eigentümerbestand still zu
leer; ein solcher Bestand bleibt ein ehrlicher Lesefehler. Backendidentität,
Paket-/Cachebytes, Lease-/Hostbesitz, Rechte und Sicherheitsentscheidungen
werden nicht verändert.

## 5. Umsetzung und eigene anschließende Anti-Regressionsreview

Der vorstehend festgehaltene Fixplan wurde umgesetzt:

- `write_document` verwendet für FIELD und `records` nur noch die beiden
  nicht einfügenden Getter und für die vorhandene Sequenz `as_array_mut`.
  Zwei Kommentarzeilen erklären die Erhaltungsanforderung. Der übrige
  Produktdateiinhalt ist exakt unverändert gegenüber `1ffbf4b0`.
- F14 prüft nach seinem ersten Commit zusätzlich die vollständige
  semantische Gleichheit des gespeicherten aktiven Dokuments mit seinem
  Eingang. Die bestehende Leser-, Inaktivitäts-, Ausschluss- und
  Payloadabnahme bleibt erhalten. Es wurde kein Testname geändert und
  keine bestehende Assertion entfernt.

### Eigene anschließende Anti-Regressionsreview

Der Autor las nach der Umsetzung den vollständigen fertigen Writer samt
Rollback-/Publikationshelfer, die ergänzte Fixturepassage und den fertigen
Diff erneut. Die nachstehende Prüfung bezieht sich auf die gesamte zuvor
ermittelte betroffene Kette und trennt Quellbeleg von Remotenachweis:

| Vertrag / Grenzfall | Eigener statischer Befund nach dem Fix |
|---|---|
| Recoveryfeld fehlt | Bereits der erste Getter liefert None; kein Schlüssel, Objekt oder Nullwert wird erzeugt. Das gesamte aktive Eingabedokument bleibt semantisch gleich. `book` sieht weiterhin ein fehlendes Feld und gibt sein bisheriges leeres Buch zurück. |
| Gültiges Buch mit leerer oder belegter Recordliste | Beide Getter erreichen dieselbe vorhandene Arrayreferenz wie vorher. Der identische Schleifenkörper kürzt nur `gaps` und entfernt flüchtiges `package_error` aus der Shadowkopie. Version, Manifesthash, Gapanzahl, Quelle, Mount-/Recovery-IDs, Cache-/Paketpfade und Besitzflags bleiben erhalten. |
| Vorhandenes ungültiges Buch | Es wird kein Default auf einen unbekannten Besitzerzustand gelegt. `book`, `list_in` und ihre Validierung sind bytegleich zum Kandidaten. Vorhandenes null, fehlende Pflichtrecords und unbekannte Versionen bleiben Lesefehler; der Fix ist keine stillschweigende Reparatur bereits beschädigter Dokumente. |
| Aktiv → owning inaktiv | `detach` entfernt die aktive Mountkonfiguration und fügt den owning Record weiter in derselben Valuekopie ein. `save_remove` hält seine Locks und Leases und ändert den In-Memory-State erst nach Erfolg. Ein bereits gesichertes Paket bleibt bei Commitfehler erhalten; Cache-/Vaultbesitz wird nicht vorgezogen freigegeben. |
| Owning inaktiv → aktiv | Quellen-/Rootprüfung, Cacheverifikation und `file_for_reactivation` bleiben unverändert. Owning Flag und aktive Mountkonfiguration wechseln gemeinsam; Auto-Remount bleibt aus. Der In-Memory-Eintrag entsteht erst nach dem Registrycommit. |
| Fehler nach OwnershipSynced | Vor Publish bleibt das bisherige Dokument sichtbar. Dessen fehlendes optionales Feld bleibt nun tatsächlich fehlend und ist nach Rückkehr weiter lesbar. F15 behält seinen exakten Bytevergleich, beide Richtungen und Originalpayloadprüfungen. |
| Fehler nach OwnershipPublished | Der unveränderte Rücknahmezweig erkennt die veröffentlichten neuen Bytes und ersetzt sie durch die vollständigen vorherigen Bytes. Beim vorher fehlenden Dokument bleibt der bestehende Entfernungs-/Namespaceweg erhalten. Der einmalige Fixturefehler ist vor dem Rücknahmecommit konsumiert; derselbe nächste Zustand bleibt wiederholbar. |
| Rücknahme selbst scheitert | Die vorhandene Meldung eines unbestätigten Eigentumscommits bleibt erhalten. Die Änderung verspricht bei dauerhaft fehlschlagender Speicherung keine erfolgreiche Rücknahme und löscht weder Paket noch Originalcache. |
| Gewöhnliche Registryänderung / Rollbackleser | `mount_registry.rs`, `mount_registry_ext.rs`, `preserve_field`, Datei-/Größenprüfungen und alle drei Kompatibilitätsfixtures sind unverändert. Vorhandene optionale Eigentümerfelder bleiben erhalten; ein Feld wird bei gewöhnlichen aktiven Daten nicht erfunden. Unbekannte spätere Quellarten und historische Erstdateibytes bleiben beim bisherigen Vertrag. |
| Rechte, Identität, Host und Nebenläufigkeit | Keine Änderung an IPCauthentifizierung, Lifecycle-/Statelocks, Host-/Backendstreamfence, Leaseerwerb, tatsächlichen Dateirechten, Manifestbindung oder Vault-/Kontoprüfung. Die Korrektur wirkt nur auf eine geklonte JSONdarstellung vor dem bisherigen Commit. |

Der statische Vergleich gegen `1ffbf4b0` bestätigt exakt den geplanten
Getter-/Kommentaraustausch im Produkt und ausschließlich die zusätzliche
Dokumentgleichheitsassertion in der Fixture. Die vollständigen Dateien
`mount_registry.rs`, `mount_registry_ext.rs`, `mount_recovery_transaction.rs`,
`mount_recovery_identity.rs`, `mount_manager.rs`, `mount_manager_start.rs`,
`mount_manager_start_cache.rs`, `registry_compat_tests.rs`,
`mount/core/{recovery_manifest,recovery_state,types}.rs`, beide
`mount/os/{windows,linux_os}/recovery_storage.rs`, `ipc_protocol_drives.rs`,
`mount_client.rs` und `native/Cargo.lock` sind gegenüber diesem Kandidaten
unverändert. Ein Vergleich vollständiger Texte ist kein Compiler- oder
Laufzeitbeleg.

Die gezielte Whitespacekontrolle der drei eigenen Schreibpfade meldete
keinen Befund. Die beiden geänderten Rustdateien behalten normale
Formatierung und liegen unter 500 Zeilen und 50 KiB. Die eigene statische
Review findet keinen verbleibenden belegten M05-Verhaltensverlust. Windows-
und Linuxausführung der bestehenden Fälle bleiben ausdrücklich unbestätigt.

## 6. Gelesene und geänderte Dateien

Vollständige Liste der in diesem Zyklus tatsächlich verwendeten Dateien;
Abschnittlektüre und reine Namenssuche werden ausdrücklich gekennzeichnet:

- Auftrag/Evidenz: `/tmp/smartexplorer-f-recovery_registry-scope.json`,
  wiederverwendeter unveränderter Nutzervertrag
  `/tmp/smartexplorer-final-review-user-contract.txt`, F14/F15 vollständig aus
  `/tmp/smartexplorer-rv1-40406246032/native-failures.json` und
  `docs/plaene/2026-10-24-batches/rv1-funktionsbefunde.json`.
- Dokumente: relevante Registry-/Mount-/OSgrenzen von
  `docs/ARCHITEKTUR.md`; M05-/F14-/F15-Plan- und Lückenabschnitte aus
  `docs/plaene/2026-10-24-batches/rv1-funktionsplan.md`; dieser vollständige
  eigene Bericht. Die globalen/projektbezogenen AGENTS-Anweisungen aus dem
  erhaltenen Nutzerkontext gelten unverändert weiter; daraus wird keine
  zusätzliche aktuelle Dateivolllektüre behauptet.
- Vollständig unter `native/src/daemon/os/shared/`:
  `mount_registry.rs`, `mount_registry_ext.rs`, `mount_inactive_recovery.rs`,
  `batches_recovery_owner_task_tests.rs`, `mount_recovery_transaction.rs`,
  `mount_recovery_identity.rs`, `mount_manager_capabilities.rs`,
  `mount_manager_host_task_tests.rs`, `mount_client.rs`, `ipc_host_ext.rs`.
- Gezielte Abschnitte unter `native/src/daemon/os/shared/`:
  `mount_manager.rs` (Laden, Registryfehler, Locks und ruhender Eintrag),
  `mount_manager_start.rs` (Start/Retry und Inaktivgate),
  `mount_manager_start_cache.rs` (Audit/Admission/Retryfence),
  `mount_manager_state.rs` (Hostende und saubere Registrybereinigung),
  `mount_manager_host.rs` (Hostgrant und Streamlease),
  `drive_autostart.rs` (Restoreentscheidung und Reconcile),
  `ipc.rs` (Authentifizierung und Driveroute),
  `ipc_protocol_drives.rs` (Recoveryvarianten, Promotion und Serveantworten),
  `registry_compat_tests.rs` (Alttypen, Helper und alle drei Registryfälle).
- Unter `native/src/mount/`: `mod.rs` (Modul-/OSzuordnung und Reexporte),
  `core/recovery_identity.rs`, `core/recovery_import.rs` und
  `core/recovery_manifest.rs` vollständig;
  `core/recovery_state.rs` (Einmalfehlerpunkte), `core/types.rs` (IDs,
  MountConfig, Validierung und optionale Felder), `core/recovery_export.rs`
  (versiegelte Inventarsicht), `os/shared/recovery_export.rs` (Cache-IDs,
  Export, Commit und regulärer Baumkopierweg),
  `os/shared/recovery_import.rs` (Verifikation, private Kopie, Namespace,
  Restore und Nichtüberschreiben). Beide `os/{windows,linux_os}/recovery_storage.rs`
  wurden vollständig gelesen.
- Weitere aktuelle Adapter: `native/src/daemon/mod.rs` (Registrierung und
  öffentliche Recoveryclientnamen), `native/src/daemon/os/windows/platform.rs`
  und `native/src/daemon/os/linux_os/platform.rs` (atomic_replace),
  `native/src/local_access/mod.rs` (OSauswahl/API),
  `native/src/local_access/os/windows/directory_handle.rs` (Pin-/Handlemodell
  und tatsächlicher Rooterwerb).
- Ausschließlich gezielte Namens-/Verweisabfragen, keine behauptete
  semantische Vollreview: `native/src/mount/core/types_split_tests.rs`,
  `native/src/mount/core/recovery_cache_task_tests.rs` und
  `native/src/daemon/os/shared/mount_recovery_cache_task_tests.rs`.
- Abhängigkeit/Auswahl: `native/Cargo.lock` (serde-/serde_json-Versionen),
  `native/review-task-manifest.json` (Präfixe und konkrete Integrationsnamen).
  `native/Cargo.toml` gehörte zur erlaubten Fläche, wurde in diesem Zyklus
  aber nicht für eine neue Abhängigkeitsentscheidung benötigt oder geändert.
- `graphify-out/graph.json` nur lesend über die oben dokumentierte Abfrage
  und gezielte JSON-Methodenzuordnung; die vier oben verlinkten offiziellen
  serde_json-/Plattformquellen, Abrufstand 2026-10-26.

Erstellt/geändert sind vollständig:

| Datei | Änderung | Größe nach Fix |
|---|---|---|
| `native/src/daemon/os/shared/mount_inactive_recovery.rs` | Optionaler nicht einfügender Getter und Kommentar | 342 Zeilen / 12.901 Byte |
| `native/src/daemon/os/shared/batches_recovery_owner_task_tests.rs` | Zusätzliche vollständige Dokumentgleichheit im bestehenden F14-Fall | 195 Zeilen / 7.945 Byte |
| `docs/plaene/2026-10-24-batches/fix-f-recovery-registry.md` | Neuer eigener vollständiger M05-Autorbericht | Keine native Größenregel |

Die ebenfalls schreibbar zugewiesenen `mount_registry.rs` und
`mount_registry_ext.rs` benötigen keine Änderung und bleiben unverändert.
Der Dokumentationsgate verwendete auf die eigenen Schreibpfade begrenztes
`git status --short`, `git log -1 --oneline`, die gezielte M05-Suche sowie
den beschriebenen aktuellen Quellen-/Diagnoseabgleich.

## 7. Entscheidungen, Scopeabhängigkeiten und Laufzeitfragen

Sämtliche konkret angefragten Modul-, Paket-, Plattform-, Kompatibilitäts-,
IPC- und Clientpfade wurden vor ihrer Lektüre von Main ausdrücklich lesend
freigegeben. Kein Scopepfad wurde selbst erweitert. Es besteht kein offener
zusätzlicher Lese-/Schreibbedarf und keine erforderliche Nutzerentscheidung.

Unveränderte Entscheidung: fehlend bleibt fehlend, ein vorhandener
unlesbarer Eigentümerbestand bleibt ein Fehler. Keine Formatversion wird
geändert, keine Migration auf leere Eigentümer erfolgt, kein Paket-/Cache-
oder Rechtevertrag wird zur Fehlerbeseitigung abgeschwächt.

### Exakte bestehende Fälle und Registrierung durch Main

Diese vier vorhandenen Fälle bleiben unter der bestehenden `batches_`-
Auswahl und dem erforderlichen `batches_task_recovery_`-Präfix erfasst:

- `batches_task_recovery_owner_document_commit_keeps_registry_readers_consistent` (F14, zusätzliche Dokumentgleichheit)
- `batches_task_recovery_owner_injected_commit_failures_restore_active_or_inactive_origin` (F15, beide Richtungen und beide Einmalfehlerpunkte)
- `batches_task_recovery_owner_rejects_conflicting_reactivation_source_without_losing_custody`
- `batches_task_recovery_owner_oversized_commit_preserves_previous_document_and_payloads`

Für die direkt betroffene alte/neue Registrykompatibilität wurden Main genau
diese drei bereits existierenden Integrationsfälle gemeldet; sie fehlten
beim eigenen Auswahlabgleich im Manifest:

- `drive_task_registry_v0_5_182_fixture_roundtrip_identical`
- `drive_task_registry_old_parser_ignores_new_fields`
- `drive_task_registry_ext_kinds_never_in_v1_file`

Es wurde kein neuer Fallname vorbereitet. Main ergänzt ausschließlich die
benötigte bestehende Auswahl im selben Remote-Suiteeinstieg; dieser Autor
bearbeitet das gemeinsame Manifest nicht und startet keinen Einzeltest.

### Verbleibende Laufzeitfragen und Abschlussgrenze

Offen ist die tatsächliche Windows-/Linuxbestätigung der vollständigen
Dokumentlesbarkeit, der beiden Eigentumsrichtungen, der exakten Rücknahme
nach OwnershipSynced/OwnershipPublished und der Wiederholung ohne
Payloadverlust. Die drei Registrykompatibilitätsfälle müssen weiterhin
Altdatenbytes, alte Parser und spätere Quellarten bestätigen. Ob ein
unterliegender Speicher bei realem mehrfachen I/Oversagen die Rücknahme
bestätigen kann, bleibt eine echte Laufzeitgrenze; der vorhandene
Unbestätigt-Fehler wird dafür erhalten. Die eigene statische Prüfung
ersetzt diese Abnahme nicht.

Dieser Autor führte keine lokalen Builds, Compiler, Tests, Formatter,
Server, Installationen, Unteragenten, Gitmutationen, Graphupdates oder
Releasebefehle aus. Der kurze ausschließlich lesende Graphaufruf ist
beendet. Der eigene vollständige statische Autorzyklus ist abgeschlossen;
Registrierung, Integration und Funktionsnachweis bleiben bei Main im selben
Remote-Suite-Fixloop. Ein neuer Remotelauf oder Release ist hiermit nicht
als erfolgt behauptet.
