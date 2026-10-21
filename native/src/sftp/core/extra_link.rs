//! One extra SSH connection for file transfers (security answer E13-S15).
//! It only ever opens the subsystem `sftp`: no exec, no shell, no PTY, no
//! forwarding, and it refuses every channel the server opens on its own. It
//! is set up on a thread of its own, one at a time per host and port, and
//! lives as long as its single pool channel (60 s idle at most) or the
//! backend. Before the login it must show the host key the verified main
//! connection showed (`same_host_key`); the login is the stored password or
//! the main login's decrypted key, never the SSH agent and never a prompt.
use super::super::connection::SftpGeneration;
use super::super::host_key::UnknownHostKey;
use super::super::known_hosts;
use super::super::link_log::{self, LinkIdent};
use super::super::link_rules::{same_host_key, ExtraLogin, LoginSlot};
use super::{auth, client_config, connect_tcp};
use crate::transfer::MemoryReservation;
use russh::client::{self, ChannelOpenHandle, Session};
use russh::keys::PublicKeyOrCertificate;
use russh::Channel;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// Everything from the TCP connection to the opened channel.
const EXTRA_DEADLINE: Duration = Duration::from_secs(30);
const DEFAULT_REASON: &str = "Backend beendet";

/// The handler of an extra connection: the host-key check and the refusal of
/// every channel the server opens (as `Client`, host_key_handler.rs).
pub(in crate::sftp) struct ExtraClient {
    host: String,
    port: u16,
    main: UnknownHostKey,
    failure: Arc<Mutex<Option<String>>>,
}

impl ExtraClient {
    fn judge(&self, key: &russh::keys::PublicKey) -> Result<(), String> {
        let verdict = known_hosts::verify(&self.host, self.port, key)
            .map_err(|error| format!("Host-Schlüssel nicht prüfbar: {error}"))?;
        same_host_key(
            verdict,
            &key.algorithm().to_string(),
            &known_hosts::fingerprint(key),
            &self.main,
        )
    }
}

impl client::Handler for ExtraClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let verdict = match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => self.judge(key),
            PublicKeyOrCertificate::Certificate(_) => {
                Err("Host-Zertifikat wird nicht anerkannt".to_string())
            }
        };
        match verdict {
            Ok(()) => Ok(true),
            Err(detail) => {
                *lock(&self.failure) = Some(detail);
                Ok(false)
            }
        }
    }

    async fn server_channel_open_session(
        &mut self,
        _channel: Channel<client::Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        drop(reply);
        Ok(())
    }

    async fn server_channel_open_agent_forward(
        &mut self,
        _channel: Channel<client::Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        drop(reply);
        Ok(())
    }

    async fn server_channel_open_x11(
        &mut self,
        _channel: Channel<client::Msg>,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        drop(reply);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        _channel: Channel<client::Msg>,
        _connected_address: &str,
        _connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        drop(reply);
        Ok(())
    }

    async fn server_channel_open_forwarded_streamlocal(
        &mut self,
        _channel: Channel<client::Msg>,
        _socket_path: &str,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        drop(reply);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn server_channel_open_direct_tcpip(
        &mut self,
        _channel: Channel<client::Msg>,
        _host_to_connect: &str,
        _port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        drop(reply);
        Ok(())
    }

    async fn server_channel_open_direct_streamlocal(
        &mut self,
        _channel: Channel<client::Msg>,
        _socket_path: &str,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        drop(reply);
        Ok(())
    }
}

/// A logged-in extra connection. It lives as long as the pool channel that
/// holds it; dropping it logs the close and disconnects.
pub(in crate::sftp) struct ExtraLink {
    /// The main generation whose host key it was checked against.
    pub(in crate::sftp) generation: Arc<SftpGeneration>,
    session: Arc<client::Handle<ExtraClient>>,
    runtime: tokio::runtime::Handle,
    ident: LinkIdent,
    /// Extra connections open (shared with the pool, for the log line).
    open_links: Arc<AtomicUsize>,
    reason: Mutex<&'static str>,
    dead: AtomicBool,
    _memory: MemoryReservation,
}

impl ExtraLink {
    /// The connection is gone or was given up.
    pub(in crate::sftp) fn is_dead(&self) -> bool {
        self.dead.load(Ordering::Acquire) || self.session.is_closed()
    }

