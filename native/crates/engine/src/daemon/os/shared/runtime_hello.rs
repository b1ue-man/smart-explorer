//! Exact semantic contract negotiation over the existing authenticated IPC.
use super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
};
use crate::mcp_launcher::{
    Installation, RuntimeContract, RuntimeHello, RuntimeHelloReply, RuntimeIdentity,
};
use std::time::Duration;

pub(super) fn reply(hello: &RuntimeHello, generation: &str) -> RuntimeHelloReply {
    reply_for(Installation::current(), hello, generation)
}

/// `image` is the installation of the daemon's own image (`Installation::current`).
pub(super) fn reply_for(
    image: Result<Option<Installation>, String>,
    hello: &RuntimeHello,
    generation: &str,
) -> RuntimeHelloReply {
    let admitted = Installation::for_daemon(
        image,
        &hello.installation_id,
        hello.installation_root.as_deref(),
    )
    .map(|installation| {
        installation.manifest.permits(&RuntimeContract {
            runtime_sha256: hello.runtime_sha256.clone(),
            ipc_contract_id: hello.ipc_contract_id.clone(),
            shared_state_contract_id: hello.shared_state_contract_id.clone(),
        })
    });
    let accepted = admitted.as_ref().is_ok_and(|permitted| *permitted);
    let outside = admitted
        .as_ref()
        .is_err_and(|error| error == crate::mcp_launcher::DAEMON_OUTSIDE_INSTALLATION);
    RuntimeHelloReply {
        accepted,
        daemon_generation: generation.into(),
        ipc_contract_id: if accepted {
            hello.ipc_contract_id.clone()
        } else {
            crate::mcp_launcher::IPC_CONTRACT.into()
        },
        shared_state_contract_id: if accepted {
            hello.shared_state_contract_id.clone()
        } else {
            crate::mcp_launcher::STATE_CONTRACT.into()
        },
        error_code: (!accepted).then(|| {
            if outside {
                crate::mcp_launcher::DAEMON_OUTSIDE_INSTALLATION.into()
            } else {
                "reconnect_required".into()
            }
        }),
    }
}

pub(super) fn ready_reply(
    hello: &RuntimeHello,
    generation: &str,
    initialized: bool,
) -> RuntimeHelloReply {
    let mut answer = reply(hello, generation);
    if !initialized {
        answer.accepted = false;
        answer.error_code = Some("daemon_starting".into());
    }
    answer
}
pub fn client(runtime: &RuntimeIdentity) -> Result<RuntimeHelloReply, String> {
    negotiate(runtime).map(|(answer, _)| answer)
}

