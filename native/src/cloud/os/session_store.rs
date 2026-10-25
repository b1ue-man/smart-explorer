//! One secure-store record is the atomic authority for identity and generation.
use crate::cloud::auth_state::Session;
use crate::cloud::{AuthError, AuthState, Provider};
use std::fs::File;
use std::sync::{Mutex, MutexGuard};

static TRANSACTION: Mutex<()> = Mutex::new(());

pub(super) struct Guard {
    _thread: MutexGuard<'static, ()>,
    _file: File,
}

pub(super) fn lock(provider: Provider) -> Result<Guard, AuthError> {
    let thread = TRANSACTION.lock().map_err(|_| storage_error())?;
    let dir = crate::support_dirs::app_data_dir().join("cloud");
    crate::support_dirs::ensure_private_dir(&dir).map_err(|_| storage_error())?;
    let file = crate::support_dirs::open_private_lock(&dir.join(format!("{}.auth.lock", provider.as_str())))
        .map_err(|_| storage_error())?;
    file.lock().map_err(|_| storage_error())?;
    Ok(Guard { _thread: thread, _file: file })
}

fn key(provider: Provider) -> String { format!("cloud:session:v1:{}", provider.as_str()) }
pub(super) fn legacy_key(provider: Provider) -> String { format!("cloud:{}", provider.as_str()) }
pub(super) fn storage_error() -> AuthError { AuthError::new(AuthState::StorageError) }

pub(super) fn read(provider: Provider) -> Result<Option<Session>, AuthError> {
    let raw = crate::creds::get_secret_checked(&key(provider)).map_err(|_| storage_error())?;
    let Some(raw) = raw else { return Ok(None) };
    let session: Session = serde_json::from_str(&raw).map_err(|_| storage_error())?;
    if session.version != 1 || session.provider != provider.as_str() || session.generation == 0 {
        return Err(storage_error());
    }
    Ok(Some(session))
}

pub(super) fn read_legacy(provider: Provider) -> Result<Option<String>, AuthError> {
    crate::creds::get_secret_checked(&legacy_key(provider)).map_err(|_| storage_error())
}

pub(super) fn write(provider: Provider, session: &Session, tracked: bool) -> Result<(), AuthError> {
    let raw = serde_json::to_string(session).map_err(|_| storage_error())?;
    let result = if tracked { crate::creds::set_secret(&key(provider), &raw) }
        else { crate::creds::set_secret_untracked(&key(provider), &raw) };
    result.map_err(|_| storage_error())
}

pub(super) fn empty(provider: Provider, client_id: &str) -> Session {
    Session { version: 1, provider: provider.as_str().into(), client_id: client_id.into(), ..Session::default() }
}

#[cfg(test)]
pub(crate) struct TestSessionSnapshot {
    session: Option<String>,
    legacy: Option<String>,
}

#[cfg(test)]
pub(crate) fn test_snapshot(provider: Provider) -> Result<TestSessionSnapshot, String> {
    let _guard = lock(provider).map_err(|error| error.to_string())?;
    Ok(TestSessionSnapshot {
        session: crate::creds::get_secret_checked(&key(provider))?,
        legacy: crate::creds::get_secret_checked(&legacy_key(provider))?,
    })
}

#[cfg(test)]
pub(crate) fn test_restore(provider: Provider, snapshot: TestSessionSnapshot) -> Result<(), String> {
    let _guard = lock(provider).map_err(|error| error.to_string())?;
    for (key, value) in [(key(provider), snapshot.session), (legacy_key(provider), snapshot.legacy)] {
        match value {
            Some(value) => crate::creds::set_secret(&key, &value)?,
            None => crate::creds::delete_secret_checked(&key)?,
        }
    }
    super::coordinator::test_clear_runtime_failure();
    Ok(())
}

#[cfg(test)]
pub(crate) fn test_reset(provider: Provider) -> Result<(), String> {
    let _guard = lock(provider).map_err(|error| error.to_string())?;
    crate::creds::delete_secret_checked(&key(provider))?;
    crate::creds::delete_secret_checked(&legacy_key(provider))?;
    super::coordinator::test_clear_runtime_failure();
    Ok(())
}
