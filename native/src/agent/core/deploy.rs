use super::backend::AgentBackend;
use crate::agent_proto;
use crate::vfs::BackendHandle;
use std::io::{self, Write};

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
    let remote = format!("{}/se-agent-p{}", dir, agent_proto::PROTO_VERSION);

    let want = format!("proto={}", agent_proto::PROTO_VERSION);
    let probe = sftp
        .exec_capture(&format!("{} --version 2>/dev/null", sh_quote(&remote)))
        .unwrap_or_default();
    if !probe.contains(&want) {
        inner.mkdir_all(&dir)?;
        let tmp = format!("{}.tmp", remote);
        {
            let mut w = inner.open_write(&tmp)?;
            w.write_all(art.bytes)?;
            w.flush()?;
        }
        let expected = sha256_hex(art.bytes);
        let sum = sftp
            .exec_capture(&format!(
                "sha256sum {} 2>/dev/null | cut -d' ' -f1",
                sh_quote(&tmp)
            ))
            .unwrap_or_default();
        if !sum.is_empty() && !sum.eq_ignore_ascii_case(&expected) {
            let _ = inner.remove_file(&tmp);
            return Err(io::Error::other("Agent-Binary: SHA-256 stimmt nicht"));
        }
        sftp.exec_capture(&format!(
            "mv -f {tmp} {remote} && chmod 700 {remote}",
            tmp = sh_quote(&tmp),
            remote = sh_quote(&remote),
        ))?;
    }

    let (r, w) = sftp.open_exec_streams(&format!("{} --serve", sh_quote(&remote)))?;
    AgentBackend::from_streams(r, w, inner)
}

/// Remove a deployed agent from a server.
pub fn remove_from_sftp(sftp: &crate::sftp::SftpBackend) -> io::Result<()> {
    sftp.exec_capture("rm -rf \"$HOME/.cache/smart-explorer\"")?;
    Ok(())
}
