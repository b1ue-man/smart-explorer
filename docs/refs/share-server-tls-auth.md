# rustls 0.23.41 und rustls-pki-types 1.14.1 – TLS-Server (blockierend), Zertifikat-Reload, Client-Pinning
Quelle: https://docs.rs/rustls/0.23.41/rustls/server/struct.ServerConfig.html · https://docs.rs/rustls-pki-types/1.14.1/rustls_pki_types/pem/trait.PemObject.html · https://github.com/rustls/rustls/tree/642a10300a1e703279bdc70e73f7e6db8d99ccbf/rustls · https://github.com/rustls/pki-types/tree/bb3c1da0e69e8ee3dcdfb16c245f974d636ed481 · https://doc.rust-lang.org/std/net/struct.TcpStream.html · https://docs.openssl.org/3.0/man1/openssl-x509/ · Abgerufen: 2026-10-02

Konvention (gilt für die ganze Datei): `[Datei:Zeile]` = Quelltext aus dem crates.io-Archiv (z. B. `https://static.crates.io/crates/rustls/rustls-0.23.41.crate`, Pfad `src/`; entspricht dem genannten GitHub-Commit). „Folgerung“ = aus den Quellen abgeleitet, steht dort nicht wörtlich; „Entwurf“ = eigener Vorschlag ohne Quelle; „Ungeprüft“ = nicht verifiziert. Alle Codebeispiele sind nicht kompiliert (lokale Builds sind untersagt), nur gegen die zitierten Signaturen gelesen.
Stand im Repo: `share-server/Cargo.lock` rustls 0.23.41, rustls-pki-types 1.14.1, ring 0.17.14, kein aws-lc-rs; `native/Cargo.lock` rustls 0.23.40 (Quelltext-Diff 0.23.40 gegen 0.23.41: nur `read_buf`-Typangaben unter nightly-cfg und eine Debug-Ausgabe, alle Signaturen unten gelten für beide).

## Cargo-Features und Crypto-Provider (ring)
- Default-Features von rustls: `aws_lc_rs`, `logging`, `prefer-post-quantum`, `std`, `tls12`; `ring = ["dep:ring", "webpki/ring"]`; `std` schaltet `pki-types/std` ein [Cargo.toml `[features]`]. Ohne `std` gibt es weder `ServerConfig::builder*` noch `ServerConnection` noch `StreamOwned` [server/server_conn.rs:458-459, lib.rs:565-566, 664-665].
- Repo-Stand: `native/Cargo.toml:58` `rustls = { version = "0.23", default-features = false, features = ["ring", "std", "tls12"] }`. `share-server/Cargo.toml` hat keine direkte rustls-Abhängigkeit; `iroh-relay` (Feature `server` enthält `tls-ring`) schaltet `rustls/ring` ein, `tokio-rustls` schaltet `rustls/std` ein, `tls12` schaltet iroh-relay nicht ein (`rustls = { version = "0.23.33", default-features = false }`) [vendor/iroh-relay-1.0.0/Cargo.toml; tokio-rustls-0.26.4 Cargo.toml `[dependencies.rustls]`].
  Folgerung: für eigenes Server-TLS in share-server `rustls = { version = "0.23.41", default-features = false, features = ["ring", "std", "tls12"] }` direkt eintragen. Cargo vereinigt auf eine rustls-Instanz; Default-Features nicht erben, sonst kommt aws-lc-rs dazu.
- Provider-Auflösung [crypto/mod.rs:227-290]: `CryptoProvider::install_default(self) -> Result<(), Arc<CryptoProvider>>` gelingt höchstens einmal je Prozess; `CryptoProvider::get_default() -> Option<&'static Arc<CryptoProvider>>`. `ServerConfig::builder()` und `ClientConfig::builder()` nehmen den Prozess-Standard; ist keiner installiert, wird er aus den Crate-Features abgeleitet, aber nur wenn genau eines von `ring`/`aws_lc_rs` aktiv ist (und `custom-provider` aus). Sonst Panic: „Could not automatically determine the process-level CryptoProvider from Rustls crate features. Call CryptoProvider::install_default() before this point to select a provider manually …“.
- `rustls::crypto::ring::default_provider() -> CryptoProvider` (kein `Arc`) [crypto/ring/mod.rs:31]; sein `KeyProvider` lädt Schlüssel über `sign::any_supported_type` [crypto/ring/mod.rs:55-61].
- Folgerung (Empfehlung): immer `ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))` bzw. `ClientConfig::builder_with_provider(..)` verwenden: kein globaler Zustand, kein Panic bei versehentlich zwei aktiven Providern. Vorbild im Repo: `vendor/iroh-relay-1.0.0/src/main.rs:623-629`; `iroh_relay::tls::default_provider() -> Arc<CryptoProvider>` (Feature `tls-ring`) liefert dasselbe [vendor/iroh-relay-1.0.0/src/tls.rs:229-231].
- Protokollversionen: `ConfigBuilder<S, WantsVersions>::with_safe_default_protocol_versions(self) -> Result<ConfigBuilder<S, WantsVerifier>, Error>` [builder.rs:200]. `DEFAULT_VERSIONS = ALL_VERSIONS = [TLS13, (nur Feature tls12) TLS12]` [versions.rs:34-46]. `with_protocol_versions(self, &[&'static SupportedProtocolVersion])` [builder.rs:207], z. B. `&[&rustls::version::TLS13]` [lib.rs:670-673]; Fehler `Error::General("no usable cipher suites configured")` [builder.rs:207-225].

## ServerConfig bauen
```rust
ServerConfig::builder() -> ConfigBuilder<ServerConfig, WantsVerifier>                               // server_conn.rs:459, nur std, Panik ohne auflösbaren Provider
ServerConfig::builder_with_provider(provider: Arc<CryptoProvider>) -> ConfigBuilder<ServerConfig, WantsVersions>   // :498
ConfigBuilder<ServerConfig, WantsVerifier>::with_no_client_auth(self) -> ConfigBuilder<ServerConfig, WantsServerCert>   // server/builder.rs:32
ConfigBuilder<ServerConfig, WantsServerCert>::with_single_cert(self, cert_chain: Vec<CertificateDer<'static>>, key_der: PrivateKeyDer<'static>) -> Result<ServerConfig, rustls::Error>   // builder.rs:65
ConfigBuilder<ServerConfig, WantsServerCert>::with_single_cert_with_ocsp(self, cert_chain, key_der, ocsp: Vec<u8>) -> Result<ServerConfig, rustls::Error>   // :87, leere OCSP-Antwort wird ignoriert
ConfigBuilder<ServerConfig, WantsServerCert>::with_cert_resolver(self, cert_resolver: Arc<dyn ResolvesServerCert>) -> ServerConfig   // :102
ServerConnection::new(config: Arc<ServerConfig>) -> Result<ServerConnection, rustls::Error>      // server_conn.rs:642
```
- `with_single_cert`: Schlüssel als PKCS#1, PKCS#8 oder SEC1 (ring und aws-lc-rs können alle drei); schlägt fehl bei ungültigem Schlüssel oder wenn die SubjectPublicKeyInfo des Schlüssels nicht zum ersten Zertifikat passt (`CertifiedKey::from_der` prüft `keys_match`, „unknown“ zählt nicht als Fehler). Das End-Entity-Zertifikat braucht eine Subject Alternative Name-Erweiterung, `commonName` wird ignoriert; ein Zertifikat für alle SNI-Namen [server/builder.rs:50-64, crypto/signer.rs:159-175].
- Voreinstellungen des fertigen `ServerConfig` [server/builder.rs:102-130]: `alpn_protocols` leer, `send_tls13_tickets: 2`, `session_storage: ServerSessionMemoryCache::new(256)`, `ticketer: NeverProducesTickets`, `max_early_data_size: 0`, `ignore_client_order: false`. Felder sind `pub` und nach dem Bauen änderbar (`alpn_protocols` [server_conn.rs:338]).
- Das `ServerConfig` wird als `Arc<ServerConfig>` geteilt, je Verbindung entsteht ein eigenes `ServerConnection`.
- Minimalbeispiel (Zertifikat aus PEM-Dateien):
```rust
use std::sync::Arc;
use rustls::{pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer}, ServerConfig};

fn server_config(cert_pem: &str, key_pem: &str) -> Result<Arc<ServerConfig>, Box<dyn std::error::Error>> {
    let chain = CertificateDer::pem_file_iter(cert_pem)?.collect::<Result<Vec<_>, _>>()?;
    let key = PrivateKeyDer::from_pem_file(key_pem)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?   // Result<_, rustls::Error>
        .with_no_client_auth()
        .with_single_cert(chain, key)?;           // Result<ServerConfig, rustls::Error>
    Ok(Arc::new(cfg))
}
```

## PEM laden (rustls-pki-types 1.14.1)
```rust
// rustls_pki_types::pem::PemObject, auch als rustls::pki_types::pem::PemObject erreichbar [lib.rs:677 in rustls]
trait PemObject: Sized {
    fn from_pem_slice(pem: &[u8]) -> Result<Self, pem::Error>;                                  // pem.rs:21, Feature alloc
    fn pem_slice_iter(pem: &[u8]) -> SliceIter<'_, Self>;                                       // :29, Feature alloc
    fn from_pem_file(file_name: impl AsRef<std::path::Path>) -> Result<Self, pem::Error>;       // :37, Feature std
    fn pem_file_iter(file_name: impl AsRef<std::path::Path>) -> Result<ReadIter<io::BufReader<File>, Self>, pem::Error>;   // :50, Feature std
    fn from_pem_reader(rd: impl io::Read) -> Result<Self, pem::Error>;                          // :60, Feature std
    fn pem_reader_iter<R: io::Read>(rd: R) -> ReadIter<io::BufReader<R>, Self>;                 // :68, Feature std
}
// Iterator-Item: Result<Self, pem::Error>
```
- Implementiert für `CertificateDer<'static>` (Abschnitt `CERTIFICATE`) und `PrivateKeyDer<'static>` (`PRIVATE KEY` -> `Pkcs8`, `RSA PRIVATE KEY` -> `Pkcs1`, `EC PRIVATE KEY` -> `Sec1`) [lib.rs:171-181 in rustls-pki-types]. Andere Abschnitte werden übersprungen (Schlüssel und Kette dürfen in einer Datei liegen); unbekannte Labels wie `ENCRYPTED PRIVATE KEY` oder `EC PARAMETERS` ebenfalls, ein verschlüsselter Schlüssel ergibt also `Error::NoItemsFound` [pem.rs:310-318, 393-480].
- Features: Modul `pem` braucht `alloc` (Default), Datei- und Reader-Funktionen `std`; rustls-Feature `std` schaltet `pki-types/std` ein, eine eigene Abhängigkeit auf rustls-pki-types ist nicht nötig, `rustls::pki_types` reicht [lib.rs:98-99; rustls Cargo.toml `std`].
- `pem::Error` (`#[non_exhaustive]`, `std::error::Error`): `MissingSectionEnd { end_marker }`, `IllegalSectionStart { line }`, `Base64Decode(String)`, `Io(io::Error)`, `NoItemsFound`, `SectionTooLarge` (Grenze 256 MiB je Abschnitt) [pem.rs:344, 481-506].
- `pem_file_iter` meldet Öffnungsfehler sofort (`Err(Error::Io)`), Lesefehler erst aus dem Iterator [pem.rs:44-52]. `from_pem_file` liefert den ersten passenden Abschnitt, sonst `NoItemsFound` [pem.rs:35-40].
- Stolperfalle: Reihenfolge der Kette ist die Reihenfolge in der Datei; das End-Entity-Zertifikat muss zuerst stehen [crypto/signer.rs:178-186]. Eine Datei ohne `CERTIFICATE`-Abschnitt ergibt eine leere Kette, die `with_single_cert` ablehnt (`Error::NoCertificatesPresented` über `end_entity_cert()`) [signer.rs:159-175, 203-208].

