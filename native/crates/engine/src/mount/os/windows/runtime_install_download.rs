use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{DOKANY_API_VERSION, DOKANY_DRIVER_PROTOCOL_VERSION};

const PINNED_MANIFEST: &str = include_str!("../../../../../../dokany-runtime.nsh");
const RELEASE_ASSET_HOST: &str = "https://release-assets.githubusercontent.com/";

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

/// The pinned MSI: `local` when given, else downloaded into `directory`
/// (`runtime_install.rs` names it: the temp folder of a process with the
/// account's environment, the app data folder for the drive host).
pub(super) fn acquire_msi(
    local: Option<&Path>,
    pinned: &PinnedMsi,
    directory: &Path,
) -> Result<MsiArtifact, String> {
    if let Some(path) = local {
        return Ok(MsiArtifact {
            path: absolute_path(path)?,
            remove_on_drop: false,
        });
    }

    fs::create_dir_all(directory)
        .map_err(|error| format!("Dokany-Downloadordner anlegen: {error}"))?;
    let mut temporary = tempfile::Builder::new()
        .prefix("dokany-")
        .suffix(".msi")
        .tempfile_in(directory)
        .map_err(|error| format!("sichere Dokany-Temp-Datei anlegen: {error}"))?;

    // No limit on the whole transfer (user rule 2026-11-17): `http_resume`
    // limits the connect and each read, continues a broken transfer with a
    // range request, refuses another final address than the pinned one or
    // GitHub's release asset host, and takes no byte beyond the pinned size.
    let source = crate::http_resume::Source {
        url: pinned.url,
        user_agent: "SmartExplorer-Dokany-Installer/1",
        https_only: true,
        redirects: 2,
        final_url: &|actual| trusted_final_url(actual, pinned.url),
    };
    let bound = crate::http_resume::Bound {
        exact: Some(pinned.size),
        ceiling: None,
    };
    let written = crate::http_resume::fetch(
        &source,
        bound,
        temporary.as_file_mut(),
        &mut |_, _| {},
        &|| false,
    )
    .map_err(|error| format!("Dokany {} herunterladen: {error}", pinned.version))?;
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
