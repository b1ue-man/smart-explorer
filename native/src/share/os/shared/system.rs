use std::io;

const QUARANTINE: &str = "SmartExplorer-Empfangen";

pub(crate) fn random_fingerprint() -> String {
    let mut raw = [0u8; 6];
    let _ = getrandom::getrandom(&mut raw);
    raw.iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join(":")
}

pub(crate) fn lan_ips() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(s) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if s.connect("8.8.8.8:80").is_ok() {
            if let Ok(a) = s.local_addr() {
                v.push(a.ip().to_string());
            }
        }
    }
    v
}

pub(crate) fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Gerät".to_string())
}

pub(crate) fn quarantine_dir() -> io::Result<std::path::PathBuf> {
    let base = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join(QUARANTINE);
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

pub(crate) fn unique_in(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let stem = std::path::Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = std::path::Path::new(name)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for i in 2..10_000 {
        let cand = dir.join(format!("{} ({}){}", stem, i, ext));
        if !cand.exists() {
            return cand;
        }
    }
    p
}
