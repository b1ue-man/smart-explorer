//! Messung der Verschluesselungskosten (E30, Meilenstein M15): Einkern-Durchsatz der
//! AEAD-Verfahren des ring-Providers und Durchsatz einer Share-Uebertragung von 1 GiB
//! ueber Loopback. Nur fuer `#[cfg(test)]`; die Tests stehen in `s4_messung_task_tests.rs`.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use iroh::endpoint::{presets, Connection};
use iroh::{Endpoint, EndpointAddr, RelayMode, TransportAddr};
use ring::aead::{
    Aad, Algorithm, LessSafeKey, Nonce, UnboundKey, AES_128_GCM, AES_256_GCM, CHACHA20_POLY1305,
};

/// Obergrenze je Messfall (S12, E30): Aufwaermlauf und drei gezaehlte Laeufe zusammen.
pub(super) const CASE_BUDGET: Duration = Duration::from_secs(60);
/// Jeder gezaehlte Lauf verarbeitet 1 GiB.
pub(super) const MEASURED_BYTES: u64 = 1 << 30;
/// Aufwaermlauf vor den drei gezaehlten Laeufen; wird nicht gezaehlt.
const WARMUP_BYTES: u64 = 64 << 20;
/// Blockgroesse der AEAD-Messung und Puffergroesse der Uebertragung (64 KiB).
const BLOCK_BYTES: usize = 64 * 1024;
/// Schwelle fuer den Anteil Share-Durchsatz zu AEAD-Durchsatz in Prozent (E30).
pub(super) const THRESHOLD_PERCENT: f64 = 10.0;
/// ALPN, den nur diese Messung verwendet.
const S4_ALPN: &[u8] = b"smart-explorer/s4-messung/1";

/// Die drei AEAD-Verfahren der TLS-1.3-Suiten des ring-Providers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Aead {
    Aes128Gcm,
    Aes256Gcm,
    ChaCha20Poly1305,
}

impl Aead {
    /// Name des Verfahrens in der Ausgabezeile.
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Aes128Gcm => "AES-128-GCM",
            Self::Aes256Gcm => "AES-256-GCM",
            Self::ChaCha20Poly1305 => "ChaCha20-Poly1305",
        }
    }

    fn algorithm(self) -> &'static Algorithm {
        match self {
            Self::Aes128Gcm => &AES_128_GCM,
            Self::Aes256Gcm => &AES_256_GCM,
            Self::ChaCha20Poly1305 => &CHACHA20_POLY1305,
        }
    }

    fn key_len(self) -> usize {
        match self {
            Self::Aes128Gcm => 16,
            Self::Aes256Gcm | Self::ChaCha20Poly1305 => 32,
        }
    }

    /// Verfahren einer TLS-1.3-Suite; andere Suiten gehoeren nicht zur Messung.
    pub(super) fn from_suite(suite: rustls::CipherSuite) -> Option<Self> {
        match suite {
            rustls::CipherSuite::TLS13_AES_128_GCM_SHA256 => Some(Self::Aes128Gcm),
            rustls::CipherSuite::TLS13_AES_256_GCM_SHA384 => Some(Self::Aes256Gcm),
            rustls::CipherSuite::TLS13_CHACHA20_POLY1305_SHA256 => Some(Self::ChaCha20Poly1305),
            _ => None,
        }
    }
}

/// Das Verfahren, das die Share-Verbindung aushandelt: die erste TLS-1.3-AEAD-Suite des
/// ring-Providers. Annahme (nicht aus den freigegebenen Quellen belegt): Der Server folgt der
/// Client-Reihenfolge, und Client wie Server verwenden denselben Suitenkatalog.
pub(super) fn negotiated_aead() -> Result<Aead, String> {
    rustls::crypto::ring::default_provider()
        .cipher_suites
        .iter()
        .find_map(|suite| Aead::from_suite(suite.suite()))
        .ok_or_else(|| "keine TLS-1.3-AEAD-Suite im ring-Provider".to_owned())
}

