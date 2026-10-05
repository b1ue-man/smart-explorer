# Owned Sync-Provider-Fixtures

Primärquellen und aktuelle lokale Source geprüft am 2026-10-05. Diese Syntax
gehört ausschließlich zur einen Remote-Task-Suite. Keine lokale Ausführung.
Vorhandene Syntaxrefs: [Taskruntime](sync-task-runtime-2026-10-04.md),
[Remote-Metadaten](sync-remote-metadata.md), [Samba](samba-container.md),
[ureq 2.12](gdrive-ureq-throughput.md), [Share TLS](share-server-tls-auth.md).

- [Docker container port](https://docs.docker.com/reference/cli/docker/container/port/):
  `docker port <owned-name> <port>/tcp` liefert die tatsächlich veröffentlichte
  Adresse. Fixture bindet ausschließlich `127.0.0.1::22` bzw. den jeweiligen
  internen Serverport; keine geratenen festen Kontrollports. FTP-Passivports
  werden als freie zusammenhängende Hostrange ermittelt und 1:1 veröffentlicht.
- [Apache mod_dav](https://httpd.apache.org/docs/2.4/mod/mod_dav.html): `Dav On`
  benötigt `mod_dav`/`mod_dav_fs` und einen für Apache schreibbaren `DavLockDB`.
  Der eigene TLS-Vhost nutzt Basic-Auth ausschließlich über HTTPS. Die Image-
  Standardkonfiguration bleibt bestehen; ein zusätzlicher Vhost lädt die DAV-
  und TLS-Module. Der Vhost besitzt `/tmp/sync-dav/data` als eigenes Repository,
  `Dav On` im URL-Scope `<Location />` und
  [DirectoryIndex disabled](https://httpd.apache.org/docs/2.4/mod/mod_dir.html#directoryindex)
  sowie `DirectoryCheckHandler On`, damit mod_dir den DAV-Handler respektiert.
  Die Image-Startseite ist kein DAV-Repository. Der
  [offizielle Image-Dockerfile](https://raw.githubusercontent.com/docker-library/httpd/master/2.4/Dockerfile)
  setzt heute `User`/`Group` auf `www-data`; der Helper ermittelt beide aus der
  tatsächlichen Konfiguration und setzt den Repositorybesitzer entsprechend.
  `PROPFIND` mit `Depth: 0` muss über authentifiziertes, CA-geprüftes HTTPS
  HTTP 207 und XML `DAV:multistatus` mit mindestens einer Response liefern.
  Remote-Lauf 37247279728 lieferte für den bisherigen Image-Root authentifiziert
  HTTP 405. Die [mod_dir-Source](https://raw.githubusercontent.com/apache/httpd/2.4.x/modules/mappers/mod_dir.c)
  zeigt den betroffenen Konfigurationsweg: der späte Directory-Fixup kann bei
  nicht aktiviertem Handlercheck die Index-Subrequest per internem Redirect
  übernehmen, obwohl mod_dav zuvor seinen Handler gesetzt hat. Eigenes leeres
  Repository, deaktivierter DirectoryIndex und Handlercheck beseitigen diesen
  Weg; HTTP 207 bleibt ein tatsächliches Remote-Abnahmeorakel.
- [Apache mod_ssl](https://httpd.apache.org/docs/2.4/mod/mod_ssl.html):
  `SSLEngine on`, `SSLCertificateFile`, `SSLCertificateKeyFile` konfigurieren
  den wirklichen TLS-Listener. Der Client prüft die tatsächliche Fixture-CA,
  den SAN und die Signatur. Die CA wird nur in cfg(test) zusätzlich in die
  vorhandenen Rustls-RootCertStores aufgenommen.
- [OpenSSL req](https://docs.openssl.org/3.0/man1/openssl-req/) und
  [OpenSSL x509](https://docs.openssl.org/3.0/man1/openssl-x509/): eigener
  kurzlebiger CA-Key, signierter Leaf mit `DNS:localhost,IP:127.0.0.1`,
  `extendedKeyUsage=serverAuth`; DER-CA für Rustls, DER-Leaf-SHA256 als normaler
  Share-Serverpin. Private Schlüssel verbleiben im eigenen Fixtureverzeichnis.
- [OpenSSL passwd](https://docs.openssl.org/3.0/man1/openssl-passwd/):
  `openssl passwd -apr1 -stdin` liefert den Apache-kompatiblen Hash für die
  eigene DAV-Passwortdatei; das Klartextpasswort steht nicht in den Argumenten.
- [ureq 2.12.1 AgentBuilder](https://raw.githubusercontent.com/algesten/ureq/2.12.1/src/agent.rs):
  `tls_config(self, Arc<rustls::ClientConfig>) -> Self` behält die tatsächliche
  Rustls-Prüfung bei. Alle drei DAV-Agenten erhalten im Test denselben zusätzlichen
  Roottrust; ihr bisheriges Pooling-/Mutation-once-Verhalten bleibt bestehen.
- [Microsoft New-SmbShare](https://learn.microsoft.com/en-us/powershell/module/smbshare/new-smbshare)
  und [New-SmbMapping](https://learn.microsoft.com/en-us/powershell/module/smbshare/new-smbmapping):
  Windows erzeugt eine eigene benannte temporäre Freigabe für den aktuellen
  Runnerprincipal, wählt eine tatsächlich freie Laufwerkskennung und erzeugt
  ein nichtpersistentes Mapping. Cleanup entfernt ausschließlich die eigene
  Freigabe und ihr eigenes Mapping; fehlende Rechte sind Abnahmefehler.
  Die gespeicherte UNC-Verbindung verwendet den bestehenden `Protocol::Share`-
  Vertrag mit Port `0`. Dieser Port gehört zum stabilen Credentialaccount,
  wird aber von `NetConnection`/Windows-WNet nicht als TCP-Port verwendet.
  Beim normalen Wiederöffnen akzeptiert der Connector deshalb explizites `0`
  ausschließlich für `Protocol::Share`; alle URL-Protokolle, einschließlich
  des separaten `Protocol::Smb`-TCP-Anschlusses, verlangen weiterhin einen
  positiven Port. Die Fixture ersetzt den UNC-Anschluss nicht durch `smb://`
  und ändert weder gespeicherte Accounts noch Credentials oder Root-Locators.
  Das mapped-Rootorakel vergleicht ausschließlich für diesen nativen Provider
  `Path`-Komponenten: `Z:/child` und der gültige native Childroot `Z:\/child`
  bezeichnen denselben Ort. Remote-Roots und Literalnamen werden dafür nicht
  über native Pfad-APIs geschickt; Namespace-, Byte- und No-op-Orakel bleiben.

## Konsistente FTP-Checksum-Signaturen

Der echte vsftpd-Anschluss im Remote-Lauf 37267225051 bestätigte STOR mit den
erwarteten Bytes, während der spätere Checksum-Scan `Unreadable` meldete.
Die [gesicherten FTP-Metadatenrefs](sync-remote-metadata.md#suppaftp-630-ftpftps-synchron)
beschreiben die Ursache: vsftpd liefert kein MLSx, LIST-Zeiten haben Minuten-
oder Tagesauflösung ohne Zeitzone, MDTM liefert dagegen UTC-Sekunden.
Ein unverändert strikter Capture-Guard darf diese unterschiedlichen Signaturen
nicht als dieselbe frische Beobachtung akzeptieren.

Reguläre, ansprechbare LIST-Kinder erhalten deshalb dieselben SIZE-/MDTM-
Fakten wie die Einzelabfrage, über einen gemeinsamen Probe ohne Enumeration.
Links, Verzeichnisse, Specialfiles und unrepräsentierbare Namen werden nicht
als reguläre Dateien geprobt. Unsupported-/550-Antworten beweisen keine
Abwesenheit: Beide Aufrufer behalten dann den vollständigen Eltern-LIST-
Vertrag einschließlich seiner gröberen Zeit. Weitere Probefehler werden als
geschützte Unreadable-Kinder gemeldet; unabhängige Einträge bleiben verfügbar.
Nur bei tatsächlich geprobten regulären Kindern wird Sekundenpräzision
gemeldet. Exakte Literalnamen einschließlich `%20` bleiben unverändert.
Der gemeinsame C04-Ablauf prüft weiterhin echte FTP/FTPS-Bytes, beidseitige
Änderungen, TLS-Verifikation und No-op; Engine-Capture, Backups, Partialscan-
und Lost-ACK-Grenzen werden dafür nicht gelockert.

## Interner Helpervertrag

`native/sync-reliability-providers.py::fixtures(logs, env, cli, share_server)`
ist ein Contextmanager. Er liefert eine vollständige Environmentkopie mit
`SE_SYNC_PROVIDER_MANIFEST`, `SE_SHARE_RELAY_URL` und auf Linux
`SE_SYNC_FIXTURE_CA_DER`. Das Manifest
enthält die zur Laufzeit gewonnenen Ports, normalen Providercredentials und
Direct-/Room-Locators. Credentials werden durch Rust über die reguläre
`save_connection_with_secret`-Transaktion gespeichert; Resolver öffnet sie
wieder. Der isolierte Profilepfad stammt aus der aufrufenden Suite.

Linux besitzt zwei unabhängige SFTP-Authorities, einen execfähigen OpenSSH-
Container samt tatsächlich deploytem Remote-Agent, FTP, FTPS, HTTPS-DAV und
Samba. Beide Plattformen besitzen einen TLS-Share-Server und zwei explizit
verwaltete `se --sync-daemon`-Prozesse. Identitäten, Direct-Code, acceptierter
Grant, Roomcode und Exportberechtigungen entstehen durch normale CLI-Aufrufe.
Der Peerprozess nutzt einen eigenen isolierten Profilepfad. Jede Prozess- und
Containerreferenz wird vor Readiness gespeichert; jeder Exitpfad schließt sie.
Signaling- und Relayport entstehen unabhängig durch OS-Portvergabe (`bind` auf
Port 0) mit tatsächlichem `listen`; Windows setzt vorher
[SO_EXCLUSIVEADDRUSE](https://learn.microsoft.com/en-us/windows/win32/winsock/using-so-reuseaddr-and-so-exclusiveaddruse).
Ein benachbarter Port wird nicht als verfügbar angenommen. Beide Listener und
das Weiterleben des eigenen Servers werden vor den Peercommands geprüft.
Die vorhandene normale Transportoption `SE_SHARE_RELAY_URL` führt die ermittelte
HTTPS-Relayadresse zu beiden Peerworkers und zum späteren Resolvertesthost;
die normalen Zertifikatpins bleiben wirksam. Peerreadiness verlangt zugleich
`worker.reachable`, `worker.running`, `worker.connected` und genau diese RelayURL.
IPC-Erreichbarkeit allein ist keine Share-Verbindungsreadiness. Ein vorzeitig
beendeter eigener Server unterbricht die nachfolgenden Readinesswaits sofort.
Der Mainpeer übernimmt den Namespace der Suite/Testhost-CLI; der separate Peer
bekommt einen eigenen gültigen `SMART_EXPLORER_E2E_TEST_NAMESPACE` (42 ASCII-Zeichen,
aus Suitekennung und eigenem Profilepfad gehasht). Windows-Credentialstore und
IPC-Kennung fallen dadurch trotz gemeinsamem Runnerbenutzer nicht zusammen.
Jeder Peer startet eine eigene byteidentische, SHA-256-geprüfte CLI-Kopie im
privaten Fixtureverzeichnis. Cleanup schreibt zuerst das normale `daemon.stop`
am tatsächlich veröffentlichten IPC-Profilepfad. Der Worker beendet sich damit
normal, und sein Guardian beendet die Beaufsichtigung. Cleanup wartet auf beide
Prozessgenerationen und das IPC-Ende. Bei Fristüberschreitung werden ausschließlich
Prozesse mit diesem exakten eigenen Executablepfad beendet: Windows bindet einen
Processhandle und prüft dessen MainModule erneut, Linux bindet einen PID-FD und
prüft Executable/Startzeit unmittelbar vor dem Signal erneut. Ein vollständiger
Abschluss wird in einem Cleanup-Log erst nach nachgewiesenem Prozess- und IPC-Ende
festgehalten; fremde Executables/Namespaces werden nicht beendet.
Die verwendeten Python-Signaturen sind
[`os.pidfd_open(pid, flags=0)`](https://docs.python.org/3/library/os.html#os.pidfd_open)
und [`signal.pidfd_send_signal(pidfd, signalnum, siginfo=None, flags=0)`](https://docs.python.org/3/library/signal.html#signal.pidfd_send_signal).
Die Linux-Eskalation benötigt Python ab 3.9 und Kernel ab 5.3; ein fehlender
PID-FD-Pfad wird nicht durch einen unsicheren numerischen PID-Kill ersetzt.

## Normaler Room-Konfigurationsrefresh

Im Remote-Lauf 37252322674 waren Direct-Setup und beide TLS-Worker verbunden;
der folgende normale Room-Export-/Policyrefresh meldete `StaleAuthorization`.
`apply_configuration_transition` erzeugte beim vorübergehenden Fehlen eines
bereits Exec-verweigernden Members eine synthetische Revocationrevision. Eine
später gelernte/persistierte Presence mit der ursprünglichen Default-deny-
Revision wurde dadurch abgelehnt. Für schon verweigernde Policies bleibt die
Revision beim Entfernen nun erhalten; entfernte tatsächlich aktive Exec-
Policies erhalten weiterhin die höhere Revocationrevision.

Die Exec-Registry validiert einen vollständigen Konfigurationsbatch unter
ihrem Lock, bevor sie Epoch, Restriktionsbarrieren, Cancellation und Policies
übernimmt. Ein abgelehnter Batch lässt den bisherigen Zustand unverändert;
ein gültiger Batch setzt alle Denybarrieren vor Veröffentlichung des neuen
Authsnapshots. Revision-Rollback und Enable derselben revoked Revision bleiben
verboten. Signaturfakten, Nonce-Replayzustand, Session-/Mount-/Rootgrenzen und
die gewöhnliche CLI-/Resolver-Konfigurationsfolge bleiben unverändert.

## Gespeicherte SFTP-Schlüsselanmeldung

Die zusätzliche Linux-Authority `sftp-key` gehört zur selben vollständigen
Providermatrix einschließlich Selbstpaar, beidseitiger Änderungen,
Gegenänderungen und No-op. Es gibt dafür keinen zweiten Suitefall.
[`ssh-keygen`](https://man.openbsd.org/ssh-keygen) erzeugt auf dem Remote-Runner
mit `-q -t ed25519 -N <eigene Passphrase> -f <Privatroot>/sftp-fixture-key`
einen verschlüsselten privaten Schlüssel und die dazugehörige `.pub`-Datei.
Beide Pfade entstehen unter dem eigenen temporären Privatroot; die Schlüssel
werden nicht in hochgeladene Logs geschrieben. Die private Datei bleibt auf
dem Host und hat Modus 0600.

Nach dem [atmoz-Key-Vertrag](https://raw.githubusercontent.com/atmoz/sftp/master/README.md)
wird ausschließlich die Publickeydatei read-only nach
`/home/<User>/.ssh/keys/sync-task.pub` gemountet. Der normale Entrypoint erstellt
`authorized_keys` mit seinen geforderten Rechten. Usersyntax `<User>::::upload`
enthält kein Loginpasswort; diese Gegenstelle kann deshalb nicht durch eine
versehentliche Passwortauthentifizierung erfolgreich geöffnet werden. Der
tatsächliche veröffentlichte SSH-Port stammt aus `docker port` nach Readiness.

Das vorhandene `SE_SYNC_PROVIDER_MANIFEST` enthält für diesen Anschluss zusätzlich
`key_path`; sein bisheriges `password`-Secretfeld enthält hier die Keypassphrase.
Rust persistiert exakt `AuthKind::Key { path }` und diese Passphrase durch die
normale Connection-/Credentialstoretransaktion, liest beides erneut und öffnet
den gespeicherten Locator über den normalen Resolver. Die bestehende Session
lädt mit `load_secret_key(path, passphrase)` und meldet sich per Publickey an.
Die unveränderten Passwortanschlüsse und der `use_agent`-Anschluss bleiben
Bestandteil der Matrix. `use_agent` bezeichnet den deployten SSH-Remote-Agent;
eine `SSH_AUTH_SOCK`-Anmeldevariante wird nicht eingeführt.
Private Keys, Credentials und Peerprofile liegen außerhalb hochgeladener Logs;
nach erfolgreichem Prozesscleanup wird ausschließlich der eigene temporäre Root
entfernt. Bei unvollständigem Cleanup bleibt dieser Root als private Diagnose
erhalten, ohne als Logartefakt veröffentlicht zu werden.

Windows besitzt UNC und dessen reales Mapping; die vollständige Protokoll-
Paarmatrix läuft auf Linux, während Windows seine tatsächlichen lokalen/UNC/
Mapping-/Direct-/Room-/Drive-Kombinationen prüft. Das Mapping ist ein Alias der
UNC-Freigabe und wird daher nicht als eigenständige Authority gegeneinander
synchronisiert. ZIP wird nur als vorhandener read-only `ZipBackend` an die
Engine gegeben; es entsteht kein erfundener gespeicherter ZIP-Locator.

`connect::sync_reliability_task_provider_fixture::providers()` stellt den
normalen Connectionstore her; `Provider::open(child)` öffnet einen eindeutigen
Child durch die normale Locatorauflösung. Der Drive-Vertragsserver wird an
exakten cfg(test)-Locators registriert und durch die wirkliche Drive-HTTP-
Implementierung verarbeitet. C10 benutzt unveränderte Google-Endpunkte.

Die Matrix liest nach Seed, beidseitigen Änderungen und Gegenänderungen die
wirklichen Bytes beider Provider. Der finale unveränderte Lauf darf keine
Datei kopieren/löschen und keine Transferbytes melden. Beide unabhängigen SFTP-
Authorities bekommen außerdem unter exakt gleichen relativen Roots bewusst
verschiedene Bytes; Identitäten und Inhalte müssen voneinander getrennt bleiben.
Dies beschreibt das Orakel, keinen bereits ausgeführten Erfolg.
