//! Exec-channel pool of an SSH-deployed agent (K5). Every SSH session channel
//! has its own flow-control window, so streams sharing one channel share its
//! window. When every open channel is busy the pool opens one more agent
//! channel in the background, until the server refuses a session. The
//! refusal is the learned limit: the server's session count (our agent
//! channels plus the main SFTP channel) minus two reserves (the main SFTP
//! channel and the short-lived posix-rename channel), so the pool keeps
//! "open agent channels - 1". Requests go to the least busy channel.
use super::transport::{AgentConnection, AgentReconnect};
use crate::agent_proto::ServerFeatures;
use std::io;
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(super) struct PooledChannel {
    connection: Arc<AgentConnection>,
    in_flight: AtomicUsize,
}

impl PooledChannel {
    fn new(connection: Arc<AgentConnection>) -> Arc<Self> {
        Arc::new(Self {
            connection,
            in_flight: AtomicUsize::new(0),
        })
    }
}

/// One request's hold on a channel; released when dropped.
pub(super) struct ChannelLease {
    channel: Arc<PooledChannel>,
}

impl Deref for ChannelLease {
    type Target = Arc<AgentConnection>;

    fn deref(&self) -> &Self::Target {
        &self.channel.connection
    }
}

impl Drop for ChannelLease {
    fn drop(&mut self) {
        self.channel.in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Default)]
struct Growth {
    opening: bool,
    /// Agent channels the server accepts, once a refusal taught it.
    limit: Option<usize>,
}

pub(super) struct AgentPool {
    primary: Arc<PooledChannel>,
    extra: Mutex<Vec<Arc<PooledChannel>>>,
    opener: Option<AgentReconnect>,
    growth: Mutex<Growth>,
    features: ServerFeatures,
}

impl AgentPool {
    /// One connection only (Share/Room through the service, fixtures).
    pub(super) fn single(connection: Arc<AgentConnection>, features: ServerFeatures) -> Arc<Self> {
        Self::build(connection, features, None)
    }

    /// A pool that may open further agent channels through `opener`.
    pub(super) fn growable(
        connection: Arc<AgentConnection>,
        features: ServerFeatures,
        opener: AgentReconnect,
    ) -> Arc<Self> {
        Self::build(connection, features, Some(opener))
    }

    fn build(
        connection: Arc<AgentConnection>,
        features: ServerFeatures,
        opener: Option<AgentReconnect>,
    ) -> Arc<Self> {
        Arc::new(Self {
            primary: PooledChannel::new(connection),
            extra: Mutex::new(Vec::new()),
            opener,
            growth: Mutex::new(Growth::default()),
            features,
        })
    }

    pub(super) fn features(&self) -> ServerFeatures {
        self.features
    }

    /// The least busy channel. When even that one is busy, the pool grows
    /// in the background; this request does not wait for the new channel.
    pub(super) fn lease(self: &Arc<Self>) -> ChannelLease {
        let mut channel = self.primary.clone();
        let mut load = channel.in_flight.load(Ordering::Acquire);
        for candidate in lock(&self.extra).iter() {
            let candidate_load = candidate.in_flight.load(Ordering::Acquire);
            if candidate_load < load {
                load = candidate_load;
                channel = candidate.clone();
            }
        }
        let busy = channel.in_flight.fetch_add(1, Ordering::AcqRel) > 0;
        if busy {
            self.grow();
        }
        ChannelLease { channel }
    }

    fn grow(self: &Arc<Self>) {
        let Some(opener) = self.opener.clone() else {
            return;
        };
        {
            let mut growth = lock(&self.growth);
            let open = self.channel_count();
            if growth.opening || growth.limit.is_some_and(|limit| open >= limit) {
                return;
            }
            growth.opening = true;
        }
        let pool: Weak<Self> = Arc::downgrade(self);
        let spawned = std::thread::Builder::new()
            .name("agent-pool-open".into())
            .spawn(move || {
                let opened = opener()
                    .and_then(|streams| AgentConnection::new(streams, Some(opener.clone())));
                if let Some(pool) = pool.upgrade() {
                    pool.finish_growth(opened);
                }
            });
        if spawned.is_err() {
            lock(&self.growth).opening = false;
        }
    }

    fn finish_growth(&self, opened: io::Result<(Arc<AgentConnection>, String)>) {
        let mut growth = lock(&self.growth);
        growth.opening = false;
        let mut extra = lock(&self.extra);
        match opened {
            Ok((connection, _)) => extra.push(PooledChannel::new(connection)),
            Err(_) => {
                let open = extra.len() + 1;
                let limit = open.saturating_sub(1).max(1);
                growth.limit = Some(limit);
                // Channels above the limit drain their requests and close
                // once their last lease ends; the first channel stays.
                extra.truncate(limit - 1);
            }
        }
    }

    /// Agent channels currently open.
    pub(super) fn channel_count(&self) -> usize {
        lock(&self.extra).len() + 1
    }

    /// Hard bound of concurrent transfer operations: requests per channel
    /// minus the browsing reserve, times the channels. A growable pool
    /// knows its bound only after the server refused a channel.
    pub(super) fn transfer_ceiling(&self) -> Option<usize> {
        let per_channel = self.features.transfer_slots();
        if self.opener.is_none() {
            return Some(per_channel);
        }
        lock(&self.growth)
            .limit
            .map(|limit| limit.saturating_mul(per_channel))
    }
}
