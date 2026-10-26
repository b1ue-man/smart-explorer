# RA-07: wiederholbarer Vault-Laufwerkstart

Stand: 2026-10-26. Statischer Fixautorzyklus abgeschlossen; gemeinsamer
Remote-Lauf und Release sind hiermit nicht bestätigt. Arbeitsbaum
`/tmp/smartexplorer-review-fixes-ee75e91b`, Branch `codex/review-fixes-ee75e91b`.
Auftragsvergleich: Releasebasis `a814f263096fe69d3746e840b5f3fad3fd7e88a0`
(v0.5.184), Reviewkandidat `ee75e91bad6b28ef9206a5b2b2857cbea75c4e5b`.
Die Releasezuordnung stammt aus dem delegierten Auftrag; dieser Bericht ist keine
Prüfung veröffentlichter Artefakte. Die weiterentwickelte gemeinsame Arbeitskopie
stand beim Dokumentationsgate auf `faf44025`; fremde Änderungen werden nicht bearbeitet.

## 1. Gelesene Dateien

Fortlaufendes Leseverzeichnis; Teilabschnitte werden ausdrücklich gekennzeichnet.

- Auftrag: `/tmp/smartexplorer-fix-vault-retry-scope.json`,
  `/tmp/smartexplorer-final-review-user-contract.txt`.
- Regeln: `/root/.codex/AGENTS.md`, beide geladenen Arbeitsweise-SKILL-Dateien,
  `/root/.codex/skills/graphify/SKILL.md` (Abfragevertrag), `AGENTS.md`,
  `docs/ARCHITEKTUR.md`, `docs/oberflaechen/INDEX.md` (Laufwerk-/Tresorzuordnung).
- Pläne: `batches.md`, `vertraege.md`, `regressionsreview.md` (auftragsbezogene
  Laufwerk-/Vault-/Recoveryabschnitte). Befundquelle:
  `review-regression-astra.md`, Abschnitt RA-07, im Main-Arbeitsbaum.
  `review-regression-sol.md` und `review-gesamt.md` wurden nur nach
  auftragsbezogenen Suchtreffern durchsucht; daraus wird kein eigener Befund abgeleitet.
- Graph: `graphify-out/graph.json`, ausschließlich lesende JSON-Abfrage zu
  `retry_retained`, `start_locked_retained`, `insert` und `mount_policy`.
  Zusätzlich lesender CLIaufruf
  `graphify query 'MountManager retry_retained start_locked_retained mount_policy start_cache recovery' --budget 1000`.
  Der AST-Graph weist Methodenzuordnung und Typreferenzen nach; tatsächliche
  Aufrufkanten werden aus den nachstehend genannten Quellen ermittelt.
  Die CLI meldete Skill 0.9.11 gegenüber installiertem Paket 0.9.17;
  es erfolgte keine Installation und kein Graphneubau durch diesen Autor.
- Vollständig: `daemon/os/shared/mount_manager.rs`, `mount_manager_start.rs`,
  `mount_manager_start_cache.rs`, `mount_manager_state.rs`, `mount_restore.rs`,
  `mount_termination.rs`, `mount_manager_host.rs`, `mount_source.rs`,
  `mount_recovery_identity.rs`, `mount_host_process.rs`,
  `mount_host_process_task_tests.rs`, `mount_recovery_cache_task_tests.rs`,
  `app/core/mount_recovery_cache_task_tests.rs`,
  `daemon/os/shared/ipc_protocol_mount.rs`, `ipc_protocol_mount_host.rs`,
  `mount/core/recovery_state.rs`, `vault/os/shared/open_policy.rs`.
- Gezielte Abschnitte: `daemon/os/shared/ipc.rs` (Laufwerksaktionen),
  `daemon/mod.rs` (öffentliche Mount-Clientreexporte), `ipc_host.rs`
  (ShareHost-Konstruktion), `mount_client.rs` (Start/List/Retry, Hostlease),
  `mount_registry.rs`, `mount_registry_ext.rs` (Persistenz),
  `mount/core/types.rs` (MountConfig), `app/core/mount_ui.rs` (Aktionen),
  `mount_runtime_ui.rs`, `mount_recovery_ui.rs`, `cli/drive.rs` (List/Retry),
  `mount_paths.rs` (Auflösung, Kandidaten, Retry, Warten).
  Alle Quellpfade sind relativ zu `native/src/`, soweit nicht anders angegeben.
- Ergänzende Volllektüre: `daemon/os/shared/mount_lock_participant.rs`,
  `mount_recovery_transaction.rs`, `mount_inactive_recovery.rs`,
  `mount_manager_capabilities.rs`, `mount/os/shared/recovery_identity.rs`,
  `vault/core/open_policy.rs` sowie die eigene neue Fixturedatei und der fertige Diff.
