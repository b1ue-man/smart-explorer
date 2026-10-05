# FTP-Verbindungs-Pool – suppaftp 6.3.0 und Server-Grenzen

Stand: 2026-10-05; Pool-/Serverrecherche vom 2026-09-28 bleibt erhalten. Quellen: lokaler Crate-Quellcode
`/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/suppaftp-6.3.0/src/{types,status}.rs`,
`src/sync_ftp/{mod,data_stream}.rs`, `src/sync_ftp/tls/rustls.rs`, `src/lib.rs` (Version aus
`native/Cargo.lock`); vsftpd-Handbuch (security.appspot.com/vsftpd/vsftpd_conf.html),
ProFTPD-User-Guide (github.com/proftpd/user-guide, `directives/MaxClientsPer{User,Host}`),
rclone-Doku (rclone.org/ftp). Reine Fakten-Referenz für Block D (Plan K24/K25).

## 1. Fehlertypen

```rust
// types.rs:20-40
pub enum FtpError {
    ConnectionError(std::io::Error),
    SecureError(String),                 // Feature "secure"/"rustls"
    UnexpectedResponse(Response),        // Server hat mit einem anderen Code geantwortet
    BadResponse,                         // Antwortzeile nicht lesbar
    InvalidAddress(std::net::AddrParseError),
}
// types.rs:43-47
pub struct Response { pub status: Status, pub body: Vec<u8> }
// types.rs:89-98: Display = "[{code}] {Text}"; Response::as_string() -> Result<String, FromUtf8Error>
```

`Status` (status.rs, `#[repr(u32)]`, `code() -> u32`, `From<u32>` mit `Unknown = 0` für
Unbekanntes): u. a. `Ready = 220`, `LoggedIn = 230`, `NeedPassword = 331`,
`LoginNeedAccount = 332`, `RequestFilePending = 350`, `NotAvailable = 421`,
`InvalidCredentials = 430`, `NotLoggedIn = 530`, `FileUnavailable = 550`, `PathCreated = 257`,
`ClosingDataConnection = 226`, `RequestedFileActionOk = 250`, `AboutToSend = 150`,
`AlreadyOpen = 125`, `CommandOk = 200`.

`read_response_in(expected)` (sync_ftp/mod.rs:801ff.) liest die (ggf. mehrzeilige) Antwort und gibt
`Err(UnexpectedResponse(response))` zurück, wenn der Code nicht in `expected` liegt – der Code einer
Ablehnung bleibt also als `response.status` erhalten, bis der Aufrufer ihn zu Text macht.

## 2. Wo 421/530 ankommen

| Schritt | Methode (sync_ftp/mod.rs) | erwartet | Ablehnung |
|---|---|---|---|
| Begrüßung | `connect_with_stream` (:79-104) | 220 | 421 „too many connections“ als `UnexpectedResponse` |
| USER | `login` (:280-293) | 230 oder 331 | 421/530 als `UnexpectedResponse` |
| PASS | `login` | 230 | 530 (falsches Passwort **oder** Nutzer-/Host-Grenze), 421 |

Server-Konventionen (Stand 2026-09-28):
- vsftpd `max_per_ip`/`max_clients` (Standard 0 = unbegrenzt): „A client will get an error message
  if they go over this limit“; in der Praxis „421 There are too many connections from your internet
  address.“ bereits bei der Begrüßung.
- ProFTPD `MaxClientsPerUser`/`MaxClientsPerHost` (Standard `none`): Antwortcode **530** mit
  „Sorry, the maximum number of clients (%m) for this user already connected.“ bzw. „… from your
  host are already connected.“ – also bei der Anmeldung.
- Pure-FTPd: „421 Too many connections (N) from this IP“.

Folgerung (K25): 530 ist mehrdeutig (falsches Passwort ↔ Verbindungsgrenze). Nur nachdem sich
dieselbe Konfiguration schon einmal erfolgreich angemeldet hat, ist eine 421/530-Ablehnung einer
**weiteren** Verbindung eine Servergrenze; vor der ersten Anmeldung ist sie ein Anmeldefehler.

## 3. Übertragungsbefehle

- `retr_as_stream(path) -> DataStream` (:463-468): RETR, erwartet 150/125; danach
  `finalize_retr_stream(stream)` (:471-479): Strom zuerst schließen, dann 226/250 lesen.
- `resume_transfer(offset: usize)` (:569-575): `REST offset`, erwartet **350**; die nächste
  RETR beginnt dort. `usize` ⇒ auf 32-Bit-Zielen höchstens 4 GiB Versatz.
- `put_with_stream(path) -> DataStream` (:512-517): STOR, erwartet 125/150;
  `finalize_put_stream(stream)` (:522-530): Strom schließen, dann 226/250 (= Upload bestätigt).
  `put_file` (:500-506) ist genau `put_with_stream` + `io::copy` + `finalize_put_stream`.
- `abort(stream)` (:551-561): ABOR, Strom schließen, 226/426 und dann 226 lesen.
- `mkdir(path)` (:361-365): MKD, erwartet 257; ein vorhandener Name ergibt eine Ablehnung
  (üblich 550) – MKD ist serverseitig ein `mkdir(2)` und damit exklusiv.
- `DataStream<T>` (data_stream.rs:12-18) implementiert `Read` und `Write`.
  `RustlsStream::drop` versucht bei gesetztem `ssl_shutdown` nach einem Flush
  genau einen `write_tls` für `close_notify` (tls/rustls.rs:85-97). Fehler werden
  nur geloggt; dies beweist keinen vollständigen TLS-/TCP-Abschluss.
- Typen: `RustlsFtpStream = ImplFtpStream<RustlsStream>` (lib.rs:188).

## 4. Pool-Verhalten vergleichbarer Clients

