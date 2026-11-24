//! Install-on-use of system components (user order 2026-11-24): a feature
//! that needs Dokany (`mount/os/windows/runtime_install.rs`) or a Linux
//! package (`login/os/linux_os/package_install.rs`) installs it at the
//! moment of use, behind the operating system's own confirmation, and then
//! runs once more.
//!
//! One install of a component at a time: a file lock in the app data folder
//! serialises every process of this account (and, through separate opens of
//! the lock file, every thread). A caller that waited for a running install
//! takes its result instead of asking the operating system again, so the
//! drives restored together at login raise one confirmation. A record next
//! to the lock tells other processes (the desktop window) that an install
//! runs and how the last one ended.
use std::fs::{File, TryLockError};
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How one install ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "reason", rename_all = "snake_case")]
pub enum Attempt {
    /// The component is installed, or was present already.
    Installed,
    /// The person declined the operating system's confirmation; the German
    /// reason.
    Declined(String),
    /// The install failed; the German reason with the failing step's message.
    Failed(String),
}

impl Attempt {
    /// `Ok` when installed, else the reason.
    pub fn result(self) -> Result<(), String> {
        match self {
            Self::Installed => Ok(()),
            Self::Declined(reason) | Self::Failed(reason) => Err(reason),
        }
    }
}

/// What other processes see of a component's install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    /// No install runs and none is recorded.
    Idle,
    /// An install runs now.
    Running,
    /// The last install ended so.
    Ended(Attempt),
}

/// The failure of an operation that needs the component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The component is missing; installing it may help.
    Missing(String),
    /// Anything else; installing would not help.
    Other(String),
}

impl Failure {
    pub fn into_reason(self) -> String {
        match self {
            Self::Missing(reason) | Self::Other(reason) => reason,
        }
    }
}