- Ergänzende Teilabschnitte: `mount/mod.rs` (Quellmodulzuordnung, Reexporte,
  Testmodul und Cacheleaseadapter), `mount/core/mount_source.rs`,
  `mount/core/spool.rs::audit_pending_state`,
  `mount/core/range_read_task_tests.rs` (wiederverwendetes Backendfixture),
  `vault/os/shared/mount_bridge.rs` (Backend, Consumer, Ende, Unlockwatch),
  `registry_store.rs` (Defaultspeicher, Resolve), `backend.rs::confine_storage`,
  `manage.rs::detect_storage`, `connect/os/shared/resolution.rs`,
  `daemon/os/shared/rooted_backend.rs` (Konstruktion, Pfadauflösung, Schreibschutz),
  `mount_restore_tests.rs`, `mount_termination_tests.rs`,
  `mount_lock_participant_tests.rs`, `mount_paths_tests.rs`,
  `mount_manager_host_task_tests.rs` und `mount_gate_bulk_task_tests.rs`.
- `native/Cargo.toml` (Edition 2021, vorhandene Testabhängigkeiten),
  `native/review-task-manifest.json` (Präfixe und UIintegration).
  Der historische Quellstand von `mount_manager.rs` und `mount_manager_start.rs`
  wurde zusätzlich direkt mit `git show a814f263…:<Pfad>` gelesen.
- Die ergänzende Callersuche verwendete ausschließlich die in der Scope-Datei
  ausdrücklich freigegebenen Dateien und Verzeichnisse. Ein scanbarer Pfad ohne
  relevanten Treffer wird dadurch nicht als vollständig semantisch gelesen behauptet.

## 2. Erstellte und geänderte Dateien

Geändert:

- `native/src/daemon/os/shared/mount_manager_start.rs`
- `native/src/daemon/os/shared/mount_manager_start_cache.rs`

Neu:

- `native/src/daemon/os/shared/batches_mount_retry_regression_task_tests.rs`
- dieser Bericht `docs/plaene/2026-10-24-batches/fix-vault-retry.md`

Die ebenfalls freigegebenen Dateien `mount_manager.rs` und
`mount_manager_state.rs` bleiben unverändert. Keine Commits, Pushes,
Graphupdates, Builds, Formatter- oder Testläufe durch diesen Autor.

## 3. Fehlerhafte Funktionen und vollständige Verknüpfungskarte

Bestätigter Ausgangsbefund: `MountManager::retry_retained` entfernt den bisherigen
`MountEntry`, bevor `start_locked_retained` die falliblen Inaktiv-/Vaultgates
ausführt. `vault::mount_policy` löst den gespeicherten Speicher auf und prüft die
Vaultidentität. Ein dortiger Fehler läuft vor `start_cache::insert` über `?`
zurück. `MountManager::list` kennt ausschließlich die Statetabelle; Registry und
Cachebytes ersetzen den fehlenden Listeneintrag im laufenden Daemon nicht.

Die Releasebasis bestätigt dieselbe alte Retryentfernung, aber noch keinen
vorangestellten Vault-Policyaufruf: `start_locked` inserierte vor
Backendauflösung. Der Reviewkandidat fügte einen falliblen Pfad vor diesen
Insert ein. Der Besitzverlust entsteht durch diese konkrete Kombination.

| Fehlerhafte oder direkt betroffene Funktion | Einordnung |
|---|---|
| `retry_retained`, bisheriges `retry` | Fehlerhafte Eigentumsübergabe: Verlust des einzigen laufend gelisteten Eintrags vor einem frühen Fehler. Der neue gemeinsame `retry_with` behält ihn. |
| `start_locked_retained` | Betroffene Integrationsgrenze: seine legitimen Policy-/Inaktivfehler brauchen beim manuellen Retry einen weiterhin besessenen Eintrag. Die Gates selbst sind kein Sicherheitsfehler und bleiben erhalten. |
| `start_cache::insert` | Bisher ausschließlich Neuerstellung mit abgelehnter IDkollision. Seine bestehende Signatur und Neuanlagesemantik bleiben; `restart` ergänzt ausschließlich den bewussten Ersatz einer ruhenden Retrygeneration. |
| `list`, GUIliste, CLIliste | Korrekte Verbraucher der Statetabelle; der fehlende Produzenteneintrag machte die Warnung und erneute Aktion unerreichbar. Kein Verbraucher repariert oder erfindet Registryeinträge. |

### Eigene direkte und indirekte Verknüpfungskarte

