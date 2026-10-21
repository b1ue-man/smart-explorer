//! The decisions behind extra SSH connections and the choice of a pool
//! channel, kept free of any network so they are testable: who may open an
//! extra connection (security answer E13-S15), when the next one is worth
//! opening (the 10 % rule of the flow control, U50), the 15-minute block per
//! host, port and user, the one unauthenticated connection at a time per
//! host and port, which channel a transfer takes (S13: at most
//! `min(open-handles, 64)` handles each) and the text shown to the person.
use super::config::{PasswordSource, SftpAuth, SftpLoginOrigin};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// How long extra connections to a host stay off after one failed (E13-S15
/// (iii)); the same 15 minutes as FTP ranges (S12).
pub(super) const BLOCK_FOR: Duration = Duration::from_secs(15 * 60);
/// The throughput must have grown by this factor (+10 %, the flow control's
/// own step, U50) for another connection to be worth it.
pub(super) const GROWTH_FACTOR: f64 = 1.10;
/// Length of one throughput window.
pub(super) const WINDOW: Duration = Duration::from_secs(1);
/// A new connection needs this long before its effect is judged: log in and
/// let the transfer hand it ranges.
pub(super) const SETTLE: Duration = Duration::from_secs(2);

/// How an extra connection logs in.
pub(super) enum ExtraLogin {
    /// The sealed store's password, read fresh for every login.
    Password(PasswordSource),
    /// The decrypted key of the main login (with its certificate next to
    /// `path`, as the main login tries it).
    Key {
        path: String,
        key: Arc<russh::keys::PrivateKey>,
    },
}

/// The login of extra connections, if the main login allows one (E13-S15
/// (i)): only a password or a key file whose credentials came from the
/// sealed store without input. The SSH agent never does (it may ask for a
/// confirmation or a touch at every signature).
pub(super) fn plan(auth: &SftpAuth, origin: SftpLoginOrigin) -> Option<ExtraLogin> {
    match (auth, origin) {
        (SftpAuth::Password(_), SftpLoginOrigin::StoredPassword(source)) => {
            Some(ExtraLogin::Password(source))
        }
        (SftpAuth::Key { path, passphrase }, SftpLoginOrigin::StoredKey) => {
            let key = russh::keys::load_secret_key(path, passphrase.as_deref()).ok()?;
            Some(ExtraLogin::Key {
                path: path.clone(),
                key: Arc::new(key),
            })
        }
        _ => None,
    }
}

/// `user@host:port`, the unit of the block (E13-S15 (iii)).
pub(super) fn account_key(host: &str, port: u16, user: &str) -> String {
    format!("{user}@{}:{port}", host.to_ascii_lowercase())
}

/// `host:port`, the unit of the one login at a time (`MaxStartups`).
pub(super) fn server_key(host: &str, port: u16) -> String {
    format!("{}:{port}", host.to_ascii_lowercase())
}

/// What the throughput window says about another connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Hold,
    /// Open one more connection now.
    Open,
    /// The last one did not raise the throughput: no more.
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// Ready to open one when the transfer asks for more streams.
    Ready,
    /// A connection is being set up; `baseline` is the rate before it.
    Opening(f64),
    /// It is up; its effect is judged by the first window starting at `from`.
    Settling {
        baseline: f64,
        from: Instant,
    },
    Stopped,
}

/// Opens connections one at a time, only while the last one raised the
/// measured throughput by at least 10 %.
pub(super) struct Growth {
    phase: Phase,
}

impl Growth {
    pub(super) fn new() -> Self {
        Self {
            phase: Phase::Ready,
        }
    }

    /// One finished window that began at `started` and moved `rate` bytes per
    /// second. `wants`: the transfer has more streams than the connections
    /// carry, nothing is being set up and the host is not blocked.
    pub(super) fn window(&mut self, started: Instant, rate: f64, wants: bool) -> Step {
        match self.phase {
            Phase::Ready => {
                if wants && rate > 0.0 {
                    self.phase = Phase::Opening(rate);
                    Step::Open
                } else {
                    Step::Hold
                }
            }
            Phase::Opening(_) | Phase::Stopped => Step::Hold,
            Phase::Settling { baseline, from } => {
                if started < from {
                    return Step::Hold;
                }
                if rate >= baseline * GROWTH_FACTOR {
                    self.phase = Phase::Ready;
                    self.window(started, rate, wants)
                } else {
                    self.phase = Phase::Stopped;
                    Step::Stop
                }
            }
        }
    }

