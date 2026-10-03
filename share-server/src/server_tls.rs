//! One validated, reloadable certificate for signaling and relay.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use rustls::crypto::CryptoProvider;
use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::ServerConfig;

type Stamp = (SystemTime, u64, SystemTime, u64);

#[derive(Debug)]
struct Certificate {
    provider: Arc<CryptoProvider>,
    cert: PathBuf,
    key: PathBuf,
    current: Mutex<Current>,
}

#[derive(Debug)]
struct Current {
    stamp: Option<Stamp>,
    key: Arc<CertifiedKey>,
}

pub(super) fn load(cert: &Path, key: &Path) -> Result<Arc<ServerConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let certified = read_key(&provider, cert, key)?;
    let resolver = Arc::new(Certificate {
        provider: provider.clone(), cert: cert.into(), key: key.into(),
        current: Mutex::new(Current { stamp: stamp(cert, key), key: Arc::new(certified) }),
    });
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions().map_err(|error| error.to_string())?
        .with_no_client_auth().with_cert_resolver(resolver);
    Ok(Arc::new(config))
}

fn read_key(provider: &CryptoProvider, cert: &Path, key: &Path) -> Result<CertifiedKey, String> {
    let chain = CertificateDer::pem_file_iter(cert)
        .map_err(|error| format!("cannot read certificate {}: {error}", cert.display()))?
        .collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
    let private = PrivateKeyDer::from_pem_file(key)
        .map_err(|error| format!("cannot read TLS key {}: {error}", key.display()))?;
    CertifiedKey::from_der(chain, private, provider)
        .map_err(|error| format!("invalid TLS certificate/key pair: {error}"))
}

fn stamp(cert: &Path, key: &Path) -> Option<Stamp> {
    let cert = std::fs::metadata(cert).ok()?;
    let key = std::fs::metadata(key).ok()?;
    Some((cert.modified().ok()?, cert.len(), key.modified().ok()?, key.len()))
}

impl ResolvesServerCert for Certificate {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let mut current = self.current.lock().unwrap_or_else(|error| error.into_inner());
        let changed = stamp(&self.cert, &self.key);
        if changed != current.stamp {
            current.stamp = changed;
            match read_key(&self.provider, &self.cert, &self.key) {
                Ok(key) => {
                    current.key = Arc::new(key);
                    eprintln!("se-share-server: TLS certificate reloaded");
                }
                Err(error) => eprintln!("se-share-server: {error}; keeping the previous certificate"),
            }
        }
        Some(current.key.clone())
    }
}
