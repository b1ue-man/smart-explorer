# FTP-Verbindungs-Pool – suppaftp 6.3.0 und Server-Grenzen

Stand: 2026-09-28. Quellen: lokaler Crate-Quellcode
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
- `DataStream<T>` (data_stream.rs:12-18) implementiert `Read` und `Write`; `RustlsStream::drop`
  sendet bei gesetztem `ssl_shutdown` `close_notify` (tls/rustls.rs:85-97), der Datenkanal endet
  also sauber beim Schließen.
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
