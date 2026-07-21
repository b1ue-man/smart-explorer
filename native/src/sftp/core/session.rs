use super::config::{SftpAuth, SftpConfig};
use super::io_err;
use super::known_hosts::known_hosts_accept;
use russh::client;
use russh_sftp::client::SftpSession;
use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::task::JoinSet;
use tokio::time::Instant;

const SSH_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const SSH_KEEPALIVE_MAX_MISSED: usize = 3;
const TCP_ATTEMPT_DELAY: Duration = Duration::from_millis(250);

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
    let socket = connect_tcp(&cfg.host, cfg.port).await?;
    if config.nodelay {
        let _ = socket.set_nodelay(true);
    }
    let mut session = match client::connect_stream(config, socket, handler).await {
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

async fn connect_tcp(host: &str, port: u16) -> io::Result<TcpStream> {
    let resolved = tokio::net::lookup_host((host, port)).await?;
    let addresses = interleave_addresses(resolved.collect());
    staggered_connect(addresses).await
}

fn interleave_addresses(addresses: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let mut unique = Vec::with_capacity(addresses.len());
    for address in addresses {
        if !unique.contains(&address) {
            unique.push(address);
        }
    }
    let Some(first) = unique.first() else {
        return unique;
    };

    let mut ipv4: VecDeque<_> = unique.iter().copied().filter(SocketAddr::is_ipv4).collect();
    let mut ipv6: VecDeque<_> = unique.iter().copied().filter(SocketAddr::is_ipv6).collect();
    let mut take_ipv6 = first.is_ipv6();
    let mut interleaved = Vec::with_capacity(unique.len());
    while !ipv4.is_empty() || !ipv6.is_empty() {
        let next = if take_ipv6 {
            ipv6.pop_front().or_else(|| ipv4.pop_front())
        } else {
            ipv4.pop_front().or_else(|| ipv6.pop_front())
        };
        if let Some(address) = next {
            interleaved.push(address);
        }
        take_ipv6 = !take_ipv6;
    }
    interleaved
}

async fn staggered_connect(addresses: Vec<SocketAddr>) -> io::Result<TcpStream> {
    let mut addresses = addresses.into_iter();
    let first = addresses.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "SFTP host did not resolve to an IP address",
        )
    })?;
    let mut attempts = JoinSet::new();
    spawn_connect(&mut attempts, first);
    let mut next = addresses.next();
    let mut next_launch = Instant::now() + TCP_ATTEMPT_DELAY;
    let mut last_error = None;

    loop {
        let completed = if next.is_some() {
            match tokio::time::timeout_at(next_launch, attempts.join_next()).await {
                Ok(completed) => completed,
                Err(_) => {
                    let Some(address) = next.take() else {
                        continue;
                    };
                    spawn_connect(&mut attempts, address);
                    next = addresses.next();
                    next_launch = Instant::now() + TCP_ATTEMPT_DELAY;
                    continue;
                }
            }
        } else {
            attempts.join_next().await
        };

        match completed {
            Some(Ok(Ok(stream))) => {
                attempts.abort_all();
                return Ok(stream);
            }
            Some(Ok(Err(error))) => last_error = Some(error),
            Some(Err(error)) => last_error = Some(io::Error::other(error.to_string())),
            None => {}
        }

        if attempts.is_empty() {
            if let Some(address) = next.take() {
                spawn_connect(&mut attempts, address);
                next = addresses.next();
                next_launch = Instant::now() + TCP_ATTEMPT_DELAY;
            } else {
                return Err(last_error.unwrap_or_else(|| {
                    io::Error::new(io::ErrorKind::NotConnected, "SFTP TCP connection failed")
                }));
            }
        }
    }
}

fn spawn_connect(attempts: &mut JoinSet<io::Result<TcpStream>>, address: SocketAddr) {
    attempts.spawn(async move { TcpStream::connect(address).await });
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
