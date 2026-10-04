# Owned Sync-Provider-Fixtures

Primärquellen und aktuelle lokale Source geprüft am 2026-10-04. Diese Syntax
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
  und TLS-Module. `PROPFIND` mit `Depth: 0` ist der authentifizierte Readinesspfad.
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

## Interner Helpervertrag

`native/sync-reliability-providers.py::fixtures(logs, env, cli, share_server)`
ist ein Contextmanager. Er liefert eine vollständige Environmentkopie mit
`SE_SYNC_PROVIDER_MANIFEST` und auf Linux `SE_SYNC_FIXTURE_CA_DER`. Das Manifest
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
