//! Sole launch boundary. Network callers provide an E4 admission closure here;
//! holding its guard through `dispatch` prevents a policy/launch TOCTOU gap.
use super::shared::{
    journal::Journal,
    platform::{HostPlatform, PlatformCatalog},
};
use crate::apps::core::model::*;
use crate::screen::core::app_icon::{IconRef, IconReply};
use sha2::{Digest, Sha256};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[path = "host_catalog.rs"]
mod catalog;
#[path = "host_launch.rs"]
mod launch;
#[path = "host_lifecycle.rs"]
mod lifecycle;

#[cfg(test)]
#[path = "batches_launch_task_tests.rs"]
mod batches_launch_task_tests;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PeerRequestBinding {
    version: u8,
    owner: String,
    request: LaunchRequest,
}

pub(crate) struct CatalogIcons {
    pub references: fn(&mut [CatalogApp]),
    pub content: fn(&CatalogApp, &IconRef) -> io::Result<Option<IconReply>>,
}
/// Main supplies the actual E4 grant end, alongside its policy/revocation guard.
/// `None` means the authoritative grant has no deadline, not "not checked".
pub(crate) struct LaunchAdmission<G> {
    pub guard: G,
    pub expires_at_unix_ms: Option<i64>,
    pub valid: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}

pub(crate) struct AppHost<P: HostPlatform = super::platform::Platform> {
    device: DeviceId,
    platform: P,
    journal: Journal,
    serial: Mutex<()>,
    icons: Option<CatalogIcons>,
    ownership_root: PathBuf,
    windows: Mutex<super::shared::window_correlation::Correlations>,
}
impl AppHost<super::platform::Platform> {
    pub(crate) fn for_current_user(
        device: DeviceId,
        root: &Path,
        icons: Option<CatalogIcons>,
    ) -> io::Result<Self> {
        let host = Self::new(device, super::platform::Platform::default(), root)?;
        Ok(if let Some(icons) = icons {
            host.with_icons(icons)
        } else {
            host
        })
    }
}
pub(crate) fn local_consent_supported() -> bool {
    cfg!(target_os = "android")
}
pub(crate) fn install_platform_hooks(
    result: fn(LaunchRequest, String, super::shared::platform::DispatchReceipt),
    consent: fn(LaunchRequest),
) -> io::Result<()> {
    #[cfg(target_os = "android")]
    {
        super::platform::bridge::set_result_sink(result)?;
        super::platform::bridge::set_consent_sink(consent)?;
    }
    #[cfg(not(target_os = "android"))]
    let _ = (result, consent);
    Ok(())
}
pub(crate) fn end_platform_consent(request: &LaunchRequest, reason: &str) {
    #[cfg(target_os = "android")]
    super::platform::bridge::end_local_consent(request, reason);
    #[cfg(not(target_os = "android"))]
    let _ = (request, reason);
}
pub(crate) fn claim_local_admission(dispatch: &str, request: &str) -> io::Result<(i64, u64)> {
    #[cfg(target_os = "android")]
    {
        let permit = super::platform::bridge::claim_admission(dispatch, request)?;
        return Ok((permit.expires_at_unix_ms, permit.valid_for_ms));
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (dispatch, request);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "local platform admission is unavailable",
        ))
    }
}
pub(crate) fn finish_local_icon(challenge: &str, reply: Option<IconReply>) -> io::Result<()> {
    #[cfg(target_os = "android")]
    {
        return super::platform::bridge::finish_icon(challenge, reply);
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (challenge, reply);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "local icon bridge is unavailable",
        ))
    }
}
impl<P: HostPlatform> AppHost<P> {
    pub(crate) fn new(device: DeviceId, platform: P, ledger_dir: &Path) -> io::Result<Self> {
        Ok(Self {
            device,
            platform,
            journal: Journal::new(ledger_dir)?,
            serial: Mutex::new(()),
            icons: None,
            ownership_root: ledger_dir.to_owned(),
            windows: Mutex::new(Default::default()),
        })
    }
    /// A stable request ID has one exact authenticated peer owner for its whole
    /// durable lifetime. A caller cannot obtain an unowned or foreign receipt.
    pub(crate) fn bind_peer_owner(
        &self,
        id: &RequestId,
        owner: &str,
        claim: bool,
    ) -> io::Result<bool> {
        use std::io::{Read, Write};
        if owner.len() != 64 || !owner.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("invalid peer ownership digest"));
        }
        let path = self
            .ownership_root
            .join(format!("{:x}.peer", Sha256::digest(id.as_str().as_bytes())));
        let read = || -> io::Result<Option<bool>> {
            let mut file = match std::fs::File::open(&path) {
                Ok(f) => f,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
            };
            if !file.metadata()?.is_file()
                || std::fs::symlink_metadata(&path)?.file_type().is_symlink()
            {
                return Err(invalid("invalid peer ownership file"));
            }
            let mut bytes = Vec::new();
            std::io::Read::by_ref(&mut file)
                .take(65)
                .read_to_end(&mut bytes)?;
            Ok(Some(bytes == owner.as_bytes()))
        };
        if let Some(same) = read()? {
            return Ok(same);
        }
        if !claim {
            return Ok(false);
        }
        let mut file = tempfile::NamedTempFile::new_in(&self.ownership_root)?;
        file.write_all(owner.as_bytes())?;
        file.as_file().sync_all()?;
        if super::platform::publish_record(file, &path, false)? {
            super::platform::sync_journal(&self.ownership_root)?;
            return Ok(true);
        }
        Ok(read()?.unwrap_or(false))
    }
    pub(crate) fn bind_peer_request(
        &self,
        request: &LaunchRequest,
        owner: &str,
        claim: bool,
    ) -> io::Result<bool> {
        use std::io::{Read, Write};
        if owner.len() != 64 || !owner.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("invalid peer ownership digest"));
        }
        let hash = format!(
            "{:x}",
            Sha256::digest(request.request_id.as_str().as_bytes())
        );
        let path = self.ownership_root.join(format!("{hash}.peer-request-v1"));
        let read = || -> io::Result<Option<bool>> {
            let file = match std::fs::File::open(&path) {
                Ok(file) => file,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
            };
            if !file.metadata()?.is_file()
                || std::fs::symlink_metadata(&path)?.file_type().is_symlink()
            {
                return Err(invalid("invalid peer request binding file"));
            }
            let mut bytes = Vec::new();
            file.take(32769).read_to_end(&mut bytes)?;
            if bytes.len() > 32768 {
                return Err(invalid("peer request binding exceeds byte budget"));
            }
            let bound: PeerRequestBinding =
                serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            if bound.version != 1 {
                return Err(invalid("unsupported peer request binding version"));
            }
            Ok(Some(bound.owner == owner && bound.request == *request))
        };
        let existing = read()?;
        if existing == Some(false) {
            return Ok(false);
        }
        if existing.is_none() {
            let legacy_owner =
                std::fs::symlink_metadata(self.ownership_root.join(format!("{hash}.peer"))).is_ok();
            let legacy_body = self.journal.bound_request(&request.request_id)?;
            if legacy_owner {
                if !self.bind_peer_owner(&request.request_id, owner, false)? {
                    return Ok(false);
                }
                if legacy_body.as_ref() != Some(request) {
                    return Err(invalid(
                        "legacy app request owner has no matching durable body",
                    ));
                }
            } else if legacy_body.is_some() || !claim {
                return Ok(false);
            }
            // Owner and complete body have one atomic no-replace publication.
            // A crash cannot leave an ID owned but accepting a changed request.
            let bytes = serde_json::to_vec(&PeerRequestBinding {
                version: 1,
                owner: owner.into(),
                request: request.clone(),
            })
            .map_err(io::Error::other)?;
            if bytes.len() > 32768 {
                return Err(invalid("peer request binding exceeds byte budget"));
            }
            let mut stage = tempfile::NamedTempFile::new_in(&self.ownership_root)?;
            stage.write_all(&bytes)?;
            stage.as_file().sync_all()?;
            if super::platform::publish_record(stage, &path, false)? {
                super::platform::sync_journal(&self.ownership_root)?;
            }
            if read()? != Some(true) {
                return Ok(false);
            }
        }
        // Maintain the existing local bridge/host journal readers only after
        // the atomic owner/body record has established their authority.
        if !self.bind_peer_owner(&request.request_id, owner, true)? {
            return Ok(false);
        }
        self.journal.bind(request)
    }
}
