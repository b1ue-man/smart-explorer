use crate::cloud::{ClientConfig, Provider};
use std::path::PathBuf;

fn cloud_dir() -> PathBuf {
    let d = crate::support_dirs::app_data_dir().join("cloud");
    d
}

fn cfg_path(p: Provider) -> PathBuf {
    cloud_dir().join(format!("{}.cfg", p.as_str()))
}

pub fn load_config(p: Provider) -> ClientConfig {
    load_config_checked(p).unwrap_or_default()
}

/// A failed read of an existing configuration must not become an empty ID.
pub fn load_config_checked(p: Provider) -> std::io::Result<ClientConfig> {
    let mut c = ClientConfig::default();
    let path = cfg_path(p);
    match crate::support_dirs::read_private_text(&path, 64 * 1024) {
        Ok(s) => {
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
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(std::io::Error::new(
                error.kind(),
                format!(
                    "Cloud-Konfiguration {} konnte nicht gelesen werden: {error}",
                    path.display()
                ),
            ));
        }
    }
    Ok(c)
}

pub fn save_config(p: Provider, c: &ClientConfig) -> std::io::Result<()> {
    if [&c.client_id, &c.client_secret].iter().any(|value| value.chars().any(char::is_control)) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,
            "Client-ID oder Secret enthält Steuerzeichen"));
    }
    let normalized = ClientConfig {
        client_id: c.client_id.trim().into(), client_secret: c.client_secret.trim().into(),
    };
    super::coordinator::config_changed(p, &normalized)
}

pub(super) fn save_config_unlocked(p: Provider, c: &ClientConfig) -> std::io::Result<()> {
    let body = format!("client_id={}\nclient_secret={}\n", c.client_id, c.client_secret);
    crate::support_dirs::write_private_atomic(&cfg_path(p), body.as_bytes())
}

/// Compatibility import: the principal is verified before the next use.
pub fn store_refresh_token(p: Provider, token: &str) -> Result<(), String> {
    super::coordinator::store_import(p, token).map_err(|error| error.to_string())
}

pub fn refresh_token_checked(p: Provider) -> Result<Option<String>, String> {
    super::coordinator::refresh_token(p).map_err(|error| error.to_string())
}

pub fn refresh_token(p: Provider) -> Option<String> { refresh_token_checked(p).ok().flatten() }

pub fn is_connected(p: Provider) -> bool { super::coordinator::status(p).signed_in }

pub fn disconnect(p: Provider) -> Result<(), String> {
    super::coordinator::disconnect(p).map_err(|error| error.to_string())
}
