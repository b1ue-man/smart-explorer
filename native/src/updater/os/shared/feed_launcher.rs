//! Optional legacy feed detection never bypasses a present launcher's hash.
use super::{
    format_http_error, os, parse_ver, Feed, VerifiedPayload, HTTP_TIMEOUT, UPDATE_USER_AGENT,
};

impl Feed {
    pub(in crate::updater) fn fetch_launcher_exe(
        &self,
        version: &str,
    ) -> Result<Option<VerifiedPayload>, String> {
        let spec = os::launcher_payload_spec();
        if let Some(hash) = self.read_sha256(spec.hash_name)? {
            return self.fetch_launcher_bound(version, &hash).map(Some);
        }
        // Old published bundles have neither file nor sidecar. A development
        // bundle with the same version may carry both; its verified runtime
        // capability probe, not its version number, establishes compatibility.
        if parse_ver(version) > (0, 5, 184) || self.launcher_present(&spec)? {
            return Err(format!(
                "Update-Feed {} enthaelt keine erforderliche Pruefsumme {}",
                self.display(),
                spec.hash_name,
            ));
        }
        Ok(None)
    }

    /// A unique stage is owned before its first write, including hash/download
    /// failures. The historical bridge can additionally pin this sidecar.
    fn fetch_launcher_bound(&self, version: &str, hash: &str) -> Result<VerifiedPayload, String> {
        use crate::updater::{
            core::{staged_payload_path, verify_sha256},
            staging::StagingGuard,
        };
        let spec = os::launcher_payload_spec();
        let dest = staged_payload_path("launcher_download", version, hash);
        let mut stage = StagingGuard::new();
        stage.track_path(&dest);
        match self {
            Feed::Local(root) => {
                let source = spec
                    .local_names
                    .iter()
                    .map(|name| root.join(name))
                    .find(|path| path.exists())
                    .ok_or_else(|| {
                        format!("Keine MCP-Starterdatei im Feed {} gefunden", root.display())
                    })?;
                std::fs::copy(&source, &dest).map_err(|e| format!("MCP-Starter stagen: {e}"))?;
            }
            Feed::Http(base) => {
                let mut failure = String::new();
                let mut downloaded = false;
                for name in spec.http_names {
                    match super::http_download(&format!("{base}/{name}"), &dest) {
                        Ok(()) => {
                            downloaded = true;
                            break;
                        }
                        Err(error) => failure = error,
                    }
                }
                if !downloaded {
                    return Err(format!("MCP-Starter herunterladen: {failure}"));
                }
            }
        }
        verify_sha256(&dest, hash)?;
        os::mark_staged_executable(&dest)?;
        let payload = VerifiedPayload::new(dest, hash.into())?;
        stage.disarm();
        Ok(payload)
    }

    fn launcher_present(&self, spec: &super::PayloadSpec) -> Result<bool, String> {
        match self {
            Feed::Local(directory) => {
                for name in spec.local_names {
                    let path = directory.join(name);
                    match std::fs::symlink_metadata(&path) {
                        Ok(_) => return Ok(true),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            return Err(format!("Starterdatei {}: {error}", path.display()))
                        }
                    }
                }
            }
            Feed::Http(base) => {
                for name in spec.http_names {
                    let url = format!("{base}/{name}");
                    match ureq::get(&url)
                        .set("User-Agent", UPDATE_USER_AGENT)
                        .timeout(HTTP_TIMEOUT)
                        .call()
                    {
                        // No unverified response body is staged or executed.
                        Ok(_) => return Ok(true),
                        Err(ureq::Error::Status(404, _)) => {}
                        Err(error) => return Err(format_http_error(&url, error)),
                    }
                }
            }
        }
        Ok(false)
    }
}
