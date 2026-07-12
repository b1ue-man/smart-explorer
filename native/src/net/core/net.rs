//! Authenticated network-share access via `WNetAddConnection2W` (mpr.dll).
//!
//! `\\server\share` UNC paths and mapped drive letters are *browsed* through
//! `LocalBackend` (std::fs) — no new filesystem code. This module only adds the
//! ability to authenticate to a share that needs credentials (deviceless: no
//! drive letter is mapped), after which the UNC path reads normally. The
//! connection is held open by `NetConnection` and torn down on drop.
//!
//! Local-network DISCOVERY (browsing the neighborhood) is intentionally NOT
//! here: it's unreliable on Win11 (SMB1 Computer Browser gone; WNetEnumResource
//! / NET VIEW widely broken). Connecting to a KNOWN address works; that's the
//! supported UX. See docs/GOTCHAS.md / REMOTE_LAYER_PLAN §4.
#![allow(dead_code)] // staged: wired in by the connect-UI step.

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};

use super::platform::{connect_impl, disconnect_impl};

/// `\\server\share` (back-slash, canonical for WNet) from any path beneath it,
/// or `None` for a non-UNC path. Accepts forward- or back-slash input (the app
/// stores paths forward-slashed).
pub fn share_root(path: &str) -> Option<String> {
    if !is_unc(path) {
        return None;
    }
    let body = path.trim().trim_start_matches(['\\', '/']);
    let mut parts = body.split(['\\', '/']).filter(|s| !s.is_empty());
    let server = parts.next()?;
    let share = parts.next()?;
    if server.is_empty() || share.is_empty() {
        return None;
    }
    Some(format!("\\\\{}\\{}", server, share))
}

/// Whether a path is a UNC path (`\\server\…` or `//server/…`).
pub fn is_unc(path: &str) -> bool {
    let p = path.trim_start();
    p.starts_with("\\\\") || p.starts_with("//")
}

/// A live authenticated connection to a network share. Connections to the same
/// share and user are process-wide leases; only the final lease disconnects the
/// WNet session, so closing one tab cannot invalidate another tab's SMB access.
#[derive(Clone)]
pub struct NetConnection {
    lease: Arc<Lease>,
}

struct Lease {
    entry: Arc<Entry>,
    generation: u64,
}

trait Platform: Send + Sync {
    fn connect(&self, share: &str, user: Option<&str>, password: Option<&str>) -> io::Result<()>;
    fn disconnect(&self, share: &str) -> io::Result<()>;
}

struct SystemPlatform;

impl Platform for SystemPlatform {
    fn connect(&self, share: &str, user: Option<&str>, password: Option<&str>) -> io::Result<()> {
        connect_impl(share, user, password)
    }

    fn disconnect(&self, share: &str) -> io::Result<()> {
        disconnect_impl(share)
    }
}

struct Registry {
    entries: Mutex<HashMap<String, Arc<Entry>>>,
    platform: Arc<dyn Platform>,
}

struct Entry {
    share: String,
    platform: Arc<dyn Platform>,
    state: Mutex<EntryState>,
    changed: Condvar,
}

struct EntryState {
    lifecycle: Lifecycle,
    next_generation: u64,
}

enum Lifecycle {
    Disconnected,
    Connecting {
        user: Option<String>,
    },
    Connected {
        user: Option<String>,
        generation: u64,
        lease: Weak<Lease>,
    },
    Disconnecting {
        user: Option<String>,
        generation: u64,
    },
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Registry::new(Arc::new(SystemPlatform)))
}

fn lease_key(share: &str) -> String {
    share.replace('/', "\\").to_lowercase()
}

fn normalized_user(user: Option<&str>) -> Option<String> {
    user.map(str::trim)
        .filter(|user| !user.is_empty())
        .map(str::to_lowercase)
}

impl NetConnection {
    /// Authenticate to the share that `unc` lives under. `user`/`password` may be
    /// `None` to use the caller's current credentials (Kerberos/NTLM SSO).
    pub fn connect(
        unc: &str,
        user: Option<&str>,
        password: Option<&str>,
    ) -> io::Result<NetConnection> {
        registry().connect(unc, user, password)
    }

    pub fn share(&self) -> &str {
        &self.lease.entry.share
    }
}

impl Registry {
    fn new(platform: Arc<dyn Platform>) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            platform,
        }
    }

    fn connect(
        &self,
        unc: &str,
        user: Option<&str>,
        password: Option<&str>,
    ) -> io::Result<NetConnection> {
        let share = share_root(unc).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "kein UNC-Pfad (\\\\server\\share)",
            )
        })?;
        let key = lease_key(&share);
        let requested_user = normalized_user(user);
        let entry = {
            let mut entries = lock_unpoisoned(&self.entries);
            entries
                .entry(key)
                .or_insert_with(|| Arc::new(Entry::new(share.clone(), Arc::clone(&self.platform))))
                .clone()
        };
        entry.acquire(requested_user, user, password)
    }
}

impl Entry {
    fn new(share: String, platform: Arc<dyn Platform>) -> Self {
        Self {
            share,
            platform,
            state: Mutex::new(EntryState {
                lifecycle: Lifecycle::Disconnected,
                next_generation: 0,
            }),
            changed: Condvar::new(),
        }
    }

