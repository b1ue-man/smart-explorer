# SHARE-DEVICE-HELPER – isolierter TLS-Host und echte Telefonannahme

Stand: 2026-10-03. Der beauftragte reine Fixture-Anschluss ist source-seitig abgeschlossen. Es wurden ausschließlich der vorhandene Desktop-Helper und dieser Bericht geändert/erstellt. Keine Produkt-, Suite-, UI-, CI-, Release- oder Graphänderung; keine neue Review-Kampagne.

## Vertrag und Befehle

| Befehl / Input | Output und Grenze |
|---|---|
| `share-desktop.sh up ROOT SE_BIN SHARE_SERVER_BIN` | Benötigt ein frisches Fixture-ROOT und ausführbare bestehende Entwicklungsbinaries. Alte `state.env`-/Desktop-/TLS-Daten werden nicht überschrieben. Erstellt private HOME/XDG-Verzeichnisse, frisches Zertifikat/Key und separate Server-Bindings. Ermittelt die Runner-IPv4 und ein freies benachbartes Portpaar. Wartet auf echte TLS-/HTTPS-Listener und den verbundenen Desktop-Relay. Schreibt `ROOT/state.env` (Version 2) mit tatsächlicher CLI-Identität, Direct-Code, Device, Room-Code, Room-Datei und SHA-256. |
| `share-desktop.sh args ROOT` | Unverändert tokenweise `-e name value` für `seShareServer`, `seRoomCode`, `seDesktopDevice`, `seDesktopDirectCode`, `seRoomFolder`, `seRoomFile`, `seRoomFileSha256`. `seShareServer` ist jetzt die tatsächliche `wss://IP:PORT/#sha256=64lowerhex`-URI. Keine erfundenen Codes oder Identitäten. |
| `share-desktop.sh accept ROOT SECONDS INSTRUMENT_OUT` | Neuer befristeter Vordergrund-Companion der ReviewShare-Instrumentierung. `SECONDS` ist 1–9999 ohne führende Null; `INSTRUMENT_OUT` ist die vom Suite-Besitzer fortlaufend geschriebene Instrumentierungsausgabe. Wartet ausschließlich auf die reale CLI-Inbox der frischen Fixture. Nimmt exakt die einzige aktuelle, annehmbare, konfliktfreie tracked Incoming-Anfrage des Phones `RV1-Android-Share` an. Gibt bei Erfolg die native CLI-JSON-Entscheidung auf stdout aus und erhält sie als `ROOT/direct-accepted.json`. Ende der Instrumentierung, Fristablauf, Fremd-/Legacy-/Mehrfach-/Konfliktanfrage oder unbestätigte Persistenz führen zu nonzero. Je Fixture wird genau ein Annahmeaufruf reserviert. |
| `share-desktop.sh members ROOT COUNT` | Bisheriger Mitgliedschaftscheck mit denselben CLI-/Room-Inputs; Prozessbesitz zusätzlich registriert. |
| `share-desktop.sh exec ROOT [INSTRUMENT_OUT]` | Bestehender expliziter Exec-Fixture-Ablauf und dessen Exit-/Markervertrag unverändert; keine zusätzliche Exec-Freigabe. |
| `share-desktop.sh reach ROOT PHONE_DEVICE SECONDS` | Bestehender Room-Erreichbarkeitscheck unverändert; Prozessbesitz zusätzlich registriert. |
| `share-desktop.sh down ROOT LOGDIR` | Beendet registrierte laufende Helper, den zur privaten XDG-Fixture gehörenden Daemon und den TLS-Server. Prüft PID plus Linux-Startzeit und signalisiert eine Prozessgruppe nur beim bestätigten Gruppenleader. TERM-Frist 10 s, danach KILL und weitere 3 s je gehaltenem Prozess; ein weiter lebender Prozess führt zu nonzero. Sichert Status- und Fixture-Logs. Ein Fehler beim Anlegen/Kopieren der Logs verhindert den Prozessabbau nicht. |

`up` führt keinen Build aus. Remote-Runtime-Voraussetzungen: Linux-/proc, `ip`, `jq`, GNU `timeout`, OpenSSL mit `req -addext`, `python3`, `setsid`, `sha256sum`, `realpath` sowie die bisherigen Shell/Coreutils-Werkzeuge. Fehlende Werkzeuge führen zu einem klaren Setupfehler; der Helper installiert nichts.

