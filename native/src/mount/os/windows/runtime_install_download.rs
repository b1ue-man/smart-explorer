use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use super::{DOKANY_API_VERSION, DOKANY_DRIVER_PROTOCOL_VERSION};

const PINNED_MANIFEST: &str = include_str!("../../../../dokany-runtime.nsh");
const RELEASE_ASSET_HOST: &str = "https://release-assets.githubusercontent.com/";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);

pub(super) struct PinnedMsi {
    pub(super) version: &'static str,
    pub(super) url: &'static str,
    pub(super) size: u64,
    pub(super) sha256: &'static str,
}

pub(super) struct MsiArtifact {
    path: PathBuf,
    remove_on_drop: bool,
}

impl MsiArtifact {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for MsiArtifact {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub(super) fn pinned_msi() -> Result<PinnedMsi, String> {
    let version = manifest_value("DOKANY_VERSION")?;
    let api = manifest_value("DOKANY_API_VERSION")?;
    let driver_protocol = manifest_value("DOKANY_DRIVER_PROTOCOL_VERSION")?;
    let file_name = manifest_value("DOKANY_MSI_FILENAME")?;
    let url = manifest_value("DOKANY_MSI_URL")?;
    let size = manifest_value("DOKANY_MSI_SIZE")?
        .parse::<u64>()
        .map_err(|_| "Dokany-Manifest enthaelt keine gueltige MSI-Groesse".to_string())?;
    let sha256 = manifest_value("DOKANY_MSI_SHA256")?;

    let expected_url =
        format!("https://github.com/dokan-dev/dokany/releases/download/v{version}/{file_name}");
    if api.parse::<u32>().ok() != Some(DOKANY_API_VERSION)
        || driver_protocol.parse::<u32>().ok() != Some(DOKANY_DRIVER_PROTOCOL_VERSION)
        || version.split('.').count() != 4
        || !version.split('.').all(|part| part.parse::<u32>().is_ok())
        || file_name != "Dokan_x64.msi"
        || url != expected_url
        || size == 0
        || sha256.len() != 64
        || !sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("eingebettetes Dokany-Abhaengigkeitsmanifest ist ungueltig".into());
    }
    Ok(PinnedMsi {
        version,
        url,
        size,
        sha256,
    })
}

pub(super) fn acquire_msi(local: Option<&Path>, pinned: &PinnedMsi) -> Result<MsiArtifact, String> {
    if let Some(path) = local {
        return Ok(MsiArtifact {
            path: absolute_path(path)?,
            remove_on_drop: false,
        });
    }

    let directory = std::env::temp_dir()
        .join("smart-explorer")
        .join("runtime-install");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Dokany-Downloadordner anlegen: {error}"))?;
    let mut temporary = tempfile::Builder::new()
        .prefix("dokany-")
        .suffix(".msi")
        .tempfile_in(&directory)
        .map_err(|error| format!("sichere Dokany-Temp-Datei anlegen: {error}"))?;

    let response = ureq::AgentBuilder::new()
        .https_only(true)
        .redirects(2)
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .get(pinned.url)
        .set("User-Agent", "SmartExplorer-Dokany-Installer/1")
        .call()
        .map_err(|error| format!("Dokany {} herunterladen: {error}", pinned.version))?;
    if response.status() != 200 || !trusted_final_url(response.get_url(), pinned.url) {
        return Err(format!(
            "Dokany-Download endete an einer nicht freigegebenen HTTPS-Adresse ({})",
            response.get_url()
        ));
    }
    if let Some(length) = response.header("Content-Length") {
        let length = length
            .parse::<u64>()
            .map_err(|_| "Dokany-Download meldet eine ungueltige Groesse".to_string())?;
        if length != pinned.size {
            return Err(format!(
                "Dokany-Downloadgroesse stimmt nicht (erwartet {}, gemeldet {length})",
                pinned.size
            ));
        }
    }

    let mut bounded = response.into_reader().take(pinned.size.saturating_add(1));
    let written = std::io::copy(&mut bounded, temporary.as_file_mut())
        .map_err(|error| format!("Dokany-MSI speichern: {error}"))?;
    if written != pinned.size {
        return Err(format!(
            "Dokany-Downloadgroesse stimmt nicht (erwartet {}, empfangen {written})",
            pinned.size
        ));
    }
    temporary
        .as_file_mut()
        .sync_all()
        .map_err(|error| format!("Dokany-MSI dauerhaft schreiben: {error}"))?;
    let (file, path) = temporary
        .keep()
        .map_err(|error| format!("Dokany-MSI fuer Installation bereitstellen: {error}"))?;
    drop(file);
    Ok(MsiArtifact {
        path,
        remove_on_drop: true,
    })
}

fn trusted_final_url(actual: &str, original: &str) -> bool {
    actual == original || actual.starts_with(RELEASE_ASSET_HOST)
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .map_err(|error| format!("Dokany-MSI-Pfad aufloesen: {error}"))
    }
}

fn manifest_value(name: &str) -> Result<&'static str, String> {
    let mut found = PINNED_MANIFEST.lines().filter_map(|line| {
        let rest = line.trim().strip_prefix("!define")?.trim_start();
        let rest = rest.strip_prefix(name)?;
        if rest
            .chars()
            .next()
            .is_some_and(|character| !character.is_whitespace())
        {
            return None;
        }
        let quoted = rest.trim_start().strip_prefix('"')?;
        let (value, tail) = quoted.split_once('"')?;
        tail.trim().is_empty().then_some(value)
    });
    let value = found
        .next()
        .ok_or_else(|| format!("Dokany-Manifestwert {name} fehlt"))?;
    if found.next().is_some() {
        return Err(format!("Dokany-Manifestwert {name} ist mehrfach definiert"));
    }
    Ok(value)
}
