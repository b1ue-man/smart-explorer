//! Connection to the running SSH agent on Unix hosts (Linux, Android):
//! the socket named by `SSH_AUTH_SOCK`, as `ssh` uses it. The login itself
//! (identities, signatures) stays in `core/session_auth.rs`.
use russh::keys::agent::client::{AgentClient, AgentStream};

/// An agent connection whose transport type is hidden from `core`.
pub(super) type Agent = AgentClient<Box<dyn AgentStream + Send + Unpin + 'static>>;

/// Connects to the agent of `SSH_AUTH_SOCK`.
pub(super) async fn connect() -> Result<Agent, russh::keys::Error> {
    let agent = AgentClient::connect_env().await?;
    Ok(agent.dynamic())
}
