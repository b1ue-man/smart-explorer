//! Messung der Verschluesselungskosten (E30, Meilenstein M15): Einkern-Durchsatz der
//! AEAD-Verfahren des ring-Providers und Durchsatz einer Share-Uebertragung von 1 GiB
//! ueber Loopback. Nur fuer `#[cfg(test)]`; die Tests stehen in `s4_messung_task_tests.rs`.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::share::route_monitor;
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
/// Zeit, die die drei gezaehlten Share-Laeufe zusammen haben; Aufbau, Aufwaermlauf und Schliessen
/// brauchen den Rest der 60 s von `CASE_BUDGET` (wie `COUNTED_RUNS` der Messung M1).
const COUNTED_RUNS: Duration = Duration::from_secs(42);
/// Schwelle fuer den Anteil Share-Durchsatz zu AEAD-Durchsatz in Prozent (E30).
pub(super) const THRESHOLD_PERCENT: f64 = 10.0;
/// ALPN, den nur diese Messung verwendet.
const S4_ALPN: &[u8] = b"smart-explorer/s4-messung/1";
/// Frist der Routenzulassung je Verbindung (wie `ADMISSION` der Messungen M1 bis M9).
const ADMISSION: Duration = Duration::from_secs(10);

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

/// Ergebnis der Share-Messung.
pub(super) struct ShareResult {
    /// Median der drei gezaehlten Laeufe in MiB/s.
    pub(super) mib_s: f64,
    /// MiB eines gezaehlten Laufs (1024, ausser der Runner ist dafuer zu langsam).
    pub(super) counted_mib: u64,
}

/// Bytes eines gezaehlten Laufs auf einer Verbindung, die den Aufwaermlauf mit `warmup_mib_s`
/// uebertragen hat: 1 GiB, oder, wo drei Laeufe zu 1 GiB bei dieser Rate nicht in
/// `COUNTED_RUNS` passen, das groesste ganze Vielfache der Aufwaermgroesse, das passt
/// (mindestens eines). Ein langsamer Runner misst dann trotzdem und meldet die gezaehlte
/// Groesse, statt an `CASE_BUDGET` ohne Zeile zu enden (der Windows-Runner des ersten
/// Suite-Laufs uebertrug 3 GiB nicht in 60 s; der Grund war nicht die Rate, sondern die fehlende
/// Routenzulassung, `admit_routes`). Die Schwelle aendert das nicht: der Anteil bleibt
/// Share-Durchsatz geteilt durch AEAD-Durchsatz.
fn counted_bytes(warmup_mib_s: f64) -> u64 {
    let per_run = warmup_mib_s * COUNTED_RUNS.as_secs_f64() / 3.0 * 1024.0 * 1024.0;
    let steps = (per_run / WARMUP_BYTES as f64).floor().max(1.0) as u64;
    MEASURED_BYTES.min(steps.saturating_mul(WARMUP_BYTES))
}

/// Share-Durchsatz ueber Loopback in MiB/s: ein Aufwaermlauf (64 MiB, nicht gezaehlt), dann
/// drei Uebertragungen zu je 1 GiB (`counted_bytes` fuer einen zu langsamen Runner) auf einer
/// Verbindung; Ergebnis ist der Median. Die gesamte Messung endet nach `CASE_BUDGET`.
pub(super) fn share_loopback_mib_s() -> Result<ShareResult, String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("Laufzeit: {error}"))?;
    let stand = Stand::new();
    runtime.block_on(async {
        match tokio::time::timeout(CASE_BUDGET, share_inner(&stand)).await {
            Ok(result) => result.map_err(|error| format!("{error} [{}]", stand.bericht())),
            Err(_) => Err(format!(
                "Share-Messung ueberschreitet {} s [{}]",
                CASE_BUDGET.as_secs(),
                stand.bericht()
            )),
        }
    })
}

/// Stand der Share-Messung fuer den Fehlertext: die laufende Phase, die Dauer und die
/// empfangenen MiB je abgeschlossener Phase und die Bytezaehler. Ohne diese Angaben meldet
/// ein Zeitablauf nur, dass die 60 s nicht reichten, nicht wo die Messung stand.
struct Stand {
    started: Instant,
    sent: AtomicU64,
    received: AtomicU64,
    phases: Mutex<PhaseState>,
}

struct PhaseState {
    current: &'static str,
    since: Instant,
    received_then: u64,
    finished: Vec<String>,
}

impl Stand {
    fn new() -> Arc<Self> {
        let now = Instant::now();
        Arc::new(Self {
            started: now,
            sent: AtomicU64::new(0),
            received: AtomicU64::new(0),
            phases: Mutex::new(PhaseState {
                current: "endpunkte",
                since: now,
                received_then: 0,
                finished: Vec::new(),
            }),
        })
    }

    /// Schliesst die laufende Phase ab und beginnt `phase`.
    fn enter(&self, phase: &'static str) {
        let received = self.received.load(Ordering::Relaxed);
        let mut state = self.phases.lock().unwrap_or_else(PoisonError::into_inner);
        let line = format!(
            "{}: {:.2} s, {} MiB empfangen",
            state.current,
            state.since.elapsed().as_secs_f64(),
            (received - state.received_then) >> 20
        );
        state.finished.push(line);
        state.current = phase;
        state.since = Instant::now();
        state.received_then = received;
    }