## Blockierender Server: ServerConnection + StreamOwned
```rust
// rustls::StreamOwned [stream.rs:164]; pub conn, pub sock; Read + Write
StreamOwned::new(conn: C, sock: T) -> StreamOwned<C, T>      // :183, kein I/O
StreamOwned::get_ref(&self) -> &T / get_mut(&mut self) -> &mut T / into_parts(self) -> (C, T)   // :188-198
```
```rust
use std::{io, net::TcpListener, sync::Arc, time::Duration};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

fn serve(config: Arc<ServerConfig>) -> io::Result<()> {
    let listener = TcpListener::bind("0.0.0.0:8443")?;
    for tcp in listener.incoming() {
        let tcp = tcp?;
        let config = Arc::clone(&config);
        std::thread::spawn(move || {
            // Fristen VOR dem TLS-Handshake setzen: er läuft lazy beim ersten read/write über diese Socket
            let _ = tcp.set_read_timeout(Some(Duration::from_secs(30)));   // Duration 0 -> Err(InvalidInput)
            let _ = tcp.set_write_timeout(Some(Duration::from_secs(30)));
            let _ = tcp.set_nodelay(true);
            let Ok(conn) = ServerConnection::new(config) else { return };
            let tls = StreamOwned::new(conn, tcp);
            // weiter: tungstenite::accept_hdr_with_config(tls, callback, Some(ws_cfg))
            let _ = tls;
        });
    }
    Ok(())
}
```
- Der Handshake läuft erst beim ersten `read`/`write` (`complete_prior_io` ruft `complete_io`, solange `is_handshaking`) [stream.rs:35-57]. TLS-Protokollfehler kommen als `io::Error` der Art `InvalidData` mit innerem `rustls::Error` [conn.rs:594, 677]. Wer den Handshake getrennt behandeln will (Fehler, SNI, ALPN, Version loggen), ruft vorher `tls.conn.complete_io(&mut tls.sock)` auf [conn.rs:602]; danach `conn.server_name() -> Option<&str>` [server_conn.rs:666], `alpn_protocol()`, `protocol_version()`, `negotiated_cipher_suite()` [common_state.rs:148-178].
- Lesen: `Ok(0)` nur nach `close_notify`; TCP-Ende ohne `close_notify` ergibt `io::ErrorKind::UnexpectedEof` („peer closed connection without sending TLS close_notify“); `WouldBlock`, wenn die Verbindung noch läuft und Daten fehlen [conn.rs:182-195, 216-230, 302]. Folgerung: `UnexpectedEof` ist bei vielen Clients ein normales Verbindungsende und sollte nicht als Angriff gewertet werden.
- Kein `Drop`-Verhalten: `StreamOwned` sendet beim Schließen kein `close_notify`. Sauber schließen: `tls.conn.send_close_notify()` [common_state.rs:581], danach `tls.conn.complete_io(&mut tls.sock)` [conn.rs:602].
- Fristen: rustls und tungstenite haben keine eigene Handshake-Frist. `set_read_timeout`/`set_write_timeout` der `TcpStream` wirken auch im TLS-Handshake; Unix liefert bei Ablauf `WouldBlock`, Windows kann `TimedOut` liefern [std TcpStream-Doku, „Platform-specific behavior“]. Folgerung: Slowloris-Schutz braucht eine eigene Gesamtfrist (z. B. Zeitpunkt merken und nach Handshake prüfen) und eine Obergrenze für gleichzeitige Verbindungen.

## Zertifikat-Neuladen ohne Neustart (ResolvesServerCert)
```rust
pub trait ResolvesServerCert: Debug + Send + Sync {                                   // server_conn.rs:124
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<rustls::sign::CertifiedKey>>;   // :129, None bricht den Handshake ab
    fn only_raw_public_keys(&self) -> bool { false }                                    // :132
}
ClientHello<'a>::server_name(&self) -> Option<&str> / signature_schemes(&self) -> &[SignatureScheme] / alpn(&self) -> Option<impl Iterator<Item = &'a [u8]>> / cipher_suites(&self) -> &[CipherSuite]   // :157-196
pub struct CertifiedKey { pub cert: Vec<CertificateDer<'static>>, pub key: Arc<dyn SigningKey>, pub ocsp: Option<Vec<u8>> }   // crypto/signer.rs:139
CertifiedKey::from_der(cert_chain: Vec<CertificateDer<'static>>, key: PrivateKeyDer<'static>, provider: &CryptoProvider) -> Result<CertifiedKey, Error>   // :159, prüft Schlüssel gegen Zertifikat
CertifiedKey::new(cert: Vec<CertificateDer<'static>>, key: Arc<dyn SigningKey>) -> CertifiedKey   // :180, OHNE Prüfung
CertifiedKey::keys_match(&self) -> Result<(), Error>                                  // :190
```
- `resolve` läuft je ClientHello: neue Handshakes sehen sofort das neue Zertifikat, laufende Verbindungen behalten das alte. Mit `ServerConfig::builder_with_provider(..).with_safe_default_protocol_versions()?.with_no_client_auth().with_cert_resolver(Arc<dyn ResolvesServerCert>)` einbauen.
- Fertige Bausteine in der Abhängigkeitskette: `rustls::sign::SingleCertAndKey` (statisch) [signer.rs:96-123]; `iroh_relay::server::reloading_resolver` (async, tokio, lädt alle 24 h neu, siehe iroh-relay-Abschnitt). Dessen Lader `rustls-cert-reloadable-resolver 0.7.1` ruft `CertifiedKey::new` ohne `keys_match` auf [rustls-cert-reloadable-resolver-0.7.1/src/loader.rs:71]: ein nicht zusammenpassendes Paar (Zertifikat erneuert, Schlüssel noch alt) wird übernommen und scheitert erst im Handshake. Reload-Fehler werden verschluckt, die Meldung „Reloaded the certificate“ erscheint auch dann [vendor/iroh-relay-1.0.0/src/server/resolver.rs:84-85]. Folgerung: Zertifikat und Schlüssel atomar ersetzen (Datei schreiben, dann `rename`) oder einen eigenen Resolver mit `from_der` benutzen.
- Entwurf (eigener, synchroner Resolver; Signaturen oben geprüft):
```rust
use std::{path::{Path, PathBuf}, sync::{Arc, RwLock}};
use rustls::{crypto::CryptoProvider, pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer},
             server::{ClientHello, ResolvesServerCert}, sign::CertifiedKey};

#[derive(Debug)]
pub struct ReloadableCert { provider: Arc<CryptoProvider>, cert: PathBuf, key: PathBuf, current: RwLock<Arc<CertifiedKey>> }

impl ReloadableCert {
    fn read(p: &CryptoProvider, cert: &Path, key: &Path) -> Result<CertifiedKey, Box<dyn std::error::Error>> {
        let chain = CertificateDer::pem_file_iter(cert)?.collect::<Result<Vec<_>, _>>()?;
        Ok(CertifiedKey::from_der(chain, PrivateKeyDer::from_pem_file(key)?, p)?)   // prüft Schlüssel <-> Zertifikat
    }
    pub fn new(provider: Arc<CryptoProvider>, cert: PathBuf, key: PathBuf) -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        let first = Self::read(&provider, &cert, &key)?;
        Ok(Arc::new(Self { provider, cert, key, current: RwLock::new(Arc::new(first)) }))
    }
    /// Periodisch oder bei geänderter Datei aufrufen; bei Fehler bleibt das alte Zertifikat aktiv.
    pub fn reload(&self) -> Result<(), Box<dyn std::error::Error>> {
        let fresh = Self::read(&self.provider, &self.cert, &self.key)?;
        *self.current.write().map_err(|_| "RwLock poisoned")? = Arc::new(fresh);
        Ok(())
    }
}
impl ResolvesServerCert for ReloadableCert {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.current.read().ok().map(|g| Arc::clone(&g))
    }
}
// let resolver = ReloadableCert::new(..)?;  ServerConfig::builder_with_provider(p).with_safe_default_protocol_versions()?.with_no_client_auth().with_cert_resolver(resolver.clone());
```

## Client: ClientConfig mit eigenem ServerCertVerifier (SHA-256-Pinning)
```rust
ClientConfig::builder_with_provider(provider: Arc<CryptoProvider>) -> ConfigBuilder<ClientConfig, WantsVersions>   // client/client_conn.rs:338
  .with_safe_default_protocol_versions()? -> ConfigBuilder<ClientConfig, WantsVerifier>
  .dangerous() -> DangerousClientConfigBuilder                                                                   // client/builder.rs:85
  .with_custom_certificate_verifier(verifier: Arc<dyn ServerCertVerifier>) -> ConfigBuilder<ClientConfig, WantsClientCert>   // :107
  .with_no_client_auth() -> ClientConfig                                                                         // :156
// Normalweg: .with_root_certificates(impl Into<Arc<RootCertStore>>) :51, .with_webpki_verifier(Arc<WebPkiServerVerifier>) :67
ClientConnection::new(config: Arc<ClientConfig>, name: ServerName<'static>) -> Result<ClientConnection, Error>   // client_conn.rs:715
```
`rustls::client::danger::ServerCertVerifier: Debug + Send + Sync` [verify.rs:69-156; Pfad lib.rs:608-612]. Pflichtmethoden:
```rust
fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, intermediates: &[CertificateDer<'_>], server_name: &ServerName<'_>, ocsp_response: &[u8], now: UnixTime) -> Result<ServerCertVerified, Error>;
fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error>;
fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error>;
fn supported_verify_schemes(&self) -> Vec<SignatureScheme>;
// optional mit Default: requires_raw_public_keys(&self) -> bool { false }, root_hint_subjects(&self) -> Option<&[DistinguishedName]> { None }
```
- Die Handshake-Signatur MUSS der Verifier selbst prüfen, sonst beweist der Server den Besitz des Schlüssels nicht: `rustls::crypto::verify_tls12_signature` / `verify_tls13_signature(message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct, supported_schemes: &WebPkiSupportedAlgorithms) -> Result<HandshakeSignatureValid, Error>` [webpki/verify.rs:155, 188; Export crypto/mod.rs:14-15]. Die Algorithmen kommen aus `provider.signature_verification_algorithms: WebPkiSupportedAlgorithms` (`Copy`, `.supported_schemes() -> Vec<SignatureScheme>`) [crypto/mod.rs:210, webpki/verify.rs:60-87].
- Entwurf (Pin-only, für selbstsignierte Zertifikate; prüft weder Namen noch Gültigkeitszeitraum, bewusst):
```rust
use std::sync::Arc;
use rustls::{client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
             crypto::{CryptoProvider, WebPkiSupportedAlgorithms},
             pki_types::{CertificateDer, ServerName, UnixTime},
             CertificateError, DigitallySignedStruct, Error, SignatureScheme};
use sha2::{Digest, Sha256};              // sha2 0.10

#[derive(Debug)]
pub struct PinnedCert { sha256: [u8; 32], algs: WebPkiSupportedAlgorithms }
impl PinnedCert {
    pub fn new(sha256: [u8; 32], provider: &CryptoProvider) -> Arc<Self> {
        Arc::new(Self { sha256, algs: provider.signature_verification_algorithms })
    }
}
impl ServerCertVerifier for PinnedCert {
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, _intermediates: &[CertificateDer<'_>],
                          _server_name: &ServerName<'_>, _ocsp: &[u8], _now: UnixTime) -> Result<ServerCertVerified, Error> {
        if Sha256::digest(end_entity.as_ref()).as_slice() == &self.sha256[..] {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure))   // error.rs:75, 503
        }
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.algs)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.algs)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> { self.algs.supported_schemes() }
}
```
- Der Pin ist SHA-256 über das DER des Endzertifikats, wie `openssl x509 -sha256 -in cert.pem -noout -fingerprint` ihn ausgibt („digest of the DER encoded version of the entire certificate“; ohne `-sha256` wäre es SHA-1) [openssl-x509(1)]. Ein Zertifikat-Pin bricht bei jeder Erneuerung. Folgerung: stattdessen die SubjectPublicKeyInfo pinnen (überlebt Erneuerung bei gleichem Schlüssel): `rustls::server::ParsedCertificate::try_from(&CertificateDer)?.subject_public_key_info() -> SubjectPublicKeyInfoDer<'static>` [webpki/verify.rs:129-143; Export lib.rs:654].
- Pin zusätzlich zur CA-Prüfung (Variante für CA-Zertifikate): `WebPkiServerVerifier::builder_with_provider(roots: Arc<RootCertStore>, provider: Arc<CryptoProvider>) -> ServerCertVerifierBuilder`, `.build() -> Result<Arc<WebPkiServerVerifier>, VerifierBuilderError>` [webpki/server_verifier.rs:114, 168]; in `verify_server_cert` zuerst an den inneren Verifier delegieren, danach den Pin prüfen.
- Namen: `ServerName::try_from(&str)` probiert erst DNS-Name, dann IP-Adresse; IPv6 nur ohne eckige Klammern [rustls-pki-types server_name.rs:117-128, 395-407]. Ein Pin-only-Verifier ignoriert `server_name`; ohne Pin braucht ein IP-Ziel eine IP-SAN im Zertifikat (Folgerung).
- Selbstsigniert ohne Pin: Ein Endzertifikat mit `basicConstraints cA=TRUE` wird von rustls-webpki als Endzertifikat abgelehnt (`CaUsedAsEndEntity`) [rustls-webpki-0.103.13 src/verify_cert.rs:436]. Ob ein `CA:FALSE`-Zertifikat als einziger Vertrauensanker (`RootCertStore::add`) akzeptiert wird: Ungeprüft, der Pin-Verifier umgeht die Frage.

