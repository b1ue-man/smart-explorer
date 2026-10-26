# RA-07: wiederholbarer Vault-Laufwerkstart

Stand: 2026-10-26. Der statische RA-07-Fixautorzyklus einschließlich der unten
dokumentierten Modultrennung und der ersten Compilerkorrekturen ist abgeschlossen.
Der ergänzende statische D04/D05-Autorzyklus zu Remote-Lauf `40368746815` ist
ebenfalls abgeschlossen und im letzten Abschnitt belegt.
Gemeinsamer Remote-Lauf und Release sind hiermit nicht bestätigt. Arbeitsbaum
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
   Die erste statische Autorprüfung vor der Remoteformatierung ermittelte für
   die drei neuen/wesentlich bearbeiteten Rustdateien 413/17.764 Byte,
   196/7.021 Byte und 265/12.470 Byte. Diese historischen Rohgrößen ersetzen
   nicht das später ergänzte Gate für tatsächlich formatierte Dateien.

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

Die zunächst benötigte Registrierung durch Main ist im Stand der unten
dokumentierten Fortsetzung vorhanden: `batches_task_vault_retry_` steht in
`native.required_prefixes` des gemeinsamen `native/review-task-manifest.json`,
die Funktionen sind dem RA-07-Meilenstein zugeordnet. Das vorhandene
Auswahlpräfix `batches_` erfasst ihre Ausführung; das Requiredpräfix sichert
ihre Auffindbarkeit.

Auch die direkt betroffenen unveränderten Integrationen sind mit den exakten
bestehenden Namen in derselben Remote-Suite registriert:
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
- Schreibbar ist ausschließlich die jeweils aktive explizite Fläche der
  Scope-Datei. Die Erweiterung für die Größenkorrektur steht im folgenden Abschnitt.
- Main besitzt gemeinsame Suite, Rootgraph, Integration, Commit/Push und Release.
- Nach dem Native-Fix ist ein Rootgraphrefresh durch Main erforderlich.
- Die konkret fehlenden Leseabhängigkeiten `ipc.rs`, `daemon/mod.rs`,
  `mount/mod.rs` und `rooted_backend.rs` wurden an Main gemeldet, ausdrücklich
  lesend ergänzt und anschließend an den benötigten Abschnitten geprüft.
- Keine ungelöste Produktabhängigkeit außerhalb der Schreibfläche ist aus
  diesem statischen Autorzyklus belegt. Die Manifestzuordnung ist vorhanden;
  ausstehend sind Rootgraphrefresh nach der Strukturfortsetzung,
  Remote-Kompilierung/-Suitebewertung und terminaler Release.
- Echtbetrieb mit nicht erreichbarem Vaultprovider und späterer Rückkehr des
  Providers bleibt bis zur gemeinsamen Remote-Suite beziehungsweise einem
  ausdrücklich benannten realen Lauf ohne Laufzeitbeleg.

## Fortsetzung: fachliche Modultrennung nach Remote-Vorbereitung 40332971455

### Erstplan und belegter Ausgangsstand

Quelle: delegierte Fortsetzung vom 26.10.2026, aktualisierte Scope-Datei,
`/tmp/smartexplorer-rv1-40332971455/format.json` und
`/tmp/smartexplorer-rv1-size-inventory.json`. Die gemeldete Vorbereitung lief vor
Kompilierung in das Größen-Gate; dies ist kein fehlgeschlagener Funktionstest.
Ihre übernommenen Formatkorrekturen stehen auf Commit `10988167`.

Der Auftrag verlangt fachliche Module unter 500 Zeilen und 50 KiB in üblicher
Formatierung bei identischen öffentlichen Mount-/Runtimeverträgen, unveränderter
Snapshot-/Host-/Retry-/Recoverysemantik und erhaltenen Fallnamen. Die drei
belegten Ausgangsgrößen sind `app/core/drive_snapshot.rs` mit 550 Zeilen/
20.417 Byte, `daemon/os/shared/mount_client.rs` mit 505 Zeilen/19.210 Byte und
`mount/mod.rs` mit 503 Zeilen/17.531 Byte. Keine Verdichtung, keine Gateausnahme.

Der Erstplan ist: Snapshotmodell und Anwendung eines fertigen Snapshots trennen;
Laufwerksverwaltung und Hostsession aufteilen; Runtime-Datentypen und
plattformspezifische Runtimeauskunft aus der Mountfassade ausgliedern. Die
verbindliche Änderungsfläche besteht aus den drei Zieldateien, den vier
freigegebenen neuen Modulpfaden `drive_snapshot_apply.rs`,
`mount_client_host.rs`, `core/runtime_types.rs`, `os/shared/runtime_info.rs`
und diesem Bericht. Ob alle vier neuen Dateien erforderlich sind, entscheidet
die konkrete Lektüre. Das bestehende Retryfixmodul und seine Fixtures sind in
dieser Phase ausschließlich lesbar.

Planungstiefe 2: bestehende Verantwortlichkeiten und Sichtbarkeitsgrenzen werden
getrennt; es entsteht keine neue Fähigkeit, Persistenzform oder öffentliche API.
Aus dem Prompt fehlen keine nur vom Nutzer klärbaren Angaben. Noch zu klären
sind die konkreten Modulpfade, benötigten privaten Zugriffe und Reexporte; der
folgende Bestands-/Strukturplan hält diese Befunde vor jedem Native-Edit fest.

### Bestandsplan, Lösungsentscheidung und zweiter Lückenabgleich

Gelesen wurden die vollständigen drei formatierten Zieldateien, das
AST-Inventar, `app/drive_modules.rs`, `app/core/batches_drive_task_tests.rs`,
`mount_client_devices.rs` sowie die konkreten HostControl-/HostInputs- und
DeviceFeed-Anschlüsse in `mount/os/shared/host_session.rs` und `host_devices.rs`.
Die vorhandene Graphabfrage zu `drive_snapshot.rs`, `MountHostSession` und
`DriveRuntimeInstallOutcome` wurde als Navigation genutzt; den Schnitt belegt
der aktuelle Quelltext. `app/mod.rs` reexportiert die Drive-Module aus
`drive_modules.rs`; der Snapshot ist dessen Untermodul. Die bestehenden Tests
sind Kinder dieses unveränderten Snapshotmoduls.

Verworfene Möglichkeiten: Zeilenkürzung oder entfernte Kommentare lösen keine
Verantwortungsgrenze; eine bloße Auslagerung von Testmodulen würde die
überbreiten Produktionszuständigkeiten nicht trennen. Eine neue parallele
Mountclient-API würde Aufruferänderungen verursachen. Gewählt ist der folgende
fachliche Schnitt mit denselben öffentlichen Namen und unveränderten Körpern:

| Meilenstein | Dateien und konkreter Schnitt | Zu erhaltender Vertrag und Abnahmesignal |
|---|---|---|
| S1 Snapshotanwendung | `drive_snapshot.rs` behält Modelle, Identitätsentscheidungen, Eventpolling und Indexdeltas; `drive_snapshot_apply.rs` übernimmt `App::apply_drive_snapshot` und `disconnect_drive_views` als privates Kindmodul. | Derselbe Deltaentscheid veröffentlicht `drives` und `drive_info` gemeinsam, pinnt Medienidentität, verwirft nur betroffene Indexgenerationen und bricht betroffene Scan-/Picker-/Analyse-/Reclaimansichten unverändert ab. Bestehendes Testmodul und seine Fallnamen bleiben am bisherigen Pfad. |
| S2 Hostsession | `mount_client.rs` behält Start/Stop/Retry/List, Recoveryrequests und Clientanmeldung; `mount_client_host.rs` übernimmt Hostsession, Hostanmeldung, Backendhandshake, Capabilityumgebung und Identitätsstub. Öffentliche Typ-/Funktionnamen werden zurückreexportiert. | IPCrequests, Timeouts, One-use-Environmentverbrauch, EOFstop, versiegelter Spoolschlüssel, Vaultlockwatch, Status-/Recoverymeldung und Backendstream bleiben identisch. Der bestehende Gerätebaumclient kann seine drei Sessionfelder, `host_stream` und den Proxyidentitätsstub weiter intern verwenden. |
| S3 Runtimefassade | `core/runtime_types.rs` übernimmt reine Versionsdomänen, Runtime-Datentypen, MSIausgangsmodell und dessen Methoden; `os/shared/runtime_info.rs` übernimmt die existierenden OSabfragen und Installationsdelegation. `mount/mod.rs` reexportiert die gleichen Namen und behält den Prozess-/Hosteinstieg. | Identische Sichtbarkeit, serde-Felder/-Tags, Konstanten, MSIcodes, Meldungen, Plattformbedingungen und Rückgabewerte. Die neue Kernquelle enthält keine OSimporte oder Plattformzweige. Alle sieben Produktionsdateien bleiben in normalem Layout deutlich unter dem Gate. |
| S4 Eigene Nachreview | Derselbe Autor vergleicht den kompletten Verschiebungsdiff mit `10988167`, private Pfade, Reexporte, Anbindungen und bestehende Fallnamen und dokumentiert Grenzen. | Keine fachliche Körperänderung außer nötiger Pfad-/Sichtbarkeitsanpassung, keine neue Testsuite und kein lokaler Lauf. Main führt denselben Remote-Einstieg erneut aus. |