    fn acquire(
        self: &Arc<Self>,
        requested_user: Option<String>,
        platform_user: Option<&str>,
        password: Option<&str>,
    ) -> io::Result<NetConnection> {
        let mut state = lock_unpoisoned(&self.state);
        loop {
            match &state.lifecycle {
                Lifecycle::Disconnected => {
                    state.lifecycle = Lifecycle::Connecting {
                        user: requested_user.clone(),
                    };
                    drop(state);
                    let connected = self.platform.connect(&self.share, platform_user, password);
                    state = lock_unpoisoned(&self.state);
                    if let Err(error) = connected {
                        state.lifecycle = Lifecycle::Disconnected;
                        self.changed.notify_all();
                        return Err(error);
                    }
                    let connection = self.activate(&mut state, requested_user.clone());
                    self.changed.notify_all();
                    return Ok(connection);
                }
                Lifecycle::Connecting { user } => {
                    if user != &requested_user {
                        return Err(user_conflict(&self.share));
                    }
                    state = wait_unpoisoned(&self.changed, state);
                }
                Lifecycle::Disconnecting { .. } => {
                    state = wait_unpoisoned(&self.changed, state);
                }
                Lifecycle::Connected {
                    user,
                    generation: _,
                    lease,
                } => {
                    if user != &requested_user {
                        return Err(user_conflict(&self.share));
                    }
                    if let Some(lease) = lease.upgrade() {
                        return Ok(NetConnection { lease });
                    }
                    // The final old Arc reached zero but has not necessarily won
                    // the state lock yet. Replacing its generation transfers the
                    // still-live WNet session to this lease; the stale Drop then
                    // observes the generation mismatch and cannot disconnect it.
                    return Ok(self.activate(&mut state, requested_user.clone()));
                }
            }
        }
    }

    fn activate(self: &Arc<Self>, state: &mut EntryState, user: Option<String>) -> NetConnection {
        state.next_generation = state.next_generation.wrapping_add(1).max(1);
        let generation = state.next_generation;
        let lease = Arc::new(Lease {
            entry: Arc::clone(self),
            generation,
        });
        state.lifecycle = Lifecycle::Connected {
            user,
            generation,
            lease: Arc::downgrade(&lease),
        };
        NetConnection { lease }
    }

    fn release(&self, generation: u64) {
        let user = {
            let mut state = lock_unpoisoned(&self.state);
            let Lifecycle::Connected {
                user,
                generation: active_generation,
                ..
            } = &state.lifecycle
            else {
                return;
            };
            if *active_generation != generation {
                return;
            }
            let user = user.clone();
            state.lifecycle = Lifecycle::Disconnecting {
                user: user.clone(),
                generation,
            };
            user
        };

        let disconnected = self.platform.disconnect(&self.share);
        let mut state = lock_unpoisoned(&self.state);
        if !matches!(
            state.lifecycle,
            Lifecycle::Disconnecting {
                generation: active_generation,
                ..
            } if active_generation == generation
        ) {
            return;
        }
        state.lifecycle = if disconnected.is_ok() {
            Lifecycle::Disconnected
        } else {
            // force=FALSE means a failed WNet cancellation did not tear down
            // the session. Keep it registered and fail closed for other users;
            // a future same-user lease retries cancellation on its final drop.
            Lifecycle::Connected {
                user,
                generation,
                lease: Weak::new(),
            }
        };
        self.changed.notify_all();
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.entry.release(self.generation);
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn wait_unpoisoned<'a, T>(condvar: &Condvar, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    condvar
        .wait(guard)
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn user_conflict(share: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("Netzlaufwerk {share} ist bereits mit einem anderen Benutzer verbunden"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unc_detection() {
        assert!(is_unc(r"\\server\share"));
        assert!(is_unc("//server/share"));
        assert!(!is_unc(r"C:\Users"));
        assert!(!is_unc("/home/user"));
        assert!(!is_unc("sftp://h/p"));
    }

    #[test]
    fn share_root_extraction() {
        assert_eq!(share_root(r"\\srv\pub\a\b").as_deref(), Some(r"\\srv\pub"));
        assert_eq!(share_root("//srv/pub/a/b").as_deref(), Some(r"\\srv\pub"));
        assert_eq!(share_root(r"\\srv\pub").as_deref(), Some(r"\\srv\pub"));
        assert_eq!(share_root(r"\\srv").as_deref(), None); // no share component
        assert_eq!(share_root(r"C:\x").as_deref(), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn connect_unsupported_off_windows() {
        let err = NetConnection::connect(r"\\srv\pub", Some("u"), Some("p"))
            .err()
            .expect("must error off-Windows");
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn connect_rejects_non_unc() {
        let err = NetConnection::connect("C:/x", None, None)
            .err()
            .expect("non-UNC must error");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn lease_identity_is_case_and_separator_stable() {
        assert_eq!(lease_key(r"\\Server\Share"), lease_key("//server/share"));
        assert_eq!(
            normalized_user(Some(" DOMAIN\\Alice ")),
            Some("domain\\alice".into())
        );
        assert_eq!(normalized_user(Some("  ")), None);
    }
}

#[cfg(test)]
#[path = "net_lifecycle_tests.rs"]
mod lifecycle_tests;