Die Karte reicht von sämtlichen bestehenden Aufrufern dieser Retryfunktion über
ihre Zustands-, Besitz-, Sicherheits- und Persistenzgrenzen bis zu den
beobachtbaren Rückwegen. Die darunterliegenden unveränderten Provider- und
OSadapter werden als konkrete Vertragsgrenzen benannt; ihre vollständige
Netzwerk-/Kernelimplementierung ist nicht Bestandteil dieses Quellfixes.

| Kante | Konkrete Funktionen und Dateien | Erhaltener Vertrag und Rückwirkung |
|---|---|---|
| GUI → Retry und Liste | `app/core/mount_ui.rs`: Aktion „Erneut versuchen“ → `spawn_mount_action` → `daemon::retry_mount`; `drain_mount_ui` verarbeitet `Err` und später `List`. `mount_ui_helpers.rs`: `status_label`, `recovery_label`, `mount_status_alert`, `upsert_mount`. | `Failed`/`RuntimeUnavailable` zeigen Retry; nichtsaubere Recovery zeigt „Sichern und entfernen“. Der Fehler erscheint unmittelbar, die folgende Liste enthält weiter die Warnung. `vault_locked` behält seine besondere Darstellung. |
| CLI → Retry und Liste | `cli/drive.rs`: `resolve_selector` → `retry` → `retry_mount` → `wait_until_started`; `list` → `list_mounts`. | Dieselbe Mount-ID und derselbe Selector bleiben auflösbar. Ein Fehler wird weiter als Fehler ausgegeben; erfolgreicher Neustart wartet auf denselben Mount. |
| Automatisches Öffnen → bestehender Retry | `mount_paths.rs`: `ensure_mounted_for`, `current_mount_policy`, `mount_rank`, `mount_path_for`, `relative_in`, `drive_path`. | Bereits `Mounting` wird beobachtet. Ein passender wiederholbarer Mount nutzt denselben Retry; nichtsaubere Recovery wird nicht unbemerkt zur automatischen Neuausführung freigegeben. Site-ID und relative Literalpfade bleiben getrennt. |
| Gemeinsamer Client → IPC | `daemon/mod.rs` reexportiert `mount_client::{retry_mount,list_mounts}`; `mount_client.rs`: `mount_request`, `authenticated_stream`, `CONTROL_TIMEOUT`. | Tokenanmeldung, Loopbackziel, Fehlerantwort und Stopzeitreserve bleiben unverändert. Kein paralleler GUI-/CLISonderpfad. |
| Authentifiziertes IPC → Mountmanager | `daemon/os/shared/ipc.rs`: Auth-/Runtimeleasezulassung, `RetryMount` → `retry_retained`, `ListMounts` → `list`. | Die konkrete Requestlease wird weiter als `Arc<dyn Send + Sync>` an die Startgeneration gegeben. Antwort bleibt `Mount` oder `Err`; kein neues Drahtformat. |
| Retry → Stop → ruhende Generation | `retry` → `retry_retained` → `retry_with`; `mount_manager.rs`: `stop_locked`, `observe_exit`, `force_stop`, `lifecycle_guard`, `state_guard`. | Stop, Abwarten, Fehler und Eskalation bleiben. Child-/Streambesitz verhindert Retry. `Required`, `Unknown` und `RequiresUnlock` bewahren Registrierung; der Eintrag wird vor den Gates nicht entfernt. |
| Startgate → Inaktivbesitz und Vaultpolicy | `start_locked_retained` → `mount_registry::inactive::is_inactive`; `vault::mount_policy` → `RegistryStore::resolve` → `connect::resolve_endpoint` → `confine_storage` → `policy_for`/`binding`. | Inaktive Recovery bleibt gesperrt. Vault-ID, Locator, Scheme, Konto, Namespace, Root und Objekt werden frisch gebunden; fehlende oder geänderte Identität bleibt ein Fehler. Keine Ausweichauflösung auf einen lokalen Pfad. |
| Vaultpolicy → tatsächlicher Speicher | `vault/os/shared/manage.rs::detect_storage` → `read_key_files`; `vault/core/open_policy.rs::{stored_policy,RootBinding,MountPolicy}`; `connect/os/shared/resolution.rs` unterscheidet Local/UNC, Saved, Drive, Peer, Vault und VaultStore. | Nicht erreichbarer Speicher darf scheitern, ohne den Mount zu verlieren. Aktuelles ausdrückliches Vertrauen und readonly-Eigenschaften bleiben maßgeblich; gespeichertes Trusted allein erteilt keine neue Freigabe. |
| Geprüfter Start → atomarer Entrycommit | `start_cache::{insert,restart,insert_generation,ensure_retryable}` unter `state_guard`; danach `forget_update_stop`. | Neue Starts ersetzen keine belegte ID. Retry ersetzt ausschließlich einen weiterhin ruhenden Eintrag und übernimmt sein Recoverywissen. Ein während der Policyprüfung entstandener Stream wird am Commit erneut erkannt. Bei Ablehnung bleiben alter Entry und Updateretention erhalten. |
| Entrycommit → Cache und Kontoidentität | `start_cache::{prepare,audit,record,recovery_of}` → plattformspezifischer `audit_pending_state` → `mount/core/spool.rs`; für Google Drive `preflight_mount_recovery_identity` und `mount_recovery_identity::authorize`. | Vor einem erfolgreichen Gate keine Cacheprüfung/-bereinigung. Offlineinventur zählt nur verifizierbare Journale, versiegelte Daten bleiben unbekannt. Kontowechsel und historische Cacheautorität bleiben eigenständige harte Grenzen. |
| Cache → Backend und Rootgrenze | `backend_of` → `mount_source::resolve`: SavedRemote, GoogleDrive, Peer, Devices oder `vault_drive_bridge().open_backend`; danach `rooted` → `RootedBackend::{new,new_in}`. | Gespeicherte Konten, Direct-/Roomidentitäten, Root und Vaultsite bleiben unverändert. Rootabsicherung, aktuelle `mount_access`-Bindung, lesende/schreibende Grenzen, POSIX-/Windowsnamen und abgewiesenes SMB-Mounting bleiben bestehen. |
| Start → Registry, Ort, Host | `with_folder`/`vault_place_refusal`; `MountBackendCapabilities`, `write_capabilities::missing`; `mount_registry::upsert`; `random_token`, Loopback-IPCprüfung, `vault_drive_bridge().started`, `spawn_host`. | Linux-Vaultort bleibt auf dem vorgesehenen Ort; readonly/Schreibgarantien bleiben wirksam. Die Registry wird nach bestehender Bedingung geschrieben. Host und Vaultconsumer entstehen erst im bestehenden erfolgreichen Pfad. `fail_start` und `defer_until_unlock` bleiben die Fehlergrenzen danach. |
| Host → Token-/Stream-/Leasebesitz | `mount_manager_host.rs`: `grant_host`, `register_control`, `take_backend`, `BackendStreamLease`, `update_status`, `backend_stream_finished`; `StartEnd`, `StartupLease::mounted`; `MountHostConfig` und `mount_client::connect_mount_host`. | Alte Tokens werden erst mit dem atomaren Generationswechsel verworfen. Eine laufende Backendgeneration bleibt bis zum tatsächlichen Streamende besessen. Die neue Startlease endet am authentifizierten Mounted oder beim tatsächlich beendeten Start; kein Leaseklonen erzeugt eine neue Berechtigung. |
| Hostende → Recovery, Registry und Vault | `mount_manager_state.rs`: `finish_exit`, `finish_clean`, `clear_host_runtime`, `clear_backend_runtime`, `removable`, `snapshot`; `mount_termination.rs`: `terminated`, `after_exit`, `notify_vault`, `reported`, `withdraw`. | Recovery wird nicht allein aus einem Hostfehler als sauber erklärt. Registryretention, Reap, Vaultconsumerende und Klartextrootentzug bleiben unverändert. `snapshot` liefert weiterhin `recovery_required` kompatibel zur eigentlichen Klassifikation. |
| Vaultsperre/Entsperren → dieselbe Verwaltung | `install_vault_hooks` → `MountLockParticipant::prepare_lock` → `prepare_vault_lock`, `LockDesk`, `note_lock_pending`, `wait_exit`, `abort_drive`; `VaultDriveBridge::watch` → `restore_vault`. | Sperrfristen, Ablehnung bei nicht veröffentlichten Änderungen, versiegelter Erhalt und tatsächliches Hostende bleiben. Der erhaltene `vault_locked`-Status oder `RequiresUnlock` lässt die bestehende Entsperrwiederherstellung weiter zu. |
| Daemonstart/Update → bestehender Start | `from_registry`, `audit_registered`, `restore_with`, `put_back`, `restorable`, `Retention`, `restore_vault` → `start_locked` im Modus New. | Diese vorhandenen Restores entfernen ihren alten Slot selbst und besitzen bereits Fehlerfallbacks. Die unveränderte `insert`-Signatur und New-Semantik erhalten ihre Anbindung. Auto-remount, Sharezurückstellung und Handretry bleiben unterscheidbar. |
| Erhaltene Warnung → Sicherung/Entfernung | `mount_recovery_ui.rs`/`mount_client::save_remove_mount` → `mount_recovery_transaction.rs::{save_remove,recovery_reactivate}` → `mount_registry::inactive::{detach,reactivate}`; `mount_registry_ext.rs`. | Derselbe Lifecycle serialisiert Retry und Ownershipwechsel. Export hält Cacheleases und schreibt Paket/Identität vor dem Detach. Reaktivierung bleibt ausdrücklich und gestoppt; der Retry umgeht weder inaktive Eigentümer noch historische Konto-/Vaultidentitäten. |

