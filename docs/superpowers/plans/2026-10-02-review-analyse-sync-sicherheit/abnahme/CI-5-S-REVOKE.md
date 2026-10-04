# CI-5-S-REVOKE: ausdrücklicher Old→New-Leseexport

Stand: 2026-10-04. Die begrenzte Fixturekorrektur ist statisch umgesetzt. Die Verhaltensbestätigung erfolgt ausschließlich durch Roots dieselbe vollständige Remote-RV1-Suite.

## Fundzuordnung und tatsächliche Ursache

Ausgangsevidenz ist der vollständig beendete Run `37167542206`, Kandidat `1378bc8fdb796ac2102ccb1e62e8fb74fe0bd796`; Root hat den zugehörigen Formatterpatch in `36eaafe9` übernommen. Die erste Zuordnung als fehlende FS-Zulassung wurde anhand der tatsächlichen Diagnosen korrigiert:

- `mixed-version.log:204` zeigt auf dem neuen Ziel nach dessen Neustart eine akzeptierte Iroh-Sitzung des Altpeers.
- `mixed-version.log:809–888` zeigt im alten Requester erfolgreiche `list_dir`-Aufrufe mit jeweils null Einträgen. `old-to-new-root-listing.txt` ist leer; die originale Probe scheitert beim Lernen eines Mountnamens.
- Im erhaltenen neuen Zielprofil ist genau ein Direct-Grant `Accepted`, während `default_direct_exports.roots` leer und `include_connections` false ist. Die echten Accept-/Grantantworten melden `authorization.active=true` und `worker_refresh=refreshed`.

Das entspricht den aktuellen Quellen: `ShareProfiles::default` verwendet den leeren `ShareExportConfig::default`; `load_checked_with` behält diesen First-Run-Default. Das vorhandene `persisted_empty_export_list_is_not_replaced_with_home` bewahrt auch nach Save/Reload eine ausdrücklich leere Liste. `FsAccess::Authorized::list_dir` prüft die aktuelle Authority vor und nach der Abfrage; die virtuelle Root-Liste projiziert anschließend die konfigurierten Exporte. Hier war somit die Fixtureannahme eines impliziten Home-Mounts veraltet. Es ist keine Grantablehnung oder fehlerhafte Annahmeübernahme belegt.

## Umsetzung und Entscheidungen

Ausschließlich `native/test-share-mixed-version-e2e.sh` wurde geändert. Nach dem vorhandenen Lernen der aktuellen Direct-Identität und vor Serverkonfiguration, Legacyanfrage und Inbox-Neustart erstellt der bestehende Ablauf den gewöhnlichen Ordner `$root/old-to-new-export`. Der reale CLI-Befehl

```sh
run_se "$se_bin" "$new_target" share export add "$root/old-to-new-export" \
  --label "Files" >"$root/old-to-new-export-add.txt"
```

persistiert ihn als ausdrücklich ausgewählten Direct-Export. Die CLI-Registrierung `Command::Export` ist frisch geprüft; `exports` ist zusätzlich ihr bestehender sichtbarer Alias. `add_export` verwendet `SharedRoot::new`: ReadOnly, `allow_system_writes=false`, ohne Aktivierung gespeicherter Verbindungen. Der Datenordner liegt außerhalb der isolierten Identitäts-, App-Daten- und Konfigurationsverzeichnisse.

Die vorhandene Inbox-Neustartprobe muss diese vorab persistierte Exportkonfiguration mitnehmen. Die entfernte Adresse bleibt die von der echten alten CLI ausgegebene Direct-Adresse; den Mountbestandteil lernt `prove_remote_filesystem` weiterhin aus der tatsächlichen entfernten Root-Liste. Der lokale Datenpfad wird nicht als entfernte Adresse eingesetzt.

Die Produktquellen, modernen und alten Zulassungsgates, Fingerprint-/Key-/Node-Pins, Unsigned-Antwortverbot, Transport-Opt-ins, Write-/Exec-Rechte und Widerrufsregeln werden durch diesen Block nicht verändert. Alle ursprünglichen Assertions und Aufrufe einschließlich NEW→OLD und des Ablehnungspfads bleiben erhalten.

## Konkrete Abnahmesignale derselben Remote-Suite

Root verwendet den vorhandenen RV1-Eintritt und darin unverändert `native/test-share-mixed-version-e2e.sh`; es gibt keinen neuen Testeintritt oder separaten Lauf.

