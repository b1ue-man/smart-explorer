use super::config::{SftpAuth, SftpConfig};
use super::io_err;
use super::known_hosts::known_hosts_accept;
use russh::client;
use russh_sftp::client::SftpSession;
use std::io;
use std::sync::Arc;
use std::time::Duration;

const SSH_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const SSH_KEEPALIVE_MAX_MISSED: usize = 3;

pub(super) struct Client {
    host: String,
    port: u16,
    host_key_error: Arc<std::sync::Mutex<Option<String>>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        match known_hosts_accept(&self.host, self.port, server_public_key) {
            Ok(true) => Ok(true),
            Ok(false) => {
                if let Ok(mut error) = self.host_key_error.lock() {
                    *error = Some(format!(
                        "SFTP host key changed for {}:{}",
                        self.host, self.port
                    ));
                }
                Ok(false)
            }
            Err(storage_error) => {
                if let Ok(mut error) = self.host_key_error.lock() {
                    *error = Some(format!(
                        "SFTP host key could not be verified for {}:{}: {storage_error}",
                        self.host, self.port
                    ));
                }
                Ok(false)
            }
        }
    }
}

pub(super) async fn connect_async(
    cfg: &SftpConfig,
) -> io::Result<(client::Handle<Client>, SftpSession)> {
    let config = Arc::new(client_config());
    let host_key_error = Arc::new(std::sync::Mutex::new(None));
    let handler = Client {
        host: cfg.host.clone(),
        port: cfg.port,
        host_key_error: host_key_error.clone(),
    };
    let mut session = match client::connect(config, (cfg.host.as_str(), cfg.port), handler).await {
        Ok(session) => session,
        Err(error) => {
            if let Ok(mut detail) = host_key_error.lock() {
                if let Some(detail) = detail.take() {
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, detail));
                }
            }
            return Err(io_err(error));
        }
    };

    let authed = match &cfg.auth {
        SftpAuth::Password(pw) => session
            .authenticate_password(&cfg.user, pw)
            .await
            .map_err(io_err)?
            .success(),
        SftpAuth::Key { path, passphrase } => {
            let key = russh::keys::load_secret_key(path, passphrase.as_deref()).map_err(io_err)?;
            let hash = session
                .best_supported_rsa_hash()
                .await
                .map_err(io_err)?
                .flatten();
            session
                .authenticate_publickey(
                    &cfg.user,
                    russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
                .await
                .map_err(io_err)?
                .success()
        }
    };
    if !authed {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "SFTP-Authentifizierung fehlgeschlagen",
        ));
    }

    let channel = session.channel_open_session().await.map_err(io_err)?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(io_err)?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(io_err)?;
    Ok((session, sftp))
}

fn client_config() -> client::Config {
    client::Config {
        keepalive_interval: Some(SSH_KEEPALIVE_INTERVAL),
        keepalive_max: SSH_KEEPALIVE_MAX_MISSED,
        ..client::Config::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_config_enables_bounded_keepalives() {
        let config = client_config();

        assert_eq!(config.keepalive_interval, Some(Duration::from_secs(15)));
        assert_eq!(config.keepalive_max, 3);
        assert_eq!(config.inactivity_timeout, None);
    }
}
