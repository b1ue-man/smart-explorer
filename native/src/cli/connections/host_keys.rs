//! `se connections trust-host-key` and `forget-host-key`: the SSH host-key
//! decisions that a terminal user takes in a prompt and an agent or script
//! takes with an explicitly compared fingerprint.
use std::io::{IsTerminal, Write};

use clap::Args;

#[derive(Args)]
pub(super) struct TrustHostKeyArgs {
    #[arg(help = "Saved SFTP label or account, or HOST[:PORT] (port 22 when omitted)")]
    target: String,
    #[arg(
        long,
        value_name = "SHA256:…",
        help = "Pin only when the server presents exactly this fingerprint (no prompt)"
    )]
    fingerprint: Option<String>,
    #[arg(
        long,
        help = "Replace a different pinned key (the server's key was changed on purpose)"
    )]
    replace: bool,
}

#[derive(Args)]
pub(super) struct ForgetHostKeyArgs {
    #[arg(help = "Saved SFTP label or account, or HOST[:PORT] (port 22 when omitted)")]
    target: String,
}

pub(super) fn trust(args: TrustHostKeyArgs) -> Result<(), String> {
    let (host, port) = host_and_port(&args.target)?;
    let fingerprint = match args.fingerprint {
        Some(fingerprint) => fingerprint,
        None => {
            let presented = crate::sftp::probe_host_key(&host, port)
                .map_err(|error| format!("SSH host key of {host}:{port}: {error}"))?;
            if presented.trusted {
                println!(
                    "host_key\t{host}:{port}\t{}\t{}\talready trusted",
                    presented.key.algorithm, presented.key.fingerprint
                );
                return Ok(());
            }
            replace_needed(&presented, args.replace)?;
            confirm(&presented)?;
            presented.key.fingerprint
        }
    };
    let pinned = crate::sftp::trust_host_key(&host, port, &fingerprint, args.replace)
        .map_err(|error| format!("SSH host key of {host}:{port}: {error}"))?;
    println!(
        "host_key\t{host}:{port}\t{}\t{}\tpinned",
        pinned.algorithm, pinned.fingerprint
    );
    Ok(())
}

pub(super) fn forget(args: ForgetHostKeyArgs) -> Result<(), String> {
    let (host, port) = host_and_port(&args.target)?;
    let removed = crate::sftp::forget_host_key(&host, port)
        .map_err(|error| format!("SSH host key of {host}:{port}: {error}"))?;
    println!(
        "host_key\t{host}:{port}\t{}",
        if removed {
            "forgotten"
        } else {
            "no pinned key"
        }
    );
    Ok(())
}

/// A key that differs from a recorded one is replaced only on request:
/// either a deliberate key change or an interception looks the same here.
fn replace_needed(presented: &crate::sftp::PresentedHostKey, replace: bool) -> Result<(), String> {
    match (&presented.changed_in, replace) {
        (Some(source), false) => Err(format!(
            "the SSH host key of {}:{} ({} {}) differs from the key recorded in {source}; \
             compare the fingerprint with the server and repeat with --replace only if the \
             server's key was replaced on purpose",
            presented.key.host,
            presented.key.port,
            presented.key.algorithm,
            presented.key.fingerprint
        )),
        _ => Ok(()),
    }
}

/// Asks on the terminal, like OpenSSH's `StrictHostKeyChecking ask`.
fn confirm(presented: &crate::sftp::PresentedHostKey) -> Result<(), String> {
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Err(format!(
            "no terminal to confirm the host key {} {} of {}:{}; compare it with the server and \
             pass --fingerprint",
            presented.key.algorithm,
            presented.key.fingerprint,
            presented.key.host,
            presented.key.port
        ));
    }
    if ask(presented)? {
        Ok(())
    } else {
        Err("host key not confirmed; nothing was saved".into())
    }
}

/// Shows the key and reads yes/no from the terminal.
pub(crate) fn ask(presented: &crate::sftp::PresentedHostKey) -> Result<bool, String> {
    let key = &presented.key;
    match &presented.changed_in {
        Some(source) => eprintln!(
            "WARNING: the SSH host key of {}:{} DIFFERS from the key recorded in {source}.",
            key.host, key.port
        ),
        None => eprintln!(
            "The SSH host key of {}:{} is not known yet.",
            key.host, key.port
        ),
    }
    eprintln!("  Key type:    {}", key.algorithm);
    eprintln!("  Fingerprint: {}", key.fingerprint);
    eprintln!("Compare it with the server (for example `ssh-keygen -lf` on the server).");
    eprint!("Trust this key and save it? [yes/NO] ");
    std::io::stderr()
        .flush()
        .map_err(|error| error.to_string())?;
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|error| error.to_string())?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "yes" | "y" | "ja" | "j"
    ))
}

/// A saved SFTP connection's host and port, or `HOST[:PORT]`.
fn host_and_port(target: &str) -> Result<(String, u16), String> {
    let target = target.trim().trim_start_matches('@');
    if let Ok(connections) = crate::creds::load_connections_checked() {
        if let Some(connection) = connections.iter().find(|connection| {
            connection.protocol == crate::creds::Protocol::Sftp
                && (connection.label == target || connection.account() == target)
        }) {
            return Ok((connection.host.clone(), connection.port));
        }
    }
    let (host, port) = match target.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') || host.starts_with('[') => (
            host.trim_start_matches('[').trim_end_matches(']'),
            port.parse::<u16>()
                .map_err(|_| format!("invalid port in {target:?}"))?,
        ),
        _ => (target.trim_start_matches('[').trim_end_matches(']'), 22),
    };
    if host.is_empty() || host.contains(char::is_whitespace) || port == 0 {
        return Err(format!("invalid SSH host {target:?}"));
    }
    Ok((host.to_string(), port))
}

#[cfg(test)]
mod tests {
    use super::{host_and_port, replace_needed};

    fn presented(changed_in: Option<&str>) -> crate::sftp::PresentedHostKey {
        crate::sftp::PresentedHostKey {
            key: crate::sftp::UnknownHostKey {
                host: "nas".into(),
                port: 22,
                algorithm: "ssh-ed25519".into(),
                fingerprint: "SHA256:new".into(),
            },
            trusted: false,
            changed_in: changed_in.map(str::to_string),
        }
    }

    #[test]
    fn luecken_task_trust_host_key_needs_replace_for_a_recorded_different_key() {
        assert!(replace_needed(&presented(None), false).is_ok());
        assert!(replace_needed(&presented(Some("known_hosts_sftp.txt")), true).is_ok());
        let error = replace_needed(&presented(Some("known_hosts_sftp.txt")), false).unwrap_err();
        assert!(error.contains("known_hosts_sftp.txt"));
        assert!(error.contains("SHA256:new"));
        assert!(error.contains("--replace"));
    }

    #[test]
    fn luecken_task_host_targets_accept_host_port_and_ipv6_brackets() {
        assert_eq!(host_and_port("nas").unwrap(), ("nas".to_string(), 22));
        assert_eq!(
            host_and_port("nas:2222").unwrap(),
            ("nas".to_string(), 2222)
        );
        assert_eq!(
            host_and_port("[::1]:2222").unwrap(),
            ("::1".to_string(), 2222)
        );
        assert_eq!(host_and_port("::1").unwrap(), ("::1".to_string(), 22));
        assert!(host_and_port("nas:x").is_err());
        assert!(host_and_port("").is_err());
    }
}