Die funktionsrelevanten Enden dieser Karte sind die bestehende GUIliste und
CLIausgabe, die gespeicherte MountConfig/Registry, die unveränderte Cache-/
Identitätsgrenze, der authentifizierte Host-/Streambesitz und die bestehenden
Vaultlock-/Restore-/Recoveryabläufe. Kein solcher Verbraucher benötigt für den
Fix eine neue Sonderimplementierung.

## 4. Konkreter Plan und umgesetzte Änderungen

### A: Erstplan aus dem Auftrag

Beauftragtes Ergebnis ist RA-07 einschließlich eigener Funktionsauflistung,
direkter/indirekter Verknüpfungskarte, Fixplan, Fix und anschließender
Anti-Regressionsreview. Quellen sind der delegierte Ergebnisauftrag und die
wörtlichen Nutzervorgaben vom 26.10.2026 in der Vertragsdatei. Der Autor erhält
die bestehende Liste, „Erneut versuchen“, `se drive retry`, `se drive list`,
Recoverywarnung, Cacheinhalt, Registryidentität, Host-/Leasebesitz sowie die
gültigen Rechte- und Sicherheitsentscheidungen. Keine neue Oberfläche.

Promptwissen: Der beanstandete Fehler liegt zwischen Entfernung des alten
Eintrags und dem frühen Vault-Policygate. Klärbar aus dem Bestand sind die
genauen Lebensdauer-, Registry-, Cache- und UIverträge. Für den statischen
Fix fehlt keine notwendige Nutzerangabe. Die zu erwartende Planungstiefe ist 2,
weil mehrere bestehende gekoppelte Verträge erhalten werden müssen.