Der Hauptagent startet `accept` parallel zur Instrumentierung, besitzt dessen Prozessgruppe, Stop und Wait und sammelt stdout/stderr in seinen Suite-Logs. Der Helper startet keine dauerhafte Annahmeschleife. `down` kann die zusätzlich registrierten Helper anhand ihrer Prozessidentität abbrechen. Die bisherigen Android-Argumentnamen und Room-Datei-Inhalte bleiben kompatibel.

## TLS-, Rechte- und Direct-Anschluss

Der Pin ist SHA-256 des DER-Endzertifikats, nicht des PEM-Texts oder nur des Public Keys. `openssl x509 -outform DER | sha256sum` folgt der gespeicherten Primärreferenz. Das Zertifikat hat den tatsächlichen Runner-IP-SAN und `CA:FALSE`; Schlüssel und temporärer Zustand entstehen mit `umask 077`.

Die vorhandene Server-API bekommt `--tls-cert`, `--tls-key` und `--state-file`. `SE_IROH_RELAY_BIND` fixiert den ermittelten Nachbarport, `SE_IROH_RELAY_DISABLE=0` aktiviert den Relay. `main.rs` gibt dasselbe Zertifikat an Signaling und Relay weiter; der Relay meldet HTTPS. `SE_SHARE_ALLOW_PLAINTEXT=0` lässt keinen stillen Klartext-Rückfall zu. Die bestehende optionale Key-Login-Policy wird für diese Fixture auf ihren Default false festgelegt; daraus entstehen keine Exec-/Schreibrechte.

Der Room bekommt explizit nur `share export policy --room Team --confirm-new-members false`. Der native Policy-Text bestätigt weiterhin `members_may_write=false`; die Export-JSON bestätigt genau `RoomDocs` mit `access=read_only` ohne Systemwrite-Opt-in. Write-/Exec-Defaults und die bisherigen expliziten Phone-Exec-Freigaben werden nicht erweitert.

Frische Desktop-/Serverdaten und die anfänglich leere reale `share request list --json`-Historie schließen die Wiederverwendung gespeicherter Anfragen aus. Der Companion verwendet anschließend `share request --json`, dessen Inbox aktuelle Pending-/Eligibility-Zustände liefert. Er verlangt `count=acceptable_count=1`, `blocked_count=0`, keine Legacy-Anfrage, passende Richtung/Phone-Name und vollständige nichtleere Device-/Node-/Public-Key-/Fingerprintwerte.

Die Mutation ist genau:

```text
se share request accept <request_uuid> --fingerprint <signed_peer_fingerprint> --json
```

Der Helper evaluiert keinen vom Provider gelieferten Befehlsstring. Der CLI-Mutator prüft die konkrete UUID-/Fingerprint-/Eligibility-Grenze erneut. Die native Ausgabe muss `action=accepted`, denselben Request, `authorization.active=true`, `decision.state=accepted` und dieselbe vollständige Peer-Identität wie die geprüfte Inbox enthalten. Bei einem unklaren Fehler wird kein zweites Accept oder Legacy-Fallback versucht; Entscheidungs-/Fehlerlogs bleiben als Evidenz erhalten.

A-CLIENT bestätigte am 2026-10-03 den echten Testnamen `RV1-Android-Share` vor `share.requestAccess` und genau eine Anfrage an `seDesktopDirectCode`. Dessen außer-scope Testquelle wurde nicht vom Helper-Worker gelesen oder geändert.

## Runtime-Artefakte und Abbau

Unter ROOT: private `desktop/`, `tls/server.pem`, `tls/server-key.pem`, `state.env`, `server-bindings.json`; das bestehende Room-Exportfile unter `desktop/home/room-export/`. Laufende Companion-Calls hinterlassen private `helper-owner-<pid>-<starttime>/process`-Records; `direct-accept-owner/` reserviert die einmalige Annahme.

Sichere Diagnosekopien nach LOGDIR: `desktop-share-status.json`, `share-server.log`, `tls-generation.log`, `room-policy.txt`, `direct-initial.json`, `direct-inbox.json`, `direct-inbox.err`, `direct-decision.json`, `direct-accepted.json`, `direct-accept.err` sowie bestehende `exec-*.out`/`exec-*.err` und `reach.err`, soweit vorhanden. Der TLS-Private-Key und Desktop-Credentials werden nicht in die Logkopien aufgenommen. Der up-Fehlerhandler beendet bereits gestartete eigene Prozesse und sammelt verfügbare Logs in `ROOT/failure-logs/`.

