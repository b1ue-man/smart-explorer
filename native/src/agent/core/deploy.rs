use super::backend::AgentBackend;
use super::transport::AgentReconnect;
use crate::agent_proto;
use crate::vfs::{Backend, BackendHandle};
use std::io::{self, Read, Write};
use std::sync::Arc;

/// A bundled agent binary for one server target. The integrity hash is computed
/// from `bytes` at deploy time.
pub struct AgentArtifact {
    pub bytes: &'static [u8],
}

/// Select the bundled agent for a server's `uname -sm`.
pub fn artifact_for(uname_sm: &str) -> Option<AgentArtifact> {
    let mut it = uname_sm.split_whitespace();
    let os = it.next().unwrap_or("");
    let arch = it.next().unwrap_or("");
    let bytes: &'static [u8] = match (os, arch) {
        ("Linux", "x86_64") => include_bytes!("../../../agent-bin/se-agent-x86_64-linux-musl"),
        ("Linux", "aarch64") | ("Linux", "arm64") => {
            include_bytes!("../../../agent-bin/se-agent-aarch64-linux-musl")
        }
        _ => return None,
    };
    Some(AgentArtifact { bytes })
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

fn sha256_reader(mut reader: Box<dyn Read + Send>) -> io::Result<String> {
    use sha2::{Digest, Sha256};

    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn remote_sha256(inner: &dyn Backend, path: &str) -> io::Result<String> {
    sha256_reader(inner.open_read(path)?)
}

fn require_remote_sha256(inner: &dyn Backend, path: &str, expected: &str) -> io::Result<()> {
    let actual = remote_sha256(inner, path)?;
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Agent-Binary: SHA-256 stimmt nicht ({path})"),
        ))
    }
}

fn upload_suffix() -> io::Result<String> {
    let mut bytes = [0u8; 12];
    getrandom::getrandom(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn open_verified_agent(
    sftp: &crate::sftp::SftpBackend,
    inner: &dyn Backend,
    remote: &str,
    expected: &str,
    command: &str,
) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    require_remote_sha256(inner, remote, expected)?;
    sftp.open_exec_streams(command)
}

/// Single-quote a string for safe interpolation into a remote `sh -c` command.
pub(super) fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r#"'\''"#))
}

/// Deploy + launch the agent over an existing SFTP backend's SSH connection.
pub fn deploy_over_sftp(
    sftp: &crate::sftp::SftpBackend,
    inner: BackendHandle,
) -> io::Result<AgentBackend> {
    let uname = sftp.exec_capture("uname -sm")?;
    let art = artifact_for(&uname)
        .ok_or_else(|| io::Error::other(format!("kein Agent-Binary gebündelt für '{uname}'")))?;

    let home = sftp.exec_capture("printf %s \"$HOME\"")?;
    let home = if home.is_empty() {
        ".".to_string()
    } else {
        home
    };
    let dir = format!("{}/.cache/smart-explorer", home.trim_end_matches('/'));
    let expected = sha256_hex(art.bytes);
    let remote = format!(
        "{}/se-agent-p{}-{}",
        dir,
        agent_proto::PROTO_VERSION,
        expected
    );

    inner.mkdir_all(&dir)?;
    let installed = if inner.try_exists(&remote)? {
        remote_sha256(&*inner, &remote)?.eq_ignore_ascii_case(&expected)
    } else {
        false
    };
    if !installed {
        let tmp = format!("{}.tmp-{}", remote, upload_suffix()?);
        let upload = (|| -> io::Result<()> {
            {
                let mut writer = inner.open_write_new(&tmp)?;
                writer.write_all(art.bytes)?;
                writer.flush()?;
            }
            require_remote_sha256(&*inner, &tmp, &expected)?;
            sftp.exec_capture(&format!(
                "mv -f {tmp} {remote} && chmod 700 {remote}",
                tmp = sh_quote(&tmp),
                remote = sh_quote(&remote),
            ))?;
            require_remote_sha256(&*inner, &remote, &expected)
        })();
        if let Err(error) = upload {
            let _ = inner.remove_file(&tmp);
            return Err(error);
        }
    } else {
        sftp.exec_capture(&format!("chmod 700 {}", sh_quote(&remote)))?;
    }

    let serve_command = format!(
        "{} --serve-root {}",
        sh_quote(&remote),
        sh_quote(&sftp.root_display())
    );
    let reconnect_sftp = sftp.clone();
    let reconnect_inner = inner.clone();
    let reconnect_remote = remote.clone();
    let reconnect_expected = expected.clone();
    let reconnect_command = serve_command.clone();
    let reconnect: AgentReconnect = Arc::new(move || {
        open_verified_agent(
            &reconnect_sftp,
            &*reconnect_inner,
            &reconnect_remote,
            &reconnect_expected,
            &reconnect_command,
        )
    });
    let (r, w) = open_verified_agent(sftp, &*inner, &remote, &expected, &serve_command)?;
    AgentBackend::from_root_confined_streams_with_reconnect(
        r,
        w,
        inner,
        reconnect,
        sftp.root_display(),
    )
}

/// Remove a deployed agent from a server.
pub fn remove_from_sftp(sftp: &crate::sftp::SftpBackend) -> io::Result<()> {
    sftp.exec_capture("rm -rf \"$HOME/.cache/smart-explorer\"")?;
    Ok(())
}
