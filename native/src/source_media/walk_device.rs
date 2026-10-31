//! Device facts of one walked folder. A walk decides the device of its root
//! once and decides again only where a folder lies on another device than its
//! parent. A folder on no protected optical device then takes neither the
//! device lock nor a probe; a protected one is listed under the lock that
//! `GuardedIterator` takes, unchanged.
use super::{acquire_source, probe, with_cancellation, Guard, Priority, Source};
use std::{
    io,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

/// The device a folder lies on, as far as the platform tells without a probe.
/// `device` is `None` where the platform has no cheap device number (Windows,
/// unsupported systems); there a walk leaves its root's volume only through a
/// link, which `WalkDevice::child` probes.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DeviceKey {
    device: Option<u64>,
    /// Synthetic device of a test profile; production has none.
    #[cfg(test)]
    profile: Option<std::path::PathBuf>,
}

#[cfg(not(test))]
fn key_of(path: &Path) -> io::Result<DeviceKey> {
    Ok(DeviceKey {
        device: super::platform::device_number(path)?,
    })
}

#[cfg(test)]
fn key_of(path: &Path) -> io::Result<DeviceKey> {
    Ok(DeviceKey {
        device: super::platform::device_number(path)?,
        profile: super::test_profiles::root_of(path),
    })
}

/// The device of one folder of a walk: its identity and its protected source
/// (`None` when the device is not protected).
#[derive(Clone, Debug)]
pub struct WalkDevice {
    key: DeviceKey,
    source: Option<Arc<Source>>,
}

impl WalkDevice {
    /// The device of a walk's root folder: one probe.
    pub fn root(path: &Path) -> io::Result<Self> {
        Ok(Self {
            key: key_of(path)?,
            source: probe(path)?.map(Arc::new),
        })
    }

    /// The device of `path`, a folder entered from a folder on `self`'s device.
    /// `links_followed`: the walk may enter links, which can lead to another
    /// device; where the platform has no device number, such a folder is probed.
    pub fn child(&self, path: &Path, links_followed: bool) -> io::Result<Self> {
        let key = key_of(path)?;
        let same_device = key == self.key && (key.device.is_some() || !links_followed);
        if same_device {
            return Ok(Self {
                key,
                source: self.source.clone(),
            });
        }
        Ok(Self {
            key,
            source: probe(path)?.map(Arc::new),
        })
    }

    /// The protected source of this folder's device; `None` when the device is
    /// not protected.
    pub fn protected_source(&self) -> Option<Source> {
        self.source.as_deref().cloned()
    }

    /// The device lock for one step in this folder (an entry's metadata read):
    /// taken on a protected device, none on a folder on no protected device.
    pub fn guard(
        &self,
        path: &Path,
        priority: Priority,
        cancel: &AtomicBool,
    ) -> io::Result<Option<Guard>> {
        let Some(source) = self.protected_source() else {
            return Ok(None);
        };
        with_cancellation(cancel, || {
            acquire_source(path, source, priority, || cancel.load(Ordering::Acquire))
        })
        .map(Some)
    }
}