### B: Stage-one-Plan aus belegtem Bestand

1. Die komplette Retry-Kette von GUI/CLI über IPC bis MountEntry und zurück
   kartieren; die öffentliche Reexport-/IPCfläche wurde nach Meldung der
   konkreten Lücke vom Main zusätzlich lesend freigegeben.
2. Das vorhandene Lifecycle-Lock und die Child-/Streamausschlüsse erhalten;
   keine Clone- oder Neuaufbaulösung für einen noch besessenen MountEntry.
3. Den bisherigen Eintrag bis nach den falliblen Startgates behalten und seine
   neue Generation unter dem Statelock atomar ersetzen. Recoverywissen erst
   durch die vorhandene autorisierte Cacheprüfung verändern.
4. Genau vorbereitete Regressionfixtures in die freigegebene Fixturedatei
   schreiben. Der Main registriert sie in der einzigen gemeinsamen Remote-Suite.
5. Dieselbe Funktionskette nach dem Fix selbst kritisch prüfen und Grenzen
   des statischen Nachweises ausdrücklich dokumentieren.

### C: Lösungsrecherche und Entscheidung

Erste Recherchefrage: Welche Lösung erhält den einzigen lebenden Eintrag auch
während eines langsamen fehlgeschlagenen Policyaufrufs, ohne Rechteprüfung oder
Ressourcengrenzen zu umgehen?

- Nur ein `put_back` nach einem Fehler würde die dauerhafte Lücke schließen,
  ließe die Liste während des falliblen Aufrufs aber vorübergehend leer.
- Eine doppelte Policy-Vorprüfung lässt ein Zeitfenster zwischen zwei Prüfungen
  und wiederholt Remotezugriffe; sie löst den zweiten möglichen Fehler nicht.
- Gewählt: ausdrücklicher Startmodus für eine neue oder eine wiederholte
  Generation. Ein Retry behält seinen bisherigen Eintrag während der Gates;
  ausschließlich sein erfolgreicher Insert ersetzt unter dem Statelock einen
  weiterhin child-/streamfreien Eintrag. Neue Starts lehnen belegte IDs weiterhin ab.