Der Portpaar-Discovery schließt seine Prüfsockets vor Serverstart. Eine spätere Bind-Kollision führt zu einem Setupfehler mit erhaltenen Logs; der Helper behauptet keine atomare Portreservierung und wechselt nicht zu einem fremden Host.

## AcceptanceSelector der einen Root-Suite

Keine Fixture wurde lokal ausgeführt. Der Hauptagent integriert diese Signale in denselben vorgesehenen Remote-Suite-Aufruf:

| Vorhandener Geräte-Selector / Fixture-Schritt | Konkretes Abnahmesignal |
|---|---|
| `app.smartexplorer.android.task.ReviewShareTaskTest#contactAndRoomAdmissionRequireCurrentFullPins` (Owner-Bestätigung A-CLIENT) | Echter frischer Desktop-Directcode, eine reale Phone-Incoming-Anfrage, native CLI-Annahme mit aktuellen vollständigen Pins; der Phone-Consumer beobachtet die persistierte echte Annahme. Companion wird anschließend beendet/gejoint. |
| `app.smartexplorer.android.task.ShareRoomTaskTest#joinTheDesktopRoomAndDownloadItsFile` | Phone verwendet die gepinnte Server-URI, tritt dem echten Room bei, findet RoomDocs und vergleicht die tatsächlich heruntergeladenen Bytes per SHA-256. Ausdrückliche Room-Read-Zulassung genügt; keine Write-/Exec-Erweiterung. |
| Dieselben Fixture-Aufrufe `up/args/down` | Wiederverwendung alter Fixture-Daten wird abgewiesen; Server/Daemon/Helper enden innerhalb der Besitzerfristen und Logs bleiben verfügbar. |

## Eigener API-/Syntax-Self-Review

CLI-Flags und Ausgabeformen wurden gegen die ausschließlich freigegebenen `config/relay/main`-, CLI-Request-/Inbox-/Lifecycle- und Export-Policy-Quellen gelesen. Die konkrete Read-Scope-Lücke für HTTPS-Relay wurde durch Parent-Freigabe von `share-server/src/{main,relay,server_tls}.rs` geschlossen.

`bash -n` hat ausschließlich die Shellsyntax gelesen und blieb erfolgreich; `git diff --check` blieb ohne Befund. Python-Portfinder und jq-Ausdrücke wurden statisch gelesen, nicht als Runtime-Fixture ausgeführt. Eigener Self-Review umfasst aktuelle UUID-/Fingerprint-Bindung, Inbox-Eligibility, frische Historie, unveränderte Argumente und Rechte, Prozessidentitäten, Fristen, Fehlerabbau und Logkopien.

Keine Builds, Compiler, Tests, Formatter, Server, Installationen, Git-Mutationen, CI, Release, Graph-Neubauten oder Unteragenten gestartet. Die Abstimmung mit dem bereits vorhandenen A-CLIENT-Owner war vom Parent ausdrücklich beauftragt.

## Gelesene / gezielt abgefragte Dateien

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/share-device-helper.json`
- `android/test-servers/share-desktop.sh`
- `docs/refs/rv1-remote-suite.md`
- `docs/refs/share-server-tls-auth.md`
- `native/src/cli/share/exports_policy.rs`
- `native/src/cli/share/requests_inbox.rs`
- `native/src/cli/share/lifecycle_output.rs`
- `android/app/src/androidTest/java/app/smartexplorer/android/task/ShareRoomTaskTest.kt`
- `share-server/src/config.rs`
- `native/src/cli/share.rs`
- `native/src/share/core/server.rs`
- `native/src/cli/share/requests.rs`
- `native/src/cli/share/exports.rs`
- `native/src/share/core/export_config.rs`
- `share-server/src/main.rs`
- `share-server/src/relay.rs`
- `share-server/src/server_tls.rs`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/SHARE-DEVICE-HELPER.md`

Bereits geladene `arbeitsweise`-/`graphify`-Anweisungen wurden wiederverwendet; gemäß dem engen Auftrag gab es keine weitere Graph-Abfrage oder Exploration.

## Geändert / erstellt / gelöscht

Geändert:

- `android/test-servers/share-desktop.sh`

Erstellt:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/SHARE-DEVICE-HELPER.md`

Gelöscht: keine Dateien.

## Offene Punkte

Keine offene API-/Scope-Anfrage. Die tatsächliche TLS-/Geräte-/Shutdown-Auswertung und die Einbindung des parallelen Companion-Aufrufs liegen beim Hauptagenten. Source-/Syntaxsignale sind kein ausgeführter Geräteabnahmenachweis. Der Worker stoppt nach diesem Handoff.
