# M04 / F02–F08: Appstartjournal und CLI-Versionen

Stand: 2026-10-26. Fixautor: delegierter Codex-Agent `fix_app_launch`.
Arbeitskopie: `/tmp/smartexplorer-review-fixes-ee75e91b`, Branch
`codex/review-fixes-ee75e91b`, gelesener HEAD `92cf56a4`.
Beobachtungskandidat: `1ffbf4b0bc90b2479947680abcd0a5941cce00f8`,
abgeschlossener Remote-Lauf `40406246032`.

## A. Auftrag, Grenze und Untersuchungsfragen

Der delegierte Ergebnisauftrag fordert ein privates, dauerhaft idempotentes
Appstartjournal und einen kollisionsfreien Versionsvertrag für Iconabruf und
gesamte Vervollständigung. Der Nutzervertrag vom 2026-10-26 verlangt vom selben
Fixautor Fehlerfunktionsliste, direkte/indirekte Verknüpfungskarte, konkreten
Erhaltungsplan, Fix und anschließende Anti-Regressionsreview. Wörtliche Quelle:
`/tmp/smartexplorer-final-review-user-contract.txt`.

Die exakte Dateigrenze steht in `/tmp/smartexplorer-f-app_launch-scope.json`.
Main besitzt Manifest, gemeinsame Remote-Suite, Graphaktualisierung, Integration,
Commit, Push und Release. Dieser Autor startet keine Compiler, Formatter,
Builds, Tests, Server, Paketinstallationen, Unteragenten oder Releaseprozesse.
Lokale Rückmeldung besteht ausschließlich aus Quelllektüre und statischem
Text-/Syntaxparsen. Die Abgabe ist kein Laufzeitpass.

Fragen: Wo entstehen F02–F08 tatsächlich? Welche produktiven Verträge dürfen
wegen einer ungeeigneten Fixture nicht abgeschwächt werden? Wie bleiben
Icon-Datenversion und Programmversion gleichzeitig erreichbar? Grundlage sind
die kandidatengebundenen Fehlermeldungen, aktuelle Quellen, gepinnte
Herstellerquellen und der veröffentlichte Quellstand. Die Auswertung trennt
belegten Produktfehler, Fixturefehler und unbestätigte Laufzeitfrage.

## B. Belegter Bestand und Fehlerfunktionen

| Befund | Beobachtete Funktion | Ursache und Einordnung |
| --- | --- | --- |
| F02 | `batches_app_ambiguous_dispatch_remains_unknown_until_matching_completion` | `host()` scheitert bereits in `AppHost::new` an `PermissionDenied`; der unbekannte OS-Ausgang wird noch nicht erreicht. Fixturefehler. |
| F03 | `batches_app_catalog_logon_revocation_and_expiry_prevent_dispatch` | Derselbe Journalaufbau scheitert vor Katalog-, Benutzer-, Sitzungs-, Ablauf- oder Widerrufsauswertung. Fixturefehler. |
| F04 | `batches_app_consent_continuation_uses_new_phase_and_never_repeats_remote_dispatch` | Derselbe Journalaufbau scheitert vor dem Consent-Phasenwechsel. Fixturefehler. |
| F05 | `batches_app_durable_claim_precedes_dispatch_and_survives_host_reconstruction` | Derselbe Journalaufbau scheitert vor dauerhafter Reservierung und Rekonstruktion. Fixturefehler. |
| F06 | `batches_app_peer_owner_and_complete_body_survive_reconstruction_without_rebinding` | Derselbe Journalaufbau scheitert vor atomarer Peer-/Bodybindung. Fixturefehler. |
| F07 | `batches_app_repeated_request_rechecks_permission_before_disclosing_receipt` | Derselbe Journalaufbau scheitert vor erneutem Berechtigungsnachweis und Receipt-Offenlegung. Fixturefehler. |
| F08 | `cli::completions_access::tests::perm_task_completions_access` sowie `se apps icon` | Produktfehler im CLI-Baum: `AppsCommand::Icon.version` erzeugt ID/Longoption `version`; `Cli` propagiert zugleich die automatisch erzeugte Programmversionsoption. `clap_builder` verweigert doppelte Argument-IDs und Longoptionen beim rekursiven Aufbau. Der Completion-Fall erreicht deshalb seine Zugriffsoptionen nicht. |