- `OLD to NEW: learn the current invite and connect both workers`: Der echte Export-Add-Befehl gelingt; `old-to-new-export-add.txt` hält seine Antwort fest. Die vorhandenen expliziten Transport-Opt-ins bleiben wirksam.
- `OLD to NEW: send the legacy request and prove its durable bare inbox`: Alle ursprünglichen Pending-, vollständigen Identitäts-/Fingerprint-, Inaktivitäts- und Unsupported-Receipt-Assertions sowie der echte Neustart bei offline gehaltenem Altrequester gelingen.
- `OLD to NEW: accept context-free, then prove history and grant persistence` und `OLD to NEW: retry the untracked decision context-free`: Die tatsächliche Annahme, persistierte aktive Autorisierung, Workerübernahme, untracked Delivery und erhöhte Retry-Attemptzahl erfüllen die unveränderten Assertions.
- `OLD to NEW: prove authorization and filesystem access`: Das tatsächliche alte `se ls` liefert den ausdrücklich angelegten Mount; `old-to-new-root-listing.txt` enthält die echte Root-Antwort. Der daraus gelernte entfernte Pfad wird über das tatsächliche alte `se stat` geprüft; `old-to-new-remote-stat.txt` enthält weiterhin das zwingende `type\tdir`-Signal.
- `OLD to NEW: active history cannot be deleted; revoke and delete context-free`: Die aktive History bleibt zunächst undeletable. Der echte Widerruf setzt Autorisierung inaktiv, der anschließende tatsächliche Altpeer-Leseaufruf wird verweigert und die danach mögliche Kontextfrei-Löschung erfüllt die bisherigen Assertions.
- Sämtliche vorhandenen NEW→OLD- und `OLD to NEW reject`-Assertions bleiben Teil genau desselben Ablaufs. Ein Logmarker ersetzt keine FS-, Lifecycle- oder Widerrufsassertion.

## Statischer Self-Review

`bash -n native/test-share-mixed-version-e2e.sh` hat ausschließlich die Shellsyntax geparst und Exit 0 geliefert. Eine vollständige Textgegenprüfung gegen die vor der Änderung gelesene Datei bestätigt: Wird genau der einmalige Export-Setupblock entfernt, ist der gesamte ursprüngliche Skriptinhalt bytegleich. Damit sind alle bisherigen Assertions, Admission-Pins, Rechte, Remote-Proben, Cleanup- und Retryabläufe unverändert.

Die Zusatzdefinitionen wurden jeweils erst nach Roots genauer Scopefreigabe gelesen. Es wurden keine lokalen Programme, Tests, Compiler, Formatter, Server, Git-, CI-, Graph- oder Releaseaktionen ausgeführt und keine Agenten gestartet. Das statische Ergebnis belegt keine neue erfolgreiche Remoteausführung.

## Dateiinventar

Gelesen, bei begrenzten Definitionen nur die freigegebenen Ausschnitte beziehungsweise gezielten Treffer:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/ci-5-s-revoke.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/ci-fifth-fixes.md`
- `docs/refs/rv1-remote-suite.md`
- `docs/refs/share-server-tls-auth.md`
- `/tmp/rv1-ci-fifth/linux/mixed-version.log`
- `native/src/share/core/legacy_direct_request_decision.rs`
- `native/src/share/core/legacy_direct_request_mutations.rs`
- `native/src/share/core/legacy_direct_request_reconciliation.rs`
- `native/src/share/core/legacy_direct_request_validation.rs`
- `native/src/share/core/legacy_direct_request.rs`
- `native/test-share-mixed-version-e2e.sh`
- `native/src/share/core/fs_authority.rs`
- `native/src/share/core/relation_rights.rs`
- `native/src/daemon/os/shared/ipc_host_profile_merge.rs`
- `native/src/daemon/os/shared/ipc_host_legacy_events.rs`
- `native/src/share/core/direct_relation.rs`
- `native/src/share/core/signal_commands_local.rs`
- `native/src/share/core/types.rs`
- `native/src/cli/share/requests_legacy.rs`
- `native/src/cli/share/grants.rs`
- `native/src/cli/share/exports.rs`
- `native/src/share/core/export_config.rs`
- `native/src/share/core/profiles.rs`
- `native/src/share/core/profile_persistence.rs`
- `native/src/share/core/profile_persistence_tests.rs`
- `native/src/share/core/fs.rs`
- `native/src/share/core/fs_access.rs`
- `native/src/cli/share.rs`

Geändert:

- `native/test-share-mixed-version-e2e.sh`

Erstellt und zur abschließenden Textprüfung gelesen:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/CI-5-S-REVOKE.md`

## Restgrenzen und Root-Handoff

Keine offene Source-/API- oder Scopeabhängigkeit. Die neue Fixturekonfiguration muss noch in Roots derselben vollständigen Remote-RV1-Suite den gesamten ursprünglichen Mixed-Version-Ablauf erfolgreich abschließen. Root besitzt Integration, Commit/Push und die eine Suite; das Ergebnis dieses Blocks ist keine Laufzeit- oder Releasefreigabe.

