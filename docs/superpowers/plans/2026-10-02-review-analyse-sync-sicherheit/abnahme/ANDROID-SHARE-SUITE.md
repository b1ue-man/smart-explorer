# ANDROID-SHARE-SUITE – reale JNI-Fixtures für RV1

Stand: 2026-10-03. Quellseitig umgesetzt nach ausdrücklichem Root-Go, nicht ausgeführt.
Scope: `scopes/android-share-suite.json`. Die eine finale RV1-Pipeline, APK-/Native-Kompilation,
Gerät, TLS-Host, gezielte Hostannahme und Ergebnisbewertung gehören dem Hauptagenten.
Dieser Block erstellt nur die Testklasse und diesen Bericht; keine Produkt-/UI-/Helper-/CI-Datei geändert.

## Vorabplan und konkret implementierte Fälle

Die API-/Fixtureplanung wurde vor Änderungen an Root übergeben. Grundlage sind die vorhandenen
`coreTest`-/`Api`-/`Fixture`-Helfer, typed Share-Consumer, tatsächliche native Facaden und ihre
Persistenz-/Statusproduzenten. Die anfänglichen Read-Lücken wurden vor dem Root-Go geschlossen.
Keine neue Reviewrunde, kein separater Prüfer, keine weitere Feature-Orchestrierung.

| Tatsächliches JUnit-Symbol in `app.smartexplorer.android.task.ReviewShareTaskTest` | Reale JNI-Grenze und konkrete Assertions |
|---|---|
| `persistedRootRoomAndAccountRightsRejectStaleCas` | Unicode-Testordner über `share.addExport`, persistiertes RO und Systemwrite=false; neuer Raum ohne Exports/Schreiben; ausdrückliches Raumwrite und separate Bestätigung ohne Rücksetzen des Schreibflags; Direct-RW→RO und veraltetes `expectedAccess` abgelehnt, Profilbytes unverändert. Eigenes gespeichertes SFTP-Konto ohne Netzwerk/Secret; tatsächliche `conn.save.id` bleibt exakter Share-account. Native neue Auswahl ohne access ist RO, nur ausdrücklicher RW-Aufruf erhöht. Nach Entzug scheitert `expectedShared:true` wiederholt, Konto bleibt ungeteilt und Profilbytes unverändert. Probeinhalt bleibt SHA-identisch. |
| `contactAndRoomAdmissionRequireCurrentFullPins` | Tatsächlicher Desktop-Direct-Code, Geräte-ID und gezielte aktuelle Anfrage; lokaler Telefonname wird als `RV1-Android-Share` gespeichert und im Status geprüft. Kein anfängliches Share-back; erst ausdrückliches true schafft lokalen Grant, dessen native Pins und anfängliches write=false gegen Profil-JSON geprüft werden. Je falscher Geräte-ID, Schlüssel, Knoten und Fingerabdruck wird Schreiben abgelehnt, mit unveränderten Profilbytes. Korrektes Schreiben, Widerruf, verweigertes inaktives Schreiben und ausdrückliches Wiederzulassen werden persistiert. Echter CLI-Raum: eigene Bestätigung vor erster Präsenz aktiviert, reales Desktopmitglied Pending/blocked; jede falsche Mitgliedspin sowie Profil-/Raum-ID wird bei admit abgelehnt. Korrektes admit/block/allow ändert tatsächliche persistierte Mitgliedsrelation; Raumwrite bleibt false. Kein neues Exec. |
| `tlsWeakPinConsentAndStoreFailuresRemainRetryable` | Echter I/O-Fehler an der Profillock-Grenze: leeres Verzeichnis statt Lock, Rechteaufruf ergibt lesbaren `CoreException("invalid",…)`, Profilbytes unverändert; Lock sofort restauriert, derselbe Aufruf wird erfolgreich wiederholt und RW in JSON geprüft. Nackte Adresse wird gespeichert WSS/encrypted; Serverbytes entsprechen der kanonischen Antwort. TCP ohne Auswahl scheitert unverändert, TCP mit ausdrücklicher Auswahl wird plaintext. Neue gemischte TLS-/TCP-Eingabe scheitert unverändert. Native sechsstellige PIN; leere PIN mit/ohne Opt-in invalid, triviale PIN ohne Auswahl weak_pin, 31 Minuten invalid. Weak-Opt-in und starke native PIN werden tatsächlich publiziert, über offerId/alias/target/expiry im Status beobachtet und beendet. |

