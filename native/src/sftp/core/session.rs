use super::config::{SftpAuth, SftpConfig};
use super::io_err;
use super::known_hosts::known_hosts_accept;
use russh::client;
use russh_sftp::client::SftpSession;
use std::io;
use std::sync::Arc;

pub(super) struct Client {
    host: String,
    port: u16,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(known_hosts_accept(&self.host, self.port, server_public_key))
    }
}

pub(super) async fn connect_async(
    cfg: SftpConfig,
) -> io::Result<(client::Handle<Client>, SftpSession)> {
    let config = Arc::new(client::Config::default());
    let handler = Client {
        host: cfg.host.clone(),
        port: cfg.port,
    };
    let mut session = client::connect(config, (cfg.host.as_str(), cfg.port), handler)
        .await
        .map_err(io_err)?;

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
