use std::io::Read;
use std::path::Path;

pub(crate) fn normalize_sha256(raw: &str) -> Result<String, String> {
    let token = raw.trim().to_ascii_lowercase();
    if token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(token)
    } else {
        Err("SHA-256 ungueltig".to_string())
    }
}

pub(crate) fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("Hash lesen {}: {}", path.display(), e))?;
    let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("Hash lesen {}: {}", path.display(), e))?;
        if n == 0 {
            break;
        }
        sha2::Digest::update(&mut hasher, &buf[..n]);
    }
    Ok(format!("{:x}", sha2::Digest::finalize(hasher)))
}

pub(crate) fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let expected = normalize_sha256(expected)?;
    let actual = sha256_file(path)?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "Pruefsumme passt nicht fuer {}: erwartet {}, erhalten {}",
            path.display(),
            expected,
            actual
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "smart-explorer-updater-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn verify_sha256_rejects_same_size_tamper() {
        let path = unique_temp_file("hash");
        std::fs::write(&path, b"good").unwrap();
        let expected = sha256_file(&path).unwrap();
        std::fs::write(&path, b"evil").unwrap();

        assert!(verify_sha256(&path, &expected).is_err());

        let _ = std::fs::remove_file(path);
    }
}