    /// The connection is up at `now`.
    pub(super) fn installed(&mut self, now: Instant) {
        if let Phase::Opening(baseline) = self.phase {
            self.phase = Phase::Settling {
                baseline,
                from: now + SETTLE,
            };
        }
    }

    /// Setting it up failed: no more attempts (the host is blocked).
    pub(super) fn failed(&mut self) {
        self.phase = Phase::Stopped;
    }

    /// The attempt did not start (blocked, another login running, no
    /// memory): it may be tried again.
    pub(super) fn cancelled(&mut self) {
        if matches!(self.phase, Phase::Opening(_)) {
            self.phase = Phase::Ready;
        }
    }

    /// A window without a transfer: the next job starts afresh.
    pub(super) fn idle(&mut self) {
        if !matches!(self.phase, Phase::Opening(_)) {
            self.phase = Phase::Ready;
        }
    }

    #[cfg(test)]
    pub(super) fn stopped(&self) -> bool {
        self.phase == Phase::Stopped
    }
}

/// Hosts with a block, by `account_key`.
#[derive(Default)]
pub(super) struct Blocks {
    until: HashMap<String, (Instant, String)>,
}

impl Blocks {
    pub(super) fn block(&mut self, now: Instant, key: &str, reason: &str) {
        self.until
            .insert(key.to_string(), (now + BLOCK_FOR, reason.to_string()));
    }

    /// The reason while the block holds.
    pub(super) fn blocked(&mut self, now: Instant, key: &str) -> Option<String> {
        self.until.retain(|_, (until, _)| *until > now);
        self.until.get(key).map(|(_, reason)| reason.clone())
    }
}

/// Servers with a login in progress, by `server_key`.
#[derive(Default)]
pub(super) struct Logins {
    busy: HashSet<String>,
}

impl Logins {
    /// `true` when the caller may log in now (and must `end` afterwards).
    pub(super) fn begin(&mut self, key: &str) -> bool {
        self.busy.insert(key.to_string())
    }

    pub(super) fn end(&mut self, key: &str) {
        self.busy.remove(key);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn blocks() -> &'static Mutex<Blocks> {
    static BLOCKS: OnceLock<Mutex<Blocks>> = OnceLock::new();
    BLOCKS.get_or_init(|| Mutex::new(Blocks::default()))
}

fn logins() -> &'static Mutex<Logins> {
    static LOGINS: OnceLock<Mutex<Logins>> = OnceLock::new();
    LOGINS.get_or_init(|| Mutex::new(Logins::default()))
}

/// Blocks extra connections of this account for 15 minutes, process-wide.
pub(super) fn block(key: &str, reason: &str) {
    lock(blocks()).block(Instant::now(), key, reason);
}

/// The reason, while extra connections of this account are blocked.
pub(super) fn blocked(key: &str) -> Option<String> {
    lock(blocks()).blocked(Instant::now(), key)
}

/// The right to have one unauthenticated connection to a server; released on
/// drop.
pub(super) struct LoginSlot {
    key: String,
}

/// `None` while another connection to the same host and port logs in.
pub(super) fn begin_login(key: &str) -> Option<LoginSlot> {
    lock(logins()).begin(key).then(|| LoginSlot {
        key: key.to_string(),
    })
}

impl Drop for LoginSlot {
    fn drop(&mut self) {
        lock(logins()).end(&self.key);
    }
}

/// A channel as the choice sees it: its leases and the handles it may hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Candidate {
    pub(super) load: usize,
    pub(super) cap: usize,
}

/// What a transfer does about a channel.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Seat {
    /// Share this channel (index into the candidates).
    Use(usize),
    /// Open one more channel on the main connection.
    Open,
    /// Every channel holds its share of handles and no more can be opened:
    /// wait for a lease to end.
    Wait,
    /// The pool may hold no channel at all.
    MainSession,
}

