//! Extra SSH connections against an SSH server in this process (G3-10): an
//! extra connection logs in only when the server shows the key the trust
//! store pins and the main connection showed. Any other key ends that one
//! attempt before a credential goes out, and an extra connection that
//! already works goes on (the pool then blocks further attempts for 15
//! minutes, link_tests.rs). The server is russh's own; its host key below
//! is a throwaway key made for this test only.
use super::host_key::UnknownHostKey;
use super::known_hosts;
use super::link_rules::{begin_login, server_key, ExtraLogin};
use super::session::connect_checked;
use russh::server::{self, Auth, Msg, Session};
use russh::Channel;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const HOST: &str = "127.0.0.1";
const HOST_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACCRBPM+PAdLGsKz5GWDefh+8rJm0af0wvu9ZL+ETSGDogAAAJCtwMx5rcDM
eQAAAAtzc2gtZWQyNTUxOQAAACCRBPM+PAdLGsKz5GWDefh+8rJm0af0wvu9ZL+ETSGDog
AAAEBChTZ0oTmDtErMZdphGAYj3wKpaBxntfBbITyFWujxUZEE8z48B0sawrPkZYN5+H7y
smbRp/TC+71kv4RNIYOiAAAADHNlLXRlc3QtaG9zdAE=
-----END OPENSSH PRIVATE KEY-----
";
/// Another valid ed25519 public key (known_hosts_tests.rs `KEY_TWO`).
const OTHER_KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIAICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgIC";

/// What the server does with an exec request (digest_exec tests).
#[derive(Clone)]
pub(super) enum ExecScript {
    /// Refuse the request (an account without commands).
    Reject,
    /// Accept and never answer, not even after EOF; a TERM signal is logged.
    Hang,
    /// Accept, read standard input until the client's EOF, then print
    /// `stdout`, exit with `status` and close (a command that reads its
    /// input first: the hash script, an SFTP server a forced command runs).
    UntilEof { stdout: String, status: u32 },
}

/// The commands the server received and the signals it saw.
#[derive(Default)]
pub(super) struct ExecLog {
    pub(super) commands: Mutex<Vec<String>>,
    pub(super) signals: Mutex<Vec<String>>,
    /// Every byte the client sent on standard input.
    pub(super) stdin: Mutex<Vec<u8>>,
}

#[derive(Clone)]
struct FakeSshd {
    passwords: Arc<AtomicUsize>,
    script: Arc<Mutex<ExecScript>>,
    log: Arc<ExecLog>,
    /// Channels waiting for EOF and what they answer then.
    waiting: Arc<Mutex<Vec<(russh::ChannelId, String, u32)>>>,
}

impl server::Handler for FakeSshd {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.passwords.fetch_add(1, Ordering::SeqCst);
        Ok(if user == "alice" && password == "secret" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: russh::ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.log
            .commands
            .lock()
            .unwrap()
            .push(String::from_utf8_lossy(data).into_owned());
        let script = self.script.lock().unwrap().clone();
        match script {
            ExecScript::Reject => session.channel_failure(channel)?,
            ExecScript::Hang => session.channel_success(channel)?,
            ExecScript::UntilEof { stdout, status } => {
                session.channel_success(channel)?;
                self.waiting.lock().unwrap().push((channel, stdout, status));
            }
        }
        Ok(())
    }

    async fn data(
        &mut self,
        _channel: russh::ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.log.stdin.lock().unwrap().extend_from_slice(data);
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: russh::ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let answer = {
            let mut waiting = self.waiting.lock().unwrap();
            let found = waiting.iter().position(|(id, _, _)| *id == channel);
            found.map(|index| waiting.remove(index))
        };
        if let Some((_, stdout, status)) = answer {
            session.data(channel, stdout.into_bytes())?;
            session.exit_status_request(channel, status)?;
            session.eof(channel)?;
            session.close(channel)?;
        }
        Ok(())
    }

    async fn signal(
        &mut self,
        _channel: russh::ChannelId,
        signal: russh::Sig,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.log.signals.lock().unwrap().push(format!("{signal:?}"));
        Ok(())
    }
}

pub(super) struct Sshd {
    pub(super) runtime: tokio::runtime::Runtime,
    pub(super) port: u16,
    passwords: Arc<AtomicUsize>,
    algorithm: String,
    pub(super) fingerprint: String,
    pub(super) script: Arc<Mutex<ExecScript>>,
    pub(super) log: Arc<ExecLog>,
}

