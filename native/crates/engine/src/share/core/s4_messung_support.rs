//! Messung der Verschluesselungskosten (E30, Meilenstein M15): Einkern-Durchsatz der
//! AEAD-Verfahren des ring-Providers und Durchsatz einer Share-Uebertragung von 1 GiB
//! ueber Loopback. Nur fuer `#[cfg(test)]`; die Tests stehen in `s4_messung_task_tests.rs`.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::verbindungen_messung_support::{close_pair, Link, TransportPurpose};
use iroh::endpoint::Connection;
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
/// Routenzulassung, jetzt Teil von `Link::connect`). Die Schwelle aendert das nicht: der Anteil bleibt
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

/// Eine Uebertragung von `total` Bytes ueber einen neuen bidirektionalen Strom, wie die
/// Messung M1 (`verbindungen_messung_bulk.rs`, `one_stream`). Kein einseitiger Strom: die
/// Transportkonfiguration der Share-Verbindung erlaubt keinen (`IROH_MAX_CONCURRENT_UNI_STREAMS`
/// ist 0, `keepalive.rs`), `open_uni` wartete deshalb ohne Ende auf ein Stromlimit der Gegenseite
/// und der Aufwaermlauf sendete kein Byte. Der Sender beendet den Strom nach dem letzten Byte,
/// der Empfaenger liest bis zum Ende; `stand` zaehlt mit.
async fn transfer(
    sender: &Connection,
    receiver: &Connection,
    total: u64,
    stand: &Stand,
) -> Result<(), String> {
    let sending = async {
        let (mut send, _recv) = sender.open_bi().await.map_err(io::Error::other)?;
        let chunk = vec![0x5au8; BLOCK_BYTES];
        let mut sent = 0u64;
        while sent < total {
            let count = (total - sent).min(BLOCK_BYTES as u64) as usize;
            send.write_all(&chunk[..count])
                .await
                .map_err(io::Error::other)?;
            sent += count as u64;
            stand.sent.fetch_add(count as u64, Ordering::Relaxed);
        }
        send.finish().map_err(io::Error::other)?;
        Ok::<(), io::Error>(())
    };
    let receiving = async {
        let (_send, mut recv) = receiver.accept_bi().await.map_err(io::Error::other)?;
        let mut buffer = vec![0u8; BLOCK_BYTES];
        let mut received = 0u64;
        while let Some(count) = recv.read(&mut buffer).await.map_err(io::Error::other)? {
            received += count as u64;
            stand.received.fetch_add(count as u64, Ordering::Relaxed);
        }
        if received != total {
            return Err(io::Error::other("Strom endet vor dem Ende der Messung"));
        }
        Ok::<(), io::Error>(())
    };
    let (sent, received) = tokio::join!(sending, receiving);
    sent.map_err(|error| format!("Senden: {error}"))?;
    received.map_err(|error| format!("Empfangen: {error}"))
}

/// Aufbau wie M1: `Link` (zwei IPv4-Loopback-Endpunkte, Verbindung mit ALPN und Profil `Files`
/// des Share-Knotens, Routenzulassung beider Seiten), danach Aufwaermlauf und drei Laeufe auf
/// derselben Verbindung.
async fn share_inner(stand: &Stand) -> Result<ShareResult, String> {
    let link = Link::new().await?;
    stand.enter("verbindung");
    let pair = link
        .connect(crate::share::node::ALPN, TransportPurpose::Files)
        .await?;
    let (sender, receiver) = (&pair.0, &pair.1);
    stand.enter("aufwaermlauf");
    let warmup = Instant::now();
    transfer(sender, receiver, WARMUP_BYTES, stand).await?;
    let counted = counted_bytes(mib_s(WARMUP_BYTES, warmup.elapsed()));
    let mut runs = [0.0; 3];
    for (index, run) in runs.iter_mut().enumerate() {
        stand.enter(["lauf1", "lauf2", "lauf3"][index]);
        let started = Instant::now();
        transfer(sender, receiver, counted, stand).await?;
        *run = mib_s(counted, started.elapsed());
    }
    stand.enter("schliessen");
    close_pair(&pair);
    link.close().await;
    Ok(ShareResult {
        mib_s: median(runs),
        counted_mib: counted >> 20,
    })
}