`consume` ruft die unveränderten echten `ShareApi`-/`SharePolicyApi`-/`ShareSecurityApi`-Consumer auf,
die über `Core` und `NativeBridge` JNI nutzen. Es trägt nur deren tatsächlichen Rückkehr-/Fehlerausgang
in `TaskReport.calls.tsv` ein. Setup-/Cleanup-/Rohantwort-Prüfungen nutzen `Api` aus `TaskSupport.kt`.
`persisted`/`changed` werden nur auf der echten Rechteantwort geprüft. Discovery liefert zunächst
`{}`; ihr Erfolg wird ausschließlich aus dem tatsächlich publizierten Angebot festgestellt.
Keine Mock-Facade, kein angenommenes Erfolgsfeld, kein manuell hergestellter Grant oder Raummitglied.

## Runnerinputs und positive Fixture-Zustimmung

| Tatsächliches Instrumentation-Argument | Produzent / Vorbedingung |
|---|---|
| `seShareServer` | Root entdeckt die neue echte verschlüsselte Serveradresse des isolierten TLS-Hosts einschließlich erforderlichem Zertifikatspin und tatsächlichem Port. `online()` übergibt sie mit `allowPlaintext=false` und verlangt security=encrypted, plaintext=false sowie tatsächlichen connected/running-Status. |
| `seDesktopDirectCode` | Direct-Code aus der frischen echten CLI-Identität desselben Hosts; nie konstruierter Lookup/Code oder Secret im Bericht. |
| `seDesktopDevice` | Geräte-ID aus der echten CLI-Identität; muss zur übergebenen Direct-Code-/Raumidentität passen. Lokaler Grant darf vor diesem Szenario nicht bereits vorhanden sein. |
| `seRoomCode` | Code eines frischen tatsächlichen Desktopraums. Sein Android-Profil darf keinem anderen Szenario gehören. Root setzt nur für diesen isolierten Raum die bewusste Hostzulassung beziehungsweise Bestätigungspolicy. |

Keine SFTP-Server-/Passwortargumente erforderlich: `conn.save` speichert ein eindeutiges
`<fixture>.invalid`-Konto mit literalem Unicode-Startordner, ohne Secret und ohne `conn.test`, Listing
oder Verbindung. Nur diese neue id wird geteilt beziehungsweise entfernt. Die Exports sind ausschließlich
unter dem eigenen einzigartigen `SmartExplorerTask/rv1-share-…`-Ordner des primären Testvolumes.
Neue Schreib-/Systemwrite-/Room-/Share-back-Rechte werden im jeweiligen Fixture ausdrücklich gesetzt;
Home, echte Nutzerkonten und Produktdefaults werden nicht als Fixtureabkürzung verändert.

Der Host-Owner wurde von Root für
`share-desktop.sh accept ROOT SECONDS INSTRUMENT_OUT` beauftragt. Exakter Phone-Name ist
**`RV1-Android-Share`**, mit `/root/v_local` abgestimmt. Die Contact-Methode sendet genau eine echte
aktuelle Anfrage, und der Root-Helper nimmt ausschließlich die eindeutige konfliktfreie passende
Anfrage per CLI an. Kein AutoAccept-JSON-/Produktpatch. Root besitzt Start, ausreichende Laufzeit,
Stop/Wait und Zuordnung des Helpers zum Contact-Test; bei separater Methodenselektion ist das oben
genannte Contact-Symbol der Startpunkt. Desktop-Schreib- und Exec-Opt-ins werden nicht benötigt.

Das Gerät benötigt die bestehenden RV1-Voraussetzungen: tatsächliche debug-/Test-APK mit Native-JNI,
laufender Core/Daemon und freigegebener primärer Teststorage. Fehlende Inputs/Storage/Pins oder TLS-
Verbindung schlagen mit eindeutiger Assertion/Frist fehl; kein Skip als Abnahmeersatz.

## Persistenz- und Bytebelege

- Echte Dateien sind `File(appContext.filesDir,"smart_explorer/share_profiles.json")`,
  `share_profiles.lock` und `share_server.txt`. Die Namen kommen aus dem Profile-/Server-Store.
  Die Root-Faktmitteilung belegt den vorhandenen Init-Produzenten (`init.rs:60–61`):
  `HostConfig.data_home=settings.files_dir`; InitConfig verwendet `app.filesDir`.
  `noBackupDir` ist ausdrücklich nicht diese Share-Datenbasis. `init.rs` wurde von diesem Block nicht gelesen.
