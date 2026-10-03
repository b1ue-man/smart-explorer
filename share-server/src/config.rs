//! Startup policy and configurable signaling limits (FC4).

use std::net::SocketAddr;
use std::path::PathBuf;

use crate::limits::ServerLimits;

pub(super) const HELP: &str = "se-share-server [IP:PORT] [--tls-cert CERT.pem --tls-key KEY.pem]\n\
    [--allow-plaintext] [--require-key-login] [--state-file FILE]\n\
    TLS files: SE_SHARE_TLS_CERT / SE_SHARE_TLS_KEY. Plaintext: SE_SHARE_ALLOW_PLAINTEXT=1.\n\
    Bind: SE_SHARE_BIND (default 0.0.0.0:51820). Loopback permits a TLS reverse proxy.\n\
    Bindings: SE_SHARE_STATE_FILE. Key login: SE_SHARE_REQUIRE_KEY_LOGIN=1.\n\
    Limits: SE_SHARE_MAX_CONNECTIONS[_PER_SOURCE|_PER_NETWORK],\n\
    SE_SHARE_MAX_CLIENTS[_PER_SOURCE|_PER_NETWORK|_PER_KEY].";

pub(super) struct Options {
    pub(super) bind: SocketAddr,
    pub(super) cert: Option<PathBuf>,
    pub(super) key: Option<PathBuf>,
    pub(super) allow_plaintext: bool,
    pub(super) require_key_login: bool,
    pub(super) state_file: Option<PathBuf>,
    pub(super) limits: ServerLimits,
}

impl Options {
    pub(super) fn from_env_and_args() -> Result<Option<Self>, String> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        Self::parse(args, |name| match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid Unicode")),
        })
    }

    fn parse(
        args: Vec<String>,
        env: impl Fn(&str) -> Result<Option<String>, String>,
    ) -> Result<Option<Self>, String> {
        if args.iter().any(|arg| arg == "--help" || arg == "-h") {
            return Ok(None);
        }
        let mut bind = env("SE_SHARE_BIND")?.unwrap_or_else(|| "0.0.0.0:51820".into());
        let mut cert = env("SE_SHARE_TLS_CERT")?;
        let mut key = env("SE_SHARE_TLS_KEY")?;
        let mut state_file = env("SE_SHARE_STATE_FILE")?;
        let mut allow_plaintext =
            flag(env("SE_SHARE_ALLOW_PLAINTEXT")?, "SE_SHARE_ALLOW_PLAINTEXT")?;
        let mut require_key_login = flag(
            env("SE_SHARE_REQUIRE_KEY_LOGIN")?,
            "SE_SHARE_REQUIRE_KEY_LOGIN",
        )?;
        let mut positional = false;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--allow-plaintext" => allow_plaintext = true,
                "--require-key-login" => require_key_login = true,
                "--tls-cert" => cert = Some(value(&mut args, &arg)?),
                "--tls-key" => key = Some(value(&mut args, &arg)?),
                "--state-file" => state_file = Some(value(&mut args, &arg)?),
                _ if !arg.starts_with('-') && !positional => {
                    bind = arg;
                    positional = true;
                }
                _ => return Err(format!("unknown argument {arg}; use --help")),
            }
        }
        let bind = bind
            .parse::<SocketAddr>()
            .map_err(|error| format!("invalid signaling bind: {error}"))?;
        if cert.is_some() != key.is_some() {
            return Err(
                "TLS requires both --tls-cert and --tls-key (or SE_SHARE_TLS_CERT/KEY)".into(),
            );
        }
        for (name, path) in [
            ("TLS certificate", &cert),
            ("TLS key", &key),
            ("state file", &state_file),
        ] {
            if path.as_ref().is_some_and(|path| path.trim().is_empty()) {
                return Err(format!("{name} path is empty"));
            }
        }
        let mut limits = ServerLimits::default();
        limits.max_connections = limit(&env, "SE_SHARE_MAX_CONNECTIONS", limits.max_connections)?;
        limits.max_connections_per_source = limit(
            &env,
            "SE_SHARE_MAX_CONNECTIONS_PER_SOURCE",
            limits.max_connections_per_source,
        )?;
        limits.max_connections_per_network = limit(
            &env,
            "SE_SHARE_MAX_CONNECTIONS_PER_NETWORK",
            limits
                .max_connections_per_source
                .saturating_mul(crate::limits::NETWORK_CAP_FACTOR),
        )?;
        limits.max_clients = limit(&env, "SE_SHARE_MAX_CLIENTS", limits.max_clients)?;
        limits.max_clients_per_source = limit(
            &env,
            "SE_SHARE_MAX_CLIENTS_PER_SOURCE",
            limits.max_clients_per_source,
        )?;
        limits.max_clients_per_network = limit(
            &env,
            "SE_SHARE_MAX_CLIENTS_PER_NETWORK",
            limits
                .max_clients_per_source
                .saturating_mul(crate::limits::NETWORK_CAP_FACTOR),
        )?;
        limits.max_clients_per_key = limit(
            &env,
            "SE_SHARE_MAX_CLIENTS_PER_KEY",
            limits.max_clients_per_key,
        )?;
        Ok(Some(Self {
            bind,
            cert: cert.map(PathBuf::from),
            key: key.map(PathBuf::from),
            allow_plaintext,
            require_key_login,
            state_file: state_file.map(PathBuf::from),
            limits,
        }))
    }

    /// Each public listener needs TLS or an explicit plaintext opt-in.
    pub(super) fn check_security(&self, relay: Option<SocketAddr>) -> Result<(), String> {
        let public = !self.bind.ip().is_loopback()
            || relay.is_some_and(|address| !address.ip().is_loopback());
        if public && self.cert.is_none() && !self.allow_plaintext {
            return Err("public signaling/relay requires --tls-cert CERT.pem --tls-key KEY.pem; \
                use --allow-plaintext only intentionally, or bind both listeners to loopback for a TLS reverse proxy".into());
        }
        Ok(())
    }
}

fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .filter(|value| !value.starts_with("--"))
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn flag(value: Option<String>, name: &str) -> Result<bool, String> {
    match value.as_deref().map(str::trim) {
        None | Some("0") => Ok(false),
        Some("1") => Ok(true),
        _ => Err(format!("{name} must be 0 or 1")),
    }
}

fn limit(
    env: &impl Fn(&str) -> Result<Option<String>, String>,
    name: &str,
    default: usize,
) -> Result<usize, String> {
    match env(name)? {
        None => Ok(default),
        Some(value) => value
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| format!("{name} must be a positive integer")),
    }
}

#[cfg(test)]
mod tests {
    use super::Options;

    fn parse(args: &[&str], values: &[(&str, &str)]) -> Result<Options, String> {
        Options::parse(args.iter().map(|value| (*value).into()).collect(), |name| {
            Ok(values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).into()))
        })?
        .ok_or_else(|| "unexpected help".into())
    }

    #[test]
    fn review_task_public_listener_requires_tls_or_explicit_plaintext() {
        let public = parse(&[], &[]).unwrap();
        assert!(public
            .check_security(None)
            .unwrap_err()
            .contains("--tls-cert"));
        assert!(parse(&["--allow-plaintext"], &[])
            .unwrap()
            .check_security(None)
            .is_ok());
        assert!(parse(&[], &[("SE_SHARE_ALLOW_PLAINTEXT", "1")])
            .unwrap()
            .check_security(None)
            .is_ok());
        let proxied = parse(&["127.0.0.1:51820"], &[]).unwrap();
        assert!(proxied
            .check_security(Some("127.0.0.1:51821".parse().unwrap()))
            .is_ok());
        assert!(proxied
            .check_security(Some("0.0.0.0:51821".parse().unwrap()))
            .is_err());
        let tls = parse(&["--tls-cert", "cert.pem", "--tls-key", "key.pem"], &[]).unwrap();
        assert!(tls
            .check_security(Some("0.0.0.0:51821".parse().unwrap()))
            .is_ok());
        assert!(parse(&["--tls-cert", "cert.pem"], &[]).is_err());
        assert!(parse(&[], &[("SE_SHARE_TLS_KEY", "key.pem")]).is_err());
        assert!(parse(&[], &[("SE_SHARE_ALLOW_PLAINTEXT", "yes")]).is_err());
        assert!(parse(&["--state-file", ""], &[]).is_err());
    }

    #[test]
    fn review_task_limits_are_configurable_and_network_defaults_follow_source_caps() {
        let config = parse(
            &["--require-key-login"],
            &[
                ("SE_SHARE_MAX_CONNECTIONS", "400"),
                ("SE_SHARE_MAX_CONNECTIONS_PER_SOURCE", "20"),
                ("SE_SHARE_MAX_CLIENTS", "200"),
                ("SE_SHARE_MAX_CLIENTS_PER_SOURCE", "12"),
                ("SE_SHARE_MAX_CLIENTS_PER_KEY", "6"),
            ],
        )
        .unwrap();
        assert!(config.require_key_login);
        assert_eq!(config.limits.max_connections, 400);
        assert_eq!(config.limits.max_connections_per_network, 80);
        assert_eq!(config.limits.max_clients, 200);
        assert_eq!(config.limits.max_clients_per_network, 48);
        assert_eq!(config.limits.max_clients_per_key, 6);
        assert!(parse(&[], &[("SE_SHARE_MAX_CLIENTS", "0")]).is_err());
        assert!(parse(&[], &[("SE_SHARE_MAX_CONNECTIONS", "-1")]).is_err());
    }
}
