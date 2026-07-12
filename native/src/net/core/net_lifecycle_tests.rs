use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::net::UncBackend;
use crate::vfs::Backend;

use super::{lock_unpoisoned, Entry, Lifecycle, Platform, Registry};

#[derive(Default)]
struct MockPlatform {
    connect_count: AtomicUsize,
    disconnect_count: AtomicUsize,
    fail_disconnects: AtomicUsize,
    block_disconnect: AtomicBool,
    gate: (Mutex<DisconnectGate>, Condvar),
    events: Mutex<Vec<&'static str>>,
}

#[derive(Default)]
struct DisconnectGate {
    started: bool,
    released: bool,
}

impl MockPlatform {
    fn registry(self: &Arc<Self>) -> Registry {
        Registry::new(self.clone())
    }

    fn events(&self) -> Vec<&'static str> {
        lock_unpoisoned(&self.events).clone()
    }

    fn wait_for_disconnect_start(&self) {
        let (gate, changed) = &self.gate;
        let guard = lock_unpoisoned(gate);
        let (guard, timeout) = changed
            .wait_timeout_while(guard, Duration::from_secs(2), |state| !state.started)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(!timeout.timed_out(), "disconnect did not start");
        assert!(guard.started);
    }

    fn release_disconnect(&self) {
        let (gate, changed) = &self.gate;
        lock_unpoisoned(gate).released = true;
        changed.notify_all();
    }

    fn fail_next_disconnect(&self) {
        self.fail_disconnects.fetch_add(1, Ordering::SeqCst);
    }
}

impl Platform for MockPlatform {
    fn connect(
        &self,
        _share: &str,
        _user: Option<&str>,
        _password: Option<&str>,
    ) -> io::Result<()> {
        lock_unpoisoned(&self.events).push("connect:start");
        self.connect_count.fetch_add(1, Ordering::SeqCst);
        lock_unpoisoned(&self.events).push("connect:end");
        Ok(())
    }

    fn disconnect(&self, _share: &str) -> io::Result<()> {
        lock_unpoisoned(&self.events).push("disconnect:start");
        self.disconnect_count.fetch_add(1, Ordering::SeqCst);
        if self.block_disconnect.load(Ordering::SeqCst) {
            let (gate, changed) = &self.gate;
            let mut guard = lock_unpoisoned(gate);
            guard.started = true;
            changed.notify_all();
            while !guard.released {
                guard = changed
                    .wait(guard)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        }
        lock_unpoisoned(&self.events).push("disconnect:end");
        if self
            .fail_disconnects
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            Err(io::Error::other("injected disconnect failure"))
        } else {
            Ok(())
        }
    }
}

#[test]
fn multiple_leases_share_one_session_and_disconnect_once() {
    let platform = Arc::new(MockPlatform::default());
    let registry = platform.registry();
    let first = registry
        .connect(r"\\server\share", Some("Alice"), Some("secret"))
        .unwrap();
    let second = registry
        .connect(r"//SERVER/share/folder", Some("alice"), Some("ignored"))
        .unwrap();

    assert_eq!(platform.connect_count.load(Ordering::SeqCst), 1);
    drop(first);
    assert_eq!(platform.disconnect_count.load(Ordering::SeqCst), 0);
    drop(second);
    assert_eq!(platform.disconnect_count.load(Ordering::SeqCst), 1);
}