/// Runs `operation`; when it fails because the component is missing, it
/// installs the component once and runs `operation` exactly once more. A
/// declined or failed install ends with its reason.
pub fn with_install<T>(
    mut operation: impl FnMut() -> Result<T, Failure>,
    install: impl FnOnce() -> Attempt,
) -> Result<T, String> {
    match operation() {
        Ok(value) => Ok(value),
        Err(Failure::Other(reason)) => Err(reason),
        Err(Failure::Missing(_)) => match install() {
            Attempt::Installed => operation().map_err(Failure::into_reason),
            Attempt::Declined(reason) | Attempt::Failed(reason) => Err(reason),
        },
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Record {
    running: bool,
    #[serde(default)]
    ended: Option<Attempt>,
    /// Unix milliseconds of the last change.
    at_ms: u64,
}

/// Serialises the installs of this process when a lock file cannot be
/// opened (the separate opens of a lock file serialise threads otherwise).
static LOCAL: Mutex<()> = Mutex::new(());

/// The single-flight gate of one component.
pub struct Gate {
    dir: PathBuf,
    name: String,
}

impl Gate {
    /// The gate of `name` in this account's app data folder.
    pub fn named(name: &str) -> Self {
        Self::at(crate::support_dirs::app_data_dir(), name)
    }

    /// The gate of `name` in `dir` (tests use a temporary folder).
    pub fn at(dir: PathBuf, name: &str) -> Self {
        Self {
            dir,
            name: name.to_string(),
        }
    }

    fn lock_path(&self) -> PathBuf {
        self.dir.join(format!("install-{}.lock", self.name))
    }

    fn record_path(&self) -> PathBuf {
        self.dir.join(format!("install-{}.json", self.name))
    }

    fn open_lock(&self) -> Option<File> {
        crate::support_dirs::open_private_lock(&self.lock_path()).ok()
    }

    fn read(&self) -> Option<Record> {
        let text = crate::support_dirs::read_private_text(&self.record_path(), 64 * 1024).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn write(&self, record: &Record) {
        if let Ok(bytes) = serde_json::to_vec(record) {
            if let Err(error) =
                crate::support_dirs::write_private_atomic(&self.record_path(), &bytes)
            {
                eprintln!("install {}: record not written: {error}", self.name);
            }
        }
    }

    /// Runs `install` unless `present()`. One install at a time; a caller
    /// that waited for a running install takes its result. `present` is
    /// asked again once the gate is entered: a component another process
    /// installed meanwhile is not installed twice.
    pub fn run(&self, present: &dyn Fn() -> bool, install: impl FnOnce() -> Attempt) -> Attempt {
        if present() {
            return Attempt::Installed;
        }
        let asked_ms = now_ms();
        let _local = LOCAL.lock().unwrap_or_else(PoisonError::into_inner);
        let lock = self.open_lock();
        if let Some(file) = &lock {
            if let Err(error) = file.lock() {
                eprintln!("install {}: lock not taken: {error}", self.name);
            }
        }
        let waited_for = self
            .read()
            .filter(|record| !record.running && record.at_ms >= asked_ms)
            .and_then(|record| record.ended);
        if let Some(ended) = waited_for {
            return ended;
        }
        if present() {
            return Attempt::Installed;
        }
        self.write(&Record {
            running: true,
            ended: None,
            at_ms: now_ms(),
        });
        let attempt = install();
        self.write(&Record {
            running: false,
            ended: Some(attempt.clone()),
            at_ms: now_ms(),
        });
        attempt
    }

    /// Whether an install runs, or how the last one ended. A record that
    /// says „running“ while nobody holds the lock is left from an ended
    /// process: no install runs.
    pub fn progress(&self) -> Progress {
        let Some(record) = self.read() else {
            return Progress::Idle;
        };
        if record.running {
            return if self.held() {
                Progress::Running
            } else {
                Progress::Idle
            };
        }
        record.ended.map_or(Progress::Idle, Progress::Ended)
    }

    /// The last install's refusal (declined or failed) unless an install
    /// runs or the last one succeeded.
    pub fn refusal(&self) -> Option<Attempt> {
        match self.progress() {
            Progress::Ended(attempt) if attempt != Attempt::Installed => Some(attempt),
            _ => None,
        }
    }

    fn held(&self) -> bool {
        self.open_lock()
            .is_some_and(|file| matches!(file.try_lock(), Err(TryLockError::WouldBlock)))
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{with_install, Attempt, Failure, Gate, Progress, Record};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::time::{Duration, Instant};

    /// A failing operation installs once and runs once more; anything else
    /// never installs; a refusal ends with its reason.
    #[test]
    fn install_gate_task_installs_then_retries_once() {
        // Case: missing, installed, the retry works.
        let runs = AtomicUsize::new(0);
        let installs = AtomicUsize::new(0);
        let result = with_install(
            || match runs.fetch_add(1, Ordering::SeqCst) {
                0 => Err(Failure::Missing("fehlt".into())),
                _ => Ok("läuft"),
            },
            || {
                installs.fetch_add(1, Ordering::SeqCst);
                Attempt::Installed
            },
        );
        assert_eq!(result, Ok("läuft"));
        assert_eq!(
            (runs.load(Ordering::SeqCst), installs.load(Ordering::SeqCst)),
            (2, 1)
        );

        // Case: installed but still missing: exactly one retry, its reason.
        let runs = AtomicUsize::new(0);
        let result: Result<(), String> = with_install(
            || {
                runs.fetch_add(1, Ordering::SeqCst);
                Err(Failure::Missing("fehlt weiter".into()))
            },
            || Attempt::Installed,
        );
        assert_eq!(result, Err("fehlt weiter".into()));
        assert_eq!(runs.load(Ordering::SeqCst), 2);

        // Case: another failure installs nothing.
        let installs = AtomicUsize::new(0);
        let result: Result<(), String> = with_install(
            || Err(Failure::Other("anderer Fehler".into())),
            || {
                installs.fetch_add(1, Ordering::SeqCst);
                Attempt::Installed
            },
        );
        assert_eq!(result, Err("anderer Fehler".into()));
        assert_eq!(installs.load(Ordering::SeqCst), 0);

        // Case: declined and failed installs end with their reason, no retry.
        for refusal in [
            Attempt::Declined("abgelehnt".into()),
            Attempt::Failed("Paketfehler".into()),
        ] {
            let runs = AtomicUsize::new(0);
            let result: Result<(), String> = with_install(
                || {
                    runs.fetch_add(1, Ordering::SeqCst);
                    Err(Failure::Missing("fehlt".into()))
                },
                || refusal.clone(),
            );
            assert_eq!(result, refusal.clone().result());
            assert_eq!(runs.load(Ordering::SeqCst), 1, "{refusal:?}");
        }
    }

    /// Parallel users of one component raise one install and share its
    /// result; a later use asks again; a present component installs nothing.
    #[test]
    fn install_gate_task_single_flight_shares_one_install() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let gate = Arc::new(Gate::at(path.clone(), "fixture"));
        let installs = Arc::new(AtomicUsize::new(0));
        let asked = Arc::new(AtomicUsize::new(0));
        let users = 4;
        let barrier = Arc::new(Barrier::new(users));
        let threads: Vec<_> = (0..users)
            .map(|_| {
                let (gate, installs, asked, barrier, path) = (
                    gate.clone(),
                    installs.clone(),
                    asked.clone(),
                    barrier.clone(),
                    path.clone(),
                );
                std::thread::spawn(move || {
                    barrier.wait();
                    let present = || {
                        asked.fetch_add(1, Ordering::SeqCst);
                        false
                    };
                    gate.run(&present, || {
                        installs.fetch_add(1, Ordering::SeqCst);
                        // Every user asked once, the installer twice.
                        let deadline = Instant::now() + Duration::from_secs(10);
                        while asked.load(Ordering::SeqCst) < users + 1 && Instant::now() < deadline
                        {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        std::thread::sleep(Duration::from_millis(100));
                        // Another view of the gate sees the install running.
                        let seen = Gate::at(path, "fixture").progress();
                        Attempt::Declined(format!("abgelehnt ({seen:?})"))
                    })
                })
            })
            .collect();
        let results: Vec<Attempt> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(installs.load(Ordering::SeqCst), 1, "{results:?}");
        let expected = Attempt::Declined("abgelehnt (Running)".into());
        assert!(
            results.iter().all(|result| *result == expected),
            "{results:?}"
        );
        assert_eq!(gate.progress(), Progress::Ended(expected.clone()));
        assert_eq!(gate.refusal(), Some(expected));

        // Case: a later use asks the operating system again.
        let later = gate.run(&|| false, || {
            installs.fetch_add(1, Ordering::SeqCst);
            Attempt::Installed
        });
        assert_eq!(later, Attempt::Installed);
        assert_eq!(installs.load(Ordering::SeqCst), 2);
        assert_eq!(gate.refusal(), None);

        // Case: a present component installs nothing.
        let present = gate.run(&|| true, || {
            installs.fetch_add(1, Ordering::SeqCst);
            Attempt::Failed("nie".into())
        });
        assert_eq!(present, Attempt::Installed);
        assert_eq!(installs.load(Ordering::SeqCst), 2);

        // Case: a record left „running“ by an ended process is no install.
        gate.write(&Record {
            running: true,
            ended: None,
            at_ms: 1,
        });
        assert_eq!(gate.progress(), Progress::Idle);
        assert_eq!(Gate::at(path, "other").progress(), Progress::Idle);
    }
}