# tungstenite 0.24.0 – blockierender WebSocket-Server und -Client über beliebige Streams
Quelle: https://docs.rs/tungstenite/0.24.0/tungstenite/ · https://docs.rs/tungstenite/0.24.0/tungstenite/fn.client_tls_with_config.html · https://docs.rs/tungstenite/0.24.0/tungstenite/protocol/struct.WebSocketConfig.html · https://github.com/snapview/tungstenite-rs/tree/00c00d4a625c9c52b125f2dbb370793e547085a6 · https://doc.rust-lang.org/std/net/struct.TcpStream.html · https://github.com/hyperium/http (http 1.4.2, Authority::host) · Abgerufen: 2026-10-02

Konvention: `[Datei:Zeile]` = Quelltext aus `https://static.crates.io/crates/tungstenite/tungstenite-0.24.0.crate`, Pfad `src/`. Stand im Repo: `share-server/Cargo.toml` `tungstenite = { version = "0.24", default-features = false, features = ["handshake"] }` (kein TLS-Feature, Server-TLS macht `rustls::StreamOwned`); `native/Cargo.toml:109` `features = ["handshake", "rustls-tls-webpki-roots"]`; beide Locks 0.24.0.

## Features
- `default = ["handshake"]`; `handshake = ["data-encoding", "http", "httparse", "sha1"]` schaltet `accept*`, `client*`, `connect*`, `Callback`, `HandshakeError` frei [Cargo.toml `[features]`, lib.rs:15-46]. `WebSocket`, `Message`, `WebSocketConfig` gibt es immer.
- TLS-Helfer `client_tls`, `client_tls_with_config`, `Connector` nur mit `handshake` UND (`native-tls` ODER `__rustls-tls`) [lib.rs:28-29, 47-48]. `__rustls-tls = ["rustls", "rustls-pki-types"]` ist intern (Doppelunterstrich), öffentlich gedacht sind `rustls-tls-webpki-roots = ["__rustls-tls", "webpki-roots"]` und `rustls-tls-native-roots`. tungstenites rustls-Abhängigkeit: `rustls = { version = "0.23.0", features = ["std"], default-features = false }`, also ohne Provider [Cargo.toml].
- `url`-Feature nur für `IntoClientRequest for url::Url`; `&str`, `String`, `Uri`, `Request<()>` gehen ohne [client.rs:208-260].
- Server-Seite: Für TLS auf der Serverseite reicht `handshake`; das TLS macht der Aufrufer (`accept` nimmt jeden `Read + Write`, die Doku nennt `rustls::Stream`) [server.rs:14-22, 30-35].

## Server: accept, accept_hdr, Callback
```rust
pub fn accept<S: Read + Write>(stream: S) -> Result<WebSocket<S>, HandshakeError<ServerHandshake<S, NoCallback>>>                                   // server.rs:36
pub fn accept_with_config<S: Read + Write>(stream: S, config: Option<WebSocketConfig>) -> Result<WebSocket<S>, HandshakeError<ServerHandshake<S, NoCallback>>>   // :23
pub fn accept_hdr<S: Read + Write, C: Callback>(stream: S, callback: C) -> Result<WebSocket<S>, HandshakeError<ServerHandshake<S, C>>>               // :63
pub fn accept_hdr_with_config<S: Read + Write, C: Callback>(stream: S, callback: C, config: Option<WebSocketConfig>) -> Result<WebSocket<S>, HandshakeError<ServerHandshake<S, C>>>   // :50
// handshake::server (handshake/server.rs:27-33, 155-177)
type Request = http::Request<()>; type Response = http::Response<()>; type ErrorResponse = http::Response<Option<String>>;
trait Callback: Sized { fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse>; }
impl<F: FnOnce(&Request, Response) -> Result<Response, ErrorResponse>> Callback for F
```
- `request` enthält URI inkl. Query und alle Header (z. B. `Authorization`, `Origin`, `Sec-WebSocket-Protocol`); `response` ist die vorbereitete 101-Antwort, Header lassen sich ergänzen. `Err(ErrorResponse)` lehnt ab: die Antwort wird an den Client geschrieben (Statuscode muss kein Erfolg sein, sonst `ProtocolError::CustomResponseSuccessful`), danach liefert `accept_hdr*` `Err(HandshakeError::Failure(Error::Http(response)))` [handshake/server.rs:155-164, 233-290].
- Geprüft wird vorab: Methode GET, HTTP/1.1 oder höher, `Connection: Upgrade`, `Upgrade: websocket`, `Sec-WebSocket-Version: 13`, `Sec-WebSocket-Key` [handshake/server.rs:35-82].
- Grenzen des Handshake-Lesers: höchstens 65 536 Byte Kopfdaten, 512 Lesevorgänge und eine Mindestpaketgröße-Heuristik (`Error::AttackAttempt`) [handshake/machine.rs:164-195]; höchstens 124 Header [handshake/headers.rs:10]. Keine Zeitfrist.
- `WouldBlock` aus dem Stream wird zu `HandshakeError::Interrupted(MidHandshake)`, jede andere Fehlerart (auch Windows-`TimedOut`) zu `HandshakeError::Failure(Error::Io(..))` [handshake/mod.rs:34-46, util.rs:15-21]. `HandshakeError<Role>` implementiert `Debug`, `Display`, `std::error::Error` [handshake/mod.rs:60-87].
- Minimalbeispiel (Header prüfen, kleine Limits):
```rust
use tungstenite::{accept_hdr_with_config, handshake::server::{ErrorResponse, Request, Response}, protocol::WebSocketConfig};

let ws_cfg = WebSocketConfig { max_message_size: Some(64 * 1024), max_frame_size: Some(64 * 1024), ..WebSocketConfig::default() };
let check = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
    let bearer = req.headers().get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
    match bearer {
        Some(_token) /* hier HMAC prüfen */ => Ok(resp),
        None => Err(tungstenite::http::Response::builder().status(401).body(Some("unauthorized".to_owned())).expect("static response")),
    }
};
let mut ws = accept_hdr_with_config(tls_stream /* StreamOwned<ServerConnection, TcpStream> */, check, Some(ws_cfg))?;
```

## WebSocketConfig (nicht `#[non_exhaustive]`)
```rust
pub struct WebSocketConfig {                                    // protocol/mod.rs:34-72, Default :76-88
    pub max_send_queue: Option<usize>,       // #[deprecated], wirkt nicht
    pub write_buffer_size: usize,            // Default 128 KiB
    pub max_write_buffer_size: usize,        // Default usize::MAX; MUSS > write_buffer_size sein, sonst Panik (assert_valid :91-98)
    pub max_message_size: Option<usize>,     // Default Some(64 MiB), None = unbegrenzt
    pub max_frame_size: Option<usize>,       // Default Some(16 MiB), nur Payload ohne Frame-Header
    pub accept_unmasked_frames: bool,        // Default false (RFC 6455: Server muss unmaskierte Frames ablehnen)
}
```
- Muster `WebSocketConfig { max_message_size: Some(n), ..WebSocketConfig::default() }` benutzt tungstenite selbst [protocol/mod.rs:841]; so wird das veraltete Feld nicht genannt.
- Wirkung: die Frame-Länge wird aus dem Frame-Header geprüft, bevor der Payload gelesen oder reserviert wird [protocol/frame/mod.rs:157-175]; die Nachrichtenlänge beim Zusammensetzen fragmentierter Nachrichten [protocol/message.rs:115-130]; Verstoß: `Error::Capacity(CapacityError::MessageTooLong { size, max_size })` [error.rs:135-148]. Folgerung: für ein Signaling mit kleinen JSON-Nachrichten beide Limits auf wenige KiB setzen; die Defaults (64 MiB/16 MiB) sind für Dienste mit nicht vertrauenswürdigen Clients zu groß.

## WebSocket-API (blockierend)
```rust
WebSocket<S>::read(&mut self) -> Result<Message>              // protocol/mod.rs:199
WebSocket<S>::send(&mut self, message: Message) -> Result<()> // :205 = write + flush
WebSocket<S>::write(&mut self, message: Message) -> Result<()> // :244, puffert; Err(WriteBufferFull) bei max_write_buffer_size
WebSocket<S>::flush(&mut self) -> Result<()>                  // :252
WebSocket<S>::close(&mut self, code: Option<CloseFrame>) -> Result<()>   // :277
WebSocket<S>::get_ref(&self) -> &S / get_mut(&mut self) -> &mut S         // :148-154
enum Message { Text(String), Binary(Vec<u8>), Ping(Vec<u8>), Pong(Vec<u8>), Close(Option<CloseFrame<'static>>), Frame(Frame) }   // protocol/message.rs:160-177
```
- Ein empfangenes `Ping` liefert `read()` als `Message::Ping` zurück UND stellt automatisch ein `Pong` in die Warteschlange; es wird bei der nächsten `read`/`write`/`flush`-Runde gesendet [protocol/mod.rs:199-211, 605-611]. tungstenite sendet nie selbst Pings, es gibt keinen Zeitgeber im Modul [Suche nach `Instant`/`Duration` in protocol/mod.rs ohne Treffer]. Folgerung: Keepalive im Server selbst bauen (Lese-Timeout als Takt, bei `Error::Io(WouldBlock)`/`TimedOut` einen `Ping` senden, verpasste `Pong`s zählen). Ping/Pong-Payload unter 125 Byte [protocol/message.rs:165-172].
- Ende: `Message::Close` empfangen oder `close()` aufgerufen, danach weiter `read()`/`flush()`, bis `Error::ConnectionClosed` kommt; erst dann Stream schließen. Danach liefert jeder Aufruf `Error::AlreadyClosed` [protocol/mod.rs:186-198, 262-280, error.rs:26-40].
- `Error::Io(WouldBlock)` ist nach einem Lese-Timeout nicht fatal (Teildaten bleiben in den Puffern), andere I/O-Fehler sind fatal [protocol/mod.rs:225-243].
- Ein `WebSocket` hat einen einzigen Besitzer; ein TLS-Stream lässt sich nicht in Lese- und Schreibhälfte teilen (`StreamOwned` hat nur einen `ServerConnection`-Zustand). Folgerung: ein Thread je Verbindung, ausgehende Nachrichten über einen Kanal einreihen und im Lese-Takt (Timeout) abarbeiten.