Die vollständigen Remote-Diagnosen für F02–F07 nennen
`batches_launch_task_tests.rs:121` und den privaten Journalvertrag. F08 nennt auf
Linux und Windows die doppelte ID `version` unter Kommando `icon`. Die Quelle
ist `/tmp/smartexplorer-rv1-40406246032/native-failures.json`; die Zuordnung
steht in `rv1-funktionsbefunde.json`.

Die Fixture verwendet `tempfile::tempdir()` direkt als Ledger. `tempfile 3.27.0`
erzeugt Unix-Verzeichnisse ohne explizite Rechte mit `0777 & !umask`; eine
übliche Umask `0022` ergibt `0755`. Die beobachteten Rechte des Remoteobjekts
sind nicht separat archiviert; der genaue Wert ist unbekannt. Belegt ist die
Ablehnung durch die Eigentums-/Modus-/Verzeichnistypprüfung. Die eigene
Quellfolgerung ist: Die Fixture muss die Privatsphäre explizit herstellen,
anstatt die Prüfung eines bestehenden Verzeichnisses zu lockern.

Die Releasequelle `a814f263096fe69d3746e840b5f3fad3fd7e88a0` enthält
`native/src/cli/apps.rs` noch nicht (`git show <release>:<path>` meldet den
fehlenden Pfad). Es wird daher kein veröffentlichter Icon-Schalter umgedeutet.

## Direkte und indirekte Verknüpfungskarte

1. Fixture `FakePlatform::new` → `host()` → `AppHost::new` → `Journal::new`
   → der in `apps/os/mod.rs` ausgewählte Plattformadapter
   `prepare_journal`. Linux prüft echtes Verzeichnis, keinen Symlink, effektive
   UID und keine Gruppen-/Fremdrechte. Android prüft den privaten App-UID-Pfad;
   Windows prüft Eigentümer/Reparsepunkt und setzt die private DACL.
2. `AppHost::launch` / `resume_after_local_consent` → `launch_inner` →
   `Journal::bind` / `read` → frischer Katalog und exakte Benutzer-/Sitzungs- und
   Revisionsbindung → Autorisierungsclosure mit gehaltenem Guard → Ablauf- und
   Widerrufskontrolle → `claim` beziehungsweise `claim_continuation` →
   `dispatch_claimed` → OS-Dispatch. `FakePlatform::dispatch` liest den schon
   dauerhaften `OutcomeUnknown`-Receipt und prüft Guard sowie Dispatch-ID.
3. Journal `staged` / `bind` / Consentclaim → synchronisierte private temporäre
   Datei → plattformspezifisches atomisches `publish_record` ohne Ersetzen beim
   Claim → `sync_journal`. `update` ersetzt nur die bestehende Quittung.
   Die SHA-256-Dateinamen, 32-KiB-Grenze, genaue ID-/Bodybindung und fehlende
   Eviction erhalten Wiederholungsschutz nach Hostrekonstruktion.
4. `bind_peer_request` → atomare `peer-request-v1`-Owner-/Bodybindung →
   kompatible `bind_peer_owner`- und Journalbindung. `receipt`,
   `note_dispatch_result`, `note_window` und `window_pending` lesen oder ändern
   nur den gebundenen Vorgang; veraltete Dispatch-ID aktiviert kein Fenster und
   Timeout löst keinen zweiten Start aus. Die Korrelation läuft über
   `shared/window_correlation` und die Plattformadapter.