    /// Gives the connection up (its channel failed or the connection broke).
    /// A break of a working connection blocks nothing; it is not replaced.
    pub(in crate::sftp) fn mark_dead(&self, reason: &'static str) {
        self.set_reason(reason);
        self.dead.store(true, Ordering::Release);
    }

    /// Why it closes, for the log line; the first reason stays.
    pub(in crate::sftp) fn set_reason(&self, reason: &'static str) {
        let mut current = lock(&self.reason);
        if *current == DEFAULT_REASON {
            *current = reason;
        }
    }

    pub(in crate::sftp) fn ident(&self) -> &LinkIdent {
        &self.ident
    }
}

impl Drop for ExtraLink {
    fn drop(&mut self) {
        let left = self
            .open_links
            .fetch_sub(1, Ordering::AcqRel)
            .saturating_sub(1);
        let reason = *lock(&self.reason);
        link_log::emit(&link_log::line(
            "geschlossen",
            &self.ident,
            left + 1,
            reason,
        ));
        let session = self.session.clone();
        // Dropping the handle alone would leave the connection to its tasks.
        drop(self.runtime.spawn(async move {
            let _ = session
                .disconnect(russh::Disconnect::ByApplication, "", "")
                .await;
        }));
    }
}

/// What `establish` needs.
pub(in crate::sftp) struct Request<'a> {
    pub(in crate::sftp) host: &'a str,
    pub(in crate::sftp) port: u16,
    pub(in crate::sftp) user: &'a str,
    pub(in crate::sftp) login: &'a ExtraLogin,
    pub(in crate::sftp) generation: Arc<SftpGeneration>,
    /// The key the verified main connection showed.
    pub(in crate::sftp) main_key: UnknownHostKey,
    /// Held until the login is done: one unauthenticated connection per
    /// server at a time.
    pub(in crate::sftp) slot: LoginSlot,
    pub(in crate::sftp) memory: MemoryReservation,
    pub(in crate::sftp) open_links: Arc<AtomicUsize>,
    pub(in crate::sftp) runtime: tokio::runtime::Handle,
}

/// A logged-in connection and its first session channel (the subsystem is
/// not requested yet).
pub(in crate::sftp) struct Established {
    pub(in crate::sftp) link: Arc<ExtraLink>,
    pub(in crate::sftp) channel: Channel<client::Msg>,
}

/// TCP, SSH handshake with the host-key checks, login, one session channel.
/// `Err` carries the reason for the block and the log.
pub(in crate::sftp) async fn establish(request: Request<'_>) -> Result<Established, String> {
    let Request {
        host,
        port,
        user,
        login,
        generation,
        main_key,
        slot,
        memory,
        open_links,
        runtime,
    } = request;
    let attempt = async move {
        let failure = Arc::new(Mutex::new(None));
        let handler = ExtraClient {
            host: host.to_string(),
            port,
            main: main_key,
            failure: failure.clone(),
        };
        let socket = connect_tcp(host, port)
            .await
            .map_err(|error| format!("Verbindungsaufbau: {error}"))?;
        let config = Arc::new(client_config());
        if config.nodelay {
            let _ = socket.set_nodelay(true);
        }
        let mut handle = client::connect_stream(config, socket, handler)
            .await
            .map_err(|error| {
                lock(&failure)
                    .take()
                    .unwrap_or_else(|| format!("SSH-Aufbau: {error}"))
            })?;
        let method = auth::login_extra(&mut handle, user, login)
            .await
            .map_err(|error| format!("Anmeldung: {error}"))?
            .ok_or_else(|| "Anmeldung abgelehnt".to_string())?;
        drop(slot);
        let session = Arc::new(handle);
        let channel = session
            .channel_open_session()
            .await
            .map_err(|error| format!("Kanal abgelehnt: {error}"))?;
        open_links.fetch_add(1, Ordering::AcqRel);
        let link = Arc::new(ExtraLink {
            generation,
            session,
            runtime,
            ident: LinkIdent {
                host: host.to_string(),
                port,
                user: user.to_string(),
                method: method.to_string(),
            },
            open_links,
            reason: Mutex::new(DEFAULT_REASON),
            dead: AtomicBool::new(false),
            _memory: memory,
        });
        Ok::<Established, String>(Established { link, channel })
    };
    tokio::time::timeout(EXTRA_DEADLINE, attempt)
        .await
        .map_err(|_| "Zeitüberschreitung beim Aufbau".to_string())?
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