/// The accepted answer and the root the daemon needed (`Some` only for a
/// daemon outside every installation). Every bound request repeats that root.
fn negotiate(runtime: &RuntimeIdentity) -> Result<(RuntimeHelloReply, Option<String>), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    let mut root = None;
    loop {
        let answer = hello_once(runtime, root.clone())?;
        if answer.error_code.as_deref() == Some("daemon_starting")
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        if answer.accepted {
            return Ok((answer, root));
        }
        if root.is_none()
            && answer.error_code.as_deref()
                == Some(crate::mcp_launcher::DAEMON_OUTSIDE_INSTALLATION)
        {
            root = Some(installation_root(runtime));
            continue;
        }
        return Err(format!(
            "reconnect_required: {}",
            answer
                .error_code
                .as_deref()
                .unwrap_or("runtime contract rejected")
        ));
    }
}
fn installation_root(runtime: &RuntimeIdentity) -> String {
    runtime.installation.root.to_string_lossy().into_owned()
}
fn hello_once(
    runtime: &RuntimeIdentity,
    installation_root: Option<String>,
) -> Result<RuntimeHelloReply, String> {
    let mut stream = connect(runtime)?;
    let token =
        super::ipc_storage::read_token().map_err(|e| format!("runtime_hello_token: {e}"))?;
    let generation = super::ipc_storage::read_ipc_generation()
        .ok_or("reconnect_required: daemon generation missing")?;
    set_stream_timeout(&stream, Some(Duration::from_secs(3)));
    write_request(
        &mut stream,
        &IpcRequest::RuntimeHello {
            token,
            hello: RuntimeHello {
                installation_id: runtime.installation.manifest.installation_id.clone(),
                runtime_sha256: runtime.contract.runtime_sha256.clone(),
                ipc_contract_id: runtime.contract.ipc_contract_id.clone(),
                shared_state_contract_id: runtime.contract.shared_state_contract_id.clone(),
                installation_root,
            },
        },
    )
    .map_err(|e| format!("reconnect_required: RuntimeHello write: {e}"))?;
    match read_response(&mut stream)
        .map_err(|e| format!("reconnect_required: RuntimeHello unsupported: {e}"))?
    {
        IpcResponse::RuntimeHello { reply }
            if reply.daemon_generation == generation
                && reply.ipc_contract_id == runtime.contract.ipc_contract_id
                && reply.shared_state_contract_id == runtime.contract.shared_state_contract_id
                && super::ipc_storage::read_ipc_generation().as_deref()
                    == Some(generation.as_str()) =>
        {
            Ok(reply)
        }
        _ => Err("reconnect_required: daemon generation or semantic contract changed".into()),
    }
}
fn connect(runtime: &RuntimeIdentity) -> Result<std::net::TcpStream, String> {
    if let Some(addr) = super::ipc_storage::read_ipc_addr() {
        if let Ok(stream) = crate::daemon::ipc_proof::connect(&addr, Duration::from_millis(700)) {
            return Ok(stream);
        }
    }
    crate::mcp_launcher::start_current_daemon(runtime)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    while std::time::Instant::now() < deadline {
        if let Some(addr) = super::ipc_storage::read_ipc_addr() {
            if let Ok(stream) = crate::daemon::ipc_proof::connect(&addr, Duration::from_millis(200))
            {
                return Ok(stream);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("reconnect_required: daemon endpoint not ready".into())
}

/// Bind every generation-owned request to the daemon that accepted Hello.
/// A switch between connecting the stream and negotiating cannot execute the
/// mutation against an unnegotiated daemon; neither path retries that mutation.
pub(super) fn encode_request(request: &IpcRequest) -> std::io::Result<String> {
    if matches!(
        request,
        IpcRequest::RuntimeHello { .. } | IpcRequest::RuntimeCall { .. }
    ) {
        return serde_json::to_string(request).map_err(std::io::Error::other);
    }
    let Some(runtime) = crate::mcp_launcher::current_runtime() else {
        return serde_json::to_string(request).map_err(std::io::Error::other);
    };
    let (accepted, installation_root) = negotiate(&runtime).map_err(std::io::Error::other)?;
    #[derive(serde::Serialize)]
    struct Bound<'a> {
        t: &'static str,
        token: String,
        hello: RuntimeHello,
        daemon_generation: String,
        request: &'a IpcRequest,
    }
    let bound = Bound {
        t: "runtime_call",
        token: super::ipc_storage::read_token()?,
        hello: RuntimeHello {
            installation_id: runtime.installation.manifest.installation_id.clone(),
            runtime_sha256: runtime.contract.runtime_sha256,
            ipc_contract_id: runtime.contract.ipc_contract_id,
            shared_state_contract_id: runtime.contract.shared_state_contract_id,
            installation_root,
        },
        daemon_generation: accepted.daemon_generation,
        request,
    };
    serde_json::to_string(&bound).map_err(std::io::Error::other)
}
pub(super) fn unwrap_request(
    request: IpcRequest,
    generation: &str,
    initialized: bool,
) -> std::io::Result<IpcRequest> {
    match request {
        IpcRequest::RuntimeCall {
            hello,
            daemon_generation,
            request,
            ..
        } => {
            if !initialized
                || daemon_generation != generation
                || !reply(&hello, generation).accepted
                || matches!(
                    *request,
                    IpcRequest::RuntimeCall { .. } | IpcRequest::RuntimeHello { .. }
                )
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "reconnect_required: daemon generation or runtime contract changed",
                ));
            }
            Ok(*request)
        }
        request => Ok(request),
    }
}

pub(super) fn request_lease(
    hello: Option<&RuntimeHello>,
    request: &IpcRequest,
) -> std::io::Result<Option<std::sync::Arc<crate::mcp_launcher::OperationGuard>>> {
    let Some(hello) = hello else {
        return Ok(None);
    };
    use super::ipc_protocol::{screen::ScreenIpcRequest, VaultIpcRequest};
    // These inspect already-held local state or cancel existing work. They
    // never admit a new network/filesystem task during prepared.
    let observation = matches!(
        request,
        IpcRequest::Ping { .. }
            | IpcRequest::ExecJobs { .. }
            | IpcRequest::CancelExec { .. }
            | IpcRequest::StopMount { .. }
            | IpcRequest::ListMounts { .. }
            | IpcRequest::RuntimeHello { .. }
    ) || matches!(
        request,
        IpcRequest::Vault {
            req: VaultIpcRequest::Status | VaultIpcRequest::Items { .. },
            ..
        }
    ) || matches!(
        request,
        IpcRequest::Screen {
            req: ScreenIpcRequest::HostSessions
                | ScreenIpcRequest::HostStop { .. }
                | ScreenIpcRequest::HostWatch { .. },
            ..
        }
    );
    if observation {
        return Ok(None);
    }
    let contract = RuntimeContract {
        runtime_sha256: hello.runtime_sha256.clone(),
        ipc_contract_id: hello.ipc_contract_id.clone(),
        shared_state_contract_id: hello.shared_state_contract_id.clone(),
    };
    crate::mcp_launcher::operation_for_contract(
        &hello.installation_id,
        &contract,
        hello.installation_root.as_deref(),
    )
    .map(Some)
    .map_err(std::io::Error::other)
}

#[cfg(test)]
#[path = "plugin_task_runtime_hello_tests.rs"]
mod plugin_task_runtime_hello_tests;
