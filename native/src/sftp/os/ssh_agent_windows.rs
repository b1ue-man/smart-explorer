//! Connection to the running SSH agent on Windows hosts: the pipe of the
//! Windows OpenSSH agent service first, then Pageant. The login itself
//! (identities, signatures) stays in `core/session_auth.rs`.
use russh::keys::agent::client::{AgentClient, AgentStream};

const OPENSSH_AGENT_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";

/// An agent connection whose transport type is hidden from `core`.
pub(super) type Agent = AgentClient<Box<dyn AgentStream + Send + Unpin + 'static>>;

/// Connects to the Windows OpenSSH agent; when its pipe is not there,
/// to Pageant.
pub(super) async fn connect() -> Result<Agent, russh::keys::Error> {
    match AgentClient::connect_named_pipe(OPENSSH_AGENT_PIPE).await {
        Ok(agent) => Ok(agent.dynamic()),
        Err(_) => {
            let agent = AgentClient::connect_pageant().await?;
            Ok(agent.dynamic())
        }
    }
}