5. `Cli` mit `propagate_version = true` → `Command::Apps` → `AppsArgs` →
   `AppsCommand::Icon` → `apps::run` → `IconRef.version` →
   `daemon::apps_icon` → typisierte Apps-IPC-/Sharegrenze →
   `AppHost::icon` → aktueller Katalog, exakte Iconreferenz und
   `IconReply::bytes_for`. Version, App-ID, SHA-256 und Maße bleiben ein
   gemeinsam validiertes Objekt. Der Fix ändert keine Wire-/IPC-Feldnamen.
6. `Cli::command` → `CompleteEnv` und `clap_complete::engine::complete` →
   rekursiver CLI-Baum einschließlich `apps icon` → Zugriffskandidaten aus
   `completions_access`. F08 scheitert beim Baumaufbau; die Logik für Geräte,
   Räume, Konten, offene Requests und Grants selbst ist kein belegter Fehler.
7. `cli/screen.rs::device_target` → `daemon::apps_icon`-Reexport in
   `daemon/mod.rs` → `apps_ipc_client::apps_icon` mit unveränderter
   `IconRef::validate`-Prüfung → `AppsIpcRequest::Command` →
   `ShareHost::serve_apps` in `apps_ipc.rs` →
   `ShareService::apps_command` in `node_apps_link.rs`. Dieser unveränderte
   Transport bindet NodeId/Peer, Relation, Antwort-ID und Requesthash und prüft
   Iconbytes gegen die genaue angeforderte Referenz.
8. Eingehend: `node_apps_client::handle_apps_connection` / `serve_apps` →
   `prove_relation` und gebundener Principal → `node_apps_host::apps_execute`.
   Iconabruf prüft `visible_catalog` vor und nach dem Renderer sowie genaue
   App-/Icon-/Katalogrevision. Launch und Receipt prüfen den dauerhaften
   Peer-/Requestbesitzer. `run_app` erzeugt die frischen E4-Facts,
   Autorisierungs-/Widerrufs-/Ablauf-/Verbindungsprüfung und den gehaltenen
   `LaunchAdmission`-Guard vor `AppHost::launch` beziehungsweise
   `resume_after_local_consent`.
9. Einrichtung: `ShareService::configure_apps_host` prüft die Geräteidentität
   und übergibt den gelieferten Root unverändert an
   `AppHost::for_current_user`. `node_apps_platform::install_platform` bindet
   Android-Resultat-/Consentcallbacks an den ursprünglichen Node.
   `platform_result` verarbeitet nur den exakten gespeicherten Request und
   bestehenden Peerbesitzer; `platform_consent` prüft den ursprünglichen
   AccessHandle und aktuelle Policy vor `run_app(..., resume=true, ...)`.

Der Rootgraph wurde ausschließlich innerhalb der freigegebenen Dateien und
Symbole gelesen. Seine extrahierten Kanten bestätigen die Hostlaunch-/Modell-
und Iconreferenz-Verbindungen; die aktuelle Quelllektüre ergänzt die
Aufruf-/Makrogrenzen. Der Graph wird von diesem Autor nicht geändert.

## C. Fixentscheidung, Recherche und Erhaltungsvertrag vor Änderung

Recherchepass 1 verwendet den abgeschlossenen `rv1-funktionsplan.md`, die
kandidatengebundenen Diagnosen und die aktuelle direkte Aufruferkette.
Recherchepass 2 prüft die konkreten neuen Aufrufe und die Releasekompatibilität:

- `tempfile 3.27.0`, [`Builder::permissions`](https://docs.rs/tempfile/3.27.0/tempfile/struct.Builder.html#method.permissions),
  abgerufen am 2026-10-26: Unix-Bits werden durch die Umask ausschließlich
  eingeschränkt. Ein Builder mit expliziten `0700` erzeugt die Fixture privat;
  es ist keine globale Umaskänderung und kein nachträglicher öffentlicher
  Erzeugungs-/chmod-Zwischenschritt erforderlich.
- Gepinnte lokale Herstellerquelle `clap_builder 4.6.0`,
  `builder/command.rs:1493/1527` und `builder/debug_asserts.rs:99/110`:
  `propagate_version(true)` gilt für Unterkommandos; doppelte ID und doppelte
  Longoption sind jeweils ungültig. Eine reine interne ID-Umbenennung ließe
  die Longoptionkollision bestehen.
- `native/Cargo.lock` bestätigt `clap 4.6.1`, `clap_builder 4.6.0`,
  `clap_complete 4.6.7` und `tempfile 3.27.0`; keine neue Abhängigkeit.
- `disable_version_flag(true)` nur unter `icon` würde den Datenversionsnamen
  bewahren, aber dort die etablierte Programmversionskonvention entfernen.
  Stattdessen wird die neue Icon-Datenoption eindeutig `--icon-version`;
  `-V` und `--version` behalten ihre Programmversionsbedeutung im ganzen Baum.
- Main hat am 2026-10-26 die aktuellen Quellen, Dokumente und Aufrufskripte in
  `native/src`, `docs`, `native/*.py`, `native/*.sh`, `README.md` und `android`
  durchsucht und keinen konkreten Aufrufer von
  `se apps icon ... --version <n>` gefunden. Das ist der ausdrücklich als
  Main-Beleg übergebene Verbrauchersuchbefund; die zugehörigen nicht
  freigegebenen Dateien wurden von diesem Autor nicht gelesen.
- Eine Lockerung der Linux-Prüfung oder automatisches chmod eines bestehenden
  ungeeigneten Journals wird verworfen: Der Testaufbau darf keine fremde oder
  öffentliche Persistenz in einen zugelassenen Startspeicher umdeuten.

Der bestehende Startvertrag bleibt unverkleinert: genaue Geräte-, App-, Benutzer-,
Login-, Katalog- und Requestbindung; dauerhafter Claim vor OS-Wirkung; atomarer
No-replace-Claim; erneute Berechtigungsprüfung; Guard bis Dispatchende;
keine Eviction; fremde Peer-/Bodyänderung abgelehnt; unbekannter Ausgang
bleibt unbekannt; Consent nutzt eigene Phase ohne Remote-Wiederholungsstart;
kein Start oder Fensterrecht aus einer Iconreferenz. CLI-Standarddatenversion
bleibt `1`, Datentyp bleibt `u8`, Geräteauflösung und JSON-Ausgabe bleiben
unverändert. Weder Suite noch Erhaltungsorakel werden verkleinert.

## D. Konkrete Meilensteine und Abnahmesignale

| Schritt | Dateien | Erwartetes Ergebnis und Abhängigkeit | Stand |
| --- | --- | --- | --- |
| M04.1 Private Fixture | `apps/os/batches_launch_task_tests.rs` | Alle F02–F07 erreichen mit explizit privatem Unix-Ledger unveränderte Verhaltensassertionen. Windows-Aufbau bleibt über den produktiven ACL-Adapter. Ergänzter Linux-Fall verweigert öffentliches und verlinktes Journal ohne Lockerung. | Quelländerung und eigene statische Review abgeschlossen; Laufzeit unbestätigt |
| M04.2 Versionsvertrag | `cli/apps.rs` | Icon-Datenversion ist `--icon-version`; `-V`/`--version` geben auf Root-, Apps- und Iconebene die Programmversion aus. IconRef/Wire/IPC unverändert. | Quelländerung und eigene statische Review abgeschlossen; Laufzeit unbestätigt |
| M04.3 Autorprüfung und Suiteübergabe | `cli/apps_task_tests.rs`, dieser Bericht | Gesamter CLI-Baum, Standard-/explizite Icon-Datenversion und wirkliche Completionkandidaten sind als Fälle desselben Remote-Einstiegs formuliert. Statischer Diff-/Syntaxabgleich bestätigt den begrenzten Änderungsweg; Laufzeit bleibt Main. | Quellfälle und statische Abgabe abgeschlossen; gemeinsame Remote-Abnahme bei Main |

Die Laufzeitabnahme ist ausschließlich die bereits bestehende gemeinsame
Remote-Suite nach Integration, Commit und Push. Die genaue ergänzende
Fallregistrierung und die abgeschlossene Autorenreview stehen unten.

## Umsetzung und eigene anschließende Anti-Regressionsreview

Der Fixautor hat nach der Änderung den eigenen begrenzten Diff und die oben
dokumentierte Verbraucherkette erneut gegen den Erhaltungsvertrag geprüft:

| Vertrag | Konkreter statischer Beleg und Ergebnis |
| --- | --- |
| Private Fixture ohne aufgeweichte Journalprüfung | `private_ledger` setzt über `Builder::permissions` bereits bei Unix-Erzeugung `0700`. Die Ausnahme ist nur der gezielt erzeugte öffentliche/verlinkte Pfad im negativen Linux-Fall. `linux_os/mod.rs` und `shared/journal.rs` sind bytegleich zum Beobachtungskandidaten. |
| Alle bisherigen Appstartorakel bleiben erhalten | Der statische AST-/Funktionskörpervergleich gegen `1ffbf4b0` bestätigt für alle sechs F02–F07-Körper identischen Inhalt nach ausschließlich der Ersetzung `tempfile::tempdir().unwrap()` → `private_ledger()`. Keine Assertion, Fehlerbedingung, Permissionclosure, Rekonstruktion oder Dispatchzählung ist entfernt oder abgeschwächt. |
| Idempotenz, Peerbesitz und Consent bleiben produktiv identisch | `host.rs`, `host_launch.rs` und `host_lifecycle.rs` sind bytegleich zum Beobachtungskandidaten. Durable Claim, Body-/Peerbindung, Unknown-Status, Guardlebensdauer, Consentclaims und ID-Abgleich bleiben unverändert. |
| Eindeutige Icon-/Programmversion | `AppsCommand::Icon.icon_version` erzeugt ausschließlich `--icon-version`; der unveränderte Root-CLI propagiert weiter die Programmversion. `apps::run` übernimmt `icon_version` ausdrücklich in das unveränderte Wirefeld `IconRef.version`. Die kompakte und formatierte JSON-Ausgabe sowie die sieben Apps-Unterbefehle bleiben erhalten. |
| Bestehende Icon- und Transportrechte | `apps_ipc_client.rs` und `screen/core/app_icon.rs` sind bytegleich zum Beobachtungskandidaten. Die zulässige Icon-Protokollversion bleibt `1`; der zusätzliche Parserfall mit explizitem Wert `2` belegt nur die unterscheidbare Argumentdurchreichung. Die bestehende `IconRef::validate`-Prüfung lehnt diesen Protokollwert weiterhin vor IPC ab. |
| Vollständige Completion bleibt erreichbar | Der neue Fall ruft `Cli::command().debug_assert()` auf und prüft tatsächliche Completionkandidaten für alle sieben Apps-Unterbefehle und neun Iconoptionen. `clap_builder 4.6.0` dokumentiert und implementiert `debug_assert` → `build` → rekursiven Aufbau sämtlicher Unterkommandos. Der bestehende F08-Fall und `completions_access.rs` bleiben unverändert. |
| Plattform- und Modulgrenze | Unix-Rechteimport liegt ausschließlich im Fixturehelper unter `cfg(unix)`; der negative Journalfall ist Linux-spezifisch. Kein neuer OS-Import gelangt in `core/`; keine Abhängigkeit und kein Persistenzformat ändern sich. Windows verwendet weiter die bestehende DACL-Vorbereitung. |

Zusätzlich wurden die drei geänderten Rust-Dateien mit dem vorhandenen
Tree-sitter-Rust-Parser rein statisch geparst; es gab keine Syntaxfehler oder
fehlenden Syntaxknoten. `git diff --check` des begrenzten Diffs meldete keine
Whitespacefehler. Die Rust-Dateien bleiben unter 500 Zeilen und 50 KiB.
Dieser Parserbeleg prüft keine Rust-Makroexpansion, Typprüfung, Clap-Ausführung,
OS-Rechtewirkung oder Startwirkung. Compiler, Formatter und Testprogramme
wurden nicht gestartet. Es existiert kein eigener Hintergrundprozess.

Die Read-Erweiterungen für `completions_access_tests.rs`, CLI-Geräteauflösung,
Apps-IPC und die vier Host-/Client-/Callbackdateien wurden jeweils von Main
ausdrücklich in den Scope aufgenommen. Es gibt keine offene notwendige
Schreibabhängigkeit und keine festgestellte zusätzliche Funktionsregression
im begrenzten statischen Änderungsweg. Die unveränderten E4-, Wire- und
P4-Implementierungen außerhalb der freigegebenen Dateien werden durch diese
Abgabe nicht als vollständig auditiert ausgegeben.

## Präzise Übergabe an denselben Remote-Einstieg

Die bisherigen F02–F08 bleiben in ihrer vorhandenen Auswahl. Folgende neue
Quellfälle gehören zur gleichen Suite:

- Linux:
  `apps::os::host::batches_launch_task_tests::batches_app_journal_rejects_public_and_symlinked_directories`.
- Linux und Windows:
  `cli::apps::task_tests::batches_app_cli_icon_data_version_retains_default_and_explicit_values`.
- Linux und Windows:
  `cli::apps::task_tests::batches_app_cli_program_version_remains_available_at_every_apps_level`.
- Linux und Windows:
  `cli::apps::task_tests::batches_app_cli_complete_tree_and_icon_options_remain_valid`.

Alle neuen Fallnamen tragen das bereits vorhandene Manifestpräfix
`batches_app_`. Die gelesenen Manifestpfade `/native/required_prefixes` und
`/compatibility/K4` wählen dieses Präfix bereits; F08 bleibt zusätzlich in
`/native/integrations` und in seinem bestehenden Ressourcenmeilenstein.
Main bestätigt die endgültige gemeinsame Manifest-/Meilensteinbindung.
Es entsteht kein zusätzlicher Einstieg oder gesonderter Prüflauf.

Unbestätigte Laufzeitfragen sind vollständig benannt: Der Remote-Linux-Lauf
muss die tatsächliche private Fixture sowie die öffentliche und verlinkte
Ablehnung zeigen und die unveränderten F02–F07 erstmals bis zu ihren
Startorakeln ausführen. Linux und Windows müssen die Clap-Makro-/Parserwirkung,
Programmversionsausgabe, Default-/explizite Icon-Datenversion und wirklichen
Completionkandidaten ausführen. Die Fixture simuliert OS-Katalog und Dispatch;
sie belegt keinen echten Windows-, Linux- oder Android-Appstart und keine
physische Fensteraktivierung. Für diese M04-Behebung ist kein neues
Providerkonto oder Zugangsschlüssel erforderlich.

## Vollständiges Dateiinventar

Die Dateiliste erfasst Volllektüre, gezielte Vertragsabschnitte, Signatur- oder
Versionsabfragen und die eigenen abschließend gelesenen Änderungen. Sie
behauptet keinen Vollreview sämtlicher Zeilen jeder aufgeführten Datei.
Repositorypfade sind relativ zur oben genannten Arbeitskopie.

Gelesene Repositorydateien:

```text
AGENTS.md
docs/ARCHITEKTUR.md
docs/plaene/2026-10-24-batches/fix-f-app-launch.md
docs/plaene/2026-10-24-batches/rv1-funktionsbefunde.json
docs/plaene/2026-10-24-batches/rv1-funktionsplan.md
graphify-out/graph.json
native/Cargo.lock
native/Cargo.toml
native/review-task-manifest.json
native/src/apps/core/grant.rs
native/src/apps/core/model.rs
native/src/apps/mod.rs
native/src/apps/os/android/bridge.rs
native/src/apps/os/android/bridge_admission.rs
native/src/apps/os/android/launch.rs
native/src/apps/os/android/mod.rs
native/src/apps/os/batches_launch_task_tests.rs
native/src/apps/os/host.rs
native/src/apps/os/host_catalog.rs
native/src/apps/os/host_launch.rs
native/src/apps/os/host_lifecycle.rs
native/src/apps/os/linux_os/launch.rs
native/src/apps/os/linux_os/mod.rs
native/src/apps/os/linux_os/window_correlation.rs
native/src/apps/os/mod.rs
native/src/apps/os/shared/journal.rs
native/src/apps/os/shared/mod.rs
native/src/apps/os/shared/platform.rs
native/src/apps/os/shared/window_correlation.rs
native/src/apps/os/windows/launch.rs
native/src/apps/os/windows/storage.rs
native/src/apps/os/windows/window_correlation.rs
native/src/cli/apps.rs
native/src/cli/apps_task_tests.rs
native/src/cli/completions.rs
native/src/cli/completions_access.rs
native/src/cli/completions_access_tests.rs
native/src/cli/mod.rs
native/src/cli/screen.rs
native/src/daemon/mod.rs
native/src/daemon/os/shared/apps_ipc.rs
native/src/daemon/os/shared/apps_ipc_client.rs
native/src/daemon/os/shared/ipc_protocol_apps.rs
native/src/screen/core/app_icon.rs
native/src/share/core/node_apps.rs
native/src/share/core/node_apps_client.rs
native/src/share/core/node_apps_host.rs
native/src/share/core/node_apps_link.rs
native/src/share/core/node_apps_platform.rs
```

Gelesene externe lokale Anweisungs-, Befund- und Herstellerdateien:

```text
/root/.agents/skills/arbeitsweise/SKILL.md
/root/.codex/AGENTS.md
/root/.codex/skills/arbeitsweise/SKILL.md
/root/.codex/skills/graphify/SKILL.md
/root/.codex/skills/graphify/references/query.md
/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.0/src/builder/command.rs
/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/clap_builder-4.6.0/src/builder/debug_asserts.rs
/tmp/smartexplorer-f-app_launch-scope.json
/tmp/smartexplorer-final-review-user-contract.txt
/tmp/smartexplorer-rv1-40406246032/native-failures.json
```

Per MCP gelesene Arbeitsweise: zentraler delegierter Lauf Revision `337` samt
Referenztexten zu Code-/Bestands-/Herstellerrecherche, Planung, Bedienbarkeit,
Analyse, Review, Änderung und Abschluss. Extern abgerufene Webseite:
die oben versionsgebunden verlinkte `tempfile 3.27.0`-Builder-Dokumentation.
Git-Quelllesung verwendete ausschließlich freigegebene Pfade an
`1ffbf4b0` sowie die belegte Abwesenheit von `cli/apps.rs` an `a814f263`.

Erstellte Dateien:

```text
docs/plaene/2026-10-24-batches/fix-f-app-launch.md
native/src/cli/apps_task_tests.rs
```

Geänderte bestehende Dateien:

```text
native/src/apps/os/batches_launch_task_tests.rs
native/src/cli/apps.rs
```

Die vollständige Autorenabgabe endet hier. Gemeinsame Remote-Abnahme,
Graphaktualisierung, Integration, Commit, Push und terminaler Release bleiben
bei Main; diese Abgabe erklärt keinen Laufzeit- oder Releaseerfolg.