## Client: client_tls_with_config und connect
```rust
pub fn client_tls_with_config<R, S>(request: R, stream: S, config: Option<WebSocketConfig>, connector: Option<Connector>)
    -> Result<(WebSocket<MaybeTlsStream<S>>, Response), HandshakeError<ClientHandshake<MaybeTlsStream<S>>>>
    where R: IntoClientRequest, S: Read + Write                                              // tls.rs:179-192 (Feature handshake + __rustls-tls|native-tls)
pub enum Connector { Plain, NativeTls(..) /*Feature native-tls*/, Rustls(std::sync::Arc<rustls::ClientConfig>) /*Feature __rustls-tls*/ }   // tls.rs:17-27, #[non_exhaustive]
pub fn connect<Req: IntoClientRequest>(request: Req) -> Result<(WebSocket<MaybeTlsStream<TcpStream>>, Response)>                                  // client.rs:125
pub fn connect_with_config<Req: IntoClientRequest>(request: Req, config: Option<WebSocketConfig>, max_redirects: u8) -> Result<(WebSocket<MaybeTlsStream<TcpStream>>, Response)>   // :44
pub fn client_with_config<Stream: Read + Write, Req: IntoClientRequest>(request: Req, stream: Stream, config: Option<WebSocketConfig>) -> Result<(WebSocket<Stream>, Response), HandshakeError<ClientHandshake<Stream>>>   // :159
type Response = http::Response<Option<Vec<u8>>>             // handshake/client.rs:30
```
- `connect*` nehmen keinen `Connector`; sie bauen immer den Standard-Connector (Prozess-Provider über `ClientConfig::builder()` plus webpki-Wurzeln nur mit Feature `rustls-tls-webpki-roots`) und verbinden ohne Timeout [client.rs:44-120, tls.rs:100-143]. Für Pinning, eigene CA oder Fristen also: `TcpStream` selbst aufbauen und `client_tls_with_config(url, tcp, Some(ws_cfg), Some(Connector::Rustls(Arc::new(client_config))))` aufrufen.
- Das Schema entscheidet über TLS: `ws` = Klartext, `wss` = TLS, alles andere `UrlError::UnsupportedUrlScheme` [client.rs:145-151]. Ohne TLS-Feature (bei `connect*`) oder mit `Connector::Plain` schlägt `wss` mit `UrlError::TlsFeatureNotEnabled` fehl [client.rs:55-58, tls.rs:146-155].
- Stolperfalle MaybeTlsStream: `#[non_exhaustive] enum MaybeTlsStream<S> { Plain(S), NativeTls(..), Rustls(StreamOwned<ClientConnection, S>) }` [stream.rs:62-72]; `match` braucht einen `_`-Arm. Timeouts lassen sich vor dem Aufruf auf dem `TcpStream` setzen (Socketoptionen bleiben erhalten) oder später über `ws.get_mut()` (`Plain(s) => s.set_read_timeout(..)`, `Rustls(s) => s.sock.set_read_timeout(..)`, `StreamOwned.sock` ist `pub`).
- Stolperfalle IPv6-Literal: `client_tls_with_config` nimmt den Servernamen aus `request.uri().host()` [tls.rs:192-195] und wandelt ihn mit `ServerName::try_from` [tls.rs:126-128]; `http::Uri::host()` liefert IPv6-Literale MIT Klammern (`"[::1]"`) [http-1.4.2 src/uri/authority.rs:141-143, 433-445, tests.rs:337-342], und `ServerName::try_from` kennt keine Klammern [rustls-pki-types server_name.rs:117-128]: `wss://[::1]:8443/` scheitert mit `TlsError::InvalidDnsName` (aus dem Quelltext abgeleitet, nicht ausgeführt). Umgehung: eigene TLS-Schicht (`ClientConnection::new(cfg, ServerName::IpAddress(ip.into()))` + `StreamOwned`) und `tungstenite::client_with_config(request, tls_stream, cfg)`; `ClientHandshake::start` prüft nur das Schema `ws`/`wss` [handshake/client.rs:54-55]. `connect()` selbst entfernt die Klammern nur für die Adressauflösung [client.rs:62].
- Minimalbeispiel mit Pin und Fristen (`PinnedCert` aus dem rustls-Abschnitt):
```rust
use std::{net::TcpStream, sync::Arc, time::Duration};
use tungstenite::{client_tls_with_config, Connector};

let provider = Arc::new(rustls::crypto::ring::default_provider());
let client_cfg = rustls::ClientConfig::builder_with_provider(provider.clone())
    .with_safe_default_protocol_versions()?
    .dangerous().with_custom_certificate_verifier(PinnedCert::new(pin_sha256, &provider))
    .with_no_client_auth();
let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(10))?;
tcp.set_nodelay(true)?; tcp.set_read_timeout(Some(Duration::from_secs(30)))?; tcp.set_write_timeout(Some(Duration::from_secs(30)))?;
let (mut ws, _resp) = client_tls_with_config("wss://signal.example.org:8443/ws", tcp, Some(ws_cfg), Some(Connector::Rustls(Arc::new(client_cfg))))?;
```

# iroh-relay 1.0.0 (im Repo vendored und gepatcht) und iroh 1.0.1 – Relay mit TLS, Zugangskontrolle, Client-Seite
Quelle: Repo `vendor/iroh-relay-1.0.0/` (Fundstellen `[R:Datei:Zeile]`, relativ dazu) · https://docs.rs/iroh-relay/1.0.0/iroh_relay/server/ · https://github.com/n0-computer/iroh/tree/6520cd6c9bacea15823c53a38267c40d754980fe/iroh-relay (Upstream 1.0.0) · https://docs.rs/iroh/1.0.1/iroh/endpoint/struct.Builder.html · https://github.com/n0-computer/iroh/tree/1ef462615e1ae4f88438eacfbeaf149aa5d067e1/iroh (Fundstellen `[I:Datei:Zeile]` = `https://static.crates.io/crates/iroh/iroh-1.0.1.crate`, Pfad `src/`) · https://github.com/n0-computer/tokio-rustls-acme (0.9.1, README) · https://letsencrypt.org/docs/challenge-types/ · Abgerufen: 2026-10-02

Stand im Repo: `share-server/Cargo.toml` `iroh-relay = { version = "1.0.0", default-features = false, features = ["server"] }`, `iroh-base = "1.0.0"`, `[patch.crates-io] iroh-relay = { path = "../vendor/iroh-relay-1.0.0" }`; Lock: iroh-relay 1.0.0 (Pfadquelle), iroh-base 1.0.0. `native/Cargo.toml:101` `iroh = { version = "1.0.0", default-features = true }`, kein `[patch]`; Lock: iroh, iroh-base, iroh-relay 1.0.1 aus crates.io (ungepatcht).

## Abweichungen der vendored Kopie von crates.io 1.0.0 (per `diff -r` gegen das Archiv geprüft)
- Geändert: `Cargo.toml` (nur `cdylib` aus `crate-type` entfernt), `src/main.rs`, `src/server.rs`, `src/server/{client,clients,http_server}.rs`. Neu: `src/server/{accept_rate_limits,connection_limits,connection_source,queue_budget}.rs`.
- API-Zusätze: `Limits` hat zusätzlich `accept_conn_limit_per_source`, `accept_conn_burst_per_source`, `max_concurrent_tcp_connections`, `max_concurrent_tcp_connections_per_source`; `accept_conn_limit` und `accept_conn_burst` wirken (Upstream: „not currently implemented“) [R:server.rs:586-624]. `AccessControl::ping_schedule` und `ClientPingSchedule` [R:server.rs:316-375]. `SpawnError::InvalidAcceptRateLimit` [R:server.rs:793]. Eine absolute Frist von 30 s ab TCP-Accept deckt TLS, WebSocket-Upgrade, Authentifizierung und `on_connect` ab [R:server/http_server.rs:75-79, 1146-1174].
- Unverändert gegenüber Upstream: `protos/handshake.rs`, `client.rs`, `client/tls.rs`, `tls.rs`, `http.rs`, `relay_map.rs`, `lib.rs`, `server/resolver.rs`, `server/streams.rs`, `tests/runtime_auth.rs`. Upstream 1.0.1 gegen 1.0.0: nur `warn_span!` zu `info_span!` in `http_server.rs`; iroh-base 1.0.1 gegen 1.0.0: `src/` identisch.

## Features
- `iroh-relay`: `default = ["metrics", "tls-ring"]`; `server = ["metrics", "tokio/signal", "dep:clap", "dep:dashmap", "dep:rcgen", "dep:reloadable-state", "dep:rustls-cert-file-reader", "dep:rustls-cert-reloadable-resolver", "dep:time", "dep:tokio-rustls-acme", "dep:tokio-websockets", "dep:simdutf8", "dep:sha1", "dep:toml", "dep:serde_json", "dep:tracing-subscriber", "noq/platform-verifier", "noq/runtime-tokio", "iroh-metrics/service", "tls-ring"]`; `tls-aws-lc-rs` ist die Alternative zu `tls-ring`; `test-utils`, `platform-verifier` optional [R:Cargo.toml `[features]`]. `iroh_relay::server` gibt es nur mit `server` [R:lib.rs:41-42]. ACME (tokio-rustls-acme) und `reloading_resolver` hängen an `server`, es gibt kein eigenes ACME-Feature; auch ohne ACME-Nutzung wird es mitgebaut.
- `iroh` 1.0.1: `default = ["metrics", "fast-apple-datapath", "portmapper", "tls-ring"]`; `tls-ring`/`tls-aws-lc-rs` wählen den Provider, `test-utils` schaltet `CaTlsConfig::insecure_skip_verify` frei [iroh-1.0.1 Cargo.toml `[features]`; R:tls.rs:93-99].

