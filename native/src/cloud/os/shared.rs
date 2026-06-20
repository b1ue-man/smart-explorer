use crate::cloud::{ClientConfig, Provider};
use std::path::PathBuf;

fn cloud_dir() -> PathBuf {
    let d = crate::support_dirs::app_data_dir().join("cloud");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn cfg_path(p: Provider) -> PathBuf {
    cloud_dir().join(format!("{}.cfg", p.as_str()))
}

pub fn load_config(p: Provider) -> ClientConfig {
    let mut c = ClientConfig::default();
    if let Ok(s) = std::fs::read_to_string(cfg_path(p)) {
        for line in s.lines() {
            if let Some((k, v)) = line.split_once('=') {
                match k.trim() {
                    "client_id" => c.client_id = v.trim().to_string(),
                    "client_secret" => c.client_secret = v.trim().to_string(),
                    _ => {}
                }
            }
        }
    }
    c
}

pub fn save_config(p: Provider, c: &ClientConfig) -> std::io::Result<()> {
    let body = format!(
        "client_id={}\nclient_secret={}\n",
        c.client_id.trim(),
        c.client_secret.trim()
    );
    std::fs::write(cfg_path(p), body)
}

fn keyring_account(p: Provider) -> String {
    format!("cloud:{}", p.as_str())
}

/// Persist the long-lived refresh token (keyring).
pub fn store_refresh_token(p: Provider, token: &str) {
    let _ = crate::creds::set_secret(&keyring_account(p), token);
}

pub fn refresh_token(p: Provider) -> Option<String> {
    crate::creds::get_secret(&keyring_account(p))
}

pub fn is_connected(p: Provider) -> bool {
    refresh_token(p).map(|t| !t.is_empty()).unwrap_or(false)
}

pub fn disconnect(p: Provider) {
    crate::creds::delete_secret(&keyring_account(p));
}
