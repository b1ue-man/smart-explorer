//! Idle keepalive for sleeping clients (`idle_keepalive_v1`).
//!
//! A client that negotiated the capability may announce `set_idle`; it then
//! stops sending heartbeats. The server drives liveness instead: every K
//! seconds it delivers deferred presence refreshes followed by `keepalive` and
//! closes the connection when no inbound data arrives within
//! [`IDLE_REPLY_WINDOW`]. Connections that are not idle keep their established
//! rule: raw TCP closes after [`ACTIVE_READ_WINDOW`] of silence, WebSocket has no
//! inbound deadline.

use std::sync::Arc;
use std::time::{Duration, Instant};

pub(super) const CAPABILITY: &str = "idle_keepalive_v1";
pub(super) const KEEPALIVE_ENV: &str = "SE_SHARE_IDLE_KEEPALIVE_SECS";
/// Inbound silence after which a raw TCP connection that is not idle is closed.
pub(super) const ACTIVE_READ_WINDOW: Duration = Duration::from_secs(60);
/// Time an idle client has to answer a server keepalive with any inbound data.
pub(super) const IDLE_REPLY_WINDOW: Duration = Duration::from_secs(60);
/// Shortest keepalive interval; a client proposal can never go below it, so a
/// sleeping phone is never woken more often than by its former heartbeat.
pub(super) const MIN_KEEPALIVE_SECS: u32 = 30;
/// Longest keepalive interval: the 300-second presence lifetime minus the
/// 60-second desktop refresh and a 30-second delivery margin. Above it a
/// deferred refresh could reach an idle observer after its copy expired.
pub(super) const MAX_KEEPALIVE_SECS: u32 = 210;
/// Three minutes, as used by push services, below the shortest measured
/// mobile-carrier TCP idle timeout (255 s) and inside [`MAX_KEEPALIVE_SECS`].
pub(super) const DEFAULT_KEEPALIVE_SECS: u32 = 180;
/// `set_read_timeout(Some(Duration::ZERO))` is rejected by the OS layer.
const MIN_READ_WAIT: Duration = Duration::from_millis(1);

/// Server keepalive interval K, always inside 30..=210 seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Keepalive(u32);

impl Default for Keepalive {
    fn default() -> Self {
        Self(DEFAULT_KEEPALIVE_SECS)
    }
}

impl Keepalive {
    pub(super) fn clamped(secs: u64) -> Self {
        let secs = secs.clamp(u64::from(MIN_KEEPALIVE_SECS), u64::from(MAX_KEEPALIVE_SECS));
        Self(u32::try_from(secs).unwrap_or(MAX_KEEPALIVE_SECS))
    }

    pub(super) fn secs(self) -> u32 {
        self.0
    }

    pub(super) fn duration(self) -> Duration {
        Duration::from_secs(u64::from(self.0))
    }

    /// Interval for one idle client: its proposal may only shorten the
    /// server's interval and never below [`MIN_KEEPALIVE_SECS`].
    pub(super) fn negotiate(self, proposal: Option<u32>) -> Self {
        match proposal {
            Some(proposal) => Self(proposal.clamp(MIN_KEEPALIVE_SECS, self.0)),
            None => self,
        }
    }
}

/// Reads [`KEEPALIVE_ENV`]. Values outside 30–210 s are clamped and the
/// returned notice says so; unparsable values are a startup error.
pub(super) fn keepalive_from_env() -> Result<(Keepalive, Option<String>), String> {
    match std::env::var(KEEPALIVE_ENV) {
        Ok(value) => parse_keepalive(&value),
        Err(std::env::VarError::NotPresent) => Ok((Keepalive::default(), None)),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(format!("{KEEPALIVE_ENV} is not valid Unicode"))
        }
    }
}

pub(super) fn parse_keepalive(value: &str) -> Result<(Keepalive, Option<String>), String> {
    let requested = value
        .trim()
        .parse::<u64>()
        .map_err(|error| format!("invalid {KEEPALIVE_ENV} value {value:?}: {error}"))?;
    let keepalive = Keepalive::clamped(requested);
    let notice = (u64::from(keepalive.secs()) != requested).then(|| {
        format!(
            "{KEEPALIVE_ENV}={requested} is outside {MIN_KEEPALIVE_SECS}-{MAX_KEEPALIVE_SECS} s; \
             using {} s",
            keepalive.secs()
        )
    });
    Ok((keepalive, notice))
}

/// Time source of the signaling transports: monotonic time drives deadlines,
/// wall-clock seconds judge the lifetime of signed presences.
pub(super) trait SignalClock: Send + Sync {
    fn now(&self) -> Instant;
    fn unix_secs(&self) -> i64;
}

struct SystemClock;

impl SignalClock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_secs(&self) -> i64 {
        super::discovery_state::unix_seconds()
    }
}

/// Timing shared by every connection of one server.
#[derive(Clone)]
pub(super) struct SignalTiming {
    pub(super) keepalive: Keepalive,
    pub(super) clock: Arc<dyn SignalClock>,
    /// Longest real wait inside one blocking read. Production waits exactly
    /// until the next deadline; tests set a short bound to poll an injected
    /// clock.
    pub(super) max_wait: Option<Duration>,
}

impl Default for SignalTiming {
    fn default() -> Self {
        Self::new(Keepalive::default())
    }
}

impl SignalTiming {
    pub(super) fn new(keepalive: Keepalive) -> Self {
        Self {
            keepalive,
            clock: Arc::new(SystemClock),
            max_wait: None,
        }
    }

    /// Socket read timeout for a wait from `now` until `until`; without a
    /// deadline the read blocks (`None`) unless tests bound the wait.
    pub(super) fn read_timeout(&self, now: Instant, until: Option<Instant>) -> Option<Duration> {
        let Some(until) = until else {
            return self.max_wait;
        };
        let wait = until.saturating_duration_since(now);
        let wait = self.max_wait.map_or(wait, |max_wait| wait.min(max_wait));
        Some(wait.max(MIN_READ_WAIT))
    }
}

/// Injectable clock for host tests. `advance` moves both clocks;
/// `advance_wall` models a suspended host whose monotonic clock stood still.
#[cfg(test)]
pub(super) struct TestClock {
    state: std::sync::Mutex<(Instant, i64)>,
}

#[cfg(test)]
impl TestClock {
    pub(super) fn new(unix_secs: i64) -> Arc<Self> {
        Arc::new(Self {
            state: std::sync::Mutex::new((Instant::now(), unix_secs)),
        })
    }

    pub(super) fn advance(&self, by: Duration) {
        let mut state = self.state.lock().unwrap();
        state.0 += by;
        state.1 += by.as_secs() as i64;
    }

    pub(super) fn advance_wall(&self, by: Duration) {
        self.state.lock().unwrap().1 += by.as_secs() as i64;
    }

    pub(super) fn timing(self: &Arc<Self>, keepalive: Keepalive) -> SignalTiming {
        SignalTiming {
            keepalive,
            clock: self.clone(),
            max_wait: Some(Duration::from_millis(5)),
        }
    }
}

#[cfg(test)]
impl SignalClock for TestClock {
    fn now(&self) -> Instant {
        self.state.lock().unwrap().0
    }

    fn unix_secs(&self) -> i64 {
        self.state.lock().unwrap().1
    }
}
