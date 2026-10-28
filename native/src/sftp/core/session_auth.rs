//! User authentication of the SSH client (RFC 4252, RFC 4256).
//!
//! - Password; when the server refuses `password` but offers
//!   `keyboard-interactive` (PAM setups with `PasswordAuthentication no`),
//!   the stored password answers one hidden password prompt, within four
//!   rounds (`KeyboardInteractiveAnswers`).
//! - Key file with optional passphrase; an OpenSSH user certificate next to
//!   it (`<key>-cert.pub`, as `ssh` loads it) is tried first.
//! - SSH agent (`SSH_AUTH_SOCK` on Unix; the Windows OpenSSH agent pipe,
//!   then Pageant): every identity the agent holds, including keys on FIDO2
//!   or smartcard tokens that only the agent can sign with. The identity
//!   that logged in and whether it signed without a person are recorded for
//!   extra connections (agent_grant.rs, G3-10).
use std::io;
use std::sync::Arc;

use russh::client::{self, AuthResult, KeyboardInteractiveAuthResponse};
use russh::keys::agent::client::AgentClient;
use russh::keys::agent::AgentIdentity;
use russh::keys::HashAlg;
use russh::MethodKind;
use tokio::io::{AsyncRead, AsyncWrite};

use super::super::agent_grant::{self, AgentGrant, TimedSigner};
use super::super::config::{SftpAuth, SftpConfig};
use super::super::io_err;
use super::super::link_rules::{account_key, ExtraLogin};
use super::super::ssh_agent;
use super::Client;

pub(super) async fn authenticate(
    session: &mut client::Handle<Client>,
    cfg: &SftpConfig,
) -> io::Result<bool> {
    match &cfg.auth {
        SftpAuth::Password(secret) => {
            Ok(password_login(session, &cfg.user, secret).await?.is_some())
        }
        SftpAuth::Key { path, passphrase } => {
            key_file_login(session, &cfg.user, path, passphrase.as_deref()).await
        }
        SftpAuth::Agent => {
            let account = account_key(&cfg.host, cfg.port, &cfg.user);
            agent_login(session, &cfg.user, &account).await
        }
    }
}

/// The method that logged in (`password`, or `keyboard-interactive` for a
/// PAM server that refuses `password`), `None` when the server refused.
async fn password_login<H: client::Handler>(
    session: &mut client::Handle<H>,
    user: &str,
    password: &str,
) -> io::Result<Option<&'static str>> {
    match session
        .authenticate_password(user, password)
        .await
        .map_err(io_err)?
    {
        AuthResult::Success => Ok(Some("password")),
        AuthResult::Failure {
            remaining_methods, ..
        } if remaining_methods.contains(&MethodKind::KeyboardInteractive) => {
            Ok(keyboard_interactive(session, user, password)
                .await?
                .then_some("keyboard-interactive"))
        }
        AuthResult::Failure { .. } => Ok(None),
    }
}

/// The login of an extra connection (security answer E13-S15 (i)): the
/// password is read from the sealed store for this login and dropped right
/// after it (`password`, or `keyboard-interactive` under the rules below:
/// one hidden password prompt, a fixed number of rounds; any other question
/// is a refusal); a key file uses the decrypted key of the main login; the
/// SSH agent signs only with the identity the main login showed to sign
/// without a person (agent_grant.rs, G3-10). No prompt for the person.
/// `None`: the server refused. `account` is `link_rules::account_key`.
pub(in crate::sftp) async fn login_extra<H: client::Handler>(
    session: &mut client::Handle<H>,
    user: &str,
    login: &ExtraLogin,
    account: &str,
) -> io::Result<Option<&'static str>> {
    match login {
        ExtraLogin::Password(source) => {
            let secret = source()?;
            let method = password_login(session, user, &secret).await;
            drop(secret);
            method
        }
        ExtraLogin::Key { path, key } => Ok(key_login(session, user, path, key.clone())
            .await?
            .then_some("publickey")),
        ExtraLogin::Agent => {
            let Some(fingerprint) = agent_grant::usable(account) else {
                return Ok(None);
            };
            let agent = ssh_agent::connect().await.map_err(agent_error)?;
            agent_extra(session, user, agent, account, &fingerprint).await
        }
    }
}

/// The extra login through the agent: only the recorded identity; a
/// signature that needed a person revokes the grant (agent_grant.rs).
async fn agent_extra<H, S>(
    session: &mut client::Handle<H>,
    user: &str,
    mut agent: AgentClient<S>,
    account: &str,
    fingerprint: &str,
) -> io::Result<Option<&'static str>>
where
    H: client::Handler,
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let identities = agent.request_identities().await.map_err(agent_error)?;
    let Some(identity) = identities
        .into_iter()
        .find(|identity| agent_grant::identity_facts(identity).0 == fingerprint)
    else {
        // The agent no longer holds it: nothing is signed.
        agent_grant::revoke(account);
        return Ok(None);
    };
    let (_, algorithm) = agent_grant::identity_facts(&identity);
    let rsa_hash = rsa_hash(session).await?;
    let mut signer = TimedSigner::new(&mut agent);
    let result = sign_in(session, user, identity, rsa_hash, &mut signer).await?;
    if !agent_grant::quick(&algorithm, signer.took) {
        agent_grant::revoke(account);
    }
    Ok(result.then_some("publickey (SSH-Agent)"))
}