## Server starten: Typen und Signaturen
```rust
// alle Strukturen #[non_exhaustive]: außerhalb der Crate nur über Default/new + Feldzuweisung (so macht es R:main.rs:786-788). ServerConfig ist NICHT generisch (der Doc-Kommentar über `<(), ()>` ist veraltet, R:server.rs:113-114).
pub struct ServerConfig { pub relay: Option<RelayConfig>, pub quic: Option<QuicConfig>, pub metrics_addr: Option<SocketAddr> /* cfg(metrics) */ }   // R:server.rs:117-125, Default
pub struct RelayConfig { pub http_bind_addr: SocketAddr, pub tls: Option<TlsConfig>, pub limits: Limits, pub key_cache_capacity: Option<usize>, pub access: Arc<dyn DynAccessControl> }   // :133-153
RelayConfig::new(http_bind_addr: impl Into<SocketAddr>) -> RelayConfig        // :161; tls None, Limits::default(), access AllowAll
pub struct TlsConfig { pub https_bind_addr: SocketAddr, pub cert: CertConfig }  // :560-571
TlsConfig::new(https_bind_addr: impl Into<SocketAddr>, cert: CertConfig) -> TlsConfig   // :575
pub enum CertConfig {                                                           // :651-668 (#[non_exhaustive], Varianten aber konstruierbar)
    LetsEncrypt { acme_config: AcmeConfig, server_config_builder: rustls::ConfigBuilder<rustls::ServerConfig, rustls::server::WantsServerCert> },
    Manual { server_config: rustls::ServerConfig },
}
pub struct QuicConfig { pub bind_addr: SocketAddr, pub server_config: Option<rustls::ServerConfig> }   // :528-541; QuicConfig::new(bind_addr) :547; braucht TLS 1.3, erbt sonst RelayConfig::tls
Server::spawn(config: ServerConfig) -> Result<Server, SpawnError>   // async fn, :817
Server::shutdown(self).await / join(&mut self).await (beide async; shutdown -> Result<(), SupervisorError>) / http_addr() / https_addr() / quic_addr() -> Option<SocketAddr> / relay_service() -> Option<&RelayService> / metrics()   // :1016-1089
```
- Zusammenspiel von HTTP- und HTTPS-Port [R:server.rs:850-967, 1250-1324]:
  - `tls: Some(..)`: Der HTTPS-Listener auf `https_bind_addr` bedient `/relay` (WebSocket-Upgrade), `/`, `/index.html`, `/ping`, `/robots.txt`, `/healthz`. Zusätzlich bindet `Server::spawn` einen Klartext-Listener auf `http_bind_addr`, der nur `GET /generate_204` beantwortet (Captive-Portal-Probe), alles andere mit 404; es gibt keine Umleitung auf HTTPS.
  - `tls: None`: Alle Dienste inklusive `/relay` und `/generate_204` laufen im Klartext auf `http_bind_addr`.
  - `Server::https_addr()` ist nur mit TLS gesetzt [R:server.rs:1001-1002]. Bindefehler: `SpawnError::BindTlsListener` (HTTP-Listener neben TLS), `BindTcpListener { addr }` [R:server.rs:774-787].
  - TLS je Verbindung über `tokio_rustls::TlsAcceptor` (bei Let's Encrypt `tokio_rustls_acme::AcmeAcceptor`), danach nur HTTP/1.1 mit Upgrades (`hyper::server::conn::http1`) [R:server/http_server.rs:1208-1254].
- `Server` beendet sich beim Drop [R:server.rs:739]: in einer synchronen `main` die tokio-Laufzeit und den `Server` am Leben halten (`rt.block_on(Server::spawn(cfg))`). `key_cache_capacity: None` bedeutet 1 048 576 Einträge (`DEFAULT_KEY_CACHE_CAPACITY`); die Doku nennt rund 56 MB für 1 Mio. Einträge; für kleine Relays z. B. `Some(1024)` (so machen es die Tests) [R:defaults.rs:17-23, server.rs:854-856, 1365-1366].
- Minimalbeispiel (TLS mit festem Zertifikat, Zugriff nur für eigene Zugangsprüfung):
```rust
use std::{net::SocketAddr, sync::Arc};
use iroh_relay::server::{CertConfig, RelayConfig, Server, ServerConfig, TlsConfig};

let sc = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
    .with_safe_default_protocol_versions()?.with_no_client_auth().with_single_cert(chain, key)?;
let mut relay = RelayConfig::new("0.0.0.0:80".parse::<SocketAddr>()?);
relay.tls = Some(TlsConfig::new("0.0.0.0:443".parse::<SocketAddr>()?, CertConfig::Manual { server_config: sc }));
relay.key_cache_capacity = Some(1024);
relay.access = access.clone();                    // Arc<T> mit T: AccessControl, siehe unten
let mut cfg = ServerConfig::default();            // quic: None, metrics_addr: None
cfg.relay = Some(relay);
let server = Server::spawn(cfg).await?;           // am Leben halten
```
- Zertifikatsquellen aus der mitgelieferten Binärdatei [R:main.rs:623-694]:
```rust
// Manual: einmal geladen (R:main.rs:631-646)
CertConfig::Manual { server_config: builder.with_single_cert(certs, private_key)? }
// Reloading: Zertifikat und Schlüssel alle 24 h neu gelesen (R:main.rs:681-691)
iroh_relay::server::reloading_resolver(crypto_provider: &CryptoProvider, cert_path: PathBuf, key_path: PathBuf, interval: Duration) -> Result<Arc<dyn ResolvesServerCert>, AnyError>   // async, R:server/resolver.rs:37-47; DEFAULT_CERT_RELOAD_INTERVAL = 24 h (:19)
CertConfig::Manual { server_config: builder.with_cert_resolver(resolver) }
// Let's Encrypt (R:main.rs:647-680): builder ist ConfigBuilder<ServerConfig, WantsServerCert> OHNE Zertifikat, der ACME-Resolver wird in Server::spawn eingesetzt (R:server.rs:884-926)
CertConfig::LetsEncrypt { acme_config: AcmeConfig::letsencrypt(true).domains(vec!["relay.example.org".into()]).contact(vec!["mailto:admin@example.org".into()]).cache_path(dir), server_config_builder: builder }
```
  `AcmeConfig` [R:server.rs:672-733]: `new(directory_url: String)`, `letsencrypt(production: bool)`, `domains(Vec<String>)`, `contact(Vec<String>)` (E-Mail mit Präfix `mailto:`), `cache_path(PathBuf)` (ohne Aufruf wird nichts zwischengespeichert [R:server.rs:716-722]; Folgerung: bei jedem Start würde ein Zertifikat neu bestellt), `tls_config(CaTlsConfig)` (Vertrauensanker für den ACME-Server selbst, Standard eingebaute Mozilla-Wurzeln).
- ACME nutzt ausschließlich `tls-alpn-01` auf demselben Port wie der normale TLS-Verkehr (ALPN `acme-tls/1`) [tokio-rustls-acme-0.9.1 README, src/acme.rs:24]; Let's Encrypt validiert dieses Verfahren „via TLS on port 443“ und nicht für Wildcard-Namen [letsencrypt.org/docs/challenge-types]. Folgerung: `https_bind_addr` muss auf der öffentlichen Adresse Port 443 erreichbar sein (Weiterleitung 443 auf den Bindeport genügt); für Staging gilt `AcmeConfig::letsencrypt(false)`.
- Einbettung statt `Server::spawn`: `RelayService::new(handlers: Handlers, headers: HeaderMap, rate_limit: Option<ClientRateLimit>, key_cache: KeyCache, access: Arc<dyn DynAccessControl>, metrics: Arc<Metrics>)` [R:server/http_server.rs:1034-1052] und `RelayService::handle_connection(self, stream: tokio::net::TcpStream, tls_config: Option<http_server::TlsConfig>, establish_timeout: Duration)` [:1125-1133] bedienen eine selbst angenommene Verbindung samt TLS; `http_server::TlsConfig::new(Arc<rustls::ServerConfig>)` [:258]. Zusätzliche HTTP-Pfade gehen über `Handlers` (Map `(Method, &'static str)` auf `Box<dyn Fn(Request<Incoming>, ResponseBuilder) -> HyperResult<Response<BytesBody>> + Send + Sync>`, Zugriff per `Deref`/`DerefMut`) [:1257-1283]; `RelayServiceWithNotify` routet `GET /relay` zum Relay, alles andere über `Handlers`, sonst 404 [:858-906]. `Server::spawn` registriert nur feste Handler und bietet keinen Haken für eigene Pfade [R:server.rs:857-866]. Wer `RelayServiceWithNotify` direkt in einen eigenen hyper-Dienst einhängt, muss `MaybeTlsStream` in `TokioIo` übergeben, sonst `DowncastUpgrade` [:770-817]. Ungeprüft: ob sich das Signaling über einen solchen Handler auf demselben Port betreiben ließe.

## Zugangskontrolle (AccessControl, ClientRequest)
```rust
pub trait AccessControl: Debug + Send + Sync + 'static {                                  // R:server.rs:291-320
    fn on_connect(&self, request: &ClientRequest) -> impl Future<Output = Access> + Send;  // implementierbar als `async fn on_connect(&self, request: &ClientRequest) -> Access`
    fn on_disconnect(&self, endpoint_id: EndpointId, connection_id: ConnectionId) {}       // sync, darf nicht blockieren
    fn ping_schedule(&self, endpoint_id: EndpointId) -> ClientPingSchedule { .. }          // nur in der vendored Kopie
}
pub enum Access { Allow, Deny { reason: Option<String> } }                                 // :430-438
impl ClientRequest {                                                                       // :195-283
    pub fn endpoint_id(&self) -> EndpointId;  pub fn connection_id(&self) -> ConnectionId;  pub fn protocol_version(&self) -> ProtocolVersion;
    pub fn uri(&self) -> &http::Uri;  pub fn headers(&self) -> &http::HeaderMap;
    pub fn query_pairs(&self) -> impl Iterator<Item = (Cow<'_, str>, Cow<'_, str>)>;       // prozentdekodiert
    pub fn auth_token(&self) -> Option<String>;
}
// dyn-kompatibel: blanket impl<T: AccessControl> DynAccessControl for T (:382-414); Standard AllowAll (:420-426)
```
- Zeitpunkt: `on_connect` läuft NACH der Schlüssel-Authentifizierung des Relay-Handshakes (die Endpoint-ID in `ClientRequest::endpoint_id()` ist bewiesen) und VOR der Bestätigung an den Client; je Verbindungsversuch einmal. `on_disconnect` läuft genau einmal für jede zugelassene Verbindung, gleiche `ConnectionId` [R:server.rs:230-236, 301-310, protos/handshake.rs:489-503, server/http_server.rs:987-997]. Die 30-s-Aufbaufrist läuft weiter, auch während `on_connect` auf externe Systeme wartet.
- `Access::Deny { reason }`: der Client sieht `handshake::Error::ServerDeniedAuth { reason }`, Standardtext „not authorized“; der Text geht an den Client, keine Interna hineinschreiben [R:protos/handshake.rs:537-551].
- `auth_token()`: erster `Authorization`-Header mit Schema `Bearer` (ohne Rücksicht auf Groß-/Kleinschreibung); gibt es keinen, der Query-Parameter `token`; ist ein `Authorization`-Wert kein UTF-8, sofort `None` [R:server.rs:258-282]. Query-Tokens landen leicht in Protokollen; der Header ist vorzuziehen (Folgerung).
- Laufzeit-Steuerung: `Server::relay_service() -> Option<&RelayService>` [R:server.rs:1087], `RelayService::clients() -> &Clients` [R:server/http_server.rs:1063], `Clients::disconnect(&self, endpoint_id: EndpointId, connection_id: Option<ConnectionId>) -> bool` (`None` = alle Verbindungen des Endpoints, asynchrones Beenden) [R:server/clients.rs:217-242]. Vorlage für eine zur Laufzeit änderbare Zulassungsliste mit Entzug: `tests/runtime_auth.rs` (Zustand aus `on_connect`/`on_disconnect`, Entzug über `disconnect`).
- Mitgelieferte Modi [R:main.rs:158-285, README.md „Access control“]: `everyone`, `allowlist`/`denylist` (Endpoint-IDs), `http` (POST je Verbindung an einen Auth-Dienst, erwartet `200` mit Text `true`), `shared_token` (Liste fester Tokens, kein Entzug ohne Neustart). `SharedTokenAccess` vergleicht mit `Vec::contains`, also nicht in konstanter Zeit [R:main.rs:274-285]; eigene Prüfung über HMAC (`verify_slice`) ist vorzuziehen (Folgerung).
- Entwurf: nur beim Signaling angemeldete Endpoint-IDs zulassen. Der Signaling-Teil trägt die ID nach erfolgreicher Anmeldung ein (mit Ablaufzeit) und entfernt sie beim Abmelden; mit `Clients::disconnect` lässt sich eine bestehende Relay-Verbindung kappen. Die ID muss VOR dem ersten Relay-Verbinden eingetragen sein (Wettlauf zwischen Anmeldung und Relay-Verbindung beachten).
```rust
use std::{collections::HashSet, sync::{Arc, RwLock}};
use iroh_base::EndpointId;
use iroh_relay::server::{Access, AccessControl, ClientRequest};

#[derive(Debug, Default)]
pub struct SignedIn(RwLock<HashSet<EndpointId>>);        // EndpointId: Hash + Eq + Copy
impl SignedIn {
    pub fn add(&self, id: EndpointId)    { if let Ok(mut s) = self.0.write() { s.insert(id); } }
    pub fn remove(&self, id: &EndpointId) { if let Ok(mut s) = self.0.write() { s.remove(id); } }
}
impl AccessControl for SignedIn {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        match self.0.read() {                             // Guard wird nicht über ein .await gehalten
            Ok(s) if s.contains(&request.endpoint_id()) => Access::Allow,
            _ => Access::Deny { reason: None },
        }
    }
}
// let access = Arc::new(SignedIn::default());  relay.access = access.clone();   // Arc<SignedIn> -> Arc<dyn DynAccessControl>, wie in tests/runtime_auth.rs:134
```
- Alternative ohne gemeinsamen Zustand (Entwurf): Das Signaling stellt nach der Anmeldung ein HMAC-Token aus, gebunden an die Endpoint-ID und eine Ablaufzeit; `on_connect` prüft `request.auth_token()` gegen `request.endpoint_id()` (Format und API siehe hmac-Abschnitt). Der Client setzt das Token über `RelayConfig::with_auth_token` (siehe Client-Seite).

## Limits (vendored)
- `Limits` [R:server.rs:586-624]: `client_rx: Option<ClientRateLimit>` (Bytes/s je Client, `ClientRateLimit::new(NonZeroU32)`, `max_burst_bytes: Option<NonZeroU32>`), `accept_conn_limit: Option<f64>` und `accept_conn_burst: Option<usize>` (global, Token-Bucket direkt nach `accept`, Burst-Standard = Rate aufgerundet, mindestens 1), `accept_conn_limit_per_source`/`accept_conn_burst_per_source` (je IPv4-Adresse, IPv4-mapped IPv6 als IPv4, natives IPv6 je /64; höchstens 4096 Buckets im LRU), `max_concurrent_tcp_connections` und `max_concurrent_tcp_connections_per_source` (zählen ab `accept` bis zum Ende der Verbindung, auch durch TLS/HTTP/Authentifizierung). Hinter einem TCP-Reverse-Proxy zählt die Proxy-Adresse als eine Quelle.
- Fristen im Relay [R:defaults.rs:49-51, server/http_server.rs:79]: Aufbau 30 s, Schreibzeitlimit 2 s (`SERVER_WRITE_TIMEOUT`), Server-Pings alle 15 s (`PING_INTERVAL`) mit RTT-abhängiger Antwortfrist [R:protos/relay.rs:36, server.rs:324-351].

## Relay-Handshake: so authentifiziert sich ein Client gegenüber dem Relay (Vorlage für das Challenge-Response-Muster)
Quelle: `R:protos/handshake.rs` (unverändert gegenüber Upstream 1.0.0) und `R:client.rs`.
- Ziel laut Modulkommentar: (1) dem Relay die EndpointId mitteilen, (2) beweisen, dass der Client den Secret Key zur EndpointId besitzt, (3) optional Zugriff prüfen [handshake.rs:1-7]. Zwei Wege:
  1. Challenge: Nach dem WebSocket-Upgrade sendet der Server `ServerChallenge { challenge: [u8; 16] }` (16 Byte, frisch aus einem kryptografisch sicheren Zufallsgenerator, `rand::rng()`, je Verbindung neu) [handshake.rs:77-82, 200-207, 452-453]. Der Client antwortet mit `ClientAuth { public_key, signature: [u8; 64] }`. Signiert wird NICHT die rohe Challenge, sondern `blake3::derive_key("iroh-relay handshake v1 challenge signature", &challenge)` (32 Byte) [handshake.rs:52, 210-220, 225-230]. Begründung im Quelltext: Ein bösartiger Relay könnte sonst beliebige 16 Byte signieren lassen; die Ableitung mit Domänentrennung verhindert das [handshake.rs:211-219]. Der Server prüft mit `PublicKey::verify` (strikt, siehe iroh-base-Abschnitt); bei Fehlschlag `ServerDeniesAuth { reason: "signature invalid" }` [handshake.rs:234-247, 458-466]. Serialisierung: postcard mit Frame-Typ-Präfix [handshake.rs:554-604].
  2. TLS-Exporter (spart eine Runde, ähnelt RFC 9729 „Concealed HTTP Authentication“ und nutzt RFC 5705): Der Client exportiert 32 Byte mit Label `b"iroh-relay handshake v1"` und Kontext = eigener öffentlicher Schlüssel (`[0u8; 32]` als Ausgabepuffer), signiert die ersten 16 Byte und sendet die letzten 16 Byte als `key_material_suffix` mit; so erkennt der Server TLS-Zwischenstellen, die den Exporter nicht durchreichen. Übertragung als Header `x-iroh-relay-client-auth-v1` (base64url ohne Padding, postcard) [handshake.rs:56, 61-73, 254-278, 288-330; http.rs:16]. Stimmt der Exporter nicht überein, fällt der Server stillschweigend auf Weg 1 zurück [handshake.rs:441-450]. Bei Klartext-Verbindungen liefert der Exporter `None`, es gilt immer Weg 1 [R:server/streams.rs:229-245].
- Replay-Schutz: Die Challenge ist je Verbindung zufällig und wird nicht wiederverwendet; ein mitgeschnittenes `ClientAuth` taugt auf keiner anderen Verbindung. Zeitstempel gibt es nicht; die Zeitgrenze ist die 30-s-Aufbaufrist. Die Challenge-Variante enthält keine Server-Identität im signierten Wert (Folgerung aus handshake.rs:210-220): ein bösartiger Server könnte eine fremde Challenge weiterreichen (Weiterleitungsangriff); die Exporter-Variante bindet an die TLS-Sitzung.
- Autorisierung ist davon getrennt: erst nach bewiesener Identität ruft der Server `AccessControl::on_connect` und schickt dann `ServerConfirmsAuth` oder `ServerDeniesAuth` [handshake.rs:489-551].

## Client-Seite: iroh 1.0.1 und iroh-relay-Client
```rust
// iroh 1.0.1 [I:endpoint.rs]
Endpoint::builder(preset: impl Preset) -> Builder (:950);  Endpoint::bind(preset) -> Result<Endpoint, BindError> (:955)
Builder::relay_mode(self, relay_mode: RelayMode) -> Self (:557);  Builder::ca_tls_config(self, CaTlsConfig) -> Self (:713; `ca_roots_config` veraltet)
Builder::secret_key(self, SecretKey) -> Self (:524);  Builder::crypto_provider(self, Arc<rustls::crypto::CryptoProvider>) -> Self (:761)
Endpoint::secret_key(&self) -> &SecretKey (:1172);  Endpoint::id(&self) -> EndpointId (:1180)
Endpoint::insert_relay(&self, relay: RelayUrl, config: Arc<RelayConfig>) -> Option<Arc<RelayConfig>>  (async, :982);  remove_relay(&self, &RelayUrl) (:996)
pub enum RelayMode { Disabled, Default, Staging, Custom(RelayMap) }  (:1922-1934);  RelayMode::custom(impl IntoIterator<Item = RelayUrl>) -> RelayMode (:1960)
// iroh_relay::{RelayConfig, RelayMap} (in iroh re-exportiert, I:lib.rs:290)  [R:relay_map.rs]
RelayConfig::new(url: RelayUrl, quic: Option<RelayQuicConfig>) -> RelayConfig (:250);  RelayConfig::with_auth_token(self, token: impl Into<String>) -> RelayConfig (:266)
RelayMap::with_auth_token(self, token) (:154; setzt das Token an alle bisherigen Einträge);  impl From<RelayConfig> for RelayMap
iroh_relay::tls::CaTlsConfig::{embedded() (Standard), custom_roots(impl IntoIterator<Item = CertificateDer<'static>>), with_extra_roots(..), custom_server_cert_verifier(ServerCertVerifierBuilder), system() (Feature platform-verifier), insecure_skip_verify() (nur test-utils)}   // R:tls.rs:73-160
type ServerCertVerifierBuilder = Arc<dyn Fn(Arc<CryptoProvider>) -> io::Result<Arc<dyn ServerCertVerifier>> + Send + Sync + 'static>   // R:tls.rs:221-223
```
- `iroh::tls::CaTlsConfig` ist derselbe Typ [I:tls.rs:22-25]. Der Endpoint baut daraus beim `bind()` den rustls-`ClientConfig` für alle Nicht-iroh-TLS-Verbindungen (Relays, Pkarr, DNS-over-HTTPS); Standard `CaTlsConfig::default()` = eingebaute Mozilla-Wurzeln; ohne gesetzten Provider schlägt `bind()` mit `BindError::InvalidCryptoProvider` fehl, `presets::Minimal` setzt nur den ring-Provider [I:endpoint.rs:225-231, 259-264, endpoint/presets.rs]. Die TLS-Wurzeln sichern nur den Relay-Weg; Endpoint-zu-Endpoint-Verbindungen authentifiziert iroh selbst über die Schlüssel (Raw Public Keys) [R:tls.rs:11-17, I:tls.rs:1-6].
- RelayUrl-Schema entscheidet über TLS [R:client.rs:271-281, client/tls.rs:97-105, 365-375]: nur `http` und `ws` schalten TLS ab (`ws`-Verbindung ohne TLS), jedes andere Schema, insbesondere `https` und `wss`, benutzt TLS. Der Pfad wird auf `/relay` gesetzt. Standardports: 80 für `http`/`ws`, 443 für `https`/`wss`; bei anderen Schemata ist ein Port Pflicht (sonst `DialError::InvalidTargetPort`) [R:client/tls.rs:168, 251]. Servername für TLS = `url.host_str()`; Zertifikatsprüfung mit IP-Host braucht IP-SAN. `Url::host_str` liefert IPv6 mit Klammern [url-2.5.8 src/lib.rs:1130], `ServerName::try_from` kennt keine (Folgerung): `https://[::1]` ergibt vermutlich `ConnectError::InvalidTlsServername` (nicht ausgeführt).
- Auth-Token: `RelayConfig::with_auth_token` sendet `Authorization: Bearer TOKEN` im WebSocket-Upgrade (WASM: `?token=`); das Token muss ein gültiger HTTP-Headerwert sein, sonst `ConnectError::InvalidAuthToken` [R:relay_map.rs:259-268, client.rs:234-247, 320-326]. Ohne TLS (`http://`) läuft das Token im Klartext: Token und Endpoint-ID sind dann mitlesbar (Folgerung); mit Token immer `https`.
- `RelayConfig::from(RelayUrl)` und `RelayMode::custom([..])` setzen `quic: Some(RelayQuicConfig::default())` (UDP-Port 7842, QUIC-Adressermittlung); `RelayConfig::new(url, None)` schaltet das ab [R:relay_map.rs:232-247, 272-286, 304-310, defaults.rs:1-7]. Folgerung: für einen Relay ohne `QuicConfig` `None` setzen. Ungeprüft: Verhalten von iroh, wenn der QUIC-Port fehlt.
- Test aus dem Quelltext als Vorlage: `RelayConfig::new(relay_url, None).with_auth_token(TOKEN).into()` (`RelayMap`), `Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Custom(map)).ca_tls_config(CaTlsConfig::insecure_skip_verify()).bind().await?` [I:endpoint.rs:3985-4040]; ein falsches Token zeigt sich in `home_relay_status()` als Fehler „not authorized“.
- Pinning eines selbstsignierten Relay-Zertifikats über den Verifier-Baustein:
```rust
use std::{io, sync::Arc};
use iroh::{endpoint::presets, Endpoint, RelayConfig, RelayMap, RelayMode, RelayUrl, tls::CaTlsConfig};
use rustls::{client::danger::ServerCertVerifier, crypto::CryptoProvider};

let relay_url: RelayUrl = "https://relay.example.org".parse()?;
let map: RelayMap = RelayConfig::new(relay_url, None).with_auth_token(token).into();
let pin = CaTlsConfig::custom_server_cert_verifier(Arc::new(move |p: Arc<CryptoProvider>| -> io::Result<Arc<dyn ServerCertVerifier>> {
    Ok(PinnedCert::new(pin_sha256, &p))              // PinnedCert aus dem rustls-Abschnitt
}));
let ep = Endpoint::builder(presets::Minimal).secret_key(device_key).relay_mode(RelayMode::Custom(map)).ca_tls_config(pin).bind().await?;
```
- Token-Wechsel: `Endpoint::insert_relay(url, Arc<RelayConfig>)` ersetzt die Konfiguration; das Token wird beim Start des aktiven Relay-Aktors aus der `RelayMap` gelesen [I:socket/transports/relay/actor.rs:1236-1248]. Ungeprüft: ob eine bestehende Relay-Verbindung dadurch neu aufgebaut wird; für Tokens mit Ablaufzeit vorher testen.
- Der iroh-relay-Client direkt: `ClientBuilder::new(url: impl Into<RelayUrl>, secret_key: SecretKey, dns_resolver: DnsResolver)`, `.tls_client_config(rustls::ClientConfig)` (Pflicht, sonst `ConnectError::MissingCryptoProvider`), `.auth_token(impl Into<String>)`, `.proxy_url(Url)`, `.connect().await` [R:client.rs:166-211, 244-257]; `CaTlsConfig::client_config(&self, Arc<CryptoProvider>) -> io::Result<ClientConfig>` [R:tls.rs:204].

# iroh-base 1.0.1 und ed25519-dalek 3.0.0-rc.0 – Geräteschlüssel, Signaturen, Challenge-Response-Muster
Quelle: https://docs.rs/iroh-base/1.0.1/iroh_base/struct.PublicKey.html · https://github.com/n0-computer/iroh/tree/1ef462615e1ae4f88438eacfbeaf149aa5d067e1/iroh-base · https://docs.rs/ed25519-dalek/3.0.0-rc.0/ed25519_dalek/struct.VerifyingKey.html · https://github.com/dalek-cryptography/curve25519-dalek/tree/70e82aa85535649926c9e12ea4a36d5ea967c50d/ed25519-dalek · https://github.com/RustCrypto/signatures/tree/f2b087c0f38fa67b62ad1bc16a2ea62a80227fc9/ed25519 (ed25519 3.0.0) · https://www.rfc-editor.org/rfc/rfc9729.html · Abgerufen: 2026-10-02

Konvention: `[B:Datei:Zeile]` = `https://static.crates.io/crates/iroh-base/iroh-base-1.0.1.crate`, Pfad `src/` (1.0.0 identisch); `[D:…]` = ed25519-dalek-3.0.0-rc.0 `src/`; `[E:…]` = ed25519-3.0.0 `src/`. Stand im Repo: `share-server/Cargo.toml` `iroh-base = "1.0.0"` (Default-Feature `relay`; `PublicKey`/`SecretKey`/`Signature` brauchen Feature `key`, das `iroh-relay` mit `features = ["key", "relay"]` einschaltet); beide Locks ed25519-dalek 3.0.0-rc.0 (iroh-base pinnt `=3.0.0-rc.0`), ed25519 3.0.0, curve25519-dalek 5.0.0-rc.0. Folgerung: `iroh-base = { version = "1.0.0", features = ["key"] }` explizit eintragen.

## Schlüssel und Signaturen
```rust
pub type EndpointId = PublicKey;                                              // B:key.rs:70 – die Endpoint-ID IST der öffentliche Ed25519-Schlüssel
PublicKey::LENGTH: usize = 32;  PublicKey::as_bytes(&self) -> &[u8; 32] (:111);  Deref<Target = [u8; 32]>
PublicKey::from_bytes(bytes: &[u8; 32]) -> Result<PublicKey, KeyParsingError> (:122)       // prüft nur, dass der Punkt dekomprimierbar ist
PublicKey::verify(&self, message: &[u8], signature: &Signature) -> Result<(), SignatureError> (:134)
SecretKey::generate() -> SecretKey (:318);  SecretKey::from_bytes(&[u8; 32]) -> SecretKey (:337);  to_bytes(&self) -> [u8; 32] (:332)
SecretKey::public(&self) -> PublicKey (:299);  SecretKey::sign(&self, msg: &[u8]) -> Signature (:323)
Signature::LENGTH: usize = 64;  Signature::to_bytes(&self) -> [u8; 64] (:460);  Signature::from_bytes(bytes: &[u8; 64]) -> Signature (:465, unfehlbar);  TryFrom<&[u8]> for Signature (SignatureParsingError)
Endpoint::secret_key(&self) -> &SecretKey;  Endpoint::id(&self) -> EndpointId      // iroh 1.0.1 endpoint.rs:1172, 1180
```
- `PublicKey::verify` ruft `VerifyingKey::verify_strict` auf [B:key.rs:134-138]. `verify_strict` lehnt zusätzlich zur Skalar-Malleability (jede `verify_*`-Variante) Signaturen mit kleiner Ordnung in `R` und öffentliche Schlüssel kleiner Ordnung ab [D:verifying.rs:296-390, 359-375]. Das Gegenstück `Verifier::verify` (nicht strikt) wird von iroh nicht verwendet [D:verifying.rs:564-570].
- `VerifyingKey::from_bytes` prüft Punkte nach ZIP-215, nicht nach RFC 8032/NIST [D:verifying.rs:134-137]. Ungeprüft: ob nicht-kanonische Kodierungen desselben Punkts (ZIP-215 erlaubt sie) als eigene `EndpointId` gelten; `PublicKey` speichert die übergebenen 32 Byte [B:key.rs:20-22, 122-127]. Zulassungslisten daher über die 32 Byte der ID führen und nur IDs aufnehmen, die zuvor erfolgreich verifiziert wurden.
- Kodierung: `Display` für `PublicKey` = 64 Zeichen Hex klein [B:key.rs:221-225]; `FromStr` akzeptiert 64 Zeichen Hex (nur Kleinbuchstaben, `HEXLOWER`) oder Base32 ohne Padding (52 Zeichen, Groß-/Kleinschreibung egal) [B:key.rs:249-256, 475-494]; `to_z32`/`from_z32` für z-base-32 [:164-176]. serde: lesbare Formate (JSON) = Hex-String, binäre (postcard) = 32 Rohbytes [B:key.rs:78-103].
- `Signature` serialisiert über serde als Tupel aus 64 Einzel-Bytes (in JSON ein Array aus 64 Zahlen) [B:key.rs:378-390]; `Display` liefert Großbuchstaben-Hex (aus `ed25519::Signature`) [B:key.rs:434-437, E:lib.rs:447-451]. Folgerung: für JSON/Text-Protokolle `signature.to_bytes()` selbst in Hex oder base64url kodieren (`data-encoding` liegt im Lock) und auf der Gegenseite `Signature::from_bytes` nach Längenprüfung auf 64 Byte benutzen.
- Der Endpoint kennt seinen Schlüssel (`Endpoint::secret_key()`), eine Anmeldung kann also ohne zweite Schlüsselverwaltung mit dem Geräteschlüssel signieren.
- Signaturen sind deterministisch (Ed25519 nach RFC 8032, `SigningKey::sign` über `ed25519_dalek::Signer`) [B:key.rs:323-327]; derselbe Schlüssel signiert bei iroh auch andere Protokolle (Relay-Handshake), daher immer eine eigene Domänentrennung verwenden.

## Challenge-Response-Muster für die Anmeldung beim Server (Entwurf, Vorlage = iroh-relay-Handshake)
Quelle der Bausteine: `vendor/iroh-relay-1.0.0/src/protos/handshake.rs` (siehe iroh-relay-Abschnitt „Relay-Handshake“); alles in diesem Abschnitt außer den Verweisen ist Entwurf.
1. Server -> Client (nach dem WebSocket-Upgrade): `nonce` = 16 oder mehr Byte aus einem kryptografisch sicheren Zufallsgenerator, je Verbindung neu (Vorbild: 16 Byte, `rand::rng()`; share-server hat `rand 0.10.1`, `getrandom 0.4.3` und `blake3 1.8.5` nur transitiv im Lock, im eigenen Code braucht jede dieser Bibliotheken einen direkten Eintrag in `share-server/Cargo.toml`). Der Server merkt sich die Nonce nur für diese Verbindung und verwirft sie nach dem ersten Antwortversuch (Replay-Schutz).
2. Client -> Server: `endpoint_id` (64 Zeichen Hex) und `signature` (64 Byte) über `msg`.
3. `msg` = Domänentrennung + Bindung: nicht die rohe Nonce signieren, sondern einen abgeleiteten Wert, der Zweck, Version, Serveridentität und Client-ID enthält, z. B. `blake3::derive_key("<app> signaling login v1", nonce || server_id || endpoint_id)` mit festen Feldlängen (Vorbild: `blake3::derive_key(DOMAIN, &challenge)` liefert `[u8; 32]`, handshake.rs:210-220) oder SHA-256 über ein längenpräfigiertes Format. `server_id` = SHA-256 des Serverzertifikats (Pin) oder Hostname:Port; so lässt sich eine Challenge nicht an einen anderen Server weiterreichen (die Challenge-Variante von iroh-relay bindet keine Serveridentität). Stärkste Bindung (Option): TLS-Exporter des laufenden TLS-Kanals in `msg` aufnehmen; rustls: `conn.export_keying_material<T: AsMut<[u8]>>(&self, output: T, label: &[u8], context: Option<&[u8]>) -> Result<T, Error>` (RFC 5705; erst nach abgeschlossenem Handshake, `output` nicht leer) [rustls conn.rs:440-476]; geht nicht durch TLS-terminierende Proxys (iroh fällt dort auf die Challenge zurück).
4. Server prüft: `PublicKey::from_bytes` bzw. `EndpointId::from_str`, dann `endpoint_id.verify(&msg, &Signature::from_bytes(&sig64))` (strikt, Fehler -> Anmeldung verweigern, Verbindung beenden).
5. Zeitfenster: Das Muster mit Server-Nonce braucht keinen Zeitstempel; die Frist ist die Zeit zwischen Challenge und Antwort (Vorbild: 30 s Gesamtaufbaufrist im Relay) plus Lese-Timeouts. Nur ein zustandsloses Muster (Client signiert Zeitstempel + eigene Nonce, kein Server-Zustand) braucht ein Zeitfenster (Uhrabweichung berücksichtigen) und einen Replay-Cache der gesehenen Nonces für die Fensterdauer; das ist hier nicht nötig.
6. Authentifizierung ist nicht Autorisierung: erst nach bewiesener ID prüfen, ob die ID zugelassen ist (Einladung, Zugangstoken, Zulassungsliste), vgl. `AccessControl::on_connect` im Relay.
7. Fehlermeldungen einheitlich halten (iroh: „signature invalid“ / „not authorized“) und nicht zwischen „ID unbekannt“ und „Signatur falsch“ unterscheiden.

# hmac 0.12.1, hkdf 0.12.4, sha2 0.10.9 – HMAC-Zugriffstoken aus einem gemeinsamen Geheimnis
Quelle: https://docs.rs/hmac/0.12.1/hmac/ · https://docs.rs/digest/0.10.7/digest/trait.Mac.html · https://docs.rs/hkdf/0.12.4/hkdf/struct.Hkdf.html · https://docs.rs/sha2/0.10.9/sha2/ · https://github.com/RustCrypto/MACs/tree/46797e3b44973a30edb9d7f3a3ebb41810061d90/hmac · https://github.com/RustCrypto/traits/tree/344389411fd9718a0742435152e933a9e71461ee/digest · https://github.com/RustCrypto/KDFs/tree/1ac16e8b9d4ee7a67613c9396c6cc1327652eaba/hkdf · https://www.rfc-editor.org/rfc/rfc2104.html · https://www.rfc-editor.org/rfc/rfc5869.html · Abgerufen: 2026-10-02

Konvention: `[H:…]` = hmac-0.12.1 `src/`, `[M:…]` = digest-0.10.7 `src/`, `[K:…]` = hkdf-0.12.4 `src/`. Stand im Repo: `native/Cargo.toml` `hmac = "0.12"` (Z. 102), `sha2 = "0.10"` (Z. 70), `hkdf = "0.12.4"` (Z. 85); `native/Cargo.lock`: hmac 0.12.1 + 0.13.0, hkdf 0.12.4 + 0.13.0, sha2 0.10.9 + 0.11.0. `share-server/Cargo.lock`: weder hmac noch hkdf, nur sha2 0.11.0 (über ed25519-dalek).

## Versionspaarung (Stolperfalle)
- `hmac 0.12.1` und `sha2 0.10.9` hängen an `digest 0.10.7`; `hmac 0.13.0` und `sha2 0.11.0` an `digest 0.11.3`; `hkdf 0.12.4` an `hmac 0.12.1`, `hkdf 0.13.0` an `hmac 0.13.0` (Lock-Einträge in `native/Cargo.lock`). Folgerung: `Hmac<Sha256>` und `Hkdf<Sha256>` nur mit `sha2 0.10` kombinieren; das im share-server vorhandene `sha2 0.11.0` passt nicht zu `hmac 0.12`. Im share-server also `hmac = "0.12"`, `hkdf = "0.12"`, `sha2 = "0.10"` eintragen (sha2 0.10.9 und 0.11.0 können nebeneinander im Lock stehen).
- Features: hmac `std = ["digest/std"]`, `reset`; hkdf `std = ["hmac/std"]`; sha2 `default = ["std"]`, optional `asm`, `oid`, `force-soft` [jeweils Cargo.toml `[features]`]. Keine Zusatz-Features nötig.

## HMAC (hmac 0.12.1 + digest 0.10.7)
```rust
use hmac::{Hmac, Mac};            // hmac::Mac = digest::Mac, H:lib.rs:92; Hmac<D> H:lib.rs:102
type HmacSha256 = Hmac<sha2::Sha256>;
Mac::new_from_slice(key: &[u8]) -> Result<Self, InvalidLength>        // M:mac.rs:33; für HMAC nie Err ("HMAC can take key of any size", H:lib.rs:31)
Mac::update(&mut self, data: &[u8])                                   // :38
Mac::chain_update(self, data: impl AsRef<[u8]>) -> Self               // :41-43
Mac::finalize(self) -> CtOutput<Self>                                 // :46;  CtOutput::into_bytes(self) -> Output<T> (GenericArray<u8, U32>)
Mac::verify_slice(self, tag: &[u8]) -> Result<(), MacError>           // :73  KONSTANTE ZEIT (subtle ct_eq, M:mac.rs:168-179); Länge != 32 -> Err(MacError)
Mac::verify(self, tag: &Output<Self>) -> Result<(), MacError>         // :60
Mac::verify_truncated_left(self, tag: &[u8]) -> Result<(), MacError>  // :88; n == 0 oder n > 32 -> Err
```
- `CtOutput` vergleicht mit `==` in konstanter Zeit; `into_bytes()` liefert die Rohbytes und hebt diesen Schutz auf [M:mac.rs:244-294, H:lib.rs:34-39]. Tags niemals mit `==` auf Bytes vergleichen, immer `verify_slice`.
- Schlüssellänge: weniger als L Byte (hier 32) ist laut RFC 2104 §3 „strongly discouraged“; Kürzen von Tags nicht unter die Hälfte der Hashlänge und nicht unter 80 Bit (RFC 2104 §5). Folgerung: volle 32 Byte Tag verwenden.
- `MacError` ist ein Einheitstyp mit `Display` („MAC tag mismatch“) und trägt keine Details [M:mac.rs:296-310].

## HKDF (hkdf 0.12.4)
```rust
use hkdf::Hkdf;
Hkdf::<Sha256>::new(salt: Option<&[u8]>, ikm: &[u8]) -> Hkdf<Sha256>                  // K:lib.rs:199; Extract mit optionalem Salt
Hkdf::<Sha256>::from_prk(prk: &[u8]) -> Result<Hkdf<Sha256>, InvalidPrkLength>        // :206; prk mindestens HashLen (32) Byte
Hkdf::<Sha256>::extract(salt: Option<&[u8]>, ikm: &[u8]) -> (Output<Sha256>, Hkdf<Sha256>)   // :219
Hkdf::expand(&self, info: &[u8], okm: &mut [u8]) -> Result<(), InvalidLength>         // :269; Err bei okm.len() > 255 * 32 = 8160
Hkdf::expand_multi_info(&self, info_components: &[&[u8]], okm: &mut [u8]) -> Result<(), InvalidLength>   // :228
```
- RFC 5869: Salt ist optional und nicht geheim; ohne Salt wird er auf HashLen Nullbytes gesetzt (§2.2); `info` bindet die abgeleiteten Schlüssel an Anwendung und Kontext (§3.2); `L <= 255 * HashLen`, PRK mindestens HashLen (§2.3).
- Die Eingangsgeheimnisse sollten selbst gleichverteilt und lang sein (z. B. 32 Zufallsbytes); ein Passwort gehört nicht direkt in HKDF (Folgerung).

## Entwurf: Zugriffstoken aus gemeinsamem Geheimnis (ohne Zustand, nicht aus einer Quelle)
```rust
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;                     // sha2 0.10
type HmacSha256 = Hmac<Sha256>;

/// Teilschlüssel je Verwendungszweck: eigener `info`-Text, z. B. b"share-server relay-access v1".
fn derive_key(secret: &[u8], info: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, secret);
    let mut okm = [0u8; 32];
    hk.expand(info, &mut okm).expect("32 Byte <= 255 * 32");
    okm
}
/// Token = base64url( expiry_unix_secs_be[8] || tag[32] ), tag = HMAC(key, "relay-access v1" 0x00 endpoint_id[32] expiry_be[8])
fn mac_for(key: &[u8; 32], endpoint_id: &[u8; 32], expiry: u64) -> HmacSha256 {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC akzeptiert jede Schlüssellänge");
    mac.update(b"relay-access v1\0");
    mac.update(endpoint_id);                   // feste Länge: keine Mehrdeutigkeit zwischen den Feldern
    mac.update(&expiry.to_be_bytes());
    mac
}
fn issue(key: &[u8; 32], endpoint_id: &[u8; 32], expiry: u64) -> [u8; 40] {
    let tag = mac_for(key, endpoint_id, expiry).finalize().into_bytes();   // GenericArray<u8, U32>
    let mut out = [0u8; 40];
    out[..8].copy_from_slice(&expiry.to_be_bytes());
    out[8..].copy_from_slice(&tag);
    out
}
fn verify(key: &[u8; 32], endpoint_id: &[u8; 32], raw: &[u8], now_unix: u64) -> bool {
    let Ok(raw) = <&[u8; 40]>::try_from(raw) else { return false };
    let expiry = u64::from_be_bytes(raw[..8].try_into().expect("8 Byte"));
    expiry > now_unix && mac_for(key, endpoint_id, expiry).verify_slice(&raw[8..]).is_ok()   // konstante Zeit
}
```
- Bindung an `endpoint_id` verhindert, dass ein weitergegebenes Token mit fremdem Schlüssel funktioniert; im Relay kommt die ID aus `ClientRequest::endpoint_id()` (bewiesen durch den Handshake), das Token aus `ClientRequest::auth_token()` (base64url nach Roh-Bytes dekodieren, z. B. mit `data-encoding` `BASE64URL_NOPAD`). Im WebSocket-Signaling kommt das Token aus dem `Authorization`-Header im `Callback`.
- Widerruf: Tokens sind bis Ablauf gültig; kurze Laufzeit wählen und bei Entzug bestehende Relay-Verbindungen mit `Clients::disconnect` kappen. Ein neues Geheimnis (Rotation) macht alle Tokens ungültig; für sanfte Rotation zwei Schlüssel akzeptieren.
- Die Gegenseite muss dasselbe Geheimnis kennen: wer das Geheimnis hat, kann Tokens für beliebige IDs ausstellen. Soll das nur der Signaling-Server können, bleibt das Geheimnis bei ihm und der Relay (beide Serverseiten); Clients erhalten nur fertige Tokens.

## Zusammenfassung der Abhängigkeiten (Entwurf, jede Zeile folgt aus den Abschnitten oben)
```toml
# share-server/Cargo.toml – zusätzlich zu den vorhandenen Einträgen (iroh-relay mit [patch], tungstenite mit "handshake", lru, serde, tokio bleiben)
rustls   = { version = "0.23.41", default-features = false, features = ["ring", "std", "tls12"] }   # TLS-Server, ring statt aws-lc-rs
iroh-base = { version = "1.0.0", features = ["key"] }                                                 # PublicKey/SecretKey/Signature explizit
hmac     = "0.12"                                                                                      # nur mit sha2 0.10 (digest 0.10)
hkdf     = "0.12"
sha2     = "0.10"                                                                                      # HMAC/HKDF (der Zertifikat-Pin wird clientseitig geprüft); sha2 0.11 (Lock) bleibt daneben bestehen
blake3   = "1.8"                                                                                       # nur falls derive_key für die Domänentrennung benutzt wird
rand     = "0.10"                                                                                      # Server-Nonce (oder getrandom 0.4)
```
- `native/Cargo.toml` hat schon alles für Client-Pinning und Token-Anhängen: `rustls` (`ring`, `std`, `tls12`), `tungstenite` (`handshake`, `rustls-tls-webpki-roots` = `__rustls-tls`, damit `client_tls_with_config` und `Connector::Rustls`), `sha2 = "0.10"`, `hmac = "0.12"`, `hkdf = "0.12.4"`, `iroh = "1.0.0"` (`ca_tls_config`, `RelayConfig::with_auth_token` ohne Zusatz-Feature) [native/Cargo.toml:58, 70, 85, 101, 102, 109].

# Offene Punkte (nicht geprüft oder nur teilweise belegt)
- Alle Codebeispiele sind nicht kompiliert; sie sind gegen die zitierten Signaturen geprüft, ein Compiler-Lauf steht aus (Build nur über die Remote-CI).
- `tls12`: Ob im share-server-Build TLS 1.2 kompiliert ist, hängt von der Feature-Vereinigung aller Abhängigkeiten ab (iroh-relay schaltet es nicht ein, `noq`/`reqwest` ungeprüft). Wer TLS-1.2-Clients bedienen muss, setzt `features = ["tls12"]` auf rustls im share-server; TLS 1.3 ist immer vorhanden.
- Selbstsigniertes Zertifikat als einziger Vertrauensanker (`CaTlsConfig::custom_roots`, `RootCertStore::add`): nur der `CaUsedAsEndEntity`-Fall ist belegt (rustls-webpki), die Annahme eines `CA:FALSE`-Zertifikats nicht. Der Pin-Verifier umgeht die Frage.
- IPv6-Literale als Ziel: `wss://[::1]` (tungstenite) und `https://[::1]` (iroh-relay-Client) scheitern nach Quelltextlage am Servernamen mit Klammern; Umgehung beschrieben, nichts davon ausgeführt.
- iroh: Verhalten bei `RelayConfig` mit QUIC-Adressermittlung (`quic: Some`) gegen einen Relay ohne QUIC-Listener, und ob `Endpoint::insert_relay` mit neuem Token eine bestehende Relay-Verbindung neu aufbaut, sind ungeprüft.
- Das Challenge-Response-Muster (Bindung an Server-Identität, Zeitfenster-Variante), das Token-Format und der `SignedIn`-Zulassungsfilter sind Entwürfe ohne externe Quelle; belegt sind nur die zitierten Bausteine (iroh-relay-Handshake, RFC 9729, RFC 2104, RFC 5869).
- Windows: Verhalten von `TcpStream`-Timeouts (`TimedOut` statt `WouldBlock`) ist der Standardbibliotheks-Doku entnommen; die tungstenite-Zuordnung (nur `WouldBlock` wird zu `Interrupted`) ist im Quelltext belegt, ein Lauf unter Windows nicht.
- Die vendored Kopie weicht von crates.io 1.0.0 ab (siehe Abschnitt „Abweichungen“); bei einem Wechsel auf crates.io-Versionen entfallen die zusätzlichen `Limits`-Felder und `ping_schedule`.