/// Verschluesselt `blocks` Bloecke zu je 64 KiB auf dem aufrufenden Thread und liefert die
/// Zahl der verschluesselten Bytes. Die Obergrenze prueft die Schleife alle 256 Bloecke.
fn seal_blocks(aead: Aead, blocks: u64, case_started: Instant) -> Result<u64, String> {
    let key_bytes = vec![0x5au8; aead.key_len()];
    let unbound = UnboundKey::new(aead.algorithm(), &key_bytes)
        .map_err(|_| "Schluessel ungueltig".to_owned())?;
    let key = LessSafeKey::new(unbound);
    let mut block = vec![0x3cu8; BLOCK_BYTES];
    let mut checksum = 0u8;
    for counter in 0..blocks {
        if counter % 256 == 0 && case_started.elapsed() > CASE_BUDGET {
            return Err(format!(
                "AEAD-Messung ueberschreitet {} s",
                CASE_BUDGET.as_secs()
            ));
        }
        let mut nonce = [0u8; 12];
        nonce[4..].copy_from_slice(&counter.to_be_bytes());
        let tag = key
            .seal_in_place_separate_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::empty(),
                &mut block,
            )
            .map_err(|_| "Verschluesselung fehlgeschlagen".to_owned())?;
        checksum ^= tag.as_ref()[0];
    }
    std::hint::black_box(checksum);
    Ok(blocks * BLOCK_BYTES as u64)
}

/// Einkern-Durchsatz in MiB/s: ein Aufwaermlauf (nicht gezaehlt), dann drei gezaehlte
/// Laeufe zu je 1 GiB; Ergebnis ist der Median der drei Laeufe.
pub(super) fn aead_mib_s(aead: Aead) -> Result<f64, String> {
    let case_started = Instant::now();
    seal_blocks(aead, WARMUP_BYTES / BLOCK_BYTES as u64, case_started)?;
    let blocks = MEASURED_BYTES / BLOCK_BYTES as u64;
    let mut runs = [0.0; 3];
    for run in &mut runs {
        let started = Instant::now();
        let bytes = seal_blocks(aead, blocks, case_started)?;
        *run = mib_s(bytes, started.elapsed());
    }
    Ok(median(runs))
}

/// Share-Durchsatz ueber Loopback in MiB/s: ein Aufwaermlauf (64 MiB, nicht gezaehlt), dann
/// drei Uebertragungen zu je 1 GiB auf einer Verbindung; Ergebnis ist der Median. Die
/// gesamte Messung endet nach `CASE_BUDGET`.
pub(super) fn share_loopback_mib_s() -> Result<f64, String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("Laufzeit: {error}"))?;
    runtime.block_on(async {
        match tokio::time::timeout(CASE_BUDGET, share_inner()).await {
            Ok(result) => result,
            Err(_) => Err(format!(
                "Share-Messung ueberschreitet {} s",
                CASE_BUDGET.as_secs()
            )),
        }
    })
}

/// Kennung der Plattform fuer die Ausgabe, z. B. `linux-x86_64`.
pub(super) fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Anteil in Prozent: Share-Durchsatz geteilt durch AEAD-Durchsatz, mal 100.
pub(super) fn anteil_prozent(share_mib_s: f64, aead_mib_s: f64) -> f64 {
    share_mib_s / aead_mib_s * 100.0
}

/// Ausgabezeile einer Einzelmessung eines AEAD-Verfahrens.
pub(super) fn aead_zeile(aead: Aead, aead_mib_s: f64) -> String {
    format!(
        "S4-AEAD verfahren={} aead_mib_s={aead_mib_s:.1}",
        aead.name()
    )
}

/// Ausgabezeile S4-MESSUNG mit den Feldern aus der Auftragsdatei (Feld 2).
pub(super) fn messung_zeile(
    plattform: &str,
    aead: Aead,
    aead_mib_s: f64,
    share_mib_s: f64,
) -> String {
    let anteil = anteil_prozent(share_mib_s, aead_mib_s);
    format!(
        "S4-MESSUNG plattform={plattform} verfahren={} aead_mib_s={aead_mib_s:.1} \
         share_mib_s={share_mib_s:.1} anteil_prozent={anteil:.1}",
        aead.name()
    )
}

/// Durchsatz in MiB/s aus Bytezahl und Dauer.
fn mib_s(bytes: u64, elapsed: Duration) -> f64 {
    bytes as f64 / (1024.0 * 1024.0) / elapsed.as_secs_f64().max(f64::MIN_POSITIVE)
}