rclone (rclone.org/ftp, 2026-09-28): `--ftp-concurrency` „Maximum number of FTP simultaneous
connections, 0 for unlimited. Note that setting this is very likely to cause deadlocks“ (Standard 0);
`--ftp-idle-timeout` Standard 1m0s: „If no connections have been returned to the connection pool in
the time given, rclone will empty the connection pool.“

Folgerungen für das Backend: Ein Pool, der beim Erreichen der Grenze wartet, kann sich mit einer
Operation verklemmen, die selbst schon eine Verbindung hält (Lesen + Schreiben auf demselben
Server). Deshalb meldet der Pool an der gelernten Grenze sofort Überlast
(`vfs::congestion_error`) statt zu warten; die Flow-Regelung nimmt die Parallelität zurück.

## 5. FTPS-Datenabschluss: zweite Recherche am 2026-10-05

Versionen aus dem aktuellen `native/Cargo.lock`: suppaftp **6.3.0**, rustls
**0.23.40**. Die gepinnte Crate nennt VCS-Commit
`96cb46417e65ccb0f9f0b7ec8613c141c151c6b5` und VCS-Pfad `suppaftp`.
Exakte Syntax ist lokal in den genannten Crates gesichert; öffentliche
[SuppaFTP-Source](https://github.com/veeso/suppaftp/tree/96cb46417e65ccb0f9f0b7ec8613c141c151c6b5/suppaftp/src).

```rust
// suppaftp 6.3.0, sync_ftp/data_stream.rs
pub enum DataStream<T> { Tcp(TcpStream), Ssl(Box<T>) } // T: internes TlsStream
pub fn get_ref(&self) -> &TcpStream;
// sync_ftp/mod.rs:522; schließt data zuerst, liest danach 226/250
pub fn finalize_put_stream(&mut self, data: impl Write) -> FtpResult<()>;
// tls/rustls.rs: standardmäßig ssl_shutdown=true, kein abschaltender Apppfad
// Drop: StreamOwned::flush; send_close_notify; einmal write_tls; Fehler nur loggen
```

Die Crate exportiert `RustlsConnector`, `RustlsFtpStream` und `DataStream`.
Ihr `RustlsStream` und dessen `TlsStream`-Trait sind am Crateroot nicht
öffentlich exportiert. Ein direkter Appaufruf von `mut_ref`, eigener
`TlsStream`-Implementation oder Zugriff auf dessen private TLS-Felder ist
damit kein verfügbarer API-Pfad dieser Version.

[Rustls 0.23.40](https://github.com/rustls/rustls/tree/v/0.23.40/rustls/src):
`CommonState::send_close_notify(&mut self)` stellt den Alert nur in die
Sendewarteschlange. `ConnectionCommon::write_tls(&mut self, &mut dyn Write)`
meldet geschriebene Bytes; der Puffer kann danach weiterhin gefüllt sein.
`wants_write()` zeigt verbleibende TLS-Bytes an. `StreamOwned::flush()`
delegiert an `Stream::flush` und dessen `complete_io`; ein einziges rohes
`write_tls` ist folglich kein genereller vollständiger Schreibbeweis.

[RFC 4217, 12.6/12.7](https://www.rfc-editor.org/rfc/rfc4217.html#section-12.7)
stellt den TLS-Abschluss und Datenkanalabschluss vor die positive
FTP-Abschlussantwort. [RFC 8446, 6.1](https://www.rfc-editor.org/rfc/rfc8446.html#section-6.1)
verlangt einen TLS-Abschlussalert vor dem Schließen der Schreibseite;
ein Transport-EOF allein beweist keine vollständige Übertragung.
Das [vsftpd-Handbuch](https://security.appspot.com/vsftpd/vsftpd_conf.html)
erläutert dieselbe Upload-Integritätsgrenze bei `strict_ssl_read_eof`.
Die bestehende echte Fixture und deren Integritätsprüfung bleiben erhalten.

[TcpStream::try_clone](https://doc.rust-lang.org/std/net/struct.TcpStream.html#method.try_clone)
liefert einen weiteren Besitzhandle desselben Sockets; Daten und Optionen
werden geteilt. `shutdown` betrifft den zugrunde liegenden Kanal, nicht nur
den jeweiligen Handle. Die
[Linux-TCP-Source](https://github.com/torvalds/linux/blob/21e4675d9305f6ccd20b95d943882d607c8ae288/net/ipv4/tcp.c)
sendet beim endgültigen Close mit ungelesenen Empfangsbytes einen Reset.
Ein unmittelbarer letzter Socket-Drop nach einem TLS-Schreibversuch ist
deshalb kein allgemein sicherer Abschluss, etwa bei TLS-Nachrichten nach
dem Handshake. Ob genau dieser Reset den konkreten Lauf ausgelöst hat,
ist aus dem Serverlog allein nicht bewiesen.

Remote-Lauf `37297823833` belegt unmittelbar: FTPS-STOR der 37-Byte-Stage von
FTP→FTPS (`pair-051`) endet mit 426; der echte Server meldet fehlenden
SSL-Abschluss. Die App finalisiert sowohl den normalen gespoolten Writer
als auch den bekannten-Längen-Stagewriter bisher durch den undurchsichtigen
Daten-Drop. Die Korrektur muss diese zusammenhängende Besitz-/Abschlussgrenze
behandeln, weiterhin die echte Abschlussantwort auswerten, Datenfehler und
451/452/552 erhalten und einen unklaren STOR niemals automatisch wiederholen.
TCP, REST/RETR, Timeouts, Poolgesundheit und verifizierte TLS-Anmeldung sind
dabei bestehende Verträge. Eine Lösung mit vorhandenen APIs vermeidet eine
für diesen Fehler nicht belegte breite Bibliotheks-/Protokollmigration.