#[test]
fn last_drop_and_new_connect_serialize_disconnect_before_connect() {
    let platform = Arc::new(MockPlatform::default());
    platform.block_disconnect.store(true, Ordering::SeqCst);
    let registry = Arc::new(platform.registry());
    let first = registry
        .connect(r"\\server\share", Some("alice"), Some("secret"))
        .unwrap();

    let dropper = std::thread::spawn(move || drop(first));
    platform.wait_for_disconnect_start();

    let (sent, received) = std::sync::mpsc::channel();
    let connecting_registry = Arc::clone(&registry);
    let connector = std::thread::spawn(move || {
        let result =
            connecting_registry.connect(r"\\server\share\folder", Some("alice"), Some("secret"));
        sent.send(result).unwrap();
    });
    assert!(received.recv_timeout(Duration::from_millis(100)).is_err());

    platform.release_disconnect();
    dropper.join().unwrap();
    let second = received
        .recv_timeout(Duration::from_secs(2))
        .expect("new connect remained blocked")
        .unwrap();
    connector.join().unwrap();
    assert_eq!(
        platform.events(),
        vec![
            "connect:start",
            "connect:end",
            "disconnect:start",
            "disconnect:end",
            "connect:start",
            "connect:end",
        ]
    );

    drop(second);
    assert_eq!(platform.disconnect_count.load(Ordering::SeqCst), 2);
}

#[test]
fn new_connect_wins_zero_strong_handoff_without_stale_disconnect() {
    let platform = Arc::new(MockPlatform::default());
    let entry = Arc::new(Entry::new(r"\\server\share".into(), platform.clone()));
    Platform::connect(
        platform.as_ref(),
        r"\\server\share",
        Some("alice"),
        Some("secret"),
    )
    .unwrap();
    let stale_generation = 7;
    {
        let mut state = lock_unpoisoned(&entry.state);
        state.next_generation = stale_generation;
        state.lifecycle = Lifecycle::Connected {
            user: Some("alice".into()),
            generation: stale_generation,
            lease: std::sync::Weak::new(),
        };
    }

    let replacement = entry
        .acquire(Some("alice".into()), Some("alice"), Some("secret"))
        .unwrap();
    entry.release(stale_generation);
    assert_eq!(platform.connect_count.load(Ordering::SeqCst), 1);
    assert_eq!(platform.disconnect_count.load(Ordering::SeqCst), 0);

    drop(replacement);
    assert_eq!(platform.disconnect_count.load(Ordering::SeqCst), 1);
}

#[test]
fn failed_disconnect_stays_registered_and_rejects_another_user() {
    let platform = Arc::new(MockPlatform::default());
    let registry = platform.registry();
    platform.fail_next_disconnect();
    let alice = registry
        .connect(r"\\server\share", Some("alice"), Some("secret"))
        .unwrap();
    drop(alice);
    assert_eq!(platform.disconnect_count.load(Ordering::SeqCst), 1);

    let error = registry
        .connect(r"\\server\share", Some("bob"), Some("other"))
        .err()
        .expect("different user must fail closed");
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(platform.connect_count.load(Ordering::SeqCst), 1);

    let alice = registry
        .connect(r"\\server\share", Some("ALICE"), Some("secret"))
        .unwrap();
    assert_eq!(platform.connect_count.load(Ordering::SeqCst), 1);
    drop(alice);
    assert_eq!(platform.disconnect_count.load(Ordering::SeqCst), 2);

    let bob = registry
        .connect(r"\\server\share", Some("bob"), Some("other"))
        .unwrap();
    assert_eq!(platform.connect_count.load(Ordering::SeqCst), 2);
    drop(bob);
}

#[test]
fn unc_io_handles_retain_the_final_lease() {
    let platform = Arc::new(MockPlatform::default());
    let registry = platform.registry();
    let connection = registry
        .connect(r"\\server\share", Some("alice"), Some("secret"))
        .unwrap();
    let base = std::env::temp_dir().join(format!(
        "se-unc-lease-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&base).unwrap();
    let source = base.join("source.txt");
    let destination = base.join("destination.txt");
    std::fs::write(&source, b"data").unwrap();
    let backend = UncBackend::new(base.to_str().unwrap(), connection);
    let reader = backend.open_read(source.to_str().unwrap()).unwrap();
    let writer = backend.open_write(destination.to_str().unwrap()).unwrap();

    drop(backend);
    drop(reader);
    assert_eq!(platform.disconnect_count.load(Ordering::SeqCst), 0);
    drop(writer);
    assert_eq!(platform.disconnect_count.load(Ordering::SeqCst), 1);
    std::fs::remove_dir_all(base).unwrap();
}
