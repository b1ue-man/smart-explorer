//! The intent lock is shared by foreground readers, exclusive for background
//! admission checks. A waiting foreground reader retains its shared intent until it
//! obtains the device lock, so an index cannot jump ahead between operations.
use super::{platform, probe, status, ReadMode, Source};
use sha2::{Digest, Sha256};
use std::{cell::RefCell, fs::File, io, path::{Path, PathBuf}, sync::atomic::{AtomicBool, Ordering}, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority { Foreground, Background }

thread_local! { static CANCEL: RefCell<Vec<*const AtomicBool>> = const { RefCell::new(Vec::new()) }; }

/// Gives synchronous backend reads the existing job's cancellation flag. The
/// pointer never escapes this thread or this scope; nested scopes restore it.
pub fn with_cancellation<T>(cancel: &AtomicBool, operation: impl FnOnce() -> T) -> T {
    struct Restore(usize);
    impl Drop for Restore { fn drop(&mut self) { CANCEL.with(|stack| stack.borrow_mut().truncate(self.0)); } }
    let previous = CANCEL.with(|stack| {
        let mut stack = stack.borrow_mut();
        let previous = stack.len(); stack.push(cancel); previous
    });
    let _restore = Restore(previous);
    operation()
}

pub(super) fn context_canceled() -> bool {
    CANCEL.with(|stack| stack.borrow().iter().any(|flag| {
        // SAFETY: with_cancellation owns every scoped borrow until its stack
        // entry is removed on this same thread, including nested scopes.
        unsafe { (**flag).load(Ordering::Acquire) }
    }))
}

pub struct Guard {
    source: Source,
    _device: File,
    _intent: File,
}

impl Guard {
    pub fn source(&self) -> &Source { &self.source }
    pub fn set_mode(&self, mode: ReadMode) {
        status::update(&self.source, |status| {
            status.mode = mode;
            status.block_bytes = (mode == ReadMode::Sequential).then_some(super::OPTICAL_BLOCK_BYTES);
        });
    }
    pub fn validate_file(&self, file: &File) -> io::Result<()> {
        if super::probe_file(file)?.as_ref() != Some(&self.source) { return Err(io::Error::other("held optical medium changed or was ejected")); }
        Ok(())
    }
    pub fn validate(&self, path: &Path) -> io::Result<()> {
        if probe(path)?.as_ref() != Some(&self.source) {
            return Err(io::Error::other("optical medium changed or was ejected"));
        }
        Ok(())
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        #[cfg(test)]
        super::test_profiles::leave(&self.source.device);
        status::update(&self.source, |status| { status.active = false; status.mode = ReadMode::Waiting; status.block_bytes = None; });
        let _ = platform::unlock(&self._device);
        // A background guard released intent immediately after admission.
        let _ = platform::unlock(&self._intent);
    }
}

pub fn acquire(path: &Path, priority: Priority, cancel: &AtomicBool) -> io::Result<Option<Guard>> {
    let Some(source) = probe(path)? else { return Ok(None); };
    with_cancellation(cancel, || acquire_source(path, source, priority, || cancel.load(Ordering::Acquire))).map(Some)
}

/// `cancel` and any active synchronous job scope are checked while waiting.
/// Callers with a cancellable UI lifetime must establish that scope.
pub fn acquire_source(path: &Path, source: Source, priority: Priority, cancel: impl Fn() -> bool) -> io::Result<Guard> {
    acquire_checked(source, priority, cancel, || probe(path))
}

pub(super) fn acquire_checked(source: Source, priority: Priority, cancel: impl Fn() -> bool,
    observe: impl Fn() -> io::Result<Option<Source>>) -> io::Result<Guard> {
    let dir = lock_directory()?;
    let key = format!("{:x}", Sha256::digest(source.device.as_bytes()));
    let intent = crate::support_dirs::open_private_lock(&dir.join(format!("{key}.intent")))?;
    let device = crate::support_dirs::open_private_lock(&dir.join(format!("{key}.reader")))?;
    struct Waiting<'a>(&'a Source, Priority);
    impl Drop for Waiting<'_> {
        fn drop(&mut self) { status::update(self.0, |status| match self.1 {
            Priority::Foreground => status.foreground_waiters = status.foreground_waiters.saturating_sub(1),
            Priority::Background => status.background_waiters = status.background_waiters.saturating_sub(1),
        }); }
    }
    status::update(&source, |status| match priority {
        Priority::Foreground => status.foreground_waiters += 1,
        Priority::Background => status.background_waiters += 1,
    });
    let waiting = Waiting(&source, priority);
    let mut intent_held = false;
    loop {
        if cancel() || context_canceled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "optical read canceled"));
        }
        if !intent_held { intent_held = platform::try_lock(&intent, priority == Priority::Background)?; }
        let device_held = intent_held && platform::try_lock(&device, true)?;
        if priority == Priority::Background && intent_held {
            platform::unlock(&intent)?;
            intent_held = false;
        }
        if device_held {
            if observe()?.as_ref() != Some(&source) {
                return Err(io::Error::other("optical medium changed while waiting"));
            }
            status::update(&source, |status| { status.active = true; status.mode = ReadMode::Exclusive; });
            drop(waiting);
            #[cfg(test)]
            super::test_profiles::enter(&source.device);
            return Ok(Guard { source, _device: device, _intent: intent });
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn lock_directory() -> io::Result<PathBuf> {
    let home = crate::support_dirs::home_dir().ok_or_else(|| io::Error::other("optical lock requires an OS user home"))?;
    let dir = home.join(".smart-explorer-optical-locks");
    crate::support_dirs::ensure_private_dir(&dir)?;
    Ok(dir)
}

/// Use the synchronously active job cancellation scope, if the caller has one.
pub fn acquire_current(path: &Path, priority: Priority) -> io::Result<Option<Guard>> {
    let Some(source) = probe(path)? else { return Ok(None); };
    acquire_source(path, source, priority, context_canceled).map(Some)
}

pub fn acquire_file(file: &File, priority: Priority, cancel: &AtomicBool) -> io::Result<Option<Guard>> {
    let Some(source) = super::probe_file(file)? else { return Ok(None); };
    with_cancellation(cancel, || acquire_checked(source, priority, || cancel.load(Ordering::Acquire), || super::probe_file(file))).map(Some)
}
pub fn acquire_file_current(file: &File, priority: Priority) -> io::Result<Option<Guard>> {
    let Some(source) = super::probe_file(file)? else { return Ok(None); };
    acquire_checked(source, priority, context_canceled, || super::probe_file(file)).map(Some)
}