Primärquellen, Abruf 26.10.2026: die Rust Reference zu
[`#[path]`](https://doc.rust-lang.org/reference/items/modules.html#the-path-attribute),
[Sichtbarkeit und privaten Nachfahren](https://doc.rust-lang.org/reference/visibility-and-privacy.html)
sowie [öffentlichen Reexporten](https://doc.rust-lang.org/reference/items/use-declarations.html#use-visibility).
Dateipfade eines nicht inline deklarierten Moduls werden relativ zur Quelldatei
aufgelöst. Ein privates Elternelement bleibt seinen Nachfahren zugänglich;
ein öffentlicher Reexport erhält den bisherigen nutzbaren Namen.

Zweiter Lückenabgleich vor dem Edit: Das Snapshotkind kann die privaten
DriveState-/Bindingfelder seines Elternmoduls verwenden; `disconnect_drive_views`
zieht mit seinem einzigen Aufrufer um. Die unveränderten Indexmethoden behalten
ihre bisherigen `super::drive_inventory`-Pfade. Die Gerätebaumfunktionen bleiben
als bisheriges Kind von `mount_client` deklariert. Die Hostsession gibt nur
`id`, `ipc_addr` und `session_token` mit `pub(super)` an denselben bisherigen
Mountclient-Modulbaum; `stop`, `vault` und `lock_watch` bleiben hostintern.
`host_stream`, `MountProxyIdentity` und dessen `scheme` erhalten dieselbe enge
Elternsichtbarkeit und private Aliase im Mountclient. Alle nach dem Umzug
veränderten `super`-Pfade verweisen ausdrücklich auf den bisherigen Daemon,
nicht versehentlich auf den Mountclient. Runtime-Dateien werden als private
Kinder der vorhandenen Mountfassade eingebunden; die physische OSablage folgt
dem etablierten `#[path]`-Muster. Damit ist keine weitere Schreibfläche nötig.

### Ergänzte direkte und indirekte Verknüpfungskarte

Die Größenverletzungen sind bestätigte Strukturfehler, keine zusätzlich
behaupteten Laufzeitdefekte. Betroffene Verantwortlichkeiten sind
`apply_drive_snapshot`/`disconnect_drive_views`, sämtliche Methoden von
`MountHostSession` samt Hostanmeldung/Capabilityeingang/Proxyidentität sowie
Runtime-Datentypen, Versionsprüfung, Ergebnisdarstellung und Runtimeabfragen.
Der bisherige RA-07-Retryfix und seine Fehlerbehandlung werden nicht verändert.

| Verbindung | Konkrete Kette nach dem Schnitt | Erhaltene Wirkung und indirekte Verbraucher |
|---|---|---|
| Moduldefinition → Snapshot | `app/mod.rs` → Reexport von `drive_modules.rs` → `drive_snapshot` → privates `apply` aus `drive_snapshot_apply.rs`. | `DriveEntry`, `DriveBinding`, `DriveState`, `diff`, `contains` und alle bestehenden Appmethodennamen bleiben an ihren nutzbaren Pfaden. Das bestehende Snapshot-Testkind bleibt im ursprünglichen Modul. |
| Monitor → Veröffentlichung | `update_drives` nimmt den bestehenden Monitor-Snapshot an → `apply_drive_snapshot` → unverändertes `diff` auf Root, Medienidentität und Inkarnation. | `drives`, `drive_info` und `DriveState.entries` werden weiter in demselben synchronen Appaufruf veröffentlicht. Sidebar, Picker, Landing, Omni, Shareexport und Scanrootverbraucher erhalten denselben Bestand; ein Inventurfehler behält den letzten erfolgreichen Snapshot. Die unveränderten Monitor-/OSproducer liegen hinter ihrer bisherigen Aufrufschnittstelle. |
| Medienwechsel → gebundene Ansichten | `apply_drive_snapshot` → `disconnect_drive_views`; unverändert geblieben sind `binding_for`, `binding_available`, `disconnected_drive`, `bind_scan_drive`, `accept_drive_consumer`, `drive_consumer_available`. | Aktiver lokaler Scan, inaktive lokale Tabs, lokaler Picker, lokale Analyse und Reclaim behalten Abbruch, Teilzugriffsentzug und Fehlermeldungen. Remotetabs werden nicht als lokale Medien verworfen. Ein Ersatzmedium unter demselben Pfad wird nicht durch Refresh übernommen; bewusste Navigation bleibt der Auswahlweg. |
| Medienwechsel → Index/Watch/Mountdarstellung | Der verschobene Snapshotkörper erhöht weiterhin nur betroffene Generationen, bereinigt betroffene Indexpfade, markiert bestehende Vollbuilds als überholt und requeued ihre noch vorhandenen Roots. Danach unverändert `update_drive_watches`, `run_folder_search`, `mount_ui.next_poll`. `start_pending_drive_index` und `drain_drive_index` bleiben im Elternmodul. | Unabhängige Roots behalten ihren Index. Indexdeltas prüfen weiter Root, Identität, Inkarnation und Generation; `Delta::drop` behält seinen Release-Cancel. Der virtuelle Gerätebaum bleibt beim eigenen DeviceWatch und die Mountliste wird nur wie bisher vorgezogen. Keine neue Index- oder Monitorinstanz entsteht durch den Split. |
| GUI/CLI/Öffnen → Verwaltungsrequests | Unveränderte `daemon`-Reexporte → `mount_client::{start_mount,stop_mount,retry_mount,list_mounts}` → unveränderte Requestkörper und `authenticated_stream`. Recovery Save/Import/Reactivate/List bleiben im selben Modul. | Tokens, Workerbereitschaft, Kontrolltimeout, Fehlerantworten und unbefristete Recoveryantwort während des Ownershipcommits bleiben identisch. Der RA-07-Manager wird weiterhin über genau denselben Retryrequest erreicht. |
| Hosteinstieg → Session | `daemon::connect_mount_host` bleibt Reexport von `mount_client::connect_mount_host`, jetzt aus dem privaten `host`-Modul → `take_host_environment` → `MountHostAttach` → `connect_backend` → dieselbe `MountHostSession`. | Alle drei Capabilityumgebungswerte werden weiterhin vor ihrer Validierung entfernt. Tokenlänge/-alphabet, Loopbackadresse, Cachepfadabsicherung, zurückgemeldete Mount-ID, Timeout und Backendtoken werden unverändert geprüft. Kein neues Credential oder Fallback wird eingeführt. |
| Hostsession → Betrieb und Ende | `mount/os/shared/host_session.rs::HostControl for MountHostSession` ruft unverändert `report_capacity`, `report_status_with_recovery`, `wait_for_stop_timeout`, `request_spool_key`, `next_lock_order` und `send_lock_answer` auf. `HostInputs::from_session` übernimmt die gleichen öffentlichen Felder. | Status- und Recoverywire einschließlich Kompatibilitätsflag, EOFstop, versiegelter Schlüssel, Nullisierung, Vaultlockwatch sowie Control-/Backendstreambesitz bleiben bestehen. Die kompletten Methodenkörper wurden zusammen mit ihren privaten Feldern verschoben. |
| Gerätebaum → private Hostanschlüsse | Unverändertes `mount_client::devices` verwendet weiterhin `super::{host_stream,MountHostSession,MountProxyIdentity}`. Der Mountclient hält dafür private Aliase; `host` gewährt nur Elternsicht auf die nötigen Sessionfelder und Proxykonstruktion. `host_devices.rs` verwendet weiterhin dieselbe Session als `DeviceFeed` und `HostChildLink`. | Separate Verbindungen für ChildBackend, ChildRoots und DeviceWatch, 50-/35-Sekunden-Reserven, Capabilities und Writableantwort bleiben erhalten. Die drei Felder werden weder öffentlich noch daemonweit freigegeben; der nutzbare interne Modulbaum entspricht dem bisherigen. |
| Hostbackend → VFSgrenze | Verschobenes `connect_backend` und unveränderter Gerätebaumclient → `AgentBackend::from_streams` → derselbe `mount_proxy::wrap`; `MountProxyIdentity` behält sein Backendtrait. | Scheme, Identitätsstring, BrokenPipe-Verhalten und sämtliche abgelehnten Stub-I/Ooperationen bleiben gleich. Es entsteht kein lokales Ersatzbackend und keine neue Schreibberechtigung. |
| Runtimefassade → reine Daten | `mount/mod.rs` reexportiert `DriveRuntimeInfo`, `DriveRuntimeInstallOutcome`, die beiden Versionskonstanten, `DokanyVersionCompatibilityError` und `validate_dokany_version_domains` aus `core/runtime_types.rs`. | Gleiche Sichtbarkeit, Konstanten, getrennte DLL-/Treiberprotokolldomänen, serde-Felder und Status-Tags. `from_msi_exit_code`, `exit_code`, `message`, `is_failure` behalten alle Körper und Werte. CLI `drive.rs`, GUI `mount_runtime_ui.rs` und vorhandene Vertragstests verwenden unverändert die Mountfassade. |
| Runtimefassade → OSdelegation | Gleiche öffentliche Funktionen in `mount/mod.rs` → `runtime_info` aus `os/shared/runtime_info.rs` → bestehende Windows- oder Linuxadapter. | Unterstützungsentscheidung, Gerätelabel, Bereitschaft, Runtimeinfo und Installationsantwort behalten alle bisherigen Plattformbedingungen, Meldungen und Rückgabewerte. Der neue Kern enthält keinen Plattformzweig; das gemeinsame OSmodul delegiert ohne direkte OScrate oder FFI. |
| Prozessaufruf → derselbe Host | `run_host_if_requested` und `exit_on_update_stop` verbleiben unverändert in `mount/mod.rs`. | Genau zwei Argumente, derselbe `--mount-host`-Selektor, Mount-IDvalidierung, Betriebssystemhost und Exit 75 bleiben bestehen. Die Runtimeausgliederung verändert weder Prozesslebensdauer noch den Einstieg in die Session. |

Die gegenüber der ersten RA-07-Karte neu hinzugekommenen Grenzen sind damit die
Snapshotanwendung, der interne Hostclient und die Runtimeauskunft. Hinter den
unverändert aufgerufenen Monitor-/Index-/OSadaptern werden keine neuen
Implementierungen oder Laufzeitgarantien behauptet.

### Umsetzung, Dateifläche und aktuelle Größen

S1 bis S3 sind umgesetzt. Die Verschiebung verwendet die vom Remote-Formatter
erzeugten Körper aus `10988167` mit ihrem normalen mehrzeiligen Layout; es
wurden keine Anweisungen oder Kommentare zur Größenreduktion verdichtet.

| Datei | Status in dieser Fortsetzung | Lokale Quellgröße nach dem Schnitt |
|---|---|---|
| `native/src/app/core/drive_snapshot.rs` | geändert; private Moduleinbindung | 371 Zeilen / 12.651 Byte |
| `native/src/app/core/drive_snapshot_apply.rs` | neu; Snapshotanwendung und Ansichtsabbruch | 190 Zeilen / 8.036 Byte |
| `native/src/daemon/os/shared/mount_client.rs` | geändert; Kontrolle/Recovery und Reexporte | 129 Zeilen / 5.521 Byte |
| `native/src/daemon/os/shared/mount_client_host.rs` | neu; Hostsession und Handshake | 395 Zeilen / 14.374 Byte |
| `native/src/mount/mod.rs` | geändert; Fassade und Hosteinstieg | 330 Zeilen / 10.895 Byte |
| `native/src/mount/core/runtime_types.rs` | neu; reine Runtimeverträge | 130 Zeilen / 5.211 Byte |
| `native/src/mount/os/shared/runtime_info.rs` | neu; OSabfragen/-delegation | 61 Zeilen / 2.182 Byte |
| dieser Bericht | geändert | Strukturplan, Kartenfortschreibung und eigene Nachreview |

Die zuvor geschriebenen RA-07-Produktions-/Fixturedateien wurden in dieser
Fortsetzung nicht geändert. Ihre übernommenen formatierten Größen sind
463 Zeilen/18.387 Byte (`mount_manager_start.rs`), 214 Zeilen/7.206 Byte
(`mount_manager_start_cache.rs`) und 373 Zeilen/13.978 Byte
(`batches_mount_retry_regression_task_tests.rs`). Die alten Rohgrößen im ersten
Autorbefund sind ausdrücklich historisch markiert. Die erneute tatsächliche
Formatterbestätigung gehört zum selben kommenden Remote-Vorbereitungslauf;
lokal wurde kein Formatter aufgerufen.

### Eigene anschließende Anti-Regressionsreview

S4 ist abgeschlossen. Derselbe Fixautor hat den gesamten Verschiebungsdiff und
die neue Modul-/Sichtbarkeitskarte nach der Umsetzung geprüft:

1. Ein statischer Textvergleich gegen `git show 10988167:<Datei>` bestätigt
   die identischen Körper von `apply_drive_snapshot` und
   `disconnect_drive_views` und den unveränderten Rest von `drive_snapshot.rs`
   nach Entfernung der neuen Moduleinbindung. Die Reihenfolge der
   Generationen-, Pending-, Cancel-, Veröffentlichungs- und Viewänderungen ist
   damit erhalten; es wurde kein Anwendungsschritt ausgelassen.
2. Derselbe Vergleich bestätigt unveränderte Verwaltungs-/Recoveryrequests
   und `authenticated_stream`. Für den Hostkörper stimmen alle Zeichen nach
   Normalisierung genau der dokumentierten Importpfade und `pub(super)`-
   Anpassungen überein. Timeout, Statuswire, Schlüsselverarbeitung, EOFstop,
   Handshake und Environmentverbrauch haben keine fachliche Änderung.
3. Der Gerätebaumclient bleibt am alten Modulpfad. Die fünf internen
   Anknüpfungspunkte (`host_stream`, Proxyidentität und die drei Sessionfelder)
   wurden gegen seine konkreten Zugriffe geprüft. Die Session-Controlfelder
   und der Lockwatch bleiben privat. Der gesamte bisherige Sessionbesitz zieht
   zusammen um; kein Stream, Token oder Arc wird neu erzeugt oder früher verworfen.
4. Die kompletten reinen Runtimekörper und die kompletten Runtimeabfragekörper
   stimmen gegen `10988167` überein. Der Präfix und der Hosteinstiegssuffix der
   bisherigen Mountfassade bleiben unverändert. Die Reexportliste enthält alle
   zuvor dort definierten nutzbaren Namen mit derselben Sichtbarkeit.
5. `#[path]`-Auflösung und die Tiefe sämtlicher veränderter `super`-Referenzen
   wurden gezielt an der offiziellen Rustreferenz und den konkreten
   Elternmodulen geprüft. `runtime_types.rs` enthält keine OSimporte,
   `cfg`-Plattformzweige, Hostprozesse oder FFI; `runtime_info.rs` delegiert an
   die bisherigen OSadapter. Es gibt keine neue öffentliche Konfigurationsfläche.
6. Die bestehenden Testmoduldeklarationen und Fallnamen bleiben erhalten.
   Die RA-07-Statetabelle, die atomare Retryübergabe, Recoveryklassifikation,
   Registryidentität und der Lease-/Hostbesitz werden in dieser Fortsetzung
   nicht bearbeitet. `git diff --check` der drei bearbeiteten bestehenden
   Produktionsdateien sowie die entsprechende Whitespaceprüfung der vier neuen
   Quellen meldeten keine Probleme. Der zusätzliche statische Namensvergleich
   bestätigt identische Snapshot- und RA-07-Fallnamen gegen `10988167`.
   Größen wurden ausschließlich textuell ermittelt.

Grenzen: Die statischen Vergleiche führen keinen Rustcode aus und belegen keine
Kompilierung, Name-Resolution durch den Compiler, Linkbarkeit oder
Kernel-/Providerreaktion. Es wurden keine lokalen Builds, Tests, Formatter,
Hostprozesse, Installationen oder Graphupdates ausgeführt. Der einzige
Graphifyprozess war eine abgeschlossene lesende Abfrage; dessen bekannte
Skill-/Paketversionswarnung ist unverändert. Die erneute Remote-Formatter-/
Größenbestätigung und die tatsächliche Kompilierung sind noch ausstehend.
Der Vorbereitungsversuch 40332971455 ist kein funktionaler Pass und hat keinen
der verlagerten Abläufe praktisch bestätigt.

### Bestehende Fallnamen und nötige Integration durch Main

Es gibt keinen neuen Fixturefall und keine neue Testmodulregistrierung.
Das Snapshot-Testmodul bleibt `batches_drive_task_tests` mit den drei Namen:

- `batches_task_drive_replacement_and_reattachment_invalidate_only_affected_root`
- `batches_task_drive_disconnected_binding_requires_original_proven_media`
- `batches_task_drive_binding_uses_deepest_root_and_literal_path_boundaries`

Sein Präfix `batches_task_drive_` ist bereits im aktuellen Manifest enthalten.
Auch `batches_task_vault_retry_` und die sechs oben dokumentierten RA-07-Namen
sind im Stand dieser Fortsetzung bereits registriert; die frühere
Registrierungsanforderung ist damit erfüllt. Fallnamen und Auswahl bleiben.

Für den direkt verschobenen reinen Runtimevertrag stehen die bestehenden Fälle
`remote_drive_task_dokany_msi_outcomes_preserve_actionable_exit_codes` und
`remote_drive_task_dokany_library_and_driver_versions_use_separate_domains`
zur Verfügung. Ihr Modul ist `#[cfg(all(test, not(windows)))]`; falls Main sie
für diese Strukturfortsetzung ergänzend bindet, gehören sie in
`native.linux_integrations`, nicht in die plattformgemeinsame Auswahl. Das ist
keine Freigabe für einen separaten Lauf. Die vorhandenen gemeinsamen
Hostsession-/Gerätebaum-/UIintegrationen werden nicht ersetzt.

### Entscheidungen und verbleibende Grenzen außerhalb der Fläche

- Alle acht aktiven Schreibpfade wurden eingehalten; kein fremder Bericht,
  keine gemeinsame Suite-/Manifestdatei und kein vorheriger Retryfixturekörper
  wurde geändert. Die zwei konkret gemeldeten zusätzlichen Lesedateien
  `app/drive_modules.rs` und `app/core/batches_drive_task_tests.rs` wurden von Main
  ausdrücklich ergänzt und vor der Ausgliederung gelesen.
- Die Größenkorrektur benötigt weder Versionsbump noch einen eigenen Release.
  Sie gehört zum selben offenen Kandidaten und derselben späteren Remote-Suite.
- Keine zusätzliche Produktabhängigkeit außerhalb der Schreibfläche ist belegt.
  Main besitzt weiterhin Rootgraphrefresh, Integration, Commit/Push, denselben
  Remote-Vorbereitungseinstieg, anschließende Suitebewertung und terminalen Release.

## Fortsetzung: konkrete Compilerbefunde aus Remote-Lauf 40354605251

### Ziel, Quellen und erster Erhaltungsplan

Die delegierte Fortsetzung vom 26.10.2026 beauftragt die Fehlergruppen C01,
C10, C24, C25, C26, C28, C29, C30, C31, C32, C41, C42, C43 sowie A01,
A03, A04, A05, A06, A07, A14, A15, A16 und A17. Der Quellenkandidat ist
`d95dc404e834edfe7be0e1c96be162b36865efa9`; konkrete Compilerbelege stehen in
`/tmp/smartexplorer-rv1-40354605251/vault-retry-errors.json`. Alle Jobs dieses
Laufs sind laut delegiertem Auftrag beendet. Die neue Scope-Datei ersetzt die
vorige Strukturfläche. Keine lokale Ausführung und keine Abschwächung von
Prüfungen oder Abnahmekriterien.

Ergebnis ist ein einziger wirksamer Recoverydialog mit denselben bestehenden
Sichern-/Entfernen-/Import-/Reaktivierungswegen, kanonische IPCtypen, für die
tatsächlichen Daemonbesitzer erreichbare Retainedmethoden, derselbe
Snapshotfehlerpfad und atomisches Nichtüberschreiben ohne neue Panik unter
Linux/Android. Der erste Plan trennt die belegten Ursachen in Recovery-
Modulidentität, IPC-Namensauflösung, interne Besitzer-Sichtbarkeit, eindeutigen
Stringparameter und den plattformabhängigen FFIflagtyp. Es entsteht keine
neue Nutzerfähigkeit, Persistenzform oder öffentliche API; Tiefe 2 genügt.

### Bestand, Entscheidungen und konkreter Fixplan vor dem Edit

| Belegter Fehler | Aktueller Funktionszusammenhang | Konkrete Korrektur und zu erhaltendes Ergebnis |
|---|---|---|
| C01, C24, C26, C28, C29, C30, C31, C32 | `app/mod.rs` registriert `mount_recovery_ui.rs` direkt, während `mount_ui.rs` dieselbe Datei als privates Kind `recovery_ui` besitzt. `MountUiState::recovery_draft` verwendet ausschließlich den Kindtyp. Die doppelte Appimpl erzeugt deshalb zwei Methoden und zugriffsfremde Draftfelder. | Nur die zusätzliche Direktregistrierung entfernen. Die Datei, das wirksame Kindmodul, Dialogmethoden, Draftfelder, Bestandsdarstellung und sämtliche Aktionen bleiben. Statt des fragilen Prelude-Globimports wird die tatsächlich benutzte `egui`-Bindung kanonisch aus `crate::app::prelude` importiert; `App` und `MountUiResult` bleiben ausdrücklich aus dem wirklichen Elternmodul importiert. |
| C10, A01 | `MountHostRequest` ist in `daemon::drive_modules::ipc_protocol_mount_host` definiert; `ipc_protocol` besitzt keinen solchen Reexport. | Den Hostclient direkt an diese kanonische Enum anbinden. Keine neue Enum, kein geändertes Drahtformat, keine Änderung von Capacity, Lock oder Gerätebaumoperationen. |
| A03, A04, A05, A06, A07 | Alle vier Recoveryclientoperationen und ihr gemeinsamer Requestparameter nennen fälschlich `ipc_protocol::DriveIpcRequest`. Der Typ des Drahtfelds `IpcRequest::Drive.req` liegt in `drive_modules::ipc_protocol_drives`. | Ein Import des tatsächlich definierten Typs und entsprechende kurze Referenzen. SaveRemove, List, Import und Reactivate bleiben vollständig erhalten; Server, Token, Antworten und Fehlerverhalten bleiben gleich. |
| C41, C42, C43, A14, A15, A16 | Methoden im Kindmodul `mount_manager::start_path` sind `pub(super)`, ihre Aufrufer liegen aber in `daemon::ipc` und `daemon::drive_modules::devices_mount`. | Nur `start`, `start_retained` und `retry_retained` erhalten `pub(in crate::daemon)`. Die Freigabe endet am tatsächlichen Daemonbesitzer; kein `pub`, keine weitergereichte Capability und keine Änderung der Methoden-/Leasekörper. |
| C25 | `drain_drive_index` übergibt `.into()` an einen bereits generischen `impl Into<String>`-Parameter. Der Compiler kann dessen Zwischentyp nicht eindeutig bestimmen. | Den unveränderten Fehlertext als `&str` übergeben; Meldung, früher Return, Deltaabbruch und Fehlerzweig bleiben vollständig bestehen. |
| A17 | libc 0.2.186 definiert `RENAME_NOREPLACE` auf Android als `c_int`, auf Linux als `c_uint`; `renameat2` verlangt `c_uint`. | Nur das unverändert aktive Flagbit explizit auf `libc::c_uint` abbilden. Keine `unwrap`-Konversion, kein Check-then-rename und kein überschreibender Ersatzpfad. Gepinnter Parent, Dateinamen, Fehlerweitergabe und Directory-fsync bleiben. |

Lösungsrecherche: Die Rustreferenz zur Sichtbarkeit wurde erneut auf die
tatsächliche Elternhierarchie angewendet. Die offizielle
[E0283-Erklärung](https://doc.rust-lang.org/error_codes/E0283.html) und der
gerenderte Compilerhinweis belegen die unnötig mehrdeutige Zwischenkonversion.
Für libc wurde die im Lockfile festgehaltene Version 0.2.186 geprüft:
[Androidquelle](https://raw.githubusercontent.com/rust-lang/libc/0.2.186/src/unix/linux_like/android/mod.rs)
definiert Flagwert 1 als `c_int` und den Funktionsparameter als `c_uint`;
die [Linuxquelle](https://raw.githubusercontent.com/rust-lang/libc/0.2.186/src/unix/linux_like/linux/mod.rs)
definiert denselben Wert als `c_uint`. Der
[Linux-Systemaufrufvertrag](https://man7.org/linux/man-pages/man2/rename.2.html)
bewahrt das Nichtüberschreiben mit `RENAME_NOREPLACE` und meldet eine Kollision
als Fehler. Abrufdatum aller ergänzten Quellen: 26.10.2026. Die zunächst
angefragten docs.rs-Androidseiten waren nicht abrufbar; die versionsgebundene
offizielle Quelle schließt die benötigte Typlücke.

Zweiter Lückenabgleich vor der Umsetzung: Die Recoverydraftfelder bleiben
privat, weil der einzig beibehaltene Dialog und der einzig verwendete Drafttyp
aus demselben Kindmodul stammen. Eine Feldfreigabe würde die falsche
Doppelinstanz verdecken und wird nicht eingeführt. `ipc_protocol.rs` benennt
die kanonischen Untermodultypen bereits in seinen Drahtfeldern; zusätzliche
Hubreexporte sind unnötig. `pub(in crate::daemon)` erreicht beide konkreten
Aufrufer über deren gemeinsamen Vorfahren und lässt den Prozess-/Hostbesitz
unverändert. Das Flag hat in beiden Zieldefinitionen den positiven Wert 1;
die ABItypabbildung ist daher ohne Laufzeitprüfung oder Panik möglich.

Die zusätzliche Abstimmung mit Main sieht vor, nur den Recoveryhunk in
`app/mod.rs` zuerst abzuschließen und diese Datei danach für den getrennten
Captureautor freizugeben. Viewerregistrierungen gehören nicht zu diesem Fix;
sie werden hier weder editiert noch geprüft. Die verbleibenden Recoverydateien
werden unabhängig weiterbearbeitet. Der spätere Remote-Nachweis verbleibt
vollständig bei Main im selben Suite-Fixloop.

### Ergänzte eigene Funktion-/Verknüpfungskarte

| Grenze | Vollständige betroffene Verbindung | Erhaltener Vertrag |
|---|---|---|
| Registrierung → einziger Dialogtyp | `app/mod.rs` bindet weiterhin `mount_ui` ein; allein dessen privates Kind `recovery_ui` lädt `mount_recovery_ui.rs`. `MountUiState::recovery_draft`, `RecoveryDraft::new` und `App::ui_mount_recovery` beziehen sich dadurch auf dieselbe Definition. `ui_mount_windows` ruft den Dialog weiter auf. | Kein zweiter Appmethodenkörper und keine zweite Draftidentität. `id`, `label`, `destination` bleiben privat; kein Feldzugriff wird nur für die Kompilierung freigegeben. `egui` stammt ausdrücklich aus dem vorhandenen Eframe-Reexport in `app::prelude`. Dialoggeometrie und Themenlimit bleiben am bisherigen Elternpfad. |
| Dialog → erfolgreiche oder fehlgeschlagene Aktion | Der vorhandene Knopf „Sichern und entfernen“ legt den Draft an; der Dialog verlangt einen absoluten Zielpfad und freie Aktion → `spawn_mount_action` → `save_remove_mount` → `MountUiResult::RecoverySaved` → `drain_mount_ui`. Schließen verwirft nur den Draft. | Zielwahl, Busy-Sperre, bestehende Warntexte, asynchroner Ablauf und Fehleranzeige bleiben. Der aktive UIeintrag wird weiterhin erst nach der erfolgreichen Antwort entfernt und der erhaltene Recoveryrecord hinzugefügt. Die saubere „Entfernen“-Aktion und die nichtsaubere Sicherungsaktion bleiben getrennt. |
| Recoverybestand → Sichtbarkeit und Reaktivierung | „Inaktiven Recoverybestand anzeigen“ → `list_mount_recoveries` → `RecoveryList` → `recovery_ui::show_retained`. Der Bestand zeigt Paket, Originalcache, Hash, Lücken, Versiegelung, Reaktivierungszustand und den bestehenden CLIreaktivierungsbefehl. | Die Zusammenführung entfernt weder Bestandsansicht noch Hinweise zu ungesicherten Inhalten. Sie erfindet keine bislang fehlenden Import-/Reaktivierungsbuttons. Die vorhandenen Import-/Reaktivierungsfunktionen bleiben über ihre Client-/IPCwege erhalten. |
| Client → derselbe Drahttyp | `save_remove_mount`, `list_mount_recoveries`, `import_mount_recovery`, `reactivate_mount_recovery` → `recovery_request(DriveIpcRequest)` → `IpcRequest::Drive { token, req }`. Clientimport und Drahtfeld verweisen auf dieselbe Definition in `drive_modules::ipc_protocol_drives`. | RecoverySaveRemove, RecoveryList, RecoveryImport und RecoveryReactivate behalten sämtliche Felder. Workerbereitschaft, Token, Loopbackstream, Kontrollreserve, Entfall eines festen Antworttimeouts während der Ownershiptransaktion und Fehlerweitergabe bleiben identisch. |
| Drahttyp → Recoverybesitzer | `ipc_protocol_drives::serve` → `MountManager::{save_remove,recovery_list,recovery_import,recovery_reactivate}`; Rückweg über `IpcResponse::{MountRecovery,MountRecoveries,Mount,Err}`. | Keine neue Enum, kein neues serde-Tag und keine entfernte Matchvariante. Die kanonischen Protokolldefinitionen sind unverändert; die Korrektur betrifft ausschließlich den Verweis des Clients. |
| SaveRemove → Paket und Ownershipcommit | `save_remove` hält Lifecycle, lehnt laufenden Child-/Streambesitz ab, bestimmt Cache-IDs, hält vorhandene Cacheleases und die historische Backend-/Vaultidentität → `export_mount_recovery` → `mount_registry::inactive::detach` bei gehaltenem State → Entfernen des aktiven Entries und `forget_mount_binding`. | Originalcache und Leasebesitz bleiben bis zum bestehenden Ownershipabschluss erhalten. Die aktive Registrierung wird erst nach Paketpublikation geändert. Ein vorgelagerter Fehler führt nicht zu einer erfolgreichen Entfernung. Keine Schlüssel- oder Klartextfreigabe wird ergänzt. |
| Import/Reaktivierung → verifizierter privater Bestand | `recovery_import` → `copy_recovery_package` → privater Appbestand → Inaktivrecord. `recovery_reactivate` prüft freie Mount-ID, Manifesthash, Google-Kontoidentität und Vaultbindung → `restore_recovery_cache` → Registryreaktivierung → `registered_entry(..., Unknown, ...)`. | Fremdes Originalpaket und Cache werden nicht gelöscht; Import verbindet keinen Mount. Reaktivierung bleibt ausdrücklich, gestoppt und ohne Auto-remount. Fehlende historische Freigaben erteilen kein Trusted. Bestehende Identitätskonflikte und Wiederholbarkeit bleiben erhalten. |
| Hostkapazität → kanonische Hostenum | `MountHostSession::report_capacity` → `MountHostRequest::Capacity` aus `drive_modules::ipc_protocol_mount_host` → `IpcRequest::MountHostExt.req`. Die gleiche Enum trägt unverändert PrepareLock, ChildBackend und DeviceWatch. | Nur der Import wird korrigiert. Status-/Recoverymeldungen, Backend-/Controlstreams, Spoolschlüssel, Vaultlock und Gerätebaumrecht bleiben unverändert. Die Protokollenum selbst wird nicht umbenannt, dupliziert oder exportseitig erweitert. |
| Daemonbesitzer → Start/Retry mit Lease | `devices_mount::enable` → `MountManager::start` → `start_retained`; `ipc::handle_client` nimmt die bestehende Runtimelease an und ruft `start_retained` oder `retry_retained` mit demselben `Arc<dyn Send + Sync>` auf. Beide erreichen die bisherigen privaten Start-/Retrykoordinatoren. | Genau diese drei Methoden sind für den gemeinsamen Vorfahren `crate::daemon` sichtbar. Ihre Körper, Lifecycle-/Registry-/Policygates, die atomare RA-07-Übergabe sowie `StartEnd` und `StartupLease` bleiben unverändert. Es entsteht kein öffentlicher Startbypass und keine verkürzte Leaselebensdauer. |
| Indexworkerende → derselbe Appfehler | `drain_drive_index` nimmt den Deltaempfänger; bei getrenntem Kanal folgt derselbe `push_app_error("Laufwerksindex", "Indexworker wurde beendet")` und derselbe frühe Return. | Die unnötige Zwischenkonversion entfällt. Text, Meldungskategorie, Delta-Drop/Cancel und Kontrollfluss bleiben. Der Empty-Zweig hält den Delta wie bisher, der Complete-Zweig validiert wie bisher Identität/Generation. |
| Paket-/Cachecommit → gemeinsame NoReplace-Grenze | `export_mount_recovery` publiziert das vollständige Paket mit `publish(..., false)` nach Payload-/Manifest-/Namespacebestätigung. `copy_recovery_package` publiziert die verifizierte private Kopie ebenfalls mit false. `restore_recovery_cache` publiziert einen fehlenden Basisordner und vollständig vorbereitete Rootstages jeweils mit false. | Alle benötigten NoReplace-Aufrufer bleiben vorhanden und unverändert. Vorhandene Cacheziele werden vor dem Kopieren gegen ihren Subtree geprüft; ein nachträglich erscheinendes Ziel kann den atomaren Kernel-Nichtüberschreibschutz nicht umgehen. Keine lokale Existenzvorprüfung ersetzt den Renameaufruf. |
| NoReplace → Linux/Android-ABI und Dauerhaftigkeit | `mount/mod.rs` wählt außerhalb Windows weiterhin `os/linux_os/recovery_storage.rs`. Dessen `publish` verlangt denselben Parent, hält dessen `DirectoryHandle`, bildet beide Literalnamen als CStrings und ruft `renameat2(fd, source, fd, target, flags)` auf. `replace=false` liefert den unverändert gesetzten Flagwert 1 im Typ `c_uint`; `replace=true` bleibt 0. Danach bleiben errno-Fehler und Parent-fsync maßgeblich. | Keine Konversion mit `unwrap`/`expect`, kein überschreibender Rückfall und keine zusätzliche Pfadauflösung. Konflikte und fehlende Dateisystemunterstützung bleiben echte Fehler. Unter Android wird ausschließlich der bisher inkompatible Cintegertyp korrigiert. |

### Tatsächlich gelesene und geänderte Dateien dieser Fortsetzung

Zusätzlich beziehungsweise erneut gelesen wurden die genaue Scope-Datei, der
wörtliche Nutzervertrag, die vollständige Compilerdiagnose, der eigene Bericht,
`rv1-modulplan.md` (Mount-/Runtimeabschnitt), `Cargo.toml` und `Cargo.lock`
(Eframe-/egui- und libc-Zuordnung) sowie der freigegebene Rootgraph ausschließlich
lesend. Die aktuelle Graphabfrage lautete
`graphify query 'DriveIpcRequest MountHostRequest mount_client_host mount_manager_start RecoveryDraft' --budget 1000`.
Die konkrete Quellenprüfung bestimmt die vorstehende Karte; die Graphabfrage
ist kein Compilerbeleg.

Die produktive Quellenlektüre umfasst die benötigten Abschnitte von
`app/mod.rs`, `app/drive_modules.rs`, `app/core/{drive_snapshot,mount_ui,mount_recovery_ui,prelude,theme}.rs`,
`daemon/{mod,drive_modules}.rs`,
`daemon/os/shared/{devices_mount,ipc,ipc_protocol,ipc_protocol_drives,ipc_protocol_mount_host,mount_client,mount_client_host,mount_manager_start,mount_recovery_transaction}.rs`,
`mount/mod.rs`, `mount/core/{recovery_state,recovery_manifest}.rs` und
`mount/os/linux_os/recovery_storage.rs`. Außerdem wurden die konkreten
Publikations-/Verifikations-/Restorekörper in
`mount/os/shared/{recovery_export,recovery_import}.rs` und die gesamte
`mount/os/shared/batches_recovery_task_tests.rs` gelesen. Quellpfade sind
relativ zu `native/src/`; Abschnittlektüre wird nicht als erneute Volllektüre
des gesamten jeweiligen Moduls behauptet.

Eigene Änderungen:

| Datei | Abgeschlossene Änderung | Quellgröße nach diesem Fix |
|---|---|---|
| `native/src/app/mod.rs` | Nur zusätzliche Direktregistrierung der Recoverydatei entfernt; exakt zwei Zeilen | 405 → 403 Zeilen unmittelbar bei Übergabe; spätere Viewerhunks gehören dem Captureautor |
| `native/src/app/core/mount_recovery_ui.rs` | `egui` ausdrücklich aus dem kanonischen Prelude importiert | 98 Zeilen / 4.330 Byte |
| `native/src/app/core/drive_snapshot.rs` | Überflüssige `.into()`-Zwischenkonversion am unveränderten Fehlertext entfernt | 371 Zeilen / 12.644 Byte |
| `native/src/daemon/os/shared/mount_client.rs` | Ein kanonischer DriveIpcRequest-Import für alle vier Recoveryoperationen und ihren gemeinsamen Parameter | 130 Zeilen / 5.480 Byte |
| `native/src/daemon/os/shared/mount_client_host.rs` | Kanonischer MountHostRequest-Import | 396 Zeilen / 14.432 Byte |
| `native/src/daemon/os/shared/mount_manager_start.rs` | Nur drei tatsächliche Besitzer-Sichtbarkeiten korrigiert | 463 Zeilen / 18.420 Byte |
| `native/src/mount/os/linux_os/recovery_storage.rs` | NoReplace-Flag im plattformübergreifend richtigen ABItyp ohne Panikkonversion | 56 Zeilen / 1.862 Byte |
| dieser Bericht | Fehlerauflistung, Erhaltungsplan, aktuelle Karte und eigene Nachreview | keine native Größenregel |

`mount_ui.rs`, `devices_mount.rs`, `ipc.rs`, alle kanonischen Protokolldefinitionen,
Export-/Importkörper und Fixturedateien wurden nicht verändert.
`daemon/mod.rs` blieb ausschließlich lesbar beim Geräteautor. Main hat den
eigenen exakten Zwei-Zeilen-Hunk aus `app/mod.rs` gesichert und dessen
Schreibfläche danach ausdrücklich dem Captureautor übertragen. Nach dieser
Übergabe wurde `app/mod.rs` von diesem Autor nicht mehr bearbeitet. Der eigene
abgeschlossene Recoveryhunk bleibt Bestandteil dieses Fixberichts.

### Eigene anschließende Anti-Regressionsreview und Grenzen

Die eigene Review nach der Umsetzung hat sämtliche zugewiesenen Fehlergruppen
gegen ihre konkreten Definitionen und Verbraucher verfolgt:

1. **Recoveryidentität:** Die aktuelle Registrierungssuche findet die Datei nur
   noch als `mount_ui::recovery_ui`. Draft, Dialog, sichtbare Aktion und
   Rückmeldung verwenden denselben Typ. Kein Feld wurde geöffnet und kein
   Dialog-/Bestands-/Clientkörper entfernt. Der explizite `egui`-Import ist
   erforderlich: Das vorhandene Prelude bindet `eframe::egui` ein, und
   Cargo.toml enthält keinen direkten `egui`-Cratenamen für einen unqualifizierten
   Ersatz. Dieser Befund wurde vor Abschluss in den finalen Import übernommen.
2. **Kanonische Typen:** Der konkrete DriveRequest des Clients ist derselbe
   wie das `IpcRequest::Drive.req`-Feld. Der konkrete HostRequest ist derselbe
   wie `MountHostExt.req`. Statische Vergleiche gegen `d95dc404` bestätigen
   unveränderte Definitionen in `ipc_protocol.rs`, `ipc_protocol_drives.rs`
   und `ipc_protocol_mount_host.rs`. Es wurde keine Wirebehauptung oder
   Enumvariante zur Kompilierung entfernt.
3. **Ressourcenbesitz:** Ein Textvergleich gegen `d95dc404` bestätigt, dass
   `mount_manager_start.rs` ausschließlich die drei dokumentierten
   Sichtbarkeitsänderungen enthält. Der Retainedleaseparameter, seine Weitergabe,
   Lifecyclelocks und sämtliche Körper stimmen überein. Die tatsächlichen
   Aufrufer sind Nachfahren von `crate::daemon`; eine weitergehende Freigabe
   ist nicht nötig und wurde nicht erteilt.
4. **Unveränderte Körper:** Entsprechende statische Vergleiche bestätigen
   im Recoveryclient ausschließlich Import-/Typreferenzanpassungen, im
   Hostclient ausschließlich den kanonischen Import, im Recoverydialog
   ausschließlich den Preludeimport und im Snapshot ausschließlich die
   überflüssige Zwischenkonversion. Save/Remove/Import/Reactivate, alle
   Antwortvarianten und der Snapshotreturn sind erhalten.
5. **Atomisches Nichtüberschreiben:** Der neue Wert hat denselben dokumentierten
   Flagbitwert 1. Beide Funktionsparameter- und Konstantentypen wurden in der
   tatsächlich gelockten libc-Version gelesen. Nur Android erhält die
   notwendige signed/unsigned-Abbildung; Linux verwendet seine bereits
   korrekt typisierte Konstante direkt. Der einzige unsichere Aufruf
   behält seine gehaltenen FDs und Namen; `RENAME_NOREPLACE`, Fehlerweitergabe
   und Directory-fsync bleiben. Weder `.unwrap()` noch `.expect()` wurde
   eingeführt. Die Export-/Import-/Restoreaufrufer behalten ihre Argumente
   `replace=false`; somit wird keine bestehende Zielkollision als
   Überschreibberechtigung interpretiert.
6. **Keine abgeschwächte Prüfung:** Native Quellen bleiben mit normalem Layout
   unter 500 Zeilen und 50 KiB. Die gezielte Whitespacekontrolle der sechs
   weiter selbst besessenen Produktdateien meldete keinen Befund. Tests,
   Testnamen, Manifestauswahl, Assertions und Gates wurden nicht verändert.
   Der neue Compilerbefund ergänzt die früheren statischen Körpervergleiche
   um die jetzt konkret korrigierten Namens-/Sichtbarkeits-/FFIfehler; er wird
   weder als Infrastrukturfehler umgedeutet noch durch eine Erfolgsaussage ersetzt.

Die Korrekturen zu allen aufgeführten Diagnose-IDs sind quellenmäßig umgesetzt.
Die erneute Compilerbestätigung für Windows, Linux und Android und die
Funktionsbestätigung stehen aus. Dieser Autor hat keine lokalen Builds,
Compiler, Tests, Formatter, Server, Installationen, Hostprozesse, Gitmutationen,
Graphupdates oder Unteragenten ausgeführt. Die nur lesende Graphabfrage ist
beendet. Es gibt keinen aus diesem Autorzyklus abgeleiteten Kompilier- oder
Funktionspass.

### Bestehende Fallnamen und offene Integration

Es wurde keine Fixturedatei geschrieben und kein neuer Fall vorbereitet.
Die vorhandene gemeinsame Auswahl `batches_task_recovery_` deckt weiterhin die
tatsächlich gelesenen Namen ab:

- `batches_task_recovery_offline_export_import_preserves_raw_bytes_after_source_removal`
- `batches_task_recovery_tamper_and_conflicting_cache_preserve_originals`
- `batches_task_recovery_sealed_bytes_report_unknown_inventory_without_invented_entries`
- `batches_task_recovery_export_refuses_symlink_without_reading_external_payload`

Der letzte Fall bleibt unverändert Unixgebunden. Seine Existenz ist kein
Laufzeitnachweis. Die bestehenden Wirefälle `drive_requests_keep_their_wire_form`,
`devices_wire_requests_keep_their_form` und `mount_host_ext_wire_form` bleiben
unverändert an ihren bisherigen Protokollmodulen. Die bestehenden RA-07-Namen
und Snapshotnamen aus den vorangegangenen Abschnitten bleiben ebenfalls erhalten.
Es wird keine neue Suite und kein Einzelaufruf verlangt; Main bewertet die
erneute Kompilierung und Funktionsausführung im selben Suite-Fixloop.

Offen bleiben ausschließlich die von Main besessene Integration des übergebenen
Appregistrierungshunks mit dem getrennten Viewerhunk, Rootgraphrefresh,
Remotebestätigung und der gemeinsame terminale Release. Keine weitere
Produktabhängigkeit oder ungeklärte notwendige Nutzerangabe ist in der eigenen
Fläche belegt. Die drei zusätzlich angefragten Recoveryquellen und der Rootgraph
wurden von Main konkret lesend freigegeben; die Schreibfläche wurde nicht
eigenmächtig erweitert.

## D04/D05 nach Remote-Lauf 40368746815: Kapazitäts- und Leserstatusanzeige

### Auftrag, Fehlerfunktionen und abgegrenzter Erstplan

Quelle dieses ergänzenden Ergebnisauftrags ist die Fortsetzung von Main vom
2026-10-26 auf Kandidat `b233e5336a0ee177e19382a7adf592d297dcf202`.
Der unveränderte Nutzervertrag verlangt Fehlerauflistung, vollständige direkte
und indirekte Verknüpfungskarte, konkreten Erhaltungsplan, Fix und anschließende
eigene Anti-Regressionsreview durch denselben Autor. Dieser Abschnitt setzt
diesen Zyklus für genau D04 und D05 fort; frühere Abschnitte dokumentieren die
vorangegangenen Kandidaten und deren damalige Grenzen.

Promptwissen: Beide Anzeigen sollen über dieselbe bestehende egui-Instanz
nutzbar bleiben. Kapazitätswerte, Unbekanntzustände, Leser-/Wartemodi und
Auftragszahlen müssen erhalten bleiben; eine neue GUI-Abhängigkeit ist
ausgeschlossen. Schreibbar sind ausschließlich `mount_capacity_ui.rs`,
`optical_status.rs` unter `native/src/app/core/` und dieser Bericht. Main hat
den Dokumentationskontextgate erledigt und besitzt Manifest, Git, Rootgraph,
gemeinsame Remote-Suite und Release. Die Architektur- und Quellenlektüre
klärt die konkreten Module, Datenquellen, Anzeigezweige und vorhandenen Fälle;
eine notwendige nur vom Nutzer lieferbare Angabe ist nicht benannt.

Die gelieferte Diagnosequelle
`/tmp/smartexplorer-rv1-40368746815/vault-retry-errors.json` wurde vollständig
gelesen. Sie enthält genau zwei E0433-Diagnosen, jeweils aus Windows- und
Linux-Produkt-/Testkompilierung von `smart_explorer`; `android_errors` ist leer.
Diese leere Liste ist kein Android-Funktionsnachweis.

| Diagnose | Fehlerhafte Funktion im Kandidaten | Konkreter Befund und notwendiger Erhalt |
|---|---|---|
| D04 | `app::mount_ui::capacity_ui::show` in `mount_capacity_ui.rs:2` | `egui::Ui` ohne egui-Import; Kapazitätsdetails des vorhandenen Laufwerksdialogs müssen ihren Report und die beiden Kein-Hostbericht-Zeilen behalten. |
| D05 | `app::transfer_window::optical_status::render` in `optical_status.rs:2` | `egui::Ui` ohne egui-Import; das vorhandene Übertragungsfenster muss Prozessleser, Wartemodi, optionale Blockgröße und beide Wartezähler behalten. |

### Belegter Bestand und vollständige betroffene Verknüpfungskarte

Die Karte verfolgt die beiden geänderten Modul-/Typgrenzen über ihre konkreten
Aufrufer bis zu ihren Datenmodellen und Statusproduzenten. Providerzugriff,
Daemon-IPCtransport und OS-Gerätesperren sind benannte unveränderte Grenzen;
ihre vollständigen Implementierungen werden durch diesen Importfix weder
verändert noch als neu praktisch geprüft behauptet.

| Direkte oder indirekte Kante | Konkrete Quellen/Funktionen | Zu erhaltender Vertrag |
|---|---|---|
| Ein bestehender GUItyp | `app/mod.rs` registriert `prelude`; `app/core/prelude.rs` reexportiert `eframe::egui::{self, Color32, RichText}` mit `pub(in crate::app)`; `mount_ui.rs` und `transfer_window.rs` verwenden dieses Prelude | Beide Kindmodule müssen genau denselben `egui::Ui`-Typ wie ihre Eltern verwenden; der Reexport ist für Nachfahren von `crate::app` zugänglich. |
| Laufwerkdialog und Kindregistrierung | `app/mod.rs → mount_ui`; `mount_ui.rs → capacity_ui` über `#[path = "mount_capacity_ui.rs"]`; `App::ui_mount_windows → ui_mount_manager → capacity_ui::show` | Bestehendes Fenster „Smart-Explorer-Laufwerke“, gruppierte Laufwerkdetails und IDbezogene Aufklappzustände bleiben; der geborgte `Ui` wird weitergereicht. |
| Eingang der Kapazität | `App::drain_mount_ui` übernimmt `MountUiResult::{Snapshot,List,Options}` in `mount_ui.mounts`; `mount/core/recovery_state.rs::MountSnapshot.capacity` ist `Option<MountCapacityReport>` mit serde-Default | Ein fehlender Hostbericht bleibt fehlend. Deserialisierung, Recoverystatus, Mount-ID, Listenaustausch und Aktionen werden nicht verändert. |
| Anzeige eines Reports | `capacity_ui::show → MountCapacityReport::display_lines → with_freshness` in `mount/core/capacity.rs` | Sämtliche erzeugten Zeilen werden angezeigt: Remoteverfügbarkeit, Cacheverfügbarkeit, konservativ nutzbares Budget, Quellen-/Erhebungsangaben und OS-Anzeige; zusätzliche Nur-Lesen-, Providerfehler- und Cachefehlerzeilen bleiben erhalten. |
| Kapazitätsprojektion und Unbekanntzustände | `MountCapacityReport::{from_observation,validate}`, `MountCapacity::from_budget` und die referenzierten `vfs::{CapacitySnapshot,CapacitySource,CapacityState,CapacityFailure}` | Bekannte Quote, Accountquote, Unbounded, Unknown, Unavailable und CacheCompatibility bleiben verschieden. Cachebudget ersetzt keine Remotequote; ein Speicherfehler erfindet kein Budget. Reine Anzeige trifft keine Schreibzulassungsentscheidung. |
| Fehlender Report | `capacity_ui::show`, `mount.capacity == None` | Beide Texte „Remote verfügbar: unbekannt (kein Hostbericht)“ und „Lokaler Zwischenspeicher verfügbar: unbekannt (kein Hostbericht)“ bleiben. |
| Übertragungsfenster und Kindregistrierung | `app/mod.rs → transfer_window`; `transfer_window.rs → optical_status` über `#[path = "optical_status.rs"]`; `ui_transfers_button → ui_transfer_window → window_body → optical_status::render` | Das Fenster bleibt ausdrücklich geöffnet. Die Leseranzeige steht weiterhin vor dem Leerzustand bei `total_count() == 0`; aktive Prozessleser verschwinden dadurch nicht bei leerer Übertragungsliste. |
| Prozessbezogene Lesersicht | `optical_status::render → source_media::snapshots`, reexportiert aus `source_media/status.rs`; `snapshots → registry` | Ein kurzer Mutexzugriff liefert geklonte `Status`-Werte. Keine Speicher-/Netzwerk-/Geräteprobe wird durch das Rendering gestartet; keine Sperre bleibt während egui-Aufrufen gehalten. |
| Statusmodell und sichtbare Werte | `source_media::Status`, `ReadMode`, `Source` aus `source_media/{status,mod}.rs` | Gerätebezeichnung; inaktiver Leser als „wartet auf Laufwerk“; aktive Modi Waiting, Exclusive, Sequential, MetadataFallback; optionale `block_bytes`; getrennte `foreground_waiters` und `background_waiters`; Hinweis auf fehlende Zuordnung zu einem einzelnen Kopierauftrag. |
| Besitzer und Warteschlange | `source_media/permit.rs::{acquire,acquire_current,acquire_source,acquire_file,acquire_file_current} → acquire_checked`; `Waiting::drop`, `Guard::{set_mode,drop}`; `source_media::mark_mode → status::update` | Zähler werden bei Anmeldung erhöht und bei Ende der Wartephase abgesenkt; Gerätebesitz aktiviert Exclusive, Moduswechsel erhalten die bestehende Blockgrößenlogik, Guardende setzt Waiting und löscht die Blockgröße. `update` entfernt nur inaktive Einträge ohne wartende Aufträge. |
| Medienidentität, Abbruch und OSgrenze | `Source::{device,generation,read_only}`, `Guard::{validate,validate_file}`, `with_cancellation`, `context_canceled`, `source_media/mod.rs::platform`-Registrierungen | Geräte-/Medienidentität, Priorität, Abbruch und tatsächliche OSsperren bleiben bei ihren vorhandenen Produzenten. Die Anzeige nimmt keinen Besitz und ändert keine Freigabe. |
| Desktop-/Androidgrenze | `native/Cargo.toml` führt eframe unter `cfg(not(target_os = "android"))`; Architektur ordnet `app/` dem Desktop und `mobile/` der Kotlinbrücke zu; `app/drive_modules.rs` registriert vorhandenen optischen Index | Kein egui-Import wird in `source_media/`, `mount/`, `vfs/` oder die mobile Brücke verschoben. Android erhält keine neue GUIabhängigkeit. CLI-/MCP-Datenverträge bleiben unverändert. |

Die gezielte lesende Rootgraphabfrage lautete
`graphify query 'mount_capacity_ui show MountCapacityReport display_lines optical_status render source_media snapshots prelude egui' --budget 1000`.
Sie bestätigte die relevanten Datei-/Typknoten, lieferte wegen der allgemeinen
Methodennamen auch sachfremde Knoten und wurde am Ausgabelimit gekürzt.
Die vorstehende vollständige betroffene Aufrufkarte stammt deshalb aus den
konkreten registrierten Quellen; der Graph ersetzt keinen Aufruf- oder Typbeleg.
Die CLI meldete erneut Skill 0.9.11 gegenüber Paket 0.9.17. Der Aufruf ist
beendet; kein Graphneubau und keine Installation wurden ausgeführt.

### Entscheidungsrecherche und konkreter Fixplan vor dem Quellfix

Bestand: Cargo.toml nennt eframe 0.29, Cargo.lock löst eframe und egui jeweils
auf 0.29.1 auf. Das App-Prelude bindet bereits genau diesen egui-Reexport ein.
Beide Kinddateien haben bisher überhaupt keinen Import. Ein im Elternmodul
vorhandener Kurzname ist kein ausdrücklich eingebundener Kurzname dieser
Kinddateien; genau dort belegt der Compiler E0433.

Die offizielle [Rust-Erklärung zu E0433](https://doc.rust-lang.org/error_codes/E0433.html)
und die [Referenz zu lokalen `use`-Bindungen und Reexporten](https://doc.rust-lang.org/reference/items/use-declarations.html)
belegen die zulässige Importkorrektur. Die tatsächlich gelockte
[eframe-0.29.1-Quelle](https://raw.githubusercontent.com/emilk/egui/0.29.1/crates/eframe/src/lib.rs)
reexportiert egui ausdrücklich und verwendet denselben Pfad in ihrem Beispiel.
Abrufstand: 2026-10-26. Die entsprechende docs.rs-Indexseite war über das
Webwerkzeug nicht erreichbar; die offizielle versionierte Quelle schließt
diese Vertragslücke. Es wird keine neue API verwendet.

Entscheidung: Beide Kinddateien erhalten ausschließlich
`use crate::app::prelude::egui;` zwischen ihrer bisherigen Modulbeschreibung
und Funktion, mit normaler Leerzeilentrennung. Damit verwenden sie denselben
App-internen Typzugang wie die vorhandenen Aufrufer. Ein direkter
`use eframe::egui` wäre ebenfalls dieselbe Bibliotheksinstanz, würde aber den
bereits vorhandenen Appzugang umgehen. Ein Eltern-Globimport würde unnötige
Namen einführen. Eine zusätzliche direkte Cargo-Abhängigkeit ist für einen
vorhandenen Reexport nicht erforderlich und vom Ergebnisauftrag ausgeschlossen.
Entfernen oder Ausblenden der Anzeigen würde ihre Endnutzerfunktion verlieren
und ist kein zulässiger Lösungsweg.

| Arbeitspunkt vor Umsetzung | Schreibfläche und Abhängigkeit | Konkretes erwartetes Ergebnis und Erhalt |
|---|---|---|
| D04import | `native/src/app/core/mount_capacity_ui.rs`; vorhandenes App-Prelude | `show` bindet denselben GUItyp wie `ui_mount_manager`. Nach Entfernen ausschließlich der neuen Importzeile samt Leerzeile ist der gesamte Dateiinhalt identisch zu `b233e533`; Report- und Kein-Report-Zweige bleiben vollständig erhalten. |
| D05import | `native/src/app/core/optical_status.rs`; dasselbe Prelude | `render` bindet denselben GUItyp wie `window_body`. Derselbe statische Inhaltsvergleich erhält sämtliche sichtbaren Texte, Moduszweige, Wartezähler, Blockgröße und das Leerlaufreturn. |
| Eigene anschließende Anti-Regressionsreview | Beide vollständigen fertigen Helfer; oben benannte Aufrufer, Modelle und Registrierung; dieser Bericht | Nach dem Quellfix Typ-/Modulweg und beide Körper erneut lesen, reine Importdifferenz zum Kandidaten sowie Whitespace/Dateigröße statisch kontrollieren. Keine lokale Compiler-/Formatter-/Testausführung. |
| Vorhandene Fallauswahl und Übergabe | Nur lesendes `native/review-task-manifest.json`; vorhandene Kapazitäts- und optische Fälle | Bestehende `batches_task_capacity_`- und `batches_task_optical_`-Auswahl erhalten; keine neuen Tests für zwei reversible Importbindungen. Exakte gelesene Namen und verbleibender Remotenachweis folgen nach der eigenen Review. |

Zweite Lückenprüfung vor der Änderung: Die tatsächlich gelockte Reexportversion,
App-interne Sichtbarkeit, beide Kindregistrierungen und die eigentlichen
Darstellungszweige sind belegt. Es gibt keine notwendige zusätzliche API,
keine Persistenzmigration und keine Sicherheits-/Besitzänderung. Die angefragte
ergänzende Laufwerk-Oberflächenquelle und optische Fallquelle betreffen die
vollständige Berichts-/Registrierungszuordnung; die Importentscheidung hängt
nicht von einem unbekannten Wert aus diesen Dateien ab.

### Umsetzung sowie gelesene und geänderte Dateien

Beide geplanten Importkorrekturen sind umgesetzt. Jeder Helfer bindet einmal
`crate::app::prelude::egui` ein. Signaturen, `pub(super)`-Sichtbarkeit und
sämtliche bisherigen Dateiinhalte bleiben erhalten. Die normale Leerzeile
trennt Import und Funktion; es wurde keine Zeile künstlich verdichtet.

| In diesem ergänzenden Zyklus geänderte Datei | Exakte Änderung | Aktuelle Größe |
|---|---|---|
| `native/src/app/core/mount_capacity_ui.rs` | Ein Import und eine Leerzeile nach dem bestehenden Modulkommentar | 13 Zeilen / 498 Byte |
| `native/src/app/core/optical_status.rs` | Derselbe Import und dieselbe Leerzeile | 34 Zeilen / 1.437 Byte |
| `docs/plaene/2026-10-24-batches/fix-vault-retry.md` | Dieser eigene Fehler-/Karten-/Plan-/Umsetzungs-/Reviewabschnitt und präzisierter Status am Anfang | Dokument, keine native Größenregel |

Es wurde keine Datei angelegt. Der Importfix berührt ausschließlich diese
drei aktuell schreibbaren Dateien. Die früheren eigenen Änderungen in diesem
Bericht bleiben ihrem jeweiligen abgeschlossenen Zyklus zugeordnet.

In diesem Zyklus tatsächlich gelesen:

- Vollständig: die aktuelle Scope-Datei, der Nutzervertrag und die oben
  genannte D04/D05-Diagnosequelle; die beiden geänderten Anzeigehelfer;
  `app/core/prelude.rs`, `app/drive_modules.rs`, `mount/core/capacity.rs`,
  `mount/core/recovery_state.rs`,
  `source_media/{mod,status,permit,tests}.rs` unter `native/src/`.
- Gezielte Registrierungs-/Verbraucherabschnitte unter `native/src/`:
  `app/mod.rs` (Prelude, Drive-, Mount- und Transfermodule),
  `app/core/mount_ui.rs` (State, Ergebnisübernahme, Polling und Laufwerkdialog),
  `app/core/transfer_window.rs` (Öffnen, `ui_transfer_window`,
  `apply_row_action`, vollständiger `window_body` und Statusaufruf).
- Dokumentationsabschnitte: `docs/ARCHITEKTUR.md` (Einstiege,
  Kapazitäts-/Recoverygrenze und Desktop-/Androidtrennung),
  `docs/oberflaechen/INDEX.md` (Oberflächen und Themenverweise),
  `docs/oberflaechen/uebertragungen.md` (optische Quellen),
  `docs/oberflaechen/laufwerke.md` (Oberflächen und aktueller
  Kapazitäts-/Recoveryabschnitt), `rv1-compilerplan.md` im selben Planordner
  (D04/D05-Zuordnung) sowie Anfang, bisheriger Abschluss und eigener neuer
  Abschnitt dieses Berichts. Die historischen Laufwerkzeilennummern werden
  nicht als aktueller Codebeleg übernommen; die Anzeigezuordnung wurde an
  den vorstehend genannten aktuellen Quellen nachgelesen.
- Abhängigkeits-/Auswahlabschnitte: `native/Cargo.toml` (vorhandene
  GUIabhängigkeiten und Desktopziel), `native/Cargo.lock` (eframe/egui 0.29.1),
  `native/review-task-manifest.json` (Präfixe, Helperausschluss und
  Meilensteinmarker P11/P23), lesende Rootgraphabfrage wie oben angegeben.
- Offizielle Webquellen: die drei oben verlinkten Rust-/eframequellen mit
  Abrufstand 2026-10-26. Die fehlgeschlagene docs.rs-Abfrage ist als solche
  dokumentiert und wird nicht als gelesener Versionsbeleg verwendet.

Main hat die konkret fehlenden Lesepfade `graphify-out/graph.json`,
`docs/oberflaechen/laufwerke.md` und `native/src/source_media/tests.rs`
ausdrücklich ergänzt. Es erfolgte keine eigenmächtige Scopeerweiterung.

### Eigene anschließende Anti-Regressionsreview

Die folgende Review wurde durch denselben Fixautor nach der Umsetzung
durchgeführt. Beide vollständigen fertigen Helfer und ihr fertiger Diff
wurden erneut gelesen; die Review verwendet die konkrete oben dokumentierte
Aufruf- und Datenkette.

1. **Modul- und Typidentität:** Beide Kindmodule bleiben nur an ihrem
   bisherigen Elternmodul registriert. Der absolute Import erreicht den
   vorhandenen, für `crate::app` freigegebenen eframe-Reexport. Deshalb
   bezeichnet `egui::Ui` in Helfern und Aufrufern dieselbe vorhandene
   Bibliothek. Kein zweiter Context, Frame oder UIbesitzer wird erzeugt.
   Funktionen bleiben `pub(super)`; die Sichtbarkeit des Preludes und der
   Modelle wurde nicht verbreitert.
2. **Kapazitätswerte und Unbekannt:** Der `Some`-Zweig zeigt weiterhin jede
   unveränderte `display_lines`-Zeile. Accountquote, unbekannt, unbegrenzt,
   nicht erreichbar, Cachekompatibilität, read-only, Provider-/Cachefehler und
   getrennte Aktualitätsmarkierungen bleiben über denselben Report erhalten.
   Der `None`-Zweig behält seine beiden Kein-Hostbericht-Zeilen. Es wird weder
   null als unbekannt ausgegeben noch eine Berechtigung aus einer Kapazität
   abgeleitet. `from_observation`, Budgetprojektion und Validierung sind
   unverändert.
3. **Leser- und Wartezustände:** Der bisherige Leerzustand ohne registrierte
   Leser kehrt weiterhin sofort zurück. Jeder registrierte inaktive Leser
   erscheint wartend; alle vier aktiven `ReadMode`-Varianten bleiben explizit
   abgedeckt. Gerätebezeichnung, optionale Blockgröße, ausdrückliche und
   Hintergrundaufträge sowie der Hinweis auf Prozessumfang sind unverändert.
   Der UIhelfer liest nur den geklonten Snapshot und verändert keinen
   Registryeintrag, Medienbeleg oder Guard.
4. **Angeschlossene Bedienwege und Besitz:** Die Laufwerkdetails behalten
   Mount-ID, Recoverywarnung und sämtliche Laufwerksaktionen. Die optische
   Anzeige bleibt im Übertragungsfenster vor dessen Leerzustand erreichbar.
   Laufende/fertige Übertragungszahlen, Abbruch, Wiederaufnahme und Aktionen
   in `transfer_window.rs` bleiben unverändert. Die Registry hält dieselbe
   Geräte-/Generationsidentität, Vordergrundpriorität, Wartezähler,
   Abbruchbehandlung und Freigabe beim Guardende; der Importpfad greift in
   keine dieser Entscheidungen ein.
5. **Statischer Inhaltsbeleg:** Gegen Kandidat `b233e533…` wurde bei jedem
   Helfer ausschließlich die neue Importzeile samt Leerzeile aus dem
   Vergleichstext entfernt. Der verbleibende gesamte Dateiinhalt stimmt
   jeweils exakt überein. Zusätzlich sind die vollständigen Dateien
   `prelude.rs`, `mount_ui.rs`, `transfer_window.rs`, `app/drive_modules.rs`,
   `mount/core/{capacity,recovery_state}.rs`,
   `source_media/{mod,status,permit,tests}.rs`, `native/Cargo.lock` und
   `native/review-task-manifest.json` statisch identisch zum Kandidaten.
   Das ist ein Quellvergleich, kein ausgeführter Funktionstest.
6. **Architektur und Prüfgrenzen:** Beide Rustdateien liegen weit unter
   500 Zeilen und 50 KiB in normalem Layout. Die auf die drei eigenen
   Schreibpfade begrenzte Whitespacekontrolle meldete keinen Befund. Es
   wurde keine Abhängigkeit hinzugefügt, kein Shared-/Androidmodell an egui
   gekoppelt, keine Assertion entfernt und keine Auswahl abgeschwächt.

Die eigene statische Review findet in diesem abgegrenzten Importfix keinen
verbleibenden belegten Verhaltensverlust. Sie bestätigt keine Kompilierung
und keine praktisch betrachtete Oberfläche. Dieser Autor führte keine
lokalen Builds, Compiler, Formatter, Tests, Server, Installationen,
Hostprozesse, Gitmutationen, Graphupdates oder Unteragenten aus.

### Bestehende Fallnamen, Entscheidungen und offene Grenzen

Es wurde kein neuer Test und keine neue Fixture geschrieben. Die bestehende
gemeinsame Auswahl verlangt bereits `batches_task_capacity_` für P23 und
`batches_task_optical_` für P11. Die tatsächlich vollständig gelesenen
Kapazitätsfälle heißen:

- `batches_task_capacity_status_preserves_quota_and_engine_budget`
- `batches_task_capacity_status_never_promotes_compatibility_to_remote_quota`
- `batches_task_capacity_status_unknown_unbounded_and_read_only_remain_distinct`
- `batches_task_capacity_status_marks_expiry_and_preserves_provider_failure`

Die tatsächlich vollständig gelesenen optischen Hauptfälle heißen:

- `batches_task_optical_two_copy_jobs_and_scan_share_device`
- `batches_task_optical_wait_cancel_foreground_priority_and_independent_drive`
- `batches_task_optical_media_change_rejects_open_reader_and_queued_index`
- `batches_task_optical_process_lock_excludes_second_process`
- `batches_task_optical_confined_reader_uses_held_root_and_cancel`

Der ebenfalls gelesene `batches_task_optical_process_child` bleibt absichtlich
aus der direkten Suiteauswahl ausgeschlossen: Der Prozesssperren-Hauptfall
ruft ihn selbst als gebundenen Kindprozess auf. Dieser Ausschluss und die
bestehenden Assertions bleiben erhalten. Die Kapazitätsfälle prüfen den
Reportvertrag; die optischen Fälle prüfen Leserbesitz, Warteschlange und
Quellenschutz. Ihre vorhandenen Namen sind kein Nachweis, dass die Oberfläche
praktisch gerendert wurde.

Main benötigt für diesen Importfix keine zusätzliche Fallregistrierung.
Die erneute Kompilierung der bestehenden Windows-/Linux-Produkt- und
Testziele muss im selben Remote-Suiteeinstieg bestätigen, dass D04/D05
verschwunden sind. Die gemeinsame Funktionsbestätigung bleibt ebenfalls bei
Main; es wird kein Einzeltestlauf oder zusätzlicher Suiteeinstieg vorbereitet.
Der eigene statische Autorzyklus ist abgeschlossen. Gitintegration,
gegebenenfalls erforderlicher Rootgraphrefresh, Remotenachweis und der
gemeinsame terminale Release gehören Main. Eine notwendige zusätzliche
Quelldatei, Schreibfreigabe oder Nutzerentscheidung ist nicht offen.