- Die Klasse liest die serialisierten nativen Namen, etwa `default_direct_exports`, `access`,
  `allow_system_writes`, `shared_connections`, `members_may_write`, `confirm_new_members`,
  `device_id`, `public_key`, `node_id`, `fingerprint`, `relation.admission`, `state` und `write`.
  Ausgelassenes false-Systemwrite und ausgelassene leere shared_connections entsprechen den
  tatsächlichen serde-Regeln. Falscher vorhandener JSON-Typ wird nicht als leer/false behandelt.
- Vor Byteassertions wird der Share-Worker beendet und sein tatsächlicher running/connected=false-
  Status abgewartet. Dadurch prüft der Fall abgelehnte Mutationen, ohne parallele Presence-Saves
  als Berechtigungsänderung zu interpretieren. Profilbytes werden weder direkt editiert noch gelöscht.
- Frischer fehlender Profilstore hat den aus `ShareProfiles::default()` belegten auto_connect=true-
  Default. Das erste `quiet()` persistiert tatsächlich false; kein Getter wird als erfolgte
  Profilerstellung behauptet. Neue Roots und Rechte werden anschließend aus der Datei gelesen.
- Rohprofile, Direct-/Room-Codes und PINs werden nicht als Evidenzdateien ausgeschrieben.
  `TaskReport.notes.txt` erhält nur erzeugte IDs/Pfade, Inhalts-SHA und Szenariobezeichnungen.

## Cleanup und Fristen

Alle Methoden nutzen `coreTest(10*60_000L)` einschließlich der vorhandenen Core-Ready-Grenze.
TLS-Verbindung und tatsächliches Angebot jeweils maximal 90 s; echte Annahme und Raummitglied
jeweils 120 s; Worker-/Offer-Ende jeweils 30 s. Polling erfolgt durch den bestehenden begrenzten
`waitFor`-Helfer. Root muss die Helper-/Instrumentierungs-/Pipelinefrist passend dazu setzen.

Jedes Szenario besitzt `finally`-Cleanup mit `NonCancellable` und eigener Höchstfrist 90 s.
Es verfolgt ausschließlich eigene erzeugte Exportpfade, Raum-/Kontakt-/Konto-IDs und Offer-Aliasse.
Es beendet das eigene Angebot, stoppt den Worker, entfernt eigene Exports/Räume/Kontakte über die
nativen Methoden und eigene Saved-Konten über `conn.delete`, restauriert kanonischen Server und
auto_connect sowie einen bekannten nichtleeren vorherigen Telefonname und entfernt den eigenen
Testordner. Es löscht keine Withdraw-/Legacy-Historie. Bereits bestehende Exec-Einstellungen bilden
eine Baseline; neue Ziele müssen ausgeschaltet bleiben. Keine unbeteiligten Grants werden deaktiviert.

Die Lockstörung hat ein zusätzliches unmittelbares `finally`: vorhandenes normales Lockfile wird
unter einer einzigartigen privaten Backupbezeichnung umbenannt, der leere Blockadeordner entfernt
und das Original-Handleobjekt per Rename zurückgebracht. Das Profil selbst bleibt unberührt.
Cleanupfehler werden gesammelt und werfen eine Assertion; bei einem Primärfehler werden sie als
suppressed ergänzt. Fehlgeschlagene Aufräumaktionen werden nicht still als Erfolg behauptet.
Root besitzt die terminale Host-/Geräteteardown-Grenze, auch bei Instrumentierungsabbruch.

## Exakte Datei-Inventur