/// Median von drei Werten.
fn median(mut values: [f64; 3]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[1]
}

/// Endpunkt mit der Transportkonfiguration der Share-Verbindung, nur Loopback, ohne Relay
/// und ohne Adresssuche.
async fn endpoint() -> Result<Endpoint, String> {
    let transport = crate::share::keepalive::iroh_transport_config_for_routes(true);
    Endpoint::builder(presets::Minimal)
        .alpns(vec![S4_ALPN.to_vec()])
        .clear_address_lookup()
        .relay_mode(RelayMode::Disabled)
        .transport_config(transport)
        .bind()
        .await
        .map_err(|error| format!("Endpunkt: {error:?}"))
}

/// Verbindet `client` mit `server` ueber dessen IPv4-Loopback-Port und liefert beide Seiten
/// der Verbindung, zuerst die des Clients.
async fn connected_pair(
    server: &Endpoint,
    client: &Endpoint,
) -> Result<(Connection, Connection), String> {
    let port = server
        .bound_sockets()
        .into_iter()
        .find_map(|socket| match socket {
            SocketAddr::V4(addr) => Some(addr.port()),
            SocketAddr::V6(_) => None,
        })
        .ok_or_else(|| "Server ohne IPv4-Socket".to_owned())?;
    let target = EndpointAddr::from_parts(
        server.id(),
        vec![TransportAddr::Ip(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
        ))],
    );
    let (dialled, accepted) = tokio::join!(client.connect(target, S4_ALPN), accept_one(server));
    let client_side = dialled.map_err(|error| format!("Verbindung: {error:?}"))?;
    Ok((client_side, accepted?))
}

async fn accept_one(server: &Endpoint) -> Result<Connection, String> {
    let incoming = server
        .accept()
        .await
        .ok_or_else(|| "Server ohne eingehende Verbindung".to_owned())?;
    incoming
        .await
        .map_err(|error| format!("Annahme: {error:?}"))
}

/// Eine Uebertragung von `total` Bytes ueber einen neuen einseitigen Strom. Der Sender
/// haelt den Strom offen, bis der Empfaenger alle Bytes hat.
async fn transfer(sender: &Connection, receiver: &Connection, total: u64) -> Result<(), String> {
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
    let sending = async {
        let mut stream = sender.open_uni().await.map_err(io::Error::other)?;
        let chunk = vec![0x5au8; BLOCK_BYTES];
        let mut sent = 0u64;
        while sent < total {
            let count = (total - sent).min(BLOCK_BYTES as u64) as usize;
            stream
                .write_all(&chunk[..count])
                .await
                .map_err(io::Error::other)?;
            sent += count as u64;
        }
        let _ = done_rx.await;
        Ok::<(), io::Error>(())
    };
    let receiving = async {
        let mut stream = receiver.accept_uni().await.map_err(io::Error::other)?;
        let mut buffer = vec![0u8; BLOCK_BYTES];
        let mut received = 0u64;
        while received < total {
            let count = stream
                .read(&mut buffer)
                .await
                .map_err(io::Error::other)?
                .ok_or_else(|| io::Error::other("Strom endet vor dem Ende der Messung"))?;
            received += count as u64;
        }
        let _ = done_tx.send(());
        Ok::<(), io::Error>(())
    };
    let (sent, received) = tokio::join!(sending, receiving);
    sent.map_err(|error| format!("Senden: {error}"))?;
    received.map_err(|error| format!("Empfangen: {error}"))
}

async fn share_inner() -> Result<f64, String> {
    let server = endpoint().await?;
    let client = endpoint().await?;
    let (sender, receiver) = connected_pair(&server, &client).await?;
    transfer(&sender, &receiver, WARMUP_BYTES).await?;
    let mut runs = [0.0; 3];
    for run in &mut runs {
        let started = Instant::now();
        transfer(&sender, &receiver, MEASURED_BYTES).await?;
        *run = mib_s(MEASURED_BYTES, started.elapsed());
    }
    sender.close(0u32.into(), b"s4 messung beendet");
    receiver.close(0u32.into(), b"s4 messung beendet");
    client.close().await;
    server.close().await;
    Ok(median(runs))
}