/// One publickey attempt with an agent identity (a key or a certificate).
async fn sign_in<H, S>(
    session: &mut client::Handle<H>,
    user: &str,
    identity: AgentIdentity,
    rsa_hash: Option<HashAlg>,
    signer: &mut S,
) -> io::Result<bool>
where
    H: client::Handler,
    S: russh::Signer + Send,
    S::Error: std::fmt::Display,
{
    match identity {
        AgentIdentity::PublicKey { key, .. } => {
            let hash = key.algorithm().is_rsa().then_some(rsa_hash).flatten();
            session
                .authenticate_publickey_with(user, key, hash, signer)
                .await
        }
        AgentIdentity::Certificate { certificate, .. } => {
            let hash = certificate
                .algorithm()
                .is_rsa()
                .then_some(rsa_hash)
                .flatten();
            session
                .authenticate_certificate_with(user, certificate, hash, signer)
                .await
        }
    }
    .map(|result| result.success())
    .map_err(|error| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("SSH agent: {error}"),
        )
    })
}

/// Most server `InfoRequest` rounds one keyboard-interactive login takes
/// part in. A PAM stack asks once for the password; four rounds cover an
/// information-only round before it, the password prompt, a closing
/// information round and one spare. A server that keeps asking is not
/// answered further (RFC 4256 sets no limit; OpenSSH's own client stops
/// after `NumberOfPasswordPrompts`, default 3).
const KEYBOARD_INTERACTIVE_MAX_ROUNDS: usize = 4;

/// Which prompts of a keyboard-interactive login get the stored password:
/// only prompts that hide the typed text (`echo` false) and ask for the
/// password, once per login (a repeated prompt means the password was
/// refused; asking again could lock the account), within the round limit.
#[derive(Debug, Default)]
struct KeyboardInteractiveAnswers {
    rounds: usize,
    answered: bool,
}

impl KeyboardInteractiveAnswers {
    /// The answers for one `InfoRequest` (`prompts` as `(echo, text)`), or
    /// `None` when the attempt ends here: a visible prompt, a question that
    /// is not for the password (one-time code, new password), a second
    /// password prompt or a round beyond the limit.
    fn answer(&mut self, prompts: &[(bool, &str)], password: &str) -> Option<Vec<String>> {
        self.rounds += 1;
        if self.rounds > KEYBOARD_INTERACTIVE_MAX_ROUNDS {
            return None;
        }
        let mut answers = Vec::with_capacity(prompts.len());
        for (echo, text) in prompts {
            if self.answered || *echo || !asks_for_password(text) {
                return None;
            }
            answers.push(password.to_string());
            self.answered = true;
        }
        Some(answers)
    }
}

/// Answers prompts that ask for the password (no echo, "password" in the
/// prompt text) with the stored password of this connection, at most once;
/// any other question ends the attempt (`KeyboardInteractiveAnswers`).
async fn keyboard_interactive<H: client::Handler>(
    session: &mut client::Handle<H>,
    user: &str,
    password: &str,
) -> io::Result<bool> {
    let mut policy = KeyboardInteractiveAnswers::default();
    let mut response = session
        .authenticate_keyboard_interactive_start(user, None::<String>)
        .await
        .map_err(io_err)?;
    loop {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                let answers = {
                    let shown: Vec<(bool, &str)> = prompts
                        .iter()
                        .map(|prompt| (prompt.echo, prompt.prompt.as_str()))
                        .collect();
                    policy.answer(&shown, password)
                };
                let Some(answers) = answers else {
                    return Ok(false);
                };
                response = session
                    .authenticate_keyboard_interactive_respond(answers)
                    .await
                    .map_err(io_err)?;
            }
        }
    }
}

fn asks_for_password(prompt: &str) -> bool {
    let prompt = prompt.to_lowercase();
    ["password", "passwort", "kennwort", "passphrase"]
        .iter()
        .any(|word| prompt.contains(word))
}

async fn key_file_login(
    session: &mut client::Handle<Client>,
    user: &str,
    path: &str,
    passphrase: Option<&str>,
) -> io::Result<bool> {
    let key = Arc::new(russh::keys::load_secret_key(path, passphrase).map_err(io_err)?);
    key_login(session, user, path, key).await
}

