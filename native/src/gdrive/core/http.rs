//! Pooled HTTPS clients of one Drive connection. Each free `ureq::get`/`post`
//! builds a new agent, and an agent keeps one idle socket per host by default,
//! so every call paid its own TCP and TLS handshake (ref §7).
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Idle sockets kept per host and in total (every Drive URL is on
/// www.googleapis.com): as many as operations the adaptive transfer flow may
/// run at once on one connection (its protective cap, recherche §2), so no
/// socket of a burst is closed only for the next burst to open it again.
const POOL_SOCKETS: usize = 256;

/// ureq 2.12.1 clears a socket's timeouts when it returns to the pool and
/// sets none before sending on it again (`Stream::reset`, `connect_socket`);
/// only the response read gets a deadline. A socket whose NAT or firewall
/// mapping expired silently would hold a request until the OS gives up.
/// Transfers reuse sockets within milliseconds, while such silent expiry takes
/// minutes; sockets idle longer than this are dropped with their pool.
const POOL_IDLE_LIMIT: Duration = Duration::from_secs(60);

pub(super) struct DriveHttp {
    api: Pool,
    stream: Pool,
}

impl DriveHttp {
    /// `inactivity` bounds each socket read/write of a streaming download.
    pub(super) fn new(inactivity: Duration) -> Self {
        Self {
            api: Pool::new(Kind::Api),
            stream: Pool::new(Kind::Stream { inactivity }),
        }
    }

    /// Metadata, mutations and upload sessions. Every request sets its own
    /// deadline; redirects are never followed (ureq would turn a 301–303 of a
    /// body request into a GET, and Drive's API answers directly).
    pub(super) fn api(&self) -> ureq::Agent {
        self.api.agent()
    }

    /// Downloads: per-socket inactivity deadlines instead of a request
    /// deadline, so a large transfer has no wall-clock cap while a stalled
    /// socket still fails; redirects as before.
    pub(super) fn stream(&self) -> ureq::Agent {
        self.stream.agent()
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Api,
    Stream { inactivity: Duration },
}

struct Pool {
    kind: Kind,
    current: Mutex<(ureq::Agent, Instant)>,
}

impl Pool {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            current: Mutex::new((build(kind), Instant::now())),
        }
    }

    fn agent(&self) -> ureq::Agent {
        let now = Instant::now();
        // The guarded pair is always valid, even after a panic elsewhere.
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if now.duration_since(current.1) > POOL_IDLE_LIMIT {
            current.0 = build(self.kind);
        }
        current.1 = now;
        current.0.clone()
    }
}

fn build(kind: Kind) -> ureq::Agent {
    let builder = ureq::AgentBuilder::new()
        .timeout_connect(super::api::DRIVE_CONNECT_TIMEOUT)
        .max_idle_connections(POOL_SOCKETS)
        .max_idle_connections_per_host(POOL_SOCKETS);
    match kind {
        Kind::Api => builder.redirects(0).build(),
        Kind::Stream { inactivity } => builder
            .timeout_read(inactivity)
            .timeout_write(inactivity)
            .build(),
    }
}
