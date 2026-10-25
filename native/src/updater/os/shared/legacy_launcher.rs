//! Complete the historical three-payload caller without mixing feed releases.
use std::path::{Path,PathBuf};
use super::{Feed,classify_feed,github_repo,os};
use crate::updater::{config,staging::StagingGuard,types::VerifiedPayload};

pub struct LegacyLauncherRequest<'a> {
    pub installation_root:&'a Path,
    pub version:&'a str,
    pub cli_staged:&'a Path,
    pub app_sha256:&'a str,
    pub cli_sha256:&'a str,
    pub helper_sha256:&'a str,
}
/// Keeps the extra staged artifact owned until the helper's apply returns.
/// No install manifest or existing staged bundle is rewritten by this bridge.
pub struct CompletedLegacyLauncher {
    payload:VerifiedPayload,
    boundary:FeedBoundary,
    _staging:StagingGuard,
}
impl CompletedLegacyLauncher {
    pub fn payload(&self)->&VerifiedPayload {&self.payload}
    /// Recheck after waiting for the old app/another helper, before Prepare.
    pub fn revalidate(&self)->Result<(),String> {
        self.boundary.check(Some(self.payload.sha256())).map(|_|())
    }
}
struct FeedBoundary {
    root:PathBuf,source:String,version:String,
    app_sha256:String,cli_sha256:String,helper_sha256:String,
}

pub fn complete_legacy_launcher(request:&LegacyLauncherRequest<'_>)->Result<Option<CompletedLegacyLauncher>,String> {
    validate_version(request.version)?;
    for hash in [request.app_sha256,request.cli_sha256,request.helper_sha256] {validate_hash(hash)?;}
    if !crate::mcp_launcher::staged_runtime_managed(request.cli_staged,request.cli_sha256,request.version)? {
        return Ok(None);
    }
    let source=configured_source(request.installation_root)?;
    let boundary=FeedBoundary {root:request.installation_root.into(),source,version:request.version.into(),app_sha256:request.app_sha256.into(),cli_sha256:request.cli_sha256.into(),helper_sha256:request.helper_sha256.into()};
    let launcher_sha256=boundary.check(None)?;
    let feed=boundary.feed();
    let mut staging=StagingGuard::new();
    let payload=feed.fetch_launcher_exe(request.version)?.ok_or_else(||format!("legacy_launcher_required: managed runtime {} requires a verified launcher from {}",request.version,feed.display()))?;
    staging.track(&payload);
    if !payload.sha256().eq_ignore_ascii_case(&launcher_sha256) {return Err("legacy_launcher_feed_changed: launcher hash changed during download".into());}
    crate::mcp_launcher::verify_staged_launcher(payload.path(),payload.sha256(),request.version)?;
    boundary.check(Some(payload.sha256()))?;
    Ok(Some(CompletedLegacyLauncher {payload,boundary,_staging:staging}))
}
impl FeedBoundary {
    fn feed(&self)->Feed {
        // Only the official repository's exact release assets replace a moving
        // branch feed. Custom Local/HTTP feeds keep their configured source.
        if let Some((owner,repo))=github_repo(&self.source) {
            if owner.eq_ignore_ascii_case("b1ue-man") && repo.eq_ignore_ascii_case("smart-explorer") {
                return Feed::Http(format!("https://github.com/{owner}/{repo}/releases/download/v{}",self.version));
            }
        }
        classify_feed(&self.source)
    }
    fn check(&self,launcher_sha256:Option<&str>)->Result<String,String> {
        if configured_source(&self.root)?!=self.source {return Err("legacy_launcher_feed_changed: configured source changed".into());}
        let feed=self.feed();
        self.check_version(&feed)?;
        for (name,expected) in [
            (os::app_payload_spec().hash_name,self.app_sha256.as_str()),
            (os::cli_payload_spec().hash_name,self.cli_sha256.as_str()),
            (os::updater_payload_spec().hash_name,self.helper_sha256.as_str()),
        ] {
            let actual=feed.read_sha256_required(name)?;
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(format!("legacy_launcher_release_mismatch: {} {name} has SHA-256 {actual}, expected {expected} for {}",feed.display(),self.version));
            }
        }
        let actual=feed.read_sha256_required(os::launcher_payload_spec().hash_name)?;
        if launcher_sha256.is_some_and(|expected|!actual.eq_ignore_ascii_case(expected)) {return Err("legacy_launcher_feed_changed: launcher sidecar changed".into());}
        self.check_version(&feed)?;
        if configured_source(&self.root)?!=self.source {return Err("legacy_launcher_feed_changed: configured source changed".into());}
        Ok(actual)
    }
    fn check_version(&self,feed:&Feed)->Result<(),String> {
        let actual=feed.read_version()?;
        if actual!=self.version {return Err(format!("legacy_launcher_release_mismatch: {} offers version {actual}, expected {}",feed.display(),self.version));}
        Ok(())
    }
}
fn configured_source(root:&Path)->Result<String,String> {
    // Match the original installed app's precedence. The detached helper's
    // own stage directory is not the original executable's source directory.
    for path in [config::appdata_dir().join("update_source.txt"),root.join("update_source.txt")] {
        match std::fs::read_to_string(&path) {
            Ok(raw)=>{if let Some(source)=raw.lines().next().map(str::trim).filter(|value|!value.is_empty()) {return Ok(source.into());}},
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
            Err(error)=>return Err(format!("legacy_launcher_source_unreadable: {}: {error}",path.display())),
        }
    }
    Err("legacy_launcher_source_missing: the managed runtime requires the original configured update feed".into())
}
fn validate_hash(hash:&str)->Result<(),String> {
    if hash.len()!=64 || !hash.bytes().all(|byte|byte.is_ascii_hexdigit()) {return Err("legacy_launcher_expected_hash_invalid".into());}Ok(())
}
fn validate_version(version:&str)->Result<(),String> {
    if version.is_empty() || version.len()>64 || !version.bytes().all(|byte|byte.is_ascii_alphanumeric()||b".-".contains(&byte)) {
        return Err("legacy_launcher_expected_version_invalid".into());
    }
    Ok(())
}