    fn bericht(&self) -> String {
        let state = self.phases.lock().unwrap_or_else(PoisonError::into_inner);
        format!(
            "Phase {} seit {:.1} s; gesendet {} MiB, empfangen {} MiB, gesamt {:.1} s; \
             abgeschlossen: {}",
            state.current,
            state.since.elapsed().as_secs_f64(),
            self.sent.load(Ordering::Relaxed) >> 20,
            self.received.load(Ordering::Relaxed) >> 20,
            self.started.elapsed().as_secs_f64(),
            if state.finished.is_empty() {
                "keine".to_owned()
            } else {
                state.finished.join("; ")
            }
        )
    }
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
    share: &ShareResult,
) -> String {
    let anteil = anteil_prozent(share.mib_s, aead_mib_s);
    format!(
        "S4-MESSUNG plattform={plattform} verfahren={} aead_mib_s={aead_mib_s:.1} \
         share_mib_s={:.1} anteil_prozent={anteil:.1} gezaehlt_mib={}",
        aead.name(),
        share.mib_s,
        share.counted_mib
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

/// Endpunkt mit der Transportkonfiguration der Share-Verbindung, nur IPv4-Loopback, ohne
/// Relay und ohne Adresssuche (gebunden wie in den Messungen M1 bis M9).
async fn endpoint() -> Result<Endpoint, String> {
    let transport = crate::share::keepalive::iroh_transport_config_for_routes(true);
    Endpoint::builder(presets::Minimal)
        .alpns(vec![S4_ALPN.to_vec()])
        .clear_address_lookup()
        .relay_mode(RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .map_err(|error| format!("Bindadresse: {error:?}"))?
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

/// Laesst die Routenueberwachung beider Seiten die Verbindung zu. Die Transportkonfiguration
/// der Share-Verbindung schaltet die Nutzdaten-Policy ein (`application_path_policy`): bis ein
/// Pfad zugelassen ist, sendet QUIC keinen einzigen STREAM-Frame (`vendor/
/// SMART_EXPLORER_ROUTE_POLICY.md`: "enabled starts blocked"). Ohne diese Zulassung blieb
/// schon der Aufwaermlauf stehen, auf jedem Runner bis `CASE_BUDGET`; produktive Verbindungen
/// und die Messungen M1 bis M9 lassen sie ebenso zu.
async fn admit_routes(
    client: (&Endpoint, &Connection),
    server: (&Endpoint, &Connection),
) -> Result<(), String> {
    for (side, (endpoint, connection)) in [("Client", client), ("Server", server)] {
        route_monitor::start(
            connection,
            endpoint.id().to_string(),
            false,
            true,
            Vec::new(),
        )
        .map_err(|error| format!("Routemonitor ({side}): {error}"))?;
    }
    let deadline = Some(Instant::now() + ADMISSION);
    for (side, (_, connection)) in [("Client", client), ("Server", server)] {
        route_monitor::wait_application_route(connection, deadline)
            .await
            .map_err(|error| format!("Routezulassung ({side}): {error}"))?;
    }
    Ok(())
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
/// haelt den Strom offen, bis der Empfaenger alle Bytes hat; `stand` zaehlt mit.
async fn transfer(
    sender: &Connection,
    receiver: &Connection,
    total: u64,
    stand: &Stand,
) -> Result<(), String> {
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
            stand.sent.fetch_add(count as u64, Ordering::Relaxed);
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
            stand.received.fetch_add(count as u64, Ordering::Relaxed);
        }
        let _ = done_tx.send(());
        Ok::<(), io::Error>(())
    };
    let (sent, received) = tokio::join!(sending, receiving);
    sent.map_err(|error| format!("Senden: {error}"))?;
    received.map_err(|error| format!("Empfangen: {error}"))
}

async fn share_inner(stand: &Stand) -> Result<ShareResult, String> {
    let server = endpoint().await?;
    let client = endpoint().await?;
    stand.enter("verbindung");
    let (sender, receiver) = connected_pair(&server, &client).await?;
    stand.enter("routenzulassung");
    admit_routes((&client, &sender), (&server, &receiver)).await?;
    stand.enter("aufwaermlauf");
    let warmup = Instant::now();
    transfer(&sender, &receiver, WARMUP_BYTES, stand).await?;
    let counted = counted_bytes(mib_s(WARMUP_BYTES, warmup.elapsed()));
    let mut runs = [0.0; 3];
    for (index, run) in runs.iter_mut().enumerate() {
        stand.enter(["lauf1", "lauf2", "lauf3"][index]);
        let started = Instant::now();
        transfer(&sender, &receiver, counted, stand).await?;
        *run = mib_s(counted, started.elapsed());
    }
    stand.enter("schliessen");
    sender.close(0u32.into(), b"s4 messung beendet");
    receiver.close(0u32.into(), b"s4 messung beendet");
    client.close().await;
    server.close().await;
    Ok(ShareResult {
        mib_s: median(runs),
        counted_mib: counted >> 20,
    })
}
