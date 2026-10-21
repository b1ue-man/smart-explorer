//! Extra SSH connections of the pool (security answer E13-S15): the thread
//! that sets one up, the check that nothing stands in the way, the install
//! into the pool and the watchdog that closes idle ones. The decisions are
//! in link_rules.rs, the connection itself in extra_link.rs.
use super::channel_pool::ChannelPool;
use super::connection::SftpConnection;
use super::link_log::{self, LinkIdent};
use super::link_rules::{
    account_key, begin_login, block, blocked, hint_text, server_key, ExtraLogin,
};
use super::pool_channel::{setup, Owner, PoolChannel};
use super::session::{establish, Established, Request, SSH_WINDOW};
use crate::transfer::try_reserve_memory;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// Idle extra connections are looked at this often, so they close after a
/// minute without any transfer asking.
const WATCH_EVERY: Duration = Duration::from_secs(5);

/// What the pool needs to open extra connections.
pub(super) struct ExtraSupport {
    pub(super) host: String,
    pub(super) port: u16,
    pub(super) user: String,
    login: ExtraLogin,
    connection: Arc<SftpConnection>,
    runtime: tokio::runtime::Handle,
}

impl ExtraSupport {
    pub(super) fn new(
        connection: &Arc<SftpConnection>,
        host: &str,
        port: u16,
        user: &str,
        login: ExtraLogin,
    ) -> Self {
        Self {
            host: host.to_string(),
            port,
            user: user.to_string(),
            login,
            runtime: connection.runtime().handle().clone(),
            connection: connection.clone(),
        }
    }

    pub(super) fn account(&self) -> String {
        account_key(&self.host, self.port, &self.user)
    }
}

impl ChannelPool {
    /// The note next to a transfer: connections in use, or the block.
    pub(super) fn hint(&self) -> Option<String> {
        let support = self.support.as_ref()?;
        let open = 1 + lock(&self.state)
            .channels
            .iter()
            .filter(|channel| channel.extra_alive())
            .count();
        let reason = blocked(&support.account());
        hint_text(&support.host, open, reason.as_deref())
    }

    /// Sets up one extra connection on a thread of its own (called by the
    /// throughput window, never from the data path).
    pub(super) fn start_growth(pool: &Arc<Self>) {
        let worker = pool.clone();
        let started = std::thread::Builder::new()
            .name("sftp-extra-link".into())
            .spawn(move || worker.grow());
        if started.is_err() {
            pool.hub.attempt_cancelled();
        }
    }

    fn grow(self: &Arc<Self>) {
        let Some(support) = self.support.as_ref() else {
            self.hub.attempt_cancelled();
            return;
        };
        let account = support.account();
        if blocked(&account).is_some() {
            self.hub.attempt_cancelled();
            return;
        }
        let Ok(generation) = support.connection.current() else {
            self.hub.attempt_cancelled();
            return;
        };
        let Some(main_key) = generation.host_key().cloned() else {
            // Nothing to compare the key with: no extra connections.
            self.hub.attempt_failed();
            return;
        };
        let Some(slot) = begin_login(&server_key(&support.host, support.port)) else {
            self.hub.attempt_cancelled();
            return;
        };
        let Some(memory) = try_reserve_memory(u64::from(SSH_WINDOW)) else {
            self.hub.attempt_cancelled();
            return;
        };
        let request = Request {
            host: &support.host,
            port: support.port,
            user: &support.user,
            login: &support.login,
            generation: generation.clone(),
            main_key,
            slot,
            memory,
            open_links: self.open_links.clone(),
            runtime: support.runtime.clone(),
        };
        let hub = self.hub.clone();
        let built = support.runtime.block_on(async move {
            let Established { link, channel } = establish(request).await?;
            match setup(channel).await {
                Ok(setup) => Ok((link, setup)),
                Err(error) => {
                    link.mark_dead("Einrichtung fehlgeschlagen");
                    Err(format!("SFTP-Subsystem: {error}"))
                }
            }
        });
        match built {
            Ok((link, setup)) => {
                let ident = link.ident().clone();
                let channel = PoolChannel::new(setup, Owner::Extra(link), hub);
                let mut state = lock(&self.state);
                state.channels.push(Arc::new(channel));
                self.hub.set_links(1 + state.extra_channels());
                drop(state);
                link_log::emit(&link_log::line(
                    "geöffnet",
                    &ident,
                    1 + self.open_links.load(Ordering::Acquire),
                    "weitere Verbindung für parallele Übertragung",
                ));
                self.hub.attempt_installed();
                self.ensure_watchdog();
            }
            Err(reason) => {
                block(&account, &reason);
                let ident = LinkIdent {
                    host: support.host.clone(),
                    port: support.port,
                    user: support.user.clone(),
                    method: "-".to_string(),
                };
                link_log::emit(&link_log::line(
                    "gesperrt für 15 Minuten",
                    &ident,
                    1 + self.open_links.load(Ordering::Acquire),
                    &reason,
                ));
                self.hub.attempt_failed();
            }
        }
    }

    /// Closes idle extra connections after a minute even when no transfer
    /// calls the pool.
    fn ensure_watchdog(self: &Arc<Self>) {
        let Some(support) = self.support.as_ref() else {
            return;
        };
        if self.watching.swap(true, Ordering::AcqRel) {
            return;
        }
        let pool = Arc::downgrade(self);
        drop(support.runtime.spawn(async move {
            loop {
                tokio::time::sleep(WATCH_EVERY).await;
                let Some(pool) = pool.upgrade() else {
                    break;
                };
                pool.retire_idle();
                let extras = lock(&pool.state).extra_channels();
                if extras == 0 {
                    pool.watching.store(false, Ordering::Release);
                    break;
                }
            }
        }));
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