/// The channel for a new transfer: an idle one; else a new one while the
/// limit allows (`pick`); else the least busy one that still has a free
/// handle (at most `cap` per channel, security answer E13-S13); else wait.
pub(super) fn seat(channels: &[Candidate], opening: usize, limit: Option<usize>) -> Seat {
    let loads: Vec<usize> = channels.iter().map(|channel| channel.load).collect();
    match pick(&loads, opening, limit) {
        Pick::Open => Seat::Open,
        Pick::MainSession => Seat::MainSession,
        Pick::Use(_) => channels
            .iter()
            .enumerate()
            .filter(|(_, channel)| channel.load < channel.cap)
            .min_by_key(|(_, channel)| channel.load)
            .map_or(Seat::Wait, |(index, _)| Seat::Use(index)),
    }
}

/// The note next to a transfer (`Backend::transfer_hint`): the connections
/// in use and, while it holds, the block with its reason.
pub(super) fn hint_text(host: &str, open: usize, blocked: Option<&str>) -> Option<String> {
    let mut parts = Vec::new();
    if open > 1 {
        parts.push(format!("{open} Verbindungen zu {host}"));
    }
    if let Some(reason) = blocked {
        parts.push(format!(
            "Weitere Verbindungen zu {host} 15 Minuten gesperrt: {reason}"
        ));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// Whether a host key presented by an extra connection is acceptable
/// (E13-S15 (ii)): the trust store must accept it in the plain `Verify`
/// mode (a pinned or OpenSSH-known key; never pinned here, never asked
/// about, a revoked key never) and it must be the key, with the same
/// algorithm and fingerprint, that the verified main connection showed.
pub(super) fn same_host_key(
    verdict: super::host_key::HostKeyVerdict,
    algorithm: &str,
    fingerprint: &str,
    main: &super::host_key::UnknownHostKey,
) -> Result<(), String> {
    use super::host_key::HostKeyVerdict;
    match verdict {
        HostKeyVerdict::Trusted => {}
        HostKeyVerdict::Unknown => {
            return Err("Host-Schlüssel steht nicht im Vertrauensspeicher".to_string())
        }
        HostKeyVerdict::Changed { .. } => {
            return Err("Host-Schlüssel weicht vom Vertrauensspeicher ab".to_string())
        }
        HostKeyVerdict::Revoked { .. } => return Err("Host-Schlüssel ist widerrufen".to_string()),
    }
    if algorithm != main.algorithm || fingerprint != main.fingerprint {
        return Err(format!(
            "Host-Schlüssel weicht von der Hauptverbindung ab ({algorithm} {fingerprint})"
        ));
    }
    Ok(())
}

/// Channels left free after a refusal: the agent's exec channel and the
/// short-lived posix-rename channel (plan K25).
const RESERVED_CHANNELS: usize = 2;

/// Pool channels allowed after the server refused one while `open` were
/// open: two stay free for exec and posix-rename.
pub(super) fn learned_limit(open: usize) -> usize {
    open.saturating_sub(RESERVED_CHANNELS)
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Pick {
    Use(usize),
    Open,
    MainSession,
}

/// The channel for a new transfer, from the channels' current `loads`: an
/// idle one; else a new one while the limit allows; else the least busy one.
/// `MainSession` when the pool may hold no channel at all.
pub(super) fn pick(loads: &[usize], opening: usize, limit: Option<usize>) -> Pick {
    let may_open = limit.is_none_or(|limit| loads.len() + opening < limit);
    let least = loads.iter().enumerate().min_by_key(|(_, load)| **load);
    match least {
        Some((index, &load)) if load == 0 || !may_open => Pick::Use(index),
        None if !may_open => Pick::MainSession,
        _ => Pick::Open,
    }
}

/// Whether the pool can serve a transfer: it is not switched off for the
/// current connection and has not learned that no channel fits next to the
/// main session (`pick` would answer `MainSession`).
pub(super) fn pool_usable(disabled: bool, channels: usize, limit: Option<usize>) -> bool {
    !disabled && !(channels == 0 && limit == Some(0))
}

/// The main connection's channel limit after the server refused one: the
/// channels it holds are the ready ones and those it accepted whose setup
/// is still running (counting only the ready ones would learn a limit far
/// too low when many channels open at once on a slow link); the smaller of
/// this and what was known before.
pub(super) fn limit_after_refusal(known: Option<usize>, ready: usize, in_setup: usize) -> usize {
    let learned = learned_limit(ready + in_setup);
    known.map_or(learned, |known| known.min(learned))
}
