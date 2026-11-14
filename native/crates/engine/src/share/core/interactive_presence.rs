//! Interactive presence (plan 2026-11-14 E3): a process-wide count of the
//! interactive sessions that run right now. While at least one runs, bulk
//! file connections yield to queueing delay (`yield_controller.rs`, E2) and
//! no further bulk connection is added (`node_bulk.rs`), because the common
//! bottleneck is usually this device's own access link (WLAN, uplink), so
//! transfers to any peer delay the picture, the terminal or the tunnel.
//!
//! A session holds an [`InteractiveGuard`] for as long as it runs; dropping
//! the guard (end, error, cancellation, panic unwinding, task abort) removes
//! the entry. Registered kinds: the screen host pipeline and the screen
//! viewer reader (modes `view` and `control` only), Exec jobs with a terminal
//! on both sides, and every running TCP pump (all tunnels count: they come
//! from "Bildschirm steuern: Erlauben", remote access and RDP).
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

/// What keeps the process interactive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum InteractiveKind {
    /// The host's picture pipeline of one viewer.
    ScreenHost,
    /// A viewer's picture reader (`view` or `control`).
    ScreenViewer,
    /// One Exec job with a terminal, on the client or the host.
    ExecTerminal,
    /// One running TCP pump of a tunnel.
    TcpPump,
}

impl InteractiveKind {
    const ALL: [InteractiveKind; 4] = [
        InteractiveKind::ScreenHost,
        InteractiveKind::ScreenViewer,
        InteractiveKind::ExecTerminal,
        InteractiveKind::TcpPump,
    ];

    fn index(self) -> usize {
        match self {
            InteractiveKind::ScreenHost => 0,
            InteractiveKind::ScreenViewer => 1,
            InteractiveKind::ExecTerminal => 2,
            InteractiveKind::TcpPump => 3,
        }
    }
}

/// Counts of running interactive sessions by kind.
#[derive(Debug, Default)]
pub(crate) struct InteractivePresence {
    counts: [AtomicUsize; 4],
}

impl InteractivePresence {
    /// An empty register (tests; the process uses [`global`]).
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Registers one running session of `kind` until the guard drops.
    pub(crate) fn enter(&'static self, kind: InteractiveKind) -> InteractiveGuard {
        self.counts[kind.index()].fetch_add(1, Ordering::AcqRel);
        InteractiveGuard {
            presence: self,
            kind,
        }
    }

    /// Whether at least one interactive session runs.
    pub(crate) fn active(&self) -> bool {
        self.counts
            .iter()
            .any(|count| count.load(Ordering::Acquire) > 0)
    }

    /// Running sessions of `kind`.
    pub(crate) fn count(&self, kind: InteractiveKind) -> usize {
        self.counts[kind.index()].load(Ordering::Acquire)
    }

    /// Running sessions of every kind.
    #[allow(dead_code)] // diagnostics and measurements (P7)
    pub(crate) fn total(&self) -> usize {
        InteractiveKind::ALL
            .iter()
            .map(|kind| self.count(*kind))
            .sum()
    }
}

/// The register of this process.
pub(crate) fn global() -> &'static InteractivePresence {
    static PRESENCE: OnceLock<InteractivePresence> = OnceLock::new();
    PRESENCE.get_or_init(InteractivePresence::new)
}

/// Registers one running session of `kind` in the process register.
#[allow(dead_code)] // callers: TCP pump (P3), screen pipeline and reader (P4)
pub(crate) fn enter(kind: InteractiveKind) -> InteractiveGuard {
    global().enter(kind)
}

/// Whether the process runs at least one interactive session.
pub(crate) fn active() -> bool {
    global().active()
}

/// One registered interactive session; dropping it ends the entry.
#[must_use = "the session counts as interactive only while the guard lives"]
#[derive(Debug)]
pub(crate) struct InteractiveGuard {
    presence: &'static InteractivePresence,
    kind: InteractiveKind,
}

impl InteractiveGuard {
    /// The kind this guard registers.
    #[allow(dead_code)] // diagnostics and tests
    pub(crate) fn kind(&self) -> InteractiveKind {
        self.kind
    }
}

impl Drop for InteractiveGuard {
    fn drop(&mut self) {
        // `fetch_update` never wraps below zero, even if a count was
        // corrupted elsewhere; every guard was counted once in `enter`.
        let _ = self.presence.counts[self.kind.index()].fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |count| count.checked_sub(1),
        );
    }
}

/// A lifetime retainer (as Exec's `retainer`) that also holds an
/// interactive entry: the entry ends when the last holder drops it.
pub(crate) fn retain_with(
    kind: InteractiveKind,
    retainer: Option<Arc<dyn Send + Sync>>,
) -> Arc<dyn Send + Sync> {
    retain_in(global(), kind, retainer)
}

fn retain_in(
    presence: &'static InteractivePresence,
    kind: InteractiveKind,
    retainer: Option<Arc<dyn Send + Sync>>,
) -> Arc<dyn Send + Sync> {
    struct Retained {
        _retainer: Option<Arc<dyn Send + Sync>>,
        _guard: InteractiveGuard,
    }
    Arc::new(Retained {
        _retainer: retainer,
        _guard: presence.enter(kind),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register() -> &'static InteractivePresence {
        Box::leak(Box::new(InteractivePresence::new()))
    }

    #[test]
    fn verbindungen_task_presence_counts_up_and_down() {
        let presence = register();
        assert!(!presence.active());
        let host = presence.enter(InteractiveKind::ScreenHost);
        let first = presence.enter(InteractiveKind::TcpPump);
        let second = presence.enter(InteractiveKind::TcpPump);
        assert!(presence.active());
        assert_eq!(presence.count(InteractiveKind::TcpPump), 2);
        assert_eq!(presence.count(InteractiveKind::ScreenHost), 1);
        assert_eq!(presence.total(), 3);
        assert_eq!(host.kind(), InteractiveKind::ScreenHost);
        drop(first);
        assert_eq!(presence.count(InteractiveKind::TcpPump), 1);
        drop(host);
        assert!(presence.active());
        drop(second);
        assert!(!presence.active());
        assert_eq!(presence.total(), 0);
    }

    #[test]
    fn verbindungen_task_presence_ends_on_abort_and_panic() {
        let presence = register();
        // A task that is aborted drops its guard.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let guard = presence.enter(InteractiveKind::ScreenViewer);
            let task = tokio::spawn(async move {
                let _guard = guard;
                std::future::pending::<()>().await;
            });
            tokio::task::yield_now().await;
            assert!(presence.active());
            task.abort();
            let _ = task.await;
        });
        assert!(!presence.active());
        // Unwinding drops it as well.
        let unwound = std::panic::catch_unwind(|| {
            let _guard = presence.enter(InteractiveKind::ExecTerminal);
            panic!("session failed");
        });
        assert!(unwound.is_err());
        assert_eq!(presence.count(InteractiveKind::ExecTerminal), 0);
    }

    #[test]
    fn verbindungen_task_presence_retainer_holds_entry_until_last_drop() {
        let presence = register();
        let inner: Arc<()> = Arc::new(());
        let held: Arc<dyn Send + Sync> = inner.clone();
        let retained = retain_in(presence, InteractiveKind::ExecTerminal, Some(held));
        let clone = retained.clone();
        assert_eq!(presence.count(InteractiveKind::ExecTerminal), 1);
        drop(retained);
        assert_eq!(presence.count(InteractiveKind::ExecTerminal), 1);
        drop(clone);
        assert_eq!(presence.count(InteractiveKind::ExecTerminal), 0);
        assert_eq!(Arc::strong_count(&inner), 1);
    }
}
