//! The per-key connection limit is checked before a device id is bound (O-S3),
//! the global limits before a registration binds anything (E-v2-1, R2), and the
//! copies that concurrent registrations save all reach the disk
//! (SEC-SERVER-BESITZER-60S Teil 3).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use iroh_base::SecretKey;

use crate::bindings::Bindings;
use crate::limits::SourceKey;
use crate::state::{register_client_with_identity, ClientIdentity, RegistrationError, State};
use crate::Writer;

/// A private directory for one state file; removed when dropped.
struct Dir {
    path: PathBuf,
}

impl Dir {
    fn new(label: &str) -> Self {
        let nonce = crate::login::nonce().expect("random nonce");
        let path = std::env::temp_dir().join(format!(
            "se-share-state-task-{label}-{}-{}",
            std::process::id(),
            crate::login::hex(&nonce)
        ));
        std::fs::create_dir(&path).expect("temporary directory");
        Self { path }
    }

    fn file(&self) -> PathBuf {
        self.path.join("bindings.json")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn proven(seed: u8) -> ClientIdentity {
    ClientIdentity::Proven(SecretKey::from_bytes(&[seed; 32]).public())
}

/// Registers `device` from the source that `seed` names, with `identity`.
fn register(
    state: &Arc<Mutex<State>>,
    seed: u8,
    device: &str,
    identity: ClientIdentity,
) -> Result<u64, RegistrationError> {
    let (writer, _receiver) = Writer::test_channel(4);
    register_client_with_identity(
        state,
        writer,
        SourceKey::Ipv4([198, 51, 100, seed]),
        device.into(),
        HashSet::new(),
        identity,
    )
}

#[test]
fn server_rand_task_key_limit_is_checked_before_a_device_id_is_bound() {
    let dir = Dir::new("limit");
    let path = dir.file();
    let mut initial = State::default();
    initial.policy.limits.max_clients_per_key = 1;
    initial.binding_path = Some(path.clone());
    let state = Arc::new(Mutex::new(initial));
    let key = proven(71);

    assert!(register(&state, 1, "first", key.clone()).is_ok());
    // The key holds its one connection: the second device id is refused, and
    // the refusal binds and saves nothing.
    assert_eq!(
        register(&state, 2, "second", key.clone()),
        Err(RegistrationError::KeyFull)
    );
    let is_new = state
        .lock()
        .expect("state")
        .bindings
        .is_new_device("second");
    assert!(is_new, "a refused registration binds nothing");
    let saved = Bindings::load(&path, 0).expect("state file");
    assert_eq!(
        saved.device_key("second"),
        None,
        "the refusal was not saved"
    );
    assert!(
        saved.device_key("first").is_some(),
        "the accepted registration was saved"
    );

    // Another key may still take the device id.
    assert!(register(&state, 3, "second", proven(72)).is_ok());
}

#[test]
fn server_rand_task_concurrent_registrations_leave_every_binding_on_disk() {
    let dir = Dir::new("threads");
    let path = dir.file();
    let mut initial = State::default();
    initial.binding_path = Some(path.clone());
    let state = Arc::new(Mutex::new(initial));
    let workers: Vec<_> = (0_u8..6)
        .map(|index| {
            let state = state.clone();
            thread::spawn(move || {
                let device = format!("device-{index}");
                register(&state, 100 + index, &device, proven(120 + index))
            })
        })
        .collect();
    for worker in workers {
        assert!(worker.join().expect("worker").is_ok());
    }

    // The newest copy holds every binding; a restart reads them back.
    let restarted = Bindings::load(&path, 0).expect("state file");
    for index in 0_u8..6 {
        let device = format!("device-{index}");
        assert!(
            restarted.device_key(&device).is_some(),
            "device {index} is saved"
        );
    }
}

#[test]
fn server_rand_task_global_limit_refuses_before_binding_and_saving() {
    let dir = Dir::new("global");
    let path = dir.file();
    let mut initial = State::default();
    initial.policy.limits.max_clients = 1;
    initial.binding_path = Some(path.clone());
    let state = Arc::new(Mutex::new(initial));
    assert!(register(&state, 11, "first", proven(91)).is_ok());
    let before = std::fs::read(&path).expect("state file after the accepted registration");

    // The one slot is taken and no client can be displaced: the second device id is
    // refused by the global limit before it is bound, and nothing is written.
    assert_eq!(
        register(&state, 12, "second", proven(92)),
        Err(RegistrationError::Full)
    );
    let is_new = state
        .lock()
        .expect("state")
        .bindings
        .is_new_device("second");
    assert!(is_new, "a refused registration binds nothing");
    assert_eq!(std::fs::read(&path).expect("state file"), before);
}

#[test]
fn server_rand_task_a_registration_that_displaces_a_client_still_succeeds() {
    let mut initial = State::default();
    initial.policy.limits.max_clients = 1;
    let state = Arc::new(Mutex::new(initial));
    // An older client without key login takes the one slot.
    assert!(register(&state, 21, "older", ClientIdentity::Legacy).is_ok());

    // The proven registration displaces it and gets the place, as it did before.
    assert!(register(&state, 22, "proven", proven(93)).is_ok());
    let guard = state.lock().expect("state");
    assert!(!guard.bindings.is_new_device("proven"));
    assert_eq!(guard.clients.len(), 1);
}