impl Sshd {
    pub(super) fn start() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("runtime");
        let key = russh::keys::decode_secret_key(HOST_KEY, None).expect("test host key");
        let public = key.public_key().clone();
        let config = Arc::new(server::Config {
            keys: vec![key],
            auth_rejection_time: Duration::from_millis(1),
            auth_rejection_time_initial: Some(Duration::from_millis(1)),
            ..Default::default()
        });
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind((HOST, 0)))
            .expect("listen");
        let port = listener.local_addr().expect("address").port();
        let passwords = Arc::new(AtomicUsize::new(0));
        let script = Arc::new(Mutex::new(ExecScript::Reject));
        let log = Arc::new(ExecLog::default());
        let handler = FakeSshd {
            passwords: passwords.clone(),
            script: script.clone(),
            log: log.clone(),
            waiting: Arc::new(Mutex::new(Vec::new())),
        };
        drop(runtime.spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let (config, handler) = (config.clone(), handler.clone());
                drop(tokio::spawn(async move {
                    if let Ok(running) = server::run_stream(config, socket, handler).await {
                        let _ = running.await;
                    }
                }));
            }
        }));
        Self {
            runtime,
            port,
            passwords,
            algorithm: public.algorithm().to_string(),
            fingerprint: known_hosts::fingerprint(&public),
            script,
            log,
        }
    }

    fn passwords(&self) -> usize {
        self.passwords.load(Ordering::SeqCst)
    }

    /// What the main connection showed: this server's algorithm with
    /// `fingerprint`.
    pub(super) fn main_key(&self, fingerprint: &str) -> UnknownHostKey {
        UnknownHostKey {
            host: HOST.into(),
            port: self.port,
            algorithm: self.algorithm.clone(),
            fingerprint: fingerprint.into(),
        }
    }

    pub(super) fn extra(&self, main: UnknownHostKey) -> Result<super::session::Checked, String> {
        let source: super::config::PasswordSource =
            Arc::new(|| Ok(zeroize::Zeroizing::new("secret".to_string())));
        let login = ExtraLogin::Password(source);
        let slot = begin_login(&server_key(HOST, self.port)).expect("no other login runs");
        self.runtime.block_on(connect_checked(
            HOST, self.port, "alice", &login, main, slot,
        ))
    }
}

impl Drop for Sshd {
    fn drop(&mut self) {
        let _ = known_hosts::forget(HOST, self.port);
    }
}

#[test]
fn speed_task_sftp_extra_connection_logs_in_with_the_main_host_key() {
    let sshd = Sshd::start();
    known_hosts::pin(HOST, sshd.port, &sshd.fingerprint, true).expect("pin");
    let (link, method, _channel) = sshd
        .extra(sshd.main_key(&sshd.fingerprint))
        .expect("same host key as the main connection: logged in");
    assert_eq!(method, "password");
    assert_eq!(sshd.passwords(), 1, "one login with the stored password");
    assert!(!link.is_closed());
}

#[test]
fn speed_task_sftp_extra_connection_host_key_mismatch_ends_only_that_attempt() {
    let sshd = Sshd::start();
    known_hosts::pin(HOST, sshd.port, &sshd.fingerprint, true).expect("pin");
    let (working, _, _channel) = sshd
        .extra(sshd.main_key(&sshd.fingerprint))
        .expect("first extra connection");
    assert_eq!(sshd.passwords(), 1);

    // The main connection showed another key than this server now shows.
    let refused = sshd
        .extra(sshd.main_key("SHA256:another-main-key"))
        .err()
        .expect("another key than the main connection's is refused");
    assert!(refused.contains("Hauptverbindung"), "{refused}");
    assert_eq!(sshd.passwords(), 1, "no credential went to that server");

    // The trust store pins another key for host and port.
    let other = russh::keys::parse_public_key_base64(OTHER_KEY).expect("other key");
    known_hosts::pin(HOST, sshd.port, &known_hosts::fingerprint(&other), true).expect("re-pin");
    let refused = sshd
        .extra(sshd.main_key(&sshd.fingerprint))
        .err()
        .expect("a key the trust store does not pin is refused");
    assert!(refused.contains("Vertrauensspeicher"), "{refused}");
    assert_eq!(sshd.passwords(), 1, "still no credential sent");

    // Only those attempts ended: the extra connection that works goes on
    // and opens channels as before.
    assert!(!working.is_closed(), "the working extra connection stays");
    let channel = sshd
        .runtime
        .block_on(working.channel_open_session())
        .expect("the working extra connection still opens a channel");
    drop(channel);
}