/// Login with an already decrypted key and, when `<path>-cert.pub` exists,
/// its OpenSSH user certificate first.
async fn key_login<H: client::Handler>(
    session: &mut client::Handle<H>,
    user: &str,
    path: &str,
    key: Arc<russh::keys::PrivateKey>,
) -> io::Result<bool> {
    if let Ok(certificate) = russh::keys::load_openssh_certificate(format!("{path}-cert.pub")) {
        if session
            .authenticate_openssh_cert(user, key.clone(), certificate)
            .await
            .map_err(io_err)?
            .success()
        {
            return Ok(true);
        }
    }
    let hash = rsa_hash(session).await?;
    Ok(session
        .authenticate_publickey(user, russh::keys::PrivateKeyWithHashAlg::new(key, hash))
        .await
        .map_err(io_err)?
        .success())
}

async fn rsa_hash<H: client::Handler>(session: &client::Handle<H>) -> io::Result<Option<HashAlg>> {
    Ok(session
        .best_supported_rsa_hash()
        .await
        .map_err(io_err)?
        .flatten())
}

/// The platform adapter (`os/ssh_agent_*.rs`) finds the agent; the login
/// with its identities is the same everywhere.
async fn agent_login(
    session: &mut client::Handle<Client>,
    user: &str,
    account: &str,
) -> io::Result<bool> {
    let agent = ssh_agent::connect().await.map_err(agent_error)?;
    agent_identities(session, user, agent, account).await
}

/// Tries every identity of the agent; the one that logs in is recorded for
/// extra connections with whether it signed without a person.
async fn agent_identities<S>(
    session: &mut client::Handle<Client>,
    user: &str,
    mut agent: AgentClient<S>,
    account: &str,
) -> io::Result<bool>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let identities = agent.request_identities().await.map_err(agent_error)?;
    if identities.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "SSH agent holds no keys (ssh-add)",
        ));
    }
    let rsa_hash = rsa_hash(session).await?;
    for identity in identities {
        let (fingerprint, algorithm) = agent_grant::identity_facts(&identity);
        let mut signer = TimedSigner::new(&mut agent);
        if sign_in(session, user, identity, rsa_hash, &mut signer).await? {
            let quick = agent_grant::quick(&algorithm, signer.took);
            agent_grant::record(account, AgentGrant { fingerprint, quick });
            return Ok(true);
        }
    }
    Ok(false)
}

fn agent_error(error: russh::keys::Error) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("SSH agent not reachable: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::{asks_for_password, KeyboardInteractiveAnswers, KEYBOARD_INTERACTIVE_MAX_ROUNDS};

    #[test]
    fn luecken_task_only_password_prompts_get_the_stored_password() {
        assert!(asks_for_password("Password: "));
        assert!(asks_for_password("alice@host's password:"));
        assert!(asks_for_password("Passwort für alice:"));
        assert!(!asks_for_password("Verification code: "));
        assert!(!asks_for_password("OTP:"));
    }

    #[test]
    fn luecken_task_keyboard_interactive_answers_one_hidden_password_prompt() {
        let mut policy = KeyboardInteractiveAnswers::default();
        assert_eq!(
            policy.answer(&[(false, "Password: ")], "secret"),
            Some(vec!["secret".to_string()])
        );
        // A repeated password prompt means the password was refused.
        assert_eq!(policy.answer(&[(false, "Password: ")], "secret"), None);
    }

    #[test]
    fn luecken_task_keyboard_interactive_refuses_visible_and_foreign_prompts() {
        let mut visible = KeyboardInteractiveAnswers::default();
        assert_eq!(visible.answer(&[(true, "Password: ")], "secret"), None);
        let mut code = KeyboardInteractiveAnswers::default();
        assert_eq!(
            code.answer(&[(false, "Verification code: ")], "secret"),
            None
        );
        // One foreign prompt among several ends the round: nothing is sent.
        let mut mixed = KeyboardInteractiveAnswers::default();
        assert_eq!(
            mixed.answer(&[(false, "Password: "), (false, "OTP: ")], "secret"),
            None
        );
    }

    #[test]
    fn luecken_task_keyboard_interactive_stops_after_a_fixed_number_of_rounds() {
        let mut policy = KeyboardInteractiveAnswers::default();
        for _ in 0..KEYBOARD_INTERACTIVE_MAX_ROUNDS {
            // Information-only rounds (no prompts) are answered empty ...
            assert_eq!(policy.answer(&[], "secret"), Some(Vec::new()));
        }
        // ... but a server that keeps sending them is not answered forever,
        // and not even a password prompt after the limit.
        assert_eq!(policy.answer(&[], "secret"), None);
        assert_eq!(policy.answer(&[(false, "Password: ")], "secret"), None);
    }

    #[test]
    fn luecken_task_keyboard_interactive_allows_info_round_password_and_closing_info() {
        let mut policy = KeyboardInteractiveAnswers::default();
        assert_eq!(policy.answer(&[], "secret"), Some(Vec::new()));
        assert_eq!(
            policy.answer(&[(false, "Password: ")], "secret"),
            Some(vec!["secret".to_string()])
        );
        assert_eq!(policy.answer(&[], "secret"), Some(Vec::new()));
    }
}
