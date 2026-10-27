# R02: Linux-musl-ABI im vollständigen Releasebuild

Stand: 2026-10-27. Fixautor: delegierter Codex-Agent
`fix_release_linux_abi`. Gelesener HEAD `6e999188`, fehlgeschlagener
Quellkandidat `a94e1ac6546acbb57f37f67212ed53bda82d1e64`, Remote-Lauf
`40433720045`, Job `121968027634`, Abschluss mit Fehler um 03:51:28 UTC.
Dieser Bericht besitzt Fehlerliste, Verkettungskarte, Plan, Umsetzung und
eigene anschließende statische Anti-Regressionsreview desselben Autors.

## Auftrag und Erstplan aus dem übergebenen Befund

Der Ergebnisauftrag ist die notwendige Behebung zweier belegter
musl-Portabilitätsfehler vor demselben vollständigen Remote-Release 0.5.185.
Die wörtliche aktuelle Nutzeranweisung lautet: „du musst jetzt langsam endlich
mal n release bauen, was machst denn noch?“ Der geltende Fixautorenvertrag
vom 2026-10-26 liegt in `/tmp/smartexplorer-final-review-user-contract.txt`.
Der Hauptagent hat die beiden Planungs- und Recherchepässe bereits in
[release-recovery.md, R02](release-recovery.md#r02-linux-musl-abi-des-vollständigen-releasebuilds)
festgehalten. Die abgeschlossenen normalen und zusätzlichen
Regressionsreviews werden nicht wiederholt; es entsteht kein neuer
funktionaler Tasklauf.

Promptwissen: Sechs konkrete Compilermeldungen betreffen genau zwei
Rustdateien und `libc 0.2.186` unter `x86_64-unknown-linux-musl`.
Windowsinstaller, GNU-Linuxprogramm und Android-APK wurden laut übergebenem
Releasebefund bereits gebaut. Das belegt keinen vollständigen Release und
keine Laufzeitabnahme des hier entstehenden Fixes.

Eigener Erstplan vor der Produktänderung: Die bestehende atomare
Recovery-Veröffentlichung und die einzelne kernelgebundene
pidfd-Übergabe erhalten. Dazu die genauen libc-Zieltypen, vorhandene
Systemaufrufkonvention und die direkten sowie indirekten Verbraucher
nachlesen; danach nur die fehlende musl-Bindung und die vier
Längenausdrücke korrigieren. Die noch fehlenden Angaben sind die konkreten
Feldtypen und die tatsächlich berührten Aufrufketten; sie lassen sich an den
freigegebenen Quellen klären. Eine notwendige Nutzerauskunft fehlt nicht.

Schreibgrenze sind ausschließlich die beiden unten aufgeführten Rustdateien
und dieser neue Bericht. Main besitzt Graph, TODO, Suite/Manifest,
Integration, Commit, Push und vollständigen Remote-Release. Dieser Autor
führt keine Builds, Compiler, Formatter, Tests, Server, Installationen,
Gitmutationen, Graphneuerzeugung, Releasebefehle oder Agentenstarts aus.

## Belegte Fehlerfunktionen und Bestandskarte

Die vollständige Diagnose steht in
`/tmp/smartexplorer-release-40433720045-linux-errors.log`.

| Fehlerfunktion | Beleg und konkrete Folge |
|---|---|
| `mount/os/linux_os/recovery_storage.rs::publish` | E0425: `libc::renameat2` ist für das musl-Ziel nicht exportiert. Die Recovery-OSgrenze verhindert dadurch das Bauen der statischen Linuxprogramme. |
| `apps/os/linux_os/window_correlation.rs::send_self` | Zwei E0308: `msg_controllen` und `cmsghdr::cmsg_len` erwarten musl-`socklen_t` statt des festgelegten `usize`. Die Senderkonstruktion ist nicht kompilierbar. |
| `ChildWitness::process` derselben Datei | E0308 für die Empfangspufferlänge sowie E0308/E0277 für den Headerlängenvergleich: dieselbe musl-/GNU-Typverwechslung. Die Empfängerkonstruktion ist nicht kompilierbar. |

Die drei fehlerhaften Funktionen gehören zu zwei ABI-Ursachen; es gibt
sechs Diagnosen, nicht sechs eigenständige Verhaltensfehler.

| Direkt oder indirekt verkettete Funktion | Bestehender, zu erhaltender Vertrag |
|---|---|
| `mount/mod.rs::recovery_storage` → `publish`; `local_access::DirectoryHandle::{open_root,directory_file}` | Nicht-Windows wählt diesen Recoveryadapter. Quelle und Ziel liegen im gleichen Parent; beide relativen Namen werden über denselben gehaltenen Parent-FD adressiert. Fehler bleiben `io::Result`; nach erfolgreichem Rename folgt Directory-`sync_all`. |
| `recovery_export_io::export_mount_recovery` → `publish(..., false)` | Privates Paket wird erst nach synchronisiertem Manifest und Cache-Namespace veröffentlicht. Bestehende Paketnamen werden nicht überschrieben; lokale Nutzdaten und gespeicherte Identitäten werden nicht neu interpretiert. |
| `recovery_import_io::{copy_recovery_package,restore_recovery_cache}` → `publish(..., false)` | Kopie und Wiederherstellung behalten Manifest-/SHA-256-Verifikation, private Stufen und unveränderte Originalcaches. Cachebasis und fehlende Cache-IDs werden nur ohne Ersetzen installiert. Vorhandene Ziele werden zuerst geprüft. |
| `recovery_identity_io::bind_mount_recovery_identity` → `publish(..., false)` | Cacheverzeichnis und erste unveränderliche Identitätsdatei werden ohne Ersetzen angelegt; vorhandene abweichende Autorität bleibt ein Konflikt. Journal-/Payloadprüfung und Lease bleiben unverändert. |
| `MountManager::{save_remove,recovery_import,recovery_reactivate}` und `ensure_recovery_directory` in `mount_recovery_transaction.rs` | Export/Import bleiben getrennt von laufendem Host und Remote-I/O. Die App-eigene Recoveryablage wird ohne Ersetzen veröffentlicht. Reaktivierung bleibt nach Identitätsprüfung gestoppt, mit `auto_remount=false`. |
| `mount_inactive_recovery.rs::{write_document,replace_private}` → `publish_recovery_object(..., true)` | Nur der Eigentümercommit erlaubt ausdrücklich Ersetzung. Dateiflush geht voraus; beide Ownership-Checkpoints und vollständige vorige Dokumentbytes für Rücknahme bleiben erhalten. Ein fehlgeschlagener Parentflush kann nach erfolgtem Rename eintreten und muss weiter zurückgemeldet werden. |
| `apps/os/host_launch.rs::dispatch_claimed` → Linux-`Platform::dispatch` → `prepare` → `Command::spawn` → `send_self` | Exakter Katalog, Benutzer, Sitzung, Ablauf und bisherige Appargumente bleiben maßgeblich. Der private nichtblockierende Datagrammkanal wird vor `exec` nur für den eigenen pidfd benutzt. Fehlender Witness blockiert den Appstart nicht. |
| Linux-`record` → `ChildWitness::process` → `alive`/`image`/Proc-Identität → gemeinsame `window_correlation::record` | Empfangener kernelgebundener pidfd, tatsächliche UID, Sitzung, Startzeit und ausführbares Image bleiben die Autorität. Numerische PID-Wiederverwendung liefert keinen Ersatzbeleg. Der spätere Reaper erhält dasselbe `Child`. |
| `dispatch_claimed` → `Correlations::take_started`; `activate_known` und `confirm` → `host_lifecycle::{correlate_window,note_window,window_pending}` | Dispatch- und vollständige Requestbindung, Prozesslebenszeit, echte Fensterprüfung und Pinübergabe bleiben erhalten. Unbekanntes Fenster bleibt ausstehend; Timeout startet keine zweite App. |

Die letzte Zeile nennt die an den freigegebenen Aufrufstellen sichtbare
Fensterintegration. Dateien der CLI-, Share-, Bildschirm- und
Remote-Windowimplementierung wurden in diesem Auftrag nicht selbst geöffnet;
deren unveränderte Oberflächen-/Transportverträge werden aus dem vorhandenen
Plan wiederverwendet, nicht als neue vollständige Quellreview ausgegeben.

## Eigener Quellenpass und finaler Erhaltungsplan vor Änderung

Erster eigener Quellenpass: `native/Cargo.lock` bindet `libc` exakt auf
0.2.186. Dessen `src/unix/linux_like/linux/gnu/mod.rs` verwendet für beide
Control-Längen `size_t`; `src/new/musl/sys/socket.rs` verwendet
`socklen_t` mit ABI-Padding. Die sichtbare E0308-Diagnose bestätigt `u32`
für den konkreten musl-Release. Die gemeinsame Implementierung in
`src/unix/linux_like/mod.rs` berechnet `CMSG_SPACE`/`CMSG_LEN` aus dem
jeweiligen `cmsghdr` und gibt `c_uint` zurück. `local_access/os/linux/`
verwendet bereits `syscall(SYS_renameat2, ...)` in `confined.rs` und
`quarantine.rs`.

Am 2026-10-27 erneut abgerufene Primärquellen:

- [rename(2)](https://man7.org/linux/man-pages/man2/rename.2.html): Parent-FDs,
  atomare Ersetzung, `RENAME_NOREPLACE` und Fehler.
- [syscall(2)](https://man7.org/linux/man-pages/man2/syscall.2.html): direkter
  Aufruf eines Kernelsystemsaufrufs über dessen Nummer und errno-Weitergabe.
- [cmsg(3)](https://man7.org/linux/man-pages/man3/cmsg.3.html): Zielheader,
  Alignment, Control-Puffer und Headerlänge.
- [unix(7)](https://man7.org/linux/man-pages/man7/unix.7.html): Datagramme und
  Deskriptorübergabe durch `SCM_RIGHTS`.

Zweiter eigener Lückenpass: Die architekturspezifischen
`SYS_renameat2`-Konstanten sind im gelockten libc für x86_64 GNU/musl und
Android aarch64/x86_64 vorhanden. Androids vorhandene Anpassung von
`RENAME_NOREPLACE` auf `c_uint` bleibt erhalten. Der Systemaufruf hat hier
nur Deskriptoren, Zeiger und ein 32-Bit-Flag, keine aufzuteilenden
64-Bit-Wertargumente. Für GNU und Android besteht kein Anlass, den schon
vorhandenen libc-Wrapper zu ersetzen.

Die Control-Nutzlast bleibt genau ein `i32`. Die `[usize; 8]`-Puffer haben
auf den betrachteten 32-/64-Bit-ABIs 32 beziehungsweise 64 Byte; die
entsprechenden Header-/Nutzlängen liegen vollständig darin und passen in
`u32` und `usize`. Drei Zuweisungen dürfen ihren konkreten Feldtyp über
`as _` erhalten. Im Vergleich werden beide Werte ausdrücklich als `usize`
verglichen; damit entsteht keine mehrdeutige RHS-Typinferenz und kein
Wertverlust. Nullinitialisierung einschließlich musl-Padding bleibt erhalten.

| Arbeitspunkt | Konkrete Änderung | Testbares erwartetes Ergebnis ohne neuen Lauf |
|---|---|---|
| Musl-Publishbindung herstellen | In `recovery_storage.rs::publish` nur unter `target_env="musl"` `libc::syscall(libc::SYS_renameat2, ...)` aufrufen; für GNU und Android denselben bisherigen `libc::renameat2`-Ausdruck behalten. | Musl besitzt eine vorhandene FFI-Bindung zum selben Linuxaufruf. Beide Parent-FDs, Namensbytes, `replace`-Flags, Fehlerpfad und anschließendes fsync bleiben identisch. Keine schwächere Rename- oder Kopierausweichstrecke. |
| Control-Längen ABI-gerecht machen | In `send_self` beide Feldzuweisungen mit `as _`; in `ChildWitness::process` Pufferlänge mit `as _` und Headerlängenvergleich mit zwei expliziten `usize`-Operanden. | GNU und musl bilden dieselbe einzelne SCM_RIGHTS-Nachricht ab. Kein Feldlayout wird selbst nachgebaut. Stackpuffer, pidfd-Besitz, Close-on-exec, Trunkierungs-/Headerprüfung und Appstart bleiben unverändert. |
| Eigene anschließende Anti-Regressionsreview | Gesamte fertige Rustdateien und ihren begrenzten Diff gegen HEAD erneut lesen; obige Aufrufketten und beide ABI-Definitionen abgleichen. | Keine Quelländerung außerhalb der notwendigen FFI-Aufrufwahl, vier Längenausdrücke und erklärenden Kommentare; keine behauptete Laufzeitabnahme. |
| Übergabe an Main | Endgültige Rustdateien melden; vollständigen Bericht abschließen. | Main kann den Rootgraph aktualisieren, integrieren, committen und pushen und denselben vollständigen Remote-Release 0.5.185 fortsetzen. Die Kompilierbestätigung entsteht dort. |

Der finale Plan wurde vor Änderung beider Produktdateien festgehalten.
Kein APIparameter, kein weiterer Schreibpfad und keine zusätzliche
Abhängigkeit sind für diesen Fix offen.

## Umsetzung und eigene anschließende statische Anti-Regressionsreview

Der vorstehende Plan ist umgesetzt. `publish` hat genau einen zusätzlichen
musl-Zweig für den unveränderten Kernelvorgang; der bisherige Aufruf bleibt
unter `not(target_env="musl")` erhalten. In `send_self` und `process` sind
genau die vier belegten Längenausdrücke zielgerecht typisiert. Ein Kommentar
begründet die GNU-/musl-Feldunterschiede; ein Kommentar benennt die fehlende
musl-Wrapperbindung. Kein Format, keine Versionsnummer, keine Abhängigkeit,
kein Aufrufer und kein Appstartargument wird geändert.

Nach dem Edit las derselbe Autor beide fertigen Rustdateien vollständig und
ihren gesamten Diff erneut. Die eigene statische Nachreview ergab:

| Bestehender Vertrag / Grenzfall | Befund am endgültigen Quellstand |
|---|---|
| GNU und Android | Der bisherige `renameat2`-Aufruf, Androids `c_uint`-Flaganpassung und sämtliche nachfolgenden Fehler-/Synczeilen sind unverändert. Der Apps-Linuxadapter wird laut `apps/os/mod.rs` nur für Linux ausgewählt; Android behält seinen eigenen Adapter. |
| Musl-Kernelschnittstelle | `SYS_renameat2` und `syscall` sind in libc 0.2.186 vorhanden. Der zusätzliche Zweig übergibt dieselben fünf Argumente, mit denselben lebenden C-Strings und zweimal demselben gehaltenen FD. Er baut keinen Pfad neu auf und implementiert keinen eigenen Renameersatz. |
| Belegtes Nichtüberschreiben | `replace=false` ergibt unverändert `RENAME_NOREPLACE`. Paket-, Cache-, Recoveryverzeichnis- und Identitätsaufrufer behalten ihre bisherigen Werte. Ein belegtes Ziel bleibt ein Fehler; es gibt keine Vorab-Existenzprüfung mit anschließend überschreibendem Rename und keine Copy/Delete-Ausweichstrecke. |
| Ausdrücklich erlaubtes Ersetzen | `replace=true` ergibt unverändert Flag 0. Der Eigentümerwriter und dessen Rücknahme behalten ihren vollständigen privaten Datensatz, Checkpoints und Dateiflush vor der Veröffentlichung. |
| Fehler und Namespace | Jeder nicht erfolgreiche musl-Aufruf wird durch `last_os_error` zurückgegeben. Erfolgreicher Rename ruft genau denselben Directory-`sync_all` auf; ein danach scheiternder Flush bleibt ein Fehler für die bestehende Registry-Rücknahme. Keine Fehlerart wird als Erfolg behandelt. |
| Header-ABI und feste Werte | GNU-Zuweisungen inferieren weiter `usize`, musl-Zuweisungen `socklen_t`; der Vergleich hat ausdrücklich zwei `usize`-Werte. `CMSG_SPACE`, `CMSG_LEN`, `CMSG_FIRSTHDR` und `CMSG_DATA` verwenden weiterhin den tatsächlichen libc-Zielheader. Keine eigene Headerstruktur oder Paddingberechnung wurde eingeführt. |
| Einzelner pidfd und Besitz | `pidfd_open` bleibt vor `exec` im wirklichen Kind, der Payload genau ein `i32` und die Markerbytefolge genau `P`. Der Sender schließt seinen FD nach dem Sendeversuch. Der Empfänger übernimmt denselben validierten FD in `OwnedFd` und danach `Arc`; die Lebenszeitclosure hält ihn unverändert. |
| Nichtblockieren, Close-on-exec und Ablehnung | `SOCK_DGRAM`, `SOCK_CLOEXEC`, `SOCK_NONBLOCK`, `MSG_DONTWAIT`, `MSG_NOSIGNAL` und `MSG_CMSG_CLOEXEC` sind bytegleich. Nullheader, Level, Typ, genaue Headerlänge, Markerbyte und `MSG_CTRUNC` werden wie bisher geprüft. Der Cast verändert keine Prüfung und keine Nutzlast. |
| Appstart und Fensterzuordnung | `prepare`, die immer erfolgreiche Pre-exec-Closure, `record`, `alive`, `image`, UID-/Sitzungs-/Startzeitabgleich, `try_wait` und Reaperübergabe sind unverändert. Fehler beim Witness führen weiterhin zu keiner erfundenen Fensterzuordnung und blockieren den eigentlichen Start nicht. Gemeinsame Korrelation, Ablauf-/Sitzungsprüfung, Aktivierung und P4-Pinübergabe sind unverändert. |
| Paket-/Identitäts-/Persistenzdaten | Die Quelländerung erreicht ausschließlich die bisherige OS-Veröffentlichung. Manifestbytes, Hashberechnung, Payloadkopie, historischer Backend-/Kontobeweis, Cachebesitz, private Verzeichnisse, Lease, Journal und vollständige vorherige Registrybytes werden nicht geändert. |

`git diff --check` auf genau den drei eigenen Schreibpfaden meldete keinen
Whitespacebefund. Eine zusätzliche reine Textkontrolle aller drei Dateien
erfasste ausdrücklich auch den noch ungetrackten Bericht und fand keine
nachgestellten Leerzeichen oder Tabs. Die endgültigen Rustgrößen sind
59 Zeilen / 2.203 Byte für
`recovery_storage.rs` und 196 Zeilen / 7.427 Byte für
`window_correlation.rs`; beide liegen unter 500 Zeilen und 50 KiB. Dies
sind Quell-/Textkontrollen ohne Compiler, Linker oder Ausführung des
Produktverhaltens. Die eigene Review findet keinen verbleibenden belegten
Verhaltensverlust durch diesen begrenzten Diff.

## Tatsächliche Lektüre und vollständige Schreibdateiliste

Projektlektüre, nach ihrem tatsächlich verwendeten Umfang:

- Regeln/Evidenz: `AGENTS.md` aus Nutzerkontext und aktueller Projektdatei;
  `/tmp/smartexplorer-final-review-user-contract.txt` und
  `/tmp/smartexplorer-release-40433720045-linux-errors.log` vollständig.
  Geladener lokaler Skill: `/root/.codex/skills/arbeitsweise/SKILL.md` und
  der aktuelle delegierte MCP-Instruktionsstand.
- Projektkontext: `docs/ARCHITEKTUR.md` mit Architekturüberblick,
  Verantwortungsgrenzen, Datenwegen und Bau-/Prüfregeln;
  `docs/plaene/2026-10-24-batches/release-recovery.md`, insbesondere R02;
  `fix-f-app-launch.md` und `fix-f-recovery-registry.md` im selben
  Planverzeichnis mit Kontext-, Fehler-, Aufrufkarten- und
  Erhaltungsabschnitten. Die beiden längeren früheren Autorenberichte
  wurden nicht als neue Vollreview aller dort genannten Dateien behandelt.
- Vollständige tatsächlich verwendete OSdateien:
  `native/src/mount/os/linux_os/recovery_storage.rs`,
  `native/src/mount/os/windows/recovery_storage.rs`,
  `native/src/mount/os/shared/recovery_import.rs`,
  `native/src/mount/os/shared/recovery_identity.rs`,
  `native/src/daemon/os/shared/mount_recovery_transaction.rs`,
  `native/src/apps/os/mod.rs`,
  `native/src/apps/os/linux_os/mod.rs`,
  `native/src/apps/os/linux_os/launch.rs`,
  `native/src/apps/os/linux_os/session.rs`,
  `native/src/apps/os/linux_os/window_correlation.rs` und
  `native/src/apps/os/shared/window_correlation.rs`.
- Gezielte Abschnitte/Symbolabfragen:
  `native/src/mount/mod.rs` (OSauswahl/Reexporte),
  `native/src/mount/os/shared/recovery_export.rs` (Paketexport,
  Cache-/Identitätsbindung, Manifestflush, Namespace und Publish),
  `native/src/daemon/os/shared/mount_inactive_recovery.rs`
  (Eigentümerwriter, vollständige vorherige Bytes, Publish und Rücknahme),
  `native/src/apps/os/host_launch.rs` (Claim, Dispatch, bekannte Aktivierung
  und Witnessübernahme), `native/src/apps/os/host_lifecycle.rs`
  (Fensterkorrelation, Pending und `note_window`),
  `native/src/local_access/os/linux/directory_handle.rs`
  (gehaltener FD, Root-/Childöffnung),
  `native/src/local_access/os/linux/confined.rs` und `quarantine.rs`
  (vorhandene `SYS_renameat2`-Aufrufe),
  `native/Cargo.toml` (libc-/Zielabhängigkeit) und
  `native/Cargo.lock` (exakter libc-Record).
- Ausschließlich gezielte Symbolsuche ohne behauptete semantische
  Volllektüre: `native/src/local_access/mod.rs`,
  `native/src/apps/mod.rs`, `native/src/apps/os/host.rs` und
  `native/src/daemon/os/shared/mount_recovery_identity.rs`.
- Gepinnte Herstellerquelle
  `/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/libc-0.2.186/src/`:
  `new/musl/sys/socket.rs` vollständig;
  `unix/linux_like/linux/gnu/mod.rs` für beide Header;
  `unix/linux_like/mod.rs` für `CMSG_ALIGN`, `CMSG_FIRSTHDR`, `CMSG_DATA`,
  `CMSG_SPACE` und `CMSG_LEN`; gezielte Symbolsuche im vorhandenen
  `unix/linux_like/`- und `new/`-Baum für `renameat2`, `syscall`,
  `RENAME_NOREPLACE`, `SYS_renameat2` und die Control-Felder.
  Konkrete zusätzliche Konstantenabfragen betrafen
  `unix/linux_like/linux/musl/mod.rs`,
  `unix/linux_like/linux/musl/b64/x86_64/mod.rs`,
  `unix/linux_like/linux/gnu/b64/x86_64/not_x32.rs`,
  `unix/linux_like/android/b64/aarch64/mod.rs` und
  `unix/linux_like/android/b64/x86_64/mod.rs`.
- Die vier oben verlinkten Linux-Manpages wurden am 2026-10-27 erneut
  abgerufen. Der Rootgraph wurde von diesem Autor weder gelesen noch
  verändert; die verbindliche Graphaktualisierung gehört Main.

Die vollständige Liste der erstellten oder geänderten Dateien lautet:

| Datei | Eigene Änderung |
|---|---|
| `native/src/mount/os/linux_os/recovery_storage.rs` | Zusätzlicher musl-spezifischer Kernelaufruf; bisheriger GNU-/Android-Aufruf bleibt erhalten. |
| `native/src/apps/os/linux_os/window_correlation.rs` | Vier ABI-gerechte Längenausdrücke und ihr Kommentar. |
| `docs/plaene/2026-10-24-batches/fix-release-linux-abi.md` | Neuer eigener Fehler-/Aufrufkarten-/Plan-/Fix-/Nachreviewbericht. |

Der Dokumentationsgate bestand vor Anlegen des Berichts aus dem auf genau
diese Schreibpfade begrenzten `git status --short`, `git log -1 --oneline`,
der gezielten Suche nach R02/musl/renameat2/pidfd/Control-/Publishbezügen und
dem oben dokumentierten aktuellen Quell-/Diagnoseabgleich.

## Entscheidungen und genaue offene Grenzen

Entscheidung: Nur der fehlende musl-Wrapper wird durch denselben
Kernelaufruf ersetzt. GNU und Android behalten ihren bisherigen Wrapper;
es wird keine gewöhnliche Renamefunktion als schwächerer Ersatz benutzt.
Die Control-Feldtypen stammen weiter aus libc; feste kleine Werte werden
zielgerecht konvertiert, ohne Nutzdaten oder Sicherheitsentscheidungen zu
ändern. Ein zusätzlicher Integrationspfad oder eine neue Schreibfreigabe
wird nicht benötigt.

Die Kompilierung dieses neuen Diffs unter GNU, musl und Android sowie sein
Laufzeitverhalten sind hier ausdrücklich unbestätigt. Die vorhandenen
Fehler-/Rücknahmepfade sind statisch erhalten; es wird keine Haltbarkeit
bei realem Stromverlust oder mehrfachen I/Ofehlern behauptet. Main setzt
nach Integration und Graphrefresh denselben vollständigen Remote-Release
für 0.5.185 fort. Kein lokaler Build, keine zusätzliche Suite und kein
weiterer Reviewagent wurde ausgeführt oder angefordert.

Der eigene begrenzte Autorenzyklus ist abgeschlossen. Commit, Push,
Remote-Kompiliernachweis und vollständige Veröffentlichung bleiben bei Main.