Primärquellen, geprüft am 26.10.2026: Rust std 1.99.1,
[`HashMap::insert/remove`](https://doc.rust-lang.org/std/collections/struct.HashMap.html),
[`Entry::or_insert`](https://doc.rust-lang.org/std/collections/hash_map/enum.Entry.html#method.or_insert),
[`Drop`](https://doc.rust-lang.org/std/ops/trait.Drop.html).
Ein entfernter Wert wird im aktuellen Code sofort verworfen; sein Feldbesitz
endet dadurch. Ein expliziter atomarer Ersatz vermeidet diesen vorzeitigen
Verlust. Die Lösung benötigt keine neue Abhängigkeit, Persistenzversion oder
öffentliche API.

### D: Stage-two-Meilensteine und zweite Lückenrecherche

Zweite Lückenrecherche vor dem ersten Native-Edit abgeschlossen:

- `stop_locked` behält Recoveryzustände `Required`, `Unknown` und
  `RequiresUnlock` samt Registryeintrag. Ein sauberer hostfreier Stop kann den
  Status hingegen auf `Unmounted` setzen. Ein früher Retryfehler muss diesen
  Status wieder als `Failed` mit Retryaktion zeigen; bestehende `Failed`- oder
  `RuntimeUnavailable`-Details und der `vault_locked`-Code bleiben erhalten.
- Das Lifecycle-Lock umfasst Stop, Startgates und Ersatz. `list` benötigt nur
  das Statelock und kann den bisherigen Eintrag daher während des Gateaufrufs
  weiterhin liefern. `tick`, Optionen, Restore und Recoverytransaktion können
  keine zweite Lifecycleaktion dazwischenschieben.
- Host-IPC arbeitet zusätzlich am Statelock. Der atomare Ersatz prüft
  Child-/Streambesitz deshalb nochmals unmittelbar am Commit. Ein später
  beobachteter Besitzer verhindert den Ersatz; seine Tokens und Leases bleiben.
- Der erste Snapshot der erfolgreichen neuen Generation übernimmt den
  bisherigen Recoveryzustand. `start_cache::prepare` behält anschließend seine
  vorhandene Audit-/Identitätszuständigkeit. Inaktivprüfung, Policyprüfung,
  `backend_of`, `authorize`, Rootabsicherung, Schreibfähigkeiten, Vaultconsumer,
  Hoststart, `StartEnd` und `StartupLease::mounted` werden fachlich nicht geändert.
- Die Fixturedatei wird als Test-Untermodul von `mount_manager_start.rs`
  eingebunden. Ein gemeinsamer `retry_with`-Koordinator macht den frühen
  Gatefehler deterministisch prüfbar, ohne produktive Pfade oder Providerkonten
  zu benutzen. Beide produktiven Retry-Einstiege benutzen diesen Koordinator.
  Alle Fixturefunktionen erhalten das von der gemeinsamen Suite gewählte
  Präfix `batches_task_vault_retry_`.

| Meilenstein | Fläche und Abhängigkeit | Konkretes erwartetes Ergebnis |
|---|---|---|
| R1: bestehende Generation behalten | `mount_manager_start.rs`, `mount_manager_start_cache.rs`; vorhandenes Lifecycle-Lock | Ein früh scheiternder Retry lässt Listeneintrag, ID, vollständige Konfiguration und Recoverywissen bestehen; dieselbe ID lässt sich erneut versuchen. |
| R2: Besitz und Sicherheitsgates erhalten | dieselbe Fläche; vorhandene Vault-/Inaktiv-/Cache-/Hostverträge | Weder laufender Host noch Backendstream wird durch einen neuen Eintrag ersetzt; neue Starts mit belegter ID bleiben abgelehnt; Fresh-Policy-/Identitätsprüfungen bleiben zwingend. |
| R3: Autorreview und Remote-Fixtures vorbereiten | eigene Fixturedatei und dieser Bericht; R1/R2 | Die gezielten Fixtures prüfen Listen-/Retryerhalt, Wiederanlauf, Konfiguration und Ressourcenbesitz; Bericht trennt statischen Befund und ausstehenden Remote-Lauf. |

Umsetzung R1/R2 abgeschlossen: `retry_retained` und der unveränderte Wrapper
`retry` benutzen jetzt den gemeinsamen Koordinator. `StartEntry::{New,Retry}`
unterscheidet Erstanlage und Wiederholung. `insert` behält seine bestehende
Signatur; `restart` ersetzt einen vorhandenen ruhenden Eintrag atomar und
übernimmt dessen Recovery. Ein früher Fehler lässt die alte Konfiguration,
Fehlerklassifikation, Token-/Controlfelder und Leases bestehen. Falls der
vorherige saubere Stop `Unmounted` gesetzt hat, setzt der Fehler den weiterhin
verwalteten Eintrag auf `Failed`, damit GUI und CLI ihn erneut bedienen können.
`forget_update_stop` liegt nach erfolgreichem Entrycommit.

R3 abgeschlossen: dieselbe Person hat die vorstehende Karte, die Umsetzung,
die unten stehende eigene Review und die Remote-Fixtures erstellt. Die
Remoteausführung ist kein Teil dieses abgeschlossenen statischen Autorzyklus.

## 5. Eigene Anti-Regressionsbefunde und Grenzen

Die eigene anschließende Review ist abgeschlossen. Prüffragen und Befunde:

1. **Früher Fehler und sichtbare Wiederholung:** Zwischen `stop_locked` und dem
   erfolgreichen `restart` existiert keine Entfernung des bisherigen Slots.
   `list` kann ihn während und nach einem gescheiterten Gate liefern. Der
   Fehler läuft weiterhin an den Aufrufer zurück; GUI-/CLIfehler bleiben
   wahrheitsgemäß. Die offene Recoverywarnung bleibt am Eintrag.
2. **Recovery und vollständige Konfiguration:** Früher Fehler verändert weder
   `config` noch `recovery`. Der atomare Ersatz übernimmt Recovery und verwendet
   die durch denselben Startpfad geprüfte Konfiguration. `prepare` behält seine
   ursprüngliche Zuständigkeit für ein neues Audit; eine Wiederholung erklärt
   den Cache selbst niemals als sauber und löscht keine Cachebytes.
3. **Besitz vor und am Commit:** Derselbe Child-/Streamausschluss gilt vor den
   Gates und nochmals beim Ersatz. Die ursprünglichen Stop-/Reap-/Fehlerpfade
   bleiben unberührt. Ein spätes `take_backend` kann keine lebende Generation
   verlieren. `StartupLease` und die Requestlease bleiben unverändert getypt.
4. **Sicherheitsentscheidungen:** Keine Policy-/Inaktiv-/Root-/Schreibprüfung
   wurde verschoben hinter einen Hoststart oder ausgelassen. `mount_access`
   prüft die aktuelle Vaultbindung auch bei Trusted. Fehler liefern keine
   neue Session, keinen Consumer, keinen Mounttoken und keine neue Freigabe.
   Ein ungültiger alter Vertrauenseintrag bleibt prüfpflichtig.
5. **Entsperren, Optionen und Sicherung:** Vorhandene `vault_locked`-Details
   bleiben nach frühem Fehler erhalten, ebenso der versiegelte Pendingzähler.
   Registryrecord und `auto_remount` bleiben bei nichtsauberer Recovery bestehen.
   „Sichern und entfernen“ bleibt über dieselbe Warnung erreichbar und mit
   Retry serialisiert. Backup-, Konflikt-, Retry- und Inaktivbesitzregeln sind
   weiterhin ihre bestehenden Verantwortlichkeiten.
6. **Restore- und Testvertragsabgleich:** Die direkte Callersuche fand bereits
   bestehende Aufrufe von `start_cache::insert` in `mount_restore_tests.rs` und
   `mount_termination_tests.rs`. Eine zunächst ergänzte Signatur wurde im
   eigenen Autorzyklus vor Abschluss verworfen; der finale Wrapper behält die
   alte Signatur und das Verhalten. Kein fremder Testkörper musste geändert werden.
7. **Updateretention:** Die eigene Review erkannte, dass eine am letzten
   Commitgate abgelehnte Generation die bisherige Retention ebenfalls behalten
   muss. Deshalb wird `forget_update_stop` erst nach erfolgreichem Insert/
   Restart ausgeführt. Diese Korrektur ist im finalen Fix enthalten.
8. **Quell-/Dokumentkonsistenz:** Geprüft wurden der vollständige eigene Diff,
   die Fixturekörper, die konkreten Aufrufer und die vorstehende Karte.
   `git diff --check` für beide Produktionsdateien meldete keine Probleme.
   Die drei neuen/wesentlich bearbeiteten Rustdateien bleiben unter 500 Zeilen
   und 50 KiB: 413/17.764 Byte, 196/7.021 Byte und 265/12.470 Byte.

Verbleibende Grenzen: Keine lokalen Builds, Formatter oder Tests wurden
ausgeführt. Ein isolierter Versuch, die Quellen mit dem vorhandenen Python
statisch als AST zu parsen, fand das Modul `tree_sitter` nicht; es wurde weder
installiert noch ein Compilerersatz benutzt. Syntax-/Typ-/Linknachweis ist
deshalb ausstehend. Die Fixtures injizieren den frühen Fehler an der realen
Retrykoordinatorgrenze; sie behaupten keinen echten Providerfehler und keinen
erfolgreichen Dokany-/FUSEmount. Echte Childprozesse, Kernelmounts, sämtliche
zeitlichen IPCüberlagerungen, vergiftete Mutexzustände und Crashzeitpunkte
wurden nicht ausgeführt. Die unterstützten Desktopplattformen bleiben Windows
und Linux; Androids DocumentProvider wird durch diesen Fix nicht geändert.
Aus der unveränderten Providergrenze folgt keine Behauptung, jedes lokale,
UNC-, SSH-Agent-, FTP-/FTPS-, WebDAV-, Drive- oder Peerlogin sei praktisch geprüft.

## 6. Vorbereitete Fallnamen und Registrierung durch Main

Die Datei `batches_mount_retry_regression_task_tests.rs` ist unter
`mount_manager::start_path::batches_mount_retry_regression_task_tests`
über `#[cfg(test)]`/`#[path]` in `mount_manager_start.rs` eingebunden. Keine
zusätzliche Modulregistrierung durch Main ist erforderlich.

| Exakter vorbereiteter Fallname | Frage und erwarteter Befund |
|---|---|
| `batches_task_vault_retry_preflight_failure_keeps_list_warning_and_same_id` | Ist derselbe Eintrag während und nach wiederholten frühen Fehlern sichtbar und später unter derselben ID startbar? Required/Unknown/RequiresUnlock, readonly/readwrite und Enforced/Trusted bleiben samt vollständiger Konfiguration erhalten. |
| `batches_task_vault_retry_failure_preserves_unlock_code_tokens_and_lease` | Bleiben Unlockcode, alte Token-/Controlfelder und bisheriger Leasebesitz beim frühen Fehler erhalten, wird nur die neue erfolglose Attemptlease freigegeben und werden alte Tokens erst beim erfolgreichen Generationswechsel ungültig? |
| `batches_task_vault_retry_existing_and_late_backend_stream_keep_owner` | Verhindert eine tatsächliche `take_backend`-/`BackendStreamLease`-Übergabe vor dem Retry oder während seines Gates den Ersatz, und ist Retry nach Streamende wieder zulässig? |
| `batches_task_vault_retry_new_start_cannot_replace_and_missing_retry_cannot_create` | Bleiben eine belegte ID beim Erststart sowie eine unbekannte ID beim Retry gegen Überschreiben oder unbeabsichtigtes Erzeugen geschützt? |
| `batches_task_vault_retry_clean_preflight_error_retains_failed_action` | Wird ein sauberer, bereits unregistrierter Eintrag nach Stop plus frühem Fehler wieder als Failed bedienbar, ohne Recoverydaten zu erfinden? |
| `batches_task_vault_retry_shared_sources_keep_endpoint_identity` | Bleiben gespeicherte SFTP-, FTP- und WebDAVkonten, Google Drive, Direct Share, Room Share und der Gerätebaum mit ihren identischen Literalroots und getrennten Gegenstellen unverändert durch denselben Retrykoordinator erhalten? |

Main registriert `batches_task_vault_retry_` in `native.required_prefixes` des
gemeinsamen `native/review-task-manifest.json` und ordnet diese Funktionen dem
RA-07-Meilenstein zu. Das vorhandene Auswahlpräfix `batches_` erfasst ihre
Ausführung bereits; das zusätzliche Requiredpräfix sichert ihre Auffindbarkeit.

Für die direkt betroffenen unveränderten Integrationen dieselbe Remote-Suite
um die exakten bestehenden Namen ergänzen, soweit noch nicht registriert:
`drive_task_restore_runs_after_recovery_audit`,
`drive_task_vault_locked_start_fails_vault_locked` und
`drive_task_update_stop_exit_clean_journal_counts_as_unmount`.
`mount_recovery_cache_task_startup_shows_details_and_later_failures_still_alert`
stand beim Lesen bereits in `native.integrations` und bleibt enthalten.
Keine separate Suite, kein zusätzlicher lokaler Lauf und kein per Fixture
gestarteter Build. Ergebnisbewertung und mögliche Fixbestätigung gehören zu
Mains genau einer gemeinsamen Remote-Suite.

## 7. Entscheidungen und ungelöste Abhängigkeiten außerhalb der Fläche

- Keine Unterdelegation; derselbe Autor erstellt diesen gesamten Zyklus.
- Nur die sechs in der Scope-Datei freigegebenen Dateien sind schreibbar.
- Main besitzt gemeinsame Suite, Rootgraph, Integration, Commit/Push und Release.
- Nach dem Native-Fix ist ein Rootgraphrefresh durch Main erforderlich.
- Die konkret fehlenden Leseabhängigkeiten `ipc.rs`, `daemon/mod.rs`,
  `mount/mod.rs` und `rooted_backend.rs` wurden an Main gemeldet, ausdrücklich
  lesend ergänzt und anschließend an den benötigten Abschnitten geprüft.
- Keine ungelöste Produktabhängigkeit außerhalb der Schreibfläche ist aus
  diesem statischen Autorzyklus belegt. Ausstehend sind Mains Manifestzuordnung,
  Rootgraphrefresh, Remote-Kompilierung/-Suitebewertung und terminaler Release.
- Echtbetrieb mit nicht erreichbarem Vaultprovider und späterer Rückkehr des
  Providers bleibt bis zur gemeinsamen Remote-Suite beziehungsweise einem
  ausdrücklich benannten realen Lauf ohne Laufzeitbeleg.