Gelesene Vorgaben: `AGENTS.md` als Sessiontext; zuvor geladene arbeitsweise/Architektur gelten
unverändert, kein Graphzugriff. Tatsächlich in dieser Fixturephase gelesene Repositorydateien:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/android-share-suite.json`
- `docs/refs/android-apis.md` (app-private Storage-/API-Belege)
- `docs/refs/android-platform.md` (Teststorage-Voraussetzungen)
- `docs/refs/android-ci.md` (Instrumentation-/Runnergrenze)
- `docs/refs/android-child-processes.md` (gezielte bestehende Plattformhinweise)
- `docs/superpowers/plans/2026-09-25-android-apk/api.md` (nur Share und conn)
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/AND-SHARE-UI.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/AND-SHARE-UI.md`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/TaskSupport.kt`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/SyncSupport.kt`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/ShareRoomTaskTest.kt`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/ShareCleanupTaskTest.kt`
- `android/app/src/main/java/app/smartexplorer/android/core/Core.kt`
- `android/app/src/main/java/app/smartexplorer/android/core/CoreException.kt`
- `android/app/src/main/java/app/smartexplorer/android/core/Dtos.kt`
- `android/app/src/main/java/app/smartexplorer/android/core/InitConfig.kt` (nur Initpfade)
- `android/app/src/main/java/app/smartexplorer/android/api/ShareApi.kt`
- `android/app/src/main/java/app/smartexplorer/android/api/SharePolicyApi.kt`
- `android/app/src/main/java/app/smartexplorer/android/api/ShareSecurityApi.kt`
- `native/src/mobile/os/shared/domains/share_policy.rs`
- `native/src/mobile/os/shared/domains/share_settings.rs`
- `native/src/mobile/os/shared/domains/share_status.rs`
- `native/src/mobile/os/shared/domains/share_peers.rs`
- `native/src/mobile/os/shared/domains/share_state.rs` (nur Pfade/getter)
- `native/src/mobile/os/shared/domains/connections.rs` (nur Save-/Delete-Inputs, -Outputs und Cleanup)
- `native/src/share/os/shared/profile_store.rs` (nur Dateinamen/Pfad, JSON-/CAS-/Fehlergrenze)
- `native/src/share/core/profiles.rs` (nur serialisiertes Profil/Defaults)
- `native/src/share/core/types.rs` (nur serialisierte Member-/Room-/Identitätsfelder und Re-Exports)
- `native/src/share/core/direct_relation.rs` (nur serialisierte Rechte/Defaults)
- `native/src/share/core/room_relation.rs` (Struct-/Default-Re-Exports)
- `native/src/share/core/room_relation_members.rs` (nur serialisierte Policy-/Memberflags/Defaults)
- `native/src/share/core/profile_policy.rs`
- `native/src/share/core/export_config.rs`
- `native/src/support_dirs.rs` (nur tatsächliche app_data_dir/file-Pfadauswahl)

Neu erstellt, selbst gelesen und statisch geprüft:

- `android/app/src/androidTest/java/app/smartexplorer/android/task/ReviewShareTaskTest.kt`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/ANDROID-SHARE-SUITE.md`

Keine bestehenden Dateien geändert. Keine Builds, Compiler, Tests, Formatter, Server, Installationen,
langen Prozesse, Commits, Pushes, CI-, Graph-, Releaseaktionen oder weiteren Agents gestartet.

## Eigener Self-Review und offene Abnahmegrenzen

Self-Review folgt den tatsächlichen Consumerargumenten bis zur nativen Mutation und unabhängigen
Datei-/Zustandsassertion. Berücksichtigt sind fehlender initialer Store, serde-omittierte Defaults,
kanonischer tatsächlich gespeicherter Rootpfad, keine neuen Konten nach stale Entzug, volle Pins,
richtige Profil-/Raum-ID, Discovery-Queue versus tatsächliches Angebot, vorhandene Exec-Baseline,
beendete Worker vor Bytevergleichen und Scopegebundene Wiederherstellung im Fehlerfall.

Statische Prüfung abgeschlossen: Kotlin-Kommentar-/String-/Interpolations-/Klammerbalance,
doppelte Imports, Leerzeichen, Größen-/Kohäsionsgrenze, tatsächliche Consumer-Symbole,
dokumentierte Testmethoden/Runnerinputs, Markdownblöcke und eigener Pfaddiff sind sauber.
Die Testklasse hat 353 Zeilen und 24.504 Bytes. Diese Signale sind weder Kotlin-Typprüfung
noch Geräteabnahme; native Facaden und ihre Antwortfelder wurden nur aus den Quellen abgeglichen.

Offen beim Root: Integration der drei Symbole in dieselbe RV1-Gerätestufe, produktpassende TLS-/
Peer-/Raumfixtures und gezielte aktuelle CLI-Annahme, Remote-Kompilation/Ausführung, Auswertung
und terminaler Teardown. Dieser enge JNI-Block behauptet keine erfolgte Compose-Bedienung von
Checkboxen/Draft-Retention/Mehrfachtaps, keine tatsächliche Gegenbestätigungsstörung oder historische
Home-Migration. Dafür gelten die weiteren RV1-Signale des [AND-SHARE-UI-Berichts](AND-SHARE-UI.md).
Kein lokaler Lauf und keine ungetestete Erfolgsaussage.
